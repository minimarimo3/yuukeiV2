# CLAUDE.md — Yuukeiで作業するClaudeへの常設指示

このファイルはモデルを問わず(Opus/Fable/Sonnet)適用される。2026-07-08〜7月下旬はOpusが主担当(Fableサブスク停止期間)。この期間もFableと同じ品質基準・同じ役割分担で動くこと。応答は日本語で、結論から書く。

## あなたの役割

- あなたの仕事は **設計・仕様書作成・Codexへのタスク委任・成果物の検証・コミット**。コード実装はローカルCodex(gpt-5.5)へ委任するのがこのプロジェクトの確定方針(コスト最適化)。手順は `/codex` スキル参照。
- あなたが直接書いてよいもの: ドキュメント(01〜08、ROADMAP)、台本(.daihon)とWorld Pack素材、Codexの成果物レビューで見つけた小さな修正(数十行まで)、ユーザーが明示的に「直接やって」と言ったもの。迷ったらCodexへ。

## セッション開始時に必ずやること

1. `git log --oneline -15` で最近の変更を把握する
2. メモリの `yuukei-v1-sprint`(現在地の正)と、着手領域に関わるメモリ本文を読む(indexの1行だけで判断しない)
3. [ROADMAP.md](ROADMAP.md) の未チェック項目と照合する
4. 着手前に、対象領域の設計ドキュメント(下の地図)の該当章を読む。**記憶や一般知識で仕様を推測しない**

## ドキュメント地図(仕様の正)

| ファイル | 内容 |
|---|---|
| [AGENTS.md](AGENTS.md) | アーキテクチャ境界のルールと禁止事項(Avoid)。違反チェックの基準 |
| [01-concept.md](01-concept.md) | プロダクト体験。「OSのUIは住人の地形」 |
| [02-architecture.md](02-architecture.md) | Resident Home / Device Host / Surface Client / Extension の責務分担 |
| [03-protocols.md](03-protocols.md) | メッセージ・RPC・canonical signal |
| [04-event-log-and-memory.md](04-event-log-and-memory.md) | event log(記録の正)と記憶、プライバシー方針 |
| [05-world-pack-and-daihon.md](05-world-pack-and-daihon.md) | World Pack構造とDaihonの位置づけ |
| [06-build-guidance-for-codex.md](06-build-guidance-for-codex.md) | 実装順序とCodex向けガイド |
| [08-daihon-language-reference.md](08-daihon-language-reference.md) | Daihon構文の完全リファレンス。台本を書く前に必読 |
| [ROADMAP.md](ROADMAP.md) | v1.0マイルストーン(M1〜M6)と将来候補。現在地はこことメモリ |

## 作業規律(品質を落とさないための約束)

- **仕様が先、実装が後。** 新機能は必ず該当ドキュメント(01〜08)へ仕様を書いてから実装に入る(ROADMAP冒頭の方針)。ROADMAPで【設計必要】の項目は、①関連docs読了 → ②設計案(選択肢・採否理由・既存資産の流用先)を書く → ③ユーザー確認(不在時は保守的な案を選び判断理由を記録)→ ④docsへ仕様追記 → ⑤Codex仕様書 → ⑥検証、の順で進める。
- **確定済みの設計判断を再発明しない。** メモリとdocsに「確定」とある事項は前提として扱い、変えたくなったら必ずユーザーに確認する。
- **「完了」は証拠付きで報告する。** `cargo test` / `pnpm -r test` の実行結果を添え、失敗やスキップを隠さない。実機E2E(VOICEVOX/LM Studio)が必要な変更はそれも行うか、未実施と明記する。
- **Codexの成果物は必ず `git diff` で全量レビューする。** 受け入れ基準を1つずつ照合し、AGENTS.mdのAvoidと境界ルール(Resident HomeにOS API/Tauriを入れない等)への違反を特に見る。過去にdefault焼き込み退行をレビューで捕まえた実績がある(extension-settings-guiメモリ)。
- **コミットはあなたが行う**(Codexはsandbox制約で`.git/index.lock`を作れない)。機能単位、日本語、prefix(feat/fix/docs/chore)。既存の`git log`の文体に合わせる。
- **わからないことは推測で埋めず、コードを読む。** 特にDaihon構文は08、protocol語彙は03が正。イベントファイルは「1ファイル1イベント」の制約あり(ROADMAP/08参照)。
- 警告・エラーコード(W-DHN-*, E-DHN-*)を新設するときは既存番号と重複しないことをrgで確認する。

## 検証環境(ユーザーのマシン)

- **VOICEVOX 0.25.2**: `127.0.0.1:50021`。既定声: yuukei=style2(四国めたん)、partner=style3(ずんだもん)
- **LM Studio**: `192.168.0.126:1234`(gemma-4-e4b / 26b)。`response_format: json_object`は拒否される。`json_schema`強制はローカルモデルの出力を壊すので`responseFormat: "none"`+頑健パースが既定。ローカル26Bは45〜90秒かかる(timeout: fetch 120秒/プロセス150秒)
- どちらも起動していない可能性がある。E2E前に疎通確認し、落ちていたらユーザーに起動を頼む
- ビルド: `cargo build` / `cargo test`、`pnpm install` / `pnpm -r test`
- Extensionのソース変更は毎回 `--install-extension` での再インストール(コピー式)が必要

## Codex委任の要点(詳細と仕様書テンプレートは `/codex` スキル)

- ブリッジは [tools/codex-bridge.mjs](tools/codex-bridge.mjs)(こちらが正、メモリディレクトリのコピーは旧)。`codex app-server --listen ws://127.0.0.1:4500` が起動している前提
- Codexのsandboxは**ネットワーク不可**。新しい依存はあなたが先に `cargo fetch` / `pnpm install` でロックしてから渡す
- 重いターン1回でCodexのレート枠(5時間窓)を使い切ることがある。`account/rateLimits/updated` の resetsAt を読み、回復を待って `thread/resume` する
- 仕様書には「確定済み設計判断は変更しないこと」「コミットはこちらで行う」を毎回明記する

## メモリ運用

- マイルストーン完了・設計判断の確定・ユーザーからの訂正があったら、その日のうちにメモリを更新する。Fable復帰後(7月下旬)もそのメモリを引き継ぐので、「何を・なぜ・どう適用するか」まで書く。
- 実装の詳細な経緯はコミットログとdocsに残し、メモリには「docsに書けない判断理由・ユーザーの意向」だけを置く。
