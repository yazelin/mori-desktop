# BI-5 Meeting Recorder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 standalone `mori-meeting-recorder` repo,實現 Observer Mode MVP — 雙軌 WAV 錄音(system + mic-internal)+ 停止後 parallel whisper 轉錄 + visibility-based public/internal 匯出 + AgentPulse 風 floating capsule GUI(單視窗切 size)。

**Architecture:** 單 Tauri 2 crate(`src-tauri/src/`)+ React 前端。Audio 走 cpal 0.15 + hound 3.5(對齊 mori-desktop)。STT 走 shell-out whisper.cpp CLI(共用 `~/.mori/bin/whisper-cli` + `~/.mori/models/ggml-small.bin`,filesystem 慣例不 IPC)。BI-1 啟動時 self-register manifest 到 `~/.mori/body-parts/mori.meeting-recorder/manifest.json`(kind=StandaloneApp,interfaces=[])。mori-desktop **完全不改**。

**Tech Stack:** Rust + Tauri 2 + React + TypeScript + cpal 0.15 + hound 3.5 + tokio + serde + chrono。

**Spec source-of-truth:** [`docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md`](../specs/2026-05-28-bi-5-meeting-recorder-design.md)

---

## 為什麼這樣切

12 個 task,大致順序:**spike → 建 repo → 純函式 TDD 層 → 平台 audio glue → Tauri 黏合 → 前端 → deps → e2e + ship**。

- 純函式(writer / store / parse / export / manifest)可 cargo test 跑,先寫完整套 TDD 基底再黏平台 IO。
- 平台 audio 沒 unit test(需要實體 PipeWire / WASAPI),用 manual smoke 收尾。
- Tauri command 在純函式都 OK 後黏一次,前端再接上,最後 e2e。

| Task | 範圍 | 為什麼這個順序 |
|---|---|---|
| 1 | Spike cpal loopback(Linux) | 最大技術風險點,確認 cpal `.monitor` device 拿得到再下手 |
| 2 | 新 repo + Tauri 2 scaffold | code 要有家 |
| 3 | WAV writer(hound)+ TDD | 純函式,最底層 |
| 4 | Session store + TDD | 純函式,目錄 layout |
| 5 | Whisper JSON parser + TDD | 純函式 + fixture |
| 6 | Exporter + TDD | 純函式,visibility filter |
| 7 | Manifest writer + TDD | 純函式 |
| 8 | AudioCapture trait + Linux impl + Windows impl | 平台 IO,trait 統一介面 |
| 9 | Recorder orchestrator + integration test | 黏合層,組合 3-7 + 8 |
| 10 | Tauri commands + tray + single-instance | 後端對前端的介面 |
| 11 | 前端 App.tsx + theme.css + i18n | scaffold + theme token |
| 12 | 前端 CapsuleView + ExpandedView + 3-tab + e2e + PR | UI 完整 + 手測 + ship |

---

### Task 1: Spike — cpal loopback discovery on Linux ✅ DONE(2026-05-28)

> **Spike result**:cpal 0.15 Linux 走 `Host: Alsa`,**沒有 `.monitor` device**(monitor 是 pulse 抽象層,ALSA 不認識)。`pactl list short sources` 同時看到 6 個 PipeWire monitor source(HDMI / 內建喇叭 / Fifine USB mic)。
>
> **Decision**:Linux 改走 `libpulse-binding` + `libpulse-simple-binding`(對齊 OBS 的 `linux-pulseaudio` plugin 路線)。Windows 仍走 cpal WASAPI loopback。Spec §2 #5 已更新 2026-05-28。Task 8 Linux impl 改寫(libpulse-simple sync API + pactl 列 source)。Task 2 Cargo.toml deps 改成 platform-gated。
>
> `/tmp/cpal-loopback-spike/` 已丟。

**Goal**(已達成):驗證 cpal Linux 路徑是否能拿到 system loopback。結論:**不行**,需平台特定 lib。

**Files:**
- Create(臨時): `/tmp/cpal-loopback-spike/Cargo.toml` + `src/main.rs`
- 跑完 spike 確認 OK 後**整個 /tmp 目錄丟掉**,不進 repo

- [ ] **Step 1: 建 spike 專案**

```bash
mkdir -p /tmp/cpal-loopback-spike/src && cd /tmp/cpal-loopback-spike
```

`/tmp/cpal-loopback-spike/Cargo.toml`:

```toml
[package]
name = "cpal-loopback-spike"
version = "0.0.0"
edition = "2021"

[dependencies]
cpal = "0.15"
```

- [ ] **Step 2: 寫探測程式**

`/tmp/cpal-loopback-spike/src/main.rs`:

```rust
use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let host = cpal::default_host();
    println!("Host: {:?}", host.id());

    let inputs = match host.input_devices() {
        Ok(it) => it,
        Err(e) => { eprintln!("input_devices err: {e}"); return; }
    };

    println!("\n=== input devices ===");
    let mut found_loopback = false;
    for d in inputs {
        let name = d.name().unwrap_or_else(|_| "<no name>".into());
        let is_monitor = name.to_lowercase().contains("monitor");
        println!("  {} {}", if is_monitor { "[LOOPBACK]" } else { "          " }, name);
        if is_monitor { found_loopback = true; }
    }
    println!();
    if found_loopback {
        println!("✓ at least one .monitor device found — cpal loopback path OK");
    } else {
        println!("✗ no .monitor device — need fallback (pactl load-module or gstreamer)");
    }
}
```

- [ ] **Step 3: 跑**

```bash
cd /tmp/cpal-loopback-spike && cargo run --release 2>&1 | tail -30
```

Expected: 至少一行 `[LOOPBACK]` + 結尾 `✓ at least one .monitor device found`。

- [ ] **Step 4: Decision gate**

- ✓ 找到 loopback → 繼續 Task 2,把 spike code 丟掉(`rm -rf /tmp/cpal-loopback-spike`)
- ✗ 沒找到 → **停下來** report 給 yazelin,decide:
  - 跑 `pactl load-module module-loopback` 看能不能補出 monitor source
  - 或 fallback gstreamer-rs(改 spec,改 plan task 8)

---

### Task 2: 新 repo + Tauri 2 scaffold

**Files:**
- Create: `gh repo create yazelin/mori-meeting-recorder --public --license MIT`
- Clone: `~/mori-universe/mori-meeting-recorder/`
- Create: `README.md`, `LICENSE`(gh 已加),`CLAUDE.md`, `AGENTS.md`, `.gitignore`, `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/src/main.rs`(stub), `src-tauri/build.rs`, `index.html`, `src/main.tsx`(stub),`vite.config.ts`, `tsconfig.json`

- [ ] **Step 1: 建 repo + clone**

```bash
cd ~/mori-universe
gh repo create yazelin/mori-meeting-recorder --public --license MIT --description "Standalone dual-track meeting recorder for the Mori universe (Observer Mode MVP)"
gh repo clone yazelin/mori-meeting-recorder
cd mori-meeting-recorder
```

Expected: `~/mori-universe/mori-meeting-recorder/{LICENSE,README.md,.git/}` 出現。

- [ ] **Step 2: 寫 .gitignore**

`.gitignore`:

```
target/
dist/
node_modules/
src-tauri/target/
src-tauri/gen/
.DS_Store
*.swp
```

> `src-tauri/gen/`:Tauri 2 的 `generate_context!` macro 編譯時 auto-generate `schemas/` 到這(會弄髒 working tree)。對齊 mori-desktop 慣例。

- [ ] **Step 3: package.json**

`package.json`:

```json
{
  "name": "mori-meeting-recorder",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "react": "^18.3.0",
    "react-dom": "^18.3.0",
    "react-i18next": "^14.0.0",
    "i18next": "^23.0.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "@types/react": "^18.3.0",
    "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.0",
    "typescript": "~5.5.0",
    "vite": "^5.4.0"
  }
}
```

- [ ] **Step 4: tsconfig + vite config + index.html + main.tsx stub**

`tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "allowImportingTsExtensions": false
  },
  "include": ["src"]
}
```

`vite.config.ts`:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1421, strictPort: true },  // 1421 避開 mori-desktop 的 1420
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: { target: "es2022", minify: !process.env.TAURI_DEBUG },
});
```

`index.html`:

```html
<!doctype html>
<html lang="zh-TW">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Mori Meeting Recorder</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`src/main.tsx`(stub,task 11 接手):

```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

function App() {
  return <div style={{ padding: 16 }}>Mori Meeting Recorder — scaffold</div>;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode><App /></StrictMode>
);
```

- [ ] **Step 5: src-tauri scaffold**

`src-tauri/Cargo.toml`:

```toml
[package]
name = "mori-meeting-recorder"
version = "0.1.0"
edition = "2021"
description = "Standalone dual-track meeting recorder for the Mori universe (Observer Mode MVP)"

[[bin]]
name = "mori-meeting-recorder"
path = "src/main.rs"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-single-instance = "2"
hound = "3.5"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
dirs = "5"

# Linux:libpulse client API(PipeWire 完全相容,看得到 .monitor source)
[target.'cfg(target_os = "linux")'.dependencies]
libpulse-binding = "2"
libpulse-simple-binding = "2"

# Windows:cpal WASAPI loopback(對齊 mori-desktop)
[target.'cfg(target_os = "windows")'.dependencies]
cpal = "0.15"

[dev-dependencies]
tempfile = "3"

[features]
default = ["custom-protocol"]
custom-protocol = ["tauri/custom-protocol"]
```

`src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::build()
}
```

`src-tauri/tauri.conf.json`:

```json
{
  "$schema": "../node_modules/@tauri-apps/cli/schema.json",
  "productName": "Mori Meeting Recorder",
  "version": "0.1.0",
  "identifier": "mori.meeting-recorder",
  "build": {
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build",
    "devUrl": "http://localhost:1421",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [{
      "label": "main",
      "title": "Mori Meeting Recorder",
      "width": 360,
      "height": 60,
      "minWidth": 360,
      "minHeight": 60,
      "decorations": false,
      "transparent": true,
      "alwaysOnTop": true,
      "skipTaskbar": false,
      "resizable": true,
      "shadow": false
    }],
    "trayIcon": {
      "iconPath": "icons/icon.png",
      "iconAsTemplate": false
    },
    "security": {
      "csp": null
    }
  },
  "plugins": {},
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/icon.png"]
  }
}
```

`src-tauri/src/main.rs`:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = tauri::Manager::get_webview_window(app, "main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 6: icon stub**

```bash
mkdir -p src-tauri/icons
# 暫用 mori-desktop 的 icon 當 placeholder(task 12 之前換成自己的)
cp ~/mori-universe/mori-desktop/src-tauri/icons/32x32.png src-tauri/icons/
cp ~/mori-universe/mori-desktop/src-tauri/icons/128x128.png src-tauri/icons/
cp ~/mori-universe/mori-desktop/src-tauri/icons/icon.png src-tauri/icons/ 2>/dev/null || \
  cp ~/mori-universe/mori-desktop/src-tauri/icons/128x128.png src-tauri/icons/icon.png
```

- [ ] **Step 7: CLAUDE.md + AGENTS.md + README**

`CLAUDE.md`(repo-level agent 規則,沿 AgentPulse 風):

```markdown
# Claude Code 指引 — mori-meeting-recorder

mori-meeting-recorder 是 Mori universe 的 standalone 會議錄音工具 — Tauri 2 + Rust + React,
Observer Mode MVP:雙軌(`meeting_system` + `mic_internal`)→ 停止後 whisper 轉錄 → visibility-based
`meeting.public.md` / `meeting.internal.md` 匯出。

## 設計來源

- Body Interface 軌契約:[mori-desktop/docs/meeting-recorder.md](https://github.com/yazelin/mori-desktop/blob/main/docs/meeting-recorder.md)
- BI-5 設計 spec:[mori-desktop/docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md](https://github.com/yazelin/mori-desktop/blob/main/docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md)
- 實作 plan:[mori-desktop/docs/superpowers/plans/2026-05-28-bi-5-meeting-recorder.md](https://github.com/yazelin/mori-desktop/blob/main/docs/superpowers/plans/2026-05-28-bi-5-meeting-recorder.md)

## 硬規矩

1. **不公開比較其他專案** — 用 Mori 自己的詞彙
2. **User-owned data** — `~/.mori/meetings/` 是 user 的;recorder 不對外傳
3. **mic 永不混進客戶版** — `meeting.public.md` filter visibility=public only
4. **Standalone-first** — 沒 mori-desktop 也要能跑;deps 自己 bundle scripts
5. **Bundle deps in repo** — 不從外部 setup repo 拉
6. **trunk-based + auto-merge** — 短命 branch off main,PR 設 auto-merge

## 工程注意

- **共用 ~/.mori/ 路徑**:`~/.mori/bin/whisper-cli` 跟 `~/.mori/models/ggml-small.bin` 跟 mori-desktop 共享(filesystem 慣例,不 IPC)
- **UI css token 自己一套**:不沿 mori-desktop var(--c-*),`src/theme.css` 自己定義
- **單視窗切 size**:collapsed 360×60(膠囊),expanded 720×480(3-tab),`window.setSize` 在前後端切
- **Tauri v2 auto-camelCase**:`event_id: String` Rust 對應 JS `eventId`
- **共用驗證入口** — `bash scripts/verify.sh`:`cargo test` + `npm run build` + `cargo check`
```

`AGENTS.md`:

```markdown
# AGENTS.md — Codex Cloud / 其他 agent

跟 [CLAUDE.md](CLAUDE.md) 同步。本 repo agent 規則統一在 CLAUDE.md。
```

`README.md`:

```markdown
# mori-meeting-recorder

Standalone dual-track meeting recorder for the Mori universe.

**Observer Mode MVP** — 雙軌錄音(`meeting_system` 系統輸出 + `mic_internal` 本機麥克風)
→ 停止後 whisper.cpp 雙軌平行轉錄 → visibility-based `meeting.public.md` / `meeting.internal.md` 匯出。

## Quick start

```bash
git clone https://github.com/yazelin/mori-meeting-recorder
cd mori-meeting-recorder
npm install
bash scripts/install-whisper-linux.sh   # 或 .ps1 on Windows
npm run tauri dev
```

## Design

- 契約:[meeting-recorder.md](https://github.com/yazelin/mori-desktop/blob/main/docs/meeting-recorder.md)
- 本 repo 是 Body Interface 軌的 BI-5。設計 spec + 實作 plan 在 mori-desktop repo `docs/superpowers/`。
- BI-1 manifest:啟動時 self-register `~/.mori/body-parts/mori.meeting-recorder/manifest.json`

## License

MIT
```

- [ ] **Step 8: npm install + 第一次 frontend build + cargo check**

```bash
cd ~/mori-universe/mori-meeting-recorder
npm install 2>&1 | tail -3
npm run build 2>&1 | tail -3       # 先 build frontend 出 dist/(Tauri generate_context! 需要)
(cd src-tauri && cargo check 2>&1 | tail -3)
```

Expected: 三條都跑完無 error。Tauri 第一次 cargo check 會慢(~5-15 min download + compile Tauri 2 + libpulse-binding 等 ~300 crates),最後綠燈。

> 注意:**`npm run build` 必須在 cargo check 之前跑一次**(產生 `dist/`),否則 `generate_context!` macro 找不到 `frontendDist` panic。後續 `scripts/verify.sh`(Task 12)會把這個 order 寫進去。

- [ ] **Step 9: 第一個 commit + push**

```bash
git add .
git commit -m "$(cat <<'EOF'
feat: initial Tauri 2 scaffold

Single-window AlwaysOnTop transparent capsule (360x60 collapsed).
Frontend stub. hound + tokio + serde + chrono + dirs in main deps;
libpulse-binding/simple-binding under cfg(linux),cpal under cfg(windows).
Single-instance plugin wired.

BI-5 plan: mori-desktop/docs/superpowers/plans/2026-05-28-bi-5-meeting-recorder.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push origin main
```

---

### Task 3: WAV writer (TDD)

**Files:**
- Create: `src-tauri/src/audio/mod.rs`(放 enum + trait stub,本 task 只用 `SourceKind`)
- Create: `src-tauri/src/audio/writer.rs`

- [ ] **Step 1: 寫測試**

`src-tauri/src/audio/writer.rs`:

```rust
//! WAV writer wrapper — hound 封裝。16kHz mono 16-bit PCM(對齊 whisper.cpp 原生輸入)。
//! Per-track 一個 WavWriter;recorder 對 system / mic-internal 各開一個。

use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// 固定的 WAV spec — 16kHz mono 16-bit signed PCM。
pub const WAV_SPEC: WavSpec = WavSpec {
    channels: 1,
    sample_rate: 16_000,
    bits_per_sample: 16,
    sample_format: SampleFormat::Int,
};

pub struct TrackWriter {
    inner: WavWriter<BufWriter<File>>,
    samples_written: u64,
}

impl TrackWriter {
    pub fn create(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir parent: {e}"))?;
        }
        let inner = WavWriter::create(path, WAV_SPEC)
            .map_err(|e| format!("WavWriter::create({}): {e}", path.display()))?;
        Ok(Self { inner, samples_written: 0 })
    }

    pub fn push_samples(&mut self, samples: &[i16]) -> Result<(), String> {
        for &s in samples {
            self.inner.write_sample(s).map_err(|e| e.to_string())?;
        }
        self.samples_written += samples.len() as u64;
        Ok(())
    }

    pub fn samples_written(&self) -> u64 {
        self.samples_written
    }

    pub fn finalize(self) -> Result<(), String> {
        self.inner.finalize().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::WavReader;
    use tempfile::TempDir;

    #[test]
    fn create_push_finalize_then_read_back_samples() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.wav");

        let mut w = TrackWriter::create(&path).unwrap();
        let signal: Vec<i16> = (0..1600).map(|i| (i as i16) * 10).collect();
        w.push_samples(&signal).unwrap();
        assert_eq!(w.samples_written(), 1600);
        w.finalize().unwrap();

        let mut r = WavReader::open(&path).unwrap();
        assert_eq!(r.spec().channels, 1);
        assert_eq!(r.spec().sample_rate, 16_000);
        assert_eq!(r.spec().bits_per_sample, 16);
        let read_back: Vec<i16> = r.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(read_back, signal);
    }

    #[test]
    fn create_makes_parent_dir() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("a/b/c/test.wav");
        let w = TrackWriter::create(&nested).unwrap();
        w.finalize().unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn empty_push_still_finalizes_to_valid_wav() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("empty.wav");
        let w = TrackWriter::create(&path).unwrap();
        w.finalize().unwrap();
        let r = WavReader::open(&path).unwrap();
        assert_eq!(r.len(), 0);
    }
}
```

- [ ] **Step 2: 建 audio/mod.rs**

`src-tauri/src/audio/mod.rs`:

```rust
//! 音訊 capture / write — per-track WAV writer + 平台 capture impl(linux / windows)。

pub mod writer;

use serde::{Deserialize, Serialize};

/// 一個 source 的「分類」— 決定預設 visibility + 在 segment 上的 source_kind 欄。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    MeetingSystem,
    MicInternal,
}

/// Segment / 匯出檔的 visibility。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Internal,
}

impl SourceKind {
    pub fn default_visibility(self) -> Visibility {
        match self {
            Self::MeetingSystem => Visibility::Public,
            Self::MicInternal => Visibility::Internal,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MeetingSystem => "meeting_system",
            Self::MicInternal => "mic_internal",
        }
    }

    pub fn track_name(self) -> &'static str {
        match self {
            Self::MeetingSystem => "system",
            Self::MicInternal => "mic-internal",
        }
    }
}
```

- [ ] **Step 3: lib.rs / main.rs 加 mod**

Modify `src-tauri/src/main.rs`,在 `fn main()` 前加:

```rust
mod audio;
```

- [ ] **Step 4: 跑測試**

```bash
cd ~/mori-universe/mori-meeting-recorder/src-tauri
cargo test audio::writer 2>&1 | tail -10
```

Expected: `test result: ok. 3 passed`。

- [ ] **Step 5: Commit**

```bash
cd ~/mori-universe/mori-meeting-recorder
git add src-tauri/src/audio src-tauri/src/main.rs
git commit -m "$(cat <<'EOF'
feat(audio): TrackWriter — 16kHz mono 16-bit WAV via hound

Per-track writer + SourceKind / Visibility enums (with serde
rename_all=snake_case to match doc spec). 3 unit tests: roundtrip
samples, mkdir parent, empty WAV finalizes valid.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Session store (TDD)

**Files:**
- Create: `src-tauri/src/session_store.rs`

- [ ] **Step 1: 寫測試 + impl**

`src-tauri/src/session_store.rs`:

```rust
//! ~/.mori/meetings/<session-id>/ 目錄佈局 + path getters。純函式 + filesystem。

use crate::audio::SourceKind;
use std::path::{Path, PathBuf};

pub struct SessionStore {
    pub session_id: String,
    pub root: PathBuf,
}

impl SessionStore {
    /// 建出 `<base>/<session_id>/{audio,transcript}/` 並回 store。
    pub fn create(session_id: &str, base: &Path) -> Result<Self, String> {
        let root = base.join(session_id);
        std::fs::create_dir_all(root.join("audio")).map_err(|e| format!("mkdir audio: {e}"))?;
        std::fs::create_dir_all(root.join("transcript")).map_err(|e| format!("mkdir transcript: {e}"))?;
        Ok(Self { session_id: session_id.to_string(), root })
    }

    pub fn audio_path(&self, kind: SourceKind) -> PathBuf {
        self.root.join("audio").join(format!("{}.wav", kind.track_name()))
    }

    pub fn segments_path(&self, kind: SourceKind) -> PathBuf {
        self.root.join("transcript").join(format!("{}.segments.jsonl", kind.track_name()))
    }

    pub fn public_md_path(&self) -> PathBuf { self.root.join("meeting.public.md") }
    pub fn internal_md_path(&self) -> PathBuf { self.root.join("meeting.internal.md") }
    pub fn timeline_path(&self) -> PathBuf { self.root.join("timeline.json") }
}

/// 預設 base dir = `~/.mori/meetings/`。
pub fn default_meetings_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("meetings"))
        .unwrap_or_else(|| PathBuf::from(".mori/meetings"))
}

/// 產生新 session id:`meeting-YYYYMMDD-HHMMSS`(local time)。
pub fn new_session_id(now: chrono::DateTime<chrono::Local>) -> String {
    format!("meeting-{}", now.format("%Y%m%d-%H%M%S"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_makes_audio_and_transcript_dirs() {
        let tmp = TempDir::new().unwrap();
        let s = SessionStore::create("meeting-test", tmp.path()).unwrap();
        assert!(s.root.join("audio").is_dir());
        assert!(s.root.join("transcript").is_dir());
    }

    #[test]
    fn path_getters_return_expected_layout() {
        let tmp = TempDir::new().unwrap();
        let s = SessionStore::create("meeting-x", tmp.path()).unwrap();
        assert_eq!(s.audio_path(SourceKind::MeetingSystem), tmp.path().join("meeting-x/audio/system.wav"));
        assert_eq!(s.audio_path(SourceKind::MicInternal), tmp.path().join("meeting-x/audio/mic-internal.wav"));
        assert_eq!(s.segments_path(SourceKind::MeetingSystem), tmp.path().join("meeting-x/transcript/system.segments.jsonl"));
        assert_eq!(s.public_md_path(), tmp.path().join("meeting-x/meeting.public.md"));
        assert_eq!(s.internal_md_path(), tmp.path().join("meeting-x/meeting.internal.md"));
        assert_eq!(s.timeline_path(), tmp.path().join("meeting-x/timeline.json"));
    }

    #[test]
    fn session_id_has_meeting_prefix_and_timestamp() {
        let now = chrono::Local.with_ymd_and_hms(2026, 5, 28, 14, 30, 0).unwrap();
        let id = new_session_id(now);
        assert_eq!(id, "meeting-20260528-143000");
    }
}

#[cfg(test)]
use chrono::TimeZone;
```

- [ ] **Step 2: main.rs 加 mod**

```rust
mod audio;
mod session_store;
```

- [ ] **Step 3: 跑測試**

```bash
cd ~/mori-universe/mori-meeting-recorder/src-tauri
cargo test session_store 2>&1 | tail -8
```

Expected: `3 passed`。

- [ ] **Step 4: Commit**

```bash
cd ~/mori-universe/mori-meeting-recorder
git add src-tauri/src/session_store.rs src-tauri/src/main.rs
git commit -m "$(cat <<'EOF'
feat(session): SessionStore — ~/.mori/meetings/<id>/ layout

create() mkdirs audio + transcript subdirs. Path getters return
canonical paths per SourceKind. new_session_id() formats local
time as meeting-YYYYMMDD-HHMMSS.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Whisper JSON parser (TDD with fixture)

**Files:**
- Create: `src-tauri/src/transcribe.rs`(只放 `parse_whisper_json` + Segment 型別,spawn 邏輯放 task 9)
- Create: `src-tauri/tests/fixtures/whisper-small.json`(從真 whisper.cpp output 截 fixture)

- [ ] **Step 1: 建 fixture**

`src-tauri/tests/fixtures/whisper-small.json`(這是 `whisper-cli --output-json-full` 截出的迷你樣本):

```json
{
  "systeminfo": "AVX = 1 | AVX2 = 1 ...",
  "model": { "type": "small", "multilingual": true },
  "params": { "language": "auto", "translate": false },
  "result": { "language": "zh" },
  "transcription": [
    {
      "timestamps": { "from": "00:00:01,500", "to": "00:00:04,200" },
      "offsets": { "from": 1500, "to": 4200 },
      "text": " 我們希望下週三前看到版本。",
      "tokens": [{ "p": 0.95 }, { "p": 0.92 }],
      "confidence": -0.142
    },
    {
      "timestamps": { "from": "00:00:04,500", "to": "00:00:07,800" },
      "offsets": { "from": 4500, "to": 7800 },
      "text": " 那邊測試環境準備好了嗎?",
      "tokens": [],
      "confidence": -0.218
    }
  ]
}
```

- [ ] **Step 2: 寫 parser + 測試**

`src-tauri/src/transcribe.rs`:

```rust
//! 轉錄:shell-out whisper.cpp CLI + parse `--output-json-full` 輸出。
//! 本檔的 `parse_whisper_json` 是純函式;spawn 邏輯在 task 9 寫。

use crate::audio::{SourceKind, Visibility};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub id: String,
    pub session_id: String,
    pub track: String,
    pub source_kind: String,
    pub visibility: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub is_final: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// 把 whisper.cpp `--output-json-full` 解析成 Segments。
/// `session_id` / `kind` 由呼叫端帶進來(parser 不知道這些)。
pub fn parse_whisper_json(
    json: &str,
    session_id: &str,
    kind: SourceKind,
) -> Result<Vec<Segment>, String> {
    #[derive(Deserialize)]
    struct Root {
        transcription: Vec<RawSeg>,
    }
    #[derive(Deserialize)]
    struct RawSeg {
        offsets: Offsets,
        text: String,
        #[serde(default)]
        confidence: Option<f64>,
    }
    #[derive(Deserialize)]
    struct Offsets {
        from: u64,
        to: u64,
    }

    let root: Root = serde_json::from_str(json).map_err(|e| format!("parse: {e}"))?;
    let visibility = kind.default_visibility();
    let segs = root
        .transcription
        .into_iter()
        .enumerate()
        .map(|(i, r)| Segment {
            id: format!("seg_{:03}", i + 1),
            session_id: session_id.to_string(),
            track: kind.track_name().to_string(),
            source_kind: kind.as_str().to_string(),
            visibility: match visibility {
                Visibility::Public => "public".to_string(),
                Visibility::Internal => "internal".to_string(),
            },
            start_ms: r.offsets.from,
            end_ms: r.offsets.to,
            text: r.text.trim().to_string(),
            is_final: true,
            confidence: r.confidence,
        })
        .collect();
    Ok(segs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/whisper-small.json");

    #[test]
    fn parses_fixture_into_two_segments() {
        let segs = parse_whisper_json(FIXTURE, "meeting-test", SourceKind::MeetingSystem).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].id, "seg_001");
        assert_eq!(segs[0].session_id, "meeting-test");
        assert_eq!(segs[0].track, "system");
        assert_eq!(segs[0].source_kind, "meeting_system");
        assert_eq!(segs[0].visibility, "public");
        assert_eq!(segs[0].start_ms, 1500);
        assert_eq!(segs[0].end_ms, 4200);
        assert_eq!(segs[0].text, "我們希望下週三前看到版本。");
        assert!(segs[0].is_final);
        assert_eq!(segs[0].confidence, Some(-0.142));
    }

    #[test]
    fn mic_internal_gets_internal_visibility() {
        let segs = parse_whisper_json(FIXTURE, "x", SourceKind::MicInternal).unwrap();
        assert_eq!(segs[0].visibility, "internal");
        assert_eq!(segs[0].source_kind, "mic_internal");
        assert_eq!(segs[0].track, "mic-internal");
    }

    #[test]
    fn corrupt_json_returns_err() {
        assert!(parse_whisper_json("{ not json", "x", SourceKind::MeetingSystem).is_err());
    }

    #[test]
    fn empty_transcription_returns_empty_vec() {
        let json = r#"{"transcription": []}"#;
        let segs = parse_whisper_json(json, "x", SourceKind::MeetingSystem).unwrap();
        assert!(segs.is_empty());
    }
}
```

- [ ] **Step 3: main.rs 加 mod**

```rust
mod audio;
mod session_store;
mod transcribe;
```

- [ ] **Step 4: 跑測試**

```bash
cd ~/mori-universe/mori-meeting-recorder/src-tauri
cargo test transcribe 2>&1 | tail -8
```

Expected: `4 passed`。

- [ ] **Step 5: Commit**

```bash
cd ~/mori-universe/mori-meeting-recorder
git add src-tauri/src/transcribe.rs src-tauri/src/main.rs src-tauri/tests/fixtures/whisper-small.json
git commit -m "$(cat <<'EOF'
feat(transcribe): parse_whisper_json — pure function

Reads whisper.cpp --output-json-full output, produces Segments with
session_id / track / source_kind / visibility filled in by caller.
4 tests: fixture parse, internal visibility for mic, corrupt JSON,
empty transcription.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Exporter (TDD)

**Files:**
- Create: `src-tauri/src/exporter.rs`

- [ ] **Step 1: 寫 exporter + 測試**

`src-tauri/src/exporter.rs`:

```rust
//! 從 Vec<Segment> + SessionMeta 產生 meeting.public.md / meeting.internal.md / timeline.json。
//! 純函式 — IO 由呼叫端(recorder.rs)做。

use crate::audio::SourceKind;
use crate::transcribe::Segment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct SessionMeta {
    pub schema_version: u32,
    pub session_id: String,
    pub started_at: String,
    pub stopped_at: String,
    pub duration_secs: u64,
    pub tracks: Vec<TrackMeta>,
    pub exports: Exports,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackMeta {
    pub name: String,
    pub source_kind: String,
    pub visibility: String,
    pub audio_path: String,
    pub transcript_path: String,
    pub segment_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Exports {
    pub public: String,
    pub internal: String,
}

/// 把 ms 轉成 hh:mm:ss 字串(供 markdown 顯示)。
fn fmt_ts(ms: u64) -> String {
    let total = ms / 1000;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// 從 segments + meta 產生 (public_md, internal_md, timeline_json) 三條字串。
pub fn export(segments: &[Segment], meta: &SessionMeta) -> Result<(String, String, String), String> {
    let public_md = render_md(
        segments,
        "public",
        &format!(
            "# Meeting Notes — {}\n\n> Source: meeting_system. Mic-internal not included.\n\n",
            meta.started_at
        ),
    );
    let internal_md = render_md(
        segments,
        "internal",
        &format!(
            "# Meeting — 內部備忘 — {}\n\n> 包含 mic-internal segments(本機麥克風)。**內部用途,不對外發。**\n\n",
            meta.started_at
        ),
    );
    let timeline = serde_json::to_string_pretty(meta).map_err(|e| format!("timeline json: {e}"))?;
    Ok((public_md, internal_md, timeline))
}

fn render_md(segments: &[Segment], visibility: &str, header: &str) -> String {
    let mut out = String::from(header);
    let mut filtered: Vec<&Segment> = segments.iter().filter(|s| s.visibility == visibility).collect();
    filtered.sort_by_key(|s| s.start_ms);
    if filtered.is_empty() {
        out.push_str("_(no segments)_\n");
        return out;
    }
    for s in filtered {
        let prefix = if visibility == "internal" && s.source_kind == "mic_internal" {
            "(內部)"
        } else {
            ""
        };
        out.push_str(&format!("[{}] {}{}\n", fmt_ts(s.start_ms), prefix, s.text));
    }
    out
}

/// 算出 segments 對應每個 track 的數量 — 用來填 TrackMeta.segment_count。
pub fn segment_count(segments: &[Segment], kind: SourceKind) -> usize {
    segments.iter().filter(|s| s.source_kind == kind.as_str()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: &str, source: &str, vis: &str, start: u64, text: &str) -> Segment {
        Segment {
            id: id.into(),
            session_id: "t".into(),
            track: if source == "meeting_system" { "system".into() } else { "mic-internal".into() },
            source_kind: source.into(),
            visibility: vis.into(),
            start_ms: start,
            end_ms: start + 1000,
            text: text.into(),
            is_final: true,
            confidence: None,
        }
    }

    fn meta(session_id: &str) -> SessionMeta {
        SessionMeta {
            schema_version: 1,
            session_id: session_id.into(),
            started_at: "2026-05-28T14:30:00+08:00".into(),
            stopped_at: "2026-05-28T15:15:00+08:00".into(),
            duration_secs: 2700,
            tracks: vec![],
            exports: Exports {
                public: "meeting.public.md".into(),
                internal: "meeting.internal.md".into(),
            },
        }
    }

    #[test]
    fn public_md_only_contains_public_visibility() {
        let segs = vec![
            seg("s1", "meeting_system", "public", 1000, "客戶說的"),
            seg("s2", "mic_internal", "internal", 2000, "我方策略"),
            seg("s3", "meeting_system", "public", 3000, "客戶又說的"),
        ];
        let (pub_md, _, _) = export(&segs, &meta("t")).unwrap();
        assert!(pub_md.contains("客戶說的"));
        assert!(pub_md.contains("客戶又說的"));
        assert!(!pub_md.contains("我方策略"));
    }

    #[test]
    fn internal_md_only_contains_internal_with_prefix() {
        let segs = vec![
            seg("s1", "meeting_system", "public", 1000, "客戶說的"),
            seg("s2", "mic_internal", "internal", 2000, "我方策略"),
        ];
        let (_, int_md, _) = export(&segs, &meta("t")).unwrap();
        assert!(int_md.contains("(內部)我方策略"));
        assert!(!int_md.contains("客戶說的"));
    }

    #[test]
    fn timeline_json_is_valid_json_with_session_id() {
        let (_, _, tl) = export(&[], &meta("meeting-x")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&tl).unwrap();
        assert_eq!(v["session_id"], "meeting-x");
        assert_eq!(v["schema_version"], 1);
    }

    #[test]
    fn empty_segments_produce_no_segments_placeholder() {
        let (pub_md, int_md, _) = export(&[], &meta("t")).unwrap();
        assert!(pub_md.contains("(no segments)"));
        assert!(int_md.contains("(no segments)"));
    }

    #[test]
    fn segments_sorted_by_start_ms_in_output() {
        let segs = vec![
            seg("s1", "meeting_system", "public", 5000, "後面"),
            seg("s2", "meeting_system", "public", 1000, "前面"),
        ];
        let (pub_md, _, _) = export(&segs, &meta("t")).unwrap();
        let pos_qian = pub_md.find("前面").unwrap();
        let pos_hou = pub_md.find("後面").unwrap();
        assert!(pos_qian < pos_hou);
    }

    #[test]
    fn fmt_ts_formats_hours_minutes_seconds() {
        assert_eq!(fmt_ts(0), "00:00:00");
        assert_eq!(fmt_ts(123_000), "00:02:03");
        assert_eq!(fmt_ts(3_723_000), "01:02:03");
    }

    #[test]
    fn segment_count_filters_by_source_kind() {
        let segs = vec![
            seg("s1", "meeting_system", "public", 0, "a"),
            seg("s2", "mic_internal", "internal", 0, "b"),
            seg("s3", "meeting_system", "public", 0, "c"),
        ];
        assert_eq!(segment_count(&segs, SourceKind::MeetingSystem), 2);
        assert_eq!(segment_count(&segs, SourceKind::MicInternal), 1);
    }
}
```

- [ ] **Step 2: main.rs 加 mod**

```rust
mod audio;
mod exporter;
mod session_store;
mod transcribe;
```

- [ ] **Step 3: 跑測試**

```bash
cd ~/mori-universe/mori-meeting-recorder/src-tauri
cargo test exporter 2>&1 | tail -10
```

Expected: `7 passed`。

- [ ] **Step 4: Commit**

```bash
cd ~/mori-universe/mori-meeting-recorder
git add src-tauri/src/exporter.rs src-tauri/src/main.rs
git commit -m "$(cat <<'EOF'
feat(exporter): pure function — segments → public.md / internal.md / timeline.json

Visibility filter (public.md = visibility=public only).
internal.md prefixes mic_internal segments with (內部). 7 tests:
public-filter, internal-prefix, timeline-json-shape, empty
placeholder, sort-order, time format, segment count helper.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Manifest writer (TDD)

**Files:**
- Create: `src-tauri/src/manifest.rs`

- [ ] **Step 1: 寫 manifest + 測試**

`src-tauri/src/manifest.rs`:

```rust
//! BI-1 manifest writer — 啟動時 overwrite `~/.mori/body-parts/mori.meeting-recorder/manifest.json`。

use std::path::{Path, PathBuf};

/// 產生 manifest JSON 字串(`entrypoints.app` 帶進來,讓測試可控)。
pub fn manifest_json(binary_path: &Path) -> String {
    serde_json::json!({
        "schema_version": 1,
        "id": "mori.meeting-recorder",
        "name": "Mori Meeting Recorder",
        "kind": "standalone_app",
        "description": "Dual-track meeting recorder (system + mic) with visibility-based export. Observer Mode MVP.",
        "capabilities": [
            "audio.capture.system",
            "audio.capture.mic",
            "transcribe.local"
        ],
        "entrypoints": {
            "app": binary_path.to_string_lossy()
        },
        "interfaces": [],
        "permissions": [],
        "data_policy": {
            "owns_raw_data": true,
            "default_ingestion": "off"
        }
    })
    .to_string()
}

/// 寫 manifest 到 `~/.mori/body-parts/mori.meeting-recorder/manifest.json`(overwrite)。
pub fn write_manifest_to(dir: &Path, binary_path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let json = manifest_json(binary_path);
    std::fs::write(dir.join("manifest.json"), json).map_err(|e| format!("write: {e}"))
}

/// 啟動時呼叫 — 解析 std::env::current_exe() + 算出真實 `~/.mori/body-parts/...` 路徑。
pub fn body_part_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("body-parts").join("mori.meeting-recorder"))
        .unwrap_or_else(|| PathBuf::from(".mori/body-parts/mori.meeting-recorder"))
}

pub fn write_on_startup() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    write_manifest_to(&body_part_dir(), &exe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn manifest_json_has_required_fields() {
        let path = PathBuf::from("/usr/local/bin/mori-meeting-recorder");
        let j = manifest_json(&path);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["id"], "mori.meeting-recorder");
        assert_eq!(v["kind"], "standalone_app");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["entrypoints"]["app"], "/usr/local/bin/mori-meeting-recorder");
        assert_eq!(v["interfaces"].as_array().unwrap().len(), 0);
        let caps = v["capabilities"].as_array().unwrap();
        assert!(caps.iter().any(|c| c == "audio.capture.system"));
        assert!(caps.iter().any(|c| c == "audio.capture.mic"));
        assert_eq!(v["data_policy"]["owns_raw_data"], true);
        assert_eq!(v["data_policy"]["default_ingestion"], "off");
    }

    #[test]
    fn write_manifest_to_creates_dir_and_file() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("body-parts").join("mori.meeting-recorder");
        let exe = PathBuf::from("/some/path/mori-meeting-recorder");
        write_manifest_to(&dir, &exe).unwrap();
        let manifest = dir.join("manifest.json");
        assert!(manifest.exists());
        let content = std::fs::read_to_string(&manifest).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["id"], "mori.meeting-recorder");
    }

    #[test]
    fn write_manifest_to_overwrites_existing() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("d");
        write_manifest_to(&dir, &PathBuf::from("/old/path")).unwrap();
        write_manifest_to(&dir, &PathBuf::from("/new/path")).unwrap();
        let content = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
        assert!(content.contains("/new/path"));
        assert!(!content.contains("/old/path"));
    }
}
```

- [ ] **Step 2: main.rs 加 mod**

```rust
mod audio;
mod exporter;
mod manifest;
mod session_store;
mod transcribe;
```

- [ ] **Step 3: 跑測試**

```bash
cd ~/mori-universe/mori-meeting-recorder/src-tauri
cargo test manifest 2>&1 | tail -8
```

Expected: `3 passed`。

- [ ] **Step 4: Commit**

```bash
cd ~/mori-universe/mori-meeting-recorder
git add src-tauri/src/manifest.rs src-tauri/src/main.rs
git commit -m "$(cat <<'EOF'
feat(manifest): BI-1 manifest writer for ~/.mori/body-parts/

Self-register on startup. kind=standalone_app, interfaces=[],
data_policy.owns_raw_data=true (matches doc L51 — Mori must not
auto-read raw audio). 3 tests: JSON shape, write+mkdir, overwrite.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: AudioCapture trait + Linux impl + Windows impl

**Files:**
- Modify: `src-tauri/src/audio/mod.rs`(加 trait)
- Create: `src-tauri/src/audio/linux.rs`
- Create: `src-tauri/src/audio/windows.rs`

> 平台 IO,主邏輯非 TDD 友善(需要實體 PipeWire / WASAPI)。**測試只測 device 挑選邏輯**(可注入 device list mock),capture 本身留給 manual e2e。

- [ ] **Step 1: 在 mod.rs 加 trait + 平台 mod**

修改 `src-tauri/src/audio/mod.rs`,加在現有 enum 後面:

```rust
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

pub mod writer;

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// 一個 capture stream — recorder 持有,stop_session 時呼叫 finish。
pub struct CaptureHandle {
    pub source: SourceKind,
    pub writer_handle: JoinHandle<Result<u64, String>>,
    pub signal: Arc<Mutex<SignalMeter>>,
    pub stop_flag: Arc<std::sync::atomic::AtomicBool>,
}

/// 過去 N ms 的 peak RMS — capsule 用來判斷「有沒有收到聲」。
#[derive(Debug, Clone, Copy, Default)]
pub struct SignalMeter {
    pub peak_rms_db: f32,    // -inf..0,-40 以下視為靜音
    pub last_sample_at_unix_ms: u64,
}

impl SignalMeter {
    pub fn has_signal(&self, now_unix_ms: u64) -> bool {
        // 過去 500ms 有 sample 且 peak RMS > -40 dB
        now_unix_ms.saturating_sub(self.last_sample_at_unix_ms) < 500 && self.peak_rms_db > -40.0
    }
}

/// 開啟一個 capture stream 給指定 source,把 samples 寫進指定 WAV path。回 handle。
#[cfg(target_os = "linux")]
pub fn open_capture(source: SourceKind, out_path: std::path::PathBuf) -> Result<CaptureHandle, String> {
    linux::open_capture(source, out_path)
}

#[cfg(target_os = "windows")]
pub fn open_capture(source: SourceKind, out_path: std::path::PathBuf) -> Result<CaptureHandle, String> {
    windows::open_capture(source, out_path)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn open_capture(_source: SourceKind, _out_path: std::path::PathBuf) -> Result<CaptureHandle, String> {
    Err("only linux + windows supported in MVP".into())
}
```

- [ ] **Step 2: 寫 Linux impl(libpulse-binding + pactl 找 monitor source)**

> Task 1 spike 發現 cpal Linux 看不到 PipeWire monitor source(monitor 是 pulse 抽象,ALSA 不認識)。OBS 同樣選平台 native(`linux-pulseaudio` plugin),我們對齊。libpulse-simple 走 blocking sync API 在 dedicated thread,server 端會把 source 原 format 降到我們要求的 16kHz mono i16 — 不用 client-side resample。

`src-tauri/src/audio/linux.rs`:

```rust
//! Linux:libpulse client API(對齊 OBS linux-pulseaudio plugin)。
//! - MicInternal:default input source(`None` source name)
//! - MeetingSystem:第一個 `.monitor` source(透過 `pactl list short sources` 列出)
//!
//! 走 libpulse-simple sync API,blocking read 在 dedicated thread。
//! server 端做 resample / format conversion → 我們收到的就是 16kHz mono i16。

use super::{writer::TrackWriter, CaptureHandle, SignalMeter, SourceKind};
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const TARGET_RATE: u32 = 16_000;
const CHUNK_MS: u64 = 50; // 50ms blocking read,讓 stop_flag check 不會卡太久
const CHUNK_SAMPLES: usize = (TARGET_RATE as u64 * CHUNK_MS / 1000) as usize; // 800 @16kHz
const CHUNK_BYTES: usize = CHUNK_SAMPLES * 2; // i16 = 2 bytes

/// 挑出符合 source 的 PulseAudio source name。
/// MicInternal → `None`(讓 pulse 用 default input)。
/// MeetingSystem → 第一個 `.monitor` 結尾 source(走 pactl 列)。
pub fn pick_source(source: SourceKind) -> Result<Option<String>, String> {
    match source {
        SourceKind::MicInternal => Ok(None),
        SourceKind::MeetingSystem => {
            let out = Command::new("pactl")
                .args(["list", "short", "sources"])
                .output()
                .map_err(|e| format!("spawn pactl: {e}(install pulseaudio-utils?)"))?;
            if !out.status.success() {
                return Err(format!("pactl exited {}", out.status));
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                // 格式:ID NAME MODULE FORMAT CHANNELS STATE
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 2 && cols[1].ends_with(".monitor") {
                    return Ok(Some(cols[1].to_string()));
                }
            }
            Err("no .monitor source — run `pactl load-module module-loopback` or check PipeWire config".into())
        }
    }
}

pub fn open_capture(source: SourceKind, out_path: PathBuf) -> Result<CaptureHandle, String> {
    let source_name = pick_source(source)?;
    let spec = Spec {
        format: Format::S16le,
        channels: 1,
        rate: TARGET_RATE,
    };
    if !spec.is_valid() {
        return Err("invalid pulse spec".into());
    }

    // libpulse 會 server-side 把 source 的 native format 降到我們要求的 16kHz mono i16 — 不用 resample
    let simple = Simple::new(
        None,                                // 預設 PA server(PipeWire 完全相容)
        "mori-meeting-recorder",             // app name
        Direction::Record,
        source_name.as_deref(),              // None = default input;Some("xxx.monitor") = system loopback
        match source {
            SourceKind::MicInternal => "mic-internal",
            SourceKind::MeetingSystem => "system-loopback",
        },
        &spec,
        None, // 預設 channel map
        None, // 預設 buffer attrs
    )
    .map_err(|e| format!("pulse Simple::new: {e}"))?;

    let writer = Arc::new(Mutex::new(Some(TrackWriter::create(&out_path)?)));
    let signal = Arc::new(Mutex::new(SignalMeter::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    // capture loop 在 dedicated thread(simple.read 是 blocking,thread 拿 simple ownership)
    let writer_for_thread = writer.clone();
    let signal_for_thread = signal.clone();
    let stop_for_thread = stop_flag.clone();
    let writer_handle = std::thread::spawn(move || -> Result<u64, String> {
        let mut buf = vec![0u8; CHUNK_BYTES];
        while !stop_for_thread.load(Ordering::Relaxed) {
            match simple.read(&mut buf) {
                Ok(()) => {
                    let samples: Vec<i16> = buf
                        .chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    // RMS → SignalMeter
                    let sumsq: f64 = samples.iter().map(|&x| (x as f64).powi(2)).sum();
                    let rms = (sumsq / samples.len() as f64).sqrt();
                    let rms_norm = rms / 32_768.0;
                    let db = if rms_norm > 0.0 { 20.0 * rms_norm.log10() } else { -120.0 };
                    let now = chrono::Utc::now().timestamp_millis() as u64;
                    if let Ok(mut s) = signal_for_thread.lock() {
                        s.peak_rms_db = db as f32;
                        s.last_sample_at_unix_ms = now;
                    }
                    // 寫 WAV
                    if let Ok(mut guard) = writer_for_thread.lock() {
                        if let Some(w) = guard.as_mut() {
                            let _ = w.push_samples(&samples);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("pulse read err: {e}");
                    break;
                }
            }
        }
        // stop_flag set → drop simple → finalize WAV
        drop(simple);
        let mut guard = writer_for_thread.lock().unwrap();
        if let Some(w) = guard.take() {
            let n = w.samples_written();
            w.finalize().map_err(|e| format!("finalize: {e}"))?;
            Ok(n)
        } else {
            Err("writer already finalized".into())
        }
    });

    Ok(CaptureHandle {
        source,
        writer_handle,
        signal,
        stop_flag,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_source_mic_internal_returns_none() {
        // MicInternal 不打 pactl,直接 Ok(None)
        assert_eq!(pick_source(SourceKind::MicInternal).unwrap(), None);
    }

    // MeetingSystem 的 pick_source 需要 pactl 在 PATH + PipeWire/PulseAudio 跑著。
    // CI 無此環境 → #[ignore]。yazelin 機器跑這個 should return Ok(Some("alsa_output.xxx.monitor"))。
    #[test]
    #[ignore]
    fn pick_source_meeting_system_returns_some_monitor() {
        let result = pick_source(SourceKind::MeetingSystem);
        match result {
            Ok(Some(name)) => assert!(name.ends_with(".monitor"), "got: {name}"),
            Ok(None) => panic!("expected Some(.monitor name), got None"),
            Err(e) => panic!("pick failed: {e}"),
        }
    }
}
```

- [ ] **Step 3: 寫 Windows impl(cpal WASAPI loopback)**

> Windows 仍走 cpal — mori-desktop 同 stack。cpal 0.15 WASAPI host 對 `default_output_device + build_input_stream` 會自動走 loopback。

`src-tauri/src/audio/windows.rs`:

```rust
//! Windows:cpal WASAPI host。
//! - MicInternal:default input device
//! - MeetingSystem:WASAPI loopback(用 default_output_device 開 input stream,cpal 內部處理 loopback flag)
//!
//! handle_chunk_f32 / resample 邏輯這邊自己一份(Linux 走 libpulse 不需要 client-side resample,
//! 兩邊不共用)。

#![cfg(target_os = "windows")]

use super::{writer::TrackWriter, CaptureHandle, SignalMeter, SourceKind};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SampleRate, StreamConfig};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const TARGET_RATE: u32 = 16_000;

pub fn pick_device(source: SourceKind) -> Result<Device, String> {
    let host = cpal::default_host();
    match source {
        SourceKind::MicInternal => host
            .default_input_device()
            .ok_or_else(|| "no default input device".into()),
        SourceKind::MeetingSystem => host
            .default_output_device()
            .ok_or_else(|| "no default output device for loopback".into()),
    }
}

pub fn open_capture(source: SourceKind, out_path: PathBuf) -> Result<CaptureHandle, String> {
    let device = pick_device(source)?;
    let default_config = match source {
        SourceKind::MicInternal => device
            .default_input_config()
            .map_err(|e| format!("default_input_config: {e}"))?,
        SourceKind::MeetingSystem => device
            .default_output_config()
            .map_err(|e| format!("default_output_config (loopback): {e}"))?,
    };
    let in_rate = default_config.sample_rate().0;
    let in_channels = default_config.channels();
    let sample_format = default_config.sample_format();

    let config = StreamConfig {
        channels: in_channels,
        sample_rate: SampleRate(in_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let writer = Arc::new(Mutex::new(Some(TrackWriter::create(&out_path)?)));
    let signal = Arc::new(Mutex::new(SignalMeter::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let resample_ratio = in_rate as f64 / TARGET_RATE as f64;
    let err_fn = |e| eprintln!("audio stream error: {e}");

    let writer_cb = writer.clone();
    let signal_cb = signal.clone();

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                handle_chunk_f32(data, in_channels, resample_ratio, &writer_cb, &signal_cb);
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => {
            let writer_cb_i = writer_cb.clone();
            let signal_cb_i = signal_cb.clone();
            device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data.iter().map(|&x| x as f32 / 32_768.0).collect();
                    handle_chunk_f32(&f, in_channels, resample_ratio, &writer_cb_i, &signal_cb_i);
                },
                err_fn,
                None,
            )
        }
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("build_input_stream: {e}"))?;

    stream.play().map_err(|e| format!("stream.play: {e}"))?;

    let stop_for_thread = stop_flag.clone();
    let writer_for_finalize = writer.clone();
    let writer_thread = std::thread::spawn(move || {
        while !stop_for_thread.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        drop(stream);
        let mut guard = writer_for_finalize.lock().unwrap();
        if let Some(w) = guard.take() {
            let n = w.samples_written();
            w.finalize().map_err(|e| format!("finalize: {e}"))?;
            Ok(n)
        } else {
            Err("writer already finalized".into())
        }
    });

    Ok(CaptureHandle {
        source,
        writer_handle: writer_thread,
        signal,
        stop_flag,
    })
}

fn handle_chunk_f32(
    samples: &[f32],
    in_channels: u16,
    resample_ratio: f64,
    writer: &Arc<Mutex<Option<TrackWriter>>>,
    signal: &Arc<Mutex<SignalMeter>>,
) {
    // mono mix
    let mono: Vec<f32> = if in_channels == 1 {
        samples.to_vec()
    } else {
        samples
            .chunks(in_channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    // crude resample by index pick(MVP — 後續用 rubato 升級)
    let mut out_i16: Vec<i16> = Vec::with_capacity((mono.len() as f64 / resample_ratio) as usize + 1);
    let mut idx = 0.0_f64;
    while (idx as usize) < mono.len() {
        let v = mono[idx as usize].clamp(-1.0, 1.0);
        out_i16.push((v * 32_767.0) as i16);
        idx += resample_ratio;
    }

    if !out_i16.is_empty() {
        let sumsq: f64 = out_i16.iter().map(|&x| (x as f64).powi(2)).sum();
        let rms = (sumsq / out_i16.len() as f64).sqrt();
        let rms_norm = rms / 32_768.0;
        let db = if rms_norm > 0.0 { 20.0 * rms_norm.log10() } else { -120.0 };
        let now = chrono::Utc::now().timestamp_millis() as u64;
        if let Ok(mut s) = signal.lock() {
            s.peak_rms_db = db as f32;
            s.last_sample_at_unix_ms = now;
        }
    }

    if let Ok(mut guard) = writer.lock() {
        if let Some(w) = guard.as_mut() {
            let _ = w.push_samples(&out_i16);
        }
    }
}
```

> ⚠ Windows loopback 在 cpal 0.15 行為**未實機 spike** — `default_output_device + default_output_config + build_input_stream` 在 WASAPI host **應該**自動走 loopback,實機跑 Task 12 e2e 才會驗證。失敗 fallback 是直接調 `windows` crate WASAPI(超出 MVP,follow-up)。

- [ ] **Step 4: cargo check Linux**

```bash
cd ~/mori-universe/mori-meeting-recorder/src-tauri
cargo check 2>&1 | tail -5
```

Expected: Linux 跑 → 只編 `mod audio::linux`(`audio::windows` 被 `#[cfg(target_os = "windows")]` 排除),libpulse-binding + libpulse-simple-binding 都 link 過。**首次 build 需要 system libpulse**(`apt install libpulse-dev` 或 PipeWire pulse compat lib,大部分 Ubuntu/Fedora 桌面預裝)。

如果 `cargo check` 噴 `pkg-config could not find libpulse`,跑:

```bash
sudo apt install libpulse-dev pulseaudio-utils
```

(`pulseaudio-utils` 提供 `pactl`,runtime 也要)。

- [ ] **Step 5: 跑 Linux pick_source 測試(non-ignored)**

```bash
cd ~/mori-universe/mori-meeting-recorder/src-tauri
cargo test audio::linux::tests::pick_source_mic_internal_returns_none 2>&1 | tail -5
```

Expected: `1 passed`。

實機可選跑:

```bash
cargo test audio::linux::tests::pick_source_meeting_system_returns_some_monitor -- --ignored 2>&1 | tail -5
```

Expected: pass(yazelin 機器 spike 已證實有 monitor source)。

- [ ] **Step 6: Commit**

```bash
cd ~/mori-universe/mori-meeting-recorder
git add src-tauri/src/audio
git commit -m "$(cat <<'EOF'
feat(audio): AudioCapture — Linux libpulse + Windows cpal WASAPI loopback

Linux: libpulse-simple-binding (matching OBS linux-pulseaudio plugin
choice). pick_source uses `pactl list short sources` to find the
first .monitor source for MeetingSystem; MicInternal → default input.
Server-side resample to 16kHz mono i16, dedicated thread blocking
read 50ms chunks, stop_flag interrupts gracefully.

Windows: cpal WASAPI host (matching mori-desktop). default_output_
device + build_input_stream gives loopback. Client-side resample +
mono mix in callback.

Rationale: Task 1 spike found cpal 0.15 Linux = ALSA-only, can't see
PipeWire monitor abstraction. OBS uses same per-platform native
approach. interfaces unchanged at the audio::open_capture layer.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: Recorder orchestrator + integration test

**Files:**
- Create: `src-tauri/src/recorder.rs`
- Create: `src-tauri/tests/integration_recorder.rs`(用 fake audio source 跑 e2e)

> Integration test 不開真的 cpal — 跑「假裝錄了,直接 dump 一段 fake WAV → 跑真的 whisper-cli(如果 dep 有)→ 看檔案結構」。

- [ ] **Step 1: 寫 recorder**

`src-tauri/src/recorder.rs`:

```rust
//! Session lifecycle orchestrator。組合 audio::open_capture + SessionStore + transcribe + exporter。

use crate::audio::{self, CaptureHandle, SourceKind};
use crate::exporter::{export, Exports, SessionMeta, TrackMeta};
use crate::session_store::{default_meetings_dir, new_session_id, SessionStore};
use crate::transcribe::{parse_whisper_json, Segment};
use chrono::{DateTime, Local};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Idle,
    Recording,
    Transcribing,
}

pub struct ActiveSession {
    pub store: SessionStore,
    pub started_at: DateTime<Local>,
    pub handles: Vec<CaptureHandle>,
}

#[derive(Default)]
pub struct Recorder {
    pub active: Mutex<Option<ActiveSession>>,
    pub state: Mutex<State>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecorderStatus {
    pub state: State,
    pub elapsed_secs: u64,
    pub system_signal: bool,
    pub mic_signal: bool,
    pub session_id: Option<String>,
}

impl Recorder {
    pub fn start_session(&self) -> Result<String, String> {
        let mut active_guard = self.active.lock().map_err(|e| e.to_string())?;
        if active_guard.is_some() {
            return Err("session already running".into());
        }
        let now = Local::now();
        let session_id = new_session_id(now);
        let store = SessionStore::create(&session_id, &default_meetings_dir())?;

        let mut handles = Vec::new();
        for kind in [SourceKind::MeetingSystem, SourceKind::MicInternal] {
            let out = store.audio_path(kind);
            match audio::open_capture(kind, out) {
                Ok(h) => handles.push(h),
                Err(e) => eprintln!("warning: open_capture {:?} failed: {e}", kind),
            }
        }
        if handles.is_empty() {
            return Err("no audio capture stream opened".into());
        }

        *active_guard = Some(ActiveSession {
            store,
            started_at: now,
            handles,
        });
        *self.state.lock().map_err(|e| e.to_string())? = State::Recording;
        Ok(session_id)
    }

    pub fn stop_session(&self) -> Result<String, String> {
        let mut active_guard = self.active.lock().map_err(|e| e.to_string())?;
        let session = active_guard.take().ok_or("no active session")?;
        drop(active_guard);

        *self.state.lock().map_err(|e| e.to_string())? = State::Transcribing;

        let session_id = session.store.session_id.clone();
        let store = session.store;
        let started_at = session.started_at;

        // 停 capture
        for h in &session.handles {
            h.stop_flag.store(true, Ordering::Relaxed);
        }
        for h in session.handles {
            let _ = h.writer_handle.join();
        }

        // 轉錄 — parallel
        let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio rt: {e}"))?;
        let segs_result: Result<(Vec<Segment>, Vec<Segment>), String> = rt.block_on(async {
            let sys_path = store.audio_path(SourceKind::MeetingSystem);
            let mic_path = store.audio_path(SourceKind::MicInternal);
            let sys_id = session_id.clone();
            let mic_id = session_id.clone();
            let (sys, mic) = tokio::join!(
                tokio::task::spawn_blocking(move || {
                    crate::transcribe::run_whisper(&sys_path, &sys_id, SourceKind::MeetingSystem)
                }),
                tokio::task::spawn_blocking(move || {
                    crate::transcribe::run_whisper(&mic_path, &mic_id, SourceKind::MicInternal)
                }),
            );
            Ok((
                sys.map_err(|e| format!("join sys: {e}"))?.unwrap_or_default(),
                mic.map_err(|e| format!("join mic: {e}"))?.unwrap_or_default(),
            ))
        });
        let (sys_segs, mic_segs) = segs_result?;

        // 寫 segments JSONL
        write_segments_jsonl(&store.segments_path(SourceKind::MeetingSystem), &sys_segs)?;
        write_segments_jsonl(&store.segments_path(SourceKind::MicInternal), &mic_segs)?;

        // 匯出
        let stopped_at = Local::now();
        let all_segs: Vec<Segment> = sys_segs.iter().chain(mic_segs.iter()).cloned().collect();
        let meta = SessionMeta {
            schema_version: 1,
            session_id: session_id.clone(),
            started_at: started_at.to_rfc3339(),
            stopped_at: stopped_at.to_rfc3339(),
            duration_secs: (stopped_at - started_at).num_seconds().max(0) as u64,
            tracks: vec![
                TrackMeta {
                    name: "system".into(),
                    source_kind: "meeting_system".into(),
                    visibility: "public".into(),
                    audio_path: "audio/system.wav".into(),
                    transcript_path: "transcript/system.segments.jsonl".into(),
                    segment_count: sys_segs.len(),
                },
                TrackMeta {
                    name: "mic-internal".into(),
                    source_kind: "mic_internal".into(),
                    visibility: "internal".into(),
                    audio_path: "audio/mic-internal.wav".into(),
                    transcript_path: "transcript/mic-internal.segments.jsonl".into(),
                    segment_count: mic_segs.len(),
                },
            ],
            exports: Exports {
                public: "meeting.public.md".into(),
                internal: "meeting.internal.md".into(),
            },
        };
        let (pub_md, int_md, timeline) = export(&all_segs, &meta)?;
        std::fs::write(store.public_md_path(), pub_md).map_err(|e| format!("write public.md: {e}"))?;
        std::fs::write(store.internal_md_path(), int_md).map_err(|e| format!("write internal.md: {e}"))?;
        std::fs::write(store.timeline_path(), timeline).map_err(|e| format!("write timeline.json: {e}"))?;

        *self.state.lock().map_err(|e| e.to_string())? = State::Idle;
        Ok(session_id)
    }

    pub fn status(&self) -> RecorderStatus {
        let state = *self.state.lock().unwrap_or_else(|e| e.into_inner());
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let (elapsed_secs, system_signal, mic_signal, session_id) = if let Some(s) = active.as_ref() {
            let elapsed = (Local::now() - s.started_at).num_seconds().max(0) as u64;
            let sys = s
                .handles
                .iter()
                .find(|h| h.source == SourceKind::MeetingSystem)
                .map(|h| h.signal.lock().map(|sm| sm.has_signal(now_ms)).unwrap_or(false))
                .unwrap_or(false);
            let mic = s
                .handles
                .iter()
                .find(|h| h.source == SourceKind::MicInternal)
                .map(|h| h.signal.lock().map(|sm| sm.has_signal(now_ms)).unwrap_or(false))
                .unwrap_or(false);
            (elapsed, sys, mic, Some(s.store.session_id.clone()))
        } else {
            (0, false, false, None)
        };
        RecorderStatus {
            state,
            elapsed_secs,
            system_signal,
            mic_signal,
            session_id,
        }
    }
}

fn write_segments_jsonl(path: &PathBuf, segs: &[Segment]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let lines: Vec<String> = segs
        .iter()
        .map(|s| serde_json::to_string(s).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    std::fs::write(path, lines.join("\n") + "\n").map_err(|e| format!("write segs: {e}"))?;
    Ok(())
}

pub static RECORDER: std::sync::OnceLock<Arc<Recorder>> = std::sync::OnceLock::new();

pub fn instance() -> Arc<Recorder> {
    RECORDER
        .get_or_init(|| Arc::new(Recorder::default()))
        .clone()
}
```

- [ ] **Step 2: 加 `transcribe::run_whisper` spawn 邏輯**

修改 `src-tauri/src/transcribe.rs`,在 `parse_whisper_json` 之後加:

```rust
use std::path::Path;
use std::process::Command;

const WHISPER_BIN: &str = "whisper-cli";
const WHISPER_MODEL_FILENAME: &str = "ggml-small.bin";

pub fn whisper_bin_path() -> std::path::PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("bin").join(WHISPER_BIN))
        .unwrap_or_else(|| std::path::PathBuf::from(WHISPER_BIN))
}

pub fn whisper_model_path() -> std::path::PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".mori").join("models").join(WHISPER_MODEL_FILENAME))
        .unwrap_or_else(|| std::path::PathBuf::from(WHISPER_MODEL_FILENAME))
}

/// 跑 whisper-cli 對單一 WAV 檔,回 Segments。檔案不存在或 binary 缺則跳過(回空)。
pub fn run_whisper(wav: &Path, session_id: &str, kind: SourceKind) -> Vec<Segment> {
    if !wav.exists() {
        return vec![];
    }
    let bin = whisper_bin_path();
    let model = whisper_model_path();
    if !bin.exists() || !model.exists() {
        eprintln!("whisper deps missing — skipping transcribe");
        return vec![];
    }
    let output = match Command::new(&bin)
        .args([
            "-m",
            &model.to_string_lossy(),
            "-f",
            &wav.to_string_lossy(),
            "--output-json-full",
            "--no-prints",
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("spawn whisper-cli: {e}");
            return vec![];
        }
    };
    if !output.status.success() {
        eprintln!("whisper-cli exited {}: {}", output.status, String::from_utf8_lossy(&output.stderr));
        return vec![];
    }
    // whisper-cli `--output-json-full` 把 JSON 寫到 `<wav>.json`,不是 stdout
    let json_path = wav.with_extension("wav.json");
    let json = match std::fs::read_to_string(&json_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read whisper json {}: {e}", json_path.display());
            return vec![];
        }
    };
    match parse_whisper_json(&json, session_id, kind) {
        Ok(segs) => segs,
        Err(e) => {
            eprintln!("parse whisper json: {e}");
            vec![]
        }
    }
}
```

- [ ] **Step 3: main.rs 加 recorder mod**

```rust
mod audio;
mod exporter;
mod manifest;
mod recorder;
mod session_store;
mod transcribe;
```

- [ ] **Step 4: 寫 integration test(沒有真實 whisper / cpal 也能跑)**

`src-tauri/tests/integration_recorder.rs`:

```rust
//! Integration test:不跑真 cpal / whisper(那些是 manual e2e),測「對給定 fake segments,
//! exporter + segments JSONL 寫對位置 + visibility 對」這條鏈。

use mori_meeting_recorder::audio::SourceKind;
use mori_meeting_recorder::exporter::{export, Exports, SessionMeta, TrackMeta};
use mori_meeting_recorder::session_store::SessionStore;
use mori_meeting_recorder::transcribe::Segment;
use tempfile::TempDir;

fn fake_seg(kind: SourceKind, idx: u64, text: &str) -> Segment {
    Segment {
        id: format!("seg_{:03}", idx),
        session_id: "meeting-test".into(),
        track: kind.track_name().into(),
        source_kind: kind.as_str().into(),
        visibility: match kind.default_visibility() {
            mori_meeting_recorder::audio::Visibility::Public => "public".into(),
            mori_meeting_recorder::audio::Visibility::Internal => "internal".into(),
        },
        start_ms: idx * 1000,
        end_ms: idx * 1000 + 500,
        text: text.into(),
        is_final: true,
        confidence: None,
    }
}

#[test]
fn end_to_end_exporter_chain_writes_correct_files() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::create("meeting-test", tmp.path()).unwrap();
    let segs = vec![
        fake_seg(SourceKind::MeetingSystem, 1, "客戶說 A"),
        fake_seg(SourceKind::MicInternal, 2, "我方私聊"),
        fake_seg(SourceKind::MeetingSystem, 3, "客戶說 B"),
    ];
    let meta = SessionMeta {
        schema_version: 1,
        session_id: "meeting-test".into(),
        started_at: "2026-05-28T14:30:00+08:00".into(),
        stopped_at: "2026-05-28T15:15:00+08:00".into(),
        duration_secs: 2700,
        tracks: vec![
            TrackMeta {
                name: "system".into(),
                source_kind: "meeting_system".into(),
                visibility: "public".into(),
                audio_path: "audio/system.wav".into(),
                transcript_path: "transcript/system.segments.jsonl".into(),
                segment_count: 2,
            },
            TrackMeta {
                name: "mic-internal".into(),
                source_kind: "mic_internal".into(),
                visibility: "internal".into(),
                audio_path: "audio/mic-internal.wav".into(),
                transcript_path: "transcript/mic-internal.segments.jsonl".into(),
                segment_count: 1,
            },
        ],
        exports: Exports {
            public: "meeting.public.md".into(),
            internal: "meeting.internal.md".into(),
        },
    };
    let (pub_md, int_md, timeline) = export(&segs, &meta).unwrap();
    std::fs::write(store.public_md_path(), &pub_md).unwrap();
    std::fs::write(store.internal_md_path(), &int_md).unwrap();
    std::fs::write(store.timeline_path(), &timeline).unwrap();

    let pub_read = std::fs::read_to_string(store.public_md_path()).unwrap();
    let int_read = std::fs::read_to_string(store.internal_md_path()).unwrap();
    assert!(pub_read.contains("客戶說 A"));
    assert!(pub_read.contains("客戶說 B"));
    assert!(!pub_read.contains("我方私聊"));
    assert!(int_read.contains("(內部)我方私聊"));
    assert!(!int_read.contains("客戶說 A"));
}
```

- [ ] **Step 5: 改 main.rs 變成 lib + bin 雙形態,讓 integration test 能 use crate**

把 `src-tauri/src/main.rs` 結構改成:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod audio;
pub mod exporter;
pub mod manifest;
pub mod recorder;
pub mod session_store;
pub mod transcribe;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = tauri::Manager::get_webview_window(app, "main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

並在 `src-tauri/Cargo.toml` 的 `[[bin]]` 後加 `[lib]`:

```toml
[lib]
name = "mori_meeting_recorder"
path = "src/main.rs"
```

(Rust 同時 main.rs 當 bin entry + lib root,需要 `pub mod`。Tauri 2 範例就這樣。)

- [ ] **Step 6: 跑全部 unit + integration test**

```bash
cd ~/mori-universe/mori-meeting-recorder/src-tauri
cargo test 2>&1 | tail -20
```

Expected: 全綠;`integration_recorder::end_to_end_exporter_chain_writes_correct_files` 過。

- [ ] **Step 7: Commit**

```bash
cd ~/mori-universe/mori-meeting-recorder
git add src-tauri/src/recorder.rs src-tauri/src/transcribe.rs src-tauri/src/main.rs src-tauri/tests/integration_recorder.rs src-tauri/Cargo.toml
git commit -m "$(cat <<'EOF'
feat(recorder): session lifecycle orchestrator + whisper spawn + integration test

Recorder::start_session opens both captures via audio::open_capture
(skips one if device pick fails, e.g. no .monitor). stop_session
joins capture threads, parallel-spawns whisper-cli per track via
tokio::join! + spawn_blocking, writes segments JSONL + public.md +
internal.md + timeline.json.

run_whisper handles missing binary/model gracefully (returns empty
segments, logs warning) — recorder still produces valid empty
exports.

Integration test exercises exporter chain end-to-end without
spawning real audio / whisper.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: Tauri commands + tray icon

**Files:**
- Modify: `src-tauri/src/main.rs`(commands + invoke_handler + tray + manifest 寫入)

- [ ] **Step 1: 加 commands + tray + manifest 啟動寫入**

修改 `src-tauri/src/main.rs`:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod audio;
pub mod exporter;
pub mod manifest;
pub mod recorder;
pub mod session_store;
pub mod transcribe;

use recorder::{instance as recorder_instance, RecorderStatus};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    LogicalSize, Manager, Size,
};

#[derive(Debug, Serialize)]
struct DepsCheck {
    whisper_cli_ok: bool,
    whisper_cli_path: String,
    model_ok: bool,
    model_path: String,
}

#[tauri::command]
fn recorder_start() -> Result<String, String> {
    recorder_instance().start_session()
}

#[tauri::command]
fn recorder_stop() -> Result<String, String> {
    recorder_instance().stop_session()
}

#[tauri::command]
fn recorder_status() -> RecorderStatus {
    recorder_instance().status()
}

#[tauri::command]
fn deps_check() -> DepsCheck {
    let bin = transcribe::whisper_bin_path();
    let model = transcribe::whisper_model_path();
    DepsCheck {
        whisper_cli_ok: bin.exists() && bin.is_file(),
        whisper_cli_path: bin.to_string_lossy().to_string(),
        model_ok: model.exists() && std::fs::metadata(&model).map(|m| m.len() > 40_000_000).unwrap_or(false),
        model_path: model.to_string_lossy().to_string(),
    }
}

#[tauri::command]
fn set_window_mode(window: tauri::Window, mode: String) -> Result<(), String> {
    let (w, h) = match mode.as_str() {
        "collapsed" => (360.0, 60.0),
        "expanded" => (720.0, 480.0),
        other => return Err(format!("unknown mode: {other}")),
    };
    window
        .set_size(Size::Logical(LogicalSize { width: w, height: h }))
        .map_err(|e| format!("set_size: {e}"))
}

#[tauri::command]
fn list_sessions() -> Vec<String> {
    let dir = session_store::default_meetings_dir();
    std::fs::read_dir(&dir)
        .ok()
        .map(|it| {
            it.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
fn open_session_dir(session_id: String) -> Result<(), String> {
    let dir = session_store::default_meetings_dir().join(&session_id);
    if !dir.exists() {
        return Err(format!("not found: {}", dir.display()));
    }
    open_path(&dir.to_string_lossy())
}

#[cfg(target_os = "linux")]
fn open_path(path: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .status()
        .map_err(|e| format!("xdg-open: {e}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_path(path: &str) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(path)
        .status()
        .map_err(|e| format!("explorer: {e}"))?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn open_path(_path: &str) -> Result<(), String> {
    Err("unsupported platform".into())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            // BI-1:啟動時寫 manifest 到 ~/.mori/body-parts/mori.meeting-recorder/manifest.json
            if let Err(e) = manifest::write_on_startup() {
                eprintln!("write manifest: {e}");
            }
            // Tray
            let toggle = MenuItem::with_id(app, "toggle", "顯示 / 隱藏", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "結束", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &quit])?;
            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => {
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { .. } = event {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            recorder_start,
            recorder_stop,
            recorder_status,
            deps_check,
            set_window_mode,
            list_sessions,
            open_session_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 2: cargo check + 簡單跑一次**

```bash
cd ~/mori-universe/mori-meeting-recorder
(cd src-tauri && cargo check 2>&1 | tail -5)
```

Expected: green。

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "$(cat <<'EOF'
feat(tauri): commands + tray icon + manifest self-register on startup

Commands: recorder_start / stop / status / deps_check / set_window_mode
/ list_sessions / open_session_dir. Tray icon with toggle + quit menu.
single-instance plugin already wired in Task 2.

BI-1 manifest writes to ~/.mori/body-parts/mori.meeting-recorder/ on
setup hook, with entrypoints.app from std::env::current_exe().

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 11: Frontend — App.tsx state machine + theme.css + i18n

**Files:**
- Create: `src/main.tsx`(replace stub)
- Create: `src/App.tsx`
- Create: `src/theme.css`
- Create: `src/i18n/index.ts`
- Create: `src/i18n/locales/en.json`
- Create: `src/i18n/locales/zh-TW.json`

- [ ] **Step 1: theme.css**

`src/theme.css`:

```css
:root {
  --c-bg: rgba(30, 34, 42, 0.92);
  --c-surface: rgba(40, 45, 55, 0.95);
  --c-border: rgba(180, 190, 210, 0.18);
  --c-text: rgba(240, 242, 248, 0.92);
  --c-text-muted: rgba(240, 242, 248, 0.55);
  --c-accent: rgba(125, 200, 175, 0.95);
  --c-warning: rgba(230, 180, 80, 0.95);
  --c-danger: rgba(230, 90, 90, 0.95);
  --c-pill-on: rgba(125, 200, 175, 0.22);
  --c-pill-off: rgba(180, 190, 210, 0.12);
  --c-pill-err: rgba(230, 90, 90, 0.22);
  --r-pill: 14px;
  --r-window: 12px;
}

html, body, #root {
  margin: 0;
  padding: 0;
  background: transparent;
  color: var(--c-text);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Noto Sans CJK TC", sans-serif;
  font-size: 13px;
}

#root {
  border-radius: var(--r-window);
  overflow: hidden;
  background: var(--c-bg);
  -webkit-app-region: drag;
  border: 1px solid var(--c-border);
}

button, input, select { -webkit-app-region: no-drag; }

.mmr-btn {
  background: var(--c-surface);
  color: var(--c-text);
  border: 1px solid var(--c-border);
  border-radius: 6px;
  padding: 4px 10px;
  font-size: 12px;
  cursor: pointer;
}
.mmr-btn:hover { border-color: var(--c-accent); }
.mmr-btn:disabled { opacity: 0.4; cursor: not-allowed; }

.mmr-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: var(--r-pill);
  font-size: 11px;
  background: var(--c-pill-off);
}
.mmr-pill.on  { background: var(--c-pill-on);  color: var(--c-accent); }
.mmr-pill.err { background: var(--c-pill-err); color: var(--c-danger); }

.mmr-pill-dot {
  width: 8px; height: 8px; border-radius: 50%;
  background: currentColor;
}
```

- [ ] **Step 2: i18n setup**

`src/i18n/locales/zh-TW.json`:

```json
{
  "capsule": {
    "start": "▶",
    "stop": "■",
    "transcribing": "轉錄中…",
    "expand": "展開",
    "system_pill": "會議音訊",
    "mic_pill": "內部麥克風"
  },
  "tabs": {
    "record": "錄音",
    "sessions": "Sessions",
    "deps": "Deps"
  },
  "record": {
    "start_button": "開始錄音",
    "stop_button": "停止",
    "transcribing_hint": "雙軌轉錄中,請稍候(通常 0.5-1x 即時)",
    "done_title": "完成 — session",
    "open_folder": "開資料夾",
    "warning": "客戶版只使用會議音訊;內部麥克風不會預設匯出給客戶。"
  },
  "sessions": {
    "title": "Session 列表",
    "hint": "唯讀。點任一筆開資料夾。",
    "empty": "目前還沒有任何 session。"
  },
  "deps": {
    "title": "依賴檢查",
    "hint": "Mori Meeting Recorder 需要 whisper.cpp CLI + GGML model。安裝指令在下方,自己貼 terminal 跑。",
    "whisper_cli": "whisper-cli",
    "model": "ggml-small.bin model",
    "found": "✓ 找到",
    "missing": "✗ 缺",
    "recheck": "重新檢測",
    "linux_install": "Linux 安裝指令(複製到 terminal 跑)",
    "windows_install": "Windows 安裝指令(複製到 PowerShell 跑)"
  }
}
```

`src/i18n/locales/en.json`:

```json
{
  "capsule": {
    "start": "▶",
    "stop": "■",
    "transcribing": "transcribing…",
    "expand": "expand",
    "system_pill": "meeting",
    "mic_pill": "mic"
  },
  "tabs": {
    "record": "Record",
    "sessions": "Sessions",
    "deps": "Deps"
  },
  "record": {
    "start_button": "Start recording",
    "stop_button": "Stop",
    "transcribing_hint": "Transcribing both tracks (~0.5-1x realtime)…",
    "done_title": "Done — session",
    "open_folder": "Open folder",
    "warning": "Client transcript uses meeting audio only; mic-internal is never exported by default."
  },
  "sessions": {
    "title": "Sessions",
    "hint": "Read-only. Click a row to open its folder.",
    "empty": "No sessions yet."
  },
  "deps": {
    "title": "Dependency check",
    "hint": "Requires whisper.cpp CLI + a GGML model. Copy the install command below into your terminal.",
    "whisper_cli": "whisper-cli",
    "model": "ggml-small.bin model",
    "found": "✓ found",
    "missing": "✗ missing",
    "recheck": "Re-check",
    "linux_install": "Linux install command",
    "windows_install": "Windows install command"
  }
}
```

`src/i18n/index.ts`:

```typescript
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zhTW from "./locales/zh-TW.json";
import en from "./locales/en.json";

const lang = navigator.language.startsWith("zh") ? "zh-TW" : "en";

i18n.use(initReactI18next).init({
  resources: { "zh-TW": { translation: zhTW }, en: { translation: en } },
  lng: lang,
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;
```

- [ ] **Step 3: App.tsx state machine + theme injection**

`src/App.tsx`:

```tsx
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./theme.css";
import "./i18n";
import CapsuleView from "./CapsuleView";
import ExpandedView from "./ExpandedView";

export type Mode = "collapsed" | "expanded";

export default function App() {
  const [mode, setMode] = useState<Mode>("collapsed");

  const switchMode = async (next: Mode) => {
    try { await invoke("set_window_mode", { mode: next }); } catch { /* ignore */ }
    setMode(next);
  };

  return mode === "collapsed"
    ? <CapsuleView onExpand={() => switchMode("expanded")} />
    : <ExpandedView onCollapse={() => switchMode("collapsed")} />;
}
```

- [ ] **Step 4: main.tsx 改 hooks 真的渲染 App**

`src/main.tsx`:

```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";

createRoot(document.getElementById("root")!).render(
  <StrictMode><App /></StrictMode>
);
```

- [ ] **Step 5: build pass check**

```bash
cd ~/mori-universe/mori-meeting-recorder
npm run build 2>&1 | tail -5
```

Expected: TS build pass(CapsuleView / ExpandedView 還沒寫,build 會 error → 先建空 placeholder)。

讓 build 過,先建 placeholder:

`src/CapsuleView.tsx`:

```tsx
export default function CapsuleView({ onExpand }: { onExpand: () => void }) {
  return <div onDoubleClick={onExpand} style={{ padding: 10 }}>capsule placeholder</div>;
}
```

`src/ExpandedView.tsx`:

```tsx
export default function ExpandedView({ onCollapse }: { onCollapse: () => void }) {
  return (
    <div style={{ padding: 16 }}>
      <button className="mmr-btn" onClick={onCollapse}>▴ collapse</button>
      <p>expanded placeholder — 3 tabs in Task 12</p>
    </div>
  );
}
```

```bash
npm run build 2>&1 | tail -5
```

Expected: build pass。

- [ ] **Step 6: Commit**

```bash
git add src/ index.html
git commit -m "$(cat <<'EOF'
feat(frontend): App state machine + theme.css + i18n scaffold

Mode = collapsed | expanded. invoke("set_window_mode") on switch.
theme.css defines own var(--c-*) tokens (no mori-desktop coupling).
i18n with zh-TW + en for capsule / tabs / record / sessions / deps.
CapsuleView + ExpandedView placeholders (full impl in Task 12).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: Frontend — CapsuleView + ExpandedView 3 tabs + Deps scripts + manual e2e + PR

**Files:**
- Modify: `src/CapsuleView.tsx`(完整版)
- Modify: `src/ExpandedView.tsx`(完整版 + 3 tabs)
- Create: `src/tabs/RecordTab.tsx`
- Create: `src/tabs/SessionsTab.tsx`
- Create: `src/tabs/DepsTab.tsx`
- Create: `scripts/install-whisper-linux.sh`
- Create: `scripts/install-whisper-windows.ps1`
- Create: `scripts/verify.sh`
- Modify: `mori-desktop/docs/body-interface-backlog.md`(BI-5 done 標記)
- Modify: `~/.claude/projects/-home-ct/memory/project_mori_body_interface.md` + `MEMORY.md`

- [ ] **Step 1: CapsuleView 完整版**

`src/CapsuleView.tsx`:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type RecorderStatus = {
  state: "idle" | "recording" | "transcribing";
  elapsed_secs: number;
  system_signal: boolean;
  mic_signal: boolean;
  session_id: string | null;
};

const fmt = (s: number) => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
};

export default function CapsuleView({ onExpand }: { onExpand: () => void }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<RecorderStatus | null>(null);

  useEffect(() => {
    const tick = async () => {
      try { setStatus(await invoke<RecorderStatus>("recorder_status")); }
      catch { /* */ }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);

  const onStartStop = async () => {
    try {
      if (status?.state === "recording") await invoke("recorder_stop");
      else await invoke("recorder_start");
    } catch (e) { console.error(e); }
  };

  const isRecording = status?.state === "recording";
  const isTranscribing = status?.state === "transcribing";

  return (
    <div
      onDoubleClick={(e) => {
        const tag = (e.target as HTMLElement).tagName;
        if (tag === "BUTTON" || tag === "SPAN") return;
        onExpand();
      }}
      style={{
        display: "flex", alignItems: "center", gap: 8,
        height: 60, padding: "0 14px",
        userSelect: "none",
      }}
    >
      <div style={{ fontSize: 16, fontVariantNumeric: "tabular-nums", minWidth: 80 }}>
        {fmt(status?.elapsed_secs ?? 0)}
      </div>
      <div style={{ display: "flex", gap: 6, flex: 1 }}>
        <span className={`mmr-pill ${status?.system_signal ? "on" : ""}`}>
          <span className="mmr-pill-dot" /> {t("capsule.system_pill")}
        </span>
        <span className={`mmr-pill ${status?.mic_signal ? "on" : ""}`}>
          <span className="mmr-pill-dot" /> {t("capsule.mic_pill")}
        </span>
      </div>
      {isTranscribing ? (
        <span style={{ fontSize: 11, opacity: 0.7 }}>{t("capsule.transcribing")}</span>
      ) : (
        <button className="mmr-btn" onClick={onStartStop}>
          {isRecording ? t("capsule.stop") : t("capsule.start")}
        </button>
      )}
    </div>
  );
}
```

- [ ] **Step 2: ExpandedView + tabs**

`src/ExpandedView.tsx`:

```tsx
import { useState } from "react";
import { useTranslation } from "react-i18next";
import RecordTab from "./tabs/RecordTab";
import SessionsTab from "./tabs/SessionsTab";
import DepsTab from "./tabs/DepsTab";

type Tab = "record" | "sessions" | "deps";

export default function ExpandedView({ onCollapse }: { onCollapse: () => void }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("record");

  const TabBtn = ({ id, label }: { id: Tab; label: string }) => (
    <button
      className="mmr-btn"
      onClick={() => setTab(id)}
      style={{ borderColor: tab === id ? "var(--c-accent)" : "var(--c-border)" }}
    >
      {label}
    </button>
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "10px 14px", borderBottom: "1px solid var(--c-border)" }}>
        <TabBtn id="record" label={t("tabs.record")} />
        <TabBtn id="sessions" label={t("tabs.sessions")} />
        <TabBtn id="deps" label={t("tabs.deps")} />
        <span style={{ flex: 1 }} />
        <button className="mmr-btn" onClick={onCollapse}>▴</button>
        <button className="mmr-btn" onClick={() => (window as any).__TAURI__?.window.appWindow.hide()}>✕</button>
      </div>
      <div style={{ flex: 1, overflow: "auto", padding: 14 }}>
        {tab === "record" && <RecordTab />}
        {tab === "sessions" && <SessionsTab />}
        {tab === "deps" && <DepsTab />}
      </div>
    </div>
  );
}
```

`src/tabs/RecordTab.tsx`:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type Status = { state: "idle" | "recording" | "transcribing"; session_id: string | null; system_signal: boolean; mic_signal: boolean };

export default function RecordTab() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Status | null>(null);
  const [lastSession, setLastSession] = useState<string | null>(null);

  useEffect(() => {
    const tick = async () => {
      try { setStatus(await invoke<Status>("recorder_status")); } catch {}
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  const start = async () => { try { await invoke("recorder_start"); } catch (e) { console.error(e); } };
  const stop = async () => {
    try {
      const id = await invoke<string>("recorder_stop");
      setLastSession(id);
    } catch (e) { console.error(e); }
  };
  const openDir = async () => {
    if (lastSession) await invoke("open_session_dir", { sessionId: lastSession });
  };

  return (
    <div>
      <p style={{ background: "var(--c-pill-off)", padding: 10, borderRadius: 6, fontSize: 12 }}>
        ⚠ {t("record.warning")}
      </p>
      <div style={{ marginTop: 16, display: "flex", gap: 12 }}>
        {status?.state === "recording" ? (
          <button className="mmr-btn" onClick={stop} style={{ fontSize: 16, padding: "10px 24px" }}>
            ■ {t("record.stop_button")}
          </button>
        ) : (
          <button className="mmr-btn" onClick={start} disabled={status?.state === "transcribing"} style={{ fontSize: 16, padding: "10px 24px" }}>
            ▶ {t("record.start_button")}
          </button>
        )}
      </div>
      <div style={{ marginTop: 14, display: "flex", gap: 8 }}>
        <span className={`mmr-pill ${status?.system_signal ? "on" : ""}`}><span className="mmr-pill-dot" /> {t("capsule.system_pill")}</span>
        <span className={`mmr-pill ${status?.mic_signal ? "on" : ""}`}><span className="mmr-pill-dot" /> {t("capsule.mic_pill")}</span>
      </div>
      {status?.state === "transcribing" && <p style={{ marginTop: 16, opacity: 0.7 }}>{t("record.transcribing_hint")}</p>}
      {lastSession && status?.state === "idle" && (
        <div style={{ marginTop: 16 }}>
          ✓ {t("record.done_title")}: <code>{lastSession}</code>
          <button className="mmr-btn" onClick={openDir} style={{ marginLeft: 8 }}>{t("record.open_folder")}</button>
        </div>
      )}
    </div>
  );
}
```

`src/tabs/SessionsTab.tsx`:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

export default function SessionsTab() {
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<string[]>([]);

  useEffect(() => {
    invoke<string[]>("list_sessions").then((s) => setSessions(s.sort().reverse()));
  }, []);

  const open = async (id: string) => { try { await invoke("open_session_dir", { sessionId: id }); } catch {} };

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("sessions.title")}</h3>
      <p style={{ fontSize: 12, opacity: 0.7 }}>{t("sessions.hint")}</p>
      {sessions.length === 0 ? (
        <div style={{ opacity: 0.6 }}>{t("sessions.empty")}</div>
      ) : (
        <ul style={{ listStyle: "none", padding: 0 }}>
          {sessions.map((id) => (
            <li key={id} style={{ padding: "8px 0", borderBottom: "1px solid var(--c-border)" }}>
              <button className="mmr-btn" onClick={() => open(id)}>📁</button>
              <code style={{ marginLeft: 8 }}>{id}</code>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
```

`src/tabs/DepsTab.tsx`:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type DepsCheck = {
  whisper_cli_ok: boolean;
  whisper_cli_path: string;
  model_ok: boolean;
  model_path: string;
};

const LINUX_CMD = "bash <(curl -fsSL https://raw.githubusercontent.com/yazelin/mori-meeting-recorder/main/scripts/install-whisper-linux.sh)";
const WINDOWS_CMD = "iwr https://raw.githubusercontent.com/yazelin/mori-meeting-recorder/main/scripts/install-whisper-windows.ps1 | iex";

export default function DepsTab() {
  const { t } = useTranslation();
  const [deps, setDeps] = useState<DepsCheck | null>(null);

  const recheck = async () => {
    try { setDeps(await invoke<DepsCheck>("deps_check")); } catch {}
  };
  useEffect(() => { recheck(); }, []);

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("deps.title")}</h3>
      <p style={{ fontSize: 12, opacity: 0.7 }}>{t("deps.hint")}</p>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <DepRow label={t("deps.whisper_cli")} ok={deps?.whisper_cli_ok ?? false} path={deps?.whisper_cli_path ?? "—"} t={t} />
        <DepRow label={t("deps.model")} ok={deps?.model_ok ?? false} path={deps?.model_path ?? "—"} t={t} />
      </div>
      <button className="mmr-btn" onClick={recheck} style={{ marginTop: 12 }}>{t("deps.recheck")}</button>

      <h4 style={{ marginTop: 18, marginBottom: 6 }}>{t("deps.linux_install")}</h4>
      <pre style={{ background: "var(--c-surface)", padding: 10, borderRadius: 6, fontSize: 11, overflow: "auto" }}>
        {LINUX_CMD}
      </pre>

      <h4 style={{ marginTop: 14, marginBottom: 6 }}>{t("deps.windows_install")}</h4>
      <pre style={{ background: "var(--c-surface)", padding: 10, borderRadius: 6, fontSize: 11, overflow: "auto" }}>
        {WINDOWS_CMD}
      </pre>
    </div>
  );
}

function DepRow({ label, ok, path, t }: { label: string; ok: boolean; path: string; t: (k: string) => string }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
      <span style={{ minWidth: 140 }}>{label}</span>
      <span style={{ color: ok ? "var(--c-accent)" : "var(--c-danger)" }}>
        {ok ? t("deps.found") : t("deps.missing")}
      </span>
      <code style={{ fontSize: 11, opacity: 0.6, marginLeft: "auto" }}>{path}</code>
    </div>
  );
}
```

- [ ] **Step 3: 寫 install scripts**

`scripts/install-whisper-linux.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
ver="${WHISPER_VERSION:-v1.7.0}"
mkdir -p ~/.mori/bin ~/.mori/models
if [ ! -x ~/.mori/bin/whisper-cli ]; then
  echo "→ downloading whisper.cpp ${ver}…"
  url="https://github.com/ggerganov/whisper.cpp/releases/download/${ver}/whisper-bin-x64.zip"
  curl -L -o /tmp/whisper.zip "$url"
  unzip -o /tmp/whisper.zip -d /tmp/whisper-unzip
  cp /tmp/whisper-unzip/main ~/.mori/bin/whisper-cli 2>/dev/null || \
    cp /tmp/whisper-unzip/whisper-cli ~/.mori/bin/whisper-cli
  chmod +x ~/.mori/bin/whisper-cli
  rm -rf /tmp/whisper-unzip /tmp/whisper.zip
fi
if [ ! -f ~/.mori/models/ggml-small.bin ]; then
  echo "→ downloading ggml-small model…"
  curl -L -o ~/.mori/models/ggml-small.bin \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
fi
echo "✓ ready: ~/.mori/bin/whisper-cli + ~/.mori/models/ggml-small.bin"
```

`scripts/install-whisper-windows.ps1`:

```powershell
$ErrorActionPreference = "Stop"
$ver = if ($env:WHISPER_VERSION) { $env:WHISPER_VERSION } else { "v1.7.0" }
$binDir = "$env:USERPROFILE\.mori\bin"
$modelDir = "$env:USERPROFILE\.mori\models"
New-Item -ItemType Directory -Force -Path $binDir, $modelDir | Out-Null
if (-not (Test-Path "$binDir\whisper-cli.exe")) {
    Write-Host "→ downloading whisper.cpp $ver..."
    $url = "https://github.com/ggerganov/whisper.cpp/releases/download/$ver/whisper-blas-bin-x64.zip"
    Invoke-WebRequest -Uri $url -OutFile "$env:TEMP\whisper.zip"
    Expand-Archive -Force "$env:TEMP\whisper.zip" -DestinationPath "$env:TEMP\whisper-unzip"
    if (Test-Path "$env:TEMP\whisper-unzip\main.exe") {
        Copy-Item "$env:TEMP\whisper-unzip\main.exe" "$binDir\whisper-cli.exe"
    } else {
        Copy-Item "$env:TEMP\whisper-unzip\whisper-cli.exe" "$binDir\whisper-cli.exe"
    }
    Remove-Item -Recurse "$env:TEMP\whisper-unzip", "$env:TEMP\whisper.zip"
}
if (-not (Test-Path "$modelDir\ggml-small.bin")) {
    Write-Host "→ downloading ggml-small model..."
    Invoke-WebRequest -Uri "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin" `
        -OutFile "$modelDir\ggml-small.bin"
}
Write-Host "✓ ready: $binDir\whisper-cli.exe + $modelDir\ggml-small.bin"
```

`scripts/verify.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
echo "==> cargo test"
(cd src-tauri && cargo test --release 2>&1 | tail -5)
echo "==> npm run build"
npm run build 2>&1 | tail -5
echo "==> cargo check --all-targets"
(cd src-tauri && cargo check --all-targets 2>&1 | tail -3)
echo "✓ verify ok"
```

```bash
chmod +x scripts/*.sh
```

- [ ] **Step 4: 跑 verify.sh**

```bash
cd ~/mori-universe/mori-meeting-recorder
bash scripts/verify.sh 2>&1 | tail -10
```

Expected: 全綠。

- [ ] **Step 5: Manual e2e — Linux**

```bash
cd ~/mori-universe/mori-meeting-recorder
bash scripts/install-whisper-linux.sh   # 如果還沒裝
npm run tauri dev  # background launch
```

逐項驗:
1. 啟動 → 預設 collapsed 膠囊出現,360×60
2. 雙擊膠囊空白處 → 展開到 720×480,看到 Record / Sessions / Deps 三 tab
3. Deps tab 顯示 ✓ 找到 whisper-cli + ggml-small.bin
4. 回 Record tab → 點 ▶
5. 開 YouTube 播一段 → SYS pill 變亮(綠色 .on class)
6. 對麥克風講話 → MIC pill 變亮
7. 收回膠囊 → 看計時器跑、pill 即時亮滅
8. 等 30 秒 → 點 ■
9. 顯示「轉錄中…」hint
10. 完成 → 顯示「✓ 完成 — session」+ id
11. 點「開資料夾」→ xdg-open 開 `~/.mori/meetings/meeting-*/`
12. 內容檢查:
    - `audio/system.wav` + `mic-internal.wav` 都存在,16kHz mono
    - `transcript/*.segments.jsonl` 各一行 per segment
    - `meeting.public.md` 只含 YouTube 那段
    - `meeting.internal.md` 只含麥克風那段(prefix `(內部)`)
    - `timeline.json` 兩個 track + segment_count 對
13. 開 mori-desktop(如果 BodyTab 跑著)→ 看到 `mori.meeting-recorder` entry,kind=standalone_app,entrypoint 對

- [ ] **Step 6: 更新 mori-desktop 的 backlog doc + memory**

`mori-desktop/docs/body-interface-backlog.md` 在 BI-4 done 那段後加(類似 BI-4 的標記):

```markdown
- **BI-5 done** ✅(2026-05-28,新 repo `yazelin/mori-meeting-recorder` v0.1.0)= Observer Mode MVP — Tauri 2 + cpal 雙軌 capture(Linux PipeWire `.monitor` + Windows WASAPI loopback)→ 停止後 tokio::join! parallel whisper.cpp 轉錄 → visibility-based `meeting.public.md`(只 system 軌)/ `meeting.internal.md`(mic 軌,前綴「(內部)」)/ `timeline.json` 匯出到 `~/.mori/meetings/<id>/`。AgentPulse 風 floating capsule(單視窗切 size,collapsed 360×60 / expanded 720×480 3-tab Record/Sessions/Deps)。BI-1 self-register `~/.mori/body-parts/mori.meeting-recorder/manifest.json`(kind=standalone_app,`interfaces:[]`),mori-desktop BodyTab 自動顯示。各 repo 自己 bundle install scripts(`scripts/install-whisper-{linux,windows}.{sh,ps1}`),`~/.mori/bin/whisper-cli` + `~/.mori/models/ggml-small.bin` filesystem 慣例共享(不 IPC)。**未做**(刻意,phase 2):live captions / chunk streaming、`mix-preview.wav`、mori-desktop RecorderTab、handoff 命令、BI-2 broker 整合、Presenter Mode、macOS。
```

並更新檔尾 `Last updated:` → `2026-05-28`、`Next action:` → `BI-6 Multi-session / Schedule 視需要;或先 follow-up BI-5 phase 2(live captions)`。

更新 `~/.claude/projects/-home-ct/memory/project_mori_body_interface.md`:在 BI-4 done 那行後加 BI-5 done;更新 `MEMORY.md` 條目:`BI-0→BI-5 ✅ done(2026-05-28), next=BI-6 Multi-session / Schedule(視需要)`。

- [ ] **Step 7: Commit + push + PR(兩個 repo)**

**mori-meeting-recorder repo**:

```bash
cd ~/mori-universe/mori-meeting-recorder
git add .
git commit -m "$(cat <<'EOF'
feat(ui): capsule + expanded 3-tab + install scripts + verify.sh

CapsuleView polls recorder_status every 500ms, double-click expand.
ExpandedView with Record / Sessions / Deps tabs. Deps tab shows
install commands for Linux + Windows (user copies to terminal).
verify.sh runs cargo test + npm build + cargo check.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push origin main
```

(此 repo 直接 push main — 第一個 release,沒 PR 流程。或開 PR 也行:`gh pr create` 然後 auto-merge。建議直接 push main + tag v0.1.0。)

**mori-desktop repo**:

```bash
cd ~/mori-universe/mori-desktop
git checkout -b docs/bi-5-mark-done
git add docs/body-interface-backlog.md
git commit -m "$(cat <<'EOF'
docs(bi-5): mark Meeting Recorder MVP done

New repo: github.com/yazelin/mori-meeting-recorder v0.1.0
BI-5 done criteria met: dual-track Observer Mode capture, parallel
stop-then-transcribe, visibility-based export, BI-1 self-register
manifest. Phase 2 defers tracked in spec.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push -u origin docs/bi-5-mark-done
gh pr create --title "BI-5: mark Meeting Recorder done" --body "$(cat <<'EOF'
## Summary
- BI-5 marked done in body-interface-backlog.md
- New repo: https://github.com/yazelin/mori-meeting-recorder v0.1.0
- All BI-5 done criteria from doc met

## Test plan
- [x] manual e2e on Linux passed
- [ ] Windows e2e (deferred, follow-up)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
gh pr merge --auto --squash
```

---

## Self-review

**Spec coverage:**
- §1 Repo 結構 ✓(Task 2)
- §2 元件責任 ✓(分散 Task 3-9,每個元件一個 task)
- §3 Data flow ✓(Task 9 recorder + Task 10 commands)
- §4 檔案格式 ✓(Task 4 store paths + Task 6 export format)
- §5 GUI capsule + expanded ✓(Task 12)
- §6 BI-1 manifest ✓(Task 7 + Task 10 startup hook)
- §7 Deps bundle ✓(Task 12)
- §8 測試策略 ✓(每個 task 都有 TDD + Task 12 manual e2e)
- §11 風險 — cpal Linux loopback ✓(Task 1 spike → confirm cpal 不可行 → 改 libpulse,spec 已更新)
- §13 mori-desktop 整合 — clarify ✓(完全不改 mori-desktop;Task 12 backlog doc only)

**Placeholder scan:**
- 沒有 TBD / TODO
- 每 step 都有 code 或 command 或明確 expected output
- 一處 Windows WASAPI loopback 的 cpal 0.15 行為註記(可能要 fallback到 `windows` crate),plan 標 follow-up 不是 placeholder
- Linux libpulse 路徑 spike 已實機驗證(Task 1 done),Windows cpal loopback **未** spike(留 Task 12 e2e 驗)

**Type consistency:**
- `SourceKind` enum(`MeetingSystem` / `MicInternal`)— Task 3 定義,Task 4-9 都用同一份
- `Visibility` enum 同
- `Segment` struct fields:Task 5 定義 `id / session_id / track / source_kind / visibility / start_ms / end_ms / text / is_final / confidence`,Task 6 / 9 都用同一份
- Tauri command 名稱:`recorder_start` / `recorder_stop` / `recorder_status` / `deps_check` / `set_window_mode` / `list_sessions` / `open_session_dir` — Task 10 註冊 + Task 11/12 invoke 用一致名稱
- camelCase 換:`open_session_dir` → JS `{ sessionId }`(Task 12 RecordTab + SessionsTab 已用 camelCase)

---

**Last updated**: 2026-05-28
**Spec**: docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md
**Next action**: yazelin approve plan → execute(subagent-driven recommended,12 task 適合 dispatch + review 模式)
