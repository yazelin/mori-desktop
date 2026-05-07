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
}

impl Recorder {
    /// 開始錄音。立刻回傳,實際錄音在背景 thread 進行。
    pub fn start() -> Result<Self> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let handle = std::thread::Builder::new()
            .name("mori-recorder".into())
            .spawn(move || -> Result<RecordedAudio> { run_recording_thread(stop_rx) })
            .context("spawn recorder thread")?;

        Ok(Self {
            handle: Some(handle),
            stop_tx: Some(stop_tx),
        })
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

fn run_recording_thread(stop_rx: mpsc::Receiver<()>) -> Result<RecordedAudio> {
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

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                let mut b = stream_buffer.lock();
                b.reserve(data.len());
                for &s in data {
                    let clamped = s.clamp(-1.0, 1.0);
                    b.push((clamped * i16::MAX as f32) as i16);
                }
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _info: &cpal::InputCallbackInfo| {
                let mut b = stream_buffer.lock();
                b.extend_from_slice(data);
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _info: &cpal::InputCallbackInfo| {
                let mut b = stream_buffer.lock();
                b.reserve(data.len());
                for &s in data {
                    // u16 [0, 65535] → i16 [-32768, 32767]
                    b.push((s as i32 - 32768) as i16);
                }
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
