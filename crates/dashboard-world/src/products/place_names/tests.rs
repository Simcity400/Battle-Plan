use std::sync::atomic::AtomicBool;
use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, content_identity, records, source};
use replay::campaign::Replay;
use replay_data::rows::{CountryAppearanceRow, CountryLinkRow, ProvinceNameRow, ProvinceRow, Recorded, ReservedCountryNamingRow, TableRows, TableSchema};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(0));
    set::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 9001,
            original_tag_reference: Recorded::Value(Some(1)),
            original_tag: Recorded::Value(Some(if hour < 106 { b"ORI".to_vec() } else { b"ALT".to_vec() })),
            ..Default::default()
        }],
    );
    set::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            tag: b"---".to_vec(),
            original_tag_reference: Recorded::Value(Some(0)),
            ..Default::default()
        }],
    );
    set::<tables::Provinces>(
        &mut frame,
        vec![
            ProvinceRow { id: 17, state: 64, owner: 9001, controller: 9001, victory_points: 10, ..Default::default() },
            ProvinceRow { id: 18, state: 64, owner: 9001, controller: 9001, ..Default::default() },
            ProvinceRow { id: 19, state: 64, owner: 9001, controller: 9001, ..Default::default() },
            ProvinceRow { id: 21, state: 0, owner: 0, controller: 0, ..Default::default() },
            ProvinceRow { id: 22, state: 0, owner: 0, controller: 0, ..Default::default() },
        ],
    );
    set::<tables::ProvinceNames>(&mut frame, vec![ProvinceNameRow { province: 18, name: b"RENAME".to_vec() }]);
    set::<tables::States>(
        &mut frame,
        vec![
            StateRow { id: 64, owner: 9001, controller: 9001, ..Default::default() },
            StateRow { id: 65, owner: 9001, custom_name: b"Custom state".to_vec(), ..Default::default() },
        ],
    );
    frame
}

fn fixture_source(frames: Vec<DecodedFrame>) -> (Source, LoadContext, std::path::PathBuf, impl Drop) {
    let mut definitions = String::from("0;0;0;0;land;false;unknown;0\n");
    for id in 1..=22_u32 {
        let (kind, coastal, terrain, continent) = if id >= 21 { ("sea", "false", "ocean", 0) } else { ("land", if id == 17 { "true" } else { "false" }, "plains", 1) };
        definitions.push_str(&format!("{id};{};0;0;{kind};{coastal};{terrain};{continent}\n", id * 10));
    }
    let (_cat, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 23 }, NBuildings = { MAX_BUILDING_LEVELS = 13 }, NNavy = { BATTLE_NAME_VP_FACTOR = 1.0, BATTLE_NAME_VP_CUTOFF = 5.0 } }",
        ),
        ("map/default.map", "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp"),
        ("map/definition.csv", &definitions),
        ("map/strategicregions/1.txt", "strategic_region={id=1 name=\"REG_SEA\" provinces={17 21}}"),
        ("history/states/mod.txt", "state={id=64 name=MOD_STATE provinces={17 18 19}}"),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n MOD_STATE:0 \"Mod state\"\n ORI_VICTORY_POINTS_17:0 \"Original town\"\n ALT_VICTORY_POINTS_17:0 \"Switched town\"\n GER_VICTORY_POINTS_17:0 \"Own-tag town\"\n RENAME:0 \"Renamed town\"\n EMPTY_RENAME:0 \"\"\n REG_SEA:0 \"Sample Sea\"\n NAVAL_BATTLE_TITLE:0 \"Battle of $NAME$\"\n NAVAL_PORT_STRIKE_TITLE:0 \"Port Strike in $NAME$\"\n",
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
    let mut input = records(1, frames);
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
    (source, LoadContext { content: Some(options) }, saved, folder)
}

#[test]
fn places_are_named_by_rename_victory_point_then_state_and_a_place_not_read_is_a_reason() {
    let (source, load, saved, _folder) = fixture_source((100..=110).map(frame).collect());
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

    let run = query.timeline.stretches[0].run;
    let history = PlaceHistory::load(query.replay.recording(), run, 100, 110, Keys::All, Keys::All, true, &AtomicBool::new(false)).unwrap();
    assert_eq!(history.province_change_hours(17, 100, 110), vec![100, 106]);
    assert_eq!(history.province_name(&content, 17, ProvinceMode::Land, 105).name.as_deref(), Some("Original town"));
    assert_eq!(history.province_name(&content, 17, ProvinceMode::Land, 106).name.as_deref(), Some("Switched town"));
    assert_eq!(history.land_battle_title(&content, 17, 105).as_deref(), Ok("Battle of Original town"));
    assert_eq!(history.naval_battle_title(&content, 21, false, 105).as_deref(), Ok("Battle of Original town"));
    assert_eq!(history.naval_battle_title(&content, 21, true, 105).as_deref(), Ok("Port Strike in Sample Sea"));
    assert_eq!(history.province_name(&content, 22, ProvinceMode::Naval, 105).name.as_deref(), Some("22"));
    assert!(history.province_name(&content, 22, ProvinceMode::Land, 105).name.is_none());

    drop(query);
    drop(source);
    std::fs::remove_dir_all(&saved).unwrap();
}

#[test]
fn truncated_tables_use_witnessed_rows_refuse_absent_renames_and_preserve_vp_tag_fallback_for_place_mode() {
    let frames: Vec<_> = (99..=103)
        .map(|hour| {
            let mut f = frame(hour);
            match hour {
                99 => set::<tables::ProvinceNames>(&mut f, Vec::new()),
                100 => {
                    *f.tables[TableId::ProvinceNames.index()] = TableView::Truncated {
                        rows: TableRows::ProvinceNames(vec![ProvinceNameRow { province: 18, name: b"RENAME".to_vec() }]),
                        missing: 2,
                    };
                }
                101 => {
                    *f.tables[TableId::CountryLinks.index()] = TableView::Truncated {
                        rows: TableRows::CountryLinks(vec![CountryLinkRow { id: 9001, canonical_slot: 7 }]),
                        missing: 1,
                    };
                }
                102 => {
                    set::<tables::CountryAppearance>(&mut f, Vec::new());
                }
                _ => {
                    set::<tables::ProvinceNames>(&mut f, vec![ProvinceNameRow { province: 18, name: b"EMPTY_RENAME".to_vec() }]);
                }
            }
            f
        })
        .collect();
    let (source, load, saved, _folder) = fixture_source(frames);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let content = Content::read(&source, &load).unwrap();
    let mut frames = at_hour::Frames::new(&query).unwrap();
    let at_trunc_rename = read_hour(&query, &mut frames, 100, &[18, 19], &[]).unwrap();
    let places = Places::new(&at_trunc_rename);
    assert_eq!(places.province(&content, 18, false).name.as_deref(), Some("Renamed town"));
    let refused = places.province(&content, 19, false);
    assert_eq!(refused.name, None, "a truncated rename table never guesses the state name");
    assert!(refused.missing.as_deref().is_some_and(|why| why.contains("truncated")));

    let at_trunc_link = read_hour(&query, &mut frames, 101, &[17], &[]).unwrap();
    assert_eq!(Places::new(&at_trunc_link).province(&content, 17, false).name.as_deref(), Some("Original town"));

    let run = query.timeline.stretches[0].run;
    let history = PlaceHistory::load(query.replay.recording(), run, 100, 103, Keys::All, Keys::All, false, &AtomicBool::new(false)).unwrap();
    assert_eq!(history.province_name(&content, 18, ProvinceMode::Place, 100).name.as_deref(), Some("Renamed town"));
    let hist_refused = history.province_name(&content, 19, ProvinceMode::Place, 100);
    assert_eq!(hist_refused.name, None);
    assert!(hist_refused.missing.as_deref().is_some_and(|why| why.contains("truncated")));
    assert_eq!(history.province_name(&content, 17, ProvinceMode::Place, 101).name.as_deref(), Some("Original town"), "a witnessed link in a truncated CountryLinks table resolves its VP tag");
    let vp_fallback = history.province_name(&content, 19, ProvinceMode::Place, 102);
    assert_eq!(vp_fallback.name.as_deref(), Some("Mod state"));
    assert!(vp_fallback.missing.is_some());
    assert!(history.land_battle_title(&content, 19, 102).is_err(), "battle titles refuse when VP tag lookup fails");
    assert_eq!(history.province_name(&content, 18, ProvinceMode::Naval, 103).name.as_deref(), Some(""), "naval mode returns an empty translated rename immediately");

    drop(query);
    drop(source);
    std::fs::remove_dir_all(&saved).unwrap();
}
