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
pub const BOUND_TRIP_TAG_DB_ID_1024: usize = 5;
pub const BOUND_TRIP_FACTION_SLOTS: usize = 6;
pub const BOUND_TRIP_FACTION_MEMBERS: usize = 7;
pub const BOUND_TRIP_ID_COUNT: usize = 8;

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
];

#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_ZERO: AtomicU64 = AtomicU64::new(0);
static TRIP_COUNTS: [AtomicU64; BOUND_TRIP_ID_COUNT] = [ATOMIC_ZERO; BOUND_TRIP_ID_COUNT];
static TRIP_LAST_RAW: [AtomicU64; BOUND_TRIP_ID_COUNT] = [ATOMIC_ZERO; BOUND_TRIP_ID_COUNT];

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

/// Drain per-frame trip counts (resetting them) and read each bound's
/// last-observed raw value. Index-aligned with [`BOUND_TRIP_UNITS`]. The
/// last-raw cell is intentionally NOT reset: it is a `_sample` (last observed
/// value), and a trip landing between the swap and the load is still a
/// genuine observed trip.
pub fn drain_frame_trips() -> [(u64, u64); BOUND_TRIP_ID_COUNT] {
    let mut out = [(0u64, 0u64); BOUND_TRIP_ID_COUNT];
    for (id, slot) in out.iter_mut().enumerate() {
        // Acquire pairs with the Release publish in record_bound_trip: a
        // counted trip's raw store is visible to the load below.
        let count = TRIP_COUNTS[id].swap(0, Ordering::Acquire);
        let raw = TRIP_LAST_RAW[id].load(Ordering::Relaxed);
        *slot = (count, raw);
    }
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
