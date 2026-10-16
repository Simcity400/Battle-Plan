//! Content diagnostics: what a reader or product check found, by kind.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    ParseFailure,
    Misread,
    Unsettled,
    Logged,
    Refused,
    Unsupported,
    Ambiguous,
    Fallback,
    Warning,
    /// Content behavior matching the game.
    Content,
    /// A recording supplies the operands; aggregated once per product/reason.
    Offline,
}

impl From<installed_content_core::DiagnosticKind> for Kind {
    fn from(kind: installed_content_core::DiagnosticKind) -> Self {
        match kind {
            installed_content_core::DiagnosticKind::ParseFailure => Self::ParseFailure,
            installed_content_core::DiagnosticKind::Misread => Self::Misread,
            installed_content_core::DiagnosticKind::Unsettled => Self::Unsettled,
            installed_content_core::DiagnosticKind::Unsupported => Self::Unsupported,
            installed_content_core::DiagnosticKind::Refused => Self::Refused,
            installed_content_core::DiagnosticKind::Ambiguous => Self::Ambiguous,
            installed_content_core::DiagnosticKind::Fallback => Self::Fallback,
            installed_content_core::DiagnosticKind::Logged => Self::Logged,
            installed_content_core::DiagnosticKind::Warning => Self::Warning,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub kind: Kind,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub subject: String,
    pub message: String,
    /// Inventory details are outside the diagnostic's diff identity.
    pub count: Option<usize>,
    pub examples: Vec<String>,
    aggregate_reason: Option<String>,
}

impl Finding {
    pub fn new(kind: Kind, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            file: None,
            line: None,
            subject: subject.into(),
            message: message.into(),
            count: None,
            examples: Vec::new(),
            aggregate_reason: None,
        }
    }

    pub fn grouped(
        kind: Kind,
        subject: impl Into<String>,
        reason: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            aggregate_reason: Some(reason.into()),
            ..Self::new(kind, subject, message)
        }
    }

    pub fn reader(subject: &str, (kind, message): installed_content_core::Diagnostic) -> Self {
        Self::new(kind.into(), subject, message)
    }

    /// The reason findings are counted together under, when they are.
    pub fn aggregate_reason(&self) -> Option<&str> {
        self.aggregate_reason.as_deref()
    }
}
