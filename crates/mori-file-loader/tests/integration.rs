//! `mori-file-loader` integration tests。
//!
//! 跑 `read_file_text(path)` 的公開行為:`.txt` / `.md` baseline、`.pdf`、`.docx`、
//! 未支援副檔名、missing file、UTF-8 邊界、case-insensitive 副檔名、壞檔錯誤分類。
//!
//! 之後加新 format 時,這層 baseline 不該被打破。

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use mori_file_loader::{read_file_text, FileLoaderError};
use tempfile::TempDir;

/// Helper:寫一個 named file 到 tempdir,回傳路徑。
fn write_file(dir: &TempDir, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    let mut f = fs::File::create(&path).expect("create tempfile");
    f.write_all(content).expect("write tempfile");
    path
}

#[test]
fn read_file_text_reads_txt() {
    let dir = TempDir::new().unwrap();
    let path = write_file(&dir, "sample.txt", b"hello world");

    let got = read_file_text(&path).expect("read .txt");
    assert_eq!(got, "hello world");
}

#[test]
fn read_file_text_reads_md() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "sample.md",
        b"# Title\n\nA paragraph with **bold**.\n",
    );

    let got = read_file_text(&path).expect("read .md");
    assert_eq!(got, "# Title\n\nA paragraph with **bold**.\n");
}

#[test]
fn read_file_text_returns_not_found_for_missing() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("nope.txt");

    let err = read_file_text(&missing).expect_err("expect NotFound");
    match err {
        FileLoaderError::NotFound(p) => assert_eq!(p, missing),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn read_file_text_returns_unsupported_for_unknown_extension() {
    let dir = TempDir::new().unwrap();
    // 用一個確定不會被 support 的副檔名(避免哪天 docx / xlsx 加進來踩到)。
    let path = write_file(&dir, "garbage.zzz", b"whatever");

    let err = read_file_text(&path).expect_err("expect UnsupportedExtension");
    match err {
        FileLoaderError::UnsupportedExtension(ext) => {
            assert_eq!(ext, "zzz", "should report lowercase extension");
        }
        other => panic!("expected UnsupportedExtension, got {other:?}"),
    }
}

#[test]
fn read_file_text_reads_pdf() {
    // checked-in fixture(`tests/fixtures/sample.pdf`)— 含已知字串 "Hello, Mori"。
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample.pdf");

    let got = read_file_text(&path).expect("read sample.pdf");
    assert!(
        got.contains("Hello, Mori"),
        "extracted text should contain 'Hello, Mori', got: {got:?}",
    );
}

#[test]
fn read_file_text_returns_extraction_error_for_corrupted_pdf() {
    let dir = TempDir::new().unwrap();
    // 寫一份壞掉的「.pdf」— 副檔名讓它走 PDF reader,內容讓 pdf-extract 解析爆炸。
    let path = write_file(&dir, "broken.pdf", b"not a real pdf file");

    let err = read_file_text(&path).expect_err("expect PdfExtraction");
    match err {
        FileLoaderError::PdfExtraction(_) => {}
        other => panic!("expected PdfExtraction, got {other:?}"),
    }
}

#[test]
fn read_file_text_reads_docx() {
    // checked-in fixture(`tests/fixtures/sample.docx`)— 由 python-docx 生成,
    // 含兩段文字「Hello, Mori」+「This is a DOCX test」。
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample.docx");

    let got = read_file_text(&path).expect("read sample.docx");
    assert!(
        got.contains("Hello, Mori"),
        "extracted text should contain 'Hello, Mori', got: {got:?}",
    );
    assert!(
        got.contains("DOCX test"),
        "extracted text should contain 'DOCX test', got: {got:?}",
    );
}

#[test]
fn read_file_text_returns_extraction_error_for_corrupted_docx() {
    let dir = TempDir::new().unwrap();
    // 寫一份壞掉的「.docx」— 副檔名讓它走 DOCX reader,內容讓 docx-rs 解析爆炸
    //(docx 本質是 zip,raw bytes 不是合法 zip header)。
    let path = write_file(&dir, "broken.docx", b"not a real docx file");

    let err = read_file_text(&path).expect_err("expect DocxExtraction");
    match err {
        FileLoaderError::DocxExtraction(_) => {}
        other => panic!("expected DocxExtraction, got {other:?}"),
    }
}

#[test]
fn read_file_text_reads_xlsx() {
    // checked-in fixture(`tests/fixtures/sample.xlsx`)— 由 openpyxl 生成,
    // 兩個 sheet:
    //   - "Data":  header「Name | Score」+ row「Mori | 100」
    //   - "Notes": 單 cell「This is a XLSX test」
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample.xlsx");

    let got = read_file_text(&path).expect("read sample.xlsx");
    assert!(
        got.contains("Sheet: Data"),
        "extracted text should contain 'Sheet: Data' header, got: {got:?}",
    );
    assert!(
        got.contains("Sheet: Notes"),
        "extracted text should contain 'Sheet: Notes' header, got: {got:?}",
    );
    assert!(
        got.contains("Mori"),
        "extracted text should contain 'Mori', got: {got:?}",
    );
    assert!(
        got.contains("100"),
        "extracted text should contain '100', got: {got:?}",
    );
    assert!(
        got.contains("This is a XLSX test"),
        "extracted text should contain 'This is a XLSX test', got: {got:?}",
    );
}

#[test]
fn read_file_text_returns_extraction_error_for_corrupted_xlsx() {
    let dir = TempDir::new().unwrap();
    // 寫一份壞掉的「.xlsx」— 副檔名讓它走 XLSX reader,內容讓 calamine 解析爆炸
    //(xlsx 本質是 zip,raw bytes 不是合法 zip header)。
    let path = write_file(&dir, "broken.xlsx", b"not a real xlsx file");

    let err = read_file_text(&path).expect_err("expect XlsxExtraction");
    match err {
        FileLoaderError::XlsxExtraction(_) => {}
        other => panic!("expected XlsxExtraction, got {other:?}"),
    }
}

#[test]
fn read_file_text_reads_epub() {
    // checked-in fixture(`tests/fixtures/sample.epub`)— 由 ebooklib 生成,
    // 兩個 chapter:
    //   - Chapter 1:含 "Hello, Mori — the forest spirit." + entity decode +
    //     `<style>` / `<script>`(都要被 strip 不洩漏)
    //   - Chapter 2:含 "This is a EPUB test." + `<br/>` 換行
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample.epub");

    let got = read_file_text(&path).expect("read sample.epub");
    assert!(
        got.contains("Hello, Mori"),
        "extracted text should contain 'Hello, Mori', got: {got:?}",
    );
    assert!(
        got.contains("forest spirit"),
        "extracted text should contain 'forest spirit', got: {got:?}",
    );
    assert!(
        got.contains("This is a EPUB test"),
        "extracted text should contain 'This is a EPUB test', got: {got:?}",
    );
    // entity decode 應 work(`&amp;` → `&`、`&#65;` → `A`);
    // 原始 XHTML 是 "Second paragraph &amp; entities &#65;.",
    // strip 後預期 "Second paragraph & entities A."。
    assert!(
        got.contains("paragraph & entities"),
        "extracted text should decode `&amp;`, got: {got:?}",
    );
    assert!(
        got.contains("entities A"),
        "expected decoded `&#65;` → 'A', got: {got:?}",
    );
    // CSS / JS 內容必須被 strip
    assert!(
        !got.contains("color:red"),
        "css leaked into output: {got:?}",
    );
    assert!(
        !got.contains("var x = 1"),
        "js leaked into output: {got:?}",
    );
}

#[test]
fn read_file_text_returns_extraction_error_for_corrupted_epub() {
    let dir = TempDir::new().unwrap();
    // 寫一份壞掉的「.epub」— 副檔名讓它走 EPUB reader,內容讓 rbook 解析爆炸
    //(epub 本質是 zip,raw bytes 不是合法 zip header)。
    let path = write_file(&dir, "broken.epub", b"not a real epub file");

    let err = read_file_text(&path).expect_err("expect EpubExtraction");
    match err {
        FileLoaderError::EpubExtraction(_) => {}
        other => panic!("expected EpubExtraction, got {other:?}"),
    }
}

#[test]
fn docx_extracts_hyperlink_text() {
    // checked-in fixture(`tests/fixtures/sample-with-hyperlink-break-tab.docx`)
    //   - paragraph 1:hyperlink display text 「MoriForestLink」
    //   - paragraph 2:見 docx_handles_runchild_break_and_tab
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample-with-hyperlink-break-tab.docx");

    let got = read_file_text(&path).expect("read docx with hyperlink");
    assert!(
        got.contains("MoriForestLink"),
        "hyperlink display text should be extracted, got: {got:?}",
    );
}

#[test]
fn docx_handles_runchild_break_and_tab() {
    // 同 fixture 的 paragraph 2 包:
    //   Run.children = [Text("LeftCell"), Tab, Text("RightCell"), Break, Text("AfterBreak")]
    // 預期 extract 出含 "LeftCell\tRightCell\nAfterBreak"。
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample-with-hyperlink-break-tab.docx");

    let got = read_file_text(&path).expect("read docx with break/tab");
    assert!(
        got.contains("LeftCell\tRightCell"),
        "Tab should join cells with '\\t', got: {got:?}",
    );
    assert!(
        got.contains("RightCell\nAfterBreak"),
        "Break should insert '\\n' between text runs, got: {got:?}",
    );
}

#[test]
fn xlsx_datetime_formatted_as_iso() {
    // checked-in fixture(`tests/fixtures/sample-with-datetime-error.xlsx`)— 由
    // openpyxl 生成,B2 = datetime(2024-01-15 09:30:00)。
    // 預期 output 含 ISO 8601 `2024-01-15T09:30:00`,**不**含 Debug 形式
    // 「ExcelDateTime { ... }」。
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample-with-datetime-error.xlsx");

    let got = read_file_text(&path).expect("read xlsx with datetime");
    assert!(
        got.contains("2024-01-15T09:30:00"),
        "expected ISO 8601 datetime in output, got: {got:?}",
    );
    assert!(
        !got.contains("ExcelDateTime"),
        "should not leak Debug-formatted ExcelDateTime, got: {got:?}",
    );
}

#[test]
fn xlsx_error_cell_returns_placeholder() {
    // 同 fixture 的 B3 是 explicit error cell (`#DIV/0!`,data_type="e")。
    // 預期 output 含 `[#ERROR]` placeholder,而不是 silently empty。
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample-with-datetime-error.xlsx");

    let got = read_file_text(&path).expect("read xlsx with error cell");
    assert!(
        got.contains("[#ERROR]"),
        "expected [#ERROR] placeholder in output, got: {got:?}",
    );
}

#[test]
fn read_file_text_handles_unicode() {
    let dir = TempDir::new().unwrap();
    let content = "森林裡有一隻 Mori 🌲 — 年輪不會說謊。";
    let path = write_file(&dir, "unicode.txt", content.as_bytes());

    let got = read_file_text(&path).expect("read unicode .txt");
    assert_eq!(got, content);
}

#[test]
fn read_file_text_extension_case_insensitive() {
    let dir = TempDir::new().unwrap();
    let path = write_file(&dir, "SHOUT.TXT", b"loud and clear");

    let got = read_file_text(&path).expect("read .TXT");
    assert_eq!(got, "loud and clear");
}
