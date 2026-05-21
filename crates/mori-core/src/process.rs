//! 跨平台 subprocess helper — 集中處理 spawn 子程序時的平台 quirk。
//!
//! 目前只有一條:Windows 上的 `CREATE_NO_WINDOW` flag,避免 GUI parent
//! (`windows_subsystem = "windows"`)spawn console child(python.exe / claude.cmd
//! / whisper-server.exe 等)時 OS 給子程序分配新 console → user 看到黑色 cmd
//! 視窗閃出來/常駐。Linux / macOS 沒這問題,完全 no-op。
//!
//! 設計成 macro 而非 generic fn,因為 std 跟 tokio 的 `Command` 都實作了
//! `std::os::windows::process::CommandExt`(同一個 trait,`creation_flags` 簽名
//! 一致),macro 直接帶 type 進去寫 expansion 比兩個 fn 重複定義乾淨,也比
//! trait object 簡單。

/// 設 `CREATE_NO_WINDOW` flag 於給定的 `Command`,Windows-only,其他平台 no-op。
///
/// 用法:
/// ```ignore
/// let mut cmd = std::process::Command::new("python");
/// cmd.arg("script.py");
/// mori_core::suppress_console_on_windows!(cmd);
/// let child = cmd.spawn()?;
/// ```
///
/// 同樣對 `tokio::process::Command` 適用 — 兩家都認 `CommandExt` trait。
///
/// 為什麼 GUI parent 必加:Tauri release 用 `windows_subsystem = "windows"`,
/// 沒繼承的 console。Console subsystem child(任何 .exe / .cmd / .bat)spawn
/// 時若不顯式抑制,Windows 會 alloc 新 console window 給 child → 黑框跳出來。
/// stdout/stderr 我們已經 pipe / null 處理,console 本身對 mori 完全沒用。
#[macro_export]
macro_rules! suppress_console_on_windows {
    ($cmd:expr) => {
        #[cfg(target_os = "windows")]
        {
            #[allow(unused_imports)]
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            $cmd.creation_flags(CREATE_NO_WINDOW);
        }
    };
}
