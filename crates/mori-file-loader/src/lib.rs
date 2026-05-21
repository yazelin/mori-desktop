//! `mori-file-loader` — 統一文件讀取(「萬卷之口」)。
//!
//! 給 Mori 一個對外穩定的入口讀使用者塞過來的文件;副檔名 dispatch 給對應實作。
//!
//! 本 crate 是 **skeleton**:現階段(E-base)只支援純文字格式 — `.txt` / `.md`,
//! 兩者都走 [`std::fs::read_to_string`]。後續 Wave 2 才會加 `.pdf` / `.docx` /
//! `.xlsx` 等 binary 格式,屆時把對應的 `FileFormatReader` 加進 [`dispatch`] 即可,
//! 公開 API([`read_file_text`])保持不變。
//!
//! # 公開 API
//!
//! 對外只有兩個型別:
//! - [`read_file_text`] — 入口函式,吃 [`std::path::Path`],回 `String`
//! - [`FileLoaderError`] — error enum
//!
//! `FileFormatReader` trait 跟內部 reader struct **目前不公開** — 等 Wave 2
//! 真的需要外部 crate 註冊 format 再開放,避免 over-design。
//!
//! # 行為決策
//!
//! - 副檔名比對 **case-insensitive**:`SHOUT.TXT` / `notes.MD` 都會被正確 dispatch
//! - 副檔名一律以 lowercase 形式回給 [`FileLoaderError::UnsupportedExtension`]
//! - 無副檔名 → [`FileLoaderError::UnsupportedExtension`]`("")`
//! - 檔案不存在 → [`FileLoaderError::NotFound`]
//! - 非 UTF-8 內容 → [`FileLoaderError::InvalidUtf8`](不 panic,不做 lossy decode)
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//! use mori_file_loader::read_file_text;
//!
//! let text = read_file_text(Path::new("notes.md")).expect("read");
//! println!("{text}");
//! ```

use std::path::{Path, PathBuf};

/// `mori-file-loader` 的錯誤型別。
#[derive(Debug, thiserror::Error)]
pub enum FileLoaderError {
    /// 路徑指到的檔案不存在。
    #[error("file not found: {0}")]
    NotFound(PathBuf),

    /// 副檔名目前還沒有對應的 reader(eg `.pdf` 要等 Wave 2)。
    /// 內含的字串是 **lowercase** 副檔名;無副檔名時為空字串。
    #[error("unsupported extension: {0}")]
    UnsupportedExtension(String),

    /// 底層 IO 失敗(非 NotFound — 例如權限不足、磁碟錯誤)。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// 檔案內容不是合法 UTF-8。
    #[error("invalid utf-8 in file: {0}")]
    InvalidUtf8(PathBuf),
}

/// 內部 trait:每個支援的副檔名對應一個 reader。
///
/// **目前不公開** — Wave 2 加新 format 時再決定要不要對外開放(plugin registry)。
/// 現階段 [`dispatch`] 用 hardcoded match,結構單純,易刪易加。
trait FileFormatReader {
    fn read(&self, path: &Path) -> Result<String, FileLoaderError>;
}

/// `.txt` / `.md` 共用:純 UTF-8 文字檔。
struct PlainTextReader;

impl FileFormatReader for PlainTextReader {
    fn read(&self, path: &Path) -> Result<String, FileLoaderError> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                // `read_to_string` 在內容非 UTF-8 時回 InvalidData。
                Err(FileLoaderError::InvalidUtf8(path.to_path_buf()))
            }
            Err(e) => Err(FileLoaderError::Io(e)),
        }
    }
}

/// 把 lowercase 副檔名 dispatch 到對應 reader。
///
/// Wave 2 加新 format 在這邊加 arm 即可。
fn dispatch(ext_lower: &str) -> Option<Box<dyn FileFormatReader>> {
    match ext_lower {
        "txt" | "md" => Some(Box::new(PlainTextReader)),
        _ => None,
    }
}

/// 讀檔回文字內容。對外的主要入口。
///
/// 依副檔名(case-insensitive)dispatch 給對應 reader。現階段只支援 `.txt` / `.md`。
///
/// # Errors
///
/// 看 [`FileLoaderError`] 各 variant 的 doc。
pub fn read_file_text(path: &Path) -> Result<String, FileLoaderError> {
    // 1. file exists?
    //    用 `try_exists` 避免 broken symlink false-negative,但 IO error
    //    要原樣往上拋(權限不足 etc)。
    match path.try_exists() {
        Ok(true) => {}
        Ok(false) => return Err(FileLoaderError::NotFound(path.to_path_buf())),
        Err(e) => return Err(FileLoaderError::Io(e)),
    }

    // 2. extension(lowercase,無副檔名 → "")
    let ext_lower = path
        .extension()
        .and_then(|os| os.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    // 3. dispatch
    match dispatch(&ext_lower) {
        Some(reader) => reader.read(path),
        None => Err(FileLoaderError::UnsupportedExtension(ext_lower)),
    }
}

#[cfg(test)]
mod tests {
    //! 內部 unit tests — 覆蓋公開 integration tests 難測的邊界(eg non-UTF-8 bytes、
    //! 無副檔名)。public API 行為的主測試在 `tests/integration.rs`。

    use super::*;
    use std::io::Write;

    #[test]
    fn invalid_utf8_in_txt_returns_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("garbled.txt");
        // 0xFF 0xFE 0xFD 不是合法 UTF-8 start byte。
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[0xFF, 0xFE, 0xFD]).unwrap();

        let err = read_file_text(&path).expect_err("expect InvalidUtf8");
        match err {
            FileLoaderError::InvalidUtf8(p) => assert_eq!(p, path),
            other => panic!("expected InvalidUtf8, got {other:?}"),
        }
    }

    #[test]
    fn no_extension_returns_unsupported_with_empty_string() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("NO_EXT");
        std::fs::write(&path, b"whatever").unwrap();

        let err = read_file_text(&path).expect_err("expect UnsupportedExtension");
        match err {
            FileLoaderError::UnsupportedExtension(ext) => assert_eq!(ext, ""),
            other => panic!("expected UnsupportedExtension, got {other:?}"),
        }
    }
}
