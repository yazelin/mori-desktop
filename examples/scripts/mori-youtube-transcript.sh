#!/bin/bash
# Helper for examples/agent/AGENT-04.YouTube 摘要.md profile.
#
# 抓 YouTube 影片字幕(auto-subs + manual subs)→ 純文字 transcript 印到 stdout。
# Mori 拿到後丟給 LLM 摘要。
#
# 使用前提:
#   - Deps tab 裝過 yt-dlp(實際在 $HOME/.local/bin/yt-dlp,uv 管 isolated venv)
#   - 或全域 PATH 自己有 yt-dlp 都行
#
# 安裝:
#   cp examples/scripts/mori-youtube-transcript.sh ~/bin/
#   chmod +x ~/bin/mori-youtube-transcript.sh
#
# 為什麼語言順序 zh-TW > zh-Hant > zh-Hans > zh > en.* > en:
#   - 優先抓繁中(台灣 user 痛點)
#   - 簡中 fallback(中國頻道 / 大量內容只有簡中字幕)
#   - 英文 fallback(國際內容)
#   - en.* glob 抓 en-US / en-GB / en-orig 各種變體
#
# 為什麼 cap 30KB:
#   1 小時影片 transcript 約 20-40KB,30KB 涵蓋大多數場景而不爆 LLM context。
#   長影片轉文字 + LLM 處理本來就 expensive,user 想完整 transcript 該另外導出。

set -e
URL="$1"
[ -z "$URL" ] && { echo "Usage: $0 <youtube-url>" >&2; exit 1; }

# Deps tab 裝的 yt-dlp 在 ~/.local/bin,確保 PATH 抓得到
export PATH="$HOME/.local/bin:$PATH"

if ! command -v yt-dlp >/dev/null 2>&1; then
  echo "ERROR: yt-dlp 找不到。請從 Deps tab 安裝(需先 uv)。" >&2
  exit 2
fi

TMP=$(mktemp -d /tmp/mori-yt-XXXXXX)
trap "rm -rf $TMP" EXIT

# 抓字幕(auto + manual),指定優先語言序。yt-dlp 沒對應字幕會 silently skip。
# stderr 丟 /dev/null,避免 progress / warning 污染 stdout(stdout 要乾淨 transcript)
yt-dlp \
  --skip-download \
  --write-auto-subs \
  --write-subs \
  --sub-langs "zh-TW,zh-Hant,zh-Hans,zh,en.*,en" \
  --convert-subs srt \
  -o "$TMP/%(id)s.%(ext)s" \
  "$URL" >/dev/null 2>&1 || true

SUB=$(ls -1 "$TMP"/*.srt 2>/dev/null | head -1)
if [ -z "$SUB" ]; then
  echo "ERROR: 此影片沒有 yt-dlp 能抓到的字幕(auto-subs + manual subs 都沒有)。" >&2
  echo "       可能是私人 / 區域鎖 / 完全無字幕影片。" >&2
  exit 3
fi

# srt → 乾淨純文字:去掉序號(純數字行)+ 時間軸(--> 行)+ HTML tag + 空行 + 連續重複行
grep -v "^[0-9]\+$" "$SUB" \
  | grep -v "\-\->" \
  | sed 's/<[^>]*>//g' \
  | sed '/^$/d' \
  | awk '!seen[$0]++' \
  | head -c 30000
