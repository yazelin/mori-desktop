import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import FloatingMori from "./FloatingMori";
import "./styles.css";
import "./floating.css";

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
} else {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}
