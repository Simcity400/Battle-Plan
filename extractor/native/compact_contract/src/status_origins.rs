//! publishes with Release and a reservation reads with Acquire; everything a

use core::sync::atomic::{AtomicU64, Ordering};

pub const ATTRIBUTION_NO_CLOCK_HOUR: i32 = i32::MIN;

crate::cause_tables::numbered_cause_ids! {
    count = STATUS_ORIGIN_ID_COUNT, names = STATUS_ORIGIN_UNITS,
    unit_prefix = "so_", id_prefix = "so_";

    SO_D45_PARTIAL_TARGETED_TIMED_STRUCTURAL = 0 => d45_partial_targeted_timed_structural,
    SO_D48_PARTIAL_INHERITED = 1 => d48_partial_inherited,
    SO_D48_PARTIAL_ROOTS_ERR = 2 => d48_partial_roots_err,
    SO_D48_PARTIAL_HARD_OR_GATE = 3 => d48_partial_hard_or_gate,
    SO_NAVAL_PARTIAL_INHERITED = 4 => naval_partial_inherited,
    SO_NAVAL_PARTIAL_TORN_WALK = 5 => naval_partial_torn_walk,
    SO_NAVAL_PARTIAL_JOIN = 6 => naval_partial_join,
    SO_NAVAL_SOURCE_EMPTY = 7 => naval_source_empty,
    SO_NAVAL_WORKER_PANIC = 8 => naval_worker_panic,
    SO_D60_SOURCE_UNAVAILABLE = 9 => d60_source_unavailable,
    SO_D60_PARTIAL_INHERITED = 10 => d60_partial_inherited,
    SO_D60_PARTIAL_TORN_ROW = 11 => d60_partial_torn_row,
    SO_D60_WORKER_PANIC = 12 => d60_worker_panic,
    SO_D53_SOURCE_UNAVAILABLE = 13 => d53_source_unavailable,
    SO_D53_PARTIAL_INHERITED = 14 => d53_partial_inherited,
    SO_D53_PARTIAL_ROW_SKIPPED = 15 => d53_partial_row_skipped,
    SO_D53_PARTIAL_SHADOW_MISMATCH = 16 => d53_partial_shadow_mismatch,
    SO_D53_WORKER_PANIC = 17 => d53_worker_panic,
    SO_D54_PARTIAL_INHERITED = 18 => d54_partial_inherited,
    SO_D54_PARTIAL_ARMY_ORDERS = 19 => d54_partial_army_orders,
    SO_D54_PARTIAL_OOB_COMMAND = 20 => d54_partial_oob_command,
    SO_D54_HARD_DEPENDENCY = 21 => d54_hard_dependency,
    SO_D54_PARTIAL_SKIPPED_ROWS = 22 => d54_partial_skipped_rows,
    SO_D65_SOURCE_EMPTY_NO_WITNESS = 23 => d65_source_empty_no_witness,
    SO_D65_PARTIAL_DROPPED_AMBIGUOUS = 24 => d65_partial_dropped_ambiguous,
    SO_D55_TRANSITION_BASELINE_RESET = 25 => d55_transition_baseline_reset,
    SO_FRAME_CONTROL_INVALID = 26 => frame_control_invalid,
    SO_D43_SOURCE_EMPTY_WARM_KNOWN = 27 => d43_source_empty_warm_known,
    SO_D43_SOURCE_EMPTY_COLD = 28 => d43_source_empty_cold,
    SO_D53_POSTBUILD_GATE = 29 => d53_postbuild_gate,
    SO_D48_WORKER_PANIC = 30 => d48_worker_panic,
    SO_D54_SOURCE_DEPENDENCY = 31 => d54_source_dependency,
    SO_D54_WORKER_PANIC = 32 => d54_worker_panic,
    SO_D65_SOURCE_NO_SCALAR = 33 => d65_source_no_scalar,
    SO_D65_SOURCE_VTABLE_RVA = 34 => d65_source_vtable_rva,
    SO_D48_LANE_FAILED_COUNTRY_ROOTS = 35 => d48_lane_failed_country_roots,
    SO_D48_LANE_FAILED_STATE_BUILDING_ROOTS = 36 => d48_lane_failed_state_building_roots,
    SO_D48_LANE_FAILED_STATE_RESOURCE_ROOTS = 37 => d48_lane_failed_state_resource_roots,
    SO_D48_LANE_FAILED_PROVINCE_BUILDING_ROOTS = 38 => d48_lane_failed_province_building_roots,
    SO_D48_LANE_SKIPPED_CONSTRUCTION_ROWS = 39 => d48_lane_skipped_construction_rows,
    SO_D48_LANE_SKIPPED_BUILDING_ROWS = 40 => d48_lane_skipped_building_rows,
    SO_D48_LANE_SKIPPED_RESOURCE_ROWS = 41 => d48_lane_skipped_resource_rows,
    SO_D65_PARTIAL_WALK_FAILURE = 42 => d65_partial_walk_failure,
    SO_D55_TERMINAL_RECORD_UNPUBLISHABLE = 43 => d55_terminal_record_unpublishable,
    SO_D65_PARTIAL_PROFILE_MISMATCH = 44 => d65_partial_profile_mismatch,
    SO_D66_PARTIAL_PROFILE_MISMATCH = 45 => d66_partial_profile_mismatch,
    SO_D48_LANE_DUPLICATE_STATE_ROOTS = 46 => d48_lane_duplicate_state_roots,
    SO_D48_LANE_DUPLICATE_PROVINCE_ROOTS = 47 => d48_lane_duplicate_province_roots,
    SO_D60_WARMUP_GAP = 48 => d60_warmup_gap,
    /// d77/d80 sealed degraded: the selected Present's d80 fold was refused for
    /// lack of sealed scene constant-buffer bytes, so the hour published a named
    /// DOMAIN_GAP instead of a production-render frame.
    SO_D77_D80_SCENE_CB_GAP = 49 => d77_d80_scene_cb_gap,
    /// d77/d80 sealed degraded: the hour observed GUI paints but its selected
    /// Present minted no hourly recorded-input observation, so no authenticated UI
    /// pass could seal; the hour publishes a named DOMAIN_GAP instead of a
    /// production-render frame.
    SO_D77_D80_HOURLY_INPUT_GAP = 50 => d77_d80_hourly_input_gap,
    /// d77/d80 sealed degraded: the selected Present's in-hook foreground pointer
    /// sample completed after its selection cutoff (benign timing race), so no
    /// recorded-input observation could authenticate; the hour publishes a named
    /// DOMAIN_GAP instead of a production-render frame.
    SO_D77_D80_PRESENT_SAMPLE_CUTOFF_GAP = 51 => d77_d80_present_sample_cutoff_gap,
    /// d53 first-attach warmup (lane RS): the shared heap sweep has not completed
    /// its first pass AND this tracker has never consumed a ready frame, so the
    /// frame publishes one named DOMAIN_GAP instead of a failing record. The
    /// sample value is FNV-1a of the static fail-closed reason.
    SO_D53_WARMUP_GAP = 52 => d53_warmup_gap,
    /// d77/d80 sealed degraded: the selected Present binds the tiled point-light
    /// textures whose semantic census is owner-unproven, so no lawful payload
    /// exists; the hour publishes a named DOMAIN_GAP instead of a
    /// production-render frame.
    SO_D77_D80_TILED_LIGHT_CENSUS_GAP = 53 => d77_d80_tiled_light_census_gap,
}

#[allow(clippy::declare_interior_mutable_const)]
const ATOMIC_ZERO: AtomicU64 = AtomicU64::new(0);
static ORIGIN_PACKED: [AtomicU64; STATUS_ORIGIN_ID_COUNT] = [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];
static ORIGIN_SAMPLES: [AtomicU64; STATUS_ORIGIN_ID_COUNT] = [ATOMIC_ZERO; STATUS_ORIGIN_ID_COUNT];
static ACTIVE_FRAME_RESERVATION: AtomicU64 = AtomicU64::new(0);
static NEXT_FRAME_RESERVATION: AtomicU64 = AtomicU64::new(1);

const COUNT_BITS: u32 = 32;
const COUNT_MASK: u64 = (1u64 << COUNT_BITS) - 1;
const SAMPLED_UNIT: u64 = 1u64 << COUNT_BITS;

const fn packed_count(packed: u64) -> u64 {
    packed & COUNT_MASK
}

const fn packed_sampled(packed: u64) -> u64 {
    packed >> COUNT_BITS
}

/// introduction publishes; [`publish_introduction`]'s Release RMW is what
/// makes it visible to an Acquire reservation.
fn stage_sample_value(id: usize, sample: u64) {
    ORIGIN_SAMPLES[id].store(sample, Ordering::Relaxed);
}

#[cfg(test)]
thread_local! {
    static MID_RECORD_OBSERVER: core::cell::Cell<Option<fn(usize)>> =
        const { core::cell::Cell::new(None) };
}

#[cfg(test)]
fn observe_between_staging_and_publish(id: usize) {
    if let Some(hook) = MID_RECORD_OBSERVER.with(|slot| slot.get()) {
        hook(id);
    }
}

#[cfg(not(test))]
#[inline(always)]
fn observe_between_staging_and_publish(_id: usize) {}

fn publish_introduction(id: usize, sampled: bool) {
    let delta = if sampled { SAMPLED_UNIT | 1 } else { 1 };
    let mut current = ORIGIN_PACKED[id].load(Ordering::Relaxed);
    loop {
        if packed_count(current) == COUNT_MASK {
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

pub fn record_status_origin(id: usize) {
    if id >= STATUS_ORIGIN_ID_COUNT {
        return;
    }
    publish_introduction(id, false);
}

/// The sample value is staged BEFORE the packed word publishes (Release); a
/// reservation loads that word with Acquire — so a reservation that counts a
pub fn record_status_origin_sample(id: usize, sample: u64) {
    if id >= STATUS_ORIGIN_ID_COUNT {
        return;
    }
    stage_sample_value(id, sample);
    observe_between_staging_and_publish(id);
    publish_introduction(id, true);
}

#[derive(Debug)]
pub struct StatusOriginFrameReservation {
    reservation_id: u64,
    snapshot: [(u64, Option<u64>); STATUS_ORIGIN_ID_COUNT],
    reserved_packed: [u64; STATUS_ORIGIN_ID_COUNT],
    active: bool,
}

impl StatusOriginFrameReservation {
    pub fn snapshot(&self) -> &[(u64, Option<u64>); STATUS_ORIGIN_ID_COUNT] {
        &self.snapshot
    }

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

    static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

    fn lock_registry() -> MutexGuard<'static, ()> {
        REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn record_and_drain_counts_reset() {
        let _guard = lock_registry();
        drain_frame_origins();
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
        drain_frame_origins();
        let id = SO_D60_SOURCE_UNAVAILABLE;
        record_status_origin_sample(id, 0x100);
        record_status_origin_sample(id, 0x200);
        let drained = drain_frame_origins()[id];
        assert_eq!(drained, (2, Some(0x200)));
        assert_eq!(drain_frame_origins()[id], (0, None));
    }

    #[test]
    fn zero_sample_value_is_still_a_sample() {
        let _guard = lock_registry();
        drain_frame_origins();
        let id = SO_FRAME_CONTROL_INVALID;
        record_status_origin_sample(id, 0);
        assert_eq!(drain_frame_origins()[id], (1, Some(0)));
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum At {
        BeforeReserve,
        DuringReservation,
        AfterCommit,
    }

    #[derive(Clone, Copy, Debug)]
    struct Intro {
        stage: Option<(At, u64)>,
        publish: At,
    }

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
        drain_frame_origins();
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
        let _guard = lock_registry();
        drain_frame_origins();
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
        let _guard = lock_registry();
        drain_frame_origins();
        let id = SO_D65_SOURCE_NO_SCALAR;

        record_status_origin_sample(id, 0xDEAD);
        assert_eq!(drain_frame_origins()[id], (1, Some(0xDEAD)));

        thread_local! {
            static OBSERVED: core::cell::RefCell<Vec<(u64, Option<u64>)>> =
                const { core::cell::RefCell::new(Vec::new()) };
        }
        OBSERVED.with(|seen| seen.borrow_mut().clear());

        fn observer(id: usize) {
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
        assert_eq!(drain_frame_origins()[id], (1, Some(0xBEEF)));
    }

    #[test]
    fn saturated_count_never_carries_into_the_sample_field() {
        let _guard = lock_registry();
        drain_frame_origins();
        let id = SO_D43_SOURCE_EMPTY_COLD;
        ORIGIN_PACKED[id].store(COUNT_MASK - 1, Ordering::Release);
        record_status_origin(id);
        assert_eq!(packed_sampled(ORIGIN_PACKED[id].load(Ordering::Acquire)), 0);
        record_status_origin(id);
        record_status_origin_sample(id, 0xDEAD);
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
            assert!(!name.ends_with("_sample"));
            assert!(seen.insert(name), "duplicate unit name {name}");
        }
    }

}
