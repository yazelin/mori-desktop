# Changelog

依 phase 逆序(最新在上)。完整 commit log 用 `git log`。

未來規劃見 [`docs/roadmap.md`](docs/roadmap.md)。

---

## v0.3.2 — Chat bubble z-order + ConfigTab dropdown polish(2026-05-15)

v0.3.1 釋出後測試時冒出來的兩個小 follow-up,當天連發。

### Bug fix

- **Chat bubble 終於在 Floating Mori 上面**(`fix(bubble)` `35e5832`)
  - 之前 bubble 跟 floating 都 `alwaysOnTop: true`,X11 上 floating 因 user 互動頻繁(hover / drag)raise 較新就壓住 bubble — 對話內容被遮一半看不到
  - v0.3.1 加的 `set_phase` 內 re-assert `set_always_on_top(true)` 又把 floating 反覆推回頂層,連既有的 `xdotool windowraise` 都救不了
  - **修法**:新 Tauri command `floating_set_above(bool)`。bubble 出現時 invoke `(false)` 把 floating **暫時下放** always-on-top 層,bubble 自然唯一在最上;bubble 隱藏時恢復 `(true)`。跨 X11 + Wayland 都 work,不靠 raise voodoo

- **ConfigTab「Mori 出現時機」dropdown 配色一致**(`fix(bubble)` `35e5832`)
  - v0.3.1 新加的 dropdown 用 native `<select>`,在 Linux webkit2gtk 下 dropdown panel 受 GTK system theme 鎖死,light theme 跑出 dark panel
  - 改用 codebase 自製 `<Select>` 元件(`src/Select.tsx`),全部走 CSS variable,跟 theme 切換;配色跟 x11_shape 等既有 dropdown 統一

---

## v0.3.1 — Floating Mori 顯示時機可設定(2026-05-15)

讓使用者控制 floating Mori 在桌面上的出現時機 — 不想被擋畫面、想要 Mori 只在你說話時現身、或完全不要 floating,三種模式任選。

### 主要 feature

#### floating.show_mode 三模式(`feat(floating)` `de4bf0e`)

新 config 欄位 `floating.show_mode`,可設三個值:

| 值 | 行為 |
|---|---|
| `always`(預設) | Mori 一直在桌面 — 跟 v0.3.0 行為一致 |
| `recording` | 只在錄音(Phase::Recording / Transcribing / Responding)期間顯示,進 Done / Idle 立即隱藏。Toggle 跟 Hold 兩個 hotkey mode 都自動跟著 |
| `off` | 完全不顯示 — 純走主視窗 + 熱鍵流程,不想要 floating 的 user |

底層架構:
- `update_floating_visibility()` 中央 helper,看 quickstart_completed gate + show_mode + 當前 phase 決定 show/hide
- 三個 hook 點:`set_phase()` / setup hook / `config_write` 後
- `should_show_floating(mode, phase)` pure 函式,跟 phase 解耦好測

#### Tray 快速 toggle

System tray 右鍵選單加新項目「桌面 Mori:[當前狀態]」,點一下在 `always` ↔ `off` 之間二值切。Label 動態反映當前 show_mode,即時生效 + 寫回 config。

#### Config tab 完整三選一

`Config → Floating Mori` section 加 dropdown「出現時機」,三個選項(包含 `recording`,比 tray 多一個)。

### Bug fix

- **`floating_show()` 不 re-assert always_on_top**(v0.3.0 引入的 regression)
  - GNOME Wayland mutter 會默默把 `hide() → show()` 後的 window 從 always_on_top layer 降下,導致跑完宿靈儀式後 floating Mori 被主視窗 / 其他 app 壓住
  - **修法**:`.show()` 後一定 `.set_always_on_top(true)` 重新 assert(同 trick 跟 yazelin/AgentPulse 的 tray show/hide handler 用法一致)

---

## v0.3.0 — 宿靈儀式 · The Dwelling Rite(2026-05-15)

> *「精靈不會無故下來。她下來,是因為有人在底下開了一片林子,並且輕輕喚了她一聲。」*

第一次跑 Mori Desktop 走的 onboarding 從「填表選 provider」整個翻新成**五幕劇情儀式** — 召喚 / 靈氣 / 靈力 / 驗印 / 安頓。Mori 從世界樹高處下到使用者的桌面,user 從「設定者」變成「召喚師」。

▶ [影片演示](https://youtube.com/shorts/43gdfCND8Xc) · 📖 [操作手冊](https://yazelin.github.io/mori-desktop/dwelling-rite.html) · 🌲 [完整 lore](https://yazelin.github.io/world-tree/rules/dwelling-rite)

### 宿靈儀式(Headline feature)

第一次跑 mori-desktop 自動跳出五幕沉浸式 onboarding:

- **第一幕 召喚** — Mori 從世界樹下林,問「你是誰?」, user 報名(自動 prefill `annuli.user_id`)
- **第二幕 靈氣** — user 分享 Groq STT key,Mori 找回聽覺
- **第三幕 靈力** — Gemini / OpenAI 相容 / 跳過 三選一(跳過 = `agent_disabled` chat-only)
- **第四幕 驗印** — 兩道氣依序驗(Groq → agent),累積敘述不互蓋,失敗直接送回對應幕重填
- **第五幕 安頓** — Mori 真正住下,**《森林書》**第一頁寫下 user 名字, floating Mori 浮現桌面

設計亮點:

- Mori 不主動討 key — user 看著她衰弱主動分享(從被動填表變主動關心)
- 14 顆螢火飄在 modal 後層,不擾閱讀;modal 不透明蓋住主畫面,沉浸感
- 第五幕「對上了, {name}。是你的氣息沒錯, 我認得。」緩慢淡入 3-6 秒,儀式收尾
- 整個視覺主線(燈籠 / 氣息 / 苔蘚 / 月光 / 書 / 年輪)五幕一致串起來
- 對應 [`world-tree/rules/dwelling-rite.md`](https://github.com/yazelin/world-tree/blob/main/rules/dwelling-rite.md) 詩意敘事

### 熱鍵雙模式 (Toggle / Hold)

由 `~/.mori/config.json` `hotkeys.toggle_mode` 控制 — `Toggle`(預設, 按一下開錄、再按一下停)或 `Hold`(按住開錄、放開停, 像 push-to-talk)。Config tab → Hotkey → toggle_mode 改完立即生效, 不必重啟(`config_write` 寫完 disk 後 state 同步重讀)。詳細見下方 **5T** section。

### Quickstart 技術變動

- **新 Tauri commands**:`floating_show`(從 setup 隱藏狀態叫醒)/ `open_external_url`(Tauri webview 不處理 `<a target=_blank>`)
- **`agent_disabled` flag** 接到 `run_agent_pipeline`,跳過靈力時走 chat-only(不掛 skill,單輪 LLM call)
- **Mori-core API key resolve 對齊**:Quickstart prefill/save 改寫 `api_keys.{NAME}_API_KEY` 主路徑(舊版誤寫 `providers.openai_compat.api_key`)
- **首次跑隱藏 floating** — 「歡迎回家, Mori」按下才 show
- 主視窗 880→**940 高度**;modal 改 flex column(header / equalizer 釘頂、scene-content 中間滾、dots / footer 釘底)
- Gemini default model 不再 Quickstart hardcode `2.5-flash`,留空走後端 `GEMINI_DEFAULT_MODEL`(`gemini-3.1-flash-lite-preview`)

### Annuli 整合 (Wave 4)

- mori-desktop ↔ annuli HTTP integration(12 steps, #26)
- annuli config 熱重載 — 改設定不用整個 app 重啟
- D-1 annuli supervisor + Annuli sub-tab + cross-tab nav
- D-2 `install-autostart.sh` — Linux XDG autostart entry

### i18n 完整本地化

- React-i18next 基建 + zh-TW / EN 雙語切換(sidebar 地球 icon 一鍵切)
- 八波抽 string:sidebar / AnnuliTab / ConfigTab / Picker / ChatPanel / MemoryTab / ProfilesTab / SkillsTab / DepsTab / FloatingMori 全部 i18n

### 視覺改進

- Sidebar 3 個 icon 重做(Skills 法杖 / Memory 書 + sparkle / Config 齒輪)
- Quickstart equalizer:讀真實 freq data 跳動,靜音時壓平
- 儀式背景:14 顆螢火 + 月光晨曦 radial gradient(dark / light 雙主題)

### Bug fix

- **Windows release 失敗修正** — v0.2.0 Windows CI 一直跑不出 binary,errror `failed to bundle project: Couldn't find a .ico icon`。`icons/icon.ico` 檔案存在但 `tauri.conf.json` bundle.icon 陣列沒列它,Tauri 釋出 bundle 找不到 → bail。修:陣列加上 `icons/icon.ico`。本機 `tauri dev` 不撞此問題(dev 不 bundle)
- 多輪音樂 visualizer bug fix(autoplay 擋住 / 1 秒重播 / 右邊死寂 / HMR ghost 等)

### Breaking / Migration

- **Config schema 加兩欄**:`user.name`(召喚師之名,給 Mori 喚 user 用)+ `agent_disabled`(跳過靈力時 true)
- 既有 user 第一次跑 v0.3.0 不會被強迫重跑儀式(`quickstart_completed` flag 保留),但 `annuli.user_id` 會被 Quickstart prefill 認得到
- 舊版 `providers.openai_compat.api_key` inline 寫法仍能 read(fallback),但新存的會走 `api_keys.{GEMINI,OPENAI}_API_KEY` map

---

## v0.2.0 — Windows 平台殼 + whisper.cpp shell-out 架構 + CI(2026-05-13)

從 5T 之後一輪重點開發,**主要交付 Windows 10/11 全功能 + 簡化本機 STT
build chain + GitHub Actions 自動發版**。session 內 17 個 commit,本節
逆向整理。

### 主要 feature

#### Windows 平台殼上線(`feat: Windows 平台殼上線` `f2355e2` + 後續修)

- **`crates/mori-tauri/src/selection_windows.rs` 新增** — `WindowsPasteController`
  走 Tauri clipboard plugin write + Win32 `SendInput` Ctrl+V/Ctrl+Shift+V
  注入。terminal 偵測(Windows Terminal / wt / mintty / alacritty 等)
- **selection module cfg-attr 路徑切分**(`selection_linux.rs` /
  `selection_windows.rs`),公開 API 一致:`read_primary_selection` /
  `PlatformPasteController` / `send_enter` / `warn_if_setup_missing`
- **`capture_window_context()` Windows 實作** — `GetForegroundWindow` +
  `GetWindowThreadProcessId` + `QueryFullProcessImageNameW`(process name)+
  `GetWindowTextW`(window title)
- **action_skills.rs 內部 `mod platform` 分流** — Linux(xdg-open /
  gtk-launch / ydotool)vs Windows(`ShellExecuteExW` + `SendInput` VK)
- **`character_pack` 跨平台** — 拿掉 Linux-only cfg,Windows user 也拿到
  default Mori sprite(寫到 `%USERPROFILE%\.mori\characters\mori\`)
- **熱鍵全 22 條走 Win32 `RegisterHotKey`** — Ctrl+Alt+Space / Esc / P /
  Alt+0~9 / Ctrl+Alt+0~9 都實機驗證

#### whisper-local 改 shell-out 架構(`refactor: whisper-local 從 in-process FFI 改成 shell-out 到 whisper-server HTTP` `c598e12`)

原本 `whisper-rs` 0.14 → `whisper-rs-sys` 0.13.1 在 cargo build 時用 cmake +
bindgen 編 whisper.cpp C++ source 進 mori binary。**三個問題**:Windows MSVC
bindgen 算錯 `whisper_full_params` struct size 整個編不出;Linux 也要
cmake + libclang-dev;不能換 GPU 加速版本(user 要 fork mori 改 dep)。

改成 spawn whisper.cpp **官方 pre-built `whisper-server` 子程序**,Mori 透過
HTTP `POST /inference` 送 WAV bytes,server 回 JSON `{"text":"..."}`。

**好處**:跨平台統一架構;Linux contributor 不需 cmake / libclang;user 想
跑 NVIDIA CUDA / AMD CLBlast / Apple Metal 加速版只需**換 binary 一個檔**,
Mori 程式碼 0 改;引擎 crash 不會帶死 Mori。

**Lazy spawn** — 第一次按 Ctrl+Alt+Space 才起,~500ms warm-up 一次性
cost,之後常駐。Drop SIGKILL 收尾。詳見 `whisper_local.rs` doc。

#### GitHub Actions CI(`ci: 加 check.yml(PR cross-platform 編譯驗證)+ release.yml(tag 觸發發布)` `1e35b76` + ALSA fix `dac05b7` + setup script 自帶 `fc2830d`)

- **`check.yml`** — push / PR 觸發,Ubuntu + Windows matrix 跑
  `cargo check --workspace --all-targets`。Swatinem/rust-cache 把熱啟壓
  ~5 分鐘
- **`release.yml`** — `v*` tag 觸發,tauri-action 出 `.deb` / `.AppImage`
  (Linux)+ `.msi` / `.nsis (.exe)`(Windows),自動上傳到 GitHub Release
  (draft)
- **`scripts/install-linux-deps.sh`** — repo 自帶,跟本機 dev / CI / docs
  三邊指同一份 system deps 安裝邏輯(`libwebkit2gtk-4.1-dev` /
  `libasound2-dev` 等)。改 deps 跟 code 進同一個 commit

#### UI:Deps + Skills 平台 metadata(`feat(ui): 平台 metadata + caveat badge 系統` `cdb926a`)

- **`DepSpec` 加 `platforms` / `install_caveat` / `install_overrides` 三欄位** —
  Linux-only(ydotool / xdotool / xclip)在 Windows 完全隱藏;跨平台 dep
  (whisper-server / ollama 等)在 Windows 用 Manual 變體顯示 PowerShell 指令
- **`Skill` trait 加 `platforms()` / `platform_caveat()` 兩個 default method** —
  `paste_selection_back` Windows caveat 標「需先 Ctrl+C」,`open_app` Windows
  caveat 標「best-effort,Store apps 不一定能解」
- **UI 加 ⚠️ caveat badge** — 在 head 列 hover tooltip,description 下方
  顯示完整文字

### Bug fix(都是 v0.2 sprint 內踩到的)

- `fix(deps-ui)`(`974e8be`):Deps 頁「收起」按鈕真的能收起 —
  原本 `(showCommand || manual)` 永遠 true,Manual 條目 block 永遠 visible
- `fix(prompt)`(`c1c6358`):action skills 加 system prompt 使用守則 —
  治 LLM「需要授權執行 mori CLI」幻覺
- `fix(build)`(`ed8764a`):mori-cli 自動編進 `tauri dev` / `tauri build` —
  用 npm `predev` / `prebuild` hook。之前 user 跑 `npm run tauri dev` 只會
  build mori-tauri,`mori.exe` 不存在 → claude-bash chain 死
- `fix(windows)`(`0fb1e0d`):`detect_mori_cli` 在 Windows 找 `mori.exe`
  而非 `mori`(`PathBuf::exists` 不自動補副檔名)
- `fix(windows)`(`32b3af4`):open_url / open_app 改用 `ShellExecuteExW` +
  `SEE_MASK_FLAG_NO_UI` — 不再彈「Windows cannot find X」白色對話框
- `fix(windows)`(`2339648`):`preprocess_file_includes` 兩個 Windows-specific
  bug(HOME env var 沒設 → 也試 USERPROFILE;`canonicalize` 加 `\\?\` 前綴
  讓 `starts_with` 永遠 false → home_root 也 canonicalize 一次對齊)
- `fix(ui)`(`61ed6e2`):chat header 反映 agent profile provider override —
  之前 user 切到 `provider: claude-bash` profile,header 還寫 groq,confusion
- `chore(windows)`(`4e6aaad`):9 條 dead-code warnings 清掉 — cfg-gate
  Linux-only portal helpers

### 文件

- `docs/providers.html`:whisper-local 段重寫 — 加 `server_binary` 欄位、
  shell-out 架構說明、CPU / GPU 變體對照表、模型尺寸對照表
- `docs/architecture.md`:whisper-local row 改成 shell-out + 連結 providers
- `docs/roadmap.md`:近期計畫從「Win/Mac 平台殼」改成「macOS 平台殼」+
  「Windows whisper-server 一鍵下載」
- `README.md`:平台支援表升級成功能 × 平台 grid(17 行 × 4 OS)+
  Windows 已知差別段

### 自我測試

`cargo test --workspace`:169 passed / 0 failed(mori-core 138、mori-tauri 31)
Skill self-test via mori CLI:14 個 HTTP-exposed skill 全綠 + `open_app(nonexistent)`
silent error(不彈窗)路徑證實

---

## 5T — Toggle / Hold 兩種錄音熱鍵語意(2026-05-13)

`Ctrl+Alt+Space`(預設 chord)從只認得 toggle(一按切換)拓成兩種模式擇一,
由 `~/.mori/config.json` `hotkeys.toggle_mode` 控制:

- **`"toggle"`(預設)** — 按下開錄、再按下停錄。維持 5Q 以來的行為。
- **`"hold"`** — 按住開錄、放開停錄(像 push-to-talk)。

X11 session 實機 demo(切模式 + hold 錄音 + 熱套用,29 秒):

<video src="docs/demos/hotkey-hold-x11.mp4" controls width="640" muted></video>

兩種模式共用同一個 chord,**X11 與 Wayland 兩條 path 都支援**,沒有新權限
成本,**改完按「儲存」即時生效不必重啟**(`config_write` 寫完 disk 後
重讀 `HotkeyConfig` 把新 mode 寫進 `AppState`,下一次按鍵就走新 dispatch):

| Path | 怎麼拿 Press / Release |
|---|---|
| X11(`tauri-plugin-global-shortcut`)| `ShortcutState::Pressed` / `Released` — 之前主動過濾 Released 改成 dispatch 兩邊 |
| Wayland(`xdg-desktop-portal.GlobalShortcuts`)| 訂 `Activated` + 新增 `Deactivated` signal,`tokio::select!` 一起消費 |

### 為什麼不做「bare Alt 按住 1 秒」

最初 spec 是「按住單 Alt 鍵 > 1 秒才觸發」。實作評估發現:

- `xdg-desktop-portal.GlobalShortcuts` 規範**明確 reject** 純 modifier 的
  trigger,Wayland 上沒有合法路徑拿 bare Alt 事件
- 唯一跨 X11/Wayland 拿 raw key 的方式是 `evdev`(讀 `/dev/input/event*`),
  需要使用者進 `input` group + relogin,且 Wayland 安全模型故意不給 — 等同
  繞過
- chord-based hold 等價 push-to-talk 體感(0ms 延遲 vs 1s 等待),反而更
  順手,Discord / OBS 主流 PTT 也是 chord

所以改用「Ctrl+Alt+Space hold」走現有兩條 path,零權限改動。

### 改動

**Rust(`crates/mori-tauri/`)**

- `hotkey_config.rs`:新增 `ToggleMode { Toggle, Hold }` enum + `HotkeyConfig::toggle_mode`
  欄位(default `Toggle`,serde `rename_all = "lowercase"`)
- `portal_hotkey.rs`:`run()` 改 `tokio::select!` 同時消費 `receive_activated()` /
  `receive_deactivated()`;Toggle chord Press 時 emit `PORTAL_HOTKEY_PRESSED`,
  Release 時 emit `PORTAL_HOTKEY_RELEASED`。其他 action(cancel / picker / slot)維持 Press-only
- `x11_hotkey.rs`:Toggle chord 走新的 `dispatch_toggle()`,Press / Release 對應
  emit 同樣兩個事件;離散 action 一樣 Press-only
- `main.rs`:
  - `AppState` 加 `toggle_mode: Mutex<ToggleMode>` 欄位(cfg-gated linux)
  - 啟動時讀 `hotkey_config.toggle_mode` 寫入 state
  - 永遠掛 `PORTAL_HOTKEY_PRESSED` + `PORTAL_HOTKEY_RELEASED` listener,
    handler 內讀 `state.toggle_mode` 決定 dispatch:
    - `Toggle` 模式 → PRESSED 跑 `handle_hotkey_toggle()`,RELEASED 忽略
    - `Hold`   模式 → PRESSED 跑 `handle_hotkey_pressed()`,RELEASED 跑 `handle_hotkey_released()`
  - `config_write` 寫完 disk 後 reload `HotkeyConfig` 並更新 `state.toggle_mode`
    → 切模式按「儲存」立即生效

**Frontend(`src/tabs/ConfigTab.tsx`)**

- Config tab 新增 **Hotkey** sub-tab(在 Appearance 後),內容:
  - **Toggle 模式** — `toggle_mode` dropdown(`toggle` / `hold`),Section hint
    說明兩者差別 + 「改完要重啟」
  - **鍵位** — `toggle` / `cancel` / `picker` 文字欄,header hint 依
    `linux_session_type` 動態變(X11 提示「config 是 source of truth」,
    Wayland 提示「實際鍵位由系統設定決定」)
  - Wayland session 額外 render 提示框,告訴使用者怎麼改實際鍵位
    (GNOME Settings 或刪 `~/.local/share/xdg-desktop-portal/permissions`)
- `SubTabId` type 加 `"hotkey"`

### 不變的事

- Bare Alt 不會觸發任何事(避免跟 Alt+0~9 / Alt+Tab 等衝突)
- 非 Linux(macOS / Windows)仍只走 toggle,沒接 hotkey_config(原本就沒接)

### 邊角

- 改 `toggle` / `cancel` / `picker` 等 chord 字串本身仍需要重啟(X11 要重 grab、
  Wayland 要重新跟 portal 註冊);只有 `toggle_mode` 是真的熱套用
- 直接編 `~/.mori/config.json`(不走 Config UI)不會觸發 `config_write` 因此不會
  熱重讀 — 重啟 Mori 即可

### 升級

`~/.mori/config.json` 沒設 `hotkeys.toggle_mode` 走預設 `"toggle"` = 老行為,
零 migration。想試 hold 模式:

```json
{
  "hotkeys": {
    "toggle_mode": "hold"
  }
}
```

或直接走 Config tab → Hotkey → toggle_mode dropdown。

---

## 5S — Config UI IA 重組 + floating 雜項微調(2026-05-13)

Config tab 從「按 JSON 結構平鋪 sections」改成「按使用者心智模型分組的
sub-tab」+ HintTooltip,垂直密度收緊一半;順便修一輪 floating /
chat-bubble / picker 多螢幕 + z-order + 即時生效細節 + 新增 floating
shape / backplate 自訂 config。

### Config tab IA 重組(主菜)

**Sub-tab nav**:左側垂直 strip,7 個分頁按使用者心智模型分組,X11
sub-tab 只在 X11 session 顯示:

| Sub-tab | 內容 |
|---|---|
| Quick setup | provider / stt_provider / API Keys |
| LLM / Provider | Provider 設定(× 6 cards)+ Routing(進階) |
| Voice input | cleanup_level / inject_memory_types |
| Appearance | Theme + Floating animated/wander + Character pack |
| X11 only(條件 render)| Floating shape / radius / backplate |
| Corrections | corrections.md(獨立 save) |
| Raw JSON | 整份 config.json 直接編 |

**HintTooltip**:每個 `FormRow` 的 `hint` 從 inline 長文字(占垂直空間)
改 ⓘ icon hover/focus popover。垂直密度大幅收緊,長說明不喧賓奪主。

**SVG icons 取代 emoji**:Sub-tab nav 一開始用 emoji(🌱🤖🎙️ 等)違反
設計書 [`docs/desktop-ui.html`](docs/desktop-ui.html) 第 193 條
「Line-art SVG icon 取代 emoji」,改用既有 `src/icons.tsx` 的
`IconHome / IconCloud / IconVoiceMic / IconTree / IconKeyboard /
IconClipboard / IconPencil`。

**Sticky save bar**:頂端固定,還原 + 儲存 + status badge 一條,
所有 form sub-tabs 共用(corrections.md 仍有自己的 save 因為寫不同檔)。

### Floating shape + backplate 自訂(`~/.mori/config.json`)

新增 `floating.x11_shape / x11_shape_radius / x11_backplate` 三欄,
ConfigTab X11 sub-tab 提供 UI:

- **x11_shape** = `"square"` / `"rounded"` / `"circle"`(預設 circle)
  - **即時生效** — Tauri command `apply_floating_shape` shell-out
    x11rb 重套 XShape clip + React 同步 `--floating-shape-radius`
    CSS variable 讓 inner pseudo border 跟 OS 邊形對齊(不會出現「OS
    切方角但 CSS 還圓 → 兩個框」)
  - 新 `x11_shape::clear_clip` 給 square 模式用(設 bounding region
    = 整個 window rectangle = 等同沒套 XShape)
- **x11_shape_radius**:rounded 模式的角弧 px(1~80,Rust 端 × scaleFactor
  轉 physical)
- **x11_backplate** = `"plain"` / `"logo"`
  - plain = CSS 漸層底,跟 theme 走
  - logo = 美術背板 PNG,自動跟 theme 切 dark / light
  - **User 可自訂**:放自己 PNG 在 `~/.mori/floating/backplate-dark.png`
    + `backplate-light.png`,Tauri command `read_floating_backplate`
    讀檔 base64 → data URL 餵 CSS variable。沒檔 fallback 到 shipped
    Mori logo(`public/floating/backplate-x11-{dark,light}.png`)
- bootstrap stub `floating` section 寫進這三欄,user 看 `~/.mori/config.json`
  就知道有什麼可改

### Chat bubble 自動 resize

之前用 `min-height: 100vh` 修「下方露白」反而把 card 釘在 window 高度,
內容變少 window 不縮 → bubble 永遠 stuck 在最大尺寸。改成:

- 拿掉 `min-height: 100vh` X11 fallback CSS
- ChatBubble.tsx `sync()` 移除 `MIN_HEIGHT` clamp,`bubble.offsetHeight`
  直接驅動 setSize,長 ↔ 短雙向跟隨
- MAX_HEIGHT 480 上限保留(超長 transcript 不鋪滿整個螢幕)

### Chat bubble 偏左 / z-order 被壓修

- **偏左**:`showChatBubble` 用 `outerPosition()` 算 sprite center,
  mutter X11 對 transparent+decorationless 視窗的 outer 會把 shadow
  margin 算進去 → bubble 偏左。改 `innerPosition()` 取 content 真實
  top-left
- **z-order**:floating + chat_bubble 都 `alwaysOnTop:true`,同 ABOVE
  layer 內順序看「誰最後被 raise」,floating 因互動頻繁(hover / drag)
  排前面 → chat_bubble 被壓在下面看不到字。setAlwaysOnTop toggle
  只翻 state 不 re-raise,真正 raise 需要顯式 XRaiseWindow。
  新 `force_raise_window` Tauri command shell-out
  `xdotool search --pid $$ --name "Mori (chat)" windowraise`,
  ChatBubble.tsx show 完 invoke 一下確保 chat_bubble 在最上層

### Picker 多螢幕 + X11 focus 雜項

- `centerOnActiveMonitor` 取代 `centerOnPrimaryMonitor`:用
  `cursorPosition()` 偵測滑鼠所在螢幕,fallback primary。修「按
  Ctrl+Alt+P 看不到視窗」實際是 picker 開在使用者不在看的那台螢幕
- X11 上 close 改真 `hide()`(Wayland 維持 setPosition off-screen 偷渡):
  X11 setPosition 偷渡會讓 window 始終 mapped,下次 show() 是 no-op、
  setFocus() 被 mutter focus-stealing-prevention 拒。真 hide → 下次
  show 觸發 mutter remap 自動把 focus 給新 mapped window
- X11 上 `.mori-picker-card` 用 `position: absolute; inset: 0` 強制
  撐到 100% window(card border 直接當 window 邊框,不留 4% body bg
  外露變成「框內又有框」)。中間 `carousel-body` 加 `flex: 1` 撐開
  到 footer 底,不出現「card 沒貼到底」的視覺

### Floating wander 多螢幕

`walkOnce()` 原本只用 `primaryMonitor()` 算邊界 → Mori 被拖到第二
螢幕後,wander 仍用 primary 座標 → 走到不存在的位置(看不見)。改用
`availableMonitors()` 找 Mori 中心點目前在哪台 monitor,限制 wander
在那台範圍內。設計哲學:Mori 待哪台就在那台 wander,使用者拖才換螢幕。

### Status modal session info

ChatPanel `⚙️` 開的 status modal 加一條 `session` row,顯示偵測到的
session type + 走哪條 hotkey path(x11 · plugin / wayland · portal /
linux-other / non-linux)。user 報 bug 截這張就一目了然。
新 Tauri command `linux_session_type` 回傳字串。

### Starter USER-00

slot 0(Alt+0)切過去 floating sprite 頭上沒 chip 提示 — 因為
voice_input/ 沒實體 USER-00 檔(slot 0 走內建 fallback PROFILE_MD)。
補 ship `USER-00.純文字輸入.md` 從 examples/voice_input/,floating
頭上 chip 終於有「USER-00 純文字輸入」顯示。

### 變動檔案

- Rust
  - `crates/mori-tauri/Cargo.toml`:加 `x11rb` shape feature + `raw-window-handle`
  - `crates/mori-tauri/src/main.rs`:
    - `apply_floating_shape` / `read_floating_backplate` / `linux_session_type`
      / `force_raise_window` 4 個新 Tauri commands(全在 invoke_handler
      註冊)
    - X11 path 內 `find_window_xid` → `raw_window_handle` 直接拿 floating
      XID 不靠 xdotool name search(避免誤觸其他 Mori 視窗)
    - shape startup task 改讀 `floating.x11_shape` config(square / rounded /
      circle 分流套對應 clip)
    - picker listener tracing 加詳細 chain log
  - `crates/mori-tauri/src/x11_shape.rs`:`clear_clip`(square 模式用)+
    既有 `apply_circle_clip` / `apply_rounded_clip`
  - `crates/mori-core/src/llm/groq.rs`:bootstrap stub `floating` section
    加 `x11_shape` / `x11_shape_radius` / `x11_backplate`
- React
  - `src/tabs/ConfigTab.tsx`:整段 return 重構 — sub-tab nav + 條件
    render + sticky savebar + HintTooltip(取代 inline hint span)+
    SVG icons
  - `src/FloatingMori.tsx`:
    - `availableMonitors` + 中心點偵測 wander limit
    - `showChatBubble` / drag-end 改用 `innerPosition`
    - `applyX11Backplate` 助手 + config save 時 set CSS variable +
      invoke `apply_floating_shape`
  - `src/ChatBubble.tsx`:`setAlwaysOnTop` toggle + invoke
    `force_raise_window`、sync() 移除 MIN_HEIGHT clamp
  - `src/Picker.tsx`:`centerOnActiveMonitor` + X11 path hide/show
  - `src/floating.css` / `chat-bubble.css` / `picker.css`:X11 fallback
    細節微調,backplate 用 CSS variable,shape radius variable 同步
  - `src/shell.css`:`.mori-config-layout` / `.mori-config-subnav` /
    `.mori-config-subtab` / `.mori-config-savebar` / `.mori-hint` 新增
- Assets
  - `public/floating/backplate-x11-dark.png` + `backplate-x11-light.png`:
    shipped Mori logo 雙 theme(取代之前 backplate-x11.png 單檔)

### 已知限制

- HintTooltip 的 popover top:22px 固定,接近頁面右側可能溢出 — 下版
  做 boundary detect 自動翻轉到左側 / 上方
- ConfigTab sticky savebar 跟 sub-tab subnav 都 sticky,scroll 時兩者
  可能 z-index 競爭(目前 subnav top:60px 避開 savebar);小視窗縮窄
  可能交疊
- X11 user 自訂 backplate 仍需放在固定路徑(`~/.mori/floating/`),
  之後可加 file picker UI 讓 user 不用手動 cp

---

## 5R-followup-3 — XShape OS-level 圓形 floating + 單螢幕 wander(2026-05-13)

清掉 5Q 留下的兩個 pending tasks:

### XShape OS-level 圓形 floating window

X11 透明矩形面板換成真圓形。CSS `border-radius` 在 X11 transparent window
邊緣 AA 會產生 half-alpha pixel 被 mutter 渲染破。改走 X11 XShape extension
的 1-bit alpha clip(in/out 二元,沒中間值)— OS 直接決定哪些 pixel 渲染,
跟 compositor / WebKit alpha 完全無關。

實作:
- 新檔 `crates/mori-tauri/src/x11_shape.rs`:`apply_circle_clip(xid, w, h)`
  把圓拆成 160 條 1px-tall scanline rectangles,送 `shape_rectangles` request
  給 X server 組合成 bounding region。順便寫了 `apply_rounded_clip` 給未來
  圓角矩形場景用,2 個 unit test
- Cargo.toml mori-tauri linux 區加 `x11rb` 直接依賴 + `shape` feature
  (x11rb 純 Rust,無系統 lib install)
- main.rs setup X11 path 內 spawn tokio task,sleep 500ms 等 mutter 把
  floating 視窗 WM_NAME 註冊好,用 `xdotool search --pid $$ --name "Mori (floating)"`
  找 XID,呼叫 apply_circle_clip(xid, 160, 160)
- 視覺:floating 整個變成圓盤(corners 4 個方角完全 OS-level transparent,
  不依賴 compositor、沒 AA、無 half-alpha)

效果:X11 上 Mori 跟 Wayland 視覺對齊(都是圓盤),aura / drop-shadow /
glow 全保留(opaque body 內 composite)。

### Multi-monitor wander 限制單螢幕

`walkOnce()` 原本用 `primaryMonitor()` 拿尺寸,只認 primary monitor。Mori
被使用者拖到第二螢幕後,wander 仍用 primary 邊界算 → 走到看不見的座標。

改用 `availableMonitors()` 找 Mori 中心點所在的 monitor,wander 限制在那台
範圍內。設計哲學:「Mori 待哪台就在那台 wander,使用者手動拖才換螢幕」 —
不會跨螢幕亂跑,使用者也保留掌控權。

### 變動檔案

- 新檔 `crates/mori-tauri/src/x11_shape.rs`
- 修改 `crates/mori-tauri/Cargo.toml`(加 x11rb shape feature)
- 修改 `crates/mori-tauri/src/main.rs`(模組宣告 + find_window_xid helper +
  X11 path 內 XShape apply)
- 修改 `src/FloatingMori.tsx`(walkOnce 用 availableMonitors + 中心點偵測)

---

## 5R-followup-2 — picker / chat-bubble 在 X11 多細節修(2026-05-13)

5R 上完後實測一輪,挖出 X11 + multi-monitor + alwaysOnTop 一連串小坑:

### 修法

- **picker 多螢幕找不到視窗** — `centerOnPrimaryMonitor()` 用 `currentMonitor()`
  抓 picker 目前所在的螢幕,但 picker 初始位置由 Tauri 自選,落在 user 不在
  看的那台螢幕 → user 按 Ctrl+Alt+P 看不到視窗以為失效。改成 `centerOnActiveMonitor()`:
  `cursorPosition()` 抓滑鼠所在螢幕,fallback primary,fallback 第一個 monitor
- **picker X11 第二次以後 focus 失靈** — 原本 close 用 `setPosition(off-screen)`
  + `setSize(1,1)` 偷渡(Wayland focus 救援);X11 上 window 始終 mapped,
  下次 show() 是 no-op、setFocus() 被 mutter focus-stealing-prevention 拒。
  改成 X11 path 用真 `hide()`/`show()`,window unmap + remap 觸發 mutter 自動
  把 focus 給新 mapped window
- **picker X11 卡片中間有空 bg** — 即使 body bg = card bg 同色,card 92%
  width / 4% gap 加上 card 自身 border line,視覺上像「框內又有框」。X11 上
  用 `position: absolute; inset: 0` 強制 card 撐到 100% window,card border
  直接當 window 邊框;carousel-body 加 `flex: 1` 撐開到 footer,沒底部空 bg
- **chat-bubble X11 底部露白** — `.mori-chat-window` 沒設 height,JS 用
  `MIN_HEIGHT=56` floor + 短文字時 window > card → 底部露出 body bg。light
  theme `surface-bg #FFFFFF` 純白超明顯。改用 `min-height: 100vh` 讓 card 至少
  跟 window 同高,內容變長 card + window 一起長(JS measure 仍正確收斂)
- **chat-bubble 偏左** — `showChatBubble` 用 `outerPosition()` 算座標,mutter
  X11 transparent+decorationless 視窗會把 shadow margin 算進去 → bubble 偏左
  (shadow margin 寬度的偏移)。改用 `innerPosition()` 拿 content 真實 top-left,
  水平正確置中。drag-end 同步 chat_bubble 位置的 emit 也跟著改
- **chat-bubble 被壓在 floating 下面** — 兩個視窗都 `alwaysOnTop:true`,X11
  mutter 同 ABOVE layer 內順序看「誰最後被 raise」。floating 因為使用者互動
  頻繁(hover/drag)raise event 較新會壓在 chat_bubble 上。setAlwaysOnTop
  toggle(false→true)只翻 state 不 re-raise,mutter 不會在 layer 內 reorder。
  唯一可靠是顯式 `XRaiseWindow` — 新增 `force_raise_window` Tauri command
  shell-out `xdotool search --pid $$ --name "Mori (chat)" windowraise`,
  ChatBubble.tsx show 完 invoke 一下
- **starter USER-00** — 5R 只 ship USER-01。USER-00(slot 0 預設極簡聽寫)
  沒實體檔,Alt+0 切過去沒 display name → floating sprite 頭上沒 chip。
  補 ship `USER-00.純文字輸入.md`(內容對齊 FALLBACK_PROFILE_MD 但可讓 user
  編)。同樣 include_str! 從 examples/ 編進 binary
- **bootstrap `floating` section** — 補進 `~/.mori/config.json` stub:
  `floating.animated: true / wander: false`。原本 React 端 `?? false` fallback
  穩,但 user 看 config.json 不知道有這欄位可改;explicit 寫進 stub 提示存在
- **tracing 加碼** — `x11_hotkey::dispatch` 加 `tracing::debug!(?action,
  "x11 hotkey fired")`,picker listener 加完整 chain log,debug 時不用瞎猜按
  鍵到底有沒有打進來

### 變動檔案

- `src/Picker.tsx`:`centerOnActiveMonitor` + X11 hide/show / Wayland setPosition
  雙 path、is_x11_session 偵測
- `src/picker.css`:X11 absolute inset:0 fill + carousel-body flex:1
- `src/chat-bubble.css`:X11 `min-height: 100vh` + 方角
- `src/FloatingMori.tsx`:`showChatBubble` / drag-end 改用 innerPosition
- `src/ChatBubble.tsx`:show 後 `invoke('force_raise_window')`
- `crates/mori-tauri/src/main.rs`:`force_raise_window` Tauri command,
  picker listener 加 tracing
- `crates/mori-tauri/src/x11_hotkey.rs`:dispatch 加 tracing
- `crates/mori-core/src/voice_input_profile.rs`:ship USER-00 starter
- `crates/mori-core/src/llm/groq.rs`:bootstrap stub 加 `floating` section

---

## 5R — Starter profiles + 基本操作流程文件(2026-05-13)

5Q 把 23 個全域熱鍵都接通了,但使用者實測時發現兩個 UX 缺口:

1. **fresh install 只有 slot 0**(`AGENT.md` + 內建 voice fallback),按
   `Alt+1` / `Ctrl+Alt+1` 全部 fallback,沒有「試試看 slot 切換」的對象
2. **README / docs hotkey 表只列鍵不講流程**,使用者不知道日常用法是「先
   選 mode、按 space 錄音、再按 space 送出」這個序列

### 修法

**Starter profiles** — `ensure_agent_dir_initialized()` 跟
`ensure_voice_input_dir_initialized()` 改成除了預設 slot 0 之外,也寫一份
slot 1 starter。檔案內容透過 `include_str!` 從 `examples/` 編進 binary,
冪等:已存在不覆蓋,使用者刪除 / 改動都會保留。

- **`AGENT-01.翻譯助手.md`** — 翻譯範本(provider: groq + translate skill),
  跟對話 mode 區分明顯,使用者一試就懂兩個 mode 差異
- **`USER-01.朋友閒聊.md`** — 放鬆語氣 + `enable_auto_enter: true`
  自動 Enter 送出,frontmatter 用了 `cleanup_level` / `paste_shortcut` /
  `enable_auto_enter`,使用者改 markdown 就學會欄位用法

slot 2~9 仍由使用者自建(範本見 `examples/`)。

**基本操作流程文件** — `README.md` / `docs/getting-started.html` /
`docs/hotkeys.html` 都加「日常 4 個鍵打天下」段落,把「Alt+0 選 mode →
Ctrl+Alt+Space 錄 → 再 Ctrl+Alt+Space 送 → Ctrl+Alt+Esc 中斷 →
Ctrl+Alt+P 不記 slot 就用 picker」的序列講清楚。

### 變動檔案

- `crates/mori-core/src/agent_profile.rs`:`ensure_agent_dir_initialized`
  多寫 starter AGENT-01
- `crates/mori-core/src/voice_input_profile.rs`:`ensure_voice_input_dir_initialized`
  多寫 starter USER-01
- `README.md` / `docs/getting-started.html` / `docs/hotkeys.html`:新增
  「基本操作流程」段落

---

## 5Q-followup — X11 paste-back 走 xdotool(2026-05-13)

5Q 上完後使用者回報「轉錄結果沒貼到游標」。根因:paste-back path
hardcode `ydotool key`,Wayland-first 設計沒考慮到 24.04 X11 user 通常
沒裝 `ydotoold` daemon、也沒加 `input` group(ydotool 兩個前置條件)。

### 修法

`crates/mori-tauri/src/selection.rs::paste_back_for_process` 加 session
偵測(複用 `x11_hotkey::is_x11_session()`):
- **X11** → `xdotool key ctrl+v` / `ctrl+shift+v`(走 X server XTEST,
  無需 daemon / 無需 group 權限,Ubuntu 24.04 + X11 開箱可用)
- **Wayland** → `ydotool key 29:1 47:1 47:0 29:0`(走 uinput,維持原路徑)

兩條 path 拆成 `run_xdotool_paste` / `run_ydotool_paste` helper,error
handling 一致(失敗都 fallback 到 `PasteResult::ClipboardOnly`)。

`warn_if_setup_missing()` 啟動健康檢查也跟著分流:X11 檢 `xclip` +
`xdotool`、Wayland 檢 `xclip` + `ydotool`,缺哪個就給對應的 install hint。

Setup script 連動:X11 user 只需要 `sudo apt install xclip xdotool`,
不用整套 setup-wayland-input.sh。

### 變動檔案

- `crates/mori-tauri/src/selection.rs`

---

## 5Q — X11 session 支援 + 自訂熱鍵(2026-05-13)

Ubuntu 24.04 LTS + X11 session 跑得起來了。Mori 原本 Wayland-only(走
`xdg-desktop-portal` GlobalShortcuts,需要 portal 1.19+ 才有
`host.portal.Registry` interface),24.04 ship 的 portal 1.18 沒這個
interface → 整個全域熱鍵掛掉。同時 X11 + WebKit2GTK 對 transparent
floating window 的 half-alpha pixel 渲染不對,sprite 周圍會有黑/白方框。

兩個問題分別有 fallback:**X11 session 直接走 `tauri-plugin-global-shortcut`
的 XGrabKey 路徑**繞開 portal,**透明視窗用 opaque 卡片**(body 純色 +
inset frame)避開 half-alpha 渲染。整套順手把熱鍵改成 user 可在
`~/.mori/config.json` 自訂。

### 設計重點

- **單一 session 偵測點**:`x11_hotkey::is_x11_session()` 讀 `XDG_SESSION_TYPE`,
  兩個 fallback path(熱鍵 + 透明)共用同一個判斷。XWayland(`wayland` session
  跑 X 程式)**仍走 portal**,因為 Wayland compositor 不會把 XGrabKey 全域
  key 送給 XWayland client、且仍受 ARGB 渲染問題影響
- **熱鍵 path 統一 event 介面**:portal_hotkey 跟新 x11_hotkey 都 emit 相同
  Tauri events(`PORTAL_HOTKEY_EVENT` 等),下游 listener 不用知道現在跑哪條
  path,呼叫端 `main.rs` 單一 if/else 切換
- **HotkeyConfig hybrid defaults + overrides schema**:預設整套不寫,要改才
  寫;voice/agent slot 0~9 共用 modifier(`Alt` / `Ctrl+Alt`),個別 slot
  可 override 成任意鍵
- **衝突偵測 + 語法驗證**:啟動 resolve config 時兩個 action 綁同鍵直接 abort,
  modifier 順序歸一化(Ctrl+Alt+P == Alt+Ctrl+P 視為衝突),單鍵 grab 失敗
  log warn 跳過不影響其他
- **X11 fallback class 注入走 React invoke 而非 Rust eval**:Rust eval 只能
  startup 跑一次,使用者 reload webview 後 class 沒了 → 黑框回來。React 每次
  mount 呼叫 `is_x11_session` Tauri command,reload 也會重新加 class
- **Session diagnostic 顯示在 status modal 而非 Config tab**:read-only 環境
  資訊跟 build SHA / provider 並列,user 報 bug 截圖即一目了然;Config tab
  保持「編輯設定」單一職責

### 子改動(時序)

- **`5Q-1`** HotkeyConfig schema + parser
  - 新檔 `crates/mori-tauri/src/hotkey_config.rs`:`HotkeyConfig` struct
    (serde + Default)、`HotkeyAction` enum、`HotkeyBinding` resolved 形式、
    `to_portal_trigger()` 把 `Ctrl+Alt+P` → `CTRL+ALT+p`(X11 keysym 格式)、
    `normalize_for_compare()` 歸一 modifier 順序做衝突檢測
  - 6 個 unit test(default 不衝突 / portal format / 衝突偵測 / slot override
    / modifier swap 歸一 / 無效鍵拒絕)
- **`5Q-2`** X11 session path
  - 新檔 `crates/mori-tauri/src/x11_hotkey.rs`:`is_x11_session()` +
    `register()` 用 `tauri-plugin-global-shortcut` 註冊全 23 鍵(toggle +
    cancel + picker + 10 voice slot + 10 agent slot)
  - `main.rs` setup callback 改成 `if x11_hotkey::is_x11_session()` 分支,
    Linux non-X11 仍走 portal_hotkey
- **`5Q-3`** portal_hotkey 改吃 HotkeyConfig
  - `portal_hotkey::run(app, config)` 收 HotkeyConfig 參數,所有 trigger
    從 `config.resolve()` 算出來,不再 hardcode `PREFERRED_TRIGGER` /
    `format!("ALT+{n}")`
- **`5Q-4`** bootstrap stub 寫進 `hotkeys` section
  - `crates/mori-core/src/llm/groq.rs` `bootstrap_mori_config()` 新增
    `hotkeys` 子樹,預設 toggle/cancel/picker + voice/agent slot modifier +
    空 overrides。User 看 `~/.mori/config.json` 就知道有什麼可以改
- **`5Q-5`** X11 透明 fallback CSS
  - `src/floating.css` / `chat-bubble.css` / `picker.css` 加
    `body.<window-name>.x11-fallback` selector,把 transparent bg 換成 opaque
    (對角漸層 page-bg → surface-bg + inset frame),floating 還強制
    sprite-area 置中(原本 fixed top-left,mutter 對 transparent+decorationless
    視窗的 inner/outer size 有時差幾 px 造成偏位)
  - **WebKit2GTK X11 ARGB 半 alpha 行為**:任何 alpha ≠ 0 / ≠ 1 的 pixel
    都會被 X11 渲染成不透明,造成 sprite 周圍黑/白方框。解法:body bg 純
    opaque,所有 aura / glow / drop-shadow / blur 都在 opaque body 內 composite
    完才送 X11 → 輸出全 alpha=1。Tradeoff:X11 上 Mori 是方塊面板而不是真
    floating sprite(沒 OS-level 圓角,要 XShape 才能做,留下版)
  - Iteration history:試過背板美術 PNG / 玻璃 specular / 半透明 0.85 alpha /
    border-radius 50%,都因為各自原因(太花 / 太暗 / 雙層渲染 / 角落 AA 半
    alpha 留方框)被回退到最簡乾淨漸層
- **`5Q-6`** React 端 X11 偵測 + class 注入
  - 新增 `is_x11_session` Tauri command
  - `src/main.tsx` 在每個 window mount 前 `invoke<boolean>("is_x11_session")`,
    true → 加 `x11-fallback` class 到 `<html>` + `<body>`。reload 也會重套
  - Rust setup 那邊原本的 startup eval 注入作 belt-and-suspenders 留著
    (idempotent;首次啟動更快,React invoke 是 reload 安全網)
- **`5Q-7`** Status modal 顯示 session info
  - 新增 `linux_session_type` Tauri command,回傳 `"x11"` / `"wayland"` /
    `"linux-other"` / `"non-linux"`
  - `ChatPanel.tsx` ⚙️ status modal 加 `session` row,顯示 session type +
    走哪條 hotkey path(`x11 · tauri-plugin-global-shortcut (XGrabKey)` /
    `wayland · xdg-desktop-portal GlobalShortcuts` / 失效情況)。User 回報
    bug 截這張即一目了然

### 變動檔案

- 新檔
  - `crates/mori-tauri/src/hotkey_config.rs`
  - `crates/mori-tauri/src/x11_hotkey.rs`
  - `docs/design/mori-floating-backplate.png`(美術背板,目前 CSS 未啟用,
    `public/floating/backplate-x11.png` 已就位,要回滾一行 CSS 即可)
- 修改
  - `crates/mori-tauri/src/main.rs`:模組宣告 / setup 兩條 hotkey path /
    `is_x11_session` + `linux_session_type` commands + invoke_handler 註冊
  - `crates/mori-tauri/src/portal_hotkey.rs`:改吃 HotkeyConfig
  - `crates/mori-core/src/llm/groq.rs`:bootstrap stub `hotkeys` section
  - `src/main.tsx`:invoke is_x11_session + 加 class
  - `src/floating.css` / `chat-bubble.css` / `picker.css`:x11-fallback selector
  - `src/ChatPanel.tsx`:status modal session row
- Docs(下個 commit)
  - `README.md`:Quick Start 補 `npm run build`、平台需求加 portal 1.19 提醒
  - `docs/getting-started.html`:三步驟順序修正、系統需求加 portal 版本
  - `docs/hotkeys.html`:23 鍵清單對齊、X11 vs Wayland path 分流說明、自訂
    熱鍵章節(欄位 / 支援鍵名 / 生效時機 / 衝突偵測)
  - `docs/mori-home.html`:config.json 欄位列表補 hotkeys
  - `docs/troubleshooting.html`:`#portal-registry` 章節加 X11 自動偵測修法

### 已知限制

- **OS-level 圓角**:CSS `border-radius` 在 X11 透明視窗邊緣 AA 仍會碰到原本
  half-alpha 問題。要真圓角必須 XShape clip(Rust + x11rb shape feature),
  下版做。目前 X11 上 floating 是矩形面板
- **Multi-monitor wander**:floating sprite 隨機走動邏輯沒檢查多螢幕可視
  區域,有可能走到完全看不到的座標(只看 primaryMonitor)。X11 fallback 下
  opaque card 走來走去本身也奇怪,建議搭配「X11 force-off wander」一起修
- **bootstrap stub `floating.wander` 沒明寫**:預設行為仰賴 React `?? false`
  fallback,沒問題但建議下版補進 stub 讓 user 看 config.json 一眼看到欄位
- **Wayland portal trigger 改 config 後**:portal 規範實際綁定由 compositor
  記住,Mori config 改了不會自動覆寫 → 要重啟 + `rm -rf ~/.local/share/xdg-desktop-portal/permissions`
  讓 Mori 重新註冊。詳見 [docs/hotkeys#customize](https://yazelin.github.io/mori-desktop/hotkeys.html#customize)

---

## 5P — Sprite 4×4 engine + Character pack 系統(2026-05-13)

Floating Mori 從 single-frame static PNG 升級成 **4×4 sprite sheet animation**,
且整套 sprite + 設定包成「character pack」讓 user 可替換角色。設計目標:讓
yazelin 另開的 sprite generator app 能輸出**完全符合規格**的 `.moripack.zip`,
其他 user import 即可。

### 設計重點
- **同一份 sprite 兩種模式**:`floating.animated: bool` 純 CSS 切換動 / 不動,
  不用兩份檔
- **Schema versioning + portable metadata**:`manifest.schema_version` 讓未來
  schema 改不破壞舊 pack
- **不依賴外部工具**:ensure_default + upgrade_pack_to_4x4 都用 Rust `image`
  crate inline 做,user 不用 sudo apt install ImageMagick
- **動 / 拖曳分層 toggle**;走動 toggle persist 但 logic 下版接

### 子 commit(時序)

- **5P-1 `39d1056`**:character_pack 規範 + default mori ensure + Tauri IPC
- **5P-2 `1446321`**:ensure_default 升 4×4 placeholder(Rust image crate)+
  upgrade_pack_to_4x4 IPC + 2 unit tests
- **5P-3 `2e417fd`**:FloatingMori 改讀 character pack + CSS 4×4 雙軸 step
  engine(取代原 spec 的 buggy 斜對角 keyframes)
- **5P-3 fix `a0fe0cf`**:啟動 fallback 到 public/floating/ 避免一閃
- **5P-4 `0db2e65`**:Config Floating section animated / wander toggle + persist
  + config-changed event 即時生效
- **5P-5 `a6d81d3`**:拖曳偵測 + .is-dragging CSS「被拎起懸空」視覺(scale +
  rotate + drop-shadow + animation paused)
- **5P-6 `d260400`**:ConfigTab character picker + 升級 button + docs/character-pack.md
  完整規範
- **5P-7 `3980a60`**:walking 邏輯實作(setPosition 隨機 + 1.5s 平滑插值移動 +
  邊界檢測)+ 拖曳 mouseup window-level listener 保險(Tauri start_dragging 接管
  後 React onMouseUp 不一定 fire 的 bug)+ IPC sprite fallback chain 升級 4 階
  (自己 pack 同 state → mori 同 state → 自己 idle → mori idle)
- **5P-8 `2d5d137`**:首輪 flicker fix attempt — 把 keyframes `to: -400%` 改
  `-300%` 想避開 wrap blank。**方向錯**(下個 commit 修)
- **5P-9 `d6ca1ee`** 真 flicker fix:CSS `background-position` 百分比公式是
  `pixel_shift = percent × (container - image)`,負值會把 image 推 off-screen
  完全 blank。改用**正百分比** `to: 100%` map 到 cell 0..3,配 `steps(4, jump-none)`
  整 cycle 都在 image 範圍內。User 看到的「持續 cycle 出現消失」徹底修
- **5P-ux `8c76eb6`**:Config tab UX 修 — 兩顆 Save 按鈕標籤區分
  (「儲存 config.json」vs「儲存 corrections.md」)、頂端 action bar 改 sticky
  (滾到下面 Floating Mori section 仍看得到主 Save,避免誤點 corrections.md
  那顆 Save)
- 中間還有 `3b53a6d` / `3294dcd` / `2e83350` 三個 debug commit(console.log +
  視覺 dirty indicator)定位 root cause 後在 5P-ux 撤掉,留作 git history 紀錄

### Sprite generator workflow 預備
- `docs/character-pack.md` 完整 schema + frame order + design tips
- `docs/floating-sprite-spec.md` 修正 keyframes 為正確 row-major 雙軸寫法
- ConfigTab Floating section 提供 active 切換 + 升級 button

### 已知限制(交給 future commit)
- **One-shot 模式**(done / error)engine 簡化都當 infinite loop,正式 sprite
  上來再接 `animation-iteration-count: 1` + `fill-mode: forwards` 停 frame 16
- **正式 sprite 美術資產**:目前 placeholder 是 16 cell 全填 frame 1,動畫 ON
  看起來不動;等 yazelin 另開的 generator app 出真 16-frame walking / dragging /
  其他動作 sheet,直接覆蓋同檔名就會動
- **個人化光效顏色 picker** / **chat bubble 顯示 toggle** / **profile chip
  顯示 toggle** — 拆下版「Floating settings page」做
- **Import / Export `.moripack.zip` UI** — 等 generator app 有 ZIP 輸出再接

## docs(roadmap): 5E-2 scope 精準化 — 不只是 paste-back(2026-05-13)

第二輪 audit 之後 user 問:「5E-2 是不是寫了 paste-back 就直接支援 Win/Mac
voice input?」答案是**不**。

原本 roadmap 描述只提 paste-back,讓人以為單塊 work 就行。實際上 Win/Mac
要支援完整 voice pipeline 需要**三塊** platform-specific code:

| 子項 | Linux 現況 | Win/Mac 缺 |
|---|---|---|
| selection capture(反白文字) | xclip -selection primary | Win GetClipboardData / Mac NSPasteboard |
| window context(焦點 app/title) | xdotool getactivewindow | Win GetForegroundWindow / Mac NSWorkspace |
| paste-back(模擬 Ctrl+V) | arboard + ydotool | Win SendInput / Mac NSEvent + accessibility permission |

Roadmap entry 改名為「Win/Mac voice pipeline 完整支援」+ 表格列三子項,
讓未來 contributor 看到完整 scope 不會誤判工作量。

也強調為何走 contributor pathway 而非 yazelin 自寫:沒 Mac/Win 主力環境,
寫了測不到 = 不知對錯。架構面 mori-core 純邏輯已跨平台 + 平台殼分離,新增
selection_macos.rs / selection_windows.rs mod 就能接,已備好基礎。

## docs(roadmap): prune 5G-10 / 3C / 3D 三條(2026-05-13)

Roadmap audit 第二輪 — 把「真的要做」跟「可能不做」混在一起會誤導讀者
(以為 Mori 還會出這些)。三條一起砍,理由寫明:

- **5G-10 Profile 自動遷移** ← 砍。5G voice/agent 設計分家時想做的 migration
  helper(掃 voice_input/ 找 Type B flags → 自動搬到 agent/)。**已過期**:
  - User 已手動完成所有遷移(`~/.mori/voice_input/_migrated_to_agent/`)
  - 新 user 從 `examples/` 起跑,範本本來就 correct
  - 遺漏的舊 user 看到 `main.rs:1493` 的 warn message 會知道該搬
  - 這是「一次性升級工具」性質,過了升級窗口 obsolete
- **3C Pure Wayland 跨 app 反白** ← 砍。Wayland session 沒 X11 PRIMARY
  等價,擔心未來 Hyprland / 純 Sway / GTK4-only app 反白抓不到。**已被
  XWayland + xclip 解掉 90%**(5O xclip 主路徑):
  - Ubuntu 26.04 + GNOME Wayland 預設啟 XWayland(99% user)
  - xdg-desktop-portal Selection API 規格還在演進,實作成本高
  - 沒 user 在抱怨「Hyprland 反白抓不到」 — 沒 demand signal
- **3D 螢幕擷取進 context** ← 砍。「Mori 看畫面」第三層感知能力。
  **多模態 LLM 在 Mori provider stack 不友善 + 隱私破口大 + 跟既有第二
  層(剪貼簿 + 反白)重疊度高**:
  - Groq 沒 vision endpoint;Ollama vision 要 GPU;Claude-bash 慢
  - 截圖風險:可能含密碼 / 私訊 / 隱私視窗
  - 觸發機制設計難:「每次都截」太貪 / 「按新 hotkey 才截」要新 UI
  - 80% use case 已被「user 自己反白要問的部分 → 剪貼簿」cover
  - 多模態 LLM 普及 + Mori provider stack 有支援後再評估
  - 順便:3D 子項 active window title 早已 done(`HotkeyWindowContext`),這次 cleanup 一起劃掉

Roadmap 從此 = 「真的會做的事」,不是「想過但可能不做的事」。後者紀錄在
CHANGELOG 這條 entry 內,以後想重啟可以回頭找。

## 3B-2 — YouTube transcript shell_skill 範本(2026-05-13)

Roadmap 上「3B-2 YouTube transcript skill」原本規劃做成 built-in skill,
這版實際落實為 **shell_skill 範本路徑** — 改邏輯都不用動 Rust,user 自己想
換語言優先序 / cap size / 字幕清理規則改 sh 就好,跟 mori 主程式解耦。

- `examples/scripts/mori-youtube-transcript.sh` — yt-dlp wrapper:
  - 抓 auto-subs + manual subs,語言優先 `zh-TW > zh-Hant > zh-Hans > zh > en.* > en`
  - srt → 純文字(去序號 / 時間軸 / HTML tag / 空行 / 連續重複)
  - 30KB cap(1 小時影片大約 20-40KB,避免 LLM context 爆)
  - PATH 自動加 `$HOME/.local/bin`(Deps tab 裝的 yt-dlp 落在那)
  - 錯誤訊息分類(yt-dlp 沒裝 / 影片無字幕)
- `examples/agent/AGENT-04.YouTube 摘要.md` — profile + shell_skill 框架:
  - `provider: claude-bash`(摘要任務 reasoning 重要 + user 自己 quota)
  - 一條 `youtube_transcript(url)` shell_skill timeout 90s
  - System prompt 明確要求 hook + 3-5 bullet + 結論的繁中摘要
  - 截斷時要備註「字幕後半未涵蓋」
- `docs/profile-examples.html` 加 AGENT-04 card,展示這個 pattern
- roadmap 3B-2 段砍掉(實作完成)

## docs: roadmap cleanup 對齊實際 code(2026-05-13)

Roadmap audit 抓到三條跟 code 實際狀態不對齊,訂正:

- **5E-2 OpenCC 簡→繁** — 砍掉。`voice_cleanup.rs:25` 內 comment 早就明確
  說「為什麼**沒**做 OpenCC」(whisper-rs initial_prompt 已 bias 繁體實測
  夠用),roadmap 卻仍列「未來規劃」自相矛盾
- **3D active window title** — 已實作但 roadmap 仍列為未來,標 done。
  `HotkeyWindowContext.process_name + window_title` 透過 xdotool 在熱鍵
  瞬間捕獲(`main.rs:851`),Mori 看得到當前焦點。3D 條改成「螢幕擷取
  進 context」單一未完成子項
- **3C 跨 app 反白** — 描述改清楚:X11 + XWayland 用 xclip + PRIMARY 已
  cover 90%(5O xclip 主路徑生效),只剩 pure Wayland(原生 GTK4 /
  Hyprland 等沒 XWayland 的 session)需走 xdg-desktop-portal Selection
  介面 — 實用 use case 不大,等 Wayland 生態普及再說
- **3B-2 YouTube transcript** — 描述補充 yt-dlp 已可從 Deps tab 裝(5K-3
  加進去),差 skill wrapper;路徑改成 examples/agent/ shell_skill 範本

## 5A-3b — Per-context opt-in LLM fallback chain(2026-05-13)

主 provider 失敗(Groq 429 quota / timeout / network / 5xx)以前只能 Phase::Error
+ user 手動切 config — 過去 roadmap 的「auto-fallback chain」設計回到桌上一輪後,
user 自己抓出設計問題:「沒配置 fallback 前不該自動切,silent 切會傷透明度」。
這版接受該回饋,做成 **opt-in、per-context** 機制:

- **Schema**:`~/.mori/config.json` `routing.fallback_chain.{agent,voice_input_cleanup}`
  各自一個 provider name list。沒設 = 維持原行為(error + cancel)
- **觸發**:任何 `provider.chat()` 回 `Err`(quota / timeout / network / 5xx)都
  triggers fallback。`Ctrl+Alt+Esc` 中斷不 triggers(那是 user intent,直接砍
  pipeline)
- **Agent 模式 option (a)**:fallback 觸發後 `respond_with_mode` 在新 provider
  上從頭重跑(避免 `tool_call_id` 跨 provider 認不得 — groq `call_xxx` /
  ollama incrementing / claude-bash 自訂格式都不同,mid-turn 切會 400)。所以
  agent 模式 fallback 只 cover「第一次 LLM call 失敗」場景
- **VoiceInput cleanup**:單輪 chat call,直接 wrap `chat_with_fallback`
- **Build 失敗的 fallback provider**:warn + drop,其他 fallback 仍可用 — 不擋整個
  routing build。Agent / skill 的 hard provider build 失敗仍 abort(行為不變)
- **Transparency**:fallback 觸發時 ChatPanel 渲染一行系統訊息「`groq` 失敗,
  自動改用 `ollama` 重試 — &lt;原因&gt;」;FloatingMori chip 透過既有
  `voice-input-status` channel 即時改顯示新 provider 名;`provider-changed`
  event 同步 emit 給其他 UI consumer。下次 pipeline 開始(recording / transcribing)
  時 ChatPanel 自動清掉舊 system message
- **New `mori_core::llm::chat_with_fallback(chain, messages, tools, on_fallback)`**:
  sync callback signature(mori-core 不依賴 Tauri AppHandle),caller 拼好
  primary + fallback 一條 slice 傳進來
- **9 unit tests**:RoutingConfig.fallback_chain 5 個解析 edge case + chat_with_fallback
  4 個 scenarios(primary succeed / fallback succeed / all fail / empty chain)
- **docs**:`docs/providers.html` 新「進階:Fallback chain」段附範例 + 全規則;
  `docs/roadmap.md` 5A-3b 整段砍掉

## 5E-3 — VoiceInput 可選載入相關記憶 / voice_dict 校正詞庫(2026-05-12)

VoiceInput cleanup pipeline 過去純單輪 LLM 轉換,完全不參與 memory(`remember`
/ `recall_memory` 只在 Agent 模式)。痛點:Whisper 一直把「Annuli」翻成
「安奴利」/「安列利」、人名 / 公司名常被翻錯,user 沒有地方放「校正詞庫」。

5E-3 開:**VoiceInput read-only inject memory by type**(寫入仍只走 Agent)。

- **新 `MemoryType::VoiceDict` variant**:校正詞庫專用,跟 user_identity /
  preference / project / reference / skill_outcome 並列為 first-class type。
  `MemoryType::as_str()` + `MemoryType::parse()` 集中 stringify / parse,
  DRY 掉 markdown.rs + main.rs 兩處重複(`5E-3a`)
- **`MemoryStore::list_by_types(&[MemoryType])`** trait default impl:
  逐檔 frontmatter 過濾(memory 通常 <50 篇,IO 量小不需 cache)
- **Voice profile frontmatter 新鍵 `inject_memory_types: [voice_dict]`**:
  inline array 寫法。`None`(沒寫)→ 走 config 全域 fallback;`Some(vec![])`
  → 強制不 inject(即使 config 全域有設)。新增 hand-rolled
  `parse_inline_string_array` helper(`5E-3b`)
- **全域 `config.json` `voice_input.inject_memory_types`** 作為 profile 沒設時
  的 default。`resolve_inject_memory_types(profile)` 統一 fallback 鏈,各 string
  經 `MemoryType::parse` 轉
- **Voice pipeline 注入點**(`crates/mori-tauri/src/main.rs:1449` 附近):
  只在 `cleanup_level: smart` fetch memory(minimal/none 跳 LLM 不需要);
  新 `build_voice_dict_section()` helper 拼成「校正參考」段落,提示 LLM
  「這是參考詞表,**不是** user 想說的話,不要照搬輸出」。失敗 fallback 空
  字串,不擋 cleanup pipeline(`5E-3c`)
- **UI**:`MemoryTab` `TYPE_OPTIONS` 加 `voice_dict`(user 能建這型 memory);
  `ProfileEditor` 加 `MemoryTypeChipsEditor` 多選 chips,讓 voice profile
  勾選要 inject 哪些 type;`ConfigTab` voice_input section 加全域 fallback
  chips(`5E-3d`)
- **7 unit tests**:list_by_types filter / MemoryType parse + roundtrip /
  inline array parser / profile inject_memory_types Some/None/empty / resolve
  fallback chain
- **docs**:`docs/memory.md` 加完整 type 對照表 + `voice_dict` 範例 +
  Agent remember → VoiceInput inject 串接流程

## 5N — voice profile 鍵大小寫整合 + 自訂 OpenAI-compat 端點(2026-05-12)

把過去散在每張 voice profile 的 `ZEROTYPE_AIPROMPT_*` frontmatter(端點 + key
+ model 三件套)收編進 `~/.mori/config.json` 的 `providers.<name>` 機制 — Azure
OpenAI / OpenRouter / 任何 OpenAI-compat 端點都當「具名 provider」用,profile
只負責寫 `provider: <name>`。順便把 voice profile parser 統一改 case-insensitive,
修一個潛藏 bug。

- **自訂 provider** — `build_named_provider` 在 5 個 hard-coded 名字未命中時,
  改去查 `/providers/<name>/api_base`,有就視為 OpenAI-compat;讀
  `api_key_env`(指 OS env 或 `api_keys.<name>`)+ `model` 組
  `GenericOpenAiProvider`。`build_chat_provider` 同步放寬 allowlist,失敗才
  fallback 到 groq + warn(`e6ec534`)
- **Parser case-insensitive** — voice profile frontmatter 鍵全部 case-insensitive,
  canonical 寫法是 lowercase snake_case;SCREAMING_SNAKE 形式偵測到 warn 一次
  +列出遷移路徑(`872e12d`)
- **修隱性 bug**:過去所有 `USER-XX` profile 的 `enable_read: true` 因 parser 只
  認 SCREAMING_SNAKE 都被 silently 忽略,`#file:` 預處理一直沒生效。5N 起會
  真的走 ReadSkill 路徑
- **ProfileEditor 收尾** — 移除 ZEROTYPE_AIPROMPT_* details 區塊(改在
  config.json 編);`enable_auto_enter` 用 lowercase canonical patch key;provider
  Select 動態補 option 讓自訂 provider 名(如 `azure-gpt41`)顯示得出來
- **6 unit tests**:custom OpenAI-compat happy path、missing api_base、missing
  api_key、lowercase enable / mixed-case / lowercase zerotype alias
- **docs**:`providers.html` 加「自訂 OpenAI-compat 端點」段落,範例貼 Azure +
  OpenRouter 兩種寫法
- **舊鍵保留**:`ZEROTYPE_AIPROMPT_*` 跟 SCREAMING `ENABLE_*` 仍作 deprecated
  alias 解析,下個版本移除

## agent reliability / ZeroType bridge(2026-05-12)

ZeroType Agent profile 卡 `Phase::Responding` 永遠不退的 bug,沿線挖出多個
agent loop / shell_skill / LLM provider 的 reliability 問題,一併補:

- `shell_skill` 加 `kill_on_drop(true)` — Ctrl+Alt+Esc 中斷後子程序(xclip /
  ydotool)連帶 SIGKILL,避免殘留 process 影響後續 trigger(`5e8af7d`)
- `groq.rs` reqwest client 加 90s timeout — LLM call hang(stream 不結束 /
  API glitch)不再讓 agent loop 永遠卡(`68c381e`)
- 新 `AgentMode::Dispatch` profile flag — bridge profile 第一個 tool_call
  execute 後直接 Done,不再 round 第二輪 LLM(`bb831ca`)
- `generic_openai`(gemini)+ `claude_cli` + `bash_cli_agent` 同樣補 timeout
  (90s / 120s / 180s)(`17f6933`)
- 新 unit test:Dispatch / MultiTurn 路徑分別驗證,加 `AgentMode::from_str_or_default`
  parse cases(`71d3ac2`)
- `examples/agent/AGENT-03.ZeroType Agent.md` + `examples/scripts/mori-trigger-zerotype.sh`
  bridge pattern 範本入 repo;docs/profile-examples.html 加進這個 pattern 給其他
  ZeroType 學員 reference(`6f4703f`)

## brand-3 follow-ups(2026-05-12)

brand-3 後續的瑣碎收尾跟 fix:

- Custom Select 取代 native `<select>`(Linux webkit2gtk 的 GTK dropdown 配色
  被 system GTK theme 鎖死;自繪 dropdown 跨 theme 同步)(`84fad44`)
- 公式書 tagline 改「**靜靜記得 · 你經過的痕跡**」(從 SOUL.md 引言截取);
  OG image 重 render(`3dd0999`)
- Nav 統一兩套各自規範:docs nav 7 link(含 ~/.mori/ + Troubleshooting),
  design book nav 6 link(`dd18add`)
- `docs/profile-examples.html` Profile 範本頁 + `examples/agent/` + `examples/voice_input/`
  starter pack(`fb7a79b`)
- `docs/roadmap.md` 5E-3 VoiceInput 可選載入相關記憶(roadmap-only)(`667242c`)
- `docs/desktop-ui.html` Phase Status 對齊 brand-2 / brand-3 完成項;拿掉舊
  emoji 描述改 SVG `IconXxx` 名稱(`dd70dd4`, `21f703d`)
- Picker / chat_bubble hide 時加 `setSize(1, 1)` — 雙保險避免 transparent
  alwaysOnTop 視窗擋下面 app click hit-test(`81846d4`)
- `scripts/restart-dev.sh` + `npm run dev:restart` 一鍵 kill + 重啟 dev(`5498abe`)
- 12 張主畫面截圖 + main-dark.png caption 精準化(STT 收音不佳時 Mori 自我察覺,
  不是 perfect 對話)(`b37504f`, `0482cbc`)
- `docs/mori-home.html` ~/.mori/ 結構說明頁 + nav link

## brand-3 — 雙 theme + VSCode-like 自訂 theme(2026-05-12)

- CSS variables 重構:`shell.css` / `chat-panel.css` / `picker.css` / `chat-bubble.css` / `floating.css` 全部色值改用 `var(--c-*)`,`:root` 內置 Mori Dark fallback
- Theme 檔在 `~/.mori/themes/*.json` — 啟動時內建 `dark.json` / `light.json` 自動寫入,user 可複製改名做自訂
- 跨視窗同步:任一 window 切 theme 後 `emit("theme-changed")`,picker / chat_bubble / floating 全部跟著切
- UI:左下角 sun/moon quick toggle + Config tab Theme picker(列所有 themes + Reload + 顯示資料夾 path)
- Light theme 對比 override:skill kind badge / memory type chip / dep ok status / 解鎖 label / kbd 等 light 下 forest-deep 暗字
- 公式書 `brand.html` 增 forest-night `#1f3329` / forest-shadow `#172620` 兩階深綠 + Dual Theme System 整段

## brand-2 — line-art SVG icon + 全套 brand palette(2026-05-12)

- Sidebar 6 個 tab 從 emoji(💬📋⚙️📓🛠️📦)→ inline line-art SVG icon(stroke=currentColor)
- 整個專案 emoji 大掃除:ChatPanel / Picker / Profiles / Memory / Deps / Skills tab 的 emoji button / chip / section header 全換 SVG
- 主視窗從「tech dark + 天空藍」改成 forest palette:main #1f3329 / sidebar #172620 / modal #243a31 + forest active / cream text
- Chat bubble:user → sand 暖色 / assistant → forest-soft 葉脈綠
- `mori-sleeping` / `mori-error` / `mori-recording` PNG 上緣 ~y20 的橫線 artifact 抹掉(原稿 frame 殘留,sleeping 無 aura ring 蓋住才裸露)
- commits: `0b51788`, `3516131`

## brand-1 — 徽章 logo + 公式書(2026-05-12)

- Logo 徽章式設計:深綠 disc(#182622,從 logo 內部 wreath 採樣)+ 米色 stroke 縮 83% + 右移 20px 補同心
- 同一張 PNG 用到底:`public/logo.png` / `crates/mori-tauri/icons/{16~256}.png` / `docs/sprites/logo.png`
- 公式書 `docs/index.html` + brand / character / desktop-ui / tray 四頁 + `_book.css` 共用 CSS variables
- `.desktop` Icon 用 absolute path + StartupWMClass=Mori-tauri 修 dock 4 icon 堆疊
- commits: `4910510`, `afefbc0`, `033ee09`, `1369cd6`, `15d99e2`

## Phase 5O — Dependencies tab(2026-05-12)

- `~/.mori` 第 6 個 tab,偵測 + 安裝 optional 工具(yt-dlp / ydotool / xdotool / xclip / whisper-local / ollama)
- 區分 needs-sudo(顯示指令給 user 自己跑)vs 無 sudo(代執行 + 顯示 stdout/stderr)
- `crates/mori-tauri/src/deps.rs` registry,UI 可一鍵 install + reload check
- commit: `8ff7142`

## Phase 5N — Chat panel 重設計(2026-05-12)

- top bar(mode chip + provider · model + status 按鈕)/ scrollable thread / progress chip / bottom input bar 四層 vertical flex
- user 訊息靠右 / Mori 靠左,bubble + tool chip
- 從 fixed-position 改 inline thread,符合常見 chat app 操作習慣
- commit: `842ba80`

## Phase 5M — 主視窗 sidebar 架構(2026-05-12)

- App 從單一 chat 視窗變 6 個 tab(Chat / Profiles / Config / Memory / Skills / Deps)
- 左 196px sidebar + 右 main area,每個 tab 一個 React component(只 mount 當前 tab,省 IPC)
- commit: `f5be5a5`

## Phase 5L — Config / Memory / Skills UI(2026-05-12)

- **5L-1**:textarea raw editor(快速版)
- **5L-2**:`ConfigTab` typed form — 常用欄位 dropdown / input,Raw JSON 模式給 routing.skills 進階(`415617e`)
- **5L-3**:`ProfileEditor` typed form + `shell_skills` 表格 + frontmatter ↔ raw 雙向 sync(`a328213`)
- **5L-4**:`MemoryTab` browser/editor + `SkillsTab` skills inspector(列當前 profile 啟用的 builtin + shell skills)(`178b935`)
- **5L-5**:command ↔ parameters placeholder 一致性檢查 + memory 全文搜尋(rust backend) + config form 細項(`7eb4e94`)

## Phase 5K — Profile Picker + Tray submenu(2026-05-12)

- **5K-1**:Ctrl+Alt+P 開 picker overlay,3-item carousel(prev / current / next),↑↓ 切 + Tab 換組 + Enter 確認(`5edb3d7`, `40b51fc`)
- **5K-2**:tray menu 列出全部 voice / agent profile(超過 Alt+0~9 / Ctrl+Alt+0~9 上限的也能點)(`718dcc4`)
- Wayland focus quirk fix:picker / chat_bubble 第二次 hide/show 拿不到 focus → 改 setPosition 進出畫面(`527faeb`, `13f943f`)

## Phase 5J — 單層 profile + 統一 context + Gemini(2026-05-12)

- Profile 結構從「voice / agent 兩種 schema」統一為單層 frontmatter:`provider` / `stt_provider` / `enabled_skills` / `shell_skills` / `cleanup_level` 一套吃
- `build_context_section()` 在 Rust 端統一注入:timestamp / clipboard / selection / `urls_detected`,所有 provider 看到相同 context
- 新 named provider `gemini`(走 OpenAI-compat 端點)
- `chat_bubble` 獨立視窗(Wayland 單窗 setSize + transparent 不穩)
- `tauri-plugin-single-instance` 防 dev 重啟 orphan 並存
- commits: `42b41c3`, `927bb65`, `ed5a74a`, `2b2f52f`

## Phase 5I — skill_server 動態 registry(2026-05-12)

- `skill_server`(claude-bash / gemini-bash / codex-bash 共用 HTTP 入口)從 hardcoded 8 skill 改成每次 request 即時讀 active agent profile build 完整 registry
- `mori skill call <name> --args '{...}'` 通用 dispatch 子命令
- bash-CLI 系列現在跟 OpenAI tool-calling 系列(groq / ollama / claude-cli)看到一致的 action_skills + shell_skills
- commit: `efc4b99`

## Phase 5H — 使用者自訂 shell skill(2026-05-12)

- Agent profile frontmatter `shell_skills: [...]` 直接定義 CLI 包裝,不用改 Rust
- `command: ["gh", "pr", "list", ...]` 是 array(不走 shell parsing),`{{name}}` 替換是字面字串 — LLM 無法 escape
- `gh` / `docker` / `kubectl` / `yt-dlp` / 自家 script 任何 PATH 內 CLI 都能變 Mori 能力
- commit: `0149f67`

## Phase 5G — 雙模式架構(VoiceInput + Agent)(2026-05-12)

- 熱鍵 + profile 二維:`Alt+0~9` = VoiceInput / `Ctrl+Alt+0~9` = Agent
- VoiceInput:STT → 單輪 LLM cleanup → 貼游標(只做「字」)
- Agent:STT → multi-turn agent loop → chat 回應 / 動作執行(`open_url` / `send_keys` / 查資料)
- Action skills 搬到 mori-core Skill trait(原 `voice_input_tools` 刪除)
- 動態 `SkillRegistry`,profile 決定該載哪些 skill
- `#file:` 前置處理,profile body 可內嵌參考檔
- 共用 `corrections.md`,新增 default `AGENT.md`
- commits: `4aecf2a`, `a7552f5`, `939f9f5`, `546fc61`, `4bd01ed`, `6299e5f`

## Phase 5F — Voice input profile(2026-05-11)

- `Alt+1~9` 切 voice input profile(ZeroType 相容格式)
- Floating widget 重設計:音量光環 / ripple / 詳細狀態 chip(轉錄中 / 處理中 / profile 名稱)
- Voice input agent loop:9 個工具(translate / polish / 標點 / etc.)+ smart_paste
- `profile.stt_provider` 可覆蓋全域 STT 設定
- `xclip` 取代 `arboard` 消除 wl-clipboard portal 對話框
- Terminal paste-back 用 `Ctrl+Shift+V`
- `config.api_keys` 支援 `ZEROTYPE_AIPROMPT_API_KEY_ENV` fallback
- commits: `da61073`, `7632c81`, `3735c7f`, `3a43621`, `67d6b86`, `c94519c`, `dee74c9`, `92c2396`, `a6c0998`, `5570745`, `95005e6`

## Phase 5E — Voice-input mode(2026-05-08)

- Mori 變 LLM-powered dictation:STT → LLM cleanup(加標點 / 修幻聽 / 切段,prompt 鎖死不准改詞)→ 貼回游標
- 三級 cleanup:`smart`(LLM)/ `minimal`(純程式 post-process)/ `none`(raw whisper)
- commit: `117221a`

## Phase 5D — Bash CLI proxy + chat-only variants(2026-05-08~09)

- **5D-1**:`claude-bash` provider — claude CLI 當 agent,透過 Bash tool 呼叫本機 `mori` CLI dispatch skill(walk Pro/Max quota)(`f06ca0f`)
- **5D-2**:`gemini-bash` / `codex-bash` agent providers + memory skills 接上 Bash CLI proxy + tighten system prompt(`38e41ff`, `b95ed53`, `d48788a`)
- **5D-3**:`gemini-cli` / `codex-cli` chat-only 變體(no agent flag)— 給 per-skill routing 用(`eed104d`)

## Phase 5C — Local STT(2026-05-08)

- whisper.cpp Rust binding (`whisper-rs`)+ rubato 重採樣,100% 離線
- `stt_provider: "whisper-local"` + `model_path` 指向 GGML 模型(small 466MB 夠用)
- commit: `cd56c9e`

## Phase 5A — Multi-provider(2026-05-08)

- **5A-1**:`OllamaProvider` — 本機 LLM(qwen3:8b)+ tool calling + warm-up + `keep_alive=30m`(`c5a6289`, `2ff9e77`)
- **5A-2**:`ClaudeCliProvider` — `claude --print` chat-only subprocess(`3f5d948`)
- **5A-3a**:per-skill provider routing(agent + 個別 skill 走不同 provider)(`74222b9`)

## Phase 4B + 4C — 系統整合(2026-05-07~08)

- **4B-1**:Wayland-compatible global hotkey via `xdg-desktop-portal` GlobalShortcuts(GNOME 唯一可用)(`ced5c9e`)
- **4B-2**:Active / Background mode + tray toggle + `set_mode` skill(`afb7d20`)
- **4B-3**:Floating Mori widget + GNOME Wayland always-on-top fix(`44580c6`)
- **4C**:Primary-selection 讀取 + `ydotool` paste-back(Linux)(`45244b6`)

## Phase 3 — Context Capture(2026-05-07 / 2026-05-12)

- **3A**:剪貼簿自動進 context + system prompt(`874cdac`)
- **3B**:URL detection(純 Rust scanner)+ `fetch_url` skill(reqwest + HTML strip,8KB cap),trigger 鎖在「這個 / 這篇」指示詞(`d414186`, `268def8`)

## Phase 2 — 基礎 text skills(2026-05-07)

- `translate` / `polish` / `summarize` / `compose` 四個純 LLM skill
- commit: `0185cc8`

## Phase 1 — Voice MVP(2026-05-07)

- **1A**:Workspace + traits + docs scaffold(`a6d191e`)
- **1B**:Voice pipeline 端到端 — hotkey + mic + Whisper(`209ff6d`)
- **1C**:Chat-back pipeline + `LocalMarkdownMemoryStore` I/O(`d9d7455`)
- **1D + 1E**:`SkillRegistry` + `RememberSkill` + multi-turn + `RecallMemorySkill`(`45dfcc0`)
- **1F**:Conversation history + tray icon + forget/edit skills(`e72d37a`)
