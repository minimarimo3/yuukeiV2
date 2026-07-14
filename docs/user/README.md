# Yuukei ユーザー・作者ガイド

このガイドは、Yuukeiを使う人と、自分の住人・台本・機能を作りたい人のための入口です。プログラミングやゲーム制作の経験がなくても、必要なところから順番に読めるようにしています。

Yuukeiは、質問に答えるだけのチャットアプリではありません。住人がデスクトップやフォルダ、ウィンドウを生活空間として受け取り、ユーザーの普段の操作を一緒の暮らしの出来事にするための基盤です。そのため、作るものも大きく2種類に分かれます。

- **World Pack**: 誰が住むか、どんな姿か、どんな世界観か、何をきっかけにどう振る舞うかをまとめたものです。
- **Extension**: AI、音声合成、イベント加工、外部サービス連携など、交換可能な能力を追加するものです。

住人らしい決められた場面は、World Pack内の**Daihon**で書きます。AI Extensionは、その台本に一致しなかった日常の余白や、台本内で明示的に生成を頼んだ一文を補います。

## 読み方

目的に合う順番で読んでください。

### Yuukeiを使いたい

まず [Yuukeiをはじめる](../../USERGUIDE.md) を読んでください。起動、住人との触れ合い、AI・VOICEVOXの設定、プライバシー設定を説明しています。

### 自分の住人や世界観を作りたい

1. [World Packを作る](01-world-pack-guide.md)
2. [Daihonをはじめから書く](02-daihon-guide.md)
3. [Daihon言語リファレンス](../../08-daihon-language-reference.md)
4. [テスト・配布・トラブルシューティング](04-testing-and-distribution.md)

最初は、モデルを用意せずCLIで動く最小Packを作るのがおすすめです。セリフが出ることを確認してから、VRM、モーション、部位判定、AI連携を足すと、問題の場所を切り分けやすくなります。

### 新しい能力や連携機能を作りたい

1. [Extensionを作る](03-extension-guide.md)
2. [テスト・配布・トラブルシューティング](04-testing-and-distribution.md)
3. 必要に応じて [通信契約の設計資料](../../03-protocols.md)

Extension開発には、JSONと、Node.jsなど外部プロセスを作れる言語の基礎知識が必要です。このガイドではNode.jsの依存パッケージなしの例を使います。

## 最初に覚える言葉

| 言葉 | やさしい説明 | 所有するもの |
| --- | --- | --- |
| 住人 | デバイスのUI空間で生活する登場人物 | World Packが人物像を定義し、Resident Homeが継続状態を持つ |
| World Pack | 住人、見た目、世界観、台本をまとめたフォルダ | 作者が配布・編集するデータ |
| Daihon | 合図、条件、セリフ、動作を書く台本言語 | World Packの一部 |
| RuntimeEvent | ユーザー操作や端末観測など、住人へ届く出来事 | Resident Homeが検証して記録する |
| RuntimeCommand | セリフ、表情、動作など、住人の見える振る舞い | Resident Homeが作り、Surfaceが表示する |
| Extension | AI、TTS、イベント加工などを提供する追加機能 | 独立したmanifestと実行プログラム |
| capability | `dialogue.generate` や `speech.synthesis` のような名前付き能力 | Extensionが提供し、Resident Homeが選択する |
| canonical event log | 住人の生活で実際に起きたことの正本 | Resident Home |
| Surface | VRM、吹き出し、CLIなど、住人の現在の身体・表示面 | 表示だけを担当し、人格や長期記憶は持たない |

## World Pack、Daihon、Extensionの選び分け

迷ったら、次の表で置き場所を決めます。

| 作りたいもの | 置き場所 | 理由 |
| --- | --- | --- |
| 初回起動の決め台詞 | Daihon | 作者が必ず見せたい場面だから |
| 頭を撫でたときの反応 | Daihon + World Packのhit zone | 人物と身体に固有だから |
| 住人のVRMモデル | World Pack | 見た目は世界観の一部だから |
| 台本にない日常会話 | `dialogue.generate` Extension | 生成方式を交換できるようにするため |
| VOICEVOXで読み上げる | `speech.synthesis` Extension | 台本由来かAI由来かに関係なく使える能力だから |
| すべてのセリフへ語尾を付ける | `beforeCommandEmit` hook Extension | 公開commandを配信直前に加工する機能だから |
| 特定イベントを検出し、新しい合図を増やす | event購読Extension | Coreの語彙を直接変更せず、Extension名前空間で増やせるから |
| フォルダを開いたときの台詞 | Daihon | OS観測自体はDevice Hostが行い、Packは届いた合図へ反応するだけだから |
| イベントログから独自の記憶DBを作る | Memory Extension | 記憶方式は交換可能で、ログが正本だから |

World PackからOS APIやAI APIを直接呼ばないでください。また、ExtensionからWorld Pack、Surface、イベントログファイルを直接書き換えないでください。どちらも、Yuukeiが公開するJSONメッセージとcapabilityの境界を通します。

## 現在の開発版を動かす

正式なインストーラは準備中です。ソースから動かす場合はNode.js 20以降、pnpm、Rustが必要です。

```sh
pnpm install
pnpm dev:cli
```

CLIは開発用の正式なSurfaceです。トップメニューでは、たとえば次の操作ができます。

| 番号 | 操作 |
| --- | --- |
| `1` | 住人を撫でる・つつく |
| `4` | 話しかける |
| `5` | 現在のsnapshotを見る |
| `7` | World Packを選ぶ・戻す・状態を見る |
| `8` | Extensionをインストール・有効化・無効化する |
| `9` | イベントログを書き出す、保存先を見る |
| `0` | 終了する |

デスクトップ版は次で起動します。

```sh
pnpm dev:tauri
```

開発中は、専用のデータディレクトリを指定すると普段使いの状態と分けられます。

PowerShell:

```powershell
$env:YUUKEI_DATA_DIR = "$PWD/.local/authoring-data"
pnpm dev:cli
```

bash / zsh:

```sh
export YUUKEI_DATA_DIR="$PWD/.local/authoring-data"
pnpm dev:cli
```

このディレクトリにはイベントログ、Pack選択、Extension、設定が保存されます。テスト用と普段使い用で分けると、`一度きり` のシーンや保存変数を新しい状態から確認しやすくなります。

## 安全について

World Packは、原則としてJSON、Daihon、モデル、モーションなどのデータです。Pack内の参照はPackの外へ出られないよう検証されます。

一方、現在の`runtime: "process"` Extensionは、信頼したローカルコードとして実行されます。manifestの権限欄は宣言とユーザー同意の境界ですが、v1ではOSレベルのsandboxを保証しません。知らない作者のExtensionをインストールする前に、配布元、ソース、実行コマンドを確認してください。

イベントログには生活の出来事が保存されます。ウィンドウ、フォルダ、ダウンロードの観測は初期状態ではOFFです。Pack作者は、観測が無効でも住人が壊れず、単にその場面が起きないように作ってください。

## 設計を深く知りたい場合

作者向けガイドは「どう作るか」を中心にしています。責務境界や設計理由を確認したい場合は、次を読んでください。

- [UI内生活者という製品思想](../../01-concept.md)
- [Resident Home、Device Host、Surface、Extensionの責務](../../02-architecture.md)
- [RuntimeEvent、RuntimeCommand、Extension RPC](../../03-protocols.md)
- [イベントログと記憶の違い](../../04-event-log-and-memory.md)
- [World PackとDaihonの設計](../../05-world-pack-and-daihon.md)

