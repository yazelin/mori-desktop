// 2026-05-22:校正盒 — STT 諧音錯字 inbox + corrections.md viewer。
//
// 兩個 section:
// 1. Inbox(校正盒)— pending entries 按 suggested 字 grouping,每行 row
//    顯示 variants(wrong × count),接受 / 改建議 / 忽略
// 2. corrections.md viewer — readonly markdown view,顯示既有字典

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconCheck, IconClose } from "../icons";
import "./corrections-tab.css";

type InboxVariant = {
  wrong: string;
  count: number;
  entry_ids: string[];
  earliest_session: string;
  max_confidence: number;
};

type InboxGroup = {
  suggested: string;
  variants: InboxVariant[];
  has_user_edit: boolean;
};

function CorrectionsTab() {
  const [groups, setGroups] = useState<InboxGroup[]>([]);
  const [corrections, setCorrections] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<Record<string, boolean>>({});

  const refresh = async () => {
    setLoading(true);
    try {
      const [g, c] = await Promise.all([
        invoke<InboxGroup[]>("correction_inbox_list"),
        invoke<string>("corrections_md_content"),
      ]);
      setGroups(g);
      setCorrections(c);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const onAccept = async (group: InboxGroup, override?: string) => {
    const target = override ?? group.suggested;
    setBusy((b) => ({ ...b, [group.suggested]: true }));
    try {
      const wrongs = group.variants.map((v) => v.wrong);
      if (override && override !== group.suggested) {
        await invoke("correction_inbox_change_suggestion", {
          args: {
            suggested: group.suggested,
            wrong_variants: wrongs,
            new_suggested: override,
          },
        });
      } else {
        await invoke("correction_inbox_accept", {
          suggested: target,
          wrongVariants: wrongs,
        });
      }
      await refresh();
    } catch (e) {
      alert(`接受失敗:${e}`);
    } finally {
      setBusy((b) => ({ ...b, [group.suggested]: false }));
      setEditing((s) => {
        const copy = { ...s };
        delete copy[group.suggested];
        return copy;
      });
    }
  };

  const onDismiss = async (group: InboxGroup) => {
    setBusy((b) => ({ ...b, [group.suggested]: true }));
    try {
      const wrongs = group.variants.map((v) => v.wrong);
      await invoke("correction_inbox_dismiss", {
        suggested: group.suggested,
        wrongVariants: wrongs,
      });
      await refresh();
    } catch (e) {
      alert(`忽略失敗:${e}`);
    } finally {
      setBusy((b) => ({ ...b, [group.suggested]: false }));
    }
  };

  if (loading) return <div className="corrections-tab">載入中...</div>;

  return (
    <div className="corrections-tab">
      {error && <div className="corrections-error">⚠ {error}</div>}

      <section className="mori-config-section">
        <h3 className="mori-config-section-title">
          校正盒
          {groups.length > 0 && (
            <span className="corrections-count">({groups.length} 個 pending)</span>
          )}
        </h3>

        {groups.length === 0 ? (
          <p className="mori-config-section-hint">沒有待處理候選。Mori 對話結束後會自動偵測諧音錯字放進來。</p>
        ) : (
          <ul className="corrections-inbox-list">
            {groups.map((group) => {
              const isBusy = busy[group.suggested];
              const editingValue = editing[group.suggested];
              const isEditing = editingValue !== undefined;
              return (
                <li
                  key={group.suggested}
                  className={`corrections-inbox-row ${group.has_user_edit ? "is-user-edit" : ""}`}
                >
                  <div className="corrections-row-head">
                    {isEditing ? (
                      <input
                        type="text"
                        className="corrections-suggested-edit"
                        value={editingValue}
                        onChange={(e) =>
                          setEditing((s) => ({ ...s, [group.suggested]: e.target.value }))
                        }
                      />
                    ) : (
                      <span className="corrections-suggested">{group.suggested}</span>
                    )}
                    <span className="corrections-arrow">←</span>
                    <span className="corrections-variants">
                      {group.variants.map((v, i) => (
                        <span key={v.wrong}>
                          {i > 0 && ", "}
                          {v.wrong}
                          {v.count > 1 && (
                            <span className="corrections-count-badge">×{v.count}</span>
                          )}
                        </span>
                      ))}
                    </span>
                  </div>
                  <div className="corrections-row-actions">
                    {isEditing ? (
                      <>
                        <button
                          className="mori-btn small"
                          disabled={isBusy}
                          onClick={() => onAccept(group, editingValue)}
                        >
                          <IconCheck width={12} height={12} /> 接受新建議
                        </button>
                        <button
                          className="mori-btn small ghost"
                          disabled={isBusy}
                          onClick={() =>
                            setEditing((s) => {
                              const copy = { ...s };
                              delete copy[group.suggested];
                              return copy;
                            })
                          }
                        >
                          取消
                        </button>
                      </>
                    ) : (
                      <>
                        <button className="mori-btn small primary" disabled={isBusy} onClick={() => onAccept(group)}>
                          <IconCheck width={12} height={12} /> 接受
                        </button>
                        <button
                          className="mori-btn small"
                          disabled={isBusy}
                          onClick={() =>
                            setEditing((s) => ({ ...s, [group.suggested]: group.suggested }))
                          }
                        >
                          改建議
                        </button>
                        <button className="mori-btn small ghost" disabled={isBusy} onClick={() => onDismiss(group)}>
                          <IconClose width={12} height={12} /> 忽略
                        </button>
                      </>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section className="mori-config-section">
        <h3 className="mori-config-section-title">字典 corrections.md</h3>
        <textarea
          className="mori-config-textarea"
          readOnly
          rows={16}
          value={corrections || "(空)"}
        />
      </section>
    </div>
  );
}

export default CorrectionsTab;
