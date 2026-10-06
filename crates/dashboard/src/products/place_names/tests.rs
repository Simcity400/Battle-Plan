use super::*;
use replay::{
    frame::{Metrics, TableView},
    rows::{
        CampaignRow, CountryAppearanceRow, CountryLinkRow, CountryRow, ProvinceNameRow,
        ProvinceRow, StateRow, TableRows, TableSchema,
    },
};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|at| {
            if NAME_TABLES.contains(&TableId::ALL[at]) {
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
                canonical_slot: id - 1,
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
                canonical_slot: id - 1,
            })
            .collect(),
    );
    set::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 15_000,
            name_tag: b"mod_original".to_vec(),
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
        (
            "common/defines/00_defines.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n MOD_STATE:0 \"Mod state\"\n mod_original_VICTORY_POINTS_17:0 \"Original town\"\n mod_tag_1_VICTORY_POINTS_17:0 \"Controller town\"\n VICTORY_POINTS_17:0 \"Generic town\"\n RENAME:0 \"Renamed town\"\n",
        ),
    ])
}

#[test]
fn place_names_use_the_recorded_owner_and_stored_name_precedence_at_5000_countries() {
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
    let places = read_places(
        &replay,
        1,
        &requests,
        &cat,
        Ok(&names),
        Ok(&BTreeMap::new()),
        &AtomicBool::new(false),
    )
    .unwrap();
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
        branch_id: "1".into(),
        places: vec![
            PlaceNameRequest {
                province_id: 17,
                hour: 1
            };
            5000
        ],
    };
    assert!(PlaceNames::validate(&params).is_ok());
    params.places.push(PlaceNameRequest {
        province_id: 18,
        hour: 2,
    });
    assert!(PlaceNames::validate(&params).is_ok());
    assert!(
        serde_json::from_value::<PlaceNamesParams>(
            serde_json::json!({ "branchId": "1", "places": [], "ignored": true })
        )
        .is_err()
    );
    let reply = PlaceNamesReply {
        places: vec![named_place(
            &params.places[0],
            (Some("Exact place".into()), None),
        )],
        warnings: Vec::new(),
    };
    let bytes = serde_json::to_vec(&reply).unwrap().len();
    assert_eq!(json_len(&reply, bytes), Some(bytes));
    assert!(json_len(&reply, bytes - 1).is_none());
}

fn resolved(
    frame: &DecodedFrame,
    province: u32,
    cat: &Catalogue,
    names: Result<&StateNames, &str>,
    regions: Result<&BTreeMap<u32, Result<String, String>>, &str>,
) -> (Option<String>, Option<String>) {
    let text = ScriptedText::new(frame, &cat.localisation, cat.scripted_content().unwrap(), 0);
    name_at(frame, &text, province, cat, names, regions)
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
    assert_eq!(
        resolved(&f, 60000, &cat, Err("irrelevant state names"), Ok(&regions))
            .0
            .as_deref(),
        Some("Mod sea region")
    );
    let regions = BTreeMap::from([(60000, Ok(String::new()))]);
    assert!(
        resolved(&f, 60000, &cat, Err("irrelevant state names"), Ok(&regions))
            .0
            .is_none()
    );
}

#[test]
fn place_names_unresolvable_names_keep_ids_out_of_display_text() {
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
    let names = BTreeMap::from([(64, Ok(String::new()))]);
    let (name, missing) = resolved(&f, 60000, &cat, Ok(&names), Ok(&BTreeMap::new()));
    assert!(name.is_none());
    assert!(missing.unwrap().contains("60000"));
    let (name, missing) = resolved(&f, 99999, &cat, Ok(&names), Ok(&BTreeMap::new()));
    assert!(name.is_none());
    assert!(missing.unwrap().contains("99999"));
}
