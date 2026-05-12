//! FetchUrlSkill (Phase 3B) — 抓網址回文字。
//!
//! 設計:
//! - 純 HTTP GET（reqwest）,no JS rendering(SPA 的 client-render 內容抓不到 — 可接受)
//! - HTML 抽 main text:抓 `<title>` + 把 script/style/svg 過濾掉、剩下 tag 拆 plain text
//! - 8KB cap(避免回給 LLM 超大 context)
//! - User-Agent 帶 Mori 識別 + Accept-Language: zh-TW
//! - timeout 10s
//!
//! LLM workflow:
//! 1. Context 裡 urls_detected 看到 URL
//! 2. 使用者說「摘要這個」/「這在講什麼」 → 呼叫 fetch_url 拿內容
//! 3. 再用 summarize / translate / polish 等 skill 處理 fetched 文字

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::context::Context;
use super::{Skill, SkillOutput};

const MAX_BYTES: usize = 8192;
const TIMEOUT_SECS: u64 = 10;
const UA: &str = concat!("Mori/", env!("CARGO_PKG_VERSION"), " (yazelin/mori-desktop)");

pub struct FetchUrlSkill {
    client: reqwest::Client,
}

impl FetchUrlSkill {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for FetchUrlSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Skill for FetchUrlSkill {
    fn name(&self) -> &'static str {
        "fetch_url"
    }

    fn description(&self) -> &'static str {
        "Fetch a URL and return the page's title + main text content. \
         Use when the user references a URL(剪貼簿 / 講話裡 / urls_detected 內)and \
         wants the content summarized / quoted / answered against. \
         Returns up to 8KB; SPA / JS-rendered content may be incomplete."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "完整 URL,含 http:// 或 https://" }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, _context: &Context) -> Result<SkillOutput> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing url"))?
            .trim()
            .to_string();

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(anyhow!("URL must start with http:// or https://"));
        }

        tracing::info!(url = %url, "fetch_url");
        let resp = self
            .client
            .get(&url)
            .header("Accept-Language", "zh-TW,zh;q=0.9,en;q=0.8")
            .send()
            .await
            .context("fetch_url: HTTP GET")?;

        let status = resp.status();
        let final_url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let html = resp
            .text()
            .await
            .context("fetch_url: read body")?;

        if !status.is_success() {
            return Err(anyhow!(
                "fetch_url: HTTP {} from {} (got {} bytes)",
                status,
                final_url,
                html.len()
            ));
        }

        let extracted = if content_type.contains("html") || html.trim_start().starts_with('<') {
            extract_html_text(&html)
        } else {
            // 非 HTML(plain text / json / etc.)直接 trim 後回
            html.trim().to_string()
        };

        let truncated = truncate_bytes(&extracted, MAX_BYTES);
        let was_truncated = truncated.len() < extracted.len();

        let user_message = format!(
            "從 {final_url} 抓到 {} 字{}。",
            truncated.chars().count(),
            if was_truncated { "(已截斷)" } else { "" }
        );

        Ok(SkillOutput {
            user_message: format!("{user_message}\n\n{truncated}"),
            data: Some(serde_json::json!({
                "url": url,
                "final_url": final_url,
                "status": status.as_u16(),
                "content_type": content_type,
                "truncated": was_truncated,
                "chars": truncated.chars().count(),
                "text": truncated,
            })),
        })
    }
}

/// 把 HTML 拆成純文字:抽 `<title>`,移除 `<script>` / `<style>` / `<svg>` 等噪音,
/// 剩下 tag 拆掉只留文字節點,壓掉連續空白。
fn extract_html_text(html: &str) -> String {
    let mut out = String::new();

    // 1. 抽 title
    if let Some(title) = extract_tag_text(html, "title") {
        let t = collapse_whitespace(&title);
        if !t.is_empty() {
            out.push_str("# ");
            out.push_str(&t);
            out.push_str("\n\n");
        }
    }

    // 2. 把 noise tag 整段(含內容)拿掉
    let mut cleaned = html.to_string();
    for tag in &["script", "style", "svg", "noscript", "iframe", "head"] {
        cleaned = strip_tag_block(&cleaned, tag);
    }

    // 3. 剩下的 tag 全部換成空白,留下文字
    let plain = strip_all_tags(&cleaned);

    // 4. HTML entity decode(最簡:&amp; &lt; &gt; &quot; &nbsp; &#39;)
    let decoded = decode_entities(&plain);

    // 5. 壓多空白 + 多行
    out.push_str(&collapse_whitespace_lines(&decoded));
    out.trim().to_string()
}

/// 抓 `<tag>...</tag>` 中間的文字(case insensitive,只抓第一個)
fn extract_tag_text(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open_pat = format!("<{tag}");
    let close_pat = format!("</{tag}>");
    let open_idx = lower.find(&open_pat)?;
    // 找到 > 之後才是內容開始
    let after_open = &lower[open_idx..];
    let gt = after_open.find('>')?;
    let content_start = open_idx + gt + 1;
    let close_idx = lower[content_start..].find(&close_pat)?;
    Some(html[content_start..content_start + close_idx].to_string())
}

/// 把 `<tag ...>...</tag>` 整段(含 tag 跟內容)拿掉。重複處理直到沒有。
fn strip_tag_block(html: &str, tag: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut remaining = html;
    let lower_open = format!("<{tag}");
    let lower_close = format!("</{tag}>");
    loop {
        let lower = remaining.to_ascii_lowercase();
        let open = match lower.find(&lower_open) {
            Some(i) => i,
            None => {
                out.push_str(remaining);
                return out;
            }
        };
        let close_lower = match lower[open..].find(&lower_close) {
            Some(i) => i + lower_close.len(),
            None => {
                // 沒收尾 — 跳過該 open tag 之後的剩下也一起丟
                out.push_str(&remaining[..open]);
                return out;
            }
        };
        out.push_str(&remaining[..open]);
        remaining = &remaining[open + close_lower..];
    }
}

/// 把所有 `<...>` 替換成空白。
fn strip_all_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut depth = 0;
    for ch in html.chars() {
        if ch == '<' {
            depth += 1;
            out.push(' ');
        } else if ch == '>' {
            if depth > 0 {
                depth -= 1;
            }
        } else if depth == 0 {
            out.push(ch);
        }
    }
    out
}

/// 解 HTML entity(最簡版,只接 6 個常見的)
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

/// 壓掉連續空白(包含 tab),保留單一空白
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for ch in s.chars() {
        if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

/// 壓多空白,但保留段落分行:連續 2+ 換行壓成 1 個 \n\n
fn collapse_whitespace_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newline_count = 0;
    let mut space_pending = false;

    for ch in s.chars() {
        if ch == '\n' || ch == '\r' {
            newline_count += 1;
            space_pending = false;
        } else if ch == ' ' || ch == '\t' {
            if newline_count == 0 {
                space_pending = true;
            }
        } else {
            if newline_count >= 2 {
                out.push_str("\n\n");
            } else if newline_count == 1 {
                out.push('\n');
            } else if space_pending {
                out.push(' ');
            }
            newline_count = 0;
            space_pending = false;
            out.push(ch);
        }
    }
    out
}

/// 依字元邊界截到 max_bytes(UTF-8 safe)。
fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title() {
        let html = "<html><head><title>Hello World</title></head><body>x</body></html>";
        let r = extract_html_text(html);
        assert!(r.contains("Hello World"));
    }

    #[test]
    fn strips_script_block() {
        let html = "<p>before</p><script>evil()</script><p>after</p>";
        let r = extract_html_text(html);
        assert!(!r.contains("evil"));
        assert!(r.contains("before"));
        assert!(r.contains("after"));
    }

    #[test]
    fn strips_style_block() {
        let html = "<style>body{display:none}</style><p>real content</p>";
        let r = extract_html_text(html);
        assert!(!r.contains("display"));
        assert!(r.contains("real content"));
    }

    #[test]
    fn decodes_entities() {
        let html = "<p>Tom &amp; Jerry &lt;3</p>";
        let r = extract_html_text(html);
        assert!(r.contains("Tom & Jerry <3"));
    }

    #[test]
    fn truncate_keeps_char_boundary() {
        let s = "中文字測試"; // 每字 3 bytes,共 15 bytes
        let r = truncate_bytes(s, 7);
        // 應該截到 2 個中文字(6 bytes)— 不會切到一半 utf-8
        assert_eq!(r.chars().count(), 2);
    }

    #[test]
    fn ignores_case_in_tags() {
        let html = "<HTML><HEAD><TITLE>UpperCase</TITLE></HEAD><BODY>body text</BODY></HTML>";
        let r = extract_html_text(html);
        assert!(r.contains("UpperCase"));
        assert!(r.contains("body text"));
    }

    #[test]
    fn handles_nested_tags() {
        let html = "<div><p>para <b>bold</b> end</p></div>";
        let r = extract_html_text(html);
        assert!(r.contains("para"));
        assert!(r.contains("bold"));
        assert!(r.contains("end"));
    }

    #[test]
    fn plain_text_passes_through() {
        // 非 HTML(plain text)應該不會被 extract_html_text 處理時破壞
        let s = "just plain text\nwith newlines";
        let r = extract_html_text(s);
        assert!(r.contains("just plain text"));
    }
}
