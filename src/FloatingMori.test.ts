import { describe, expect, it } from "vitest";
import { spriteStyle, visualFor } from "./FloatingMori";

const manifest = {
  schema_version: "1.2",
  package_name: "mori",
  display_name: "Mori",
  states: ["idle", "recording", "thinking", "done", "error"],
  sprite_spec: {
    format: "png",
    grid: "4x4",
    total_size: "1024x1024",
    frame_size: "256x256",
    frame_order: "row-major",
    background: "transparent",
  },
  loop_durations_ms: {
    idle: 1200,
  },
};

describe("Floating Mori visual state", () => {
  it("keeps background mode sleeping regardless of active phase", () => {
    expect(visualFor("background", { kind: "recording", started_at_ms: 1 }, null, false, false)).toBe("sleeping");
  });

  it("prioritizes transient and drag states before phase visuals", () => {
    expect(visualFor("agent", { kind: "idle" }, "done", false, true)).toBe("done");
    expect(visualFor("agent", { kind: "idle" }, null, false, true)).toBe("dragging");
  });

  it("uses walking only for idle wander mode", () => {
    expect(visualFor("agent", { kind: "idle" }, null, true, false)).toBe("walking");
    expect(visualFor("agent", { kind: "responding", transcript: "hi" }, null, true, false)).toBe("thinking");
  });
});

describe("Floating Mori sprite style", () => {
  it("falls back to a static public sprite before IPC sprite data arrives", () => {
    expect(spriteStyle("idle", undefined, manifest, true)).toMatchObject({
      backgroundImage: 'url("/floating/mori-idle.png")',
      backgroundSize: "100% 100%",
      backgroundRepeat: "no-repeat",
    });
  });

  it("freezes 4x4 sprites on frame one when animation is disabled", () => {
    expect(spriteStyle("idle", "data:image/png;base64,abc", manifest, false)).toMatchObject({
      backgroundImage: 'url("data:image/png;base64,abc")',
      backgroundSize: "400% 400%",
      backgroundPosition: "0% 0%",
      backgroundRepeat: "no-repeat",
    });
  });

  it("animates 4x4 sprites with paired x/y keyframes when enabled", () => {
    expect(spriteStyle("idle", "data:image/png;base64,abc", manifest, true)).toMatchObject({
      animationName: "mori-sprite-x, mori-sprite-y",
      animationDuration: "300ms, 1200ms",
      animationTimingFunction: "steps(4, jump-none), steps(4, jump-none)",
      animationIterationCount: "infinite, infinite",
    });
  });
});
