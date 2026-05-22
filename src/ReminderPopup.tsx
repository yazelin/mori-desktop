// 2026-05-22:in-app reminder popup window — Mori 自家通知,因為 Linux GNOME
// 對「有 ABOVE 視窗的 app」會抑制 OS 通知 banner。
//
// 由 mori-time on_fire callback → TauriEventEmitter → emit "reminder-fire-show"
// 觸發;mount 時也 invoke reminder_active_queue 補抓未 dismissed reminders。
//
// 對齊 ChatBubble.tsx pattern:transparent + decorationless + alwaysOnTop;
// 隱藏雙保險 = setPosition(-10000, -10000) + setSize(1, 1)。

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

type ActiveReminder = {
  id: number;
  text: string;
  dueAt: string;     // ISO8601
  firedAt: string;
};

type SpriteMoved = { x: number; y: number };

const POPUP_WIDTH = 320;
const POPUP_MAX_HEIGHT = 480;
const CHIP_SIZE = 32;
const POPUP_TO_CHIP_MINUTES = 5;
const DEBOUNCE_MS = 200;
const SPRITE_GAP = 12;
// MVP sprite 預設大小(對齊 FloatingMori),sprite 寬高都 200,gap 12 後 popup 在下方
const SPRITE_HEIGHT = 200;

function ReminderPopup() {
  const [queue, setQueue] = useState<ActiveReminder[]>([]);
  const [mode, setMode] = useState<"popup" | "chip">("popup");
  const [spritePos, setSpritePos] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const cardRef = useRef<HTMLDivElement | null>(null);
  const debounceTimer = useRef<number | null>(null);
  const inactivityTimer = useRef<number | null>(null);

  // === debounce buffer:多個 emit 同瞬間進來時合成一次 setQueue ===
  const pendingNew = useRef<ActiveReminder[]>([]);
  const flushPending = () => {
    if (pendingNew.current.length === 0) return;
    setQueue((prev) => {
      const merged = [...prev];
      for (const r of pendingNew.current) {
        if (!merged.some((x) => x.id === r.id)) merged.push(r);
      }
      return merged;
    });
    pendingNew.current = [];
  };

  // === listen + 啟動補抓 ===
  useEffect(() => {
    let unlistenFire: (() => void) | null = null;
    let unlistenSpriteMoved: (() => void) | null = null;

    (async () => {
      // 1) 補抓 mount 前 emit 過、popup 還沒 ready 收到的 reminder
      try {
        const active = await invoke<ActiveReminder[]>("reminder_active_queue");
        if (active.length > 0) {
          setQueue(active);
          setMode("popup");
        }
      } catch (e) {
        console.warn("[reminder_popup] active_queue fetch failed", e);
      }

      // 2) 訂閱新 fire 事件
      const u1 = await listen<ActiveReminder>("reminder-fire-show", (e) => {
        pendingNew.current.push(e.payload);
        if (debounceTimer.current !== null) window.clearTimeout(debounceTimer.current);
        debounceTimer.current = window.setTimeout(() => {
          flushPending();
          setMode("popup");  // 新 fire 進來,從 chip 拉回 popup
        }, DEBOUNCE_MS);
      });
      unlistenFire = u1;

      // 3) sprite 拖動同步位置
      const u2 = await listen<SpriteMoved>("sprite-moved", (e) => {
        setSpritePos(e.payload);
      });
      unlistenSpriteMoved = u2;
    })();

    return () => {
      if (debounceTimer.current !== null) window.clearTimeout(debounceTimer.current);
      if (inactivityTimer.current !== null) window.clearTimeout(inactivityTimer.current);
      unlistenFire?.();
      unlistenSpriteMoved?.();
    };
  }, []);

  // === queue 變化 → setSize / setPosition / show ===
  useEffect(() => {
    const win = getCurrentWindow();
    if (queue.length === 0) {
      // 完全 dismiss 過渡 — 雙保險:移 off-screen + 縮 1x1
      win.setPosition(new LogicalPosition(-10000, -10000)).catch(() => {});
      win.setSize(new LogicalSize(1, 1)).catch(() => {});
      return;
    }
    // sprite 旁 anchor:預設貼 sprite 下方
    const anchorX = spritePos.x;
    const anchorY = spritePos.y + SPRITE_HEIGHT + SPRITE_GAP;
    win.setPosition(new LogicalPosition(anchorX, anchorY)).catch(() => {});

    if (mode === "chip") {
      win.setSize(new LogicalSize(CHIP_SIZE, CHIP_SIZE)).catch(() => {});
      return;
    }
    // popup mode:跟著 card 內容高度
    requestAnimationFrame(() => {
      const measured = cardRef.current?.offsetHeight ?? 0;
      if (measured <= 0) return;  // 沿用 ChatBubble pattern,offsetHeight=0 skip
      const h = Math.min(POPUP_MAX_HEIGHT, measured);
      win.setSize(new LogicalSize(POPUP_WIDTH, h)).catch(() => {});
    });
  }, [queue, mode, spritePos]);

  // === inactivity timer:popup → chip 自動退場 ===
  useEffect(() => {
    if (mode !== "popup" || queue.length === 0) return;
    if (inactivityTimer.current !== null) window.clearTimeout(inactivityTimer.current);
    inactivityTimer.current = window.setTimeout(() => {
      setMode("chip");
    }, POPUP_TO_CHIP_MINUTES * 60 * 1000);
    return () => {
      if (inactivityTimer.current !== null) window.clearTimeout(inactivityTimer.current);
    };
  }, [mode, queue.length]);

  const onSnooze = async (id: number) => {
    try {
      await invoke("reminder_snooze", { id, minutes: 5 });
      setQueue((q) => q.filter((r) => r.id !== id));
    } catch (e) {
      console.error("[reminder_popup] snooze failed", e);
      alert(`稍後失敗:${e}`);  // MVP:粗暴 alert,follow-up 改 inline 紅 chip
    }
  };

  const onDismiss = async (id: number) => {
    try {
      await invoke("reminder_dismiss", { id });
      setQueue((q) => q.filter((r) => r.id !== id));
    } catch (e) {
      console.error("[reminder_popup] dismiss failed", e);
      alert(`關閉失敗:${e}`);
    }
  };

  if (queue.length === 0) return null;

  // === chip mode render ===
  if (mode === "chip") {
    return (
      <div
        className="reminder-chip"
        onClick={() => setMode("popup")}
        title={`${queue.length} 條未讀提醒`}
      >
        🔔 {queue.length}
      </div>
    );
  }

  // === popup mode render ===
  const visible = queue.slice(0, 5);
  const overflow = Math.max(0, queue.length - 5);

  return (
    <div ref={cardRef} className="reminder-card">
      {visible.map((r) => (
        <div key={r.id} className="reminder-row">
          <div className="reminder-row-head">
            <span className="reminder-bell">🔔</span>
            <span className="reminder-text">{r.text}</span>
          </div>
          <div className="reminder-row-meta">
            <span className="reminder-due">原定 {formatDueChip(r.dueAt)}</span>
            <button onClick={() => onSnooze(r.id)}>稍後 5 分</button>
            <button onClick={() => onDismiss(r.id)}>關閉</button>
          </div>
        </div>
      ))}
      {overflow > 0 && (
        <div className="reminder-overflow-chip">+{overflow} 條歷史提醒</div>
      )}
    </div>
  );
}

function formatDueChip(iso: string): string {
  try {
    const d = new Date(iso);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  } catch {
    return "?";
  }
}

export default ReminderPopup;
