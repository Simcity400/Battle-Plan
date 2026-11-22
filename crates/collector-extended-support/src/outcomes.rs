use replay_data::scope::CaptureMode;

use crate::clock::qpc;
use engine_native::walk::Outcome;
use replay_data::rows::TableId;

pub const MISSING: Outcome = Outcome::Unavailable {
    reason: "acquisition source not supplied",
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaidReadCounts {
    pub torn_memo_reads: u32,
    pub refused_leaders: u32,
}

pub fn failed_capture(
    table: TableId,
    hour: i32,
    force_full: bool,
    reason: &'static str,
) -> Outcome {
    Outcome::Unavailable {
        reason: if replay_data::cadence::should_walk(table, hour, force_full) {
            reason
        } else {
            replay_data::cadence::SKIPPED_REASON
        },
    }
}

pub fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

pub fn truncated_source(len: usize, omitted: usize) -> Outcome {
    match u32::try_from(omitted) {
        Ok(unread_rows) if unread_rows > 0 => Outcome::Truncated { len, unread_rows },
        _ => Outcome::Unavailable {
            reason: "source omitted row count invalid or exceeds u32",
        },
    }
}

pub const NOT_RECORDED: &str = "not recorded in this capture mode";

pub fn records_any(mode: CaptureMode, tables: &[TableId]) -> bool {
    tables
        .iter()
        .any(|&table| replay_data::scope::records(mode, table))
}
