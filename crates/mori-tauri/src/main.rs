// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mori_core::{PHASE, VERSION};

/// 給前端用的:回傳 mori-core 版本號(供 sanity check)
#[tauri::command]
fn mori_version() -> String {
    VERSION.to_string()
}

/// 給前端用的:回傳目前 phase 名稱
#[tauri::command]
fn mori_phase() -> String {
    PHASE.to_string()
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mori_tauri=debug,mori_core=debug".into()),
        )
        .init();

    tracing::info!("Mori starting — phase {}", PHASE);

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![mori_version, mori_phase])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
