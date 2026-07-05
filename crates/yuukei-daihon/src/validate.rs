use std::collections::{BTreeMap, BTreeSet};

use crate::ast::*;
use crate::diagnostic::{DaihonDiagnostic, Severity};
use crate::function::FunctionRegistry;
use crate::runtime::{INTERPRET_FUNCTION_NAME, UNKNOWN_INTERPRETATION};
use crate::value::DaihonValue;
use crate::variable::VariableRef;

pub fn validate_script(
    script: &Script,
    registry: Option<&FunctionRegistry>,
) -> Vec<DaihonDiagnostic> {
    let mut validator = Validator {
        diagnostics: Vec::new(),
        scenes: BTreeSet::new(),
        registry,
    };
    validator.validate(script);
    validator.diagnostics
}

#[derive(Clone, Copy)]
enum FunctionContext {
    Statement,
    Expr,
}

struct Validator<'a> {
    diagnostics: Vec<DaihonDiagnostic>,
    scenes: BTreeSet<String>,
    registry: Option<&'a FunctionRegistry>,
}

impl<'a> Validator<'a> {
    fn validate(&mut self, script: &Script) {
        if script.event.name.value.trim().is_empty() {
            self.diagnostics.push(DaihonDiagnostic::error(
                "E-DHN-SEM-001",
                "イベント名が空です。",
                script.event.name.span,
            ));
        }

        let mut seen = BTreeMap::<String, _>::new();
        for scene in &script.event.scenes {
            if scene.name.value.trim().is_empty() {
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SEM-004",
                    "シーン名が空です。",
                    scene.name.span,
                ));
            }
            if let Some(previous) = seen.insert(scene.name.value.clone(), scene.name.span) {
                self.diagnostics.push(
                    DaihonDiagnostic::error(
                        "E-DHN-SEM-005",
                        format!("シーン名「{}」が重複しています。", scene.name.value),
                        scene.name.span,
                    )
                    .with_label(previous, "最初の定義はここです。"),
                );
            }
            self.scenes.insert(scene.name.value.clone());
        }
        if script.event.scenes.is_empty() {
            self.diagnostics.push(DaihonDiagnostic::error(
                "E-DHN-SEM-003",
                "シーンが1つもありません。",
                script.event.span,
            ));
        }

        for assignment in &script.event.defaults {
            self.validate_assignment(assignment);
        }
        for precondition in &script.event.preconditions {
            self.validate_conditional_stmt(precondition);
        }
        for scene in &script.event.scenes {
            self.validate_scene(scene);
        }
    }

    fn validate_scene(&mut self, scene: &Scene) {
        if scene.metadata.raw.signal_used_and {
            if let Some(raw) = &scene.metadata.raw.signal_text {
                self.diagnostics.push(
                    DaihonDiagnostic::error(
                        "E-DHN-SEM-010",
                        "合図: では かつ を使用できません。",
                        raw.span,
                    )
                    .with_help("複数の合図は または で列挙してください。"),
                );
            }
        }
        if scene.metadata.raw.condition_had_marker {
            self.diagnostics.push(
                DaihonDiagnostic::error(
                    "E-DHN-SEM-011",
                    "条件: 行では ※ は不要です。",
                    scene
                        .metadata
                        .raw
                        .signal_text
                        .as_ref()
                        .map(|raw| raw.span)
                        .unwrap_or(scene.span),
                )
                .with_help("条件:（好感度 10 以上）のように書いてください。"),
            );
        }
        if let Some(raw) = &scene.metadata.raw.priority_text {
            if raw.value.parse::<i32>().is_err() {
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SEM-012",
                    "優先度には数値を指定してください。",
                    raw.span,
                ));
            }
        }
        if let Some(raw) = &scene.metadata.raw.weight_text {
            if raw.value.parse::<i32>().map(|v| v <= 0).unwrap_or(true) {
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SEM-013",
                    "重みには1以上の数値を指定してください。",
                    raw.span,
                ));
            }
        }
        if let Some(raw) = &scene.metadata.raw.cooldown_text {
            if scene.metadata.cooldown.is_none() {
                self.diagnostics.push(
                    DaihonDiagnostic::error(
                        "E-DHN-SEM-014",
                        "クールダウンの単位が不正です。",
                        raw.span,
                    )
                    .with_help("秒、分、時間、s、m、h のいずれかを使ってください。"),
                );
            }
        }
        if let Some(condition) = &scene.metadata.condition {
            self.validate_expr(condition, FunctionContext::Expr);
        }
        for statement in &scene.statements {
            self.validate_stmt(statement);
        }
        self.validate_interpret_consumption(scene);
    }

    fn validate_interpret_consumption(&mut self, scene: &Scene) {
        for (index, statement) in scene.statements.iter().enumerate() {
            let Stmt::Assignment(assignment) = statement else {
                continue;
            };
            if !expr_is_interpret_call(&assignment.value) {
                continue;
            }
            let consumed = scene
                .statements
                .iter()
                .skip(index + 1)
                .any(|statement| stmt_has_interpret_consumer(statement, &assignment.target));
            if !consumed {
                self.diagnostics.push(
                    DaihonDiagnostic::error(
                        "E-DHN-SEM-048",
                        format!(
                            "解釈結果「{}」は同じシーン内の条件分岐で不明時の枝まで処理してください。",
                            assignment.target.display_name()
                        ),
                        assignment.span,
                    )
                    .with_help(
                        "後続に ※（判定 = 不明）なら: または ※それ以外: を持つ条件分岐を追加してください。",
                    ),
                );
            }
        }
    }

    fn validate_conditional_stmt(&mut self, stmt: &ConditionalStmt) {
        self.validate_expr(&stmt.condition, FunctionContext::Expr);
        self.validate_stmt(&stmt.action);
    }

    fn validate_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Display(display) => self.validate_display(display),
            Stmt::SpeakerDisplay { display, .. } => self.validate_display(display),
            Stmt::Assignment(assignment) => self.validate_assignment(assignment),
            Stmt::Jump(jump) => self.validate_jump(jump),
            Stmt::Conditional(block) => self.validate_conditional_block(block),
        }
    }

    fn validate_display(&mut self, display: &DisplayLine) {
        for part in &display.parts {
            match part {
                DisplayPart::Dialogue(dialogue) => {
                    for part in &dialogue.parts {
                        if let DialoguePart::Embed(function) = part {
                            if !function.positional.is_empty() || !function.named.is_empty() {
                                self.validate_function(function, FunctionContext::Statement);
                            }
                        }
                    }
                }
                DisplayPart::FunctionCall(function) => {
                    self.validate_function(function, FunctionContext::Statement)
                }
            }
        }
    }

    fn validate_assignment(&mut self, assignment: &Assignment) {
        self.validate_variable_ref(&assignment.target, true);
        if assignment.target.is_read_only() {
            self.diagnostics.push(
                DaihonDiagnostic::error(
                    "E-DHN-SEM-020",
                    format!("{} には代入できません。", assignment.target.display_name()),
                    assignment.target.span(),
                )
                .with_help("入力はTauri側から渡される読み取り専用の値です。"),
            );
        }
        self.validate_expr(&assignment.value, FunctionContext::Expr);
    }

    fn validate_jump(&mut self, jump: &JumpTarget) {
        if let JumpTarget::Scene { name } = jump {
            if !self.scenes.contains(&name.value) {
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SEM-006",
                    format!("ジャンプ先シーン「{}」が存在しません。", name.value),
                    name.span,
                ));
            }
        }
    }

    fn validate_conditional_block(&mut self, block: &ConditionalBlock) {
        if block.one_line {
            if let Some(branch) = block.branches.first() {
                if branch.statements.len() != 1 {
                    self.diagnostics.push(DaihonDiagnostic::error(
                        "E-DHN-SEM-034",
                        "条件付き1行記法ではアクションは1つだけです。",
                        block.span,
                    ));
                }
                if let Some(Stmt::Display(display)) = branch.statements.first() {
                    if display.parts.len() > 1 {
                        self.diagnostics.push(DaihonDiagnostic::error(
                            "E-DHN-SEM-034",
                            "条件付き1行記法では表示要素も1つだけです。",
                            display.span,
                        ));
                    }
                }
            }
        }
        for branch in &block.branches {
            self.validate_expr(&branch.condition, FunctionContext::Expr);
            for stmt in &branch.statements {
                self.validate_stmt(stmt);
            }
        }
        if let Some(else_branch) = &block.else_branch {
            for stmt in else_branch {
                self.validate_stmt(stmt);
            }
        }
    }

    fn validate_expr(&mut self, expr: &Expr, context: FunctionContext) {
        match expr {
            Expr::Value(_) => {}
            Expr::Variable(reference) => self.validate_variable_ref(reference, false),
            Expr::FunctionCall(function) => self.validate_function(function, context),
            Expr::Unary { expr, .. } | Expr::Truthy { expr, .. } => {
                self.validate_expr(expr, FunctionContext::Expr)
            }
            Expr::Binary { left, right, .. } | Expr::Comparison { left, right, .. } => {
                self.validate_expr(left, FunctionContext::Expr);
                self.validate_expr(right, FunctionContext::Expr);
            }
            Expr::PostfixComparison { left, value, .. } => {
                if !matches!(**left, Expr::Variable(_)) {
                    self.diagnostics.push(
                        DaihonDiagnostic::error(
                            "E-DHN-SEM-030",
                            "後置記法の左辺に算術式は使えません。",
                            left.span(),
                        )
                        .with_help("好感度 + 10 >= 50 のように中置記法を使ってください。"),
                    );
                }
                self.validate_expr(left, FunctionContext::Expr);
                self.validate_expr(value, FunctionContext::Expr);
            }
            Expr::Range {
                left, start, end, ..
            } => {
                self.validate_expr(left, FunctionContext::Expr);
                if let Some(start) = start {
                    self.validate_expr(start, FunctionContext::Expr);
                }
                if let Some(end) = end {
                    self.validate_expr(end, FunctionContext::Expr);
                }
            }
            Expr::TimeRange { .. } => {}
        }
    }

    fn validate_function(&mut self, function: &FunctionCall, context: FunctionContext) {
        for arg in &function.positional {
            if let FuncArg::Expr(expr) = arg {
                self.validate_expr(expr, FunctionContext::Expr);
            }
        }
        for arg in function.named.values() {
            if let FuncArg::Expr(expr) = arg {
                self.validate_expr(expr, FunctionContext::Expr);
            }
        }
        if let Some(registry) = self.registry {
            self.diagnostics
                .extend(registry.validate_call(function, matches!(context, FunctionContext::Expr)));
        }
    }

    fn validate_variable_ref(&mut self, reference: &VariableRef, _write: bool) {
        if let VariableRef::Unsupported { scope, parts } = reference {
            let scope_name = scope.value.as_str();
            let display = reference.display_name();
            let (code, message, help) = match scope_name {
                "端末" => (
                    "E-DHN-SEM-021",
                    format!("{display} はDaihon完成版では使用できません。"),
                    "端末情報を渡したい場合は 入力#名前 を使ってください。",
                ),
                "世界" => (
                    "E-DHN-SEM-021",
                    format!("{display} はDaihon完成版では使用できません。"),
                    "環境情報を渡したい場合は 入力#名前 を使ってください。",
                ),
                _ if parts.len() == 1 => (
                    "E-DHN-SEM-022",
                    "イベント名#変数名 による他イベント参照は使用できません。".to_owned(),
                    "共有したい状態は 全体#名前 を使ってください。",
                ),
                _ => (
                    "E-DHN-SEM-023",
                    format!("{display} は使用できない変数スコープです。"),
                    "採用スコープは name, _name, 全体#name, 入力#name, 住人#actor#name, 関係#a#b#name です。",
                ),
            };
            self.diagnostics
                .push(DaihonDiagnostic::error(code, message, reference.span()).with_help(help));
        }
    }
}

fn expr_is_interpret_call(expr: &Expr) -> bool {
    matches!(expr, Expr::FunctionCall(function) if function.name.value == INTERPRET_FUNCTION_NAME)
}

fn stmt_has_interpret_consumer(stmt: &Stmt, variable: &VariableRef) -> bool {
    match stmt {
        Stmt::Conditional(block) => {
            conditional_consumes_interpret(block, variable)
                || block.branches.iter().any(|branch| {
                    branch
                        .statements
                        .iter()
                        .any(|stmt| stmt_has_interpret_consumer(stmt, variable))
                })
                || block.else_branch.as_ref().is_some_and(|statements| {
                    statements
                        .iter()
                        .any(|stmt| stmt_has_interpret_consumer(stmt, variable))
                })
        }
        Stmt::Display(_) | Stmt::SpeakerDisplay { .. } | Stmt::Assignment(_) | Stmt::Jump(_) => {
            false
        }
    }
}

fn conditional_consumes_interpret(block: &ConditionalBlock, variable: &VariableRef) -> bool {
    let uses_variable = block
        .branches
        .iter()
        .any(|branch| expr_uses_variable(&branch.condition, variable));
    if !uses_variable {
        return false;
    }
    block.else_branch.is_some()
        || block.branches.iter().any(|branch| {
            expr_uses_variable(&branch.condition, variable)
                && expr_contains_string(&branch.condition, UNKNOWN_INTERPRETATION)
        })
}

fn expr_uses_variable(expr: &Expr, variable: &VariableRef) -> bool {
    match expr {
        Expr::Variable(reference) => reference.display_name() == variable.display_name(),
        Expr::Value(_) | Expr::TimeRange { .. } => false,
        Expr::FunctionCall(function) => {
            function.positional.iter().any(|arg| match arg {
                FuncArg::Expr(expr) => expr_uses_variable(expr, variable),
                FuncArg::BareWord(_) => false,
            }) || function.named.values().any(|arg| match arg {
                FuncArg::Expr(expr) => expr_uses_variable(expr, variable),
                FuncArg::BareWord(_) => false,
            })
        }
        Expr::Unary { expr, .. } | Expr::Truthy { expr, .. } => expr_uses_variable(expr, variable),
        Expr::Binary { left, right, .. } | Expr::Comparison { left, right, .. } => {
            expr_uses_variable(left, variable) || expr_uses_variable(right, variable)
        }
        Expr::PostfixComparison { left, value, .. } => {
            expr_uses_variable(left, variable) || expr_uses_variable(value, variable)
        }
        Expr::Range {
            left, start, end, ..
        } => {
            expr_uses_variable(left, variable)
                || start
                    .as_ref()
                    .is_some_and(|expr| expr_uses_variable(expr, variable))
                || end
                    .as_ref()
                    .is_some_and(|expr| expr_uses_variable(expr, variable))
        }
    }
}

fn expr_contains_string(expr: &Expr, needle: &str) -> bool {
    match expr {
        Expr::Value(value) => matches!(&value.value, DaihonValue::String(text) if text == needle),
        Expr::Variable(_) | Expr::TimeRange { .. } => false,
        Expr::FunctionCall(function) => {
            function.positional.iter().any(|arg| match arg {
                FuncArg::Expr(expr) => expr_contains_string(expr, needle),
                FuncArg::BareWord(word) => word.value == needle,
            }) || function.named.values().any(|arg| match arg {
                FuncArg::Expr(expr) => expr_contains_string(expr, needle),
                FuncArg::BareWord(word) => word.value == needle,
            })
        }
        Expr::Unary { expr, .. } | Expr::Truthy { expr, .. } => expr_contains_string(expr, needle),
        Expr::Binary { left, right, .. } | Expr::Comparison { left, right, .. } => {
            expr_contains_string(left, needle) || expr_contains_string(right, needle)
        }
        Expr::PostfixComparison { left, value, .. } => {
            expr_contains_string(left, needle) || expr_contains_string(value, needle)
        }
        Expr::Range {
            left, start, end, ..
        } => {
            expr_contains_string(left, needle)
                || start
                    .as_ref()
                    .is_some_and(|expr| expr_contains_string(expr, needle))
                || end
                    .as_ref()
                    .is_some_and(|expr| expr_contains_string(expr, needle))
        }
    }
}

pub fn has_errors(diagnostics: &[DaihonDiagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
}
