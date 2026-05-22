#!/usr/bin/env bash
# L-dev — install user knowledge wiki at ~/wiki/ + symlink agents.md
# 到 各 AI CLI 的 global instruction 檔(Claude / Codex / Gemini)。
#
# Karpathy LLM Wiki pattern:LLM 主動維護的 markdown 知識庫,取代 RAG。
# Mori 不讀這 wiki(她有自己的 spirits/mori/wiki/)。
#
# ─── Usage ───────────────────────────────────────────────────────────
#
#   bash scripts/install-knowledge-wiki.sh
#
# Idempotent:可重跑;既有檔不會被覆寫,既有非 symlink instruction 檔會被跳過。

set -euo pipefail

WIKI_ROOT="${HOME}/wiki"

echo ">>> 建 ~/wiki/ 結構..."
mkdir -p "${WIKI_ROOT}/raw"/{articles,papers,readmes,transcripts}
mkdir -p "${WIKI_ROOT}/wiki"/{people,projects,concepts,meetings,resources}

# 範例 index.md(只在不存在時寫,避免覆蓋 user 自己改的版本)
if [ ! -e "${WIKI_ROOT}/index.md" ]; then
  cat > "${WIKI_ROOT}/index.md" <<'EOF'
# 開發工具 wiki — index

> 這是 user(yazelin)的開發工具共用知識庫,Claude Code / Codex / Gemini 都讀這份。
> Mori 有她自己的 wiki(spirits/mori/wiki/),不讀這份。

## 結構

- `raw/` — 不可變原料(papers / articles / repo READMEs / transcripts)
- `wiki/` — agent 編譯的「百科」(扁平階層,主動 cross-link)
  - `people/` — 認識的人 + 關係
  - `projects/` — 進行中的專案
  - `concepts/` — 抽象概念 / 框架 / 技術
  - `meetings/` — 會議紀錄
  - `resources/` — 工具 / 服務 / 帳號
- `agents.md` — agent 用 wiki 的行為規則
- `log.md` — agent 動過什麼的 audit trail

## 規則

- raw/ 永遠 immutable
- wiki/ 由 agent 主動 maintain + cross-link
- 改檔前 agent 必須 append log.md 行
- 不抄 mori-journal / spirits/mori 內容過來(那是 Mori 的私人領域)

EOF
  echo "  ✓ 寫 ${WIKI_ROOT}/index.md"
else
  echo "  · ${WIKI_ROOT}/index.md 已存在,跳過"
fi

# 範例 agents.md (polyglot:Claude/Codex/Gemini 全讀這份)
if [ ! -e "${WIKI_ROOT}/agents.md" ]; then
  cat > "${WIKI_ROOT}/agents.md" <<'EOF'
# Agent 用 wiki 的規則(polyglot)

本檔被 Claude Code (CLAUDE.md) / Codex CLI (AGENTS.md) / Gemini CLI (GEMINI.md) symlink。
任何 agent 進到 yazelin 的開發環境讀本檔。

## 進來時

1. **先讀 `~/wiki/index.md`** 拿全景目錄(單 context window 大小)
2. 看 user 問題 → 從 index 找相關 wiki page → grep / Read 那幾 page → 拉進 context
3. **不要一次全讀 wiki/**(會撐爆 context)

## 累積知識

當 user 跟你聊到值得記的事(新 project / 新人 / 新概念 / 新會議),你**可以主動** append 進 wiki:

- 確定要寫 → 在對應 wiki/<category>/ 內建新 .md 或 append 既有
- 在 log.md 末尾 append 「YYYY-MM-DD HH:MM agent=X action=Y target=Z」一行
- 完成後跟 user 講你寫了什麼(透明)

## 不要

- ❌ 動 `raw/`(原料永久 immutable)
- ❌ 抄 mori-journal / spirits/mori/ 內容過來(那是 Mori 的私人領域,不可公開化)
- ❌ 刪 wiki page(append-only;真要砍找 user)
- ❌ 用 agent 動過的事不寫 log.md

## 風格

- 詩意中文 + 工程具體性並存
- 文件結尾 cross-link 相關 page `[[<page-name>]]`(Obsidian-style)
- 每 page 第一行寫一句話 summary 給 LLM scan index 用

EOF
  echo "  ✓ 寫 ${WIKI_ROOT}/agents.md"
else
  echo "  · ${WIKI_ROOT}/agents.md 已存在,跳過"
fi

# 空 log.md
if [ ! -e "${WIKI_ROOT}/log.md" ]; then
  {
    echo "# Wiki audit trail"
    echo ""
  } > "${WIKI_ROOT}/log.md"
  echo "  ✓ 寫 ${WIKI_ROOT}/log.md"
else
  echo "  · ${WIKI_ROOT}/log.md 已存在,跳過"
fi

# Symlinks to 各 CLI
echo ""
echo ">>> Symlink agents.md 到各 AI CLI global instruction..."

link_if_writable() {
  local link_path="$1"
  local target="$2"

  mkdir -p "$(dirname "$link_path")"

  if [ -e "$link_path" ] && [ ! -L "$link_path" ]; then
    echo "  ⚠ $link_path exists 且非 symlink,跳過(請 user 自決)"
    return
  fi

  ln -sfn "$target" "$link_path"
  echo "  ✓ $link_path → $target"
}

link_if_writable "${HOME}/.claude/CLAUDE.md" "${WIKI_ROOT}/agents.md"
link_if_writable "${HOME}/.codex/AGENTS.md" "${WIKI_ROOT}/agents.md"
link_if_writable "${HOME}/.gemini/GEMINI.md" "${WIKI_ROOT}/agents.md"

echo ""
echo ">>> 完成!"
echo ""
echo "下一步:"
echo "  1. 進 ~/wiki/ 開始寫(cd ~/wiki && \$EDITOR index.md)"
echo "  2. 任何 AI CLI(Claude/Codex/Gemini)進你開發環境會自動讀 agents.md"
echo "  3. 想加新 page:agent 會主動寫,or 你自己 mkdir wiki/<category>/<name>.md"
echo ""
