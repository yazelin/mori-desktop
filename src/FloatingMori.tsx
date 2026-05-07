import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  getCurrentWindow,
  currentMonitor,
  PhysicalPosition,
} from "@tauri-apps/api/window";

// Same shape as the main App's Phase + Mode types (kept inline here so the
// floating widget can be moved into its own bundle later if we ever split
// the build per window — right now both windows share `main.tsx` and the
// same dist).

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

// Visual state — a single label so the CSS is one selector per state.
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
      const margin = 20;
      // Account for typical taskbar height (~48–60 px). Leave room.
      const taskbarReserve = 60;
      const x = m.size.width - size.width - margin;
      const y = m.size.height - size.height - margin - taskbarReserve;
      await w.setPosition(new PhysicalPosition(x, y));
    })();
  }, []);

  // Subscribe to backend state. Same event names the main window uses;
  // Tauri broadcasts events to all webview windows, so no extra plumbing.
  useEffect(() => {
    invoke<Mode>("current_mode").then(setMode).catch(() => {});
    invoke<Phase>("current_phase").then(setPhase).catch(() => {});

    const unlistenMode = listen<Mode>("mode-changed", (e) => setMode(e.payload));
    const unlistenPhase = listen<Phase>("phase-changed", (e) => setPhase(e.payload));
    return () => {
      unlistenMode.then((f) => f());
      unlistenPhase.then((f) => f());
    };
  }, []);

  // Click → toggle the main window. Drag → moves the floating widget
  // (handled by the data-tauri-drag-region attribute, which Tauri picks up
  // for borderless windows).
  const onClick = async () => {
    try {
      // We rely on the global Tauri singleton to find the main window.
      // Using the typed api: getAllWindows() — but to keep this widget
      // tiny, just toggle visibility via an IPC the backend already has.
      // Falling back to the WebviewWindow class is also fine.
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
      className={`floating-mori floating-${visual}`}
      data-tauri-drag-region
      onClick={onClick}
      title={`Mori — ${VISUAL_LABEL[visual]}\n(click 切顯示主視窗,長按拖曳)`}
    >
      <div className="floating-eye floating-eye-left" />
      <div className="floating-eye floating-eye-right" />
      <div className="floating-mouth" />
    </div>
  );
}

export default FloatingMori;
