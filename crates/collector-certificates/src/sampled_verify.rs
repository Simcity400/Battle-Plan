//! Continuous sampled verification of change-certified reuse.
//!
//! A family that carries entries no hooked writer touched (relation vectors,
//! opinions, the province name, region access and world tension tables, air
//! histories, character rows, portraits, advisors, script
//! flags, building statuses, available-advisor pools, the advisor-token
//! table, quiet HiredAdvisors and character record hours, party leaders, script variable stores, political parties and
//! active ideas, air wing equipment, registered air missions, rail infos,
//! States and Provinces rows, the skipped navy mission ledgers, controller
//! and owner areas, division stat-block attack operands, the carried country
//! and commander naval hover trees, regional air-combat holders, the
//! commanders' retreat bans, the quiet roster, scientist and politics hours,
//! the stability, research, appearance, counter and visibility inputs of
//! countries no hooked writer marked, countries' local sub-unit clones, division template
//! name keys, ships' own ShipModels inputs, the quiet war relations, the
//! provinces' ordered occupant lists and the raid intel countries' modifier
//! answers) also
//! reads a rotating slice of its carried entries afresh every hour, compares
//! each with the held values and publishes the fresh read.
//! An entry is in an hour's slice when its stable key falls in that hour of
//! its family's rotation ([`Check::covers`]). A family still in its live
//! burn-in rotates over the [`ROTATION`], so each carried entry is compared
//! once per game day ([`Check::hourly`]); a family whose burn-in passed
//! rotates over [`BURNED_IN_DAYS`] game days, so each is compared once in
//! 576 hours ([`Check::burned_in`]; owner decision 2026-10-11). Relation
//! opinions and available-advisor pools are read once a day at their
//! owner's slice hour, and local sub-unit clones change only on research and
//! doctrine unlocks, so their rotation runs over days
//! ([`Check::covers_daily`]) in either stage. A family that joins the
//! burned-in list (the test below) takes [`Check::burned_in`] or
//! [`current_burned_in`] at its call sites and [`Sampled::burned_in`] for its
//! status.
//! The raid state inputs and the naval mission countries (sliced families
//! that read only the owners a hooked writer marked, or whose certificate
//! moved, beside their daily slice) compare every owner their slice reads
//! with its previous read
//! ([`crate::slicing::SliceSelection::compare`]).
//! Any difference is a missed write: the family records it ([`Sampled`]),
//! poisons its observation for the rest of the attachment, and from then on
//! every entry is read afresh.
//!
//! The `*_verify` diagnostic features compare every carried entry instead
//! ([`Check::Every`]); a family still in its live burn-in keeps its feature
//! on by default.
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Hours in one rotation: one game day.
pub const ROTATION: u64 = replay::cadence::STAGGER_PERIOD as u64;
/// Days in a burned-in family's rotation.
pub const BURNED_IN_DAYS: u64 = ROTATION;
/// Hours in a burned-in family's rotation.
pub const BURNED_IN_ROTATION: u64 = BURNED_IN_DAYS * ROTATION;

/// Which carried entries a capture reads afresh and compares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Check {
    /// None: before the first hour, and tests of reuse alone.
    #[default]
    Nothing,
    /// The entries whose stable key falls in this hour of the day: a family
    /// in its live burn-in.
    Slice { hour: i32 },
    /// The entries whose stable key falls in this hour of the
    /// [`BURNED_IN_ROTATION`]: a family whose burn-in passed.
    BurnedIn { hour: i32 },
    /// Every carried entry (a `*_verify` feature).
    Every,
}

impl Check {
    /// Production, a family in its live burn-in: every carried entry under
    /// the family's `*_verify` feature, otherwise the hour's slice of the day.
    pub const fn hourly(hour: i32, every: bool) -> Self {
        if every {
            Self::Every
        } else {
            Self::Slice { hour }
        }
    }

    /// Production, a family whose burn-in passed: every carried entry under
    /// the family's `*_verify` feature, otherwise the hour's slice of the
    /// [`BURNED_IN_ROTATION`].
    pub const fn burned_in(hour: i32, every: bool) -> Self {
        if every {
            Self::Every
        } else {
            Self::BurnedIn { hour }
        }
    }

    /// Whether the carried entry with stable `key` is compared this hour.
    pub fn covers(self, key: u64) -> bool {
        match self {
            Self::Nothing => false,
            Self::Every => true,
            Self::Slice { hour } => key % ROTATION == phase_of(hour, ROTATION),
            Self::BurnedIn { hour } => {
                key % BURNED_IN_ROTATION == phase_of(hour, BURNED_IN_ROTATION)
            }
        }
    }

    /// [`Self::covers`] for an entry read once a day at its own slice hour:
    /// compared on the day of the [`BURNED_IN_DAYS`]-day rotation its key
    /// names, whichever stage the family is in.
    pub fn covers_daily(self, key: u64) -> bool {
        match self {
            Self::Slice { hour } | Self::BurnedIn { hour } => {
                (key / ROTATION) % BURNED_IN_DAYS
                    == phase_of(hour.div_euclid(ROTATION as i32), BURNED_IN_DAYS)
            }
            other => other.covers(key),
        }
    }

    pub fn any(self) -> bool {
        self != Self::Nothing
    }

    /// Whether this hour's slice may hold a key in `phase` of the day
    /// (`key % ROTATION`), for a family that buckets its entries by it; the
    /// bucket's entries still need [`Self::covers`].
    pub fn covers_phase(self, phase: u64) -> bool {
        match self {
            Self::Nothing => false,
            Self::Every => true,
            Self::Slice { hour } | Self::BurnedIn { hour } => phase == phase_of(hour, ROTATION),
        }
    }

    /// A slice of either rotation: neither nothing nor every entry.
    pub fn sampled(self) -> bool {
        matches!(self, Self::Slice { .. } | Self::BurnedIn { .. })
    }
}

fn phase_of(at: i32, rotation: u64) -> u64 {
    at.rem_euclid(rotation as i32) as u64
}

/// The hour being captured: set on the game thread before any job of the
/// hour is dispatched, read by the jobs, which all finish within the hour's
/// callback. Zero before the first hour.
#[cfg(not(any(test, feature = "test-support")))]
static HOUR: AtomicU64 = AtomicU64::new(0);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Tests: the hour of this thread's captures, so parallel tests never
    /// share one.
    static HOUR: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

/// Game thread, at the start of an hour's captures. Allocation-free.
pub fn begin_hour(hour: i32) {
    #[cfg(not(any(test, feature = "test-support")))]
    HOUR.store((1 << 32) | u64::from(hour as u32), Ordering::Release);
    #[cfg(any(test, feature = "test-support"))]
    HOUR.set(Some(hour));
}

/// The check of the hour being captured for a family in its live burn-in
/// ([`Check::hourly`]); `every` under the family's `*_verify` feature.
pub fn current(every: bool) -> Check {
    at_hour(every, Check::hourly)
}

/// [`current`] for a family whose burn-in passed ([`Check::burned_in`]).
pub fn current_burned_in(every: bool) -> Check {
    at_hour(every, Check::burned_in)
}

fn at_hour(every: bool, check: fn(i32, bool) -> Check) -> Check {
    #[cfg(not(any(test, feature = "test-support")))]
    let hour = {
        let word = HOUR.load(Ordering::Acquire);
        (word >> 32 != 0).then_some(word as u32 as i32)
    };
    #[cfg(any(test, feature = "test-support"))]
    let hour = HOUR.get();
    match hour {
        Some(hour) => check(hour, every),
        None if every => Check::Every,
        None => Check::Nothing,
    }
}

/// One family's sampled comparisons, cumulative since attach. Lock-free, so
/// any capture thread records.
pub struct Sampled {
    /// The family's named mismatch reasons.
    reasons: &'static [&'static str],
    /// Hours in the family's rotation, for its status.
    rotation_hours: u64,
    verified: AtomicU64,
    mismatches: AtomicU64,
    checks: AtomicU64,
    last_entries: AtomicU64,
    /// One more than the latest mismatch's reason index; zero for none.
    last: AtomicUsize,
}

impl Sampled {
    /// A family in its live burn-in, or one whose slice is counted in daily
    /// reads: its status names the [`ROTATION`].
    pub const fn new(reasons: &'static [&'static str]) -> Self {
        Self::with_rotation(reasons, ROTATION)
    }

    /// A family whose burn-in passed: its status names the
    /// [`BURNED_IN_ROTATION`].
    pub const fn burned_in(reasons: &'static [&'static str]) -> Self {
        Self::with_rotation(reasons, BURNED_IN_ROTATION)
    }

    const fn with_rotation(reasons: &'static [&'static str], rotation_hours: u64) -> Self {
        Self {
            reasons,
            rotation_hours,
            verified: AtomicU64::new(0),
            mismatches: AtomicU64::new(0),
            checks: AtomicU64::new(0),
            last_entries: AtomicU64::new(0),
            last: AtomicUsize::new(0),
        }
    }

    /// One check: `verified` carried entries compared, `mismatches` of them
    /// differing for `reasons[reason]`. Returns whether any differed, which
    /// poisons the family.
    pub fn record(&self, verified: u64, mismatches: u64, reason: usize) -> bool {
        self.checks.fetch_add(1, Ordering::Relaxed);
        self.last_entries.store(verified, Ordering::Relaxed);
        self.verified.fetch_add(verified, Ordering::Relaxed);
        if mismatches == 0 {
            return false;
        }
        self.mismatches.fetch_add(mismatches, Ordering::Relaxed);
        debug_assert!(reason < self.reasons.len());
        self.last
            .store(reason.min(self.reasons.len() - 1) + 1, Ordering::Relaxed);
        true
    }

    pub fn verified(&self) -> u64 {
        self.verified.load(Ordering::Relaxed)
    }

    pub fn mismatches(&self) -> u64 {
        self.mismatches.load(Ordering::Relaxed)
    }

    pub fn last_mismatch(&self) -> Option<&'static str> {
        self.last
            .load(Ordering::Relaxed)
            .checked_sub(1)
            .and_then(|index| self.reasons.get(index).copied())
    }

    /// The slice: the rotation and the entries the latest check compared.
    pub fn slice(&self) -> impl serde::Serialize {
        crate::status::status!({
            "rotation_hours": self.rotation_hours,
            "last_entries": self.last_entries.load(Ordering::Relaxed),
            "checks": self.checks.load(Ordering::Relaxed),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every stable key is compared exactly once in any run of [`ROTATION`]
    /// consecutive hours, including across a day boundary and before hour 0.
    #[test]
    fn a_rotation_of_hours_covers_every_key_exactly_once() {
        for first in [-30, 0, 7, 24 * 365 + 13] {
            for key in 0..5_000_u64 {
                let hits = (first..first + ROTATION as i32)
                    .filter(|&hour| Check::Slice { hour }.covers(key))
                    .count();
                assert_eq!(hits, 1, "key {key} from hour {first}");
            }
        }
        assert!((0..5_000).all(|key| Check::Every.covers(key)));
        assert!((0..5_000).all(|key| !Check::Nothing.covers(key)));
    }

    /// An entry read once a day at its slice hour is compared at exactly one
    /// of any [`ROTATION`] consecutive daily reads, and each hour compares
    /// about one in [`ROTATION`] of that hour's entries.
    #[test]
    fn a_daily_rotation_covers_every_key_once_per_rotation_of_reads() {
        for key in 0..5_000_u64 {
            let slot = (key % ROTATION) as i32;
            let hits = (40..40 + ROTATION as i32)
                .filter(|&day| {
                    Check::Slice {
                        hour: day * 24 + slot,
                    }
                    .covers_daily(key)
                })
                .count();
            assert_eq!(hits, 1, "key {key}");
        }
        let hour = 24 * 3 + 5;
        let read = (0..24 * 24 * 4_u64).filter(|key| key % ROTATION == 5);
        let compared = read
            .clone()
            .filter(|&key| Check::Slice { hour }.covers_daily(key))
            .count();
        assert_eq!(compared * ROTATION as usize, read.count());
    }

    /// A burned-in family compares each key in one hour of 24 days; a
    /// family in its live burn-in in one hour of each day; across rotation
    /// boundaries and before hour 0. A key read once a day keeps the same
    /// day in either stage.
    #[test]
    fn burned_in_families_cover_each_key_once_in_twenty_four_days() {
        assert_eq!(BURNED_IN_ROTATION, 576);
        for first in [-600, 0, 13, 24 * 365 + 7] {
            let hours = first..first + BURNED_IN_ROTATION as i32;
            for key in 0..1_500_u64 {
                let hits = |check: fn(i32, bool) -> Check| {
                    hours
                        .clone()
                        .filter(|&hour| check(hour, false).covers(key))
                        .count()
                };
                assert_eq!(hits(Check::burned_in), 1, "key {key}");
                assert_eq!(hits(Check::hourly), BURNED_IN_DAYS as usize, "key {key}");
            }
        }
        for hour in -50..BURNED_IN_ROTATION as i32 + 50 {
            for key in 0..1_200_u64 {
                assert_eq!(
                    Check::burned_in(hour, false).covers_daily(key),
                    Check::hourly(hour, false).covers_daily(key)
                );
            }
        }
    }

    #[test]
    fn mismatches_name_their_latest_reason() {
        static SAMPLED: Sampled = Sampled::new(&["first", "second"]);
        assert!(!SAMPLED.record(3, 0, 0));
        assert_eq!(SAMPLED.last_mismatch(), None);
        assert!(SAMPLED.record(2, 1, 1));
        assert!(SAMPLED.record(1, 1, 0));
        assert_eq!(
            (
                SAMPLED.verified(),
                SAMPLED.mismatches(),
                SAMPLED.last_mismatch()
            ),
            (6, 2, Some("first"))
        );
    }
}
