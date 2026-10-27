//! Navies whose mission ledgers need no read. A strategic navy files each
//! naval mission in the ledger of every region it serves and, once, in its
//! registered-mission set ([`engine_native::cstrategic_navy::REGISTERED_MISSIONS`]).
//! Registration, removal and reset write the set and ledger in the same
//! call; construction builds both empty. A navy whose set is empty has
//! no ledger entries. NavalMissionLedgers reads such a navy's set header
//! instead of its region headers ([`crate::extended::naval_mission_inputs`]).
//! No hook is involved: the hour reads every set.
use std::sync::atomic::{AtomicU64, Ordering};

/// Skipped and read navies.
static NAVIES: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

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

/// One hour's ledger read of the navies it could skip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Navies without registered missions whose region headers were not read.
    pub(crate) skipped: u64,
    /// Navies whose region headers were read.
    pub(crate) read: u64,
}

/// Off the game thread, after the hour's ledger read.
pub(crate) fn record(counts: Counts) {
    for (total, value) in NAVIES.iter().zip([counts.skipped, counts.read]) {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

pub(crate) fn status() -> impl serde::Serialize {
    let navies = NAVIES.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "navy_ledger_changes"),
        "skipped_navies": navies[0],
        "read_navies": navies[1],
        "scope": "cumulative since attach; NavalMissionLedgers reads a navy's region ledger headers only when its registered-mission set is nonempty, since every ledger writer writes the set in the same call"
    })
}
