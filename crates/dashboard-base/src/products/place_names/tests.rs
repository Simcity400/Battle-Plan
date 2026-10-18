use super::*;
use replay_data::{
    rows::CampaignRow, rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow,
    rows::ProvinceNameRow, rows::ProvinceRow, rows::Recorded, rows::StateRow, rows::TableRows,
    rows::TableSchema,
};
use replay_frame::{frame::Metrics, frame::TableView};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|at| {
            if TableId::ALL[at] == TableId::ReservedCountryNaming {
                TableView::Unavailable {
                    reason: "reserved country naming not sampled in place fixture".into(),
                }
            } else if NAME_TABLES.contains(&TableId::ALL[at]) {
                TableView::Complete(TableRows::empty(TableId::ALL[at]))
            } else {
                TableView::Unavailable {
                    reason: "outside place fixture".into(),
                }
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    set::<tables::Campaign>(
        &mut frame,
        vec![CampaignRow {
            id: b"analytics-test".to_vec(),
        }],
    );
    set::<tables::Countries>(
        &mut frame,
        (1..=5_000)
            .map(|id| CountryRow {
                id: id + 10_000,
                canonical_slot: id,
                tag: format!("mod_tag_{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::CountryLinks>(
        &mut frame,
        (1..=5_000)
            .map(|id| CountryLinkRow {
                id: id + 10_000,
                canonical_slot: id,
            })
            .collect(),
    );
    set::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 15_000,
            original_tag_reference: Recorded::Value(Some(1)),
            original_tag: Recorded::Value(Some(b"mod_original".to_vec())),
            ..Default::default()
        }],
    );
    set::<tables::Provinces>(
        &mut frame,
        vec![ProvinceRow {
            id: 17,
            state: 64,
            owner: 15_000,
            controller: 10_001,
            ..Default::default()
        }],
    );
    set::<tables::States>(
        &mut frame,
        vec![StateRow {
            id: 64,
            owner: 15_000,
            controller: 10_001,
            ..Default::default()
        }],
    );
    frame
}

fn catalogue() -> (Catalogue, crate::catalogue::tests::Folder) {
    crate::catalogue::tests::folder_catalogue(&[
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
            "common/defines/00_defines.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NBuildings = { MAX_BUILDING_LEVELS = 15 } }",
        ),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        ("history/states/mod.txt", "state={id=64 name=MOD_STATE}"),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n MOD_STATE:0 \"Mod state\"\n mod_original_VICTORY_POINTS_17:0 \"Original town\"\n mod_tag_1_VICTORY_POINTS_17:0 \"Controller town\"\n VICTORY_POINTS_17:0 \"Generic town\"\n OR0_VICTORY_POINTS_60000:0 \"Reserved town\"\n RENAME:0 \"Renamed town\"\n",
        ),
    ])
}

fn content_source(
    frames: Vec<DecodedFrame>,
    folder: &crate::catalogue::tests::Folder,
) -> (Source, LoadContext) {
    let (options, identity) = crate::test_fixtures::content_identity(&folder.0);
    let mut input = crate::test_fixtures::records(1, frames);
    let replay::spool::Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "state-place-fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(b"analytics-test".to_vec(), &[input]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    (
        source,
        LoadContext {
            content: Some(options),
        },
    )
}

#[test]
fn state_names_use_recorded_custom_owner_and_content_precedence_at_5000_countries() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_cat, folder) = catalogue();
    let mut f = frame(1);
    let template = f.complete::<tables::States>().unwrap()[0].clone();
    set::<tables::States>(
        &mut f,
        vec![
            template.clone(),
            StateRow {
                id: 65,
                owner_name: b"Stored owner state".to_vec(),
                ..template.clone()
            },
            StateRow {
                id: 66,
                custom_name: b"Custom state".to_vec(),
                owner_name: b"Ignored owner name".to_vec(),
                ..template
            },
        ],
    );
    let (source, context) = content_source(vec![f], &folder);
    let params = PlaceNamesParams {
        land: false,
        branch_id: "1".into(),
        places: Vec::new(),
        states: [66, 65, 64, 99999]
            .map(|state_id| StateNameRequest { state_id, hour: 1 })
            .to_vec(),
    };
    let reply = PlaceNames::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert!(reply.places.is_empty());
    for (state, expected) in
        reply.states[..3]
            .iter()
            .zip(["Custom state", "Stored owner state", "Mod state"])
    {
        assert_eq!(state.name.as_deref(), Some(expected));
        assert!(state.missing.is_none());
        assert_eq!(state.hour, 1);
    }
    assert!(reply.states[3].name.is_none());
    assert_eq!(
        reply.states[3].missing.as_deref(),
        Some("Not in this recording: state 99999 at this hour")
    );
}

#[test]
fn state_names_only_need_states_and_the_catalogues_own_state_cache() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_cat, folder) = catalogue();
    std::fs::write(
        folder.0.join("common/defines/00_defines.lua"),
        "NDefines={NBuildings={MAX_BUILDING_LEVELS=15}}",
    )
    .unwrap();
    let mut f = frame(1);
    for table in [TableId::Provinces, TableId::ProvinceNames] {
        *f.tables[table.index()] = TableView::Unavailable {
            reason: "No province read at this hour".into(),
        };
    }
    let (source, context) = content_source(vec![f], &folder);
    let cat = catalogue::catalogue(&source, context.content.as_ref().unwrap()).unwrap();
    let mut content = scripted_text::read_state_content(&cat.content.layers).unwrap();
    content.warnings.extend([
        installed_content_states::states::Diagnostic {
            kind: installed_content_core::DiagnosticKind::Warning,
            message: "native state template skip".into(),
            cause: None,
        },
        installed_content_states::states::Diagnostic {
            kind: installed_content_core::DiagnosticKind::Misread,
            message: "proven state recovery".into(),
            cause: Some(installed_content_states::states::MisreadCause::InvalidImpassable),
        },
        installed_content_states::states::Diagnostic {
            kind: installed_content_core::DiagnosticKind::Unsupported,
            message: "unsupported fixture state form".into(),
            cause: None,
        },
    ]);
    assert!(cat.state_content.set(Ok(content)).is_ok());
    let reply = PlaceNames::load(
        &source,
        &PlaceNamesParams {
            land: false,
            branch_id: "1".into(),
            places: Vec::new(),
            states: vec![StateNameRequest {
                state_id: 64,
                hour: 1,
            }],
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.states[0].name.as_deref(), Some("Mod state"));
    assert!(reply.states[0].missing.is_none());
    assert!(reply.places.is_empty());
    assert!(cat.scripted_content().is_err());
    assert!(
        source
            .state::<PlaceState>()
            .lock()
            .unwrap()
            .content
            .is_none()
    );
    assert_eq!(
        reply
            .warnings
            .iter()
            .filter(|warning| warning.as_str() == "unsupported fixture state form")
            .count(),
        1
    );
    assert!(reply.warnings.iter().all(|warning| {
        !warning.contains("native state template skip")
            && !warning.contains("proven state recovery")
    }));
}

#[test]
fn place_names_mixed_requests_preserve_province_bytes_and_state_only_labels() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_cat, folder) = catalogue();
    let (source, context) = content_source(vec![frame(1)], &folder);
    let mut params = PlaceNamesParams {
        land: false,
        branch_id: "1".into(),
        places: vec![PlaceNameRequest {
            province_id: 17,
            hour: 1,
        }],
        states: Vec::new(),
    };
    let places = PlaceNames::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    params.states.push(StateNameRequest {
        state_id: 64,
        hour: 1,
    });
    let mixed = PlaceNames::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        serde_json::to_vec(&places.places).unwrap(),
        serde_json::to_vec(&mixed.places).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&mixed.places[0]).unwrap(),
        r#"{"provinceId":17,"hour":1,"name":"Original town","missing":null}"#
    );
    assert_eq!(mixed.states[0].name.as_deref(), Some("Mod state"));
    assert!(mixed.states[0].missing.is_none());
}

#[test]
fn state_names_key_recorded_renames_by_each_requested_hour_and_keep_gaps() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_cat, folder) = catalogue();
    let mut first = frame(1);
    let mut states = first.complete::<tables::States>().unwrap().to_vec();
    states[0].custom_name = b"First hour state".to_vec();
    states.push(StateRow {
        id: 65,
        custom_name: b"First hour only state".to_vec(),
        ..Default::default()
    });
    set::<tables::States>(&mut first, states.clone());
    let mut second = first.clone();
    second.hour = 2;
    states[0].custom_name = b"Renamed second hour state".to_vec();
    states.retain(|state| state.id == 64);
    set::<tables::States>(&mut second, states);
    let (source, context) = content_source(vec![first, second], &folder);
    let mut params = PlaceNamesParams {
        land: false,
        branch_id: "1".into(),
        places: Vec::new(),
        states: [2, 1, 3, 2]
            .map(|hour| StateNameRequest { state_id: 64, hour })
            .to_vec(),
    };
    params.states.extend([
        StateNameRequest {
            state_id: 65,
            hour: 2,
        },
        StateNameRequest {
            state_id: 65,
            hour: 1,
        },
    ]);
    let reply = PlaceNames::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reply.states[0].name.as_deref(),
        Some("Renamed second hour state")
    );
    assert_eq!(reply.states[1].name.as_deref(), Some("First hour state"));
    assert_eq!(reply.states[0], reply.states[3]);
    assert_eq!(
        reply
            .states
            .iter()
            .map(|state| state.hour)
            .collect::<Vec<_>>(),
        [2, 1, 3, 2, 2, 1]
    );
    assert!(reply.states[2].name.is_none());
    assert!(
        reply.states[2]
            .missing
            .as_ref()
            .unwrap()
            .contains("the row hour was not recorded")
    );
    assert!(reply.states[4].name.is_none());
    assert_eq!(
        reply.states[4].missing.as_deref(),
        Some("Not in this recording: state 65 at this hour")
    );
    assert_eq!(
        reply.states[5].name.as_deref(),
        Some("First hour only state")
    );
}

#[test]
fn state_names_preserve_successful_empty_names_and_stored_language_failures() {
    let (mut cat, _folder) = catalogue();
    let empty = BTreeMap::from([(64, Ok(String::new()))]);
    let mut f = frame(1);
    let (name, missing) = state_name_at(&f, 64, &cat, Ok(&empty));
    assert_eq!(name.as_deref(), Some(""));
    assert!(missing.is_none());
    let mut states = f.complete::<tables::States>().unwrap().to_vec();
    states[0].custom_name = b"Stored state".to_vec();
    set::<tables::States>(&mut f, states);
    cat.recorded_language = "other-language".into();
    let (name, missing) = state_name_at(&f, 64, &cat, Err("state templates unavailable"));
    assert!(name.is_none());
    assert!(missing.unwrap().contains("other-language"));
    *f.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "No state read".into(),
    };
    assert_eq!(
        state_name_at(&f, 64, &cat, Ok(&empty)),
        (
            None,
            Some("Not in this recording: state 64 at this hour".into())
        )
    );
}

#[test]
fn place_names_return_all_states_and_refuse_the_combined_transport_budget() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_cat, folder) = catalogue();
    let (source, context) = content_source(vec![frame(1)], &folder);
    let state = StateNameRequest {
        state_id: 64,
        hour: 1,
    };
    let mut params = PlaceNamesParams {
        land: false,
        branch_id: "1".into(),
        places: Vec::new(),
        states: vec![state.clone(); 5000],
    };
    let reply = PlaceNames::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.states.len(), params.states.len());
    assert!(
        reply
            .states
            .iter()
            .all(|state| { state.name.as_deref() == Some("Mod state") && state.missing.is_none() })
    );
    let context = LoadContext { content: None };
    let reply = PlaceNames::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    let entry_bytes = serde_json::to_vec(&reply.states[0]).unwrap().len() + 1;
    params
        .states
        .resize(MAX_PRODUCT_REPLY_BYTES / entry_bytes + 1, state);
    let error = PlaceNames::load(&source, &params, &context, &AtomicBool::new(false)).unwrap_err();
    assert!(error.to_string().contains("reply exceeds its size budget"));
    let states = params.states.split_off(params.states.len() / 2);
    params.places = states
        .iter()
        .map(|request| PlaceNameRequest {
            province_id: request.state_id,
            hour: request.hour,
        })
        .collect();
    let error = PlaceNames::load(&source, &params, &context, &AtomicBool::new(false)).unwrap_err();
    assert!(error.to_string().contains("reply exceeds its size budget"));
}

#[test]
fn place_names_use_the_recorded_owner_and_stored_name_precedence_at_5000_countries() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (cat, _folder) = catalogue();
    let names = BTreeMap::from([(64, Ok("MOD_STATE".into()))]);
    let mut f = frame(1);
    assert_eq!(name(&f, 17, &cat, Ok(&names)).unwrap(), "Original town");
    set::<tables::ProvinceNames>(
        &mut f,
        vec![ProvinceNameRow {
            province: 17,
            name: b"RENAME".to_vec(),
        }],
    );
    let mut states = f.complete::<tables::States>().unwrap().to_vec();
    states[0].owner_name = b"Stored owner state".to_vec();
    set::<tables::States>(&mut f, states.clone());
    assert_eq!(
        name(&f, 17, &cat, Err("unreadable state content")).unwrap(),
        "Renamed town"
    );
    states[0].custom_name = b"Custom state".to_vec();
    set::<tables::States>(&mut f, states);
    assert_eq!(name(&f, 17, &cat, Ok(&names)).unwrap(), "Renamed town");
    // The name and state lookup use row IDs, without a chosen province count.
    set::<tables::Provinces>(
        &mut f,
        vec![ProvinceRow {
            id: 60_000,
            state: 64,
            ..Default::default()
        }],
    );
    set::<tables::ProvinceNames>(
        &mut f,
        vec![ProvinceNameRow {
            province: 60_000,
            name: b"RENAME".to_vec(),
        }],
    );
    assert_eq!(name(&f, 60_000, &cat, Ok(&names)).unwrap(), "Renamed town");
    set::<tables::ProvinceNames>(&mut f, Vec::new());
    assert_eq!(name(&f, 60_000, &cat, Ok(&names)).unwrap(), "Custom state");
}

#[test]
fn place_names_refuse_missing_rename_data_and_preserve_recorded_language() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut cat, _folder) = catalogue();
    let names = BTreeMap::from([(64, Ok("MOD_STATE".into()))]);
    let mut f = frame(1);
    *f.tables[TableId::ProvinceNames.index()] = TableView::Unavailable {
        reason: "Not in this recording".into(),
    };
    let (known, error) = resolved(&f, 17, &cat, Ok(&names), Ok(&BTreeMap::new()));
    assert_eq!(known.as_deref(), Some("Mod state"));
    let error = error.unwrap();
    assert!(
        error.contains("ProvinceNames") && error.contains("Not in this recording"),
        "{error}"
    );
    set::<tables::ProvinceNames>(&mut f, Vec::new());
    set::<tables::Provinces>(
        &mut f,
        vec![ProvinceRow {
            id: 60000,
            state: 64,
            ..Default::default()
        }],
    );
    let mut states = f.complete::<tables::States>().unwrap().to_vec();
    states[0].custom_name = b"Stored state".to_vec();
    set::<tables::States>(&mut f, states);
    cat.recorded_language = "other-language".into();
    assert!(
        resolved(&f, 60000, &cat, Ok(&names), Ok(&BTreeMap::new()))
            .1
            .unwrap()
            .contains("other-language")
    );
}

#[test]
fn place_names_selected_reads_keep_each_row_hour_and_gaps() {
    let (cat, _folder) = catalogue();
    let names = BTreeMap::from([(64, Ok("MOD_STATE".into()))]);
    let mut first = frame(1);
    set::<tables::Provinces>(
        &mut first,
        vec![ProvinceRow {
            id: 60000,
            state: 64,
            ..Default::default()
        }],
    );
    let mut second = first.clone();
    second.hour = 2;
    let mut states = second.complete::<tables::States>().unwrap().to_vec();
    states[0].custom_name = b"Second hour state".to_vec();
    set::<tables::States>(&mut second, states);
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, vec![first, second])],
    )
    .unwrap();
    let requests = [2, 1, 3, 2].map(|hour| PlaceNameRequest {
        province_id: 60000,
        hour,
    });
    let (places, states) = read_names(
        &replay,
        1,
        &PlaceNamesParams {
            land: false,
            branch_id: "1".into(),
            places: requests.to_vec(),
            states: Vec::new(),
        },
        &cat,
        Ok(&names),
        Ok(&BTreeMap::new()),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(states.is_empty());
    assert_eq!(places[0].name.as_deref(), Some("Second hour state"));
    assert_eq!(places[1].name.as_deref(), Some("Mod state"));
    assert!(places[2].name.is_none());
    assert!(
        places[2]
            .missing
            .as_ref()
            .unwrap()
            .contains("Not in this recording")
    );
    assert_eq!(places[0], places[3]);
}

#[test]
fn place_names_have_no_chosen_request_count_limit() {
    let mut params = PlaceNamesParams {
        land: false,
        branch_id: "1".into(),
        places: vec![
            PlaceNameRequest {
                province_id: 17,
                hour: 1
            };
            5000
        ],
        states: vec![
            StateNameRequest {
                state_id: 64,
                hour: 1,
            };
            5000
        ],
    };
    assert!(PlaceNames::validate(&params).is_ok());
    params.places.push(PlaceNameRequest {
        province_id: 18,
        hour: 2,
    });
    params.states.push(StateNameRequest {
        state_id: 65,
        hour: 2,
    });
    assert!(PlaceNames::validate(&params).is_ok());
}

fn resolved(
    frame: &DecodedFrame,
    province: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    regions: Result<&BTreeMap<u32, Result<String, String>>, &str>,
) -> (Option<String>, Option<String>) {
    let text = ScriptedText::new(frame, &cat.localisation, cat.scripted_content().unwrap(), 0);
    name_at(frame, &text, province, cat, names, regions, false)
}
fn name(
    frame: &DecodedFrame,
    province: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
) -> Result<String, String> {
    let (name, missing) = resolved(frame, province, cat, names, Ok(&BTreeMap::new()));
    name.ok_or_else(|| missing.unwrap())
}
#[test]
fn place_names_primary_name_needs_no_state_lookup_and_names_sea_regions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (cat, _folder) = catalogue();
    let mut f = frame(1);
    let (name, missing) = resolved(
        &f,
        17,
        &cat,
        Err("state templates unavailable"),
        Ok(&BTreeMap::new()),
    );
    assert_eq!(name.as_deref(), Some("Original town"));
    assert!(missing.is_none());
    set::<tables::Provinces>(
        &mut f,
        vec![ProvinceRow {
            id: 60000,
            ..Default::default()
        }],
    );
    let regions = BTreeMap::from([(60000, Ok("Mod sea region".into()))]);
    let named = resolved(&f, 60000, &cat, Err("irrelevant state names"), Ok(&regions));
    assert_eq!(named.0.as_deref(), Some("Mod sea region"));
    let regions = BTreeMap::from([(60000, Ok(String::new()))]);
    let empty = resolved(&f, 60000, &cat, Err("irrelevant state names"), Ok(&regions));
    assert_eq!(empty.0.as_deref(), Some(""));
    assert_eq!(empty.1, named.1);
    let regions = BTreeMap::from([(60000, Err("region unavailable".into()))]);
    let unavailable = resolved(&f, 60000, &cat, Err("irrelevant state names"), Ok(&regions));
    assert!(unavailable.0.is_none());
    assert!(unavailable.1.as_ref().is_some_and(|reason| {
        reason.contains("region unavailable") && reason.contains(named.1.as_deref().unwrap())
    }));
}

#[test]
fn land_place_names_try_the_state_after_empty_primary_and_never_use_a_region() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_cat, folder) = catalogue();
    let path = folder.0.join("localisation/mod_l_english.yml");
    let mut localisation = std::fs::read_to_string(&path).unwrap();
    localisation.push_str(" EMPTY:0 \"\"\n");
    std::fs::write(path, localisation).unwrap();
    let mut f = frame(1);
    let mut provinces = f.complete::<tables::Provinces>().unwrap().to_vec();
    provinces.extend([
        ProvinceRow {
            id: 60000,
            ..Default::default()
        },
        ProvinceRow {
            id: 60001,
            state: 99999,
            ..Default::default()
        },
    ]);
    set::<tables::Provinces>(&mut f, provinces);
    set::<tables::ProvinceNames>(
        &mut f,
        vec![ProvinceNameRow {
            province: 17,
            name: b"EMPTY".to_vec(),
        }],
    );
    let (source, context) = content_source(vec![f], &folder);
    let cat = catalogue::catalogue(&source, context.content.as_ref().unwrap()).unwrap();
    let names = BTreeMap::from([(64, Ok("Mod state".into()))]);
    let regions = BTreeMap::from([(60000, Ok("Sea region".into()))]);
    let mut params = PlaceNamesParams {
        branch_id: "1".into(),
        land: true,
        places: [17, 60000, 60001]
            .map(|province_id| PlaceNameRequest {
                province_id,
                hour: 1,
            })
            .to_vec(),
        states: Vec::new(),
    };
    let read = |params: &PlaceNamesParams| {
        read_names(
            source.replay_ref(),
            1,
            params,
            &cat,
            Ok(&names),
            Ok(&regions),
            &AtomicBool::new(false),
        )
        .unwrap()
        .0
    };
    let land = read(&params);
    assert_eq!(land[0].name.as_deref(), Some("Mod state"));
    assert!(
        land[1..]
            .iter()
            .all(|place| place.name.is_none() && place.missing.is_some())
    );
    params.land = false;
    let native = read(&params);
    assert_eq!(native[0].name.as_deref(), Some(""));
    assert!(native[0].missing.is_none());
    assert_eq!(native[1].name.as_deref(), Some("Sea region"));
}

#[test]
fn place_names_unresolvable_names_keep_ids_out_of_display_text() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (cat, _folder) = catalogue();
    let mut f = frame(1);
    set::<tables::Provinces>(
        &mut f,
        vec![ProvinceRow {
            id: 60000,
            state: 64,
            ..Default::default()
        }],
    );
    let names = BTreeMap::new();
    let (name, missing) = resolved(&f, 60000, &cat, Ok(&names), Ok(&BTreeMap::new()));
    assert!(name.is_none());
    assert!(missing.unwrap().contains("64"));
    let (name, missing) = resolved(&f, 99999, &cat, Ok(&names), Ok(&BTreeMap::new()));
    assert!(name.is_none());
    assert!(missing.unwrap().contains("99999"));
}

#[test]
fn selected_place_reads_keep_the_reserved_original_tag_operand() {
    let (cat, _folder) = catalogue();
    let mut f = frame(1);
    let positive_province = f.complete::<tables::Provinces>().unwrap()[0].clone();
    set::<tables::Provinces>(
        &mut f,
        vec![
            positive_province,
            ProvinceRow {
                id: 60000,
                ..Default::default()
            },
        ],
    );
    set::<tables::ReservedCountryNaming>(
        &mut f,
        vec![replay_data::rows::ReservedCountryNamingRow {
            tag: b"ZZ0".to_vec(),
            original_tag_reference: replay_data::rows::Recorded::Value(Some(1)),
            original_tag: replay_data::rows::Recorded::Value(Some(b"OR0".to_vec())),
            ..Default::default()
        }],
    );
    let replay = crate::test_fixtures::replay(vec![f]);
    let params = PlaceNamesParams {
        land: false,
        branch_id: "1".into(),
        places: vec![
            PlaceNameRequest {
                province_id: 17,
                hour: 1,
            },
            PlaceNameRequest {
                province_id: 60000,
                hour: 1,
            },
        ],
        states: Vec::new(),
    };
    let regions = BTreeMap::from([(60000, Ok("Fallback sea region".into()))]);
    let (places, states) = read_names(
        &replay,
        1,
        &params,
        &cat,
        Err("irrelevant state templates"),
        Ok(&regions),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(states.is_empty());
    assert_eq!(places[0].name.as_deref(), Some("Original town"));
    assert_eq!(places[1].name.as_deref(), Some("Reserved town"));
    assert!(places.iter().all(|place| place.missing.is_none()));
}
