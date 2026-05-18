#!/usr/bin/env python3
"""mori-tts-bake.py — 用 Gemini 2.5 TTS 預先 bake Mori 的 wake-ack 短句。

Phase 3A.1.2 — wake event 觸發時要播一段 Mori 的應答音(「我在喔」/「Mori 在」),
讓 user 不用盯畫面就知道「可以開始講指令了」。每次都 call TTS round-trip 太慢
(~500-1500ms),改成一次 bake 出 N 個 wav 存在 disk,wake 時隨機選一個播。

## 用法

  python3 examples/scripts/mori-tts-bake.py
  # → ~/.mori/wakeword/sounds/wake-ack-samples/<voice>/<idx>-<phrase>.wav
  # 預設生 3 voice × 8 phrase = 24 個試聽檔

  python3 examples/scripts/mori-tts-bake.py --voice Aoede --phrase "嗯,我在聽"
  # 單一 voice + 單一 phrase

  python3 examples/scripts/mori-tts-bake.py --output ~/.mori/wakeword/sounds/wake-ack/
  # 直接 bake 到正式資料夾(挑完聲音後跑)

## API key

從 `~/.mori/config.json` `api_keys.GEMINI_API_KEY` 拿。或環境變數 GEMINI_API_KEY
override。

## Gemini TTS endpoint

  POST https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-tts:generateContent

回 base64 PCM16 mono 24kHz raw audio,要自己包 WAV header。
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import struct
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HOME = Path.home()
MORI_DIR = HOME / ".mori"
CONFIG_PATH = MORI_DIR / "config.json"
DEFAULT_OUTPUT = MORI_DIR / "wakeword" / "sounds" / "wake-ack-samples"

# Gemini 2.5 TTS — preview API,response 是 PCM16 24kHz mono
TTS_MODEL = "gemini-2.5-flash-preview-tts"
TTS_ENDPOINT = (
    f"https://generativelanguage.googleapis.com/v1beta/models/{TTS_MODEL}:generateContent"
)

# Gemini 30 prebuilt voices 性別權威表(對齊 ching-tech-os/extends/voice/voice_tts.py)
# 之前憑「character 描述詞」推測害我推了一堆男聲給 Mori(精靈少女),所以這裡寫死。
GEMINI_FEMALE_VOICES = {
    "Achernar", "Aoede", "Autonoe", "Callirrhoe", "Despina", "Erinome",
    "Gacrux", "Kore", "Laomedeia", "Leda", "Sulafat", "Zephyr",
    "Pulcherrima", "Vindemiatrix",
}
GEMINI_MALE_VOICES = {
    "Achird", "Algenib", "Algieba", "Alnilam", "Charon", "Enceladus",
    "Fenrir", "Iapetus", "Orus", "Puck", "Rasalgethi", "Sadachbia",
    "Sadaltager", "Schedar", "Umbriel", "Zubenelgenubi",
}

# Voice 候選 — Mori 形象(綠髮花飾精靈少女,SOUL.md 「直接、會嘴、不囉嗦」),
# user 2026-05-19 拍板「聰明的小朋友 + 比少女再小一點點」向。
# 確認偏好:Leda(youthful)、Erinome(clear)。剩待試女聲(避免重蹈推男聲覆轍):
#   Callirrhoe — easy-going(輕鬆不嚴肅)
#   Zephyr     — bright
#   Pulcherrima — forward(主動有 backbone)
VOICES = ["Leda", "Erinome", "Callirrhoe", "Zephyr", "Pulcherrima"]

# Wake-ack 短句(TW 用語、避「森林」字眼,user 2026-05-19 確認)
PHRASES = [
    "嗯?",
    "我在",
    "我在喔",
    "Mori 在",
    "嗯,我在聽",
    "我聽見了~",
    "怎麼了?",
    "想跟我說什麼?",
]

# Style prompt 前綴 — Gemini TTS docs 範例格式是「Say <style>: <text>」短英文。
# 中文長 style 會被模型誤認成「對話內容」(finish_reason=OTHER + 沒 audio),
# 必須短指令冒號 prefix。user 2026-05-19 取向:「日系動漫女主、異世界女神精靈」。
STYLE_PROMPT = "Say warmly and brightly like a cheerful anime-heroine"


def log(msg: str, level: str = "info") -> None:
    tag = {"info": "[bake]", "warn": "[bake WARN]", "err": "[bake ERROR]"}[level]
    print(f"{tag} {msg}", flush=True)


def load_api_key() -> str:
    """從環境變數或 config.json 拿 GEMINI_API_KEY。"""
    key = os.environ.get("GEMINI_API_KEY")
    if key:
        return key
    if not CONFIG_PATH.exists():
        log(f"config 不存在:{CONFIG_PATH}", "err")
        sys.exit(2)
    try:
        cfg = json.loads(CONFIG_PATH.read_text())
    except json.JSONDecodeError as e:
        log(f"config.json 不是合法 JSON:{e}", "err")
        sys.exit(2)
    key = cfg.get("api_keys", {}).get("GEMINI_API_KEY")
    if not key:
        log("找不到 GEMINI_API_KEY(環境變數沒設、config.json `api_keys.GEMINI_API_KEY` 也沒設)", "err")
        sys.exit(2)
    return key


def gen_speech(api_key: str, text: str, voice: str, style_prompt: str) -> bytes:
    """call Gemini TTS,return raw PCM16 mono 24kHz bytes。"""
    # Docs 格式:`Say <style>: <text>`(英文冒號 + 空格)。中文冒號或長 prompt
    # 會讓模型走 chat 模式回 text response,API 報 finish_reason=OTHER 沒 audio。
    prompt = f"{style_prompt}: {text}" if style_prompt else text
    body = {
        "contents": [{"parts": [{"text": prompt}]}],
        "generationConfig": {
            "responseModalities": ["AUDIO"],
            "speechConfig": {
                "voiceConfig": {
                    "prebuiltVoiceConfig": {"voiceName": voice}
                }
            },
        },
    }
    data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(
        f"{TTS_ENDPOINT}?key={api_key}",
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            payload = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        err_body = e.read().decode("utf-8", errors="replace")
        log(f"HTTP {e.code} — {err_body[:500]}", "err")
        raise
    candidates = payload.get("candidates", [])
    if not candidates:
        raise RuntimeError(f"no candidates in response: {payload}")
    parts = candidates[0].get("content", {}).get("parts", [])
    for part in parts:
        inline = part.get("inlineData") or part.get("inline_data")
        if inline and "data" in inline:
            return base64.b64decode(inline["data"])
    raise RuntimeError(f"no inline audio data in response: {payload}")


def wrap_wav(pcm: bytes, sample_rate: int = 24000) -> bytes:
    """把 raw PCM16 mono bytes 包成 WAV(加 44-byte RIFF header)。"""
    num_samples = len(pcm) // 2
    byte_rate = sample_rate * 2  # mono, 16-bit
    block_align = 2
    data_size = num_samples * 2
    fmt_chunk = struct.pack(
        "<4sIHHIIHH",
        b"fmt ",
        16,
        1,  # PCM
        1,  # mono
        sample_rate,
        byte_rate,
        block_align,
        16,  # bits per sample
    )
    data_chunk = struct.pack("<4sI", b"data", data_size) + pcm
    riff_size = 4 + len(fmt_chunk) + len(data_chunk)
    riff = struct.pack("<4sI4s", b"RIFF", riff_size, b"WAVE") + fmt_chunk + data_chunk
    return riff


def safe_filename(text: str) -> str:
    """phrase → 檔名 friendly slug。中文保留,只拔掉 fs 不友善的字元。"""
    bad = set('/\\:*?"<>|\0')
    cleaned = "".join("_" if c in bad else c for c in text)
    return cleaned.strip().replace(" ", "_")


def bake(
    api_key: str,
    voices: list[str],
    phrases: list[str],
    style_prompt: str,
    output_dir: Path,
    flat: bool = False,
) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    total = len(voices) * len(phrases)
    done = 0
    t_start = time.time()
    for voice in voices:
        v_dir = output_dir if flat else output_dir / voice
        v_dir.mkdir(parents=True, exist_ok=True)
        for idx, phrase in enumerate(phrases):
            slug = safe_filename(phrase)
            fname = f"{idx:02d}-{slug}.wav" if not flat else f"{voice}-{idx:02d}-{slug}.wav"
            out = v_dir / fname
            done += 1
            log(f"[{done}/{total}] {voice} / {phrase!r} → {out.name}")
            try:
                pcm = gen_speech(api_key, phrase, voice, style_prompt)
            except Exception as e:
                log(f"  failed: {e}", "warn")
                continue
            out.write_bytes(wrap_wav(pcm))
    log(f"done — {done}/{total} 個試聽檔({time.time() - t_start:.1f}s)")
    log(f"輸出位置:{output_dir}")
    if not flat:
        log("試聽方式:")
        log(f"  ls {output_dir}/*/")
        log(f"  paplay {output_dir}/Aoede/00-嗯_.wav   # 換檔名/voice 試各個")
        log("選好喜歡的,搬進正式資料夾:")
        log("  mkdir -p ~/.mori/wakeword/sounds/wake-ack")
        log(f"  cp {output_dir}/<voice>/*.wav ~/.mori/wakeword/sounds/wake-ack/")


def main() -> None:
    ap = argparse.ArgumentParser(
        description="Bake Mori wake-ack short phrases via Gemini 2.5 TTS",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    ap.add_argument(
        "--voice",
        action="append",
        help=f"voice name(可重複)。預設全試 {VOICES}",
    )
    ap.add_argument(
        "--phrase",
        action="append",
        help="phrase 文字(可重複)。預設用內建 8 句 TW 短應答",
    )
    ap.add_argument(
        "--style",
        default=STYLE_PROMPT,
        help=f"style prompt(預設「{STYLE_PROMPT}」)。空字串 = 不加 prefix",
    )
    ap.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT,
        help=f"輸出資料夾(預設 {DEFAULT_OUTPUT})",
    )
    ap.add_argument(
        "--flat",
        action="store_true",
        help="平鋪所有檔到 --output(不分 voice 子資料夾)。挑完聲音 bake 到正式資料夾時用",
    )
    args = ap.parse_args()

    voices = args.voice or VOICES
    # 防呆:user 自己 --voice 傳了不在女聲表 / 不存在的名字 → 警告但不擋
    for v in voices:
        if v in GEMINI_MALE_VOICES:
            log(f"voice '{v}' 是男聲,Mori 是精靈少女,確定要嗎?", "warn")
        elif v not in GEMINI_FEMALE_VOICES:
            log(f"voice '{v}' 不在 Gemini 30 voices 表內,API 可能會 reject", "warn")
    phrases = args.phrase or PHRASES
    style = args.style if args.style else ""

    log(f"voices = {voices}")
    log(f"phrases = {phrases}")
    log(f"style = {style!r}")
    log(f"output = {args.output}")
    log("")

    api_key = load_api_key()
    log(f"GEMINI_API_KEY loaded(...{api_key[-6:]})")
    bake(api_key, voices, phrases, style, args.output, flat=args.flat)


if __name__ == "__main__":
    main()
