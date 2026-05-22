//! L-mori — read `~/mori-universe/spirits/<name>/wiki/` Karpathy LLM Wiki 結構。
//!
//! # 為什麼存在
//!
//! Wave 7 「L 記憶之森」(Karpathy LLM Wiki pattern)在 mori-desktop 側的整合。
//! Mori 在 `~/mori-universe/spirits/<name>/wiki/` 維護一份累積的內在百科 — 每個
//! page 是 .md 檔(`people/yazelin.md`、`projects/mori.md`、`concepts/...`)。
//! 啟動時讀 `wiki/index.md` 進 system prompt 讓 LLM 知道**有哪些 page 可拉**;
//! LLM 需要時主動呼叫 `read_wiki_page(page)` skill 把 specific page 內容拉進
//! context window。
//!
//! # 範圍 — 純 READ
//!
//! 本 module **read-only**。**不寫 vault**。對應 mori-journal CLAUDE.md 硬規矩 3
//! 「identity/ / memories/ 禁止 ghost-write」— wiki/ 雖然不是 identity / memories
//! 但同層次,屬 Mori 的「內在生命」,要寫先 explicit re-authorize。後續 Wave 8
//! annuli reflection / curator 自動編譯 wiki page 是 future work(needs yazelin
//! per-dir auth)— **本 stream 不碰**。
//!
//! # 預期 wiki 結構(per `mori-jarvis-direction.md §6`)
//!
//! ```text
//! ~/mori-universe/spirits/<spirit>/wiki/
//! ├── raw/                  # 不可變來源(conversations / articles / meetings)
//! ├── wiki/                 # LLM 編譯的「百科」(flat 階層 people/projects/...)
//! ├── index.md              # 入口:列全部 page + 短描述(單 context window 大小)
//! ├── AGENTS.md             # Mori 怎麼用 wiki 的規則(user 可改)
//! └── log.md                # Mori 動過什麼的 audit trail
//! ```
//!
//! # graceful skip
//!
//! - 第一次跑 mori-desktop:wiki/ 還沒建 → [`read_index`] 回 `None`,system prompt
//!   整段不 emit,行為不破。
//! - User 後續 mkdir + 寫 index.md → 下次啟動自動讀到。
//! - 空 index.md → 同 `None`(沒內容塞 prompt 沒意義)。
//!
//! # path traversal 防護
//!
//! [`read_wiki_page`] resolve 完路徑 **必須留在 wiki/ 內**;
//! `../../../../etc/passwd`、絕對路徑、symlink 跳出 wiki/ → `WikiError::PathTraversal`。

use std::path::{Path, PathBuf};

/// `<vault_root>/<spirit_name>/wiki/` 的絕對路徑。
///
/// 不檢查存在性 — caller 用 [`read_index`] / [`read_wiki_page`] 個別 dispatch。
pub fn wiki_root(vault_root: &Path, spirit_name: &str) -> PathBuf {
    vault_root.join(spirit_name).join("wiki")
}

/// 讀 `wiki/index.md` 全文。
///
/// 行為:
/// - 不存在 / read 失敗 → `None`
/// - 空檔(trim 後 empty)→ `None`(沒內容塞 system prompt 沒意義)
/// - 有內容 → `Some(content)`
///
/// 取 graceful skip 設計:wiki 沒建好不該擋 startup,LLM 多/少這段都能跑。
pub fn read_index(vault_root: &Path, spirit_name: &str) -> Option<String> {
    let path = wiki_root(vault_root, spirit_name).join("index.md");
    read_md_file(&path)
}

/// 讀 `wiki/AGENTS.md` 全文 — Mori 怎麼用 wiki 的規則(user 可改)。
///
/// 同 [`read_index`] graceful 行為:不存在 / 空 → `None`。
pub fn read_agents_md(vault_root: &Path, spirit_name: &str) -> Option<String> {
    let path = wiki_root(vault_root, spirit_name).join("AGENTS.md");
    read_md_file(&path)
}

/// 讀 `wiki/<page_relative>` 內容。
///
/// `page_relative` 是 wiki/ 內的相對路徑(eg `"people/yazelin.md"`、
/// `"concepts/transformer.md"`)。
///
/// # 安全性(path traversal 防護)
///
/// 1. resolve 後路徑 **必須留在 wiki_root() 之下**。`../../etc/passwd` /
///    絕對路徑 / symlink 跳出 → [`WikiError::PathTraversal`]。
/// 2. canonicalize 前先做 string-level reject(`..` segments 直接拒);
///    canonicalize 需要檔案存在,缺檔的 path traversal 也擋得住。
///
/// # 錯誤
///
/// - [`WikiError::PathTraversal`]:path 含 `..` 或 resolved 跳出 wiki_root
/// - [`WikiError::NotFound`]:檔案不存在
/// - [`WikiError::Io`]:read 失敗(權限 / IO error)
///
/// 註:目前 mori-core 的 `ReadWikiPageSkill` 走 self-contained inline copy
/// (避免 mori-core depend mori-tauri 的反向依賴)。這個 public function 是
/// 留給未來 Tauri command(eg `read_wiki_page_cmd` 給前端 JS)/ admin 工具用,
/// 所以掛 `#[allow(dead_code)]` 對齊 `fetch_canonical_soul` 同 pattern。
#[allow(dead_code)]
pub fn read_wiki_page(
    vault_root: &Path,
    spirit_name: &str,
    page_relative: &str,
) -> Result<String, WikiError> {
    let wiki = wiki_root(vault_root, spirit_name);

    // String-level reject:含 `..` segment、絕對路徑、Windows drive letter 一律拒。
    // 在 canonicalize 前做 — 缺檔的 traversal attempt 也擋得住。
    if page_relative.is_empty() {
        return Err(WikiError::PathTraversal(page_relative.to_string()));
    }
    let pr_path = PathBuf::from(page_relative);
    if pr_path.is_absolute() {
        return Err(WikiError::PathTraversal(page_relative.to_string()));
    }
    for component in pr_path.components() {
        use std::path::Component;
        match component {
            Component::Normal(_) => {}
            // ParentDir = `..`、RootDir = `/`、Prefix = `C:` 等都拒
            _ => return Err(WikiError::PathTraversal(page_relative.to_string())),
        }
    }

    let target = wiki.join(&pr_path);

    if !target.exists() {
        return Err(WikiError::NotFound(page_relative.to_string()));
    }

    // canonicalize 雙保險 — 防 symlink 跳出 wiki_root。wiki_root 不存在會 fail,
    // 但走到這 target 存在 + target 一定 under wiki_root.join(...),所以 wiki_root
    // 也存在。
    let wiki_canon = wiki
        .canonicalize()
        .map_err(WikiError::Io)?;
    let target_canon = target.canonicalize().map_err(WikiError::Io)?;
    if !target_canon.starts_with(&wiki_canon) {
        return Err(WikiError::PathTraversal(page_relative.to_string()));
    }

    std::fs::read_to_string(&target_canon).map_err(WikiError::Io)
}

/// 讀 .md 檔的共用 helper:不存在 / 失敗 / 空 → `None`。
fn read_md_file(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => Some(s),
        Ok(_) => {
            tracing::debug!(
                path = %path.display(),
                "wiki .md file exists but is empty, treating as missing"
            );
            None
        }
        Err(e) => {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "wiki .md not readable"
            );
            None
        }
    }
}

/// `read_wiki_page` 的錯誤型別。同上,留 public 給未來 Tauri command。
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum WikiError {
    #[error("path traversal not allowed: {0}")]
    PathTraversal(String),
    #[error("page not found: {0}")]
    NotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 建一個 fake vault `vault_root/<spirit>/wiki/` + 內部結構,給 tests 用。
    fn make_vault(spirit: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let wiki = dir.path().join(spirit).join("wiki");
        fs::create_dir_all(&wiki).expect("mkdir wiki");
        (dir, wiki)
    }

    #[test]
    fn wiki_root_returns_expected_path() {
        let dir = tempfile::tempdir().unwrap();
        let got = wiki_root(dir.path(), "mori");
        assert_eq!(got, dir.path().join("mori").join("wiki"));
    }

    #[test]
    fn read_index_returns_none_when_missing() {
        let (dir, _) = make_vault("mori");
        // 沒寫 index.md → None
        let got = read_index(dir.path(), "mori");
        assert!(got.is_none());
    }

    #[test]
    fn read_index_returns_content_when_exists() {
        let (dir, wiki) = make_vault("mori");
        fs::write(
            wiki.join("index.md"),
            "# Mori 的 wiki index\n\n- people/yazelin.md — 主要 user\n",
        )
        .unwrap();

        let got = read_index(dir.path(), "mori").expect("should read index");
        assert!(got.contains("Mori 的 wiki index"));
        assert!(got.contains("people/yazelin.md"));
    }

    #[test]
    fn read_index_returns_none_for_empty_file() {
        let (dir, wiki) = make_vault("mori");
        fs::write(wiki.join("index.md"), "   \n\n  \n").unwrap();
        // trim 後 empty → None(沒內容塞 system prompt 沒意義)
        assert!(read_index(dir.path(), "mori").is_none());
    }

    #[test]
    fn read_agents_md_returns_content_when_exists() {
        let (dir, wiki) = make_vault("mori");
        fs::write(
            wiki.join("AGENTS.md"),
            "# Wiki 使用規則\n\n當 user 問到 X 時,先讀 wiki/X.md\n",
        )
        .unwrap();

        let got = read_agents_md(dir.path(), "mori").expect("should read AGENTS");
        assert!(got.contains("Wiki 使用規則"));
    }

    #[test]
    fn read_agents_md_returns_none_when_missing() {
        let (dir, _) = make_vault("mori");
        assert!(read_agents_md(dir.path(), "mori").is_none());
    }

    #[test]
    fn read_wiki_page_returns_content() {
        let (dir, wiki) = make_vault("mori");
        fs::create_dir_all(wiki.join("people")).unwrap();
        fs::write(
            wiki.join("people").join("yazelin.md"),
            "# Yazelin\n\nMori 的 主要 user。\n",
        )
        .unwrap();

        let got = read_wiki_page(dir.path(), "mori", "people/yazelin.md")
            .expect("should read page");
        assert!(got.contains("Yazelin"));
        assert!(got.contains("主要 user"));
    }

    #[test]
    fn read_wiki_page_rejects_path_traversal() {
        let (dir, wiki) = make_vault("mori");
        // 在 vault_root 外造一個 secret 檔案。
        fs::write(dir.path().join("secret.txt"), "TOP SECRET\n").unwrap();
        // 假裝 wiki/ 下也有 dummy 讓 wiki 存在(canonicalize 可走)
        fs::write(wiki.join("decoy.md"), "decoy\n").unwrap();

        // `../../secret.txt` — 應該拒(`..` segment 直接拒)
        let err = read_wiki_page(dir.path(), "mori", "../../secret.txt")
            .expect_err("should reject path traversal");
        assert!(matches!(err, WikiError::PathTraversal(_)));

        // `../../../etc/passwd` — 同
        let err = read_wiki_page(dir.path(), "mori", "../../../etc/passwd")
            .expect_err("should reject path traversal");
        assert!(matches!(err, WikiError::PathTraversal(_)));

        // 絕對路徑 — 拒
        let err = read_wiki_page(dir.path(), "mori", "/etc/passwd")
            .expect_err("should reject absolute path");
        assert!(matches!(err, WikiError::PathTraversal(_)));
    }

    #[test]
    fn read_wiki_page_returns_not_found_for_missing() {
        let (dir, _) = make_vault("mori");
        let err = read_wiki_page(dir.path(), "mori", "people/nonexistent.md")
            .expect_err("missing page should err");
        assert!(matches!(err, WikiError::NotFound(_)));
    }

    #[test]
    fn read_wiki_page_rejects_empty_relative() {
        let (dir, _) = make_vault("mori");
        let err = read_wiki_page(dir.path(), "mori", "")
            .expect_err("empty page_relative should err");
        assert!(matches!(err, WikiError::PathTraversal(_)));
    }
}
