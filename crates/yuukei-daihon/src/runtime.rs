use std::cmp::Ordering;
use std::collections::BTreeMap;

use async_recursion::async_recursion;
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Local, Timelike};
use rand::distributions::{Distribution, WeightedIndex};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::ast::*;
use crate::diagnostic::{DaihonDiagnostic, DaihonRuntimeError};
use crate::function::{FunctionRegistry, ValidationMode};
use crate::span::Span;
use crate::validate::{has_errors, validate_script};
use crate::value::{DaihonNumber, DaihonValue};
use crate::variable::{builtin_time_value, VariableRef, VariableStore};

pub const INTERPRET_FUNCTION_NAME: &str = "解釈";
pub const GENERATE_FUNCTION_NAME: &str = "生成";
pub const UNKNOWN_INTERPRETATION: &str = "不明";
pub const DEFAULT_MAX_INTERPRETATIONS_PER_DISPATCH: usize = 4;
pub const DEFAULT_MAX_GENERATIONS_PER_DISPATCH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedDialogue {
    pub text: String,
    pub expression: Option<String>,
    pub motion: Option<String>,
}

#[async_trait]
pub trait ActionHandler {
    async fn show_dialogue(
        &mut self,
        speaker_id: Option<&str>,
        text: &str,
    ) -> Result<(), DaihonRuntimeError>;

    async fn show_generated_dialogue(
        &mut self,
        speaker_id: Option<&str>,
        dialogue: GeneratedDialogue,
        _instruction: &str,
    ) -> Result<(), DaihonRuntimeError> {
        self.show_dialogue(speaker_id, &dialogue.text).await
    }

    async fn call_function(
        &mut self,
        speaker_id: Option<&str>,
        name: &str,
        positional: Vec<DaihonValue>,
        named: BTreeMap<String, DaihonValue>,
    ) -> Result<DaihonValue, DaihonRuntimeError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpretRequest {
    pub input_text: String,
    pub question: String,
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateRequest {
    pub instruction: String,
    pub speaker_id: Option<String>,
}

#[async_trait]
pub trait InterpretHandler {
    async fn interpret(&mut self, request: InterpretRequest) -> Result<String, DaihonRuntimeError>;

    async fn generate(
        &mut self,
        _request: GenerateRequest,
    ) -> Result<Option<GeneratedDialogue>, DaihonRuntimeError> {
        Ok(None)
    }
}

#[derive(Debug, Default)]
pub struct NoopInterpretHandler;

#[async_trait]
impl InterpretHandler for NoopInterpretHandler {
    async fn interpret(
        &mut self,
        _request: InterpretRequest,
    ) -> Result<String, DaihonRuntimeError> {
        Ok(UNKNOWN_INTERPRETATION.to_string())
    }
}

pub trait SceneHistoryStore {
    fn last_executed_at(&self, event_name: &str, scene_name: &str)
        -> Option<DateTime<FixedOffset>>;

    fn record_executed(&mut self, event_name: &str, scene_name: &str, at: DateTime<FixedOffset>);
}

#[derive(Debug, Default, Clone)]
pub struct InMemorySceneHistory {
    entries: BTreeMap<(String, String), DateTime<FixedOffset>>,
}

impl InMemorySceneHistory {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SceneHistoryStore for InMemorySceneHistory {
    fn last_executed_at(
        &self,
        event_name: &str,
        scene_name: &str,
    ) -> Option<DateTime<FixedOffset>> {
        self.entries
            .get(&(event_name.to_owned(), scene_name.to_owned()))
            .copied()
    }

    fn record_executed(&mut self, event_name: &str, scene_name: &str, at: DateTime<FixedOffset>) {
        self.entries
            .insert((event_name.to_owned(), scene_name.to_owned()), at);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOptions {
    pub trigger: Option<SystemEvent>,
    pub default_speaker: Option<String>,
    pub random_seed: Option<u64>,
    pub max_jumps: usize,
    pub max_interpretations_per_dispatch: usize,
    pub max_generations_per_dispatch: usize,
    pub now: Option<DateTime<FixedOffset>>,
    pub validation_mode: ValidationMode,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            trigger: None,
            default_speaker: None,
            random_seed: None,
            max_jumps: 1000,
            max_interpretations_per_dispatch: DEFAULT_MAX_INTERPRETATIONS_PER_DISPATCH,
            max_generations_per_dispatch: DEFAULT_MAX_GENERATIONS_PER_DISPATCH,
            now: None,
            validation_mode: ValidationMode::Strict,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaihonRunResult {
    pub event_name: String,
    pub selected_scene: Option<DaihonExecutedScene>,
    pub completed: bool,
    pub diagnostics: Vec<DaihonDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaihonExecutedScene {
    pub name: String,
    pub trigger: Option<SystemEvent>,
    pub priority: i32,
    pub weight: u32,
    pub started_at: DateTime<FixedOffset>,
    pub ended_at: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlFlow {
    Continue,
    EndScene,
    EndEvent,
    JumpScene(String),
}

pub struct Interpreter<'a, A, V, H, I> {
    pub action_handler: &'a mut A,
    pub interpret_handler: &'a mut I,
    pub variable_store: &'a mut V,
    pub scene_history: &'a mut H,
    pub function_registry: &'a FunctionRegistry,
    pub options: RunOptions,
    pub interpretation_count: usize,
    pub generation_count: usize,
    pub diagnostics: Vec<DaihonDiagnostic>,
}

impl<'a, A, V, H, I> Interpreter<'a, A, V, H, I>
where
    A: ActionHandler + Send,
    I: InterpretHandler + Send,
    V: VariableStore + Send,
    H: SceneHistoryStore + Send,
{
    pub async fn run_script(
        &mut self,
        script: &Script,
    ) -> Result<DaihonRunResult, DaihonRuntimeError> {
        self.interpretation_count = 0;
        self.generation_count = 0;
        self.diagnostics.clear();
        let mut registry = self.function_registry.clone();
        registry.set_mode(self.options.validation_mode);
        let diagnostics = validate_script(script, Some(&registry));
        if has_errors(&diagnostics) {
            return Ok(DaihonRunResult {
                event_name: script.event.name.value.clone(),
                selected_scene: None,
                completed: false,
                diagnostics,
            });
        }

        let now = self.now();
        for assignment in &script.event.defaults {
            let value = self.eval_expr(&assignment.value, None).await?;
            self.variable_store
                .set_default_value(&assignment.target, value)?;
        }

        for precondition in &script.event.preconditions {
            if self.eval_condition(&precondition.condition, None).await? {
                match self.execute_stmt(&precondition.action, None).await? {
                    ControlFlow::EndEvent => {
                        self.variable_store.clear_temporary_variables();
                        return Ok(DaihonRunResult {
                            event_name: script.event.name.value.clone(),
                            selected_scene: None,
                            completed: true,
                            diagnostics: std::mem::take(&mut self.diagnostics),
                        });
                    }
                    ControlFlow::Continue | ControlFlow::EndScene | ControlFlow::JumpScene(_) => {}
                }
            }
        }

        let Some(scene) = self.select_scene(script).await? else {
            self.variable_store.clear_temporary_variables();
            return Ok(DaihonRunResult {
                event_name: script.event.name.value.clone(),
                selected_scene: None,
                completed: true,
                diagnostics: std::mem::take(&mut self.diagnostics),
            });
        };

        let mut executed = DaihonExecutedScene {
            name: scene.name.value.clone(),
            trigger: self.options.trigger.clone(),
            priority: scene.metadata.priority,
            weight: scene.metadata.weight,
            started_at: now,
            ended_at: None,
        };

        let mut jumps = 0usize;
        let mut current_scene = scene;
        loop {
            let speaker_owned = current_scene
                .metadata
                .speaker
                .as_ref()
                .map(|speaker| speaker.value.clone())
                .or_else(|| self.options.default_speaker.clone());
            match self
                .execute_scene(current_scene, speaker_owned.as_deref())
                .await?
            {
                ControlFlow::Continue | ControlFlow::EndScene => break,
                ControlFlow::EndEvent => break,
                ControlFlow::JumpScene(target) => {
                    jumps += 1;
                    if jumps > self.options.max_jumps {
                        return Err(DaihonRuntimeError::new(
                            "E-DHN-RUN-030",
                            "ジャンプ回数が上限を超えました。無限ループの可能性があります。",
                            current_scene.span,
                        ));
                    }
                    current_scene = script
                        .event
                        .scenes
                        .iter()
                        .find(|scene| scene.name.value == target)
                        .ok_or_else(|| {
                            DaihonRuntimeError::new(
                                "E-DHN-RUN-031",
                                format!("ジャンプ先シーン「{target}」が見つかりません。"),
                                current_scene.span,
                            )
                        })?;
                }
            }
        }
        let ended_at = self.now();
        executed.ended_at = Some(ended_at);
        self.scene_history
            .record_executed(&script.event.name.value, &executed.name, ended_at);
        self.variable_store.clear_temporary_variables();

        Ok(DaihonRunResult {
            event_name: script.event.name.value.clone(),
            selected_scene: Some(executed),
            completed: true,
            diagnostics: std::mem::take(&mut self.diagnostics),
        })
    }

    async fn select_scene<'s>(
        &mut self,
        script: &'s Script,
    ) -> Result<Option<&'s Scene>, DaihonRuntimeError> {
        let mut candidates = Vec::<&Scene>::new();
        for scene in &script.event.scenes {
            let signal_match = match &self.options.trigger {
                Some(trigger) => {
                    !scene.metadata.signals.is_empty()
                        && scene
                            .metadata
                            .signals
                            .iter()
                            .any(|signal| signal.name.value == trigger.name.value)
                }
                None => scene.metadata.signals.is_empty(),
            };
            if !signal_match {
                continue;
            }
            if let Some(condition) = &scene.metadata.condition {
                if !self.eval_condition(condition, None).await? {
                    continue;
                }
            }
            candidates.push(scene);
        }

        if candidates.is_empty() {
            candidates.extend(script.event.scenes.iter().filter(|scene| {
                scene.metadata.signals.is_empty() && scene.metadata.condition.is_none()
            }));
        }

        let now = self.now();
        candidates.retain(|scene| {
            let Some(cooldown) = scene.metadata.cooldown else {
                return true;
            };
            let Some(last) = self
                .scene_history
                .last_executed_at(&script.event.name.value, &scene.name.value)
            else {
                return true;
            };
            match now.signed_duration_since(last).to_std() {
                Ok(elapsed) => elapsed >= cooldown,
                Err(_) => false,
            }
        });

        if candidates.is_empty() {
            return Ok(None);
        }
        let max_priority = candidates
            .iter()
            .map(|scene| scene.metadata.priority)
            .max()
            .unwrap_or(0);
        candidates.retain(|scene| scene.metadata.priority == max_priority);
        Ok(weighted_pick(&candidates, self.options.random_seed))
    }

    async fn execute_scene(
        &mut self,
        scene: &Scene,
        speaker: Option<&str>,
    ) -> Result<ControlFlow, DaihonRuntimeError> {
        for statement in &scene.statements {
            match self.execute_stmt(statement, speaker).await? {
                ControlFlow::Continue => {}
                flow => return Ok(flow),
            }
        }
        Ok(ControlFlow::Continue)
    }

    #[async_recursion]
    async fn execute_stmt(
        &mut self,
        statement: &Stmt,
        speaker: Option<&str>,
    ) -> Result<ControlFlow, DaihonRuntimeError> {
        match statement {
            Stmt::Display(display) => self.execute_display(display, speaker).await,
            Stmt::SpeakerDisplay {
                speaker: line_speaker,
                display,
            } => {
                self.execute_display(display, Some(&line_speaker.value))
                    .await
            }
            Stmt::Assignment(assignment) => {
                let value = self.eval_expr(&assignment.value, speaker).await?;
                self.variable_store.set_value(&assignment.target, value)?;
                Ok(ControlFlow::Continue)
            }
            Stmt::Jump(jump) => Ok(match jump {
                JumpTarget::EndEvent { .. } => ControlFlow::EndEvent,
                JumpTarget::EndScene { .. } => ControlFlow::EndScene,
                JumpTarget::Scene { name } => ControlFlow::JumpScene(name.value.clone()),
            }),
            Stmt::Conditional(block) => self.execute_conditional(block, speaker).await,
        }
    }

    async fn execute_display(
        &mut self,
        display: &DisplayLine,
        speaker: Option<&str>,
    ) -> Result<ControlFlow, DaihonRuntimeError> {
        for part in &display.parts {
            match part {
                DisplayPart::Dialogue(dialogue) => {
                    let text = self.render_dialogue(dialogue, speaker).await?;
                    self.action_handler.show_dialogue(speaker, &text).await?;
                }
                DisplayPart::FunctionCall(function) => {
                    self.call_function(function, speaker).await?;
                }
            }
        }
        Ok(ControlFlow::Continue)
    }

    #[async_recursion]
    async fn execute_conditional(
        &mut self,
        block: &ConditionalBlock,
        speaker: Option<&str>,
    ) -> Result<ControlFlow, DaihonRuntimeError> {
        for branch in &block.branches {
            if self.eval_condition(&branch.condition, speaker).await? {
                for stmt in &branch.statements {
                    match self.execute_stmt(stmt, speaker).await? {
                        ControlFlow::Continue => {}
                        flow => return Ok(flow),
                    }
                }
                return Ok(ControlFlow::Continue);
            }
        }
        if let Some(else_branch) = &block.else_branch {
            for stmt in else_branch {
                match self.execute_stmt(stmt, speaker).await? {
                    ControlFlow::Continue => {}
                    flow => return Ok(flow),
                }
            }
        }
        Ok(ControlFlow::Continue)
    }

    async fn render_dialogue(
        &mut self,
        dialogue: &Dialogue,
        speaker: Option<&str>,
    ) -> Result<String, DaihonRuntimeError> {
        let mut output = String::new();
        for part in &dialogue.parts {
            match part {
                DialoguePart::Text(text) => output.push_str(&text.value),
                DialoguePart::Embed(function) => {
                    let variable_ref = VariableRef::EventLocal {
                        name: function.name.clone(),
                    };
                    if self.variable_store.is_defined(&variable_ref) {
                        let value = self.get_variable(&variable_ref)?;
                        output.push_str(&value.to_display_string());
                    } else {
                        let value = self.call_function(function, speaker).await?;
                        output.push_str(&value.to_display_string());
                    }
                }
            }
        }
        Ok(output)
    }

    #[async_recursion]
    async fn eval_condition(
        &mut self,
        expr: &Expr,
        speaker: Option<&str>,
    ) -> Result<bool, DaihonRuntimeError> {
        match expr {
            Expr::Binary {
                left,
                op: BinaryOp::And,
                right,
                ..
            } => Ok(self.eval_condition(left, speaker).await?
                && self.eval_condition(right, speaker).await?),
            Expr::Binary {
                left,
                op: BinaryOp::Or,
                right,
                ..
            } => Ok(self.eval_condition(left, speaker).await?
                || self.eval_condition(right, speaker).await?),
            Expr::Comparison {
                left, op, right, ..
            } => {
                let left = self.eval_expr(left, speaker).await?;
                let right = self.eval_expr(right, speaker).await?;
                Ok(compare_values(&left, *op, &right))
            }
            Expr::PostfixComparison {
                left, op, value, ..
            } => {
                let left = self.eval_expr(left, speaker).await?;
                let right = self.eval_expr(value, speaker).await?;
                Ok(compare_values(&left, *op, &right))
            }
            Expr::Range {
                left, start, end, ..
            } => {
                let value = self.eval_expr(left, speaker).await?;
                let Some(value) = value.as_number() else {
                    return Ok(false);
                };
                if let Some(start) = start {
                    let Some(start) = self.eval_expr(start, speaker).await?.as_number() else {
                        return Ok(false);
                    };
                    if value.as_f64() < start.as_f64() {
                        return Ok(false);
                    }
                }
                if let Some(end) = end {
                    let Some(end) = self.eval_expr(end, speaker).await?.as_number() else {
                        return Ok(false);
                    };
                    if value.as_f64() > end.as_f64() {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Expr::TimeRange { start, end, .. } => {
                let now = self.now();
                let current = (now.hour() * 60 + now.minute()) as i32;
                Ok(match (start, end) {
                    (Some(start), Some(end)) => {
                        let from = start.total_minutes();
                        let to = end.total_minutes();
                        if from <= to {
                            current >= from && current <= to
                        } else {
                            current >= from || current <= to
                        }
                    }
                    (Some(start), None) => current >= start.total_minutes(),
                    (None, Some(end)) => current <= end.total_minutes(),
                    (None, None) => true,
                })
            }
            Expr::Truthy { expr, .. } => match self.eval_expr(expr, speaker).await? {
                DaihonValue::Boolean(value) => Ok(value),
                other => Err(DaihonRuntimeError::new(
                    "E-DHN-RUN-040",
                    format!(
                        "{:?} 型の値は真偽値として評価できません。",
                        other.value_type()
                    ),
                    expr.span(),
                )),
            },
            other => match self.eval_expr(other, speaker).await? {
                DaihonValue::Boolean(value) => Ok(value),
                _ => Ok(false),
            },
        }
    }

    #[async_recursion]
    async fn eval_expr(
        &mut self,
        expr: &Expr,
        speaker: Option<&str>,
    ) -> Result<DaihonValue, DaihonRuntimeError> {
        match expr {
            Expr::Value(value) => Ok(value.value.clone()),
            Expr::Variable(reference) => self.get_variable(reference),
            Expr::FunctionCall(function) => self.call_function(function, speaker).await,
            Expr::Unary { op, expr, span } => {
                let value = self.eval_expr(expr, speaker).await?;
                let Some(number) = value.as_number() else {
                    return Err(DaihonRuntimeError::new(
                        "E-DHN-RUN-041",
                        "単項演算子は数値にだけ使えます。",
                        *span,
                    ));
                };
                Ok(DaihonValue::Number(match op {
                    UnaryOp::Plus => number,
                    UnaryOp::Minus => match number {
                        DaihonNumber::Integer(value) => DaihonNumber::Integer(-value),
                        DaihonNumber::Float(value) => DaihonNumber::Float(-value),
                    },
                }))
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                let left_value = self.eval_expr(left, speaker).await?;
                let right_value = self.eval_expr(right, speaker).await?;
                self.eval_binary(left_value, *op, right_value, *span)
            }
            Expr::Truthy { expr, .. } => self.eval_expr(expr, speaker).await,
            Expr::Comparison { .. }
            | Expr::PostfixComparison { .. }
            | Expr::Range { .. }
            | Expr::TimeRange { .. } => Ok(DaihonValue::Boolean(
                self.eval_condition(expr, speaker).await?,
            )),
        }
    }

    fn eval_binary(
        &self,
        left: DaihonValue,
        op: BinaryOp,
        right: DaihonValue,
        span: Span,
    ) -> Result<DaihonValue, DaihonRuntimeError> {
        match op {
            BinaryOp::Add => {
                if matches!(left, DaihonValue::String(_)) || matches!(right, DaihonValue::String(_))
                {
                    return Ok(DaihonValue::String(format!(
                        "{}{}",
                        left.to_display_string(),
                        right.to_display_string()
                    )));
                }
                match (left.as_number(), right.as_number()) {
                    (Some(left), Some(right)) => Ok(DaihonValue::Number(left.checked_add(right))),
                    _ => Err(DaihonRuntimeError::new(
                        "E-DHN-RUN-042",
                        "+ は数値加算または文字列結合にだけ使えます。",
                        span,
                    )),
                }
            }
            BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide | BinaryOp::Modulo => {
                let (Some(left), Some(right)) = (left.as_number(), right.as_number()) else {
                    return Err(DaihonRuntimeError::new(
                        "E-DHN-RUN-043",
                        "算術演算は数値同士でだけ使えます。",
                        span,
                    ));
                };
                let result = match op {
                    BinaryOp::Subtract => Some(left.checked_sub(right)),
                    BinaryOp::Multiply => Some(left.checked_mul(right)),
                    BinaryOp::Divide => left.checked_div(right),
                    BinaryOp::Modulo => left.checked_rem(right),
                    _ => None,
                };
                result.map(DaihonValue::Number).ok_or_else(|| {
                    DaihonRuntimeError::new("E-DHN-RUN-044", "0で除算することはできません。", span)
                })
            }
            BinaryOp::And | BinaryOp::Or => unreachable!("logical operators are conditions"),
        }
    }

    fn get_variable(&self, reference: &VariableRef) -> Result<DaihonValue, DaihonRuntimeError> {
        if let Some(value) = builtin_time_value(reference, self.now()) {
            return Ok(value);
        }
        self.variable_store.get_value(reference)
    }

    #[async_recursion]
    async fn call_function(
        &mut self,
        function: &FunctionCall,
        speaker: Option<&str>,
    ) -> Result<DaihonValue, DaihonRuntimeError> {
        let mut positional = Vec::new();
        for arg in &function.positional {
            positional.push(self.eval_func_arg(arg, speaker).await?);
        }
        let mut named = BTreeMap::new();
        for (name, arg) in &function.named {
            named.insert(name.clone(), self.eval_func_arg(arg, speaker).await?);
        }
        if function.name.value == INTERPRET_FUNCTION_NAME {
            return self.call_interpret(function, positional, named).await;
        }
        if function.name.value == GENERATE_FUNCTION_NAME {
            return self
                .call_generate(function, speaker, positional, named)
                .await;
        }
        self.action_handler
            .call_function(speaker, &function.name.value, positional, named)
            .await
    }

    async fn call_interpret(
        &mut self,
        function: &FunctionCall,
        positional: Vec<DaihonValue>,
        named: BTreeMap<String, DaihonValue>,
    ) -> Result<DaihonValue, DaihonRuntimeError> {
        if !named.is_empty() || positional.len() < 3 {
            return Ok(DaihonValue::String(UNKNOWN_INTERPRETATION.to_string()));
        }
        if self.interpretation_count >= self.options.max_interpretations_per_dispatch {
            self.diagnostics.push(
                DaihonDiagnostic::warning(
                    "W-DHN-RUN-050",
                    "解釈関数の呼び出し回数が上限を超えました。",
                    function.span,
                )
                .with_help("同じdispatch内の以後の解釈結果は「不明」になります。"),
            );
            return Ok(DaihonValue::String(UNKNOWN_INTERPRETATION.to_string()));
        }
        self.interpretation_count += 1;

        let input_text = positional[0].to_display_string();
        let question = positional[1].to_display_string();
        let choices = parse_interpret_choices(&positional[2].to_display_string());
        if choices.is_empty() {
            return Ok(DaihonValue::String(UNKNOWN_INTERPRETATION.to_string()));
        }
        let request = InterpretRequest {
            input_text,
            question,
            choices: choices.clone(),
        };
        let Ok(choice) = self.interpret_handler.interpret(request).await else {
            return Ok(DaihonValue::String(UNKNOWN_INTERPRETATION.to_string()));
        };
        let choice = choice.trim();
        if choice == UNKNOWN_INTERPRETATION || choices.iter().any(|candidate| candidate == choice) {
            Ok(DaihonValue::String(choice.to_string()))
        } else {
            Ok(DaihonValue::String(UNKNOWN_INTERPRETATION.to_string()))
        }
    }

    async fn call_generate(
        &mut self,
        function: &FunctionCall,
        speaker: Option<&str>,
        positional: Vec<DaihonValue>,
        named: BTreeMap<String, DaihonValue>,
    ) -> Result<DaihonValue, DaihonRuntimeError> {
        if !named.is_empty() || positional.is_empty() {
            return Ok(DaihonValue::None);
        }
        let instruction = positional[0].to_display_string();
        let fallback = positional
            .get(1)
            .map(DaihonValue::to_display_string)
            .filter(|value| !value.trim().is_empty());
        if self.generation_count >= self.options.max_generations_per_dispatch {
            self.diagnostics.push(
                DaihonDiagnostic::warning(
                    "W-DHN-RUN-051",
                    "生成関数の呼び出し回数が上限を超えました。",
                    function.span,
                )
                .with_help(
                    "同じdispatch内の以後の生成行はフォールバックまたはスキップになります。",
                ),
            );
            if let Some(fallback) = fallback {
                self.action_handler
                    .show_dialogue(speaker, &fallback)
                    .await?;
            }
            return Ok(DaihonValue::None);
        }
        self.generation_count += 1;

        let request = GenerateRequest {
            instruction: instruction.clone(),
            speaker_id: speaker.map(ToOwned::to_owned),
        };
        let generated = self.interpret_handler.generate(request).await;
        match generated {
            Ok(Some(dialogue)) if !dialogue.text.trim().is_empty() => {
                self.action_handler
                    .show_generated_dialogue(speaker, dialogue, &instruction)
                    .await?;
            }
            _ => {
                if let Some(fallback) = fallback {
                    self.action_handler
                        .show_dialogue(speaker, &fallback)
                        .await?;
                }
            }
        }
        Ok(DaihonValue::None)
    }

    #[async_recursion]
    async fn eval_func_arg(
        &mut self,
        arg: &FuncArg,
        speaker: Option<&str>,
    ) -> Result<DaihonValue, DaihonRuntimeError> {
        match arg {
            FuncArg::BareWord(word) => Ok(DaihonValue::String(word.value.clone())),
            FuncArg::Expr(expr) => self.eval_expr(expr, speaker).await,
        }
    }

    fn now(&self) -> DateTime<FixedOffset> {
        self.options.now.unwrap_or_else(|| {
            let local = Local::now();
            local.with_timezone(local.offset())
        })
    }
}

pub fn parse_interpret_choices(text: &str) -> Vec<String> {
    text.split(['|', '/', '／', '、', ',', '，'])
        .map(str::trim)
        .filter(|choice| !choice.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn compare_values(left: &DaihonValue, op: ComparisonOp, right: &DaihonValue) -> bool {
    let Some(ordering) = left.compare_same_type(right) else {
        return false;
    };
    match op {
        ComparisonOp::Eq => ordering == Ordering::Equal,
        ComparisonOp::Ne => ordering != Ordering::Equal,
        ComparisonOp::Lt => ordering == Ordering::Less,
        ComparisonOp::Lte => matches!(ordering, Ordering::Less | Ordering::Equal),
        ComparisonOp::Gt => ordering == Ordering::Greater,
        ComparisonOp::Gte => matches!(ordering, Ordering::Greater | Ordering::Equal),
    }
}

fn weighted_pick<'a>(scenes: &[&'a Scene], seed: Option<u64>) -> Option<&'a Scene> {
    if scenes.is_empty() {
        return None;
    }
    let weights = scenes
        .iter()
        .map(|scene| scene.metadata.weight.max(1))
        .collect::<Vec<_>>();
    let distribution = WeightedIndex::new(&weights).ok()?;
    let index = match seed {
        Some(seed) => {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            distribution.sample(&mut rng)
        }
        None => {
            let mut rng = rand::thread_rng();
            distribution.sample(&mut rng)
        }
    };
    scenes.get(index).copied()
}
