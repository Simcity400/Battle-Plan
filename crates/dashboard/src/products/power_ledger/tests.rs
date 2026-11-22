use super::{PowerSpendingParams, SpendCoverage, SpendCurrency, naming, spending};
use crate::{
    Source,
    test_fixtures::{CAMPAIGN, records, source},
};
use dashboard_core::config::LoadContext;
use replay::campaign::Replay;
use replay_data::rows::{
    CountryLinkRow, CountryPowerChangeRow, CountryPowerDailyCostRow, CountryPowerDailyRow, CountryRow, PoliticsCaptureCoverageRow, PowerCause, PowerCurrency, PowerDailyCostKind, PowerItemRow,
    PowerOperation, PowerSite, PowerSpendCoverageRow, Recorded, TableId, TableRows,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

const FIRST: i32 = 100;
const LAST: i32 = 170;
const GER: u32 = 9001;
const SLOT: u32 = 7;
const PP: i64 = 100_000;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    frame.tables[table.index()] = TableView::Complete(rows).into();
}

fn decision(sequence: u64, hour: i32, charge: i64) -> CountryPowerChangeRow {
    CountryPowerChangeRow {
        country: GER,
        sequence,
        epoch: 1,
        hour,
        currency: PowerCurrency::Political,
        operation: PowerOperation::Subtract,
        balance_before_raw: 200 * PP,
        balance_after_raw: 200 * PP - charge,
        requested_delta_raw: Some(-charge),
        cause: PowerCause::Decision,
        site: PowerSite::AddPoliticalPower,
        ..Default::default()
    }
}

fn daily(day_hour: i32, gross: i64, cost: i64, realized: i64, sequence: u64) -> CountryPowerDailyRow {
    CountryPowerDailyRow {
        country: GER,
        epoch: 1,
        day_hour,
        pp_gross_gain_raw: Some(gross),
        pp_cost_raw: Some(cost),
        pp_net_gain_raw: Some(gross - cost),
        pp_realized_delta_raw: Some(realized),
        pp_sequence: Some(sequence),
        ..Default::default()
    }
}

fn cost(day_hour: i32, amount: i64, sequence: u64) -> CountryPowerDailyCostRow {
    CountryPowerDailyCostRow {
        country: GER,
        epoch: 1,
        day_hour,
        pp_sequence: sequence,
        component: PowerDailyCostKind::PoliticalPowerCostModifier,
        cost_raw: amount,
        cause: PowerCause::DailyGain,
        site: PowerSite::AddPoliticalPower,
        ..Default::default()
    }
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1_000));
    set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: SLOT, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
    set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: SLOT }]));
    let changes = match hour {
        104 => vec![decision(7, 103, 50 * PP)],
        120 => vec![decision(9, 120, 75 * PP)],
        _ => Vec::new(),
    };
    set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(changes));
    let day = match hour {
        ..=119 => None,
        120..=143 => Some((daily(120, 3 * PP, 2 * PP, PP, 50), cost(120, 2 * PP, 50))),
        144..=167 => Some((daily(144, 5 * PP, 25 * PP / 10, 15 * PP / 10, 80), cost(144, 25 * PP / 10, 80))),
        _ => Some((daily(168, 3 * PP, 2 * PP, PP, 110), cost(168, 2 * PP, 110))),
    };
    let (days, costs) = day.map_or((Vec::new(), Vec::new()), |(day, cost)| (vec![day], vec![cost]));
    set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
    set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
    let capture = PoliticsCaptureCoverageRow { country: GER, epoch: 1, since_hour: FIRST, through_hour: hour, streams: PoliticsCaptureCoverageRow::ALL_STREAMS, ..Default::default() };
    set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(vec![capture]));
    frame
}

fn recording() -> Source {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(crate::SourceKey { source_id: "power-ledger".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn asked(source: &Source, from: i32, to: i32) -> super::PowerSpendingReply {
    let query = crate::answer::query(source, 1).unwrap();
    let naming = naming::context(source, &LoadContext { content: None }).unwrap();
    let params = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from, to };
    spending(source, &query, &params, &naming, &AtomicBool::new(false)).unwrap()
}

#[test]
fn a_spans_payments_are_the_changes_of_its_totals_after_the_hour_before_it_and_recurring_days_and_gains_are_listed_as_counted() {
    let source = recording();
    let late = asked(&source, 105, LAST);
    assert_eq!(late.payments.len(), 1, "the decision paid at 104 is before the span: its totals there are the baseline");
    let payment = &late.payments[0];
    assert_eq!((payment.read, payment.hour, payment.payments, payment.known_raw.as_deref()), (120, Some(120), 1, Some("7500000")));
    assert_eq!(payment.sequence.as_deref(), Some("9"));
    let decision = &late.items[payment.item as usize];
    assert_eq!(decision.id, "7:0:104", "an item is its holder, ordinal and first read");
    assert_eq!(decision.category, "decisions");
    assert!(!decision.recurring);
    assert!(decision.unknown.is_some(), "no content: its name is not known, and says why");
    let days: Vec<_> = late.days.iter().map(|day| (day.read, day.days, day.charge_raw.clone(), day.day)).collect();
    assert_eq!(days, [(144, 1, Some("200000".into()), Some(120)), (168, 1, Some("250000".into()), Some(144))], "a day's costs are counted as the day ends");
    let cost = &late.items[late.days[0].item as usize];
    assert!(cost.recurring);
    assert_eq!(cost.category, "dailyCosts");
    let gains: Vec<_> = late.gains.iter().map(|gain| (gain.read, gain.gained_raw.clone(), gain.capped_raw.clone(), gain.days)).collect();
    assert_eq!(gains, [(144, Some("300000".into()), Some("0".into()), 1), (168, Some("500000".into()), Some("100000".into()), 1)]);
    assert_eq!(late.coverage, [SpendCoverage { first_hour: 105, last_hour: LAST, covered: true, reason: None }]);

    let whole = asked(&source, FIRST, LAST);
    let reads: Vec<_> = whole.payments.iter().map(|payment| (payment.read, payment.known_raw.clone())).collect();
    assert_eq!(reads, [(104, Some("5000000".into())), (120, Some("7500000".into()))], "the run's totals start from nothing");
    assert_eq!(whole.coverage.len(), 2);
    assert_eq!((whole.coverage[0].first_hour, whole.coverage[0].last_hour, whole.coverage[0].covered), (FIRST, FIRST, false), "the run's first hour lists what the game held: not counted");
    assert_eq!(whole.coverage[0].reason.as_deref(), Some(PowerSpendCoverageRow::REASONS[usize::from(PowerSpendCoverageRow::BACKLOG)]));
    assert_eq!((whole.coverage[1].first_hour, whole.coverage[1].last_hour, whole.coverage[1].covered), (FIRST + 1, LAST, true));
}

#[test]
fn a_tag_no_holder_held_lists_nothing_and_no_hour_as_covered() {
    let source = recording();
    let held = asked(&source, FIRST, LAST);
    assert!(!held.payments.is_empty() && !held.coverage.is_empty(), "the recording holds another tag's spending");
    let query = crate::answer::query(&source, 1).unwrap();
    let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
    let params = PowerSpendingParams { branch_id: "1".into(), country: "ENG".into(), currency: SpendCurrency::Political, from: FIRST, to: LAST };
    let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert!(reply.payments.is_empty() && reply.days.is_empty() && reply.items.is_empty());
    assert!(reply.coverage.is_empty(), "hours no holding names are not recorded, never covered");
}

#[test]
fn an_empty_lexical_literal_names_no_item_token() {
    use replay_data::rows::LexicalTokenRow;
    let mut frame = source(1, FIRST, None);
    set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(vec![LexicalTokenRow { token: 5, literal: Vec::new() }, LexicalTokenRow { token: 6, literal: b"GER_idea".to_vec() }]));
    let row = |token| PowerItemRow { holder: SLOT, cause: PowerCause::Idea, item_token: Some(token), ..Default::default() };
    assert_eq!(naming::token_of(Some(&frame), &row(5)), None);
    assert_eq!(naming::token_of(Some(&frame), &row(6)).as_deref(), Some("GER_idea"));
}

#[test]
fn out_of_order_closed_days_keep_their_native_date_in_spending_replies() {
    let make_frame = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: SLOT, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: SLOT }]));
        set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(Vec::new()));
        let published = match hour {
            193 => Some((daily(144, 3 * PP, 2 * PP, PP, 80), cost(144, 2 * PP, 80))),
            194 => Some((daily(120, 3 * PP, PP, 2 * PP, 50), cost(120, PP, 50))),
            195 => Some((daily(120, 3 * PP, 3 * PP, 0, 50), cost(120, 3 * PP, 50))),
            196 => Some((daily(168, 3 * PP, 4 * PP, 0, 110), cost(168, 4 * PP, 110))),
            197 => Some((daily(168, 3 * PP, 4 * PP, 0, 110), cost(168, 4 * PP, 110))),
            _ => None,
        };
        let (days, costs) = published.map_or((Vec::new(), Vec::new()), |(day, cost)| (vec![day], vec![cost]));
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
        let capture = PoliticsCaptureCoverageRow { country: GER, epoch: 1, since_hour: FIRST, through_hour: hour, streams: PoliticsCaptureCoverageRow::ALL_STREAMS, ..Default::default() };
        set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(vec![capture]));
        frame
    };
    let frames = (FIRST..=197).map(make_frame).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(crate::SourceKey { source_id: "power-ledger-ooo".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = asked(&source, FIRST, 197);
    let listed: Vec<_> = reply.days.iter().map(|day| (day.read, day.days, day.payments, day.charge_raw.clone(), day.day)).collect();
    assert_eq!(
        listed,
        [
            (193, 1, 1, Some("200000".into()), Some(144)),
            (194, 1, 1, Some("100000".into()), Some(120)),
            (196, 1, 1, Some("400000".into()), Some(168)),
        ],
        "out-of-order closed days report their true native date and never settle duplicates"
    );

    let middle_frame = |hour: i32| {
        let mut frame = source_frame(hour);
        let published = match hour {
            193 => Some((daily(120, 3 * PP, PP, 2 * PP, 50), cost(120, PP, 50))),
            194 => Some((daily(168, 3 * PP, 4 * PP, 0, 110), cost(168, 4 * PP, 110))),
            195 => Some((daily(144, 3 * PP, 2 * PP, PP, 80), cost(144, 2 * PP, 80))),
            _ => None,
        };
        let (days, costs) = published.map_or((Vec::new(), Vec::new()), |(day, cost)| (vec![day], vec![cost]));
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
        frame
    };
    let frames = (FIRST..=195).map(middle_frame).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(crate::SourceKey { source_id: "power-ledger-mid".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = asked(&source, FIRST, 195);
    let listed: Vec<_> = reply.days.iter().map(|day| (day.read, day.days, day.payments, day.charge_raw.clone(), day.day)).collect();
    assert_eq!(
        listed,
        [
            (193, 1, 1, Some("100000".into()), Some(120)),
            (194, 1, 1, Some("400000".into()), Some(168)),
            (195, 1, 1, Some("200000".into()), Some(144)),
        ],
        "a closed day landing between earlier and later closed days counts as its own day and keeps its own native date"
    );

    let mut legacy = [super::Value::Number(0); super::TOTALS.len()];
    legacy[super::position("days")] = super::Value::Number(1);
    legacy[super::position("day_payments")] = super::Value::Number(1);
    legacy[super::position("day_charge")] = super::Value::Number(PP);
    legacy[super::position("last_day")] = super::Value::Number(144);
    legacy[super::position("counted_day")] = super::Value::Unrecorded;
    let (_, fallback) = super::listed(&super::Change { ordinal: 0, hour: 168, before: None, after: &legacy }, 0);
    assert_eq!(fallback.expect("a day").day, Some(144), "recordings predating counted_day fall back to last_day");
}

fn source_frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1_000));
    set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: SLOT, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
    set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: SLOT }]));
    set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(Vec::new()));
    let capture = PoliticsCaptureCoverageRow { country: GER, epoch: 1, since_hour: FIRST, through_hour: hour, streams: PoliticsCaptureCoverageRow::ALL_STREAMS, ..Default::default() };
    set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(vec![capture]));
    frame
}
