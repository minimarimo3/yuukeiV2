import { ExtensionSettingsForm } from "./ExtensionSettingsPanel";
import { ObservationToggle } from "./ObservationToggle";
import type {
  ExtensionSettingsChangeResult,
  InstalledExtension,
  ObservationSettingsState,
  ObservationSettingsUpdate,
  WorldPackSelectionState,
  YuukeiClient,
} from "./yuukeiClient";

export type OnboardingFlowProps = {
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
    enabled: boolean,
  ) => void;
  onStepChange: (step: number) => void;
  onDismiss: () => void;
  onComplete: () => void;
};

export function OnboardingFlow({
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
  onComplete,
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
      <section
        className="onboarding-progress"
        aria-label="オンボーディングの進行"
      >
        {["ようこそ", "AI", "観測", "完了"].map((label, index) => (
          <span
            className={[
              "onboarding-progress-step",
              index === clampedStep ? "is-active" : "",
            ]
              .filter(Boolean)
              .join(" ")}
            key={label}
          >
            {label}
          </span>
        ))}
      </section>
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
                onChange={(checked) =>
                  onToggleObservation("downloads", checked)
                }
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
