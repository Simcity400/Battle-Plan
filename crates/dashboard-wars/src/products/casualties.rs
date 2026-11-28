use super::wars::{self, WarRelationMembership, WarsReply};
use dashboard_base::{
    Source,
    products::{
        Country, LoadContext, Product,
        breakdown::{self, Amount, Breakdown, Exact, Leaf, Node, Unrecorded},
        read_ahead::Prepare,
    },
    span::HourSpan,
};
use dashboard_query::{
    Query,
    coverage::row_coverage,
    identity::{Holder, HolderRun, Holders, TAG_UNLINKED},
};
use dashboard_world::products::{
    army_windows::windows,
    holder_tags::{HolderTags, Identity},
};
use replay_columnar::manifest::Status;
use replay_data::rows::{CasualtyLifetimeRow, TableId};
use replay_lanes::lanes::{self, Cell, History, Keys, Reading, StatusRun, Step};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
const NOT_OBSERVED: &str = "War casualties were not recorded at this hour";
const READ_IN_PART: &str = "The game's war casualty list was read only in part";
const COUNTER_NOT_RECORDED: &str = "A war relation's casualties were not recorded at this hour";
const ENDED_NOT_RECORDED: &str = "An ended war relation's final casualties were not recorded";
const CONTENT_STACK_WARNING: &str = "War names are Not recorded until the recorded content stack is resolved.";

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

fn manifest(query: &Query<'_>) -> io::Result<Vec<HourSpan>> {
    Ok(windows(query)?.into_iter().map(|span| HourSpan { first_hour: span.first_hour, last_hour: span.last_hour }).collect())
}

fn branch(text: &str) -> Result<u64, &'static str> {
    text.parse::<u64>().map_err(|_| "Invalid branch")
}

fn unlinked_spans(holders: &Holders) -> Vec<(i32, i32)> {
    let mut spans: Vec<(i32, i32)> = holders.tags.values().flatten().filter(|run| run.holder == Holder::Unresolved).map(|run| (run.first, run.last)).collect();
    spans.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        match out.last_mut() {
            Some(held) if i64::from(first) <= i64::from(held.1) + 1 => held.1 = held.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

fn clip_spans(spans: &[(i32, i32)], first: i32, last: i32) -> Vec<(i32, i32)> {
    spans
        .iter()
        .filter_map(|&(a, b)| {
            let (a, b) = (a.max(first), b.min(last));
            (a <= b).then_some((a, b))
        })
        .collect()
}

fn covering<'a, T>(runs: &'a [T], hour: i32, bounds: impl Fn(&T) -> (i32, i32)) -> Option<&'a T> {
    let at = runs.partition_point(|run| bounds(run).1 < hour);
    runs.get(at).filter(|run| bounds(run).0 <= hour)
}

fn push_edge(changes: &mut Vec<i32>, first: i32, last: i32, edge: i64) {
    if i64::from(first) < edge && edge <= i64::from(last) {
        changes.push(edge as i32);
    }
}

fn push_span_edges(changes: &mut Vec<i32>, first: i32, last: i32, a: i32, b: i32) {
    push_edge(changes, first, last, i64::from(a));
    push_edge(changes, first, last, i64::from(b) + 1);
}

fn status_at<'a>(statuses: &'a [StatusRun], reasons: &'a HashMap<u32, String>, hour: i32) -> Option<(Status, Option<&'a str>)> {
    let run = covering(statuses, hour, |run| (run.first, run.last))?;
    let reason = match run.status {
        Status::Unavailable { reason } => reasons.get(&reason).map(String::as_str),
        _ => None,
    };
    Some((run.status, reason))
}

fn stretch_lifetimes(source: &Source, run: u32, last: i32) -> io::Result<Vec<(u128, i64, i64)>> {
    Ok(dashboard_base::queries::lifetimes(source, run, TableId::CasualtyLifetimes)?
        .iter()
        .filter(|&&(_, born, _)| born <= i64::from(last))
        .copied()
        .collect())
}

fn slot_tag_runs(holders: &Holders, slots: &HashSet<u32>) -> HashMap<u32, Vec<(i32, i32, String)>> {
    let mut out: HashMap<u32, Vec<(i32, i32, String)>> = HashMap::new();
    for (tag, runs) in &holders.tags {
        for run in runs {
            if let Holder::Found { slot, .. } = run.holder
                && slots.contains(&slot)
            {
                out.entry(slot).or_default().push((run.first, run.last, tag.clone()));
            }
        }
    }
    for runs in out.values_mut() {
        runs.sort_unstable_by(|a, b| (a.0, &a.2).cmp(&(b.0, &b.2)));
    }
    out
}

pub struct CasualtyBreakdown;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasualtyBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownReply {
    pub from: i32,
    pub hour: i32,
    pub through: i32,
    pub breakdown: Breakdown,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CasualtyBreakdownReply);

struct TrackedLife {
    key: u128,
    country: Identity,
    enemy: Identity,
    is_a: bool,
    born: i64,
    gone: i64,
    ended_value: Option<i64>,
    groupings: Vec<Grouping>,
}

struct StretchData {
    first: i32,
    last: i32,
    taken_a_pos: usize,
    taken_b_pos: usize,
    partial_pos: usize,
    lifetimes: History,
    totals: History,
    lives: Vec<TrackedLife>,
}

struct WarEntry {
    ordinal: u32,
    names: Vec<(i32, i32, Node)>,
}

enum HolderState {
    Found(Identity),
    Absent,
    NotRecorded(String),
}

pub(crate) struct CasualtyIndex {
    from: i32,
    to: i32,
    country_key: String,
    unknown: Vec<(i32, i32, String)>,
    unlinked: Vec<(i32, i32)>,
    holders: Vec<HolderRun>,
    stretches: Vec<StretchData>,
    enemy_tags: HashMap<u32, Vec<(i32, i32, String)>>,
    wars: HashMap<String, WarEntry>,
    changes: Vec<i32>,
    warnings: Vec<String>,
}

impl CasualtyIndex {
    fn holder_state(&self, hour: i32) -> HolderState {
        if let Some((_, _, reason)) = covering(&self.unknown, hour, |&(first, last, _)| (first, last)) {
            return HolderState::NotRecorded(format!("Who holds {} is not known: {reason}", self.country_key));
        }
        if let Some(run) = covering(&self.holders, hour, |run| (run.first, run.last)) {
            return match run.holder {
                Holder::Found { slot, id } => HolderState::Found((slot, id)),
                Holder::Unresolved => HolderState::NotRecorded(format!("Who holds {} is not known: {TAG_UNLINKED}", self.country_key)),
            };
        }
        if covering(&self.unlinked, hour, |&(first, last)| (first, last)).is_some() {
            return HolderState::NotRecorded(format!("Who holds {} is not known: {TAG_UNLINKED}", self.country_key));
        }
        HolderState::Absent
    }

    fn enemy_tag_at(&self, slot: u32, hour: i32) -> Option<&str> {
        let runs = self.enemy_tags.get(&slot)?;
        runs.iter().find(|&&(first, last, _)| first <= hour && hour <= last).map(|(_, _, tag)| tag.as_str())
    }

    fn war_node(&self, grouping: Option<&Grouping>, grouping_hour: i32) -> Node {
        let Some(id) = grouping.and_then(|run| run.war.as_deref()) else {
            if grouping.is_some_and(|run| run.ambiguous) {
                return Node::new("~war:several", "Multiple wars", u32::MAX - 1).other().name_missing(Some(SEVERAL_WARS.into()));
            }
            return Node::new("~war:none", "Ungrouped casualties", u32::MAX).other().name_missing(Some(NO_WAR.into()));
        };
        let Some(entry) = self.wars.get(id) else {
            return Node::new(id, id, u32::MAX - 2).name_missing(Some("The war's name was not recorded at this hour".into()));
        };
        covering(&entry.names, grouping_hour, |&(first, last, _)| (first, last))
            .map(|(_, _, node)| node.clone())
            .unwrap_or_else(|| Node::new(id, format!("War {}", entry.ordinal + 1), entry.ordinal).name_missing(Some("The war's name was not recorded at this hour".into())))
    }

    pub(crate) fn at(&self, hour: i32) -> CasualtyBreakdownReply {
        let next = self.changes.get(self.changes.partition_point(|&at| at <= hour)).copied();
        let through = next.map_or(self.to, |next| next - 1).min(self.to).max(hour);
        let reply_with = |leaves: &[Leaf]| CasualtyBreakdownReply {
            from: self.from,
            hour,
            through,
            breakdown: breakdown::breakdown(leaves, &Unrecorded::default()),
            warnings: self.warnings.clone(),
        };
        let identity = match self.holder_state(hour) {
            HolderState::NotRecorded(reason) => {
                return reply_with(&[Leaf { path: Vec::new(), amount: Amount::NotRecorded(reason), count: None, last: None }]);
            }
            HolderState::Absent => None,
            HolderState::Found(identity) => Some(identity),
        };
        let Some(stretch) = covering(&self.stretches, hour, |stretch| (stretch.first, stretch.last)) else {
            return reply_with(&[Leaf { path: Vec::new(), amount: Amount::NotRecorded(NOT_OBSERVED.into()), count: None, last: None }]);
        };
        let truncated_reason = match status_at(&stretch.lifetimes.statuses, &stretch.lifetimes.reasons, hour) {
            Some((Status::Complete, _)) => None,
            Some((status @ Status::Truncated { .. }, reason)) => row_coverage(TableId::CasualtyLifetimes, status, reason, false).into_missing(),
            Some((status, reason)) => {
                let why = row_coverage(TableId::CasualtyLifetimes, status, reason, false).into_missing().unwrap_or_else(|| NOT_OBSERVED.into());
                return reply_with(&[Leaf { path: Vec::new(), amount: Amount::NotRecorded(why), count: None, last: None }]);
            }
            None => {
                return reply_with(&[Leaf { path: Vec::new(), amount: Amount::NotRecorded(NOT_OBSERVED.into()), count: None, last: None }]);
            }
        };
        let Some(identity) = identity else {
            return reply_with(&[]);
        };
        let totals_issue: Option<String> = match stretch.totals.reading(holder_key(identity), hour) {
            Reading::Cells(cells) => match cells.get(stretch.partial_pos).copied() {
                Some(Cell::Number(0)) => None,
                _ => Some(READ_IN_PART.into()),
            },
            Reading::Absent => None,
            Reading::Unread { status, reason } => row_coverage(TableId::CasualtyTotals, status, reason, false).into_missing(),
        };
        let mut active: Vec<(&TrackedLife, Node, bool)> = Vec::new();
        let mut labels: HashMap<String, (i32, Node)> = HashMap::new();
        let mut pair_last: HashMap<(String, u32), Option<i32>> = HashMap::new();
        for life in &stretch.lives {
            if life.country != identity || life.born > i64::from(hour) {
                continue;
            }
            let living = i64::from(hour) <= life.gone;
            let grouping_hour = if living { hour } else { life.gone as i32 };
            let war = self.war_node(grouping_at(&life.groupings, grouping_hour), grouping_hour);
            match labels.get_mut(&war.key) {
                Some(held) if held.0 >= grouping_hour => {}
                Some(held) => *held = (grouping_hour, war.clone()),
                None => {
                    labels.insert(war.key.clone(), (grouping_hour, war.clone()));
                }
            }
            let pair_key = (war.key.clone(), life.enemy.0);
            match pair_last.get_mut(&pair_key) {
                Some(held) => {
                    if living {
                        *held = None;
                    } else if let Some(prev) = held {
                        *prev = (*prev).max(life.gone as i32);
                    }
                }
                None => {
                    pair_last.insert(pair_key, (!living).then_some(life.gone as i32));
                }
            }
            active.push((life, war, living));
        }
        let mut leaves = Vec::new();
        for (life, war, living) in active {
            let war = labels.get(&war.key).map_or(war, |(_, label)| label.clone());
            let value = if living {
                match stretch.lifetimes.reading(life.key, hour) {
                    Reading::Cells(cells) => {
                        let col = if life.is_a { stretch.taken_a_pos } else { stretch.taken_b_pos };
                        match cells.get(col).copied() {
                            Some(Cell::Number(value)) => Ok(Some(value)),
                            _ => Err(COUNTER_NOT_RECORDED.to_owned()),
                        }
                    }
                    Reading::Absent => Ok(None),
                    Reading::Unread { status, reason } => Err(row_coverage(TableId::CasualtyLifetimes, status, reason, false).into_missing().unwrap_or_else(|| COUNTER_NOT_RECORDED.to_owned())),
                }
            } else {
                match life.ended_value {
                    Some(value) => Ok(Some(value)),
                    None => Err(ENDED_NOT_RECORDED.to_owned()),
                }
            };
            let amount = match value {
                Ok(None) => continue,
                Ok(Some(0)) if totals_issue.is_none() => continue,
                Ok(Some(value)) => match &totals_issue {
                    Some(reason) => Amount::Partial(Exact::whole(i128::from(value)), reason.clone()),
                    None => Amount::Known(Exact::whole(i128::from(value))),
                },
                Err(reason) => Amount::NotRecordedSigned(reason),
            };
            let last = pair_last.get(&(war.key.clone(), life.enemy.0)).copied().flatten();
            let enemy_hour = last.unwrap_or(hour);
            let enemy = match self.enemy_tag_at(life.enemy.0, enemy_hour) {
                Some(tag) => Node::new(format!("slot:{}", life.enemy.0), tag, life.enemy.0),
                None => Node::new(format!("slot:{}", life.enemy.0), "Country not named at this hour", life.enemy.0).other(),
            };
            leaves.push(Leaf { path: vec![war, enemy], amount, count: None, last });
        }
        if let Some(reason) = truncated_reason {
            leaves.push(Leaf { path: vec![Node::new("~unread", "Unread war relations", u32::MAX).other()], amount: Amount::NotRecordedSigned(reason), count: None, last: None });
        } else if leaves.is_empty()
            && let Some(reason) = totals_issue
        {
            leaves.push(Leaf { path: Vec::new(), amount: Amount::NotRecorded(reason), count: None, last: None });
        }
        reply_with(&leaves)
    }
}

fn build_wars(view: &WarsReply) -> HashMap<String, WarEntry> {
    view.wars
        .iter()
        .enumerate()
        .map(|(ordinal, war)| {
            let names = war.observations.iter().map(|run| {
                let node = if run.name.known {
                    Node::new(&war.id, &run.name.text, ordinal as u32)
                } else {
                    let scope = match (run.viewpoint_side.first(), run.opposing_side.first()) {
                        (Some(first), Some(second)) => format!("{first} side vs {second} side"),
                        _ => format!("War {}", ordinal + 1),
                    };
                    Node::new(&war.id, scope, ordinal as u32).name_missing(Some(run.name.reason.clone().unwrap_or_else(|| "The war's name was not recorded".into())))
                };
                (run.first_hour, run.last_hour, node)
            }).collect();
            (war.id.clone(), WarEntry { ordinal: ordinal as u32, names })
        })
        .collect()
}

fn build_index(
    source: &Source,
    query: &Query<'_>,
    branch_id: u64,
    country: &Country,
    first: i32,
    last: i32,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<CasualtyIndex> {
    let stretches_in: Vec<(u32, i32, i32)> = query.timeline.within(first, last).map(|stretch| (stretch.run, stretch.first, stretch.last)).collect();
    let tags = HolderTags::new(query);
    let mut per_stretch = Vec::with_capacity(stretches_in.len());
    let mut all_keys: HashSet<u128> = HashSet::new();
    let mut enemy_slots: HashSet<u32> = HashSet::new();
    for &(run, s_first, s_last) in &stretches_in {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let lifetimes = stretch_lifetimes(source, run, s_last)?;
        let holding: Vec<Identity> = tags.holding(country, s_first, s_last).into_iter().map(|run| run.0).collect();
        let asked: Vec<(u128, i64, i64)> = lifetimes
            .iter()
            .filter(|&&(key, ..)| {
                let lifetime = Lifetime::of(key);
                holding.contains(&lifetime.a) || holding.contains(&lifetime.b)
            })
            .copied()
            .collect();
        for &(key, ..) in &asked {
            all_keys.insert(key);
            let lifetime = Lifetime::of(key);
            if holding.contains(&lifetime.a) {
                enemy_slots.insert(lifetime.b.0);
            }
            if holding.contains(&lifetime.b) {
                enemy_slots.insert(lifetime.a.0);
            }
        }
        per_stretch.push((run, s_first, s_last, holding, asked));
    }
    let (groupings, wars, warnings) = if all_keys.is_empty() {
        (HashMap::new(), HashMap::new(), Vec::new())
    } else {
        let view = wars::viewpoint_wars(source, branch_id, country, context, cancelled)?;
        let groupings = lifetime_groupings(view.wars.iter().map(|war| (war.id.as_str(), war.relations.as_slice())), &all_keys);
        let wars = build_wars(&view);
        let warnings = if context.content.is_none() && !wars.is_empty() { vec![CONTENT_STACK_WARNING.into()] } else { Vec::new() };
        (groupings, wars, warnings)
    };
    let recording = query.replay.recording();
    let taken_a_col = dashboard_declare::field(TableId::CasualtyLifetimes, "taken_a")?;
    let taken_b_col = dashboard_declare::field(TableId::CasualtyLifetimes, "taken_b")?;
    let partial_col = dashboard_declare::field(TableId::CasualtyTotals, "partial")?;
    let mut stretches = Vec::with_capacity(per_stretch.len());
    for (run, s_first, s_last, holding, asked) in per_stretch {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let ended = cached_casualty_ends(source, run)?;
        let live_keys: Vec<u128> = asked.iter().filter(|&&(_, _, gone)| gone >= i64::from(s_first)).map(|life| life.0).collect();
        let lifetimes = lanes::history(recording, run, TableId::CasualtyLifetimes, &[taken_a_col, taken_b_col], Keys::These(&live_keys), s_first, s_last)?;
        let taken_a_pos = lifetimes.columns.binary_search(&taken_a_col).map_err(|_| invalid("CasualtyLifetimes missing taken_a column"))?;
        let taken_b_pos = lifetimes.columns.binary_search(&taken_b_col).map_err(|_| invalid("CasualtyLifetimes missing taken_b column"))?;
        let mut holder_keys: Vec<u128> = holding.iter().map(|&identity| holder_key(identity)).collect();
        holder_keys.sort_unstable();
        holder_keys.dedup();
        let totals = lanes::history(recording, run, TableId::CasualtyTotals, &[partial_col], Keys::These(&holder_keys), s_first, s_last)?;
        let partial_pos = totals.columns.binary_search(&partial_col).map_err(|_| invalid("CasualtyTotals missing partial column"))?;
        let mut lives = Vec::new();
        for &(key, born, gone) in &asked {
            let lifetime = Lifetime::of(key);
            let grouped = groupings.get(&key).cloned().unwrap_or_default();
            for (country_id, enemy_id, is_a) in [(lifetime.a, lifetime.b, true), (lifetime.b, lifetime.a, false)] {
                if !holding.contains(&country_id) {
                    continue;
                }
                let ended_value = ended.0.get(&key).and_then(|&(a, b)| if is_a { a } else { b });
                lives.push(TrackedLife { key, country: country_id, enemy: enemy_id, is_a, born, gone, ended_value, groupings: grouped.clone() });
            }
        }
        stretches.push(StretchData { first: s_first, last: s_last, taken_a_pos, taken_b_pos, partial_pos, lifetimes, totals, lives });
    }
    let unknown: Vec<(i32, i32, String)> = query
        .holders
        .unknown
        .iter()
        .filter_map(|(a, b, reason)| {
            let (a, b) = ((*a).max(first), (*b).min(last));
            (a <= b).then(|| (a, b, reason.clone()))
        })
        .collect();
    let unlinked = if matches!(country, Country::Slot(_)) { clip_spans(&unlinked_spans(&query.holders), first, last) } else { Vec::new() };
    let holders = query.holders.within(country, first, last);
    let enemy_tags = slot_tag_runs(&query.holders, &enemy_slots);
    Ok(finalize_index(query.timeline.first, first, last, country.key().into_owned(), unknown, unlinked, holders, stretches, enemy_tags, wars, warnings))
}

#[expect(clippy::too_many_arguments, reason = "window index construction takes each resolved per-input history directly")]
fn finalize_index(
    from: i32,
    first: i32,
    last: i32,
    country_key: String,
    unknown: Vec<(i32, i32, String)>,
    unlinked: Vec<(i32, i32)>,
    holders: Vec<HolderRun>,
    stretches: Vec<StretchData>,
    enemy_tags: HashMap<u32, Vec<(i32, i32, String)>>,
    wars: HashMap<String, WarEntry>,
    warnings: Vec<String>,
) -> CasualtyIndex {
    let mut changes = Vec::new();
    for &(a, b, _) in &unknown {
        push_span_edges(&mut changes, first, last, a, b);
    }
    for &(a, b) in &unlinked {
        push_span_edges(&mut changes, first, last, a, b);
    }
    for run in &holders {
        push_span_edges(&mut changes, first, last, run.first, run.last);
    }
    for stretch in &stretches {
        push_span_edges(&mut changes, first, last, stretch.first, stretch.last);
        for run in &stretch.lifetimes.statuses {
            push_span_edges(&mut changes, first, last, run.first, run.last);
        }
        let steps_of: HashMap<u128, &[Step]> = stretch.lifetimes.keys.iter().map(|(key, steps)| (*key, steps.as_slice())).collect();
        for life in &stretch.lives {
            let column = if life.is_a { stretch.taken_a_pos } else { stretch.taken_b_pos };
            let mut held: Option<Option<Cell>> = None;
            for step in steps_of.get(&life.key).copied().unwrap_or_default() {
                let cell = step.cells.as_ref().and_then(|cells| cells.get(column).copied());
                if held != Some(cell) {
                    push_edge(&mut changes, first, last, i64::from(step.hour));
                    held = Some(cell);
                }
            }
        }
        for run in &stretch.totals.statuses {
            push_span_edges(&mut changes, first, last, run.first, run.last);
        }
        for (_, steps) in &stretch.totals.keys {
            for step in steps {
                push_edge(&mut changes, first, last, i64::from(step.hour));
            }
        }
        for life in &stretch.lives {
            push_edge(&mut changes, first, last, life.born);
            push_edge(&mut changes, first, last, life.gone.saturating_add(1));
            for grouping in &life.groupings {
                if i64::from(grouping.first) <= life.gone {
                    push_edge(&mut changes, first, last, i64::from(grouping.first));
                }
                let after = i64::from(grouping.last) + 1;
                if after <= life.gone {
                    push_edge(&mut changes, first, last, after);
                }
            }
            if let Some(runs) = enemy_tags.get(&life.enemy.0) {
                for &(a, b, _) in runs {
                    if i64::from(a) <= life.gone {
                        push_edge(&mut changes, first, last, i64::from(a));
                    }
                    let after = i64::from(b) + 1;
                    if after <= life.gone {
                        push_edge(&mut changes, first, last, after);
                    }
                }
            }
        }
    }
    for entry in wars.values() {
        for &(a, b, _) in &entry.names {
            push_span_edges(&mut changes, first, last, a, b);
        }
    }
    changes.sort_unstable();
    changes.dedup();
    CasualtyIndex { from, to: last, country_key, unknown, unlinked, holders, stretches, enemy_tags, wars, changes, warnings }
}

impl Product for CasualtyBreakdown {
    const KEY: &'static str = "casualty-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    const PREPARE: Option<Prepare<Self::Params>> = Some(prepare_wars);
    type Params = CasualtyBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: End];
    type Reply = CasualtyBreakdownReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CasualtyBreakdownParams::decl(), CasualtyBreakdownReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        dashboard_base::products::validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch_id = branch(&params.branch_id).map_err(invalid)?;
        let query = dashboard_base::queries::query(source, branch_id)?;
        let windows = manifest(&query)?;
        let Some(at) = windows.iter().position(|span| span.first_hour <= params.hour && params.hour <= span.last_hour) else {
            let next = windows.iter().map(|span| span.first_hour).filter(|&first| first > params.hour).min();
            let to = next.map_or(query.timeline.last.max(params.hour), |next| next - 1);
            let gap = finalize_index(
                query.timeline.first,
                params.hour,
                to,
                params.country.key().into_owned(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                HashMap::new(),
                HashMap::new(),
                Vec::new(),
            );
            return Ok(gap.at(params.hour));
        };
        let (first, last) = (windows[at].first_hour, windows[at].last_hour);
        let key = (params.branch_id.clone(), params.country.clone(), at, query.timeline.first, query.timeline.last);
        let held = breakdown::held(source, context, key, || dashboard_base::catalogue::Drawn::try_new(|| {
            build_index(source, &query, branch_id, &params.country, first, last, context, cancelled)
        }))?;
        let reply = held.get().at(params.hour);
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn prepare_wars(source: &std::sync::Arc<Source>, params: &CasualtyBreakdownParams, load: &LoadContext) -> io::Result<Option<std::sync::Arc<dashboard_base::products::read_ahead::Preparation>>> {
    let branch = branch(&params.branch_id).map_err(invalid)?;
    let query = dashboard_base::queries::query(source, branch)?;
    let windows = manifest(&query)?;
    let Some(window) = windows.iter().find(|span| span.first_hour <= params.hour && params.hour <= span.last_hour) else { return Ok(None) };
    let tags = HolderTags::new(&query);
    for stretch in query.timeline.within(window.first_hour, window.last_hour) {
        let holding = tags.holding(&params.country, stretch.first, stretch.last);
        if stretch_lifetimes(source, stretch.run, stretch.last)?.iter().any(|&(key, ..)| {
            let life = Lifetime::of(key);
            holding.iter().any(|(country, ..)| *country == life.a || *country == life.b)
        }) {
            return wars::prepare(source, branch, Some(params.country.clone()), load);
        }
    }
    Ok(None)
}

enum RunKey {}

type CachedRuns = dashboard_base::queries::RunCache<RunKey, EndedCounters>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    CachedRuns::forget(source);
}

pub fn cached_casualty_ends(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<EndedCounters>> {
    CachedRuns::held(&source.state::<CachedRuns>(), run, || run_ends(source, run))
}
