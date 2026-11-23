use replay::campaign::Replay;
use replay_data::rows::{self, *};
use replay_frame::frame::{
    DecodedFrame, FrameDecoder, FrameEncoder, TableView,
    endpoint_step::{EndpointStep, EndpointWriter},
};

fn alias_frame(hour: i32) -> DecodedFrame {
    let mut frame = crate::test_fixtures::source(1, hour, Some(7));
    let division = (71_u64 << 32) | 19;
    let panel = DivisionPanelStatsRow {
        division,
        panel_stats: std::array::from_fn(|at| {
            Some(100_001 + hour as i64 * 20_000 + at as i64 * 10_001)
        }),
        suppression: Some(320_023 + hour as i64 * 20_000).into(),
        status_maxima: [None; 2],
    };
    let strength = 90_000 + hour as i64;
    let air = 30_000 + hour as i64;
    let fuel = 20_000 + hour as i64;
    let member = LandCombatDivisionRow {
        combat: (47_u64 << 32) | 42,
        division,
        ..Default::default()
    };
    let status = DivisionStatusRow {
        division,
        supply_ratio: None,
        supply_ratio_exact: Some(12_345 + i64::from(hour) * 1_000).into(),
        supply_grace_remaining: Recorded::Value(0),
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
    let value = i64::MAX.wrapping_add(i64::from(hour));
    let mut naval = NavalCombatDetailsRow {
        combat: 17,
        custom_sea_error: 1,
        ..Default::default()
    };
    naval.sides[0].damage_history[0][0] = value;
    naval.sides[0].total_damage = value;
    let day = FuelAwardDay {
        own_gain_raw: value.wrapping_add(9),
        gains_raw: [value, 9, 0, 0],
        received_raw: [0; 3],
        ..Default::default()
    };
    for rows in [
        TableRows::CombatLogObservers(vec![
            CombatLogObserverRow {
                combat: 17,
                ordinal: 1,
                kind: CombatLogObserverKind::State as u8,
                a: value.wrapping_add(9),
                ..Default::default()
            },
            CombatLogObserverRow {
                combat: 17,
                ordinal: 2,
                kind: CombatLogObserverKind::Damage as u8,
                a: value,
                ..Default::default()
            },
            CombatLogObserverRow {
                combat: 17,
                ordinal: 3,
                kind: CombatLogObserverKind::Damage as u8,
                a: 9,
                ..Default::default()
            },
        ]),
        TableRows::NavalCombatDetails(vec![naval]),
        TableRows::CountryFuel(vec![CountryFuelRow {
            country: 7,
            award_day: day.into(),
            ..Default::default()
        }]),
    ] {
        let table = rows.table();
        frame.tables[table.index()] = TableView::Complete(rows).into();
        frame.metrics.table_ns[table.index()] = 1;
    }
    match (hour - 300) % 12 {
        4 => {
            for table in [
                TableId::DivisionPanelStats,
                TableId::DivisionStatus,
                TableId::DivisionMovement,
                TableId::ProductionLineModifiers,
            ] {
                frame.tables[table.index()] = TableView::Unavailable {
                    reason: "independent source refusal".into(),
                }
                .into();
            }
        }
        5 => {
            for table in [
                TableId::DivisionMovement,
                TableId::DivisionStatus,
                TableId::CombatLogObservers,
                TableId::NavalCombatDetails,
                TableId::CountryFuel,
                TableId::LandCombatDivisions,
                TableId::LandCombatDivisionModifiers,
                TableId::Divisions,
                TableId::ProductionLines,
            ] {
                frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
            }
        }
        6 => {
            let TableView::Complete(rows) = &*frame.tables[TableId::LandCombatDivisions.index()]
            else {
                panic!()
            };
            frame.tables[TableId::LandCombatDivisions.index()] = TableView::Truncated {
                rows: rows.clone(),
                missing: 7,
            }
            .into();
        }
        _ => {}
    }
    replay_frame::frame::retire_absent(frame)
}

fn read_back(mut frame: DecodedFrame, held: &DecodedFrame) -> DecodedFrame {
    assert_eq!((frame.branch, frame.hour), (held.branch, held.hour));
    for (index, view) in held.tables.iter().enumerate() {
        if !view.is_not_selected() {
            frame.tables[index] = view.clone();
            frame.metrics.table_ns[index] = held.metrics.table_ns[index];
        }
    }
    frame
}

fn canonical(frame: &DecodedFrame) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame.encode_checkpoint(&mut bytes).unwrap();
    bytes
}
#[test]
fn endpoint_accepted_stream_matches_plain_frames_across_aliases_and_keyframes() {
    let frames: Vec<_> = (300..352).map(alias_frame).collect();
    let mut records = crate::test_fixtures::records(1, vec![frames[0].clone()]);
    records.truncate(2);
    let mut encoder = FrameEncoder::new();
    let mut plain = FrameDecoder::new();
    let mut decoded = Vec::new();
    for frame in frames {
        let mut raw = encoder.encode(frame, records.len() % 24 == 2).unwrap();
        raw.metrics = raw.metrics.stored();
        decoded.push(plain.advance(&raw).unwrap().clone());
        records.push(replay_capture_spool::spool::Record::Frame(raw));
    }
    let (replay, derived) = Replay::from_records_derived(recorder_derive::writers(), crate::test_fixtures::CAMPAIGN.into(), &[records]).unwrap();
    assert_eq!(derived.len(), decoded.len());
    let expected: Vec<_> = decoded.into_iter().zip(&derived).map(|(frame, held)| canonical(&read_back(frame, held))).collect();
    let mut writer = EndpointWriter::for_tables(&TableId::ALL);
    let mut coarse = EndpointWriter::for_tables(&TableId::ALL);
    let mut previous = None;
    let mut previous_coarse = None;
    let mut interval = Vec::new();
    let mut count = 0;
    let blocks: Vec<_> = replay.index.iter().collect();
    replay
        .visit_blocks(
            &blocks,
            &TableId::ALL,
            |_, raw, frame| {
                assert_eq!(canonical(frame), expected[count]);
                writer.observe(raw, frame)?;
                coarse.observe(raw, frame)?;
                let mut bytes = Vec::new();
                writer.finish(frame, &mut bytes)?;
                let step = EndpointStep::decode(&bytes)?;
                interval.extend(step.boundaries().cloned());
                drop(bytes);
                let restored = step.apply(previous.as_ref())?;
                assert_eq!(canonical(&restored), expected[count]);
                if let TableView::Complete(TableRows::ProductionLines(lines)) =
                    &*restored.tables[TableId::ProductionLines.index()]
                    && let Some(line) = lines.first()
                {
                    assert_eq!(line.efficiency_raw, 200_001 + i64::from(frame.hour));
                }
                if count % 24 == 0 || count == 51 {
                    let mut bytes = Vec::new();
                    coarse.finish(frame, &mut bytes)?;
                    let step = EndpointStep::decode(&bytes)?;
                    drop(bytes);
                    let mut boundaries: Vec<_> = step.boundaries().cloned().collect();
                    interval.sort_by_key(|b| (b.table.index(), b.hour));
                    boundaries.sort_by_key(|b| (b.table.index(), b.hour));
                    assert_eq!(boundaries, interval);
                    interval.clear();
                    let restored = step.apply(previous_coarse.as_ref())?;
                    assert_eq!(canonical(&restored), expected[count]);
                    let mut bytes = Vec::new();
                    EndpointWriter::root(frame, &TableId::ALL, &mut bytes)?;
                    let step = EndpointStep::decode(&bytes)?;
                    drop(bytes);
                    assert_eq!(canonical(&step.apply(None)?), expected[count]);
                    previous_coarse = Some(restored);
                }
                previous = Some(restored);
                count += 1;
                Ok(true)
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(count, 52);
}
