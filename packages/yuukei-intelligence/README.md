# yuukei-intelligence

Yuukei公式のローカルIntelligence Extensionです。Rust製の常駐プロセスから
LiteRT-LM 0.14.0のC APIを呼び、同梱されたGemma 4 E4Bを次の限定用途に使います。

- Daihonが指示した短い台詞の補完
- Daihon内で受け取った曖昧な入力の選択肢判定・値抽出
- 日次memory indexと気分評価

常設チャットや自由会話の生成器ではありません。`dialogue.generate`はDaihon由来の
`instruction`がない呼び出しを無言で拒否します。

## 制約付きデコード

モデルが返す構造は、プロンプトでJSONを依頼するだけではありません。各capability
専用のJSON SchemaをLiteRT-LMのtool定義として渡し、conversationのconstrained
decodingを有効にしています。モデルは`submit_yuukei_result`を一度だけ呼び、その
argumentsだけがCoreへの結果になります。通常テキストや想定外のtool callは
後処理でJSONらしい箇所を拾わず、推論失敗として安全側に倒します。

この経路を採用する理由は、LiteRT-LM 0.14.0の公開C ABIが任意のRegex／Lark／
LlGuidance JSON Schema引数を公開していない一方、JSON Schema付きtoolとGemma用
制約付きデコードを公開しているためです。

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
