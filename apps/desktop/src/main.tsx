import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ActorApp, actorIdFromLocation } from "./ActorApp";
import { App } from "./App";
import "./styles.css";

const windowLabel = currentWindowLabel();
const actorId = actorIdFromLocation();
const isActorSurface = Boolean(actorId) || windowLabel.startsWith("actor-");
document.body.dataset.surface = isActorSurface ? "actor" : windowLabel;
const root = isActorSurface ? <ActorApp actorId={actorId} /> : <App />;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{root}</React.StrictMode>
);

function currentWindowLabel() {
  if (!("__TAURI_INTERNALS__" in window)) return "settings";
  try {
    return getCurrentWindow().label;
  } catch {
    return "settings";
  }
}
