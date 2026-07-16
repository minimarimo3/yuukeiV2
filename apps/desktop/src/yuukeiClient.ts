import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  ExtensionCapabilityDeclaration,
  ExtensionEventSubscription,
  ExtensionHealth,
  ExtensionHookPoint,
  ExtensionHookSubscription,
  ExtensionPermissions,
  ExtensionRuntimeKind,
  ExtensionSettingsSchema,
  ExtensionSignalAlias,
  ResidentSnapshot,
  RuntimeCommand,
} from "@yuukei/protocol";

export type WorldPackSource = "bundledDefault" | "externalDirectory";

export type WorldPackInstall = {
  installId: string;
  residentId: string;
  worldPackId: string;
  displayName: string;
  canonicalRoot: string;
  source: WorldPackSource;
  lastLoadError?: string;
};

export type DaihonDiagnosticSeverity = "error" | "warning" | "info";

export type DaihonDiagnosticPhase =
  | "loadParse"
  | "loadValidate"
  | "loadSpeaker"
  | "runtimeValidate"
  | "runtimeExecute";

export type DaihonDiagnosticEntry = {
  phase: DaihonDiagnosticPhase;
  severity: DaihonDiagnosticSeverity;
  code: string;
  message: string;
  scriptPath?: string;
  line?: number;
  column?: number;
  help?: string;
  occurredAt?: string;
  installId?: string;
  worldPackId?: string;
  packRoot?: string;
  sourceEventType?: string;
  sourceEventId?: string;
};

export type WorldPackSelectionState = {
  configuredInstallId: string;
  runningInstallId: string;
  activeInstall: WorldPackInstall;
  installs: WorldPackInstall[];
  fallbackActive: boolean;
  lastLoadError?: string;
  daihonDiagnostics: DaihonDiagnosticEntry[];
  settingsPath: string;
};

export type WorldPackSwitchResult = {
  status: WorldPackSelectionState;
  snapshot: ResidentSnapshot;
};

export type WorldPackZipInspection = {
  packId: string;
  displayName: string;
  licenseText?: string | null;
  licenseSource?: string | null;
  importedRoot: string;
  replacesExisting: boolean;
};

export type InstalledExtension = {
  extensionId: string;
  displayName: string;
  enabled: boolean;
  runtime: ExtensionRuntimeKind;
  permissions: ExtensionPermissions;
  hooks: ExtensionHookSubscription[];
  eventSubscriptions: ExtensionEventSubscription[];
  emittedEvents: string[];
  capabilities: ExtensionCapabilityDeclaration[];
  signalAliases: ExtensionSignalAlias[];
  settingsSchema?: ExtensionSettingsSchema;
  settingValues: Record<string, unknown>;
  secretsSet: string[];
  installedPath: string;
  manifestPath: string;
  installedAt: string;
  updatedAt: string;
  lastLoadError?: string;
  runtimeStatus?: {
    health: ExtensionHealth;
    failureCount: number;
    suspended: boolean;
    message?: string | null;
  };
};

export type ExtensionSettingsState = {
  installed: InstalledExtension[];
  hookOrder: Partial<Record<ExtensionHookPoint, string[]>>;
  capabilityDefaults: Record<string, string>;
  settingsPath: string;
  extensionRoot: string;
  trustedCodeNotice: string;
};

export type ExtensionSettingsChangeResult = {
  state: ExtensionSettingsState;
  snapshot: ResidentSnapshot;
};

export type AppSettingsState = {
  talkIntervalMinutes: number;
  actorScalePercent: number;
  conversationSendShortcut: ConversationSendShortcut;
  settingsPath: string;
};

export type ConversationSendShortcut = "ctrlEnter" | "enter" | "shiftEnter";

export type RuntimeSettingsState = {
  llmTimeoutMs: number;
  recentContextCount: number;
  talkDesireLow: number;
  talkDesireHigh: number;
  settingsPath: string;
};

export type RuntimeSettingsUpdate = {
  llmTimeoutMs: number;
  recentContextCount: number;
  talkDesireLow: number;
  talkDesireHigh: number;
};

export type SceneHistoryEntry = {
  eventName: string;
  sceneName: string;
  lastExecutedAt: string;
};

export type SceneHistoryState = {
  installId: string;
  historyPath: string;
  entries: SceneHistoryEntry[];
};

export type ObservationSettingsState = {
  windows: boolean;
  folders: boolean;
  downloads: boolean;
  settingsPath: string;
};

export type ObservationSettingsUpdate = {
  windows: boolean;
  folders: boolean;
  downloads: boolean;
};

export type OnboardingState = {
  completed: boolean;
  completedAt?: string | null;
  dismissed: boolean;
  dismissedAt?: string | null;
  settingsPath: string;
};

export type TokenUsageTotals = {
  requests: number;
  inputTokens: number;
  outputTokens: number;
};

export type ModelCapabilityUsage = {
  provider: string;
  model: string;
  allTime: TokenUsageTotals;
  last7Days: TokenUsageTotals;
};

export type CapabilityUsageByCapability = {
  capability: string;
  models: ModelCapabilityUsage[];
};

export type ExtensionCapabilityUsage = {
  extensionId: string;
  capabilities: CapabilityUsageByCapability[];
};

export type CapabilityUsageState = {
  extensions: ExtensionCapabilityUsage[];
};

export type MemoryEntryKind = "fact" | "episode";

export type ResidentMemoryFact = {
  id: string;
  text: string;
  createdAt: string;
  updatedAt: string;
};

export type ResidentMemoryEpisode = {
  id: string;
  text: string;
  timestamp: string;
};

export type ResidentMemoryState = {
  facts: ResidentMemoryFact[];
  episodes: ResidentMemoryEpisode[];
  episodeTotal: number;
};

export type MemoryForgetEntry = {
  kind: MemoryEntryKind;
  id: string;
};

export type MemoryUpdateResult = {
  updated: boolean;
};

export type MemoryForgetResult = {
  removedFacts: number;
  removedEpisodes: number;
};

export type EventLogPrivacyCategoryFilter =
  | "all"
  | "desktopObservation"
  | "none";

export type EventLogRecord = {
  sequence: number;
  id: string;
  kind: string;
  timestamp: string;
  residentId: string;
  source: string;
  deviceId?: string | null;
  surfaceId?: string | null;
  actorId?: string | null;
  payload: Record<string, unknown>;
  privacy?: {
    category: string;
    retention: string;
    extensionReadable: boolean;
  } | null;
};

export type EventLogPage = {
  records: EventLogRecord[];
  nextCursor?: number | null;
  total: number;
};

export type EventLogDeleteResult = {
  deleted: number;
};

export type ActorSurfaceAssetCatalog = {
  worldPackId: string;
  actors: ActorSurfaceAsset[];
};

export type ActorSurfaceAsset = {
  actorId: string;
  displayName: string;
  renderer?: ActorSurfaceRendererAsset;
};

export type ActorSurfaceRendererAsset = {
  kind: "vrm";
  modelUrl: string;
  motions: Record<string, string>;
  hitZones: ActorHitZoneDefinition[];
};

export type ActorHitZoneDefinition = {
  id: string;
  label?: string;
  source: "humanoidBone" | "nodeName";
  bones?: string[];
  nodes?: string[];
  shape?: "auto" | "mesh";
  events?: string[];
  priority?: number;
};

export type AvatarGesturePokeInput = {
  actorId: string;
  hitZoneId: string;
  hitZoneLabel?: string;
  hitSurface?: string;
  hitBone?: string;
  input: {
    kind: "pointer";
    button: string;
  };
  screen: {
    x: number;
    y: number;
  };
};

export type StageRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type StageAnchor = {
  x: number;
  y: number;
  visible: boolean;
};

export type StageMonitor = {
  id: string;
  label: string;
  name?: string;
  bounds: StageRect;
  scaleFactor: number;
};

export type StageActor = {
  actorId: string;
  displayName: string;
  windowLabel: string;
  bounds: StageRect;
  anchor: StageAnchor;
  visible: boolean;
};

export type StageBubble = {
  bubbleId: string;
  actorId: string;
  text: string;
  choice?: {
    choiceId: string;
    choices: string[];
    timeoutSeconds: number;
  };
  createdAtMs: number;
  durationMs: number;
  speechPending: boolean;
  audioStartedAtMs?: number;
  audioDurationMs?: number;
};

export type DesktopStageState = {
  monitors: StageMonitor[];
  actors: StageActor[];
  bubbles: StageBubble[];
  conversationComposer?: DesktopConversationComposer;
};

export type DesktopConversationComposer = {
  actorId: string;
  monitorId: string;
  anchor: StageAnchor;
};

export type ActorWindowDragStarted = { sessionId: string };
export type ActorWindowDragFinished = {
  actorId: string;
  movedDistance: number;
};

export type YuukeiClient = {
  attachSurface(): Promise<ResidentSnapshot>;
  getSnapshot(): Promise<ResidentSnapshot>;
  getWorldPackStatus(): Promise<WorldPackSelectionState>;
  getExtensionSettings(): Promise<ExtensionSettingsState>;
  getAppSettings(): Promise<AppSettingsState>;
  getRuntimeSettings(): Promise<RuntimeSettingsState>;
  getSceneHistory(): Promise<SceneHistoryState>;
  getAutostartEnabled(): Promise<boolean>;
  getAutostartCanEnable?(): Promise<boolean>;
  setAutostartEnabled(enabled: boolean): Promise<boolean>;
  surfaceReady?(): Promise<void>;
  getObservationSettings(): Promise<ObservationSettingsState>;
  getOnboardingState(): Promise<OnboardingState>;
  completeOnboarding(): Promise<OnboardingState>;
  dismissOnboarding(): Promise<OnboardingState>;
  setObservationSettings(
    settings: ObservationSettingsUpdate,
  ): Promise<ObservationSettingsState>;
  getCapabilityUsage(): Promise<CapabilityUsageState>;
  listResidentMemories(
    episodeLimit?: number,
    episodeOffset?: number,
  ): Promise<ResidentMemoryState>;
  updateResidentMemory(
    kind: MemoryEntryKind,
    id: string,
    text: string,
  ): Promise<MemoryUpdateResult>;
  forgetResidentMemories(
    entries?: MemoryForgetEntry[],
    all?: boolean,
  ): Promise<MemoryForgetResult>;
  readEventLogPage(
    kindPrefix?: string,
    privacyCategory?: EventLogPrivacyCategoryFilter,
    beforeSequence?: number,
    limit?: number,
  ): Promise<EventLogPage>;
  countEventLogDeleteBefore(timestamp: string): Promise<number>;
  countEventLogDeleteByKindPrefix(prefix: string): Promise<number>;
  countEventLogDeleteAll(): Promise<number>;
  deleteEventLogBefore(timestamp: string): Promise<EventLogDeleteResult>;
  deleteEventLogByKindPrefix(prefix: string): Promise<EventLogDeleteResult>;
  deleteEventLogAll(): Promise<EventLogDeleteResult>;
  getActorSurfaceAssets(): Promise<ActorSurfaceAssetCatalog>;
  setActorWindowClickThrough(passthrough: boolean): Promise<void>;
  setStageOverlayClickThrough(passthrough: boolean): Promise<void>;
  getDesktopStageState(): Promise<DesktopStageState>;
  reportActorStageAnchor(actorId: string, anchor: StageAnchor): Promise<void>;
  dismissStageBubble(bubbleId: string): Promise<void>;
  openSettingsWindow(): Promise<void>;
  openConversationComposer(actorId: string): Promise<void>;
  closeConversationComposer(): Promise<void>;
  sendConversationText(text: string): Promise<RuntimeCommand[]>;
  sendConversationChoice(
    choiceId: string,
    choice: string,
    index: number,
  ): Promise<RuntimeCommand[]>;
  sendAvatarGesturePoke(
    gesture: AvatarGesturePokeInput,
  ): Promise<RuntimeCommand[]>;
  beginActorWindowDrag(actorId: string): Promise<ActorWindowDragStarted>;
  moveActorWindowDrag(
    actorId: string,
    sessionId: string,
    dx: number,
    dy: number,
  ): Promise<void>;
  finishActorWindowDrag(
    actorId: string,
    sessionId: string,
  ): Promise<ActorWindowDragFinished>;
  cancelActorWindowDrag(actorId: string, sessionId: string): Promise<void>;
  notifyAvatarGestureGrab(actorId: string): Promise<RuntimeCommand[]>;
  notifyAvatarGestureDrop(
    actorId: string,
    movedDistance: number,
  ): Promise<RuntimeCommand[]>;
  openWorldPackDirectory(): Promise<string | null>;
  openWorldPackZip(): Promise<string | null>;
  openExtensionDirectory(): Promise<string | null>;
  selectWorldPackDirectory(path: string): Promise<WorldPackSwitchResult>;
  inspectWorldPackZip(path: string): Promise<WorldPackZipInspection>;
  importWorldPackZip(path: string): Promise<WorldPackSwitchResult>;
  resetWorldPackToDefault(): Promise<WorldPackSwitchResult>;
  installExtensionDirectory(
    path: string,
  ): Promise<ExtensionSettingsChangeResult>;
  uninstallExtension(
    extensionId: string,
  ): Promise<ExtensionSettingsChangeResult>;
  setExtensionEnabled(
    extensionId: string,
    enabled: boolean,
  ): Promise<ExtensionSettingsChangeResult>;
  setExtensionHookOrder(
    hookPoint: ExtensionHookPoint,
    extensionIds: string[],
  ): Promise<ExtensionSettingsChangeResult>;
  setExtensionSettingValues(
    extensionId: string,
    values: Record<string, unknown>,
  ): Promise<ExtensionSettingsChangeResult>;
  setExtensionSecret(
    extensionId: string,
    key: string,
    value: string | null,
  ): Promise<ExtensionSettingsChangeResult>;
  restartExtensionProcess(extensionId: string): Promise<ExtensionSettingsState>;
  setAppTalkIntervalMinutes(minutes: number): Promise<AppSettingsState>;
  setAppActorScalePercent(percent: number): Promise<AppSettingsState>;
  setAppConversationSendShortcut(
    shortcut: ConversationSendShortcut,
  ): Promise<AppSettingsState>;
  setRuntimeSettings(
    settings: RuntimeSettingsUpdate,
  ): Promise<RuntimeSettingsState>;
  resetSceneHistory(): Promise<SceneHistoryState>;
  onCommand(callback: (command: RuntimeCommand) => void): Promise<() => void>;
  onSnapshot(
    callback: (snapshot: ResidentSnapshot) => void,
  ): Promise<() => void>;
  onWorldPackStatus(
    callback: (status: WorldPackSelectionState) => void,
  ): Promise<() => void>;
  onOnboardingDismissed(callback: () => void): Promise<() => void>;
  onAssetsChanged(
    callback: (catalog: ActorSurfaceAssetCatalog) => void,
  ): Promise<() => void>;
  onStageState(
    callback: (state: DesktopStageState) => void,
  ): Promise<() => void>;
  onAppSettings(
    callback: (settings: AppSettingsState) => void,
  ): Promise<() => void>;
};

export const tauriYuukeiClient: YuukeiClient = {
  attachSurface: () => invoke<ResidentSnapshot>("attach_surface"),
  getSnapshot: () => invoke<ResidentSnapshot>("get_snapshot"),
  getWorldPackStatus: () =>
    invoke<WorldPackSelectionState>("get_world_pack_status"),
  getExtensionSettings: () =>
    invoke<ExtensionSettingsState>("get_extension_settings"),
  getAppSettings: () => invoke<AppSettingsState>("get_app_settings"),
  getRuntimeSettings: () =>
    invoke<RuntimeSettingsState>("get_runtime_settings"),
  getSceneHistory: () => invoke<SceneHistoryState>("get_scene_history"),
  getAutostartEnabled: () => invoke<boolean>("get_autostart_enabled"),
  getAutostartCanEnable: () => invoke<boolean>("get_autostart_can_enable"),
  setAutostartEnabled: (enabled: boolean) =>
    invoke<boolean>("set_autostart_enabled", { enabled }),
  surfaceReady: () => invoke<void>("surface_ready"),
  getObservationSettings: () =>
    invoke<ObservationSettingsState>("get_observation_settings"),
  getOnboardingState: () => invoke<OnboardingState>("get_onboarding_state"),
  completeOnboarding: () => invoke<OnboardingState>("complete_onboarding"),
  dismissOnboarding: () => invoke<OnboardingState>("dismiss_onboarding"),
  setObservationSettings: (settings: ObservationSettingsUpdate) =>
    invoke<ObservationSettingsState>("set_observation_settings", { settings }),
  getCapabilityUsage: () =>
    invoke<CapabilityUsageState>("get_capability_usage"),
  listResidentMemories: (episodeLimit?: number, episodeOffset?: number) =>
    invoke<ResidentMemoryState>("list_resident_memories", {
      episodeLimit,
      episodeOffset,
    }),
  updateResidentMemory: (kind: MemoryEntryKind, id: string, text: string) =>
    invoke<MemoryUpdateResult>("update_resident_memory", { kind, id, text }),
  forgetResidentMemories: (entries?: MemoryForgetEntry[], all?: boolean) =>
    invoke<MemoryForgetResult>("forget_resident_memories", { entries, all }),
  readEventLogPage: (
    kindPrefix?: string,
    privacyCategory: EventLogPrivacyCategoryFilter = "all",
    beforeSequence?: number,
    limit?: number,
  ) =>
    invoke<EventLogPage>("read_event_log_page", {
      kindPrefix,
      privacyCategory,
      beforeSequence,
      limit,
    }),
  countEventLogDeleteBefore: (timestamp: string) =>
    invoke<number>("count_event_log_delete_before", { timestamp }),
  countEventLogDeleteByKindPrefix: (prefix: string) =>
    invoke<number>("count_event_log_delete_by_kind_prefix", { prefix }),
  countEventLogDeleteAll: () => invoke<number>("count_event_log_delete_all"),
  deleteEventLogBefore: (timestamp: string) =>
    invoke<EventLogDeleteResult>("delete_event_log_before", { timestamp }),
  deleteEventLogByKindPrefix: (prefix: string) =>
    invoke<EventLogDeleteResult>("delete_event_log_by_kind_prefix", { prefix }),
  deleteEventLogAll: () => invoke<EventLogDeleteResult>("delete_event_log_all"),
  getActorSurfaceAssets: () =>
    invoke<ActorSurfaceAssetCatalog>("get_actor_surface_assets"),
  setActorWindowClickThrough: (passthrough: boolean) =>
    invoke<void>("set_actor_window_click_through", { passthrough }),
  setStageOverlayClickThrough: (passthrough: boolean) =>
    invoke<void>("set_stage_overlay_click_through", { passthrough }),
  getDesktopStageState: () =>
    invoke<DesktopStageState>("get_desktop_stage_state"),
  reportActorStageAnchor: (actorId: string, anchor: StageAnchor) =>
    invoke<void>("report_actor_stage_anchor", {
      actorId,
      report: { anchor },
    }),
  dismissStageBubble: (bubbleId: string) =>
    invoke<void>("dismiss_stage_bubble", { bubbleId }),
  openSettingsWindow: () => invoke<void>("open_settings_window"),
  openConversationComposer: (actorId: string) =>
    invoke<void>("open_conversation_composer", { actorId }),
  closeConversationComposer: () => invoke<void>("close_conversation_composer"),
  sendConversationText: (text: string) =>
    invoke<RuntimeCommand[]>("send_conversation_text", { text }),
  sendConversationChoice: (choiceId: string, choice: string, index: number) =>
    invoke<RuntimeCommand[]>("send_conversation_choice", {
      choiceId,
      choice,
      index,
    }),
  sendAvatarGesturePoke: (gesture: AvatarGesturePokeInput) =>
    invoke<RuntimeCommand[]>("send_avatar_gesture_poke", { gesture }),
  beginActorWindowDrag: (actorId: string) =>
    invoke<ActorWindowDragStarted>("begin_actor_window_drag", { actorId }),
  moveActorWindowDrag: (
    actorId: string,
    sessionId: string,
    dx: number,
    dy: number,
  ) => invoke<void>("move_actor_window_drag", { actorId, sessionId, dx, dy }),
  finishActorWindowDrag: (actorId: string, sessionId: string) =>
    invoke<ActorWindowDragFinished>("finish_actor_window_drag", {
      actorId,
      sessionId,
    }),
  cancelActorWindowDrag: (actorId: string, sessionId: string) =>
    invoke<void>("cancel_actor_window_drag", { actorId, sessionId }),
  notifyAvatarGestureGrab: (actorId: string) =>
    invoke<RuntimeCommand[]>("notify_avatar_gesture_grab", { actorId }),
  notifyAvatarGestureDrop: (actorId: string, movedDistance: number) =>
    invoke<RuntimeCommand[]>("notify_avatar_gesture_drop", {
      actorId,
      movedDistance,
    }),
  openWorldPackDirectory: async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    return typeof selected === "string" ? selected : null;
  },
  openWorldPackZip: async () => {
    const selected = await openDialog({
      directory: false,
      multiple: false,
      filters: [{ name: "World Pack zip", extensions: ["zip"] }],
    });
    return typeof selected === "string" ? selected : null;
  },
  openExtensionDirectory: async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    return typeof selected === "string" ? selected : null;
  },
  selectWorldPackDirectory: (path: string) =>
    invoke<WorldPackSwitchResult>("select_world_pack_directory", { path }),
  inspectWorldPackZip: (path: string) =>
    invoke<WorldPackZipInspection>("inspect_world_pack_zip", { path }),
  importWorldPackZip: (path: string) =>
    invoke<WorldPackSwitchResult>("import_world_pack_zip", { path }),
  resetWorldPackToDefault: () =>
    invoke<WorldPackSwitchResult>("reset_world_pack_to_default"),
  installExtensionDirectory: (path: string) =>
    invoke<ExtensionSettingsChangeResult>("install_extension_directory", {
      path,
    }),
  uninstallExtension: (extensionId: string) =>
    invoke<ExtensionSettingsChangeResult>("uninstall_extension", {
      extensionId,
    }),
  setExtensionEnabled: (extensionId: string, enabled: boolean) =>
    invoke<ExtensionSettingsChangeResult>("set_extension_enabled", {
      extensionId,
      enabled,
    }),
  setExtensionHookOrder: (
    hookPoint: ExtensionHookPoint,
    extensionIds: string[],
  ) =>
    invoke<ExtensionSettingsChangeResult>("set_extension_hook_order", {
      hookPoint,
      extensionIds,
    }),
  setExtensionSettingValues: (
    extensionId: string,
    values: Record<string, unknown>,
  ) =>
    invoke<ExtensionSettingsChangeResult>("set_extension_setting_values", {
      extensionId,
      values,
    }),
  setExtensionSecret: (
    extensionId: string,
    key: string,
    value: string | null,
  ) =>
    invoke<ExtensionSettingsChangeResult>("set_extension_secret", {
      extensionId,
      key,
      value,
    }),
  restartExtensionProcess: (extensionId: string) =>
    invoke<ExtensionSettingsState>("restart_extension_process", {
      extensionId,
    }),
  setAppTalkIntervalMinutes: (minutes: number) =>
    invoke<AppSettingsState>("set_app_talk_interval_minutes", { minutes }),
  setAppActorScalePercent: (percent: number) =>
    invoke<AppSettingsState>("set_app_actor_scale_percent", { percent }),
  setAppConversationSendShortcut: (shortcut: ConversationSendShortcut) =>
    invoke<AppSettingsState>("set_app_conversation_send_shortcut", {
      shortcut,
    }),
  setRuntimeSettings: (settings: RuntimeSettingsUpdate) =>
    invoke<RuntimeSettingsState>("set_runtime_settings", { settings }),
  resetSceneHistory: () => invoke<SceneHistoryState>("reset_scene_history"),
  onCommand: async (callback) => {
    const unlisten = await listen<RuntimeCommand>("yuukei-command", (event) => {
      callback(event.payload);
    });
    return unlisten;
  },
  onSnapshot: async (callback) => {
    const unlisten = await listen<ResidentSnapshot>(
      "yuukei-snapshot",
      (event) => {
        callback(event.payload);
      },
    );
    return unlisten;
  },
  onWorldPackStatus: async (callback) => {
    const unlisten = await listen<WorldPackSelectionState>(
      "yuukei-world-pack-status",
      (event) => {
        callback(event.payload);
      },
    );
    return unlisten;
  },
  onOnboardingDismissed: async (callback) => {
    const unlisten = await listen("yuukei-onboarding-dismissed", () => {
      callback();
    });
    return unlisten;
  },
  onAssetsChanged: async (callback) => {
    const unlisten = await listen<ActorSurfaceAssetCatalog>(
      "yuukei-assets-changed",
      (event) => {
        callback(event.payload);
      },
    );
    return unlisten;
  },
  onStageState: async (callback) => {
    const unlisten = await listen<DesktopStageState>(
      "yuukei-stage-state",
      (event) => {
        callback(event.payload);
      },
    );
    return unlisten;
  },
  onAppSettings: async (callback) => {
    const unlisten = await listen<AppSettingsState>(
      "yuukei-app-settings",
      (event) => {
        callback(event.payload);
      },
    );
    return unlisten;
  },
};
