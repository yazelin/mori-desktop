//! Wayland-compatible global shortcut, via xdg-desktop-portal.
//!
//! `tauri-plugin-global-shortcut` registers via the X11 keyboard-grab API,
//! which Wayland deliberately blocks. The official path on Wayland is the
//! `org.freedesktop.portal.GlobalShortcuts` DBus interface — the first
//! time we bind a shortcut, the compositor pops a permission dialog
//! ("Mori 想註冊 Ctrl+Alt+Space"). After the user grants it, key presses
//! arrive as DBus `Activated` signals; nobody has to grab the keyboard.
//!
//! Linux only. macOS / Windows use tauri-plugin-global-shortcut.

use anyhow::{Context as _, Result};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use ashpd::{register_host_app, AppID};
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};

/// App ID we register with the portal Registry. Must match
/// `tauri.conf.json` `identifier` so per-app portal permissions are
/// stored under one key. GNOME requires a registered app id before any
/// portal call that needs to attribute permissions to a specific app
/// (without it, GlobalShortcuts.CreateSession returns
/// `org.freedesktop.portal.Error.NotAllowed: An app id is required`).
const APP_ID: &str = "ai.yazelin.mori";

/// Stable id we register with the portal — the `Activated` signal
/// echoes this back so we can tell which shortcut fired.
pub const TOGGLE_SHORTCUT_ID: &str = "toggle";

/// 5J: 錄音中按 Ctrl+Alt+Esc 直接丟掉錄音(不送 STT、不進 chat)。
pub const CANCEL_SHORTCUT_ID: &str = "cancel";

/// 5K-1: Ctrl+Alt+P 開 picker overlay 選 profile(支援 9 個 slot 之外的)。
pub const PICKER_SHORTCUT_ID: &str = "picker";

/// Tauri event emitted when the toggle shortcut fires.
pub const PORTAL_HOTKEY_EVENT: &str = "portal-hotkey-fired";

/// Tauri event emitted when the cancel shortcut fires(錄音中丟棄)。
pub const PORTAL_CANCEL_EVENT: &str = "portal-cancel-fired";

/// Tauri event emitted when the picker shortcut fires(開 profile picker)。
pub const PORTAL_PICKER_EVENT: &str = "portal-picker-fired";

/// Prefix for VoiceInput slot shortcuts (Alt+0~9 → slot-0 … slot-9).
const SLOT_ID_PREFIX: &str = "slot-";

/// Prefix for Agent slot shortcuts (5G — Ctrl+Alt+0~9 → agent-slot-0 … agent-slot-9).
const AGENT_SLOT_ID_PREFIX: &str = "agent-slot-";

/// Tauri event emitted when Alt+N fires. Payload is the slot number as u8 (0–9).
pub const PROFILE_SLOT_EVENT: &str = "portal-profile-slot";

/// Tauri event emitted when Ctrl+Alt+N fires. Payload is the slot number as u8 (0–9).
pub const AGENT_SLOT_EVENT: &str = "portal-agent-slot";

const PREFERRED_TRIGGER: &str = "CTRL+ALT+space";

/// Run forever, dispatching portal Activated signals into Tauri events.
///
/// Spawn this in a tokio task at app start. If the portal call fails
/// (no GNOME, sandboxed env without xdg-portal, user denied), this
/// returns an error and the caller should log + fall back to the UI
/// "manual trigger" button — there's no global shortcut, but Mori still
/// works.
pub async fn run(app: AppHandle) -> Result<()> {
    // GNOME's portal-gnome looks up a `.desktop` file for our app id
    // before letting us register. Non-flatpak dev binaries don't
    // get one for free — write a minimal one into the user-level
    // applications dir pointing at our current binary.
    if let Err(e) = ensure_desktop_file() {
        tracing::warn!(?e, "could not write user desktop entry; portal may reject registration");
    }

    // Tell xdg-desktop-portal who we are. For non-flatpak apps this is
    // necessary so the portal can scope permissions to our app id;
    // flatpak'd apps inherit it from the sandbox manifest and ashpd
    // skips this call automatically.
    let app_id: AppID = APP_ID
        .parse()
        .context("APP_ID is not a valid reverse-DNS identifier")?;
    tracing::info!(app_id = APP_ID, "registering host app with xdg-desktop-portal Registry...");
    register_host_app(app_id)
        .await
        .context("register host app id with xdg-desktop-portal Registry")?;
    tracing::info!("✓ host app registered — Mori 應被 portal 視為 trusted");

    let proxy = GlobalShortcuts::new()
        .await
        .context("connect to xdg-desktop-portal GlobalShortcuts (is xdg-desktop-portal-gnome installed?)")?;

    let session = proxy
        .create_session()
        .await
        .context("create GlobalShortcuts session")?;

    // 5G: 三組熱鍵 + 一個錄音 toggle，共 21 個全域快捷鍵
    //
    // Alt+0~9        → VoiceInput profile（slot 0 = USER-00 預設極簡聽寫，1~9 切 voice profile）
    // Ctrl+Alt+0~9   → Agent profile（slot 0 = default Mori，1~9 切 agent profile）
    // Ctrl+Alt+Space → 錄音 toggle（兩個 mode 共用）
    let slot_ids: Vec<String> = (0u8..=9).map(|n| format!("{SLOT_ID_PREFIX}{n}")).collect();
    let slot_descriptions: Vec<String> = std::iter::once(
        "Mori — VoiceInput 純語音輸入（USER-00 極簡聽寫）".to_string(),
    )
    .chain((1u8..=9).map(|n| format!("Mori — 切換 VoiceInput Profile {n}")))
    .collect();
    let slot_triggers: Vec<String> = (0u8..=9).map(|n| format!("ALT+{n}")).collect();

    let agent_slot_ids: Vec<String> = (0u8..=9)
        .map(|n| format!("{AGENT_SLOT_ID_PREFIX}{n}"))
        .collect();
    let agent_slot_descriptions: Vec<String> = std::iter::once(
        "Mori — Agent 自由判斷模式（default Mori）".to_string(),
    )
    .chain((1u8..=9).map(|n| format!("Mori — 切換 Agent Profile {n}")))
    .collect();
    let agent_slot_triggers: Vec<String> = (0u8..=9).map(|n| format!("CTRL+ALT+{n}")).collect();

    let mut shortcuts = vec![
        NewShortcut::new(TOGGLE_SHORTCUT_ID, "Mori — 開始 / 停止錄音")
            .preferred_trigger(Some(PREFERRED_TRIGGER)),
        NewShortcut::new(CANCEL_SHORTCUT_ID, "Mori — 錄音中按下取消（丟棄音檔，不送出）")
            .preferred_trigger(Some("CTRL+ALT+Escape")),
        NewShortcut::new(PICKER_SHORTCUT_ID, "Mori — 開 Profile picker 視窗（方向鍵選）")
            .preferred_trigger(Some("CTRL+ALT+p")),
    ];
    for i in 0..=9usize {
        shortcuts.push(
            NewShortcut::new(&slot_ids[i], &slot_descriptions[i])
                .preferred_trigger(Some(slot_triggers[i].as_str())),
        );
        shortcuts.push(
            NewShortcut::new(&agent_slot_ids[i], &agent_slot_descriptions[i])
                .preferred_trigger(Some(agent_slot_triggers[i].as_str())),
        );
    }

    // First-ever call pops the GNOME permission dialog. After grant, the
    // binding persists per-user — subsequent runs are silent.
    let bind = proxy
        .bind_shortcuts(&session, &shortcuts, None)
        .await
        .context("bind shortcuts (first run shows permission dialog — user must grant)")?;
    let bound = bind
        .response()
        .context("portal returned error / user denied")?;

    if bound.shortcuts().is_empty() {
        anyhow::bail!("portal accepted bind but returned no shortcuts");
    }
    for sc in bound.shortcuts() {
        tracing::info!(
            id = sc.id(),
            description = sc.description(),
            trigger = sc.trigger_description(),
            "portal global shortcut bound",
        );
    }

    let mut activated = proxy
        .receive_activated()
        .await
        .context("subscribe to Activated signal")?;

    while let Some(event) = activated.next().await {
        let id = event.shortcut_id();
        tracing::debug!(
            id,
            ts_ms = event.timestamp().as_millis() as u64,
            "portal hotkey activated",
        );

        if id == TOGGLE_SHORTCUT_ID {
            if let Err(e) = app.emit(PORTAL_HOTKEY_EVENT, ()) {
                tracing::warn!(?e, "failed to emit portal-hotkey-fired event");
            }
        } else if id == CANCEL_SHORTCUT_ID {
            if let Err(e) = app.emit(PORTAL_CANCEL_EVENT, ()) {
                tracing::warn!(?e, "failed to emit portal-cancel-fired event");
            }
        } else if id == PICKER_SHORTCUT_ID {
            if let Err(e) = app.emit(PORTAL_PICKER_EVENT, ()) {
                tracing::warn!(?e, "failed to emit portal-picker-fired event");
            }
        } else if let Some(slot_str) = id.strip_prefix(AGENT_SLOT_ID_PREFIX) {
            // 注意：要先 check AGENT_SLOT_ID_PREFIX，因為它以 "slot-" 結尾，
            // 直接 strip_prefix("slot-") 會誤命中。
            if let Ok(n) = slot_str.parse::<u8>() {
                if let Err(e) = app.emit(AGENT_SLOT_EVENT, n) {
                    tracing::warn!(?e, slot = n, "failed to emit agent-slot event");
                }
            }
        } else if let Some(slot_str) = id.strip_prefix(SLOT_ID_PREFIX) {
            if let Ok(n) = slot_str.parse::<u8>() {
                if let Err(e) = app.emit(PROFILE_SLOT_EVENT, n) {
                    tracing::warn!(?e, slot = n, "failed to emit profile-slot event");
                }
            }
        }
    }

    Ok(())
}

/// Make sure `~/.local/share/applications/<APP_ID>.desktop` exists and
/// points at the current binary. Idempotent — overwrites every run so a
/// moved / rebuilt binary stays addressable. Without this file
/// xdg-desktop-portal-gnome rejects host-app registration with
/// `Could not register app ID: App info not found`.
fn ensure_desktop_file() -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let dir = std::path::PathBuf::from(home).join(".local/share/applications");
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;

    let exe = std::env::current_exe().context("get current_exe")?;
    let exe_str = exe.to_str().context("current_exe path is not valid UTF-8")?;

    let path = dir.join(format!("{APP_ID}.desktop"));
    // 5O fix: StartupWMClass 必須跟 Tauri 視窗實際 WM_CLASS 一致,GNOME taskbar
    // 才會把多個視窗(main / floating / chat_bubble / picker)歸到同一個 app
    // entry,不會每次重啟堆一條。Tauri 2 在 GTK/X11 下 WM_CLASS = "mori-tauri" /
    // "Mori-tauri"(Cargo.toml package name)。
    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Mori\n\
         Comment=森林精靈 Mori 的桌面身體\n\
         Exec={exe_str}\n\
         Icon=mori\n\
         Categories=Utility;AudioVideo;\n\
         StartupWMClass=Mori-tauri\n\
         X-GNOME-UsesNotifications=true\n\
         NoDisplay=false\n",
    );

    let needs_write = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != content,
        Err(_) => true,
    };

    if needs_write {
        std::fs::write(&path, &content)
            .with_context(|| format!("write {}", path.display()))?;
        tracing::info!(path = %path.display(), exec = exe_str, "wrote desktop entry for portal");

        // Best-effort cache refresh — newer GNOME picks up the file
        // immediately, but older portals cache and need a kick.
        let _ = std::process::Command::new("update-desktop-database")
            .arg(&dir)
            .status();
    }

    Ok(())
}
