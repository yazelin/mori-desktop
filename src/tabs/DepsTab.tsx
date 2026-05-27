// Phase 5O: Optional dependencies tab。
// 列 mori 在意的 optional dep,顯示「✓ 已裝 / ✗ 未裝」+ 觸發安裝按鈕。
// needs_sudo 的 dep 不代執行,直接複製指令給 user 在 terminal 跑。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { IconRefresh, IconClipboard } from "../icons";

type CheckSpec =
  | { kind: "Which"; bin: string }
  | { kind: "File"; path_template: string }
  | { kind: "WhisperServer"; path_template: string };
type InstallSpec =
  | { kind: "Shell"; script: string }
  | { kind: "Manual"; commands: string[] }
  | {
      kind: "Download";
      url: string;
      dest_dir: string;
      extract_members: string[];
      make_executable: boolean;
    };

type DepStatus = {
  id: string;
  installed: boolean;
  detail: string | null;
};

// 後端 deps_list 已經過濾掉跟當前平台無關的條目 + 把 install 解析成當前
// 平台適用的版本(install_overrides 不送)。前端只看一個 flat shape。
type DepInfo = {
  id: string;
  name: string;
  description: string;
  unlocks: string;
  size_hint: string | null;
  needs_sudo: boolean;
  /// 「能用但有限制」的警告 — 例:whisper-server 在 Windows = "請手動下載 .exe"。
  /// 有值就 render 成 ⚠️ badge。
  install_caveat: string | null;
  check: CheckSpec;
  install: InstallSpec;
  status: DepStatus;
};

type InstallResult = {
  success: boolean;
  exit_code: number | null;
  output: string;
};

function useCommandPreview() {
  const { t } = useTranslation();
  return (install: InstallSpec): string => {
    switch (install.kind) {
      case "Shell":
        return install.script;
      case "Manual":
        return install.commands.join("\n");
      case "Download":
        return [
          `${t("deps_tab.comment_auto_extract")} ${install.dest_dir}`,
          `curl -L -o /tmp/dep.zip "${install.url}"`,
          install.extract_members.length === 0
            ? `unzip /tmp/dep.zip -d ${install.dest_dir}`
            : `unzip /tmp/dep.zip ${install.extract_members.join(" ")} -d ${install.dest_dir}`,
        ].join("\n");
    }
  };
}

function DepCard({ dep, onRefresh }: { dep: DepInfo; onRefresh: () => void }) {
  const { t } = useTranslation();
  const commandPreview = useCommandPreview();
  const [installing, setInstalling] = useState(false);
  const [result, setResult] = useState<InstallResult | null>(null);
  const cmdPreview = commandPreview(dep.install);
  const manual = dep.install.kind === "Manual";
  // Manual 條目初始就展開(user 本來就要看指令)— 之前 showCommand 預設 false +
  // 顯示條件 `(showCommand || manual)` 讓 Manual block 永遠 visible,「收起」
  // 按鈕變裝飾。改成 Manual 預設 true + 顯示條件單看 showCommand,「收起」
  // 真的能收起。
  const [showCommand, setShowCommand] = useState(manual && !dep.status.installed);

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
          {dep.install_caveat && (
            <span
              className="mori-dep-caveat"
              title={dep.install_caveat}
            >
              {t("deps_tab.caveat_badge")}
            </span>
          )}
        </div>
        <div className="mori-dep-actions">
          {dep.status.installed ? (
            <>
              <span className="mori-dep-status-detail">{dep.status.detail}</span>
              {manual ? (
                <button className="mori-btn" onClick={() => setShowCommand(!showCommand)}>
                  {showCommand ? t("deps_tab.hide_command") : t("deps_tab.show_command")}
                </button>
              ) : (
                <button
                  className="mori-btn"
                  onClick={install}
                  disabled={installing}
                >
                  {installing ? t("deps_tab.reinstalling_button") : t("deps_tab.reinstall_button")}
                </button>
              )}
            </>
          ) : manual ? (
            <button className="mori-btn" onClick={() => setShowCommand(!showCommand)}>
              {showCommand ? t("deps_tab.hide_command") : t("deps_tab.show_command")}
            </button>
          ) : (
            <button
              className="mori-btn primary"
              onClick={install}
              disabled={installing}
            >
              {installing ? t("deps_tab.installing_button") : t("deps_tab.install_button")}
            </button>
          )}
        </div>
      </div>
      <div className="mori-dep-desc">{dep.description}</div>
      <div className="mori-dep-unlocks">
        <span className="label">{t("deps_tab.unlocks_label")}</span> {dep.unlocks}
      </div>
      {dep.install_caveat && (
        <div className="mori-dep-caveat-detail">
          ⚠️ <span className="label">{t("deps_tab.platform_note_label")}</span> {dep.install_caveat}
        </div>
      )}
      {showCommand && (
        <div className="mori-dep-cmd">
          <div className="mori-dep-cmd-head">
            <span className="label">
              {manual ? t("deps_tab.manual_label") : t("deps_tab.auto_label")}
            </span>
            <button className="mori-btn small ghost" onClick={copyCmd}><IconClipboard width={12} height={12} /> {t("deps_tab.copy_button")}</button>
          </div>
          <pre>{cmdPreview}</pre>
          {manual && (
            <button className="mori-btn small" onClick={onRefresh}>{t("deps_tab.recheck_after_manual")}</button>
          )}
        </div>
      )}
      {result && (
        <div className={`mori-dep-result ${result.success ? "ok" : "err"}`}>
          <div className="mori-dep-result-head">
            <span>{result.success ? t("deps_tab.install_ok") : t("deps_tab.install_fail")}</span>
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
  const { t } = useTranslation();
  const [deps, setDeps] = useState<DepInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // force=true 強制重檢(走 Refresh 鈕 / install 完成的 callback);
  // force=false / 省略 → 後端有 cache 就直接回,沒 cache 才真檢。
  // 第一次 mount 用 false → 從快取秒開,沒快取才真跑檢測。
  const reload = async (force = false) => {
    setLoading(true);
    try {
      const list = await invoke<DepInfo[]>("deps_list", { force });
      setDeps(list);
      setError(null);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { reload(false); }, []);

  const installedCount = deps.filter((d) => d.status.installed).length;

  return (
    <div className="mori-tab mori-tab-deps">
      <h2 className="mori-tab-title">{t("deps_tab.title")}</h2>
      <p className="mori-tab-hint">{t("deps_tab.hint")}</p>
      {/* D 方案提示:狀態不會自動更新,UI 不會背景 poll,user 在外面手動裝 / 升級 deps
          後要按 Refresh 才看得到新狀態。透過 Mori 自己的「安裝」按鈕裝完會自動 invalidate
          快取,所以那條路徑 user 不用按。 */}
      <p className="mori-tab-hint" style={{ opacity: 0.7, fontSize: "0.85em" }}>
        ⓘ 狀態使用快取顯示,進 Tab 不會重新偵測。在 Mori 外面自己裝 / 升級任何依賴後,按右側「重新整理」才會更新。
      </p>

      {error && <div className="mori-config-error">{error}</div>}

      <div className="mori-deps-toolbar">
        <span className="mori-memory-count">{installedCount} / {deps.length} {t("deps_tab.installed_count_suffix")}</span>
        <button className="mori-btn" onClick={() => reload(true)}><IconRefresh width={13} height={13} /> {t("deps_tab.refresh_button")}</button>
      </div>

      {loading ? (
        <div className="mori-tab-placeholder"><p>{t("deps_tab.detecting")}</p></div>
      ) : (
        <div className="mori-deps-list">
          {deps.map((d) => (
            // install 完 / manual recheck 都當 force refresh — 同 Refresh 鈕路徑,
            // 確保剛裝完外部 deps 立刻看到綠勾,不被殘留 cache 騙。
            <DepCard key={d.id} dep={d} onRefresh={() => reload(true)} />
          ))}
        </div>
      )}
    </div>
  );
}

export default DepsTab;
