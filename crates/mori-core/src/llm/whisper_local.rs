//! 本機 Whisper(whisper.cpp via whisper-rs)— 100% 離線 STT。
//!
//! 解決 5A-1 ~ 5A-3 之後 Mori 還是綁 Groq 的最後一塊。配上 Ollama 或
//! Claude CLI 後就能完全 Groq-free,語音輸入也照常用。
//!
//! ## 模型檔
//! 走 ggml `.bin` 格式,從 huggingface 抓:
//!   <https://huggingface.co/ggerganov/whisper.cpp/tree/main>
//!
//! 中文場景大小取捨(Acer Intel CPU 實測):
//! | 模型 | 檔案大小 | 中文準度 | CPU 速度(real-time 比) |
//! |---|---|---|---|
//! | base   | 142MB | 普通,常分不清同音字 | ~3x realtime |
//! | small  | 466MB | 不錯,日常對話足夠 | ~1x realtime |
//! | medium | 1.5GB | 好,專有名詞少出錯 | ~0.5x realtime |
//!
//! 5C 預設 `~/.mori/models/ggml-small.bin`(中文夠用、檔案不過大、CPU 撐得住),
//! user 想換更大的就在 config 改 `providers.whisper-local.model_path`。
//!
//! ## 為什麼吃 WAV bytes 而不直接收 PCM
//! 為了跟 Groq 路徑共用 `TranscriptionProvider` trait — 兩邊都收 WAV bytes,
//! main.rs 不必知道走哪條路。WAV decode 在 CPU 上 < 50ms,一次性成本可
//! 忽略;真正花時間的是 whisper inference。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context as _, Result};
use async_trait::async_trait;
use parking_lot_compat::Mutex;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::transcribe::TranscriptionProvider;

/// whisper.cpp 推理需要 16kHz、單聲道、f32 PCM
const TARGET_SAMPLE_RATE: u32 = 16_000;

/// LocalWhisperProvider 內部需要可變 state(whisper-rs 的 state object 不是
/// `Sync`),用 Mutex 包起來,讓 `transcribe()` 在不同 task 間 serialize。
/// 在 CPU 上做 whisper 推理本來就 CPU-bound,平行也吃同一顆 CPU,序列
/// 化反而簡化生命週期。
pub struct LocalWhisperProvider {
    /// 把 WhisperContext 包在 Arc 裡,便於 .clone() 不重新讀檔。
    ctx: Arc<WhisperContext>,
    /// 模型檔路徑(供 IPC / log 顯示用)
    model_path: PathBuf,
    /// "zh" / "en" / "auto"。Some("auto") 會讓 whisper-rs 設 detect-mode。
    language: Option<String>,
    /// 推理 thread 數。預設 = available_parallelism()。
    n_threads: i32,
    /// state 不是 Sync,要 mutex 串起來。
    state: Arc<Mutex<whisper_rs::WhisperState>>,
}

impl LocalWhisperProvider {
    pub const NAME: &'static str = "whisper-local";

    /// 從 config 蓋出 provider — 路徑由 `providers.whisper-local.model_path`
    /// 指定,沒設就用 [`default_model_path`]。
    pub fn from_config() -> Result<Self> {
        let model_path = mori_config_path()
            .as_deref()
            .and_then(|p| super::groq::read_json_pointer(p, "/providers/whisper-local/model_path"))
            .map(PathBuf::from)
            .unwrap_or_else(default_model_path);
        let language = mori_config_path()
            .as_deref()
            .and_then(|p| super::groq::read_json_pointer(p, "/providers/whisper-local/language"));

        Self::new(&model_path, language)
    }

    pub fn new(model_path: &Path, language: Option<String>) -> Result<Self> {
        if !model_path.exists() {
            bail!(
                "whisper model not found at {}\n\nDownload one from \
                 https://huggingface.co/ggerganov/whisper.cpp/tree/main and put it there.\n\
                 Recommended for Chinese: ggml-small.bin (466MB) — `wget -O {} https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin`",
                model_path.display(),
                model_path.display(),
            );
        }

        let path_str = model_path
            .to_str()
            .ok_or_else(|| anyhow!("model path not valid UTF-8: {}", model_path.display()))?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .with_context(|| format!("load whisper model from {}", model_path.display()))?;
        let state = ctx
            .create_state()
            .context("create whisper state")?;

        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4);

        Ok(Self {
            ctx: Arc::new(ctx),
            model_path: model_path.to_path_buf(),
            language,
            n_threads,
            state: Arc::new(Mutex::new(state)),
        })
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }
}

#[async_trait]
impl TranscriptionProvider for LocalWhisperProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn transcribe(&self, audio: Vec<u8>) -> Result<String> {
        // whisper.cpp inference 是純 CPU、阻塞、長時間(可能秒級到分鐘級),
        // 不能在 tokio reactor thread 上跑,要 spawn_blocking。
        let ctx = self.ctx.clone();
        let state = self.state.clone();
        let language = self.language.clone();
        let n_threads = self.n_threads;
        let model_path = self.model_path.clone();

        tokio::task::spawn_blocking(move || -> Result<String> {
            tracing::debug!(
                bytes = audio.len(),
                model = %model_path.display(),
                threads = n_threads,
                "whisper-local transcribe request",
            );

            // 1. WAV → f32 PCM mono 16kHz
            let samples = decode_wav_to_mono16k(&audio)?;
            tracing::debug!(
                input_samples = samples.len(),
                duration_secs = samples.len() as f32 / TARGET_SAMPLE_RATE as f32,
                "wav decoded + resampled",
            );

            // 2. 跑 whisper inference
            let mut state = state.lock();
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_n_threads(n_threads);
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);

            // Language:沒設或設 "auto" 都讓 whisper 自偵測;明確設值則照給。
            let lang_str = language.as_deref().unwrap_or("auto");
            params.set_language(Some(lang_str));

            // 對 AI 助手講話的 prompt(跟 GroqProvider 一致),減少 "Thank you"
            // 那類 caption hallucination。whisper-rs 0.14 的 set_initial_prompt
            // 直接吃 &str。
            params.set_initial_prompt(
                "以下是使用者直接對 AI 助手 Mori 說的話,繁體中文。\
                 常見用語:程式、軟體、檔案、影片、電腦、滑鼠、伺服器、資料庫、\
                 記住、提醒、行事曆、會議。",
            );

            state
                .full(params, &samples)
                .context("whisper full inference")?;

            // 3. 收 segments 拼成最終字串
            let n_segments = state.full_n_segments().context("read n_segments")?;
            let mut text = String::new();
            for i in 0..n_segments {
                let seg = state
                    .full_get_segment_text(i)
                    .context("read segment text")?;
                text.push_str(&seg);
            }
            // whisper 偶爾會加開頭空白,trim 掉
            let text = text.trim().to_string();

            // 防呆:整段都是空字串(可能因為環境噪音 + initial prompt 沒鎮住)
            if text.is_empty() {
                tracing::warn!("whisper-local returned empty transcription — mic may be muted");
            }

            // 釋放 _ctx,不然 compiler 警告未使用。但實際上 WhisperState 持有
            // 的指標來自 ctx,Arc 確保它活著。
            let _ = ctx;
            Ok(text)
        })
        .await
        .context("whisper-local: tokio join")?
    }
}

/// `~/.mori/models/ggml-small.bin` — 5C 預設模型路徑。
pub fn default_model_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".mori").join("models").join("ggml-small.bin")
}

fn mori_config_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| PathBuf::from(h).join(".mori").join("config.json"))
}

/// 解 WAV bytes 成 mono 16kHz f32 samples。
///
/// 步驟:
/// 1. 用 hound 讀 WAV header + samples
/// 2. i16/i32/f32 統一轉 f32 [-1.0, 1.0]
/// 3. 多聲道 → mean-mix down to mono
/// 4. sample rate ≠ 16000 → 用 rubato 重採樣(高品質 sinc)
fn decode_wav_to_mono16k(wav: &[u8]) -> Result<Vec<f32>> {
    let cursor = std::io::Cursor::new(wav);
    let mut reader = hound::WavReader::new(cursor).context("open WAV from bytes")?;
    let spec = reader.spec();

    // Step 1+2:samples → f32
    let mut interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            // 依 bits_per_sample 正規化到 [-1, 1]
            let max = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("read int samples")?
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("read float samples")?,
    };

    // Step 3:多聲道 → mono(平均)
    let channels = spec.channels as usize;
    if channels > 1 {
        interleaved = interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect();
    }

    // Step 4:resample(若需要)
    if spec.sample_rate == TARGET_SAMPLE_RATE {
        return Ok(interleaved);
    }
    resample_to_target(&interleaved, spec.sample_rate, TARGET_SAMPLE_RATE)
}

fn resample_to_target(input: &[f32], from_hz: u32, to_hz: u32) -> Result<Vec<f32>> {
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = to_hz as f64 / from_hz as f64;
    // chunk_size 給整段一次處理 — 我們的音訊是有限長度(< 30s 通常),
    // 一次性 resampler 比 streaming 簡單,品質一樣。
    let mut resampler = SincFixedIn::<f32>::new(
        ratio,
        2.0,
        params,
        input.len(),
        1, // mono
    )
    .context("build sinc resampler")?;

    let chunk = vec![input.to_vec()];
    let out = resampler
        .process(&chunk, None)
        .context("sinc resample")?;
    Ok(out.into_iter().next().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 寫一段 1 秒 440Hz sine,sample_rate 可指定 — 用來測 decode + resample 路徑。
    fn synth_wav_bytes(sample_rate: u32, channels: u16) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = hound::WavWriter::new(std::io::Cursor::new(&mut buf), spec).unwrap();
            let n = sample_rate as usize;
            for i in 0..n {
                let t = i as f32 / sample_rate as f32;
                let s = (t * 440.0 * std::f32::consts::TAU).sin();
                let v = (s * i16::MAX as f32 * 0.3) as i16;
                for _ in 0..channels {
                    writer.write_sample(v).unwrap();
                }
            }
            writer.finalize().unwrap();
        }
        buf
    }

    #[test]
    fn decode_16k_mono_pass_through() {
        let wav = synth_wav_bytes(16_000, 1);
        let samples = decode_wav_to_mono16k(&wav).unwrap();
        assert_eq!(samples.len(), 16_000, "1s @ 16kHz mono → 16000 samples");
    }

    #[test]
    fn decode_48k_mono_resamples() {
        let wav = synth_wav_bytes(48_000, 1);
        let samples = decode_wav_to_mono16k(&wav).unwrap();
        // rubato sinc 重採樣會留 short head/tail 寬容,16k ± a few hundred 都算對
        let diff = (samples.len() as i64 - 16_000).abs();
        assert!(
            diff < 1000,
            "48k → 16k 應產生 ~16000 samples,實際 {} (差 {})",
            samples.len(),
            diff
        );
    }

    #[test]
    fn decode_48k_stereo_mixes_to_mono_then_resamples() {
        let wav = synth_wav_bytes(48_000, 2);
        let samples = decode_wav_to_mono16k(&wav).unwrap();
        let diff = (samples.len() as i64 - 16_000).abs();
        assert!(
            diff < 1000,
            "48k stereo → 16k mono 應產生 ~16000 samples,實際 {}",
            samples.len()
        );
    }

    #[test]
    fn default_model_path_in_mori_dir() {
        let path = default_model_path();
        assert!(
            path.to_string_lossy().contains(".mori/models/"),
            "default 路徑該指向 ~/.mori/models/, 實際:{}",
            path.display()
        );
        assert!(path.file_name().unwrap().to_string_lossy().ends_with(".bin"));
    }

    #[test]
    fn provider_construction_fails_when_model_missing() {
        let nonexistent = PathBuf::from("/tmp/this-does-not-exist-mori-test.bin");
        let result = LocalWhisperProvider::new(&nonexistent, None);
        let err = match result {
            Ok(_) => panic!("expected Err for nonexistent model file"),
            Err(e) => e,
        };
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("not found") && msg.contains("huggingface"),
            "should include download instructions: {msg}"
        );
    }
}

// 用一個 alias 避免直接 depend on parking_lot — mori-core 用 std::sync::Mutex
// 也夠,但 std Mutex 在 lock 時要 unwrap PoisonError,囉嗦。為了不引入新 dep,
// 這裡用 std Mutex 配 .lock().expect() 也行,但既然我們已經透過 transitive
// (whisper-rs / rubato 不依賴 parking_lot)沒 parking_lot,我們就乖乖用 std。
// 把 alias 拉一個 module 好維護:換成 parking_lot 只動一個檔案。
mod parking_lot_compat {
    use std::sync::{Mutex as StdMutex, MutexGuard};

    pub struct Mutex<T>(StdMutex<T>);

    impl<T> Mutex<T> {
        pub fn new(t: T) -> Self {
            Self(StdMutex::new(t))
        }

        pub fn lock(&self) -> MutexGuard<'_, T> {
            // poison 表示前一個 holder panic 過 — 對我們的 cases(whisper
            // 推理)幾乎不可能。直接 expect 簡化呼叫端。
            self.0.lock().expect("whisper state mutex poisoned")
        }
    }
}
