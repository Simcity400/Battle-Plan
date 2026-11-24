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
pub const GRANT_SOURCE: &str = "War goal grant source was not recorded";
pub const REMOVAL_REASON: &str = "War goal removal reason was not recorded";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incarnation {
    pub handle: u64,
    pub granted: Option<i32>,
    pub removed: Option<i32>,
    pub removed_exact: bool,
    pub country: Option<i64>,
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
}

fn value_at(runs: Option<&Runs>, hour: i32) -> Option<i64> {
    let runs = runs?;
    let at = runs.partition_point(|run| run.1 < hour);
    runs.get(at).filter(|run| run.0 <= hour).and_then(|run| run.2)
}

pub(crate) fn incarnations(
    handles: &HashMap<u128, Vec<(i32, i32, CoveredValue)>, foldhash::fast::RandomState>,
    attributes: &[HashMap<u128, Runs>; 5],
    first_read: Option<i32>,
    last: i32,
) -> Vec<Incarnation> {
    let mut by_handle: HashMap<u64, Vec<Segment>> = HashMap::new();
    for (key, runs) in handles {
        for (at, (start, end, value)) in runs.iter().enumerate() {
            let CoveredValue::Value(handle) = value else { continue };
            let unseen = |run: Option<&(i32, i32, CoveredValue)>| matches!(run, Some((_, _, CoveredValue::Uncovered)));
            by_handle.entry(*handle as u64).or_default().push(Segment {
                first: *start,
                last: *end,
                key: *key,
                starts_unseen: at > 0 && unseen(runs.get(at - 1)),
                ends_unseen: unseen(runs.get(at + 1)),
            });
        }
    }
    let mut out = Vec::new();
    for (handle, mut segments) in by_handle {
        segments.sort_by_key(|segment| segment.first);
        let mut merged: Vec<Segment> = Vec::new();
        for segment in segments {
            match merged.last_mut() {
                Some(held) if segment.first <= held.last.saturating_add(1) || (held.ends_unseen && segment.starts_unseen) => {
                    if segment.last >= held.last {
                        held.last = segment.last;
                        held.key = segment.key;
                        held.ends_unseen = segment.ends_unseen;
                    }
                }
                _ => merged.push(segment),
            }
        }
        for segment in merged {
            let granted = (first_read.is_none_or(|read| segment.first > read) && !segment.starts_unseen).then_some(segment.first);
            let removed = (segment.last < last).then(|| segment.last + 1);
            let at = |column: usize| value_at(attributes[column].get(&segment.key), segment.last);
            out.push(Incarnation {
                handle,
                granted,
                removed,
                removed_exact: removed.is_some() && !segment.ends_unseen,
                country: at(0),
                actor: at(1),
                recipient: at(2),
                goal_type: at(3),
                expires: at(4),
            });
        }
    }
    out.sort_by_key(|incarnation| (incarnation.removed.unwrap_or(i32::MAX), incarnation.handle));
    out
}

#[derive(Default)]
struct Kept(Option<(u64, Arc<Vec<Incarnation>>)>);

pub fn follow(source: &Source, branch: u64) -> io::Result<Arc<Vec<Incarnation>>> {
    let state = source.state::<Kept>();
    if let Some(held) = state.lock().unwrap_or_else(PoisonError::into_inner).0.as_ref().filter(|(at, _)| *at == branch).map(|(_, held)| Arc::clone(held)) {
        return Ok(held);
    }
    let table = TableId::CountryWargoals;
    let column = |name: &str| dashboard_declare::field(table, name);
    let handle = column("handle")?;
    let attributes = [column("country")?, column("actor")?, column("recipient")?, column("goal_type")?, column("expires")?];
    let query = dashboard_base::queries::query(source, branch)?;
    let recording = query.replay.recording();
    let mut all = Vec::new();
    for stretch in &query.timeline.stretches {
        let (handles, _) = cost::storage(|| lanes::keyed_covered_runs(recording, stretch.run, table, summary::Source::Cell(handle), None, stretch.first, stretch.last))?;
        let read = |at: usize| -> io::Result<HashMap<u128, Runs>> {
            Ok(cost::storage(|| lanes::runs(recording, stretch.run, table, attributes[at], stretch.first, stretch.last))?.into_iter().collect())
        };
        let columns = [read(0)?, read(1)?, read(2)?, read(3)?, read(4)?];
        let first_read = cost::storage(|| lanes::reads(recording, stretch.run, table, stretch.first, stretch.last))?.first().map(|span| span.0);
        all.extend(incarnations(&handles, &columns, first_read, stretch.last));
    }
    let held = Arc::new(all);
    state.lock().unwrap_or_else(PoisonError::into_inner).0 = Some((branch, Arc::clone(&held)));
    Ok(held)
}
