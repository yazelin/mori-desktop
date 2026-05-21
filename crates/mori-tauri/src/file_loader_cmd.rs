//! Tauri command bridging [`mori_file_loader::read_file_text`] 到 IPC / LLM tool。
//!
//! 「萬卷之口」整合層:`mori-file-loader` 是純 lib，這層把它包成
//! `#[tauri::command]`，LLM 透過 system prompt 內的工具描述（見
//! `build_system_prompt`）就能叫到它。
//!
//! 失敗一律收成 `String` — Tauri IPC 需要 `Serialize` error，且 LLM 端拿到
//! 文字訊息比 typed enum 更可讀。內部完整型別在 [`mori_file_loader::FileLoaderError`]，
//! 走 `Display`(`#[error(..)]`)轉字串。

use std::path::PathBuf;

use mori_file_loader::read_file_text;

/// LLM-visible name = `read_file_text`。Rust 函式加 `_cmd` 對齊 `transcribe_*_cmd`
/// 既有風格（區分「Tauri 入口」vs「底層 lib 函式」）。
#[tauri::command]
pub fn read_file_text_cmd(path: String) -> Result<String, String> {
    let p = PathBuf::from(path);
    read_file_text(&p).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    //! Tauri runtime mock 麻煩，這層直接 unit-test pure function `read_file_text_cmd`
    //! 本身（`#[tauri::command]` 只是註冊 macro，不影響直接 call）。
    //!
    //! PDF / DOCX / XLSX 真檔案 E2E 等對應 reader merge 後另開 follow-up，
    //! 這裡只覆蓋 baseline `.txt` 成功 + 兩條 error path。

    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn reads_txt_via_cmd() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("test.txt");
        fs::write(&p, "Hello, Mori").unwrap();
        let result = read_file_text_cmd(p.to_string_lossy().to_string());
        assert_eq!(result.unwrap(), "Hello, Mori");
    }

    #[test]
    fn returns_error_for_missing_file() {
        let result = read_file_text_cmd("/nonexistent/path.txt".to_string());
        assert!(result.is_err(), "expected error for missing file");
    }

    #[test]
    fn returns_error_for_unsupported_extension() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("test.zzz");
        fs::write(&p, "anything").unwrap();
        let result = read_file_text_cmd(p.to_string_lossy().to_string());
        assert!(result.is_err(), "expected error for unsupported extension");
    }
}
