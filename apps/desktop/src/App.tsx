import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { ExtensionSettingField } from "@yuukei/protocol";
import {
  tauriYuukeiClient,
  type AppSettingsState,
  type CapabilityUsageState,
  type ExtensionCapabilityUsage,
  type DaihonDiagnosticEntry,
  type ExtensionSettingsChangeResult,
  type ExtensionSettingsState,
  type InstalledExtension,
  type MemoryEntryKind,
  type MemoryForgetEntry,
  type ObservationSettingsState,
  type ObservationSettingsUpdate,
  type OnboardingState,
  type ResidentMemoryState,
  type WorldPackSelectionState,
  type YuukeiClient
} from "./yuukeiClient";

type AppProps = {
  client?: YuukeiClient;
};

type SettingsCategoryId =
  | "app"
  | "worldPack"
  | "observations"
  | "memories"
  | "extensions";
const MEMORY_PAGE_SIZE = 50;

type SettingsCategory = {
  id: SettingsCategoryId;
  label: string;
  ariaLabel: string;
  panelId: string;
  panelClassName?: string;
  content: ReactNode;
};

export function App({ client = tauriYuukeiClient }: AppProps) {
  const [status, setStatus] = useState("connecting");
  const [activeSettingsCategoryId, setActiveSettingsCategoryId] =
    useState<SettingsCategoryId>("worldPack");
  const [worldPackStatus, setWorldPackStatus] =
    useState<WorldPackSelectionState | null>(null);
  const [appSettings, setAppSettings] = useState<AppSettingsState | null>(null);
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
  const [observationSettingsError, setObservationSettingsError] =
    useState<string | null>(null);
  const [extensionError, setExtensionError] = useState<string | null>(null);
  const [memoryState, setMemoryState] = useState<ResidentMemoryState | null>(null);
  const [memoryError, setMemoryError] = useState<string | null>(null);
  const [loadingMemories, setLoadingMemories] = useState(false);
  const [editingFactId, setEditingFactId] = useState<string | null>(null);
  const [editingFactText, setEditingFactText] = useState("");
  const [switchingPack, setSwitchingPack] = useState(false);
  const [changingObservationSettings, setChangingObservationSettings] =
    useState(false);
  const [changingExtensions, setChangingExtensions] = useState(false);
  const [showAllDaihonDiagnostics, setShowAllDaihonDiagnostics] =
    useState(false);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function connect() {
      try {
        unlisteners.push(await client.onAssetsChanged(() => {
          void refreshSettings();
          void loadMemories();
        }));
        unlisteners.push(
          await client.onWorldPackStatus((nextWorldPackStatus) => {
            if (!disposed) {
              setWorldPackStatus(nextWorldPackStatus);
            }
            void loadMemories();
          })
        );
        unlisteners.push(
          await client.onOnboardingDismissed(() => {
            if (!disposed) {
              setOnboardingDismissed(true);
            }
          })
        );
        await refreshSettings();
        await loadMemories();
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
        nextObservationSettings,
        nextOnboardingState,
        nextExtensionState,
        nextCapabilityUsage
      ] =
        await Promise.all([
          client.getWorldPackStatus(),
          client.getAppSettings(),
          client.getObservationSettings(),
          client.getOnboardingState(),
          client.getExtensionSettings(),
          client.getCapabilityUsage()
        ]);
      if (!disposed) {
        setWorldPackStatus(nextWorldPackStatus);
        setAppSettings(nextAppSettings);
        setObservationSettings(nextObservationSettings);
        setOnboardingState(nextOnboardingState);
        setExtensionState(nextExtensionState);
        setCapabilityUsage(nextCapabilityUsage);
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
      extensionState?.hookOrder.beforeCommandEmit ?? []
    );
  }, [extensionState]);
  const orderedExtensions = useMemo(() => {
    return orderExtensionsForHook(
      extensionState?.installed ?? [],
      extensionState?.hookOrder.beforeCommandEmit ?? []
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
              episodes: [...current.episodes, ...next.episodes]
            }
          : next
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
        `このWorld Packの配布条件\n\n${inspection.displayName} (${inspection.packId})\n${inspection.licenseSource ?? "ライセンス表記なし"}\n\n${licenseText}${replaceNotice}\n\nこのWorld Packを読み込みますか？`
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
      applyExtensionResult(await client.installExtensionDirectory(path));
    } catch (error) {
      setExtensionError(error instanceof Error ? error.message : String(error));
    } finally {
      setChangingExtensions(false);
    }
  }

  async function toggleExtension(extensionId: string, enabled: boolean) {
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      applyExtensionResult(await client.setExtensionEnabled(extensionId, enabled));
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

  async function moveExtension(extensionId: string, direction: -1 | 1) {
    if (!extensionState) return;
    const currentOrder = orderedBeforeCommandEmitExtensions.map(
      (extension) => extension.extensionId
    );
    const index = currentOrder.indexOf(extensionId);
    const nextIndex = index + direction;
    if (index < 0 || nextIndex < 0 || nextIndex >= currentOrder.length) return;
    const nextOrder = [...currentOrder];
    [nextOrder[index], nextOrder[nextIndex]] = [
      nextOrder[nextIndex],
      nextOrder[index]
    ];
    setExtensionError(null);
    setChangingExtensions(true);
    try {
      applyExtensionResult(
        await client.setExtensionHookOrder("beforeCommandEmit", nextOrder)
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
      setAppSettingsError(error instanceof Error ? error.message : String(error));
    }
  }

  async function toggleObservationSetting(
    key: keyof ObservationSettingsUpdate,
    enabled: boolean
  ) {
    const current =
      observationSettings ??
      ({
        windows: false,
        folders: false,
        downloads: false
      } satisfies ObservationSettingsUpdate);
    const next: ObservationSettingsUpdate = {
      windows: current.windows,
      folders: current.folders,
      downloads: current.downloads,
      [key]: enabled
    };
    setObservationSettingsError(null);
    setChangingObservationSettings(true);
    try {
      setObservationSettings(await client.setObservationSettings(next));
    } catch (error) {
      setObservationSettingsError(
        error instanceof Error ? error.message : String(error)
      );
    } finally {
      setChangingObservationSettings(false);
    }
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
      "すべての記憶を忘れます。この操作は取り消せません。続けますか？"
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

  const settingsCategories: SettingsCategory[] = [
    {
      id: "app",
      label: "App",
      ariaLabel: "App settings",
      panelId: "settings-app-panel",
      content: (
        <>
          <div className="settings-copy">
            <h2>App</h2>
            <p className="settings-title">おしゃべりの間隔</p>
            <p className="settings-note">
              分単位で設定します。0分で話さなくなります。
            </p>
            <label className="app-setting-field" htmlFor="talk-interval-minutes">
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
            {appSettingsError ? (
              <p className="settings-error">{appSettingsError}</p>
            ) : null}
          </div>
        </>
      )
    },
    {
      id: "worldPack",
      label: "World Pack",
      ariaLabel: "World Pack settings",
      panelId: "settings-world-pack-panel",
      content: (
        <>
          <div className="settings-copy">
            <h2>World Pack</h2>
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
          <div className="settings-actions">
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
              Default
            </button>
          </div>
        </>
      )
    },
    {
      id: "observations",
      label: "観測",
      ariaLabel: "Observation and privacy settings",
      panelId: "settings-observations-panel",
      content: (
        <div className="settings-copy observation-settings">
          <h2>観測とプライバシー</h2>
          <p className="settings-title">Desktop Terrain Observation</p>
          <p className="settings-path">
            {observationSettings?.settingsPath ?? ""}
          </p>
          {observationSettingsError ? (
            <p className="settings-error">{observationSettingsError}</p>
          ) : null}
          <ObservationToggle
            label="ウィンドウ"
            description="アプリ名とウィンドウの出現・消滅だけを記録します(タイトルは記録しません)"
            checked={observationSettings?.windows ?? false}
            disabled={changingObservationSettings}
            onChange={(checked) => toggleObservationSetting("windows", checked)}
          />
          <ObservationToggle
            label="フォルダ"
            description="開いた場所の種類だけを記録します(パスは記録しません)"
            checked={observationSettings?.folders ?? false}
            disabled={changingObservationSettings}
            onChange={(checked) => toggleObservationSetting("folders", checked)}
          />
          <ObservationToggle
            label="ダウンロード"
            description="ファイル名と種類を記録します(場所は記録しません)"
            checked={observationSettings?.downloads ?? false}
            disabled={changingObservationSettings}
            onChange={(checked) =>
              toggleObservationSetting("downloads", checked)
            }
          />
        </div>
      )
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
      )
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
            <h2>Extensions</h2>
            <p className="settings-title">
              {extensionState
                ? `${extensionState.installed.length} installed`
                : "loading"}
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
                const hookIndex =
                  orderedBeforeCommandEmitExtensions.findIndex(
                    (candidate) =>
                      candidate.extensionId === extension.extensionId
                  );
                const canReorderHook = hookIndex >= 0;
                const permissionRows = extensionPermissionRows(extension);
                const usage = capabilityUsage?.extensions.find(
                  (usage) => usage.extensionId === extension.extensionId
                );
                return (
                  <article
                    className="extension-row"
                    key={extension.extensionId}
                  >
                    <div className="extension-main">
                      <label className="extension-toggle">
                        <input
                          type="checkbox"
                          aria-label={`${extension.displayName} ${extension.extensionId}`}
                          checked={extension.enabled}
                          disabled={changingExtensions}
                          onChange={(event) =>
                            toggleExtension(
                              extension.extensionId,
                              event.currentTarget.checked
                            )
                          }
                        />
                        <span>
                          <strong>{extension.displayName}</strong>
                          <small>{extension.extensionId}</small>
                        </span>
                      </label>
                      {permissionRows.length > 0 ? (
                        <dl className="extension-permissions">
                          {permissionRows.map((row) => (
                            <div
                              className={[
                                "extension-permission-row",
                                row.warning ? "is-warning" : ""
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
                    <div className="extension-actions">
                      <button
                        type="button"
                        className="secondary-button compact-button"
                        disabled={
                          changingExtensions || !canReorderHook || hookIndex === 0
                        }
                        onClick={() => moveExtension(extension.extensionId, -1)}
                      >
                        上
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
                        onClick={() => moveExtension(extension.extensionId, 1)}
                      >
                        下
                      </button>
                      <button
                        type="button"
                        className="secondary-button compact-button"
                        disabled={changingExtensions}
                        onClick={() => uninstallExtension(extension.extensionId)}
                      >
                        削除
                      </button>
                    </div>
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
      )
    }
  ];
  const activeSettingsCategory =
    settingsCategories.find(
      (category) => category.id === activeSettingsCategoryId
    ) ?? settingsCategories[0];
  const intelligenceExtension = orderedExtensions.find(
    (extension) => extension.extensionId === "yuukei-intelligence"
  );
  const showOnboarding =
    !!onboardingState && !onboardingState.completed && !onboardingDismissed;

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
          extension={intelligenceExtension}
          client={client}
          changingExtensions={changingExtensions}
          onExtensionResult={applyExtensionResult}
          observationSettings={observationSettings}
          observationSettingsError={observationSettingsError}
          changingObservationSettings={changingObservationSettings}
          onToggleObservation={toggleObservationSetting}
          onStepChange={setOnboardingStep}
          onDismiss={() => setOnboardingDismissed(true)}
          onComplete={() => void completeOnboarding()}
        />
      </main>
    );
  }

  return (
    <main className="surface-shell settings-shell" data-status={status}>
      <section className="settings-workspace" aria-label="Settings">
        <aside className="settings-sidebar">
          <div className="settings-sidebar-head">
            <p className="settings-eyebrow">Preferences</p>
            <h2>設定</h2>
          </div>
          <nav className="settings-menu" aria-label="設定カテゴリ" role="tablist">
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
          </nav>
        </aside>
        <div className="settings-content">
          <header className="settings-content-header">
            <div>
              <p className="settings-eyebrow">Selected</p>
              <h2>{activeSettingsCategory.label}</h2>
            </div>
            <span className="settings-badge">{activeSettingsCategory.id}</span>
          </header>
          <section
            className={[
              "settings-panel",
              activeSettingsCategory.panelClassName
            ]
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
    </main>
  );
}

type OnboardingFlowProps = {
  step: number;
  worldPackStatus: WorldPackSelectionState | null;
  worldPackError: string | null;
  switchingPack: boolean;
  onChooseWorldPack: () => void;
  extension?: InstalledExtension;
  client: YuukeiClient;
  changingExtensions: boolean;
  onExtensionResult: (result: ExtensionSettingsChangeResult) => void;
  observationSettings: ObservationSettingsState | null;
  observationSettingsError: string | null;
  changingObservationSettings: boolean;
  onToggleObservation: (
    key: keyof ObservationSettingsUpdate,
    enabled: boolean
  ) => void;
  onStepChange: (step: number) => void;
  onDismiss: () => void;
  onComplete: () => void;
};

function OnboardingFlow({
  step,
  worldPackStatus,
  worldPackError,
  switchingPack,
  onChooseWorldPack,
  extension,
  client,
  changingExtensions,
  onExtensionResult,
  observationSettings,
  observationSettingsError,
  changingObservationSettings,
  onToggleObservation,
  onStepChange,
  onDismiss,
  onComplete
}: OnboardingFlowProps) {
  const clampedStep = Math.max(0, Math.min(step, 3));
  return (
    <section className="onboarding-flow" aria-label="初回設定">
      <header className="onboarding-header">
        <div>
          <p className="settings-eyebrow">はじめまして</p>
          <h1>Yuukeiを始める</h1>
        </div>
        <button type="button" className="secondary-button" onClick={onDismiss}>
          あとで
        </button>
      </header>
      <div className="onboarding-progress" aria-label="オンボーディングの進行">
        {["ようこそ", "AI", "観測", "完了"].map((label, index) => (
          <span
            className={[
              "onboarding-progress-step",
              index === clampedStep ? "is-active" : ""
            ]
              .filter(Boolean)
              .join(" ")}
            key={label}
          >
            {label}
          </span>
        ))}
      </div>
      <div className="onboarding-panel">
        {clampedStep === 0 ? (
          <>
            <div className="settings-copy">
              <h2>ようこそ</h2>
              <p className="settings-title">
                この子はあなたのデバイスに住みます。
              </p>
              <p className="settings-note">
                World Packが、住人の世界観や台本、暮らし方を決めます。
              </p>
              <p className="settings-title">
                {worldPackStatus?.activeInstall.displayName ?? "読み込み中"}
              </p>
              <p className="settings-path">
                {worldPackStatus?.activeInstall.canonicalRoot ?? ""}
              </p>
              {worldPackError ? (
                <p className="settings-error">{worldPackError}</p>
              ) : null}
            </div>
            <div className="settings-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={onChooseWorldPack}
                disabled={switchingPack}
              >
                別のWorld Packを選ぶ
              </button>
              <button type="button" onClick={() => onStepChange(1)}>
                次へ
              </button>
            </div>
          </>
        ) : null}

        {clampedStep === 1 ? (
          <>
            <div className="settings-copy onboarding-ai-step">
              <h2>AI(ことば)の設定</h2>
              <p className="settings-title">
                AIがなくても、台本で毎日の生活は動きます。あとから設定画面で変えられます。
              </p>
              {extension?.settingsSchema ? (
                <ExtensionSettingsForm
                  extension={extension}
                  client={client}
                  disabled={changingExtensions}
                  onResult={onExtensionResult}
                />
              ) : (
                <p className="settings-note">
                  yuukei-intelligence拡張が見つからないため、このまま進めます。
                </p>
              )}
            </div>
            <div className="settings-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => onStepChange(0)}
              >
                戻る
              </button>
              <button
                type="button"
                className="secondary-button"
                onClick={() => onStepChange(2)}
              >
                AIなしで始める
              </button>
              <button type="button" onClick={() => onStepChange(2)}>
                次へ
              </button>
            </div>
          </>
        ) : null}

        {clampedStep === 2 ? (
          <>
            <div className="settings-copy observation-settings">
              <h2>観測とプライバシー</h2>
              <p className="settings-title">
                ONにした観測だけを記録します。どれもあとから設定で変えられます。
              </p>
              {observationSettingsError ? (
                <p className="settings-error">{observationSettingsError}</p>
              ) : null}
              <ObservationToggle
                label="ウィンドウ"
                description="アプリ名とウィンドウの出現・消滅だけを記録します(タイトルは記録しません)"
                checked={observationSettings?.windows ?? false}
                disabled={changingObservationSettings}
                onChange={(checked) => onToggleObservation("windows", checked)}
              />
              <ObservationToggle
                label="フォルダ"
                description="開いた場所の種類だけを記録します(パスは記録しません)"
                checked={observationSettings?.folders ?? false}
                disabled={changingObservationSettings}
                onChange={(checked) => onToggleObservation("folders", checked)}
              />
              <ObservationToggle
                label="ダウンロード"
                description="ファイル名と種類を記録します(場所は記録しません)"
                checked={observationSettings?.downloads ?? false}
                disabled={changingObservationSettings}
                onChange={(checked) => onToggleObservation("downloads", checked)}
              />
            </div>
            <div className="settings-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => onStepChange(1)}
              >
                戻る
              </button>
              <button type="button" onClick={() => onStepChange(3)}>
                次へ
              </button>
            </div>
          </>
        ) : null}

        {clampedStep === 3 ? (
          <>
            <div className="settings-copy">
              <h2>完了</h2>
              <p className="settings-title">いってらっしゃい。</p>
              <p className="settings-note">
                今日から、このデバイスで一緒の生活が始まります。
              </p>
            </div>
            <div className="settings-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => onStepChange(2)}
              >
                戻る
              </button>
              <button type="button" onClick={onComplete}>
                完了して始める
              </button>
            </div>
          </>
        ) : null}
      </div>
    </section>
  );
}

type ObservationToggleProps = {
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
};

function ObservationToggle({
  label,
  description,
  checked,
  disabled,
  onChange
}: ObservationToggleProps) {
  return (
    <label className="extension-toggle observation-toggle">
      <input
        type="checkbox"
        aria-label={label}
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      <span>
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
    </label>
  );
}

type MemorySettingsPanelProps = {
  memoryState: ResidentMemoryState | null;
  memoryError: string | null;
  loading: boolean;
  editingFactId: string | null;
  editingFactText: string;
  onBeginFactEdit: (id: string, text: string) => void;
  onCancelFactEdit: () => void;
  onFactDraftChange: (text: string) => void;
  onSaveFact: (id: string) => Promise<void>;
  onForgetEntry: (kind: MemoryEntryKind, id: string) => Promise<void>;
  onForgetAll: () => Promise<void>;
  onLoadMore: () => Promise<void>;
  onRefresh: () => Promise<void>;
};

function MemorySettingsPanel({
  memoryState,
  memoryError,
  loading,
  editingFactId,
  editingFactText,
  onBeginFactEdit,
  onCancelFactEdit,
  onFactDraftChange,
  onSaveFact,
  onForgetEntry,
  onForgetAll,
  onLoadMore,
  onRefresh
}: MemorySettingsPanelProps) {
  const facts = memoryState?.facts ?? [];
  const episodes = memoryState?.episodes ?? [];
  const episodeTotal = memoryState?.episodeTotal ?? 0;
  const hasMemory = facts.length > 0 || episodeTotal > 0;
  const hasMoreEpisodes = episodes.length < episodeTotal;

  return (
    <>
      <div className="settings-copy memory-copy">
        <h2>記憶</h2>
        <p className="settings-title">派生記憶</p>
        <p className="settings-note">
          facts は編集できます。episodes は出来事の記録として削除のみできます。
        </p>
        {memoryError ? <p className="settings-error">{memoryError}</p> : null}
        {!memoryError && !loading && !hasMemory ? (
          <p className="settings-note">まだ記憶がありません。</p>
        ) : null}

        <section className="memory-section" aria-label="facts">
          <div className="memory-section-head">
            <h3>facts</h3>
            <span>{facts.length}</span>
          </div>
          <div className="memory-list">
            {facts.map((fact) => {
              const editing = editingFactId === fact.id;
              return (
                <article className="memory-row" key={fact.id}>
                  {editing ? (
                    <textarea
                      aria-label={`fact ${fact.id}`}
                      value={editingFactText}
                      maxLength={500}
                      onChange={(event) =>
                        onFactDraftChange(event.currentTarget.value)
                      }
                    />
                  ) : (
                    <div className="memory-text">
                      <p>{fact.text}</p>
                      <small>{formatMemoryTimestamp(fact.updatedAt)}</small>
                    </div>
                  )}
                  <div className="memory-actions">
                    {editing ? (
                      <>
                        <button
                          type="button"
                          className="compact-button"
                          disabled={loading}
                          onClick={() => void onSaveFact(fact.id)}
                        >
                          保存
                        </button>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={loading}
                          onClick={onCancelFactEdit}
                        >
                          キャンセル
                        </button>
                      </>
                    ) : (
                      <>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={loading}
                          onClick={() => onBeginFactEdit(fact.id, fact.text)}
                        >
                          編集
                        </button>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={loading}
                          onClick={() => void onForgetEntry("fact", fact.id)}
                        >
                          削除
                        </button>
                      </>
                    )}
                  </div>
                </article>
              );
            })}
          </div>
        </section>

        <section className="memory-section" aria-label="episodes">
          <div className="memory-section-head">
            <h3>episodes</h3>
            <span>
              {episodes.length}/{episodeTotal}
            </span>
          </div>
          <div className="memory-list">
            {episodes.map((episode) => (
              <article className="memory-row" key={episode.id}>
                <div className="memory-text">
                  <p>{episode.text}</p>
                  <small>{formatMemoryTimestamp(episode.timestamp)}</small>
                </div>
                <div className="memory-actions">
                  <button
                    type="button"
                    className="secondary-button compact-button"
                    disabled={loading}
                    onClick={() => void onForgetEntry("episode", episode.id)}
                  >
                    削除
                  </button>
                </div>
              </article>
            ))}
          </div>
          {hasMoreEpisodes ? (
            <button
              type="button"
              className="secondary-button memory-more-button"
              disabled={loading}
              onClick={() => void onLoadMore()}
            >
              もっと見る
            </button>
          ) : null}
        </section>
      </div>
      <div className="settings-actions memory-panel-actions">
        <button type="button" onClick={() => void onRefresh()} disabled={loading}>
          更新
        </button>
        <button
          type="button"
          className="secondary-button"
          onClick={() => void onForgetAll()}
          disabled={loading || !hasMemory}
        >
          すべて忘れる
        </button>
      </div>
    </>
  );
}

type ExtensionSettingsFormProps = {
  extension: InstalledExtension;
  client: YuukeiClient;
  disabled: boolean;
  onResult: (result: ExtensionSettingsChangeResult) => void;
};

type ExtensionUsageSectionProps = {
  usage?: ExtensionCapabilityUsage;
};

function ExtensionUsageSection({ usage }: ExtensionUsageSectionProps) {
  const rows =
    usage?.capabilities.flatMap((capability) =>
      capability.models.map((model) => ({
        capability: capability.capability,
        ...model
      }))
    ) ?? [];
  if (rows.length === 0) {
    return null;
  }

  return (
    <section
      className="extension-usage"
      aria-label={`${usage?.extensionId ?? "extension"} token usage`}
    >
      <h3>トークン使用量</h3>
      <div className="extension-usage-table">
        <div className="extension-usage-row extension-usage-head">
          <span>capability / model</span>
          <span>全期間</span>
          <span>直近7日</span>
        </div>
        {rows.map((row) => (
          <div
            className="extension-usage-row"
            key={`${row.capability}:${row.provider}:${row.model}`}
          >
            <span>
              <strong>{row.capability}</strong>
              <small>
                {row.provider} / {row.model}
              </small>
            </span>
            <TokenUsageTotalsView totals={row.allTime} />
            <TokenUsageTotalsView totals={row.last7Days} />
          </div>
        ))}
      </div>
    </section>
  );
}

type TokenUsageTotalsViewProps = {
  totals: {
    requests: number;
    inputTokens: number;
    outputTokens: number;
  };
};

function TokenUsageTotalsView({ totals }: TokenUsageTotalsViewProps) {
  return (
    <span className="extension-usage-totals">
      <span>リクエスト {formatNumber(totals.requests)}</span>
      <span>入力 {formatNumber(totals.inputTokens)}</span>
      <span>出力 {formatNumber(totals.outputTokens)}</span>
    </span>
  );
}

function ExtensionSettingsForm({
  extension,
  client,
  disabled,
  onResult
}: ExtensionSettingsFormProps) {
  const [draft, setDraft] = useState<Record<string, unknown>>(() =>
    initialSettingDraft(extension)
  );
  const [secretDraft, setSecretDraft] = useState<Record<string, string>>({});
  const [dirtyKeys, setDirtyKeys] = useState<Set<string>>(() => new Set());
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(initialSettingDraft(extension));
    setSecretDraft({});
    setDirtyKeys(new Set());
    setError(null);
  }, [extension.extensionId, extension.settingsSchema, extension.settingValues]);

  const fields = extension.settingsSchema?.fields ?? [];
  const visibleFields = fields.filter((field) => fieldIsVisible(field, draft));

  async function saveSettings() {
    setSaving(true);
    setError(null);
    try {
      const nonSecretValues: Record<string, unknown> = {};
      for (const field of fields) {
        if (field.type === "secret") continue;
        const hasSavedValue = Object.prototype.hasOwnProperty.call(
          extension.settingValues,
          field.key
        );
        if (!hasSavedValue && !dirtyKeys.has(field.key)) continue;
        if (
          hasSavedValue &&
          dirtyKeys.has(field.key) &&
          valuesEqual(draft[field.key], fieldDefaultValue(field))
        ) {
          nonSecretValues[field.key] = null;
        } else {
          nonSecretValues[field.key] = draft[field.key] ?? null;
        }
      }
      let result = await client.setExtensionSettingValues(
        extension.extensionId,
        nonSecretValues
      );
      for (const field of fields) {
        if (field.type !== "secret") continue;
        const value = secretDraft[field.key];
        if (value && value.length > 0) {
          result = await client.setExtensionSecret(
            extension.extensionId,
            field.key,
            value
          );
        }
      }
      setSecretDraft({});
      setDirtyKeys(new Set());
      onResult(result);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  async function clearSecret(key: string) {
    setSaving(true);
    setError(null);
    try {
      const result = await client.setExtensionSecret(
        extension.extensionId,
        key,
        null
      );
      setSecretDraft((current) => ({ ...current, [key]: "" }));
      onResult(result);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section
      className="extension-settings-form"
      aria-label={`${extension.displayName} settings`}
    >
      {visibleFields.map((field) => (
        <ExtensionSettingControl
          key={field.key}
          field={field}
          value={draft[field.key]}
          secretValue={secretDraft[field.key] ?? ""}
          secretSet={extension.secretsSet.includes(field.key)}
          disabled={disabled || saving}
          onValueChange={(value) => {
            setDraft((current) => ({ ...current, [field.key]: value }));
            setDirtyKeys((current) => new Set(current).add(field.key));
          }}
          onSecretChange={(value) =>
            setSecretDraft((current) => ({ ...current, [field.key]: value }))
          }
          onSecretClear={() => clearSecret(field.key)}
        />
      ))}
      {error ? <p className="settings-error">{error}</p> : null}
      <div className="extension-settings-actions">
        <button
          type="button"
          className="secondary-button compact-button"
          disabled={disabled || saving}
          onClick={saveSettings}
        >
          保存
        </button>
      </div>
    </section>
  );
}

type ExtensionSettingControlProps = {
  field: ExtensionSettingField;
  value: unknown;
  secretValue: string;
  secretSet: boolean;
  disabled: boolean;
  onValueChange: (value: unknown) => void;
  onSecretChange: (value: string) => void;
  onSecretClear: () => void;
};

function ExtensionSettingControl({
  field,
  value,
  secretValue,
  secretSet,
  disabled,
  onValueChange,
  onSecretChange,
  onSecretClear
}: ExtensionSettingControlProps) {
  const id = `extension-setting-${field.key.replace(/[^A-Za-z0-9_-]/g, "-")}`;
  return (
    <label className="extension-setting-field" htmlFor={id}>
      <span>
        <strong>{field.label}</strong>
        {"description" in field && field.description ? (
          <small>{field.description}</small>
        ) : null}
      </span>
      {field.type === "string" ? (
        <input
          id={id}
          type="text"
          value={typeof value === "string" ? value : ""}
          disabled={disabled}
          onChange={(event) => onValueChange(event.currentTarget.value)}
        />
      ) : null}
      {field.type === "number" ? (
        <input
          id={id}
          type="number"
          value={typeof value === "number" ? String(value) : ""}
          min={field.min}
          max={field.max}
          disabled={disabled}
          onChange={(event) => {
            const next = event.currentTarget.value;
            onValueChange(next === "" ? null : Number(next));
          }}
        />
      ) : null}
      {field.type === "boolean" ? (
        <input
          id={id}
          type="checkbox"
          checked={Boolean(value)}
          disabled={disabled}
          onChange={(event) => onValueChange(event.currentTarget.checked)}
        />
      ) : null}
      {field.type === "select" ? (
        <select
          id={id}
          value={typeof value === "string" ? value : ""}
          disabled={disabled}
          onChange={(event) => onValueChange(event.currentTarget.value)}
        >
          {field.options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      ) : null}
      {field.type === "secret" ? (
        <span className="extension-secret-control">
          <input
            id={id}
            type="password"
            value={secretValue}
            placeholder={secretSet ? "設定済み" : ""}
            disabled={disabled}
            onChange={(event) => onSecretChange(event.currentTarget.value)}
          />
          {secretSet ? (
            <button
              type="button"
              className="secondary-button compact-button"
              disabled={disabled}
              onClick={onSecretClear}
            >
              クリア
            </button>
          ) : null}
        </span>
      ) : null}
    </label>
  );
}

function initialSettingDraft(extension: InstalledExtension): Record<string, unknown> {
  const draft: Record<string, unknown> = {};
  for (const field of extension.settingsSchema?.fields ?? []) {
    if (field.type === "secret") continue;
    draft[field.key] =
      extension.settingValues[field.key] ?? fieldDefaultValue(field) ?? null;
  }
  return draft;
}

function fieldDefaultValue(field: ExtensionSettingField): unknown {
  if ("default" in field) {
    return field.default;
  }
  return undefined;
}

function valuesEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("ja-JP").format(value);
}

function fieldIsVisible(
  field: ExtensionSettingField,
  values: Record<string, unknown>
): boolean {
  if (!("visibleWhen" in field) || !field.visibleWhen) {
    return true;
  }
  return values[field.visibleWhen.key] === field.visibleWhen.equals;
}

type DaihonDiagnosticsPanelProps = {
  diagnostics: DaihonDiagnosticEntry[];
  expanded: boolean;
  onToggle: () => void;
};

function DaihonDiagnosticsPanel({
  diagnostics,
  expanded,
  onToggle
}: DaihonDiagnosticsPanelProps) {
  if (diagnostics.length === 0) {
    return null;
  }

  const collapsed = diagnostics.length >= 5 && !expanded;
  const visibleDiagnostics = collapsed ? diagnostics.slice(0, 4) : diagnostics;

  return (
    <section className="daihon-diagnostics" aria-label="Daihon errors">
      <div className="daihon-diagnostics-head">
        <h3>Daihon エラー {diagnostics.length}件</h3>
        {diagnostics.length >= 5 ? (
          <button type="button" onClick={onToggle}>
            {expanded ? "折りたたむ" : "すべて表示"}
          </button>
        ) : null}
      </div>
      <ol className="daihon-diagnostic-list">
        {visibleDiagnostics.map((diagnostic, index) => (
          <li
            className={`daihon-diagnostic-row is-${diagnostic.severity}`}
            key={[
              diagnostic.occurredAt,
              diagnostic.code,
              diagnostic.scriptPath,
              diagnostic.line,
              diagnostic.column,
              index
            ].join(":")}
          >
            <div className="daihon-diagnostic-meta">
              <span>{daihonPhaseLabel(diagnostic.phase)}</span>
              <span>{daihonLocationLabel(diagnostic)}</span>
            </div>
            <strong>{diagnostic.message}</strong>
            <small>{diagnostic.code}</small>
            {diagnostic.help ? <p>{diagnostic.help}</p> : null}
            {diagnostic.sourceEventType ? (
              <small>
                {diagnostic.sourceEventType}
                {diagnostic.sourceEventId
                  ? ` / ${diagnostic.sourceEventId}`
                  : ""}
              </small>
            ) : null}
          </li>
        ))}
      </ol>
    </section>
  );
}

function daihonPhaseLabel(phase: DaihonDiagnosticEntry["phase"]): string {
  switch (phase) {
    case "loadParse":
      return "ロード/構文";
    case "loadValidate":
      return "ロード/検証";
    case "loadSpeaker":
      return "ロード/話者";
    case "runtimeValidate":
      return "実行/検証";
    case "runtimeExecute":
      return "実行";
  }
}

function daihonLocationLabel(diagnostic: DaihonDiagnosticEntry): string {
  const path = diagnostic.scriptPath ?? diagnostic.packRoot ?? "unknown";
  if (diagnostic.line && diagnostic.column) {
    return `${path}:${diagnostic.line}:${diagnostic.column}`;
  }
  if (diagnostic.line) {
    return `${path}:${diagnostic.line}`;
  }
  return path;
}

function orderExtensionsForHook(
  extensions: InstalledExtension[],
  orderedIds: string[]
): InstalledExtension[] {
  const byId = new Map(
    extensions.map((extension) => [extension.extensionId, extension])
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

function subscribesToBeforeCommandEmit(extension: InstalledExtension): boolean {
  return extension.hooks.some((hook) => hook.hookPoint === "beforeCommandEmit");
}

function memoryErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (/memory\.|capability|extension|provider/i.test(message)) {
    return "記憶機能が無効です";
  }
  return message;
}

function formatMemoryTimestamp(value: string): string {
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
    minute: "2-digit"
  });
}

type ExtensionPermissionRow = {
  label: string;
  value: string;
  warning?: boolean;
};

function extensionPermissionRows(
  extension: InstalledExtension
): ExtensionPermissionRow[] {
  const rows: ExtensionPermissionRow[] = [];
  const broadEventSubscription =
    extension.permissions.broadEventSubscription ||
    extension.eventSubscriptions.some((subscription) =>
      subscription.eventTypes.some((eventType) => eventType.trim() === "*")
    );

  if (broadEventSubscription) {
    rows.push({
      label: "全イベント購読",
      value: "全イベントを受け取ります",
      warning: true
    });
  }
  if (extension.permissions.eventLogRead) {
    const permission = extension.permissions.eventLogRead;
    rows.push({
      label: "event log読み出し",
      value: `${joinOrAll(permission.eventTypes)} / max ${permission.maxRecords}`
    });
  }
  if (extension.capabilities.length > 0) {
    rows.push({
      label: "capability提供",
      value: extension.capabilities
        .map((capability) => capability.capability)
        .join(", ")
    });
  }
  if (extension.emittedEvents.length > 0) {
    rows.push({
      label: "発行イベント",
      value: extension.emittedEvents.join(", ")
    });
  }

  return rows;
}

function joinOrAll(values: string[]): string {
  return values.length > 0 ? values.join(", ") : "*";
}
