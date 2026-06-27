use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::span::{Span, Spanned};
use crate::value::DaihonValue;
use crate::variable::VariableRef;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SystemEvent {
    pub name: Spanned<String>,
}

impl SystemEvent {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: Spanned::new(name.into(), span),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub event: Event,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub name: Spanned<String>,
    pub preconditions: Vec<ConditionalStmt>,
    pub defaults: Vec<Assignment>,
    pub scenes: Vec<Scene>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub name: Spanned<String>,
    pub metadata: SceneMetadata,
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneMetadata {
    pub signals: Vec<SystemEvent>,
    pub condition: Option<Expr>,
    pub priority: i32,
    pub weight: u32,
    pub cooldown: Option<Duration>,
    pub speaker: Option<Spanned<String>>,
    pub raw: SceneMetadataRaw,
}

impl Default for SceneMetadata {
    fn default() -> Self {
        Self {
            signals: Vec::new(),
            condition: None,
            priority: 0,
            weight: 1,
            cooldown: None,
            speaker: None,
            raw: SceneMetadataRaw::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneMetadataRaw {
    pub signal_text: Option<Spanned<String>>,
    pub signal_used_and: bool,
    pub condition_had_marker: bool,
    pub priority_text: Option<Spanned<String>>,
    pub weight_text: Option<Spanned<String>>,
    pub cooldown_text: Option<Spanned<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Stmt {
    Display(DisplayLine),
    SpeakerDisplay {
        speaker: Spanned<String>,
        display: DisplayLine,
    },
    Assignment(Box<Assignment>),
    Jump(JumpTarget),
    Conditional(ConditionalBlock),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalStmt {
    pub condition: Expr,
    pub action: Box<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalBlock {
    pub branches: Vec<ConditionalBranch>,
    pub else_branch: Option<Vec<Stmt>>,
    pub span: Span,
    pub one_line: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalBranch {
    pub condition: Expr,
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub target: VariableRef,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JumpTarget {
    EndEvent { span: Span },
    EndScene { span: Span },
    Scene { name: Spanned<String> },
}

impl JumpTarget {
    pub fn span(&self) -> Span {
        match self {
            Self::EndEvent { span } | Self::EndScene { span } => *span,
            Self::Scene { name } => name.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayLine {
    pub parts: Vec<DisplayPart>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisplayPart {
    Dialogue(Dialogue),
    FunctionCall(FunctionCall),
}

impl DisplayPart {
    pub fn span(&self) -> Span {
        match self {
            Self::Dialogue(dialogue) => dialogue.span,
            Self::FunctionCall(function) => function.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dialogue {
    pub parts: Vec<DialoguePart>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DialoguePart {
    Text(Spanned<String>),
    Embed(FunctionCall),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: Spanned<String>,
    pub positional: Vec<FuncArg>,
    pub named: BTreeMap<String, FuncArg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FuncArg {
    BareWord(Spanned<String>),
    Expr(Expr),
}

impl FuncArg {
    pub fn span(&self) -> Span {
        match self {
            Self::BareWord(value) => value.span,
            Self::Expr(expr) => expr.span(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    Value(Spanned<DaihonValue>),
    Variable(VariableRef),
    FunctionCall(FunctionCall),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
    Truthy {
        expr: Box<Expr>,
        span: Span,
    },
    Comparison {
        left: Box<Expr>,
        op: ComparisonOp,
        right: Box<Expr>,
        span: Span,
    },
    PostfixComparison {
        left: Box<Expr>,
        value: Box<Expr>,
        op: ComparisonOp,
        span: Span,
    },
    Range {
        left: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        span: Span,
    },
    TimeRange {
        start: Option<TimeOfDay>,
        end: Option<TimeOfDay>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::Value(value) => value.span,
            Self::Variable(reference) => reference.span(),
            Self::FunctionCall(function) => function.span,
            Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::Truthy { span, .. }
            | Self::Comparison { span, .. }
            | Self::PostfixComparison { span, .. }
            | Self::Range { span, .. }
            | Self::TimeRange { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Plus,
    Minus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonOp {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
    pub span: Span,
}

impl TimeOfDay {
    pub fn total_minutes(self) -> i32 {
        i32::from(self.hour) * 60 + i32::from(self.minute)
    }
}
