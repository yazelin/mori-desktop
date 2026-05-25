# Mori 自我開發 / 自舉設計草案

> 目標:讓 mori desktop 內的 Mori AI(可接 Claude/Gemini/Codex CLI)在**安全邊界內**自我改進 mori-desktop,並逐步走向可自舉。

## 1) 設計原則

1. **Human-in-the-loop,永不靜默寫入主分支**
   - Mori 只能產生提案、分支與 PR 草稿。
   - 合併到 `main` 必須由使用者確認。
2. **本機優先,資料主權不外流**
   - 所有程式碼分析、patch、測試結果預設儲存在本機 workspace。
   - LLM provider 可切雲端或本機,但 repo 寫入流程不依賴第三方 SaaS 中樞。
3. **能力最小化(Least Privilege)**
   - 將自動化行為拆成「讀取、規劃、修改、驗證、提 PR」五個 capability gate。
   - 每個 gate 都可在 UI 明確開關與審計。
4. **可回滾、可審計、可重播**
   - 每次自我開發任務都留下事件紀錄(任務輸入、模型、命令、diff、測試摘要、PR 連結)。

## 2) 自舉能力分層(由低到高)

### Level 0: Assistant-only
- Mori 僅產生建議與 patch 草稿,不寫檔。

### Level 1: Local patcher
- 允許在隔離工作樹(worktree)寫檔。
- 禁止 `git push`、禁止直接改 `main`。

### Level 2: Verify runner
- 可執行專案驗證腳本(例如 `bash scripts/verify.sh`)。
- 只回傳結果與失敗摘要,不自動重跑危險命令。

### Level 3: Branch + PR drafter
- 自動建立 feature branch、commit、PR 內容草稿。
- 仍需使用者按下「送出 PR / merge」。

### Level 4: Guarded self-iteration
- 允許在單一任務內自動迭代 N 輪(修測試→重跑→再修)。
- 受限於成本上限、時間上限、檔案白名單。

## 3) 建議的系統架構

```text
Mori Desktop UI
  └─ Development Orchestrator (new)
      ├─ Planner: 需求拆解 / 風險評分 / 任務 DAG
      ├─ Executor: 透過 CLI providers 執行 patch 與命令
      ├─ Verifier: 統一測試入口 + 結果正規化
      ├─ Git Operator: branch/commit/pr 草稿
      └─ Policy Engine: capability gate + budget + file allowlist
```

### 關鍵模組

1. **Development Orchestrator(建議放在 `mori-core`)**
   - 定義 `DevTask`、`DevPlan`、`DevAction`、`DevReport`。
   - 讓不同 provider(Claude/Gemini/Codex CLI)共用同一執行契約。

2. **Policy Engine(建議放在 `mori-tauri` + `mori-core`)**
   - UI 提供權限控制(例如:是否允許寫檔、是否允許跑測試、是否允許 git commit)。
   - core 僅接收已授權 capability token。

3. **Workspace Isolation**
   - 每個任務使用 `git worktree` 或臨時副本。
   - 預設只允許修改 `src/`、`crates/`、`docs/` 等白名單路徑。

4. **Verification Profiles**
   - `quick`: 靜態檢查 + 單元測試子集。
   - `full`: `bash scripts/verify.sh`。
   - `strict`: `VERIFY_STRICT=1 bash scripts/verify.sh`。

5. **PR Composer**
   - 根據 commit 與測試輸出產生 PR title/body。
   - 自動標記:平台影響(Linux/Windows/macOS)、風險等級、回滾方式。

## 4) 任務流程(範例)

1. 使用者在「自我開發」面板輸入需求。
2. Planner 產生計畫(含風險與預估成本)。
3. 使用者核准 capability 範圍(是否可寫檔/跑測試/commit)。
4. Executor 在隔離工作樹實作。
5. Verifier 執行對應 profile。
6. Git Operator 產生 branch + commit + PR 草稿。
7. 使用者審閱 diff 與測試結果後手動送出。

## 5) 安全與防失控機制

- **Kill Switch**:任何時刻可中斷任務並回收子程序。
- **Budget Guard**:限制 token、執行時間、命令次數。
- **Command Sandbox Profile**:
  - denylist: `rm -rf /`, 網路外連敏感位址、系統層套件安裝。
  - allowlist: `git`, `cargo`, `npm`, 專案內腳本。
- **Secrets Redaction**:
  - 日誌輸出前先遮蔽 API keys / tokens / 私密路徑。

## 6) 與現有 Mori 架構的對接點

- `mori-core`
  - 新增 `dev_orchestrator` 模組與 provider-agnostic 任務模型。
- `mori-tauri`
  - 新增 IPC: `start_dev_task`, `approve_dev_capability`, `abort_dev_task`, `get_dev_report`。
- `src/` 前端
  - 新增「自我開發」頁籤:任務輸入、權限核准、diff 檢視、測試摘要、PR 草稿。

## 7) 實作里程碑(建議)

### Phase A(1~2 週)
- 完成 `DevTask` 資料模型與事件記錄。
- 支援 Level 1(只 patch、不 commit)。

### Phase B(2~3 週)
- 加入 verify profile 與失敗摘要。
- 支援 Level 2/3(commit + PR draft)。

### Phase C(3+ 週)
- 加入多輪自動修復(Level 4)與 budget guard。
- 加入任務回放(replay)與品質評分。

### Phase D1 (平台落地檢核)
- 建立 Linux / Ubuntu 的自我開發前置檢查(preflight)流程。
- 在 UI 顯示依賴健康度與缺失修復指引，避免任務執行到一半才因環境失敗。
- 驗證 `bash scripts/verify.sh` 在 Ubuntu runner 可完整通過。

### Phase D2 (跨平台前置檢查)
- 在 Self-Dev preflight 顯示目前 OS 與對應建議安裝路徑。
- Linux 維持 `scripts/install-linux-deps.sh`，Windows/macOS 顯示「先走手動 setup 指引」。
- 補齊 D2 文件，建立跨平台驗證矩陣與已知限制。

### Phase D3 (平台驗證收斂)
- 補齊 Windows/macOS 的可執行驗證腳本與 checklist。
- 在 Self-Dev preflight 顯示「建議下一步」(Recheck / Verify / 回報)。
- 形成跨平台 release gate：Linux + Windows compile + 核心測試。

### Phase D4 (Release Gate 自動化)
- 將平台 preflight 結果輸出為可附在 PR 的檢查摘要。
- 在 Self-Dev 報告中附帶 gate checklist 狀態（pass/fail/unknown）。
- 讓人類審核者可快速判斷是否可進入 merge 候選。

### Phase D5 (審核交付優化)
- 在 UI 提供「一鍵複製 gate 摘要」供 PR description 直接貼上。
- 將 gate 摘要與任務 report 對齊，降低審核上下文切換成本。
- 明確標註 gate 為輔助訊號，不影響 human final decision。

### Phase D6 (審核封裝輸出)
- 一鍵複製「PR-ready 摘要」(task id / summary / gate)。
- 讓審核者不用手動拼接任務狀態與 gate 結果。
- 仍維持 human 最終決策與手動 merge。

### Phase D7 (審核記錄標準化)
- 提供可重用的「審核備註模板」(觀察 / 風險 / 決策)。
- 讓 Self-Dev 輸出可直接貼進 PR review comment。
- 將人類決策理由保留在審核紀錄中。

### Phase E (Stabilization)
- E1:把 gate 狀態接到更真實的 build/test signal(依 verify profile / command 判斷)。
- E2:加 end-to-end regression 測試(IPC + UI flow)。
- E3:定義可發版門檻與失敗回復流程。

## 8) 自舉定義(DoD)

當以下條件達成,可稱為「可自舉」:

1. Mori 能在隔離環境完成**小型 feature 或 bugfix**。
2. 能自動跑專案標準驗證並解讀失敗。
3. 能自行修正至少一次失敗並重新驗證。
4. 能產生可審核 PR(含風險與平台影響說明)。
5. 全程保持人類最終批准權與可回滾性。

---

這份草案重點是:先把「**可控的自動化開發回路**」做穩,再追求更高自主。這樣 Mori 才能在不背離資料主權與安全邊界下,逐步走向真正自舉。

- 若你在執行層面要開始實作,請先看 [`docs/self-hosting-phase-a.md`](self-hosting-phase-a.md) 的下一步執行清單。
- 若你要做 Self-Dev Beta 上線前驗證,請直接跑 [`docs/self-dev-beta-checklist.md`](self-dev-beta-checklist.md)。
