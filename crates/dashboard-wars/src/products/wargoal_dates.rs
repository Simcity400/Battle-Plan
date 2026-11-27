use dashboard_base::Source;
use dashboard_query::cost;
use replay_columnar::summary;
use replay_data::rows::TableId;
use replay_lanes::lanes::{self, CoveredValue, Runs};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, PoisonError},
};

pub const HELD_AT_BRANCH_START: &str = "The war goal was already held when this branch began";
pub const FIRST_SEEN_AFTER_GAP: &str = "The war goal was first seen after an unrecorded stretch, so its grant date was not recorded";
pub const GRANT_NOT_READ: &str = "The war goal's grant was not read";
pub const REMOVAL_REASON: &str = "War goal removal reason was not recorded";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incarnation {
    pub handle: u64,
    pub granted: Option<i32>,
    pub removed: Option<i32>,
    pub removed_exact: bool,
    pub granted_missing: Option<&'static str>,
    pub last_seen: i32,
    pub country: u32,
    pub actor: Option<i64>,
    pub recipient: Option<i64>,
    pub goal_type: Option<i64>,
    pub expires: Option<i64>,
}

struct Segment {
    first: i32,
    last: i32,
    key: u128,
    starts_unseen: bool,
    ends_unseen: bool,
    absent_at: Option<i32>,
}

fn value_at(runs: Option<&Runs>, hour: i32) -> Option<i64> {
    let runs = runs?;
    let at = runs.partition_point(|run| run.1 < hour);
    runs.get(at).filter(|run| run.0 <= hour).and_then(|run| run.2)
}

pub(crate) fn incarnations<'a>(
    handles: impl IntoIterator<Item = (&'a u128, &'a Vec<(i32, i32, CoveredValue)>)>,
    attributes: &[HashMap<u128, Runs>; 4],
    first_read: Option<i32>,
    branch_start: Option<i32>,
    last: i32,
) -> Vec<Incarnation> {
    let mut by_handle: HashMap<u64, Vec<Segment>> = HashMap::new();
    for (key, runs) in handles {
        for (at, (start, end, value)) in runs.iter().enumerate() {
            let CoveredValue::Value(handle) = value else { continue };
            let unseen = |run: Option<&(i32, i32, CoveredValue)>| matches!(run, Some((_, _, CoveredValue::Uncovered)));
            let absent_at = runs[at + 1..].iter().find(|run| !matches!(run.2, CoveredValue::Uncovered)).map(|run| run.0);
            by_handle.entry(*handle as u64).or_default().push(Segment {
                first: *start,
                last: *end,
                key: *key,
                starts_unseen: at > 0 && unseen(runs.get(at - 1)),
                ends_unseen: unseen(runs.get(at + 1)),
                absent_at,
            });
        }
    }
    let mut out = Vec::new();
    for (handle, mut segments) in by_handle {
        segments.sort_by_key(|segment| segment.first);
        let mut merged: Vec<Segment> = Vec::new();
        for segment in segments {
            match merged.last_mut() {
                Some(held) if segment.first <= held.last.saturating_add(1) || (held.ends_unseen && segment.starts_unseen && held.absent_at.is_none_or(|at| at >= segment.first)) => {
                    if segment.last >= held.last {
                        held.last = segment.last;
                        held.key = segment.key;
                        held.ends_unseen = segment.ends_unseen;
                        held.absent_at = segment.absent_at;
                    }
                }
                _ => merged.push(segment),
            }
        }
        for segment in merged {
            let held_at_first_read = first_read.is_some_and(|read| segment.first <= read);
            let granted = (first_read.is_some() && !held_at_first_read && !segment.starts_unseen).then_some(segment.first);
            let granted_missing = match granted {
                Some(_) => None,
                None if first_read.is_none() => Some(GRANT_NOT_READ),
                None if held_at_first_read && first_read == branch_start => Some(HELD_AT_BRANCH_START),
                None => Some(FIRST_SEEN_AFTER_GAP),
            };
            let removed = if segment.ends_unseen { segment.absent_at } else { (segment.last < last).then(|| segment.last + 1) };
            let at = |column: usize| value_at(attributes[column].get(&segment.key), segment.last);
            out.push(Incarnation {
                handle,
                granted,
                removed,
                removed_exact: removed.is_some() && !segment.ends_unseen,
                granted_missing,
                last_seen: segment.last,
                country: (segment.key >> 32) as u32,
                actor: at(0),
                recipient: at(1),
                goal_type: at(2),
                expires: at(3),
            });
        }
    }
    out.sort_by_key(|incarnation| (incarnation.removed.unwrap_or(i32::MAX), incarnation.handle));
    out
}

pub struct Incarnations {
    all: Vec<Incarnation>,
    by_handle: HashMap<u64, Vec<usize>>,
    by_country: HashMap<u32, Vec<usize>>,
}

impl Incarnations {
    pub fn new(all: Vec<Incarnation>) -> Self {
        let mut by_handle: HashMap<u64, Vec<usize>> = HashMap::new();
        let mut by_country: HashMap<u32, Vec<usize>> = HashMap::new();
        for (at, incarnation) in all.iter().enumerate() {
            by_handle.entry(incarnation.handle).or_default().push(at);
            by_country.entry(incarnation.country).or_default().push(at);
        }
        Self { all, by_handle, by_country }
    }

    pub fn of_handle(&self, handle: u64) -> impl Iterator<Item = &Incarnation> {
        self.by_handle.get(&handle).into_iter().flatten().map(|at| &self.all[*at])
    }

    pub fn removed(&self) -> impl Iterator<Item = &Incarnation> {
        self.all.iter().filter(|incarnation| incarnation.removed.is_some())
    }

    pub fn of_country(&self, country: u32) -> impl Iterator<Item = &Incarnation> {
        self.by_country.get(&country).into_iter().flatten().map(|at| &self.all[*at])
    }
}

fn split_absent(runs: &[(i32, i32, CoveredValue)], complete: &[(i32, i32)], first: i32) -> Vec<(i32, i32, CoveredValue)> {
    let mut out = Vec::with_capacity(runs.len());
    let push_gap = |out: &mut Vec<(i32, i32, CoveredValue)>, mut from: i32, to: i32| {
        if from > to {
            return;
        }
        let mut at = complete.partition_point(|&(_, end)| end < from);
        while from <= to {
            let Some(&(c_first, c_last)) = complete.get(at) else {
                out.push((from, to, CoveredValue::Uncovered));
                break;
            };
            if c_first > to {
                out.push((from, to, CoveredValue::Uncovered));
                break;
            }
            if from < c_first {
                out.push((from, c_first - 1, CoveredValue::Uncovered));
                from = c_first;
            }
            let covered_end = c_last.min(to);
            out.push((from, covered_end, CoveredValue::Unvalued));
            if covered_end == i32::MAX {
                break;
            }
            from = covered_end + 1;
            at += 1;
        }
    };
    if let Some(&(start, _, _)) = runs.first()
        && start > first
    {
        push_gap(&mut out, first, start - 1);
    }
    for &(start, end, value) in runs {
        if value == CoveredValue::Uncovered {
            push_gap(&mut out, start, end);
        } else {
            out.push((start, end, value));
        }
    }
    out
}

#[derive(Default)]
struct Kept(Option<(u64, Arc<Incarnations>)>);

pub fn follow(source: &Source, branch: u64) -> io::Result<Arc<Incarnations>> {
    let state = source.state::<Kept>();
    if let Some(held) = state.lock().unwrap_or_else(PoisonError::into_inner).0.as_ref().filter(|(at, _)| *at == branch).map(|(_, held)| Arc::clone(held)) {
        return Ok(held);
    }
    let table = TableId::CountryWargoals;
    let column = |name: &str| dashboard_declare::field(table, name);
    let handle = column("handle")?;
    let attributes = [column("actor")?, column("recipient")?, column("goal_type")?, column("expires")?];
    let query = dashboard_base::queries::query(source, branch)?;
    let recording = query.replay.recording();
    let mut all = Vec::new();
    for (index, stretch) in query.timeline.stretches.iter().enumerate() {
        let (handles, complete) = cost::storage(|| lanes::keyed_covered_runs(recording, stretch.run, table, summary::Source::Cell(handle), None, stretch.first, stretch.last))?;
        let handles: Vec<(u128, Vec<(i32, i32, CoveredValue)>)> = handles
            .into_iter()
            .map(|(key, runs)| (key, split_absent(&runs, &complete, stretch.first)))
            .collect();
        let read = |at: usize| -> io::Result<HashMap<u128, Runs>> {
            Ok(cost::storage(|| lanes::runs(recording, stretch.run, table, attributes[at], stretch.first, stretch.last))?.into_iter().collect())
        };
        let columns = [read(0)?, read(1)?, read(2)?, read(3)?];
        let first_read = cost::storage(|| lanes::reads(recording, stretch.run, table, stretch.first, stretch.last))?.first().map(|span| span.0);
        all.extend(incarnations(handles.iter().map(|(key, runs)| (key, runs)), &columns, first_read, (index == 0).then_some(query.timeline.first), stretch.last));
    }
    let held = Arc::new(Incarnations::new(all));
    state.lock().unwrap_or_else(PoisonError::into_inner).0 = Some((branch, Arc::clone(&held)));
    Ok(held)
}
