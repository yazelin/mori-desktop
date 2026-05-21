//! K4 parser — 待實作(chrono-english 自然語言時間解析)
//!
//! TODO: 由 K4 stream impl。本 stub 確保 K1 ship 後 `lib.rs` 不 break。
//!
//! 預期 API(by K4):
//! - `parse_when(input: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, ParseError>`
//! - 支援「明天早上 9 點」/ "tomorrow 9am" / "in 30 minutes" 等
//! - cron pattern 偵測:「每天早上 8 點」→ cron expr
