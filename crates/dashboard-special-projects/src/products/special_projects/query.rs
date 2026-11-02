//! Special projects' replies, read from the family's series publication ([`super::emit`]): a
//! country's episodes, presentations and people built from its observations through an hour,
//! and the event feed's completions.

use super::{
    Builder, Look, ProjectDate, ProjectFacts, ProjectState,
    SpecialProjectRun, SpecialProjectsParams, SpecialProjectsReply, append_identities,
    build_reply,
    emit::{
        COMPLETIONS, Change, FAMILY, IDENTITY, LEXICAL, METADATA, Metadata, OBSERVED, PROGRESS,
        READS, dimension, kind,
    },
};
use dashboard_base::{
    catalogue,
    prepared::{engine, series::LateIndex},
    products::{
        MAX_PRODUCT_REPLY_BYTES,
        country_groups::HourSpan,
        country_history::{self, CountryIdentityRun},
    },
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span, family::Directory,
    query::EventView,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// The bytes an event page reads at once.
const PAGE: usize = 1 << 20;

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// The family's metadata; none before its first cut.
fn metadata(directory: &Directory) -> io::Result<Option<Metadata>> {
    directory
        .scalar(METADATA)
        .map(|scalar| -> io::Result<Metadata> {
            let text = scalar
                .text()
                .ok_or_else(|| invalid("Prepared project metadata is not text"))?;
            Ok(serde_json::from_slice(text)?)
        })
        .transpose()
}

/// An observation as its event holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Observation {
    identity: (u32, u32),
    projects: Option<Vec<(ProjectFacts, Look)>>,
}

/// An observation and the hours it holds.
struct Interval {
    first: i32,
    last: i32,
    observation: Observation,
}

/// The text of `fields`' only cell.
fn text<P: PageRead>(snapshot: &mut Snapshot<'_, P>, fields: dashboard_series::Blob) -> io::Result<Vec<u8>> {
    let Cell::Text(blob) = snapshot.field(fields, 0)? else {
        return Err(invalid("A prepared project event holds no text"));
    };
    let length = usize::try_from(blob.len).map_err(|_| invalid("Prepared project text length"))?;
    let mut bytes = vec![0_u8; length];
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok(bytes)
}

/// The events of `key` dated by `through`, in order, each with its fields' text.
fn events<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    through: i32,
    expected: u64,
    cancelled: &AtomicBool,
) -> io::Result<Vec<(i32, Vec<u8>)>> {
    let mut views = Vec::new();
    let selection = [Selection {
        key,
        membership: None,
    }];
    snapshot.each_event(
        EventQuery {
            selection: &selection,
            native: Span {
                first: i32::MIN,
                last: through,
            },
            as_of: None,
            order: EventOrder::Native,
        },
        PAGE,
        |event| {
            views.push(event);
            Ok(())
        },
    )?;
    let mut events = Vec::with_capacity(views.len());
    for view in views {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let EventView::Family(event) = view else {
            return Err(invalid("A prepared project event is not the family's"));
        };
        if event.kind != expected {
            return Err(invalid("A prepared project event is of another kind"));
        }
        events.push((event.native_hour, text(snapshot, event.fields)?));
    }
    Ok(events)
}

/// The run of `key` holding `hour`: its value and last hour.
fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<(i128, i32)>> {
    Ok(engine::run_at(snapshot, key, hour)?.map(|(run, _)| (run.value, run.span.last)))
}

/// The project observations of `key` through `through`, each interval ending by then.
fn intervals<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    through: i32,
    cancelled: &AtomicBool,
) -> io::Result<Vec<Interval>> {
    let _cpu = replay_frame::cpu::scope_phase("projects:query_intervals");
    let mut result = Vec::new();
    for (hour, bytes) in events(snapshot, key, through, kind::OBSERVATION, cancelled)? {
        let (_, last) = run_at(snapshot, key, hour)?
            .ok_or_else(|| invalid("Prepared project observation has no interval"))?;
        result.push(Interval {
            first: hour,
            last: last.min(through),
            observation: serde_json::from_slice(&bytes)?,
        });
    }
    Ok(result)
}

/// The identity runs of `key` through `through`, each ending by then.
fn identities<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    through: i32,
) -> io::Result<Vec<CountryIdentityRun>> {
    let _cpu = replay_frame::cpu::scope_phase("projects:query_identities");
    Ok(engine::runs(snapshot, key, i32::MIN, through)?
        .into_iter()
        .map(|run| CountryIdentityRun {
            first_hour: run.span.first,
            last_hour: run.span.last,
            slot: (run.value >> 32) as u32,
            country_id: run.value as u32,
        })
        .collect())
}

fn apply_interval(
    builder: &mut Builder,
    tag: &str,
    interval: &Interval,
    first: i32,
    last: i32,
    facts: &[ProjectFacts],
    refusals: &mut catalogue::Refusals,
) {
    let Some(projects) = &interval.observation.projects else {
        builder.last = Some((interval.observation.identity, last));
        builder.admitted = false;
        return;
    };
    if builder
        .identity
        .is_some_and(|old| old != interval.observation.identity)
    {
        builder.open.clear();
    }
    builder.identity = Some(interval.observation.identity);
    // Admit the first hour before processing the attribution boundary. Extending
    // coverage now would make that second observation overlap the whole interval.
    if builder
        .available
        .last()
        .is_some_and(|span| span.last_hour.checked_add(1) == Some(first))
    {
        builder.available.last_mut().unwrap().last_hour = first;
    } else {
        builder.available.push(HourSpan {
            first_hour: first,
            last_hour: first,
        });
    }
    builder.last = Some((interval.observation.identity, first));
    let mut present = |builder: &mut Builder,
                       ordinal: usize,
                       _index: usize,
                       fact: &ProjectFacts,
                       previous: Option<&SpecialProjectRun>,
                       _: Option<&ProjectDate>| {
        let look = &projects[ordinal].1;
        let id = builder.present_look(
            look,
            fact.definition_id,
            tag,
            fact.state == ProjectState::Completed,
            first,
            previous,
            refusals,
        );
        if let Some(count) = fact.iterations
            && !builder.iteration_texts.contains_key(&count)
        {
            builder
                .iteration_texts
                .insert(count, look.iteration_text.clone());
            refusals.merge(&look.iteration_refusals);
        }
        id
    };
    builder.observe_projects(first, facts, &mut present);
    // A completion first seen with a real scientist gains dated attribution on
    // its next consecutive observation. Process that boundary once, then extend.
    let second_needed = last > first
        && builder.open.values().any(|index| {
            let run = builder.episodes[*index].runs.last().unwrap();
            let info = &builder.presentations[run.presentation as usize];
            run.state == ProjectState::Completed
                && info.scientist_as_of.is_none()
                && builder.people[info.scientist as usize].id.is_some()
        });
    let processed = if second_needed {
        let second = first + 1;
        let one = Interval {
            first: second,
            last: second,
            observation: interval.observation.clone(),
        };
        apply_interval(builder, tag, &one, second, second, facts, refusals);
        second
    } else {
        first
    };
    for fact in facts {
        let key = (fact.local_id, fact.definition_id);
        if let Some(&index) = builder.open.get(&key) {
            builder.episodes[index].runs.last_mut().unwrap().last_hour = last;
            if let Some(reason) = &fact.warning {
                *builder.warnings.entry(reason.clone()).or_default() +=
                    usize::try_from(i64::from(last) - i64::from(processed))
                        .expect("nonnegative interval");
            }
        }
    }
    builder.available.last_mut().unwrap().last_hour = last;
    builder.last = Some((interval.observation.identity, last));
}

/// The reply of a branch the family has not published: nothing read yet.
pub fn unprepared(params: &SpecialProjectsParams) -> SpecialProjectsReply {
    build_reply(Builder::default(), &params.country(), false, Vec::new()).0
}

/// `params`' reply from the publication `snapshot` pins, its history read through `through`
/// alone, the last hour the query admitted. Its art stays the operands preparation recorded
/// ([`super::present_reply`] draws it).
pub fn reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &SpecialProjectsParams,
    through: i32,
    cancelled: &AtomicBool,
) -> io::Result<SpecialProjectsReply> {
    let Some(metadata) = metadata(directory)? else {
        return Ok(unprepared(params));
    };
    let key = |tag: &str, measure: u64| {
        directory
            .actor(tag.as_bytes())
            .and_then(|actor| directory.key(actor, measure, None))
    };
    let selected_intervals = match key(&params.tag, OBSERVED) {
        Some(key) => intervals(snapshot, key, through, cancelled)?,
        None => Vec::new(),
    };
    let mut history = country_history::RecordedHistories::default();
    let mut all_ids = match key(&params.tag, IDENTITY) {
        Some(key) => identities(snapshot, key, through)?,
        None => Vec::new(),
    };
    let mut selected_country = country_history::RecordedCountry::default();
    selected_country.identities = Arc::new(all_ids.clone());
    history
        .countries
        .insert(params.tag.clone(), selected_country);
    for root in &params.selection_tags {
        if history.countries.contains_key(root) {
            continue;
        }
        if let Some(root_key) = key(root, IDENTITY) {
            let mut country = country_history::RecordedCountry::default();
            country.identities = Arc::new(identities(snapshot, root_key, through)?);
            history.countries.insert(root.clone(), country);
        }
    }
    let selected = country_history::selected_identities(&history, &params.selection_tags);
    all_ids.retain(|id| {
        params.selection_tags.is_empty() || selected.contains(&(id.slot, id.country_id))
    });
    let actor = directory.actor(params.tag.as_bytes());
    let family = |measure: u64| directory.key(FAMILY, measure, None);
    let mut builder = Builder {
        campaign_start: Some(metadata.start),
        ..Default::default()
    };
    let mut lexical = None;
    let mut refusals = catalogue::Refusals::default();
    let cpu = replay_frame::cpu::scope_phase("projects:query_episodes");
    for interval in &selected_intervals {
        let _cpu = replay_frame::cpu::scope_phase("projects:query_interval");
        dashboard_text::metadata::check_cancel(cancelled)?;
        if !params.selection_tags.is_empty() && !selected.contains(&interval.observation.identity) {
            builder.last = None;
            builder.admitted = false;
            continue;
        }
        let Some(projects) = &interval.observation.projects else {
            if builder
                .identity
                .is_some_and(|old| old != interval.observation.identity)
            {
                builder.open.clear();
            }
            builder.identity = Some(interval.observation.identity);
            builder.last = Some((interval.observation.identity, interval.last));
            builder.admitted = false;
            continue;
        };
        // The branch's read marker dates each complete read; the interval holds an
        // observation through every hour of it.
        let mut hour = interval.first;
        loop {
            let mut end = interval.last;
            let (revision, until) = match family(LEXICAL) {
                Some(key) => run_at(snapshot, key, hour)?,
                None => None,
            }
            .ok_or_else(|| invalid("Prepared project lexical revision is missing"))?;
            if lexical != Some(revision) {
                builder.content_ids.clear();
                lexical = Some(revision);
            }
            end = end.min(until);
            let mut facts: Vec<_> = projects.iter().map(|(facts, _)| facts.clone()).collect();
            for fact in &mut facts {
                let key = actor
                    .and_then(|actor| {
                        directory.key(
                            actor,
                            PROGRESS,
                            Some(dimension(fact.local_id, fact.definition_id)),
                        )
                    })
                    .ok_or_else(|| invalid("Prepared project progress is missing"))?;
                let (progress, until) = run_at(snapshot, key, hour)?
                    .ok_or_else(|| invalid("Prepared project progress is missing"))?;
                fact.progress = i32::try_from(progress)
                    .map_err(|_| invalid("Prepared project progress is invalid"))?;
                end = end.min(until);
            }
            apply_interval(
                &mut builder,
                &params.tag,
                interval,
                hour,
                end,
                &facts,
                &mut refusals,
            );
            if end == interval.last {
                break;
            }
            hour = end
                .checked_add(1)
                .ok_or_else(|| invalid("Project interval overflow"))?;
        }
    }
    drop(cpu);
    let _cpu = replay_frame::cpu::scope_phase("projects:query_reads_finish");
    if let Some(key) = family(READS) {
        for span in &builder.available {
            dashboard_text::metadata::check_cancel(cancelled)?;
            for run in engine::runs(snapshot, key, span.first_hour, span.last_hour)? {
                let count = usize::try_from(i64::from(run.span.last) - i64::from(run.span.first) + 1)
                    .map_err(|_| invalid("Project read interval is invalid"))?;
                // Even a one-digit hour plus its separator takes two JSON bytes.
                if builder
                    .reads
                    .len()
                    .saturating_add(count)
                    .saturating_mul(2)
                    .saturating_sub(1)
                    > MAX_PRODUCT_REPLY_BYTES
                {
                    return Err(io::Error::other(
                        "Special projects reply exceeds its transport reply budget",
                    ));
                }
                builder.reads.try_reserve(count).map_err(|error| {
                    io::Error::other(format!(
                        "{}: {error}",
                        dashboard_core::transport::CAPACITY_REACHED
                    ))
                })?;
                builder.reads.extend(run.span.first..=run.span.last);
            }
        }
    }
    let mut warnings = metadata.warnings;
    warnings.extend(refusals.lines("Project texts"));
    warnings.extend(
        metadata
            .scoped_warnings
            .get(&params.tag)
            .into_iter()
            .flatten()
            .cloned(),
    );
    let mut reply = build_reply(
        builder,
        &params.country(),
        metadata.definitions_empty,
        warnings,
    )
    .0;
    if let Some(reason) = metadata.content_error {
        reply.warnings.push(reason);
    }
    if !reply.projects.is_empty() {
        reply.warnings.push(
            "Project icons and portraits are visual assets outside content hash coverage.".into(),
        );
    }
    append_identities(&mut reply, &all_ids.iter().collect::<Vec<_>>())?;
    Ok(reply)
}

/// The key every completion is listed under.
pub const FEED_KEY: u64 = 0;

/// The event feed's view of the completions: every change in order, by the hour it was read,
/// with the changes read after their dates, and each episode's changes.
#[derive(Default)]
pub struct Feed {
    pub changes: Vec<Change>,
    pub late: LateIndex,
    /// Whether any country's projects were read complete.
    pub available: bool,
    pub warnings: Vec<String>,
    pub content_error: Option<String>,
    /// Each episode's changes: the hour each was read, its ordinal and whether it presents a
    /// completion.
    episodes: HashMap<u64, Vec<(i32, u64, bool)>>,
}

impl Feed {
    /// The feed of `changes`, in the order they were read.
    pub fn of(
        changes: Vec<Change>,
        available: bool,
        warnings: Vec<String>,
        content_error: Option<String>,
    ) -> Self {
        let mut feed = Self {
            available,
            warnings,
            content_error,
            ..Self::default()
        };
        for (ordinal, change) in (0..).zip(&changes) {
            feed.late
                .insert(change.date.hour, change.observed_hour, FEED_KEY, ordinal);
            feed.episodes.entry(change.id).or_default().push((
                change.observed_hour,
                ordinal,
                change.completion.is_some(),
            ));
        }
        feed.changes = changes;
        feed
    }

    /// Whether the change at `ordinal` is its episode's completion at `live`: the last change
    /// read by then, presenting a completion.
    pub fn current(&self, id: u64, ordinal: u64, live: i32) -> bool {
        self.episodes.get(&id).is_some_and(|changes| {
            let at = changes.partition_point(|&(hour, _, _)| hour <= live);
            at > 0 && changes[at - 1].1 == ordinal && changes[at - 1].2
        })
    }

    /// The bytes it holds, about.
    pub fn bytes(&self) -> usize {
        self.changes.len() * 512 + self.episodes.len() * 64
    }
}

/// The completions the publication `snapshot` pins.
pub fn feed<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Feed> {
    let metadata = metadata(directory)?.unwrap_or_default();
    let mut changes = Vec::new();
    if let Some(key) = directory.key(FAMILY, COMPLETIONS, None) {
        let through = snapshot.publication().accepted.last;
        for (_, bytes) in events(snapshot, key, through, kind::CHANGE, cancelled)? {
            changes.push(serde_json::from_slice(&bytes)?);
        }
    }
    Ok(Feed::of(
        changes,
        metadata.available,
        metadata.completion_warnings,
        metadata.content_error,
    ))
}
