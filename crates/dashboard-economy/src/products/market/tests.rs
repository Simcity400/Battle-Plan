use super::*;
use crate::products::test_support::*;
use super::{Field, LEDGER_INDEX_READS, LEDGER_ROWS_READ};
use replay_data::rows::*;

fn pool(items: &[(u64, i64)]) -> SharedMarketPool {
    MarketPool {
        allow_zero: false,
        items: items
            .iter()
            .map(|&(variant, amount_raw)| MarketEquipment { variant, amount_raw })
            .collect(),
    }
    .into()
}

fn base(hour: i32) -> DecodedFrame {
    let mut at = frame(hour);
    countries(&mut at, &[(1, 10, "SEL"), (2, 20, "BUY"), (3, 30, "OTH")]);
    at
}

fn terms(seller: u32, buyer: u32, id: u64, kind: u8) -> MarketContractTermsRow {
    MarketContractTermsRow {
        country: seller,
        buyer,
        id,
        kind,
        speed: 3,
        equipment: pool(&[(0xa1, 500_000), (0xa2, 100_000)]),
        prices: pool(&[(0xa1, 2_000_000)]),
        ..Default::default()
    }
}

fn event(id: u64, kind: u8, hour: i32) -> MarketEventRow {
    MarketEventRow {
        id,
        occurrence: id,
        hour,
        agreement: 77,
        country: 1,
        receiver: 2,
        kind,
        variant: 0xa1,
        amount_raw: 100_000,
        ..Default::default()
    }
}

struct Published {
    fixture: dashboard_base::prepared::series::tests::Fixture,
}

fn publish(state: &mut Prepared) -> Published {
    let fixture = fixture();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    Published { fixture }
}

impl Published {
    fn history(&self, view: MarketView, tags: &[&str], through: i32) -> MarketHistoryReply {
        history(
            &self.fixture.store,
            &self.fixture.key,
            &MarketHistoryParams {
                branch_id: "1".into(),
                view,
                tags: tags.iter().map(|tag| tag.to_string()).collect(),
            },
            Some(through),
            None,
            Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    fn ledger(&self, tags: &[&str], before: Option<&str>, size: u32) -> MarketLedgerReply {
        self.ledger_at(tags, before, size, 100)
    }
    fn ledger_at(&self, tags: &[&str], before: Option<&str>, size: u32, hour: i32) -> MarketLedgerReply {
        ledger(
            &self.fixture.store,
            &self.fixture.key,
            &MarketLedgerParams {
                branch_id: "1".into(),
                tags: tags.iter().map(|tag| tag.to_string()).collect(),
                members: None,
                kinds: Vec::new(),
                last_hour: hour,
                before: before.map(str::to_owned),
                page_size: size,
            },
            Some(hour.max(100)),
            None,
            Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
}

#[test]
fn requests_and_contracts_stay_apart_with_ordered_pools_and_unknown_tables() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(
        &mut at,
        TableRows::MarketContractTerms(vec![terms(1, 2, 9, 0), terms(1, 2, 9, 1)]),
    );
    unavailable(&mut at, TableId::CountryMarket);
    state.step(&at);
    let published = publish(&mut state);
    let contracts = published.history(MarketView::Contracts, &[], 5);
    assert_eq!(contracts.contracts.len(), 1);
    let contract = &contracts.contracts[0];
    assert_eq!(contract.seller, "10");
    assert_eq!(contract.buyer[0].2, "20");
    let manifest: Vec<_> = contract.items.iter().filter(|item| item.pool == 0).collect();
    assert_eq!(manifest[0].variant, "a1");
    assert_eq!(manifest[1].amounts[0].2, "100000");
    assert_eq!(contracts.known, vec![EconomyRun(5, 5, "1".into())]);
    assert_eq!(published.history(MarketView::Requests, &[], 5).contracts.len(), 1);
    // The unrecorded inventory is unknown, not an empty market.
    let inventory = published.history(MarketView::Inventory, &[], 5);
    assert!(inventory.known.is_empty());
    assert!(inventory.warnings.iter().any(|line| line.contains("market inventory unavailable")));
}

#[test]
fn chosen_countries_keep_rows_they_sell_or_buy_in() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(
        &mut at,
        TableRows::MarketContractTerms(vec![terms(1, 2, 9, 1), terms(3, 3, 8, 1)]),
    );
    state.step(&at);
    let published = publish(&mut state);
    let bought = published.history(MarketView::Contracts, &["BUY"], 5);
    assert_eq!(bought.contracts.len(), 1);
    assert_eq!(bought.contracts[0].id, "0000000000000009");
    assert!(bought.holders.contains_key("20"));
}

#[test]
fn purchase_progress_joins_only_while_its_parties_match() {
    let mut state = Prepared::default();
    for (hour, buyer) in [(5, 2), (6, 3)] {
        let mut at = base(hour);
        set(&mut at, TableRows::MarketContractTerms(vec![terms(1, 2, 9, 1)]));
        set(
            &mut at,
            TableRows::PurchaseContracts(vec![PurchaseContractRow {
                id: 9,
                seller: 1,
                buyer,
                convoys_assigned: 2,
                convoys_total: 4,
                collected_raw: 300_000,
                completed_deliveries: 1,
                ..Default::default()
            }]),
        );
        state.step(&at);
    }
    let published = publish(&mut state);
    let contract = &published.history(MarketView::Contracts, &[], 6).contracts[0];
    assert_eq!(contract.purchase[0], vec![EconomyRun(5, 5, "2".into())]);
    assert_eq!(contract.purchase[2], vec![EconomyRun(5, 5, "300000".into())]);
}

#[test]
fn purchase_progress_joins_only_the_buyer_the_contract_records_at_that_hour() {
    let mut state = Prepared::default();
    // The contract's buyer is BUY at 5 and OTH from 6; the purchase names BUY
    // at both hours, so only hour 5 joins.
    for (hour, contract_buyer) in [(5, 2), (6, 3)] {
        let mut at = base(hour);
        set(&mut at, TableRows::MarketContractTerms(vec![terms(1, contract_buyer, 9, 1)]));
        set(
            &mut at,
            TableRows::PurchaseContracts(vec![PurchaseContractRow {
                id: 9,
                seller: 1,
                buyer: 2,
                convoys_assigned: 1,
                ..Default::default()
            }]),
        );
        state.step(&at);
    }
    let published = publish(&mut state);
    let contract = &published.history(MarketView::Contracts, &[], 6).contracts[0];
    assert_eq!(contract.purchase[0], vec![EconomyRun(5, 5, "1".into())]);
}

#[test]
fn agreements_keep_schedules_retained_totals_and_fuel_exact() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(
        &mut at,
        TableRows::LendLeaseAgreements(vec![LendLeaseAgreementRow {
            country: 1,
            id: 4,
            receiver: 2,
            active_since: 1_000,
            last_delivery: 1_200,
            monthly: pool(&[(0xa1, 6_000_000)]),
            delivered: pool(&[(0xa1, 12_000_000)]),
            sunk: pool(&[(0xa1, 0)]),
            fuel_daily_raw: 32_769,
            ..Default::default()
        }]),
    );
    state.step(&at);
    let published = publish(&mut state);
    let reply = published.history(MarketView::Agreements, &["BUY"], 5);
    let agreement = &reply.agreements[0];
    assert_eq!(agreement.giver, "10");
    assert_eq!(agreement.receiver[0].2, "20");
    assert_eq!(agreement.since[0].2, "1000");
    assert_eq!(agreement.fuel[0][0].2, "32769");
    let pools: Vec<_> = agreement.items.iter().map(|item| item.pool).collect();
    assert_eq!(pools, vec![0, 3, 4]);
    // A known zero loss is a value.
    assert_eq!(agreement.items[2].amounts[0].2, "0");
}

#[test]
fn the_ledger_keeps_each_occurrence_once_newest_first_with_named_refusals() {
    let mut state = Prepared::default();
    for hour in 5..=7 {
        let mut at = base(hour);
        let mut rows = vec![event(u64::from(hour as u32) * 10, 21, hour)];
        if hour == 6 {
            // Carried from the hour before: kept once.
            rows.push(event(50, 21, 5));
            rows.push(MarketEventRow {
                kind: 34,
                mode: 20,
                ordinal: 1,
                ..event(61, 34, 6)
            });
            // A lend-lease occurrence belongs to the agreements, not here.
            rows.push(event(62, 3, 6));
        }
        set(&mut at, TableRows::MarketEvents(rows));
        state.step(&at);
    }
    let published = publish(&mut state);
    let page = published.ledger(&["BUY"], None, 2);
    assert_eq!(page.total, None);
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.rows[0].id, "70");
    assert_eq!(page.rows[1].kind, 34);
    assert_eq!(page.rows[1].amount_raw, None);
    assert!(page.rows[1].reason.as_deref().unwrap().contains("unavailable"));
    let older = published.ledger(&["BUY"], page.next.as_deref(), 2);
    assert_eq!(
        older.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec!["60", "50"]
    );
    assert_eq!(older.next, None);
    // Seller and buyer both chosen: still once.
    assert_eq!(published.ledger(&["SEL", "BUY"], None, 10).rows.len(), 4);
    assert_eq!(published.ledger(&["OTH"], None, 10).rows.len(), 0);
    assert_eq!(published.ledger(&[], None, 10).rows.len(), 4);
    // The occurrences were read at every hour: an empty page there is no trades.
    assert_eq!(page.known, vec![EconomyRun(5, 7, "1".into())]);
}

#[test]
fn every_country_pages_all_kinds_newest_first_with_an_exact_total() {
    let mut state = Prepared::default();
    for hour in 5..=8 {
        let mut at = base(hour);
        let kind = if hour % 2 == 0 { 20 } else { 21 };
        set(&mut at, TableRows::MarketEvents(vec![event(u64::from(hour as u32), kind, hour)]));
        state.step(&at);
    }
    let published = publish(&mut state);
    let first = published.ledger(&[], None, 3);
    assert_eq!(first.total.as_deref(), Some("4"));
    assert_eq!(first.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), vec!["8", "7", "6"]);
    let older = published.ledger(&[], first.next.as_deref(), 3);
    assert_eq!(older.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), vec!["5"]);
    assert_eq!(older.next, None);
    // Cut mid-history: the total and pages stop at the hour shown.
    let cut = published.ledger_at(&[], None, 10, 6);
    assert_eq!(cut.total.as_deref(), Some("2"));
    assert_eq!(cut.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), vec!["6", "5"]);
}

#[test]
fn a_page_decodes_only_the_chunks_it_reads_however_long_the_trade() {
    use dashboard_base::prepared::series::decoded_event_rows;
    const HOURS: i32 = 6_000;
    let mut state = Prepared::default();
    for hour in 0..HOURS {
        let mut at = base(hour);
        set(&mut at, TableRows::MarketEvents(vec![event(u64::from(hour as u32), 21, hour)]));
        state.step(&at);
    }
    // One stream and two indexes of 6,000 entries each, split into chunks.
    let chunks = |party: bool| {
        state
            .keyed
            .meta
            .fields
            .values()
            .filter(|field| if party { matches!(field, Field::Party(10, ..)) } else { matches!(field, Field::Stream(..)) })
            .count()
    };
    assert_eq!((chunks(false), chunks(true)), (12, 12));
    let published = publish(&mut state);
    // A cold first page decodes the chunks its index entries and rows sit in,
    // never the 18,000 entries the trade holds.
    let before = decoded_event_rows();
    let page = published.ledger_at(&["SEL"], None, 50, HOURS);
    let decoded = decoded_event_rows() - before;
    assert_eq!(page.rows.len(), 50);
    assert_eq!(page.rows[0].hour, HOURS - 1);
    assert!(decoded <= 4 * CHUNK, "a page decoded {decoded} entries");
    // The next page, and a page cut mid-history, are as bounded.
    let before = decoded_event_rows();
    let older = published.ledger_at(&["SEL"], page.next.as_deref(), 50, HOURS);
    assert_eq!(older.rows[0].hour, HOURS - 51);
    let cut = published.ledger_at(&["SEL"], None, 50, HOURS / 2);
    assert_eq!(cut.rows[0].hour, HOURS / 2);
    assert_eq!(cut.rows.last().unwrap().hour, HOURS / 2 - 49);
    let every = published.ledger_at(&[], None, 50, HOURS / 2);
    assert_eq!(every.total.as_deref(), Some("3001"));
    let decoded = decoded_event_rows() - before;
    assert!(decoded <= 12 * CHUNK, "three pages decoded {decoded} entries");
}

#[test]
fn an_unread_market_ledger_has_no_known_hours() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(&mut at, TableRows::MarketEvents(vec![event(5, 21, 5)]));
    state.step(&at);
    let mut at = base(6);
    unavailable(&mut at, TableId::MarketEvents);
    state.step(&at);
    let published = publish(&mut state);
    // Hour 6 is unknown, not a market without trades.
    assert_eq!(published.ledger(&["OTH"], None, 10).known, vec![EconomyRun(5, 5, "1".into())]);
}

#[test]
fn a_ledger_of_both_parties_pages_each_occurrence_once() {
    let mut state = Prepared::default();
    for hour in 5..=7 {
        let mut at = base(hour);
        set(&mut at, TableRows::MarketEvents(vec![event(u64::from(hour as u32), 21, hour)]));
        state.step(&at);
    }
    let published = publish(&mut state);
    let first = published.ledger(&["SEL", "BUY"], None, 1);
    assert_eq!(first.total, None);
    let mut ids = vec![first.rows[0].id.clone()];
    let mut next = first.next.clone();
    while let Some(before) = next {
        let page = published.ledger(&["SEL", "BUY"], Some(&before), 1);
        assert_eq!(page.rows.len(), 1);
        ids.push(page.rows[0].id.clone());
        next = page.next;
    }
    assert_eq!(ids, vec!["7", "6", "5"]);
}

#[test]
fn a_ledger_matches_a_party_by_its_tag_at_the_occurrence_hour() {
    let mut state = Prepared::default();
    for hour in 5..=7 {
        let mut at = frame(hour);
        // Holder 10 is SEL until 5, then renamed NEW; holder 30 takes SEL from 6.
        let (old, new) = if hour <= 5 { ("SEL", "OTH") } else { ("NEW", "SEL") };
        countries(&mut at, &[(1, 10, old), (2, 20, "BUY"), (3, 30, new)]);
        set(&mut at, TableRows::MarketEvents(vec![event(u64::from(hour as u32), 21, hour)]));
        state.step(&at);
    }
    let published = publish(&mut state);
    // The occurrences are holder 10's; it had SEL only at hour 5.
    let sel = published.ledger(&["SEL"], None, 10);
    assert_eq!(sel.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), vec!["5"]);
    assert_eq!(sel.next, None);
    assert_eq!(published.ledger(&["NEW"], None, 10).rows.len(), 2);
}

#[test]
fn a_chosen_group_pages_by_its_rows_and_segment_heads_at_campaign_scale() {
    const COUNTRIES: u32 = 1_500;
    const HOURS: i32 = 2_000;
    const GROUP: u32 = 40;
    let mut state = Prepared::default();
    for hour in 0..HOURS {
        let mut at = frame(hour);
        many_countries(&mut at, COUNTRIES);
        let n = hour as u32;
        // One sale a group member makes to someone outside it, and one trade
        // the group has no part in.
        let ours = MarketEventRow {
            country: n % GROUP + 1,
            receiver: GROUP + n % (COUNTRIES - GROUP) + 1,
            ..event(u64::from(n) * 2, 21, hour)
        };
        let theirs = MarketEventRow {
            country: GROUP + (n * 7) % (COUNTRIES - GROUP) + 1,
            receiver: GROUP + (n * 13 + 1) % (COUNTRIES - GROUP) + 1,
            ..event(u64::from(n) * 2 + 1, 21, hour)
        };
        set(&mut at, TableRows::MarketEvents(vec![ours, theirs]));
        state.step(&at);
    }
    let published = publish(&mut state);
    let tags: Vec<String> = (0..GROUP).map(|slot| format!("T{slot:04}")).collect();
    let ask = |before: Option<String>| {
        ledger(
            &published.fixture.store,
            &published.fixture.key,
            &MarketLedgerParams {
                branch_id: "1".into(),
                tags: Vec::new(),
                members: Some(
                    tags.iter()
                        .map(|tag| LedgerMember { tag: tag.clone(), first_hour: 0, last_hour: HOURS })
                        .collect(),
                ),
                kinds: Vec::new(),
                last_hour: HOURS,
                before,
                page_size: 50,
            },
            Some(HOURS),
            None,
            Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    LEDGER_ROWS_READ.with(|read| read.set(0));
    LEDGER_INDEX_READS.with(|read| read.set(0));
    let first = ask(None);
    assert_eq!(first.rows.len(), 50);
    assert_eq!(first.rows[0].hour, HOURS - 1);
    assert_eq!(first.total, None);
    // Fifty rows, and per segment a count and a head, plus the next head of
    // each entry consumed: never the group's 2,000 index entries.
    assert_eq!(LEDGER_ROWS_READ.with(|read| read.get()), 50);
    let reads = LEDGER_INDEX_READS.with(|read| read.get());
    assert!(reads <= 2 * GROUP as usize + 50, "{reads} index entries read for one page");
    // Paging on covers the group's occurrences exactly once, newest first.
    let mut hours = first.rows.iter().map(|row| row.hour).collect::<Vec<_>>();
    let mut next = first.next;
    while let Some(before) = next {
        let page = ask(Some(before));
        hours.extend(page.rows.iter().map(|row| row.hour));
        next = page.next;
    }
    assert_eq!(hours, (0..HOURS).rev().collect::<Vec<_>>());
}

#[test]
fn a_ledger_cursor_keeps_paging_older_rows_after_later_occurrences() {
    let mut state = Prepared::default();
    for hour in 5..=7 {
        let mut at = base(hour);
        set(&mut at, TableRows::MarketEvents(vec![event(u64::from(hour as u32), 21, hour)]));
        state.step(&at);
    }
    let published = publish(&mut state);
    let first = published.ledger(&["BUY", "OTH"], None, 1);
    assert_eq!(first.rows[0].id, "7");
    // Later, OTH trades for the first time and BUY trades again.
    let mut at = base(8);
    set(
        &mut at,
        TableRows::MarketEvents(vec![MarketEventRow { country: 3, ..event(8, 21, 8) }]),
    );
    state.step(&at);
    let mut writer = published.fixture.writer();
    state.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    assert_eq!(published.ledger(&["BUY", "OTH"], None, 1).rows[0].id, "8");
    let later = published.ledger(&["BUY", "OTH"], first.next.as_deref(), 10);
    assert_eq!(later.rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), vec!["6", "5"]);
    assert_eq!(later.next, None);
}

#[test]
fn ledger_cursors_name_each_segment_and_its_position() {
    let params = |tags: &[&str], before: &str| MarketLedgerParams {
        branch_id: "1".into(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
        members: None,
        kinds: Vec::new(),
        last_hour: 10,
        before: Some(before.into()),
        page_size: 10,
    };
    assert!(MarketLedger::validate(&params(&[], "4:-2147483648:6")).is_ok());
    assert!(MarketLedger::validate(&params(&["BUY"], "4:-5:6.7:8:9")).is_ok());
    assert!(MarketLedger::validate(&params(&[], "12")).is_err());
    assert!(MarketLedger::validate(&params(&["BUY"], "4:5")).is_err());
    assert!(MarketLedger::validate(&params(&["BUY"], "")).is_err());
}

#[test]
fn partner_turnover_adds_keys_per_holder_not_per_pair_and_idle_hours_publish_little() {
    let mut state = Prepared::default();
    let mut at = frame(5);
    many_countries(&mut at, 60);
    // Thirty sellers each trade with thirty buyers: 900 pairs.
    let rows: Vec<_> = (0..900)
        .map(|n| MarketEventRow {
            id: n,
            occurrence: n,
            hour: 5,
            agreement: n,
            country: (n / 30) as u32 + 1,
            receiver: (n % 30) as u32 + 31,
            kind: 21,
            amount_raw: 1,
            ..Default::default()
        })
        .collect();
    set(&mut at, TableRows::MarketEvents(rows));
    state.step(&at);
    let parties = state.keyed.meta.fields.values().filter(|field| matches!(field, Field::Party(..))).count();
    let streams = state.keyed.meta.fields.values().filter(|field| matches!(field, Field::Stream(..))).count();
    // Each holder's thirty occurrences fit one index key; the 900 rows take two
    // stream chunks.
    assert_eq!((parties, streams), (60, 2));
    let fixture = fixture();
    let flag = AtomicBool::new(false);
    let size = |fixture: &dashboard_base::prepared::series::tests::Fixture| -> u64 {
        fn walk(path: &std::path::Path) -> u64 {
            std::fs::read_dir(path).map_or(0, |entries| entries.flatten().map(|entry| {
                let path = entry.path();
                if path.is_dir() { walk(&path) } else { entry.metadata().map_or(0, |meta| meta.len()) }
            }).sum())
        }
        walk(&fixture.directory)
    };
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let busy = size(&fixture);
    // A later idle hour republishes no occurrence history.
    let mut at = frame(6);
    many_countries(&mut at, 60);
    state.step(&at);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let idle = size(&fixture) - busy;
    assert!(idle * 4 < busy, "an idle flush wrote {idle} bytes after {busy}");
}

#[test]
fn a_group_ledger_lists_members_only_while_members() {
    let mut state = Prepared::default();
    for hour in 5..=9 {
        let mut at = base(hour);
        set(&mut at, TableRows::MarketEvents(vec![event(u64::from(hour as u32), 21, hour)]));
        state.step(&at);
    }
    let published = publish(&mut state);
    let group = |members: Vec<LedgerMember>| {
        ledger(
            &published.fixture.store,
            &published.fixture.key,
            &MarketLedgerParams {
                branch_id: "1".into(),
                tags: Vec::new(),
                members: Some(members),
                kinds: Vec::new(),
                last_hour: 100,
                before: None,
                page_size: 10,
            },
            Some(100),
            None,
            Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let ids = |reply: &MarketLedgerReply| reply.rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
    // SEL was a member 6–7 and BUY from 9: hour 8 is nobody's.
    let member = |tag: &str, first_hour, last_hour| LedgerMember { tag: tag.into(), first_hour, last_hour };
    let reply = group(vec![member("SEL", 6, 7), member("BUY", 9, 9)]);
    assert_eq!(ids(&reply), vec!["9", "7", "6"]);
    assert_eq!(reply.total, None);
    // Overlapping memberships of one country list each occurrence once.
    assert_eq!(ids(&group(vec![member("SEL", 5, 7), member("SEL", 6, 9)])), vec!["9", "8", "7", "6", "5"]);
    // A member that never held its tag then lists nothing, never every country.
    assert!(group(vec![member("OTH", 5, 9)]).rows.is_empty());
    let params = MarketLedgerParams {
        branch_id: "1".into(),
        tags: Vec::new(),
        members: Some(vec![member("SEL", 7, 6)]),
        kinds: Vec::new(),
        last_hour: 10,
        before: None,
        page_size: 10,
    };
    assert!(MarketLedger::validate(&params).is_err());
}

/// Observed by its changes, with quiet hours before supplying variant reads, it publishes what whole hours do.
#[test]
fn observing_by_changes_publishes_what_observing_every_hour_does() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, series::tests::assert_payloads_equal};
    use replay_frame::frame::{FrameDecoder, FrameEncoder, Metrics, TableView};
    let format = FormatVersion::CURRENT;
    let supplying = |hour: i32| {
        dashboard_base::metadata::slot_read(format, &Metrics::default(), TableId::VariantDefinitions, hour)
    };
    let at = |hour: i32| {
        let mut at = base(hour);
        // Balances move every fifth hour; identities never.
        if let TableView::Complete(TableRows::Countries(rows)) = &mut *at.tables[TableId::Countries.index()] {
            for row in rows.iter_mut() {
                row.political_power_raw = Some(i64::from(hour / 5) * 10 + i64::from(row.id));
            }
        }
        let mut contracts = vec![terms(1, 2, 7, 0), terms(1, 2, 9, 1)];
        if hour >= 6 {
            contracts.push(terms(2, 3, 8, 1));
        }
        if hour >= 14 {
            contracts.retain(|row| row.id != 7);
        }
        contracts.sort_by_key(|row| (row.country, row.id, row.kind));
        set(&mut at, TableRows::MarketContractTerms(contracts));
        // The design changes only at the variant table's supplying reads.
        let reads = (0..=hour).filter(|&hour| supplying(hour)).count();
        set(
            &mut at,
            TableRows::VariantDefinitions(vec![
                VariantDefinitionRow { id: 0xa1, creator_link: 1, obsolete: (reads % 2) as u8, ..Default::default() },
                VariantDefinitionRow { id: 0xa2, creator_link: 2, ..Default::default() },
            ]),
        );
        at
    };
    let blocks: Vec<Vec<i32>> = (0..4).map(|block| (block * 20..(block * 20 + 20).min(72)).collect()).collect();
    // A supplying read after an hour whose countries and variants did not change.
    assert!(
        blocks.iter().any(|hours| hours.windows(3).any(|hours| {
            supplying(hours[2]) && hours[1] % 5 != 0 && hours[2] % 5 != 0 && !supplying(hours[1])
        })),
        "no supplying read follows a quiet hour"
    );
    let (mut changed, mut whole) = (Prepared::default(), Prepared::default());
    let (published, expected) = (Published { fixture: fixture() }, Published { fixture: fixture() });
    for (block, hours) in blocks.iter().enumerate() {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut tracker = FrameChangeTracker::new();
        for (index, &hour) in hours.iter().enumerate() {
            let wire = encoder.encode(at(hour), index == 0).unwrap();
            let decoded = decoder.advance(&wire).unwrap().clone();
            let changes = tracker.observe_updates(&decoded, std::sync::Arc::new(wire), format);
            changed.observe_changes(format, &decoded, &changes).unwrap();
            whole.step(&decoded);
        }
        for (state, published) in [(&mut changed, &published), (&mut whole, &expected)] {
            let mut writer = published.fixture.writer();
            state.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &published.fixture.payloads(),
            &expected.fixture.payloads(),
            &format!("market block {block}"),
        );
        let through = *hours.last().unwrap();
        for view in [MarketView::Contracts, MarketView::Requests, MarketView::Inventory, MarketView::Agreements] {
            for tags in [&[][..], &["SEL"][..]] {
                assert_eq!(
                    serde_json::to_string(&published.history(view, tags, through)).unwrap(),
                    serde_json::to_string(&expected.history(view, tags, through)).unwrap(),
                    "block {block} {view:?} {tags:?}"
                );
            }
        }
        changed = serde_json::from_slice(&serde_json::to_vec(&changed).unwrap()).unwrap();
    }
}
