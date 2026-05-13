import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import MainShell from "./MainShell";
import FloatingMori from "./FloatingMori";
import ChatBubble from "./ChatBubble";
import Picker from "./Picker";
import { subscribeTheme } from "./theme";
import "./styles.css";
import "./shell.css";
import "./chat-panel.css";
import "./floating.css";
import "./chat-bubble.css";
import "./picker.css";

const label = getCurrentWindow().label;
const root = document.getElementById("root")!;

// brand-3: 每個 window 都 subscribe 主 theme(load 一次 + listen "theme-changed"),
// 任一視窗切 theme 後其他 window 收到 event 一起更新。
subscribeTheme();

// X11 透明 fallback class — 詢問 Rust 後端 session type,X11 加 class 觸發
// CSS 的 opaque 背景 + 美術背板。每個 window mount 都跑一次,reload 後也
// 會重新加,不像 Rust startup eval 只能一次。WebKit2GTK on X11 的 ARGB
// alpha 處理問題見 src/floating.css 同名 selector 註解。
invoke<boolean>("is_x11_session")
  .then((isX11) => {
    if (isX11) {
      document.documentElement.classList.add("x11-fallback");
      document.body.classList.add("x11-fallback");
    }
  })
  .catch(() => {});

if (label === "floating") {
  // The floating window has no chrome and a transparent background; we
  // tag both <html> and <body> so the floating.css reset can wipe out
  // any default margins/padding either inherits from styles.css.
  document.documentElement.classList.add("floating-window");
  document.body.classList.add("floating-window");
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <FloatingMori />
    </React.StrictMode>,
  );
} else if (label === "chat_bubble") {
  // 5J: 獨立 chat_bubble window — 跟 sprite 視窗解耦,避免單窗 setSize 在 Wayland 不穩
  document.documentElement.classList.add("chat-bubble-window");
  document.body.classList.add("chat-bubble-window");
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <ChatBubble />
    </React.StrictMode>,
  );
} else if (label === "picker") {
  // 5K-1: Profile Picker overlay,Ctrl+Alt+P 開啟
  document.documentElement.classList.add("picker-window");
  document.body.classList.add("picker-window");
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <Picker />
    </React.StrictMode>,
  );
} else {
  // 5M: main window 換成 sidebar shell,App(chat)變成 sidebar 的一個 tab。
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <MainShell />
    </React.StrictMode>,
  );
}
