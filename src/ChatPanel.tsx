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
import { useTranslation } from "react-i18next";

// 5A-3b: ChatPanel 接的 system message payload(fallback chain 觸發時 backend 推)
type FallbackSystemMessage = {
  kind: "fallback";
  context: "agent" | "voice_input_cleanup";
  failed_provider: string;
  next_provider: string;
  reason: string;
};
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

// 跟 Picker / Profiles tab / ProfileEditor modal 對齊用詞:Agent / VoiceInput / Sleep
const MODE_LABEL: Record<Mode, { Icon: ComponentType<SVGProps<SVGSVGElement>>; label: string }> = {
  agent: { Icon: IconBubble, label: "Agent" },
  voice_input: { Icon: IconKeyboard, label: "VoiceInput" },
  background: { Icon: IconSleep, label: "Sleep" },
};

function ChatPanel() {
  const { t } = useTranslation();
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
  const [sessionType, setSessionType] = useState<string>("");
  const [lastContext, setLastContext] = useState<{
    clipboard?: string | null;
    selected_text?: string | null;
  } | null>(null);
  // 5A-3b: fallback chain 觸發時 backend 推 system message,渲染在 thread 內。
  // 每次 recording / transcribing 重置(新 pipeline 開始時清舊訊息)。
  const [systemMessages, setSystemMessages] = useState<FallbackSystemMessage[]>([]);

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
    invoke<string>("linux_session_type").then(setSessionType).catch(() => {});
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
      // 5A-3b: 新 pipeline 啟動(recording / transcribing)→ 清掉舊 fallback 訊息
      if (e.payload.kind === "recording" || e.payload.kind === "transcribing") {
        setSystemMessages([]);
      }
    });
    const unlistenMode = listen<Mode>("mode-changed", (e) => setMode(e.payload));
    const unlistenAudio = listen<number>("audio-level", (e) => setAudioLevel(e.payload));
    const unlistenWarmup = listen<WarmupState>("ollama-warmup", (e) => setWarmup(e.payload));
    const unlistenCtx = listen<typeof lastContext>("context-captured", (e) =>
      setLastContext(e.payload),
    );
    // 5A-3b: fallback chain 觸發時 backend emit 一條 system message
    const unlistenSys = listen<FallbackSystemMessage>("chat-system-message", (e) => {
      setSystemMessages((prev) => [...prev, e.payload]);
    });

    return () => {
      unlistenPhase.then((f) => f());
      unlistenMode.then((f) => f());
      unlistenAudio.then((f) => f());
      unlistenWarmup.then((f) => f());
      unlistenCtx.then((f) => f());
      unlistenSys.then((f) => f());
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
            title={mode === "background" ? t("chat_panel.wake_title") : t("chat_panel.sleep_title")}
          >
            {mode === "background" ? <IconSun width={16} height={16} /> : <IconSleep width={16} height={16} />}
          </button>
          <button
            className="mori-icon-btn"
            onClick={onReset}
            disabled={conv.length === 0}
            title={t("chat_panel.clear_title")}
          >
            <IconRefresh width={16} height={16} />
          </button>
          <button
            className="mori-icon-btn"
            onClick={() => setShowStatus(true)}
            title={t("chat_panel.status_title")}
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
              {" "}{t("chat_panel.greeting_hint")}
            </p>
          </div>
        )}
        {conv.map((turn, i) => (
          <ChatBubble key={i} turn={turn} />
        ))}
        {inProgress && <ChatBubble turn={inProgress} />}
        {/* 5A-3b: fallback chain 觸發時的提示行(每次 pipeline 開始清空) */}
        {systemMessages.map((sm, i) => (
          <div key={`sys-${i}`} className="mori-chat-system">
            <span className="label">
              <IconWarning width={12} height={12} /> {sm.context === "agent" ? "Agent" : "VoiceInput"} fallback
            </span>
            <p>
              <code>{sm.failed_provider}</code>{t("chat_panel.fallback_msg_part_a")}<code>{sm.next_provider}</code>{t("chat_panel.fallback_msg_part_b")}<span className="dim">{sm.reason}</span>
            </p>
          </div>
        ))}
        {phase.kind === "error" && (
          <div className="mori-chat-error">
            <span className="label"><IconWarning width={13} height={13} /> {t("chat_panel.error_label")}</span>
            <p>{phase.message}</p>
          </div>
        )}
      </div>

      {/* ── In-progress chip ──────────────────────────── */}
      {recording && (
        <div className="mori-chat-progress recording">
          <span className="dot pulse" />
          <span className="text">{t("chat_panel.recording")} {recElapsed}s</span>
          <LevelMeter level={audioLevel} compact />
          <span className="hint"><kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Esc</kbd> {t("chat_panel.cancel_hint")}</span>
        </div>
      )}
      {phase.kind === "transcribing" && (
        <div className="mori-chat-progress thinking">
          <span className="dot spin" />
          <span className="text">{t("chat_panel.transcribing")}</span>
        </div>
      )}
      {phase.kind === "responding" && (
        <div className="mori-chat-progress thinking">
          <span className="dot spin" />
          <span className="text">{t("chat_panel.thinking")}</span>
          <span className="hint"><kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Esc</kbd> {t("chat_panel.interrupt_hint")}</span>
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
              ? t("chat_panel.sleeping_rec_title")
              : recording
              ? t("chat_panel.stop_rec_title")
              : t("chat_panel.start_rec_title")
          }
        >
          {recording ? <IconStop width={16} height={16} /> : <IconMic width={16} height={16} />}
        </button>
        <textarea
          className="mori-chat-textarea"
          placeholder={
            mode === "background"
              ? t("chat_panel.sleeping_input_placeholder")
              : t("chat_panel.type_placeholder")
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
          {t("chat_panel.send_button")}
        </button>
      </div>

      {/* ── Status modal ──────────────────────────────── */}
      {showStatus && (
        <div className="mori-modal-backdrop" onClick={() => setShowStatus(false)}>
          <div className="mori-modal mori-status-modal" onClick={(e) => e.stopPropagation()}>
            <div className="mori-modal-header">
              <div className="mori-modal-title">
                <span className="mori-modal-stem">{t("chat_panel.status_modal_title")}</span>
              </div>
              <button className="mori-btn ghost" onClick={() => setShowStatus(false)} title={t("chat_panel.close_title")}>
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
                sessionType={sessionType}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function ChatBubble({ turn }: { turn: ChatTurn }) {
  const { t } = useTranslation();
  return (
    <div className={`mori-bubble ${turn.role}`}>
      <span className="role-label">{turn.role === "user" ? t("chat_panel.role_user") : "Mori"}</span>
      <div className="bubble-body">
        <p>{turn.content || <span className="empty">{t("chat_panel.role_empty")}</span>}</p>
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
  sessionType,
}: {
  coreVersion: string;
  buildInfo: BuildInfo | null;
  chatProvider: ChatProviderInfo | null;
  warmup: WarmupState | null;
  hasKey: boolean | null;
  lastContext: { clipboard?: string | null; selected_text?: string | null } | null;
  convLength: number;
  sessionType: string;
}) {
  const { t } = useTranslation();
  // Mori 偵測到的 session type → 對應的 hotkey path,讓使用者報 bug 時
  // 一眼看出走的是哪條(plugin / portal),不用翻 log。
  const sessionPath = (() => {
    switch (sessionType) {
      case "x11":
        return t("chat_panel.session_path_x11");
      case "wayland":
        return t("chat_panel.session_path_wayland");
      case "linux-other":
        return t("chat_panel.session_path_linux_other");
      case "non-linux":
        return t("chat_panel.session_path_other");
      default:
        return sessionType || "...";
    }
  })();
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
      <Row label="session" value={sessionPath} title={t("chat_panel.session_path_title")} />
      <Row
        label="chat"
        value={
          chatProvider
            ? `${chatProvider.name} · ${chatProvider.model}${
                chatProvider.name === "ollama"
                  ? warmup === "ready"
                    ? t("chat_panel.warmup_ready")
                    : warmup === "loading"
                    ? t("chat_panel.warmup_loading")
                    : warmup === "failed"
                    ? t("chat_panel.warmup_failed")
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
        value={lastContext?.clipboard ? `${lastContext.clipboard.length} ${t("chat_panel.chars_suffix")}` : "—"}
        title={lastContext?.clipboard ?? undefined}
      />
      <Row
        label="selection"
        value={lastContext?.selected_text ? `${lastContext.selected_text.length} ${t("chat_panel.chars_suffix")}` : "—"}
        title={lastContext?.selected_text ?? undefined}
      />
    </div>
  );
}

export default ChatPanel;
