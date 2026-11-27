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
        balance_after_raw: replay_data::rows::RetiredI64(0),
        realized_delta_raw: replay_data::rows::Recorded::Value(-charge),
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
    spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap()
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

    let (whole, counts) = replay_recording::cells::count_reads(|| asked(&source, FIRST, LAST));
    let items = &counts[&TableId::PowerItems];
    assert_eq!((items.opened, items.restores), (1, 1), "a complete single-block run reads PowerItems once at the last asked hour");
    assert_eq!(items.stretches, 0, "a complete span skips the PowerItems membership history walk");
    assert_eq!(items.columns, TableId::PowerItems.schema().len() as u64, "the single end-of-span reader opens the detail columns once");
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
    let reply = spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap();
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
    let command = spending(&query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: FIRST, to: 168 }, &naming, &AtomicBool::new(false)).unwrap();
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
    let reply = spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap();
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
        spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap()
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
fn a_credit_whose_amount_was_not_recorded_is_listed_with_or_without_the_day_log() {
    use super::{Change, TOTALS, Value, listed};
    let at = |name: &str| TOTALS.iter().position(|held| *held == name).unwrap();
    let before = [Value::Number(0); TOTALS.len()];
    let mut after = before;
    after[at("unknown")] = Value::Number(2);
    for include_days in [true, false] {
        let (payment, day) = listed(&Change { ordinal: 3, hour: 144, before: Some(&before), after: &after }, 3, include_days);
        assert!(payment.is_none(), "a credit is not a payment");
        let day = day.expect("a credit whose amount was not recorded is listed, never dropped");
        assert_eq!((day.read, day.unknown_credits, day.credits, day.days, day.payments), (144, 2, 0, 0, 0));
    }
    after[at("payments")] = Value::Number(2);
    let (payment, day) = listed(&Change { ordinal: 3, hour: 144, before: Some(&before), after: &after }, 3, false);
    assert_eq!(payment.map(|payment| payment.unknown), Some(2), "a charged item's unrecorded payments stay payments");
    assert!(day.is_none());
}

#[test]
fn instant_gains_deduplicate_aliases_epochs_caps_and_unknown_operands() {
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
            balance_after_raw: replay_data::rows::RetiredI64(0),
            realized_delta_raw: replay_data::rows::Recorded::Value(after.wrapping_sub(before)),
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

    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let naming = naming::context(&source, &LoadContext { content: None }).unwrap();
    let cp_reply = spending(&query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: FIRST, to: 114 }, &naming, &AtomicBool::new(false)).unwrap();
    let cp_gains: Vec<_> = cp_reply.gains.iter().map(|gain| (gain.read, gain.hour, gain.gained_raw.as_deref(), gain.capped_raw.as_deref())).collect();
    assert_eq!(
        cp_gains,
        [(105, Some(105), Some("1000000"), Some("3000000"))],
        "over-cap CP grant +10 moving 120 -> 100 computes full signed cap loss 30, not 10"
    );
}

#[test]
fn unavailable_or_unread_power_changes_withhold_gain_coverage() {
    use replay_data::{format::FormatVersion, rows::{ClockRow, tables}};
    use replay_journal::synthetic::{Input, Sample};

    let make_frame = |hour: i32| {
        let mut frame = source_frame(hour);
        set(&mut frame, TableId::Clock, TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]));
        if hour == 105 {
            frame.tables[TableId::CountryPowerChanges.index()] = TableView::Unavailable { reason: "changes gap".into() }.into();
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

    let held_frames: Vec<DecodedFrame> = (FIRST..=110)
        .map(|hour| {
            let mut frame = source_frame(hour);
            set(&mut frame, TableId::Clock, TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]));
            set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(Vec::new()));
            set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(Vec::new()));
            frame
        })
        .collect();
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
    let held_src = Source::from_replay(dashboard_base::SourceKey { source_id: "power-gain-cov-held".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(held_replay), &AtomicBool::new(false)).unwrap();
    let held_reply = asked(&held_src, 102, 108);
    assert!(
        held_reply.gain_coverage.iter().any(|run| run.first_hour == 105 && run.last_hour == 105 && !run.covered && run.reason.as_deref().is_some_and(|reason| reason.contains("was not read at this hour"))),
        "held Complete status without fresh read marks hour 105 uncovered in gain_coverage: {:?}",
        held_reply.gain_coverage
    );
}

#[test]
fn command_power_allocation_clamps_and_capped_refunds_record_cap_losses() {
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
                balance_after_raw: replay_data::rows::RetiredI64(0),
                realized_delta_raw: replay_data::rows::Recorded::Value(-20 * PP),
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
                balance_after_raw: replay_data::rows::RetiredI64(0),
                realized_delta_raw: replay_data::rows::Recorded::Value(20 * PP),
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
                balance_after_raw: replay_data::rows::RetiredI64(0),
                realized_delta_raw: replay_data::rows::Recorded::Value(-15 * PP),
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
                balance_after_raw: replay_data::rows::RetiredI64(0),
                realized_delta_raw: replay_data::rows::Recorded::Value(10 * PP),
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
    let reply = spending(&query, &PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Command, from: 102, to: 112 }, &naming, &AtomicBool::new(false)).unwrap();
    assert!(reply.payments.is_empty() && reply.days.is_empty(), "ground-crew holds, allocation clamps, and capped refunds are not spending");
    let gains: Vec<_> = reply.gains.iter().map(|gain| (gain.read, gain.hour, gain.gained_raw.as_deref(), gain.capped_raw.as_deref(), gain.unknown_days, gain.unmatched)).collect();
    assert_eq!(
        gains,
        [
            (108, Some(108), Some("0"), Some("1500000"), 0, 0),
            (110, Some(110), Some("0"), Some("1500000"), 0, 0),
        ],
        "allocation clamp and capped allocation refund record exact cap losses without counting ground-crew holds as gains or spending"
    );
}

#[test]
fn a_payments_item_is_read_at_its_charge_not_at_a_spans_end_whose_items_are_not_recorded() {
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
    let reply = spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap();
    let reads: Vec<_> = reply.payments.iter().map(|payment| (payment.read, payment.known_raw.clone())).collect();
    assert_eq!(reads, [(104, Some("5000000".into())), (120, Some("7500000".into()))], "witnessed spending keeps its amount");
    let item = &reply.items[reply.payments[1].item as usize];
    assert_eq!((item.category.as_str(), item.name.as_deref()), ("decisions", None));
    assert!(item.unknown.as_deref().is_some_and(|why| !why.starts_with("Power items")), "{:?}", item.unknown);
    assert!(!reply.categories.iter().any(|category| category.id == "notIdentified"));
}

#[test]
fn spending_names_a_slot_by_its_holder_at_the_spans_end() {
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
    let reply = spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.tag, "GER", "a slot is named by who held it at the statement's end, not at the branch's last hour");

    let later = Holders::new([("GER".to_owned(), vec![run(110, LAST, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: query.timeline, holders: Arc::new(later) };
    let params = PowerSpendingParams { to: FIRST, ..params };
    let reply = spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.tag, params.country.key(), "a slot held by no one at the statement's end is the slot, never its later holder");

    let earlier = Holders::new([("GER".to_owned(), vec![run(FIRST, 140, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: query.timeline, holders: Arc::new(earlier) };
    let params = PowerSpendingParams { to: 150, ..params };
    let reply = spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.tag, params.country.key(), "a slot held by no one at the statement's end is the slot, never its earlier holder");

    let through = Holders::new([("GER".to_owned(), vec![run(FIRST, LAST, GER)])].into(), Vec::new());
    let query = Query { replay: &replay, timeline: query.timeline, holders: Arc::new(through) };
    let params = PowerSpendingParams { to: LAST + 1, ..params };
    let reply = spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.tag, params.country.key(), "rv5-12b: a statement ending after the branch's last hour is named at its own end, where the slot holds no tag");
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
    assert_eq!(shown, [(104, false, None, 1), (120, true, Some(119), 3)], "an undated payment is placed by its read; several in one read say the interval since the ledger was last read, which can be in the hour of that read");
    whole.payments[1].hour = Some(110);
    let stale = super::markers::index(&whole, Vec::new()).at(LAST);
    assert_eq!((stale.markers[1].hour, stale.markers[1].dated, stale.markers[1].since), (120, false, Some(119)), "the ledger read at 119 without this payment: an own hour of 110 belongs to an earlier count, even with no payment read since 104");

    let mut first = asked(&source, FIRST, LAST);
    first.payments[0].payments = 2;
    first.payments[1].hour = Some(100);
    first.payments[1].unknown = first.payments[1].payments;
    let dated = super::markers::index(&first, Vec::new()).at(LAST);
    let shown: Vec<_> = dated.markers.iter().map(|marker| (marker.hour, marker.dated, marker.since, marker.charge_raw.clone())).collect();
    assert_eq!(shown[0].2, Some(103), "an item's first count is from the hour the ledger was last read, never from the span's start");
    assert!(shown[0].2.unwrap() <= shown[0].0, "the interval holds the latest payment");
    assert_eq!((shown[1].0, shown[1].1), (first.payments[1].read, false), "an hour from before the previous read belongs to an earlier count: the count is dated by its read");
    assert_eq!(shown[1].3, None, "nothing of the charge was recorded: no figure");

    let mut again = asked(&source, FIRST, LAST);
    again.payments[0].read = 119;
    again.payments[0].hour = Some(119);
    again.payments[1].item = again.payments[0].item;
    again.payments[1].hour = again.payments[0].hour;
    again.payments[1].sequence = again.payments[0].sequence.clone();
    again.payments[1].ordinal = again.payments[0].ordinal;
    let older = super::markers::index(&again, Vec::new()).at(LAST);
    assert_eq!((older.markers[1].hour, older.markers[1].dated), (again.payments[1].read, false), "a count whose latest payment is the one already dated gives no date of its own");
    let mut unwitnessed = again.items[again.payments[0].item as usize].clone();
    unwitnessed.id = format!("{}:unrecorded", unwitnessed.id);
    again.items.push(unwitnessed);
    again.payments[1].item = (again.items.len() - 1) as u32;
    let relisted = super::markers::index(&again, Vec::new()).at(LAST);
    assert_eq!((relisted.markers[1].hour, relisted.markers[1].dated), (again.payments[1].read, false), "nor when a read that did not witness its item lists the same ledger item again");

    let gap = super::markers::index(&first, vec![(110, 150, "The ledger was not read".to_owned())]);
    let (inside, later) = (gap.at(115), gap.at(130));
    assert_eq!(inside.unrecorded[0].last_hour, 150, "an open gap is given to its own end; the chart shows it through the displayed hour");
    assert_eq!(inside.unrecorded, later.unrecorded, "playing through the gap and seeking into it give one answer");
}

#[test]
fn naming_power_items_does_not_read_visual_overlays() {
    use dashboard_base::{catalogue, test_fixtures::visual_mods::{BLUE, Machine, RED, png, select}};

    let machine = Machine::new("power_item_art_notes");
    machine.write("common/defines/fixture.lua", b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
    machine.write("common/buildings/fixture.txt", b"buildings={fixture_building={}}");
    machine.write("common/terrain/fixture.txt", b"categories={fixture_terrain={}}");
    machine.write("common/abilities/fixture.txt", b"ability={fixture_ability={icon=GFX_fixture_ability}}");
    machine.write("localisation/english/fixture_l_english.yml", b"\xEF\xBB\xBFl_english:\n fixture_ability:0 \"Fixture Ability\"\n");
    machine.write("interface/fixture.gfx", b"spriteTypes={spriteType={name=GFX_fixture_ability textureFile=gfx/fixture_ability.png}}");
    machine.write("gfx/fixture_ability.png", &png(RED));
    machine.write_mod("fixture_art", "gfx/fixture_ability.png", &png(BLUE));
    let (source, mut options) = machine.open();
    select(&source, &mut options, &["fixture_art"]);
    let load = LoadContext { content: Some(options) };
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let context = naming::context(&source, &load).unwrap();
    let row = PowerItemRow { holder: 7, first_read: 100, item_key: b"fixture_ability".to_vec(), cause: PowerCause::AbilityActivation, ..Default::default() };
    let read = || {
        let (items, noted) = catalogue::capture(|| naming::name(&query, &[naming::Asked { run: 1, row: &row }], &context, &AtomicBool::new(false)).unwrap());
        assert_eq!(items[0].name.as_deref(), Some("Fixture Ability"));
        let mut warnings = Vec::new();
        noted.diagnostics().append_to(&mut warnings);
        warnings
    };
    let first = read();
    assert!(!first.iter().any(|line| line.starts_with("Artwork from visual mods") && line.contains("fixture_art")), "{first:?}");
    assert_eq!(read(), first);
}

#[test]
fn naming_reads_the_country_tables_with_only_the_columns_it_names_by() {
    use replay_data::rows::PowerObjectKind;
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let context = naming::context(&source, &LoadContext { content: None }).unwrap();
    let columns = |row: PowerItemRow| {
        let (_, counts) = replay_recording::cells::count_reads(|| naming::name(&query, &[naming::Asked { run: 1, row: &row }], &context, &AtomicBool::new(false)).unwrap());
        [TableId::Countries, TableId::CountryLinks].map(|table| counts[&table].columns)
    };
    let decision = PowerItemRow { holder: SLOT, first_read: 120, cause: PowerCause::Decision, ..Default::default() };
    assert_eq!(columns(decision.clone()), [2, 1], "a country is named by its slot and tag, and a link by its slot");
    let recording = query.replay.recording();
    let whole = [TableId::Countries, TableId::CountryLinks].map(|table| replay_recording::cells::read_columns(recording, 0, table.index(), None).len() as u64);
    assert!(whole[0] > 2, "the fixture's countries hold more columns than naming reads: {whole:?}");
    let organisation = PowerItemRow { ordinal: 1, object_kind: PowerObjectKind::Organisation, object_reference: Some(1), ..decision };
    assert_eq!(columns(organisation), whole, "an organisation's name reads whole country rows");
}

#[test]
fn naming_does_not_read_the_lexicon_for_an_item_with_its_own_key() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let context = naming::context(&source, &LoadContext { content: None }).unwrap();
    let row = PowerItemRow { holder: SLOT, first_read: 120, item_key: b"fixture_decision".to_vec(), item_token: Some(9), cause: PowerCause::Decision, ..Default::default() };
    let (_, counts) = replay_recording::cells::count_reads(|| {
        naming::name(&query, &[naming::Asked { run: 1, row: &row }], &context, &AtomicBool::new(false)).unwrap()
    });
    assert!(!counts.contains_key(&TableId::LexicalTokens));
}

#[test]
fn spending_breakdown_and_markers_never_walk_country_balance_history() {
    use super::breakdown::{PowerSpendingBreakdown, PowerSpendingBreakdownParams};
    use super::markers::{PowerSpendMarkers, PowerSpendMarkersParams};
    use dashboard_base::products::Product;

    let source = recording();
    let context = LoadContext { content: None };
    let cancel = AtomicBool::new(false);

    let (breakdown, breakdown_counts) = replay_recording::cells::count_reads(|| {
        PowerSpendingBreakdown::load(
            &source,
            &PowerSpendingBreakdownParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from: FIRST, hour: LAST },
            &context,
            &cancel,
        )
        .unwrap()
    });
    assert_eq!(breakdown_counts.get(&TableId::Countries).map_or(0, |held| held.stretches), 0, "breakdown skips Countries balance history");
    assert!(!breakdown.breakdown.parts.is_empty());

    let (markers, marker_counts) = replay_recording::cells::count_reads(|| {
        PowerSpendMarkers::load(
            &source,
            &PowerSpendMarkersParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, hour: LAST },
            &context,
            &cancel,
        )
        .unwrap()
    });
    assert_eq!(marker_counts.get(&TableId::Countries).map_or(0, |held| held.stretches), 0, "markers skip Countries balance history");
    assert_eq!(markers.markers.len(), 2);

    let (direct, direct_counts) = replay_recording::cells::count_reads(|| asked(&source, 105, LAST));
    assert_eq!(direct_counts.get(&TableId::Countries).map_or(0, |held| held.stretches), 0, "spending does not walk Countries balance history");
    assert_eq!(direct.payments.len(), 1);
}

#[test]
fn power_items_end_of_span_read_falls_back_on_internal_gap_and_later_first_read() {
    use dashboard_query::{
        Query,
        identity::{Holder, HolderRun, Holders},
        timeline::{Join, Stretch, Timeline},
    };
    use replay_data::rows::{ClockRow, PowerSpendTotalRow, compatibility::CAPTURE_ADDITIONS_FORMAT, tables};
    use replay_journal::synthetic::{Input, Sample};

    let gap_frames: Vec<DecodedFrame> = (FIRST..=LAST)
        .map(|hour| {
            let mut frame = frame(hour);
            if hour == 110 {
                frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "countries gap".into() }.into();
            }
            frame
        })
        .collect();
    let gap_replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, gap_frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let gap_source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-item-gap".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(gap_replay), &AtomicBool::new(false)).unwrap();
    let held = HolderRun { first: FIRST, last: LAST, holder: Holder::Found { slot: SLOT, id: GER }, starts_segment: true };
    let holders = Holders::new([("GER".to_owned(), vec![held])].into(), Vec::new());
    let timeline = Timeline { id: 1, first: FIRST, last: LAST, stretches: vec![Stretch { run: 0, first: FIRST, last: LAST, join: Join::Start }], unheld: Vec::new() };
    let replay = gap_source.replay();
    let query = Query { replay: &replay, timeline: Arc::new(timeline), holders: Arc::new(holders) };
    let naming = naming::context(&gap_source, &LoadContext { content: None }).unwrap();
    let params = PowerSpendingParams { branch_id: "1".into(), country: "GER".into(), currency: SpendCurrency::Political, from: FIRST, to: LAST };
    let (reply, counts) = replay_recording::cells::count_reads(|| spending(&query, &params, &naming, &AtomicBool::new(false)).unwrap());
    assert!(counts[&TableId::PowerItems].stretches > 0, "an unavailable PowerItems hour inside the asked span falls back to membership");
    assert_eq!(reply.payments.len(), 2);
    assert!(reply.payments.iter().all(|payment| reply.items[payment.item as usize].category == "decisions"));

    let late_frames: Vec<DecodedFrame> = (FIRST..=145)
        .map(|hour| {
            let mut frame = source_frame(hour);
            set(&mut frame, TableId::Clock, TableRows::Clock(vec![ClockRow { hour, ..Default::default() }]));
            frame
        })
        .collect();
    let item = PowerItemRow {
        holder: SLOT,
        ordinal: 0,
        kind: PowerItemRow::CHANGE,
        currency: PowerCurrency::Political,
        cause: PowerCause::Decision,
        charged: true,
        site: PowerSite::AddPoliticalPower,
        first_hour: 144,
        first_read: 144,
        first_country: GER,
        first_epoch: 1,
        first_sequence: 9,
        ..Default::default()
    };
    let total_at = |payments: i64, known: i64, hour: i32, sequence: u64| PowerSpendTotalRow {
        holder: SLOT,
        currency: PowerCurrency::Political as u8,
        ordinal: 0,
        known: Recorded::Value(known),
        payments: Recorded::Value(payments),
        entries: Recorded::Value(payments),
        unknown: Recorded::Value(0),
        uncovered: Recorded::Value(0),
        balances: Recorded::Value(0),
        latest_hour: Recorded::Value(Some(hour)),
        latest_sequence: Recorded::Value(Some(sequence)),
        latest_ordinal: Recorded::Value(Some(0)),
        days: Recorded::Value(0),
        day_payments: Recorded::Value(0),
        day_entries: Recorded::Value(0),
        day_charge: Recorded::Value(0),
        day_unknown: Recorded::Value(0),
        days_uncovered: Recorded::Value(0),
        realized_gained: Recorded::Value(0),
        realized_lost: Recorded::Value(0),
        realized_unknown: Recorded::Value(0),
        credits: Recorded::Value(0),
        gained: Recorded::NotRecorded,
        capped: Recorded::NotRecorded,
        gain_days: Recorded::NotRecorded,
        gain_unknown_days: Recorded::NotRecorded,
        unmatched: Recorded::NotRecorded,
        ..Default::default()
    };
    let bytes = replay::campaign::fixtures::encode_framed_at(recorder_derive::writers_within, CAPTURE_ADDITIONS_FORMAT, CAMPAIGN.into(), &[records(1, late_frames)], |synthetic, decoded| {
        let countries = decoded.complete::<tables::Countries>()?;
        synthetic.table(TableId::Countries, Input::Snapshot { rows: countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        let links = decoded.complete::<tables::CountryLinks>()?;
        synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: links, partial: None, read: true, sample: Sample::FromReadBit })?;
        let coverage = [PowerSpendCoverageRow {
            holder: SLOT,
            political: true,
            command: true,
            reason: PowerSpendCoverageRow::KNOWN,
            political_since: Some(FIRST),
            command_since: Some(FIRST),
            ..Default::default()
        }];
        synthetic.table(TableId::PowerSpendCoverage, Input::Snapshot { rows: &coverage, partial: None, read: true, sample: Sample::FromReadBit })?;
        let items = if decoded.hour >= 144 { std::slice::from_ref(&item) } else { &[] };
        synthetic.table(TableId::PowerItems, Input::Snapshot { rows: items, partial: None, read: true, sample: Sample::FromReadBit })?;
        let totals = match decoded.hour {
            ..=119 => Vec::new(),
            120..=143 => vec![total_at(1, 50 * PP, 120, 7)],
            _ => vec![total_at(2, 125 * PP, 144, 9)],
        };
        synthetic.table(TableId::PowerSpendTotals, Input::Snapshot { rows: &totals, partial: None, read: true, sample: Sample::FromReadBit })?;
        Ok(())
    })
    .unwrap();
    let late_replay = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
    let late_source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-late-item".into(), revision: "1".into() }, "late.battleplan".into(), Arc::new(late_replay), &AtomicBool::new(false)).unwrap();
    let (late_reply, late_counts) = replay_recording::cells::count_reads(|| asked(&late_source, FIRST, 145));
    assert!(late_counts[&TableId::PowerItems].stretches > 0, "an item whose first_read is after an asked hour falls back to membership");
    assert_eq!(late_reply.payments.len(), 2);
    assert_eq!(late_reply.items[late_reply.payments[0].item as usize].category, "notIdentified");
    assert_eq!(late_reply.items[late_reply.payments[1].item as usize].category, "decisions");
}

#[test]
fn damaged_final_block_clock_does_not_abort_complete_items_read() {
    let end = FIRST + 725;
    let second_charge = FIRST + 722;
    let frames: Vec<DecodedFrame> = (FIRST..=end)
        .map(|hour| {
            let mut frame = source_frame(hour);
            let changes = match hour {
                104 => vec![decision(7, 103, 50 * PP)],
                h if h == second_charge => vec![decision(9, second_charge, 75 * PP)],
                _ => Vec::new(),
            };
            set(&mut frame, TableId::CountryPowerChanges, TableRows::CountryPowerChanges(changes));
            set(&mut frame, TableId::CountryPowerDaily, TableRows::CountryPowerDaily(Vec::new()));
            set(&mut frame, TableId::CountryPowerDailyCosts, TableRows::CountryPowerDailyCosts(Vec::new()));
            frame
        })
        .collect();
    let clean = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let head = clean.recording().head(0, 1).unwrap();
    let mut damaged = clean.recording().memory().unwrap().to_vec();
    let at = (head.clock.offset + u64::from(head.clock.stored) / 2) as usize;
    damaged[at] ^= 0x40;
    let replay = Replay::open(damaged).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "power-ledger-damaged-clock".into(), revision: "1".into() }, "damaged.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let (reply, counts) = replay_recording::cells::count_reads(|| asked(&source, FIRST, end));
    let items = &counts[&TableId::PowerItems];
    assert_eq!((items.opened, items.restores), (1, 1), "the multi-block complete span still uses one end-of-span PowerItems restore");
    assert_eq!(items.stretches, 0, "the damaged final-block clock does not force a fallback membership walk");
    assert_eq!(reply.payments.len(), 2);
    assert!(reply.payments.iter().all(|payment| reply.items[payment.item as usize].category == "decisions"));
}
