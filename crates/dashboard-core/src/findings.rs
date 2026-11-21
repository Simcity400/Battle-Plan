
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
    Content,
    Offline,
}

impl From<content_reader_core::DiagnosticKind> for Kind {
    fn from(kind: content_reader_core::DiagnosticKind) -> Self {
        match kind {
            content_reader_core::DiagnosticKind::ParseFailure => Self::ParseFailure,
            content_reader_core::DiagnosticKind::Misread => Self::Misread,
            content_reader_core::DiagnosticKind::Unsettled => Self::Unsettled,
            content_reader_core::DiagnosticKind::Unsupported => Self::Unsupported,
            content_reader_core::DiagnosticKind::Refused => Self::Refused,
            content_reader_core::DiagnosticKind::Ambiguous => Self::Ambiguous,
            content_reader_core::DiagnosticKind::Fallback => Self::Fallback,
            content_reader_core::DiagnosticKind::Logged => Self::Logged,
            content_reader_core::DiagnosticKind::Warning => Self::Warning,
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

    pub fn reader(subject: &str, (kind, message): content_reader_core::Diagnostic) -> Self {
        Self::new(kind.into(), subject, message)
    }

    pub fn aggregate_reason(&self) -> Option<&str> {
        self.aggregate_reason.as_deref()
    }
}
