export const SHELL_TAB_IDS = [
  "chat",
  "profiles",
  "config",
  "memory",
  "annuli",
  "skills",
  "body",
  "permissions",
  "transcribe",
  "recordings",
  "corrections",
  "deps",
  "logs",
] as const;

export type TabId = (typeof SHELL_TAB_IDS)[number];
export type HiddenTabId = "self_dev";

export type NavPayload = {
  tab: TabId | HiddenTabId;
  subTab?: string;
};

export type VisibleNavPayload = {
  tab: TabId;
  subTab: string | null;
};

const visibleTabs = new Set<string>(SHELL_TAB_IDS);

export function isVisibleTabId(tab: string): tab is TabId {
  return visibleTabs.has(tab);
}

export function coerceVisibleNavPayload(payload: NavPayload): VisibleNavPayload | null {
  if (!isVisibleTabId(payload.tab)) return null;
  return {
    tab: payload.tab,
    subTab: payload.subTab ?? null,
  };
}
