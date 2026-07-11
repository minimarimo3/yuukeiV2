import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StageOverlayApp } from "./StageOverlayApp";
import type { DesktopStageState, YuukeiClient } from "./yuukeiClient";

describe("StageOverlayApp", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders stage bubbles from desktop stage state", async () => {
    render(<StageOverlayApp client={clientFixture(stageState())} monitorId="monitor-0" />);

    const bubble = await screen.findByText("ここに出ます");

    expect(bubble).toBeInTheDocument();
    expect(bubble.closest(".stage-bubble")).toHaveClass(
      "stage-bubble--right",
      "actor-bubble--right"
    );
  });

  it("does not render bubbles for hidden actors", async () => {
    const client = clientFixture(stageState({}, { visible: false }));

    render(<StageOverlayApp client={client} monitorId="monitor-0" />);

    await waitFor(() => {
      expect(client.getDesktopStageState).toHaveBeenCalled();
    });
    expect(screen.queryByText("ここに出ます")).not.toBeInTheDocument();
  });

  it("marks bubbles placed above the actor with the above side class", async () => {
    render(
      <StageOverlayApp
        client={clientFixture(
          stageState(
            {},
            {
              bounds: {
                x: 240,
                y: 200,
                width: 420,
                height: 420
              },
              anchor: {
                x: 450,
                y: 360,
                visible: true
              }
            }
          )
        )}
        monitorId="monitor-0"
      />
    );

    const bubble = await screen.findByText("ここに出ます");

    expect(bubble.closest(".stage-bubble")).toHaveClass("stage-bubble--above");
    expect(bubble.closest(".stage-bubble")).not.toHaveClass(
      "actor-bubble--right"
    );
    expect(bubble.closest(".stage-bubble")).not.toHaveClass("actor-bubble--left");
  });

  it("marks bubbles placed below the actor with the below side class", async () => {
    render(
      <StageOverlayApp
        client={clientFixture(
          stageState(
            {},
            {
              bounds: {
                x: 240,
                y: 20,
                width: 420,
                height: 420
              },
              anchor: {
                x: 450,
                y: 260,
                visible: true
              }
            }
          )
        )}
        monitorId="monitor-0"
      />
    );

    const bubble = await screen.findByText("ここに出ます");

    expect(bubble.closest(".stage-bubble")).toHaveClass("stage-bubble--below");
    expect(bubble.closest(".stage-bubble")).not.toHaveClass(
      "actor-bubble--right"
    );
    expect(bubble.closest(".stage-bubble")).not.toHaveClass("actor-bubble--left");
  });

  it("keeps the left side actor bubble class for left placements", async () => {
    render(
      <StageOverlayApp
        client={clientFixture(
          stageState(
            {},
            {
              bounds: {
                x: 420,
                y: 72,
                width: 420,
                height: 560
              },
              anchor: {
                x: 640,
                y: 190,
                visible: true
              }
            }
          )
        )}
        monitorId="monitor-0"
      />
    );

    const bubble = await screen.findByText("ここに出ます");

    expect(bubble.closest(".stage-bubble")).toHaveClass(
      "stage-bubble--left",
      "actor-bubble--left"
    );
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

  it("renders choice buttons and sends the selected choice", async () => {
    const client = clientFixture(
      stageState({
        choice: {
          choiceId: "choice-1",
          choices: ["見る", "あとで"],
          timeoutSeconds: 30
        }
      })
    );
    const user = userEvent.setup();

    render(<StageOverlayApp client={client} monitorId="monitor-0" />);

    const choice = await screen.findByRole("button", { name: "見る" });
    expect(screen.getByRole("button", { name: "あとで" })).toBeInTheDocument();

    await user.click(choice);

    expect(client.sendConversationChoice).toHaveBeenCalledWith(
      "choice-1",
      "見る",
      0
    );
    expect(screen.queryByRole("button", { name: "見る" })).not.toBeInTheDocument();
  });
});

function clientFixture(stage: DesktopStageState): YuukeiClient {
  // StageOverlayが使わないAPIはstub省略し、型はunknown経由でYuukeiClientへ寄せる。
  const partial: Partial<YuukeiClient> = {
    attachSurface: vi.fn(),
    getSnapshot: vi.fn(),
    getWorldPackStatus: vi.fn(),
    getAppSettings: vi.fn(async () => ({
      talkIntervalMinutes: 5,
      actorScalePercent: 100,
      settingsPath: "/tmp/yuukei-v2/settings/app.json"
    })),
    getExtensionSettings: vi.fn(),
    getCapabilityUsage: vi.fn(),
    getActorSurfaceAssets: vi.fn(),
    setActorWindowClickThrough: vi.fn(async () => undefined),
    setStageOverlayClickThrough: vi.fn(async () => undefined),
    getDesktopStageState: vi.fn(async () => stage),
    reportActorStageAnchor: vi.fn(async () => undefined),
    dismissStageBubble: vi.fn(async () => undefined),
    openSettingsWindow: vi.fn(),
    sendConversationText: vi.fn(),
    sendConversationChoice: vi.fn(async () => []),
    sendAvatarGesturePoke: vi.fn(),
    beginActorWindowDrag: vi.fn(),
    moveActorWindowDrag: vi.fn(),
    finishActorWindowDrag: vi.fn(),
    cancelActorWindowDrag: vi.fn(),
    notifyAvatarGestureGrab: vi.fn(),
    notifyAvatarGestureDrop: vi.fn(),
    openWorldPackDirectory: vi.fn(),
    openExtensionDirectory: vi.fn(),
    selectWorldPackDirectory: vi.fn(),
    resetWorldPackToDefault: vi.fn(),
    installExtensionDirectory: vi.fn(),
    uninstallExtension: vi.fn(),
    setExtensionEnabled: vi.fn(),
    setAppTalkIntervalMinutes: vi.fn(async (minutes: number) => ({
      talkIntervalMinutes: minutes,
      actorScalePercent: 100,
      settingsPath: "/tmp/yuukei-v2/settings/app.json"
    })),
    setAppActorScalePercent: vi.fn(async (percent: number) => ({
      talkIntervalMinutes: 5,
      actorScalePercent: percent,
      settingsPath: "/tmp/yuukei-v2/settings/app.json"
    })),
    setExtensionHookOrder: vi.fn(),
    setExtensionSettingValues: vi.fn(),
    setExtensionSecret: vi.fn(),
    onCommand: vi.fn(async () => () => undefined),
    onSnapshot: vi.fn(async () => () => undefined),
    onWorldPackStatus: vi.fn(async () => () => undefined),
    onAssetsChanged: vi.fn(async () => () => undefined),
    onStageState: vi.fn(async () => () => undefined)
  };
  return partial as unknown as YuukeiClient;
}

type StageActorFixture = Partial<
  Omit<DesktopStageState["actors"][number], "bounds" | "anchor">
> & {
  bounds?: Partial<DesktopStageState["actors"][number]["bounds"]>;
  anchor?: Partial<DesktopStageState["actors"][number]["anchor"]>;
};

function stageState(
  bubble: Partial<DesktopStageState["bubbles"][number]> = {},
  actor: StageActorFixture = {}
): DesktopStageState {
  const bounds = {
    x: 64,
    y: 72,
    width: 420,
    height: 560,
    ...actor.bounds
  };
  const anchor = {
    x: 260,
    y: 190,
    visible: true,
    ...actor.anchor
  };
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
        ...actor,
        bounds,
        anchor,
        visible: actor.visible ?? true
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
