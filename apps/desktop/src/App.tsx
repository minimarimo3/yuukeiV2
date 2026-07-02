import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  tauriYuukeiClient,
  type ExtensionSettingsChangeResult,
  type ExtensionSettingsState,
  type InstalledExtension,
  type WorldPackSelectionState,
  type YuukeiClient
} from "./yuukeiClient";

type AppProps = {
  client?: YuukeiClient;
};

type SettingsCategoryId = "worldPack" | "extensions";

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
  const [extensionState, setExtensionState] =
    useState<ExtensionSettingsState | null>(null);
  const [worldPackError, setWorldPackError] = useState<string | null>(null);
  const [extensionError, setExtensionError] = useState<string | null>(null);
  const [switchingPack, setSwitchingPack] = useState(false);
  const [changingExtensions, setChangingExtensions] = useState(false);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function connect() {
      try {
        unlisteners.push(await client.onAssetsChanged(() => {
          void refreshSettings();
        }));
        await refreshSettings();
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
      const [nextWorldPackStatus, nextExtensionState] = await Promise.all([
        client.getWorldPackStatus(),
        client.getExtensionSettings()
      ]);
      if (!disposed) {
        setWorldPackStatus(nextWorldPackStatus);
        setExtensionState(nextExtensionState);
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

  async function chooseWorldPack() {
    setWorldPackError(null);
    setSwitchingPack(true);
    try {
      const path = await client.openWorldPackDirectory();
      if (!path) return;
      const result = await client.selectWorldPackDirectory(path);
      setWorldPackStatus(result.status);
      setStatus("ready");
    } catch (error) {
      setWorldPackError(error instanceof Error ? error.message : String(error));
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
    } catch (error) {
      setWorldPackError(error instanceof Error ? error.message : String(error));
    } finally {
      setSwitchingPack(false);
    }
  }

  function applyExtensionResult(result: ExtensionSettingsChangeResult) {
    setExtensionState(result.state);
    setStatus("ready");
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

  const settingsCategories: SettingsCategory[] = [
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
          </div>
        </>
      )
    }
  ];
  const activeSettingsCategory =
    settingsCategories.find(
      (category) => category.id === activeSettingsCategoryId
    ) ?? settingsCategories[0];

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
