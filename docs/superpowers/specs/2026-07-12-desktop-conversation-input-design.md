# Desktop Conversation Input Design

## Goal

Tauri Desktop Surfaceで、常設チャット画面を作らず、デスクトップ上にいる住人へ直接話しかけられるようにする。

## Interaction

- キャラを右クリックすると、そのキャラの口元付近に一時的な会話入力欄を表示する。
- 入力欄を開いたら自動的にフォーカスする。
- 既定の送信操作は `Ctrl+Enter` とする。日本語IMEの変換確定に使うEnter単独では送信しない。
- 設定画面の「キー設定」で、送信操作を `Ctrl+Enter`、`Enter`、`Shift+Enter` から選択できる。
- 送信操作に割り当てていないEnter系操作は改行として扱う。
- `Escape` または入力欄の外側クリックで閉じる。
- 空白だけの入力は送信しない。
- 送信中は二重送信を防ぎ、成功後に入力欄を閉じる。
- 送信失敗時は入力を保持し、その場に短いエラーを表示する。
- IME composition中のキー操作では送信しない。

## Ownership and Data Flow

- `ActorApp` は右クリックされたactorと口元anchorをDevice Hostへ通知する。
- `DesktopStageManager` は会話入力の表示対象と位置を、該当monitorのStage Overlayへ配信する。
- `StageOverlayApp` は入力UIを描画し、現在のアプリ設定に従って送信キーを判定する。
- 送信には既存の `YuukeiClient.sendConversationText()` を使う。
- 入力文は既存経路で `conversation.text` RuntimeEventとなり、Resident Home、Daihon、CapabilityRouterを通る。
- Surfaceは人格、記憶、台本選択、capability選択を所有しない。
- 現行protocolには会話対象actorの指定がないため、右クリックしたactor IDは入力欄の配置にのみ使う。会話protocolは今回変更しない。

## Settings

- Device Host所有の `settings/app.json` に会話送信キーを保存する。
- 値は `ctrlEnter`、`enter`、`shiftEnter` の列挙として検証する。
- 未設定の既存ファイルは `ctrlEnter` として読む。読み取り時に暗黙の書き戻しは行わない。
- Desktop設定画面へ「キー設定」カテゴリを追加し、選択変更を即時保存する。
- 変更は再起動なしで、次のキー入力から反映する。

## Components

- 会話入力欄は独立したReact componentとし、Stage Overlayの既存吹き出し配置・クリック透過制御と協調する。
- 右クリック検出は既存のpoke、長押しdrag、pointer gesture reducerの意味を変更しない形で追加する。
- 入力欄表示中だけStage Overlayの該当領域をinteractiveにし、透明な残り領域はclick-throughを維持する。

## Error Handling

- 設定値が未知の場合はDevice Hostで拒否する。
- 会話送信に失敗した場合、入力欄を閉じず、再試行できる状態を保つ。
- actorまたはmonitor anchorを解決できない場合は、actorの所属monitor内で安全にclampしたフォールバック位置を使う。

## Testing

- Device Host: 既定値、既存 `app.json` の後方互換、各列挙値の保存、未知値の拒否。
- Settings UI: 「キー設定」の表示、三つの選択肢、変更API呼び出し。
- Conversation input: 右クリックで表示、auto-focus、Escape、外側クリック、空入力、送信中、成功、失敗。
- Keyboard behavior: 各設定値、改行側の動作、IME composition中に送信しないこと。
- Regression: poke、drag、吹き出し選択肢、Stage Overlay click-throughが維持されること。

## Out of Scope

- STTまたはマイクボタン。
- 会話対象actorを指定するprotocol拡張。
- 会話履歴を並べるチャット画面。
- グローバルキーボードショートカット。
