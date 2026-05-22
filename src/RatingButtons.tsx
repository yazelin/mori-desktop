// 2026-05-23: RatingButtons — voice message / session 旁的 👍/👎/✏️ 評分按鈕。
//
// Chat panel / RecordingsTab 都用同一個。評分寫進 ~/.mori/recordings/<session>/feedback.json,
// ✏️ 改寫額外觸發 transcript diff,把改過的詞作為 user_edit candidate 進 correction inbox。

import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./rating-buttons.css";

export type RatingButtonsProps = {
  sessionId: string;
  originalTranscript: string;
};

export function RatingButtons({ sessionId, originalTranscript }: RatingButtonsProps) {
  const [editing, setEditing] = useState(false);
  const [correctedText, setCorrectedText] = useState(originalTranscript);
  const [busy, setBusy] = useState(false);

  const setRating = async (rating: "good" | "bad") => {
    setBusy(true);
    try {
      await invoke("voice_feedback_set", {
        args: {
          session_id: sessionId,
          rating,
          corrected_transcript: null,
          original_transcript: null,
          comment: null,
        },
      });
    } catch (e) {
      alert(`評分失敗:${e}`);
    } finally {
      setBusy(false);
    }
  };

  const submitEdit = async () => {
    setBusy(true);
    try {
      await invoke("voice_feedback_set", {
        args: {
          session_id: sessionId,
          rating: "edit",
          corrected_transcript: correctedText,
          original_transcript: originalTranscript,
          comment: null,
        },
      });
      setEditing(false);
    } catch (e) {
      alert(`改寫儲存失敗:${e}`);
    } finally {
      setBusy(false);
    }
  };

  if (editing) {
    return (
      <div className="rating-edit-box">
        <textarea
          value={correctedText}
          onChange={(e) => setCorrectedText(e.target.value)}
          rows={3}
        />
        <div>
          <button disabled={busy} onClick={submitEdit}>儲存</button>
          <button disabled={busy} onClick={() => setEditing(false)}>取消</button>
        </div>
      </div>
    );
  }

  return (
    <div className="rating-buttons">
      <button disabled={busy} onClick={() => setRating("good")} title="這次轉文字準確">👍</button>
      <button disabled={busy} onClick={() => setRating("bad")} title="這次轉文字不準">👎</button>
      <button disabled={busy} onClick={() => setEditing(true)} title="改寫成正確的">✏️</button>
    </div>
  );
}

export default RatingButtons;
