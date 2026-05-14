// brand-2: sidebar tab icon — line-art SVG（stroke = currentColor）。
//
// 風格規則:
// - viewBox 0 0 24 24,stroke=1.5,linecap/linejoin round
// - 全部 stroke 不 fill,跟 logo cream-stroke 描邊一致
// - color 從 CSS 繼承(.mori-sidebar-item / .mori-sidebar-item.active)

import type { SVGProps } from "react";

const base: SVGProps<SVGSVGElement> = {
  width: 18,
  height: 18,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round",
  strokeLinejoin: "round",
};

// 💬 Chat — 對話泡泡 + 三點
export function IconChat(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M4 6 a2 2 0 0 1 2 -2 h12 a2 2 0 0 1 2 2 v8 a2 2 0 0 1 -2 2 H10 l-4 4 v-4 H6 a2 2 0 0 1 -2 -2 z" />
      <circle cx="8.5" cy="10" r="0.6" fill="currentColor" stroke="none" />
      <circle cx="12" cy="10" r="0.6" fill="currentColor" stroke="none" />
      <circle cx="15.5" cy="10" r="0.6" fill="currentColor" stroke="none" />
    </svg>
  );
}

// 📋 Profiles — 人 + 列表(voice / agent profile)
export function IconProfiles(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <circle cx="7.5" cy="8" r="2.8" />
      <path d="M3 19 a4.5 4.5 0 0 1 9 0" />
      <path d="M14 7 h7" />
      <path d="M14 12 h7" />
      <path d="M14 17 h5" />
    </svg>
  );
}

// ⚙️ Config — 齒輪。Lucide 風格,有齒形輪廓不是太陽光芒。
// 6 個 chunky 齒突繞著中心圓,每個齒包進 outer ring 一起連成完整外形。
export function IconConfig(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M12.22 2 h-.44 a2 2 0 0 0 -2 2 v.18 a2 2 0 0 1 -1 1.73 l-.43 .25 a2 2 0 0 1 -2 0 l-.15 -.08 a2 2 0 0 0 -2.73 .73 l-.22 .38 a2 2 0 0 0 .73 2.73 l.15 .1 a2 2 0 0 1 1 1.72 v.51 a2 2 0 0 1 -1 1.74 l-.15 .09 a2 2 0 0 0 -.73 2.73 l.22 .38 a2 2 0 0 0 2.73 .73 l.15 -.08 a2 2 0 0 1 2 0 l.43 .25 a2 2 0 0 1 1 1.73 V20 a2 2 0 0 0 2 2 h.44 a2 2 0 0 0 2 -2 v-.18 a2 2 0 0 1 1 -1.73 l.43 -.25 a2 2 0 0 1 2 0 l.15 .08 a2 2 0 0 0 2.73 -.73 l.22 -.39 a2 2 0 0 0 -.73 -2.73 l-.15 -.08 a2 2 0 0 1 -1 -1.74 v-.5 a2 2 0 0 1 1 -1.74 l.15 -.09 a2 2 0 0 0 .73 -2.73 l-.22 -.38 a2 2 0 0 0 -2.73 -.73 l-.15 .08 a2 2 0 0 1 -2 0 l-.43 -.25 a2 2 0 0 1 -1 -1.73 V4 a2 2 0 0 0 -2 -2 z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

// 📓 Memory — 螺旋筆記本 + 精靈閃光。左側裝訂環 + 內頁橫線書寫,
// 右上角內疊一個小 sparkle ✨(在書本範圍內,不破壞置中構圖)。
export function IconMemory(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      {/* 筆記本邊框(置中)*/}
      <rect x="5" y="3" width="15" height="18" rx="1.5" />
      {/* 左側裝訂環(4 個小圓) */}
      <circle cx="7.5" cy="6.5" r="0.7" fill="currentColor" stroke="none" />
      <circle cx="7.5" cy="10" r="0.7" fill="currentColor" stroke="none" />
      <circle cx="7.5" cy="13.5" r="0.7" fill="currentColor" stroke="none" />
      <circle cx="7.5" cy="17" r="0.7" fill="currentColor" stroke="none" />
      {/* 內頁書寫橫線(第一條讓位給右上 sparkle) */}
      <path d="M10.5 7 h4" />
      <path d="M10.5 11 h7" />
      <path d="M10.5 15 h5" />
      {/* 精靈 sparkle(疊在右上角,在 notebook 範圍內) */}
      <path d="M17.5 5 v3 M16 6.5 h3" />
    </svg>
  );
}

// 🌳 Annuli — 樹(年輪),vault 反思
export function IconAnnuli(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      {/* 3 圈年輪 */}
      <circle cx="12" cy="12" r="3" />
      <circle cx="12" cy="12" r="6.5" />
      <circle cx="12" cy="12" r="10" />
    </svg>
  );
}

// 🪄 Skills — 法杖(magic wand)。
// 斜立的法杖,杖尖在右上爆出 4-point 星光(skill 從這裡釋放);
// 法杖周圍飄 2 顆小 sparkle dust = 魔法殘留塵。完全不會被看成人形。
export function IconSkills(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      {/* 法杖本體(從左下斜向右上) */}
      <path d="M4 20 L15 9" />
      {/* 杖尖星光(4-point sparkle 在右上) */}
      <path d="M16 4 V12" />
      <path d="M12 8 H20" />
      {/* 第二顆魔法塵(右下) */}
      <path d="M19 15 v2 M18 16 h2" />
      {/* 第三顆魔法塵(左上) */}
      <path d="M7 5 v1.5 M6.25 5.75 h1.5" />
    </svg>
  );
}

// 📦 Deps — 包裹箱
export function IconDeps(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M21 16 V8 a2 2 0 0 0 -1 -1.73 l-7 -4 a2 2 0 0 0 -2 0 l-7 4 A2 2 0 0 0 3 8 v8 a2 2 0 0 0 1 1.73 l7 4 a2 2 0 0 0 2 0 l7 -4 A2 2 0 0 0 21 16 z" />
      <path d="M3.3 7 L12 12 l8.7 -5" />
      <path d="M12 22 V12" />
      <path d="M7.5 4.2 L16.5 9.4" />
    </svg>
  );
}

// ☀️ Sun — light theme indicator
export function IconSun(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2 v2 M12 20 v2 M4.93 4.93 l1.41 1.41 M17.66 17.66 l1.41 1.41 M2 12 h2 M20 12 h2 M6.34 17.66 l-1.41 1.41 M19.07 4.93 l-1.41 1.41" />
    </svg>
  );
}

// 🌙 Moon — dark theme indicator
export function IconMoon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M21 12.79 A9 9 0 1 1 11.21 3 A7 7 0 0 0 21 12.79 z" />
    </svg>
  );
}

// 🌐 Globe — language switcher
export function IconGlobe(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <circle cx="12" cy="12" r="10" />
      <path d="M2 12 h20" />
      <path d="M12 2 a15 15 0 0 1 0 20 a15 15 0 0 1 0 -20" />
    </svg>
  );
}

// 🎚 Equalizer — 4 條 vertical bar,有 CSS animation 跳動(audio 播時),
// 靜音時三條都縮短不跳。class .playing 控制動畫狀態。
export function IconEqualizer({ playing = false, ...props }: SVGProps<SVGSVGElement> & { playing?: boolean }) {
  return (
    <svg {...base} {...props} className={`mori-icon-eq ${playing ? "playing" : ""}`}>
      <rect x="4" y="10" width="2.5" height="4" rx="1" className="eq-bar eq-bar-1" />
      <rect x="9" y="8" width="2.5" height="8" rx="1" className="eq-bar eq-bar-2" />
      <rect x="14" y="6" width="2.5" height="12" rx="1" className="eq-bar eq-bar-3" />
      <rect x="19" y="10" width="2.5" height="4" rx="1" className="eq-bar eq-bar-4" />
    </svg>
  );
}

// ❓ Help — 圓圈內問號(打開 Quickstart 引導)
export function IconHelp(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <circle cx="12" cy="12" r="10" />
      <path d="M9.09 9 a3 3 0 0 1 5.83 1 c0 2 -3 3 -3 3" />
      <path d="M12 17 h.01" />
    </svg>
  );
}

// ─── brand-3 batch 2: inline action icons(取代 chat / dep / skill emoji)──────

// ✕ Close
export function IconClose(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M6 6 L18 18 M18 6 L6 18" />
    </svg>
  );
}

// ✓ Check
export function IconCheck(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M4 12 L10 18 L20 6" />
    </svg>
  );
}

// ⚠ Warning triangle
export function IconWarning(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M12 3 L22 20 H2 z" />
      <path d="M12 10 v5" />
      <circle cx="12" cy="18" r="0.6" fill="currentColor" stroke="none" />
    </svg>
  );
}

// 🔄 Refresh — 圓弧 + 箭頭
export function IconRefresh(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M21 12 a9 9 0 1 1 -3.2 -6.9" />
      <path d="M21 4 v5 h-5" />
    </svg>
  );
}

// 🎤 Mic
export function IconMic(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <rect x="9" y="3" width="6" height="12" rx="3" />
      <path d="M5 11 a7 7 0 0 0 14 0" />
      <path d="M12 18 v3" />
    </svg>
  );
}

// ■ Stop(錄音中按鈕)
export function IconStop(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <rect x="6" y="6" width="12" height="12" rx="2" />
    </svg>
  );
}

// ⌨ Keyboard(語音輸入 mode)
export function IconKeyboard(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <path d="M6 10 h0 M10 10 h0 M14 10 h0 M18 10 h0 M6 14 h12" />
    </svg>
  );
}

// 💤 Sleep(休眠模式)— 三條斜線
export function IconSleep(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M6 6 h6 L6 14 h6 M14 10 h4 L14 16 h4" />
    </svg>
  );
}

// 👋 Wave — 用 hand symbol(簡化:4 指 + 拇指 outline)
export function IconWave(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M10 12 V5 a1.5 1.5 0 0 1 3 0 v6" />
      <path d="M13 11 V4 a1.5 1.5 0 0 1 3 0 v7" />
      <path d="M16 11 V6 a1.5 1.5 0 0 1 3 0 v8 a7 7 0 0 1 -7 7 h-1 a7 7 0 0 1 -6.5 -4.5 L2 11 a1.5 1.5 0 0 1 2.5 -1.5 L7 12 V6 a1.5 1.5 0 0 1 3 0" />
    </svg>
  );
}

// 🔧 Wrench tool(chat tool chip)— 重用 Skills 但較小
export function IconTool(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M14.7 6.3 a4.5 4.5 0 0 0 5.79 5.79 L14.5 18.08 a2.12 2.12 0 0 1 -3 -3 z" />
    </svg>
  );
}

// 📋 Clipboard
export function IconClipboard(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <rect x="6" y="4" width="12" height="18" rx="2" />
      <rect x="9" y="2" width="6" height="4" rx="1" />
    </svg>
  );
}

// 🖱 Mouse pointer
export function IconPointer(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M4 3 L14 13 H9 L11.5 19 L9.5 20 L7 14 L3 17 z" />
    </svg>
  );
}

// 🏠 Home(local LLM)
export function IconHome(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M3 11 L12 3 L21 11 V20 a2 2 0 0 1 -2 2 h-4 v-7 h-6 v7 H5 a2 2 0 0 1 -2 -2 z" />
    </svg>
  );
}

// ☁ Cloud(cloud LLM)
export function IconCloud(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M18 16 a4 4 0 0 0 -1.2 -7.8 a6 6 0 0 0 -11.5 1.7 A4 4 0 0 0 6 17 H18 a3.5 3.5 0 0 0 0 -7" />
    </svg>
  );
}

// ⚡ Lightning(處理中)
export function IconLightning(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M13 2 L4 14 h7 L9 22 L20 10 H13 z" />
    </svg>
  );
}

// 📝 Pencil note(轉錄中)
export function IconPencil(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M12 20 H20" />
      <path d="M16.5 3.5 a2.12 2.12 0 0 1 3 3 L7 19 L3 20 L4 16 z" />
    </svg>
  );
}

// 🌳 Tree(Agent profile section)— 跟 logo wreath 呼應
export function IconTree(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M12 3 L7 9 H10 L6 14 H10 L4 21 H20 L14 14 H18 L14 9 H17 z" />
      <path d="M12 21 v-4" />
    </svg>
  );
}

// 🎙 Voice profile mic w/ stand — 跟 IconMic 區分
export function IconVoiceMic(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <rect x="9" y="3" width="6" height="10" rx="3" />
      <circle cx="12" cy="6" r="0.6" fill="currentColor" stroke="none" />
      <path d="M5 11 a7 7 0 0 0 14 0" />
      <path d="M12 18 v3 M9 21 h6" />
    </svg>
  );
}
