use super::*;
use crate::catalogue::tests::folder_catalogue;
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{ProvinceRow, TableRows},
};

fn geo() -> Geography {
    Geography {
        geometry: ProvinceGeometry {
            width: 100,
            centroids: BTreeMap::from([(1, [95, 0]), (20, [5, 0]), (21, [85, 0])]),
            ..Default::default()
        },
        regions: BTreeMap::from([(
            7,
            StrategicRegion {
                id: 7,
                name_key: "mod_sea".into(),
                ..Default::default()
            },
        )]),
        province_regions: BTreeMap::from([(1, Ok(7))]),
        candidates: BTreeMap::from([(7, vec![21, 20])]),
        factor: 10_000_000,
        cutoff: 100_000,
        warnings: vec![],
    }
}

fn points(values: &[(u32, i32)]) -> BTreeMap<u32, i32> {
    values.iter().copied().collect()
}

fn frame(points: &[(u32, i32)]) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 119,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not recorded".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        points
            .iter()
            .map(|&(id, victory_points)| ProvinceRow {
                id,
                victory_points,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame
}

#[test]
fn sinking_titles_use_content_formatter_and_text_argument_without_an_english_prefix() {
    let (cat, _folder) = folder_catalogue(&[(
        "localisation/mod_l_english.yml",
        "\u{feff}l_english:\n NAVAL_BATTLE_TITLE:0 \"Engagement: \u{a7}Y$NAME$\u{a7}! [literal]\"\n",
    )]);
    assert_eq!(
        title(&cat, "Mod port"),
        Ok("Engagement: Mod port [literal]".into())
    );
    let (missing, _folder) = folder_catalogue(&[]);
    assert_eq!(title(&missing, "Mod port"), Ok("NAVAL_BATTLE_TITLE".into()));
    let (empty, _folder) = folder_catalogue(&[(
        "localisation/mod_l_english.yml",
        "\u{feff}l_english:\n NAVAL_BATTLE_TITLE:0 \"\"\n",
    )]);
    assert_eq!(title(&empty, "Mod port"), Ok(String::new()));
}

#[test]
fn titles_use_the_shared_native_pick_and_refuse_missing_operands() {
    let geo = geo();
    // Equal wrapped distances keep the first candidate in native content order.
    assert_eq!(
        pick(&geo, &points(&[(20, 1), (21, 1)]), 1),
        Ok((7, Some(21)))
    );
    assert_eq!(
        pick(&geo, &points(&[(20, 3), (21, 1)]), 1),
        Ok((7, Some(20)))
    );
    assert_eq!(pick(&geo, &points(&[(20, 0), (21, 0)]), 1), Ok((7, None)));
    assert!(
        pick(&geo, &points(&[(20, 3)]), 1)
            .unwrap_err()
            .contains("province 21")
    );
    assert!(
        pick(&geo, &points(&[]), 999)
            .unwrap_err()
            .contains("province 999")
    );
    let mut ambiguous = geo;
    ambiguous
        .province_regions
        .insert(1, Err("ambiguous region".into()));
    assert_eq!(
        pick(&ambiguous, &points(&[(20, 3), (21, 1)]), 1),
        Err("ambiguous region".into())
    );
}

#[test]
fn partial_province_tables_keep_proven_points_and_missing_titles_are_counted() {
    let mut frame = frame(&[(20, 1), (21, 1)]);
    let rows = recorded_rows::<tables::Provinces>(&frame).unwrap().to_vec();
    frame.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(rows),
        missing: 1,
    }
    .into();
    let points = recorded_rows::<tables::Provinces>(&frame)
        .unwrap()
        .iter()
        .map(|row| (row.id, row.victory_points))
        .collect();
    assert_eq!(pick(&geo(), &points, 1), Ok((7, Some(21))));
    let reply = checked(ShipsSunkBattleTitlesReply {
        rows: vec![
            ShipBattleTitle {
                index: 0,
                hour: 119,
                name: None,
                missing: Some("Battle province not recorded".into()),
            },
            ShipBattleTitle {
                index: 1,
                hour: 119,
                name: None,
                missing: Some("Battle province not recorded".into()),
            },
        ],
        warnings: vec![],
    })
    .unwrap();
    assert!(
        reply.warnings[0]
            .starts_with("2 Ships sunk battle titles not shown: Battle province not recorded")
    );
}

#[test]
fn selected_title_names_keep_current_original_reserved_and_rename_operands() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay::{
        container::Replay,
        rows::{
            CountryAppearanceRow, CountryLinkRow, CountryRow, ProvinceNameRow, Recorded,
            ReservedCountryNamingRow, StateRow,
        },
    };
    let (cat, folder) = folder_catalogue(&[
        ("hoi4.exe", "test"),
        (
            "user/settings.txt",
            "language=l_english force_pow2_textures=no",
        ),
        (
            "user/dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        ),
        (
            "common/defines/fixture.lua",
            "NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 15 }, NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continents.txt",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;fixture;1\n"),
        ("map/continents.txt", "continents={fixture_continent}"),
        ("history/states/fixture.txt", "state={id=64 name=MOD_STATE}"),
        (
            "map/strategicregions/fixture.txt",
            "strategic_region={id=7 name=MOD_SEA provinces={19}}",
        ),
        (
            "localisation/fixture_l_english.yml",
            concat!(
                "\u{feff}l_english:\n NAVAL_BATTLE_TITLE:0 \"Engagement $NAME$\"\n",
                " MOD_ORIGINAL_VICTORY_POINTS_17:0 \"Original town\"\n",
                " MOD_CURRENT_VICTORY_POINTS_17:0 \"Wrong current tag\"\n",
                " MOD_RESERVED_VICTORY_POINTS_19:0 \"Reserved town\"\n",
                " RENAME:0 \"Renamed town\"\n MOD_SEA:0 \"Mod sea\"\n",
            ),
        ),
    ]);
    let mut f = crate::test_fixtures::source(1, 119, Some(1));
    f.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 11,
            canonical_slot: 7,
            tag: b"MOD_CURRENT".to_vec(),
            ..Default::default()
        }]))
        .into();
    f.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 11,
            canonical_slot: 7,
        }]))
        .into();
    f.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 11,
            original_tag_reference: Recorded::Value(Some(1)),
            original_tag: Recorded::Value(Some(b"MOD_ORIGINAL".to_vec())),
            ..Default::default()
        }]))
        .into();
    f.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"MOD_EMPTY".to_vec(),
                original_tag_reference: Recorded::Value(Some(1)),
                original_tag: Recorded::Value(Some(b"MOD_RESERVED".to_vec())),
                ..Default::default()
            },
        ]))
        .into();
    f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 64,
        owner: 11,
        ..Default::default()
    }]))
    .into();
    f.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        ProvinceRow {
            id: 17,
            state: 64,
            ..Default::default()
        },
        ProvinceRow {
            id: 18,
            state: 64,
            ..Default::default()
        },
        ProvinceRow {
            id: 19,
            ..Default::default()
        },
    ]))
    .into();
    f.tables[TableId::ProvinceNames.index()] =
        TableView::Complete(TableRows::ProvinceNames(vec![ProvinceNameRow {
            province: 18,
            name: b"RENAME".to_vec(),
        }]))
        .into();
    let (content, identity) = crate::test_fixtures::content_identity(&folder.0);
    let mut records = crate::test_fixtures::records(1, vec![f]);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "naval naming fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(crate::test_fixtures::CAMPAIGN.into(), &[records]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext {
        content: Some(content),
    };
    let names = place_names::PlaceNames::load(
        &source,
        &place_names::PlaceNamesParams {
            land: false,
            branch_id: "1".into(),
            places: [17, 18, 19]
                .into_iter()
                .map(|province_id| place_names::PlaceNameRequest {
                    province_id,
                    hour: 119,
                })
                .collect(),
            states: vec![],
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    let titles: Vec<_> = names
        .places
        .iter()
        .map(|row| {
            assert!(row.missing.is_none(), "{row:?}");
            title(&cat, row.name.as_deref().unwrap()).unwrap()
        })
        .collect();
    assert_eq!(
        titles,
        [
            "Engagement Original town",
            "Engagement Renamed town",
            "Engagement Reserved town"
        ]
    );
}
