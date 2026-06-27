import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import { App } from "./App";
import type { WorldPackSelectionState, YuukeiClient } from "./yuukeiClient";

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

function worldPackStatus(
  worldPackId = "default-yuukei",
  fallbackActive = false
): WorldPackSelectionState {
  return {
    configuredInstallId: worldPackId,
    runningInstallId: worldPackId,
    activeInstall: {
      installId: worldPackId,
      residentId: "resident-default",
      worldPackId,
      displayName: worldPackId === "default-yuukei" ? "Default Yuukei" : "Custom Yuukei",
      canonicalRoot:
        worldPackId === "default-yuukei"
          ? "/workspace/packs/default-yuukei"
          : "/Users/example/custom-pack",
      source:
        worldPackId === "default-yuukei"
          ? "bundledDefault"
          : "externalDirectory"
    },
    installs: [],
    fallbackActive,
    lastLoadError: fallbackActive ? "pack.json is missing" : undefined,
    settingsPath: "/tmp/yuukei-v2/settings/world-packs.json"
  };
}

function clientFixture(overrides: Partial<YuukeiClient> = {}): YuukeiClient {
  return {
    attachSurface: vi.fn(async () => snapshot("ただいま")),
    getSnapshot: vi.fn(async () => snapshot("返事しました")),
    getWorldPackStatus: vi.fn(async () => worldPackStatus()),
    sendConversationText: vi.fn(async () => [command("返事しました", "cmd_3")]),
    openWorldPackDirectory: vi.fn(async () => null),
    selectWorldPackDirectory: vi.fn(),
    resetWorldPackToDefault: vi.fn(async () => ({
      status: worldPackStatus(),
      snapshot: snapshot("ただいま")
    })),
    onCommand: vi.fn(async () => () => undefined),
    onSnapshot: vi.fn(async () => () => undefined),
    ...overrides
  };
}

describe("App", () => {
  afterEach(() => {
    cleanup();
  });

  it("attaches, renders snapshot, and displays dialogue commands", async () => {
    const commandCallbacks: Array<(command: RuntimeCommand) => void> = [];
    const client = clientFixture({
      onCommand: vi.fn(async (callback) => {
        commandCallbacks.push(callback);
        return () => undefined;
      })
    });

    render(<App client={client} />);

    expect(await screen.findByText("Yuukei")).toBeInTheDocument();
    expect(await screen.findByTestId("bubble")).toHaveTextContent("ただいま");
    expect(await screen.findByText("Default Yuukei")).toBeInTheDocument();

    commandCallbacks[0]?.(command("聞こえています", "cmd_2"));
    expect(await screen.findByText("聞こえています")).toBeInTheDocument();

    await userEvent.type(screen.getByLabelText("Conversation text"), "こんにちは");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(client.sendConversationText).toHaveBeenCalledWith("こんにちは");
    });
    expect(await screen.findAllByText("返事しました")).toHaveLength(2);
  });

  it("ignores a canceled World Pack directory dialog", async () => {
    const client = clientFixture({
      openWorldPackDirectory: vi.fn(async () => null),
      selectWorldPackDirectory: vi.fn()
    });

    render(<App client={client} />);

    await screen.findByText("Default Yuukei");
    await userEvent.click(screen.getByRole("button", { name: "フォルダを選択" }));

    await waitFor(() => {
      expect(client.openWorldPackDirectory).toHaveBeenCalled();
    });
    expect(client.selectWorldPackDirectory).not.toHaveBeenCalled();
    expect(screen.getByTestId("bubble")).toHaveTextContent("ただいま");
  });

  it("switches to a selected World Pack and refreshes the snapshot", async () => {
    const customSnapshot = snapshot("外部Packです");
    customSnapshot.worldPackId = "custom-yuukei";
    customSnapshot.actors.yuukei!.displayName = "Custom Yuukei";
    const client = clientFixture({
      openWorldPackDirectory: vi.fn(async () => "/Users/example/custom-pack"),
      selectWorldPackDirectory: vi.fn(async () => ({
        status: worldPackStatus("custom-yuukei"),
        snapshot: customSnapshot
      }))
    });

    render(<App client={client} />);

    await screen.findByText("Default Yuukei");
    await userEvent.click(screen.getByRole("button", { name: "フォルダを選択" }));

    await waitFor(() => {
      expect(client.selectWorldPackDirectory).toHaveBeenCalledWith(
        "/Users/example/custom-pack"
      );
    });
    expect(await screen.findAllByText("Custom Yuukei")).toHaveLength(2);
    expect(screen.getByTestId("bubble")).toHaveTextContent("外部Packです");
  });

  it("shows World Pack selection errors without replacing the current snapshot", async () => {
    const client = clientFixture({
      openWorldPackDirectory: vi.fn(async () => "/Users/example/broken-pack"),
      selectWorldPackDirectory: vi.fn(async () => {
        throw new Error("pack.json is missing");
      })
    });

    render(<App client={client} />);

    await screen.findByText("Default Yuukei");
    await userEvent.click(screen.getByRole("button", { name: "フォルダを選択" }));

    expect(await screen.findByText("pack.json is missing")).toBeInTheDocument();
    expect(screen.getByTestId("bubble")).toHaveTextContent("ただいま");
  });
});
