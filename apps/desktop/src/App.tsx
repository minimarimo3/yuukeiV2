import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";
import {
  extensionPermissionRows,
  extensionRuntimeStatusLabel,
  memoryErrorMessage,
  orderExtensionsForHook,
  subscribesToBeforeCommandEmit,
  voicevoxCreditText,
} from "./appShared";
import { DaihonDiagnosticsPanel } from "./DaihonDiagnosticsPanel";
import { EventLogSettingsPanel } from "./EventLogSettingsPanel";
import { ExtensionConsentDialog } from "./ExtensionConsentDialog";
import {
  ExtensionSettingsForm,
  ExtensionUsageSection,
} from "./ExtensionSettingsPanel";
import { MemorySettingsPanel } from "./MemorySettingsPanel";
import { OnboardingFlow } from "./OnboardingFlow";
import { OwnedOverlay } from "./OwnedOverlay";
import {
  type AppSettingsState,
  type CapabilityUsageState,
  type EventLogPage,
  type EventLogPrivacyCategoryFilter,
  type ExtensionInstallInspection,
  type ExtensionSettingsChangeResult,
  type ExtensionSettingsState,
  type MemoryEntryKind,
  type MemoryForgetEntry,
  type ObservationSettingsState,
  type ObservationSettingsUpdate,
  type OnboardingState,
  type ResidentMemoryState,
  type RuntimeSettingsState,
  type SceneHistoryState,
  type StageOwnedOverlay,
  tauriYuukeiClient,
  type WorldPackSelectionState,
  type YuukeiClient,
} from "./yuukeiClient";

type AppProps = {
  client?: YuukeiClient;
};

type SettingsCategoryId =
  | "app"
  | "keys"
  | "worldPack"
  | "sceneHistory"
  | "eventLog"
  | "memories"
  | "extensions";
const MEMORY_PAGE_SIZE = 50;
const EVENT_LOG_PAGE_SIZE = 50;

type SettingsCategory = {
  id: SettingsCategoryId;
  label: string;
  ariaLabel: string;
  panelId: string;
  panelClassName?: string;
  content: ReactNode;
};

type PendingExtensionInstall = {
  path: string;
  inspection: ExtensionInstallInspection;
};

type TalkFrequencyPreset = "quiet" | "normal" | "chatty";

const TALK_FREQUENCY_PRESETS: Record<
  TalkFrequencyPreset,
  { low: number; high: number }
> = {
  quiet: { low: 45, high: 90 },
  normal: { low: 30, high: 80 },
  chatty: { low: 15, high: 65 },
};

export function App({ client = tauriYuukeiClient }: AppProps) {
  const [status, setStatus] = useState("connecting");
  const [activeSettingsCategoryId, setActiveSettingsCategoryId] =
    useState<SettingsCategoryId>("worldPack");
  const [worldPackStatus, setWorldPackStatus] =
    useState<WorldPackSelectionState | null>(null);
  const [appSettings, setAppSettings] = useState<AppSettingsState | null>(null);
  const [runtimeSettings, setRuntimeSettings] =
    useState<RuntimeSettingsState | null>(null);
  const [sceneHistory, setSceneHistory] = useState<SceneHistoryState | null>(
    null,
  );
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [autostartCanEnable, setAutostartCanEnable] = useState(true);
  const [observationSettings, setObservationSettings] =
    useState<ObservationSettingsState | null>(null);
  const [onboardingState, setOnboardingState] =
    useState<OnboardingState | null>(null);
  const [onboardingDismissed, setOnboardingDismissed] = useState(false);
  const [onboardingStep, setOnboardingStep] = useState(0);
  const [extensionState, setExtensionState] =
    useState<ExtensionSettingsState | null>(null);
  const [capabilityUsage, setCapabilityUsage] =
    useState<CapabilityUsageState | null>(null);
  const [worldPackError, setWorldPackError] = useState<string | null>(null);
  const [appSettingsError, setAppSettingsError] = useState<string | null>(null);
  const [observationSettingsError, setObservationSettingsError] = useState<
    string | null
  >(null);
  const [extensionError, setExtensionError] = useState<string | null>(null);
  const [memoryState, setMemoryState] = useState<ResidentMemoryState | null>(
    null,
  );
  const [memoryError, setMemoryError] = useState<string | null>(null);
  const [loadingMemories, setLoadingMemories] = useState(false);
  const [eventLogPage, setEventLogPage] = useState<EventLogPage | null>(null);
  const [eventLogError, setEventLogError] = useState<string | null>(null);
  const [loadingEventLog, setLoadingEventLog] = useState(false);
  const [eventLogKindPrefix, setEventLogKindPrefix] = useState("");
  const [eventLogPrivacyFilter, setEventLogPrivacyFilter] =
    useState<EventLogPrivacyCategoryFilter>("all");
  const [eventLogDeleteBefore, setEventLogDeleteBefore] = useState("");
  const [eventLogDeletePrefix, setEventLogDeletePrefix] = useState("");
  const [editingFactId, setEditingFactId] = useState<string | null>(null);
  const [editingFactText, setEditingFactText] = useState("");
  const [switchingPack, setSwitchingPack] = useState(false);
  const [changingObservationSettings, setChangingObservationSettings] =
    useState(false);
  const [changingExtensions, setChangingExtensions] = useState(false);
  const [pendingExtensionInstall, setPendingExtensionInstall] =
    useState<PendingExtensionInstall | null>(null);
  const [showAllDaihonDiagnostics, setShowAllDaihonDiagnostics] =
    useState(false);
  const [ownedOverlay, setOwnedOverlay] = useState<StageOwnedOverlay | null>(
    null,
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: loadMemories/loadEventLogは毎レンダー再生成されるため依存に含めない(client変更時のみ再接続する意図)
  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function connect() {
      try {
        unlisteners.push(
          await client.onAssetsChanged(() => {
            void refreshSettings();
            void loadMemories();
            void loadEventLog();
          }),
        );
        unlisteners.push(
          await client.onWorldPackStatus((nextWorldPackStatus) => {
            if (!disposed) {
              setWorldPackStatus(nextWorldPackStatus);
            }
            void loadMemories();
            void loadEventLog();
          }),
        );
        unlisteners.push(
          await client.onOnboardingDismissed(() => {
            if (!disposed) {
              setOnboardingDismissed(true);
            }
          }),
        );
        unlisteners.push(
          await client.onStageState((stage) => {
            if (!disposed) {
              setOwnedOverlay(latestOwnedOverlay(stage.ownedOverlays ?? []));
            }
          }),
        );
        await refreshSettings();
        await loadMemories();
        await loadEventLog();
        if (!disposed) {
          setStatus("ready");
        }
      } catch (error) {
        if (!disposed) {
          setStatus(error instanceof Error ? error.message : String(error));
        }
      }
    }

    async function refreshSettings() {
      const [
        nextWorldPackStatus,
        nextAppSettings,
        nextRuntimeSettings,
        nextSceneHistory,
        nextAutostartEnabled,
        nextAutostartCanEnable,
        nextObservationSettings,
        nextOnboardingState,
        nextExtensionState,
        nextCapabilityUsage,
        nextStage,
      ] = await Promise.all([
        client.getWorldPackStatus(),
        client.getAppSettings(),
        client.getRuntimeSettings(),
        client.getSceneHistory(),
        client.getAutostartEnabled(),
        client.getAutostartCanEnable?.() ?? Promise.resolve(true),
        client.getObservationSettings(),
        client.getOnboardingState(),
        client.getExtensionSettings(),
        client.getCapabilityUsage(),
        client.getDesktopStageState(),
      ]);
      if (!disposed) {
        setWorldPackStatus(nextWorldPackStatus);
        setAppSettings(nextAppSettings);
        setRuntimeSettings(nextRuntimeSettings);
        setSceneHistory(nextSceneHistory);
        setAutostartEnabled(nextAutostartEnabled);
        setAutostartCanEnable(nextAutostartCanEnable);
        setObservationSettings(nextObservationSettings);
        setOnboardingState(nextOnboardingState);
        setExtensionState(nextExtensionState);
        setCapabilityUsage(nextCapabilityUsage);
        setOwnedOverlay(latestOwnedOverlay(nextStage.ownedOverlays ?? []));
      }
    }

    void connect();
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, [client]);

  const orderedBeforeCommandEmitExtensions = useMemo(() => {
    return orderExtensionsForHook(
      (extensionState?.installed ?? []).filter(subscribesToBeforeCommandEmit),
      extensionState?.hookOrder.beforeCommandEmit ?? [],
    );
  }, [extensionState]);
  const orderedExtensions = useMemo(() => {
    return orderExtensionsForHook(
      extensionState?.installed ?? [],
      extensionState?.hookOrder.beforeCommandEmit ?? [],
    );
  }, [extensionState]);

  async function loadMemories(offset = 0, append = false) {
    setLoadingMemories(true);
    try {
      const next = await client.listResidentMemories(MEMORY_PAGE_SIZE, offset);
      setMemoryError(null);
      setMemoryState((current) =>
        append && current
          ? {
              ...next,
              facts: next.facts,
              episodes: [...current.episodes, ...next.episodes],
            }
          : next,
      );
    } catch (error) {
      setMemoryError(memoryErrorMessage(error));
      if (!append) {
        setMemoryState(null);
      }
    } finally {
      setLoadingMemories(false);
    }
  }

  async function loadEventLog(beforeSequence?: number, append = false) {
    setLoadingEventLog(true);
    try {
      const next = await client.readEventLogPage(
        eventLogKindPrefix.trim() || undefined,
        eventLogPrivacyFilter,
        beforeSequence,
        EVENT_LOG_PAGE_SIZE,
      );
      setEventLogError(null);
      setEventLogPage((current) =>
        append && current
          ? {
              ...next,
              records: [...current.records, ...next.records],
            }
          : next,
      );
    } catch (error) {
      setEventLogError(
        userFacingError(
          error,
          "生活の記録を操作できませんでした。もう一度お試しください。",
        ),
      );
      if (!append) {
        setEventLogPage(null);
      }
    } finally {
      setLoadingEventLog(false);
    }
  }

  async function chooseWorldPack() {
    setWorldPackError(null);
    setSwitchingPack(true);
    try {
      const path = await client.openWorldPackDirectory();
      if (!path) return;
      const result = await client.selectWorldPackDirectory(path);
      setWorldPackStatus(result.status);
      setStatus("ready");
      void loadMemories();
    } catch (error) {
      setWorldPackError(
        userFacingError(
          error,
          "選んだ住人と世界を読み込めませんでした。必要なファイルが揃っているか確認してください。",
        ),
      );
      try {
        setWorldPackStatus(await client.getWorldPackStatus());
      } catch {
        // The Tauri event path normally refreshes this; the direct refresh is best effort.
      }
    } finally {
      setSwitchingPack(false);
    }
  }

  async function importWorldPackZip() {
    setWorldPackError(null);
    setSwitchingPack(true);
    try {
      const path = await client.openWorldPackZip();
      if (!path) return;
      const inspection = await client.inspectWorldPackZip(path);
      const licenseText =
        inspection.licenseText?.trim() ||
        "ライセンス表記が見つかりませんでした。配布元の条件を確認してください。";
      const replaceNotice = inspection.replacesExisting
        ? "\n\n同じ住人と世界がすでに追加されています。続けると新しい内容に置き換わります。"
        : "";
      const confirmed = window.confirm(
        `配布条件の確認\n\n${inspection.displayName}\n${inspection.licenseSource ?? "配布条件の記載なし"}\n\n${licenseText}${replaceNotice}\n\nこの住人と世界を追加しますか？`,
      );
      if (!confirmed) return;
      const result = await client.importWorldPackZip(path);
      setWorldPackStatus(result.status);
      setStatus("ready");
      void loadMemories();
    } catch (error) {
      setWorldPackError(
        userFacingError(
          error,
          "配布ファイルを読み込めませんでした。ファイルが壊れていないか確認してください。",
        ),
      );
      try {
        setWorldPackStatus(await client.getWorldPackStatus());
      } catch {
        // The Tauri event path normally refreshes this; the direct refresh is best effort.
      }
    } finally {
      setSwitchingPack(false);
    }
  }

  async function resetWorldPack() {
    setWorldPackError(null);
    setSwitchingPack(true);
    try {
      const result = await client.resetWorldPackToDefault();
      setWorldPackStatus(result.status);
      setStatus("ready");
      void loadMemories();
    } catch (error) {
      setWorldPackError(
        userFacingError(error, "標準の内容に戻せませんでした。"),
      );
    } finally {
      setSwitchingPack(false);
    }
  }

  function applyExtensionResult(result: ExtensionSettingsChangeResult) {
    setExtensionState(result.state);
    setStatus("ready");
    void loadMemories();
  }

  async function chooseExtension() {
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      const path = await client.openExtensionDirectory();
      if (!path) return;
      const inspection = await client.inspectExtensionDirectory(path);
      setPendingExtensionInstall({ path, inspection });
    } catch (error) {
      setExtensionError(
        userFacingError(
          error,
          "追加機能を変更できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setChangingExtensions(false);
    }
  }

  async function approveExtensionInstall() {
    if (!pendingExtensionInstall) return;
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      applyExtensionResult(
        await client.installExtensionDirectory(
          pendingExtensionInstall.path,
          pendingExtensionInstall.inspection.manifestDigest,
        ),
      );
      setPendingExtensionInstall(null);
    } catch (error) {
      setExtensionError(
        userFacingError(
          error,
          "追加機能を変更できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setChangingExtensions(false);
    }
  }

  function cancelExtensionInstall() {
    if (changingExtensions) return;
    setPendingExtensionInstall(null);
    setExtensionError(null);
  }

  async function toggleExtension(extensionId: string, enabled: boolean) {
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      applyExtensionResult(
        await client.setExtensionEnabled(extensionId, enabled),
      );
    } catch (error) {
      setExtensionError(
        userFacingError(
          error,
          "追加機能を変更できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setChangingExtensions(false);
    }
  }

  async function uninstallExtension(extensionId: string) {
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      applyExtensionResult(await client.uninstallExtension(extensionId));
    } catch (error) {
      setExtensionError(
        userFacingError(
          error,
          "追加機能を変更できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setChangingExtensions(false);
    }
  }

  async function restartExtensionProcess(extensionId: string) {
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      setExtensionState(await client.restartExtensionProcess(extensionId));
    } catch (error) {
      setExtensionError(
        userFacingError(
          error,
          "追加機能を変更できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setChangingExtensions(false);
    }
  }

  async function moveExtension(extensionId: string, direction: -1 | 1) {
    if (!extensionState) return;
    const currentOrder = orderedBeforeCommandEmitExtensions.map(
      (extension) => extension.extensionId,
    );
    const index = currentOrder.indexOf(extensionId);
    const nextIndex = index + direction;
    if (index < 0 || nextIndex < 0 || nextIndex >= currentOrder.length) return;
    const nextOrder = [...currentOrder];
    [nextOrder[index], nextOrder[nextIndex]] = [
      nextOrder[nextIndex],
      nextOrder[index],
    ];
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      applyExtensionResult(
        await client.setExtensionHookOrder("beforeCommandEmit", nextOrder),
      );
    } catch (error) {
      setExtensionError(
        userFacingError(
          error,
          "追加機能を変更できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setChangingExtensions(false);
    }
  }

  async function refreshCapabilityUsage() {
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      setCapabilityUsage(await client.getCapabilityUsage());
    } catch (error) {
      setExtensionError(
        userFacingError(
          error,
          "追加機能を変更できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setChangingExtensions(false);
    }
  }

  async function saveTalkInterval(minutes: number) {
    const normalized = Math.max(0, Math.trunc(minutes || 0));
    setAppSettingsError(null);
    try {
      setAppSettings(await client.setAppTalkIntervalMinutes(normalized));
    } catch (error) {
      setAppSettingsError(
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  async function saveActorScalePercent(percent: number) {
    const normalized = Math.trunc(percent || 100);
    setAppSettingsError(null);
    try {
      setAppSettings(await client.setAppActorScalePercent(normalized));
    } catch (error) {
      setAppSettingsError(
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  async function toggleAutostart(enabled: boolean) {
    setAppSettingsError(null);
    try {
      setAutostartEnabled(await client.setAutostartEnabled(enabled));
    } catch (error) {
      setAppSettingsError(
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  async function saveRuntimeSettings(
    key:
      | "llmTimeoutMs"
      | "recentContextCount"
      | "talkDesireLow"
      | "talkDesireHigh",
    value: number,
  ) {
    const current = runtimeSettings ?? defaultRuntimeSettings();
    const next = {
      llmTimeoutMs:
        key === "llmTimeoutMs"
          ? Math.max(0, Math.trunc(value || 0))
          : current.llmTimeoutMs,
      recentContextCount:
        key === "recentContextCount"
          ? Math.max(0, Math.trunc(value || 0))
          : current.recentContextCount,
      talkDesireLow:
        key === "talkDesireLow"
          ? Math.max(0, Math.trunc(value || 0))
          : current.talkDesireLow,
      talkDesireHigh:
        key === "talkDesireHigh"
          ? Math.max(0, Math.trunc(value || 0))
          : current.talkDesireHigh,
    };
    setAppSettingsError(null);
    try {
      setRuntimeSettings(await client.setRuntimeSettings(next));
    } catch (error) {
      setAppSettingsError(
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  async function saveTalkFrequency(preset: TalkFrequencyPreset) {
    const current = runtimeSettings ?? defaultRuntimeSettings();
    const thresholds = TALK_FREQUENCY_PRESETS[preset];
    setAppSettingsError(null);
    try {
      setRuntimeSettings(
        await client.setRuntimeSettings({
          llmTimeoutMs: current.llmTimeoutMs,
          recentContextCount: current.recentContextCount,
          talkDesireLow: thresholds.low,
          talkDesireHigh: thresholds.high,
        }),
      );
    } catch (error) {
      setAppSettingsError(
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  async function resetSceneHistory() {
    if (
      !window.confirm(
        "この住人の物語の進み具合を最初に戻します。この操作は取り消せません。続けますか？",
      )
    ) {
      return;
    }
    setWorldPackError(null);
    try {
      setSceneHistory(await client.resetSceneHistory());
    } catch (error) {
      setWorldPackError(
        userFacingError(error, "物語の進み具合を元に戻せませんでした。"),
      );
    }
  }

  async function toggleObservationSetting(
    key: keyof ObservationSettingsUpdate,
    enabled: boolean,
  ) {
    const current =
      observationSettings ??
      ({
        windows: false,
        folders: false,
        downloads: false,
      } satisfies ObservationSettingsUpdate);
    const next: ObservationSettingsUpdate = {
      windows: current.windows,
      folders: current.folders,
      downloads: current.downloads,
      [key]: enabled,
    };
    setObservationSettingsError(null);
    setChangingObservationSettings(true);
    try {
      setObservationSettings(await client.setObservationSettings(next));
    } catch (error) {
      setObservationSettingsError(
        userFacingError(
          error,
          "プライバシー設定を保存できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setChangingObservationSettings(false);
    }
  }

  async function dismissOnboarding() {
    setOnboardingDismissed(true);
    setOnboardingState(await client.dismissOnboarding());
  }

  async function completeOnboarding() {
    setOnboardingState(await client.completeOnboarding());
    setOnboardingStep(0);
    setActiveSettingsCategoryId("worldPack");
  }

  function beginFactEdit(id: string, text: string) {
    setEditingFactId(id);
    setEditingFactText(text);
  }

  async function saveFactEdit(id: string) {
    const text = editingFactText.trim();
    if (!text) {
      setMemoryError("空の記憶は保存できません。");
      return;
    }
    setMemoryError(null);
    setLoadingMemories(true);
    try {
      const result = await client.updateResidentMemory("fact", id, text);
      if (!result.updated) {
        throw new Error("記憶を保存できませんでした。");
      }
      setEditingFactId(null);
      setEditingFactText("");
      await loadMemories();
    } catch (error) {
      setMemoryError(memoryErrorMessage(error));
    } finally {
      setLoadingMemories(false);
    }
  }

  async function forgetMemoryEntry(kind: MemoryEntryKind, id: string) {
    const entry: MemoryForgetEntry = { kind, id };
    setMemoryError(null);
    setLoadingMemories(true);
    try {
      await client.forgetResidentMemories([entry], false);
      await loadMemories();
    } catch (error) {
      setMemoryError(memoryErrorMessage(error));
    } finally {
      setLoadingMemories(false);
    }
  }

  async function forgetAllMemories() {
    const confirmed = window.confirm(
      "すべての記憶を忘れます。この操作は取り消せません。続けますか？",
    );
    if (!confirmed) return;
    setMemoryError(null);
    setLoadingMemories(true);
    try {
      await client.forgetResidentMemories(undefined, true);
      setEditingFactId(null);
      setEditingFactText("");
      await loadMemories();
    } catch (error) {
      setMemoryError(memoryErrorMessage(error));
    } finally {
      setLoadingMemories(false);
    }
  }

  async function loadMoreEpisodes() {
    await loadMemories(memoryState?.episodes.length ?? 0, true);
  }

  async function deleteEventLogBefore() {
    if (!eventLogDeleteBefore) {
      setEventLogError("日時を指定してください。");
      return;
    }
    const timestamp = new Date(eventLogDeleteBefore).toISOString();
    setLoadingEventLog(true);
    try {
      const count = await client.countEventLogDeleteBefore(timestamp);
      if (
        !confirmEventLogDeletion(
          `${new Date(timestamp).toLocaleString("ja-JP")}より前の記録`,
          count,
        )
      ) {
        return;
      }
      await client.deleteEventLogBefore(timestamp);
      await loadEventLog();
    } catch (error) {
      setEventLogError(
        userFacingError(
          error,
          "生活の記録を操作できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setLoadingEventLog(false);
    }
  }

  async function deleteEventLogByKindPrefix() {
    const prefix = eventLogDeletePrefix.trim();
    if (!prefix) {
      setEventLogError("削除するできごとを選んでください。");
      return;
    }
    setLoadingEventLog(true);
    try {
      const count = await client.countEventLogDeleteByKindPrefix(prefix);
      if (!confirmEventLogDeletion(eventLogGroupLabel(prefix), count)) {
        return;
      }
      await client.deleteEventLogByKindPrefix(prefix);
      await loadEventLog();
    } catch (error) {
      setEventLogError(
        userFacingError(
          error,
          "生活の記録を操作できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setLoadingEventLog(false);
    }
  }

  async function deleteAllEventLog() {
    setLoadingEventLog(true);
    try {
      const count = await client.countEventLogDeleteAll();
      if (!confirmEventLogDeletion("すべての生活の記録", count)) return;
      await client.deleteEventLogAll();
      await loadEventLog();
    } catch (error) {
      setEventLogError(
        userFacingError(
          error,
          "生活の記録を操作できませんでした。もう一度お試しください。",
        ),
      );
    } finally {
      setLoadingEventLog(false);
    }
  }

  function confirmEventLogDeletion(label: string, count: number) {
    return window.confirm(
      `${label}を削除します。\n\n削除予定: ${count}件\nこの操作は取り消せません。\nここで履歴を消しても、住人が別に覚えている内容は残る場合があります。『住人の記憶』から個別に忘れさせられます。\n\n続けますか？`,
    );
  }

  const settingsCategories: SettingsCategory[] = [
    {
      id: "app",
      label: "基本設定",
      ariaLabel: "基本設定",
      panelId: "settings-app-panel",
      content: (
        <>
          <div className="settings-copy app-settings-copy">
            <h2>基本設定</h2>
            <p className="settings-title">住人のようす</p>
            <p className="settings-note">
              デスクトップでの見え方や、住人から話しかける頻度を変えられます。
            </p>
            <label
              className="app-setting-field"
              htmlFor="talk-interval-minutes"
            >
              <span>
                <strong>おしゃべりの間隔</strong>
                <small>
                  何分おきに話しかけるかの目安です。0にすると、住人からは話しかけません。
                </small>
              </span>
              <span className="number-setting-control">
                <input
                  id="talk-interval-minutes"
                  type="number"
                  min={0}
                  step={1}
                  value={appSettings?.talkIntervalMinutes ?? 5}
                  onChange={(event) => {
                    const value = Number(event.currentTarget.value);
                    void saveTalkInterval(Number.isFinite(value) ? value : 0);
                  }}
                />
                <span aria-hidden="true">分</span>
              </span>
            </label>
            <label className="app-setting-field" htmlFor="actor-scale-percent">
              <span>
                <strong>住人の大きさ</strong>
                <small>
                  デスクトップに表示される住人の大きさを変えられます。
                </small>
              </span>
              <span className="range-setting-control">
                <input
                  id="actor-scale-percent"
                  type="range"
                  min={50}
                  max={200}
                  step={10}
                  value={appSettings?.actorScalePercent ?? 100}
                  onChange={(event) => {
                    const value = Number(event.currentTarget.value);
                    void saveActorScalePercent(
                      Number.isFinite(value) ? value : 100,
                    );
                  }}
                />
                <output htmlFor="actor-scale-percent">
                  {appSettings?.actorScalePercent ?? 100}%
                </output>
              </span>
            </label>
            <label className="extension-toggle" htmlFor="autostart-enabled">
              <span>
                <strong>ログイン時に自動起動</strong>
                <small>
                  {autostartCanEnable
                    ? "このパソコンにログインしたとき、Yuukeiを起動します。"
                    : autostartEnabled
                      ? "このバージョンでは新しく設定できませんが、現在の設定は解除できます。"
                      : "このバージョンでは自動起動を設定できません。インストール済みのYuukeiから設定してください。"}
                </small>
              </span>
              <input
                id="autostart-enabled"
                type="checkbox"
                checked={autostartEnabled}
                disabled={!autostartCanEnable && !autostartEnabled}
                onChange={(event) => {
                  void toggleAutostart(event.currentTarget.checked);
                }}
              />
            </label>
            <p className="settings-title">会話のしかた</p>
            <p className="settings-note">
              返事を待つ長さと、会話をどこまで振り返るかを調整できます。迷ったときは初期値のままで大丈夫です。
            </p>
            <label className="app-setting-field" htmlFor="llm-timeout-ms">
              <span>
                <strong>返事を待つ時間</strong>
                <small>
                  時間を過ぎると、住人は会話を待たずに生活を続けます。
                </small>
              </span>
              <span className="number-setting-control">
                <input
                  id="llm-timeout-ms"
                  type="number"
                  min={1}
                  max={300}
                  step={1}
                  value={(runtimeSettings?.llmTimeoutMs ?? 30000) / 1000}
                  onChange={(event) => {
                    const value = Number(event.currentTarget.value);
                    void saveRuntimeSettings(
                      "llmTimeoutMs",
                      Number.isFinite(value) ? value * 1000 : 30000,
                    );
                  }}
                />
                <span aria-hidden="true">秒</span>
              </span>
            </label>
            <label className="app-setting-field" htmlFor="recent-context-count">
              <span>
                <strong>会話を振り返る長さ</strong>
                <small>
                  最近のやりとりを何件まで参考にするかを決めます。多いほど前の話を踏まえやすくなります。
                </small>
              </span>
              <input
                id="recent-context-count"
                type="number"
                min={0}
                max={100}
                step={1}
                value={runtimeSettings?.recentContextCount ?? 20}
                onChange={(event) => {
                  const value = Number(event.currentTarget.value);
                  void saveRuntimeSettings(
                    "recentContextCount",
                    Number.isFinite(value) ? value : 20,
                  );
                }}
              />
            </label>
            <label className="app-setting-field" htmlFor="talk-frequency">
              <span>
                <strong>話しかける頻度</strong>
                <small>住人が気分の変化をきっかけに話しかける頻度です。</small>
              </span>
              <select
                id="talk-frequency"
                value={talkFrequencyPreset(runtimeSettings)}
                onChange={(event) => {
                  void saveTalkFrequency(
                    event.currentTarget.value as TalkFrequencyPreset,
                  );
                }}
              >
                <option value="quiet">ひかえめ</option>
                <option value="normal">ふつう</option>
                <option value="chatty">よく話す</option>
              </select>
            </label>
            {appSettingsError ? (
              <p className="settings-error">
                {userFacingError(
                  appSettingsError,
                  "設定を保存できませんでした。もう一度お試しください。",
                )}
              </p>
            ) : null}
          </div>
        </>
      ),
    },
    {
      id: "worldPack",
      label: "住人と世界",
      ariaLabel: "住人と世界の設定",
      panelId: "settings-world-pack-panel",
      panelClassName: "world-pack-panel",
      content: (
        <>
          <div className="settings-copy">
            <p className="settings-section-label">現在使用中</p>
            <p className="settings-title">
              {worldPackStatus?.activeInstall.displayName ?? "読み込み中"}
            </p>
            <p className="settings-note">
              住人の見た目、性格、台詞や暮らし方がひとまとまりになっています。
            </p>
            {worldPackStatus?.fallbackActive ? (
              <p className="settings-error">
                前回選んだ住人と世界を読み込めなかったため、標準の内容を表示しています。もう一度選び直してください。
              </p>
            ) : null}
            {worldPackError ? (
              <p className="settings-error">
                {userFacingError(
                  worldPackError,
                  "住人と世界を変更できませんでした。もう一度お試しください。",
                )}
              </p>
            ) : null}
            <DaihonDiagnosticsPanel
              diagnostics={worldPackStatus?.daihonDiagnostics ?? []}
              expanded={showAllDaihonDiagnostics}
              onToggle={() =>
                setShowAllDaihonDiagnostics((current) => !current)
              }
            />
          </div>
          <div className="settings-actions settings-actions-wrap">
            <button
              type="button"
              onClick={chooseWorldPack}
              disabled={switchingPack}
            >
              別の住人と世界を選ぶ
            </button>
            <button
              type="button"
              className="secondary-button"
              onClick={importWorldPackZip}
              disabled={switchingPack}
            >
              配布ファイルから追加
            </button>
            <button
              type="button"
              className="secondary-button"
              onClick={resetWorldPack}
              disabled={switchingPack}
            >
              標準に戻す
            </button>
          </div>
        </>
      ),
    },
    {
      id: "sceneHistory",
      label: "物語の進み具合",
      ariaLabel: "物語の進み具合",
      panelId: "settings-scene-history-panel",
      panelClassName: "scene-history-panel",
      content: (
        <div className="settings-copy">
          <p className="settings-title">これまでに進んだ場面</p>
          <p className="settings-note">
            一度きりの場面など、住人との物語がどこまで進んだかを確認できます。
          </p>
          {sceneHistory?.entries.length ? (
            <section className="scene-history-list" aria-label="シーン実行履歴">
              {sceneHistory.entries.map((entry) => (
                <article
                  className="scene-history-row"
                  key={`${entry.eventName}:${entry.sceneName}`}
                >
                  <div className="scene-history-main">
                    <strong>{entry.sceneName}</strong>
                    <small>最後に進んだ日時</small>
                  </div>
                  <time dateTime={entry.lastExecutedAt}>
                    {new Date(entry.lastExecutedAt).toLocaleString()}
                  </time>
                </article>
              ))}
            </section>
          ) : (
            <p className="settings-note">まだ記録されたシーンはありません。</p>
          )}
          {worldPackError ? (
            <p className="settings-error">
              {userFacingError(
                worldPackError,
                "住人と世界を変更できませんでした。もう一度お試しください。",
              )}
            </p>
          ) : null}
          <div className="danger-zone">
            <div>
              <strong>物語を最初から始める</strong>
              <p>ここに表示されている進み具合をすべて元に戻します。</p>
            </div>
            <button
              type="button"
              className="danger-button"
              onClick={resetSceneHistory}
            >
              最初に戻す
            </button>
          </div>
        </div>
      ),
    },
    {
      id: "eventLog",
      label: "生活の記録",
      ariaLabel: "生活の記録 settings",
      panelId: "settings-event-log-panel",
      panelClassName: "memory-panel",
      content: (
        <EventLogSettingsPanel
          page={eventLogPage}
          error={eventLogError}
          loading={loadingEventLog}
          kindPrefix={eventLogKindPrefix}
          privacyFilter={eventLogPrivacyFilter}
          deleteBefore={eventLogDeleteBefore}
          deletePrefix={eventLogDeletePrefix}
          onKindPrefixChange={setEventLogKindPrefix}
          onPrivacyFilterChange={setEventLogPrivacyFilter}
          onDeleteBeforeChange={setEventLogDeleteBefore}
          onDeletePrefixChange={setEventLogDeletePrefix}
          onApplyFilters={() => void loadEventLog()}
          onLoadMore={() =>
            void loadEventLog(eventLogPage?.nextCursor ?? undefined, true)
          }
          onRefresh={() => void loadEventLog()}
          onDeleteBefore={() => void deleteEventLogBefore()}
          onDeletePrefix={() => void deleteEventLogByKindPrefix()}
          onDeleteAll={() => void deleteAllEventLog()}
        />
      ),
    },
    {
      id: "memories",
      label: "住人の記憶",
      ariaLabel: "住人の記憶",
      panelId: "settings-memories-panel",
      panelClassName: "memory-panel",
      content: (
        <MemorySettingsPanel
          memoryState={memoryState}
          memoryError={memoryError}
          loading={loadingMemories}
          editingFactId={editingFactId}
          editingFactText={editingFactText}
          onBeginFactEdit={beginFactEdit}
          onCancelFactEdit={() => {
            setEditingFactId(null);
            setEditingFactText("");
          }}
          onFactDraftChange={setEditingFactText}
          onSaveFact={saveFactEdit}
          onForgetEntry={forgetMemoryEntry}
          onForgetAll={forgetAllMemories}
          onLoadMore={loadMoreEpisodes}
          onRefresh={() => loadMemories()}
        />
      ),
    },
    {
      id: "extensions",
      label: "追加機能",
      ariaLabel: "追加機能の設定",
      panelId: "settings-extensions-panel",
      panelClassName: "extension-panel",
      content: (
        <>
          <div className="settings-copy">
            <p className="settings-title">
              {extensionState
                ? `${extensionState.installed.length}件の追加機能を利用できます`
                : "読み込み中"}
            </p>
            <p className="settings-note">
              声や会話などの機能を追加できます。追加元が信頼できることを確認してから利用してください。
            </p>
            {extensionError ? (
              <p className="settings-error">
                {userFacingError(
                  extensionError,
                  "追加機能を変更できませんでした。もう一度お試しください。",
                )}
              </p>
            ) : null}
            <div className="extension-list">
              {orderedExtensions.map((extension) => {
                const hookIndex = orderedBeforeCommandEmitExtensions.findIndex(
                  (candidate) =>
                    candidate.extensionId === extension.extensionId,
                );
                const canReorderHook = hookIndex >= 0;
                const permissionRows = extensionPermissionRows(extension);
                const usage = capabilityUsage?.extensions.find(
                  (usage) => usage.extensionId === extension.extensionId,
                );
                return (
                  <article
                    className="extension-row"
                    key={extension.extensionId}
                  >
                    <div className="extension-row-header">
                      <label className="extension-toggle">
                        <input
                          type="checkbox"
                          aria-label={extension.displayName}
                          checked={extension.enabled}
                          disabled={changingExtensions}
                          onChange={(event) =>
                            toggleExtension(
                              extension.extensionId,
                              event.currentTarget.checked,
                            )
                          }
                        />
                        <span>
                          <strong>{extension.displayName}</strong>
                          <small
                            className={[
                              "extension-runtime-status",
                              extension.runtimeStatus?.suspended
                                ? "is-suspended"
                                : "",
                            ]
                              .filter(Boolean)
                              .join(" ")}
                          >
                            {extensionRuntimeStatusLabel(extension)}
                          </small>
                        </span>
                      </label>
                      <div className="extension-actions">
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={
                            changingExtensions ||
                            !canReorderHook ||
                            hookIndex === 0
                          }
                          onClick={() =>
                            moveExtension(extension.extensionId, -1)
                          }
                        >
                          上へ
                        </button>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={
                            changingExtensions ||
                            !canReorderHook ||
                            hookIndex ===
                              orderedBeforeCommandEmitExtensions.length - 1
                          }
                          onClick={() =>
                            moveExtension(extension.extensionId, 1)
                          }
                        >
                          下へ
                        </button>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={changingExtensions}
                          onClick={() =>
                            restartExtensionProcess(extension.extensionId)
                          }
                        >
                          再起動
                        </button>
                        <button
                          type="button"
                          className="danger-button compact-button"
                          disabled={changingExtensions}
                          onClick={() =>
                            uninstallExtension(extension.extensionId)
                          }
                        >
                          削除
                        </button>
                      </div>
                    </div>
                    <div className="extension-main">
                      <p className="settings-note">
                        この機能が利用できる情報は、追加したときに確認した内容から変わりません。変更された場合は、いったん削除して追加し直してください。
                      </p>
                      {voicevoxCreditText(extension) ? (
                        <p className="extension-credit-note">
                          {voicevoxCreditText(extension)}
                        </p>
                      ) : null}
                      {permissionRows.length > 0 ? (
                        <dl className="extension-permissions">
                          {permissionRows.map((row) => (
                            <div
                              className={[
                                "extension-permission-row",
                                row.warning ? "is-warning" : "",
                              ]
                                .filter(Boolean)
                                .join(" ")}
                              key={row.label}
                            >
                              <dt>{row.label}</dt>
                              <dd>{row.value}</dd>
                            </div>
                          ))}
                        </dl>
                      ) : null}
                      {extension.settingsSchema ? (
                        <ExtensionSettingsForm
                          extension={extension}
                          client={client}
                          disabled={changingExtensions}
                          onResult={applyExtensionResult}
                        />
                      ) : null}
                      <ExtensionUsageSection usage={usage} />
                    </div>
                    {extension.lastLoadError ? (
                      <p className="settings-error">
                        この追加機能を読み込めませんでした。いったん削除し、配布元の案内を確認してから追加し直してください。
                      </p>
                    ) : null}
                  </article>
                );
              })}
            </div>
          </div>
          <div className="settings-actions">
            <button
              type="button"
              onClick={chooseExtension}
              disabled={changingExtensions}
            >
              追加機能を選ぶ
            </button>
            <button
              type="button"
              className="secondary-button"
              onClick={refreshCapabilityUsage}
              disabled={changingExtensions}
            >
              使用量を更新
            </button>
          </div>
        </>
      ),
    },
  ];
  const activeSettingsCategory =
    settingsCategories.find(
      (category) => category.id === activeSettingsCategoryId,
    ) ?? settingsCategories[0];
  const showOnboarding =
    !!onboardingState &&
    !onboardingState.completed &&
    !onboardingState.dismissed &&
    !onboardingDismissed;

  if (showOnboarding) {
    return (
      <main
        className="surface-shell settings-shell onboarding-shell"
        data-status={status}
      >
        <OnboardingFlow
          step={onboardingStep}
          worldPackStatus={worldPackStatus}
          worldPackError={worldPackError}
          switchingPack={switchingPack}
          onChooseWorldPack={chooseWorldPack}
          observationSettings={observationSettings}
          observationSettingsError={observationSettingsError}
          changingObservationSettings={changingObservationSettings}
          onToggleObservation={toggleObservationSetting}
          onStepChange={setOnboardingStep}
          onDismiss={() => void dismissOnboarding()}
          onComplete={() => void completeOnboarding()}
        />
      </main>
    );
  }

  const settingsBlocked = Boolean(ownedOverlay || pendingExtensionInstall);

  return (
    <main className="surface-shell settings-shell" data-status={status}>
      <section
        aria-hidden={settingsBlocked ? true : undefined}
        aria-label="設定"
        className="settings-workspace"
        inert={settingsBlocked ? true : undefined}
      >
        <aside className="settings-sidebar">
          <div className="settings-sidebar-head">
            <h2>設定</h2>
          </div>
          <div
            className="settings-menu"
            aria-label="設定カテゴリ"
            role="tablist"
          >
            {settingsCategories.map((category) => {
              const selected = category.id === activeSettingsCategory.id;
              return (
                <button
                  key={category.id}
                  id={`settings-${category.id}-tab`}
                  type="button"
                  className="settings-menu-item"
                  role="tab"
                  aria-selected={selected}
                  aria-controls={category.panelId}
                  onClick={() => setActiveSettingsCategoryId(category.id)}
                >
                  <span className="settings-menu-mark" aria-hidden="true">
                    {category.label.slice(0, 1)}
                  </span>
                  <span>{category.label}</span>
                </button>
              );
            })}
          </div>
        </aside>
        <div className="settings-content">
          <header className="settings-content-header">
            <div>
              <p className="settings-eyebrow">設定項目</p>
              <h2>{activeSettingsCategory.label}</h2>
            </div>
          </header>
          <section
            className={["settings-panel", activeSettingsCategory.panelClassName]
              .filter(Boolean)
              .join(" ")}
            id={activeSettingsCategory.panelId}
            role="tabpanel"
            aria-label={activeSettingsCategory.ariaLabel}
          >
            {activeSettingsCategory.content}
          </section>
        </div>
      </section>
      {ownedOverlay ? (
        <OwnedOverlay
          key={ownedOverlay.overlayId}
          overlay={ownedOverlay}
          onDismiss={async (reason) => {
            try {
              await client.dismissOwnedOverlay(ownedOverlay.overlayId, reason);
            } catch (error) {
              console.warn("Failed to dismiss Yuukei-owned overlay", error);
              throw error;
            }
          }}
        />
      ) : pendingExtensionInstall ? (
        <ExtensionConsentDialog
          busy={changingExtensions}
          error={extensionError}
          inspection={pendingExtensionInstall.inspection}
          onApprove={() => void approveExtensionInstall()}
          onCancel={cancelExtensionInstall}
        />
      ) : null}
    </main>
  );
}

function latestOwnedOverlay(
  overlays: StageOwnedOverlay[],
): StageOwnedOverlay | null {
  return (
    [...overlays].sort(
      (left, right) => right.createdAtMs - left.createdAtMs,
    )[0] ?? null
  );
}

function defaultRuntimeSettings(): RuntimeSettingsState {
  return {
    llmTimeoutMs: 30_000,
    recentContextCount: 20,
    talkDesireLow: 30,
    talkDesireHigh: 80,
    settingsPath: "",
  };
}

function talkFrequencyPreset(
  settings: RuntimeSettingsState | null,
): TalkFrequencyPreset {
  const current = settings ?? defaultRuntimeSettings();
  return (
    Object.entries(TALK_FREQUENCY_PRESETS) as Array<
      [TalkFrequencyPreset, { low: number; high: number }]
    >
  ).reduce((closest, candidate) => {
    const closestValues = TALK_FREQUENCY_PRESETS[closest];
    const closestDistance =
      Math.abs(current.talkDesireLow - closestValues.low) +
      Math.abs(current.talkDesireHigh - closestValues.high);
    const candidateDistance =
      Math.abs(current.talkDesireLow - candidate[1].low) +
      Math.abs(current.talkDesireHigh - candidate[1].high);
    return candidateDistance < closestDistance ? candidate[0] : closest;
  }, "normal" as TalkFrequencyPreset);
}

function eventLogGroupLabel(prefix: string): string {
  const labels: Record<string, string> = {
    "conversation.": "あなたからの会話の記録",
    "dialogue.": "住人からの会話の記録",
    "desktop.": "パソコン上で気づいたことの記録",
    "presence.": "住人の日々のようすの記録",
    "memory.": "記憶の変化の記録",
    "scene.": "物語の進行の記録",
  };
  return labels[prefix] ?? "選んだ生活の記録";
}

function userFacingError(error: unknown, fallback: string): string {
  const message = error instanceof Error ? error.message : String(error);
  const looksTechnical =
    /(?:\.json|manifest|protocol|capability|extension|world.?pack|failed|invalid|missing|unknown|timeout|[\\/][^ ]+)/i.test(
      message,
    );
  return message.trim() && !looksTechnical ? message : fallback;
}
