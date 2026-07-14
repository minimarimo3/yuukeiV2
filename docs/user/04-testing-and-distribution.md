# テスト・配布・トラブルシューティング

この章では、World Pack、Daihon、Extensionを壊れにくく確認し、安全に配布する手順をまとめます。最初からすべてを自動化する必要はありません。まず「入力を1つ起こすと、期待するcommandが1つ出る」という小さな確認を積み重ねます。

## 1. テスト環境を普段使いと分ける

Yuukeiの状態は`YUUKEI_DATA_DIR`へ保存できます。作者用の空ディレクトリを指定してください。

PowerShell:

```powershell
$env:YUUKEI_DATA_DIR = "$PWD/.local/yuukei-authoring"
pnpm dev:cli
```

bash / zsh:

```sh
export YUUKEI_DATA_DIR="$PWD/.local/yuukei-authoring"
pnpm dev:cli
```

主な内容は次です。

```text
YUUKEI_DATA_DIR/
├─ events.sqlite3
├─ app-activity.jsonl
├─ extensions/
│  └─ <extensionId>/
├─ extension-data/
│  └─ <extensionId>/
├─ packs-imported/
└─ settings/
   ├─ app.json
   ├─ extensions.json
   ├─ extension-secrets.json
   └─ stage.json
```

- `events.sqlite3`はcanonical event logです。Extensionから直接編集しないでください。
- `app-activity.jsonl`は起動、attach、入力、エラーなどの実装診断ログです。
- `extensions/`はインストール時にコピーされたコードです。
- `extension-data/`はExtensionの派生データです。
- `settings/`はPack選択、Extension有効状態、hook順序などのユーザー設定です。

テストごとに別ディレクトリを使うと、`一度きり`のscene、頻度履歴、保存変数、Extension有効状態が混ざりません。

## 2. CLI Surfaceを使う

CLIは単なる補助ツールではなく、Desktop Surfaceと同じcanonical signalを同じResident Home入口へ送る正式な開発Surfaceです。

```sh
pnpm dev:cli
```

### 固定メニュー

| 番号 | 操作 | 発生するsignal・結果 |
| --- | --- | --- |
| `1` | 撫でる・つつく | `avatar.gesture.poke` |
| `2` | つまむ | `avatar.gesture.grab` |
| `3` | おろす | `avatar.gesture.drop` |
| `4` | 話しかける | `conversation.text` |
| `5` | 状態を見る | `ResidentSnapshot`を表示 |
| `6` | command履歴 | 発行済みcommandを表示 |
| `7` | World Pack | 選択、リセット、状態表示 |
| `8` | Extension | インストール、有効・無効切替 |
| `9` | ログとパス | event log書き出し、パス表示 |
| `10` | 出力モード | human / JSONL切替 |
| `0` | 戻る・終了 | 現在階層に応じて戻る |

不正な番号を入力すると、同じ状態に留まります。パイプテストで番号がずれたとき、別の操作を誤実行しにくくするためです。

### 自動入力する

PowerShellで会話入力を送る例:

```powershell
@'
4
こんにちは
0
'@ | cargo run -p yuukei-cli-surface
```

bash / zsh:

```sh
printf '4\nこんにちは\n0\n' | cargo run -p yuukei-cli-surface
```

メニューとpromptはstderr、実行結果はstdoutへ出ます。stdoutだけをファイルやJSON parserへ渡せます。

### JSONL出力

```powershell
$env:YUUKEI_CLI_OUTPUT = "jsonl"
@'
4
こんにちは
0
'@ | cargo run -p yuukei-cli-surface
```

RuntimeCommandが1行1 JSONで出ます。テストでは`type`、`payload.text`、`target.actorId`などを確認します。IDとtimestampは実行ごとに変わるため、全文一致ではなく意味フィールドを比較してください。

### presence loop

CLIでは決定性を保つため、生活時計が既定で停止しています。定期sceneを手動確認するときだけ有効化します。

```powershell
$env:YUUKEI_CLI_PRESENCE = "1"
pnpm dev:cli
```

タイマーは実行時刻や揺らぎの影響を受けます。自動テストへ混ぜるより、CoreのテストでRuntimeEventを直接投入する方が安定します。

## 3. World Packの確認順序

### 段階1: manifestだけ

最初はrendererやAIを外し、次だけで読み込みます。

- 1 actor
- 1 script
- `app.startup`と`conversation.text`
- required capabilityなし

ここで失敗するなら、`pack.json`のJSON、ID、script path、speaker aliasを確認します。

### 段階2: Daihon

1つの合図に条件なしsceneを1つだけ置きます。

```daihon
## 会話_入力

### 到達確認
合図: ＠会話_入力
話者: ア
「会話イベントが届きました。」
```

これが動いたあとで、条件、入力、変数、AI関数を1つずつ足します。

### 段階3: renderer

CLIで台本が動いたら、Desktop Surfaceで次を確認します。

- VRMが表示される。
- 輪郭と透明背景が正しい。
- actor IDとwindowが対応する。
- `表情`、`動作`が存在する語彙を使う。
- `歩く`のmotion IDが`renderer.motions`にある。
- 長押し、drop、位置保存が動く。

### 段階4: OS観測

設定画面で観測を個別にONにします。

- ウィンドウ
- フォルダ
- ダウンロード

一度に全部をONにせず、1種類ずつevent logとDaihon反応を確認します。ウィンドウタイトルや生のフォルダパスが台本へ来ることを前提にしないでください。仕様上、最小化・カテゴリ化された入力だけが届きます。

### 段階5: optional capability

最後にAIやTTSを追加します。

1. providerなしでDaihon fallbackを確認。
2. providerを有効化。
3. 正常応答を確認。
4. providerを停止し、timeoutやfallbackを確認。
5. 再度起動し、Pack全体が壊れないことを確認。

## 4. Daihonの診断を読む

Daihonの診断は`E-DHN-*`が実行を妨げるエラー、`W-DHN-*`が動作はするが注意が必要な警告です。

コードは大きく分かれます。

| prefix | 段階 | 例 |
| --- | --- | --- |
| `E-DHN-LEX-*` | 文字の読み取り | `「」`や`＜＞`の閉じ忘れ |
| `E-DHN-SYN-*` | 構文 | 見出し不足、分岐の`おわり`不足 |
| `E-DHN-SEM-*` | 意味検証 | unknown関数、引数型、話者、変数scope |
| `E-DHN-RUN-*` | 実行時 | 未定義変数、型不一致、0除算、jump過多 |
| `W-DHN-*` | 警告 | 廃止metadata、AI呼び出し上限、型違い比較 |

直し方:

1. 最初のerrorを見る。後続errorは最初の閉じ忘れから連鎖することがある。
2. script path、行、列を確認する。
3. `「」`、`＜＞`、`（）`、`おわり`の対応を先に確認する。
4. unknown speakerなら`pack.json`のactor IDとaliasを確認する。
5. unknown functionなら [言語リファレンス](../../08-daihon-language-reference.md) の関数名と引数を確認する。
6. load errorを直したあと、runtime warningを確認する。

デスクトップ版のWorld Pack設定画面には、現在セッションのload・dispatch診断が表示されます。多数の場合は折りたたまれます。次回起動までUIへ保持されないため、再現条件と時刻もメモしてください。

## 5. event logを使って原因を分ける

トップメニュー`9`からcanonical event logをJSONLで書き出せます。

確認する順序:

1. 入力RuntimeEventが記録されているか。
2. `daihon.scene.executed`など、期待sceneの実行記録があるか。
3. RuntimeCommandが記録されているか。
4. Extensionを使う場合、`extension.hook.result`やcapability結果があるか。
5. Desktop Surfaceだけで見えないなら、commandは正しいが表示側に問題がないか。

切り分けの目安:

| CLI | Desktop | 可能性 |
| --- | --- | --- |
| 失敗 | 失敗 | Pack、Daihon、Resident Home、Extension側 |
| 成功 | 失敗 | Desktop Surface、VRM、window、入力判定側 |
| 成功 | 成功 | 再現条件、頻度履歴、データディレクトリ違いを確認 |

event logは生活史の正本です。調査のためにSQLiteを直接書き換えないでください。削除は設定画面の機能を使い、必要なら先にエクスポートします。

## 6. Extensionのテスト

### 1. 単体テスト

Yuukeiへ入れる前に、stdin fixtureを使って確認します。

- 有効なhook input
- 対象外command
- textなしcommand
- event購読input
- capabilityの各method
- 不正JSON
- 設定環境変数なし
- 外部API停止

各ケースで確認すること:

- stdoutが有効なJSON object 1個だけ。
- exit codeが意図どおり。
- stderrへ秘密が出ない。
- timeoutより十分早く終わる。
- 同じ入力で暴走や重複が起きない。

### 2. インストール確認

インストール後、`YUUKEI_DATA_DIR/extensions/<id>/`に必要ファイルがすべてコピーされているか確認します。

よくある不足:

- `node_modules`を前提にしているが同梱していない。
- build前のTypeScriptだけを配布し、実行するJavaScriptがない。
- manifestのargsが開発マシンだけのpathを指す。
- package manager scriptをcommandにしているが、利用環境にpackage managerがない。
- native binaryが対象OS・CPUと合わない。

初心者向けには、依存なしの`.mjs`、または対象OS向けに自己完結した実行ファイルが配布しやすくなります。

### 3. hook統合テスト

1. Extension無効で元セリフを確認。
2. 有効化して加工後セリフを確認。
3. 同じhookの別Extensionも有効化。
4. 順序を入れ替える。
5. 対象外commandが変わらないことを確認。
6. Extensionを意図的に失敗させ、元commandが残ることを確認。

### 4. event統合テスト

1. 元eventが記録される。
2. Extension invocationが起きる。
3. proposed eventが`ext.<id>.`名前空間になる。
4. 正規化後eventにcausalityとhop metadataがある。
5. World PackのallowlistとDaihon aliasが一致する。
6. Extension無効時はsceneが発火しないだけで、Packが壊れない。

### 5. capability統合テスト

1. manifestのcapabilityとmethodが正しい。
2. `invocationId`、`extensionId`、`capability`をそのまま返す。
3. capability固有outputの必須値がある。
4. 「結果なし」が正常結果として表せる。
5. provider timeout時にPack側fallbackが出る。
6. metadataに秘密や全文を入れていない。

## 7. リポジトリの検証コマンド

Yuukei本体と一緒に開発している場合は、変更範囲に応じて確認します。

```sh
cargo test -p yuukei-daihon
cargo test -p yuukei-world
cargo test -p yuukei-extension
cargo test -p yuukei-device-host
```

全体確認:

```sh
pnpm check
```

`pnpm check`はformat、lint、Rust check、TypeScript typecheck、testをまとめて実行します。ドキュメントだけの変更では全バイナリ動作まで保証しませんが、manifest例やPack例を実装へ追加した場合は関連crateのテストを必ず通してください。

## 8. World Packをzipで配布する

### 推奨構成

zipのルート直下にPack本体を置く方法:

```text
my-pack.zip
├─ pack.json
├─ README.md
├─ LICENSE
├─ scripts/
├─ character/
└─ motion/
```

単一のトップディレクトリで包む方法も使えます。

```text
my-pack.zip
└─ my-pack/
   ├─ pack.json
   ├─ README.md
   ├─ LICENSE
   └─ ...
```

次は避けてください。

- `pack.json`を複数含める。
- Pack rootの外に別ファイルを混ぜる。
- 絶対pathや`../`で外へ出るentryを入れる。
- 巨大な不要ファイル、編集用cache、秘密鍵を含める。
- OSが作る一時ファイルを大量に含める。

Yuukeiはzip slip対策、`pack.json`の位置、複数Pack、サイズ上限などを検証してから`packs-imported/<packId>/`へ展開します。

### 配布前のREADME

最低限、次を書きます。

- Pack名、ID、versionまたは更新日
- 作者と連絡先
- 対応Yuukei版
- 導入手順
- 必須・任意capability
- OS観測を使うsceneと、観測OFF時の動作
- 含まれるactorと操作方法
- 素材ごとの著作権・ライセンス
- 動画配信、スクリーンショット、改変、再配布、商用利用の条件
- 既知の問題
- 更新時にIDや保存変数を維持する方針

### Pack更新の互換性

更新時に変えないもの:

- Pack ID
- 既存actor ID
- 公開済みspeaker aliasを、必要なく削除しない
- 保存変数のkeyと型
- 台本から参照するmotion ID
- Extension eventのcanonical ID

変数名や型を変えると、既存ユーザーの保存値と合わなくなります。新keyを追加し、Daihonで旧値から段階的に移す設計を検討してください。現時点ではWorld Packが任意の移行コードを直接実行する仕組みではないため、単純で後方互換なデータ変更を優先します。

## 9. Extensionを配布する

### 推奨構成

```text
my-extension/
├─ manifest.json
├─ README.md
├─ LICENSE
├─ src/
│  └─ main.mjs
└─ THIRD-PARTY-LICENSES.md
```

ビルドが必要な場合は、配布物に実行可能な成果物を含めます。

```text
my-extension/
├─ manifest.json
├─ README.md
├─ LICENSE
└─ dist/
   └─ main.mjs
```

manifest:

```json
{
  "process": {
    "command": "node",
    "args": ["dist/main.mjs"]
  }
}
```

### Extension READMEに書くこと

- Extension名、ID、versionまたは更新日
- 何をするか、何をしないか
- hook、event購読、発行event、capabilityの一覧
- 各権限が必要な具体的理由
- 外部へ送信するデータ、送信先、保存期間
- API keyなど秘密の扱い
- 必要runtimeとversion(Node.jsなど)
- 対応OS・CPU
- install、設定、無効化、削除方法
- 派生データの保存場所と削除方法
- timeoutやprovider停止時のfallback
- ライセンスと依存ライブラリの通知

### 署名とsandboxについて

現在のprocess Extensionは信頼済みローカルコードとして実行され、OSレベルsandboxを保証しません。配布者はソース公開、release hash、再現可能なbuild、署名など、ユーザーが信頼を判断できる情報を提供するのが望ましいです。

World PackとExtensionを同梱配布する場合も、別フォルダ・別導入手順に分けてください。PackがExtensionの実行ファイルを内包し、暗黙に起動する構成にはしません。

## 10. プライバシー確認

配布前に、次の問いへ答えられるようにします。

### World Pack

- どのsignalをallowしているか。
- OS観測がOFFでも壊れないか。
- `profile`やAI指示に秘密情報を含めていないか。
- セリフへファイル名などを不用意に復唱しないか。
- AIへ委任するsignalと日次予算は妥当か。
- `conversation.text`へcooldownを付けていないか。

### Extension

- 購読eventを最小化しているか。
- `*`購読が本当に必要か。
- payload全文をログへ出していないか。
- 外部APIへ送る項目をREADMEで説明したか。
- secretがstdout、stderr、metadata、event payloadへ出ないか。
- 派生データをユーザーが消せるか。
- event logを直接読み書きしていないか。

## 11. 症状別トラブルシューティング

### Packを選べない

確認順:

1. 選んだフォルダ直下に`pack.json`があるか。
2. JSONとして読めるか。末尾commaやコメントはJSONでは使えない。
3. `schemaVersion`が1か。
4. `defaultActorId`がactors内にあるか。
5. script、model、motionの相対pathが存在するか。
6. speaker aliasが重複していないか。
7. required capabilityが利用可能か。

### Daihonが読み込めない

確認順:

1. `##`イベントと`###`sceneがあるか。
2. セリフの`「」`が閉じているか。
3. 関数の`＜＞`が閉じているか。
4. 分岐の最後に`おわり`があるか。
5. 話者がactor IDまたはaliasか。
6. 関数の引数数・型が正しいか。
7. `選択`・`解釈`の`不明`枝があるか。

### Daihonは読めるが反応しない

確認順:

1. signalが実際に発生しているか。
2. `signals.allow`にcanonical IDがあるか。
3. `合図:`の別名と綴りが正しいか。
4. `条件:`が真か。入力値を一時表示して確認。
5. `頻度:`制限中ではないか。
6. より具体的な別sceneが選ばれていないか。
7. 退場中のactorへ吹き出しを出そうとしていないか。

### Extensionをインストールできない

確認順:

1. `manifest.json`が直下にあるか。
2. IDの文字がASCII英数字、`-`、`_`、`.`だけか。
3. `runtime`が`process`か。
4. 何らかのhook、購読、発行、capability、aliasを宣言したか。
5. `*`購読なのに権限宣言が欠けていないか。
6. signal aliasが自分のnamespaceかつ`emittedEvents`内か。
7. settings key、select default、secret defaultが妥当か。

### Extensionは有効だが動かない

確認順:

1. `process.command`がPATH上にあるか。
2. argsのファイルがインストール済みコピー内にあるか。
3. 単体fixtureでexit code 0になるか。
4. stdoutが結果JSONだけか。
5. timeoutが短すぎないか。
6. hookの`commandTypes`またはevent filterが一致するか。
7. 設定が未保存でもfallbackできるか。
8. 連続失敗で休止されていないか。設定画面から再起動する。

### hookの変更が採用されない

確認順:

1. `action`が`replaceCommand`か。
2. `command`を含むか。
3. `id`、`type`、`residentId`を変えていないか。
4. 結果JSONのkeyがcamelCaseか。
5. 別hookが後段でさらに書き換えていないか。
6. `extension.hook.result`のerrorとoutputCommandを見る。

### Extension eventが拒否される

確認順:

1. typeが`ext.<manifest id>.`で始まるか。
2. `emittedEvents`の完全一致またはprefix patternに含まれるか。
3. alias targetも同じnamespaceか。
4. event連鎖がhop上限を超えていないか。
5. `extension.event.rejected`のreasonを見る。

### AIや音声だけ動かない

確認順:

1. capability Extensionが有効か。
2. World Packがrequired/optional capabilityを宣言しているか。
3. provider URL、model、API keyが保存済みか。
4. ローカルserverやVOICEVOXが起動しているか。
5. `process.timeoutMs`とResident Home側timeoutが十分か。
6. AIなし・TTSなしfallbackでテキスト生活は続くか。

## 12. release前の最終チェックリスト

### World Pack

- [ ] 新しい`YUUKEI_DATA_DIR`で初回起動した。
- [ ] 2回目起動で保存履歴を確認した。
- [ ] CLIで主要signalを確認した。
- [ ] DesktopでVRM、吹き出し、選択肢、dragを確認した。
- [ ] 観測OFFでも破綻しない。
- [ ] AI・TTSなしでも最低限の反応がある。
- [ ] AI・TTSありも確認した。
- [ ] 退場した住人が戻れる。
- [ ] 全script pathとasset pathがPack内相対path。
- [ ] README、LICENSE、素材クレジットがある。
- [ ] zipを別の空環境へインポートできる。

### Extension

- [ ] manifestを新しい環境へインストールできる。
- [ ] stdin fixtureの正常・異常系がある。
- [ ] stdoutにJSON以外を出さない。
- [ ] stderrに秘密を出さない。
- [ ] 権限とevent filterが最小限。
- [ ] hook固定フィールドを保持する。
- [ ] proposed event namespaceが正しい。
- [ ] capability envelopeと固有outputが正しい。
- [ ] 外部provider停止時のfallbackを確認した。
- [ ] 依存runtimeと対応OSをREADMEへ書いた。
- [ ] LICENSEとthird-party noticesがある。
- [ ] ソースまたは信頼検証手段を提供した。

設計上の判断に迷った場合は、[ユーザー・作者ガイドの入口](README.md)の選び分け表と、[Architecture](../../02-architecture.md)を確認してください。

