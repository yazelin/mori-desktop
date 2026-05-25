import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Select } from "../Select";

type VerifyProfile = "none" | "quick" | "full";


type DepInfo = {
  id: string;
  label: string;
  installed: boolean;
  install_hint: string;
};

type DevTask = {
  id: string;
  prompt: string;
  created_at_ms: number;
  status: string;
  verify_profile: VerifyProfile;
  finished_at_ms?: number | null;
};

type DevCapability = {
  allow_verify: boolean;
};

type DevPrDraft = {
  task_id: string;
  title: string;
  body: string;
};

type DevTaskStats = {
  total: number;
  queued: number;
  planning: number;
  executing: number;
  succeeded: number;
  failed: number;
  aborted: number;
};

type DevTaskSnapshot = {
  task: DevTask;
  report: DevReport | null;
};

type DevReport = {
  task_id: string;
  summary: string;
  changed_files: string[];
  verify_command?: string | null;
  verify_ok?: boolean | null;
  verify_output?: string | null;
  replay_log?: string[];
  quality_score?: number;
};

const fmtTime = (ms?: number | null) => (ms ? new Date(ms).toLocaleString() : "-");
const depDisplayLabel = (dep: DepInfo) => dep.label || dep.id;
const depInstallHint = (dep: DepInfo) => dep.install_hint || "sudo bash scripts/install-linux-deps.sh";

export default function SelfDevTab() {
  const { t } = useTranslation();
  const [prompt, setPrompt] = useState("");
  const [verify, setVerify] = useState<VerifyProfile>("none");
  const [tasks, setTasks] = useState<DevTask[]>([]);
  const [report, setReport] = useState<DevReport | null>(null);
  const [cap, setCap] = useState<DevCapability>({ allow_verify: false });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showVerifyOutput, setShowVerifyOutput] = useState(false);
  const [confirmFull, setConfirmFull] = useState(false);
  const [statusFilter, setStatusFilter] = useState<"all" | "running" | "done">("all");
  const [query, setQuery] = useState("");
  const [autoRefresh, setAutoRefresh] = useState(false);
  const [stats, setStats] = useState<DevTaskStats | null>(null);
  const [prDraft, setPrDraft] = useState<DevPrDraft | null>(null);
  const [deps, setDeps] = useState<DepInfo[]>([]);
  const [depsLoading, setDepsLoading] = useState(false);


  const refreshDeps = async (force = false) => {
    try {
      setDepsLoading(true);
      const next = await invoke<DepInfo[]>("self_dev_preflight_deps", { force });
      setDeps(next);
    } catch (e) {
      setError(String(e));
    } finally {
      setDepsLoading(false);
    }
  };

  const refreshStats = async () => {
    try {
      setStats(await invoke<DevTaskStats>("get_dev_task_stats"));
    } catch {}
  };

  const refresh = async () => {
    try {
      setError(null);
      setTasks(await invoke<DevTask[]>("list_dev_tasks"));
      await refreshStats();
    } catch (e) {
      setError(String(e));
    }
  };
  const refreshCap = async () => {
    try {
      setError(null);
      setCap(await invoke<DevCapability>("get_dev_capability"));
    } catch (e) {
      setError(String(e));
    }
  };
  useEffect(() => {
    refresh().catch(() => {});
    refreshCap().catch(() => {});
    refreshDeps(false).catch(() => {});
  }, []);

  useEffect(() => {
    if (!autoRefresh) return;
    const id = setInterval(() => {
      refresh().catch(() => {});
    }, 2500);
    return () => clearInterval(id);
  }, [autoRefresh]);

  const start = async () => {
    if (missingDeps.length > 0) {
      setError(t("self_dev_tab.preflight_blocked", { count: missingDeps.length }));
      return;
    }
    if (verify === "full" && !confirmFull) {
      setConfirmFull(true);
      return;
    }
    setConfirmFull(false);
    setBusy(true);
    try {
      setError(null);
      await invoke("start_dev_task", { input: { prompt, verifyProfile: verify } });
      setPrompt("");
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const setVerifyCapability = async (allow: boolean) => {
    try {
      setError(null);
      await invoke("approve_dev_capability", { input: { allowVerify: allow } });
      await refreshCap();
    } catch (e) {
      setError(String(e));
    }
  };

  const clearCompleted = async () => {
    try {
      setError(null);
      await invoke("delete_completed_dev_tasks");
      setReport(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const removeTask = async (id: string) => {
    try {
      setError(null);
      await invoke("delete_dev_task", { taskId: id });
      if (report?.task_id === id) setReport(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const abort = async (id: string) => {
    try {
      setError(null);
      await invoke("abort_dev_task", { taskId: id });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const rerun = async (id: string) => {
    setBusy(true);
    try {
      setError(null);
      await invoke("rerun_dev_task", { taskId: id });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const draftPr = async (id: string) => {
    try {
      setError(null);
      const draft = await invoke<DevPrDraft | null>("draft_dev_pr", { taskId: id });
      setPrDraft(draft);
    } catch (e) {
      setError(String(e));
    }
  };

  const openReport = async (id: string) => {
    try {
      setError(null);
      const snap = await invoke<DevTaskSnapshot | null>("get_dev_task_snapshot", { taskId: id });
      setReport(snap?.report ?? null);
      setShowVerifyOutput(false);
    } catch (e) {
      setError(String(e));
    }
  };

  const filteredTasks = useMemo(() => {
    return tasks.filter((t) => {
      const isRunning = t.status === "executing" || t.status === "planning" || t.status === "queued";
      const statusOk = statusFilter === "all" || (statusFilter === "running" ? isRunning : !isRunning);
      const q = query.trim().toLowerCase();
      const queryOk = !q || t.id.toLowerCase().includes(q) || t.prompt.toLowerCase().includes(q);
      return statusOk && queryOk;
    });
  }, [tasks, statusFilter, query]);

  const statusSummary = useMemo(() => {
    const total = tasks.length;
    const running = tasks.filter((x) => x.status === "executing" || x.status === "planning" || x.status === "queued").length;
    return { total, running };
  }, [tasks]);

  const missingDeps = useMemo(() => deps.filter((d) => !d.installed), [deps]);

  const gateSummary = useMemo(() => {
    const deps = missingDeps.length === 0 ? "pass" : "fail";
    const cmd = (report?.verify_command || "").toLowerCase();
    const verifyOk = report?.verify_ok;

    if (!report || !report.verify_command) {
      return { deps, build: "unknown", core: "unknown" };
    }

    const isFull = cmd.includes("scripts/verify.sh");
    const isQuick = cmd.includes("cargo check");

    if (isFull) {
      const state = verifyOk ? "pass" : "fail";
      return { deps, build: state, core: state };
    }

    if (isQuick) {
      return { deps, build: verifyOk ? "pass" : "fail", core: "unknown" };
    }

    return { deps, build: verifyOk ? "pass" : "fail", core: "unknown" };
  }, [missingDeps.length, report]);

  const releaseReady = useMemo(() => {
    return gateSummary.deps !== "fail" && gateSummary.build === "pass" && gateSummary.core === "pass";
  }, [gateSummary]);

  const releaseStatus = releaseReady ? "ready" : "not-ready";

  const copyReleaseGateDecision = async () => {
    const text = `release-gate=${releaseStatus} | deps=${gateSummary.deps} | build=${gateSummary.build} | core=${gateSummary.core}`;
    await copySetupHint(text);
  };

  const platformName = useMemo(() => {
    const p = (navigator.platform || "").toLowerCase();
    if (p.includes("win")) return "windows";
    if (p.includes("mac")) return "macos";
    if (p.includes("linux")) return "linux";
    return "unknown";
  }, []);

  const isLinux = platformName === "linux";
  const nonLinuxSetupCmd = platformName === "windows" ? "npm ci; npm run build" : "npm ci && npm run build";

  const copySetupHint = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const copyGateSummary = async () => {
    const text = `gate: deps=${gateSummary.deps}, build=${gateSummary.build}, core=${gateSummary.core}`;
    await copySetupHint(text);
  };

  const copyPrReadySummary = async () => {
    const summary = report?.summary ?? "(no report)";
    const taskId = report?.task_id ?? "(no task)";
    const text = `task=${taskId} | summary=${summary} | gate(deps=${gateSummary.deps}, build=${gateSummary.build}, core=${gateSummary.core})`;
    await copySetupHint(text);
  };

  const copyReviewNoteTemplate = async () => {
    const taskId = report?.task_id ?? "(no task)";
    const summary = report?.summary ?? "(no report)";
    const text = [
      "[Review Note]",
      `- task: ${taskId}`,
      `- observations: ${summary}`,
      `- risks: gate deps=${gateSummary.deps}, build=${gateSummary.build}, core=${gateSummary.core}`,
      "- decision: (human reviewer fill)"
    ].join("\n");
    await copySetupHint(text);
  };

  const copyRegressionChecklist = async () => {
    const text = [
      "[E2 Regression Checklist]",
      "1. start_dev_task",
      "2. list_dev_tasks",
      "3. get_dev_task_snapshot",
      "4. rerun_dev_task",
      "5. abort_dev_task",
      "6. delete_dev_task"
    ].join("\n");
    await copySetupHint(text);
  };

  const copyRecoveryChecklist = async () => {
    const text = [
      "[E3 Recovery Checklist]",
      "1. Check preflight missing deps",
      "2. Install deps and Recheck deps",
      "3. Run quick verify, then full verify if needed",
      "4. Refresh gate summary and review note",
      "5. If still failing, stop at human review (no merge)"
    ].join("\n");
    await copySetupHint(text);
  };

  const copyLinuxInstallCmd = async () => {
    const cmd = "sudo bash scripts/install-linux-deps.sh";
    try {
      await copySetupHint(cmd);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section className="mori-tab mori-self-dev-tab">
      <h2 className="mori-tab-title">{t("self_dev_tab.title")}</h2>
      <p className="mori-tab-hint">{t("self_dev_tab.hint")}</p>
      <p className="mori-self-dev-hint">{t("self_dev_tab.summary", { total: statusSummary.total, running: statusSummary.running })}</p>
      {stats && (
        <p className="mori-self-dev-hint">
          q:{stats.queued} p:{stats.planning} x:{stats.executing} ok:{stats.succeeded} fail:{stats.failed} abort:{stats.aborted}
        </p>
      )}

      <div className="mori-self-dev-section">
        <div className="mori-self-dev-section-header">
          <div className="mori-self-dev-section-title">
            <strong>{t("self_dev_tab.preflight_title")}</strong>
            <span className="mori-self-dev-hint">{t("self_dev_tab.preflight_platform", { platform: platformName })}</span>
          </div>
          <button className="mori-btn" onClick={() => refreshDeps(true)}>{depsLoading ? t("self_dev_tab.preflight_checking") : t("self_dev_tab.preflight_refresh")}</button>
        </div>
        {missingDeps.length === 0 ? (
          <div className="mori-self-dev-ok">
            <p className="mori-self-dev-hint">{t("self_dev_tab.preflight_ok")}</p>
            <p className="mori-self-dev-hint">{t("self_dev_tab.preflight_next_step_ok")}</p>
          </div>
        ) : (
          <div className="mori-warning" role="alert">
            <p>{t("self_dev_tab.preflight_missing", { count: missingDeps.length })}</p>
            <ul>
              {missingDeps.slice(0, 6).map((d) => (
                <li key={d.id}>
                  <strong>{depDisplayLabel(d)}</strong> — <code>{depInstallHint(d)}</code>
                </li>
              ))}
            </ul>
            <p className="mori-self-dev-hint">{t("self_dev_tab.preflight_note")}</p>
            <p className="mori-self-dev-hint">{t("self_dev_tab.preflight_next_step_fix")}</p>
            {isLinux ? (
              <div className="mori-self-dev-actions">
                <code className="mori-self-dev-command">sudo bash scripts/install-linux-deps.sh</code>
                <button className="mori-btn" onClick={copyLinuxInstallCmd}>{t("self_dev_tab.copy_linux_install")}</button>
              </div>
            ) : (
              <div>
                <p className="mori-self-dev-hint">{t("self_dev_tab.non_linux_setup_hint")}</p>
                <div className="mori-self-dev-actions">
                  <code className="mori-self-dev-command">{nonLinuxSetupCmd}</code>
                  <button
                    className="mori-btn"
                    onClick={() => copySetupHint(nonLinuxSetupCmd)}
                  >
                    {t("self_dev_tab.copy_non_linux_hint")}
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
      {error && <div className="mori-error" role="alert">{t("self_dev_tab.error_prefix")}{error}</div>}

      <div className="mori-self-dev-section">
        <strong>{t("self_dev_tab.gate_title")}</strong>
        <p className="mori-self-dev-hint">deps: {gateSummary.deps} · build: {gateSummary.build} · core: {gateSummary.core}</p>
        <p className="mori-self-dev-hint">{t("self_dev_tab.gate_hint")}</p>
        <p className="mori-self-dev-hint">{t("self_dev_tab.release_gate_status", { status: releaseStatus })}</p>
        <div className="mori-self-dev-actions">
          <button className="mori-btn" onClick={copyGateSummary}>{t("self_dev_tab.copy_gate_summary")}</button>
          <button className="mori-btn" onClick={copyPrReadySummary}>{t("self_dev_tab.copy_pr_ready_summary")}</button>
          <button className="mori-btn" onClick={copyReviewNoteTemplate}>{t("self_dev_tab.copy_review_template")}</button>
          <button className="mori-btn" onClick={copyRegressionChecklist}>{t("self_dev_tab.copy_regression_checklist")}</button>
          <button className="mori-btn" onClick={copyRecoveryChecklist}>{t("self_dev_tab.copy_recovery_checklist")}</button>
          <button className="mori-btn" onClick={copyReleaseGateDecision}>{t("self_dev_tab.copy_release_gate")}</button>
        </div>
      </div>

      <div className="mori-self-dev-controls">
        <input className="mori-input" value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder={t("self_dev_tab.prompt_placeholder")} />
        <Select
          className="mori-self-dev-select"
          value={verify}
          onChange={(value) => {
            setVerify(value as VerifyProfile);
            setConfirmFull(false);
          }}
          options={[
            { value: "none", label: t("self_dev_tab.verify_none") },
            { value: "quick", label: t("self_dev_tab.verify_quick") },
            { value: "full", label: t("self_dev_tab.verify_full") },
          ]}
        />
        <button className="mori-btn" onClick={start} disabled={!prompt.trim()}>{busy ? t("self_dev_tab.busy") : t("self_dev_tab.start")}</button>
        <button className="mori-btn" onClick={refresh}>{t("self_dev_tab.refresh")}</button>
        <button className="mori-btn" onClick={() => setAutoRefresh((v) => !v)}>
          {autoRefresh ? t("self_dev_tab.auto_refresh_off") : t("self_dev_tab.auto_refresh_on")}
        </button>
        <button className="mori-btn danger" onClick={clearCompleted}>{t("self_dev_tab.clear_completed")}</button>
      </div>

      {confirmFull && (
        <div className="mori-warning" role="alert" style={{ marginBottom: 12 }}>
          {t("self_dev_tab.full_verify_confirm")}
        </div>
      )}

      <div className="mori-self-dev-actions mori-self-dev-capability">
        <span>{t("self_dev_tab.verify_cap")}: <strong>{cap.allow_verify ? t("self_dev_tab.enabled") : t("self_dev_tab.disabled")}</strong></span>
        <button className="mori-btn" onClick={() => setVerifyCapability(true)}>{t("self_dev_tab.enable_verify")}</button>
        <button className="mori-btn" onClick={() => setVerifyCapability(false)}>{t("self_dev_tab.disable_verify")}</button>
      </div>


      <div className="mori-self-dev-controls">
        <input
          className="mori-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("self_dev_tab.search_placeholder")}
        />
        <Select
          className="mori-self-dev-select"
          value={statusFilter}
          onChange={(value) => setStatusFilter(value as "all" | "running" | "done")}
          options={[
            { value: "all", label: t("self_dev_tab.filter_all") },
            { value: "running", label: t("self_dev_tab.filter_running") },
            { value: "done", label: t("self_dev_tab.filter_done") },
          ]}
        />
      </div>

      <div className="mori-list">
        {filteredTasks.map((task) => (
          <div key={task.id} className="mori-item mori-self-dev-task">
            <div className="mori-self-dev-task-body">
              <strong>{task.id}</strong> · {task.status} · {task.verify_profile}
              <div style={{ opacity: 0.75 }}>{task.prompt}</div>
              <div style={{ opacity: 0.7, fontSize: 12 }}>
                {t("self_dev_tab.created_at")}: {fmtTime(task.created_at_ms)} · {t("self_dev_tab.finished_at")}: {fmtTime(task.finished_at_ms)}
              </div>
            </div>
            <div className="mori-self-dev-actions">
              <button className="mori-btn" onClick={() => openReport(task.id)}>{t("self_dev_tab.report")}</button>
              <button className="mori-btn" onClick={() => rerun(task.id)}>{t("self_dev_tab.rerun")}</button>
              <button className="mori-btn" onClick={() => abort(task.id)}>{t("self_dev_tab.abort")}</button>
              <button className="mori-btn" onClick={() => draftPr(task.id)}>{t("self_dev_tab.draft_pr")}</button>
              <button className="mori-btn danger" onClick={() => removeTask(task.id)}>{t("self_dev_tab.delete")}</button>
            </div>
          </div>
        ))}
      </div>

      {prDraft && (
        <section className="mori-self-dev-output">
          <h3 className="mori-self-dev-output-title">{t("self_dev_tab.pr_draft")}</h3>
          <div><strong>{prDraft.title}</strong></div>
          <pre className="mori-code">{prDraft.body}</pre>
        </section>
      )}

      {report && (
        <section className="mori-self-dev-output">
          <h3 className="mori-self-dev-output-title">{t("self_dev_tab.report")}</h3>
          <div className="mori-self-dev-hint">{report.summary}</div>
          <div className="mori-self-dev-hint">{t("self_dev_tab.changed_files")}: {report.changed_files.length}</div>
          <ul>
            {report.changed_files.map((f) => <li key={f}><code>{f}</code></li>)}
          </ul>
          {report.verify_command && (
            <div className="mori-self-dev-hint">
              {t("self_dev_tab.verify_command")}: <code>{report.verify_command}</code> · {report.verify_ok ? t("self_dev_tab.verify_pass") : t("self_dev_tab.verify_fail")}
            </div>
          )}
          <div className="mori-self-dev-hint">Quality score: {report.quality_score ?? "-"}</div>
          {report.replay_log && report.replay_log.length > 0 && (
            <details>
              <summary>Replay log ({report.replay_log.length})</summary>
              <pre className="mori-code">{report.replay_log.join("\n")}</pre>
            </details>
          )}
          {report.verify_output && (
            <div>
              <button className="mori-btn" onClick={() => setShowVerifyOutput((v) => !v)}>
                {showVerifyOutput ? t("self_dev_tab.hide_verify_output") : t("self_dev_tab.show_verify_output")}
              </button>
              {showVerifyOutput && <pre className="mori-code">{report.verify_output}</pre>}
            </div>
          )}
        </section>
      )}
    </section>
  );
}
