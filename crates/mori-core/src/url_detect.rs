//! Phase 3B: 偵測文字內的 URL,給 Context.urls_detected 用。
//!
//! 不用 regex crate,避免新增 dep — URL 結構固定，pure Rust 跑 scanner 即可:
//! 1. 找 `http://` 或 `https://` 開頭
//! 2. 收字元直到 whitespace / 中文 / 終止標點(逗號、句號等視情況)
//! 3. 去掉常見的結尾標點(`,` `.` `!` `?` `)` `]` `>` `"` `'`)

/// 從文字裡抽出所有 URL,依出現順序、去重。
pub fn extract_urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 找 http:// 或 https://(不分大小寫)
        let start_match = if bytes[i..].len() >= 7 && bytes[i..i + 7].eq_ignore_ascii_case(b"http://") {
            Some(7)
        } else if bytes[i..].len() >= 8 && bytes[i..i + 8].eq_ignore_ascii_case(b"https://") {
            Some(8)
        } else {
            None
        };

        if let Some(scheme_len) = start_match {
            let url_start = i;
            // 從 scheme 後開始收字元,直到「非 URL 字元」
            let mut j = i + scheme_len;
            while j < bytes.len() && is_url_char(bytes[j]) {
                j += 1;
            }
            if j > url_start + scheme_len {
                let url = &text[url_start..j];
                let trimmed = trim_trailing_punct(url);
                if trimmed.len() > url_start - i + scheme_len {
                    let owned = trimmed.to_string();
                    if seen.insert(owned.clone()) {
                        out.push(owned);
                    }
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// 哪些 byte 視為 URL 內合法字元。
/// RFC 3986 寬鬆版:ASCII alnum + `-._~:/?#[]@!$&'()*+,;=%`
/// 注意:`,` `;` `(` `)` `.` 都允許,因為合法 URL 可以包含;
/// 結尾的標點再由 trim_trailing_punct 拿掉。
fn is_url_char(b: u8) -> bool {
    matches!(b,
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' |
        b'-' | b'_' | b'.' | b'~' |
        b':' | b'/' | b'?' | b'#' | b'[' | b']' | b'@' |
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' |
        b'%'
    )
}

/// 拿掉結尾常見的句末標點 — 句號 / 逗號 / 問號 / 驚嘆號 / 右括號 / 引號等。
/// 但不要拿到只剩 scheme(`http://` 不留)。
fn trim_trailing_punct(url: &str) -> &str {
    let mut end = url.len();
    while end > 0 {
        let b = url.as_bytes()[end - 1];
        if matches!(b, b'.' | b',' | b';' | b'!' | b'?' | b')' | b']' | b'>' | b'"' | b'\'') {
            end -= 1;
        } else {
            break;
        }
    }
    &url[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_single_url() {
        let urls = extract_urls("看這個 https://example.com");
        assert_eq!(urls, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn extracts_multiple_urls() {
        let urls = extract_urls("compare http://a.org and https://b.org/x");
        assert_eq!(urls, vec!["http://a.org".to_string(), "https://b.org/x".to_string()]);
    }

    #[test]
    fn strips_trailing_punct() {
        let urls = extract_urls("link: https://example.com/foo, then more");
        assert_eq!(urls, vec!["https://example.com/foo".to_string()]);
    }

    #[test]
    fn strips_trailing_period() {
        let urls = extract_urls("Read https://example.com.");
        assert_eq!(urls, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn keeps_query_string() {
        let urls = extract_urls("https://example.com/search?q=mori&page=2");
        assert_eq!(urls, vec!["https://example.com/search?q=mori&page=2".to_string()]);
    }

    #[test]
    fn keeps_fragment() {
        let urls = extract_urls("https://example.com/page#section-2");
        assert_eq!(urls, vec!["https://example.com/page#section-2".to_string()]);
    }

    #[test]
    fn dedups() {
        let urls = extract_urls("see https://x.com\n https://x.com");
        assert_eq!(urls, vec!["https://x.com".to_string()]);
    }

    #[test]
    fn ignores_plain_text() {
        let urls = extract_urls("no url here 沒有網址");
        assert!(urls.is_empty());
    }

    #[test]
    fn stops_at_whitespace() {
        let urls = extract_urls("https://a.com https://b.com");
        assert_eq!(urls, vec!["https://a.com".to_string(), "https://b.com".to_string()]);
    }

    #[test]
    fn stops_at_chinese() {
        let urls = extract_urls("https://example.com真的有用");
        assert_eq!(urls, vec!["https://example.com".to_string()]);
    }

    #[test]
    fn handles_youtube() {
        let urls = extract_urls("youtube link: https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(urls, vec!["https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string()]);
    }

    #[test]
    fn handles_uppercase_scheme() {
        let urls = extract_urls("看 HTTPS://EXAMPLE.COM");
        assert_eq!(urls, vec!["HTTPS://EXAMPLE.COM".to_string()]);
    }

    #[test]
    fn ignores_bare_domain() {
        // 沒有 scheme 不算 — 避免抓到 example.com 之類
        let urls = extract_urls("contact example.com or sales@example.com");
        assert!(urls.is_empty());
    }
}
