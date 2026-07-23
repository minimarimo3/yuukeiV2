import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";

export type StartupErrorInfo = {
  packRoot: string;
  detail: string;
};

type StartupErrorAppProps = {
  loadError?: () => Promise<StartupErrorInfo>;
  quit?: () => Promise<void>;
};

const loadTauriStartupError = () =>
  invoke<StartupErrorInfo>("get_startup_error");
const quitTauriApp = () => getCurrentWindow().close();

export function StartupErrorApp({
  loadError = loadTauriStartupError,
  quit = quitTauriApp,
}: StartupErrorAppProps) {
  const [error, setError] = useState<StartupErrorInfo | null>(null);
  const [loadFailure, setLoadFailure] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void loadError()
      .then((nextError) => {
        if (!disposed) setError(nextError);
      })
      .catch((reason) => {
        if (!disposed) {
          setLoadFailure(
            reason instanceof Error ? reason.message : String(reason),
          );
        }
      });
    return () => {
      disposed = true;
    };
  }, [loadError]);

  return (
    <main className="startup-error-shell">
      <section
        className="startup-error-card"
        aria-labelledby="startup-error-title"
      >
        <div className="startup-error-mark" aria-hidden="true">
          !
        </div>
        <div className="startup-error-copy">
          <p className="startup-error-eyebrow">起動を中止しました</p>
          <h1 id="startup-error-title">Yuukeiを安全に起動できませんでした</h1>
          <p className="startup-error-summary">
            Default World Packを読み込めません。必要なファイルが揃っているか、
            <code>pack.json</code>{" "}
            内の参照先が正しいか確認してから、Yuukeiを再起動してください。
          </p>

          {error ? (
            <dl className="startup-error-details">
              <div>
                <dt>World Pack</dt>
                <dd>{error.packRoot}</dd>
              </div>
              <div>
                <dt>詳細</dt>
                <dd>{error.detail}</dd>
              </div>
            </dl>
          ) : loadFailure ? (
            <p className="startup-error-load-failure" role="alert">
              起動エラーの詳細を取得できませんでした: {loadFailure}
            </p>
          ) : (
            <p className="startup-error-loading" aria-live="polite">
              エラー情報を確認しています…
            </p>
          )}

          <div className="startup-error-actions">
            <button type="button" onClick={() => void quit()}>
              Yuukeiを終了
            </button>
          </div>
        </div>
      </section>
    </main>
  );
}
