# `mori-wake-train.py` — 訓練自己的 wake-word 模型

Phase 3A.1 — 給 Mori 配上自訂的「Hey Mori」(或任何 phrase)wake-word
detector,搭配 Phase 3A 的 Listening mode 使用。

## 一次性 setup(約 15-20 min)

需要建一個獨立 venv + clone 兩個 repo + 下 Piper voice model。**只做一次**,
之後重訓直接跑 train script。

```bash
# 1. 建 training venv (Python 3.11,因為 piper-phonemize 沒 3.12 wheel)
uv venv ~/.mori/wake-train-venv --python 3.11

# 2. 持久化 clone(放在 ~/.mori/wake-train/,跟 datasets 一起)
mkdir -p ~/.mori/wake-train
cd ~/.mori/wake-train
git clone --depth 1 https://github.com/dscripka/openWakeWord.git
git clone --depth 1 https://github.com/rhasspy/piper-sample-generator.git

# 3. 抓 Piper voice model(~75 MB,英文女聲;之後可換其他 voice)
mkdir -p piper-sample-generator/models
curl -L -o piper-sample-generator/models/en_US-libritts_r-medium.pt \
  https://github.com/rhasspy/piper-sample-generator/releases/download/v2.0.0/en_US-libritts_r-medium.pt

# 4. 裝 training deps 進 train-venv(~5 GB,GPU 版 torch 約佔 3GB)
VIRTUAL_ENV=~/.mori/wake-train-venv uv pip install -e ./openWakeWord
VIRTUAL_ENV=~/.mori/wake-train-venv uv pip install \
  piper-phonemize webrtcvad \
  'mutagen==1.47.0' 'torchinfo==1.8.0' 'torchmetrics==1.2.0' \
  'speechbrain==0.5.14' 'audiomentations==0.33.0' 'torch-audiomentations==0.11.0' \
  'acoustics==0.2.6' 'pronouncing==0.2.0' 'datasets==2.14.6' 'deep-phonemizer==0.0.19' \
  'piper-tts==1.3.0' pytorch-lightning torchaudio

# 5. 部署 train script
cp examples/scripts/mori-wake-train.py ~/.mori/bin/
chmod +x ~/.mori/bin/mori-wake-train.py
```

確認 setup OK:

```bash
~/.mori/wake-train-venv/bin/python ~/.mori/bin/mori-wake-train.py --help
```

## 訓「Hey Mori」

```bash
~/.mori/wake-train-venv/bin/python ~/.mori/bin/mori-wake-train.py "Hey Mori"
```

**第一次跑會自動下載 datasets**(~14 GB 一次性,留著之後重訓不再下載):
- MIT RIRs(房間脈衝響應 ~100 MB)
- AudioSet bal_train09(背景音 ~7 GB)
- ACAV100M precomputed features(~6 GB)
- Validation set features(~1 GB)

下載完跑 3 個 phase:
1. **generate_clips** ~5-10 min:Piper TTS 合成幾千個「Hey Mori」變體(不同口音 / 語速 / 性別)
2. **augment_clips** ~5-10 min:跟 RIR / noise 混音強化 robustness
3. **train_model** ~10-15 min(RTX 4060 Mobile):訓 DNN 分類器

訓完 ONNX 自動 copy 到 `~/.mori/wakeword/hey-mori.onnx`。

第一次總時間估 **~45-60 min**;之後重訓(換 phrase、調參數)**~25-35 min**
(datasets 留著)。

## 套到 Mori

訓完後:

1. Mori 預設 `listening_mode.model_path = ~/.mori/wakeword/hey-mori.onnx`
   train 出的檔名剛好對齊 → 不必改 config
2. Mori-tauri 重啟(或 mode 退進)讓 wake_word.rs 載新 model
3. Tray menu → 「Hey Mori 待命」→ 對麥克風喊「Hey Mori」→ recording 觸發

## 重訓 / 換 phrase

同條 script:

```bash
~/.mori/wake-train-venv/bin/python ~/.mori/bin/mori-wake-train.py "Mori 起床"
# → 訓完 ~/.mori/wakeword/mori-起床.onnx
# → 改 config.json listening_mode.model_path 指到新檔
```

dataset 不重抓(idempotent),只重跑 generate / augment / train(~25-30 min)。

## 調訓練參數

| flag | 預設 | 意義 |
|---|---:|---|
| `--samples` | 10000 | 訓練集正樣本數;高 = 準度好但 generate 久 |
| `--samples-val` | 2000 | 驗證集正樣本 |
| `--steps` | 50000 | 最大訓練步數;早 stop 機制會看 val FP rate |
| `--force-datasets` | off | 重抓 datasets |
| `--force-clips` | off | 重 generate TTS clips(訓練集) |
| `--skip-setup` | off | 跳過 dataset check(假設你已備好) |
| `--output PATH` | `~/.mori/wakeword/<slug>.onnx` | 輸出位置覆蓋 |

## Troubleshooting

**`piper-phonemize` 裝不上**:你用 Python 3.12+,沒 wheel。回 3.11 重建 venv。

**`No module named 'piper_train'`**:`PYTHONPATH` 沒抓到 piper-sample-generator
clone。腳本本身會自動加,但若你直接呼叫 train.py 要手動 `export PYTHONPATH=...`。

**GPU 沒被用到**:torch 沒裝 CUDA build。試
```bash
~/.mori/wake-train-venv/bin/python -c "import torch; print(torch.cuda.is_available())"
```
若 False,重裝 torch 用 CUDA wheel:
```bash
VIRTUAL_ENV=~/.mori/wake-train-venv uv pip install --reinstall \
  torch --index-url https://download.pytorch.org/whl/cu121
```

**Disk full**:datasets ~14 GB + venv ~5 GB + 訓練中間檔 ~3-5 GB,
**至少 25 GB free 才跑得起來**。

**訓出的 model 誤觸太多 / 漏觸太多**:
- 提高 listening_mode.threshold(誤觸多 → 從 0.5 拉到 0.6+)
- 重訓增加 `--samples 30000 --samples-val 5000`(更多樣本準度更高)
- Custom negative phrases — 改 YAML 加 `custom_negative_phrases` 列出像你 phrase 但不該觸發的詞
