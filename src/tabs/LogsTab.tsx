// v0.4.0 Phase A 觀測:LogsTab — 撈 ~/.mori/logs/mori-YYYY-MM-DD.jsonl 顯示。
//
// 簡單表格 + date 選擇 + kind/provider filter pill + refresh。
// 後端 (event_log) append-only,前端只讀 tail-N。
// Phase B(per-pipeline artifacts)+ Phase C(installed apps catalog)留下版。

import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { IconRefresh } from "../icons";
import { Select } from "../Select";

type LogEntry = {
  ts?: string;
  kind?: string;
  provider?: string;
  model?: string;
  binary?: string;
  latency_ms?: number;
  ok?: boolean;
  error?: string;
  output_chars?: number;
  [k: string]: unknown;
};

const KIND_FILTERS = ["all", "llm_call", "spawn_error", "skill_dispatch", "transcribe", "error"];

function formatTs(ts: string | undefined): string {
  if (!ts) return "";
  try {
    const d = new Date(ts);
    return d.toLocaleTimeString("zh-TW", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    });
  } catch {
    return ts;
  }
}

function formatLatency(ms: number | undefined): string {
  if (ms == null) return "";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

function LogsTab() {
  const { t } = useTranslation();
  const [date, setDate] = useState<string>("");
  const [availableDates, setAvailableDates] = useState<string[]>([]);
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [kindFilter, setKindFilter] = useState<string>("all");
  const [providerFilter, setProviderFilter] = useState<string>("all");
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const loadDates = useCallback(async () => {
    try {
      const dates = await invoke<string[]>("log_dates");
      setAvailableDates(dates);
      if (dates.length > 0 && !date) {
        setDate(dates[0]); // newest = first
      }
    } catch (e) {
      console.error("[logs] log_dates failed", e);
    }
  }, [date]);

  const loadEntries = useCallback(async () => {
    if (!date) {
      setEntries([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const list = await invoke<LogEntry[]>("log_tail", { date, limit: 500 });
      setEntries(list);
    } catch (e) {
      console.error("[logs] log_tail failed", e);
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }, [date]);

  useEffect(() => {
    loadDates();
  }, [loadDates]);

  useEffect(() => {
    if (date) loadEntries();
  }, [date, loadEntries]);

  // 從 entries 萃出實際出現過的 provider,給 filter dropdown 用
  const providers = useMemo(() => {
    const set = new Set<string>();
    entries.forEach((e) => {
      if (typeof e.provider === "string" && e.provider) set.add(e.provider);
    });
    return ["all", ...Array.from(set).sort()];
  }, [entries]);

  const filtered = useMemo(() => {
    return entries.filter((e) => {
      if (kindFilter !== "all" && e.kind !== kindFilter) return false;
      if (providerFilter !== "all" && e.provider !== providerFilter) return false;
      return true;
    });
  }, [entries, kindFilter, providerFilter]);

  const refresh = () => {
    loadDates();
    loadEntries();
  };

  const toggleExpand = (idx: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  };

  return (
    <div className="mori-tab mori-tab-logs">
      <h2 className="mori-tab-title">{t("logs_tab.title")}</h2>
      <p className="mori-tab-hint">{t("logs_tab.hint")}</p>

      <div className="mori-logs-toolbar">
        <label className="mori-logs-toolbar-field">
          <span>{t("logs_tab.date_label")}</span>
          <Select
            value={date}
            onChange={setDate}
            options={availableDates.length > 0
              ? availableDates.map((d) => ({ value: d, label: d }))
              : [{ value: "", label: t("logs_tab.no_logs") }]}
          />
        </label>
        <label className="mori-logs-toolbar-field">
          <span>{t("logs_tab.kind_label")}</span>
          <Select
            value={kindFilter}
            onChange={setKindFilter}
            options={KIND_FILTERS.map((k) => ({ value: k, label: k }))}
          />
        </label>
        <label className="mori-logs-toolbar-field">
          <span>{t("logs_tab.provider_label")}</span>
          <Select
            value={providerFilter}
            onChange={setProviderFilter}
            options={providers.map((p) => ({ value: p, label: p }))}
          />
        </label>
        <button className="mori-btn" onClick={refresh} title={t("logs_tab.refresh_title")}>
          <IconRefresh width={13} height={13} />
        </button>
        <span className="mori-logs-count">
          {filtered.length} / {entries.length}
        </span>
      </div>

      {loading ? (
        <div className="mori-tab-placeholder"><p>{t("logs_tab.loading")}</p></div>
      ) : filtered.length === 0 ? (
        <div className="mori-tab-placeholder">
          <p>{availableDates.length === 0 ? t("logs_tab.empty_title") : t("logs_tab.no_match")}</p>
          <p>{t("logs_tab.empty_hint")}</p>
        </div>
      ) : (
        <div className="mori-logs-list">
          {filtered.map((e, idx) => {
            const isOk = e.ok !== false; // undefined treated as ok
            const isExpanded = expanded.has(idx);
            return (
              <div
                key={idx}
                className={`mori-logs-row ${isOk ? "" : "err"}`}
                onClick={() => toggleExpand(idx)}
              >
                <div className="mori-logs-row-head">
                  <span className="mori-logs-ts">{formatTs(e.ts)}</span>
                  <span className="mori-logs-kind">{e.kind ?? "—"}</span>
                  {e.provider && <span className="mori-logs-provider">{e.provider}</span>}
                  {e.model && <span className="mori-logs-model">{e.model}</span>}
                  {e.latency_ms != null && (
                    <span className="mori-logs-latency">{formatLatency(e.latency_ms)}</span>
                  )}
                  <span className={`mori-logs-status ${isOk ? "ok" : "err"}`}>
                    {isOk ? "✓" : "✗"}
                  </span>
                </div>
                {e.error && (
                  <div className="mori-logs-error">{e.error}</div>
                )}
                {isExpanded && (
                  <pre className="mori-logs-json">{JSON.stringify(e, null, 2)}</pre>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

export default LogsTab;
