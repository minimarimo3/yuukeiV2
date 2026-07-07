# Windows実機確認チェックリスト(M5)

Windows実機での動作確認手順。M2のWindows観測コード(EnumWindows / IShellWindows / notify watcher)はmacOS上で書かれ、**コンパイル・実行とも実機未検証**。ここが最優先の確認対象。

## 0. ビルド環境

- [ ] Rust(rustup)、Node.js 20+、pnpm、Visual Studio Build Tools(C++)、WebView2 Runtime(Win11は標準)
- [ ] `pnpm install` → `pnpm dev:tauri` でビルドが通り、住人が現れる
  - まずここでWindows専用コード(`window_observer.rs` ほか)のコンパイルエラーが出る可能性が一番高い。エラーはそのままIssue/メモへ

## 1. 基本動作

- [ ] 透過ウィンドウ: 住人の背景が透過し、クリックスルーが機能する
- [ ] バルーン表示、話しかけ、クリック/なで反応
- [ ] 選択肢バルーン: 問いかけ→ボタンで返答→タイムアウト(30秒放置)で畳まれる
- [ ] オンボーディングが初回に出る(データディレクトリを消して確認)
- [ ] VOICEVOX(Windows版)起動状態で声が出る。未起動で無音のまま生活が続く

## 2. M2観測(要: 設定画面で観測をON)

- [ ] ウィンドウ観測: 新しいウィンドウを開くと `窓_出現` 反応(住人が枠に座りに来ることがある)。ウィンドウを動かすと追従、閉じると降りる
- [ ] cloaked window(UWPの最小化等)がイベントを出しすぎないこと
- [ ] フォルダ遭遇: ExplorerでDownloads/ドキュメント/ピクチャ/ごみ箱を開くと反応。**ごみ箱**はシェル特殊フォルダ扱いの確認を重点的に
- [ ] ダウンロード観測: ブラウザで画像を保存→完了時に反応。`.crdownload` / `.part` 中は反応しないこと
- [ ] あんぱんシナリオ: 「あんぱん」を含む名前の画像をDownloadsへ保存→Explorerで Downloads を開く→つまみ食いシーン
- [ ] 「生活の記録」にフルパス・ウィンドウタイトルが**記録されていない**こと(プライバシー確認)

## 3. 常駐と安定

- [ ] ログイン時自動起動トグルが効く(サインアウト→インで起動)
- [ ] スリープ→復帰で `端末_復帰` 反応、破綻なし
- [ ] 数時間常駐でタスクマネージャのメモリ/CPUが安定

## 4. インストーラ(tauri build)

- [ ] `pnpm build:tauri` でNSIS/MSIが生成される(注: アプリアイコン未設定のため失敗する場合は `tauri icon` でアイコン生成が先)
- [ ] インストール→起動→アンインストールが通る
- [ ] SmartScreen警告の文言確認(署名なしの想定内挙動)

## 既知の自信が低い箇所(Codex申告)

- `K32GetModuleBaseNameW` でのexe名取得
- `DwmGetWindowAttribute(DWMWA_CLOAKED)` の除外判定
- IShellWindowsのCOM呼び出し全般

問題が出たら、症状と「生活の記録」の該当イベント、可能なら `app-activity.jsonl` を添えてメモしてください。
