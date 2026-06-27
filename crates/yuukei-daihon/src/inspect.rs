use std::collections::BTreeSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ast::*;
use crate::diagnostic::DaihonDiagnostic;
use crate::parser::parse_script;
use crate::variable::VariableRef;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaihonScriptInfo {
    pub event_name: String,
    pub scenes: Vec<DaihonSceneInfo>,
    pub variables_read: Vec<VariableRef>,
    pub variables_written: Vec<VariableRef>,
    pub functions_called: Vec<String>,
    pub signals: Vec<SystemEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaihonSceneInfo {
    pub name: String,
    pub signals: Vec<SystemEvent>,
    pub has_condition: bool,
    pub priority: i32,
    pub weight: u32,
    pub cooldown: Option<Duration>,
    pub speaker: Option<String>,
}

pub fn inspect_script(source: &str) -> Result<DaihonScriptInfo, Vec<DaihonDiagnostic>> {
    let script = parse_script(source)?;
    Ok(inspect_ast(&script))
}

pub fn inspect_ast(script: &Script) -> DaihonScriptInfo {
    let mut visitor = InspectVisitor::default();
    visitor.visit_script(script);
    DaihonScriptInfo {
        event_name: script.event.name.value.clone(),
        scenes: script
            .event
            .scenes
            .iter()
            .map(|scene| DaihonSceneInfo {
                name: scene.name.value.clone(),
                signals: scene.metadata.signals.clone(),
                has_condition: scene.metadata.condition.is_some(),
                priority: scene.metadata.priority,
                weight: scene.metadata.weight,
                cooldown: scene.metadata.cooldown,
                speaker: scene
                    .metadata
                    .speaker
                    .as_ref()
                    .map(|speaker| speaker.value.clone()),
            })
            .collect(),
        variables_read: visitor.variables_read.into_iter().collect(),
        variables_written: visitor.variables_written.into_iter().collect(),
        functions_called: visitor.functions_called.into_iter().collect(),
        signals: script
            .event
            .scenes
            .iter()
            .flat_map(|scene| scene.metadata.signals.clone())
            .collect(),
    }
}

#[derive(Default)]
struct InspectVisitor {
    variables_read: BTreeSet<VariableRef>,
    variables_written: BTreeSet<VariableRef>,
    functions_called: BTreeSet<String>,
}

impl InspectVisitor {
    fn visit_script(&mut self, script: &Script) {
        for assignment in &script.event.defaults {
            self.variables_written.insert(assignment.target.clone());
            self.visit_expr(&assignment.value);
        }
        for precondition in &script.event.preconditions {
            self.visit_expr(&precondition.condition);
            self.visit_stmt(&precondition.action);
        }
        for scene in &script.event.scenes {
            if let Some(condition) = &scene.metadata.condition {
                self.visit_expr(condition);
            }
            for stmt in &scene.statements {
                self.visit_stmt(stmt);
            }
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Display(display) | Stmt::SpeakerDisplay { display, .. } => {
                for part in &display.parts {
                    match part {
                        DisplayPart::Dialogue(dialogue) => {
                            for part in &dialogue.parts {
                                if let DialoguePart::Embed(function) = part {
                                    self.visit_function(function);
                                }
                            }
                        }
                        DisplayPart::FunctionCall(function) => self.visit_function(function),
                    }
                }
            }
            Stmt::Assignment(assignment) => {
                self.variables_written.insert(assignment.target.clone());
                self.visit_expr(&assignment.value);
            }
            Stmt::Jump(_) => {}
            Stmt::Conditional(block) => {
                for branch in &block.branches {
                    self.visit_expr(&branch.condition);
                    for stmt in &branch.statements {
                        self.visit_stmt(stmt);
                    }
                }
                if let Some(else_branch) = &block.else_branch {
                    for stmt in else_branch {
                        self.visit_stmt(stmt);
                    }
                }
            }
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Variable(reference) => {
                self.variables_read.insert(reference.clone());
            }
            Expr::FunctionCall(function) => self.visit_function(function),
            Expr::Unary { expr, .. } | Expr::Truthy { expr, .. } => self.visit_expr(expr),
            Expr::Binary { left, right, .. }
            | Expr::Comparison { left, right, .. }
            | Expr::PostfixComparison {
                left, value: right, ..
            } => {
                self.visit_expr(left);
                self.visit_expr(right);
            }
            Expr::Range {
                left, start, end, ..
            } => {
                self.visit_expr(left);
                if let Some(start) = start {
                    self.visit_expr(start);
                }
                if let Some(end) = end {
                    self.visit_expr(end);
                }
            }
            Expr::Value(_) | Expr::TimeRange { .. } => {}
        }
    }

    fn visit_function(&mut self, function: &FunctionCall) {
        self.functions_called.insert(function.name.value.clone());
        for arg in &function.positional {
            if let FuncArg::Expr(expr) = arg {
                self.visit_expr(expr);
            }
        }
        for arg in function.named.values() {
            if let FuncArg::Expr(expr) = arg {
                self.visit_expr(expr);
            }
        }
    }
}
