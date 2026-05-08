//! `~/.mori/runtime.json` 共用 schema。
//!
//! Mori-tauri 啟動時會起一個 localhost HTTP server 暴露 skill endpoints,
//! 把 port 跟 auth token 寫到 runtime.json,讓 mori CLI(以及外部 AI agent
//! 透過 Bash tool 呼叫的 mori CLI)能讀到、連回主程式 dispatch skill。
//!
//! 設計考量:
//! - **Random port**:bind 0 讓 OS 給空閒 port,避免衝突。每次啟動 port 不同。
//! - **Auth token**:即便 bind 127.0.0.1 也要,避免同機其他 user / process 亂呼叫。
//!   每次啟動產生一個 32-char 隨機 token。
//! - **Atomic write**:寫檔走 tempfile + rename,避免 partial read。
//! - **Stale 偵測**:CLI 連不上時應該回友善訊息「Mori 主程式沒在跑」,不是
//!   crash。

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInfo {
    /// Mori 主程式 listen 的 localhost port。一律 127.0.0.1。
    pub port: u16,
    /// CLI 呼叫時要帶的 bearer token(`Authorization: Bearer <token>`)
    pub auth_token: String,
    /// 啟動時的 PID,給 stale 偵測用(file 還在但 process 不在 → 視為 stale)
    pub pid: u32,
    /// runtime.json 寫入時間(epoch seconds),除錯用
    pub started_at_epoch: u64,
}

impl RuntimeInfo {
    /// `~/.mori/runtime.json`
    pub fn default_path() -> Option<PathBuf> {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()
            .map(|h| PathBuf::from(h).join(".mori").join("runtime.json"))
    }

    /// 寫到磁碟(atomic via tempfile + rename)
    pub fn write_to_default(&self) -> Result<PathBuf> {
        let path = Self::default_path()
            .context("could not determine ~/.mori/runtime.json path")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    /// 從預設路徑讀。CLI 端用。
    pub fn read_from_default() -> Result<Self> {
        let path = Self::default_path()
            .context("could not determine ~/.mori/runtime.json path")?;
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "read {} (Mori 主程式沒在跑?啟動 `npm run tauri dev` 試試)",
                path.display()
            )
        })?;
        let info: Self = serde_json::from_str(&text)
            .with_context(|| format!("parse {}", path.display()))?;
        Ok(info)
    }

    /// `Authorization: Bearer <token>`
    pub fn bearer(&self) -> String {
        format!("Bearer {}", self.auth_token)
    }

    /// `http://127.0.0.1:<port>`
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// 產生 32 字元的隨機 token。簡單版 — 用 SystemTime + ProcessId hash,夠擋
/// 同機其他普通使用者的探測。我們不抗 active attacker,localhost 都信任。
pub fn generate_auth_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    // 把 nanos 和 pid 不同 bit 組合洗 4 次,輸出 hex 32 字
    let mut state: u64 = nanos.wrapping_mul(6364136223846793005).wrapping_add(pid);
    let mut out = String::with_capacity(32);
    for _ in 0..4 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        out.push_str(&format!("{:016x}", state));
    }
    out.truncate(32);
    out
}
