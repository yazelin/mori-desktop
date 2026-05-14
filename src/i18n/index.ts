// i18n 基建:react-i18next 初始化 + locale 決策。
//
// 載入順序(高 → 低):
//   1. ~/.mori/config.json 的 `locale` 欄位
//   2. browser/OS navigator.language
//   3. 預設 zh-TW
//
// Proper noun(Mori / Annuli / SOUL.md / MEMORY.md / API key 名等)**不進**
// locale file — 翻譯後變不認得的字串會打破連線、影響 user-vault 一致性。

import { invoke } from "@tauri-apps/api/core";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import zhTW from "./locales/zh-TW.json";

export type Locale = "zh-TW" | "en";
export const DEFAULT_LOCALE: Locale = "zh-TW";
export const SUPPORTED_LOCALES: Locale[] = ["zh-TW", "en"];

function detectLocaleSync(): Locale {
  // navigator.language fallback(同步,給 i18n.init 用)
  const nav = (typeof navigator !== "undefined" && navigator.language) || "";
  if (nav.toLowerCase().startsWith("zh")) return "zh-TW";
  if (nav.toLowerCase().startsWith("en")) return "en";
  return DEFAULT_LOCALE;
}

async function detectLocaleAsync(): Promise<Locale> {
  // 從 config.json 讀(IPC),失敗 fallback navigator
  try {
    const text = await invoke<string>("config_read");
    const parsed = JSON.parse(text);
    const fromConfig = parsed?.locale as string | undefined;
    if (fromConfig && SUPPORTED_LOCALES.includes(fromConfig as Locale)) {
      return fromConfig as Locale;
    }
  } catch {
    // 沒 config / 讀失敗 → 走 navigator
  }
  return detectLocaleSync();
}

// 同步 init(用 navigator),啟動快;之後 async 讀 config.json 覆寫
i18n
  .use(initReactI18next)
  .init({
    resources: {
      "zh-TW": { translation: zhTW },
      en: { translation: en },
    },
    lng: detectLocaleSync(),
    fallbackLng: DEFAULT_LOCALE,
    interpolation: { escapeValue: false }, // React 已經處理 XSS,不用 escape
    returnEmptyString: false, // 空字串 fall back 到 key,debug 時看得到漏抽
  });

/** 啟動 mount 後呼叫:用 config.json 的 locale 覆寫 navigator detect。 */
export async function syncLocaleFromConfig(): Promise<Locale> {
  const locale = await detectLocaleAsync();
  if (locale !== i18n.language) {
    await i18n.changeLanguage(locale);
  }
  return locale;
}

/** Locale switcher 用(ConfigTab → Appearance sub-tab)。寫進 config.json + 即時切換。 */
export async function setLocale(locale: Locale): Promise<void> {
  if (!SUPPORTED_LOCALES.includes(locale)) {
    throw new Error(`unsupported locale: ${locale}`);
  }
  await i18n.changeLanguage(locale);
  // 寫進 config.json(B 階段 ConfigTab 改 form 後會用 saveConfig 統一寫,
  // 這裡只是即時切;持久化由 ConfigTab raw JSON 控)
}

export { i18n };
export default i18n;
