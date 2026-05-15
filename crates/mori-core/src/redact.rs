//! Pre-LLM secret redaction。
//!
//! 進 LLM system prompt 前,把 clipboard / selection 內疑似 API key / Bearer
//! token 等高敏感字串替換成 `<REDACTED:probable-secret>`。設計目的:
//!
//! - **provider 端洩漏防護** — Mori 把 clipboard 內容送進 Groq / Gemini /
//!   Claude API 推理時,user 剛複製的 `gsk_*` / `sk-*` / `AIzaSy*` 不該整段過去
//! - **本機磁碟洩漏防護**(v0.5 Phase B 加 per-pipeline artifacts 時)— 寫進
//!   `~/.mori/recordings/<時戳>/` 也要先 redact
//!
//! ## 偵測樣式
//! 從**精準**到**寬鬆**,順序重要(精準先吃掉,避免被 fallback 過度遮蔽):
//!
//! 1. `gsk_[A-Za-z0-9]{40,}` — Groq API key
//! 2. `sk-[A-Za-z0-9_-]{40,}` — OpenAI / Anthropic style
//! 3. `AIzaSy[A-Za-z0-9_-]{30,}` — Google Cloud(Gemini / Maps / etc.)
//! 4. `Bearer\s+[A-Za-z0-9._-]{20,}` — HTTP Authorization header
//! 5. `[A-Za-z0-9_\-]{40,}` — 寬鬆 fallback,長高熵字串可能是未知 token
//!
//! ## 不做什麼
//! - **不**處理 PII(身分證 / 信用卡 / 電話)— 那需要更精準的 schema,且偽陽性高
//! - **不**處理 password / 私訊 — 純文字無法可靠分辨
//! - **不**對 audio 做任何處理 — 它存的話是另一層問題(opt-in)
//!
//! ## 為什麼回 `(String, usize)`
//! Caller 可拿 count 寫 audit log(event_log)— 讓 user 看得到「這次 LLM call
//! 我們替你遮蔽了 N 個疑似 secret」,但**不存原文**,redaction event 本身不
//! 包含被遮的字串內容。

use std::sync::OnceLock;

use regex::Regex;

/// Replacement marker。包含 `:probable-secret` 讓 LLM 知道是有意遮蔽,而不是
/// 文本本來就有奇怪佔位符。
pub const REDACTION_MARKER: &str = "<REDACTED:probable-secret>";

fn patterns() -> &'static [(&'static str, Regex)] {
    static CELL: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    CELL.get_or_init(|| {
        // 順序:精準 → 寬鬆。Regex::new 一次性建,後續呼叫 O(1) 拿。
        // 失敗(編譯期 typo)直接 panic — 我們有 unit test 鎖住所有 pattern 正確。
        vec![
            ("groq", Regex::new(r"gsk_[A-Za-z0-9]{40,}").unwrap()),
            ("openai_anthropic", Regex::new(r"sk-[A-Za-z0-9_\-]{40,}").unwrap()),
            ("google", Regex::new(r"AIzaSy[A-Za-z0-9_\-]{30,}").unwrap()),
            (
                "bearer",
                Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-]{20,}").unwrap(),
            ),
            // 寬鬆 fallback:純 alphanumeric/_/-,連續 40+ 字。可能 false positive
            // (例如長 base64 hash),但 LLM 看 marker 知道有意 redact,影響有限。
            ("high_entropy", Regex::new(r"[A-Za-z0-9_\-]{40,}").unwrap()),
        ]
    })
}

/// 掃 input,替換所有疑似 secret 樣式。回 `(redacted_text, hit_count)`。
///
/// 每 pattern 都跑一輪 `replace_all`,前面 pattern 命中的部分變成
/// `<REDACTED:...>` marker,marker 內無 40+ 字 alphanumeric 連續所以
/// 不會被後續 pattern 二次命中。
pub fn redact_secrets(input: &str) -> (String, usize) {
    let mut result = input.to_string();
    let mut total = 0usize;
    for (_, re) in patterns() {
        let count = re.find_iter(&result).count();
        if count > 0 {
            result = re.replace_all(&result, REDACTION_MARKER).into_owned();
            total += count;
        }
    }
    (result, total)
}

/// 純查詢版 — 不替換,只回 hit count。給「不想動內容只想算 audit」場景用。
/// 直接 delegate 給 `redact_secrets`(避免精準 pattern + fallback 重複計)。
#[allow(dead_code)]
pub fn count_secrets(input: &str) -> usize {
    redact_secrets(input).1
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test fixtures 用合成 secret(明顯 fake 的 prefix `_TEST_` / 全部 X)。
    // 不能複製真實洩漏的 sample key 進來 — GitHub secret scanning 會誤判 push
    // 被擋,且就算 fake 的也會養成複製真值的壞習慣。

    const FAKE_GROQ: &str = "gsk_TESTXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX";
    const FAKE_GOOGLE: &str = "AIzaSyTESTXXXXXXXXXXXXXXXXXXXXXXXXXXX";
    const FAKE_OPENAI: &str = "sk-test-XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX";
    const FAKE_BEARER: &str = "Bearer fakeXXXXXXXXXXXXXXXXXXXX.fakeXXXXXX.fakeXXXXX";
    const FAKE_HIGH_ENTROPY: &str =
        "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH";

    #[test]
    fn redacts_groq_key() {
        let s = format!("API key 是 {FAKE_GROQ} 拿去用");
        let (out, n) = redact_secrets(&s);
        assert!(out.contains(REDACTION_MARKER));
        assert!(!out.contains("gsk_TEST"));
        assert_eq!(n, 1);
    }

    #[test]
    fn redacts_google_key() {
        let s = format!("Gemini key: {FAKE_GOOGLE} done");
        let (out, n) = redact_secrets(&s);
        assert!(out.contains(REDACTION_MARKER));
        assert!(!out.contains("AIzaSy"));
        assert_eq!(n, 1);
    }

    #[test]
    fn redacts_openai_style() {
        let s = format!("{FAKE_OPENAI} rest");
        let (out, n) = redact_secrets(&s);
        assert!(out.contains(REDACTION_MARKER));
        assert_eq!(n, 1);
    }

    #[test]
    fn redacts_bearer_header() {
        let s = format!("Header: {FAKE_BEARER} done");
        let (out, n) = redact_secrets(&s);
        assert!(out.contains(REDACTION_MARKER));
        assert_eq!(n, 1);
    }

    #[test]
    fn multiple_keys_in_one_string() {
        let s = format!("{FAKE_GROQ} 跟 {FAKE_GOOGLE} 都");
        let (_out, n) = redact_secrets(&s);
        assert_eq!(n, 2, "兩條 key 應該都被找到");
    }

    #[test]
    fn safe_text_unchanged() {
        let s = "今天天氣很好,翻譯成英文。";
        let (out, n) = redact_secrets(s);
        assert_eq!(out, s);
        assert_eq!(n, 0);
    }

    #[test]
    fn short_alphanumeric_not_redacted() {
        // 短字串不該被當 secret(避免誤觸普通英文 / 短 id)
        let s = "user_id=abc123 commit=4e16005";
        let (out, n) = redact_secrets(s);
        assert_eq!(out, s);
        assert_eq!(n, 0);
    }

    #[test]
    fn high_entropy_fallback_catches_unknown_token() {
        // 40 字以上連續 alphanumeric,即使不 match 已知 prefix 也該遮
        let s = format!("token={FAKE_HIGH_ENTROPY} end");
        let (out, n) = redact_secrets(&s);
        assert!(out.contains(REDACTION_MARKER));
        assert_eq!(n, 1);
    }

    #[test]
    fn count_secrets_doesnt_mutate() {
        let s = format!("{FAKE_GROQ} here");
        let before = s.clone();
        let n = count_secrets(&s);
        assert_eq!(n, 1);
        assert_eq!(s, before);
    }
}
