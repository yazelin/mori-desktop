#!/usr/bin/env python3
"""mori-voice-enroll.py — 註冊使用者聲紋(Phase 3E)。

用 resemblyzer 抽 30 秒語音的 256-dim 聲紋向量,存到 user 指定路徑。
mori-tauri 之後在每次 wake event 後比對 incoming 聲音是不是同一個人。

## 用法

  python mori-voice-enroll.py <output-embedding.npy> [--seconds 30]

  錄音從預設 input device 抓,sample_rate 16kHz mono(resemblyzer 內部會
  preprocess,但給標準格式比較穩)。

## stdout / stderr protocol

  stdout: JSON 進度事件,line-delimited
    {"event":"recording_start", "seconds": 30}
    {"event":"recording_progress", "elapsed": 5.0}
    {"event":"recording_done"}
    {"event":"embedding_start"}
    {"event":"embedding_done", "path": "/path/to/embedding.npy", "dim": 256}
    {"event":"error", "msg": "..."}

  Rust 端讀 stdout JSON 給 UI 顯示「錄音中... 25 秒剩」。

## 為什麼 30 秒

resemblyzer encoder 在 5-10s 已能抽穩定 embedding,但越長越穩定,30 秒涵蓋
不同 prosody / 停頓 / 音高範圍,提高 day-to-day 辨識穩定度。建議錄各種句子
(短/長、平靜/興奮、Hey Mori 變體)。

## 為什麼用 resemblyzer 不用 speechbrain / pyannote

resemblyzer:
  - 純 Python,純 PyTorch,沒外部 C++ dep
  - Pretrained on VoxCeleb,~80MB 模型
  - API 一行:`VoiceEncoder().embed_utterance(audio)`
  - 安裝 1 個 pip,2-3 分鐘

speechbrain / pyannote 準度高但裝起來重,Phase 3E MVP 用 resemblyzer 即可。
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path


def emit(obj: dict) -> None:
    print(json.dumps(obj), flush=True)


def main() -> None:
    ap = argparse.ArgumentParser(description="Enroll user voice for Mori speaker verification")
    ap.add_argument("output", type=Path, help="輸出 .npy 路徑,例 ~/.mori/voiceid/user_embedding.npy")
    ap.add_argument("--seconds", type=float, default=30.0, help="錄音秒數(預設 30,建議 ≥15)")
    ap.add_argument("--sample-rate", type=int, default=16000)
    args = ap.parse_args()

    try:
        import numpy as np
        import sounddevice as sd
        from resemblyzer import VoiceEncoder, preprocess_wav
    except ImportError as e:
        emit({"event": "error", "msg": f"missing dep: {e}. 跑 DepsTab → 「聲紋辨識 runtime」一鍵裝"})
        sys.exit(2)

    args.output.parent.mkdir(parents=True, exist_ok=True)

    # ── 錄音 ────────────────────────────────────────────────────────────────
    emit({"event": "recording_start", "seconds": args.seconds})
    total_frames = int(args.seconds * args.sample_rate)
    recording = np.zeros((total_frames,), dtype=np.float32)

    # 用 InputStream 邊錄邊報進度(每 0.5s emit 一次)
    written = [0]
    last_progress_emit = [time.time()]

    def callback(indata, frames, time_info, status):
        if status:
            print(f"[enroll] stream status: {status}", file=sys.stderr)
        n = min(frames, total_frames - written[0])
        if n <= 0:
            return
        recording[written[0]:written[0] + n] = indata[:n, 0]
        written[0] += n
        now = time.time()
        if now - last_progress_emit[0] >= 0.5:
            elapsed = written[0] / args.sample_rate
            emit({"event": "recording_progress", "elapsed": round(elapsed, 1)})
            last_progress_emit[0] = now

    stream = sd.InputStream(
        samplerate=args.sample_rate,
        channels=1,
        dtype="float32",
        callback=callback,
    )
    stream.start()
    # Block 直到錄滿
    start = time.time()
    while written[0] < total_frames:
        elapsed = time.time() - start
        if elapsed > args.seconds + 2:  # safety:超過預期 +2s 還沒滿就斷
            break
        time.sleep(0.05)
    stream.stop()
    stream.close()

    if written[0] < total_frames * 0.5:
        emit({"event": "error", "msg": f"錄音不足({written[0]}/{total_frames} samples)"})
        sys.exit(3)

    actual_audio = recording[:written[0]]
    emit({"event": "recording_done"})

    # ── 抽聲紋 ──────────────────────────────────────────────────────────────
    emit({"event": "embedding_start"})
    try:
        # resemblyzer preprocess_wav 需 16kHz mono float32 numpy。Vad-trim 自動做。
        wav_preprocessed = preprocess_wav(actual_audio, source_sr=args.sample_rate)
        encoder = VoiceEncoder(verbose=False)
        embedding = encoder.embed_utterance(wav_preprocessed)
    except Exception as e:
        emit({"event": "error", "msg": f"embedding 失敗:{e}"})
        sys.exit(4)

    # ── 存檔 ────────────────────────────────────────────────────────────────
    np.save(args.output, embedding)
    emit({
        "event": "embedding_done",
        "path": str(args.output),
        "dim": int(embedding.shape[0]),
    })


if __name__ == "__main__":
    main()
