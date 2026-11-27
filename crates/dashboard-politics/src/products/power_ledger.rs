
pub mod breakdown;
pub mod markers;
mod naming;
#[cfg(test)]
mod tests;

use dashboard_base::products::Country;
use super::politics::{Holding, holdings};
use dashboard_base::products::at_hour;
use dashboard_query::{Query, coverage};
use replay_frame::frame::{DecodedFrame, Unsayable, cells_rows};
use replay_lanes::{lanes::{self, Keys}, points::{self, Cell}};
use replay_columnar::manifest::Status;
use replay_data::rows::{PowerCause, PowerCurrency, PowerItemRow, PowerOperation, PowerSite, PowerSpendCoverageRow, PowerSpendTotalRow, TableId, TableRows, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub enum SpendCurrency {
    Political,
    Command,
}

impl SpendCurrency {
    fn code(self) -> u8 {
        match self {
            Self::Political => 0,
            Self::Command => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PowerSpendingParams {
    pub branch_id: String,
    pub country: Country,
    pub currency: SpendCurrency,
    #[doc = " The span's first and last hour: a payment read in it is listed."]
    pub from: i32,
    pub to: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendItem {
    #[doc = " Unique in the recording: holder, the holder's item ordinal and its first read (ordinals"]
    #[doc = " restart with a run). `power-day` names its parts by it."]
    pub id: String,
    #[doc = " Items shown as one share it: their category, operands and names are equal."]
    pub group_id: String,
    pub cause: String,
    pub category: String,
    #[doc = " Charged every day (daily costs, deficits, upkeep): listed by day, not by payment."]
    pub recurring: bool,
    pub first_hour: i32,
    pub first_read: i32,
    pub name: Option<String>,
    #[doc = " Plain text naming the payment by its cause, item and target."]
    pub action: String,
    pub character: Option<u32>,
    pub target_tag: Option<String>,
    pub target_name: Option<String>,
    #[doc = " Why a name or target is not known."]
    pub unknown: Option<String>,
    #[doc = " A decision's category or an advisor's role, from the installed content; none for other spending."]
    pub kind: Option<SpendKind>,
    #[serde(skip)]
    #[cfg_attr(any(test, feature = "export-types"), ts(skip))]
    pub ledger: (u32, u32, u32),
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendKind {
    pub id: Option<String>,
    pub name: Option<String>,
    #[doc = " Why the category or role is not known; with `id` and `name`, why that name is its key (the"]
    #[doc = " recorded localisation holds none)."]
    pub unknown: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendCategory {
    pub id: String,
    pub name: String,
    #[doc = " False for payments whose cause was not recorded."]
    pub identified: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendPayment {
    #[doc = " Its item's index in `items`."]
    pub item: u32,
    #[doc = " The latest payment's own hour."]
    pub hour: Option<i32>,
    #[doc = " The hour it was read and counted."]
    pub read: i32,
    #[doc = " The latest payment's native order: its sequence, then its cost ordinal."]
    pub sequence: Option<String>,
    pub ordinal: Option<u32>,
    #[doc = " What the payments whose charge was recorded charged, fixed5; none where the sum was not"]
    #[doc = " recorded."]
    pub known_raw: Option<String>,
    pub payments: u32,
    #[doc = " Payments whose charge was not recorded."]
    pub unknown: u32,
    #[doc = " A payment read while the capture did not cover the currency."]
    pub uncovered: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendDay {
    pub item: u32,
    #[doc = " The latest counted day's midnight."]
    pub day: Option<i32>,
    pub read: i32,
    pub days: u32,
    #[doc = " What the days charged, fixed5; none where it was not recorded."]
    pub charge_raw: Option<String>,
    pub payments: u32,
    pub unknown: u32,
    #[doc = " Days read while the capture did not cover the currency."]
    pub uncovered: u32,
    #[doc = " The balance moves the days' payments made, fixed5; none where one was not recorded."]
    pub realized_raw: Option<String>,
    #[doc = " Negative daily costs: a credit, not spending."]
    pub credits: u32,
    #[doc = " Credits whose amount was not recorded."]
    pub unknown_credits: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendGain {
    #[doc = " The event's own hour when distinct from `read`; none for daily totals counted at day close."]
    pub hour: Option<i32>,
    pub read: i32,
    #[doc = " What the period gained before daily costs, fixed5; none where not recorded."]
    pub gained_raw: Option<String>,
    #[doc = " What the cap kept off the period's gains."]
    pub capped_raw: Option<String>,
    pub days: u32,
    #[doc = " Days or instant gains whose amount was not read."]
    pub unknown_days: u32,
    #[doc = " Daily costs whose day's application was not recorded."]
    pub unmatched: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendCoverage {
    pub first_hour: i32,
    pub last_hour: i32,
    pub covered: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerSpendingReply {
    pub branch_id: String,
    pub tag: String,
    pub currency: SpendCurrency,
    pub from: i32,
    pub to: i32,
    pub decimals: u8,
    pub items: Vec<SpendItem>,
    pub categories: Vec<SpendCategory>,
    #[doc = " In read order."]
    pub payments: Vec<SpendPayment>,
    #[doc = " In read order."]
    pub days: Vec<SpendDay>,
    #[doc = " In read order."]
    pub gains: Vec<SpendGain>,
    #[doc = " The span's hours the tag was held, in order; an hour of the span in none was not recorded."]
    pub coverage: Vec<SpendCoverage>,
    #[doc = " The span's hours where instant power changes were read completely."]
    pub gain_coverage: Vec<SpendCoverage>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PowerSpendingReply);

pub(super) fn recurring(row: &PowerItemRow) -> bool {
    row.kind != PowerItemRow::CHANGE || matches!(row.cause, PowerCause::DailyGain | PowerCause::AbilityUpkeep)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Piece {
    pub run: u32,
    pub slot: u32,
    pub run_first: i32,
    pub first: i32,
    pub last: i32,
    pub baseline: Option<i32>,
}

pub(super) fn pieces(query: &Query<'_>, tag: &Country, first: i32, last: i32) -> io::Result<Vec<Piece>> {
    let held = holdings(&query.holders);
    let mut tagged: Vec<&Holding> = held.values().flatten().filter(|holding| dashboard_world::products::holder_tags::matches(tag, (holding.slot, holding.id), &holding.tag)).collect();
    tagged.sort_by_key(|holding| holding.first);
    let mut out = Vec::new();
    for stretch in &query.timeline.stretches {
        if stretch.last < first || stretch.first > last {
            continue;
        }
        let run_first = query.replay.recording().run(stretch.run)?.first_hour;
        for holding in &tagged {
            let (from, to) = (holding.first.max(stretch.first).max(first), holding.last.min(stretch.last).min(last));
            if from > to {
                continue;
            }
            let baseline = if from > stretch.first {
                Some(from - 1)
            } else if stretch.first == run_first {
                None
            } else {
                Some(stretch.first)
            };
            out.push(Piece { run: stretch.run, slot: holding.slot, run_first, first: from, last: to, baseline });
        }
    }
    out.sort_by_key(|piece| piece.first);
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Value {
    Number(i64),
    None,
    Unrecorded,
}

impl Value {
    fn of(cell: Cell) -> Self {
        match cell {
            Cell::Number(value) => Self::Number(value),
            Cell::Missing => Self::None,
            Cell::Unrecorded | Cell::Text(_) | Cell::Written(_) => Self::Unrecorded,
        }
    }

    fn number(self) -> Option<i64> {
        match self {
            Self::Number(value) => Some(value),
            _ => None,
        }
    }
}

const TOTALS: [&str; 22] = [
    "known",
    "payments",
    "unknown",
    "uncovered",
    "latest_hour",
    "latest_sequence",
    "latest_ordinal",
    "days",
    "day_payments",
    "day_charge",
    "day_unknown",
    "days_uncovered",
    "realized_gained",
    "realized_lost",
    "realized_unknown",
    "last_day",
    "credits",
    "gained",
    "capped",
    "gain_days",
    "gain_unknown_days",
    "unmatched",
];

const DAYS: [&str; 11] = [
    "ordinal",
    "day",
    "days",
    "payments",
    "charge",
    "unknown",
    "uncovered",
    "realized_gained",
    "realized_lost",
    "realized_unknown",
    "credits",
];

type Cells = [Value; TOTALS.len()];

pub(super) struct Change<'a> {
    pub ordinal: u32,
    pub hour: i32,
    pub before: Option<&'a Cells>,
    pub after: &'a Cells,
}

fn position(name: &str) -> usize {
    TOTALS.iter().position(|held| *held == name).expect("a totals column the ledger reads")
}

fn delta(change: &Change<'_>, name: &str) -> Option<i64> {
    let at = position(name);
    let after = change.after[at];
    let before = change.before.map_or(Value::Number(0), |before| before[at]);
    match (before, after) {
        (Value::Number(before), Value::Number(after)) => after.checked_sub(before),
        (Value::None, Value::Number(after)) => Some(after),
        _ => None,
    }
}

fn count(change: &Change<'_>, name: &str) -> u32 {
    delta(change, name).map_or(0, |delta| u32::try_from(delta.max(0)).unwrap_or(u32::MAX))
}

fn raw(value: Option<i64>) -> Option<String> {
    value.map(|value| value.to_string())
}

pub(super) fn listed(change: &Change<'_>, item: u32, include_days: bool) -> (Option<SpendPayment>, Option<SpendDay>) {
    let after = |name: &str| change.after[position(name)];
    let payment = (count(change, "payments") > 0).then(|| {
        let unknown = count(change, "unknown");
        SpendPayment {
            item,
            hour: after("latest_hour").number().map(|hour| hour as i32),
            read: change.hour,
            sequence: after("latest_sequence").number().map(|sequence| (sequence as u64).to_string()),
            ordinal: after("latest_ordinal").number().map(|ordinal| ordinal as u32),
            known_raw: raw(delta(change, "known")),
            payments: count(change, "payments"),
            unknown,
            uncovered: count(change, "uncovered") > 0,
        }
    });
    let unknown_credits = if count(change, "payments") == 0 { count(change, "unknown") } else { 0 };
    let days = count(change, "days");
    let day_payments = count(change, "day_payments");
    let day = if include_days && (days > 0 || day_payments > 0 || count(change, "credits") > 0 || unknown_credits > 0) {
        Some(SpendDay {
            item,
            day: after("last_day").number().map(|day| day as i32),
            read: change.hour,
            days,
            charge_raw: raw(delta(change, "day_charge")),
            payments: day_payments,
            unknown: count(change, "day_unknown"),
            uncovered: count(change, "days_uncovered"),
            realized_raw: (count(change, "realized_unknown") == 0)
                .then(|| delta(change, "realized_gained").zip(delta(change, "realized_lost")).map(|(gained, lost)| gained.saturating_add(lost)))
                .flatten()
                .map(|value| value.to_string()),
            credits: count(change, "credits"),
            unknown_credits,
        })
    } else {
        (unknown_credits > 0).then(|| SpendDay {
            item,
            day: None,
            read: change.hour,
            days: 0,
            charge_raw: Some("0".into()),
            payments: 0,
            unknown: 0,
            uncovered: 0,
            realized_raw: None,
            credits: 0,
            unknown_credits,
        })
    };
    (payment, day)
}

pub(super) fn gained(change: &Change<'_>) -> Option<SpendGain> {
    let gain = SpendGain {
        hour: None,
        read: change.hour,
        gained_raw: raw(delta(change, "gained")),
        capped_raw: raw(delta(change, "capped")),
        days: count(change, "gain_days"),
        unknown_days: count(change, "gain_unknown_days"),
        unmatched: count(change, "unmatched"),
    };
    let moved = |raw: &Option<String>| raw.as_deref() != Some("0");
    (gain.days > 0 || gain.unknown_days > 0 || gain.unmatched > 0 || moved(&gain.gained_raw) || moved(&gain.capped_raw)).then_some(gain)
}

fn changes(query: &Query<'_>, piece: &Piece, low: u128, high: u128, has_day_log: bool, each: &mut dyn FnMut(Change<'_>)) -> io::Result<()> {
    let recording = query.replay.recording();
    let table = TableId::PowerSpendTotals;
    let all: Vec<usize> = TOTALS.iter().map(|name| dashboard_declare::field(table, name)).collect::<io::Result<_>>()?;
    let recorded = recording.columns()[table.index()];
    let day_log = has_day_log && all.iter().all(|&column| column < recorded && !table.column_withheld_at(column, recording.format()));
    let day = position("days")..=position("credits");
    let slots: Vec<_> = (0..TOTALS.len()).filter(|at| !day_log || !day.contains(at)).collect();
    let columns: Vec<_> = slots.iter().map(|&at| all[at]).collect();
    let from = piece.baseline.unwrap_or(piece.first);
    let order = column_order(&columns);
    let mut previous = HashMap::<u128, Cells>::new();
    let (read, _) = dashboard_query::cost::storage(|| {
        points::cells(recording, piece.run, table, &columns, Keys::Range(low, high), from, piece.last, None, &mut |hour, key, cells| {
            let Some(cells) = cells.filter(|cells| cells.len() == order.len()) else { return true };
            let mut after = [Value::Unrecorded; TOTALS.len()];
            for (&slot, &at) in slots.iter().zip(&order) {
                after[slot] = Value::of(cells[at]);
            }
            let before = previous.insert(key, after);
            if piece.baseline != Some(hour) && before.as_ref() != Some(&after) {
                each(Change { ordinal: key as u32, hour, before: before.as_ref(), after: &after });
            }
            true
        })
    })?;
    if read.len() != columns.len() {
        return Err(dashboard_core::invalid("A totals column was not read"));
    }
    Ok(())
}

fn column_order(columns: &[usize]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..columns.len()).collect();
    order.sort_unstable_by_key(|&at| columns[at]);
    let mut places = vec![0; columns.len()];
    for (projected, original) in order.into_iter().enumerate() {
        places[original] = projected;
    }
    places
}

fn day_log(query: &Query<'_>, piece: &Piece, currency: SpendCurrency, each: &mut dyn FnMut(i32, u32, SpendDay)) -> io::Result<Vec<lanes::LogGap>> {
    if piece.first > piece.last {
        return Ok(Vec::new());
    }
    let columns: Vec<usize> = DAYS.iter().map(|name| dashboard_declare::field(TableId::PowerSpendDays, name)).collect::<io::Result<_>>()?;
    let low = u128::from(piece.slot) << 72;
    let high = low | ((1_u128 << 72) - 1);
    let wanted_currency = currency.code();
    let span = dashboard_query::cost::storage(|| {
        lanes::log_span(query.replay.recording(), piece.run, TableId::PowerSpendDays, &columns, Keys::Range(low, high), piece.first, piece.last, &mut |hour, key, cells| {
            if (key >> 32) as u8 != wanted_currency {
                return;
            }
            let value = |at: usize| Value::of(cells[at]);
            let Some(ordinal) = value(0).number().and_then(|value| u32::try_from(value).ok()) else { return };
            let Some(day_midnight) = value(1).number().map(|value| value as i32) else { return };
            let count_u32 = |at: usize| value(at).number().map_or(0, |value| u32::try_from(value.max(0)).unwrap_or(u32::MAX));
            let days = count_u32(2);
            let payments = count_u32(3);
            let credits = count_u32(10);
            if days == 0 && payments == 0 && credits == 0 {
                return;
            }
            let realized_unknown = count_u32(9);
            each(
                hour,
                ordinal,
                SpendDay {
                    item: 0,
                    day: Some(day_midnight),
                    read: hour,
                    days,
                    charge_raw: raw(value(4).number()),
                    payments,
                    unknown: count_u32(5),
                    uncovered: count_u32(6),
                    realized_raw: (realized_unknown == 0)
                        .then(|| value(7).number().zip(value(8).number()).map(|(gained, lost)| i128::from(gained) + i128::from(lost)))
                        .flatten()
                        .map(|value| value.to_string()),
                    credits,
                    unknown_credits: 0,
                },
            );
        })
    })?;
    Ok(span.gaps)
}

fn day_gap_reason(gap: &lanes::LogGap) -> String {
    match gap.status {
        Status::Unavailable { .. } => match gap.reason.as_deref().filter(|reason| !reason.is_empty()) {
            Some(reason) => format!("Daily spending log unavailable: {reason}"),
            None => "Daily spending log unavailable".into(),
        },
        Status::NotRecorded => "Daily spending log was not recorded".into(),
        Status::Truncated { missing } => format!("Daily spending log was read in part: {missing} missing"),
        Status::Complete => String::new(),
    }
}

fn push_coverage(out: &mut Vec<SpendCoverage>, run: SpendCoverage) {
    match out.last_mut() {
        Some(last) if last.last_hour.checked_add(1) == Some(run.first_hour) && last.covered == run.covered && last.reason == run.reason => {
            last.last_hour = run.last_hour;
        }
        _ => out.push(run),
    }
}

fn combine_gap_reason(covered: bool, existing: Option<&str>, day_reason: &str) -> String {
    if covered {
        return day_reason.to_owned();
    }
    match existing.filter(|reason| !reason.is_empty()) {
        None => day_reason.to_owned(),
        Some(existing) if existing.contains(day_reason) => existing.to_owned(),
        Some(existing) => format!("{existing}; {day_reason}"),
    }
}

fn apply_day_gaps(coverage: &mut Vec<SpendCoverage>, start_at: usize, gaps: &[lanes::LogGap], gap_hours: &mut BTreeMap<String, u64>) {
    if gaps.is_empty() {
        return;
    }
    let mut resolved = Vec::with_capacity(gaps.len());
    for gap in gaps {
        let reason = day_gap_reason(gap);
        if reason.is_empty() {
            continue;
        }
        *gap_hours.entry(reason.clone()).or_default() += u64::try_from(i64::from(gap.last) - i64::from(gap.first) + 1).unwrap_or(0);
        resolved.push((gap.first, gap.last, reason));
    }
    if resolved.is_empty() {
        return;
    }
    let mut next: Vec<SpendCoverage> = Vec::with_capacity(coverage.len().saturating_sub(start_at) + resolved.len());
    let mut gap_at = 0usize;
    for run in coverage.drain(start_at..) {
        let mut cursor = run.first_hour;
        let mut done = false;
        while gap_at < resolved.len() && resolved[gap_at].1 < cursor {
            gap_at += 1;
        }
        while gap_at < resolved.len() && resolved[gap_at].0 <= run.last_hour {
            let (first, last, ref day_reason) = resolved[gap_at];
            let a = first.max(cursor);
            let b = last.min(run.last_hour);
            if cursor < a {
                push_coverage(&mut next, SpendCoverage { first_hour: cursor, last_hour: a - 1, covered: run.covered, reason: run.reason.clone() });
            }
            let reason = combine_gap_reason(run.covered, run.reason.as_deref(), day_reason);
            push_coverage(&mut next, SpendCoverage { first_hour: a, last_hour: b, covered: false, reason: Some(reason) });
            if last <= run.last_hour {
                gap_at += 1;
            }
            if b >= run.last_hour {
                done = true;
                break;
            }
            cursor = b + 1;
        }
        if !done && cursor <= run.last_hour {
            push_coverage(&mut next, SpendCoverage { first_hour: cursor, last_hour: run.last_hour, covered: run.covered, reason: run.reason });
        }
    }
    coverage.extend(next);
}

fn coverage(query: &Query<'_>, piece: &Piece, currency: SpendCurrency, out: &mut Vec<SpendCoverage>) -> io::Result<()> {
    let names = ["political", "command", "reason", "input_reason"];
    let columns: Vec<usize> = names.iter().map(|name| dashboard_declare::field(TableId::PowerSpendCoverage, name)).collect::<io::Result<_>>()?;
    let keys = [u128::from(piece.slot)];
    let history = dashboard_query::cost::storage(|| lanes::history(query.replay.recording(), piece.run, TableId::PowerSpendCoverage, &columns, Keys::These(&keys), piece.first, piece.last))?;
    let at = |column: usize| history.columns.binary_search(&column).map_err(|_| dashboard_core::invalid("A coverage column was not read"));
    let (political, command, reason, input) = (at(columns[0])?, at(columns[1])?, at(columns[2])?, at(columns[3])?);
    let steps = history.keys.iter().find(|(key, _)| *key == keys[0]).map(|(_, steps)| steps.as_slice()).unwrap_or_default();
    let mut runs: Vec<(i32, bool, Option<String>)> = Vec::new();
    if steps.first().is_none_or(|step| step.hour > piece.first) {
        runs.push((piece.first, false, Some("Spending coverage was not recorded".into())));
    }
    for step in steps {
        let (covered, why) = match &step.cells {
            None => (false, Some("Spending coverage was not recorded".to_owned())),
            Some(cells) => {
                let covered = matches!(cells[if currency == SpendCurrency::Political { political } else { command }], Cell::Number(1));
                let code = match cells[reason] {
                    Cell::Number(code) => usize::try_from(code).ok(),
                    _ => None,
                };
                let input = history.text(cells[input]).filter(|text| !text.is_empty()).map(|text| String::from_utf8_lossy(text).into_owned());
                let why = (!covered).then(|| {
                    let named = code.and_then(|code| PowerSpendCoverageRow::REASONS.get(code)).filter(|text| !text.is_empty()).map(|text| (*text).to_owned());
                    match (named, input) {
                        (Some(named), Some(input)) => format!("{named} ({input})"),
                        (Some(named), None) => named,
                        (None, Some(input)) => input,
                        (None, None) => "Spending was not recorded whole".into(),
                    }
                });
                (covered, why)
            }
        };
        match runs.last() {
            Some((_, held, reason)) if *held == covered && *reason == why => {}
            _ => runs.push((step.hour, covered, why)),
        }
    }
    for (at, (first, covered, reason)) in runs.iter().enumerate() {
        let last = runs.get(at + 1).map_or(piece.last, |next| next.0 - 1);
        out.push(SpendCoverage { first_hour: *first, last_hour: last, covered: *covered, reason: reason.clone() });
    }
    Ok(())
}

struct GainBucket {
    gained: Option<i128>,
    capped: Option<i128>,
    days: u32,
    unknown_days: u32,
    unmatched: u32,
}

impl Default for GainBucket {
    fn default() -> Self {
        Self { gained: Some(0), capped: Some(0), days: 0, unknown_days: 0, unmatched: 0 }
    }
}

impl GainBucket {
    fn add_amount(target: &mut Option<i128>, value: Option<i128>) {
        *target = match (*target, value) {
            (Some(held), Some(add)) => held.checked_add(add),
            _ => None,
        };
    }

    fn push(&mut self, gained: Option<i128>, capped: Option<i128>, days: u32, unknown_days: u32, unmatched: u32) {
        Self::add_amount(&mut self.gained, gained);
        Self::add_amount(&mut self.capped, capped);
        self.days = self.days.saturating_add(days);
        self.unknown_days = self.unknown_days.saturating_add(unknown_days);
        self.unmatched = self.unmatched.saturating_add(unmatched);
    }
}

const CHANGE_COLS: [&str; 9] = [
    "epoch",
    "hour",
    "currency",
    "operation",
    "balance_before_raw",
    "balance_after_raw",
    "requested_delta_raw",
    "cause",
    "site",
];

const CHANGE_COLS_DELTA: [&str; 9] = [
    "epoch",
    "hour",
    "currency",
    "operation",
    "balance_before_raw",
    "realized_delta_raw",
    "requested_delta_raw",
    "cause",
    "site",
];

fn is_hold(currency: PowerCurrency, site: PowerSite, cause: PowerCause) -> bool {
    currency == PowerCurrency::Command
        && (matches!(site, PowerSite::AllocateCommandPower | PowerSite::ReleaseCommandPower | PowerSite::GroundCrew) || cause == PowerCause::AllocationRefund)
}

fn status_reason(status: Status, label: &str) -> Option<String> {
    match status {
        Status::Complete => None,
        Status::Unavailable { .. } => Some(format!("{label} unavailable")),
        Status::NotRecorded => Some(format!("{label} was not recorded")),
        Status::Truncated { missing } => Some(format!("{label} was read in part: {missing} missing")),
    }
}

fn push_status_run(out: &mut Vec<(i32, i32, Option<String>)>, a: i32, b: i32, reason: Option<String>) {
    if a > b {
        return;
    }
    match out.last_mut() {
        Some((_, end, held)) if end.checked_add(1) == Some(a) && *held == reason => *end = b,
        _ => out.push((a, b, reason)),
    }
}

fn status_runs(query: &Query<'_>, run: u32, table: TableId, first: i32, last: i32, label: &str) -> io::Result<Vec<(i32, i32, Option<String>)>> {
    if first > last {
        return Ok(Vec::new());
    }
    let recording = query.replay.recording();
    if recording.predates(table.index()).is_some() {
        return Ok(vec![(first, last, Some(format!("{label} was not recorded in this format")))]);
    }
    let runs = dashboard_query::cost::storage(|| lanes::statuses(recording, run, table, first, last))?;
    let reads = dashboard_query::cost::storage(|| lanes::reads(recording, run, table, first, last))?;
    let mut out: Vec<(i32, i32, Option<String>)> = Vec::new();
    let mut read_at = 0usize;
    let mut cursor = first;
    for run in runs {
        let (a, b) = (run.first.max(first), run.last.min(last));
        if a > b {
            continue;
        }
        if cursor < a {
            push_status_run(&mut out, cursor, a - 1, Some(format!("{label} was not recorded")));
        }
        match status_reason(run.status, label) {
            None => {
                let mut complete_cursor = a;
                while read_at < reads.len() && reads[read_at].1 < complete_cursor {
                    read_at += 1;
                }
                while read_at < reads.len() && reads[read_at].0 <= b {
                    let (ra, rb) = reads[read_at];
                    let hit_a = ra.max(complete_cursor);
                    let hit_b = rb.min(b);
                    if complete_cursor < hit_a {
                        push_status_run(&mut out, complete_cursor, hit_a - 1, Some(format!("{label} was not read at this hour")));
                    }
                    push_status_run(&mut out, hit_a, hit_b, None);
                    complete_cursor = hit_b.saturating_add(1);
                    if rb <= b {
                        read_at += 1;
                    } else {
                        break;
                    }
                }
                if complete_cursor <= b {
                    push_status_run(&mut out, complete_cursor, b, Some(format!("{label} was not read at this hour")));
                }
            }
            Some(reason) => push_status_run(&mut out, a, b, Some(reason)),
        }
        cursor = b.saturating_add(1);
    }
    if cursor <= last {
        push_status_run(&mut out, cursor, last, Some(format!("{label} was not recorded")));
    }
    Ok(out)
}

fn intersect_status_runs(a: &[(i32, i32, Option<String>)], b: &[(i32, i32, Option<String>)]) -> Vec<(i32, i32, Option<String>)> {
    let mut out: Vec<(i32, i32, Option<String>)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        let (aa, ab, ref ar) = a[i];
        let (ba, bb, ref br) = b[j];
        let lo = aa.max(ba);
        let hi = ab.min(bb);
        if lo <= hi {
            let reason = ar.clone().or_else(|| br.clone());
            match out.last_mut() {
                Some((_, end, held)) if end.checked_add(1) == Some(lo) && *held == reason => *end = hi,
                _ => out.push((lo, hi, reason)),
            }
        }
        if ab < bb {
            i += 1;
        } else if bb < ab {
            j += 1;
        } else {
            i += 1;
            j += 1;
        }
    }
    out
}

fn maps_to_slot(steps: &[(i32, Option<u32>)], hour: i32, slot: u32) -> bool {
    let at = steps.partition_point(|&(step_hour, _)| step_hour <= hour);
    at > 0 && steps[at - 1].1 == Some(slot)
}

fn instant_gains(
    query: &Query<'_>,
    piece: &Piece,
    currency: SpendCurrency,
    gain_coverage: &mut Vec<SpendCoverage>,
    buckets: &mut BTreeMap<(i32, Option<i32>), GainBucket>,
) -> io::Result<()> {
    let change_statuses = status_runs(query, piece.run, TableId::CountryPowerChanges, piece.first, piece.last, "Power changes")?;
    let link_statuses = status_runs(query, piece.run, TableId::CountryLinks, piece.first, piece.last, "Country links")?;
    for (first_hour, last_hour, reason) in intersect_status_runs(&change_statuses, &link_statuses) {
        push_coverage(gain_coverage, SpendCoverage { first_hour, last_hour, covered: reason.is_none(), reason });
    }
    let recording = query.replay.recording();
    if recording.predates(TableId::CountryPowerChanges.index()).is_some() {
        return Ok(());
    }
    let start = piece.baseline.unwrap_or(piece.first);
    let link_col = dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?;
    let link_history = dashboard_query::cost::storage(|| lanes::history(recording, piece.run, TableId::CountryLinks, &[link_col], Keys::All, start, piece.last))?;
    let link_at = link_history.columns.binary_search(&link_col).map_err(|_| dashboard_core::invalid("CountryLinks canonical_slot column was not read"))?;
    let mut links_by_raw: HashMap<u32, Vec<(i32, Option<u32>)>> = HashMap::new();
    for (key, steps) in &link_history.keys {
        let slot_steps: Vec<(i32, Option<u32>)> = steps
            .iter()
            .map(|step| (step.hour, step.cells.as_ref().and_then(|cells| Value::of(cells[link_at]).number()).map(|value| value as u32)))
            .collect();
        let active_for_slot = slot_steps.iter().enumerate().any(|(at, &(hour, slot))| {
            slot == Some(piece.slot) && hour <= piece.last && slot_steps.get(at + 1).map_or(piece.last, |&(next, _)| next - 1) >= piece.first
        });
        if active_for_slot {
            links_by_raw.insert(*key as u32, slot_steps);
        }
    }
    let mut raws: Vec<u32> = links_by_raw.keys().copied().collect();
    raws.sort_unstable();
    let ranges: Vec<(u128, u128)> = raws
        .iter()
        .map(|&raw| {
            let low = u128::from(raw) << 64;
            (low, low | u128::from(u64::MAX))
        })
        .collect();
    if ranges.is_empty() {
        return Ok(());
    }
    let uses_delta = recording.format() >= replay_data::rows::compatibility::RECORDING_SIZE_FORMAT;
    let col_names = if uses_delta { &CHANGE_COLS_DELTA } else { &CHANGE_COLS };
    let columns: Vec<usize> = col_names.iter().map(|name| dashboard_declare::field(TableId::CountryPowerChanges, name)).collect::<io::Result<_>>()?;
    let order = column_order(&columns);
    let wanted_currency = match currency {
        SpendCurrency::Political => PowerCurrency::Political,
        SpendCurrency::Command => PowerCurrency::Command,
    };
    let mut seen_epochs: HashMap<u128, Vec<u64>> = HashMap::new();
    let (read, _) = dashboard_query::cost::storage(|| {
        points::cells(recording, piece.run, TableId::CountryPowerChanges, &columns, Keys::Ranges(&ranges), start, piece.last, None, &mut |hour, key, cells| {
            let Some(cells) = cells.filter(|cells| cells.len() == order.len()) else { return true };
            let epoch = Value::of(cells[order[0]]).number().map(|value| value as u64);
            if let Some(ep) = epoch {
                let epochs = seen_epochs.entry(key).or_default();
                if epochs.contains(&ep) {
                    return true;
                }
                epochs.push(ep);
            }
            if piece.baseline == Some(hour) || hour < piece.first || hour <= piece.run_first {
                return true;
            }
            let raw = (key >> 64) as u32;
            if !links_by_raw.get(&raw).is_some_and(|links| maps_to_slot(links, hour, piece.slot)) {
                return true;
            }
            let Some(row_currency) = Value::of(cells[order[2]]).number().and_then(|value| u8::try_from(value).ok()).and_then(|value| PowerCurrency::try_from(value).ok()) else {
                return true;
            };
            if row_currency != wanted_currency {
                return true;
            }
            let Some(operation) = Value::of(cells[order[3]]).number().and_then(|value| u8::try_from(value).ok()).and_then(|value| PowerOperation::try_from(value).ok()) else {
                return true;
            };
            let Some(cause) = Value::of(cells[order[7]]).number().and_then(|value| u16::try_from(value).ok()).and_then(|value| PowerCause::try_from(value).ok()) else {
                return true;
            };
            let Some(site) = Value::of(cells[order[8]]).number().and_then(|value| u16::try_from(value).ok()).and_then(|value| PowerSite::try_from(value).ok()) else {
                return true;
            };
            let event_hour = Value::of(cells[order[1]]).number().map(|value| value as i32);
            let before = Value::of(cells[order[4]]).number();
            let after = Value::of(cells[order[5]]).number().and_then(|value| {
                if uses_delta {
                    before.map(|before| before.wrapping_add(value))
                } else {
                    Some(value)
                }
            });
            let requested = Value::of(cells[order[6]]).number();
            if is_hold(row_currency, site, cause) {
                match operation {
                    PowerOperation::AllocationClamp => match before.zip(after) {
                        Some((b, a)) => {
                            let delta = i128::from(a) - i128::from(b);
                            if delta < 0 {
                                buckets.entry((hour, event_hour)).or_default().push(Some(0), Some(-delta), 0, 0, 0);
                            }
                        }
                        None => {
                            buckets.entry((hour, event_hour)).or_default().push(Some(0), None, 0, 1, 0);
                        }
                    },
                    PowerOperation::Refund => {
                        if let Some((b, a)) = before.zip(after) {
                            let delta = i128::from(a) - i128::from(b);
                            if let Some(req) = requested.map(i128::from).filter(|&req| req > delta && delta >= 0) {
                                buckets.entry((hour, event_hour)).or_default().push(Some(0), Some(req - delta), 0, 0, 0);
                            }
                        }
                    }
                    _ => {}
                }
                return true;
            }
            if matches!(cause, PowerCause::DailyGain | PowerCause::CapCorrection | PowerCause::ResetSetup | PowerCause::Console) {
                return true;
            }
            let entry = match operation {
                PowerOperation::Grant | PowerOperation::Refund | PowerOperation::Add => match requested {
                    Some(req) if req > 0 => {
                        let req = i128::from(req);
                        let capped = before.zip(after).map(|(b, a)| {
                            let realized = i128::from(a) - i128::from(b);
                            if realized < req { req - realized } else { 0 }
                        });
                        Some((Some(req), capped, u32::from(capped.is_none())))
                    }
                    Some(_) => None,
                    None => Some((None, None, 1)),
                },
                PowerOperation::Set => match requested {
                    Some(req) if req > 0 => {
                        let req = i128::from(req);
                        let capped = before.zip(after).map(|(b, a)| {
                            let realized = i128::from(a) - i128::from(b);
                            if realized < req { req - realized } else { 0 }
                        });
                        Some((Some(req), capped, u32::from(capped.is_none())))
                    }
                    Some(_) => None,
                    None if before.zip(after).is_some_and(|(b, a)| a <= b) => None,
                    None => Some((None, None, 1)),
                },
                PowerOperation::Subtract | PowerOperation::Reset | PowerOperation::AllocationClamp | PowerOperation::Clamp => None,
            };
            if let Some((gained, capped, unknown)) = entry {
                buckets.entry((hour, event_hour)).or_default().push(gained, capped, 0, unknown, 0);
            }
            true
        })
    })?;
    if read.len() != columns.len() {
        return Err(dashboard_core::invalid("A power change column was not read"));
    }
    Ok(())
}

fn item_key(slot: u32, ordinal: u32) -> u128 {
    PowerItemRow { holder: slot, ordinal, ..PowerItemRow::default() }.key()
}

struct ItemAsks<'a> {
    piece: &'a Piece,
    payments: std::ops::Range<usize>,
    days: std::ops::Range<usize>,
    asks: BTreeMap<(i32, u32), bool>,
}

fn complete_span(statuses: &[lanes::StatusRun], first: i32, last: i32) -> bool {
    let mut cursor = first;
    for run in statuses {
        if run.last < cursor {
            continue;
        }
        if run.first > cursor || run.status != Status::Complete {
            return false;
        }
        if run.last >= last {
            return true;
        }
        cursor = match run.last.checked_add(1) {
            Some(next) => next,
            None => return true,
        };
    }
    false
}

fn complete_items(query: &Query<'_>, batch: &ItemAsks<'_>) -> io::Result<Option<HashMap<u32, PowerItemRow>>> {
    let Some((&(first, _), _)) = batch.asks.first_key_value() else { return Ok(None) };
    let (&(last, _), _) = batch.asks.last_key_value().expect("a nonempty batch has a last ask");
    let piece = batch.piece;
    let recording = query.replay.recording();
    if recording.predates(TableId::PowerItems.index()).is_some() {
        return Ok(None);
    }
    let statuses = match dashboard_query::cost::storage(|| lanes::statuses(recording, piece.run, TableId::PowerItems, first, last)) {
        Ok(runs) => runs,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(None),
        Err(error) => return Err(error),
    };
    if !complete_span(&statuses, first, last) {
        return Ok(None);
    }
    let mut first_ask: HashMap<u32, i32> = HashMap::new();
    for (&(hour, ordinal), _) in &batch.asks {
        first_ask.entry(ordinal).or_insert(hour);
    }
    let mut keys: Vec<u128> = first_ask.keys().map(|&ordinal| item_key(piece.slot, ordinal)).collect();
    keys.sort_unstable();
    let snapshot = match dashboard_query::cost::storage(|| lanes::at_keys(recording, piece.run, TableId::PowerItems, last, None, Keys::These(&keys))) {
        Ok(snapshot) => snapshot,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(None),
        Err(error) => return Err(error),
    };
    if snapshot.status != Status::Complete {
        return Ok(None);
    }
    let decoded = match cells_rows(TableId::PowerItems, &snapshot.view(), &snapshot.rows, recording.format()) {
        Ok(rows) => rows,
        Err(error) if error.kind() == io::ErrorKind::InvalidData || Unsayable::of(&error).is_some() => return Ok(None),
        Err(error) => return Err(error),
    };
    let TableRows::PowerItems(rows) = decoded else { return Ok(None) };
    let mut by_ordinal = HashMap::with_capacity(rows.len());
    for row in rows {
        if row.holder == piece.slot {
            by_ordinal.insert(row.ordinal, row);
        }
    }
    if first_ask.iter().any(|(&ordinal, &hour)| !by_ordinal.get(&ordinal).is_some_and(|row| row.first_read <= hour)) {
        return Ok(None);
    }
    Ok(Some(by_ordinal))
}

fn item_membership(query: &Query<'_>, batches: &[&ItemAsks<'_>]) -> io::Result<HashMap<u32, lanes::History>> {
    let mut runs = BTreeMap::<u32, (i32, i32, Vec<u128>)>::new();
    for batch in batches {
        let piece = batch.piece;
        let Some((&(first, _), _)) = batch.asks.first_key_value() else { continue };
        let (&(last, _), _) = batch.asks.last_key_value().expect("a nonempty batch has a last ask");
        let held = runs.entry(piece.run).or_insert_with(|| (first, last, Vec::new()));
        held.0 = held.0.min(first);
        held.1 = held.1.max(last);
        held.2.extend(batch.asks.keys().map(|&(_, ordinal)| item_key(piece.slot, ordinal)));
    }
    let mut members = HashMap::with_capacity(runs.len());
    for (run, (first, last, mut keys)) in runs {
        keys.sort_unstable();
        keys.dedup();
        let read = dashboard_query::cost::storage(|| {
            lanes::history(query.replay.recording(), run, TableId::PowerItems, &[], Keys::These(&keys), first, last)
        });
        match read {
            Ok(history) => { members.insert(run, history); }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {}
            Err(error) => return Err(error),
        }
    }
    Ok(members)
}

const UNRECORDED_ITEM: u32 = 1 << 31;
const ITEM_ABSENT: &str = "Its item was not among the power items read when it was charged";

struct UnrecordedItem {
    run: u32,
    slot: u32,
    ordinal: u32,
    read: i32,
    recurring: bool,
    reason: String,
}

#[derive(Default)]
struct Items {
    asked: Vec<(u32, PowerItemRow)>,
    index: HashMap<(u32, u32, u32), u32>,
    unrecorded: Vec<UnrecordedItem>,
    unrecorded_index: HashMap<(u32, u32, u32, String), u32>,
}

impl Items {
    fn read_complete(&mut self, batch: &ItemAsks<'_>, mut rows: HashMap<u32, PowerItemRow>) -> HashMap<(u32, i32), (u32, bool)> {
        let mut out = HashMap::with_capacity(batch.asks.len());
        let piece = batch.piece;
        for (&(hour, ordinal), _) in &batch.asks {
            let at = *self.index.entry((piece.run, piece.slot, ordinal)).or_insert_with(|| {
                let row = rows.remove(&ordinal).expect("complete_items validated every asked item");
                self.asked.push((piece.run, row));
                (self.asked.len() - 1) as u32
            });
            out.insert((ordinal, hour), (at, true));
        }
        out
    }

    fn read(&mut self, frames: &mut at_hour::Frames<'_>, batch: &ItemAsks<'_>, membership: Option<&lanes::History>) -> io::Result<HashMap<(u32, i32), (u32, bool)>> {
        let mut out = HashMap::with_capacity(batch.asks.len());
        let piece = batch.piece;
        let asks: Vec<_> = batch.asks.iter().collect();
        for at_hour in asks.chunk_by(|(left, _), (right, _)| left.0 == right.0) {
            let (&(hour, _), _) = at_hour[0];
            let membership = membership.filter(|history| {
                history.statuses.get(history.statuses.partition_point(|span| span.last < hour)).is_some_and(|status| status.first <= hour)
            });
            let mut details = Vec::new();
            let mut witnesses = Vec::with_capacity(at_hour.len());
            for &(&(_, ordinal), &recurring) in at_hour {
                let key = item_key(piece.slot, ordinal);
                let witness = membership.map(|history| coverage::reading_witness(TableId::PowerItems, history.reading(key, hour)));
                let needs_details = match witness {
                    None => true,
                    Some(coverage::Witness::Row(_)) => !self.index.contains_key(&(piece.run, piece.slot, ordinal)),
                    _ => false,
                };
                if needs_details {
                    details.push(key);
                }
                witnesses.push((ordinal, recurring, witness));
            }
            let frame = if details.is_empty() { None } else {
                Some(frames.frame(hour, &[at_hour::Read::these(TableId::PowerItems, details)], "Politics spending")?)
            };
            for (ordinal, recurring, witness) in witnesses {
                out.insert((ordinal, hour), self.at(frame.as_ref(), piece, witness, ordinal, hour, recurring));
            }
        }
        Ok(out)
    }

    fn at(&mut self, frame: Option<&DecodedFrame>, piece: &Piece, witness: Option<coverage::Witness<'_, [Cell]>>, ordinal: u32, read: i32, recurring: bool) -> (u32, bool) {
        let (run, slot) = (piece.run, piece.slot);
        let reason = match witness {
            Some(coverage::Witness::Absent) => ITEM_ABSENT.to_owned(),
            Some(coverage::Witness::NotRecorded(reason)) => reason,
            witness => {
                if matches!(witness, Some(coverage::Witness::Row(_))) && let Some(&at) = self.index.get(&(run, slot, ordinal)) {
                    return (at, true);
                }
                let frame = frame.expect("a first or unproven witness requests its detail row");
                match coverage::witnessed::<tables::PowerItems>(frame, item_key(slot, ordinal)) {
                    coverage::Witness::Row(row) => {
                        let at = *self.index.entry((run, slot, ordinal)).or_insert_with(|| {
                            self.asked.push((run, row.clone()));
                            (self.asked.len() - 1) as u32
                        });
                        return (at, true);
                    }
                    coverage::Witness::Absent => ITEM_ABSENT.to_owned(),
                    coverage::Witness::NotRecorded(reason) => reason,
                }
            }
        };
        let unrecorded = &mut self.unrecorded;
        let at = *self.unrecorded_index.entry((run, slot, ordinal, reason.clone())).or_insert_with(|| {
            unrecorded.push(UnrecordedItem { run, slot, ordinal, read, recurring, reason });
            (unrecorded.len() - 1) as u32
        });
        (UNRECORDED_ITEM | at, false)
    }

    fn resolved(&self, at: u32) -> u32 {
        if at & UNRECORDED_ITEM == 0 { at } else { self.asked.len() as u32 + (at & !UNRECORDED_ITEM) }
    }
}

fn unrecorded_item(item: &UnrecordedItem) -> SpendItem {
    SpendItem {
        id: format!("{}:{}:{}:unrecorded", item.slot, item.ordinal, item.read),
        group_id: format!("unrecorded:{}:{}", item.slot, item.ordinal),
        cause: format!("{:?}", PowerCause::NotIdentified),
        category: "notIdentified".into(),
        recurring: item.recurring,
        first_hour: item.read,
        first_read: item.read,
        name: None,
        action: "Spending item not recorded".into(),
        character: None,
        target_tag: None,
        target_name: None,
        unknown: Some(item.reason.clone()),
        kind: None,
        ledger: (item.run, item.slot, item.ordinal),
    }
}

fn spending(query: &Query<'_>, params: &PowerSpendingParams, naming: &naming::Context, cancelled: &AtomicBool) -> io::Result<PowerSpendingReply> {
    let currency = params.currency;
    let mut reply = PowerSpendingReply {
        branch_id: params.branch_id.clone(),
        tag: String::new(),
        currency,
        from: params.from,
        to: params.to,
        decimals: 5,
        items: Vec::new(),
        categories: Vec::new(),
        payments: Vec::new(),
        days: Vec::new(),
        gains: Vec::new(),
        coverage: Vec::new(),
        gain_coverage: Vec::new(),
        warnings: Vec::new(),
    };
    let mut items = Items::default();
    let mut credits: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut day_gap_hours: BTreeMap<String, u64> = BTreeMap::new();
    let mut gain_buckets: BTreeMap<(i32, Option<i32>), GainBucket> = BTreeMap::new();
    let mut unmatched = 0_u64;
    let has_day_log = query.replay.recording().predates(TableId::PowerSpendDays.index()).is_none();
    let held = pieces(query, &params.country, params.from, params.to)?;
    reply.tag = query.holders.tag_at(&params.country, params.to).map_or_else(|| params.country.key().into_owned(), str::to_owned);
    let mut batches = Vec::new();
    for piece in &held {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let coverage_start = reply.coverage.len();
        coverage(query, piece, currency, &mut reply.coverage)?;
        instant_gains(query, piece, currency, &mut reply.gain_coverage, &mut gain_buckets)?;
        let low = u128::from(piece.slot) << 40 | u128::from(currency.code()) << 32;
        let (payments_from, days_from) = (reply.payments.len(), reply.days.len());
        let mut asks = BTreeMap::new();
        let mut ask = |ordinal: u32, hour: i32, recurring: bool| {
            asks.entry((hour, ordinal)).or_insert(recurring);
        };
        changes(query, piece, low, low | u128::from(u32::MAX), has_day_log, &mut |change| {
            if change.ordinal == PowerSpendTotalRow::GAINS {
                if let Some(gain) = gained(&change) {
                    unmatched += u64::from(gain.unmatched);
                    let gained_val = gain.gained_raw.as_deref().and_then(|value| value.parse::<i128>().ok());
                    let capped_val = gain.capped_raw.as_deref().and_then(|value| value.parse::<i128>().ok());
                    gain_buckets.entry((gain.read, None)).or_default().push(gained_val, capped_val, gain.days, gain.unknown_days, gain.unmatched);
                }
                return;
            }
            let (payment, day) = listed(&change, change.ordinal, !has_day_log);
            if payment.is_none() && day.is_none() {
                return;
            }
            ask(change.ordinal, change.hour, payment.is_none());
            reply.payments.extend(payment);
            reply.days.extend(day);
        })?;
        if has_day_log {
            let gaps = day_log(query, piece, currency, &mut |hour, ordinal, mut day| {
                ask(ordinal, hour, true);
                day.item = ordinal;
                day.read = hour;
                reply.days.push(day);
            })?;
            apply_day_gaps(&mut reply.coverage, coverage_start, &gaps, &mut day_gap_hours);
        }
        batches.push(ItemAsks { piece, payments: payments_from..reply.payments.len(), days: days_from..reply.days.len(), asks });
    }
    let mut fast_items: Vec<Option<HashMap<u32, PowerItemRow>>> = Vec::with_capacity(batches.len());
    let mut fallback: Vec<&ItemAsks<'_>> = Vec::new();
    for batch in &batches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let fast = complete_items(query, batch)?;
        if fast.is_none() && !batch.asks.is_empty() {
            fallback.push(batch);
        }
        fast_items.push(fast);
    }
    let membership = if fallback.is_empty() { HashMap::new() } else { item_membership(query, &fallback)? };
    let mut frames = if fallback.is_empty() { None } else { Some(at_hour::Frames::new(query)?) };
    for (batch, fast) in batches.into_iter().zip(fast_items) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let resolved = match fast {
            Some(rows) => items.read_complete(&batch, rows),
            None if batch.asks.is_empty() => HashMap::new(),
            None => items.read(frames.as_mut().expect("fallback batches open Frames"), &batch, membership.get(&batch.piece.run))?,
        };
        let missing = resolved.values().filter(|(_, known)| !known).count();
        for payment in &mut reply.payments[batch.payments] {
            payment.item = resolved[&(payment.item, payment.read)].0;
        }
        for day in &mut reply.days[batch.days] {
            day.item = resolved[&(day.item, day.read)].0;
            if day.credits > 0 || day.unknown_credits > 0 {
                let held = credits.entry(day.item).or_default();
                held.0 += day.credits;
                held.1 += day.unknown_credits;
            }
        }
        if missing != 0 {
            reply.warnings.push(format!("Politics spending item is not recorded in power items (count: {missing})"));
        }
    }
    for payment in &mut reply.payments {
        payment.item = items.resolved(payment.item);
    }
    for day in &mut reply.days {
        day.item = items.resolved(day.item);
    }
    let credits: BTreeMap<u32, (u32, u32)> = credits.into_iter().map(|(at, count)| (items.resolved(at), count)).collect();
    reply.payments.sort_by_key(|payment| (payment.read, payment.hour, payment.item));
    reply.days.sort_by_key(|day| (day.read, day.day, day.item));
    reply.gains = gain_buckets
        .into_iter()
        .map(|((read, hour), bucket)| SpendGain {
            hour,
            read,
            gained_raw: bucket.gained.map(|value| value.to_string()),
            capped_raw: bucket.capped.map(|value| value.to_string()),
            days: bucket.days,
            unknown_days: bucket.unknown_days,
            unmatched: bucket.unmatched,
        })
        .collect();
    let asked = &items.asked;
    let rows: Vec<naming::Asked<'_>> = asked.iter().map(|(run, row)| naming::Asked { run: *run, row }).collect();
    let named = naming::name(query, &rows, naming, cancelled)?;
    let mut categories: BTreeMap<String, SpendCategory> = BTreeMap::new();
    for ((run, row), named) in asked.iter().zip(named) {
        categories.entry(named.category.0.clone()).or_insert_with(|| SpendCategory {
            id: named.category.0.clone(),
            name: named.category.1.clone(),
            identified: named.category.0 != "notIdentified",
        });
        reply.items.push(naming::ledger_item(format!("{}:{}:{}", row.holder, row.ordinal, row.first_read), (*run, row.holder, row.ordinal), row, named));
    }
    if !items.unrecorded.is_empty() {
        let (id, name) = naming::cause_category(PowerCause::NotIdentified);
        categories.entry(id.to_owned()).or_insert_with(|| SpendCategory { id: id.to_owned(), name: name.to_owned(), identified: false });
    }
    reply.items.extend(items.unrecorded.iter().map(unrecorded_item));
    for (at, (known, unknown)) in credits {
        let item = &reply.items[at as usize];
        let label = item.name.as_deref().unwrap_or(&item.action);
        if known > 0 {
            reply.warnings.push(format!("Negative daily political power cost component {label}: recorded credit is not included in spending (count: {known})"));
        }
        if unknown > 0 {
            reply.warnings.push(format!("Negative daily political power cost component {label}: credit whose amount was not recorded is not included in spending (count: {unknown})"));
        }
    }
    for (reason, hours) in day_gap_hours {
        reply.warnings.push(format!("{reason} (hours: {hours})"));
    }
    if unmatched != 0 {
        reply.warnings.push(format!("Daily political power cost: matched PP application is unavailable (count: {unmatched})"));
    }
    reply.categories = categories.into_values().collect();
    Ok(reply)
}
