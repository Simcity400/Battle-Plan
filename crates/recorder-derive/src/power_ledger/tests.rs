use super::*;
use crate::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::{CountryCommandPowerRow, CountryLinkRow, CountryPowerDailyRow, CountryRow, PowerDailyCostKind, column};

const TEXTS: &[&[u8]] = &[b"", b"not read"];
const Q: i64 = 100_000;

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(PowerLedger::default())]);
    writers.start_run();
    writers
}

fn idea(country: u32, sequence: u64, hour: i32, token: u32, amount: i64) -> CountryPowerChangeRow {
    CountryPowerChangeRow {
        country,
        sequence,
        epoch: 1,
        hour,
        currency: PowerCurrency::Political,
        operation: PowerOperation::Subtract,
        balance_before_raw: 10 * Q,
        balance_after_raw: 10 * Q - amount,
        requested_delta_raw: Some(-amount),
        cause: PowerCause::Idea,
        site: PowerSite::AddPoliticalPower,
        identity: PowerIdentity { item_token: Some(token), ..PowerIdentity::default() },
        ..Default::default()
    }
}

fn hold(sequence: u64, hour: i32, cause: PowerCause, site: PowerSite, requested: i64) -> CountryPowerChangeRow {
    CountryPowerChangeRow {
        country: 1,
        sequence,
        epoch: 1,
        hour,
        currency: PowerCurrency::Command,
        operation: if requested < 0 { PowerOperation::Subtract } else { PowerOperation::Add },
        requested_delta_raw: Some(requested),
        cause,
        site,
        identity: PowerIdentity { object_kind: PowerObjectKind::Leader, object_reference: Some(9), character: Some(4), ..PowerIdentity::default() },
        ..Default::default()
    }
}

fn daily(day_hour: i32, gross: i64, cost: i64, sequence: u64) -> CountryPowerDailyRow {
    CountryPowerDailyRow {
        country: 1,
        epoch: 1,
        day_hour,
        pp_gross_gain_raw: Some(gross),
        pp_cost_raw: Some(cost),
        pp_net_gain_raw: Some(gross - cost),
        pp_realized_delta_raw: Some(gross - cost),
        pp_sequence: Some(sequence),
        ..Default::default()
    }
}

fn cost(day_hour: i32, ordinal: u32, component: PowerDailyCostKind, amount: i64, sequence: u64) -> CountryPowerDailyCostRow {
    CountryPowerDailyCostRow {
        country: 1,
        ordinal,
        epoch: 1,
        day_hour,
        pp_sequence: sequence,
        component,
        cost_raw: amount,
        cause: PowerCause::DailyGain,
        site: PowerSite::AddPoliticalPower,
        identity: PowerIdentity { target_country: Some(2), ..PowerIdentity::default() },
        ..Default::default()
    }
}

const NO_CAPTURE: i32 = 30;
const WIDER: i32 = 50;
const SHORTER: i32 = 51;
const EMPTIED: i32 = 52;
const RESTORED: i32 = 53;
const REMOVED: i32 = 54;
const ZEROED: i32 = 55;
const LATER_DAY: i32 = 73;
const CLOSED_DELETED: i32 = 97;
const SAME_ITEM: i32 = 98;
const SAME_ITEM_DROPPED: i32 = 99;
const SAME_ITEM_RESTORED: i32 = 100;
const SAME_ITEM_UNRECONCILED: i32 = 101;
const UNLINKED: i32 = 102;
const LINKED_DELETED: i32 = 103;
const SHARED: i32 = 104;
const SHARED_PUBLISHED: i32 = 105;
const SHARED_DELETED: i32 = 106;
const REPUBLISHED: i32 = 107;
const OTHER_PUBLISHED: i32 = 108;
const UNSHARED_DELETED: i32 = 109;
const NO_CHANGES: i32 = 26;

fn edit(store: &mut Store, hour: i32) {
    let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
    if hour == 0 {
        for table in [TableId::Countries, TableId::CountryLinks, TableId::CountryPowerDaily, TableId::CountryPowerDailyCosts, TableId::CountryCommandPower] {
            complete(store, table);
        }
        for (id, slot) in [(1, 10), (2, 20)] {
            testing::put(store, TableId::Countries, &CountryRow { id, canonical_slot: slot, ..Default::default() }, TEXTS).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: slot }, TEXTS).unwrap();
            testing::put(store, TableId::CountryCommandPower, &CountryCommandPowerRow { canonical_slot: slot, ..Default::default() }, TEXTS).unwrap();
        }
    }
    if hour == NO_CAPTURE {
        store.status(TableId::PoliticsCaptureCoverage.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
    } else {
        complete(store, TableId::PoliticsCaptureCoverage);
        for country in [1, 2] {
            let since_hour = if hour >= SAME_ITEM { SAME_ITEM - 2 } else if hour >= SHORTER { hour - 1 } else { 0 };
            let witness = Streams { country, epoch: 1, since_hour, through_hour: hour, streams: Streams::ALL_STREAMS, ..Default::default() };
            testing::put(store, TableId::PoliticsCaptureCoverage, &witness, TEXTS).unwrap();
        }
    }
    let held = testing::rows(store, TableId::CountryPowerChanges);
    for (key, _) in held {
        testing::remove(store, TableId::CountryPowerChanges, key).unwrap();
    }
    if hour == NO_CHANGES {
        store.status(TableId::CountryPowerChanges.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
        return;
    }
    complete(store, TableId::CountryPowerChanges);
    let changes = match hour {
        0 => vec![idea(1, 10, 0, 7, Q)],
        1 => {
            let focus = CountryPowerChangeRow { operation: PowerOperation::Add, requested_delta_raw: Some(Q), cause: PowerCause::Focus, ..idea(1, 30, 1, 9, 0) };
            vec![idea(1, 15, 1, 7, 2 * Q), idea(1, 20, 1, 8, Q), focus, idea(3, 5, 1, 7, Q)]
        }
        2 => vec![hold(40, 2, PowerCause::Allocation, PowerSite::AllocateCommandPower, -2 * Q)],
        3 => vec![hold(45, 3, PowerCause::AllocationRefund, PowerSite::ReleaseCommandPower, 2 * Q)],
        25 => vec![idea(1, 60, 25, 7, Q)],
        49 => vec![idea(1, 70, 49, 7, 2 * Q)],
        _ => Vec::new(),
    };
    for row in &changes {
        testing::put(store, TableId::CountryPowerChanges, row, TEXTS).unwrap();
    }
    match hour {
        0 => {
            testing::put(store, TableId::CountryPowerDaily, &daily(0, 3 * Q, Q, 5), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(0, 0, PowerDailyCostKind::Relation, Q, 5), TEXTS).unwrap();
        }
        2 => testing::put(store, TableId::CountryCommandPower, &CountryCommandPowerRow { canonical_slot: 10, allocated_command_power_raw: 2 * Q, ..Default::default() }, TEXTS).unwrap(),
        24 => {
            testing::put(store, TableId::CountryPowerDaily, &daily(24, 3 * Q, Q, 50), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q, 50), TEXTS).unwrap();
        }
        25 => {
            testing::put(store, TableId::CountryPowerDaily, &daily(24, 3 * Q + Q / 10, Q + Q / 10, 50), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 1, PowerDailyCostKind::MioResearch, Q / 10, 50), TEXTS).unwrap();
        }
        28 => testing::put(store, TableId::CountryPowerDaily, &daily(24, 4 * Q, 2 * Q, 50), TEXTS).unwrap(),
        29 => testing::put(store, TableId::CountryPowerDaily, &daily(24, 3 * Q + Q / 10, Q + Q / 10, 50), TEXTS).unwrap(),
        48 => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            testing::put(store, TableId::CountryPowerDaily, &daily(48, 2 * Q, Q / 2, 80), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap();
        }
        WIDER => {
            testing::put(store, TableId::CountryPowerDaily, &daily(48, 2 * Q + Q / 10, Q / 2 + Q / 10, 80), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 1, PowerDailyCostKind::MioResearch, Q / 10, 80), TEXTS).unwrap();
        }
        SHORTER => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            testing::put(store, TableId::CountryPowerDaily, &daily(48, 2 * Q, Q / 2, 80), TEXTS).unwrap();
        }
        EMPTIED => testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap(),
        RESTORED => testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap(),
        REMOVED => testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap(),
        ZEROED => testing::put(store, TableId::CountryPowerDaily, &daily(48, 2 * Q, 0, 80), TEXTS).unwrap(),
        LATER_DAY => {
            testing::put(store, TableId::CountryPowerDaily, &daily(72, 3 * Q, Q / 2 + Q / 10, 90), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 0, PowerDailyCostKind::Relation, Q / 2, 90), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 1, PowerDailyCostKind::MioResearch, Q / 10, 90), TEXTS).unwrap();
        }
        CLOSED_DELETED => testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap(),
        SAME_ITEM => {
            testing::put(store, TableId::CountryPowerDaily, &daily(96, 3 * Q, Q / 2 + Q / 4, 99), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(96, 0, PowerDailyCostKind::Relation, Q / 2, 99), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(96, 1, PowerDailyCostKind::Relation, Q / 4, 99), TEXTS).unwrap();
        }
        SAME_ITEM_DROPPED => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            testing::put(store, TableId::CountryPowerDaily, &daily(96, 3 * Q, Q / 2, 99), TEXTS).unwrap();
        }
        SAME_ITEM_RESTORED => {
            testing::put(store, TableId::CountryPowerDaily, &daily(96, 3 * Q, Q / 2 + Q / 4, 99), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(96, 1, PowerDailyCostKind::Relation, Q / 4, 99), TEXTS).unwrap();
        }
        SAME_ITEM_UNRECONCILED => testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap(),
        UNLINKED => {
            testing::put(store, TableId::CountryPowerDaily, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2 + Q / 10, 7) }, TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(96, 0, PowerDailyCostKind::Relation, Q / 2, 7) }, TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(96, 1, PowerDailyCostKind::MioResearch, Q / 10, 7) }, TEXTS).unwrap();
        }
        LINKED_DELETED => {
            testing::put(store, TableId::Countries, &CountryRow { id: 3, canonical_slot: 30, ..Default::default() }, TEXTS).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 3, canonical_slot: 30 }, TEXTS).unwrap();
            testing::remove(store, TableId::CountryPowerDailyCosts, 3 << 32 | 1).unwrap();
        }
        SHARED => {
            testing::put(store, TableId::Countries, &CountryRow { id: 4, canonical_slot: 30, ..Default::default() }, TEXTS).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 4, canonical_slot: 30 }, TEXTS).unwrap();
        }
        SHARED_PUBLISHED => {
            testing::put(store, TableId::CountryPowerDaily, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2 + Q / 10, 8) }, TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(96, 0, PowerDailyCostKind::Relation, Q / 2, 8) }, TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(96, 1, PowerDailyCostKind::MioResearch, Q / 10, 8) }, TEXTS).unwrap();
        }
        SHARED_DELETED => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 3 << 32 | 1).unwrap();
            testing::put(store, TableId::CountryPowerDaily, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2, 8) }, TEXTS).unwrap();
        }
        REPUBLISHED => {
            testing::put(store, TableId::CountryPowerDaily, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2 + Q / 10, 8) }, TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(96, 1, PowerDailyCostKind::MioResearch, Q / 10, 8) }, TEXTS).unwrap();
        }
        OTHER_PUBLISHED => {
            testing::put(store, TableId::CountryPowerDaily, &CountryPowerDailyRow { country: 4, ..daily(96, 3 * Q, Q / 4, 9) }, TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 4, ..cost(96, 0, PowerDailyCostKind::Relation, Q / 4, 9) }, TEXTS).unwrap();
        }
        UNSHARED_DELETED => {
            testing::put(store, TableId::Countries, &CountryRow { id: 4, canonical_slot: 40, ..Default::default() }, TEXTS).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 4, canonical_slot: 40 }, TEXTS).unwrap();
            testing::remove(store, TableId::CountryPowerDailyCosts, 3 << 32 | 1).unwrap();
            testing::put(store, TableId::CountryPowerDaily, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2, 8) }, TEXTS).unwrap();
        }
        _ => {}
    }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    edit(store, hour);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn through(hours: std::ops::RangeInclusive<i32>) -> (Store, Writers) {
    let (mut store, mut writers) = (testing::store(), writers());
    for at in hours {
        run(&mut store, &mut writers, at);
    }
    (store, writers)
}

fn cell<R: replay_data::rows::Fields>(store: &Store, table: TableId, key: u128, name: &str) -> Option<i64> {
    let view = store.view(table.index());
    view.number(column::<R>(name), view.slot(key).expect("a row"))
}

fn spend(store: &Store, holder: u32, currency: u8, ordinal: u32, name: &str) -> Option<i64> {
    cell::<Total>(store, TableId::PowerSpendTotals, total_key(holder, currency, ordinal), name)
}

fn cost_items(store: &Store) -> Vec<u32> {
    keys(store, TableId::PowerItems).into_iter().filter(|&key| (key >> 32) as u32 == 10 && cell::<Item>(store, TableId::PowerItems, key, "kind") == Some(i64::from(Item::COST))).map(|key| key as u32).collect()
}

fn text<R: replay_data::rows::Fields>(store: &Store, table: TableId, key: u128, name: &str) -> String {
    let view = store.view(table.index());
    let id = view.text(column::<R>(name), view.slot(key).expect("a row")).expect("a text cell");
    String::from_utf8_lossy(view.string(id).expect("a held string")).into_owned()
}

fn keys(store: &Store, table: TableId) -> Vec<u128> {
    testing::rows(store, table).into_iter().map(|(key, _)| key).collect()
}

#[test]
fn a_payment_counts_under_its_items_typed_operands_and_a_country_no_link_resolves_under_none() {
    let (store, _) = through(0..=1);
    assert_eq!(keys(&store, TableId::PowerItems), [item_key(10, 0), item_key(10, 1), item_key(10, 2), item_key(NONE, 0)]);
    assert_eq!(spend(&store, 10, POLITICAL, 0, "payments"), Some(1), "token 7 listed at the run's first hour is not counted; hour 1's is");
    assert_eq!(spend(&store, 10, POLITICAL, 0, "known"), Some(2 * Q));
    assert_eq!(spend(&store, 10, POLITICAL, 0, "latest_hour"), Some(1));
    assert_eq!(spend(&store, 10, POLITICAL, 0, "latest_sequence"), Some(15));
    assert_eq!(spend(&store, 10, POLITICAL, 0, "uncovered"), Some(0));
    assert_eq!(spend(&store, 10, POLITICAL, 1, "payments"), Some(1));
    assert_eq!(spend(&store, 10, POLITICAL, 2, "balances"), Some(1), "an addition is a balance row of its item, no payment");
    assert_eq!(spend(&store, 10, POLITICAL, 2, "payments"), Some(0));
    assert_eq!(spend(&store, NONE, POLITICAL, 0, "payments"), Some(1));
    assert_eq!(spend(&store, 10, POLITICAL, 0, "gained"), None, "an item row holds no gains");
    assert_eq!(cell::<Item>(&store, TableId::PowerItems, item_key(10, 0), "first_sequence"), Some(15));
    assert_eq!(cell::<Item>(&store, TableId::PowerItems, item_key(10, 0), "first_read"), Some(1));
}

#[test]
fn a_runs_first_hour_lists_what_the_game_held_and_none_of_it_is_counted() {
    let (store, _) = through(0..=0);
    let coverage = |store: &Store, name: &str| cell::<Coverage>(store, TableId::PowerSpendCoverage, 10, name);
    for table in [TableId::PowerSpendTotals, TableId::PowerItems, TableId::PowerDayParts, TableId::PowerHolds] {
        assert!(keys(&store, table).is_empty(), "{table:?}: the payment and the day listed at attach are a backlog");
    }
    assert_eq!((coverage(&store, "political"), coverage(&store, "command"), coverage(&store, "reason")), (Some(0), Some(0), Some(i64::from(Coverage::BACKLOG))));
    assert_eq!(coverage(&store, "political_since"), None);
    let (store, _) = through(0..=1);
    assert_eq!(coverage(&store, "political_since"), Some(1), "spending is counted from the run's second hour, and the coverage says so");
    let (store, _) = through(0..=48);
    assert_eq!(spend(&store, 10, POLITICAL, Total::GAINS, "gain_days"), Some(1), "day 0 was listed at attach: only day 24 counts");
}

#[test]
fn every_abilitys_upkeep_is_one_item_and_identity_ignores_where_a_payment_was_read() {
    let upkeep = |reference: u64| CountryPowerChangeRow {
        currency: PowerCurrency::Command,
        cause: PowerCause::AbilityUpkeep,
        identity: PowerIdentity { object_kind: PowerObjectKind::Ability, object_reference: Some(reference), ..PowerIdentity::default() },
        ..idea(1, 1, 0, 0, Q)
    };
    assert_eq!(ItemId::change(&upkeep(1)), ItemId::change(&upkeep(2)));
    assert_eq!(ItemId::change(&upkeep(1)).hash(), ItemId::change(&upkeep(2)).hash());
    let (first, later) = (idea(1, 10, 0, 7, Q), CountryPowerChangeRow { site: PowerSite::AddPoliticalPowerNoDeficit, ..idea(5, 99, 70, 7, 3 * Q) });
    assert_eq!(ItemId::change(&first), ItemId::change(&later), "neither its country, hour, sequence, amount nor site tells an item apart");
    let refund = CountryPowerChangeRow { requested_delta_raw: Some(Q), ..first.clone() };
    assert_ne!(ItemId::change(&first), ItemId::change(&refund), "a charge and a refund of one operand are two items");
    let row = ItemId::change(&first).row(10, 3, &First { site: first.site, hour: 0, read: 0, country: 1, epoch: 1, sequence: 10 });
    assert_eq!(ItemId::of(&row), ItemId::change(&first), "an item reads back from its row");
}

#[test]
fn a_recurring_item_merges_its_payments_by_native_day_and_counts_an_uncovered_day_once() {
    let mut total = tally::item(10, POLITICAL, 0);
    let payment = |hour: i32, covered: bool| Payment { recurring: true, upkeep: true, hour, read: hour + 1, sequence: hour as u64, ordinal: 0, charge: Some(i128::from(Q)), covered, realized: Some(-i128::from(Q)) };
    for (hour, covered) in [(0, true), (5, false), (7, false), (24, true), (3, true)] {
        tally::pay(&mut total, &payment(hour, covered));
    }
    let number = |cell: Recorded<i64>| match cell {
        Recorded::Value(value) => Some(value),
        Recorded::NotRecorded => None,
    };
    assert_eq!(number(total.days), Some(2), "hours 0, 5, 7 and the late 3 are day 0; 24 is day 24");
    assert_eq!(number(total.day_payments), Some(5));
    assert_eq!(number(total.day_entries), Some(2), "upkeep's day is one entry");
    assert_eq!(number(total.days_uncovered), Some(1), "day 0 is uncovered once");
    assert_eq!(number(total.day_charge), Some(5 * Q));
    assert_eq!(number(total.realized_lost), Some(-5 * Q));
    assert_eq!(total.last_day, Recorded::Value(Some(24)));
    assert_eq!(total.first_day, Recorded::Value(Some(0)));
    assert_eq!(number(total.payments), Some(0), "a recurring payment is no one-off");
    let mut overflow = tally::item(10, POLITICAL, 1);
    for _ in 0..2 {
        tally::pay(&mut overflow, &Payment { recurring: false, charge: Some(i128::from(i64::MAX)), ..payment(0, true) });
    }
    assert_eq!(overflow.known, Recorded::NotRecorded, "a sum past i64 is not recorded, never wrapped");
    tally::pay(&mut overflow, &Payment { recurring: false, ..payment(1, true) });
    assert_eq!(overflow.known, Recorded::NotRecorded, "and stays so");
}

#[test]
fn a_daily_gain_is_its_gross_and_what_the_cap_kept_and_command_powers_deficit_is_no_gain() {
    let row = CountryPowerDailyRow { pp_realized_delta_raw: Some(Q / 2), cp_gain_raw: Some(-Q), cp_realized_delta_raw: Some(-Q), cp_sequence: Some(3), ..daily(0, 3 * Q, Q, 5) };
    assert_eq!(tally::day_gains(&row), [(Some(3 * Q), 2 * Q - Q / 2, true), (Some(0), 0, true)]);
    let unread = CountryPowerDailyRow { cp_gain_raw: None, cp_realized_delta_raw: None, cp_sequence: None, ..row };
    assert_eq!(tally::day_gains(&unread)[1], (None, 0, false));
}

#[test]
fn a_day_counts_once_when_it_ends_from_its_last_revision() {
    let (open, _) = through(0..=47);
    assert!(testing::rows(&open, TableId::PowerSpendTotals).iter().all(|(key, _)| *key as u32 != Total::GAINS), "day 24 is open: its gains are the daily row's");
    let costs = cost_items(&open);
    assert_eq!(costs.len(), 2, "the relation and the organisation upkeep are two items");
    assert!(costs.iter().all(|&ordinal| open.view(TableId::PowerSpendTotals.index()).slot(total_key(10, POLITICAL, ordinal)).is_none()), "an open day's costs are its parts");
    let (store, _) = through(0..=48);
    assert_eq!(spend(&store, 10, POLITICAL, Total::GAINS, "gained"), Some(3 * Q + Q / 10), "day 24 counted once, from its last revision");
    assert_eq!(spend(&store, 10, POLITICAL, Total::GAINS, "gain_days"), Some(1));
    assert_eq!(spend(&store, 10, POLITICAL, Total::GAINS, "known"), None, "a gains row holds no item columns");
    let relation = costs[0];
    assert_eq!(spend(&store, 10, POLITICAL, relation, "day_payments"), Some(1), "ordinal 0 counted once though its day was published again");
    assert_eq!(spend(&store, 10, POLITICAL, relation, "days"), Some(1));
    assert_eq!(spend(&store, 10, POLITICAL, relation, "day_charge"), Some(Q));
    assert_eq!(spend(&store, 10, POLITICAL, relation, "days_uncovered"), Some(0));
    assert_eq!(spend(&store, 10, POLITICAL, relation, "last_day"), Some(24));
    assert_eq!(spend(&store, 10, POLITICAL, costs[1], "day_charge"), Some(Q / 10));
}

#[test]
fn the_open_days_parts_hold_what_was_spent_since_midnight_and_end_with_the_day() {
    let (store, _) = through(0..=25);
    let costs = cost_items(&store);
    let (relation, organisation) = (part_key(10, POLITICAL, costs[0]), part_key(10, POLITICAL, costs[1]));
    let idea = part_key(10, POLITICAL, 0);
    assert_eq!(keys(&store, TableId::PowerDayParts), [idea, relation, organisation], "a part per item paid today, which the reply files under its content's category");
    assert_eq!(cell::<Part>(&store, TableId::PowerDayParts, relation, "spent"), Some(Q));
    assert_eq!(cell::<Part>(&store, TableId::PowerDayParts, relation, "sequence"), Some(50));
    assert_eq!(cell::<Part>(&store, TableId::PowerDayParts, idea, "spent"), Some(Q));
    assert_eq!(cell::<Part>(&store, TableId::PowerDayParts, idea, "day"), Some(24));
    let (later, _) = through(0..=NO_CHANGES);
    let status = later.view(TableId::PowerDayParts.index()).status();
    assert_eq!(status, Status::Unavailable { reason: 1 }, "an input not read leaves the parts unavailable with its own reason");
    assert_eq!(text::<Coverage>(&later, TableId::PowerSpendCoverage, 10, "input_reason"), "CountryPowerChanges: not read");
    let (next, _) = through(0..=48);
    assert_eq!(keys(&next, TableId::PowerDayParts), [relation], "day 24's parts end at midnight; day 48's relation is published");
    assert_eq!((cell::<Part>(&next, TableId::PowerDayParts, relation, "day"), cell::<Part>(&next, TableId::PowerDayParts, relation, "spent")), (Some(48), Some(Q / 2)));
    let (next, _) = through(0..=49);
    assert_eq!(cell::<Part>(&next, TableId::PowerDayParts, idea, "spent"), Some(2 * Q), "a later day's part starts from nothing");
    assert_eq!(cell::<Part>(&next, TableId::PowerDayParts, idea, "day"), Some(48));
}

#[test]
fn a_shorter_republished_day_withdraws_its_dropped_costs_and_keeps_todays_other_spending() {
    let (store, _) = through(0..=25);
    let costs = cost_items(&store);
    let (relation, organisation, idea) = (part_key(10, POLITICAL, costs[0]), part_key(10, POLITICAL, costs[1]), part_key(10, POLITICAL, 0));
    let (wider, _) = through(0..=WIDER);
    assert_eq!(keys(&wider, TableId::PowerDayParts), [idea, relation, organisation]);
    let (shorter, _) = through(0..=SHORTER);
    let covered = |store: &Store| {
        let view = store.view(TableId::PowerDayParts.index());
        view.bool(column::<Part>("covered"), view.slot(relation).expect("a row"))
    };
    assert_eq!(covered(&wider), Some(true));
    assert_eq!(covered(&shorter), Some(true), "a deletion leaves the surviving cost's proven coverage as it was");
    assert_eq!(keys(&shorter, TableId::PowerDayParts), [idea, relation], "the withdrawn cost is no longer today's, and the idea paid today stays");
    assert_eq!(cell::<Part>(&shorter, TableId::PowerDayParts, idea, "spent"), Some(2 * Q));
    let (emptied, _) = through(0..=EMPTIED);
    assert_eq!(keys(&emptied, TableId::PowerDayParts), [idea, relation], "a country left with no cost rows withdraws nothing");
    let (removed, _) = through(0..=REMOVED);
    assert_eq!(keys(&removed, TableId::PowerDayParts), [idea, relation], "rows gone before the application says why withdraw nothing");
    let (zeroed, _) = through(0..=ZEROED);
    assert_eq!(keys(&zeroed, TableId::PowerDayParts), [idea], "an application of no cost covered this hour then empties the batch");
    let (next, _) = through(0..=72);
    assert_eq!(spend(&next, 10, POLITICAL, costs[1], "days"), Some(1), "day 48 settles without the withdrawn cost");
    assert_eq!(spend(&next, 10, POLITICAL, costs[1], "day_charge"), Some(Q / 10));
    assert_eq!(spend(&next, 10, POLITICAL, costs[0], "days"), Some(1), "day 48's relation was withdrawn by its zero-cost application");
    let (settled, _) = through(0..=CLOSED_DELETED - 1);
    let (deleted, _) = through(0..=CLOSED_DELETED);
    for name in ["day_payments", "day_charge", "days"] {
        assert_eq!(spend(&deleted, 10, POLITICAL, costs[0], name), spend(&settled, 10, POLITICAL, costs[0], name), "a deletion from a closed day settles none of its costs again ({name})");
    }
    assert_eq!(spend(&settled, 10, POLITICAL, costs[0], "days"), Some(2), "day 72's relation settled at its midnight");
    let (paired, _) = through(0..=SAME_ITEM);
    assert_eq!((cell::<Part>(&paired, TableId::PowerDayParts, relation, "spent"), cell::<Part>(&paired, TableId::PowerDayParts, relation, "payments")), (Some(Q / 2 + Q / 4), Some(2)));
    let (dropped, _) = through(0..=SAME_ITEM_DROPPED);
    assert_eq!((cell::<Part>(&dropped, TableId::PowerDayParts, relation, "spent"), cell::<Part>(&dropped, TableId::PowerDayParts, relation, "payments")), (Some(Q / 2), Some(1)), "a deleted row of a shared item leaves the item what its remaining rows pay");
    assert_eq!((covered(&paired), covered(&dropped)), (Some(true), Some(true)), "and keeps the coverage it proved while its batch reconciles");
    let (restored, _) = through(0..=SAME_ITEM_RESTORED);
    assert_eq!(covered(&restored), Some(true));
    let (unreconciled, _) = through(0..=SAME_ITEM_UNRECONCILED);
    assert_eq!(cell::<Part>(&unreconciled, TableId::PowerDayParts, relation, "spent"), Some(Q / 2));
    assert_eq!(covered(&unreconciled), Some(false), "a survivor whose batch no longer matches the day's total is not covered");
    let holders = |store: &Store| keys(store, TableId::PowerDayParts).into_iter().map(|key| (key >> 40) as u32).filter(|&holder| holder == NONE || holder == 30).collect::<std::collections::BTreeSet<_>>();
    let (unlinked, _) = through(0..=UNLINKED);
    assert_eq!(holders(&unlinked), [NONE].into());
    let (linked, _) = through(0..=LINKED_DELETED);
    assert_eq!(holders(&linked), [NONE].into(), "a deletion opens no part under a holder its costs were never published under");
    let shared = |store: &Store, ordinal: u32| cell::<Part>(store, TableId::PowerDayParts, part_key(30, POLITICAL, ordinal), "spent");
    let (published, _) = through(0..=SHARED_PUBLISHED);
    assert_eq!((shared(&published, 0), shared(&published, 1)), (Some(Q / 2), Some(Q / 10)));
    let (shared_deleted, _) = through(0..=SHARED_DELETED);
    assert_eq!((shared(&shared_deleted, 0), shared(&shared_deleted, 1)), (Some(Q / 2), None), "on a shared holder a deletion withdraws the dropped cost and keeps the survivor as published");
    let (other, _) = through(0..=OTHER_PUBLISHED);
    assert_eq!(shared(&other, 0), Some(Q / 4));
    let (unshared, _) = through(0..=UNSHARED_DELETED);
    assert_eq!(shared(&unshared, 0), Some(Q / 4), "a deletion leaves a part another application published as it was");
}

#[test]
fn coverage_names_why_spending_is_not_recorded_and_a_stretch_restarts_after_a_break() {
    let (store, _) = through(0..=25);
    let coverage = |store: &Store, name: &str| cell::<Coverage>(store, TableId::PowerSpendCoverage, 10, name);
    assert_eq!(keys(&store, TableId::PowerSpendCoverage), [10, 20], "one row per live country's slot");
    assert_eq!((coverage(&store, "political"), coverage(&store, "command"), coverage(&store, "reason")), (Some(1), Some(1), Some(0)));
    assert_eq!(coverage(&store, "political_since"), Some(1));
    assert_eq!(text::<Coverage>(&store, TableId::PowerSpendCoverage, 10, "input_reason"), "");
    let (store, _) = through(0..=NO_CHANGES);
    assert_eq!((coverage(&store, "political"), coverage(&store, "reason")), (Some(0), Some(i64::from(Coverage::CHANGES_UNREAD))));
    assert_eq!(coverage(&store, "political_since"), None);
    let (store, _) = through(0..=NO_CHANGES + 1);
    assert_eq!(coverage(&store, "political_since"), Some(i64::from(NO_CHANGES + 1)));
    let (store, _) = through(0..=28);
    assert_eq!((coverage(&store, "political"), coverage(&store, "reason")), (Some(0), Some(i64::from(Coverage::DAILY_COSTS_UNREAD))));
    assert_eq!(coverage(&store, "daily_mismatch"), Some(i64::from(Coverage::STEADY_MISMATCH)), "a daily cost its costs do not add up to is named apart");
    let (store, _) = through(0..=29);
    assert_eq!(coverage(&store, "daily_mismatch"), Some(0));
    let (store, _) = through(0..=NO_CAPTURE);
    assert_eq!((coverage(&store, "command"), coverage(&store, "reason")), (Some(0), Some(i64::from(Coverage::NO_CAPTURE))));
    assert_eq!(text::<Coverage>(&store, TableId::PowerSpendCoverage, 10, "input_reason"), "PoliticsCaptureCoverage: not read");
}

#[test]
fn an_hour_that_moves_no_coverage_sets_no_coverage_row() {
    let (mut store, mut writers) = (testing::store(), Writers::new(vec![Box::new(testing::noting::Noting(PowerLedger::default()))]));
    writers.start_run();
    let coverage = |hours: &[testing::noting::Noted]| -> Vec<Vec<u128>> {
        hours.iter().map(|hour| hour.iter().filter(|(table, ..)| *table == TableId::PowerSpendCoverage).flat_map(|(_, sets, _)| sets.clone()).collect()).collect()
    };
    for at in 0..=3 {
        run(&mut store, &mut writers, at);
    }
    testing::noting::take();
    for at in 4..=6 {
        run(&mut store, &mut writers, at);
    }
    assert_eq!(coverage(&testing::noting::take()), [Vec::<u128>::new(), Vec::new(), Vec::new()]);
    for at in 7..NO_CHANGES {
        run(&mut store, &mut writers, at);
    }
    testing::noting::take();
    run(&mut store, &mut writers, NO_CHANGES);
    assert_eq!(coverage(&testing::noting::take()), [vec![10, 20]], "the changes not read move both");
    assert_eq!(cell::<Coverage>(&store, TableId::PowerSpendCoverage, 20, "reason"), Some(i64::from(Coverage::CHANGES_UNREAD)));
}

#[test]
fn an_allocation_holds_until_its_refund_names_it() {
    let (store, _) = through(0..=2);
    let held = keys(&store, TableId::PowerHolds);
    assert_eq!(held.len(), 1);
    assert_eq!((held[0] >> 64) as u32, 10);
    assert_eq!(cell::<Hold>(&store, TableId::PowerHolds, held[0], "amount"), Some(2 * Q));
    assert_eq!(cell::<Hold>(&store, TableId::PowerHolds, held[0], "character"), Some(4));
    assert_eq!((cell::<Hold>(&store, TableId::PowerHolds, held[0], "capture_country"), cell::<Hold>(&store, TableId::PowerHolds, held[0], "capture_epoch")), (Some(1), Some(1)), "the hold names the capture it was applied under");
    let (store, _) = through(0..=3);
    assert!(keys(&store, TableId::PowerHolds).is_empty(), "the refund of the whole amount ends the hold");
    let mut active = BTreeMap::new();
    apply(&mut active, 10, (1, 1), &hold(1, 0, PowerCause::Allocation, PowerSite::AllocateCommandPower, -Q)).unwrap();
    let anonymous = CountryPowerChangeRow { identity: PowerIdentity::default(), ..hold(2, 0, PowerCause::AllocationRefund, PowerSite::ReleaseCommandPower, Q / 2) };
    apply(&mut active, 10, (1, 1), &anonymous).unwrap();
    assert!(active.is_empty(), "a refund that names no allocation clears every hold");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (unbroken, _) = through(0..=49);
    let (stopped, before) = through(0..=24);
    let saved = before.save().unwrap();
    let mut resumed = testing::resumed(&stopped, TEXTS);
    let mut after = writers();
    after.load(&saved, &mut resumed).unwrap();
    after.prime(&resumed, 1, 24);
    for at in 25..=49 {
        run(&mut resumed, &mut after, at);
    }
    for table in TABLES.iter().map(|output| output.table()) {
        assert_eq!(testing::rows(&resumed, table), testing::rows(&unbroken, table), "{table:?}");
    }
}

#[test]
fn its_blob_holds_two_hours_and_a_damaged_one_is_refused() {
    let ledger = PowerLedger { last: Some(5), run_first: Some(-3), ..PowerLedger::default() };
    let mut blob = Bytes::new();
    ledger.save(&mut blob).unwrap();
    assert!(blob.bytes().len() <= 10, "two hours, nothing of an input's rows");
    let mut loaded = PowerLedger::default();
    loaded.load(blob.bytes()).unwrap();
    assert!(loaded.rebuild);
    assert_eq!((loaded.last, loaded.run_first), (Some(5), Some(-3)));
    assert!(loaded.load(&blob.bytes()[..blob.bytes().len() - 1]).is_err());
}
