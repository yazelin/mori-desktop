// brand-3: theme apply 機制。
// 從 Rust IPC 拿 active theme,把 colors map 寫成 document.documentElement
// 的 CSS variables(--c-<key>)。CSS 用 var(--c-xxx) 取。
//
// 切 theme 不重 load 整頁 — overwrite CSS variables 即可(平滑切換)。
// `html` 上的 `data-theme-base` 屬性 = "dark" | "light",給 CSS 做差異化
// (例如 native widget color-scheme、scrollbar tint)。

import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Theme = {
  name: string;
  base: "dark" | "light";
  builtin?: boolean;
  colors: Record<string, string>;
};

export type ThemeEntry = {
  stem: string;
  name: string;
  base: "dark" | "light";
  builtin: boolean;
};

/** 把 theme.colors 寫成 :root 的 --c-* variables,並更新 data-theme-base。 */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  for (const [key, value] of Object.entries(theme.colors)) {
    root.style.setProperty(`--c-${key}`, value);
  }
  root.setAttribute("data-theme-base", theme.base);
  // 給 native widget (select dropdown / scrollbar) 用
  root.style.colorScheme = theme.base;
}

/** 啟動時呼叫:抓 active theme 套上去。
 *
 * v0.4.1:傳 OS 的 `prefers-color-scheme` 給後端當 fallback hint — 沒
 * active_theme 檔(fresh install)時依 OS 設定選 light/dark。已 set 過的
 * 用戶後端會忽略 hint 走自己設定。
 */
export async function loadActiveTheme(): Promise<[string, Theme] | null> {
  try {
    const defaultLight =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-color-scheme: light)").matches === true;
    const [stem, theme] = await invoke<[string, Theme]>("theme_get_active", { defaultLight });
    applyTheme(theme);
    return [stem, theme];
  } catch (e) {
    console.error("[theme] loadActiveTheme failed", e);
    return null;
  }
}

export async function listThemes(): Promise<ThemeEntry[]> {
  return await invoke<ThemeEntry[]>("theme_list");
}

export async function setActiveTheme(stem: string): Promise<Theme> {
  const theme = await invoke<Theme>("theme_set_active", { stem });
  applyTheme(theme);
  await broadcastTheme(theme);
  return theme;
}

/** quick dark <-> light toggle */
export async function toggleTheme(): Promise<[string, Theme]> {
  const [stem, theme] = await invoke<[string, Theme]>("theme_toggle");
  applyTheme(theme);
  await broadcastTheme(theme);
  return [stem, theme];
}

export async function themesDir(): Promise<string> {
  return await invoke<string>("theme_dir");
}

// brand-3: 跨視窗同步 — 任一 window 切 theme 後 emit "theme-changed",
// 其他 window 收到 event 馬上 applyTheme(payload)。
async function broadcastTheme(theme: Theme): Promise<void> {
  try {
    await emit("theme-changed", theme);
  } catch (e) {
    console.error("[theme] broadcast failed", e);
  }
}

/** 在 mount 時呼叫:既 load 一次,也 listen 後續 theme-changed 事件 */
export async function subscribeTheme(): Promise<UnlistenFn> {
  await loadActiveTheme();
  return await listen<Theme>("theme-changed", (e) => {
    applyTheme(e.payload);
  });
}
