//! Phase 5E voice-input 程式化後處理。
//!
//! 不管 LLM 有沒有先過一輪,whisper / LLM 出來的東西都需要這層守住格式
//! 一致性 — 跨 provider / 跨 model 結果常飄(claude 可能輸出半形,qwen
//! 可能漏標點,gpt-oss 可能加引號)。這層程式化規則做 **deterministic**
//! 收尾。
//!
//! ## 三級 cleanup_level
//! - `smart`(預設):LLM 加標點 + segmentation,**然後**過 [`programmatic_cleanup`]
//!   收一致性。LLM 做 irreducible 智能,程式守格式。
//! - `minimal`:跳過 LLM,只跑 [`programmatic_cleanup`]。最快(~ms 級),但
//!   Whisper 不出標點所以結果會是「一長串字」, 適合「我自己後面加標點」型
//!   user 或 quota 緊張時。
//! - `none`:Whisper 出來的字直接 paste,跳所有 cleanup。
//!
//! ## `programmatic_cleanup` 做的事
//! 1. **Strip Whisper 幻聽**:常見英文字幕殘骸(`thank you for watching` /
//!    `subscribe` / `感謝觀看` / `請訂閱按讚`)直接拿掉
//! 2. **半形 → 全形 punctuation**(只當左右鄰是 CJK 字才轉,否則保留 — 不
//!    動英文標點)
//! 3. **去掉 LLM 偶爾包整段的 `「」` / `""` 引號**(輸出純文字,不要它幫你
//!    包)
//! 4. **Normalize 空白**:trim、合併多重空白、刪空行
//!
//! ## 為什麼**沒**做 OpenCC 簡→繁
//! whisper.cpp 我們已經用 `initial_prompt` 把它 bias 到繁體中文(看
//! `whisper_local.rs` 的 prompt — 全用繁體用語),實測幾乎不會吐簡體字。
//! LLM cleanup 也都是繁體優先。真的要保底再加 `opencc-rust` 系統依賴
//! 不太划算。如果之後遇到 mixed-script 問題再回來補。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CleanupLevel {
    /// LLM 加標點 + 程式守格式(預設)
    #[default]
    Smart,
    /// 只跑程式處理,跳 LLM
    Minimal,
    /// 完全不處理,raw whisper 直貼
    None,
}

impl CleanupLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            CleanupLevel::Smart => "smart",
            CleanupLevel::Minimal => "minimal",
            CleanupLevel::None => "none",
        }
    }
}

/// 從 `~/.mori/config.json` 讀 `voice_input.cleanup_level`。沒設或解析
/// 失敗都退到 `Smart`(預設)。
pub fn read_cleanup_level() -> CleanupLevel {
    let path = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".mori").join("config.json"));
    let Some(path) = path else {
        return CleanupLevel::Smart;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return CleanupLevel::Smart;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return CleanupLevel::Smart;
    };
    match json
        .pointer("/voice_input/cleanup_level")
        .and_then(|v| v.as_str())
    {
        Some("smart") => CleanupLevel::Smart,
        Some("minimal") => CleanupLevel::Minimal,
        Some("none") => CleanupLevel::None,
        _ => CleanupLevel::Smart,
    }
}

/// 程式化 cleanup — 純 string-in / string-out,所有規則 deterministic。
pub fn programmatic_cleanup(input: &str) -> String {
    let mut s = input.to_string();
    s = strip_whisper_hallucinations(&s);
    s = strip_wrapping_quotes(&s);
    s = halfwidth_to_fullwidth_near_cjk(&s);
    s = normalize_whitespace(&s);
    s
}

/// Whisper 在低音量 / 沉默音段常吐 YouTube subtitles 殘渣。把這幾段直接挖掉。
fn strip_whisper_hallucinations(s: &str) -> String {
    let phrases = [
        "Thank you for watching.",
        "Thank you for watching!",
        "Thanks for watching.",
        "Thanks for watching!",
        "Please subscribe.",
        "Please like and subscribe.",
        "Don't forget to subscribe.",
        "感謝觀看",
        "感謝收看",
        "請訂閱",
        "請按讚訂閱",
        "請按讚並訂閱",
        "歡迎訂閱",
    ];
    let mut out = s.to_string();
    for p in phrases.iter() {
        out = out.replace(p, "");
    }
    out
}

/// LLM 偶爾會把整段答案用「」/ ""/ 『』 包起來(雖然 prompt 說不要)。如果
/// 整段被同一對 quote 包,把外層拆掉。內部的引號保留。
fn strip_wrapping_quotes(s: &str) -> String {
    let trimmed = s.trim();
    let pairs = [
        ('"', '"'),
        ('「', '」'),
        ('『', '』'),
        ('“', '”'),
        ('‘', '’'),
    ];
    for (open, close) in pairs.iter() {
        if let (Some(first), Some(last)) = (trimmed.chars().next(), trimmed.chars().last()) {
            if first == *open && last == *close && trimmed.chars().count() >= 2 {
                let inner: String = trimmed
                    .chars()
                    .skip(1)
                    .take(trimmed.chars().count() - 2)
                    .collect();
                return inner;
            }
        }
    }
    s.to_string()
}

/// 半形 punctuation 在 CJK 字旁邊時轉全形。英文 context 中的標點不動。
///
/// 規則:
/// - 看每個半形字元的**前一個非空白字元**跟**後一個非空白字元**
/// - 兩邊任一是 CJK(\u{4E00}-\u{9FFF} / \u{3400}-\u{4DBF})就轉全形
/// - 兩邊都不是 CJK(全英文 context)→ 保持半形
fn halfwidth_to_fullwidth_near_cjk(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let full = match c {
            ',' => Some('\u{FF0C}'), // ,
            '.' => Some('\u{3002}'), // 。 (中文用句號不是全形句點)
            '!' => Some('\u{FF01}'), // !
            '?' => Some('\u{FF1F}'), // ?
            ':' => Some('\u{FF1A}'), // :
            ';' => Some('\u{FF1B}'), // ;
            _ => None,
        };
        if let Some(full_char) = full {
            // 看左右非空白鄰居是不是 CJK
            let near_cjk_left = (0..i).rev().find_map(|j| {
                let ch = chars[j];
                if ch.is_whitespace() {
                    None
                } else {
                    Some(is_cjk(ch))
                }
            });
            let near_cjk_right = (i + 1..chars.len()).find_map(|j| {
                let ch = chars[j];
                if ch.is_whitespace() {
                    None
                } else {
                    Some(is_cjk(ch))
                }
            });
            // 任一邊是 CJK 就轉
            if near_cjk_left == Some(true) || near_cjk_right == Some(true) {
                out.push(full_char);
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn is_cjk(c: char) -> bool {
    // CJK Unified Ideographs + Extension A 涵蓋繁中常用 99%+
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}

/// trim 整段、collapse 多重空白為單個、刪空行
fn normalize_whitespace(s: &str) -> String {
    let lines: Vec<String> = s
        .lines()
        .map(|line| {
            // 每行內把 run 的 ASCII 空白 / tab 壓成單個空格
            let mut prev_space = false;
            let mut buf = String::with_capacity(line.len());
            for c in line.chars() {
                let is_ws_compress = c == ' ' || c == '\t';
                if is_ws_compress {
                    if !prev_space {
                        buf.push(' ');
                    }
                    prev_space = true;
                } else {
                    buf.push(c);
                    prev_space = false;
                }
            }
            buf.trim().to_string()
        })
        .filter(|line| !line.is_empty())
        .collect();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_default_is_smart() {
        assert_eq!(CleanupLevel::default(), CleanupLevel::Smart);
    }

    #[test]
    fn strips_thank_you_for_watching() {
        let input = "今天天氣很好。Thank you for watching!";
        let out = programmatic_cleanup(input);
        assert_eq!(out, "今天天氣很好。");
    }

    #[test]
    fn strips_subscribe_chinese() {
        let input = "我想說的就是這樣。請按讚訂閱。";
        let out = programmatic_cleanup(input);
        // "請按讚訂閱" 拿掉,後面剩的句號跟空格 normalize 掉
        assert!(!out.contains("訂閱"));
        assert!(out.contains("我想說的就是這樣"));
    }

    #[test]
    fn unwraps_chinese_quotes() {
        let out = programmatic_cleanup("「今天天氣很好」");
        assert_eq!(out, "今天天氣很好");
    }

    #[test]
    fn keeps_inner_quotes() {
        // 整段不是被同一對 quote 包,內部 quote 保留
        let out = programmatic_cleanup("他說「你好」然後走了");
        assert!(out.contains("「你好」"));
    }

    #[test]
    fn halfwidth_punct_near_cjk_to_fullwidth() {
        let out = programmatic_cleanup("今天天氣很好,我覺得心情也變好了.");
        // U+FF0C fullwidth comma, U+3002 ideographic full stop
        assert!(
            out.contains("好\u{FF0C}我"),
            "expected fullwidth comma between 好 and 我, got: {out:?}"
        );
        assert!(
            out.contains("了\u{3002}"),
            "expected ideographic period after 了, got: {out:?}"
        );
        assert!(!out.contains(","), "raw ascii comma should be gone: {out:?}");
        assert!(!out.contains("."), "raw ascii period should be gone: {out:?}");
    }

    #[test]
    fn halfwidth_punct_in_english_kept() {
        let out = programmatic_cleanup("Hello, world.");
        // 兩邊都是 ASCII letter,不轉
        assert_eq!(out, "Hello, world.");
    }

    #[test]
    fn mixed_chinese_english_punct() {
        // 「OK,」這種 — , 左是 K(non-CJK)、右是 CJK,任一是 CJK 就轉
        let out = programmatic_cleanup("我說 OK,然後走了");
        assert!(
            out.contains("OK\u{FF0C}然後"),
            "expected fullwidth comma after OK (because right neighbor is CJK), got: {out:?}"
        );
    }

    #[test]
    fn collapses_multiple_spaces_keeps_lines() {
        let out = programmatic_cleanup("第一行\n\n   第二行   有空白\n");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, vec!["第一行", "第二行 有空白"]);
    }

    #[test]
    fn empty_input_stays_empty() {
        assert_eq!(programmatic_cleanup(""), "");
        assert_eq!(programmatic_cleanup("   \n  \n  "), "");
    }
}
