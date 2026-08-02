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
  if (!extension.enabled) return "オフ";
  const status = extension.runtimeStatus;
  if (!status) return "利用できます";
  if (status.suspended) return "問題が続いたため停止中です";
  if (status.health === "degraded") {
    return "一時的な問題が起きています";
  }
  return "利用できます";
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
  if (
    /(?:\.json|protocol|failed|invalid|missing|unknown|timeout|[\\/][^ ]+)/i.test(
      message,
    )
  ) {
    return "住人の記憶を読み込めませんでした。もう一度お試しください。";
  }
  return message || "住人の記憶を読み込めませんでした。";
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
  const text = textValue(payload.text);
  if (text) {
    if (record.kind === "dialogue.say") return `住人「${text}」`;
    if (record.kind === "conversation.text") return `あなた「${text}」`;
    return text;
  }
  const fileName = textValue(payload.fileName);
  if (fileName) return `${fileName}について住人が気づきました`;
  const app = textValue(payload.app);
  if (app) return `${app}を使ったことに住人が気づきました`;
  const choice = textValue(payload.choice);
  if (choice) return `「${choice}」を選びました`;
  if (typeof payload.deleted === "number") {
    return `${payload.deleted}件の記録を削除しました`;
  }
  return "住人の生活に関するできごとです";
}

export function eventKindLabel(kind: string): string {
  const exact: Record<string, string> = {
    "desktop.download.completed": "ダウンロードに気づいた",
    "conversation.text": "あなたが話しかけた",
    "dialogue.say": "住人が話した",
    "app.settings.opened": "設定を開いた",
    "memory.updated": "住人の記憶が変わった",
    "memory.forgotten": "住人が記憶を忘れた",
  };
  if (exact[kind]) return exact[kind];
  if (kind.startsWith("desktop.")) return "パソコン上の変化に気づいた";
  if (kind.startsWith("conversation.") || kind.startsWith("dialogue.")) {
    return "住人と会話した";
  }
  if (kind.startsWith("memory.")) return "住人の記憶が変わった";
  if (kind.startsWith("presence.")) return "住人が過ごした";
  if (kind.startsWith("scene.")) return "物語が進んだ";
  return "住人の生活に変化があった";
}

export function eventPrivacyLabel(category: string | null | undefined): string {
  return category === "desktop-observation"
    ? "パソコン上の変化から記録"
    : "住人とのやりとりから記録";
}

export type ExtensionPermissionRow = {
  label: string;
  value: string;
  warning?: boolean;
};

export function extensionPermissionRows(
  extension: Pick<
    InstalledExtension,
    | "permissions"
    | "hooks"
    | "eventSubscriptions"
    | "capabilities"
    | "emittedEvents"
  >,
): ExtensionPermissionRow[] {
  const rows: ExtensionPermissionRow[] = [];
  const broadEventSubscription =
    extension.permissions.broadEventSubscription ||
    extension.eventSubscriptions.some((subscription) =>
      subscription.eventTypes.some((eventType) => eventType.trim() === "*"),
    );

  if (broadEventSubscription) {
    rows.push({
      label: "すべてのできごとを利用",
      value: "住人の生活で起きたすべてのできごとを受け取ります",
      warning: true,
    });
  }
  if (extension.eventSubscriptions.length > 0) {
    const eventTypeCount = new Set(
      extension.eventSubscriptions.flatMap(
        (subscription) => subscription.eventTypes,
      ),
    ).size;
    rows.push({
      label: "住人のできごとを利用",
      value: broadEventSubscription
        ? "すべてのできごと"
        : `${eventTypeCount}種類のできごと`,
      warning: broadEventSubscription,
    });
  }
  if (extension.hooks.length > 0) {
    rows.push({
      label: "住人の動作や会話を調整",
      value: "住人が実行する直前に内容を調整できます",
    });
  }
  if (extension.permissions.eventLogRead) {
    const permission = extension.permissions.eventLogRead;
    rows.push({
      label: "生活の記録を読む",
      value: [
        `目的: ${permission.purpose}`,
        `一度に最大${permission.maxRecords}件`,
        permission.allowPayloads ? "記録の内容を含む" : "記録の種類と日時のみ",
      ].join("・"),
    });
  }
  if (extension.capabilities.length > 0) {
    rows.push({
      label: "追加される機能",
      value: extension.capabilities
        .map((capability) => capabilityLabel(capability.capability))
        .join(", "),
    });
  }
  if (extension.emittedEvents.length > 0) {
    rows.push({
      label: "Yuukeiへ変化を知らせる",
      value: `${extension.emittedEvents.length}種類の変化を住人の生活に反映します`,
    });
  }

  return rows;
}

export function capabilityLabel(capability: string): string {
  if (capability.startsWith("dialogue.")) return "会話";
  if (capability.startsWith("memory.")) return "記憶";
  if (capability.startsWith("voice.") || capability.includes("speech")) {
    return "音声";
  }
  if (capability.startsWith("mood.")) return "気分";
  return "住人の追加機能";
}

function textValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function joinOrAll(values: string[]): string {
  return values.length > 0 ? values.join(", ") : "*";
}
