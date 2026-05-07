import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  getCurrentWindow,
  currentMonitor,
  PhysicalPosition,
} from "@tauri-apps/api/window";

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

function FloatingMori() {
  const [mode, setMode] = useState<Mode>("active");
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });

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

  const onClick = async () => {
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
      data-tauri-drag-region
      onClick={onClick}
      title={`Mori — ${VISUAL_LABEL[visual]}\nclick 切顯示主視窗,長按拖曳`}
    >
      {/* 外層:狀態指示環(脈動 / 旋轉 / 暈染) */}
      <div className="mori-aura" />
      {/* 主體:Mori 雙圓 logo */}
      <svg
        className="mori-logo"
        viewBox="0 0 100 100"
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          {/* 漸層讓 logo 有立體感 */}
          <radialGradient id="moriDark" cx="35%" cy="35%" r="65%">
            <stop offset="0%" stopColor="#5a8a6e" />
            <stop offset="60%" stopColor="#2d5a3f" />
            <stop offset="100%" stopColor="#1a3a28" />
          </radialGradient>
          <radialGradient id="moriLight" cx="35%" cy="35%" r="65%">
            <stop offset="0%" stopColor="#e8d8be" />
            <stop offset="60%" stopColor="#c8b896" />
            <stop offset="100%" stopColor="#9a8a72" />
          </radialGradient>
        </defs>
        {/* 雙圓 yin-yang 風格 — 精靈與森林互抱 */}
        <circle cx="36" cy="50" r="30" fill="url(#moriDark)" />
        <circle cx="64" cy="50" r="30" fill="url(#moriLight)" opacity="0.85" />
        {/* 中心一片葉子當焦點(品牌符號) */}
        <path
          className="mori-leaf"
          d="M 50 35 Q 56 50 50 65 Q 44 50 50 35 Z"
          fill="#6b8e5a"
          stroke="#2d5a3f"
          strokeWidth="0.8"
        />
      </svg>
    </div>
  );
}

export default FloatingMori;
