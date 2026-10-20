use super::*;
use crate::{
    format::FormatVersion,
    rows::{
        self, RowCodec,
        compatibility::{
            HISTORY, LastChange, PROVEN_FROM, RECORDER_INPUTS_FORMAT, RETIRED_INPUTS_FORMAT,
        },
    },
};
use std::collections::BTreeSet;

fn legacy_rows(table: TableId, hour: Hour, format: FormatVersion) -> TableRows {
    match table {
        TableId::FleetCounterInputs => TableRows::FleetCounterInputs(vec![Default::default()]),
        TableId::RaidCountryInputs => TableRows::RaidCountryInputs(vec![Default::default()]),
        TableId::AceRangeInputs => TableRows::AceRangeInputs(vec![Default::default()]),
        TableId::NavalMissionCountries => {
            TableRows::NavalMissionCountries(vec![Default::default()])
        }
        TableId::NavalMissionCommanders => {
            TableRows::NavalMissionCommanders(vec![Default::default()])
        }
        TableId::VolunteerHosts => TableRows::VolunteerHosts(vec![Default::default()]),
        TableId::ResearchSlots => TableRows::ResearchSlots(vec![rows::ResearchSlotRow {
            country: 1,
            technology_handle: 7,
            technology_present: 1,
            database_index: rows::Recorded::Value(2),
            ..Default::default()
        }]),
        TableId::ResearchInputs => TableRows::ResearchInputs(vec![rows::ResearchInputRow {
            country: 1,
            technology_handle: 7,
            technology_present: 1,
            token: b"synthetic_technology".to_vec(),
            database_index: rows::Recorded::Value(2),
            ..Default::default()
        }]),
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
        TableId::RelationLinks => TableRows::RelationLinks(vec![rows::RelationLinkRow {
            country: 1,
            target_index: 2,
            links: rows::RelationLinkRow::MILITARY_ACCESS,
            ..Default::default()
        }]),
        TableId::CountryAppearance => {
            TableRows::CountryAppearance(vec![rows::CountryAppearanceRow {
                country: 1,
                ..Default::default()
            }])
        }
        TableId::CountryWars => TableRows::CountryWars(vec![rows::CountryWarRow {
            country: 1,
            can_occupy_non_war: 1,
            enemies_read: 1,
            enemies: vec![2, 3],
            ordered: if format.version() >= rows::compatibility::POLITICAL_FORMAT {
                Some(rows::OrderedEnemyInputs {
                    sample_hour: rows::PacketHour::Host,
                    entries: vec![3, 2],
                })
                .into()
            } else {
                Default::default()
            },
        }]),
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
        TableId::DivisionEtaInputs => {
            TableRows::DivisionEtaInputs(vec![rows::DivisionEtaInputsRow {
                division: 1,
                sample_hour: hour,
                speed_base: 100_000,
                ..Default::default()
            }])
        }
        TableId::ArmyOrderPlans => TableRows::ArmyOrderPlans(vec![rows::ArmyOrderPlanRow {
            owner_group: 1,
            instance: 1,
            name_postfix: b"synthetic postfix".to_vec(),
            ..Default::default()
        }]),
        TableId::TaskForceMapMotion => {
            TableRows::TaskForceMapMotion(vec![rows::TaskForceMapMotionRow {
                task_force: 1,
                target: 8,
                path: vec![7, 8],
                alternate_path: vec![9, 8],
                ..Default::default()
            }])
        }
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
        TableId::Provinces => TableRows::Provinces(vec![rows::ProvinceRow {
            id: 1,
            state: 7,
            controller: 8,
            ..Default::default()
        }]),
        TableId::WeatherRegions => TableRows::WeatherRegions(vec![rows::WeatherRegionRow {
            id: 1,
            max_snow: i64::MIN,
            ..Default::default()
        }]),
        TableId::States => TableRows::States(vec![rows::StateRow {
            id: 1,
            owner: 7,
            controller: 8,
            movement_infrastructure: 100_000,
            ..Default::default()
        }]),
        TableId::RailCooldowns => TableRows::RailCooldowns(vec![rows::RailCooldownRow {
            province: 1,
            cooldown_ticks: -1,
            ..Default::default()
        }]),
        TableId::AirAces => TableRows::AirAces(vec![Default::default()]),
        TableId::Politics => TableRows::Politics(vec![Default::default()]),
        TableId::Operations => TableRows::Operations(vec![Default::default()]),
        TableId::ConstructionLineOutput => {
            TableRows::ConstructionLineOutput(vec![Default::default()])
        }
        TableId::NavalBases => TableRows::NavalBases(vec![Default::default()]),
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
        // Older formats end before the reserve speed share.
        TableId::LandCombatDivisions => {
            TableRows::LandCombatDivisions(vec![rows::LandCombatDivisionRow {
                combat: 1,
                ..Default::default()
            }])
        }
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
                forward_visibility: crate::rows::Recorded::Value(None),
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
                attacker_progress: rows::RetiredI64::holding(33_333),
                defender_progress: rows::RetiredI64::holding(66_667),
                direction_attacker: rows::RetiredU64::holding(9 << 32 | 4713),
                direction_province: rows::RetiredU32::holding(7),
                ..Default::default()
            }])
        }
        TableId::UnitLeaders => TableRows::UnitLeaders(vec![Default::default()]),
        TableId::AirWings => TableRows::AirWings(vec![Default::default()]),
        TableId::AirMissionInputs => TableRows::AirMissionInputs(vec![rows::AirMissionInputRow {
            wing: 1,
            ..Default::default()
        }]),
        TableId::AirNavalPatrolHistories => {
            TableRows::AirNavalPatrolHistories(vec![rows::AirNavalPatrolHistoryRow {
                country: 1,
                region: 7,
                enemy_ships_sunk_raw: 1,
                ..Default::default()
            }])
        }
        TableId::AirBases => TableRows::AirBases(vec![Default::default()]),
        TableId::AirBaseRegistrations => TableRows::AirBaseRegistrations(vec![Default::default()]),
        TableId::OrdersGroupCounterInputs => {
            TableRows::OrdersGroupCounterInputs(vec![Default::default()])
        }
        TableId::TaskForceCounterInputs => {
            TableRows::TaskForceCounterInputs(vec![rows::TaskForceCounterInputRow {
                task_force: 1,
                attached_headquarters: 0,
                ..Default::default()
            }])
        }
        TableId::DivisionTemplateIcons => {
            TableRows::DivisionTemplateIcons(vec![Default::default()])
        }
        TableId::ProjectPrograms => TableRows::ProjectPrograms(vec![Default::default()]),
        TableId::Projects => TableRows::Projects(vec![Default::default()]),
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
        TableId::Capitulation => TableRows::Capitulation(vec![Default::default()]),
        TableId::CountryStabilityInputs => {
            TableRows::CountryStabilityInputs(vec![rows::CountryStabilityInputRow {
                country: 1,
                keys: vec![u32::MAX; rows::CountryStabilityInputRow::KEY_NAMES.len()],
                ..Default::default()
            }])
        }
        TableId::StateResourceInputs => TableRows::StateResourceInputs(vec![Default::default()]),
        TableId::StateCores => TableRows::StateCores(vec![Default::default()]),
        TableId::RaidTooltipOperands => TableRows::RaidTooltipOperands(vec![Default::default()]),
        TableId::RaidAirScores => TableRows::RaidAirScores(vec![rows::RaidAirScoreRow {
            country: 1,
            region: 7,
            friendly: 2,
            enemy: 3,
            ..Default::default()
        }]),
        TableId::Ships => TableRows::Ships(vec![rows::ShipRow {
            id: 1,
            ..Default::default()
        }]),
        TableId::ShipModels => TableRows::ShipModels(vec![rows::ShipModelRow {
            ship: 1,
            ..Default::default()
        }]),
        _ => match source(1, hour).tables[table.index()].clone().into_inner() {
            TableView::Complete(value)
                if value.is_empty() && !table.last_change().appended.is_empty() =>
            {
                // New appended tables need a real synthetic row too; the
                // general frame fixture intentionally leaves many empty.
                rows::sample_extended_table(table, 1).unwrap_or(value)
            }
            TableView::Complete(rows) => rows,
            // A table today's format retired was recorded by older writers.
            _ if crate::frame::current_absence(table).is_some() => TableRows::empty(table),
            _ => unreachable!(),
        },
    }
}

#[test]
fn format_101_transport_packets_and_split_tables_are_refused_by_name() {
    let previous = FormatVersion::read(rows::compatibility::RESERVED_SPLIT_FORMAT - 1).unwrap();
    let payload = legacy_frame(previous, 1000, true);
    let old = FrameDecoder::new()
        .apply(&Frame::decode_version(&mut Decoder::new(&payload), None, previous).unwrap())
        .unwrap();
    // Neither format records the split tables; each refusal names its table.
    for format in [previous, FormatVersion::CURRENT] {
        for table in rows::RESERVED_SPLIT_TABLES {
            assert!(!table.last_change().present(format.version()));
            let reason = format.table_refusal(table).unwrap();
            assert!(reason.contains(&format!("{table:?}")), "{reason}");
        }
    }
    for table in rows::RESERVED_SPLIT_TABLES {
        assert!(
            FormatVersion::CURRENT
                .table_refusal(table)
                .unwrap()
                .contains("retired at format")
        );
        assert!(matches!(
            &*old.tables[table.index()],
            TableView::Unavailable { reason } if reason.contains("not recorded")
        ));
    }
    // The singleton itself stays readable; its packet field is a tombstone.
    assert!(
        previous
            .table_refusal(TableId::ReservedCountryNaming)
            .is_none()
    );
    let row = &old
        .complete::<rows::tables::ReservedCountryNaming>()
        .unwrap()[0];
    assert_eq!(
        row.transport_mission_inputs,
        rows::Recorded::Value(rows::Retired)
    );
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
    assert_eq!(old.award_day, rows::Recorded::NotRecorded);
    // Recordings before the day sums hold the update-by-update window, which
    // decodes through its framing and is discarded.
    assert_eq!(
        FormatVersion::read(rows::compatibility::POLITICAL_FORMAT)
            .unwrap()
            .fields(TableId::CountryFuel),
        22
    );

    let row = rows::CountryFuelRow {
        country: 1,
        current_raw: 17,
        country_instance: rows::Recorded::Value(Some(7)),
        award_day: rows::Recorded::Value(rows::FuelAwardDay {
            country_instance: 7,
            continuity_start_hour: Some(977),
            last_hour: 1000,
            gains_raw: [i64::MIN, 0, 1, -1],
            recipients: (0..5000)
                .map(|index| rows::FuelAwardDayRecipient {
                    reference: index << 32 | 29,
                    owner: 8,
                    kind: 0,
                    first_hour: 977,
                    last_hour: 1000,
                    amount_raw: if index == 0 { 0 } else { -1 },
                    name: Some(b"synthetic retained name".to_vec()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let delta = row.delta(None).unwrap().unwrap();
    assert_eq!(delta.apply::<rows::CountryFuelRow>(None).unwrap(), row);
    let mut corrected = row.clone();
    let rows::Recorded::Value(day) = &mut corrected.award_day else {
        panic!()
    };
    day.recipients[4999].amount_raw = i64::MIN;
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
        let fields = history[table.index()].fields(table, format.version());
        // Stamped formats wrote every hourly-host packet with its frame hour.
        bytes.extend(if format.version() <= rows::STAMPED_PACKET_FORMAT {
            rows::with_historical_stamp(hour, || rows.fixture_prefix(fields))
        } else {
            rows.fixture_prefix(fields)
        });
    }
    bytes
}

fn replace_legacy_snapshot(
    format: FormatVersion,
    mut payload: Vec<u8>,
    table: TableId,
    update: &[u8],
) -> Vec<u8> {
    let prefix = 37
        + 8 * TableId::ALL
            .into_iter()
            .filter(|table| table.last_change().present(format.version()))
            .count();
    let mut input = Decoder::new(&payload[prefix..]);
    let mut range = None;
    for id in TableId::ALL
        .into_iter()
        .filter(|table| table.last_change().present(format.version()))
    {
        let start = payload.len() - input.remaining();
        assert_eq!(input.u8().unwrap(), 0);
        id.skip_snapshot(format.fields(id), &mut input).unwrap();
        let end = payload.len() - input.remaining();
        if id == table {
            range = Some(start..end);
        }
    }
    input.finish().unwrap();
    payload.splice(
        range.expect("fixture table is present"),
        update.iter().copied(),
    );
    payload
}

fn legacy_replay(format: u16) -> Replay {
    legacy_replay_with(
        format,
        &legacy_frame(FormatVersion::read(format).unwrap(), 1000, true),
    )
}

/// A file of `format` whose one block holds `payload`, hour 1000's frame.
fn legacy_replay_with(format: u16, payload: &[u8]) -> Replay {
    let mut records = records(1, &[1000]);
    let Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity.coverage_policy = 3;
    let current = Replay::from_records(CAMPAIGN.to_vec(), &[records]).unwrap();
    let replay = edited_block(&current, |raw| {
        raw.clear();
        raw.extend_from_slice(&1_u32.to_le_bytes());
        put_bytes(raw, payload).unwrap();
    });
    let mut bytes = replay.bytes().to_vec();
    bytes[4..6].copy_from_slice(&format.to_le_bytes());
    Replay::open(bytes).unwrap()
}

#[test]
fn older_recording_opens_without_weather_display_boundary() {
    let replay = legacy_replay(rows::compatibility::EFFECT_INPUTS_FORMAT);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing historical hour")
    };
    let weather = frame.complete::<rows::tables::WeatherProvinces>().unwrap();
    assert_eq!(weather.len(), 1);
    assert_eq!(weather[0].display, rows::Recorded::NotRecorded);
    assert_eq!(weather[0].snow, 0);
    assert_eq!(weather[0].clock, rows::DayNightClock::NOT_RECORDED);
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
fn compatibility_published_air_history_updates_preserve_prefixes_and_siblings() {
    let version = rows::compatibility::WIDGET_CAPTURE_FORMAT;
    let format = FormatVersion::read(version).unwrap();
    let table = TableId::AirNavalPatrolHistories;
    let key = (1_u128 << 64) | (7_u128 << 32);
    let mut previous = rows::AirNavalPatrolHistoryRow {
        country: 1,
        region: 7,
        enemy_ships_sunk_raw: 1,
        ..Default::default()
    };
    let mut changed = rows::AirNavalPatrolHistoryRow {
        enemy_ships_sunk_raw: 300_000,
        our_fighters_lost_raw: 200_000,
        ..previous.clone()
    };
    let delta = changed.delta(Some(&previous)).unwrap().unwrap();
    previous.scenario_channels = rows::Recorded::NotRecorded;
    changed.scenario_channels = rows::Recorded::NotRecorded;
    // Main's published format has two qwords and no scenario suffix.
    for status in 0..=4 {
        let hour = 1001 + i32::from(status);
        let mut update = vec![status];
        match status {
            0 | 3 => {
                if status == 3 {
                    update.extend_from_slice(&1_u32.to_le_bytes());
                }
                update.extend_from_slice(&1_u32.to_le_bytes());
                table.encode_key(key, 0, &mut update).unwrap();
                update.extend_from_slice(&300_000_i64.to_le_bytes());
                update.extend_from_slice(&200_000_i64.to_le_bytes());
            }
            1 => {
                update.extend_from_slice(&1000_i32.to_le_bytes());
                update.extend_from_slice(&1_u32.to_le_bytes());
                table.encode_key(key, 0, &mut update).unwrap();
                update.push(delta.mask as u8);
                update.extend_from_slice(&delta.values);
                update.extend_from_slice(&0_u32.to_le_bytes());
            }
            2 => update.extend_from_slice(&1000_i32.to_le_bytes()),
            4 => put_string(&mut update, "synthetic history unavailable").unwrap(),
            _ => unreachable!(),
        }
        let payload = replace_legacy_snapshot(
            format,
            legacy_frame(format, hour, status == 0),
            table,
            &update,
        );
        for selected in [None, Some(&[TableId::Clock, table][..])] {
            let mut input = Decoder::new(&payload);
            let frame = Frame::decode_version(&mut input, selected, format).unwrap();
            input.finish().unwrap();
            assert_eq!(
                frame.table_encoded_sizes_version(format).unwrap()[table.index()],
                update.len()
            );
            let baseline = Frame::decode_version(
                &mut Decoder::new(&legacy_frame(format, 1000, true)),
                selected,
                format,
            )
            .unwrap();
            let mut decoder = FrameDecoder::new();
            decoder.apply(&baseline).unwrap();
            let decoded = decoder.apply(&frame).unwrap();
            let view = &*decoded.tables[table.index()];
            let rows = match (status, view) {
                (0..=2, TableView::Complete(rows)) => Some(rows),
                (3, TableView::Truncated { rows, missing: 1 }) => Some(rows),
                (4, TableView::Unavailable { reason }) => {
                    assert_eq!(reason, "synthetic history unavailable");
                    None
                }
                _ => panic!("unexpected air history at status {status}: {view:?}"),
            };
            if let Some(rows) = rows {
                let row = &rows
                    .rows::<rows::tables::AirNavalPatrolHistories>()
                    .unwrap()[0];
                assert_eq!(row, if status == 2 { &previous } else { &changed });
                assert_eq!(row.scenario_channels, rows::Recorded::NotRecorded);
            }
            assert_eq!(
                decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
                hour
            );
            if selected.is_none() {
                assert_eq!(
                    decoded.complete::<rows::tables::Countries>().unwrap()[0].tag,
                    b"AAA"
                );
            }
        }
    }
    let older = legacy_replay(version);
    let SeekResult::Frame(frame) = older.seek(1, 1000).unwrap() else {
        panic!("missing historical hour")
    };
    let [row] = frame
        .complete::<rows::tables::AirNavalPatrolHistories>()
        .unwrap()
    else {
        panic!("missing historical patrol prefix")
    };
    assert_eq!(row.enemy_ships_sunk_raw, 1);
    assert_eq!(row.scenario_channels, rows::Recorded::NotRecorded);
}

#[test]
fn compatibility_before_political_runtime_inputs_keeps_carrier_prefixes() {
    let before = crate::rows::compatibility::POLITICAL_FORMAT - 1;
    let replay = legacy_replay(before);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing hour");
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
        frame.complete::<rows::tables::AirBases>().unwrap()[0].carrier,
        rows::Recorded::NotRecorded
    );
    assert_eq!(
        frame
            .complete::<rows::tables::AirBaseRegistrations>()
            .unwrap()[0]
            .carrier,
        rows::Recorded::NotRecorded
    );
    assert_eq!(replay.format.fields(TableId::AirBases), 5);
    assert_eq!(replay.format.fields(TableId::AirBaseRegistrations), 2);
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
    // OwnedWargoals, introduced with these tables, retired since: its
    // framing is consumed and every recording refuses it by name.
    assert_eq!(
        *frame.tables[TableId::OwnedWargoals.index()],
        TableView::Unavailable {
            reason: format!(
                "OwnedWargoals retired at format {}; this recording is format {}",
                RETIRED_INPUTS_FORMAT, RECORDER_INPUTS_FORMAT
            )
        }
    );
    let changed_tables = [TableId::UnitLeaders];
    for table in changed_tables {
        assert!(
            tables.contains(&table),
            "{table:?} missing from selected fixture"
        );
        let reason = format!(
            "{table:?} layout or meaning changed at format {}; this recording is format {}",
            rows::compatibility::EXPERIENCE_FORMAT,
            RECORDER_INPUTS_FORMAT
        );
        assert_eq!(replay.format.table_refusal(table).unwrap(), reason);
        assert_eq!(
            *frame.tables[table.index()],
            TableView::Unavailable {
                reason: reason.clone()
            },
            "{table:?}"
        );
        let TableUpdate::Incompatible {
            reason: selected_reason,
            source_format,
            recorded_bytes,
        } = &selected[0].tables[table.index()]
        else {
            panic!("{table:?} selected read did not retain its named refusal")
        };
        assert_eq!(selected_reason, &reason, "{table:?}");
        assert_eq!(*source_format, replay.format, "{table:?}");
        assert!(*recorded_bytes > 0, "{table:?}");
    }
    for table in tables {
        if changed_tables.contains(&table) {
            continue;
        }
        let mut expected = legacy_rows(table, 1000, replay.format);
        // The fixture writes the historical prefix, not today's appended operands.
        match &mut expected {
            TableRows::DivisionStatus(rows) => {
                for row in rows {
                    row.experience = rows::Recorded::NotRecorded;
                    row.pending_template = rows::Recorded::NotRecorded;
                    row.officer_experience = rows::Recorded::NotRecorded;
                    row.career_counter = rows::Recorded::NotRecorded;
                    row.panel_source = rows::Recorded::NotRecorded;
                    row.commander_width_factor = rows::Recorded::NotRecorded;
                    row.combat_movement = rows::Recorded::NotRecorded;
                    row.organisation_exact = rows::Recorded::NotRecorded;
                    row.maximum_organisation_exact = rows::Recorded::NotRecorded;
                    row.country_ordinal = rows::Recorded::NotRecorded;
                    row.invalid_orders_reason = rows::Recorded::NotRecorded;
                    row.supply = rows::Recorded::NotRecorded;
                    row.attrition_inputs = rows::Recorded::NotRecorded;
                    row.dig_in_factor = rows::Recorded::NotRecorded;
                    row.name = rows::Recorded::NotRecorded;
                    row.attack = rows::Recorded::NotRecorded;
                    row.camouflage = rows::Recorded::NotRecorded;
                    row.attack_stats = rows::Recorded::NotRecorded;
                    row.weather_factor = rows::Recorded::NotRecorded;
                    row.name_template = rows::Recorded::NotRecorded;
                    row.movement_stats = rows::Recorded::NotRecorded;
                    row.out_of_supply_days = rows::Recorded::NotRecorded;
                    row.commander_modifiers = rows::Recorded::NotRecorded;
                    row.supply_ratio_exact = rows::Recorded::NotRecorded;
                    row.supply_grace_exact = rows::Recorded::NotRecorded;
                }
            }
            TableRows::CountryCounterInputs(rows) => {
                for row in rows {
                    row.combat_width_factor = rows::Recorded::NotRecorded;
                    row.recruitment_modifiers = rows::Recorded::NotRecorded;
                    row.sent_expeditionaries = rows::Recorded::NotRecorded;
                    row.mio_policies = rows::Recorded::NotRecorded;
                    row.neighbors = rows::Recorded::NotRecorded;
                    row.co_belligerents = rows::Recorded::NotRecorded;
                    row.zero_war = rows::Recorded::NotRecorded;
                    row.movement = rows::Recorded::NotRecorded;
                    row.zero_subject = rows::Recorded::NotRecorded;
                    row.pride_name = rows::Recorded::NotRecorded;
                    row.forward_visibility = rows::Recorded::NotRecorded;
                    row.attack = rows::Recorded::NotRecorded;
                    row.directed_wars = rows::Recorded::NotRecorded;
                    row.attack_modifiers = rows::Recorded::NotRecorded;
                    row.movement_modifiers = rows::Recorded::NotRecorded;
                    row.air_modifiers = rows::Recorded::NotRecorded;
                    row.mines_effect_reduction = rows::Recorded::NotRecorded;
                    row.naval_detection = rows::Recorded::NotRecorded;
                    row.air_superiority_efficiency = rows::Recorded::NotRecorded;
                    row.air_intercept_efficiency = rows::Recorded::NotRecorded;
                    row.air_cas_efficiency = rows::Recorded::NotRecorded;
                    row.air_nav_efficiency = rows::Recorded::NotRecorded;
                    row.air_attack_factor = rows::Recorded::NotRecorded;
                    row.air_defence_factor = rows::Recorded::NotRecorded;
                    row.air_strategic_bomber_defence_factor = rows::Recorded::NotRecorded;
                    row.air_combat_modifiers_known = rows::Recorded::NotRecorded;
                }
            }
            TableRows::VariantDefinitions(rows) => {
                for row in rows {
                    row.design = rows::Recorded::NotRecorded;
                }
            }
            TableRows::AdvisorRoles(rows) => {
                for row in rows {
                    row.tooltip_keys = rows::Recorded::NotRecorded;
                }
            }
            TableRows::LandCombatDetails(rows) => {
                for row in rows {
                    row.side_inputs = rows::Recorded::NotRecorded;
                    // The older side layout's CAS operands decode and stay.
                    for side in &mut row.sides {
                        side.anti_air_attack = rows::RetiredKept::older_value(0);
                        side.prevented_damage = rows::RetiredKept::older_value([0; 2]);
                    }
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
    // Script history changed its whole layout; admission names the table
    // rather than claiming that its other nested fields still decode.
    assert_eq!(replay.format.nested_field_gaps().count(), 0);
    let event_refusal = format!(
        "EventFeedHistory not recorded at format 98 (introduced at format {})",
        crate::rows::compatibility::WIDGET_CAPTURE_FORMAT
    );
    let previous =
        FormatVersion::read(crate::rows::compatibility::EVENT_FEED_COLUMNS_FORMAT - 1).unwrap();
    assert_eq!(
        previous.table_refusal(TableId::EventFeedHistory),
        Some(format!(
            "EventFeedHistory layout or meaning changed at format {}; this recording is format {}",
            crate::rows::compatibility::EVENT_FEED_COLUMNS_FORMAT,
            previous.version()
        ))
    );
    assert_eq!(
        replay.format.table_refusal(TableId::EventFeedHistory),
        Some(event_refusal.clone())
    );
    assert_eq!(
        *frame.tables[TableId::EventFeedHistory.index()],
        TableView::Unavailable {
            reason: event_refusal.clone()
        }
    );
    let history = replay
        .read_block_selected(&replay.index[0], &[TableId::EventFeedHistory])
        .unwrap();
    assert_eq!(
        history[0].tables[TableId::EventFeedHistory.index()],
        TableUpdate::Unavailable {
            reason: event_refusal
        }
    );
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
    // OwnedWargoals, introduced with these tables, retired since: its
    // framing is consumed and every recording refuses it by name.
    assert_eq!(
        *frame.tables[TableId::OwnedWargoals.index()],
        TableView::Unavailable {
            reason: format!(
                "OwnedWargoals retired at format {}; this recording is format {}",
                RETIRED_INPUTS_FORMAT, RECORDER_INPUTS_FORMAT
            )
        }
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
        // The retired progress and direction still follow their framing.
        assert_eq!(
            progress.attacker_progress,
            rows::RetiredI64::holding(33_333)
        );
        assert_eq!(
            progress.defender_progress,
            rows::RetiredI64::holding(66_667)
        );
        assert_eq!(
            progress.direction_attacker,
            rows::RetiredU64::holding(9 << 32 | 4713)
        );
        assert_eq!(progress.direction_province, rows::RetiredU32::holding(7));
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
    assert_eq!(next.attacker_progress, rows::RetiredI64::holding(50_000));
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
        let payload =
            replace_legacy_snapshot(format, legacy_frame(format, hour, false), table, &update);
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
fn compatibility_changed_leader_contract_refuses_historical_prefixes() {
    for version in [91, 97, 99] {
        let format = FormatVersion::read(version).unwrap();
        let table = TableId::UnitLeaders;
        assert_eq!(
            format.table_refusal(table).unwrap(),
            format!(
                "{table:?} layout or meaning changed at format 104; this recording is format {version}"
            )
        );
    }
}

fn check_format99_seam<S: rows::TableSchema>(
    fields: usize,
    before: &S::Row,
    after: &S::Row,
) -> (S::Row, S::Row) {
    let format = FormatVersion::read(rows::compatibility::EFFECT_INPUTS_FORMAT).unwrap();
    let table = S::ID;
    assert_eq!(format.fields(table), fields, "{table:?}");
    assert_eq!(before.not_recorded_fields(), 0);
    assert_eq!(after.not_recorded_fields(), 0);
    let expected_gaps = table
        .last_change()
        .appended
        .iter()
        .filter(|field| field.introduced > format.version())
        .fold(0, |mask, field| mask | (1 << field.index));
    assert_ne!(expected_gaps, 0);

    let mut update = vec![0]; // historical snapshot
    update.extend_from_slice(&1_u32.to_le_bytes());
    update.extend(TableRows::from_rows::<S>(vec![before.clone()]).fixture_prefix(fields));
    let payload = replace_legacy_snapshot(format, legacy_frame(format, 1000, true), table, &update);
    let mut input = Decoder::new(&payload);
    let first = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    assert_eq!(
        first.table_encoded_sizes_version(format).unwrap()[table.index()],
        update.len()
    );
    let mut decoder = FrameDecoder::new();
    let snapshot = decoder.advance(&first).unwrap().complete::<S>().unwrap()[0].clone();
    assert_eq!(snapshot.key(), before.key());
    assert_eq!(snapshot.not_recorded_fields(), expected_gaps);

    // Change the final OLD field only. The mask has the historical width,
    // followed immediately by its value and then the next table's framing.
    let delta = after.delta(Some(before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << (fields - 1));
    update.clear();
    update.push(1);
    update.extend_from_slice(&1000_i32.to_le_bytes());
    update.extend_from_slice(&1_u32.to_le_bytes());
    table.encode_key(after.key(), 0, &mut update).unwrap();
    update.extend_from_slice(&delta.mask.to_le_bytes()[..fields.div_ceil(8)]);
    update.extend_from_slice(&delta.values);
    update.extend_from_slice(&0_u32.to_le_bytes()); // no deletions
    let payload =
        replace_legacy_snapshot(format, legacy_frame(format, 1001, false), table, &update);
    let mut input = Decoder::new(&payload);
    let frame = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    assert_eq!(
        frame.table_encoded_sizes_version(format).unwrap()[table.index()],
        update.len()
    );
    let TableUpdate::Delta { changed, .. } = &frame.tables[table.index()] else {
        panic!("fixture did not decode a delta");
    };
    assert_eq!(changed[0].recorded_fields, fields);
    assert_eq!(changed[0].mask, delta.mask);
    let changed = decoder.advance(&frame).unwrap().complete::<S>().unwrap()[0].clone();
    assert_eq!(changed.key(), after.key());
    assert_eq!(changed.not_recorded_fields(), expected_gaps);
    assert!(changed.encode_fields(&mut Vec::new()).is_err());
    (snapshot, changed)
}

#[test]
fn compatibility_format99_political_snapshots_and_deltas_keep_the_old_field_seams() {
    macro_rules! seam {
        ($table:ident, $fields:literal, $before:expr, $field:ident => $value:expr) => {{
            let before = $before;
            let mut after = before.clone();
            after.$field = $value;
            let (snapshot, changed) =
                check_format99_seam::<rows::tables::$table>($fields, &before, &after);
            assert_eq!(snapshot.$field, before.$field);
            assert_eq!(changed.$field, after.$field);
        }};
    }
    seam!(AirWings, 16, rows::AirWingRow {
        id: 1,
        owner: 0,
        planes: 0,
        name: Some(vec![0xff, 0, b'A']).into(),
        ..Default::default()
    }, name => None.into());
    let format = FormatVersion::read(99).unwrap();
    let payload = legacy_frame(format, 1000, true);
    let frame = Frame::decode_version(&mut Decoder::new(&payload), None, format).unwrap();
    assert_eq!(
        frame.tables[TableId::AirMissionInputs.index()],
        TableUpdate::Unavailable {
            reason: format.table_refusal(TableId::AirMissionInputs).unwrap(),
        }
    );
    seam!(AirNavalPatrolHistories, 2, rows::AirNavalPatrolHistoryRow {
        country: 4,
        region: 9,
        index: 7,
        enemy_ships_sunk_raw: 1,
        our_fighters_lost_raw: -1,
        scenario_channels: Some([0; 7]).into(),
    }, our_fighters_lost_raw => i64::MIN);
    seam!(ArmyOrderPlans, 20, rows::ArmyOrderPlanRow {
        owner_group: 1,
        instance: 1,
        name_postfix: b"old postfix".to_vec(),
        ..Default::default()
    }, name_postfix => vec![0xff, 0, b'Z']);
    seam!(DivisionMovement, 15, rows::DivisionMovementRow {
        division: 1,
        path_total: 1,
        path: vec![7],
        strategic_redeployment: 1,
        strategic_action: rows::StrategicAction::Withheld(rows::StrategicActionFailure::CountryModifier).into(),
        ..Default::default()
    }, strategic_action => rows::StrategicAction::Withheld(rows::StrategicActionFailure::Owner).into());
    seam!(ShipModels, 6, rows::ShipModelRow {
        ship: 1,
        counter: Some(rows::ShipCounterInputs {
            roster_ordinal: u32::MAX,
            definition_valid: 1,
            anti_air_base: i64::MIN,
            maximum_organisation: i64::MAX,
            ..Default::default()
        }),
        ..Default::default()
    }, counter => None);
    seam!(TaskForceMapMotion, 4, rows::TaskForceMapMotionRow {
        task_force: 1,
        combat_role: 0,
        path: vec![7, 8],
        alternate_path: vec![9, 8],
        ..Default::default()
    }, alternate_path => vec![0, 7, 7]);
    seam!(TaskForceCounterInputs, 10, rows::TaskForceCounterInputRow {
        task_force: 1,
        attached_headquarters: 0,
        ..Default::default()
    }, attached_headquarters => i32::MIN);
    seam!(ReservedCountryNaming, 13, rows::ReservedCountryNamingRow {
        tag: b"synthetic reserved".to_vec(),
        owned_state_count: Some(0).into(),
        ..Default::default()
    }, owned_state_count => Some(i32::MIN).into());
    seam!(RailwayGunCounterInputs, 26, rows::RailwayGunCounterInputRow {
        gun: 1,
        railway_building_level: Some(0),
        supply: Some(rows::UnitSupplyInputs::default()).into(),
        ..Default::default()
    }, supply => None.into());
    seam!(States, 6, rows::StateRow {
        id: 1,
        controller: 0,
        manpower_available: i32::MIN,
        ..Default::default()
    }, manpower_available => 0);
    seam!(RailCooldowns, 1, rows::RailCooldownRow {
        province: 1,
        cooldown_ticks: -1,
        ..Default::default()
    }, cooldown_ticks => 0);
}

#[test]
fn compatibility_format99_movement_prefix_delta_cannot_admit_a_current_profile() {
    let before = rows::DivisionMovementRow {
        division: 1,
        path_total: 1,
        path: vec![7],
        strategic_redeployment: 1,
        strategic_action: rows::StrategicAction::Withheld(
            rows::StrategicActionFailure::CountryModifier,
        )
        .into(),
        ..Default::default()
    };
    let after = rows::DivisionMovementRow {
        strategic_action: rows::StrategicAction::Withheld(rows::StrategicActionFailure::Owner)
            .into(),
        ..before.clone()
    };
    // The helper decodes historical frames at1000/1001, with the native
    // published mask width and the final published field's bit14.
    let (snapshot, changed) =
        check_format99_seam::<rows::tables::DivisionMovement>(15, &before, &after);
    assert_eq!(snapshot.fresh, rows::Recorded::NotRecorded);
    assert_eq!(changed.fresh, rows::Recorded::NotRecorded);
    assert_eq!(snapshot.strategic_action, before.strategic_action);
    assert_eq!(changed.strategic_action, after.strategic_action);
    assert_eq!(snapshot.path, before.path);
    assert_eq!(changed.path, before.path);
}

#[test]
fn compatibility_current_movement_profile_delta_uses_only_its_actual_suffix() {
    let format = FormatVersion::CURRENT;
    let table = TableId::DivisionMovement;
    let fields = format.fields(table);
    assert_eq!(fields, 25);
    let suffix = table
        .last_change()
        .appended
        .iter()
        .find(|field| field.name == "base_speed")
        .expect("movement base speed suffix history");
    assert_eq!(suffix.index, 22);
    let before = rows::DivisionMovementRow {
        division: 1,
        source: 7,
        target: 7,
        numerator: i64::MIN,
        combat_count: 1,
        combat_references: Some(rows::UnitCombatReferences {
            speed_witness: Default::default(),
            sample_hour: rows::PacketHour::Host,
            entries: vec![rows::UnitCombatReference {
                combat: u64::MAX,
                kind: 0,
            }],
        })
        .into(),
        base_speed: rows::Recorded::Value(None),
        ..Default::default()
    };
    let after = rows::DivisionMovementRow {
        base_speed: Some(400_000).into(),
        ..before.clone()
    };
    let mut update = vec![0];
    update.extend_from_slice(&1_u32.to_le_bytes());
    update.extend(TableRows::DivisionMovement(vec![before.clone()]).fixture_prefix(fields));
    let payload = replace_legacy_snapshot(format, legacy_frame(format, 1000, true), table, &update);
    let mut input = Decoder::new(&payload);
    let first = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    assert_eq!(
        first.table_encoded_sizes_version(format).unwrap()[table.index()],
        update.len()
    );
    let mut decoder = FrameDecoder::new();
    let snapshot = decoder
        .advance(&first)
        .unwrap()
        .complete::<rows::tables::DivisionMovement>()
        .unwrap()[0]
        .clone();
    assert_eq!(snapshot, before);
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << suffix.index);
    update.clear();
    update.push(1);
    update.extend_from_slice(&1000_i32.to_le_bytes());
    update.extend_from_slice(&1_u32.to_le_bytes());
    rows::encode_delta(table, &delta, 0, &mut update).unwrap();
    update.extend_from_slice(&0_u32.to_le_bytes());
    let payload =
        replace_legacy_snapshot(format, legacy_frame(format, 1001, false), table, &update);
    let mut input = Decoder::new(&payload);
    let frame = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    assert_eq!(
        frame.table_encoded_sizes_version(format).unwrap()[table.index()],
        update.len()
    );
    let TableUpdate::Delta { changed, .. } = &frame.tables[table.index()] else {
        panic!("movement current suffix delta unavailable");
    };
    assert_eq!(changed[0].mask, 1 << suffix.index);
    assert_eq!(changed[0].recorded_fields, fields);
    let current = decoder.advance(&frame).unwrap();
    assert_eq!(current.hour, 1001);
    assert_eq!(
        current
            .complete::<rows::tables::DivisionMovement>()
            .unwrap()[0],
        after
    );
}

#[test]
fn compatibility_format99_eta_rows_are_discarded_through_their_published_layout() {
    let format = FormatVersion::read(rows::compatibility::EFFECT_INPUTS_FORMAT).unwrap();
    let table = TableId::DivisionEtaInputs;
    let fields = format.fields(table);
    assert_eq!(fields, 18);
    assert_eq!(fields.div_ceil(8), 3);
    assert!(table.last_change().appended.is_empty());
    let before = rows::DivisionEtaInputsRow {
        division: 1,
        sample_hour: 1000,
        current_speed: i64::MIN,
        speed_base: i64::MAX,
        leg_river_combined: vec![-1, 0, 1],
        ..Default::default()
    };
    let after = rows::DivisionEtaInputsRow {
        sample_hour: 1001,
        ..before.clone()
    };
    let mut update = vec![0];
    update.extend_from_slice(&1_u32.to_le_bytes());
    update.extend(TableRows::DivisionEtaInputs(vec![before.clone()]).fixture_prefix(fields));
    let payload = replace_legacy_snapshot(format, legacy_frame(format, 1000, true), table, &update);
    let mut input = Decoder::new(&payload);
    let first = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    assert_eq!(
        first.table_encoded_sizes_version(format).unwrap()[table.index()],
        update.len()
    );
    // The retired table's published framing is consumed and refused by name;
    // the frame's other tables still decode.
    let retired = format!(
        "DivisionEtaInputs retired at format 102; this recording is format {}",
        format.version()
    );
    let mut decoder = FrameDecoder::new();
    let decoded = decoder.advance(&first).unwrap();
    assert_eq!(
        *decoded.tables[table.index()],
        TableView::Unavailable {
            reason: retired.clone()
        }
    );
    assert_eq!(
        decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
        1000
    );
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 17);
    update.clear();
    update.push(1);
    update.extend_from_slice(&1000_i32.to_le_bytes());
    update.extend_from_slice(&1_u32.to_le_bytes());
    table.encode_key(after.key(), 0, &mut update).unwrap();
    update.extend_from_slice(&delta.mask.to_le_bytes()[..fields.div_ceil(8)]);
    update.extend_from_slice(&delta.values);
    update.extend_from_slice(&0_u32.to_le_bytes());
    let payload =
        replace_legacy_snapshot(format, legacy_frame(format, 1001, false), table, &update);
    let mut input = Decoder::new(&payload);
    let frame = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    assert_eq!(
        frame.table_encoded_sizes_version(format).unwrap()[table.index()],
        update.len()
    );
    let decoded = decoder.advance(&frame).unwrap();
    assert_eq!(
        *decoded.tables[table.index()],
        TableView::Unavailable { reason: retired }
    );
    assert_eq!(
        decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
        1001
    );
}

#[test]
fn compatibility_oracle_witnesses_keep_integrated_recordings_readable() {
    let replay = legacy_replay(crate::rows::compatibility::POLITICAL_FORMAT - 1);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing historical hour");
    };
    let construction = frame
        .complete::<rows::tables::ConstructionLineOutput>()
        .unwrap();
    assert_eq!(construction.len(), 1);
    assert_eq!(construction[0].completion, rows::Recorded::Value(None));
    assert_eq!(construction[0].count_reset, rows::Recorded::NotRecorded);
    let naval = frame.complete::<rows::tables::NavalBases>().unwrap();
    assert_eq!(naval.len(), 1);
    assert_eq!(naval[0].country_tags, rows::Recorded::NotRecorded);
}

#[test]
fn compatibility_every_appended_field_is_explicitly_not_recorded() {
    // Raw MIO suffixes keep their framing but do not claim an absent-value
    // representation, unlike the Recorded fields exercised below.
    for (table, prefix) in [
        (TableId::VariantStats, 10),
        (TableId::IndustrialOrganisationHistory, 1),
    ] {
        let history = table.last_change();
        assert_eq!(history.kind, rows::compatibility::ChangeKind::Layout);
        assert_eq!(history.appended[0].index, prefix);
        assert!(FormatVersion::CURRENT.table_refusal(table).is_none());
    }
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
fn incompatible_technology_updates_keep_source_byte_accounting() {
    let format = FormatVersion::read(rows::compatibility::WIDGET_CAPTURE_FORMAT).unwrap();
    let snapshot = vec![0, 0, 0, 0, 0];
    let mut delta = vec![1];
    delta.extend_from_slice(&999_i32.to_le_bytes());
    delta.extend_from_slice(&[0; 8]);
    let mut unchanged = vec![2];
    unchanged.extend_from_slice(&999_i32.to_le_bytes());
    let mut truncated = vec![3];
    truncated.extend_from_slice(&7_u32.to_le_bytes());
    truncated.extend_from_slice(&0_u32.to_le_bytes());
    let mut unavailable = vec![4];
    put_string(&mut unavailable, "original source refusal").unwrap();
    for table in [
        TableId::ResearchTechnologies,
        TableId::TechnologyDefinitions,
    ] {
        for update in [&snapshot, &delta, &unchanged, &truncated, &unavailable] {
            let payload =
                replace_legacy_snapshot(format, legacy_frame(format, 1000, false), table, update);
            let mut input = Decoder::new(&payload);
            let frame = Frame::decode_version(&mut input, None, format).unwrap();
            input.finish().unwrap();
            assert_eq!(
                frame.table_encoded_sizes_version(format).unwrap()[table.index()],
                update.len(),
            );
            let decoded = FrameDecoder::new().apply(&frame).unwrap();
            assert!(matches!(
                &*decoded.tables[table.index()],
                TableView::Unavailable { reason } if reason.contains(&format!("{table:?} layout or meaning changed"))
            ));
            let mut emitted = Frame {
                branch: 1,
                hour: 1000,
                keyframe: true,
                metrics: Default::default(),
                tables: std::array::from_fn(|_| TableUpdate::Unavailable {
                    reason: "fixture omitted".into(),
                }),
            };
            emitted.tables[table.index()] = frame.tables[table.index()].clone();
            let mut current = Vec::new();
            let sizes = emitted.encode_with_sizes(&mut current).unwrap();
            assert_eq!(emitted.table_encoded_sizes().unwrap(), sizes);
            let emitted = Frame::decode(&mut Decoder::new(&current)).unwrap();
            assert_eq!(
                *FrameDecoder::new().apply(&emitted).unwrap().tables[table.index()],
                *decoded.tables[table.index()],
            );
        }
    }
}

#[test]
fn compatibility_table_lifetimes_keep_the_old_frame_stream_readable() {
    // Synthetic introduction/retirement independent of the actual table lifetimes.
    let mut history = HISTORY;
    let added = TableId::NavalModifierBreakdowns;
    history[added.index()].introduced = 96;
    history[added.index()].format = 96;
    history[added.index()].retired = None;
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

#[test]
fn compatibility_previous_format_opens_and_refuses_only_nuclear_strikes() {
    let replay = legacy_replay(rows::compatibility::NUCLEAR_STRIKES_FORMAT - 1);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing previous-format hour")
    };
    assert_eq!(
        frame.complete::<rows::tables::Countries>().unwrap()[0].tag,
        b"AAA"
    );
    let TableView::Unavailable { reason } = &*frame.tables[TableId::NuclearStrikes.index()] else {
        panic!("an unrecorded strike ledger is not a complete empty interval")
    };
    assert!(reason.contains("not recorded"));
}

#[test]
fn compatibility_previous_format_keeps_whole_fuel_packs_and_refuses_only_their_parts() {
    let replay = legacy_replay(rows::compatibility::FUEL_DEMAND_PARTS_FORMAT - 1);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing previous-format hour")
    };
    let fuel = frame.complete::<rows::tables::CountryFuel>().unwrap();
    // The previous writer stored the pack's hour, which reads as a stamp.
    let rows::Recorded::Value(pack) = &fuel[0].demand else {
        panic!("previous-format pack missing")
    };
    assert_eq!(pack.hour, rows::FuelDemandHour::Historical(1000));
    let parts = &*frame.tables[TableId::FuelDemandParts.index()];
    let TableView::Unavailable { reason } = parts else {
        panic!("unrecorded parts are not a complete empty table")
    };
    assert!(reason.contains("not recorded"), "{reason}");
    // Its packs keep their own lists.
    assert_eq!(
        rows::with_demand_parts(fuel, parts, replay.format).unwrap(),
        fuel
    );
    // A selected read of both tables, as the dashboard makes, refuses the
    // parts the same way.
    let selected = [TableId::CountryFuel, TableId::FuelDemandParts];
    let frames = replay
        .read_block_selected(&replay.index[0], &selected)
        .unwrap();
    let mut decoder = FrameDecoder::for_tables(&selected);
    let decoded = decoder.advance(&frames[0]).unwrap();
    assert!(matches!(
        &*decoded.tables[TableId::FuelDemandParts.index()],
        TableView::Unavailable { reason } if reason.contains("not recorded")
    ));
    assert_eq!(
        decoded.complete::<rows::tables::CountryFuel>().unwrap(),
        fuel
    );
}

#[test]
fn compatibility_previous_format_keeps_modifier_sides_and_refuses_only_member_rows() {
    use crate::rows::{CombatModifierValue, LandCombatModifierRow, Retired};
    use crate::value::Value;
    use replay_data::test_support::OldDivision;
    fn opaque(value: &impl Value, output: &mut Vec<u8>) {
        let mut payload = Vec::new();
        Value::encode(value, &mut payload).unwrap();
        output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        output.extend(payload);
    }
    let entry = |id, value: i64| CombatModifierValue {
        id,
        value_a: value,
        value_b: -value,
    };
    let format = FormatVersion::read(rows::compatibility::LAND_MODIFIER_ROWS_FORMAT - 1).unwrap();
    let table = TableId::LandCombatModifiers;
    let fields = format.fields(table);
    assert_eq!(fields, 2);
    // Border war 62 (both sides' border values, no members) and ordinary
    // combat 63, whose attacker lists two members and defender one.
    // (combat, side, border values, member divisions)
    type OldSide = (u64, u8, Option<[i64; 3]>, Vec<OldDivision>);
    let sides: [OldSide; 4] = [
        (62, 0, Some([1, -2, 300_000]), Vec::new()),
        (62, 1, Some([-4, 5, i64::MAX]), Vec::new()),
        (
            63,
            0,
            None,
            vec![
                OldDivision {
                    division: (0x35 << 32) | 0x33,
                    ordinal: 0,
                    entries: vec![entry(4, 10), entry(4, 10), entry(9, -7_000)],
                },
                OldDivision {
                    division: 77,
                    ordinal: 1,
                    entries: Vec::new(),
                },
            ],
        ),
        (
            63,
            1,
            None,
            vec![OldDivision {
                division: 78,
                ordinal: 0,
                entries: vec![entry(i32::MAX as u32, i64::MIN + 1)],
            }],
        ),
    ];
    let key = |combat: u64, side: u8| u128::from(combat) << 1 | u128::from(side);
    let mut update = vec![0];
    update.extend_from_slice(&(sides.len() as u32).to_le_bytes());
    let mut previous = 0;
    for (combat, side, border, divisions) in &sides {
        table
            .encode_key(key(*combat, *side), previous, &mut update)
            .unwrap();
        previous = key(*combat, *side);
        opaque(border, &mut update);
        opaque(divisions, &mut update);
    }
    let payload = replace_legacy_snapshot(format, legacy_frame(format, 1000, true), table, &update);
    let mut input = Decoder::new(&payload);
    let first = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    let mut decoder = FrameDecoder::new();
    let decoded = decoder.advance(&first).unwrap();
    let expected = |border: [Option<[i64; 3]>; 4]| {
        sides
            .iter()
            .zip(border)
            .map(
                |((combat, side, _, _), border_values)| LandCombatModifierRow {
                    combat: *combat,
                    side: *side,
                    border_values,
                    divisions: Retired,
                },
            )
            .collect::<Vec<_>>()
    };
    // The side rows and their border values decode; the member lists are
    // discarded through their framing; only the member rows are refused.
    assert_eq!(
        decoded
            .complete::<rows::tables::LandCombatModifiers>()
            .unwrap(),
        expected(sides.clone().map(|side| side.2)).as_slice()
    );
    let TableView::Unavailable { reason } =
        &*decoded.tables[TableId::LandCombatDivisionModifiers.index()]
    else {
        panic!("unrecorded member rows are not a complete empty table")
    };
    assert!(reason.contains("LandCombatDivisionModifiers"), "{reason}");
    assert!(reason.contains("not recorded"), "{reason}");
    assert_eq!(
        decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
        1000
    );
    // The same frame in a previous-format file opens and seeks alike.
    let replay = legacy_replay_with(format.version(), &payload);
    let SeekResult::Frame(sought) = replay.seek(1, 1000).unwrap() else {
        panic!("missing previous-format hour")
    };
    assert_eq!(
        sought
            .complete::<rows::tables::LandCombatModifiers>()
            .unwrap(),
        expected(sides.clone().map(|side| side.2)).as_slice()
    );
    let TableView::Unavailable { reason } =
        &*sought.tables[TableId::LandCombatDivisionModifiers.index()]
    else {
        panic!("a sought previous-format hour has no member rows")
    };
    assert!(reason.contains("not recorded"), "{reason}");
    assert_eq!(
        sought.complete::<rows::tables::Clock>().unwrap()[0].hour,
        1000
    );

    // A delta that moves the defender's border values in 62 and replaces
    // the attacker's member list in 63.
    update.clear();
    update.push(1);
    update.extend_from_slice(&1000_i32.to_le_bytes());
    update.extend_from_slice(&2_u32.to_le_bytes());
    table.encode_key(key(62, 1), 0, &mut update).unwrap();
    update.push(1 << 0);
    opaque(&Some([-4_i64, 6, 0]), &mut update);
    table
        .encode_key(key(63, 0), key(62, 1), &mut update)
        .unwrap();
    update.push(1 << 1);
    opaque(
        &vec![OldDivision {
            division: 79,
            ordinal: 0,
            entries: vec![entry(1, 1)],
        }],
        &mut update,
    );
    update.extend_from_slice(&0_u32.to_le_bytes());
    let payload =
        replace_legacy_snapshot(format, legacy_frame(format, 1001, false), table, &update);
    let mut input = Decoder::new(&payload);
    let frame = Frame::decode_version(&mut input, None, format).unwrap();
    input.finish().unwrap();
    let decoded = decoder.advance(&frame).unwrap();
    assert_eq!(
        decoded
            .complete::<rows::tables::LandCombatModifiers>()
            .unwrap(),
        expected([Some([1, -2, 300_000]), Some([-4, 6, 0]), None, None]).as_slice()
    );
    assert!(matches!(
        &*decoded.tables[TableId::LandCombatDivisionModifiers.index()],
        TableView::Unavailable { .. }
    ));
    assert_eq!(
        decoded.complete::<rows::tables::Clock>().unwrap()[0].hour,
        1001
    );
}

#[test]
fn compatibility_stamped_packets_still_prove_freshness_in_their_own_recordings() {
    use crate::rows::{PACKET_HOUR_HOSTS, PacketHour, STAMPED_PACKET_FORMAT, packet_hours};
    let format = FormatVersion::read(STAMPED_PACKET_FORMAT).unwrap();
    let replay = legacy_replay(STAMPED_PACKET_FORMAT);
    let SeekResult::Frame(frame) = replay.seek(1, 1000).unwrap() else {
        panic!("missing stamped hour");
    };
    let mut stamped = 0;
    for table in PACKET_HOUR_HOSTS {
        assert!(format.table_refusal(table).is_none(), "{table:?}");
        let TableView::Complete(rows) = &*frame.tables[table.index()] else {
            panic!("{table:?} unreadable at format {STAMPED_PACKET_FORMAT}");
        };
        for hour in packet_hours(rows) {
            assert_eq!(hour, PacketHour::Stamped(1000), "{table:?}");
            assert!(hour.fresh_at(frame.hour));
            stamped += 1;
        }
    }
    assert!(stamped >= 2);

    // A stamped table carried unchanged into the next hour keeps its read
    // hour, so its packets are stale there; a reacquired snapshot is fresh.
    let table = TableId::CountryWars;
    let first = legacy_frame(format, 1000, true);
    let mut unchanged = vec![2];
    unchanged.extend_from_slice(&1000_i32.to_le_bytes());
    let mut decoder = FrameDecoder::new();
    for (hour, payload, fresh) in [
        (1000, first, true),
        (
            1001,
            replace_legacy_snapshot(format, legacy_frame(format, 1001, false), table, &unchanged),
            false,
        ),
        (1002, legacy_frame(format, 1002, false), true),
    ] {
        let mut input = Decoder::new(&payload);
        let wire = Frame::decode_version(&mut input, None, format).unwrap();
        input.finish().unwrap();
        let decoded = decoder.advance(&wire).unwrap();
        assert_eq!(decoded.hour, hour);
        let TableView::Complete(rows) = &*decoded.tables[table.index()] else {
            panic!("stamped enemy lists unavailable at {hour}");
        };
        let hours = packet_hours(rows);
        assert!(!hours.is_empty());
        assert!(
            hours.iter().all(|stamp| stamp.fresh_at(hour) == fresh),
            "hour {hour}: {hours:?}"
        );
    }
}
