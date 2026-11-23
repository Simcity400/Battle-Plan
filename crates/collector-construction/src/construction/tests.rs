use super::*;

#[test]
fn buildings_read_on_their_slot_while_construction_rest_waits_for_its_slot() {
    use construction_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::CountedMemory::new(engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    });
    let direct = kit::direct(&memory, &kit::capacities());
    let mut captured = Captured::with_capacities(&kit::capacities());
    let skipped = Outcome::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON,
    };
    for (hour, force_full) in (-24_i32..24).map(|hour| (hour, false)).chain([(0, true)]) {
        let active = force_full || hour.rem_euclid(24) == 3;
        captured.rest_raw_mode = !active;
        let native = read_accounting::Capture::new();
        let mut buildings = captured.take_buildings();
        let ticks = buildings.capture(&memory, 0, kit::BASE, hour, force_full);
        captured.restore_buildings(buildings, Ok(ticks));
        let mut ticks = captured.capture(&memory, 0, kit::BASE, hour, force_full);
        ticks[0] = captured.finish_buildings(hour, force_full);
        let counts = native.finish();
        if replay_data::cadence::should_walk(TableId::StateBuildings, hour, force_full) {
            assert_eq!(&captured.outcomes()[..2], &direct.outcomes()[..2]);
            assert!(counts[TABLES[0].index()].spans > 0);
        } else {
            assert_eq!(&captured.outcomes()[..2], &[skipped; 2], "{hour}");
            assert_eq!(ticks[0], 0);
            assert_eq!(
                counts[TABLES[0].index()],
                engine_native_memory::memory::ReadCounts::ZERO
            );
        }
        if active {
            kit::assert_matches_direct(&captured, &direct);
            assert!(counts[TABLES[2].index()].spans > 0);
        } else {
            for (index, table) in TABLES.iter().enumerate().skip(2) {
                assert_eq!(captured.outcomes()[index], skipped, "{hour}: {index}");
                assert_eq!(ticks[index], 0);
                assert_eq!(
                    counts[table.index()],
                    engine_native_memory::memory::ReadCounts::ZERO
                );
            }
            assert_eq!((captured.routes.len, captured.routes.required), (0, 0));
            assert_eq!(
                (captured.law_bytes.len, captured.law_bytes.required),
                (0, 0)
            );
        }
    }
}

#[test]
fn construction_rest_terminal_failure_preserves_finished_buildings() {
    let mut captured = Captured::with_capacities(&construction_rest_test_kit::capacities());
    captured.rest_raw_mode = true;
    captured.buildings.state_buildings.outcome = Outcome::Rows { len: 1 };
    captured.fail_construction_rest_capture("accepted acquisition failed");
    assert_eq!(
        captured.buildings.state_buildings.outcome,
        Outcome::Rows { len: 1 }
    );
    assert!(!captured.rest_raw_mode);
    assert!(captured.outcomes()[2..].iter().all(|o| *o
        == Outcome::Unavailable {
            reason: "accepted acquisition failed"
        }));
}

#[test]
fn building_handoff_consumes_its_report_without_live_retry_and_retains_growth() {
    let mut capacity = capacity();
    let mut captured = Captured::with_capacities(&capacity);
    captured.rest_raw_mode = true;
    let before = captured.allocated_bytes();
    let mut part = captured.take_buildings();
    let borrowed = part.allocated_bytes();
    assert_eq!(Buildings::vacant().allocated_bytes(), 0);
    assert_eq!(captured.allocated_bytes() + borrowed, before);
    part.state_buildings
        .rows
        .resize(20, cbuilding::Row::default());
    part.state_buildings.outcome = Outcome::Rows { len: 20 };
    part.province_buildings.outcome = Outcome::Rows { len: 0 };
    let grown = part.allocated_bytes();
    captured.restore_buildings(part, Ok(37));
    assert_eq!(captured.allocated_bytes(), before - borrowed + grown);
    let memory = engine_native_memory::memory::CountedMemory::new(engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &[],
    });
    let reads = read_accounting::Capture::new();
    assert_eq!(captured.capture(&memory, 0, 0, 0, true)[0], 0);
    assert_eq!(captured.finish_buildings(0, true), 37);
    assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 20 });
    assert_eq!(captured.finish_buildings(0, true), 0);
    assert_eq!(
        &captured.outcomes()[..2],
        &[Outcome::Unavailable {
            reason: "building acquisition not supplied"
        }; 2]
    );
    let part = captured.take_buildings();
    captured.restore_buildings(part, Err("building worker failed"));
    assert_eq!(captured.finish_buildings(0, true), 0);
    assert_eq!(
        &captured.outcomes()[..2],
        &[Outcome::Unavailable {
            reason: "building worker failed"
        }; 2]
    );
    assert_eq!(reads.finish(), read_accounting::ZERO);
    captured.reset();
    captured.grow(&mut capacity);
    assert_eq!(capacity.rows[0], 20);
    assert_eq!(captured.allocated_bytes(), before - borrowed + grown);
}

#[test]
fn construction_rest_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
    let _hint = crate::country_scale::hint_guard();
    use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
    static LOCAL: Tracker = Tracker::new();
    let mut job = ConstructionRestJob::default();
    job.census.reserve_exact(3);
    job.capacity_charge = Charge::empty_on(&LOCAL);
    job.update_capacity_charge();
    let expected = |job: &ConstructionRestJob| vector_bytes(&job.census);
    let initial = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(initial, expected(&job));
    job.grow();
    let grown = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(grown, initial, "unused census capacity is not live demand");
    assert_eq!(grown, expected(&job));
    job.clear();
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}

use replay_data::rows::TableSchema;

fn capacity() -> Capacities {
    Capacities {
        rows: [2; N],
        routes: 4,
        law_bytes: 8,
        occupied: 8,
        group_laws: 2,
        equipment: 0,
    }
}

fn fixture() -> Captured {
    let mut captured = Captured::with_capacities(&capacity());
    captured.buildings.state_buildings.outcome = Outcome::Rows { len: 1 };
    captured.buildings.province_buildings.outcome = Outcome::Rows { len: 1 };
    captured.base_resources.outcome = Outcome::Rows { len: 1 };
    captured.building_construction.outcome = Outcome::Rows { len: 1 };
    captured.railway_construction.outcome = Outcome::Rows { len: 1 };
    captured.settings.outcome = Outcome::Rows { len: 1 };
    captured.laws.outcome = Outcome::Rows { len: 1 };
    captured.garrisons.outcome = Outcome::Rows { len: 1 };
    captured.assigned_manpower.outcome = Outcome::Rows { len: 1 };
    captured.reserve_manpower.outcome = Outcome::Rows { len: 1 };
    captured.resistance.outcome = Outcome::Rows { len: 1 };
    captured.collaboration.outcome = Outcome::Rows { len: 1 };
    captured.buildings.state_buildings.rows[0] = cbuilding::Row {
        location: u32::MAX,
        building_type: 99,
        ordinal: 4,
        status_flags: 0x102,
        country: 0x8000_0001,
        level: i16::MIN,
        healthy_level: i16::MAX,
        partial_health: i64::MIN,
        active: true,
    };
    captured.buildings.province_buildings.rows[0] = cbuilding::Row {
        location: 12,
        status_flags: 1,
        ..captured.buildings.state_buildings.rows[0]
    };
    captured.base_resources.rows[0] = state_resource_entry::Row {
        state: 9,
        resource: u32::MAX,
        ordinal: 2,
        amount: i64::MIN + 1,
    };
    captured.building_construction.rows[0] = cbuilding_production_line::Row {
        id: u64::MAX,
        country: 1,
        queue_ordinal: 2,
        active_factories: i32::MAX,
        cost: i64::MAX,
        speed: i64::MIN,
        produced: -9_876_543_210,
        priority: i32::MIN,
        amount: i32::MAX,
        created_hour: 60_000_123,
        building_type: 77,
        target_flags: 0x102,
        target_location: 9,
        target_state: 9,
        target_country: u32::MAX,
        repair_levels: -3,
    };
    captured.railway_construction.rows[0] = crailway_production_line::Row {
        id: u64::MAX - 1,
        country: 1,
        queue_ordinal: 3,
        active_factories: i32::MAX,
        cost: i64::MAX,
        speed: i64::MIN,
        produced: -9_876_543_210,
        priority: i32::MIN,
        amount: i32::MAX,
        created_hour: 60_000_123,
        base_level: i32::MIN,
        target_level: i32::MAX,
        completed: 1,
        current_segment: -1,
        remaining_cost: i64::MAX - 1,
        route_start: 0,
        route_len: 3,
    };
    captured.routes.first[..3].copy_from_slice(&[7, 2, 7]);
    captured.routes.len = 3;
    captured.routes.required = 3;
    captured.settings.rows[0] = ccountry_occupation_status::Row {
        country: 1,
        garrison_template: Some(u64::MAX),
        default_law_present: 1,
        law_start: 0,
        law_len: 3,
        ..Default::default()
    };
    captured.laws.rows[0] = ccountry_occupation_data::LawRow {
        controller: u32::MAX,
        occupied: u32::MAX - 1,
        state: 0,
        name_start: 3,
        name_len: 2,
    };
    captured.law_bytes.first[..5].copy_from_slice(&[b'A', 0, 255, 0, b'B']);
    captured.law_bytes.len = 5;
    captured.law_bytes.required = 5;
    captured.garrisons.rows[0] = cstate_garrison_data::Row {
        controller: 1,
        occupied: 2,
        state: 9,
        required_manpower: i32::MIN,
        strength_ratio: i64::MAX,
        garrison_required: i64::MIN,
        serialized_flag: 0xff,
        equipment: None,
        equipment_failure: Some("fixture equipment source unavailable"),
    };
    captured.assigned_manpower.rows[0] = cstate_garrison_data::ManpowerRow {
        controller: 1,
        occupied: 2,
        state: 9,
        ordinal: u32::MAX,
        source_country: 0x8000_0001,
        amount: i32::MIN,
    };
    captured.reserve_manpower.rows[0] = ccountry_occupation_status::ReserveManpowerRow {
        country: 1,
        ordinal: 8,
        source_country: 0x8000_0001,
        amount: i32::MAX,
    };
    captured.resistance.rows[0] = cresistance::Row {
        state: 9,
        country: u32::MAX,
        level: i64::MIN,
        speed: i64::MAX,
        compliance: -6_000_000_001,
        compliance_speed: 7_000_000_003,
        active: 0xfe,
    };
    captured.collaboration.rows[0] = ccountry_collaboration_data::Row {
        source: u32::MAX,
        target: u32::MAX - 1,
        progress: i64::MIN,
    };
    captured
}

fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = view else {
        panic!("expected complete table");
    };
    rows.rows::<S>().expect("correct typed table")
}

#[test]
fn conversions_preserve_native_widths_order_bytes_and_scope() {
    let captured = fixture();
    let views = captured.tables();
    assert_eq!(
        complete::<tables::StateBuildings>(&views[0])[0].level,
        i16::MIN
    );
    assert_eq!(
        complete::<tables::StateBuildings>(&views[0])[0].partial_health,
        i64::MIN
    );
    assert_eq!(
        complete::<tables::ProvinceBuildings>(&views[1])[0].status_flags,
        1
    );
    assert!(complete::<tables::StateBuildings>(&views[0])[0].active);
    assert_eq!(
        complete::<tables::StateBaseResources>(&views[2])[0].amount,
        i64::MIN + 1
    );
    let construction = &complete::<tables::BuildingConstruction>(&views[3])[0];
    assert_eq!(
        (
            construction.id,
            construction.cost,
            construction.speed,
            construction.produced
        ),
        (u64::MAX, i64::MAX, i64::MIN, -9_876_543_210)
    );
    assert_eq!(
        (
            construction.target_flags,
            construction.priority,
            construction.repair_levels
        ),
        (0x102, i32::MIN, -3)
    );
    let railway = &complete::<tables::RailwayConstruction>(&views[4])[0];
    assert_eq!(
        (
            railway.cancelled,
            railway.current_segment,
            railway.remaining_cost
        ),
        (1, -1, i64::MAX - 1)
    );
    assert_eq!(railway.route, [7, 2, 7]);
    assert_eq!(
        complete::<tables::OccupationSettings>(&views[5])[0].law,
        [b'A', 0, 255]
    );
    let law = &complete::<tables::StateOccupationLaws>(&views[6])[0];
    assert_eq!(
        (law.controller, law.occupied, law.state),
        (u32::MAX, u32::MAX - 1, 0)
    );
    assert_eq!(law.name, [0, b'B']);
    assert!(law.key() > u64::MAX as u128);
    assert_eq!(
        complete::<tables::StateGarrisons>(&views[7])[0].garrison_required,
        i64::MIN
    );
    let rows::Recorded::Value(equipment) =
        &complete::<tables::StateGarrisons>(&views[7])[0].equipment
    else {
        panic!("expected recorded equipment refusal");
    };
    assert!(equipment.rows.is_empty());
    assert_eq!(
        equipment.failure.as_deref(),
        Some(b"fixture equipment source unavailable".as_slice())
    );
    assert_eq!(
        complete::<tables::StateAssignedManpower>(&views[8])[0].amount,
        i32::MIN
    );
    assert_eq!(
        complete::<tables::CountryOccupationReserveManpower>(&views[9])[0].amount,
        i32::MAX
    );
    assert_eq!(
        complete::<tables::StateResistance>(&views[10])[0].compliance,
        -6_000_000_001
    );
    assert_eq!(
        complete::<tables::CountryCollaboration>(&views[11])[0].progress,
        i64::MIN
    );
}

#[test]
fn invalid_arenas_duplicate_keys_and_source_faults_are_table_local() {
    let mut captured = fixture();
    captured.railway_construction.rows[0].route_start = usize::MAX;
    captured.settings.rows[0].law_start = 6;
    captured.buildings.state_buildings.outcome = Outcome::Unavailable {
        reason: "building source unreadable",
    };
    let views = captured.tables();
    for index in [0, 4, 5] {
        assert!(matches!(views[index], TableView::Unavailable { .. }));
    }
    assert!(matches!(views[1], TableView::Complete(_)));
    assert!(matches!(views[6], TableView::Complete(_)));
    captured.building_construction.rows[1] = captured.building_construction.rows[0];
    captured.building_construction.outcome = Outcome::Rows { len: 2 };
    assert!(matches!(
        captured.tables()[3],
        TableView::Unavailable { .. }
    ));
    captured.laws.rows[1] = captured.laws.rows[0];
    captured.laws.rows[1].occupied -= 1;
    captured.laws.outcome = Outcome::Rows { len: 2 };
    assert_eq!(
        complete::<tables::StateOccupationLaws>(&captured.tables()[6]).len(),
        2
    );
    captured.buildings.province_buildings.outcome = Outcome::Truncated {
        len: 1,
        unread_rows: 5,
    };
    assert!(matches!(
        captured.tables()[1],
        TableView::Truncated { missing: 5, .. }
    ));
    captured.reset();
    assert!(
        captured
            .outcomes()
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
    );
}

#[test]
fn growth_counts_omitted_rows_and_primary_arena_pressure() {
    let mut capacity = capacity();
    let mut captured = Captured::with_capacities(&capacity);
    captured.railway_construction.outcome = Outcome::Truncated {
        len: 1,
        unread_rows: 20,
    };
    captured.routes.required = 29;
    captured.law_bytes.required = 41;
    captured.grow(&mut capacity);
    assert!(capacity.rows[4] >= 21 && capacity.routes >= 29 && capacity.law_bytes >= 41);
    assert!(captured.routes.first.len() >= 29 && captured.law_bytes.first.len() >= 41);
    let before = captured.allocated_bytes();
    captured.reset();
    captured.grow(&mut capacity);
    assert_eq!(captured.allocated_bytes(), before);
    let fresh = Captured::with_capacities(&capacity);
    assert_eq!(fresh.routes.first.len(), captured.routes.first.len());
    assert_eq!(fresh.law_bytes.first.len(), captured.law_bytes.first.len());
}

#[test]
fn construction_rest_split_copy_matches_direct_walk() {
    use super::construction_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::CountedMemory::new(engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    });
    let capacities = kit::capacities();
    let native = read_accounting::Capture::new();
    let direct = kit::direct(&memory, &capacities);
    let direct_counts = native.finish();
    let native = read_accounting::Capture::new();
    let (split, _, _) = kit::split(&memory, &capacities);
    let counts = native.finish();
    let mut expected_counts = read_accounting::ZERO;
    expected_counts[TABLES[0].index()] = direct_counts[TABLES[0].index()];
    expected_counts[TABLES[2].index()] = counts[TABLES[2].index()];
    assert!(counts[TABLES[0].index()].spans > 0);
    assert!(counts[TABLES[2].index()].spans > 0);
    assert_eq!(counts, expected_counts);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.base_resources.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        split.building_construction.outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(split.railway_construction.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        split.resistance.outcome,
        Outcome::Unavailable {
            reason: "state resistance is acquired by the state worker"
        }
    );
    assert_eq!(
        counts[TableId::StateResistance.index()],
        engine_native_memory::memory::ReadCounts::ZERO
    );
    assert_eq!(split.reserve_manpower.outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.collaboration.outcome, Outcome::Rows { len: 0 });
}

#[test]
fn occupied_roster_split_copy_preserves_settings_from_empty_storage_and_on_native_failure() {
    use super::construction_rest_test_kit as kit;
    for failure in [false, true] {
        let mut bytes = kit::fixture();
        if failure {
            let address = 0x7300 + ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER;
            bytes[address - kit::BASE..address - kit::BASE + 8]
                .copy_from_slice(&0xdead_0000_u64.to_le_bytes());
        }
        let memory = engine_native_memory::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        for storage in [0, 1] {
            let mut capacity = kit::capacities();
            capacity.occupied = storage;
            let direct = kit::direct(&memory, &capacity);
            let (split, _, _) = kit::split(&memory, &capacity);
            kit::assert_matches_direct(&split, &direct);
            let tables = split.tables();
            let TableView::Complete(rows::TableRows::OccupationSettings(settings)) = &tables[5]
            else {
                panic!("existing settings lost");
            };
            assert_eq!(settings[0].default_law_present, 1);
            assert_eq!(settings[0].law, b"civilian");
            assert_eq!(
                settings[0].occupied,
                rows::Recorded::Value((!failure).then(|| vec![1]))
            );
        }
    }
}

#[test]
fn construction_rest_truncation_matrix_preserves_unrelated_caps() {
    use super::construction_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    for rows in [[0; N], [1; N], [2; N], [64; N]] {
        let capacities = Capacities {
            rows,
            routes: 64,
            law_bytes: 64,
            occupied: 8,
            group_laws: 1,
            equipment: 0,
        };
        let mut expected = capacities;
        expected.rows[5] = expected.rows[5].max(1);
        let direct = kit::direct(&memory, &expected);
        let mut captured = Captured::with_capacities(&capacities);
        let ctx = ConstructionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
        };
        let mut first = construction_rest_test_kit::job();
        let report = crate::construction::copy_construction_rest_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok, "rows={rows:?}");
        captured.rest_raw_mode = true;
        let mut buildings = captured.take_buildings();
        let ticks = buildings.capture(&memory, 0, kit::BASE, 0, true);
        captured.restore_buildings(buildings, Ok(ticks));
        let _ = captured.capture(&memory, 0, kit::BASE, 0, true);
        captured.finish_buildings(0, true);
        let mut ticks = [0u64; 10];
        captured.decode_construction_rest_payload(0, kit::BASE, &first, &mut ticks);
        kit::assert_matches_direct(&captured, &direct);
    }
}

#[test]
fn construction_rest_split_copy_matches_direct_walk_under_truncation() {
    use super::construction_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let capacities = Capacities {
        rows: [1; N],
        routes: 1,
        law_bytes: 4,
        occupied: 8,
        group_laws: 1,
        equipment: 0,
    };
    let mut expected = capacities;
    expected.law_bytes = 16;
    let direct = kit::direct(&memory, &expected);
    let (split, _, _) = {
        let mut captured = Captured::with_capacities(&capacities);
        let ctx = ConstructionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
        };
        let mut first = construction_rest_test_kit::job();
        let report = crate::construction::copy_construction_rest_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        captured.rest_raw_mode = true;
        let mut buildings = captured.take_buildings();
        let ticks = buildings.capture(&memory, 0, kit::BASE, 0, true);
        captured.restore_buildings(buildings, Ok(ticks));
        let _ = captured.capture(&memory, 0, kit::BASE, 0, true);
        captured.finish_buildings(0, true);
        let mut ticks = [0u64; 10];
        captured.decode_construction_rest_payload(0, kit::BASE, &first, &mut ticks);
        (captured, report, first)
    };
    kit::assert_matches_direct(&split, &direct);
}

#[test]
fn construction_rest_first_decode_grows_settings_and_shared_law_bytes() {
    use super::construction_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::CountedMemory::new(engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    });
    let native = read_accounting::Capture::new();
    let (roomy, roomy_report, _) = kit::split(&memory, &kit::capacities());
    let roomy_counts = native.finish();
    for law_bytes in [0, 8] {
        let mut capacity = kit::capacities();
        capacity.rows[5] = 0;
        capacity.law_bytes = law_bytes;
        let native = read_accounting::Capture::new();
        let (mut captured, report, mut job) = kit::split(&memory, &capacity);
        assert_eq!(native.finish(), roomy_counts);
        assert_eq!(report.raw_bytes, roomy_report.raw_bytes);
        kit::assert_matches_direct(&captured, &roomy);
        assert_eq!(captured.settings.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.laws.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            &captured.law_bytes.first[..captured.law_bytes.len],
            b"civiliancivilian"
        );
        assert_eq!(job.census.len(), 1);
        assert_eq!(job.settings_required, 1);
        assert_eq!(job.law_bytes_required, 16);
        assert!(!job.law_demand_overflowed);
        assert_eq!(captured.settings.rows.len(), job.settings_required);
        assert_eq!(captured.law_bytes.first.len(), job.law_bytes_required);
        let allocated = captured.allocated_bytes();
        captured.reset();
        captured.grow(&mut capacity);
        assert_eq!(captured.allocated_bytes(), allocated);
        assert_eq!(capacity.rows[5], job.settings_required);
        assert_eq!(capacity.law_bytes, job.law_bytes_required);
        job.clear();
        assert_eq!((job.settings_required, job.law_bytes_required), (0, 0));
        assert!(!job.law_demand_overflowed);
    }
}

const GARRISON_OPERANDS: usize = 5;

fn garrison_equipment_fixture() -> Vec<u8> {
    use super::construction_rest_test_kit as kit;
    use engine_native_parts::cequipment_type as ty;
    use engine_native::cequipment_variant::pool as pool;
    use engine_occupation::garrison_equipment_pool;
    let mut bytes = kit::fixture();
    const REQUIRED: usize = 0x18000;
    const ALLOCATED: usize = REQUIRED + GARRISON_OPERANDS * 16;
    const EQUIPMENT: usize = ALLOCATED + 24;
    bytes.resize(EQUIPMENT + 0x600 - kit::BASE, 0);
    let mut put = |at: usize, value: &[u8]| {
        bytes[at - kit::BASE..at - kit::BASE + value.len()].copy_from_slice(value)
    };
    for (offset, vtable, data, count) in [
        (
            cstate_garrison_data::REQUIRED_EQUIPMENT,
            garrison_equipment_pool::VTABLE_RVA,
            REQUIRED,
            GARRISON_OPERANDS as i32,
        ),
        (
            cstate_garrison_data::LOCAL_EQUIPMENT,
            pool::VTABLE_RVA,
            ALLOCATED,
            1,
        ),
    ] {
        put(kit::GAR_OBJ + offset, &(vtable.0 as u64).to_le_bytes());
        put(kit::GAR_OBJ + offset + 8, &(data as u64).to_le_bytes());
        put(kit::GAR_OBJ + offset + 16, &count.to_le_bytes());
        put(kit::GAR_OBJ + offset + 20, &count.to_le_bytes());
    }
    put(EQUIPMENT, &(ty::VTABLE_RVA.0 as u64).to_le_bytes());
    put(EQUIPMENT + ty::TOKEN, &700_u32.to_le_bytes());
    put(EQUIPMENT + ty::ACTIVE, &[1]);
    put(EQUIPMENT + ty::ARCHETYPE, &(EQUIPMENT as u64).to_le_bytes());
    put(EQUIPMENT + ty::ARCHETYPE_ID, &1_i32.to_le_bytes());
    put(EQUIPMENT + ty::BUILD_COST_IC, &200_003_i64.to_le_bytes());
    put(ALLOCATED, &(EQUIPMENT as u64).to_le_bytes());
    put(ALLOCATED + 16, &50_000_i64.to_le_bytes());
    for index in 0..GARRISON_OPERANDS {
        put(REQUIRED + index * 16, &(EQUIPMENT as u64).to_le_bytes());
        put(
            REQUIRED + index * 16 + 8,
            &(100_000 + index as i64).to_le_bytes(),
        );
    }
    bytes
}

#[test]
fn garrison_equipment_first_acquisition_grows_operands_and_matches_owned_read_witness() {
    use super::construction_rest_test_kit as kit;
    let bytes = garrison_equipment_fixture();
    let memory = engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let (split, report, job) = kit::split(&memory, &kit::capacities());
    assert!(report.raw_ok);
    assert!(job.equipment_required >= GARRISON_OPERANDS);
    assert!(split.equipment.first.len() >= GARRISON_OPERANDS);
    let mut capacity = kit::capacities();
    capacity.equipment = job.equipment_required;
    let direct = kit::direct(&memory, &capacity);
    assert_eq!(split.tables()[7], direct.tables()[7]);
    let tables = split.tables();
    let rows = complete::<tables::StateGarrisons>(&tables[7]);
    let rows::Recorded::Value(inputs) = &rows[0].equipment else {
        panic!()
    };
    assert!(inputs.failure.is_none());
    assert_eq!(inputs.rows.len(), GARRISON_OPERANDS);
    assert_eq!(
        inputs.rows[GARRISON_OPERANDS - 1].required_raw,
        100_000 + GARRISON_OPERANDS as i64 - 1
    );
    assert_eq!(
        inputs.rows[GARRISON_OPERANDS - 1].required_lookup_raw,
        100_000
    );
    assert_eq!(inputs.rows[GARRISON_OPERANDS - 1].allocated_raw, 50_000);
    assert_eq!(inputs.rows[GARRISON_OPERANDS - 1].weight_raw, 200_003);
}

#[test]
fn failed_garrison_equipment_read_keeps_scalar_siblings_and_names_the_local_refusal() {
    use super::construction_rest_test_kit as kit;
    struct MissingWeight<'a> {
        bytes: &'a [u8],
    }
    impl engine_native_memory::Memory for MissingWeight<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            const EQUIPMENT: usize = 0x18000 + GARRISON_OPERANDS * 16 + 24;
            if address == EQUIPMENT + engine_native_parts::cequipment_type::BUILD_COST_IC {
                return false;
            }
            engine_native_memory::memory::SliceMemory {
                base: kit::BASE,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }
    let bytes = garrison_equipment_fixture();
    let memory = MissingWeight { bytes: &bytes };
    let (split, _, job) = kit::split(&memory, &kit::capacities());
    let mut capacity = kit::capacities();
    capacity.equipment = job.equipment_required;
    let direct = kit::direct(&memory, &capacity);
    assert_eq!(split.tables()[7], direct.tables()[7]);
    assert!(matches!(split.garrisons.outcome, Outcome::Rows { len: 1 }));
    assert!(matches!(
        split.assigned_manpower.outcome,
        Outcome::Rows { len: 1 }
    ));
    let tables = split.tables();
    let rows = complete::<tables::StateGarrisons>(&tables[7]);
    let rows::Recorded::Value(inputs) = &rows[0].equipment else {
        panic!()
    };
    assert_eq!(
        inputs.failure.as_deref(),
        Some(b"garrison equipment operand unreadable".as_slice())
    );
    assert!(inputs.rows.is_empty());
}

#[test]
fn construction_rest_law_growth_preserves_original_faults_and_failed_payload_demand() {
    use super::construction_rest_test_kit as kit;
    struct Fault<'a> {
        memory: engine_native_memory::memory::SliceMemory<'a>,
        address: usize,
        len: usize,
        occurrence: usize,
        seen: std::cell::Cell<usize>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == self.address && destination.len() == self.len {
                self.seen.set(self.seen.get() + 1);
                if self.seen.get() == self.occurrence {
                    return false;
                }
            }
            self.memory.copy(address, destination)
        }
    }
    let mut bytes = kit::fixture();
    let name = kit::LAW_OBJ + coccupation_law::NAME;
    bytes[name - kit::BASE + 7] = 0;
    let length = name - kit::BASE + msvc_string::LENGTH;
    bytes[length..length + 8].copy_from_slice(&7_u64.to_le_bytes());
    for (address, len, occurrence, required) in [
        (name, 32, 1, 7),
        (name, 7, 1, 14),
        (name, 7, 2, 14),
        (name + 7, 1, 1, 14),
    ] {
        let memory = || Fault {
            memory: engine_native_memory::memory::SliceMemory {
                base: kit::BASE,
                bytes: &bytes,
            },
            address,
            len,
            occurrence,
            seen: std::cell::Cell::new(0),
        };
        let direct = kit::direct(&memory(), &kit::capacities());
        let mut capacity = kit::capacities();
        capacity.rows[5] = 0;
        capacity.law_bytes = 7;
        let (mut captured, _, mut job) = kit::split(&memory(), &capacity);
        kit::assert_matches_direct(&captured, &direct);
        assert_eq!(job.census.len(), 1);
        assert_eq!(job.settings_required, 1);
        assert_eq!(job.law_bytes_required, required);
        assert_eq!(captured.law_bytes.first.len(), required);
        assert_eq!(captured.law_bytes.required, required);
        captured.grow(&mut capacity);
        assert!(capacity.rows[5] >= job.settings_required);
        assert!(capacity.law_bytes >= required);
        job.law_demand_overflowed = true;
        captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
        for (actual, expected) in captured.outcomes()[5..7]
            .iter()
            .zip(&direct.outcomes()[5..7])
        {
            if matches!(expected, Outcome::Unavailable { .. }) {
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn construction_rest_demand_failures_are_explicit_and_keep_successful_allocations() {
    use super::construction_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let (reference, _, mut job) = kit::split(&memory, &kit::capacities());
    job.law_bytes_required = usize::MAX - 4;
    copy_construction_string(
        &mut job.log,
        &memory,
        kit::LAW_OBJ + coccupation_law::NAME,
        M_C_OCC,
        &mut job.law_bytes_required,
        &mut job.law_demand_overflowed,
    );
    assert!(job.law_demand_overflowed);
    for (settings, names, overflow, reason) in [
        (2, 0, true, "occupation law byte demand overflow"),
        (
            2,
            usize::MAX,
            false,
            "occupation law byte allocation failed",
        ),
        (
            usize::MAX,
            32,
            false,
            "occupation settings allocation failed",
        ),
    ] {
        job.settings_required = settings;
        job.law_bytes_required = names;
        job.law_demand_overflowed = overflow;
        let mut capacity = kit::capacities();
        capacity.rows[5] = 0;
        capacity.law_bytes = 0;
        let mut captured = Captured::with_capacities(&capacity);
        let before = captured.allocated_bytes();
        captured.decode_construction_rest_payload(0, kit::BASE, &job, &mut [0; 10]);
        assert_eq!(captured.settings.outcome, Outcome::Unavailable { reason });
        if settings == usize::MAX {
            assert_eq!(captured.laws.outcome, reference.laws.outcome);
        } else {
            assert_eq!(captured.laws.outcome, Outcome::Unavailable { reason });
            assert_eq!(captured.settings.rows.len(), 2);
            assert!(captured.allocated_bytes() > before);
        }
        assert_eq!(&captured.outcomes()[7..], &reference.outcomes()[7..]);
    }
}

#[test]
fn occupation_group_and_database_laws_survive_raw_copy_without_changing_state_zero() {
    use construction_rest_test_kit as kit;
    const DATABASE: usize = 0x8500;
    struct DatabaseMemory<'a> {
        inner: engine_native_memory::memory::SliceMemory<'a>,
        fail_group: bool,
    }
    impl Memory for DatabaseMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == coccupation_law::DATABASE_RVA.0 && destination.len() == 8 {
                destination.copy_from_slice(&(DATABASE as u64).to_le_bytes());
                return true;
            }
            if self.fail_group && address == kit::OCC_DATA + ccountry_occupation_data::GROUP_LAW {
                return false;
            }
            self.inner.copy(address, destination)
        }
    }
    for (group_present, fail_group) in [(false, false), (true, false), (true, true)] {
        let mut bytes = kit::fixture();
        let at = kit::OCC_DATA + ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER - kit::BASE;
        bytes[at..at + 8].copy_from_slice(&(kit::COUNTRY as u64).to_le_bytes());
        let at = DATABASE + coccupation_law::DATABASE_FALLBACK - kit::BASE;
        bytes[at..at + 8].copy_from_slice(&(kit::LAW_OBJ as u64).to_le_bytes());
        if group_present {
            let at = kit::OCC_DATA + ccountry_occupation_data::GROUP_LAW - kit::BASE;
            bytes[at..at + 8].copy_from_slice(&(kit::LAW_OBJ as u64).to_le_bytes());
        }
        let memory = DatabaseMemory {
            inner: engine_native_memory::memory::SliceMemory {
                base: kit::BASE,
                bytes: &bytes,
            },
            fail_group,
        };
        let direct = kit::direct(&memory, &kit::capacities());
        let mut capacity = kit::capacities();
        capacity.group_laws = 0;
        let (split, _, job) = kit::split(&memory, &capacity);
        assert_eq!(job.census.len(), 1);
        assert_eq!(job.group_laws_required, 1);
        kit::assert_matches_direct(&split, &direct);
        let tables = split.tables();
        let row = &complete::<tables::OccupationSettings>(&tables[5])[0];
        assert_eq!(
            row.fallback_law,
            rows::Recorded::Value(Some(b"civilian".to_vec()))
        );
        assert_eq!(
            complete::<tables::StateOccupationLaws>(&tables[6])[0].state,
            0
        );
        if fail_group {
            assert_eq!(row.group_laws, rows::Recorded::Value(None));
            assert_eq!(row.law, b"civilian");
        } else {
            assert_eq!(
                row.group_laws,
                rows::Recorded::Value(Some(vec![rows::OccupationGroupLaw {
                    occupied: 1,
                    scope_country: Some(1),
                    law: group_present.then(|| b"civilian".to_vec()),
                }]))
            );
        }
    }
}

fn put_u32(bytes: &mut [u8], address: usize, value: u32) {
    let at = address - construction_rest_test_kit::BASE;
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn an_occupied_raw_alias_the_census_never_reads_resolves_as_the_live_walk_resolves_it() {
    use construction_rest_test_kit as kit;
    for countries in [3, 4] {
        let mut bytes = kit::fixture();
        for (offset, count) in [
            (cgame_state::COUNTRIES, countries),
            (cgame_state::COUNTRY_LINKS, 4),
        ] {
            put_u32(&mut bytes, kit::BASE + offset + 8, count);
            put_u32(&mut bytes, kit::BASE + offset + 12, count);
        }
        put_u32(&mut bytes, 0x2100 + 3 * 4, 1);
        put_u32(
            &mut bytes,
            kit::OCC_DATA + ccountry_occupation_data::OCCUPIED_COUNTRY,
            3,
        );
        let memory = engine_native_memory::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let direct = kit::direct(&memory, &kit::capacities());
        let (split, _, _) = kit::split(&memory, &kit::capacities());
        kit::assert_matches_direct(&split, &direct);
        for outcome in [
            split.laws.outcome,
            split.group_laws.outcome,
            split.garrisons.outcome,
            split.assigned_manpower.outcome,
        ] {
            assert!(
                matches!(outcome, Outcome::Rows { len: 1.. }),
                "countries={countries} {outcome:?}"
            );
        }
        assert_eq!(split.group_laws.rows[0].occupied, 3);
    }
}

#[test]
fn a_failed_state_or_country_root_refuses_only_the_families_that_read_it() {
    use construction_rest_test_kit as kit;
    const STATE_ROOT: u8 = 0;
    const COUNTRY_ROOT: u8 = 1;
    const COUNTRY_SLOT: u8 = 2;
    for failure in [STATE_ROOT, COUNTRY_ROOT, COUNTRY_SLOT] {
        let mut bytes = kit::fixture();
        match failure {
            STATE_ROOT => put_u32(&mut bytes, kit::BASE + cgame_state::STATES + 8, 1),
            COUNTRY_ROOT => put_u32(&mut bytes, kit::BASE + cgame_state::COUNTRIES + 8, 1),
            _ => {
                let at = 0x2000 + 2 * 8 - kit::BASE;
                bytes[at..at + 8].copy_from_slice(&0x11000_u64.to_le_bytes());
            }
        }
        let memory = engine_native_memory::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let direct = kit::direct(&memory, &kit::capacities());
        let (split, _, _) = kit::split(&memory, &kit::capacities());
        kit::assert_matches_direct(&split, &direct);
        let resources = split.base_resources.outcome;
        let countries = [
            split.settings.outcome,
            split.laws.outcome,
            split.group_laws.outcome,
            split.reserve_manpower.outcome,
            split.collaboration.outcome,
        ];
        if failure == STATE_ROOT {
            assert!(matches!(resources, Outcome::Unavailable { .. }));
            assert!(matches!(
                split.garrisons.outcome,
                Outcome::Unavailable { .. }
            ));
            assert!(
                countries
                    .iter()
                    .all(|outcome| !matches!(outcome, Outcome::Unavailable { .. })),
                "{countries:?}"
            );
        } else {
            assert_eq!(resources, Outcome::Rows { len: 1 }, "failure={failure}");
            assert!(
                countries
                    .iter()
                    .all(|outcome| matches!(outcome, Outcome::Unavailable { .. })),
                "{countries:?}"
            );
        }
    }
}
