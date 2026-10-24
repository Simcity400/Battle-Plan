use super::*;
use replay_data::rows::{LexicalTokenRow, tables};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandStatus<'a> {
    Complete,
    WrongLayout,
    TruncatedWrongLayout(u32),
    Truncated(u32),
    Unavailable(&'a str),
    NotRecorded(crate::scope::CaptureMode),
}

impl<'a> OperandStatus<'a> {
    pub fn for_table(view: &'a TableView, table: TableId) -> Self {
        match view {
            TableView::Complete(rows) if rows.table() != table => Self::WrongLayout,
            TableView::Truncated { rows, missing } if rows.table() != table => {
                Self::TruncatedWrongLayout(*missing)
            }
            _ => Self::of(view),
        }
    }
    pub fn of(view: &'a TableView) -> Self {
        match view {
            TableView::Complete(_) => Self::Complete,
            TableView::Truncated { missing, .. } => Self::Truncated(*missing),
            TableView::Unavailable { reason } => Self::Unavailable(reason),
            TableView::NotRecorded { mode } => Self::NotRecorded(*mode),
        }
    }
    /// Whole-table acquisition evidence, independent of the row layout.
    pub fn is_table_complete(self) -> bool {
        matches!(self, Self::Complete | Self::WrongLayout)
    }
    pub fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
    pub fn complete(self, hour: Hour, table: TableId) -> io::Result<()> {
        let reason = match self {
            Self::Complete => return Ok(()),
            Self::WrongLayout => format!("hour {hour} {table:?} contains another table's rows"),
            Self::Truncated(missing) | Self::TruncatedWrongLayout(missing) => {
                format!("hour {hour} {table:?} is truncated ({missing} missing rows)")
            }
            Self::Unavailable(reason) => format!("hour {hour} {table:?} is unavailable: {reason}"),
            Self::NotRecorded(mode) => {
                format!("hour {hour} {table:?} is not recorded in a {mode} recording")
            }
        };
        Err(crate::codec::invalid(reason))
    }
    pub fn gap(self) -> String {
        match self {
            Self::Complete => "the complete table has no row for it".into(),
            Self::WrongLayout | Self::TruncatedWrongLayout(_) => {
                "the table contains another table's rows".into()
            }
            Self::Truncated(missing) => format!("the table is truncated ({missing} rows missing)"),
            Self::Unavailable(reason) => format!("the table is unavailable: {reason}"),
            Self::NotRecorded(mode) => format!("the table is not recorded in a {mode} recording"),
        }
    }
}

#[derive(Clone)]
pub struct SharedLexical(SharedView);
impl std::fmt::Debug for SharedLexical {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedLexical")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}
impl SharedLexical {
    pub fn rows(&self) -> Rows<'_, LexicalTokenRow> {
        lexical_rows(&self.0)
    }
    pub fn status(&self) -> OperandStatus<'_> {
        OperandStatus::for_table(&self.0, TableId::LexicalTokens)
    }
    pub fn holds(&self, view: &SharedView) -> bool {
        self.0 == *view
    }
}
impl From<SharedView> for SharedLexical {
    fn from(view: SharedView) -> Self {
        Self(view)
    }
}
pub(super) fn lexical_rows(view: &TableView) -> Rows<'_, LexicalTokenRow> {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows
            .rows::<tables::LexicalTokens>()
            .unwrap_or_default()
            .into(),
        _ => Rows::empty(),
    }
}
