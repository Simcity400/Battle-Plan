//! R2 bound-trip attribution registry (`R2_VALIDATION_BOUNDS_AUDIT.md` follow-up).
//!
//! Every hardcoded plausibility bound tuned on *vanilla* observation is a place
//! a modded save can legitimately exceed the limit. The dominant pre-R2 pattern
//! was CLAMP-DROP: `.filter(|v| *v <= MAX)` silently turns the field into
//! `None` and the frame stays CLEAN — invisible per-field data loss on any
//! save that trips it. This registry converts those sites to the d69/d47
//! attribution law: the acceptance behavior is UNCHANGED (the value is still
//! dropped — the bound exists because a larger value cannot be validated), but
//! every trip is recorded here and drained once per frame by the collector into
//! `bt_*` diagnostic units on the reserved probe domain, so a modded trip
//! names itself (bound + observed raw value) in a single capture instead of
//! relying on a vanilla-baseline row-count diff to notice the loss.
//!
//! Concurrency: recorded from collector worker threads via relaxed atomics
//! (counters + a last-value cell; exact cross-field consistency is not
//! required — any recorded raw value is a genuine observed trip).

use core::sync::atomic::{AtomicU64, Ordering};

/// Bound identities. Indexes into the registries and [`BOUND_TRIP_UNITS`].
pub const BOUND_TRIP_UNIT_LEADER_SKILL: usize = 0;
pub const BOUND_TRIP_FOCUS_COUNT: usize = 1;
pub const BOUND_TRIP_COMPLETED_TECH_COUNT: usize = 2;
pub const BOUND_TRIP_OWNER_TAG_DB_ID: usize = 3;
pub const BOUND_TRIP_PLAYER_TAG_DB_ID: usize = 4;
/// A country-database id fell outside the LIVE database's tag range
/// (`0 <= id < *(int32 *)(country_database + 0x0C)`; see
/// [`crate::country_tag_db`]). Before the bound was derived from the engine
/// this was a hardcoded `1024`, and the unit name keeps that suffix so the
/// diagnostic key stays comparable across captures either side of the change.
pub const BOUND_TRIP_TAG_DB_ID_1024: usize = 5;
pub const BOUND_TRIP_FACTION_SLOTS: usize = 6;
pub const BOUND_TRIP_FACTION_MEMBERS: usize = 7;
/// A country-database id could not be validated at all: no tag-database size
/// has been observed, so there is nothing to range-check against. Deliberately
/// separate from [`BOUND_TRIP_TAG_DB_ID_1024`] - "the id was out of range" and
/// "we never learned the range" are different defects with different fixes,
/// and merging them is how a trip counter comes to mean nothing in particular.
pub const BOUND_TRIP_TAG_DB_ID_UNRESOLVED: usize = 8;
pub const BOUND_TRIP_ID_COUNT: usize = 9;

/// `(per-frame trip counter unit, last-observed raw value unit)` per bound id.
/// The counter unit is summable in the recorder; the `_sample` suffix routes
/// the raw value through the recorder's last-value unit class (never summed —
/// summing destroys raw words). Names must satisfy the recorder's diagnostic
/// name charset (ascii lowercase / digits / underscore).
pub const BOUND_TRIP_UNITS: [(&str, &str); BOUND_TRIP_ID_COUNT] = [
    (
        "bt_unit_leader_skill_trips",
        "bt_unit_leader_skill_raw_sample",
    ),
    ("bt_focus_count_trips", "bt_focus_count_raw_sample"),
    (
        "bt_completed_tech_count_trips",
        "bt_completed_tech_count_raw_sample",
    ),
    ("bt_owner_tag_db_id_trips", "bt_owner_tag_db_id_raw_sample"),
    (
        "bt_player_tag_db_id_trips",
        "bt_player_tag_db_id_raw_sample",
    ),
    ("bt_tag_db_id_1024_trips", "bt_tag_db_id_1024_raw_sample"),
    ("bt_faction_slots_trips", "bt_faction_slots_raw_sample"),
    ("bt_faction_members_trips", "bt_faction_members_raw_sample"),
    (
        "bt_tag_db_id_unresolved_trips",
        "bt_tag_db_id_unresolved_raw_sample",
    ),
];

#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_ZERO: AtomicU64 = AtomicU64::new(0);
static TRIP_COUNTS: [AtomicU64; BOUND_TRIP_ID_COUNT] = [ATOMIC_ZERO; BOUND_TRIP_ID_COUNT];
static TRIP_LAST_RAW: [AtomicU64; BOUND_TRIP_ID_COUNT] = [ATOMIC_ZERO; BOUND_TRIP_ID_COUNT];
static ACTIVE_FRAME_RESERVATION: AtomicU64 = AtomicU64::new(0);
static NEXT_FRAME_RESERVATION: AtomicU64 = AtomicU64::new(1);

/// Immutable frame-scoped view of the bound-trip registry. The reservation
/// keeps the represented counter prefix live until the collector either
/// commits it after authoritative publication or rolls it back.
#[derive(Debug)]
pub struct BoundTripFrameReservation {
    reservation_id: u64,
    snapshot: [(u64, u64); BOUND_TRIP_ID_COUNT],
    active: bool,
}

impl BoundTripFrameReservation {
    pub fn snapshot(&self) -> &[(u64, u64); BOUND_TRIP_ID_COUNT] {
        &self.snapshot
    }

    /// Consume exactly the counter prefix represented by this reservation.
    /// Records that raced the snapshot stay in the registry for the next
    /// frame; last-raw samples remain last-value cells and are never cleared.
    pub fn commit(mut self) -> Result<(), String> {
        ensure_active_reservation(self.reservation_id)?;
        for (id, (reserved, _)) in self.snapshot.iter().copied().enumerate() {
            let current = TRIP_COUNTS[id].load(Ordering::Acquire);
            if current < reserved {
                return Err(format!(
                    "bound-trip reservation {} counter {} regressed from {} to {}",
                    self.reservation_id, id, reserved, current
                ));
            }
        }
        for (id, (reserved, _)) in self.snapshot.iter().copied().enumerate() {
            if reserved != 0 {
                TRIP_COUNTS[id].fetch_sub(reserved, Ordering::AcqRel);
            }
        }
        release_active_reservation(self.reservation_id)?;
        self.active = false;
        Ok(())
    }

    pub fn rollback(mut self) -> Result<(), String> {
        release_active_reservation(self.reservation_id)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for BoundTripFrameReservation {
    fn drop(&mut self) {
        if self.active {
            let _ = release_active_reservation(self.reservation_id);
            self.active = false;
        }
    }
}

fn ensure_active_reservation(reservation_id: u64) -> Result<(), String> {
    let active = ACTIVE_FRAME_RESERVATION.load(Ordering::Acquire);
    if active != reservation_id {
        return Err(format!(
            "bound-trip reservation mismatch: active {}, requested {}",
            active, reservation_id
        ));
    }
    Ok(())
}

fn release_active_reservation(reservation_id: u64) -> Result<(), String> {
    ACTIVE_FRAME_RESERVATION
        .compare_exchange(reservation_id, 0, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|active| {
            format!(
                "bound-trip reservation release mismatch: active {}, requested {}",
                active, reservation_id
            )
        })
}

/// Reserve the current counter prefix without changing registry state.
pub fn reserve_frame_trips() -> Result<BoundTripFrameReservation, String> {
    let reservation_id = NEXT_FRAME_RESERVATION.fetch_add(1, Ordering::Relaxed);
    if reservation_id == 0 {
        return Err("bound-trip reservation id wrapped".into());
    }
    ACTIVE_FRAME_RESERVATION
        .compare_exchange(0, reservation_id, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|active| format!("bound-trip frame reservation already active: {}", active))?;
    let mut snapshot = [(0u64, 0u64); BOUND_TRIP_ID_COUNT];
    for (id, slot) in snapshot.iter_mut().enumerate() {
        let count = TRIP_COUNTS[id].load(Ordering::Acquire);
        let raw = TRIP_LAST_RAW[id].load(Ordering::Relaxed);
        *slot = (count, raw);
    }
    Ok(BoundTripFrameReservation {
        reservation_id,
        snapshot,
        active: true,
    })
}

/// Record one bound trip: the observed `raw` value exceeded the bound `id`
/// guards. Out-of-range `id` is ignored (defensive; all callers use the
/// constants above).
///
/// Ordering (R2 gate finding 1): the raw value is stored BEFORE the counter is
/// published (Release), and the drain swaps the counter with Acquire before
/// loading raw — so whenever a drain observes a nonzero count, at least one
/// counted trip's raw store happens-before its raw load. The old
/// count-then-raw order let a drain race in between and emit (trips=1, raw=0)
/// with the genuine value lost forever (its count already consumed).
pub fn record_bound_trip(id: usize, raw: u64) {
    if id >= BOUND_TRIP_ID_COUNT {
        return;
    }
    TRIP_LAST_RAW[id].store(raw, Ordering::Relaxed);
    TRIP_COUNTS[id].fetch_add(1, Ordering::Release);
}

/// Upper-bound gate preserving CLAMP-DROP acceptance: `Some(value)` iff
/// `value <= max`, otherwise records the trip and returns `None`. Zero /
/// lower-bound checks stay at the call site (a zero id is a vacant slot, not
/// a bound trip).
#[inline]
pub fn gate_u32(value: u32, max: u32, id: usize) -> Option<u32> {
    if value <= max {
        Some(value)
    } else {
        record_bound_trip(id, u64::from(value));
        None
    }
}

/// Predicate form for `if value > MAX { reject }` sites: `true` (and records
/// the trip) iff `value > max`. Use as the drop-in for the `> MAX` arm so the
/// site's other reject arms (torn-read shapes like `cap < size`) stay
/// unattributed — those are read failures, not content bounds.
#[inline]
pub fn bound_exceeded_u32(value: u32, max: u32, id: usize) -> bool {
    if value > max {
        record_bound_trip(id, u64::from(value));
        true
    } else {
        false
    }
}

/// Engine-exact country-database id gate, in the `bound_exceeded_*` polarity:
/// `true` (and records the trip) iff `id` is not a valid country-database id
/// for the live game.
///
/// The range is the database's own - `0 <= id < *(int32 *)(country_database +
/// 0x0C)`, see [`crate::country_tag_db`] - never a constant. Callers holding a
/// `CountryTagTable` should use its `len` through [`tag_db_id_out_of_table`]
/// instead: that cannot go stale.
///
/// This is a RANGE gate only, exactly like the `bound_exceeded_u32(id, 1024,
/// ..)` calls it replaces: `id == 0` is the no-country sentinel, which is in
/// range and passes without a trip. Sites that must not emit the null country
/// keep their own `id == 0` arm, so a vacant slot stays a vacancy rather than
/// becoming a bound violation.
///
/// With no observed database size the id is not validatable, so it is rejected
/// and attributed to [`BOUND_TRIP_TAG_DB_ID_UNRESOLVED`]. Accepting instead
/// would let a heap-scan candidate word through, which is the one thing this
/// gate exists to prevent.
#[inline]
pub fn tag_db_id_out_of_range(id: u32) -> bool {
    match crate::country_tag_db::observed_tag_db_size() {
        Some(size) => {
            if id < size {
                false
            } else {
                record_bound_trip(BOUND_TRIP_TAG_DB_ID_1024, u64::from(id));
                true
            }
        }
        None => {
            record_bound_trip(BOUND_TRIP_TAG_DB_ID_UNRESOLVED, u64::from(id));
            true
        }
    }
}

/// [`tag_db_id_out_of_range`] in the `gate_u32` polarity.
#[inline]
pub fn gate_tag_db_id(id: u32) -> Option<u32> {
    (!tag_db_id_out_of_range(id)).then_some(id)
}

/// Engine-exact country-database id gate against a size the caller already
/// read from the table the id indexes (`0 <= id < table_size`). Records a trip
/// on an out-of-table id so a silently skipped row still names itself. Like
/// [`tag_db_id_out_of_range`] this is a range gate: the null country is in
/// range and each caller keeps its own `id == 0` policy.
#[inline]
pub fn tag_db_id_out_of_table(id: u32, table_size: u32) -> bool {
    if crate::country_tag_db::tag_db_id_in_table(id, table_size) {
        false
    } else {
        record_bound_trip(BOUND_TRIP_TAG_DB_ID_1024, u64::from(id));
        true
    }
}

/// [`tag_db_id_out_of_table`] in the `gate_u32` polarity.
#[inline]
pub fn gate_tag_db_id_in_table(id: u32, table_size: u32) -> Option<u32> {
    (!tag_db_id_out_of_table(id, table_size)).then_some(id)
}

/// Drain per-frame trip counts (resetting them) and read each bound's
/// last-observed raw value. Index-aligned with [`BOUND_TRIP_UNITS`]. The
/// last-raw cell is intentionally NOT reset: it is a `_sample` (last observed
/// value), and a trip landing between the swap and the load is still a
/// genuine observed trip.
pub fn drain_frame_trips() -> [(u64, u64); BOUND_TRIP_ID_COUNT] {
    let reservation = reserve_frame_trips().expect("bound-trip compatibility drain reservation");
    let out = *reservation.snapshot();
    reservation
        .commit()
        .expect("bound-trip compatibility drain commit");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    // R2 gate finding 4: distinct ids are NOT enough — drain_frame_trips()
    // resets EVERY id, so a parallel test's drain steals another test's
    // counts. Every test that records or drains takes this lock.
    static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

    fn lock_registry() -> MutexGuard<'static, ()> {
        REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The tag-db gate reads a process-global published size, so a test that
    /// publishes must also hold the country-database publish lock.
    fn lock_tag_db() -> MutexGuard<'static, ()> {
        crate::country_tag_db::test_support::lock_publish()
    }

    #[test]
    fn tag_db_gate_follows_the_live_database_not_a_constant() {
        let _guard = lock_registry();
        let _db_guard = lock_tag_db();
        drain_frame_trips(); // clean slate
        let id = BOUND_TRIP_TAG_DB_ID_1024;

        // A 440-row vanilla database (the size the t11 capture measures: 439
        // country tags plus slot 0). The old 1024 admitted 584 ids the
        // engine's own accessor refuses.
        assert!(crate::country_tag_db::publish_observed_tag_db_size(440, 1_024));
        assert_eq!(gate_tag_db_id(1), Some(1));
        assert_eq!(gate_tag_db_id(439), Some(439));
        assert_eq!(gate_tag_db_id(440), None, "size is exclusive");
        assert_eq!(gate_tag_db_id(1_000), None, "the old 1024 admitted this");
        assert_eq!(
            gate_tag_db_id(0),
            Some(0),
            "the null country is in range; refusing it is caller policy"
        );
        let drained = drain_frame_trips();
        assert_eq!(
            drained[id].0, 2,
            "440 and 1000 trip; the zero id is a vacant slot, never a bound trip"
        );
        assert_eq!(drained[id].1, 1_000);
        assert_eq!(
            drained[BOUND_TRIP_TAG_DB_ID_UNRESOLVED].0, 0,
            "an observed database is never reported unresolved"
        );

        // A total-conversion database with 12_000 tags: ids the old ceiling
        // silently dropped are ordinary rows here.
        assert!(crate::country_tag_db::publish_observed_tag_db_size(12_000, 16_384));
        assert_eq!(gate_tag_db_id(1_025), Some(1_025));
        assert_eq!(gate_tag_db_id(11_999), Some(11_999));
        assert_eq!(gate_tag_db_id(12_000), None);
        let drained = drain_frame_trips();
        assert_eq!(drained[id].0, 1);
        assert_eq!(drained[id].1, 12_000);

        crate::country_tag_db::forget_observed_tag_db_size();
    }

    #[test]
    fn tag_db_gate_rejects_and_attributes_when_no_database_was_observed() {
        let _guard = lock_registry();
        let _db_guard = lock_tag_db();
        drain_frame_trips(); // clean slate
        crate::country_tag_db::forget_observed_tag_db_size();

        // Unvalidatable is NOT "valid": a heap-scan candidate word must not
        // reach a row just because the database was unreadable. 1_131_650_984
        // is a real raw sample from the t11 capture's discovery scan.
        assert_eq!(gate_tag_db_id(1), None);
        assert_eq!(gate_tag_db_id(1_131_650_984), None);
        assert!(
            tag_db_id_out_of_range(0),
            "not even the null country validates without a database"
        );
        assert!(tag_db_id_out_of_range(7));

        let drained = drain_frame_trips();
        assert_eq!(drained[BOUND_TRIP_TAG_DB_ID_UNRESOLVED].0, 4);
        assert_eq!(drained[BOUND_TRIP_TAG_DB_ID_UNRESOLVED].1, 7);
        assert_eq!(
            drained[BOUND_TRIP_TAG_DB_ID_1024].0, 0,
            "an unobserved database is not an out-of-range id"
        );
    }

    #[test]
    fn tag_db_table_gate_uses_the_callers_own_count() {
        let _guard = lock_registry();
        let _db_guard = lock_tag_db();
        drain_frame_trips(); // clean slate
        // Deliberately publish a DIFFERENT size: the table form must not
        // consult the global at all.
        assert!(crate::country_tag_db::publish_observed_tag_db_size(9, 16));

        assert_eq!(gate_tag_db_id_in_table(1, 440), Some(1));
        assert_eq!(gate_tag_db_id_in_table(439, 440), Some(439));
        assert_eq!(gate_tag_db_id_in_table(440, 440), None);
        assert_eq!(gate_tag_db_id_in_table(0, 440), Some(0));
        let drained = drain_frame_trips();
        assert_eq!(drained[BOUND_TRIP_TAG_DB_ID_1024].0, 1);
        assert_eq!(drained[BOUND_TRIP_TAG_DB_ID_1024].1, 440);
        assert_eq!(
            drained[BOUND_TRIP_TAG_DB_ID_UNRESOLVED].0, 0,
            "the table form never reports an unresolved database"
        );

        crate::country_tag_db::forget_observed_tag_db_size();
    }

    #[test]
    fn gate_passes_in_range_and_records_over_max() {
        let _guard = lock_registry();
        drain_frame_trips(); // clean slate
        let id = BOUND_TRIP_UNIT_LEADER_SKILL;
        assert_eq!(gate_u32(15, 15, id), Some(15));
        assert_eq!(gate_u32(0, 15, id), Some(0));
        assert_eq!(gate_u32(16, 15, id), None);
        assert_eq!(gate_u32(99, 15, id), None);
        let drained = drain_frame_trips()[id];
        assert_eq!(drained.0, 2);
        assert_eq!(drained.1, 99);
        // Drain resets the counter but keeps the last-raw sample.
        let drained = drain_frame_trips()[id];
        assert_eq!(drained.0, 0);
        assert_eq!(drained.1, 99);
    }

    #[test]
    fn bound_exceeded_records_only_over_max() {
        let _guard = lock_registry();
        drain_frame_trips(); // clean slate
        let id = BOUND_TRIP_FACTION_SLOTS;
        assert!(!bound_exceeded_u32(512, 512, id));
        assert!(bound_exceeded_u32(513, 512, id));
        let drained = drain_frame_trips()[id];
        assert_eq!(drained, (1, 513));
    }

    #[test]
    fn frame_reservation_rolls_back_or_commits_only_its_prefix() {
        let _guard = lock_registry();
        drain_frame_trips();
        let id = BOUND_TRIP_FACTION_MEMBERS;
        record_bound_trip(id, 600);
        let rolled_back = reserve_frame_trips().unwrap();
        assert_eq!(rolled_back.snapshot()[id], (1, 600));
        record_bound_trip(id, 601);
        rolled_back.rollback().unwrap();
        assert_eq!(drain_frame_trips()[id], (2, 601));

        record_bound_trip(id, 700);
        let committed = reserve_frame_trips().unwrap();
        record_bound_trip(id, 701);
        committed.commit().unwrap();
        assert_eq!(
            drain_frame_trips()[id],
            (1, 701),
            "post-reservation records must remain queued"
        );
    }

    #[test]
    fn out_of_range_id_is_ignored() {
        record_bound_trip(BOUND_TRIP_ID_COUNT, 42);
        record_bound_trip(usize::MAX, 42);
        // No panic; nothing to assert beyond not corrupting valid ids.
    }

    #[test]
    fn concurrent_drain_never_sees_count_without_raw() {
        // R2 gate finding 1: with the old count-then-raw publish order, a
        // drain racing between them consumed the count while raw was still
        // the 0 sentinel — the genuine value was lost forever. All recorded
        // raws here are nonzero, so a drained (count>0, raw==0) pair is
        // exactly that lost-attribution defect.
        let _guard = lock_registry();
        drain_frame_trips(); // clean slate
        let id = BOUND_TRIP_PLAYER_TAG_DB_ID;
        let recorder = std::thread::spawn(move || {
            for i in 0..20_000u64 {
                record_bound_trip(id, 2_049 + (i % 7));
            }
        });
        let mut drained_total = 0u64;
        loop {
            let (count, raw) = drain_frame_trips()[id];
            if count != 0 {
                assert_ne!(
                    raw, 0,
                    "drained a counted trip whose raw attribution was lost"
                );
                drained_total += count;
            }
            if recorder.is_finished() {
                break;
            }
        }
        recorder.join().unwrap();
        let (count, raw) = drain_frame_trips()[id];
        if count != 0 {
            assert_ne!(raw, 0);
            drained_total += count;
        }
        assert_eq!(drained_total, 20_000, "no trip count may be lost");
    }

    // Name-table test needs no lock: it never touches the registry.
    #[test]
    fn unit_names_satisfy_diagnostic_charset_and_sample_suffix() {
        for (trips, sample) in BOUND_TRIP_UNITS {
            for name in [trips, sample] {
                assert!(!name.is_empty());
                assert!(name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
                assert!(name.starts_with("bt_"));
            }
            assert!(trips.ends_with("_trips"));
            assert!(!trips.ends_with("_sample"));
            assert!(sample.ends_with("_sample"));
        }
    }
}
