use super::*;
use std::collections::HashMap;

fn names() -> Names {
    Names { slots: HashMap::from([(4, vec![("GER".to_owned(), 0, 500)]), (9, vec![("SOV".to_owned(), 100, 500)])]) }
}

fn cells(values: &[(usize, Cell)]) -> Vec<Cell> {
    let mut cells = vec![Cell::Unrecorded; 26];
    for (column, value) in values {
        cells[*column] = value.clone();
    }
    cells
}

fn shown(mapped: &Mapped, name: &str) -> LogCell {
    mapped.cells[COLUMNS.iter().position(|column| *column == name).unwrap()].clone()
}

fn key(kind: u8, hour: i32, division: u64) -> u128 {
    u128::from(kind) << 96 | u128::from(hour as u32) << 64 | u128::from(division)
}

#[test]
fn a_removal_is_named_by_its_cause_else_its_disposition_and_a_sighting_is_first_observed() {
    assert_eq!(status(DivisionEventRow::CREATED, None, None), "first_observed");
    assert_eq!(status(DivisionEventRow::LOST, Some(0), Some(13)), "division_destroyed");
    assert_eq!(status(DivisionEventRow::TRANSFER, Some(2), None), "merged");
    assert_eq!(status(DivisionEventRow::TRANSFER, Some(5), None), "exile_transferred");
    assert_eq!(status(DivisionEventRow::LOST, Some(9), None), "unknown");
}

#[test]
fn a_covered_death_is_exact_at_its_province_with_its_killer_and_a_sighting_is_bounded_by_the_last_census() {
    let death = cells(&[
        (column::DISPOSITION, Cell::Number(0)),
        (column::CAUSE, Cell::Number(13)),
        (column::SLOT, Cell::Number(4)),
        (column::NATIVE_HOUR, Cell::Number(205)),
        (column::EXACT, Cell::Number(1)),
        (column::AFTER, Cell::Missing),
        (column::PROVINCE, Cell::Number(77)),
        (column::PLACE_HOUR, Cell::Number(205)),
        (column::KILLER_SLOT, Cell::Number(9)),
        (column::KILLER_DIVISION, Cell::Number(12_345)),
        (column::TEMPLATE_ID, Cell::Number(31)),
        (column::MATCHED_DEATHS, Cell::Number(1)),
        (column::TEMPLATE_LITERAL, Cell::Text("1. Panzer".into())),
        (column::TEMPLATE_COUNTRY, Cell::Number(4)),
    ]);
    let lost = map(206, key(DivisionEventRow::LOST, 206, 9 << 32 | 3), &death, &names());
    assert_eq!(shown(&lost, "division"), LogCell::Text { value: (9u64 << 32 | 3).to_string() });
    assert_eq!(shown(&lost, "status"), LogCell::Text { value: "division_destroyed".into() });
    assert_eq!(shown(&lost, "hour"), LogCell::Number { value: "205".into() });
    assert_eq!(shown(&lost, "by"), LogCell::Number { value: "0".into() });
    assert_eq!(shown(&lost, "tag"), LogCell::Text { value: "GER".into() });
    assert_eq!(shown(&lost, "killerTag"), LogCell::Text { value: "SOV".into() });
    assert_eq!(shown(&lost, "killerDivision"), LogCell::Text { value: "12345".into() });
    assert_eq!(shown(&lost, "placeAtEvent"), LogCell::Number { value: "1".into() });
    assert_eq!(shown(&lost, "template"), LogCell::Text { value: "31".into() });
    assert_eq!(shown(&lost, "afterHour"), LogCell::Missing);
    assert_eq!(lost.template, Some((b"1. Panzer".to_vec(), None, 4)));
    assert!(!lost.ambiguous);
    assert_eq!(shown(&lost, "nameMissing"), LogCell::Text { value: "Division name not recorded at this observation".into() });

    let sighting = cells(&[(column::SLOT, Cell::Number(i64::from(TagHolderRow::NONE))), (column::NATIVE_HOUR, Cell::Number(90)), (column::EXACT, Cell::Number(0)), (column::AFTER, Cell::Number(80))]);
    let created = map(90, key(DivisionEventRow::CREATED, 90, 5 << 32 | 1), &sighting, &names());
    assert_eq!(shown(&created, "by"), LogCell::Number { value: "1".into() });
    assert_eq!(shown(&created, "afterHour"), LogCell::Number { value: "80".into() });
    assert_eq!(shown(&created, "tag"), LogCell::Missing, "an unresolved owner names no country, never a guess");
    assert_eq!(shown(&created, "placeAtEvent"), LogCell::Number { value: "1".into() });
    assert_eq!(shown(&created, "killerTag"), LogCell::Missing);

    let unmatched = cells(&[(column::CAUSE, Cell::Number(13)), (column::EXACT, Cell::Number(0)), (column::MATCHED_DEATHS, Cell::Number(2)), (column::KILLER_SLOT, Cell::Missing)]);
    let ambiguous = map(300, key(DivisionEventRow::LOST, 300, 7 << 32 | 1), &unmatched, &names());
    assert!(ambiguous.ambiguous);
    assert_eq!(shown(&ambiguous, "placeAtEvent"), LogCell::Number { value: "0".into() }, "a death not dated is placed where the division was last seen");
    assert_eq!(shown(&ambiguous, "killerTag"), LogCell::Missing);
}

fn templates_frame(hour: i32) -> replay_frame::frame::DecodedFrame {
    use replay_data::rows::{DivisionEquipmentDeletionRow, DivisionRow, DivisionTemplateIconRow, PacketHour, TableRows, TemplateDisplayName, TemplateNameValue};
    use replay_frame::frame::TableView;
    let template = |id: u64, literal: &[u8]| {
        let name = TemplateDisplayName { sample_hour: PacketHour::Host, value: TemplateNameValue::Changed { literal: literal.to_vec(), fallback_key: None } };
        DivisionTemplateIconRow { template: id << 32 | 3, country: 9001, display_name: Recorded::Value(Some(name)), ..Default::default() }
    };
    let division = |id: u64, template: u64| DivisionRow { id: id << 32 | 9, roster_owner: 9001, logical_owner: 9001, template: template << 32 | 3, hit_points: 100_000, ..Default::default() };
    let mut divisions = vec![division(1, 1)];
    if (1_100..1_500).contains(&hour) {
        divisions.push(division(2, 2));
    }
    if hour >= 1_900 {
        divisions.push(division(3, 1));
    }
    if hour >= 2_500 {
        divisions.push(division(4, 2));
    }
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divisions));
    *frame.tables[TableId::DivisionEquipmentDeletions.index()] = TableView::Complete(TableRows::DivisionEquipmentDeletions(Vec::<DivisionEquipmentDeletionRow>::new()));
    *frame.tables[TableId::DivisionTemplateIcons.index()] = TableView::Complete(TableRows::DivisionTemplateIcons(vec![template(1, b"1. Infanterie"), template(2, b"2. Panzer")]));
    frame
}

#[test]
fn a_runs_templates_are_named_by_their_first_row_in_the_log() {
    let frames = (1_000..=2_600).map(templates_frame).collect();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), dashboard_base::test_fixtures::CAMPAIGN.into(), &[dashboard_base::test_fixtures::records(1, frames)]).unwrap();
    let seen = run_templates(replay.recording(), 0).unwrap();
    let named = |id: u32, hour: i32, literal: &[u8]| SeenTemplate { id, hour, literal: literal.to_vec(), key: None, country: Some(9001) };
    assert_eq!(seen, vec![named(1, 1_900, b"1. Infanterie"), named(2, 1_100, b"2. Panzer")]);
}

#[test]
fn hours_not_read_are_the_stretch_less_its_read_runs() {
    assert_eq!(unread(&[(10, 19), (25, 30)], 5, 40), vec![(5, 9), (20, 24), (31, 40)]);
    assert_eq!(unread(&[(0, 50)], 5, 40), Vec::<(i32, i32)>::new());
    assert_eq!(unread(&[], 5, 6), vec![(5, 6)]);
}

#[test]
fn division_log_tail_and_dated_panel_match_page_and_count_partial_tail_bin() {
    use replay::campaign::Replay;
    let frames = (1_000..=1_905).map(templates_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), dashboard_base::test_fixtures::CAMPAIGN.into(), &[dashboard_base::test_fixtures::records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let source = Source::from_replay(
        dashboard_base::SourceKey { source_id: "div-tail".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = LoadContext { content: None };
    let facets = vec![FacetFilter { name: "kind".into(), values: vec![0, 1, 2], ..Default::default() }];
    let page = DivisionLog::load(
        &source,
        &DivisionLogParams { branch_id: "1".into(), facets: facets.clone(), from: None, to: 1_901.0, cursor: None, page_size: 64 },
        &load,
        &AtomicBool::new(false),
    )
    .unwrap();
    let count_facets = vec![
        LogFacetCountAsk { key: "created".into(), facets: vec![FacetFilter { name: "kind".into(), values: vec![0], ..Default::default() }] },
        LogFacetCountAsk { key: "vanished".into(), facets: vec![FacetFilter { name: "kind".into(), values: vec![3], ..Default::default() }] },
    ];
    let tail = DivisionLogTail::load(
        &source,
        &DivisionLogTailParams { branch_id: "1".into(), facets: facets.clone(), from: None, hour: Some(1_901), limit: 64, count_facets: count_facets.clone() },
        &load,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(tail.total, page.total);
    assert_eq!(tail.columns, page.columns);
    assert_eq!(tail.rows, page.rows.into_iter().rev().collect::<Vec<_>>());
    assert_eq!(tail.facet_totals[0].total, tail.total);
    assert!(tail.rows.iter().all(|row| matches!(row.cells[PLACE_NAME], LogCell::Missing) && matches!(row.cells[PLACE_MISSING], LogCell::Text { .. })));

    let asked = serde_json::json!({
        "panel": "division-log-tail",
        "branchId": "1",
        "facets": facets,
        "limit": 64,
        "countFacets": count_facets
    });
    let panel = dated_panel(&source, &load, &asked).unwrap();
    let exact = panel.exact().unwrap();
    for hour in [1_100, 1_901, 1_500, 1_901, 1_000] {
        let stepped = (exact.advance)(hour, true, &AtomicBool::new(false), None);
        let cold = (exact.cold)(hour, &AtomicBool::new(false));
        assert_eq!(*stepped.value, cold, "stepped vs cold at {hour}");
    }

    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let links = links_by_run(&source, &query).unwrap();
    let art = Content::load(&source, &load);
    let params = DivisionLogTailParams { branch_id: "1".into(), facets, from: None, hour: None, limit: 64, count_facets };
    let cache = source.state::<PlaceCache>();
    let log_caches = dashboard_base::queries::log_caches(&source);
    for hour in [1_901, 1_100, 1_500, 1_901, 1_100] {
        let cached = load_tail_at(&query, &links, &art, Err("content unavailable"), Some(&cache), Some(&log_caches), &params, hour, &AtomicBool::new(false)).unwrap();
        let uncached = load_tail_at(&query, &links, &art, Err("content unavailable"), None, None, &params, hour, &AtomicBool::new(false)).unwrap();
        assert_eq!(cached, uncached, "cached vs uncached at {hour}");
    }
}

#[test]
fn place_cache_resolves_names_hits_evicts_and_forgets_matching_uncached() {
    use replay::campaign::Replay;
    use replay_data::rows::{CountryAppearanceRow, DivisionRow, ProvinceNameRow, ProvinceRow, ReservedCountryNamingRow, StateRow, TableRows};
    use replay_frame::frame::TableView;

    let (cat, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        ("common/defines/00_defines.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NBuildings = { MAX_BUILDING_LEVELS = 15 } }"),
        ("map/default.map", "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp"),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        ("history/states/mod.txt", "state={id=64 name=MOD_STATE}"),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n MOD_STATE:0 \"Mod state\"\n ORI_VICTORY_POINTS_17:0 \"Original town\"\n RENAME_EARLY:0 \"Early town\"\n RENAME_LATE:0 \"Late town\"\n",
        ),
    ]);
    let places = PlaceContent::from_catalogue(Arc::new(cat));
    let frames = (100..=108)
        .map(|hour| {
            let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
            *frame.tables[TableId::CountryAppearance.index()] = TableView::Complete(TableRows::CountryAppearance(vec![
                CountryAppearanceRow { country: 9001, original_tag_reference: Recorded::Value(Some(1)), original_tag: Recorded::Value(Some(b"ORI".to_vec())), ..Default::default() },
            ]));
            *frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Complete(TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow::default()]));
            *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
                [17, 18, 19].map(|id| ProvinceRow { id, state: 64, owner: 9001, controller: 9001, ..Default::default() }).to_vec(),
            ));
            let rename = if hour < 105 { b"RENAME_EARLY".to_vec() } else { b"RENAME_LATE".to_vec() };
            *frame.tables[TableId::ProvinceNames.index()] = TableView::Complete(TableRows::ProvinceNames(vec![ProvinceNameRow { province: 18, name: rename }]));
            *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow { id: 64, owner: 9001, controller: 9001, ..Default::default() }]));
            let mut divs = vec![DivisionRow { id: 1u64 << 32 | 1, roster_owner: 9001, logical_owner: 9001, province: 17, hit_points: 100_000, ..Default::default() }];
            if hour >= 102 {
                divs.push(DivisionRow { id: 2u64 << 32 | 1, roster_owner: 9001, logical_owner: 9001, province: 18, hit_points: 100_000, ..Default::default() });
            }
            if hour >= 104 {
                divs.push(DivisionRow { id: 3u64 << 32 | 1, roster_owner: 9001, logical_owner: 9001, province: 19, hit_points: 100_000, ..Default::default() });
            }
            if hour >= 106 {
                divs.push(DivisionRow { id: 4u64 << 32 | 1, roster_owner: 9001, logical_owner: 9001, province: 18, hit_points: 100_000, ..Default::default() });
            }
            *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divs));
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), dashboard_base::test_fixtures::CAMPAIGN.into(), &[dashboard_base::test_fixtures::records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let source = Source::from_replay(
        dashboard_base::SourceKey { source_id: "div-places".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let links = links_by_run(&source, &query).unwrap();
    let art = Content::load(&source, &LoadContext { content: None });
    let facets = vec![FacetFilter { name: "kind".into(), values: vec![0, 1, 2], ..Default::default() }];
    let params = DivisionLogTailParams { branch_id: "1".into(), facets, from: None, hour: None, limit: 64, count_facets: Vec::new() };
    let source_cache = source.state::<PlaceCache>();

    let first = load_tail_at(&query, &links, &art, Ok(&places), Some(&source_cache), None, &params, 108, &AtomicBool::new(false)).unwrap();
    let uncached = load_tail_at(&query, &links, &art, Ok(&places), None, None, &params, 108, &AtomicBool::new(false)).unwrap();
    assert_eq!(first, uncached);
    let place_names: Vec<Option<&str>> = first
        .rows
        .iter()
        .map(|row| match &row.cells[PLACE_NAME] {
            LogCell::Text { value } => Some(value.as_str()),
            _ => None,
        })
        .collect();
    assert!(place_names.contains(&Some("Early town")), "resolved early province rename: {place_names:?}");
    assert!(place_names.contains(&Some("Late town")), "resolved late province rename: {place_names:?}");
    assert!(place_names.contains(&Some("Mod state")), "resolved fallback state name: {place_names:?}");
    assert!(PlaceCache::len(&source_cache) >= 3, "cache populated on resolution");

    let warm = load_tail_at(&query, &links, &art, Ok(&places), Some(&source_cache), None, &params, 108, &AtomicBool::new(false)).unwrap();
    assert_eq!(warm, uncached, "warm cache hit matches uncached");

    let bounded = Mutex::new(PlaceCache::with_capacity(2));
    for hour in [102, 104, 106, 108, 102] {
        let got = load_tail_at(&query, &links, &art, Ok(&places), Some(&bounded), None, &params, hour, &AtomicBool::new(false)).unwrap();
        let want = load_tail_at(&query, &links, &art, Ok(&places), None, None, &params, hour, &AtomicBool::new(false)).unwrap();
        assert_eq!(got, want, "bounded cache with eviction matches uncached at {hour}");
        assert!(PlaceCache::len(&bounded) <= 2, "bounded cache never exceeds capacity");
    }

    forget(&source);
    assert_eq!(PlaceCache::len(&source_cache), 0, "source invalidation clears PlaceCache");
    let after_forget = load_tail_at(&query, &links, &art, Ok(&places), Some(&source_cache), None, &params, 108, &AtomicBool::new(false)).unwrap();
    assert_eq!(after_forget, uncached, "re-resolution after forget matches uncached");
}
