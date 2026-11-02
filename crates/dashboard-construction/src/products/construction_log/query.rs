//! Construction's replies, read from the family's series publication ([`super::emit`]) as of
//! the coverage the query accepts: the manifest, a window of the log, and a country's spending.
//! Building and state names and art come from the recorded content at query time; nothing reads
//! the recording's frames.

use super::{
    COST_SCALE, ConstructionLog, ConstructionLogManifestReply, ConstructionLogParams,
    ConstructionLogReply, ConstructionOutputRead, ConstructionProject, ContentNames, Cut,
    NameSource, ScannedLog, count_warnings,
    emit::{
        FAMILY, LEAF, LeafMetadata, LeafRecord, NAME, READS, ReadCounts, SCHEMA, SHARED, Shared,
        WINDOW, gap_of, kind, run_of,
    },
    present_stored, windows,
};
use crate::products::construction_spending::{
    ConstructionSpendingReply, SpendingGap, SpendingLeaf, SpendingUnknown, SpentStep,
};
use dashboard_base::{
    Source,
    prepared::{engine, prefix::Prefix, published},
    products::Product,
};
use dashboard_core::config::{ContentOptions, LoadContext};
use dashboard_series::{
    Cell, Event, EventOrder, EventQuery, Observation, PageRead, Selection, Snapshot, Span,
    codec::Binary,
    family::Directory,
    query::EventView,
};
use serde::de::DeserializeOwned;
use std::{
    collections::BTreeSet,
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// The bytes an events page reads at most.
const BUDGET: usize = 1 << 20;

fn invalid(what: &str) -> io::Error {
    dashboard_core::invalid(what.to_owned())
}

/// `key`'s events over `span` (clipped to the accepted hours), in the order they were written.
fn events_of<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    span: Span,
) -> io::Result<Vec<Event>> {
    let Some(span) = span.intersect(snapshot.publication().accepted) else {
        return Ok(Vec::new());
    };
    let selection = [Selection {
        key,
        membership: None,
    }];
    let mut events = Vec::new();
    snapshot.each_event(
        EventQuery {
            selection: &selection,
            native: span,
            as_of: None,
            order: EventOrder::Publication,
        },
        BUDGET,
        |view| {
            if let EventView::Family(event) = view {
                events.push(event);
            }
            Ok(())
        },
    )?;
    Ok(events)
}

/// `key`'s last event through `through`.
fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    through: i32,
) -> io::Result<Option<Event>> {
    let selection = Selection {
        key,
        membership: None,
    };
    Ok(match snapshot.latest_event(selection, through, None)? {
        Some(EventView::Family(event)) => Some(event),
        _ => None,
    })
}

/// An event's `at`th field.
fn field<P: PageRead>(snapshot: &mut Snapshot<'_, P>, event: &Event, at: u64) -> io::Result<Cell> {
    if event.fields.len < (at + 1) * Cell::SIZE as u64 {
        return Err(invalid("A construction event lacks a field"));
    }
    snapshot.field(event.fields, at)
}

/// An event's one field, JSON text, read back.
fn json<T: DeserializeOwned, P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: &Event,
) -> io::Result<T> {
    let Cell::Text(blob) = field(snapshot, event, 0)? else {
        return Err(invalid("A construction event holds no text"));
    };
    let length = usize::try_from(blob.len).map_err(|_| invalid("A construction text is too long"))?;
    let mut text = vec![0_u8; length];
    snapshot.cell_bytes(blob, 0, &mut text)?;
    Ok(serde_json::from_slice(&text)?)
}

/// An integer field, none where it is null.
fn integer<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: &Event,
    at: u64,
) -> io::Result<Option<i128>> {
    match field(snapshot, event, at)? {
        Cell::Integer(value) => Ok(Some(value)),
        Cell::Null => Ok(None),
        _ => Err(invalid("A construction field is not an integer")),
    }
}

fn hour_of(value: Option<i128>) -> io::Result<Option<i32>> {
    value
        .map(|value| i32::try_from(value).map_err(|_| invalid("A construction hour is not valid")))
        .transpose()
}

/// An output read as its event holds it.
fn output_read<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: &Event,
) -> io::Result<ConstructionOutputRead> {
    let reason = match field(snapshot, event, 3)? {
        Cell::Unsigned(code) => {
            Some(gap_of(code).ok_or_else(|| invalid("A construction output reason is not valid"))?)
        }
        Cell::Null => None,
        _ => return Err(invalid("A construction output reason is not valid")),
    };
    let Cell::Boolean(observed_since_creation) = field(snapshot, event, 4)? else {
        return Err(invalid("A construction output read is not valid"));
    };
    Ok(ConstructionOutputRead {
        known_at: event.native_hour,
        since_hour: hour_of(integer(snapshot, event, 0)?)?,
        construction_raw: integer(snapshot, event, 1)?.map(|raw| raw.to_string()),
        repair_raw: integer(snapshot, event, 2)?.map(|raw| raw.to_string()),
        reason,
        observed_since_creation,
        repair_ticks: hour_of(integer(snapshot, event, 5)?)?
            .ok_or_else(|| invalid("A construction output read is not valid"))?,
    })
}

/// What the replies share, as the pinned publication's last cut left it.
fn shared<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<Option<Shared>> {
    let Some(key) = directory.key(FAMILY, SHARED, None) else {
        return Ok(None);
    };
    let through = snapshot.publication().accepted.last;
    match latest(snapshot, key, through)? {
        Some(event) if event.kind == kind::SHARED => json(snapshot, &event).map(Some),
        Some(_) => Err(invalid("A construction shared event is not valid")),
        None => Ok(None),
    }
}

/// The projects `keys` name, in key order (the order the scan first read them), each with its
/// runs, counts, ending, first repair advance and output reads.
fn project_log<P: PageRead>(snapshot: &mut Snapshot<'_, P>, keys: &[u64]) -> io::Result<ScannedLog> {
    let accepted = snapshot.publication().accepted;
    let mut log = ScannedLog::default();
    for &key in keys {
        let mut project: Option<ConstructionProject> = None;
        let mut output = Vec::new();
        for event in events_of(snapshot, key, accepted)? {
            if event.kind == kind::START {
                project = Some(json(snapshot, &event)?);
                continue;
            }
            if event.kind == kind::OUTPUT {
                output.push(output_read(snapshot, &event)?);
                continue;
            }
            let held = project
                .as_mut()
                .ok_or_else(|| invalid("Construction start fact missing"))?;
            match event.kind {
                kind::PROGRESS => {
                    let mut values = [0; 5];
                    for (at, value) in values.iter_mut().enumerate() {
                        *value = integer(snapshot, &event, at as u64)?
                            .ok_or_else(|| invalid("A construction run is not valid"))?;
                    }
                    held.runs.push(run_of(values)?);
                }
                kind::COUNTS => {
                    let counts: ReadCounts = json(snapshot, &event)?;
                    held.built.extend(counts.built);
                    held.unaccounted.extend(counts.unaccounted);
                    held.possible.extend(counts.possible);
                    held.added.extend(counts.added);
                }
                kind::END => held.end = Some(json(snapshot, &event)?),
                kind::REPAIR => held.first_repair_tick = Some(event.native_hour),
                _ => return Err(invalid("A construction project event is not valid")),
            }
        }
        let Some(mut project) = project else {
            continue;
        };
        // A run's hours reach as far as its line was carried.
        for run in engine::runs(snapshot, key, accepted.first, accepted.last)? {
            if let Some(value) = project
                .runs
                .iter_mut()
                .find(|value| i128::from(value.first_hour) == run.value)
            {
                value.last_hour = value.last_hour.max(run.span.last);
            }
        }
        log.projects.push(project);
        log.outputs.push(output);
    }
    Ok(log)
}

/// Every project listed in `window`, in key order.
fn members<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    window: u64,
) -> io::Result<Vec<u64>> {
    let Some(key) = directory.key(FAMILY, WINDOW, Some(window)) else {
        return Ok(Vec::new());
    };
    let accepted = snapshot.publication().accepted;
    let mut members = BTreeSet::new();
    for event in events_of(snapshot, key, accepted)? {
        match field(snapshot, &event, 0)? {
            Cell::Unsigned(project) => members.insert(project),
            _ => return Err(invalid("A construction member is not valid")),
        };
    }
    Ok(members.into_iter().collect())
}

/// The output read events of `key` holds, as the log reads them.
fn read_of<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: &Event,
) -> io::Result<(i32, Option<SpendingGap>)> {
    let reason = match field(snapshot, event, 0)? {
        Cell::Unsigned(code) => {
            Some(gap_of(code).ok_or_else(|| invalid("A construction read reason is not valid"))?)
        }
        Cell::Null => None,
        _ => return Err(invalid("A construction read is not valid")),
    };
    Ok((event.native_hour, reason))
}

/// The window's reply: every project with a row date in it and the output its tooltips read,
/// with the output reads from the last before it to the next window.
pub(crate) fn window_reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    windows: Vec<dashboard_base::products::battles::BattleWindow>,
    window: usize,
) -> io::Result<ConstructionLogReply> {
    let selected = members(snapshot, directory, window as u64)?;
    let mut log = project_log(snapshot, &selected)?;
    let interval = &windows[window];
    let next = windows
        .get(window + 1)
        .map_or(i32::MAX, |window| window.first_hour.saturating_sub(1));
    if let Some(key) = directory.key(FAMILY, READS, None) {
        if let Some(before) = latest(snapshot, key, interval.first_hour.saturating_sub(1))? {
            log.output_reads.push(read_of(snapshot, &before)?);
        }
        if interval.first_hour <= next {
            for event in events_of(snapshot, key, Span::new(interval.first_hour, next)?)? {
                log.output_reads.push(read_of(snapshot, &event)?);
            }
        }
    }
    Ok(Cut::new(Arc::new(log), windows).reply(window))
}

/// Every leaf `tag` holds, by repair, kind, building type and states, its spent steps empty,
/// each with its spending key.
pub(crate) fn leaves<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
) -> io::Result<Vec<(SpendingLeaf, u64)>> {
    let Some(actor) = directory.actor(tag.as_bytes()) else {
        return Ok(Vec::new());
    };
    let accepted = snapshot.publication().accepted;
    let mut leaves = Vec::new();
    for &(_, key) in directory.dimensions(actor, LEAF) {
        let mut live = None;
        let mut metadata = LeafMetadata::default();
        for event in events_of(snapshot, key, accepted)? {
            match event.kind {
                kind::LIVE => live = Some(event),
                kind::FINAL => metadata.extend(json(snapshot, &event)?),
                _ => return Err(invalid("A construction leaf event is not valid")),
            }
        }
        // A leaf no cut has published yet.
        let Some(live) = live else {
            continue;
        };
        let mut record: LeafRecord = json(snapshot, &live)?;
        metadata.extend(std::mem::take(&mut record.metadata));
        leaves.push((metadata.materialize(&record), key));
    }
    leaves.sort_by_key(|(leaf, _)| {
        (
            leaf.repair,
            leaf.kind == super::ProjectKind::Railway,
            leaf.building,
            leaf.state,
            leaf.to_state,
        )
    });
    Ok(leaves)
}

/// Content's names and pictures of what `shared` names, and their warnings.
fn names(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    shared: &Shared,
    cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<ContentNames> {
    let log = ScannedLog {
        types: shared.types.clone(),
        states: shared.states.clone(),
        ..ScannedLog::default()
    };
    let context = LoadContext {
        content: content.cloned(),
    };
    NameSource::of(source, &context).names(branch, &log, cancelled, warnings)
}

/// The manifest `shared` gives, before any publication one with nothing recorded.
fn manifest(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    shared: Option<&Shared>,
    cancelled: &AtomicBool,
) -> io::Result<ConstructionLogManifestReply> {
    let windows = windows(source.replay_ref(), branch);
    let Some(shared) = shared else {
        return Ok(ConstructionLogManifestReply {
            cost_scale: COST_SCALE,
            windows,
            newest: None,
            recorded: false,
            types: Vec::new(),
            repair_icon: None,
            warnings: Vec::new(),
        });
    };
    let mut warnings = Vec::new();
    let names = names(source, content, branch, shared, cancelled, &mut warnings)?;
    count_warnings(&shared.counts, &mut warnings);
    Ok(ConstructionLogManifestReply {
        newest: None,
        cost_scale: COST_SCALE,
        windows,
        recorded: shared.counts.recorded,
        types: names.types,
        repair_icon: names.repair_icon,
        warnings,
    })
}

/// A spending reply of nothing prepared.
fn unprepared(branch: u64, tag: &str) -> ConstructionSpendingReply {
    ConstructionSpendingReply {
        branch_id: branch.to_string(),
        tag: tag.to_owned(),
        cost_scale: COST_SCALE,
        reads: Vec::new(),
        railway_reads: Vec::new(),
        types: Vec::new(),
        repair_icon: None,
        states: Vec::new(),
        leaves: Vec::new(),
        before_first_read: 0,
        unattributed: Vec::new(),
        warnings: Vec::new(),
    }
}

/// `tag`'s spending at `hours` as the pinned publication holds it.
#[allow(clippy::too_many_arguments)]
fn spending<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    tag: &str,
    hours: &BTreeSet<i32>,
    cancelled: &AtomicBool,
) -> io::Result<(ConstructionSpendingReply, Option<i32>)> {
    let Some(shared) = shared(snapshot, directory)? else {
        return Ok((unprepared(branch, tag), None));
    };
    let mut warnings = Vec::new();
    let names = names(source, content, branch, &shared, cancelled, &mut warnings)?;
    count_warnings(&shared.counts, &mut warnings);
    let leaves = leaves(snapshot, directory, tag)?;
    let states: BTreeSet<u32> = leaves
        .iter()
        .flat_map(|(leaf, _)| [leaf.state, leaf.to_state])
        .flatten()
        .collect();
    if leaves.is_empty() && shared.counts.recorded {
        warnings.push(format!("{tag}: no recorded construction in this branch"));
    }
    let mut spent = Vec::with_capacity(leaves.len());
    for (mut leaf, key) in leaves {
        for &hour in hours {
            if let Observation::Value { run, .. } = snapshot.value_at(key, hour)? {
                leaf.spent.push(SpentStep {
                    known_at: hour,
                    total_raw: run.value.to_string(),
                });
            }
        }
        spent.push(leaf);
    }
    let reply = ConstructionSpendingReply {
        branch_id: branch.to_string(),
        tag: tag.to_owned(),
        cost_scale: COST_SCALE,
        reads: shared.reads,
        railway_reads: shared.railway_reads,
        states: names.states(&states),
        types: names.types,
        repair_icon: names.repair_icon,
        leaves: spent,
        before_first_read: shared.before_first.get(tag).copied().unwrap_or(0),
        unattributed: shared.unattributed,
        warnings,
    };
    Ok((reply, shared.last))
}

/// A construction product's reply on the branch its params name; None is never answered.
pub fn query(
    source: &Source,
    product: &str,
    params: serde_json::Value,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<serde_json::Value>> {
    let branch: u64 = params["branchId"]
        .as_str()
        .ok_or_else(|| invalid("Missing construction branch"))?
        .parse()
        .map_err(|_| invalid("Invalid construction branch"))?;
    // Construction prepared on its own answers from its own coverage.
    let coverage = dashboard_base::prepared::prefix::read_or_empty(
        source, content, product, branch, None, cancelled,
    )?
    .coverage;
    // Pinned through that coverage: no read passes it.
    let opened = published::open_through(
        source,
        content,
        NAME,
        SCHEMA,
        branch,
        (!coverage.complete).then_some(coverage.last_hour),
    )?;
    match product {
        "construction-log-manifest" => {
            let shared = match &opened {
                Some(opened) => opened.read(cancelled, |snapshot, directory| {
                    shared(snapshot, directory)
                })?,
                None => None,
            };
            let mut manifest = manifest(source, content, branch, shared.as_ref(), cancelled)?;
            present_stored(
                source,
                content,
                &mut manifest.types,
                Some(&mut manifest.repair_icon),
                cancelled,
            )?;
            let prefix = Prefix::new(source, branch, coverage, None);
            let mut data = serde_json::to_value(manifest)?;
            prefix.mark_revisable(source, branch, &mut data)?;
            Ok(Some(data))
        }
        "construction-log" => {
            let params: ConstructionLogParams = serde_json::from_value(params)?;
            ConstructionLog::validate(&params).map_err(dashboard_core::invalid)?;
            let windows = windows(source.replay_ref(), branch);
            if params.window >= windows.len() {
                return Err(invalid("Construction log window not in recording"));
            }
            let prefix = Prefix::new(source, branch, coverage.clone(), None);
            // Before coverage completes, a window the prepared prefix has not reached waits.
            if !coverage.complete && windows[params.window].last_hour > coverage.last_hour {
                let mut data = serde_json::to_value(ConstructionLogReply {
                    window: params.window,
                    interval: windows[params.window].clone(),
                    types: None,
                    exact_from: None,
                    output_reads: Vec::new(),
                    output_unavailable: Vec::new(),
                    projects: Vec::new(),
                    outputs: Vec::new(),
                    warnings: Vec::new(),
                })?;
                prefix.mark(&mut data)?;
                return Ok(Some(data));
            }
            let mut reply = match &opened {
                Some(opened) => opened.read(cancelled, |snapshot, directory| {
                    window_reply(snapshot, directory, windows, params.window)
                })?,
                None => Cut::new(Arc::new(ScannedLog::default()), windows).reply(params.window),
            };
            if let Some(types) = &mut reply.types {
                present_stored(source, content, types, None, cancelled)?;
            }
            dashboard_base::products::check_reply_budget(&reply)?;
            let mut data = serde_json::to_value(reply)?;
            prefix.mark_revisable(source, branch, &mut data)?;
            Ok(Some(data))
        }
        "construction-spending" => {
            let tag = params["tag"]
                .as_str()
                .ok_or_else(|| invalid("Missing construction country"))?;
            dashboard_core::protocol::validate_tag(tag).map_err(dashboard_core::invalid)?;
            let hours: Vec<i32> = serde_json::from_value(params["hours"].clone())?;
            // Spending is cumulative factory output per leaf: before coverage completes, an
            // hour after the prepared prefix reads the total through the prefix's last hour,
            // and every leaf's output after it is still being prepared.
            let prefix = Prefix::new(
                source,
                branch,
                coverage.clone(),
                hours.iter().max().copied(),
            );
            let hours: BTreeSet<i32> = hours
                .into_iter()
                .map(|hour| prefix.through(Some(hour)).unwrap_or(hour))
                .collect();
            let (mut reply, last) = match &opened {
                Some(opened) => opened.read(cancelled, |snapshot, directory| {
                    spending(
                        snapshot, directory, source, content, branch, tag, &hours, cancelled,
                    )
                })?,
                None => (unprepared(branch, tag), None),
            };
            if let Some(missing) = last.and_then(|last| last.checked_add(1))
                && missing <= coverage.last_hour
            {
                reply.unattributed.push(SpendingUnknown {
                    known_at: missing,
                    reason: SpendingGap::NotRecorded,
                    project: String::new(),
                    until: None,
                });
            }
            if let Some(pending) = prefix.pending.first() {
                for leaf in &mut reply.leaves {
                    leaf.unknown.push(SpendingUnknown {
                        known_at: pending.first_hour,
                        reason: SpendingGap::Preparing,
                        project: String::new(),
                        until: None,
                    });
                }
            }
            present_stored(
                source,
                content,
                &mut reply.types,
                Some(&mut reply.repair_icon),
                cancelled,
            )?;
            let mut data = serde_json::to_value(reply)?;
            // Inferred work intervals and previously unresolved building names can be
            // repaired by later reads; the prefix's presentation is provisional.
            prefix.mark_revisable(source, branch, &mut data)?;
            Ok(Some(data))
        }
        _ => Err(invalid("Unknown construction query")),
    }
}
