//! e2e(本機真機,`#[ignore]`):驗 `EarTranscriptionProvider` 的整條 runtime 路徑——
//! mori-ear 沒在線就 lazy-spawn `mori-ear --serve` → 發現 descriptor → POST /inference
//! → 真實轉錄。不開 GUI。對齊 `integration_annuli.rs`:env-gated + `#[ignore]`。
//!
//! 需要:
//!   - `MORI_EAR_E2E_WAV=<真實語音 WAV 路徑>`(沒設 → skip)
//!   - 本機裝 `mori-ear`(`~/.cargo/bin` / `~/.mori/bin` / PATH)
//!   - 轉錄後端:本機 whisper-server(會被 `mori-whisper-serve --ensure` 冷啟)或 Groq key
//!
//! 跑:
//!   MORI_EAR_E2E_WAV=~/.mori/recordings/<ts>/audio-raw.wav \
//!     cargo test -p mori-core --test ear_e2e -- --ignored --nocapture

use mori_core::llm::ear_transcribe::EarTranscriptionProvider;
use mori_core::llm::transcribe::TranscriptionProvider;

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .expect("HOME / USERPROFILE"),
    )
}

/// 從 descriptor JSON 粗解 `pid`(避免把 serde_json 拉進 dev-deps)。
fn descriptor_pid(txt: &str) -> Option<u32> {
    let i = txt.find("\"pid\"")?;
    txt[i + 5..]
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

#[tokio::test]
#[ignore = "real-machine e2e: set MORI_EAR_E2E_WAV; needs mori-ear binary + backend"]
async fn lazy_spawn_ear_and_transcribe() {
    // env-gated:沒設 WAV 路徑就 skip(對齊 integration_annuli 的 ANNULI_E2E gate)
    let Ok(wav_path) = std::env::var("MORI_EAR_E2E_WAV") else {
        eprintln!("skip:設 MORI_EAR_E2E_WAV=<真實語音 WAV 路徑> 才跑此 e2e");
        return;
    };
    let wav = std::fs::read(&wav_path).unwrap_or_else(|e| panic!("讀 WAV {wav_path} 失敗:{e}"));
    eprintln!("--- 送入 {} bytes WAV", wav.len());

    let ear_desc = home().join(".mori/mori-ear-server.json");
    let _ = std::fs::remove_file(&ear_desc); // 清掉 → 確保走 lazy-spawn,而非用現成服務

    // backend=auto(desktop 語音實際走這條):沒本機 server → ear `--ensure` 冷啟、否則 Groq
    let provider =
        EarTranscriptionProvider::from_config().expect("build EarTranscriptionProvider");
    let text = provider
        .transcribe(wav)
        .await
        .expect("transcribe via lazy-spawned mori-ear");

    eprintln!("--- 轉錄結果:{text}");
    assert!(!text.trim().is_empty(), "應轉出非空文字");
    assert!(
        ear_desc.exists(),
        "lazy-spawn 後該有 mori-ear-server.json descriptor(代表 `mori-ear --serve` 被拉起)"
    );

    // best-effort 清理:收掉這次 lazy-spawn 的 `mori-ear --serve`(讀 descriptor pid),不留殭屍
    if let Ok(txt) = std::fs::read_to_string(&ear_desc) {
        if let Some(pid) = descriptor_pid(&txt) {
            let _ = std::process::Command::new("kill")
                .arg("-9")
                .arg(pid.to_string())
                .status();
        }
    }
    let _ = std::fs::remove_file(&ear_desc);
}
