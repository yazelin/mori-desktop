import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

interface BodyInterface { name: string; transport: string; base_url?: string; url?: string; }
interface BodyManifest { id: string; name: string; interfaces?: BodyInterface[]; }
interface DiscoveredBodyPart { source: string; status: string; manifest: BodyManifest | null; }

interface SessionInfo {
  id: string; provider: string; state: string; project_name: string;
  cwd?: string | null;
  is_active: boolean; formatted_time: string;
}
interface SessionsSnapshot { sessions: SessionInfo[]; active_count: number; }
interface Cue {
  event_id: string; type: string; session_id: string; severity: string;
  summary: string; time: string;
}

// last-action-wins 的本地 state map(由 cue_state_list 回填,SSE 不帶這個)。
// serde tag = "kind" + snake_case rename。
type CueAction =
  | { kind: "ack" }
  | { kind: "snooze"; until: string }
  | { kind: "dismiss" };
type CueStateMap = Record<string, CueAction>;

type Effective = "unread" | "acked" | "snoozed" | "dismissed";

function effectiveState(cueId: string, state: CueStateMap, nowIso: string): Effective {
  const a = state[cueId];
  if (!a) return "unread";
  if (a.kind === "ack") return "acked";
  if (a.kind === "dismiss") return "dismissed";
  if (a.kind === "snooze") {
    // 字串字典序對 RFC3339 偏移混雜不安全,走 timestamp 比。
    return new Date(a.until).getTime() > new Date(nowIso).getTime() ? "snoozed" : "unread";
  }
  return "unread";
}

export default function PulseTab() {
  const { t } = useTranslation();
  const [base, setBase] = useState<string | null>(null);
  const [sseUrl, setSseUrl] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [cues, setCues] = useState<Cue[]>([]);
  const [cueState, setCueState] = useState<CueStateMap>({});
  const [now, setNow] = useState<string>(() => new Date().toISOString());
  const [err, setErr] = useState<string | null>(null);
  const seen = useRef<Set<string>>(new Set());

  // 1) 從 BI-1 registry 找 AgentPulse manifest,取 http base + sse url。
  const discover = async () => {
    try {
      const parts = await invoke<DiscoveredBodyPart[]>("body_registry_list");
      const ap = parts.find((p) => p.manifest?.id === "mori.agent-pulse");
      if (!ap?.manifest?.interfaces) { setBase(null); setSseUrl(null); return; }
      const http = ap.manifest.interfaces.find((i) => i.transport === "http")?.base_url ?? null;
      const sse = ap.manifest.interfaces.find((i) => i.transport === "sse")?.url ?? null;
      setBase(http); setSseUrl(sse); setErr(null);
    } catch (e: any) { setErr(String(e)); }
  };
  useEffect(() => { discover(); }, []);

  // 2) 抓 session 清單(輪詢備援,SSE 只送 cue 不送全清單)。
  const refreshSessions = async () => {
    if (!base) return;
    try {
      const r = await fetch(`${base}/sessions`);
      const snap: SessionsSnapshot = await r.json();
      setSessions(snap.sessions ?? []);
    } catch { /* AgentPulse 沒跑就忽略 */ }
  };
  useEffect(() => {
    if (!base) return;
    refreshSessions();
    const id = setInterval(refreshSessions, 5000);
    return () => clearInterval(id);
  }, [base]);

  // 3) SSE 訂閱 cue(dedup by event_id)。
  useEffect(() => {
    if (!sseUrl) return;
    const es = new EventSource(sseUrl);
    es.onmessage = (e) => {
      try {
        const cue: Cue = JSON.parse(e.data);
        if (seen.current.has(cue.event_id)) return;
        seen.current.add(cue.event_id);
        setCues((prev) => [cue, ...prev].slice(0, 50));
        refreshSessions();
      } catch { /* ignore non-json keepalive */ }
    };
    es.onerror = () => { /* EventSource 會自動重連 */ };
    return () => es.close();
  }, [sseUrl]);

  // 4) BI-4:載 cue state map(ack / snooze / dismiss 持久化在 ~/.mori/cue-state.jsonl)。
  const reloadCueState = async () => {
    try {
      const m = await invoke<CueStateMap>("cue_state_list");
      setCueState(m ?? {});
    } catch { /* 缺檔 → 空 map */ }
  };
  useEffect(() => { reloadCueState(); }, []);

  // 5) 30 秒 tick 推 `now`,讓過期 snooze 自動復活。
  useEffect(() => {
    const id = setInterval(() => setNow(new Date().toISOString()), 30_000);
    return () => clearInterval(id);
  }, []);

  const notRunning = base === null;

  return (
    <div className="mori-tab">
      <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 6 }}>
        <h2 className="mori-tab-title" style={{ marginBottom: 0 }}>{t("pulse_tab.title")}</h2>
        <button className="mori-btn small ghost" onClick={() => { discover(); refreshSessions(); reloadCueState(); }}>
          {t("pulse_tab.refresh")}
        </button>
      </div>
      <p className="mori-tab-hint">{t("pulse_tab.hint")}</p>
      {err && <div className="mori-tab-error" style={{ fontSize: 12, marginBottom: 12 }}>❌ {err}</div>}
      {notRunning && <div className="mori-tab-placeholder">{t("pulse_tab.not_running")}</div>}

      {!notRunning && (
        <>
          <h3 style={{ marginBottom: 6 }}>{t("pulse_tab.sessions_title")}</h3>
          {sessions.length === 0 && <div style={{ opacity: 0.6 }}>{t("pulse_tab.sessions_empty")}</div>}
          <div style={{ display: "flex", flexDirection: "column", gap: 8, marginBottom: 16 }}>
            {sessions.map((s) => (
              <div key={s.id} style={{ border: "1px solid var(--c-border)", borderRadius: 8, padding: 10 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <StateBadge state={s.state} />
                  <strong>{s.project_name}</strong>
                  <span style={{ fontSize: 11, opacity: 0.7 }}>{s.provider}</span>
                  <span style={{ fontSize: 11, opacity: 0.5, marginLeft: "auto" }}>{s.formatted_time}</span>
                </div>
              </div>
            ))}
          </div>

          <h3 style={{ marginBottom: 6 }}>{t("pulse_tab.cues_title")}</h3>
          {cues.length === 0 && <div style={{ opacity: 0.6 }}>{t("pulse_tab.cues_empty")}</div>}
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {cues
              .map((c) => ({ c, eff: effectiveState(c.event_id, cueState, now) }))
              .filter(({ eff }) => eff !== "dismissed" && eff !== "snoozed")
              .map(({ c, eff }) => (
                <CueRow
                  key={c.event_id}
                  cue={c}
                  effective={eff}
                  session={sessions.find((s) => s.id === c.session_id) ?? null}
                  onChanged={reloadCueState}
                />
              ))}
          </div>
        </>
      )}
    </div>
  );
}

function StateBadge({ state }: { state: string }) {
  const tone =
    state === "working" ? "tone-success"
    : state === "waiting_for_user" ? "tone-warning"
    : "tone-neutral";
  return <span className={`mori-pill-badge ${tone}`}>{state}</span>;
}

function CueBadge({ type }: { type: string }) {
  const { t } = useTranslation();
  if (type === "cue.waiting_input")
    return <span className="mori-pill-badge tone-warning">{t("pulse_tab.cue_waiting")}</span>;
  if (type === "cue.done")
    return <span className="mori-pill-badge tone-success">{t("pulse_tab.cue_done")}</span>;
  return <span className="mori-pill-badge tone-neutral">{type}</span>;
}

// 真正的 action row 在 Task 4 寫。Task 3 先放 placeholder 讓 build 過。
function CueRow(
  { cue, effective }:
    { cue: Cue; effective: Effective; session: SessionInfo | null; onChanged: () => void }
) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13, opacity: effective === "acked" ? 0.55 : 1 }}>
      <CueBadge type={cue.type} />
      <span>{cue.summary}</span>
      <span style={{ fontSize: 10, opacity: 0.4, marginLeft: "auto" }}>{cue.time}</span>
    </div>
  );
}
