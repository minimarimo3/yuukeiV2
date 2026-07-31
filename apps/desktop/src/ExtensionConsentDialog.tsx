import { useEffect, useRef } from "react";
import { extensionPermissionRows } from "./appShared";
import type { ExtensionInstallInspection } from "./yuukeiClient";

export function ExtensionConsentDialog({
  inspection,
  busy,
  error,
  onApprove,
  onCancel,
}: {
  inspection: ExtensionInstallInspection;
  busy: boolean;
  error: string | null;
  onApprove(): void;
  onCancel(): void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const rows = extensionPermissionRows(inspection);

  useEffect(() => {
    const previouslyFocused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    cancelRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || busy) return;
      event.preventDefault();
      onCancel();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      if (previouslyFocused?.isConnected) previouslyFocused.focus();
    };
  }, [busy, onCancel]);

  return (
    <div className="extension-consent-layer">
      <section
        aria-labelledby="extension-consent-title"
        aria-modal="true"
        className="extension-consent-dialog"
        role="dialog"
      >
        <header>
          <p className="settings-eyebrow">Extensionの権限確認</p>
          <h2 id="extension-consent-title">{inspection.displayName}</h2>
          <p className="settings-path">{inspection.extensionId}</p>
        </header>

        <p className="extension-consent-warning">
          {inspection.trustedCodeNotice}
        </p>
        <p>
          許可すると、このmanifestの内容を固定してExtensionをロードします。権限は後から変更できません。変更する場合はExtensionを削除し、もう一度追加してください。
        </p>

        {rows.length > 0 ? (
          <dl className="extension-permissions extension-consent-permissions">
            {rows.map((row) => (
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
        ) : (
          <p className="settings-note">manifest上の追加権限はありません。</p>
        )}

        {error ? <p className="settings-error">{error}</p> : null}
        <div className="extension-consent-actions">
          <button
            className="secondary-button"
            disabled={busy}
            onClick={onCancel}
            ref={cancelRef}
            type="button"
          >
            許可しない
          </button>
          <button disabled={busy} onClick={onApprove} type="button">
            {busy ? "追加中…" : "この内容で許可して追加"}
          </button>
        </div>
      </section>
    </div>
  );
}
