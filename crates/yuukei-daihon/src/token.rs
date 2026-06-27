use serde::{Deserialize, Serialize};

use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    EventHeader,
    SceneHeader,
    HeaderName,
    Newline,
    Identifier,
    Number,
    Time,
    Boolean,
    Keyword,
    Operator,
    At,
    Colon,
    Dot,
    ConditionMarker,
    Arrow,
    DialogueOpen,
    DialogueText,
    DialogueClose,
    DialogueEscape,
    FunctionOpen,
    FunctionClose,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub kind: TokenKind,
    pub normalized: Option<String>,
    pub original: String,
    pub span: Span,
}

impl Token {
    pub fn new(
        kind: TokenKind,
        original: impl Into<String>,
        normalized: Option<String>,
        span: Span,
    ) -> Self {
        Self {
            kind,
            normalized,
            original: original.into(),
            span,
        }
    }
}
