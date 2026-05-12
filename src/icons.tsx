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

// ⚙️ Config — 齒輪(簡化版,小尺寸不會糊)
export function IconConfig(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2 v3 M12 19 v3 M2 12 h3 M19 12 h3 M4.9 4.9 l2.1 2.1 M17 17 l2.1 2.1 M4.9 19.1 l2.1 -2.1 M17 7 l2.1 -2.1" />
    </svg>
  );
}

// 📓 Memory — 開的書本
export function IconMemory(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M4 5.5 A2.5 2.5 0 0 1 6.5 3 H20 v15 H6.5 A2.5 2.5 0 0 0 4 20.5 z" />
      <path d="M4 20.5 A2.5 2.5 0 0 1 6.5 18 H20" />
      <path d="M8 7 h8 M8 11 h8" />
    </svg>
  );
}

// 🛠️ Skills — 板手(工具)
export function IconSkills(props: SVGProps<SVGSVGElement>) {
  return (
    <svg {...base} {...props}>
      <path d="M14.7 6.3 a4.5 4.5 0 0 0 5.79 5.79 L14.5 18.08 a2.12 2.12 0 0 1 -3 -3 l5.99 -5.99 a4.5 4.5 0 0 0 -2.79 -2.79 z" />
      <path d="M14 10 l-9 9" />
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
