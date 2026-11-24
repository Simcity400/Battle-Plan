use super::*;

#[test]
fn growing_naval_history_raw_jobs_decode_full_first_and_later_mod_populations() {
    let capacity = Capacities {
        rows: [0; N],
        sunk_names: 0,
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    let mut supremacy = SupremacyJob::new();
    let mut sunk = SunkJob::new();
    for count in [5_000_usize, 12_000] {
        let bytes = supremacy_test_kit::mod_fixture(count);
        let report = copy_supremacy_refresh(
            &supremacy_test_kit::memory(&bytes),
            SupremacyCopyContext {
                base: supremacy_test_kit::BASE,
                game_state: supremacy_test_kit::GAME_STATE,
                hour: 0,
                force_full: true,
            },
            &mut supremacy,
        );
        assert!(report.raw_ok);
        captured.supremacy.rows.truncate(0);
        captured.decode_supremacy_payload(supremacy_test_kit::BASE, &supremacy, &mut [0]);
        assert_eq!(captured.supremacy.outcome, Outcome::Rows { len: count });

        let bytes = sunk_test_kit::mod_fixture(count);
        let report = copy_sunk_refresh(
            &sunk_test_kit::memory(&bytes),
            SunkCopyContext {
                base: sunk_test_kit::BASE,
                game_state: sunk_test_kit::GAME_STATE,
                hour: sunk_test_kit::HOUR,
                force_full: true,
            },
            &mut sunk,
        );
        assert!(report.raw_ok);
        captured.sunk.rows.truncate(0);
        captured.sunk_names.first.truncate(0);
        captured.decode_sunk_payload(sunk_test_kit::BASE, &sunk, &mut [0]);
        assert_eq!(captured.sunk.outcome, Outcome::Rows { len: count });
        assert_eq!(captured.sunk_names.len, captured.sunk_names.required);
        assert!(captured.sunk_names.len > count);
    }
}

#[test]
fn railway_optional_projection_status_and_failed_growth_keep_their_domains() {
    use crailway_manager::fresh_inputs::{RailwayBuildingRow, RailwayBuildingState};
    let mut captured = Captured::vacant();
    captured.cooldowns.rows = vec![
        cprovince_railway_info::Row {
            province: 7,
            cooldown_ticks: 9,
            building_bound: true,
        },
        cprovince_railway_info::Row {
            province: 3,
            cooldown_ticks: 0,
            building_bound: false,
        },
        cprovince_railway_info::Row {
            province: 1,
            cooldown_ticks: -1,
            building_bound: true,
        },
    ];
    captured.cooldowns.outcome = Outcome::Rows { len: 3 };
    captured.rail_buildings.rows = vec![
        RailwayBuildingRow {
            province: 1,
            building: Err("rail health unreadable"),
            ..Default::default()
        },
        RailwayBuildingRow {
            province: 3,
            building: Ok(Some(RailwayBuildingState { building: None })),
            ..Default::default()
        },
        RailwayBuildingRow {
            province: 7,
            building: Ok(None),
            ..Default::default()
        },
    ];
    captured.rail_buildings.outcome = Outcome::Rows { len: 3 };
    captured.rail_buildings_required = 5;
    let tables = captured.tables();
    let TableView::Complete(TableRows::RailCooldowns(cooldowns)) = &tables[2] else {
        panic!("cooldowns were denied by an optional field");
    };
    assert_eq!(
        cooldowns.iter().map(|row| row.province).collect::<Vec<_>>(),
        [1, 3, 7]
    );
    assert_eq!(cooldowns[0].cooldown_ticks, -1);
    assert_eq!(cooldowns[0].building, rows::Recorded::Value(None));
    assert_eq!(
        cooldowns[1].building,
        rows::Recorded::Value(Some(rows::RailwayBuildingState { building: None }))
    );
    assert_eq!(cooldowns[2].building, rows::Recorded::Value(None));
    assert_eq!(
        cooldowns.iter().map(|row| row.building_bound).collect::<Vec<_>>(),
        [true, false, true].map(|bound| rows::Recorded::Value(Some(bound)))
    );
    captured.fail_late_capture(0, false, "late worker failed");
    assert_eq!(captured.rail_buildings_required, 5);
    assert_eq!(captured.railway_building(3), None);
    let mut capacity = Capacities {
        rows: [0; N],
        theatre_names: 0,
        sunk_names: 0,
        orders_sources: 0,
        wing_sources: 0,
        roster_scratch: 0,
        rail_levels: 0,
    };
    captured.grow(&mut capacity);
    assert!(capacity.rows[2] >= 5);
    assert_eq!(captured.cooldowns.rows.len(), capacity.rows[2]);
    assert_eq!(captured.rail_buildings.rows.len(), capacity.rows[2]);
    captured.reset();
    assert_eq!(captured.rail_buildings_required, 0);
    assert_eq!(captured.railway_building(3), None);
}

#[test]
fn selected_railway_projection_counts_refusals_and_retains_arena_demand_through_failure() {
    use crailway_manager::{
        fresh_inputs::RailwayBuildingRow,
        selected_inputs::{LevelSpan, SelectionRow},
    };
    let mut captured = Captured::vacant();
    captured.cooldowns.rows = vec![cprovince_railway_info::Row {
        province: 1,
        cooldown_ticks: 0,
        building_bound: false,
    }];
    captured.cooldowns.outcome = Outcome::Rows { len: 1 };
    captured.rail_buildings.rows = vec![RailwayBuildingRow {
        province: 1,
        building: Err("ETA-only healthy value unavailable"),
        selection: Some(SelectionRow {
            levels: Ok(LevelSpan {
                first: 0,
                len: 3,
                positive: false,
            }),
            pair_count: Ok(-2),
            building_level: Ok(Some(Some(-3))),
        }),
    }];
    captured.rail_buildings.outcome = Outcome::Rows { len: 1 };
    captured.rail_buildings_required = 2;
    captured.rail_levels.first = vec![0, -1, i32::MIN];
    captured.rail_levels.len = 3;
    captured.rail_levels.required = 9;
    captured.rail_hour = Some(12);
    let tables = captured.tables();
    let TableView::Complete(TableRows::RailCooldowns(rows)) = &tables[2] else {
        panic!()
    };
    assert_eq!(rows[0].building, None.into());
    assert_eq!(
        rows[0].selection,
        Some(rows::RailwaySelectionInputs {
            levels: Some(vec![0, -1, i32::MIN]),
            pair_count: Some(-2),
            building_level: Some(Some(-3)),
        })
        .into()
    );
    assert_eq!(rows[0].selection_hour, Some(12).into());
    captured.rail_buildings.rows[0]
        .selection
        .as_mut()
        .unwrap()
        .levels = Err("selected railway level capacity insufficient");
    assert_eq!(captured.railway_selection(1).unwrap().levels, None);
    captured.fail_late_capture(12, true, "late failed");
    assert_eq!(captured.railway_selection(1), None);
    assert_eq!(captured.rail_levels.required, 9);
    let mut capacities = Capacities {
        rows: [0; N],
        theatre_names: 0,
        sunk_names: 0,
        orders_sources: 0,
        wing_sources: 0,
        roster_scratch: 0,
        rail_levels: 0,
    };
    captured.grow(&mut capacities);
    assert!(capacities.rail_levels >= 9);
    assert!(captured.rail_levels.first.len() >= 9);
    assert!(capacities.rows[2] >= 2);
    captured.reset();
    assert_eq!(
        (captured.rail_levels.len, captured.rail_levels.required),
        (0, 0)
    );
    assert_eq!(captured.rail_hour, None);
    assert_eq!(captured.railway_selection(1), None);
}

fn owned_failure_fixture() -> Captured {
    Captured::with_capacities(&Capacities {
        rows: [1; N],
        theatre_names: 8,
        sunk_names: 8,
        orders_sources: 1,
        wing_sources: 1,
        roster_scratch: 8,
        rail_levels: 0,
    })
}

#[test]
fn census_roots_snapshot_grows_in_the_pass_and_never_outlives_it() {
    let roots = [corders_group::Root::default(); 5];
    let mut job = NavalJob::default();
    job.capture_roots(&roots, Outcome::Rows { len: 3 });
    assert_eq!(job.roots_len, Some(3));
    assert_eq!(job.roots, roots[..3]);
    job.capture_roots(
        &roots,
        Outcome::Truncated {
            len: 3,
            unread_rows: 4,
        },
    );
    assert_eq!(job.roots_len, None);
    assert_eq!(job.roots_required, 7);
    job.capture_roots(&roots, Outcome::Unavailable { reason: "torn" });
    assert_eq!(job.roots_len, None);
    job.capture_roots(&roots, Outcome::Rows { len: 3 });
    job.clear();
    assert_eq!((job.roots_len, job.roots.len()), (None, 0));
    job.capture_roots(&roots[..2], Outcome::Rows { len: 3 });
    assert_eq!(job.roots_len, None);
}

#[test]
fn oob_theatre_dedup_keeps_first_occurrences_in_root_order() {
    let first_seen = |values: &[usize]| {
        let mut seen = Vec::new();
        for &value in values {
            if !seen.contains(&value) {
                seen.push(value);
            }
        }
        seen
    };
    for values in [
        vec![],
        vec![7],
        vec![7, 7, 7],
        vec![0x30, 0x10, 0x30, 0x20, 0x10, 0x40, 0x20],
        vec![5, 4, 3, 2, 1, 1, 2, 3, 4, 5],
        (0..500).map(|i| (i * 7919) % 97 * 0x40).collect(),
    ] {
        let mut deduped = values.clone();
        let mut positions = Vec::with_capacity(values.len());
        let capacity = positions.capacity();
        dedup_first_seen(&mut deduped, &mut positions);
        assert_eq!(deduped, first_seen(&values));
        assert_eq!(positions.capacity(), capacity);
    }
}

#[test]
fn owned_supremacy_and_sunk_failures_are_table_local() {
    let mut captured = owned_failure_fixture();
    captured.raw_mode = true;
    captured.supremacy_raw_mode = true;
    captured.sunk_raw_mode = true;
    captured.supremacy.outcome = Outcome::Rows { len: 1 };
    captured.sunk.outcome = Outcome::Rows { len: 1 };
    captured.convoys.outcome = Outcome::Rows { len: 1 };
    captured.sunk_names.len = 4;
    captured.sunk_names.required = 9;
    let bytes = captured.allocated_bytes();
    captured.fail_supremacy_capture("supremacy worker failed");
    assert_eq!(
        captured.supremacy.outcome,
        Outcome::Unavailable {
            reason: "supremacy worker failed"
        }
    );
    assert!(!captured.supremacy_raw_mode);
    assert!(captured.sunk_raw_mode && captured.raw_mode);
    assert_eq!(captured.sunk.outcome, Outcome::Rows { len: 1 });
    captured.fail_sunk_capture("sunk worker failed");
    assert_eq!(
        captured.sunk.outcome,
        Outcome::Unavailable {
            reason: "sunk worker failed"
        }
    );
    assert_eq!(
        (captured.sunk_names.len, captured.sunk_names.required),
        (0, 0)
    );
    assert!(!captured.sunk_raw_mode && captured.raw_mode);
    assert_eq!(captured.convoys.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.allocated_bytes(), bytes);
}

#[test]
fn owned_naval_failure_preserves_independent_sources_and_other_families() {
    let mut captured = owned_failure_fixture();
    captured.raw_mode = true;
    captured.supremacy_raw_mode = true;
    captured.sunk_raw_mode = true;
    captured.supremacy.outcome = Outcome::Rows { len: 1 };
    captured.convoys.outcome = Outcome::Rows { len: 1 };
    captured.cooldowns.outcome = Outcome::Rows { len: 1 };
    captured.edges.outcome = Outcome::Rows { len: 1 };
    captured.results.outcome = Outcome::Rows { len: 1 };
    captured.result_members.outcome = Outcome::Rows { len: 1 };
    captured.result_countries.outcome = Outcome::Rows { len: 1 };
    captured.sunk.outcome = Outcome::Rows { len: 1 };
    captured.orders_sources.outcome = Outcome::Rows { len: 1 };
    captured.wing_sources.outcome = Outcome::Rows { len: 1 };
    captured.orders_finished = true;
    captured.wings_finished = true;
    captured.source_ticks = [17, 23, 29];
    captured.theatre_names.len = 4;
    captured.theatre_names.required = 9;
    let mut outcomes = captured.outcomes();
    let bytes = captured.allocated_bytes();
    captured.fail_naval_capture("naval worker failed");
    for index in [4, 5, 6, 7, 8, 9, 14, 15, 16] {
        outcomes[index] = Outcome::Unavailable {
            reason: "naval worker failed",
        };
    }
    assert_eq!(captured.outcomes(), outcomes);
    assert_eq!(captured.orders_sources.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.wing_sources.outcome, Outcome::Rows { len: 1 });
    assert!(captured.orders_finished);
    assert!(captured.wings_finished);
    assert_eq!(captured.source_ticks, [17, 23, 29]);
    assert_eq!(captured.theatre_names.len, 0);
    assert_eq!(captured.theatre_names.required, 0);
    assert!(!captured.raw_mode);
    assert!(captured.supremacy_raw_mode && captured.sunk_raw_mode);
    assert_eq!(captured.allocated_bytes(), bytes);
}

#[test]
fn naval_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
    use collector_primitives::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
    static LOCAL: Tracker = Tracker::new();
    let mut job = NavalJob::default();
    job.wing_sources.reserve_exact(3);
    job.capacity_charge = Charge::empty_on(&LOCAL);
    job.update_capacity_charge();
    let expected = |job: &NavalJob| vector_bytes(&job.wing_sources);
    let initial = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(initial, expected(&job));
    job.wing_outcome = Some(Outcome::Truncated {
        len: 3,
        unread_rows: 5,
    });
    job.grow();
    let grown = LOCAL.snapshot().current.auxiliary_vectors;
    assert!(grown > initial);
    assert_eq!(grown, expected(&job));
    job.clear();
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}

#[test]
fn first_hour_naval_memberships_handoff_all_wings() {
    use crate::unit_fixture as fixture;
    use collector_air::extended::air;
    const WINGS: usize = 12;
    let world = fixture::build(fixture::Shape {
        countries: 4,
        divisions: 0,
        wings: WINGS,
        planes: WINGS,
        ships: 0,
    });
    let mut wings = air::AirWingJob::default();
    let mut naval = NavalJob::default();
    naval.wing_sources.reserve_exact(1);
    let context = air::AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: world.base,
        game_state: world.state,
        limits: [1; 3],
        game_score: false,
    };
    assert!(air::copy_air_refresh(&world.live(), context, &mut wings).raw_ok);
    assert_eq!(wings.naval_membership_outcome, Outcome::Rows { len: WINGS });
    let original_rows = wings.naval_memberships[..WINGS].to_vec();
    let prepared = wings.naval_memberships.as_ptr();
    let parked = naval.wing_sources.as_ptr();
    naval.install_wings(&mut wings);
    assert_eq!(naval.wing_sources.as_ptr(), prepared);
    assert_eq!(wings.naval_memberships.as_ptr(), parked);
    assert_eq!(naval.wing_outcome, Some(Outcome::Rows { len: WINGS }));
    assert_eq!(naval.wing_sources, original_rows);
    assert_eq!(naval.wing_required, WINGS);

    let mut captured = Captured::with_capacities(&Capacities {
        wing_sources: 1,
        ..Default::default()
    });
    let parked = captured.wing_sources.rows.as_ptr();
    captured.install_owned_wings(&mut naval);
    assert_eq!(captured.wing_sources.rows.as_ptr(), prepared);
    assert_eq!(naval.wing_sources.as_ptr(), parked);
    assert_eq!(captured.wing_sources.outcome, Outcome::Rows { len: WINGS });
    assert_eq!(captured.wing_sources.rows[..WINGS], original_rows);
    assert!(captured.wings_finished);
}

#[test]
fn naval_decode_grows_shared_memberships_from_the_original_census() {
    use super::naval_test_kit as kit;
    use collector_air::extended::air;
    use engine_native_air::{cair_wing, cstrategic_air};
    use engine_native_parts::cair_wing_pool;
    let mut bytes = kit::fixture();
    let pointer = |bytes: &mut [u8], address: usize, value: usize| {
        bytes[address..address + 8].copy_from_slice(&(value as u64).to_le_bytes());
    };
    let array = |bytes: &mut [u8], address: usize, data: usize, count: u32| {
        pointer(bytes, address, data);
        bytes[address + 8..address + 12].copy_from_slice(&count.to_le_bytes());
        bytes[address + 12..address + 16].copy_from_slice(&count.to_le_bytes());
    };
    const OWNER: usize = 0x70000;
    const POOL: usize = 0x71000;
    const WINGS: usize = 0x72000;
    pointer(&mut bytes, 0x41008, OWNER);
    pointer(&mut bytes, OWNER, cstrategic_air::VTABLE_RVA.0);
    bytes[OWNER + cstrategic_air::COUNTRY..OWNER + cstrategic_air::COUNTRY + 4]
        .copy_from_slice(&1_u32.to_le_bytes());
    array(&mut bytes, OWNER + cstrategic_air::POOLS, 0x72100, 1);
    pointer(&mut bytes, 0x72100, POOL);
    pointer(&mut bytes, POOL, cair_wing_pool::VTABLE_RVA.0);
    pointer(&mut bytes, POOL + cair_wing_pool::OWNER, OWNER);
    array(&mut bytes, POOL + cair_wing_pool::WINGS, WINGS, 2);
    for (ordinal, wing) in [0x73000, 0x75000].into_iter().enumerate() {
        pointer(&mut bytes, WINGS + ordinal * 8, wing);
        pointer(&mut bytes, wing, engine_native_parts::cair_wing_layout::VTABLE_RVA.0);
        pointer(
            &mut bytes,
            wing + engine_native_parts::cair_wing_layout::SECONDARY,
            engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA.0,
        );
        pointer(
            &mut bytes,
            wing + engine_native_parts::cair_wing_layout::REFERENCE_TYPE,
            ((ordinal + 1) << 32) | engine_native_parts::cair_wing_layout::TYPE as usize,
        );
        pointer(&mut bytes, wing + engine_native_parts::cair_wing_layout::POOL, POOL);
    }
    let memory = kit::memory(&bytes);
    let mut wings = air::AirWingJob::default();
    assert!(
        air::copy_air_refresh(
            &memory,
            air::AirWingCopyContext {
                hour: 0,
                force_full: true,
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                limits: [1; 3],
                game_score: false,
            },
            &mut wings,
        )
        .raw_ok
    );
    let mut job = NavalJob::fixture();
    job.install_wings(&mut wings);
    assert_eq!(job.wing_outcome, Some(Outcome::Rows { len: 2 }));
    assert!(
        super::copy_naval_refresh(
            &memory,
            NavalCopyContext {
                hour: 0,
                force_full: true,
                base: kit::BASE,
                game_state: kit::GAME_STATE,
            },
            &mut job,
        )
        .raw_ok
    );
    let mut captured = Captured::with_capacities(&Capacities {
        wing_sources: 1,
        ..Default::default()
    });
    super::decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
    assert_eq!(captured.wing_sources.outcome, Outcome::Rows { len: 2 });
    assert_eq!(captured.wing_sources.rows[..2], job.wing_sources);
}

#[test]
fn a_copy_whose_wing_census_follows_it_decodes_as_one_given_it_first() {
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let context = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    let decode = |job: &NavalJob| {
        let mut captured = Captured::with_capacities(&Capacities::fixture());
        super::decode_naval_payload(&mut captured, kit::BASE, job, &mut [0; NAVAL_PACK_N]);
        (
            kit::decoded(&captured),
            captured.wing_sources.outcome,
            captured.wing_sources.rows.clone(),
        )
    };
    let mut first = NavalJob::fixture();
    kit::stage_wings(&memory, &mut first);
    assert!(super::copy_naval_refresh(&memory, context, &mut first).raw_ok);

    let mut following = NavalJob::fixture();
    let mut wings = kit::wing_census(&memory);
    following.defer_wings();
    let report = super::copy_naval_refresh(&memory, context, &mut following);
    assert!(report.raw_ok, "the copy reads no wing value");
    assert!(following.theatres_active);
    let (_, stands) = following.install_wings_after(Some(&mut wings));
    assert!(stands);
    assert!(following.theatres_active, "the copy's cadence is kept");
    assert_eq!(following.wing_outcome, first.wing_outcome);
    assert_eq!(following.log.spans.len(), first.log.spans.len());
    assert_eq!(decode(&following), decode(&first));

    let mut refused = NavalJob::fixture();
    refused.defer_wings();
    assert!(super::copy_naval_refresh(&memory, context, &mut refused).raw_ok);
    assert_eq!(refused.install_wings_after(None), (0, false));
    assert!(!refused.positive_raw_complete);
    let mut unstaged = NavalJob::fixture();
    assert!(!super::copy_naval_refresh(&memory, context, &mut unstaged).raw_ok);
}

#[test]
fn naval_decode_grows_shared_memberships_past_the_frame_capacity() {
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let mut job = NavalJob::fixture();
    kit::stage_wings(&memory, &mut job);
    let context = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    assert!(super::copy_naval_refresh(&memory, context, &mut job).raw_ok);
    job.wing_sources.resize(2, Default::default());
    let capacity = Capacities {
        wing_sources: 1,
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    for missing in [0, 3] {
        job.wing_outcome = Some(if missing == 0 {
            Outcome::Rows { len: 2 }
        } else {
            Outcome::Truncated {
                len: 2,
                unread_rows: missing,
            }
        });
        super::decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
        assert_eq!(Some(captured.wing_sources.outcome), job.wing_outcome);
        assert!(captured.wing_sources.rows.len() >= 2);
    }
}

#[test]
fn shared_wing_fallback_charges_acquisition_and_installation() {
    use collector_air::extended::air::{AirWingCopyContext, AirWingJob, wing_test_kit as kit};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let mut wings = AirWingJob::fixture();
    let context = AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,

        limits: [2, 3, 2],
        game_score: false,
    };
    assert!(collector_air::extended::air::copy_air_refresh(&memory, context, &mut wings).raw_ok);
    wings.membership_ticks = 101;
    let mut captured = small();
    captured.add_wing_source_ticks(17);
    assert_eq!(captured.source_ticks[1], 17);
    captured.install_map_wings(&wings);
    assert!(captured.source_ticks[1] >= 118);
    assert_eq!(captured.wing_sources.outcome, Outcome::Rows { len: 2 });
    let first_ticks = captured.source_ticks[1];
    captured.add_wing_source_ticks(23);
    assert_eq!(captured.source_ticks[1], first_ticks + 23);
}

fn small() -> Captured {
    Captured::with_capacities(&Capacities {
        rows: [2; N],
        theatre_names: 32,
        sunk_names: 32,
        orders_sources: 2,
        wing_sources: 2,
        roster_scratch: 8,
        rail_levels: 0,
    })
}

#[test]
fn primary_membership_rejects_omissions_duplicates_and_incomplete_censuses() {
    let link = |row: &(u64, u64)| *row;
    let first_id = 0x1234_5678_0000_0035;
    let second_id = 0x9abc_def0_0000_0035;
    let group = 0xfedc_ba98_0000_004e;
    let rows = [(second_id, group), (first_id, group)];
    let sources = [(first_id, group), (42, 0), (second_id, group)];
    let full = Outcome::Rows { len: 2 };
    let census = Outcome::Rows { len: 3 };
    assert!(membership_issue(full, &rows, link, census, &sources, link).is_none());
    for bad in [
        vec![(first_id, group)],
        vec![(first_id, group), (first_id, group)],
        vec![(first_id, group), (second_id, group ^ (1_u64 << 32))],
    ] {
        let outcome = Outcome::Rows { len: bad.len() };
        assert!(membership_issue(outcome, &bad, link, census, &sources, link).is_some());
    }
    let partial = Outcome::Truncated {
        len: 2,
        unread_rows: 1,
    };
    assert!(membership_issue(full, &rows, link, partial, &sources, link).is_some());
    assert!(
        membership_issue(full, &rows, link, Outcome::Rows { len: 4 }, &sources, link).is_some()
    );
}

#[test]
fn primary_name_ranges_preserve_bytes_and_refuse_invalid_extents() {
    let mut rows = CaptureRows::<ctheater_group::Row>::new(1);
    rows.outcome = Outcome::Rows { len: 1 };
    rows.rows[0] = ctheater_group::Row {
        id: u64::MAX,
        priority: -1,
        name_start: 0,
        name_len: 2,
        ..Default::default()
    };
    let name = |row: &ctheater_group::Row| (row.name_start, row.name_len);
    assert!(valid_named(rows.outcome, &rows.rows, Some(b"ab"), name));
    assert!(!valid_named(rows.outcome, &rows.rows, Some(b"a"), name));
    rows.rows[0].name_start = usize::MAX;
    assert!(!valid_named(rows.outcome, &rows.rows, Some(b"ab"), name));
}

#[test]
fn sunk_history_preserves_duplicate_records_and_never_publishes_a_prefix() {
    let mut captured = small();
    captured.sunk.outcome = Outcome::Rows { len: 2 };
    captured.sunk_names.first[..2].copy_from_slice(&[0xff, 0]);
    captured.sunk_names.len = 2;
    captured.sunk_names.required = 2;
    captured.sunk.rows[0].ship_name = csunk_ship_info::TextRange { start: 0, len: 2 };
    captured.sunk.rows[0].victim_equipment_variant = u64::MAX;
    captured.sunk.rows[1] = captured.sunk.rows[0];
    let TableView::Complete(TableRows::SunkShips(rows)) = captured.sunk_table() else {
        panic!("complete sequence expected");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entries.len(), 2);
    assert_eq!(rows[0].entries[0], rows[0].entries[1]);
    assert_eq!(rows[0].entries[0].ship_name, [0xff, 0]);
    captured.sunk.outcome = Outcome::Truncated {
        len: 2,
        unread_rows: 1,
    };
    assert_eq!(
        captured.outcomes()[13],
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(
        captured.sunk_table(),
        TableView::Truncated {
            rows: TableRows::SunkShips(Vec::new()),
            missing: 1
        }
    );
    captured.sunk.outcome = Outcome::Rows { len: 2 };
    captured.sunk.rows[0].ship_name.start = usize::MAX;
    assert!(matches!(
        captured.sunk_table(),
        TableView::Unavailable { .. }
    ));
}

#[test]
fn hourly_land_theatre_headers_need_no_daily_details_or_wing_storage() {
    use super::naval_test_kit as kit;
    struct NoTheatreDetails<'a>(engine_native_memory::memory::SliceMemory<'a>);
    impl Memory for NoTheatreDetails<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            assert_ne!(address, kit::THEATRE + ctheatre::THEATER_GROUP_DATA);
            assert_ne!(address, kit::C0 + engine_native::ccountry::AIR_THEATRES);
            self.0.copy(address, bytes)
        }
    }
    let bytes = kit::fixture();
    let expected = kit::direct(&kit::memory(&bytes));
    let memory = NoTheatreDetails(kit::memory(&bytes));
    let mut job = NavalJob::fixture();
    job.wing_sources = Vec::new();
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [8; N],
        theatre_names: 0,
        sunk_names: 0,
        orders_sources: 0,
        wing_sources: 0,
        roster_scratch: 0,
        rail_levels: 0,
    });
    for hour in (0..24).filter(|hour| *hour != 13) {
        job.clear();
        captured.prepare_hour(hour, false);
        let copy = copy_naval_refresh(
            &memory,
            NavalCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
            },
            &mut job,
        );
        assert!(copy.raw_ok && !job.grow_for_next());
        let mut ticks = [0; NAVAL_PACK_N];
        decode_naval_payload(&mut captured, kit::BASE, &job, &mut ticks);
        assert_eq!(captured.land.outcome, expected.land_outcome);
        assert_eq!(
            &captured.land.rows[..captured.land.outcome.len()],
            &expected.land[..expected.land_outcome.len()]
        );
        assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
        assert_eq!(captured.wing_sources.outcome, SKIPPED);
        assert_eq!(captured.groups.outcome, expected.oob_groups_outcome);
        assert_eq!(captured.group_members.outcome, expected.oob_members_outcome);
        assert_eq!(
            captured.group_children.outcome,
            expected.oob_children_outcome
        );
        let len = captured.groups.outcome.len();
        assert_eq!(&captured.groups.rows[..len], &expected.oob_groups[..len]);
        captured.fail_naval_capture("worker failed");
        assert_eq!(
            captured.land.outcome,
            Outcome::Unavailable {
                reason: "worker failed"
            }
        );
        assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
        assert_eq!(
            captured.groups.outcome,
            Outcome::Unavailable {
                reason: "worker failed"
            }
        );
    }
}
#[test]
fn roster_land_theatres_match_the_census_copy_and_the_typed_walk() {
    use super::naval_test_kit as kit;
    use engine_native::{ccountry, cgame_state};
    use engine_native_memory::memory::SliceMemory;
    use engine_native_intelligence::country_memory::Cache;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: Option<usize>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .fault
                .is_some_and(|fault| (address..address + output.len()).contains(&fault))
            {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    let owner = kit::THEATRE + ctheatre::OWNER;
    let mut base = kit::fixture();
    put(
        &mut base,
        kit::GAME_STATE + cgame_state::COUNTRIES + 8,
        &[4, 0, 0, 0, 4, 0, 0, 0],
    );
    put(
        &mut base,
        kit::GAME_STATE + cgame_state::COUNTRY_LINKS + 8,
        &[4, 0, 0, 0, 4, 0, 0, 0],
    );
    put(&mut base, LTAB + 12, &1_u32.to_le_bytes());
    put(
        &mut base,
        0x40000 + engine_native_air::cstrategic_air_manager::COUNTRY_AIR + 8,
        &[4, 0, 0, 0, 4, 0, 0, 0],
    );
    let mut variants: Vec<(&str, Vec<u8>, Option<usize>)> = vec![("clean", base.clone(), None)];
    for (name, value) in [
        ("alias owner", 2_u32),
        ("null-slot owner", 3),
        ("zero owner", 0),
        ("owner past links", 9),
    ] {
        let mut bytes = base.clone();
        put(&mut bytes, owner, &value.to_le_bytes());
        variants.push((name, bytes, None));
    }
    let mut torn = base.clone();
    put(&mut torn, LTAB + 8, &0_u32.to_le_bytes());
    variants.push(("torn census", torn, None));
    let mut vtable = base.clone();
    vtable[kit::THEATRE] ^= 0x10;
    variants.push(("theatre vtable", vtable, None));
    for (offset, len) in ctheatre::HEADER_SPANS {
        for at in [kit::THEATRE + offset, kit::THEATRE + offset + len - 1] {
            variants.push(("header fault", base.clone(), Some(at)));
        }
    }
    for at in [
        kit::C0 + ccountry::THEATRES,
        0x36000,
        CTAB + 8,
        kit::C0 + ccountry::ID,
    ] {
        variants.push(("country fault", base.clone(), Some(at)));
    }
    let hourly = |memory: &dyn Fn(&mut NavalJob) -> NavalCopyReport, job: &mut NavalJob| {
        let copy = memory(job);
        assert!(copy.raw_ok);
        let mut captured = Captured::with_capacities(&Capacities::fixture());
        captured.prepare_hour(1, false);
        decode_naval_payload(&mut captured, kit::BASE, job, &mut [0; NAVAL_PACK_N]);
        let len = captured.land.outcome.len();
        (captured.land.outcome, captured.land.rows[..len].to_vec())
    };
    let context = NavalCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 1,
        force_full: false,
    };
    let mut roster_used = 0;
    for (name, bytes, fault) in &variants {
        let memory = Fault {
            memory: kit::memory(bytes),
            fault: *fault,
        };
        let mut cache = Cache::try_with_capacity(Cache::cells_for_countries(4, 64)).unwrap();
        let _ = cache.prepare(&memory, kit::BASE, kit::GAME_STATE);
        let roster = cache.roster_view(&memory);
        let has_roster = roster
            .authenticated_countries(kit::BASE, kit::GAME_STATE)
            .is_some();
        roster_used += usize::from(has_roster);
        let case = format!("{name} {fault:x?} roster={has_roster}");

        let direct = kit::direct(&memory);
        kit::assert_matches_direct(&kit::direct(&roster), &direct);
        let (census, ..) = kit::split(&memory);
        kit::assert_matches_direct(&census, &direct);
        let (split, _, job) = kit::split(&roster);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(
            job.roster.roster(kit::BASE, kit::GAME_STATE).is_some(),
            has_roster,
            "{case}"
        );

        let mut plain = NavalJob::fixture();
        let expected = hourly(&|job| copy_naval_refresh(&memory, context, job), &mut plain);
        assert_eq!(
            (expected.0, &expected.1[..]),
            (
                direct.land_outcome,
                &direct.land[..direct.land_outcome.len()]
            ),
            "{case}"
        );
        let mut job = NavalJob::fixture();
        let actual = hourly(&|job| copy_naval_refresh(&roster, context, job), &mut job);
        assert_eq!(actual, expected, "{case}");
        if has_roster {
            let spans: Vec<_> = job.log.spans.iter().map(|span| span.addr).collect();
            for address in [
                kit::GAME_STATE + cgame_state::COUNTRIES,
                kit::GAME_STATE + cgame_state::COUNTRY_LINKS,
                CTAB + 8,
                CTAB + 16,
                kit::C0,
                kit::C0 + ccountry::ID,
                LTAB + 4,
                LTAB + 8,
            ] {
                assert!(!spans.contains(&address), "{case}: {address:#x}");
            }
            assert!(spans.len() < plain.log.spans.len(), "{case}");
            if *name == "null-slot owner" {
                assert!(spans.contains(&(LTAB + 12)), "{case}");
            }
        }
    }
    assert!(roster_used >= variants.len() - 3, "{roster_used}");
}

#[test]
fn hourly_land_theatre_owner_fault_does_not_publish_a_header() {
    use super::naval_test_kit as kit;
    let mut bytes = kit::fixture();
    bytes[kit::THEATRE + ctheatre::OWNER..kit::THEATRE + ctheatre::OWNER + 4].fill(0);
    let memory = kit::memory(&bytes);
    let mut job = NavalJob::fixture();
    let copy = copy_naval_refresh(
        &memory,
        NavalCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: 1,
            force_full: false,
        },
        &mut job,
    );
    assert!(copy.raw_ok);
    let mut captured = Captured::with_capacities(&Capacities::fixture());
    captured.prepare_hour(1, false);
    decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
    assert!(matches!(captured.land.outcome, Outcome::Unavailable { .. }));
    assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
}

#[test]
fn supremacy_and_sunk_offslot_jobs_skip_reads_and_decoding() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            panic!("inactive naval read at {address:#x}");
        }
    }
    let mut supremacy = SupremacyJob::default();
    let mut sunk = SunkJob::default();
    let mut captured = owned_failure_fixture();
    for hour in 0..24 {
        captured.prepare_hour(hour, false);
        if !replay_data::cadence::is_refresh(replay_data::rows::TableId::NavalSupremacy, hour) {
            let report = copy_supremacy_refresh(
                &NoReads,
                SupremacyCopyContext {
                    base: 0,
                    game_state: 1,
                    hour,
                    force_full: false,
                },
                &mut supremacy,
            );
            assert!(!report.raw_ok && !supremacy.grow_for_next());
            assert_eq!(report.copy_ticks, [0]);
            captured.fail_supremacy_capture("worker failed");
            let mut ticks = [0];
            captured.decode_supremacy_payload(0, &supremacy, &mut ticks);
            assert_eq!((captured.supremacy.outcome, ticks), (SKIPPED, [0]));
        }
        if !replay_data::cadence::is_refresh(replay_data::rows::TableId::SunkShips, hour) {
            let report = copy_sunk_refresh(
                &NoReads,
                SunkCopyContext {
                    base: 0,
                    game_state: 1,
                    hour,
                    force_full: false,
                },
                &mut sunk,
            );
            assert!(!report.raw_ok && !sunk.grow_for_next());
            assert_eq!(report.copy_ticks, [0]);
            captured.fail_sunk_capture("worker failed");
            let mut ticks = [0];
            captured.decode_sunk_payload(0, &sunk, &mut ticks);
            assert_eq!((captured.sunk.outcome, ticks), (SKIPPED, [0]));
        }
    }
}

#[test]
fn naval_split_copy_matches_direct_walk() {
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.land_outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.members_outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.groups_outcome, Outcome::Rows { len: 1 });
}

#[test]
fn naval_truncation_matrix_matches_direct_with_same_caps() {
    use super::Capacities;
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for rows in [[0; 17], [1; 17], [2; 17], [64; 17]] {
        let capacity = Capacities {
            rows,
            theatre_names: 1024,
            sunk_names: 1024,
            orders_sources: 64,
            wing_sources: 64,
            roster_scratch: 64,
            rail_levels: 0,
        };
        let direct = kit::direct_with(&memory, &capacity);
        let (split, report, _) = kit::split_with(&memory, &capacity);
        assert!(report.raw_ok, "rows={rows:?}");
        kit::assert_matches_direct(&split, &direct);
    }
    let capacity = Capacities {
        rows: [2; 17],
        theatre_names: 1024,
        sunk_names: 1024,
        orders_sources: 64,
        wing_sources: 64,
        roster_scratch: 64,
        rail_levels: 0,
    };
    let (split, _, _) = kit::split_with(&memory, &capacity);
    assert_eq!(split.land_outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.members_outcome, Outcome::Rows { len: 2 });
}

#[test]
fn naval_split_copy_matches_direct_walk_under_truncation() {
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct_truncated(&memory);
    let (split, report, _) = kit::split_truncated(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
}

#[test]
fn naval_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::naval_test_kit as kit;
    use engine_native_memory::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native_memory::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine_native::cgame_state::COUNTRIES,
        kit::THEATRE,
        kit::GROUP,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn supremacy_split_copy_matches_direct_walk() {
    use super::supremacy_test_kit as kit;
    use engine_native_memory::memory::{CountedMemory, ReadCounts};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let memory = CountedMemory::new(memory);
    let capture = read_accounting::Capture::new();
    let (split, report, mut job) = kit::split(&memory);
    let counts = capture.finish();
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.rows[0].country, u32::MAX);
    assert_eq!(split.rows[0].current, 1000);
    assert_eq!(split.rows[1].country, 1);
    let anchor = TableId::NavalSupremacy;
    let mut expected = read_accounting::ZERO;
    expected[anchor.index()] = ReadCounts {
        copy_calls: 9,
        spans: 9,
        bytes: 260,
        ..ReadCounts::ZERO
    };
    assert_eq!(counts, expected);
    let mut refreshes = 0;
    for hour in 0..24 {
        if replay_data::cadence::is_refresh(anchor, hour) {
            refreshes += 1;
            continue;
        }
        let capture = read_accounting::Capture::new();
        let skipped = copy_supremacy_refresh(
            &memory,
            SupremacyCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
            },
            &mut job,
        );
        assert_eq!(capture.finish(), read_accounting::ZERO);
        assert!(!skipped.raw_ok && !job.grow_for_next());
        assert_eq!(skipped.copy_ticks, [0]);
    }
    assert_eq!(refreshes, 1);
    let measured = counts[anchor.index()];
    let per_refresh = [
        measured.copy_calls,
        measured.scattered_calls,
        measured.record_calls,
        measured.plan_calls,
        measured.spans,
        measured.bytes,
    ];
    println!(
        "daily_fixture cohort=supremacy_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
        per_refresh.map(|value| value * 24)
    );
}

#[test]
fn supremacy_truncation_matrix_matches_direct_with_same_caps() {
    use super::supremacy_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for cap in [0, 1, 2, 64] {
        let direct = kit::direct_with(&memory, cap);
        let (split, report, _) = kit::split_with(&memory, cap);
        assert!(report.raw_ok, "cap={cap}");
        kit::assert_matches_direct(&split, &direct);
    }
    let direct = kit::direct_with(&memory, 2);
    assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn supremacy_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::supremacy_test_kit as kit;
    use engine_native_memory::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native_memory::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine_native::cgame_state::STRATEGIC_REGIONS,
        kit::REGION_OBJECT,
        kit::DOMINANCE,
        kit::DOMINANCE + engine_naval_history::cnaval_region_dominance::ENTRIES,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn sunk_split_copy_matches_direct_walk() {
    use super::sunk_test_kit as kit;
    use engine_native_memory::memory::{CountedMemory, ReadCounts};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let memory = CountedMemory::new(memory);
    let capture = read_accounting::Capture::new();
    let (split, report, mut job) = kit::split(&memory);
    let counts = capture.finish();
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.rows[0].victim_country, 3);
    assert_eq!(split.rows[0].province, 77);
    assert_eq!(split.rows[0].attribution_class_present, 1);
    assert_eq!(split.rows[1].victim_country, 5);
    assert_eq!(split.rows[1].province_present, 0);
    assert_eq!(split.rows[1].attribution_class_present, 0);
    let anchor = TableId::SunkShips;
    let mut expected = read_accounting::ZERO;
    expected[anchor.index()] = ReadCounts {
        copy_calls: 24,
        spans: 24,
        bytes: 585,
        ..ReadCounts::ZERO
    };
    assert_eq!(counts, expected);
    let mut refreshes = 0;
    for hour in kit::HOUR..kit::HOUR + 24 {
        if replay_data::cadence::is_refresh(anchor, hour) {
            refreshes += 1;
            continue;
        }
        let capture = read_accounting::Capture::new();
        let skipped = copy_sunk_refresh(
            &memory,
            SunkCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
            },
            &mut job,
        );
        assert_eq!(capture.finish(), read_accounting::ZERO);
        assert!(!skipped.raw_ok && !job.grow_for_next());
        assert_eq!(skipped.copy_ticks, [0]);
    }
    assert_eq!(refreshes, 1);
    let measured = counts[anchor.index()];
    let per_refresh = [
        measured.copy_calls,
        measured.scattered_calls,
        measured.record_calls,
        measured.plan_calls,
        measured.spans,
        measured.bytes,
    ];
    println!(
        "daily_fixture cohort=sunk_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
        per_refresh.map(|value| value * 24)
    );
}

#[test]
fn sunk_truncation_matrix_matches_direct_with_same_caps() {
    use super::sunk_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for (rows, names) in [(0, 0), (1, 16), (2, 64), (64, 4096)] {
        let direct = kit::direct_with(&memory, rows, names);
        let (split, report, _) = kit::split_with(&memory, rows, names);
        assert!(report.raw_ok, "rows={rows} names={names}");
        kit::assert_matches_direct(&split, &direct);
    }
    let direct = kit::direct_with(&memory, 2, 64);
    assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn sunk_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::sunk_test_kit as kit;
    use engine_native_memory::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native_memory::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine_native::cgame_state::SUNK_SHIP_HISTORY,
        kit::FIRST,
        kit::FIRST + engine_naval_history::csunk_ship_info::SINK_HOUR,
        kit::CLASS,
        kit::FIRST + engine_naval_history::csunk_ship_info::SHIP_NAME,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn growing_theatre_job_publishes_first_payload_from_empty_raw_and_typed_arenas() {
    let mut bytes = naval_test_kit::fixture();
    let command_name = 0x62000 + corders_group::NAME;
    bytes[command_name..command_name + 3].copy_from_slice(b"OOB");
    bytes[command_name + 16..command_name + 24].copy_from_slice(&3_u64.to_le_bytes());
    bytes[command_name + 24..command_name + 32].copy_from_slice(&15_u64.to_le_bytes());
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut job = NavalJob::new();
    naval_test_kit::stage_wings(&memory, &mut job);
    let report = copy_naval_refresh(
        &memory,
        NavalCopyContext {
            base: naval_test_kit::BASE,
            game_state: naval_test_kit::GAME_STATE,
            hour: 0,
            force_full: true,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    assert!(!job.log.overflowed);
    let mut captured = Captured::with_capacities(&Capacities::default());
    decode_naval_payload(
        &mut captured,
        naval_test_kit::BASE,
        &job,
        &mut [0; NAVAL_PACK_N],
    );
    assert_eq!(captured.land.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.land_groups.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.land_members.outcome, Outcome::Rows { len: 2 });
    assert_eq!(captured.groups.outcome, Outcome::Rows { len: 1 });
    assert!(captured.theatre_names.len > 0);
    assert!(captured.group_names.len > 0);
}

#[test]
fn attempted_root_census_failure_is_terminal_for_naval_copy_and_decode() {
    struct NoRegistry<'a> {
        inner: engine_native_memory::memory::SliceMemory<'a>,
        attempts: std::cell::Cell<usize>,
    }
    impl Memory for NoRegistry<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let root = engine_native_parts::creference_registry::LOW_ROOTS_RVA.0
                + corders_group::REFERENCE_TYPE as usize * 8;
            if address == root {
                self.attempts.set(self.attempts.get() + 1);
                return false;
            }
            self.inner.copy(address, output)
        }
    }
    let bytes = naval_test_kit::fixture();
    let memory = NoRegistry {
        inner: engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        attempts: Default::default(),
    };
    for rejected in [false, true] {
        let reason = if rejected {
            "root producer rejected"
        } else {
            "original root read failed"
        };
        let mut job = NavalJob::default();
        if rejected {
            job.fail_roots(reason);
        } else {
            job.capture_roots(&[], Outcome::Unavailable { reason });
        }
        naval_test_kit::stage_wings(&memory, &mut job);
        assert!(
            copy_naval_refresh(
                &memory,
                NavalCopyContext {
                    base: naval_test_kit::BASE,
                    game_state: naval_test_kit::GAME_STATE,
                    hour: 0,
                    force_full: true,
                },
                &mut job
            )
            .raw_ok
        );
        assert_eq!(memory.attempts.get(), 0);
        let mut captured = Captured::vacant();
        decode_naval_payload(
            &mut captured,
            naval_test_kit::BASE,
            &job,
            &mut [0; NAVAL_PACK_N],
        );
        for outcome in [
            captured.groups.outcome,
            captured.group_members.outcome,
            captured.group_children.outcome,
            captured.orders_sources.outcome,
        ] {
            assert_eq!(outcome, Outcome::Unavailable { reason });
        }
        assert_eq!(captured.land.outcome, Outcome::Rows { len: 1 });
        job.clear();
        assert_eq!(job.roots_outcome, None);
    }
}
