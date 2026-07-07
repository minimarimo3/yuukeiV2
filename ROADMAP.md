# Yuukei ロードマップ・アイディア集

将来の機能候補を「価値 / 設計スケッチ / 実装の進め方」で整理する。仕様の正は01〜08であり、ここは候補置き場。着手時は先に該当ドキュメントへ仕様を書いてから実装する。

## リリースまでのロードマップ(v1.0)

コアの土台(Daihon、event log、Extension境界、LLM統合3ケース、記憶、設定GUI、シーン選択再設計)は完成している。リリースまでの残りは「新機能」よりも、初めての他人が使える状態にする作業が中心。マイルストーンは依存順であり、M2以降は一部並行できる。

### M1: 体験の核を締める(機能の最終ピース)

伺か系デスクトップ生活者として、v1の対話体験に必要な最後の機能。詳細は本書の「近距離」を参照。

- [x] 選択肢バルーン【設計必要・大物】— 住人の問いかけにクリックで答える。`解釈` のawait機構を流用(ac1b880)
- [x] hitSurface(肌/服/髪)をDaihonへ接続【小物】(45e5ab3)
- [x] ユーザー不在/復帰の検出(`presence.idle.*`)【小物】— 「おかえりなさい」が記憶と接続できる(45e5ab3)
- [x] VOICEVOX読み上げ【中物】— v1に含める。provider不在・失敗時は無音でテキスト継続(3cd6a40)

### M2: デスクトップ地形と遭遇(ウィンドウに座る・フォルダで出くわす)【設計必要・大物】

「OSのUIはキャラクターにとっての地形」(01)をv1で最初に体現する層。完成条件は、default packで「Finder/ExplorerでDownloadsを開いたら、この前ダウンロードしたあんぱん画像を住人が食べていた」というsceneが書けること。4つの積み木に分解して設計する。

- [x] ウィンドウ地形の観測 — Device HostがOSのウィンドウ一覧を観測し、canonical signal(`desktop.window.*`)とSurface向け地形スナップショットにする。v1はポーリング実装(macOS: CGWindowList、Windows: EnumWindows。Windowsは実機未検証)(6adfb55)
- [x] 地形への配置 — actorウィンドウが対象ウィンドウの枠(上辺)に追従して座るRuntimeCommand(`stage.perch` 系)。移動・リサイズ追従と消滅時のフォールバック実装済み(6adfb55)
- [x] フォルダ遭遇の観測 — `desktop.folder.opened`。Windows: IShellWindows(実機未検証)、macOS: Finder最前面時のosascript。パスは既知カテゴリへ正規化し生記録しない(1bdb575)
- [x] ダウンロード観測 — notify watcherで `desktop.download.completed`(ファイル名・種類カテゴリ)。フォルダ遭遇dispatch時に直近7日の最新DLを 入力#最近のダウンロード としてenrich(1bdb575, 3e1c1cf)
- [x] Daihon接続 — 別名5種+日本語入力名+`枠に座る`/`枠から降りる`。08の11章に記載(3e1c1cf)
- [x] プライバシー設計を先行 — 観測3種は既定OFF、設定画面「観測とプライバシー」でON/OFF。タイトル・フルパス不記録、privacy.category=desktop-observation(6adfb55, 1bdb575)

### M3: 初回体験とコンテンツ

開発者でない人がインストール直後に「生きている」と感じられるまでの導線。

- [x] 初回起動オンボーディング — World Pack選択、LLM設定(AIなしで始めるを明示)、観測プライバシーの3ステップ(3d2249e)
- [x] World Packのzipインポート — zip-slip対策・上限つき検証、ライセンス表示、packs-importedへ展開(63c2e2e)
- [x] default packの拡充 — 新シグナル全種への反応台本と掛け合いを追加。あんぱんシナリオは統合テストで恒久保証(7c45624, 1f8d9a2)
- [x] LLM未設定状態での通し確認 — capabilities/extensionsゼロの新規データディレクトリで起動・生活イベント・あんぱんシナリオが成立することを確認(会話は設計どおり沈黙)。数日規模のソークはM4の長時間稼働テストで行う

### M4: プライバシーと安定性

ユーザーの操作を生活史として記録する設計上、公開前に必須の層(04の方針)。

- [ ] イベントログの閲覧・削除UI(記憶の閲覧・編集・忘却UIは済み。event log側にも同等を)
- [x] 観測の種類ごとのON/OFF設定と、何を記録しているかの説明画面(M2のウィンドウ・フォルダ・ダウンロード観測を含む)(1bdb575)
- [ ] events.sqlite3 / app-activity.jsonl のサイズ管理(ローテーションまたは上限)
- [ ] Extensionプロセスの異常系 — クラッシュ・タイムアウト・不正JSONからの回復と、ユーザーへの通知
- [ ] 長時間稼働テスト(数日つけっぱなしでのメモリ・CPU・ログ肥大)

### M5: パッケージングと配布

対象OSはmacOSとWindowsを公式サポート。LinuxはCIビルド・動作を意図的に壊さない(締め出さない)が、実機確認と不具合対応は約束しない。

- [ ] アプリアイコン一式 — 現状 `tauri.conf.json` の `icon` が空配列。`tauri icon` で全サイズ生成
- [ ] macOS実機確認 — 署名+notarization、Accessibility権限の取得導線(M2の観測に必須)、`macOSPrivateApi` の影響確認
- [ ] Windows実機確認 — インストーラ、透過ウィンドウ動作、M2のウィンドウ観測。伺か文化圏はWindows比重が高いので優先度は高い
- [ ] ログイン時自動起動オプション(住人は常駐してこそ)
- [ ] 更新手段 — tauri-plugin-updaterか、まずは手動更新告知か。v1は手動+更新通知で十分
- [ ] バージョニング方針、CHANGELOG開始

### M6: ドキュメントと公開

- [x] ユーザー向け導入ガイド — USERGUIDE.mdを追加(インストール〜World Pack導入〜LLM/VOICEVOX設定〜プライバシー)
- [ ] World Pack作者向け公開ドキュメント(08がほぼそのまま使える。公開場所を決める)
- [ ] ライセンス選定 — 依存ライブラリ・素材(VOICEVOX利用規約含む)を棚卸しした上で最後に決定
- [ ] β配布(GitHub Releases)→ フィードバック反映 → v1.0タグ

### v1に含めないもの

埋め込み検索フェーズ2、M2以外のデスクトップ観測(ゴミ箱を空にした等の追加観測)、フォルダ内アイコン単位の配置、Extension配布エコシステム、モバイルSurface、event logタイムラインビューア。いずれも本書の中距離・遠距離のまま維持し、v1.xで検討する。

### リリース判定基準(Definition of Done)

- 開発ツールなしのクリーンなmacOS/Windowsマシンで、インストーラから起動して住人が現れる
- あんぱんシナリオ(Downloadsで過去のダウンロード物に住人が反応)がdefault packで動く
- LLM設定なしで台本のみの生活が破綻しない
- 記録している情報をユーザーが一覧・削除できる
- 3日間の常駐でクラッシュ・ログ肥大・体感劣化がない

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

Downloads遭遇・フォルダ観測・ウィンドウ地形はリリースM2へ前倒し済み。ここに残るのは「ゴミ箱を空にした」などの追加観測。

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
