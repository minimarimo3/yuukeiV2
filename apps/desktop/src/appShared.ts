import type { EventLogRecord, InstalledExtension } from "./yuukeiClient";

export function orderExtensionsForHook(
  extensions: InstalledExtension[],
  orderedIds: string[],
): InstalledExtension[] {
  const byId = new Map(
    extensions.map((extension) => [extension.extensionId, extension]),
  );
  const ordered = orderedIds
    .map((extensionId) => byId.get(extensionId))
    .filter((extension): extension is InstalledExtension => Boolean(extension));
  const seen = new Set(ordered.map((extension) => extension.extensionId));
  for (const extension of extensions) {
    if (!seen.has(extension.extensionId)) {
      ordered.push(extension);
    }
  }
  return ordered;
}

export function subscribesToBeforeCommandEmit(
  extension: InstalledExtension,
): boolean {
  return extension.hooks.some((hook) => hook.hookPoint === "beforeCommandEmit");
}

export function extensionRuntimeStatusLabel(
  extension: InstalledExtension,
): string {
  if (!extension.enabled) return "状態: 無効";
  const status = extension.runtimeStatus;
  if (!status) return "状態: 正常";
  if (status.suspended) return "状態: 休止";
  if (status.health === "degraded") {
    return `状態: 注意 (${status.failureCount})`;
  }
  return "状態: 正常";
}

export function voicevoxCreditText(
  extension: InstalledExtension,
): string | null {
  if (extension.extensionId !== "yuukei-voicevox") return null;
  return "音声合成にVOICEVOXを使用します。生成音声の利用は各キャラクターの規約に従ってください(既定の声: VOICEVOX:四国めたん / VOICEVOX:ずんだもん)";
}

export function memoryErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (/memory\.|capability|extension|provider/i.test(message)) {
    return "記憶機能が無効です";
  }
  return message;
}

export function formatMemoryTimestamp(value: string): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString("ja-JP", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatEventLogTimestamp(value: string): string {
  return formatMemoryTimestamp(value);
}

export function eventLogSummary(record: EventLogRecord): string {
  const payload = record.payload ?? {};
  const preferredKeys = [
    "text",
    "fileName",
    "choice",
    "category",
    "app",
    "windowKey",
    "reason",
    "deleted",
  ];
  for (const key of preferredKeys) {
    const value = payload[key];
    if (typeof value === "string" && value.trim()) {
      return `${key}: ${value}`;
    }
    if (typeof value === "number" || typeof value === "boolean") {
      return `${key}: ${String(value)}`;
    }
  }
  const json = JSON.stringify(payload);
  if (!json || json === "{}") {
    return "(payloadなし)";
  }
  return json.length > 120 ? `${json.slice(0, 117)}...` : json;
}

export type ExtensionPermissionRow = {
  label: string;
  value: string;
  warning?: boolean;
};

export function extensionPermissionRows(
  extension: InstalledExtension,
): ExtensionPermissionRow[] {
  const rows: ExtensionPermissionRow[] = [];
  const broadEventSubscription =
    extension.permissions.broadEventSubscription ||
    extension.eventSubscriptions.some((subscription) =>
      subscription.eventTypes.some((eventType) => eventType.trim() === "*"),
    );

  if (broadEventSubscription) {
    rows.push({
      label: "全イベント購読",
      value: "全イベントを受け取ります",
      warning: true,
    });
  }
  if (extension.permissions.eventLogRead) {
    const permission = extension.permissions.eventLogRead;
    rows.push({
      label: "event log読み出し",
      value: `${joinOrAll(permission.eventTypes)} / max ${permission.maxRecords}`,
    });
  }
  if (extension.capabilities.length > 0) {
    rows.push({
      label: "capability提供",
      value: extension.capabilities
        .map((capability) => capability.capability)
        .join(", "),
    });
  }
  if (extension.emittedEvents.length > 0) {
    rows.push({
      label: "発行イベント",
      value: extension.emittedEvents.join(", "),
    });
  }

  return rows;
}

export function joinOrAll(values: string[]): string {
  return values.length > 0 ? values.join(", ") : "*";
}
