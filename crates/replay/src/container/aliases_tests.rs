use super::super::aliases;
use super::*;
use crate::{
    frame::{FrameEncoder, TableUpdate},
    rows::{
        self, DivisionMovementRow, DivisionPanelStatsRow, DivisionRow, DivisionStatusRow,
        LandCombatDivisionModifierRow, LandCombatDivisionRow, ProductionLineModifiersRow,
        ProductionLineRow, ProvinceUnitOrderRow, Recorded,
    },
};

fn fixture(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour);
    let division = (71_u64 << 32) | 19;
    let panel = DivisionPanelStatsRow {
        division,
        panel_stats: std::array::from_fn(|at| {
            Some(100_001 + hour as i64 * 20_000 + at as i64 * 10_001)
        }),
        suppression: Some(100_001 + hour as i64 * 20_000 + 22 * 10_001).into(),
    };
    let strength = 90_000 + hour as i64;
    let air = 30_000 + hour as i64;
    let fuel = 20_000 + hour as i64;
    let mut member = LandCombatDivisionRow {
        combat: (47_u64 << 32) | 42,
        division,
        strength_combat_factor: strength,
        armour: panel.panel_stats[13].unwrap(),
        piercing: panel.panel_stats[14].unwrap(),
        reconnaissance: panel.panel_stats[3].unwrap(),
        initiative: panel.panel_stats[15].unwrap(),
        air_attack: air,
        fuel_inputs: [fuel.into(), 99_999.into()],
        ..Default::default()
    };
    for (at, stat) in [11, 12, 20, 9, 10].into_iter().enumerate() {
        member.stat_values[at] = panel.panel_stats[stat].unwrap().into();
    }
    let status = DivisionStatusRow {
        division,
        supply_ratio: rows::DivisionStatusRow::SUPPLY_RATIO_STEP
            .round(12_345 + i64::from(hour) * 1_000),
        supply_ratio_exact: Some(12_345 + i64::from(hour) * 1_000).into(),
        supply_grace_remaining: rows::DivisionStatusRow::displayed_grace(
            700_000 + i64::from(hour) * 143_000,
            Some(143),
        )
        .into(),
        supply_grace_exact: Some(700_000 + i64::from(hour) * 143_000).into(),
        organisation_exact: Some(123_456 + i64::from(hour) * 10_000).into(),
        strength_factor: strength,
        movement_stats: Some(rows::DivisionMovementStats {
            air_defense: air,
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    let movement = DivisionMovementRow {
        division,
        fuel: Some(fuel).into(),
        ..Default::default()
    };
    let modifier = LandCombatDivisionModifierRow {
        combat: member.combat,
        division,
        ..Default::default()
    };
    let factories = vec![100_001 + hour as i64, 300_001 + hour as i64, i64::MAX];
    let line = ProductionLineRow {
        handle: (73_u64 << 32) | 81,
        country: 7,
        active_factories: 3,
        max_factories: 2,
        efficiency_raw: 200_001 + hour as i64,
        ..Default::default()
    };
    let modifiers = ProductionLineModifiersRow {
        line: line.handle,
        country: line.country,
        active_factories: line.active_factories,
        factory_efficiencies: factories,
        ..Default::default()
    };
    for rows in [
        TableRows::SimulationFeatures(vec![rows::SimulationFeatureRow {
            movement_defines: rows::MovementDefines {
                supply_grace: Some(143),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }]),
        TableRows::ProductionLines(vec![line]),
        TableRows::ProductionLineModifiers(vec![modifiers]),
        TableRows::Divisions(vec![DivisionRow {
            id: division,
            province: hour as u32,
            organisation: rows::DivisionRow::ORGANISATION_STEP
                .round(123_456 + i64::from(hour) * 10_000),
            ..Default::default()
        }]),
        TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: hour as u32,
            mixed: vec![division],
            ..Default::default()
        }]),
        TableRows::DivisionStatus(vec![status]),
        TableRows::DivisionPanelStats(vec![panel]),
        TableRows::DivisionMovement(vec![movement]),
        TableRows::LandCombatDivisions(vec![member]),
        TableRows::LandCombatDivisionModifiers(vec![modifier]),
    ] {
        let table = rows.table();
        frame.tables[table.index()] = TableView::Complete(rows).into();
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

fn recording(frames: &[DecodedFrame]) -> Replay {
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, frames[0].hour)),
    ];
    for frame in frames {
        records.push(Record::Frame(encoder.encode(frame.clone(), false).unwrap()));
    }
    Replay::from_records(CAMPAIGN.into(), &[records]).unwrap()
}

fn expected(mut frame: DecodedFrame) -> DecodedFrame {
    frame.metrics = frame.metrics.stored();
    frame
}

#[test]
fn private_copy_aliases_materialize_copies_before_any_public_reader_or_checkpoint() {
    let frames: Vec<_> = (1..=4).map(fixture).collect();
    let replay = recording(&frames);
    let targets = [
        TableId::LandCombatDivisions,
        TableId::LandCombatDivisionModifiers,
        TableId::Divisions,
        TableId::ProductionLines,
    ];
    for block in &replay.index {
        let mut decoder = FrameDecoder::new();
        for frame in replay.read_block(block).unwrap() {
            let actual = decoder.advance(&frame).unwrap();
            assert_eq!(*actual, expected(frames[(frame.hour - 1) as usize].clone()));
            let mut bytes = Vec::new();
            actual.encode_checkpoint(&mut bytes).unwrap();
            let restored =
                DecodedFrame::decode_checkpoint(&mut crate::codec::Decoder::new(&bytes)).unwrap();
            assert_eq!(*actual, restored);
        }
        let mut decoder = FrameDecoder::for_tables(&targets);
        for frame in replay.read_block_selected(block, &targets).unwrap() {
            let actual = decoder.advance(&frame).unwrap();
            for target in targets {
                assert_eq!(
                    actual.tables[target.index()],
                    frames[(frame.hour - 1) as usize].tables[target.index()]
                );
            }
            assert!(matches!(
                &*actual.tables[TableId::DivisionStatus.index()],
                TableView::Unavailable { .. }
            ));
        }
        let first = replay.read_keyframe_selected(block, &targets).unwrap();
        let actual = FrameDecoder::for_tables(&targets).apply(&first).unwrap();
        for target in targets {
            assert_eq!(
                actual.tables[target.index()],
                frames[(block.first_hour - 1) as usize].tables[target.index()]
            );
        }
    }
}

#[test]
fn private_copy_aliases_keep_every_literal_exception_and_independent_source_refusal() {
    let mut frames: Vec<_> = (1..=6).map(fixture).collect();
    for (at, frame) in frames.iter_mut().enumerate() {
        if let TableView::Complete(TableRows::LandCombatDivisions(rows)) =
            &mut *frame.tables[TableId::LandCombatDivisions.index()]
        {
            rows[0].armour = i64::MIN;
            rows[0].initiative = i64::MAX;
            rows[0].stat_values[2] = (-(at as i64) - 1).into();
            rows[0].fuel_inputs[0] = (-10).into();
        }
        if at % 2 == 1 {
            for source in [
                TableId::DivisionStatus,
                TableId::DivisionMovement,
                TableId::DivisionPanelStats,
            ] {
                frame.tables[source.index()] = TableView::Unavailable {
                    reason: "independent source refusal".into(),
                }
                .into();
            }
        }
        if at == 2 {
            let TableView::Complete(rows) = &*frame.tables[TableId::DivisionPanelStats.index()]
            else {
                unreachable!()
            };
            frame.tables[TableId::DivisionPanelStats.index()] = TableView::Truncated {
                rows: rows.clone(),
                missing: 1,
            }
            .into();
        }
    }
    let replay = recording(&frames);
    let mut decoder = FrameDecoder::new();
    for block in &replay.index {
        for frame in replay.read_block(block).unwrap() {
            assert_eq!(
                *decoder.advance(&frame).unwrap(),
                expected(frames[(frame.hour - 1) as usize].clone())
            );
        }
    }
}

#[test]
fn private_copy_aliases_do_not_rewrite_sampling_provenance_or_carried_literals() {
    let first = fixture(1);
    let mut second = fixture(2);
    second.tables[TableId::LandCombatDivisions.index()] =
        first.tables[TableId::LandCombatDivisions.index()].clone();
    second.metrics.table_ns[TableId::LandCombatDivisions.index()] = 0;
    let replay = recording(&[first, second]);
    let mut decoder = FrameDecoder::new();
    for frame in replay.read_block(&replay.index[0]).unwrap() {
        let actual = decoder.advance(&frame).unwrap();
        let view = &actual.tables[TableId::LandCombatDivisions.index()];
        assert_eq!(view.sampled_at(), Some(1));
        let TableView::Complete(TableRows::LandCombatDivisions(rows)) = &**view else {
            panic!("member missing")
        };
        assert_eq!(rows[0].strength_combat_factor, 90_001);
    }
}

#[test]
fn private_copy_aliases_use_markers_only_for_exact_complete_joins() {
    let frame = fixture(1);
    let wire = FrameEncoder::new().encode(frame.clone(), false).unwrap();
    let mut payload = Vec::new();
    let sizes = wire.encode_with_sizes(&mut payload).unwrap();
    let prepared = aliases::Encoder::default()
        .prepare(&wire, &frame, &payload, &sizes)
        .unwrap();
    let TableView::Complete(TableRows::LandCombatDivisions(rows)) =
        &*prepared.views.tables[TableId::LandCombatDivisions.index()]
    else {
        panic!()
    };
    assert_eq!(
        prepared.private.column(TableId::LandCombatDivisions, true),
        &[0xff, 0x1f, 0]
    );
    assert_eq!(rows[0].fuel_inputs(), Some([0, 99_999]));
    assert_eq!(rows[0].stat_values(), Some([0; 5]));
    let TableView::Complete(TableRows::LandCombatDivisionModifiers(rows)) =
        &*prepared.views.tables[TableId::LandCombatDivisionModifiers.index()]
    else {
        panic!()
    };
    assert_eq!(rows[0].division, 0);
    assert_eq!(
        prepared
            .private
            .column(TableId::LandCombatDivisionModifiers, true),
        &[1, 0]
    );
}

#[test]
fn private_copy_aliases_watch_source_driven_changes_even_without_raw_target_changes() {
    let frames: Vec<_> = (1..=5).map(fixture).collect();
    let replay = recording(&frames);
    let table = TableId::LandCombatDivisions;
    let fields = [TableFields::named(table, &["strength_combat_factor"]).unwrap()];
    let mut hours = Vec::new();
    replay
        .read_block_changes(
            &replay.index[0],
            &fields,
            |change| {
                match change {
                    BlockChange::Changed { frame, .. } => {
                        hours.push(frame.hour);
                        assert_eq!(
                            frame.tables[table.index()],
                            frames[(frame.hour - 1) as usize].tables[table.index()]
                        );
                    }
                    BlockChange::Quiet(_) => panic!("the aliased strength changes every hour"),
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(hours, [1, 2, 3, 4, 5]);
}

#[test]
fn private_copy_aliases_anchor_to_raw_rows_but_resolve_at_the_appended_hour() {
    let frames: Vec<_> = (1..=6).map(fixture).collect();
    let first = recording(&frames[..3]);
    let mut encoder = FrameEncoder::new();
    let mut suffix = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for frame in &frames[3..] {
        suffix.push(Record::Frame(encoder.encode(frame.clone(), false).unwrap()));
    }
    let replay = first.append(&[suffix]).unwrap();
    assert_eq!(replay.index.len(), 2);
    let tables = [
        TableId::LandCombatDivisions,
        TableId::LandCombatDivisionModifiers,
        TableId::Divisions,
        TableId::ProductionLines,
    ];
    for cache in [false, true] {
        replay.cache_decoded_sections(cache);
        for index in replay.index.iter().rev() {
            let parallel = Parallel {
                workers: 3,
                on_worker: &|| {},
            };
            let mut decoder = FrameDecoder::new();
            for wire in replay.read_block_parallel(index, &parallel).unwrap() {
                assert_eq!(
                    *decoder.advance(&wire).unwrap(),
                    expected(frames[(wire.hour - 1) as usize].clone())
                );
            }
            let keyframe = replay.read_keyframe_selected(index, &tables).unwrap();
            let actual = FrameDecoder::for_tables(&tables).apply(&keyframe).unwrap();
            for table in tables {
                assert_eq!(
                    actual.tables[table.index()],
                    frames[(index.first_hour - 1) as usize].tables[table.index()]
                );
            }
            for end in index.first_hour..=index.last_hour {
                let read = replay
                    .read_block_selected_through(index, Some(&tables), end, None)
                    .unwrap();
                let mut decoder = FrameDecoder::for_tables(&tables);
                for wire in read {
                    let actual = decoder.advance(&wire).unwrap();
                    for table in tables {
                        assert_eq!(
                            actual.tables[table.index()],
                            frames[(wire.hour - 1) as usize].tables[table.index()]
                        );
                    }
                }
            }
            let parallel = Parallel {
                workers: 3,
                on_worker: &|| {},
            };
            let read = replay.read_block_parallel(index, &parallel).unwrap();
            let mut decoder = FrameDecoder::new();
            for wire in read {
                assert_eq!(
                    *decoder.advance(&wire).unwrap(),
                    expected(frames[(wire.hour - 1) as usize].clone())
                );
            }
        }
    }
}

#[test]
fn private_copy_aliases_without_markers_need_no_source_and_invalid_sources_never_invent_values() {
    let frame = fixture(1);
    let table = TableId::LandCombatDivisions;
    let TableView::Complete(rows) = &*frame.tables[table.index()] else {
        panic!()
    };
    let literal = TableUpdate::Snapshot(rows.clone());
    let decoded =
        aliases::private::resolve(table, &[1], vec![literal.clone()], None, None, true, |_| {
            panic!("literal has no dependencies")
        })
        .unwrap();
    assert_eq!(decoded, [literal]);
    let wire = FrameEncoder::new().encode(frame.clone(), false).unwrap();
    let mut payload = Vec::new();
    let sizes = wire.encode_with_sizes(&mut payload).unwrap();
    let prepared = aliases::Encoder::default()
        .prepare(&wire, &frame, &payload, &sizes)
        .unwrap();
    let TableView::Complete(rows) = &*prepared.views.tables[table.index()] else {
        panic!()
    };
    let result = aliases::private::resolve(
        table,
        &[1],
        vec![TableUpdate::Snapshot(rows.clone())],
        Some(prepared.private.column(table, true)),
        None,
        true,
        |_| {
            crate::frame::recorded::Updates::new(vec![TableUpdate::Unavailable {
                reason: "source unavailable".into(),
            }])
        },
    );
    assert!(
        result.is_err(),
        "a stored alias needs its actual complete source"
    );
}

fn storage_rows(frame: &DecodedFrame, table: TableId) -> TableRows {
    let wire = FrameEncoder::new().encode(frame.clone(), false).unwrap();
    let mut payload = Vec::new();
    let sizes = wire.encode_with_sizes(&mut payload).unwrap();
    let prepared = aliases::Encoder::default()
        .prepare(&wire, frame, &payload, &sizes)
        .unwrap();
    let TableView::Complete(rows) = &*prepared.views.tables[table.index()] else {
        panic!("complete alias fixture")
    };
    rows.clone()
}

fn storage_marks(frame: &DecodedFrame, table: TableId) -> Vec<u8> {
    let wire = FrameEncoder::new().encode(frame.clone(), false).unwrap();
    let mut payload = Vec::new();
    let sizes = wire.encode_with_sizes(&mut payload).unwrap();
    aliases::Encoder::default()
        .prepare(&wire, frame, &payload, &sizes)
        .unwrap()
        .private
        .column(table, true)
        .to_vec()
}

fn storage_bits(frame: &DecodedFrame, table: TableId, wanted: u128) -> u16 {
    let marks = storage_marks(frame, table);
    let mut input = crate::codec::Decoder::new(&marks);
    let default = input.varint().unwrap() as u16;
    let count = input.varint().unwrap();
    let mut key = 0;
    let mut result = default;
    for _ in 0..count {
        key += input.varint().unwrap();
        let bits = input.varint().unwrap() as u16;
        if key == wanted {
            result ^= bits;
        }
    }
    input.finish().unwrap();
    result
}

#[test]
fn province_alias_requires_unique_mixed_membership_and_preserves_order_and_literals() {
    let mut frames: Vec<_> = (1..=7).map(fixture).collect();
    for (at, frame) in frames.iter_mut().enumerate() {
        let table = TableId::ProvinceUnitOrder;
        let TableView::Complete(TableRows::ProvinceUnitOrder(rows)) =
            &mut *frame.tables[table.index()]
        else {
            panic!()
        };
        let id = rows[0].mixed[0];
        match at {
            0 => rows[0].mixed = vec![u64::MAX, id, 1],
            1 => rows[0].mixed.push(id),
            2 => rows.push(ProvinceUnitOrderRow {
                province: 99,
                mixed: vec![id],
                ..Default::default()
            }),
            3 => rows[0].mixed.clear(),
            4 => rows[0].province += 1,
            5 => {
                frame.tables[table.index()] = TableView::Truncated {
                    rows: TableRows::ProvinceUnitOrder(rows.clone()),
                    missing: 1,
                }
                .into();
            }
            6 => {
                frame.tables[table.index()] = TableView::Unavailable {
                    reason: "independent occupant refusal".into(),
                }
                .into();
            }
            _ => unreachable!(),
        }
        let TableRows::Divisions(rows) = storage_rows(frame, TableId::Divisions) else {
            panic!()
        };
        assert_eq!(
            storage_bits(frame, TableId::Divisions, rows[0].id as u128),
            u16::from(at == 0) | 2
        );
        assert_eq!(
            rows[0].province,
            if at == 0 { 0 } else { frame.hour as u32 }
        );
    }
    let replay = recording(&frames);
    let mut decoder = FrameDecoder::new();
    for wire in replay.read_block(&replay.index[0]).unwrap() {
        assert_eq!(
            *decoder.advance(&wire).unwrap(),
            expected(frames[(wire.hour - 1) as usize].clone())
        );
    }
    let mut decoder = FrameDecoder::for_tables(&[TableId::Divisions]);
    for wire in replay
        .read_block_selected(&replay.index[0], &[TableId::Divisions])
        .unwrap()
    {
        let actual = decoder.advance(&wire).unwrap();
        assert_eq!(
            actual.tables[TableId::Divisions.index()],
            frames[(wire.hour - 1) as usize].tables[TableId::Divisions.index()]
        );
        assert!(matches!(
            &*actual.tables[TableId::ProvinceUnitOrder.index()],
            TableView::Unavailable { .. }
        ));
    }
}

#[test]
fn province_alias_watches_membership_changes_without_an_availability_cycle() {
    let frames: Vec<_> = (1..=5).map(fixture).collect();
    let replay = recording(&frames);
    let table = TableId::Divisions;
    let fields = [TableFields::named(table, &["province"]).unwrap()];
    let mut hours = Vec::new();
    replay
        .read_block_changes(
            &replay.index[0],
            &fields,
            |change| {
                match change {
                    BlockChange::Changed { frame, .. } => {
                        hours.push(frame.hour);
                        assert_eq!(
                            frame.tables[table.index()],
                            frames[(frame.hour - 1) as usize].tables[table.index()]
                        );
                    }
                    BlockChange::Quiet(_) => panic!("province changes with its unique membership"),
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(hours, [1, 2, 3, 4, 5]);
    // Occupant order has no dependency on the table which aliases its membership.
    let actual = replay
        .read_block_selected(&replay.index[0], &[TableId::ProvinceUnitOrder])
        .unwrap();
    let mut decoder = FrameDecoder::for_tables(&[TableId::ProvinceUnitOrder]);
    for wire in actual {
        let frame = decoder.advance(&wire).unwrap();
        assert_eq!(
            frame.tables[TableId::ProvinceUnitOrder.index()],
            frames[(wire.hour - 1) as usize].tables[TableId::ProvinceUnitOrder.index()]
        );
    }
}

#[test]
fn province_alias_never_invents_values_from_missing_or_ambiguous_sources() {
    let frame = fixture(1);
    let table = TableId::Divisions;
    let stored = storage_rows(&frame, table);
    for duplicate in [false, true] {
        let source = if duplicate {
            let TableView::Complete(TableRows::ProvinceUnitOrder(rows)) =
                &*frame.tables[TableId::ProvinceUnitOrder.index()]
            else {
                panic!()
            };
            let mut rows = rows.clone();
            let id = rows[0].mixed[0];
            rows[0].mixed.push(id);
            TableUpdate::Snapshot(TableRows::ProvinceUnitOrder(rows))
        } else {
            TableUpdate::Unavailable {
                reason: "no source".into(),
            }
        };
        assert!(
            aliases::private::resolve(
                table,
                &[1],
                vec![TableUpdate::Snapshot(stored.clone())],
                Some(&storage_marks(&frame, table)),
                None,
                true,
                |_| { crate::frame::recorded::Updates::new(vec![source.clone()]) }
            )
            .is_err()
        );
    }
    let TableView::Complete(literal) = &*frame.tables[table.index()] else {
        panic!()
    };
    let update = TableUpdate::Snapshot(literal.clone());
    assert_eq!(
        aliases::private::resolve(table, &[1], vec![update.clone()], None, None, true, |_| {
            panic!("literal location has no source dependency")
        })
        .unwrap(),
        [update]
    );
}

#[test]
fn efficiency_alias_keeps_the_signed_wrapping_prefix_average_and_literal_exceptions() {
    let cases = [
        (3, 2, vec![-7, 0, i64::MAX], -3),
        (3, 3, vec![i64::MAX, i64::MAX, 8], 2),
        (3, 3, vec![(1_i64 << 48) + 6, 0, 0], -(1_i64 << 48) / 3 + 2),
        (-1, 3, vec![], 0),
        (3, -1, vec![], 0),
    ];
    let mut frames = Vec::new();
    for (at, (active, maximum, values, expected_value)) in cases.into_iter().enumerate() {
        let mut frame = fixture(at as i32 + 1);
        let TableView::Complete(TableRows::ProductionLines(rows)) =
            &mut *frame.tables[TableId::ProductionLines.index()]
        else {
            panic!()
        };
        rows[0].active_factories = active;
        rows[0].max_factories = maximum;
        rows[0].efficiency_raw = expected_value;
        let line = rows[0].clone();
        let TableView::Complete(TableRows::ProductionLineModifiers(rows)) =
            &mut *frame.tables[TableId::ProductionLineModifiers.index()]
        else {
            panic!()
        };
        rows[0].active_factories = active;
        rows[0].factory_efficiencies = values;
        assert_eq!(line.factory_efficiency(&rows[0]), Some(expected_value));
        let TableRows::ProductionLines(rows) = storage_rows(&frame, TableId::ProductionLines)
        else {
            panic!()
        };
        assert_eq!(rows[0].efficiency_raw, 0);
        assert_eq!(
            storage_bits(&frame, TableId::ProductionLines, rows[0].handle as u128),
            1
        );
        frames.push(frame);
    }
    let replay = recording(&frames);
    let mut decoder = FrameDecoder::new();
    for wire in replay.read_block(&replay.index[0]).unwrap() {
        assert_eq!(
            *decoder.advance(&wire).unwrap(),
            expected(frames[(wire.hour - 1) as usize].clone())
        );
    }
}

#[test]
fn efficiency_alias_requires_complete_matching_context_and_the_entire_factory_prefix() {
    let mut frames: Vec<_> = (1..=10).map(fixture).collect();
    for (at, frame) in frames.iter_mut().enumerate() {
        let table = TableId::ProductionLineModifiers;
        let TableView::Complete(TableRows::ProductionLineModifiers(rows)) =
            &mut *frame.tables[table.index()]
        else {
            panic!()
        };
        match at {
            0 => rows[0].country += 1,
            1 => rows[0].variant += 1,
            2 => rows[0].organisation += 1,
            3 => rows[0].kind += 1,
            4 => rows[0].active_factories += 1,
            5 => rows[0].factory_efficiencies.truncate(1),
            6 => rows.clear(),
            7 => {
                frame.tables[table.index()] = TableView::Truncated {
                    rows: TableRows::ProductionLineModifiers(rows.clone()),
                    missing: 1,
                }
                .into();
            }
            8 => {
                frame.tables[table.index()] = TableView::Unavailable {
                    reason: "factory census unavailable".into(),
                }
                .into();
            }
            9 => {
                let TableView::Complete(TableRows::ProductionLines(rows)) =
                    &mut *frame.tables[TableId::ProductionLines.index()]
                else {
                    panic!()
                };
                rows[0].efficiency_raw = i64::MIN;
            }
            _ => unreachable!(),
        }
        let TableRows::ProductionLines(rows) = storage_rows(frame, TableId::ProductionLines) else {
            panic!()
        };
        assert_eq!(
            storage_bits(frame, TableId::ProductionLines, rows[0].handle as u128),
            0
        );
        assert_eq!(
            rows[0].efficiency_raw,
            if at == 9 {
                i64::MIN
            } else {
                200_001 + frame.hour as i64
            }
        );
    }
    let replay = recording(&frames);
    let mut decoder = FrameDecoder::for_tables(&[TableId::ProductionLines]);
    for wire in replay
        .read_block_selected(&replay.index[0], &[TableId::ProductionLines])
        .unwrap()
    {
        assert_eq!(
            decoder.advance(&wire).unwrap().tables[TableId::ProductionLines.index()],
            frames[(wire.hour - 1) as usize].tables[TableId::ProductionLines.index()]
        );
    }
}

fn movement_fixture(hour: i32) -> DecodedFrame {
    let mut frame = fixture(hour);
    let TableView::Complete(TableRows::DivisionPanelStats(panel)) =
        &*frame.tables[TableId::DivisionPanelStats.index()]
    else {
        panic!()
    };
    let speed = panel[0].panel_stats[21].unwrap();
    let TableView::Complete(TableRows::DivisionMovement(rows)) =
        &mut *frame.tables[TableId::DivisionMovement.index()]
    else {
        panic!()
    };
    rows[0].path = (0..hour as u32 + 2).collect();
    rows[0].path_total = rows[0].path.len() as u32;
    rows[0].base_speed = Some(speed).into();
    frame
}

#[test]
fn private_movement_aliases_restore_source_only_changes_paths_and_independent_absence() {
    let mut frames: Vec<_> = (1..=8).map(movement_fixture).collect();
    for (at, frame) in frames.iter_mut().enumerate() {
        let TableView::Complete(TableRows::DivisionMovement(rows)) =
            &mut *frame.tables[TableId::DivisionMovement.index()]
        else {
            panic!()
        };
        match at {
            1 => {
                rows[0].path_total = u32::MAX;
                rows[0].base_speed = Some(i64::MIN).into();
            }
            2 => rows[0].base_speed = None.into(),
            3 => {
                rows[0].path_total = 1;
                rows[0].base_speed = Some(0).into();
            }
            4 => {
                rows[0].base_speed = Some(17).into();
                frame.tables[TableId::DivisionPanelStats.index()] = TableView::Unavailable {
                    reason: "panel unavailable".into(),
                }
                .into();
            }
            _ => {}
        }
    }
    let replay = recording(&frames);
    for block in &replay.index {
        let mut decoder = FrameDecoder::new();
        for wire in replay.read_block(block).unwrap() {
            assert_eq!(
                *decoder.advance(&wire).unwrap(),
                expected(frames[(wire.hour - 1) as usize].clone())
            );
        }
        let mut decoder = FrameDecoder::for_tables(&[TableId::DivisionMovement]);
        for wire in replay
            .read_block_selected(block, &[TableId::DivisionMovement])
            .unwrap()
        {
            let actual = decoder.advance(&wire).unwrap();
            assert_eq!(
                actual.tables[TableId::DivisionMovement.index()],
                frames[(wire.hour - 1) as usize].tables[TableId::DivisionMovement.index()]
            );
            assert!(matches!(
                &*actual.tables[TableId::DivisionPanelStats.index()],
                TableView::Unavailable { .. }
            ));
        }
    }
    assert_eq!(TableId::DivisionMovement.field_count(), 25);
    assert!(
        replay
            .block_columns(&replay.index[0])
            .unwrap()
            .unwrap()
            .iter()
            .filter(|section| matches!(
                section.section,
                Section::Keyframe(TableId::DivisionMovement)
                    | Section::Later(TableId::DivisionMovement)
            ))
            .all(|section| section.columns.len() == 3 + 25 + 1)
    );
}

#[test]
fn private_movement_marks_restart_at_keyframes_and_preserve_strategic_action_validation() {
    let mut frames: Vec<_> = (1..=6).map(movement_fixture).collect();
    for frame in &mut frames {
        let TableView::Complete(TableRows::DivisionMovement(rows)) =
            &mut *frame.tables[TableId::DivisionMovement.index()]
        else {
            panic!()
        };
        rows[0].strategic_redeployment = 1;
        rows[0].strategic_action =
            rows::StrategicAction::Withheld(rows::StrategicActionFailure::Owner).into();
    }
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for frame in &frames {
        records.push(Record::Frame(
            encoder.encode(frame.clone(), frame.hour % 2 == 1).unwrap(),
        ));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    for block in &replay.index {
        let targets = &[TableId::DivisionMovement];
        let first = replay.read_keyframe_selected(block, targets).unwrap();
        let actual = FrameDecoder::for_tables(targets).apply(&first).unwrap();
        assert_eq!(
            actual.tables[TableId::DivisionMovement.index()],
            frames[(first.hour - 1) as usize].tables[TableId::DivisionMovement.index()]
        );
        let mut decoder = FrameDecoder::for_tables(targets);
        for wire in replay.read_block_selected(block, targets).unwrap() {
            assert_eq!(
                decoder.advance(&wire).unwrap().tables[TableId::DivisionMovement.index()],
                frames[(wire.hour - 1) as usize].tables[TableId::DivisionMovement.index()]
            );
        }
    }
}

#[test]
fn private_marks_change_even_when_the_native_placeholder_has_no_entry() {
    let mut frames: Vec<_> = (1..=3).map(movement_fixture).collect();
    for (at, frame) in frames.iter_mut().enumerate() {
        let TableView::Complete(TableRows::DivisionMovement(rows)) =
            &mut *frame.tables[TableId::DivisionMovement.index()]
        else {
            panic!()
        };
        rows[0].path = vec![1, 2];
        rows[0].path_total = 2;
        rows[0].base_speed = Some(0).into();
        let TableView::Complete(TableRows::DivisionPanelStats(rows)) =
            &mut *frame.tables[TableId::DivisionPanelStats.index()]
        else {
            panic!()
        };
        rows[0].panel_stats[21] = Some(i64::from(at == 1));
    }
    let replay = recording(&frames);
    let selected = &[TableId::DivisionMovement];
    let mut decoder = FrameDecoder::for_tables(selected);
    for wire in replay
        .read_block_selected(&replay.index[0], selected)
        .unwrap()
    {
        assert_eq!(
            decoder.advance(&wire).unwrap().tables[TableId::DivisionMovement.index()],
            frames[(wire.hour - 1) as usize].tables[TableId::DivisionMovement.index()]
        );
    }
    let prefix = replay
        .read_block_selected_through(&replay.index[0], Some(selected), 2, None)
        .unwrap();
    assert_eq!(prefix.len(), 2);
    let mut decoder = FrameDecoder::for_tables(selected);
    for wire in prefix {
        assert_eq!(
            decoder.advance(&wire).unwrap().tables[TableId::DivisionMovement.index()],
            frames[(wire.hour - 1) as usize].tables[TableId::DivisionMovement.index()]
        );
    }
}

fn private_nested_fixture(hour: i32) -> DecodedFrame {
    let mut frame = movement_fixture(hour);
    let TableView::Complete(TableRows::DivisionPanelStats(panel)) =
        &*frame.tables[TableId::DivisionPanelStats.index()]
    else {
        panic!()
    };
    let hp = DivisionStatusRow::MAXIMUM_STEP.round(panel[0].panel_stats[0].unwrap().max(100));
    let org = DivisionStatusRow::MAXIMUM_STEP.round(panel[0].panel_stats[1].unwrap().max(100));
    let name = format!("division {hour}").into_bytes();
    let TableView::Complete(TableRows::DivisionStatus(status)) =
        &mut *frame.tables[TableId::DivisionStatus.index()]
    else {
        panic!()
    };
    status[0].maximum_hit_points = hp;
    status[0].maximum_organisation = org;
    status[0].name = Some(name.clone()).into();
    let damage = i64::MAX.wrapping_add(i64::from(hour));
    let observers = vec![
        rows::CombatLogObserverRow {
            combat: 17,
            ordinal: 1,
            kind: rows::CombatLogObserverKind::State as u8,
            a: damage,
            b: i64::from(hour),
            ..Default::default()
        },
        rows::CombatLogObserverRow {
            combat: 17,
            ordinal: 2,
            kind: rows::CombatLogObserverKind::Damage as u8,
            a: damage,
            ..Default::default()
        },
    ];
    let mut naval = rows::NavalCombatDetailsRow {
        combat: 17,
        custom_sea_error: 1,
        ..Default::default()
    };
    naval.sides[0].damage_history[0][0] = damage;
    naval.sides[0].total_damage = damage;
    let day = rows::FuelAwardDay {
        own_gain_raw: damage,
        gains_raw: [damage, 0, 0, 0],
        own_gain_quantity_raw: i64::from(hour),
        received_raw: [0, damage, 0],
        recipients: vec![rows::FuelAwardDayRecipient {
            kind: 1,
            amount_raw: damage,
            ..Default::default()
        }],
        ..Default::default()
    };
    for rows in [
        TableRows::CombatLogObservers(observers),
        TableRows::NavalCombatDetails(vec![naval]),
        TableRows::CountryFuel(vec![rows::CountryFuelRow {
            country: 7,
            award_day: day.into(),
            ..Default::default()
        }]),
    ] {
        let table = rows.table();
        frame.tables[table.index()] = TableView::Complete(rows).into();
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

#[test]
fn private_alias_tables_materialize_before_selected_reads_keyframes_and_checkpoints() {
    let mut frames: Vec<_> = (1..=6).map(private_nested_fixture).collect();
    frames[2].tables[TableId::DivisionPanelStats.index()] = TableView::Unavailable {
        reason: "independent panel refusal".into(),
    }
    .into();
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for frame in &frames {
        records.push(Record::Frame(
            encoder.encode(frame.clone(), frame.hour % 2 == 1).unwrap(),
        ));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    let targets = &[
        TableId::DivisionMovement,
        TableId::DivisionStatus,
        TableId::CombatLogObservers,
        TableId::NavalCombatDetails,
        TableId::CountryFuel,
    ];
    for block in &replay.index {
        for &target in targets {
            let mut decoder = FrameDecoder::for_tables(&[target]);
            for wire in replay.read_block_selected(block, &[target]).unwrap() {
                let actual = decoder.advance(&wire).unwrap();
                assert_eq!(
                    actual.tables[target.index()],
                    frames[(wire.hour - 1) as usize].tables[target.index()]
                );
                assert_eq!(actual.tables[target.index()].sampled_at(), Some(wire.hour));
                let mut bytes = Vec::new();
                actual.encode_checkpoint(&mut bytes).unwrap();
                assert_eq!(
                    *actual,
                    DecodedFrame::decode_checkpoint(&mut crate::codec::Decoder::new(&bytes))
                        .unwrap()
                );
            }
            let keyframe = replay.read_keyframe_selected(block, &[target]).unwrap();
            assert_eq!(
                FrameDecoder::for_tables(&[target])
                    .apply(&keyframe)
                    .unwrap()
                    .tables[target.index()],
                frames[(keyframe.hour - 1) as usize].tables[target.index()]
            );
        }
    }
}

#[test]
fn conditional_display_aliases_keep_literal_demand_changes_and_independent_source_failures() {
    let mut frames: Vec<_> = (1..=8).map(fixture).collect();
    for (at, frame) in frames.iter_mut().enumerate() {
        if at == 1 || at == 2 {
            let TableView::Complete(TableRows::DivisionStatus(rows)) =
                &mut *frame.tables[TableId::DivisionStatus.index()]
            else {
                panic!()
            };
            if at == 1 {
                rows[0].supply_ratio_exact = None.into();
                rows[0].supply_grace_exact = None.into();
                rows[0].organisation_exact = None.into();
            } else {
                rows[0].supply_ratio += 1_000;
                let Recorded::Value(grace) = &mut rows[0].supply_grace_remaining else {
                    panic!()
                };
                *grace += 1_000;
                let TableView::Complete(TableRows::Divisions(rows)) =
                    &mut *frame.tables[TableId::Divisions.index()]
                else {
                    panic!()
                };
                rows[0].organisation += 10_000;
            }
        }
        if (3..=5).contains(&at) {
            let table = TableId::SimulationFeatures;
            frame.tables[table.index()] = match at {
                3 => TableView::Unavailable {
                    reason: "independent define refusal".into(),
                },
                4 => {
                    let TableView::Complete(rows) = &*frame.tables[table.index()] else {
                        panic!()
                    };
                    TableView::Truncated {
                        rows: rows.clone(),
                        missing: 1,
                    }
                }
                _ => TableView::Complete(TableRows::SimulationFeatures(vec![
                    rows::SimulationFeatureRow {
                        movement_defines: rows::MovementDefines {
                            supply_grace: Some(i64::MAX),
                            ..Default::default()
                        }
                        .into(),
                        ..Default::default()
                    },
                ])),
            }
            .into();
        }
        if at >= 6 {
            let table = TableId::DivisionStatus;
            frame.tables[table.index()] = if at == 6 {
                let TableView::Complete(rows) = &*frame.tables[table.index()] else {
                    panic!()
                };
                TableView::Truncated {
                    rows: rows.clone(),
                    missing: 1,
                }
            } else {
                TableView::Unavailable {
                    reason: "independent exact-status refusal".into(),
                }
            }
            .into();
        }
    }
    let replay = recording(&frames);
    let block = &replay.index[0];
    let mut decoder = FrameDecoder::new();
    for frame in replay.read_block(block).unwrap() {
        assert_eq!(
            *decoder.advance(&frame).unwrap(),
            expected(frames[(frame.hour - 1) as usize].clone())
        );
    }
    for table in [TableId::DivisionStatus, TableId::Divisions] {
        let mut decoder = FrameDecoder::for_tables(&[table]);
        for frame in replay.read_block_selected(block, &[table]).unwrap() {
            let actual = decoder.advance(&frame).unwrap();
            assert_eq!(
                actual.tables[table.index()],
                frames[(frame.hour - 1) as usize].tables[table.index()]
            );
            for source in [
                TableId::SimulationFeatures,
                TableId::ProvinceUnitOrder,
                TableId::DivisionPanelStats,
            ] {
                assert!(matches!(
                    &*actual.tables[source.index()],
                    TableView::Unavailable { .. }
                ));
            }
        }
    }
}

#[test]
fn conditional_display_aliases_watch_resolved_changes_with_quiet_placeholders() {
    let frames: Vec<_> = (1..=4).map(fixture).collect();
    let replay = recording(&frames);
    for (table, fields) in [
        (
            TableId::DivisionStatus,
            &["supply_ratio", "supply_grace_remaining"][..],
        ),
        (TableId::Divisions, &["organisation"][..]),
    ] {
        let fields = [TableFields::named(table, fields).unwrap()];
        let mut hours = Vec::new();
        replay
            .read_block_changes(
                &replay.index[0],
                &fields,
                |change| {
                    match change {
                        BlockChange::Changed { frame, .. } => {
                            hours.push(frame.hour);
                            assert_eq!(
                                frame.tables[table.index()],
                                frames[(frame.hour - 1) as usize].tables[table.index()]
                            );
                        }
                        BlockChange::Quiet(_) => panic!("the resolved display changes every hour"),
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(hours, [1, 2, 3, 4]);
    }
}

/// Resolution hooks are process-wide because decode workers share them.
fn resolved_as<T>(mode: aliases::private::Resolution, work: impl FnOnce() -> T) -> (T, usize) {
    use aliases::private::hooks;
    hooks::with(|hooks| {
        hooks.resolution = Some(mode);
        hooks.candidate_hours = 0;
    });
    let result = work();
    let hours = hooks::with(|hooks| {
        hooks.resolution = None;
        hooks.candidate_hours
    });
    (result, hours)
}

/// Verify candidate bytes and views against whole-table resolution.
#[test]
fn every_alias_test_passes_with_candidate_rows_verified_and_with_whole_tables() {
    use aliases::private::Resolution;
    let tests: [fn(); 18] = [
        private_copy_aliases_materialize_copies_before_any_public_reader_or_checkpoint,
        private_copy_aliases_keep_every_literal_exception_and_independent_source_refusal,
        private_copy_aliases_do_not_rewrite_sampling_provenance_or_carried_literals,
        private_copy_aliases_use_markers_only_for_exact_complete_joins,
        private_copy_aliases_watch_source_driven_changes_even_without_raw_target_changes,
        private_copy_aliases_anchor_to_raw_rows_but_resolve_at_the_appended_hour,
        private_copy_aliases_without_markers_need_no_source_and_invalid_sources_never_invent_values,
        province_alias_requires_unique_mixed_membership_and_preserves_order_and_literals,
        province_alias_watches_membership_changes_without_an_availability_cycle,
        province_alias_never_invents_values_from_missing_or_ambiguous_sources,
        efficiency_alias_keeps_the_signed_wrapping_prefix_average_and_literal_exceptions,
        efficiency_alias_requires_complete_matching_context_and_the_entire_factory_prefix,
        private_movement_aliases_restore_source_only_changes_paths_and_independent_absence,
        private_movement_marks_restart_at_keyframes_and_preserve_strategic_action_validation,
        private_marks_change_even_when_the_native_placeholder_has_no_entry,
        private_alias_tables_materialize_before_selected_reads_keyframes_and_checkpoints,
        conditional_display_aliases_keep_literal_demand_changes_and_independent_source_failures,
        conditional_display_aliases_watch_resolved_changes_with_quiet_placeholders,
    ];
    let ((), verified) = resolved_as(Resolution::Verify, || tests.iter().for_each(|test| test()));
    assert!(verified > 0, "the alias tests must reach candidate hours");
    let ((), whole) = resolved_as(Resolution::Whole, || tests.iter().for_each(|test| test()));
    assert_eq!(whole, 0);
}

/// Churn covers membership, marks, source-only changes and incomplete sources.
fn churn_frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour);
    let h = i64::from(hour);
    let division = |d: u64| (71_u64 << 32) | d;
    let present: Vec<u64> = (1..=8)
        .filter(|&d| !(d == 3 && (6..=9).contains(&hour)) && (d != 8 || hour >= 5))
        .collect();
    let grace_define: i32 = if hour >= 7 { 150 } else { 143 };
    let (mut panels, mut statuses, mut movements, mut divisions) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut provinces = std::collections::BTreeMap::<u32, Vec<u64>>::new();
    for &d in &present {
        let id = division(d);
        let di = d as i64;
        let base = 100_001
            + di * 7_001
            + match d {
                1 => h * 20_000,
                2 if hour % 4 == 0 => h * 5_000,
                _ => 0,
            };
        let panel = DivisionPanelStatsRow {
            division: id,
            panel_stats: std::array::from_fn(|at| Some(base + at as i64 * 10_001)),
            suppression: Some(base + 22 * 10_001).into(),
        };
        let maximum = |stat: usize| {
            DivisionStatusRow::MAXIMUM_STEP.round(panel.panel_stats[stat].unwrap().max(100))
        };
        let organisation = 123_456 + di * 10_000 + if d == 5 { h * 10_000 } else { 0 };
        let ratio = 12_345 + if d % 2 == 1 { h * 1_000 } else { 0 };
        let grace = 700_000 + di * 1_000;
        let mut status = DivisionStatusRow {
            division: id,
            maximum_hit_points: maximum(0),
            maximum_organisation: maximum(1),
            supply_ratio: DivisionStatusRow::SUPPLY_RATIO_STEP.round(ratio),
            supply_ratio_exact: Some(ratio).into(),
            supply_grace_remaining: DivisionStatusRow::displayed_grace(grace, Some(grace_define))
                .into(),
            supply_grace_exact: Some(grace).into(),
            organisation_exact: Some(organisation).into(),
            strength_factor: 90_000 + di,
            ..Default::default()
        };
        if d == 4 && hour % 3 == 1 {
            status.maximum_hit_points += 10_000;
        }
        let movement = DivisionMovementRow {
            division: id,
            source: 300,
            path: if d == 6 {
                vec![400 + hour as u32, 401 + hour as u32]
            } else {
                vec![500, 501]
            },
            path_total: 2,
            base_speed: panel.panel_stats[21].into(),
            fuel: Some(20_000 + di).into(),
            ..Default::default()
        };
        let province = if d == 1 {
            1_000 + hour as u32 / 2
        } else {
            1_000 + d as u32 * 10
        };
        provinces.entry(province).or_default().push(id);
        divisions.push(DivisionRow {
            id,
            province,
            organisation: DivisionRow::ORGANISATION_STEP.round(organisation),
            ..Default::default()
        });
        panels.push(panel);
        statuses.push(status);
        movements.push(movement);
    }
    if (10..=11).contains(&hour) {
        provinces.entry(2_000).or_default().push(division(2));
    }
    let combat = (47_u64 << 32) | 42;
    let members: Vec<_> = [1_u64, 2]
        .into_iter()
        .enumerate()
        .map(|(ordinal, d)| {
            (
                ordinal as u32,
                division(if ordinal == 0 && hour >= 6 { 5 } else { d }),
            )
        })
        .collect();
    let (mut lines, mut line_modifiers) = (Vec::new(), Vec::new());
    for line in 1..=3_u64 {
        let handle = (73_u64 << 32) | line;
        let shift = if line == 2 && hour % 2 == 0 { h } else { 0 };
        lines.push(ProductionLineRow {
            handle,
            country: 7,
            active_factories: 3,
            max_factories: 2,
            efficiency_raw: 200_001 + shift + i64::from(line == 3),
            ..Default::default()
        });
        line_modifiers.push(ProductionLineModifiersRow {
            line: handle,
            country: 7,
            active_factories: 3,
            factory_efficiencies: vec![100_001 + shift, 300_001 + shift, i64::MAX],
            ..Default::default()
        });
    }
    let panels = match hour {
        12 => TableView::Unavailable {
            reason: "independent panel refusal".into(),
        },
        13 => TableView::Truncated {
            rows: TableRows::DivisionPanelStats(panels),
            missing: 1,
        },
        _ => TableView::Complete(TableRows::DivisionPanelStats(panels)),
    };
    frame.tables[TableId::DivisionPanelStats.index()] = panels.into();
    frame.metrics.table_ns[TableId::DivisionPanelStats.index()] = 1;
    for rows in [
        TableRows::SimulationFeatures(vec![rows::SimulationFeatureRow {
            movement_defines: rows::MovementDefines {
                supply_grace: Some(i64::from(grace_define)),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        }]),
        TableRows::DivisionStatus(statuses),
        TableRows::DivisionMovement(movements),
        TableRows::Divisions(divisions),
        TableRows::ProvinceUnitOrder(
            provinces
                .into_iter()
                .map(|(province, mixed)| ProvinceUnitOrderRow {
                    province,
                    mixed,
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::LandCombatDivisions(
            members
                .iter()
                .map(|&(ordinal, division)| LandCombatDivisionRow {
                    combat,
                    ordinal,
                    division,
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::LandCombatDivisionModifiers(
            members
                .iter()
                .map(|&(ordinal, division)| LandCombatDivisionModifierRow {
                    combat,
                    ordinal,
                    division,
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::ProductionLines(lines),
        TableRows::ProductionLineModifiers(line_modifiers),
    ] {
        let table = rows.table();
        frame.tables[table.index()] = TableView::Complete(rows).into();
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

/// The churn recording: hours 1..=18, its second block from hour 11.
fn churn_recording(frames: &[DecodedFrame]) -> Replay {
    let first = recording(&frames[..10]);
    let mut encoder = FrameEncoder::new();
    let mut suffix = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for frame in &frames[10..] {
        suffix.push(Record::Frame(encoder.encode(frame.clone(), false).unwrap()));
    }
    let replay = first.append(&[suffix]).unwrap();
    assert_eq!(
        replay.index.len(),
        2,
        "the second block restarts at its keyframe"
    );
    replay
}

/// Compare candidate and whole-table resolution against independently encoded frames.
#[test]
fn candidate_rows_resolve_churn_membership_marks_and_source_changes_exactly() {
    use aliases::private::{Resolution, TABLES};
    let frames: Vec<_> = (1..=18).map(churn_frame).collect();
    let replay = churn_recording(&frames);
    for mode in [
        Resolution::Verify,
        Resolution::Candidates,
        Resolution::Whole,
    ] {
        let ((), candidate_hours) = resolved_as(mode, || {
            for block in &replay.index {
                let mut decoder = FrameDecoder::new();
                for frame in replay.read_block(block).unwrap() {
                    let actual = decoder.advance(&frame).unwrap();
                    assert_eq!(
                        *actual,
                        expected(frames[(frame.hour - 1) as usize].clone()),
                        "{mode:?} hour {}",
                        frame.hour
                    );
                }
                let mut decoder = FrameDecoder::for_tables(TABLES);
                for frame in replay.read_block_selected(block, TABLES).unwrap() {
                    let actual = decoder.advance(&frame).unwrap();
                    for &table in TABLES {
                        assert_eq!(
                            actual.tables[table.index()],
                            frames[(frame.hour - 1) as usize].tables[table.index()],
                            "{mode:?} {table:?} hour {}",
                            frame.hour
                        );
                    }
                }
            }
        });
        if mode == Resolution::Whole {
            assert_eq!(candidate_hours, 0);
        } else {
            assert!(candidate_hours > 0, "{mode:?} must resolve candidate hours");
        }
    }
}

/// Compare the refusing hour and every preceding resolved-byte count.
#[test]
fn candidate_rows_keep_the_resolved_size_refusal_and_its_byte_counts() {
    use aliases::private::{Resolution, hooks};
    let frames: Vec<_> = (1..=10).map(churn_frame).collect();
    let replay = recording(&frames);
    let read = |mode, limit| {
        hooks::with(|hooks| {
            hooks.limit = limit;
            hooks.budget.clear();
        });
        let (result, candidate_hours) = resolved_as(mode, || {
            replay
                .read_block(&replay.index[0])
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        let budget = hooks::with(|hooks| {
            hooks.limit = None;
            std::mem::take(&mut hooks.budget)
        });
        (result, budget, candidate_hours)
    };
    let (whole, budget, _) = read(Resolution::Whole, None);
    assert!(whole.is_ok());
    // Every table's first hour fits; a later hour of some table does not.
    let limit = budget
        .iter()
        .filter(|(_, at, _)| *at == 0)
        .map(|(_, _, bytes)| *bytes)
        .max()
        .unwrap();
    assert!(budget.iter().any(|(_, _, bytes)| *bytes > limit));
    let (whole, whole_budget, _) = read(Resolution::Whole, Some(limit));
    assert!(
        whole.as_ref().is_err_and(
            |error| error.contains("resolved private aliases exceed raw block size limit")
        ),
        "{whole:?}"
    );
    let refused_at = whole_budget.last().map(|&(_, at, _)| at).unwrap();
    assert!(
        refused_at > 0,
        "the refusing hour follows a candidate-eligible hour"
    );
    for mode in [Resolution::Candidates, Resolution::Verify] {
        let (refused, refused_budget, candidate_hours) = read(mode, Some(limit));
        assert_eq!(refused, whole, "{mode:?}");
        assert_eq!(refused_budget, whole_budget, "{mode:?}");
        assert!(
            candidate_hours > 0,
            "{mode:?} must resolve candidate hours before refusing"
        );
    }
}
