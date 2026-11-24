use dashboard_world::products::equipment_names as names;
use super::{
    catalogue::{EquipmentDesigns, IndustrialOrganisations, ProductionLicences},
    lines::{ProductionLinesParams, lines_reply},
    live::ProductionLinesAt,
    *,
};
use dashboard_base::{Source, products::{LoadContext, at_hour::entry}, test_fixtures::{CAMPAIGN, records, source}};
use replay::campaign::Replay;
use replay_data::rows::{
    CountryLinkRow, CountryRow, IndustrialOrganisationDetailsRow, IndustrialOrganisationRow, ProductionLicenseRecordRow, ProductionLineOutputRow, ProductionLineRow, Recorded, TableRows,
    VariantDefinitionRow,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

const FIRST: i32 = 1_000;
const LAST: i32 = 1_300;
const UNREAD: (i32, i32) = (1_240, 1_250);

fn line(handle: u64, country: u32, variant: u64, active: i32, priority: i32) -> ProductionLineRow {
    ProductionLineRow { handle, country, kind: 0, variant, mio: 0, active_factories: active, requested_factories: 3, cost_raw: 700_000, speed_raw: 200_000, priority, amount: -1, max_factories: 10, efficiency_raw: 1_638_400, ..Default::default() }
}

fn output(line: u64, country: u32, variant: u64, units: i64, ended: bool) -> ProductionLineOutputRow {
    ProductionLineOutputRow { line, ordinal: 0, country, variant, mio: 0, units, since_hour: FIRST, observed_since_creation: line != 1, ended, exact: true, spent_raw: Recorded::Value(None), purpose: Recorded::Value(1), ..Default::default() }
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) = &mut *frame.tables[TableId::Countries.index()] {
        rows.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..CountryRow::default() });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *frame.tables[TableId::CountryLinks.index()] {
        rows.push(CountryLinkRow { id: 9002, canonical_slot: 8 });
    }
    let mut lines = vec![line(1, 11, if hour < 1_100 { 100 } else { 101 }, if hour < 1_150 { 2 } else { 3 }, 1), line(3, 9002, 300, 1, 0)];
    let mut outputs = vec![output(1, 11, if hour < 1_100 { 100 } else { 101 }, i64::from((hour - FIRST) / 24), false), output(3, 9002, 300, 0, false)];
    if (1_050..1_200).contains(&hour) {
        lines.push(line(2, 9001, 200, 1, 0));
        outputs.push(output(2, 9001, 200, i64::from((hour - 1_050) / 24), false));
    }
    if (1_200..1_224).contains(&hour) {
        outputs.push(output(2, 9001, 200, 7, true));
    }
    lines.sort_by_key(|row| row.handle);
    outputs.sort_by_key(|row| (row.line, row.ordinal));
    *frame.tables[TableId::ProductionLines.index()] = if (UNREAD.0..=UNREAD.1).contains(&hour) {
        TableView::Unavailable { reason: "lines unread in the fixture".into() }
    } else {
        TableView::Complete(TableRows::ProductionLines(lines))
    };
    *frame.tables[TableId::ProductionLineOutput.index()] = TableView::Complete(TableRows::ProductionLineOutput(outputs));
    let mut licences = vec![ProductionLicenseRecordRow { owner: 11, ordinal: 0, giver: 9002, lended_cic: 2, required_cic: 1, start_date: FIRST, equipment: vec![300], ..Default::default() }];
    if hour >= 1_200 {
        licences.push(ProductionLicenseRecordRow { owner: 9002, ordinal: 0, giver: 77, lended_cic: 0, required_cic: 0, start_date: 1_200, equipment: vec![100], ..Default::default() });
    }
    *frame.tables[TableId::ProductionLicenseRecords.index()] = TableView::Complete(TableRows::ProductionLicenseRecords(licences));
    *frame.tables[TableId::IndustrialOrganisations.index()] = TableView::Complete(TableRows::IndustrialOrganisations(vec![
        IndustrialOrganisationRow { organisation: 40, country: 11, owner: 9001, ..Default::default() },
        IndustrialOrganisationRow { organisation: 41, country: 9002, owner: 9002, ..Default::default() },
    ]));
    *frame.tables[TableId::IndustrialOrganisationDetails.index()] = if hour < 1_100 {
        TableView::Unavailable { reason: "details unread in the fixture".into() }
    } else {
        let mut details = IndustrialOrganisationDetailsRow { organisation: 40, country: 11, unlocked: vec![1, 2], ..Default::default() };
        details.state.size = 3;
        details.state.funds_raw = 250_000;
        details.state.policy = 0x4c17;
        TableView::Complete(TableRows::IndustrialOrganisationDetails(vec![details]))
    };
    let mut designs = vec![VariantDefinitionRow { id: 500, creator_link: 9001, obsolete: u8::from(hour >= 1_200), ..Default::default() }, VariantDefinitionRow { id: 501, creator_link: 9002, ..Default::default() }];
    if hour >= 1_100 {
        designs.push(VariantDefinitionRow { id: 502, creator_link: 11, ..Default::default() });
    }
    *frame.tables[TableId::VariantDefinitions.index()] = TableView::Complete(TableRows::VariantDefinitions(designs));
    frame
}

fn recording() -> Source {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "production".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn moved_frame(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour);
    if hour >= 1_150
        && let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *frame.tables[TableId::CountryLinks.index()]
    {
        for row in rows.iter_mut().filter(|row| row.id == 11) {
            row.canonical_slot = 8;
        }
    }
    frame
}

fn moved_recording() -> Source {
    let frames = (FIRST..=LAST).map(moved_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "production-moved".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

#[test]
fn a_scoped_reply_holds_only_what_its_identity_owned_inside_the_scope() {
    let source = moved_recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let params = ProductionLinesParams { branch_id: "1".into(), country: "GER".into(), within: Some(vec![dashboard_base::span::HourSpan { first_hour: FIRST, last_hour: 1_205 }]) };
    let reply = lines_reply(&source, &query, &params, &no_content(), &AtomicBool::new(false)).unwrap();
    let moved = reply.lines.iter().find(|line| line.key == "1#1").expect("raw country 11's line is GER's while 11 links to GER's slot");
    assert_eq!(moved.steps, [step(1_000, 1_099, true, "100", 2), step(1_100, 1_149, false, "101", 2)], "the line leaves GER at the hour its raw country's link moves");
    assert_eq!((moved.ended_by, moved.final_output.is_none()), (None, true), "a line whose country moves on did not end for GER, so it takes none of the next holder's output");
    let ended = reply.lines.iter().find(|line| line.key == "2#1").and_then(|line| line.final_output.as_ref()).expect("GER's ended line keeps its final count");
    assert!(ended.last_hour <= 1_205, "ended output is read only inside the scope, not through {}", ended.last_hour);
}

fn no_content() -> names::Context {
    Err("no content in the fixture".to_owned())
}

fn step(first: i32, last: i32, starts: bool, variant: &str, active: i32) -> lines::LineStep {
    lines::LineStep { first_hour: first, last_hour: last, starts_segment: starts, variant: variant.into(), active_factories: active, requested_factories: 3 }
}

#[test]
fn a_countrys_lines_follow_its_holder_through_aliases_and_end_with_their_published_count() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let params = ProductionLinesParams { branch_id: "1".into(), country: "GER".into(), within: None };
    let reply = lines_reply(&source, &query, &params, &no_content(), &AtomicBool::new(false)).unwrap();
    let keys: Vec<&str> = reply.lines.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(keys, ["1#1", "2#1"], "ITA's line is not GER's; a line the alias produced is");
    let first = &reply.lines[0];
    assert!(first.before_recording);
    assert_eq!(first.ended_by, None, "hours the lines were not read neither end a line nor start another appearance");
    assert_eq!(first.kind, Some(LineKind::Military));
    assert_eq!(
        first.steps,
        [step(1_000, 1_099, true, "100", 2), step(1_100, 1_149, false, "101", 2), step(1_150, 1_239, false, "101", 3), step(1_251, 1_300, true, "101", 3)]
    );
    let second = &reply.lines[1];
    assert!(!second.before_recording);
    assert_eq!(second.steps, [step(1_050, 1_199, true, "200", 1)]);
    assert_eq!(second.ended_by, Some(1_200));
    let ended = second.final_output.as_ref().expect("its ended rows are its final count");
    assert_eq!((ended.first_hour, ended.last_hour, ended.whole_life), (1_200, 1_223, true));
    assert_eq!(ended.parts, [OutputPart { variant: "200".into(), mio: "0".into(), units: "7".into(), exact: true }]);
    assert_eq!(reply.line_hours, [dashboard_base::span::HourSpan { first_hour: FIRST, last_hour: LAST }]);
    let variants: Vec<&str> = reply.variants.iter().map(|variant| variant.id.as_str()).collect();
    assert_eq!(variants, ["100", "101", "200"]);
    assert!(reply.variants.iter().all(|variant| variant.name.is_none() && variant.name_unknown.is_some()), "a name not known says why, never blank");
    let held = ProductionLinesParams { within: Some(vec![dashboard_base::span::HourSpan { first_hour: 1_000, last_hour: 1_099 }]), ..params };
    let within = lines_reply(&source, &query, &held, &no_content(), &AtomicBool::new(false)).unwrap();
    assert_eq!(within.lines[0].steps, [step(1_000, 1_099, true, "100", 2)], "a held identity's lines are read only within its own runs");
    assert_eq!(within.lines[0].key, reply.lines[0].key, "a line keeps its key whatever scope asks for it");
    assert_eq!(within.line_hours, [dashboard_base::span::HourSpan { first_hour: 1_000, last_hour: 1_099 }], "the scoped reply's hours are the held identity's");
}

fn load() -> LoadContext {
    LoadContext { content: None }
}

fn asked(source: &Source, entry: dashboard_base::products::at_hour::Entry, request: Value) -> Value {
    let text = (entry.registered.load)(source, "lease", request, &load(), &AtomicBool::new(false)).unwrap();
    let mut reply: Value = serde_json::from_str(&text).unwrap();
    reply["data"].take()
}

fn cells(reply: &Value) -> std::collections::BTreeMap<(String, String), Result<f64, String>> {
    let live = &reply["live"];
    let flat: Vec<usize> = live["cells"].as_array().unwrap().iter().map(|item| item.as_u64().unwrap() as usize).collect();
    let mut out = std::collections::BTreeMap::new();
    let mut at = 0;
    while at < flat.len() {
        let (row, kind, count) = (flat[at], flat[at + 1], flat[at + 2]);
        let mut value: Result<f64, String> = Ok(0.0);
        for source in 0..count {
            let (group, key, column) = (flat[at + 3 + 3 * source], flat[at + 4 + 3 * source], flat[at + 5 + 3 * source]);
            let width = live["groups"][group]["columns"].as_array().unwrap().len();
            let held = &live["groups"][group]["values"][key * width + column];
            value = match (value, held.as_f64()) {
                (Ok(sum), Some(number)) => Ok(sum + number),
                (Err(why), _) => Err(why),
                (Ok(_), None) => Err(held.as_str().unwrap_or("no value").to_owned()),
            };
        }
        let scale = live["kinds"][kind]["scale"].as_f64().unwrap();
        out.insert((live["rows"][row].as_str().unwrap().to_owned(), live["kinds"][kind]["id"].as_str().unwrap().to_owned()), value.map(|sum| sum * scale));
        at += 3 + 3 * count;
    }
    out
}

fn cell(cells: &std::collections::BTreeMap<(String, String), Result<f64, String>>, row: &str, column: &str) -> Result<f64, String> {
    cells.get(&(row.to_owned(), column.to_owned())).cloned().unwrap_or_else(|| Err(format!("no cell ({row}, {column})")))
}

#[test]
fn the_live_table_lists_the_countrys_lines_in_queue_order_and_says_when_they_are_not_read() {
    let source = recording();
    let reply = asked(&source, entry::<ProductionLinesAt>(), json!({ "branchId": "1", "country": "GER", "hour": 1_120 }));
    let handles = |reply: &Value| reply["lines"].as_array().unwrap().iter().map(|line| line["handle"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(handles(&reply), ["2", "1"], "queue order; ITA's line is not GER's");
    assert!(reply["lines"][0]["output"]["parts"][0].get("units").is_none(), "units made move: they are a cell, not the reply's");
    let values = cells(&reply);
    assert_eq!(cell(&values, "2", "output"), Ok(2.0));
    assert_eq!(cell(&values, "1", "output"), Ok(5.0));
    assert_eq!((cell(&values, "1", "active"), cell(&values, "1", "requested")), (Ok(2.0), Ok(3.0)));
    assert_eq!((cell(&values, "1", "speed"), cell(&values, "1", "cost"), cell(&values, "1", "efficiency")), (Ok(200_000.0), Ok(700_000.0), Ok(1_638_400.0)));
    assert!(values.keys().all(|(row, _)| row == "1" || row == "2"), "no cell of ITA's line");
    assert!(reply["missing"].is_null());
    let by_factories = asked(&source, entry::<ProductionLinesAt>(), json!({ "branchId": "1", "country": "GER", "hour": 1_120, "order": "factories" }));
    assert_eq!(handles(&by_factories), ["1", "2"], "most factories first");
    let by_output = asked(&source, entry::<ProductionLinesAt>(), json!({ "branchId": "1", "country": "GER", "hour": 1_120, "order": "output" }));
    assert_eq!(handles(&by_output), ["2", "1"], "equal speeds keep the queue's order");
    let unread = asked(&source, entry::<ProductionLinesAt>(), json!({ "branchId": "1", "country": "GER", "hour": 1_245 }));
    assert!(unread["lines"].as_array().unwrap().is_empty());
    assert!(unread["missing"].as_str().is_some_and(|why| why.contains("not recorded")), "unread lines are missing, never none");
}

fn followed(source: &Source, entry: dashboard_base::products::at_hour::Entry, params: &Value, hour: i32) -> dashboard_follow::panel::PanelData {
    let mut panel = (entry.panel)(source, &load(), params).unwrap();
    let tables = panel.tables().to_vec();
    let replay::campaign::SeekResult::Frame(frame) = source.replay_ref().seek_selected(1, hour, Some(&tables)).unwrap() else { panic!("hour {hour} is recorded") };
    panel.data(&frame)
}

#[test]
fn a_request_reading_the_owners_rows_and_tokens_by_key_answers_as_the_panel_over_whole_tables() {
    let source = recording();
    let products = [
        (entry::<ProductionLinesAt>(), json!({ "branchId": "1", "country": "GER" })),
        (entry::<ProductionLinesAt>(), json!({ "branchId": "1", "country": "GER", "order": "factories" })),
        (entry::<IndustrialOrganisations>(), json!({ "branchId": "1", "country": "GER" })),
        (entry::<ProductionLicences>(), json!({ "branchId": "1" })),
    ];
    for (product, params) in products {
        let mut as_panel = params.clone();
        as_panel["panel"] = json!(product.registered.key);
        for hour in (FIRST..=LAST).step_by(7).chain([1_049, 1_050, 1_199, 1_200, UNREAD.0, UNREAD.1 + 1]) {
            let mut request = params.clone();
            request["hour"] = json!(hour);
            let mut reply = asked(&source, product, request);
            reply.as_object_mut().unwrap().remove("live");
            assert_eq!(followed(&source, product, &as_panel, hour), Ok(reply), "{} at hour {hour}", product.registered.key);
        }
    }
}

#[test]
fn licences_name_both_parties_by_their_countries_and_organisations_say_when_their_state_is_unread() {
    let source = recording();
    let reply = asked(&source, entry::<ProductionLicences>(), json!({ "branchId": "1", "hour": 1_210, "subject": "GER" }));
    let country = |at: &serde_json::Value| reply["countries"][at.as_u64().unwrap() as usize].as_str().map(str::to_owned);
    let incoming = reply["incoming"].as_array().unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!((country(&incoming[0]["from"]).as_deref(), incoming[0]["total"].as_str()), (Some("ITA"), Some("1")), "the alias is GER");
    assert!(reply["partial"].as_str().is_some(), "an unlinked party is left out and said so, not counted as another country");
    let world = asked(&source, entry::<ProductionLicences>(), json!({ "branchId": "1", "hour": 1_210 }));
    assert_eq!(world["links"].as_array().unwrap().len(), 1);
    let early = asked(&source, entry::<IndustrialOrganisations>(), json!({ "branchId": "1", "country": "GER", "hour": 1_050 }));
    let organisations = early["organisations"].as_array().unwrap();
    assert_eq!(organisations.len(), 1, "ITA's organisation is not GER's");
    assert!(organisations[0]["size"].is_null() && organisations[0]["detailsMissing"].as_str().is_some());
    let early_cells = cells(&early);
    assert!(cell(&early_cells, "40", "funds").is_err(), "funds not read are missing, never zero");
    let later = asked(&source, entry::<IndustrialOrganisations>(), json!({ "branchId": "1", "country": "GER", "hour": 1_150 }));
    let organisation = &later["organisations"][0];
    assert_eq!((organisation["size"].as_i64(), organisation["traits"].as_i64()), (Some(3), Some(2)));
    assert!(organisation.get("fundsRaw").is_none(), "funds move daily: they are a cell, not the reply's");
    let later_cells = cells(&later);
    assert_eq!((cell(&later_cells, "40", "funds"), cell(&later_cells, "40", "points")), (Ok(250_000.0), Ok(0.0)));
    assert!(organisation["policyToken"].is_null() && organisation["policyUnknown"].is_null(), "no policy selected is none, not unknown");
}

#[test]
fn a_countrys_designs_at_the_hour_are_its_creators_with_their_obsolete_flags_current_first() {
    let source = recording();
    let designs = |hour: i32| {
        let reply = asked(&source, entry::<EquipmentDesigns>(), json!({ "branchId": "1", "country": "GER", "hour": hour }));
        assert!(reply["designs"].as_array().unwrap().iter().all(|design| design["name"]["name"].is_null() && design["name"]["nameUnknown"].is_string()));
        reply["designs"].as_array().unwrap().iter().map(|design| (design["name"]["id"].as_str().unwrap().to_owned(), design["obsolete"].as_bool().unwrap())).collect::<Vec<_>>()
    };
    assert_eq!(designs(1_050), [("500".to_owned(), false)], "502 is not saved yet; ITA's 501 is not GER's");
    assert_eq!(designs(1_250), [("502".to_owned(), false), ("500".to_owned(), true)]);
}

#[test]
fn a_line_key_names_its_handle_and_a_positive_appearance() {
    assert_eq!(parse_key("7460358193208#1"), Some((7_460_358_193_208, 1)));
    assert_eq!(parse_key("12#0"), None);
    assert_eq!(parse_key("12"), None);
    assert_eq!(parse_key("x#1"), None);
    assert_eq!(weekly_units_raw(200_000, 700_000), Some(200_000));
    assert_eq!(weekly_units_raw(1, 0), None);
}

#[test]
fn artwork_queries_fetch_state_continent_tokens_appearance_refs_and_reserved_naming() {
    use crate::products::stockpile::artwork::{StockpileArtwork, StockpileArtworkParams};
    use dashboard_base::products::at_hour::{AtHour, Keys, Owner, ReadsAt};
    use dashboard_economy::products::economy::market::{MarketLendLease, MarketLendLeaseParams, MarketSection};
    use replay_data::rows::{
        CountryAppearanceRow, CountryMarketRow, LexicalTokenRow, PoliticsRow,
        ReservedCountryNamingRow, StateRow, tables,
    };

    assert!(names::TABLES.contains(&TableId::ReservedCountryNaming));
    assert!(names::WHOLE.contains(&TableId::ReservedCountryNaming));
    assert!(
        StockpileArtwork::tables(&StockpileArtworkParams {
            branch_id: "1".into(),
            country: "GER".into(),
            variants: Vec::new(),
        })
        .contains(&TableId::ReservedCountryNaming)
    );

    let mut frame = frame(FIRST);
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 9001,
            original_tag_reference: Recorded::Value(Some(41)),
            original_tag: Recorded::Value(Some(b"AAA".to_vec())),
            ..Default::default()
        }]));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                original_tag_reference: Recorded::Value(Some(41)),
                original_tag: Recorded::Value(Some(b"AAA".to_vec())),
                dynamic_tag_boundary: Recorded::Value(Some(50)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
        PoliticsRow {
            country: 9001,
            capital: 12,
            original_capital: 12,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 12,
            continent: Recorded::Value(Some(701)),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 601,
                literal: b"tanks_1".to_vec(),
            },
            LexicalTokenRow {
                token: 701,
                literal: b"europe".to_vec(),
            },
        ]));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 100,
            equipment_token: 601,
            creator_link: 11,
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryMarket.index()] =
        TableView::Complete(TableRows::CountryMarket(vec![CountryMarketRow {
            country: 9001,
            prices: vec![replay_data::rows::MarketPriceLevel {
                variant: 100,
                level: 1,
            }]
            .into(),
            ..Default::default()
        }]));
    for table in [TableId::PoliticalParties, TableId::Factions, TableId::FactionMembers] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }

    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![frame])])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let source = Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "artwork-reads".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let recording = source.replay_ref().recording();

    let live_params = live::ProductionLinesAtParams {
        branch_id: "1".into(),
        country: "GER".into(),
        order: None,
    };
    let live_ctx = ProductionLinesAt::context(&source, &load()).unwrap();
    let state_cols = ProductionLinesAt::projection(&live_params, TableId::States, &[], &live_ctx)
        .expect("States projection");
    assert!(
        !state_cols.is_empty(),
        "live production must project States.continent"
    );
    let app_cols =
        ProductionLinesAt::projection(&live_params, TableId::CountryAppearance, &[], &live_ctx)
            .expect("CountryAppearance projection");
    let expected_app = dashboard_base::products::at_hour::field_columns(
        TableId::CountryAppearance,
        &["original_tag", "original_tag_reference"],
    );
    assert!(
        expected_app.iter().all(|col| app_cols.contains(col)),
        "live production must project CountryAppearance original_tag and original_tag_reference"
    );

    let mut held = DecodedFrame {
        branch: 1,
        hour: FIRST,
        tables: std::array::from_fn(|_| replay_frame::frame::not_selected()),
        metrics: Default::default(),
    };
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let reads_at = ReadsAt {
        holders: &query.holders,
        run: 0,
        hour: FIRST,
        context: &live_ctx,
        since: None,
    };
    let owner = Some(Owner { id: 9001, slot: 7 });
    for step in 0..8 {
        let step_reads = ProductionLinesAt::reads_in(&live_params, owner, step, &held, &reads_at);
        if step_reads.is_empty() {
            break;
        }
        let projections: Vec<Option<Vec<usize>>> = step_reads
            .iter()
            .map(|read| ProductionLinesAt::projection(&live_params, read.table, &[], &live_ctx))
            .collect();
        let keyed: Vec<replay_lanes::at::Read<'_>> = step_reads
            .iter()
            .zip(&projections)
            .map(|(read, cols)| replay_lanes::at::Read {
                table: read.table,
                columns: cols.as_deref(),
                keys: match &read.keys {
                    Keys::All => replay_lanes::at::Keys::All,
                    Keys::These(keys) => replay_lanes::at::Keys::These(keys),
                    Keys::Range(low, high) => replay_lanes::at::Keys::Range(*low, *high),
                    Keys::Ranges(ranges) => replay_lanes::at::Keys::Ranges(ranges),
                },
            })
            .collect();
        let read = replay_lanes::at::frame(recording, 0, FIRST, &keyed).unwrap();
        for asked in &step_reads {
            held.tables[asked.table.index()] = read.tables[asked.table.index()].clone();
        }
    }
    let lexical = held.complete::<tables::LexicalTokens>().unwrap();
    assert!(
        lexical.iter().any(|row| row.token == 701 && row.literal == b"europe"),
        "live production must fetch distinct capital continent lexical token: {lexical:?}"
    );
    let states = held.complete::<tables::States>().unwrap();
    assert_eq!(states[0].continent, Recorded::Value(Some(701)));
    let appearance = held.complete::<tables::CountryAppearance>().unwrap();
    assert_eq!(appearance[0].original_tag_reference, Recorded::Value(Some(41)));
    assert_eq!(appearance[0].original_tag, Recorded::Value(Some(b"AAA".to_vec())));
    assert!(held.complete::<tables::ReservedCountryNaming>().is_ok());

    let market_params = MarketLendLeaseParams {
        branch_id: "1".into(),
        countries: Vec::new(),
        only: Some(MarketSection::Offers),
    };
    let mut market_held = DecodedFrame {
        branch: 1,
        hour: FIRST,
        tables: std::array::from_fn(|_| replay_frame::frame::not_selected()),
        metrics: Default::default(),
    };
    for step in 0..8 {
        let step_reads = MarketLendLease::reads(&market_params, None, step, &market_held);
        if step_reads.is_empty() {
            break;
        }
        let keyed: Vec<replay_lanes::at::Read<'_>> = step_reads
            .iter()
            .map(|read| replay_lanes::at::Read {
                table: read.table,
                columns: None,
                keys: match &read.keys {
                    Keys::All => replay_lanes::at::Keys::All,
                    Keys::These(keys) => replay_lanes::at::Keys::These(keys),
                    Keys::Range(low, high) => replay_lanes::at::Keys::Range(*low, *high),
                    Keys::Ranges(ranges) => replay_lanes::at::Keys::Ranges(ranges),
                },
            })
            .collect();
        let read = replay_lanes::at::frame(recording, 0, FIRST, &keyed).unwrap();
        for asked in &step_reads {
            market_held.tables[asked.table.index()] = read.tables[asked.table.index()].clone();
        }
    }
    let market_lexical = market_held.complete::<tables::LexicalTokens>().unwrap();
    assert!(
        market_lexical
            .iter()
            .any(|row| row.token == 701 && row.literal == b"europe"),
        "market query must fetch distinct capital continent lexical token: {market_lexical:?}"
    );
    assert!(market_held.complete::<tables::ReservedCountryNaming>().is_ok());
}
