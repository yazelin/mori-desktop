# Meeting Recorder 決議

> 狀態:design decision,尚未實作。
> 目的:定義未來 Mori Meeting Recorder 的專案邊界、會議錄音 / 即時字幕 /
> 客戶版與內部版紀錄的資料邊界。

## 背景

目前 Transcribe tab 的 meeting mode 是單一路麥克風長錄音:開始錄音,停止後存 WAV 到
`~/.mori/meetings/`,再用 whisper-local 轉錄。這不足以支援正式會議場景,因為:

- Google Meet / Zoom / Teams 的對方聲音通常在系統輸出音訊。
- 本機麥克風會收到現場私聊、旁白、策略討論,不應預設進客戶版會議紀錄。
- 同一台電腦如果同時負責發言,使用者自己的聲音通常不會從系統輸出回放,客戶版可能漏掉我方發言。
- 未來需要錄音中即時字幕,而不是只在停止後轉整段音檔。

## 核心決議

Meeting Recorder 應採用 **standalone-first** 設計,未來專案名稱暫定
**Mori Meeting Recorder**。

這不是 Mori Desktop chat / Transcribe tab 的附屬功能,而是 Mori universe 內一個可獨立運行的會議記錄工具。Mori Desktop 未來可以整合它,但不應擁有它的 raw audio、internal transcript、session lifecycle。

第一版實作應優先建立獨立 repo,而不是直接塞進 mori-desktop:

```text
mori-meeting-recorder/
  owns:
    audio capture
    system loopback
    mic capture
    realtime captions
    public/internal export
    session storage

mori-desktop/
  may later:
    open Mori Meeting Recorder
    import/export meeting.public.md
    ask Mori to summarize selected outputs
    share provider/model settings
```

Mori Desktop 和 Mori Meeting Recorder 的連接面應保持低耦合,大致只包含:

- STT provider / API key / model path 等使用者設定。
- 匯出檔案或 session metadata 的 handoff。
- 未來可選的 CLI / local HTTP API。
- 使用者明確要求時,才把指定 public/internal 產物交給 Mori 整理。

Mori Desktop / Annuli / Mori agent 不得自動讀取 `mic_internal` raw audio 或 internal transcript,也不得把內部私聊自動寫入 memory / knowledge。

## MVP 錄音模式

第一版 Meeting Recorder 採用 **Observer Mode / 旁聽錄音模式**:

- 錄音電腦加入會議,只收聽,不在該台電腦對 Google Meet 解除麥克風靜音。
- `system` 軌收錄會議輸出音訊,視為客戶版會議紀錄來源。
- `mic` 軌收錄本機麥克風,視為公司內部備忘來源。
- 客戶版紀錄永遠不預設包含 `mic` 軌內容。

這個模式的產品規則是:

```text
客戶版會議紀錄 = system 軌
內部備忘       = mic 軌
```

如果團隊需要對客戶發言,建議由另一台裝置、會議室設備、或另一個會議帳號發言,讓錄音電腦持續維持旁聽角色。如此錄音電腦收到的 system 音訊就能代表完整正式會議內容。

## 來源與可見性

Meeting Recorder 的資料模型必須把「音訊來源」和「可見性」分開,避免日後把私聊混進客戶版輸出。

| source_kind | 來源 | 預設 visibility | 用途 |
|---|---|---|---|
| `meeting_system` | 系統輸出 / 會議接收音訊 | `public` | 客戶版會議紀錄 |
| `mic_internal` | 本機麥克風 / 現場私聊 | `internal` | 公司內部備忘 |
| `mic_public` | 使用者明確標記的對外發言 | `public` | 未來 Presenter Mode |

MVP 只需要 `meeting_system` 和 `mic_internal`。`mic_public` 保留給未來同機發言模式,不列入第一版必要範圍。

## 不混音原則

原始音訊必須多軌保存,不要先混成一條再轉錄。

建議輸出結構:

```text
~/.mori/meetings/<session-id>/
  audio/
    system.wav
    mic-internal.wav
    mix-preview.wav          # 只供本機回放檢查,不作為客戶版來源
  transcript/
    system.segments.jsonl
    mic-internal.segments.jsonl
  meeting.public.md
  meeting.internal.md
  timeline.json
```

`mix-preview.wav` 可以幫使用者回聽整場,但不能當成正式轉錄來源。正式文件應從帶 metadata 的 segment 產生。

## 即時字幕

即時字幕也要分流,不做單一混合字幕。

UI 概念:

```text
會議字幕
[00:12:03] 客戶:我們希望下週三前看到版本。

內部字幕
[00:12:08] 我方私聊:這個時程可能要先保守回覆。
```

字幕 segment 建議帶這些欄位:

```json
{
  "id": "seg_001",
  "session_id": "meeting-20260527-143000",
  "track": "system",
  "source_kind": "meeting_system",
  "visibility": "public",
  "start_ms": 123000,
  "end_ms": 128500,
  "text": "我們希望下週三前看到版本。",
  "is_final": false,
  "confidence": null
}
```

錄音中可先顯示 `is_final:false` 的草稿字幕;停止錄音後再整理成 `is_final:true` 的正式 segment。

## 轉錄策略

實作順序採漸進式:

1. 停止後轉錄雙軌:先把 system / mic 各自存 WAV,停止後各自轉錄。
2. 準即時 chunk 轉錄:錄音中每 10-30 秒切片送 whisper-local,產生草稿字幕。
3. VAD 分句轉錄:偵測停頓後送出完整語句,減少切半句與延遲。
4. 真 streaming STT:未來再評估更適合 token/partial streaming 的引擎。

第一版即時字幕不要求真正 token streaming。可接受短延遲的 chunk 字幕,但必須維持來源分流。

## 匯出規則

`meeting.public.md`:

- 只包含 `visibility: public` 的 segment。
- MVP 中等同 `meeting_system` 軌。
- 不包含本機 mic 私聊、策略討論、旁白。

`meeting.internal.md`:

- 包含 `visibility: internal` 的 segment。
- 可引用 public 會議內容作上下文,但輸出檔本身標示為內部用途。

`timeline.json`:

- 保存所有 segment、音軌、來源、visibility、時間戳、檔案路徑。
- 是後續搜尋、回放、摘要、人工校正的 canonical metadata。

## Presenter Mode 留白

如果同一台電腦未來要同時在 Google Meet 發言,才進入 Presenter Mode。

Presenter Mode 需要:

- 明確的 `On air` / `Mic Public` 狀態。
- `On air` 期間的 mic segment 才能進客戶版紀錄。
- 會後允許把 mic segment 從 internal 提升為 public,或從 public 撤回 internal。

但這不是 MVP。MVP 應先把 Observer Mode 做穩,避免把 Google Meet mute 狀態、自動化快捷鍵、瀏覽器分頁偵測綁進第一版。

## 風險與限制

- 錄整個 system output 時,其他 App 的通知聲或媒體也可能進 `meeting_system`。MVP 必須在 UI 清楚告知。
- 如果不用耳機,會議聲音可能從喇叭漏進 mic 軌,造成 internal transcript 重複。後續可做去重或 echo suppression。
- 若錄音電腦同時在 Google Meet 發言,system 軌通常不會錄到使用者自己的聲音。Observer Mode 的前提是錄音電腦只旁聽。
- Mori 不能假裝知道 Google Meet 是否真的 mute/unmute。未來 Presenter Mode 必須以使用者明確狀態為準。

## 非目標

第一版不做:

- 直接在 mori-desktop 內完成完整 Meeting Recorder MVP。
- 指定單一 Google Meet 視窗或分頁音訊。
- 自動偵測 Google Meet 麥克風是否解除靜音。
- 把 mic 軌自動混進客戶版會議紀錄。
- 雲端 relay、central OAuth hub、第三方資料中介。

## Standalone 實作提示

新 repo 應先專注建立可測試的 audio / caption core,不要依賴 Mori Desktop 的狀態機或 UI:

- `MeetingRecorder`:管理多來源、多音軌、session manifest、即時字幕事件。
- `AudioCapture`:平台音訊 capture 抽象,分別實作 Windows WASAPI loopback、macOS ScreenCaptureKit、Linux PipeWire。
- `SessionStore`:保存 raw audio、segments、timeline、exports。
- `Exporter`:只根據 `visibility` 產生 public/internal 文件。

可先做 standalone app,再補 CLI / local HTTP API。API 可朝這個形狀演進:

```text
meeting_recorder_start(config)
meeting_recorder_stop(session_id)
meeting_recorder_status(session_id)
meeting_recorder_list_sources()
meeting_recorder_export(session_id, visibility)
```

UI 應顯示至少兩個來源狀態:

- 會議音訊:系統輸出,客戶版來源。
- 內部麥克風:本機收音,內部來源。

UI 文案要明確:「客戶版只使用會議音訊;內部麥克風不會預設匯出給客戶。」

## Mori Desktop 整合留白

等 Mori Meeting Recorder standalone MVP 穩定後,mori-desktop 可以用低耦合方式整合:

- 開啟或啟動 Mori Meeting Recorder。
- 顯示最近 session 列表。
- 匯入 `meeting.public.md` 作為可整理的會議素材。
- 在使用者明確選取後,把 `meeting.internal.md` 交給 Mori 產生內部摘要。
- 共用 provider/model 設定,但不共用 raw internal data。

這個整合不是第一版 blocker。第一版成功標準是 Mori Meeting Recorder 自己能獨立完成雙軌錄音、即時字幕分流、public/internal export。
