# Yuukei Design Notes

既存MVPの実装を説明する文書ではなく、次の実装者やCodexが「何を作るべきか」を読み取るための思想と境界をまとめます。

Yuukei Coreは、LLMアプリでも、チャットUIでも、デスクトップマスコットでもありません。Coreの責務は、`Daihon`、canonical event log、内部`CapabilityRouter`、Extension実行境界、surface protocolを束ね、UI内生活者が継続して存在するための土台を提供することです。

LLM、長期記憶エンジン、TTS、STT、embedding、画像認識、ローカルAI専用機材連携、message変換、event log購読、RuntimeEvent発行は、公式同梱を含む交換可能なExtensionとして実装します。Yuukei本体は、それらの出力を生活イベントへ接続しますが、特定のAI方式や記憶方式を所有しません。

ExtensionはCore内部状態、Surface実装、event logファイルを直接書き換えず、`RuntimeEvent`、`RuntimeCommand`、`CapabilityInvocation` などの公開契約を入力として受け取り、変換結果や新しいevent提案をResident Homeへ返します。Resident Homeはmanifestの権限宣言を確認し、採用する結果だけをcanonical event logへ記録します。

## Reading Order

1. [01-concept.md](01-concept.md): UI内生活者としての思想と避けるべき方向。
2. [02-architecture.md](02-architecture.md): Resident Home、Device Host、Surface Client、Extensionの完成形。
3. [03-protocols.md](03-protocols.md): 意味境界の間を流れる最小の通信契約。
4. [04-event-log-and-memory.md](04-event-log-and-memory.md): 本体が持つ「記録」と、Extensionが作る「記憶」。
5. [05-world-pack-and-daihon.md](05-world-pack-and-daihon.md): 世界観パック、Daihon、台本とAIの関係。
6. [06-build-guidance-for-codex.md](06-build-guidance-for-codex.md): 新規実装時の判断基準と作る順番。
7. [08-daihon-language-reference.md](08-daihon-language-reference.md): World Pack作者向けのDaihon言語リファレンス。

## User and Author Guides

設計資料ではなく、Yuukeiを使う人・World Pack作者・Extension作者向けの手順を探している場合は、次から始めます。

- [USERGUIDE.md](USERGUIDE.md): インストール、初回設定、住人との触れ合い、AI・音声・プライバシー。
- [docs/user/README.md](docs/user/README.md): 初心者向け作者ガイドの入口と、World Pack / Daihon / Extensionの選び分け。
- [docs/user/01-world-pack-guide.md](docs/user/01-world-pack-guide.md): World Packの作成。
- [docs/user/02-daihon-guide.md](docs/user/02-daihon-guide.md): Daihonのチュートリアル。
- [docs/user/03-extension-guide.md](docs/user/03-extension-guide.md): process Extensionの作成。
- [docs/user/04-testing-and-distribution.md](docs/user/04-testing-and-distribution.md): テスト、配布、トラブルシューティング。

## Non-Negotiable Product Intent

- Yuukeiは、ユーザーのデジタル生活圏に住むUI内生活者を実現する。
- OSのUIは、キャラクターにとっての地形、部屋、道具、外界である。
- ユーザーの通常操作を、生活史の出来事として扱う。
- 台本はキャラクターの核を作り、AIは日常の余白を埋める。
- キャラクター、世界観、台本、声、AI、記憶エンジンは差し替え可能にする。
- Resident Homeはローカルでもクラウドでも動ける。どちらかを唯一の前提にしない。
- Surfaceは身体であり、人格や長期状態を所有しない。
- Extensionは、CoreやWorld Packの所有者にならない。

## Recommended Technical Anchor

最初の実装はRust/Tauri軸でよい。ただし、Resident Home内部はTauri非依存にする。TauriはDevice HostやDesktop Surfaceを実装するための選択肢であり、Coreの境界に染み込ませない。

最小構成では、同一マシン内でResident Home、Device Host、Surface Client、Extension実行プロセスを起動してよい。将来のクラウド構成では、同じprotocolをWebSocketまたはHTTP/JSON-RPC越しに流せるようにする。

## Development Surfaces

開発時の既定SurfaceはCLIである。

- `pnpm dev`: CLI Surfaceを番号メニュー形式で起動する。
- `pnpm dev:cli`: CLI Surfaceを起動する。
- `pnpm dev:tauri`: Tauri版Desktop Surfaceを起動する。
- `pnpm release`: リリース向けのTauri版Surfaceをビルドする。

CLI Surfaceは番号入力の状態機械REPLであり、手動確認にもパイプ入力による機械的テストにも同じ経路で使える(仕様は [03-protocols.md](03-protocols.md) の「CLI Surfaceの番号入力状態機械」)。メニューとプロンプトはstderr、実行結果はstdoutに出る。GUIと同じcanonical signalを同じCore入口へ送るため、GUIで起きた不具合がCLIでも再現すればCore側、しなければSurface側と切り分けられる。

- `printf '4\nこんにちは\n0\n' | cargo run -p yuukei-cli-surface`: `conversation.text` を送り、発行された `RuntimeCommand` を出力する。
- `printf '1\n2\n1\n0\n' | cargo run -p yuukei-cli-surface`: yuukeiの頭を撫でる(`avatar.gesture.poke`。アクターとヒットゾーンの番号はID辞書順)。
- `printf '5\n0\n' | cargo run -p yuukei-cli-surface`: `ResidentSnapshot` を出力する。
- `printf '9\n1\ntarget/events.jsonl\n0\n' | cargo run -p yuukei-cli-surface`: canonical event logをJSONLで書き出す。
- `printf '8\n1\npackages/yuukei-intelligence\n0\n' | cargo run -p yuukei-cli-surface`: ローカルExtensionを `YUUKEI_DATA_DIR/extensions/` へインストールする。
- `YUUKEI_CLI_OUTPUT=jsonl` を付けるとRuntimeCommandを1行1JSONで出力する。presence loop(生活時計)は既定で起動せず、`YUUKEI_CLI_PRESENCE=1` で有効化する。

アプリ動作ログは `YUUKEI_DATA_DIR` が指定されていればその中、未指定ならOSの一時ディレクトリ配下の `yuukei-v2/app-activity.jsonl` に保存する。canonical event logは同じデータディレクトリの `events.sqlite3` に保存する。

## Local Extensions

ローカルExtensionは、設定画面で選んだフォルダを `YUUKEI_DATA_DIR/extensions/<extensionId>/` へコピーしてインストールする。manifestは `YUUKEI_DATA_DIR/extensions/<extensionId>/manifest.json` に置く。

ユーザー所有の有効/無効状態、インストール済みID、hook pointごとの実行順は `YUUKEI_DATA_DIR/settings/extensions.json` に保存する。Device Hostは起動時にこの設定を読み、Resident HomeへExtensionとして登録する。`beforeCommandEmit` では、前のExtensionが返したcommandが次のExtensionの入力になる。設定に残っているが削除済みのIDは無視し、新規インストールしたExtensionは購読しているhook pointの末尾へ追加する。event購読、capability提供、signal alias寄贈はmanifest宣言から登録する。

Extensionは信頼したローカルコードとして実行する。YuukeiはCore内部状態、Tauri AppHandle、Surface実装、event logファイルを直接渡さず、公開protocol messageの入力/出力だけを検証する。manifestのpermissionsは「宣言とユーザー同意」のための境界であり、v1のprocess runtimeではOSレベルのファイルアクセス隔離を約束しない。将来、`runtime: "wasm"` のような軽量runtimeで権限ゼロExtensionを実際にsandbox実行できる余地は残す。

最小例:

```json
{
  "schemaVersion": 1,
  "id": "nya-suffix",
  "displayName": "Nya Suffix",
  "runtime": "process",
  "permissions": {
    "broadEventSubscription": false
  },
  "hooks": [
    {
      "hookPoint": "beforeCommandEmit",
      "commandTypes": ["dialogue.say"]
    }
  ],
  "eventSubscriptions": [
    {
      "eventTypes": ["conversation.*"]
    }
  ],
  "emittedEvents": ["ext.nya-suffix.*"],
  "capabilities": [
    {
      "capability": "speech.synthesis",
      "methods": ["synthesize"]
    }
  ],
  "signalAliases": [
    {
      "alias": "活動時間_開始",
      "signal": "ext.nya-suffix.active-period.start"
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

`onEventAppended` を購読するExtensionは、event logへ追記された `RuntimeEvent` のコピーを受け取り、必要なら `ext.<extensionId>.` で始まる新しい `RuntimeEvent` を提案できる。Resident Homeはsource、causality、hop countを付与し、自己購読とhop上限を検証してからcanonical event logへ追記する。`eventTypes: ["*"]` は広域購読権限として `permissions.broadEventSubscription: true` をmanifestで明示する。

### Official Default Extension: yuukei-intelligence

`packages/yuukei-intelligence` は `dialogue.generate` と `dialogue.interpret` を提供する公式Default Extensionで、Daihonが一致しなかった余白イベントに対する発話案の生成と、Daihon scene内の曖昧な入力の選択肢判定を行う。`printf '8\n1\npackages/yuukei-intelligence\n0\n' | cargo run -p yuukei-cli-surface` でこのフォルダをインストールすると、`YUUKEI_DATA_DIR/extensions/yuukei-intelligence/` へコピーされ、Device Host起動時にmanifestのcapability提供がResident Homeへ登録される。

LM StudioなどのOpenAI互換APIを使う場合は、ローカルサーバーを `http://127.0.0.1:1234/v1` で起動し、必要に応じて `OPENAI_COMPATIBLE_MODEL` またはmanifest内の `config.openaiCompatible.model` を設定する。ChatGPT互換の別endpointを使う場合も `openai-compatible` providerの `baseUrl`、`apiKey`、`model` を差し替えるだけでよい。

Geminiを使う場合は、`YUUKEI_INTELLIGENCE_PROVIDER=gemini` と `GEMINI_API_KEY` を設定する。モデルは既定で `gemini-2.5-flash`、必要なら `GEMINI_MODEL` またはmanifest内の `config.gemini.model` で変更できる。

Extensionをインストールした同じ `YUUKEI_DATA_DIR` で、以下のようにCLIから `conversation.text` を送ると、World Packの `llmDelegation` とDaihon不一致条件を満たした場合だけ `dialogue.generate` が呼ばれる。

```sh
printf '4\nこんにちは\n0\n' | \
  YUUKEI_DATA_DIR=/path/to/yuukei-data \
  OPENAI_COMPATIBLE_MODEL=local-model \
  cargo run -p yuukei-cli-surface
```

`dialogue.interpret` の手動確認には `packs/demo-interpret` を使える。このPackはCLI起動時に「今日はお出かけの日だよね！」と聞き、同じREPLセッション内の会話入力を `はい/いいえ/不明` に解釈してDaihonへ戻す。

```sh
export YUUKEI_DATA_DIR=/tmp/yuukei-interpret-demo
printf '8\n1\npackages/yuukei-intelligence\n0\n' | cargo run -p yuukei-cli-surface
printf '7\n1\npacks/demo-interpret\n0\n' | cargo run -p yuukei-cli-surface
printf '4\nあ〜うん。いやちょっと忙しくて...\n0\n' | \
  OPENAI_COMPATIBLE_BASE_URL=http://127.0.0.1:1234/v1 \
  OPENAI_COMPATIBLE_MODEL=local-model \
  cargo run -p yuukei-cli-surface
printf '7\n2\n0\n0\n' | cargo run -p yuukei-cli-surface
```
