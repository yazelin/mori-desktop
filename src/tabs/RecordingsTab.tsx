// Phase B per-pipeline recordings UI(v0.6.2 ship 完 backend,v0.6.3 上 UI)。
//
// 列出 ~/.mori/recordings/<ts>/ 內每個 session,展開可看:
//   - meta.json(timing / scores / profile / provider / skill_calls)
//   - context.json(clipboard / selection / window / urls)
//   - history.json(對話 history 摘要)
//   - transcript.txt + response.txt
//   - system-prompt.txt(完整 prompt LLM 看的)
//   - audio-raw.wav / audio-trimmed.wav(可播放)
//
// 設計:
//   - 列表 mount 載 summary(輕量,不含 audio bytes)
//   - 點開單筆 → lazy fetch session_detail
//   - 點播放 audio → 拉 bytes → blob URL <audio>
//   - 刪除單筆 → confirm → IPC

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type SessionSummary = {
  timestamp: string;
  iso_time: string;
  mode: string | null;
  profile: string | null;
  provider: string | null;
  transcript_preview: string | null;
  response_preview: string | null;
  duration_ms: number | null;
  size_bytes: number;
};

type SessionDetail = {
  timestamp: string;
  meta: Record<string, unknown> | null;
  context: Record<string, unknown> | null;
  history: Record<string, unknown> | null;
  transcript: string | null;
  response: string | null;
  system_prompt: string | null;
  has_audio_raw: boolean;
  has_audio_trimmed: boolean;
};

type RecordingsStats = {
  session_count: number;
  total_bytes: number;
  retention_days: number;
  enabled: boolean;
};

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function formatDuration(ms: number | null): string {
  if (!ms) return "—";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatTime(iso: string): string {
  // ISO 8601 → 「2026-05-19 17:12:34」(去掉 T 跟微秒)
  return iso.replace("T", " ").substring(0, 19);
}

function RecordingsTab() {
  const { t: _ } = useTranslation(); // i18n 之後接,目前先 hardcode 中文
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [stats, setStats] = useState<RecordingsStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [audioUrls, setAudioUrls] = useState<{ raw?: string; trimmed?: string }>({});
  const [modeFilter, setModeFilter] = useState<string>("all");
  const [providerFilter, setProviderFilter] = useState<string>("all");
  const [msg, setMsg] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    try {
      const [list, st] = await Promise.all([
        invoke<SessionSummary[]>("recordings_list"),
        invoke<RecordingsStats>("recordings_stats"),
      ]);
      setSessions(list);
      setStats(st);
    } catch (e) {
      console.error("recordings list", e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  // 清掉 blob URL(避免 memory leak)
  useEffect(() => {
    return () => {
      Object.values(audioUrls).forEach((u) => u && URL.revokeObjectURL(u));
    };
  }, [audioUrls]);

  const flashMsg = (m: string) => {
    setMsg(m);
    setTimeout(() => setMsg(null), 2500);
  };

  const onExpand = async (timestamp: string) => {
    // toggle:再點同一筆 → 收起
    if (expanded === timestamp) {
      setExpanded(null);
      setDetail(null);
      Object.values(audioUrls).forEach((u) => u && URL.revokeObjectURL(u));
      setAudioUrls({});
      return;
    }
    setExpanded(timestamp);
    setDetail(null);
    Object.values(audioUrls).forEach((u) => u && URL.revokeObjectURL(u));
    setAudioUrls({});
    try {
      const d = await invoke<SessionDetail>("recordings_session_detail", { timestamp });
      setDetail(d);
    } catch (e) {
      flashMsg(`讀失敗:${String(e)}`);
    }
  };

  const loadAudio = async (timestamp: string, which: "raw" | "trimmed") => {
    if (audioUrls[which]) return; // 已 load
    try {
      const bytes = await invoke<number[]>("recordings_audio_bytes", { timestamp, which });
      const blob = new Blob([new Uint8Array(bytes)], { type: "audio/wav" });
      const url = URL.createObjectURL(blob);
      setAudioUrls((prev) => ({ ...prev, [which]: url }));
    } catch (e) {
      flashMsg(`audio 讀失敗:${String(e)}`);
    }
  };

  const onDelete = async (timestamp: string) => {
    if (!confirm(`刪除 session ${timestamp}?(audio + transcript + 全部 metadata)`)) return;
    try {
      await invoke("recordings_delete_session", { timestamp });
      if (expanded === timestamp) setExpanded(null);
      flashMsg(`已刪除 ${timestamp}`);
      await refresh();
    } catch (e) {
      flashMsg(`刪除失敗:${String(e)}`);
    }
  };

  const modes = useMemo(() => {
    const s = new Set<string>();
    sessions.forEach((x) => x.mode && s.add(x.mode));
    return ["all", ...Array.from(s)];
  }, [sessions]);

  const providers = useMemo(() => {
    const s = new Set<string>();
    sessions.forEach((x) => x.provider && s.add(x.provider));
    return ["all", ...Array.from(s)];
  }, [sessions]);

  const filtered = sessions.filter((s) => {
    if (modeFilter !== "all" && s.mode !== modeFilter) return false;
    if (providerFilter !== "all" && s.provider !== providerFilter) return false;
    return true;
  });

  return (
    <div className="mori-tab">
      <h2 className="mori-tab-title">Recordings</h2>
      <p className="mori-tab-hint">
        每次 voice pipeline 的完整 I/O snapshot(audio / transcript / system prompt /
        context / meta)。給 Whisper fine-tune dataset、debug、隱私自管用。
        資料夾:<code>~/.mori/recordings/</code>
      </p>

      {/* Stats bar */}
      {stats && (
        <div
          style={{
            display: "flex",
            gap: 18,
            padding: "10px 14px",
            background: "var(--c-surface-bg)",
            border: "1px solid var(--c-border)",
            borderRadius: 6,
            fontSize: 13,
            marginBottom: 12,
            alignItems: "center",
          }}
        >
          <span>
            <strong>{stats.session_count}</strong> sessions
          </span>
          <span>
            <strong>{formatBytes(stats.total_bytes)}</strong> disk
          </span>
          <span style={{ color: "var(--c-text-muted)" }}>
            retention <strong>{stats.retention_days}</strong> days
            {!stats.enabled && (
              <span style={{ marginLeft: 8, color: "var(--c-danger-text)" }}>(recordings disabled)</span>
            )}
          </span>
          <span style={{ marginLeft: "auto", display: "flex", gap: 8 }}>
            <label style={{ fontSize: 12, color: "var(--c-text-muted)" }}>
              mode{" "}
              <select
                value={modeFilter}
                onChange={(e) => setModeFilter(e.target.value)}
                style={{ marginLeft: 4 }}
              >
                {modes.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </label>
            <label style={{ fontSize: 12, color: "var(--c-text-muted)" }}>
              provider{" "}
              <select
                value={providerFilter}
                onChange={(e) => setProviderFilter(e.target.value)}
                style={{ marginLeft: 4 }}
              >
                {providers.map((p) => (
                  <option key={p} value={p}>
                    {p}
                  </option>
                ))}
              </select>
            </label>
            <button className="mori-btn small ghost" onClick={refresh}>
              重新整理
            </button>
          </span>
        </div>
      )}

      {msg && (
        <div style={{ padding: 8, fontSize: 12, color: "var(--c-text-muted)" }}>{msg}</div>
      )}

      {loading && <div className="mori-tab-placeholder"><p>讀取中...</p></div>}

      {!loading && filtered.length === 0 && (
        <div className="mori-tab-placeholder">
          <p>沒 session(或都被 filter 掉)。對 mic 講一輪 Mori 試試。</p>
        </div>
      )}

      {/* Session list */}
      {!loading && filtered.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {filtered.map((s) => (
            <div
              key={s.timestamp}
              style={{
                border: "1px solid var(--c-border)",
                borderRadius: 6,
                background: "var(--c-surface-bg)",
              }}
            >
              {/* Row header */}
              <div
                onClick={() => onExpand(s.timestamp)}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  padding: "8px 12px",
                  cursor: "pointer",
                  borderBottom: expanded === s.timestamp ? "1px solid var(--c-border)" : "none",
                }}
              >
                <span style={{ fontFamily: "monospace", fontSize: 12, color: "var(--c-text-muted)", minWidth: 160 }}>
                  {formatTime(s.iso_time)}
                </span>
                <span style={{
                  fontSize: 11,
                  padding: "1px 6px",
                  background: "var(--c-input-bg)",
                  border: "1px solid var(--c-border)",
                  borderRadius: 3,
                  color: "var(--c-text-muted)",
                }}>
                  {s.mode ?? "?"}
                </span>
                <span style={{ fontSize: 11, color: "var(--c-text-muted)" }}>
                  {s.profile ?? "—"} · {s.provider ?? "—"} · {formatDuration(s.duration_ms)} · {formatBytes(s.size_bytes)}
                </span>
                <span style={{ fontSize: 13, flex: 1, marginLeft: 8 }}>
                  {s.transcript_preview ? `「${s.transcript_preview}」` : <em style={{ color: "var(--c-text-muted)" }}>(無 transcript)</em>}
                </span>
                <span style={{ color: "var(--c-text-muted)" }}>
                  {expanded === s.timestamp ? "▾" : "▸"}
                </span>
              </div>

              {/* Expanded detail */}
              {expanded === s.timestamp && (
                <div style={{ padding: 12 }}>
                  {!detail && <p style={{ color: "var(--c-text-muted)", fontSize: 13 }}>讀取 detail 中...</p>}
                  {detail && (
                    <>
                      {/* Transcript / response */}
                      {detail.transcript && (
                        <DetailField label="Transcript" content={detail.transcript} />
                      )}
                      {detail.response && (
                        <DetailField label="Response" content={detail.response} />
                      )}

                      {/* Audio players */}
                      {(detail.has_audio_raw || detail.has_audio_trimmed) && (
                        <div style={{ marginTop: 10 }}>
                          <div style={{ fontSize: 12, color: "var(--c-text-muted)", marginBottom: 4 }}>
                            🎵 Audio
                          </div>
                          {detail.has_audio_raw && (
                            <AudioRow
                              label="raw(silence-trim 前)"
                              url={audioUrls.raw}
                              onLoad={() => loadAudio(s.timestamp, "raw")}
                            />
                          )}
                          {detail.has_audio_trimmed && (
                            <AudioRow
                              label="trimmed(STT 用)"
                              url={audioUrls.trimmed}
                              onLoad={() => loadAudio(s.timestamp, "trimmed")}
                            />
                          )}
                        </div>
                      )}

                      {/* Meta json — collapsible */}
                      <CollapsibleJson label="meta.json" data={detail.meta} defaultOpen />
                      <CollapsibleJson label="context.json" data={detail.context} />
                      <CollapsibleJson label="history.json" data={detail.history} />
                      {detail.system_prompt && (
                        <DetailField
                          label={`system-prompt.txt (${detail.system_prompt.length} chars)`}
                          content={detail.system_prompt}
                          collapsible
                        />
                      )}

                      {/* Actions */}
                      <div style={{ marginTop: 14, textAlign: "right" }}>
                        <button
                          className="mori-btn small ghost"
                          onClick={() => onDelete(s.timestamp)}
                          style={{ color: "var(--c-danger-text)" }}
                        >
                          ✕ 刪除這個 session
                        </button>
                      </div>
                    </>
                  )}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function DetailField({
  label,
  content,
  collapsible = false,
}: {
  label: string;
  content: string;
  collapsible?: boolean;
}) {
  const [open, setOpen] = useState(!collapsible);
  return (
    <div style={{ marginTop: 8 }}>
      <div
        onClick={collapsible ? () => setOpen(!open) : undefined}
        style={{
          fontSize: 12,
          color: "var(--c-text-muted)",
          marginBottom: 4,
          cursor: collapsible ? "pointer" : "default",
        }}
      >
        {collapsible && <span style={{ marginRight: 4 }}>{open ? "▾" : "▸"}</span>}
        {label}
      </div>
      {open && (
        <pre
          style={{
            padding: 8,
            background: "var(--c-input-bg)",
            color: "var(--c-text)",
            border: "1px solid var(--c-border)",
            borderRadius: 4,
            fontSize: 12,
            lineHeight: 1.55,
            whiteSpace: "pre-wrap",
            maxHeight: 280,
            overflow: "auto",
            margin: 0,
          }}
        >
          {content}
        </pre>
      )}
    </div>
  );
}

function CollapsibleJson({
  label,
  data,
  defaultOpen = false,
}: {
  label: string;
  data: unknown;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  if (data === null || data === undefined) return null;
  return (
    <div style={{ marginTop: 8 }}>
      <div
        onClick={() => setOpen(!open)}
        style={{
          fontSize: 12,
          color: "var(--c-text-muted)",
          marginBottom: 4,
          cursor: "pointer",
        }}
      >
        <span style={{ marginRight: 4 }}>{open ? "▾" : "▸"}</span>
        {label}
      </div>
      {open && (
        <pre
          style={{
            padding: 8,
            background: "var(--c-input-bg)",
            color: "var(--c-text)",
            border: "1px solid var(--c-border)",
            borderRadius: 4,
            fontSize: 11,
            lineHeight: 1.55,
            whiteSpace: "pre-wrap",
            maxHeight: 240,
            overflow: "auto",
            fontFamily: "ui-monospace, monospace",
            margin: 0,
          }}
        >
          {JSON.stringify(data, null, 2)}
        </pre>
      )}
    </div>
  );
}

function AudioRow({
  label,
  url,
  onLoad,
}: {
  label: string;
  url?: string;
  onLoad: () => void;
}) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
      <span style={{ fontSize: 11, color: "var(--c-text-muted)", minWidth: 160 }}>{label}</span>
      {url ? (
        <audio controls src={url} style={{ height: 28, flex: 1 }} />
      ) : (
        <button className="mori-btn small ghost" onClick={onLoad}>
          ▶ 載入
        </button>
      )}
    </div>
  );
}

export default RecordingsTab;
