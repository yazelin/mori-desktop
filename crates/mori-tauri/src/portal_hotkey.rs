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
/// echoes this back so we can tell which shortcut fired (we'll have
/// more than one in later phases, e.g. an "ask about selection"
/// modifier variant).
pub const TOGGLE_SHORTCUT_ID: &str = "toggle";

/// Tauri event emitted when the toggle shortcut fires. The main loop
/// listens for this and runs `handle_hotkey_toggle`.
pub const PORTAL_HOTKEY_EVENT: &str = "portal-hotkey-fired";

/// Hint to the portal — XDG "shortcuts" spec format. The user can
/// override via the GNOME settings dialog if they want a different
/// chord; we just say "this is what we'd prefer".
const PREFERRED_TRIGGER: &str = "CTRL+ALT+space";

/// Run forever, dispatching portal Activated signals into Tauri events.
///
/// Spawn this in a tokio task at app start. If the portal call fails
/// (no GNOME, sandboxed env without xdg-portal, user denied), this
/// returns an error and the caller should log + fall back to the UI
/// "manual trigger" button — there's no global shortcut, but Mori still
/// works.
pub async fn run(app: AppHandle) -> Result<()> {
    // Tell xdg-desktop-portal who we are. For non-flatpak apps this is
    // necessary so the portal can scope permissions to our app id;
    // flatpak'd apps inherit it from the sandbox manifest and ashpd
    // skips this call automatically.
    let app_id: AppID = APP_ID
        .parse()
        .context("APP_ID is not a valid reverse-DNS identifier")?;
    register_host_app(app_id)
        .await
        .context("register host app id with xdg-desktop-portal Registry")?;

    let proxy = GlobalShortcuts::new()
        .await
        .context("connect to xdg-desktop-portal GlobalShortcuts (is xdg-desktop-portal-gnome installed?)")?;

    let session = proxy
        .create_session()
        .await
        .context("create GlobalShortcuts session")?;

    let shortcuts = vec![NewShortcut::new(TOGGLE_SHORTCUT_ID, "Mori — 開始 / 停止錄音")
        .preferred_trigger(Some(PREFERRED_TRIGGER))];

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
        tracing::debug!(
            id = event.shortcut_id(),
            ts_ms = event.timestamp().as_millis() as u64,
            "portal hotkey activated",
        );
        if event.shortcut_id() == TOGGLE_SHORTCUT_ID {
            if let Err(e) = app.emit(PORTAL_HOTKEY_EVENT, ()) {
                tracing::warn!(?e, "failed to emit portal-hotkey-fired event");
            }
        }
    }

    Ok(())
}
