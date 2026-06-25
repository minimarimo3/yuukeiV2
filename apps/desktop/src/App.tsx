import { FormEvent, useEffect, useMemo, useState } from "react";
import type { ResidentSnapshot, RuntimeCommand } from "@yuukei/protocol";
import { tauriYuukeiClient, YuukeiClient } from "./yuukeiClient";

type AppProps = {
  client?: YuukeiClient;
};

export function App({ client = tauriYuukeiClient }: AppProps) {
  const [snapshot, setSnapshot] = useState<ResidentSnapshot | null>(null);
  const [commands, setCommands] = useState<RuntimeCommand[]>([]);
  const [draft, setDraft] = useState("");
  const [status, setStatus] = useState("connecting");

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

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    const emitted = await client.sendConversationText(text);
    setCommands((current) => [...emitted.reverse(), ...current].slice(0, 20));
    setSnapshot(await client.getSnapshot());
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
