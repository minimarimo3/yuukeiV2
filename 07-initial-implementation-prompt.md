# Initial Implementation Prompt

この文書は、新しいYuukeiの最初の実装をCodexに依頼するときの貼り付け用プロンプトである。

目的は、既存MVPの修正ではなく、`` を正本として、意味境界が崩れない最初の縦切りを作ること。サブエージェントが使える環境では、境界ごとに並列調査・実装し、メインエージェントが契約と統合を管理する。

## Copy-Paste Prompt

```text
このリポジトリで、新しいYuukeiの最初の縦切りを実装してください。

既存コードは参考にしてよいですが、クラス名、ディレクトリ構造、単一runtime構造、互換レイヤーには引きずられないでください。プロジェクトは未公開なので、より良い設計になる破壊的変更は許容します。

最初に必ず読んでください:

- `AGENTS.md`
- AGENTS.md`
- README.md`
- 01-concept.md`
- 02-architecture.md`
- 03-protocols.md`
- 04-event-log-and-memory.md`
- 05-world-pack-and-daihon.md`
- 06-build-guidance-for-codex.md`

守るべき中心:

- Yuukeiは、ユーザーのデジタル生活圏に住むUI内生活者のためのplatformです。
- `Resident Home` は住人の継続性、canonical event log、capability routing、surface protocolを持つ中核です。
- `Device Host` は端末ごとのOS観測、ローカル権限、ローカルprovider起動、Surface管理を担当します。
- `Surface Client` は身体と演出です。人格、長期状態、capability選択を持たせないでください。
- `Capability Provider` はLLM、TTS、STT、Memory、Embeddingなどの交換可能な能力です。公式同梱providerもCoreではありません。
- Coreが持つのは「記録」です。記憶DB、要約、facts、episodes、embedding index、独自バイナリ形式はproviderが作る派生物です。
- Daihonは作者が意図した生活イベントを実行する層です。AIは台本の代替ではなく余白を埋めるcapabilityです。

今回のゴール:

「テキスト入力がRuntimeEventとして入る -> canonical event logへ保存される -> Resident Homeが最小Daihon adapterまたは仮scene adapterを通じて `dialogue.say` RuntimeCommandを生成する -> Surface Clientがsnapshotとcommand streamを購読して表示する」

ここまでを、境界が分かれた状態で動くようにしてください。

最初に作るべきもの:

1. Protocol and Event Log
   - `RuntimeEvent`
   - `RuntimeCommand`
   - `ResidentSnapshot`
   - `SurfaceSession`
   - `CapabilityInvocation`
   - `EventLogRecord`
   - append/read/export/delete可能な最小canonical event log

2. Headless Resident Home
   - Tauri非依存のRust coreとして作る
   - World Pack読み込みの最小形
   - signal allowlistの最小形
   - event logへのappend
   - snapshot取得
   - command stream購読
   - capability routerの最小形
   - 仮Daihon adapterで `conversation.text` から `dialogue.say` を返せる

3. Minimal Surface Client
   - snapshot取得
   - command stream購読
   - `dialogue.say` を表示
   - ユーザー入力を `conversation.text` として送信
   - Surfaceは人格や長期状態を所有しない

4. Minimal Device Host
   - Resident Homeへ接続
   - SurfaceSession登録
   - `presence`、`device`、ユーザー入力の最小RuntimeEvent送信
   - TauriやOS APIはここに閉じ込める

5. Minimal Capability Provider Stub
   - provider登録の最小schema
   - `speech.synthesis` stubが、Daihon由来の文でも将来のLLM由来の文でも同じ入力を受け取れる形
   - 今回は本格TTSやLLMは不要。交換可能な境界だけ作る

サブエージェントを使用して次のように並列化してください。メインエージェントは必ず最初に共通protocolと所有境界を決め、各サブエージェントの成果を統合してください。

- Subagent A: Protocol/Event Log
  - `03-protocols.md` と `04-event-log-and-memory.md` を読み、Rust/TypeScript境界で使える最小型とevent log APIを実装する。
  - 出力: 型、保存API、単体テスト、未決事項。

- Subagent B: Resident Home
  - `02-architecture.md` と `06-build-guidance-for-codex.md` を読み、Tauri非依存のheadless coreを実装する。
  - 出力: event受付、snapshot、command stream、capability router、仮Daihon adapter、単体テスト。

- Subagent C: Device Host and Surface Client
  - `02-architecture.md` と `03-protocols.md` を読み、最小UIと接続部分を実装する。
  - 出力: SurfaceSession登録、snapshot購読、command表示、ユーザー入力送信。Surfaceに人格を持たせないこと。

- Subagent D: World Pack and Daihon Boundary
  - `05-world-pack-and-daihon.md` を読み、World Pack最小schema、signal allowlist、Daihon Host adapter境界を設計・実装する。
  - 出力: 仮adapter、本物Daihon Hostへ差し替えられるinterface、サンプルWorld Pack。

- Subagent E: Capability Provider Boundary
  - `03-protocols.md` と `04-event-log-and-memory.md` を読み、provider registryとstub providerを実装する。
  - 出力: provider登録、capability invocation、`speech.synthesis` stub、将来のMemory/LLM providerがevent logを読むための権限境界メモ。

サブエージェント間のルール:

- 同じファイルを複数サブエージェントが同時編集しないようにしてください。
- 共通型はSubagent Aが作り、他のサブエージェントはそれに合わせてください。
- 迷ったら新しい互換層を作らず、メインエージェントに境界判断を戻してください。
- Provider同士を直接つなげないでください。CompositionはResident Homeのcapability routerを通してください。
- Device HostのOS APIやTauri handleをResident Homeに入れないでください。
- Surface Clientに人格、記憶、Daihon実行、capability選択を持たせないでください。

実装の置き場所:

- 既存構成を調査したうえで、最も自然な場所を提案してから実装してください。
- ただし、`Resident Home` はTauri非依存のRust crate/moduleとして独立させてください。
- 思想に合わない既存設計へのshimは作らないでください。

最低限の受け入れ条件:

- Headless testで、`conversation.text` RuntimeEventを入れるとevent logに残り、`dialogue.say` RuntimeCommandが出る。
- Surface testまたは簡易起動で、snapshotを受け取りcommandを表示できる。
- Device Host/Surface側から送った入力がResident Homeへ届く。
- EventLogRecordに `id`, `type`, `timestamp`, `source`, `residentId`, `payload`, `causality` を持たせる。
- SurfaceSessionで `surfaceId`, `deviceId`, `kind`, `active`, `capabilities`, `presentation` を扱う。
- CapabilityInvocationがprovider registryを通る。
- Tauri型、WebView、window handle、OS APIがResident Homeに入っていない。
- Memory/LLM/TTSがCore固定実装になっていない。

検証:

- 変更した言語ごとのformat/typecheck/testを実行してください。
- 少なくともheadless coreの単体テストを追加してください。
- 実行できない検証がある場合は、理由と代替確認を明記してください。
- 最後に、どの境界をどのファイル/モジュールに置いたかを短く説明してください。

完了報告には次を含めてください:

- 実装した縦切りの流れ
- 作成/変更した主要ファイル
- 各境界の責務配置
- 実行した検証コマンドと結果
- 次にサブエージェントへ切るべき作業
```

## Notes for the Requester

このプロンプトは、最初の実装を「全部入りアプリ」ではなく、境界が壊れていない縦切りに絞るためのもの。

最初から本格的なLLM、TTS、Memory DB、VRM描画まで入れようとすると、境界が曖昧になりやすい。初回は、`conversation.text` から `dialogue.say` までが通ること、Surfaceが受動的であること、event logがsource of truthであること、Capability Providerが差し替え可能であることを優先する。

サブエージェントを使う場合も、最初の共通protocolだけはメインエージェントが握る。ここが揺れると全員が似たような型を別々に作ってしまう。
