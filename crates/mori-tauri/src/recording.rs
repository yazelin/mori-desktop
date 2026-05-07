//! 麥克風錄音 — cross-platform via cpal,輸出 16-bit WAV bytes。
//!
//! cpal::Stream 在大部分平台是 `!Send`,所以我們把整個錄音邏輯隔離在
//! 自己的 OS thread,透過 channel 跟 tokio 世界溝通:
//!
//! ```text
//!   start_recording()
//!       │
//!       ▼
//!   spawn OS thread ──── cpal::Stream::play() ──── audio callback
//!       │                                              │
//!       │                                              ▼ pushes samples
//!       │                                          shared Vec<i16>
//!       │
//!       ▼ blocks on stop_rx.recv()
//!   when stop signaled → drop stream → return RecordedAudio

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

/// PCM samples + format metadata.
pub struct RecordedAudio {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl RecordedAudio {
    /// 編碼成 WAV bytes(供 Whisper API 用)。
    pub fn to_wav_bytes(&self) -> Result<Vec<u8>> {
        let spec = hound::WavSpec {
            channels: self.channels,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        {
            let mut writer =
                hound::WavWriter::new(&mut cursor, spec).context("create WAV writer")?;
            for &s in &self.samples {
                writer.write_sample(s).context("write WAV sample")?;
            }
            writer.finalize().context("finalize WAV")?;
        }
        Ok(cursor.into_inner())
    }

    /// 約略秒數(供 UI / log 顯示)。
    pub fn duration_secs(&self) -> f32 {
        let total_samples = self.samples.len() as f32;
        let frame_rate = (self.sample_rate as f32) * (self.channels as f32);
        if frame_rate == 0.0 {
            0.0
        } else {
            total_samples / frame_rate
        }
    }
}

/// 進行中的錄音 handle。drop 不會自動停 — 必須呼叫 `stop()` 拿結果。
pub struct Recorder {
    handle: Option<JoinHandle<Result<RecordedAudio>>>,
    stop_tx: Option<mpsc::Sender<()>>,
    /// 即時 RMS 等級,0..=u16::MAX。Audio callback 會持續更新。
    /// 共享給 UI polling task 用。
    level: Arc<AtomicU16>,
}

impl Recorder {
    /// 開始錄音。立刻回傳,實際錄音在背景 thread 進行。
    pub fn start() -> Result<Self> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let level = Arc::new(AtomicU16::new(0));
        let level_for_thread = level.clone();

        let handle = std::thread::Builder::new()
            .name("mori-recorder".into())
            .spawn(move || -> Result<RecordedAudio> {
                run_recording_thread(stop_rx, level_for_thread)
            })
            .context("spawn recorder thread")?;

        Ok(Self {
            handle: Some(handle),
            stop_tx: Some(stop_tx),
            level,
        })
    }

    /// 取得 0.0..=1.0 的當下 RMS 音量。供 UI 即時顯示用。
    pub fn current_level(&self) -> f32 {
        self.level.load(Ordering::Relaxed) as f32 / u16::MAX as f32
    }

    /// 共享 atomic 給外部(例如 polling task)直接讀,免每次經過 `&self`。
    pub fn level_arc(&self) -> Arc<AtomicU16> {
        self.level.clone()
    }

    /// 停止錄音,等到 background thread 結束,回傳錄到的音訊。
    pub fn stop(mut self) -> Result<RecordedAudio> {
        if let Some(tx) = self.stop_tx.take() {
            // 即使 receiver 已經斷線也不算錯
            let _ = tx.send(());
        }
        let handle = self
            .handle
            .take()
            .ok_or_else(|| anyhow!("recorder already stopped"))?;
        match handle.join() {
            Ok(result) => result,
            Err(_) => bail!("recorder thread panicked"),
        }
    }
}

fn run_recording_thread(
    stop_rx: mpsc::Receiver<()>,
    level: Arc<AtomicU16>,
) -> Result<RecordedAudio> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("no default input device"))?;
    let device_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());

    let config = device
        .default_input_config()
        .context("get default input config")?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();
    let sample_format = config.sample_format();
    tracing::info!(
        device = %device_name,
        sample_rate,
        channels,
        ?sample_format,
        "recorder: opening input stream"
    );

    let buffer: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::with_capacity(
        sample_rate as usize * channels as usize * 8, // ~8s pre-allocated
    )));
    let stream_buffer = buffer.clone();

    let stream_config: cpal::StreamConfig = config.config();
    let err_fn = |err| tracing::error!(?err, "recorder: stream error");

    let level_f32 = level.clone();
    let level_i16 = level.clone();
    let level_u16 = level.clone();

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                let mut b = stream_buffer.lock();
                b.reserve(data.len());
                let mut sum_sq = 0.0f64;
                for &s in data {
                    let clamped = s.clamp(-1.0, 1.0);
                    sum_sq += (clamped as f64) * (clamped as f64);
                    b.push((clamped * i16::MAX as f32) as i16);
                }
                update_level(&level_f32, sum_sq, data.len());
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _info: &cpal::InputCallbackInfo| {
                let mut b = stream_buffer.lock();
                b.extend_from_slice(data);
                let sum_sq: f64 = data
                    .iter()
                    .map(|&s| {
                        let n = s as f64 / i16::MAX as f64;
                        n * n
                    })
                    .sum();
                update_level(&level_i16, sum_sq, data.len());
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _info: &cpal::InputCallbackInfo| {
                let mut b = stream_buffer.lock();
                b.reserve(data.len());
                let mut sum_sq = 0.0f64;
                for &s in data {
                    let n = (s as f64 - 32768.0) / 32768.0;
                    sum_sq += n * n;
                    // u16 [0, 65535] → i16 [-32768, 32767]
                    b.push((s as i32 - 32768) as i16);
                }
                update_level(&level_u16, sum_sq, data.len());
            },
            err_fn,
            None,
        ),
        other => bail!("unsupported sample format: {:?}", other),
    }
    .context("build input stream")?;

    stream.play().context("start input stream")?;
    tracing::info!("recorder: stream playing, waiting for stop signal");

    // Block until stop signaled. A drop of the sender also wakes us up.
    let _ = stop_rx.recv();

    drop(stream); // explicit: stop capture

    let samples = std::mem::take(&mut *buffer.lock());
    let audio = RecordedAudio {
        samples,
        sample_rate,
        channels,
    };
    tracing::info!(
        duration_secs = audio.duration_secs(),
        bytes = audio.samples.len() * 2,
        "recorder: stopped"
    );
    Ok(audio)
}

/// 從 callback 算出 RMS,放進 atomic(0..=u16::MAX),供 UI polling 讀取。
/// 用 sqrt 但保留 0..1 range,scale 到 u16。
fn update_level(level: &Arc<AtomicU16>, sum_sq: f64, n: usize) {
    if n == 0 {
        return;
    }
    let rms = (sum_sq / n as f64).sqrt().clamp(0.0, 1.0);
    let scaled = (rms * u16::MAX as f64) as u16;
    level.store(scaled, Ordering::Relaxed);
}
