//! Navies whose mission ledgers need no read. A strategic navy files each
//! naval mission in the ledger of every region it serves and, once, in its
//! registered-mission set ([`engine_native::cstrategic_navy::REGISTERED_MISSIONS`]).
//! Registration, removal and reset write the set and ledger in the same
//! call; construction builds both empty. Those writers and the navy
//! lifecycle move the navy's ledger certificate
//! ([`crate::naval_map_changes::ledger_token`]): NavalMissionLedgers
//! republishes a navy's held registrations while it holds, and reads only
//! their presence counts ([`crate::extended::naval_mission_inputs`]). A navy
//! whose set is empty has no ledger entries, so a read navy without
//! registered missions reads its set header instead of its region headers.
use std::sync::atomic::{AtomicU64, Ordering};

/// Skipped, read and carried navies.
static NAVIES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
/// The naval mission fingerprint's fleet-leader references, carried and resolved.
static LEADERS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// Off the game thread, after the hour's fingerprint scan.
pub(crate) fn record_leaders(counts: [u64; 2]) {
    for (total, value) in LEADERS.iter().zip(counts) {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

/// How one hour's ledger read treats navies without registered missions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Skip {
    /// Such a navy's region headers are not read (the family is configured).
    pub(crate) enabled: bool,
}

impl Skip {
    /// Every navy's region headers are read.
    #[cfg(test)]
    pub(crate) const NEVER: Self = Self { enabled: false };

    /// The production job's skip.
    pub(crate) fn current() -> Self {
        Self {
            enabled: cfg!(feature = "navy_ledger_changes"),
        }
    }
}

/// One hour's ledger read of the navies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Navies without registered missions whose region headers were not read.
    pub(crate) skipped: u64,
    /// Navies whose region headers were read.
    pub(crate) read: u64,
    /// Navies whose held registrations were republished under their certificate.
    pub(crate) carried: u64,
}

/// Off the game thread, after the hour's ledger read.
pub(crate) fn record(counts: Counts) {
    for (total, value) in NAVIES
        .iter()
        .zip([counts.skipped, counts.read, counts.carried])
    {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

