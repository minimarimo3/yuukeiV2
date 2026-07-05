# Event Log and Memory

Yuukei Coreが持つのは「記憶」ではなく「記録」である。

記憶方式は変化が速い。将来、OSSの記憶エンジン、独自バイナリDB、専用ハードウェア、ローカルLLM、クラウドAI、全く別の検索方式が登場しても差し替えられる必要がある。CoreがSummaries、Facts、Episodes、vector storeのような固定モデルを所有すると、その変化に弱くなる。

## Canonical Event Log

canonical event logは、住人の生活史のsource of truthである。

保存するもの:

- ユーザー入力。
- Surface上のジェスチャーやドラッグ。
- OSや端末の観測イベント。
- Presence tick、起動、スリープ、復帰、終了。
- Daihonが選んだsceneと実行結果。
- RuntimeCommandとして出た発話、動作、UI演出。
- CapabilityInvocationの要求と、許可された範囲の結果メタデータ。
- Extension hookの呼び出し結果、採用されたmessage変換のメタデータ、Extension発eventの正規化結果。

保存しない、または参照化するもの:

- 巨大なファイル本文。
- 無制限の画面内容。
- マイクやカメラの生データ。
- Extension固有の内部DB。
- Extensionの内部状態や実行環境そのもの。
- ユーザーが許可していない個人情報。

Event logは、後から別の記憶エンジンで再indexできるよう、安定した順序、因果関係、residentId、deviceId、surfaceId、event typeを保持する。

Extensionが `RuntimeCommand` を変換した場合、Resident Homeは `extension.hook.result` を記録してから、変換後のcommandを通常の `RuntimeCommand` として記録する。ExtensionがRuntimeEventを提案した場合、Resident Homeは `ext.<extensionId>.` 名前空間、manifestの `emittedEvents`、source、causality、hop countを検証してからcanonical event logへ追記する。Extensionがevent logファイルを直接書き換えることは許可しない。

## Memory Is a Derived Extension Capability

長期記憶はMemory Extensionが作る派生物である。

Extensionはmanifestで許可されたevent log範囲を読み、好きな方式で記憶を構築できる。

- 要約DB。
- facts/episodes分類。
- vector index。
- graph memory。
- 独自バイナリ形式。
- ローカルLLM用キャッシュ。
- クラウド検索サービス。

Yuukei Coreは、それらの内部構造を知らない。Coreは `memory.index`、`memory.retrieve`、`memory.forget`、`memory.rebuild` のようなcapabilityを、選択されたExtensionへルーティングするだけにする。

最初の契約は次の2つに限定する。

- `memory.index`: Resident Homeが日単位のevent log抜粋を渡し、Memory Extensionが内部DBへ統合する。入力は `residentId`、`worldPackId`、`date`、最小payload化されたevent一覧であり、出力は `indexed` と任意の `noteCount` だけである。
- `memory.retrieve`: Resident Homeが発話生成前に短いqueryと件数上限を渡し、Memory Extensionが `fact` / `episode` の短いテキスト断片を返す。Coreはその内部根拠や保存形式を知らず、`dialogue.generate` にはtext配列として渡す。

統合は遅延実行でよい。Resident Homeは `app.startup` と `device.sleep.before` を受けたとき、event log上の `memory.index` 成功記録を見て、今日より前の未統合日を直近7日分までMemory Extensionへ渡す。失敗やprovider未登録はCoreの動作を止めず、次回の起動やスリープ前に再試行される。

想起には小さな予算を置く。発話生成の直前に `memory.retrieve` を呼び、facts最大10件、episodes最大5件を `DialogueGenerateInput.memories` へ入れる。retrieveの失敗、timeout、provider未登録は、記憶なしの発話生成として扱う。`dialogue.interpret` は機械的判定なので記憶を受け取らない。

外部プロセスExtensionが派生記憶DBを持つ場合、Device Hostは `YUUKEI_DATA_DIR/extension-data/<extensionId>` を作成し、起動時に `YUUKEI_EXTENSION_DATA_DIR` として渡す。この領域はExtension再インストール用のコピー先とは別であり、Extensionの内部DBはcanonical event logから再構築できる派生物として扱う。

公式 `yuukei-intelligence` の初期実装は、`YUUKEI_EXTENSION_DATA_DIR/memory/<worldPackId>/<residentId>/facts.json` と `episodes.jsonl` に派生記憶を保存する。世界観のツマミはExtension側にあり、恒久ノートは最大50件、取り出し予算はfacts 10件 / episodes 5件、Resident Homeからの遡り統合は直近7日、episodeの新しさ減衰は半減期14日である。

Extensionが再起動後に内部状態を再構築する場合、manifestでevent log読み出し権限を宣言する。

```ts
type EventLogReadGrant = {
  extensionId: string;
  residentId: string;
  eventTypes: string[];
  privacyCategories: string[];
  cursorAfterSequence?: number;
  untilTimestamp?: string;
  maxRecords: number;
  allowPayloads: boolean;
  allowReferences: boolean;
  expiresAt: string;
  purpose: string;
};
```

Resident Homeはgrantを検証し、許可されたevent type、privacy category、payload/reference範囲だけを返す。Extensionは読み出したlogから内部DBを作ってよいが、canonical event logそのものの所有者にはならない。

## Official Intelligence Extension

公式のLLM/Memory Extensionは用意してよい。ただし、それは推奨実装であってCoreではない。

公式Extensionの役割:

- `dialogue.generate` を提供する。
- canonical event logを読んで記憶索引を作る。
- 現在文脈に必要な記憶を検索する。
- World Packの口調や人格に合わせて発話を生成する。
- 必要ならTTSやembedding capabilityと連携する。

公式Extensionを無効化しても、Daihon、World Pack、Surface、event logは動く。別のMemory/LLM Extensionへ差し替えても、住人の生活史はevent logから再構築できる。

## Privacy and Consent

Event logは強力な生活記録であるため、Coreが権限、保存期間、削除、エクスポートを管理する。

必須方針:

- どの種類のイベントを保存するか、ユーザーが確認できる。
- 機微な観測は明示的な権限を必要とする。
- ユーザーはevent logをエクスポートできる。
- ユーザーは期間指定、種類指定、全削除ができる。
- Extensionが作った派生記憶も削除・再構築できる。
- Extensionは許可されたlog範囲だけを読める。

安全説明を世界観に完全に隠してはいけない。ただし、日常体験中に説明過多なUIで没入を壊さない。設定や初回導入で明確に説明し、通常時は生活表現へ変換する。

## Event Log Record

最小概念:

```ts
type EventLogRecord = {
  sequence: number;
  id: string;
  type: string;
  timestamp: string;
  residentId: string;
  source: string;
  deviceId?: string;
  surfaceId?: string;
  actorId?: string;
  payload: Record<string, unknown>;
  causality?: {
    sourceEventId?: string;
    sourceCommandId?: string;
    traceId?: string;
  };
  privacy?: {
    category: string;
    retention: "session" | "short" | "long" | "manual";
    extensionReadable: boolean;
  };
};
```

`payload` は意味を持つ最小データにする。重い観測やファイル内容はreference化する。

## Reindexing

Memory Extensionを交換したとき、Coreはevent logを再生して新Extensionへ `memory.rebuild` を依頼できるべきである。

再index時の考え方:

- Coreはevent logを順序付きで提供する。
- Extensionは自分の内部DBを破棄して再構築してよい。
- 途中失敗してもcanonical event logは壊れない。
- Extension固有の移行はExtension責任。

この構造により、Yuukeiは特定の記憶研究や実装に賭けず、将来の良い方式を取り込める。
