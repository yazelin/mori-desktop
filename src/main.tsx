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
  // The floating window has no chrome and a transparent background; clear
  // anything the main stylesheet might have put on body (e.g. background
  // color, padding) so the only thing visible is the Mori widget itself.
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
