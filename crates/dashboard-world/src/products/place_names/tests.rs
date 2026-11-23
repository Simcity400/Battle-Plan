use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, content_identity, records, source};
use replay::campaign::Replay;
use replay_data::rows::{CountryAppearanceRow, ProvinceNameRow, ProvinceRow, Recorded, ReservedCountryNamingRow, TableSchema};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(0));
    set::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow { country: 9001, original_tag_reference: Recorded::Value(Some(1)), original_tag: Recorded::Value(Some(b"ORI".to_vec())), ..Default::default() }],
    );
    set::<tables::ReservedCountryNaming>(&mut frame, vec![ReservedCountryNamingRow::default()]);
    set::<tables::Provinces>(&mut frame, [17, 18, 19].map(|id| ProvinceRow { id, state: 64, owner: 9001, controller: 9001, ..Default::default() }).to_vec());
    set::<tables::ProvinceNames>(&mut frame, vec![ProvinceNameRow { province: 18, name: b"RENAME".to_vec() }]);
    set::<tables::States>(
        &mut frame,
        vec![StateRow { id: 64, owner: 9001, controller: 9001, ..Default::default() }, StateRow { id: 65, owner: 9001, custom_name: b"Custom state".to_vec(), ..Default::default() }],
    );
    frame
}

fn named(source: &Source, load: &LoadContext, places: &[(u32, i32)], states: &[(u32, i32)]) -> PlaceNamesReply {
    let params = PlaceNamesParams {
        branch_id: "1".into(),
        land: false,
        places: places.iter().map(|&(province_id, hour)| PlaceNameRequest { province_id, hour }).collect(),
        states: states.iter().map(|&(state_id, hour)| StateNameRequest { state_id, hour }).collect(),
    };
    PlaceNames::load(source, &params, load, &AtomicBool::new(false)).unwrap()
}

#[test]
fn places_are_named_by_rename_victory_point_then_state_and_a_place_not_read_is_a_reason() {
    engine_executable::executable_tables::initialize_for_tests().expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_cat, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        ("common/defines/00_defines.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NBuildings = { MAX_BUILDING_LEVELS = 15 } }"),
        ("map/default.map", "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp"),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        ("history/states/mod.txt", "state={id=64 name=MOD_STATE}"),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n MOD_STATE:0 \"Mod state\"\n ORI_VICTORY_POINTS_17:0 \"Original town\"\n GER_VICTORY_POINTS_17:0 \"Own-tag town\"\n RENAME:0 \"Renamed town\"\n",
        ),
    ]);
    let (options, identity) = content_identity(&folder.0);
    let mut input = records(1, (100..=110).map(frame).collect());
    let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] else { unreachable!() };
    session.identity = identity;
    let saved = std::env::temp_dir().join(format!("place-names-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&saved).unwrap();
    let path = saved.join("synthetic.battleplan");
    Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[input]).unwrap().write_atomic(&path).unwrap();
    let folders = recording_upgrade::Folders { games: vec![folder.0.clone()], user: folder.0.join("user"), cache: folder.0.join("cache") };
    recording_upgrade::upgrade(&path, &folders).unwrap();
    let replay = Replay::open_newest(&path).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "place-names".into(), revision: "1".into() }, path, Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let load = LoadContext { content: Some(options) };
    let reply = named(&source, &load, &[(17, 105), (18, 105), (19, 105), (20, 105), (17, 500)], &[(64, 105), (65, 105), (66, 105)]);
    let names: Vec<Option<&str>> = reply.places.iter().map(|place| place.name.as_deref()).collect();
    assert_eq!(names, [Some("Original town"), Some("Renamed town"), Some("Mod state"), None, None], "the victory point is the state owner's original tag's");
    assert!(reply.places[..3].iter().all(|place| place.missing.is_none()));
    assert_eq!(reply.places[3].missing.as_deref(), Some("Not in this recording: province 20 at this hour"));
    assert!(reply.places[4].missing.as_deref().is_some_and(|reason| reason.starts_with("Not in this recording")), "an hour no run holds is named, never guessed");
    let states: Vec<Option<&str>> = reply.states.iter().map(|state| state.name.as_deref()).collect();
    assert_eq!(states, [Some("Mod state"), Some("Custom state"), None]);
    assert_eq!(reply.states[2].missing.as_deref(), Some("Not in this recording: state 66 at this hour"));
    let asked = [(18, 101), (17, 103), (19, 103), (18, 107), (17, 110)];
    let together = named(&source, &load, &asked, &[(64, 102), (65, 109)]);
    for (at, &(province, hour)) in asked.iter().enumerate() {
        let alone = named(&source, &load, &[(province, hour)], &[]);
        assert_eq!((together.places[at].name.as_deref(), together.places[at].missing.as_deref()), (alone.places[0].name.as_deref(), alone.places[0].missing.as_deref()), "province {province} at {hour}");
    }
    assert_eq!(together.states.iter().map(|state| state.name.as_deref()).collect::<Vec<_>>(), [Some("Mod state"), Some("Custom state")]);
    drop(source);
    std::fs::remove_dir_all(&saved).unwrap();
}

#[test]
fn a_request_naming_every_place_a_widget_shows_is_taken_whatever_its_count() {
    let params = PlaceNamesParams {
        branch_id: "1".into(),
        land: false,
        places: (0..5_000).map(|province_id| PlaceNameRequest { province_id, hour: 10 }).collect(),
        states: (0..5_000).map(|state_id| StateNameRequest { state_id, hour: 10 }).collect(),
    };
    assert_eq!(<PlaceNames as Product>::validate(&params), Ok(()));
}
