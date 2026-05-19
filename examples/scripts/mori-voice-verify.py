#!/usr/bin/env python3
"""mori-voice-verify.py — 驗證 incoming 聲音是不是 enrolled user(Phase 3E)。

mori-tauri 在每次 wake event 後,user 講完 + STT 之前(或之後)呼叫這個 script:
- 給 enrolled embedding `.npy` + recording `.wav`
- 回 cosine similarity score(0-1)+ pass/fail 判定

## 用法

  python mori-voice-verify.py <user-embedding.npy> <audio.wav> [--threshold 0.7]

## stdout protocol(JSON,單行)

  {"score": 0.85, "pass": true, "threshold": 0.7}
  {"error": "..."}

  Rust 端 parse → score < threshold 就 silent reject 不送 agent。

## 為什麼用 audio file 不用 stdin pipe raw audio

WAV file API 跨 Python/Rust 最穩,沒 byte-order / endianness / sample-rate
誤判風險。recording.rs 已會寫 WAV,直接把暫存路徑 passthrough 即可。
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def emit(obj: dict) -> None:
    print(json.dumps(obj), flush=True)


def main() -> None:
    ap = argparse.ArgumentParser(description="Verify incoming voice matches enrolled user")
    ap.add_argument("user_embedding", type=Path, help="enrolled user .npy(從 mori-voice-enroll.py 產出)")
    ap.add_argument("audio", type=Path, help="incoming recording .wav")
    ap.add_argument("--threshold", type=float, default=0.7, help="cosine threshold(0~1,預設 0.7)")
    args = ap.parse_args()

    if not args.user_embedding.exists():
        emit({"error": f"user embedding 不存在:{args.user_embedding}(先跑 enrollment)"})
        sys.exit(2)
    if not args.audio.exists():
        emit({"error": f"audio file 不存在:{args.audio}"})
        sys.exit(2)

    try:
        import numpy as np
        from resemblyzer import VoiceEncoder, preprocess_wav
    except ImportError as e:
        emit({"error": f"missing dep: {e}. 跑 DepsTab → 「聲紋辨識 runtime」"})
        sys.exit(3)

    try:
        user_emb = np.load(args.user_embedding)
    except Exception as e:
        emit({"error": f"讀 embedding 失敗:{e}"})
        sys.exit(4)

    try:
        # preprocess_wav 接 wav file path 或 numpy array;會自動 resample 到 16kHz mono +
        # VAD trim silence(對短錄音很有幫助)
        wav = preprocess_wav(args.audio)
        encoder = VoiceEncoder(verbose=False)
        incoming_emb = encoder.embed_utterance(wav)
    except Exception as e:
        emit({"error": f"embedding 失敗:{e}"})
        sys.exit(5)

    # resemblyzer embeddings 已 L2-normalized,dot product = cosine similarity
    score = float(np.dot(user_emb, incoming_emb))
    passed = score >= args.threshold

    emit({
        "score": round(score, 4),
        "pass": passed,
        "threshold": args.threshold,
    })


if __name__ == "__main__":
    main()
