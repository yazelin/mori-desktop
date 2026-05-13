// 5J: 獨立 chat_bubble Tauri 視窗,只負責顯示 Mori 完整回應 / 長轉錄。
// 跟 floating sprite 視窗解耦,避免單窗 setSize + transparent 在 Wayland 上的不穩。
//
// - 由 FloatingMori.tsx 透過 emit("chat-bubble-show", text) 觸發
// - 自己根據內容 offsetHeight setSize,跟 setPosition 緊貼 sprite 下方
// - emit("chat-bubble-hide") 關閉

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

// FloatingMori.tsx 已經算好 bubble 應該放的絕對 logical position,
// 這邊只負責 setPosition + setSize + show/hide。不重新算 sprite 位置(GNOME mutter
// outerSize 在 transparent+decorationless 視窗會飄,導致 bubble 越離 sprite 越遠)。
type ShowPayload = { text: string; x: number; y: number };
type MovedPayload = { x: number; y: number };

const WIDTH = 360;
const MAX_HEIGHT = 480;
const MIN_HEIGHT = 56;

function ChatBubble() {
  const [text, setText] = useState<string>("");
  const bubbleRef = useRef<HTMLDivElement | null>(null);

  // 訊息監聽 — show / hide / sprite-moved
  //
  // 5J: 用「移到 off-screen 當作隱藏」而非 hide(),因為 hide 過的 transparent 視窗
  // 在 GNOME Wayland 重新 show 時有時不會回來。tauri.conf.json 已把 chat_bubble
  // 初始放在 (-10000, -10000) + visible: true,確保 webview / React 一啟動就 mount,
  // listeners 立刻註冊。
  useEffect(() => {
    const win = getCurrentWindow();
    console.log("[chat_bubble] listeners attaching");

    const unlistenShow = listen<ShowPayload>("chat-bubble-show", async (e) => {
      const { text, x, y } = e.payload;
      console.log("[chat_bubble] show", { x, y, len: text.length });
      setText(text);
      try {
        await win.setSize(new LogicalSize(WIDTH, MIN_HEIGHT));
        await win.setPosition(new LogicalPosition(x, y));
        await win.show();
        // X11:兩個視窗都 alwaysOnTop:true,同 layer 內 floating 因互動較頻
        // 繁(hover / drag)raise 較新會壓在 chat_bubble 上面 — 文字看不到。
        // setAlwaysOnTop toggle 只翻 state 不 re-raise within layer,要真正
        // raise 必須走 X11 XRaiseWindow。force_raise_window 後端 shell-out
        // xdotool windowraise 解決。Wayland no-op。
        await invoke("force_raise_window", { label: "chat_bubble" }).catch(
          (err: unknown) => console.warn("[chat_bubble] force_raise failed", err),
        );
      } catch (err) { console.error("[chat_bubble] show pos/size error", err); }
    });

    const unlistenHide = listen("chat-bubble-hide", async () => {
      console.log("[chat_bubble] hide");
      try {
        // 5K-1c: 跟 picker 同樣策略,移 off-screen 而非 hide() —
        // visible 保留,WMClass group 已建立,後續 show 不會再造成 dock 堆疊
        await win.setPosition(new LogicalPosition(-10000, -10000));
      } catch (err) { console.error("[chat_bubble] move off-screen error", err); }
      // brand-3 follow-up: 雙保險縮成 1×1。Wayland 偶爾 setPosition 沒成功
      // (mutter 對 transparent + decorationless window 不穩),視窗停在
      // user 上次顯示位置(常在 navbar / 左上角),仍 alwaysOnTop 吃 click。
      // 即使 setPosition fail,1×1 透明窗擋不住任何 click。
      try {
        await win.setSize(new LogicalSize(1, 1));
      } catch (err) { console.error("[chat_bubble] shrink error", err); }
      setText("");
    });

    // sprite 拖動時,跟著移動 — payload 就是要放的絕對位置
    const unlistenMoved = listen<MovedPayload>("sprite-moved", async (e) => {
      const { x, y } = e.payload;
      try {
        await win.setPosition(new LogicalPosition(x, y));
      } catch (err) { console.error("[chat_bubble] moved pos error", err); }
    });

    return () => {
      unlistenShow.then((f) => f());
      unlistenHide.then((f) => f());
      unlistenMoved.then((f) => f());
    };
  }, []);

  // 內容 mounted / 改變時,resize window 配合 bubble 高度
  useEffect(() => {
    if (!text || !bubbleRef.current) return;
    const win = getCurrentWindow();
    const bubble = bubbleRef.current;
    const sync = () => {
      const h = Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, bubble.offsetHeight));
      win.setSize(new LogicalSize(WIDTH, h)).catch(() => {});
    };
    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(bubble);
    return () => ro.disconnect();
  }, [text]);

  if (!text) return null;

  return (
    <div ref={bubbleRef} className="mori-chat-window">
      {text}
    </div>
  );
}

export default ChatBubble;
