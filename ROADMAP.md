# Yuukei ロードマップ・アイディア集

将来の機能候補を「価値 / 設計スケッチ / 実装の進め方」で整理する。仕様の正は01〜08であり、ここは候補置き場。着手時は先に該当ドキュメントへ仕様を書いてから実装する。

「Codex単独」= 既存パターンの踏襲で実装可能。「設計必要」= 着手前に構文・契約・意味論の設計判断が必要。

## 近距離(次にやると効果が大きい)

### 選択肢バルーン(ユーザーが返答を選ぶ)【設計必要・大物】

伺かの選択肢の系譜。住人の問いかけに、ユーザーがバルーン上の選択肢クリックで答える。

- Daihonに選択肢ブロック構文を新設し、`ui.choice` 系RuntimeCommandを発行、ユーザーのクリックを `conversation.choice` RuntimeEventとして受けてsceneを続行する。
- **鍵になる資産**: `解釈` のインラインawait機構(scene中断→会話系イベントのみキュー→再開)がほぼそのまま流用できる。待ち先がLLMかユーザークリックかの違いだけ。
- タイムアウト(無視されたら選択肢を畳んで沈黙継続)と、選択結果の変数格納(`_返答 = ＜選択 「行く/行かない」＞` のような式)を設計する。

### hitSurface(肌/服/髪)をDaihonへ接続【Codex単独・小物】

スキンウェイト分類は実装済みで、Device Hostの gesture payload には `hit_surface`(skin/cloth/hair/face)が既にあるが、resident-homeの `major_payload` 許可リストに入っておらずDaihonへ届いていない。許可リストへ追加し `入力#hitSurface` として渡すだけで、「服を引っぱられた」「髪を触られた」反応が台本で書ける。05への追記も忘れずに。

### VOICEVOX読み上げ(speech.synthesis)【設計必要・中物】

03で予約済みの `speech.synthesis` capabilityの初実装。VOICEVOXはローカル・無料・日本語特化でプロジェクトと相性が最良。

- 新extension(またはyuukei-intelligenceに追加)がVOICEVOX HTTP API(既定 127.0.0.1:50021)を叩く。話者ID・話速は設定スキーマで宣言。
- `dialogue.say` 発行時にResident Homeが任意で `speech.synthesis` を呼び、音声参照をcommandに添付。Device Hostが再生。provider不在・失敗は無音でテキスト表示のみ(既存のLLM無し継続と同じ精神)。
- バルーン表示と音声のタイミング同期は最初は「再生開始のみ同期」で妥協する。

### ユーザー不在/復帰の検出【Codex単独〜軽い設計】

03に「実際のidle検出は別のcanonical signalとしてDevice Hostが観測する」と既に方針がある。OSのidle時間から `presence.idle.start` / `presence.idle.end` を発行し、別名 `不在_開始` / `復帰` を配る。「おかえりなさい」「いない間にこんなことがあった」(記憶と接続)が書ける。プライバシー負荷はほぼゼロ。

## 中距離(土台が揃ってから)

### 複数住人の掛け合い強化

default packに2住人はいるが、掛け合い(片方の発話がもう片方の合図になる、`関係#`変数の活用)のパターンが薄い。まず台本表現力の課題として、必要になった構文だけを08に足す。

### 埋め込み検索フェーズ2(記憶の意味検索)【設計必要】

現行のバイグラム+半減期検索を、`embedding.generate`(LM Studioのnomic-embed等)による意味検索に拡張。facts/episodesにembeddingを持たせ、retrieve時にコサイン類似度と新しさを混合。embedding不在時は現行方式へフォールバック。モデル差し替え時の再indexは `memory.rebuild` の初の実用例になる。

### デスクトップ観測(ゴミ箱、Downloads遭遇など)【設計必要・プライバシー重要】

05がDaihon向き例として挙げている「初回Downloads遭遇」「ゴミ箱を空にしたとき」の観測元。Device Hostのobserverとして実装し、canonical signal化。**機微な観測は明示権限**(04の方針)なので、観測の種類ごとにON/OFFする設定UIとevent logのprivacyカテゴリ付けを先に設計する。

### World Packのインポート/配布UX

現状は開発者向け配置のみ。zipインポート、pack.json検証、素材ライセンス表示、更新。ゴースト文化の「配布して交換する」楽しさの入口なので、コミュニティを狙うなら優先度を上げる。

## 既知の小粒バックログ(Codex単独可)

- 感情モジュレーターのしきい値(talkDesire 30/80)・語彙の設定化、気分の永続化
- シーン実行履歴の閲覧/リセットUI
- LLMタイムアウト・recentContext件数の設定化
- 設定GUIのselect動的選択肢(listModels照会)、APIキーのOSキーチェーン格納、使用量のコスト換算表示
- Claude専用provider、ChatGPT(openai-compatibleのbaseUrl変更)の動作確認

## 遠距離(構想のみ)

- Extension配布エコシステム(署名、権限表示、ストア)
- モバイル/別デバイスSurface(02のSurface抽象はこれを見越している)
- 住人の「生活」の可視化(event logタイムラインビューア。デバッグ道具としても有用)
