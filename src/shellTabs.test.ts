import { describe, expect, it } from "vitest";
import { coerceVisibleNavPayload, isVisibleTabId, SHELL_TAB_IDS } from "./shellTabs";

describe("shell tab gate", () => {
  it("keeps the public sidebar tab list free of internal self-dev", () => {
    expect(SHELL_TAB_IDS).not.toContain("self_dev");
  });

  it("accepts visible navigation payloads and normalizes missing subTab", () => {
    expect(coerceVisibleNavPayload({ tab: "config" })).toEqual({
      tab: "config",
      subTab: null,
    });
    expect(coerceVisibleNavPayload({ tab: "config", subTab: "voice" })).toEqual({
      tab: "config",
      subTab: "voice",
    });
  });

  it("rejects hidden self-dev navigation events", () => {
    expect(coerceVisibleNavPayload({ tab: "self_dev" })).toBeNull();
  });

  it("guards arbitrary string tab ids before state updates", () => {
    expect(isVisibleTabId("chat")).toBe(true);
    expect(isVisibleTabId("unknown")).toBe(false);
  });
});
