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

use crate::hotkey_config::{
    HotkeyConfig, AGENT_SLOT_EVENT, AGENT_SLOT_ID_PREFIX, CANCEL_SHORTCUT_ID, MORI_SLEEP_EVENT, PICKER_SHORTCUT_ID, SLEEP_SHORTCUT_ID,
    PORTAL_CANCEL_EVENT, PORTAL_HOTKEY_PRESSED, PORTAL_HOTKEY_RELEASED, PORTAL_PICKER_EVENT,
    PROFILE_SLOT_EVENT, SLOT_ID_PREFIX, TOGGLE_SHORTCUT_ID,
};

/// App ID we register with the portal Registry. Must match
/// `tauri.conf.json` `identifier` so per-app portal permissions are
/// stored under one key. GNOME requires a registered app id before any
/// portal call that needs to attribute permissions to a specific app
/// (without it, GlobalShortcuts.CreateSession returns
/// `org.freedesktop.portal.Error.NotAllowed: An app id is required`).
const APP_ID: &str = "ai.yazelin.mori";

/// Run forever, dispatching portal Activated signals into Tauri events.
///
/// Spawn this in a tokio task at app start. If the portal call fails
/// (no GNOME, sandboxed env without xdg-portal, user denied), this
/// returns an error and the caller should log + fall back to the UI
/// "manual trigger" button — there's no global shortcut, but Mori still
/// works.
///
/// `config` 提供 portal `preferred_trigger` 建議值。注意 portal 規範:使
/// 用者第一次同意後,實際綁定由 compositor 紀錄,之後 Mori config 改了
/// 不會自動覆寫,要使用者去 GNOME Settings → Keyboard 改、或刪掉
/// `~/.local/share/xdg-desktop-portal/permissions` 讓 Mori 重新註冊。
pub async fn run(app: AppHandle, config: HotkeyConfig) -> Result<()> {
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

    // 5G: 三組熱鍵 + 一個錄音 toggle、cancel、picker,共 23 個全域快捷鍵
    //
    // 預設:
    // - Ctrl+Alt+Space → 錄音 toggle(兩個 mode 共用)
    // - Ctrl+Alt+Esc   → cancel(錄音中丟掉音檔)
    // - Ctrl+Alt+P     → picker overlay
    // - Alt+0~9        → VoiceInput profile(slot 0 = USER-00 預設極簡聽寫,1~9 切 voice profile)
    // - Ctrl+Alt+0~9   → Agent profile(slot 0 = default Mori,1~9 切 agent profile)
    //
    // 全部 trigger 從 [`HotkeyConfig`] 算出來,使用者可在 `~/.mori/config.json`
    // `hotkeys` 子樹覆寫(見 [`crate::hotkey_config`])。
    let bindings = config
        .resolve()
        .context("resolve hotkey config (check ~/.mori/config.json hotkeys section)")?;

    // 預先算好每筆的 portal trigger + id,避免在 NewShortcut builder lifetime
    // 中 borrow 暫存 String。
    let portal_specs: Vec<(String, String, String)> = bindings
        .iter()
        .map(|b| (b.action.portal_id(), b.action.description(), b.to_portal_trigger()))
        .collect();

    let shortcuts: Vec<NewShortcut> = portal_specs
        .iter()
        .map(|(id, desc, trigger)| {
            NewShortcut::new(id.as_str(), desc.as_str()).preferred_trigger(Some(trigger.as_str()))
        })
        .collect();

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
    // hold 模式才需要 Deactivated,但這裡無論 mode 都訂閱:訂閱本身 cost
    // 接近零,且 mode 切換在 main.rs 那層處理(emit 出去後讓 listener 決定
    // 怎麼解讀),這樣 portal 層不必跟 ToggleMode coupling。
    let mut deactivated = proxy
        .receive_deactivated()
        .await
        .context("subscribe to Deactivated signal")?;

    loop {
        tokio::select! {
            Some(event) = activated.next() => {
                let id = event.shortcut_id();
                tracing::debug!(
                    id,
                    ts_ms = event.timestamp().as_millis() as u64,
                    "portal hotkey activated",
                );

                if id == TOGGLE_SHORTCUT_ID {
                    if let Err(e) = app.emit(PORTAL_HOTKEY_PRESSED, ()) {
                        tracing::warn!(?e, "failed to emit portal-hotkey-pressed event");
                    }
                } else if id == CANCEL_SHORTCUT_ID {
                    if let Err(e) = app.emit(PORTAL_CANCEL_EVENT, ()) {
                        tracing::warn!(?e, "failed to emit portal-cancel-fired event");
                    }
                } else if id == PICKER_SHORTCUT_ID {
                    if let Err(e) = app.emit(PORTAL_PICKER_EVENT, ()) {
                        tracing::warn!(?e, "failed to emit portal-picker-fired event");
                    }
                } else if id == SLEEP_SHORTCUT_ID {
                    if let Err(e) = app.emit(MORI_SLEEP_EVENT, ()) {
                        tracing::warn!(?e, "failed to emit mori-sleep event");
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
            Some(event) = deactivated.next() => {
                // 只有 toggle chord 在 hold 模式下會用 Release。其他 action
                // (cancel / picker / slot) 是離散事件,沒 release 語意。
                let id = event.shortcut_id();
                tracing::debug!(
                    id,
                    ts_ms = event.timestamp().as_millis() as u64,
                    "portal hotkey deactivated",
                );
                if id == TOGGLE_SHORTCUT_ID {
                    if let Err(e) = app.emit(PORTAL_HOTKEY_RELEASED, ()) {
                        tracing::warn!(?e, "failed to emit portal-hotkey-released event");
                    }
                }
            }
            else => break,
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
    //
    // Icon 用絕對路徑指向 binary 旁邊的 256x256 icon — `Icon=mori` 會走 freedesktop
    // 系統 icon 查找(/usr/share/icons / ~/.local/share/icons),沒裝就空白;
    // 絕對路徑保證 GNOME shell / dock 一定找得到。
    let icon_path = exe
        .parent()
        .and_then(|p| {
            // mori-tauri binary 在 target/debug/,icons 在 crates/mori-tauri/icons/
            // 從 binary 推回 crates/mori-tauri/icons/icon.png
            let candidates = [
                p.join("../../crates/mori-tauri/icons/icon.png"),
                p.join("icons/icon.png"),
            ];
            candidates.into_iter().find(|p| p.exists())
        })
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mori".to_string());
    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Mori\n\
         Comment=森林精靈 Mori 的桌面身體\n\
         Exec={exe_str}\n\
         Icon={icon_path}\n\
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
