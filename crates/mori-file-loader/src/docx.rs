//! `.docx` reader — 走 [`docx_rs::read_docx`] + 手動 traverse document tree。
//!
//! # 設計筆記
//!
//! - **API 形狀**:`docx_rs::read_docx` 吃 `&[u8]`,所以這層先 [`std::fs::read`]
//!   把整份檔讀進 memory,再交給 docx-rs parse。對個人 vault 等級的文件(MB 量級)
//!   一次性 load 沒問題;未來若要支援巨型檔再走 streaming。
//! - **多段文字 join**:`.docx` 主文以 paragraph 為單位,paragraph 內可能含多個
//!   run(同段不同 formatting / 顏色 / 字體)。我們抽 text 的策略:
//!   - **段落間**:用 `"\n\n"` 分(對應 Markdown / plain text 慣例,給 LLM 看時
//!     段落邊界清楚)
//!   - **段落內 runs**:用空字串連(run 自帶的文字本身已含必要的 spacing,
//!     重複插空白會破壞像 "hello world" 這種跨 run 寫法)
//!   - **Hyperlink**:`ParagraphChild::Hyperlink` 內部結構跟段落一樣含 Runs,
//!     我們 recurse 進去取 anchor 文字(不抽 URL — LLM 通常要的是 link text
//!     本身,URL 噪音多)
//!   - **Run 內 `Break` / `Tab`**:`Shift+Enter`(同段換行) 跟 tab character
//!     都常被當段內語意 marker,silent skip 會丟資訊。Break → `\n`,Tab → `\t`。
//!   - **表格 / 圖 / 註腳 / header / footer**:**先 skip**,只抓主文
//!     [`docx_rs::DocumentChild::Paragraph`];之後 stream 再加(YAGNI:先讓
//!     baseline 走通,真實 vault 文件 99% 是純段落)
//! - **錯誤 wrap**:`docx_rs::ReaderError` 涵蓋 zip / xml / 結構不合 spec 等
//!   多種失敗。一律 wrap 進 [`FileLoaderError::DocxExtraction`],internal 細節
//!   收進 `String` 而不揭露 `docx_rs::ReaderError` type,避免下游 crate 因 docx-rs
//!   bump major 版被連帶 break(對齊 PDF reader 的 wrap 策略)。
//! - **caller 預期**:呼叫前 [`crate::read_file_text`] 已經保證檔案存在,所以這層
//!   不再 `try_exists`,直接 [`std::fs::read`];IO error(權限等)走
//!   [`FileLoaderError::Io`](透過 `?` + `From<std::io::Error>`)。

use std::path::Path;

use docx_rs::{DocumentChild, ParagraphChild, Run, RunChild};

use crate::{FileFormatReader, FileLoaderError};

pub(crate) struct DocxReader;

impl FileFormatReader for DocxReader {
    fn read(&self, path: &Path) -> Result<String, FileLoaderError> {
        let bytes = std::fs::read(path)?;
        let docx = docx_rs::read_docx(&bytes)
            .map_err(|e| FileLoaderError::DocxExtraction(e.to_string()))?;

        let mut paragraphs: Vec<String> = Vec::new();
        for child in &docx.document.children {
            // 只抓主文 paragraph;table / 其他 child 暫時略過(見 module 註解)。
            if let DocumentChild::Paragraph(p) = child {
                let mut buf = String::new();
                for pc in &p.children {
                    append_paragraph_child(&mut buf, pc);
                }
                paragraphs.push(buf);
            }
        }

        Ok(paragraphs.join("\n\n"))
    }
}

/// 把單一 [`ParagraphChild`] 的可讀文字 append 到 buffer。
///
/// 處理 `Run` 跟 `Hyperlink`(後者結構含 inner Runs)— 其他 variant
/// (`BookmarkStart` / `CommentStart` / `PageNum` 等)silent skip,跟原本行為一致。
fn append_paragraph_child(buf: &mut String, pc: &ParagraphChild) {
    match pc {
        ParagraphChild::Run(run) => append_run(buf, run),
        ParagraphChild::Hyperlink(h) => {
            // Hyperlink.children 是 Vec<ParagraphChild>,內部 99% 是 Run,
            // 但 docx-rs 也允許 nested structure(eg insert/delete),所以 recurse。
            for child in &h.children {
                append_paragraph_child(buf, child);
            }
        }
        _ => {}
    }
}

/// 把一個 [`Run`] 的可讀文字 append 到 buffer。
///
/// `RunChild`:
/// - `Text(t)` → 原文
/// - `Break(_)` → `'\n'`(同段 Shift+Enter 換行)
/// - `Tab(_)` → `'\t'`(Word 顯示為 tab character)
/// - 其他(`Drawing` / `Sym` / `FieldChar` 等)silent skip
fn append_run(buf: &mut String, run: &Run) {
    for rc in &run.children {
        match rc {
            RunChild::Text(t) => buf.push_str(&t.text),
            RunChild::Break(_) => buf.push('\n'),
            RunChild::Tab(_) => buf.push('\t'),
            _ => {}
        }
    }
}
