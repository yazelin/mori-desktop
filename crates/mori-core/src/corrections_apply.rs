//! Deterministic post-cleanup substitution for corrections.md。
//!
//! Voice cleanup pipeline 的 LLM step 漏率 ~50%,即使字典已注入 system prompt
//! 也不保證套。此模組提供確定性的 string substitute 兜底:
//!
//! 1. Parse corrections.md entries — 每行 `wrong, wrong2 -> suggested`
//! 2. 排序 wrong variants 按字串長度遞減(避免 substring 部分套到)
//! 3. 對 cleaned text 跑 `text.replace(wrong, suggested)`
//!
//! 設計:單純字典條目自然 context-free(就是 STT 諧音怪字 → 正字),正常書面
//! 中文不會誤觸。如果 user 自己寫進去某個會誤觸的 pair,user 是 quality gate。

use std::collections::HashSet;

/// 從 corrections.md 抽出 (wrong_variant, suggested) entries。
///
/// 接受 `## Baseline` / `## User` / `### 用戶自加` 等任意 heading 結構 — 純掃描
/// 開頭 `- ` 的行,parse `wrong1, wrong2, ... -> suggested` format。
pub fn parse_corrections(content: &str) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for line in content.lines() {
        let t = line.trim();
        if !t.starts_with("- ") {
            continue;
        }
        let inner = &t[2..];
        let arrow_pos = match inner.rfind(" -> ") {
            Some(p) => p,
            None => continue,
        };
        let wrongs_str = inner[..arrow_pos].trim();
        let suggested = inner[arrow_pos + 4..].trim();
        if suggested.is_empty() {
            continue;
        }
        for wrong in wrongs_str.split(',') {
            let w = wrong.trim();
            if w.is_empty() || w == suggested {
                continue;
            }
            let pair = (w.to_string(), suggested.to_string());
            if seen.insert(pair.clone()) {
                entries.push(pair);
            }
        }
    }
    // 排序按 wrong 長度遞減,避免 substring conflict
    // (例 `馬當奴 -> X` + `馬當 -> Y`,先套長的免得短的吃掉)
    entries.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()));
    entries
}

/// 對 text 套用 corrections。strict string `text.replace(wrong, suggested)`,
/// 從最長 wrong 開始套。
pub fn apply_corrections(text: &str, corrections_md: &str) -> String {
    let entries = parse_corrections(corrections_md);
    let mut result = text.to_string();
    for (wrong, suggested) in &entries {
        if result.contains(wrong.as_str()) {
            result = result.replace(wrong.as_str(), suggested.as_str());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_entry() {
        let md = "- 英檔 -> 音檔\n";
        let entries = parse_corrections(md);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], ("英檔".to_string(), "音檔".to_string()));
    }

    #[test]
    fn parse_multi_variant_entry() {
        let md = "- 英檔, 雲檔, 音檔錯 -> 音檔\n";
        let entries = parse_corrections(md);
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|(w, s)| w == "英檔" && s == "音檔"));
        assert!(entries.iter().any(|(w, s)| w == "雲檔" && s == "音檔"));
        assert!(entries.iter().any(|(w, s)| w == "音檔錯" && s == "音檔"));
    }

    #[test]
    fn parse_with_baseline_and_user_headings() {
        let md = "## Baseline\n\n### 諧音\n\n- 馬當 -> Markdown\n- 殺核 -> 沙盒\n\n## User\n\n### 用戶自加\n\n- Mardong -> Markdown\n";
        let entries = parse_corrections(md);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn parse_dedupes_repeated_pair() {
        let md = "- A -> X\n- A -> X\n";
        let entries = parse_corrections(md);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn parse_ignores_lines_without_arrow() {
        let md = "## Heading\n\nsome text\n- 馬當 -> Markdown\n  random\n";
        let entries = parse_corrections(md);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn apply_replaces_single_word() {
        let md = "- 英檔 -> 音檔\n";
        let result = apply_corrections("我看英檔內容", md);
        assert_eq!(result, "我看音檔內容");
    }

    #[test]
    fn apply_replaces_multiple_variants_to_same_target() {
        let md = "- Makdang, Mardong, Modang -> Markdown\n";
        let cleaned = "Makdang 是 Mardong 又是 Modang";
        let result = apply_corrections(cleaned, md);
        assert_eq!(result, "Markdown 是 Markdown 又是 Markdown");
    }

    #[test]
    fn apply_long_variant_first_avoids_substring_conflict() {
        // 長 variant `馬當奴` 跟短 variant `馬當` 同時存在,長的先套
        let md = "- 馬當 -> Markdown\n- 馬當奴 -> 米切爾\n";
        let result = apply_corrections("馬當奴的馬當", md);
        // 預期:`馬當奴` → `米切爾`,然後剩下的單獨 `馬當` → `Markdown`
        assert_eq!(result, "米切爾的Markdown");
    }

    #[test]
    fn apply_noop_when_no_match() {
        let md = "- 英檔 -> 音檔\n";
        let text = "完全無關的中文";
        assert_eq!(apply_corrections(text, md), text);
    }

    #[test]
    fn apply_noop_when_empty_corrections() {
        assert_eq!(apply_corrections("文字", ""), "文字");
    }

    #[test]
    fn apply_noop_when_identical_wrong_suggested() {
        // wrong == suggested 不會建 entry
        let md = "- 音檔 -> 音檔\n";
        assert_eq!(apply_corrections("音檔", md), "音檔");
    }
}
