import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import { App } from "./App";
import type { YuukeiClient } from "./yuukeiClient";

function snapshot(bubble: string | null = null): ResidentSnapshot {
  return {
    residentId: "resident-default",
    worldPackId: "default-yuukei",
    activeSurfaceId: "surface-main",
    actors: {
      yuukei: {
        displayName: "Yuukei",
        expression: "neutral",
        motion: "idle",
        location: "desktop",
        speaking: Boolean(bubble),
        bubble: bubble ?? undefined
      }
    },
    surfaces: {},
    capabilities: {},
    recentEventCursor: "1"
  };
}

function command(text: string, id = "cmd_1"): RuntimeCommand {
  return {
    id,
    type: "dialogue.say",
    timestamp: "2026-06-25T00:00:00.000Z",
    source: "daihon",
    residentId: "resident-default",
    payload: {
      text,
      speakerId: "yuukei"
    },
    target: {
      actorId: "yuukei",
      surfaceId: "surface-main"
    }
  };
}

describe("App", () => {
  it("attaches, renders snapshot, and displays dialogue commands", async () => {
    const commandCallbacks: Array<(command: RuntimeCommand) => void> = [];
    const client: YuukeiClient = {
      attachSurface: vi.fn(async () => snapshot("ただいま")),
      getSnapshot: vi.fn(async () => snapshot("返事しました")),
      sendConversationText: vi.fn(async () => [command("返事しました", "cmd_3")]),
      onCommand: vi.fn(async (callback) => {
        commandCallbacks.push(callback);
        return () => undefined;
      }),
      onSnapshot: vi.fn(async () => () => undefined)
    };

    render(<App client={client} />);

    expect(await screen.findByText("Yuukei")).toBeInTheDocument();
    expect(await screen.findByTestId("bubble")).toHaveTextContent("ただいま");

    commandCallbacks[0]?.(command("聞こえています", "cmd_2"));
    expect(await screen.findByText("聞こえています")).toBeInTheDocument();

    await userEvent.type(screen.getByLabelText("Conversation text"), "こんにちは");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(client.sendConversationText).toHaveBeenCalledWith("こんにちは");
    });
    expect(await screen.findAllByText("返事しました")).toHaveLength(2);
  });
});
