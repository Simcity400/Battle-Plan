use super::{PowerSpendingParams, SpendCoverage, SpendCurrency, naming, spending};
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
fn instant_grants_and_refunds_reconcile_gained_minus_spent_with_balance() {
    let grant = |sequence: u64, hour: i32, operation: PowerOperation, cause: PowerCause, before: i64, after: i64, requested: i64| CountryPowerChangeRow {
        country: GER,
        sequence,
        epoch: 1,
        hour,
        currency: PowerCurrency::Political,
        operation,
        balance_before_raw: before,
        balance_after_raw: after,
        requested_delta_raw: Some(requested),
        cause,
        site: PowerSite::AddPoliticalPower,
        ..Default::default()
    };
    let cp_refund = CountryPowerChangeRow {
        country: GER,
        sequence: 90,
        epoch: 1,
        hour: 122,
        currency: PowerCurrency::Command,
        operation: PowerOperation::Refund,
        balance_before_raw: 10 * PP,
        balance_after_raw: 20 * PP,
        requested_delta_raw: Some(10 * PP),
        cause: PowerCause::AllocationRefund,
        site: PowerSite::ReleaseCommandPower,
        ..Default::default()
    };
    let make_frame = |hour: i32| {
        let mut frame = source_frame(hour);
        let changes = match hour {
            FIRST => vec![grant(1, FIRST, PowerOperation::Grant, PowerCause::Focus, 0, 99 * PP, 99 * PP)],
            104 => vec![
                grant(2, 104, PowerOperation::Grant, PowerCause::Focus, 0, 120 * PP, 120 * PP),
                grant(3, 104, PowerOperation::Refund, PowerCause::FocusRefund, 120 * PP, 130 * PP, 10 * PP),
            ],
            120 => vec![
                decision(4, 120, 50 * PP),
                grant(5, 120, PowerOperation::Add, PowerCause::DailyGain, 80 * PP, 81 * PP, PP),
            ],
            122 => vec![cp_refund.clone()],
            144 => vec![
                grant(6, 144, PowerOperation::Grant, PowerCause::ScriptEffect, 1950 * PP, 2000 * PP, 200 * PP),
                grant(7, 144, PowerOperation::Clamp, PowerCause::CapCorrection, 2000 * PP, 2000 * PP, -150 * PP),
            ],
            _ => Vec::new(),
        };
        set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(changes));
        let day = match hour {
            ..=119 => None,
            120..=143 => Some((daily(120, 3 * PP, 2 * PP, PP, 5), cost(120, 2 * PP, 5))),
            _ => Some((daily(144, 5 * PP, 2 * PP, 3 * PP, 8), cost(144, 2 * PP, 8))),
        };
        let (days, costs) = day.map_or((Vec::new(), Vec::new()), |(day, cost)| (vec![day], vec![cost]));
        set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(days));
        set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(costs));
        frame
    };
    let frames = (FIRST..=145).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-grants".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let reply = asked(&source, FIRST, 145);
    let gained: i128 = reply.gains.iter().map(|entry| entry.gained_raw.as_deref().unwrap().parse::<i128>().unwrap()).sum();
    let capped: i128 = reply.gains.iter().map(|entry| entry.capped_raw.as_deref().unwrap().parse::<i128>().unwrap()).sum();
    let spent_payments: i128 = reply.payments.iter().map(|entry| entry.known_raw.as_deref().unwrap().parse::<i128>().unwrap()).sum();
    let spent_days: i128 = reply.days.iter().map(|entry| entry.charge_raw.as_deref().unwrap().parse::<i128>().unwrap()).sum();
    assert_eq!(gained, i128::from(333 * PP), "focus grant 120 + focus refund 10 + daily gross 3 + script grant 200; backlog and DailyGain change excluded");
    assert_eq!(capped, i128::from(150 * PP), "200 PP grant capped at +50 realized loses 150 to the cap");
    assert_eq!(spent_payments + spent_days, i128::from(52 * PP), "50 PP decision + 2 PP daily cost");
    assert_eq!(gained - capped - (spent_payments + spent_days), i128::from(131 * PP), "net balance move equals +120 + 10 - 50 + (3 - 2) + 50");

    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
    let command = spending(&source, &query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: FIRST, to: 145 }, &naming, &AtomicBool::new(false)).unwrap();
    let cp_gained: i128 = command.gains.iter().map(|entry| entry.gained_raw.as_deref().unwrap().parse::<i128>().unwrap()).sum();
    assert_eq!(cp_gained, 0, "command-power allocation release is a hold release, not gained power");
}
