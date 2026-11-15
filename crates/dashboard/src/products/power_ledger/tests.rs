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

/// GER's decision of power sequence `sequence`, dated `hour`, charging `charge`.
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

/// GER's daily row of the day from `day_hour`: its gain, its cost, and what the balance took.
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

/// The day from `day_hour`'s one cost: the political power cost modifier.
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

/// What the collector reads of GER (slot 7), from which the recorder's PowerLedger writer derives
/// the ledger: a decision paid at hour 103 (read 104) and again at 120; the days from 120 and 144
/// with one daily cost each, which end (and are counted) at 144 and 168; a politics capture of
/// every stream since the run's first hour. The run's first hour is a backlog: not covered.
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
    let params = PowerSpendingParams { branch_id: "1".into(), tag: "GER".into(), currency: SpendCurrency::Political, from, to };
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
    let params = PowerSpendingParams { branch_id: "1".into(), tag: "ENG".into(), currency: SpendCurrency::Political, from: FIRST, to: LAST };
    let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert!(reply.payments.is_empty() && reply.days.is_empty() && reply.items.is_empty());
    assert!(reply.coverage.is_empty(), "hours no holding names are not recorded, never covered");
}

/// An item whose key is empty and whose lexical token's literal is empty names no token: an idea
/// tooltip keyed by an empty token is refused whole by the widget.
#[test]
fn an_empty_lexical_literal_names_no_item_token() {
    use replay_data::rows::LexicalTokenRow;
    let mut frame = source(1, FIRST, None);
    set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(vec![LexicalTokenRow { token: 5, literal: Vec::new() }, LexicalTokenRow { token: 6, literal: b"GER_idea".to_vec() }]));
    let row = |token| PowerItemRow { holder: SLOT, cause: PowerCause::Idea, item_token: Some(token), ..Default::default() };
    assert_eq!(naming::token_of(Some(&frame), &row(5)), None);
    assert_eq!(naming::token_of(Some(&frame), &row(6)).as_deref(), Some("GER_idea"));
}
