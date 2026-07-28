# yuukei-intelligence

Yuukei公式のローカルIntelligence Extensionです。Rust製の常駐プロセスから
LiteRT-LM 0.14.0のC APIを呼び、同梱されたGemma 4 E4Bを次の限定用途に使います。

- Daihonが指示した短い台詞の補完
- Daihon内で受け取った曖昧な入力の選択肢判定・値抽出
- 日次memory indexと気分評価

常設チャットや自由会話の生成器ではありません。`dialogue.generate`はDaihon由来の
`instruction`がない呼び出しを無言で拒否します。

## Windowsパッケージ

```powershell
pnpm --filter @yuukei/intelligence package:windows -- `
  -ModelPath C:\path\to\gemma-4-E4B-it.litertlm
```

生成物は`packages/yuukei-intelligence/dist/yuukei-intelligence/`です。Rust実行
ファイル、LiteRT-LM DLL、モデル、manifestをすべて含むため、利用者側に
Node.js、Python、モデルパス設定は必要ありません。この生成物ディレクトリを
YuukeiのExtension設定からインストールします。

モデルとランタイムは巨大なバイナリなのでGitには保存しません。パッケージ処理は
LiteRT-LMランタイムを公式PyPI wheelからバージョン・SHA-256固定で取得し、
指定されたモデルのSHA-256も検証します。
