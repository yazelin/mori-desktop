//! `mori-file-loader` integration tests。
//!
//! 跑 `read_file_text(path)` 的公開行為:`.txt` / `.md` baseline、未支援副檔名、
//! missing file、UTF-8 邊界、case-insensitive 副檔名。
//!
//! 之後 Wave 2 各 format crate 加 impl 時,這層 baseline 不該被打破。

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
fn read_file_text_returns_unsupported_for_pdf() {
    let dir = TempDir::new().unwrap();
    // 寫一個假 .pdf — 副檔名才是重點,內容無所謂
    let path = write_file(&dir, "doc.pdf", b"%PDF-1.4 fake");

    let err = read_file_text(&path).expect_err("expect UnsupportedExtension");
    match err {
        FileLoaderError::UnsupportedExtension(ext) => {
            assert_eq!(ext, "pdf", "should report lowercase extension");
        }
        other => panic!("expected UnsupportedExtension, got {other:?}"),
    }
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
