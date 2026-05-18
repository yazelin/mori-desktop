#!/usr/bin/env python3
"""mori-wake-train.py — 訓練 Mori 自己的 wake-word 模型(Phase 3A.1)。

把 openWakeWord 的 automatic_model_training notebook 包成一條 CLI。
給 phrase + 等 ~30-50 分鐘,輸出 .onnx 到 ~/.mori/wakeword/<phrase>.onnx,
Listening mode 立即 pick up。重訓 / 換 phrase 都跑同一條:

  ~/.mori/wake-train-venv/bin/python ~/.mori/bin/mori-wake-train.py "Hey Mori"
  ~/.mori/wake-train-venv/bin/python ~/.mori/bin/mori-wake-train.py "Mori 起床"
  ~/.mori/wake-train-venv/bin/python ~/.mori/bin/mori-wake-train.py "Hey Mori" --output /tmp/test.onnx

## 設計

Idempotent — 已下載 dataset / 已 clone repo / 已 generate clips 都會 skip。
要強制重做用 --force-{datasets,clips}。

## 目錄佈局

  ~/.mori/wake-train/
    openWakeWord/            ← upstream clone(訓練腳本 + train.py 在這)
    piper-sample-generator/  ← TTS clone(產合成「Hey Mori」音檔)
    datasets/                ← 持久 dataset(只下載一次)
      mit_rirs/                  房間脈衝響應 ~100 MB
      audioset_16k/              背景音(AudioSet bal_train09)~7 GB
      fma/                       FMA small music dataset ~7 GB
      openwakeword_features_ACAV100M_2000_hrs_16bit.npy  pre-computed features ~6 GB
      validation_set_features.npy                        ~1 GB
    runs/<phrase_slug>/      ← 每個 phrase 的訓練輸出
      my_model/my_model.onnx

訓完 .onnx 會 copy 一份到 ~/.mori/wakeword/<phrase_slug>.onnx。

## 為什麼用 Python 不是 bash

訓練流程需要(a) 改 YAML config(b) 呼叫 train.py 3 次 different args
(c) 處理路徑 / sanitize phrase / cp 檔。Python 寫比 bash 簡潔很多。
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

HOME = Path.home()
MORI_DIR = HOME / ".mori"
WAKE_TRAIN_DIR = MORI_DIR / "wake-train"
OWAKE_DIR = WAKE_TRAIN_DIR / "openWakeWord"
PIPER_DIR = WAKE_TRAIN_DIR / "piper-sample-generator"
DATASETS_DIR = WAKE_TRAIN_DIR / "datasets"
RUNS_DIR = WAKE_TRAIN_DIR / "runs"
WAKEWORD_DIR = MORI_DIR / "wakeword"
VENV_PYTHON = MORI_DIR / "wake-train-venv" / "bin" / "python"

# ── Datasets URLs(對齊 notebook cells 8-10) ─────────────────────────
AUDIOSET_URL = (
    "https://huggingface.co/datasets/agkphysics/AudioSet/resolve/main/data/bal_train09.tar"
)
ACAV_FEATURES_URL = (
    "https://huggingface.co/datasets/davidscripka/openwakeword_features/resolve/main/"
    "openwakeword_features_ACAV100M_2000_hrs_16bit.npy"
)
VALIDATION_FEATURES_URL = (
    "https://huggingface.co/datasets/davidscripka/openwakeword_features/resolve/main/"
    "validation_set_features.npy"
)


def log(msg: str, level: str = "info") -> None:
    """Tagged stderr log,主要 stdout 留給 train.py 自己的 progress / debug。"""
    tag = {"info": "[mori-wake-train]", "warn": "[mori-wake-train WARN]", "err": "[mori-wake-train ERROR]"}.get(level, "[mori-wake-train]")
    print(f"{tag} {msg}", file=sys.stderr, flush=True)


def slug(phrase: str) -> str:
    """phrase → safe filename slug。「Hey Mori」→ hey-mori、「Mori 起床」→ mori-起床。"""
    s = phrase.strip().lower().replace(" ", "-")
    s = re.sub(r"[^\w一-鿿-]", "", s)
    return s or "wakeword"


def run(cmd: list[str], cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    """跑 subprocess,失敗 raise(讓 main 印錯誤 + exit)。"""
    log(f"$ {' '.join(str(c) for c in cmd)}" + (f"  (cwd={cwd})" if cwd else ""))
    subprocess.check_call(cmd, cwd=cwd, env=env)


def ensure_venv() -> None:
    """檢查 train-venv 在 + 跑的就是它。"""
    if not VENV_PYTHON.exists():
        log(
            f"train-venv 不存在:{VENV_PYTHON}\n"
            "請先建好 venv + 裝 training deps。詳見 examples/scripts/README.wake-train.md",
            "err",
        )
        sys.exit(2)
    # 確認當前 interpreter 是這個 venv(scripts inside venv 才有 piper_train / openwakeword editable)
    if Path(sys.executable).resolve() != VENV_PYTHON.resolve():
        log(
            f"this script must be run with the wake-train-venv python:\n"
            f"  {VENV_PYTHON} {' '.join(sys.argv)}",
            "err",
        )
        sys.exit(3)


def ensure_clones() -> None:
    """確認 openWakeWord + piper-sample-generator clone 在(setup 階段已 clone,本 script
    不負責 clone — setup README 帶 user 跑過)。"""
    for name, path in [("openWakeWord", OWAKE_DIR), ("piper-sample-generator", PIPER_DIR)]:
        if not path.is_dir():
            log(
                f"{name} clone 不存在:{path}\n"
                "請依 README.wake-train.md setup 步驟先 clone。",
                "err",
            )
            sys.exit(4)


def download(url: str, dest: Path, force: bool = False) -> None:
    """curl 下載到 dest,resumeable(curl -C -)。force=True 重抓。"""
    if dest.exists() and not force and dest.stat().st_size > 1024:
        log(f"skip:{dest.name} 已存在({dest.stat().st_size // (1024*1024)} MB)")
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    log(f"downloading {url} → {dest}")
    run(["curl", "-L", "--progress-bar", "-C", "-", "-o", str(dest), url])


def ensure_mit_rirs(force: bool = False) -> None:
    """MIT RIRs 走 datasets library streaming(notebook cell 8)。"""
    out = DATASETS_DIR / "mit_rirs"
    if out.is_dir() and any(out.iterdir()) and not force:
        log(f"skip:mit_rirs 已存在({sum(1 for _ in out.iterdir())} files)")
        return
    out.mkdir(parents=True, exist_ok=True)
    log("downloading MIT RIRs(via huggingface datasets streaming, ~100 MB)")
    # 用 train-venv 跑(需要 datasets / scipy / numpy)
    snippet = f"""
import datasets, scipy.io.wavfile, numpy as np, os
out_dir = {str(out)!r}
ds = datasets.load_dataset('davidscripka/MIT_environmental_impulse_responses', split='train', streaming=True)
for i, row in enumerate(ds):
    name = row['audio']['path'].split('/')[-1]
    scipy.io.wavfile.write(os.path.join(out_dir, name), 16000, (row['audio']['array']*32767).astype(np.int16))
    if i % 50 == 0:
        print(f'  {{i}} done', flush=True)
print('done', flush=True)
"""
    run([sys.executable, "-c", snippet])


def ensure_audioset(force: bool = False) -> None:
    """AudioSet bal_train09.tar(~7 GB)+ 轉 16kHz wav(notebook cell 9 上半段)。"""
    tarpath = DATASETS_DIR / "audioset" / "bal_train09.tar"
    audio16k = DATASETS_DIR / "audioset_16k"
    if audio16k.is_dir() and any(audio16k.glob("*.wav")) and not force:
        log(f"skip:audioset_16k 已存在({sum(1 for _ in audio16k.glob('*.wav'))} files)")
        return

    download(AUDIOSET_URL, tarpath, force=force)
    log("extracting bal_train09.tar")
    run(["tar", "-xf", tarpath.name], cwd=tarpath.parent)

    log("converting AudioSet flac → 16kHz wav")
    snippet = f"""
import datasets, scipy.io.wavfile, numpy as np, os
from pathlib import Path
audioset_dir = {str(tarpath.parent)!r}
out_dir = {str(audio16k)!r}
os.makedirs(out_dir, exist_ok=True)
files = list(Path(audioset_dir).glob('audio/**/*.flac'))
print(f'found {{len(files)}} flac', flush=True)
ds = datasets.Dataset.from_dict({{'audio': [str(f) for f in files]}})
ds = ds.cast_column('audio', datasets.Audio(sampling_rate=16000))
for i, row in enumerate(ds):
    name = row['audio']['path'].split('/')[-1].replace('.flac', '.wav')
    scipy.io.wavfile.write(os.path.join(out_dir, name), 16000, (row['audio']['array']*32767).astype(np.int16))
    if i % 100 == 0:
        print(f'  {{i}} done', flush=True)
print('done', flush=True)
"""
    run([sys.executable, "-c", snippet])


def ensure_features(force: bool = False) -> None:
    """ACAV100M precomputed features(~6 GB)+ validation set(~1 GB)— notebook cell 10。"""
    acav = DATASETS_DIR / "openwakeword_features_ACAV100M_2000_hrs_16bit.npy"
    val = DATASETS_DIR / "validation_set_features.npy"
    download(ACAV_FEATURES_URL, acav, force=force)
    download(VALIDATION_FEATURES_URL, val, force=force)


def write_yaml(phrase: str, run_dir: Path, n_samples: int, n_val: int, steps: int) -> Path:
    """產 YAML config 寫到 run_dir/my_model.yaml,給 train.py --training_config 用。

    模板對齊 openWakeWord/examples/custom_model.yml,改:
    - target_phrase = [phrase]
    - n_samples / n_samples_val / steps
    - output_dir = run_dir
    - dataset 路徑全部絕對(避免 cd 問題)
    """
    cfg_yaml = f"""# Auto-generated by mori-wake-train.py
model_name: "my_model"
target_phrase:
  - "{phrase}"
custom_negative_phrases: []
n_samples: {n_samples}
n_samples_val: {n_val}
tts_batch_size: 50
augmentation_batch_size: 16
piper_sample_generator_path: "{PIPER_DIR}"
output_dir: "{run_dir}"
rir_paths:
  - "{DATASETS_DIR / 'mit_rirs'}"
background_paths:
  - "{DATASETS_DIR / 'audioset_16k'}"
background_paths_duplication_rate:
  - 1
false_positive_validation_data_path: "{DATASETS_DIR / 'validation_set_features.npy'}"
augmentation_rounds: 1
feature_data_files:
  ACAV100M_sample: "{DATASETS_DIR / 'openwakeword_features_ACAV100M_2000_hrs_16bit.npy'}"
batch_n_per_class:
  ACAV100M_sample: 1024
  adversarial_negative: 50
  positive: 50
model_type: "dnn"
layer_size: 32
steps: {steps}
max_negative_weight: 1500
target_false_positives_per_hour: 0.2
"""
    cfg_path = run_dir / "my_model.yaml"
    cfg_path.write_text(cfg_yaml)
    log(f"wrote training config: {cfg_path}")
    return cfg_path


def run_train(cfg: Path, flag: str) -> None:
    """跑 openWakeWord/openwakeword/train.py 一個 phase。flag = generate_clips / augment_clips / train_model"""
    train_py = OWAKE_DIR / "openwakeword" / "train.py"
    if not train_py.exists():
        log(f"train.py 不存在:{train_py}", "err")
        sys.exit(5)
    log(f"=== phase: {flag} ===")
    t0 = time.time()
    # 加 piper-sample-generator path 進 PYTHONPATH,讓 train.py 內 import piper_train work
    env = os.environ.copy()
    env["PYTHONPATH"] = str(PIPER_DIR) + os.pathsep + env.get("PYTHONPATH", "")
    run(
        [sys.executable, str(train_py), "--training_config", str(cfg), f"--{flag}"],
        cwd=OWAKE_DIR,
        env=env,
    )
    log(f"=== phase {flag} took {time.time() - t0:.0f}s ===")


def main() -> None:
    ap = argparse.ArgumentParser(
        description="Train a custom openWakeWord model for Mori",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    ap.add_argument("phrase", help='wake phrase, e.g. "Hey Mori"')
    ap.add_argument(
        "--output",
        type=Path,
        default=None,
        help="output .onnx path (default: ~/.mori/wakeword/<phrase_slug>.onnx)",
    )
    ap.add_argument("--samples", type=int, default=10_000, help="positive samples to generate (default 10000)")
    ap.add_argument("--samples-val", type=int, default=2_000, help="validation samples (default 2000)")
    ap.add_argument("--steps", type=int, default=50_000, help="max training steps (default 50000)")
    ap.add_argument("--force-datasets", action="store_true", help="re-download datasets")
    ap.add_argument("--force-clips", action="store_true", help="re-generate TTS clips")
    ap.add_argument("--skip-setup", action="store_true", help="skip dataset/clone checks (assume ready)")
    args = ap.parse_args()

    log(f"phrase = {args.phrase!r}")
    log(f"samples train={args.samples} val={args.samples_val} steps={args.steps}")

    # ── 0. Pre-flight checks ───────────────────────────────────────
    ensure_venv()
    ensure_clones()

    # ── 1. Datasets ────────────────────────────────────────────────
    if not args.skip_setup:
        log("[1/4] checking / downloading datasets")
        ensure_mit_rirs(force=args.force_datasets)
        ensure_audioset(force=args.force_datasets)
        ensure_features(force=args.force_datasets)
    else:
        log("[1/4] skipped (--skip-setup)")

    # ── 2. Write YAML config + create run dir ──────────────────────
    phrase_slug = slug(args.phrase)
    run_dir = RUNS_DIR / phrase_slug
    run_dir.mkdir(parents=True, exist_ok=True)

    if args.force_clips:
        clips_dir = run_dir / "my_model"
        if clips_dir.exists():
            log(f"--force-clips: removing {clips_dir}")
            shutil.rmtree(clips_dir)

    cfg = write_yaml(
        args.phrase,
        run_dir=run_dir,
        n_samples=args.samples,
        n_val=args.samples_val,
        steps=args.steps,
    )

    # ── 3. Train (3 phases) ────────────────────────────────────────
    log("[2/4] generating TTS clips (Piper)")
    run_train(cfg, "generate_clips")

    log("[3/4] augmenting clips (noise / RIR mix)")
    run_train(cfg, "augment_clips")

    log("[4/4] training model")
    run_train(cfg, "train_model")

    # ── 4. Copy .onnx → ~/.mori/wakeword/ ──────────────────────────
    src_onnx = run_dir / "my_model" / "my_model.onnx"
    if not src_onnx.exists():
        log(f"trained model not found at {src_onnx}", "err")
        sys.exit(6)
    WAKEWORD_DIR.mkdir(parents=True, exist_ok=True)
    dest = args.output if args.output else WAKEWORD_DIR / f"{phrase_slug}.onnx"
    shutil.copy2(src_onnx, dest)
    log(f"✓ wake-word model installed: {dest}")
    log(f"  size: {dest.stat().st_size // 1024} KB")
    log("")
    log("Done! 若要立即啟用,把 ~/.mori/config.json 的 listening_mode.model_path")
    log(f"指向: {dest}")
    log("然後在 Mori tray menu 切「Hey Mori 待命」即可。")


if __name__ == "__main__":
    main()
