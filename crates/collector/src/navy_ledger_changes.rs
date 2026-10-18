//! Navies whose mission ledgers need no read. A strategic navy files each
//! naval mission in the ledger of every region it serves and, once, in its
//! registered-mission set ([`engine_native::cstrategic_navy::REGISTERED_MISSIONS`]).
//! Registration, removal and reset write the set and ledger in the same
//! call; construction builds both empty. A navy whose set is empty has
//! no ledger entries. NavalMissionLedgers reads such a navy's set header
//! instead of its region headers ([`crate::extended::naval_mission_inputs`]).
//! No hook is involved: the hour reads every set.
//!
//! Each hour still reads the region headers of the skippable navies of its
//! sampled slice (by canonical country slot, one in
//! [`crate::sampled_verify::BURNED_IN_ROTATION`]) and publishes that read; a
//! nonempty ledger there is a missed writer, which is recorded and stops
//! every later skip for the attachment. `navy_ledger_verify` reads every
//! skippable navy instead.
use crate::sampled_verify::{Check, Sampled};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Every skippable navy is read and compared (`navy_ledger_verify`).
pub(crate) const VERIFY: bool = cfg!(feature = "navy_ledger_verify");

static POISONED: AtomicBool = AtomicBool::new(false);
/// Skipped, read and verified navies.
static NAVIES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static SAMPLED: Sampled =
    Sampled::burned_in(&["a navy without registered missions holds a nonempty region ledger"]);

/// How one hour's ledger read treats navies without registered missions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Skip {
    /// Such a navy's region headers are not read (the family is configured
    /// and has seen no mismatch).
    pub(crate) enabled: bool,
    /// Which of them are read anyway and compared.
    pub(crate) check: Check,
}

impl Skip {
    /// Every navy's region headers are read.
    #[cfg(test)]
    pub(crate) const NEVER: Self = Self {
        enabled: false,
        check: Check::Nothing,
    };

    /// The hour being captured, as the production job reads it.
    pub(crate) fn current() -> Self {
        Self {
            enabled: cfg!(feature = "navy_ledger_changes") && !POISONED.load(Ordering::Relaxed),
            check: crate::sampled_verify::current_burned_in(VERIFY),
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
    /// Read navies without registered missions, compared with an empty ledger.
    pub(crate) verified: u64,
    /// Compared navies holding a nonempty region ledger.
    pub(crate) mismatches: u64,
}

/// Off the game thread, after the hour's ledger read: a mismatch stops every
/// later skip.
pub(crate) fn record(counts: Counts) {
    for (total, value) in NAVIES
        .iter()
        .zip([counts.skipped, counts.read, counts.verified])
    {
        total.fetch_add(value, Ordering::Relaxed);
    }
    if counts.verified != 0 && SAMPLED.record(counts.verified, counts.mismatches, 0) {
        POISONED.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn status() -> impl serde::Serialize {
    let navies = NAVIES.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "navy_ledger_changes"),
        "verify": VERIFY,
        "poisoned": POISONED.load(Ordering::Relaxed),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "skipped_navies": navies[0],
        "read_navies": navies[1],
        "verified_navies": navies[2],
        "scope": "cumulative since attach; NavalMissionLedgers reads a navy's region ledger headers only when its registered-mission set is nonempty, since every ledger writer writes the set in the same call; every hour also reads the skippable navies of its slice (one in rotation_hours by country slot; every one in verify mode), publishes that read and stops skipping at the first nonempty ledger (sampled_*)"
    })
}
