use dashboard_world::products::equipment_names as names;
use super::{
    catalogue::{EquipmentDesigns, IndustrialOrganisations, ProductionLicences},
    breakdown::{ProductionBreakdownParams, ProductionLineBreakdown},
    live::ProductionLinesAt,
    *,
};
use dashboard_base::{Source, products::{LoadContext, at_hour::entry}, test_fixtures::{CAMPAIGN, records, source}};
use replay::campaign::Replay;
use replay_data::rows::{
    CountryLinkRow, CountryProductionModifiersRow, CountryRow, ModifierBreakdownNode, ProductionLineModifiersRow, ProductionModifierTree, IndustrialOrganisationDetailsRow, IndustrialOrganisationRow, ProductionLicenseRecordRow, ProductionLineOutputRow, ProductionLineRow, Recorded, TableRows,
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

fn factor(source: &Source, handle: &str, group: &str, hour: i32) -> std::io::Result<breakdown::ProductionLineBreakdownReply> {
    let params = ProductionBreakdownParams { branch_id: "1".into(), country: "GER".into(), handle: handle.into(), group: group.into(), hour };
    <ProductionLineBreakdown as dashboard_base::products::Product>::load(source, &params, &load(), &AtomicBool::new(false))
}

fn breakdown(source: &Source, handle: &str, hour: i32) -> std::io::Result<breakdown::ProductionLineBreakdownReply> {
    factor(source, handle, "efficiency", hour)
}

fn modified_recording() -> Source {
    let frames = (FIRST..=LAST)
        .map(|hour| {
            let mut frame = frame(hour);
            *frame.tables[TableId::ProductionLineModifiers.index()] = TableView::Complete(TableRows::ProductionLineModifiers(vec![ProductionLineModifiersRow {
                line: 1,
                organisation_dirty: (1_100..1_130).contains(&hour),
                ..Default::default()
            }]));
            *frame.tables[TableId::CountryProductionModifiers.index()] = if hour < 1_150 {
                TableView::Complete(TableRows::CountryProductionModifiers(vec![CountryProductionModifiersRow {
                    country: 11,
                    trees: engine_executable::native_values::production_modifiers::KEYS.into_iter().map(|key| ProductionModifierTree { key, nodes: vec![ModifierBreakdownNode { children: 0, value: 0, name: b"synthetic source".to_vec() }], failure: Vec::new() }).collect(),
                    ..Default::default()
                }]))
            } else {
                TableView::Unavailable { reason: "country modifiers unread in the fixture".into() }
            };
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "production-modified".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

#[test]
fn a_breakdown_is_the_clicked_line_and_its_operands_at_the_clicked_hour_alone() {
    let source = modified_recording();
    let pending = breakdown(&source, "1", 1_120).unwrap();
    assert_eq!(pending.hour, 1_120);
    assert_eq!(pending.group.as_ref().and_then(|group| group.total_raw.as_deref()), Some("5000000"), "the line's stored efficiency at the hour");
    assert!(pending.warnings.iter().any(|warning| warning.starts_with("Organisation bonuses were pending refresh")), "the hour's own warnings reach the reply: {:?}", pending.warnings);
    assert!(pending.warnings.iter().all(|warning| !warning.contains("not recorded")), "operands missing at a LATER hour say nothing about this one: {:?}", pending.warnings);
    let settled = breakdown(&source, "1", 1_140).unwrap();
    assert!(settled.warnings.iter().all(|warning| !warning.starts_with("Organisation bonuses")), "the pending refresh of an earlier hour is not this hour's: {:?}", settled.warnings);
    let unread = breakdown(&source, "1", 1_160).unwrap();
    let kept = unread.group.as_ref().expect("the witnessed efficiency total is kept when the country's modifiers are unread");
    assert_eq!(kept.total_raw.as_deref(), Some("5000000"));
    const UNREAD_MODIFIERS: &str = "Country production modifiers not available at this hour: country modifiers unread in the fixture";
    assert!(kept.parts.iter().all(|part| part.unknown.as_deref() == Some(UNREAD_MODIFIERS)), "the table's own reason names why: {:?}", kept.parts);
    assert!(unread.warnings.iter().any(|warning| warning == UNREAD_MODIFIERS), "and the kept total carries it as a warning: {:?}", unread.warnings);
    assert_eq!(unread.missing, None);
    assert_eq!(factor(&source, "1", "output", 1_160).unwrap().group.and_then(|group| group.total_raw), Some("200000".into()), "the recorded IC rate is the line's own");
    let start = factor(&source, "1", "start", 1_160).unwrap();
    assert_eq!((start.group, start.missing.as_deref()), (None, Some(UNREAD_MODIFIERS)));
    assert!(breakdown(&source, "1", UNREAD.0).is_err(), "the lines are unread at the clicked hour: nothing from another hour stands in");
}

#[test]
fn a_breakdown_opens_for_the_line_the_country_holds_at_the_clicked_hour() {
    let source = recording();
    let alias = breakdown(&source, "1", 1_120).expect("raw country 11's line is GER's: GER's slot holds it");
    assert_eq!(alias.hour, 1_120);
    assert!(breakdown(&source, "3", 1_120).is_err(), "ITA's line is not GER's");
    assert!(breakdown(&source, "2", 1_210).is_err(), "a line that ended at 1200 is not the country's after it");
    let moved = moved_recording();
    assert!(breakdown(&moved, "1", 1_120).is_ok());
    assert!(breakdown(&moved, "1", 1_160).is_err(), "the line leaves GER at the hour its raw country's link moves");
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
    let endpoint = |reply: &Value, at: &Value| reply["endpoints"][at.as_u64().unwrap() as usize].clone();
    let incoming = reply["incoming"]["links"].as_array().unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!((endpoint(&reply, &incoming[0]["from"])["tag"].as_str(), incoming[0]["total"].as_str()), (Some("ITA"), Some("1")), "the alias is GER");
    assert!(reply["outgoing"]["missing"].as_str().is_some() && reply["outgoing"]["total"].is_null(), "a licence whose giver cannot be linked may be GER's: that side is unproven, never empty");
    let world = asked(&source, entry::<ProductionLicences>(), json!({ "branchId": "1", "hour": 1_210 }));
    let links = world["links"].as_array().unwrap();
    assert_eq!(links.len(), 2, "a licence with an unlinked giver stays one licence");
    let unlinked = links.iter().find(|link| endpoint(&world, &link["from"])["slot"].is_null()).unwrap();
    assert_eq!((endpoint(&world, &unlinked["to"])["tag"].as_str(), unlinked["total"].as_str()), (Some("ITA"), Some("1")));
    assert!(endpoint(&world, &unlinked["from"])["unknown"].as_str().is_some());
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
fn weekly_units_need_a_positive_cost() {
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
        lease_side: None,
        group: None,
        row: None,
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

#[test]
fn a_truncated_link_table_keeps_a_licence_whose_parties_it_witnessed_and_unlinked_givers_stay_distinct() {
    use super::catalogue::witnessed_licences;
    use dashboard_base::products::flows::{FlowEnd, FlowGap};
    let mut frame = frame(1_210);
    let links = match &*frame.tables[TableId::CountryLinks.index()] {
        TableView::Complete(TableRows::CountryLinks(rows)) => rows.clone(),
        _ => panic!("the fixture records country links"),
    };
    let ger = links.iter().find(|link| link.id == 11).map(|link| link.canonical_slot).unwrap();
    *frame.tables[TableId::CountryLinks.index()] = TableView::Truncated { rows: TableRows::CountryLinks(links), missing: 5 };
    let licences = vec![
        ProductionLicenseRecordRow { owner: 11, ordinal: 0, giver: 9002, ..Default::default() },
        ProductionLicenseRecordRow { owner: 11, ordinal: 1, giver: 77, ..Default::default() },
        ProductionLicenseRecordRow { owner: 11, ordinal: 2, giver: 78, ..Default::default() },
    ];
    *frame.tables[TableId::ProductionLicenseRecords.index()] = TableView::Complete(TableRows::ProductionLicenseRecords(licences.clone()));
    let read = witnessed_licences(&frame).unwrap();
    let ends: Vec<(FlowEnd, FlowEnd)> = read.edges.iter().map(|edge| (edge.from, edge.to)).collect();
    assert_eq!(ends, [(FlowEnd::Slot(8), FlowEnd::Slot(ger)), (FlowEnd::Raw(77), FlowEnd::Slot(ger)), (FlowEnd::Raw(78), FlowEnd::Slot(ger))], "both parties witnessed: the licence stands; two unlinked givers are two partners");
    assert!(read.unlinked[&77].contains("read only in part at this hour (5 rows not read)"), "an unlinked giver says why it is unlinked");
    assert!(read.gaps.is_empty());
    *frame.tables[TableId::ProductionLicenseRecords.index()] = TableView::Truncated { rows: TableRows::ProductionLicenseRecords(licences[..1].to_vec()), missing: 2 };
    let read = witnessed_licences(&frame).unwrap();
    assert_eq!(read.edges.len(), 1, "a truncated licence table keeps the licence it witnessed");
    assert!(matches!(&read.gaps[..], [gap] if gap.partial && gap.from == FlowGap::everything(String::new()).from && gap.reason.ends_with("read only in part at this hour (2 rows not read)")));
    *frame.tables[TableId::ProductionLicenseRecords.index()] = TableView::Unavailable { reason: "reader fault".into() };
    assert!(witnessed_licences(&frame).is_err());
}

#[test]
fn a_licence_subject_whose_holder_is_not_known_is_not_recorded_with_the_reason_though_the_licences_are_withheld_too() {
    let gapped = |hour: i32| {
        let mut frame = frame(hour);
        if (1_210..=1_220).contains(&hour) {
            *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "countries unread in the fixture".into() };
            *frame.tables[TableId::ProductionLicenseRecords.index()] = TableView::Unavailable { reason: "licences unread in the fixture".into() };
        }
        frame
    };
    let frames = (FIRST..=LAST).map(gapped).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "production-gap".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let during = asked(&source, entry::<ProductionLicences>(), json!({ "branchId": "1", "hour": 1_215, "subject": "GER" }));
    assert!(during["missing"].as_str().is_some_and(|missing| missing.starts_with("Who holds GER is not known")), "{}", during["missing"]);
    let after = asked(&source, entry::<ProductionLicences>(), json!({ "branchId": "1", "hour": 1_230, "subject": "GER" }));
    assert!(after["missing"].as_str().is_none_or(|missing| !missing.contains("is not known")), "{}", after["missing"]);
}
