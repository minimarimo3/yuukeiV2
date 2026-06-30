import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import { App } from "./App";
import type {
  ExtensionSettingsState,
  WorldPackSelectionState,
  YuukeiClient
} from "./yuukeiClient";

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
    extensions: {},
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

function extensionSettings(
  installed: ExtensionSettingsState["installed"] = []
): ExtensionSettingsState {
  return {
    installed,
    hookOrder: {
      beforeCommandEmit: installed.map((extension) => extension.extensionId)
    },
    settingsPath: "/tmp/yuukei-v2/settings/extensions.json",
    extensionRoot: "/tmp/yuukei-v2/extensions",
    trustedCodeNotice:
      "Extensionは信頼したローカルコードとして実行されます。Yuukeiは公開protocolへの入力と出力を検証しますが、OSレベルのファイルアクセス隔離はv1では行いません。"
  };
}

function installedExtension(
  extensionId: string,
  displayName = extensionId,
  enabled = true
): ExtensionSettingsState["installed"][number] {
  return {
    extensionId,
    displayName,
    enabled,
    hooks: [
      {
        hookPoint: "beforeCommandEmit",
        commandTypes: ["dialogue.say"]
      }
    ],
    installedPath: `/tmp/yuukei-v2/extensions/${extensionId}`,
    manifestPath: `/tmp/yuukei-v2/extensions/${extensionId}/manifest.json`,
    installedAt: "2026-06-25T00:00:00.000Z",
    updatedAt: "2026-06-25T00:00:00.000Z"
  };
}

function clientFixture(overrides: Partial<YuukeiClient> = {}): YuukeiClient {
  return {
    attachSurface: vi.fn(async () => snapshot("ただいま")),
    getSnapshot: vi.fn(async () => snapshot("返事しました")),
    getWorldPackStatus: vi.fn(async () => worldPackStatus()),
    getExtensionSettings: vi.fn(async () => extensionSettings()),
    getActorSurfaceAssets: vi.fn(async () => ({
      worldPackId: "default-yuukei",
      actors: []
    })),
    setActorWindowClickThrough: vi.fn(async () => undefined),
    openSettingsWindow: vi.fn(async () => undefined),
    sendConversationText: vi.fn(async () => [command("返事しました", "cmd_3")]),
    openWorldPackDirectory: vi.fn(async () => null),
    openExtensionDirectory: vi.fn(async () => null),
    selectWorldPackDirectory: vi.fn(),
    resetWorldPackToDefault: vi.fn(async () => ({
      status: worldPackStatus(),
      snapshot: snapshot("ただいま")
    })),
    installExtensionDirectory: vi.fn(),
    uninstallExtension: vi.fn(),
    setExtensionEnabled: vi.fn(),
    setExtensionHookOrder: vi.fn(),
    onCommand: vi.fn(async () => () => undefined),
    onSnapshot: vi.fn(async () => () => undefined),
    onAssetsChanged: vi.fn(async () => () => undefined),
    ...overrides
  };
}

describe("App", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders settings without attaching the actor surface", async () => {
    const client = clientFixture();

    render(<App client={client} />);

    expect(await screen.findByText("Default Yuukei")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "World Pack" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    expect(client.attachSurface).not.toHaveBeenCalled();
    expect(client.sendConversationText).not.toHaveBeenCalled();
  });

  it("switches settings categories without leaving the app surface", async () => {
    const client = clientFixture();

    render(<App client={client} />);

    expect(await screen.findByText("Default Yuukei")).toBeInTheDocument();
    expect(screen.queryByText("0 installed")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("tab", { name: "Extensions" }));

    expect(await screen.findByText("0 installed")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Extensions" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    expect(screen.getByRole("tab", { name: "World Pack" })).toHaveAttribute(
      "aria-selected",
      "false"
    );
    expect(screen.queryByText("Default Yuukei")).not.toBeInTheDocument();
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
    expect(screen.getByText("Default Yuukei")).toBeInTheDocument();
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
    expect(await screen.findByText("Custom Yuukei")).toBeInTheDocument();
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
    expect(screen.getByText("Default Yuukei")).toBeInTheDocument();
  });

  it("installs an Extension directory and refreshes extension state", async () => {
    const installed = installedExtension("nya-suffix", "Nya Suffix");
    const client = clientFixture({
      openExtensionDirectory: vi.fn(async () => "/Users/example/nya-suffix"),
      installExtensionDirectory: vi.fn(async () => ({
        state: extensionSettings([installed]),
        snapshot: snapshot("Extensionを読み込みました")
      }))
    });

    render(<App client={client} />);

    await userEvent.click(screen.getByRole("tab", { name: "Extensions" }));
    await screen.findByText("0 installed");
    await userEvent.click(screen.getByRole("button", { name: "追加" }));

    await waitFor(() => {
      expect(client.installExtensionDirectory).toHaveBeenCalledWith(
        "/Users/example/nya-suffix"
      );
    });
    expect(await screen.findByText("Nya Suffix")).toBeInTheDocument();
  });

  it("toggles, reorders, and uninstalls Extensions", async () => {
    const nya = installedExtension("nya-suffix", "Nya Suffix");
    const translate = installedExtension("translate-en", "Translate EN");
    const client = clientFixture({
      getExtensionSettings: vi.fn(async () =>
        extensionSettings([nya, translate])
      ),
      setExtensionEnabled: vi.fn(async () => ({
        state: extensionSettings([
          installedExtension("nya-suffix", "Nya Suffix", false),
          translate
        ]),
        snapshot: snapshot("無効にしました")
      })),
      setExtensionHookOrder: vi.fn(async () => ({
        state: {
          ...extensionSettings([translate, nya]),
          hookOrder: {
            beforeCommandEmit: ["translate-en", "nya-suffix"]
          }
        },
        snapshot: snapshot("順序を変えました")
      })),
      uninstallExtension: vi.fn(async () => ({
        state: extensionSettings([translate]),
        snapshot: snapshot("削除しました")
      }))
    });

    render(<App client={client} />);

    await userEvent.click(screen.getByRole("tab", { name: "Extensions" }));
    await screen.findByText("Nya Suffix");
    await userEvent.click(screen.getByLabelText("Nya Suffix nya-suffix"));
    await waitFor(() => {
      expect(client.setExtensionEnabled).toHaveBeenCalledWith(
        "nya-suffix",
        false
      );
    });

    await userEvent.click(screen.getAllByRole("button", { name: "下" })[0]!);
    await waitFor(() => {
      expect(client.setExtensionHookOrder).toHaveBeenCalledWith(
        "beforeCommandEmit",
        ["translate-en", "nya-suffix"]
      );
    });

    await userEvent.click(screen.getAllByRole("button", { name: "削除" })[1]!);
    await waitFor(() => {
      expect(client.uninstallExtension).toHaveBeenCalledWith("nya-suffix");
    });
  });
});
