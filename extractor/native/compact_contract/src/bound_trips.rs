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
/// A `_CountryLinkTags` index fell outside that vector's live size
/// (`CGameState + 0x364`). Deliberately NOT
/// [`BOUND_TRIP_TAG_DB_ID_1024`]: the link vector and the country database
/// are different index spaces (see [`crate::country_tag_db`]), and a counter
/// that merged them would report a database-range failure for an id that was
/// never a database id. Its own id is what keeps the two readable apart in a
/// capture.
pub const BOUND_TRIP_COUNTRY_LINK_INDEX: usize = 8;
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
        "bt_country_link_index_trips",
        "bt_country_link_index_raw_sample",
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
/// `true` (and records the trip) iff `id` is outside the tag vector the caller
/// is about to index.
///
/// `table_size` is the vector's OWN live size, read by whoever authenticated
/// it - `0 <= id < *(int32 *)(country_database + 0x0C)`, see
/// [`crate::country_tag_db`] - never a constant, and never a process-global
/// that could be stale or belong to another campaign.
///
/// This is a RANGE gate only, exactly like the `bound_exceeded_u32(id, 1024,
/// ..)` calls it replaces: `id == 0` is the no-country sentinel, which is in
/// range and passes without a trip. Sites that must not emit the null country
/// keep their own `id == 0` arm, so a vacant slot stays a vacancy rather than
/// becoming a bound violation.
#[inline]
pub fn tag_db_id_out_of_table(id: u32, table_size: u32) -> bool {
    if crate::country_tag_db::tag_db_id_in_table(id, table_size) {
        false
    } else {
        record_bound_trip(BOUND_TRIP_TAG_DB_ID_1024, u64::from(id));
        true
    }
}

/// The same range rule for a `_CountryLinkTags` index against THAT vector's
/// live size, attributed to [`BOUND_TRIP_COUNTRY_LINK_INDEX`].
///
/// A link index is not a country-database id and must not be counted as one:
/// the two vectors have different lengths and grow for different reasons, so a
/// merged counter would report a database-range failure for a value that never
/// indexed the database. See [`crate::country_tag_db`].
#[inline]
pub fn country_link_index_out_of_vector(index: u32, vector_len: u32) -> bool {
    if index < vector_len {
        false
    } else {
        record_bound_trip(BOUND_TRIP_COUNTRY_LINK_INDEX, u64::from(index));
        true
    }
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

    #[test]
    fn tag_db_gate_follows_the_callers_table_not_a_constant() {
        let _guard = lock_registry();
        drain_frame_trips(); // clean slate
        let id = BOUND_TRIP_TAG_DB_ID_1024;

        // A 440-row vanilla database (the size the t11 capture measures: 439
        // country tags plus slot 0). The old 1024 admitted 584 ids the
        // engine's own accessor refuses.
        assert!(!tag_db_id_out_of_table(1, 440));
        assert!(!tag_db_id_out_of_table(439, 440));
        assert!(tag_db_id_out_of_table(440, 440), "size is exclusive");
        assert!(
            tag_db_id_out_of_table(1_000, 440),
            "the old 1024 ceiling admitted this"
        );
        assert!(
            !tag_db_id_out_of_table(0, 440),
            "the null country is in range; refusing it is caller policy"
        );
        let drained = drain_frame_trips();
        assert_eq!(
            drained[id].0, 2,
            "440 and 1000 trip; the zero id is a vacant slot, never a bound trip"
        );
        assert_eq!(drained[id].1, 1_000);

        // A total-conversion database with 12_000 tags: ids the old ceiling
        // silently dropped are ordinary rows here.
        assert!(!tag_db_id_out_of_table(1_025, 12_000));
        assert!(!tag_db_id_out_of_table(11_999, 12_000));
        assert!(tag_db_id_out_of_table(12_000, 12_000));
        let drained = drain_frame_trips();
        assert_eq!(drained[id].0, 1);
        assert_eq!(drained[id].1, 12_000);
    }

    #[test]
    fn a_link_index_never_reports_as_a_country_database_id() {
        let _guard = lock_registry();
        drain_frame_trips(); // clean slate

        // `_CountryLinkTags` grows on every dynamic release, so its live size
        // is routinely different from the database's. An index inside it is
        // not a bound trip; one past it is, under its OWN id.
        assert!(!country_link_index_out_of_vector(1_500, 2_000));
        assert!(country_link_index_out_of_vector(2_000, 2_000));
        assert!(country_link_index_out_of_vector(9_999, 2_000));

        let drained = drain_frame_trips();
        assert_eq!(drained[BOUND_TRIP_COUNTRY_LINK_INDEX].0, 2);
        assert_eq!(drained[BOUND_TRIP_COUNTRY_LINK_INDEX].1, 9_999);
        assert_eq!(
            drained[BOUND_TRIP_TAG_DB_ID_1024].0, 0,
            "a link-vector failure must never name the country database's bound"
        );
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
