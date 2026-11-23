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
    frame.metrics.table_ns[T::ID.index()] = 1;
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
        let unread = asked::<LendLeaseBalance>(&source, every, UNREAD).unwrap();
        assert_eq!(unread["rows"], json!([]));
        assert!(unread["missing"].as_str().unwrap().starts_with("Lend-lease histories are not recorded"), "unknown is not no lend-lease");
    }

    #[test]
    fn a_holders_partners_over_time_sum_its_raw_ids_rows_per_partner_slot_and_skip_unread_hours() {
        use dashboard_base::products::Product;
        use crate::products::economy::lend_lease_partners::LendLeasePartners;
        let source = recording();
        let params = serde_json::from_value(json!({ "branchId": "1", "holder": "GER", "measure": "industry" })).unwrap();
        let reply = serde_json::to_value(LendLeasePartners::load(&source, &params, &load(), &AtomicBool::new(false)).unwrap()).unwrap();
        assert_eq!(reply["known"], json!([{ "firstHour": FIRST, "lastHour": UNREAD - 1 }, { "firstHour": UNREAD + 1, "lastHour": LAST }]), "an unread hour is not known");
        assert_eq!(reply["partners"].as_array().unwrap().len(), 1, "both of GER's raw ids send to one partner slot");
        let partner = &reply["partners"][0];
        assert_eq!(partner["slot"], 8);
        assert_eq!(partner["tags"][0]["tag"], "FRA");
        let sent = json!([[100, 109, "1100"], [110, 111, "1200"], [113, 119, "1200"], [120, 129, "1300"], [130, 130, "1400"]]);
        let at = asked::<LendLeaseBalance>(&source, json!({ "branchId": "1", "holder": "GER" }), 125).unwrap();
        assert_eq!(at["rows"][0]["values"][0], "1300", "the partner's sum over time is the at-hour reply's at the same hour");
        assert_eq!(partner["values"][0], sent);
        assert_eq!(partner["values"][1], json!([[100, 111, "0"], [113, 130, "0"]]), "a recorded zero stays zero");
        assert_eq!(partner["values"].as_array().unwrap().len(), 2, "the measure asked: sent and received");
        assert_eq!(reply["totals"][0], sent, "the holder's totals are its partners' sum");
    }

    #[test]
    fn partner_pieces_sum_by_hour_within_the_known_hours_and_hold_zero_after_they_end() {
        use crate::products::economy::lend_lease_partners::summed;
        let pieces = [(10, 14, 5), (12, 19, 7)];
        let known = [(10, 15), (17, 25)];
        assert_eq!(summed(&pieces, &known), [(10, 11, 5), (12, 14, 12), (15, 15, 7), (17, 19, 7), (20, 25, 0)]);
        assert_eq!(summed(&[], &known), [(10, 15, 0), (17, 25, 0)]);
    }
}

mod market {
    use super::*;
    use crate::products::economy::market::{MarketLendLease, history::{EndedSpan, ended_spans}};
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
        assert!(followed_panel.exact().is_some(), "registered market panel runs incrementally");
        for hour in FIRST..=LAST {
            let reply = asked::<MarketLendLease>(&source, params.clone(), hour).unwrap();
            assert_eq!(followed(&source, &mut *followed_panel, hour), Ok(reply), "hour {hour}");
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
        let contracts = panel(&source, &params);
        let exact = contracts.exact().expect("registered market panel runs incrementally");
        let first = (exact.advance)(110, false, &AtomicBool::new(false), None);
        let second = (exact.advance)(111, false, &AtomicBool::new(false), None);
        assert!(first.value.is_ok());
        assert_eq!(*first.value, *second.value, "collected CIC moves each hour in a live cell, never in the panel");
        assert!(first.live.is_some() && second.live.is_some());
        let at_110 = (exact.read)(110, &AtomicBool::new(false)).unwrap();
        let at_111 = (exact.read)(111, &AtomicBool::new(false)).unwrap();
        assert_eq!(without_live(at_110.clone()), without_live(at_111.clone()));
        let key = at_110["contracts"][0]["key"].as_str().unwrap().to_owned();
        assert_eq!(cells(&at_110)[&(key.clone(), "collected".into())], json!(110_000));
        assert_eq!(cells(&at_111)[&(key, "collected".into())], json!(111_000));
        let unread = asked::<MarketLendLease>(&source, params, UNREAD).unwrap();
        assert!(unread["purchasesMissing"].as_str().unwrap().contains("purchases unreadable"));
        assert_eq!(unread["contracts"][0]["purchase"], false);
        assert!(unread["offers"].is_null() && unread["agreements"].is_null(), "only the section asked is read");
        let every = asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": [] }), 110).unwrap();
        assert_eq!(every["contracts"].as_array().unwrap().len(), 2);
        assert_eq!(every["contracts"][1]["buyer"], Value::Null, "a buyer with no identity is unnamed, never dropped");
    }

    #[test]
    fn an_agreement_ends_only_when_a_census_read_proves_it_absent() {
        let shown = |hour| Step { hour, cells: Some(vec![Cell::Number(0)].into_boxed_slice()) };
        let hidden = |hour| Step { hour, cells: None };
        let history = vec![
            (1u128, vec![shown(100), hidden(110), shown(115), hidden(120)]),
            (2, vec![shown(100)]),
            (3, vec![shown(100), hidden(130)]),
            (4, vec![shown(100), hidden(105), shown(118)]),
        ];
        let reads = [(100, 104), (115, 117), (120, 129)];
        assert_eq!(
            ended_spans(&history, 135, &reads, &[]),
            vec![
                EndedSpan { key: 1, first: 100, last: 119, ended_by: 120 },
                EndedSpan { key: 4, first: 100, last: 104, ended_by: 115 },
            ],
            "unread gaps without a census read keep the agreement open; an unread tail never fabricates an end; a later census read proves absence at ended_by"
        );
        let truncated_history = vec![
            (5u128, vec![shown(100), hidden(110), shown(115)]),
            (6, vec![shown(100), hidden(124)]),
        ];
        assert_eq!(
            ended_spans(&truncated_history, 130, &[(100, 109), (115, 123), (125, 130)], &[(110, 110), (124, 124)]),
            vec![EndedSpan { key: 6, first: 100, last: 123, ended_by: 124 }],
            "a final truncated census ends at the truncated hour while a mid-life truncated hour where the agreement returns does not"
        );
    }

    #[test]
    fn truncated_standing_agreements_keep_exact_main_diagnostic_and_end_behavior() {
        use crate::products::economy::market::history::LendLeaseHistory;
        use dashboard_base::products::{BranchParams, Product};
        use replay_data::rows::TableRows;
        let frames = (FIRST..=LAST)
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
                if hour < ENDS {
                    set::<tables::LendLeaseAgreements>(&mut frame, vec![LendLeaseAgreementRow {
                        country: GER,
                        id: 9,
                        receiver: FRA,
                        active_since: FIRST,
                        monthly: equipment(6, 50_000),
                        fuel_sent_raw: i64::from(hour) * 32_768,
                        ..LendLeaseAgreementRow::default()
                    }]);
                } else if hour == ENDS {
                    *frame.tables[TableId::LendLeaseAgreements.index()] = TableView::Truncated {
                        rows: TableRows::LendLeaseAgreements(Vec::new()),
                        missing: 2,
                    };
                    frame.metrics.table_ns[TableId::LendLeaseAgreements.index()] = 1;
                } else {
                    set::<tables::LendLeaseAgreements>(&mut frame, Vec::new());
                }
                frame
            })
            .collect();
        let source = open(frames);
        let reply = LendLeaseHistory::load(&source, &BranchParams { branch_id: "1".into() }, &load(), &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.agreements.len(), 1);
        assert_eq!(reply.agreements[0].last_hour, ENDS - 1);
        assert_eq!(reply.agreements[0].ended_by, ENDS, "truncated census keeps main's ending hour");
        let standing = asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": ["GER"], "only": "agreements" }), ENDS).unwrap();
        assert!(standing["agreements"].is_null());
        assert!(
            standing["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w.as_str().unwrap().ends_with(&format!("hour {ENDS} LendLeaseAgreements is truncated (2 missing rows)"))),
            "truncated standing agreements keep main's exact diagnostic: {standing}"
        );
    }

    #[test]
    fn ended_agreement_history_keeps_known_values_including_zero() {
        use crate::products::economy::market::history::LendLeaseHistory;
        use dashboard_base::products::{BranchParams, Product};
        let source = recording();
        let reply = LendLeaseHistory::load(&source, &BranchParams { branch_id: "1".into() }, &load(), &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.agreements.len(), 1);
        let ended = &reply.agreements[0];
        assert_eq!(ended.last_hour, ENDS - 1);
        assert_eq!(ended.ended_by, ENDS);
        assert_eq!(ended.terms.active_since.value, Some(FIRST));
        assert_eq!(ended.terms.active_since.missing, None);
        assert_eq!(ended.terms.last_delivery.value, Some(0));
        assert_eq!(ended.terms.last_delivery.missing, None);
        assert_eq!(ended.fuel_sent_raw.value_raw.as_deref(), Some(&((i64::from(ENDS - 1) * 32_768).to_string())[..]));
        assert_eq!(ended.fuel_sunk_raw.value_raw.as_deref(), Some("0"));
        assert_eq!(ended.fuel_sunk_raw.missing, None);
        assert!(ended.terms.monthly.items.as_ref().is_some_and(|items| items.len() == 1));
    }

    #[test]
    fn partial_standing_agreement_keeps_readable_fields_and_intact_siblings_on_both_paths() {
        use replay_columnar::{
            container::{SectionKey, Tags},
            envelope::{self, Identity, Kind},
            lane::{Clock, ClockHour},
            manifest::Status,
            state::Cell as StoreCell,
            store::Store,
            unit::{self, Encoder},
        };
        use replay_data::{
            format::FormatVersion,
            identity::{CaptureMode as Mode, Identity as Game, IdentityCodec, SampleBoundary, SessionStart, Verification},
            rows::TABLE_COUNT,
            value::Value as _,
        };
        use replay_frame::branches::{BranchReason, BranchStart, Parentage};
        use replay_journal::{columnar_file::{BlockEnd, FileWriter}, summaries::Summaries};
        use replay_recording::{records::Records, schema::{keeps_last, schema}};
        use replay_settings::compression::Settings;

        let game = Game {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "synthetic".into(),
            recorder_version: "synthetic".into(),
            collector_hash: [3; 32],
            verification: Verification::Unchecked,
        };
        let full_schema = schema().unwrap();
        let columns: Vec<usize> = full_schema.iter().map(Vec::len).collect();
        let mut content = Vec::new();
        game.encode(&mut content).unwrap();
        let identity = Identity {
            schema: envelope::schema_digest_prefix(&full_schema, &columns).unwrap(),
            campaign: CAMPAIGN.to_vec(),
            content,
            capture_mode: Mode::Full.byte(),
        };
        let unit = unit::Settings { restore_blocks: 1, ..unit::Settings::at(3) };
        let mut file = FileWriter::create(std::io::Cursor::new(Vec::new()), identity, TABLE_COUNT, Settings { level: unit.level }, None).unwrap();
        let mut store = Store::new(full_schema.clone(), keeps_last().to_vec());
        store.declare_logs(&replay_data::rows::logs()).unwrap();
        store.attach(Encoder::new(&full_schema, unit).unwrap());
        let mut summaries = Summaries::new(unit.level).unwrap();
        file.start_run(1, envelope::epoch::START, 110).unwrap();
        store.begin_run(110);
        summaries.start_run();
        summaries.open(&mut store, 110);
        let mut clock = Clock::default();
        let mut strings = Vec::new();
        let mut string_count = 0_u64;
        let mut intern = |store: &mut Store, text: &[u8]| -> u32 {
            let before = store.strings().next();
            let id = store.intern(text).unwrap();
            if store.strings().next() > before {
                unit::put_varint(&mut strings, text.len() as u64);
                strings.extend_from_slice(text);
                string_count += 1;
            }
            id
        };
        let ger_tag = intern(&mut store, b"GER");
        let fra_tag = intern(&mut store, b"FRA");
        let campaign_id = intern(&mut store, CAMPAIGN);

        let mut pool5 = Vec::new();
        MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 5, amount_raw: 50_000 }] }.encode(&mut pool5).unwrap();
        let mut pool6 = Vec::new();
        MarketPool { allow_zero: false, items: vec![MarketEquipment { variant: 6, amount_raw: 75_000 }] }.encode(&mut pool6).unwrap();
        let mut empty_pool = Vec::new();
        MarketPool::default().encode(&mut empty_pool).unwrap();

        for (hour, fuel_sent) in [(110, 32_768_i64), (111, 65_536_i64), (112, 65_536_i64)] {
            store.begin_hour(hour, &[]).unwrap();
            store.status(TableId::Clock.index(), Status::Complete, true, Some(hour), true).unwrap();
            {
                let mut t = store.table(TableId::Clock.index()).unwrap();
                let slot = t.slot(0).unwrap_or_else(|| t.insert(0).unwrap());
                t.write(slot, 0, StoreCell::I32(hour)).unwrap();
                if hour == 110 {
                    t.write(slot, 1, StoreCell::U64(0)).unwrap();
                    t.write(slot, 2, StoreCell::I64(0)).unwrap();
                    t.write(slot, 3, StoreCell::I32(0)).unwrap();
                    t.write(slot, 4, StoreCell::I32(0)).unwrap();
                    t.write(slot, 5, StoreCell::OptionalU32(None)).unwrap();
                    for col in 6..18 {
                        t.write(slot, col, StoreCell::OptionalI32(None)).unwrap();
                    }
                }
            }
            store.status(TableId::Campaign.index(), Status::Complete, true, Some(hour), true).unwrap();
            store.status(TableId::Countries.index(), Status::Complete, true, Some(hour), true).unwrap();
            store.status(TableId::CountryLinks.index(), Status::Complete, true, Some(hour), true).unwrap();
            store.status(TableId::TagNames.index(), Status::Complete, true, Some(hour), true).unwrap();
            store.status(TableId::TagHolders.index(), Status::Complete, true, Some(hour), true).unwrap();
            for empty in [TableId::LexicalTokens, TableId::VariantDefinitions, TableId::CountryMarket, TableId::MarketContractTerms, TableId::PurchaseContracts] {
                store.status(empty.index(), Status::Complete, true, Some(hour), true).unwrap();
            }
            if hour == 112 {
                store.status(TableId::LendLeaseAgreements.index(), Status::Truncated { missing: 1 }, true, None, true).unwrap();
                let mut t = store.table(TableId::LendLeaseAgreements.index()).unwrap();
                let partial_slot = t.slot(u128::from(GER) << 64 | 10u128).unwrap();
                t.read_rows(&[partial_slot]).unwrap();
            } else {
                store.status(TableId::LendLeaseAgreements.index(), Status::Complete, true, Some(hour), true).unwrap();
            }
            if hour == 110 {
                {
                    let mut t = store.table(TableId::Campaign.index()).unwrap();
                    let slot = t.insert(0).unwrap();
                    t.write(slot, 0, StoreCell::Bytes(campaign_id)).unwrap();
                }
                {
                    let mut t = store.table(TableId::TagNames.index()).unwrap();
                    for (number, tag) in [(1_u32, ger_tag), (2_u32, fra_tag)] {
                        let s = t.insert(u128::from(number)).unwrap();
                        t.write(s, 0, StoreCell::Bytes(tag)).unwrap();
                    }
                }
                {
                    let mut t = store.table(TableId::TagHolders.index()).unwrap();
                    for (number, epoch, canonical_slot, id, tag) in [(1_u32, 1_u32, 1_u32, GER, ger_tag), (2_u32, 2_u32, 3_u32, FRA, fra_tag)] {
                        let key = u128::from(number) << 96 | u128::from(epoch) << 64 | u128::from(canonical_slot) << 32 | u128::from(id);
                        let s = t.insert(key).unwrap();
                        t.write(s, 0, StoreCell::Bytes(tag)).unwrap();
                    }
                }
                {
                    let mut t = store.table(TableId::Countries.index()).unwrap();
                    for (id, canonical_slot, tag) in [(GER, 1_u32, ger_tag), (FRA, 3_u32, fra_tag)] {
                        let s = t.insert(u128::from(id)).unwrap();
                        t.write(s, 0, StoreCell::U32(canonical_slot)).unwrap();
                        t.write(s, 1, StoreCell::Bytes(tag)).unwrap();
                        t.write(s, 2, StoreCell::OptionalI64(None)).unwrap();
                        t.write(s, 3, StoreCell::I64(0)).unwrap();
                        t.write(s, 4, StoreCell::I64(0)).unwrap();
                        t.write(s, 5, StoreCell::I64(0)).unwrap();
                        t.write(s, 6, StoreCell::OptionalI64(None)).unwrap();
                        t.write(s, 7, StoreCell::U8(0)).unwrap();
                        t.write(s, 8, StoreCell::I32(0)).unwrap();
                        t.write(s, 9, StoreCell::OptionalI32(None)).unwrap();
                    }
                }
                {
                    let mut t = store.table(TableId::CountryLinks.index()).unwrap();
                    let s0 = t.insert(u128::from(GER)).unwrap();
                    t.write(s0, 0, StoreCell::U32(1)).unwrap();
                    let s1 = t.insert(u128::from(FRA)).unwrap();
                    t.write(s1, 0, StoreCell::U32(3)).unwrap();
                }
                {
                    let mut t = store.table(TableId::LendLeaseAgreements.index()).unwrap();
                    let intact = t.insert(u128::from(GER) << 64 | 9u128).unwrap();
                    t.write(intact, 0, StoreCell::U32(FRA)).unwrap();
                    t.write(intact, 1, StoreCell::U64(1)).unwrap();
                    t.write(intact, 2, StoreCell::I32(100)).unwrap();
                    t.write(intact, 3, StoreCell::I32(105)).unwrap();
                    t.write(intact, 4, StoreCell::Opaque(&pool5)).unwrap();
                    t.write(intact, 5, StoreCell::Opaque(&empty_pool)).unwrap();
                    t.write(intact, 6, StoreCell::Opaque(&empty_pool)).unwrap();
                    t.write(intact, 7, StoreCell::Opaque(&empty_pool)).unwrap();
                    t.write(intact, 8, StoreCell::Opaque(&empty_pool)).unwrap();
                    t.write(intact, 9, StoreCell::I64(1_000)).unwrap();
                    t.write(intact, 10, StoreCell::I64(0)).unwrap();
                    t.write(intact, 11, StoreCell::I64(fuel_sent)).unwrap();
                    t.write(intact, 12, StoreCell::I64(0)).unwrap();
                    t.write(intact, 13, StoreCell::I64(32_768)).unwrap();

                    let partial = t.insert(u128::from(GER) << 64 | 10u128).unwrap();
                    t.write(partial, 2, StoreCell::I32(102)).unwrap();
                    t.write(partial, 4, StoreCell::Opaque(&pool6)).unwrap();
                }
            } else if hour == 111 {
                let mut t = store.table(TableId::LendLeaseAgreements.index()).unwrap();
                let intact = t.slot(u128::from(GER) << 64 | 9u128).unwrap();
                t.write(intact, 11, StoreCell::I64(fuel_sent)).unwrap();
            }
            store.settle();
            summaries.hour(&store, hour);
            clock.hour(&ClockHour { flags: 0, total_ns: 0, pool_bytes: 0, dropped_frames: 0 });
            store.end_hour(&mut file).unwrap();
        }
        store.end_run(&mut file).unwrap();
        let mut string_payload = Vec::new();
        unit::put_varint(&mut string_payload, string_count);
        string_payload.extend_from_slice(&strings);
        let (summary_pieces, span) = summaries.finish(112, 0, 110, &mut file).unwrap();
        let (_, span) = file.end_block(BlockEnd { clock: &clock.finish(), strings: (&string_payload, 0), summaries: summary_pieces, span }).unwrap();
        summaries.written(&span).unwrap();
        store.end_block();
        let tail = summaries.tail(110, 3, &mut file).unwrap();
        file.tail(&tail).unwrap();
        file.finish_run().unwrap();

        let records = Records {
            sessions: vec![SessionStart { id: 1, process_id: 1, process_start_time: 1, qpc_frequency: 1, sample_boundary: SampleBoundary::EndOfHour, capture_mode: Mode::Full, identity: game }],
            branches: vec![BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: 110, parent: Parentage::Unknown, reason: BranchReason::SessionStart }],
            ..Records::default()
        };
        let record_bytes = records.encode(FormatVersion::CURRENT).unwrap();
        let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &record_bytes, 3).unwrap();
        file.container().append(SectionKey::RECORDS, extent).unwrap();
        let bytes = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();
        let replay = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "partial-lease".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();

        let params = json!({ "branchId": "1", "countries": ["GER"], "only": "agreements" });
        let reply = asked::<MarketLendLease>(&source, params.clone(), 110).unwrap();
        let mut registered = panel(&source, &params);
        let exact = registered.exact().expect("registered market panel runs incrementally");
        let read_110 = (exact.advance)(110, true, &AtomicBool::new(false), None);
        assert!(read_110.lanes.iter().any(|l| l.table == TableId::LendLeaseAgreements.index()), "registered panel declares LendLeaseAgreements lane");
        let at_110 = (*read_110.value).clone().unwrap();
        assert_eq!(at_110, reply, "registered incremental panel matches exact request including live cells");
        let read_111 = (exact.advance)(111, true, &AtomicBool::new(false), None);
        let at_111 = (*read_111.value).clone().unwrap();
        assert_eq!(without_live(at_111.clone()), without_live(reply.clone()), "live fuel update keeps unchanged terms");
        let read_112 = (exact.advance)(112, true, &AtomicBool::new(false), None);
        let at_112 = (*read_112.value).clone().unwrap();
        let asked_112 = asked::<MarketLendLease>(&source, params.clone(), 112).unwrap();
        assert_eq!(at_112, asked_112, "truncated hour with unread retained column matches on both paths");
        assert!(asked_112["agreements"].is_null());
        assert!(
            asked_112["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w.as_str().unwrap().ends_with("hour 112 LendLeaseAgreements is unavailable: LendLeaseAgreements.receiver is not recorded and its row cannot say so")),
            "truncated hour with unread required column keeps main's exact diagnostic: {asked_112}"
        );
        assert_eq!(followed(&source, &mut *registered, 110), Ok(reply.clone()), "followed panel matches exact reply on partial agreements");

        let agreements = reply["agreements"].as_array().unwrap();
        assert_eq!(agreements.len(), 2, "both intact and partial standing agreements survive");
        let intact = agreements.iter().find(|row| row["id"] == "0000000000000009").unwrap();
        let partial = agreements.iter().find(|row| row["id"] == "000000000000000a").unwrap();
        assert_eq!(intact["receiver"], "FRA");
        assert_eq!(intact["monthly"]["items"][0]["variant"], "5");
        assert!(partial["receiver"].is_null());
        assert_eq!(partial["receiverMissing"], "Receiver not recorded at this hour");
        assert!(partial["fuelDailyRaw"]["valueRaw"].is_null());
        assert!(partial["fuelDailyRaw"]["missing"].as_str().unwrap().contains("not recorded"));
        assert_eq!(partial["activeSince"]["value"], 102);
        assert_eq!(partial["monthly"]["items"][0]["variant"], "6", "readable pool survives unrecorded sibling fields");
        assert!(reply["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("unreadable or unrecorded fields")));

        let live = cells(&at_110);
        let intact_key = intact["key"].as_str().unwrap().to_owned();
        let partial_key = partial["key"].as_str().unwrap().to_owned();
        assert_eq!(live[&(intact_key.clone(), "fuelSent".into())], json!(32_768), "intact agreement keeps live fuelSent despite partial sibling");
        assert_eq!(live[&(intact_key.clone(), "fuelLost".into())], json!(0));
        assert!(live[&(partial_key.clone(), "fuelSent".into())].is_string(), "partial agreement's unrecorded live fuelSent reports a missing reason");
        let live_111 = cells(&at_111);
        assert_eq!(live_111[&(intact_key, "fuelSent".into())], json!(65_536), "exact panel updates live fuelSent across unchanged terms");
        assert!(live_111[&(partial_key, "fuelSent".into())].is_string());

        let fra_reply = asked::<MarketLendLease>(&source, json!({ "branchId": "1", "countries": ["FRA"], "only": "agreements" }), 110).unwrap();
        let fra_agreements = fra_reply["agreements"].as_array().unwrap();
        assert_eq!(fra_agreements.len(), 2, "FRA filter keeps intact FRA agreement and unattributed unread-receiver agreement");
        let fra_partial = fra_agreements.iter().find(|row| row["id"] == "000000000000000a").unwrap();
        assert!(fra_partial["receiver"].is_null());
        assert_eq!(fra_partial["receiverMissing"], "Receiver not recorded at this hour");
        assert_eq!(fra_reply["unattributed"], 1);
        assert!(fra_reply["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("no tag at this hour")));
    }
}