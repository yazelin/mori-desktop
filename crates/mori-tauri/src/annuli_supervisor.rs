//! 啟動時看 annuli config:若 enabled + localhost endpoint,先 health-check;
//! 連不到就 spawn `python main.py admin --port N` from venv。
//!
//! `kill_on_drop(true)` 確保 app 退出時 annuli 子 process 跟著掛 — 不留 zombie。
//! 非 localhost endpoint(例正式機 https://ching-tech.ddns.net/jinn)什麼都不做,
//! 那個 user 自己管。

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};

use crate::annuli_config::AnnuliConfig;

pub struct AnnuliSupervisor {
    /// 子 process。`kill_on_drop(true)`,Arc<AppState> drop 時跟著掛。
    #[allow(dead_code)]
    child: Option<Child>,
    pub info: SupervisorInfo,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SupervisorInfo {
    /// 狀態字串(給 status command / 前端顯示):
    /// - `"disabled"`:annuli.enabled=false
    /// - `"remote"`:endpoint 非 localhost,我們不戳
    /// - `"already-running"`:health-check 通過,有人在跑
    /// - `"spawned"`:我們起的且 ready
    /// - `"spawned-not-ready"`:起了但 15s 內沒等到 health
    /// - `"failed"`:venv / main.py 缺、spawn 失敗等
    pub state: &'static str,
    pub annuli_root: Option<String>,
    pub python: Option<String>,
    pub port: Option<u16>,
    pub reason: String,
}

impl AnnuliSupervisor {
    pub async fn maybe_spawn(cfg: &AnnuliConfig) -> Self {
        if !cfg.enabled {
            return Self::noop("disabled", "annuli.enabled=false");
        }

        let (host, port) = match parse_endpoint(&cfg.endpoint) {
            Some(hp) => hp,
            None => {
                return Self::noop(
                    "failed",
                    &format!("cannot parse endpoint: {}", cfg.endpoint),
                );
            }
        };

        if !is_localhost(&host) {
            return Self::noop(
                "remote",
                &format!("endpoint host {host} is not localhost — assuming user-managed remote"),
            );
        }

        if check_alive(&cfg.endpoint).await {
            return Self::noop(
                "already-running",
                &format!("{} reachable — not spawning", cfg.endpoint),
            );
        }

        let annuli_root = annuli_root_dir();
        let python = venv_python(&annuli_root);
        let main_py = annuli_root.join("main.py");

        if !python.exists() {
            return Self::noop(
                "failed",
                &format!("venv python not found: {}", python.display()),
            );
        }
        if !main_py.exists() {
            return Self::noop(
                "failed",
                &format!("annuli main.py not found: {}", main_py.display()),
            );
        }

        let mut cmd = Command::new(&python);
        cmd.current_dir(&annuli_root);
        cmd.args(["main.py", "admin", "--port", &port.to_string()]);

        if !cfg.soul_token.is_empty() {
            cmd.env("ANNULI_SOUL_TOKEN", &cfg.soul_token);
        }
        if let Some(ba) = &cfg.basic_auth {
            cmd.env("ANNULI_ADMIN_USER", &ba.user);
            cmd.env("ANNULI_ADMIN_PASS", &ba.pass);
        } else {
            // 沒設就清空,避免 inherit parent env 不小心帶進去
            cmd.env("ANNULI_ADMIN_USER", "");
            cmd.env("ANNULI_ADMIN_PASS", "");
        }

        cmd.stdin(std::process::Stdio::null());
        // stdout / stderr inherit → 進 mori-desktop console / dev log
        cmd.kill_on_drop(true);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Self::noop("failed", &format!("spawn: {e}"));
            }
        };
        tracing::info!(
            annuli_root = %annuli_root.display(),
            python = %python.display(),
            port,
            "spawned annuli admin server"
        );

        // 等 server up — 最多 15s
        for i in 0..30 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if check_alive(&cfg.endpoint).await {
                let after_ms = (i + 1) * 500;
                tracing::info!(after_ms, "annuli admin server ready");
                return Self {
                    child: Some(child),
                    info: SupervisorInfo {
                        state: "spawned",
                        annuli_root: Some(annuli_root.display().to_string()),
                        python: Some(python.display().to_string()),
                        port: Some(port),
                        reason: format!("spawned + healthy after {after_ms}ms"),
                    },
                };
            }
        }
        tracing::warn!("annuli child spawned but didn't become reachable in 15s");
        Self {
            child: Some(child),
            info: SupervisorInfo {
                state: "spawned-not-ready",
                annuli_root: Some(annuli_root.display().to_string()),
                python: Some(python.display().to_string()),
                port: Some(port),
                reason: "spawned, but health-check never succeeded in 15s".into(),
            },
        }
    }

    fn noop(state: &'static str, reason: &str) -> Self {
        tracing::info!(state, reason, "annuli supervisor: not spawning");
        Self {
            child: None,
            info: SupervisorInfo {
                state,
                annuli_root: None,
                python: None,
                port: None,
                reason: reason.into(),
            },
        }
    }
}

fn annuli_root_dir() -> PathBuf {
    if let Ok(p) = std::env::var("MORI_ANNULI_ROOT") {
        return PathBuf::from(p);
    }
    let home_var = if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    };
    if let Ok(home) = std::env::var(home_var) {
        return PathBuf::from(home).join("mori-universe").join("annuli");
    }
    PathBuf::from("annuli")
}

fn venv_python(annuli_root: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        annuli_root.join(".venv").join("Scripts").join("python.exe")
    } else {
        annuli_root.join(".venv").join("bin").join("python")
    }
}

fn parse_endpoint(endpoint: &str) -> Option<(String, u16)> {
    let s = endpoint.trim_end_matches('/');
    let (scheme, rest) = s.split_once("://")?;
    let default_port: u16 = if scheme == "https" { 443 } else { 80 };
    let host_and_port = rest.split('/').next().unwrap_or(rest);
    if let Some((host, port_s)) = host_and_port.rsplit_once(':') {
        let port = port_s.parse::<u16>().ok()?;
        Some((host.to_string(), port))
    } else {
        Some((host_and_port.to_string(), default_port))
    }
}

fn is_localhost(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
}

async fn check_alive(endpoint: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
    else {
        return false;
    };
    // annuli admin / 是 web admin UI;401 = basic auth challenged → server up
    match client.get(endpoint).send().await {
        Ok(r) => r.status().is_success() || r.status().as_u16() == 401,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_localhost_with_port() {
        assert_eq!(
            parse_endpoint("http://localhost:5000"),
            Some(("localhost".into(), 5000)),
        );
    }

    #[test]
    fn parses_path_suffix() {
        assert_eq!(
            parse_endpoint("http://localhost:5000/admin"),
            Some(("localhost".into(), 5000)),
        );
    }

    #[test]
    fn parses_https_default_port() {
        assert_eq!(
            parse_endpoint("https://example.com"),
            Some(("example.com".into(), 443)),
        );
    }

    #[test]
    fn parses_with_trailing_slash() {
        assert_eq!(
            parse_endpoint("http://127.0.0.1:5099/"),
            Some(("127.0.0.1".into(), 5099)),
        );
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(parse_endpoint("not-a-url"), None);
    }

    #[test]
    fn localhost_detection() {
        assert!(is_localhost("localhost"));
        assert!(is_localhost("127.0.0.1"));
        assert!(is_localhost("::1"));
        assert!(!is_localhost("ching-tech.ddns.net"));
    }
}
