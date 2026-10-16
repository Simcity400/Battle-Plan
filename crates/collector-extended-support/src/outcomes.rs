//! Outcome and timing helpers the extended capture families share.

use crate::clock::qpc;
use engine::walk::Outcome;
use replay::rows::TableId;

pub const MISSING: Outcome = Outcome::Unavailable {
    reason: "acquisition source not supplied",
};

/// One hour's raid reads that were torn or refused (`status.json`
/// `raid_reads`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaidReadCounts {
    /// Path memo copies a game writer overlapped (`raid_path_memo`).
    pub torn_memo_reads: u32,
    /// Raid, cooldown and target rows refused for an unproven leader captor.
    pub refused_leaders: u32,
}

/// A terminal job failure invalidates scheduled reads, never carried samples.
pub fn failed_capture(
    table: TableId,
    hour: i32,
    force_full: bool,
    reason: &'static str,
) -> Outcome {
    Outcome::Unavailable {
        reason: if replay::cadence::should_walk(table, hour, force_full) {
            reason
        } else {
            replay::cadence::SKIPPED_REASON
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
