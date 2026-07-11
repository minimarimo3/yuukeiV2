---
name: codex
description: 実装タスクをローカルCodex(codex@openai-codexプラグインの/codex:rescue)へ委任する。仕様書の書き方、モデル/努力度の使い分け、実行と監視、成果物の検証手順。Yuukeiで実装作業が発生したら必ずこの手順を使う。
---

# Codexへの実装委任

Yuukeiのコード実装はローカルCodexへ委任する。Claudeの役割は仕様書作成・監視・検証・コミット。委任は `codex@openai-codex` プラグインの `/codex:rescue`(内部で `codex:codex-rescue` サブエージェント → `codex-companion.mjs task` を叩く)を使う。ランタイムは初回コマンドでオンデマンド起動されるので、事前起動やポート待ち受けの確認は不要。

## モデル選択(ユーザー確定方針、2026-07-11)

タスクの難度で使い分ける。`--model` と `--effort` を `/codex:rescue` に渡す(`gpt-5.6-terra` / `gpt-5.6-sol` はそのまま `--model` に渡せる正式モデルID)。

| タスク | モデル | 指定 |
|---|---|---|
| 基本(既定) | gpt-5.6 Terra High | `--model gpt-5.6-terra --effort high` |
| 難しいタスク(設計をまたぐ大物、境界が繊細な変更) | gpt-5.6 Sol High | `--model gpt-5.6-sol --effort high` |
| 単純作業(機械的な置換、fixture更新など) | gpt-5.6 Terra Medium | `--model gpt-5.6-terra --effort medium` |

`--effort` の取りうる値は `none|minimal|low|medium|high|xhigh`。

## 前提の確認

1. Codexが利用可能で認証済みであること。未確認なら `/codex:setup` で確認する(初回のみ)
2. **Codexのsandboxはネットワーク不可。** 仕様書が新しい依存(crate/npm)を要求するなら、先にこちらで `cargo add <crate> && cargo fetch`(または `pnpm add`)を実行し、ロックファイルとキャッシュを整えてから渡す
3. Codexはコミットできない(`.git/index.lock`を作れない)。コミットは必ずClaude側で行う
4. `/codex:rescue` は既定で書き込み可(`--write` 相当)。実装タスクはそのままでよい

## 仕様書の書き方

仕様書はscratchpadに `spec-<名前>.md` として書き、**その本文をそのまま `/codex:rescue` のタスクテキストとして渡す**(Codexはリポジトリ外のscratchpadを読めないことがあるため、パスを渡すのではなく本文を渡す)。構成は次の8節。確定済み設計判断は「変更しないこと」と明記すると精度が上がる。

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

- 委任: `codex:codex-rescue` サブエージェント(Agentツールの `subagent_type: "codex:codex-rescue"`)に、先頭へモデル/努力度フラグを付けた仕様書本文をpromptとして渡す。例: `--model gpt-5.6-terra --effort high\n\n<仕様書本文>`。ユーザーが自分で叩く場合は `/codex:rescue --model ... --effort ... <仕様書本文>`
- 重いターンは20〜30分かかる。長時間タスクは `--background` を付けて背景実行し、`/codex:status` で進捗、完了後 `/codex:result` で最終出力を確認する(前景で待つなら `--wait`)
- 同一機能の続き・追いタスクは `--resume`(直近スレッドを継続)。まっさらからやり直すときは `--fresh`
- 承認要求で安全リスト外のコマンドが拒否されたら、内容を見て必要ならこちらで代行するか仕様書を直して再投入する

## レート制限への対応

- 重いターン1回でprimaryレート枠(5時間窓)を使い切ることがある
- 枯渇したら時間を置き、回復後に `--resume` で追タスクを再投入する。回復待ちは ScheduleWakeup を使うとよい
- **Codexレート枯渇時もClaude直接実装は最終手段**(ユーザー指示)。待って再開が原則

## 成果物の検証(省略禁止)

1. `git diff` を全量読む。受け入れ基準を1つずつ照合
2. AGENTS.mdの境界ルール違反を確認(Resident HomeにOS API/Tauri/WebViewが入っていないか、Extension同士の直接呼び出しがないか、defaultの焼き込みがないか)
3. `cargo test` / `pnpm -r test` を実行。仕様書のVerification節のコマンドも実行
4. docsの更新漏れを確認(仕様変更なら01〜08の該当ファイル、構文追加なら08)
5. 小さな問題はこちらで直し、大きな手戻りは `--resume` で修正タスクを追い投入
6. 機能単位で日本語コミット(feat/fix/docs/chore prefix)
