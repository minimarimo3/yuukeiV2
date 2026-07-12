# Desktop Conversation Input Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tauri Desktop Surfaceでキャラを右クリックし、設定可能な誤送信防止キーで一時入力欄から会話できるようにする。

**Architecture:** Device Host所有のアプリ設定へ送信キー列挙を追加し、Actor Surfaceの右クリックをDesktopStageManagerがStage Overlay向けの一時UI状態へ変換する。Stage Overlayは受動的な入力・描画面として既存の`sendConversationText`を呼び、Resident Homeの会話処理は変更しない。

**Tech Stack:** Rust, serde, Tauri 2, React 19, TypeScript, Vitest, React Testing Library

## Global Constraints

- 既定の送信操作は `Ctrl+Enter`。
- 選択肢は `Ctrl+Enter`、`Enter`、`Shift+Enter` の三つだけ。
- IME composition中は送信しない。
- Surfaceに人格、記憶、台本選択、capability選択を持たせない。
- 既存の `conversation.text` protocolと`sendConversationText()`を再利用し、会話対象actor protocolは追加しない。
- 既存`app.json`を後方互換で読み、未設定値を読み取り時に暗黙保存しない。
- poke、drag、吹き出し選択肢、透明領域のclick-throughを維持する。
- 新しい依存関係を追加しない。

---

## File Structure

- `crates/yuukei-device-host/src/settings.rs`: 送信キー列挙、保存、検証、後方互換。
- `crates/yuukei-device-host/src/runtime_settings_api.rs`: 設定更新API。
- `apps/desktop/src-tauri/src/lib.rs`: Tauri設定commandと会話入力表示command。
- `apps/desktop/src-tauri/src/desktop_stage/mod.rs`: actor anchorから会話入力のStage状態を管理・配信。
- `apps/desktop/src/yuukeiClient.ts`: Rust/React間の型とclient methods。
- `apps/desktop/src/ActorApp.tsx`: actor右クリックを通知。
- `apps/desktop/src/ConversationComposer.tsx`: 入力、キー判定、送信状態、エラー表示。
- `apps/desktop/src/ConversationComposer.test.tsx`: 入力コンポーネントの振る舞い。
- `apps/desktop/src/StageOverlayApp.tsx`: composer配置とStage click-through統合。
- `apps/desktop/src/StageOverlayApp.test.tsx`: Stage状態からcomposerを表示する統合テスト。
- `apps/desktop/src/App.tsx`: 「キー設定」カテゴリ。
- `apps/desktop/src/App.test.tsx`: 設定UIテスト。
- `apps/desktop/src/styles.css`: composerとキー設定の見た目。

### Task 1: Device Hostの送信キー設定

**Files:**
- Modify: `crates/yuukei-device-host/src/settings.rs`
- Modify: `crates/yuukei-device-host/src/runtime_settings_api.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/yuukeiClient.ts`

**Interfaces:**
- Produces: Rust `ConversationSendShortcut::{CtrlEnter, Enter, ShiftEnter}`、camelCase JSON文字列、`set_app_conversation_send_shortcut`。
- Produces: TypeScript `ConversationSendShortcut = "ctrlEnter" | "enter" | "shiftEnter"` と `setAppConversationSendShortcut(shortcut)`。

- [ ] **Step 1: 後方互換と保存の失敗テストを書く**

`settings.rs`の既存テストへ、旧JSONが`CtrlEnter`として読めること、三値が保存・再読込できることを追加する。

```rust
#[test]
fn old_app_settings_default_conversation_send_shortcut_to_ctrl_enter() -> Result<()> {
    let data = tempdir()?;
    write_settings(data.path(), r#"{"schemaVersion":1,"talkIntervalMinutes":7}"#)?;
    let registry = AppSettingsRegistry::open(data.path())?;
    assert_eq!(registry.state().conversation_send_shortcut, ConversationSendShortcut::CtrlEnter);
    Ok(())
}

#[test]
fn conversation_send_shortcut_round_trips() -> Result<()> {
    let data = tempdir()?;
    let mut registry = AppSettingsRegistry::open(data.path())?;
    registry.set_conversation_send_shortcut(ConversationSendShortcut::ShiftEnter)?;
    assert_eq!(AppSettingsRegistry::open(data.path())?.state().conversation_send_shortcut, ConversationSendShortcut::ShiftEnter);
    Ok(())
}
```

- [ ] **Step 2: REDを確認する**

Run: `cargo test -p yuukei-device-host settings::tests::old_app_settings_default_conversation_send_shortcut_to_ctrl_enter settings::tests::conversation_send_shortcut_round_trips`

Expected: `ConversationSendShortcut`とsetterが未定義でコンパイル失敗。

- [ ] **Step 3: 最小実装を追加する**

```rust
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationSendShortcut {
    #[default]
    CtrlEnter,
    Enter,
    ShiftEnter,
}
```

`StoredAppSettings`には`Option<ConversationSendShortcut>`、公開stateには既定適用済み値を置く。registry/runtime API/Tauri command/client methodを既存のactor scale setterと同じ経路で追加する。

- [ ] **Step 4: GREENと境界型を確認する**

Run: `cargo test -p yuukei-device-host settings`

Expected: PASS。

Run: `corepack pnpm --filter @yuukei/desktop typecheck`

Expected: fixture更新前の不足箇所だけが型エラーとして列挙される。各`AppSettingsState` fixtureへ`conversationSendShortcut: "ctrlEnter"`を追加して再実行しPASSにする。

- [ ] **Step 5: Task 1をコミットする**

```bash
git add crates/yuukei-device-host/src/settings.rs crates/yuukei-device-host/src/runtime_settings_api.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src/yuukeiClient.ts apps/desktop/src/*.test.tsx
git commit -m "feat: 会話送信キー設定を追加"
```

### Task 2: Actor右クリックとDesktop Stage状態

**Files:**
- Modify: `apps/desktop/src/ActorApp.tsx`
- Modify: `apps/desktop/src/ActorApp.test.tsx`
- Modify: `apps/desktop/src/yuukeiClient.ts`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/desktop_stage/mod.rs`
- Modify: `apps/desktop/src-tauri/src/desktop_stage/tests.rs`

**Interfaces:**
- Consumes: `StageAnchor { x, y, visible }` と既存actor state。
- Produces: `DesktopConversationComposer { actorId, monitorId, anchor } | null`を含む`DesktopStageState`。
- Produces: `openConversationComposer(actorId)` / `closeConversationComposer()` client methods。

- [ ] **Step 1: Stage managerの失敗テストを書く**

actorのmouth anchorでcomposerを開き、再度別actorで開くと置換し、closeで`None`になるテストを`desktop_stage/tests.rs`へ追加する。

```rust
stage.open_conversation_composer("yuukei")?;
let state = stage.state()?;
assert_eq!(state.conversation_composer.as_ref().unwrap().actor_id, "yuukei");
stage.close_conversation_composer()?;
assert!(stage.state()?.conversation_composer.is_none());
```

- [ ] **Step 2: REDを確認する**

Run: `cargo test -p yuukei-desktop conversation_composer`

Expected: stage methods/state fieldが未定義でコンパイル失敗。

- [ ] **Step 3: Stage状態とTauri commandを実装する**

`DesktopStageState`へoptional composerを追加し、actorの現在anchorとmonitorから位置を作る。open/close時は既存stage-state emit経路でoverlayへ通知する。未知actorは明示的エラーにする。

- [ ] **Step 4: Actor右クリックの失敗テストを書く**

`ActorApp.test.tsx`でsolidなactor canvas上の`contextmenu`が`openConversationComposer(actorId)`を一度呼び、`preventDefault`されることを検証する。通常pointer downのpoke/dragテストは変更せず残す。

- [ ] **Step 5: REDを確認して最小実装する**

Run: `corepack pnpm --filter @yuukei/desktop test -- ActorApp.test.tsx`

Expected: client method未呼び出しでFAIL。

`VrmStage`へ`onConversationOpen(actorId)`を渡し、rendererのinteractive rootで`onContextMenu`を処理する。右クリックをpointer gesture reducerへ新しい意味イベントとして入れない。

- [ ] **Step 6: GREENを確認する**

Run: `cargo test -p yuukei-desktop conversation_composer`

Run: `corepack pnpm --filter @yuukei/desktop test -- ActorApp.test.tsx`

Expected: 全てPASS。

- [ ] **Step 7: Task 2をコミットする**

```bash
git add apps/desktop/src/ActorApp.tsx apps/desktop/src/ActorApp.test.tsx apps/desktop/src/yuukeiClient.ts apps/desktop/src-tauri/src/lib.rs apps/desktop/src-tauri/src/desktop_stage
git commit -m "feat: キャラの右クリックで会話入力を開く"
```

### Task 3: 会話入力コンポーネントとStage Overlay統合

**Files:**
- Create: `apps/desktop/src/ConversationComposer.tsx`
- Create: `apps/desktop/src/ConversationComposer.test.tsx`
- Modify: `apps/desktop/src/StageOverlayApp.tsx`
- Modify: `apps/desktop/src/StageOverlayApp.test.tsx`
- Modify: `apps/desktop/src/styles.css`

**Interfaces:**
- Consumes: `ConversationSendShortcut`、`onSubmit(text): Promise<void>`、`onDismiss(): void`。
- Produces: accessible label `住人に話しかける`を持つtextareaと送信button。

- [ ] **Step 1: キー判定と送信状態の失敗テストを書く**

```tsx
it.each([
  ["ctrlEnter", { ctrlKey: true }],
  ["enter", {}],
  ["shiftEnter", { shiftKey: true }]
] as const)("submits with %s", async (shortcut, modifiers) => {
  render(<ConversationComposer shortcut={shortcut} onSubmit={submit} onDismiss={dismiss} />);
  const input = screen.getByRole("textbox", { name: "住人に話しかける" });
  await user.type(input, "こんにちは");
  fireEvent.keyDown(input, { key: "Enter", ...modifiers });
  expect(submit).toHaveBeenCalledWith("こんにちは");
});
```

別テストでcomposition中、空白、非割当Enter、Escape、二重送信、reject時の入力保持と`role="alert"`を検証する。

- [ ] **Step 2: REDを確認する**

Run: `corepack pnpm --filter @yuukei/desktop test -- ConversationComposer.test.tsx`

Expected: module未存在でFAIL。

- [ ] **Step 3: 最小コンポーネントを実装する**

controlled textarea、composition guard、`matchesSendShortcut(event, shortcut)`、pending/error stateを実装する。成功時のみ`onDismiss()`し、失敗時は入力を保持する。送信buttonは全設定で利用可能にする。

- [ ] **Step 4: GREENを確認する**

Run: `corepack pnpm --filter @yuukei/desktop test -- ConversationComposer.test.tsx`

Expected: PASS、console warningなし。

- [ ] **Step 5: Overlay統合の失敗テストを書く**

`DesktopStageState.conversationComposer`をfixtureへ入れ、該当monitorだけにcomposerが表示され、初期設定取得後に送信・close APIが呼ばれること、composer表示中はclick-throughがfalseになることを検証する。

- [ ] **Step 6: REDを確認して統合する**

Run: `corepack pnpm --filter @yuukei/desktop test -- StageOverlayApp.test.tsx`

Expected: textbox未表示でFAIL。

Stage Overlayでapp settingsを読み、actor bubble placement helperを再利用してcomposerをviewport内へclampする。外側pointer downはclose、composer内は伝播停止。composerまたはbubbleがinteractiveな間だけ既存hit-testing hookへ件数を渡す。

- [ ] **Step 7: GREENを確認する**

Run: `corepack pnpm --filter @yuukei/desktop test -- ConversationComposer.test.tsx StageOverlayApp.test.tsx`

Expected: PASS。

- [ ] **Step 8: Task 3をコミットする**

```bash
git add apps/desktop/src/ConversationComposer.tsx apps/desktop/src/ConversationComposer.test.tsx apps/desktop/src/StageOverlayApp.tsx apps/desktop/src/StageOverlayApp.test.tsx apps/desktop/src/styles.css
git commit -m "feat: デスクトップ会話入力欄を追加"
```

### Task 4: キー設定UIと統合回帰

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/App.test.tsx`
- Modify: `apps/desktop/src/styles.css`

**Interfaces:**
- Consumes: `AppSettingsState.conversationSendShortcut`。
- Calls: `setAppConversationSendShortcut(shortcut)`。

- [ ] **Step 1: 設定UIの失敗テストを書く**

「キー設定」カテゴリを選ぶと「会話を送信」selectが表示され、初期値が`Ctrl+Enter`で、`Shift+Enter`への変更がsetterを呼ぶテストを追加する。

```tsx
await user.click(screen.getByRole("button", { name: "キー設定" }));
const select = screen.getByRole("combobox", { name: "会話を送信" });
expect(select).toHaveValue("ctrlEnter");
await user.selectOptions(select, "shiftEnter");
expect(client.setAppConversationSendShortcut).toHaveBeenCalledWith("shiftEnter");
```

- [ ] **Step 2: REDを確認する**

Run: `corepack pnpm --filter @yuukei/desktop test -- App.test.tsx`

Expected: 「キー設定」button未存在でFAIL。

- [ ] **Step 3: 最小設定UIを実装する**

`SettingsCategoryId`へ`keys`を追加し、三つのoptionを持つselectを作る。変更時は既存`appSettingsError`経路で即時保存し、返却stateを反映する。

- [ ] **Step 4: GREENを確認する**

Run: `corepack pnpm --filter @yuukei/desktop test -- App.test.tsx`

Expected: PASS。

- [ ] **Step 5: 全体検証を実行する**

Run: `cargo fmt --check`

Run: `cargo test --workspace`

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Run: `corepack pnpm -r typecheck`

Run: `corepack pnpm -r test`

Run: `corepack pnpm -r build`

Expected: 全コマンド終了コード0、warning/errorなし。

- [ ] **Step 6: Tauri startup smokeを実行する**

Run: `corepack pnpm dev:tauri`

Expected: settings、actor、stage-overlay windowが起動し、最初のruntime errorがない。キャラ右クリックでcomposerが開き、既定Enterは改行、Ctrl+Enterで送信、Escapeで閉じることを確認する。`app-activity.jsonl`に`surface.attach`と`app.startup`があることを確認する。

- [ ] **Step 7: Task 4をコミットする**

```bash
git add apps/desktop/src/App.tsx apps/desktop/src/App.test.tsx apps/desktop/src/styles.css
git commit -m "feat: 会話送信のキー設定を追加"
```

### Task 5: Completion Review

**Files:**
- Review: all files changed since `a1800c4`

**Interfaces:**
- Verifies: design specの全要件と、Surface/Device Host/Resident Home責務境界。

- [ ] **Step 1: 差分と設計仕様を照合する**

Run: `git diff --check a1800c4..HEAD`

Run: `git diff --stat a1800c4..HEAD`

Expected: whitespace errorなし。変更が会話入力、設定、Desktop Stageとそのテストに限定される。

- [ ] **Step 2: verification-before-completionを実行する**

Task 4 Step 5の6コマンドを再実行し、その最新出力だけを根拠に完了判定する。

- [ ] **Step 3: requesting-code-reviewでレビューする**

設計適合、右クリックとgesture競合、IME誤送信、click-through、後方互換を重点確認し、指摘があればTDDで修正する。
