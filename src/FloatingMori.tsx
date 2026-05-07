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

// Done / Error 在 floating widget 上是 transient — 顯示動畫期間後就該回
// idle,不像主視窗會保留結果卡片。後端的 Phase 不變(主視窗仍顯示
// 對話結果),只在 floating 端用 transient state 控制視覺生命週期。
const TRANSIENT_DURATION_MS: Record<"done" | "error", number> = {
  done: 1500,  // 對齊 mori-done-glow keyframe 的 1.6s
  error: 2000, // 抖動 0.5s 後再多停一下,讓使用者有意識到
};

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
      // 過了 transient 時間後 fall-through 回 idle,不卡在 done/error
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
  // Transient visual override — only for done / error so the celebration
  // glow / shake plays once and then we fade back to idle. Cleared by a
  // timer on phase change.
  const [transient, setTransient] = useState<Visual | null>(null);

  // Initial anchor: horizontally centred, ~10% down from the top edge —
  // matches the AgentPulse capsule placement so Mori reads as a "perched
  // companion at the top of the workspace" rather than a corner widget.
  // User can drag elsewhere; we only set this once on mount.
  //
  // Also re-asserts always-on-top after positioning; GNOME mutter on
  // Wayland sometimes drops the flag during the initial geometry dance,
  // so doing it once from JS after we're settled is more reliable than
  // trusting the conf.json `alwaysOnTop: true` alone.
  useEffect(() => {
    (async () => {
      try {
        const w = getCurrentWindow();
        const m = await currentMonitor();
        if (!m) return;

        // tauri.conf.json declares the window 160×160 (logical CSS px).
        // We use the *configured logical size* × monitor scale factor as
        // the position anchor — outerSize() on GNOME Wayland for
        // transparent borderless windows includes a hefty invisible
        // shadow region (we measured 424×504 reported for a 160×160
        // configured window on a 3456×2160 sf:2 monitor), which threw
        // the centring math off by ~80 logical pixels.
        const LOGICAL_W = 160;
        const physW = LOGICAL_W * m.scaleFactor;
        const x = Math.max(0, Math.round((m.size.width - physW) / 2));
        const y = Math.max(0, Math.round(m.size.height * 0.05));

        dlog(
          "anchor: monitor",
          { w: m.size.width, h: m.size.height, sf: m.scaleFactor },
          "→ pos (phys)",
          { x, y },
        );

        try {
          await w.setPosition(new PhysicalPosition(x, y));
          const after = await w.outerPosition();
          dlog("after setPosition outerPosition:", { x: after.x, y: after.y });
        } catch (e) {
          dlog("setPosition failed:", String(e));
        }
        try {
          await w.setAlwaysOnTop(true);
        } catch (e) {
          dlog("setAlwaysOnTop failed:", String(e));
        }
      } catch (outer) {
        dlog("anchor effect threw:", String(outer));
      }
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

  // 進 done / error 時:先設 transient(讓 floating 顯示對應 sprite +
  // glow / shake),動畫結束的時間後 clear → fall-through 回 idle。
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
    // 任何其他 phase 都立刻清掉 transient,避免「上次的 done flash 跑到
    // 下一輪 recording 中閃一下」這種視覺殘留。
    setTransient(null);
  }, [phase]);

  // Track visual changes so we can confirm the sprite swap actually fires.
  useEffect(() => {
    const v = visualFor(mode, phase, transient);
    dlog("visual ->", v, "src:", SPRITE_SRC[v]);
  }, [mode, phase, transient]);

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

  const visual = visualFor(mode, phase, transient);

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
