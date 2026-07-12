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
pub const STATUS_ORIGIN_ID_COUNT: usize = 27;

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
];

#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_ZERO: AtomicU64 = AtomicU64::new(0);
static ORIGIN_COUNTS: [AtomicU64; STATUS_ORIGIN_ID_COUNT] =
    [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];
static ORIGIN_SAMPLES: [AtomicU64; STATUS_ORIGIN_ID_COUNT] =
    [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];
static ORIGIN_SAMPLE_SET: [AtomicU64; STATUS_ORIGIN_ID_COUNT] =
    [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];

/// Count one introduction of a status bit at site `id`. Out-of-range ids are
/// ignored (defensive; callers use the constants).
pub fn record_status_origin(id: usize) {
    if id >= STATUS_ORIGIN_ID_COUNT {
        return;
    }
    ORIGIN_COUNTS[id].fetch_add(1, Ordering::Relaxed);
}

/// Count one introduction AND attach this frame's deciding raw word (flags,
/// reason hash, packed counts). Last value wins within a frame.
pub fn record_status_origin_sample(id: usize, sample: u64) {
    if id >= STATUS_ORIGIN_ID_COUNT {
        return;
    }
    ORIGIN_COUNTS[id].fetch_add(1, Ordering::Relaxed);
    ORIGIN_SAMPLES[id].store(sample, Ordering::Relaxed);
    ORIGIN_SAMPLE_SET[id].store(1, Ordering::Relaxed);
}

/// Drain this frame's introductions: per id, (count since last drain,
/// Some(sample) if a sample was recorded since last drain). Index-aligned
/// with [`STATUS_ORIGIN_UNITS`]. Counts and the sample-set flag reset on
/// drain; a racing record lands in the next frame's drain (never lost).
pub fn drain_frame_origins() -> [(u64, Option<u64>); STATUS_ORIGIN_ID_COUNT] {
    let mut out = [(0u64, None); STATUS_ORIGIN_ID_COUNT];
    for (id, slot) in out.iter_mut().enumerate() {
        let count = ORIGIN_COUNTS[id].swap(0, Ordering::Relaxed);
        let sample_set = ORIGIN_SAMPLE_SET[id].swap(0, Ordering::Relaxed) != 0;
        let sample = sample_set.then(|| ORIGIN_SAMPLES[id].load(Ordering::Relaxed));
        *slot = (count, sample);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Distinct ids per test — the registry is global and tests run threaded.

    #[test]
    fn record_and_drain_counts_reset() {
        let id = SO_D45_PARTIAL_TARGETED_TIMED_STRUCTURAL;
        record_status_origin(id);
        record_status_origin(id);
        let drained = drain_frame_origins()[id];
        assert_eq!(drained, (2, None));
        assert_eq!(drain_frame_origins()[id], (0, None));
    }

    #[test]
    fn sample_emitted_only_on_recording_frames_last_value_wins() {
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
        let id = SO_FRAME_CONTROL_INVALID;
        record_status_origin_sample(id, 0);
        assert_eq!(drain_frame_origins()[id], (1, Some(0)));
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
}
