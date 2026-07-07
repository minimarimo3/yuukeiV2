import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import { App } from "./App";
import type {
  AppSettingsState,
  CapabilityUsageState,
  DaihonDiagnosticEntry,
  EventLogPage,
  EventLogRecord,
  ExtensionSettingsState,
  ObservationSettingsState,
  OnboardingState,
  ResidentMemoryState,
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

function appSettings(talkIntervalMinutes = 5): AppSettingsState {
  return {
    talkIntervalMinutes,
    settingsPath: "/tmp/yuukei-v2/settings/app.json"
  };
}

function observationSettings(
  overrides: Partial<ObservationSettingsState> = {}
): ObservationSettingsState {
  return {
    windows: false,
    folders: false,
    downloads: false,
    settingsPath: "/tmp/yuukei-v2/settings/observations.json",
    ...overrides
  };
}

function onboardingState(
  overrides: Partial<OnboardingState> = {}
): OnboardingState {
  return {
    completed: true,
    completedAt: "2026-07-07T00:00:00.000Z",
    settingsPath: "/tmp/yuukei-v2/settings/onboarding.json",
    ...overrides
  };
}

function capabilityUsage(
  extensions: CapabilityUsageState["extensions"] = []
): CapabilityUsageState {
  return { extensions };
}

function residentMemories(): ResidentMemoryState {
  return {
    facts: [
      {
        id: "fact-1",
        text: "唐揚げが好き。",
        createdAt: "2026-06-25T00:00:00.000Z",
        updatedAt: "2026-06-25T00:00:00.000Z"
      }
    ],
    episodes: [
      {
        id: "episode-1",
        text: "昨日は公園へ行った。",
        timestamp: "2026-06-26T00:00:00.000Z"
      }
    ],
    episodeTotal: 1
  };
}

function eventLogRecord(
  sequence: number,
  kind: string,
  payload: Record<string, unknown> = { text: "こんにちは" }
): EventLogRecord {
  return {
    sequence,
    id: `evt_${sequence}`,
    kind,
    timestamp: `2026-07-0${sequence}T00:00:00.000Z`,
    residentId: "resident-default",
    source: "test",
    payload,
    privacy: kind.startsWith("desktop.")
      ? {
          category: "desktop-observation",
          retention: "short",
          extensionReadable: false
        }
      : null
  };
}

function eventLogPage(records: EventLogRecord[] = [
  eventLogRecord(3, "desktop.download.completed", {
    fileName: "photo.png",
    fileCategory: "image"
  }),
  eventLogRecord(2, "dialogue.say", { text: "おはよう" }),
  eventLogRecord(1, "conversation.text", { text: "hello" })
]): EventLogPage {
  return {
    records,
    nextCursor: null,
    total: records.length
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
    getAppSettings: vi.fn(async () => appSettings()),
    getObservationSettings: vi.fn(async () => observationSettings()),
    getOnboardingState: vi.fn(async () => onboardingState()),
    completeOnboarding: vi.fn(async () => onboardingState()),
    setObservationSettings: vi.fn(async (settings) =>
      observationSettings(settings)
    ),
    getExtensionSettings: vi.fn(async () => extensionSettings()),
    getCapabilityUsage: vi.fn(async () => capabilityUsage()),
    listResidentMemories: vi.fn(async () => residentMemories()),
    updateResidentMemory: vi.fn(async () => ({ updated: true })),
    forgetResidentMemories: vi.fn(async () => ({
      removedFacts: 1,
      removedEpisodes: 0
    })),
    readEventLogPage: vi.fn(async () => eventLogPage()),
    countEventLogDeleteBefore: vi.fn(async () => 2),
    countEventLogDeleteByKindPrefix: vi.fn(async () => 1),
    countEventLogDeleteAll: vi.fn(async () => 3),
    deleteEventLogBefore: vi.fn(async () => ({ deleted: 2 })),
    deleteEventLogByKindPrefix: vi.fn(async () => ({ deleted: 1 })),
    deleteEventLogAll: vi.fn(async () => ({ deleted: 3 })),
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
    sendConversationChoice: vi.fn(async () => []),
    sendAvatarGesturePoke: vi.fn(async () => [command("つつかれました", "cmd_4")]),
    openWorldPackDirectory: vi.fn(async () => null),
    openWorldPackZip: vi.fn(async () => null),
    openExtensionDirectory: vi.fn(async () => null),
    selectWorldPackDirectory: vi.fn(),
    inspectWorldPackZip: vi.fn(async () => ({
      packId: "zip-yuukei",
      displayName: "Zip Yuukei",
      licenseText: "配布条件です。",
      licenseSource: "LICENSE",
      importedRoot: "/tmp/yuukei-v2/packs-imported/zip-yuukei",
      replacesExisting: false
    })),
    importWorldPackZip: vi.fn(async () => ({
      status: worldPackStatus("zip-yuukei"),
      snapshot: snapshot("zipです")
    })),
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
    setAppTalkIntervalMinutes: vi.fn(async (minutes: number) =>
      appSettings(minutes)
    ),
    onCommand: vi.fn(async () => () => undefined),
    onSnapshot: vi.fn(async () => () => undefined),
    onWorldPackStatus: vi.fn(async () => () => undefined),
    onOnboardingDismissed: vi.fn(async () => () => undefined),
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

  it("shows onboarding for an initial launch", async () => {
    const client = clientFixture({
      getOnboardingState: vi.fn(async () =>
        onboardingState({ completed: false, completedAt: null })
      )
    });

    render(<App client={client} />);

    expect(await screen.findByRole("heading", { name: "ようこそ" })).toBeInTheDocument();
    expect(screen.getByText("Default Yuukei")).toBeInTheDocument();
    expect(
      screen.getByText("この子はあなたのデバイスに住みます。")
    ).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "World Pack" })).not.toBeInTheDocument();
  });

  it("skips the AI step when starting without AI", async () => {
    const client = clientFixture({
      getOnboardingState: vi.fn(async () =>
        onboardingState({ completed: false, completedAt: null })
      )
    });

    render(<App client={client} />);

    await userEvent.click(await screen.findByRole("button", { name: "次へ" }));
    expect(
      await screen.findByRole("heading", { name: "AI(ことば)の設定" })
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "AIなしで始める" }));

    expect(
      await screen.findByRole("heading", { name: "観測とプライバシー" })
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "アプリ名とウィンドウの出現・消滅だけを記録します(タイトルは記録しません)"
      )
    ).toBeInTheDocument();
  });

  it("completes onboarding and returns to the normal settings screen", async () => {
    const client = clientFixture({
      getOnboardingState: vi.fn(async () =>
        onboardingState({ completed: false, completedAt: null })
      ),
      completeOnboarding: vi.fn(async () => onboardingState())
    });

    render(<App client={client} />);

    await userEvent.click(await screen.findByRole("button", { name: "次へ" }));
    await userEvent.click(screen.getByRole("button", { name: "AIなしで始める" }));
    await userEvent.click(screen.getByRole("button", { name: "次へ" }));
    await userEvent.click(screen.getByRole("button", { name: "完了して始める" }));

    await waitFor(() => expect(client.completeOnboarding).toHaveBeenCalled());
    expect(await screen.findByRole("tab", { name: "World Pack" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    expect(screen.queryByRole("heading", { name: "完了" })).not.toBeInTheDocument();
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

  it("lists resident memories and saves fact edits", async () => {
    const client = clientFixture();

    render(<App client={client} />);

    await userEvent.click(await screen.findByRole("tab", { name: "記憶" }));
    const factText = await screen.findByText("唐揚げが好き。");
    const factRow = factText.closest("article");
    expect(factRow).not.toBeNull();

    await userEvent.click(
      within(factRow as HTMLElement).getByRole("button", { name: "編集" })
    );
    const editor = screen.getByRole("textbox", { name: "fact fact-1" });
    await userEvent.clear(editor);
    await userEvent.type(editor, "唐揚げと散歩が好き。");
    await userEvent.click(
      within(factRow as HTMLElement).getByRole("button", { name: "保存" })
    );

    await waitFor(() =>
      expect(client.updateResidentMemory).toHaveBeenCalledWith(
        "fact",
        "fact-1",
        "唐揚げと散歩が好き。"
      )
    );
  });

  it("forgets memories from the settings panel", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const client = clientFixture();

    render(<App client={client} />);

    await userEvent.click(await screen.findByRole("tab", { name: "記憶" }));
    const episodeText = await screen.findByText("昨日は公園へ行った。");
    const episodeRow = episodeText.closest("article");
    expect(episodeRow).not.toBeNull();
    await userEvent.click(
      within(episodeRow as HTMLElement).getByRole("button", { name: "削除" })
    );
    await waitFor(() =>
      expect(client.forgetResidentMemories).toHaveBeenCalledWith(
        [{ kind: "episode", id: "episode-1" }],
        false
      )
    );

    await userEvent.click(screen.getByRole("button", { name: "すべて忘れる" }));
    expect(confirm).toHaveBeenCalledWith(
      "すべての記憶を忘れます。この操作は取り消せません。続けますか？"
    );
    await waitFor(() =>
      expect(client.forgetResidentMemories).toHaveBeenCalledWith(undefined, true)
    );
    confirm.mockRestore();
  });

  it("saves app talk interval settings", async () => {
    const client = clientFixture();

    render(<App client={client} />);

    await userEvent.click(screen.getByRole("tab", { name: "App" }));
    const input = await screen.findByRole("spinbutton", {
      name: /おしゃべりの間隔/
    });
    await userEvent.clear(input);
    await userEvent.type(input, "12");

    await waitFor(() => {
      expect(client.setAppTalkIntervalMinutes).toHaveBeenLastCalledWith(12);
    });
  });

  it("toggles observation privacy settings", async () => {
    const client = clientFixture();

    render(<App client={client} />);

    await userEvent.click(await screen.findByRole("tab", { name: "観測" }));
    expect(await screen.findByText("観測とプライバシー")).toBeInTheDocument();
    expect(
      screen.getByText(
        "開いた場所の種類だけを記録します(パスは記録しません)"
      )
    ).toBeInTheDocument();

    await userEvent.click(screen.getByRole("checkbox", { name: "フォルダ" }));

    await waitFor(() => {
      expect(client.setObservationSettings).toHaveBeenCalledWith({
        windows: false,
        folders: true,
        downloads: false
      });
    });
  });

  it("lists event log records with payload summaries", async () => {
    const client = clientFixture();

    render(<App client={client} />);

    await userEvent.click(await screen.findByRole("tab", { name: "生活の記録" }));

    expect(await screen.findByText("fileName: photo.png")).toBeInTheDocument();
    expect(screen.getByText(/desktop.download.completed/)).toBeInTheDocument();
    expect(screen.getAllByText(/desktop-observation/).length).toBeGreaterThan(1);
    expect(screen.getByText("text: おはよう")).toBeInTheDocument();
  });

  it("confirms and deletes event log records before a timestamp", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const client = clientFixture();

    render(<App client={client} />);

    await userEvent.click(await screen.findByRole("tab", { name: "生活の記録" }));
    const input = await screen.findByLabelText("この日時より前");
    await userEvent.type(input, "2026-07-08T12:00");
    await userEvent.click(screen.getByRole("button", { name: "期間指定で削除" }));

    await waitFor(() =>
      expect(client.countEventLogDeleteBefore).toHaveBeenCalledWith(
        "2026-07-08T03:00:00.000Z"
      )
    );
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("削除予定: 2件"));
    expect(confirm).toHaveBeenCalledWith(
      expect.stringContaining("住人の記憶(要約)には残っている場合があります。")
    );
    await waitFor(() =>
      expect(client.deleteEventLogBefore).toHaveBeenCalledWith(
        "2026-07-08T03:00:00.000Z"
      )
    );
    confirm.mockRestore();
  });

  it("confirms and deletes all event log records", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const client = clientFixture();

    render(<App client={client} />);

    await userEvent.click(await screen.findByRole("tab", { name: "生活の記録" }));
    await userEvent.click(screen.getByRole("button", { name: "全削除" }));

    await waitFor(() => expect(client.countEventLogDeleteAll).toHaveBeenCalled());
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("削除予定: 3件"));
    await waitFor(() => expect(client.deleteEventLogAll).toHaveBeenCalled());
    confirm.mockRestore();
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

  it("imports a World Pack from zip after license confirmation", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const client = clientFixture({
      openWorldPackZip: vi.fn(async () => "/Users/example/zip-yuukei.zip"),
      inspectWorldPackZip: vi.fn(async () => ({
        packId: "zip-yuukei",
        displayName: "Zip Yuukei",
        licenseText: "このPackの配布条件です。",
        licenseSource: "LICENSE",
        importedRoot: "/tmp/yuukei-v2/packs-imported/zip-yuukei",
        replacesExisting: true
      })),
      importWorldPackZip: vi.fn(async () => ({
        status: worldPackStatus("zip-yuukei"),
        snapshot: snapshot("zipです")
      }))
    });

    render(<App client={client} />);

    await screen.findByText("Default Yuukei");
    await userEvent.click(screen.getByRole("button", { name: "zipから読み込む" }));

    await waitFor(() =>
      expect(client.inspectWorldPackZip).toHaveBeenCalledWith(
        "/Users/example/zip-yuukei.zip"
      )
    );
    expect(confirm).toHaveBeenCalledWith(
      expect.stringContaining("このPackの配布条件です。")
    );
    expect(confirm).toHaveBeenCalledWith(
      expect.stringContaining("続行すると置き換えます。")
    );
    await waitFor(() =>
      expect(client.importWorldPackZip).toHaveBeenCalledWith(
        "/Users/example/zip-yuukei.zip"
      )
    );
    expect(await screen.findByText("Custom Yuukei")).toBeInTheDocument();
    confirm.mockRestore();
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
