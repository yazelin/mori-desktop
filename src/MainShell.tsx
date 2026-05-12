// 5M: 主視窗 sidebar 包裝。把原本的 App(chat panel)塞進 Chat tab,
// 其他 tab(Profiles / Config / Memory / Skills)目前是 placeholder,
// 等 5L 填內容。
//
// 目的:讓主視窗有足夠的 UI 表面承載未來的 config / memory / skills 編輯,
// 不要再把所有東西堆在小 chat 視窗。
//
// 設計原則:
// - 左側 sidebar 寬 200px,深色塊
// - 主區域填滿剩下空間,內捲 overflow scroll
// - 每個 tab 一個 React component;只 mount 當前選中的 tab,避免重複 IPC

import { useState, type ComponentType, type SVGProps } from "react";
import ChatPanel from "./ChatPanel";
import ProfilesTab from "./tabs/ProfilesTab";
import ConfigTab from "./tabs/ConfigTab";
import MemoryTab from "./tabs/MemoryTab";
import SkillsTab from "./tabs/SkillsTab";
import DepsTab from "./tabs/DepsTab";
import {
  IconChat, IconProfiles, IconConfig, IconMemory, IconSkills, IconDeps,
} from "./icons";

type TabId = "chat" | "profiles" | "config" | "memory" | "skills" | "deps";

type TabDef = {
  id: TabId;
  Icon: ComponentType<SVGProps<SVGSVGElement>>;
  label: string;
  sub: string;
};

const TABS: TabDef[] = [
  { id: "chat",     Icon: IconChat,     label: "對話",     sub: "Chat with Mori" },
  { id: "profiles", Icon: IconProfiles, label: "Profiles", sub: "Voice / Agent" },
  { id: "config",   Icon: IconConfig,   label: "Config",   sub: "config.json" },
  { id: "memory",   Icon: IconMemory,   label: "Memory",   sub: "~/.mori/memory" },
  { id: "skills",   Icon: IconSkills,   label: "Skills",   sub: "Built-in / Shell" },
  { id: "deps",     Icon: IconDeps,     label: "Deps",     sub: "Optional tools" },
];

function MainShell() {
  const [tab, setTab] = useState<TabId>("chat");

  return (
    <div className="mori-shell">
      <aside className="mori-sidebar">
        <div className="mori-sidebar-brand">
          <img className="mori-sidebar-brand-icon" src="/logo.png" alt="Mori" />
          <span className="mori-sidebar-brand-name">Mori</span>
        </div>
        <nav className="mori-sidebar-nav">
          {TABS.map((t) => (
            <button
              key={t.id}
              className={`mori-sidebar-item ${tab === t.id ? "active" : ""}`}
              onClick={() => setTab(t.id)}
              title={t.sub}
            >
              <span className="mori-sidebar-item-icon"><t.Icon /></span>
              <span className="mori-sidebar-item-text">
                <span className="mori-sidebar-item-label">{t.label}</span>
                <span className="mori-sidebar-item-sub">{t.sub}</span>
              </span>
            </button>
          ))}
        </nav>
      </aside>

      <main className="mori-main">
        {tab === "chat" && <ChatPanel />}
        {tab === "profiles" && <ProfilesTab />}
        {tab === "config" && <ConfigTab />}
        {tab === "memory" && <MemoryTab />}
        {tab === "skills" && <SkillsTab />}
        {tab === "deps" && <DepsTab />}
      </main>
    </div>
  );
}

export default MainShell;
