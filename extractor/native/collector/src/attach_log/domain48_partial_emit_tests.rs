use super::*;

#[test]
fn domain48_lane_partial_causes_match_partial_predicate_per_cause() {
    use compact_contract::status_origins as so;
    let clean = domain48::collector_module::CollectorStats::default();
    assert!(domain48_lane_partial_causes(&clean)
        .iter()
        .all(|(_, count)| *count == 0));
    let expected_ids = [
        so::SO_D48_LANE_FAILED_COUNTRY_ROOTS,
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
    assert_eq!(tracker.last_construction[&key].produced_raw, 5_000_000);

    apply(
        partial(vec![cq(77, 9_000_000)], vec![], vec![]),
        &mut tracker,
    );
    assert_eq!(
        tracker.last_construction[&key].produced_raw, 5_000_000,
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
    assert!(tracker.last_construction.is_empty());
    assert!(tracker.partial_projection.is_none());
}
