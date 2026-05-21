#!/usr/bin/env python3
# MORI_LISTENER_VERSION: 4
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
    try:
        print(json.dumps(obj), flush=True)
    except OSError:
        # parent kill 後 stdout pipe 已關閉,寫入會炸 OSError 22 (Windows) /
        # BrokenPipeError (Linux)。emit 純診斷用途,失敗就吞,別讓 sounddevice
        # callback 把 exception 冒到 cffi 層 → 跳 Python-CFFI error 對話框。
        pass


def diag(line):
    """印一行診斷字到 stderr,失敗吞掉(同 emit 的理由)。Rust 端 stderr drain
    抓 `[wake-listener]` prefix 倒進 event_log JSONL。"""
    try:
        print(f"[wake-listener] {line}", file=sys.stderr, flush=True)
    except OSError:
        pass


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

    # 診斷:把 sounddevice 看到的 default input device + 全 device list 印到 stderr,
    # Windows 乾淨機常踩「sounddevice 預設挑到錯的 mic / 預設 mic 是 stereo mix /
    # 沒 mic」這類沒 error 但 audio 一直靜音的狀況。Rust 端 stderr drain 抓
    # `[wake-listener]` prefix 倒進 event_log JSONL。
    #
    # sd.default.device 在 default in/out idx 不同時(常見:USB mic input + 內建喇叭
    # output),取 `[0]` 會炸 "Input and output device are different"。改成各別
    # 用 `sd.default.device['input']` 拿,避免那條 sounddevice 內部 assert。
    try:
        try:
            input_idx = sd.default.device["input"]
        except Exception:
            input_idx = None
        default_info = sd.query_devices(input_idx) if input_idx is not None else sd.query_devices(kind="input")
        diag(f"default input device: idx={input_idx} info={default_info}")
        all_inputs = [
            (i, d["name"], d["max_input_channels"], d.get("default_samplerate"))
            for i, d in enumerate(sd.query_devices())
            if d["max_input_channels"] > 0
        ]
        diag(f"all input devices: {all_inputs}")
    except Exception as e:
        diag(f"device query failed: {e}")

    emit({"event": "ready", "model": model_path})

    # openWakeWord 接 16kHz mono int16,80ms 一塊(1280 samples)。
    SAMPLE_RATE = 16000
    CHUNK_SAMPLES = 1280

    # 連續觸發抑制 — 一次偵測到 wake 後 1.5s 內不再 emit(避免「Hey Mori Hey
    # Mori」連噴),由 Rust 端管 cooldown 更彈性,但 Python 端基本守一道。
    last_wake_ts = 0.0
    SUPPRESS_SECS = 1.5

    # 診斷:每 ~5s 評估「該不該印 audio level + 最高 score」。
    #
    # v4 起改成「只在有信號時才印」 — idle 期間(沒人講話、mic 沒輸入)完全靜默,
    # 避免 mori log JSONL 被每分鐘 12 筆 `max_score=0.000 max_rms=0.0000` 的空 diag
    # 塞爆。下面兩 gate 任一過就印一行,長期保留診斷價值但不刷屏:
    #
    #   - max_score > DIAG_SCORE_GATE → near-miss(像 wake-word 但沒過 threshold)
    #   - max_rms   > DIAG_RMS_GATE   → mic 真的有收到聲音(idle 噪音 < 0.001,
    #                                   有人說話 > 0.02),確認 mic 沒死
    diag_state = {"counter": 0, "max_score": 0.0, "max_rms": 0.0}
    DIAG_EVERY_CHUNKS = 62  # 62 * 80ms ≈ 5s 評估一次
    DIAG_SCORE_GATE = 0.30  # near-miss 門檻(threshold 0.55 的下半)
    DIAG_RMS_GATE = 0.02    # 有人說話的音量門檻

    def callback(indata, frames, time_info, status):
        nonlocal last_wake_ts
        # indata: shape (frames, 1) int16
        audio = indata[:, 0]
        predictions = model.predict(audio)
        # 累積本輪 5s 內的 max score + max RMS(int16 → /32768 normalize 到 [0,1])
        rms = float(np.sqrt(np.mean(audio.astype(np.float64) ** 2))) / 32768.0
        diag_state["max_rms"] = max(diag_state["max_rms"], rms)
        diag_state["counter"] += 1
        for word, score in predictions.items():
            s = float(score)
            if s > diag_state["max_score"]:
                diag_state["max_score"] = s
            if s >= threshold:
                now = time.time()
                if now - last_wake_ts < SUPPRESS_SECS:
                    continue  # 抑制連續觸發
                last_wake_ts = now
                emit({"event": "wake", "word": word, "score": s})
        if diag_state["counter"] >= DIAG_EVERY_CHUNKS:
            should_emit = (
                diag_state["max_score"] > DIAG_SCORE_GATE
                or diag_state["max_rms"] > DIAG_RMS_GATE
            )
            # 用 helper 而非 raw print — parent kill 後 stderr pipe 關掉,直接 print
            # 會在 sounddevice 的 cffi callback 內冒 OSError 22 → Windows 跳
            # "Python-CFFI error" 對話框,擋使用者 Mori 結束流程。helper 吞 OSError。
            if should_emit:
                diag(
                    f"5s diag: max_score={diag_state['max_score']:.3f} "
                    f"(threshold={threshold}) max_rms={diag_state['max_rms']:.4f} "
                    f"(0=silent, normal speech ~0.05-0.2)"
                )
            diag_state["counter"] = 0
            diag_state["max_score"] = 0.0
            diag_state["max_rms"] = 0.0

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
