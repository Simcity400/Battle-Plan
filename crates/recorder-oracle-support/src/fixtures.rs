use content_reader_save::{self as save, Decimal, Save};
use replay_data::{
    rows::CampaignRow, rows::ClockRow, rows::CountryLinkRow, rows::CountryRow, rows::DivisionRow,
    rows::ProvinceRow, rows::Recorded, rows::StateRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
use std::{collections::BTreeMap, io};

pub const FIXTURE_CATEGORY_TOKEN: u32 = 0x5a1;

pub fn fixture() -> (
    DecodedFrame,
    Save,
    save::PartialMapMembership,
    content_reader_key_names::equipment::Database,
) {
    let save = save::parse(br#"HOI4txt
player="GER" date="1936.1.1.1" game_unique_id="campaign" game_unique_seed=7 debug_current_ref_id=65580 unit=4008 order_index=7484 front_index=8230 theatre_index=112 theater_group_index=910 military_deployment_line_index=7783 military_deployment_conveyor_index=736 equipment_variant_index=6284 country_leader_index=1 navy_id=8184 land_combat_id=15224 multiplayer_random_count=-1334833904 version="build"
faction_system={countries={0} extracted={}}
states={1={owner="GER" demilitarized=no is_border_conflict=no state_category=town}} provinces={2={controller="GER"}}
countries={GER={politics={political_power=10.00001} command_power=2 stability=0.7 war_support=0.3 convoys={equipment={}}}}
"#).unwrap();
    let hour = save.date.engine_hour().unwrap();
    let frame = DecodedFrame {
        branch: 2,
        hour,
        metrics: Metrics::default(),
        tables: {
            let mut tables = std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            });
            tables[TableId::LocalPlayer.index()] =
                TableView::Complete(TableRows::LocalPlayer(vec![
                    replay_data::rows::LocalPlayerRow {
                        country: 102,
                        observer_country: 0,
                    },
                ]));
            tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![
                    replay_data::rows::ReservedCountryNamingRow {
                        country_array: Recorded::NotRecorded,
                        launch_sources: Recorded::NotRecorded,
                        ..Default::default()
                    },
                ]));
            for (index, table) in [
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour,
                    seed: 7,
                    ..Default::default()
                }])),
                TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                    id: b"campaign".to_vec(),
                }])),
                TableView::Complete(TableRows::Countries(vec![CountryRow {
                    border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
                    id: 101,
                    canonical_slot: 7,
                    tag: b"GER".to_vec(),
                    political_power_raw: Some(1_000_001),
                    command_power_raw: 200_000,
                    stability_raw: 70_000,
                    war_support_raw: 30_000,
                    convoys_raw: Some(0),
                    peace_conference_participant: 0,
                    manpower_residual: 0,
                }])),
                TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow {
                        id: 101,
                        canonical_slot: 7,
                    },
                    CountryLinkRow {
                        id: 102,
                        canonical_slot: 7,
                    },
                ])),
                TableView::Complete(TableRows::Provinces(vec![
                    ProvinceRow {
                        id: 1,
                        state: 1,
                        owner: 102,
                        controller: 0,
                        victory_points: 0,
                        selected_areas: Recorded::NotRecorded,
                        controller_area: Recorded::NotRecorded,
                        owner_area: Recorded::NotRecorded,
                        area_node: Recorded::NotRecorded,
                        ..ProvinceRow::default()
                    },
                    ProvinceRow {
                        id: 2,
                        state: 0,
                        owner: 0,
                        controller: 101,
                        victory_points: 0,
                        selected_areas: Recorded::NotRecorded,
                        controller_area: Recorded::NotRecorded,
                        owner_area: Recorded::NotRecorded,
                        area_node: Recorded::NotRecorded,
                        ..ProvinceRow::default()
                    },
                ])),
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 1,
                    owner: 102,
                    controller: 101,
                    railway_infrastructure: Recorded::Value(Some(0)),
                    category: Recorded::Value(Some(FIXTURE_CATEGORY_TOKEN)),
                    marked_owners: Recorded::Value(Some(Vec::new())),
                    ..StateRow::default()
                }])),
                TableView::Complete(TableRows::Divisions(Vec::new())),
                TableView::Complete(TableRows::WarRelations(Vec::new())),
                TableView::Complete(TableRows::Factions(Vec::new())),
                TableView::Complete(TableRows::FactionMembers(Vec::new())),
            ]
            .into_iter()
            .enumerate()
            {
                tables[index] = table;
            }
            tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![
                    replay_data::rows::LexicalTokenRow {
                        token: FIXTURE_CATEGORY_TOKEN,
                        literal: b"town".to_vec(),
                    },
                ]));
            tables[TableId::HumanControllers.index()] =
                TableView::Complete(TableRows::HumanControllers(vec![
                    replay_data::rows::HumanControllerRow {
                        country: 101,
                        count: 0,
                    },
                ]));
            tables[TableId::SunkShips.index()] = TableView::Complete(TableRows::SunkShips(vec![
                replay_data::rows::SunkShipsRow::default(),
            ]));
            tables[TableId::StateFlags.index()] = TableView::Complete(TableRows::StateFlags(vec![
                replay_data::rows::StateFlagRow {
                    border_conflict_active: replay_data::rows::Recorded::Value(0),
                    border_conflict_countries: replay_data::rows::Recorded::Value(Vec::new()),
                    state: 1,
                    demilitarized: 0,
                },
            ]));
            tables[TableId::CountryGameScoreInputs.index()] =
                TableView::Complete(TableRows::CountryGameScoreInputs(vec![
                    replay_data::rows::CountryGameScoreInputRow {
                        country: 101,
                        army_count: Some(0),
                        navy_count: Some(0),
                        land_count: Some(2),
                        air_state: replay_data::rows::CountryGameScoreInputRow::AIR_ABSENT,
                        ..Default::default()
                    },
                ]));
            tables[TableId::GameScoreMultipliers.index()] =
                TableView::Complete(TableRows::GameScoreMultipliers(vec![
                    replay_data::rows::GameScoreMultiplierRow {
                        total_raw: Some(0),
                        army_raw: Some(0),
                        navy_raw: Some(0),
                        air_raw: Some(0),
                        industry_raw: Some(0),
                        land_raw: Some(0),
                        ..Default::default()
                    },
                ]));
            for table in TableId::ALL {
                if let Some(reason) =
                    replay_data::format::FormatVersion::CURRENT.table_refusal(table)
                {
                    tables[table.index()] = TableView::Unavailable { reason };
                }
            }
            tables.map(Into::into)
        },
    };
    let membership = save::MapMembership {
        provinces: BTreeMap::from([(0, None), (1, Some(1)), (2, None)]),
    }
    .into();
    (
        frame,
        save,
        membership,
        content_reader_key_names::equipment::Database::default(),
    )
}

pub fn add_country(frame: &mut DecodedFrame, save: &mut Save, id: u32, slot: u32, tag: &[u8]) {
    let mut saved = save.countries[b"GER".as_slice()].clone();
    saved.tag = tag.to_vec();
    save.countries.insert(tag.to_vec(), saved);
    save.counter_inputs
        .division_orders
        .insert(tag.to_vec(), Vec::new());
    save.counter_inputs
        .fleet_orders
        .insert(tag.to_vec(), Vec::new());
    let TableView::Complete(TableRows::CountryGameScoreInputs(rows)) =
        &mut *frame.tables[TableId::CountryGameScoreInputs.index()]
    else {
        unreachable!()
    };
    rows.push(replay_data::rows::CountryGameScoreInputRow {
        country: id,
        army_count: Some(0),
        navy_count: Some(0),
        land_count: Some(0),
        air_state: replay_data::rows::CountryGameScoreInputRow::AIR_ABSENT,
        ..Default::default()
    });
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    let mut row = rows[0].clone();
    row.id = id;
    row.canonical_slot = slot;
    row.tag = tag.to_vec();
    rows.push(row);
    let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    rows.push(CountryLinkRow {
        id,
        canonical_slot: slot,
    });
    let TableView::Complete(TableRows::HumanControllers(rows)) =
        &mut *frame.tables[TableId::HumanControllers.index()]
    else {
        unreachable!()
    };
    rows.push(replay_data::rows::HumanControllerRow {
        country: id,
        count: 0,
    });
}

pub fn division_fixture() -> (
    DecodedFrame,
    Save,
    save::PartialMapMembership,
    content_reader_key_names::equipment::Database,
) {
    let (mut frame, mut save, membership, equipment) = fixture();
    add_country(&mut frame, &mut save, 201, 8, b"D01");
    add_country(&mut frame, &mut save, 301, 9, b"SOV");
    let id = (2_147_483_649_u64 << 32) | 4919;
    let template = (u64::from(u32::MAX) << 32) | 6000;
    save.divisions.insert(
        id,
        save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"D01".to_vec(),
            expeditionary_owner: Some(b"SOV".to_vec()),
            province: Some(1),
            template: Some(template),
            alternate_template: None,
            separate_counter: false,
            hit_points: Decimal::parse(b"90071992547.40993").unwrap(),
            organisation: Decimal::parse(b"53.45454").unwrap(),
            country_intel: Vec::new(),
            presentation: save::DivisionPresentation {
                max_supply: Some(Decimal::parse(b"0").unwrap()),
                army_current_supply_ratio: Some(Decimal::parse(b"0").unwrap()),
                ..Default::default()
            },
        },
    );
    *frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id,
            roster_owner: 102,
            logical_owner: 201,
            expeditionary_owner: 301,
            province: 1,
            template,
            hit_points: 9_007_199_254_740_000,
            organisation: 5_340_000,
            orders_group: 0,
            reveal_skip: 0,
            theatre: Recorded::Value(Some(0)),
        }]));
    (frame, save, membership, equipment)
}

pub fn membership(history: &str, province_count: u32) -> save::PartialMapMembership {
    read_membership(history, province_count).unwrap()
}

pub fn read_membership(
    history: &str,
    province_count: u32,
) -> io::Result<save::PartialMapMembership> {
    with_content(history, province_count, save::read_partial_map_membership)
}

pub fn with_content<T>(
    history: &str,
    province_count: u32,
    read: impl FnOnce(&content_reader_layers::ResolvedLayers) -> io::Result<T>,
) -> io::Result<T> {
    with_content_files(history, province_count, &[], read)
}

pub fn with_content_files<T>(
    history: &str,
    province_count: u32,
    extra_files: &[(&str, &str)],
    read: impl FnOnce(&content_reader_layers::ResolvedLayers) -> io::Result<T>,
) -> io::Result<T> {
    if !engine_executable::executable_tables::is_initialized() {
        content_reader_script::test_game::enter();
    }
    struct Content(std::path::PathBuf);
    impl Drop for Content {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let content = Content(std::env::temp_dir().join(format!(
        "hoi4_oracle_state_history_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    )));
    let definitions = (0..=province_count)
        .map(|id| format!("{id};{id};0;0;land;false;test\n"))
        .collect::<String>();
    for (path, bytes) in [
        (
            "map/default.map",
            "definitions=\"definition.csv\" provinces=\"unused.bmp\" terrain=\"unused.bmp\" \
             rivers=\"unused.bmp\" heightmap=\"unused.bmp\" tree_definition=\"unused.csv\"",
        ),
        ("map/definition.csv", definitions.as_str()),
        (
            "common/defines/00.lua",
            "NDefines={NBuildings={MAX_BUILDING_LEVELS=20}}",
        ),
        (
            "common/buildings/00.txt",
            "buildings={civ={general_production=1}}",
        ),
        ("history/states/test.txt", history),
        ("common/resources/00.txt", "resources={synthetic={}}"),
    ]
    .into_iter()
    .chain(extra_files.iter().copied())
    {
        let path = content.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let bitmap_path = content.0.join("map/unused.bmp");
    if !bitmap_path.exists() {
        let mut bmp = vec![0_u8; 58];
        bmp[..2].copy_from_slice(b"BM");
        for (at, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
            bmp[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
        if province_count >= 1 {
            bmp[54..57].copy_from_slice(&[0, 0, 1]);
        }
        std::fs::write(bitmap_path, bmp).unwrap();
    }
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&content.0, &[])]).unwrap();
    read(&layers)
}
