//! Regression gate for the withheld delta baseline.
//!
//! A compact frame carrying any failure-class status bit is discarded whole by
//! the recorder fold (`WorldFold::handle_compact_failure`) unless the domain has
//! a `COMPACT_EXPECTED_PARTIAL_TOLERANCES` entry. None of the domains exercised
//! here (29/30/31/32/34/36/38/42) has one, so the collector must NOT advance its
//! delta baseline on such a frame — otherwise the rows the discarded frame
//! carried are marked delivered and never re-sent, and the fold accumulator
//! diverges from the collector's view for the rest of the campaign.
//!
//! Every test in this module fails if the baseline advance is restored.

use super::*;

/// Mirrors `WorldFold`'s keyed upsert/tombstone semantics for one domain:
/// a failure-class frame is dropped whole and the accumulator is retained;
/// otherwise every emitted row is applied (deleted rows remove their key).
#[derive(Default)]
struct FoldMirror<K: Ord + Clone, V: Clone> {
    accumulator: BTreeMap<K, V>,
    dropped_frames: u32,
}

impl<K: Ord + Clone, V: Clone> FoldMirror<K, V> {
    fn new() -> Self {
        Self {
            accumulator: BTreeMap::new(),
            dropped_frames: 0,
        }
    }

    fn apply_frame<R>(
        &mut self,
        status_flags: u32,
        rows: &[R],
        key: impl Fn(&R) -> K,
        deleted: impl Fn(&R) -> bool,
        value: impl Fn(&R) -> V,
    ) {
        // No domain under test has an expected-partial tolerance, so any
        // failure-class bit discards the record before a single row is read.
        assert!(
            compact_contract::expected_partial_tolerance_applies(31, status_flags).is_none(),
            "d31 must stay untolerated for this mirror to model the fold"
        );
        if compact_contract::compact_status_is_failure(status_flags) {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            return;
        }
        for row in rows {
            if deleted(row) {
                self.accumulator.remove(&key(row));
            } else {
                self.accumulator.insert(key(row), value(row));
            }
        }
    }
}

fn air_region_bucket(
    region_id: u32,
    owner_tag_db_id: u32,
    mission_mask: u32,
    plane_count: u32,
) -> CompactAirRegionActivityRow {
    CompactAirRegionActivityRow {
        region_id,
        owner_tag_db_id,
        mission_mask,
        deleted: false,
        wing_count: 1,
        plane_count,
        effective_planes: plane_count,
    }
}

fn superiority_row(variant_id: u32, amount_raw: i64) -> CompactAirSuperiorityInputRow {
    CompactAirSuperiorityInputRow {
        region_id: 88,
        owner_tag_db_id: 1,
        mission_mask: 7,
        variant_id,
        deleted: false,
        equipment_token_raw: 4242,
        equipment_type_token_raw: 11,
        interface_type_raw: 3,
        wing_count: 1,
        amount_raw,
    }
}

// The exact bug reported for d31: an `invalid_air_wing_rows > 0` frame sets
// PARTIAL_DATA, the fold discards the record, and the changes it carried must
// still reach the fold on the next clean frame.
#[test]
fn d31_partial_frame_change_is_redelivered_by_the_next_clean_frame() {
    let mut tracker = CompactAirRegionActivityTracker::default();
    let mut fold = FoldMirror::<(u32, u32, u32), u32>::new();
    let apply = |fold: &mut FoldMirror<(u32, u32, u32), u32>,
                 status: u32,
                 rows: &[CompactAirRegionActivityRow]| {
        fold.apply_frame(
            status,
            rows,
            |row| (row.region_id, row.owner_tag_db_id, row.mission_mask),
            |row| row.deleted,
            |row| row.plane_count,
        );
    };

    // Hour 1: clean baseline.
    let (rows, ..) =
        compact_air_region_activity_delta_rows(&mut tracker, vec![air_region_bucket(88, 1, 7, 100)], 0);
    apply(&mut fold, 0, &rows);
    assert_eq!(fold.accumulator[&(88, 1, 7)], 100);

    // Hour 2: one air wing failed to read -> PARTIAL_DATA. The bucket moved to
    // 140 planes. The frame is discarded by the fold.
    let partial_status = COMPACT_STATUS_PARTIAL_DATA;
    let (rows, changed_rows, deleted_rows) = compact_air_region_activity_delta_rows(
        &mut tracker,
        vec![air_region_bucket(88, 1, 7, 140)],
        partial_status,
    );
    assert!(rows.is_empty(), "a withheld frame emits no rows");
    assert_eq!((changed_rows, deleted_rows), (0, 0));
    apply(&mut fold, partial_status, &rows);
    assert_eq!(fold.dropped_frames, 1);

    // Hour 3: clean read of the SAME 140 planes. With the baseline advanced by
    // hour 2 this emits nothing and the fold is stuck at 100 forever.
    let (rows, changed_rows, _) =
        compact_air_region_activity_delta_rows(&mut tracker, vec![air_region_bucket(88, 1, 7, 140)], 0);
    assert_eq!(changed_rows, 1, "the withheld change must be re-emitted");
    apply(&mut fold, 0, &rows);
    assert_eq!(
        fold.accumulator[&(88, 1, 7)],
        140,
        "fold must end stored-equal to the collector view"
    );
    assert_eq!(tracker.previous[&(88, 1, 7)].plane_count, 140);
}

// A withheld FIRST frame must not seed a baseline: the following clean frame
// still owes the fold a complete snapshot, not a delta against a set the fold
// never received.
#[test]
fn d31_withheld_first_frame_does_not_seed_a_baseline() {
    let mut tracker = CompactAirRegionActivityTracker::default();
    let (rows, ..) = compact_air_region_activity_delta_rows(
        &mut tracker,
        vec![air_region_bucket(88, 1, 7, 100), air_region_bucket(89, 1, 8, 50)],
        COMPACT_STATUS_PARTIAL_DATA,
    );
    assert!(rows.is_empty());
    assert!(
        tracker.previous.is_empty(),
        "a discarded frame must not become the delta baseline"
    );

    let (rows, changed_rows, deleted_rows) = compact_air_region_activity_delta_rows(
        &mut tracker,
        vec![air_region_bucket(88, 1, 7, 100), air_region_bucket(89, 1, 8, 50)],
        0,
    );
    assert_eq!(changed_rows, 2, "the next clean frame is a full snapshot");
    assert_eq!(deleted_rows, 0);
    assert_eq!(rows.len(), 2);
}

// An air wing that failed to read makes its bucket disappear from the sample.
// That is an unread bucket, never a deleted one: a withheld frame must not
// tombstone it (and must not lose it either).
#[test]
fn d31_withheld_frame_never_tombstones_an_unread_bucket() {
    let mut tracker = CompactAirRegionActivityTracker::default();
    let _ = compact_air_region_activity_delta_rows(
        &mut tracker,
        vec![air_region_bucket(88, 1, 7, 100), air_region_bucket(89, 1, 8, 50)],
        0,
    );

    // Region 89's only wing was unreadable this frame.
    let (rows, changed_rows, deleted_rows) = compact_air_region_activity_delta_rows(
        &mut tracker,
        vec![air_region_bucket(88, 1, 7, 100)],
        COMPACT_STATUS_PARTIAL_DATA,
    );
    assert!(rows.iter().all(|row| !row.deleted));
    assert_eq!((changed_rows, deleted_rows), (0, 0));
    assert!(
        tracker.previous.contains_key(&(89, 1, 8)),
        "an unread bucket stays in the baseline so a real deletion is still detectable"
    );

    // A genuine disappearance on a clean frame still tombstones.
    let (rows, _, deleted_rows) = compact_air_region_activity_delta_rows(
        &mut tracker,
        vec![air_region_bucket(88, 1, 7, 100)],
        0,
    );
    assert_eq!(deleted_rows, 1);
    assert!(rows.iter().any(|row| row.deleted && row.region_id == 89));
}

// d38 is the sibling bucket lane: same aggregate-over-wings shape, same fold
// discard, same defect.
#[test]
fn d38_partial_frame_change_is_redelivered_by_the_next_clean_frame() {
    let mut tracker = CompactAirSuperiorityInputTracker::default();
    let mut fold = FoldMirror::<(u32, u32, u32, u32), i64>::new();
    let apply = |fold: &mut FoldMirror<(u32, u32, u32, u32), i64>,
                 status: u32,
                 rows: &[CompactAirSuperiorityInputRow]| {
        fold.apply_frame(
            status,
            rows,
            |row| (row.region_id, row.owner_tag_db_id, row.mission_mask, row.variant_id),
            |row| row.deleted,
            |row| row.amount_raw,
        );
    };

    let (rows, ..) =
        compact_air_superiority_input_delta_rows(&mut tracker, vec![superiority_row(900, 100_000)], 0);
    apply(&mut fold, 0, &rows);
    assert_eq!(fold.accumulator[&(88, 1, 7, 900)], 100_000);

    // An unresolvable variant sets PARTIAL_DATA on a frame whose amount moved.
    let partial_status = COMPACT_STATUS_PARTIAL_DATA;
    let (rows, changed_rows, _) = compact_air_superiority_input_delta_rows(
        &mut tracker,
        vec![superiority_row(900, 140_000)],
        partial_status,
    );
    assert!(rows.is_empty());
    assert_eq!(changed_rows, 0);
    apply(&mut fold, partial_status, &rows);
    assert_eq!(fold.dropped_frames, 1);

    let (rows, changed_rows, _) =
        compact_air_superiority_input_delta_rows(&mut tracker, vec![superiority_row(900, 140_000)], 0);
    assert_eq!(changed_rows, 1);
    apply(&mut fold, 0, &rows);
    assert_eq!(fold.accumulator[&(88, 1, 7, 900)], 140_000);
}

// d42 rows are INCREMENTS (loss/reinforcement deltas), so a discarded frame
// whose baseline advanced drops those increments permanently. After a withheld
// frame the next clean frame must carry the whole interval's movement.
#[test]
fn d42_withheld_frame_defers_the_full_count_delta_to_the_next_clean_frame() {
    fn sample(count: u32) -> CompactAirStrengthSample {
        CompactAirStrengthSample {
            owner_tag_db_id: 1,
            wing_id: 4,
            generation: 0,
            body_addr: HEAP_LO + 0x100,
            count,
            mission_mask: 1,
            mission_region: Some(154),
            equipment_amount_sum: (count as i64) * COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE,
            air_accidents: 0,
            experience_raw: 0,
        }
    }

    let mut tracker = CompactAirStrengthDeltaTracker::default();
    let (rows, ..) = compact_air_strength_delta_rows(&mut tracker, vec![sample(100)], 0);
    assert_eq!(rows.len(), 1);
    let generation_after_seed = tracker.next_generation;

    // 100 -> 90 on a discarded frame: the ten losses must not be consumed.
    let (rows, changed_rows, loss_rows, ..) = compact_air_strength_delta_rows(
        &mut tracker,
        vec![sample(90)],
        COMPACT_STATUS_PARTIAL_DATA,
    );
    assert!(rows.is_empty());
    assert_eq!((changed_rows, loss_rows), (0, 0));
    assert_eq!(
        tracker.next_generation, generation_after_seed,
        "a withheld frame must not burn generations"
    );

    // Clean frame at 85: the delta must cover the whole 100 -> 85 interval.
    let (rows, changed_rows, loss_rows, ..) =
        compact_air_strength_delta_rows(&mut tracker, vec![sample(85)], 0);
    assert_eq!((changed_rows, loss_rows), (1, 1));
    assert_eq!(rows[0].count_delta, -15);
    assert_eq!(rows[0].old_count, 100);
    assert_eq!(rows[0].new_count, 85);
}

// A scale mismatch is only observable mid-diff, so `compact_air_strength_delta_rows`
// discovers after the fact that its caller will flag the frame PARTIAL. It must
// withhold the baseline just the same, or the movement it saw is lost: an
// unchanged next sample produces no delta at all.
#[test]
fn d42_scale_mismatch_frame_withholds_and_the_movement_survives_an_unchanged_frame() {
    fn sample(wing_id: u32, count: u32, equipment_amount_sum: i64) -> CompactAirStrengthSample {
        CompactAirStrengthSample {
            owner_tag_db_id: 1,
            wing_id,
            generation: 0,
            body_addr: HEAP_LO + 0x100 * (wing_id as usize),
            count,
            mission_mask: 1,
            mission_region: Some(154),
            equipment_amount_sum,
            air_accidents: 0,
            experience_raw: 0,
        }
    }
    let scaled = |count: u32| (count as i64) * COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE;

    let mut tracker = CompactAirStrengthDeltaTracker::default();
    let _ = compact_air_strength_delta_rows(&mut tracker, vec![sample(4, 100, scaled(100))], 0);
    let generation_after_seed = tracker.next_generation;

    // Wing 4 goes 100 -> 99 planes but its equipment sum moves off the
    // whole-plane scale, and wing 5 appears for the first time on the same
    // frame. The mismatch withholds the whole frame, so wing 5's generation
    // must be rolled back with it: the fold never saw that generation.
    let (rows, changed_rows, _, _, _, scale_mismatch_rows) = compact_air_strength_delta_rows(
        &mut tracker,
        vec![sample(4, 99, scaled(99) + 1), sample(5, 20, scaled(20))],
        0,
    );
    assert_eq!(scale_mismatch_rows, 1, "the caller flags this frame PARTIAL");
    assert!(rows.is_empty(), "a mismatch frame is withheld, not published");
    assert_eq!(changed_rows, 0);
    assert_eq!(tracker.previous[&(HEAP_LO + 0x400)].count, 100);
    assert!(!tracker.previous.contains_key(&(HEAP_LO + 0x500)));
    assert_eq!(
        tracker.next_generation, generation_after_seed,
        "a withheld frame must not burn the generation it handed the new wing"
    );

    // The very next samples are UNCHANGED from the withheld frame. Against an
    // advanced baseline this would produce no delta at all and both the lost
    // plane and wing 5's baseline row would never reach the fold; against the
    // retained baseline they are still pending and are emitted as soon as they
    // are representable.
    let (rows, changed_rows, loss_rows, _, _, scale_mismatch_rows) =
        compact_air_strength_delta_rows(
            &mut tracker,
            vec![sample(4, 99, scaled(99)), sample(5, 20, scaled(20))],
            0,
        );
    assert_eq!(scale_mismatch_rows, 0);
    assert_eq!((changed_rows, loss_rows), (2, 1));
    let loss = rows.iter().find(|row| row.wing_id == 4).unwrap();
    assert_eq!((loss.old_count, loss.new_count), (100, 99));
    let seeded = rows.iter().find(|row| row.wing_id == 5).unwrap();
    assert!(seeded.baseline, "wing 5 still owes the fold its baseline row");
    assert_eq!(
        seeded.generation, generation_after_seed,
        "the rolled-back generation is reissued to the same wing"
    );
}

// The remaining lanes in the class share the identical `*_delta_rows` shape.
#[test]
fn sibling_lane_withheld_frames_keep_their_baselines_and_emit_nothing() {
    let ship = CompactShipRow {
        instance_id: 300,
        deleted: false,
        task_force_id: Some(201),
        owner_tag_db_id: Some(1),
        definition: Some("destroyer_1".into()),
        category_enum: 1,
        strength_raw: 1_000_000,
        organisation_raw: 1_000_000,
        experience_raw: 0,
        sunk_convoys: 0,
    };
    let mut ship_tracker = CompactShipTracker::default();
    let (rows, changed_rows, deleted_rows) =
        compact_ship_delta_rows(&mut ship_tracker, vec![ship.clone()], COMPACT_STATUS_PARTIAL_DATA);
    assert!(rows.is_empty() && changed_rows == 0 && deleted_rows == 0);
    assert!(ship_tracker.previous.is_empty(), "d29 baseline must not advance");
    let (rows, ..) = compact_ship_delta_rows(&mut ship_tracker, vec![ship], 0);
    assert_eq!(rows.len(), 1, "d29 re-delivers on the next clean frame");

    let wing = CompactAirWingRow {
        owner_tag_db_id: 1,
        wing_id: 700,
        generation: 0,
        body_addr: HEAP_LO + 0x100,
        deleted: false,
        count: 77,
        mission_mask: 1,
        mission_region: Some(88),
        effective_planes: 70,
    };
    let mut wing_tracker = CompactAirWingTracker::default();
    let (rows, ..) = compact_air_wing_delta_rows(&mut wing_tracker, vec![wing.clone()], COMPACT_STATUS_PARTIAL_DATA);
    assert!(rows.is_empty());
    assert!(wing_tracker.previous.is_empty(), "d30 baseline must not advance");
    assert_eq!(
        wing_tracker.next_generation, 0,
        "d30 must not burn a generation on a discarded frame"
    );
    let (rows, ..) = compact_air_wing_delta_rows(&mut wing_tracker, vec![wing], 0);
    assert_eq!(rows.len(), 1, "d30 re-delivers on the next clean frame");

    let loadout = CompactAirWingLoadoutRow {
        owner_tag_db_id: 1,
        wing_id: 700,
        generation: 0,
        body_addr: HEAP_LO + 0x100,
        deleted: false,
        air_base_id: 9001,
        equipment: vec![CompactAirWingLoadoutEntry {
            variant_id: 900,
            amount_raw: 100_000,
        }],
    };
    let mut loadout_tracker = CompactAirWingLoadoutTracker::default();
    let (rows, changed_rows, deleted_rows, emitted_equipment_rows) =
        compact_air_wing_loadout_delta_rows(&mut loadout_tracker, vec![loadout.clone()], COMPACT_STATUS_PARTIAL_DATA);
    assert!(rows.is_empty());
    assert_eq!((changed_rows, deleted_rows, emitted_equipment_rows), (0, 0, 0));
    assert!(
        loadout_tracker.previous.is_empty(),
        "d32 baseline must not advance"
    );
    let (rows, ..) = compact_air_wing_loadout_delta_rows(&mut loadout_tracker, vec![loadout], 0);
    assert_eq!(rows.len(), 1, "d32 re-delivers on the next clean frame");

    let air_base = CompactAirBaseStatusRow {
        air_base_id: 9001,
        state_id: 2,
        province_id: 321,
        deleted: false,
        capacity: 1600,
        building_level: 8,
        based_wing_count: 3,
        based_plane_count: 278,
    };
    let mut air_base_tracker = CompactAirBaseStatusTracker::default();
    let (rows, ..) =
        compact_air_base_status_delta_rows(&mut air_base_tracker, vec![air_base.clone()], COMPACT_STATUS_PARTIAL_DATA);
    assert!(rows.is_empty());
    assert!(
        air_base_tracker.previous.is_empty(),
        "d34 baseline must not advance"
    );
    let (rows, ..) = compact_air_base_status_delta_rows(&mut air_base_tracker, vec![air_base], 0);
    assert_eq!(rows.len(), 1, "d34 re-delivers on the next clean frame");

    let reserve = CompactAirVariantReserveRow {
        tag_db_id: 1,
        variant_id: 900,
        deleted: false,
        amount_raw: 250_000,
    };
    let mut reserve_tracker = CompactAirVariantReserveTracker::default();
    let (rows, ..) =
        compact_air_variant_reserve_delta_rows(&mut reserve_tracker, vec![reserve], COMPACT_STATUS_PARTIAL_DATA);
    assert!(rows.is_empty());
    assert!(
        reserve_tracker.previous.is_empty(),
        "d36 baseline must not advance"
    );
    let (rows, ..) =
        compact_air_variant_reserve_delta_rows(&mut reserve_tracker, vec![reserve], 0);
    assert_eq!(rows.len(), 1, "d36 re-delivers on the next clean frame");
}

// The predicate must cover the whole failure vocabulary, not just PARTIAL_DATA:
// a frame-scoped CONTROL_INVALID / NO_CLOCK / PROFILE_MISMATCH is fanned out to
// every domain read and is discarded by the fold exactly the same way.
#[test]
fn withheld_predicate_covers_every_failure_class_bit() {
    for bit in 0..32u32 {
        let flag = 1u32 << bit;
        let is_failure = compact_contract::COMPACT_STATUS_FAILURE_MASK & flag != 0;
        assert_eq!(
            compact_delta_baseline_withheld(flag),
            is_failure,
            "bit {bit:#x} classification must follow COMPACT_STATUS_FAILURE_MASK"
        );
    }
    assert!(!compact_delta_baseline_withheld(0));
    assert!(!compact_delta_baseline_withheld(COMPACT_STATUS_DELTA_DATA));
    assert!(!compact_delta_baseline_withheld(
        COMPACT_STATUS_EXTRACTION_PERF_GATE
    ));
    assert!(compact_delta_baseline_withheld(
        COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA
    ));
}

// Guard the assumption the whole fix rests on: none of these domains may gain
// an expected-partial tolerance without revisiting the collector, because their
// rows are aggregates whose values (not just their presence) are corrupted by
// an omitted source row.
#[test]
fn class_domains_have_no_expected_partial_tolerance() {
    for domain_id in [29u16, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 42, 46] {
        assert!(
            compact_contract::expected_partial_tolerance(domain_id).is_none(),
            "d{domain_id} gained an expected-partial tolerance; the collector's \
             withheld-baseline contract must be re-derived for it"
        );
    }
}
