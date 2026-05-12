// Phase 5O: Optional dependencies tab。
// 列 mori 在意的 optional dep,顯示「✓ 已裝 / ✗ 未裝」+ 觸發安裝按鈕。
// needs_sudo 的 dep 不代執行,直接複製指令給 user 在 terminal 跑。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconRefresh, IconClipboard } from "../icons";

type CheckSpec =
  | { kind: "Which"; bin: string }
  | { kind: "File"; path_template: string };
type InstallSpec =
  | { kind: "Shell"; script: string }
  | { kind: "Manual"; commands: string[] };

type DepSpec = {
  id: string;
  name: string;
  description: string;
  unlocks: string;
  size_hint: string | null;
  needs_sudo: boolean;
  check: CheckSpec;
  install: InstallSpec;
};

type DepStatus = {
  id: string;
  installed: boolean;
  detail: string | null;
};

type DepInfo = DepSpec & { status: DepStatus };

type InstallResult = {
  success: boolean;
  exit_code: number | null;
  output: string;
};

function commandPreview(install: InstallSpec): string {
  switch (install.kind) {
    case "Shell":
      return install.script;
    case "Manual":
      return install.commands.join("\n");
  }
}

function DepCard({ dep, onRefresh }: { dep: DepInfo; onRefresh: () => void }) {
  const [installing, setInstalling] = useState(false);
  const [result, setResult] = useState<InstallResult | null>(null);
  const [showCommand, setShowCommand] = useState(false);
  const cmdPreview = commandPreview(dep.install);
  const manual = dep.install.kind === "Manual";

  const install = async () => {
    if (manual) return; // UI 不會 disable,但 click 也不送
    setInstalling(true);
    setResult(null);
    try {
      const r = await invoke<InstallResult>("deps_install", { id: dep.id });
      setResult(r);
      onRefresh(); // 重新 check status
    } catch (e: any) {
      setResult({ success: false, exit_code: null, output: String(e) });
    } finally {
      setInstalling(false);
    }
  };

  const copyCmd = async () => {
    try {
      await navigator.clipboard.writeText(cmdPreview);
    } catch (e) { console.error("copy failed", e); }
  };

  return (
    <div className={`mori-dep-card ${dep.status.installed ? "installed" : ""}`}>
      <div className="mori-dep-head">
        <span className={`mori-dep-status ${dep.status.installed ? "ok" : "missing"}`}>
          {dep.status.installed ? "✓" : "✗"}
        </span>
        <div className="mori-dep-title">
          <span className="mori-dep-name">{dep.name}</span>
          {dep.size_hint && <span className="mori-dep-size">{dep.size_hint}</span>}
          {dep.needs_sudo && <span className="mori-dep-sudo">sudo</span>}
        </div>
        <div className="mori-dep-actions">
          {dep.status.installed ? (
            <span className="mori-dep-status-detail">{dep.status.detail}</span>
          ) : manual ? (
            <button className="mori-btn" onClick={() => setShowCommand(!showCommand)}>
              {showCommand ? "收起" : "顯示安裝指令"}
            </button>
          ) : (
            <button
              className="mori-btn primary"
              onClick={install}
              disabled={installing}
            >
              {installing ? "安裝中…" : "安裝"}
            </button>
          )}
        </div>
      </div>
      <div className="mori-dep-desc">{dep.description}</div>
      <div className="mori-dep-unlocks">
        <span className="label">解鎖:</span> {dep.unlocks}
      </div>
      {!dep.status.installed && (showCommand || manual) && (
        <div className="mori-dep-cmd">
          <div className="mori-dep-cmd-head">
            <span className="label">{manual ? "請複製指令在 terminal 跑(需 sudo)" : "將執行的指令"}</span>
            <button className="mori-btn small ghost" onClick={copyCmd}><IconClipboard width={12} height={12} /> 複製</button>
          </div>
          <pre>{cmdPreview}</pre>
          {manual && (
            <button className="mori-btn small" onClick={onRefresh}>裝完後按這 重新檢測</button>
          )}
        </div>
      )}
      {result && (
        <div className={`mori-dep-result ${result.success ? "ok" : "err"}`}>
          <div className="mori-dep-result-head">
            <span>{result.success ? "✓ 安裝成功" : "✗ 失敗"}</span>
            {result.exit_code != null && (
              <span className="exit-code">exit={result.exit_code}</span>
            )}
          </div>
          {result.output && <pre>{result.output}</pre>}
        </div>
      )}
    </div>
  );
}

function DepsTab() {
  const [deps, setDeps] = useState<DepInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await invoke<DepInfo[]>("deps_list");
      setDeps(list);
      setError(null);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { reload(); }, []);

  const installedCount = deps.filter((d) => d.status.installed).length;

  return (
    <div className="mori-tab mori-tab-deps">
      <h2 className="mori-tab-title">Dependencies</h2>
      <p className="mori-tab-hint">
        Optional 工具 / 模型。裝了某個就解鎖對應 feature。需 sudo 的(系統套件)
        會顯示指令給你在 terminal 自己跑,其他(pip / wget / curl install.sh)
        mori 可代執行。
      </p>

      {error && <div className="mori-config-error">{error}</div>}

      <div className="mori-deps-toolbar">
        <span className="mori-memory-count">{installedCount} / {deps.length} 已裝</span>
        <button className="mori-btn" onClick={reload}><IconRefresh width={13} height={13} /> 重新檢測</button>
      </div>

      {loading ? (
        <div className="mori-tab-placeholder"><p>檢測中…</p></div>
      ) : (
        <div className="mori-deps-list">
          {deps.map((d) => (
            <DepCard key={d.id} dep={d} onRefresh={reload} />
          ))}
        </div>
      )}
    </div>
  );
}

export default DepsTab;
