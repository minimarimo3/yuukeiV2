# Yuukei Design

この文書群は、Yuukeiを新規に実装する際のプロダクト思想、責務、通信境界を定義します。既存MVPの実装構造は対象としません。

Yuukei Coreは、LLMアプリでも、チャットUIでも、デスクトップマスコットでもありません。Coreの責務は、`Daihon`、canonical event log、内部`CapabilityRouter`、Extension実行境界、surface protocolを束ね、UI内生活者が継続して存在するための土台を提供することです。

LLM、長期記憶エンジン、TTS、STT、embedding、画像認識、ローカルAI専用機材連携、message変換、event log購読、RuntimeEvent発行は、公式同梱を含む交換可能なExtensionとして実装します。Yuukei本体は、それらの出力を生活イベントへ接続しますが、特定のAI方式や記憶方式を所有しません。

ExtensionはCore内部状態、Surface実装、event logファイルを直接書き換えず、`RuntimeEvent`、`RuntimeCommand`、`CapabilityInvocation` などの公開契約を入力として受け取り、変換結果や新しいevent提案をResident Homeへ返します。Resident Homeはmanifestの権限宣言を確認し、採用する結果だけをcanonical event logへ記録します。

## Reading Order

1. [00-experience-contract.md](00-experience-contract.md): 実装判断と完成判定に優先する体験契約、Golden Scene、受け入れ条件。
2. [01-concept.md](01-concept.md): UI内生活者としての思想と避けるべき方向。
3. [02-architecture.md](02-architecture.md): Resident Home、Device Host、Surface Client、Extensionの完成形。
4. [03-protocols.md](03-protocols.md): 意味境界の間を流れる最小の通信契約。
5. [04-event-log-and-memory.md](04-event-log-and-memory.md): 本体が持つ「記録」と、Extensionが作る「記憶」。
6. [05-world-pack-and-daihon.md](05-world-pack-and-daihon.md): 世界観パック、Daihon、台本とAIの関係。
7. [06-build-guidance-for-codex.md](06-build-guidance-for-codex.md): 新規実装時の判断基準と作る順番。
8. [08-daihon-language-reference.md](08-daihon-language-reference.md): World Pack作者向けのDaihon言語リファレンス。

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

## Default Technical Anchor

初期実装の既定技術はRust/Tauriとする。ただし、Resident Home内部はTauri非依存とする。TauriはDevice HostやDesktop Surfaceの実装基盤であり、Coreの境界には持ち込まない。

最小構成では、Resident Home、Device Host、Surface Client、Extension実行プロセスを同一マシン上で起動する。クラウド構成でも同じprotocolを使用し、WebSocketまたはHTTP/JSON-RPC越しに通信できるようにする。

## Development Surfaces

開発時の既定SurfaceはCLIである。

- `pnpm dev`: CLI Surfaceを番号メニュー形式で起動する。
- `pnpm dev:cli`: CLI Surfaceを起動する。
- `pnpm dev:tauri`: Tauri版Desktop Surfaceを起動する。
- `pnpm release`: リリース向けのTauri版Surfaceをビルドする。

CLI Surfaceは番号入力の状態機械REPLであり、手動確認にもパイプ入力による機械的テストにも同じ経路で使える(仕様は [03-protocols.md](03-protocols.md) の「CLI Surfaceの番号入力状態機械」)。メニューとプロンプトはstderr、実行結果はstdoutに出る。GUIと同じcanonical signalを同じCore入口へ送るため、GUIで起きた不具合がCLIでも再現すればCore側、しなければSurface側と切り分けられる。

- `printf '1\n2\n1\n0\n' | cargo run -p yuukei-cli-surface`: yuukeiの頭を撫でる(`avatar.gesture.poke`。アクターとヒットゾーンの番号はID辞書順)。
- `printf '5\n0\n' | cargo run -p yuukei-cli-surface`: `ResidentSnapshot` を出力する。
- `printf '9\n1\ntarget/events.jsonl\n0\n' | cargo run -p yuukei-cli-surface`: canonical event logをJSONLで書き出す。
- `printf '8\n1\npackages/yuukei-intelligence/dist/yuukei-intelligence\nyes\n0\n' | cargo run -p yuukei-cli-surface`: 権限表示へ`yes`で同意し、配布用に生成済みのローカルIntelligence Extensionを `YUUKEI_DATA_DIR/extensions/` へインストールする。
- `YUUKEI_CLI_OUTPUT=jsonl` を付けるとRuntimeCommandを1行1JSONで出力する。presence loop(生活時計)は既定で起動せず、`YUUKEI_CLI_PRESENCE=1` で有効化する。

アプリ動作ログは `YUUKEI_DATA_DIR` が指定されていればその中、未指定ならOSの一時ディレクトリ配下の `yuukei-v2/app-activity.jsonl` に保存する。canonical event logは同じデータディレクトリの `events.sqlite3` に保存する。

## Local Extensions

ローカルExtensionは、設定画面でフォルダを選ぶと、まずmanifestの権限と信頼済みコード実行に関する確認を表示する。許可しなければロードもコピーも行わない。許可した場合だけ `YUUKEI_DATA_DIR/extensions/<extensionId>/` へコピーしてインストールし、承認したmanifestのdigestと権限を保存する。以後manifestまたは権限が変わったExtensionはロードを拒否するため、変更する場合は一度削除して、内容を再確認してから追加し直す。manifestは `YUUKEI_DATA_DIR/extensions/<extensionId>/manifest.json` に置く。

ユーザー所有の有効/無効状態、インストール済みID、hook pointごとの実行順は `YUUKEI_DATA_DIR/settings/extensions.json` に保存する。Device Hostは起動時にこの設定を読み、Resident HomeへExtensionとして登録する。`beforeCommandEmit` では、前のExtensionが返したcommandが次のExtensionの入力になる。設定に残っているが削除済みのIDは無視し、新規インストールしたExtensionは購読しているhook pointの末尾へ追加する。event購読、capability提供、signal alias寄贈はmanifest宣言から登録する。

Extensionは信頼したローカルコードとして実行する。YuukeiはCore内部状態、Tauri AppHandle、Surface実装、event logファイルを直接渡さず、公開protocol messageの入力/出力だけを検証する。manifestのpermissionsは追加時に一度だけユーザーが許可する固定契約であり、通常設定から後付けで変更できない。v1のprocess runtimeではOSレベルのファイルアクセス隔離を約束しない。将来、`runtime: "wasm"` のような軽量runtimeで権限ゼロExtensionを実際にsandbox実行できる余地は残す。

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

`packages/yuukei-intelligence` はRust製の公式Default Extensionで、`dialogue.generate`、`dialogue.interpret`、`dialogue.extract` などを提供する。Daihonが明示した生成指示によるセリフ補完と、Daihon場面内で受け取った曖昧な入力の分類・抽出が役割であり、常設チャットや汎用アシスタントには使わない。

実行時にNode.js、Python、外部AI API、モデル設定画面は必要ない。配布パッケージにはLiteRT-LMのWindowsランタイムと `gemma-4-E4B-it.litertlm` を同梱し、Extensionプロセスを常駐させてモデルを一度だけロードする。

```powershell
pnpm --dir packages/yuukei-intelligence package:windows
```

生成物は `packages/yuukei-intelligence/dist/yuukei-intelligence/` に置かれる。開発時だけ、パッケージスクリプトの `-ModelPath` または `YUUKEI_INTELLIGENCE_MODEL_SOURCE` で入力モデルを変更できる。インストール後の利用者向けモデル選択設定は設けない。
