//! Per-subsource status-origin evidence units.
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
//! Concurrency: lock-free atomics, recorded from worker threads, drained by
//! the frame thread (same contract as [`crate::bound_trips`]). A recorder
//! publishes with Release and a reservation reads with Acquire; everything a
//! drain decides from lives in ONE word per site, so a reservation can never
//! observe a count and its sample evidence out of step. See `ORIGIN_PACKED`.

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
/// d55 discarded a captured terminal combat record it PROVED has no publishable
/// frame left, and introduced PARTIAL_DATA for it.
///
/// A terminal record row is legal only in the payload that also carries its
/// `EVENT_PHASE_ENDED` row, and that row is emitted exactly once. When the
/// record cannot go there — its ended frame has passed, its bytes fail
/// authentication, or it disagrees with the ended row that is present — the
/// record is real data that will never be published, so the deciding frame says
/// PARTIAL_DATA once and drops it, rather than re-offering a decided record
/// forever. NOT `SOURCE_UNAVAILABLE`: the read completed, and this verdict is
/// its result.
///
/// Sample word: `(reason_code << 32) | discarded_this_frame`, where the reason
/// code is the LAST reason decided in the frame.
pub const SO_D55_TERMINAL_RECORD_UNPUBLISHABLE: usize = 43;
/// d65 / d66 refused to walk because the build profile the frame is running
/// against is not the one their offsets were measured against.
///
/// NOT a torn read: nothing was read. Every vtable RVA and member offset the
/// walk would use is unverified for this build, so publishing its rows would
/// present an unverified read as the domain's answer. The sibling d52 walk over
/// the same game objects already refuses here. Sample word: the incoming frame
/// status flags.
pub const SO_D65_PARTIAL_PROFILE_MISMATCH: usize = 44;
pub const SO_D66_PARTIAL_PROFILE_MISMATCH: usize = 45;
pub const STATUS_ORIGIN_ID_COUNT: usize = 46;

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
    "so_d55_terminal_record_unpublishable",
    "so_d65_partial_profile_mismatch",
    "so_d66_partial_profile_mismatch",
];

#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_ZERO: AtomicU64 = AtomicU64::new(0);
/// Packed per-id word — the SINGLE cell every drain decision is derived from.
///
/// * bits 0..32  — introductions recorded since the last commit;
/// * bits 32..64 — how many of those carried a sample.
///
/// Count and sample evidence MUST be inseparable (R2-gate finding-1 fix
/// round, second iteration): with a separate flag cell, a drain racing a
/// recorder could consume the flag against the WRONG count batch, leaving the
/// next drain to report counted sample-recordings with no sample.
///
/// The first repair kept the flag in this word but added a SEPARATE
/// `ORIGIN_SAMPLE_GENERATIONS` cell so `commit` could tell "the samples I
/// reserved" from "a sample recorded after I reserved". That re-introduced
/// exactly the defect it was meant to close, one level down, because a
/// recorder bumps the generation BEFORE it publishes the count: a reservation
/// that read the packed word and THEN the generation could capture a
/// generation belonging to an introduction its own count snapshot had not yet
/// seen. `commit` then compared equal generations, concluded "no new sample",
/// and cleared the flag over a still-queued sampled introduction — the next
/// drain published a count with no sample, and the collector emitted
/// `so_<name>` with no `so_<name>_sample` beside it (attach_log.rs). Reading
/// the generation first does not help: the recorder can be preempted between
/// its generation bump and its count publish, so no read order of two cells is
/// safe. The generation is therefore folded INTO this word as an exact
/// sampled-introduction count, which `commit` subtracts atomically together
/// with the plain count. There is no second cell left to skew.
///
/// The count field saturates instead of carrying into the sampled field: an
/// overflowing count must never fabricate sample evidence. 2^32-1 unread
/// introductions for a single site between two frame drains is not reachable
/// in the collector (every frame reserves), and under-reporting fails safe
/// where field corruption would not.
static ORIGIN_PACKED: [AtomicU64; STATUS_ORIGIN_ID_COUNT] = [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];
static ORIGIN_SAMPLES: [AtomicU64; STATUS_ORIGIN_ID_COUNT] = [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];
static ACTIVE_FRAME_RESERVATION: AtomicU64 = AtomicU64::new(0);
static NEXT_FRAME_RESERVATION: AtomicU64 = AtomicU64::new(1);

const COUNT_BITS: u32 = 32;
const COUNT_MASK: u64 = (1u64 << COUNT_BITS) - 1;
/// Adding this bumps the sampled-introduction field by one.
const SAMPLED_UNIT: u64 = 1u64 << COUNT_BITS;

const fn packed_count(packed: u64) -> u64 {
    packed & COUNT_MASK
}

const fn packed_sampled(packed: u64) -> u64 {
    packed >> COUNT_BITS
}

/// Stage this frame's deciding raw word for site `id`. Must happen BEFORE the
/// introduction publishes; [`publish_introduction`]'s Release RMW is what
/// makes it visible to an Acquire reservation.
fn stage_sample_value(id: usize, sample: u64) {
    ORIGIN_SAMPLES[id].store(sample, Ordering::Relaxed);
}

// Observation point between staging a sample value and publishing its
// introduction. Compiles to nothing outside the crate's own test build; it
// exists so `sample_value_is_staged_before_its_introduction_publishes` can
// prove the staging comes FIRST — a reservation taken at this point must not
// yet see the introduction, and therefore cannot claim a value that is still
// the previous frame's.
//
// Thread-local rather than a lock: an observer must not add synchronisation to
// the recorder path even in a test build, or the concurrent stress test would
// be measuring the hook's latency instead of the registry's.
#[cfg(test)]
thread_local! {
    static MID_RECORD_OBSERVER: core::cell::Cell<Option<fn(usize)>> =
        const { core::cell::Cell::new(None) };
}

/// See `MID_RECORD_OBSERVER`. A no-op outside the crate's own test build.
#[cfg(test)]
fn observe_between_staging_and_publish(id: usize) {
    if let Some(hook) = MID_RECORD_OBSERVER.with(|slot| slot.get()) {
        hook(id);
    }
}

#[cfg(not(test))]
#[inline(always)]
fn observe_between_staging_and_publish(_id: usize) {}

/// Publish one introduction at site `id` — the single atomic event that makes
/// the introduction, and (when `sampled`) its evidence, visible to a drain.
fn publish_introduction(id: usize, sampled: bool) {
    let delta = if sampled { SAMPLED_UNIT | 1 } else { 1 };
    let mut current = ORIGIN_PACKED[id].load(Ordering::Relaxed);
    loop {
        if packed_count(current) == COUNT_MASK {
            // Saturated: drop the introduction rather than carry into the
            // sampled field and fabricate evidence for it.
            return;
        }
        match ORIGIN_PACKED[id].compare_exchange_weak(
            current,
            current + delta,
            Ordering::Release,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

/// Count one introduction of a status bit at site `id`. Out-of-range ids are
/// ignored (defensive; callers use the constants).
pub fn record_status_origin(id: usize) {
    if id >= STATUS_ORIGIN_ID_COUNT {
        return;
    }
    publish_introduction(id, false);
}

/// Count one introduction AND attach this frame's deciding raw word (flags,
/// reason hash, packed counts). Last value wins within a frame.
///
/// The sample value is staged BEFORE the packed word publishes (Release); a
/// reservation loads that word with Acquire — so a reservation that counts a
/// sampled introduction also observes a genuine sample value.
pub fn record_status_origin_sample(id: usize, sample: u64) {
    if id >= STATUS_ORIGIN_ID_COUNT {
        return;
    }
    stage_sample_value(id, sample);
    observe_between_staging_and_publish(id);
    publish_introduction(id, true);
}

/// Immutable frame-scoped view of status-origin introductions. The represented
/// counter prefix remains live until authoritative frame publication commits
/// the reservation.
#[derive(Debug)]
pub struct StatusOriginFrameReservation {
    reservation_id: u64,
    snapshot: [(u64, Option<u64>); STATUS_ORIGIN_ID_COUNT],
    /// The exact packed words this reservation read — both fields, as one
    /// value. `commit` subtracts these, so the counts it consumes and the
    /// sampled-introduction credit it consumes can never come from different
    /// observations of the registry.
    reserved_packed: [u64; STATUS_ORIGIN_ID_COUNT],
    active: bool,
}

impl StatusOriginFrameReservation {
    pub fn snapshot(&self) -> &[(u64, Option<u64>); STATUS_ORIGIN_ID_COUNT] {
        &self.snapshot
    }

    /// Consume only the snapshotted counter prefix. Introductions recorded
    /// after the snapshot remain queued, and a post-snapshot SAMPLED
    /// introduction keeps its sampled credit in the packed word, so the next
    /// frame reports it with its sample.
    pub fn commit(mut self) -> Result<(), String> {
        ensure_active_reservation(self.reservation_id)?;
        for (id, reserved) in self.reserved_packed.iter().copied().enumerate() {
            let current = ORIGIN_PACKED[id].load(Ordering::Acquire);
            if packed_count(current) < packed_count(reserved)
                || packed_sampled(current) < packed_sampled(reserved)
            {
                return Err(format!(
                    "status-origin reservation {} counter {} regressed from {}/{} to {}/{}",
                    self.reservation_id,
                    id,
                    packed_count(reserved),
                    packed_sampled(reserved),
                    packed_count(current),
                    packed_sampled(current)
                ));
            }
        }
        for (id, reserved) in self.reserved_packed.iter().copied().enumerate() {
            // One RMW consumes the reserved prefix of BOTH fields. Neither
            // field can be consumed against a different observation of the
            // other, and whatever a concurrent recorder adds — count and its
            // sampled credit together — survives untouched into the next
            // frame. Both fields only grow between commits and the guard
            // above proved `current >= reserved` in each, so the subtraction
            // cannot borrow across the field boundary.
            ORIGIN_PACKED[id].fetch_sub(reserved, Ordering::AcqRel);
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

impl Drop for StatusOriginFrameReservation {
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
            "status-origin reservation mismatch: active {}, requested {}",
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
                "status-origin reservation release mismatch: active {}, requested {}",
                active, reservation_id
            )
        })
}

/// Reserve the current status-origin prefix without resetting any counter.
pub fn reserve_frame_origins() -> Result<StatusOriginFrameReservation, String> {
    let reservation_id = NEXT_FRAME_RESERVATION.fetch_add(1, Ordering::Relaxed);
    if reservation_id == 0 {
        return Err("status-origin reservation id wrapped".into());
    }
    ACTIVE_FRAME_RESERVATION
        .compare_exchange(0, reservation_id, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|active| format!("status-origin frame reservation already active: {}", active))?;
    let mut snapshot = [(0u64, None); STATUS_ORIGIN_ID_COUNT];
    let mut reserved_packed = [0u64; STATUS_ORIGIN_ID_COUNT];
    for (id, slot) in snapshot.iter_mut().enumerate() {
        // ONE load decides both the reserved count and whether the reserved
        // prefix contains sample evidence. No second cell is consulted, so
        // there is no window in which the two can disagree.
        let packed = ORIGIN_PACKED[id].load(Ordering::Acquire);
        let sample =
            (packed_sampled(packed) != 0).then(|| ORIGIN_SAMPLES[id].load(Ordering::Relaxed));
        *slot = (packed_count(packed), sample);
        reserved_packed[id] = packed;
    }
    Ok(StatusOriginFrameReservation {
        reservation_id,
        snapshot,
        reserved_packed,
        active: true,
    })
}

/// Drain this frame's introductions: per id, (count since last drain,
/// Some(sample) if a sample was recorded since last drain). Index-aligned
/// with [`STATUS_ORIGIN_UNITS`]. Both fields of the packed word — the count
/// and the sampled-introduction tally — are consumed by exactly the drained
/// prefix; a racing record lands in the next frame's drain, still paired with
/// its sample, never orphaned. It is also never lost EXCEPT at count
/// saturation, where `publish_introduction` drops the introduction outright
/// rather than corrupt the sampled tally; see `ORIGIN_PACKED` for why that
/// bound is not reachable in the collector.
pub fn drain_frame_origins() -> [(u64, Option<u64>); STATUS_ORIGIN_ID_COUNT] {
    let reservation =
        reserve_frame_origins().expect("status-origin compatibility drain reservation");
    let out = *reservation.snapshot();
    reservation
        .commit()
        .expect("status-origin compatibility drain commit");
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
    fn frame_reservation_rolls_back_or_commits_only_its_prefix() {
        let _guard = lock_registry();
        drain_frame_origins();
        let id = SO_D65_PARTIAL_WALK_FAILURE;
        record_status_origin_sample(id, 0x100);
        let rolled_back = reserve_frame_origins().unwrap();
        assert_eq!(rolled_back.snapshot()[id], (1, Some(0x100)));
        record_status_origin_sample(id, 0x200);
        rolled_back.rollback().unwrap();
        assert_eq!(drain_frame_origins()[id], (2, Some(0x200)));

        record_status_origin_sample(id, 0x300);
        let committed = reserve_frame_origins().unwrap();
        record_status_origin_sample(id, 0x400);
        committed.commit().unwrap();
        assert_eq!(
            drain_frame_origins()[id],
            (1, Some(0x400)),
            "post-reservation sampled introductions must remain queued"
        );
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

    /// Where one recorder step lands relative to a frame reservation.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum At {
        BeforeReserve,
        DuringReservation,
        AfterCommit,
    }

    /// One introduction: where its sample value is staged (`None` = an
    /// unsampled `record_status_origin`) and where it publishes.
    #[derive(Clone, Copy, Debug)]
    struct Intro {
        stage: Option<(At, u64)>,
        publish: At,
    }

    /// Drive one interleaving and return
    /// `(reservation snapshot for id, next drain for id)`.
    ///
    /// The recorder's two steps are the SAME private functions
    /// `record_status_origin_sample` itself calls — `stage_sample_value` and
    /// `publish_introduction` — not reimplementations of them, so a change to
    /// the recorder's publish mechanics is exercised here rather than shadowed
    /// by a stale copy.
    fn run_interleaving(id: usize, intros: &[Intro]) -> ((u64, Option<u64>), (u64, Option<u64>)) {
        fn emit(id: usize, intros: &[Intro], at: At) {
            for intro in intros {
                if let Some((stage_at, value)) = intro.stage {
                    if stage_at == at {
                        stage_sample_value(id, value);
                    }
                }
                if intro.publish == at {
                    publish_introduction(id, intro.stage.is_some());
                }
            }
        }
        drain_frame_origins(); // clean slate
        emit(id, intros, At::BeforeReserve);
        let reservation = reserve_frame_origins().unwrap();
        let reserved = reservation.snapshot()[id];
        emit(id, intros, At::DuringReservation);
        reservation.commit().unwrap();
        emit(id, intros, At::AfterCommit);
        (reserved, drain_frame_origins()[id])
    }

    #[test]
    fn every_publish_position_around_a_reservation_keeps_count_and_evidence_together() {
        // The deterministic guard for "a count is never published without its
        // sample". The concurrent stress test below can only reach the defect
        // by luck — it needs a recorder preempted at one specific instruction
        // AND no later introduction to repair the corrupted word before the
        // next drain, which is why it failed roughly 1 run in 6 on a loaded
        // machine and passed in isolation. Here each recorder step is placed by
        // hand at each of the three phases a reservation defines, so the
        // pairing is re-proved on every run instead of when scheduling obliges.
        //
        // Case 2 is the one that was broken: a sample staged before the
        // reservation whose count publishes during it. The generation-cell
        // design cleared the sample flag over that still-queued introduction
        // and the next drain reported a count with no evidence.
        //
        // What this does NOT cover, so nobody reads more into a green run than
        // it earns: CAS contention between concurrent recorders, the timing
        // between commit's guard loop and its subtraction, ids progressing at
        // different rates, and the source order inside
        // `record_status_origin_sample` itself — that last one is pinned by
        // `sample_value_is_staged_before_its_introduction_publishes`.
        use At::*;
        let _guard = lock_registry();
        let id = SO_D53_POSTBUILD_GATE;

        let sampled = |stage_at, publish, value| Intro {
            stage: Some((stage_at, value)),
            publish,
        };
        let plain = |publish| Intro {
            stage: None,
            publish,
        };

        #[allow(clippy::type_complexity)]
        let cases: &[(&str, Vec<Intro>, (u64, Option<u64>), (u64, Option<u64>))] = &[
            (
                "1: staged and published before the reservation",
                vec![sampled(BeforeReserve, BeforeReserve, 0xA1)],
                (1, Some(0xA1)),
                (0, None),
            ),
            (
                "2: staged before the reservation, published during it",
                vec![sampled(BeforeReserve, DuringReservation, 0xA2)],
                (0, None),
                (1, Some(0xA2)),
            ),
            (
                "3: staged before the reservation, published after commit",
                vec![sampled(BeforeReserve, AfterCommit, 0xA3)],
                (0, None),
                (1, Some(0xA3)),
            ),
            (
                "4: staged and published during the reservation",
                vec![sampled(DuringReservation, DuringReservation, 0xA4)],
                (0, None),
                (1, Some(0xA4)),
            ),
            (
                "5: staged during the reservation, published after commit",
                vec![sampled(DuringReservation, AfterCommit, 0xA5)],
                (0, None),
                (1, Some(0xA5)),
            ),
            (
                "6: staged and published after commit",
                vec![sampled(AfterCommit, AfterCommit, 0xA6)],
                (0, None),
                (1, Some(0xA6)),
            ),
            (
                "7: unsampled introduction before the reservation",
                vec![plain(BeforeReserve)],
                (1, None),
                (0, None),
            ),
            (
                "8: unsampled introduction during the reservation",
                vec![plain(DuringReservation)],
                (0, None),
                (1, None),
            ),
            (
                // The case the retired generation cell existed for: the
                // reserved prefix carried the only sample, so the remaining
                // unsampled introduction must NOT inherit it.
                "9: reserved sample, then an unsampled introduction",
                vec![
                    sampled(BeforeReserve, BeforeReserve, 0xB1),
                    plain(DuringReservation),
                ],
                (1, Some(0xB1)),
                (1, None),
            ),
            (
                "10: reserved unsampled introduction, then a sampled one",
                vec![
                    plain(BeforeReserve),
                    sampled(DuringReservation, DuringReservation, 0xB2),
                ],
                (1, None),
                (1, Some(0xB2)),
            ),
            (
                // Known last-value imprecision, pinned rather than glossed:
                // the second introduction stages its value before the
                // reservation reads it but publishes its count afterwards, so
                // BOTH frames report 0xC2. The invariant under test is that a
                // count always has SOME genuine recorded sample beside it, not
                // that the sample belongs to that exact introduction.
                "11: a later introduction's value staged before the reservation",
                vec![
                    sampled(BeforeReserve, BeforeReserve, 0xC1),
                    sampled(BeforeReserve, DuringReservation, 0xC2),
                ],
                (1, Some(0xC2)),
                (1, Some(0xC2)),
            ),
        ];

        for (label, intros, expect_reserved, expect_next) in cases {
            let (reserved, next) = run_interleaving(id, intros);
            assert_eq!(reserved, *expect_reserved, "{label}: reservation snapshot");
            assert_eq!(next, *expect_next, "{label}: next drain");
            let sampled_intros = intros.iter().filter(|i| i.stage.is_some()).count() as u64;
            assert_eq!(
                reserved.0 + next.0,
                intros.len() as u64,
                "{label}: every introduction must be counted exactly once"
            );
            if sampled_intros != 0 {
                assert!(
                    reserved.1.is_some() || next.1.is_some(),
                    "{label}: a sampled introduction was counted with no sample anywhere"
                );
            }
        }
    }

    #[test]
    fn concurrent_drain_never_sees_count_without_sample() {
        // Sample-publish ordering (R2 gate finding 1 twin): every recording
        // here carries a sample, so a drained (count>0, None) pair means a
        // consumed count whose deciding word was lost.
        //
        // This is a stress test, NOT the proof — reaching the defect depends
        // on scheduling, which is why it failed roughly 1 run in 6 on a loaded
        // machine and passed in isolation. The deterministic proofs are
        // `every_publish_position_around_a_reservation_keeps_count_and_evidence_together`
        // and `sample_value_is_staged_before_its_introduction_publishes`.
        // The drained value is checked against the set the recorder actually
        // wrote; that catches a corrupted or uninitialised sample cell, but it
        // is NOT a reliable detector of a reordered recorder (measured: 0
        // catches in 40 runs) — that order is pinned deterministically above.
        let _guard = lock_registry();
        drain_frame_origins(); // clean slate
        let id = SO_D48_PARTIAL_HARD_OR_GATE;
        let recorder = std::thread::spawn(move || {
            for i in 0..20_000u64 {
                record_status_origin_sample(id, 0x1000 + (i % 5));
            }
        });
        let mut drained_total = 0u64;
        let check = |count: u64, sample: Option<u64>| {
            assert!(
                sample.is_some(),
                "drained counted sample-recordings without their sample"
            );
            let value = sample.unwrap();
            assert!(
                (0x1000..=0x1004).contains(&value),
                "drained sample {value:#x} was never recorded (count {count} published \
                 ahead of its staged value)"
            );
        };
        loop {
            let (count, sample) = drain_frame_origins()[id];
            if count != 0 {
                check(count, sample);
                drained_total += count;
            }
            if recorder.is_finished() {
                break;
            }
        }
        recorder.join().unwrap();
        let (count, sample) = drain_frame_origins()[id];
        if count != 0 {
            check(count, sample);
            drained_total += count;
        }
        assert_eq!(drained_total, 20_000, "no incidence count may be lost");
    }

    #[test]
    fn sample_value_is_staged_before_its_introduction_publishes() {
        // A count that publishes ahead of its value lets a reservation land in
        // between and report the count with the PREVIOUS frame's word — a
        // sample the recorder never wrote for that introduction. That is the
        // same integrity failure as a missing sample, only harder to spot,
        // and no stress test detects it: the two steps are nanoseconds apart,
        // so a racing drain effectively never lands between them (measured:
        // the concurrent stress test caught a reordered recorder 0 times in 40
        // runs). Pin the order directly instead.
        let _guard = lock_registry();
        drain_frame_origins(); // clean slate
        let id = SO_D65_SOURCE_NO_SCALAR;

        // Seed a previous frame's value so a premature publish has something
        // stale to be caught claiming.
        record_status_origin_sample(id, 0xDEAD);
        assert_eq!(drain_frame_origins()[id], (1, Some(0xDEAD)));

        thread_local! {
            static OBSERVED: core::cell::RefCell<Vec<(u64, Option<u64>)>> =
                const { core::cell::RefCell::new(Vec::new()) };
        }
        OBSERVED.with(|seen| seen.borrow_mut().clear());

        fn observer(id: usize) {
            // Reserve from inside the recorder, at the point between its two
            // steps. Correct code has published nothing yet.
            let reservation = reserve_frame_origins().expect("mid-record reservation");
            let seen = reservation.snapshot()[id];
            reservation.rollback().expect("mid-record rollback");
            OBSERVED.with(|slot| slot.borrow_mut().push(seen));
        }

        MID_RECORD_OBSERVER.with(|slot| slot.set(Some(observer as fn(usize))));
        record_status_origin_sample(id, 0xBEEF);
        MID_RECORD_OBSERVER.with(|slot| slot.set(None));

        let observed = OBSERVED.with(|slot| slot.borrow().clone());
        assert_eq!(observed.len(), 1, "observer must run exactly once");
        assert_eq!(
            observed[0],
            (0, None),
            "the introduction must not be visible until AFTER its value is staged; \
             a reservation here saw a count claiming a sample the recorder had not \
             yet written for it"
        );
        // And the completed recording is intact.
        assert_eq!(drain_frame_origins()[id], (1, Some(0xBEEF)));
    }

    #[test]
    fn saturated_count_never_carries_into_the_sample_field() {
        // The count and the sampled-introduction tally share one word. If the
        // count could overflow its field it would increment the sampled tally
        // and fabricate evidence for introductions that never carried any.
        let _guard = lock_registry();
        drain_frame_origins(); // clean slate
        let id = SO_D43_SOURCE_EMPTY_COLD;
        // Park the count one below its field maximum without the 2^32-1
        // increments it would take to get there honestly.
        ORIGIN_PACKED[id].store(COUNT_MASK - 1, Ordering::Release);
        record_status_origin(id); // reaches the maximum
        assert_eq!(packed_sampled(ORIGIN_PACKED[id].load(Ordering::Acquire)), 0);
        record_status_origin(id); // saturates: dropped, not carried
        record_status_origin_sample(id, 0xDEAD); // also dropped
        let packed = ORIGIN_PACKED[id].load(Ordering::Acquire);
        assert_eq!(packed_count(packed), COUNT_MASK, "count must saturate");
        assert_eq!(
            packed_sampled(packed),
            0,
            "a saturating count must never manufacture sample evidence"
        );
        assert_eq!(
            drain_frame_origins()[id],
            (COUNT_MASK, None),
            "no sample may be reported for introductions that carried none"
        );
        assert_eq!(drain_frame_origins()[id], (0, None));
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
            (SO_D65_PARTIAL_WALK_FAILURE, "so_d65_partial_walk_failure"),
            (
                SO_D55_TERMINAL_RECORD_UNPUBLISHABLE,
                "so_d55_terminal_record_unpublishable",
            ),
            (
                SO_D65_PARTIAL_PROFILE_MISMATCH,
                "so_d65_partial_profile_mismatch",
            ),
            (
                SO_D66_PARTIAL_PROFILE_MISMATCH,
                "so_d66_partial_profile_mismatch",
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
