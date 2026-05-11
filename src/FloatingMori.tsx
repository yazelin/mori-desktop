import { CSSProperties, useEffect, useRef, useState } from "react";
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

type Mode = "active" | "voice_input" | "background";

type Visual =
  | "sleeping"
  | "idle"
  | "recording"
  | "thinking"
  | "done"
  | "error";

const TRANSIENT_DURATION_MS: Record<"done" | "error", number> = {
  done: 1500,
  error: 2000,
};

// 轉錄原文泡泡顯示時長
const TRANSCRIPT_LABEL_MS = 3000;
// Profile 名稱顯示時長
const PROFILE_LABEL_MS = 1500;

// 麥克風 RMS 值通常在 0.01–0.20，需要 sqrt + 放大讓效果更明顯
const amplify = (v: number) => Math.sqrt(Math.min(v * 4, 1.0));
// 音量 → aura 縮放：靜音 0.82，大聲 1.18
const volumeToScale = (v: number) => 0.82 + 0.36 * amplify(v);
// 注：ring 設計不再需要 opacity 控制（ring 本身 solid，透明度固定 1.0）

function visualFor(
  mode: Mode,
  phase: Phase,
  transient: Visual | null,
): Visual {
  if (mode === "background") return "sleeping";
  if (transient) return transient;
  switch (phase.kind) {
    case "idle":
      return "idle";
    case "recording":
      return "recording";
    case "transcribing":
    case "responding":
      return "thinking";
    case "done":
    case "error":
      return "idle";
  }
}

const VISUAL_LABEL: Record<Visual, string> = {
  sleeping: "休眠中",
  idle: "在這",
  recording: "聽中",
  thinking: "想中",
  done: "完成",
  error: "出錯",
};

const SPRITE_SRC: Record<Visual, string> = {
  idle: "/floating/mori-idle.png",
  sleeping: "/floating/mori-sleeping.png",
  recording: "/floating/mori-recording.png",
  thinking: "/floating/mori-thinking.png",
  done: "/floating/mori-done.png",
  error: "/floating/mori-error.png",
};

function FloatingMori() {
  const [mode, setMode] = useState<Mode>("active");
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [transient, setTransient] = useState<Visual | null>(null);

  // 5F-3A: 音量驅動的 aura（0.0–1.0，後端 ~30Hz emit）
  const [volume, setVolume] = useState(0);

  // 5F-3B: 轉錄原文泡泡 / 5F-3C: profile 名稱泡泡（同一個 slot，後者優先覆蓋）
  const [infoLabel, setInfoLabel] = useState<string | null>(null);
  // key 用來讓相同文字再次出現時也能觸發 fade-in 動畫
  const [infoKey, setInfoKey] = useState(0);

  const showInfo = (text: string) => {
    setInfoLabel(text);
    setInfoKey((k) => k + 1);
  };

  // ── 初始化 & 事件訂閱 ─────────────────────────────────────────────

  useEffect(() => {
    invoke<Mode>("current_mode").then(setMode).catch(() => {});
    invoke<Phase>("current_phase").then(setPhase).catch(() => {});

    const unlistenMode = listen<Mode>("mode-changed", (e) => setMode(e.payload));
    const unlistenPhase = listen<Phase>("phase-changed", (e) => setPhase(e.payload));

    // 5F-3A: 音量事件（main.rs 在錄音中每 ~33ms emit 一次）
    const unlistenVolume = listen<number>("audio-level", (e) => {
      setVolume(e.payload);
    });

    // 5F-3C: profile 切換事件（PR 3 的 Alt+N 會 emit，現在先接好）
    const unlistenProfile = listen<string>("voice-input-profile-switched", (e) => {
      showInfo(e.payload);
      const t = setTimeout(() => setInfoLabel(null), PROFILE_LABEL_MS);
      return () => clearTimeout(t);
    });

    return () => {
      unlistenMode.then((f) => f());
      unlistenPhase.then((f) => f());
      unlistenVolume.then((f) => f());
      unlistenProfile.then((f) => f());
    };
  }, []);

  // ── transient done / error flash ──────────────────────────────────

  useEffect(() => {
    if (phase.kind === "done") {
      setTransient("done");
      const t = setTimeout(() => setTransient(null), TRANSIENT_DURATION_MS.done);
      return () => clearTimeout(t);
    }
    if (phase.kind === "error") {
      setTransient("error");
      const t = setTimeout(() => setTransient(null), TRANSIENT_DURATION_MS.error);
      return () => clearTimeout(t);
    }
    setTransient(null);
  }, [phase]);

  // ── 5F-3B: 完成後浮動提示 ────────────────────────────────────────
  // - VoiceInput mode: 顯示轉錄原文（確認有沒有聽對）
  // - Active mode (chat): 顯示 Mori 的回應（讓使用者不用看主視窗也能追蹤對話）

  useEffect(() => {
    if (phase.kind !== "done") return;

    if (mode === "voice_input" && phase.transcript.trim()) {
      const MAX = 40;
      const text = phase.transcript.trim();
      showInfo(text.length > MAX ? text.slice(0, MAX - 1) + "…" : text);
      const t = setTimeout(() => setInfoLabel(null), TRANSCRIPT_LABEL_MS);
      return () => clearTimeout(t);
    }

    if (mode === "active" && phase.response.trim()) {
      const MAX = 60;
      const text = phase.response.trim();
      showInfo(text.length > MAX ? text.slice(0, MAX - 1) + "…" : text);
      const t = setTimeout(() => setInfoLabel(null), 4000);
      return () => clearTimeout(t);
    }
  }, [phase, mode]);

  // 錄音開始時清掉舊的 info label，避免上輪的轉錄文字殘留
  useEffect(() => {
    if (phase.kind === "recording") {
      setInfoLabel(null);
    }
  }, [phase.kind]);

  // ── Drag ──────────────────────────────────────────────────────────

  const dragRef = useRef<{ x: number; y: number; armed: boolean } | null>(null);
  const DRAG_THRESHOLD_PX = 4;

  const onMouseDown = (e: React.MouseEvent) => {
    if (e.buttons !== 1) return;
    dragRef.current = { x: e.clientX, y: e.clientY, armed: true };
  };

  const onMouseMove = (e: React.MouseEvent) => {
    const d = dragRef.current;
    if (!d || !d.armed) return;
    const dx = Math.abs(e.clientX - d.x);
    const dy = Math.abs(e.clientY - d.y);
    if (dx > DRAG_THRESHOLD_PX || dy > DRAG_THRESHOLD_PX) {
      d.armed = false;
      invoke("plugin:window|start_dragging", { label: "floating" }).catch(
        (err) => console.error("start_dragging failed", err),
      );
    }
  };

  const onMouseUp = () => { dragRef.current = null; };

  const onDoubleClick = async () => {
    try {
      const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      const main = await WebviewWindow.getByLabel("main");
      if (!main) return;
      const visible = await main.isVisible();
      if (visible) { await main.hide(); }
      else { await main.show(); await main.setFocus(); }
    } catch (e) {
      console.error("toggle main from floating failed", e);
    }
  };

  const visual = visualFor(mode, phase, transient);

  // 5F-3A: 錄音中 aura 由 volume 驅動
  // --vol: CSS 變數供 ::before 發光強度使用（0.0~1.0）
  // transform: scale 控制整體大小（帶動旋轉環一起縮放）
  // animation: none 取消 .mori-aura 本身的 CSS animation（不影響 ::before 的旋轉）
  const auraStyle: CSSProperties | undefined =
    visual === "recording"
      ? ({
          "--vol": amplify(volume).toFixed(3),
          transform: `scale(${volumeToScale(volume)})`,
          animation: "none",
        } as CSSProperties)
      : undefined;

  return (
    <div
      className={`mori-stage mori-${visual}`}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onMouseUp={onMouseUp}
      onDoubleClick={onDoubleClick}
      title={`Mori — ${VISUAL_LABEL[visual]}\n拖曳:移動 / 雙擊:切顯示主視窗`}
    >
      {/* 背景光暈：錄音中由音量驅動；其他狀態 CSS animation */}
      <div className="mori-aura" style={auraStyle} />

      {/* 角色 sprite */}
      <img
        className="mori-sprite"
        src={SPRITE_SRC[visual]}
        alt={VISUAL_LABEL[visual]}
        draggable={false}
      />

      {/* 5F-3B/C: 轉錄原文 / profile 名稱泡泡 */}
      {infoLabel && (
        <div key={infoKey} className="mori-info-label">
          {infoLabel}
        </div>
      )}
    </div>
  );
}

export default FloatingMori;
