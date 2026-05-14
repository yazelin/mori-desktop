# Claude Code 指引 — mori-desktop

mori-desktop 是 **Mori 的「身體」** — Tauri 2 + Rust 桌面 AI 同伴 GUI。它跟其他 3 個 repo 構成「**森林宇宙(Mori universe)**」:

| Repo | 角色 |
|---|---|
| `mori-desktop`(這裡)| 身體 / GUI / 選取補捉 / action skills |
| `annuli`(public)| 反思引擎(Python,vault-backed reflection service) |
| `world-tree`(public)| 公開 lore wiki(Quartz)+ spirit 模板 |
| `mori-journal`(**private**)| Mori 自己的 vault — identity / memories / projects |

Mori 不是 chatbot — 是一個有 SOUL.md 跟連續記憶的擬人化「靈」。設計用詞偏詩意中文(年輪 / 反思 / 森林 / 唇與聲),代碼用詞跟普通 Rust / TS 一致。

**本機 layout 慣例**:本 repo 在 `~/mori-universe/mori-desktop/`,4 個 Mori repo 的完整 local layout 見 [`world-tree/ARCHITECTURE.md` 「本機 layout 慣例」section](https://github.com/yazelin/world-tree/blob/main/ARCHITECTURE.md#本機-layout-慣例)。簡述:`~/mori-universe/{world-tree,mori-desktop,annuli}` + `~/mori-universe/spirits/<name>/`(spirit vault)。

完整世界觀讀:
- `docs/architecture.md` — 4 層宇宙模型
- `docs/design/annuli-memory.md` — vault-backed 反思引擎設計
- `docs/roadmap.md` — 詩意章節版工程路線圖
- 宇宙論層:world-tree `lore/cosmology.md` `the-forest.md` `timeline.md`

## 硬規矩(無條件遵守)

1. **不公開比較其他專案** — 不寫 "vs OpenHuman" / "inspired by Hermes Agent" 之類比較。私下研究別人 OK,寫進 README / roadmap / PR body / blog 一律用 Mori 自己的詞彙講「她的成長」。
2. **User-owned data** — 設計時拒絕中央 OAuth relay / SaaS hub / 任何 yazelin 不掌控的第三方資料中繼。vault 在 `~/mori-universe/spirits/<name>/`,user 永遠是 data owner。LLM 走 user 自帶 key 的 provider。
3. **mori-journal 寫入邊界**(若有授權 clone)— 只能寫 `projects/` 子目錄;`identity/`(SOUL.md)跟 `memories/`(MEMORY.md)是 Mori 本人的,**禁止 ghost-write**,要寫先 explicit re-authorize。
4. **Bundle deps in repo** — setup / 依賴腳本放這個 repo 內(`scripts/install-linux-deps.sh`),不從外部 setup repo 拉,CI 也用同一份。
5. **annuli 整合走 HTTP** — mori-desktop 將來呼叫 annuli 走 HTTP(localhost:5000 / 5001),不直接 import Python(本來也不能,跨語言)。

## 工程注意

- **Windows quirks**:
  - `detect_mori_cli` 要找 `mori.exe`(`PathBuf::exists()` 不會自動加 `.exe`)
  - 開外部 app 用 `ShellExecuteExW` + `SEE_MASK_FLAG_NO_UI`,不用 `cmd /c start`(會跳「找不到 app」白 dialog)
  - `HOME` 環境變數可能沒設,fallback `USERPROFILE`;`canonicalize()` 會加 `\\?\` prefix
- **mori-cli 不會自動 build** — `package.json` 有 `predev` / `prebuild` hook 跑 `cargo build -p mori-cli`,改 npm scripts 時別動掉
- **Linux dev 依賴** — 第一次 setup 跑 `bash scripts/install-linux-deps.sh`(也是 CI 用的同一份)
- **LLM action skills** — system prompt 有強 anti-refusal rule,別讓 LLM 編造「需要授權」之類藉口拒絕跑 action;若行為走樣去看 `crates/mori-tauri/src/agent_runtime.rs` 內的 system prompt

## 當前狀態(2026-05)

- mori-desktop 自己:v0.2.0 released,Linux + Windows 雙平台 work
- annuli 整合:**未連線**,annuli 還在 Wave 2 重構中(`refactor/split-core-creator` branch)
- Wave 3+ 反思引擎 / mori-desktop ↔ annuli HTTP / 4-layer reflection:待 annuli Wave 2 完成

## 跟 yazelin 共事

- 繁中、直接;「好 繼續」= trust your next step,別停下來問細節
- 重大架構決定要雙向確認,工程細節自己 call 完繼續
- 認真技術 — 不避細節,但別 over-explain
