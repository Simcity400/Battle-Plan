use replay_data::{BranchId, Hour, SessionId, rows::TableId};
use replay_frame::frame::{DecodedFrame, Metrics, TableView};
use replay_data::{
    identity::Identity, identity::Roster, identity::SessionStart, identity::Verification,
    rows::AirWingRow, rows::ArmyOrderRow, rows::CampaignRow, rows::ClockRow,
    rows::ConvoyClientRow, rows::CountryOilFactorRow, rows::CountryRow, rows::DivisionRow,
    rows::FactionMemberRow, rows::FactionRow, rows::FleetRow, rows::HumanControllerRow,
    rows::LandCombatRow, rows::NavalCombatParticipantRow, rows::NavalCombatRow,
    rows::PlayerRow, rows::ProvinceRow, rows::RegionalConvoyEfficiencyRow,
    rows::ResourceExchangeRow, rows::ResourceOriginRow, rows::ResourceRightRow, rows::ShipRow,
    rows::StateRow, rows::StateTemporaryResourceRow, rows::TableRows, rows::TaskForceRow,
    rows::TradeRouteRow, rows::WarRelationRow,
};

pub fn session(id: SessionId) -> SessionStart {
    SessionStart {
        id,
        process_id: id as u32,
        process_start_time: id,
        qpc_frequency: 10_000_000,
        sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
        capture_mode: replay_data::identity::CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

pub fn source(branch: BranchId, hour: Hour) -> DecodedFrame {
    replay_frame::frame::retire_absent(DecodedFrame {
        branch,
        hour,
        metrics: Metrics::default(),
        tables: {
            let map_tables = [
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour,
                    ..ClockRow::default()
                }])),
                TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                    id: b"campaign".to_vec(),
                }])),
                TableView::Complete(TableRows::Countries(vec![CountryRow {
                    id: 1,
                    canonical_slot: 1,
                    tag: b"GER".to_vec(),
                    political_power_raw: Some(i64::MAX - i64::from(hour)),
                    command_power_raw: 100_001,
                    stability_raw: 70_000,
                    war_support_raw: 30_000,
                    convoys_raw: None,
                    peace_conference_participant: 0,
                    manpower_residual: 0,
                    border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
                }])),
                TableView::Complete(TableRows::CountryLinks(Vec::new())),
                TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                    id: 1,
                    state: 1,
                    owner: 1,
                    controller: 1,
                    victory_points: hour - 103,
                    ..Default::default()
                }])),
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 1,
                    owner: 1,
                    controller: 1,
                    continent: replay_data::rows::Recorded::Value(Some(0)),
                    ..Default::default()
                }])),
                TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                    id: (1_u64 << 32) | 4713,
                    roster_owner: 1,
                    logical_owner: 1,
                    expeditionary_owner: 0,
                    province: 1,
                    template: (2_u64 << 32) | 52,
                    hit_points: 26_420_000,
                    organisation: 6_000_000,
                    orders_group: (57_u64 << 32) | 53,
                    reveal_skip: 0,
                    theatre: Some(0).into(),
                }])),
                TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                    first_country: 1,
                    second_country: 2,
                    start_hour: 99,
                    end_hour: 0,
                    cancel: 0,
                    first_casualties: i64::from(hour),
                    second_casualties: 0,
                    first_unknown_casualties: 0,
                    second_unknown_casualties: 0,
                    first_was_instigator: 1,
                }])),
                TableView::Complete(TableRows::Factions(vec![FactionRow {
                    id: 1_u64 << 32 | 42,
                    name: Vec::new(),
                    ..FactionRow::default()
                }])),
                TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
                    country: 1,
                    faction: 1_u64 << 32 | 42,
                    ordinal: 0,
                }])),
                TableView::Complete(TableRows::Players(vec![PlayerRow {
                    ordinal: 0.into(),
                    id: i32::MIN,
                    country: 1,
                    flags: 0x81,
                    user: b"synthetic user".to_vec(),
                    name: b"synthetic name".to_vec(),
                }])),
                TableView::Complete(TableRows::HumanControllers(vec![HumanControllerRow {
                    country: 1,
                    count: i8::MIN,
                }])),
                TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                    id: (9_u64 << 32) | 62,
                    province: 1,
                    duration: hour,
                    day: -1,
                    attackers: vec![(1_u64 << 32) | 4713],
                    defenders: vec![(2_u64 << 32) | 4713],
                    attacker_classes: vec![0],
                    defender_classes: vec![0],
                }])),
                TableView::Complete(TableRows::Fleets(vec![FleetRow {
                    id: (1_u64 << 32) | 61,
                    owner: 1,
                    home_base: 1,
                    name: replay_data::rows::Recorded::Value(Some(Vec::new())),
                    leader: replay_data::rows::Recorded::Value(Some(0)),
                }])),
                TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                    id: (2_u64 << 32) | 51,
                    fleet: (1_u64 << 32) | 61,
                    roster_owner: 1,
                    logical_owner: 1,
                    ..Default::default()
                }])),
                TableView::Complete(TableRows::Ships(vec![ShipRow {
                    id: (3_u64 << 32) | 51,
                    task_force: (2_u64 << 32) | 51,
                    strength: i64::from(hour),
                    organisation: i64::MAX,
                    ..Default::default()
                }])),
                TableView::Complete(TableRows::NavalCombats(vec![NavalCombatRow {
                    id: u64::MAX,
                    province: 1,
                    attacker_positioning: i64::MIN,
                    defender_anti_air: i64::MAX,
                    ..Default::default()
                }])),
                TableView::Complete(TableRows::NavalCombatParticipants(vec![
                    NavalCombatParticipantRow {
                        combat: u64::MAX,
                        side: 1,
                        battle_slot: u32::MAX,
                        ship: (3_u64 << 32) | 51,
                        owner: 1,
                        state: 0x8000_0020,
                    },
                ])),
                TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                    id: (4_u64 << 32) | 69,
                    owner: 1,
                    air_base: 65,
                    base_override: (5_u64 << 32) | 65,
                    planes: hour,
                    mission_region: Some(u32::MAX),
                    name: replay_data::rows::Recorded::Value(Some(Vec::new())),
                    ..Default::default()
                }])),
                TableView::Complete(TableRows::ArmyOrders(vec![ArmyOrderRow {
                    owner_group: u64::MAX,
                    instance: u32::MAX,
                    country: 1,
                    order_type: 4,
                    active: 0x80,
                    path: vec![1, hour as u32, 1],
                    target_states: vec![u32::MAX, 1],
                    paradrop_target: 7,
                }])),
            ];
            let mut tables = std::array::from_fn(|index| {
                let table = TableId::ALL[index];
                TableView::Unavailable {
                    reason: match table.last_change().retired {
                        Some(retired) => format!(
                            "{table:?} retired at format {retired}; this recording is format {}",
                            replay_data::FORMAT_VERSION
                        ),
                        None => "outside the map playback fixture".into(),
                    },
                }
            });
            for (table, rows) in tables.iter_mut().zip(map_tables) {
                *table = rows;
            }
            tables[TableId::NuclearStrikes.index()] =
                TableView::Complete(TableRows::NuclearStrikes(if hour == 105 {
                    vec![replay_data::rows::NuclearStrikeRow {
                        id: 7,
                        hour,
                        province: 1,
                        nuke_type: 1,
                        ..Default::default()
                    }]
                } else {
                    Vec::new()
                }));
            for rows in resource_map_rows(hour) {
                let table = rows.table();
                tables[table.index()] = TableView::Complete(rows);
            }
            tables.map(Into::into)
        },
    })
}

pub fn resource_map_rows(hour: Hour) -> [TableRows; 8] {
    let efficiency = i64::from(hour) * 1_000;
    [
        TableRows::ResourceExchanges(vec![ResourceExchangeRow {
            id: 9_u64 << 32,
            resource: 7,
            exporter: 2,
            importer: 1,
            efficiency_a: efficiency,
            ..Default::default()
        }]),
        TableRows::ResourceOrigins(vec![ResourceOriginRow {
            ordinal: hour as u32,
            state: 1,
            country: 1,
            efficiency,
            convoys_assigned: 2,
            convoys_required: 3,
            ..Default::default()
        }]),
        TableRows::ResourceRights(vec![ResourceRightRow {
            ordinal: hour as u32,
            state: 1,
            resource: 1,
            holder: 2,
        }]),
        TableRows::CountryOilFactors(vec![CountryOilFactorRow {
            country: 1,
            factor: efficiency,
        }]),
        TableRows::StateTemporaryResources(vec![StateTemporaryResourceRow {
            state: 1,
            resource: 7,
            amount: efficiency,
        }]),
        TableRows::ConvoyClients(vec![ConvoyClientRow {
            owner_ordinal: Some(hour as u32),
            other_ordinal: Some(hour as u32 + 1),
            handle: 9,
            category: 2,
            owner: 1,
            other: 2,
            route_class: 1,
            route_country: 2,
            route_other: 1,
            convoys_assigned: 1,
            convoys_required: 1,
            ..Default::default()
        }]),
        TableRows::TradeRoutes(vec![TradeRouteRow {
            country: 2,
            class: 1,
            other: 1,
            kind: 2,
            from: 1,
            to: 1,
            path: vec![hour as u32],
            source_state: Some(hour as u32),
            destination_state: Some(hour as u32 + 1),
        }]),
        TableRows::RegionalConvoyEfficiency(vec![RegionalConvoyEfficiencyRow {
            country: 1,
            region: hour as u32,
            efficiency,
        }]),
    ]
}
