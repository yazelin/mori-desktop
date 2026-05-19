#!/usr/bin/env python3
"""mori-tts-edge.py — Mori 講話的 edge-tts bridge(Phase 3D)。

mori-tauri 在 agent 回應完成後(若 `tts.enabled=true`),spawn 這隻 script
用 Microsoft Edge 瀏覽器的 TTS endpoint 把回應文字合成 MP3。Rust 端 rodio
讀那個 MP3 放出來。

## 為什麼 edge-tts

- 完全免費(借 MS Edge browser 端點,無 API key、無官方 quota)
- Native zh-TW 女聲(`zh-TW-HsiaoYuNeural` / `HsiaoChenNeural`),沒英文口音味
- Python lib 輕量(~10MB),跟 wake-listener 共用 `~/.mori/wake-venv`
- 比 Gemini TTS quota 友善(後者免費層 100 req/day,Mori 對答幾句就破)

## 用法

  python mori-tts-edge.py <output-mp3-path> <voice> < stdin

  text 從 stdin 讀(避免長字串 + 特殊字元在 argv 出包)。

## stdout / stderr protocol

  stdout:純資訊 log,Rust 端會吞掉
  stderr:錯誤訊息(non-zero exit code 時 Rust 看 stderr decide 怎麼報)

  exit code:
    0 = success(MP3 寫到指定路徑)
    1 = bad usage(arg 不對)
    2 = edge-tts 失敗(網路 / endpoint 問題)

## 中斷

  Rust 端 Drop child 會 SIGKILL,沒寫完的 MP3 caller 應該刪掉。

## Voice 清單

  edge-tts --list-voices 看完整,zh-TW 常用:
    zh-TW-HsiaoChenNeural  (女,標準)
    zh-TW-HsiaoYuNeural    (女,偏年輕)
    zh-TW-YunJheNeural     (男)
"""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path

DEFAULT_VOICE = "zh-TW-HsiaoYuNeural"


async def synth(text: str, voice: str, out_path: Path) -> None:
    """call edge-tts → save MP3 到 out_path"""
    try:
        import edge_tts
    except ImportError:
        print("edge-tts 沒裝。跑 DepsTab → 「TTS runtime」一鍵裝。", file=sys.stderr)
        sys.exit(2)

    if not text.strip():
        print("text 是空的,跳過", file=sys.stderr)
        sys.exit(1)

    communicate = edge_tts.Communicate(text, voice)
    try:
        await communicate.save(str(out_path))
    except Exception as e:
        print(f"edge-tts 合成失敗:{e}", file=sys.stderr)
        sys.exit(2)


def main() -> None:
    if len(sys.argv) < 2:
        print("用法:mori-tts-edge.py <output-mp3-path> [voice]", file=sys.stderr)
        print("  text 從 stdin 讀。voice 預設 zh-TW-HsiaoYuNeural", file=sys.stderr)
        sys.exit(1)

    out_path = Path(sys.argv[1])
    voice = sys.argv[2] if len(sys.argv) > 2 else DEFAULT_VOICE

    text = sys.stdin.read()
    if not text:
        print("stdin 沒文字", file=sys.stderr)
        sys.exit(1)

    out_path.parent.mkdir(parents=True, exist_ok=True)
    asyncio.run(synth(text, voice, out_path))


if __name__ == "__main__":
    main()
