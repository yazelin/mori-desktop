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
    /// 跑指令 + 看 stdout 含某字串(例:`ollama list` 看有沒 `qwen3:8b`)
    CommandStdoutContains {
        cmd: &'static str,
        args: &'static [&'static str],
        needle: &'static str,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum InstallSpec {
    /// 用 sh -c 包(可含 pipe / redirect / curl | sh 等需要 shell 的)
    Shell { script: &'static str },
    /// 給 user 在 terminal 自己跑(needs_sudo / 多步)
    Manual { commands: &'static [&'static str] },
}

/// mori-desktop 在意的所有 optional deps。
pub fn registry() -> Vec<DepSpec> {
    vec![
        DepSpec {
            id: "uv",
            name: "uv",
            description: "Astral 出的 Python pkg / tool manager(static binary,取代 pip / pipx,不依賴系統 python3-venv)",
            unlocks: "yt-dlp 等 Python CLI 的安裝前置;同時是 mori 之後跑 Python skill 的標準 runtime",
            size_hint: Some("~30MB"),
            needs_sudo: false,
            check: CheckSpec::File {
                path_template: "$HOME/.local/bin/uv",
            },
            install: InstallSpec::Shell {
                script: "curl -LsSf https://astral.sh/uv/install.sh | sh",
            },
        },
        DepSpec {
            id: "yt-dlp",
            name: "yt-dlp",
            description: "YouTube / 影音平台抓字幕、metadata 用 CLI(由 uv 管 isolated venv)",
            unlocks: "youtube_transcript skill(待 3B-2),Mori 可以幫你摘要影片內容。需先裝 uv。",
            size_hint: Some("~5MB Python script + deps"),
            needs_sudo: false,
            check: CheckSpec::File {
                path_template: "$HOME/.local/bin/yt-dlp",
            },
            install: InstallSpec::Shell {
                // 一鍵 bootstrap:沒 uv 先 curl install.sh,再用 uv 裝 yt-dlp
                script: "if [ ! -x \"$HOME/.local/bin/uv\" ]; then curl -LsSf https://astral.sh/uv/install.sh | sh; fi && \"$HOME/.local/bin/uv\" tool install --upgrade yt-dlp",
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
            description: "抓活躍視窗 / process name。GNOME Wayland 下對 XWayland app(Chrome / VSCode / Electron 多數)能讀;純 Wayland-only app(部分 GTK4)拿不到。",
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
            description: "X11 PRIMARY selection / clipboard 讀寫。GNOME Wayland 下 Mutter 會把 PRIMARY 同步到 XWayland,xclip 透過 X server 仍能讀到反白 — mori 5F 之後 selection / clipboard 的 production path 就走它(避開 wl-paste 觸發的 portal 對話框)。",
            unlocks: "反白文字直接被 Mori 看到(不用 Ctrl+C);Wayland 下等同必裝",
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
            description: "離線 STT 模型(466MB 中文版)。同檔 Linux / Windows 通用。",
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
            id: "whisper-server",
            name: "whisper-server (whisper.cpp 引擎)",
            description: "本機 STT 推理引擎 — whisper.cpp 官方 pre-built HTTP server。\
                          Mori 啟動時 lazy spawn,送 WAV 到 localhost。\
                          Linux 自動下載 + 解壓 + 放到 ~/.mori/bin/;Windows 給手動步驟。",
            unlocks: "stt_provider=whisper-local 能真的 spawn 起來跑(沒這個就只有 .bin 沒人讀)",
            size_hint: Some("~5-10MB(僅 CPU 版,GPU 版可手動換)"),
            needs_sudo: false,
            check: CheckSpec::File {
                path_template: "$HOME/.mori/bin/whisper-server",
            },
            install: InstallSpec::Shell {
                // 從 whisper.cpp GitHub release 抓 Linux x86_64 build,解壓出
                // whisper-server。版本固定 pin 一個近期 stable;升級換 tag 即可。
                // whisper.cpp 官方在 release 提供 ubuntu-22-x64.zip / ubuntu-22-x64.tar.xz,
                // 內含 whisper-server + 共享 lib。
                script: "mkdir -p \"$HOME/.mori/bin\" && \
                         cd /tmp && \
                         curl -L -o whisper-cpp-bin.zip \
                           https://github.com/ggml-org/whisper.cpp/releases/latest/download/whisper-bin-x64.zip && \
                         (unzip -o whisper-cpp-bin.zip -d whisper-cpp-bin || tar -xJf whisper-cpp-bin.zip -C whisper-cpp-bin) && \
                         find whisper-cpp-bin -name 'whisper-server' -type f -executable -exec cp {} \"$HOME/.mori/bin/whisper-server\" \\; && \
                         chmod +x \"$HOME/.mori/bin/whisper-server\" && \
                         rm -rf /tmp/whisper-cpp-bin /tmp/whisper-cpp-bin.zip && \
                         echo \"whisper-server installed to $HOME/.mori/bin/whisper-server\"",
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
        DepSpec {
            id: "ollama-qwen3-8b",
            name: "qwen3:8b(Ollama 模型本體)",
            description: "Mori 預設離線 LLM 模型,支援 tool calling(Agent 模式必需)。\
                          需要 ollama binary 已裝。",
            unlocks: "ollama 真的能跑 LLM(只裝 ollama binary 沒模型也叫不起來)",
            size_hint: Some("~5GB"),
            needs_sudo: false,
            check: CheckSpec::CommandStdoutContains {
                cmd: "ollama",
                args: &["list"],
                needle: "qwen3:8b",
            },
            install: InstallSpec::Shell {
                script: "ollama pull qwen3:8b",
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
        CheckSpec::CommandStdoutContains { cmd, args, needle } => {
            match Command::new(cmd).args(args.iter()).output() {
                Ok(out) if out.status.success() => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    if stdout.contains(needle) {
                        // 取含 needle 的那一行當 detail(像 `ollama list` 顯示 size)
                        let line = stdout.lines().find(|l| l.contains(needle)).unwrap_or(needle);
                        DepStatus {
                            id: spec.id,
                            installed: true,
                            detail: Some(line.trim().to_string()),
                        }
                    } else {
                        DepStatus {
                            id: spec.id,
                            installed: false,
                            detail: Some(format!("`{cmd}` 沒列出 `{needle}`")),
                        }
                    }
                }
                Ok(out) => DepStatus {
                    id: spec.id,
                    installed: false,
                    detail: Some(format!(
                        "`{cmd}` 失敗:{}",
                        String::from_utf8_lossy(&out.stderr).trim()
                    )),
                },
                Err(_) => DepStatus {
                    id: spec.id,
                    installed: false,
                    detail: Some(format!("`{cmd}` 不在 PATH")),
                },
            }
        }
    }
}

/// 跑 install command,回傳 (stdout+stderr 合併、success flag)。
/// 只處理 Run / Shell — Manual 不在這條路,UI 直接顯示指令給 user。
pub fn run_install(spec: &DepSpec) -> Result<InstallResult> {
    let (cmd, args) = match &spec.install {
        InstallSpec::Shell { script } => (
            "sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
        ),
        InstallSpec::Manual { .. } => {
            anyhow::bail!("Manual install — UI should show commands to user, not call run_install");
        }
    };

    tracing::info!(dep = spec.id, cmd = %cmd, "install start");
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
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "~".into());
    p.replace("$HOME", &home)
}
