use crate::rows::{TABLE_COUNT, TableId};
pub use replay_rows::measurement::*;

impl Table for TableId {
    fn position(self) -> usize {
        self.index()
    }
    fn at(position: usize) -> Option<Self> {
        Self::ALL.get(position).copied()
    }
}

pub fn enable() {
    replay_rows::measurement::enable(TABLE_COUNT);
}

pub fn snapshot() -> Vec<(Option<TableId>, Vec<(&'static str, u64)>)> {
    replay_rows::measurement::snapshot()
}

pub fn source_masks() -> Vec<(TableId, TableId, u64)> {
    replay_rows::measurement::source_masks()
}
