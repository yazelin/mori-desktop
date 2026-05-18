#!/usr/bin/env python3
"""mori-wake-verifier.py — 用你自己的聲音訓 wake-word verifier(Phase 3A.1.1)。

Phase 3A.1 用 Piper TTS 合成英文「Hey Mori」訓出的 base model 對 user 實際口音
偵測率低(score 卡在 0.001 ~ 0.3 區間,過不了 threshold)。openWakeWord 設計
的應對方案是「custom verifier」— 用 user 自己錄的 ~20 個正樣本 + ~20 個負
樣本訓一個 voice-specific 微調 model,推論時 base + verifier 兩階段判定,
**對個人聲線命中率大幅提高**。

## 流程

1. Record N positive clips:你連續喊「Hey Mori」(每次 2 秒)
2. Record N negative clips:你講雜七雜八的話(不含「Hey Mori」)
3. 訓練 verifier(~30 秒,scikit-learn `.joblib`)
4. 寫進 ~/.mori/wakeword/hey-mori.verifier.joblib
5. 更新 config.json `listening_mode.verifier_path`

之後 listener 載 base model + verifier,推論流程:
- base model 預測 score(門檻可放低,例 0.1)
- 過門檻 → verifier 二階段確認(是不是 user 本人講的)
- 兩階段都過才 emit wake event

## 用法

  ~/.mori/wake-train-venv/bin/python ~/.mori/bin/mori-wake-verifier.py
  # 互動式:Enter 開始錄、Ctrl+C 中斷

  # 進階:指定樣本數
  python mori-wake-verifier.py --positive 20 --negative 20

## 為什麼用 train-venv 而非 wake-venv

`train_custom_verifier` 在 openwakeword package 內,兩個 venv 都裝了。但 train
邏輯需要 audio features pipeline(melspec.onnx),train-venv 有完整 resources
+ pip-editable openwakeword,跑得穩。
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path

import numpy as np
import scipy.io.wavfile
import sounddevice as sd

HOME = Path.home()
MORI_DIR = HOME / ".mori"
WAKEWORD_DIR = MORI_DIR / "wakeword"
SAMPLES_DIR = WAKEWORD_DIR / "verifier-samples"
DEFAULT_MODEL = WAKEWORD_DIR / "hey-mori.onnx"
DEFAULT_VERIFIER_OUTPUT = WAKEWORD_DIR / "hey-mori.verifier.joblib"
CONFIG_PATH = MORI_DIR / "config.json"

SAMPLE_RATE = 16000
CLIP_DURATION_SECS = 2.0


def log(msg: str, level: str = "info") -> None:
    tag = {"info": "[verifier]", "warn": "[verifier WARN]", "err": "[verifier ERROR]"}[level]
    print(f"{tag} {msg}", flush=True)


def record_clip(out_path: Path, prompt: str) -> None:
    """錄一個 clip。Block 等 user 按 Enter,錄 CLIP_DURATION_SECS 秒。"""
    input(f"\n{prompt}\n  按 Enter 開始錄(2 秒)... ")
    log("錄音中...", "info")
    audio = sd.rec(
        int(CLIP_DURATION_SECS * SAMPLE_RATE),
        samplerate=SAMPLE_RATE,
        channels=1,
        dtype="int16",
    )
    sd.wait()
    audio = audio[:, 0]  # (frames, 1) → (frames,)
    rms = float(np.sqrt(np.mean((audio.astype(np.float32) / 32768.0) ** 2)))
    log(f"錄好(RMS={rms:.4f})", "info")
    if rms < 0.005:
        log("  ⚠ 聲音很小 — mic 距離 / 增益可能不夠,建議重錄這條", "warn")
    scipy.io.wavfile.write(out_path, SAMPLE_RATE, audio)


def collect_samples(label: str, target_dir: Path, count: int, prompts: list[str]) -> None:
    """連續錄 count 條 samples,每條 prompt 從 prompts cycle。"""
    target_dir.mkdir(parents=True, exist_ok=True)
    # 先清舊樣本避免新舊混
    for old in target_dir.glob("*.wav"):
        old.unlink()
    log(f"=== 錄 {count} 條「{label}」樣本 ===")
    for i in range(count):
        prompt = prompts[i % len(prompts)]
        clip_path = target_dir / f"{label}-{i:02d}.wav"
        record_clip(clip_path, f"[{i + 1}/{count}] {prompt}")
    log(f"✓ {count} 條已存到 {target_dir}")


def train_verifier(
    positive_dir: Path,
    negative_dir: Path,
    base_model: Path,
    output_path: Path,
) -> None:
    """呼叫 openwakeword.train_custom_verifier。

    `model_name` 給 base model 的 onnx path(openwakeword 內部會載入它取 features)。

    Upstream bug:函式 type hint 寫 `positive_reference_clips: str` 文件說「directory」,
    但實作 `for i in positive_reference_clips` 直接 iterate string 會拆出字元(`/`, `h`, ...)。
    要傳 list of file paths 才 work。我們在這 glob dir 自己組 list。
    """
    log(f"=== 訓 verifier(base={base_model.name}) ===")
    t0 = time.time()
    from openwakeword import train_custom_verifier

    positive_files = sorted(str(p) for p in positive_dir.glob("*.wav"))
    negative_files = sorted(str(p) for p in negative_dir.glob("*.wav"))
    log(f"  positive: {len(positive_files)} files, negative: {len(negative_files)} files")
    if not positive_files or not negative_files:
        log("no clips found — recording skipped or failed", "err")
        sys.exit(4)

    train_custom_verifier(
        positive_reference_clips=positive_files,
        negative_reference_clips=negative_files,
        output_path=str(output_path),
        model_name=str(base_model),
    )
    log(f"✓ verifier saved: {output_path}({time.time() - t0:.0f}s)")


def update_config(verifier_path: Path) -> None:
    """把 verifier path 寫進 config.json,讓 mori-tauri 的 wake_word.rs 載入。"""
    cfg = json.loads(CONFIG_PATH.read_text()) if CONFIG_PATH.exists() else {}
    cfg.setdefault("listening_mode", {})
    cfg["listening_mode"]["verifier_path"] = str(verifier_path)
    CONFIG_PATH.write_text(json.dumps(cfg, indent=2, ensure_ascii=False))
    log(f"✓ config.json updated: listening_mode.verifier_path = {verifier_path}")


def main() -> None:
    ap = argparse.ArgumentParser(
        description="Train a custom wake-word verifier on your own voice",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    ap.add_argument("--positive", type=int, default=15, help="positive samples to record (default 15)")
    ap.add_argument("--negative", type=int, default=15, help="negative samples to record (default 15)")
    ap.add_argument(
        "--base-model",
        type=Path,
        default=DEFAULT_MODEL,
        help=f"base .onnx model (default {DEFAULT_MODEL})",
    )
    ap.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_VERIFIER_OUTPUT,
        help=f"verifier output path (default {DEFAULT_VERIFIER_OUTPUT})",
    )
    ap.add_argument(
        "--samples-dir",
        type=Path,
        default=SAMPLES_DIR,
        help=f"where to store recorded clips (default {SAMPLES_DIR})",
    )
    ap.add_argument(
        "--skip-record",
        action="store_true",
        help="跳過錄音直接訓(已有 samples_dir/positive + negative 的話)",
    )
    args = ap.parse_args()

    if not args.base_model.exists():
        log(f"base model 不存在:{args.base_model}", "err")
        log("先跑 mori-wake-train.py 訓 base model,或指定 --base-model 路徑", "err")
        sys.exit(2)

    pos_dir = args.samples_dir / "positive"
    neg_dir = args.samples_dir / "negative"

    if not args.skip_record:
        log("=== 階段 1/2:錄 positive 樣本(自己講「Hey Mori」)===")
        log("提示:每次按 Enter 後有 2 秒可講。錄完一條馬上按 Enter 錄下一條。")
        log("變化錄法:大聲 / 小聲 / 快 / 慢 / 平靜 / 興奮 / 距離 mic 遠近")
        positive_prompts = [
            "清楚地說:Hey Mori",
            "稍快一點:Hey Mori",
            "稍慢一點:Hey Mori",
            "輕聲:Hey Mori",
            "正常音量:Hey Mori",
            "離 mic 遠一點:Hey Mori",
            "離 mic 近一點:Hey Mori",
            "尾音上揚(問句):Hey Mori?",
            "平淡:Hey Mori",
            "強調 Mori:Hey MORI",
        ]
        collect_samples("positive", pos_dir, args.positive, positive_prompts)

        log("=== 階段 2/2:錄 negative 樣本(隨便講話、不含「Hey Mori」)===")
        log("提示:講任何句子,中英文都行,只要不是 wake phrase。")
        negative_prompts = [
            "說「今天天氣不錯」",
            "說「我去買咖啡」",
            "說「the quick brown fox」",
            "說「待會打開瀏覽器」",
            "說「給我看看時間」",
            "說「help me find this」",
            "說「來吃個午餐」",
            "說「play music please」",
            "說「等一下我有事」",
            "說隨便一句你想到的話",
        ]
        collect_samples("negative", neg_dir, args.negative, negative_prompts)
    else:
        log("skip recording — 假設 samples 已在 {pos_dir} / {neg_dir}")
        if not pos_dir.is_dir() or not any(pos_dir.glob("*.wav")):
            log(f"沒找到 positive samples 於 {pos_dir}", "err")
            sys.exit(3)
        if not neg_dir.is_dir() or not any(neg_dir.glob("*.wav")):
            log(f"沒找到 negative samples 於 {neg_dir}", "err")
            sys.exit(3)

    # ── 訓 verifier ──────────────────────────────────────────────────
    args.output.parent.mkdir(parents=True, exist_ok=True)
    train_verifier(pos_dir, neg_dir, args.base_model, args.output)

    # ── 更新 config ──────────────────────────────────────────────────
    update_config(args.output)

    log("")
    log("Done! Verifier 訓好了。下一步:")
    log("  1. 重啟 mori-tauri(load 新 config)")
    log("  2. Tray menu → Hey Mori 待命")
    log("  3. 對 mic 喊「Hey Mori」— verifier 會用你自己聲音 fine-tune base model")
    log("")
    log("驗證 verifier 是否載入(從 mori-tauri stderr 看):")
    log("  ┌─ wake-word listener spawned ... verifier=/home/ct/.mori/wakeword/hey-mori.verifier.joblib")


if __name__ == "__main__":
    main()
