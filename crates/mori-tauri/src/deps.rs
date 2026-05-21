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
    /// 此 dep 適用的平台。`deps_list()` IPC handler 用 `std::env::consts::OS`
    /// 過濾,只把當前平台適用的條目送給前端。Linux-only 工具(ydotool /
    /// xdotool / xclip)在 Windows / macOS 不會顯示。
    /// 值對齊 Rust `std::env::consts::OS`:"linux" / "windows" / "macos"。
    pub platforms: &'static [&'static str],
    /// 平台 caveat — 此 dep 在當前平台「能用但有限制」時的警告字串。
    /// 例:whisper-server 在 Windows 一鍵下載還沒接(只有 Linux Shell 腳本),
    /// 設「請手動下載 whisper-server.exe 放到 ~/.mori/bin/」。
    /// `None` 表示無 caveat。
    pub install_caveat: Option<&'static str>,
    /// 檢測指令(只回 0=有 / 非 0=沒有,stdout 拿來顯示版本資訊)
    pub check: CheckSpec,
    /// 平台特定 check override。mirror `install_overrides` — Windows 的 binary
    /// 通常多 `.exe` / venv 走 `Scripts\python.exe` 而非 `bin/python`,需要單獨
    /// 寫一份 CheckSpec 才能正確偵測「裝過了沒」。
    pub check_overrides: &'static [(&'static str, CheckSpec)],
    /// 安裝指令(若 needs_sudo,只給 user 看不執行)
    pub install: InstallSpec,
    /// 平台特定 install override。lookup 順序:此 list 內找符合 OS 的 →
    /// 都沒有 → fallback `install`。給「跨平台支援但 install 方法各異」的
    /// dep 用(像 whisper-server:Linux 走 Shell curl,Windows 走 Manual)。
    pub install_overrides: &'static [(&'static str, InstallSpec)],
}

impl DepSpec {
    /// 取得當前 OS 適用的 InstallSpec — 先看 install_overrides 有沒有
    /// 平台特定條目,沒有再退到 install。
    pub fn effective_install(&self) -> &InstallSpec {
        let os = std::env::consts::OS;
        for (platform, spec) in self.install_overrides {
            if *platform == os {
                return spec;
            }
        }
        &self.install
    }

    /// 取得當前 OS 適用的 CheckSpec — 跟 effective_install 同邏輯。
    pub fn effective_check(&self) -> &CheckSpec {
        let os = std::env::consts::OS;
        for (platform, spec) in self.check_overrides {
            if *platform == os {
                return spec;
            }
        }
        &self.check
    }

    /// 此 dep 是否適用當前 OS(用於 deps_list filter)。
    pub fn applies_to_current_os(&self) -> bool {
        let os = std::env::consts::OS;
        self.platforms.iter().any(|p| *p == os)
    }
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
    /// 用 sh -c 包(可含 pipe / redirect / curl | sh 等需要 shell 的)。
    /// 注意:Windows 沒 sh,要跨平台 install 走 `Download` variant。
    Shell { script: &'static str },
    /// 給 user 在 terminal 自己跑(needs_sudo / 多步)。
    /// UI 顯示指令清單給 user copy + 自己貼到 terminal 跑。
    Manual { commands: &'static [&'static str] },
    /// 跨平台原生下載 + 解壓(不依賴 sh / curl / unzip)。
    /// reqwest blocking 拉 archive → zip crate 解壓 → 把指定 member(或全部)
    /// 複製到 `$HOME/.mori/bin/` 等目標。Linux + Windows 都 work。
    Download {
        /// archive URL(目前只支援 .zip,後續可加 tar.xz 等)
        url: &'static str,
        /// 解壓目標資料夾(支援 `$HOME` 展開,跨平台同樣語法)
        dest_dir: &'static str,
        /// 從 archive 抽出哪些檔名(basename match,大小寫敏感)。
        /// 空 array → 全部抽。匹配的檔案(以及它們的相依 .dll / .so)
        /// 一起放進 dest_dir。
        extract_members: &'static [&'static str],
        /// 是否要對抽出來的 binary chmod +x(Unix only,Windows ignored)
        make_executable: bool,
    },
}

/// mori-desktop 在意的所有 optional deps。
///
/// 每個 spec 有 `platforms` 欄位 — `deps_list()` IPC 會用 `std::env::consts::OS`
/// 過濾,Linux-only 條目(ydotool / xdotool / xclip)在 Windows 不會顯示,
/// 跨平台條目(uv / whisper-model / ollama 等)在所有平台都顯示但裝法可能
/// 不同(走 `install_overrides`)。
pub fn registry() -> Vec<DepSpec> {
    vec![
        DepSpec {
            id: "uv",
            name: "uv",
            description: "Astral 出的 Python pkg / tool manager(static binary,取代 pip / pipx,不依賴系統 python3-venv)",
            unlocks: "yt-dlp 等 Python CLI 的安裝前置;同時是 mori 之後跑 Python skill 的標準 runtime",
            size_hint: Some("~30MB"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::File {
                path_template: "$HOME/.local/bin/uv",
            },
            check_overrides: &[
                ("windows", CheckSpec::File {
                    path_template: "$HOME/.local/bin/uv.exe",
                }),
            ],
            install: InstallSpec::Shell {
                script: "curl -LsSf https://astral.sh/uv/install.sh | sh",
            },
            install_overrides: &[
                ("windows", InstallSpec::Manual {
                    commands: &[
                        "# Windows 用 PowerShell 一鍵裝(Astral 官方):",
                        "powershell -c \"irm https://astral.sh/uv/install.ps1 | iex\"",
                    ],
                }),
            ],
        },
        DepSpec {
            id: "yt-dlp",
            name: "yt-dlp",
            description: "YouTube / 影音平台抓字幕、metadata 用 CLI(由 uv 管 isolated venv)",
            unlocks: "youtube_transcript skill(待 3B-2),Mori 可以幫你摘要影片內容。需先裝 uv。",
            size_hint: Some("~5MB Python script + deps"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::File {
                path_template: "$HOME/.local/bin/yt-dlp",
            },
            check_overrides: &[
                ("windows", CheckSpec::File {
                    path_template: "$HOME/.local/bin/yt-dlp.exe",
                }),
            ],
            install: InstallSpec::Shell {
                // 一鍵 bootstrap:沒 uv 先 curl install.sh,再用 uv 裝 yt-dlp
                script: "if [ ! -x \"$HOME/.local/bin/uv\" ]; then curl -LsSf https://astral.sh/uv/install.sh | sh; fi && \"$HOME/.local/bin/uv\" tool install --upgrade yt-dlp",
            },
            install_overrides: &[
                ("windows", InstallSpec::Manual {
                    commands: &[
                        "# Windows 在 PowerShell 跑(需先有 uv):",
                        "uv tool install --upgrade yt-dlp",
                    ],
                }),
            ],
        },
        DepSpec {
            id: "ydotool",
            name: "ydotool",
            description: "Wayland 下模擬鍵盤輸入(Ctrl+V 貼回游標)",
            unlocks: "Mori 把 LLM 處理結果貼到當前游標位置(語音輸入 / 反白改寫)",
            size_hint: None,
            needs_sudo: true,
            platforms: &["linux"],
            install_caveat: None,
            check: CheckSpec::Which { bin: "ydotool" },
            check_overrides: &[],
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
            install_overrides: &[],
        },
        DepSpec {
            id: "xdotool",
            name: "xdotool",
            description: "抓活躍視窗 / process name。GNOME Wayland 下對 XWayland app(Chrome / VSCode / Electron 多數)能讀;純 Wayland-only app(部分 GTK4)拿不到。",
            unlocks: "Mori 知道你當下在哪個 app(寫 prompt context 用)",
            size_hint: None,
            needs_sudo: true,
            platforms: &["linux"],
            install_caveat: None,
            check: CheckSpec::Which { bin: "xdotool" },
            check_overrides: &[],
            install: InstallSpec::Manual {
                commands: &["sudo apt install xdotool"],
            },
            install_overrides: &[],
        },
        DepSpec {
            id: "xclip",
            name: "xclip",
            description: "X11 PRIMARY selection / clipboard 讀寫。GNOME Wayland 下 Mutter 會把 PRIMARY 同步到 XWayland,xclip 透過 X server 仍能讀到反白 — mori 5F 之後 selection / clipboard 的 production path 就走它(避開 wl-paste 觸發的 portal 對話框)。",
            unlocks: "反白文字直接被 Mori 看到(不用 Ctrl+C);Wayland 下等同必裝",
            size_hint: None,
            needs_sudo: true,
            platforms: &["linux"],
            install_caveat: None,
            check: CheckSpec::Which { bin: "xclip" },
            check_overrides: &[],
            install: InstallSpec::Manual {
                commands: &["sudo apt install xclip"],
            },
            install_overrides: &[],
        },
        // 時之鳥(mori-time)桌面通知用 — Linux 走 notify-rust → libnotify → libdbus
        // session bus。沒裝 → K2 scheduler 觸發 fire() 會回 NotifyError::Notify("no
        // session bus" / "spawn error"),整個 reminder 提醒失敗。
        //
        // 注意:Tauri 2 + libayatana-appindicator-dev 的 build deps 通常會自動把
        // libdbus-1-3 拉進來,所以多數 dev 機其實「已經有」。仍然顯式列出來,讓
        // user 知道時之鳥需要這個 lib + 一鍵看狀態,fresh install / minimal container
        // 環境不至於默默失敗。
        DepSpec {
            id: "libdbus",
            name: "libdbus(時之鳥桌面通知)",
            description: "Linux 桌面通知(notify-rust → libnotify → dbus session bus)\
                          需要的共享 lib。Windows / macOS 走 native API,不需要。",
            unlocks: "mori-time「時之鳥」reminder 到時間真的彈通知 popup",
            size_hint: Some("~500KB"),
            needs_sudo: true,
            platforms: &["linux"],
            install_caveat: None,
            // ldconfig -p 列 cache 內所有 .so → grep libdbus-1.so。
            // 比 pkg-config dbus-1 / dpkg -l 跨發行版更穩(pkg-config 看的是 -dev pkg,
            // runtime 不需要;dpkg 只 Debian 系)。
            check: CheckSpec::CommandStdoutContains {
                cmd: "sh",
                args: &["-c", "ldconfig -p 2>/dev/null | grep -i 'libdbus-1.so'"],
                needle: "libdbus-1.so",
            },
            check_overrides: &[],
            // apt / dnf / pacman 都需要 sudo,給 user 自己跑指令(Manual)。
            // 預設給 Debian/Ubuntu 指令,其他發行版在 commands 內也列出來給 user 看。
            install: InstallSpec::Manual {
                commands: &[
                    "# Ubuntu / Debian:",
                    "sudo apt install libdbus-1-3",
                    "# Fedora / RHEL:",
                    "# sudo dnf install dbus-libs",
                    "# Arch / Manjaro:",
                    "# sudo pacman -S dbus",
                ],
            },
            install_overrides: &[],
        },
        DepSpec {
            id: "whisper-model",
            name: "whisper-local model (ggml-small.bin)",
            description: "離線 STT 模型(466MB 中文版)。同檔 Linux / Windows / macOS 通用。",
            unlocks: "stt_provider=whisper-local 走 100% 離線 STT,不用 Groq 雲端",
            size_hint: Some("~466MB"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::File {
                path_template: "$HOME/.mori/models/ggml-small.bin",
            },
            check_overrides: &[],
            install: InstallSpec::Shell {
                script: "mkdir -p \"$HOME/.mori/models\" && \
                         wget -O \"$HOME/.mori/models/ggml-small.bin\" \
                         https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
            },
            install_overrides: &[
                ("windows", InstallSpec::Manual {
                    commands: &[
                        "# PowerShell:",
                        "mkdir -Force $env:USERPROFILE\\.mori\\models | Out-Null",
                        "curl.exe -L -o $env:USERPROFILE\\.mori\\models\\ggml-small.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
                    ],
                }),
            ],
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
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::File {
                path_template: "$HOME/.mori/bin/whisper-server",
            },
            check_overrides: &[
                ("windows", CheckSpec::File {
                    path_template: "$HOME/.mori/bin/whisper-server.exe",
                }),
            ],
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
            install_overrides: &[
                ("windows", InstallSpec::Download {
                    // 直接拉 latest release 的 CPU 版 zip。
                    // 想換 GPU 加速版本(CUDA / CLBlast),user 在 config 改
                    // server_binary 絕對路徑指向另一份 binary。
                    url: "https://github.com/ggml-org/whisper.cpp/releases/latest/download/whisper-bin-x64.zip",
                    dest_dir: "$HOME/.mori/bin",
                    // 空 list = 解壓 zip 裡所有檔案(whisper-server.exe + ggml.dll
                    // + whisper.dll 等同 zip 內的相依 lib 全要)。
                    extract_members: &[],
                    make_executable: false, // Windows .exe 不需要 +x
                }),
            ],
        },
        // Phase 3 polish B:三個 AI CLI 偵測 — claude-bash / gemini-bash / codex-bash
        // 這幾個 provider 需要對應 binary 在 PATH 才能跑。fresh install 沒裝
        // 時 agent profile 用了會炸 spawn,DepsTab 顯示 ✗ 提示去裝。
        //
        // 全部用 Manual install — 各家 CLI 官方安裝都很多步(npm / brew / installer),
        // 我們不冒險跑自動腳本。
        DepSpec {
            id: "claude-code-cli",
            name: "Claude Code CLI",
            description: "Anthropic 官方 AI coding CLI。Mori provider `claude-bash` / \
                          `claude-cli` 需要這個 binary。",
            unlocks: "provider=claude-bash / claude-cli 啟動 Claude Code 當 agent loop",
            size_hint: Some("~100MB(Node.js based)"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::Which { bin: "claude" },
            check_overrides: &[],
            install: InstallSpec::Manual {
                commands: &[
                    "# 官方安裝(需先有 Node.js 18+):",
                    "npm install -g @anthropic-ai/claude-code",
                    "# 安裝完跑 `claude login` 完成 OAuth 認證(用 Anthropic 帳號)",
                    "# 詳見 https://docs.anthropic.com/claude/docs/claude-code",
                ],
            },
            install_overrides: &[],
        },
        DepSpec {
            id: "gemini-cli",
            name: "Gemini CLI",
            description: "Google 官方 Gemini CLI。Mori provider `gemini-bash` / \
                          `gemini-cli` 需要這個 binary。\
                          \n\n注意:**只用 Gemini API**(`provider: gemini`)的話 \
                          **不需要**這個 CLI,直接設 API key 就能跑。",
            unlocks: "provider=gemini-bash / gemini-cli 啟動 Gemini CLI 當 agent loop",
            size_hint: Some("~50MB(Node.js based)"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::Which { bin: "gemini" },
            check_overrides: &[],
            install: InstallSpec::Manual {
                commands: &[
                    "# 官方安裝(需先有 Node.js 18+):",
                    "npm install -g @google/gemini-cli",
                    "# 安裝完跑 `gemini config set api_key <your-key>` 或 GEMINI_API_KEY env",
                    "# 詳見 https://github.com/google-gemini/gemini-cli",
                ],
            },
            install_overrides: &[],
        },
        DepSpec {
            id: "codex-cli",
            name: "OpenAI Codex CLI",
            description: "OpenAI 官方 Codex CLI(AI coding helper)。Mori provider \
                          `codex-bash` / `codex-cli` 需要這個 binary。\
                          \n\nWindows 注意:v0.130 起才有純 JS 版,native variant 不支援 Win。",
            unlocks: "provider=codex-bash / codex-cli 啟動 Codex 當 agent loop",
            size_hint: Some("~30MB(Node.js based)"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: Some(
                "Windows 需要 v0.130+(JS 版),舊 native variant 不支援。",
            ),
            check: CheckSpec::Which { bin: "codex" },
            check_overrides: &[],
            install: InstallSpec::Manual {
                commands: &[
                    "# 官方安裝(需先有 Node.js 18+):",
                    "npm install -g @openai/codex",
                    "# 安裝完跑 `codex login` 完成 OAuth 認證(用 ChatGPT 帳號)",
                    "# 或 `OPENAI_API_KEY` env",
                    "# 詳見 https://github.com/openai/codex",
                ],
            },
            install_overrides: &[],
        },
        DepSpec {
            id: "ollama",
            name: "ollama",
            description: "本機 LLM runtime(qwen3:8b / llama3 / 等)",
            unlocks: "provider=ollama 走 100% 離線 LLM",
            size_hint: Some("~600MB binary,每個 model 額外 4~30GB"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::Which { bin: "ollama" },
            check_overrides: &[],
            install: InstallSpec::Shell {
                script: "curl -fsSL https://ollama.com/install.sh | sh",
            },
            install_overrides: &[
                ("windows", InstallSpec::Manual {
                    commands: &[
                        "# 從官方下載 Windows installer:",
                        "# https://ollama.com/download/windows",
                        "# 下載完雙擊 OllamaSetup.exe 安裝即可。",
                    ],
                }),
            ],
        },
        DepSpec {
            id: "ollama-qwen3-8b",
            name: "qwen3:8b(Ollama 模型本體)",
            description: "Mori 預設離線 LLM 模型,支援 tool calling(Agent 模式必需)。\
                          需要 ollama binary 已裝。",
            unlocks: "ollama 真的能跑 LLM(只裝 ollama binary 沒模型也叫不起來)",
            size_hint: Some("~5GB"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::CommandStdoutContains {
                cmd: "ollama",
                args: &["list"],
                needle: "qwen3:8b",
            },
            check_overrides: &[],
            install: InstallSpec::Shell {
                script: "ollama pull qwen3:8b",
            },
            install_overrides: &[
                ("windows", InstallSpec::Manual {
                    commands: &[
                        "# PowerShell(需先裝 ollama):",
                        "ollama pull qwen3:8b",
                    ],
                }),
            ],
        },
        // Phase 3B:Hey Mori wake-word listener 的 Python runtime venv。
        // 沒裝 → Listening mode 進去後 spawn python 失敗,Tray 開「Hey Mori 待命」
        // 不會 work。
        DepSpec {
            id: "wake-listener-runtime",
            name: "Hey Mori 偵測 runtime(Python venv)",
            description: "Listening mode 用的 Python 環境 + openWakeWord。需要 Python 3.11+ \
                          (uv 會自動處理)。沒裝就無法用「Hey Mori」喚醒。",
            unlocks: "Listening mode + 對 mic 喊「Hey Mori」觸發 recording",
            size_hint: Some("~150MB"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            // venv python 路徑跨平台:Linux/macOS `bin/python`、Windows `Scripts/python.exe`。
            // CheckSpec::File 走 effective_check,各平台 override。
            check: CheckSpec::File {
                path_template: "$HOME/.mori/wake-venv/bin/python",
            },
            check_overrides: &[
                ("windows", CheckSpec::File {
                    path_template: "$HOME/.mori/wake-venv/Scripts/python.exe",
                }),
            ],
            // Linux/macOS:有 uv 用 uv,沒 uv fallback system python3.11
            // ⚠ uv venv 建的 venv **沒 pip 模組** — 必須用 `uv pip install` 或先
            // `python -m ensurepip` 補 pip 才能用。
            install: InstallSpec::Shell {
                script: "set -e; \
                         VENV=\"$HOME/.mori/wake-venv\"; \
                         UV=\"$HOME/.local/bin/uv\"; \
                         if [ ! -d \"$VENV\" ]; then \
                            if [ -x \"$UV\" ]; then \
                                echo '用 uv 建 venv...'; \
                                \"$UV\" venv \"$VENV\" --python 3.11; \
                            else \
                                echo '用 python -m venv...'; \
                                python3.11 -m venv \"$VENV\" || python3 -m venv \"$VENV\"; \
                            fi; \
                         fi; \
                         PACKAGES='openwakeword sounddevice numpy onnxruntime scikit-learn'; \
                         if [ -x \"$UV\" ]; then \
                            echo '用 uv pip install...'; \
                            \"$UV\" pip install --python \"$VENV/bin/python\" $PACKAGES; \
                         elif [ -x \"$VENV/bin/pip\" ]; then \
                            echo '用 venv pip...'; \
                            \"$VENV/bin/pip\" install $PACKAGES; \
                         else \
                            echo '用 ensurepip bootstrap...'; \
                            \"$VENV/bin/python\" -m ensurepip --upgrade; \
                            \"$VENV/bin/python\" -m pip install $PACKAGES; \
                         fi; \
                         echo '下載 openwakeword pipeline models(mel / embedding / silero_vad,~4 MB)...'; \
                         \"$VENV/bin/python\" -c 'from openwakeword.utils import download_models; download_models([\"__pipeline_only__\"])'; \
                         echo '  ↑ 傳 dummy name 跳過 pre-trained wake-words(jarvis/alexa/mycroft/...)— Mori 用自家 hey-mori.onnx'; \
                         echo '✓ wake-venv + openwakeword + pipeline models 裝好了'",
            },
            // Windows 走同樣的 uv 邏輯,只是 venv 內 python 在 Scripts\python.exe。
            // 走 Git Bash(sh -c),Tauri dep installer 端用 Command::new(\"sh\")。
            // 假設 user 已裝 Git for Windows(repo verify.sh 也依賴),沒裝就走 Manual
            // fallback。uv 沒裝會 fallback 提示去先裝 uv-runtime dep。
            install_overrides: &[
                ("windows", InstallSpec::Shell {
                    script: "set -e; \
                             VENV=\"$HOME/.mori/wake-venv\"; \
                             UV=\"$HOME/.local/bin/uv.exe\"; \
                             [ -x \"$UV\" ] || UV=\"$(command -v uv || true)\"; \
                             if [ -z \"$UV\" ] || [ ! -x \"$UV\" ]; then \
                                echo '✗ 找不到 uv — 請先安裝 uv-runtime(Deps tab 上方那一條)'; exit 1; \
                             fi; \
                             VENV_PY=\"$VENV/Scripts/python.exe\"; \
                             if [ ! -d \"$VENV\" ]; then \
                                echo '用 uv 建 venv(會自動下載 Python 3.11)...'; \
                                \"$UV\" venv \"$VENV\" --python 3.11; \
                             fi; \
                             PACKAGES='openwakeword sounddevice numpy onnxruntime scikit-learn'; \
                             echo '用 uv pip install...'; \
                             \"$UV\" pip install --python \"$VENV_PY\" $PACKAGES; \
                             echo '下載 openwakeword pipeline models(mel / embedding / silero_vad,~4 MB)...'; \
                             \"$VENV_PY\" -c 'from openwakeword.utils import download_models; download_models([\"__pipeline_only__\"])'; \
                             echo '  ↑ 傳 dummy name 跳過 pre-trained wake-words — Mori 用自家 hey-mori.onnx'; \
                             echo '✓ wake-venv + openwakeword + pipeline models 裝好了'",
                }),
            ],
        },
        // Phase 3D:edge-tts(Mori 講話)— 跟 wake-listener 共用 wake-venv,
        // 安裝小(~10MB),detect 看 edge-tts import 得起來。
        //
        // ⚠ wake-venv 多半是 uv 建的(`uv venv ...`),內部沒 pip 模組 — uv 走自己的
        // `uv pip install --python <venv-py>`。所以 install script 優先用 uv,失敗才
        // fallback 到 `python -m ensurepip` 補 pip 後再裝。
        DepSpec {
            id: "tts-runtime",
            name: "Mori 講話 runtime(edge-tts)",
            description: "Phase 3D TTS speak-back 用的 Python edge-tts。借 Microsoft Edge \
                          瀏覽器 TTS endpoint,免費 + native zh-TW 女聲。沒裝 → Config tab \
                          開 tts.enabled 也不會講話(只 log warn)。跟 wake-listener 共用 \
                          wake-venv,沒裝 wake-listener-runtime 的話先裝那個。",
            unlocks: "tts.enabled=true 時 Mori 用聲音回答你",
            size_hint: Some("~10MB"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::CommandStdoutContains {
                cmd: "sh",
                args: &[
                    "-c",
                    "$HOME/.mori/wake-venv/bin/python -c 'import edge_tts; print(\"ok\")' 2>&1",
                ],
                needle: "ok",
            },
            check_overrides: &[
                // Windows venv 內 python 在 Scripts\python.exe;sh -c 走 Git Bash
                ("windows", CheckSpec::CommandStdoutContains {
                    cmd: "sh",
                    args: &[
                        "-c",
                        "$HOME/.mori/wake-venv/Scripts/python.exe -c 'import edge_tts; print(\"ok\")' 2>&1",
                    ],
                    needle: "ok",
                }),
            ],
            install: InstallSpec::Shell {
                script: "set -e; \
                         VENV=\"$HOME/.mori/wake-venv\"; \
                         UV=\"$HOME/.local/bin/uv\"; \
                         if [ ! -x \"$VENV/bin/python\" ]; then \
                            echo '⚠ wake-venv 不存在 — 先裝 wake-listener-runtime'; exit 2; \
                         fi; \
                         if [ -x \"$UV\" ]; then \
                            echo '用 uv pip install...'; \
                            \"$UV\" pip install --python \"$VENV/bin/python\" edge-tts; \
                         elif [ -x \"$VENV/bin/pip\" ]; then \
                            echo '用 venv pip...'; \
                            \"$VENV/bin/pip\" install edge-tts; \
                         else \
                            echo '用 ensurepip bootstrap...'; \
                            \"$VENV/bin/python\" -m ensurepip --upgrade; \
                            \"$VENV/bin/python\" -m pip install edge-tts; \
                         fi; \
                         echo '✓ edge-tts 裝好了'",
            },
            install_overrides: &[
                // Windows 走 uv Shell(假設 wake-listener-runtime 先裝好,wake-venv
                // 已存在)。跟 Linux 同一條 uv 邏輯,只是 venv python 在 Scripts\python.exe。
                ("windows", InstallSpec::Shell {
                    script: "set -e; \
                             VENV=\"$HOME/.mori/wake-venv\"; \
                             UV=\"$HOME/.local/bin/uv.exe\"; \
                             [ -x \"$UV\" ] || UV=\"$(command -v uv || true)\"; \
                             VENV_PY=\"$VENV/Scripts/python.exe\"; \
                             if [ ! -x \"$VENV_PY\" ]; then \
                                echo '✗ wake-venv 不存在 — 先裝 wake-listener-runtime'; exit 2; \
                             fi; \
                             if [ -z \"$UV\" ] || [ ! -x \"$UV\" ]; then \
                                echo '✗ 找不到 uv — 請先安裝 uv-runtime'; exit 1; \
                             fi; \
                             echo '用 uv pip install...'; \
                             \"$UV\" pip install --python \"$VENV_PY\" edge-tts; \
                             echo '✓ edge-tts 裝好了'",
                }),
            ],
        },
        // Phase 3E:resemblyzer 聲紋辨識 — 共用 wake-venv,~100MB(含 80MB pretrained
        // VoiceEncoder + librosa + numpy deps)。預設 OFF,user 開啟才會 gate。
        DepSpec {
            id: "speaker-id-runtime",
            name: "聲紋辨識 runtime(resemblyzer)",
            description: "Phase 3E speaker verification 用的 Python resemblyzer(VoxCeleb \
                          pretrained voice encoder)。沒裝 → Config 開 speaker_id.enabled \
                          也不會 gate(只 log)。跟 wake-listener / edge-tts 共用 wake-venv。",
            unlocks: "speaker_id.enabled=true 時擋下別人聲音,只有 enrolled user 能叫 Mori",
            size_hint: Some("~100MB(含 pretrained model)"),
            needs_sudo: false,
            platforms: &["linux", "macos", "windows"],
            install_caveat: None,
            check: CheckSpec::CommandStdoutContains {
                cmd: "sh",
                args: &[
                    "-c",
                    "$HOME/.mori/wake-venv/bin/python -c 'import resemblyzer; print(\"ok\")' 2>&1",
                ],
                needle: "ok",
            },
            check_overrides: &[
                ("windows", CheckSpec::CommandStdoutContains {
                    cmd: "sh",
                    args: &[
                        "-c",
                        "$HOME/.mori/wake-venv/Scripts/python.exe -c 'import resemblyzer; print(\"ok\")' 2>&1",
                    ],
                    needle: "ok",
                }),
            ],
            install: InstallSpec::Shell {
                script: "set -e; \
                         VENV=\"$HOME/.mori/wake-venv\"; \
                         UV=\"$HOME/.local/bin/uv\"; \
                         if [ ! -x \"$VENV/bin/python\" ]; then \
                            echo '⚠ wake-venv 不存在 — 先裝 wake-listener-runtime'; exit 2; \
                         fi; \
                         if [ -x \"$UV\" ]; then \
                            echo '用 uv pip install resemblyzer...'; \
                            \"$UV\" pip install --python \"$VENV/bin/python\" resemblyzer; \
                         elif [ -x \"$VENV/bin/pip\" ]; then \
                            echo '用 venv pip...'; \
                            \"$VENV/bin/pip\" install resemblyzer; \
                         else \
                            echo '用 ensurepip bootstrap...'; \
                            \"$VENV/bin/python\" -m ensurepip --upgrade; \
                            \"$VENV/bin/python\" -m pip install resemblyzer; \
                         fi; \
                         echo '✓ resemblyzer 裝好了。第一次跑 enrollment 會自動下載 80MB pretrained model。'",
            },
            install_overrides: &[
                // Windows 走 uv Shell,同 tts-runtime / wake-listener-runtime pattern。
                ("windows", InstallSpec::Shell {
                    script: "set -e; \
                             VENV=\"$HOME/.mori/wake-venv\"; \
                             UV=\"$HOME/.local/bin/uv.exe\"; \
                             [ -x \"$UV\" ] || UV=\"$(command -v uv || true)\"; \
                             VENV_PY=\"$VENV/Scripts/python.exe\"; \
                             if [ ! -x \"$VENV_PY\" ]; then \
                                echo '✗ wake-venv 不存在 — 先裝 wake-listener-runtime'; exit 2; \
                             fi; \
                             if [ -z \"$UV\" ] || [ ! -x \"$UV\" ]; then \
                                echo '✗ 找不到 uv — 請先安裝 uv-runtime'; exit 1; \
                             fi; \
                             echo '用 uv pip install resemblyzer(~100MB,含 80MB pretrained model)...'; \
                             \"$UV\" pip install --python \"$VENV_PY\" resemblyzer; \
                             echo '✓ resemblyzer 裝好了。第一次跑 enrollment 會自動下載 pretrained model。'",
                }),
            ],
        },
        // Wave 3 整合:annuli reflection 服務的 runtime。
        //
        // 狀態:annuli Wave 2 + Wave 3 已落地(2026-05),Wave 4 進行中。整合可
        // 以開始實機跑;ship-day checklist 見 docs/design/annuli-wave3-integration.md。
        DepSpec {
            id: "annuli-runtime",
            name: "Annuli 反思服務 runtime(Python venv)",
            description: "Mori 的「年輪」反思服務 — vault-backed,寫長期記憶 + 跑 \
                          /sleep ring reflection + 4-layer reflection(events / digest \
                          / rings / curator)。沒裝 → mori-desktop 走 ~/.mori/memory/ \
                          本機 fallback(目前狀態 OK,SOUL.md / 4-layer ring 全失效)。",
            unlocks: "annuli.enabled=true 時走 HTTP 對接 ~/mori-universe/spirits/<name>/ \
                      vault(SOUL.md + MEMORY.md + events + rings)",
            size_hint: Some("~200MB(annuli code + venv + deps)"),
            needs_sudo: false,
            platforms: &["linux", "macos"],
            install_caveat: Some(
                "Annuli Wave 4 仍在進行中。基本對接(SOUL / MEMORY / events / rings / sleep)\
                 都接好可用,Wave 4 新 endpoints(memory section)還在收尾。裝完到 Config tab \
                 開 annuli.enabled + 填 endpoint http://localhost:5000 即可連線。\
                 整合 checklist 見 docs/design/annuli-wave3-integration.md。",
            ),
            check: CheckSpec::File {
                path_template: "$HOME/mori-universe/annuli/.venv/bin/python",
            },
            check_overrides: &[],
            install: InstallSpec::Shell {
                script: "set -e; \
                         ANNULI=\"$HOME/mori-universe/annuli\"; \
                         UV=\"$HOME/.local/bin/uv\"; \
                         if [ ! -d \"$ANNULI\" ]; then \
                            echo '從 GitHub 拉 annuli...'; \
                            mkdir -p \"$HOME/mori-universe\"; \
                            git clone https://github.com/yazelin/annuli \"$ANNULI\"; \
                         fi; \
                         cd \"$ANNULI\"; \
                         if [ ! -d \"$ANNULI/.venv\" ]; then \
                            if [ -x \"$UV\" ]; then \
                                echo '用 uv 建 venv...'; \
                                \"$UV\" venv \"$ANNULI/.venv\" --python 3.11; \
                            else \
                                python3.11 -m venv \"$ANNULI/.venv\" || python3 -m venv \"$ANNULI/.venv\"; \
                            fi; \
                         fi; \
                         if [ -x \"$UV\" ]; then \
                            echo '用 uv pip install -e .'; \
                            \"$UV\" pip install --python \"$ANNULI/.venv/bin/python\" -e \"$ANNULI\"; \
                         elif [ -x \"$ANNULI/.venv/bin/pip\" ]; then \
                            \"$ANNULI/.venv/bin/pip\" install -e \"$ANNULI\"; \
                         else \
                            \"$ANNULI/.venv/bin/python\" -m ensurepip --upgrade; \
                            \"$ANNULI/.venv/bin/python\" -m pip install -e \"$ANNULI\"; \
                         fi; \
                         echo '✓ annuli runtime 裝好了。記得到 Config tab 開 annuli.enabled + 填 endpoint http://localhost:5000。'",
            },
            install_overrides: &[
                ("windows", InstallSpec::Manual {
                    commands: &[
                        "# Windows 一鍵安裝尚未實作 — annuli wave 2 ship 後再補。",
                        "# 暫時手動跑(需 git + Python 3.11):",
                        "git clone https://github.com/yazelin/annuli %USERPROFILE%\\mori-universe\\annuli",
                        "cd %USERPROFILE%\\mori-universe\\annuli",
                        "uv venv .venv --python 3.11",
                        "uv pip install --python .venv\\Scripts\\python.exe -e .",
                    ],
                }),
            ],
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

/// 跑 detect 用的子程序。集中設 `CREATE_NO_WINDOW`(GUI parent spawn console
/// child 才不會閃黑框)+ 短 timeout(check 不該超過 5s,卡住的 ollama / pip
/// 不能拖住整個 DepsTab)。
fn run_check_cmd(cmd: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut c = Command::new(cmd);
    c.args(args.iter());
    mori_core::suppress_console_on_windows!(c);
    c.output()
}

pub fn check_dep(spec: &DepSpec) -> DepStatus {
    // 走 effective_check — 平台特定 override 優先(像 Windows 的 uv 用 .exe 副檔名),
    // 沒有再 fallback 預設(Linux 慣例 path)。
    match spec.effective_check() {
        CheckSpec::Which { bin } => {
            // Windows 沒 `which`,用內建的 `where.exe`(Cmd built-in,但 where.exe 是
            // 真檔案在 System32)。Linux/macOS 用 `which`。
            let cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
            match run_check_cmd(cmd, &[bin]) {
                Ok(out) if out.status.success() => DepStatus {
                    id: spec.id,
                    installed: true,
                    detail: Some(String::from_utf8_lossy(&out.stdout).trim().to_string()),
                },
                _ => DepStatus { id: spec.id, installed: false, detail: None },
            }
        }
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
            match run_check_cmd(cmd, args) {
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
    // 走 effective_install — 平台特定 override 優先(像 Windows 的 ollama
    // 用 Manual variant),沒有再 fallback 預設(Linux 走 Shell 那條)。
    match spec.effective_install() {
        InstallSpec::Shell { script } => {
            tracing::info!(dep = spec.id, "shell install start");
            run_shell(script)
        }
        InstallSpec::Manual { .. } => {
            anyhow::bail!("Manual install — UI should show commands to user, not call run_install");
        }
        InstallSpec::Download {
            url,
            dest_dir,
            extract_members,
            make_executable,
        } => {
            tracing::info!(dep = spec.id, %url, dest_dir, "download install start");
            run_download(url, dest_dir, extract_members, *make_executable)
        }
    }
}

fn run_shell(script: &str) -> Result<InstallResult> {
    let output = Command::new("sh")
        .args(["-c", script])
        .output()
        .map_err(|e| anyhow::anyhow!("spawn sh: {e}"))?;

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

/// 跨平台:reqwest blocking 抓 archive → zip crate 解壓 → 把 `extract_members`
/// 對應檔案(或全部)複製到 `dest_dir`。
///
/// 不用 sh / curl / unzip,純 Rust。Windows / Linux 都跑。
fn run_download(
    url: &str,
    dest_dir: &str,
    extract_members: &[&str],
    make_executable: bool,
) -> Result<InstallResult> {
    let mut log = String::new();
    let dest = expand_home(dest_dir);
    let dest_path = std::path::PathBuf::from(&dest);
    std::fs::create_dir_all(&dest_path)
        .map_err(|e| anyhow::anyhow!("create dest dir {dest}: {e}"))?;
    log.push_str(&format!("==> dest: {dest}\n"));

    // 1. 下載
    log.push_str(&format!("==> downloading {url}\n"));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| anyhow::anyhow!("build reqwest client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Ok(InstallResult {
            success: false,
            exit_code: None,
            output: format!("{log}HTTP {} — download failed", resp.status()),
        });
    }
    let bytes = resp
        .bytes()
        .map_err(|e| anyhow::anyhow!("read response body: {e}"))?;
    log.push_str(&format!("==> got {} bytes\n", bytes.len()));

    // 2. 解壓(目前只支援 .zip)
    let cursor = std::io::Cursor::new(bytes.as_ref());
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow::anyhow!("open zip archive: {e}"))?;

    let mut extracted = 0usize;
    let want_all = extract_members.is_empty();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| anyhow::anyhow!("zip entry {i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let basename = std::path::Path::new(&name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        // 過濾:want_all 或 basename 在 extract_members 內
        let want = want_all || extract_members.iter().any(|m| *m == basename);
        if !want {
            continue;
        }

        // 攤平 path,只取 basename — 不複製 archive 內部資料夾結構
        let out_path = dest_path.join(basename);
        let mut out_file = std::fs::File::create(&out_path)
            .map_err(|e| anyhow::anyhow!("create {}: {e}", out_path.display()))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| anyhow::anyhow!("write {}: {e}", out_path.display()))?;
        log.push_str(&format!("==> extracted {basename}\n"));
        extracted += 1;

        // Unix:對 executable 加 +x bit。Windows ignore(.exe 自帶可執行屬性)。
        #[cfg(unix)]
        if make_executable {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o755));
        }
        #[cfg(not(unix))]
        {
            let _ = make_executable;
        }
    }

    if extracted == 0 {
        return Ok(InstallResult {
            success: false,
            exit_code: None,
            output: format!("{log}no files extracted (extract_members={extract_members:?})"),
        });
    }

    log.push_str(&format!("==> done, {extracted} file(s) extracted to {dest}\n"));
    Ok(InstallResult {
        success: true,
        exit_code: None,
        output: log,
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
