//! Raids' replies, read from the family's series publication ([`super::emit`]), pinned through
//! the coverage its gate accepted: the scan's result rebuilt from its episode chunks, reads,
//! tallies, presence and state, then answered as the row scan's result is.

use super::{
    RaidEpisode, RaidTallyRun, Raids, RaidsParams, RaidsReply, RaidsView, ScannedRaids,
    emit::{CHUNK_KIND, Chunk, COVERAGE, EPISODES, FAMILY, READS, SCHEMA, STATE, State, TALLY},
    prepared_content, present_log, reply,
};
use dashboard_base::{
    prepared::{coverage::Coverage, engine, published},
    products::{Product, country_groups::HourSpan, country_history::RecordedHistories},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span, family::Directory,
    query::EventView,
};
use dashboard_text::metadata::check_cancel;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// `key`'s runs through the accepted hours.
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
) -> io::Result<Vec<dashboard_series::Run>> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    let accepted = snapshot.publication().accepted;
    engine::runs(snapshot, key, accepted.first, accepted.last)
}

/// Whether raids were ever present, and the scan's result as the publication holds it for
/// `view`; none for the catalogue.
pub(crate) fn scanned<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    view: RaidsView,
    cancelled: &AtomicBool,
) -> io::Result<(bool, Option<ScannedRaids>)> {
    let state: State = match directory.scalar(STATE).and_then(|scalar| scalar.text()) {
        Some(text) => serde_json::from_slice(text)?,
        None => State::default(),
    };
    if view == RaidsView::Catalogue {
        return Ok((state.present, None));
    }
    let mut scanned = ScannedRaids {
        types: state.types,
        recording_present: state.recording_present,
        warnings: state.warnings,
        noted: state.noted,
        ..ScannedRaids::default()
    };
    for run in runs(snapshot, directory.key(FAMILY, READS, None))? {
        scanned.read_hours.extend(run.span.first..=run.span.last);
    }
    if view == RaidsView::Log {
        let mut episodes: BTreeMap<u64, RaidEpisode> = BTreeMap::new();
        let mut gather = |chunk: Chunk| {
            let episode = episodes.entry(chunk.ordinal).or_insert_with(|| RaidEpisode {
                id: chunk.episode.id.clone(),
                start: chunk.episode.start.clone(),
                runs: Vec::new(),
                preparation: Vec::new(),
            });
            episode.runs.extend(chunk.episode.runs);
            episode.preparation.extend(chunk.episode.preparation);
        };
        if let Some(key) = directory.key(FAMILY, EPISODES, None) {
            let accepted = snapshot.publication().accepted;
            let selection = [Selection {
                key,
                membership: None,
            }];
            let malformed = || invalid("A prepared raid episode is not valid");
            let mut cursor = None;
            loop {
                check_cancel(cancelled)?;
                let mut events = Vec::new();
                let page = snapshot.event_page(
                    EventQuery {
                        selection: &selection,
                        native: Span::new(i32::MIN, accepted.last)?,
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
                    if event.kind != CHUNK_KIND {
                        return Err(malformed());
                    }
                    let Cell::Text(text) = snapshot.field(event.fields, 0)? else {
                        return Err(malformed());
                    };
                    let mut bytes = vec![0; usize::try_from(text.len).map_err(|_| malformed())?];
                    snapshot.cell_bytes(text, 0, &mut bytes)?;
                    gather(serde_json::from_slice(&bytes)?);
                }
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
        }
        for (ordinal, episode) in state.open {
            gather(Chunk { ordinal, episode });
        }
        scanned.episodes = episodes.into_values().collect();
    } else {
        for (actor, described) in directory.actors.iter().enumerate() {
            check_cancel(cancelled)?;
            let actor = actor as u64;
            let Ok(tag) = std::str::from_utf8(&described.name) else {
                continue;
            };
            if let Some(key) = directory.key(actor, TALLY, None) {
                scanned.series.insert(
                    tag.to_owned(),
                    runs(snapshot, Some(key))?
                        .into_iter()
                        .map(|run| RaidTallyRun {
                            first_hour: run.span.first,
                            last_hour: run.span.last,
                            read_hour: run.span.first,
                            launched: (run.value >> 32) as u32,
                            succeeded: (run.value & i128::from(u32::MAX)) as u32,
                        })
                        .collect(),
                );
            }
            if let Some(key) = directory.key(actor, COVERAGE, None) {
                scanned.segments.insert(
                    tag.to_owned(),
                    runs(snapshot, Some(key))?
                        .into_iter()
                        .map(|run| HourSpan {
                            first_hour: run.span.first,
                            last_hour: run.span.last,
                        })
                        .collect(),
                );
            }
        }
    }
    Ok((state.present, Some(scanned)))
}

/// The reply to `params` from what the publication holds: whether raids were ever present and
/// the scan's result, a tally's countries named through `histories`.
pub(crate) fn answer(
    source: &Source,
    context: &LoadContext,
    params: &RaidsParams,
    (present, scanned): (bool, Option<ScannedRaids>),
    histories: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<RaidsReply> {
    let (content, warnings) = prepared_content(source, context)?;
    let available = if present || content.as_ref().is_some_and(|c| !c.raids.types.is_empty()) {
        Some(true)
    } else {
        content
            .as_ref()
            .filter(|c| c.raids.uncertain.is_empty())
            .map(|_| false)
    };
    check_cancel(cancelled)?;
    let mut reply = reply(
        params,
        content.as_deref(),
        available,
        warnings,
        scanned.as_ref(),
        histories,
        cancelled,
    )?;
    present_log(source, context, content.as_deref(), &mut reply)?;
    Ok(reply)
}

/// The coverage a reply answers through when the publication it read (through `through`; none
/// read, no hour) ends before the `accepted` coverage its gate gives: the hours after it pending.
pub(crate) fn answered(accepted: &Coverage, through: Option<i32>) -> Option<Coverage> {
    through
        .is_none_or(|last| last < accepted.last_hour)
        .then(|| Coverage {
            last_hour: through.unwrap_or(accepted.first_hour.saturating_sub(1)),
            complete: false,
            ..accepted.clone()
        })
}

/// A `raids` request, answered from the family's publication on the branch through the coverage
/// its gate accepted; the scan of no hours before it publishes.
pub fn query(
    source: &Source,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let params: RaidsParams = serde_json::from_value(params)?;
    Raids::validate(&params).map_err(dashboard_core::invalid)?;
    let branch = params.branch_id.parse::<u64>().expect("validated branch");
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|row| row.id == branch)
    {
        return Err(invalid("Branch is not in this recording"));
    }
    let tally = params.view == RaidsView::Tally;
    let mut prefix = dashboard_base::prepared::prefix::read_or_empty(
        source,
        context.content.as_ref(),
        if tally { "raids-tally" } else { Raids::KEY },
        branch,
        None,
        cancelled,
    )?;
    let accepted =
        (prefix.coverage.first_hour <= prefix.coverage.last_hour).then(|| prefix.coverage.clone());
    // Pinned through the accepted coverage, which a tally shares with the identities.
    let opened = match &accepted {
        Some(coverage) => published::open_through(
            source,
            context.content.as_ref(),
            "raids",
            SCHEMA,
            branch,
            (!coverage.complete).then_some(coverage.last_hour),
        )?,
        None => None,
    };
    let (published, through) = match &opened {
        Some(opened) => opened.read(cancelled, |snapshot, directory| {
            let through = snapshot.publication().accepted.last;
            Ok((scanned(snapshot, directory, params.view, cancelled)?, Some(through)))
        })?,
        None => (
            (
                false,
                (params.view != RaidsView::Catalogue).then(ScannedRaids::default),
            ),
            None,
        ),
    };
    if let Some(read) = accepted
        .as_ref()
        .and_then(|coverage| answered(coverage, through))
    {
        prefix = dashboard_base::prepared::prefix::Prefix::new(source, branch, read, None);
    }
    let histories = match through.filter(|_| tally) {
        Some(last) => {
            let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
            dashboard_base::products::country_history::read_prepared_histories(
                &store,
                &key,
                branch,
                Some(last),
            )?
        }
        None => None,
    };
    let histories = histories.or_else(|| tally.then(Default::default));
    let reply = answer(
        source,
        context,
        &params,
        published,
        histories.as_deref(),
        cancelled,
    )?;
    let mut data = serde_json::to_value(reply)?;
    prefix.mark_coverage(&mut data, "coverage")?;
    Ok(data)
}
