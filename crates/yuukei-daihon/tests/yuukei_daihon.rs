use std::collections::{BTreeMap, BTreeSet, VecDeque};

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use yuukei_daihon::*;

#[derive(Default)]
struct MockActionHandler {
    dialogues: Vec<(Option<String>, String)>,
    calls: Vec<MockFunctionCall>,
}

type MockFunctionCall = (
    Option<String>,
    String,
    Vec<DaihonValue>,
    BTreeMap<String, DaihonValue>,
);

#[derive(Default)]
struct MockInterpretHandler {
    responses: VecDeque<std::result::Result<String, DaihonRuntimeError>>,
    requests: Vec<InterpretRequest>,
}

impl MockInterpretHandler {
    fn with_responses(responses: impl IntoIterator<Item = String>) -> Self {
        Self {
            responses: responses.into_iter().map(Ok).collect(),
            requests: Vec::new(),
        }
    }

    fn with_error() -> Self {
        Self {
            responses: VecDeque::from([Err(DaihonRuntimeError::new(
                "E-TEST-INTERPRET",
                "interpret failed",
                Span::empty(),
            ))]),
            requests: Vec::new(),
        }
    }
}

#[async_trait]
impl InterpretHandler for MockInterpretHandler {
    async fn interpret(&mut self, request: InterpretRequest) -> Result<String, DaihonRuntimeError> {
        self.requests.push(request);
        self.responses
            .pop_front()
            .unwrap_or_else(|| Ok(UNKNOWN_INTERPRETATION.to_string()))
    }
}

#[async_trait]
impl ActionHandler for MockActionHandler {
    async fn show_dialogue(
        &mut self,
        speaker_id: Option<&str>,
        text: &str,
    ) -> Result<(), DaihonRuntimeError> {
        self.dialogues
            .push((speaker_id.map(ToOwned::to_owned), text.to_owned()));
        Ok(())
    }

    async fn call_function(
        &mut self,
        speaker_id: Option<&str>,
        name: &str,
        positional: Vec<DaihonValue>,
        named: BTreeMap<String, DaihonValue>,
    ) -> Result<DaihonValue, DaihonRuntimeError> {
        self.calls.push((
            speaker_id.map(ToOwned::to_owned),
            name.to_owned(),
            positional.clone(),
            named.clone(),
        ));
        match name {
            "ランダム" => Ok(DaihonValue::Number(DaihonNumber::Integer(42))),
            "表示" => Ok(positional.first().cloned().unwrap_or(DaihonValue::None)),
            _ => Ok(DaihonValue::None),
        }
    }
}

fn registry() -> FunctionRegistry {
    let mut registry = FunctionRegistry::new();
    registry.register(FunctionSpec {
        name: "笑顔".to_owned(),
        positional: vec![],
        named: BTreeMap::new(),
        return_type: None,
    });
    registry.register(FunctionSpec {
        name: "表情".to_owned(),
        positional: vec![ParamSpec {
            name: Some("名前".to_owned()),
            ty: ParamType::BareWord,
            required: true,
        }],
        named: BTreeMap::new(),
        return_type: None,
    });
    registry.register(FunctionSpec {
        name: "表示".to_owned(),
        positional: vec![ParamSpec {
            name: Some("値".to_owned()),
            ty: ParamType::Any,
            required: true,
        }],
        named: BTreeMap::new(),
        return_type: Some(ValueType::String),
    });
    registry.register(FunctionSpec {
        name: "ランダム".to_owned(),
        positional: vec![
            ParamSpec {
                name: Some("min".to_owned()),
                ty: ParamType::Number,
                required: true,
            },
            ParamSpec {
                name: Some("max".to_owned()),
                ty: ParamType::Number,
                required: true,
            },
        ],
        named: BTreeMap::new(),
        return_type: Some(ValueType::Number),
    });
    registry.register(FunctionSpec {
        name: INTERPRET_FUNCTION_NAME.to_owned(),
        positional: vec![
            ParamSpec {
                name: Some("入力".to_owned()),
                ty: ParamType::Any,
                required: true,
            },
            ParamSpec {
                name: Some("質問".to_owned()),
                ty: ParamType::String,
                required: true,
            },
            ParamSpec {
                name: Some("選択肢".to_owned()),
                ty: ParamType::String,
                required: true,
            },
        ],
        named: BTreeMap::new(),
        return_type: Some(ValueType::String),
    });
    registry
}

fn fixed_now(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

#[test]
fn lexer_preserves_dialogue_comments_and_reports_unclosed() {
    let tokens = lex_source("## イベント\n### 場面\n「$$ コメントじゃない」＜笑顔＞\n").unwrap();
    assert!(tokens.iter().any(|token| {
        token.kind == TokenKind::DialogueText && token.original.contains("$$ コメントじゃない")
    }));

    let diagnostics = lex_source("## イベント\n### 場面\n「閉じない").unwrap_err();
    assert_eq!(diagnostics[0].code, "E-DHN-LEX-001");

    let diagnostics = lex_source("## イベント\n### 場面\n＜閉じない").unwrap_err();
    assert_eq!(diagnostics[0].code, "E-DHN-LEX-002");
}

#[test]
fn parser_reads_metadata_speaker_scoped_display_and_bareword() {
    let script = parse_script(
        r#"
## 起動
初期値:
好感度=10
### クリック反応
合図: ＠クリック または @ダブルクリック
条件:（好感度 10 以上）
優先度: 20
重み: 3
クールダウン: 30分
話者: ミカ
ミカ: 「こんにちは」＜表情 笑顔＞
"#,
    )
    .unwrap();

    let scene = &script.event.scenes[0];
    assert_eq!(scene.metadata.signals.len(), 2);
    assert_eq!(scene.metadata.priority, 20);
    assert_eq!(scene.metadata.weight, 3);
    assert_eq!(scene.metadata.cooldown.unwrap().as_secs(), 1800);
    assert_eq!(scene.metadata.speaker.as_ref().unwrap().value, "ミカ");
    match &scene.statements[0] {
        Stmt::SpeakerDisplay { speaker, display } => {
            assert_eq!(speaker.value, "ミカ");
            match &display.parts[1] {
                DisplayPart::FunctionCall(call) => {
                    assert!(matches!(call.positional[0], FuncArg::BareWord(_)));
                }
                _ => panic!("expected function call"),
            }
        }
        _ => panic!("expected speaker display"),
    }
}

#[test]
fn parser_reads_scoped_variables_and_dialogue_embed() {
    let script = parse_script(
        r#"
## 会話
### 通常
全体#起動回数=全体#起動回数 + 1
住人#ミカ#機嫌=「良い」
関係#ミカ#ユーザー#好感度=10
「こんにちは＜入力#ユーザー名＞さん」
"#,
    )
    .unwrap();

    assert!(matches!(
        &script.event.scenes[0].statements[0],
        Stmt::Assignment(assignment) if matches!(assignment.target, VariableRef::Global { .. })
    ));
    assert!(matches!(
        &script.event.scenes[0].statements[1],
        Stmt::Assignment(assignment) if matches!(assignment.target, VariableRef::Resident { .. })
    ));
    assert!(matches!(
        &script.event.scenes[0].statements[2],
        Stmt::Assignment(assignment) if matches!(assignment.target, VariableRef::Relation { .. })
    ));
}

#[test]
fn parser_splits_dialogue_embeds_before_following_scene_metadata() {
    let script = parse_script(
        r#"
## 生活
### 起動
合図: ＠app.startup
「起動：＜入力#時間帯＞」
### 接続
合図: ＠surface.attach
「ここにいます。」
"#,
    )
    .unwrap();

    assert_eq!(script.event.scenes.len(), 2);
    assert_eq!(
        script.event.scenes[0].metadata.signals[0].name.value,
        "app.startup"
    );
    assert_eq!(
        script.event.scenes[1].metadata.signals[0].name.value,
        "surface.attach"
    );
}

#[test]
fn validator_rejects_completed_spec_errors() {
    let script = parse_script(
        r#"
## 検証
### A
重み: 0
入力#ユーザー発言=「x」
世界#天気=「晴れ」
※（好感度 + 10 50以上）「好き」
→Missing
### A
「重複」
"#,
    )
    .unwrap();
    let diagnostics = validate_script(&script, Some(&registry()));
    let codes = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<BTreeSet<_>>();
    assert!(codes.contains("E-DHN-SEM-005"));
    assert!(codes.contains("E-DHN-SEM-013"));
    assert!(codes.contains("E-DHN-SEM-020"));
    assert!(codes.contains("E-DHN-SEM-021"));
    assert!(codes.contains("E-DHN-SEM-030"));
    assert!(codes.contains("E-DHN-SEM-006"));
}

#[test]
fn validator_checks_function_registry() {
    let script = parse_script(
        r#"
## 関数
### 通常
値=＜笑顔＞
＜未知＞
＜ランダム 1＞
"#,
    )
    .unwrap();
    let diagnostics = validate_script(&script, Some(&registry()));
    let codes = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"E-DHN-SEM-047"));
    assert!(codes.contains(&"E-DHN-SEM-040"));
    assert!(codes.contains(&"E-DHN-SEM-041"));
}

#[test]
fn validator_requires_interpret_result_to_have_unknown_or_catch_all_branch() {
    let script = parse_script(
        r#"
## 解釈
### 通常
判定=＜解釈 (入力#ユーザー発言) 「お出かけOK？」 「はい/いいえ」＞
※（判定 = 「はい」）なら:
「行こう」
おわり
"#,
    )
    .unwrap();

    let diagnostics = validate_script(&script, Some(&registry()));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E-DHN-SEM-048"));
}

#[tokio::test]
async fn runtime_selects_one_triggered_scene_with_speaker_and_bareword() {
    let script = parse_script(
        r#"
## 反応
### 低優先
合図: ＠クリック
優先度: 1
「低」
### 高優先
合図: ＠クリック
優先度: 10
話者: ミカ
＜表情 笑顔＞
「呼んだ？」
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = NoopInterpretHandler;
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions {
            trigger: Some(SystemEvent::new("クリック", Span::empty())),
            now: Some(fixed_now("2026-06-27T10:00:00+09:00")),
            ..RunOptions::default()
        },
        interpretation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(result.selected_scene.unwrap().name, "高優先");
    assert_eq!(action.calls[0].0.as_deref(), Some("ミカ"));
    assert_eq!(action.calls[0].2[0], DaihonValue::String("笑顔".to_owned()));
    assert_eq!(action.dialogues[0].0.as_deref(), Some("ミカ"));
}

#[tokio::test]
async fn runtime_interpret_choice_branches_scene() {
    let script = parse_script(
        r#"
## 解釈
### 通常
判定=＜解釈 (入力#ユーザー発言) 「お出かけOK？」 「はい/いいえ」＞
※（判定 = 「はい」）なら:
「出発」
※あるいは（判定 = 「不明」）なら:
「わからない」
※それ以外:
「また今度」
おわり
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = MockInterpretHandler::with_responses(["はい".to_string()]);
    let mut variables = InMemoryVariableStore::new().with_input(
        "ユーザー発言",
        DaihonValue::String("うん、行ける".to_string()),
    );
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions::default(),
        interpretation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert!(result.diagnostics.is_empty());
    assert_eq!(interpret.requests.len(), 1);
    assert_eq!(interpret.requests[0].input_text, "うん、行ける");
    assert_eq!(interpret.requests[0].question, "お出かけOK？");
    assert_eq!(interpret.requests[0].choices, vec!["はい", "いいえ"]);
    assert_eq!(action.dialogues[0].1, "出発");
}

#[tokio::test]
async fn runtime_interpret_error_becomes_unknown() {
    let script = parse_script(
        r#"
## 解釈
### 通常
判定=＜解釈 (入力#ユーザー発言) 「お出かけOK？」 「はい/いいえ」＞
※（判定 = 「はい」）なら:
「出発」
※あるいは（判定 = 「不明」）なら:
「わからない」
※それ以外:
「また今度」
おわり
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = MockInterpretHandler::with_error();
    let mut variables = InMemoryVariableStore::new()
        .with_input("ユーザー発言", DaihonValue::String("無理かも".to_string()));
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions::default(),
        interpretation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert!(result.diagnostics.is_empty());
    assert_eq!(action.dialogues[0].1, "わからない");
}

#[tokio::test]
async fn runtime_interpret_limit_returns_unknown_and_warns() {
    let script = parse_script(
        r#"
## 解釈
### 通常
一回目=＜解釈 (入力#ユーザー発言) 「一回目？」 「はい/いいえ」＞
※（一回目 = 「不明」）なら:
「一回目不明」
※それ以外:
「一回目既知」
おわり
二回目=＜解釈 (入力#ユーザー発言) 「二回目？」 「はい/いいえ」＞
※（二回目 = 「不明」）なら:
「二回目不明」
※それ以外:
「二回目既知」
おわり
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret =
        MockInterpretHandler::with_responses(["はい".to_string(), "はい".to_string()]);
    let mut variables = InMemoryVariableStore::new()
        .with_input("ユーザー発言", DaihonValue::String("うん".to_string()));
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions {
            max_interpretations_per_dispatch: 1,
            ..RunOptions::default()
        },
        interpretation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(interpret.requests.len(), 1);
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W-DHN-RUN-050"));
    assert_eq!(action.dialogues[0].1, "一回目既知");
    assert_eq!(action.dialogues[1].1, "二回目不明");
}

#[tokio::test]
async fn runtime_uses_defaults_preconditions_and_assignments() {
    let script = parse_script(
        r#"
## 算術
初期値:
好感度=10
### 通常
結果=好感度 / 3
文=「値」 + 結果
「＜文＞」
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = NoopInterpretHandler;
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &FunctionRegistry::permissive(),
        options: RunOptions::default(),
        interpretation_count: 0,
        diagnostics: Vec::new(),
    };

    interpreter.run_script(&script).await.unwrap();
    assert_eq!(
        variables
            .get_value(&VariableRef::EventLocal {
                name: Spanned::new("結果".to_owned(), Span::empty())
            })
            .unwrap(),
        DaihonValue::Number(DaihonNumber::Integer(3))
    );
    assert_eq!(action.dialogues[0].1, "値3");
}

#[tokio::test]
async fn runtime_falls_back_to_default_and_respects_cooldown() {
    let script = parse_script(
        r#"
## 既定
### クリック
合図: ＠クリック
「クリック」
### 既定A
クールダウン: 10分
「A」
### 既定B
「B」
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = NoopInterpretHandler;
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    history.record_executed("既定", "既定A", fixed_now("2026-06-27T10:00:00+09:00"));
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &FunctionRegistry::permissive(),
        options: RunOptions {
            now: Some(fixed_now("2026-06-27T10:05:00+09:00")),
            random_seed: Some(1),
            ..RunOptions::default()
        },
        interpretation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(result.selected_scene.unwrap().name, "既定B");
}

#[tokio::test]
async fn runtime_supports_overnight_time_range() {
    let script = parse_script(
        r#"
## 深夜
### 夜
条件:（22:00~02:00）
優先度: 10
「夜」
### 昼
「昼」
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = NoopInterpretHandler;
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &FunctionRegistry::permissive(),
        options: RunOptions {
            now: Some(fixed_now("2026-06-27T01:30:00+09:00")),
            ..RunOptions::default()
        },
        interpretation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(result.selected_scene.unwrap().name, "夜");
}

#[test]
fn inspect_reports_scene_variables_functions_and_signals() {
    let info = inspect_script(
        r#"
## 情報
### クリック
合図: ＠クリック
話者: ミカ
好感度=好感度 + 1
＜笑顔＞
"#,
    )
    .unwrap();
    assert_eq!(info.event_name, "情報");
    assert_eq!(info.scenes[0].speaker.as_deref(), Some("ミカ"));
    assert_eq!(info.signals[0].name.value, "クリック");
    assert!(info.functions_called.contains(&"笑顔".to_owned()));
    assert_eq!(info.variables_written.len(), 1);
    assert_eq!(info.variables_read.len(), 1);
}
