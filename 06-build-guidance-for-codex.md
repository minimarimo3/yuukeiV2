# Build Guidance for Codex

この文書は、Yuukeiを一から実装するCodexおよび実装者向けの判断基準である。既存MVPのクラス名、ディレクトリ構造、単一AppRuntimeは新規設計の制約としない。継承するのはプロダクト思想と意味境界である。

実装を始める前に `00-experience-contract.md` を読む。体験契約、Golden Scene、測定可能な受け入れ条件は、アーキテクチャ上の整然さや機能数より優先する。既存の境界は捨てないが、境界を実装しただけで完成とは判定しない。

## Build Order

### 1. Golden Scene First

最初に、`00-experience-contract.md` のGolden Sceneから一つを選び、実際のWindowsデスクトップで始まりから終わりまで観察可能な短い体験として定義する。

- ユーザーが見る開始状態。
- 住人の準備、継続、中断、終了または遷移。
- どのOS変化が身体へ何を起こすか。
- 発話を消しても伝わる身体、配置、間。
- 画面外活動を含む場合に、再会まで保持する状態。
- 実画面での観察手順と合格条件。

機能一覧から実装を始めない。最初の場面で必要になった最小のevent、command、状態だけを縦に通し、目視できる結果まで実装する。

### 2. Continuity and Event Log

Golden Sceneで生じた出来事と活動状態を、Resident Homeとcanonical event logで継続可能にする。

- RuntimeEvent、RuntimeCommand、ResidentSnapshot、SurfaceSessionを必要な範囲で定義する。
- 住人の場所だけでなく、活動ID、活動段階、開始時刻、関心対象、中断理由を必要な範囲で保持する。
- event logのappend/read/export/deleteを成立させる。
- Surfaceの切断やアプリ再起動後に、活動を再開、経過完了、安全な既定活動への復帰のいずれかへ解決する。
- 同じ活動を二重開始しないよう、時刻と因果を扱う。

Resident HomeはTauri、OS window handle、renderer、WebViewへ依存させない。人格と生活の継続性を所有し、Device HostとSurfaceが入れ替わっても同じ住人であることを守る。

### 3. Embodied Windows Surface

Windowsを第一対象として、既存VRM Surfaceで行動の時間とUIとの身体的因果を完成させる。

- 対象ウィンドウを足場、境界、遮蔽物として扱う。
- 移動、リサイズ、最小化、閉じるに対し、座標追従だけでなく姿勢と活動を遷移させる。
- 歩く、近づく、座る、つかまる、飛び退く、隠れるなどを複数段階の行動として構成する。
- Surfaceは人格の長期状態を所有せず、Resident Homeが決めた活動と身体表現を描画する。
- 常設チャット欄を製品体験の中心にしない。

CLI Surfaceはprotocolや状態機械の機械的テストに使ってよい。ただし、CLIで通ったことをGolden Sceneの完成としない。実際のWindowsデスクトップで動き、間と身体を目視できることが必要である。

2D Surfaceは、VRM完成後に余裕があれば足す装飾ではなく、第一級の表現方式として扱う。ただし初期は、既存VRMで活動と因果の基盤を検証してよい。VRMが立って話すだけの状態を完成にしない。

### 4. Device Observation and Safe Staging

Device Hostをローカル端末の感覚器と能力ホストにする。

- Resident Homeへ接続する。
- Surfaceを登録する。
- ローカルExtensionを登録する。
- Extensionがmanifestで宣言したcapability、hook、event購読、event発行、signal aliasをResident Homeへ登録する。
- 端末のpresence、生活時計tick、実idle、起動、終了をRuntimeEventとして送る。
- ユーザーが選んだWorld Packディレクトリをローカル設定に保存し、選択されたPack installに対応するResident Home起動設定を作る。

OS観測はGolden Sceneで必要なものから段階的に増やす。Windowsのウィンドウ、Explorer、音楽状態などはDevice Host側の拡張として扱い、明示的な許可と記録内容の説明を必須にする。

観測結果を使った演出はYuukei所有のSurfaceまたはオーバーレイで行う。ユーザーのファイルを勝手に移動、編集、削除しない。他アプリへ勝手に入力やクリックを行わない。外部アプリを改変したように見せる場面も、安全な演出層で成立させる。

初期のExtension実装は `beforeCommandEmit` に限定できる。ただし、同じmanifestモデルでcapability提供、`onEventAppended`、RuntimeEvent発行、Daihon signal alias寄贈を扱える設計とする。外部プロセス型Extensionは、Device Hostが設定画面で選ばれたフォルダを `YUUKEI_DATA_DIR/extensions/<extensionId>/` へコピーし、`manifest.json` と `YUUKEI_DATA_DIR/settings/extensions.json` を読んでResident Homeへ公開protocol Extensionとして登録する。ExtensionにCore内部状態、Tauri AppHandle、Surface実装、event logファイルを直接渡さない。v1では信頼済みローカルコードとして扱い、manifest permissionsは宣言とユーザー同意であり、OS sandboxを仕様として約束しない。

World Pack選択UIはDevice Hostに置く。ただし、active World Packの解釈、required capability確認、Packごとのresident/event-log分離はResident Home起動境界の責務として扱う。Surface Clientは `ResidentSnapshot.worldPackId` を表示できるが、Pack選択や人格継続性を所有しない。

### 5. Daihon and Activity Integration

Daihonはsidecarまたはservice境界として接続する。

- Resident HomeはDaihon内部型へ依存しない。
- Daihon Hostはevent、variables、context、cooldownを受け取る。
- Daihon Hostはcommands、variable patches、executed scenesを返す。
- callbackでruntime queryやcapability invocationを要求できる。

Daihonなしでも最小Resident Homeは起動できるようにする。ただし、製品のキャラクターらしさはDaihonで作る。

Daihonのsceneは単発の発話とmotionだけでなく、活動の開始、段階遷移、中断、再開、断念を表現できるようにする。ただし、最初から万能な活動DSLを設計しない。Golden Sceneで完成した活動を別の場面で反復し、実際に共通だった制御だけをDaihonの公開機能へ昇格する。

Daihon作者向けの標準日本語合図名は、YuukeiのWorld/Daihon境界でcanonical RuntimeEvent typeへ解決する。Extensionがmanifestで寄贈したsignal aliasも同じ境界で解決する。Daihon coreにYuukei固有signal辞書を焼き込まず、event logやExtensionへは `device.wake` や `ext.<extensionId>.*` などのcanonical IDだけを流す。

複数actorの掛け合いでは、World Packのactor定義に `speakerAliases` を置き、Daihonの短い話者名をYuukeiのWorld/Daihon境界でcanonical actor IDへ解決する。`yuukei-daihon` はWorld Packのactor一覧を知らないままにし、actor存在検証、alias重複検証、RuntimeCommandの `target.actorId` / `payload.speakerId` 正規化は `yuukei-world` 側で行う。

OSのsleep/wake、生活時計tick、時間帯変化、実idleなどの観測はDevice Hostで行う。Resident Homeは受け取った `RuntimeEvent` を記録してDaihonへ渡すだけにし、Tauri、AppKit、OS通知APIを内部へ入れない。

### 6. Repeat, Then Abstract

一つ目のGolden Sceneが実画面で成立したら、同じ仕組みを別の場面で使う。

- ウィンドウへの身体反応を、座る以外の場面でも使う。
- 活動の中断・再開を、Downloads遭遇と留守番の両方で使う。
- 関心対象と複数段階の行動を、音楽と設定隠しの両方で使う。

二つ以上の具体的な場面で共通したものだけを、RuntimeEvent、RuntimeCommand、World Pack、Daihon関数、Extension hookとして安定化する。一つの演出しか使わない値まで公開protocolへ持ち上げない。一方、既定住人の固有演出をCoreへ焼き込まず、共通制御とWorld Pack固有データの境界を保つ。

### 7. Official Default Extensions

最後に公式同梱のDefault Extensionを足す。

- `yuukei-intelligence`: Rust常駐プロセスとして配布し、同梱LiteRT-LMモデルで `dialogue.generate`, `dialogue.interpret`, `dialogue.extract`, `memory.index`, `memory.retrieve` を提供する。Node.js、Python、外部AI APIを実行時依存にしない。
- `yuukei-tts`: `speech.synthesis`。
- `yuukei-stt`: `speech.recognition`。

これらはデフォルトで同梱・有効化できるが、Coreには含めない。無効化、差し替え、同じcapabilityを提供する別Extension選択ができるようにする。

## Decision Rules

- 体験契約とGolden Sceneを最上位の受け入れ基準にする。
- 住人の継続性はResident Homeへ置く。
- 端末固有の感覚器と権限はDevice Hostへ置く。
- 表示と演出はSurface Clientへ置く。
- AI、TTS、STT、記憶検索、message加工、外部アプリ連携の入口はExtensionへ置く。
- 出来事のsource of truthはcanonical event logへ置く。
- World Packはデータと台本に限定する。
- Coreには特定の研究成果やAI方式を入れない。
- 最初はWindowsと既定住人 `yuukei` に集中するが、複数住人、別Surface、別OSを不可能にする固定化はしない。
- 抽象化は、具体的な体験を二つ以上実装してから行う。
- 住人の固有の芝居を、未検証の汎用性のために単発発話や単発motionへ縮退させない。

## Common Mistakes

- チャットUIを中心に作り始める。
- LLMの品質を製品の中心に置く。
- Memory Extensionの内部形式をCore schemaに固定する。
- Surfaceが人格や長期状態を持つ。
- Device HostのOS APIをResident Homeへ漏らす。
- Extension同士を直接つなぐ。
- ExtensionをCore内部関数名やmutable内部状態に結びつける。
- World Packから特定Extensionを直接呼ぶ。
- event logを後回しにする。
- 既存MVPの単一runtime構造をそのまま拡張する。
- protocolや設定画面が完成したことを製品体験の完成とみなす。
- `away`、座標追従、idle motionの存在だけで生活を実装したとみなす。
- フォルダを開いた瞬間に住人の活動を初期化し、以前からそこにいたような台詞だけを出す。
- ユーザーの実ファイルや他アプリを演出のために勝手に操作する。

## First Vertical Slice

最初の縦切りはGolden Scene「窓辺の住人」とし、次の構成で実際のWindowsデスクトップまで通す。

1. Resident Homeをローカルプロセスとして起動する。
2. Device Hostが接続し、SurfaceSessionを登録する。
3. Surfaceがsnapshotとcommand streamを購読し、既定住人の継続中の活動を描画する。
4. 住人が対象ウィンドウへ近づき、枠へ座り、くつろぐ。
5. Device Hostが対象ウィンドウの移動、リサイズ、最小化、閉じるを観測してRuntimeEventを送る。
6. Resident Homeが活動の中断または遷移を決め、event logへ保存する。
7. Surfaceがつかまる、姿勢を直す、降りる、飛び退くのいずれかを時間を持つ身体表現として描画する。
8. 対象消滅後も不可視位置や停止状態に残らず、次の活動へ遷移する。

この縦切りでは、canonical event logとは別にアプリ動作ログもJSONLで保存する。event logは住人の生活史のsource of truthであり、アプリ動作ログは起動、Surface attach、入力、エラー、書き出しなどの実装検証に使う。

この縦切りで、通信境界、event log、Surfaceの受動性に加えて、会話なしの自発活動、UIとの身体的因果、中断から次の活動への遷移を確認する。

次の縦切りは「Downloadsで見つかる」とし、画面外活動と `caught in the act`、再会継続を確認する。その後、体験契約の10分受け入れ試験へ二つの場面を統合する。

## Documentation Discipline

新規実装で仕様を足すときは、まずどの境界に属するかを決める。

- 体験契約、Golden Scene、完成判定なら `00-experience-contract.md`。
- 世界観と長期構想なら `01-concept.md`。
- 構造なら `02-architecture.md`。
- messageやRPCなら `03-protocols.md`。
- event logやMemory Extensionなら `04-event-log-and-memory.md`。
- World PackやDaihonなら `05-world-pack-and-daihon.md`。

仕様がどこにも入らない場合、その仕様はYuukeiの中核から外れている可能性が高い。構造上は正しくても体験契約の条件を前進させない作業は、Golden Sceneの阻害要因でない限り後回しにする。
