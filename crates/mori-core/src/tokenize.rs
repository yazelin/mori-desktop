//! Token 數估算 — char-class 啟發法,0 deps,~±10% 準確度。
//!
//! 給 Profiles tab UI 顯示「這 profile system prompt 大概多少 token」用。
//! 不是精確 billing,是 UX hint:user 改 .md 時看得到「我這版比上版重多少」。
//!
//! ## 為什麼不 bundle tiktoken
//! - tiktoken-rs / tokenizers crate ~2-5MB binary 漲幅,Mori 已不小
//! - exact 對 Profiles UX 而言過度準確 — 「~512 vs ~520」對 user 沒差
//! - Gemini SentencePiece 沒 release,本來就無法 bundle exact 版
//!
//! ## Empirical ratios
//! 從 [`docs/tokenizer-comparison.md`](../../../docs/tokenizer-comparison.md) 4 對中英
//! starter(USER-03/04/05 + AGENT-05)實測得出的平均 chars/tok:
//!
//! | 內容類型 | gpt-oss (o200k_harmony) | Gemini Flash |
//! |---|---|---|
//! | 中文(CJK) | 1.50 chars/tok | 1.74 chars/tok |
//! | 非 CJK(英數 + 標點 + markdown) | ~3.8 chars/tok | ~3.5 chars/tok |
//!
//! 純英文 chars/tok 偏高(4-4.5),但 starter 通常含 markdown 標點 → 折衷取 3.8。
//! 「英文比較貴」的偏差約 5-10%,跟 docs 提到的 EN 比 ZH 省 26% / 8% directional 一致。

use serde::Serialize;

/// 一筆內容對兩家 tokenizer 的 token 估算。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TokenEstimate {
    /// gpt-oss-120b(`o200k_harmony` encoding)系列,Mori 預設 groq provider 走的
    pub gpt_oss: usize,
    /// Gemini Flash 系列(SentencePiece),中文比 gpt-oss 友善 14%、英文貴 7%
    pub gemini: usize,
}

/// 估算 `body` 在兩家 tokenizer 各為多少 token。±10% 範圍。
/// 空字串 / 全空白 → 0。
pub fn estimate_tokens(body: &str) -> TokenEstimate {
    let cjk_count = body.chars().filter(is_cjk).count();
    let non_cjk_count = body
        .chars()
        .filter(|c| !c.is_whitespace())
        .count()
        .saturating_sub(cjk_count);

    let gpt_oss = (cjk_count as f64 / 1.50 + non_cjk_count as f64 / 3.8).round() as usize;
    let gemini = (cjk_count as f64 / 1.74 + non_cjk_count as f64 / 3.5).round() as usize;

    TokenEstimate { gpt_oss, gemini }
}

/// 去掉 YAML frontmatter,只回 body(LLM 真正看到的 system prompt 部分)。
/// 跟 [`agent_profile`] 內的 frontmatter 偵測規則一致:第一行 `---` + 接著
/// 找到下一個 `\n---\n`。沒 frontmatter 回原樣。
pub fn strip_frontmatter(text: &str) -> &str {
    if !text.starts_with("---\n") {
        return text;
    }
    if let Some(end) = text.find("\n---\n") {
        return text[end + 5..].trim_start();
    }
    text
}

/// CJK 字判斷:Unified Han(常用)+ Extension A(較罕)+ Compatibility(舊版字)。
/// 不含 CJK 標點(全形括號等),那些走 non_cjk path 因為 token cost 跟英文標點同。
fn is_cjk(c: &char) -> bool {
    let code = *c as u32;
    (0x4E00..=0x9FFF).contains(&code)     // CJK Unified Ideographs
        || (0x3400..=0x4DBF).contains(&code)  // CJK Extension A
        || (0xF900..=0xFAFF).contains(&code)  // CJK Compatibility Ideographs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_zero() {
        let est = estimate_tokens("");
        assert_eq!(est.gpt_oss, 0);
        assert_eq!(est.gemini, 0);
    }

    #[test]
    fn whitespace_only_zero() {
        let est = estimate_tokens("   \n\n\t   ");
        assert_eq!(est.gpt_oss, 0);
        assert_eq!(est.gemini, 0);
    }

    #[test]
    fn pure_chinese_uses_cjk_ratio() {
        // 30 字 CJK → gpt-oss 約 30/1.5 = 20 tok,Gemini 30/1.74 ≈ 17 tok
        let s = "今天天氣很好我們去公園散步順便買點咖啡跟麵包回家當晚餐";
        assert_eq!(s.chars().count(), 27);
        let est = estimate_tokens(s);
        assert!((est.gpt_oss as i32 - 18).abs() <= 2, "gpt_oss={} expected ~18", est.gpt_oss);
        assert!((est.gemini as i32 - 16).abs() <= 2, "gemini={} expected ~16", est.gemini);
    }

    #[test]
    fn pure_english_uses_ascii_ratio() {
        // ~40 non-whitespace chars → gpt-oss ~10 tok, gemini ~11
        let s = "Hello world this is a translation profile.";
        let nws = s.chars().filter(|c| !c.is_whitespace()).count();
        assert_eq!(nws, 36);
        let est = estimate_tokens(s);
        assert!((est.gpt_oss as i32 - 9).abs() <= 2, "gpt_oss={} expected ~9", est.gpt_oss);
        assert!((est.gemini as i32 - 10).abs() <= 2, "gemini={} expected ~10", est.gemini);
    }

    #[test]
    fn strip_frontmatter_basic() {
        let s = "---\nprovider: groq\ntype: voice\n---\n\nbody content here";
        assert_eq!(strip_frontmatter(s), "body content here");
    }

    #[test]
    fn strip_frontmatter_no_fm_returns_original() {
        let s = "no frontmatter here, just body";
        assert_eq!(strip_frontmatter(s), s);
    }

    #[test]
    fn strip_frontmatter_unclosed_returns_original() {
        // 故意壞的 frontmatter(沒第二個 `---`)→ 回原樣,不 panic
        let s = "---\nbroken frontmatter no end marker\nbody";
        assert_eq!(strip_frontmatter(s), s);
    }

    #[test]
    fn directional_zh_more_tokens_than_en_on_gpt_oss() {
        // Sanity check:同訊息量(直覺上)中文 token 應該比英文多(gpt-oss)
        let zh = "翻譯這段中文成英文,輸出只給結果不要解釋。";
        let en = "Translate this Chinese to English. Output result only.";
        let zh_est = estimate_tokens(zh);
        let en_est = estimate_tokens(en);
        assert!(zh_est.gpt_oss > en_est.gpt_oss, "zh {} should > en {}", zh_est.gpt_oss, en_est.gpt_oss);
    }
}
