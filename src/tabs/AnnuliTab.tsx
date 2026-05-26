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
import { emit } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { IconRefresh, IconConfig } from "../icons";

function gotoConfig() {
  emit("mori-nav", { tab: "config", subTab: "annuli" }).catch((e) =>
    console.error("[annuli-tab] navigate failed", e),
  );
}

type AnnuliStatus = {
  configured: boolean;
  reachable: boolean;
  endpoint: string | null;
  spirit: string | null;
  user_id: string | null;
  soul_token_configured: boolean;
  error: string | null;
};

type SupervisorInfo = {
  state: string;
  annuli_root: string | null;
  python: string | null;
  port: number | null;
  reason: string;
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
  const { t } = useTranslation();
  const [status, setStatus] = useState<AnnuliStatus | null>(null);
  const [supervisor, setSupervisor] = useState<SupervisorInfo | null>(null);
  const [soul, setSoul] = useState<string | null>(null);
  const [sections, setSections] = useState<MemorySection[]>([]);
  const [events, setEvents] = useState<AnnuliEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [controlBusy, setControlBusy] = useState<"stop" | "restart" | null>(null);
  const [controlMsg, setControlMsg] = useState<string | null>(null);
  const [sleepBusy, setSleepBusy] = useState(false);
  const [sleepResult, setSleepResult] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  // 一鍵啟用相關:`runtimeInstalled` 偵測 `~/mori-universe/annuli/.venv` 在不在;
  // 在 → unconfigured 狀態下顯示快速啟用按鈕。`enableBusy` / `enableMsg` 給按鈕回饋。
  const [runtimeInstalled, setRuntimeInstalled] = useState<boolean | null>(null);
  const [enableBusy, setEnableBusy] = useState(false);
  const [enableMsg, setEnableMsg] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const st = await invoke<AnnuliStatus>("annuli_status");
      setStatus(st);
      invoke<SupervisorInfo>("annuli_supervisor_status").then(setSupervisor).catch(() => {});
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
    // 偵測 runtime 是否已裝(同 mount 一次,enable 後再 refresh status 帶到)
    invoke<boolean>("annuli_runtime_installed")
      .then(setRuntimeInstalled)
      .catch(() => setRuntimeInstalled(false));
    // Wave 4 step 11:30s 自動刷新 status / events 數,給 background polling 感
    const id = setInterval(() => {
      // 只 refresh status / events,不重 fetch SOUL / sections(那兩個基本不變)
      invoke<AnnuliStatus>("annuli_status").then(setStatus).catch(() => {});
      invoke<SupervisorInfo>("annuli_supervisor_status").then(setSupervisor).catch(() => {});
      invoke<AnnuliEvent[]>("annuli_list_events_today").then(setEvents).catch(() => {});
    }, 30_000);
    return () => clearInterval(id);
  }, [refresh]);

  const quickEnable = async () => {
    setEnableBusy(true);
    setEnableMsg(null);
    try {
      const out = await invoke<string>("annuli_quick_enable");
      setEnableMsg(out);
      // 等 supervisor 跑(spawn + health-check 最多 15s),再 refresh
      setTimeout(() => {
        refresh();
        invoke<SupervisorInfo>("annuli_supervisor_status").then(setSupervisor).catch(() => {});
        invoke<boolean>("annuli_runtime_installed").then(setRuntimeInstalled).catch(() => {});
      }, 2000);
    } catch (e) {
      setEnableMsg(`❌ ${String(e)}`);
    } finally {
      setEnableBusy(false);
    }
  };

  const runControl = async (kind: "stop" | "restart") => {
    setControlBusy(kind);
    setControlMsg(null);
    try {
      const command = kind === "stop" ? "annuli_supervisor_stop" : "annuli_supervisor_resync_restart";
      const out = await invoke<string>(command);
      setControlMsg(out);
      setTimeout(() => {
        refresh();
        invoke<SupervisorInfo>("annuli_supervisor_status").then(setSupervisor).catch(() => {});
      }, kind === "restart" ? 2000 : 300);
    } catch (e) {
      setControlMsg(`Error: ${String(e)}`);
      invoke<SupervisorInfo>("annuli_supervisor_status").then(setSupervisor).catch(() => {});
    } finally {
      setControlBusy(null);
    }
  };

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
    return <div className="mori-tab mori-annuli-loading">{t("annuli_tab.loading_status")}</div>;
  }

  if (!status.configured) {
    return (
      <div className="mori-tab">
        <h1 className="mori-tab-title">{t("annuli_tab.title_unconfigured")}</h1>
        <p className="mori-tab-hint">{t("annuli_tab.unconfigured_hint")}</p>
        <div className="mori-annuli-sleep-row" style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
          {runtimeInstalled && (
            <button
              className="mori-btn primary"
              onClick={quickEnable}
              disabled={enableBusy}
              title="偵測到 ~/mori-universe/annuli runtime 已裝。按此自動寫入 annuli.enabled=true + 預設 endpoint/spirit/user_id + 啟動 supervisor"
            >
              {enableBusy ? "啟用中…" : "✨ 一鍵啟用 Annuli(runtime 已偵測到)"}
            </button>
          )}
          <button className="mori-btn" onClick={gotoConfig}>
            <IconConfig /> {t("annuli_tab.go_config")}
          </button>
          {runtimeInstalled === false && (
            <span style={{ fontSize: 12, color: "var(--c-text-muted)" }}>
              提示:Annuli runtime 未偵測到(`~/mori-universe/annuli/.venv` 不存在)。先到 Deps tab 裝「Annuli 反思服務 runtime」。
            </span>
          )}
          {enableMsg && (
            <span
              style={{
                fontSize: 12,
                color: enableMsg.startsWith("❌") ? "var(--c-danger-text)" : "var(--c-success-text)",
                marginLeft: 4,
              }}
            >
              {enableMsg}
            </span>
          )}
        </div>
        <p className="mori-tab-hint">{t("annuli_tab.schema_hint")}</p>
        <pre className="mori-annuli-pre">
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
    <div className="mori-tab">
      <div className="mori-annuli-header">
        <h1 className="mori-tab-title">Annuli — {status.spirit ?? "(unknown spirit)"}</h1>
        <div className="mori-annuli-header-actions">
          <button className="mori-btn" onClick={gotoConfig} title={t("annuli_tab.configure_button")}>
            <IconConfig /> {t("annuli_tab.configure_button")}
          </button>
          <button className="mori-btn" onClick={refresh} disabled={loading} title={t("annuli_tab.refresh_button")}>
            <IconRefresh /> {loading ? t("annuli_tab.refreshing_button") : t("annuli_tab.refresh_button")}
          </button>
        </div>
      </div>

      {/* Status row */}
      <div className={`mori-annuli-status ${status.reachable ? "ok" : "bad"}`}>
        <div className="mori-annuli-status-main">
          {status.reachable ? t("annuli_tab.status_connected") : t("annuli_tab.status_unreachable")} · endpoint: <code>{status.endpoint}</code> · spirit: <code>{status.spirit}</code> · user_id: <code>{status.user_id}</code>
        </div>
        <div className="mori-annuli-status-sub">
          X-Soul-Token: {status.soul_token_configured ? t("annuli_tab.soul_token_set") : t("annuli_tab.soul_token_unset")}
        </div>
        {status.error && (
          <div className="mori-annuli-status-err">
            {t("annuli_tab.error_prefix")}{status.error}
          </div>
        )}
      </div>

      <section className="mori-annuli-section">
        <h2 className="mori-annuli-section-title">Annuli process</h2>
        <div className="mori-annuli-status">
          <div className="mori-annuli-status-main">
            supervisor: <code>{supervisor?.state ?? "loading"}</code>
            {supervisor?.port ? <> · port: <code>{supervisor.port}</code></> : null}
          </div>
          <div className="mori-annuli-status-sub">
            {supervisor?.reason ?? "checking supervisor state"}
          </div>
          {supervisor?.annuli_root && (
            <div className="mori-annuli-status-sub">
              root: <code>{supervisor.annuli_root}</code>
            </div>
          )}
          {supervisor?.state === "already-running" && (
            <div className="mori-annuli-status-err">
              This Annuli process was not started by Mori. Stop the external process first if you want Mori to restart it with the synced token.
            </div>
          )}
          <div className="mori-annuli-sleep-row">
            <button
              className="mori-btn"
              onClick={() => runControl("restart")}
              disabled={controlBusy !== null || !status.configured}
              title="Sync config/.env token, then restart Mori-managed Annuli"
            >
              {controlBusy === "restart" ? "Restarting..." : "Sync token & restart"}
            </button>
            <button
              className="mori-btn"
              onClick={() => runControl("stop")}
              disabled={controlBusy !== null}
              title="Stop only the Annuli process spawned by Mori"
            >
              {controlBusy === "stop" ? "Stopping..." : "Stop Mori-managed Annuli"}
            </button>
            {controlMsg && (
              <div className={`mori-annuli-sleep-result ${controlMsg.startsWith("Error:") ? "bad" : ""}`}>
                {controlMsg}
              </div>
            )}
          </div>
        </div>
      </section>

      {/* Sleep button */}
      <div className="mori-annuli-sleep-row">
        <button
          className="mori-btn"
          onClick={triggerSleep}
          disabled={sleepBusy || !status.reachable}
        >
          {sleepBusy ? t("annuli_tab.sleep_busy") : t("annuli_tab.sleep_button")}
        </button>
        {sleepResult && (
          <div className="mori-annuli-sleep-result">
            {sleepResult}
          </div>
        )}
      </div>

      {/* SOUL.md */}
      <section className="mori-annuli-section">
        <h2 className="mori-annuli-section-title">{t("annuli_tab.soul_section")}</h2>
        <pre className="mori-annuli-pre mori-annuli-soul">
          {soul ?? t("annuli_tab.soul_loading")}
        </pre>
      </section>

      {/* MEMORY § sections */}
      <section className="mori-annuli-section">
        <h2 className="mori-annuli-section-title">{t("annuli_tab.memory_section")} ({sections.length})</h2>
        {sections.length === 0 ? (
          <p className="mori-annuli-empty">{t("annuli_tab.memory_empty")}</p>
        ) : (
          <ul className="mori-annuli-list">
            {sections.map((s) => (
              <li key={s.index} className="mori-annuli-mem-item">
                <button
                  className="mori-annuli-mem-header"
                  onClick={() => toggleSection(s.index)}
                >
                  {expanded.has(s.index) ? "▼" : "▶"} § {s.header}
                </button>
                {expanded.has(s.index) && s.body && (
                  <pre className="mori-annuli-pre mori-annuli-mem-body">
                    {s.body}
                  </pre>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* Today's events */}
      <section className="mori-annuli-section">
        <h2 className="mori-annuli-section-title">{t("annuli_tab.events_section")} ({events.length})</h2>
        {events.length === 0 ? (
          <p className="mori-annuli-empty">{t("annuli_tab.events_empty")}</p>
        ) : (
          <ul className="mori-annuli-events">
            {events.map((e, i) => (
              <li key={i} className="mori-annuli-event-row">
                <code className="mori-annuli-event-ts">{formatTime(e.ts)}</code>
                <code className="mori-annuli-event-kind">{e.kind}</code>
                <span className="mori-annuli-event-data">{previewData(e.data_json)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
