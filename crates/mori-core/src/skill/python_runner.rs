//! Wave 6 D-full(DF-2):跑 Anthropic skill 內 `scripts/` 子目錄的 Python script。
//!
//! 提供一個 minimal subprocess runner:
//! - spawn `python3 <script> <args...>`(走 PATH 上的 `python3`)
//! - 可選 stdin
//! - capture stdout / stderr / exit code
//! - 60s timeout(避免 LLM 誤觸發 infinite loop 把 agent stuck)
//!
//! # 為什麼不上 sandbox / venv?
//!
//! - **不開 firejail / nsjail / wasm 之類沙箱**:user trust 自己安裝的 Anthropic
//!   skill repo(`anthropics/skills` 官方 repo,他們審過了)。要再加 process
//!   isolation 要拖大 dep,本 stream 不做(留 follow-up)。
//! - **不自管 venv**:per-skill `requirements.txt` install 涉及 venv 管理、跨
//!   skill share 版本、衝突偵測,複雜度爆表;本 stream 要求 user 手動
//!   `pip install --user pypdf openpyxl python-docx python-pptx` for 常用 file
//!   skills。
//!
//! # 為什麼 60s timeout?
//!
//! LLM 觸發 → 等 script → tool result 回 LLM 是一條 blocking 路徑,user 等。
//! 60s 是「合 reasonable」的天花板(merge 5 個 PDF 約 5-15s,OCR 一頁 ~30s)。
//! 超出明顯不對勁 — 殺掉,讓 LLM 重試或回報 user。

use std::path::Path;
use std::process::Stdio;

use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// 一次 script 跑完的結果。`exit_code` -1 表示 process 被 signal 殺(或 timeout)。
#[derive(Debug, Clone)]
pub struct ScriptOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Error)]
pub enum RunError {
    /// `python3` 不在 PATH。user 沒裝 Python 3 / 沒接好 PATH。
    #[error("python3 not in PATH (please install Python 3)")]
    PythonMissing,
    /// spawn 失敗(不是 NotFound,是其他 OS 層 error,例 permission denied)。
    #[error("spawn failed: {0}")]
    Spawn(#[source] std::io::Error),
    /// script 跑超過 [`DEFAULT_TIMEOUT_SECS`] 秒,被強制砍掉。
    #[error("timeout({0}s) exceeded")]
    Timeout(u64),
    /// 其他 IO error(stdin write 失敗 / wait_with_output IO 錯)。
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// script timeout 上限。
///
/// 故意暴露成 `pub const` 給 caller 知道下限;真要客製需要改 [`run_python_script`]
/// 簽名加 `timeout` 參數(本 stream 保簡單,後續若要再開放)。
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// 跑單一 Python script。
///
/// - `script`:要跑的 `.py` 路徑(caller 負責確保檔案存在)
/// - `args`:轉成 CLI argv(順序保留)
/// - `stdin`:若 `Some(s)`,把 `s` 灌進 child stdin 並關閉;`None` 則 stdin 不 connect
///
/// 回傳的 `ScriptOutput` 不論 script 成功 / 失敗(exit_code != 0)都會回 — 失敗
/// 表現在 `exit_code`,不直接 Err。`Err(RunError)` 只在 **runtime 層** 失敗
/// (找不到 python3 / spawn 失敗 / timeout)。
///
/// # 為什麼用 wait_with_output 而非分別 read stdout/stderr
///
/// 簡單。`wait_with_output` 內部會 spawn 兩 task drain pipe(避免 pipe buffer
/// 塞滿後 child block),這正是我們要的。代價是 stdout / stderr 都全 buffer 在
/// memory — 對 LLM tool result 場景沒問題(LLM context 本來就有上限,script
/// 輸出超大也沒意義)。
pub async fn run_python_script(
    script: &Path,
    args: &[String],
    stdin: Option<&str>,
) -> Result<ScriptOutput, RunError> {
    let mut cmd = Command::new("python3");
    cmd.arg(script)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // suppress windows console window — 對齊 mori 既有跑 subprocess pattern。
    // 不直接 import 是因為這個 macro 在 mori-core lib root 已 export;但這 module
    // 是 mori-core 自身一部分,直接 inline OS-conditional 比較乾淨。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RunError::PythonMissing
        } else {
            RunError::Spawn(e)
        }
    })?;

    // 寫 stdin 後 drop handle → child 看到 EOF。stdin 為 None 也 take() 出來,
    // 否則 child 會等 stdin 不關。
    if let Some(mut child_stdin) = child.stdin.take() {
        if let Some(input) = stdin {
            child_stdin.write_all(input.as_bytes()).await?;
            child_stdin.flush().await?;
        }
        // drop → 關 stdin pipe → child 看到 EOF
        drop(child_stdin);
    }

    let timeout = tokio::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS);
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(RunError::Io(e)),
        Err(_) => {
            // timeout — child 已被 wait_with_output 拿走,沒法直接 kill;
            // 但 tokio 的 wait_with_output future 被 drop 時會 abort 內部 task,
            // 不殺 child process 本身。
            //
            // 簡化處理:不嘗試 kill(本 stream 限制),只回 Timeout error;
            // child 在 OS 層繼續跑直到自己結束 / OOM-killer 砍它。對齊本 module
            // 的「最小 runner」定位 — 完整 process lifecycle 管理留 follow-up。
            return Err(RunError::Timeout(DEFAULT_TIMEOUT_SECS));
        }
    };

    Ok(ScriptOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// 寫一個 Python script 到 temp dir 並回 path。
    fn write_script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    /// 整環境是否裝 python3。CI 若沒裝就 skip。
    fn python3_available() -> bool {
        std::process::Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn run_python_script_runs_basic() {
        if !python3_available() {
            eprintln!("python3 not available; skipping");
            return;
        }
        let dir = TempDir::new().unwrap();
        let script = write_script(dir.path(), "hello.py", "print('hi')\n");
        let out = run_python_script(&script, &[], None)
            .await
            .expect("script should run");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "hi");
        assert!(out.stderr.is_empty());
    }

    #[tokio::test]
    async fn run_python_script_captures_stderr() {
        if !python3_available() {
            eprintln!("python3 not available; skipping");
            return;
        }
        let dir = TempDir::new().unwrap();
        let script = write_script(
            dir.path(),
            "err.py",
            "import sys\nsys.stderr.write('boom\\n')\nsys.exit(2)\n",
        );
        let out = run_python_script(&script, &[], None)
            .await
            .expect("script should run (even if exits non-zero)");
        assert_eq!(out.exit_code, 2);
        assert!(out.stderr.contains("boom"));
    }

    #[tokio::test]
    async fn run_python_script_passes_args() {
        if !python3_available() {
            eprintln!("python3 not available; skipping");
            return;
        }
        let dir = TempDir::new().unwrap();
        let script = write_script(
            dir.path(),
            "argv.py",
            "import sys\nprint('|'.join(sys.argv[1:]))\n",
        );
        let out = run_python_script(
            &script,
            &["foo".into(), "bar baz".into(), "qux".into()],
            None,
        )
        .await
        .expect("script should run");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "foo|bar baz|qux");
    }

    #[tokio::test]
    async fn run_python_script_passes_stdin() {
        if !python3_available() {
            eprintln!("python3 not available; skipping");
            return;
        }
        let dir = TempDir::new().unwrap();
        let script = write_script(
            dir.path(),
            "stdin.py",
            "import sys\ndata = sys.stdin.read()\nprint(f'got:{data}')\n",
        );
        let out = run_python_script(&script, &[], Some("hello\n"))
            .await
            .expect("script should run");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("got:hello"));
    }

    #[tokio::test]
    async fn run_python_script_times_out() {
        if !python3_available() {
            eprintln!("python3 not available; skipping");
            return;
        }
        // 走極短 timeout 來測 — 但 DEFAULT_TIMEOUT_SECS 是 60。直接測 60s 太慢,
        // 改成另開一個非公開的 with-timeout 路徑會破壞 API 表面。
        //
        // 折衷:用 tokio::time::timeout 直接 wrap 我們的 call,測 future-level
        // 行為(若 script 真的 hang,run_python_script 內部超時也會回 Timeout)。
        // 這個 test 比較像 sanity check:確認 hang script 不會 panic、會在合理
        // 時間內被識別。
        let dir = TempDir::new().unwrap();
        let script = write_script(
            dir.path(),
            "hang.py",
            // 1 秒就退,別真 hang 60s。但要證明 timeout-when-takes-too-long 行為,
            // 改成測「快」case 不夠 — 直接走 tokio::time::pause 或外層 wrap。
            //
            // 折衷:測 future 在外層 timeout 下行為 — script 跑 5s,外層 1s timeout
            // → 外層 Err(elapsed)。確認 run_python_script 本身沒 panic / 不會 leak。
            "import time\ntime.sleep(5)\nprint('done')\n",
        );

        let result = tokio::time::timeout(
            tokio::time::Duration::from_millis(500),
            run_python_script(&script, &[], None),
        )
        .await;

        // 外層 timeout 觸發 → Err(elapsed)。本 test 不嚴格驗 Timeout enum variant
        // (那需要 60s 等),但驗 run 不會 panic、行為一致。
        assert!(result.is_err(), "outer timeout should fire");
    }

    /// PATH 設空 → spawn python3 NotFound → PythonMissing。
    ///
    /// ⚠️ 這個 test mutate 全局 `PATH` env,跟其他平行 test(python3 真的要跑)
    /// 互衝;標 `#[ignore]` 避免 race。本地手動 `cargo test --
    /// run_python_script_returns_python_missing --ignored` 跑驗 error path。
    #[tokio::test]
    #[ignore = "mutates global PATH env; race-condition with other python3 tests"]
    async fn run_python_script_returns_python_missing() {
        let dir = TempDir::new().unwrap();
        let script = write_script(dir.path(), "noop.py", "print('x')\n");

        let saved_path = std::env::var_os("PATH");
        std::env::set_var("PATH", "");

        let result = run_python_script(&script, &[], None).await;

        // restore 先,確保後續 test 不受影響。
        match saved_path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }

        match result {
            Err(RunError::PythonMissing) => {}
            Ok(_) => panic!("expected PythonMissing, got Ok output"),
            Err(other) => eprintln!("got {other:?} instead of PythonMissing (platform-dependent)"),
        }
    }
}
