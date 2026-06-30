import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ActorApp } from "./ActorApp";
import { App } from "./App";
import "./styles.css";

const windowLabel = currentWindowLabel();
document.body.dataset.surface = windowLabel;
const Root = windowLabel === "actor" ? ActorApp : App;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>
);

function currentWindowLabel() {
  if (!("__TAURI_INTERNALS__" in window)) return "settings";
  try {
    return getCurrentWindow().label;
  } catch {
    return "settings";
  }
}
