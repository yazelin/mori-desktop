# 開發工具 wiki(L-dev)

> Mori 有她自己的內在 wiki(`spirits/<name>/wiki/`,phase 9+ 的記憶之森 L-mori 側)。
> 本文講**你**(user)的開發工具 wiki — Claude Code / Codex CLI / Gemini CLI
> 共用一份 markdown 知識庫,跟 Mori 完全 decouple。

## 為什麼分兩個 wiki

- **Mori 的 wiki**(`spirits/<name>/wiki/`)= 她的內在記憶 / 對你的觀察 / 跟你的關係(private,在她 vault 內)
- **Dev wiki**(`~/wiki/`)= 你的工程知識 / API refs / 工具用法 / 認識的人(可能 share 給未來 user 看)

兩種 mode、兩種 audience,自然不該同份。Mori 不讀 `~/wiki/`;Claude/Codex/Gemini
也不讀 Mori 的 vault。

## 安裝

```bash
bash scripts/install-knowledge-wiki.sh
```

完成後:

- `~/wiki/` 結構建好(`raw/` + `wiki/{people,projects,concepts,meetings,resources}` + `index.md` + `agents.md` + `log.md`)
- `~/.claude/CLAUDE.md` / `~/.codex/AGENTS.md` / `~/.gemini/GEMINI.md` 各 symlink 到 `~/wiki/agents.md`
- 已存在且非 symlink 的 instruction 檔會被**跳過**(腳本不會覆寫 user 自己寫的 CLAUDE.md)

Script idempotent — 可重跑,既有檔不會被覆寫。

## 結構

```
~/wiki/
├── index.md                 ← LLM 進來第一個讀的全景目錄
├── agents.md                ← polyglot agent 規則(Claude/Codex/Gemini symlink 到這)
├── log.md                   ← agent 動過什麼的 append-only audit trail
├── raw/                     ← 不可變原料 dump
│   ├── articles/
│   ├── papers/
│   ├── readmes/
│   └── transcripts/
└── wiki/                    ← agent 編譯的「百科」(扁平階層 + cross-link)
    ├── people/              ← 認識的人 + 關係
    ├── projects/            ← 進行中的專案
    ├── concepts/            ← 抽象概念 / 框架 / 技術
    ├── meetings/            ← 會議紀錄
    └── resources/           ← 工具 / 服務 / 帳號
```

## Karpathy LLM Wiki pattern

設計靈感:LLM **主動維護** markdown 知識庫,取代 RAG。

- `raw/` 永遠 immutable(原料 append-only,不要改)
- `wiki/` 由 agent 主動 maintain + cross-link(Obsidian-style `[[<page-name>]]`)
- 改檔前 agent 必須 append 一行進 `log.md`
- LLM 進來時先讀 `index.md` 拿全景,再 selective grep / Read 相關 page 進 context

## Mori 跟這份 wiki 的關係

**無**。

Mori 的記憶 / 識身寫在 `~/mori-universe/spirits/<name>/`(SOUL.md / MEMORY.md /
她自己的 wiki/),完全獨立。本 wiki 是給 yazelin 的開發工具 agent(Claude Code /
Codex / Gemini)共用的;Mori 不會也不該讀。

agent 規則明文寫死:**不抄 mori-journal / spirits/mori/ 內容過來**(那是
Mori 的私人領域,不可公開化)。

## 維護

- agent 主動寫 `wiki/`(per `agents.md` 規則)
- 你自己手動 edit 也 OK(plain markdown,git 可 manage)
- `raw/` append-only 原料 dump
- `log.md` append-only audit trail

想把整個 `~/wiki/` 放進 private git repo 也很自然 — 結構就是 markdown,跟
Obsidian / VSCode 都相容。
