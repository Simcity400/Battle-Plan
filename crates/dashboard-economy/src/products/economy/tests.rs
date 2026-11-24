use dashboard_base::{Source, products::{LoadContext, at_hour::{AtHour, entry}}, test_fixtures::{CAMPAIGN, records, source}};
use dashboard_follow::panel::{Panel, PanelData};
use replay::campaign::{Replay, SeekResult};
use replay_data::rows::{TableId, TableSchema};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

pub(super) const FIRST: i32 = 100;
pub(super) const LAST: i32 = 130;
pub(super) const GER: u32 = 9001;

pub(super) fn open(frames: Vec<DecodedFrame>) -> Source {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "economy".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

pub(super) fn load() -> LoadContext {
    LoadContext { content: None }
}

pub(super) fn asked<P: AtHour>(source: &Source, mut params: Value, hour: i32) -> Result<Value, String> {
    params["hour"] = json!(hour);
    let text = (entry::<P>().registered.load)(source, "lease", params, &load(), &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let mut reply: Value = serde_json::from_str(&text).unwrap();
    Ok(reply["data"].take())
}

pub(super) fn followed(source: &Source, panel: &mut dyn Panel, hour: i32) -> PanelData {
    let tables = panel.tables().to_vec();
    let SeekResult::Frame(frame) = source.replay_ref().seek_selected(1, hour, Some(&tables)).unwrap() else { panic!("hour {hour} is recorded") };
    panel.data(&frame)
}

pub(super) fn same_on_both_paths<P: AtHour>(source: &Source, params: Value) {
    let mut asked_panel = params.clone();
    asked_panel["panel"] = json!(P::KEY);
    let mut panel = (entry::<P>().panel)(source, &load(), &asked_panel).unwrap();
    for hour in FIRST..=LAST {
        let mut asked = asked::<P>(source, params.clone(), hour).unwrap();
        asked.as_object_mut().unwrap().remove("live");
        assert_eq!(followed(source, &mut *panel, hour), Ok(asked), "hour {hour}");
    }
}

pub(super) fn cells(reply: &Value) -> std::collections::BTreeMap<(String, String), Value> {
    let live = &reply["live"];
    let numbers = |value: &Value| value.as_array().unwrap().iter().map(|item| item.as_u64().unwrap() as usize).collect::<Vec<_>>();
    let flat = numbers(&live["cells"]);
    let mut out = std::collections::BTreeMap::new();
    let mut at = 0;
    while at < flat.len() {
        let (row, kind, count) = (flat[at], flat[at + 1], flat[at + 2]);
        let (group, key, column) = (flat[at + 3], flat[at + 4], flat[at + 5]);
        let width = live["groups"][group]["columns"].as_array().unwrap().len();
        let value = live["groups"][group]["values"][key * width + column].clone();
        out.insert((live["rows"][row].as_str().unwrap().to_owned(), live["kinds"][kind]["id"].as_str().unwrap().to_owned()), value);
        at += 3 + 3 * count;
    }
    out
}

pub(super) fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

mod fuel {
    use super::*;
    use crate::products::economy::fuel_branches::FuelBranches;
    use replay_data::rows::{CountryFuelConsumerRow, tables};

    const UNREAD: i32 = 120;

    fn consumer(country: u32, index: u32, requested_raw: i64, received_raw: i64) -> CountryFuelConsumerRow {
        CountryFuelConsumerRow { country, index, priority: 1, requested_raw, received_raw }
    }

    fn recording() -> Source {
        open(
            (FIRST..=LAST)
                .map(|hour| {
                    let mut frame = source(1, hour, Some(1));
                    if hour == UNREAD {
                        *frame.tables[TableId::CountryFuelConsumers.index()] = TableView::Unavailable { reason: "fuel status unreadable".into() };
                    } else {
                        let asked = i64::from(hour / 10) * 100_000;
                        set::<tables::CountryFuelConsumers>(&mut frame, vec![consumer(44, 0, 7, 7), consumer(GER, 0, asked, asked / 2), consumer(GER, 2, 0, 0), consumer(GER + 1, 1, 9, 9)]);
                    }
                    frame
                })
                .collect(),
        )
    }

    #[test]
    fn fuel_by_branch_is_the_countrys_own_consumers_at_the_hour_on_both_paths() {
        let source = recording();
        let params = json!({ "branchId": "1", "country": "GER" });
        same_on_both_paths::<FuelBranches>(&source, params.clone());
        let at = asked::<FuelBranches>(&source, params.clone(), 115).unwrap();
        assert_eq!(at["consumers"], json!([
            { "index": 0, "requested": "1100000", "received": "550000", "priority": 1 },
            { "index": 2, "requested": "0", "received": "0", "priority": 1 },
        ]));
        assert!(at["missing"].is_null());
        assert_eq!(at["consumerNames"], json!([null, null, null]), "no content names no consumer");
        assert!(at["warnings"].as_array().unwrap().iter().any(|warning| warning.as_str().unwrap().contains("TOPBAR_ARMY")));
        let unread = asked::<FuelBranches>(&source, params, UNREAD).unwrap();
        assert_eq!(unread["consumers"], json!([]));
        assert!(unread["readHour"].is_null());
        assert!(unread["missing"].as_str().unwrap().starts_with("Fuel consumers are not recorded at this hour"), "unknown is a named reason, never no consumers");
    }
}

mod resources {
    use super::*;
    use dashboard_base::products::Product;
    use crate::products::economy::resources::{ResourceBalance, ResourceSizes};
    use replay_data::rows::{CountryResourceRow, CountryRow, LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow, tables};

    const DEALS_UNREAD: i32 = 118;
    const FRA: u32 = 44;

    fn pools(country: u32, resource: u32, produced: i64) -> CountryResourceRow {
        CountryResourceRow { country, resource, produced, used: produced, special_total: produced, production_demand: -3, ..Default::default() }
    }

    fn deal(id: u64, exporter: u32, importer: u32, resource: u32, delivered: i64) -> ResourceExchangeRow {
        ResourceExchangeRow { id, exporter, importer, resource, delivered, assigned_factories: 1, convoys_assigned: 2, convoys_required: 3, ..Default::default() }
    }

    fn recording() -> Source {
        open(
            (FIRST..=LAST)
                .map(|hour| {
                    let mut frame = source(1, hour, Some(1));
                    if let TableView::Complete(replay_data::rows::TableRows::Countries(rows)) = &mut *frame.tables[TableId::Countries.index()] {
                        rows.push(CountryRow { id: FRA, canonical_slot: 8, tag: b"FRA".to_vec(), ..CountryRow::default() });
                        rows.sort_by_key(|row| row.id);
                    }
                    set::<tables::ResourceDefinitions>(&mut frame, vec![
                        ResourceDefinitionRow { id: 1, token: 70, active: 1, factory_cost: 0 },
                        ResourceDefinitionRow { id: 2, token: 71, active: 1, factory_cost: 0 },
                    ]);
                    set::<tables::LexicalTokens>(&mut frame, vec![
                        LexicalTokenRow { token: 70, literal: b"oil".to_vec() },
                        LexicalTokenRow { token: 71, literal: b"steel".to_vec() },
                    ]);
                    let oil = i64::from(hour / 10) * 100_000;
                    set::<tables::CountryResources>(&mut frame, vec![pools(FRA, 1, 5), pools(GER, 1, oil), pools(GER, 2, 700_000), pools(GER + 1, 2, 9)]);
                    if hour == DEALS_UNREAD {
                        *frame.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable { reason: "deals unreadable".into() };
                    } else {
                        set::<tables::ResourceExchanges>(&mut frame, vec![deal(5, GER, FRA, 2, 200_000), deal(6, FRA, GER, 1, 300_000), deal(7, FRA, 66, 1, 1)]);
                    }
                    frame
                })
                .collect(),
        )
    }

    #[test]
    fn a_resource_balance_is_the_countrys_pools_and_deals_at_the_hour_on_both_paths() {
        let source = recording();
        let params = json!({ "branchId": "1", "country": "GER" });
        same_on_both_paths::<ResourceBalance>(&source, params.clone());
        let at = asked::<ResourceBalance>(&source, params.clone(), 125).unwrap();
        let tokens: Vec<&str> = at["resources"].as_array().unwrap().iter().map(|resource| resource["token"].as_str().unwrap()).collect();
        assert_eq!(tokens, ["oil", "steel"]);
        assert!(at["resources"][0]["name"].is_null(), "no content names no resource");
        assert_eq!(at["pools"], json!([1, 2]), "the country's own rows, in the game's order");
        assert_eq!(at["shown"], json!([1, 2]));
        let cells = cells(&at);
        let cell = |row: &str, column: &str| cells[&(row.to_owned(), column.to_owned())].clone();
        assert_eq!((cell("1", "produced"), cell("2", "produced"), cell("1", "production")), (json!(1_200_000), json!(700_000), json!(-3)));
        assert_eq!(cells.len(), 18, "eight pools of each of GER's two rows and each of its two deals' delivery, none of another country's");
        assert_eq!((cell("6", "delivered"), cell("5", "delivered")), (json!(300_000), json!(200_000)));
        let ordered = |order: &str| asked::<ResourceBalance>(&source, json!({ "branchId": "1", "country": "GER", "order": order }), 125).unwrap()["shown"].clone();
        assert_eq!((ordered("game"), ordered("shortfall"), ordered("volume")), (json!([1, 2]), json!([2, 1]), json!([1, 2])));
        assert_eq!(at["deals"], json!([
            { "deal": "6", "resource": 1, "direction": "import", "partner": "FRA", "factories": 1, "convoysAssigned": 2, "convoysRequired": 3 },
            { "deal": "5", "resource": 2, "direction": "export", "partner": "FRA", "factories": 1, "convoysAssigned": 2, "convoysRequired": 3 },
        ]));
        let unread = asked::<ResourceBalance>(&source, params, DEALS_UNREAD).unwrap();
        assert_eq!(unread["pools"].as_array().unwrap().len(), 2, "the pools go on where the deals are not known");
        assert_eq!(unread["deals"], json!([]));
        assert!(unread["dealsMissing"].as_str().unwrap().starts_with("Trade deals are not recorded at this hour"), "unknown deals are not no deals");
    }

    #[test]
    fn a_resources_campaign_size_is_its_longest_bar_over_the_span() {
        let source = recording();
        let params = serde_json::from_value(json!({ "branchId": "1", "country": "GER" })).unwrap();
        let reply = ResourceSizes::load(&source, &params, &load(), &AtomicBool::new(false)).unwrap();
        let sizes: Vec<(u32, Option<&str>)> = reply.sizes.iter().map(|size| (size.id, size.longest_raw.as_deref())).collect();
        assert_eq!(sizes, [(1, Some("1300000")), (2, Some("700000"))]);
    }
}

mod lend_lease {
    use super::*;
    use crate::products::economy::lend_lease_balance::LendLeaseBalance;
    use replay_data::rows::{CountryLinkRow, CountryRow, TableRows, WarLendLeaseHistoryRow, WarOwnedRelationRow, tables};

    const UNREAD: i32 = 112;

    fn history(country: u32, target: u32, industry_sent: i64, industry_received: i64) -> WarLendLeaseHistoryRow {
        WarLendLeaseHistoryRow { country, target, industry_sent, fuel_sent: 0, industry_received, fuel_received: 0 }
    }

    fn recording() -> Source {
        open(
            (FIRST..=LAST)
                .map(|hour| {
                    let mut frame = source(1, hour, Some(1));
                    if let TableView::Complete(TableRows::Countries(rows)) = &mut *frame.tables[TableId::Countries.index()] {
                        rows.push(CountryRow { id: 44, canonical_slot: 8, tag: b"FRA".to_vec(), ..CountryRow::default() });
                        rows.push(CountryRow { id: 66, canonical_slot: 10, tag: b"MOD".to_vec(), ..CountryRow::default() });
                        rows.sort_by_key(|row| row.id);
                    }
                    if let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *frame.tables[TableId::CountryLinks.index()] {
                        rows.extend([CountryLinkRow { id: 44, canonical_slot: 8 }, CountryLinkRow { id: 66, canonical_slot: 10 }]);
                        rows.sort_by_key(|row| row.id);
                    }
                    set::<tables::WarOwnedRelations>(&mut frame, vec![WarOwnedRelationRow { owner: 66, ordinal: 0, first_country: GER, second_country: 44, ..Default::default() }]);
                    if hour == UNREAD {
                        *frame.tables[TableId::WarLendLeaseHistories.index()] = TableView::Unavailable { reason: "histories unreadable".into() };
                    } else {
                        let sent = i64::from(hour / 10) * 100;
                        set::<tables::WarLendLeaseHistories>(&mut frame, vec![history(11, 44, 100, 0), history(44, GER, 0, sent + 100), history(GER, 44, sent, 0)]);
                    }
                    frame
                })
                .collect(),
        )
    }

    #[test]
    fn lend_lease_totals_sum_each_slots_histories_and_hold_rostered_countries_at_zero() {
        let source = recording();
        let every = json!({ "branchId": "1" });
        same_on_both_paths::<LendLeaseBalance>(&source, every.clone());
        let at = asked::<LendLeaseBalance>(&source, every.clone(), 125).unwrap();
        assert_eq!(at["rows"], json!([
            { "slot": 7, "tag": "GER", "values": ["1300", "0", "0", "0"] },
            { "slot": 8, "tag": "FRA", "values": ["0", "0", "1300", "0"] },
            { "slot": 10, "tag": "MOD", "values": ["0", "0", "0", "0"] },
        ]));
        let partners = json!({ "branchId": "1", "holder": "GER" });
        same_on_both_paths::<LendLeaseBalance>(&source, partners.clone());
        assert_eq!(asked::<LendLeaseBalance>(&source, partners, 125).unwrap()["rows"], json!([{ "slot": 8, "tag": "FRA", "values": ["1300", "0", "0", "0"] }]));
        let top = json!({ "branchId": "1", "rank": "industry", "top": 1 });
        same_on_both_paths::<LendLeaseBalance>(&source, top.clone());
        assert_eq!(asked::<LendLeaseBalance>(&source, top, 125).unwrap()["rows"], json!([
            { "slot": 7, "tag": "GER", "values": ["1300", "0", "0", "0"] },
        ]), "the product cuts the top after ranking by sent plus received, ties by slot");
        let ranked = asked::<LendLeaseBalance>(&source, json!({ "branchId": "1", "rank": "fuel" }), 125).unwrap();
        assert_eq!(ranked["rows"], json!([]), "a ranked population leaves out countries with none of the measure");
        let chosen = asked::<LendLeaseBalance>(&source, json!({ "branchId": "1", "rank": "industry", "countries": ["MOD", 8] }), 125).unwrap();
        assert_eq!(chosen["rows"], json!([
            { "slot": 8, "tag": "FRA", "values": ["0", "0", "1300", "0"] },
            { "slot": 10, "tag": "MOD", "values": ["0", "0", "0", "0"] },
        ]), "chosen countries keep a recorded zero and come in rank order");
        let unread = asked::<LendLeaseBalance>(&source, every, UNREAD).unwrap();
        assert_eq!(unread["rows"], json!([]));
        assert!(unread["missing"].as_str().unwrap().starts_with("Lend-lease histories are not recorded"), "unknown is not no lend-lease");
    }
}

mod market {
    use super::*;
    use crate::products::economy::market::{MarketLendLease, history::ended_spans};
    use replay_lanes::lanes::{Cell, Step};
    use replay_data::rows::{CountryLinkRow, CountryMarketRow, CountryRow, LendLeaseAgreementRow, MarketContractTermsRow, MarketEquipment, MarketPool, PurchaseContractRow, SharedMarketPool, tables};

    const FRA: u32 = 44;
    const UNREAD: i32 = 117;
    const ENDS: i32 = 124;

    fn equipment(variant: u64, amount_raw: i64) -> SharedMarketPool {
        MarketPool { allow_zero: false, items: vec![MarketEquipment { variant, amount_raw }] }.into()
    }

    fn recording() -> Source {
        open(
            (FIRST..=LAST)
                .map(|hour| {
                    let mut frame = source(1, hour, Some(1));
                    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
                    countries.push(CountryRow { id: FRA, canonical_slot: 3, tag: b"FRA".to_vec(), ..CountryRow::default() });
                    countries.sort_by_key(|row| row.id);
                    set::<tables::Countries>(&mut frame, countries);
                    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
                    links.push(CountryLinkRow { id: FRA, canonical_slot: 3 });
                    links.sort_by_key(|row| row.id);
                    set::<tables::CountryLinks>(&mut frame, links);
                    set::<tables::LexicalTokens>(&mut frame, Vec::new());
                    set::<tables::VariantDefinitions>(&mut frame, Vec::new());
                    set::<tables::CountryMarket>(&mut frame, vec![
                        CountryMarketRow { country: FRA, inventory: equipment(5, 300_000), ..CountryMarketRow::default() },
                        CountryMarketRow { country: GER, inventory: equipment(6, i64::from(hour) * 100_000), subsidy_total_raw: 7, ..CountryMarketRow::default() },
                    ]);
                    set::<tables::MarketContractTerms>(&mut frame, vec![
                        MarketContractTermsRow { country: FRA, id: 0x51, kind: 1, buyer: GER, speed: 2, equipment: equipment(5, 100_000), ..MarketContractTermsRow::default() },
                        MarketContractTermsRow { country: FRA, id: 0x52, kind: 0, buyer: 77, ..MarketContractTermsRow::default() },
                    ]);
                    if hour == UNREAD {
                        *frame.tables[TableId::PurchaseContracts.index()] = TableView::Unavailable { reason: "purchases unreadable".into() };
                    } else {
                        set::<tables::PurchaseContracts>(&mut frame, vec![PurchaseContractRow {
                            id: 0x51,
                            seller: FRA,
                            buyer: GER,
                            convoys_assigned: 2,
                            convoys_total: 3,
                            collected_raw: i64::from(hour) * 1_000,
                            completed_deliveries: 1,
                            ..PurchaseContractRow::default()
                        }]);
                    }
                    let leases = if hour < ENDS {
                        vec![LendLeaseAgreementRow { country: GER, id: 9, receiver: FRA, active_since: FIRST, monthly: equipment(6, 50_000), fuel_sent_raw: i64::from(hour) * 32_768, ..LendLeaseAgreementRow::default() }]
                    } else {
                        Vec::new()
                    };
                    set::<tables::LendLeaseAgreements>(&mut frame, leases);
                    frame
                })
                .collect(),
        )
    }

    fn without_live(mut reply: Value) -> Value {
        reply.as_object_mut().unwrap().remove("live");
        reply
    }

    fn panel(source: &Source, params: &Value) -> Box<dyn Panel> {
        let mut asked = params.clone();
        asked["panel"] = json!(MarketLendLease::KEY);
        (entry::<MarketLendLease>().panel)(source, &load(), &asked).unwrap()
    }

    #[test]
    fn the_market_at_an_hour_is_the_rows_a_chosen_country_takes_part_in_on_both_paths() {
        let source = recording();
        let params = json!({ "branchId": "1", "countries": ["GER"] });
        let mut followed_panel = panel(&source, &params);
        for hour in FIRST..=LAST {
            let reply = asked::<MarketLendLease>(&source, params.clone(), hour).unwrap();
            assert_eq!(followed(&source, &mut *followed_panel, hour), Ok(without_live(reply)), "hour {hour}");
        }
        let at = asked::<MarketLendLease>(&source, params.clone(), 110).unwrap();
        assert_eq!(at["offers"].as_array().unwrap().len(), 1, "FRA's listing is not GER's");
        assert_eq!(at["offers"][0]["tag"], "GER");
        assert_eq!(at["offers"][0]["inventory"], json!([{ "variant": "6", "name": null, "amountRaw": "11000000" }]));
        let contracts = at["contracts"].as_array().unwrap();
        assert_eq!(contracts.len(), 1, "GER buys on FRA's contract; FRA's request to a country with no identity is not GER's");
        assert_eq!((contracts[0]["seller"].as_str(), contracts[0]["buyer"].as_str(), contracts[0]["purchase"].as_bool()), (Some("FRA"), Some("GER"), Some(true)));
        assert_eq!(at["agreements"][0]["receiver"], "FRA");
        assert!(at["live"].is_object(), "purchase progress and fuel are named as live cells");
        let ended = asked::<MarketLendLease>(&source, params, ENDS).unwrap();
        assert_eq!(ended["agreements"], json!([]));
    }

    #[test]
    fn purchase_progress_moves_as_live_cells_and_unread_progress_is_named() {
        let source = recording();
        let params = json!({ "branchId": "1", "countries": ["GER"], "only": "contracts" });
        let mut contracts = panel(&source, &params);
        let (first, second) = (followed(&source, &mut *contracts, 110), followed(&source, &mut *contracts, 111));
        assert!(first.is_ok());
        assert_eq!(first, second, "collected CIC moves each hour in a live cell, never in the panel");
        assert!(contracts.live().is_some());
        let unread = asked::<MarketLendLease>(&source, params, UNREAD).unwrap();
        assert!(unread["purchasesMissing"].as_str().unwrap().contains("purchases unreadable"));
        assert_eq!(unread["contracts"][0]["purchase"], false);
        assert!(unread["offers"].is_null() && unread["agreements"].is_null(), "only the section asked is read");
        let every = asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": [] }), 110).unwrap();
        assert_eq!(every["contracts"].as_array().unwrap().len(), 2);
        assert_eq!(every["contracts"][1]["buyer"], Value::Null, "a buyer with no identity is unnamed, never dropped");
    }

    #[test]
    fn an_agreement_ends_where_its_row_leaves_for_good_or_in_a_read_hour_not_where_the_table_was_unread() {
        let shown = |hour| Step { hour, cells: Some(vec![Cell::Number(0)].into_boxed_slice()) };
        let hidden = |hour| Step { hour, cells: None };
        let history = vec![(1u128, vec![shown(100), hidden(110), shown(115), hidden(120)]), (2, vec![shown(100)]), (3, vec![shown(100), hidden(130)])];
        assert_eq!(ended_spans(&history, |hour| hour != 110 && hour != 130), vec![(1, 115, 119), (3, 100, 129)], "a row gone for good ended though its hour was not read");
    }
}