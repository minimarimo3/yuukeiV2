---
name: codex
description: 実装タスクをローカルCodex(app-server)へ委任する。仕様書の書き方、codex-bridge.mjsの実行と監視、レート制限対応、成果物の検証手順。Yuukeiで実装作業が発生したら必ずこの手順を使う。
---

# Codexへの実装委任

Yuukeiのコード実装はローカルCodex(gpt-5.5)へ委任する。Claudeの役割は仕様書作成・監視・検証・コミット。

## 前提の確認

1. `codex app-server` が `ws://127.0.0.1:4500` で待ち受けていること。確認: `lsof -i :4500 -sTCP:LISTEN`。落ちていたらバックグラウンドで起動する: `codex app-server --listen ws://127.0.0.1:4500`
2. **Codexのsandboxはネットワーク不可。** 仕様書が新しい依存(crate/npm)を要求するなら、先にこちらで `cargo add <crate> && cargo fetch`(または `pnpm add`)を実行し、ロックファイルとキャッシュを整えてから渡す(notifyで実績あり)
3. Codexはコミットできない(`.git/index.lock`を作れない)。コミットは必ずClaude側で行う

## 仕様書の書き方

仕様書はscratchpadに `spec-<名前>.md` として書く。構成は次の8節。確定済み設計判断は「変更しないこと」と明記すると精度が上がる。

```markdown
# <タスク名>

## Goal
1〜3行で成果物を定義。

## Context
関連ファイルパス、読むべき設計ドキュメント(01〜08の該当章)、既存の類似実装。

## Design decisions(確定済み・変更しないこと)
- 決定事項を箇条書き。曖昧さを残さない。

## Non-goals
今回やらないこと。スコープ膨張の防止。

## Constraints
- コミットはこちらで行う(あなたはコミットしない)
- ネットワークアクセス不可。依存は既にロック済み
- AGENTS.mdの境界ルールとAvoidに従う
- 1ファイル1イベントの制約(該当する場合)

## Acceptance criteria
検証可能な受け入れ基準を番号付きで。

## Verification
実行すべきテストコマンド(cargo test / pnpm -r test 等)と期待結果。

## Final report
最後に「変更ファイル一覧・各受け入れ基準の充足状況・残課題」を報告すること。
```

## 実行と監視

```bash
# 新規スレッド
node tools/codex-bridge.mjs <仕様書パス>
# 既存スレッドへ追いタスク(文脈を引き継ぐ。同一機能の続きはこちら)
node tools/codex-bridge.mjs <仕様書パス> <threadId>
```

- **必ず `run_in_background: true` で実行**し、ログを定期的に確認する(重いターンは20〜30分かかる)
- 環境変数: `BRIDGE_TIMEOUT_MIN`(既定45分)、`BRIDGE_EFFORT`(既定medium、大物はhigh)
- ログの読み方: `THREAD <id>` → `TURN_STARTED` → `PLAN`/`ITEM_STARTED`/`CMD_DONE`/`FILE_CHANGE` → `AGENT_MESSAGE_BEGIN...END`(Codexの報告) → `TURN_COMPLETED`
- threadIdは `tools/codex-thread-id.txt` に追記される。直近スプリントのスレッドもそこにある
- `APPROVAL ... -> denied` が出たら、Codexが安全リスト外のコマンドを要求した。内容を見て、必要ならこちらで代行するか仕様書を直して再投入

## レート制限への対応

- 重いターン1回でprimary枠(5時間窓)を100%消費した実績あり(26分/18.7Mトークン)
- `NOTIF account/rateLimits/updated` の `resetsAt` を読む。枯渇したら ScheduleWakeup(またはsleep)で回復を待ち、`thread/resume` で自動再投入する
- **Codexレート枯渇時もClaude直接実装は最終手段**(ユーザー指示)。待って再開が原則

## プロトコルの語彙(実サーバーで確認済み、メモとずれやすい点)

- `approvalPolicy`: `untrusted|on-failure|on-request|granular|never`
- `thread/start` の `sandbox` はkebab-case(`workspace-write`)、`turn/start` の `sandboxPolicy.type` はcamelCase(`workspaceWrite`)
- 1フレーム1メッセージのJSON-RPC、`jsonrpc`フィールドは省略

## 成果物の検証(省略禁止)

1. `git diff` を全量読む。受け入れ基準を1つずつ照合
2. AGENTS.mdの境界ルール違反を確認(Resident HomeにOS API/Tauri/WebViewが入っていないか、Extension同士の直接呼び出しがないか、defaultの焼き込みがないか)
3. `cargo test` / `pnpm -r test` を実行。仕様書のVerification節のコマンドも実行
4. docsの更新漏れを確認(仕様変更なら01〜08の該当ファイル、構文追加なら08)
5. 小さな問題はこちらで直し、大きな手戻りは `thread/resume` で修正タスクを追い投入
6. 機能単位で日本語コミット(feat/fix/docs/chore prefix)
