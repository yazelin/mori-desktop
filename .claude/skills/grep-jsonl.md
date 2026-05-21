---
description: Inspect Mori's `~/.mori/logs/mori-YYYY-MM-DD.jsonl` event log to diagnose user-reported issues — find errors, trace Hey Mori cycles, verify skill dispatches were real (not LLM hallucination), compare what claude/codex got as prompt vs what they returned. Invoke when user says "看一下 log" / "what happened" / "為什麼壞掉" / "log 有什麼" / "diagnose this error".
---

# grep-jsonl — Mori 事件 log 解讀

Mori 的所有 release-mode 觀測都在 `~/.mori/logs/mori-YYYY-MM-DD.jsonl`(append-only JSONL,UTC timestamp,每日 rotate)。release build tracing 沒接 file appender,**這份 JSONL 是唯一可見路徑**。

## When to invoke

- User 說「看 log / 查 log / 看一下 jsonl / 看 mori 在做什麼」
- User 報 bug「壞掉了 / 沒反應 / 沒打開 / Mori 死了」
- 你自己想驗「我剛改的東西真的生效嗎」
- ship-and-verify skill 的 step 4(讀 log 對照)會 dispatch 過來

## Steps

### 1. 拿日期 + 行數

```bash
LOG=~/.mori/logs/mori-$(date +%Y-%m-%d).jsonl
wc -l "$LOG"
```

跨平台 `~` 都解(Git Bash on Windows / Linux bash)。timestamps 是 UTC,user 在 GMT+8 → 加 8 小時對到本地時間。

### 2. 只讀「新增段」

別重讀已知的行 — 浪費 context 又沒新資訊。看 conversation 上次讀到第幾行,從那行 `+1` offset 開始讀。完全沒讀過就讀全檔。

### 3. 認得 event kind 的「意義」

| `kind` 欄位 | 意義 | 重點欄 |
|---|---|---|
| `wake_listener_spawned` | 進 Listening / cycle respawn 後啟動新 Python wake-listener | `prev_mode` |
| `wake_listener_ready` | openwakeword Model 載完(~2s after spawn) | `model` 路徑 |
| `wake_listener_error` | Python 死於 ImportError / model load fail / mic 開不起來 | `msg` |
| `wake_listener_respawn` | `set_phase` 每完成一輪 cycle 補位 respawn | `reason` / `phase` |
| `wake_listener_stopped` | 離開 Listening mode → drop child | `new_mode`(切到哪) |
| `wake_listener_diag` | Python signal-gated(`max_score > 0.3` 或 `max_rms > 0.02`)才寫,idle 不寫 | `text` |
| `wake_word_event` | 偵測到 wake-word → 觸發錄音 | `score`(confidence) |
| `silence_trim` | VAD silence-stop + trim 前後靜音 | `trimmed_secs` |
| `voice_input_completed` | VoiceInput mode(非 wake)的 transcript 完成 | `target_process` / `transcript_cleaned` |
| `skill_dispatch` | skill_server `POST /skill/<name>` 收到 + 跑完 | `skill` / `args_preview` / `ok` / `user_message_preview` |
| `llm_call` | bash-cli-agent(claude/codex/gemini)單次 round-trip | `binary` / `ok` / `latency_ms` / `stdin_tail_preview` / `response_preview` |
| `agent_completed` | 整段 agent pipeline(含 skill loop)完成 | `profile` / `provider` / `response_chars` / `skill_calls` |

### 4. 常見 diagnose pattern

#### 「Hey Mori 沒觸發 / 一次後就沒了」

看 `wake_listener_*` 序列。健康的 cycle:

```
wake_listener_spawned → wake_listener_ready
wake_word_event       ← user 喊
silence_trim          ← 錄音 + VAD
skill_dispatch / llm_call
agent_completed
wake_listener_stopped → wake_listener_spawned ← cycle 補位 respawn
wake_listener_ready
(下一輪 wake_word_event ...)
```

- **只看到 spawned 沒 ready** → Python 載 model 卡 / 死,看 stderr drain 的 diag
- **ready 後完全沒 wake_word_event** → mic 沒收到聲音 / 講太小聲 / threshold 太高,看 `wake_listener_diag` 的 `max_rms` 跟 `max_score`
- **一輪後沒下一輪 wake** → respawn 沒跑(`set_phase` 沒進 Done/Error),或 user 切走 mode(看 `wake_listener_stopped` 的 `new_mode`)

#### 「Mori 說開啟了,但 Chrome 沒開 / app 沒反應」

對照 `skill_dispatch` 跟 `agent_completed.response`:

| 兩者都有對應的 entry | 兩者文字對齊 | 結論 |
|---|---|---|
| ✅ | ✅ | skill 真的跑了,問題在 OS 端(Chrome 把 URL 開背景 tab 沒 focus / Windows ShellExecute 沒生效) |
| ✅ `skill_dispatch` 缺 | ✅ agent_completed 有「已開啟」 | **LLM 幻覺** — 沒呼叫 skill,response 是憑空編。改強系統 prompt anti-hallucination |
| 兩者都缺 | — | agent 根本沒跑那條 skill,可能是 LLM 不認得 task / profile 沒 enable 那條 skill |

#### 「codex / claude / gemini agent 壞掉」

看 `llm_call` 的 `ok=false` 跟 `error` 欄:

- `Not inside a trusted directory and --skip-git-repo-check was not specified` → codex CLI 預設 git repo 限制(v0.7.1 已修)
- `command not found` / `failed to spawn` → bash-cli-agent 找不到 binary(claude / codex / gemini 不在 PATH,或 mori CLI 沒 bundled)
- `timed out` → 上層 180s timeout(claude 卡 OAuth / network),抓 `latency_ms` 接近 180000

`stdin_tail_preview` 看 claude / codex 拿到的最後幾百 chars(user message + 部分 history)。`response_preview` 看回的內容。對得起來嗎?

#### 「Mori 反應變奇怪 / context 不對」

`llm_call.system_prompt_chars` + `stdin_chars` 看 prompt 規模。如果突然變大 / 變小代表 context provider(時間 / 視窗 / 剪貼簿 / 反白)生壞 context。

### 5. 報告

別整段 dump JSONL 給 user — context 浪費。用表格 / 條列摘要關鍵 events + timestamps + 解讀。pattern 例:

```
時間軸(UTC):

03:18:12  wake_listener_spawned (進 Listening)
03:18:14  wake_listener_ready
03:18:16  wake_word_event score=0.96  ← 1st wake
03:18:21  silence_trim
03:18:33  skill_dispatch open_url ok=true   ← Chrome 真有開
03:18:33  agent_completed "已開啟 https://..."  ← 跟 dispatch 對齊
03:18:33  wake_listener_respawn → spawned → ready ← cycle 補位
03:18:41  wake_word_event score=0.91  ← 2nd wake ✓

結論:Hey Mori + Chrome 兩條都對。N 輪後不靜默 bug 沒重現。
```

## Cross-platform

- 路徑 `~/.mori/logs/` 全平台同 layout(`HOME`/`USERPROFILE` 解析)。
- `date +%Y-%m-%d` 跨平台 bash 都吃。
- JSONL utf-8,中文不會壞。

## 失敗 recovery

- **找不到當天 log** — 確認 Mori 真的有跑過(`~/.mori/runtime.json` 有沒有今天 timestamp)。Mori 沒啟動過 → 沒 log。
- **時區對不上 user 說的時間** — JSONL 是 UTC,user 在 GMT+8 → log timestamp + 8h = local。別搞錯 day boundary。
- **行數太多** — 用 `Read` tool 的 offset+limit 切段讀,別整檔吞。
