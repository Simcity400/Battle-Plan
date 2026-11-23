use super::*;
use engine_native_memory::memory::SliceMemory;

#[test]
fn projection_compares_every_model_field_and_preserves_encoded_boundaries() {
    use crate::projection::ProjectionScratch;
    use replay_data::rows::TableRows;
    use replay_frame::{frame::FrameEncoder, frame::FrameInput, frame::Metrics};

    let fixture = || {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 2,
            modifiers: SnapshotCapacity::default(),
        });
        let mut row = cship::ModelRow {
            ship: u64::MAX,
            variant: u64::MAX,
            model_number: i32::MIN,
            formation_flags: 1 << 63,
            ..Default::default()
        };
        row.hull_sprite.bytes[..3].copy_from_slice(&[65, 0, 255]);
        row.hull_sprite.len = 3;
        row.fallback_sprite.bytes[..2].copy_from_slice(&[255, 0]);
        row.fallback_sprite.len = 2;
        capture.rows.rows[0] = row.clone();
        row.ship = 51 | (1 << 32);
        capture.rows.rows[1] = row;
        capture.rows.outcome = Outcome::Rows { len: 2 };
        capture
    };
    let frame = |branch, hour, [table]: [TableInput; N]| {
        let mut frame = FrameInput {
            branch,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                crate::table_conversion::unavailable("fixture table").into()
            }),
        };
        frame.tables[TABLES[0].index()] = table;
        frame
    };
    for change in 0..34 {
        let mut current = fixture();
        match change {
            0 => {}
            1 => current.rows.rows[0].variant ^= 1,
            2 => current.rows.rows[0].hull_sprite.bytes[1] ^= 1,
            3 => current.rows.rows[0].fallback_sprite.bytes[0] ^= 1,
            4 => current.rows.rows[0].model_number ^= 1,
            5 => current.rows.rows[0].formation_flags ^= 1,
            6 => current.rows.rows[0].roster_ordinal = u32::MAX,
            7 => current.rows.rows[0].definition_valid = u8::MAX,
            8 => current.rows.rows[0].carrier_air_base = u64::MAX,
            9 => current.rows.rows[0].maximum_strength = i64::MIN,
            10 => current.rows.rows[0].maximum_organisation = i64::MAX,
            11 => current.rows.rows[0].positive_equipment_flags = 1 << 63,
            12 => current.rows.rows[0].hull_sprite.len = 0,
            13 => current.rows.rows[0].fallback_sprite.len = 0,
            14 => current.rows.rows.reverse(),
            15 => current.rows.rows[0].ship -= 1,
            16 => current.rows.rows[0].ship = current.rows.rows[1].ship,
            17 => current.rows.outcome = Outcome::Rows { len: 0 },
            18 => {
                current.rows.outcome = Outcome::Truncated {
                    len: 1,
                    unread_rows: 1,
                }
            }
            19 => {
                current.rows.outcome = Outcome::Unavailable {
                    reason: "native model fault",
                }
            }
            20 => {
                current.fail_owned_capture("owned model request failed");
            }
            21 => current.rows.outcome = Outcome::Rows { len: 3 },
            22 => {
                current.rows.outcome = Outcome::Truncated {
                    len: 1,
                    unread_rows: 0,
                }
            }
            23..=26 => {}
            27 => current.rows.rows[0].anti_air_base = i64::MIN,
            28 => current.rows.rows[0].subunit_index = Some(i32::MIN),
            29 => current.rows.rows[0].subunit_index = Some(0),
            30 => current.rows.rows[0].has_criticals = Some(0),
            31 => current.rows.rows[0].has_criticals = Some(1),
            32 => current.rows.rows[0].air_capacity_kind = Some(1),
            33 => current.rows.rows[0].air_capacity_kind = Some(2),
            _ => unreachable!(),
        }
        let mut baseline = frame(1, 100, fixture().table_inputs(None));
        if change == 23 {
            let TableInput::Sampled(TableView::Complete(TableRows::ShipModels(rows))) =
                &mut baseline.tables[TABLES[0].index()]
            else {
                panic!("complete fixture")
            };
            rows[1].counter = None;
        }
        let force = change == 24;
        let branch = if change == 25 { 2 } else { 1 };
        let hour = if change == 26 { 105 } else { 101 };
        let mut projected = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        projected.encode_input(baseline.clone(), false).unwrap();
        reference.encode_input(baseline, false).unwrap();
        let scratch = ProjectionScratch::default();
        let mut projection = Projection::new(&projected, branch, hour, force, &scratch);
        fixture().table_inputs(Some(&mut projection));
        let inputs = current.table_inputs(Some(&mut projection));
        if matches!(change, 0 | 14) {
            assert!(matches!(inputs[0], TableInput::ComparedEqual(_)));
        }
        if (1..=13).contains(&change) || matches!(change, 23 | 27..=29) {
            let TableInput::ComparedChanges { indices, .. } = &inputs[0] else {
                panic!("model field {change} must replace its row");
            };
            assert_eq!(indices, &[1]);
        }
        if matches!(change, 16 | 19..=22) {
            assert!(matches!(
                inputs[0],
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
        assert_eq!(
            projected
                .encode_input(frame(branch, hour, inputs), force)
                .unwrap(),
            reference
                .encode_input(frame(branch, hour, current.table_inputs(None)), force)
                .unwrap(),
            "change {change}",
        );
    }
}

#[test]
fn source_faults_capacity_and_empty_primary_census_remain_explicit() {
    let memory = SliceMemory {
        base: 0,
        bytes: &[],
    };
    let mut capacity = Capacities {
        rows: 0,
        modifiers: SnapshotCapacity::default(),
    };
    let mut capture = Captured::with_capacities(&capacity);
    capture.begin_sources();
    capture.visit_source(0, &cship::Row::default());
    capture.finish_sources(Outcome::Truncated {
        len: 1,
        unread_rows: 3,
    });
    capture.capture_owned(&memory, CaptureContext::new(0));
    assert_eq!(
        capture.outcomes(),
        [Outcome::Unavailable {
            reason: "ship model fields unreadable"
        }]
    );
    capture.grow(&mut capacity);
    assert!(capacity.rows >= 4);
    capture.reset();
    capture.begin_sources();
    capture.visit_source(0, &cship::Row::default());
    capture.finish_sources(Outcome::Rows { len: 1 });
    capture.capture_owned(&memory, CaptureContext::new(0));
    assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
    capture.reset();
    capture.begin_sources();
    capture.finish_sources(Outcome::Rows { len: 0 });
    capture.capture_owned(&memory, CaptureContext::new(0));
    assert_eq!(capture.outcomes(), [Outcome::Rows { len: 0 }]);
    assert!(matches!(capture.tables()[0], TableView::Complete(_)));
}
#[test]
fn owned_seed_handoff_preserves_order_demand_and_clears_callback_pointers() {
    let mut capacity = Capacities {
        rows: 2,
        modifiers: SnapshotCapacity::default(),
    };
    let mut capture = Captured::with_capacities(&capacity);
    capture.begin_sources();
    for (pointer, id) in [(0x1100, 11_u64), (0x2200, 22), (0x3300, 33)] {
        capture.visit_source(
            pointer,
            &cship::Row {
                id,
                ..Default::default()
            },
        );
    }
    capture.finish_sources(Outcome::Rows { len: 3 });
    assert_eq!(capture.seed_len, 3);
    assert_eq!(
        capture.seeds[..3]
            .iter()
            .map(|s| (s.pointer, s.row.id))
            .collect::<Vec<_>>(),
        [(0x1100, 11), (0x2200, 22), (0x3300, 33)]
    );
    assert_eq!(capture.required_rows, 3);
    let allocated = capture.allocated_bytes();
    assert_eq!(
        capture
            .fail_owned_capture("owned ShipModels job failed")
            .ticks,
        [0]
    );
    assert!(
        capture.seeds[..capture.seed_len]
            .iter()
            .all(|s| s.pointer == 0)
    );
    assert_eq!(
        capture.seeds[..2]
            .iter()
            .map(|s| s.row.id)
            .collect::<Vec<_>>(),
        [11, 22]
    );
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("accepted failure must not retry cleared sources")
        }
    }
    capture.capture_owned(&NoRead, CaptureContext::new(0));
    assert_eq!(
        capture.outcomes(),
        [Outcome::Unavailable {
            reason: "owned ShipModels job failed"
        }]
    );
    capture.grow(&mut capacity);
    assert!(capacity.rows >= 3);
    assert!(capture.allocated_bytes() >= allocated);
    capture.reset();
    assert_eq!(capture.seed_len, 0);
    assert_eq!(capture.required_rows, 0);
    capture.begin_sources();
    capture.visit_source(0x6600, &cship::Row::default());
    assert_eq!(capture.seeds[0].pointer, 0x6600);
    capture.reset();
    assert_eq!(capture.seeds[0].pointer, 0);
}
fn census(ships: u64, rows: usize) -> Captured {
    let mut capture = Captured::with_capacities(&Capacities {
        rows,
        modifiers: SnapshotCapacity::default(),
    });
    capture.begin_sources();
    for id in 1..=ships {
        capture.visit_source(
            0x1000 * id as usize,
            &cship::Row {
                id,
                roster_ordinal: id as u32 * 3,
                ..Default::default()
            },
        );
    }
    capture.finish_sources(Outcome::Rows {
        len: ships as usize,
    });
    capture
}

fn model(seed: &Seed) -> cship::ModelRow {
    let id = seed.row.id;
    let mut row = cship::ModelRow {
        ship: id,
        variant: id.rotate_left(17) | 1,
        model_number: -(id as i32),
        formation_flags: id << 7,
        roster_ordinal: seed.row.roster_ordinal,
        definition_valid: (id % 2) as u8,
        carrier_air_base: id.wrapping_mul(0x9e37_79b9),
        maximum_strength: id as i64 * 1_000,
        maximum_organisation: -(id as i64) * 7,
        positive_equipment_flags: !id,
        ..Default::default()
    };
    let hull = format!("hull_{id}");
    row.hull_sprite.bytes[..hull.len()].copy_from_slice(hull.as_bytes());
    row.hull_sprite.len = hull.len();
    row.fallback_sprite.bytes[..2].copy_from_slice(&[0xff, id as u8]);
    row.fallback_sprite.len = 2;
    row
}

fn run(capture: &mut Captured, fail: Option<u64>) -> CaptureReport {
    capture.capture_with(
        |this, seed| {
            this.record(|_| match fail {
                Some(id) if id == seed.row.id => Err("synthetic ship read failure"),
                _ => Ok(model(&seed)),
            })
        },
        |_| {},
    )
}

fn captured(ships: u64, parts: usize, fail: Option<u64>) -> (Outcome, [TableView; N]) {
    let mut capture = census(ships, (ships as usize).max(1));
    assert_eq!(capture.split_details(parts), parts);
    let mut lent: Vec<_> = (0..parts - 1)
        .map(|index| capture.take_part(index))
        .collect();
    run(&mut capture, fail);
    for part in &mut lent {
        run(part, fail);
    }
    for (index, part) in lent.into_iter().enumerate() {
        capture.restore_part(index, part);
    }
    capture.finish_split();
    (capture.outcomes()[0], capture.tables())
}

#[test]
fn parts_follow_the_live_ship_count_and_every_range_fits() {
    assert_eq!(parts_for(0), 1);
    assert_eq!(parts_for(512), 1);
    assert_eq!(parts_for(513), 2);
    assert_eq!(parts_for(2_000), 4);
    assert_eq!(parts_for(100_000), MAX_PARTS);
    for rows in [
        1_usize, 511, 512, 513, 2_048, 2_049, 3_072, 3_073, 4_096, 10_000, 16_384,
    ] {
        for ships in [1, rows / 2, rows.saturating_sub(1), rows] {
            let parts = parts_for(ships);
            assert!(
                ships.div_ceil(parts) <= part_rows(rows),
                "{ships} of {rows}"
            );
        }
    }
}

#[test]
fn split_outcomes_and_rows_equal_the_unsplit_walk_at_every_boundary() {
    for ships in [511_u64, 512, 513, 1024, 1025, 1536, 1537, 2048, 2049] {
        let parts = parts_for(ships as usize);
        let unsplit = captured(ships, 1, None);
        assert_eq!(
            unsplit.0,
            Outcome::Rows {
                len: ships as usize
            }
        );
        assert_eq!(captured(ships, parts, None), unsplit, "{ships} ships");
        for fail in [1, ships / 2 + 1, ships] {
            assert_eq!(
                captured(ships, parts, Some(fail)),
                captured(ships, 1, Some(fail)),
                "{ships} ships, failing {fail}"
            );
        }
    }
    let [TableView::Complete(replay_data::rows::TableRows::ShipModels(rows))] =
        captured(2049, 4, None).1
    else {
        panic!("complete")
    };
    assert_eq!(rows.len(), 2049);
    assert!(rows.iter().zip(1..).all(|(row, id)| row.ship == id));
    assert_eq!(rows[1500].hull_sprite, b"hull_1501");
}

#[test]
fn families_of_ten_thousand_ships_grow_their_parts_and_never_truncate() {
    let mut capacity = Capacities {
        rows: 4096,
        modifiers: SnapshotCapacity::default(),
    };
    let mut capture = Captured::with_capacities(&capacity);
    capture.begin_sources();
    for id in 1..=10_000_u64 {
        capture.visit_source(
            id as usize,
            &cship::Row {
                id,
                ..Default::default()
            },
        );
    }
    capture.finish_sources(Outcome::Rows { len: 10_000 });
    run(&mut capture, None);
    capture.grow(&mut capacity);
    assert!(capacity.rows >= 10_000);
    assert!(
        capture
            .extra
            .iter()
            .all(|part| part.rows.rows.len() >= 2_500)
    );
    let mut next = census(10_000, capacity.rows);
    std::mem::swap(&mut next.extra, &mut capture.extra);
    assert_eq!(next.split_details(parts_for(10_000)), 4);
    let mut lent: Vec<_> = (0..3).map(|index| next.take_part(index)).collect();
    run(&mut next, None);
    for part in &mut lent {
        run(part, None);
    }
    for (index, part) in lent.into_iter().enumerate() {
        next.restore_part(index, part);
    }
    next.finish_split();
    assert_eq!(next.outcomes(), [Outcome::Rows { len: 10_000 }]);
}

#[test]
fn a_range_a_part_cannot_hold_is_unavailable_not_partial() {
    let mut capture = census(8, 8);
    capture.extra[0] = Captured::part(&Capacities {
        rows: 2,
        modifiers: SnapshotCapacity::default(),
    });
    capture.split_details(2);
    let mut part = capture.take_part(0);
    run(&mut capture, None);
    run(&mut part, None);
    capture.restore_part(0, part);
    capture.finish_split();
    assert_eq!(
        capture.outcomes(),
        [Outcome::Unavailable {
            reason: "ship model part capacity below its census range"
        }]
    );
}

#[test]
fn an_interrupted_later_part_is_terminal_for_the_family() {
    let mut capture = census(7, 8);
    capture.split_details(3);
    let mut parts: Vec<_> = (0..2).map(|index| capture.take_part(index)).collect();
    run(&mut capture, None);
    run(&mut parts[0], None);
    parts[1].fail_owned_capture("ship models acquisition stopped before reading");
    for (index, part) in parts.into_iter().enumerate() {
        capture.restore_part(index, part);
    }
    capture.finish_split();
    assert!(capture.has_terminal_failure());
    assert_eq!(
        capture.outcomes(),
        [Outcome::Unavailable {
            reason: "ship models acquisition stopped before reading"
        }]
    );
}

#[test]
fn model_conversion_preserves_primary_handles_text_and_counter_operands() {
    let mut capture = Captured::with_capacities(&Capacities {
        rows: 1,
        modifiers: SnapshotCapacity::default(),
    });
    let row = &mut capture.rows.rows[0];
    row.ship = 51 | (1 << 32);
    row.variant = 70 | (2 << 32);
    row.hull_sprite.bytes[..3].copy_from_slice(&[65, 0, 255]);
    row.hull_sprite.len = 3;
    row.fallback_sprite.bytes[..2].copy_from_slice(&[0xff, 0]);
    row.fallback_sprite.len = 2;
    row.model_number = i32::MIN;
    row.formation_flags = 1 << 39;
    row.roster_ordinal = u32::MAX;
    row.definition_valid = u8::MAX;
    row.carrier_air_base = u64::MAX;
    row.maximum_strength = i64::MIN;
    row.anti_air_base = i64::MIN + 1;
    row.maximum_organisation = i64::MAX;
    row.positive_equipment_flags = u64::MAX;
    capture.rows.outcome = Outcome::Rows { len: 1 };
    let [TableView::Complete(replay_data::rows::TableRows::ShipModels(rows))] = capture.tables()
    else {
        panic!("complete")
    };
    assert_eq!(
        (rows[0].ship, rows[0].variant),
        (51 | (1 << 32), 70 | (2 << 32))
    );
    assert_eq!(rows[0].hull_sprite, [65, 0, 255]);
    assert_eq!(rows[0].fallback_sprite, [255, 0]);
    assert_eq!(
        (rows[0].model_number, rows[0].formation_flags),
        (i32::MIN, 1 << 39)
    );
    let counter = rows[0].counter.as_ref().unwrap();
    assert_eq!(
        (
            counter.roster_ordinal,
            counter.definition_valid,
            counter.carrier_air_base
        ),
        (u32::MAX, u8::MAX, u64::MAX)
    );
    assert_eq!(
        (
            counter.maximum_strength,
            counter.maximum_organisation,
            counter.positive_equipment_flags
        ),
        (i64::MIN, i64::MAX, u64::MAX)
    );
    assert_eq!(counter.anti_air_base, i64::MIN + 1);
    assert_eq!(
        rows[0].strike,
        replay_data::rows::Recorded::Value(replay_data::rows::Retired)
    );
}

#[test]
fn hull_projection_tracks_each_refused_scalar_and_null_ordinal() {
    use crate::projection::ProjectionScratch;
    use replay_data::{rows::Recorded, rows::TableRows};
    use replay_frame::{frame::FrameEncoder, frame::FrameInput, frame::Metrics};
    let complete = cship::HullVariantInputs {
        definition_index: Some(7),
        module_count: Some(i32::MIN),
        upgrade_level_sum: Some(-1),
        module_priority_sum: Some(i32::MAX),
        version: Some(u8::MAX),
        organisation_marker: Some(i32::MIN),
    };
    let fixture = |hull| {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            modifiers: SnapshotCapacity::default(),
        });
        capture.rows.rows[0] = cship::ModelRow {
            ship: (42 << 32) | 51,
            variant: (17 << 32) | 70,
            hull_variant: hull,
            ..Default::default()
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture
    };
    let frame = |hour, [table]: [TableInput; N]| {
        let mut frame = FrameInput {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                crate::table_conversion::unavailable("fixture table").into()
            }),
        };
        frame.tables[TABLES[0].index()] = table;
        frame
    };
    let capture = fixture(Some(complete));
    let [TableInput::Sampled(TableView::Complete(TableRows::ShipModels(rows)))] =
        capture.table_inputs(None)
    else {
        panic!("complete fixture")
    };
    assert_eq!(
        rows[0].hull_variant,
        Recorded::Value(Some(replay_data::rows::HullVariantInputs {
            definition_index: Some(7),
            module_count: Some(i32::MIN),
            upgrade_level_sum: Some(-1),
            module_priority_sum: Some(i32::MAX),
            version: Some(u8::MAX),
            organisation_marker: Some(i32::MIN),
        }))
    );
    for change in 0..10 {
        let mut hull = Some(complete);
        match change {
            0 => {}
            1 => hull = None,
            2 => hull.as_mut().unwrap().definition_index = None,
            3 => hull.as_mut().unwrap().module_count = None,
            4 => hull.as_mut().unwrap().upgrade_level_sum = None,
            5 => hull.as_mut().unwrap().module_priority_sum = None,
            6 => hull.as_mut().unwrap().version = None,
            7 => hull.as_mut().unwrap().organisation_marker = None,
            8 => hull.as_mut().unwrap().definition_index = Some(-1),
            9 => hull = Some(cship::HullVariantInputs::default()),
            _ => unreachable!(),
        }
        let mut projected = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        projected
            .encode_input(
                frame(100, fixture(Some(complete)).table_inputs(None)),
                false,
            )
            .unwrap();
        reference
            .encode_input(
                frame(100, fixture(Some(complete)).table_inputs(None)),
                false,
            )
            .unwrap();
        let scratch = ProjectionScratch::default();
        let mut projection = Projection::new(&projected, 1, 101, false, &scratch);
        let inputs = fixture(hull).table_inputs(Some(&mut projection));
        if change == 0 {
            assert!(matches!(inputs[0], TableInput::ComparedEqual(_)));
        } else {
            let TableInput::ComparedChanges { indices, .. } = &inputs[0] else {
                panic!("hull field {change} must replace its row")
            };
            assert_eq!(indices, &[0]);
        }
        assert_eq!(
            projected.encode_input(frame(101, inputs), false).unwrap(),
            reference
                .encode_input(frame(101, fixture(hull).table_inputs(None)), false)
                .unwrap(),
            "hull change {change}"
        );
    }
}

#[test]
fn growing_ship_census_keeps_the_first_mod_population_and_prepares_every_split() {
    let mut capture = Captured::with_capacities(&Capacities {
        rows: 0,
        modifiers: SnapshotCapacity::default(),
    });
    for ships in [5_000_u64, 12_001] {
        capture.begin_sources();
        for id in 1..=ships {
            capture.visit_source(
                id as usize,
                &cship::Row {
                    id,
                    ..Default::default()
                },
            );
        }
        capture.finish_sources(Outcome::Rows {
            len: ships as usize,
        });
        assert_eq!(capture.seed_len, ships as usize);
        let parts = capture.split_details(parts_for(ships as usize));
        let mut lent: Vec<_> = (0..parts - 1)
            .map(|index| capture.take_part(index))
            .collect();
        run(&mut capture, None);
        for part in &mut lent {
            run(part, None);
        }
        for (index, part) in lent.into_iter().enumerate() {
            capture.restore_part(index, part);
        }
        capture.finish_split();
        assert_eq!(
            capture.outcomes(),
            [Outcome::Rows {
                len: ships as usize
            }]
        );
    }
}

#[test]
fn cold_ship_models_use_only_original_census_and_later_population_demand() {
    let capacity = Capacities::default();
    assert_eq!(capacity.rows, 0);
    assert_eq!(capacity.modifiers, SnapshotCapacity::default());
    let mut capture = Captured::with_capacities(&capacity);
    assert_eq!(capture.allocated_bytes(), 0);
    assert_eq!(part_rows(0), 0);
    for count in [5_000_usize, 15_000] {
        capture.begin_sources();
        for index in 0..count {
            let id = (index as u64 + 1) << 32 | u64::from(cship::HANDLE_TYPE);
            capture.visit_source(
                0x1000 + index,
                &cship::Row {
                    id,
                    roster_ordinal: index as u32,
                    ..Default::default()
                },
            );
        }
        capture.finish_sources(Outcome::Rows { len: count });
        assert_eq!(capture.seed_len, count);
        assert_eq!(capture.missing, 0);
        let parts = capture.split_details(parts_for(count));
        let mut loans: Vec<_> = (0..parts - 1)
            .map(|index| capture.take_part(index))
            .collect();
        run(&mut capture, None);
        for part in &mut loans {
            run(part, None);
        }
        for (index, part) in loans.into_iter().enumerate() {
            capture.restore_part(index, part);
        }
        capture.finish_split();
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: count }]);
        let [TableView::Complete(replay_data::rows::TableRows::ShipModels(rows))] =
            capture.tables()
        else {
            panic!("complete original ship census");
        };
        assert_eq!(rows.len(), count);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                row.ship,
                (index as u64 + 1) << 32 | u64::from(cship::HANDLE_TYPE)
            );
            assert_eq!(row.counter.as_ref().unwrap().roster_ordinal, index as u32);
        }
    }
}
