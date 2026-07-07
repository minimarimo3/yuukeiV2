# Changelog

Yuukeiの変更履歴。[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/)の形式に従う。

バージョニング方針: [SemVer](https://semver.org/lang/ja/)。v1.0.0はROADMAPのリリース判定基準(DoD)達成時。それまでは0.x系で、Daihon言語・protocol・pack.json形式の破壊的変更はminorを上げて本書に明記する。World Pack互換性(08の構文)はv1.0以降、majorバージョン内で維持する。

## [Unreleased]

### Added

- Daihon `選択` 式と選択肢バルーン — 住人の問いかけにボタンで答える。タイムアウトで `不明`
- `入力#hitSurface` — 肌/服/髪/顔で反応を分けられる
- ユーザー不在/復帰の検出(`presence.idle.*`、別名 `不在_開始`/`復帰`、`入力#不在分`)
- VOICEVOX読み上げ(yuukei-voicevox拡張、`speech.synthesis` 初実装、住人ごとの声設定)
- デスクトップ地形: ウィンドウ観測(`desktop.window.*`)と `stage.perch`(枠に座る・追従・消滅時フォールバック)
- フォルダ遭遇(`desktop.folder.opened`、カテゴリ正規化)とダウンロード観測(`desktop.download.completed`)
- Daihon接続: 別名5種、日本語入力名、`枠に座る`/`枠から降りる`、`入力#最近のダウンロード`(7日enrich)
- 観測プライバシー設定(3種、既定OFF)と設定GUI
- default pack: あんぱんシナリオ、掛け合い、触られ反応、おかえりなさい、選択肢デモ
- 初回起動オンボーディング(World Pack/AI設定/プライバシーの3ステップ)
- World Packのzipインポート(検証+ライセンス表示)
- 生活の記録(イベントログ)の閲覧・削除UI(期間/種類/全削除+監査record)
- ログローテーションとevent log上限trim、Extensionプロセスの休止・再起動導線
- ログイン時自動起動、LLMタイムアウト/文脈件数/感情しきい値の設定化、気分の永続化、シーン実行履歴UI
- ユーザーガイド(USERGUIDE.md)、Windows実機確認チェックリスト

### Changed

- Daihon: 1ファイルに複数イベントを書けるように緩和(予定: タスクM)
- Daihon: イベント名一致の暗黙合図(予定: タスクM)

### Notes

- Windows観測コードは実機未検証(windows-verification.md参照)
- アプリアイコン・署名・インストーラはM5で対応予定
