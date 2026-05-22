//! `.xlsx` reader — 走 [`calamine::open_workbook`] + `Xlsx<_>` 多 sheet 串接。
//!
//! # 設計筆記
//!
//! - **「萬卷之口」面向 LLM**:LLM 不會 render Excel grid,輸出要直接可讀。所以
//!   我們把整本 workbook flatten 成單一 `String`,sheet 間有清楚的標題分隔,
//!   row 內 cell 用 TSV 慣例的 `\t` 分。
//! - **Sheet 分隔策略**:每個 sheet 開頭加 `## Sheet: <name>` 標題行(markdown
//!   `##`),sheet 間用空白行(`\n\n`)分。LLM 容易透過這個 marker 認出 sheet
//!   邊界,使用者要 quote 也方便。
//! - **Row / cell 分隔**:row 內 cell 用 `\t`、row 間用 `\n`,對齊 TSV 慣例。
//!   完全空白的 row(每個 cell 都是 `Empty` / `Error`)整列 skip,避免大量空白
//!   雜訊汙染 LLM context。
//! - **Cell 型別格式化**(對齊 `calamine::Data` 各 variant):
//!   - `String(s)` → `s`(原樣)
//!   - `Float(f)` → `format!("{}", f)`(Rust 的 `Display` 已 trim trailing 0)
//!   - `Int(i)` → `format!("{}", i)`
//!   - `Bool(b)` → `format!("{}", b)`(`true` / `false`)
//!   - `DateTime(dt)` → `dt.as_datetime()` → `%Y-%m-%dT%H:%M:%S`(ISO 8601 second
//!     精度;對 LLM consumption 比 `{:?}` Debug 友善太多。`as_datetime()` 偶爾
//!     回 None(極端值或 1904/1900 epoch quirk),fallback 回 Debug 形式)
//!   - `DateTimeIso(s)` / `DurationIso(s)` → 直接用字串(已是 ISO)
//!   - `Empty` → 空字串(該 cell 不輸出內容,但 row 仍按位置補 `\t`)
//!   - `Error(_)` → `"[#ERROR]"` placeholder(debug 用 — 之前直接吃成空字串,
//!     `#REF!` / `#DIV/0!` 之類錯誤 cell 整個沒了 trace,排查很痛)
//! - **格式範圍**:目前 **只支援 `.xlsx`**。calamine 也能讀 `.xls` / `.xlsb` /
//!   `.ods`,但本 stream 只動 xlsx,其他副檔名不掛 dispatch arm。
//! - **錯誤 wrap**:`calamine::XlsxError`(以及更上層的 `calamine::Error`)涵蓋
//!   zip / xml / 結構不合 spec 等多種失敗。一律 wrap 進
//!   [`FileLoaderError::XlsxExtraction`],internal 細節收進 `String` 而不揭露
//!   `calamine::Error` type,避免下游 crate 因 calamine bump major 版被連帶 break
//!   (對齊 PDF / DOCX reader 的 wrap 策略)。
//! - **caller 預期**:呼叫前 [`crate::read_file_text`] 已經保證檔案存在,所以這層
//!   不再 `try_exists`,直接交給 calamine 讀。

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use calamine::{open_workbook, Data, Reader, Xlsx, XlsxError};

use crate::{FileFormatReader, FileLoaderError};

pub(crate) struct XlsxReader;

impl FileFormatReader for XlsxReader {
    fn read(&self, path: &Path) -> Result<String, FileLoaderError> {
        // 顯式標 `Xlsx<BufReader<File>>` — `open_workbook` 的 `R::Error` 要靠
        // `R` 完全解析才推得出來,只寫 `Xlsx<_>` Rust 在 `.map_err` 那裡會卡推導。
        let mut workbook: Xlsx<BufReader<File>> = open_workbook(path)
            .map_err(|e: XlsxError| FileLoaderError::XlsxExtraction(e.to_string()))?;

        // `sheet_names()` 回傳 workbook 自帶順序,跟 Excel 顯示順序一致。
        let sheet_names = workbook.sheet_names();

        let mut sections: Vec<String> = Vec::with_capacity(sheet_names.len());
        for name in sheet_names {
            let range = workbook
                .worksheet_range(&name)
                .map_err(|e: XlsxError| FileLoaderError::XlsxExtraction(e.to_string()))?;

            let mut section = String::new();
            section.push_str("## Sheet: ");
            section.push_str(&name);

            for row in range.rows() {
                let cells: Vec<String> = row.iter().map(format_cell).collect();
                // 空 row(每個 cell 都 format 成空字串)整列 skip。
                if cells.iter().all(|c| c.is_empty()) {
                    continue;
                }
                section.push('\n');
                section.push_str(&cells.join("\t"));
            }

            sections.push(section);
        }

        Ok(sections.join("\n\n"))
    }
}

/// 把單一 cell 的 [`calamine::Data`] 格式化成 LLM 友善的字串。
///
/// `Empty` 輸出空字串(該 cell 不揭露內容,但 row 仍會用 `\t` 保留位置)。
/// `Error` 輸出 `[#ERROR]` placeholder 方便 debug — 之前直接吃成空字串,
/// `#REF!` / `#DIV/0!` 之類錯誤 cell 整個沒了 trace。
fn format_cell(data: &Data) -> String {
    match data {
        Data::String(s) => s.clone(),
        Data::Float(f) => format!("{}", f),
        Data::Int(i) => format!("{}", i),
        Data::Bool(b) => format!("{}", b),
        Data::DateTime(dt) => dt
            .as_datetime()
            // chrono 的 `NaiveDateTime` 預設 Display 就是 `YYYY-MM-DD HH:MM:SS`,
            // 我們改用 `%Y-%m-%dT%H:%M:%S` 拿 ISO 8601(date 跟 time 中間用 `T`)。
            .map(|ndt| ndt.format("%Y-%m-%dT%H:%M:%S").to_string())
            // as_datetime() 偶爾回 None(極端 epoch 值);fallback 不揭露 None,
            // 而是退回原本 Debug 形式,讓 caller 至少看得到「這 cell 是 DateTime」。
            .unwrap_or_else(|| format!("{:?}", dt)),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Empty => String::new(),
        Data::Error(_) => "[#ERROR]".to_string(),
    }
}
