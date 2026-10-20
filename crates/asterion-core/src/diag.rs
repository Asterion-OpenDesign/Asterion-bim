//! Structured failures produced while a transaction is open.

use crate::id::ElementId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The change is kept; the user is informed.
    Warning,
    /// The change cannot be committed as is; the transaction rolls back unless resolved.
    Error,
    DocumentCorruption,
}

impl Severity {
    /// The reference product's literal of the severity (1 Warning, 2 Error, 3 DocumentCorruption).
    pub fn literal(self) -> u8 {
        match self {
            Severity::Warning => 1,
            Severity::Error => 2,
            Severity::DocumentCorruption => 3,
        }
    }

    /// The commit outcome: anything but a Warning rolls the transaction back.
    pub fn rolls_back(self) -> bool {
        !matches!(self, Severity::Warning)
    }
}

/// One failure message. `code` is a stable ASTERION identifier (never a copy of a
/// reference product's internal id); `reference_hint` may record the observed
/// reference message for traceability in compatibility reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<ElementId>,
}

impl Failure {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Failure { severity: Severity::Error, code: code.to_owned(), message: message.into(), elements: Vec::new() }
    }

    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Failure { severity: Severity::Warning, code: code.to_owned(), message: message.into(), elements: Vec::new() }
    }

    pub fn on(mut self, ids: &[ElementId]) -> Self {
        self.elements.extend_from_slice(ids);
        self
    }
}
