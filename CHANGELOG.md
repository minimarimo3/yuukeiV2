# Changelog

Yuukeiの変更履歴。[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/)の形式に従う。

バージョニング方針: [SemVer](https://semver.org/lang/ja/)。v1.0.0はROADMAPのリリース判定基準(DoD)達成時。それまでは0.x系で、Daihon言語・protocol・pack.json形式の破壊的変更はminorを上げて本書に明記する。World Pack互換性(08の構文)はv1.0以降、majorバージョン内で維持する。

## [Unreleased]

### Added

- Daihon `選択` 式と選択肢バルーン — 住人の問いかけにボタンで答える。タイムアウトで `不明`
- Daihon `場所`・`退場`・`登場` — 住人の意味上の現在地と在席状態を台本から変更し、フォルダ探検などの一時不在を表現できる
- `入力#hitSurface` — 肌/服/髪/顔で反応を分けられる
- ユーザー不在/復帰の検出(`presence.idle.*`、別名 `不在_開始`/`復帰`、`入力#不在分`)
- VOICEVOX読み上げ(yuukei-voicevox拡張、`speech.synthesis` 初実装、住人ごとの声設定)
- デスクトップ地形: ウィンドウ観測(`desktop.window.*`)と `stage.perch`(枠に座る・追従・消滅時フォールバック)
- フォルダ遭遇(`desktop.folder.opened`、カテゴリ正規化)とダウンロード観測(`desktop.download.completed`)
- Daihon接続: 別名5種、日本語入力名、`枠に座る`/`枠から降りる`、`入力#最近のダウンロード`(7日enrich)
- 観測プライバシー設定(3種、既定OFF)と設定GUI
- default pack: あんぱんシナリオ、掛け合い、触られ反応、おかえりなさい、選択肢デモ
- default pack: 生活時計による自発歩行・部屋への外出と帰宅・歩行終了反応、およびダウンロードや留守番が後の会話へ続く生活シーン
- default pack: LLMを用いた呼び名の抽出、調子の解釈、時間帯・届きもの・留守番・注目中アプリに応じた生成シーン(すべて固定セリフへフォールバック)
- Daihon: `生成` などの文字列引数で、入力値や保存変数を含む文字列結合式を利用可能に
- 初回起動オンボーディング(World Pack/AI設定/プライバシーの3ステップ)
- World Packのzipインポート(検証+ライセンス表示)
- 生活の記録(イベントログ)の閲覧・削除UI(期間/種類/全削除+監査record)
- ログローテーションとevent log上限trim、Extensionプロセスの休止・再起動導線
- ログイン時自動起動、LLMタイムアウト/文脈件数/感情しきい値の設定化、気分の永続化、シーン実行履歴UI
- ユーザーガイド(USERGUIDE.md)、Windows実機確認チェックリスト
- THIRD-PARTY-LICENSES生成スクリプトとVOICEVOXクレジット表示

### Fixed

- デュアルディスプレイでYuukei起動中、住人がいる画面の自動非表示タスクバーが画面端から表示されなくなる問題を修正
- default pack: 靴やズボンをつついても「そで、伸びます」と言う誤反応を修正(clothセリフを `入力#hitZoneId` で腕/足/その他に出し分け。VRoidモデルは靴・ズボンも `_CLOTH` マテリアルのため `hitSurface` だけでは部位を特定できない)
- 既定データディレクトリをOSの一時領域からOS標準のアプリデータ領域へ移動(既存データは初回起動時に自動移行)。生活史がOS再起動・temp掃除で消える問題の修正
- オンボーディングを「あとで」やウィンドウを閉じるで抜けても、起動のたびに設定画面が勝手に開いていた問題を修正(却下を `onboarding.json` に永続化し、自動オープンは未完了かつ未却下のときだけに)

### Changed

- Daihon: 1ファイルに複数イベントを書けるように緩和(同一ファイル内の重複イベント名は `E-DHN-SEM-007`)
- Daihon: イベント名が合図と一致するとき、合図なしシーンをその合図の候補として扱う(暗黙合図)

### Notes

- Windows観測コードは実機未検証(windows-verification.md参照)
- アプリアイコン・署名・インストーラはM5で対応予定
