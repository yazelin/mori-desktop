//! K4 parser — 自然語言時間解析
//!
//! User input(中/英文)→ `chrono::DateTime<Utc>`。
//!
//! 支援例子:
//! - 英文(走 `chrono-english`):
//!   - `"6pm"` / `"tomorrow 9am"` / `"in 30 minutes"` / `"30 minutes"` / `"next monday"`
//! - 中文(走自寫薄 layer,std string ops,不引 regex):
//!   - `"30 分鐘後"` / `"1 小時後"` / `"2 天後"`
//!   - `"明天 9 點"` / `"明天早上 9 點"` / `"明天晚上 9 點"` / `"後天 9 點"`
//!   - `"6 點"` / `"晚上 6 點"` / `"早上 6 點"` / `"下午 3 點"` / `"中午"` / `"半夜"`
//!   - `"下週一"` ~ `"下週日"`(到 00:00,可選 trailing `" X 點"`)
//!
//! 過去時間 → `PastTime` error(嚴格 reject)。
//!
//! 兩條解析路徑:
//! 1. 先試 `chrono-english::parse_date_string`(UK dialect — 比較少奇怪 mm/dd 推斷)。
//! 2. fail 就試 `parse_chinese` 自寫 layer。
//! 3. 都 fail → `Unrecognized`。
//!
//! 結果若早於 now → `PastTime`。
//!
//! K5 commands 會用 [`parse_or_default`] 給 LLM tool call 餵 fallback default。

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc, Weekday};
use chrono_english::{parse_date_string, Dialect};

/// parse 錯誤類型。
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// 無法解析的 expression。
    #[error("can't parse time expression: {0}")]
    Unrecognized(String),
    /// 成功 parse 但結果在過去 — K1 reminder 不接受。
    #[error("parsed time in past: {0}")]
    PastTime(String),
}

/// 解析自然語言時間 expression 成 UTC `DateTime`。
///
/// 兩條 fallback:chrono-english(英文)→ 中文 layer。
/// 結果若在過去 → `PastTime` error。
pub fn parse(expr: &str) -> Result<DateTime<Utc>, ParseError> {
    parse_at(expr, Local::now())
}

/// 同 [`parse`],但 caller 控制「now」基準(tests 用)。
pub fn parse_at(expr: &str, now: DateTime<Local>) -> Result<DateTime<Utc>, ParseError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Unrecognized(expr.to_string()));
    }

    // 路徑 A:chrono-english(英文 NL)
    if let Ok(dt_local) = parse_date_string(trimmed, now, Dialect::Uk) {
        return finalize(dt_local.with_timezone(&Utc), now);
    }

    // 路徑 B:中文 fallback
    if let Some(dt_utc) = parse_chinese(trimmed, now) {
        return finalize(dt_utc, now);
    }

    Err(ParseError::Unrecognized(expr.to_string()))
}

/// 解析失敗就回 default — K5 commands 給 LLM tool call 用。
pub fn parse_or_default(expr: &str, default: DateTime<Utc>) -> DateTime<Utc> {
    parse(expr).unwrap_or(default)
}

fn finalize(
    candidate: DateTime<Utc>,
    now_local: DateTime<Local>,
) -> Result<DateTime<Utc>, ParseError> {
    let now_utc = now_local.with_timezone(&Utc);
    if candidate < now_utc {
        return Err(ParseError::PastTime(candidate.to_rfc3339()));
    }
    Ok(candidate)
}

// -----------------------------------------------------------------------------
// 中文 fallback — 簡薄 layer,std string ops 只
// -----------------------------------------------------------------------------

/// 中文 NL → UTC DateTime。不認 → `None`。
///
/// 試的順序:
/// 1. `X 分鐘/小時/天後`
/// 2. `下週X` (+ optional 時間)
/// 3. `(明天|後天)? (早上|上午|中午|下午|晚上|半夜)? N 點(M 分)?`
fn parse_chinese(expr: &str, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    // 全形空白也轉成半形再 trim,容忍一些 user 輸入。
    let normalized: String = expr.chars().map(|c| if c == '\u{3000}' { ' ' } else { c }).collect();
    let s = normalized.trim();

    if let Some(dt) = parse_relative_later(s, now) {
        return Some(dt);
    }
    if let Some(dt) = parse_next_weekday(s, now) {
        return Some(dt);
    }
    if let Some(dt) = parse_clock(s, now) {
        return Some(dt);
    }
    None
}

/// `30 分鐘後` / `1 小時後` / `2 天後` / `5分鐘以後`
fn parse_relative_later(s: &str, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    let suffixes = [("分鐘後", 60i64), ("分鐘以後", 60), ("分後", 60),
                    ("小時後", 3600), ("小時以後", 3600),
                    ("天後", 86400), ("天以後", 86400),
                    ("日後", 86400)];
    for (suffix, sec_per_unit) in suffixes {
        if let Some(prefix) = s.strip_suffix(suffix) {
            let n: i64 = prefix.trim().parse().ok()?;
            if n < 0 {
                return None;
            }
            let dt = now + Duration::seconds(n * sec_per_unit);
            return Some(dt.with_timezone(&Utc));
        }
    }
    None
}

/// `下週一` / `下週五` / `下週日`(+ optional `" X 點"` / `" 早上 X 點"`...)
fn parse_next_weekday(s: &str, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    // 先抓 weekday prefix。
    let weekday_pairs = [
        ("下週一", Weekday::Mon),
        ("下週二", Weekday::Tue),
        ("下週三", Weekday::Wed),
        ("下週四", Weekday::Thu),
        ("下週五", Weekday::Fri),
        ("下週六", Weekday::Sat),
        ("下週日", Weekday::Sun),
        ("下週天", Weekday::Sun),
        ("下星期一", Weekday::Mon),
        ("下星期二", Weekday::Tue),
        ("下星期三", Weekday::Wed),
        ("下星期四", Weekday::Thu),
        ("下星期五", Weekday::Fri),
        ("下星期六", Weekday::Sat),
        ("下星期日", Weekday::Sun),
        ("下星期天", Weekday::Sun),
    ];

    for (prefix, target_wd) in weekday_pairs {
        if let Some(rest) = s.strip_prefix(prefix) {
            // 目標日期 = 至少 +1 天的下一個對應 weekday(包含「下週」語意:
            // 中文「下週X」一般指「下一週」的那天,不是「這週 X 還沒到的那天」。
            // 簡化:從今天 +1 day 開始找,直到 weekday match,但若這樣只跳到「本週後幾天」就再 +7。
            let today = now.date_naive();
            let mut candidate = today + chrono::Duration::days(1);
            while candidate.weekday() != target_wd {
                candidate += chrono::Duration::days(1);
            }
            // 確保是「下一週」:如果 candidate 跟 today 在同一個 ISO week 直接 +7。
            if candidate.iso_week() == today.iso_week() {
                candidate += chrono::Duration::days(7);
            }

            let rest = rest.trim();
            let (hour, minute) = if rest.is_empty() {
                (9, 0) // default 早上 9 點
            } else if let Some((h, m)) = extract_clock(rest) {
                (h, m)
            } else {
                // 認 prefix 但 trailing 看不懂 — 還是先回 default 時間。
                (9, 0)
            };

            return build_local_dt(candidate, hour, minute, now);
        }
    }
    None
}

/// `6 點` / `早上 9 點` / `晚上 6 點半` / `下午 3 點` / `明天 9 點` / `明天早上 9 點`
fn parse_clock(s: &str, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    // 抓 day offset prefix(明天 / 後天 / 大後天 / 今天)。
    let (day_offset, rest) = if let Some(r) = s.strip_prefix("大後天") {
        (3i64, r.trim())
    } else if let Some(r) = s.strip_prefix("後天") {
        (2, r.trim())
    } else if let Some(r) = s.strip_prefix("明天") {
        (1, r.trim())
    } else if let Some(r) = s.strip_prefix("明日") {
        (1, r.trim())
    } else if let Some(r) = s.strip_prefix("今天") {
        (0, r.trim())
    } else if let Some(r) = s.strip_prefix("今日") {
        (0, r.trim())
    } else {
        (0, s)
    };

    // 特殊整點 keyword:中午 / 半夜
    if rest == "中午" {
        let date = now.date_naive() + chrono::Duration::days(day_offset);
        return build_local_dt(date, 12, 0, now);
    }
    if rest == "半夜" {
        let date = now.date_naive() + chrono::Duration::days(day_offset);
        return build_local_dt(date, 0, 0, now);
    }

    let (hour, minute) = extract_clock(rest)?;
    let target_date = now.date_naive() + chrono::Duration::days(day_offset);

    // 沒有 day prefix、結果時間又在 now 之前(eg 現在 20:00 user 講「6 點」)— 不自動推到明天,
    // 由 caller 決定(嚴格 PastTime)。spec 講 strict reject。
    build_local_dt(target_date, hour, minute, now)
}

/// 從中間 fragment 抽 `(時段)? N 點 (M 分)?` 的 `(hour, minute)`。
/// 不認回 `None`。
///
/// 支援 period prefix:`早上` / `上午` / `中午` / `下午` / `晚上` / `傍晚` / `凌晨` / `半夜`。
/// `下午 / 晚上 / 傍晚` 會把 1-11 點推成 13-23 點;`中午` 強制 12;`半夜 / 凌晨` 保留 0-x 不變,但 12 點半夜會變 0。
fn extract_clock(s: &str) -> Option<(u32, u32)> {
    let (period, rest) = strip_period(s);
    let rest = rest.trim();

    // `N 點半` / `N 點` / `N 點 M 分`
    // 先找 「點」
    let hour_end = rest.find('點')?;
    let hour_str = rest[..hour_end].trim();
    let hour: u32 = hour_str.parse().ok()?;

    let after_dian = rest[hour_end + '點'.len_utf8()..].trim();

    let minute: u32 = if after_dian.is_empty() {
        0
    } else if after_dian == "半" {
        30
    } else if let Some(m_str) = after_dian.strip_suffix('分') {
        m_str.trim().parse().ok()?
    } else if let Some(m_str) = after_dian.strip_suffix("分鐘") {
        m_str.trim().parse().ok()?
    } else {
        return None;
    };

    if minute >= 60 {
        return None;
    }

    let hour = apply_period(period, hour)?;
    if hour >= 24 {
        return None;
    }
    Some((hour, minute))
}

#[derive(Debug, Clone, Copy)]
enum Period {
    None,
    Morning, // 早上 / 上午 / 凌晨
    Noon,    // 中午
    Afternoon, // 下午
    Evening, // 晚上 / 傍晚 / 夜裡 / 夜晚
    Midnight, // 半夜 / 深夜
}

fn strip_period(s: &str) -> (Period, &str) {
    // 順序:長的 prefix 先試。
    let trials: &[(&str, Period)] = &[
        ("早上", Period::Morning),
        ("上午", Period::Morning),
        ("凌晨", Period::Morning),
        ("中午", Period::Noon),
        ("下午", Period::Afternoon),
        ("晚上", Period::Evening),
        ("傍晚", Period::Evening),
        ("夜晚", Period::Evening),
        ("夜裡", Period::Evening),
        ("半夜", Period::Midnight),
        ("深夜", Period::Midnight),
    ];
    for (prefix, period) in trials {
        if let Some(rest) = s.strip_prefix(prefix) {
            return (*period, rest);
        }
    }
    (Period::None, s)
}

fn apply_period(period: Period, hour: u32) -> Option<u32> {
    match period {
        Period::None => {
            if hour <= 24 {
                Some(hour % 24)
            } else {
                None
            }
        }
        Period::Morning => {
            // 早上 12 點 → 0;早上 1-11 → 不變;早上 12+ → 怪,reject。
            if hour == 12 {
                Some(0)
            } else if hour < 12 {
                Some(hour)
            } else {
                None
            }
        }
        Period::Noon => {
            // 中午 12 點 → 12;中午 N 點 (N != 12) — 比較怪,接受 12 並 ignore N。
            // 嚴格點:N == 12 才接受。
            if hour == 12 {
                Some(12)
            } else {
                None
            }
        }
        Period::Afternoon => {
            // 下午 1-11 → 13-23;下午 12 → 12。
            if hour == 12 {
                Some(12)
            } else if (1..12).contains(&hour) {
                Some(hour + 12)
            } else {
                None
            }
        }
        Period::Evening => {
            // 晚上 6 點 → 18 / 晚上 11 → 23 / 晚上 12 → 0(過了午夜) / 晚上 7 → 19。
            if hour == 12 {
                Some(0)
            } else if (1..12).contains(&hour) {
                Some(hour + 12)
            } else {
                None
            }
        }
        Period::Midnight => {
            // 半夜 12 → 0;半夜 1 → 1;半夜 N (N >= 12) — reject。
            if hour == 12 {
                Some(0)
            } else if hour < 12 {
                Some(hour)
            } else {
                None
            }
        }
    }
}

fn build_local_dt(
    date: NaiveDate,
    hour: u32,
    minute: u32,
    _now: DateTime<Local>,
) -> Option<DateTime<Utc>> {
    let naive = date.and_hms_opt(hour, minute, 0)?;
    let local = Local
        .from_local_datetime(&naive)
        .single()
        .or_else(|| {
            // DST 邊界:取較早的那個（保守）。
            Local.from_local_datetime(&naive).earliest()
        })?;
    Some(local.with_timezone(&Utc))
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as ChronoDuration, TimeZone};

    /// fixed local "now" — 2026-05-22 (Fri) 10:00:00 local。
    /// 用 fixed base 避免 flaky / DST,但有些 test 還是用 `Local::now()`。
    fn fixed_now() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 5, 22, 10, 0, 0)
            .single()
            .expect("fixed now")
    }

    fn assert_close(a: DateTime<Utc>, b: DateTime<Utc>, tol_sec: i64) {
        let diff = (a - b).num_seconds().abs();
        assert!(
            diff <= tol_sec,
            "expected within {tol_sec}s, got {diff}s ({a} vs {b})"
        );
    }

    // ----- 英文 (chrono-english) -----

    #[test]
    fn parse_english_relative_minutes() {
        let now = fixed_now();
        let expected = (now + ChronoDuration::minutes(30)).with_timezone(&Utc);
        let got = parse_at("30 minutes", now).expect("parse 30 minutes");
        // chrono-english 的 "30 minutes" 是相對 base — 應該 = base + 30min。
        assert_close(got, expected, 2);

        let got2 = parse_at("30m", now).expect("parse 30m");
        assert_close(got2, expected, 2);
    }

    #[test]
    fn parse_english_tomorrow_time() {
        let now = fixed_now();
        let got = parse_at("tomorrow 9am", now).expect("parse tomorrow 9am");
        // 2026-05-23 09:00 local
        let expected = Local
            .with_ymd_and_hms(2026, 5, 23, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got, expected);
    }

    #[test]
    fn parse_english_next_monday() {
        let now = fixed_now(); // 2026-05-22 (Fri)
        // chrono-english Dialect::Uk: "next mon" → next week's Monday = 2026-06-01
        let got = parse_at("next mon", now).expect("parse next mon");
        let expected = Local
            .with_ymd_and_hms(2026, 6, 1, 0, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got, expected);
    }

    #[test]
    fn parse_english_past_time_errors() {
        let now = fixed_now(); // 10:00
        // "3 hours ago" → 2026-05-22 07:00 — 過去。
        match parse_at("3 hours ago", now) {
            Err(ParseError::PastTime(_)) => {}
            other => panic!("expected PastTime, got {other:?}"),
        }
    }

    #[test]
    fn parse_english_unrecognized_errors() {
        let now = fixed_now();
        match parse_at("blah blah blah xyzzy", now) {
            Err(ParseError::Unrecognized(s)) => {
                assert!(s.contains("blah"));
            }
            other => panic!("expected Unrecognized, got {other:?}"),
        }

        // 空 string 也應該 Unrecognized
        match parse_at("   ", now) {
            Err(ParseError::Unrecognized(_)) => {}
            other => panic!("expected Unrecognized for empty, got {other:?}"),
        }
    }

    // ----- 中文 -----

    #[test]
    fn parse_chinese_minutes_later() {
        let now = fixed_now();
        let got = parse_at("30 分鐘後", now).expect("30 分鐘後");
        let expected = (now + ChronoDuration::minutes(30)).with_timezone(&Utc);
        assert_close(got, expected, 2);

        // 1 小時後
        let got2 = parse_at("1 小時後", now).expect("1 小時後");
        let expected2 = (now + ChronoDuration::hours(1)).with_timezone(&Utc);
        assert_close(got2, expected2, 2);

        // 2 天後
        let got3 = parse_at("2 天後", now).expect("2 天後");
        let expected3 = (now + ChronoDuration::days(2)).with_timezone(&Utc);
        assert_close(got3, expected3, 2);

        // 無 space 也 OK
        let got4 = parse_at("45分鐘後", now).expect("45分鐘後");
        let expected4 = (now + ChronoDuration::minutes(45)).with_timezone(&Utc);
        assert_close(got4, expected4, 2);
    }

    #[test]
    fn parse_chinese_tomorrow_time() {
        let now = fixed_now();
        // 明天 9 點 → 2026-05-23 09:00 local
        let got = parse_at("明天 9 點", now).expect("明天 9 點");
        let expected = Local
            .with_ymd_and_hms(2026, 5, 23, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got, expected);

        // 明天早上 9 點 (帶 period)
        let got2 = parse_at("明天早上 9 點", now).expect("明天早上 9 點");
        assert_eq!(got2, expected);

        // 明天晚上 6 點 → 2026-05-23 18:00 local
        let got3 = parse_at("明天晚上 6 點", now).expect("明天晚上 6 點");
        let expected3 = Local
            .with_ymd_and_hms(2026, 5, 23, 18, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got3, expected3);
    }

    #[test]
    fn parse_chinese_today_time() {
        let now = fixed_now(); // 10:00
        // 下午 3 點 → today 15:00
        let got = parse_at("下午 3 點", now).expect("下午 3 點");
        let expected = Local
            .with_ymd_and_hms(2026, 5, 22, 15, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got, expected);

        // 晚上 8 點半 → today 20:30
        let got2 = parse_at("晚上 8 點半", now).expect("晚上 8 點半");
        let expected2 = Local
            .with_ymd_and_hms(2026, 5, 22, 20, 30, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got2, expected2);

        // 中午 → today 12:00
        let got3 = parse_at("中午", now).expect("中午");
        let expected3 = Local
            .with_ymd_and_hms(2026, 5, 22, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got3, expected3);
    }

    #[test]
    fn parse_chinese_next_monday() {
        let now = fixed_now(); // 2026-05-22 (Fri) — ISO week 21
        // 下週一 → 2026-05-25 (Mon) 09:00
        let got = parse_at("下週一", now).expect("下週一");
        let expected = Local
            .with_ymd_and_hms(2026, 5, 25, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got, expected);

        // 下週五 — 2026-05-22 (今天) 也是 Fri,所以 下週五 = 2026-05-29
        let got2 = parse_at("下週五", now).expect("下週五");
        let expected2 = Local
            .with_ymd_and_hms(2026, 5, 29, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got2, expected2);

        // 下週日 3 點 — 2026-05-31 (Sun) 15:00 (因為「3 點」要 ambient period... 預設無 period → 直接 3:00)
        // 實際:無 period 直接 hour=3 → 03:00。
        let got3 = parse_at("下週日 3 點", now).expect("下週日 3 點");
        let expected3 = Local
            .with_ymd_and_hms(2026, 5, 31, 3, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(got3, expected3);
    }

    #[test]
    fn parse_chinese_past_time_errors() {
        let now = fixed_now(); // 10:00
        // 早上 6 點 → today 06:00 < 10:00 → PastTime
        match parse_at("早上 6 點", now) {
            Err(ParseError::PastTime(_)) => {}
            other => panic!("expected PastTime, got {other:?}"),
        }
    }

    #[test]
    fn parse_or_default_fallback() {
        let default = Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).single().unwrap();
        // unrecognized → 回 default
        let got = parse_or_default("nonsensical wibble wobble", default);
        assert_eq!(got, default);

        // 過去時間也走 default(parse() return Err)
        let got2 = parse_or_default("3 hours ago", default);
        assert_eq!(got2, default);

        // 有效 expression 不走 default — parse() success
        let now = Local::now();
        let expected = (now + ChronoDuration::minutes(10)).with_timezone(&Utc);
        let got3 = parse_or_default("10 minutes", default);
        // 用 5 秒 tolerance — wall clock 不固定
        let diff = (got3 - expected).num_seconds().abs();
        assert!(
            diff <= 5,
            "parse_or_default for valid input should match (diff {diff}s)"
        );
    }

    // ----- 內部 helpers smoke -----

    #[test]
    fn extract_clock_basic() {
        assert_eq!(extract_clock("9 點"), Some((9, 0)));
        assert_eq!(extract_clock("9點"), Some((9, 0)));
        assert_eq!(extract_clock("9 點 30 分"), Some((9, 30)));
        assert_eq!(extract_clock("9點30分"), Some((9, 30)));
        assert_eq!(extract_clock("8 點半"), Some((8, 30)));
        assert_eq!(extract_clock("下午 3 點"), Some((15, 0)));
        assert_eq!(extract_clock("晚上 6 點"), Some((18, 0)));
        assert_eq!(extract_clock("早上 9 點"), Some((9, 0)));
        assert_eq!(extract_clock("中午 12 點"), Some((12, 0)));
        // 無效 / 看不懂 → None
        assert_eq!(extract_clock("無關"), None);
        // 60 分 不合法
        assert_eq!(extract_clock("9 點 60 分"), None);
    }
}
