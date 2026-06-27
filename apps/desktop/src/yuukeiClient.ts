import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";

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

export type YuukeiClient = {
  attachSurface(): Promise<ResidentSnapshot>;
  getSnapshot(): Promise<ResidentSnapshot>;
  getWorldPackStatus(): Promise<WorldPackSelectionState>;
  sendConversationText(text: string): Promise<RuntimeCommand[]>;
  openWorldPackDirectory(): Promise<string | null>;
  selectWorldPackDirectory(path: string): Promise<WorldPackSwitchResult>;
  resetWorldPackToDefault(): Promise<WorldPackSwitchResult>;
  onCommand(callback: (command: RuntimeCommand) => void): Promise<() => void>;
  onSnapshot(callback: (snapshot: ResidentSnapshot) => void): Promise<() => void>;
};

export const tauriYuukeiClient: YuukeiClient = {
  attachSurface: () => invoke<ResidentSnapshot>("attach_surface"),
  getSnapshot: () => invoke<ResidentSnapshot>("get_snapshot"),
  getWorldPackStatus: () =>
    invoke<WorldPackSelectionState>("get_world_pack_status"),
  sendConversationText: (text: string) =>
    invoke<RuntimeCommand[]>("send_conversation_text", { text }),
  openWorldPackDirectory: async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    return typeof selected === "string" ? selected : null;
  },
  selectWorldPackDirectory: (path: string) =>
    invoke<WorldPackSwitchResult>("select_world_pack_directory", { path }),
  resetWorldPackToDefault: () =>
    invoke<WorldPackSwitchResult>("reset_world_pack_to_default"),
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
  }
};
