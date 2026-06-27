use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticLabel {
    pub span: Span,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaihonDiagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
    pub help: Option<String>,
}

impl DaihonDiagnostic {
    pub fn error(code: impl Into<String>, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            code: code.into(),
            message: message.into(),
            labels: vec![DiagnosticLabel {
                span,
                message: None,
            }],
            help: None,
        }
    }

    pub fn warning(code: impl Into<String>, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            code: code.into(),
            message: message.into(),
            labels: vec![DiagnosticLabel {
                span,
                message: None,
            }],
            help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(DiagnosticLabel {
            span,
            message: Some(message.into()),
        });
        self
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct DaihonRuntimeError {
    pub diagnostic: Box<DaihonDiagnostic>,
    pub message: String,
}

impl DaihonRuntimeError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, span: Span) -> Self {
        let message = message.into();
        Self {
            diagnostic: Box::new(DaihonDiagnostic::error(code, message.clone(), span)),
            message,
        }
    }

    pub fn from_diagnostic(diagnostic: DaihonDiagnostic) -> Self {
        let message = diagnostic.message.clone();
        Self {
            diagnostic: Box::new(diagnostic),
            message,
        }
    }
}
