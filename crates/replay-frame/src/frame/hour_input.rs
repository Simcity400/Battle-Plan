use super::{ChangeFacts, DecodedFrame, TableView, ViewIdentity};
use crate::rows::TableId;

#[derive(Clone, Copy)]
pub struct HourInput<'a> {
    frame: &'a DecodedFrame,
    changes: HourChanges<'a>,
}
impl<'a> HourInput<'a> {
    pub fn frame(self) -> &'a DecodedFrame {
        self.frame
    }
    pub fn changes(self) -> HourChanges<'a> {
        self.changes
    }
    pub fn new(frame: &'a DecodedFrame) -> Self {
        Self {
            frame,
            changes: HourChanges { frame },
        }
    }
}
#[derive(Clone, Copy)]
pub struct HourChanges<'a> {
    frame: &'a DecodedFrame,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeDiscontinuity {
    Initial,
    SnapshotSeekOrSkippedEdit,
    Partial,
    Unavailable,
    NotRecorded,
}
#[derive(Clone, Copy, Debug)]
pub enum TableChanges<'a> {
    Unchanged,
    Rows(&'a ChangeFacts),
    Discontinuity(ChangeDiscontinuity),
}
impl<'a> HourChanges<'a> {
    pub fn input(self) -> HourInput<'a> {
        HourInput::new(self.frame)
    }
    pub fn since(
        self,
        table: TableId,
        previous: Option<&ViewIdentity>,
        fields: u64,
    ) -> TableChanges<'a> {
        let view = &self.frame.tables[table.index()];
        let Some(previous) = previous else {
            return TableChanges::Discontinuity(ChangeDiscontinuity::Initial);
        };
        if previous.fields_unchanged(view, fields) {
            return TableChanges::Unchanged;
        }
        if let Some(changes) = view.changes_for(previous, fields) {
            return TableChanges::Rows(changes);
        }
        TableChanges::Discontinuity(match &**view {
            TableView::Complete(_) => ChangeDiscontinuity::SnapshotSeekOrSkippedEdit,
            TableView::Truncated { .. } => ChangeDiscontinuity::Partial,
            TableView::Unavailable { .. } => ChangeDiscontinuity::Unavailable,
            TableView::NotRecorded { .. } => ChangeDiscontinuity::NotRecorded,
        })
    }
}
