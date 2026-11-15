//! Log counts: the rows a log inserted, counted per bin and facet from its summary pieces.

use replay::columnar_frames::Recording;
use replay_data::rows::TableId;
use std::io;

/// The logged rows of run `run`'s `table` inserted through `from..=to`, by facet key, per bin of
/// `bin_hours` (a multiple of six) on the absolute grid; only the facet keys `keep` takes.
pub fn counts(recording: &Recording, run: u32, table: TableId, (from, to): (i32, i32), bin_hours: i64, keep: &dyn Fn(u128) -> bool) -> io::Result<Vec<(u128, Vec<u64>)>> {
    crate::cost::storage(|| replay::lanes::log_of(recording, run, table, (from, to), bin_hours, keep))
}
