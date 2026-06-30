import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StageOverlayApp } from "./StageOverlayApp";
import type { DesktopStageState, YuukeiClient } from "./yuukeiClient";

describe("StageOverlayApp", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders stage bubbles from desktop stage state", async () => {
    render(<StageOverlayApp client={clientFixture(stageState())} monitorId="monitor-0" />);

    expect(await screen.findByText("ここに出ます")).toBeInTheDocument();
  });

  it("dismisses expired bubbles through the stage manager", async () => {
    const state = stageState({
      createdAtMs: Date.now() - 20_000,
      durationMs: 1
    });
    const client = clientFixture(state);

    render(<StageOverlayApp client={client} monitorId="monitor-0" />);

    await waitFor(() => {
      expect(client.dismissStageBubble).toHaveBeenCalledWith("bubble-1");
    });
  });
});

function clientFixture(stage: DesktopStageState): YuukeiClient {
  return {
    attachSurface: vi.fn(),
    getSnapshot: vi.fn(),
    getWorldPackStatus: vi.fn(),
    getExtensionSettings: vi.fn(),
    getActorSurfaceAssets: vi.fn(),
    setActorWindowClickThrough: vi.fn(async () => undefined),
    setStageOverlayClickThrough: vi.fn(async () => undefined),
    getDesktopStageState: vi.fn(async () => stage),
    reportActorStageAnchor: vi.fn(async () => undefined),
    dismissStageBubble: vi.fn(async () => undefined),
    openSettingsWindow: vi.fn(),
    sendConversationText: vi.fn(),
    sendAvatarGesturePoke: vi.fn(),
    openWorldPackDirectory: vi.fn(),
    openExtensionDirectory: vi.fn(),
    selectWorldPackDirectory: vi.fn(),
    resetWorldPackToDefault: vi.fn(),
    installExtensionDirectory: vi.fn(),
    uninstallExtension: vi.fn(),
    setExtensionEnabled: vi.fn(),
    setExtensionHookOrder: vi.fn(),
    onCommand: vi.fn(async () => () => undefined),
    onSnapshot: vi.fn(async () => () => undefined),
    onAssetsChanged: vi.fn(async () => () => undefined),
    onStageState: vi.fn(async () => () => undefined)
  };
}

function stageState(
  bubble: Partial<DesktopStageState["bubbles"][number]> = {}
): DesktopStageState {
  return {
    monitors: [
      {
        id: "monitor-0",
        label: "stage-overlay-0",
        bounds: {
          x: 0,
          y: 0,
          width: 900,
          height: 640
        },
        scaleFactor: 1
      }
    ],
    actors: [
      {
        actorId: "yuukei",
        displayName: "Yuukei",
        windowLabel: "actor-7975756b6569",
        bounds: {
          x: 64,
          y: 72,
          width: 420,
          height: 560
        },
        anchor: {
          x: 260,
          y: 190,
          visible: true
        },
        visible: true
      }
    ],
    bubbles: [
      {
        bubbleId: "bubble-1",
        actorId: "yuukei",
        text: "ここに出ます",
        createdAtMs: Date.now(),
        durationMs: 9000,
        ...bubble
      }
    ]
  };
}
