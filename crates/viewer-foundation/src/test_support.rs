pub fn test_frame(hour: i32) -> replay_frame::frame::DecodedFrame {
    use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};

    DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "unused".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    }
}

pub fn test_put(frame: &mut replay_frame::frame::DecodedFrame, rows: replay_data::rows::TableRows) {
    let index = rows.table().index();
    frame.tables[index] = replay_frame::frame::TableView::Complete(rows).into();
}

pub fn complete_frame(
    hour: i32,
    sample: impl Fn(replay_data::rows::TableId) -> Option<replay_data::rows::TableRows>,
) -> replay_frame::frame::DecodedFrame {
    use replay_data::rows::{TableId, TableRows};
    let mut frame = test_frame(hour);
    for table in TableId::ALL {
        let rows = sample(table).unwrap_or_else(|| match table.key_bits() {
            0 => TableRows::one_default(table),
            _ => TableRows::empty(table),
        });
        test_put(&mut frame, rows);
    }
    frame
}

pub fn put_sample_campaign(frame: &mut replay_frame::frame::DecodedFrame) {
    use replay_data::rows::*;
    let country = |slot: u32, tag: &[u8]| CountryRow {
        id: slot,
        canonical_slot: slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    test_put(
        frame,
        TableRows::Countries(vec![
            country(1, b"AAA"),
            country(2, b"BBB"),
            country(3, b"CCC"),
        ]),
    );
    test_put(
        frame,
        TableRows::CountryLinks(
            (1..=3)
                .map(|slot| CountryLinkRow {
                    id: slot,
                    canonical_slot: slot,
                })
                .collect(),
        ),
    );
    test_put(
        frame,
        TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 1,
            observer_country: 1,
        }]),
    );
    test_put(
        frame,
        TableRows::Subjects(vec![SubjectRow {
            overlord: 1,
            subject: 3,
            end_hour: i32::MAX,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::Autonomy(vec![AutonomyRow {
            subject: 3,
            state: b"autonomy_puppet".to_vec(),
            ..Default::default()
        }]),
    );
    let province = |id: u32, owner: u32| ProvinceRow {
        id,
        state: owner,
        owner,
        controller: owner,
        ..Default::default()
    };
    test_put(
        frame,
        TableRows::Provinces(vec![
            province(1, 1),
            province(2, 2),
            province(3, 0),
            province(4, 3),
            province(5, 0),
        ]),
    );
    let state = |id: u32| StateRow {
        id,
        owner: id,
        controller: id,
        ..Default::default()
    };
    test_put(frame, TableRows::States((1..=3).map(state).collect()));
    let division = |id: u64, owner: u32, province: u32| DivisionRow {
        id,
        roster_owner: owner,
        logical_owner: owner,
        province,
        template: 1,
        ..Default::default()
    };
    test_put(
        frame,
        TableRows::Divisions(vec![division(10, 1, 1), division(11, 2, 2)]),
    );
    test_put(
        frame,
        TableRows::DivisionStatus(vec![
            DivisionStatusRow {
                division: 10,
                ..Default::default()
            },
            DivisionStatusRow {
                division: 11,
                ..Default::default()
            },
        ]),
    );
    test_put(
        frame,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: 10,
            source: 1,
            target: 2,
            combat_count: 1,
            combat_province: 2,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::LandCombats(vec![LandCombatRow {
            id: 100,
            province: 2,
            attackers: vec![10],
            defenders: vec![11],
            attacker_classes: vec![0],
            defender_classes: vec![0],
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::LandCombatSides(vec![LandCombatSideRow {
            combat: 100,
            attacker_primary: vec![1],
            defender_primary: vec![2],
            ..Default::default()
        }]),
    );
    let fighting = |side: u8, division: u64| LandCombatDivisionRow {
        combat: 100,
        side,
        province: 2,
        division,
        own: Some(replay_data::rows::LandCombatMemberValues {
            ..Default::default()
        }),
        ..Default::default()
    };
    test_put(
        frame,
        TableRows::LandCombatDivisions(vec![fighting(0, 10), fighting(1, 11)]),
    );
    test_put(
        frame,
        TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
            combat: 100,
            province: 2,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombats(vec![NavalCombatRow {
            id: 200,
            province: 3,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
            combat: 200,
            province: 3,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombatShips(vec![NavalCombatShipRow {
            combat: 200,
            ship: 40,
            owner: 1,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombatParticipants(vec![NavalCombatParticipantRow {
            combat: 200,
            ship: 40,
            owner: 1,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombatResultDetails(vec![NavalCombatResultDetailsRow {
            result: 300,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalResults(vec![NavalResultRow {
            id: 300,
            province: 3,
            retire_hour: i32::MAX,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 20,
            fleet: 30,
            roster_owner: 1,
            logical_owner: 1,
            province: 3,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::Fleets(vec![FleetRow {
            id: 30,
            owner: 1,
            home_base: 1,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::Ships(vec![ShipRow {
            id: 40,
            task_force: 20,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::TaskForceMissions(vec![TaskForceMissionRow {
            task_force: 20,
            owner: 1,
            current_region: 1,
            regions: vec![1],
        }]),
    );
    test_put(
        frame,
        TableRows::AirBases(vec![AirBaseRow {
            id: 50,
            state: Some(1),
            province: Some(1),
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::AirBaseRegistrations(vec![AirBaseRegistrationRow {
            base: 50,
            slot: 1,
            wings: vec![60],
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::AirWings(vec![AirWingRow {
            id: 60,
            owner: 1,
            air_base: 50,
            mission_mask: 2,
            mission_region: Some(1),
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::AirRegionalMissions(vec![AirRegionalMissionRow {
            wing: 60,
            ordinal: 1,
            active_order: None,
        }]),
    );
    test_put(
        frame,
        TableRows::WeatherProvinces(
            (1..=5)
                .map(|id| WeatherProvinceRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ),
    );
}

pub fn map_scoped(
    mut frame: replay_frame::frame::DecodedFrame,
) -> replay_frame::frame::DecodedFrame {
    use replay_data::scope::{self, CaptureMode};
    for table in replay_data::rows::TableId::ALL {
        if table.last_change().present(replay_data::FORMAT_VERSION)
            && !scope::records(CaptureMode::Map, table)
        {
            frame.tables[table.index()] = replay_frame::frame::TableView::NotRecorded {
                mode: CaptureMode::Map,
            }
            .into();
        }
    }
    frame
}

pub fn assert_reads_recorded<T>(
    frame: &replay_frame::frame::DecodedFrame,
    allowed: &[replay_data::rows::TableId],
    read: impl FnOnce() -> T,
) -> T {
    use replay_frame::frame::{TableView, view_reads};
    let (value, reads) = view_reads::record(read);
    let unrecorded: Vec<_> = replay_data::rows::TableId::ALL
        .into_iter()
        .filter(|table| !allowed.contains(table))
        .filter(|table| {
            let view = &frame.tables[table.index()];
            reads.iter().any(|read| read.is(view))
                && view_reads::unrecorded(|| matches!(**view, TableView::NotRecorded { .. }))
        })
        .collect();
    assert!(
        unrecorded.is_empty(),
        "read tables the recording leaves out: {unrecorded:?}"
    );
    value
}

pub mod allegiance;
pub mod localisation;
pub mod movement_timing;
pub mod unit_types;

pub fn enter_fixed_modifier_tables() {
    use engine_executable::{build::FIXED_MODIFIER_GAP, executable_tables as tables};

    static TABLES: std::sync::OnceLock<tables::Tables> = std::sync::OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        let defaults = content_reader_script::test_game::tables();
        let first = defaults.keywords.as_ref().unwrap().modifier_names.len();
        content_reader_script::test_game::executable()
            .modifiers(
                (first..FIXED_MODIFIER_GAP as usize).map(|index| format!("fixture_modifier_{index}")),
            )
            .pinned()
            .build()
    });
    tables::enter(Some(tables));
}

pub fn equipment_from_documents(documents: &[&[u8]]) -> content_reader_key_names::equipment::Database {
    let paths: Vec<String> = (0..documents.len())
        .map(|index| format!("common/units/equipment/{index:03}.txt"))
        .collect();
    let files: Vec<(&str, &str)> = paths
        .iter()
        .zip(documents)
        .map(|(path, text)| (path.as_str(), std::str::from_utf8(text).unwrap()))
        .collect();
    let (equipment, _) = content_reader_key_names::equipment::fixture::load(&files);
    assert!(equipment.lost.is_none(), "{:?}", equipment.lost);
    equipment
}
