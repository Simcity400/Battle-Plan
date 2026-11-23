#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod coverage;
pub mod crt;
pub mod fields;
pub mod fixed5;
pub mod parse_counts;
pub mod parser;
pub mod tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticKind {
    ParseFailure,
    Misread,
    Unsettled,
    Unsupported,
    Refused,
    Ambiguous,
    Fallback,
    Logged,
    Warning,
}

pub type Diagnostic = (DiagnosticKind, String);

pub trait DiagnosticMessage: Sized {
    fn from_diagnostic(note: Diagnostic) -> Self;
}

impl DiagnosticMessage for Diagnostic {
    fn from_diagnostic(note: Diagnostic) -> Self {
        note
    }
}

impl DiagnosticMessage for String {
    fn from_diagnostic((_, message): Diagnostic) -> Self {
        message
    }
}
