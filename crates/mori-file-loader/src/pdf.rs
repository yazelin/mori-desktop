//! `.pdf` reader — 走 [`pdf_extract::extract_text`]。
//!
//! # 設計筆記
//!
//! - **多頁串接**:`pdf_extract::extract_text` 內部已經把所有 page 的文字 join 成
//!   一個 `String`,page 間以 `\n` 分(實測會在開頭加 leading `\n\n`)。我們不再
//!   做額外 normalize — pdf-extract 的 output 直接回給 caller,保留原貌。
//! - **錯誤 wrap**:`pdf_extract::Error` 涵蓋 parse / 加密 / 不合 spec 等多種失敗,
//!   不是 IO error。一律 wrap 進 [`FileLoaderError::PdfExtraction`],internal 細節
//!   收進 `String` 而不揭露 `pdf_extract::Error` type,避免下游 crate 因 pdf-extract
//!   bump major 版被連帶 break。
//! - **caller 預期**:呼叫前 [`crate::read_file_text`] 已經保證檔案存在,所以這層
//!   不再 `try_exists`,直接交給 pdf-extract 讀。

use std::path::Path;

use crate::{FileFormatReader, FileLoaderError};

pub(crate) struct PdfReader;

impl FileFormatReader for PdfReader {
    fn read(&self, path: &Path) -> Result<String, FileLoaderError> {
        pdf_extract::extract_text(path).map_err(|e| FileLoaderError::PdfExtraction(e.to_string()))
    }
}
