
pub mod day;
mod naming;
#[cfg(test)]
mod tests;

use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product, check_reply_budget};
use super::{politics::{Holding, holdings}, ideas::{IdeaInfo, effect_keys, recorded_definitions}};
use dashboard_world::products::equipment_names::EquipmentIcon;
use dashboard_base::products::idea_format::{self, LocalisedText, ModifierFormat};
use dashboard_core::dto::Artwork;
use dashboard_base::{Source, products::at_hour};
use dashboard_query::Query;
use replay_lanes::{lanes::{self, Keys}, points::Cell};
use replay_columnar::manifest::Status;
use replay_data::rows::{PowerCause, PowerItemRow, PowerSpendCoverageRow, PowerSpendTotalRow, TableId, tables};
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
    #[doc = " The key of its picture in `icons`."]
    pub icon: Option<String>,
    pub character: Option<u32>,
    pub target_tag: Option<String>,
    pub target_name: Option<String>,
    #[doc = " Why a name, target or picture is not known."]
    pub unknown: Option<String>,
    #[doc = " The idea in `ideaTooltips` its tooltip shows."]
    pub tooltip_token: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendIdeaTooltips {
    pub ideas: Vec<IdeaInfo>,
    pub artwork: Vec<Artwork>,
    pub modifiers: Vec<ModifierFormat>,
    pub texts: Vec<LocalisedText>,
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
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendGain {
    pub read: i32,
    #[doc = " What the days gained before daily costs, fixed5; none where not recorded."]
    pub gained_raw: Option<String>,
    #[doc = " What the cap kept off the days' gains."]
    pub capped_raw: Option<String>,
    pub days: u32,
    #[doc = " Days whose gain was not read."]
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
    pub icons: Vec<EquipmentIcon>,
    #[doc = " None where no item is an idea or an advisor."]
    pub idea_tooltips: Option<SpendIdeaTooltips>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PowerSpendingReply);

pub struct PowerSpending;

impl Product for PowerSpending {
    const KEY: &'static str = "power-spending";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PowerSpendingParams;
    type Reply = PowerSpendingReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            PowerSpendingParams::decl(),
            PowerSpendingReply::decl(),
            SpendCurrency::decl(),
            SpendItem::decl(),
            SpendCategory::decl(),
            SpendPayment::decl(),
            SpendDay::decl(),
            SpendGain::decl(),
            SpendCoverage::decl(),
            SpendIdeaTooltips::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid power spending branch")?;
        validate_country(&params.country)?;
        if params.from > params.to {
            return Err("A power spending span ends before it starts");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid power spending branch"))?)?;
        let naming = naming::context(source, context)?;
        let reply = spending(source, &query, params, &naming, cancelled)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub(super) fn recurring(row: &PowerItemRow) -> bool {
    row.kind != PowerItemRow::CHANGE || matches!(row.cause, PowerCause::DailyGain | PowerCause::AbilityUpkeep)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Piece {
    pub run: u32,
    pub slot: u32,
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
            out.push(Piece { run: stretch.run, slot: holding.slot, first: from, last: to, baseline });
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
    let days = count(change, "days");
    let day_payments = count(change, "day_payments");
    let day = (include_days && (days > 0 || day_payments > 0 || count(change, "credits") > 0)).then(|| SpendDay {
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
    });
    (payment, day)
}

pub(super) fn gained(change: &Change<'_>) -> Option<SpendGain> {
    let gain = SpendGain {
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

fn changes(query: &Query<'_>, piece: &Piece, low: u128, high: u128, each: &mut dyn FnMut(Change<'_>)) -> io::Result<()> {
    let columns: Vec<usize> = TOTALS.iter().map(|name| dashboard_declare::field(TableId::PowerSpendTotals, name)).collect::<io::Result<_>>()?;
    let from = piece.baseline.unwrap_or(piece.first);
    let history = dashboard_query::cost::storage(|| lanes::history(query.replay.recording(), piece.run, TableId::PowerSpendTotals, &columns, Keys::Range(low, high), from, piece.last))?;
    let order: Vec<usize> = columns.iter().map(|column| history.columns.binary_search(column).map_err(|_| dashboard_core::invalid("A totals column was not read"))).collect::<io::Result<_>>()?;
    for (key, steps) in &history.keys {
        let mut before: Option<Cells> = None;
        for step in steps {
            let Some(cells) = &step.cells else { continue };
            let after: Cells = std::array::from_fn(|at| Value::of(cells[order[at]]));
            if piece.baseline == Some(step.hour) {
                before = Some(after);
                continue;
            }
            if before.as_ref() != Some(&after) {
                each(Change { ordinal: *key as u32, hour: step.hour, before: before.as_ref(), after: &after });
            }
            before = Some(after);
        }
    }
    Ok(())
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

fn items_at(query: &Query<'_>, slot: u32, hour: i32) -> io::Result<BTreeMap<u32, PowerItemRow>> {
    let low = u128::from(slot) << 32;
    let reads = [at_hour::Read { table: TableId::PowerItems, keys: at_hour::Keys::Range(low, low | u128::from(u32::MAX)) }];
    let frame = at_hour::frame(query, hour, &reads, "Politics spending")?;
    Ok(frame.complete::<tables::PowerItems>().map(|rows| rows.iter().map(|row| (row.ordinal, row.clone())).collect()).unwrap_or_default())
}

fn spending(source: &Source, query: &Query<'_>, params: &PowerSpendingParams, naming: &naming::Context, cancelled: &AtomicBool) -> io::Result<PowerSpendingReply> {
    let currency = params.currency;
    let mut reply = PowerSpendingReply {
        branch_id: params.branch_id.clone(),
        tag: query.holders.name(&params.country, query.timeline.last),
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
        icons: Vec::new(),
        idea_tooltips: None,
        warnings: Vec::new(),
    };
    let mut asked: Vec<(u32, PowerItemRow)> = Vec::new();
    let mut index: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut credits: BTreeMap<u32, u32> = BTreeMap::new();
    let mut day_gap_hours: BTreeMap<String, u64> = BTreeMap::new();
    let mut unmatched = 0_u64;
    let has_day_log = query.replay.recording().predates(TableId::PowerSpendDays.index()).is_none();
    for piece in pieces(query, &params.country, params.from, params.to)? {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let coverage_start = reply.coverage.len();
        coverage(query, &piece, currency, &mut reply.coverage)?;
        let items = items_at(query, piece.slot, piece.last)?;
        let low = u128::from(piece.slot) << 40 | u128::from(currency.code()) << 32;
        let mut missing = 0_u64;
        changes(query, &piece, low, low | u128::from(u32::MAX), &mut |change| {
            if change.ordinal == PowerSpendTotalRow::GAINS {
                if let Some(gain) = gained(&change) {
                    unmatched += u64::from(gain.unmatched);
                    reply.gains.push(gain);
                }
                return;
            }
            let (payment, day) = listed(&change, 0, !has_day_log);
            if payment.is_none() && day.is_none() {
                return;
            }
            let Some(row) = items.get(&change.ordinal) else {
                missing += 1;
                return;
            };
            let at = *index.entry((piece.run, piece.slot, change.ordinal)).or_insert_with(|| {
                asked.push((piece.run, row.clone()));
                (asked.len() - 1) as u32
            });
            if let Some(mut payment) = payment {
                payment.item = at;
                reply.payments.push(payment);
            }
            if let Some(mut day) = day {
                day.item = at;
                if day.credits > 0 {
                    *credits.entry(at).or_default() += day.credits;
                }
                reply.days.push(day);
            }
        })?;
        if has_day_log {
            let gaps = day_log(query, &piece, currency, &mut |hour, ordinal, mut day| {
                let Some(row) = items.get(&ordinal) else {
                    missing += 1;
                    return;
                };
                let at = *index.entry((piece.run, piece.slot, ordinal)).or_insert_with(|| {
                    asked.push((piece.run, row.clone()));
                    (asked.len() - 1) as u32
                });
                day.item = at;
                day.read = hour;
                if day.credits > 0 {
                    *credits.entry(at).or_default() += day.credits;
                }
                reply.days.push(day);
            })?;
            apply_day_gaps(&mut reply.coverage, coverage_start, &gaps, &mut day_gap_hours);
        }
        if missing != 0 {
            reply.warnings.push(format!("Politics spending item is not recorded in power items (count: {missing})"));
        }
    }
    reply.payments.sort_by_key(|payment| (payment.read, payment.hour, payment.item));
    reply.days.sort_by_key(|day| (day.read, day.day, day.item));
    reply.gains.sort_by_key(|gain| gain.read);
    let rows: Vec<naming::Asked<'_>> = asked.iter().map(|(run, row)| naming::Asked { run: *run, row }).collect();
    let mut icons = BTreeMap::new();
    let named = naming::name(query, &rows, naming, &mut icons, cancelled)?;
    let mut categories: BTreeMap<String, SpendCategory> = BTreeMap::new();
    let mut ideas: BTreeMap<String, (IdeaInfo, Artwork)> = BTreeMap::new();
    for named in &named {
        if let Some((info, description)) = &named.idea {
            ideas.entry(info.token.clone()).or_insert_with(|| {
                let art = Artwork { token: info.token.clone(), name: named.name.clone(), description: description.clone(), base_duration_days: None, icon: None };
                (info.clone(), art)
            });
        }
    }
    if !ideas.is_empty()
        && let Ok(content) = naming.content()
    {
        let infos: Vec<IdeaInfo> = ideas.values().map(|(info, _)| info.clone()).collect();
        let (modifier_keys, text_keys) = effect_keys(&infos);
        let recorded = recorded_definitions(source, query)?;
        let (modifiers, formats) = idea_format::modifier_formats_from_recorded((*recorded).as_ref(), &modifier_keys, content);
        reply.warnings.extend(formats);
        let texts = idea_format::texts(content, &text_keys);
        reply.idea_tooltips = Some(SpendIdeaTooltips { ideas: infos, artwork: ideas.into_values().map(|(_, art)| art).collect(), modifiers, texts });
    }
    for ((_, row), named) in asked.iter().zip(named) {
        categories.entry(named.category.0.clone()).or_insert_with(|| SpendCategory {
            id: named.category.0.clone(),
            name: named.category.1.clone(),
            identified: named.category.0 != "notIdentified",
        });
        reply.items.push(naming::ledger_item(format!("{}:{}:{}", row.holder, row.ordinal, row.first_read), row, named));
    }
    for (at, count) in credits {
        let item = &reply.items[at as usize];
        reply.warnings.push(format!(
            "Negative daily political power cost component {}: recorded credit is not included in spending (count: {count})",
            item.name.as_deref().unwrap_or(&item.action)
        ));
    }
    for (reason, hours) in day_gap_hours {
        reply.warnings.push(format!("{reason} (hours: {hours})"));
    }
    if unmatched != 0 {
        reply.warnings.push(format!("Daily political power cost: matched PP application is unavailable (count: {unmatched})"));
    }
    reply.categories = categories.into_values().collect();
    reply.icons = naming::icons(icons);
    Ok(reply)
}
