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

/// 跑 `git clone --depth 1` 拉 Anthropic skills repo 到 `~/.mori/skills/anthropics-skills/`,
/// 完成後對每個 `anthropics-skills/skills/<name>/` 建 symlink 到 `~/.mori/skills/<name>/`
/// (DF-2 flatten step)。
///
/// **DF-2 升級**:clone 完自動 flatten。`discover_skills` 掃扁平
/// `~/.mori/skills/<name>/SKILL.md`,symlink 走 follow,is_dir / is_file 全 work。
/// user 已有同名 skill 時 skip(以 user 自寫為優先)。
///
/// 失敗模式:
/// - `git` 不在 PATH → [`SkillInstallError::GitMissing`]
/// - `git clone` 退非 0 → [`SkillInstallError::CloneFailed`](exit code)
/// - HOME / USERPROFILE 都沒設 → [`SkillInstallError::NoHome`]
/// - mkdir parent / 其他 IO 錯 → [`SkillInstallError::Io`]
/// - flatten 階段個別 symlink 失敗 → log warning 跳過(不擋整 install)
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
    // 但 flatten 步驟還是要跑(可能第一次 clone 完崩了沒 flatten,或新加的 skill)。
    if target.join(".git").exists() {
        tracing::info!(path = %target.display(), "anthropic skills already installed; skipping clone");
        let _ = flatten_anthropic_skills(dest_root);
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

    // Flatten symlinks: anthropics-skills/skills/<name>/ → <dest_root>/<name>/
    match flatten_anthropic_skills(dest_root) {
        Ok(n) => tracing::info!(linked = n, "flattened anthropic skills"),
        Err(e) => tracing::warn!(error = %e, "flatten step failed (skills cloned but not symlinked)"),
    }

    Ok(target)
}

/// 對 `<dest_root>/anthropics-skills/skills/<name>/` 內每個 skill 目錄建一條
/// symlink 到 `<dest_root>/<name>/`,讓 [`mori_core::skill::discover_anthropic_skills`]
/// 掃扁平 layout 就能看到。
///
/// 行為:
/// - `anthropics-skills/skills/` 不存在 → 回 `Ok(0)`(沒 clone 完就被 cancel 的場景)
/// - 同名目錄已存在 → skip(user 自寫優先)
/// - symlink 個別失敗 → log warning 跳過,繼續其他 entry,最後回成功 count
///
/// Windows symlink 需要 dev mode / admin。fallback 在某些 Windows env 會失敗
/// (`ERROR_PRIVILEGE_NOT_HELD`);本 fn 把它當作普通 IO error 回傳,呼叫端 log
/// warning 後繼續。Linux / macOS 沒這問題。
fn flatten_anthropic_skills(dest_root: &Path) -> Result<usize, std::io::Error> {
    let anthropic_dir = dest_root.join(ANTHROPIC_SKILLS_SUBDIR).join("skills");
    if !anthropic_dir.exists() {
        tracing::debug!(
            path = %anthropic_dir.display(),
            "anthropics-skills/skills/ not present; nothing to flatten"
        );
        return Ok(0);
    }

    let mut count = 0;
    for entry in std::fs::read_dir(&anthropic_dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "skipping unreadable entry");
                continue;
            }
        };
        let link_target = entry.path();
        // 只 link directories(每個 skill = 一個目錄)
        if !link_target.is_dir() {
            continue;
        }
        let skill_name = entry.file_name();
        let link_path = dest_root.join(&skill_name);

        // user 自寫同名 skill 在 → skip(對齊 PATH-style: user 優先)
        // symlink 自身 .exists() follows link → 第二次跑也會 skip(idempotent)
        if link_path.exists() {
            tracing::debug!(
                name = %skill_name.to_string_lossy(),
                "skill already at flat path; skipping symlink"
            );
            continue;
        }

        let symlink_result = make_symlink(&link_target, &link_path);
        match symlink_result {
            Ok(()) => {
                count += 1;
                tracing::debug!(
                    name = %skill_name.to_string_lossy(),
                    target = %link_target.display(),
                    "linked anthropic skill"
                );
            }
            Err(e) => {
                tracing::warn!(
                    name = %skill_name.to_string_lossy(),
                    error = %e,
                    "symlink failed (continuing)"
                );
            }
        }
    }
    Ok(count)
}

/// 跨平台 symlink helper。Unix → `symlink`;Windows → `symlink_dir`(NTFS 對
/// dir / file symlink 分兩種 API)。
#[cfg(unix)]
fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
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

    // ─── flatten_anthropic_skills ─────────────────────────────

    #[test]
    fn flatten_returns_zero_when_anthropic_dir_missing() {
        // dest_root 完全空 → anthropics-skills/skills/ 不存在 → 0 link。
        let dir = TempDir::new().unwrap();
        let n = flatten_anthropic_skills(dir.path()).expect("should not error");
        assert_eq!(n, 0);
    }

    #[cfg(unix)]
    #[test]
    fn flatten_creates_symlinks_for_each_skill() {
        // mock: dest_root/anthropics-skills/skills/{pdf,docx}/SKILL.md
        // expect: dest_root/{pdf,docx} symlink 出現
        let dir = TempDir::new().unwrap();
        let dest = dir.path();
        let skills_inner = dest.join(ANTHROPIC_SKILLS_SUBDIR).join("skills");
        std::fs::create_dir_all(skills_inner.join("pdf")).unwrap();
        std::fs::create_dir_all(skills_inner.join("docx")).unwrap();
        std::fs::write(skills_inner.join("pdf").join("SKILL.md"), "x").unwrap();
        std::fs::write(skills_inner.join("docx").join("SKILL.md"), "y").unwrap();

        let n = flatten_anthropic_skills(dest).expect("flatten ok");
        assert_eq!(n, 2);
        assert!(dest.join("pdf").exists());
        assert!(dest.join("docx").exists());
        // symlink follows → SKILL.md 透過 link 可達
        assert!(dest.join("pdf").join("SKILL.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn flatten_skips_existing_user_skill() {
        // user 自寫 `~/.mori/skills/pdf/` 在 → flatten 不該覆蓋。
        let dir = TempDir::new().unwrap();
        let dest = dir.path();
        let skills_inner = dest.join(ANTHROPIC_SKILLS_SUBDIR).join("skills");
        std::fs::create_dir_all(skills_inner.join("pdf")).unwrap();
        std::fs::write(skills_inner.join("pdf").join("SKILL.md"), "anthropic-version").unwrap();

        // pre-create user own pdf dir(非 symlink)
        let user_pdf = dest.join("pdf");
        std::fs::create_dir_all(&user_pdf).unwrap();
        std::fs::write(user_pdf.join("SKILL.md"), "user-version").unwrap();

        let _ = flatten_anthropic_skills(dest).expect("flatten ok");
        // user 自寫不該被覆蓋:檔案內容仍是 user-version
        let content = std::fs::read_to_string(user_pdf.join("SKILL.md")).unwrap();
        assert_eq!(content, "user-version");
        // 而且 user_pdf 不該變成 symlink
        let meta = std::fs::symlink_metadata(&user_pdf).unwrap();
        assert!(!meta.file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn flatten_is_idempotent() {
        // 跑兩次:第二次不該爆 + 結果不變
        let dir = TempDir::new().unwrap();
        let dest = dir.path();
        let skills_inner = dest.join(ANTHROPIC_SKILLS_SUBDIR).join("skills");
        std::fs::create_dir_all(skills_inner.join("xlsx")).unwrap();
        std::fs::write(skills_inner.join("xlsx").join("SKILL.md"), "x").unwrap();

        let n1 = flatten_anthropic_skills(dest).expect("flatten 1");
        let n2 = flatten_anthropic_skills(dest).expect("flatten 2");
        assert_eq!(n1, 1);
        assert_eq!(n2, 0, "second flatten should skip existing link");
        assert!(dest.join("xlsx").exists());
    }

    #[cfg(unix)]
    #[test]
    fn flatten_skips_non_directory_entries() {
        // anthropics-skills/skills/README.md 之類的檔案不要建 symlink
        let dir = TempDir::new().unwrap();
        let dest = dir.path();
        let skills_inner = dest.join(ANTHROPIC_SKILLS_SUBDIR).join("skills");
        std::fs::create_dir_all(&skills_inner).unwrap();
        std::fs::write(skills_inner.join("README.md"), "info").unwrap();
        std::fs::create_dir_all(skills_inner.join("real-skill")).unwrap();
        std::fs::write(skills_inner.join("real-skill").join("SKILL.md"), "x").unwrap();

        let n = flatten_anthropic_skills(dest).expect("flatten ok");
        assert_eq!(n, 1);
        assert!(dest.join("real-skill").exists());
        assert!(!dest.join("README.md").exists(), "files at top level shouldn't be linked");
    }
}
