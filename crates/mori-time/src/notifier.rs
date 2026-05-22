//! K3 notifier — 桌面通知 wrap notify-rust。
//!
//! 把 [`Reminder`] 觸發轉成跨平台桌面通知:
//! - Linux: libnotify / libdbus(走 session dbus,需 `libdbus-1-3` 系統 lib)
//! - Windows: native toast
//! - macOS: NSUserNotification
//!
//! 為什麼自己包一層 wrapper(不直接讓 K2 / K5 用 `notify_rust::Notification`):
//!
//! 1. **可測**:`Notification::show()` 真去敲 dbus / system API,CI 沒 display 跟 dbus
//!    session,直接呼叫就會炸。我們把「組裝 Notification」跟「真發送」分兩步:
//!    [`Notifier::build_notification`](私有 helper)負責組欄位、回傳 `Notification`
//!    物件 — test 對這個物件的 `summary` / `body` / `appname` / `icon` 欄位做 assert,
//!    不會碰 dbus。[`Notifier::fire`] / [`Notifier::notify`] 再額外呼叫 `.show()`。
//!
//! 2. **Reminder 語意統一**:K2 scheduler / K5 Tauri command 都要從 [`Reminder`]
//!    轉通知,集中在這裡決定 summary/body 形狀,以後改格式只動一處。
//!
//! 3. **Error 收斂**:notify-rust 的 `Error` 各平台不同 type,我們統一吐
//!    [`NotifyError`] 給上層 handle。

use crate::schema::Reminder;
use notify_rust::Notification;

/// Notifier 發通知時可能出的錯。
#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    /// notify-rust / dbus / OS API 那一層失敗。
    #[error("notification failed: {0}")]
    Notify(String),
    /// 缺系統依賴(Linux 沒裝 libdbus 等)。
    ///
    /// 目前 [`Notifier::fire`] / [`Notifier::notify`] 不會主動偵測 system lib —
    /// notify-rust 找不到 libdbus 會 panic / Err,我們把那個 err 包成 [`Self::Notify`]。
    /// 這條 variant 留給未來「主動 preflight」用(也讓 caller `match` 看得到形狀)。
    #[error("missing system dep: {0}")]
    MissingDep(String),
}

/// 桌面通知發送器。
///
/// Cheap to construct,可以 hold 在 K2 scheduler / K5 Tauri AppState 裡長期重用。
/// 沒有內部 mutable state,`fire` / `notify` 取 `&self`,可跨 thread 用。
#[derive(Debug, Clone)]
pub struct Notifier {
    /// 通知右下顯示的 app 名稱(Linux freedesktop 規範 `appname` 欄)。
    app_name: String,
    /// 通知圖示。Linux 走 freedesktop icon theme name(像 "dialog-information")
    /// 或 absolute path;Windows/macOS 多半 ignore。`None` = 不設,讓 desktop env
    /// 用 default。
    icon_path: Option<String>,
}

impl Notifier {
    /// 新建一個 Notifier。`app_name` 通常是 "Mori"。
    pub fn new(app_name: impl Into<String>) -> Self {
        Self {
            app_name: app_name.into(),
            icon_path: None,
        }
    }

    /// Builder:設 icon 路徑 / freedesktop icon 名。
    pub fn with_icon(mut self, icon_path: impl Into<String>) -> Self {
        self.icon_path = Some(icon_path.into());
        self
    }

    /// 拿 app name(test / inspect 用)。
    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    /// 拿 icon path(test / inspect 用)。
    pub fn icon_path(&self) -> Option<&str> {
        self.icon_path.as_deref()
    }

    /// 從 Reminder 觸發桌面通知。
    ///
    /// summary 用 reminder text、body 是 "Mori 提醒你"。會真的呼叫 `.show()`,
    /// 在沒 dbus / 沒 display 的環境會 Err。
    pub fn fire(&self, reminder: &Reminder) -> Result<(), NotifyError> {
        let n = self.build_for_reminder(reminder);
        Self::show(&n)
    }

    /// 純文字通知(不綁 Reminder)。K5 / 外部 caller 用。
    pub fn notify(&self, subject: &str, body: &str) -> Result<(), NotifyError> {
        let n = self.build_text(subject, body);
        Self::show(&n)
    }

    // ─── 以下 helpers 公開給 unit test 用,production 不直接呼叫 ─────────────

    /// 組「Reminder 通知」對應的 [`Notification`] 物件 — 不發送。
    ///
    /// summary = reminder.text、body = "Mori 提醒你"。
    /// 故意設 `pub(crate)` 不對外:外部 caller 應該走 [`Self::fire`],
    /// 這個 helper 是為了讓 unit test 能 assert 內容。
    pub(crate) fn build_for_reminder(&self, reminder: &Reminder) -> Notification {
        self.build_text(&reminder.text, "Mori 提醒你")
    }

    /// 組純文字通知對應的 [`Notification`] 物件 — 不發送。
    ///
    /// **Sticky by default**:freedesktop `Timeout::Never` + Linux Resident hint +
    /// Critical urgency,user 必須手動點關才會消(2026-05-22 user 需求:reminder
    /// 不能彈一下就消失,user 在錄音/講話時錯過就找不回來)。GNOME 預設對 Normal
    /// urgency 會強制 auto-dismiss,所以一定要 Critical 才留得住;KDE 對 Resident
    /// hint 就聽話,不一定要 Critical。三層下去最穩。
    ///
    /// Windows/macOS:.timeout() / .urgency() / .hint() 在那兩平台是 no-op
    /// (notify-rust 在 cfg 上會 strip 掉 Linux 專屬的 hint / urgency)。
    pub(crate) fn build_text(&self, summary: &str, body: &str) -> Notification {
        let mut n = Notification::new();
        n.appname(&self.app_name).summary(summary).body(body);
        if let Some(icon) = &self.icon_path {
            n.icon(icon);
        }
        // freedesktop 標準「永不超時」— 0 = 不自動關
        n.timeout(notify_rust::Timeout::Never);
        // Linux/BSD 才有 Hint / Urgency type,Windows/macOS 沒這個概念
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            n.hint(notify_rust::Hint::Resident(true));
            n.urgency(notify_rust::Urgency::Critical);
        }
        n.finalize()
    }

    /// 把 `Notification` 真的送出去。集中包 `.show()` 是為了把 notify-rust 的
    /// 平台特定 Error 收斂成 [`NotifyError::Notify`]。
    ///
    /// Linux 沒 dbus session / Windows 沒對應 API、macOS 沒權限 → 一律 Err,不 panic。
    fn show(n: &Notification) -> Result<(), NotifyError> {
        // Linux:`.show()` 回 `Result<NotificationHandle>`;
        // Windows/macOS:回 `Result<()>` 或 `Result<NotificationHandle>`(macOS)。
        // 我們不關心 handle,丟掉只看 success/failure。
        n.show().map(|_| ()).map_err(|e| {
            // notify-rust 的 dbus 失敗 message 大概長 "No such interface" / "no
            // session bus" / "spawn error"。我們不做精細分類,全包 Notify。
            // 未來想 preflight detect libdbus 再考慮 hoisting 到 MissingDep。
            NotifyError::Notify(e.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    //! 桌面通知的 `.show()` path 在 CI / headless 環境會炸(沒 dbus / 沒 display),
    //! 因此 tests 全走 [`Notifier::build_for_reminder`] / [`Notifier::build_text`]
    //! pure helper — 對組好的 `Notification` 物件 assert 欄位內容,不碰 system API。

    use super::*;
    use crate::schema::ReminderStatus;
    use chrono::Utc;

    fn sample_reminder(text: &str) -> Reminder {
        let now = Utc::now();
        Reminder {
            id: 1,
            text: text.to_string(),
            due_at: now,
            cron_expr: None,
            created_at: now,
            fired_at: None,
            dismissed_at: None,
            snoozed_until: None,
            status: ReminderStatus::Pending,
        }
    }

    #[test]
    fn notifier_builds_correct_summary_from_reminder() {
        let n = Notifier::new("Mori");
        let r = sample_reminder("喝水");
        let built = n.build_for_reminder(&r);
        assert_eq!(built.summary, "喝水");
    }

    #[test]
    fn notifier_builds_correct_body_from_reminder() {
        let n = Notifier::new("Mori");
        let r = sample_reminder("散步");
        let built = n.build_for_reminder(&r);
        // body 固定文案 — 不跟 reminder text 重複,避免 summary/body 一樣冗。
        assert_eq!(built.body, "Mori 提醒你");
    }

    #[test]
    fn notifier_uses_app_name() {
        let n = Notifier::new("Mori-Test-App");
        let r = sample_reminder("a");
        let built = n.build_for_reminder(&r);
        assert_eq!(built.appname, "Mori-Test-App");
    }

    #[test]
    fn notifier_uses_icon_if_set() {
        // 沒設 icon → Notification.icon 是 default(空字串)
        let n_no_icon = Notifier::new("Mori");
        let built_no_icon = n_no_icon.build_text("subj", "body");
        assert!(
            built_no_icon.icon.is_empty(),
            "expected empty icon when none set, got: {:?}",
            built_no_icon.icon
        );

        // 設了 icon → Notification.icon 帶 path
        let n_with_icon = Notifier::new("Mori").with_icon("/tmp/mori-icon.png");
        let built_with_icon = n_with_icon.build_text("subj", "body");
        assert_eq!(built_with_icon.icon, "/tmp/mori-icon.png");
        // 同時 getter 也應該回值
        assert_eq!(n_with_icon.icon_path(), Some("/tmp/mori-icon.png"));
        assert_eq!(n_with_icon.app_name(), "Mori");
    }

    #[test]
    fn notify_text_helper_separates_subject_and_body() {
        // 純文字 path(K5 / external use)— subject / body 兩段是分開的。
        let n = Notifier::new("Mori");
        let built = n.build_text("吃藥", "別忘記吃晚餐後的藥");
        assert_eq!(built.summary, "吃藥");
        assert_eq!(built.body, "別忘記吃晚餐後的藥");
        assert_eq!(built.appname, "Mori");
    }
}
