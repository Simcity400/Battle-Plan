//! publishes with Release and a reservation reads with Acquire; everything a

use core::sync::atomic::{AtomicU64, Ordering};

pub const ATTRIBUTION_NO_CLOCK_HOUR: i32 = i32::MIN;

pub const SO_D45_PARTIAL_TARGETED_TIMED_STRUCTURAL: usize = 0;
pub const SO_D48_PARTIAL_INHERITED: usize = 1;
pub const SO_D48_PARTIAL_ROOTS_ERR: usize = 2;
pub const SO_D48_PARTIAL_HARD_OR_GATE: usize = 3;
pub const SO_NAVAL_PARTIAL_INHERITED: usize = 4;
pub const SO_NAVAL_PARTIAL_TORN_WALK: usize = 5;
pub const SO_NAVAL_PARTIAL_JOIN: usize = 6;
pub const SO_NAVAL_SOURCE_EMPTY: usize = 7;
pub const SO_NAVAL_WORKER_PANIC: usize = 8;
pub const SO_D60_SOURCE_UNAVAILABLE: usize = 9;
pub const SO_D60_PARTIAL_INHERITED: usize = 10;
pub const SO_D60_PARTIAL_TORN_ROW: usize = 11;
pub const SO_D60_WORKER_PANIC: usize = 12;
pub const SO_D53_SOURCE_UNAVAILABLE: usize = 13;
pub const SO_D53_PARTIAL_INHERITED: usize = 14;
pub const SO_D53_PARTIAL_ROW_SKIPPED: usize = 15;
pub const SO_D53_PARTIAL_SHADOW_MISMATCH: usize = 16;
pub const SO_D53_WORKER_PANIC: usize = 17;
pub const SO_D54_PARTIAL_INHERITED: usize = 18;
pub const SO_D54_PARTIAL_ARMY_ORDERS: usize = 19;
pub const SO_D54_PARTIAL_OOB_COMMAND: usize = 20;
pub const SO_D54_HARD_DEPENDENCY: usize = 21;
pub const SO_D54_PARTIAL_SKIPPED_ROWS: usize = 22;
pub const SO_D65_SOURCE_EMPTY_NO_WITNESS: usize = 23;
pub const SO_D65_PARTIAL_DROPPED_AMBIGUOUS: usize = 24;
pub const SO_D55_TRANSITION_BASELINE_RESET: usize = 25;
pub const SO_FRAME_CONTROL_INVALID: usize = 26;
pub const SO_D43_SOURCE_EMPTY_WARM_KNOWN: usize = 27;
pub const SO_D43_SOURCE_EMPTY_COLD: usize = 28;
pub const SO_D53_POSTBUILD_GATE: usize = 29;
pub const SO_D48_WORKER_PANIC: usize = 30;
pub const SO_D54_SOURCE_DEPENDENCY: usize = 31;
pub const SO_D54_WORKER_PANIC: usize = 32;
pub const SO_D65_SOURCE_NO_SCALAR: usize = 33;
pub const SO_D65_SOURCE_VTABLE_RVA: usize = 34;
pub const SO_D48_LANE_FAILED_COUNTRY_ROOTS: usize = 35;
pub const SO_D48_LANE_FAILED_STATE_BUILDING_ROOTS: usize = 36;
pub const SO_D48_LANE_FAILED_STATE_RESOURCE_ROOTS: usize = 37;
pub const SO_D48_LANE_FAILED_PROVINCE_BUILDING_ROOTS: usize = 38;
pub const SO_D48_LANE_SKIPPED_CONSTRUCTION_ROWS: usize = 39;
pub const SO_D48_LANE_SKIPPED_BUILDING_ROWS: usize = 40;
pub const SO_D48_LANE_SKIPPED_RESOURCE_ROWS: usize = 41;
pub const SO_D65_PARTIAL_WALK_FAILURE: usize = 42;
pub const SO_D55_TERMINAL_RECORD_UNPUBLISHABLE: usize = 43;
pub const SO_D65_PARTIAL_PROFILE_MISMATCH: usize = 44;
pub const SO_D66_PARTIAL_PROFILE_MISMATCH: usize = 45;
pub const STATUS_ORIGIN_ID_COUNT: usize = 46;

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

    #[test]
    fn every_id_constant_indexes_its_exact_unit_name() {
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
        let mut ids: Vec<usize> = expected.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), STATUS_ORIGIN_ID_COUNT);
        assert_eq!(ids.first(), Some(&0));
        assert_eq!(ids.last(), Some(&(STATUS_ORIGIN_ID_COUNT - 1)));
    }
}
