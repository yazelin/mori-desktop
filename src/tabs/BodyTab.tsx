import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

interface BodyManifest {
  schema_version: number;
  id: string;
  name: string;
  kind: string;
  description?: string;
  capabilities?: string[];
  interfaces?: { name: string; transport: string }[];
  permissions?: string[];
}
interface DiscoveredBodyPart {
  source: string;
  status: string; // valid | unsupported_schema | parse_error
  detail: string | null;
  manifest: BodyManifest | null;
}

export default function BodyTab() {
  const { t } = useTranslation();
  const [parts, setParts] = useState<DiscoveredBodyPart[]>([]);
  const [err, setErr] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setParts(await invoke<DiscoveredBodyPart[]>("body_registry_list"));
      setErr(null);
    } catch (e: any) {
      setErr(String(e));
    }
  };
  useEffect(() => { refresh(); }, []);

  return (
    <div style={{ padding: 16 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12 }}>
        <h2 style={{ margin: 0 }}>{t("body_tab.title")}</h2>
        <button className="mori-btn small ghost" onClick={refresh}>{t("body_tab.refresh")}</button>
      </div>
      <p style={{ opacity: 0.7, fontSize: 12 }}>
        {t("body_tab.hint")} (<code>~/.mori/body-parts/</code>)
      </p>
      {err && <div style={{ color: "rgba(255,160,160,.95)", fontSize: 12 }}>❌ {err}</div>}
      {parts.length === 0 && !err && <div style={{ opacity: 0.6 }}>{t("body_tab.empty")}</div>}
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        {parts.map((p) => (
          <div key={p.source} style={{ border: "1px solid var(--c-border)", borderRadius: 8, padding: 10 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <strong>{p.manifest?.name ?? t("body_tab.parse_error_name")}</strong>
              <span style={{ fontSize: 11, opacity: 0.7 }}>{p.manifest?.kind}</span>
              <StatusBadge status={p.status} />
            </div>
            {p.manifest?.id && <div style={{ fontSize: 11, opacity: 0.6 }}>{p.manifest.id}</div>}
            {p.manifest?.capabilities?.length ? (
              <div style={{ fontSize: 12, marginTop: 4 }}>{t("body_tab.capabilities")}: {p.manifest.capabilities.join(", ")}</div>
            ) : null}
            {p.manifest?.interfaces?.length ? (
              <div style={{ fontSize: 12 }}>{t("body_tab.interfaces")}: {p.manifest.interfaces.map((i) => `${i.name}(${i.transport})`).join(", ")}</div>
            ) : null}
            {p.detail && <div style={{ fontSize: 12, color: "rgba(255,160,160,.95)" }}>{p.detail}</div>}
            <div style={{ fontSize: 10, opacity: 0.4, wordBreak: "break-all", marginTop: 4 }}>{p.source}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const { t } = useTranslation();
  const map: Record<string, { label: string; c: string }> = {
    valid: { label: t("body_tab.status_valid"), c: "rgba(140,220,160,.9)" },
    unsupported_schema: { label: t("body_tab.status_unsupported"), c: "rgba(230,200,120,.9)" },
    parse_error: { label: t("body_tab.status_parse_error"), c: "rgba(255,160,160,.95)" },
  };
  const s = map[status] ?? { label: status, c: "var(--c-text-muted)" };
  return <span style={{ fontSize: 11, color: s.c }}>{s.label}</span>;
}
