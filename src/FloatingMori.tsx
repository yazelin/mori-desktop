import { CSSProperties, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

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

// 波紋觸發門檻 — 低於這個視為靜音不發波
const RIPPLE_THRESHOLD = 0.04;
// 波紋發射間隔 — 最快多久一次（防止暴雷洗版）
const RIPPLE_MIN_INTERVAL_MS = 180;
// 單個波紋存活時間
const RIPPLE_LIFETIME_MS = 1200;

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
  const [mode, setMode] = useState<Mode>("agent");
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [transient, setTransient] = useState<Visual | null>(null);

  // 5F-3A: 音量驅動的 aura（0.0–1.0，後端 ~30Hz emit）
  const [volume, setVolume] = useState(0);

  // 5F-3A 波紋：音量超過門檻時 spawn 一個 ripple element，CSS animation
  // 自動 fade out。lastRippleAtRef 限流避免每 33ms 都發一個。
  const [ripples, setRipples] = useState<Array<{ id: number; intensity: number }>>([]);
  const lastRippleAtRef = useRef(0);

  // 暫時性 info（有 timeout 會消失）
  const [infoLabel, setInfoLabel] = useState<string | null>(null);
  const [infoKey, setInfoKey] = useState(0);
  const showInfo = (text: string) => {
    setInfoLabel(text);
    setInfoKey((k) => k + 1);
  };

  // 持久性狀態 label（錄音中、轉錄中、處理中）
  const [statusLabel, setStatusLabel] = useState<string | null>(null);

  // 當前 profile 常駐標籤（Alt+N 設定後一直記著，錄音中持續顯示）
  const [currentProfileLabel, setCurrentProfileLabel] = useState<string>("");

  // 5J: 完整 chat bubble(Mori 完整回應 / 完整轉錄)。
  // 跟 infoLabel 不一樣 — infoLabel 是頂端 chip 顯示「切到哪個 profile / 狀態」;
  // chatBubble 在 sprite 下方,可多行 wrap、容納長回應、會撐大 floating window。
  const [chatBubble, setChatBubble] = useState<string | null>(null);
  const chatBubbleRef = useRef<HTMLDivElement | null>(null);

  // ── 初始化 & 事件訂閱 ─────────────────────────────────────────────

  useEffect(() => {
    invoke<Mode>("current_mode").then(setMode).catch(() => {});
    invoke<Phase>("current_phase").then(setPhase).catch(() => {});

    const unlistenMode = listen<Mode>("mode-changed", (e) => setMode(e.payload));
    const unlistenPhase = listen<Phase>("phase-changed", (e) => setPhase(e.payload));

    // 5F-3A: 音量事件（main.rs 在錄音中每 ~33ms emit 一次）
    const unlistenVolume = listen<number>("audio-level", (e) => {
      const v = e.payload;
      setVolume(v);

      // 音量超過門檻 + 距離上一個波紋 > 限流間隔 → 發新波紋
      const now = performance.now();
      if (v >= RIPPLE_THRESHOLD && now - lastRippleAtRef.current >= RIPPLE_MIN_INTERVAL_MS) {
        lastRippleAtRef.current = now;
        const id = now;
        const intensity = amplify(v);
        setRipples((rs) => [...rs, { id, intensity }]);
        // 動畫結束後自動移除
        setTimeout(() => {
          setRipples((rs) => rs.filter((r) => r.id !== id));
        }, RIPPLE_LIFETIME_MS);
      }
    });

    // profile 切換："朋友閒聊 · groq" 格式
    const unlistenProfile = listen<string>("voice-input-profile-switched", (e) => {
      setCurrentProfileLabel(e.payload); // 持久記住
      showInfo(e.payload);               // 短暫顯示
      const t = setTimeout(() => setInfoLabel(null), PROFILE_LABEL_MS);
      return () => clearTimeout(t);
    });

    // 轉錄中 / 處理中狀態（後端 emit，有狀態就持續顯示直到下一個狀態）
    const unlistenStatus = listen<string>("voice-input-status", (e) => {
      setStatusLabel(e.payload);
    });

    return () => {
      unlistenMode.then((f) => f());
      unlistenPhase.then((f) => f());
      unlistenVolume.then((f) => f());
      unlistenProfile.then((f) => f());
      unlistenStatus.then((f) => f());
    };
  }, []);

  // 結束狀態時清掉 statusLabel（"轉錄中" / "處理中" 不應該留在 done 之後）
  useEffect(() => {
    if (phase.kind === "done" || phase.kind === "error" || phase.kind === "idle") {
      setStatusLabel(null);
    }
  }, [phase.kind]);

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

  // ── 5J: 完成後浮動提示 ────────────────────────────────────────
  // - VoiceInput mode: 短轉錄(≤40 字)→ infoLabel(頂端 chip);長轉錄 → chatBubble(下方多行)
  // - Agent mode: Mori 完整回應一律進 chatBubble(下方多行,可滾動,不截斷)

  useEffect(() => {
    if (phase.kind !== "done") return;

    if (mode === "voice_input" && phase.transcript.trim()) {
      const text = phase.transcript.trim();
      // 短文字直接 chip 顯示就好,避免動到 window size
      if (text.length <= 40) {
        showInfo(text);
        const t = setTimeout(() => setInfoLabel(null), TRANSCRIPT_LABEL_MS);
        return () => clearTimeout(t);
      }
      // 長轉錄走 bubble(完整顯示讓使用者驗證 STT)
      setChatBubble(text);
      const t = setTimeout(() => setChatBubble(null), 6000);
      return () => clearTimeout(t);
    }

    if (mode === "agent" && phase.response.trim()) {
      // 5J 修:不截斷,完整 chat bubble 顯示。Bubble 自動 wrap + 過長 scroll。
      setChatBubble(phase.response.trim());
      // 訊息越長給越久時間讀 — 每 30 字 +1 秒,base 5 秒,最多 15 秒
      const dwell = Math.min(15000, 5000 + Math.floor(phase.response.length / 30) * 1000);
      const t = setTimeout(() => setChatBubble(null), dwell);
      return () => clearTimeout(t);
    }
  }, [phase, mode]);

  // 錄音開始時清掉舊的 info label + chat bubble,避免上輪的內容殘留
  useEffect(() => {
    if (phase.kind === "recording") {
      setInfoLabel(null);
      setChatBubble(null);
    }
  }, [phase.kind]);

  // 5J: 動態 resize floating window
  // - 沒 chatBubble → 160×160(只露 sprite)
  // - 有 chatBubble → 360 寬 × 依內容 max 540 高
  // ResizeObserver 監聽 bubble 實際渲染高度,跟著調 window。
  useEffect(() => {
    const win = getCurrentWindow();
    if (!chatBubble) {
      // 收回基本尺寸
      win.setSize(new LogicalSize(160, 160)).catch(() => {});
      return;
    }
    // 預設先給個合理值,ResizeObserver 之後再微調
    const BASE_WIDTH = 360;
    const SPRITE_AREA_HEIGHT = 160;
    const BUBBLE_MAX_HEIGHT = 380;
    const PADDING_FOR_SHADOW = 8;
    win.setSize(new LogicalSize(BASE_WIDTH, SPRITE_AREA_HEIGHT + 120)).catch(() => {});

    if (!chatBubbleRef.current) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const bubbleHeight = entry.contentRect.height;
        const target = Math.min(
          BUBBLE_MAX_HEIGHT,
          Math.ceil(bubbleHeight) + 20, // 20px breathing room
        );
        win
          .setSize(new LogicalSize(BASE_WIDTH, SPRITE_AREA_HEIGHT + target + PADDING_FOR_SHADOW))
          .catch(() => {});
      }
    });
    ro.observe(chatBubbleRef.current);
    return () => ro.disconnect();
  }, [chatBubble]);

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

  // 基底環不再 scale（避免 box-shadow 外溢出視窗被切），只用 --vol 控制
  // ::before 的發光強度。實際的「音量波動」由獨立的 ripple elements 表現。
  const auraStyle: CSSProperties | undefined =
    visual === "recording"
      ? ({ "--vol": amplify(volume).toFixed(3) } as CSSProperties)
      : undefined;

  // 標籤顯示優先序：
  //   infoLabel (時效性訊息：profile 切換 / done 結果) 最優先
  //   → statusLabel (轉錄中 / 處理中)
  //   → recording 中常駐顯示 profile 名稱
  //   → VoiceInput mode idle 時也顯示當前 profile（讓使用者知道現在會用哪個）
  const labelToShow: string | null =
    infoLabel
    ?? statusLabel
    ?? (visual === "recording" && currentProfileLabel ? `🎙 ${currentProfileLabel}` : null)
    ?? (visual === "idle" && mode === "voice_input" && currentProfileLabel ? currentProfileLabel : null);

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

      {/* 5F-3A: 音量波紋層 — 音量超過門檻時 spawn ripple，向外擴散後 fade。
          最大擴張到 145px（< 160px 視窗），不會被切。 */}
      {visual === "recording" &&
        ripples.map((r) => (
          <div
            key={r.id}
            className="mori-ripple"
            style={{ "--ripple-intensity": r.intensity.toFixed(3) } as CSSProperties}
          />
        ))}

      {/* 角色 sprite */}
      <img
        className="mori-sprite"
        src={SPRITE_SRC[visual]}
        alt={VISUAL_LABEL[visual]}
        draggable={false}
      />

      {/* 5J: 頂端 chip — profile 切換 / 狀態 / 短文字。位置在 sprite 上方。 */}
      {labelToShow && (
        <div key={`${labelToShow}-${infoKey}`} className="mori-info-label">
          {labelToShow}
        </div>
      )}

      {/* 5J: 下方完整 chat bubble — Mori 的完整回應 / 長轉錄。
          多行 wrap、可滾動,動態 resize floating window。 */}
      {chatBubble && (
        <div ref={chatBubbleRef} className="mori-chat-bubble">
          {chatBubble}
        </div>
      )}
    </div>
  );
}

export default FloatingMori;
