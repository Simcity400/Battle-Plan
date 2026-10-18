//! Air bases whose country-container vector needs no full scan. Every
//! `cair_base::COUNTRY_CONTAINERS` vector has a slot per country (air base construction sizes it to
//! the game state's country count), almost all null; its only run-time writers, create country
//! container and remove country access, keep the dense list (`cair_base::REGISTERED_CONTAINERS`) in
//! step in the same call. AirBaseRegistrations therefore reads, per base, the dense list and each
//! listed container's identity, span, link entry and slot word ([`engine_native::cair_base::dense_plan`]),
//! and no other slot: the listed containers are the occupied slots, in slot order. A list that does
//! not prove the slots leaves the base to the full scan. Containers, pools, wings, capacity and
//! fuel are still read every hour. No hook is involved.
//!
//! Each hour the acquisition also scans every chunk of its sampled slice of
//! fast-path bases (by base handle, one in
//! [`crate::sampled_verify::ROTATION`]) outside the recorded log and compares
//! the occupied slots with the list's; a difference is a writer the list
//! missed: it is counted, the base is recorded from the full scan that hour,
//! and the fast path stops for the attachment. `air_base_verify`, on by
//! default during its burn-in, compares every fast-path base instead.
use crate::sampled_verify::{Check, Sampled};
use engine_native::cair_base::DenseRefusal;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Every fast-path base is compared with its full scan (`air_base_verify`).
pub(crate) const VERIFY: bool = cfg!(feature = "air_base_verify");

static POISONED: AtomicBool = AtomicBool::new(false);
/// Fast-path, full-scan and compared bases.
static BASES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
/// Bases left to the full scan, by [`DenseRefusal`].
static REFUSALS: [AtomicU64; DenseRefusal::COUNT] =
    [const { AtomicU64::new(0) }; DenseRefusal::COUNT];
/// Compared bases whose chunks could not all be read outside the log.
static UNREAD: AtomicU64 = AtomicU64::new(0);
static SAMPLED: Sampled =
    Sampled::burned_in(&["a compared base's occupied slots differ from its dense container list"]);

/// How one hour's registration acquisition finds each base's occupied slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlotRun {
    /// A base whose dense list proves its slots reads only their slot words.
    pub(crate) dense: bool,
    /// Which fast-path bases are also scanned whole and compared.
    pub(crate) check: Check,
}

impl SlotRun {
    /// The hour being captured, as the production job reads it (tests set
    /// their own).
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn current() -> Self {
        Self {
            dense: cfg!(feature = "air_base_changes") && !POISONED.load(Ordering::Relaxed),
            check: crate::sampled_verify::current_burned_in(VERIFY),
        }
    }
}

/// One hour's registration acquisition of its bases' slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlotReport {
    /// Bases that read only their listed containers' slot words.
    pub(crate) dense: u64,
    /// Bases that read every chunk.
    pub(crate) full: u64,
    /// Fast-path bases compared with their full scan.
    pub(crate) verified: u64,
    /// Compared bases whose occupied slots differed from the list.
    pub(crate) mismatches: u64,
    /// Compared bases whose whole scan did not read.
    pub(crate) unread: u64,
    pub(crate) refusals: [u64; DenseRefusal::COUNT],
}

/// Off the game thread, after the hour's acquisition: any difference stops
/// the fast path for the attachment.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn record(report: &SlotReport) {
    for (total, value) in BASES
        .iter()
        .zip([report.dense, report.full, report.verified])
    {
        total.fetch_add(value, Ordering::Relaxed);
    }
    for (total, value) in REFUSALS.iter().zip(report.refusals) {
        total.fetch_add(value, Ordering::Relaxed);
    }
    UNREAD.fetch_add(report.unread, Ordering::Relaxed);
    if SAMPLED.record(report.verified, report.mismatches, 0) {
        POISONED.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn status() -> impl serde::Serialize {
    let bases = BASES.each_ref().map(|value| value.load(Ordering::Relaxed));
    let refusals = crate::status::map(
        DenseRefusal::ALL
            .iter()
            .zip(&REFUSALS)
            .map(|(refusal, count)| (refusal.reason(), count.load(Ordering::Relaxed))),
    );
    crate::status::status!({
        "configured": cfg!(feature = "air_base_changes"),
        "verify": VERIFY,
        "poisoned": POISONED.load(Ordering::Relaxed),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "dense_bases": bases[0],
        "full_scan_bases": bases[1],
        "verified_bases": bases[2],
        "unread_comparisons": UNREAD.load(Ordering::Relaxed),
        "full_scan_reasons": refusals,
        "scope": "cumulative since attach; AirBaseRegistrations reads a base's country-container vector only at the slot words of the containers its dense list names, since country-container creation and country access removal write both in the same call; a list that does not prove the slots (full_scan_reasons) reads every chunk; containers, pools, wings, capacity and fuel are read every hour; every hour also scans every chunk of the fast-path bases of its slice (one in rotation_hours by base handle; every one in verify mode) outside the recorded log and compares (sampled_*); a difference records that base from the full scan and stops the fast path"
    })
}
