import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";
import { ActorApp, actorIdFromLocation } from "./ActorApp";
import { App } from "./App";
import {
  BubbleSurfaceApp,
  bubbleActorIdFromLocation,
} from "./BubbleSurfaceApp";
import { StartupErrorApp } from "./StartupErrorApp";
import "./styles.css";

const windowLabel = currentWindowLabel();
const actorId = actorIdFromLocation();
const bubbleActorId = bubbleActorIdFromLocation();
const isStartupError = windowLabel === "startup-error";
const isActorSurface = Boolean(actorId) || windowLabel.startsWith("actor-");
const isBubbleSurface =
  Boolean(bubbleActorId) || windowLabel.startsWith("bubble-");
document.body.dataset.surface = isStartupError
  ? "startup-error"
  : isBubbleSurface
    ? "bubble"
    : isActorSurface
      ? "actor"
      : windowLabel;
const root = isStartupError ? (
  <StartupErrorApp />
) : isBubbleSurface ? (
  <BubbleSurfaceApp actorId={bubbleActorId} />
) : isActorSurface ? (
  <ActorApp actorId={actorId} />
) : (
  <App />
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{root}</React.StrictMode>,
);

function currentWindowLabel() {
  if (!("__TAURI_INTERNALS__" in window)) return "settings";
  try {
    return getCurrentWindow().label;
  } catch {
    return "settings";
  }
}
