import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import { App } from "./App";
import type {
  CapabilityUsageState,
  DaihonDiagnosticEntry,
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
    daihonDiagnostics: [],
    settingsPath: "/tmp/yuukei-v2/settings/world-packs.json"
  };
}

function daihonDiagnostic(
  index: number,
  overrides: Partial<DaihonDiagnosticEntry> = {}
): DaihonDiagnosticEntry {
  return {
    phase: "loadValidate",
    severity: "error",
    code: `E-DHN-${index}`,
    message: `Daihon diagnostic ${index}`,
    scriptPath: "scripts/desktop_reactions.daihon",
    line: index,
    column: 1,
    occurredAt: `2026-06-25T00:00:0${index}.000Z`,
    ...overrides
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
    capabilityDefaults: {},
    settingsPath: "/tmp/yuukei-v2/settings/extensions.json",
    extensionRoot: "/tmp/yuukei-v2/extensions",
    trustedCodeNotice:
      "Extensionは信頼したローカルコードとして実行されます。Yuukeiは公開protocolへの入力と出力を検証しますが、OSレベルのファイルアクセス隔離はv1では行いません。"
  };
}

function capabilityUsage(
  extensions: CapabilityUsageState["extensions"] = []
): CapabilityUsageState {
  return { extensions };
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
    runtime: "process",
    permissions: { broadEventSubscription: false, eventLogRead: null },
    hooks: [
      {
        hookPoint: "beforeCommandEmit",
        commandTypes: ["dialogue.say"]
      }
    ],
    eventSubscriptions: [],
    emittedEvents: [],
    capabilities: [],
    signalAliases: [],
    settingValues: {},
    secretsSet: [],
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
    getCapabilityUsage: vi.fn(async () => capabilityUsage()),
    getActorSurfaceAssets: vi.fn(async () => ({
      worldPackId: "default-yuukei",
      actors: []
    })),
    setActorWindowClickThrough: vi.fn(async () => undefined),
    setStageOverlayClickThrough: vi.fn(async () => undefined),
    getDesktopStageState: vi.fn(async () => ({
      monitors: [],
      actors: [],
      bubbles: []
    })),
    reportActorStageAnchor: vi.fn(async () => undefined),
    dismissStageBubble: vi.fn(async () => undefined),
    openSettingsWindow: vi.fn(async () => undefined),
    sendConversationText: vi.fn(async () => [command("返事しました", "cmd_3")]),
    sendAvatarGesturePoke: vi.fn(async () => [command("つつかれました", "cmd_4")]),
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
    setExtensionSettingValues: vi.fn(),
    setExtensionSecret: vi.fn(),
    onCommand: vi.fn(async () => () => undefined),
    onSnapshot: vi.fn(async () => () => undefined),
    onWorldPackStatus: vi.fn(async () => () => undefined),
    onAssetsChanged: vi.fn(async () => () => undefined),
    onStageState: vi.fn(async () => () => undefined),
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

  it("shows up to four Daihon diagnostics until expanded", async () => {
    const status = {
      ...worldPackStatus(),
      daihonDiagnostics: [1, 2, 3, 4, 5].map((index) =>
        daihonDiagnostic(index)
      )
    };
    const client = clientFixture({
      getWorldPackStatus: vi.fn(async () => status)
    });

    render(<App client={client} />);

    expect(await screen.findByText("Daihon エラー 5件")).toBeInTheDocument();
    expect(screen.getByText("Daihon diagnostic 1")).toBeInTheDocument();
    expect(screen.getByText("Daihon diagnostic 4")).toBeInTheDocument();
    expect(screen.queryByText("Daihon diagnostic 5")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "すべて表示" }));

    expect(screen.getByText("Daihon diagnostic 5")).toBeInTheDocument();
  });

  it("refreshes World Pack diagnostics after a failed selection", async () => {
    const brokenStatus = {
      ...worldPackStatus(),
      daihonDiagnostics: [daihonDiagnostic(1)]
    };
    const client = clientFixture({
      getWorldPackStatus: vi
        .fn()
        .mockResolvedValueOnce(worldPackStatus())
        .mockResolvedValueOnce(brokenStatus),
      openWorldPackDirectory: vi.fn(async () => "/Users/example/broken-pack"),
      selectWorldPackDirectory: vi.fn(async () => {
        throw new Error("Daihon load failed");
      })
    });

    render(<App client={client} />);

    await screen.findByText("Default Yuukei");
    await userEvent.click(screen.getByRole("button", { name: "フォルダを選択" }));

    expect(await screen.findByText("Daihon load failed")).toBeInTheDocument();
    expect(await screen.findByText("Daihon diagnostic 1")).toBeInTheDocument();
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

  it("shows Extension permission rows with a broad subscription warning", async () => {
    const watcher = {
      ...installedExtension("watcher", "Watcher"),
      permissions: {
        broadEventSubscription: true,
        eventLogRead: {
          eventTypes: ["conversation.*"],
          privacyCategories: [],
          allowPayloads: true,
          allowReferences: false,
          maxRecords: 50,
          purpose: "rebuild"
        }
      },
      eventSubscriptions: [{ eventTypes: ["*"] }],
      emittedEvents: ["ext.watcher.activity"],
      capabilities: [
        {
          capability: "memory.retrieve",
          methods: ["retrieve"],
          requiredPermissions: []
        }
      ]
    };
    const client = clientFixture({
      getExtensionSettings: vi.fn(async () => extensionSettings([watcher]))
    });

    render(<App client={client} />);
    await userEvent.click(screen.getByRole("tab", { name: "Extensions" }));

    expect(await screen.findByText("Watcher")).toBeInTheDocument();
    expect(screen.getByText("全イベント購読")).toBeInTheDocument();
    expect(screen.getByText("全イベントを受け取ります")).toBeInTheDocument();
    expect(screen.getByText("event log読み出し")).toBeInTheDocument();
    expect(screen.getByText("conversation.* / max 50")).toBeInTheDocument();
    expect(screen.getByText("capability提供")).toBeInTheDocument();
    expect(screen.getByText("memory.retrieve")).toBeInTheDocument();
    expect(screen.getByText("発行イベント")).toBeInTheDocument();
    expect(screen.getByText("ext.watcher.activity")).toBeInTheDocument();
  });

  it("shows token usage per Extension and refreshes it manually", async () => {
    const intelligence = {
      ...installedExtension("yuukei-intelligence", "Yuukei Intelligence"),
      capabilities: [
        {
          capability: "dialogue.generate",
          methods: ["generate"],
          requiredPermissions: []
        }
      ]
    };
    const usage = capabilityUsage([
      {
        extensionId: "yuukei-intelligence",
        capabilities: [
          {
            capability: "dialogue.generate",
            models: [
              {
                provider: "openai-compatible",
                model: "local-model",
                allTime: {
                  requests: 3,
                  inputTokens: 1200,
                  outputTokens: 345
                },
                last7Days: {
                  requests: 1,
                  inputTokens: 400,
                  outputTokens: 90
                }
              }
            ]
          }
        ]
      }
    ]);
    const refreshedUsage = capabilityUsage([
      {
        extensionId: "yuukei-intelligence",
        capabilities: [
          {
            capability: "dialogue.generate",
            models: [
              {
                provider: "openai-compatible",
                model: "local-model",
                allTime: {
                  requests: 4,
                  inputTokens: 1500,
                  outputTokens: 444
                },
                last7Days: {
                  requests: 2,
                  inputTokens: 700,
                  outputTokens: 189
                }
              }
            ]
          }
        ]
      }
    ]);
    const client = clientFixture({
      getExtensionSettings: vi.fn(async () => extensionSettings([intelligence])),
      getCapabilityUsage: vi
        .fn()
        .mockResolvedValueOnce(usage)
        .mockResolvedValueOnce(refreshedUsage)
    });

    render(<App client={client} />);
    await userEvent.click(screen.getByRole("tab", { name: "Extensions" }));

    expect(await screen.findByText("トークン使用量")).toBeInTheDocument();
    const usageSection = screen.getByLabelText("yuukei-intelligence token usage");
    expect(within(usageSection).getByText("dialogue.generate")).toBeInTheDocument();
    expect(within(usageSection).getByText("openai-compatible / local-model")).toBeInTheDocument();
    expect(within(usageSection).getByText("リクエスト 3")).toBeInTheDocument();
    expect(within(usageSection).getByText("入力 1,200")).toBeInTheDocument();
    expect(within(usageSection).getByText("出力 345")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "使用量を更新" }));

    expect(await screen.findByText("リクエスト 4")).toBeInTheDocument();
    expect(client.getCapabilityUsage).toHaveBeenCalledTimes(2);
  });

  it("renders schema-driven Extension settings and saves visible values and secrets", async () => {
    const intelligence = {
      ...installedExtension("yuukei-intelligence", "Yuukei Intelligence"),
      settingsSchema: {
        fields: [
          {
            key: "provider",
            type: "select" as const,
            label: "プロバイダ",
            options: [
              { value: "gemini", label: "Gemini" },
              {
                value: "openai-compatible",
                label: "OpenAI互換 (LM Studio等)"
              }
            ],
            default: "openai-compatible"
          },
          {
            key: "timeoutMs",
            type: "number" as const,
            label: "タイムアウト(ms)",
            default: 30000,
            min: 1000
          },
          {
            key: "gemini.apiKey",
            type: "secret" as const,
            label: "Gemini APIキー",
            visibleWhen: { key: "provider", equals: "gemini" }
          },
          {
            key: "openaiCompatible.baseUrl",
            type: "string" as const,
            label: "OpenAI互換 Base URL",
            default: "http://127.0.0.1:1234/v1",
            visibleWhen: {
              key: "provider",
              equals: "openai-compatible"
            }
          }
        ]
      },
      settingValues: {
        provider: "openai-compatible"
      },
      secretsSet: ["gemini.apiKey"]
    };
    const client = clientFixture({
      getExtensionSettings: vi.fn(async () => extensionSettings([intelligence])),
      setExtensionSettingValues: vi.fn(async () => ({
        state: extensionSettings([
          {
            ...intelligence,
            settingValues: {
              provider: "gemini",
              timeoutMs: 45000,
              "openaiCompatible.baseUrl": "http://127.0.0.1:1234/v1"
            }
          }
        ]),
        snapshot: snapshot("設定を保存しました")
      })),
      setExtensionSecret: vi.fn(async () => ({
        state: extensionSettings([
          {
            ...intelligence,
            settingValues: { provider: "gemini", timeoutMs: 45000 },
            secretsSet: ["gemini.apiKey"]
          }
        ]),
        snapshot: snapshot("secretを保存しました")
      }))
    });

    render(<App client={client} />);
    await userEvent.click(screen.getByRole("tab", { name: "Extensions" }));

    expect(await screen.findByText("Yuukei Intelligence")).toBeInTheDocument();
    expect(screen.getByLabelText("OpenAI互換 Base URL")).toHaveValue(
      "http://127.0.0.1:1234/v1"
    );
    expect(screen.queryByLabelText("Gemini APIキー")).not.toBeInTheDocument();

    await userEvent.selectOptions(screen.getByLabelText("プロバイダ"), "gemini");
    expect(await screen.findByLabelText("Gemini APIキー")).toHaveValue("");
    expect(screen.getByPlaceholderText("設定済み")).toBeInTheDocument();
    expect(screen.queryByLabelText("OpenAI互換 Base URL")).not.toBeInTheDocument();

    await userEvent.clear(screen.getByLabelText("タイムアウト(ms)"));
    await userEvent.type(screen.getByLabelText("タイムアウト(ms)"), "45000");
    await userEvent.type(screen.getByLabelText("Gemini APIキー"), "new-secret");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(client.setExtensionSettingValues).toHaveBeenCalledWith(
        "yuukei-intelligence",
        expect.objectContaining({
          provider: "gemini",
          timeoutMs: 45000
        })
      );
    });
    expect(client.setExtensionSecret).toHaveBeenCalledWith(
      "yuukei-intelligence",
      "gemini.apiKey",
      "new-secret"
    );
  });

  it("does not save untouched default Extension setting values", async () => {
    const intelligence = {
      ...installedExtension("yuukei-intelligence", "Yuukei Intelligence"),
      settingsSchema: {
        fields: [
          {
            key: "provider",
            type: "select" as const,
            label: "プロバイダ",
            options: [
              { value: "gemini", label: "Gemini" },
              { value: "openai-compatible", label: "OpenAI互換" }
            ],
            default: "openai-compatible"
          },
          {
            key: "timeoutMs",
            type: "number" as const,
            label: "タイムアウト(ms)",
            default: 30000,
            min: 1000
          }
        ]
      },
      settingValues: {},
      secretsSet: []
    };
    const client = clientFixture({
      getExtensionSettings: vi.fn(async () => extensionSettings([intelligence])),
      setExtensionSettingValues: vi.fn(async () => ({
        state: extensionSettings([
          {
            ...intelligence,
            settingValues: { provider: "gemini" }
          }
        ]),
        snapshot: snapshot("設定を保存しました")
      }))
    });

    render(<App client={client} />);
    await userEvent.click(screen.getByRole("tab", { name: "Extensions" }));
    await screen.findByText("Yuukei Intelligence");

    expect(screen.getByLabelText("タイムアウト(ms)")).toHaveValue(30000);
    await userEvent.selectOptions(screen.getByLabelText("プロバイダ"), "gemini");
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(client.setExtensionSettingValues).toHaveBeenCalledWith(
        "yuukei-intelligence",
        { provider: "gemini" }
      );
    });
  });

  it("clears Extension secrets without exposing their values", async () => {
    const intelligence = {
      ...installedExtension("yuukei-intelligence", "Yuukei Intelligence"),
      settingsSchema: {
        fields: [
          {
            key: "provider",
            type: "select" as const,
            label: "プロバイダ",
            options: [{ value: "gemini", label: "Gemini" }],
            default: "gemini"
          },
          {
            key: "gemini.apiKey",
            type: "secret" as const,
            label: "Gemini APIキー",
            visibleWhen: { key: "provider", equals: "gemini" }
          }
        ]
      },
      settingValues: {
        provider: "gemini"
      },
      secretsSet: ["gemini.apiKey"]
    };
    const client = clientFixture({
      getExtensionSettings: vi.fn(async () => extensionSettings([intelligence])),
      setExtensionSecret: vi.fn(async () => ({
        state: extensionSettings([{ ...intelligence, secretsSet: [] }]),
        snapshot: snapshot("secretを消しました")
      }))
    });

    render(<App client={client} />);
    await userEvent.click(screen.getByRole("tab", { name: "Extensions" }));

    const secret = await screen.findByLabelText("Gemini APIキー");
    expect(secret).toHaveValue("");
    expect(screen.getByPlaceholderText("設定済み")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "クリア" }));

    await waitFor(() => {
      expect(client.setExtensionSecret).toHaveBeenCalledWith(
        "yuukei-intelligence",
        "gemini.apiKey",
        null
      );
    });
  });
});
