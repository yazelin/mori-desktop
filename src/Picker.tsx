// 5K-1: Profile Picker overlay (refined 5K-1b)
//
// 設計:始終顯示 3 個 item(prev / current / next),用「轉輪」概念
// 上下方向鍵移動 cursor,項目超多也不會超出視窗、不會被切。
//
// Wayland focus 救援:setFocus() 在 GNOME 下對非 user-activated window 經常被
// 拒絕。改用 document.addEventListener('keydown') + 多次 retry setFocus +
// 立即 focus rootRef,把 chance maximize。

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalPosition, currentMonitor } from "@tauri-apps/api/window";

type ProfileEntry = { stem: string; display: string };
type Section = "voice" | "agent";

const WIDTH = 520;
const HEIGHT = 280; // 縮小:3-item carousel 不需要 480

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
  // 把 state mirror 到 ref 給 document-level keydown listener 用
  // (listener 不會跟 state 更新 re-bind)
  const openRef = useRef(open);
  const sectionRef = useRef(section);
  const voiceRef = useRef(voice);
  const agentRef = useRef(agent);
  const voiceIdxRef = useRef(voiceIdx);
  const agentIdxRef = useRef(agentIdx);
  openRef.current = open;
  sectionRef.current = section;
  voiceRef.current = voice;
  agentRef.current = agent;
  voiceIdxRef.current = voiceIdx;
  agentIdxRef.current = agentIdx;

  // 初次 mount:預先抓 profile lists
  useEffect(() => {
    invoke<ProfileEntry[]>("picker_list_voice_profiles").then(setVoice).catch(console.error);
    invoke<ProfileEntry[]>("picker_list_agent_profiles").then(setAgent).catch(console.error);
  }, []);

  const close = async () => {
    setOpen(false);
    const win = getCurrentWindow();
    // 5K-1c: Wayland 對 hide/show 反覆切換 focus 給不穩,改用「保持 visible 但移
    // off-screen」— 第一次 show() 後 GNOME 把 picker 歸到 Mori-tauri WMClass group
    // (因 skipTaskbar 不堆 dock),之後 setPosition 進/出畫面 focus 穩。
    try {
      await win.setPosition(new LogicalPosition(-10000, -10000));
    } catch (e) { console.error("[picker] move off-screen failed", e); }
  };

  const confirm = async () => {
    const list = sectionRef.current === "voice" ? voiceRef.current : agentRef.current;
    const idx = sectionRef.current === "voice" ? voiceIdxRef.current : agentIdxRef.current;
    const entry = list[idx];
    if (!entry) { close(); return; }
    try {
      if (sectionRef.current === "voice") {
        await invoke("picker_switch_voice_profile", { stem: entry.stem });
      } else {
        await invoke("picker_switch_agent_profile", { stem: entry.stem });
      }
    } catch (e) { console.error("[picker] switch failed", e); }
    close();
  };

  // open / hide handlers
  useEffect(() => {
    const win = getCurrentWindow();
    const unlistenOpen = listen("picker-open", async () => {
      console.log("[picker] open");
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
      // 第一次 show()(visible:false → true);之後 close 不 hide 只移 off-screen,
      // 所以這裡 show() 第二次以後是 no-op 但安全。
      try { await win.show(); } catch (e) { console.error("[picker] show failed", e); }
      setOpen(true);
      // Wayland 救援:多次 setFocus + focus rootRef(state 還沒 commit 前先 retry)
      const tryFocus = async () => {
        try {
          await win.setFocus();
        } catch {}
        rootRef.current?.focus();
      };
      tryFocus();
      setTimeout(tryFocus, 30);
      setTimeout(tryFocus, 120);
    });
    return () => { unlistenOpen.then((f) => f()); };
  }, []);

  // Global keydown — 同時掛 document 跟 rootRef onKeyDown,二重保險
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!openRef.current) return;
      const sec = sectionRef.current;
      const list = sec === "voice" ? voiceRef.current : agentRef.current;
      const idx = sec === "voice" ? voiceIdxRef.current : agentIdxRef.current;
      const setIdx = sec === "voice" ? setVoiceIdx : setAgentIdx;

      if (e.key === "Escape") { e.preventDefault(); close(); return; }
      if (e.key === "Enter") { e.preventDefault(); confirm(); return; }
      if (e.key === "Tab") {
        e.preventDefault();
        setSection(sec === "voice" ? "agent" : "voice");
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
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, []);

  if (!open) return null;

  const list = section === "voice" ? voice : agent;
  const idx = section === "voice" ? voiceIdx : agentIdx;

  // 3-item carousel:取 prev / current / next(空時填 null)
  const at = (i: number): ProfileEntry | null =>
    list.length === 0 ? null : list[((i % list.length) + list.length) % list.length];
  const prev = list.length > 1 ? at(idx - 1) : null;
  const cur = at(idx);
  const next = list.length > 1 ? at(idx + 1) : null;

  return (
    <div
      ref={rootRef}
      tabIndex={0}
      className="mori-picker-root"
    >
      <div className="mori-picker-card mori-picker-carousel">
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

        <div className="mori-picker-carousel-body">
          {list.length === 0 ? (
            <div className="mori-picker-empty">（這個目錄沒有 profile）</div>
          ) : (
            <>
              <div className="mori-picker-row prev" onClick={() => {
                const setI = section === "voice" ? setVoiceIdx : setAgentIdx;
                if (list.length > 0) setI((idx - 1 + list.length) % list.length);
              }}>
                {prev && <>
                  <span className="mori-picker-display">{prev.display}</span>
                  <span className="mori-picker-stem">{prev.stem}</span>
                </>}
              </div>
              <div
                className="mori-picker-row cur"
                onClick={confirm}
                onDoubleClick={confirm}
                title="Enter / 點擊確認"
              >
                {cur && <>
                  <span className="mori-picker-cursor">▸</span>
                  <span className="mori-picker-display">{cur.display}</span>
                  <span className="mori-picker-stem">{cur.stem}</span>
                </>}
              </div>
              <div className="mori-picker-row next" onClick={() => {
                const setI = section === "voice" ? setVoiceIdx : setAgentIdx;
                if (list.length > 0) setI((idx + 1) % list.length);
              }}>
                {next && <>
                  <span className="mori-picker-display">{next.display}</span>
                  <span className="mori-picker-stem">{next.stem}</span>
                </>}
              </div>
              <div className="mori-picker-position">
                {idx + 1} / {list.length}
              </div>
            </>
          )}
        </div>

        <div className="mori-picker-footer">
          <kbd>↑↓</kbd> 選 &nbsp; <kbd>Tab</kbd> 切組 &nbsp;
          <kbd>Enter</kbd> 確認 &nbsp; <kbd>Esc</kbd> 取消
        </div>
      </div>
    </div>
  );
}

export default Picker;
