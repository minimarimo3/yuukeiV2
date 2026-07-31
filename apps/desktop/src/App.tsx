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
  type ExtensionSettingsChangeResult,
  type ExtensionInstallInspection,
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
      setEventLogError(error instanceof Error ? error.message : String(error));
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
      setWorldPackError(error instanceof Error ? error.message : String(error));
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
        ? "\n\n同じpackIdのインポート済みPackがあります。続行すると置き換えます。"
        : "";
      const confirmed = window.confirm(
        `このWorld Packの配布条件\n\n${inspection.displayName} (${inspection.packId})\n${inspection.licenseSource ?? "ライセンス表記なし"}\n\n${licenseText}${replaceNotice}\n\nこのWorld Packを読み込みますか？`,
      );
      if (!confirmed) return;
      const result = await client.importWorldPackZip(path);
      setWorldPackStatus(result.status);
      setStatus("ready");
      void loadMemories();
    } catch (error) {
      setWorldPackError(error instanceof Error ? error.message : String(error));
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
      setWorldPackError(error instanceof Error ? error.message : String(error));
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
      setExtensionError(error instanceof Error ? error.message : String(error));
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
      setExtensionError(error instanceof Error ? error.message : String(error));
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
      setExtensionError(error instanceof Error ? error.message : String(error));
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
      setExtensionError(error instanceof Error ? error.message : String(error));
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
      setExtensionError(error instanceof Error ? error.message : String(error));
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
      setExtensionError(error instanceof Error ? error.message : String(error));
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
      setExtensionError(error instanceof Error ? error.message : String(error));
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
    const current =
      runtimeSettings ??
      ({
        llmTimeoutMs: 30_000,
        recentContextCount: 20,
        talkDesireLow: 30,
        talkDesireHigh: 80,
        settingsPath: "",
      } satisfies RuntimeSettingsState);
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

  async function resetSceneHistory() {
    if (
      !window.confirm(
        "このWorld Packのシーン実行履歴をすべてリセットします。この操作は取り消せません。続けますか？",
      )
    ) {
      return;
    }
    setWorldPackError(null);
    try {
      setSceneHistory(await client.resetSceneHistory());
    } catch (error) {
      setWorldPackError(error instanceof Error ? error.message : String(error));
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
        error instanceof Error ? error.message : String(error),
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
      if (!confirmEventLogDeletion(`${timestamp}より前`, count)) return;
      await client.deleteEventLogBefore(timestamp);
      await loadEventLog();
    } catch (error) {
      setEventLogError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoadingEventLog(false);
    }
  }

  async function deleteEventLogByKindPrefix() {
    const prefix = eventLogDeletePrefix.trim();
    if (!prefix) {
      setEventLogError("種類の前方一致を入力してください。");
      return;
    }
    setLoadingEventLog(true);
    try {
      const count = await client.countEventLogDeleteByKindPrefix(prefix);
      if (!confirmEventLogDeletion(`種類が「${prefix}」で始まる記録`, count)) {
        return;
      }
      await client.deleteEventLogByKindPrefix(prefix);
      await loadEventLog();
    } catch (error) {
      setEventLogError(error instanceof Error ? error.message : String(error));
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
      setEventLogError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoadingEventLog(false);
    }
  }

  function confirmEventLogDeletion(label: string, count: number) {
    return window.confirm(
      `${label}を削除します。\n\n削除予定: ${count}件\nこの操作は取り消せません。\n住人の記憶(要約)には残っている場合があります。記憶は『記憶』タブから個別に忘れさせられます。\n\n続けますか？`,
    );
  }

  const settingsCategories: SettingsCategory[] = [
    {
      id: "app",
      label: "App",
      ariaLabel: "App settings",
      panelId: "settings-app-panel",
      content: (
        <>
          <div className="settings-copy app-settings-copy">
            <h2>App</h2>
            <p className="settings-title">おしゃべりの間隔</p>
            <p className="settings-note">
              分単位で設定します。0分で話さなくなります。
            </p>
            <label
              className="app-setting-field"
              htmlFor="talk-interval-minutes"
            >
              <span>
                <strong>おしゃべりの間隔</strong>
                <small>{appSettings?.settingsPath ?? ""}</small>
              </span>
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
                    ? "このデバイスにログインしたとき、Yuukeiを起動します。"
                    : autostartEnabled
                      ? "開発版からの新規登録はできません。OFFにしてbuild版から設定し直してください。"
                      : "開発版では利用できません。build版から設定してください。"}
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
            <p className="settings-title">AI呼び出し</p>
            <p className="settings-note">
              台本や会話補完で使うAI待ち時間と、直近文脈の件数です。
            </p>
            <label className="app-setting-field" htmlFor="llm-timeout-ms">
              <span>
                <strong>AI待ち時間</strong>
                <small>1000〜300000ミリ秒に丸められます。</small>
              </span>
              <input
                id="llm-timeout-ms"
                type="number"
                min={1000}
                max={300000}
                step={1000}
                value={runtimeSettings?.llmTimeoutMs ?? 30000}
                onChange={(event) => {
                  const value = Number(event.currentTarget.value);
                  void saveRuntimeSettings(
                    "llmTimeoutMs",
                    Number.isFinite(value) ? value : 30000,
                  );
                }}
              />
            </label>
            <label className="app-setting-field" htmlFor="recent-context-count">
              <span>
                <strong>直近文脈の件数</strong>
                <small>{runtimeSettings?.settingsPath ?? ""}</small>
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
            <p className="settings-title">気分の話しかけやすさ</p>
            <p className="settings-note">
              低い値未満では話しかけを控え、高い値以上では気分変化で話しかけます。
            </p>
            <label className="app-setting-field" htmlFor="talk-desire-low">
              <span>
                <strong>話したい度: 低</strong>
                <small>0〜100。高より小さく丸められます。</small>
              </span>
              <input
                id="talk-desire-low"
                type="number"
                min={0}
                max={100}
                step={1}
                value={runtimeSettings?.talkDesireLow ?? 30}
                onChange={(event) => {
                  const value = Number(event.currentTarget.value);
                  void saveRuntimeSettings(
                    "talkDesireLow",
                    Number.isFinite(value) ? value : 30,
                  );
                }}
              />
            </label>
            <label className="app-setting-field" htmlFor="talk-desire-high">
              <span>
                <strong>話したい度: 高</strong>
                <small>0〜100。低より大きく丸められます。</small>
              </span>
              <input
                id="talk-desire-high"
                type="number"
                min={0}
                max={100}
                step={1}
                value={runtimeSettings?.talkDesireHigh ?? 80}
                onChange={(event) => {
                  const value = Number(event.currentTarget.value);
                  void saveRuntimeSettings(
                    "talkDesireHigh",
                    Number.isFinite(value) ? value : 80,
                  );
                }}
              />
            </label>
            {appSettingsError ? (
              <p className="settings-error">{appSettingsError}</p>
            ) : null}
          </div>
        </>
      ),
    },
    {
      id: "worldPack",
      label: "World Pack",
      ariaLabel: "World Pack settings",
      panelId: "settings-world-pack-panel",
      panelClassName: "world-pack-panel",
      content: (
        <>
          <div className="settings-copy">
            <p className="settings-section-label">現在使用中</p>
            <p className="settings-title">
              {worldPackStatus?.activeInstall.displayName ?? "loading"}
            </p>
            <p className="settings-path">
              {worldPackStatus?.activeInstall.canonicalRoot ?? ""}
            </p>
            {worldPackStatus?.fallbackActive ? (
              <p className="settings-error">
                保存済み Pack を読み込めませんでした:{" "}
                {worldPackStatus.lastLoadError ?? "unknown error"}
              </p>
            ) : null}
            {worldPackError ? (
              <p className="settings-error">{worldPackError}</p>
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
              フォルダを選択
            </button>
            <button
              type="button"
              className="secondary-button"
              onClick={importWorldPackZip}
              disabled={switchingPack}
            >
              zipから読み込む
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
      label: "シーン履歴",
      ariaLabel: "Scene history settings",
      panelId: "settings-scene-history-panel",
      panelClassName: "scene-history-panel",
      content: (
        <div className="settings-copy">
          <p className="settings-title">このWorld Packの実行履歴</p>
          <p className="settings-path">{sceneHistory?.historyPath ?? ""}</p>
          {sceneHistory?.entries.length ? (
            <section className="scene-history-list" aria-label="シーン実行履歴">
              {sceneHistory.entries.map((entry) => (
                <article
                  className="scene-history-row"
                  key={`${entry.eventName}:${entry.sceneName}`}
                >
                  <div className="scene-history-main">
                    <strong>{entry.sceneName}</strong>
                    <small>合図: {entry.eventName}</small>
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
            <p className="settings-error">{worldPackError}</p>
          ) : null}
          <div className="danger-zone">
            <div>
              <strong>履歴をリセット</strong>
              <p>このWorld Packのシーン実行履歴をすべて削除します。</p>
            </div>
            <button
              type="button"
              className="danger-button"
              onClick={resetSceneHistory}
            >
              全リセット
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
      label: "記憶",
      ariaLabel: "記憶 settings",
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
      label: "Extensions",
      ariaLabel: "Extension settings",
      panelId: "settings-extensions-panel",
      panelClassName: "extension-panel",
      content: (
        <>
          <div className="settings-copy">
            <p className="settings-title">
              {extensionState
                ? `${extensionState.installed.length}件のExtensionをインストール済み`
                : "読み込み中"}
            </p>
            <p className="settings-path">
              {extensionState?.extensionRoot ?? ""}
            </p>
            <p className="settings-note">
              {extensionState?.trustedCodeNotice ?? ""}
            </p>
            {extensionError ? (
              <p className="settings-error">{extensionError}</p>
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
                          aria-label={`${extension.displayName} ${extension.extensionId}`}
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
                          <small>{extension.extensionId}</small>
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
                        権限は追加時に許可したmanifestで固定されています。変更するには、このExtensionを削除して追加し直してください。
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
                        {extension.lastLoadError}
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
              追加
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
        aria-label="Settings"
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
