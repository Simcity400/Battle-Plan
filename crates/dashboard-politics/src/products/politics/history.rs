
use dashboard_effects::ScriptedEffects;
use dashboard_base::products::Country;
use super::{
    HolderTag, Holding, PoliticsRun, by_id, decimal, holdings, pieces,
    state::{BalanceRange, BalanceSide, NameScope, resolve_balance_name},
    stretch_runs, tags_of, token, value_at, within,
};
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{LoadContext, Product, check_reply_budget}};
use dashboard_base::span::HourSpan;
use dashboard_query::{Query, timeline::Stretch};
use dashboard_text::scripted_text::ScriptedText;
use replay_lanes::lanes::{self, Cell, History, Keys};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PoliticsHistoryMode {
    Country,
    Elections,
    Memberships,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoliticsHistoryParams {
    pub branch_id: String,
    pub mode: PoliticsHistoryMode,
    #[doc = " The country, for [`PoliticsHistoryMode::Country`]."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub country: Option<Country>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ElectionHistory {
    pub tags: Vec<HolderTag>,
    #[doc = r#" "1" when the game holds elections, "0" when it does not."#]
    pub held: Vec<PoliticsRun>,
    #[doc = r#" "1" while its schedule was read but its eligibility was not: unknown, not denied."#]
    pub refused: Vec<PoliticsRun>,
    #[doc = " Signed months."]
    pub interval: Vec<PoliticsRun>,
    #[doc = " The next election's hour."]
    pub next: Vec<PoliticsRun>,
    #[doc = " The term's start: the stored last election's hour, where it is a date."]
    pub last: Vec<PoliticsRun>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HolderBalances {
    pub holder: String,
    pub tags: Vec<HolderTag>,
    pub balances: Vec<BalanceHistory>,
    #[doc = " The balance the game shows the holder, by token."]
    pub shown: Vec<PoliticsRun>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BalanceHistory {
    pub token: String,
    pub name: Option<String>,
    pub left: Option<BalanceSide>,
    pub right: Option<BalanceSide>,
    #[doc = " The authored ranges in authored order; empty when content refuses."]
    pub ranges: Vec<BalanceRange>,
    #[doc = " Fixed5 values while the holder belonged to it."]
    pub values: Vec<PoliticsRun>,
    #[doc = " The record's stored position: holders sharing it at an hour share that record. Not an"]
    #[doc = " identity across hours."]
    pub record: Vec<PoliticsRun>,
    #[doc = " The authored range the record is in, by token, where it names one."]
    pub range: Vec<PoliticsRun>,
    #[doc = " The side the record is moving toward, by token, where it names one."]
    pub trending: Vec<PoliticsRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PoliticsHistoryReply {
    pub branch_id: String,
    pub mode: PoliticsHistoryMode,
    #[doc = " The branch's last hour: the history reads through it."]
    pub last_hour: Option<i32>,
    pub hour: Option<i32>,
    pub next_change: Option<i32>,
    pub unread: Option<String>,
    pub selected: Option<String>,
    pub tag: Option<String>,
    #[doc = " The country the request named, which a reply is matched to: a slot follows its country through any tag."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional = nullable))]
    pub asked: Option<dashboard_base::products::Country>,
    #[doc = " The hours the mode's table was read; others are unknown."]
    pub known: Vec<HourSpan>,
    pub elections: Vec<ElectionHistory>,
    #[doc = " Content tokens to their in-game names, where they resolve."]
    pub names: BTreeMap<String, String>,
    #[doc = " Every holder that held the country's tag, its balances whole."]
    pub holder_balances: Vec<HolderBalances>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PoliticsHistoryReply);

pub struct PoliticsStateHistory;

impl Product for PoliticsStateHistory {
    const KEY: &'static str = "politics-state-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PoliticsHistoryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, mode, country, hour];
    type Reply = PoliticsHistoryReply;

    #[cfg(any(test, feature = "export-types"))]
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
            PoliticsHistoryMode::Country => {
                params.hour.ok_or("A country's balance history names its hour")?;
                dashboard_base::products::validate_country(params.country.as_ref().ok_or("Missing country")?)
            }
            _ => Ok(()),
        }
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid politics history branch"))?)?;
        let catalogue = matches!(params.mode, PoliticsHistoryMode::Country)
            .then(|| context.content().and_then(|content| catalogue::catalogue(source, content)).map_err(|error| error.to_string()));
        let reply = match (params.mode, params.country.as_ref(), params.hour) {
            (PoliticsHistoryMode::Country, Some(country), Some(hour)) => {
                let index = indexed(source, &query, country, cancelled)?;
                country_reply(&query, params, &index, hour, catalogue)
            }
            _ => answer(&query, params, catalogue, cancelled)?,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

const KEPT: usize = 8;

#[derive(Default)]
struct Kept(Vec<((u64, Country), Arc<Mutex<Option<Arc<BalanceIndex>>>>)>);

fn indexed(source: &Source, query: &Query<'_>, country: &Country, cancelled: &AtomicBool) -> io::Result<Arc<BalanceIndex>> {
    let slot = {
        let state = source.state::<Kept>();
        let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (query.timeline.id, country.clone());
        let entry = match kept.0.iter().position(|(held, _)| *held == key) {
            Some(at) => kept.0.remove(at),
            None => (key, Arc::new(Mutex::new(None))),
        };
        let slot = Arc::clone(&entry.1);
        kept.0.push(entry);
        if kept.0.len() > KEPT {
            kept.0.remove(0);
        }
        slot
    };
    let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(index) = held.as_ref() {
        return Ok(Arc::clone(index));
    }
    let index = Arc::new(balance_index(query, &holdings(&query.holders), country, cancelled)?);
    *held = Some(Arc::clone(&index));
    Ok(index)
}

fn shell(query: &Query<'_>, params: &PoliticsHistoryParams) -> PoliticsHistoryReply {
    PoliticsHistoryReply {
        branch_id: params.branch_id.clone(),
        mode: params.mode,
        last_hour: Some(query.timeline.last),
        hour: None,
        next_change: None,
        unread: None,
        selected: None,
        tag: params.country.as_ref().map(|country| query.holders.nearest_name(country, query.timeline.last)),
        asked: params.country.clone(),
        known: Vec::new(),
        elections: Vec::new(),
        names: BTreeMap::new(),
        holder_balances: Vec::new(),
        warnings: Vec::new(),
    }
}

fn country_reply(query: &Query<'_>, params: &PoliticsHistoryParams, index: &BalanceIndex, hour: i32, catalogue: Option<Result<Arc<Catalogue>, String>>) -> PoliticsHistoryReply {
    let mut reply = shell(query, params);
    let hour = hour.min(query.timeline.last);
    reply.tag = params.country.as_ref().map(|country| query.holders.nearest_name(country, hour));
    balances_at(index, &holdings(&query.holders), hour, catalogue, &mut reply);
    reply
}

pub fn answer(query: &Query<'_>, params: &PoliticsHistoryParams, catalogue: Option<Result<Arc<Catalogue>, String>>, cancelled: &AtomicBool) -> io::Result<PoliticsHistoryReply> {
    let mut reply = shell(query, params);
    let held = holdings(&query.holders);
    match params.mode {
        PoliticsHistoryMode::Elections => elections(query, &held, &mut reply, cancelled)?,
        PoliticsHistoryMode::Country => {
            let country = params.country.as_ref().ok_or_else(|| dashboard_core::invalid("A country history names its country"))?;
            let hour = params.hour.ok_or_else(|| dashboard_core::invalid("A country's balance history names its hour"))?;
            let index = balance_index(query, &held, country, cancelled)?;
            return Ok(country_reply(query, params, &index, hour, catalogue));
        }
        PoliticsHistoryMode::Memberships => {
            let holders: Vec<u32> = held.keys().copied().collect();
            let gathered = gather(query, &held, &holders, false, cancelled)?;
            reply.known = joined(query.timeline.stretches.iter().map(|stretch| (stretch.first, stretch.last)).collect());
            present(gathered.holders, &held, catalogue, &mut reply, gathered.unresolved.len(), gathered.unnamed.len());
        }
    }
    Ok(reply)
}

fn check(cancelled: &AtomicBool) -> io::Result<()> {
    dashboard_text::metadata::check_cancel(cancelled)
}

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
                    let cached = value_at(&cache, from).map(|value| Some(value as i32));
                    if let Some(next) = crate::products::country_people::next_election(last_election, months, cached) {
                        parts.next.push((from, to, i128::from(next)));
                    }
                    if !engine_native_politics::cpolitical_status::needs_election_cache(last_election) {
                        parts.last.push((from, to, i128::from(last_election)));
                    }
                }
            }
        }
    }
    reply.known = joined(known);
    for (slot, mut parts) in by_slot {
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

struct RecordRuns {
    value: Vec<(i32, i32, Option<Cell>)>,
    balance: Vec<(i32, i32, Option<Cell>)>,
    trending: Vec<(i32, i32, Option<Cell>)>,
    range: Vec<(i32, i32, Option<Cell>)>,
}

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

fn tokens(history: &History, runs: &[(i32, i32, Option<Cell>)], first: i32, last: i32) -> Vec<(i32, i32, String)> {
    runs.iter()
        .filter_map(|&(from, to, cell)| {
            let (from, to) = (from.max(first), to.min(last));
            let text = token(history.text(cell?)?);
            (from <= to && !text.is_empty()).then_some((from, to, text))
        })
        .collect()
}

struct Presentation<'a> {
    catalogue: Option<&'a Catalogue>,
    catalogue_reason: Option<&'a str>,
    balances: Option<&'a content_reader_politics::power_balance::Database>,
}

impl<'a> Presentation<'a> {
    fn resolve(
        &self,
        frame: Option<&'a DecodedFrame>,
        scripted: &mut Option<Result<ScriptedText<'a>, String>>,
        token: &str,
        scope: NameScope,
        names: &mut BTreeMap<String, String>,
        refused: &mut u64,
    ) -> (Option<String>, Option<String>) {
        if token.is_empty() {
            return (None, None);
        }
        if let Some(name) = names.get(token) {
            return (Some(name.clone()), None);
        }
        let is_scripted = self.catalogue.and_then(|catalogue| raw_scripted_text(catalogue, token)).is_some();
        let (resolved, reason) = resolve_balance_name(self.catalogue, self.catalogue_reason, frame, scripted, token, scope);
        if let Some(text) = &resolved {
            if !is_scripted {
                names.insert(token.to_owned(), text.clone());
            }
        } else if self.catalogue.is_some() {
            *refused += 1;
        }
        (resolved, reason)
    }

    fn side(
        &self,
        frame: Option<&'a DecodedFrame>,
        scripted: &mut Option<Result<ScriptedText<'a>, String>>,
        bytes: &[u8],
        names: &mut BTreeMap<String, String>,
        refused: &mut u64,
    ) -> BalanceSide {
        let token = token(bytes);
        let (name, reason) = self.resolve(frame, scripted, &token, NameScope::Empty, names, refused);
        BalanceSide { token, name, reason }
    }

    fn authored(
        &self,
        frame: Option<&'a DecodedFrame>,
        scripted: &mut Option<Result<ScriptedText<'a>, String>>,
        token: &str,
        slot: u32,
        names: &mut BTreeMap<String, String>,
        refused: &mut u64,
    ) -> (Option<BalanceSide>, Option<BalanceSide>, Vec<BalanceRange>) {
        let Some(balances) = self.balances else { return (None, None, Vec::new()) };
        let definition = balances.definition(token.as_bytes());
        let ranges = balances
            .ranges(token.as_bytes())
            .unwrap_or_default()
            .iter()
            .map(|range| {
                let token = super::token(&range.id);
                let (name, reason) = self.resolve(frame, scripted, &token, NameScope::Country(Some(slot)), names, refused);
                BalanceRange { token, name, reason, min: Some(range.min.to_string()), max: Some(range.max.to_string()) }
            })
            .collect();
        (
            definition.map(|definition| self.side(frame, scripted, &definition.left, names, refused)),
            definition.map(|definition| self.side(frame, scripted, &definition.right, names, refused)),
            ranges,
        )
    }
}

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

#[derive(Clone, Default)]
pub(crate) struct Gathered {
    values: BTreeMap<String, Vec<(i32, i32, i128)>>,
    records: BTreeMap<String, Vec<(i32, i32, i128)>>,
    ranges: BTreeMap<String, Vec<(i32, i32, String)>>,
    trending: BTreeMap<String, Vec<(i32, i32, String)>>,
    shown: Vec<(i32, i32, String)>,
}

fn clip<T: Clone>(runs: &[(i32, i32, T)], hour: i32) -> Vec<(i32, i32, T)> {
    runs.iter().filter(|run| run.0 <= hour).map(|(first, last, value)| (*first, (*last).min(hour), value.clone())).collect()
}

impl Gathered {
    fn sort(&mut self) {
        for runs in self.values.values_mut().chain(self.records.values_mut()) {
            runs.sort_unstable_by_key(|run| run.0);
        }
        for runs in self.ranges.values_mut().chain(self.trending.values_mut()) {
            runs.sort_by_key(|run| run.0);
        }
        self.shown.sort_by_key(|run| run.0);
    }

    fn through(&self, hour: i32) -> Self {
        let numbers = |map: &BTreeMap<String, Vec<(i32, i32, i128)>>| map.iter().map(|(token, runs)| (token.clone(), clip(runs, hour))).filter(|(_, runs)| !runs.is_empty()).collect();
        let words = |map: &BTreeMap<String, Vec<(i32, i32, String)>>| map.iter().map(|(token, runs)| (token.clone(), clip(runs, hour))).collect();
        Self { values: numbers(&self.values), records: numbers(&self.records), ranges: words(&self.ranges), trending: words(&self.trending), shown: clip(&self.shown, hour) }
    }

    fn edges(&self, out: &mut Vec<i32>) {
        let mut push = |first: i32, last: i32| {
            out.push(first);
            out.push(last.saturating_add(1));
        };
        for runs in self.values.values().chain(self.records.values()) {
            runs.iter().for_each(|run| push(run.0, run.1));
        }
        for runs in self.ranges.values().chain(self.trending.values()) {
            runs.iter().for_each(|run| push(run.0, run.1));
        }
        self.shown.iter().for_each(|run| push(run.0, run.1));
    }
}

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

#[derive(Default)]
pub(crate) struct Gather {
    holders: BTreeMap<u32, Gathered>,
    unresolved: Vec<i32>,
    unnamed: Vec<i32>,
    read: Vec<(i32, i32)>,
}

fn complete_spans(query: &Query<'_>, stretch: &Stretch, table: TableId) -> io::Result<Vec<(i32, i32)>> {
    let statuses = dashboard_query::cost::storage(|| lanes::statuses(query.replay.recording(), stretch.run, table, stretch.first, stretch.last))?;
    Ok(statuses.into_iter().filter(|run| replay_recording::columnar_frames::complete(run.status)).map(|run| (run.first, run.last)).collect())
}

fn both_read(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        let (first, last) = (a[i].0.max(b[j].0), a[i].1.min(b[j].1));
        if first <= last {
            out.push((first, last));
        }
        if a[i].1 < b[j].1 { i += 1 } else { j += 1 }
    }
    out
}

fn gather(query: &Query<'_>, held: &BTreeMap<u32, Vec<Holding>>, only: &[u32], whole: bool, cancelled: &AtomicBool) -> io::Result<Gather> {
    let ids = by_id(held);
    let mut out = Gather::default();
    for stretch in &query.timeline.stretches {
        check(cancelled)?;
        if whole {
            let read = both_read(&complete_spans(query, stretch, TableId::PowerBalances)?, &complete_spans(query, stretch, TableId::PowerBalanceMembers)?);
            out.read.extend(read);
        }
        let members = stretch_runs(query, stretch, TableId::PowerBalanceMembers, "country")?;
        let mut held_by: Vec<(i32, i32, u32, u32)> = Vec::new();
        for (&key, runs) in &members {
            let record = (key >> 32) as u32;
            for (first, last, country) in within(Some(runs), stretch.first, stretch.last) {
                let Some(holdings) = u32::try_from(country).ok().and_then(|country| ids.get(&country)) else {
                    out.unresolved.push(first);
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
        let mut local: BTreeMap<u32, Vec<(i32, i32, u32, String)>> = BTreeMap::new();
        for &(from, to, record, slot) in &held_by {
            let named = runs.get(&u128::from(record)).map(|runs| tokens(&history, &runs.balance, from, to)).unwrap_or_default();
            let covered: i64 = named.iter().map(|&(a, b, _)| i64::from(b) - i64::from(a) + 1).sum();
            if covered < i64::from(to) - i64::from(from) + 1 {
                out.unnamed.push(from);
            }
            local.entry(slot).or_default().extend(named.into_iter().map(|(a, b, token)| (a, b, record, token)));
        }
        for (slot, memberships) in local {
            let holder = out.holders.entry(slot).or_default();
            holder.shown.extend(earliest(&memberships).into_iter().map(|(first, last, _, token)| (first, last, token)));
            let mut by_token: BTreeMap<&str, Vec<(i32, i32, u32, String)>> = BTreeMap::new();
            for held in &memberships {
                by_token.entry(held.3.as_str()).or_default().push(held.clone());
            }
            for (token, held) in by_token {
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
    for holder in out.holders.values_mut() {
        holder.sort();
    }
    out.unresolved.sort_unstable();
    out.unnamed.sort_unstable();
    Ok(out)
}

pub(crate) struct BalanceIndex {
    gather: Gather,
    runs: Vec<(i32, i32)>,
    boundaries: Vec<i32>,
}

pub(crate) fn balance_index(query: &Query<'_>, held: &BTreeMap<u32, Vec<Holding>>, country: &Country, cancelled: &AtomicBool) -> io::Result<BalanceIndex> {
    let mut holders: Vec<u32> = held.iter().filter(|(_, held)| held.iter().any(|holding| dashboard_world::products::holder_tags::matches(country, (holding.slot, holding.id), &holding.tag))).map(|(&slot, _)| slot).collect();
    holders.sort_unstable();
    let gather = gather(query, held, &holders, true, cancelled)?;
    let runs: Vec<(i32, i32)> = holders.iter().flat_map(|slot| held.get(slot).into_iter().flatten()).map(|holding| (holding.first, holding.last)).collect();
    let mut boundaries = Vec::new();
    for holder in gather.holders.values() {
        holder.edges(&mut boundaries);
    }
    for &(first, last) in gather.read.iter().chain(&runs) {
        boundaries.push(first);
        boundaries.push(last.saturating_add(1));
    }
    boundaries.extend(gather.unresolved.iter().chain(&gather.unnamed));
    boundaries.sort_unstable();
    boundaries.dedup();
    Ok(BalanceIndex { gather, runs, boundaries })
}

const UNREAD_BEFORE: &str = "The balance of power was not read for part of this campaign before this date";

impl BalanceIndex {
    fn unread_through(&self, hour: i32, known: &[HourSpan]) -> bool {
        let held: i64 = joined(self.runs.iter().map(|&(first, last)| (first, last.min(hour))).filter(|(first, last)| first <= last).collect())
            .iter()
            .map(|span| i64::from(span.last_hour) - i64::from(span.first_hour) + 1)
            .sum();
        let read: i64 = known.iter().map(|span| i64::from(span.last_hour) - i64::from(span.first_hour) + 1).sum();
        read < held
    }
}

fn selected(holder: &Gathered, hour: i32) -> Option<String> {
    let at = holder.shown.partition_point(|run| run.0 <= hour);
    holder.shown[..at].last().map(|run| run.2.clone()).or_else(|| holder.values.iter().min_by_key(|(_, runs)| runs[0].0).map(|(token, _)| token.clone()))
}

pub(crate) fn balances_at(index: &BalanceIndex, held: &BTreeMap<u32, Vec<Holding>>, hour: i32, catalogue: Option<Result<Arc<Catalogue>, String>>, reply: &mut PoliticsHistoryReply) {
    let known: Vec<(i32, i32)> = index.gather.read.iter().filter(|span| span.0 <= hour).map(|&(first, last)| (first, last.min(hour))).collect();
    reply.known = joined(known);
    reply.hour = Some(hour);
    reply.next_change = index.boundaries.get(index.boundaries.partition_point(|&at| at <= hour)).copied();
    reply.unread = index.unread_through(hour, &reply.known).then(|| UNREAD_BEFORE.to_owned());
    let holders: BTreeMap<u32, Gathered> = index.gather.holders.iter().map(|(&slot, holder)| (slot, holder.through(hour))).collect();
    reply.selected = holders.values().next().and_then(|holder| selected(holder, hour));
    let unresolved = index.gather.unresolved.partition_point(|&at| at <= hour);
    let unnamed = index.gather.unnamed.partition_point(|&at| at <= hour);
    present(holders, held, catalogue, reply, unresolved, unnamed);
}

fn present(gathered: BTreeMap<u32, Gathered>, held: &BTreeMap<u32, Vec<Holding>>, catalogue: Option<Result<Arc<Catalogue>, String>>, reply: &mut PoliticsHistoryReply, unresolved: usize, unnamed: usize) {
    let (catalogue, catalogue_reason) = match catalogue {
        Some(Ok(catalogue)) => (Some(catalogue), None),
        Some(Err(error)) => {
            reply.warnings.push(format!("1 refused balance names: {error}"));
            (None, Some(error))
        }
        None => (None, None),
    };
    let effects = catalogue.as_deref().map(|catalogue| catalogue.scripted_content().and_then(|content| content.effects()));
    if let Some(Err(error)) = &effects {
        reply.warnings.push(format!("1 refused balance ranges: {error}"));
    }
    let power_balances = effects.as_ref().and_then(|effects| effects.as_ref().ok()).and_then(|effects| effects.view(None).power_balances);
    let presentation = Presentation { catalogue: catalogue.as_deref(), catalogue_reason: catalogue_reason.as_deref(), balances: power_balances };
    let mut refused = 0_u64;
    let mut scripted = None;
    for (slot, mut holder) in gathered {
        let mut balances = Vec::new();
        for (token, values) in std::mem::take(&mut holder.values) {
            let records = holder.records.remove(&token).unwrap_or_default();
            let held_ranges = holder.ranges.remove(&token).unwrap_or_default();
            let trending = holder.trending.remove(&token).unwrap_or_default();
            let (left, right, ranges) = presentation.authored(None, &mut scripted, &token, slot, &mut reply.names, &mut refused);
            let (name, _) = presentation.resolve(None, &mut scripted, &token, NameScope::Empty, &mut reply.names, &mut refused);
            balances.push(BalanceHistory { name, token, left, right, ranges, values: decimal(values), record: decimal(records), range: texts(held_ranges), trending: texts(trending) });
        }
        reply.holder_balances.push(HolderBalances { holder: slot.to_string(), tags: held.get(&slot).map(|held| tags_of(held)).unwrap_or_default(), balances, shown: texts(holder.shown) });
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
}
