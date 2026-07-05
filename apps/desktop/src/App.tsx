import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { ExtensionSettingField } from "@yuukei/protocol";
import {
  tauriYuukeiClient,
  type DaihonDiagnosticEntry,
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
  const [showAllDaihonDiagnostics, setShowAllDaihonDiagnostics] =
    useState(false);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function connect() {
      try {
        unlisteners.push(await client.onAssetsChanged(() => {
          void refreshSettings();
        }));
        unlisteners.push(
          await client.onWorldPackStatus((nextWorldPackStatus) => {
            if (!disposed) {
              setWorldPackStatus(nextWorldPackStatus);
            }
          })
        );
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
                      {extension.settingsSchema ? (
                        <ExtensionSettingsForm
                          extension={extension}
                          client={client}
                          disabled={changingExtensions}
                          onResult={applyExtensionResult}
                        />
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

type ExtensionSettingsFormProps = {
  extension: InstalledExtension;
  client: YuukeiClient;
  disabled: boolean;
  onResult: (result: ExtensionSettingsChangeResult) => void;
};

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
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(initialSettingDraft(extension));
    setSecretDraft({});
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
        nonSecretValues[field.key] = draft[field.key] ?? null;
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
          onValueChange={(value) =>
            setDraft((current) => ({ ...current, [field.key]: value }))
          }
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
