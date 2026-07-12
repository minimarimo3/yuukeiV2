import { type KeyboardEvent, useState } from "react";
import type { ConversationSendShortcut } from "./yuukeiClient";

type ConversationComposerProps = {
  shortcut: ConversationSendShortcut;
  onSubmit(text: string): Promise<void>;
  onDismiss(): void;
};

export function ConversationComposer({
  shortcut,
  onSubmit,
  onDismiss,
}: ConversationComposerProps) {
  const [text, setText] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    const normalized = text.trim();
    if (!normalized || pending) return;
    setPending(true);
    setError(null);
    try {
      await onSubmit(normalized);
      onDismiss();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setPending(false);
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onDismiss();
      return;
    }
    if (
      event.key !== "Enter" ||
      event.nativeEvent.isComposing ||
      !matchesSendShortcut(event, shortcut)
    ) {
      return;
    }
    event.preventDefault();
    void submit();
  }

  return (
    <form
      className="conversation-composer"
      data-stage-interactive="true"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <label
        className="conversation-composer__label"
        htmlFor="conversation-composer-input"
      >
        住人に話しかける
      </label>
      <textarea
        /* biome-ignore lint/a11y/noAutofocus: コンポーザ表示時に即入力可能にする意図的なフォーカス */
        autoFocus
        id="conversation-composer-input"
        value={text}
        disabled={pending}
        onChange={(event) => setText(event.currentTarget.value)}
        onKeyDown={handleKeyDown}
        rows={2}
      />
      <div className="conversation-composer__footer">
        <small>{shortcutLabel(shortcut)}で送信</small>
        <button type="submit" disabled={pending || !text.trim()}>
          {pending ? "送信中…" : "送信"}
        </button>
      </div>
      {error ? <p role="alert">{error}</p> : null}
    </form>
  );
}

export function matchesSendShortcut(
  event: Pick<
    KeyboardEvent<HTMLTextAreaElement>,
    "ctrlKey" | "shiftKey" | "altKey" | "metaKey"
  >,
  shortcut: ConversationSendShortcut,
): boolean {
  if (event.altKey || event.metaKey) return false;
  if (shortcut === "ctrlEnter") return event.ctrlKey && !event.shiftKey;
  if (shortcut === "shiftEnter") return event.shiftKey && !event.ctrlKey;
  return !event.ctrlKey && !event.shiftKey;
}

function shortcutLabel(shortcut: ConversationSendShortcut): string {
  if (shortcut === "shiftEnter") return "Shift+Enter";
  if (shortcut === "enter") return "Enter";
  return "Ctrl+Enter";
}
