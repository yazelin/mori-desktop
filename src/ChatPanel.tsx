// 5N: Chat panel 重設計 — scrollable thread + bottom input bar。
//
// 結構:
//   ┌─── top bar(mode chip + 三個 icon 按鈕)───────┐
//   ├─── scrollable thread(歷次 user / assistant)──┤
//   ├─── inline status chip(錄音/思考/錯誤)──────┤
//   └─── bottom input bar(mic + textarea + send)──┘
//
//   點 ⚙️ 開 status modal 顯示 build SHA / provider / clipboard 等

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type SkillCallSummary = {
  name: string;
  args_brief: string;
  success: boolean;
};

type Phase =
  | { kind: "idle" }
  | { kind: "recording"; started_at_ms: number }
  | { kind: "transcribing" }
  | { kind: "responding"; transcript: string }
  | {
      kind: "done";
      transcript: string;
      response: string;
      skill_calls: SkillCallSummary[];
    }
  | { kind: "error"; message: string };

type Mode = "agent" | "voice_input" | "background";

type ChatTurn = {
  role: "user" | "assistant";
  content: string;
  tools_called: string[];
};

type BuildInfo = {
  sha: string;
  dirty: boolean;
  build_time: string;
  phase: string;
  version: string;
};

type WarmupState = "loading" | "ready" | "failed";
type SttInfo = { name: string; model: string; language: string | null };
type ChatProviderInfo = {
  name: string;
  model: string;
  warmup: WarmupState | null;
  stt: SttInfo;
};

import {
  IconChat as IconBubble,
  IconKeyboard,
  IconSleep,
  IconSun,
  IconRefresh,
  IconConfig,
  IconClose,
  IconMic,
  IconStop,
  IconWarning,
  IconWave,
  IconTool,
} from "./icons";
import type { ComponentType, SVGProps } from "react";

const MODE_LABEL: Record<Mode, { Icon: ComponentType<SVGProps<SVGSVGElement>>; label: string }> = {
  agent: { Icon: IconBubble, label: "對話模式" },
  voice_input: { Icon: IconKeyboard, label: "語音輸入" },
  background: { Icon: IconSleep, label: "休眠" },
};

function ChatPanel() {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [mode, setMode] = useState<Mode>("agent");
  const [conv, setConv] = useState<ChatTurn[]>([]);
  const [recElapsed, setRecElapsed] = useState(0);
  const [audioLevel, setAudioLevel] = useState(0);
  const [textInput, setTextInput] = useState("");
  const [showStatus, setShowStatus] = useState(false);
  const [buildInfo, setBuildInfo] = useState<BuildInfo | null>(null);
  const [chatProvider, setChatProvider] = useState<ChatProviderInfo | null>(null);
  const [warmup, setWarmup] = useState<WarmupState | null>(null);
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [coreVersion, setCoreVersion] = useState("");
  const [lastContext, setLastContext] = useState<{
    clipboard?: string | null;
    selected_text?: string | null;
  } | null>(null);

  const threadRef = useRef<HTMLDivElement | null>(null);

  // 重抓對話歷史(每次 phase 進入 done / 切 mode / 重置時拉)
  const refreshConversation = () => {
    invoke<ChatTurn[]>("get_conversation").then(setConv).catch(console.error);
  };

  useEffect(() => {
    invoke<Phase>("current_phase").then(setPhase).catch(() => {});
    invoke<Mode>("current_mode").then(setMode).catch(() => {});
    invoke<string>("mori_version").then(setCoreVersion).catch(() => {});
    invoke<boolean>("has_groq_key").then(setHasKey).catch(() => {});
    invoke<BuildInfo>("build_info").then(setBuildInfo).catch(() => {});
    invoke<ChatProviderInfo>("chat_provider_info")
      .then((info) => {
        setChatProvider(info);
        if (info.warmup) setWarmup(info.warmup);
      })
      .catch(() => {});
    refreshConversation();

    const unlistenPhase = listen<Phase>("phase-changed", (e) => {
      setPhase(e.payload);
      if (e.payload.kind === "done") refreshConversation();
    });
    const unlistenMode = listen<Mode>("mode-changed", (e) => setMode(e.payload));
    const unlistenAudio = listen<number>("audio-level", (e) => setAudioLevel(e.payload));
    const unlistenWarmup = listen<WarmupState>("ollama-warmup", (e) => setWarmup(e.payload));
    const unlistenCtx = listen<typeof lastContext>("context-captured", (e) =>
      setLastContext(e.payload),
    );

    return () => {
      unlistenPhase.then((f) => f());
      unlistenMode.then((f) => f());
      unlistenAudio.then((f) => f());
      unlistenWarmup.then((f) => f());
      unlistenCtx.then((f) => f());
    };
  }, []);

  // 錄音時計時
  useEffect(() => {
    if (phase.kind !== "recording") { setRecElapsed(0); return; }
    const start = phase.started_at_ms;
    const tick = () => setRecElapsed(Math.floor((Date.now() - start) / 1000));
    tick();
    const t = setInterval(tick, 250);
    return () => clearInterval(t);
  }, [phase]);

  // thread 有新訊息自動滾到底
  useEffect(() => {
    const el = threadRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [conv.length, phase.kind]);

  const pipelineBusy = phase.kind === "transcribing" || phase.kind === "responding";
  const recording = phase.kind === "recording";

  const onToggle = () => invoke("toggle").catch(console.error);
  const onReset = () =>
    invoke("reset_conversation").then(() => {
      setConv([]);
      setPhase({ kind: "idle" });
    }).catch(console.error);
  const onSubmit = () => {
    const t = textInput.trim();
    if (!t) return;
    invoke("submit_text", { text: t }).then(() => setTextInput("")).catch(console.error);
  };

  const toggleSleep = () => {
    invoke("set_mode_cmd", { mode: mode === "background" ? "agent" : "background" }).catch(console.error);
  };

  // 把 in-progress 那筆放在 thread 末尾(尚未 commit 到 conv,所以 Rust 還沒回來)
  const inProgress: ChatTurn | null =
    phase.kind === "responding"
      ? { role: "user", content: phase.transcript, tools_called: [] }
      : phase.kind === "done" && conv.length === 0
      ? null  // 已經 refresh 進 conv 了
      : null;

  return (
    <div className="mori-chat">
      {/* ── Top bar ─────────────────────────────────────── */}
      <div className="mori-chat-topbar">
        <div className="mori-chat-mode">
          <span className="mori-chat-mode-icon">
            {(() => { const I = MODE_LABEL[mode].Icon; return <I width={16} height={16} />; })()}
          </span>
          <span className="mori-chat-mode-text">{MODE_LABEL[mode].label}</span>
          {chatProvider && (
            <span className="mori-chat-mode-provider">
              · {chatProvider.name} · {chatProvider.model}
            </span>
          )}
        </div>
        <div className="mori-chat-topbar-actions">
          <button
            className="mori-icon-btn"
            onClick={toggleSleep}
            title={mode === "background" ? "醒醒(回對話模式)" : "進休眠(關麥克風)"}
          >
            {mode === "background" ? <IconSun width={16} height={16} /> : <IconSleep width={16} height={16} />}
          </button>
          <button
            className="mori-icon-btn"
            onClick={onReset}
            disabled={conv.length === 0}
            title="清掉本次對話歷史(長期記憶不動)"
          >
            <IconRefresh width={16} height={16} />
          </button>
          <button
            className="mori-icon-btn"
            onClick={() => setShowStatus(true)}
            title="顯示系統狀態"
          >
            <IconConfig width={16} height={16} />
          </button>
        </div>
      </div>

      {/* ── Thread ────────────────────────────────────── */}
      <div className="mori-chat-thread" ref={threadRef}>
        {conv.length === 0 && phase.kind === "idle" && (
          <div className="mori-chat-empty">
            <p>
              <span className="mori-empty-icon"><IconWave width={18} height={18} /></span>
              {" "}跟 Mori 說話 — 按 <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Space</kbd> 開始錄音,或下面直接打字。
            </p>
          </div>
        )}
        {conv.map((turn, i) => (
          <ChatBubble key={i} turn={turn} />
        ))}
        {inProgress && <ChatBubble turn={inProgress} />}
        {phase.kind === "error" && (
          <div className="mori-chat-error">
            <span className="label"><IconWarning width={13} height={13} /> 錯誤</span>
            <p>{phase.message}</p>
          </div>
        )}
      </div>

      {/* ── In-progress chip ──────────────────────────── */}
      {recording && (
        <div className="mori-chat-progress recording">
          <span className="dot pulse" />
          <span className="text">錄音中 {recElapsed}s</span>
          <LevelMeter level={audioLevel} compact />
          <span className="hint"><kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Esc</kbd> 取消</span>
        </div>
      )}
      {phase.kind === "transcribing" && (
        <div className="mori-chat-progress thinking">
          <span className="dot spin" />
          <span className="text">轉錄中…</span>
        </div>
      )}
      {phase.kind === "responding" && (
        <div className="mori-chat-progress thinking">
          <span className="dot spin" />
          <span className="text">Mori 思考中…</span>
          <span className="hint"><kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Esc</kbd> 中斷</span>
        </div>
      )}

      {/* ── Bottom input bar ─────────────────────────── */}
      <div className="mori-chat-input">
        <button
          className={`mori-mic-btn ${recording ? "recording" : ""}`}
          onClick={onToggle}
          disabled={pipelineBusy || mode === "background"}
          title={
            mode === "background"
              ? "Mori 在休眠,按上方的太陽按鈕喚醒"
              : recording
              ? "停止錄音"
              : "開始錄音(等同 Ctrl+Alt+Space)"
          }
        >
          {recording ? <IconStop width={16} height={16} /> : <IconMic width={16} height={16} />}
        </button>
        <textarea
          className="mori-chat-textarea"
          placeholder={
            mode === "background"
              ? "Mori 休眠中..."
              : "輸入訊息或貼文章 / Ctrl+Enter 送出"
          }
          value={textInput}
          onChange={(e) => setTextInput(e.target.value)}
          disabled={pipelineBusy}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              onSubmit();
            }
          }}
          rows={1}
        />
        <button
          className="mori-send-btn"
          onClick={onSubmit}
          disabled={pipelineBusy || !textInput.trim()}
        >
          送出
        </button>
      </div>

      {/* ── Status modal ──────────────────────────────── */}
      {showStatus && (
        <div className="mori-modal-backdrop" onClick={() => setShowStatus(false)}>
          <div className="mori-modal mori-status-modal" onClick={(e) => e.stopPropagation()}>
            <div className="mori-modal-header">
              <div className="mori-modal-title">
                <span className="mori-modal-stem">系統狀態</span>
              </div>
              <button className="mori-btn ghost" onClick={() => setShowStatus(false)} title="關閉">
                <IconClose width={14} height={14} />
              </button>
            </div>
            <div className="mori-modal-body">
              <StatusRows
                coreVersion={coreVersion}
                buildInfo={buildInfo}
                chatProvider={chatProvider}
                warmup={warmup}
                hasKey={hasKey}
                lastContext={lastContext}
                convLength={conv.length}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function ChatBubble({ turn }: { turn: ChatTurn }) {
  return (
    <div className={`mori-bubble ${turn.role}`}>
      <span className="role-label">{turn.role === "user" ? "你" : "Mori"}</span>
      <div className="bubble-body">
        <p>{turn.content || <span className="empty">(空)</span>}</p>
        {turn.tools_called.length > 0 && (
          <div className="bubble-tools">
            {turn.tools_called.map((t, i) => (
              <span key={i} className="tool-chip"><IconTool width={11} height={11} /> {t}</span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function LevelMeter({ level, compact = false }: { level: number; compact?: boolean }) {
  const db = level > 0 ? 20 * Math.log10(level) : -90;
  const pct = Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  const quiet = db < -45;
  return (
    <div className={`mori-level-meter ${compact ? "compact" : ""}`}>
      <div className={`fill ${quiet ? "quiet" : ""}`} style={{ width: `${pct}%` }} />
    </div>
  );
}

function StatusRows({
  coreVersion,
  buildInfo,
  chatProvider,
  warmup,
  hasKey,
  lastContext,
  convLength,
}: {
  coreVersion: string;
  buildInfo: BuildInfo | null;
  chatProvider: ChatProviderInfo | null;
  warmup: WarmupState | null;
  hasKey: boolean | null;
  lastContext: { clipboard?: string | null; selected_text?: string | null } | null;
  convLength: number;
}) {
  const Row = ({ label, value, title }: { label: string; value: string; title?: string }) => (
    <div className="mori-status-row">
      <span className="label">{label}</span>
      <span className="value" title={title}>{value}</span>
    </div>
  );
  return (
    <div className="mori-status-rows">
      <Row label="core" value={coreVersion || "..."} />
      <Row
        label="build"
        value={buildInfo ? `${buildInfo.sha}${buildInfo.dirty ? "*" : ""} · ${buildInfo.build_time}` : "..."}
      />
      <Row
        label="chat"
        value={
          chatProvider
            ? `${chatProvider.name} · ${chatProvider.model}${
                chatProvider.name === "ollama"
                  ? warmup === "ready"
                    ? " · 就緒"
                    : warmup === "loading"
                    ? " · 載入中"
                    : warmup === "failed"
                    ? " · 失敗"
                    : ""
                  : hasKey === true
                  ? " · ready"
                  : hasKey === false
                  ? " · no key"
                  : ""
              }`
            : "..."
        }
      />
      <Row
        label="stt"
        value={
          chatProvider?.stt
            ? chatProvider.stt.name === "whisper-local"
              ? `[local] ${(chatProvider.stt.model.split("/").pop() || chatProvider.stt.model)}`
              : `[cloud] ${chatProvider.stt.model}`
            : "..."
        }
      />
      <Row label="history" value={`${convLength} msgs`} />
      <Row
        label="clipboard"
        value={lastContext?.clipboard ? `${lastContext.clipboard.length} 字` : "—"}
        title={lastContext?.clipboard ?? undefined}
      />
      <Row
        label="selection"
        value={lastContext?.selected_text ? `${lastContext.selected_text.length} 字` : "—"}
        title={lastContext?.selected_text ?? undefined}
      />
    </div>
  );
}

export default ChatPanel;
