import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  ExtensionHookPoint,
  ExtensionHookSubscription,
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

export type WorldPackSelectionState = {
  configuredInstallId: string;
  runningInstallId: string;
  activeInstall: WorldPackInstall;
  installs: WorldPackInstall[];
  fallbackActive: boolean;
  lastLoadError?: string;
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
  hooks: ExtensionHookSubscription[];
  installedPath: string;
  manifestPath: string;
  installedAt: string;
  updatedAt: string;
  lastLoadError?: string;
};

export type ExtensionSettingsState = {
  installed: InstalledExtension[];
  hookOrder: Partial<Record<ExtensionHookPoint, string[]>>;
  settingsPath: string;
  extensionRoot: string;
  trustedCodeNotice: string;
};

export type ExtensionSettingsChangeResult = {
  state: ExtensionSettingsState;
  snapshot: ResidentSnapshot;
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

export type YuukeiClient = {
  attachSurface(): Promise<ResidentSnapshot>;
  getSnapshot(): Promise<ResidentSnapshot>;
  getWorldPackStatus(): Promise<WorldPackSelectionState>;
  getExtensionSettings(): Promise<ExtensionSettingsState>;
  getActorSurfaceAssets(): Promise<ActorSurfaceAssetCatalog>;
  setActorWindowClickThrough(passthrough: boolean): Promise<void>;
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
  onCommand(callback: (command: RuntimeCommand) => void): Promise<() => void>;
  onSnapshot(callback: (snapshot: ResidentSnapshot) => void): Promise<() => void>;
  onAssetsChanged(
    callback: (catalog: ActorSurfaceAssetCatalog) => void
  ): Promise<() => void>;
};

export const tauriYuukeiClient: YuukeiClient = {
  attachSurface: () => invoke<ResidentSnapshot>("attach_surface"),
  getSnapshot: () => invoke<ResidentSnapshot>("get_snapshot"),
  getWorldPackStatus: () =>
    invoke<WorldPackSelectionState>("get_world_pack_status"),
  getExtensionSettings: () =>
    invoke<ExtensionSettingsState>("get_extension_settings"),
  getActorSurfaceAssets: () =>
    invoke<ActorSurfaceAssetCatalog>("get_actor_surface_assets"),
  setActorWindowClickThrough: (passthrough: boolean) =>
    invoke<void>("set_actor_window_click_through", { passthrough }),
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
  onAssetsChanged: async (callback) => {
    const unlisten = await listen<ActorSurfaceAssetCatalog>(
      "yuukei-assets-changed",
      (event) => {
        callback(event.payload);
      }
    );
    return unlisten;
  }
};
