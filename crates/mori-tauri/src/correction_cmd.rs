//! Correction Inbox / Voice Feedback / Corrections viewer 的 Tauri commands wrapper。
//!
//! 對應 spec §4.4-4.7。

use chrono::Utc;
use mori_core::correction_inbox::{
    self, group_pending_by_suggested, InboxEntry, InboxGroup, InboxSource, InboxStatus,
};
use mori_core::corrections_writer::append_correction;
use mori_core::voice_feedback::{diff_words, write_feedback, Feedback, FeedbackRating};
use serde::Deserialize;
use std::path::PathBuf;

fn inbox_path() -> PathBuf {
    crate::mori_dir().join("correction_inbox.jsonl")
}

fn corrections_md_path() -> PathBuf {
    crate::mori_dir().join("corrections.md")
}

fn recordings_dir() -> PathBuf {
    crate::mori_dir().join("recordings")
}

#[tauri::command]
pub fn correction_inbox_list() -> Result<Vec<InboxGroup>, String> {
    group_pending_by_suggested(&inbox_path()).map_err(|e| e.to_string())
}

/// `wrong_variants` 是要寫進 corrections.md 那行的 wrong 字 list(已 dedupe by UI)。
/// 後台:append corrections.md + 對 inbox 內所有 (wrong ∈ wrong_variants, suggested) 都
/// append 一筆 status=Accepted 的 entry(append-only,舊 pending entries 留)。
#[tauri::command]
pub fn correction_inbox_accept(suggested: String, wrong_variants: Vec<String>) -> Result<(), String> {
    if wrong_variants.is_empty() {
        return Err("wrong_variants 空".into());
    }
    append_correction(&corrections_md_path(), &wrong_variants, &suggested)
        .map_err(|e| format!("append corrections.md: {e}"))?;

    let now = Utc::now();
    for wrong in &wrong_variants {
        let mut accepted = InboxEntry::new_pending(
            "marker",
            InboxSource::LlmAudit,
            wrong,
            &suggested,
            1.0,
            "user accepted",
        );
        accepted.status = InboxStatus::Accepted;
        accepted.accepted_at = Some(now);
        correction_inbox::append_entry(&inbox_path(), &accepted)
            .map_err(|e| format!("append accepted entry: {e}"))?;
    }
    mori_core::event_log::append(serde_json::json!({
        "kind": "correction_inbox_accepted",
        "suggested": suggested,
        "wrong_variants": wrong_variants,
    }));
    Ok(())
}

#[tauri::command]
pub fn correction_inbox_dismiss(suggested: String, wrong_variants: Vec<String>) -> Result<(), String> {
    if wrong_variants.is_empty() {
        return Err("wrong_variants 空".into());
    }
    let now = Utc::now();
    for wrong in &wrong_variants {
        let mut dismissed = InboxEntry::new_pending(
            "marker",
            InboxSource::LlmAudit,
            wrong,
            &suggested,
            0.0,
            "user dismissed",
        );
        dismissed.status = InboxStatus::Dismissed;
        dismissed.dismissed_at = Some(now);
        correction_inbox::append_entry(&inbox_path(), &dismissed)
            .map_err(|e| format!("append dismissed entry: {e}"))?;
    }
    mori_core::event_log::append(serde_json::json!({
        "kind": "correction_inbox_dismissed",
        "suggested": suggested,
        "wrong_variants": wrong_variants,
    }));
    Ok(())
}

/// 「刪除」一筆 inbox entry — 從 pending list 移走,但**不**加進 dismissed 白名單,也**不**寫
/// corrections.md。等同「這次跳過,別留痕跡」,下次 audit 再標同 (wrong, suggested) 還是會
/// 進 inbox(不像 dismiss 會永久過濾)。
///
/// 實作:對每個 (wrong, suggested) 寫 status=Accepted 但 **不 call append_correction**。
/// 因為 list_pending filter 只看 Pending,所以 entry 從 UI 消失;is_dismissed 只看
/// Dismissed,所以下次 audit 不 filter 掉。
#[tauri::command]
pub fn correction_inbox_delete(suggested: String, wrong_variants: Vec<String>) -> Result<(), String> {
    if wrong_variants.is_empty() {
        return Err("wrong_variants 空".into());
    }
    let now = Utc::now();
    for wrong in &wrong_variants {
        let mut deleted = InboxEntry::new_pending(
            "marker",
            InboxSource::LlmAudit,
            wrong,
            &suggested,
            0.0,
            "user deleted (skip once, no whitelist)",
        );
        deleted.status = InboxStatus::Accepted; // 用 Accepted 讓 list_pending 不再顯示
        deleted.accepted_at = Some(now);
        correction_inbox::append_entry(&inbox_path(), &deleted)
            .map_err(|e| format!("append deleted entry: {e}"))?;
    }
    mori_core::event_log::append(serde_json::json!({
        "kind": "correction_inbox_deleted",
        "suggested": suggested,
        "wrong_variants": wrong_variants,
    }));
    Ok(())
}

#[derive(Deserialize)]
pub struct ChangeSuggestionArgs {
    pub suggested: String,
    pub wrong_variants: Vec<String>,
    pub new_suggested: String,
}

#[tauri::command]
pub fn correction_inbox_change_suggestion(args: ChangeSuggestionArgs) -> Result<(), String> {
    // dismiss 原 (wrong, old_suggested) + accept 新 (wrong, new_suggested)
    correction_inbox_dismiss(args.suggested, args.wrong_variants.clone())?;
    correction_inbox_accept(args.new_suggested, args.wrong_variants)
}

#[derive(Deserialize)]
pub struct VoiceFeedbackArgs {
    pub session_id: String,
    pub rating: FeedbackRating,
    pub corrected_transcript: Option<String>,
    pub original_transcript: Option<String>,
    pub comment: Option<String>,
}

#[tauri::command]
pub fn voice_feedback_set(args: VoiceFeedbackArgs) -> Result<(), String> {
    let session_dir = recordings_dir().join(&args.session_id);
    let feedback = Feedback {
        rating: args.rating,
        rated_at: Utc::now(),
        corrected_transcript: args.corrected_transcript.clone(),
        comment: args.comment,
    };
    write_feedback(&session_dir, &feedback).map_err(|e| format!("write feedback.json: {e}"))?;

    // 若 rating=Edit 且兩段 transcript 都齊 → 跑 diff,把 (wrong, suggested) 寫進 inbox
    if matches!(args.rating, FeedbackRating::Edit) {
        if let (Some(orig), Some(corr)) = (args.original_transcript, args.corrected_transcript) {
            if orig != corr {
                let pairs = diff_words(&orig, &corr);
                for (wrong, suggested) in pairs {
                    // skip 純空白變化
                    if wrong.trim().is_empty() || suggested.trim().is_empty() {
                        continue;
                    }
                    let entry = InboxEntry::new_pending(
                        &args.session_id,
                        InboxSource::UserEdit,
                        &wrong,
                        &suggested,
                        0.95,
                        "user edit transcript diff",
                    );
                    correction_inbox::append_entry(&inbox_path(), &entry)
                        .map_err(|e| format!("append user_edit inbox entry: {e}"))?;
                }
            }
        }
    }
    mori_core::event_log::append(serde_json::json!({
        "kind": "voice_feedback_rated",
        "session_id": args.session_id,
        "rating": format!("{:?}", feedback.rating),
    }));
    Ok(())
}

#[tauri::command]
pub fn corrections_md_content() -> Result<String, String> {
    let path = corrections_md_path();
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("read corrections.md: {e}"))
}
