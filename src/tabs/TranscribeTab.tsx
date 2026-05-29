// 轉錄 tab — 本機 whisper.cpp 把音檔 / 影片轉成逐字稿。
//
// 兩個模式(top tab 切換):
// 1. **單檔** — picker / drag-drop 一個 file → ffmpeg 抽音軌 → whisper → transcript
// 2. **批次** — picker 一個資料夾 → scan supported exts → 逐個轉 → 旁邊存 .txt
//
// 全部走本機 whisper-local provider(不送 Groq 雲端),user 已透過 Quickstart
// / Deps tab / 手動 setup whisper-server + model。Dep 檢查紅標時直接擋轉錄按鈕。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { IconRefresh, IconClipboard, IconCheck, IconWarning } from "../icons";
import { Select } from "../Select";

// ─── shared types ───────────────────────────────────────────────────────

type DepStatus = {
  ffmpeg_ok: boolean;
  ffmpeg_version?: string;
  whisper_binary_ok: boolean;
  whisper_binary_path: string;
  whisper_model_ok: boolean;
  whisper_model_path: string;
};

type TranscribeOutput = {
  source_path: string;
  text: string;
  duration_secs: number;
  chunks: number;
};

type BatchEntry = TranscribeOutput & {
  ok: boolean;
  error?: string | null;
};

type FolderScanEntry = { path: string; name: string; size_bytes: number };

type Mode = "file" | "batch";

type Language = "auto" | "zh" | "en" | "ja";

// ─── tab root ───────────────────────────────────────────────────────────

export default function TranscribeTab() {
  const { t } = useTranslation();
  const [mode, setMode] = useState<Mode>("file");
  const [language, setLanguage] = useState<Language>("auto");
  const [deps, setDeps] = useState<DepStatus | null>(null);
  const [depsRefreshing, setDepsRefreshing] = useState(false);

  const refreshDeps = async () => {
    setDepsRefreshing(true);
    try {
      const d = await invoke<DepStatus>("transcribe_check_deps");
      setDeps(d);
    } finally {
      setDepsRefreshing(false);
    }
  };

  useEffect(() => {
    refreshDeps();
  }, []);

  const depsOk = !!deps && deps.ffmpeg_ok && deps.whisper_binary_ok && deps.whisper_model_ok;

  return (
    <div className="mori-tab mori-tab-transcribe">
      <h2 className="mori-tab-title">{t("transcribe_tab.title")}</h2>
      <p className="mori-tab-hint">{t("transcribe_tab.hint")}</p>

      <DepsPanel deps={deps} onRefresh={refreshDeps} refreshing={depsRefreshing} />

      <div className="mori-transcribe-mode-bar">
        {(["file", "batch"] as const).map((m) => (
          <button
            key={m}
            className={`mori-btn small ${mode === m ? "primary" : ""}`}
            onClick={() => setMode(m)}
          >
            {t(`transcribe_tab.mode_${m}`)}
          </button>
        ))}

        <div className="mori-transcribe-lang-picker">
          <label>{t("transcribe_tab.language_label")}</label>
          <Select
            value={language}
            onChange={(v) => setLanguage(v as Language)}
            options={[
              { value: "auto", label: t("transcribe_tab.language_auto") },
              { value: "zh", label: t("transcribe_tab.language_zh") },
              { value: "en", label: t("transcribe_tab.language_en") },
              { value: "ja", label: t("transcribe_tab.language_ja") },
            ]}
          />
        </div>
      </div>

      {!depsOk && (
        <div className="mori-transcribe-block-banner">
          <IconWarning width={14} height={14} /> {t("transcribe_tab.error_no_deps")}
        </div>
      )}

      {mode === "file" && <FileMode language={language} disabled={!depsOk} />}
      {mode === "batch" && <BatchMode language={language} disabled={!depsOk} />}
    </div>
  );
}

// ─── deps panel ─────────────────────────────────────────────────────────

function DepsPanel({
  deps,
  onRefresh,
  refreshing,
}: {
  deps: DepStatus | null;
  onRefresh: () => void;
  refreshing: boolean;
}) {
  const { t } = useTranslation();
  if (!deps) {
    return <div className="mori-transcribe-deps loading">…</div>;
  }
  const Row = ({
    ok,
    label,
    detail,
    installHint,
  }: {
    ok: boolean;
    label: string;
    detail?: string;
    installHint: string;
  }) => (
    <div className={`mori-transcribe-dep-row ${ok ? "ok" : "missing"}`}>
      <span className="mori-transcribe-dep-badge">
        {ok ? t("transcribe_tab.deps_ok") : t("transcribe_tab.deps_missing")}
      </span>
      <span className="mori-transcribe-dep-label">{label}</span>
      {detail && <span className="mori-transcribe-dep-detail">{detail}</span>}
      {!ok && <span className="mori-transcribe-dep-hint">{installHint}</span>}
    </div>
  );

  return (
    <section className="mori-transcribe-deps">
      <div className="mori-transcribe-deps-header">
        <h3>{t("transcribe_tab.deps_section")}</h3>
        <button className="mori-btn small ghost" onClick={onRefresh} disabled={refreshing}>
          <IconRefresh width={12} height={12} /> {t("transcribe_tab.deps_recheck")}
        </button>
      </div>
      <Row
        ok={deps.ffmpeg_ok}
        label={t("transcribe_tab.deps_ffmpeg")}
        detail={deps.ffmpeg_version ?? undefined}
        installHint={t("transcribe_tab.deps_install_hint_ffmpeg")}
      />
      <Row
        ok={deps.whisper_binary_ok}
        label={t("transcribe_tab.deps_whisper_binary")}
        detail={deps.whisper_binary_path}
        installHint={t("transcribe_tab.deps_install_hint_whisper")}
      />
      <Row
        ok={deps.whisper_model_ok}
        label={t("transcribe_tab.deps_whisper_model")}
        detail={deps.whisper_model_path}
        installHint={t("transcribe_tab.deps_install_hint_model")}
      />
    </section>
  );
}

// ─── shared: result display + copy/save controls ────────────────────────

function ResultBlock({
  sourcePath,
  text,
  duration,
  chunks,
}: {
  sourcePath: string;
  text: string;
  duration: number;
  chunks: number;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      console.warn("copy fail", e);
    }
  };
  const save = async () => {
    try {
      const path = await invoke<string>("transcribe_save_alongside", {
        sourcePath,
        text,
      });
      setSaveMsg(t("transcribe_tab.save_alongside_ok", { path }));
      setTimeout(() => setSaveMsg(null), 4000);
    } catch (e: any) {
      setSaveMsg(t("transcribe_tab.save_alongside_fail", { e: String(e) }));
    }
  };

  return (
    <div className="mori-transcribe-result">
      <div className="mori-transcribe-result-meta">
        <span>{duration.toFixed(1)}s</span>
        {chunks > 1 && <span>· {chunks} chunks</span>}
      </div>
      <textarea className="mori-transcribe-result-textarea" value={text} readOnly />
      {!text && <p className="mori-transcribe-empty">{t("transcribe_tab.result_empty")}</p>}
      <div className="mori-transcribe-result-actions">
        <button className="mori-btn small" onClick={copy} disabled={!text}>
          {copied ? (
            <><IconCheck width={12} height={12} /> {t("transcribe_tab.copy_done")}</>
          ) : (
            <><IconClipboard width={12} height={12} /> {t("transcribe_tab.copy_button")}</>
          )}
        </button>
        <button className="mori-btn small" onClick={save} disabled={!text}>
          {t("transcribe_tab.save_txt_button")}
        </button>
        {saveMsg && <span className="mori-transcribe-save-msg">{saveMsg}</span>}
      </div>
    </div>
  );
}

// ─── Mode A: single file ────────────────────────────────────────────────

function FileMode({ language, disabled }: { language: Language; disabled: boolean }) {
  const { t } = useTranslation();
  const [pickedPath, setPickedPath] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<TranscribeOutput | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chunkProgress, setChunkProgress] = useState<{ chunk: number; total: number } | null>(
    null,
  );

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<{ chunk: number; total: number; path: string }>("transcribe-chunk-progress", (e) => {
      if (pickedPath && e.payload.path === pickedPath) {
        setChunkProgress({ chunk: e.payload.chunk, total: e.payload.total });
      }
    }).then((u) => (unlisten = u));
    return () => {
      unlisten?.();
    };
  }, [pickedPath]);

  // Tauri 2 drag-drop: file 拖到視窗時 webview 會 emit。我們監聽 'tauri://drag-drop'
  // 與 'tauri://drop' 兩個歷史命名,擇一觸發。
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    const handler = (paths: string[]) => {
      if (paths.length > 0) setPickedPath(paths[0]);
    };
    ["tauri://drag-drop", "tauri://drop"].forEach((evt) => {
      listen<any>(evt, (e) => {
        const p = (e.payload as any)?.paths;
        if (Array.isArray(p)) handler(p);
      })
        .then((u) => unlisteners.push(u))
        .catch(() => {});
    });
    return () => unlisteners.forEach((u) => u());
  }, []);

  const pick = async () => {
    const path = await openDialog({
      multiple: false,
      filters: [
        {
          name: "Audio / Video",
          extensions: ["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "mp4", "mkv", "webm", "mov", "avi"],
        },
      ],
    });
    if (typeof path === "string") setPickedPath(path);
  };

  const run = async () => {
    if (!pickedPath) return;
    setRunning(true);
    setError(null);
    setResult(null);
    setChunkProgress(null);
    try {
      const r = await invoke<TranscribeOutput>("transcribe_file_cmd", {
        path: pickedPath,
        language: language === "auto" ? null : language,
      });
      setResult(r);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setRunning(false);
      setChunkProgress(null);
    }
  };

  return (
    <section className="mori-transcribe-mode-section">
      <div className="mori-transcribe-droparea">
        <button className="mori-btn" onClick={pick} disabled={disabled || running}>
          {t("transcribe_tab.file_pick_button")}
        </button>
        <p className="mori-transcribe-droparea-hint">
          {pickedPath ?? t("transcribe_tab.file_drop_hint")}
        </p>
        <p className="mori-transcribe-droparea-supported">{t("transcribe_tab.file_supported")}</p>
      </div>

      <div className="mori-transcribe-run-row">
        <button
          className="mori-btn primary"
          onClick={run}
          disabled={disabled || running || !pickedPath}
        >
          {running
            ? t("transcribe_tab.transcribe_button_running")
            : t("transcribe_tab.transcribe_button")}
        </button>
        {chunkProgress && (
          <span className="mori-transcribe-progress">
            {t("transcribe_tab.progress_chunk", chunkProgress)}
          </span>
        )}
      </div>

      {error && (
        <div className="mori-transcribe-error">
          {t("transcribe_tab.error_transcribe", { e: error })}
        </div>
      )}
      {result && (
        <ResultBlock
          sourcePath={result.source_path}
          text={result.text}
          duration={result.duration_secs}
          chunks={result.chunks}
        />
      )}
    </section>
  );
}

// ─── Mode B: batch ──────────────────────────────────────────────────────

function BatchMode({ language, disabled }: { language: Language; disabled: boolean }) {
  const { t } = useTranslation();
  const [folder, setFolder] = useState<string | null>(null);
  const [scan, setScan] = useState<FolderScanEntry[]>([]);
  const [running, setRunning] = useState(false);
  const [statuses, setStatuses] = useState<Record<string, "pending" | "running" | "ok" | "err">>(
    {},
  );
  const [results, setResults] = useState<BatchEntry[]>([]);
  const [overall, setOverall] = useState<{ index: number; total: number } | null>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<{ index: number; total: number; path: string; status: string }>(
      "transcribe-file-progress",
      (e) => {
        setOverall({ index: e.payload.index, total: e.payload.total });
        setStatuses((s) => ({
          ...s,
          [e.payload.path]: e.payload.status === "start"
            ? "running"
            : e.payload.status === "ok"
            ? "ok"
            : "err",
        }));
      },
    ).then((u) => (unlisten = u));
    return () => {
      unlisten?.();
    };
  }, []);

  const pickFolder = async () => {
    const dir = await openDialog({ directory: true, multiple: false });
    if (typeof dir === "string") {
      setFolder(dir);
      try {
        const entries = await invoke<FolderScanEntry[]>("transcribe_scan_folder", { folder: dir });
        setScan(entries);
        setStatuses(
          Object.fromEntries(entries.map((e) => [e.path, "pending"])) as any,
        );
        setResults([]);
      } catch (e) {
        console.error(e);
      }
    }
  };

  const run = async () => {
    if (scan.length === 0) return;
    setRunning(true);
    setResults([]);
    try {
      const res = await invoke<BatchEntry[]>("transcribe_paths_cmd", {
        paths: scan.map((e) => e.path),
        language: language === "auto" ? null : language,
      });
      setResults(res);
    } catch (e) {
      console.error(e);
    } finally {
      setRunning(false);
      setOverall(null);
    }
  };

  const saveAll = async () => {
    for (const r of results) {
      if (r.ok && r.text) {
        try {
          await invoke("transcribe_save_alongside", {
            sourcePath: r.source_path,
            text: r.text,
          });
        } catch (e) {
          console.warn("save fail", r.source_path, e);
        }
      }
    }
  };

  return (
    <section className="mori-transcribe-mode-section">
      <div className="mori-transcribe-droparea">
        <button className="mori-btn" onClick={pickFolder} disabled={disabled || running}>
          {t("transcribe_tab.folder_pick_button")}
        </button>
        <p className="mori-transcribe-droparea-hint">
          {folder ?? t("transcribe_tab.folder_no_selection")}
        </p>
        {folder && (
          <p className="mori-transcribe-droparea-supported">
            {scan.length === 0
              ? t("transcribe_tab.folder_empty")
              : t("transcribe_tab.folder_found", { count: scan.length })}
          </p>
        )}
      </div>

      {scan.length > 0 && (
        <>
          <div className="mori-transcribe-run-row">
            <button
              className="mori-btn primary"
              onClick={run}
              disabled={disabled || running}
            >
              {running
                ? t("transcribe_tab.transcribe_button_running")
                : t("transcribe_tab.transcribe_button")}
            </button>
            {overall && (
              <span className="mori-transcribe-progress">
                {t("transcribe_tab.progress_file", overall)}
              </span>
            )}
            {results.length > 0 && results.some((r) => r.ok && r.text) && (
              <button className="mori-btn small" onClick={saveAll}>
                {t("transcribe_tab.batch_save_all_button")}
              </button>
            )}
          </div>

          <ul className="mori-transcribe-batch-list">
            {scan.map((e) => {
              const status = statuses[e.path] ?? "pending";
              const result = results.find((r) => r.source_path === e.path);
              return (
                <li key={e.path} className={`mori-transcribe-batch-row status-${status}`}>
                  <span className="mori-transcribe-batch-name">{e.name}</span>
                  <span className="mori-transcribe-batch-status">
                    {t(`transcribe_tab.batch_status_${status}`)}
                  </span>
                  {result && result.ok && (
                    <span className="mori-transcribe-batch-meta">
                      {result.duration_secs.toFixed(1)}s · {result.chunks} ch
                    </span>
                  )}
                  {result && !result.ok && (
                    <span className="mori-transcribe-batch-err">{result.error}</span>
                  )}
                </li>
              );
            })}
          </ul>

          {results.filter((r) => r.ok && r.text).map((r) => (
            <details key={r.source_path} className="mori-transcribe-batch-detail">
              <summary>{r.source_path}</summary>
              <ResultBlock
                sourcePath={r.source_path}
                text={r.text}
                duration={r.duration_secs}
                chunks={r.chunks}
              />
            </details>
          ))}
        </>
      )}
    </section>
  );
}

