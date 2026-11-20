//! Casualties between countries, from the recorder's `CasualtyLifetimes` (one row per live
//! lifetime of a war relation between two country identities; an ended lifetime's row goes, and
//! its counters are its row's at its last hour) and `CasualtyTotals` (each
//! identity's sums; its `partial` flag says a lifetime's last hours were lost in a gap). A
//! country's own totals are the declared series `casualty-totals`; these products add what a
//! series cannot: the pairs (`casualty-pairs`), one country's losses by enemy and lifetime
//! (`casualty-breakdown`), and the content's name for losses no attacker dealt
//! (`casualty-labels`). The breakdown groups each lifetime by the war holding it in the asked
//! country's own view of its wars (the `wars` walk with that country as viewpoint).
//!
//! Both history products answer per replay block (a window): the manifest of windows, and one
//! window's runs when asked (its index names one of the manifest's). A window reads the lifetimes the run kept at open, then the runs of the asked
//! lifetimes' counters and their identities' `partial` flags over the window: it costs the
//! lifetimes and the changes in them, never the hours of the campaign. Identities are named by
//! the one identity rule; an hour an identity holds no tag is left out, never named by a guess.

use dashboard_base::products::{Country, validate_country};
use super::{
    BranchParams, LoadContext, Product,
    army::windows,
    wars::{self, WarRelationMembership, WarsReply},
};
use crate::{Source, catalogue, span::HourSpan};
use dashboard_query::{Query, identity::Holder, read};
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

/// The content's name for losses no attacker dealt, and what it is shown as without it.
const UNKNOWN_KEY: &str = "CASUALTIES_UNKNOWN";
const UNKNOWN_FALLBACK: &str = "Unknown attacker";
/// Why a breakdown run is under no war: several wars hold its lifetime, or none does.
pub const SEVERAL_WARS: &str = "The native view places this relation in multiple war episodes.";
pub const NO_WAR: &str = "The native war grouping was not recorded for this relation observation.";

/// A country identity: its canonical slot and the raw id of its Countries row.
pub type Identity = (u32, u32);

/// A `CasualtyLifetimes` key's identities (`a` the lesser) and its relation's start hour, as the
/// row codec packs them.
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

/// The `CasualtyTotals` key of an identity (`Key::Holder`).
pub fn holder_key((slot, id): Identity) -> u128 {
    dashboard_declare::Key::Holder.of(slot, id)
}

fn identity_text((slot, id): Identity) -> String {
    format!("{slot}:{id}")
}

/// Whether `identity`, holding `tag`, is `country`.
pub fn matches(country: &Country, identity: Identity, tag: &str) -> bool {
    match country {
        Country::Tag(chosen) => chosen == tag,
        Country::Slot(slot) => identity.0 == *slot,
    }
}

/// Each holder identity's tags over a timeline, from the identity layer's holder runs.
#[derive(Default)]
pub struct HolderTags {
    by: HashMap<Identity, Vec<(String, i32, i32)>>,
}

impl HolderTags {
    pub fn new(query: &Query<'_>) -> Self {
        let mut by: HashMap<Identity, Vec<(String, i32, i32)>> = HashMap::new();
        for (tag, runs) in &query.holders.tags {
            for run in runs {
                if let Holder::Found { slot, id } = run.holder {
                    by.entry((slot, id)).or_default().push((tag.clone(), run.first, run.last));
                }
            }
        }
        Self::from_runs(by)
    }

    pub fn from_runs(mut by: HashMap<Identity, Vec<(String, i32, i32)>>) -> Self {
        for runs in by.values_mut() {
            runs.sort_unstable_by_key(|run| run.1);
        }
        Self { by }
    }

    /// The tag `identity` holds at `hour`.
    pub fn tag_at(&self, identity: Identity, hour: i32) -> Option<&str> {
        let runs = self.by.get(&identity)?;
        let at = runs.partition_point(|run| run.2 < hour);
        runs.get(at).filter(|run| run.1 <= hour).map(|run| run.0.as_str())
    }

    /// Each hour `identity`'s tag changes within `first..=last` (its runs' edges).
    fn edges(&self, identity: Identity, first: i32, last: i32, out: &mut BTreeSet<i32>) {
        for (_, a, b) in self.by.get(&identity).map(Vec::as_slice).unwrap_or_default() {
            for edge in [*a, b.saturating_add(1)] {
                if first < edge && edge <= last {
                    out.insert(edge);
                }
            }
        }
    }

    /// The identities that were `country` within `first..=last`, each with its hours there: a
    /// tag's holders, or a slot's own identities whatever tags they held.
    pub fn holding(&self, country: &Country, first: i32, last: i32) -> Vec<(Identity, i32, i32)> {
        let mut held: Vec<(Identity, i32, i32)> = self
            .by
            .iter()
            .flat_map(|(identity, runs)| runs.iter().filter(move |run| matches(country, *identity, &run.0)).map(move |run| (*identity, run.1.max(first), run.2.min(last))))
            .filter(|(_, a, b)| a <= b)
            .collect();
        held.sort_unstable();
        let mut out: Vec<(Identity, i32, i32)> = Vec::with_capacity(held.len());
        for run in held {
            match out.last_mut() {
                Some(last) if last.0 == run.0 && i64::from(last.2) + 1 >= i64::from(run.1) => last.2 = last.2.max(run.2),
                _ => out.push(run),
            }
        }
        out.sort_unstable_by_key(|run| run.1);
        out
    }
}

/// The value of `runs` (ascending) at `hour`, none outside them or where not recorded.
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

/// Whether `hour` lies in `spans` (ascending, disjoint).
fn within(spans: &[(i32, i32)], hour: i32) -> bool {
    let at = spans.partition_point(|span| span.1 < hour);
    spans.get(at).is_some_and(|span| span.0 <= hour)
}

/// What a window read of the casualty tables holds: each asked lifetime's two counters, each
/// asked identity's `partial` flag, and the hours the lifetimes were written.
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

/// Consecutive hours `first..=last` cut where any of `edges` falls, each piece's first hour.
fn pieces(edges: &BTreeSet<i32>, first: i32, last: i32) -> impl Iterator<Item = (i32, i32)> + '_ {
    let starts: Vec<i32> = edges.range(first..=last).copied().collect();
    (0..starts.len()).map(move |at| (starts[at], starts.get(at + 1).map_or(last, |next| next - 1)))
}

/// A lifetime's counter at `hour`: its row's while it lives, its last row's once it has ended.
fn held_at(runs: &read::Runs, gone: i64, hour: i32) -> Option<i64> {
    value_at(runs, i64::from(hour).min(gone) as i32)
}

/// The sum of `counters` (one per lifetime) at `hour`; none when no lifetime was born by then or
/// one that was has its counter not recorded.
fn summed(counters: &[Option<&read::Runs>], lives: &[(i64, i64)], hour: i32) -> Option<i128> {
    let mut total = 0i128;
    let mut any = false;
    for (runs, &(born, gone)) in counters.iter().zip(lives) {
        if i64::from(hour) < born {
            continue;
        }
        total += i128::from(held_at((*runs)?, gone, hour)?);
        any = true;
    }
    any.then_some(total)
}

fn push_run<T>(runs: &mut Vec<T>, run: T, joins: impl Fn(&mut T, &T) -> bool) {
    if let Some(last) = runs.last_mut()
        && joins(last, &run)
    {
        return;
    }
    runs.push(run);
}

// ---------------------------------------------------------------------------------------------
// casualty-pairs

pub struct CasualtyPairs;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasualtyPairsParams {
    pub branch_id: String,
    pub countries: Vec<Country>,
    /// Otherwise each chosen country's casualties with every enemy.
    pub between: bool,
    /// None asks for the window manifest.
    pub window: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyPairRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
    pub hourly: bool,
    /// What the first country took from the second, over every lifetime of the pair.
    pub first_taken: String,
    pub second_taken: String,
    /// Either country's recorded history lost a lifetime's last hours in a gap.
    pub incomplete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyPairSeries {
    pub identity: String,
    pub first_country: String,
    pub second_country: String,
    pub runs: Vec<CasualtyPairRun>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyPairsReply {
    pub branch_id: String,
    pub available: bool,
    pub windows: Vec<HourSpan>,
    pub window: Option<usize>,
    /// Hours of the window the casualty tables were written.
    pub observed: Vec<HourSpan>,
    pub pairs: Vec<CasualtyPairSeries>,
    /// The window has hours the casualty tables were not written.
    pub has_gap: bool,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CasualtyPairsReply, CasualtyBreakdownReply, CasualtyLabelsReply);

/// The pairs of `lifetimes` (key, born, gone) over `first..=last`: every lifetime of one
/// identity pair summed, cut where either identity's tag changes, kept where `keep` says so
/// for the two tags.
pub fn fold_pairs(
    lifetimes: &[(u128, i64, i64)],
    reads: &WindowReads,
    tags: &HolderTags,
    first: i32,
    last: i32,
    keep: impl Fn((Identity, &str), (Identity, &str)) -> bool,
) -> Vec<CasualtyPairSeries> {
    let mut by_pair: BTreeMap<(Identity, Identity), Vec<(u128, i64, i64)>> = BTreeMap::new();
    for &(key, born, gone) in lifetimes {
        let lifetime = Lifetime::of(key);
        by_pair.entry((lifetime.a, lifetime.b)).or_default().push((key, born, gone));
    }
    let mut out: BTreeMap<(String, String, String), Vec<CasualtyPairRun>> = BTreeMap::new();
    for ((a, b), held) in by_pair {
        let keys: Vec<u128> = held.iter().map(|life| life.0).collect();
        let lives: Vec<(i64, i64)> = held.iter().map(|life| (life.1, life.2)).collect();
        let taken_a: Vec<Option<&read::Runs>> = keys.iter().map(|key| reads.taken_a.get(key)).collect();
        let taken_b: Vec<Option<&read::Runs>> = keys.iter().map(|key| reads.taken_b.get(key)).collect();
        let mut edges = reads.edges(&keys, &[a, b], first, last);
        for &(born, gone) in &lives {
            for edge in [born, gone.saturating_add(1)] {
                if i64::from(first) < edge && edge <= i64::from(last) {
                    edges.insert(edge as i32);
                }
            }
        }
        tags.edges(a, first, last, &mut edges);
        tags.edges(b, first, last, &mut edges);
        let identity = format!("{}-{}", identity_text(a), identity_text(b));
        for (from, to) in pieces(&edges, first, last) {
            if !within(&reads.observed, from) {
                continue;
            }
            let (Some(first_tag), Some(second_tag)) = (tags.tag_at(a, from), tags.tag_at(b, from)) else { continue };
            if !keep((a, first_tag), (b, second_tag)) {
                continue;
            }
            let (Some(first_taken), Some(second_taken)) = (summed(&taken_a, &lives, from), summed(&taken_b, &lives, from)) else { continue };
            let run = CasualtyPairRun {
                first_hour: from,
                last_hour: to,
                read_hour: from,
                hourly: true,
                first_taken: first_taken.to_string(),
                second_taken: second_taken.to_string(),
                incomplete: reads.incomplete(a, from) || reads.incomplete(b, from),
            };
            let runs = out.entry((identity.clone(), first_tag.to_owned(), second_tag.to_owned())).or_default();
            push_run(runs, run, |held, next| {
                let joins = i64::from(held.last_hour) + 1 == i64::from(next.first_hour)
                    && (&held.first_taken, &held.second_taken, held.incomplete) == (&next.first_taken, &next.second_taken, next.incomplete);
                if joins {
                    held.last_hour = next.last_hour;
                }
                joins
            });
        }
    }
    out.into_iter()
        .map(|((identity, first_country, second_country), runs)| CasualtyPairSeries { identity, first_country, second_country, runs })
        .collect()
}

/// A window of `query`'s timeline, its run, and the lifetimes the run kept that meet it.
struct Window {
    first: i32,
    last: i32,
    run: u32,
    lifetimes: Vec<(u128, i64, i64)>,
}

fn window(source: &Source, query: &Query<'_>, spans: &[HourSpan], index: usize) -> io::Result<Window> {
    let span = spans.get(index).ok_or_else(|| invalid("Unknown casualty history window"))?;
    let (first, last) = (span.first_hour, span.last_hour);
    let run = query.timeline.at(first).ok_or_else(|| invalid("The window's hours are not recorded"))?.run;
    // A lifetime ended before the window still counts: its last counters hold.
    let lifetimes = crate::answer::lifetimes(source, run, TableId::CasualtyLifetimes)?
        .iter()
        .filter(|&&(_, born, _)| born <= i64::from(last))
        .copied()
        .collect();
    Ok(Window { first, last, run, lifetimes })
}

/// The counters of `lives` and the `partial` flags of `identities` over `window`, and the hours
/// the lifetimes were available there. A lifetime ended before the window is its row at its last
/// hour, one run of that hour, from the run's ended counters kept at open (`ended`).
fn window_reads(query: &Query<'_>, window: &Window, lives: &[(u128, i64, i64)], identities: &[Identity], ended: &EndedCounters) -> io::Result<WindowReads> {
    let recording = query.replay.recording();
    let column = |table: TableId, name: &str| dashboard_declare::field(table, name).map(Column::Cell);
    let (first, last, run) = (window.first, window.last, window.run);
    let mut reads = WindowReads {
        // An hour read but refused holds no rows: it is not an observed hour.
        observed: replay::lanes::available_reads(recording, run, TableId::CasualtyLifetimes, first, last)?,
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

/// Each lifetime of a run that ended before the run's last hour: its two counters at its last
/// hour (`taken_a`, `taken_b`; none where not recorded), read once at open.
#[derive(Default)]
pub struct EndedCounters(pub HashMap<u128, (Option<i64>, Option<i64>)>);

/// Run `run`'s ended lifetimes' last counters: one reader through the run's blocks for every
/// ended lifetime, so a window never reads before itself.
pub fn run_ends(source: &Source, run: u32) -> io::Result<EndedCounters> {
    let recording = source.replay_ref().recording();
    let last = recording.run(run)?.last_hour();
    let lifetimes = crate::answer::lifetimes(source, run, TableId::CasualtyLifetimes)?;
    let mut asks: Vec<(i32, u128)> = lifetimes.iter().filter(|&&(_, _, gone)| gone < i64::from(last)).map(|&(key, _, gone)| (gone as i32, key)).collect();
    asks.sort_unstable();
    let mut out = EndedCounters::default();
    if asks.is_empty() {
        return Ok(out);
    }
    let columns = [dashboard_declare::field(TableId::CasualtyLifetimes, "taken_a")?, dashboard_declare::field(TableId::CasualtyLifetimes, "taken_b")?];
    replay::lanes::at_each(recording, run, TableId::CasualtyLifetimes, Some(&columns), &asks, &mut |_, key, row| {
        let [a, b] = columns.map(|column| row.as_ref().and_then(|(view, slot)| view.number(column, *slot)));
        out.0.insert(key, (a, b));
        Ok(())
    })?;
    Ok(out)
}

/// A lifetime's war over consecutive hours: the one war holding it, or none (`ambiguous` when
/// several hold it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grouping {
    pub first: i32,
    pub last: i32,
    pub war: Option<String>,
    pub ambiguous: bool,
}

/// Each of `keys`' groupings (ascending) by the wars (an id, the lifetimes it held and when) that
/// hold it; the hours between are a grouping of no war.
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

/// The hours losses were read over the branch, for a manifest: from page entries alone.
fn branch_observed(query: &Query<'_>) -> io::Result<Vec<HourSpan>> {
    let mut out = Vec::new();
    for stretch in &query.timeline.stretches {
        out.extend(spans(&replay::lanes::available_reads(query.replay.recording(), stretch.run, TableId::CasualtyLifetimes, stretch.first, stretch.last)?));
    }
    Ok(out)
}

fn manifest(query: &Query<'_>) -> io::Result<Vec<HourSpan>> {
    Ok(windows(query)?.into_iter().map(|span| HourSpan { first_hour: span.first_hour, last_hour: span.last_hour }).collect())
}

fn branch(text: &str) -> Result<u64, &'static str> {
    text.parse::<u64>().map_err(|_| "Invalid branch")
}

impl Product for CasualtyPairs {
    const KEY: &'static str = "casualty-pairs";
    const CONTENT_DEPENDENT: bool = false;
    type Params = CasualtyPairsParams;
    type Reply = CasualtyPairsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CasualtyPairsParams::decl(), CasualtyPairRun::decl(), CasualtyPairSeries::decl(), HourSpan::decl(), CasualtyPairsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        params.countries.iter().try_for_each(validate_country)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = crate::answer::query(source, branch(&params.branch_id).map_err(invalid)?)?;
        let windows = manifest(&query)?;
        let mut reply = CasualtyPairsReply {
            branch_id: params.branch_id.clone(),
            available: true,
            windows: windows.clone(),
            window: params.window,
            observed: Vec::new(),
            pairs: Vec::new(),
            has_gap: false,
            warnings: Vec::new(),
        };
        let Some(index) = params.window else { return Ok(reply) };
        let window = window(source, &query, &windows, index)?;
        crate::metadata::check_cancel(cancelled)?;
        let tags = HolderTags::new(&query);
        let chosen = |(identity, tag): (Identity, &str)| params.countries.iter().any(|country| matches(country, identity, tag));
        let keep = |a: (Identity, &str), b: (Identity, &str)| params.countries.is_empty() || if params.between { chosen(a) && chosen(b) } else { chosen(a) || chosen(b) };
        // A lifetime is read only when some tag its identities held in the window is kept.
        let names = |identity: Identity| -> BTreeSet<&str> {
            let mut out = BTreeSet::new();
            if let Some(runs) = tags.by.get(&identity) {
                for (tag, a, b) in runs {
                    if *a <= window.last && *b >= window.first {
                        out.insert(tag.as_str());
                    }
                }
            }
            out
        };
        let asked: Vec<(u128, i64, i64)> = window
            .lifetimes
            .iter()
            .filter(|&&(key, ..)| {
                let lifetime = Lifetime::of(key);
                let (a, b) = (names(lifetime.a), names(lifetime.b));
                a.iter().any(|first| b.iter().any(|second| keep((lifetime.a, first), (lifetime.b, second))))
            })
            .copied()
            .collect();
        let identities: Vec<Identity> = asked.iter().flat_map(|&(key, ..)| {
            let lifetime = Lifetime::of(key);
            [lifetime.a, lifetime.b]
        }).collect();
        let ended = crate::answer::casualty_ends(source, window.run)?;
        let reads = window_reads(&query, &window, &asked, &identities, &ended)?;
        crate::metadata::check_cancel(cancelled)?;
        reply.pairs = fold_pairs(&asked, &reads, &tags, window.first, window.last, keep);
        reply.has_gap = reads.observed != [(window.first, window.last)];
        reply.observed = spans(&reads.observed);
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

// ---------------------------------------------------------------------------------------------
// casualty-breakdown

pub struct CasualtyBreakdown;

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    /// What the country took in this lifetime.
    pub value: String,
    /// The war the lifetime belongs to; null with `reason` when not known.
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

/// A war's name over hours, as the wars family names it.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyWarName {
    pub text: String,
    pub known: bool,
    pub reason: Option<String>,
    pub key: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyWarNameRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: CasualtyWarName,
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownReply {
    pub branch_id: String,
    pub tag: String,
    pub available: bool,
    pub windows: Vec<HourSpan>,
    pub window: Option<usize>,
    pub observed: Vec<HourSpan>,
    /// The identities holding the tag within the observed hours.
    pub countries: Vec<CasualtyBreakdownCountryRun>,
    pub series: Vec<CasualtyBreakdownSeries>,
    pub wars: Vec<CasualtyBreakdownWar>,
    pub has_gap: bool,
    pub warnings: Vec<String>,
}

/// `tag`'s losses over `first..=last`: a series per lifetime of each identity holding it (cut
/// where the enemy's tag or the lifetime's war changes), and the identities' runs. An ended
/// lifetime stays in the war that held it at its last hour.
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

/// The wars of `view` that `series` names, as shown to `last`: their hours and names to it, never
/// a later name.
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
        super::validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch_id = branch(&params.branch_id).map_err(invalid)?;
        let query = crate::answer::query(source, branch_id)?;
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
        let window = window(source, &query, &windows, index)?;
        crate::metadata::check_cancel(cancelled)?;
        let tags = HolderTags::new(&query);
        let holding: Vec<Identity> = tags.holding(&params.country, window.first, window.last).into_iter().map(|run| run.0).collect();
        let asked: Vec<(u128, i64, i64)> = window
            .lifetimes
            .iter()
            .filter(|&&(key, ..)| {
                let lifetime = Lifetime::of(key);
                holding.contains(&lifetime.a) || holding.contains(&lifetime.b)
            })
            .copied()
            .collect();
        let ended = crate::answer::casualty_ends(source, window.run)?;
        let reads = window_reads(&query, &window, &asked, &holding, &ended)?;
        crate::metadata::check_cancel(cancelled)?;
        let (groupings, view) = if asked.is_empty() {
            (HashMap::new(), None)
        } else {
            let view = wars::viewpoint_wars(source, branch_id, &params.country, context, cancelled)?;
            let keys: HashSet<u128> = asked.iter().map(|life| life.0).collect();
            (lifetime_groupings(view.wars.iter().map(|war| (war.id.as_str(), war.relations.as_slice())), &keys), Some(view))
        };
        let (countries, series) = fold_breakdown(&params.country, &asked, &reads, &tags, window.first, window.last, &groupings);
        if let Some(view) = view {
            reply.wars = shown_wars(&view, &series, window.last);
            reply.warnings.extend(view.warnings.iter().cloned());
        }
        reply.countries = countries;
        reply.series = series;
        reply.has_gap = reads.observed != [(window.first, window.last)];
        reply.observed = spans(&reads.observed);
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

// ---------------------------------------------------------------------------------------------
// casualty-labels

pub struct CasualtyLabels;

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyLabelsReply {
    /// The game's name for losses no attacker dealt, in the recording's language.
    pub unknown_label: String,
    pub warnings: Vec<String>,
}

impl Product for CasualtyLabels {
    const KEY: &'static str = "casualty-labels";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    type Reply = CasualtyLabelsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CasualtyLabelsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id).map(|_| ())
    }

    fn load(source: &Source, _params: &Self::Params, context: &LoadContext, _cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let mut warnings = Vec::new();
        let label = match context.content.as_ref() {
            Some(options) => match catalogue::catalogue(source, options) {
                Ok(catalogue) => {
                    let label = catalogue.localised(UNKNOWN_KEY, catalogue::View::Plain);
                    warnings.extend(catalogue.refusal_lines());
                    label
                }
                Err(error) => {
                    warnings.push(format!("The name of losses from unknown attackers is unavailable: {error}"));
                    None
                }
            },
            None => {
                warnings.push("The name of losses from unknown attackers is unavailable until the game folders are chosen.".into());
                None
            }
        };
        Ok(CasualtyLabelsReply { unknown_label: label.unwrap_or_else(|| UNKNOWN_FALLBACK.to_owned()), warnings })
    }
}
