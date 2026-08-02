import { ObservationToggle } from "./ObservationToggle";
import type {
  ObservationSettingsState,
  ObservationSettingsUpdate,
  WorldPackSelectionState,
} from "./yuukeiClient";

export type OnboardingFlowProps = {
  step: number;
  worldPackStatus: WorldPackSelectionState | null;
  worldPackError: string | null;
  switchingPack: boolean;
  onChooseWorldPack: () => void;
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
        {["ようこそ", "会話", "プライバシー", "完了"].map((label, index) => (
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
                選ぶ内容によって、住人の見た目や性格、台詞、暮らし方が変わります。
              </p>
              <p className="settings-title">
                {worldPackStatus?.activeInstall.displayName ?? "読み込み中"}
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
                別の住人と世界を選ぶ
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
              <h2>住人との会話</h2>
              <p className="settings-title">
                住人が自然に話したり、あなたの返事を受け取ったりするためにAIを使います。
              </p>
              <p className="settings-note">
                必要なものはYuukeiに含まれているため、接続先などの難しい設定はありません。会話の内容は外部サービスへ送りません。
              </p>
            </div>
            <div className="settings-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => onStepChange(0)}
              >
                戻る
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
              <h2>パソコン上の変化への気づき</h2>
              <p className="settings-title">
                許可したものだけ、住人がパソコン上の変化に気づけるようになります。あとから変更することもできます。
              </p>
              {observationSettingsError ? (
                <p className="settings-error">{observationSettingsError}</p>
              ) : null}
              <ObservationToggle
                label="使っているアプリに気づく"
                description="アプリ名と、画面を開いた・閉じたことだけを記録します。画面のタイトルや内容は記録しません。"
                checked={observationSettings?.windows ?? false}
                disabled={changingObservationSettings}
                onChange={(checked) => onToggleObservation("windows", checked)}
              />
              <ObservationToggle
                label="開いたフォルダに気づく"
                description="フォルダの種類だけを記録します。フォルダ名や保存場所は記録しません。"
                checked={observationSettings?.folders ?? false}
                disabled={changingObservationSettings}
                onChange={(checked) => onToggleObservation("folders", checked)}
              />
              <ObservationToggle
                label="ダウンロードに気づく"
                description="ファイル名と種類を記録します。保存場所やファイルの中身は記録しません。"
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
