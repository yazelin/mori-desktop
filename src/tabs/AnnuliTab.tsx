// Wave 4 step 10:AnnuliTab — Mori vault 狀態 browser
//
// 唯讀 MVP:
// - status bar:annuli endpoint reachable + soul_token_configured + spirit / user_id
// - SOUL.md preview(monospace pre block)
// - MEMORY.md § sections list(header only,點開展開 body)
// - 今日 events 列表(時序 + JSON data preview)
// - /sleep 按鈕(觸發 trigger_sleep)
// - 重新整理按鈕
//
// Wave 5+ 加:
// - in-UI edit SOUL.md(需 X-Soul-Token)
// - curator dry-run review / apply 介面
// - 跨 spirit switcher

import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconRefresh } from "../icons";

type AnnuliStatus = {
  configured: boolean;
  reachable: boolean;
  endpoint: string | null;
  spirit: string | null;
  user_id: string | null;
  soul_token_configured: boolean;
  error: string | null;
};

type MemorySection = {
  header: string;
  index: number;
  body: string | null;
};

type AnnuliEvent = {
  ts: string;
  kind: string;
  user_id: string;
  source: string;
  data_json: string;
};

function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  } catch {
    return iso;
  }
}

function previewData(json: string, max = 80): string {
  try {
    const parsed = JSON.parse(json);
    const text = (parsed.text || parsed.message || JSON.stringify(parsed)) as string;
    return text.length > max ? text.slice(0, max) + "…" : text;
  } catch {
    return json.slice(0, max);
  }
}

export default function AnnuliTab() {
  const [status, setStatus] = useState<AnnuliStatus | null>(null);
  const [soul, setSoul] = useState<string | null>(null);
  const [sections, setSections] = useState<MemorySection[]>([]);
  const [events, setEvents] = useState<AnnuliEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [sleepBusy, setSleepBusy] = useState(false);
  const [sleepResult, setSleepResult] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const st = await invoke<AnnuliStatus>("annuli_status");
      setStatus(st);
      if (!st.configured || !st.reachable) {
        setSoul(null);
        setSections([]);
        setEvents([]);
        return;
      }
      try {
        setSoul(await invoke<string>("annuli_get_soul"));
      } catch (e) {
        setSoul(`(讀 SOUL.md 失敗:${e})`);
      }
      try {
        setSections(await invoke<MemorySection[]>("annuli_list_memory", { includeBody: true }));
      } catch (e) {
        console.error("[annuli-tab] list_memory failed", e);
        setSections([]);
      }
      try {
        setEvents(await invoke<AnnuliEvent[]>("annuli_list_events_today"));
      } catch (e) {
        console.error("[annuli-tab] list_events_today failed", e);
        setEvents([]);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    // Wave 4 step 11:30s 自動刷新 status / events 數,給 background polling 感
    const id = setInterval(() => {
      // 只 refresh status / events,不重 fetch SOUL / sections(那兩個基本不變)
      invoke<AnnuliStatus>("annuli_status").then(setStatus).catch(() => {});
      invoke<AnnuliEvent[]>("annuli_list_events_today").then(setEvents).catch(() => {});
    }, 30_000);
    return () => clearInterval(id);
  }, [refresh]);

  const triggerSleep = async () => {
    setSleepBusy(true);
    setSleepResult(null);
    try {
      const ringPath = await invoke<string>("annuli_trigger_sleep");
      setSleepResult(`✅ ring 寫好:${ringPath}`);
      await refresh();
    } catch (e) {
      setSleepResult(`❌ ${e}`);
    } finally {
      setSleepBusy(false);
    }
  };

  const toggleSection = (idx: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) {
        next.delete(idx);
      } else {
        next.add(idx);
      }
      return next;
    });
  };

  if (!status) {
    return <div style={{ padding: "1.5rem" }}>載入 Annuli 狀態中…</div>;
  }

  if (!status.configured) {
    return (
      <div style={{ padding: "1.5rem", maxWidth: "60rem" }}>
        <h1>Annuli</h1>
        <p style={{ marginTop: "1rem", color: "var(--text-muted)" }}>
          Annuli 整合**沒接**(`~/.mori/config.json` 的 `annuli.enabled` 沒設或缺欄位)。
        </p>
        <pre
          style={{
            background: "var(--surface-alt, #1a1a2e)",
            padding: "1rem",
            borderRadius: "0.5rem",
            overflow: "auto",
            fontSize: "0.85rem",
          }}
        >
{`{
  "annuli": {
    "enabled": true,
    "endpoint": "http://localhost:5000",
    "spirit_name": "mori",
    "user_id": "yazelin",
    "soul_token": "<隨機 hex 字串>",
    "basic_auth": { "user": "ct", "pass": "..." }
  }
}`}
        </pre>
      </div>
    );
  }

  return (
    <div style={{ padding: "1.5rem", maxWidth: "60rem" }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: "1rem" }}>
        <h1 style={{ margin: 0 }}>Annuli — {status.spirit ?? "(unknown spirit)"}</h1>
        <button onClick={refresh} disabled={loading} title="重新整理">
          <IconRefresh /> {loading ? "載入中…" : "重新整理"}
        </button>
      </div>

      {/* Status row */}
      <div
        style={{
          padding: "0.75rem 1rem",
          marginBottom: "1rem",
          borderRadius: "0.5rem",
          background: status.reachable ? "rgba(0,180,80,0.1)" : "rgba(220,80,80,0.1)",
          border: `1px solid ${status.reachable ? "rgba(0,180,80,0.3)" : "rgba(220,80,80,0.3)"}`,
        }}
      >
        <div style={{ fontSize: "0.9rem" }}>
          {status.reachable ? "🟢 connected" : "🔴 unreachable"} · endpoint: <code>{status.endpoint}</code> · spirit: <code>{status.spirit}</code> · user_id: <code>{status.user_id}</code>
        </div>
        <div style={{ fontSize: "0.8rem", color: "var(--text-muted)", marginTop: "0.25rem" }}>
          X-Soul-Token: {status.soul_token_configured ? "✅ 設好(可 PUT /soul)" : "⚠️ 沒設(PUT /soul 一律 403)"}
        </div>
        {status.error && (
          <div style={{ marginTop: "0.5rem", fontSize: "0.8rem", color: "var(--error, #cc6666)" }}>
            錯誤:{status.error}
          </div>
        )}
      </div>

      {/* Sleep button */}
      <div style={{ marginBottom: "1.5rem" }}>
        <button
          onClick={triggerSleep}
          disabled={sleepBusy || !status.reachable}
          style={{ padding: "0.5rem 1rem", fontSize: "0.95rem" }}
        >
          {sleepBusy ? "🌙 reflecting…(LLM 寫 ring 中)" : "🌙 /sleep — 寫一輪反思年輪"}
        </button>
        {sleepResult && (
          <div style={{ marginTop: "0.5rem", fontSize: "0.85rem" }}>
            {sleepResult}
          </div>
        )}
      </div>

      {/* SOUL.md */}
      <section style={{ marginBottom: "2rem" }}>
        <h2>SOUL.md</h2>
        <pre
          style={{
            background: "var(--surface-alt, #1a1a2e)",
            padding: "1rem",
            borderRadius: "0.5rem",
            overflow: "auto",
            maxHeight: "20rem",
            fontSize: "0.85rem",
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
          }}
        >
          {soul ?? "(載入中)"}
        </pre>
      </section>

      {/* MEMORY § sections */}
      <section style={{ marginBottom: "2rem" }}>
        <h2>MEMORY.md sections ({sections.length})</h2>
        {sections.length === 0 ? (
          <p style={{ color: "var(--text-muted)" }}>(沒有 § sections 或讀取失敗)</p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0 }}>
            {sections.map((s) => (
              <li
                key={s.index}
                style={{
                  marginBottom: "0.5rem",
                  border: "1px solid var(--border, #333)",
                  borderRadius: "0.5rem",
                  overflow: "hidden",
                }}
              >
                <button
                  onClick={() => toggleSection(s.index)}
                  style={{
                    width: "100%",
                    padding: "0.5rem 1rem",
                    textAlign: "left",
                    background: "transparent",
                    border: "none",
                    color: "inherit",
                    cursor: "pointer",
                  }}
                >
                  {expanded.has(s.index) ? "▼" : "▶"} § {s.header}
                </button>
                {expanded.has(s.index) && s.body && (
                  <pre
                    style={{
                      padding: "0.75rem 1rem",
                      margin: 0,
                      fontSize: "0.85rem",
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                      background: "var(--surface-alt, #1a1a2e)",
                    }}
                  >
                    {s.body}
                  </pre>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* Today's events */}
      <section style={{ marginBottom: "2rem" }}>
        <h2>今日 events ({events.length})</h2>
        {events.length === 0 ? (
          <p style={{ color: "var(--text-muted)" }}>(今天還沒事件,或對話沒接 annuli)</p>
        ) : (
          <ul style={{ listStyle: "none", padding: 0, fontSize: "0.85rem" }}>
            {events.map((e, i) => (
              <li
                key={i}
                style={{
                  display: "flex",
                  padding: "0.4rem 0.5rem",
                  borderBottom: "1px solid var(--border, #333)",
                  gap: "0.75rem",
                }}
              >
                <code style={{ minWidth: "5rem", color: "var(--text-muted)" }}>{formatTime(e.ts)}</code>
                <code style={{ minWidth: "4rem", color: "var(--text-muted)" }}>{e.kind}</code>
                <span style={{ flex: 1, wordBreak: "break-word" }}>{previewData(e.data_json)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
