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
4. **Bundle deps in repo** — setup / 依賴清單放這個 repo 內(`scripts/linux-build-packages.txt` + `scripts/install-linux-deps.sh`),不從外部 setup repo 拉,CI 也用同一份。
5. **annuli 整合走 HTTP** — mori-desktop 將來呼叫 annuli 走 HTTP(localhost:5000 / 5001),不直接 import Python(本來也不能,跨語言)。

## 工程注意

- **Windows quirks**:
  - `detect_mori_cli` 要找 `mori.exe`(`PathBuf::exists()` 不會自動加 `.exe`)
  - 開外部 app 用 `ShellExecuteExW` + `SEE_MASK_FLAG_NO_UI`,不用 `cmd /c start`(會跳「找不到 app」白 dialog)
  - `HOME` 環境變數可能沒設,fallback `USERPROFILE`;`canonicalize()` 會加 `\\?\` prefix
- **mori-cli 不會自動 build** — `package.json` 有 `predev` / `prebuild` hook 跑 `cargo build -p mori-cli`,改 npm scripts 時別動掉
- **Linux dev 依賴** — 第一次 setup 跑 `sudo bash scripts/install-linux-deps.sh`(也是 CI 用的同一份)。Codex Cloud / CI image 要預裝的 apt 清單在 `scripts/linux-build-packages.txt`。
- **UI 配色一律走 theme token,別寫死色值** — 雙 theme(dark/light/自訂):`theme.ts::applyTheme()` 啟動時把 active theme 寫進 `:root` 的 `--c-*` 變數,所以**任何顏色都要用 `var(--c-...)`**(`--c-text` / `--c-text-muted` / `--c-border` / `--c-success-text` / `--c-warning-text` / `--c-danger-text` …,全清單見 `src/shell.css` 開頭 `:root`)。**寫死的 inline `rgba(...)` / hex 不會跟著切 theme,也對不上 palette**。狀態 / 分級 badge 用 class `.mori-pill-badge tone-{success,warning,danger,neutral}`,錯誤字用 `.mori-tab-error`(都已在 `shell.css` 定義 dark + `[data-theme-base="light"]` 覆寫);要新增 badge 配色就照 `.mori-skill-kind` / `.mori-memory-type` 的 pattern 在 `shell.css` 補 light override,別在 .tsx 寫死。
- **新 tab 版面用既有 class,別自己 inline** — 頁面容器用 `.mori-tab`(`max-width: 920px` + 標準 padding)、標題 `.mori-tab-title`、副說明 `.mori-tab-hint`、空 / 找不到狀態 `.mori-tab-placeholder`(都在 `shell.css`;**`src/tabs/LogsTab.tsx` 是範本**)。**別**用 `<div style={{padding:16}}>` + 預設 `<h2>` 起手 —— 會撐滿全寬、標題大小 / 色不對,跟其他頁不一致(BI-1~3 的 Body/Permissions/Pulse tab 犯過,已修)。
- **LLM action skills** — system prompt 有強 anti-refusal rule,別讓 LLM 編造「需要授權」之類藉口拒絕跑 action;若行為走樣去看 `crates/mori-tauri/src/agent_runtime.rs` 內的 system prompt
- **共用驗證入口** — 開 PR / follow-up 前跑 `bash scripts/verify.sh`。預設包含 `npm run build`、`cargo test -p mori-core --lib`、`cargo check --workspace --all-targets`。`VERIFY_STRICT=1 bash scripts/verify.sh` 會額外跑 `cargo fmt --check` / `cargo clippy`,但目前既有 Rust tree 尚未全 rustfmt-clean,不要為了 unrelated formatting 大量改檔。
- **Codex / 通用 agent 指引** — `AGENTS.md` 是給 Codex Cloud / 其他 agent 的共用規則;改 workflow 或驗證命令時要跟這份同步。
- **Cloud agent SOP** — `docs/agent-workflow.md` 記錄 Codex Cloud / Claude Code 訂閱型工作流、PR、CI failure loop、release 邊界;雲端開工前先讀。
- **Release body 格式** — `docs/release-format.md` 是 GitHub Release 頁面 body 的統一 template + section-by-section 該寫不該寫。每次 tag push 後 draft Release 出來,publish 前照這份改寫(不要照搬 CHANGELOG,讀者目標不同)。

## 當前狀態(2026-05-22 大爆發後)

- **mori-desktop**:v0.7.1 released + 一個 session ship 約 25 個 PR 進 main(2026-05-21 → 22):

  **核心 streams(Wave 1)**:
  - **Stream A** (#71):SOUL vault loader(`build_system_prompt()` 注 `spirits/<name>/identity/SOUL.md`)
  - **Stream C** (#72):Obsidian CLI 6 個 starter shell_skills(AGENT-06.Obsidian)
  - **Stream E 萬卷之口** (#73-77 + #93):`mori-file-loader` crate + .txt/.md/.pdf/.docx/.xlsx/.epub + Tauri command + `ReadFileSkill` LLM dispatch + DOCX hyperlink/break/tab + XLSX DateTime ISO/Error placeholder
  - **Stream G** (#81):canonical SOUL distribution(world-tree pull + bundle fallback)
  - **Stream I** (#79):SKILL.md loader D-light(Anthropic skill markdown body 進 system prompt)
  - **Stream J** (#78):prompt 措辭清理
  - **Stream K 時之鳥** (#80, #86, #87, #88, #89, #92):`mori-time` crate + SQLite + tokio-cron-scheduler + notify-rust + chrono-english + NL parser + 4 Tauri commands + `RemindMeSkill` LLM dispatch + Deps 頁 libdbus 整合 + K2/K5 polish

  **Wave 5 快贏批**:
  - #90 CLAUDE.md status / #91 Obsidian Deps detect / #92 K stream polish / #93 file-loader polish

  **Wave 6 生態解鎖批**:
  - **MCP-1/2** (#94, #95):`mori-mcp` crate(rmcp client + config + discovery)+ `McpToolSkill` + Node.js DepSpec — LLM 可叫 `mcp_<server>_<tool>` 連 GitHub / Playwright / Notion 等
  - **DF-1/2** (#96, #98):Python runtime + Anthropic skills install(`anthropics/skills` clone)+ AnthropicScriptSkill + python_runner subprocess engine — pdf 填表 / docx 寫 / xlsx 改 / pptx 全功能

  **Wave 7 大進步批 L 記憶之森**:
  - **L-mori** (#100):mori-desktop 讀 `spirits/<name>/wiki/index.md` 進 system prompt + `ReadWikiPageSkill`(read-only,write 留 annuli reflection)
  - **L-dev** (#99):`~/wiki/` user 開發工具 wiki + `install-knowledge-wiki.sh` + Claude/Codex/Gemini symlinks polyglot `agents.md`

  **Wave 8 跨界之手**:
  - **Gmail Gm-1/2** (#101, #102):`mori-gmail` crate(OAuth2 + token + read-only API + send scope)+ `ListGmailSkill` / `ReadGmailSkill` / `SendGmailSkill` + Tauri commands + Gmail OAuth DepSpec
  - Calendar / Slack / Discord:**未做**

  **§13 lore canon 釘死**:「同 SOUL 異 Rings」— Mori 是世界樹既有存在,每 user 召喚自己的 instance

- **annuli**:Wave 2/3/4 全落地。可上線。整合對接點走 HTTP,vault path 已遷 `~/mori-universe/spirits/<name>/`。
- **leaf crates**(全新):`mori-file-loader` / `mori-time` / `mori-mcp` / `mori-gmail` — 各自 leaf,不 depend 既有 crate。

**Jarvis 缺口表進度**(本 session 後):
- 🎙 語音 80%(待 VAD 升 85%)
- 👁 視覺 0%(觀之眼未開)
- 🧠 記憶/RAG 30% → **45%**(SOUL 注入 + L-mori wiki read + L-dev wiki)
- 🛠 工具使用 70% → **90%**(SKILL.md + 萬卷之口 + MCP + D-full + Gmail)
- 📅 時間/排程 5% → **80%**(時之鳥)
- 🌐 服務整合 5% → **30%**(Gmail;Calendar/Slack/Discord 未做)
- 🖥 系統控制 20%
- 🏠 IoT 0%

**累積 leaf crate**:`mori-core` / `mori-tauri` / `mori-cli`(既有)+ `mori-file-loader` / `mori-time` / `mori-mcp` / `mori-gmail`(本 session 新加)。

**Manual test 等 yazelin** — `npm run dev` 啟 app,逐 feature 試(時之鳥 / 萬卷之口 / MCP / Gmail OAuth 等)。

## 跟 yazelin 共事

- 繁中、直接;「好 繼續」= trust your next step,別停下來問細節
- 重大架構決定要雙向確認,工程細節自己 call 完繼續
- 認真技術 — 不避細節,但別 over-explain
