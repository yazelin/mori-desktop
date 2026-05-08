use std::process::Command;

fn main() {
    tauri_build::build();

    // 把 git SHA、dirty flag、build 時間編進 binary,讓 UI 能秀出來,
    // user 一眼就能分辨「我現在跑的是哪個 build」。
    let sha = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=MORI_GIT_SHA={}", sha);

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    println!(
        "cargo:rustc-env=MORI_GIT_DIRTY={}",
        if dirty { "1" } else { "" }
    );

    // UTC,不靠 chrono(避免拉 build-dep)。`date` 在 Linux/macOS 都有。
    // Windows build 走這條會拿到 "unknown" — Mori 是 Linux-first 不擋。
    let time = Command::new("date")
        .args(["-u", "+%Y-%m-%d %H:%MZ"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=MORI_BUILD_TIME={}", time);

    // 確保 git checkout / commit 後重 build 也會更新 SHA。HEAD ref
    // 變動 + index 變動都讓 build.rs rerun。
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    if let Ok(head) = std::fs::read_to_string("../../.git/HEAD") {
        if let Some(rest) = head.strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=../../.git/{}", rest.trim());
        }
    }
}
