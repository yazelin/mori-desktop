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

import { useEffect, useState, type ComponentType, type SVGProps } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import ChatPanel from "./ChatPanel";
import ProfilesTab from "./tabs/ProfilesTab";
import ConfigTab from "./tabs/ConfigTab";
import MemoryTab from "./tabs/MemoryTab";
import AnnuliTab from "./tabs/AnnuliTab";
import SkillsTab from "./tabs/SkillsTab";
import DepsTab from "./tabs/DepsTab";
import {
  IconChat, IconProfiles, IconConfig, IconMemory, IconAnnuli, IconSkills, IconDeps,
  IconSun, IconMoon,
} from "./icons";
import { toggleTheme, loadActiveTheme } from "./theme";

type NavPayload = { tab: TabId; subTab?: string };

type TabId = "chat" | "profiles" | "config" | "memory" | "annuli" | "skills" | "deps";

type TabDef = {
  id: TabId;
  Icon: ComponentType<SVGProps<SVGSVGElement>>;
  /** i18n key suffix:`sidebar.<key>` 對應 label,`sidebar.<key>_sub` 對應副標 */
  key: string;
};

const TABS: TabDef[] = [
  { id: "chat",     Icon: IconChat,     key: "chat" },
  { id: "profiles", Icon: IconProfiles, key: "profiles" },
  { id: "config",   Icon: IconConfig,   key: "config" },
  { id: "memory",   Icon: IconMemory,   key: "memory" },
  { id: "annuli",   Icon: IconAnnuli,   key: "annuli" },
  { id: "skills",   Icon: IconSkills,   key: "skills" },
  { id: "deps",     Icon: IconDeps,     key: "deps" },
];

function MainShell() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<TabId>("chat");
  // 跨 tab 導航:接到 "mori-nav" event 時切 tab,如果帶 subTab 就傳給子 tab 套用
  const [pendingSubTab, setPendingSubTab] = useState<string | null>(null);
  // brand-3: theme base 給 toggle button 判斷該秀 sun 還是 moon
  const [themeBase, setThemeBase] = useState<"dark" | "light">("dark");

  useEffect(() => {
    const u = listen<NavPayload>("mori-nav", (e) => {
      setTab(e.payload.tab);
      setPendingSubTab(e.payload.subTab ?? null);
    });
    return () => {
      u.then((fn) => fn()).catch(() => {});
    };
  }, []);

  // 啟動時把 active theme base 同步到 state(避免 toggle 圖示對不上)
  useEffect(() => {
    loadActiveTheme().then((res) => {
      if (res) setThemeBase(res[1].base);
    });
    // 監聽 data-theme-base 變化(從 Config tab 切 theme 也要同步)
    const obs = new MutationObserver(() => {
      const base = document.documentElement.getAttribute("data-theme-base");
      if (base === "dark" || base === "light") setThemeBase(base);
    });
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme-base"] });
    return () => obs.disconnect();
  }, []);

  const handleToggle = async () => {
    try {
      const [, theme] = await toggleTheme();
      setThemeBase(theme.base);
    } catch (e) {
      console.error("[shell] toggle theme failed", e);
    }
  };

  return (
    <div className="mori-shell">
      <aside className="mori-sidebar">
        <div className="mori-sidebar-brand">
          <img className="mori-sidebar-brand-icon" src="/logo.png" alt="Mori" />
          <span className="mori-sidebar-brand-name">Mori</span>
        </div>
        <nav className="mori-sidebar-nav">
          {TABS.map((tab_def) => {
            const label = t(`sidebar.${tab_def.key}`);
            const sub = t(`sidebar.${tab_def.key}_sub`);
            return (
              <button
                key={tab_def.id}
                className={`mori-sidebar-item ${tab === tab_def.id ? "active" : ""}`}
                onClick={() => setTab(tab_def.id)}
                title={sub}
              >
                <span className="mori-sidebar-item-icon"><tab_def.Icon /></span>
                <span className="mori-sidebar-item-text">
                  <span className="mori-sidebar-item-label">{label}</span>
                  <span className="mori-sidebar-item-sub">{sub}</span>
                </span>
              </button>
            );
          })}
        </nav>
        <button
          className="mori-sidebar-theme-toggle"
          onClick={handleToggle}
          title={themeBase === "dark" ? "切到亮色" : "切到深色"}
        >
          <span className="mori-sidebar-item-icon">
            {themeBase === "dark" ? <IconSun /> : <IconMoon />}
          </span>
          <span className="mori-sidebar-theme-label">
            {themeBase === "dark" ? "Light" : "Dark"}
          </span>
        </button>
      </aside>

      <main className="mori-main">
        {tab === "chat" && <ChatPanel />}
        {tab === "profiles" && <ProfilesTab />}
        {tab === "config" && (
          <ConfigTab
            pendingSubTab={pendingSubTab}
            onSubTabApplied={() => setPendingSubTab(null)}
          />
        )}
        {tab === "memory" && <MemoryTab />}
        {tab === "annuli" && <AnnuliTab />}
        {tab === "skills" && <SkillsTab />}
        {tab === "deps" && <DepsTab />}
      </main>
    </div>
  );
}

export default MainShell;
