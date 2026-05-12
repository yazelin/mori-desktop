import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import FloatingMori from "./FloatingMori";
import ChatBubble from "./ChatBubble";
import Picker from "./Picker";
import "./styles.css";
import "./floating.css";
import "./chat-bubble.css";
import "./picker.css";

const label = getCurrentWindow().label;
const root = document.getElementById("root")!;

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
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}
