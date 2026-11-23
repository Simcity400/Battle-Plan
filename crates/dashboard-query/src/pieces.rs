
use replay_recording::columnar_frames::Recording;
use replay_data::rows::TableId;
use std::io;

pub fn counts(recording: &Recording, run: u32, table: TableId, (from, to): (i32, i32), bin_hours: i64, keep: &dyn Fn(u128) -> bool) -> io::Result<Vec<(u128, Vec<u64>)>> {
    crate::cost::storage(|| replay_lanes::lanes::log_of(recording, run, table, (from, to), bin_hours, keep))
}
