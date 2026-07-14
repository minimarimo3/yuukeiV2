# Extensionを作る

Extensionは、Yuukeiへ交換可能な能力やイベント処理を追加する仕組みです。AI、音声合成、記憶検索、外部サービス連携、セリフ加工、新しいDaihon合図などを、Resident HomeやSurfaceの内部実装へ入り込まずに追加できます。

この章では、現在実装されている`runtime: "process"` ExtensionをNode.jsで作ります。最初にセリフの語尾を変えるhookを作り、そのあとイベント購読、signal alias、capability、設定を説明します。

## 1. World Packとの違い

Extensionを作り始める前に、World Packで解決すべきものではないか確認します。

| World Pack / Daihon向き | Extension向き |
| --- | --- |
| 特定の住人だけが言う決め台詞 | どのPackにも適用できるセリフ加工 |
| キャラクターのVRM・モーション | 音声合成エンジンとの接続 |
| 頭を撫でたときの固有反応 | 外部センサーから新しいeventを提案 |
| 初回起動、再会、外出のscene | LLM、STT、embedding、記憶索引 |
| 世界観上の場所や関係変数 | 公開RuntimeCommandのhook |

Extensionは住人の人格や生活史の所有者ではありません。canonical event logが記録の正本であり、ExtensionのDBやキャッシュは再構築可能な派生物として扱います。

## 2. process Extensionの実行モデル

現在のprocess runtimeは、呼び出しごとに次の流れで動きます。

1. Device Hostがmanifestの`process.command`を新しいプロセスとして起動する。
2. Extensionのインストール先を既定の作業ディレクトリにする。
3. stdinへ1個のJSON objectと改行を書き込む。
4. ExtensionはstdinをEOFまで読み、処理する。
5. stdoutへ結果のJSON objectを1個だけ出す。
6. exit code 0で終了する。
7. Device HostがJSONとmanifest宣言を検証する。

つまり、常駐サーバーとして複数行を読み続ける方式ではありません。1 invocationにつき1 process、1入力、1出力です。将来runtimeが増えても、公開messageの意味は維持されます。

> [!WARNING]
> stdoutは機械が読むJSON専用です。デバッグログを`console.log`でstdoutへ出すと、結果が壊れます。ログは`console.error`でstderrへ出してください。

## 3. 最小の語尾Extensionを作る

次のフォルダを作ります。

```text
gentle-suffix/
├─ manifest.json
└─ index.mjs
```

外部パッケージは使わないため、`package.json`や`npm install`は不要です。

### `manifest.json`

```json
{
  "schemaVersion": 1,
  "id": "gentle-suffix",
  "displayName": "やさしい語尾",
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
  "process": {
    "command": "node",
    "args": ["index.mjs"],
    "timeoutMs": 5000
  }
}
```

### `index.mjs`

```js
const chunks = [];

for await (const chunk of process.stdin) {
  chunks.push(chunk);
}

const invocation = JSON.parse(Buffer.concat(chunks).toString("utf8"));
const command = invocation.command;
const text = command?.payload?.text;

let result;

if (typeof text !== "string" || text.length === 0 || text.endsWith("ね。")) {
  result = {
    action: "unchanged",
    metadata: { reason: "text is empty or already transformed" }
  };
} else {
  const replaced = structuredClone(command);
  replaced.payload.text = `${text} ですね。`;
  result = {
    action: "replaceCommand",
    command: replaced,
    metadata: { appended: "ですね。" }
  };
}

process.stdout.write(JSON.stringify(result));
```

このExtensionは`dialogue.say`だけを受け取り、`payload.text`を書き換えます。すでに同じ語尾なら`unchanged`を返すため、同じ入力に何度適用しても語尾が増えません。この性質を冪等性と呼び、hookで意図しない重複を防ぐのに役立ちます。

## 4. hookの入力と出力

`beforeCommandEmit`の入力は次の形です。

```json
{
  "id": "hook-...",
  "hookPoint": "beforeCommandEmit",
  "extensionId": "gentle-suffix",
  "residentId": "resident-...",
  "worldPackId": "my-first-pack",
  "command": {
    "id": "cmd-...",
    "type": "dialogue.say",
    "timestamp": "2026-07-14T12:00:00Z",
    "source": "daihon",
    "residentId": "resident-...",
    "payload": {
      "text": "おかえりなさい。",
      "speakerId": "alice"
    },
    "target": {
      "actorId": "alice"
    }
  }
}
```

返せるactionは2つです。

### 変更しない

```json
{
  "action": "unchanged",
  "metadata": {
    "reason": "not applicable"
  }
}
```

### commandを置き換える

```json
{
  "action": "replaceCommand",
  "command": {
    "id": "入力と同じcommand ID",
    "type": "dialogue.say",
    "timestamp": "2026-07-14T12:00:00Z",
    "source": "daihon",
    "residentId": "入力と同じresident ID",
    "payload": {
      "text": "加工した文"
    }
  },
  "metadata": {
    "rule": "suffix-v1"
  }
}
```

replacementでは、次の3フィールドを変えられません。

- `command.id`
- `command.type`
- `command.residentId`

これらはcommandの同一性と生活史の因果関係を保つための固定フィールドです。変えるとResident Homeが結果を拒否し、元のcommandを使います。

`metadata`は任意です。個人情報や巨大なデータを入れず、適用ルール、バージョン、変更理由など診断に役立つ最小情報だけにします。hook結果と採用後commandはcanonical event logへ記録されます。

### 複数hookの順序

同じhook pointに複数Extensionがある場合、ユーザーが設定した順序で実行されます。前のExtensionが返したcommandが、次のExtensionの入力です。

manifestにpriorityはありません。作者が他Extensionより先・後を強制しない設計です。順序が変わっても破綻しない加工を目指してください。

## 5. ローカルで単体確認する

次のような`sample-hook-input.json`を作ります。

```json
{
  "id": "hook-test",
  "hookPoint": "beforeCommandEmit",
  "extensionId": "gentle-suffix",
  "residentId": "resident-test",
  "worldPackId": "pack-test",
  "command": {
    "id": "cmd-test",
    "type": "dialogue.say",
    "timestamp": "2026-07-14T12:00:00Z",
    "source": "daihon",
    "residentId": "resident-test",
    "payload": {
      "text": "おかえりなさい。",
      "speakerId": "alice"
    },
    "target": {
      "actorId": "alice"
    }
  }
}
```

PowerShell:

```powershell
Get-Content -Raw sample-hook-input.json | node index.mjs
```

bash / zsh:

```sh
node index.mjs < sample-hook-input.json
```

出力をJSONとして再解析できることも確認します。

PowerShell:

```powershell
$result = Get-Content -Raw sample-hook-input.json | node index.mjs | ConvertFrom-Json
$result.action
$result.command.payload.text
```

stdoutへログが混ざる、JSONの末尾に余計な文字がある、exit codeが0でない、といった問題をYuukeiへ入れる前に見つけられます。

## 6. Yuukeiへインストールする

### デスクトップ版

設定画面のExtension欄からフォルダを選びます。Yuukeiはフォルダを次へコピーします。

```text
YUUKEI_DATA_DIR/extensions/gentle-suffix/
```

実行時の既定cwdも、このインストール済みフォルダです。

### CLI

`pnpm dev:cli`を起動し、次を選びます。

1. `8` 拡張機能
2. `1` インストール
3. `gentle-suffix`フォルダの絶対パス
4. 一覧に表示されたExtensionを有効化
5. `4`で話しかけ、加工後セリフを確認

インストールは参照ではなくコピーです。元の作業フォルダを編集しただけでは、インストール済みコピーは更新されません。開発中は変更後に再インストールしてください。

## 7. manifestの全体像

```json
{
  "schemaVersion": 1,
  "id": "example-extension",
  "displayName": "Example Extension",
  "runtime": "process",
  "permissions": {
    "broadEventSubscription": false,
    "eventLogRead": {
      "eventTypes": ["conversation.*"],
      "privacyCategories": [],
      "allowPayloads": true,
      "allowReferences": false,
      "maxRecords": 1000,
      "purpose": "会話から派生索引を再構築するため"
    }
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
  "emittedEvents": ["ext.example-extension.*"],
  "capabilities": [
    {
      "capability": "dialogue.generate",
      "methods": ["generate"],
      "requiredPermissions": []
    }
  ],
  "signalAliases": [
    {
      "alias": "例_検出",
      "signal": "ext.example-extension.detected"
    }
  ],
  "settings": {
    "fields": []
  },
  "process": {
    "command": "node",
    "args": ["index.mjs"],
    "timeoutMs": 5000
  }
}
```

すべてを宣言する必要はありません。ただし、hook、event購読、event発行、capability、signal aliasの少なくとも1つが必要です。使わない配列は省略できます。

### 基本フィールド

| フィールド | 必須 | 説明 |
| --- | --- | --- |
| `schemaVersion` | はい | 現在は`1` |
| `id` | はい | ASCII英数字、`-`、`_`、`.`。`.`と`..`単体は不可 |
| `displayName` | はい | ユーザーへ見せる名前 |
| `runtime` | 推奨 | process manifestでは`"process"`のみ |
| `permissions` | いいえ | 権限宣言。省略時は狭い既定値 |
| `hooks` | いいえ | RuntimeCommandの購読 |
| `eventSubscriptions` | いいえ | canonical event log追記後のevent購読 |
| `emittedEvents` | いいえ | 提案できるExtension eventのパターン |
| `capabilities` | いいえ | 提供する名前付き能力 |
| `signalAliases` | いいえ | Daihon作者向けの別名 |
| `settings` | いいえ | Device Hostが描画する汎用設定フォーム |
| `process` | はい | 実行コマンド、引数、cwd、timeout |

### `process`

```json
{
  "process": {
    "command": "node",
    "args": ["src/main.mjs"],
    "cwd": ".",
    "timeoutMs": 5000
  }
}
```

- `command`: 実行ファイル。`node`、同梱した実行ファイルなど。
- `args`: コマンドへ渡す固定引数。
- `cwd`: 任意。省略時はインストール済みExtensionフォルダ。
- `timeoutMs`: 任意。省略時5000ms。

相対`command`がExtension内のファイルを指す場合、Device Hostはインストール先を基準に解決します。環境依存の絶対パスは配布に向きません。

## 8. eventを購読する

`onEventAppended`は、canonical event logへ追記されたRuntimeEventのコピーを受け取ります。manifestではevent typeのfilterだけを宣言します。

```json
{
  "eventSubscriptions": [
    {
      "eventTypes": [
        "conversation.text",
        "desktop.*"
      ]
    }
  ]
}
```

patternは次を使えます。

| pattern | 一致 |
| --- | --- |
| `conversation.text` | 完全一致 |
| `conversation.*` | `conversation.`で始まるtype |
| `*` | すべてのevent |

`*`は実質的に非常に広い観測となるため、`permissions.broadEventSubscription: true`が必須です。必要なtypeだけを購読してください。

入力は次の形です。

```json
{
  "id": "ext_evt-...",
  "extensionId": "greeting-watcher",
  "residentId": "resident-...",
  "worldPackId": "my-first-pack",
  "event": {
    "sequence": 42,
    "id": "evt-...",
    "type": "conversation.text",
    "timestamp": "2026-07-14T12:00:00Z",
    "residentId": "resident-...",
    "source": "user",
    "payload": {
      "text": "こんにちは"
    }
  }
}
```

何も提案しない結果:

```json
{
  "proposedEvents": [],
  "metadata": {
    "matched": false
  }
}
```

event購読は通知であり、元eventを変更・取り消しできません。

## 9. 新しいeventとDaihon別名を提供する

たとえば、会話文に挨拶が含まれたとき`ext.greeting-watcher.detected`を提案するExtensionを考えます。

### manifest

```json
{
  "schemaVersion": 1,
  "id": "greeting-watcher",
  "displayName": "挨拶検出",
  "runtime": "process",
  "permissions": {
    "broadEventSubscription": false
  },
  "eventSubscriptions": [
    {
      "eventTypes": ["conversation.text"]
    }
  ],
  "emittedEvents": ["ext.greeting-watcher.detected"],
  "signalAliases": [
    {
      "alias": "挨拶_検出",
      "signal": "ext.greeting-watcher.detected"
    }
  ],
  "process": {
    "command": "node",
    "args": ["index.mjs"],
    "timeoutMs": 5000
  }
}
```

### `index.mjs`

```js
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const invocation = JSON.parse(Buffer.concat(chunks).toString("utf8"));

const text = invocation.event?.payload?.text;
const matched =
  typeof text === "string" &&
  ["こんにちは", "おはよう", "こんばんは"].some((word) => text.includes(word));

const proposedEvents = matched
  ? [
      {
        id: "proposal",
        type: "ext.greeting-watcher.detected",
        timestamp: new Date().toISOString(),
        source: "extension",
        residentId: invocation.residentId,
        payload: {
          greetingText: text
        }
      }
    ]
  : [];

process.stdout.write(
  JSON.stringify({
    proposedEvents,
    metadata: { matched }
  })
);
```

Resident Homeは提案をそのまま盲目的に保存しません。採用時に次を検証・正規化します。

- typeが`ext.<extensionId>.`で始まること。
- typeが`emittedEvents`に一致すること。
- `id`と`timestamp`をResident Homeが新しく付ける。
- `source`を`extension`へ上書きする。
- `residentId`、device、surface、actorを元eventに結び付ける。
- causalityとhop countを付ける。
- 自分が発行したeventは自分へ再通知しない。
- hop上限を超える連鎖を拒否する。

Extensionが`conversation.text`や`device.wake`を偽造することはできません。組み込み語彙ではなく、自分のnamespaceへ新しい語彙を作ります。

### World Packから使う

World Packのallowlistへcanonical IDを追加します。

```json
{
  "signals": {
    "allow": ["ext.greeting-watcher.detected"]
  }
}
```

Daihonでは寄贈aliasを使えます。

```daihon
## 挨拶_検出

### 挨拶に気づく
合図: ＠挨拶_検出
話者: ア
「きちんと挨拶してくれると、部屋が明るくなりますね。」
```

Extensionが未導入または無効ならaliasは解決されず、そのsceneが発火しないだけです。World Packには、Extension固有eventがなくても通常の暮らしが続くようにしてください。特定Extensionが本当に必須なら、Packの配布READMEで明示します。

## 10. capabilityを提供する

capabilityは、`dialogue.generate`、`speech.synthesis`、`memory.retrieve`のような名前付き能力です。Extension同士が直接呼び合うのではなく、Resident HomeのCapabilityRouterが選択されたproviderへ依頼します。

manifest:

```json
{
  "capabilities": [
    {
      "capability": "dialogue.generate",
      "methods": ["generate"]
    }
  ]
}
```

入力は次の共通envelopeです。

```json
{
  "id": "cap-...",
  "capability": "dialogue.generate",
  "method": "generate",
  "residentId": "resident-...",
  "actorId": "alice",
  "input": {
    "event": {
      "type": "conversation.text",
      "payload": {
        "text": "こんにちは"
      }
    },
    "persona": {
      "actorId": "alice",
      "displayName": "アリス",
      "profile": {
        "speechStyle": "短く穏やか"
      }
    },
    "recentContext": [],
    "constraints": {
      "maxLength": 120
    }
  }
}
```

`dialogue.generate`の結果例:

```json
{
  "invocationId": "cap-...",
  "extensionId": "simple-dialogue",
  "capability": "dialogue.generate",
  "output": {
    "speak": true,
    "text": "こんにちは。今日は窓が静かですね。",
    "expression": "smile",
    "motion": "idle"
  },
  "metadata": {
    "model": "rule-based-demo",
    "usage": {
      "inputTokens": 0,
      "outputTokens": 0
    }
  }
}
```

共通の必須フィールドは次です。

- `invocationId`: 入力の`id`と同じ値。
- `extensionId`: 自分のmanifest ID。
- `capability`: 入力のcapabilityと同じ値。
- `output`: capability固有のJSON object。
- `metadata`: 任意情報。空objectでもよい。

`dialogue.generate`では、話さない判断も正当です。

```json
{
  "output": {
    "speak": false
  }
}
```

capabilityごとのinput/output契約を守ってください。名前が同じでも独自形式を返すと、Resident Homeが利用できません。現在の主な契約は [Protocols](../../03-protocols.md) と、公式実装 [yuukei-intelligence](../../packages/yuukei-intelligence/)・[yuukei-voicevox](../../packages/yuukei-voicevox/) を参照してください。

### 1つのprocessで複数種類を扱う

同じmanifestでhook、event、capabilityを宣言した場合、入力の形で分岐します。

```js
if (invocation.hookPoint && invocation.command) {
  // ExtensionHookInvocation
} else if (invocation.event) {
  // ExtensionEventInvocation
} else if (invocation.capability && invocation.method) {
  // CapabilityInvocation
} else {
  console.error("unknown invocation shape");
  process.exit(1);
}
```

入力には現時点で共通の`invocationKind` discriminatorがないため、必要フィールドの存在を確認します。分岐後も、想定外のmethodやcapabilityは明確に失敗させます。

## 11. 設定画面を宣言する

Extension自身が独自のWeb UIを持つのではなく、manifestのschemaからDevice Hostが汎用フォームを描画します。

```json
{
  "settings": {
    "fields": [
      {
        "key": "provider",
        "type": "select",
        "label": "プロバイダ",
        "options": [
          { "value": "local", "label": "ローカル" },
          { "value": "cloud", "label": "クラウド" }
        ],
        "default": "local"
      },
      {
        "key": "endpoint",
        "type": "string",
        "label": "接続先URL",
        "default": "http://127.0.0.1:8080"
      },
      {
        "key": "timeoutMs",
        "type": "number",
        "label": "タイムアウト(ms)",
        "default": 5000,
        "min": 1000,
        "max": 60000
      },
      {
        "key": "enabledFeature",
        "type": "boolean",
        "label": "追加機能を使う",
        "default": true
      },
      {
        "key": "apiKey",
        "type": "secret",
        "label": "APIキー",
        "visibleWhen": {
          "key": "provider",
          "equals": "cloud"
        }
      }
    ]
  }
}
```

使えるtypeは次の5つです。

| type | 値 | 主な追加項目 |
| --- | --- | --- |
| `string` | 文字列 | `default` |
| `number` | 数値 | `default`、`min`、`max` |
| `boolean` | 真偽値 | `default` |
| `select` | 宣言済み文字列 | `options`、`default` |
| `secret` | 秘密文字列 | default不可 |

`key`はExtension内で一意にし、ASCII英数字、`_`、`-`、`.`だけを使います。`provider.apiKey`のようなドット区切りも、渡されるJSONではflat keyのままです。

```json
{
  "provider": "cloud",
  "provider.apiKey": "secret-value"
}
```

`visibleWhen`は、同じschemaに存在する1つのkeyとの等値比較だけです。任意式や動的optionsは使えません。

### processから設定を読む

ユーザーが保存した設定は、環境変数`YUUKEI_EXTENSION_SETTINGS_JSON`へflat JSON objectとして渡されます。

```js
function readSettings() {
  const raw = process.env.YUUKEI_EXTENSION_SETTINGS_JSON;
  if (!raw) return {};

  try {
    return JSON.parse(raw);
  } catch (error) {
    console.error("invalid YUUKEI_EXTENSION_SETTINGS_JSON", error);
    return {};
  }
}

const settings = readSettings();
const timeoutMs = settings.timeoutMs ?? 5000;
```

schemaの`default`は設定画面表示用です。ユーザーが明示保存していないdefaultが、必ず環境変数へ焼き込まれるとは考えないでください。Extensionコードにも安全なfallbackを持たせます。

secret本文は通常の設定stateやAPI応答には出ず、別ファイルへ保存されます。Extension processへは他の保存済み値と同じ環境変数内で渡されるため、ログ、metadata、stdout、エラーメッセージへ出さないでください。

## 12. Extension用データを保存する

Device Hostは、process Extensionへ次の環境変数を渡せます。

```text
YUUKEI_EXTENSION_DATA_DIR
```

これはインストール済みコードとは別の、Extension固有データ領域です。キャッシュ、派生索引、記憶DBなどを置けます。

```js
const dataDir = process.env.YUUKEI_EXTENSION_DATA_DIR;
```

次の原則を守ります。

- canonical event logを直接開いたり書き換えたりしない。
- resident IDとWorld Pack IDを混ぜない。
- event logから再構築できる派生物として設計する。
- 一時ファイルの途中状態を正本にしない。
- 削除・再構築ができるようにする。

Memory Extensionは、manifestで許可範囲を宣言し、Resident Homeから渡されたevent抜粋やmemory capability契約を使います。イベントログのSQLiteファイルパスを探して直接読む方式は避けてください。

## 13. 権限を宣言する

### 広域event購読

```json
{
  "permissions": {
    "broadEventSubscription": true
  },
  "eventSubscriptions": [
    { "eventTypes": ["*"] }
  ]
}
```

本当に全eventが必要な場合だけ使います。通常は`conversation.*`などへ絞ります。

### event log読み出し

```json
{
  "permissions": {
    "broadEventSubscription": false,
    "eventLogRead": {
      "eventTypes": [
        "conversation.text",
        "dialogue.say"
      ],
      "privacyCategories": [],
      "allowPayloads": true,
      "allowReferences": false,
      "maxRecords": 1000,
      "purpose": "日ごとの会話要約索引を再構築するため"
    }
  }
}
```

`purpose`はユーザーが判断できる具体的な説明にします。「機能のため」のような曖昧な文は避けます。宣言したからといってevent logファイルへの直接アクセス権が与えられるわけではありません。Resident Homeがgrantと公開契約に従って、許可された範囲だけを渡します。

> [!IMPORTANT]
> v1のprocess runtimeは、信頼済みローカルコードです。manifestの権限は宣言と同意の境界ですが、OSレベルで任意ファイルアクセスを完全にsandboxするものではありません。作者は必要最小限を宣言し、配布者は実行コードを公開し、ユーザーは信頼できるExtensionだけを導入してください。

## 14. エラーとtimeoutを設計する

process Extensionでは、次が失敗になります。

- プロセスを起動できない。
- timeoutまでに終了しない。
- exit codeが0でない。
- stdoutが有効なJSONではない。
- 結果のschemaが違う。
- hookで固定フィールドを変える。
- 提案eventがnamespaceやmanifest宣言に違反する。

hookが失敗した場合、Resident Homeは元commandを維持して処理を続けます。capability失敗時も、Daihonやテキスト表示が可能な範囲でfallbackします。

同種の失敗が繰り返されると、Extensionは一時休止され、ユーザーへ通知されます。設定画面から再起動できます。したがって、外部APIが落ちているだけの状態を毎回crashにせず、capability契約で正常な「結果なし」を返せる場合はそうします。

例: `dialogue.generate`が応答できない場合

```json
{
  "invocationId": "cap-...",
  "extensionId": "my-dialogue",
  "capability": "dialogue.generate",
  "output": {
    "speak": false
  },
  "metadata": {
    "reason": "provider unavailable"
  }
}
```

秘密値、リクエスト全文、会話全文をエラーへ入れないでください。stderrもアプリ動作ログや開発環境から見える可能性があります。

## 15. Extension設計のチェックリスト

### manifest

- [ ] `schemaVersion`は`1`か。
- [ ] IDは安定したASCII名か。
- [ ] hook、購読、発行、capability、aliasの少なくとも1つがあるか。
- [ ] event filterは必要最小限か。
- [ ] `*`購読時に`broadEventSubscription: true`があるか。
- [ ] aliasの宛先は`ext.<自分のID>.`で始まるか。
- [ ] aliasの宛先は`emittedEvents`にも含まれるか。
- [ ] timeoutは処理時間に対して妥当か。
- [ ] secretにdefaultを書いていないか。

### process

- [ ] stdinを1 JSONとして読めるか。
- [ ] stdoutは結果JSONだけか。
- [ ] ログはstderrか。
- [ ] 正常時のexit codeは0か。
- [ ] 想定外inputを明確に処理するか。
- [ ] hookは固定フィールドを保持するか。
- [ ] 同じhookが複数回適用されても暴走しないか。
- [ ] 設定が未保存でも安全なdefaultがあるか。
- [ ] API keyや会話全文をログへ出していないか。

### アーキテクチャ

- [ ] Extension同士を直接呼んでいないか。
- [ ] SurfaceやTauri APIを直接操作していないか。
- [ ] canonical event logファイルを直接変更していないか。
- [ ] 組み込みeventを偽造せず、自分のnamespaceを使っているか。
- [ ] 内部DBを消してもログから再構築できるか。
- [ ] World Pack固有の人格をExtensionへ固定していないか。

配布と統合テストは [テスト・配布・トラブルシューティング](04-testing-and-distribution.md) を参照してください。
