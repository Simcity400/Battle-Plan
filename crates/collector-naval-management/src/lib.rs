pub mod bases;
pub mod changes;
pub mod forces;

#[cfg(test)]
mod admission_tests;

use replay_data::rows::{TableId, TableRows};
use replay_frame::frame::{TableInput, TableView};
use std::sync::Arc;

#[derive(Clone)]
pub struct Snapshot<const N: usize> {
    pub tables: [Result<Arc<TableRows>, &'static str>; N],
}
impl<const N: usize> Default for Snapshot<N> {
    fn default() -> Self {
        Self {
            tables: std::array::from_fn(|_| Err("naval management census not acquired")),
        }
    }
}
impl<const N: usize> Snapshot<N> {
    pub fn inputs(&self) -> [TableInput; N] {
        self.tables.each_ref().map(|table| match table {
            Ok(rows) => TableInput::SharedComplete(Arc::clone(rows)),
            Err(reason) => TableView::Unavailable {
                reason: (*reason).into(),
            }
            .into(),
        })
    }
    pub fn outcomes(&self) -> [engine_native_memory::walk::Outcome; N] {
        self.tables.each_ref().map(|table| match table {
            Ok(rows) => engine_native_memory::walk::Outcome::Rows { len: rows.len() },
            Err(reason) => engine_native_memory::walk::Outcome::Unavailable { reason },
        })
    }
    pub fn insert(&self, ids: [TableId; N], tables: &mut [TableInput]) {
        for (id, input) in ids.into_iter().zip(self.inputs()) {
            tables[id.index()] = input;
        }
    }
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: collector_test_support::FailingAllocator =
    collector_test_support::FailingAllocator;
