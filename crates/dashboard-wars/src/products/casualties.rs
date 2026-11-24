
use dashboard_base::products::Country;
use dashboard_world::products::{army_windows::windows, holder_tags::{HolderTags, Identity}};
use dashboard_base::products::{LoadContext, Product};
use super::wars::{self, WarRelationMembership, WarsReply};
use dashboard_base::{Source, span::HourSpan};
use dashboard_query::{Query, read};
use replay_columnar::summary::Source as Column;
use replay_data::rows::{CasualtyLifetimeRow, TableId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub const SEVERAL_WARS: &str = "The native view places this relation in multiple war episodes.";
pub const NO_WAR: &str = "The native war grouping was not recorded for this relation observation.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Lifetime {
    pub a: Identity,
    pub b: Identity,
    pub start: i32,
}

impl Lifetime {
    pub fn of(key: u128) -> Self {
        let field = |shift: u32| (key >> shift) as u32 & CasualtyLifetimeRow::FIELD;
        Self { a: (field(104), field(80)), b: (field(56), field(32)), start: key as u32 as i32 }
    }
}

fn holder_key((slot, id): Identity) -> u128 {
    dashboard_declare::Key::Holder.of(slot, id)
}

fn identity_text((slot, id): Identity) -> String {
    format!("{slot}:{id}")
}

fn value_at(runs: &read::Runs, hour: i32) -> Option<i64> {
    let at = runs.partition_point(|run| run.1 < hour);
    runs.get(at).filter(|run| run.0 <= hour).and_then(|run| run.2)
}

fn sorted(mut runs: HashMap<u128, read::Runs>) -> HashMap<u128, read::Runs> {
    for runs in runs.values_mut() {
        runs.sort_unstable_by_key(|run| run.0);
    }
    runs
}

fn within(spans: &[(i32, i32)], hour: i32) -> bool {
    let at = spans.partition_point(|span| span.1 < hour);
    spans.get(at).is_some_and(|span| span.0 <= hour)
}

#[derive(Default)]
pub struct WindowReads {
    pub taken_a: HashMap<u128, read::Runs>,
    pub taken_b: HashMap<u128, read::Runs>,
    pub partial: HashMap<u128, read::Runs>,
    pub observed: Vec<(i32, i32)>,
}

impl WindowReads {
    fn incomplete(&self, identity: Identity, hour: i32) -> bool {
        self.partial.get(&holder_key(identity)).and_then(|runs| value_at(runs, hour)).is_some_and(|flag| flag != 0)
    }

    fn edges(&self, keys: &[u128], identities: &[Identity], first: i32, last: i32) -> BTreeSet<i32> {
        let mut out = BTreeSet::from([first]);
        let mut add = |runs: &read::Runs| {
            for &(a, b, _) in runs {
                for edge in [a, b.saturating_add(1)] {
                    if first < edge && edge <= last {
                        out.insert(edge);
                    }
                }
            }
        };
        for key in keys {
            for held in [&self.taken_a, &self.taken_b] {
                if let Some(runs) = held.get(key) {
                    add(runs);
                }
            }
        }
        for &identity in identities {
            if let Some(runs) = self.partial.get(&holder_key(identity)) {
                add(runs);
            }
        }
        for &(a, b) in &self.observed {
            add(&vec![(a, b, None)]);
        }
        out
    }
}

fn pieces(edges: &BTreeSet<i32>, first: i32, last: i32) -> impl Iterator<Item = (i32, i32)> + '_ {
    let starts: Vec<i32> = edges.range(first..=last).copied().collect();
    (0..starts.len()).map(move |at| (starts[at], starts.get(at + 1).map_or(last, |next| next - 1)))
}

fn held_at(runs: &read::Runs, gone: i64, hour: i32) -> Option<i64> {
    value_at(runs, i64::from(hour).min(gone) as i32)
}

fn push_run<T>(runs: &mut Vec<T>, run: T, joins: impl Fn(&mut T, &T) -> bool) {
    if let Some(last) = runs.last_mut()
        && joins(last, &run)
    {
        return;
    }
    runs.push(run);
}

dashboard_base::reply_warnings!(CasualtyBreakdownReply);

struct Window {
    first: i32,
    last: i32,
    stretches: Vec<(u32, i32, i32)>,
}

fn window(query: &Query<'_>, spans: &[HourSpan], index: usize) -> io::Result<Window> {
    let span = spans.get(index).ok_or_else(|| invalid("Unknown casualty history window"))?;
    let (first, last) = (span.first_hour, span.last_hour);
    let stretches: Vec<(u32, i32, i32)> = query.timeline.within(first, last).map(|stretch| (stretch.run, stretch.first, stretch.last)).collect();
    if stretches.is_empty() {
        return Err(invalid("The window's hours are not recorded"));
    }
    Ok(Window { first, last, stretches })
}

fn stretch_lifetimes(source: &Source, run: u32, last: i32) -> io::Result<Vec<(u128, i64, i64)>> {
    Ok(dashboard_base::queries::lifetimes(source, run, TableId::CasualtyLifetimes)?
        .iter()
        .filter(|&&(_, born, _)| born <= i64::from(last))
        .copied()
        .collect())
}

fn stretch_reads(query: &Query<'_>, run: u32, first: i32, last: i32, lives: &[(u128, i64, i64)], identities: &[Identity], ended: &EndedCounters) -> io::Result<WindowReads> {
    let recording = query.replay.recording();
    let column = |table: TableId, name: &str| dashboard_declare::field(table, name).map(Column::Cell);
    let mut reads = WindowReads {
        observed: replay_lanes::lanes::available_reads(recording, run, TableId::CasualtyLifetimes, first, last)?,
        ..WindowReads::default()
    };
    let keys: Vec<u128> = lives.iter().filter(|&&(_, _, gone)| gone >= i64::from(first)).map(|life| life.0).collect();
    if !keys.is_empty() {
        reads.taken_a = sorted(read::runs(recording, run, TableId::CasualtyLifetimes, column(TableId::CasualtyLifetimes, "taken_a")?, Some(&keys), first, last)?);
        reads.taken_b = sorted(read::runs(recording, run, TableId::CasualtyLifetimes, column(TableId::CasualtyLifetimes, "taken_b")?, Some(&keys), first, last)?);
    }
    for &(key, _, gone) in lives.iter().filter(|&&(_, _, gone)| gone < i64::from(first)) {
        let (a, b) = ended.0.get(&key).copied().unwrap_or((None, None));
        let hour = gone as i32;
        reads.taken_a.insert(key, vec![(hour, hour, a)]);
        reads.taken_b.insert(key, vec![(hour, hour, b)]);
    }
    let mut holders: Vec<u128> = identities.iter().map(|&identity| holder_key(identity)).collect();
    holders.sort_unstable();
    holders.dedup();
    if !holders.is_empty() {
        reads.partial = sorted(read::runs(recording, run, TableId::CasualtyTotals, column(TableId::CasualtyTotals, "partial")?, Some(&holders), first, last)?);
    }
    Ok(reads)
}

#[derive(Default)]
pub struct EndedCounters(pub HashMap<u128, (Option<i64>, Option<i64>)>);

pub fn run_ends(source: &Source, run: u32) -> io::Result<EndedCounters> {
    let recording = source.replay_ref().recording();
    let last = recording.run(run)?.last_hour();
    let lifetimes = dashboard_base::queries::lifetimes(source, run, TableId::CasualtyLifetimes)?;
    let mut asks: Vec<(i32, u128)> = lifetimes.iter().filter(|&&(_, _, gone)| gone < i64::from(last)).map(|&(key, _, gone)| (gone as i32, key)).collect();
    asks.sort_unstable();
    let mut out = EndedCounters::default();
    if asks.is_empty() {
        return Ok(out);
    }
    let columns = [dashboard_declare::field(TableId::CasualtyLifetimes, "taken_a")?, dashboard_declare::field(TableId::CasualtyLifetimes, "taken_b")?];
    replay_lanes::lanes::at_each(recording, run, TableId::CasualtyLifetimes, Some(&columns), &asks, &mut |_, key, row| {
        let [a, b] = columns.map(|column| row.as_ref().and_then(|(view, slot)| view.number(column, *slot)));
        out.0.insert(key, (a, b));
        Ok(())
    })?;
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grouping {
    pub first: i32,
    pub last: i32,
    pub war: Option<String>,
    pub ambiguous: bool,
}

pub(crate) fn lifetime_groupings<'a>(wars: impl IntoIterator<Item = (&'a str, &'a [WarRelationMembership])>, keys: &HashSet<u128>) -> HashMap<u128, Vec<Grouping>> {
    let mut changes: HashMap<u128, BTreeMap<i64, Vec<(&'a str, i32)>>> = HashMap::new();
    for (war, held) in wars {
        for membership in held {
            let key = membership.key.lifetime();
            if keys.contains(&key) {
                let at = changes.entry(key).or_default();
                at.entry(i64::from(membership.first_hour)).or_default().push((war, 1));
                at.entry(i64::from(membership.last_hour) + 1).or_default().push((war, -1));
            }
        }
    }
    changes
        .into_iter()
        .map(|(key, at)| {
            let mut active: BTreeMap<&str, i32> = BTreeMap::new();
            let mut groupings = Vec::new();
            let mut at = at.into_iter().peekable();
            while let Some((first, events)) = at.next() {
                for (war, delta) in events {
                    let count = active.entry(war).or_default();
                    *count += delta;
                    if *count == 0 {
                        active.remove(war);
                    }
                }
                let Some(&(next, _)) = at.peek() else { continue };
                let war = match active.len() {
                    1 => active.keys().next().map(|war| (*war).to_owned()),
                    _ => None,
                };
                groupings.push(Grouping { first: first as i32, last: (next - 1) as i32, war, ambiguous: active.len() > 1 });
            }
            (key, groupings)
        })
        .collect()
}

fn grouping_at(groupings: &[Grouping], hour: i32) -> Option<&Grouping> {
    let at = groupings.partition_point(|run| run.last < hour);
    groupings.get(at).filter(|run| run.first <= hour)
}

fn spans(runs: &[(i32, i32)]) -> Vec<HourSpan> {
    runs.iter().map(|&(first_hour, last_hour)| HourSpan { first_hour, last_hour }).collect()
}

fn branch_observed(query: &Query<'_>) -> io::Result<Vec<HourSpan>> {
    let mut out = Vec::new();
    for stretch in &query.timeline.stretches {
        out.extend(spans(&replay_lanes::lanes::available_reads(query.replay.recording(), stretch.run, TableId::CasualtyLifetimes, stretch.first, stretch.last)?));
    }
    Ok(out)
}

fn manifest(query: &Query<'_>) -> io::Result<Vec<HourSpan>> {
    Ok(windows(query)?.into_iter().map(|span| HourSpan { first_hour: span.first_hour, last_hour: span.last_hour }).collect())
}

fn branch(text: &str) -> Result<u64, &'static str> {
    text.parse::<u64>().map_err(|_| "Invalid branch")
}


pub struct CasualtyBreakdown;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasualtyBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    pub window: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
    pub hourly: bool,
    pub grouping_hour: i32,
    pub grouping_hourly: bool,
    pub enemy_hour: i32,
    pub enemy_hourly: bool,
    #[doc = " What the country took in this lifetime."]
    pub value: String,
    #[doc = " The war the lifetime belongs to; null with `reason` when not known."]
    pub war_id: Option<String>,
    pub reason: Option<String>,
    pub incomplete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownSeries {
    pub id: String,
    pub country_identity: String,
    pub enemy_tag: String,
    pub enemy_identity: String,
    pub runs: Vec<CasualtyBreakdownRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownCountryRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub identity: String,
    pub incomplete: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyWarName {
    pub text: String,
    pub known: bool,
    pub reason: Option<String>,
    pub key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyWarNameRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: CasualtyWarName,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownWar {
    pub id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub started_by: bool,
    pub ended_by: bool,
    pub names: Vec<CasualtyWarNameRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownReply {
    pub branch_id: String,
    pub tag: String,
    pub available: bool,
    pub windows: Vec<HourSpan>,
    pub window: Option<usize>,
    pub observed: Vec<HourSpan>,
    #[doc = " The identities holding the tag within the observed hours."]
    pub countries: Vec<CasualtyBreakdownCountryRun>,
    pub series: Vec<CasualtyBreakdownSeries>,
    pub wars: Vec<CasualtyBreakdownWar>,
    pub has_gap: bool,
    pub warnings: Vec<String>,
}

pub fn fold_breakdown(
    tag: &Country,
    lifetimes: &[(u128, i64, i64)],
    reads: &WindowReads,
    tags: &HolderTags,
    first: i32,
    last: i32,
    groupings: &HashMap<u128, Vec<Grouping>>,
) ->(Vec<CasualtyBreakdownCountryRun>, Vec<CasualtyBreakdownSeries>) {
    let holding = tags.holding(tag, first, last);
    let mut countries: Vec<CasualtyBreakdownCountryRun> = Vec::new();
    for &(identity, a, b) in &holding {
        let mut edges = reads.edges(&[], &[identity], a, b);
        edges.insert(a);
        for (from, to) in pieces(&edges, a, b) {
            if !within(&reads.observed, from) {
                continue;
            }
            let run = CasualtyBreakdownCountryRun { first_hour: from, last_hour: to, identity: identity_text(identity), incomplete: reads.incomplete(identity, from) };
            push_run(&mut countries, run, |held, next| {
                let joins = i64::from(held.last_hour) + 1 == i64::from(next.first_hour) && (&held.identity, held.incomplete) == (&next.identity, next.incomplete);
                if joins {
                    held.last_hour = next.last_hour;
                }
                joins
            });
        }
    }
    let mut series: BTreeMap<(u128, String), CasualtyBreakdownSeries> = BTreeMap::new();
    for &(key, born, gone) in lifetimes {
        let lifetime = Lifetime::of(key);
        for &(identity, a, b) in &holding {
            let (enemy, held) = if lifetime.a == identity {
                (lifetime.b, &reads.taken_a)
            } else if lifetime.b == identity {
                (lifetime.a, &reads.taken_b)
            } else {
                continue;
            };
            let (a, b) = (i64::from(a).max(born) as i32, b);
            if a > b {
                continue;
            }
            let Some(runs) = held.get(&key) else { continue };
            let grouped = groupings.get(&key).map(Vec::as_slice).unwrap_or_default();
            let mut edges = reads.edges(&[key], &[identity], a, b);
            tags.edges(enemy, a, b, &mut edges);
            let ended = gone.saturating_add(1);
            for edge in grouped.iter().flat_map(|run| [i64::from(run.first), i64::from(run.last) + 1]).chain([ended]) {
                if i64::from(a) < edge && edge <= i64::from(b) && edge <= ended {
                    edges.insert(edge as i32);
                }
            }
            {
                for (from, to) in pieces(&edges, a, b) {
                    if !within(&reads.observed, from) {
                        continue;
                    }
                    let (Some(enemy_tag), Some(value)) = (tags.tag_at(enemy, from), held_at(runs, gone, from)) else { continue };
                    let id = format!("{key:x}|{enemy_tag}");
                    let entry = series.entry((key, enemy_tag.to_owned())).or_insert_with(|| CasualtyBreakdownSeries {
                        id,
                        country_identity: identity_text(identity),
                        enemy_tag: enemy_tag.to_owned(),
                        enemy_identity: identity_text(enemy),
                        runs: Vec::new(),
                    });
                    let living = i64::from(from) <= gone;
                    let grouping_hour = if living { from } else { gone as i32 };
                    let grouping = grouping_at(grouped, grouping_hour);
                    let war_id = grouping.and_then(|run| run.war.clone());
                    let why = if grouping.is_some_and(|run| run.ambiguous) { SEVERAL_WARS } else { NO_WAR };
                    let reason = war_id.is_none().then(|| why.to_owned());
                    let run = CasualtyBreakdownRun {
                        first_hour: from,
                        last_hour: to,
                        read_hour: from,
                        hourly: true,
                        grouping_hour,
                        grouping_hourly: living,
                        enemy_hour: from,
                        enemy_hourly: true,
                        value: value.to_string(),
                        war_id,
                        reason,
                        incomplete: reads.incomplete(identity, from),
                    };
                    push_run(&mut entry.runs, run, |held, next| {
                        let grouped_alike = (&held.war_id, &held.reason, held.grouping_hourly) == (&next.war_id, &next.reason, next.grouping_hourly)
                            && (next.grouping_hourly || held.grouping_hour == next.grouping_hour);
                        let joins = i64::from(held.last_hour) + 1 == i64::from(next.first_hour)
                            && (&held.value, held.incomplete) == (&next.value, next.incomplete)
                            && grouped_alike;
                        if joins {
                            held.last_hour = next.last_hour;
                        }
                        joins
                    });
                }
            }
        }
    }
    (countries, series.into_values().collect())
}

fn shown_wars(view: &WarsReply, series: &[CasualtyBreakdownSeries], last: i32) -> Vec<CasualtyBreakdownWar> {
    let used: HashSet<&str> = series.iter().flat_map(|series| &series.runs).filter_map(|run| run.war_id.as_deref()).collect();
    view.wars
        .iter()
        .filter(|war| used.contains(war.id.as_str()))
        .map(|war| CasualtyBreakdownWar {
            id: war.id.clone(),
            first_hour: war.first_hour,
            last_hour: war.last_hour.min(last),
            started_by: war.started_by,
            ended_by: war.ended_by || war.last_hour > last,
            names: war
                .observations
                .iter()
                .filter(|run| run.first_hour <= last)
                .map(|run| CasualtyWarNameRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour.min(last),
                    name: CasualtyWarName { text: run.name.text.clone(), known: run.name.known, reason: run.name.reason.clone(), key: run.name.key.clone() },
                })
                .collect(),
        })
        .collect()
}

impl Product for CasualtyBreakdown {
    const KEY: &'static str = "casualty-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CasualtyBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, window];
    type Reply = CasualtyBreakdownReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            CasualtyBreakdownParams::decl(),
            CasualtyBreakdownRun::decl(),
            CasualtyBreakdownSeries::decl(),
            CasualtyBreakdownCountryRun::decl(),
            CasualtyWarName::decl(),
            CasualtyWarNameRun::decl(),
            CasualtyBreakdownWar::decl(),
            HourSpan::decl(),
            CasualtyBreakdownReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        dashboard_base::products::validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch_id = branch(&params.branch_id).map_err(invalid)?;
        let query = dashboard_base::queries::query(source, branch_id)?;
        let windows = manifest(&query)?;
        let mut reply = CasualtyBreakdownReply {
            branch_id: params.branch_id.clone(),
            tag: query.holders.name(&params.country, query.timeline.last),
            available: true,
            windows: windows.clone(),
            window: params.window,
            observed: Vec::new(),
            countries: Vec::new(),
            series: Vec::new(),
            wars: Vec::new(),
            has_gap: false,
            warnings: Vec::new(),
        };
        let Some(index) = params.window else {
            reply.observed = branch_observed(&query)?;
            return Ok(reply);
        };
        let window = window(&query, &windows, index)?;
        dashboard_text::metadata::check_cancel(cancelled)?;
        let tags = HolderTags::new(&query);
        let mut per_stretch = Vec::with_capacity(window.stretches.len());
        let mut all_keys: HashSet<u128> = HashSet::new();
        for &(run, s_first, s_last) in &window.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let lifetimes = stretch_lifetimes(source, run, s_last)?;
            let holding: Vec<Identity> = tags.holding(&params.country, s_first, s_last).into_iter().map(|run| run.0).collect();
            let asked: Vec<(u128, i64, i64)> = lifetimes
                .iter()
                .filter(|&&(key, ..)| {
                    let lifetime = Lifetime::of(key);
                    holding.contains(&lifetime.a) || holding.contains(&lifetime.b)
                })
                .copied()
                .collect();
            all_keys.extend(asked.iter().map(|life| life.0));
            per_stretch.push((run, s_first, s_last, holding, asked));
        }
        let (groupings, view) = if all_keys.is_empty() {
            (HashMap::new(), None)
        } else {
            let view = wars::viewpoint_wars(source, branch_id, &params.country, context, cancelled)?;
            (lifetime_groupings(view.wars.iter().map(|war| (war.id.as_str(), war.relations.as_slice())), &all_keys), Some(view))
        };
        let mut observed_runs: Vec<(i32, i32)> = Vec::new();
        let mut countries: Vec<CasualtyBreakdownCountryRun> = Vec::new();
        let mut merged: BTreeMap<String, CasualtyBreakdownSeries> = BTreeMap::new();
        for (run, s_first, s_last, holding, asked) in per_stretch {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let ended = crate::products::casualties::cached_casualty_ends(source, run)?;
            let reads = stretch_reads(&query, run, s_first, s_last, &asked, &holding, &ended)?;
            observed_runs.extend_from_slice(&reads.observed);
            let (part_countries, part_series) = fold_breakdown(&params.country, &asked, &reads, &tags, s_first, s_last, &groupings);
            countries.extend(part_countries);
            for series in part_series {
                merged
                    .entry(series.id.clone())
                    .and_modify(|held| held.runs.extend(series.runs.iter().cloned()))
                    .or_insert(series);
            }
        }
        let series: Vec<CasualtyBreakdownSeries> = merged.into_values().collect();
        if let Some(view) = view {
            reply.wars = shown_wars(&view, &series, window.last);
            reply.warnings.extend(view.warnings.iter().cloned());
        }
        reply.countries = countries;
        reply.series = series;
        reply.has_gap = observed_runs != [(window.first, window.last)];
        reply.observed = spans(&observed_runs);
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

enum RunKey {}

type CachedRuns = dashboard_base::queries::RunCache<RunKey, EndedCounters>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    CachedRuns::forget(source);
}

pub fn cached_casualty_ends(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<EndedCounters>> {
    CachedRuns::held(&source.state::<CachedRuns>(), run, || run_ends(source, run))
}
