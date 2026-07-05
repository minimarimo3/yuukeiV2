import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  ExtensionCapabilityDeclaration,
  ExtensionEventSubscription,
  ExtensionHookPoint,
  ExtensionHookSubscription,
  ExtensionPermissions,
  ExtensionRuntimeKind,
  ExtensionSettingsSchema,
  ExtensionSignalAlias,
  ResidentSnapshot,
  RuntimeCommand
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
  createdAtMs: number;
  durationMs: number;
};

export type DesktopStageState = {
  monitors: StageMonitor[];
  actors: StageActor[];
  bubbles: StageBubble[];
};

export type YuukeiClient = {
  attachSurface(): Promise<ResidentSnapshot>;
  getSnapshot(): Promise<ResidentSnapshot>;
  getWorldPackStatus(): Promise<WorldPackSelectionState>;
  getExtensionSettings(): Promise<ExtensionSettingsState>;
  getCapabilityUsage(): Promise<CapabilityUsageState>;
  getActorSurfaceAssets(): Promise<ActorSurfaceAssetCatalog>;
  setActorWindowClickThrough(passthrough: boolean): Promise<void>;
  setStageOverlayClickThrough(passthrough: boolean): Promise<void>;
  getDesktopStageState(): Promise<DesktopStageState>;
  reportActorStageAnchor(actorId: string, anchor: StageAnchor): Promise<void>;
  dismissStageBubble(bubbleId: string): Promise<void>;
  openSettingsWindow(): Promise<void>;
  sendConversationText(text: string): Promise<RuntimeCommand[]>;
  sendAvatarGesturePoke(
    gesture: AvatarGesturePokeInput
  ): Promise<RuntimeCommand[]>;
  openWorldPackDirectory(): Promise<string | null>;
  openExtensionDirectory(): Promise<string | null>;
  selectWorldPackDirectory(path: string): Promise<WorldPackSwitchResult>;
  resetWorldPackToDefault(): Promise<WorldPackSwitchResult>;
  installExtensionDirectory(
    path: string
  ): Promise<ExtensionSettingsChangeResult>;
  uninstallExtension(extensionId: string): Promise<ExtensionSettingsChangeResult>;
  setExtensionEnabled(
    extensionId: string,
    enabled: boolean
  ): Promise<ExtensionSettingsChangeResult>;
  setExtensionHookOrder(
    hookPoint: ExtensionHookPoint,
    extensionIds: string[]
  ): Promise<ExtensionSettingsChangeResult>;
  setExtensionSettingValues(
    extensionId: string,
    values: Record<string, unknown>
  ): Promise<ExtensionSettingsChangeResult>;
  setExtensionSecret(
    extensionId: string,
    key: string,
    value: string | null
  ): Promise<ExtensionSettingsChangeResult>;
  onCommand(callback: (command: RuntimeCommand) => void): Promise<() => void>;
  onSnapshot(callback: (snapshot: ResidentSnapshot) => void): Promise<() => void>;
  onWorldPackStatus(
    callback: (status: WorldPackSelectionState) => void
  ): Promise<() => void>;
  onAssetsChanged(
    callback: (catalog: ActorSurfaceAssetCatalog) => void
  ): Promise<() => void>;
  onStageState(callback: (state: DesktopStageState) => void): Promise<() => void>;
};

export const tauriYuukeiClient: YuukeiClient = {
  attachSurface: () => invoke<ResidentSnapshot>("attach_surface"),
  getSnapshot: () => invoke<ResidentSnapshot>("get_snapshot"),
  getWorldPackStatus: () =>
    invoke<WorldPackSelectionState>("get_world_pack_status"),
  getExtensionSettings: () =>
    invoke<ExtensionSettingsState>("get_extension_settings"),
  getCapabilityUsage: () => invoke<CapabilityUsageState>("get_capability_usage"),
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
      report: { anchor }
    }),
  dismissStageBubble: (bubbleId: string) =>
    invoke<void>("dismiss_stage_bubble", { bubbleId }),
  openSettingsWindow: () => invoke<void>("open_settings_window"),
  sendConversationText: (text: string) =>
    invoke<RuntimeCommand[]>("send_conversation_text", { text }),
  sendAvatarGesturePoke: (gesture: AvatarGesturePokeInput) =>
    invoke<RuntimeCommand[]>("send_avatar_gesture_poke", { gesture }),
  openWorldPackDirectory: async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    return typeof selected === "string" ? selected : null;
  },
  openExtensionDirectory: async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    return typeof selected === "string" ? selected : null;
  },
  selectWorldPackDirectory: (path: string) =>
    invoke<WorldPackSwitchResult>("select_world_pack_directory", { path }),
  resetWorldPackToDefault: () =>
    invoke<WorldPackSwitchResult>("reset_world_pack_to_default"),
  installExtensionDirectory: (path: string) =>
    invoke<ExtensionSettingsChangeResult>("install_extension_directory", {
      path
    }),
  uninstallExtension: (extensionId: string) =>
    invoke<ExtensionSettingsChangeResult>("uninstall_extension", {
      extensionId
    }),
  setExtensionEnabled: (extensionId: string, enabled: boolean) =>
    invoke<ExtensionSettingsChangeResult>("set_extension_enabled", {
      extensionId,
      enabled
    }),
  setExtensionHookOrder: (
    hookPoint: ExtensionHookPoint,
    extensionIds: string[]
  ) =>
    invoke<ExtensionSettingsChangeResult>("set_extension_hook_order", {
      hookPoint,
      extensionIds
    }),
  setExtensionSettingValues: (
    extensionId: string,
    values: Record<string, unknown>
  ) =>
    invoke<ExtensionSettingsChangeResult>("set_extension_setting_values", {
      extensionId,
      values
    }),
  setExtensionSecret: (
    extensionId: string,
    key: string,
    value: string | null
  ) =>
    invoke<ExtensionSettingsChangeResult>("set_extension_secret", {
      extensionId,
      key,
      value
    }),
  onCommand: async (callback) => {
    const unlisten = await listen<RuntimeCommand>("yuukei-command", (event) => {
      callback(event.payload);
    });
    return unlisten;
  },
  onSnapshot: async (callback) => {
    const unlisten = await listen<ResidentSnapshot>("yuukei-snapshot", (event) => {
      callback(event.payload);
    });
    return unlisten;
  },
  onWorldPackStatus: async (callback) => {
    const unlisten = await listen<WorldPackSelectionState>(
      "yuukei-world-pack-status",
      (event) => {
        callback(event.payload);
      }
    );
    return unlisten;
  },
  onAssetsChanged: async (callback) => {
    const unlisten = await listen<ActorSurfaceAssetCatalog>(
      "yuukei-assets-changed",
      (event) => {
        callback(event.payload);
      }
    );
    return unlisten;
  },
  onStageState: async (callback) => {
    const unlisten = await listen<DesktopStageState>(
      "yuukei-stage-state",
      (event) => {
        callback(event.payload);
      }
    );
    return unlisten;
  }
};
