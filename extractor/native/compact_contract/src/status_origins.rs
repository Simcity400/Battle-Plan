//! C1 stage A — per-subsource status-origin evidence units (G4 amendment 4;
//! spec = the C0_DOSSIERS.md "ranked telemetry" ledger + per-domain deciding
//! units, 2026-07-11/12).
//!
//! A domain's OR'd status word says a bit was set but not WHERE or WHY: seven
//! d69 failure reasons once collapsed into one boolean, and the C0 census left
//! nine domain/bit pairs "cannot-determine-from-context" because the first
//! introduction site of the bit is invisible in the stream. This registry
//! gives every such site a NAMED unit: readers call `record()` (a counter)
//! and optionally `record_sample()` (a last-value raw word — flags, reason
//! hash, row count) at the point the bit is FIRST introduced. The collector
//! drains once per frame into `so_*` diagnostic units on the reserved probe
//! domain; the recorder aggregates each into {count, first_hour, last_hour}
//! (+ the sample), which is exactly the incidence ledger C0 round 3 needs —
//! one live capture then attributes every recurring bit without further
//! static reading.
//!
//! NEVER changes capture semantics: pure instrumentation, bits are still set
//! exactly as before; the fold ignores PERF records and the coverage/
//! perf-count gates exclude the probe domain.
//!
//! Concurrency: relaxed atomics, recorded from worker threads, drained by the
//! frame thread (same contract as [`crate::bound_trips`]).

use core::sync::atomic::{AtomicU64, Ordering};

/// Sentinel game hour carried by an attribution-probe record emitted on a
/// frame with no valid clock (NO_CLOCK). The recorder counts the incidence
/// but must NOT fold this into a ledger row's first/last-hour span — a
/// NO_CLOCK frame stamping calendar hour 0 fabricated a false `first_hour=0`
/// (C1 stage A gate finding 3).
pub const ATTRIBUTION_NO_CLOCK_HOUR: i32 = i32::MIN;

/// Site identities. One per FIRST-INTRODUCTION site of a status bit in a
/// reader (or per cause branch within one site when the dossier requires the
/// split). Line references are the C0 round-2 verified sites at tree
/// c2f3a817f; they name the code, the unit name is the stable contract.
pub const SO_D45_PARTIAL_TARGETED_TIMED_STRUCTURAL: usize = 0; // lib.rs 38628
pub const SO_D48_PARTIAL_INHERITED: usize = 1; // lib.rs 23422
pub const SO_D48_PARTIAL_ROOTS_ERR: usize = 2; // lib.rs 23437
pub const SO_D48_PARTIAL_HARD_OR_GATE: usize = 3; // lib.rs 23463
pub const SO_NAVAL_PARTIAL_INHERITED: usize = 4; // lib.rs 22945 (d49+d50)
pub const SO_NAVAL_PARTIAL_TORN_WALK: usize = 5; // lib.rs 23029/23066
pub const SO_NAVAL_PARTIAL_JOIN: usize = 6; // lib.rs 23116 (Torn|Degraded)
pub const SO_NAVAL_SOURCE_EMPTY: usize = 7; // lib.rs 22907
pub const SO_NAVAL_WORKER_PANIC: usize = 8;
pub const SO_D60_SOURCE_UNAVAILABLE: usize = 9; // lib.rs 21352-53
pub const SO_D60_PARTIAL_INHERITED: usize = 10; // lib.rs 21409
pub const SO_D60_PARTIAL_TORN_ROW: usize = 11; // lib.rs 21571
pub const SO_D60_WORKER_PANIC: usize = 12; // lib.rs 21611
pub const SO_D53_SOURCE_UNAVAILABLE: usize = 13; // domain53_source_unavailable_read
pub const SO_D53_PARTIAL_INHERITED: usize = 14; // lib.rs 19924
pub const SO_D53_PARTIAL_ROW_SKIPPED: usize = 15; // partial_frame: skipped_rows != 0
pub const SO_D53_PARTIAL_SHADOW_MISMATCH: usize = 16; // relation_shadow_mismatches != 0
pub const SO_D53_WORKER_PANIC: usize = 17;
pub const SO_D54_PARTIAL_INHERITED: usize = 18; // lib.rs 27110
pub const SO_D54_PARTIAL_ARMY_ORDERS: usize = 19; // lib.rs 27154
pub const SO_D54_PARTIAL_OOB_COMMAND: usize = 20; // lib.rs 27166
pub const SO_D54_HARD_DEPENDENCY: usize = 21; // lib.rs 27174 (PARTIAL|SOURCE)
pub const SO_D54_PARTIAL_SKIPPED_ROWS: usize = 22; // lib.rs 27250
pub const SO_D65_SOURCE_EMPTY_NO_WITNESS: usize = 23; // lib.rs 37474
pub const SO_D65_PARTIAL_DROPPED_AMBIGUOUS: usize = 24; // lib.rs 37480
pub const SO_D55_TRANSITION_BASELINE_RESET: usize = 25; // lib.rs 25310 (census had misattributed to d51)
pub const SO_FRAME_CONTROL_INVALID: usize = 26; // lib.rs 14247 (extraction gate, frame-scoped)
/// d43 empty contract set while the async sweep snapshot claimed
/// warm-complete-empty — fails closed (G1-residue finding 2: an async pass is
/// not a current-frame emptiness witness) but counted separately from cold so
/// C0 can see the would-be MeasuredEmpty incidence the C1 registry needs.
pub const SO_D43_SOURCE_EMPTY_WARM_KNOWN: usize = 27;
/// d43 empty contract set with a cold/degraded snapshot (no coverage proof).
pub const SO_D43_SOURCE_EMPTY_COLD: usize = 28;
/// d53 post-build gate: `!report.ok() || payload_failure` introduces
/// SOURCE|PARTIAL from a DECODE/gate failure of the built payload — a
/// distinct first-introduction site from the source-unavailable funnel
/// (stage-A gate finding 1).
pub const SO_D53_POSTBUILD_GATE: usize = 29;
/// d48 worker panic introduces WORKER_PANIC|PARTIAL (stage-A finding 1).
pub const SO_D48_WORKER_PANIC: usize = 30;
/// d54 dependency SOURCE-unavailable helper (missing army/oob dependency).
pub const SO_D54_SOURCE_DEPENDENCY: usize = 31;
/// d54 worker panic.
pub const SO_D54_WORKER_PANIC: usize = 32;
/// d65 early SOURCE: country identity has no scalar source this frame (the
/// walk never starts). Stage-A residue producer (fix-round re-gate
/// adjudication): the terminal empty-no-witness site could not distinguish
/// "walk found nothing" from "walk never ran".
pub const SO_D65_SOURCE_NO_SCALAR: usize = 33;
/// d65 early SOURCE: a DOMAIN65 vtable RVA overflowed module_base — build
/// profile/base mismatch, the walk never starts.
pub const SO_D65_SOURCE_VTABLE_RVA: usize = 34;
/// d48 lane-walk partial causes (stage-A residue producer): the lane module
/// itself sets PARTIAL from per-arm walk failures the inherited/hard-or-gate
/// sites never see. ONE ID PER CAUSE (stage-A residue gate MEDIUM: a packed
/// multi-cause sample under last-value semantics kept only the FINAL frame's
/// causes — hour 100 country-root + hour 101 resource-row left no trace of
/// the country arm). Count sums affected frames per cause; sample = the
/// cause's last per-frame counter; hour spans come from the recorder agg.
pub const SO_D48_LANE_FAILED_COUNTRY_ROOTS: usize = 35;
pub const SO_D48_LANE_FAILED_STATE_BUILDING_ROOTS: usize = 36;
pub const SO_D48_LANE_FAILED_STATE_RESOURCE_ROOTS: usize = 37;
pub const SO_D48_LANE_FAILED_PROVINCE_BUILDING_ROOTS: usize = 38;
pub const SO_D48_LANE_SKIPPED_CONSTRUCTION_ROWS: usize = 39;
pub const SO_D48_LANE_SKIPPED_BUILDING_ROWS: usize = 40;
pub const SO_D48_LANE_SKIPPED_RESOURCE_ROWS: usize = 41;
/// d65 rooted walk began from a validated country but a status/vector/program
/// read was torn or structurally invalid.  The row set must be retained as
/// partial rather than interpreted as authoritative deletion.
pub const SO_D65_PARTIAL_WALK_FAILURE: usize = 42;
pub const STATUS_ORIGIN_ID_COUNT: usize = 43;

/// Unit names, index-aligned with the ids. `<name>` counts introductions
/// (summed in the recorder); `<name>_sample` (emitted only when a site
/// recorded one this frame) carries the site's deciding raw word — incoming
/// status flags, a reason hash, a packed row count — with last-value
/// semantics. Charset: ascii lowercase/digits/underscore (recorder rule).
pub const STATUS_ORIGIN_UNITS: [&str; STATUS_ORIGIN_ID_COUNT] = [
    "so_d45_partial_targeted_timed_structural",
    "so_d48_partial_inherited",
    "so_d48_partial_roots_err",
    "so_d48_partial_hard_or_gate",
    "so_naval_partial_inherited",
    "so_naval_partial_torn_walk",
    "so_naval_partial_join",
    "so_naval_source_empty",
    "so_naval_worker_panic",
    "so_d60_source_unavailable",
    "so_d60_partial_inherited",
    "so_d60_partial_torn_row",
    "so_d60_worker_panic",
    "so_d53_source_unavailable",
    "so_d53_partial_inherited",
    "so_d53_partial_row_skipped",
    "so_d53_partial_shadow_mismatch",
    "so_d53_worker_panic",
    "so_d54_partial_inherited",
    "so_d54_partial_army_orders",
    "so_d54_partial_oob_command",
    "so_d54_hard_dependency",
    "so_d54_partial_skipped_rows",
    "so_d65_source_empty_no_witness",
    "so_d65_partial_dropped_ambiguous",
    "so_d55_transition_baseline_reset",
    "so_frame_control_invalid",
    "so_d43_source_empty_warm_known",
    "so_d43_source_empty_cold",
    "so_d53_postbuild_gate",
    "so_d48_worker_panic",
    "so_d54_source_dependency",
    "so_d54_worker_panic",
    "so_d65_source_no_scalar",
    "so_d65_source_vtable_rva",
    "so_d48_lane_failed_country_roots",
    "so_d48_lane_failed_state_building_roots",
    "so_d48_lane_failed_state_resource_roots",
    "so_d48_lane_failed_province_building_roots",
    "so_d48_lane_skipped_construction_rows",
    "so_d48_lane_skipped_building_rows",
    "so_d48_lane_skipped_resource_rows",
    "so_d65_partial_walk_failure",
];

#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_ZERO: AtomicU64 = AtomicU64::new(0);
/// Packed per-id word: low 63 bits = introduction count since last drain,
/// bit 63 = "a sample was recorded in this batch". Count and sample-flag MUST
/// live in ONE atomic (R2-gate finding-1 fix round, second iteration): with a
/// separate flag cell, a drain racing a recorder could consume the flag
/// against the WRONG count batch — either a count=0 drain stealing a fresh
/// flag, or a fresh flag being consumed by an older batch's drain, leaving
/// the next drain to report counted sample-recordings with no sample. A
/// single swapped word makes (count, flag) inseparable.
static ORIGIN_PACKED: [AtomicU64; STATUS_ORIGIN_ID_COUNT] = [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];
static ORIGIN_SAMPLES: [AtomicU64; STATUS_ORIGIN_ID_COUNT] = [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];

const SAMPLE_FLAG: u64 = 1 << 63;
const COUNT_MASK: u64 = SAMPLE_FLAG - 1;

/// Count one introduction of a status bit at site `id`. Out-of-range ids are
/// ignored (defensive; callers use the constants).
pub fn record_status_origin(id: usize) {
    if id >= STATUS_ORIGIN_ID_COUNT {
        return;
    }
    ORIGIN_PACKED[id].fetch_add(1, Ordering::Release);
}

/// Count one introduction AND attach this frame's deciding raw word (flags,
/// reason hash, packed counts). Last value wins within a frame.
///
/// The sample value is stored BEFORE the packed count+flag publishes
/// (Release); the drain swaps the packed word with Acquire — so a drain
/// observing the flagged count also observes a genuine sample value.
pub fn record_status_origin_sample(id: usize, sample: u64) {
    if id >= STATUS_ORIGIN_ID_COUNT {
        return;
    }
    ORIGIN_SAMPLES[id].store(sample, Ordering::Relaxed);
    // CAS loop: increment the count AND set the flag in one atomic word.
    let mut current = ORIGIN_PACKED[id].load(Ordering::Relaxed);
    loop {
        let next = (current.wrapping_add(1)) | SAMPLE_FLAG;
        match ORIGIN_PACKED[id].compare_exchange_weak(
            current,
            next,
            Ordering::Release,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

/// Drain this frame's introductions: per id, (count since last drain,
/// Some(sample) if a sample was recorded since last drain). Index-aligned
/// with [`STATUS_ORIGIN_UNITS`]. Counts and the sample-set flag reset on
/// drain; a racing record lands in the next frame's drain (never lost).
pub fn drain_frame_origins() -> [(u64, Option<u64>); STATUS_ORIGIN_ID_COUNT] {
    let mut out = [(0u64, None); STATUS_ORIGIN_ID_COUNT];
    for (id, slot) in out.iter_mut().enumerate() {
        // Acquire pairs with the Release publishes in the record fns: a
        // flagged count's sample store is visible below. Swapping ONE word
        // keeps the count and its sample-flag inseparable.
        let packed = ORIGIN_PACKED[id].swap(0, Ordering::Acquire);
        let count = packed & COUNT_MASK;
        let sample =
            (packed & SAMPLE_FLAG != 0).then(|| ORIGIN_SAMPLES[id].load(Ordering::Relaxed));
        *slot = (count, sample);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    // R2 gate finding 4 (bound_trips twin): drain_frame_origins() resets
    // EVERY id, so parallel tests would steal each other's counts. Every
    // test that records or drains takes this lock.
    static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

    fn lock_registry() -> MutexGuard<'static, ()> {
        REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn record_and_drain_counts_reset() {
        let _guard = lock_registry();
        drain_frame_origins(); // clean slate
        let id = SO_D45_PARTIAL_TARGETED_TIMED_STRUCTURAL;
        record_status_origin(id);
        record_status_origin(id);
        let drained = drain_frame_origins()[id];
        assert_eq!(drained, (2, None));
        assert_eq!(drain_frame_origins()[id], (0, None));
    }

    #[test]
    fn sample_emitted_only_on_recording_frames_last_value_wins() {
        let _guard = lock_registry();
        drain_frame_origins(); // clean slate
        let id = SO_D60_SOURCE_UNAVAILABLE;
        record_status_origin_sample(id, 0x100);
        record_status_origin_sample(id, 0x200);
        let drained = drain_frame_origins()[id];
        assert_eq!(drained, (2, Some(0x200)));
        // Next frame without recordings: no count, no sample echo.
        assert_eq!(drain_frame_origins()[id], (0, None));
    }

    #[test]
    fn zero_sample_value_is_still_a_sample() {
        // A site recording flags=0 must not be confused with "no sample".
        let _guard = lock_registry();
        drain_frame_origins(); // clean slate
        let id = SO_FRAME_CONTROL_INVALID;
        record_status_origin_sample(id, 0);
        assert_eq!(drain_frame_origins()[id], (1, Some(0)));
    }

    #[test]
    fn concurrent_drain_never_sees_count_without_sample() {
        // Sample-publish ordering (R2 gate finding 1 twin): every recording
        // here carries a sample, so a drained (count>0, None) pair means a
        // consumed count whose deciding word was lost.
        let _guard = lock_registry();
        drain_frame_origins(); // clean slate
        let id = SO_D48_PARTIAL_HARD_OR_GATE;
        let recorder = std::thread::spawn(move || {
            for i in 0..20_000u64 {
                record_status_origin_sample(id, 0x1000 + (i % 5));
            }
        });
        let mut drained_total = 0u64;
        loop {
            let (count, sample) = drain_frame_origins()[id];
            if count != 0 {
                assert!(
                    sample.is_some(),
                    "drained counted sample-recordings without their sample"
                );
                drained_total += count;
            }
            if recorder.is_finished() {
                break;
            }
        }
        recorder.join().unwrap();
        let (count, sample) = drain_frame_origins()[id];
        if count != 0 {
            assert!(sample.is_some());
            drained_total += count;
        }
        assert_eq!(drained_total, 20_000, "no incidence count may be lost");
    }

    #[test]
    fn out_of_range_ids_are_ignored() {
        record_status_origin(STATUS_ORIGIN_ID_COUNT);
        record_status_origin_sample(usize::MAX, 7);
    }

    #[test]
    fn unit_names_satisfy_charset_prefix_and_uniqueness() {
        let mut seen = std::collections::BTreeSet::new();
        for name in STATUS_ORIGIN_UNITS {
            assert!(name.starts_with("so_"));
            assert!(name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
            // The recorder derives `<name>_sample`; the base name must not
            // itself look like a sample unit.
            assert!(!name.ends_with("_sample"));
            assert!(seen.insert(name), "duplicate unit name {name}");
        }
    }

    #[test]
    fn every_id_constant_indexes_its_exact_unit_name() {
        // Combined-gate MEDIUM: swapping two names in the array compiles and
        // passes the uniqueness test, permanently mislabeling two causes in
        // every manifest. Pin the FULL id -> name registry mechanically.
        let expected: [(usize, &str); STATUS_ORIGIN_ID_COUNT] = [
            (
                SO_D45_PARTIAL_TARGETED_TIMED_STRUCTURAL,
                "so_d45_partial_targeted_timed_structural",
            ),
            (SO_D48_PARTIAL_INHERITED, "so_d48_partial_inherited"),
            (SO_D48_PARTIAL_ROOTS_ERR, "so_d48_partial_roots_err"),
            (SO_D48_PARTIAL_HARD_OR_GATE, "so_d48_partial_hard_or_gate"),
            (SO_NAVAL_PARTIAL_INHERITED, "so_naval_partial_inherited"),
            (SO_NAVAL_PARTIAL_TORN_WALK, "so_naval_partial_torn_walk"),
            (SO_NAVAL_PARTIAL_JOIN, "so_naval_partial_join"),
            (SO_NAVAL_SOURCE_EMPTY, "so_naval_source_empty"),
            (SO_NAVAL_WORKER_PANIC, "so_naval_worker_panic"),
            (SO_D60_SOURCE_UNAVAILABLE, "so_d60_source_unavailable"),
            (SO_D60_PARTIAL_INHERITED, "so_d60_partial_inherited"),
            (SO_D60_PARTIAL_TORN_ROW, "so_d60_partial_torn_row"),
            (SO_D60_WORKER_PANIC, "so_d60_worker_panic"),
            (SO_D53_SOURCE_UNAVAILABLE, "so_d53_source_unavailable"),
            (SO_D53_PARTIAL_INHERITED, "so_d53_partial_inherited"),
            (SO_D53_PARTIAL_ROW_SKIPPED, "so_d53_partial_row_skipped"),
            (
                SO_D53_PARTIAL_SHADOW_MISMATCH,
                "so_d53_partial_shadow_mismatch",
            ),
            (SO_D53_WORKER_PANIC, "so_d53_worker_panic"),
            (SO_D54_PARTIAL_INHERITED, "so_d54_partial_inherited"),
            (SO_D54_PARTIAL_ARMY_ORDERS, "so_d54_partial_army_orders"),
            (SO_D54_PARTIAL_OOB_COMMAND, "so_d54_partial_oob_command"),
            (SO_D54_HARD_DEPENDENCY, "so_d54_hard_dependency"),
            (SO_D54_PARTIAL_SKIPPED_ROWS, "so_d54_partial_skipped_rows"),
            (
                SO_D65_SOURCE_EMPTY_NO_WITNESS,
                "so_d65_source_empty_no_witness",
            ),
            (
                SO_D65_PARTIAL_DROPPED_AMBIGUOUS,
                "so_d65_partial_dropped_ambiguous",
            ),
            (
                SO_D55_TRANSITION_BASELINE_RESET,
                "so_d55_transition_baseline_reset",
            ),
            (SO_FRAME_CONTROL_INVALID, "so_frame_control_invalid"),
            (
                SO_D43_SOURCE_EMPTY_WARM_KNOWN,
                "so_d43_source_empty_warm_known",
            ),
            (SO_D43_SOURCE_EMPTY_COLD, "so_d43_source_empty_cold"),
            (SO_D53_POSTBUILD_GATE, "so_d53_postbuild_gate"),
            (SO_D48_WORKER_PANIC, "so_d48_worker_panic"),
            (SO_D54_SOURCE_DEPENDENCY, "so_d54_source_dependency"),
            (SO_D54_WORKER_PANIC, "so_d54_worker_panic"),
            (SO_D65_SOURCE_NO_SCALAR, "so_d65_source_no_scalar"),
            (SO_D65_SOURCE_VTABLE_RVA, "so_d65_source_vtable_rva"),
            (
                SO_D48_LANE_FAILED_COUNTRY_ROOTS,
                "so_d48_lane_failed_country_roots",
            ),
            (
                SO_D48_LANE_FAILED_STATE_BUILDING_ROOTS,
                "so_d48_lane_failed_state_building_roots",
            ),
            (
                SO_D48_LANE_FAILED_STATE_RESOURCE_ROOTS,
                "so_d48_lane_failed_state_resource_roots",
            ),
            (
                SO_D48_LANE_FAILED_PROVINCE_BUILDING_ROOTS,
                "so_d48_lane_failed_province_building_roots",
            ),
            (
                SO_D48_LANE_SKIPPED_CONSTRUCTION_ROWS,
                "so_d48_lane_skipped_construction_rows",
            ),
            (
                SO_D48_LANE_SKIPPED_BUILDING_ROWS,
                "so_d48_lane_skipped_building_rows",
            ),
            (
                SO_D48_LANE_SKIPPED_RESOURCE_ROWS,
                "so_d48_lane_skipped_resource_rows",
            ),
            (
                SO_D65_PARTIAL_WALK_FAILURE,
                "so_d65_partial_walk_failure",
            ),
        ];
        for (id, name) in expected {
            assert_eq!(
                STATUS_ORIGIN_UNITS[id], name,
                "id {id} must index unit {name}"
            );
        }
        // Every id 0..COUNT is covered exactly once by a named constant.
        let mut ids: Vec<usize> = expected.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), STATUS_ORIGIN_ID_COUNT);
        assert_eq!(ids.first(), Some(&0));
        assert_eq!(ids.last(), Some(&(STATUS_ORIGIN_ID_COUNT - 1)));
    }
}
