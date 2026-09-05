use super::*;

#[test]
fn domain48_lane_partial_causes_match_partial_predicate_per_cause() {
    use compact_contract::status_origins as so;
    let clean = domain48::collector_module::CollectorStats::default();
    assert!(domain48_lane_partial_causes(&clean)
        .iter()
        .all(|(_, count)| *count == 0));
    // The two duplicate-root causes joined this list with the D48V census: a
    // repeated root id is attempted ONCE (a second attempt appends identical
    // rows, which the gate rejects as duplicate keys), and the duplicate marks
    // the frame PARTIAL and clears the matching authority flag. They sit where
    // `domain48_lane_partial_causes` puts them, and this array is positional.
    let expected_ids = [
        so::SO_D48_LANE_FAILED_COUNTRY_ROOTS,
        so::SO_D48_LANE_DUPLICATE_STATE_ROOTS,
        so::SO_D48_LANE_DUPLICATE_PROVINCE_ROOTS,
        so::SO_D48_LANE_FAILED_STATE_BUILDING_ROOTS,
        so::SO_D48_LANE_FAILED_STATE_RESOURCE_ROOTS,
        so::SO_D48_LANE_FAILED_PROVINCE_BUILDING_ROOTS,
        so::SO_D48_LANE_SKIPPED_CONSTRUCTION_ROWS,
        so::SO_D48_LANE_SKIPPED_BUILDING_ROWS,
        so::SO_D48_LANE_SKIPPED_RESOURCE_ROWS,
    ];
    for (slot, build) in [
        ((|s: &mut domain48::collector_module::CollectorStats| s.failed_country_roots = 7)
            as fn(&mut domain48::collector_module::CollectorStats)),
        |s| s.duplicate_state_roots = 7,
        |s| s.duplicate_province_roots = 7,
        |s| s.failed_state_building_roots = 7,
        |s| s.failed_state_resource_roots = 7,
        |s| s.failed_province_building_roots = 7,
        |s| s.skipped_construction_rows = 7,
        |s| s.skipped_building_rows = 7,
        |s| s.skipped_resource_rows = 7,
    ]
    .into_iter()
    .enumerate()
    {
        let mut stats = domain48::collector_module::CollectorStats::default();
        build(&mut stats);
        let pairs = domain48_lane_partial_causes(&stats);
        assert_eq!(pairs[slot], (expected_ids[slot], 7), "slot {slot}");
        assert!(
            pairs
                .iter()
                .enumerate()
                .all(|(i, (_, c))| i == slot || *c == 0),
            "cause {slot} must not bleed into other ids"
        );
    }
    let mut frame_a = domain48::collector_module::CollectorStats::default();
    frame_a.failed_country_roots = 1;
    let mut frame_b = domain48::collector_module::CollectorStats::default();
    frame_b.skipped_resource_rows = 1;
    let recorded_a: Vec<usize> = domain48_lane_partial_causes(&frame_a)
        .iter()
        .filter(|(_, c)| *c != 0)
        .map(|(id, _)| *id)
        .collect();
    let recorded_b: Vec<usize> = domain48_lane_partial_causes(&frame_b)
        .iter()
        .filter(|(_, c)| *c != 0)
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(recorded_a, vec![so::SO_D48_LANE_FAILED_COUNTRY_ROOTS]);
    assert_eq!(recorded_b, vec![so::SO_D48_LANE_SKIPPED_RESOURCE_ROWS]);
    assert_ne!(recorded_a, recorded_b, "distinct ids preserve both causes");
}

#[test]
fn domain48_diagnostics_preserve_exact_first_invalid_progress_row() {
    let stats = domain48::collector_module::CollectorStats {
        skipped_construction_rows: 1,
        skipped_construction_invalid_progress_rows: 1,
        first_invalid_construction_progress: Some(domain48::InvalidConstructionProgress {
            line_address: 0x1000_0000_2000,
            tag_db_id: 123,
            queue_index: 7,
            line_type: domain48::LINE_TYPE_CONSTRUCTION,
            line_id: 77,
            cost_raw: -73_728,
            produced_raw: 5_000_000,
        }),
        ..Default::default()
    };

    let units = domain48_diagnostic_units(&stats, &[]);
    let value = |name| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    assert_eq!(
        value("domain48_first_invalid_progress_line_address"),
        Some(0x1000_0000_2000)
    );
    assert_eq!(
        value("domain48_first_invalid_progress_tag_db_id"),
        Some(123)
    );
    assert_eq!(
        value("domain48_first_invalid_progress_queue_index"),
        Some(7)
    );
    assert_eq!(
        value("domain48_first_invalid_progress_line_type"),
        Some(u64::from(domain48::LINE_TYPE_CONSTRUCTION))
    );
    assert_eq!(value("domain48_first_invalid_progress_line_id"), Some(77));
    assert_eq!(
        value("domain48_first_invalid_progress_cost_raw_i32_bits"),
        Some(u64::from((-73_728i32) as u32))
    );
    assert_eq!(
        value("domain48_first_invalid_progress_produced_raw_i32_bits"),
        Some(5_000_000)
    );
}

fn source_flags() -> u32 {
    domain48::SOURCE_FLAG_DLL_NATIVE
        | domain48::SOURCE_FLAG_SHARED_COUNTRY_ROOTS
        | domain48::SOURCE_FLAG_SHARED_MAP_TOPOLOGY
        | domain48::SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
}

fn cq(line_id: u32, produced_raw: i32) -> domain48::ConstructionQueueRow {
    domain48::ConstructionQueueRow {
        tag_db_id: 1,
        line_id,
        template_id: Some(20_013),
        active_factories: 5,
        cost_raw: 10_000_000,
        produced_raw,
        priority: 0,
        target_state_id: Some(42),
        target_location_id: Some(42),
        target_scope: Some(domain48::SCOPE_STATE),
        repair_levels_remaining: Some(0),
        created_date_raw: Some(60_827_043 + line_id),
    }
}

fn bld(type_id: u32, level: u32) -> domain48::BuildingRow {
    domain48::BuildingRow {
        scope: domain48::SCOPE_STATE,
        location_id: 42,
        state_id: 42,
        type_id,
        level,
        healthy_level: Some(level),
        partial_health_raw: Some(0),
    }
}

fn res(resource_id: u32, amount_raw: i32) -> domain48::StateResourceRow {
    domain48::StateResourceRow {
        state_id: 42,
        resource_id,
        amount_raw,
    }
}

fn make_read(
    status_flags: u32,
    construction_rows: Vec<domain48::ConstructionQueueRow>,
    building_rows: Vec<domain48::BuildingRow>,
    resource_rows: Vec<domain48::StateResourceRow>,
) -> CompactConstructionBuildingsStateDamageRead {
    let frame = domain48::LaneFrame {
        game_hour_raw: 60_759_360,
        status_flags,
        source_flags: source_flags(),
        observed_country_roots: 1,
        observed_state_roots: 1,
        observed_province_roots: 1,
        construction_rows,
        railway_construction_rows: None,
        building_rows,
        resource_rows,
        coverage: domain48::CoverageCensus {
            census_flags: domain48::CENSUS_FLAG_STATE_ROOTS_AUTHORITATIVE
                | domain48::CENSUS_FLAG_PROVINCE_FAILURES_AUTHORITATIVE,
            scanned_states: vec![42],
            ..Default::default()
        },
    };
    let emitted_rows = domain48_emitted_rows(&frame);
    CompactConstructionBuildingsStateDamageRead {
        frame,
        read_time_us: 0,
        status_flags,
        observed_root_rows: 3,
        emitted_rows,
        diagnostic_units: Vec::new(),
    }
}

fn apply(
    read: CompactConstructionBuildingsStateDamageRead,
    tracker: &mut CompactDomain48Tracker,
) -> CompactConstructionBuildingsStateDamageRead {
    compact_domain48_apply_emit_mode(read, D48EmitMode::Delta, tracker)
}

fn clean(
    construction_rows: Vec<domain48::ConstructionQueueRow>,
    building_rows: Vec<domain48::BuildingRow>,
    resource_rows: Vec<domain48::StateResourceRow>,
) -> CompactConstructionBuildingsStateDamageRead {
    make_read(0, construction_rows, building_rows, resource_rows)
}

fn partial(
    construction_rows: Vec<domain48::ConstructionQueueRow>,
    building_rows: Vec<domain48::BuildingRow>,
    resource_rows: Vec<domain48::StateResourceRow>,
) -> CompactConstructionBuildingsStateDamageRead {
    make_read(
        COMPACT_STATUS_PARTIAL_DATA,
        construction_rows,
        building_rows,
        resource_rows,
    )
}

#[test]
fn partial_reduces_churn_to_changed_subset_not_full_reemit() {
    let mut tracker = CompactDomain48Tracker::default();

    let base = apply(
        clean(
            vec![cq(77, 5_000_000)],
            vec![bld(0x2fab, 5)],
            vec![res(0x3e03, 2_500_000)],
        ),
        &mut tracker,
    );
    assert_eq!(base.emitted_rows, 3, "baseline emits the full row set");

    let identical = partial(
        vec![cq(77, 5_000_000)],
        vec![bld(0x2fab, 5)],
        vec![res(0x3e03, 2_500_000)],
    );
    let legacy_full_reemit = domain48_emitted_rows(&identical.frame);
    assert_eq!(
        legacy_full_reemit, 3,
        "legacy behavior re-emits the whole partial read"
    );
    let reduced = apply(identical, &mut tracker);
    assert_eq!(
        reduced.emitted_rows, 0,
        "with the fix an unchanged partial emits nothing (churn eliminated)"
    );
    assert!(reduced.frame.construction_rows.is_empty());
    assert!(reduced.frame.building_rows.is_empty());
    assert!(reduced.frame.resource_rows.is_empty());
    assert_eq!(
        reduced.status_flags & COMPACT_STATUS_PARTIAL_DATA,
        COMPACT_STATUS_PARTIAL_DATA,
        "reduced frame stays an expected-partial frame"
    );
    assert_eq!(
        reduced.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "a partial frame is never promoted to a DELTA frame"
    );

    let one_change = partial(
        vec![cq(77, 5_000_000)],
        vec![bld(0x2fab, 6)],
        vec![res(0x3e03, 2_500_000)],
    );
    assert_eq!(domain48_emitted_rows(&one_change.frame), 3);
    let reduced = apply(one_change, &mut tracker);
    assert_eq!(
        reduced.emitted_rows, 1,
        "only the genuinely-changed row is emitted"
    );
    assert_eq!(reduced.frame.building_rows.len(), 1);
    assert_eq!(reduced.frame.building_rows[0].level, 6);
}

#[test]
fn partial_never_advances_delta_baseline() {
    let mut tracker = CompactDomain48Tracker::default();
    apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    let key = domain48_construction_key(&cq(77, 0));
    assert_eq!(tracker.baseline.last_construction[&key].produced_raw, 5_000_000);

    apply(
        partial(vec![cq(77, 9_000_000)], vec![], vec![]),
        &mut tracker,
    );
    assert_eq!(
        tracker.baseline.last_construction[&key].produced_raw, 5_000_000,
        "partial frame must not advance the delta baseline"
    );

    let d = apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    assert_eq!(
        d.status_flags & COMPACT_STATUS_DELTA_DATA,
        COMPACT_STATUS_DELTA_DATA
    );
    assert!(
        d.frame.construction_rows.is_empty(),
        "no churn: the reverted value equals the pinned baseline"
    );
}

#[test]
fn partial_corrects_value_reverting_to_baseline_via_projection_diff() {
    let mut tracker = CompactDomain48Tracker::default();
    apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);

    let p1 = apply(
        partial(vec![cq(77, 9_000_000)], vec![], vec![]),
        &mut tracker,
    );
    assert_eq!(p1.emitted_rows, 1);
    assert_eq!(p1.frame.construction_rows[0].produced_raw, 9_000_000);

    let p2 = apply(
        partial(vec![cq(77, 5_000_000)], vec![], vec![]),
        &mut tracker,
    );
    assert_eq!(
        p2.emitted_rows, 1,
        "reverted-to-baseline value must be re-emitted to correct the projection"
    );
    assert_eq!(p2.frame.construction_rows[0].produced_raw, 5_000_000);
}

#[test]
fn partial_never_emits_tombstone_for_omitted_row() {
    let mut tracker = CompactDomain48Tracker::default();
    apply(
        clean(vec![cq(77, 5_000_000), cq(88, 2_000_000)], vec![], vec![]),
        &mut tracker,
    );

    let p = apply(
        partial(vec![cq(77, 5_000_000)], vec![], vec![]),
        &mut tracker,
    );
    assert!(
        p.frame
            .construction_rows
            .iter()
            .all(|row| domain48::construction_row_deleted_id(row).is_none()),
        "partial frame must not emit tombstones"
    );
    assert!(
        !p.frame
            .construction_rows
            .iter()
            .any(|row| { (row.line_id & !domain48::ROW_ID_DELETED) == 88 }),
        "omitted line 88 must not appear at all"
    );
}

#[test]
fn clean_delta_frame_still_tombstones_removed_row() {
    let mut tracker = CompactDomain48Tracker::default();
    apply(
        clean(vec![cq(77, 5_000_000), cq(88, 2_000_000)], vec![], vec![]),
        &mut tracker,
    );

    let d = apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    let tombstones: Vec<_> = d
        .frame
        .construction_rows
        .iter()
        .filter_map(domain48::construction_row_deleted_id)
        .collect();
    assert_eq!(tombstones, vec![88], "removed line 88 must be tombstoned");
}

#[test]
fn clean_frame_resets_partial_projection() {
    let mut tracker = CompactDomain48Tracker::default();
    apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    apply(
        partial(vec![cq(77, 9_000_000)], vec![], vec![]),
        &mut tracker,
    );
    assert!(tracker.partial_projection.is_some());

    apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    assert!(
        tracker.partial_projection.is_none(),
        "clean frame must clear the partial projection overlay"
    );

    let p = apply(
        partial(vec![cq(77, 9_000_000)], vec![], vec![]),
        &mut tracker,
    );
    assert_eq!(p.emitted_rows, 1);
    assert_eq!(p.frame.construction_rows[0].produced_raw, 9_000_000);
}

#[test]
fn full_mode_emits_full_rows_and_clears_state() {
    let mut tracker = CompactDomain48Tracker::default();
    apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    apply(
        partial(vec![cq(77, 9_000_000)], vec![], vec![]),
        &mut tracker,
    );
    let full = compact_domain48_apply_emit_mode(
        partial(vec![cq(77, 9_000_000)], vec![bld(0x2fab, 5)], vec![]),
        D48EmitMode::Full,
        &mut tracker,
    );
    assert_eq!(full.emitted_rows, 2, "full mode emits every read row");
    assert!(tracker.baseline.last_construction.is_empty());
    assert!(tracker.partial_projection.is_none());
}

#[test]
fn a_partial_after_its_own_rejected_partial_corrects_the_row_the_fold_still_projects() {
    // kills: dropping `partial_projection` on d48's own rejection. The fold
    // keeps its projected divergence across a d48 gap, so after P1 it still
    // shows 9_000_000; a collector that re-seeded its overlay from the baseline
    // would find P3's 5_000_000 equal to the baseline and emit nothing.
    let mut tracker = CompactDomain48Tracker::default();
    apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    tracker.close_frame_transactions();
    let p1 = apply(partial(vec![cq(77, 9_000_000)], vec![], vec![]), &mut tracker);
    assert_eq!(p1.emitted_rows, 1);
    tracker.close_frame_transactions();

    let p2 = apply(partial(vec![cq(77, 7_000_000)], vec![], vec![]), &mut tracker);
    assert_eq!(p2.emitted_rows, 1, "P2 was emitted, then its record became a gap");
    tracker.restore_frame_entries();
    let key = domain48_construction_key(&cq(77, 0));
    let overlay = tracker
        .partial_projection
        .as_ref()
        .expect("the overlay the fold still projects");
    assert_eq!(overlay.construction[&key].produced_raw, 9_000_000);

    let p3 = apply(partial(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    assert_eq!(
        p3.emitted_rows, 1,
        "the row the fold projects at 9_000_000 read back at the baseline value and must be \
         corrected"
    );
    assert_eq!(p3.frame.construction_rows[0].produced_raw, 5_000_000);
}

#[test]
fn a_rejected_clean_frame_puts_the_overlay_back_whole() {
    // A clean frame drops the overlay because the fold's delta branch clears its
    // divergence; when that frame's record becomes a gap the fold never took
    // the branch, so the overlay comes back and the next partial diffs against
    // it.
    let mut tracker = CompactDomain48Tracker::default();
    apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    tracker.close_frame_transactions();
    apply(partial(vec![cq(77, 9_000_000)], vec![], vec![]), &mut tracker);
    tracker.close_frame_transactions();

    let rejected = apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    assert_ne!(rejected.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert!(tracker.partial_projection.is_none());
    tracker.restore_frame_entries();
    assert!(tracker.partial_projection.is_some(), "the overlay the fold still projects");

    let p = apply(partial(vec![cq(77, 9_000_000)], vec![], vec![]), &mut tracker);
    assert_eq!(p.emitted_rows, 0, "the fold already projects 9_000_000");
    tracker.close_frame_transactions();
    let p = apply(partial(vec![cq(77, 5_000_000)], vec![], vec![]), &mut tracker);
    assert_eq!(p.emitted_rows, 1);
}

#[test]
fn the_reset_restores_the_overlay_through_the_trackers_own_tag() {
    let mut trackers = CompactPerfGateTrackers::default();
    let tracker = &mut trackers.construction_buildings_state_damage_live;
    apply(clean(vec![cq(77, 5_000_000)], vec![], vec![]), tracker);
    tracker.close_frame_transactions();
    apply(partial(vec![cq(77, 9_000_000)], vec![], vec![]), tracker);
    tracker.close_frame_transactions();
    apply(partial(vec![cq(77, 7_000_000)], vec![], vec![]), tracker);

    let own: BTreeSet<u16> = [COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE]
        .into_iter()
        .collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &own);
    let key = domain48_construction_key(&cq(77, 0));
    let overlay = trackers
        .construction_buildings_state_damage_live
        .partial_projection
        .as_ref()
        .expect("restored, not dropped");
    assert_eq!(overlay.construction[&key].produced_raw, 9_000_000);
}

#[derive(Default, Clone)]
struct FoldRows {
    construction: BTreeMap<Domain48ConstructionKey, domain48::ConstructionQueueRow>,
    buildings: BTreeMap<Domain48BuildingKey, domain48::BuildingRow>,
    resources: BTreeMap<Domain48ResourceKey, domain48::StateResourceRow>,
}

fn fold_rows<K: Ord + Copy, R: Clone>(
    map: &mut BTreeMap<K, R>,
    rows: &[R],
    key_fn: fn(&R) -> K,
    deleted_id: fn(&R) -> Option<u32>,
    delta: bool,
) {
    for row in rows {
        let key = key_fn(row);
        if deleted_id(row).is_some() {
            assert!(delta, "only a delta frame carries a tombstone");
            map.remove(&key);
        } else {
            map.insert(key, row.clone());
        }
    }
}

/// The fold's d48 rule (recorder_host `world_fold.rs`, the `Domain48Seed`
/// match): a delta frame patches the delta baseline and clears the divergence;
/// an expected-partial frame sparse-merges into the divergence, seeded from the
/// baseline when none is held; a whole frame replaces the baseline. A gap
/// touches neither.
#[derive(Default)]
struct FoldMirror {
    delta_state: FoldRows,
    divergence: Option<FoldRows>,
}

impl FoldMirror {
    fn fold(&mut self, read: &CompactConstructionBuildingsStateDamageRead) {
        let delta = read.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
        let partial = !delta && read.status_flags & COMPACT_STATUS_PARTIAL_DATA != 0;
        let mut state = if delta {
            self.delta_state.clone()
        } else if partial {
            self.divergence
                .clone()
                .unwrap_or_else(|| self.delta_state.clone())
        } else {
            FoldRows::default()
        };
        fold_rows(
            &mut state.construction,
            &read.frame.construction_rows,
            domain48_construction_key,
            domain48::construction_row_deleted_id,
            delta,
        );
        fold_rows(
            &mut state.buildings,
            &read.frame.building_rows,
            domain48_building_key,
            domain48::building_row_deleted_id,
            delta,
        );
        fold_rows(
            &mut state.resources,
            &read.frame.resource_rows,
            domain48_resource_key,
            domain48::resource_row_deleted_id,
            delta,
        );
        if partial {
            self.divergence = Some(state);
        } else {
            self.delta_state = state;
            self.divergence = None;
        }
    }

    fn shown(&self) -> &FoldRows {
        self.divergence.as_ref().unwrap_or(&self.delta_state)
    }
}

/// About 40% of frames rejected, in a fixed pattern.
fn frame_rejected(hour: u32) -> bool {
    matches!(hour % 5, 1 | 3) || hour % 17 == 0
}

#[test]
fn a_partial_run_across_rejections_folds_to_the_rows_each_hour_read() {
    // kills: dropping the overlay on d48's own rejection (a row that reverts
    // after a gapped partial is never corrected), or journalling it with the
    // rejected frame's own upserts left in (rows the fold never received would
    // count as projected). Every ninth hour is a full read; the rest are
    // partial reads of a shifting subset whose values revert to the baseline
    // every few hours.
    let mut tracker = CompactDomain48Tracker::default();
    let mut mirror = FoldMirror::default();
    let mut corrected_after_a_rejection = 0u32;
    let mut previous_rejected = false;
    for hour in 0..240u32 {
        let clean_hour = hour % 9 == 0;
        let line_value = |line: u32| -> i32 {
            let base = 1_000_000 * line as i32;
            if (hour / 2 + line) % 3 == 0 {
                base
            } else {
                base + 250_000 * ((hour / 2) % 4) as i32
            }
        };
        let construction: Vec<_> = (1..=6u32)
            .filter(|line| clean_hour || (line + hour) % 4 != 0)
            .map(|line| cq(line, line_value(line)))
            .collect();
        let buildings: Vec<_> = (1..=3u32)
            .filter(|kind| clean_hour || (kind + hour) % 2 == 0)
            .map(|kind| bld(kind, 5 + (hour / 4 + kind) % 3))
            .collect();
        let resources: Vec<_> = (1..=2u32)
            .filter(|id| clean_hour || (id + hour) % 3 != 0)
            .map(|id| res(id, 1_000 * ((hour / 3 + id) % 5) as i32))
            .collect();
        let read = if clean_hour {
            clean(construction.clone(), buildings.clone(), resources.clone())
        } else {
            partial(construction.clone(), buildings.clone(), resources.clone())
        };
        let emitted = apply(read, &mut tracker);
        if frame_rejected(hour) {
            tracker.restore_frame_entries();
            previous_rejected = true;
            continue;
        }
        mirror.fold(&emitted);
        tracker.close_frame_transactions();
        if previous_rejected && !clean_hour && emitted.emitted_rows > 0 {
            corrected_after_a_rejection += 1;
        }
        previous_rejected = false;
        let shown = mirror.shown();
        for row in &construction {
            assert_eq!(
                shown.construction.get(&domain48_construction_key(row)),
                Some(row),
                "hour {hour}"
            );
        }
        for row in &buildings {
            assert_eq!(
                shown.buildings.get(&domain48_building_key(row)),
                Some(row),
                "hour {hour}"
            );
        }
        for row in &resources {
            assert_eq!(
                shown.resources.get(&domain48_resource_key(row)),
                Some(row),
                "hour {hour}"
            );
        }
    }
    assert!(corrected_after_a_rejection > 20, "{corrected_after_a_rejection}");
}
