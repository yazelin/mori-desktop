import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import {
  getCurrentWindow,
  currentMonitor,
  PhysicalPosition,
} from "@tauri-apps/api/window";

// Diagnostic helper: emit a `floating-log` event so the backend's tracing
// subscriber picks it up and we can SSH-grep it. Webview console.log is
// invisible without devtools.
function dlog(...parts: unknown[]) {
  const msg = parts
    .map((p) => (typeof p === "string" ? p : JSON.stringify(p)))
    .join(" ");
  console.log("[floating]", msg);
  emit("floating-log", msg).catch(() => {});
}

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

type Mode = "active" | "background";

type Visual =
  | "sleeping"
  | "idle"
  | "recording"
  | "thinking"
  | "done"
  | "error";

function visualFor(mode: Mode, phase: Phase): Visual {
  if (mode === "background") return "sleeping";
  switch (phase.kind) {
    case "idle":
      return "idle";
    case "recording":
      return "recording";
    case "transcribing":
    case "responding":
      return "thinking";
    case "done":
      return "done";
    case "error":
      return "error";
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

// Path to each sprite under /public/floating/. Vite serves /public at root.
// Spec for the PNG drop-ins: 512×512, transparent BG, character centred
// with ~10% padding. Swap any PNG to update Mori's look.
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

  // Anchor bottom-right on first paint, then leave alone (user can drag).
  useEffect(() => {
    (async () => {
      const w = getCurrentWindow();
      const m = await currentMonitor();
      if (!m) return;
      const size = await w.outerSize();
      const margin = 24;
      const taskbarReserve = 60;
      const x = m.size.width - size.width - margin;
      const y = m.size.height - size.height - margin - taskbarReserve;
      await w.setPosition(new PhysicalPosition(x, y));
    })();
  }, []);

  // Same events the main window subscribes to — Tauri broadcasts to all
  // webviews, no extra IPC needed.
  useEffect(() => {
    dlog("mounting on window:", getCurrentWindow().label);
    invoke<Mode>("current_mode")
      .then((m) => { dlog("initial mode:", m); setMode(m); })
      .catch((e) => dlog("current_mode err:", String(e)));
    invoke<Phase>("current_phase")
      .then((p) => { dlog("initial phase:", p); setPhase(p); })
      .catch((e) => dlog("current_phase err:", String(e)));

    const unlistenMode = listen<Mode>("mode-changed", (e) => {
      dlog("mode-changed:", e.payload);
      setMode(e.payload);
    });
    const unlistenPhase = listen<Phase>("phase-changed", (e) => {
      dlog("phase-changed:", e.payload);
      setPhase(e.payload);
    });
    return () => {
      unlistenMode.then((f) => f());
      unlistenPhase.then((f) => f());
    };
  }, []);

  // Track visual changes so we can confirm the sprite swap actually fires.
  useEffect(() => {
    const v = visualFor(mode, phase);
    dlog("visual ->", v, "src:", SPRITE_SRC[v]);
  }, [mode, phase]);

  // Drag: hand the window-move to Tauri via the raw plugin invoke. The
  // higher-level `getCurrentWindow().startDragging()` JS wrapper is
  // unreliable on GNOME Wayland with transparent decorationless windows
  // — events don't propagate through alpha pixels cleanly. The direct
  // IPC call to `plugin:window|start_dragging` (same approach used in
  // yazelin/AgentPulse) sidesteps that wrapper and works.
  const onMouseDown = (e: React.MouseEvent) => {
    if (e.buttons !== 1) return; // primary button held only
    invoke("plugin:window|start_dragging", { label: "floating" }).catch((err) =>
      dlog("start_dragging failed:", String(err)),
    );
  };

  // Double-click → toggle main window visibility. (Single-click conflicts
  // with drag-detection on borderless windows; double-click is unambiguous.)
  const onDoubleClick = async () => {
    try {
      const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      const main = await WebviewWindow.getByLabel("main");
      if (!main) return;
      const visible = await main.isVisible();
      if (visible) {
        await main.hide();
      } else {
        await main.show();
        await main.setFocus();
      }
    } catch (e) {
      console.error("toggle main from floating failed", e);
    }
  };

  const visual = visualFor(mode, phase);

  return (
    <div
      className={`mori-stage mori-${visual}`}
      onMouseDown={onMouseDown}
      onDoubleClick={onDoubleClick}
      title={`Mori — ${VISUAL_LABEL[visual]}\n拖曳:移動 / 雙擊:切顯示主視窗`}
    >
      {/* Behind the sprite: a state-coloured aura/halo that pulses or spins. */}
      <div className="mori-aura" />
      {/* The sprite itself — swappable PNG, see SPRITE_SRC. The CSS adds
          per-state breathing / pulse / shake without touching the bitmap. */}
      <img
        className="mori-sprite"
        src={SPRITE_SRC[visual]}
        alt={VISUAL_LABEL[visual]}
        draggable={false}
      />
    </div>
  );
}

export default FloatingMori;
