//! Subjects' replies, read from the family's series publication ([`super::emit`]): the
//! Subjects reply and what the event feed reads of the relations' starts.

use super::{
    Feed, LevelStep, SubjectReply, SubjectRun, SubjectStart,
    emit::{COUNTS, FAMILY, RELATION, STARTS, UNOBSERVED, counts_of, kind, levels_of, relation_of},
    sort_runs, warnings,
};
use dashboard_base::{
    Source,
    prepared::engine,
    products::{
        LoadContext,
        country_groups::{HourSpan, mark_unobserved},
    },
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, codec::Binary,
    family::Directory, query::EventView,
};
use dashboard_text::metadata::{branch_bounds, check_cancel};
use std::{io, sync::atomic::AtomicBool};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// The first and last hour of `branch`.
fn bounds(source: &Source, branch: u64) -> io::Result<(i32, i32)> {
    let replay = source.replay_ref();
    replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start))
        .ok_or_else(|| invalid("Unknown subjects branch"))
}

/// The tag `actor` names.
fn tag(directory: &Directory, actor: u64) -> io::Result<String> {
    directory
        .actors
        .get(actor as usize)
        .and_then(|actor| String::from_utf8(actor.name.to_vec()).ok())
        .ok_or_else(|| invalid("A prepared subject relation names an unknown country"))
}

/// The reply on `branch` from `snapshot`, its publication pinned through the family's accepted
/// coverage: every run it holds, the hours whose relations were not observed and, once the
/// branch is `complete`, the hours after its last as unobserved too.
#[allow(clippy::too_many_arguments)]
pub fn reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    source: &Source,
    branch: u64,
    branch_id: &str,
    context: &LoadContext,
    complete: bool,
    cancelled: &AtomicBool,
) -> io::Result<SubjectReply> {
    let (first, last) = bounds(source, branch)?;
    let accepted = snapshot.publication().accepted;
    let levels = levels_of(directory)?;
    let mut runs = Vec::new();
    for subject in 0..directory.actors.len() as u64 {
        for &(dimension, key) in directory.dimensions(subject, RELATION) {
            check_cancel(cancelled)?;
            let (overlord, start_hour) = relation_of(dimension);
            let (overlord, subject) = (tag(directory, overlord)?, tag(directory, subject)?);
            let mut held: Option<SubjectRun> = None;
            for run in engine::runs(snapshot, key, accepted.first, accepted.last)? {
                let level = match run.value {
                    0 => None,
                    value => Some(
                        usize::try_from(value - 1)
                            .ok()
                            .and_then(|id| levels.get(id))
                            .cloned()
                            .ok_or_else(|| invalid("A prepared subject level is unknown"))?,
                    ),
                };
                let (from, to) = (run.span.first, run.span.last);
                let continues = !run.break_before
                    && held
                        .as_ref()
                        .is_some_and(|held| held.last_hour.checked_add(1) == Some(from));
                if !continues {
                    runs.extend(held.take());
                }
                let held = held.get_or_insert_with(|| SubjectRun {
                    overlord: overlord.clone(),
                    subject: subject.clone(),
                    start_hour,
                    first_hour: from,
                    last_hour: to,
                    steps: Vec::new(),
                });
                held.last_hour = to;
                match held.steps.last_mut() {
                    Some(step) if step.level == level && step.last_hour.checked_add(1) == Some(from) => {
                        step.last_hour = to;
                    }
                    _ => held.steps.push(LevelStep {
                        first_hour: from,
                        last_hour: to,
                        level,
                    }),
                }
            }
            runs.extend(held);
        }
    }
    sort_runs(&mut runs);
    let mut unobserved = Vec::new();
    if first < accepted.first {
        mark_unobserved(&mut unobserved, first, accepted.first - 1);
    }
    if let Some(key) = directory.key(FAMILY, UNOBSERVED, None) {
        for run in engine::runs(snapshot, key, accepted.first, accepted.last)? {
            if run.value == 1 {
                mark_unobserved(&mut unobserved, run.span.first, run.span.last);
            }
        }
    }
    if complete && accepted.last < last {
        mark_unobserved(&mut unobserved, accepted.last + 1, last);
    }
    let counts = counts_of(directory.scalar(COUNTS))?;
    super::reply(
        source,
        branch_id,
        context,
        runs,
        unobserved,
        &warnings(&counts),
    )
}

/// The reply on `branch` before the family publishes it: nothing observed, and every hour of a
/// `complete` branch unobserved.
pub fn unpublished(
    source: &Source,
    branch: u64,
    branch_id: &str,
    context: &LoadContext,
    complete: bool,
) -> io::Result<SubjectReply> {
    let (first, last) = bounds(source, branch)?;
    let unobserved = if complete && first <= last {
        vec![HourSpan {
            first_hour: first,
            last_hour: last,
        }]
    } else {
        Vec::new()
    };
    super::reply(source, branch_id, context, Vec::new(), unobserved, &[])
}

/// What the event feed reads of the publication: whether some hour's relations were observed,
/// the warnings, and every start in the order it was published.
pub fn feed<P: PageRead>(snapshot: &mut Snapshot<'_, P>, directory: &Directory) -> io::Result<Feed> {
    let malformed = || invalid("A prepared subject start is not valid");
    let accepted = snapshot.publication().accepted;
    let mut starts: Vec<(u64, SubjectStart)> = Vec::new();
    if let Some(key) = directory.key(FAMILY, STARTS, None) {
        let selection = [Selection {
            key,
            membership: None,
        }];
        let mut cursor = None;
        loop {
            let mut events = Vec::new();
            let page = snapshot.event_page(
                EventQuery {
                    selection: &selection,
                    native: accepted,
                    as_of: None,
                    order: EventOrder::Native,
                },
                cursor,
                1 << 20,
                |event| {
                    events.push(event);
                    Ok(())
                },
            )?;
            for event in events {
                let EventView::Family(event) = event else {
                    return Err(malformed());
                };
                if event.kind != kind::START || event.fields.len != 3 * Cell::SIZE as u64 {
                    return Err(malformed());
                }
                let actor = |cell| match cell {
                    Cell::Unsigned(actor) => Ok(actor),
                    _ => Err(malformed()),
                };
                let subject = actor(snapshot.field(event.fields, 0)?)?;
                let overlord = actor(snapshot.field(event.fields, 1)?)?;
                let Cell::Boolean(exact) = snapshot.field(event.fields, 2)? else {
                    return Err(malformed());
                };
                starts.push((
                    event.sequence,
                    SubjectStart {
                        subject: tag(directory, subject)?,
                        overlord: tag(directory, overlord)?,
                        hour: event.native_hour,
                        read_hour: event.read_hour.ok_or_else(malformed)?,
                        exact,
                    },
                ));
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
    }
    // Published in the order the scan found them, each hour's by date, subject and overlord.
    starts.sort_unstable_by_key(|&(sequence, _)| sequence);
    let counts = counts_of(directory.scalar(COUNTS))?;
    Ok(Feed::new(
        starts.into_iter().map(|(_, start)| start).collect(),
        counts.observed,
        warnings(&counts),
    ))
}
