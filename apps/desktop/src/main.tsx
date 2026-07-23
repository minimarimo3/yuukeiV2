import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";
import { ActorApp, actorIdFromLocation } from "./ActorApp";
import { App } from "./App";
import { StageOverlayApp, stageOverlayIdFromLocation } from "./StageOverlayApp";
import { StartupErrorApp } from "./StartupErrorApp";
import "./styles.css";

const windowLabel = currentWindowLabel();
const actorId = actorIdFromLocation();
const stageOverlayId = stageOverlayIdFromLocation();
const isStartupError = windowLabel === "startup-error";
const isActorSurface = Boolean(actorId) || windowLabel.startsWith("actor-");
const isStageOverlay =
  Boolean(stageOverlayId) || windowLabel.startsWith("stage-overlay-");
document.body.dataset.surface = isStartupError
  ? "startup-error"
  : isStageOverlay
    ? "stage-overlay"
    : isActorSurface
      ? "actor"
      : windowLabel;
const root = isStartupError ? (
  <StartupErrorApp />
) : isStageOverlay ? (
  <StageOverlayApp monitorId={stageOverlayId} />
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
