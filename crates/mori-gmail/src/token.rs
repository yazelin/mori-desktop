//! Token storage — `~/.mori/gmail-token.json` 的 load / save / 過期判斷。
//!
//! Token 由 [`crate::oauth`] 模組產生(初次 consent 或 refresh 後),由
//! [`crate::client::GmailClient`] 在每次 API call 前判斷新鮮度並按需 refresh。
//!
//! ## 路徑慣例
//!
//! 預設 token 路徑由 [`default_token_path`] 決定:
//! - Unix:`$HOME/.mori/gmail-token.json`
//! - Windows:`$USERPROFILE\.mori\gmail-token.json`
//!
//! 若兩個環境變數都拿不到,回 `None`,caller 自己拿一個合理路徑(例如測試用
//! tempdir)。CLAUDE.md 註明 Windows 的 `HOME` 可能沒設要 fallback `USERPROFILE`,
//! 對齊既有 [`mori_cli`] 等 crate 的處理。
//!
//! ## 過期判斷
//!
//! [`GmailToken::is_expired`] 預留 **30 秒 buffer** — 若 `expires_at` 已經在「30
//! 秒內」到期,視為過期。避免 client 剛判斷 fresh、發 request 時 token 正好過期
//! 拿到 401 的競態。Google OAuth access_token 預設壽命 1 小時,30 秒 buffer 不影響
//! 整體有效時間。

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// `token.rs` 層錯誤類型。
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// 讀寫 token 檔的 IO 錯誤。
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// JSON 解析失敗(檔案壞了 / 不是預期格式)。
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Gmail OAuth token + metadata。
///
/// 對應 Google OAuth2 token endpoint 回傳的 JSON,額外加 `expires_at`(absolute
/// time,在 [`crate::oauth`] 由 `expires_in` 秒數 + `Utc::now()` 算出),方便
/// load 後直接判斷 fresh。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailToken {
    /// 短期 access token(預設 ~1 小時)。
    pub access_token: String,

    /// 長期 refresh token(只在初次 consent 時拿到,Google 不一定每次 refresh 都回傳)。
    pub refresh_token: String,

    /// `access_token` 過期的絕對時間。
    pub expires_at: DateTime<Utc>,

    /// Google 回的 granted scope(space-separated)。Gm-1 應該只有 `gmail.readonly`。
    pub scope: String,

    /// 一律 `"Bearer"`,Google 不會給其他類型。保留欄位讓 caller 顯式寫 Authorization
    /// header 時可讀。
    pub token_type: String,
}

/// 過期判斷的安全 buffer(秒)。
const EXPIRY_BUFFER_SECS: i64 = 30;

impl GmailToken {
    /// 若 `expires_at` 已經過,或距現在不到 [`EXPIRY_BUFFER_SECS`] 秒,回 `true`。
    pub fn is_expired(&self) -> bool {
        let threshold = Utc::now() + Duration::seconds(EXPIRY_BUFFER_SECS);
        self.expires_at <= threshold
    }

    /// 此 token 是否包含某個 OAuth scope。Google 在 token endpoint 回的 `scope`
    /// 欄位是 **space-separated 完整 URL**(eg
    /// `"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send"`),
    /// 我們直接 split-whitespace + 等值比對。
    ///
    /// Gm-2 在 `SendGmailSkill::execute` 進場前先 check `has_scope(GMAIL_SEND_SCOPE)`,
    /// false 就回 "需要重新 OAuth" 給 LLM,LLM 再轉達給 user。
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scope.split_whitespace().any(|s| s == scope)
    }

    /// 從 disk load。檔案不存在 → [`TokenError::Io`](kind = NotFound);
    /// 內容不合 JSON → [`TokenError::Json`]。
    pub fn load(path: &Path) -> Result<Self, TokenError> {
        let raw = std::fs::read_to_string(path)?;
        let token: Self = serde_json::from_str(&raw)?;
        Ok(token)
    }

    /// 寫入 disk(原子性:先寫到 `.tmp`,再 rename)。
    ///
    /// 若 parent 目錄不存在會自動建立(`create_dir_all`),token 檔放 `~/.mori/`,
    /// 不一定先存在。
    pub fn save(&self, path: &Path) -> Result<(), TokenError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;

        // 原子寫入:.tmp -> rename。避免半寫狀態被下次 load 拿到。
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// 預設 token 路徑 — `~/.mori/gmail-token.json`。
///
/// Linux / macOS 走 `$HOME`,Windows fallback `$USERPROFILE`。兩個都拿不到回
/// `None`(罕見:CI / 沙箱環境),caller 自己決定要 panic 還是用其他路徑。
pub fn default_token_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".mori").join("gmail-token.json"))
}

/// home 目錄解析。內部 helper,留給 [`crate::oauth`] 跟 [`crate::client`] 共用
/// (gmail-config.json 也走同一個目錄)。
pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_token(expires_at: DateTime<Utc>) -> GmailToken {
        GmailToken {
            access_token: "ya29.fake_access".into(),
            refresh_token: "1//fake_refresh".into(),
            expires_at,
            scope: "https://www.googleapis.com/auth/gmail.readonly".into(),
            token_type: "Bearer".into(),
        }
    }

    #[test]
    fn token_save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        // 故意指到不存在的 nested dir,確認 save 會 mkdir。
        let path = dir.path().join("nested").join("gmail-token.json");

        let original = sample_token(Utc::now() + Duration::hours(1));
        original.save(&path).expect("save");

        let loaded = GmailToken::load(&path).expect("load");
        assert_eq!(loaded.access_token, original.access_token);
        assert_eq!(loaded.refresh_token, original.refresh_token);
        assert_eq!(loaded.scope, original.scope);
        assert_eq!(loaded.token_type, original.token_type);
        // chrono round-trip 精度到秒以下,但 to/from JSON 走 RFC 3339 應 byte-equal。
        assert_eq!(loaded.expires_at, original.expires_at);
    }

    #[test]
    fn token_is_expired_returns_true_for_past_time() {
        let token = sample_token(Utc::now() - Duration::seconds(1));
        assert!(token.is_expired(), "past expiry must be reported expired");
    }

    #[test]
    fn token_is_expired_returns_false_for_future() {
        // 1 小時後,遠超 30s buffer。
        let token = sample_token(Utc::now() + Duration::hours(1));
        assert!(!token.is_expired(), "1h-future expiry must be fresh");
    }

    #[test]
    fn token_is_expired_respects_30s_buffer() {
        // 10 秒後到期 — 在 30s buffer 內,視為過期。
        let token = sample_token(Utc::now() + Duration::seconds(10));
        assert!(
            token.is_expired(),
            "within-30s expiry should be flagged as expired (buffer)"
        );
    }

    #[test]
    fn has_scope_detects_individual_scope_in_space_separated_list() {
        let mut t = sample_token(Utc::now() + Duration::hours(1));
        // 單一 scope
        t.scope = "https://www.googleapis.com/auth/gmail.readonly".into();
        assert!(t.has_scope("https://www.googleapis.com/auth/gmail.readonly"));
        assert!(!t.has_scope("https://www.googleapis.com/auth/gmail.send"));

        // 雙 scope(Gm-2 升級後)
        t.scope = "https://www.googleapis.com/auth/gmail.readonly \
                   https://www.googleapis.com/auth/gmail.send"
            .into();
        assert!(t.has_scope("https://www.googleapis.com/auth/gmail.readonly"));
        assert!(t.has_scope("https://www.googleapis.com/auth/gmail.send"));
        assert!(!t.has_scope("https://www.googleapis.com/auth/gmail.compose"));
    }

    #[test]
    fn default_token_path_uses_home() {
        // 暫時設 HOME 到 tempdir 並清掉 USERPROFILE,確認 default_token_path()
        // 回 `<HOME>/.mori/gmail-token.json`。
        //
        // ⚠️ env::set_var 在 Rust 1.74+ 是 unsafe 在多 thread 場景,這裡 test
        // single-thread 內 OK;Rust 早期版本只是普通 fn,呼叫 site 無需 unsafe。
        let dir = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_userprofile = std::env::var_os("USERPROFILE");

        std::env::set_var("HOME", dir.path());
        std::env::remove_var("USERPROFILE");

        let got = default_token_path().expect("HOME set, should resolve");
        assert_eq!(got, dir.path().join(".mori").join("gmail-token.json"));

        // 還原 — 避免污染同進程其他 test。
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        if let Some(v) = prev_userprofile {
            std::env::set_var("USERPROFILE", v);
        }
    }
}
