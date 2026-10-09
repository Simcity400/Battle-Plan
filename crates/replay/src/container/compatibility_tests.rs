use super::*;
use crate::{
    format::FormatVersion,
    rows::{
        self, RowCodec,
        compatibility::{HISTORY, LastChange, PROVEN_FROM, RECORDER_INPUTS_FORMAT},
    },
};

fn legacy_rows(table: TableId, hour: Hour, format: FormatVersion) -> TableRows {
    match table {
        TableId::Fleets => TableRows::Fleets(vec![rows::FleetRow {
            id: 7,
            owner: 3,
            home_base: 11,
            name: Some(b"synthetic fleet".to_vec()).into(),
            leader: Some(0).into(),
        }]),
        TableId::OrdersGroups => TableRows::OrdersGroups(vec![rows::OrdersGroupRow {
            handle: 9,
            name: Some(b"synthetic command".to_vec()).into(),
            ..Default::default()
        }]),
        TableId::CountryAppearance => {
            TableRows::CountryAppearance(vec![rows::CountryAppearanceRow {
                country: 1,
                ..Default::default()
            }])
        }
        TableId::DivisionTemplates => {
            TableRows::DivisionTemplates(vec![rows::DivisionTemplateRow {
                template: 1,
                name: b"synthetic template".to_vec(),
                ..Default::default()
            }])
        }
        TableId::Decisions => TableRows::Decisions(vec![rows::DecisionRow {
            country: 1,
            token: b"synthetic_decision".to_vec(),
            ..Default::default()
        }]),
        TableId::DivisionMovement => TableRows::DivisionMovement(vec![rows::DivisionMovementRow {
            division: 1,
            source: 7,
            target: 8,
            ..Default::default()
        }]),
        TableId::OccupationSettings => {
            TableRows::OccupationSettings(vec![rows::OccupationSettingsRow {
                country: 1,
                default_law_present: 1,
                law: b"synthetic_law".to_vec(),
                ..Default::default()
            }])
        }
        TableId::StateGarrisons => TableRows::StateGarrisons(vec![rows::StateGarrisonRow {
            controller: 1,
            occupied: 2,
            state: 3,
            required_manpower: 100,
            strength_ratio: 7,
            garrison_required: 100000,
            serialized_flag: 0,
            ..Default::default()
        }]),
        TableId::Countries => TableRows::Countries(vec![rows::CountryRow {
            id: 1,
            tag: b"AAA".to_vec(),
            ..Default::default()
        }]),
        TableId::States => TableRows::States(vec![Default::default()]),
        TableId::AirAces => TableRows::AirAces(vec![Default::default()]),
        TableId::Operations => TableRows::Operations(vec![Default::default()]),
        TableId::ConstructionLineOutput => {
            TableRows::ConstructionLineOutput(vec![Default::default()])
        }
        TableId::ProductionLineOutput => TableRows::ProductionLineOutput(vec![Default::default()]),
        TableId::CountryFuel => TableRows::CountryFuel(vec![rows::CountryFuelRow {
            country: 1,
            current_raw: 17,
            ..Default::default()
        }]),
        TableId::LogisticsEquipment => {
            TableRows::LogisticsEquipment(vec![rows::LogisticsEquipmentRow {
                country: 1,
                archetype: 2,
                token: 3,
                balance_raw: -7,
                stock_raw: 11,
                ..Default::default()
            }])
        }
        TableId::LandCombatDetails if format.version() >= RECORDER_INPUTS_FORMAT => {
            TableRows::LandCombatDetails(vec![rows::LandCombatDetailsRow {
                combat: 1,
                border_war: rows::Recorded::Value(Some(rows::LandBorderWarInputs {
                    combat_width: rows::Recorded::Value(1_234_567),
                    ..Default::default()
                })),
                ..Default::default()
            }])
        }
        TableId::LandCombatDetails => TableRows::LandCombatDetails(vec![Default::default()]),
        TableId::NavalCombatDetails if format.version() >= RECORDER_INPUTS_FORMAT => {
            TableRows::NavalCombatDetails(vec![rows::NavalCombatDetailsRow {
                combat: 1,
                sides: std::array::from_fn(|_| rows::NavalCombatSideDetails {
                    commander_inputs: Some(rows::NavalCommanderInputs {
                        timed_traits: rows::Recorded::Value(vec![
                            rows::LandCombatCommanderTimedTrait::default(),
                        ]),
                        ..Default::default()
                    }),
                    units: vec![7, 8],
                    ..Default::default()
                }),
                custom_sea: Some(Vec::new()),
                ..Default::default()
            }])
        }
        TableId::NavalCombatDetails => {
            TableRows::NavalCombatDetails(vec![rows::NavalCombatDetailsRow {
                custom_sea: Some(Vec::new()),
                ..Default::default()
            }])
        }
        TableId::NavalCombatResultDetails => {
            TableRows::NavalCombatResultDetails(vec![Default::default()])
        }
        TableId::AdvisorRoles => TableRows::AdvisorRoles(vec![rows::AdvisorRoleRow {
            character: 1,
            country: 1,
            modifier_cache: rows::Recorded::Value(Some(rows::AdvisorModifierCache {
                entries: vec![rows::CachedModifier {
                    definition: 7,
                    value: -12,
                }],
            })),
            ..Default::default()
        }]),
        TableId::AvailableAdvisors => {
            TableRows::AvailableAdvisors(vec![rows::AvailableAdvisorRow {
                country: 1,
                character: 7,
                idea: b"synthetic_advisor".to_vec(),
                ..Default::default()
            }])
        }
        TableId::OwnedWargoals => TableRows::OwnedWargoals(vec![rows::OwnedWargoalRow {
            country: 1,
            recipient: 2,
            goal_type: -3,
            states: vec![4, 0, 8],
            ..Default::default()
        }]),
        TableId::CharacterOperatives => {
            TableRows::CharacterOperatives(vec![rows::CharacterOperativeRow {
                character: 1,
                reference: 1,
                ..Default::default()
            }])
        }
        TableId::OperativeMemberships => {
            TableRows::OperativeMemberships(vec![rows::OperativeMembershipRow {
                id: 1,
                roster_country: 1,
                ..Default::default()
            }])
        }
        TableId::CountryCounterInputs => {
            TableRows::CountryCounterInputs(vec![rows::CountryCounterInputRow {
                country: 1,
                ..Default::default()
            }])
        }
        TableId::EquipmentDestruction => {
            TableRows::EquipmentDestruction(vec![rows::EquipmentDestructionRow {
                country: 1,
                cause: rows::EquipmentDestructionCause::LandCombat,
                variant: (1_u64 << 32) | 70,
                amount_raw: 123_456,
                ..Default::default()
            }])
        }
        TableId::VariantDefinitions => {
            TableRows::VariantDefinitions(vec![rows::VariantDefinitionRow {
                id: 1,
                version: rows::Recorded::Value(7),
                ..Default::default()
            }])
        }
        TableId::ScriptVariables => TableRows::ScriptVariables(vec![rows::ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 1,
            ..Default::default()
        }]),
        TableId::TrainInventory => TableRows::TrainInventory(vec![rows::TrainInventoryRow {
            country: 1,
            ..Default::default()
        }]),
        TableId::LandCombatProgress => {
            TableRows::LandCombatProgress(vec![rows::LandCombatProgressRow {
                combat: 7 << 32 | 62,
                attacker_progress: 33_333,
                defender_progress: 66_667,
                direction_attacker: 9 << 32 | 4713,
                direction_province: 7,
                ..Default::default()
            }])
        }
        TableId::UnitLeaders => TableRows::UnitLeaders(vec![Default::default()]),
        TableId::AirWings => TableRows::AirWings(vec![Default::default()]),
        TableId::OrdersGroupCounterInputs => {
            TableRows::OrdersGroupCounterInputs(vec![Default::default()])
        }
        TableId::DivisionTemplateIcons => {
            TableRows::DivisionTemplateIcons(vec![Default::default()])
        }
        TableId::Scientists => TableRows::Scientists(vec![Default::default()]),
        TableId::CountryLeaderRoles => TableRows::CountryLeaderRoles(vec![Default::default()]),
        TableId::RailwayGunCounterInputs => {
            TableRows::RailwayGunCounterInputs(vec![Default::default()])
        }
        TableId::WeatherProvinces => TableRows::WeatherProvinces(vec![Default::default()]),
        TableId::StateFlags => TableRows::StateFlags(vec![Default::default()]),
        TableId::DivisionStatus => TableRows::DivisionStatus(vec![rows::DivisionStatusRow {
            division: 1,
            ..Default::default()
        }]),
        TableId::RaidTooltipOperands => TableRows::RaidTooltipOperands(vec![Default::default()]),
        TableId::Ships => TableRows::Ships(vec![rows::ShipRow {
            id: 1,
            ..Default::default()
        }]),
        _ => match source(1, hour).tables[table.index()].clone().into_inner() {
            TableView::Complete(rows) => rows,
            _ => unreachable!(),
        },
    }
}

#[test]
fn fuel_award_suffix_keeps_the_previous_prefix_readable_and_signed_receipts_exact() {
    let prior = rows::compatibility::ECONOMY_INPUTS_FORMAT - 1;
    let format = FormatVersion::read(prior).unwrap();
    assert_eq!(format.fields(TableId::CountryFuel), 19);
    let payload = legacy_frame(format, 1000, true);
    let decoded = FrameDecoder::new()
        .apply(&Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap())
        .unwrap();
    let old = &decoded.complete::<rows::tables::CountryFuel>().unwrap()[0];
    assert_eq!(old.current_raw, 17);
    assert_eq!(old.country_instance, rows::Recorded::NotRecorded);
    assert_eq!(old.awards, rows::Recorded::NotRecorded);
    assert_eq!(old.demand, rows::Recorded::NotRecorded);

    let row = rows::CountryFuelRow {
        country: 1,
        current_raw: 17,
        country_instance: rows::Recorded::Value(Some(7)),
        awards: rows::Recorded::Value(rows::FuelAwardWindow {
            country_instance: 7,
            continuity_start_hour: Some(977),
            reason: Vec::new(),
            hours: vec![rows::FuelAwardHour {
                hour: 1000,
                gains_raw: [i64::MIN, 0, 1, -1],
                own_gain_raw: 0,
                recipients: (0..5000)
                    .map(|index| rows::FuelAwardRecipient {
                        reference: index << 32 | 29,
                        owner: 8,
                        kind: 0,
                        amount_raw: if index == 0 { 0 } else { -1 },
                        name: Some(b"synthetic retained name".to_vec()),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
        }),
        ..Default::default()
    };
    let delta = row.delta(None).unwrap().unwrap();
    assert_eq!(delta.apply::<rows::CountryFuelRow>(None).unwrap(), row);
    let mut corrected = row.clone();
    let rows::Recorded::Value(window) = &mut corrected.awards else {
        panic!()
    };
    window.hours[0].recipients[4999].amount_raw = i64::MIN;
    assert_eq!(
        corrected
            .delta(Some(&row))
            .unwrap()
            .unwrap()
            .apply(Some(row))
            .unwrap(),
        corrected
    );
}

#[test]
fn logistics_available_suffix_preserves_the_previous_prefix_and_unknown_zero_distinction() {
    let format = FormatVersion::read(rows::compatibility::ECONOMY_INPUTS_FORMAT - 1).unwrap();
    assert_eq!(format.fields(TableId::LogisticsEquipment), 3);
    let payload = legacy_frame(format, 1000, true);
    let decoded = FrameDecoder::new()
        .apply(&Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap())
        .unwrap();
    let old = &decoded
        .complete::<rows::tables::LogisticsEquipment>()
        .unwrap()[0];
    assert_eq!((old.balance_raw, old.stock_raw), (-7, 11));
    assert_eq!(old.available_raw, rows::Recorded::NotRecorded);

    let unknown = rows::LogisticsEquipmentRow {
        available_raw: rows::Recorded::Value(None),
        ..old.clone()
    };
    let zero = rows::LogisticsEquipmentRow {
        available_raw: rows::Recorded::Value(Some(0)),
        ..unknown.clone()
    };
    assert_eq!(
        unknown
            .delta(Some(old))
            .unwrap()
            .unwrap()
            .apply(Some(old.clone()))
            .unwrap(),
        unknown
    );
    assert_eq!(
        zero.delta(Some(&unknown))
            .unwrap()
            .unwrap()
            .apply(Some(unknown))
            .unwrap(),
        zero
    );
    let negative = rows::LogisticsEquipmentRow {
        available_raw: rows::Recorded::Value(Some(-1)),
        ..zero
    };
    assert!(negative.delta(None).is_err());
}

// The fixtures contain synthetic rows in the proven historical field prefixes,
// not old row decoders. Only fields absent from that history are omitted.
fn legacy_frame(format: FormatVersion, hour: Hour, first: bool) -> Vec<u8> {
    legacy_frame_with_history(format, hour, first, &HISTORY)
}

fn legacy_frame_with_history(
    format: FormatVersion,
    hour: Hour,
    first: bool,
    history: &[LastChange; HISTORY.len()],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&hour.to_le_bytes());
    bytes.push(u8::from(first));
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    for table in TableId::ALL {
        if history[table.index()].present(format.version()) {
            bytes.extend_from_slice(&0_u64.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    for table in TableId::ALL {
        if !history[table.index()].present(format.version()) {
            continue;
        }
        let rows = legacy_rows(table, hour, format);
        bytes.push(0);
        bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        bytes.extend(rows.fixture_prefix(history[table.index()].fields(table, format.version())));
    }
    bytes
}

fn legacy_replay(format: u16) -> Replay {
    let mut records = records(1, &[1000]);
    let Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity.coverage_policy = 3;
    let current = Replay::from_records(CAMPAIGN.to_vec(), &[records]).unwrap();
    let payload = legacy_frame(FormatVersion::read(format).unwrap(), 1000, true);
    let replay = edited_block(&current, |raw| {
        raw.clear();
        raw.extend_from_slice(&1_u32.to_le_bytes());
        put_bytes(raw, &payload).unwrap();
    });
    let mut bytes = replay.bytes().to_vec();
    bytes[4..6].copy_from_slice(&format.to_le_bytes());
    Replay::open(bytes).unwrap()
}

#[test]
fn compatibility_format94_policy3_reads_appended_tables_with_absent_fields() {
    let replay = legacy_replay(94);
    assert_eq!(replay.format.version(), 94);
    assert_eq!(replay.identity.coverage_policy, 3);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing hour")
    };
    let countries = frame.complete::<rows::tables::Countries>().unwrap();
    assert_eq!(countries[0].tag, b"AAA");
    assert_eq!(
        frame.complete::<rows::tables::Clock>().unwrap()[0].hour,
        1000
    );
    let expected_gaps = TableId::ALL
        .into_iter()
        .map(|table| {
            usize::from(replay.format.table_refusal(table).is_some())
                + replay.format.field_gaps(table).count()
        })
        .sum::<usize>()
        + replay.format.nested_field_gaps().count();
    assert_eq!(replay.compatibility_gaps().len(), expected_gaps);
    for table in TableId::ALL {
        match replay.format.table_refusal(table) {
            Some(reason) => assert_eq!(
                *frame.tables[table.index()],
                TableView::Unavailable { reason }
            ),
            None => assert!(
                matches!(*frame.tables[table.index()], TableView::Complete(_)),
                "{table:?}"
            ),
        }
    }
    let selected = replay
        .read_block_selected(
            &replay.index[0],
            &[TableId::Clock, TableId::DivisionMovement],
        )
        .unwrap();
    assert!(matches!(
        selected[0].tables[TableId::Clock.index()],
        TableUpdate::Snapshot(_)
    ));
    assert!(
        matches!(&selected[0].tables[TableId::DivisionMovement.index()], TableUpdate::Snapshot(rows) if rows.rows::<rows::tables::DivisionMovement>().unwrap()[0].strategic_action == rows::Recorded::NotRecorded)
    );
    assert_eq!(
        frame.complete::<rows::tables::CountryAppearance>().unwrap()[0].subjects,
        rows::Recorded::NotRecorded
    );
    assert_eq!(
        frame.complete::<rows::tables::DivisionTemplates>().unwrap()[0].name_key,
        rows::Recorded::NotRecorded
    );
    assert_eq!(
        frame.complete::<rows::tables::Decisions>().unwrap()[0].description_seed,
        rows::Recorded::NotRecorded
    );
    let garrison = &frame.complete::<rows::tables::StateGarrisons>().unwrap()[0];
    assert_eq!(garrison.required_manpower, 100);
    assert_eq!(garrison.equipment, rows::Recorded::NotRecorded);
    let settings = frame
        .complete::<rows::tables::OccupationSettings>()
        .unwrap();
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0].law, b"synthetic_law");
    assert_eq!(settings[0].group_laws, rows::Recorded::NotRecorded);
    assert_eq!(settings[0].fallback_law, rows::Recorded::NotRecorded);
    assert!(
        replay
            .append(&[])
            .err()
            .unwrap()
            .to_string()
            .contains(&format!("Replay append requires format {FORMAT_VERSION}"))
    );
}

#[test]
fn compatibility_garrison_old_prefix_retains_values_without_equipment() {
    let format = FormatVersion::read(rows::compatibility::WIDGET_CAPTURE_FORMAT - 1).unwrap();
    let payload = legacy_frame(format, 1000, true);
    let frame = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
    let decoded = FrameDecoder::new().apply(&frame).unwrap();
    let row = &decoded.complete::<rows::tables::StateGarrisons>().unwrap()[0];
    assert_eq!((row.controller, row.occupied, row.state), (1, 2, 3));
    assert_eq!(
        (
            row.required_manpower,
            row.strength_ratio,
            row.garrison_required,
            row.serialized_flag
        ),
        (100, 7, 100_000, 0)
    );
    assert_eq!(row.equipment, rows::Recorded::NotRecorded);
    assert_eq!(row.not_recorded_fields(), 1 << 5);
}

#[test]
fn historical_positive_equipment_removals_decode_with_absent_supplemental_columns() {
    for version in [91, 94, 96, 97, 98, 99] {
        let replay = legacy_replay(version);
        let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
            panic!("missing historical hour")
        };
        let amounts = frame
            .complete::<rows::tables::EquipmentDestruction>()
            .unwrap();
        assert_eq!(amounts[0].amount_raw, 123_456);
        assert_eq!(amounts[0].variant, (1_u64 << 32) | 70);
        assert_eq!(amounts[0].coverage, rows::Recorded::NotRecorded);
        assert_eq!(amounts[0].unavailable_reason, rows::Recorded::NotRecorded);
        assert_eq!(amounts[0].not_recorded_fields(), (1 << 1) | (1 << 2));
        assert_eq!(
            amounts[0].cause,
            rows::EquipmentDestructionCause::LandCombat
        );
        assert_eq!(
            replay
                .format
                .field_gaps(TableId::EquipmentDestruction)
                .count(),
            2
        );
        assert!(
            frame
                .complete::<rows::tables::DivisionEquipmentDeletions>()
                .is_err()
        );
        for table in [TableId::EventFeedHistory, TableId::RocketProductionOutput] {
            let TableView::Unavailable { reason } = &*frame.tables[table.index()] else {
                panic!("{table:?} was invented for format {version}")
            };
            assert_eq!(reason, &replay.format.table_refusal(table).unwrap());
            assert!(reason.contains("not recorded"));
        }
        assert!(frame.complete::<rows::tables::Clock>().is_ok());
        let selected = replay
            .read_block_selected(&replay.index[0], &[TableId::EquipmentDestruction])
            .unwrap();
        assert!(
            matches!(&selected[0].tables[TableId::EquipmentDestruction.index()], TableUpdate::Snapshot(rows) if rows.rows::<rows::tables::EquipmentDestruction>().unwrap()[0].amount_raw == 123_456)
        );
    }
}

#[test]
fn equipment_destruction_format98_keeps_main_tables_fields_and_selected_framing() {
    let replay = legacy_replay(RECORDER_INPUTS_FORMAT);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing historical hour");
    };
    let tables = TableId::ALL
        .into_iter()
        .filter(|table| {
            [
                TableId::AvailableAdvisors,
                TableId::OwnedWargoals,
                TableId::LandCombatDetails,
                TableId::NavalCombatDetails,
            ]
            .contains(table)
                || table
                    .last_change()
                    .appended
                    .iter()
                    .any(|field| field.introduced == RECORDER_INPUTS_FORMAT)
        })
        .collect::<Vec<_>>();
    let selected = replay
        .read_block_selected(&replay.index[0], &tables)
        .unwrap();
    for table in tables {
        let mut expected = legacy_rows(table, 1000, replay.format);
        // The fixture writes the historical prefix, not today's appended operands.
        match &mut expected {
            TableRows::AdvisorRoles(rows) => {
                for row in rows {
                    row.tooltip_keys = rows::Recorded::NotRecorded;
                }
            }
            TableRows::UnitLeaders(rows) => {
                for row in rows {
                    row.formation_reference = rows::Recorded::NotRecorded;
                }
            }
            TableRows::OperativeMemberships(rows) => {
                for row in rows {
                    row.target_country = rows::Recorded::NotRecorded;
                    row.operation = rows::Recorded::NotRecorded;
                    row.name = rows::Recorded::NotRecorded;
                    row.cooldown_kind = rows::Recorded::NotRecorded;
                    row.cooldown_start_hour = rows::Recorded::NotRecorded;
                    row.cooldown_end_hour = rows::Recorded::NotRecorded;
                }
            }
            TableRows::DivisionStatus(rows) => {
                for row in rows {
                    row.country_ordinal = rows::Recorded::NotRecorded;
                    row.invalid_orders_reason = rows::Recorded::NotRecorded;
                    row.supply = rows::Recorded::NotRecorded;
                    row.attrition_inputs = rows::Recorded::NotRecorded;
                    row.dig_in_factor = rows::Recorded::NotRecorded;
                    row.name = rows::Recorded::NotRecorded;
                }
            }
            TableRows::CountryCounterInputs(rows) => {
                for row in rows {
                    row.mio_policies = rows::Recorded::NotRecorded;
                    row.neighbors = rows::Recorded::NotRecorded;
                }
            }
            _ => {}
        }
        assert!(!expected.is_empty(), "{table:?}");
        let missing_fields = table
            .last_change()
            .appended
            .iter()
            .filter(|field| field.introduced > replay.format.version())
            .fold(0u64, |mask, field| mask | (1u64 << field.index));
        for row in expected.iter() {
            assert_eq!(row.not_recorded_fields(), missing_fields, "{table:?}");
        }
        assert_eq!(
            replay.format.field_gaps(table).count(),
            missing_fields.count_ones() as usize,
            "{table:?}"
        );
        assert_eq!(
            *frame.tables[table.index()],
            TableView::Complete(expected.clone()),
            "{table:?}"
        );
        assert_eq!(
            selected[0].tables[table.index()],
            TableUpdate::Snapshot(expected),
            "{table:?}"
        );
    }
    assert!(replay.format.nested_field_gaps().next().is_none());
    // Existing prefixes remain readable; additions are named field/table gaps.
    assert_eq!(
        replay.compatibility_gaps().len(),
        TableId::ALL
            .into_iter()
            .map(|table| {
                usize::from(replay.format.table_refusal(table).is_some())
                    + replay.format.field_gaps(table).count()
            })
            .sum::<usize>()
            + replay.format.nested_field_gaps().count()
    );
}

#[test]
fn compatibility_land_appends_preserve_old_progress_and_mark_casualties_absent() {
    for version in [91, 94, 96, 97, 98, 99] {
        let replay = legacy_replay(version);
        let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
            panic!("missing hour")
        };
        let [progress] = frame
            .complete::<rows::tables::LandCombatProgress>()
            .unwrap()
        else {
            panic!("missing progress")
        };
        assert_eq!(progress.attacker_progress, 33_333);
        assert_eq!(progress.defender_progress, 66_667);
        assert_eq!(progress.direction_attacker, 9 << 32 | 4713);
        assert_eq!(progress.direction_province, 7);
        assert_eq!(progress.attacker_manpower_loss, rows::Recorded::NotRecorded);
        assert_eq!(progress.defender_manpower_loss, rows::Recorded::NotRecorded);
        assert_eq!(
            progress.attacker_equipment_losses,
            rows::Recorded::NotRecorded
        );
        assert_eq!(
            progress.defender_equipment_losses,
            rows::Recorded::NotRecorded
        );
        for table in [
            TableId::DivisionLandInputs,
            TableId::LandCombatResults,
            TableId::DestroyedDivisions,
            TableId::LandEventCoverage,
        ] {
            let TableView::Unavailable { reason } = &*frame.tables[table.index()] else {
                panic!("{table:?} was invented for format {version}")
            };
            assert!(reason.contains("not recorded"), "{reason}");
        }
    }
    // Historical inserts and changes keep the four-field prefix too.
    let mut delta = rows::DeltaRow {
        key: 7 << 32 | 62,
        format: 98,
        mask: 0xf,
        values: Vec::new(),
        recorded_fields: 4,
    };
    // Numeric changes use zigzag varints, with zero as the insertion baseline.
    for value in [33_333_u128, 66_667, 9 << 32 | 4713, 7] {
        crate::codec::put_varint(&mut delta.values, value * 2);
    }
    let first = delta.apply::<rows::LandCombatProgressRow>(None).unwrap();
    assert_eq!(first.attacker_manpower_loss, rows::Recorded::NotRecorded);
    assert_eq!(first.attacker_equipment_losses, rows::Recorded::NotRecorded);
    delta.mask = 1;
    delta.values.clear();
    crate::codec::put_varint(&mut delta.values, (50_000 - 33_333) * 2);
    let next = delta
        .apply::<rows::LandCombatProgressRow>(Some(first))
        .unwrap();
    assert_eq!(next.attacker_progress, 50_000);
    assert_eq!(next.defender_manpower_loss, rows::Recorded::NotRecorded);
    assert_eq!(next.defender_equipment_losses, rows::Recorded::NotRecorded);
}

#[test]
fn compatibility_format91_framing_survives_snapshot_delta_truncation_and_unchanged() {
    let format = FormatVersion::read(91).unwrap();
    let table = TableId::DivisionStatus;
    assert_eq!(format.fields(table), 16); // old delta mask was TWO bytes, not three
    let payload = legacy_frame(format, 1000, true);
    let mut decoder = FrameDecoder::new();
    let first = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
    decoder.advance(&first).unwrap();
    for (i, status) in [1, 2, 3, 2, 4].into_iter().enumerate() {
        let hour = 1001 + i as i32;
        let mut update = vec![status];
        match status {
            1 => {
                update.extend_from_slice(&(hour - 1).to_le_bytes());
                update.extend_from_slice(&1_u32.to_le_bytes());
                update.push(1); // key
                update.extend_from_slice(&[0, 0x80]); // historical 16-field mask
                update.push(2); // move_flags += 1, zigzag-encoded
                update.extend_from_slice(&0_u32.to_le_bytes()); // no deletions
            }
            2 => update.extend_from_slice(&(hour - 1).to_le_bytes()),
            3 => {
                update.extend_from_slice(&5_u32.to_le_bytes());
                let rows = legacy_rows(table, hour, format);
                update.extend_from_slice(&(rows.len() as u32).to_le_bytes());
                update.extend(rows.fixture_prefix(format.fields(table)));
            }
            4 => put_string(&mut update, "synthetic unavailable").unwrap(),
            _ => unreachable!(),
        }
        let mut payload = legacy_frame(format, hour, false);
        let prefix = 37
            + 8 * TableId::ALL
                .into_iter()
                .filter(|table| table.last_change().present(format.version()))
                .count();
        let mut input = Decoder::new(&payload[prefix..]);
        let mut range = 0..0;
        for id in TableId::ALL
            .into_iter()
            .filter(|table| table.last_change().present(format.version()))
        {
            let start = payload.len() - input.remaining();
            assert_eq!(input.u8().unwrap(), 0);
            id.skip_snapshot(format.fields(id), &mut input).unwrap();
            let end = payload.len() - input.remaining();
            if id == table {
                range = start..end;
            }
        }
        input.finish().unwrap();
        payload.splice(range, update);
        let frame = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
        let decoded = decoder.advance(&frame).unwrap();
        assert_eq!(
            decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
            hour
        );
        match status {
            1 => {
                let rows = decoded.complete::<rows::tables::DivisionStatus>().unwrap();
                assert_eq!(rows[0].move_flags, 1);
                assert_eq!(rows[0].border_combat_stats, rows::Recorded::NotRecorded);
            }
            3 => assert!(
                matches!(&*decoded.tables[table.index()], TableView::Truncated { missing: 5, rows } if rows.rows::<rows::tables::DivisionStatus>().unwrap()[0].border_combat_stats == rows::Recorded::NotRecorded)
            ),
            2 if i == 1 => {
                let rows = decoded.complete::<rows::tables::DivisionStatus>().unwrap();
                assert_eq!(rows[0].move_flags, 1);
                assert_eq!(rows[0].border_combat_stats, rows::Recorded::NotRecorded);
            }
            2 | 4 => assert!(matches!(
                &*decoded.tables[table.index()],
                TableView::Unavailable { .. }
            )),
            _ => unreachable!(),
        }
        assert_eq!(
            format.table_refusal(TableId::RaidTooltipOperands).unwrap(),
            "RaidTooltipOperands layout or meaning changed at format 94; this recording is format 91"
        );
        assert_eq!(
            *decoded.tables[TableId::RaidTooltipOperands.index()],
            TableView::Unavailable {
                reason: format.table_refusal(TableId::RaidTooltipOperands).unwrap()
            }
        );
        assert!(matches!(
            &*decoded.tables[TableId::Ships.index()],
            TableView::Complete(_)
        ));
    }
}

#[test]
fn compatibility_historical_delta_insertion_keeps_the_appended_field_absent() {
    let format = FormatVersion::read(91).unwrap();
    let table = TableId::DivisionStatus;
    let row = rows::DivisionStatusRow {
        division: 2,
        move_flags: 7,
        ..Default::default()
    };
    let mut delta = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&delta.values);
    for kind in &table.schema()[..format.fields(table)] {
        kind.decode_change(&mut input).unwrap();
    }
    delta
        .values
        .truncate(delta.values.len() - input.remaining());
    delta.format = format.version();
    delta.mask = (1 << format.fields(table)) - 1;
    delta.recorded_fields = format.fields(table);
    let inserted = delta.apply::<rows::DivisionStatusRow>(None).unwrap();
    assert_eq!(inserted.move_flags, 7);
    assert_eq!(inserted.border_combat_stats, rows::Recorded::NotRecorded);
    let mut bytes = vec![0x55];
    assert_eq!(
        rows::encode_delta(table, &delta, 0, &mut bytes)
            .unwrap_err()
            .to_string(),
        "historical fields cannot be re-encoded"
    );
    assert_eq!(bytes, [0x55]);
    delta.mask &= !(1 << 15);
    assert!(delta.apply::<rows::DivisionStatusRow>(None).is_err());
}

#[test]
fn compatibility_every_appended_field_is_explicitly_not_recorded() {
    let format = FormatVersion::read(91).unwrap();
    let payload = legacy_frame(format, 1000, true);
    let frame = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
    let decoded = FrameDecoder::new().apply(&frame).unwrap();
    for table in TableId::ALL {
        let appended = table.last_change().appended;
        if appended.is_empty() || format.table_refusal(table).is_some() {
            continue;
        }
        if !table.last_change().present(format.version()) {
            assert_eq!(
                *decoded.tables[table.index()],
                TableView::Unavailable {
                    reason: format.table_refusal(table).unwrap()
                },
                "{table:?} did not exist in the historical stream"
            );
            continue;
        }
        let TableView::Complete(rows) = &*decoded.tables[table.index()] else {
            panic!("{table:?} was withheld");
        };
        assert!(!rows.is_empty(), "{table:?}");
        let expected = appended
            .iter()
            .fold(0, |mask, field| mask | (1 << field.index));
        for row in rows.iter() {
            assert_eq!(row.not_recorded_fields(), expected, "{table:?}");
        }
        assert_eq!(rows.field_gaps().count(), appended.len());
    }
    assert!(frame.table_encoded_sizes().is_ok());
    // Reader compatibility never lets a writer turn an absent field into a value.
    assert!(frame.encode(&mut Vec::new()).is_err());
}

#[test]
fn compatibility_layout_change_still_withholds_the_whole_table() {
    let mut history = HISTORY;
    let table = TableId::CountryAppearance;
    history[table.index()].kind = rows::compatibility::ChangeKind::Layout;
    let format = FormatVersion::read(94).unwrap();
    let payload = legacy_frame_with_history(format, 1000, true, &history);
    let parsed =
        Frame::decode_with_history(&mut Decoder::new(&payload), None, format, &history).unwrap();
    let decoded = FrameDecoder::new().apply(&parsed).unwrap();
    assert!(
        matches!(&*decoded.tables[table.index()], TableView::Unavailable { reason } if reason == &format!("CountryAppearance layout or meaning changed at format {}; this recording is format 94", history[table.index()].format))
    );
    assert!(decoded.complete::<rows::tables::Countries>().is_ok());
}

#[test]
fn compatibility_table_lifetimes_keep_the_old_frame_stream_readable() {
    // Synthetic introduction/retirement independent of the actual table lifetimes.
    let mut history = HISTORY;
    let added = TableId::NavalModifierBreakdowns;
    history[added.index()].introduced = 96;
    history[added.index()].format = 96;
    let retired = TableId::Ships;
    history[retired.index()].retired = Some(96);
    for version in [94, 97] {
        let format = FormatVersion::read(version).unwrap();
        let payload = legacy_frame_with_history(format, 1000, true, &history);
        let mut input = Decoder::new(&payload);
        let frame = Frame::decode_with_history(&mut input, None, format, &history).unwrap();
        input.finish().unwrap();
        let mut decoder = FrameDecoder::new();
        let decoded = decoder.advance(&frame).unwrap();
        assert_eq!(
            decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
            1000
        );
        assert_eq!(
            *decoded.tables[retired.index()],
            TableView::Unavailable {
                reason: format!("Ships retired at format 96; this recording is format {version}")
            }
        );
        if version == 94 {
            assert_eq!(*decoded.tables[added.index()], TableView::Unavailable {
                reason: "NavalModifierBreakdowns not recorded at format 94 (introduced at format 96)".into()
            });
        } else {
            assert!(matches!(
                &*decoded.tables[added.index()],
                TableView::Complete(_)
            ));
        }
        assert!(matches!(
            &*decoded.tables[TableId::Fleets.index()],
            TableView::Complete(_)
        ));
    }
}

#[test]
fn compatibility_framing_change_refuses_the_whole_file_by_name() {
    let replay = legacy_replay(94);
    let mut bytes = replay.bytes().to_vec();
    bytes[4..6].copy_from_slice(&(PROVEN_FROM - 1).to_le_bytes());
    assert!(
        Replay::open(bytes)
            .err()
            .unwrap()
            .to_string()
            .contains("format 90 predates the proven compatibility history")
    );
    let mut bytes = replay.bytes().to_vec();
    bytes[..4].copy_from_slice(b"BAD!");
    assert_eq!(
        Replay::open(bytes).err().unwrap().to_string(),
        "invalid replay magic"
    );
    let mut records = records(1, &[1000]);
    records.push(Record::Control(ControlRecord::Partial {
        session: 1,
        reason: "synthetic failure".into(),
        failed_hour: None,
    }));
    let current = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    let mut bytes = current.bytes().to_vec();
    bytes[4..6].copy_from_slice(&91_u16.to_le_bytes());
    assert_eq!(
        Replay::open(bytes).err().unwrap().to_string(),
        "Partial layout changed at format 92; this recording is format 91"
    );
}

#[test]
fn compatibility_old_spool_is_readable_but_cannot_be_rewritten_or_mixed() {
    let format = FormatVersion::read(94).unwrap();
    let mut bytes = Vec::new();
    for record in records(1, &[1000]) {
        let mut encoded = encode_record(&record).unwrap();
        if matches!(record, Record::Frame(_)) {
            encoded.truncate(crate::spool::RECORD_HEADER_BYTES);
            encoded.push(2);
            encoded.extend(legacy_frame(format, 1000, true));
            let length = encoded.len() - crate::spool::RECORD_HEADER_BYTES;
            encoded[6..10].copy_from_slice(&(length as u32).to_le_bytes());
            let crc = crc32fast::hash(&encoded[crate::spool::RECORD_HEADER_BYTES..]);
            encoded[10..14].copy_from_slice(&crc.to_le_bytes());
        }
        encoded[4..6].copy_from_slice(&94_u16.to_le_bytes());
        bytes.extend(encoded);
    }
    let recovered = recover_prefix(&bytes);
    assert_eq!(recovered.format, Some(format));
    assert!(recovered.damage.is_none(), "{:?}", recovered.damage);
    let current = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1000])]).unwrap();
    assert!(
        current
            .append_recovered(&[(0, recover_prefix(&bytes))])
            .err()
            .unwrap()
            .to_string()
            .contains("segment 0 is format 94")
    );
    assert!(
        Replay::from_recovered(CAMPAIGN.into(), &[(0, recovered)])
            .err()
            .unwrap()
            .to_string()
            .contains("historical rows are never rewritten")
    );
    bytes.extend(
        encode_record(&Record::Control(ControlRecord::ClockWitness {
            session: 1,
            qpc: 0,
            hour: 1000,
            load_signal: None,
        }))
        .unwrap(),
    );
    assert!(
        recover_prefix(&bytes)
            .damage
            .unwrap()
            .contains("SpoolEnvelope format changed inside a segment")
    );
}

#[test]
#[ignore = "read-only local recordings; BATTLE_PLAN_COMPAT_RECORDINGS supplies paths"]
fn compatibility_real_recordings() {
    let paths = std::env::var("BATTLE_PLAN_COMPAT_RECORDINGS").or_else(|_| std::env::var("HOI4_COMPAT_RECORDINGS")).expect("semicolon-separated paths");
    for path in paths.split(';') {
        let replay = Replay::open(std::fs::read(path).unwrap()).unwrap();
        println!(
            "RECORDING {path} format={} policy={} blocks={} branches={}",
            replay.format.version(),
            replay.identity.coverage_policy,
            replay.index.len(),
            replay.branches.len()
        );
        for gap in replay.compatibility_gaps() {
            println!("COMPAT_GAP {gap}");
        }
        let mut frames = 0;
        let mut gaps = BTreeMap::<TableId, BTreeSet<String>>::new();
        let mut fields = BTreeMap::<String, usize>::new();
        for index in &replay.index {
            let mut decoder = FrameDecoder::new();
            for frame in replay.read_block(index).unwrap() {
                let decoded = decoder.advance(&frame).unwrap();
                frames += 1;
                for table in TableId::ALL {
                    match &*decoded.tables[table.index()] {
                        TableView::Unavailable { reason } => {
                            gaps.entry(table).or_default().insert(reason.clone());
                        }
                        TableView::Truncated { missing, .. } => {
                            gaps.entry(table)
                                .or_default()
                                .insert(format!("{missing} missing rows"));
                        }
                        TableView::Complete(rows) => {
                            for (field, count) in rows.field_gaps() {
                                *fields.entry(format!("{table:?}.{field}")).or_default() += count;
                            }
                        }
                    }
                }
            }
        }
        println!("FRAMES {frames}");
        for (field, count) in fields {
            println!("FIELD_GAP {field} {count} row samples not recorded");
        }
        for (table, reasons) in gaps {
            println!(
                "TABLE_GAP {table:?} {}",
                reasons.into_iter().collect::<Vec<_>>().join(" | ")
            );
        }
    }
}

#[test]
fn command_name_suffixes_preserve_old_prefix_and_explicit_absence() {
    let format = FormatVersion::read(98).unwrap();
    let payload = legacy_frame(format, 1000, true);
    let frame = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
    let mut decoder = FrameDecoder::new();
    let decoded = decoder.advance(&frame).unwrap();
    let fleet = &decoded.complete::<rows::tables::Fleets>().unwrap()[0];
    assert_eq!((fleet.id, fleet.owner, fleet.home_base), (7, 3, 11));
    assert_eq!(fleet.name, rows::Recorded::NotRecorded);
    assert_eq!(fleet.leader, rows::Recorded::NotRecorded);
    let group = &decoded.complete::<rows::tables::OrdersGroups>().unwrap()[0];
    assert_eq!(group.handle, 9);
    assert_eq!(group.name, rows::Recorded::NotRecorded);
}

#[test]
fn compatibility_previous_format_keeps_positive_destruction_and_refuses_only_new_air_ledgers() {
    let replay = legacy_replay(rows::compatibility::AIR_LEDGER_FORMAT - 1);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing previous-format hour")
    };
    assert_eq!(
        frame.complete::<rows::tables::Clock>().unwrap()[0].hour,
        1000
    );
    assert_eq!(
        frame.complete::<rows::tables::Countries>().unwrap()[0].tag,
        b"AAA"
    );
    assert_eq!(
        frame
            .complete::<rows::tables::EquipmentDestruction>()
            .unwrap()[0]
            .amount_raw,
        123_456
    );
    for table in [
        TableId::AirLossEvents,
        TableId::BuildingBombingDamage,
        TableId::AirDailyRoster,
        TableId::RocketProductionOutput,
    ] {
        let TableView::Unavailable { reason } = &*frame.tables[table.index()] else {
            panic!("an unrecorded ledger is not a complete empty interval")
        };
        assert!(reason.contains("not recorded"));
    }
}
