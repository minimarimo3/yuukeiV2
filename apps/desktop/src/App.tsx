import { FormEvent, useEffect, useMemo, useState } from "react";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
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

export function App({ client = tauriYuukeiClient }: AppProps) {
  const [snapshot, setSnapshot] = useState<ResidentSnapshot | null>(null);
  const [commands, setCommands] = useState<RuntimeCommand[]>([]);
  const [draft, setDraft] = useState("");
  const [status, setStatus] = useState("connecting");
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
        unlisteners.push(await client.onCommand((command) => {
          setCommands((current) => [command, ...current].slice(0, 20));
        }));
        unlisteners.push(await client.onSnapshot((nextSnapshot) => {
          setSnapshot(nextSnapshot);
        }));
        const attached = await client.attachSurface();
        if (!disposed) {
          setSnapshot(attached);
          setWorldPackStatus(await client.getWorldPackStatus());
          setExtensionState(await client.getExtensionSettings());
          setStatus("ready");
        }
      } catch (error) {
        if (!disposed) {
          setStatus(error instanceof Error ? error.message : String(error));
        }
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

  const activeActor = useMemo(() => {
    if (!snapshot) return null;
    return Object.values(snapshot.actors)[0] ?? null;
  }, [snapshot]);

  const orderedBeforeCommandEmitExtensions = useMemo(() => {
    return orderExtensionsForHook(
      extensionState?.installed ?? [],
      extensionState?.hookOrder.beforeCommandEmit ?? []
    );
  }, [extensionState]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    const emitted = await client.sendConversationText(text);
    setCommands((current) => [...emitted.reverse(), ...current].slice(0, 20));
    setSnapshot(await client.getSnapshot());
  }

  async function chooseWorldPack() {
    setWorldPackError(null);
    setSwitchingPack(true);
    try {
      const path = await client.openWorldPackDirectory();
      if (!path) return;
      const result = await client.selectWorldPackDirectory(path);
      setWorldPackStatus(result.status);
      setSnapshot(result.snapshot);
      setCommands([]);
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
      setSnapshot(result.snapshot);
      setCommands([]);
      setStatus("ready");
    } catch (error) {
      setWorldPackError(error instanceof Error ? error.message : String(error));
    } finally {
      setSwitchingPack(false);
    }
  }

  function applyExtensionResult(result: ExtensionSettingsChangeResult) {
    setExtensionState(result.state);
    setSnapshot(result.snapshot);
    setCommands([]);
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

  return (
    <main className="surface-shell">
      <section className="resident-pane" aria-label="Resident surface">
        <div className="resident-avatar" aria-hidden="true">
          Y
        </div>
        <div className="resident-state">
          <h1>{activeActor?.displayName ?? "Yuukei"}</h1>
          <p className="resident-meta">
            {snapshot?.worldPackId ?? "loading"} / {status}
          </p>
          <p className="bubble" data-testid="bubble">
            {activeActor?.bubble ?? "…"}
          </p>
        </div>
      </section>

      <section className="settings-panel" aria-label="World Pack settings">
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
      </section>

      <section className="settings-panel extension-panel" aria-label="Extension settings">
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
            {orderedBeforeCommandEmitExtensions.map((extension, index) => (
              <article className="extension-row" key={extension.extensionId}>
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
                {extension.lastLoadError ? (
                  <p className="settings-error">{extension.lastLoadError}</p>
                ) : null}
                <div className="extension-actions">
                  <button
                    type="button"
                    className="secondary-button compact-button"
                    disabled={changingExtensions || index === 0}
                    onClick={() => moveExtension(extension.extensionId, -1)}
                  >
                    上
                  </button>
                  <button
                    type="button"
                    className="secondary-button compact-button"
                    disabled={
                      changingExtensions ||
                      index === orderedBeforeCommandEmitExtensions.length - 1
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
            ))}
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
      </section>

      <form className="input-row" onSubmit={submit}>
        <input
          aria-label="Conversation text"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          placeholder="話しかける"
        />
        <button type="submit">Send</button>
      </form>

      <section className="command-feed" aria-label="Command stream">
        {commands.map((command) => (
          <article key={command.id} className="command-card">
            <strong>{command.type}</strong>
            {command.type === "dialogue.say" ? (
              <p>{String(command.payload.text ?? "")}</p>
            ) : (
              <pre>{JSON.stringify(command.payload, null, 2)}</pre>
            )}
          </article>
        ))}
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
