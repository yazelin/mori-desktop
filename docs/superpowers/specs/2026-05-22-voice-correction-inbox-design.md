# Voice Correction Inbox + Rating UI — Design Spec

**Status**: Approved (2026-05-22)
**Owner**: yazelin
**Source brainstorm**: 2026-05-22 session(reminder popup / cron skill 完成後)

## 1. 背景

`~/.mori/corrections.md` 是 voice input / agent profile 透過 `#file:` 引用的共用 STT 校正字典(116 行 baseline + user 自加段)。格式:`錯字, 錯字2 -> 正字`(多錯對 1 正)。

問題:**字典只能手動 vim 編輯**。User 每次發現新諧音錯字,要記住 → 打開 corrections.md → 找正確段落 → 加新行。門檻太高,user 不會做,corrections 字典就停滯。

但 Mori 自身已經有「LLM cleanup 知道哪些詞被改」的資訊 — voice pipeline 已把 `transcript_raw` + `transcript_cleaned` 雙版本寫進 event_log。LLM 在 cleanup 那步事實上每次都「校了一次」,只是這個校正知識**沒寫回字典**,下次同錯字 LLM 還要重 校。

## 2. 目標

讓 STT 校正字典變成**半自動成長**:
- 對話結束後 background LLM audit 偵測「可能錯字」候選 + AI 建議正字
- 候選進 `~/.mori/correction_inbox.jsonl`,UI「校正盒」一頁顯示
- User 一鍵接受 → append corrections.md(下次熱鍵就生效,LLM 不必再校)
- 評分 UI(👍 / 👎 / ✏️)平行入口:user 改寫 transcript = 確定正字,直接接受

User 是 quality control gate,LLM 是 candidate generator,corrections.md 是 deterministic 後盾。

## 3. 非目標

- **多裝置同步 corrections.md**:vault sync follow-up
- **Auto-accept by confidence**:全 user gate,confidence 只顯示不自動寫
- **Inbox 內聽 audio 回放**:預留 link 跳 RecordingsTab,MVP 不展開 audio player
- **corrections.md 直接編輯(diff-aware editor)**:MVP 提供 readonly view + system editor 開檔
- **dismissed 白名單 cleanup 命令**:MVP 不做 GC,append-only 累積
- **多帳號 / 多 spirit corrections**:現在 `~/.mori/corrections.md` 一份共用

## 4. 設計

### 4.1 架構與資料流

```
voice input pipeline 完成
    │
    ├─ event_log: voice_input_completed { transcript_raw, transcript_cleaned, target_process, profile }
    │
    └─ spawn correction_audit task(背景,不擋 user)
            │
            ▼
       Groq LLM audit call(若 correction_audit.enabled)
       input:{ transcript_raw, transcript_cleaned, corrections_md_full }
       prompt:「對照 raw → cleaned 差異 + corrections.md 已有 entries
                 + 語義推測,列出 raw 內可能是 STT 諧音錯字的詞 + 建議正字」
       output: JSON [{wrong, suggested, confidence, reason}]
            │
            ▼
   過濾:
   - LLM 幻覺(wrong 不在 transcript_raw)→ drop
   - 已在 dismissed 白名單 → drop
   - 已在 corrections.md(同 wrong → 同 suggested)→ drop
            │
            ▼
   append 進 ~/.mori/correction_inbox.jsonl(status: pending)
            │
            ▼
   Chat panel 評分 UI(👍 / 👎 / ✏️)平行入口:
       user 點 ✏️ 改寫 transcript → diff wrong→corrected →
       **直接接受進 corrections.md**(user 明確改 = 確定正字,不經 inbox)
       同時寫 feedback.json 記 rating
```

### 4.2 LLM Audit Prompt 結構

System prompt(寫入 mori-core 一個常數,follow-up 可拉成 prompt template):
```
你是 STT 諧音錯字偵測器。我會給你三段:
1. transcript_raw — STT 直接輸出(可能有諧音錯字、無標點)
2. transcript_cleaned — LLM 校正後版本(已用 corrections.md 處理過 + LLM 自己加標點 segmentation)
3. corrections.md — 現有校正字典(格式:錯字, 錯字2 -> 正字)

任務:列出 transcript_raw 內**可能是 STT 諧音錯字**的詞 + 建議正字。

判斷依據:
- raw → cleaned 過程被改寫的詞(LLM 校過 = 高機率錯字)
- 跟 corrections.md 內錯字組同音 / 形近(已知類型擴散)
- 語義不合 / 諧音怪 / 不像合理中文用語(新類型)

排除:
- 純標點 / segmentation 差異(LLM 加標點不算錯字)
- 已在 corrections.md 內的 entries(那條已經會用)

回 JSON array,每筆 { wrong, suggested, confidence: 0..1, reason: 一句中文說明 }。
找不到候選回 []。
```

Provider:`groq`,model `openai/gpt-oss-120b`(便宜),溫度低(0.2)避免幻覺。

### 4.3 Storage

**3 個檔**:

| 檔 | 形態 | 用途 |
|---|---|---|
| `~/.mori/correction_inbox.jsonl` | JSON Lines append-only | inbox entries + dismissed 白名單,status `pending` / `accepted` / `dismissed` |
| `~/.mori/recordings/<session>/feedback.json` | 一個 session 一檔 | 該 session 的 rating + corrected_transcript |
| `~/.mori/corrections.md`(既有) | markdown | append User 段,user 接受 inbox entry / 評分 ✏️ 後寫 |

**`correction_inbox.jsonl` entry shape**:
```json
{
  "id": "uuid-v4",
  "created_at": "2026-05-22T10:30:00+08:00",
  "source_session": "2026-05-22T18-30-00-xxx",
  "source": "llm_audit" | "user_edit",
  "wrong": "英檔",
  "suggested": "音檔",
  "confidence": 0.85,
  "reason": "raw → cleaned 改寫 + 同音諧音匹配 corrections.md 內既有錯字組",
  "status": "pending" | "accepted" | "dismissed",
  "accepted_at": null,
  "dismissed_at": null
}
```

**`source` 欄**:`llm_audit`(自動 audit 產生)or `user_edit`(user 點 ✏️ 改寫 diff 出來),UI 排序時 user_edit 優先顯示。

**`feedback.json` shape**:
```json
{
  "rating": "good" | "bad" | "edit",
  "rated_at": "2026-05-22T20:35:00+08:00",
  "corrected_transcript": null,
  "comment": null
}
```

### 4.4 UI 整合 — 新 `CorrectionsTab.tsx`

**Tab 位置**:跟 ConfigTab / DepsTab / SkillsTab / LogsTab / RecordingsTab 同層,加新 tab「校正」(`CorrectionsTab.tsx`)。

**Tab 內結構**(上下 panel,或上方 toolbar + 下方 list):

#### Section A:校正盒(Inbox)— pending entries
按 `suggested` 字 grouping:
```
🔔 校正盒(5 個 pending)

[音檔]   ← 英檔 (×3), 雲檔 (×1)
   來源: 2026-05-22T18-30, 2026-05-22T19-12, 2026-05-22T20-05, 2026-05-22T20-30
   [✓ 接受]  [改建議]  [✗ 忽略]

[Markdown] ← 馬當 (×2), modem (×1)
   來源: ...
   [✓ 接受]  [改建議]  [✗ 忽略]
```
- **[✓ 接受]**:append 進 corrections.md User 段(同 row 多 variant 用 `, ` 分隔),所有同 suggested entries 標 accepted
- **[改建議]**:inline 改 suggested 字 + 接受
- **[✗ 忽略]**:該 (wrong, suggested) pair 進 dismissed 白名單,下次 LLM audit 不再標

點 row 展開:列每個 variant + 來源 session link(點開跳 RecordingsTab 該 session)

#### Section B:corrections.md viewer
- Readonly markdown view(用 same chat-panel style)
- 上方按鈕「在外部編輯器開啟」→ Tauri `dialog::open` or shell open
- 區分 baseline 段(灰色)vs User 段(白色)
- 顯示 entry 數統計:「Baseline 95 條 / User 12 條」

### 4.5 Chat panel + RecordingsTab 評分 UI

每筆 voice message 旁(對齊 copy 按鈕同 row)加三按鈕:`👍 / 👎 / ✏️`

- **👍 / 👎**:點下去寫 `feedback.json` rating,不做別的
- **✏️**:popup inline editor,user 改寫 transcript:
  - 確認後寫 `feedback.json` `rating: "edit"` + `corrected_transcript`(完整改後版本,作為 ground-truth 訓練資料)
  - 對改後版本跑 token 級 diff(原 cleaned vs corrected),把改過的詞當 **inbox candidate**(status: pending,**不直接寫 corrections.md**)
  - 為什麼不直接寫:`✏️` 改寫可能含意圖修正(user 改成自己想說的)而非純 STT 諧音校正,LLM 分不出來,所以一律走 inbox 走 user 第二次 gate(在 CorrectionsTab 看到 → 點 [接受])
  - 候選 confidence 設高(0.95)+ `source: "user_edit"`,UI 上排在 LLM audit 候選前面

對於既有歷史 session(RecordingsTab 看到的),同樣三按鈕,點下去走同 path。

### 4.6 Settings(在 ConfigTab 通知 sub-tab 旁加新 sub-tab「校正」)

- `correction_audit.enabled: bool = true` — 對話結束跑 LLM audit
- `correction_audit.provider: string = "groq"` — provider 名
- `correction_audit.model: string = "openai/gpt-oss-120b"` — model 名(便宜)

寫進 `~/.mori/config.json` `correction_audit` 子樹,read-on-call(對齊 notification_config pattern)。

### 4.7 跟既有 corrections.md 對齊

**只有 user 在 CorrectionsTab 明確點 [✓ 接受] 才會 append**(評分 ✏️ 改寫只進 inbox,不直接寫)。

`corrections.md` 結構:
```
## Baseline(Mori 自帶)
### 常見對話 / 諧音校正
- 馬當, 馬档 -> Markdown
...
(Baseline 區 — 寫入時嚴格不動)

## User
### 用戶自加
- 英檔, 雲檔 -> 音檔
...
(User 區 — 寫入只動這裡)
```

若 `## User` section 不存在(fresh install / user 沒手動加過),寫入時自動建 + 加 `### 用戶自加` subsection。

**寫入規則**:
- 同 suggested 字已在 User 段存在(corrections.md 已有「X -> 音檔」)→ 在那行加 wrong variant: `X, 英檔 -> 音檔`
- 新 suggested 字 → 在 `### 用戶自加` 底下加新行: `- 英檔 -> 音檔`
- 對齊既有風格:縮排 `- `、`, ` 分隔 variants、` -> ` 分隔錯正
- 寫入用 atomic pattern:寫到 `.tmp` + rename(避免 voice profile 同時 read 撞 partial write)

**Baseline / User 段檢測**:解析 markdown heading,`## Baseline` 段內 entries 不動,`## User` 段內 append。若 corrections.md 結構被 user 手改成非預期 format,fallback 在檔尾 append(不動原內容)。

**Read-on-call** 機制不變:既有 voice profile / agent profile 透過 `#file:` 引用 corrections.md,下次熱鍵就生效,不必重啟 Mori。

## 5. Error handling

| 情境 | 處理 |
|---|---|
| Groq LLM audit call 失敗(network / quota) | log warn,inbox entry 不寫,user 不受影響(等下次對話再 audit) |
| LLM 回的 JSON malformed / schema 錯 | parse 失敗 log error,該次 audit 結果丟掉(不寫 inbox) |
| LLM 標出來的 `wrong` 字實際不在 transcript_raw 內 | filter 掉(LLM 幻覺) |
| User 點 [接受] 但 corrections.md 寫不入(disk full / 權限) | UI 顯示紅 chip「儲存失敗:<reason>」,entry status 不改,user 重試 |
| `correction_inbox.jsonl` corrupt(某行 JSON 壞) | 讀檔時逐行 parse,壞行 skip + log warn,其他正常 |
| `feedback.json` 寫入失敗 | 同上,inline 紅 chip 提示 |
| 評分 ✏️ corrected_transcript 跟原本一字不差 | 不建 inbox entry(user 沒實質改) |
| dismissed 白名單膨脹 | jsonl 內 dismissed entries 累積無上限 — MVP 不清理,follow-up 加 cleanup 命令 |
| Settings 讀檔失敗 | 走 default(enabled=true,groq/gpt-oss-120b),log warn |

## 6. Testing

**Unit / integration tests**(`cargo test` 過):

| 測試 | 檔 |
|---|---|
| `correction_inbox` 寫入 / 讀回 / merge by suggested word | `crates/mori-core/src/correction_inbox.rs`(新) |
| dismiss 進白名單後同 (wrong, suggested) 不重複建 entry | 同上 |
| LLM audit prompt → mock LLM response → 解析 JSON → entries 寫對 | 同上 + mock provider trait |
| corrupted jsonl 部分讀回 + warn | 同上 |
| feedback.json 寫入 / 讀回 / corrected_transcript diff 抽 wrong word | `crates/mori-core/src/voice_feedback.rs`(新) |
| 接受 inbox entry → corrections.md User 段 append + grouping 對 | `crates/mori-core/src/corrections_writer.rs`(新)|
| 同 suggested 已存在 → 合進同行而非新行 | 同上 |
| Settings `correction_audit` config 讀寫 round-trip | `crates/mori-tauri/src/correction_audit_config.rs`(新)|

**Manual smoke**(寫 PR checklist):
- [ ] 對 Mori 講「英檔有什麼內容」(故意諧音)→ Mori 校成「音檔」→ 對話結束後 LogsTab 看到 `kind: correction_audit_completed`,CorrectionsTab inbox 一筆「音檔 ← 英檔」
- [ ] 點 [✓ 接受] → corrections.md User 段多一行「英檔 -> 音檔」
- [ ] 同詞再講一次 → inbox 不再出現(corrections.md 已收錄,LLM audit 跳過)
- [ ] 點 [✗ 忽略] → entry 標 dismissed,同詞下次不再進 inbox
- [ ] Chat panel 對某 voice message 點 ✏️ 改寫 → corrected_transcript 寫進 feedback.json + 改過的詞直接 append corrections.md
- [ ] RecordingsTab 對歷史 session 點 👍 / 👎 → feedback.json rating 欄變化
- [ ] Settings 通知 sub-tab 旁「校正」sub-tab 切 `correction_audit.enabled = false` → 對話結束後不跑 LLM audit(LogsTab 無 audit event)
- [ ] 故意給 Groq invalid response(stub mock)→ audit fail log warn,user 不受影響

## 7. 開放決定 / Follow-up

- **完整 CorrectionsTab UI**:audio player 嵌入 / corrections.md inline edit
- **Dismissed 白名單 GC**:超過 N 天 / N 條自動清理
- **多裝置 sync corrections.md**:走 vault / world-tree
- **語義 dismiss grouping**:dismiss 一個 (wrong, suggested) 暗示同類型也 dismiss(LLM 判斷)
- **Auto-accept by confidence**:高 confidence 自動寫 corrections.md(Settings 加 threshold)
- **LLM audit prompt template 拉成檔**:`~/.mori/prompts/correction_audit.md` 給 user 客製化
- **Corrections.md 多人 / 多 spirit 區分**:目前一份共用

## 8. 變更影響

- **新檔**:
  - `crates/mori-core/src/correction_inbox.rs`(jsonl I/O + filter)
  - `crates/mori-core/src/voice_feedback.rs`(feedback.json I/O + diff)
  - `crates/mori-core/src/corrections_writer.rs`(append 進 corrections.md User 段)
  - `crates/mori-core/src/correction_audit.rs`(LLM audit call + JSON parse)
  - `crates/mori-tauri/src/correction_audit_config.rs`(Settings 讀寫)
  - `crates/mori-tauri/src/correction_cmd.rs`(Tauri commands wrapper)
  - `src/tabs/CorrectionsTab.tsx`(UI)
- **改既有**:
  - `crates/mori-tauri/src/main.rs`:voice pipeline 完成 hook spawn correction_audit + 註冊新 Tauri commands + mod registrations
  - `src/MainShell.tsx` 或 tabs config:加 CorrectionsTab
  - `src/ChatPanel.tsx`:每筆 voice message 旁加 👍 / 👎 / ✏️
  - `src/tabs/RecordingsTab.tsx`:同樣加評分按鈕
  - `src/tabs/ConfigTab.tsx`:通知 sub-tab 旁加「校正」sub-tab
- **新 event_log kind**:`correction_audit_started` / `correction_audit_completed` / `correction_inbox_accepted` / `correction_inbox_dismissed` / `voice_feedback_rated`
- **既有 voice pipeline / corrections.md / voice profile 機制不動**(只 hook 進 spawn audit + reads)
