import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type Decision = "allow" | "deny" | "ask";

interface PolicyRule {
  risk: string;
  decision: Decision;
}
interface AuditEntry {
  timestamp: string;
  request_id: string;
  session_id?: string | null;
  source: string;
  tool: string;
  risk: string;
  decision: Decision;
  reason?: string | null;
}
interface BrokerResponse {
  request_id: string;
  decision: Decision;
}

// demo:三條 canned 請求,送過 broker 示範 allow/ask/deny 三路徑。
const DEMO_REQUESTS: { label: string; risk: string }[] = [
  { label: "read.public", risk: "read.public" },
  { label: "read.project", risk: "read.project" },
  { label: "exec.destructive", risk: "exec.destructive" },
];

export default function PermissionsTab() {
  const { t } = useTranslation();
  const [policy, setPolicy] = useState<PolicyRule[]>([]);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [lastDecision, setLastDecision] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setPolicy(await invoke<PolicyRule[]>("permission_policy_list"));
      setAudit(await invoke<AuditEntry[]>("permission_audit_list", { limit: 50 }));
      setErr(null);
    } catch (e: any) {
      setErr(String(e));
    }
  };
  useEffect(() => { refresh(); }, []);

  const fireDemo = async (risk: string) => {
    try {
      const resp = await invoke<BrokerResponse>("permission_decide", {
        request: {
          schema_version: 1,
          request_id: `demo_${Date.now()}`,
          source: "permissions.tab.demo",
          tool: "demo.tool",
          risk,
          reason: "PermissionsTab demo",
        },
      });
      setLastDecision(`${risk} → ${resp.decision}`);
      await refresh();
    } catch (e: any) {
      setLastDecision(null); // 別讓上一次成功的決策訊息跟錯誤橫幅並存
      setErr(String(e));
    }
  };

  return (
    <div className="mori-tab">
      <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 6 }}>
        <h2 className="mori-tab-title" style={{ marginBottom: 0 }}>{t("permissions_tab.title")}</h2>
        <button className="mori-btn small ghost" onClick={refresh}>{t("permissions_tab.refresh")}</button>
      </div>
      <p className="mori-tab-hint">
        {t("permissions_tab.hint")} (<code>~/.mori/permission-audit.jsonl</code>)
      </p>
      {err && <div className="mori-tab-error" style={{ fontSize: 12, marginBottom: 12 }}>❌ {err}</div>}

      <h3 style={{ marginBottom: 6 }}>{t("permissions_tab.policy_title")}</h3>
      <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 16 }}>
        {policy.map((r) => (
          <div key={r.risk} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
            <code style={{ minWidth: 150 }}>{r.risk}</code>
            <DecisionBadge decision={r.decision} />
          </div>
        ))}
      </div>

      <h3 style={{ marginBottom: 6 }}>{t("permissions_tab.demo_title")}</h3>
      <p style={{ opacity: 0.6, fontSize: 11 }}>{t("permissions_tab.demo_hint")}</p>
      <div style={{ display: "flex", gap: 8, marginBottom: 6, flexWrap: "wrap" }}>
        {DEMO_REQUESTS.map((d) => (
          <button key={d.risk} className="mori-btn small" onClick={() => fireDemo(d.risk)}>
            {d.label}
          </button>
        ))}
      </div>
      {lastDecision && <div style={{ fontSize: 12, opacity: 0.8, marginBottom: 16 }}>{lastDecision}</div>}

      <h3 style={{ marginBottom: 6 }}>{t("permissions_tab.audit_title")}</h3>
      {audit.length === 0 && !err && <div style={{ opacity: 0.6 }}>{t("permissions_tab.audit_empty")}</div>}
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {audit.map((a, i) => (
          <div key={`${a.request_id}_${i}`} style={{ border: "1px solid var(--c-border)", borderRadius: 8, padding: 10 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <DecisionBadge decision={a.decision} />
              <code style={{ fontSize: 12 }}>{a.risk}</code>
              <span style={{ fontSize: 12, opacity: 0.7 }}>{a.tool}</span>
            </div>
            <div style={{ fontSize: 11, opacity: 0.6 }}>{a.source} · {a.timestamp}</div>
            {a.reason && <div style={{ fontSize: 12, opacity: 0.8 }}>{a.reason}</div>}
          </div>
        ))}
      </div>
    </div>
  );
}

function DecisionBadge({ decision }: { decision: string }) {
  const { t } = useTranslation();
  const map: Record<string, { label: string; tone: string }> = {
    allow: { label: t("permissions_tab.decision_allow"), tone: "tone-success" },
    ask: { label: t("permissions_tab.decision_ask"), tone: "tone-warning" },
    deny: { label: t("permissions_tab.decision_deny"), tone: "tone-danger" },
  };
  const s = map[decision] ?? { label: decision, tone: "tone-neutral" };
  return <span className={`mori-pill-badge ${s.tone}`}>{s.label}</span>;
}
