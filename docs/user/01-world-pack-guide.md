# World Packを作る

World Packは、Yuukeiへ「誰が住むか」「この住人にとってUIがどんな場所か」「どんな場面でどう振る舞うか」を渡すフォルダです。住人の設定、VRM、モーション、Daihon台本、利用する合図や能力の宣言を、1つの配布単位にまとめます。

この章では、まずモデルなしで動く最小Packを作り、そのあとVRM、複数住人、AI委任、hit zoneを順に追加します。

## 1. 作業フォルダを作る

任意の場所に、次の構成を作ります。

```text
my-first-pack/
├─ pack.json
└─ scripts/
   └─ hello.daihon
```

PackフォルダはYuukeiのリポジトリ内に置く必要はありません。設定画面やCLIから外部フォルダとして選べます。編集内容をそのまま参照できるので、作者は作業フォルダを直接選ぶのが便利です。

## 2. 最小の`pack.json`

`pack.json`へ次を書きます。

```json
{
  "schemaVersion": 1,
  "id": "my-first-pack",
  "displayName": "はじめての住人",
  "defaultActorId": "alice",
  "actors": [
    {
      "id": "alice",
      "displayName": "アリス",
      "speakerAliases": ["ア"],
      "profile": {
        "role": "デスクトップの小さな住人",
        "speechStyle": "短く、穏やかに話す"
      }
    }
  ],
  "signals": {
    "allow": [
      "app.startup",
      "conversation.text",
      "avatar.gesture.poke"
    ]
  },
  "capabilities": {
    "required": [],
    "optional": []
  },
  "daihon": {
    "scripts": ["scripts/hello.daihon"]
  },
  "initialVariables": {
    "全体#起動回数": 0
  },
  "uiSpace": {
    "desktop": "小さな共有部屋"
  }
}
```

このPackにはrendererがないため、まずCLIで台本を確認します。デスクトップで姿を表示するVRM設定は後で追加します。

### 必須フィールド

| フィールド | 意味 | 注意 |
| --- | --- | --- |
| `schemaVersion` | manifest形式の版 | 現在は`1` |
| `id` | Packを識別する安定ID | あとから変更すると別Packとして扱われる可能性があるため、公開後は変えない |
| `displayName` | 設定画面へ出す名前 | 読みやすい日本語でよい |
| `defaultActorId` | 合図に対象住人がない場合の住人 | `actors[].id`のどれかと一致させる |
| `actors` | 住人の一覧 | IDの重複は不可 |
| `signals.allow` | Packが受け取る出来事の許可リスト | 必要なものだけ書く |

`capabilities`、`daihon`、`initialVariables`、`uiSpace`には既定値がありますが、作者が意図を確認しやすいよう、最初は明示しておくことをおすすめします。

### IDの付け方

Pack IDとactor IDは、短く安定したASCIIのkebab-caseまたはlowercaseをおすすめします。

```text
良い例: my-first-pack, alice, room-keeper
避けたい例: 新しいパック, Alice 2, test-final-final
```

表示名はあとから変えられます。台本・保存状態・外部参照に使われるIDは、公開後に変えないでください。

## 3. 最小のDaihonを書く

`scripts/hello.daihon`へ次を書きます。

```daihon
## アプリ_起動

### はじめての挨拶
合図: ＠アプリ_起動
条件:（全体#起動回数 = 0）
頻度: 一度きり
話者: ア
全体#起動回数 = 全体#起動回数 + 1
「はじめまして。今日から、ここで暮らします。」

### いつもの挨拶
合図: ＠アプリ_起動
条件:（全体#起動回数 >= 1）
話者: ア
全体#起動回数 = 全体#起動回数 + 1
「おかえりなさい。起動は＜全体#起動回数＞回目ですね。」

## 会話_入力

### AIなしの相槌
合図: ＠会話_入力
話者: ア
「聞こえています。『＜入力#ユーザー発言＞』と言いましたね。」

## 住人_つつく

### つつかれた
合図: ＠住人_つつく
話者: ア
「わっ。そこにいましたか。」
```

`speakerAliases`に`"ア"`を登録したため、台本では`話者: alice`の代わりに`話者: ア`と短く書けます。Surfaceやイベントログへ流れる前に、Yuukeiがcanonical actor IDの`alice`へ変換します。

> [!NOTE]
> `「」`の中へ別の引用符を入れる場合、Daihonでは`「「...」」`と二重にしてエスケープするのが正式です。上の`『』`は普通の文字なので、そのまま使えます。

## 4. CLIで読み込む

開発用のデータディレクトリを指定してCLIを起動します。

```powershell
$env:YUUKEI_DATA_DIR = "$PWD/.local/my-first-pack-data"
pnpm dev:cli
```

トップメニューで次を選びます。

1. `7` World Pack
2. `1` 選択
3. `my-first-pack`フォルダの絶対パスを入力
4. 読み込み後、トップへ戻る
5. `4` 話しかける、または`1` 撫でる・つつく
6. `5` 状態を見る

Packを選んだ直後にResident HomeがそのPackで起動し、`app.startup`が台本へ届きます。エラーがあれば、World Packの状態表示またはデスクトップ版の設定画面に診断が出ます。

### `一度きり`をもう一度試したい

`一度きり`の実行履歴と`全体#`変数は、Pack installごとのデータとして保存されます。次のどちらかで新しい状態から試せます。

- 別の空の`YUUKEI_DATA_DIR`を使う。
- テスト専用Pack IDへ一時的に変える。ただし公開済みPackのIDは変えない。

通常の再起動だけでは履歴が消えないのが正しい動作です。

## 5. `pack.json`を詳しく理解する

### `actors`: 住人の定義

```json
{
  "id": "alice",
  "displayName": "アリス",
  "speakerAliases": ["ア"],
  "profile": {
    "role": "デスクトップの小さな住人",
    "speechStyle": "短く、穏やかに話す",
    "likes": ["静かな窓", "画像フォルダ"]
  }
}
```

- `id`は台本、snapshot、TTSの話者振り分けなどに使うcanonical IDです。
- `displayName`は画面表示用です。台本の話者名として暗黙には使われません。
- `speakerAliases`は台本専用の短縮名です。別actorのIDやaliasと重複できません。
- `profile`は自由なJSON objectです。`dialogue.generate`へ人物像として渡されることがあるため、AIへ伝えたい役割や話し方を書けます。秘密情報は置かないでください。

### `signals.allow`: 届いてよい出来事

World Packは、allowlistにあるsignalだけを台本へ受け取ります。canonical IDを基本にします。

```json
{
  "signals": {
    "allow": [
      "app.startup",
      "conversation.text",
      "presence.life_tick",
      "desktop.folder.opened"
    ]
  }
}
```

`アプリ_起動`、`会話_入力`のようなYuukei標準の日本語別名を書いても、読み込み時にcanonical IDへ変換されます。ただし、manifestは実装者やツールも読むため、canonical IDに統一する方が分かりやすくなります。Daihonでは日本語別名を使うのがおすすめです。

allowlistはOS観測を有効化する設定ではありません。たとえば`desktop.folder.opened`を許可しても、ユーザーがプライバシー設定でフォルダ観測をONにしない限り、イベントは届きません。

### `capabilities`: 必須能力と任意能力

```json
{
  "capabilities": {
    "required": [],
    "optional": [
      "dialogue.generate",
      "dialogue.interpret",
      "dialogue.extract",
      "speech.synthesis"
    ]
  }
}
```

- `required`: 利用可能なExtensionがなければPackを有効化できない能力です。
- `optional`: あれば使うが、なくてもPackは動く能力です。

初心者向けPackでは、できるだけoptionalにしてください。AIや音声合成がなくても、Daihonだけで暮らしが続くPackは導入しやすくなります。

特定のExtension IDをPackから直接指定しません。Packは`dialogue.generate`のような能力名だけを宣言し、どのExtensionを使うかはResident Homeとユーザー設定が決めます。

### `daihon.scripts`: 台本一覧

```json
{
  "daihon": {
    "scripts": [
      "scripts/startup.daihon",
      "scripts/conversation.daihon",
      "scripts/desktop-life.daihon"
    ]
  }
}
```

パスはPack rootからの相対パスです。絶対パス、`..`、Pack外へ抜けるsymlinkは拒否されます。ファイルを分けても、Daihonの動作は変わりません。合図や題材ごとに分けると管理しやすくなります。

### `initialVariables`: 初期状態

```json
{
  "initialVariables": {
    "全体#初対面済み": false,
    "全体#呼び名": "",
    "全体#散歩回数": 0
  }
}
```

`全体#`、`住人#actor#`、`関係#a#b#`の変数は保存されます。初期値は保存値がまだないkeyにだけ使われます。公開後に初期値を書き換えても、既存ユーザーの保存値を上書きしません。

Daihonファイル内の`初期値:`でも初期化できます。Pack全体で共有する値は`pack.json`、特定のイベントを理解するために近くへ置きたい値はDaihonの`初期値:`という使い分けができます。

### `uiSpace`: UI空間の解釈

```json
{
  "uiSpace": {
    "desktop": "shared-room",
    "downloads": "entrance",
    "pictures": "gallery"
  }
}
```

これは「デスクトップは共有部屋」「Downloadsは玄関」のような世界観上の解釈です。OSのパスやAPI設定ではありません。値はPack固有のデータであり、現時点ですべてのSurfaceが自動演出へ使うとは限りません。台本・プロフィール・将来のPack UIで一貫した語彙を使うために置きます。

## 6. VRMを追加する

次の構成にします。

```text
my-first-pack/
├─ pack.json
├─ character/
│  └─ alice.vrm
├─ motion/
│  └─ walk.vrma
└─ scripts/
   └─ hello.daihon
```

actorへ`renderer`を追加します。

```json
{
  "id": "alice",
  "displayName": "アリス",
  "speakerAliases": ["ア"],
  "profile": {
    "role": "デスクトップの小さな住人",
    "speechStyle": "短く、穏やかに話す"
  },
  "renderer": {
    "kind": "vrm",
    "model": "character/alice.vrm",
    "motions": {
      "walk": "motion/walk.vrma"
    }
  }
}
```

現在のrenderer kindは`vrm`です。modelとmotionのパスはPack rootからの相対パスで、実在するファイルを指す必要があります。

`motions`のkeyはDaihonから使う安定IDです。

```daihon
＜動作 walk＞
＜歩く 右端 動作=walk＞
```

モデルのファイル名をあとから変えても、`motions`のkeyを維持すれば台本を変更せずに済みます。

## 7. 触る場所を定義する

VRMは、標準humanoid boneから頭、腕、脚などを自動分類できます。尻尾、羽、帽子など、モデル固有の場所を追加したい場合は`hitZones`を書きます。

### humanoid boneを使う

```json
{
  "id": "head",
  "label": "頭",
  "source": "humanoidBone",
  "bones": ["head"],
  "shape": "auto",
  "events": [
    "avatar.gesture.poke",
    "avatar.gesture.pat"
  ],
  "priority": 40
}
```

### ノード名を使う

```json
{
  "id": "tail",
  "label": "しっぽ",
  "source": "nodeName",
  "nodes": ["Tail", "Tail_001"],
  "shape": "mesh",
  "events": ["avatar.gesture.poke"],
  "priority": 50
}
```

`source: "humanoidBone"`では`bones`が1つ以上必要です。`source: "nodeName"`では`nodes`が1つ以上必要です。同じactor内でhit zone IDを重複させられません。

Daihonでは次の入力を使えます。

```daihon
## 住人_つつく

### しっぽ
合図: ＠住人_つつく
条件:（入力#actorId = 「alice」 かつ 入力#hitZoneId = 「tail」）
話者: ア
「しっぽは、びっくりするので優しくしてください。」
```

代表入力は`入力#actorId`、`入力#hitZoneId`、`入力#hitZoneLabel`、`入力#hitSurface`です。`hitSurface`は`skin`、`cloth`、`hair`、`face`などの表面分類で、判定できない場合は`unknown`になり得ます。必ずしも特定値が来る前提にしないでください。

## 8. 複数の住人を登場させる

`actors`へ複数定義し、aliasを重複させないようにします。

```json
{
  "defaultActorId": "alice",
  "actors": [
    {
      "id": "alice",
      "displayName": "アリス",
      "speakerAliases": ["ア"]
    },
    {
      "id": "bob",
      "displayName": "ボブ",
      "speakerAliases": ["ボ"]
    }
  ]
}
```

掛け合いは次のように書けます。

```daihon
## アプリ_起動

### 二人の挨拶
合図: ＠アプリ_起動
話者: ア
「おかえりなさい。」
ボ: 「待っていました。」
ア: ＜表情 笑顔＞「今日は何をしましょうか。」
```

`話者:`は以後の既定話者です。`ボ: 「...」`の形は、その行だけ話者を切り替えます。関数もその行の話者へ作用します。

## 9. AIへ台本の余白を委任する

Daihonに一致するsceneがないときだけ、特定signalを`dialogue.generate`へ委任できます。

```json
{
  "capabilities": {
    "required": [],
    "optional": ["dialogue.generate"]
  },
  "llmDelegation": {
    "signals": [
      { "signal": "conversation.text" },
      {
        "signal": "presence.talk_impulse",
        "cooldownSeconds": 300
      }
    ],
    "dailyBudget": 50
  }
}
```

重要な順序は次です。

1. signalがallowlistにあるか確認される。
2. Daihonのsceneが探される。
3. 一致するsceneが実行できれば、台本が使われる。
4. 一致するsceneがなく、`llmDelegation`にsignalがあればAIへ委任される。
5. providerがなければ沈黙する。

`conversation.text`へcooldownを付けないでください。cooldown中の入力はエラー表示なしで見送られるため、ユーザーには故障したように見えます。cooldownは`presence.talk_impulse`のような環境起点のsignalへ使います。

AIなしでも反応させたい場合は、Daihonで`入力#AI接続`を確認します。

```daihon
## 会話_入力

### AIなしの相槌
合図: ＠会話_入力
条件:（入力#AI接続 = いいえ）
話者: ア
「聞いています。今は、うまく返事を考えられないのですが。」
```

AIが接続されているとこのsceneは候補から外れ、Daihonに別の一致sceneがなければ`llmDelegation`へ進みます。

## 10. ライセンスとREADMEを用意する

配布するPackには、少なくとも次を追加してください。

```text
my-first-pack/
├─ README.md
├─ LICENSE
├─ pack.json
├─ character/
├─ motion/
└─ scripts/
```

READMEには次を書きます。

- Packの名前と概要
- 作者・連絡先
- 対応するYuukeiの版
- 導入方法
- 必須・任意capability
- モデル、モーション、音声、画像など各素材の出典と利用条件
- 改変・再配布・配信利用・商用利用の可否
- AIへ送られ得るprofileや会話文脈についての注意

zipインポート時、Yuukeiは`LICENSE`、`README.md`、`pack.json`の`license`候補から配布条件を探してユーザーへ表示します。素材ごとに条件が違う場合は、READMEで明確に分けてください。

## 11. よくある読み込みエラー

| 症状 | 主な原因 | 直し方 |
| --- | --- | --- |
| `pack.json`が見つからない | 選択した階層が1つ上・下 | `pack.json`が直下にあるフォルダを選ぶ |
| `defaultActorId is not declared` | default IDとactor IDが不一致 | 綴りと大文字小文字を合わせる |
| `duplicate speaker alias` | aliasを複数actorで共有 | actorごとに一意にする |
| modelやscriptを読めない | 相対パスの誤り、ファイル欠落 | Pack root基準でパスを確認する |
| Pack root外参照として拒否 | `..`、絶対パス、外向きsymlink | 必要な素材をPack内へ置き直す |
| Daihonのunknown speaker | 台本の話者がactor IDでもaliasでもない | `speakerAliases`へ追加するか話者名を直す |
| required capability不足 | 必須能力のExtensionが未導入・無効 | Extensionを導入するかoptionalへ設計変更する |
| デスクトップで姿が出ない | rendererがない、VRMパスが誤り | CLIで台本を確認後、renderer設定を見直す |

次は [Daihonをはじめから書く](02-daihon-guide.md) で、合図、条件、変数、選択肢、AI関数、デスクトップ演出を詳しく学びます。
