//! Append corrections.md `## User` 段。atomic write(.tmp + rename)避免 voice
//! profile 同時 read 撞 partial write。
//!
//! 規則:
//! - `## User` heading 不存在 → 在檔尾建 + `### 用戶自加` subsection + 加新行
//! - `## User` heading 存在 → 找 `### 用戶自加`,沒有就建,然後同 suggested
//!   存在的 line merge wrong variants 進去,否則加新行
//! - Baseline 段(`## Baseline`)嚴格不動
//! - line format:`- 錯字1, 錯字2 -> 正字`(對齊既有 baseline 風格)

use std::fs;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum WriterError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrections.md 結構異常: {0}")]
    StructureError(String),
}

/// Append (wrong_variants -> suggested) 到 corrections.md User 段。
///
/// `wrong_variants`:要寫入的錯字 list(可能多個 variant 一次寫,例 ["英檔", "雲檔"])。
/// `suggested`:正字。
///
/// 若該 suggested 已在 User 段存在 → 合進那行(去重後);否則加新行。
/// 若 `## User` heading 不存在 → 在檔尾建。
pub fn append_correction(
    path: &Path,
    wrong_variants: &[String],
    suggested: &str,
) -> Result<(), WriterError> {
    if wrong_variants.is_empty() {
        return Err(WriterError::StructureError("wrong_variants 不能空".into()));
    }

    let content = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    let new_content = merge_into_user_section(&content, wrong_variants, suggested)?;

    // atomic write
    let tmp_path = path.with_extension("md.tmp");
    fs::write(&tmp_path, new_content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// 純函式版本,給 test 用 / 容易 reason。
pub(crate) fn merge_into_user_section(
    content: &str,
    wrong_variants: &[String],
    suggested: &str,
) -> Result<String, WriterError> {
    // 找 ## User heading 跟 ### 用戶自加 subsection 位置
    let lines: Vec<&str> = content.lines().collect();
    let user_h2_idx = lines.iter().position(|l| l.trim() == "## User");

    if user_h2_idx.is_none() {
        // ## User heading 不存在,在檔尾建整段
        let mut out = content.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("## User\n\n### 用戶自加\n\n");
        out.push_str(&format_line(wrong_variants, suggested));
        out.push('\n');
        return Ok(out);
    }

    let user_h2_idx = user_h2_idx.unwrap();
    // 找 ## User 之後的 ### 用戶自加 subsection
    let subsection_idx = lines
        .iter()
        .enumerate()
        .skip(user_h2_idx + 1)
        .find(|(_, l)| l.trim() == "### 用戶自加")
        .map(|(i, _)| i);

    if subsection_idx.is_none() {
        // 有 ## User 但沒 ### 用戶自加,在 ## User 之後插入 subsection + entry
        let mut out_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        // 在 user_h2_idx 之後插入空行 + ### 用戶自加 + 空行 + entry
        out_lines.insert(user_h2_idx + 1, String::new());
        out_lines.insert(user_h2_idx + 2, "### 用戶自加".into());
        out_lines.insert(user_h2_idx + 3, String::new());
        out_lines.insert(user_h2_idx + 4, format_line(wrong_variants, suggested));
        return Ok(join_lines(&out_lines));
    }

    let subsection_idx = subsection_idx.unwrap();
    // 找 subsection 段內是否有同 suggested 的 line
    // subsection 結束:下一個 ## 或 ### heading,或檔尾
    let subsection_end = lines
        .iter()
        .enumerate()
        .skip(subsection_idx + 1)
        .find(|(_, l)| {
            let t = l.trim();
            t.starts_with("## ") || t.starts_with("### ")
        })
        .map(|(i, _)| i)
        .unwrap_or(lines.len());

    let mut out_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    let target_suffix = format!(" -> {suggested}");
    // 找 subsection_idx..subsection_end 範圍內 "-" 起頭、結尾是 " -> <suggested>" 的 line
    let merge_line_idx = (subsection_idx + 1..subsection_end).find(|&i| {
        let l = out_lines[i].trim();
        l.starts_with("- ") && l.ends_with(&target_suffix)
    });

    if let Some(line_idx) = merge_line_idx {
        // merge:解 existing wrongs,union 新的,format 回寫
        let l = out_lines[line_idx].trim().to_string();
        let inner = l.strip_prefix("- ").unwrap();
        let arrow_pos = inner.rfind(" -> ").ok_or_else(|| {
            WriterError::StructureError(format!("既有 line 找不到 ' -> ': {l}"))
        })?;
        let existing_wrongs_str = &inner[..arrow_pos];
        let mut all_wrongs: Vec<String> = existing_wrongs_str
            .split(", ")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for w in wrong_variants {
            if !all_wrongs.iter().any(|e| e == w) {
                all_wrongs.push(w.clone());
            }
        }
        out_lines[line_idx] = format_line(&all_wrongs, suggested);
    } else {
        // 沒同 suggested line,在 subsection_end 之前(若 subsection_end 是 lines.len(),
        // 等於 push 到 subsection 末尾)插入新 line。
        // 跳過 trailing empty lines 找最後一個 non-empty line 之後插
        let mut insert_at = subsection_end;
        while insert_at > subsection_idx + 1
            && out_lines[insert_at - 1].trim().is_empty()
        {
            insert_at -= 1;
        }
        out_lines.insert(insert_at, format_line(wrong_variants, suggested));
    }

    Ok(join_lines(&out_lines))
}

fn format_line(wrong_variants: &[String], suggested: &str) -> String {
    format!("- {} -> {}", wrong_variants.join(", "), suggested)
}

fn join_lines(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_user_section_when_missing() {
        let content = "# Mori STT 校正字典\n\n## Baseline\n\n- 馬當 -> Markdown\n";
        let result = merge_into_user_section(content, &["英檔".into()], "音檔").unwrap();
        assert!(result.contains("## User"));
        assert!(result.contains("### 用戶自加"));
        assert!(result.contains("- 英檔 -> 音檔"));
        // baseline 不動
        assert!(result.contains("- 馬當 -> Markdown"));
    }

    #[test]
    fn merges_variant_into_existing_line() {
        let content = "## User\n\n### 用戶自加\n\n- 英檔 -> 音檔\n";
        let result = merge_into_user_section(content, &["雲檔".into()], "音檔").unwrap();
        assert!(result.contains("- 英檔, 雲檔 -> 音檔"));
        // 不該變兩行
        assert_eq!(result.matches(" -> 音檔").count(), 1);
    }

    #[test]
    fn adds_new_line_for_different_suggested() {
        let content = "## User\n\n### 用戶自加\n\n- 英檔 -> 音檔\n";
        let result = merge_into_user_section(content, &["馬當".into()], "Markdown").unwrap();
        assert!(result.contains("- 英檔 -> 音檔"));
        assert!(result.contains("- 馬當 -> Markdown"));
    }

    #[test]
    fn dedupes_variants() {
        let content = "## User\n\n### 用戶自加\n\n- 英檔 -> 音檔\n";
        // 「英檔」已存在,加同 variant 不應重複
        let result = merge_into_user_section(content, &["英檔".into(), "雲檔".into()], "音檔").unwrap();
        let line = result
            .lines()
            .find(|l| l.contains(" -> 音檔"))
            .unwrap();
        assert_eq!(line.matches("英檔").count(), 1);
        assert!(line.contains("雲檔"));
    }

    #[test]
    fn appends_subsection_when_user_section_exists_but_subsection_missing() {
        let content = "## User\n\n(空白)\n";
        let result = merge_into_user_section(content, &["英檔".into()], "音檔").unwrap();
        assert!(result.contains("### 用戶自加"));
        assert!(result.contains("- 英檔 -> 音檔"));
    }

    #[test]
    fn baseline_section_untouched() {
        let content = "## Baseline\n\n### 常見對話\n\n- 馬當 -> Markdown\n\n## User\n\n### 用戶自加\n\n- 英檔 -> 音檔\n";
        let result = merge_into_user_section(content, &["雲檔".into()], "音檔").unwrap();
        // baseline 馬當 line 該完整存在,且不該有 baseline 內被改動
        let baseline_line = result.lines().find(|l| l.contains("馬當")).unwrap();
        assert_eq!(baseline_line.trim(), "- 馬當 -> Markdown");
    }

    #[test]
    fn writes_atomically_via_tmp_rename() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("corrections.md");
        std::fs::write(&path, "## Baseline\n\n- 馬當 -> Markdown\n").unwrap();
        append_correction(&path, &["英檔".into()], "音檔").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("- 英檔 -> 音檔"));
        // .tmp 不該留下
        assert!(!path.with_extension("md.tmp").exists());
    }
}
