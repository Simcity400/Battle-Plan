//! Grouped-table decode work on the calling thread, for consumer tests.
use super::{BlockIndex, TableId};
use std::cell::RefCell;

type Decodes = Vec<(u64, i32, TableId, usize)>;

thread_local! {
    static DECODES: RefCell<Option<Decodes>> = const { RefCell::new(None) };
}

pub(super) fn decoded(block: &BlockIndex, table: TableId, hours: usize) {
    DECODES.with(|counts| {
        if let Some(counts) = &mut *counts.borrow_mut() {
            counts.push((block.branch, block.first_hour, table, hours));
        }
    });
}

/// Each grouped table decoded by `run`, in order, as (branch, block's first
/// hour, table, hours decoded). Keyframe and later entries form one table decode; anchor
/// reads are part of it. Other threads and sections the decoder skips are excluded.
pub fn table_decodes<T>(run: impl FnOnce() -> T) -> (T, Decodes) {
    struct Reset(Option<Decodes>);
    impl Drop for Reset {
        fn drop(&mut self) {
            DECODES.with(|counts| *counts.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(DECODES.with(|counts| counts.replace(Some(Vec::new()))));
    let result = run();
    let counts = DECODES.with(|counts| counts.take().unwrap());
    (result, counts)
}
