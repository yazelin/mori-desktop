//! `.epub` reader — 走 [`rbook::Epub`] + 自寫 XHTML tag stripper。
//!
//! # 設計筆記
//!
//! - **API 形狀**:rbook 提供 [`Epub::open`](rbook::Epub::open) + `Reader` 介面。
//!   Reader 用 spine 順序(canonical reading order)逐 chapter 給出 XHTML 字串。
//!   我們對每個 chapter 把 tags strip 掉留純文字,chapter 間用 `"\n\n"` 分,
//!   對齊 DOCX / XLSX reader 的 paragraph / sheet 邊界慣例。
//! - **HTML strip 策略**:不引入 `scraper` / `html2text` 之類重 dep — EPUB 的
//!   readable content 是 XHTML(規範要求 well-formed),用一個輕量 state machine
//!   就能 strip:
//!     1. 把 `<...>` 全部消掉(含 `<script>` / `<style>` 內容區段)
//!     2. 解碼最常見的 named entities(`&nbsp; &amp; &lt; &gt; &quot; &apos;`)
//!        跟 numeric entities(`&#NN;` / `&#xNN;`)
//!     3. block 級 tag(`<p>` / `<br>` / `<div>` / `<h1..6>` / `<li>`)的開合
//!        會 emit 一個 `\n`(以保留段落感),最後 collapse 連續 whitespace
//!   這對 LLM consumption 是最低必要 — 我們不做 list bullet / table layout
//!   render,專心給可讀文字。
//! - **圖片 / CSS / NCX / nav doc**:Reader 只走 spine readable items(XHTML),
//!   image / CSS 自然 skip,不用我們特別判 mime type。
//! - **錯誤 wrap**:rbook 的 `EbookError` 含 archive / format / reader / IO 多種
//!   failure。一律 wrap 進 [`FileLoaderError::EpubExtraction`],internal 細節
//!   收進 `String` 而不揭露 `rbook::ebook::errors::EbookError` type,避免下游
//!   crate 因 rbook bump major 版被連帶 break(對齊 PDF / DOCX / XLSX reader 的
//!   wrap 策略)。
//! - **caller 預期**:呼叫前 [`crate::read_file_text`] 已經保證檔案存在,所以這層
//!   不再 `try_exists`,直接交給 rbook 讀。
//!
//! # 為什麼不用 epub-rs(`epub` crate)
//!
//! `epub` crate 是 GPL-3.0,跟 mori-desktop 的 MIT license 不兼容(會強迫整個
//! workspace 改 GPL)。`rbook` 是 Apache-2.0,可以直接用。

use std::path::Path;

use crate::{FileFormatReader, FileLoaderError};

pub(crate) struct EpubReader;

impl FileFormatReader for EpubReader {
    fn read(&self, path: &Path) -> Result<String, FileLoaderError> {
        let epub = rbook::Epub::open(path)
            .map_err(|e| FileLoaderError::EpubExtraction(e.to_string()))?;

        let mut chapters: Vec<String> = Vec::new();
        let mut reader = epub.reader();
        while let Some(item) = reader.read_next() {
            let content = item.map_err(|e| FileLoaderError::EpubExtraction(e.to_string()))?;
            let text = strip_html(content.content());
            // 整章都空白 / 只含 nav marker → 不要污染輸出。
            if !text.trim().is_empty() {
                chapters.push(text);
            }
        }

        Ok(chapters.join("\n\n"))
    }
}

/// 把 XHTML / HTML 字串 strip 成可讀純文字。
///
/// 策略見 module-level 註解。實作刻意保持 small + dependency-free。
fn strip_html(input: &str) -> String {
    // 1) 先消掉 <script>…</script> / <style>…</style> 內容區段,
    //    避免 strip 完一堆 JS / CSS 流出來。
    let mut work = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(rest_len) = consume_block(bytes, i, b"script") {
            i += rest_len;
            continue;
        }
        if let Some(rest_len) = consume_block(bytes, i, b"style") {
            i += rest_len;
            continue;
        }
        work.push(bytes[i] as char);
        i += 1;
    }

    // 2) 對 work 跑 tag-stripping state machine。block 級 tag 開合
    //    插 `\n`,其他 tag 視為 inline(無額外 spacing)。
    let block_tags: &[&str] = &[
        "p", "br", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr", "section", "article",
        "blockquote", "hr",
    ];
    let mut out = String::with_capacity(work.len());
    let mut chars = work.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            // 讀整個 tag 到 `>`(忽略 quoted attribute 內的 `>` — XHTML well-formed,
            // attribute 用 `"..."` 包,內部不該有 raw `>`,真有也只是顯示問題,
            // 不會造成 unsafe 行為)。
            let mut tag = String::new();
            for tc in chars.by_ref() {
                if tc == '>' {
                    break;
                }
                tag.push(tc);
            }
            // 抽 tag name(去掉 leading `/` 跟 attribute)
            let trimmed = tag.trim_start_matches('/').trim();
            let name: String = trimmed
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if block_tags.iter().any(|t| *t == name) {
                out.push('\n');
            }
            continue;
        }
        if c == '&' {
            // entity:讀到下一個 `;`(最多 6 char)→ decode
            let mut ent = String::new();
            let mut consumed = 0;
            while let Some(&pc) = chars.peek() {
                if pc == ';' || consumed > 8 {
                    chars.next(); // consume `;` or bail
                    break;
                }
                ent.push(pc);
                chars.next();
                consumed += 1;
            }
            out.push_str(&decode_entity(&ent));
            continue;
        }
        out.push(c);
    }

    // 3) collapse 連續 whitespace / 多 newline。我們想保留 paragraph 級 `\n\n`,
    //    但 squash >2 newlines 跟同段內 weird whitespace。
    collapse_whitespace(&out)
}

/// 若 `bytes[start..]` 是 `<tag ...>...</tag>` 起頭(case-insensitive),
/// 回傳消掉整段(含 closing tag)後相對 `start` 的字節數;否則回 None。
fn consume_block(bytes: &[u8], start: usize, tag: &[u8]) -> Option<usize> {
    if start >= bytes.len() || bytes[start] != b'<' {
        return None;
    }
    let after_lt = start + 1;
    if after_lt + tag.len() > bytes.len() {
        return None;
    }
    // case-insensitive tag name match
    for (idx, &want) in tag.iter().enumerate() {
        let got = bytes[after_lt + idx].to_ascii_lowercase();
        if got != want {
            return None;
        }
    }
    let after_name = after_lt + tag.len();
    // 必須是 attribute boundary:`>` / space / `/`
    if after_name >= bytes.len() {
        return None;
    }
    let boundary = bytes[after_name];
    if !(boundary == b'>'
        || boundary == b' '
        || boundary == b'\t'
        || boundary == b'/'
        || boundary == b'\n'
        || boundary == b'\r')
    {
        return None;
    }
    // 找到 `>` 結束 opening tag
    let mut i = after_name;
    while i < bytes.len() && bytes[i] != b'>' {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    i += 1;
    // self-closing(`<script/>`)— rare but valid
    if i >= 2 && bytes[i - 2] == b'/' {
        return Some(i - start);
    }
    // 找 closing `</tag>` (case-insensitive)
    while i + tag.len() + 3 <= bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'/' {
            let mut ok = true;
            for (idx, &want) in tag.iter().enumerate() {
                if bytes[i + 2 + idx].to_ascii_lowercase() != want {
                    ok = false;
                    break;
                }
            }
            if ok {
                // skip closing tag content to `>`
                let mut j = i + 2 + tag.len();
                while j < bytes.len() && bytes[j] != b'>' {
                    j += 1;
                }
                if j < bytes.len() {
                    return Some(j + 1 - start);
                }
                return None;
            }
        }
        i += 1;
    }
    None
}

/// Decode `&name;` / `&#NN;` / `&#xNN;`(name 不含 leading `&` 跟 trailing `;`)。
fn decode_entity(name: &str) -> String {
    match name {
        "nbsp" => " ".to_string(),
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        s if s.starts_with("#x") || s.starts_with("#X") => {
            u32::from_str_radix(&s[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("&{name};"))
        }
        s if s.starts_with('#') => s[1..]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("&{name};")),
        // unknown named entity → 原樣放回,避免吃字
        _ => format!("&{name};"),
    }
}

/// Collapse 連續 space / tab(同行內)成單空格;連續 newline ≤ 2(保留段落)。
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_was_space = false;
    let mut newline_run = 0u32;
    for c in s.chars() {
        if c == '\n' {
            newline_run += 1;
            prev_was_space = false;
            if newline_run <= 2 {
                out.push('\n');
            }
            continue;
        }
        if c == ' ' || c == '\t' || c == '\r' {
            // strip leading whitespace on a fresh line
            if newline_run > 0 {
                continue;
            }
            if !prev_was_space {
                out.push(' ');
                prev_was_space = true;
            }
            continue;
        }
        newline_run = 0;
        prev_was_space = false;
        out.push(c);
    }
    // strip trailing whitespace / newline
    while out.ends_with([' ', '\n']) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_basic_paragraph() {
        let html = "<p>Hello, <em>Mori</em>!</p>";
        let got = strip_html(html);
        assert!(got.contains("Hello, Mori!"), "got: {got:?}");
    }

    #[test]
    fn strip_html_decodes_entities() {
        let html = "<p>A &amp; B &lt;3 &#65;</p>";
        let got = strip_html(html);
        assert!(got.contains("A & B <3 A"), "got: {got:?}");
    }

    #[test]
    fn strip_html_skips_script_and_style() {
        let html = "<style>p{color:red}</style><p>Visible</p><script>alert('x')</script>";
        let got = strip_html(html);
        assert!(got.contains("Visible"), "got: {got:?}");
        assert!(!got.contains("color"), "css leaked: {got:?}");
        assert!(!got.contains("alert"), "js leaked: {got:?}");
    }

    #[test]
    fn strip_html_inserts_paragraph_breaks() {
        let html = "<p>One</p><p>Two</p>";
        let got = strip_html(html);
        assert!(got.contains("One"), "got: {got:?}");
        assert!(got.contains("Two"), "got: {got:?}");
        assert!(got.contains('\n'), "expect newline between paragraphs: {got:?}");
    }
}
