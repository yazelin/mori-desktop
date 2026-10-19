import { describe, expect, it } from "vitest";
import { chordFromEvent, normalizeChord } from "./hotkeyChord";

const ev = (code: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("chordFromEvent", () => {
  it("組合出預設的錄音熱鍵", () => {
    expect(chordFromEvent(ev("Space", { ctrlKey: true, altKey: true }))).toBe("Ctrl+Alt+Space");
  });
  it("字母與數字鍵", () => {
    expect(chordFromEvent(ev("KeyR", { ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+R");
    expect(chordFromEvent(ev("Digit4", { altKey: true }))).toBe("Alt+4");
  });
  it("方向鍵與 Escape 用外掛吃的名字", () => {
    expect(chordFromEvent(ev("ArrowUp", { ctrlKey: true }))).toBe("Ctrl+Up");
    expect(chordFromEvent(ev("Escape", { ctrlKey: true, altKey: true }))).toBe("Ctrl+Alt+Escape");
  });
  it("Meta 鍵寫成 Super", () => {
    expect(chordFromEvent(ev("KeyV", { metaKey: true }))).toBe("Super+V");
  });
  it("功能鍵可以不帶修飾鍵", () => {
    expect(chordFromEvent(ev("F12"))).toBe("F12");
  });
  it("單獨的一般鍵不接受,避免吃掉正常打字", () => {
    expect(chordFromEvent(ev("KeyA"))).toBeNull();
    expect(chordFromEvent(ev("Space"))).toBeNull();
  });
  it("只按修飾鍵時還沒有結果", () => {
    expect(chordFromEvent(ev("ControlLeft", { ctrlKey: true }))).toBeNull();
    expect(chordFromEvent(ev("AltRight", { altKey: true }))).toBeNull();
  });
  it("不支援的鍵回 null", () => {
    expect(chordFromEvent(ev("NumpadAdd", { ctrlKey: true }))).toBeNull();
  });
});

describe("normalizeChord", () => {
  it("修飾鍵順序與大小寫不影響比對", () => {
    expect(normalizeChord("Alt+Ctrl+P")).toBe(normalizeChord("ctrl+alt+p"));
    expect(normalizeChord("Control+Alt+Space")).toBe(normalizeChord("Ctrl+Alt+Space"));
  });
  it("不同的鍵不相等", () => {
    expect(normalizeChord("Ctrl+Alt+P")).not.toBe(normalizeChord("Ctrl+Alt+O"));
  });
});
