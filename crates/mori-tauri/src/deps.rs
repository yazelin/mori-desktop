//! Phase 5O: Optional dependencies registry。
//!
//! mori 為了某些 feature 需要外部工具 / 模型,使用者裝沒裝是條件性的。
//! 這個 module 集中定義「我們關心哪些 dep / 怎麼檢測 / 怎麼裝」,UI 顯示
//! status table + install 按鈕。
//!
//! 安裝策略:
//! - **無需 sudo** 的(pip --user / curl install.sh / wget 下載到 home):直接
//!   subprocess 跑,捕捉 stdout/stderr 給 UI
//! - **需要 sudo** 的(apt install):回「複製指令給 user 自己在 terminal 跑」,
//!   不嘗試代執行(密碼提示在背景 spawn 出來會卡住)
//!
//! 安全:install command 跟 check command 都是 hardcoded、不接 LLM / user input,
//! 沒有 shell injection 風險。

use anyhow::Result;
use serde::Serialize;
use std::process::Command;

/// 一個可選依賴的描述。
#[derive(Debug, Clone, Serialize)]
pub struct DepSpec {
    /// 機器可讀 id(也用於 UI 按鈕 id)
    pub id: &'static str,
    /// 顯示名稱
    pub name: &'static str,
    /// 顯示用簡介
    pub description: &'static str,
    /// 解鎖什麼 feature?(顯示給 user 看)
    pub unlocks: &'static str,
    /// Approximate 下載 / 安裝大小(顯示用)
    pub size_hint: Option<&'static str>,
    /// 是否需要 sudo(影響「直接 install」還是「給 user 指令」UI 模式)
    pub needs_sudo: bool,
    /// 檢測指令(只回 0=有 / 非 0=沒有,stdout 拿來顯示版本資訊)
    pub check: CheckSpec,
    /// 安裝指令(若 needs_sudo,只給 user 看不執行)
    pub install: InstallSpec,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum CheckSpec {
    /// `which <bin>` 找 binary
    Which { bin: &'static str },
    /// 檔案存在
    File { path_template: &'static str },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum InstallSpec {
    /// 單一 shell command(無 sudo)
    Run {
        cmd: &'static str,
        args: &'static [&'static str],
    },
    /// 用 sh -c 包(裡面含 pipe / redirect / curl | sh 等需要 shell 的)
    Shell { script: &'static str },
    /// 給 user 在 terminal 自己跑(needs_sudo / 多步)
    Manual { commands: &'static [&'static str] },
}

/// mori-desktop 在意的所有 optional deps。
pub fn registry() -> Vec<DepSpec> {
    vec![
        DepSpec {
            id: "yt-dlp",
            name: "yt-dlp",
            description: "YouTube / 影音平台抓字幕、metadata 用 CLI",
            unlocks: "youtube_transcript skill(待 3B-2),Mori 可以幫你摘要影片內容",
            size_hint: Some("~5MB Python script + deps"),
            needs_sudo: false,
            check: CheckSpec::Which { bin: "yt-dlp" },
            install: InstallSpec::Run {
                cmd: "pip",
                args: &["install", "--user", "--upgrade", "yt-dlp"],
            },
        },
        DepSpec {
            id: "ydotool",
            name: "ydotool",
            description: "Wayland 下模擬鍵盤輸入(Ctrl+V 貼回游標)",
            unlocks: "Mori 把 LLM 處理結果貼到當前游標位置(語音輸入 / 反白改寫)",
            size_hint: None,
            needs_sudo: true,
            check: CheckSpec::Which { bin: "ydotool" },
            install: InstallSpec::Manual {
                commands: &[
                    "sudo apt install ydotool",
                    "# 加入 input group(才有 /dev/uinput 權限):",
                    "sudo usermod -aG input $USER",
                    "# 啟動 ydotoold daemon(GNOME 起 user service):",
                    "systemctl --user enable --now ydotoold",
                    "# 重開機讓 group 生效",
                ],
            },
        },
        DepSpec {
            id: "xdotool",
            name: "xdotool",
            description: "抓活躍視窗 / process name(XWayland 也吃)",
            unlocks: "Mori 知道你當下在哪個 app(寫 prompt context 用)",
            size_hint: None,
            needs_sudo: true,
            check: CheckSpec::Which { bin: "xdotool" },
            install: InstallSpec::Manual {
                commands: &["sudo apt install xdotool"],
            },
        },
        DepSpec {
            id: "xclip",
            name: "xclip",
            description: "X11 PRIMARY selection / clipboard 讀取",
            unlocks: "反白文字直接被 Mori 看到(不用 Ctrl+C)",
            size_hint: None,
            needs_sudo: true,
            check: CheckSpec::Which { bin: "xclip" },
            install: InstallSpec::Manual {
                commands: &["sudo apt install xclip"],
            },
        },
        DepSpec {
            id: "whisper-model",
            name: "whisper-local model (ggml-small.bin)",
            description: "離線 STT 模型(466MB 中文版)",
            unlocks: "stt_provider=whisper-local 走 100% 離線 STT,不用 Groq 雲端",
            size_hint: Some("~466MB"),
            needs_sudo: false,
            check: CheckSpec::File {
                path_template: "$HOME/.mori/models/ggml-small.bin",
            },
            install: InstallSpec::Shell {
                script: "mkdir -p \"$HOME/.mori/models\" && \
                         wget -O \"$HOME/.mori/models/ggml-small.bin\" \
                         https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
            },
        },
        DepSpec {
            id: "ollama",
            name: "ollama",
            description: "本機 LLM runtime(qwen3:8b / llama3 / 等)",
            unlocks: "provider=ollama 走 100% 離線 LLM",
            size_hint: Some("~600MB binary,每個 model 額外 4~30GB"),
            needs_sudo: false,
            check: CheckSpec::Which { bin: "ollama" },
            install: InstallSpec::Shell {
                script: "curl -fsSL https://ollama.com/install.sh | sh",
            },
        },
    ]
}

/// 偵測單一 dep 狀態。
#[derive(Debug, Clone, Serialize)]
pub struct DepStatus {
    pub id: &'static str,
    pub installed: bool,
    /// 若 installed,顯示路徑 / 版本資訊(`which` 路徑 / 檔案大小等)
    pub detail: Option<String>,
}

pub fn check_dep(spec: &DepSpec) -> DepStatus {
    match &spec.check {
        CheckSpec::Which { bin } => match Command::new("which").arg(bin).output() {
            Ok(out) if out.status.success() => DepStatus {
                id: spec.id,
                installed: true,
                detail: Some(String::from_utf8_lossy(&out.stdout).trim().to_string()),
            },
            _ => DepStatus { id: spec.id, installed: false, detail: None },
        },
        CheckSpec::File { path_template } => {
            let path = expand_home(path_template);
            match std::fs::metadata(&path) {
                Ok(meta) => DepStatus {
                    id: spec.id,
                    installed: true,
                    detail: Some(format!("{path} ({:.1} MB)", meta.len() as f64 / 1024.0 / 1024.0)),
                },
                Err(_) => DepStatus {
                    id: spec.id,
                    installed: false,
                    detail: Some(format!("not at {path}")),
                },
            }
        }
    }
}

/// 跑 install command,回傳 (stdout+stderr 合併、success flag)。
/// 只處理 Run / Shell — Manual 不在這條路,UI 直接顯示指令給 user。
pub fn run_install(spec: &DepSpec) -> Result<InstallResult> {
    let (cmd, args, shell_mode) = match &spec.install {
        InstallSpec::Run { cmd, args } => (cmd.to_string(), args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), false),
        InstallSpec::Shell { script } => (
            "sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
            true,
        ),
        InstallSpec::Manual { .. } => {
            anyhow::bail!("Manual install — UI should show commands to user, not call run_install");
        }
    };

    tracing::info!(dep = spec.id, cmd = %cmd, shell = shell_mode, "install start");
    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .map_err(|e| anyhow::anyhow!("spawn {cmd}: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{stdout}\n--- stderr ---\n{stderr}")
    };

    Ok(InstallResult {
        success: output.status.success(),
        exit_code: output.status.code(),
        output: combined,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub output: String,
}

fn expand_home(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".into());
    p.replace("$HOME", &home)
}
