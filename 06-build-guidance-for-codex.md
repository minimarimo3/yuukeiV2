# Build Guidance for Codex

この文書は、Yuukeiを一から実装するCodexおよび実装者向けの判断基準である。既存MVPのクラス名、ディレクトリ構造、単一AppRuntimeは新規設計の制約としない。継承するのはプロダクト思想と意味境界である。

## Build Order

### 1. Protocol and Event Log First

最初に作るもの:

- RuntimeEvent。
- RuntimeCommand。
- ResidentSnapshot。
- SurfaceSession。
- CapabilityInvocation。
- ExtensionHookInvocation。
- ExtensionHookResult。
- EventLogRecord。
- canonical event logのappend/read/export/deleteの最小実装。

この段階ではUIを完了条件に含めない。CLIやテストからeventを入力し、commandが出力されることを確認する。

### 2. Headless Resident Home

Resident HomeをTauri非依存で作る。

最初の責務:

- World Packを読む。
- signal allowlistを確認する。
- Daihon Hostを呼ぶか、最小Daihon adapterでsceneを返す。
- event logへ記録する。
- command streamを購読者へ流す。
- capability routerの最小実装を持つ。
- extension hook pipelineの最小実装を持つ。

禁止:

- Tauri AppHandleをResident Homeへ入れる。
- OS window handleをResident Homeへ入れる。
- rendererやWebViewに依存する。

### 3. Minimal Surface Client

最初のSurfaceは、protocol境界の検証に必要な機能だけを持つ。

- snapshotを取得する。
- command streamを購読する。
- `dialogue.say`、`avatar.expression`、`avatar.motion` のような最小commandを表示する。
- Daihonが開始した場面で必要な場合だけ、文脈付きユーザー入力をprotocol eventとして送る。常設チャット欄は作らない。

VRMやLive2Dは後続段階で実装する。この段階ではHTMLまたは簡単なcanvasを使い、Surfaceが人格を持たずcommandの描画だけを担うことを確認する。

開発時はCLI Surfaceを既定とする。CLIは番号入力の状態機械REPL(03の「CLI Surfaceの番号入力状態機械」が仕様の正)で、手動確認にもパイプ入力(例: `printf '1\n2\n1\n0\n' | yuukei-cli-surface`)による機械的テストにも同じ経路を使う。リリース向けの既定SurfaceはTauri版Desktop Surfaceにする。

### 4. Device Host

Device Hostはローカル端末の感覚器と能力ホストである。

最初の責務:

- Resident Homeへ接続する。
- Surfaceを登録する。
- ローカルExtensionを登録する。
- Extensionがmanifestで宣言したcapability、hook、event購読、event発行、signal aliasをResident Homeへ登録する。
- 端末のpresence、生活時計tick、実idle、起動、終了をRuntimeEventとして送る。
- ユーザーが選んだWorld Packディレクトリをローカル設定に保存し、選択されたPack installに対応するResident Home起動設定を作る。

OS観測は段階的に増やす。Finder/Explorer、ファイル、通知、スマホセンサーなどは、すべてDevice Host側の拡張として扱う。

初期のExtension実装は `beforeCommandEmit` に限定できる。ただし、同じmanifestモデルでcapability提供、`onEventAppended`、RuntimeEvent発行、Daihon signal alias寄贈を扱える設計とする。外部プロセス型Extensionは、Device Hostが設定画面で選ばれたフォルダを `YUUKEI_DATA_DIR/extensions/<extensionId>/` へコピーし、`manifest.json` と `YUUKEI_DATA_DIR/settings/extensions.json` を読んでResident Homeへ公開protocol Extensionとして登録する。ExtensionにCore内部状態、Tauri AppHandle、Surface実装、event logファイルを直接渡さない。v1では信頼済みローカルコードとして扱い、manifest permissionsは宣言とユーザー同意であり、OS sandboxを仕様として約束しない。

World Pack選択UIはDevice Hostに置く。ただし、active World Packの解釈、required capability確認、Packごとのresident/event-log分離はResident Home起動境界の責務として扱う。Surface Clientは `ResidentSnapshot.worldPackId` を表示できるが、Pack選択や人格継続性を所有しない。

### 5. Daihon Integration

Daihonはsidecarまたはservice境界として接続する。

- Resident HomeはDaihon内部型へ依存しない。
- Daihon Hostはevent、variables、context、cooldownを受け取る。
- Daihon Hostはcommands、variable patches、executed scenesを返す。
- callbackでruntime queryやcapability invocationを要求できる。

Daihonなしでも最小Resident Homeは起動できるようにする。ただし、製品のキャラクターらしさはDaihonで作る。

Daihon作者向けの標準日本語合図名は、YuukeiのWorld/Daihon境界でcanonical RuntimeEvent typeへ解決する。Extensionがmanifestで寄贈したsignal aliasも同じ境界で解決する。Daihon coreにYuukei固有signal辞書を焼き込まず、event logやExtensionへは `device.wake` や `ext.<extensionId>.*` などのcanonical IDだけを流す。

複数actorの掛け合いでは、World Packのactor定義に `speakerAliases` を置き、Daihonの短い話者名をYuukeiのWorld/Daihon境界でcanonical actor IDへ解決する。`yuukei-daihon` はWorld Packのactor一覧を知らないままにし、actor存在検証、alias重複検証、RuntimeCommandの `target.actorId` / `payload.speakerId` 正規化は `yuukei-world` 側で行う。

OSのsleep/wake、生活時計tick、時間帯変化、実idleなどの観測はDevice Hostで行う。Resident Homeは受け取った `RuntimeEvent` を記録してDaihonへ渡すだけにし、Tauri、AppKit、OS通知APIを内部へ入れない。

### 6. Official Default Extensions

最後に公式同梱のDefault Extensionを足す。

- `yuukei-intelligence`: Rust常駐プロセスとして配布し、同梱LiteRT-LMモデルで `dialogue.generate`, `dialogue.interpret`, `dialogue.extract`, `memory.index`, `memory.retrieve` を提供する。Node.js、Python、外部AI APIを実行時依存にしない。
- `yuukei-tts`: `speech.synthesis`。
- `yuukei-stt`: `speech.recognition`。

これらはデフォルトで同梱・有効化できるが、Coreには含めない。無効化、差し替え、同じcapabilityを提供する別Extension選択ができるようにする。

## Decision Rules

- 住人の継続性はResident Homeへ置く。
- 端末固有の感覚器と権限はDevice Hostへ置く。
- 表示と演出はSurface Clientへ置く。
- AI、TTS、STT、記憶検索、message加工、外部アプリ連携の入口はExtensionへ置く。
- 出来事のsource of truthはcanonical event logへ置く。
- World Packはデータと台本に限定する。
- Coreには特定の研究成果やAI方式を入れない。

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

## First Vertical Slice

最初の縦切りは次の構成とする。

1. Resident Homeをローカルプロセスとして起動する。
2. Device Hostが接続し、SurfaceSessionを登録する。
3. Surfaceがsnapshotとcommand streamを購読する。
4. ユーザーがテキストを送る。
5. RuntimeEventがevent logへ保存される。
6. 最小Daihon adapterが `dialogue.say` を返す。
7. Surfaceが発話を表示する。
8. 公式ではない簡易TTS Extensionがあれば音声化する。

この縦切りでは、canonical event logとは別にアプリ動作ログもJSONLで保存する。event logは住人の生活史のsource of truthであり、アプリ動作ログは起動、Surface attach、入力、エラー、書き出しなどの実装検証に使う。

この縦切りで、通信境界、event log、Surfaceの受動性、Extensionの交換可能性を確認する。

## Documentation Discipline

新規実装で仕様を足すときは、まずどの境界に属するかを決める。

- 体験原則なら `01-concept.md`。
- 構造なら `02-architecture.md`。
- messageやRPCなら `03-protocols.md`。
- event logやMemory Extensionなら `04-event-log-and-memory.md`。
- World PackやDaihonなら `05-world-pack-and-daihon.md`。

仕様がどこにも入らない場合、その仕様はYuukeiの中核から外れている可能性が高い。
