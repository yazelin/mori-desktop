//! 統一文件讀取 stub — 還沒實作。
//!
//! 真正 API spec 見 tests/integration.rs。

use std::path::{Path, PathBuf};

/// File loader 錯誤型別。
#[derive(Debug, thiserror::Error)]
pub enum FileLoaderError {
    #[error("file not found: {0}")]
    NotFound(PathBuf),
    #[error("unsupported extension: {0}")]
    UnsupportedExtension(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid utf-8 in file: {0}")]
    InvalidUtf8(PathBuf),
}

/// Stub — 永遠回 unsupported 讓 tests 確實 fail。
pub fn read_file_text(_path: &Path) -> Result<String, FileLoaderError> {
    Err(FileLoaderError::UnsupportedExtension(String::from("stub")))
}
