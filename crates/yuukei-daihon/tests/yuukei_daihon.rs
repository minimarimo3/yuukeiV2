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
    generate_responses:
        VecDeque<std::result::Result<Option<GeneratedDialogue>, DaihonRuntimeError>>,
    generate_requests: Vec<GenerateRequest>,
}

impl MockInterpretHandler {
    fn with_responses(responses: impl IntoIterator<Item = String>) -> Self {
        Self {
            responses: responses.into_iter().map(Ok).collect(),
            requests: Vec::new(),
            generate_responses: VecDeque::new(),
            generate_requests: Vec::new(),
        }
    }

    fn with_generate_responses(
        responses: impl IntoIterator<
            Item = std::result::Result<Option<GeneratedDialogue>, DaihonRuntimeError>,
        >,
    ) -> Self {
        Self {
            responses: VecDeque::new(),
            requests: Vec::new(),
            generate_responses: responses.into_iter().collect(),
            generate_requests: Vec::new(),
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
            generate_responses: VecDeque::new(),
            generate_requests: Vec::new(),
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

    async fn generate(
        &mut self,
        request: GenerateRequest,
    ) -> Result<Option<GeneratedDialogue>, DaihonRuntimeError> {
        self.generate_requests.push(request);
        self.generate_responses.pop_front().unwrap_or(Ok(None))
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
    registry.register(FunctionSpec {
        name: GENERATE_FUNCTION_NAME.to_owned(),
        positional: vec![
            ParamSpec {
                name: Some("指示".to_owned()),
                ty: ParamType::String,
                required: true,
            },
            ParamSpec {
                name: Some("フォールバック".to_owned()),
                ty: ParamType::String,
                required: false,
            },
        ],
        named: BTreeMap::new(),
        return_type: None,
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
fn parser_ignores_line_and_trailing_comments() {
    for source in [
        "## t\n$$ c\n### s\n「a」\n",
        "## t\n＄＄ c\n### s\n「a」\n",
        "## t\n### s\n$$ c\n「a」\n",
        "## t\n### s\n＄＄ c\n「a」\n",
        "## t\n### s\n好感度=1 $$ c\n「a」\n",
        "## t\n### s\n好感度=1 ＄＄ c",
    ] {
        parse_script(source).unwrap_or_else(|diagnostics| {
            panic!("expected comments to parse, got diagnostics: {diagnostics:?}")
        });
    }
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
頻度: 2時間に1回
話者: ミカ
ミカ: 「こんにちは」＜表情 笑顔＞
"#,
    )
    .unwrap();

    let scene = &script.event.scenes[0];
    assert_eq!(scene.metadata.signals.len(), 2);
    assert!(scene.metadata.raw.priority_text.is_some());
    assert!(scene.metadata.raw.weight_text.is_some());
    assert!(scene.metadata.raw.cooldown_text.is_some());
    assert_eq!(
        scene.metadata.frequency,
        Some(SceneFrequency::PerDuration(std::time::Duration::from_secs(
            7200
        )))
    );
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
fn parser_accepts_fullwidth_arithmetic_operators_in_assignments_and_conditions() {
    let script = parse_script(
        r#"
## 全角演算
初期値:
好感度=5
### 通常
条件:（好感度 ＊ 2 >= 10）
好感度=好感度＋1
差=好感度－2
積=好感度＊2
商=好感度／2
余=好感度％2
「ok」
"#,
    )
    .unwrap();

    match script.event.scenes[0].metadata.condition.as_ref().unwrap() {
        Expr::Comparison { left, op, .. } => {
            assert_eq!(*op, ComparisonOp::Gte);
            assert!(matches!(
                left.as_ref(),
                Expr::Binary {
                    op: BinaryOp::Multiply,
                    ..
                }
            ));
        }
        other => panic!("expected comparison, got {other:?}"),
    }
    let expected = [
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::Divide,
        BinaryOp::Modulo,
    ];
    for (statement, expected_op) in script.event.scenes[0]
        .statements
        .iter()
        .take(expected.len())
        .zip(expected)
    {
        match statement {
            Stmt::Assignment(assignment) => {
                assert!(matches!(
                    assignment.value,
                    Expr::Binary {
                        op,
                        ..
                    } if op == expected_op
                ));
            }
            other => panic!("expected assignment, got {other:?}"),
        }
    }
}

#[test]
fn parser_reads_postfix_not_string_match_and_day_frequency() {
    let script = parse_script(
        r#"
## 条件
初期値:
好感度=5
### 否定
条件:（好感度 10以上 でない）
頻度: 1日に1回
「not」
### 含む
条件:（入力#ファイル名 「レポート」を含む）
「contains」
### 始まる
条件:（入力#ファイル名 「月次」で始まる）
「starts」
### 終わる
条件:（入力#ファイル名 「.xlsx」で終わる）
「ends」
"#,
    )
    .unwrap();

    match script.event.scenes[0].metadata.condition.as_ref().unwrap() {
        Expr::Not { expr, .. } => {
            assert!(matches!(
                expr.as_ref(),
                Expr::PostfixComparison {
                    op: ComparisonOp::Gte,
                    ..
                }
            ));
        }
        other => panic!("expected not expression, got {other:?}"),
    }
    assert_eq!(
        script.event.scenes[0].metadata.frequency,
        Some(SceneFrequency::PerDuration(std::time::Duration::from_secs(
            86_400
        )))
    );

    let expected = [
        StringMatchOp::Contains,
        StringMatchOp::StartsWith,
        StringMatchOp::EndsWith,
    ];
    for (scene, expected_op) in script.event.scenes.iter().skip(1).zip(expected) {
        match scene.metadata.condition.as_ref().unwrap() {
            Expr::StringMatch { op, .. } => assert_eq!(*op, expected_op),
            other => panic!("expected string match, got {other:?}"),
        }
    }
}

#[test]
fn parser_accepts_wave_dash_and_fullwidth_tilde_ranges() {
    for separator in ["〜", "～"] {
        let script = parse_script(&format!(
            r#"
## 範囲
### 時刻
条件: 9:00{separator}12:00
「ok」
"#
        ))
        .unwrap();

        match script.event.scenes[0].metadata.condition.as_ref().unwrap() {
            Expr::TimeRange { start, end, .. } => {
                assert_eq!(start.as_ref().map(|time| time.hour), Some(9));
                assert_eq!(start.as_ref().map(|time| time.minute), Some(0));
                assert_eq!(end.as_ref().map(|time| time.hour), Some(12));
                assert_eq!(end.as_ref().map(|time| time.minute), Some(0));
            }
            other => panic!("expected time range, got {other:?}"),
        }
    }
}

#[test]
fn diagnostics_use_character_columns_in_japanese_lines() {
    let script = parse_script(
        r#"
## 検証
### A
「あいう＜端末#OS＞」
"#,
    )
    .unwrap();
    let diagnostics = validate_script(&script, Some(&registry()));
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E-DHN-SEM-021")
        .unwrap();

    assert_eq!(diagnostic.labels[0].span.line, 4);
    assert_eq!(diagnostic.labels[0].span.column, 5);
}

#[test]
fn validator_points_condition_marker_error_at_condition_line() {
    let script = parse_script(
        r#"
## 検証
### A
合図: @x
条件:※（はい）
「x」
"#,
    )
    .unwrap();
    let diagnostics = validate_script(&script, Some(&registry()));
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E-DHN-SEM-011")
        .unwrap();
    let condition_span = script.event.scenes[0]
        .metadata
        .raw
        .condition_text
        .as_ref()
        .unwrap()
        .span;
    let signal_span = script.event.scenes[0]
        .metadata
        .raw
        .signal_text
        .as_ref()
        .unwrap()
        .span;

    assert_eq!(diagnostic.labels[0].span, condition_span);
    assert_ne!(diagnostic.labels[0].span, signal_span);
}

#[test]
fn validator_rejects_assignment_to_builtin_time_variables() {
    let script = parse_script(
        r#"
## 検証
初期値:
時=5
### A
「x」
"#,
    )
    .unwrap();
    let diagnostics = validate_script(&script, Some(&registry()));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E-DHN-SEM-049"));
}

#[test]
fn validator_warns_for_metadata_key_typos() {
    let script = parse_script(
        r#"
## 検証
### A
クールタイム: 30秒
「x」
"#,
    )
    .unwrap();
    let diagnostics = validate_script(&script, Some(&registry()));
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "W-DHN-SEM-051")
        .unwrap();

    assert!(diagnostic.message.contains("もしかして: 頻度"));
}

#[test]
fn validator_warns_and_ignores_deprecated_selection_metadata() {
    let script = parse_script(
        r#"
## 検証
### A
優先度: 強い
重み: 0
クールダウン: 永遠
「x」
"#,
    )
    .unwrap();
    let diagnostics = validate_script(&script, Some(&registry()));
    let codes = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<BTreeSet<_>>();

    assert!(codes.contains("W-DHN-SEM-061"));
    assert!(codes.contains("W-DHN-SEM-062"));
    assert!(codes.contains("W-DHN-SEM-063"));
    assert!(!codes.contains("E-DHN-SEM-012"));
    assert!(!codes.contains("E-DHN-SEM-013"));
    assert!(!codes.contains("E-DHN-SEM-014"));
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
頻度: たくさん
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
    assert!(codes.contains("E-DHN-SEM-015"));
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
async fn docs_ai_examples_match_strict_registry() {
    let interpret_script = parse_script(
        r#"
## 解釈
### 解釈の例
話者: ゆうけい
_意図 = ＜解釈 (入力#発言) 「発言の意図は何ですか」 「挨拶/質問/終了」＞

※（_意図 = 「挨拶」）なら:
「こんにちは。」
※あるいは（_意図 = 「質問」）:
「質問ですね。」
※あるいは（_意図 = 「終了」）:
「またあとで。」
※それ以外:
「うまく読み取れませんでした。」
おわり
"#,
    )
    .unwrap();

    let generate_script = parse_script(
        r#"
## 生成
### 生成の例
話者: ゆうけい
「おはようございます。」
＜生成 「朝の気分をひとことつぶやく」 「今日もいい天気ですね。」＞
"#,
    )
    .unwrap();

    for script in [&interpret_script, &generate_script] {
        let diagnostics = validate_script(script, Some(&registry()));
        assert!(
            diagnostics.is_empty(),
            "expected docs examples to validate, got {diagnostics:?}"
        );
    }

    let mut action = MockActionHandler::default();
    let mut interpret = MockInterpretHandler::with_responses(["挨拶".to_owned()]);
    let mut variables =
        InMemoryVariableStore::new().with_input("発言", DaihonValue::String("こんにちは".into()));
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions::default(),
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    interpreter.run_script(&interpret_script).await.unwrap();
    assert_eq!(interpret.requests.len(), 1);
    assert_eq!(interpret.requests[0].input_text, "こんにちは");
    assert_eq!(interpret.requests[0].question, "発言の意図は何ですか");
    assert_eq!(interpret.requests[0].choices, ["挨拶", "質問", "終了"]);
}

#[tokio::test]
async fn runtime_selects_one_triggered_scene_with_speaker_and_bareword() {
    let script = parse_script(
        r#"
## 反応
初期値:
好感度=10
### 低優先
合図: ＠クリック
「低」
### 高優先
合図: ＠クリック
条件:（好感度 10以上）
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
        generation_count: 0,
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
        generation_count: 0,
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
        generation_count: 0,
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
        generation_count: 0,
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
async fn runtime_generate_outputs_current_speaker_dialogue() {
    let script = parse_script(
        r#"
## 生成
### 通常
話者: ミカ
「固定」
＜生成 「お出かけの楽しみを一言」＞
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret =
        MockInterpretHandler::with_generate_responses([Ok(Some(GeneratedDialogue {
            text: "寄り道も楽しみ。".to_string(),
            expression: None,
            motion: None,
        }))]);
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions::default(),
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert!(result.diagnostics.is_empty());
    assert_eq!(
        interpret.generate_requests[0].instruction,
        "お出かけの楽しみを一言"
    );
    assert_eq!(
        interpret.generate_requests[0].speaker_id.as_deref(),
        Some("ミカ")
    );
    assert_eq!(
        action.dialogues,
        vec![
            (Some("ミカ".to_string()), "固定".to_string()),
            (Some("ミカ".to_string()), "寄り道も楽しみ。".to_string())
        ]
    );
}

#[tokio::test]
async fn runtime_generate_failure_uses_fallback_dialogue() {
    let script = parse_script(
        r#"
## 生成
### 通常
話者: ミカ
＜生成 「お出かけの楽しみを一言」 「楽しみだなあ」＞
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = MockInterpretHandler::with_generate_responses([Err(
        DaihonRuntimeError::new("E-TEST-GENERATE", "generate failed", Span::empty()),
    )]);
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions::default(),
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert!(result.diagnostics.is_empty());
    assert_eq!(action.dialogues[0].1, "楽しみだなあ");
}

#[tokio::test]
async fn runtime_generate_failure_without_fallback_skips_and_continues() {
    let script = parse_script(
        r#"
## 生成
### 通常
＜生成 「お出かけの楽しみを一言」＞
「続き」
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = MockInterpretHandler::with_generate_responses([Ok(None)]);
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions::default(),
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert!(result.diagnostics.is_empty());
    assert_eq!(action.dialogues.len(), 1);
    assert_eq!(action.dialogues[0].1, "続き");
}

#[tokio::test]
async fn runtime_generate_limit_warns_and_uses_fallback() {
    let script = parse_script(
        r#"
## 生成
### 通常
＜生成 「一回目」 「一回目fallback」＞
＜生成 「二回目」 「二回目fallback」＞
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret =
        MockInterpretHandler::with_generate_responses([Ok(Some(GeneratedDialogue {
            text: "一回目生成".to_string(),
            expression: None,
            motion: None,
        }))]);
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &registry(),
        options: RunOptions {
            max_generations_per_dispatch: 1,
            ..RunOptions::default()
        },
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(interpret.generate_requests.len(), 1);
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W-DHN-RUN-051"));
    assert_eq!(action.dialogues[0].1, "一回目生成");
    assert_eq!(action.dialogues[1].1, "二回目fallback");
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
        generation_count: 0,
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
async fn runtime_ignores_trailing_comments_but_keeps_dialogue_dollars() {
    let script = parse_script(
        r#"
## コメント
### 通常
好感度=1 $$ ここはコメント
「価格は$$です。好感度は＜好感度＞です。」
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
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    interpreter.run_script(&script).await.unwrap();
    assert_eq!(action.dialogues[0].1, "価格は$$です。好感度は1です。");
    assert_eq!(
        variables
            .get_value(&VariableRef::EventLocal {
                name: Spanned::new("好感度".to_owned(), Span::empty())
            })
            .unwrap(),
        DaihonValue::Number(DaihonNumber::Integer(1))
    );
}

#[tokio::test]
async fn runtime_resolves_scoped_dialogue_embeds_as_variables() {
    let script = parse_script(
        r#"
## 起動
### 通常
「今は＜入力#時間帯＞です。天気は＜全体#天気＞、ミカは＜住人#ミカ#機嫌＞。」
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = NoopInterpretHandler;
    let mut variables =
        InMemoryVariableStore::new().with_input("時間帯", DaihonValue::String("朝".to_owned()));
    variables
        .set_value(
            &VariableRef::Global {
                name: Spanned::new("天気".to_owned(), Span::empty()),
            },
            DaihonValue::String("晴れ".to_owned()),
        )
        .unwrap();
    variables
        .set_value(
            &VariableRef::Resident {
                actor: Spanned::new("ミカ".to_owned(), Span::empty()),
                name: Spanned::new("機嫌".to_owned(), Span::empty()),
            },
            DaihonValue::String("ごきげん".to_owned()),
        )
        .unwrap();
    let mut history = InMemorySceneHistory::new();
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &FunctionRegistry::permissive(),
        options: RunOptions::default(),
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert!(result.diagnostics.is_empty());
    assert_eq!(
        action.dialogues[0].1,
        "今は朝です。天気は晴れ、ミカはごきげん。"
    );
    assert!(action.calls.is_empty());
}

#[tokio::test]
async fn runtime_warns_for_missing_scoped_dialogue_embed_without_function_fallback() {
    let script = parse_script(
        r#"
## 起動
### 通常
「今は＜入力#時間帯＞です」
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
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(action.dialogues[0].1, "今はです");
    assert!(action.calls.is_empty());
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W-DHN-RUN-054"));
}

#[tokio::test]
async fn runtime_evaluates_postfix_not_condition() {
    let script = parse_script(
        r#"
## 否定
初期値:
好感度=5
### 低い
条件:（好感度 10以上 でない）
「低い」
### 高い
「高い」
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
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    interpreter.run_script(&script).await.unwrap();
    assert_eq!(action.dialogues[0].1, "低い");
}

#[tokio::test]
async fn runtime_evaluates_nested_logical_conditions_without_panicking() {
    let cases = [("（a かつ b）", "false"), ("（a かつ b）でない", "true")];

    for (condition, expected) in cases {
        let script = parse_script(&format!(
            r#"
## 論理
初期値:
a=はい
b=いいえ
### 条件
条件:（{condition}）
「true」
### 既定
「false」
"#
        ))
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
            generation_count: 0,
            diagnostics: Vec::new(),
        };

        interpreter.run_script(&script).await.unwrap();
        assert_eq!(action.dialogues[0].1, expected);
    }
}

#[tokio::test]
async fn runtime_evaluates_string_match_conditions() {
    let cases = [
        ("「レポート」を含む", "月次レポート.xlsx", "matched"),
        ("「レポート」を含む", "写真.png", "fallback"),
        ("「月次」で始まる", "月次レポート.xlsx", "matched"),
        ("「月次」で始まる", "年次レポート.xlsx", "fallback"),
        ("「.xlsx」で終わる", "月次レポート.xlsx", "matched"),
        ("「.xlsx」で終わる", "写真.png", "fallback"),
    ];

    for (condition, input, expected) in cases {
        let script = parse_script(&format!(
            r#"
## 文字列
### 一致
条件:（入力#ファイル名 {condition}）
「matched」
### 既定
「fallback」
"#
        ))
        .unwrap();
        let mut action = MockActionHandler::default();
        let mut interpret = NoopInterpretHandler;
        let mut variables = InMemoryVariableStore::new()
            .with_input("ファイル名", DaihonValue::String(input.to_owned()));
        let mut history = InMemorySceneHistory::new();
        let mut interpreter = Interpreter {
            action_handler: &mut action,
            interpret_handler: &mut interpret,
            variable_store: &mut variables,
            scene_history: &mut history,
            function_registry: &FunctionRegistry::permissive(),
            options: RunOptions::default(),
            interpretation_count: 0,
            generation_count: 0,
            diagnostics: Vec::new(),
        };

        interpreter.run_script(&script).await.unwrap();
        assert_eq!(action.dialogues[0].1, expected);
    }
}

#[tokio::test]
async fn runtime_warns_when_comparing_different_types() {
    let script = parse_script(
        r#"
## 型
初期値:
好感度=10
### 不一致
条件:（好感度 = 「高い」）
「bad」
### 既定
「ok」
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
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(action.dialogues[0].1, "ok");
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W-DHN-RUN-052"));
}

#[tokio::test]
async fn runtime_warns_and_treats_non_boolean_conditions_as_false() {
    let script = parse_script(
        r#"
## 真偽
初期値:
好感度=10
### 非bool
条件:（好感度）
「bad」
### 既定
「ok」
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
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(action.dialogues[0].1, "ok");
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W-DHN-RUN-055"));
}

#[tokio::test]
async fn runtime_reports_number_overflow_without_panicking() {
    let scripts = [
        r#"
## 桁あふれ
### 加算
結果=9223372036854775807 + 1
"#,
        r#"
## 桁あふれ
### 乗算
結果=9223372036854775807 * 2
"#,
        r#"
## 桁あふれ
### 単項
結果=-(0 - 9223372036854775807 - 1)
"#,
    ];

    for source in scripts {
        let script = parse_script(source).unwrap();
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
            generation_count: 0,
            diagnostics: Vec::new(),
        };

        let err = interpreter.run_script(&script).await.unwrap_err();
        assert_eq!(err.diagnostic.code, "E-DHN-RUN-045");
    }
}

#[tokio::test]
async fn runtime_falls_back_to_default_and_respects_frequency() {
    let script = parse_script(
        r#"
## 既定
### クリック
合図: ＠クリック
「クリック」
### 既定A
頻度: 10分に1回
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
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(result.selected_scene.unwrap().name, "既定B");
}

#[tokio::test]
async fn runtime_respects_once_frequency() {
    let script = parse_script(
        r#"
## 記念
### 初回
頻度: 一度きり
「初回」
### 通常
「通常」
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = NoopInterpretHandler;
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    history.record_executed("記念", "初回", fixed_now("2026-06-27T10:00:00+09:00"));
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &FunctionRegistry::permissive(),
        options: RunOptions {
            now: Some(fixed_now("2026-06-27T10:05:00+09:00")),
            ..RunOptions::default()
        },
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(result.selected_scene.unwrap().name, "通常");
}

#[tokio::test]
async fn runtime_prefers_more_specific_conjunction() {
    let script = parse_script(
        r#"
## 朝
初期値:
誕生日=はい
### 朝だけ
条件:（時 6~11）
「朝」
### 誕生日の朝
条件:（時 6~11 かつ 誕生日）
「誕生日」
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
            now: Some(fixed_now("2026-06-27T08:00:00+09:00")),
            ..RunOptions::default()
        },
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(result.selected_scene.unwrap().name, "誕生日の朝");
}

#[tokio::test]
async fn runtime_avoids_last_scene_for_same_event_when_possible() {
    let script = parse_script(
        r#"
## 雑談
### A
「A」
### B
「B」
"#,
    )
    .unwrap();
    let mut action = MockActionHandler::default();
    let mut interpret = NoopInterpretHandler;
    let mut variables = InMemoryVariableStore::new();
    let mut history = InMemorySceneHistory::new();
    history.record_executed("雑談", "A", fixed_now("2026-06-27T10:00:00+09:00"));
    let mut interpreter = Interpreter {
        action_handler: &mut action,
        interpret_handler: &mut interpret,
        variable_store: &mut variables,
        scene_history: &mut history,
        function_registry: &FunctionRegistry::permissive(),
        options: RunOptions {
            random_seed: Some(0),
            ..RunOptions::default()
        },
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(result.selected_scene.unwrap().name, "B");
}

#[tokio::test]
async fn runtime_uniform_pick_is_seeded_and_ignores_deprecated_weight() {
    let script = parse_script(
        r#"
## 抽選
### A
重み: 1000
「A」
### B
重み: 1
「B」
### C
重み: 1
「C」
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
            random_seed: Some(2),
            ..RunOptions::default()
        },
        interpretation_count: 0,
        generation_count: 0,
        diagnostics: Vec::new(),
    };

    let result = interpreter.run_script(&script).await.unwrap();
    assert_eq!(result.selected_scene.unwrap().name, "C");
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W-DHN-SEM-062"));
}

#[tokio::test]
async fn runtime_supports_overnight_time_range() {
    let script = parse_script(
        r#"
## 深夜
### 夜
条件:（22:00~02:00）
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
        generation_count: 0,
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
