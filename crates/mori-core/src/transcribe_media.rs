//! 音/影檔批次轉錄。
//!
//! 給 mori-desktop「轉錄」tab 跟潛在 skill 用。重點:
//!
//! 1. **接受任何格式**:ffmpeg 抽出 16kHz mono PCM WAV(Whisper 期待的格式),
//!    Whisper 端不必管 mp3/mp4/m4a/flac/mov/...。
//! 2. **長檔自動分塊**:>5 分鐘的音訊切成 5 分鐘 chunk 逐個轉,避免單次推論
//!    打爆 whisper-server 內存或超 timeout。Chunk 邊界刻意對齊整秒,不切音節
//!    中間(WAV header 偏移 + 16kHz 整數倍計算,reduce 漏字風險)。
//! 3. **進度回呼**:長檔/批次跑時透過 callback 回報「現在第幾塊 / 共幾塊」,
//!    UI 可即時顯示。
//!
//! ## 為什麼走 ffmpeg subprocess 不是 in-process decoder
//!
//! 走 ffmpeg 一條:
//! - **覆蓋面廣**:user 丟什麼格式都能吃,不必在 Rust 端編譯 mp3/aac/opus
//!   decoder(crate 體積爆 + 平台不穩)。
//! - **影片直接餵**:`-vn` 自動忽略 video stream,音軌直取。
//! - **runtime dep**:scripts/install-linux-deps.sh 已 ensure ffmpeg。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use tokio::process::Command;

use crate::llm::transcribe::TranscriptionProvider;

/// Chunk 大小:5 分鐘。經驗值 — whisper.cpp small model 在 CPU 上跑 5min 音
/// 訊 ~30-60 秒,留 buffer 對齊 `INFERENCE_TIMEOUT_SECS=120`(whisper_local.rs)。
const CHUNK_SECONDS: u32 = 300;

/// 取樣率 — Whisper 內部就是 16kHz mono,ffmpeg 輸出時直接降採樣免 server 端再做。
const TARGET_SAMPLE_RATE: u32 = 16_000;

/// 單次 transcribe 結果。
#[derive(Debug, Clone, Serialize)]
pub struct TranscribeResult {
    /// 來源檔絕對路徑(供 batch UI 對齊原檔顯示)
    pub source_path: PathBuf,
    /// 轉好的逐字稿(已 trim,chunked 結果空白接連)
    pub text: String,
    /// 音訊總長(秒,從 ffmpeg probe 拿)
    pub duration_secs: f32,
    /// 用了幾塊 chunk(1 = 單一短檔,>1 = 長檔分塊)
    pub chunks: u32,
}

/// 轉錄選項。
#[derive(Debug, Clone, Default)]
pub struct TranscribeOpts {
    /// `None` → 用 provider 預設(whisper-local 看 config 的 language 欄位)。
    pub language: Option<String>,
    /// 每塊長度,單位秒。None → 預設 [`CHUNK_SECONDS`] = 300。設 0 不分塊。
    pub chunk_seconds: Option<u32>,
}

/// 進度 callback signature:`(current_chunk, total_chunks, source_path)`。
/// Chunk 從 1 起算,total = 1 表示沒分塊(短檔)。
pub type ProgressFn = Arc<dyn Fn(u32, u32, &Path) + Send + Sync>;

/// 把任意音/影格式抽成 Whisper 要的 16kHz mono PCM WAV bytes。
///
/// stdin 不接;檔案路徑 → stdout WAV bytes。Buffer 全在 memory,1 小時音訊
/// ~115MB(可接受);10+ 小時的 user 應該丟伺服器跑而不是 desktop。
pub async fn extract_wav_bytes(input: &Path) -> Result<Vec<u8>> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
    ])
    .arg(input)
    .args([
        "-vn", // ignore video stream(影片檔直接拿音軌)
        "-ar",
        &TARGET_SAMPLE_RATE.to_string(),
        "-ac",
        "1", // mono
        "-c:a",
        "pcm_s16le",
        "-f",
        "wav",
        "-", // stdout
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    crate::suppress_console_on_windows!(cmd);
    let output = cmd
        .output()
        .await
        .context("spawn ffmpeg — 確認系統有裝 ffmpeg")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "ffmpeg failed (exit {:?}) on {}\n--- stderr ---\n{}",
            output.status.code(),
            input.display(),
            stderr.trim()
        );
    }
    if output.stdout.len() < 44 {
        bail!(
            "ffmpeg produced too-small output ({}B) — input 檔可能沒音軌或損壞: {}",
            output.stdout.len(),
            input.display()
        );
    }
    Ok(output.stdout)
}

/// 探測音/影檔總長度(秒)— 走 ffprobe。失敗給 0.0,不擋轉錄。
pub async fn probe_duration_secs(input: &Path) -> f32 {
    // 用 ffmpeg -i 也能拿,但 stderr parse 麻煩;ffprobe 一行 JSON 乾淨。
    // ffprobe 在 ffmpeg 套件裡,跟 ffmpeg 同步存在。
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input)
        .output()
        .await;

    match probe {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<f32>()
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

/// 主要 entry point — 單檔轉錄。長檔自動分塊。
pub async fn transcribe_media_file(
    input: &Path,
    provider: Arc<dyn TranscriptionProvider>,
    opts: TranscribeOpts,
    progress: Option<ProgressFn>,
) -> Result<TranscribeResult> {
    if !input.exists() {
        bail!("file not found: {}", input.display());
    }
    let duration = probe_duration_secs(input).await;
    let wav = extract_wav_bytes(input).await?;
    tracing::info!(
        path = %input.display(),
        duration_secs = duration,
        wav_bytes = wav.len(),
        chunk_secs = opts.chunk_seconds.unwrap_or(CHUNK_SECONDS),
        "transcribe_media: extracted WAV"
    );
    transcribe_wav_bytes(wav, duration, input, provider, opts, progress).await
}

/// 拿到 WAV bytes 後的轉錄核心 — 抽出讓 mock provider 可單元測。
///
/// 跟 `transcribe_media_file` 差別:跳過 ffmpeg 抽取階段,直接接受 WAV bytes
/// 跟「已知 duration」。生產路徑由 `transcribe_media_file` 餵 ffmpeg 結果進來;
/// 測試路徑可餵手工合成的 WAV(免 ffmpeg subprocess)。
pub async fn transcribe_wav_bytes(
    wav: Vec<u8>,
    duration_secs: f32,
    source_path: &Path,
    provider: Arc<dyn TranscriptionProvider>,
    opts: TranscribeOpts,
    progress: Option<ProgressFn>,
) -> Result<TranscribeResult> {
    let chunk_secs = opts.chunk_seconds.unwrap_or(CHUNK_SECONDS);

    // 不分塊 → 整檔 POST
    let should_chunk = chunk_secs > 0 && duration_secs > chunk_secs as f32 + 5.0; // +5s 容差,避免邊界檔多跑一塊
    if !should_chunk {
        if let Some(cb) = &progress {
            cb(1, 1, source_path);
        }
        let text = provider
            .transcribe(wav)
            .await
            .with_context(|| format!("transcribe {}", source_path.display()))?;
        return Ok(TranscribeResult {
            source_path: source_path.to_path_buf(),
            text,
            duration_secs,
            chunks: 1,
        });
    }

    // 分塊:把 WAV 解成 raw samples,切等長 chunk,each chunk 重新包成 WAV header
    let chunks = split_wav_into_chunks(&wav, chunk_secs)
        .with_context(|| format!("split WAV {}", source_path.display()))?;
    let total = chunks.len() as u32;
    tracing::info!(
        total_chunks = total,
        chunk_secs,
        "transcribe_media: split into chunks"
    );

    let mut combined = String::new();
    for (i, chunk_wav) in chunks.into_iter().enumerate() {
        let idx = i as u32 + 1;
        if let Some(cb) = &progress {
            cb(idx, total, source_path);
        }
        let part = provider.transcribe(chunk_wav).await.with_context(|| {
            format!(
                "transcribe chunk {idx}/{total} of {}",
                source_path.display()
            )
        })?;
        let part = part.trim();
        if !part.is_empty() {
            if !combined.is_empty() {
                combined.push(' ');
            }
            combined.push_str(part);
        }
    }

    Ok(TranscribeResult {
        source_path: source_path.to_path_buf(),
        text: combined,
        duration_secs,
        chunks: total,
    })
}

/// 批次轉錄一組檔案。Sequential — whisper-server 同時只能服務一個 inference,
/// 平行送只會在 server 內部排隊浪費 IPC。每檔完成發一次 progress。
///
/// 回傳 Vec 跟 inputs 一一對應;個別檔失敗存 Err,不擋其他檔繼續跑。
pub async fn transcribe_paths(
    inputs: &[PathBuf],
    provider: Arc<dyn TranscriptionProvider>,
    opts: TranscribeOpts,
    file_progress: Option<Arc<dyn Fn(usize, usize, &Path, &str) + Send + Sync>>,
    chunk_progress: Option<ProgressFn>,
) -> Vec<Result<TranscribeResult>> {
    let total = inputs.len();
    let mut results = Vec::with_capacity(total);
    for (i, path) in inputs.iter().enumerate() {
        if let Some(cb) = &file_progress {
            cb(i + 1, total, path, "start");
        }
        let r =
            transcribe_media_file(path, provider.clone(), opts.clone(), chunk_progress.clone())
                .await;
        if let Some(cb) = &file_progress {
            let status = if r.is_ok() { "ok" } else { "err" };
            cb(i + 1, total, path, status);
        }
        results.push(r);
    }
    results
}

/// WAV chunking:接 ffmpeg 出的 16kHz mono PCM_S16LE WAV bytes,切成等長 chunk,
/// 每個 chunk 重新包 WAV header 後輸出。
///
/// 為什麼自己切不交給 ffmpeg `-ss`:
/// - 已經抽過一次 ffmpeg,再對檔下指令 = 重新 decode,浪費
/// - WAV 結構單純(PCM 是 fixed-size sample),Rust 切位元組就行
///
/// 切點對齊整秒 + sample 邊界:`16000 samples/sec * chunk_secs * 2 bytes/sample`(mono)。
fn split_wav_into_chunks(wav: &[u8], chunk_secs: u32) -> Result<Vec<Vec<u8>>> {
    if wav.len() < 44 {
        bail!("WAV too small: {} bytes", wav.len());
    }
    if &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file");
    }
    // 找 data chunk — WAV header 後可能有 fmt, LIST, fact, ... 才到 data
    let (data_offset, data_size) = find_data_chunk(wav)?;
    let pcm = &wav[data_offset..data_offset + data_size];

    // 16kHz mono S16LE → 32000 bytes/sec
    let bytes_per_sec = (TARGET_SAMPLE_RATE * 2) as usize;
    let chunk_bytes = bytes_per_sec * chunk_secs as usize;
    if pcm.len() <= chunk_bytes {
        // 整個 PCM 還沒到一塊就放回去當單塊(should_chunk gating 通常會擋掉這條,
        // 但 caller 可能直接呼叫 split,留 defensive return)
        return Ok(vec![wav.to_vec()]);
    }

    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < pcm.len() {
        let end = (offset + chunk_bytes).min(pcm.len());
        let slice = &pcm[offset..end];
        out.push(wrap_pcm_as_wav(slice));
        offset = end;
    }
    Ok(out)
}

/// 找 WAV 的 data chunk:回 (data 內容起始 offset, data 長度)。
fn find_data_chunk(wav: &[u8]) -> Result<(usize, usize)> {
    let mut i = 12; // 跳過 RIFF(4) + size(4) + WAVE(4)
    while i + 8 <= wav.len() {
        let id = &wav[i..i + 4];
        let sz = u32::from_le_bytes([wav[i + 4], wav[i + 5], wav[i + 6], wav[i + 7]]) as usize;
        let data_start = i + 8;
        if id == b"data" {
            let data_end = (data_start + sz).min(wav.len());
            return Ok((data_start, data_end - data_start));
        }
        // skip 這個 chunk;chunk 長度奇數要 pad 1 byte
        i = data_start + sz + (sz & 1);
    }
    Err(anyhow!("no `data` chunk found in WAV"))
}

/// 把 PCM bytes 重包成 WAV(16kHz mono S16LE)。Header 44 bytes,size 欄位寫死照
/// 計算填,whisper-server multipart 不嚴格驗 RIFF size。
fn wrap_pcm_as_wav(pcm: &[u8]) -> Vec<u8> {
    let data_size = pcm.len() as u32;
    let riff_size = 36 + data_size;
    let mut buf = Vec::with_capacity(44 + pcm.len());
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&TARGET_SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&(TARGET_SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    buf.extend_from_slice(pcm);
    buf
}

/// 副檔名白名單 — 用來篩資料夾批次轉錄。所有 ffmpeg 能 demux 的都納入,
/// 但這只用於 UI filter:user 顯式指定一個檔不在清單我們也試。
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // 純音訊
    "wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "wma", "amr",
    // 影片(只取音軌)
    "mp4", "mkv", "webm", "mov", "avi", "wmv", "flv", "m4v", "ts",
];

/// 判斷副檔名是否在白名單(case-insensitive)。
pub fn has_supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

/// 確認 ffmpeg 可呼叫;失敗 = 系統沒裝。
pub async fn check_ffmpeg() -> Result<String> {
    let out = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("spawn ffmpeg —— 系統沒裝 ffmpeg。Ubuntu: `sudo apt install ffmpeg`")?;
    if !out.status.success() {
        bail!("ffmpeg --version failed");
    }
    let first_line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    Ok(first_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_ext_check() {
        assert!(has_supported_extension(Path::new("/x/foo.mp3")));
        assert!(has_supported_extension(Path::new("/x/Foo.MP4")));
        assert!(has_supported_extension(Path::new("/x/bar.wav")));
        assert!(!has_supported_extension(Path::new("/x/foo.txt")));
        assert!(!has_supported_extension(Path::new("/x/foo")));
    }

    #[test]
    fn wrap_pcm_as_wav_header() {
        let pcm = vec![0u8; 32_000]; // 1 sec mono 16kHz s16le
        let wav = wrap_pcm_as_wav(&pcm);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        // data size 欄位應該是 32000
        let data_sz = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_sz, 32_000);
        assert_eq!(wav.len(), 44 + 32_000);
    }

    #[test]
    fn split_wav_short_returns_single() {
        // 1 sec WAV — 應該回單一 chunk(不該分塊到比 input 還多)
        let wav = wrap_pcm_as_wav(&vec![0u8; 32_000]);
        let chunks = split_wav_into_chunks(&wav, 300).unwrap();
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn split_wav_long_chunks_correctly() {
        // 10 秒 WAV → chunk=3 秒 → 期望 4 塊(3+3+3+1)
        let pcm = vec![0u8; 32_000 * 10];
        let wav = wrap_pcm_as_wav(&pcm);
        let chunks = split_wav_into_chunks(&wav, 3).unwrap();
        assert_eq!(chunks.len(), 4);
        // 每塊都該是合法 WAV
        for c in &chunks {
            assert_eq!(&c[..4], b"RIFF");
            assert_eq!(&c[8..12], b"WAVE");
        }
    }

    // ─── transcribe_wav_bytes with mock provider ──────────────────────
    //
    // Mock `TranscriptionProvider`:每次 `transcribe()` 拿 canned 字串(or 依
    // call index 拿不同字串),記錄 call 次數,讓 test 驗證 chunks → text 流程。

    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex as TokioMutex;

    struct MockProvider {
        /// 每次 call 拿 chunk index 對應字串。空 → 拿 default。
        canned: Vec<String>,
        call_count: AtomicUsize,
        /// 也記下每次收到的 bytes 大小(用來驗 chunked input)
        received_sizes: TokioMutex<Vec<usize>>,
    }

    impl MockProvider {
        fn new(canned: Vec<&str>) -> Self {
            Self {
                canned: canned.into_iter().map(String::from).collect(),
                call_count: AtomicUsize::new(0),
                received_sizes: TokioMutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl TranscriptionProvider for MockProvider {
        fn name(&self) -> &'static str {
            "mock"
        }
        async fn transcribe(&self, audio: Vec<u8>) -> anyhow::Result<String> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            self.received_sizes.lock().await.push(audio.len());
            let text = self
                .canned
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("mock-chunk-{}", idx + 1));
            Ok(text)
        }
    }

    #[tokio::test]
    async fn transcribe_wav_bytes_single_chunk() {
        // 1 秒 WAV、chunk_secs=300 → 不切,單次 call,回 canned 字串。
        let wav = wrap_pcm_as_wav(&vec![0u8; 32_000]);
        let provider = Arc::new(MockProvider::new(vec!["hello world"]));
        let r = transcribe_wav_bytes(
            wav,
            1.0,
            Path::new("/test/foo.mp3"),
            provider.clone(),
            TranscribeOpts {
                language: None,
                chunk_seconds: Some(300),
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(r.text, "hello world");
        assert_eq!(r.chunks, 1);
        assert_eq!(r.duration_secs, 1.0);
        assert_eq!(r.source_path, Path::new("/test/foo.mp3"));
        assert_eq!(provider.call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn transcribe_wav_bytes_multi_chunk_concatenates() {
        // 10 秒 WAV、chunk_secs=3 → 切 4 塊,call 4 次,文字以空白接起。
        let wav = wrap_pcm_as_wav(&vec![0u8; 32_000 * 10]);
        let provider = Arc::new(MockProvider::new(vec![
            "chunk one",
            "chunk two",
            "chunk three",
            "chunk four",
        ]));
        let r = transcribe_wav_bytes(
            wav,
            10.0,
            Path::new("/test/long.mp3"),
            provider.clone(),
            TranscribeOpts {
                language: None,
                chunk_seconds: Some(3),
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(r.text, "chunk one chunk two chunk three chunk four");
        assert_eq!(r.chunks, 4);
        assert_eq!(provider.call_count.load(Ordering::SeqCst), 4);
        // 每塊應該收到合理大小的 WAV(都是 chunk-3sec 上下;最後一塊較小)
        let sizes = provider.received_sizes.lock().await.clone();
        assert_eq!(sizes.len(), 4);
        for size in &sizes[..3] {
            assert!(*size > 44, "chunk WAV should have header + data, got {size}");
        }
    }

    #[tokio::test]
    async fn transcribe_wav_bytes_chunk_secs_zero_disables_split() {
        // chunk_seconds=Some(0) → 不分塊不論多長
        let wav = wrap_pcm_as_wav(&vec![0u8; 32_000 * 60]); // 60 秒
        let provider = Arc::new(MockProvider::new(vec!["whole file"]));
        let r = transcribe_wav_bytes(
            wav,
            60.0,
            Path::new("/test/long.mp3"),
            provider.clone(),
            TranscribeOpts {
                language: None,
                chunk_seconds: Some(0),
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(r.text, "whole file");
        assert_eq!(r.chunks, 1);
        assert_eq!(provider.call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn transcribe_wav_bytes_progress_callback_fires_per_chunk() {
        let wav = wrap_pcm_as_wav(&vec![0u8; 32_000 * 10]);
        let provider = Arc::new(MockProvider::new(vec!["a", "b", "c", "d"]));
        let calls: Arc<std::sync::Mutex<Vec<(u32, u32)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls_ref = calls.clone();
        let cb: ProgressFn = Arc::new(move |cur, total, _p| {
            calls_ref.lock().unwrap().push((cur, total));
        });
        let _ = transcribe_wav_bytes(
            wav,
            10.0,
            Path::new("/test/long.mp3"),
            provider,
            TranscribeOpts {
                language: None,
                chunk_seconds: Some(3),
            },
            Some(cb),
        )
        .await
        .unwrap();
        let observed = calls.lock().unwrap().clone();
        assert_eq!(observed, vec![(1, 4), (2, 4), (3, 4), (4, 4)]);
    }

    #[tokio::test]
    async fn transcribe_wav_bytes_empty_parts_are_skipped() {
        // 模擬 whisper 偶爾在一塊回空字串 — 不該影響其他塊的串接
        let wav = wrap_pcm_as_wav(&vec![0u8; 32_000 * 10]);
        let provider = Arc::new(MockProvider::new(vec!["one", "", "three", ""]));
        let r = transcribe_wav_bytes(
            wav,
            10.0,
            Path::new("/test/long.mp3"),
            provider,
            TranscribeOpts {
                language: None,
                chunk_seconds: Some(3),
            },
            None,
        )
        .await
        .unwrap();
        // 空字串被 skip,不該變成 "one  three" 多空格
        assert_eq!(r.text, "one three");
        assert_eq!(r.chunks, 4);
    }
}
