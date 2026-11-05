//! Special projects' replies, read from the family's series publication ([`super::emit`]): a
//! country's episodes, presentations and people through an hour, read from the ledger
//! preparation built ([`super::ledger`]), and the event feed's completions.

use super::{
    Builder, SpecialProjectsParams, SpecialProjectsReply, append_identities, build_reply,
    emit::{
        COMPLETIONS, Change, FAMILY, IDENTITY, LEDGER, LEXICAL, METADATA, Metadata, OBSERVED,
        PIECES, PROGRESS, READS, dimension, kind,
    },
    ledger::{self, Entry, Observation},
};
use dashboard_base::{
    catalogue,
    prepared::{codec, engine, series::LateIndex},
    products::{
        MAX_PRODUCT_REPLY_BYTES,
        country_history::{self, CountryIdentityRun},
    },
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span, family::Directory,
    query::EventView,
};
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
            observation: codec::decode(&bytes)?,
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
    reply_with(snapshot, directory, params, through, false, cancelled)
}

/// [`reply`], its episodes built again from the country's observations where `rebuilt`, as
/// a selection of some of its holders builds them.
#[cfg(test)]
pub(super) fn rebuilt_reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &SpecialProjectsParams,
    through: i32,
    cancelled: &AtomicBool,
) -> io::Result<SpecialProjectsReply> {
    reply_with(snapshot, directory, params, through, true, cancelled)
}

fn reply_with<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &SpecialProjectsParams,
    through: i32,
    rebuilt: bool,
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
    // Every observation by then is of a holder its identities hold by then.
    let whole = params.selection_tags.is_empty()
        || all_ids
            .iter()
            .all(|id| selected.contains(&(id.slot, id.country_id)));
    all_ids.retain(|id| {
        params.selection_tags.is_empty() || selected.contains(&(id.slot, id.country_id))
    });
    let family = |measure: u64| directory.key(FAMILY, measure, None);
    let mut builder = Builder {
        campaign_start: Some(metadata.start),
        ..Default::default()
    };
    let mut refusals = catalogue::Refusals::default();
    if whole && !rebuilt {
        if let Some(ledger) = key(&params.tag, LEDGER) {
            let pieces = key(&params.tag, PIECES)
                .ok_or_else(|| invalid("Prepared project pieces are missing"))?;
            replay(
                snapshot,
                (ledger, pieces),
                through,
                &mut builder,
                &mut refusals,
                cancelled,
            )?;
        }
    } else if !all_ids.is_empty() {
        // Some of its holders alone: the episodes of the others' hours are not the selection's,
        // and those of the selected holders go on over them.
        rebuild(
            snapshot,
            directory,
            params,
            (!whole).then_some(&selected),
            through,
            &mut builder,
            &mut refusals,
            cancelled,
        )?;
    }
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

/// The country's ledger (`keys`: its LEDGER and PIECES keys) through `through`: every entry
/// dated by then, each piece clipped to it.
fn replay<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    (ledger, pieces): (u64, u64),
    through: i32,
    builder: &mut Builder,
    refusals: &mut catalogue::Refusals,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let _cpu = replay_frame::cpu::scope_phase("projects:query_ledger");
    // Each piece's ordinal and last hour; ordinals rise with their hours.
    let ends: Vec<(i128, i32)> = engine::runs(snapshot, pieces, i32::MIN, through)?
        .into_iter()
        .map(|run| (run.value, run.span.last))
        .collect();
    let end = |piece: u64| {
        let piece = i128::from(piece);
        ends.binary_search_by_key(&piece, |&(ordinal, _)| ordinal)
            .ok()
            .map(|at| ends[at].1)
    };
    for (_, bytes) in events(snapshot, ledger, through, kind::LEDGER, cancelled)? {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let entries: Vec<Entry> = codec::decode(&bytes)?;
        for entry in entries {
            entry.apply(builder, refusals, end)?;
        }
    }
    Ok(())
}

/// The country's episodes built again from its observations through `through`, those of a
/// holder `selected` does not hold left out.
#[allow(clippy::too_many_arguments)]
fn rebuild<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &SpecialProjectsParams,
    selected: Option<&std::collections::BTreeSet<(u32, u32)>>,
    through: i32,
    builder: &mut Builder,
    refusals: &mut catalogue::Refusals,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let actor = directory.actor(params.tag.as_bytes());
    let Some(key) = actor.and_then(|actor| directory.key(actor, OBSERVED, None)) else {
        return Ok(());
    };
    let selected_intervals = intervals(snapshot, key, through, cancelled)?;
    let family = |measure: u64| directory.key(FAMILY, measure, None);
    let mut lexical = None;
    let _cpu = replay_frame::cpu::scope_phase("projects:query_episodes");
    for interval in &selected_intervals {
        let _cpu = replay_frame::cpu::scope_phase("projects:query_interval");
        dashboard_text::metadata::check_cancel(cancelled)?;
        if selected.is_some_and(|selected| !selected.contains(&interval.observation.identity)) {
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
            ledger::apply_interval(
                builder,
                &params.tag,
                &interval.observation,
                hour,
                end,
                &facts,
                refusals,
            );
            if end == interval.last {
                break;
            }
            hour = end
                .checked_add(1)
                .ok_or_else(|| invalid("Project interval overflow"))?;
        }
    }
    Ok(())
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
            changes.push(codec::decode(&bytes)?);
        }
    }
    Ok(Feed::of(
        changes,
        metadata.available,
        metadata.completion_warnings,
        metadata.content_error,
    ))
}
