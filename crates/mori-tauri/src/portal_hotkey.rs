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

/// Tauri event emitted when the toggle shortcut fires.
pub const PORTAL_HOTKEY_EVENT: &str = "portal-hotkey-fired";

/// Prefix for the 9 profile-slot shortcut ids (e.g. "slot-1" … "slot-9").
const SLOT_ID_PREFIX: &str = "slot-";

/// Tauri event emitted when Alt+N fires. Payload is the slot number as u8 (1–9).
pub const PROFILE_SLOT_EVENT: &str = "portal-profile-slot";

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

    // 主錄音熱鍵 + Alt+0（切回對話模式）+ Alt+1~9（切 VoiceInput profile）
    // 先把 String 存好，確保生命週期夠長
    // Alt+0 = slot 0 = 切回 Active（對話）模式
    // Alt+1~9 = slot 1~9 = 切到 VoiceInput + 對應 profile
    let slot_ids: Vec<String> = (0u8..=9).map(|n| format!("{SLOT_ID_PREFIX}{n}")).collect();
    let slot_descriptions: Vec<String> = std::iter::once("Mori — 切回對話模式".to_string())
        .chain((1u8..=9).map(|n| format!("Mori — 切換語音輸入 Profile {n}")))
        .collect();
    let slot_triggers: Vec<String> = (0u8..=9).map(|n| format!("ALT+{n}")).collect();

    let mut shortcuts = vec![
        NewShortcut::new(TOGGLE_SHORTCUT_ID, "Mori — 開始 / 停止錄音")
            .preferred_trigger(Some(PREFERRED_TRIGGER)),
    ];
    for i in 0..=9usize {
        shortcuts.push(
            NewShortcut::new(&slot_ids[i], &slot_descriptions[i])
                .preferred_trigger(Some(slot_triggers[i].as_str())),
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
    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Mori\n\
         Comment=森林精靈 Mori 的桌面身體\n\
         Exec={exe_str}\n\
         Icon=mori\n\
         Categories=Utility;AudioVideo;\n\
         StartupWMClass=Mori\n\
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
