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
    use crate::products::economy::resources::ResourceBalance;
    use replay_data::rows::{CountryResourceRow, CountryRow, LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow, tables};

    const DEALS_UNREAD: i32 = 118;
    const DEAL_BEGINS: i32 = 122;
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
                        ResourceDefinitionRow { id: 3, token: 72, active: 1, factory_cost: 0 },
                        ResourceDefinitionRow { id: 4, token: 73, active: 1, factory_cost: 0 },
                        ResourceDefinitionRow { id: 5, token: 74, active: 1, factory_cost: 0 },
                    ]);
                    set::<tables::LexicalTokens>(&mut frame, vec![
                        LexicalTokenRow { token: 70, literal: b"oil".to_vec() },
                        LexicalTokenRow { token: 71, literal: b"steel".to_vec() },
                        LexicalTokenRow { token: 72, literal: b"coal".to_vec() },
                        LexicalTokenRow { token: 73, literal: b"chromium".to_vec() },
                        LexicalTokenRow { token: 74, literal: b"tungsten".to_vec() },
                    ]);
                    let oil = i64::from(hour / 10) * 100_000;
                    set::<tables::CountryResources>(&mut frame, vec![
                        pools(FRA, 1, 5),
                        pools(GER, 1, oil),
                        pools(GER, 2, 700_000),
                        pools(GER, 3, 0),
                        CountryResourceRow { country: GER, resource: 4, ..Default::default() },
                        pools(GER + 1, 2, 9),
                    ]);
                    if hour == DEALS_UNREAD {
                        *frame.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable { reason: "deals unreadable".into() };
                    } else {
                        let mut deals = vec![deal(5, GER, FRA, 2, 200_000), deal(6, FRA, GER, 1, 300_000), deal(7, FRA, 66, 1, 1)];
                        if hour >= DEAL_BEGINS {
                            deals.push(deal(8, FRA, GER, 3, 50_000));
                        }
                        set::<tables::ResourceExchanges>(&mut frame, deals);
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
        assert_eq!(tokens, ["oil", "steel", "coal", "chromium", "tungsten"]);
        assert!(at["resources"][0]["name"].is_null(), "no content names no resource");
        assert_eq!(at["pools"], json!([1, 2, 3, 4]), "the country's own rows, in the game's order");
        assert_eq!(at["rows"], json!([
            { "resource": 3, "net": "-3", "supply": "0", "demand": "3", "missing": null },
            { "resource": 1, "net": "1199997", "supply": "1200000", "demand": "3", "missing": null },
            { "resource": 2, "net": "699997", "supply": "700000", "demand": "3", "missing": null },
            { "resource": 4, "net": "0", "supply": "0", "demand": "0", "missing": null },
            { "resource": 5, "net": null, "supply": null, "demand": null, "missing": "No row of this resource was recorded for this country at this hour" },
        ]), "shortages, then surpluses largest first, then the balanced, then the unrecorded; exact fixed-point figures");
        assert_eq!((at["short"].clone(), at["unrecorded"].clone()), (json!(1), json!(1)), "a resource not recorded is never counted short");
        let hidden = asked::<ResourceBalance>(&source, json!({ "branchId": "1", "country": "GER", "hideEmpty": true }), 125).unwrap();
        let ids: Vec<u64> = hidden["rows"].as_array().unwrap().iter().map(|row| row["resource"].as_u64().unwrap()).collect();
        assert_eq!(ids, [3, 1, 2, 5], "only a recorded row with nothing in or out is hidden");
        let cells = cells(&at);
        let cell = |row: &str, column: &str| cells[&(row.to_owned(), column.to_owned())].clone();
        assert_eq!((cell("1", "produced"), cell("2", "produced"), cell("1", "production")), (json!(1_200_000), json!(700_000), json!(-3)));
        assert_eq!(cells.len(), 35, "eight pools of each of GER's four rows and each of its three deals' delivery, none of another country's");
        assert_eq!((cell("6", "delivered"), cell("5", "delivered"), cell("8", "delivered")), (json!(300_000), json!(200_000), json!(50_000)));
        assert_eq!(at["deals"], json!([
            { "deal": "6", "resource": 1, "direction": "import", "partner": "FRA", "factories": 1, "convoysAssigned": 2, "convoysRequired": 3 },
            { "deal": "5", "resource": 2, "direction": "export", "partner": "FRA", "factories": 1, "convoysAssigned": 2, "convoysRequired": 3 },
            { "deal": "8", "resource": 3, "direction": "import", "partner": "FRA", "factories": 1, "convoysAssigned": 2, "convoysRequired": 3 },
        ]), "a deal begun after the first hour asked is read from its own first hour");
        let before = asked::<ResourceBalance>(&source, params.clone(), DEAL_BEGINS - 1).unwrap();
        assert_eq!(before["deals"].as_array().unwrap().len(), 2);
        let unread = asked::<ResourceBalance>(&source, params, DEALS_UNREAD).unwrap();
        assert_eq!(unread["pools"].as_array().unwrap().len(), 4, "the pools go on where the deals are not known");
        assert_eq!(unread["deals"], json!([]));
        assert!(unread["dealsMissing"].as_str().unwrap().starts_with("Trade deals are not recorded at this hour"), "unknown deals are not no deals");
    }

    #[test]
    fn a_countrys_deals_are_read_again_when_one_begins_or_ends() {
        use crate::products::{convoys::RunConvoys, economy::resources::deals_through};
        let mut index = RunConvoys::default();
        index.deals.insert(GER, vec![(5, 100, 110), (8, 122, 200)]);
        index.exports.insert(GER, vec![(6, 90, 140)]);
        index.deals.insert(FRA, vec![(9, 105, 106)]);
        assert_eq!(deals_through(&index, GER, 100), (vec![5, 6], 110));
        assert_eq!(deals_through(&index, GER, 111), (vec![6], 121));
        assert_eq!(deals_through(&index, GER, 122), (vec![6, 8], 140));
        assert_eq!(deals_through(&index, GER, 141), (vec![8], 200));
        assert_eq!(deals_through(&index, GER, 201), (vec![], i32::MAX), "with no deal begun or to begin, nothing expires");
    }
}

mod lend_lease {
    use super::*;
    use crate::products::economy::lend_lease_balance::LendLeaseBalance;
    use replay_data::rows::{CountryLinkRow, CountryRow, TableRows, WarLendLeaseHistoryRow, WarOwnedRelationRow, tables};

    const UNREAD: i32 = 112;
    const ORPHAN: i32 = 118;

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
                        let mut rows = vec![history(11, 44, 100, 0), history(44, GER, 0, sent + 100), history(GER, 44, sent, 0)];
                        if hour == ORPHAN {
                            rows.push(history(999, 44, 5_000, 0));
                            rows.sort_by_key(|row| (row.country, row.target));
                        }
                        set::<tables::WarLendLeaseHistories>(&mut frame, rows);
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
        assert_eq!((&at["unlisted"], &at["rankMissing"]), (&json!(null), &json!(null)), "every history and roster entry resolved: absence and rank are proven");
        let orphan = json!("1 lend-lease histories name a country with no identity; they are left out");
        let unresolved = asked::<LendLeaseBalance>(&source, every.clone(), ORPHAN).unwrap();
        assert_eq!(unresolved["unlisted"], orphan, "an unresolved history leaves an unlisted country unknown, never absent");
        let top = asked::<LendLeaseBalance>(&source, json!({ "branchId": "1", "rank": "industry", "top": 1 }), ORPHAN).unwrap();
        assert_eq!((&top["rows"], &top["rankMissing"]), (&json!([]), &orphan), "an unresolved history of 5000 could be any country's: the top is refused, not cut from the rest");
        let chosen = asked::<LendLeaseBalance>(&source, json!({ "branchId": "1", "rank": "industry", "countries": ["MOD", 8] }), ORPHAN).unwrap();
        assert_eq!((chosen["rows"].as_array().unwrap().len(), &chosen["rankMissing"]), (2, &orphan), "chosen countries stay, their order unproven");
        let unread = asked::<LendLeaseBalance>(&source, every, UNREAD).unwrap();
        assert_eq!(unread["rows"], json!([]));
        assert!(unread["missing"].as_str().unwrap().starts_with("Lend-lease histories are not recorded"), "unknown is not no lend-lease");
    }
}

mod market {
    use super::*;
    use crate::products::economy::market::MarketLendLease;
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
                .map(frame)
                .collect(),
        )
    }

    fn frame(hour: i32) -> DecodedFrame {
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
        assert_eq!(at["unattributed"], 1, "the request to a buyer with no country is left out but counted: it may be GER's");
        assert!(at["live"].is_object(), "purchase progress and fuel are named as live cells");
        let side = |side: &str| asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": ["GER"], "only": "agreements", "leaseSide": side }), 110).unwrap();
        assert_eq!(side("sent")["agreements"].as_array().unwrap().len(), 1, "GER gives to FRA");
        assert_eq!(side("received")["agreements"], json!([]), "GER receives nothing");
        let ended = asked::<MarketLendLease>(&source, params, ENDS).unwrap();
        assert_eq!(ended["agreements"], json!([]));
    }

    #[test]
    fn each_terms_tab_counts_only_its_own_kind_and_a_group_is_its_members_at_the_hour() {
        use crate::products::economy::market::MarketContext;
        use dashboard_base::{products::at_hour::At, span::HourSpan};
        use dashboard_follow::live::Cells;
        use dashboard_query::series::ranked::{GroupKind, GroupMembership, GroupSelection};
        use dashboard_world::products::{group_members::GroupsAt, groups::{CountryGroupReply, FactionGroup, MembershipRun, UnobservedHours}};
        let source = recording();
        let tab = |only: &str| asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": ["GER"], "only": only }), 110).unwrap();
        let requests = tab("requests");
        assert_eq!((&requests["contracts"], requests["unattributed"].as_u64()), (&json!([]), Some(1)), "FRA's request to a buyer with no country may be GER's");
        let contracts = tab("contracts");
        assert_eq!((contracts["contracts"].as_array().unwrap().len(), contracts["unattributed"].as_u64()), (1, Some(0)), "a request never makes the contracts tab unknown");

        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let run = |tag: &str, first_hour, last_hour| MembershipRun { tag: tag.into(), first_hour, last_hour };
        let groups = GroupsAt::new(
            Arc::new(CountryGroupReply {
                branch_id: "1".into(),
                humans: Vec::new(),
                factions: vec![FactionGroup {
                    id: "42".into(),
                    name: None,
                    name_reason: None,
                    names: Vec::new(),
                    recorded_name: String::new(),
                    first_hour: FIRST,
                    last_hour: LAST,
                    members: vec![run("GER", FIRST, LAST), run("FRA", 112, LAST)],
                    leaders: Vec::new(),
                    incomplete: vec![HourSpan { first_hour: 120, last_hour: 121 }],
                }],
                wars: Vec::new(),
                war_starts: Vec::new(),
                capitulations: Vec::new(),
                identity_changes: Vec::new(),
                country_identities: Vec::new(),
                unobserved: UnobservedHours::default(),
                warnings: Vec::new(),
            }),
            FIRST,
        );
        let context = MarketContext { names: Err("no content".into()), groups };
        let faction = GroupSelection { kind: GroupKind::Faction, id: Some("42".into()), tag: None, membership: GroupMembership::Hour };
        let params: crate::products::economy::market::MarketLendLeaseParams = serde_json::from_value(json!({ "branchId": "1", "countries": [], "only": "offers" })).unwrap();
        let params = crate::products::economy::market::MarketLendLeaseParams { group: Some(faction), ..params };
        assert!(MarketLendLease::validate(&crate::products::economy::market::MarketLendLeaseParams { countries: vec![dashboard_base::products::Country::from("GER")], ..params.clone() }).is_err());
        let (live, cancelled) = (Cells::default(), AtomicBool::new(false));
        let at = |hour: i32| At { owner: None, holders: &query.holders, hour, context: &context, cancelled: &cancelled, live: &live };
        let sellers = |hour: i32| {
            let reply = MarketLendLease::reply(&frame(hour), &params, &at(hour)).unwrap();
            (reply.offers.map(|offers| offers.into_iter().filter_map(|offer| offer.tag).collect::<Vec<_>>()), reply.refused)
        };
        assert_eq!(sellers(110), (Some(vec!["GER".to_owned()]), None), "FRA joins at 112: its listing is not the group's at 110");
        assert_eq!(MarketLendLease::valid_through(&frame(110), &params, &at(110)), 111);
        assert_eq!(sellers(112), (Some(vec!["FRA".to_owned(), "GER".to_owned()]), None));
        assert_eq!(sellers(120), (None, Some("Membership of this group is not recorded at this hour".to_owned())), "unknown membership is never a guess");
    }

    #[test]
    fn a_detail_reads_the_clicked_row_by_its_full_recorded_key_even_when_its_seller_is_not_named() {
        const LINKS_UNREAD: i32 = 115;
        let source = open(
            (FIRST..=LAST)
                .map(|hour| {
                    let mut frame = frame(hour);
                    let mut terms = frame.complete::<tables::MarketContractTerms>().unwrap().to_vec();
                    terms.push(MarketContractTermsRow { country: GER, id: 0x52, kind: 0, buyer: FRA, speed: 9, ..MarketContractTermsRow::default() });
                    terms.sort_by_key(|row| (row.country, row.id, row.kind));
                    set::<tables::MarketContractTerms>(&mut frame, terms);
                    if hour == LINKS_UNREAD {
                        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "links unreadable".into() };
                    }
                    frame
                })
                .collect(),
        );
        let key = |seller: u32, id: u128, kind: u128| (u128::from(seller) << 72 | id << 8 | kind).to_string();
        let row = |only: &str, row: String, hour: i32| asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": [], "only": only, "row": row }), hour).unwrap();
        let german = row("contracts", key(GER, 0x52, 0), 110);
        let contracts = german["contracts"].as_array().unwrap();
        assert_eq!(contracts.len(), 1, "two sellers share request id 0x52: only the clicked seller's is read");
        assert_eq!((contracts[0]["seller"].as_str(), contracts[0]["speed"].as_i64()), (Some("GER"), Some(9)));
        let french = row("contracts", key(FRA, 0x52, 0), 110);
        assert_eq!(french["contracts"][0]["seller"], "FRA");
        assert_eq!(french["contracts"][0]["buyer"], Value::Null);
        let unnamed = row("offers", FRA.to_string(), LINKS_UNREAD);
        let offers = unnamed["offers"].as_array().unwrap();
        assert_eq!(offers.len(), 1, "an offer whose seller cannot be named at the hour still opens from its raw seller");
        assert_eq!((offers[0]["country"].as_u64(), offers[0]["tag"].is_null()), (Some(u64::from(FRA)), true));
        assert_eq!(offers[0]["inventory"][0]["variant"], "5");
        let every = asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": [], "only": "offers" }), LINKS_UNREAD).unwrap();
        assert_eq!(every["offers"].as_array().unwrap().len(), 2, "under All, witnessed offers are kept when countries are unread");
        let chosen = asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": ["GER"], "only": "offers" }), LINKS_UNREAD).unwrap();
        assert_eq!((chosen["offers"].as_array().unwrap().len(), chosen["unattributed"].as_u64()), (0, Some(2)), "a chosen country cannot be matched: the rows are counted, not dropped silently");
        assert!(row("contracts", key(FRA, 0x53, 0), 110)["contracts"].as_array().unwrap().is_empty(), "a key with no row at the hour is no row");
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
}
