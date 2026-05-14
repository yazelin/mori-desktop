//! 直接跟 OS 註冊的全域熱鍵 path — via `tauri-plugin-global-shortcut`。
//!
//! 用在所有「不需要 portal 中介」的平台:
//!
//! | 平台 | 底層 |
//! |---|---|
//! | Linux X11(含 GDK_BACKEND=x11)| XGrabKey |
//! | Windows | `RegisterHotKey` Win32 API |
//! | macOS | Carbon `RegisterEventHotKey` |
//! | Linux Wayland | **不能用** — compositor 擋掉 XGrabKey,改走 [`portal_hotkey`] |
//!
//! XWayland(Wayland session 跑 X11 程式)仍要走 portal — `XDG_SESSION_TYPE` 是
//! `wayland`,Mori 啟動時偵測到會自動切 portal path。
//!
//! 跟 portal 路徑共用同一份 [`HotkeyConfig`],callback 也 emit 同樣的 Tauri
//! event(`PORTAL_HOTKEY_PRESSED` / `PORTAL_HOTKEY_RELEASED` 等),所以
//! main.rs 下游 listener 不用知道現在跑哪條 path。

use anyhow::{Context as _, Result};
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::hotkey_config::{
    HotkeyAction, HotkeyConfig, AGENT_SLOT_EVENT, MORI_SLEEP_EVENT, PORTAL_CANCEL_EVENT,
    PORTAL_HOTKEY_PRESSED, PORTAL_HOTKEY_RELEASED, PORTAL_PICKER_EVENT, PROFILE_SLOT_EVENT,
};

/// Linux session 是否為 X11(`XDG_SESSION_TYPE=x11`)。
///
/// XWayland(`XDG_SESSION_TYPE=wayland` 但 GDK_BACKEND=x11)仍要走 portal,
/// 因為 Wayland compositor 不會把 XGrabKey 的全域 key 送給 XWayland client。
///
/// 非 Linux 平台一律回 `false`(它們沒有 XDG_SESSION_TYPE 概念,但會走 direct
/// path — main.rs 用 `cfg(not(target_os = "linux"))` 直接 call [`register`])。
// `is_x11_session` 只在 Linux build 有意義 — main.rs 的 tauri command
// wrapper(#[cfg(not(target_os = "linux"))] 走 false)不需要這層,所以拿掉
// 非 Linux 版本避免 dead-code warning。
#[cfg(target_os = "linux")]
pub fn is_x11_session() -> bool {
    matches!(
        std::env::var("XDG_SESSION_TYPE").as_deref(),
        Ok("x11") | Ok("X11"),
    )
}

/// 註冊所有 23 個全域快捷鍵。每筆 grab 失敗單獨 log warn 不中斷其他 binding —
/// 比方 Ctrl+Alt+Esc 被 GNOME cycle-panels 佔住,這條 grab 會 fail,但其他鍵
/// 仍可註冊成功。回 Err 只在「整批 config 解析失敗」這種 hard error。
pub fn register(app: &AppHandle, config: &HotkeyConfig) -> Result<()> {
    let bindings = config
        .resolve()
        .context("resolve hotkey config (check ~/.mori/config.json hotkeys section)")?;

    let mut registered = 0usize;
    let mut failed = 0usize;
    for binding in bindings {
        let shortcut = match binding.to_shortcut() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    ?e,
                    action = ?binding.action,
                    key = %binding.key,
                    "skipping hotkey (parse failed)",
                );
                failed += 1;
                continue;
            }
        };

        let action = binding.action.clone();
        let app_clone = app.clone();
        let key_for_log = binding.key.clone();
        let result = app.global_shortcut().on_shortcut(shortcut, move |_app, _sc, event| {
            // Toggle action 在 Press / Release 都 dispatch — main.rs 那層依
            // 當下 `state.toggle_mode` 決定怎麼解讀(toggle = Press 跑 toggle、
            // Release 忽略;hold = Press 開錄、Release 停錄)。其他 action 是
            // 離散事件,只在 Pressed 觸發。
            let is_pressed =
                event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed;
            match &action {
                HotkeyAction::Toggle => dispatch_toggle(&app_clone, is_pressed),
                _ => {
                    if is_pressed {
                        dispatch(&app_clone, &action);
                    }
                }
            }
        });

        match result {
            Ok(_) => {
                registered += 1;
                tracing::debug!(action = ?binding.action, key = %key_for_log, "x11 hotkey grabbed");
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(
                    ?e,
                    action = ?binding.action,
                    key = %key_for_log,
                    "x11 hotkey grab failed — another app may have grabbed this key (e.g. GNOME cycle-panels for Ctrl+Alt+Esc)",
                );
            }
        }
    }

    tracing::info!(registered, failed, "x11 global shortcuts registered");
    Ok(())
}

/// 把 (離散) action 轉成下游 listener 已 subscribe 的 Tauri event。
/// 跟 [`portal_hotkey::run`] 內 dispatch 一致,只是觸發源頭不同。
/// 注意:Toggle 不走這條,走 [`dispatch_toggle`]。
fn dispatch(app: &AppHandle, action: &HotkeyAction) {
    tracing::debug!(?action, "x11 hotkey fired");
    let emit_result = match action {
        HotkeyAction::Toggle => unreachable!("toggle action handled by dispatch_toggle"),
        HotkeyAction::Cancel => app.emit(PORTAL_CANCEL_EVENT, ()),
        HotkeyAction::Picker => app.emit(PORTAL_PICKER_EVENT, ()),
        HotkeyAction::Sleep => app.emit(MORI_SLEEP_EVENT, ()),
        HotkeyAction::VoiceSlot(n) => app.emit(PROFILE_SLOT_EVENT, *n),
        HotkeyAction::AgentSlot(n) => app.emit(AGENT_SLOT_EVENT, *n),
    };
    if let Err(e) = emit_result {
        tracing::warn!(?e, ?action, "x11 hotkey event emit failed");
    }
}

/// Toggle chord 專用 dispatch。Press → `PORTAL_HOTKEY_PRESSED`;Release →
/// `PORTAL_HOTKEY_RELEASED`。main.rs 那層 listener 依當下 `state.toggle_mode`
/// 決定怎麼處理(toggle = Press 走 handle_hotkey_toggle、Release no-op;
/// hold = Press 開錄、Release 停錄)。
fn dispatch_toggle(app: &AppHandle, is_pressed: bool) {
    if is_pressed {
        tracing::debug!("x11 toggle pressed");
        if let Err(e) = app.emit(PORTAL_HOTKEY_PRESSED, ()) {
            tracing::warn!(?e, "x11 toggle PORTAL_HOTKEY_PRESSED emit failed");
        }
    } else {
        tracing::debug!("x11 toggle released");
        if let Err(e) = app.emit(PORTAL_HOTKEY_RELEASED, ()) {
            tracing::warn!(?e, "x11 toggle PORTAL_HOTKEY_RELEASED emit failed");
        }
    }
}
