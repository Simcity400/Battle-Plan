use super::{PowerSpendingParams, SpendCoverage, SpendCurrency, SpendReconciliation, naming, spending};
use dashboard_base::{Source, test_fixtures::{CAMPAIGN, records, source}};
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
    frame.metrics.table_ns[table.index()] = 1;
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
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn asked(source: &Source, from: i32, to: i32) -> super::PowerSpendingReply {
    let query = dashboard_base::queries::query(source, 1).unwrap();
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
    let query = dashboard_base::queries::query(&source, 1).unwrap();
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
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-ooo".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
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
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-mid".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
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
}

#[test]
fn format_142_recording_without_day_log_falls_back_to_last_day_end_to_end() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::{ClockRow, PowerSpendTotalRow, compatibility::CAPTURE_ADDITIONS_FORMAT, tables};
    let frames: Vec<DecodedFrame> = (FIRST..=168)
        .map(|hour| {
            let mut frame = source_frame(hour);
            set(&mut frame, TableId::Clock, TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]));
            frame
        })
        .collect();
    let item = PowerItemRow {
        holder: SLOT,
        ordinal: 0,
        kind: PowerItemRow::COST,
        currency: PowerCurrency::Political,
        cause: PowerCause::DailyGain,
        component: Some(PowerDailyCostKind::PoliticalPowerCostModifier as u32),
        charged: true,
        site: PowerSite::AddPoliticalPower,
        first_hour: 120,
        first_read: 144,
        first_country: GER,
        first_epoch: 1,
        first_sequence: 0,
        ..Default::default()
    };
    let total_at = |days: i64, charge: i64, last_day: i32, sequence: u64| PowerSpendTotalRow {
        holder: SLOT,
        currency: PowerCurrency::Political as u8,
        ordinal: 0,
        days: Recorded::Value(days),
        day_payments: Recorded::Value(days),
        day_entries: Recorded::Value(days),
        day_charge: Recorded::Value(charge),
        day_unknown: Recorded::Value(0),
        days_uncovered: Recorded::Value(0),
        realized_gained: Recorded::Value(0),
        realized_lost: Recorded::Value(0),
        realized_unknown: Recorded::Value(days),
        first_day: Recorded::Value(Some(120)),
        last_day: Recorded::Value(Some(last_day)),
        last_lag: Recorded::Value(Some(24)),
        last_hour: Recorded::Value(Some(last_day)),
        last_sequence: Recorded::Value(Some(sequence)),
        last_ordinal: Recorded::Value(Some(0)),
        last_covered: Recorded::Value(true),
        known: Recorded::Value(0),
        payments: Recorded::Value(0),
        entries: Recorded::Value(0),
        unknown: Recorded::Value(0),
        uncovered: Recorded::Value(0),
        balances: Recorded::Value(0),
        credits: Recorded::Value(0),
        gained: Recorded::NotRecorded,
        capped: Recorded::NotRecorded,
        gain_days: Recorded::NotRecorded,
        gain_unknown_days: Recorded::NotRecorded,
        unmatched: Recorded::NotRecorded,
        ..Default::default()
    };
    let bytes = replay::campaign::fixtures::encode_framed_at(recorder_derive::writers_within, CAPTURE_ADDITIONS_FORMAT, CAMPAIGN.into(), &[records(1, frames)], |synthetic, decoded| {
        let countries = decoded.complete::<tables::Countries>()?;
        synthetic.table(TableId::Countries, Input::Snapshot { rows: countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        let links = decoded.complete::<tables::CountryLinks>()?;
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: links, partial: None, read: true, sample: Sample::FromReadBit })?;
        let coverage = [PowerSpendCoverageRow {
            holder: SLOT,
            political: decoded.hour > FIRST,
            command: decoded.hour > FIRST,
            reason: if decoded.hour == FIRST { PowerSpendCoverageRow::BACKLOG } else { PowerSpendCoverageRow::KNOWN },
            political_since: (decoded.hour > FIRST).then_some(FIRST + 1),
            command_since: (decoded.hour > FIRST).then_some(FIRST + 1),
            ..Default::default()
        }];
        synthetic.table(TableId::PowerSpendCoverage, Input::Snapshot { rows: &coverage, partial: None, read: true, sample: Sample::FromReadBit })?;
        let items = if decoded.hour >= 144 { std::slice::from_ref(&item) } else { &[] };
        synthetic.table(TableId::PowerItems, Input::Snapshot { rows: items, partial: None, read: true, sample: Sample::FromReadBit })?;
        let totals = match decoded.hour {
            ..=143 => Vec::new(),
            144..=167 => vec![total_at(1, 2 * PP, 120, 50)],
            _ => vec![total_at(2, 45 * PP / 10, 144, 80)],
        };
        synthetic.table(TableId::PowerSpendTotals, Input::Snapshot { rows: &totals, partial: None, read: true, sample: Sample::FromReadBit })?;
        Ok(())
    })
    .unwrap();
    let replay = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
    assert_eq!(replay.format.version(), CAPTURE_ADDITIONS_FORMAT);
    assert!(replay.recording().predates(TableId::PowerSpendDays.index()).is_some());
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-f142".into(), revision: "1".into() }, "format142.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = asked(&source, FIRST, 168);
    let listed: Vec<_> = reply.days.iter().map(|day| (day.read, day.day, day.days, day.payments, day.charge_raw.clone(), day.credits)).collect();
    assert_eq!(
        listed,
        [
            (144, Some(120), 1, 1, Some("200000".into()), 0),
            (168, Some(144), 1, 1, Some("250000".into()), 0),
        ],
        "an encoded format-142 recording without PowerSpendDays opens and reads recurring days via the last_day fallback"
    );
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

#[test]
fn two_native_days_in_one_read_hour_keep_separate_dates_and_amounts() {
    const GER2: u32 = 9002;
    let relation = |country: u32, day_hour: i32, amount: i64, sequence: u64| CountryPowerDailyCostRow {
        country,
        ordinal: 0,
        epoch: 1,
        day_hour,
        pp_sequence: sequence,
        component: PowerDailyCostKind::Relation,
        cost_raw: amount,
        cause: PowerCause::Diplomacy,
        site: PowerSite::AddPoliticalPowerNoDeficit,
        identity: replay_data::rows::PowerIdentity { target_country: Some(555), ..Default::default() },
        ..Default::default()
    };
    let blockade = |country: u32, day_hour: i32, amount: i64, sequence: u64| CountryPowerDailyCostRow {
        country,
        ordinal: 1,
        epoch: 1,
        day_hour,
        pp_sequence: sequence,
        component: PowerDailyCostKind::Blockade,
        cost_raw: amount,
        cause: PowerCause::Blockade,
        site: PowerSite::AddPoliticalPowerNoDeficit,
        ..Default::default()
    };
    let make_frame = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: SLOT, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
        set(
            &mut frame,
            TableId::CountryLinks,
            TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: SLOT }, CountryLinkRow { id: GER2, canonical_slot: SLOT }]),
        );
        set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(Vec::new()));
        let mut days = Vec::new();
        let mut costs = Vec::new();
        if (144..=167).contains(&hour) {
            days.push(daily(144, 20 * PP, 15 * PP, 5 * PP, 80));
            costs.push(relation(GER, 144, 10 * PP, 80));
            costs.push(blockade(GER, 144, 5 * PP, 80));
        }
        if hour == 168 {
            days.push(CountryPowerDailyRow { country: GER2, ..daily(120, 40 * PP, 27 * PP, 13 * PP, 50) });
            costs.push(relation(GER2, 120, 20 * PP, 50));
            costs.push(blockade(GER2, 120, 7 * PP, 50));
        }
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
        let captures = [GER, GER2]
            .into_iter()
            .map(|country| PoliticsCaptureCoverageRow { country, epoch: 1, since_hour: FIRST, through_hour: hour, streams: PoliticsCaptureCoverageRow::ALL_STREAMS, ..Default::default() })
            .collect();
        set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(captures));
        frame
    };
    let frames = (FIRST..=168).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-same-read".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = asked(&source, FIRST, 168);
    assert!(reply.payments.is_empty(), "relation (Diplomacy) and blockade daily costs must not emit one-off payments");
    let listed: Vec<_> = reply
        .days
        .iter()
        .map(|day| (reply.items[day.item as usize].cause.as_str(), day.read, day.day, day.days, day.payments, day.charge_raw.clone(), day.credits))
        .collect();
    assert_eq!(
        listed,
        [
            ("Diplomacy", 168, Some(120), 1, 1, Some((20 * PP).to_string()), 0),
            ("Blockade", 168, Some(120), 1, 1, Some((7 * PP).to_string()), 0),
            ("Diplomacy", 168, Some(144), 1, 1, Some((10 * PP).to_string()), 0),
            ("Blockade", 168, Some(144), 1, 1, Some((5 * PP).to_string()), 0),
        ],
        "two native days counted in the same read hour emit separate dated charges for collector Diplomacy and Blockade causes"
    );
}

#[test]
fn unavailable_roll_hour_is_withheld_and_named_while_later_available_roll_displays() {
    let make_frame = |hour: i32| {
        let mut frame = source_frame(hour);
        let day = match hour {
            ..=119 => None,
            120..=143 => Some((daily(120, 3 * PP, 2 * PP, PP, 50), cost(120, 2 * PP, 50))),
            144 => None,
            145..=167 => Some((daily(144, 5 * PP, 25 * PP / 10, 15 * PP / 10, 80), cost(144, 25 * PP / 10, 80))),
            _ => Some((daily(168, 3 * PP, 2 * PP, PP, 110), cost(168, 2 * PP, 110))),
        };
        if hour == 144 {
            set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(Vec::new()));
            frame.tables[TableId::CountryPowerDailyCosts.index()] = TableView::Unavailable { reason: "costs unavailable".into() }.into();
        } else {
            let (days, costs) = day.map_or((Vec::new(), Vec::new()), |(day, cost)| (vec![day], vec![cost]));
            set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
            set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
        }
        frame
    };
    let frames = (FIRST..=168).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-unavailable-roll".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = asked(&source, FIRST, 168);
    let listed: Vec<_> = reply.days.iter().map(|day| (day.read, day.day, day.days, day.payments, day.charge_raw.clone())).collect();
    assert_eq!(
        listed,
        [(168, Some(144), 1, 1, Some("250000".into()))],
        "an unavailable midnight roll must be withheld from reply.days while a later available roll still displays"
    );
    assert!(
        reply.warnings.iter().any(|warning| warning.contains("costs unavailable")),
        "the unavailable day-log gap must propagate its named reason to warnings: {:?}",
        reply.warnings
    );
    assert!(
        reply.coverage.iter().any(|run| run.first_hour <= 144 && 144 <= run.last_hour && !run.covered),
        "hour 144 must not be reported as covered"
    );
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
    let command = spending(&source, &query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: FIRST, to: 168 }, &naming, &AtomicBool::new(false)).unwrap();
    assert!(
        command.warnings.iter().any(|warning| warning.contains("costs unavailable")),
        "command currency also propagates the unavailable day-log gap warning: {:?}",
        command.warnings
    );
    assert!(
        command.coverage.iter().any(|run| run.first_hour <= 144 && 144 <= run.last_hour && !run.covered && run.reason.as_deref().is_some_and(|reason| reason.contains("costs unavailable"))),
        "command currency marks hour 144 uncovered with the named day-log unavailability reason: {:?}",
        command.coverage
    );
}

#[test]
fn overlapping_coverage_gap_preserves_specific_day_log_refusal_on_affected_hour() {
    use dashboard_query::{
        Query,
        identity::{Holder, HolderRun, Holders},
        timeline::{Join, Stretch, Timeline},
    };
    use replay_lanes::lanes::LogGap;
    use replay_columnar::manifest::Status;
    let make_frame = |hour: i32| {
        let mut frame = source_frame(hour);
        let day = match hour {
            ..=119 => None,
            120..=143 => Some((daily(120, 3 * PP, 2 * PP, PP, 50), cost(120, 2 * PP, 50))),
            _ => Some((daily(144, 5 * PP, 25 * PP / 10, 15 * PP / 10, 80), cost(144, 25 * PP / 10, 80))),
        };
        let (days, costs) = day.map_or((Vec::new(), Vec::new()), |(day, cost)| (vec![day], vec![cost]));
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
        if (143..=145).contains(&hour) {
            set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(Vec::new()));
        }
        if hour == 144 {
            frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "links unavailable".into() }.into();
        }
        frame
    };
    let frames = (FIRST..=168).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-overlap-gap".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let timeline = Timeline {
        id: 1,
        first: FIRST,
        last: 168,
        stretches: vec![Stretch { run: 0, first: FIRST, last: 168, join: Join::Start }],
        unheld: Vec::new(),
    };
    let holders = Holders::new(
        [("GER".to_owned(), vec![HolderRun { first: FIRST, last: 168, holder: Holder::Found { slot: SLOT, id: GER }, starts_segment: true }])].into(),
        Vec::new(),
    );
    let held_replay = source.replay();
    let query = Query { replay: &held_replay, timeline: Arc::new(timeline), holders: Arc::new(holders) };
    let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
    let params = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from: FIRST, to: 168 };
    let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
    let at_144 = reply.coverage.iter().find(|run| run.first_hour <= 144 && 144 <= run.last_hour).expect("coverage run for hour 144");
    assert_eq!((at_144.first_hour, at_144.last_hour, at_144.covered), (144, 144, false), "overlapping day-log gap splits at the affected hour: {:?}", reply.coverage);
    let reason = at_144.reason.as_deref().unwrap_or("");
    assert!(
        reason.contains("Spending coverage was not recorded") && reason.contains("links unavailable"),
        "hour 144 must preserve both the coverage fallback and the specific day-log refusal: {reason:?}"
    );
    for unaffected in [143, 145] {
        let run = reply.coverage.iter().find(|run| run.first_hour <= unaffected && unaffected <= run.last_hour).expect("neighbor coverage run");
        assert!(!run.covered);
        assert!(
            !run.reason.as_deref().unwrap_or("").contains("links unavailable"),
            "adjacent hour {unaffected} outside the day-log gap must keep only its own coverage reason: {:?}",
            run.reason
        );
    }

    let mut runs = vec![
        SpendCoverage { first_hour: 140, last_hour: 150, covered: false, reason: Some("Spending coverage was not recorded".into()) },
        SpendCoverage { first_hour: 151, last_hour: 160, covered: true, reason: None },
    ];
    let gaps = [
        LogGap { first: 142, last: 143, status: Status::Unavailable { reason: 1 }, reason: Some("gap a".into()) },
        LogGap { first: 149, last: 153, status: Status::Unavailable { reason: 2 }, reason: Some("gap b".into()) },
    ];
    let mut hours = std::collections::BTreeMap::new();
    super::apply_day_gaps(&mut runs, 0, &gaps, &mut hours);
    assert_eq!(
        runs,
        [
            SpendCoverage { first_hour: 140, last_hour: 141, covered: false, reason: Some("Spending coverage was not recorded".into()) },
            SpendCoverage { first_hour: 142, last_hour: 143, covered: false, reason: Some("Spending coverage was not recorded; Daily spending log unavailable: gap a".into()) },
            SpendCoverage { first_hour: 144, last_hour: 148, covered: false, reason: Some("Spending coverage was not recorded".into()) },
            SpendCoverage { first_hour: 149, last_hour: 150, covered: false, reason: Some("Spending coverage was not recorded; Daily spending log unavailable: gap b".into()) },
            SpendCoverage { first_hour: 151, last_hour: 153, covered: false, reason: Some("Daily spending log unavailable: gap b".into()) },
            SpendCoverage { first_hour: 154, last_hour: 160, covered: true, reason: None },
        ],
        "forward merge splits uncovered and covered runs across overlapping day-log gaps without dropping or hiding reasons"
    );
}

#[test]
fn same_run_stretch_boundary_reads_first_hour_log_event_and_gap() {
    use dashboard_query::{
        Query,
        identity::{Holder, HolderRun, Holders},
        timeline::{Join, Stretch, Timeline, Unheld},
    };
    let build = |unavailable_at_boundary: bool| {
        let frames: Vec<DecodedFrame> = (FIRST..=168)
            .map(|hour| {
                let mut frame = source_frame(hour);
                if unavailable_at_boundary && hour == 144 {
                    set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(Vec::new()));
                    frame.tables[TableId::CountryPowerDailyCosts.index()] = TableView::Unavailable { reason: "boundary costs unavailable".into() }.into();
                } else {
                    let day = match hour {
                        ..=119 => None,
                        120..=143 => Some((daily(120, 3 * PP, 2 * PP, PP, 50), cost(120, 2 * PP, 50))),
                        _ => Some((daily(144, 5 * PP, 25 * PP / 10, 15 * PP / 10, 80), cost(144, 25 * PP / 10, 80))),
                    };
                    let (days, costs) = day.map_or((Vec::new(), Vec::new()), |(day, cost)| (vec![day], vec![cost]));
                    set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
                    set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: format!("power-ledger-stretch-{unavailable_at_boundary}"), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let timeline = Timeline {
            id: 1,
            first: FIRST,
            last: 168,
            stretches: vec![
                Stretch { run: 0, first: FIRST, last: 140, join: Join::Start },
                Stretch { run: 0, first: 144, last: 168, join: Join::Break },
            ],
            unheld: vec![Unheld { first: 141, last: 143, reason: "gap".into() }],
        };
        let run = |first: i32, last: i32| HolderRun { first, last, holder: Holder::Found { slot: SLOT, id: GER }, starts_segment: true };
        let holders = Holders::new([("GER".to_owned(), vec![run(FIRST, 140), run(144, 168)])].into(), Vec::new());
        let replay = source.replay();
        let query = Query { replay: &replay, timeline: Arc::new(timeline), holders: Arc::new(holders) };
        let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
        let params = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from: 144, to: 144 };
        spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap()
    };

    let available = build(false);
    let listed: Vec<_> = available.days.iter().map(|day| (day.read, day.day, day.days, day.payments, day.charge_raw.clone())).collect();
    assert_eq!(
        listed,
        [(144, Some(120), 1, 1, Some("200000".into()))],
        "a 1-hour query on the first hour of a later same-run stretch must still read its PowerSpendDays event"
    );

    let unavailable = build(true);
    assert!(unavailable.days.is_empty(), "an unavailable roll at the stretch boundary must be withheld");
    assert!(
        unavailable.warnings.iter().any(|warning| warning.contains("boundary costs unavailable")),
        "a 1-hour query on an unavailable stretch boundary must still report the day-log gap warning: {:?}",
        unavailable.warnings
    );
    assert!(
        unavailable
            .coverage
            .iter()
            .any(|run| run.first_hour == 144 && run.last_hour == 144 && !run.covered && run.reason.as_deref().is_some_and(|reason| reason.contains("boundary costs unavailable"))),
        "a 1-hour query on an unavailable stretch boundary must preserve the day-log refusal in coverage: {:?}",
        unavailable.coverage
    );
}

#[test]
fn daily_credit_carries_its_native_date_separately_from_debits() {
    let make_frame = |hour: i32| {
        let mut frame = source_frame(hour);
        let (days, costs) = if (120..=143).contains(&hour) {
            (vec![daily(120, 3 * PP, -2 * PP, 5 * PP, 50)], vec![cost(120, -2 * PP, 50)])
        } else {
            (Vec::new(), Vec::new())
        };
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
        frame
    };
    let frames = (FIRST..=144).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-credit".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = asked(&source, FIRST, 144);
    let listed: Vec<_> = reply.days.iter().map(|day| (day.read, day.day, day.days, day.payments, day.charge_raw.clone(), day.credits)).collect();
    assert_eq!(
        listed,
        [(144, Some(120), 0, 0, Some("0".into()), 1)],
        "a settled daily credit carries its native date while staying separate from debits"
    );
    assert!(reply.warnings.iter().any(|warning| warning.contains("recorded credit is not included in spending")));
}

#[test]
fn instant_gains_and_balances_reconcile_across_aliases_epochs_caps_and_unknown_operands() {
    const GER2: u32 = 9002;
    let grant = |country: u32, sequence: u64, epoch: u64, hour: i32, currency: PowerCurrency, operation: PowerOperation, before: i64, after: i64, requested: Option<i64>, cause: PowerCause| {
        CountryPowerChangeRow {
            country,
            sequence,
            epoch,
            hour,
            currency,
            operation,
            balance_before_raw: before,
            balance_after_raw: after,
            requested_delta_raw: requested,
            cause,
            site: if currency == PowerCurrency::Political { PowerSite::AddPoliticalPower } else { PowerSite::AddCommandPower },
            ..Default::default()
        }
    };
    let make_frame = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        let pp = match hour {
            ..=103 => 100 * PP,
            104..=105 => 220 * PP,
            106..=107 => 250 * PP,
            108..=109 => 200 * PP,
            _ => 260 * PP,
        };
        let cp = match hour {
            ..=104 => 120 * PP,
            _ => 100 * PP,
        };
        set(
            &mut frame,
            TableId::Countries,
            TableRows::Countries(vec![CountryRow {
                id: GER,
                canonical_slot: SLOT,
                tag: b"GER".to_vec(),
                political_power_raw: Some(pp),
                command_power_raw: cp,
                border_land_doctrine_level: Recorded::Value(None),
                ..Default::default()
            }]),
        );
        let links = if hour >= 106 {
            vec![CountryLinkRow { id: GER, canonical_slot: SLOT }, CountryLinkRow { id: GER2, canonical_slot: SLOT }]
        } else {
            vec![CountryLinkRow { id: GER, canonical_slot: SLOT }]
        };
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(links));
        let changes = match hour {
            104 => vec![grant(GER, 1, 1, 103, PowerCurrency::Political, PowerOperation::Add, 100 * PP, 220 * PP, Some(120 * PP), PowerCause::Focus)],
            105 => vec![
                grant(GER, 1, 1, 103, PowerCurrency::Political, PowerOperation::Add, 100 * PP, 220 * PP, Some(120 * PP), PowerCause::Focus),
                grant(GER, 2, 1, 105, PowerCurrency::Command, PowerOperation::Grant, 120 * PP, 100 * PP, Some(10 * PP), PowerCause::ScriptEffect),
            ],
            106 => vec![grant(GER2, 3, 1, 106, PowerCurrency::Political, PowerOperation::Refund, 220 * PP, 250 * PP, Some(30 * PP), PowerCause::FocusRefund)],
            108 => vec![grant(GER, 4, 1, 108, PowerCurrency::Political, PowerOperation::Set, 250 * PP, 200 * PP, Some(-50 * PP), PowerCause::ScriptEffect)],
            110 => vec![grant(GER, 1, 2, 110, PowerCurrency::Political, PowerOperation::Set, 200 * PP, 260 * PP, Some(60 * PP), PowerCause::ScriptEffect)],
            112 => vec![grant(GER, 5, 2, 112, PowerCurrency::Political, PowerOperation::Add, 260 * PP, 260 * PP, None, PowerCause::ScriptEffect)],
            _ => Vec::new(),
        };
        set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(changes));
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(Vec::new()));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(Vec::new()));
        let capture = PoliticsCaptureCoverageRow { country: GER, epoch: 1, since_hour: FIRST, through_hour: hour, streams: PoliticsCaptureCoverageRow::ALL_STREAMS, ..Default::default() };
        set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(vec![capture]));
        frame
    };
    let frames = (FIRST..=114).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-instant".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let pp_reply = asked(&source, FIRST, 114);
    let gains: Vec<_> = pp_reply.gains.iter().map(|gain| (gain.read, gain.hour, gain.gained_raw.as_deref(), gain.capped_raw.as_deref(), gain.unknown_days)).collect();
    assert_eq!(
        gains,
        [
            (104, Some(103), Some("12000000"), Some("0"), 0),
            (106, Some(106), Some("3000000"), Some("0"), 0),
            (110, Some(110), Some("6000000"), Some("0"), 0),
            (112, Some(112), None, None, 1),
        ],
        "instant gains deduplicate same epoch, attribute aliases, skip downward Set, process new epoch on reused sequence, and withhold unknown requested delta"
    );
    assert_eq!(pp_reply.gain_coverage, [SpendCoverage { first_hour: FIRST, last_hour: 114, covered: true, reason: None }]);
    assert_eq!(pp_reply.balances.first().and_then(|run| run.value_raw.as_deref()), Some("10000000"));
    assert_eq!(pp_reply.balances.last().and_then(|run| run.value_raw.as_deref()), Some("26000000"));

    let range_reply = asked(&source, 105, 111);
    assert_eq!(
        range_reply.balances.first().map(|run| (run.first_hour, run.value_raw.as_deref())),
        Some((104, Some("22000000"))),
        "range query includes baseline hour balance at from - 1"
    );

    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
    let cp_reply = spending(&source, &query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: FIRST, to: 114 }, &naming, &AtomicBool::new(false)).unwrap();
    let cp_gains: Vec<_> = cp_reply.gains.iter().map(|gain| (gain.read, gain.hour, gain.gained_raw.as_deref(), gain.capped_raw.as_deref())).collect();
    assert_eq!(
        cp_gains,
        [(105, Some(105), Some("1000000"), Some("3000000"))],
        "over-cap CP grant +10 moving 120 -> 100 computes full signed cap loss 30, not 10"
    );
}

#[test]
fn unavailable_power_changes_or_countries_withholds_gain_coverage_and_balance_runs() {
    let make_frame = |hour: i32| {
        let mut frame = source_frame(hour);
        if hour == 105 {
            frame.tables[TableId::CountryPowerChanges.index()] = TableView::Unavailable { reason: "changes gap".into() }.into();
        }
        if hour == 107 {
            frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "countries gap".into() }.into();
        } else {
            set(
                &mut frame,
                TableId::Countries,
                TableRows::Countries(vec![CountryRow {
                    id: GER,
                    canonical_slot: SLOT,
                    tag: b"GER".to_vec(),
                    political_power_raw: (hour != 109).then_some(50 * PP),
                    border_land_doctrine_level: Recorded::Value(None),
                    ..Default::default()
                }]),
            );
        }
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(Vec::new()));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(Vec::new()));
        frame
    };
    let frames = (FIRST..=110).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-gaps".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = asked(&source, FIRST, 110);
    assert!(
        reply.gain_coverage.iter().any(|run| run.first_hour == 105 && run.last_hour == 105 && !run.covered && run.reason.is_some()),
        "unavailable CountryPowerChanges marks hour 105 uncovered in gain_coverage: {:?}",
        reply.gain_coverage
    );
    assert!(
        reply.balances.iter().any(|run| run.first_hour == 107 && run.last_hour == 107 && run.value_raw.is_none() && run.reason.is_some()),
        "unavailable Countries marks hour 107 missing in balances: {:?}",
        reply.balances
    );
    let changes_gap = asked(&source, 104, 106);
    assert_eq!(changes_gap.reconciliation.opening_raw.as_deref(), Some("5000000"));
    assert_eq!(changes_gap.reconciliation.closing_raw.as_deref(), Some("5000000"));
    assert_eq!(changes_gap.reconciliation.gained_raw, None);
    assert_eq!(changes_gap.reconciliation.other_changes_raw, None);
    assert!(
        changes_gap.reconciliation.reason.as_deref().is_some_and(|reason| reason.contains("changes gap") || reason.contains("Power changes")),
        "unexpected changes gap reason: {:?}",
        changes_gap.reconciliation.reason
    );

    let closing_gap = asked(&source, 106, 107);
    assert_eq!(closing_gap.reconciliation.opening_raw.as_deref(), Some("5000000"));
    assert_eq!(closing_gap.reconciliation.closing_raw, None);
    assert_eq!(closing_gap.reconciliation.other_changes_raw, None);
    assert!(closing_gap.reconciliation.reason.is_some());

    let opening_gap = asked(&source, 108, 108);
    assert_eq!(opening_gap.reconciliation.opening_raw, None);
    assert_eq!(opening_gap.reconciliation.closing_raw.as_deref(), Some("5000000"));
    assert_eq!(opening_gap.reconciliation.other_changes_raw, None);
    assert!(opening_gap.reconciliation.reason.as_deref().is_some_and(|reason| reason.contains("Country balances")));

    let missing_balance = asked(&source, 108, 109);
    assert_eq!(missing_balance.reconciliation.closing_raw, None);
    assert_eq!(missing_balance.reconciliation.other_changes_raw, None);
    assert!(missing_balance.reconciliation.reason.is_some());

    let missing_closing_only = asked(&source, 109, 109);
    assert_eq!(missing_closing_only.reconciliation.opening_raw.as_deref(), Some("5000000"));
    assert_eq!(missing_closing_only.reconciliation.closing_raw, None);
    assert_eq!(missing_closing_only.reconciliation.other_changes_raw, None);
    assert_eq!(missing_closing_only.reconciliation.reason.as_deref(), Some("Balance was not recorded"));

    let missing_opening_only = asked(&source, 110, 110);
    assert_eq!(missing_opening_only.reconciliation.opening_raw, None);
    assert_eq!(missing_opening_only.reconciliation.closing_raw.as_deref(), Some("5000000"));
    assert_eq!(missing_opening_only.reconciliation.other_changes_raw, None);
    assert_eq!(missing_opening_only.reconciliation.reason.as_deref(), Some("Balance was not recorded"));
}

#[test]
fn reconciliation_verifies_identity_for_political_and_command_power_and_withholds_on_gaps() {
    use replay_data::{format::FormatVersion, rows::{ClockRow, tables}};
    use replay_journal::synthetic::{Input, Sample};

    let make_frame = |branch: u64, hour: i32| {
        let mut frame = source(branch, hour, Some(1_000));
        set(&mut frame, TableId::Clock, TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]));
        let pp = match hour {
            ..=103 => 100 * PP,
            104..=109 => 220 * PP,
            110..=143 => 170 * PP,
            _ => 171 * PP,
        };
        let cp = match hour {
            ..=104 => 120 * PP,
            105..=109 => 100 * PP,
            110..=114 => 75 * PP,
            _ => 100 * PP,
        };
        set(
            &mut frame,
            TableId::Countries,
            TableRows::Countries(vec![CountryRow {
                id: GER,
                canonical_slot: SLOT,
                tag: b"GER".to_vec(),
                political_power_raw: Some(pp),
                command_power_raw: cp,
                border_land_doctrine_level: Recorded::Value(None),
                ..Default::default()
            }]),
        );
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: SLOT }]));
        let changes = match hour {
            104 => vec![CountryPowerChangeRow {
                country: GER,
                sequence: 1,
                epoch: 1,
                hour: 104,
                currency: PowerCurrency::Political,
                operation: PowerOperation::Add,
                balance_before_raw: 100 * PP,
                balance_after_raw: 220 * PP,
                requested_delta_raw: Some(120 * PP),
                cause: PowerCause::Focus,
                site: PowerSite::AddPoliticalPower,
                ..Default::default()
            }],
            105 => vec![CountryPowerChangeRow {
                country: GER,
                sequence: 2,
                epoch: 1,
                hour: 105,
                currency: PowerCurrency::Command,
                operation: PowerOperation::Grant,
                balance_before_raw: 120 * PP,
                balance_after_raw: 100 * PP,
                requested_delta_raw: Some(10 * PP),
                cause: PowerCause::ScriptEffect,
                site: PowerSite::AddCommandPower,
                ..Default::default()
            }],
            110 => vec![
                decision(3, 110, 50 * PP),
                CountryPowerChangeRow {
                    country: GER,
                    sequence: 4,
                    epoch: 1,
                    hour: 110,
                    currency: PowerCurrency::Command,
                    operation: PowerOperation::Subtract,
                    balance_before_raw: 100 * PP,
                    balance_after_raw: 75 * PP,
                    requested_delta_raw: Some(-25 * PP),
                    cause: PowerCause::Allocation,
                    site: PowerSite::AllocateCommandPower,
                    ..Default::default()
                },
            ],
            115 => vec![CountryPowerChangeRow {
                country: GER,
                sequence: 5,
                epoch: 1,
                hour: 115,
                currency: PowerCurrency::Command,
                operation: PowerOperation::Add,
                balance_before_raw: 75 * PP,
                balance_after_raw: 100 * PP,
                requested_delta_raw: Some(25 * PP),
                cause: PowerCause::AllocationRefund,
                site: PowerSite::ReleaseCommandPower,
                ..Default::default()
            }],
            _ => Vec::new(),
        };
        set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(changes));
        let (days, costs) = if (120..=143).contains(&hour) {
            (vec![daily(120, 3 * PP, 2 * PP, PP, 50)], vec![cost(120, 2 * PP, 50)])
        } else {
            (Vec::new(), Vec::new())
        };
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
        let capture = PoliticsCaptureCoverageRow { country: GER, epoch: 1, since_hour: FIRST, through_hour: hour, streams: PoliticsCaptureCoverageRow::ALL_STREAMS, ..Default::default() };
        set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(vec![capture]));
        frame
    };

    let frames: Vec<DecodedFrame> = (FIRST..=146).map(|hour| make_frame(1, hour)).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "power-reconcile-exact".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();

    let pp_span = asked(&src, 102, 146);
    assert_eq!(
        pp_span.reconciliation,
        SpendReconciliation {
            opening_raw: Some((100 * PP).to_string()),
            gained_raw: Some((123 * PP).to_string()),
            lost_at_cap_raw: Some("0".into()),
            spent_raw: Some((52 * PP).to_string()),
            allocated_raw: Some("0".into()),
            released_raw: Some("0".into()),
            closing_raw: Some((171 * PP).to_string()),
            other_changes_raw: Some("0".into()),
            reason: None,
        },
        "PP span reconciles opening + daily & instant gains - daily & instant spend == closing with 0 other changes"
    );

    let pp_from_start = asked(&src, FIRST, 146);
    assert_eq!(pp_from_start.reconciliation.opening_raw, None);
    assert_eq!(pp_from_start.reconciliation.allocated_raw, None);
    assert_eq!(pp_from_start.reconciliation.released_raw, None);
    assert_eq!(pp_from_start.reconciliation.other_changes_raw, None);
    assert!(pp_from_start.reconciliation.reason.is_some());

    let query = dashboard_base::queries::query(&src, 1).unwrap();
    let naming = naming::context(&src, &LoadContext { content: None }).unwrap();
    let cp_hold = spending(&src, &query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: 102, to: 112 }, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        cp_hold.reconciliation,
        SpendReconciliation {
            opening_raw: Some((120 * PP).to_string()),
            gained_raw: Some((10 * PP).to_string()),
            lost_at_cap_raw: Some((30 * PP).to_string()),
            spent_raw: Some("0".into()),
            allocated_raw: Some((25 * PP).to_string()),
            released_raw: Some("0".into()),
            closing_raw: Some((75 * PP).to_string()),
            other_changes_raw: Some("0".into()),
            reason: None,
        },
        "CP hold of 25 CP after over-cap grant attributes 25 CP to allocated and reconciles to exact 0 other_changes"
    );
    let cp_released = spending(&src, &query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: 102, to: 116 }, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        cp_released.reconciliation,
        SpendReconciliation {
            opening_raw: Some((120 * PP).to_string()),
            gained_raw: Some((10 * PP).to_string()),
            lost_at_cap_raw: Some((30 * PP).to_string()),
            spent_raw: Some("0".into()),
            allocated_raw: Some((25 * PP).to_string()),
            released_raw: Some((25 * PP).to_string()),
            closing_raw: Some((100 * PP).to_string()),
            other_changes_raw: Some("0".into()),
            reason: None,
        },
        "CP hold followed by release reports both allocated and released totals with 0 other_changes"
    );

    let run0: Vec<DecodedFrame> = (FIRST..=106).map(|hour| make_frame(1, hour)).collect();
    let run1: Vec<DecodedFrame> = (107..=112).map(|hour| make_frame(2, hour)).collect();
    let split = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, run0), records(2, run1)]).unwrap().into_timelines(&Default::default()).unwrap();
    let split_src = Source::from_replay(dashboard_base::SourceKey { source_id: "power-reconcile-split".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(split), &AtomicBool::new(false)).unwrap();
    let split_reply = asked(&split_src, 102, 112);
    assert_eq!(split_reply.reconciliation.allocated_raw, None);
    assert_eq!(split_reply.reconciliation.released_raw, None);
    assert_eq!(split_reply.reconciliation.other_changes_raw, None);
    assert!(split_reply.reconciliation.reason.is_some());

    let held_frames: Vec<DecodedFrame> = (FIRST..=110).map(|hour| make_frame(1, hour)).collect();
    let held_bytes = replay::campaign::fixtures::encode_framed_at(recorder_derive::writers_within, FormatVersion::CURRENT.version(), CAMPAIGN.into(), &[records(1, held_frames)], |synthetic, decoded| {
        let countries = decoded.complete::<tables::Countries>()?;
        synthetic.table(TableId::Countries, Input::Snapshot { rows: countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        let links = decoded.complete::<tables::CountryLinks>()?;
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: links, partial: None, read: true, sample: Sample::FromReadBit })?;
        let changes = decoded.complete::<tables::CountryPowerChanges>()?;
        synthetic.table(TableId::CountryPowerChanges, Input::Snapshot { rows: changes, partial: None, read: decoded.hour != 105, sample: Sample::FromReadBit })?;
        let daily_rows = decoded.complete::<tables::CountryPowerDaily>()?;
        synthetic.table(TableId::CountryPowerDaily, Input::Snapshot { rows: daily_rows, partial: None, read: true, sample: Sample::FromReadBit })?;
        let cost_rows = decoded.complete::<tables::CountryPowerDailyCosts>()?;
        synthetic.table(TableId::CountryPowerDailyCosts, Input::Snapshot { rows: cost_rows, partial: None, read: true, sample: Sample::FromReadBit })?;
        let cov = decoded.complete::<tables::PoliticsCaptureCoverage>()?;
        synthetic.table(TableId::PoliticsCaptureCoverage, Input::Snapshot { rows: cov, partial: None, read: true, sample: Sample::FromReadBit })?;
        Ok(())
    })
    .unwrap();
    let held_replay = Replay::open(held_bytes).unwrap().into_timelines(&Default::default()).unwrap();
    let held_src = Source::from_replay(dashboard_base::SourceKey { source_id: "power-reconcile-held".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(held_replay), &AtomicBool::new(false)).unwrap();
    let held_reply = asked(&held_src, 102, 108);
    assert_eq!(held_reply.reconciliation.gained_raw, None);
    assert_eq!(held_reply.reconciliation.allocated_raw, None);
    assert_eq!(held_reply.reconciliation.released_raw, None);
    assert_eq!(held_reply.reconciliation.other_changes_raw, None);
    assert!(
        held_reply.reconciliation.reason.as_deref().is_some_and(|reason| reason.contains("was not read at this hour")),
        "held Complete status without fresh read withholds reconciliation: {:?}",
        held_reply.reconciliation
    );
}

#[test]
fn command_power_ground_crews_capped_refunds_and_allocation_clamps_reconcile() {
    let make_frame = |hour: i32| {
        let mut frame = source(1, hour, Some(1_000));
        let cp = match hour {
            ..=103 => 100 * PP,
            104..=105 => 80 * PP,
            106..=107 => 100 * PP,
            108..=109 => 85 * PP,
            _ => 95 * PP,
        };
        set(
            &mut frame,
            TableId::Countries,
            TableRows::Countries(vec![CountryRow {
                id: GER,
                canonical_slot: SLOT,
                tag: b"GER".to_vec(),
                command_power_raw: cp,
                border_land_doctrine_level: Recorded::Value(None),
                ..Default::default()
            }]),
        );
        set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: SLOT }]));
        let changes = match hour {
            104 => vec![CountryPowerChangeRow {
                country: GER,
                sequence: 1,
                epoch: 1,
                hour: 104,
                currency: PowerCurrency::Command,
                operation: PowerOperation::Subtract,
                balance_before_raw: 100 * PP,
                balance_after_raw: 80 * PP,
                requested_delta_raw: Some(-20 * PP),
                cause: PowerCause::AirAction,
                site: PowerSite::GroundCrew,
                ..Default::default()
            }],
            106 => vec![CountryPowerChangeRow {
                country: GER,
                sequence: 2,
                epoch: 1,
                hour: 106,
                currency: PowerCurrency::Command,
                operation: PowerOperation::Refund,
                balance_before_raw: 80 * PP,
                balance_after_raw: 100 * PP,
                requested_delta_raw: Some(20 * PP),
                cause: PowerCause::AllocationRefund,
                site: PowerSite::GroundCrew,
                ..Default::default()
            }],
            108 => vec![CountryPowerChangeRow {
                country: GER,
                sequence: 3,
                epoch: 1,
                hour: 108,
                currency: PowerCurrency::Command,
                operation: PowerOperation::AllocationClamp,
                balance_before_raw: 100 * PP,
                balance_after_raw: 85 * PP,
                requested_delta_raw: None,
                cause: PowerCause::CapCorrection,
                site: PowerSite::AllocateCommandPower,
                ..Default::default()
            }],
            110 => vec![CountryPowerChangeRow {
                country: GER,
                sequence: 4,
                epoch: 1,
                hour: 110,
                currency: PowerCurrency::Command,
                operation: PowerOperation::Refund,
                balance_before_raw: 85 * PP,
                balance_after_raw: 95 * PP,
                requested_delta_raw: Some(25 * PP),
                cause: PowerCause::AllocationRefund,
                site: PowerSite::AddCommandPower,
                ..Default::default()
            }],
            _ => Vec::new(),
        };
        set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(changes));
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(Vec::new()));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(Vec::new()));
        let capture = PoliticsCaptureCoverageRow { country: GER, epoch: 1, since_hour: FIRST, through_hour: hour, streams: PoliticsCaptureCoverageRow::ALL_STREAMS, ..Default::default() };
        set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(vec![capture]));
        frame
    };
    let frames: Vec<DecodedFrame> = (FIRST..=112).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let src = Source::from_replay(dashboard_base::SourceKey { source_id: "power-reconcile-cp-shapes".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&src, 1).unwrap();
    let naming = naming::context(&src, &LoadContext { content: None }).unwrap();
    let reply = spending(&src, &query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: 102, to: 112 }, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reply.reconciliation,
        SpendReconciliation {
            opening_raw: Some((100 * PP).to_string()),
            gained_raw: Some("0".into()),
            lost_at_cap_raw: Some((30 * PP).to_string()),
            spent_raw: Some("0".into()),
            allocated_raw: Some((20 * PP).to_string()),
            released_raw: Some((45 * PP).to_string()),
            closing_raw: Some((95 * PP).to_string()),
            other_changes_raw: Some("0".into()),
            reason: None,
        },
        "ground-crew start/stop, allocation clamp, and capped refund reconcile to exact 0 other_changes"
    );
}

#[test]
fn a_payment_whose_item_is_not_witnessed_at_the_spans_end_keeps_its_amount_with_an_unrecorded_item() {
    let frames = (FIRST..=LAST)
        .map(|hour| {
            let mut frame = frame(hour);
            if hour == LAST {
                frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "countries refused".into() }.into();
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-unwitnessed".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    use dashboard_query::{
        Query,
        identity::{Holder, HolderRun, Holders},
        timeline::{Join, Stretch, Timeline},
    };
    let held = HolderRun { first: FIRST, last: LAST, holder: Holder::Found { slot: SLOT, id: GER }, starts_segment: true };
    let holders = Holders::new([("GER".to_owned(), vec![held])].into(), Vec::new());
    let timeline = Timeline { id: 1, first: FIRST, last: LAST, stretches: vec![Stretch { run: 0, first: FIRST, last: LAST, join: Join::Start }], unheld: Vec::new() };
    let replay = source.replay();
    let query = Query { replay: &replay, timeline: Arc::new(timeline), holders: Arc::new(holders) };
    let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
    let params = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from: FIRST, to: LAST };
    let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
    let reads: Vec<_> = reply.payments.iter().map(|payment| (payment.read, payment.known_raw.clone())).collect();
    assert_eq!(reads, [(104, Some("5000000".into())), (120, Some("7500000".into()))], "witnessed spending keeps its amount");
    let item = &reply.items[reply.payments[1].item as usize];
    assert_eq!((item.category.as_str(), item.name.as_deref()), ("notIdentified", None));
    assert!(item.unknown.as_deref().is_some_and(|why| why.starts_with("Power items")), "{:?}", item.unknown);
    assert!(reply.categories.iter().any(|category| category.id == "notIdentified" && !category.identified));
}

#[test]
fn pieces_apart_only_across_unrecorded_hours_break_continuity_and_the_statement_names_its_own_end() {
    use super::{Piece, continuity};
    let piece = |first: i32, last: i32, baseline: Option<i32>| Piece { run: 0, slot: SLOT, id: GER, run_first: FIRST, first, last, baseline };
    let held = [piece(FIRST, 140, None), piece(144, 168, Some(144))];
    let unrecorded_gap = |first: i32, last: i32| i64::from(!(first >= 141 && last <= 143));
    let every = |first: i32, last: i32| i64::from(last - first + 1);
    assert_eq!(continuity(FIRST, 168, &held, unrecorded_gap, |_, _| false).as_deref(), Some("Continuity broke across a load or recording gap during this span"));
    assert_eq!(continuity(FIRST, 168, &held, every, |_, _| false).as_deref(), Some("Country was not held for the entire span"), "recorded hours between pieces are hours not held");

    use dashboard_query::{
        Query,
        identity::{Holder, HolderRun, Holders},
        timeline::{Join, Stretch, Timeline},
    };
    let source = recording();
    let run = |first: i32, last: i32, id: u32| HolderRun { first, last, holder: Holder::Found { slot: SLOT, id }, starts_segment: true };
    let holders = Holders::new([("GER".to_owned(), vec![run(FIRST, 140, GER)]), ("ITA".to_owned(), vec![run(141, LAST, 9002)])].into(), Vec::new());
    let timeline = Timeline { id: 1, first: FIRST, last: LAST, stretches: vec![Stretch { run: 0, first: FIRST, last: LAST, join: Join::Start }], unheld: Vec::new() };
    let replay = source.replay();
    let query = Query { replay: &replay, timeline: Arc::new(timeline), holders: Arc::new(holders) };
    let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
    let params = PowerSpendingParams { branch_id: "1".into(), country: dashboard_base::products::Country::Slot(SLOT), currency: SpendCurrency::Political, from: FIRST, to: 130 };
    let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.tag, "GER", "a slot is named by who held it at the statement's end, not at the branch's last hour");

    let later = Holders::new([("GER".to_owned(), vec![run(110, LAST, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: query.timeline, holders: Arc::new(later) };
    let params = PowerSpendingParams { to: FIRST, ..params };
    let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.tag, params.country.key(), "a slot held by no one at the statement's end is the slot, never its later holder");

    let earlier = Holders::new([("GER".to_owned(), vec![run(FIRST, 140, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: query.timeline, holders: Arc::new(earlier) };
    let params = PowerSpendingParams { to: 150, ..params };
    let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.tag, params.country.key(), "a slot held by no one at the statement's end is the slot, never its earlier holder");

    let through = Holders::new([("GER".to_owned(), vec![run(FIRST, LAST, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: query.timeline, holders: Arc::new(through) };
    let params = PowerSpendingParams { to: LAST + 1, ..params };
    let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.tag, params.country.key(), "rv5-12b: a statement ending after the branch's last hour is named at its own end, where the slot holds no tag");

    let gap = |first: i32, last: i32| dashboard_query::timeline::Unheld { first, last, reason: "not recorded".into() };
    let stretches = vec![Stretch { run: 0, first: FIRST, last: 140, join: Join::Start }, Stretch { run: 0, first: 144, last: 160, join: Join::Break }];
    let timeline = Timeline { id: 1, first: FIRST, last: LAST, stretches, unheld: vec![gap(141, 143), gap(161, LAST)] };
    let holders = Holders::new([("GER".to_owned(), vec![run(FIRST, LAST, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: Arc::new(timeline), holders: Arc::new(holders) };
    for (from, to, why) in [
        (FIRST, 160, "Continuity broke across a load or recording gap during this span"),
        (FIRST, LAST, "Continuity broke across a load or recording gap during this span"),
        (141, 143, "This span was not recorded"),
    ] {
        let params = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from, to };
        let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.reconciliation.reason.as_deref(), Some(why), "the statement {from}..{to} gives the recording gap as its reason");
    }
    let apart = Holders::new([("GER".to_owned(), vec![run(FIRST, 139, GER), run(144, LAST, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: query.timeline, holders: Arc::new(apart) };
    for (from, to, why) in [(140, 143, "Country was not held for the entire span"), (140, 140, "Country was not held during this span")] {
        let params = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from, to };
        let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.reconciliation.reason.as_deref(), Some(why), "no holder in {from}..{to}: absence for the whole span needs every hour recorded");
    }
    let unread = HolderRun { first: 140, last: 140, holder: Holder::Unresolved, starts_segment: true };
    let unresolved = Holders::new([("GER".to_owned(), vec![run(FIRST, 139, GER), unread, run(144, LAST, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: query.timeline, holders: Arc::new(unresolved) };
    for (from, to) in [(140, 140), (140, 143)] {
        let params = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from, to };
        let reply = spending(&source, &query, &params, &naming, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.reconciliation.reason.as_deref(), Some(super::HOLDER_UNRESOLVED), "R4-S6: an unresolved holder in {from}..{to} is not \"not held\"");
    }
}

#[test]
fn spending_markers_are_payments_at_their_own_hour_shown_once_read_with_the_next_read_as_their_change() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let span = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from: FIRST, to: LAST };
    let held = super::markers::build(&source, &query, &span, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();

    let before = held.at(103);
    assert!(before.markers.is_empty(), "the decision paid at 103 is read at 104: not shown before");
    assert_eq!(before.next_change, Some(104));

    let first = held.at(110);
    let shown: Vec<_> = first.markers.iter().map(|marker| (marker.hour, marker.dated, marker.since, marker.payments, marker.charge_raw.clone())).collect();
    assert_eq!(shown, [(103, true, None, 1, Some("5000000".into()))], "one marker at the payment's own hour, never at a bin's closing hour");
    assert_eq!(first.next_change, Some(120));
    let gaps: Vec<_> = first.unrecorded.iter().map(|gap| (gap.first_hour, gap.last_hour)).collect();
    assert_eq!(gaps, [(FIRST, FIRST)], "an hour whose spending was not counted is said, never left as no spending");

    let last = held.at(LAST);
    assert_eq!(last.markers.iter().map(|marker| marker.hour).collect::<Vec<_>>(), [103, 120]);
    assert_eq!(last.next_change, None);

    let mut whole = asked(&source, FIRST, LAST);
    let item = whole.payments[0].item;
    whole.payments[1].item = item;
    whole.payments[1].payments = 3;
    whole.payments[0].hour = None;
    let undated = super::markers::index(&whole, Vec::new()).at(LAST);
    let shown: Vec<_> = undated.markers.iter().map(|marker| (marker.hour, marker.dated, marker.since, marker.payments)).collect();
    assert_eq!(shown, [(104, false, None, 1), (120, true, Some(105), 3)], "an undated payment is placed by its read; several in one read say the interval they fall in and their count");
}
