//! `politics-state-history`: every election schedule, or one country's balances of power, over the
//! whole branch, as runs per holder (canonical slot) named by the tags it held.
//!
//! - Elections: the recorded game predicate (`has_elections`) for a country owning states (the
//!   game holds no others' elections), the stored signed interval, the next date from the native
//!   calendar step and the term's start. A known "not held" and an unrecorded eligibility stay
//!   apart (`refused`).
//! - Balances: records have no identity beyond their stored position, so the history follows
//!   what each holder belongs to: per holder and balance token, the record's value and stored
//!   position, range and trending while the holder is a member, and the balance the game shows it
//!   (the first record listing it). The records a holder was listed by are read as one history of
//!   their rows per stretch; no other record is read.
//!
//! Every run is read from its column's tracks per stretch; nothing carries across a gap.

use super::{
    HolderTag, Holding, PoliticsRun, by_id, decimal, holdings, pieces, state::{BalanceRange, BalanceSide}, stretch_runs, tags_of, token,
    value_at, within,
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    products::{LoadContext, Product, check_reply_budget, validate_tag},
};
use dashboard_base::span::HourSpan;
use dashboard_query::{Query, timeline::Stretch};
use dashboard_text::catalogue::View;
use replay::lanes::{self, Cell, History, Keys};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum PoliticsHistoryMode {
    /// One country's balances over the whole branch.
    Country,
    /// Every election schedule over the whole branch.
    Elections,
    /// Every holder's balances over the whole branch, as the records listing it only: no values,
    /// ranges or leanings.
    Memberships,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoliticsHistoryParams {
    pub branch_id: String,
    pub mode: PoliticsHistoryMode,
    /// The country, for [`PoliticsHistoryMode::Country`].
    #[serde(default)]
    #[ts(optional)]
    pub tag: Option<String>,
}

/// One holder's election schedule over the branch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ElectionHistory {
    pub tags: Vec<HolderTag>,
    /// "1" when the game holds elections, "0" when it does not.
    pub held: Vec<PoliticsRun>,
    /// "1" while its schedule was read but its eligibility was not: unknown, not denied.
    pub refused: Vec<PoliticsRun>,
    /// Signed months.
    pub interval: Vec<PoliticsRun>,
    /// The next election's hour.
    pub next: Vec<PoliticsRun>,
    /// The term's start: the stored last election's hour, where it is a date.
    pub last: Vec<PoliticsRun>,
}

/// One holder's balances over the branch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HolderBalances {
    pub holder: String,
    pub tags: Vec<HolderTag>,
    pub balances: Vec<BalanceHistory>,
    /// The balance the game shows the holder, by token.
    pub shown: Vec<PoliticsRun>,
}

/// One balance a holder belonged to, with its authored presentation.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceHistory {
    pub token: String,
    pub name: Option<String>,
    pub left: Option<BalanceSide>,
    pub right: Option<BalanceSide>,
    /// The authored ranges in authored order; empty when content refuses.
    pub ranges: Vec<BalanceRange>,
    /// Fixed5 values while the holder belonged to it.
    pub values: Vec<PoliticsRun>,
    /// The record's stored position: holders sharing it at an hour share that record. Not an
    /// identity across hours.
    pub record: Vec<PoliticsRun>,
    /// The authored range the record is in, by token, where it names one.
    pub range: Vec<PoliticsRun>,
    /// The side the record is moving toward, by token, where it names one.
    pub trending: Vec<PoliticsRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoliticsHistoryReply {
    pub branch_id: String,
    pub mode: PoliticsHistoryMode,
    /// The branch's last hour: the history reads through it.
    pub last_hour: Option<i32>,
    pub tag: Option<String>,
    /// The hours the mode's table was read; others are unknown.
    pub known: Vec<HourSpan>,
    pub elections: Vec<ElectionHistory>,
    /// Content tokens to their in-game names, where they resolve.
    pub names: BTreeMap<String, String>,
    /// Every holder that held the country's tag, its balances whole.
    pub holder_balances: Vec<HolderBalances>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PoliticsHistoryReply);

pub struct PoliticsStateHistory;

impl Product for PoliticsStateHistory {
    const KEY: &'static str = "politics-state-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PoliticsHistoryParams;
    type Reply = PoliticsHistoryReply;

    fn types() -> Vec<String> {
        vec![
            PoliticsHistoryParams::decl(),
            PoliticsHistoryReply::decl(),
            PoliticsHistoryMode::decl(),
            PoliticsRun::decl(),
            HolderTag::decl(),
            ElectionHistory::decl(),
            BalanceHistory::decl(),
            HolderBalances::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid politics history branch")?;
        match params.mode {
            PoliticsHistoryMode::Country => validate_tag(params.tag.as_deref().ok_or("Missing country")?),
            _ => Ok(()),
        }
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = crate::answer::query(source, params.branch_id.parse().map_err(|_| crate::invalid("Invalid politics history branch"))?)?;
        let catalogue = matches!(params.mode, PoliticsHistoryMode::Country)
            .then(|| context.content().and_then(|content| catalogue::catalogue(source, content)).map_err(|error| error.to_string()));
        let reply = answer(&query, params, catalogue, cancelled)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

/// The history `params` asks of `query`'s timeline.
pub fn answer(query: &Query<'_>, params: &PoliticsHistoryParams, catalogue: Option<Result<Arc<Catalogue>, String>>, cancelled: &AtomicBool) -> io::Result<PoliticsHistoryReply> {
    let mut reply = PoliticsHistoryReply {
        branch_id: params.branch_id.clone(),
        mode: params.mode,
        last_hour: Some(query.timeline.last),
        tag: params.tag.clone(),
        known: Vec::new(),
        elections: Vec::new(),
        names: BTreeMap::new(),
        holder_balances: Vec::new(),
        warnings: Vec::new(),
    };
    let held = holdings(&query.holders);
    match params.mode {
        PoliticsHistoryMode::Elections => elections(query, &held, &mut reply, cancelled)?,
        PoliticsHistoryMode::Country => {
            let tag = params.tag.as_deref().ok_or_else(|| crate::invalid("A country history names its country"))?;
            let holders: Vec<u32> = held.iter().filter(|(_, held)| held.iter().any(|holding| holding.tag == tag)).map(|(&slot, _)| slot).collect();
            balances(query, &held, &holders, true, catalogue, &mut reply, cancelled)?;
        }
        PoliticsHistoryMode::Memberships => {
            let holders: Vec<u32> = held.keys().copied().collect();
            balances(query, &held, &holders, false, catalogue, &mut reply, cancelled)?;
        }
    }
    Ok(reply)
}

fn check(cancelled: &AtomicBool) -> io::Result<()> {
    crate::metadata::check_cancel(cancelled)
}

/// `spans` in order, joined where they overlap or meet.
fn joined(mut spans: Vec<(i32, i32)>) -> Vec<HourSpan> {
    spans.sort_unstable();
    let mut out: Vec<HourSpan> = Vec::new();
    for (first, last) in spans {
        match out.last_mut() {
            Some(span) if i64::from(first) <= i64::from(span.last_hour) + 1 => span.last_hour = span.last_hour.max(last),
            _ => out.push(HourSpan { first_hour: first, last_hour: last }),
        }
    }
    out
}

fn elections(query: &Query<'_>, held: &BTreeMap<u32, Vec<Holding>>, reply: &mut PoliticsHistoryReply, cancelled: &AtomicBool) -> io::Result<()> {
    struct Parts {
        held: Vec<(i32, i32, i128)>,
        refused: Vec<(i32, i32, i128)>,
        interval: Vec<(i32, i32, i128)>,
        next: Vec<(i32, i32, i128)>,
        last: Vec<(i32, i32, i128)>,
    }
    let mut by_slot: BTreeMap<u32, Parts> = BTreeMap::new();
    let mut known = Vec::new();
    let mut unresolved = 0_u64;
    let ids = by_id(held);
    for stretch in &query.timeline.stretches {
        check(cancelled)?;
        let last = stretch_runs(query, stretch, TableId::Politics, "last_election")?;
        let has = stretch_runs(query, stretch, TableId::Politics, "has_elections")?;
        let interval = stretch_runs(query, stretch, TableId::Politics, "election_interval")?;
        let cache = stretch_runs(query, stretch, TableId::Politics, "next_election_cache")?;
        let owned = stretch_runs(query, stretch, TableId::CountryCounterInputs, "owned_state_count")?;
        for (&key, runs) in &last {
            let Ok(id) = u32::try_from(key) else { continue };
            let live = within(Some(runs), stretch.first, stretch.last);
            known.extend(live.iter().map(|&(first, last, _)| (first, last)));
            let Some(holdings) = ids.get(&id) else {
                unresolved += 1;
                continue;
            };
            for holding in holdings {
                let (first, end) = (holding.first.max(stretch.first), holding.last.min(stretch.last));
                if first > end {
                    continue;
                }
                let live = within(Some(runs), first, end);
                let has = within(has.get(&key), first, end);
                let interval = within(interval.get(&key), first, end);
                let cache = within(cache.get(&key), first, end);
                let owned = within(owned.get(&key), first, end);
                let parts = by_slot.entry(holding.slot).or_insert_with(|| Parts { held: Vec::new(), refused: Vec::new(), interval: Vec::new(), next: Vec::new(), last: Vec::new() });
                for (from, to) in pieces(first, end, &[&live, &has, &interval, &cache, &owned]) {
                    let Some(last_election) = value_at(&live, from) else { continue };
                    let last_election = last_election as i32;
                    let months = value_at(&interval, from).map(|value| value as i32);
                    let exists = value_at(&owned, from).map(|count| count > 0);
                    let eligible = match (value_at(&has, from).map(|value| value != 0), exists) {
                        (Some(false), _) | (Some(true), Some(false)) => Some(false),
                        (Some(true), Some(true)) => Some(true),
                        (Some(true), None) | (None, _) => None,
                    };
                    match eligible {
                        Some(value) => parts.held.push((from, to, i128::from(value))),
                        None => parts.refused.push((from, to, 1)),
                    }
                    if let Some(months) = months {
                        parts.interval.push((from, to, i128::from(months)));
                    }
                    // An unrecorded cache and a recorded empty one both leave the date unknown.
                    let cached = value_at(&cache, from).map(|value| Some(value as i32));
                    if let Some(next) = crate::products::country_people::next_election(last_election, months, cached) {
                        parts.next.push((from, to, i128::from(next)));
                    }
                    if !engine_native::cpolitical_status::needs_election_cache(last_election) {
                        parts.last.push((from, to, i128::from(last_election)));
                    }
                }
            }
        }
    }
    reply.known = joined(known);
    for (slot, mut parts) in by_slot {
        // A holder's raw ids are read in no order: its runs are put in hour order before joining.
        for runs in [&mut parts.held, &mut parts.refused, &mut parts.interval, &mut parts.next, &mut parts.last] {
            runs.sort_unstable_by_key(|run| run.0);
        }
        let history = ElectionHistory {
            tags: held.get(&slot).map(|held| tags_of(held)).unwrap_or_default(),
            held: decimal(parts.held),
            refused: decimal(parts.refused),
            interval: decimal(parts.interval),
            next: decimal(parts.next),
            last: decimal(parts.last),
        };
        if !history.held.is_empty() || !history.refused.is_empty() || !history.interval.is_empty() {
            reply.elections.push(history);
        }
    }
    if unresolved != 0 {
        reply.warnings.push(format!("{unresolved} refused election schedules: the country has no identity"));
    }
    Ok(())
}

/// One stored record's columns over a stretch, each as runs of its cell: none where its row is
/// not shown.
struct RecordRuns {
    value: Vec<(i32, i32, Option<Cell>)>,
    balance: Vec<(i32, i32, Option<Cell>)>,
    trending: Vec<(i32, i32, Option<Cell>)>,
    range: Vec<(i32, i32, Option<Cell>)>,
}

/// `records` of `stretch`: their balance runs and, `whole`, their value, trending and range runs,
/// read as one history of those rows (their changes, one pass of the stretch's blocks), with the
/// texts they name. A column not read has no runs.
fn record_runs(query: &Query<'_>, stretch: &Stretch, records: &[u128], whole: bool) -> io::Result<(HashMap<u128, RecordRuns>, History)> {
    let table = TableId::PowerBalances;
    let columns = [
        dashboard_declare::field(table, "value_raw")?,
        dashboard_declare::field(table, "balance")?,
        dashboard_declare::field(table, "trending")?,
        dashboard_declare::field(table, "active_range")?,
    ];
    let mut sorted = if whole { columns.to_vec() } else { vec![columns[1]] };
    sorted.sort_unstable();
    let history = dashboard_query::cost::storage(|| lanes::history(query.replay.recording(), stretch.run, table, &sorted, Keys::These(records), stretch.first, stretch.last))?;
    let of = |column: usize| -> HashMap<u128, Vec<(i32, i32, Option<Cell>)>> { history.runs(column).unwrap_or_default().into_iter().collect() };
    let [mut value, mut balance, mut trending, mut range] = columns.map(of);
    let out = records
        .iter()
        .map(|key| {
            let runs = RecordRuns {
                value: value.remove(key).unwrap_or_default(),
                balance: balance.remove(key).unwrap_or_default(),
                trending: trending.remove(key).unwrap_or_default(),
                range: range.remove(key).unwrap_or_default(),
            };
            (*key, runs)
        })
        .collect();
    Ok((out, history))
}

/// `runs` within `first..=last` holding a number, cut to it.
fn numbers(runs: &[(i32, i32, Option<Cell>)], first: i32, last: i32) -> Vec<(i32, i32, i128)> {
    runs.iter()
        .filter_map(|&(from, to, cell)| {
            let (from, to) = (from.max(first), to.min(last));
            match cell {
                Some(Cell::Number(value)) if from <= to => Some((from, to, i128::from(value))),
                _ => None,
            }
        })
        .collect()
}

/// `runs` within `first..=last` naming a token, cut to it: an empty text names none.
fn tokens(history: &History, runs: &[(i32, i32, Option<Cell>)], first: i32, last: i32) -> Vec<(i32, i32, String)> {
    runs.iter()
        .filter_map(|&(from, to, cell)| {
            let (from, to) = (from.max(first), to.min(last));
            let text = token(history.text(cell?)?);
            (from <= to && !text.is_empty()).then_some((from, to, text))
        })
        .collect()
}

/// Names and authored presentation from the recording's content stack.
struct Presentation<'a> {
    catalogue: Option<&'a Catalogue>,
    balances: Option<&'a installed_content_core::power_balance::Database>,
}

impl Presentation<'_> {
    fn name(&self, token: &str, names: &mut BTreeMap<String, String>, refused: &mut u64) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        if let Some(name) = names.get(token) {
            return Some(name.clone());
        }
        match self.catalogue.and_then(|catalogue| catalogue.localised(token, View::Plain)) {
            Some(text) if !(text.contains('[') && text.contains(']')) => {
                names.insert(token.to_owned(), text.clone());
                Some(text)
            }
            _ if self.catalogue.is_some() => {
                *refused += 1;
                None
            }
            _ => None,
        }
    }

    fn side(&self, bytes: &[u8], names: &mut BTreeMap<String, String>, refused: &mut u64) -> BalanceSide {
        let token = token(bytes);
        BalanceSide { name: self.name(&token, names, refused), token }
    }

    /// `token`'s sides and authored ranges, named.
    fn authored(&self, token: &str, names: &mut BTreeMap<String, String>, refused: &mut u64) -> (Option<BalanceSide>, Option<BalanceSide>, Vec<BalanceRange>) {
        let Some(balances) = self.balances else { return (None, None, Vec::new()) };
        let definition = balances.definition(token.as_bytes());
        let ranges = balances
            .ranges(token.as_bytes())
            .unwrap_or_default()
            .iter()
            .map(|range| {
                let token = super::token(&range.id);
                BalanceRange { name: self.name(&token, names, refused), token, min: Some(range.min.to_string()), max: Some(range.max.to_string()) }
            })
            .collect();
        (definition.map(|definition| self.side(&definition.left, names, refused)), definition.map(|definition| self.side(&definition.right, names, refused)), ranges)
    }
}

/// `runs` of text, adjacent runs of one text joined.
fn texts(runs: impl IntoIterator<Item = (i32, i32, String)>) -> Vec<PoliticsRun> {
    let mut out: Vec<PoliticsRun> = Vec::new();
    for (first, last, value) in runs {
        match out.last_mut() {
            Some(held) if held.value == value && i64::from(held.last_hour) + 1 == i64::from(first) => held.last_hour = last,
            _ => out.push(PoliticsRun { first_hour: first, last_hour: last, value }),
        }
    }
    out
}

/// What one holder's balances gathered over the stretches: per token its value, position, range
/// and trending runs, and the balance shown.
#[derive(Default)]
struct Gathered {
    values: BTreeMap<String, Vec<(i32, i32, i128)>>,
    records: BTreeMap<String, Vec<(i32, i32, i128)>>,
    ranges: BTreeMap<String, Vec<(i32, i32, String)>>,
    trending: BTreeMap<String, Vec<(i32, i32, String)>>,
    shown: Vec<(i32, i32, String)>,
}

/// For each piece of `memberships`' hours, the earliest record covering it: (first, last,
/// record, token).
pub(crate) fn earliest(memberships: &[(i32, i32, u32, String)]) -> Vec<(i32, i32, u32, String)> {
    let Some(first) = memberships.iter().map(|held| held.0).min() else { return Vec::new() };
    let last = memberships.iter().map(|held| held.1).max().unwrap_or(first);
    let spans: Vec<(i32, i32, i64)> = memberships.iter().map(|held| (held.0, held.1, 0)).collect();
    let mut out = Vec::new();
    for (from, to) in pieces(first, last, &[&spans]) {
        if let Some(held) = memberships.iter().filter(|held| held.0 <= from && to <= held.1).min_by_key(|held| held.2) {
            out.push((from, to, held.2, held.3.clone()));
        }
    }
    out
}

/// The balances of the holders `only` names (ascending), `whole` with their values, ranges and
/// leanings.
fn balances(
    query: &Query<'_>,
    held: &BTreeMap<u32, Vec<Holding>>,
    only: &[u32],
    whole: bool,
    catalogue: Option<Result<Arc<Catalogue>, String>>,
    reply: &mut PoliticsHistoryReply,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let ids = by_id(held);
    let mut gathered: BTreeMap<u32, Gathered> = BTreeMap::new();
    let (mut unresolved, mut unnamed) = (0_u64, 0_u64);
    for stretch in &query.timeline.stretches {
        check(cancelled)?;
        let members = stretch_runs(query, stretch, TableId::PowerBalanceMembers, "country")?;
        // The followed holders' memberships in the stretch: (first, last, record, holder slot).
        let mut held_by: Vec<(i32, i32, u32, u32)> = Vec::new();
        for (&key, runs) in &members {
            let record = (key >> 32) as u32;
            for (first, last, country) in within(Some(runs), stretch.first, stretch.last) {
                let Some(holdings) = u32::try_from(country).ok().and_then(|country| ids.get(&country)) else {
                    unresolved += 1;
                    continue;
                };
                for holding in holdings.iter().filter(|holding| only.binary_search(&holding.slot).is_ok()) {
                    let (from, to) = (first.max(holding.first), last.min(holding.last));
                    if from <= to {
                        held_by.push((from, to, record, holding.slot));
                    }
                }
            }
        }
        let mut records: Vec<u128> = held_by.iter().map(|held| u128::from(held.2)).collect();
        records.sort_unstable();
        records.dedup();
        if records.is_empty() {
            continue;
        }
        let (runs, history) = record_runs(query, stretch, &records, whole)?;
        // Each membership cut where its record's balance changes: (first, last, record, token).
        let mut local: BTreeMap<u32, Vec<(i32, i32, u32, String)>> = BTreeMap::new();
        for &(from, to, record, slot) in &held_by {
            let named = runs.get(&u128::from(record)).map(|runs| tokens(&history, &runs.balance, from, to)).unwrap_or_default();
            let covered: i64 = named.iter().map(|&(a, b, _)| i64::from(b) - i64::from(a) + 1).sum();
            if covered < i64::from(to) - i64::from(from) + 1 {
                unnamed += 1;
            }
            local.entry(slot).or_default().extend(named.into_iter().map(|(a, b, token)| (a, b, record, token)));
        }
        for (slot, memberships) in local {
            let holder = gathered.entry(slot).or_default();
            holder.shown.extend(earliest(&memberships).into_iter().map(|(first, last, _, token)| (first, last, token)));
            let mut by_token: BTreeMap<&str, Vec<(i32, i32, u32, String)>> = BTreeMap::new();
            for held in &memberships {
                by_token.entry(held.3.as_str()).or_default().push(held.clone());
            }
            for (token, held) in by_token {
                // Two records listing the holder under one token: the earlier, as the game reads it.
                for (first, last, record, _) in earliest(&held) {
                    holder.records.entry(token.to_owned()).or_default().push((first, last, i128::from(record)));
                    let Some(runs) = runs.get(&u128::from(record)) else { continue };
                    holder.values.entry(token.to_owned()).or_default().extend(numbers(&runs.value, first, last));
                    holder.ranges.entry(token.to_owned()).or_default().extend(tokens(&history, &runs.range, first, last));
                    holder.trending.entry(token.to_owned()).or_default().extend(tokens(&history, &runs.trending, first, last));
                }
            }
        }
    }
    reply.known = joined(query.timeline.stretches.iter().map(|stretch| (stretch.first, stretch.last)).collect());
    let catalogue = match catalogue {
        Some(Ok(catalogue)) => Some(catalogue),
        Some(Err(error)) => {
            reply.warnings.push(format!("1 refused balance names: {error}"));
            None
        }
        None => None,
    };
    let effects = catalogue.as_deref().map(|catalogue| catalogue.scripted_content().and_then(|content| content.effects()));
    if let Some(Err(error)) = &effects {
        reply.warnings.push(format!("1 refused balance ranges: {error}"));
    }
    let presentation = Presentation {
        catalogue: catalogue.as_deref(),
        balances: effects.as_ref().and_then(|effects| effects.as_ref().ok()).and_then(|effects| effects.view(None).power_balances),
    };
    let mut refused = 0_u64;
    for (slot, mut holder) in gathered {
        let mut balances = Vec::new();
        for (token, mut values) in std::mem::take(&mut holder.values) {
            values.sort_unstable_by_key(|run| run.0);
            let mut records = holder.records.remove(&token).unwrap_or_default();
            records.sort_unstable_by_key(|run| run.0);
            let mut held_ranges = holder.ranges.remove(&token).unwrap_or_default();
            held_ranges.sort_by_key(|run| run.0);
            let mut trending = holder.trending.remove(&token).unwrap_or_default();
            trending.sort_by_key(|run| run.0);
            let (left, right, ranges) = presentation.authored(&token, &mut reply.names, &mut refused);
            balances.push(BalanceHistory {
                name: presentation.name(&token, &mut reply.names, &mut refused),
                token,
                left,
                right,
                ranges,
                values: decimal(values),
                record: decimal(records),
                range: texts(held_ranges),
                trending: texts(trending),
            });
        }
        holder.shown.sort_by_key(|run| run.0);
        reply.holder_balances.push(HolderBalances {
            holder: slot.to_string(),
            tags: held.get(&slot).map(|held| tags_of(held)).unwrap_or_default(),
            balances,
            shown: texts(holder.shown),
        });
    }
    if refused != 0 {
        reply.warnings.push(format!("{refused} refused balance names: no plain localisation in the recorded content"));
    }
    if unresolved != 0 {
        reply.warnings.push(format!("{unresolved} refused balance members: the member has no identity"));
    }
    if unnamed != 0 {
        reply.warnings.push(format!("{unnamed} refused balance memberships: the record's balance was not read"));
    }
    Ok(())
}

