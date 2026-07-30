import { useCallback, useEffect, useRef } from "react";
import type { StageOwnedOverlay } from "./yuukeiClient";

export type OwnedOverlayDismissReason = "user-dismissed" | "expired";

export function OwnedOverlay({
  overlay,
  onDismiss,
}: {
  overlay: StageOwnedOverlay;
  onDismiss(reason: OwnedOverlayDismissReason): void | Promise<void>;
}) {
  const dismissPending = useRef(false);
  const onDismissRef = useRef(onDismiss);
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  const requestDismiss = useCallback((reason: OwnedOverlayDismissReason) => {
    if (dismissPending.current) return;
    dismissPending.current = true;
    let result: void | Promise<void>;
    try {
      result = onDismissRef.current(reason);
    } catch {
      dismissPending.current = false;
      return;
    }
    void Promise.resolve(result).catch(() => {
      // The host keeps the overlay when its runtime dispatch fails. Release the
      // UI-side guard as well so close, Escape, or a later expiry retry can try again.
      dismissPending.current = false;
    });
  }, []);

  useEffect(() => {
    onDismissRef.current = onDismiss;
  }, [onDismiss]);

  useEffect(() => {
    dismissPending.current = false;
    const remaining = Math.max(
      overlay.createdAtMs + overlay.durationMs - Date.now(),
      0,
    );
    const timer = window.setTimeout(() => {
      requestDismiss("expired");
    }, remaining);
    return () => window.clearTimeout(timer);
  }, [overlay.createdAtMs, overlay.durationMs, requestDismiss]);

  useEffect(() => {
    const previouslyFocused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    closeButtonRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      requestDismiss("user-dismissed");
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      if (previouslyFocused?.isConnected) {
        previouslyFocused.focus();
      }
    };
  }, [requestDismiss]);

  return (
    <div className="owned-overlay-layer" data-overlay-style={overlay.style}>
      <section
        aria-labelledby={`owned-overlay-title-${overlay.overlayId}`}
        aria-modal="true"
        className="owned-overlay-card"
        role="dialog"
      >
        <header className="owned-overlay-titlebar">
          <span
            className="owned-overlay-title"
            id={`owned-overlay-title-${overlay.overlayId}`}
          >
            {overlay.title}
          </span>
          <button
            aria-label="ごまかし画面を閉じる"
            className="owned-overlay-close"
            onClick={() => requestDismiss("user-dismissed")}
            ref={closeButtonRef}
            type="button"
          >
            ×
          </button>
        </header>
        <div className="owned-overlay-body">
          <span aria-hidden="true" className="owned-overlay-error-mark">
            ×
          </span>
          <p>{overlay.message}</p>
        </div>
      </section>
    </div>
  );
}
