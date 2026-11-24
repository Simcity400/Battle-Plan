use super::{PowerSpendingParams, PowerSpendingReply, SpendCoverage, SpendCurrency, SpendGain, naming, spending};
use dashboard_base::products::breakdown::{self, Amount, Breakdown, Dated, History, HistoryBuilder, Node};
use dashboard_base::products::{Country, LoadContext, Product, check_reply_budget, validate_country};
use dashboard_base::Source;
use dashboard_query::{identity::{Holder, Holders}, timeline::Timeline};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

const OTHER_CATEGORY: &str = "notIdentified";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PowerSpendingBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    pub currency: SpendCurrency,
    #[doc = " The period's first hour."]
    pub from: i32,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerSpendingBreakdownReply {
    pub from: i32,
    pub hour: i32,
    #[doc = " Everything in the reply stands unchanged from `hour` through this hour."]
    pub through: i32,
    pub breakdown: Breakdown,
    #[doc = " Gained over the same period before the cap, scaled by the breakdown's decimals; none when an"]
    #[doc = " amount gained was read without its value. With `gained_reason`, hours of the period whose gains"]
    #[doc = " were not read are left out: the figure is partial."]
    pub gained_raw: Option<String>,
    pub lost_at_cap_raw: Option<String>,
    #[doc = " Why the gain is not known, or (with `gained_raw`) why it is partial."]
    pub gained_reason: Option<String>,
    #[doc = " Why the loss at the cap is not known, or (with `lost_at_cap_raw`) why it is partial."]
    pub lost_at_cap_reason: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PowerSpendingBreakdownReply);

pub struct PowerSpendingBreakdown;

impl Product for PowerSpendingBreakdown {
    const KEY: &'static str = "power-spending-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PowerSpendingBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, currency, from: Start, hour: End];
    type Reply = PowerSpendingBreakdownReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        use dashboard_base::products::breakdown::{BreakdownPart, BreakdownState, BreakdownTotal};
        vec![
            PowerSpendingBreakdownParams::decl(),
            PowerSpendingBreakdownReply::decl(),
            SpendCurrency::decl(),
            Breakdown::decl(),
            BreakdownTotal::decl(),
            BreakdownPart::decl(),
            BreakdownState::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid power spending branch")?;
        validate_country(&params.country)?;
        if params.from > params.hour {
            return Err("A power spending breakdown ends before it starts");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid power spending branch"))?)?;
        let span = PowerSpendingParams {
            branch_id: params.branch_id.clone(),
            country: params.country.clone(),
            currency: params.currency,
            from: params.from,
            to: query.timeline.last.max(params.hour),
        };
        let held = breakdown::held(source, context, span.clone(), || {
            let naming = naming::context(source, context)?;
            let whole = spending(source, &query, &span, &naming, cancelled)?;
            let spend = unrecorded(&query.timeline, &query.holders, &span.country, &whole.coverage, whole.from, whole.to);
            let gain = unrecorded(&query.timeline, &query.holders, &span.country, &whole.gain_coverage, whole.from, whole.to);
            Ok(index(&whole, &spend, &gain))
        })?;
        let reply = held.at(params.hour);
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn holes(runs: &[SpendCoverage], from: i32, to: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut cursor = from;
    for run in runs {
        let (first, last) = (run.first_hour.max(from), run.last_hour.min(to));
        if first > last {
            continue;
        }
        if first > cursor {
            out.push((cursor, first - 1));
        }
        cursor = cursor.max(last.saturating_add(1));
    }
    if cursor <= to {
        out.push((cursor, to));
    }
    out
}

pub(super) fn unrecorded(timeline: &Timeline, holders: &Holders, country: &Country, runs: &[SpendCoverage], from: i32, to: i32) -> Vec<(i32, i32, String)> {
    let mut out: Vec<(i32, i32, String)> = runs
        .iter()
        .filter(|run| !run.covered && run.first_hour.max(from) <= run.last_hour.min(to))
        .map(|run| (run.first_hour.max(from), run.last_hour.min(to), run.reason.clone().unwrap_or_else(|| "Spending was not recorded".into())))
        .collect();
    for (first, last) in holes(runs, from, to) {
        for gap in timeline.unheld_within(first, last) {
            out.push((gap.first, gap.last, gap.reason));
        }
        for &(unknown_first, unknown_last, ref reason) in &holders.unknown {
            let (unknown_first, unknown_last) = (unknown_first.max(first), unknown_last.min(last));
            if unknown_first <= unknown_last {
                out.push((unknown_first, unknown_last, reason.clone()));
            }
        }
        for stretch in timeline.within(first, last) {
            for run in holders.within(country, stretch.first, stretch.last) {
                if run.holder == Holder::Unresolved {
                    out.push((run.first.max(stretch.first), run.last.min(stretch.last), "The country's holder was not recorded".into()));
                }
            }
        }
    }
    out.sort_by_key(|gap| gap.0);
    out
}

fn parse(raw: Option<&str>) -> Option<i128> {
    raw.and_then(|raw| raw.parse().ok())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Gain {
    gained: Option<i128>,
    gained_reason: Option<String>,
    lost: Option<i128>,
    lost_reason: Option<String>,
}

pub(super) struct PowerHistory {
    from: i32,
    history: History,
    gained: Dated<Gain>,
    warnings: Dated<Vec<String>>,
}

impl PowerHistory {
    pub(super) fn at(&self, hour: i32) -> PowerSpendingBreakdownReply {
        let (breakdown, through) = self.history.at(hour);
        let gain = self.gained.at(hour).cloned().unwrap_or_default();
        PowerSpendingBreakdownReply {
            from: self.from,
            hour,
            through,
            breakdown,
            gained_raw: gain.gained.map(|value| value.to_string()),
            lost_at_cap_raw: gain.lost.map(|value| value.to_string()),
            gained_reason: gain.gained_reason,
            lost_at_cap_reason: gain.lost_reason,
            warnings: self.warnings.at(hour).cloned().unwrap_or_default(),
        }
    }
}

struct Entry {
    item: u32,
    read: i32,
    charge: Option<i128>,
    payments: u32,
    unknown: u32,
    uncovered: bool,
    credits: u32,
}

fn entries(reply: &PowerSpendingReply) -> Vec<Entry> {
    let mut entries: Vec<Entry> = reply
        .payments
        .iter()
        .map(|payment| Entry {
            item: payment.item,
            read: payment.read,
            charge: parse(payment.known_raw.as_deref()),
            payments: payment.payments,
            unknown: payment.unknown,
            uncovered: payment.uncovered,
            credits: 0,
        })
        .chain(reply.days.iter().map(|day| Entry {
            item: day.item,
            read: day.read,
            charge: parse(day.charge_raw.as_deref()),
            payments: day.payments,
            unknown: day.unknown,
            uncovered: day.uncovered > 0,
            credits: day.credits,
        }))
        .collect();
    entries.sort_by_key(|entry| (entry.read, entry.item));
    entries
}

fn first_read(from: i32, to: i32, unread: &[(i32, i32, String)]) -> Option<i32> {
    let mut spans: Vec<(i32, i32)> = unread.iter().map(|(first, last, _)| (*first, *last)).collect();
    spans.sort_unstable();
    let mut cursor = from;
    for (first, last) in spans {
        if first > cursor {
            break;
        }
        cursor = cursor.max(last.saturating_add(1));
    }
    (cursor <= to).then_some(cursor)
}

#[derive(Default)]
struct Measure {
    sum: i128,
    reason: Option<String>,
}

impl Measure {
    fn add(&mut self, raw: Option<&str>, exact: bool, missing: &str) {
        match parse(raw) {
            Some(value) => self.sum = self.sum.saturating_add(value),
            None => {
                self.reason.get_or_insert_with(|| missing.to_owned());
            }
        }
        if !exact {
            self.reason.get_or_insert_with(|| missing.to_owned());
        }
    }
}

fn gains(reply: &PowerSpendingReply, unread: &[(i32, i32, String)]) -> Dated<Gain> {
    let (mut gained, mut lost) = (Measure::default(), Measure::default());
    let read_from = first_read(reply.from, reply.to, unread);
    let mut seen = read_from == Some(reply.from);
    let mut events: Vec<(i32, Option<&SpendGain>, Option<&str>)> =
        reply.gains.iter().filter(|gain| gain.read >= reply.from).map(|gain| (gain.read, Some(gain), None)).collect();
    events.extend(unread.iter().map(|(first, _, reason)| ((*first).max(reply.from), None, Some(reason.as_str()))));
    events.extend(read_from.map(|hour| (hour, None, None)));
    events.sort_by_key(|event| event.0);
    let held = |seen: bool, gained: &Measure, lost: &Measure| Gain {
        gained: seen.then_some(gained.sum),
        gained_reason: gained.reason.clone(),
        lost: seen.then_some(lost.sum),
        lost_reason: lost.reason.clone(),
    };
    let mut values = vec![(reply.from, held(seen, &gained, &lost))];
    for (hour, gain, unread) in events {
        seen |= read_from.is_some_and(|from| from <= hour);
        if let Some(reason) = unread {
            let text = format!("Gains in hours when power changes were not read are not counted: {reason}.");
            gained.reason.get_or_insert_with(|| text.clone());
            lost.reason.get_or_insert(text);
        }
        if let Some(gain) = gain {
            seen = true;
            let exact = gain.unknown_days == 0 && gain.unmatched == 0;
            gained.add(gain.gained_raw.as_deref(), exact, "Some gains were not recorded.");
            lost.add(gain.capped_raw.as_deref(), exact, "Some losses at the cap were not recorded.");
        }
        values.push((hour, held(seen, &gained, &lost)));
    }
    Dated::new(values)
}

pub(super) fn index(reply: &PowerSpendingReply, spend_unread: &[(i32, i32, String)], gain_unread: &[(i32, i32, String)]) -> PowerHistory {
    let entries = entries(reply);
    let names: HashMap<&str, (&str, bool)> = reply.categories.iter().map(|category| (category.id.as_str(), (category.name.as_str(), category.identified))).collect();
    let mut group_names: HashMap<&str, &str> = HashMap::new();
    for item in &reply.items {
        if let Some(name) = &item.name {
            group_names.insert(item.group_id.as_str(), name.as_str());
        }
    }
    let mut categories: HashMap<String, u32> = HashMap::new();
    let mut kinds: HashMap<String, u32> = HashMap::new();
    let mut groups: HashMap<String, u32> = HashMap::new();
    let mut built = HistoryBuilder::new(reply.decimals, reply.from, reply.to).dated();
    let mut dated_warnings: Vec<(i32, String)> = Vec::new();
    for entry in &entries {
        let Some(item) = reply.items.get(entry.item as usize) else { continue };
        let next = categories.len() as u32;
        let category_ordinal = *categories.entry(item.category.clone()).or_insert(next);
        let (name, identified) = names.get(item.category.as_str()).copied().unwrap_or((OTHER_CATEGORY, false));
        let mut category = Node::new(item.category.clone(), if identified { name } else { "Other spending" }, category_ordinal);
        category = if identified { category.with_art(naming::category_art(&item.category)) } else { category.other() };
        let mut path = vec![category];
        if let Some(kind) = &item.kind {
            let key = kind.id.clone().unwrap_or_else(|| format!("{}:unrecorded", item.category));
            let next = kinds.len() as u32;
            let ordinal = *kinds.entry(key.clone()).or_insert(next);
            path.push(match &kind.name {
                Some(name) if kind.id.is_some() => Node::new(key, name.clone(), ordinal).name_missing(kind.unknown.clone()),
                _ => Node::new(key, "Category not recorded", ordinal).other(),
            });
        }
        if let Some(&name) = group_names.get(item.group_id.as_str()).filter(|_| item.name.is_some()) {
            let next = groups.len() as u32;
            let ordinal = *groups.entry(item.group_id.clone()).or_insert(next);
            path.push(Node::new(item.group_id.clone(), name, ordinal));
        }
        let leaf = built.leaf(path);
        let count = Some(u64::from(entry.payments));
        match entry.charge {
            Some(charge) => {
                let amount = if entry.uncovered { Amount::Partial(charge, "Some payments may be missing.".into()) } else { Amount::Known(charge) };
                built.add(entry.read, leaf, amount, count);
                if entry.unknown > 0 {
                    built.add(entry.read, leaf, Amount::NotRecorded("Some payment costs were not recorded.".into()), None);
                }
            }
            None => built.add(entry.read, leaf, Amount::NotRecorded("Some payment costs were not recorded.".into()), count),
        }
        if entry.credits > 0 {
            let label = item.name.as_deref().unwrap_or(&item.action);
            let warning = format!("Negative daily cost component {label}: its credit is not counted as spending");
            if !dated_warnings.iter().any(|(_, held)| *held == warning) {
                dated_warnings.push((entry.read, warning));
            }
        }
        if let Some(reason) = item.unknown.as_deref().filter(|_| item.category == OTHER_CATEGORY) {
            let warning = format!("The item of some spending was not recorded: {reason}");
            if !dated_warnings.iter().any(|(_, held)| *held == warning) {
                dated_warnings.push((entry.read, warning));
            }
        }
    }
    for (first, last, reason) in spend_unread {
        built.unrecorded(*first, *last, reason.clone());
    }
    let gained = gains(reply, gain_unread);
    for hour in gained.hours() {
        built.change(hour);
    }
    let mut warnings: Vec<(i32, Vec<String>)> = Vec::new();
    dated_warnings.sort_by_key(|(hour, _)| *hour);
    for (hour, warning) in dated_warnings {
        built.change(hour);
        let mut list = warnings.last().map(|(_, list)| list.clone()).unwrap_or_default();
        list.push(warning);
        warnings.push((hour, list));
    }
    PowerHistory { from: reply.from, history: built.build(), gained, warnings: Dated::new(warnings) }
}
