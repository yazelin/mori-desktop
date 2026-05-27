import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

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

export function BodyTab() {
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
        <h2 style={{ margin: 0 }}>身體部件</h2>
        <button className="mori-btn small ghost" onClick={refresh}>重新整理</button>
      </div>
      <p style={{ opacity: 0.7, fontSize: 12 }}>
        掃描 <code>~/.mori/body-parts/</code> 的 manifest(唯讀;不會啟動或執行任何部件)。
      </p>
      {err && <div style={{ color: "rgba(255,160,160,.95)", fontSize: 12 }}>❌ {err}</div>}
      {parts.length === 0 && !err && <div style={{ opacity: 0.6 }}>還沒有任何 body part。</div>}
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        {parts.map((p) => (
          <div key={p.source} style={{ border: "1px solid var(--c-border)", borderRadius: 8, padding: 10 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <strong>{p.manifest?.name ?? "(無法解析)"}</strong>
              <span style={{ fontSize: 11, opacity: 0.7 }}>{p.manifest?.kind}</span>
              <StatusBadge status={p.status} />
            </div>
            {p.manifest?.id && <div style={{ fontSize: 11, opacity: 0.6 }}>{p.manifest.id}</div>}
            {p.manifest?.capabilities?.length ? (
              <div style={{ fontSize: 12, marginTop: 4 }}>能力:{p.manifest.capabilities.join(", ")}</div>
            ) : null}
            {p.manifest?.interfaces?.length ? (
              <div style={{ fontSize: 12 }}>介面:{p.manifest.interfaces.map((i) => `${i.name}(${i.transport})`).join(", ")}</div>
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
  const map: Record<string, { t: string; c: string }> = {
    valid: { t: "✓ 正常", c: "rgba(140,220,160,.9)" },
    unsupported_schema: { t: "⚠ 版本不支援", c: "rgba(230,200,120,.9)" },
    parse_error: { t: "✗ 解析失敗", c: "rgba(255,160,160,.95)" },
  };
  const s = map[status] ?? { t: status, c: "var(--c-text-muted)" };
  return <span style={{ fontSize: 11, color: s.c }}>{s.t}</span>;
}
