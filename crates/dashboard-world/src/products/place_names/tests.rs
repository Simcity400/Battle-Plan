use std::sync::atomic::AtomicBool;
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

#[test]
fn places_are_named_by_rename_victory_point_then_state_and_a_place_not_read_is_a_reason() {
    let (_cat, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        ("common/defines/00_defines.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 23 }, NBuildings = { MAX_BUILDING_LEVELS = 13 } }"),
        ("map/default.map", "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp"),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        ("history/states/mod.txt", "state={id=64 name=MOD_STATE}"),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n MOD_STATE:0 \"Mod state\"\n ORI_VICTORY_POINTS_17:0 \"Original town\"\n GER_VICTORY_POINTS_17:0 \"Own-tag town\"\n RENAME:0 \"Renamed town\"\n",
        ),
    ]);
    let mut bitmap = [0_u8; 58];
    bitmap[..2].copy_from_slice(b"BM");
    for (offset, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
        bitmap[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
    std::fs::write(folder.0.join("map/provinces.bmp"), bitmap).unwrap();
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
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let content = Content::read(&source, &load).unwrap();
    let mut frames = at_hour::Frames::new(&query).unwrap();
    let frame = read_hour(&query, &mut frames, 105, &[17, 18, 19, 20], &[64, 65, 66]).unwrap();
    let places = Places::new(&frame);
    let names = [17, 18, 19, 20].map(|id| places.province(&content, id, false));
    assert_eq!(names.each_ref().map(|p| p.name.as_deref()), [Some("Original town"), Some("Renamed town"), Some("Mod state"), None], "{names:?}");
    assert!(names[..3].iter().all(|p| p.missing.is_none()));
    assert_eq!(names[3].missing.as_deref(), Some("Not in this recording: province 20 at this hour"));
    assert!(read_hour(&query, &mut frames, 500, &[17], &[]).err().unwrap().starts_with("Not in this recording"));
    let states = [64, 65, 66].map(|id| places.state(&content, id));
    assert_eq!(states.each_ref().map(|s| s.name.as_deref()), [Some("Mod state"), Some("Custom state"), None]);
    assert_eq!(states[2].missing.as_deref(), Some("Not in this recording: state 66 at this hour"));
    drop(query);
    drop(source);
    std::fs::remove_dir_all(&saved).unwrap();
}
