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
- Extension hookの呼び出し結果と、採用されたmessage変換のメタデータ。

保存しない、または参照化するもの:

- 巨大なファイル本文。
- 無制限の画面内容。
- マイクやカメラの生データ。
- Provider固有の内部DB。
- Extensionの内部状態や実行環境そのもの。
- ユーザーが許可していない個人情報。

Event logは、後から別の記憶エンジンで再indexできるよう、安定した順序、因果関係、residentId、deviceId、surfaceId、event typeを保持する。

Hook Extensionが `RuntimeCommand` を変換した場合、Resident Homeは `extension.hook.result` を記録してから、変換後のcommandを通常の `RuntimeCommand` として記録する。Extensionがevent logファイルを直接書き換えることは許可しない。

## Memory Is a Derived Capability

長期記憶はCapability Providerが作る派生物である。

Providerはevent logを読み、好きな方式で記憶を構築できる。

- 要約DB。
- facts/episodes分類。
- vector index。
- graph memory。
- 独自バイナリ形式。
- ローカルLLM用キャッシュ。
- クラウド検索サービス。

Yuukei Coreは、それらの内部構造を知らない。Coreは `memory.index`、`memory.retrieve`、`memory.forget`、`memory.rebuild` のようなcapabilityを呼ぶだけにする。

## Official Intelligence Extension

公式のLLM/Memory Extensionは用意してよい。ただし、それは推奨実装であってCoreではない。

公式Extensionの役割:

- `dialogue.generate` を提供する。
- canonical event logを読んで記憶索引を作る。
- 現在文脈に必要な記憶を検索する。
- World Packの口調や人格に合わせて発話を生成する。
- 必要ならTTSやembedding providerと連携する。

公式Extensionを無効化しても、Daihon、World Pack、Surface、event logは動く。別のMemory/LLM Providerへ差し替えても、住人の生活史はevent logから再構築できる。

## Privacy and Consent

Event logは強力な生活記録であるため、Coreが権限、保存期間、削除、エクスポートを管理する。

必須方針:

- どの種類のイベントを保存するか、ユーザーが確認できる。
- 機微な観測は明示的な権限を必要とする。
- ユーザーはevent logをエクスポートできる。
- ユーザーは期間指定、種類指定、全削除ができる。
- Providerが作った派生記憶も削除・再構築できる。
- Providerは許可されたlog範囲だけを読める。

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
    providerReadable: boolean;
  };
};
```

`payload` は意味を持つ最小データにする。重い観測やファイル内容はreference化する。

## Reindexing

記憶Providerを交換したとき、Coreはevent logを再生して新Providerへ `memory.rebuild` を依頼できるべきである。

再index時の考え方:

- Coreはevent logを順序付きで提供する。
- Providerは自分の内部DBを破棄して再構築してよい。
- 途中失敗してもcanonical event logは壊れない。
- Provider固有の移行はProvider責任。

この構造により、Yuukeiは特定の記憶研究や実装に賭けず、将来の良い方式を取り込める。
