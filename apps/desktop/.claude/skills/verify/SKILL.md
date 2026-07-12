---
name: verify
description: Yuukeiデスクトップアプリ(Tauri)の変更を実機で検証する手順。ビルド・隔離起動・ウィンドウ観察・後片付け。
---

# Yuukei desktop の実機検証

## 起動(ユーザーデータを汚さない)

```bash
mkdir -p <scratchpad>/yuukei-verify-data
cd apps/desktop
YUUKEI_DATA_DIR=<scratchpad>/yuukei-verify-data pnpm tauri dev > <scratchpad>/tauri-dev.log 2>&1 &
```

- `YUUKEI_DATA_DIR` で data_dir を完全に隔離できる(crates/yuukei-device-host/src/paths.rs)。実データは `~/Library/Application Support/Yuukei/v2`。
- 初回ビルドは数分。プロセス検知: `pgrep -f "target/debug/yuukei-desktop"`。
- 起動確認ログ: `Yuukei app log: .../app-activity.jsonl` が tauri-dev.log に出る。

## 観察(GUIはAppleScriptで)

`screencapture` は権限不足で失敗する。System Events は使える:

```bash
osascript -e 'tell application "System Events" to tell (first process whose unix id is <PID>) to get name of windows'
# 設定ウィンドウを閉じる(×クリック)
osascript -e '... to click button 1 of window "Yuukei Settings"'
```

- ウィンドウ一覧の無名3つはアクター/ステージオーバーレイ。設定画面は "Yuukei Settings"。
- 永続化の確認は `<data-dir>/settings/*.json` を直接読む。

## 後片付け

```bash
pkill -f "target/debug/yuukei-desktop"
lsof -ti:1420 | xargs kill   # vite dev server
```
