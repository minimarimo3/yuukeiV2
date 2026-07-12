import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  bubbleTypingProgress,
  StageOverlayApp,
  stageOverlayPassthrough
} from "./StageOverlayApp";
import type { DesktopStageState, YuukeiClient } from "./yuukeiClient";

describe("StageOverlayApp", () => {
  afterEach(() => {
    cleanup();
  });

  it("keeps transparent overlay space click-through around interactive content", () => {
    expect(stageOverlayPassthrough(false)).toBe(true);
    expect(stageOverlayPassthrough(true)).toBe(false);
  });

  it("uses code-point reading progress for bubbles without pending speech", () => {
    const bubble = stageState({
      text: "A😀",
      createdAtMs: 0,
      durationMs: 1_000
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
      speechPending: true
    }).bubbles[0];

    expect(bubbleTypingProgress(bubble, 4_999)).toBe(0);
    expect(bubbleTypingProgress(bubble, 5_135)).toBe(0.5);
    expect(
      bubbleTypingProgress(
        { ...bubble, audioStartedAtMs: 6_000, audioDurationMs: 10_000 },
        6_500
      )
    ).toBe(1);
  });

  it("uses audio duration as the typing clock before fallback begins", () => {
    const bubble = stageState({
      text: "abcd",
      createdAtMs: 0,
      durationMs: 8_000,
      speechPending: true,
      audioStartedAtMs: 1_000,
      audioDurationMs: 2_000
    }).bubbles[0];

    expect(bubbleTypingProgress(bubble, 2_000)).toBe(0.5);
    expect(bubbleTypingProgress(bubble, 3_000)).toBe(1);
  });

  it("keeps the full text layout while a pending-speech bubble shows its placeholder and choices", async () => {
    const state = stageState({
      text: "全文を確保",
      createdAtMs: Date.now(),
      speechPending: true,
      choice: {
        choiceId: "choice-typing",
        choices: ["すぐ選ぶ"],
        timeoutSeconds: 30
      }
    });

    render(<StageOverlayApp client={clientFixture(state)} monitorId="monitor-0" />);

    const placeholder = await screen.findByLabelText("読み上げを待っています");
    const content = placeholder.parentElement;
    expect(content?.querySelectorAll(".actor-bubble-character")).toHaveLength(5);
    expect(content?.querySelectorAll('[data-typing-visible="false"]')).toHaveLength(5);
    expect(screen.getByRole("button", { name: "すぐ選ぶ" })).toBeInTheDocument();
  });

  it("renders stage bubbles from desktop stage state", async () => {
    render(<StageOverlayApp client={clientFixture(stageState())} monitorId="monitor-0" />);

    const bubble = await findBubbleText("ここに出ます");

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

    const bubble = await findBubbleText("ここに出ます");

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

    const bubble = await findBubbleText("ここに出ます");

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

    const bubble = await findBubbleText("ここに出ます");

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

  it("renders the conversation composer and sends text through the existing client path", async () => {
    const state = stageState();
    state.conversationComposer = {
      actorId: "yuukei",
      monitorId: "monitor-0",
      anchor: { x: 450, y: 190, visible: true }
    };
    const client = clientFixture(state);
    const user = userEvent.setup();

    render(<StageOverlayApp client={client} monitorId="monitor-0" />);

    const input = await screen.findByRole("textbox", { name: "住人に話しかける" });
    await user.type(input, "こんにちは{Control>}{Enter}{/Control}");

    await waitFor(() => {
      expect(client.sendConversationText).toHaveBeenCalledWith("こんにちは");
      expect(client.closeConversationComposer).toHaveBeenCalled();
    });
    expect(client.setStageOverlayClickThrough).toHaveBeenCalled();
  });

  it("does not render a composer belonging to another monitor", async () => {
    const state = stageState();
    state.conversationComposer = {
      actorId: "yuukei",
      monitorId: "monitor-other",
      anchor: { x: 2450, y: 190, visible: true }
    };

    render(<StageOverlayApp client={clientFixture(state)} monitorId="monitor-0" />);

    await waitFor(() => {
      expect(screen.queryByRole("textbox", { name: "住人に話しかける" })).not.toBeInTheDocument();
    });
  });

  it("refreshes the send shortcut when a composer opens", async () => {
    const state = stageState();
    state.conversationComposer = {
      actorId: "yuukei",
      monitorId: "monitor-0",
      anchor: { x: 450, y: 190, visible: true }
    };
    const client = clientFixture(state);
    let publishSettings: ((settings: Awaited<ReturnType<YuukeiClient["getAppSettings"]>>) => void) | undefined;
    client.onAppSettings = vi.fn(async (callback) => {
      publishSettings = callback;
      return () => undefined;
    });

    render(<StageOverlayApp client={client} monitorId="monitor-0" />);

    expect(await screen.findByText("Ctrl+Enterで送信")).toBeInTheDocument();
    act(() => {
      publishSettings?.({
        talkIntervalMinutes: 5,
        actorScalePercent: 100,
        conversationSendShortcut: "shiftEnter",
        settingsPath: "/tmp/yuukei-v2/settings/app.json"
      });
    });

    expect(await screen.findByText("Shift+Enterで送信")).toBeInTheDocument();
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
      conversationSendShortcut: "ctrlEnter" as const,
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
    openConversationComposer: vi.fn(),
    closeConversationComposer: vi.fn(async () => undefined),
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
      conversationSendShortcut: "ctrlEnter" as const,
      settingsPath: "/tmp/yuukei-v2/settings/app.json"
    })),
    setAppActorScalePercent: vi.fn(async (percent: number) => ({
      talkIntervalMinutes: 5,
      actorScalePercent: percent,
      conversationSendShortcut: "ctrlEnter" as const,
      settingsPath: "/tmp/yuukei-v2/settings/app.json"
    })),
    setAppConversationSendShortcut: vi.fn(),
    setExtensionHookOrder: vi.fn(),
    setExtensionSettingValues: vi.fn(),
    setExtensionSecret: vi.fn(),
    onCommand: vi.fn(async () => () => undefined),
    onSnapshot: vi.fn(async () => () => undefined),
    onWorldPackStatus: vi.fn(async () => () => undefined),
    onAssetsChanged: vi.fn(async () => () => undefined),
    onAppSettings: vi.fn(async () => () => undefined),
    onStageState: vi.fn(async () => () => undefined)
  };
  return partial as unknown as YuukeiClient;
}

function findBubbleText(text: string) {
  return screen.findByText(
    (_content, element) =>
      element?.classList.contains("actor-bubble-content") === true &&
      element.textContent === text
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
        speechPending: false,
        ...bubble
      }
    ]
  };
}
