//! Wave 6 D-full(DF-1):Anthropic skills install helper + Tauri commands。
//!
//! 提供一個 **更精細** 的 install 路徑(對比 DepsTab 的 `InstallSpec::Shell`):
//! - 自家檢測 git 在不在 PATH(`SkillInstallError::GitMissing` 明確錯誤)
//! - 自家管 destination layout(`~/.mori/skills/anthropics-skills/`)
//! - 跨平台:不 spawn `sh`(`InstallSpec::Shell` 走 sh -c 在 Windows 需要 Git Bash),
//!   走 `Command::new("git")` direct invoke
//!
//! 整合路徑:
//! 1. 本 module 提供 [`install_anthropic_skills_cmd`] / [`anthropic_skills_status_cmd`]
//!    兩個 Tauri commands(前端可直接呼叫,例如「Install Anthropic Skills」按鈕)
//! 2. 也提供 internal helpers([`skills_dir`] / [`is_anthropic_skills_installed`] /
//!    [`install_anthropic_skills`])給 main.rs / 其他 module 用
//! 3. install 完成後 Stream I (#79) 的 `discover_skills` 會掃到 — 注意 Anthropic
//!    repo 結構是 `anthropics-skills/skills/<name>/`,而 loader 掃 `~/.mori/skills/<name>/`
//!    扁平路徑,完整 discovery 整合(symlink flatten / loader 補)留 DF-2 做。
//!
//! 對應 deps.rs 內的 `anthropic-skills` DepSpec entry:那條走 `InstallSpec::Shell`
//! 直接跑 `git clone`,這份 module 是「更乾淨的 Rust path」— 兩條路徑同時存在,user
//! 走任一條都能成功。

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

/// Anthropic 官方 skills repo。
const ANTHROPIC_SKILLS_REPO: &str = "https://github.com/anthropics/skills.git";

/// Anthropic skills 在本機的安裝子目錄(相對於 [`skills_dir`])。
const ANTHROPIC_SKILLS_SUBDIR: &str = "anthropics-skills";

#[derive(Debug, Error)]
pub enum SkillInstallError {
    #[error("git not found in PATH (please install git first)")]
    GitMissing,
    #[error("git clone failed (exit {0:?})")]
    CloneFailed(Option<i32>),
    #[error("home dir not found (HOME / USERPROFILE both unset)")]
    NoHome,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// `~/.mori/skills/` — 跟 [`mori_core::skill::anthropic_skill::default_skills_dir`]
/// 對齊,但本 module 不 depend mori-core(避免循環),自己組同樣 path。
///
/// 回 `None` 表示 HOME / USERPROFILE 都沒設(極稀有,容器 / 純 minimal env 才會)。
pub fn skills_dir() -> Option<PathBuf> {
    skills_dir_with_home(home_dir())
}

/// 取 home dir — HOME 優先,fallback USERPROFILE(Windows)。
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 純 functional helper,給 test 注入 home dir 用。`None` 入 → `None` 出。
fn skills_dir_with_home(home: Option<PathBuf>) -> Option<PathBuf> {
    home.map(|h| h.join(".mori").join("skills"))
}

/// 檢查 Anthropic skills 是否已 install — 看 `~/.mori/skills/anthropics-skills/.git`
/// 是否存在(`git clone` 完一定有 `.git` 目錄)。
pub fn is_anthropic_skills_installed() -> bool {
    skills_dir()
        .map(|dir| is_anthropic_skills_installed_at(&dir))
        .unwrap_or(false)
}

/// 純 functional helper,給 test 注入 skills dir 用。
fn is_anthropic_skills_installed_at(skills_dir: &Path) -> bool {
    skills_dir.join(ANTHROPIC_SKILLS_SUBDIR).join(".git").exists()
}

/// 跑 `git clone --depth 1` 拉 Anthropic skills repo 到 `~/.mori/skills/anthropics-skills/`。
///
/// **不負責 discovery flatten** — 該事留 DF-2 補。本 fn 完成 → user 重啟 Mori,
/// `discover_skills` 走原邏輯掃不到深層 `skills/<name>/SKILL.md`,需要前端 / loader
/// 升級配合。
///
/// 失敗模式:
/// - `git` 不在 PATH → [`SkillInstallError::GitMissing`]
/// - `git clone` 退非 0 → [`SkillInstallError::CloneFailed`](exit code)
/// - HOME / USERPROFILE 都沒設 → [`SkillInstallError::NoHome`]
/// - mkdir parent / 其他 IO 錯 → [`SkillInstallError::Io`]
pub fn install_anthropic_skills() -> Result<PathBuf, SkillInstallError> {
    let dest_root = skills_dir().ok_or(SkillInstallError::NoHome)?;
    install_anthropic_skills_at(&dest_root)
}

/// 內部 helper:把 install destination 注入,給 test 用(同時也是 prod path 的
/// 真正實作)。
fn install_anthropic_skills_at(dest_root: &Path) -> Result<PathBuf, SkillInstallError> {
    std::fs::create_dir_all(dest_root)?;
    let target = dest_root.join(ANTHROPIC_SKILLS_SUBDIR);

    // 已經有 .git → 不重複 clone,直接回傳 path(idempotent)。
    // user 想更新走 `git pull`(deps.rs 內的 Shell variant 有 fallback path)。
    if target.join(".git").exists() {
        tracing::info!(path = %target.display(), "anthropic skills already installed; skipping clone");
        return Ok(target);
    }

    let mut cmd = Command::new("git");
    cmd.args(["clone", "--depth", "1", ANTHROPIC_SKILLS_REPO])
        .arg(&target);
    mori_core::suppress_console_on_windows!(cmd);

    let status = cmd.status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SkillInstallError::GitMissing
        } else {
            SkillInstallError::Io(e)
        }
    })?;

    if !status.success() {
        return Err(SkillInstallError::CloneFailed(status.code()));
    }

    tracing::info!(path = %target.display(), "anthropic skills installed");
    Ok(target)
}

// ─── Tauri commands ────────────────────────────────────────────────

/// 前端「Install Anthropic Skills」按鈕呼叫。spawn_blocking 內跑 — `git clone`
/// 可能跑數十秒(網路 + repo size),不該擋 async runtime。
#[tauri::command]
pub async fn install_anthropic_skills_cmd() -> Result<String, String> {
    let path = tokio::task::spawn_blocking(install_anthropic_skills)
        .await
        .map_err(|e| format!("install join: {e}"))?
        .map_err(|e| e.to_string())?;
    Ok(format!("Installed to {}", path.display()))
}

/// 前端「is installed?」狀態查詢。同步 — 只看一個檔案存不存在,微秒級。
#[tauri::command]
pub fn anthropic_skills_status_cmd() -> bool {
    is_anthropic_skills_installed()
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ─── skills_dir_with_home ─────────────────────────────────

    #[test]
    fn skills_dir_with_home_returns_some_when_home_set() {
        let home = PathBuf::from("/home/test");
        let got = skills_dir_with_home(Some(home.clone())).expect("should be Some");
        assert_eq!(got, home.join(".mori").join("skills"));
    }

    #[test]
    fn skills_dir_with_home_returns_none_when_home_absent() {
        assert!(skills_dir_with_home(None).is_none());
    }

    // ─── is_anthropic_skills_installed_at ─────────────────────

    #[test]
    fn is_installed_returns_false_when_dir_missing() {
        let dir = TempDir::new().unwrap();
        let skills_root = dir.path().join("skills");
        // skills_root 不存在 — `.git` 路徑解析會成功(不存在的 join),
        // 但 .exists() 回 false。
        assert!(!is_anthropic_skills_installed_at(&skills_root));
    }

    #[test]
    fn is_installed_returns_false_when_subdir_present_but_no_git() {
        // anthropics-skills/ 在但沒 .git → 不算 installed(可能是 user 手動建空目錄)。
        let dir = TempDir::new().unwrap();
        let skills_root = dir.path().to_path_buf();
        std::fs::create_dir_all(skills_root.join(ANTHROPIC_SKILLS_SUBDIR)).unwrap();
        assert!(!is_anthropic_skills_installed_at(&skills_root));
    }

    #[test]
    fn is_installed_returns_true_when_git_subdir_exists() {
        // mock git clone 後的狀態:anthropics-skills/.git/ 存在。
        let dir = TempDir::new().unwrap();
        let skills_root = dir.path().to_path_buf();
        let git_dir = skills_root.join(ANTHROPIC_SKILLS_SUBDIR).join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        assert!(is_anthropic_skills_installed_at(&skills_root));
    }

    #[test]
    fn is_installed_accepts_git_file_or_dir() {
        // git worktree / submodule 下 `.git` 可能是 file 而非 dir。.exists() 兩者皆 true。
        let dir = TempDir::new().unwrap();
        let skills_root = dir.path().to_path_buf();
        let subdir = skills_root.join(ANTHROPIC_SKILLS_SUBDIR);
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join(".git"), "gitdir: ../../somewhere\n").unwrap();
        assert!(is_anthropic_skills_installed_at(&skills_root));
    }

    // ─── install_anthropic_skills_at ──────────────────────────

    #[test]
    fn install_short_circuits_when_already_present() {
        // pre-create .git → install 不該再 spawn git(就算 PATH 沒 git 也要回 Ok)。
        let dir = TempDir::new().unwrap();
        let skills_root = dir.path().to_path_buf();
        let target = skills_root.join(ANTHROPIC_SKILLS_SUBDIR);
        std::fs::create_dir_all(target.join(".git")).unwrap();

        let got = install_anthropic_skills_at(&skills_root).expect("idempotent install");
        assert_eq!(got, target);
    }

    #[test]
    fn install_creates_parent_dirs() {
        // dest_root 深層巢狀不存在 → create_dir_all 要先把它建出來(才好 clone)。
        // 不實際跑 clone(會打網路),用 short-circuit 路徑驗:預先 mock .git 後,
        // 確認 install 不會 fail / 不會清掉父目錄。
        let dir = TempDir::new().unwrap();
        let deep = dir.path().join("a").join("b").join("c");
        // pre-create .git 走 short-circuit(跳過 git clone)
        let target = deep.join(ANTHROPIC_SKILLS_SUBDIR);
        std::fs::create_dir_all(target.join(".git")).unwrap();

        let got = install_anthropic_skills_at(&deep).expect("should succeed");
        assert!(got.exists());
        assert!(deep.exists(), "parent dir should be preserved");
    }

    #[test]
    fn install_returns_git_missing_when_git_not_in_path() {
        // 走真正的 spawn path,但 PATH 設成空 → `git` 找不到 → GitMissing。
        // 注意:set_var 是 process-global,單測會跟其他並行 test 互干擾;這個 test
        // 故意只測 install_anthropic_skills_at(不碰 skills_dir / env),改動完
        // 立刻 restore。
        let dir = TempDir::new().unwrap();
        let skills_root = dir.path().to_path_buf();

        // Save + clear PATH
        let saved_path = std::env::var_os("PATH");
        std::env::set_var("PATH", "");

        let result = install_anthropic_skills_at(&skills_root);

        // Restore PATH 先,免得後續 test 受影響。
        match saved_path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }

        match result {
            Err(SkillInstallError::GitMissing) => {}
            other => panic!("expected GitMissing, got {other:?}"),
        }
    }

    // ─── ANTHROPIC_SKILLS_SUBDIR constant ─────────────────────

    #[test]
    fn anthropic_skills_subdir_is_stable() {
        // Stream I loader 跟 deps.rs 內的 check path_template 都依賴這個常數值。
        // 改了這個 = 同步要改 deps.rs 的 `$HOME/.mori/skills/anthropics-skills/.git`。
        assert_eq!(ANTHROPIC_SKILLS_SUBDIR, "anthropics-skills");
    }
}
