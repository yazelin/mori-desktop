// 5L: ~/.mori/config.json + corrections.md 編輯器。
// 第一版用 textarea + 即時 JSON 驗證,5L-2 再做表單版。
//
// 設計:三個 sub-section
// - config.json:JSON textarea,parse 失敗顯示錯誤、不讓存
// - corrections.md:純文字 textarea
// - (5L-2)providers / api_keys 表單

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type SaveStatus =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok"; at: number }
  | { kind: "err"; message: string };

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mori-config-section">
      <h3 className="mori-config-section-title">{title}</h3>
      {hint && <p className="mori-config-section-hint">{hint}</p>}
      {children}
    </section>
  );
}

function StatusBadge({ status }: { status: SaveStatus }) {
  if (status.kind === "idle") return null;
  if (status.kind === "saving") return <span className="mori-save-status saving">儲存中…</span>;
  if (status.kind === "ok") return <span className="mori-save-status ok">✓ 已儲存</span>;
  return <span className="mori-save-status err">✗ {status.message}</span>;
}

function ConfigTab() {
  const [configText, setConfigText] = useState<string>("");
  const [configOrig, setConfigOrig] = useState<string>("");
  const [configStatus, setConfigStatus] = useState<SaveStatus>({ kind: "idle" });
  const [configError, setConfigError] = useState<string | null>(null);

  const [correctionsText, setCorrectionsText] = useState<string>("");
  const [correctionsOrig, setCorrectionsOrig] = useState<string>("");
  const [correctionsStatus, setCorrectionsStatus] = useState<SaveStatus>({ kind: "idle" });

  useEffect(() => {
    invoke<string>("config_read")
      .then((t) => {
        setConfigText(t);
        setConfigOrig(t);
      })
      .catch((e) => setConfigError(`load: ${e}`));
    invoke<string>("corrections_read")
      .then((t) => {
        setCorrectionsText(t);
        setCorrectionsOrig(t);
      })
      .catch(() => {
        // corrections.md 不存在不算錯,提供一個 starter
        setCorrectionsText("# Mori STT 校正表\n\n# 看到左邊 → 改成右邊\n# 例:modem -> Markdown\n\n");
      });
  }, []);

  // 即時 JSON validate
  useEffect(() => {
    if (!configText) { setConfigError(null); return; }
    try {
      JSON.parse(configText);
      setConfigError(null);
    } catch (e: any) {
      setConfigError(e.message);
    }
  }, [configText]);

  const saveConfig = async () => {
    if (configError) return;
    setConfigStatus({ kind: "saving" });
    try {
      await invoke("config_write", { text: configText });
      setConfigOrig(configText);
      setConfigStatus({ kind: "ok", at: Date.now() });
      setTimeout(() => setConfigStatus({ kind: "idle" }), 2500);
    } catch (e: any) {
      setConfigStatus({ kind: "err", message: String(e) });
    }
  };

  const saveCorrections = async () => {
    setCorrectionsStatus({ kind: "saving" });
    try {
      await invoke("corrections_write", { text: correctionsText });
      setCorrectionsOrig(correctionsText);
      setCorrectionsStatus({ kind: "ok", at: Date.now() });
      setTimeout(() => setCorrectionsStatus({ kind: "idle" }), 2500);
    } catch (e: any) {
      setCorrectionsStatus({ kind: "err", message: String(e) });
    }
  };

  const configDirty = configText !== configOrig;
  const correctionsDirty = correctionsText !== correctionsOrig;

  return (
    <div className="mori-tab mori-tab-config">
      <h2 className="mori-tab-title">Config</h2>
      <p className="mori-tab-hint">
        編輯 ~/.mori/config.json + corrections.md。改完不需要重啟,下一次熱鍵
        會即時讀新設定(profile 也是)。
      </p>

      <Section
        title="config.json"
        hint="provider / stt_provider / providers.* / api_keys / routing"
      >
        <textarea
          className={`mori-config-textarea ${configError ? "has-error" : ""}`}
          spellCheck={false}
          value={configText}
          onChange={(e) => setConfigText(e.target.value)}
          rows={20}
        />
        {configError && (
          <div className="mori-config-error">JSON parse error: {configError}</div>
        )}
        <div className="mori-config-actions">
          <button
            className="mori-btn primary"
            onClick={saveConfig}
            disabled={!configDirty || !!configError}
          >
            儲存
          </button>
          <button
            className="mori-btn"
            onClick={() => setConfigText(configOrig)}
            disabled={!configDirty}
          >
            還原
          </button>
          <StatusBadge status={configStatus} />
        </div>
      </Section>

      <Section
        title="corrections.md"
        hint="共用 STT 校正表(voice / agent profile 用 #file: 引用)"
      >
        <textarea
          className="mori-config-textarea"
          spellCheck={false}
          value={correctionsText}
          onChange={(e) => setCorrectionsText(e.target.value)}
          rows={14}
        />
        <div className="mori-config-actions">
          <button
            className="mori-btn primary"
            onClick={saveCorrections}
            disabled={!correctionsDirty}
          >
            儲存
          </button>
          <button
            className="mori-btn"
            onClick={() => setCorrectionsText(correctionsOrig)}
            disabled={!correctionsDirty}
          >
            還原
          </button>
          <StatusBadge status={correctionsStatus} />
        </div>
      </Section>
    </div>
  );
}

export default ConfigTab;
