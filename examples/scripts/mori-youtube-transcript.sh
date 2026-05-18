#!/bin/bash
# Helper for examples/agent/AGENT-04.YouTube 摘要.md profile.
#
# 抓 YouTube 影片字幕(auto-subs + manual subs)→ 純文字 transcript。
# **長篇自動切塊** + cache,讓 LLM 分批 summarise 避免 context 爆。
#
# ─── USAGE ──────────────────────────────────────────────────────────────
#
#   mori-youtube-transcript.sh <url>            # 第一次:fetch + split + cache + 印 meta + chunk 1
#   mori-youtube-transcript.sh <url> 0          # 同上(顯式)
#   mori-youtube-transcript.sh <url> <n>        # 印 cache 中第 n 塊(1-based)
#
# ─── OUTPUT FORMAT ──────────────────────────────────────────────────────
#
# chunk_idx == 0(首次呼叫):
#
#   __MORI_META__
#   {"video_id":"x","total_chunks":N,"duration_secs":"S","chunk_bytes":B}
#   __MORI_CHUNK_1_OF_N__
#   <chunk 1 text>
#
# chunk_idx >= 1(後續呼叫):
#
#   __MORI_CHUNK_n_OF_N__
#   <chunk n text>
#
# 為什麼用 `__MORI_*__` 這種 marker 而不是 JSON wrapping:
#   LLM 直接看裸文字效果好,marker 也容易識別 chunk 邊界。包成 JSON 會
#   把 transcript 內的 `"` 通通跳脫,反而干擾理解。
#
# ─── CACHE ──────────────────────────────────────────────────────────────
#
#   ~/.cache/mori/youtube/<urlhash>/
#     meta.json
#     chunk-01.txt
#     chunk-02.txt
#     ...
#
# urlhash = sha256(url) 前 16 字。同 URL 重抓會清舊 cache 再重切,避免新舊
# 字幕版本混雜。手動 clear:`rm -rf ~/.cache/mori/youtube/`
#
# ─── 設計選擇 ───────────────────────────────────────────────────────────
#
# 語言順序 zh-TW > zh-Hant > zh-Hans > zh > en.* > en:
#   - 優先繁中(台灣 user 痛點)
#   - 簡中 fallback(中國頻道大量內容只有簡中)
#   - 英文 fallback,en.* 抓 en-US / en-GB / en-orig 變體
#
# Chunk 預設 20KB(MORI_YT_CHUNK_BYTES 可覆寫):
#   - ~5K tokens / 塊,給 LLM intermediate-summary 充足上下文
#   - 大部分 provider context 128K+ 沒問題,但對話歷史已佔用部分 budget
#   - 太小 → chunk 多 + tool call latency 多;太大 → 接近 context 邊界
#
# Chunk 切點對齊整行(awk 累積 byte 計數):
#   - 不切到單行中間,避免 LLM 收到半句
#   - srt 已經是行式格式,line boundary 通常 = 句點 / 換氣處

set -e
URL="$1"
CHUNK_IDX="${2:-0}"
CHUNK_BYTES="${MORI_YT_CHUNK_BYTES:-20000}"

[ -z "$URL" ] && { echo "Usage: $0 <youtube-url> [chunk_idx]" >&2; exit 1; }

# Deps tab 裝的 yt-dlp 在 ~/.local/bin
export PATH="$HOME/.local/bin:$PATH"

# URL → cache dir(sha256 前 16 字當 hash;同 URL 不同 chunk_idx 共用 cache)
URL_HASH=$(printf "%s" "$URL" | sha256sum | cut -c1-16)
CACHE_DIR="$HOME/.cache/mori/youtube/$URL_HASH"

# ─── chunk_idx >= 1 → 從 cache 印 ────────────────────────────────────
if [ "$CHUNK_IDX" -ge 1 ] 2>/dev/null; then
  META="$CACHE_DIR/meta.json"
  if [ ! -f "$META" ]; then
    echo "ERROR: cache 不存在 — 你需要先呼叫 chunk=0(或不傳)觸發 fetch + split。" >&2
    echo "       Cache 位置:$CACHE_DIR" >&2
    exit 4
  fi
  TOTAL=$(grep -oE '"total_chunks":[[:space:]]*[0-9]+' "$META" | grep -oE '[0-9]+' || echo "0")
  if [ "$CHUNK_IDX" -gt "$TOTAL" ]; then
    echo "ERROR: chunk $CHUNK_IDX 超出範圍(total=$TOTAL)。" >&2
    exit 5
  fi
  CHUNK_FILE=$(printf "%s/chunk-%02d.txt" "$CACHE_DIR" "$CHUNK_IDX")
  if [ ! -f "$CHUNK_FILE" ]; then
    echo "ERROR: chunk 檔不存在 — cache 不完整:$CHUNK_FILE" >&2
    exit 6
  fi
  echo "__MORI_CHUNK_${CHUNK_IDX}_OF_${TOTAL}__"
  cat "$CHUNK_FILE"
  exit 0
fi

# ─── chunk_idx == 0 → fetch + split + cache + 印 meta + chunk 1 ──────

if ! command -v yt-dlp >/dev/null 2>&1; then
  echo "ERROR: yt-dlp 找不到。請從 Deps tab 安裝(需先 uv)。" >&2
  exit 2
fi

# 清舊 cache → 重抓(避免新舊版本字幕混在一起)
rm -rf "$CACHE_DIR"
mkdir -p "$CACHE_DIR"

TMP=$(mktemp -d /tmp/mori-yt-XXXXXX)
trap "rm -rf $TMP" EXIT

# 拿 metadata + 字幕(同一輪 yt-dlp 呼叫,省得跑兩次)
VIDEO_ID=$(yt-dlp --get-id "$URL" 2>/dev/null || echo "unknown")
DURATION=$(yt-dlp --get-duration "$URL" 2>/dev/null || echo "0")

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

# srt → 乾淨純文字:去序號 / 時間軸 / HTML tag / 空行 / 連續重複行
CLEAN="$TMP/clean.txt"
grep -v "^[0-9]\+$" "$SUB" \
  | grep -v "\-\->" \
  | sed 's/<[^>]*>//g' \
  | sed '/^$/d' \
  | awk '!seen[$0]++' \
  > "$CLEAN"

# 切塊 — awk 線性累積 byte 計數,line-aligned 切點。回傳總塊數。
TOTAL=$(awk -v limit="$CHUNK_BYTES" -v outdir="$CACHE_DIR" '
BEGIN {
  idx = 1
  cur = 0
  out = sprintf("%s/chunk-%02d.txt", outdir, idx)
}
{
  line_len = length($0) + 1   # +1 給 newline
  if (cur + line_len > limit && cur > 0) {
    close(out)
    idx++
    cur = 0
    out = sprintf("%s/chunk-%02d.txt", outdir, idx)
  }
  print > out
  cur += line_len
}
END {
  close(out)
  print idx
}' "$CLEAN")

# 寫 meta.json — 簡單 hand-rolled 避免依賴 jq(可能沒裝)
cat > "$CACHE_DIR/meta.json" <<EOF
{"video_id":"$VIDEO_ID","total_chunks":$TOTAL,"duration_secs":"$DURATION","chunk_bytes":$CHUNK_BYTES}
EOF

# 輸出:meta + chunk 1 連起來(LLM 一次拿到兩個資訊,少一次 tool call round-trip)
echo "__MORI_META__"
cat "$CACHE_DIR/meta.json"
echo "__MORI_CHUNK_1_OF_${TOTAL}__"
cat "$CACHE_DIR/chunk-01.txt"
