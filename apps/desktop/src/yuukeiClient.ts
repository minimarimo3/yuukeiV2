import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";

export type YuukeiClient = {
  attachSurface(): Promise<ResidentSnapshot>;
  getSnapshot(): Promise<ResidentSnapshot>;
  sendConversationText(text: string): Promise<RuntimeCommand[]>;
  onCommand(callback: (command: RuntimeCommand) => void): Promise<() => void>;
  onSnapshot(callback: (snapshot: ResidentSnapshot) => void): Promise<() => void>;
};

export const tauriYuukeiClient: YuukeiClient = {
  attachSurface: () => invoke<ResidentSnapshot>("attach_surface"),
  getSnapshot: () => invoke<ResidentSnapshot>("get_snapshot"),
  sendConversationText: (text: string) =>
    invoke<RuntimeCommand[]>("send_conversation_text", { text }),
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
