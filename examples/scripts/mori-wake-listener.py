#!/usr/bin/env python3
"""mori-wake-listener.py — Hey Mori wake-word detector(Phase 3A)。

mori-tauri 在 Listening mode 下會 spawn 這隻 script,持續開麥克風跑
openWakeWord 偵測 wake phrase。偵測到 → stdout 印 line-delimited JSON
event,mori-tauri 端 background thread 讀取觸發 recording pipeline。

## stdout protocol (Rust 端 wake_word.rs 對應 WakeEvent enum)

  {"event":"ready", "model":"/path/to/model.onnx"}
  {"event":"wake", "word":"hey_mori", "score":0.81}
  {"event":"error", "msg":"<reason>"}

stderr 給 diagnostic / openwakeword 內部 log。**永遠別印非 JSON 到 stdout**,
會打破 Rust 端 parse。

## CLI

  mori-wake-listener.py <model_path> [threshold]

  model_path  — openWakeWord `.onnx` 檔路徑(pre-trained 或自訓)
  threshold   — detection threshold 0~1,預設 0.5

## 安裝

  cp examples/scripts/mori-wake-listener.py ~/.mori/bin/
  chmod +x ~/.mori/bin/mori-wake-listener.py

  # openwakeword 依賴 — 用 uv 管 isolated venv 避免污染系統
  uv tool install openwakeword
  # 或 pip install openwakeword

  # Model — 自訓「Hey Mori」需 GPU + 數小時;先用 openWakeWord 內建的
  # 「hey jarvis」之類 placeholder 驗 pipeline 通,後期再換 custom。
  # https://github.com/dscripka/openWakeWord#pre-trained-models
"""

import json
import sys
import time
from pathlib import Path

# Import 失敗時要 emit error event 給 mori-tauri,不能直接 raise。
def emit(obj):
    print(json.dumps(obj), flush=True)


def main():
    if len(sys.argv) < 2:
        emit({"event": "error", "msg": "missing model_path argument"})
        sys.exit(1)
    model_path = sys.argv[1]
    threshold = float(sys.argv[2]) if len(sys.argv) > 2 else 0.5
    # 第三個 arg(可選):custom verifier `.joblib` 路徑。傳了就用 base + verifier 兩階段
    # 判定 — base 過(可放低 threshold)再 verifier 過(對 user 個人聲線 fine-tuned)。
    verifier_path = sys.argv[3] if len(sys.argv) > 3 else None

    if not Path(model_path).exists():
        emit({"event": "error", "msg": f"model not found: {model_path}"})
        sys.exit(2)
    if verifier_path and not Path(verifier_path).exists():
        emit({"event": "error", "msg": f"verifier not found: {verifier_path}"})
        sys.exit(2)

    try:
        from openwakeword import Model
        import sounddevice as sd
        import numpy as np
    except ImportError as e:
        emit({"event": "error", "msg": f"missing dep: {e}. Run: pip install openwakeword sounddevice"})
        sys.exit(3)

    try:
        # openWakeWord 0.4+ API:kwarg 是 wakeword_model_paths(複數 + _paths 後綴)
        kwargs = {"wakeword_model_paths": [model_path]}
        if verifier_path:
            # custom_verifier_models 的 key 對應 wake-word name(從 model filename stem 拿,
            # 例 hey-mori.onnx → "hey-mori"),value 是 .joblib 路徑
            ww_name = Path(model_path).stem
            kwargs["custom_verifier_models"] = {ww_name: verifier_path}
        model = Model(**kwargs)
    except Exception as e:
        emit({"event": "error", "msg": f"model load failed: {e}"})
        sys.exit(4)

    emit({"event": "ready", "model": model_path})

    # openWakeWord 接 16kHz mono int16,80ms 一塊(1280 samples)。
    SAMPLE_RATE = 16000
    CHUNK_SAMPLES = 1280

    # 連續觸發抑制 — 一次偵測到 wake 後 1.5s 內不再 emit(避免「Hey Mori Hey
    # Mori」連噴),由 Rust 端管 cooldown 更彈性,但 Python 端基本守一道。
    last_wake_ts = 0.0
    SUPPRESS_SECS = 1.5

    def callback(indata, frames, time_info, status):
        nonlocal last_wake_ts
        # indata: shape (frames, 1) int16
        audio = indata[:, 0]
        predictions = model.predict(audio)
        for word, score in predictions.items():
            if score >= threshold:
                now = time.time()
                if now - last_wake_ts < SUPPRESS_SECS:
                    continue  # 抑制連續觸發
                last_wake_ts = now
                emit({"event": "wake", "word": word, "score": float(score)})

    try:
        with sd.InputStream(
            samplerate=SAMPLE_RATE,
            channels=1,
            dtype="int16",
            blocksize=CHUNK_SAMPLES,
            callback=callback,
        ):
            # 主執行緒 idle 等 keyboard interrupt / parent kill。
            while True:
                time.sleep(0.5)
    except KeyboardInterrupt:
        pass
    except Exception as e:
        emit({"event": "error", "msg": f"audio stream failed: {e}"})
        sys.exit(5)


if __name__ == "__main__":
    main()
