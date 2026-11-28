use super::*;
use recorder_derive_framework::{framework::Writers, testing};
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
        balance_after_raw: replay_data::rows::RetiredI64(0),
        realized_delta_raw: replay_data::rows::Recorded::Value(-amount),
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
const UNMATCHED: i32 = 120;
const MATCHED_DELETED: i32 = 121;
const CANCELLED: i32 = 122;
const STALE_ZERO: i32 = 123;
const MATCHED: i32 = 124;
const DAILY_GONE: i32 = 125;
const PAIRED: i32 = 144;
const TOTAL_KEPT: i32 = 145;
const GROWN_BLOCKED: i32 = 169;
const GROWN: i32 = 170;
const SHRUNK_BLOCKED: i32 = 193;
const SHRUNK: i32 = 194;
const MOVED: i32 = 217;
const MOVED_DELETED: i32 = 218;
const OVERNIGHT_BLOCKED: i32 = 263;
const OVERNIGHT: i32 = 264;
const REPLACED_BLOCKED: i32 = 311;
const REPLACED: i32 = 312;
const REVISED_BLOCKED: i32 = 359;
const REVISED: i32 = 360;
const UNLINKED_AGAIN: i32 = 385;
const RELINKED: i32 = 386;
const DEFERRED_BLOCKED: i32 = 431;
const DEFERRED: i32 = 432;
const SWAPPED: i32 = 457;
const CONTRADICTED_BLOCKED: i32 = 490;
const CONTRADICTED: i32 = 491;
const EMPTIED_KEPT: i32 = 505;
const MOVED_EMPTY: i32 = 506;
const DELAYED_BLOCKED: i32 = 538;
const DELAYED_UNREAD: i32 = 539;
const DELAYED: i32 = 540;
const RETOTALLED_BLOCKED: i32 = 562;
const RETOTALLED: i32 = 563;
const NO_CHANGES: i32 = 26;

fn put_daily(store: &mut Store, hour: i32, row: &CountryPowerDailyRow) {
    testing::put(store, TableId::CountryPowerDaily, row, TEXTS).unwrap();
    store.status(TableId::CountryPowerDaily.index(), Status::Complete, true, Some(hour), true).unwrap();
}

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
    if matches!(hour, GROWN_BLOCKED | SHRUNK_BLOCKED | OVERNIGHT_BLOCKED | REPLACED_BLOCKED | REVISED_BLOCKED | DEFERRED_BLOCKED | CONTRADICTED_BLOCKED | DELAYED_BLOCKED | RETOTALLED_BLOCKED) {
        store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
    } else if matches!(hour, GROWN | SHRUNK | OVERNIGHT | REPLACED | REVISED | DEFERRED | CONTRADICTED | DELAYED_UNREAD | RETOTALLED) {
        complete(store, TableId::CountryLinks);
    }
    if hour == DEFERRED {
        store.status(TableId::CountryPowerDailyCosts.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
    } else if hour == DEFERRED + 1 {
        complete(store, TableId::CountryPowerDailyCosts);
    }
    if matches!(hour, DELAYED_UNREAD | RETOTALLED) {
        store.status(TableId::CountryPowerDaily.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
    } else if matches!(hour, GROWN | SHRUNK | OVERNIGHT | CONTRADICTED | DELAYED) {
        complete(store, TableId::CountryPowerDaily);
    }
    if hour == NO_CAPTURE {
        store.status(TableId::PoliticsCaptureCoverage.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
    } else {
        complete(store, TableId::PoliticsCaptureCoverage);
        for country in [1, 2] {
            let since_hour = if hour >= SHRUNK {
                SHRUNK - 1
            } else if hour >= GROWN {
                GROWN - 1
            } else if hour >= SAME_ITEM {
                SAME_ITEM - 2
            } else if hour >= SHORTER {
                hour - 1
            } else {
                0
            };
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
            put_daily(store, hour, &daily(0, 3 * Q, Q, 5));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(0, 0, PowerDailyCostKind::Relation, Q, 5), TEXTS).unwrap();
        }
        2 => testing::put(store, TableId::CountryCommandPower, &CountryCommandPowerRow { canonical_slot: 10, allocated_command_power_raw: 2 * Q, ..Default::default() }, TEXTS).unwrap(),
        24 => {
            put_daily(store, hour, &daily(24, 3 * Q, Q, 50));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q, 50), TEXTS).unwrap();
        }
        25 => {
            put_daily(store, hour, &daily(24, 3 * Q + Q / 10, Q + Q / 10, 50));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 1, PowerDailyCostKind::MioResearch, Q / 10, 50), TEXTS).unwrap();
        }
        28 => put_daily(store, hour, &daily(24, 4 * Q, 2 * Q, 50)),
        29 => put_daily(store, hour, &daily(24, 3 * Q + Q / 10, Q + Q / 10, 50)),
        48 => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            put_daily(store, hour, &daily(48, 2 * Q, Q / 2, 80));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap();
        }
        WIDER => {
            put_daily(store, hour, &daily(48, 2 * Q + Q / 10, Q / 2 + Q / 10, 80));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 1, PowerDailyCostKind::MioResearch, Q / 10, 80), TEXTS).unwrap();
        }
        SHORTER => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            put_daily(store, hour, &daily(48, 2 * Q, Q / 2, 80));
        }
        EMPTIED => testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap(),
        RESTORED => testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap(),
        REMOVED => testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap(),
        ZEROED => put_daily(store, hour, &daily(48, 2 * Q, 0, 80)),
        LATER_DAY => {
            put_daily(store, hour, &daily(72, 3 * Q, Q / 2 + Q / 10, 90));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 0, PowerDailyCostKind::Relation, Q / 2, 90), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 1, PowerDailyCostKind::MioResearch, Q / 10, 90), TEXTS).unwrap();
        }
        CLOSED_DELETED => testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap(),
        SAME_ITEM => {
            put_daily(store, hour, &daily(96, 3 * Q, Q / 2 + Q / 4, 99));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(96, 0, PowerDailyCostKind::Relation, Q / 2, 99), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(96, 1, PowerDailyCostKind::Relation, Q / 4, 99), TEXTS).unwrap();
        }
        SAME_ITEM_DROPPED => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            put_daily(store, hour, &daily(96, 3 * Q, Q / 2, 99));
        }
        SAME_ITEM_RESTORED => {
            put_daily(store, hour, &daily(96, 3 * Q, Q / 2 + Q / 4, 99));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(96, 1, PowerDailyCostKind::Relation, Q / 4, 99), TEXTS).unwrap();
        }
        SAME_ITEM_UNRECONCILED => testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap(),
        UNLINKED => {
            put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2 + Q / 10, 7) });
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
            put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2 + Q / 10, 8) });
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(96, 0, PowerDailyCostKind::Relation, Q / 2, 8) }, TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(96, 1, PowerDailyCostKind::MioResearch, Q / 10, 8) }, TEXTS).unwrap();
        }
        SHARED_DELETED => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 3 << 32 | 1).unwrap();
            put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2, 8) });
        }
        REPUBLISHED => {
            put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2 + Q / 10, 8) });
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(96, 1, PowerDailyCostKind::MioResearch, Q / 10, 8) }, TEXTS).unwrap();
        }
        OTHER_PUBLISHED => {
            put_daily(store, hour, &CountryPowerDailyRow { country: 4, ..daily(96, 3 * Q, Q / 4, 9) });
            testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 4, ..cost(96, 0, PowerDailyCostKind::Relation, Q / 4, 9) }, TEXTS).unwrap();
        }
        UNSHARED_DELETED => {
            testing::put(store, TableId::Countries, &CountryRow { id: 4, canonical_slot: 40, ..Default::default() }, TEXTS).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 4, canonical_slot: 40 }, TEXTS).unwrap();
            testing::remove(store, TableId::CountryPowerDailyCosts, 3 << 32 | 1).unwrap();
            put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(96, 3 * Q, Q / 2, 8) });
        }
        UNMATCHED => {
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(120, 0, PowerDailyCostKind::Relation, Q / 2, 120), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(120, 1, PowerDailyCostKind::MioResearch, Q / 10, 120), TEXTS).unwrap();
        }
        MATCHED_DELETED => {
            put_daily(store, hour, &daily(120, 3 * Q, Q / 2, 120));
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
        }
        CANCELLED => {
            put_daily(store, hour, &daily(120, 3 * Q, 0, 121));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(120, 0, PowerDailyCostKind::Relation, Q / 4, 121), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(120, 1, PowerDailyCostKind::MioResearch, -Q / 4, 121), TEXTS).unwrap();
        }
        STALE_ZERO => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap();
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
        }
        MATCHED => {
            put_daily(store, hour, &daily(120, 3 * Q, Q / 2 + Q / 10, 124));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(120, 0, PowerDailyCostKind::Relation, Q / 2, 124), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(120, 1, PowerDailyCostKind::MioResearch, Q / 10, 124), TEXTS).unwrap();
        }
        DAILY_GONE => {
            testing::remove(store, TableId::CountryPowerDaily, 1).unwrap();
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
        }
        PAIRED => {
            put_daily(store, hour, &daily(144, 3 * Q, Q / 2 + Q / 10, 144));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(144, 0, PowerDailyCostKind::Relation, Q / 2, 144), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(144, 1, PowerDailyCostKind::MioResearch, Q / 10, 144), TEXTS).unwrap();
        }
        TOTAL_KEPT => {
            put_daily(store, hour, &daily(144, 3 * Q, Q / 2 + Q / 10, 144));
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
        }
        168 => {
            put_daily(store, hour, &daily(168, 3 * Q, Q / 2, 168));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(168, 0, PowerDailyCostKind::Relation, Q / 2, 168), TEXTS).unwrap();
        }
        GROWN_BLOCKED => {
            put_daily(store, hour, &daily(168, 3 * Q, Q / 2 + Q / 10, 168));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(168, 1, PowerDailyCostKind::MioResearch, Q / 10, 168), TEXTS).unwrap();
        }
        192 | 216 | 240 | 288 | REPLACED | 336 | 384 | 408 | 456 | 480 | 504 | 528 | 552 => {
            put_daily(store, hour, &daily(hour, 3 * Q, Q / 2 + Q / 10, hour as u64));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(hour, 0, PowerDailyCostKind::Relation, Q / 2, hour as u64), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(hour, 1, PowerDailyCostKind::MioResearch, Q / 10, hour as u64), TEXTS).unwrap();
        }
        SHRUNK_BLOCKED => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            put_daily(store, hour, &daily(192, 3 * Q, Q / 2, 192));
        }
        MOVED => {
            testing::put(store, TableId::Countries, &CountryRow { id: 1, canonical_slot: 50, ..Default::default() }, TEXTS).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 50 }, TEXTS).unwrap();
        }
        REVISED => {
            put_daily(store, hour, &daily(336, 3 * Q, Q / 4 + Q / 10, 336));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(336, 0, PowerDailyCostKind::Relation, Q / 4, 336), TEXTS).unwrap();
        }
        UNLINKED_AGAIN => testing::remove(store, TableId::CountryLinks, 1).unwrap(),
        SWAPPED => {
            for (id, slot) in [(1, 20), (2, 50)] {
                testing::put(store, TableId::Countries, &CountryRow { id, canonical_slot: slot, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: slot }, TEXTS).unwrap();
            }
        }
        RETOTALLED_BLOCKED => put_daily(store, hour, &daily(552, 3 * Q, Q / 5, 552)),
        EMPTIED_KEPT => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap();
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
        }
        MOVED_EMPTY => {
            testing::put(store, TableId::Countries, &CountryRow { id: 1, canonical_slot: 60, ..Default::default() }, TEXTS).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 60 }, TEXTS).unwrap();
        }
        CONTRADICTED_BLOCKED | DELAYED_BLOCKED => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap();
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            let day = midnight(hour);
            put_daily(store, hour, &daily(day, 3 * Q, Q / 5, day as u64));
        }
        RELINKED => {
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 50 }, TEXTS).unwrap();
            put_daily(store, hour, &daily(384, 3 * Q, Q / 4 + Q / 10, 384));
            testing::put(store, TableId::CountryPowerDailyCosts, &cost(384, 0, PowerDailyCostKind::Relation, Q / 4, 384), TEXTS).unwrap();
        }
        MOVED_DELETED | OVERNIGHT_BLOCKED | REPLACED_BLOCKED | DEFERRED_BLOCKED => {
            testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            let day = midnight(hour);
            put_daily(store, hour, &daily(day, 3 * Q, Q / 2, day as u64));
        }
        _ => {}
    }
}

fn second_country(store: &mut Store, hour: i32) {
    if hour == 456 {
        put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(456, 3 * Q, Q / 2, 456) });
        testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(456, 0, PowerDailyCostKind::Relation, Q / 3, 456) }, TEXTS).unwrap();
    }
}

#[derive(Default)]
struct DaySummary {
    days: i64,
    day_payments: i64,
    day_charge: i64,
    day_unknown: i64,
    days_uncovered: i64,
    credits: i64,
    first_day: Option<i32>,
    last_day: Option<i32>,
}

#[derive(Default)]
struct PaymentSummary {
    known: i64,
    payments: i64,
    unknown: i64,
    uncovered: i64,
    latest_hour: Option<i32>,
    latest_sequence: Option<u64>,
}

#[derive(Default)]
struct GainSummary {
    gained: i128,
    capped: i128,
    gain_days: i64,
    gain_unknown_days: i64,
    unmatched: i64,
}

struct LandedLogs {
    store: Store,
    days: BTreeMap<(u32, u8, u32), DaySummary>,
    payments: BTreeMap<(u32, u8, u32), PaymentSummary>,
    gains: BTreeMap<(u32, u8), GainSummary>,
}

impl std::ops::Deref for LandedLogs {
    type Target = Store;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl LandedLogs {
    fn new(store: Store) -> Self {
        Self { store, days: BTreeMap::new(), payments: BTreeMap::new(), gains: BTreeMap::new() }
    }

    fn record_landed_rows(&mut self) {
        assert!(keys(&self.store, TableId::PowerSpendTotals).is_empty(), "PowerSpendTotals is retired in format 155");
        let view = self.store.view(TableId::PowerSpendDays.index());
        for row in inputs::every::<SpendDay>(&view).unwrap() {
            let entry = self.days.entry((row.holder, row.currency, row.ordinal)).or_default();
            entry.days += row.days;
            entry.day_payments += row.payments;
            if let Recorded::Value(charge) = row.charge {
                entry.day_charge += charge;
            }
            entry.day_unknown += row.unknown;
            entry.days_uncovered += row.uncovered;
            entry.credits += row.credits;
            if row.days > 0 || row.payments > 0 {
                entry.first_day = Some(entry.first_day.map_or(row.day, |first| first.min(row.day)));
                entry.last_day = Some(entry.last_day.map_or(row.day, |last| last.max(row.day)));
            }
        }
        let view = self.store.view(TableId::PowerSpendPayments.index());
        for row in inputs::every::<SpendPayment>(&view).unwrap() {
            let entry = self.payments.entry((row.holder, row.currency, row.ordinal)).or_default();
            if let Recorded::Value(known) = row.known {
                entry.known += known;
            }
            entry.payments += row.payments;
            entry.unknown += row.unknown;
            entry.uncovered += row.uncovered;
            if row.latest_hour.is_some() {
                entry.latest_hour = row.latest_hour;
                entry.latest_sequence = row.latest_sequence;
            }
        }
        let view = self.store.view(TableId::PowerSpendGains.index());
        for row in inputs::every::<SpendGain>(&view).unwrap() {
            let entry = self.gains.entry((row.holder, row.currency)).or_default();
            if let Recorded::Value(gained) = row.gained {
                entry.gained += i128::from(gained);
            }
            if let Recorded::Value(capped) = row.capped {
                entry.capped += i128::from(capped);
            }
            entry.gain_days += row.days;
            entry.gain_unknown_days += row.unknown_days;
            entry.unmatched += row.unmatched;
        }
    }

    fn has_item_activity(&self, holder: u32, currency: u8, ordinal: u32) -> bool {
        self.days.contains_key(&(holder, currency, ordinal)) || self.payments.contains_key(&(holder, currency, ordinal))
    }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    edit(store, hour);
    second_country(store, hour);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn through(hours: std::ops::RangeInclusive<i32>) -> (LandedLogs, Writers) {
    let (mut logged, mut writers) = (LandedLogs::new(testing::store()), writers());
    for at in hours {
        run(&mut logged.store, &mut writers, at);
        logged.record_landed_rows();
    }
    (logged, writers)
}

fn cell<R: replay_data::rows::Fields>(store: &Store, table: TableId, key: u128, name: &str) -> Option<i64> {
    let view = store.view(table.index());
    view.number(column::<R>(name), view.slot(key).expect("a row"))
}

fn spend(logged: &LandedLogs, holder: u32, currency: u8, ordinal: u32, name: &str) -> Option<i64> {
    if ordinal == Total::GAINS {
        let entry = logged.gains.get(&(holder, currency))?;
        return match name {
            "gained" => i64::try_from(entry.gained).ok(),
            "capped" => i64::try_from(entry.capped).ok(),
            "gain_days" => Some(entry.gain_days),
            "gain_unknown_days" => Some(entry.gain_unknown_days),
            "unmatched" => Some(entry.unmatched),
            _ => None,
        };
    }
    let day = logged.days.get(&(holder, currency, ordinal));
    let payment = logged.payments.get(&(holder, currency, ordinal));
    if day.is_none() && payment.is_none() {
        return None;
    }
    match name {
        "days" => Some(day.map_or(0, |d| d.days)),
        "day_payments" => Some(day.map_or(0, |d| d.day_payments)),
        "day_charge" => Some(day.map_or(0, |d| d.day_charge)),
        "day_unknown" => Some(day.map_or(0, |d| d.day_unknown)),
        "days_uncovered" => Some(day.map_or(0, |d| d.days_uncovered)),
        "credits" => Some(day.map_or(0, |d| d.credits)),
        "first_day" => day.and_then(|d| d.first_day).map(i64::from),
        "last_day" => day.and_then(|d| d.last_day).map(i64::from),
        "known" => Some(payment.map_or(0, |p| p.known)),
        "payments" => Some(payment.map_or(0, |p| p.payments)),
        "unknown" => Some(payment.map_or(0, |p| p.unknown)),
        "uncovered" => Some(payment.map_or(0, |p| p.uncovered)),
        "latest_hour" => payment.and_then(|p| p.latest_hour).map(i64::from),
        "latest_sequence" => payment.and_then(|p| p.latest_sequence).map(|s| s as i64),
        _ => None,
    }
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
    assert!(!store.has_item_activity(10, POLITICAL, 2), "an addition logs as an instant gain, not a payment");
    assert_eq!(spend(&store, 10, POLITICAL, Total::GAINS, "gained"), Some(Q), "focus addition logs to PowerSpendGains");
    assert_eq!(spend(&store, NONE, POLITICAL, 0, "payments"), Some(1));
    assert_eq!(cell::<Item>(&store, TableId::PowerItems, item_key(10, 0), "first_sequence"), Some(15));
    assert_eq!(cell::<Item>(&store, TableId::PowerItems, item_key(10, 0), "first_read"), Some(1));
}

#[test]
fn a_runs_first_hour_lists_what_the_game_held_and_none_of_it_is_counted() {
    let (store, _) = through(0..=0);
    let coverage = |store: &Store, name: &str| cell::<Coverage>(store, TableId::PowerSpendCoverage, 10, name);
    for table in [TableId::PowerSpendTotals, TableId::PowerItems, TableId::PowerDayParts, TableId::PowerHolds, TableId::PowerSpendDays, TableId::PowerSpendPayments, TableId::PowerSpendGains] {
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
    let mut seen = ItemDays::default();
    let payment = |hour: i32, covered: bool| Payment { recurring: true, hour, sequence: hour as u64, ordinal: 0, charge: Some(i128::from(Q)), covered, realized: Some(-i128::from(Q)), count: 1 };
    let mut deltas = Vec::new();
    let mut day_event = DayEvent::default();
    for (hour, covered) in [(0, true), (5, false), (7, false), (24, true), (3, true)] {
        let pay = payment(hour, covered);
        let delta = tally::count_day(&mut seen, &pay);
        if let Some((_, new_day, newly_uncovered)) = delta {
            day_event.pay(&pay, new_day, newly_uncovered);
        }
        deltas.push(delta);
    }
    assert_eq!(deltas, [Some((0, true, false)), Some((0, false, true)), Some((0, false, false)), Some((24, true, false)), Some((0, false, false))]);
    assert_eq!(day_event.days, 2, "hours 0, 5, 7 and the late 3 are day 0; 24 is day 24");
    assert_eq!(day_event.payments, 5);
    assert_eq!(day_event.uncovered, 1, "day 0 is uncovered once");
    assert_eq!(day_event.charge, Recorded::Value(5 * Q));
    assert_eq!(day_event.realized_lost, Recorded::Value(-5 * Q));
    let mut overflow = PaymentEvent::default();
    for _ in 0..2 {
        overflow.pay(&Payment { recurring: false, charge: Some(i128::from(i64::MAX)), ..payment(0, true) });
    }
    assert_eq!(overflow.known, Recorded::NotRecorded, "a sum past i64 is not recorded, never wrapped");
    overflow.pay(&Payment { recurring: false, ..payment(1, true) });
    assert_eq!(overflow.known, Recorded::NotRecorded, "and stays so");
}

#[test]
fn aggregate_settlement_matches_repeated_payments_in_constant_steps() {
    for (recurring, charge, covered, count) in [
        (true, Some(5 * Q), true, 7_i64),
        (true, Some(5 * Q), false, 7),
        (true, None, false, 7),
        (false, Some(5 * Q), true, 7),
        (false, None, false, 7),
    ] {
        let make = |charge, count| Payment { recurring, hour: 24, sequence: 50, ordinal: 3, charge, covered, realized: None, count };
        if recurring {
            let mut batched = DayEvent::default();
            let mut batched_seen = ItemDays::default();
            let p = make(charge.map(i128::from), count);
            if let Some((_, new_day, newly_uncovered)) = tally::count_day(&mut batched_seen, &p) {
                batched.pay(&p, new_day, newly_uncovered);
            }
            let mut looped = DayEvent::default();
            let mut looped_seen = ItemDays::default();
            for at in 0..count {
                let piece = if at == 0 { charge.map(i128::from) } else { charge.map(|_| 0) };
                let p = make(piece, 1);
                if let Some((_, new_day, newly_uncovered)) = tally::count_day(&mut looped_seen, &p) {
                    looped.pay(&p, new_day, newly_uncovered);
                }
            }
            assert_eq!((batched.days, batched.payments, batched.charge, batched.unknown, batched.uncovered, batched.realized_unknown), (looped.days, looped.payments, looped.charge, looped.unknown, looped.uncovered, looped.realized_unknown));
            assert_eq!(batched_seen, looped_seen);
        } else {
            let mut batched = PaymentEvent::default();
            batched.pay(&make(charge.map(i128::from), count));
            let mut looped = PaymentEvent::default();
            for at in 0..count {
                let piece = if at == 0 { charge.map(i128::from) } else { charge.map(|_| 0) };
                looped.pay(&make(piece, 1));
            }
            assert_eq!((batched.known, batched.payments, batched.unknown, batched.uncovered, batched.latest), (looped.known, looped.payments, looped.unknown, looped.uncovered, looped.latest));
        }
    }
    let mut big = DayEvent::default();
    let mut big_seen = ItemDays::default();
    let large = MAX_BATCH_PAYMENTS as i64;
    let pay = Payment { recurring: true, hour: 48, sequence: 90, ordinal: 0, charge: Some(i128::from(Q)), covered: false, realized: None, count: large };
    let result = tally::count_day(&mut big_seen, &pay);
    assert_eq!(result, Some((48, true, true)));
    big.pay(&pay, true, true);
    assert_eq!((big.payments, big.realized_unknown, big.days, big.uncovered, big.charge), (large, large, 1, 1, Recorded::Value(Q)));
    let mut credits = PaymentEvent::default();
    credits.unknown_credit(large);
    assert_eq!((credits.payments, credits.unknown), (0, large));
    let mut gains = Vec::new();
    push_gain(&mut gains, Some(0), Some(0), 0, 0, large);
    assert_eq!(gains.len(), 1);
    assert_eq!(gains[0].unmatched, large);
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
    assert_eq!(spend(&open, 10, POLITICAL, Total::GAINS, "gain_days"), Some(0), "day 24 is open: no daily gain counted yet");
    let costs = cost_items(&open);
    assert_eq!(costs.len(), 2, "the relation and the organisation upkeep are two items");
    assert!(costs.iter().all(|&ordinal| !open.has_item_activity(10, POLITICAL, ordinal)), "an open day's costs are its parts");
    let (store, _) = through(0..=48);
    assert_eq!(spend(&store, 10, POLITICAL, Total::GAINS, "gained"), Some(Q + 3 * Q + Q / 10), "hour 1 focus gain plus day 24 counted once from its last revision");
    assert_eq!(spend(&store, 10, POLITICAL, Total::GAINS, "gain_days"), Some(1));
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
    assert_eq!(cell::<Coverage>(&unreconciled, TableId::PowerSpendCoverage, 10, "daily_mismatch"), Some(i64::from(Coverage::STEADY_MISMATCH)), "and its holder's costs are named as not adding up until a daily application confirms them");
    let holders = |store: &Store| keys(store, TableId::PowerDayParts).into_iter().map(|key| (key >> 40) as u32).filter(|&holder| holder == NONE || holder == 30).collect::<std::collections::BTreeSet<_>>();
    let (unlinked, _) = through(0..=UNLINKED);
    assert_eq!(holders(&unlinked), [NONE].into());
    let (linked, _) = through(0..=LINKED_DELETED);
    assert_eq!(holders(&linked), [30].into(), "linking a country that published while unlinked withdraws its batch from NONE and reads the survivor under its new holder");
    let shared = |store: &Store, ordinal: u32| cell::<Part>(store, TableId::PowerDayParts, part_key(30, POLITICAL, ordinal), "spent");
    let (published, _) = through(0..=SHARED_PUBLISHED);
    assert_eq!((shared(&published, 0), shared(&published, 1)), (Some(Q / 2), Some(Q / 10)));
    let (shared_deleted, _) = through(0..=SHARED_DELETED);
    assert_eq!(shared(&shared_deleted, 0), Some(Q / 2), "on a shared holder a deletion keeps the survivor as published");
    assert!(!keys(&shared_deleted, TableId::PowerDayParts).contains(&part_key(30, POLITICAL, 1)), "and withdraws the dropped cost of that country's own batch");
    let (other, _) = through(0..=OTHER_PUBLISHED);
    assert_eq!(
        (shared(&other, 0), cell::<Part>(&other, TableId::PowerDayParts, part_key(30, POLITICAL, 0), "payments")),
        (Some(Q / 2 + Q / 4), Some(2)),
        "a second country paying the same item on a shared holder sums into the part rather than overwriting"
    );
    let (unshared, _) = through(0..=UNSHARED_DELETED);
    assert_eq!(shared(&unshared, 0), Some(Q / 2), "a country leaving a shared holder takes only its own batch and leaves the staying country's part intact");
    assert_eq!(cell::<Part>(&unshared, TableId::PowerDayParts, part_key(40, POLITICAL, 0), "spent"), Some(Q / 4), "and its batch is read again under its new holder");
    let gains = part_key(10, POLITICAL, Total::GAINS);
    let (unmatched, _) = through(0..=UNMATCHED);
    assert_eq!(cell::<Part>(&unmatched, TableId::PowerDayParts, gains, "payments"), Some(2));
    let (matched, _) = through(0..=MATCHED_DELETED);
    assert!(keys(&matched, TableId::PowerDayParts).contains(&gains), "a deletion keeps the survivor of an application it held unmatched");
    assert_eq!((cell::<Part>(&matched, TableId::PowerDayParts, gains, "payments"), cell::<Part>(&matched, TableId::PowerDayParts, gains, "spent")), (Some(1), Some(Q / 2)));
    let open = |store: &Store| keys(store, TableId::PowerDayParts).into_iter().filter(|&key| (key >> 40) as u32 == 10).count();
    let (cancelled, _) = through(0..=CANCELLED);
    let (stale, _) = through(0..=STALE_ZERO);
    assert_eq!(open(&stale), open(&cancelled), "a daily zero carried from an earlier hour confirms no empty batch");
    let (paid, _) = through(0..=MATCHED);
    assert_eq!((cell::<Part>(&paid, TableId::PowerDayParts, relation, "spent"), covered(&paid)), (Some(Q / 2), Some(true)));
    let (gone, _) = through(0..=DAILY_GONE);
    assert_eq!((cell::<Part>(&gone, TableId::PowerDayParts, relation, "spent"), covered(&gone)), (Some(Q / 2), Some(false)), "a deletion keeps a survivor in the part it was published in, uncovered while no daily application confirms the batch");
    let mismatch = |store: &Store| cell::<Coverage>(store, TableId::PowerSpendCoverage, 10, "daily_mismatch");
    assert_eq!(mismatch(&gone), Some(i64::from(Coverage::STEADY_MISMATCH)), "and the daily row leaving with it confirms nothing");
    let (batch, _) = through(0..=PAIRED);
    assert_eq!((mismatch(&batch), covered(&batch)), (Some(0), Some(true)));
    let (kept, _) = through(0..=TOTAL_KEPT);
    assert_eq!(mismatch(&kept), Some(i64::from(Coverage::STEADY_MISMATCH)), "a deletion whose remaining costs miss the total read that hour is a mismatch");
    assert_eq!(covered(&kept), Some(false), "and its unchanged survivor is no longer covered");
}

#[test]
fn the_first_hour_after_a_blocked_one_reconciles_each_open_batch_with_its_current_rows() {
    let (store, _) = through(0..=25);
    let costs = cost_items(&store);
    let (relation, organisation) = (part_key(10, POLITICAL, costs[0]), part_key(10, POLITICAL, costs[1]));
    let covered = |store: &Store, key: u128| {
        let view = store.view(TableId::PowerDayParts.index());
        view.bool(column::<Part>("covered"), view.slot(key).expect("a row"))
    };
    let (published, _) = through(0..=192);
    assert_eq!(keys(&published, TableId::PowerDayParts), [relation, organisation]);
    let (shrunk, _) = through(0..=SHRUNK);
    assert_eq!(keys(&shrunk, TableId::PowerDayParts), [relation], "a cost removed while the ledger was blocked withdraws its part");
    assert_eq!(covered(&shrunk, relation), covered(&published, relation), "and the survivor keeps the coverage it proved");
    let (settled, _) = through(0..=216);
    assert_eq!(spend(&settled, 10, POLITICAL, costs[1], "days"), spend(&shrunk, 10, POLITICAL, costs[1], "days"), "day 192 settles without the withdrawn cost");
    let (before, _) = through(0..=168);
    assert_eq!(keys(&before, TableId::PowerDayParts), [relation]);
    let (blocked, _) = through(0..=GROWN_BLOCKED);
    assert!(matches!(blocked.view(TableId::PowerDayParts.index()).status(), Status::Unavailable { .. }), "the links not read block the ledger");
    let (grown, _) = through(0..=GROWN);
    assert_eq!(keys(&grown, TableId::PowerDayParts), [relation, organisation], "a cost added while the ledger was blocked opens its part");
    assert_eq!(cell::<Part>(&grown, TableId::PowerDayParts, organisation, "spent"), Some(Q / 10));
    assert_eq!((covered(&grown, relation), covered(&grown, organisation)), (Some(true), Some(false)), "an unchanged part keeps the coverage it proved, and a new one needs a capture since its day began");
    assert_eq!(cell::<Part>(&grown, TableId::PowerDayParts, relation, "read"), Some(168));
    for name in ["day_payments", "day_charge", "days"] {
        for (holder, ordinal) in [(10, costs[0]), (10, costs[1]), (30, 0), (30, 1), (40, 0)] {
            assert_eq!(spend(&grown, holder, POLITICAL, ordinal, name), spend(&before, holder, POLITICAL, ordinal, name), "reconciling settles no closed day again ({name})");
        }
    }
}

#[test]
fn a_country_that_changes_holders_mid_day_takes_its_open_batch_with_it() {
    let (store, _) = through(0..=216);
    let held = |store: &Store, holder: u32| keys(store, TableId::PowerDayParts).into_iter().filter(|&key| (key >> 40) as u32 == holder).collect::<Vec<_>>();
    assert_eq!(held(&store, 10).len(), 2);
    let (moved, _) = through(0..=MOVED);
    assert_eq!(held(&moved, 10), Vec::<u128>::new(), "the old holder keeps none of the batch");
    let spent = |store: &Store, ordinal: u32| cell::<Part>(store, TableId::PowerDayParts, part_key(50, POLITICAL, ordinal), "spent");
    assert_eq!(held(&moved, 50), [part_key(50, POLITICAL, 0), part_key(50, POLITICAL, 1)], "the new holder reads it again");
    assert_eq!((spent(&moved, 0), spent(&moved, 1)), (Some(Q / 2), Some(Q / 10)));
    let (deleted, _) = through(0..=MOVED_DELETED);
    assert_eq!(held(&deleted, 50), [part_key(50, POLITICAL, 0)], "a cost deleted after the move is withdrawn under the new holder");
    assert_eq!(held(&deleted, 10), Vec::<u128>::new());
    let (next, _) = through(0..=240);
    for ordinal in cost_items(&store) {
        assert_eq!(spend(&next, 10, POLITICAL, ordinal, "days"), spend(&store, 10, POLITICAL, ordinal, "days"), "day 216 settles nothing under the old holder");
    }
    assert!(!next.has_item_activity(50, POLITICAL, 1), "nor the deleted cost under the new one");
    assert_eq!((spend(&next, 50, POLITICAL, 0, "days"), spend(&next, 50, POLITICAL, 0, "day_charge")), (Some(1), Some(Q / 2)));
    let (overnight, _) = through(0..=OVERNIGHT);
    assert!(!overnight.has_item_activity(50, POLITICAL, 1), "a cost removed in a blocked hour before midnight is withdrawn before its day settles");
    assert_eq!((spend(&overnight, 50, POLITICAL, 0, "days"), spend(&overnight, 50, POLITICAL, 0, "day_charge")), (Some(2), Some(Q)));
    let (replaced, _) = through(0..=REPLACED);
    assert_eq!((spend(&replaced, 50, POLITICAL, 0, "day_unknown"), spend(&replaced, 50, POLITICAL, 1, "day_unknown")), (Some(1), Some(1)), "a batch replaced by the next day's while the ledger was blocked settles its day as unknown");
    assert_eq!(spend(&replaced, 50, POLITICAL, 0, "day_charge"), Some(Q), "and charges nothing it could not prove");
    let (blocked, _) = through(0..=REVISED_BLOCKED);
    let (revised, _) = through(0..=REVISED);
    let grew = |name: &str| spend(&revised, 50, POLITICAL, 0, name).zip(spend(&blocked, 50, POLITICAL, 0, name)).map(|(after, before)| after - before);
    assert_eq!((grew("day_payments"), grew("day_charge")), (Some(1), Some(Q / 4)), "a cost revised on the first hour after a gap settles its day once, at its revision");
    let (unlinked, _) = through(0..=UNLINKED_AGAIN);
    assert!(held(&unlinked, NONE).is_empty(), "a country losing its link opens nothing under the no-holder identity");
    assert_eq!((spent(&unlinked, 0), spent(&unlinked, 1)), (None, None), "and the batch it left reads as unknown");
    let (relinked, _) = through(0..=RELINKED);
    assert_eq!((spent(&relinked, 0), spent(&relinked, 1)), (Some(Q / 4), Some(Q / 10)), "until its holder publishes it again");
    let (waiting, _) = through(0..=DEFERRED_BLOCKED);
    let (deferred, _) = through(0..=DEFERRED);
    assert_eq!(
        deferred.view(TableId::PowerSpendDays.index()).status(),
        Status::Unavailable { reason: 1 },
        "an unavailable midnight roll hour still marks PowerSpendDays unavailable"
    );
    for ordinal in [0, 1] {
        let unknown = |store: &LandedLogs| spend(store, 50, POLITICAL, ordinal, "day_unknown");
        assert_eq!(unknown(&deferred).zip(unknown(&waiting)).map(|(after, before)| after - before), Some(1), "a day that closes while its reconciliation waits for the cost rows settles as unknown");
    }
    let (swapped, _) = through(0..=SWAPPED);
    let part = |holder: u32, ordinal: u32| cell::<Part>(&swapped, TableId::PowerDayParts, part_key(holder, POLITICAL, ordinal), "spent");
    assert_eq!((part(20, 0), part(20, 1), part(50, 0)), (Some(Q / 2), Some(Q / 10), Some(Q / 3)), "two countries swapping holders each take their batch to the other's");
    assert!(held(&swapped, 50).iter().all(|&key| key != part_key(50, POLITICAL, 1)));
    let view = swapped.view(TableId::PowerDayParts.index());
    assert_eq!(view.bool(column::<Part>("covered"), view.slot(part_key(20, POLITICAL, 0)).expect("a row")), Some(true), "and a part arriving on an ordinal the other withdrew keeps none of its coverage or read hour");
    let (contradicted, _) = through(0..=CONTRADICTED);
    let after = |ordinal: u32| cell::<Part>(&contradicted, TableId::PowerDayParts, part_key(20, POLITICAL, ordinal), "spent");
    assert_eq!((after(0), after(1)), (None, None), "rows gone while the application read that hour no longer adds up to the held parts leave them unknown");
    assert_eq!(cell::<Coverage>(&contradicted, TableId::PowerSpendCoverage, 20, "daily_mismatch"), Some(i64::from(Coverage::STEADY_MISMATCH)), "and name the holder's costs as not adding up");
    let (kept, _) = through(0..=EMPTIED_KEPT);
    assert_eq!(held(&kept, 20).len(), 2, "rows gone before the application says why withdraw nothing");
    let (moved_empty, _) = through(0..=MOVED_EMPTY);
    let left = |ordinal: u32| cell::<Part>(&moved_empty, TableId::PowerDayParts, part_key(20, POLITICAL, ordinal), "spent");
    assert_eq!((left(0), left(1)), (None, None), "a country moving with no rows to read again leaves its parts unknown under the old holder rather than withdrawn");
    assert!(held(&moved_empty, 60).is_empty());
    let delayed_part = |store: &Store, ordinal: u32| cell::<Part>(store, TableId::PowerDayParts, part_key(60, POLITICAL, ordinal), "spent");
    let (unread, _) = through(0..=DELAYED_UNREAD);
    assert_eq!((delayed_part(&unread, 0), delayed_part(&unread, 1)), (None, None), "rows gone during a gap with no daily application read to confirm them leave the parts unknown");
    let (delayed, _) = through(0..=DELAYED);
    assert_eq!((delayed_part(&delayed, 0), delayed_part(&delayed, 1)), (None, None), "and a later daily read gives them back no amount");
    let (retotalled, _) = through(0..=RETOTALLED);
    assert_eq!((delayed_part(&retotalled, 0), delayed_part(&retotalled, 1)), (Some(Q / 2), Some(Q / 10)), "unchanged rows keep their parts");
    assert_eq!(cell::<Coverage>(&retotalled, TableId::PowerSpendCoverage, 60, "daily_mismatch"), Some(i64::from(Coverage::STEADY_MISMATCH)), "but a daily total changed during the gap names the holder's mismatch");
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
    for (stop, end) in [(24, 49), (MOVED - 1, MOVED_DELETED), (GROWN_BLOCKED, GROWN)] {
        let (unbroken, _) = through(0..=end);
        let (stopped, before) = through(0..=stop);
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for at in stop + 1..=end {
            run(&mut resumed, &mut after, at);
        }
        for table in TABLES.iter().map(|output| output.table()) {
            assert_eq!(testing::rows(&resumed, table), testing::rows(&unbroken, table), "{table:?} resumed after hour {stop}");
        }
        for table in [TableId::PowerSpendDays, TableId::PowerSpendPayments, TableId::PowerSpendGains] {
            assert_eq!(
                testing::listed(&resumed, table),
                testing::listed(&unbroken, table),
                "{table:?} listed rows resumed after hour {stop}"
            );
        }
    }
}

#[test]
fn its_blob_holds_hours_links_recovery_settled_days_and_open_batches_and_a_damaged_one_is_refused() {
    let key = BatchKey { raw: 1, epoch: 1, day: 24, sequence: 50 };
    let batch = CostBatch {
        holder: 10,
        unmatched: false,
        parts: [(0, CostPart { spent: Recorded::Value(-Q / 4), payments: 2, covered: true, cost_ordinal: 0, read: 25 }), (1, CostPart { spent: Recorded::NotRecorded, payments: 1, covered: false, cost_ordinal: 1, read: 26 })].into(),
    };
    let mut runs = DayRuns::default();
    for day in [0, 24, 72] {
        runs.insert(day);
    }
    let mut item_counted = DayRuns::default();
    for day in [24, 72] {
        item_counted.insert(day);
    }
    let mut item_uncovered = DayRuns::default();
    item_uncovered.insert(72);
    let ledger = PowerLedger {
        last: Some(5),
        run_first: Some(-3),
        recover: true,
        links: [(1, 10), (2, 10), (3, 20)].into(),
        mismatched: [2].into(),
        settled: [(1, runs), (2, DayRuns(vec![(48, 48)]))].into(),
        item_days: [((10, POLITICAL, 0), ItemDays { counted: item_counted, uncovered: item_uncovered })].into(),
        batches: [(key, batch)].into(),
        seen_changes: [(1, (1, 5))].into(),
        ..PowerLedger::default()
    };
    let mut blob = Bytes::new();
    ledger.save(&mut blob).unwrap();
    assert!(blob.bytes().len() <= 96, "two hours, a flag, links, mismatch, settled runs, item days, one open batch and seen changes");
    let mut loaded = PowerLedger::default();
    loaded.load(blob.bytes()).unwrap();
    assert!(loaded.rebuild && loaded.recover);
    assert_eq!((loaded.last, loaded.run_first), (Some(5), Some(-3)));
    assert_eq!(loaded.links, ledger.links);
    assert_eq!(loaded.mismatched, ledger.mismatched);
    assert_eq!(loaded.settled, ledger.settled);
    assert_eq!(loaded.item_days, ledger.item_days);
    assert_eq!(loaded.batches, ledger.batches);
    assert_eq!(loaded.seen_changes, ledger.seen_changes);
    assert_eq!(loaded.raw_batches(1), [key]);
    assert_eq!(loaded.holder_batches(10), [key]);
    assert_eq!(loaded.aliases.get(&10).map(Vec::as_slice), Some(&[1, 2][..]), "the holders are rebuilt from the links");
    assert_eq!(loaded.aliases.get(&20).map(Vec::as_slice), Some(&[3][..]));
    assert!(loaded.load(&blob.bytes()[..blob.bytes().len() - 1]).is_err());

    let raw_batch_blob = |unmatched: u64, parts: &[(u32, u64, u64, u32)]| {
        let mut raw = Bytes::new();
        for _ in 0..7 {
            raw.varint(0).unwrap();
        }
        raw.varint(1).unwrap();
        raw.varint(1).unwrap();
        raw.varint(1).unwrap();
        raw.varint(24).unwrap();
        raw.varint(50).unwrap();
        raw.varint(10).unwrap();
        raw.varint(unmatched).unwrap();
        raw.varint(parts.len() as u64).unwrap();
        for &(ordinal, flags, payments, cost_ordinal) in parts {
            raw.varint(u64::from(ordinal)).unwrap();
            raw.varint(flags).unwrap();
            if flags & 2 != 0 {
                raw.varint(encode_signed(Q)).unwrap();
            }
            raw.varint(payments).unwrap();
            raw.varint(u64::from(cost_ordinal)).unwrap();
            raw.varint(24).unwrap();
        }
        raw.into_bytes()
    };
    for bad in [
        raw_batch_blob(0, &[(0, 3, 0, 0)]),
        raw_batch_blob(0, &[(0, 3, MAX_BATCH_PAYMENTS + 1, 0)]),
        raw_batch_blob(0, &[(Total::GAINS, 3, 1, 0)]),
        raw_batch_blob(1, &[(0, 3, 1, 0)]),
        raw_batch_blob(0, &[(0, 1, 1, 0)]),
        raw_batch_blob(0, &[(0, 3, 2, u32::MAX)]),
        raw_batch_blob(0, &[(0, 3, MAX_BATCH_PAYMENTS, 0), (1, 3, 1, 1)]),
        raw_batch_blob(0, &[(0, 3, 1, 5), (1, 3, 1, 5)]),
        raw_batch_blob(0, &[(0, 3, 2, u32::MAX - 1), (1, 3, 1, u32::MAX)]),
    ] {
        assert!(loaded.load(&bad).is_err(), "structurally invalid batch resume state must be refused");
    }
    let bad_save = PowerLedger {
        batches: [(
            key,
            CostBatch {
                holder: 10,
                unmatched: false,
                parts: [
                    (0, CostPart { spent: Recorded::Value(Q), payments: 2, covered: true, cost_ordinal: u32::MAX - 1, read: 24 }),
                    (1, CostPart { spent: Recorded::Value(Q), payments: 1, covered: true, cost_ordinal: u32::MAX, read: 24 }),
                ]
                .into(),
            },
        )]
        .into(),
        ..PowerLedger::default()
    };
    let mut bad_out = Bytes::new();
    assert!(bad_save.save(&mut bad_out).is_err(), "save must also reject structurally impossible batch payment bounds");
    let bad_item_save = PowerLedger {
        item_days: [((10, POLITICAL, 0), ItemDays { counted: DayRuns(vec![(24, 24)]), uncovered: DayRuns(vec![(48, 48)]) })].into(),
        ..PowerLedger::default()
    };
    let mut bad_item_out = Bytes::new();
    assert!(bad_item_save.save(&mut bad_item_out).is_err(), "save must reject uncovered days outside counted days");
    let mut bad_item_blob = Bytes::new();
    for value in [0u64, 0, 0, 0, 0, 0, 1, 10, 0, 0, 1, 24, 0, 1, 48, 0, 0] {
        bad_item_blob.varint(value).unwrap();
    }
    assert!(loaded.load(bad_item_blob.bytes()).is_err(), "load must reject uncovered days outside counted days");
    let mut bad_runs = Bytes::new();
    for value in [0u64, 0, 0, 0, 0, 1, 1, 2, 24, 0, 48, 0, 0, 0] {
        bad_runs.varint(value).unwrap();
    }
    assert!(loaded.load(bad_runs.bytes()).is_err(), "unmerged adjacent settled runs must be refused");
}

fn step_with(store: &mut Store, writers: &mut Writers, hour: i32, f: impl FnOnce(&mut Store, i32)) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    f(store, hour);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn setup_base(store: &mut Store, hour: i32, countries: &[(u32, u32)], witnessed: &[u32]) {
    let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
    if hour == 0 {
        for table in [TableId::Countries, TableId::CountryLinks, TableId::CountryPowerDaily, TableId::CountryPowerDailyCosts, TableId::CountryCommandPower, TableId::CountryPowerChanges] {
            complete(store, table);
        }
        for &(id, slot) in countries {
            testing::put(store, TableId::Countries, &CountryRow { id, canonical_slot: slot, ..Default::default() }, TEXTS).unwrap();
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: slot }, TEXTS).unwrap();
            testing::put(store, TableId::CountryCommandPower, &CountryCommandPowerRow { canonical_slot: slot, ..Default::default() }, TEXTS).unwrap();
        }
    }
    complete(store, TableId::PoliticsCaptureCoverage);
    for &country in witnessed {
        let witness = Streams { country, epoch: 1, since_hour: 0, through_hour: hour, streams: Streams::ALL_STREAMS, ..Default::default() };
        testing::put(store, TableId::PoliticsCaptureCoverage, &witness, TEXTS).unwrap();
    }
}

fn run_custom(end: i32, f: impl Fn(&mut Store, i32)) -> (LandedLogs, Writers) {
    let (mut logged, mut writers) = (LandedLogs::new(testing::store()), writers());
    for at in 0..=end {
        step_with(&mut logged.store, &mut writers, at, |s, h| f(s, h));
        logged.record_landed_rows();
    }
    (logged, writers)
}

fn assert_resumes_match(stops: &[i32], end: i32, f: impl Fn(&mut Store, i32)) {
    let (unbroken, _) = run_custom(end, &f);
    for &stop in stops {
        let (stopped, before) = run_custom(stop, &f);
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for at in stop + 1..=end {
            step_with(&mut resumed, &mut after, at, |s, h| f(s, h));
        }
        for table in TABLES.iter().map(|output| output.table()) {
            assert_eq!(testing::rows(&resumed, table), testing::rows(&unbroken, table), "{table:?} resumed after hour {stop}");
        }
        for table in [TableId::PowerSpendDays, TableId::PowerSpendPayments, TableId::PowerSpendGains] {
            assert_eq!(
                testing::listed(&resumed, table),
                testing::listed(&unbroken, table),
                "{table:?} listed rows resumed after hour {stop}"
            );
        }
    }
}

#[test]
fn late_revision_of_a_settled_day_does_not_settle_again() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 20)], &[1, 2]);
        let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            48 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 4, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 50), TEXTS).unwrap();
            }
            49 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 3, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 3, 50), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 60) }, TEXTS).unwrap();
            }
            50 => {
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 4, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 60) }, TEXTS).unwrap();
            }
            72 => {
                put_daily(store, hour, &daily(72, 3 * Q, Q / 2, 72));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 0, PowerDailyCostKind::Relation, Q / 2, 72), TEXTS).unwrap();
            }
            95 => {
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
                put_daily(store, hour, &daily(72, 3 * Q, Q / 3, 72));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 0, PowerDailyCostKind::Relation, Q / 3, 72), TEXTS).unwrap();
            }
            96 => {
                complete(store, TableId::CountryLinks);
                store.status(TableId::CountryPowerDailyCosts.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
            }
            97 => {
                complete(store, TableId::CountryPowerDailyCosts);
                put_daily(store, hour, &daily(72, 3 * Q, Q / 4, 72));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 0, PowerDailyCostKind::Relation, Q / 4, 72), TEXTS).unwrap();
            }
            120 => {
                put_daily(store, hour, &daily(120, 3 * Q, Q / 2 - Q / 10, 120));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(120, 0, PowerDailyCostKind::Relation, Q / 2, 120), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(120, 1, PowerDailyCostKind::MioResearch, -Q / 10, 120), TEXTS).unwrap();
            }
            143 => {
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
            }
            144 => {
                complete(store, TableId::CountryLinks);
                store.status(TableId::CountryPowerChanges.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
                store.status(TableId::CountryPowerDailyCosts.index(), Status::Truncated { missing: 1 }, true, Some(hour), false).unwrap();
            }
            145 => {
                complete(store, TableId::CountryPowerChanges);
                complete(store, TableId::CountryPowerDailyCosts);
            }
            _ => {}
        }
    };
    let (midnight_revised, _) = run_custom(48, scenario);
    assert_eq!(
        (spend(&midnight_revised, 10, POLITICAL, 0, "days"), spend(&midnight_revised, 10, POLITICAL, 0, "day_payments"), spend(&midnight_revised, 10, POLITICAL, 0, "day_charge")),
        (Some(1), Some(1), Some(Q / 2)),
        "a late revision written on the midnight roll hour does not settle day 24 a second time"
    );
    let (later_revised, _) = run_custom(49, scenario);
    assert_eq!(
        (spend(&later_revised, 10, POLITICAL, 0, "days"), spend(&later_revised, 10, POLITICAL, 0, "day_payments"), spend(&later_revised, 10, POLITICAL, 0, "day_charge")),
        (Some(1), Some(1), Some(Q / 2)),
        "a late revision written after midnight does not settle day 24 a second time"
    );
    assert_eq!(
        (spend(&later_revised, 20, POLITICAL, 0, "days"), spend(&later_revised, 20, POLITICAL, 0, "day_payments"), spend(&later_revised, 20, POLITICAL, 0, "day_charge")),
        (Some(1), Some(1), Some(Q / 2)),
        "a closed day never settled before settles once on its first late publication"
    );
    let (twice_late, _) = run_custom(50, scenario);
    assert_eq!(
        (spend(&twice_late, 20, POLITICAL, 0, "days"), spend(&twice_late, 20, POLITICAL, 0, "day_payments"), spend(&twice_late, 20, POLITICAL, 0, "day_charge")),
        (Some(1), Some(1), Some(Q / 2)),
        "a second late revision of that closed day does not settle it again"
    );
    let (withheld, _) = run_custom(96, scenario);
    assert_eq!(
        withheld.view(TableId::PowerSpendDays.index()).status(),
        Status::Unavailable { reason: 1 },
        "an unavailable midnight roll hour still marks PowerSpendDays unavailable"
    );
    assert_eq!(
        (spend(&withheld, 10, POLITICAL, 0, "days"), spend(&withheld, 10, POLITICAL, 0, "day_payments"), spend(&withheld, 10, POLITICAL, 0, "day_unknown"), spend(&withheld, 10, POLITICAL, 0, "day_charge")),
        (Some(2), Some(2), Some(1), Some(Q / 2)),
        "day 72 closes as unknown while recovery waits for daily costs"
    );
    let (after_deferred, _) = run_custom(97, scenario);
    for name in ["days", "day_payments", "day_unknown", "day_charge"] {
        assert_eq!(
            spend(&after_deferred, 10, POLITICAL, 0, name),
            spend(&withheld, 10, POLITICAL, 0, name),
            "a late revision after unknown-withheld settlement does not settle day 72 again ({name})"
        );
    }
    let (withheld_both, _) = run_custom(144, scenario);
    assert_eq!(
        withheld_both.view(TableId::PowerSpendDays.index()).status(),
        Status::Unavailable { reason: 1 },
        "simultaneous changes unavailability and daily-cost truncation still marks PowerSpendDays unavailable"
    );
    assert_eq!(
        withheld_both.view(TableId::PowerSpendPayments.index()).status(),
        Status::Unavailable { reason: 1 },
        "CountryPowerChanges unavailability still marks PowerSpendPayments unavailable even when an unknown negative daily credit lands"
    );
    assert_eq!(
        (spend(&withheld_both, 10, POLITICAL, 0, "days"), spend(&withheld_both, 10, POLITICAL, 0, "day_unknown"), spend(&withheld_both, 10, POLITICAL, 1, "unknown")),
        (Some(3), Some(2), Some(1)),
        "day 120 still lands both its unknown positive day and its unknown negative credit"
    );
    assert_resumes_match(&[48, 49, 96, 144], 145, scenario);
}

#[test]
fn no_holder_identity_is_followed_across_unlink_publish_and_relink() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1, 2]);
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2 + Q / 10, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 1, PowerDailyCostKind::MioResearch, Q / 10, 50), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 60) }, TEXTS).unwrap();
            }
            25 => {
                testing::remove(store, TableId::CountryLinks, 1).unwrap();
                testing::put(store, TableId::Countries, &CountryRow { id: 2, canonical_slot: 20, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 2, canonical_slot: 20 }, TEXTS).unwrap();
            }
            26 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 4 + Q / 10, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 50), TEXTS).unwrap();
            }
            27 => {
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 10 }, TEXTS).unwrap();
                put_daily(store, hour, &daily(24, 3 * Q, Q / 5 + Q / 10, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 5, 50), TEXTS).unwrap();
            }
            48 => {
                testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
                put_daily(store, hour, &daily(48, 3 * Q, Q / 2, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap();
            }
            49 => {
                testing::remove(store, TableId::CountryLinks, 1).unwrap();
            }
            50 => {
                put_daily(store, hour, &daily(48, 3 * Q, Q / 3, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 3, 80), TEXTS).unwrap();
            }
            51 => {
                testing::put(store, TableId::Countries, &CountryRow { id: 1, canonical_slot: 30, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 30 }, TEXTS).unwrap();
            }
            52 => {
                put_daily(store, hour, &daily(48, 3 * Q, Q / 4, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 4, 80), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let held = |store: &Store, holder: u32| keys(store, TableId::PowerDayParts).into_iter().filter(|&key| (key >> 40) as u32 == holder).collect::<Vec<_>>();
    let has_holder_activity = |logged: &LandedLogs, holder: u32| {
        logged.days.keys().any(|&(h, _, _)| h == holder) || logged.payments.keys().any(|&(h, _, _)| h == holder)
    };
    let (h25, _) = run_custom(25, scenario);
    assert!(held(&h25, NONE).is_empty(), "country 2 moved from NONE to 20 and country 1's link loss opened nothing under NONE");
    assert_eq!(cell::<Part>(&h25, TableId::PowerDayParts, part_key(20, POLITICAL, 0), "spent"), Some(Q / 2), "country 2's batch moved from NONE to 20 without republishing");
    assert_eq!(cell::<Part>(&h25, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "spent"), None, "country 1's link loss marks holder 10's parts unknown");
    let (h26, _) = run_custom(26, scenario);
    assert!(held(&h26, 10).is_empty(), "publishing while unlinked withdraws the old unknown copy from holder 10");
    assert_eq!(cell::<Part>(&h26, TableId::PowerDayParts, part_key(NONE, POLITICAL, 0), "spent"), Some(Q / 4));
    let (h27, _) = run_custom(27, scenario);
    assert!(held(&h27, NONE).is_empty(), "relinking and publishing withdraws the copy under NONE");
    assert_eq!(cell::<Part>(&h27, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "spent"), Some(Q / 5));
    let (h48, _) = run_custom(48, scenario);
    assert!(!has_holder_activity(&h48, NONE), "nothing settles under NONE once both countries linked");
    assert_eq!((spend(&h48, 10, POLITICAL, 0, "days"), spend(&h48, 10, POLITICAL, 0, "day_payments"), spend(&h48, 10, POLITICAL, 0, "day_charge")), (Some(1), Some(1), Some(Q / 5)));
    assert_eq!((spend(&h48, 20, POLITICAL, 0, "days"), spend(&h48, 20, POLITICAL, 0, "day_payments"), spend(&h48, 20, POLITICAL, 0, "day_charge")), (Some(1), Some(1), Some(Q / 2)));
    let (h51, _) = run_custom(51, scenario);
    assert!(held(&h51, NONE).is_empty(), "relinking to a different holder moves the unlinked batch from NONE");
    assert_eq!(cell::<Part>(&h51, TableId::PowerDayParts, part_key(30, POLITICAL, 0), "spent"), Some(Q / 3));
    let (h72, _) = run_custom(72, scenario);
    assert!(!has_holder_activity(&h72, NONE), "nothing settles under NONE for day 48 either");
    assert_eq!((spend(&h72, 10, POLITICAL, 0, "days"), spend(&h72, 10, POLITICAL, 0, "day_charge")), (Some(1), Some(Q / 5)), "holder 10 did not settle day 48");
    assert_eq!((spend(&h72, 30, POLITICAL, 0, "days"), spend(&h72, 30, POLITICAL, 0, "day_payments"), spend(&h72, 30, POLITICAL, 0, "day_charge")), (Some(1), Some(1), Some(Q / 4)));
    assert_resumes_match(&[24, 26, 50], 72, scenario);
}

#[test]
fn two_applications_on_one_holder_keep_independent_parts_normally_and_across_moves() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 10), (3, 10)], &[1, 2, 3]);
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            25 => {
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 4, 50) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 50) }, TEXTS).unwrap();
            }
            48 => {
                testing::put(store, TableId::Countries, &CountryRow { id: 2, canonical_slot: 20, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 2, canonical_slot: 20 }, TEXTS).unwrap();
                put_daily(store, hour, &daily(48, 3 * Q, Q / 2, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(48, 3 * Q, Q / 3, 90) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(48, 0, PowerDailyCostKind::Relation, Q / 3, 90) }, TEXTS).unwrap();
            }
            49 => {
                testing::put(store, TableId::Countries, &CountryRow { id: 2, canonical_slot: 10, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 2, canonical_slot: 10 }, TEXTS).unwrap();
            }
            50 => {
                testing::put(store, TableId::Countries, &CountryRow { id: 2, canonical_slot: 30, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 2, canonical_slot: 30 }, TEXTS).unwrap();
            }
            72 => {
                put_daily(store, hour, &daily(72, 3 * Q, Q / 2 + Q / 10, 100));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 0, PowerDailyCostKind::MioResearch, Q / 10, 100), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 1, PowerDailyCostKind::Relation, Q / 2, 100), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(72, 3 * Q, Q / 4, 101) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(72, 0, PowerDailyCostKind::Relation, Q / 4, 101) }, TEXTS).unwrap();
            }
            73 => {
                testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
                put_daily(store, hour, &daily(72, 3 * Q, Q / 10, 100));
            }
            _ => {}
        }
    };
    let held = |store: &Store, holder: u32| keys(store, TableId::PowerDayParts).into_iter().filter(|&key| (key >> 40) as u32 == holder).collect::<Vec<_>>();
    let (h25, _) = run_custom(25, scenario);
    assert_eq!(
        (cell::<Part>(&h25, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "spent"), cell::<Part>(&h25, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "payments")),
        (Some(Q / 2 + Q / 4), Some(2)),
        "two countries publishing the same item on one holder sum into its open part instead of overwriting"
    );
    let (h48, _) = run_custom(48, scenario);
    assert_eq!(
        (spend(&h48, 10, POLITICAL, 0, "days"), spend(&h48, 10, POLITICAL, 0, "day_payments"), spend(&h48, 10, POLITICAL, 0, "day_charge"), spend(&h48, 10, POLITICAL, 0, "day_unknown")),
        (Some(1), Some(2), Some(Q / 2 + Q / 4), Some(0)),
        "both applications on one holder settle at midnight"
    );
    let (h49, _) = run_custom(49, scenario);
    assert!(held(&h49, 20).is_empty(), "country 2 left holder 20");
    assert_eq!(
        (cell::<Part>(&h49, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "spent"), cell::<Part>(&h49, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "payments")),
        (Some(Q / 2 + Q / 3), Some(2)),
        "a holder move onto a holder paying the same item keeps both applications"
    );
    let (h50, _) = run_custom(50, scenario);
    assert_eq!(
        (cell::<Part>(&h50, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "spent"), cell::<Part>(&h50, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "payments")),
        (Some(Q / 2), Some(1)),
        "when country 2 moves away, country 1's part stays intact"
    );
    assert_eq!(cell::<Part>(&h50, TableId::PowerDayParts, part_key(30, POLITICAL, 0), "spent"), Some(Q / 3));
    let (h72, _) = run_custom(72, scenario);
    assert_eq!(spend(&h72, 10, POLITICAL, 0, "day_charge"), Some(Q / 2 + Q / 4 + Q / 2));
    assert_eq!(spend(&h72, 30, POLITICAL, 0, "day_charge"), Some(Q / 3));
    let (h73, _) = run_custom(73, scenario);
    assert_eq!(
        (cell::<Part>(&h73, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "spent"), cell::<Part>(&h73, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "payments")),
        (Some(Q / 4), Some(1)),
        "country 1 dropping its shared item leaves country 3's part intact"
    );
    assert_eq!(cell::<Part>(&h73, TableId::PowerDayParts, part_key(10, POLITICAL, 1), "spent"), Some(Q / 10));
    assert_resumes_match(&[25, 49, 72], 96, scenario);
}

#[test]
fn out_of_order_closed_days_and_confirmed_empty_replacements_settle_honestly() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 10)], &[1, 2]);
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 4, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 60) }, TEXTS).unwrap();
            }
            25 => {
                testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap();
                put_daily(store, hour, &daily(24, 3 * Q, 0, 51));
            }
            48 => {
                put_daily(store, hour, &daily(48, 3 * Q, Q / 2, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(48, 3 * Q, Q / 3, 90) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(48, 0, PowerDailyCostKind::Relation, Q / 3, 90) }, TEXTS).unwrap();
            }
            49 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            72 => {
                testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32).unwrap();
                testing::remove(store, TableId::CountryPowerDailyCosts, 2 << 32).unwrap();
            }
            73 => {
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(48, 3 * Q, Q / 5, 110) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(48, 0, PowerDailyCostKind::Relation, Q / 5, 110) }, TEXTS).unwrap();
            }
            74 => {
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(24, 3 * Q, Q / 4, 100) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 100) }, TEXTS).unwrap();
            }
            75 => {
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(24, 3 * Q, Q / 2, 100) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 100) }, TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h25, _) = run_custom(25, scenario);
    assert_eq!(
        (cell::<Part>(&h25, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "spent"), cell::<Part>(&h25, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "payments")),
        (Some(Q / 4), Some(1)),
        "confirmed zero-cost replacement with a new sequence withdraws country 1's prior batch while leaving country 2 intact"
    );
    let (h48, _) = run_custom(48, scenario);
    assert_eq!(
        (spend(&h48, 10, POLITICAL, 0, "days"), spend(&h48, 10, POLITICAL, 0, "day_payments"), spend(&h48, 10, POLITICAL, 0, "day_charge")),
        (Some(1), Some(1), Some(Q / 4)),
        "only country 2 settles day 24 at midnight"
    );
    let (h49, _) = run_custom(49, scenario);
    assert_eq!(
        (spend(&h49, 10, POLITICAL, 0, "day_payments"), spend(&h49, 10, POLITICAL, 0, "day_charge")),
        (Some(1), Some(Q / 4)),
        "a stale revision of a day confirmed zero-cost does not settle after midnight"
    );
    let (h74, _) = run_custom(74, scenario);
    assert_eq!(
        (spend(&h74, NONE, POLITICAL, 0, "days"), spend(&h74, NONE, POLITICAL, 0, "day_payments"), spend(&h74, NONE, POLITICAL, 0, "day_charge")),
        (Some(2), Some(2), Some(Q / 5 + Q / 4)),
        "an unsettled earlier day still settles once even after a later day settled first"
    );
    let (h75, _) = run_custom(75, scenario);
    assert_eq!(
        (spend(&h75, NONE, POLITICAL, 0, "days"), spend(&h75, NONE, POLITICAL, 0, "day_payments"), spend(&h75, NONE, POLITICAL, 0, "day_charge")),
        (Some(2), Some(2), Some(Q / 5 + Q / 4)),
        "a repeat revision of that earlier day is then deduplicated"
    );
    assert_resumes_match(&[25, 48, 73, 74], 75, scenario);
}

#[test]
fn unlinked_deletions_blocked_unlinks_and_same_holder_relinks_follow_the_batch() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 20), (3, 30)], &[1, 2, 3]);
        let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2 + Q / 10, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 1, PowerDailyCostKind::MioResearch, Q / 10, 50), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 60) }, TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(24, 3 * Q, Q / 2, 70) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 70) }, TEXTS).unwrap();
            }
            25 => {
                testing::remove(store, TableId::CountryLinks, 1).unwrap();
                testing::remove(store, TableId::CountryLinks, 3).unwrap();
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
            }
            26 => {
                complete(store, TableId::CountryLinks);
                testing::remove(store, TableId::CountryLinks, 2).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 4, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 60) }, TEXTS).unwrap();
            }
            27 => {
                testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 3, canonical_slot: 30 }, TEXTS).unwrap();
            }
            48 => {
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 10 }, TEXTS).unwrap();
                testing::put(store, TableId::Countries, &CountryRow { id: 4, canonical_slot: 40, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 4, canonical_slot: 40 }, TEXTS).unwrap();
                put_daily(store, hour, &daily(48, 3 * Q, Q / 2, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 4, ..daily(48, 3 * Q, Q / 2, 90) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 4, ..cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 90) }, TEXTS).unwrap();
            }
            71 => {
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 4, ..daily(48, 3 * Q, Q / 5, 90) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 4, ..cost(48, 0, PowerDailyCostKind::Relation, Q / 5, 90) }, TEXTS).unwrap();
            }
            72 => {
                complete(store, TableId::CountryLinks);
                testing::remove(store, TableId::CountryLinks, 1).unwrap();
                testing::remove(store, TableId::CountryLinks, 4).unwrap();
                put_daily(store, hour, &daily(48, 3 * Q, Q / 3, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 3, 80), TEXTS).unwrap();
            }
            73 => {
                put_daily(store, hour, &daily(48, 3 * Q, Q / 5, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 5, 80), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let held = |store: &Store, holder: u32| keys(store, TableId::PowerDayParts).into_iter().filter(|&key| (key >> 40) as u32 == holder).collect::<Vec<_>>();
    let (h26, _) = run_custom(26, scenario);
    assert!(held(&h26, 20).is_empty(), "unlinking with an actual cost write on the first usable hour after a block moves off the old holder");
    assert_eq!(cell::<Part>(&h26, TableId::PowerDayParts, part_key(NONE, POLITICAL, 0), "spent"), Some(Q / 4), "and publishes the revised cost under NONE");
    let (h27, _) = run_custom(27, scenario);
    assert!(held(&h27, 10).is_empty(), "a confirmed deletion while unlinked withdraws from the old holder and moves the survivor to NONE");
    assert_eq!(cell::<Part>(&h27, TableId::PowerDayParts, part_key(NONE, POLITICAL, 0), "spent"), Some(Q / 4 + Q / 2));
    assert_eq!(cell::<Part>(&h27, TableId::PowerDayParts, part_key(30, POLITICAL, 0), "spent"), Some(Q / 2), "relinking to the original holder rereads unchanged complete costs");
    let view = h27.view(TableId::PowerDayParts.index());
    assert_eq!(view.bool(column::<Part>("covered"), view.slot(part_key(30, POLITICAL, 0)).expect("a row")), Some(false), "while preserving the coverage break from the link loss");
    let (h48, _) = run_custom(48, scenario);
    assert!(!h48.has_item_activity(10, POLITICAL, 1), "withdrawn item contributes zero payments on holder 10");
    assert!(!h48.has_item_activity(NONE, POLITICAL, 1), "and zero payments on NONE");
    assert_eq!((spend(&h48, NONE, POLITICAL, 0, "day_payments"), spend(&h48, NONE, POLITICAL, 0, "day_charge")), (Some(2), Some(Q / 4 + Q / 2)));
    assert_eq!((spend(&h48, 30, POLITICAL, 0, "day_payments"), spend(&h48, 30, POLITICAL, 0, "day_charge"), spend(&h48, 30, POLITICAL, 0, "day_unknown")), (Some(1), Some(Q / 2), Some(0)));
    let (h72, _) = run_custom(72, scenario);
    assert!(!h72.has_item_activity(10, POLITICAL, 0), "midnight recovery with unlink and revised write settles no unknown payment on holder 10");
    assert_eq!(
        (spend(&h72, NONE, POLITICAL, 0, "days"), spend(&h72, NONE, POLITICAL, 0, "day_payments"), spend(&h72, NONE, POLITICAL, 0, "day_charge"), spend(&h72, NONE, POLITICAL, 0, "day_unknown")),
        (Some(2), Some(3), Some(Q / 4 + Q / 2 + Q / 3), Some(0)),
        "midnight recovery with unlink and revised matching write settles the revised payment under NONE"
    );
    assert_eq!(
        (spend(&h72, 40, POLITICAL, 0, "days"), spend(&h72, 40, POLITICAL, 0, "day_payments"), spend(&h72, 40, POLITICAL, 0, "day_unknown"), spend(&h72, 40, POLITICAL, 0, "day_charge")),
        (Some(1), Some(1), Some(1), Some(0)),
        "midnight recovery with unlink and no hour-72 write still settles unknown on its prior holder"
    );
    let (h73, _) = run_custom(73, scenario);
    assert_eq!(
        (spend(&h73, NONE, POLITICAL, 0, "day_payments"), spend(&h73, NONE, POLITICAL, 0, "day_charge")),
        (Some(3), Some(Q / 4 + Q / 2 + Q / 3)),
        "a stale day-48 revision after midnight recovery under NONE does not settle again"
    );
    assert_resumes_match(&[25, 26, 27, 71, 72], 73, scenario);
}

#[test]
fn confirmed_empty_days_from_outset_and_zero_revision_recovery_reject_stale_revisions() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 20), (3, 30)], &[1, 2, 3]);
        let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, 0, 50));
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 60) }, TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(24, 3 * Q, 0, 70) });
            }
            25 => {
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, 0, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, 0, 60) }, TEXTS).unwrap();
            }
            26 => {
                complete(store, TableId::CountryLinks);
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(24, 3 * Q, Q / 4, 71) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 71) }, TEXTS).unwrap();
            }
            48 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            49 => {
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 60) }, TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h48, _) = run_custom(48, scenario);
    assert!(
        !h48.has_item_activity(10, POLITICAL, 0),
        "a fresh zero-cost daily row with an empty cost table confirms day 24 empty and blocks stale revisions"
    );
    assert_eq!(
        (spend(&h48, 30, POLITICAL, 0, "days"), spend(&h48, 30, POLITICAL, 0, "day_charge")),
        (Some(1), Some(Q / 4)),
        "a same-day nonzero publication after a zero-cost confirmation still opens and settles at midnight"
    );
    let (h49, _) = run_custom(49, scenario);
    assert!(
        !h49.has_item_activity(20, POLITICAL, 0),
        "a positive cost revised to zero during recovery confirms day 24 empty and blocks stale revisions"
    );
    assert_resumes_match(&[24, 25, 26, 48], 49, scenario);
}

#[test]
fn command_power_only_daily_row_keeps_unresolved_political_mismatch() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 10)], &[1, 2]);
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2 + Q / 10, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 1, PowerDailyCostKind::MioResearch, Q / 10, 50), TEXTS).unwrap();
            }
            25 => {
                testing::remove(store, TableId::CountryPowerDaily, 1).unwrap();
                testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            }
            26 => {
                put_daily(
                    store,
                    hour,
                    &CountryPowerDailyRow {
                        country: 1,
                        epoch: 1,
                        day_hour: 24,
                        cp_gain_raw: Some(Q),
                        cp_realized_delta_raw: Some(Q),
                        cp_sequence: Some(90),
                        ..Default::default()
                    },
                );
            }
            27 => {
                put_daily(
                    store,
                    hour,
                    &CountryPowerDailyRow {
                        cp_gain_raw: Some(Q),
                        cp_realized_delta_raw: Some(Q),
                        cp_sequence: Some(90),
                        ..daily(24, 3 * Q, Q / 2, 50)
                    },
                );
            }
            _ => {}
        }
    };
    let (h26, _) = run_custom(26, scenario);
    assert_eq!(
        (cell::<Coverage>(&h26, TableId::PowerSpendCoverage, 10, "daily_mismatch"), cell::<Coverage>(&h26, TableId::PowerSpendCoverage, 10, "political")),
        (Some(i64::from(Coverage::STEADY_MISMATCH)), Some(0)),
        "a command-power-only daily row must not clear an unresolved political mismatch"
    );
    let (h27, _) = run_custom(27, scenario);
    assert_eq!(
        (cell::<Coverage>(&h27, TableId::PowerSpendCoverage, 10, "daily_mismatch"), cell::<Coverage>(&h27, TableId::PowerSpendCoverage, 10, "political")),
        (Some(0), Some(1)),
        "coverage restores once political rows reconcile"
    );
    assert_resumes_match(&[25, 26], 27, scenario);
}

#[test]
fn out_of_order_closed_days_count_distinct_days_and_uncovered_days_once() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 10)], &[1, 2]);
        let witness = |store: &mut Store, country: u32, since_hour: i32| {
            let row = Streams { country, epoch: 1, since_hour, through_hour: hour, streams: Streams::ALL_STREAMS, ..Default::default() };
            testing::put(store, TableId::PoliticsCaptureCoverage, &row, TEXTS).unwrap();
        };
        if hour >= 97 {
            witness(store, 1, 97);
            witness(store, 2, 97);
        }
        match hour {
            96 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            97 => {
                put_daily(store, hour, &daily(72, 3 * Q, Q / 4, 72));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(72, 0, PowerDailyCostKind::Relation, Q / 4, 72), TEXTS).unwrap();
            }
            98 => {
                put_daily(store, hour, &daily(48, 3 * Q, Q / 5, 48));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 5, 48), TEXTS).unwrap();
            }
            99 => {
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(48, 3 * Q, Q / 10, 49) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(48, 0, PowerDailyCostKind::Relation, Q / 10, 49) }, TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h99, _) = run_custom(99, scenario);
    assert_eq!(
        (
            spend(&h99, 10, POLITICAL, 0, "days"),
            spend(&h99, 10, POLITICAL, 0, "day_payments"),
            spend(&h99, 10, POLITICAL, 0, "days_uncovered"),
            spend(&h99, 10, POLITICAL, 0, "first_day"),
            spend(&h99, 10, POLITICAL, 0, "last_day"),
            spend(&h99, 10, POLITICAL, 0, "day_charge"),
        ),
        (Some(3), Some(4), Some(2), Some(24), Some(72), Some(Q / 2 + Q / 4 + Q / 5 + Q / 10)),
        "closed days 24, 72, 48, 48 count three distinct days and two uncovered days"
    );
    let logged = keys(&h99, TableId::PowerSpendDays);
    assert_eq!(logged.len(), 1, "hour 99 logs its own recurring day entry");
    assert_eq!(
        (
            cell::<SpendDay>(&h99, TableId::PowerSpendDays, logged[0], "ordinal"),
            cell::<SpendDay>(&h99, TableId::PowerSpendDays, logged[0], "day"),
            cell::<SpendDay>(&h99, TableId::PowerSpendDays, logged[0], "days"),
            cell::<SpendDay>(&h99, TableId::PowerSpendDays, logged[0], "payments"),
            cell::<SpendDay>(&h99, TableId::PowerSpendDays, logged[0], "charge"),
            cell::<SpendDay>(&h99, TableId::PowerSpendDays, logged[0], "uncovered"),
        ),
        (Some(0), Some(48), Some(0), Some(1), Some(Q / 10), Some(0)),
        "hour 99's day log row records native day 48 without incrementing distinct days or uncovered days"
    );
    assert_resumes_match(&[96, 97, 98], 99, scenario);
}

#[test]
fn reordered_open_batch_across_blocked_hour_refreshes_ordinals_and_saves_cleanly() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1]);
        let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            25 => {
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2 + Q / 10, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::MioResearch, Q / 10, 50), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 1, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            26 => {
                complete(store, TableId::CountryLinks);
                complete(store, TableId::CountryPowerDaily);
            }
            48 => {
                put_daily(store, hour, &daily(48, 3 * Q, Q / 2 + Q / 10, 80));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 1, PowerDailyCostKind::MioResearch, Q / 10, 80), TEXTS).unwrap();
            }
            49 => {
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 0, PowerDailyCostKind::MioResearch, Q / 10, 80), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(48, 1, PowerDailyCostKind::Relation, Q / 2, 80), TEXTS).unwrap();
            }
            50 => {
                complete(store, TableId::CountryLinks);
            }
            _ => {}
        }
    };
    let (h26, writers26) = run_custom(26, scenario);
    writers26.save().expect("reordered open batch after recovery must keep distinct first ordinals and save cleanly");
    let view26 = h26.view(TableId::PowerDayParts.index());
    let slot_a = view26.slot(part_key(10, POLITICAL, 0)).expect("item A part");
    let slot_b = view26.slot(part_key(10, POLITICAL, 1)).expect("item B part");
    assert_eq!(
        (
            cell::<Part>(&h26, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "spent"),
            cell::<Part>(&h26, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "cost_ordinal"),
            cell::<Part>(&h26, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "read"),
            view26.bool(column::<Part>("covered"), slot_a),
        ),
        (Some(Q / 2), Some(1), Some(24), Some(true)),
        "unchanged item A refreshes its first source ordinal while preserving proven spent, read, and coverage"
    );
    assert_eq!(
        (
            cell::<Part>(&h26, TableId::PowerDayParts, part_key(10, POLITICAL, 1), "spent"),
            cell::<Part>(&h26, TableId::PowerDayParts, part_key(10, POLITICAL, 1), "cost_ordinal"),
            view26.bool(column::<Part>("covered"), slot_b),
        ),
        (Some(Q / 10), Some(0), Some(true)),
        "inserted item B owns source ordinal 0"
    );
    let (h50, writers50) = run_custom(50, scenario);
    writers50.save().expect("reorder-only recovery must save cleanly");
    assert_eq!(
        (
            cell::<Part>(&h50, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "cost_ordinal"),
            cell::<Part>(&h50, TableId::PowerDayParts, part_key(10, POLITICAL, 1), "cost_ordinal"),
            cell::<Part>(&h50, TableId::PowerDayParts, part_key(10, POLITICAL, 0), "read"),
        ),
        (Some(1), Some(0), Some(48)),
        "reorder-only recovery refreshes both parts' source ordinals while preserving proven read hour"
    );
    assert_resumes_match(&[26, 50], 72, scenario);
}

#[test]
fn ordinary_zero_revision_with_late_witness_confirms_empty_and_blocks_stale_revisions() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1]);
        if hour >= 26 {
            let witness = Streams { country: 1, epoch: 1, since_hour: 26, through_hour: hour, streams: Streams::ALL_STREAMS, ..Default::default() };
            testing::put(store, TableId::PoliticsCaptureCoverage, &witness, TEXTS).unwrap();
        }
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            27 => {
                put_daily(store, hour, &daily(24, 3 * Q, 0, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, 0, 50), TEXTS).unwrap();
            }
            48 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h48, _) = run_custom(48, scenario);
    assert!(
        !h48.has_item_activity(10, POLITICAL, 0),
        "an ordinary explicit-zero revision with a fresh daily zero and current-hour cost witness confirms day 24 empty even when spending was not covered since midnight"
    );
    assert_resumes_match(&[27], 48, scenario);
}

#[test]
fn midnight_recovery_deletion_only_with_unlink_moves_survivor_and_withdraws_deleted() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 20)], &[1, 2]);
        let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2 + Q / 10, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 1, PowerDailyCostKind::MioResearch, Q / 10, 50), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2 + Q / 10, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 60) }, TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 1, PowerDailyCostKind::MioResearch, Q / 10, 60) }, TEXTS).unwrap();
            }
            47 => {
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
                testing::remove(store, TableId::CountryPowerDailyCosts, 2 << 32 | 1).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2, 60) });
            }
            48 => {
                complete(store, TableId::CountryLinks);
                testing::remove(store, TableId::CountryLinks, 1).unwrap();
                testing::remove(store, TableId::CountryLinks, 2).unwrap();
                testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
            }
            49 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 4, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 50), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h48, _) = run_custom(48, scenario);
    assert!(
        !h48.has_item_activity(10, POLITICAL, 0)
            && !h48.has_item_activity(10, POLITICAL, 1),
        "deletion-only midnight recovery with unlink settles nothing on the old holder"
    );
    assert!(
        !h48.has_item_activity(NONE, POLITICAL, 1),
        "tail item B is withdrawn rather than settled under NONE"
    );
    assert_eq!(
        (
            spend(&h48, NONE, POLITICAL, 0, "days"),
            spend(&h48, NONE, POLITICAL, 0, "day_payments"),
            spend(&h48, NONE, POLITICAL, 0, "day_charge"),
            spend(&h48, NONE, POLITICAL, 0, "day_unknown"),
            spend(&h48, NONE, POLITICAL, 0, "days_uncovered"),
        ),
        (Some(1), Some(1), Some(Q / 2), Some(0), Some(0)),
        "surviving item A moves to NONE and settles at its confirmed amount"
    );
    assert_eq!(
        (
            spend(&h48, 20, POLITICAL, 0, "day_unknown"),
            spend(&h48, 20, POLITICAL, 1, "day_unknown"),
        ),
        (Some(1), Some(1)),
        "a country unlinking on midnight recovery with no hour-48 publication keeps the unknown settlement on its prior holder"
    );
    let (h49, _) = run_custom(49, scenario);
    assert_eq!(
        (spend(&h49, NONE, POLITICAL, 0, "day_payments"), spend(&h49, NONE, POLITICAL, 0, "day_charge")),
        (Some(1), Some(Q / 2)),
        "a later stale revision does not settle day 24 again"
    );
    assert_resumes_match(&[24, 47, 48], 49, scenario);
}

#[test]
fn re_read_unchanged_daily_zero_confirms_empty_when_witness_or_recovery_arrives() {
    let scenario = |store: &mut Store, hour: i32| {
        let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        setup_base(store, hour, &[(1, 10), (2, 20), (3, 30)], &[2, 3]);
        if hour < 25 {
            let unwitnessed = Streams { country: 1, epoch: 1, since_hour: 24, through_hour: hour, streams: 0, ..Default::default() };
            testing::put(store, TableId::PoliticsCaptureCoverage, &unwitnessed, TEXTS).unwrap();
        } else {
            let witnessed = Streams { country: 1, epoch: 1, since_hour: 25, through_hour: hour, streams: Streams::ALL_STREAMS, ..Default::default() };
            testing::put(store, TableId::PoliticsCaptureCoverage, &witnessed, TEXTS).unwrap();
        }
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, 0, 50));
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 60) }, TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, ..daily(24, 3 * Q, 0, 70) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 70) }, TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(24, 1, PowerDailyCostKind::MioResearch, -Q / 4, 70) }, TEXTS).unwrap();
            }
            25 => {
                complete(store, TableId::CountryPowerDaily);
                complete(store, TableId::CountryPowerDailyCosts);
            }
            26 => {
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, 0, 60) }, TEXTS).unwrap();
            }
            27 => {
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, 0, 60) });
            }
            28 => {
                complete(store, TableId::CountryLinks);
                complete(store, TableId::CountryPowerDaily);
                complete(store, TableId::CountryPowerDailyCosts);
            }
            48 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 2, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 60) }, TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h25, _) = run_custom(25, scenario);
    assert_eq!(
        (
            cell::<Part>(&h25, TableId::PowerDayParts, part_key(30, POLITICAL, 0), "spent"),
            cell::<Part>(&h25, TableId::PowerDayParts, part_key(30, POLITICAL, 1), "spent"),
        ),
        (Some(Q / 4), Some(-Q / 4)),
        "re-reading unchanged offsetting nonzero costs must not confirm an empty day"
    );
    let (h28, _) = run_custom(28, scenario);
    assert_eq!(
        (cell::<Coverage>(&h28, TableId::PowerSpendCoverage, 20, "daily_mismatch"), cell::<Coverage>(&h28, TableId::PowerSpendCoverage, 20, "political")),
        (Some(0), Some(1)),
        "recovery with unchanged re-read daily zero and matching zero cost row clears the mismatch"
    );
    let (h48, _) = run_custom(48, scenario);
    assert!(
        !h48.has_item_activity(10, POLITICAL, 0),
        "an unchanged daily zero confirmed once its witness arrives must block stale post-midnight revisions"
    );
    assert!(
        !h48.has_item_activity(20, POLITICAL, 0),
        "a staggered daily zero landing during a blocked hour and re-read unchanged on recovery must block stale post-midnight revisions"
    );
    assert_eq!(
        (spend(&h48, 30, POLITICAL, 0, "day_charge"), spend(&h48, 30, POLITICAL, 1, "credits")),
        (Some(Q / 4), Some(1)),
        "offsetting nonzero costs still settle at midnight"
    );
    assert_resumes_match(&[24, 25, 26, 27, 28], 48, scenario);
}

#[test]
fn staggered_zero_confirmation_clears_mismatch_and_blocks_stale_revisions() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1]);
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            26 => {
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, 0, 50), TEXTS).unwrap();
            }
            27 => {
                put_daily(store, hour, &daily(24, 3 * Q, 0, 50));
            }
            48 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h26, _) = run_custom(26, scenario);
    assert!(keys(&h26, TableId::PowerDayParts).is_empty(), "zeroing the last cost row removes the open batch");
    assert_eq!(cell::<Coverage>(&h26, TableId::PowerSpendCoverage, 10, "daily_mismatch"), Some(i64::from(Coverage::STEADY_MISMATCH)));
    let (h27, _) = run_custom(27, scenario);
    assert_eq!(
        (cell::<Coverage>(&h27, TableId::PowerSpendCoverage, 10, "daily_mismatch"), cell::<Coverage>(&h27, TableId::PowerSpendCoverage, 10, "political")),
        (Some(0), Some(1)),
        "a fresh daily zero matching the stored zero-valued cost row clears the mismatch"
    );
    let (h48, _) = run_custom(48, scenario);
    assert!(
        !h48.has_item_activity(10, POLITICAL, 0),
        "a stale positive revision after midnight must not settle a day confirmed empty via a staggered zero update"
    );
    assert!(keys(&h48, TableId::PowerSpendDays).is_empty());
    assert_resumes_match(&[24, 26, 27], 48, scenario);
}

#[test]
fn daily_costs_unavailable_keeps_payments_and_gains_complete_and_logs_instant_gains() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1]);
        let held = testing::rows(store, TableId::CountryPowerChanges);
        for (key, _) in held {
            testing::remove(store, TableId::CountryPowerChanges, key).unwrap();
        }
        if hour == 5 {
            store.status(TableId::CountryPowerDailyCosts.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap();
            testing::put(store, TableId::CountryPowerChanges, &idea(1, 10, 5, 7, Q), TEXTS).unwrap();
            testing::put(
                store,
                TableId::CountryPowerChanges,
                &CountryPowerChangeRow {
                    country: 1,
                    sequence: 11,
                    epoch: 1,
                    hour: 5,
                    currency: PowerCurrency::Political,
                    operation: PowerOperation::Add,
                    balance_before_raw: 1995 * Q,
                    realized_delta_raw: Recorded::Value(5 * Q),
                    requested_delta_raw: Some(20 * Q),
                    cause: PowerCause::Focus,
                    site: PowerSite::AddPoliticalPower,
                    ..Default::default()
                },
                TEXTS,
            )
            .unwrap();
        } else if hour == 6 {
            store.status(TableId::CountryPowerDailyCosts.index(), Status::Complete, true, Some(hour), true).unwrap();
        }
    };
    let (h5, _) = run_custom(5, scenario);
    assert_eq!(h5.view(TableId::PowerSpendDays.index()).status(), Status::Unavailable { reason: 1 });
    assert_eq!(h5.view(TableId::PowerDayParts.index()).status(), Status::Unavailable { reason: 1 });
    assert_eq!(h5.view(TableId::PowerSpendPayments.index()).status(), Status::Complete, "CountryPowerDailyCosts unavailability must not drop one-off payments");
    assert_eq!(h5.view(TableId::PowerSpendGains.index()).status(), Status::Complete, "CountryPowerDailyCosts unavailability must not drop gains");
    assert_eq!(spend(&h5, 10, POLITICAL, 0, "payments"), Some(1));
    assert_eq!(spend(&h5, 10, POLITICAL, 0, "known"), Some(Q));
    assert_eq!(spend(&h5, 10, POLITICAL, Total::GAINS, "gained"), Some(20 * Q));
    assert_eq!(spend(&h5, 10, POLITICAL, Total::GAINS, "capped"), Some(15 * Q));
    let (h6, _) = run_custom(6, scenario);
    assert!(keys(&h6, TableId::PowerSpendPayments).is_empty(), "log rows live only their read hour");
    assert!(keys(&h6, TableId::PowerSpendGains).is_empty(), "gain log rows live only their read hour");
    assert_resumes_match(&[5], 6, scenario);
}

#[test]
fn relation_and_blockade_daily_costs_settle_into_day_totals_and_day_log() {
    let relation = |day: i32, amount: i64, seq: u64| CountryPowerDailyCostRow {
        cause: PowerCause::Diplomacy,
        ..cost(day, 0, PowerDailyCostKind::Relation, amount, seq)
    };
    let blockade = |day: i32, amount: i64, seq: u64| CountryPowerDailyCostRow {
        cause: PowerCause::Blockade,
        identity: PowerIdentity::default(),
        ..cost(day, 1, PowerDailyCostKind::Blockade, amount, seq)
    };
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1]);
        if hour == 24 {
            put_daily(store, hour, &daily(24, 3 * Q, Q / 2 + Q / 4, 50));
            testing::put(store, TableId::CountryPowerDailyCosts, &relation(24, Q / 2, 50), TEXTS).unwrap();
            testing::put(store, TableId::CountryPowerDailyCosts, &blockade(24, Q / 4, 50), TEXTS).unwrap();
        }
    };
    let (h48, _) = run_custom(48, scenario);
    for (ordinal, expected) in [(0_u32, Q / 2), (1_u32, Q / 4)] {
        assert_eq!(
            (
                spend(&h48, 10, POLITICAL, ordinal, "payments"),
                spend(&h48, 10, POLITICAL, ordinal, "days"),
                spend(&h48, 10, POLITICAL, ordinal, "day_payments"),
                spend(&h48, 10, POLITICAL, ordinal, "day_charge"),
            ),
            (Some(0), Some(1), Some(1), Some(expected)),
            "daily cost ordinal {ordinal} with collector cause settles as recurring, not one-off"
        );
    }
    let logged = keys(&h48, TableId::PowerSpendDays);
    assert_eq!(logged.len(), 2, "both Diplomacy relation and Blockade daily costs emit PowerSpendDays rows");
}

#[test]
fn shared_holder_mismatch_stays_isolated_and_overflowing_sum_is_uncovered() {
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10), (2, 10), (3, 20), (4, 20)], &[1, 2, 3, 4]);
        match hour {
            24 => {
                put_daily(store, hour, &daily(24, 3 * Q, Q / 2 + Q / 10, 50));
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 0, PowerDailyCostKind::Relation, Q / 2, 50), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerDailyCosts, &cost(24, 1, PowerDailyCostKind::MioResearch, Q / 10, 50), TEXTS).unwrap();
                let big = i64::MAX - 10;
                put_daily(store, hour, &CountryPowerDailyRow { country: 3, pp_gross_gain_raw: Some(big), pp_cost_raw: Some(big), pp_net_gain_raw: Some(0), pp_realized_delta_raw: Some(0), ..daily(24, 0, 0, 70) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 3, ..cost(24, 0, PowerDailyCostKind::Relation, big, 70) }, TEXTS).unwrap();
                put_daily(store, hour, &CountryPowerDailyRow { country: 4, ..daily(24, 3 * Q, 20, 71) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 4, ..cost(24, 0, PowerDailyCostKind::Relation, 20, 71) }, TEXTS).unwrap();
            }
            25 => {
                testing::remove(store, TableId::CountryPowerDaily, 1).unwrap();
                testing::remove(store, TableId::CountryPowerDailyCosts, 1 << 32 | 1).unwrap();
            }
            26 => {
                put_daily(store, hour, &CountryPowerDailyRow { country: 2, ..daily(24, 3 * Q, Q / 4, 60) });
                testing::put(store, TableId::CountryPowerDailyCosts, &CountryPowerDailyCostRow { country: 2, ..cost(24, 0, PowerDailyCostKind::Relation, Q / 4, 60) }, TEXTS).unwrap();
            }
            27 => {
                testing::put(store, TableId::Countries, &CountryRow { id: 1, canonical_slot: 30, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 30 }, TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h24, _) = run_custom(24, scenario);
    let view24 = h24.view(TableId::PowerDayParts.index());
    let slot20 = view24.slot(part_key(20, POLITICAL, 0)).expect("a row");
    assert_eq!(cell::<Part>(&h24, TableId::PowerDayParts, part_key(20, POLITICAL, 0), "spent"), None, "overflowing shared sum is not recorded");
    assert_eq!(view24.bool(column::<Part>("covered"), slot20), Some(false), "and an overflowing shared sum is never covered");
    let (h26, _) = run_custom(26, scenario);
    assert_eq!(cell::<Coverage>(&h26, TableId::PowerSpendCoverage, 10, "daily_mismatch"), Some(i64::from(Coverage::STEADY_MISMATCH)), "country 2 publishing a balanced batch does not clear country 1's mismatch");
    let (h27, _) = run_custom(27, scenario);
    assert_eq!(cell::<Coverage>(&h27, TableId::PowerSpendCoverage, 10, "daily_mismatch"), Some(0), "once mismatching country 1 leaves holder 10, holder 10 clears its mismatch");
    let view27 = h27.view(TableId::PowerDayParts.index());
    assert_eq!(view27.bool(column::<Part>("covered"), view27.slot(part_key(10, POLITICAL, 0)).expect("a row")), Some(true), "and country 2's own balanced part on holder 10 kept its coverage");
    assert_resumes_match(&[24, 25, 26], 27, scenario);
}

#[test]
fn republished_changes_are_deduped_large_gains_split_daily_gains_survive_change_outage_and_idle_stays_unavailable() {
    let half = 1_i64 << 62;
    let grant = |seq: u64, amount: i64| CountryPowerChangeRow {
        country: 1,
        epoch: 7,
        sequence: seq,
        hour: 3,
        currency: PowerCurrency::Political,
        operation: PowerOperation::Add,
        balance_before_raw: 0,
        realized_delta_raw: Recorded::Value(amount),
        requested_delta_raw: Some(amount),
        cause: PowerCause::Focus,
        site: PowerSite::AddPoliticalPower,
        ..Default::default()
    };
    let charge_row = |seq: u64, amount: i64| CountryPowerChangeRow {
        country: 1,
        epoch: 7,
        sequence: seq,
        hour: 2,
        currency: PowerCurrency::Political,
        operation: PowerOperation::Subtract,
        balance_before_raw: 100 * Q,
        realized_delta_raw: Recorded::Value(-amount),
        requested_delta_raw: Some(-amount),
        cause: PowerCause::Decision,
        site: PowerSite::AddPoliticalPower,
        ..Default::default()
    };
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1]);
        match hour {
            2 => {
                testing::put(store, TableId::CountryPowerChanges, &grant(10, 5 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &charge_row(11, 25 * Q), TEXTS).unwrap();
            }
            3 => {
                testing::remove(store, TableId::CountryPowerChanges, (1_u128 << 64) | 10).unwrap();
                testing::remove(store, TableId::CountryPowerChanges, (1_u128 << 64) | 11).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &grant(20, half), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &grant(21, half), TEXTS).unwrap();
            }
            4 => {
                testing::put(store, TableId::CountryPowerChanges, &grant(10, 5 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &charge_row(11, 25 * Q), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h3, _) = run_custom(3, scenario);
    let gain_keys = keys(&h3, TableId::PowerSpendGains);
    assert_eq!(gain_keys.len(), 2, "two 2^62 gains in one bucket split across two PowerSpendGains entries instead of overflowing to NotRecorded");
    let total_gained: i128 = gain_keys
        .iter()
        .map(|&key| i128::from(cell::<SpendGain>(&h3, TableId::PowerSpendGains, key, "gained").expect("each split entry is exact")))
        .sum();
    assert_eq!(total_gained, i128::from(half) * 2);
    assert_eq!(h3.gains[&(10, POLITICAL)].gained, i128::from(5 * Q) + i128::from(half) * 2);
    let (h4, _) = run_custom(4, scenario);
    assert!(keys(&h4, TableId::PowerSpendGains).is_empty(), "republishing an instant gain with the same country, sequence and epoch must not count it twice");
    assert!(keys(&h4, TableId::PowerSpendPayments).is_empty(), "republishing a charge with the same country, sequence and epoch must not count it twice");
    let charge_ordinal = keys(&h4, TableId::PowerItems)
        .into_iter()
        .find(|&key| (key >> 32) as u32 == 10 && cell::<Item>(&h4, TableId::PowerItems, key, "cause") == Some(i64::from(PowerCause::Decision as u16)))
        .expect("decision item recorded") as u32;
    assert_eq!(spend(&h4, 10, POLITICAL, charge_ordinal, "payments"), Some(1));
    assert_eq!(spend(&h4, 10, POLITICAL, charge_ordinal, "known"), Some(25 * Q));
    assert_resumes_match(&[2, 3], 4, scenario);

    let daily_during_outage = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1]);
        if hour == 24 {
            put_daily(store, hour, &daily(24, 3 * Q, Q, 50));
        } else if hour == 48 {
            store.status(TableId::CountryPowerChanges.index(), Status::Unavailable { reason: 1 }, false, None, false).unwrap();
            put_daily(store, hour, &daily(48, 4 * Q, Q, 60));
        }
    };
    let (d48, _) = run_custom(48, daily_during_outage);
    assert_eq!(d48.view(TableId::PowerSpendPayments.index()).status(), Status::Unavailable { reason: 1 });
    assert_eq!(d48.view(TableId::PowerSpendGains.index()).status(), Status::Complete, "a closed daily gain keeps PowerSpendGains readable despite CountryPowerChanges outage");
    assert_eq!(spend(&d48, 10, POLITICAL, Total::GAINS, "gained"), Some(3 * Q));

    let idle_unavailable = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[], &[]);
        if hour >= 2 {
            store.status(TableId::CountryPowerChanges.index(), Status::Unavailable { reason: 1 }, false, None, false).unwrap();
        }
    };
    let (u1, _) = run_custom(1, idle_unavailable);
    for derived in [TableId::PowerDayParts, TableId::PowerSpendDays, TableId::PowerSpendPayments, TableId::PowerSpendGains] {
        assert_eq!(u1.view(derived.index()).status(), Status::Complete, "writer runs cleanly and becomes idle at hour 1");
    }
    let (u3, _) = run_custom(3, idle_unavailable);
    for derived in [TableId::PowerDayParts, TableId::PowerSpendDays, TableId::PowerSpendPayments, TableId::PowerSpendGains] {
        assert_eq!(
            u3.view(derived.index()).status(),
            Status::Unavailable { reason: 1 },
            "second consecutive idle unavailable hour must keep {derived:?} unavailable"
        );
    }

    let startup_outage = |store: &mut Store, hour: i32| {
        let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        match hour {
            0 => {
                for table in [TableId::Countries, TableId::CountryPowerDaily, TableId::CountryPowerDailyCosts, TableId::CountryCommandPower, TableId::CountryPowerChanges, TableId::PoliticsCaptureCoverage] {
                    complete(store, table);
                }
                testing::put(store, TableId::Countries, &CountryRow { id: 1, canonical_slot: 10, ..Default::default() }, TEXTS).unwrap();
                store.status(TableId::CountryLinks.index(), Status::Unavailable { reason: 1 }, false, None, false).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &grant(10, 5 * Q), TEXTS).unwrap();
            }
            1 => {
                complete(store, TableId::CountryLinks);
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 10 }, TEXTS).unwrap();
                testing::remove(store, TableId::CountryPowerChanges, (1_u128 << 64) | 10).unwrap();
            }
            2 => {
                testing::put(store, TableId::CountryPowerChanges, &grant(10, 5 * Q), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (s0, _) = run_custom(0, startup_outage);
    assert_eq!(s0.view(TableId::PowerSpendGains.index()).status(), Status::Unavailable { reason: 1 });
    let (s2, _) = run_custom(2, startup_outage);
    assert!(keys(&s2, TableId::PowerSpendGains).is_empty(), "a pre-attachment backlog grant captured while CountryLinks was unavailable on hour 0 must not be counted if republished after recovery");
    assert_eq!(spend(&s2, 10, POLITICAL, Total::GAINS, "gained"), None);
    assert_resumes_match(&[0, 1], 2, startup_outage);

    let later_outage = |store: &mut Store, hour: i32| {
        let complete = |store: &mut Store, table: TableId| store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        match hour {
            0 => {
                setup_base(store, hour, &[(1, 10)], &[1]);
            }
            1 => {
                store.status(TableId::Countries.index(), Status::Unavailable { reason: 1 }, false, None, false).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &grant(10, 5 * Q), TEXTS).unwrap();
            }
            2 => {
                complete(store, TableId::Countries);
                testing::put(store, TableId::Countries, &CountryRow { id: 1, canonical_slot: 20, ..Default::default() }, TEXTS).unwrap();
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 20 }, TEXTS).unwrap();
                testing::remove(store, TableId::CountryPowerChanges, (1_u128 << 64) | 10).unwrap();
            }
            3 => {
                testing::put(store, TableId::CountryPowerChanges, &grant(10, 5 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &grant(11, 7 * Q), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (l1, _) = run_custom(1, later_outage);
    assert_eq!(l1.view(TableId::PowerSpendGains.index()).status(), Status::Unavailable { reason: 1 });
    let (l3, _) = run_custom(3, later_outage);
    let l3_gains = keys(&l3, TableId::PowerSpendGains);
    assert_eq!(l3_gains.len(), 1, "only the fresh grant after recovery should be emitted for holder 20");
    assert_eq!(cell::<SpendGain>(&l3, TableId::PowerSpendGains, l3_gains[0], "gained"), Some(7 * Q));
    assert_eq!(spend(&l3, 10, POLITICAL, Total::GAINS, "gained"), None);
    assert_eq!(spend(&l3, 20, POLITICAL, Total::GAINS, "gained"), Some(7 * Q));
    assert_resumes_match(&[0, 1, 2], 3, later_outage);
}

#[test]
fn seen_changes_high_water_mark_bounds_saved_state_size_and_round_trips() {
    let mut ledger = PowerLedger::default();
    assert!(ledger.see_change(1, 1, 5));
    let mut small = Bytes::new();
    ledger.save(&mut small).unwrap();
    for step in 2..=200_u64 {
        assert!(ledger.see_change(1, 1, step * 5));
    }
    assert_eq!(ledger.seen_changes.len(), 1, "many spaced sequences for one country keep a single mark");
    assert_eq!(ledger.seen_changes.get(&1), Some(&(1, 1000)));
    let mut after_many = Bytes::new();
    ledger.save(&mut after_many).unwrap();
    assert!(
        after_many.bytes().len() <= small.bytes().len() + 4,
        "saved state size must stay bounded with event count: small={} after_many={}",
        small.bytes().len(),
        after_many.bytes().len()
    );
    let mut loaded = PowerLedger::default();
    loaded.load(after_many.bytes()).unwrap();
    assert_eq!(loaded.seen_changes, ledger.seen_changes);

    let mut dup = Bytes::new();
    for _ in 0..8 {
        dup.varint(0).unwrap();
    }
    dup.varint(2).unwrap();
    for _ in 0..2 {
        dup.varint(1).unwrap();
        dup.varint(1).unwrap();
        dup.varint(5).unwrap();
    }
    assert!(loaded.load(dup.bytes()).is_err(), "duplicate country mark in saved state must fail load");
}

#[test]
fn seen_changes_ordering_rejects_at_or_below_mark_and_accepts_newer_epoch_lower_sequence() {
    let grant = |epoch: u64, sequence: u64, amount: i64| CountryPowerChangeRow {
        country: 1,
        sequence,
        epoch,
        hour: 1,
        currency: PowerCurrency::Political,
        operation: PowerOperation::Add,
        balance_before_raw: 0,
        realized_delta_raw: Recorded::Value(amount),
        requested_delta_raw: Some(amount),
        cause: PowerCause::Focus,
        site: PowerSite::AddPoliticalPower,
        ..Default::default()
    };
    let decision = |epoch: u64, sequence: u64, cost_raw: i64| CountryPowerChangeRow {
        country: 1,
        sequence,
        epoch,
        hour: 1,
        currency: PowerCurrency::Political,
        operation: PowerOperation::Subtract,
        balance_before_raw: cost_raw,
        realized_delta_raw: Recorded::Value(-cost_raw),
        requested_delta_raw: Some(-cost_raw),
        cause: PowerCause::Decision,
        site: PowerSite::AddPoliticalPower,
        identity: PowerIdentity { item_token: Some(7), ..Default::default() },
        ..Default::default()
    };
    let scenario = |store: &mut Store, hour: i32| {
        setup_base(store, hour, &[(1, 10)], &[1]);
        match hour {
            1 => {
                testing::put(store, TableId::CountryPowerChanges, &grant(2, 50, 3 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &decision(2, 55, 10 * Q), TEXTS).unwrap();
            }
            2 => {
                testing::remove(store, TableId::CountryPowerChanges, (1_u128 << 64) | 50).unwrap();
                testing::remove(store, TableId::CountryPowerChanges, (1_u128 << 64) | 55).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &grant(2, 55, 9 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &grant(2, 20, 8 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &decision(2, 15, 7 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &grant(1, 999, 6 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &decision(1, 998, 5 * Q), TEXTS).unwrap();
            }
            3 => {
                for seq in [15_u64, 20, 55, 998, 999] {
                    testing::remove(store, TableId::CountryPowerChanges, (1_u128 << 64) | u128::from(seq)).unwrap();
                }
                testing::put(store, TableId::CountryPowerChanges, &grant(3, 5, 4 * Q), TEXTS).unwrap();
                testing::put(store, TableId::CountryPowerChanges, &decision(3, 10, 15 * Q), TEXTS).unwrap();
            }
            _ => {}
        }
    };
    let (h1, _) = run_custom(1, scenario);
    let decision_ordinal = keys(&h1, TableId::PowerItems)
        .into_iter()
        .filter(|&key| (key >> 32) as u32 == 10 && cell::<Item>(&h1, TableId::PowerItems, key, "cause") == Some(PowerCause::Decision as i64))
        .map(|key| key as u32)
        .next()
        .expect("decision item");
    assert_eq!(spend(&h1, 10, POLITICAL, Total::GAINS, "gained"), Some(3 * Q));
    assert_eq!(spend(&h1, 10, POLITICAL, decision_ordinal, "payments"), Some(1));
    assert_eq!(spend(&h1, 10, POLITICAL, decision_ordinal, "known"), Some(10 * Q));

    let (h2, _) = run_custom(2, scenario);
    assert!(keys(&h2, TableId::PowerSpendGains).is_empty(), "same-epoch equal/lower sequence and older-epoch rows must not emit gains");
    assert!(keys(&h2, TableId::PowerSpendPayments).is_empty(), "same-epoch equal/lower sequence and older-epoch rows must not emit payments");
    assert_eq!(spend(&h2, 10, POLITICAL, Total::GAINS, "gained"), Some(3 * Q));
    assert_eq!(spend(&h2, 10, POLITICAL, decision_ordinal, "payments"), Some(1));
    assert_eq!(spend(&h2, 10, POLITICAL, decision_ordinal, "known"), Some(10 * Q));

    let (h3, _) = run_custom(3, scenario);
    assert_eq!(spend(&h3, 10, POLITICAL, Total::GAINS, "gained"), Some(7 * Q), "a newer epoch with a lower sequence must be counted");
    assert_eq!(spend(&h3, 10, POLITICAL, decision_ordinal, "payments"), Some(2));
    assert_eq!(spend(&h3, 10, POLITICAL, decision_ordinal, "known"), Some(25 * Q));
    assert_resumes_match(&[1, 2], 3, scenario);
}
