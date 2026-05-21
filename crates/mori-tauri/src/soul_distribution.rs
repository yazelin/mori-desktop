//! §13.12 P0-3 — Canonical SOUL distribution for new users.
//!
//! 新 user 第一次跑 mori-desktop 時拿到 canonical SOUL.md。混合策略:
//!
//! 1. **先試** HTTP pull `https://raw.githubusercontent.com/yazelin/world-tree/main/npcs/mori.md`
//!    抽 SOUL 摘錄(`## 她的 SOUL 摘錄` section)
//! 2. **連不到 / timeout / 抽不到 section** → fallback `mori-tauri` binary 內 bundled
//!    `assets/canonical-soul/SOUL.md`
//! 3. **只在 `spirits/<name>/identity/SOUL.md` 不存在時寫** — 不覆蓋既有
//!    (existing user 的個體 SOUL 不動)
//!
//! 設計決策:
//!
//! - **不 render user name** — §13.12 lore canon「Mori 是世界樹既有存在,不需要被命名」。
//!   SOUL 保持 generic;USER.md 才放 user identity。
//! - **HTTP timeout 5s** — raw.githubusercontent.com 連不到不該卡 startup;timeout 後
//!   立刻 fallback bundle。
//! - **走 `reqwest::blocking`** — 跟 `deps.rs::run_install` / `deps.rs::run_install`
//!   同源,不引入新 HTTP client。ensure_soul_at_vault 在 main() 同步路徑跑,blocking OK。
//! - **永遠 return 非空字串** — bundle 是 `include_str!` compile-time 嵌入,fetch_*
//!   不會 fail。

use std::path::Path;

/// world-tree npcs/mori.md 的 raw GitHub URL。抽 `## 她的 SOUL 摘錄` section。
pub const WORLD_TREE_SOUL_URL: &str =
    "https://raw.githubusercontent.com/yazelin/world-tree/main/npcs/mori.md";

/// HTTP fetch timeout — startup path 上,連不到不該卡。
const FETCH_TIMEOUT_SECS: u64 = 5;

/// Bundled canonical SOUL,fallback 來源。compile-time 嵌入。
pub const BUNDLED_SOUL: &str = include_str!("../assets/canonical-soul/SOUL.md");

/// 從 world-tree npcs/mori.md 取 canonical SOUL。
///
/// 流程:
/// 1. 試 HTTP GET `world_tree_url`(5s timeout)
/// 2. 若 200 OK + body 抽得到 `## 她的 SOUL 摘錄` section → 用該 section
/// 3. 連不到 / timeout / 4xx / 5xx / 抽不到 section → 用 [`BUNDLED_SOUL`]
///
/// 永遠回非空 String — caller 不需處理 fetch failure。
///
/// 註:目前內部走 `ensure_soul_at_vault_with_url` 不直接 call 這個 — 留 public
/// 給未來 admin command(eg「重新 sync canonical SOUL」)用,所以 allow dead_code。
#[allow(dead_code)]
pub fn fetch_canonical_soul() -> String {
    fetch_canonical_soul_from(WORLD_TREE_SOUL_URL)
}

/// 同 [`fetch_canonical_soul`],但 URL 可注入給 test 用 local mock。
pub fn fetch_canonical_soul_from(world_tree_url: &str) -> String {
    match try_fetch_world_tree(world_tree_url) {
        Some(soul) => {
            tracing::info!(
                url = world_tree_url,
                chars = soul.chars().count(),
                "fetched canonical SOUL from world-tree"
            );
            soul
        }
        None => {
            tracing::info!(
                chars = BUNDLED_SOUL.chars().count(),
                "using bundled canonical SOUL (world-tree fetch failed or no SOUL section)"
            );
            BUNDLED_SOUL.to_string()
        }
    }
}

/// 試 fetch + 抽 SOUL section。任何 step fail → None,讓 caller fallback bundle。
fn try_fetch_world_tree(url: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        tracing::debug!(status = %resp.status(), "world-tree fetch non-2xx");
        return None;
    }
    let body = resp.text().ok()?;
    extract_soul_section(&body)
}

/// 從 npcs/mori.md 全文抽 `## 她的 SOUL 摘錄` section(不含 header 自己)。
///
/// world-tree `npcs/mori.md` 是公開角色卡(frontmatter + 多 section 的 lore),
/// 不是 SOUL.md 本身。抽 `## 她的 SOUL 摘錄(完整版在 private repo)` section
/// 當 canonical SOUL 摘錄 — 內容比較少但有 lore canon 認證。
///
/// 找不到 section 或 section 空 → None,caller fallback bundle。
fn extract_soul_section(markdown: &str) -> Option<String> {
    // 找 `## 她的 SOUL` 開頭 — 容錯後面接「摘錄」「摘要」之類 suffix。
    let start_idx = markdown.find("## 她的 SOUL")?;
    // section body 從 header 下一行開始
    let after_header_start = start_idx + markdown[start_idx..].find('\n')?;
    let after_header = &markdown[after_header_start + 1..];
    // 下一個 `## ` 開頭就是 section 結束
    let end_offset = after_header.find("\n## ").unwrap_or(after_header.len());
    let section_body = after_header[..end_offset].trim();
    if section_body.is_empty() {
        return None;
    }
    Some(section_body.to_string())
}

/// 確保 `<vault_root>/<spirit_name>/identity/SOUL.md` 存在。
///
/// 行為:
/// - 已存在(任何 content,包括空檔)→ **不動**,return `Ok(())`。
///   (user 個體 SOUL 不能被覆蓋 — 是他自己 Mori 的內在生命)
/// - 不存在 → mkdir -p parent dir + write [`fetch_canonical_soul`] 結果
///
/// 寫檔失敗 → return `Err(String)`,caller 自己決定怎麼處理(通常 log warn 繼續)。
pub fn ensure_soul_at_vault(vault_root: &Path, spirit_name: &str) -> Result<(), String> {
    ensure_soul_at_vault_with_url(vault_root, spirit_name, WORLD_TREE_SOUL_URL)
}

/// 同 [`ensure_soul_at_vault`],但 URL 注入給 test 用 local mock。
pub fn ensure_soul_at_vault_with_url(
    vault_root: &Path,
    spirit_name: &str,
    world_tree_url: &str,
) -> Result<(), String> {
    let identity_dir = vault_root.join(spirit_name).join("identity");
    let soul_path = identity_dir.join("SOUL.md");

    // 已存在 → 不動。包含 empty file:user 可能故意清空想自己重寫。
    if soul_path.exists() {
        tracing::debug!(
            path = %soul_path.display(),
            "SOUL.md already exists at vault, leaving untouched"
        );
        return Ok(());
    }

    std::fs::create_dir_all(&identity_dir).map_err(|e| {
        format!(
            "create identity dir {}: {e}",
            identity_dir.display()
        )
    })?;

    let canonical = fetch_canonical_soul_from(world_tree_url);
    std::fs::write(&soul_path, &canonical).map_err(|e| {
        format!("write SOUL.md to {}: {e}", soul_path.display())
    })?;

    tracing::info!(
        path = %soul_path.display(),
        chars = canonical.chars().count(),
        "wrote canonical SOUL.md to vault (first-launch distribution)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// 啟動一個只回應一次的 mock HTTP server。
    ///
    /// Returns (url, join_handle)。Test 結束時 thread 自然退出(handler 已 return)。
    fn spawn_mock_server(status: u16, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 0");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{}/mori.md", addr);
        thread::spawn(move || {
            // accept 一次就退出
            if let Ok((mut stream, _)) = listener.accept() {
                // 讀完 request(只讀到 \r\n\r\n)
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let status_line = match status {
                    200 => "200 OK",
                    404 => "404 Not Found",
                    500 => "500 Internal Server Error",
                    _ => "200 OK",
                };
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_line,
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        url
    }

    /// 啟動一個故意不回應的 mock server(模擬 timeout)。
    /// accept 完不寫任何 byte → client 等到 5s timeout。
    /// 為了測試別跑太久,test 自己用更短 timeout 走 fetch_canonical_soul_from。
    fn spawn_silent_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 0");
        let addr = listener.local_addr().expect("local_addr");
        let url = format!("http://{}/mori.md", addr);
        thread::spawn(move || {
            // hold connection 不回應
            if let Ok((stream, _)) = listener.accept() {
                std::thread::sleep(std::time::Duration::from_secs(30));
                drop(stream);
            }
        });
        url
    }

    #[test]
    fn fetch_canonical_soul_uses_bundle_when_offline() {
        // URL 指向 127.0.0.1:1(reserved port,connection refused 立刻 fail)
        let got = fetch_canonical_soul_from("http://127.0.0.1:1/no-such");
        // 連不到 → fallback bundle。bundle 帶 "Mori" 或「森」字。
        assert!(
            got.contains("Mori") || got.contains("森"),
            "expected bundled SOUL fallback, got: {}",
            &got[..got.len().min(120)]
        );
        // 跟 bundled 一樣(完整 fallback,不是部分內容)
        assert_eq!(got, BUNDLED_SOUL.to_string());
    }

    #[test]
    fn fetch_canonical_soul_uses_world_tree_when_ok() {
        let body = "---\nname: Mori\n---\n\n# Mori\n\n## 基本資料\n\nfoo\n\n\
                    ## 她的 SOUL 摘錄(完整版在 private repo)\n\n\
                    ```\n名字:Mori(森)\n類型:精靈\n```\n\n\
                    ## 寫給未來接手這個檔案的 AI\n\nbar\n";
        // body 必須 'static — 用 leak 簡化
        let leaked: &'static str = Box::leak(body.to_string().into_boxed_str());
        let url = spawn_mock_server(200, leaked);

        let got = fetch_canonical_soul_from(&url);
        // 應該抽到 SOUL 摘錄 section 內容,不含 header / 下一段
        assert!(
            got.contains("名字:Mori(森)"),
            "expected SOUL excerpt content, got: {got}"
        );
        assert!(
            !got.contains("寫給未來接手"),
            "should not include next section, got: {got}"
        );
        assert!(
            !got.contains("## 她的 SOUL"),
            "should not include the header line itself, got: {got}"
        );
    }

    #[test]
    fn fetch_canonical_soul_uses_bundle_when_section_missing() {
        let body = "---\nname: Mori\n---\n\n# Mori\n\n## 其他 section\n\nfoo\n";
        let leaked: &'static str = Box::leak(body.to_string().into_boxed_str());
        let url = spawn_mock_server(200, leaked);

        let got = fetch_canonical_soul_from(&url);
        assert_eq!(got, BUNDLED_SOUL.to_string());
    }

    #[test]
    fn fetch_canonical_soul_uses_bundle_when_non_2xx() {
        let url = spawn_mock_server(404, "Not Found");
        let got = fetch_canonical_soul_from(&url);
        assert_eq!(got, BUNDLED_SOUL.to_string());
    }

    #[test]
    fn ensure_soul_at_vault_writes_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let vault_root = dir.path();

        // 沒 SOUL.md → 應該寫入
        ensure_soul_at_vault_with_url(vault_root, "mori", "http://127.0.0.1:1/no-such")
            .expect("ensure should succeed");

        let soul_path = vault_root.join("mori").join("identity").join("SOUL.md");
        assert!(soul_path.exists(), "SOUL.md should have been written");
        let content = std::fs::read_to_string(&soul_path).unwrap();
        assert!(!content.trim().is_empty(), "SOUL.md should not be empty");
        // 走 offline path → bundle
        assert_eq!(content, BUNDLED_SOUL.to_string());
    }

    #[test]
    fn ensure_soul_at_vault_preserves_existing() {
        let dir = tempfile::tempdir().unwrap();
        let vault_root = dir.path();
        let identity_dir = vault_root.join("mori").join("identity");
        std::fs::create_dir_all(&identity_dir).unwrap();
        let custom = "Custom soul written by user\n";
        std::fs::write(identity_dir.join("SOUL.md"), custom).unwrap();

        // 已存在 → 不該覆蓋
        ensure_soul_at_vault_with_url(vault_root, "mori", "http://127.0.0.1:1/no-such")
            .expect("ensure should succeed even when existing");

        let content = std::fs::read_to_string(identity_dir.join("SOUL.md")).unwrap();
        assert_eq!(
            content, custom,
            "existing SOUL.md should not be overwritten"
        );
    }

    #[test]
    fn ensure_soul_at_vault_preserves_empty_existing() {
        // edge case:user 故意清空 SOUL.md 想自己重寫 → 不該被 canonical 覆蓋回去
        let dir = tempfile::tempdir().unwrap();
        let vault_root = dir.path();
        let identity_dir = vault_root.join("mori").join("identity");
        std::fs::create_dir_all(&identity_dir).unwrap();
        std::fs::write(identity_dir.join("SOUL.md"), "").unwrap();

        ensure_soul_at_vault_with_url(vault_root, "mori", "http://127.0.0.1:1/no-such")
            .expect("ensure should succeed");

        let content = std::fs::read_to_string(identity_dir.join("SOUL.md")).unwrap();
        assert_eq!(content, "", "empty existing SOUL.md should not be overwritten");
    }

    #[test]
    fn ensure_soul_at_vault_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let vault_root = dir.path();
        // vault_root/aoi/identity/ 不存在 — ensure 應該 mkdir -p
        assert!(!vault_root.join("aoi").exists());

        ensure_soul_at_vault_with_url(vault_root, "aoi", "http://127.0.0.1:1/no-such")
            .expect("ensure should create parent dirs");

        let soul_path = vault_root.join("aoi").join("identity").join("SOUL.md");
        assert!(soul_path.exists(), "SOUL.md should exist after mkdir -p");
        assert!(vault_root.join("aoi").join("identity").is_dir());
    }

    #[test]
    fn bundled_soul_is_not_empty() {
        // sanity:include_str! 真的塞到了 — 不要 ship empty bundle
        assert!(!BUNDLED_SOUL.trim().is_empty());
        // 應該有「Mori」或「森」字 — 不要 ship 錯 file
        assert!(
            BUNDLED_SOUL.contains("Mori") || BUNDLED_SOUL.contains("森"),
            "bundled SOUL should mention Mori / 森"
        );
    }

    #[test]
    fn extract_soul_section_finds_header_with_suffix() {
        let md = "## 她的 SOUL 摘錄(完整版在 private repo)\n\nbody line 1\nbody line 2\n\n## next section\n";
        let got = extract_soul_section(md).expect("should find section");
        assert!(got.contains("body line 1"));
        assert!(got.contains("body line 2"));
        assert!(!got.contains("next section"));
    }

    #[test]
    fn extract_soul_section_returns_none_when_absent() {
        let md = "## 別的 section\n\nbody\n";
        assert!(extract_soul_section(md).is_none());
    }

    #[test]
    fn extract_soul_section_returns_none_when_empty_body() {
        // header 後直接是下一個 ## — body 空
        let md = "## 她的 SOUL 摘錄\n\n## next\n";
        assert!(extract_soul_section(md).is_none());
    }

    #[test]
    #[ignore = "needs network — manual smoke test against real world-tree URL"]
    fn smoke_real_world_tree() {
        // Run with: cargo test -p mori-tauri --lib smoke_real_world_tree -- --ignored
        let got = fetch_canonical_soul_from(WORLD_TREE_SOUL_URL);
        assert!(!got.trim().is_empty());
        // 用 silent_server 測 timeout 路徑(會跑滿 5s):
        let _silent = spawn_silent_server();
        // (不放 assert,只壓 spawn_silent_server 不被 dead_code 警告)
    }
}
