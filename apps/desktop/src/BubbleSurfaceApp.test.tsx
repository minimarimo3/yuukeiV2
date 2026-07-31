import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  BubbleSurfaceApp,
  bubbleSurfaceBounds,
  bubbleTypingProgress,
} from "./BubbleSurfaceApp";
import type { DesktopStageState, YuukeiClient } from "./yuukeiClient";

describe("BubbleSurfaceApp", () => {
  afterEach(() => {
    cleanup();
  });

  it("uses code-point reading progress for bubbles without pending speech", () => {
    const bubble = stageState({
      text: "A😀",
      createdAtMs: 0,
      durationMs: 1_000,
    }).bubbles[0];

    expect(bubbleTypingProgress(bubble, 0)).toBe(0);
    expect(bubbleTypingProgress(bubble, 90)).toBe(0.5);
    expect(bubbleTypingProgress(bubble, 180)).toBe(1);
  });

  it("waits for speech, falls back after five seconds, and never lets late audio rewind text", () => {
    const bubble = stageState({
      text: "abc",
      createdAtMs: 0,
      durationMs: 7_500,
      speechPending: true,
    }).bubbles[0];

    expect(bubbleTypingProgress(bubble, 4_999)).toBe(0);
    expect(bubbleTypingProgress(bubble, 5_135)).toBe(0.5);
    expect(
      bubbleTypingProgress(
        { ...bubble, audioStartedAtMs: 6_000, audioDurationMs: 10_000 },
        6_500,
      ),
    ).toBe(1);
  });

  it("uses audio duration as the typing clock before fallback begins", () => {
    const bubble = stageState({
      text: "abcd",
      createdAtMs: 0,
      durationMs: 8_000,
      speechPending: true,
      audioStartedAtMs: 1_000,
      audioDurationMs: 2_000,
    }).bubbles[0];

    expect(bubbleTypingProgress(bubble, 2_000)).toBe(0.5);
    expect(bubbleTypingProgress(bubble, 3_000)).toBe(1);
  });

  it("bounds the native surface to the bubble instead of the monitor", () => {
    const state = stageState();
    const bounds = bubbleSurfaceBounds(state.monitors[0].bounds, {
      side: "above",
      left: 200,
      top: 120,
      width: 260,
      maxWidth: 260,
      tailTop: 40,
      tailLeft: 130,
      rect: { x: 200, y: 120, width: 260, height: 72 },
    });

    expect(bounds).toEqual({
      x: 184,
      y: 104,
      width: 292,
      height: 104,
    });
    expect(bounds.width).toBeLessThan(state.monitors[0].bounds.width);
    expect(bounds.height).toBeLessThan(state.monitors[0].bounds.height);
  });

  it("keeps the full text layout while a pending-speech bubble shows its placeholder and choices", async () => {
    const state = stageState({
      text: "全文を確保",
      createdAtMs: Date.now(),
      speechPending: true,
      choice: {
        choiceId: "choice-typing",
        choices: ["すぐ選ぶ"],
        timeoutSeconds: 30,
      },
    });

    render(<BubbleSurfaceApp client={clientFixture(state)} actorId="yuukei" />);

    const placeholder = await screen.findByLabelText("読み上げを待っています");
    const content = placeholder.parentElement;
    expect(content?.querySelectorAll(".actor-bubble-character")).toHaveLength(
      5,
    );
    expect(
      content?.querySelectorAll('[data-typing-visible="false"]'),
    ).toHaveLength(5);
    expect(
      screen.getByRole("button", { name: "すぐ選ぶ" }),
    ).toBeInTheDocument();
  });

  it("renders stage bubbles from desktop stage state", async () => {
    const client = clientFixture(stageState());
    render(<BubbleSurfaceApp client={client} actorId="yuukei" />);

    const bubble = await findBubbleText("ここに出ます");

    expect(bubble).toBeInTheDocument();
    await waitFor(() => {
      expect(client.surfaceReady).toHaveBeenCalledTimes(1);
    });
    expect(bubble.closest(".stage-bubble")).toHaveClass(
      "stage-bubble--right",
      "actor-bubble--right",
    );
  });

  it("does not render bubbles for hidden actors", async () => {
    const client = clientFixture(stageState({}, { visible: false }));

    render(<BubbleSurfaceApp client={client} actorId="yuukei" />);

    await waitFor(() => {
      expect(client.getDesktopStageState).toHaveBeenCalled();
    });
    expect(screen.queryByText("ここに出ます")).not.toBeInTheDocument();
  });

  it("marks bubbles placed above the actor with the above side class", async () => {
    render(
      <BubbleSurfaceApp
        client={clientFixture(
          stageState(
            {},
            {
              bounds: {
                x: 240,
                y: 200,
                width: 420,
                height: 420,
              },
              anchor: {
                x: 450,
                y: 360,
                visible: true,
              },
            },
          ),
        )}
        actorId="yuukei"
      />,
    );

    const bubble = await findBubbleText("ここに出ます");

    expect(bubble.closest(".stage-bubble")).toHaveClass("stage-bubble--above");
    expect(bubble.closest(".stage-bubble")).not.toHaveClass(
      "actor-bubble--right",
    );
    expect(bubble.closest(".stage-bubble")).not.toHaveClass(
      "actor-bubble--left",
    );
  });

  it("marks bubbles placed below the actor with the below side class", async () => {
    render(
      <BubbleSurfaceApp
        client={clientFixture(
          stageState(
            {},
            {
              bounds: {
                x: 240,
                y: 20,
                width: 420,
                height: 420,
              },
              anchor: {
                x: 450,
                y: 260,
                visible: true,
              },
            },
          ),
        )}
        actorId="yuukei"
      />,
    );

    const bubble = await findBubbleText("ここに出ます");

    expect(bubble.closest(".stage-bubble")).toHaveClass("stage-bubble--below");
    expect(bubble.closest(".stage-bubble")).not.toHaveClass(
      "actor-bubble--right",
    );
    expect(bubble.closest(".stage-bubble")).not.toHaveClass(
      "actor-bubble--left",
    );
  });

  it("keeps the left side actor bubble class for left placements", async () => {
    render(
      <BubbleSurfaceApp
        client={clientFixture(
          stageState(
            {},
            {
              bounds: {
                x: 420,
                y: 72,
                width: 420,
                height: 560,
              },
              anchor: {
                x: 640,
                y: 190,
                visible: true,
              },
            },
          ),
        )}
        actorId="yuukei"
      />,
    );

    const bubble = await findBubbleText("ここに出ます");

    expect(bubble.closest(".stage-bubble")).toHaveClass(
      "stage-bubble--left",
      "actor-bubble--left",
    );
  });

  it("dismisses expired bubbles through the stage manager", async () => {
    const state = stageState({
      createdAtMs: Date.now() - 20_000,
      durationMs: 1,
    });
    const client = clientFixture(state);

    render(<BubbleSurfaceApp client={client} actorId="yuukei" />);

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
          timeoutSeconds: 30,
        },
      }),
    );
    const user = userEvent.setup();

    render(<BubbleSurfaceApp client={client} actorId="yuukei" />);

    const choice = await screen.findByRole("button", { name: "見る" });
    expect(screen.getByRole("button", { name: "あとで" })).toBeInTheDocument();

    await user.click(choice);

    expect(client.sendConversationChoice).toHaveBeenCalledWith(
      "choice-1",
      "見る",
      0,
    );
    expect(
      screen.queryByRole("button", { name: "見る" }),
    ).not.toBeInTheDocument();
  });
});

function clientFixture(stage: DesktopStageState): YuukeiClient {
  // BubbleSurfaceが使わないAPIはstub省略し、型はunknown経由でYuukeiClientへ寄せる。
  const partial: Partial<YuukeiClient> = {
    attachSurface: vi.fn(),
    getSnapshot: vi.fn(),
    getWorldPackStatus: vi.fn(),
    getAppSettings: vi.fn(async () => ({
      talkIntervalMinutes: 5,
      actorScalePercent: 100,
      settingsPath: "/tmp/yuukei-v2/settings/app.json",
    })),
    getExtensionSettings: vi.fn(),
    getCapabilityUsage: vi.fn(),
    getActorSurfaceAssets: vi.fn(),
    setActorWindowClickThrough: vi.fn(async () => undefined),
    setBubbleSurfaceClickThrough: vi.fn(async () => undefined),
    placeBubbleSurface: vi.fn(async () => undefined),
    surfaceReady: vi.fn(async () => undefined),
    getDesktopStageState: vi.fn(async () => stage),
    reportActorStageAnchor: vi.fn(async () => undefined),
    dismissStageBubble: vi.fn(async () => undefined),
    openSettingsWindow: vi.fn(),
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
    inspectExtensionDirectory: vi.fn(),
    installExtensionDirectory: vi.fn(),
    uninstallExtension: vi.fn(),
    setExtensionEnabled: vi.fn(),
    setAppTalkIntervalMinutes: vi.fn(async (minutes: number) => ({
      talkIntervalMinutes: minutes,
      actorScalePercent: 100,
      settingsPath: "/tmp/yuukei-v2/settings/app.json",
    })),
    setAppActorScalePercent: vi.fn(async (percent: number) => ({
      talkIntervalMinutes: 5,
      actorScalePercent: percent,
      settingsPath: "/tmp/yuukei-v2/settings/app.json",
    })),
    setExtensionHookOrder: vi.fn(),
    setExtensionSettingValues: vi.fn(),
    setExtensionSecret: vi.fn(),
    onCommand: vi.fn(async () => () => undefined),
    onSnapshot: vi.fn(async () => () => undefined),
    onWorldPackStatus: vi.fn(async () => () => undefined),
    onAssetsChanged: vi.fn(async () => () => undefined),
    onAppSettings: vi.fn(async () => () => undefined),
    onStageState: vi.fn(async () => () => undefined),
  };
  return partial as unknown as YuukeiClient;
}

function findBubbleText(text: string) {
  return screen.findByText(
    (_content, element) =>
      element?.classList.contains("actor-bubble-content") === true &&
      element.textContent === text,
  );
}

type StageActorFixture = Partial<
  Omit<DesktopStageState["actors"][number], "bounds" | "anchor">
> & {
  bounds?: Partial<DesktopStageState["actors"][number]["bounds"]>;
  anchor?: Partial<DesktopStageState["actors"][number]["anchor"]>;
};

function stageState(
  bubble: Partial<DesktopStageState["bubbles"][number]> = {},
  actor: StageActorFixture = {},
): DesktopStageState {
  const bounds = {
    x: 64,
    y: 72,
    width: 420,
    height: 560,
    ...actor.bounds,
  };
  const anchor = {
    x: 260,
    y: 190,
    visible: true,
    ...actor.anchor,
  };
  return {
    monitors: [
      {
        id: "monitor-0",
        bounds: {
          x: 0,
          y: 0,
          width: 900,
          height: 640,
        },
        scaleFactor: 1,
      },
    ],
    actors: [
      {
        actorId: "yuukei",
        displayName: "Yuukei",
        windowLabel: "actor-7975756b6569",
        ...actor,
        bounds,
        anchor,
        visible: actor.visible ?? true,
      },
    ],
    bubbles: [
      {
        bubbleId: "bubble-1",
        actorId: "yuukei",
        text: "ここに出ます",
        createdAtMs: Date.now(),
        durationMs: 9000,
        speechPending: false,
        ...bubble,
      },
    ],
  };
}
