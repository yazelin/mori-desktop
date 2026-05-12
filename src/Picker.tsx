// 5K-1: Profile Picker overlay。
//
// 跟 chat_bubble 視窗一樣的「啟動時 off-screen + 收 event 才出來」pattern。
// 收到 `picker-open` event → 移到螢幕中央 + 抓焦點 + 抓鍵盤(等 Tab / Arrow / Enter / Esc)。
//
// UX:
// - Tab 切「VoiceInput」/「Agent」section
// - ↑↓ 在當前 section 內挑 item
// - Enter 觸發切換 + 關閉
// - Esc 關閉(不切換)
// - 點擊也可選

import { useEffect, useRef, useState, KeyboardEvent as ReactKeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalPosition, currentMonitor } from "@tauri-apps/api/window";

type ProfileEntry = { stem: string; display: string };
type Section = "voice" | "agent";

const WIDTH = 520;
const HEIGHT = 480;

async function centerOnPrimaryMonitor() {
  const win = getCurrentWindow();
  try {
    const mon = await currentMonitor();
    if (!mon) return;
    const scale = mon.scaleFactor || 1;
    const screen_w = mon.size.width / scale;
    const screen_h = mon.size.height / scale;
    const x = Math.round((screen_w - WIDTH) / 2);
    const y = Math.round((screen_h - HEIGHT) / 2);
    await win.setPosition(new LogicalPosition(x, y));
  } catch (e) {
    console.error("[picker] center failed", e);
  }
}

function Picker() {
  const [voice, setVoice] = useState<ProfileEntry[]>([]);
  const [agent, setAgent] = useState<ProfileEntry[]>([]);
  const [section, setSection] = useState<Section>("voice");
  const [voiceIdx, setVoiceIdx] = useState(0);
  const [agentIdx, setAgentIdx] = useState(0);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // 初次 mount:預先抓 profile lists(picker 一打開就有)
  useEffect(() => {
    invoke<ProfileEntry[]>("picker_list_voice_profiles").then(setVoice).catch(console.error);
    invoke<ProfileEntry[]>("picker_list_agent_profiles").then(setAgent).catch(console.error);
  }, []);

  // 訊息監聽 — open
  useEffect(() => {
    const win = getCurrentWindow();
    const unlistenOpen = listen("picker-open", async () => {
      console.log("[picker] open");
      // 重新抓 list(中途使用者新增 profile 也能反映)
      try {
        const [v, a] = await Promise.all([
          invoke<ProfileEntry[]>("picker_list_voice_profiles"),
          invoke<ProfileEntry[]>("picker_list_agent_profiles"),
        ]);
        setVoice(v);
        setAgent(a);
      } catch (e) { console.error(e); }
      setSection("voice");
      setVoiceIdx(0);
      setAgentIdx(0);
      await centerOnPrimaryMonitor();
      await win.setFocus();
      setOpen(true);
      // focus root div 才能接到 keydown
      setTimeout(() => rootRef.current?.focus(), 0);
    });
    return () => { unlistenOpen.then((f) => f()); };
  }, []);

  const close = async () => {
    setOpen(false);
    const win = getCurrentWindow();
    try {
      await win.setPosition(new LogicalPosition(-10000, -10000));
    } catch (e) { console.error("[picker] close move-off failed", e); }
  };

  const confirm = async () => {
    const list = section === "voice" ? voice : agent;
    const idx = section === "voice" ? voiceIdx : agentIdx;
    const entry = list[idx];
    if (!entry) { close(); return; }
    try {
      if (section === "voice") {
        await invoke("picker_switch_voice_profile", { stem: entry.stem });
      } else {
        await invoke("picker_switch_agent_profile", { stem: entry.stem });
      }
    } catch (e) { console.error("[picker] switch failed", e); }
    close();
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!open) return;
    const list = section === "voice" ? voice : agent;
    const idx = section === "voice" ? voiceIdx : agentIdx;
    const setIdx = section === "voice" ? setVoiceIdx : setAgentIdx;

    if (e.key === "Escape") { e.preventDefault(); close(); return; }
    if (e.key === "Enter") { e.preventDefault(); confirm(); return; }
    if (e.key === "Tab") {
      e.preventDefault();
      setSection(section === "voice" ? "agent" : "voice");
      return;
    }
    if (e.key === "ArrowDown" || e.key === "j") {
      e.preventDefault();
      if (list.length > 0) setIdx((idx + 1) % list.length);
      return;
    }
    if (e.key === "ArrowUp" || e.key === "k") {
      e.preventDefault();
      if (list.length > 0) setIdx((idx - 1 + list.length) % list.length);
      return;
    }
    if (e.key === "ArrowLeft" || e.key === "h") {
      e.preventDefault();
      setSection("voice");
      return;
    }
    if (e.key === "ArrowRight" || e.key === "l") {
      e.preventDefault();
      setSection("agent");
      return;
    }
  };

  if (!open) return null;

  const list = section === "voice" ? voice : agent;
  const idx = section === "voice" ? voiceIdx : agentIdx;

  return (
    <div
      ref={rootRef}
      tabIndex={0}
      className="mori-picker-root"
      onKeyDown={onKeyDown}
    >
      <div className="mori-picker-card">
        <div className="mori-picker-header">
          <div
            className={`mori-picker-tab ${section === "voice" ? "active" : ""}`}
            onClick={() => setSection("voice")}
          >
            🎙 VoiceInput
            <span className="mori-picker-tab-count">({voice.length})</span>
          </div>
          <div
            className={`mori-picker-tab ${section === "agent" ? "active" : ""}`}
            onClick={() => setSection("agent")}
          >
            🌳 Agent
            <span className="mori-picker-tab-count">({agent.length})</span>
          </div>
        </div>

        <ul className="mori-picker-list">
          {list.length === 0 && (
            <li className="mori-picker-empty">
              （這個目錄沒有 profile）
            </li>
          )}
          {list.map((entry, i) => (
            <li
              key={entry.stem}
              className={`mori-picker-item ${i === idx ? "selected" : ""}`}
              onClick={() => {
                if (section === "voice") setVoiceIdx(i); else setAgentIdx(i);
              }}
              onDoubleClick={confirm}
            >
              <span className="mori-picker-display">{entry.display}</span>
              <span className="mori-picker-stem">{entry.stem}</span>
            </li>
          ))}
        </ul>

        <div className="mori-picker-footer">
          <kbd>↑↓</kbd> 選 &nbsp; <kbd>Tab</kbd> 切換組 &nbsp;
          <kbd>Enter</kbd> 確認 &nbsp; <kbd>Esc</kbd> 取消
        </div>
      </div>
    </div>
  );
}

export default Picker;
