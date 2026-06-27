# Yuukei Design Notes

既存MVPの実装を説明する文書ではなく、次の実装者やCodexが「何を作るべきか」を読み取るための思想と境界をまとめます。

Yuukei Coreは、LLMアプリでも、チャットUIでも、デスクトップマスコットでもありません。Coreの責務は、`Daihon`、canonical event log、capability router、surface protocolを束ね、UI内生活者が継続して存在するための土台を提供することです。

LLM、長期記憶エンジン、TTS、STT、embedding、画像認識、ローカルAI専用機材連携は、公式同梱を含む交換可能なCapability Providerとして実装します。Yuukei本体は、それらの出力を生活イベントへ接続しますが、特定のAI方式や記憶方式を所有しません。

ユーザーが生活イベントや表示直前の命令へ介入したい場合は、Trusted Hook Extensionとしてprotocol messageを変換します。Hook ExtensionはCore内部状態、Surface実装、event logファイルを直接書き換えず、`RuntimeEvent`、`RuntimeCommand`、`CapabilityInvocation` などの公開契約を入力として受け取り、変換結果をResident Homeへ返します。

## Reading Order

1. [01-concept.md](01-concept.md): UI内生活者としての思想と避けるべき方向。
2. [02-architecture.md](02-architecture.md): Resident Home、Device Host、Surface Client、Capability Providerの完成形。
3. [03-protocols.md](03-protocols.md): 意味境界の間を流れる最小の通信契約。
4. [04-event-log-and-memory.md](04-event-log-and-memory.md): 本体が持つ「記録」と、Extensionが作る「記憶」。
5. [05-world-pack-and-daihon.md](05-world-pack-and-daihon.md): 世界観パック、Daihon、台本とAIの関係。
6. [06-build-guidance-for-codex.md](06-build-guidance-for-codex.md): 新規実装時の判断基準と作る順番。

## Non-Negotiable Product Intent

- Yuukeiは、ユーザーのデジタル生活圏に住むUI内生活者を実現する。
- OSのUIは、キャラクターにとっての地形、部屋、道具、外界である。
- ユーザーの通常操作を、生活史の出来事として扱う。
- 台本はキャラクターの核を作り、AIは日常の余白を埋める。
- キャラクター、世界観、台本、声、AI、記憶エンジンは差し替え可能にする。
- Resident Homeはローカルでもクラウドでも動ける。どちらかを唯一の前提にしない。
- Surfaceは身体であり、人格や長期状態を所有しない。
- Capability ProviderとHook Extensionは、CoreやWorld Packの所有者にならない。

## Recommended Technical Anchor

最初の実装はRust/Tauri軸でよい。ただし、Resident Home内部はTauri非依存にする。TauriはDevice HostやDesktop Surfaceを実装するための選択肢であり、Coreの境界に染み込ませない。

最小構成では、同一マシン内でResident Home、Device Host、Surface Client、Capability Providerを起動してよい。将来のクラウド構成では、同じprotocolをWebSocketまたはHTTP/JSON-RPC越しに流せるようにする。

## Development Surfaces

開発時の既定SurfaceはCLIである。

- `pnpm dev`: CLI Surfaceをウィザード形式で起動する。
- `pnpm dev:cli`: CLI Surfaceを起動する。
- `pnpm dev:tauri`: Tauri版Desktop Surfaceを起動する。
- `pnpm release`: リリース向けのTauri版Surfaceをビルドする。

CLI Surfaceは手動確認だけでなく、機械的なテストに使える非対話モードを持つ。

- `cargo run -p yuukei-cli-surface -- --say "こんにちは"`: `conversation.text` を送り、発行された `RuntimeCommand` をJSONで出力する。
- `cargo run -p yuukei-cli-surface -- --snapshot`: Surface attach後の `ResidentSnapshot` をJSONで出力する。
- `cargo run -p yuukei-cli-surface -- --export-events target/events.jsonl`: canonical event logをJSONLで書き出す。

アプリ動作ログは `YUUKEI_DATA_DIR` が指定されていればその中、未指定ならOSの一時ディレクトリ配下の `yuukei-v2/app-activity.jsonl` に保存する。canonical event logは同じデータディレクトリの `events.sqlite3` に保存する。

## Local Trusted Hook Extensions

ローカルの信頼済みHook Extensionは、設定画面で選んだフォルダを `YUUKEI_DATA_DIR/extensions/<extensionId>/` へコピーしてインストールする。manifestは `YUUKEI_DATA_DIR/extensions/<extensionId>/manifest.json` に置く。

ユーザー所有の有効/無効状態、インストール済みID、hook pointごとの実行順は `YUUKEI_DATA_DIR/settings/extensions.json` に保存する。Device Hostは起動時にこの設定を読み、Resident Homeへ公開protocol hookとして登録する。`beforeCommandEmit` では、前のExtensionが返したcommandが次のExtensionの入力になる。設定に残っているが削除済みのIDは無視し、新規インストールしたExtensionは購読しているhook pointの末尾へ追加する。

Extensionは信頼したローカルコードとして実行する。YuukeiはCore内部状態、Tauri AppHandle、Surface実装、event logファイルを直接渡さず、公開protocol messageの入力/出力だけを検証する。ただしv1ではOSレベルのファイルアクセス隔離を約束しない。

最小例:

```json
{
  "schemaVersion": 1,
  "id": "nya-suffix",
  "displayName": "Nya Suffix",
  "hooks": [
    {
      "hookPoint": "beforeCommandEmit",
      "commandTypes": ["dialogue.say"]
    }
  ],
  "process": {
    "command": "node",
    "args": ["nya-extension.mjs"],
    "timeoutMs": 5000
  }
}
```

外部プロセスはデフォルトでインストール済みExtensionディレクトリをcwdとして起動する。stdinで `ExtensionHookInvocation` を受け取り、stdoutへ `ExtensionHookResult` をJSONで返す。たとえば `dialogue.say` の `payload.text` を変更した `replaceCommand` を返すと、Resident Homeが検証して `extension.hook.result` と変換後commandをevent logへ記録する。
