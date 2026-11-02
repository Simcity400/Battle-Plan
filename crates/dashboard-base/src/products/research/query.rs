//! Research's replies, read from the family's series publication ([`super::emit`]) pinned
//! through its prepared prefix: a country's slot states and grants at hours (the active
//! research view) and its whole assignment history (the history view's events page). Nothing
//! reads the recording at query time.

use super::{
    CountryParams, LoadContext, Product, Research, ResearchReply, State,
    emit::{AVAILABLE, Closed, GRANTS, HISTORY, Open, PROGRESS, SCHEMA, STATE, TAIL, kind},
    facts::Content,
    grants::{self, GrantInventory},
};
use crate::{
    ResearchCoverage, ResearchHistory, ResearchProgress, ResearchSlotHistory, ResearchSlotState,
    Source,
    prepared::{engine, prefix, published},
    prepared_measure::query::MeasureRead,
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span,
    family::Directory,
    query::EventView,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io, sync::atomic::AtomicBool};

/// Event bytes one page reads at most.
const PAGE: usize = 1 << 20;

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchStateSlot {
    pub index: u32,
    pub available: bool,
    pub state: Option<ResearchSlotState>,
    pub progress: Option<ResearchProgress>,
    pub read_hour: Option<i32>,
}

#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchState {
    pub tag: String,
    pub hour: i32,
    pub slots: Vec<ResearchStateSlot>,
    pub data: ResearchReply,
    pub preparing: Vec<String>,
    pub grants: GrantInventory,
}

#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchStateReply {
    pub branch_id: String,
    pub states: Vec<ResearchState>,
}

/// The JSON text of a family event of `kind`.
fn text<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: EventView,
    kind: u64,
) -> io::Result<Vec<u8>> {
    let malformed = || crate::invalid("Prepared research event is not valid");
    let EventView::Family(event) = event else {
        return Err(malformed());
    };
    if event.kind != kind {
        return Err(malformed());
    }
    let Cell::Text(blob) = snapshot.field(event.fields, 0)? else {
        return Err(malformed());
    };
    let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| malformed())?];
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok(bytes)
}

/// `key`'s latest event by `hour`, with its hour.
fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
    kind: u64,
) -> io::Result<Option<(i32, Vec<u8>)>> {
    let selection = Selection {
        key,
        membership: None,
    };
    match snapshot.latest_event(selection, hour, None)? {
        Some(event) => Ok(Some((event.native_hour(), text(snapshot, event, kind)?))),
        None => Ok(None),
    }
}

/// Every event of `key`, in native order.
fn events<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    kind: u64,
    cancelled: &AtomicBool,
) -> io::Result<Vec<Vec<u8>>> {
    let selection = [Selection {
        key,
        membership: None,
    }];
    let mut found = Vec::new();
    let mut cursor = None;
    loop {
        crate::metadata::check_cancel(cancelled)?;
        let page = snapshot.event_page(
            EventQuery {
                selection: &selection,
                native: Span {
                    first: i32::MIN,
                    last: i32::MAX,
                },
                as_of: None,
                order: EventOrder::Native,
            },
            cursor,
            PAGE,
            |event| {
                found.push(event);
                Ok(())
            },
        )?;
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    found
        .into_iter()
        .map(|event| text(snapshot, event, kind))
        .collect()
}

fn slot(history: &mut ResearchHistory, index: u32) -> &mut ResearchSlotHistory {
    let position = history
        .slots
        .iter()
        .position(|slot| slot.index == index)
        .unwrap_or_else(|| {
            history.slots.push(ResearchSlotHistory {
                index,
                availability: Vec::new(),
                entries: Vec::new(),
                states: Vec::new(),
                progress: Vec::new(),
            });
            history.slots.len() - 1
        });
    &mut history.slots[position]
}

fn missing(what: &'static str) -> io::Error {
    crate::invalid(what)
}

/// `tag`'s whole history and its slot reads not joined: its closed items, each slot's states
/// to the end of their runs, and its open items as of the publication's last cut.
pub(super) fn history<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<(ResearchHistory, usize)> {
    let Some(actor) = directory.actor(tag.as_bytes()) else {
        return Err(io::Error::other(
            "No recorded research history for this country",
        ));
    };
    let key = |measure| {
        directory
            .key(actor, measure, None)
            .ok_or_else(|| missing("Prepared research keys are missing"))
    };
    let (closed, tail) = (key(HISTORY)?, key(TAIL)?);
    let accepted = snapshot.publication().accepted.last;
    // An actor its grants alone named was never observed: it has no history.
    let Some((_, open)) = latest(snapshot, tail, accepted, kind::OPEN)? else {
        return Err(io::Error::other(
            "No recorded research history for this country",
        ));
    };
    let open: Open = serde_json::from_slice(&open)?;
    let mut history = ResearchHistory::default();
    for item in events(snapshot, closed, kind::CLOSED, cancelled)? {
        match serde_json::from_slice::<Closed>(&item)? {
            Closed::Coverage { span } => history.coverage.push(span),
            Closed::Available { slot: index, span } => {
                slot(&mut history, index).availability.push(span)
            }
            Closed::Progress {
                slot: index,
                progress,
            } => slot(&mut history, index).progress.push(progress),
            Closed::Entry { slot: index, entry } => slot(&mut history, index).entries.push(entry),
        }
    }
    for &(index, key) in directory.dimensions(actor, STATE) {
        let index = u32::try_from(index).map_err(|_| missing("Prepared research slot"))?;
        for item in events(snapshot, key, kind::STATE, cancelled)? {
            let mut state: ResearchSlotState = serde_json::from_slice(&item)?;
            state.last_hour = engine::run_at(snapshot, key, state.first_hour)?
                .ok_or_else(|| missing("Prepared research state is missing"))?
                .0
                .span
                .last;
            slot(&mut history, index).states.push(state);
        }
    }
    history.coverage.extend(open.history.coverage);
    for latest in open.history.slots {
        let slot = slot(&mut history, latest.index);
        slot.entries.extend(latest.entries);
        slot.availability.extend(latest.availability);
        slot.progress.extend(latest.progress);
    }
    history.coverage.sort_by_key(|span| span.first_hour);
    history.slots.sort_by_key(|slot| slot.index);
    for slot in &mut history.slots {
        slot.entries.sort_by_key(|entry| entry.first_observed_hour);
        slot.availability.sort_by_key(|span| span.first_hour);
        slot.states.sort_by_key(|state| state.first_hour);
        slot.progress.sort_by_key(|progress| progress.first_hour);
    }
    Ok((history, open.unjoined))
}

/// A country's complete assignment history, in its established sorting and time breakdown:
/// the `events-page` of the research family kind, answering the prepared prefix through
/// `requested` and naming the later hours pending.
pub fn history_page(
    source: &Source,
    context: &LoadContext,
    params: &Value,
    branch: u64,
    requested: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let content = context.content.as_ref();
    let prefix = prefix::read_or_empty(source, content, "research", branch, requested, cancelled)?;
    let params: CountryParams = serde_json::from_value(
        serde_json::json!({ "branchId": params.get("branchId"), "tag": params.get("tag") }),
    )?;
    Research::validate(&params).map_err(crate::invalid)?;
    let limit = (!prefix.coverage.complete).then_some(prefix.coverage.last_hour);
    let (history, unjoined) =
        match published::open_through(source, content, "research", SCHEMA, params.branch(), limit)?
        {
            Some(opened) => opened.read(cancelled, |snapshot, directory| {
                history(snapshot, directory, &params.tag, cancelled)
            })?,
            None => (ResearchHistory::default(), 0),
        };
    let state = source.state::<State>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let mut data = serde_json::to_value(super::present(
        source, context, &mut state, history, unjoined, cancelled,
    )?)?;
    prefix.mark(&mut data)?;
    Ok(data)
}

/// `tag`'s slots at `hour`: each slot it ever held, whether it is available, its state and the
/// supplying read's progress.
pub(super) fn slots_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    hour: i32,
) -> io::Result<Vec<ResearchStateSlot>> {
    let Some(actor) = directory.actor(tag.as_bytes()) else {
        return Ok(Vec::new());
    };
    let mut slots = Vec::new();
    for &(index, available) in directory.dimensions(actor, AVAILABLE) {
        let key = |measure| {
            directory
                .key(actor, measure, Some(index))
                .ok_or_else(|| missing("Prepared research slot keys are missing"))
        };
        let (state_key, progress_key) = (key(STATE)?, key(PROGRESS)?);
        let mut state = None;
        if let Some((run, _)) = engine::run_at(snapshot, state_key, hour)?
            && let Some((_, value)) = latest(snapshot, state_key, hour, kind::STATE)?
        {
            let mut value: ResearchSlotState = serde_json::from_slice(&value)?;
            value.last_hour = run.span.last.min(hour);
            state = Some(value);
        }
        let mut progress = None;
        let mut read_hour = None;
        if let Some((read, value)) = latest(snapshot, progress_key, hour, kind::READ)? {
            read_hour = Some(read);
            if let Some(mut value) = serde_json::from_slice::<Option<ResearchProgress>>(&value)? {
                value.last_hour = value.last_hour.min(hour);
                progress = Some(value);
            }
        }
        slots.push(ResearchStateSlot {
            index: u32::try_from(index).map_err(|_| missing("Prepared research slot"))?,
            available: engine::run_at(snapshot, available, hour)?.is_some(),
            state,
            progress,
            read_hour,
        });
    }
    Ok(slots)
}

/// `tag`'s grant inventory at `hour`, every name unlocalised: what its rows held there while a
/// supplying read admits them.
pub(super) fn grants_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    hour: i32,
) -> io::Result<Result<GrantInventory, String>> {
    let Some(key) = directory
        .actor(tag.as_bytes())
        .and_then(|actor| directory.key(actor, GRANTS, None))
    else {
        return Ok(Err(grants::UNADMITTED.into()));
    };
    if engine::run_at(snapshot, key, hour)?.is_none() {
        return Ok(Err(grants::UNADMITTED.into()));
    }
    let (_, content) = latest(snapshot, key, hour, kind::GRANT)?
        .ok_or_else(|| missing("Prepared research grants are missing"))?;
    Ok(match serde_json::from_slice::<Content>(&content)? {
        Content::Inventory(inventory) => Ok(inventory),
        Content::Refused(reason) => Err(reason),
        Content::Table(reason) => Err(format!("hour {hour} {reason}")),
    })
}

/// Slot state at the requested hours, as the active research view reads it: each read's slots,
/// the history they imply, and the grants its supplying read admits. Pending while any
/// requested hour is past the prepared prefix.
pub fn state(
    source: &Source,
    context: &LoadContext,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let content = context.content.as_ref();
    let branch_id = params
        .get("branchId")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::invalid("Invalid research branch"))?;
    let branch: u64 = branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid research branch"))?;
    let reads: Vec<MeasureRead> = serde_json::from_value(
        params
            .get("reads")
            .cloned()
            .ok_or_else(|| crate::invalid("Research reads are missing"))?,
    )?;
    for read in &reads {
        super::super::validate_tag(&read.tag).map_err(crate::invalid)?;
    }
    let prefix = prefix::read_or_empty(
        source,
        content,
        "research",
        branch,
        reads.iter().map(|read| read.hour).max(),
        cancelled,
    )?;
    let covered = prefix.coverage.complete;
    if !covered
        && reads
            .iter()
            .any(|read| read.hour > prefix.coverage.last_hour)
    {
        return Ok(Value::Null);
    }
    let limit = (!covered).then_some(prefix.coverage.last_hour);
    let Some(opened) = published::open_through(source, content, "research", SCHEMA, branch, limit)?
    else {
        return Ok(Value::Null);
    };
    let read = opened.read(cancelled, |snapshot, directory| {
        let mut found = Vec::with_capacity(reads.len());
        for read in &reads {
            crate::metadata::check_cancel(cancelled)?;
            let slots = slots_at(snapshot, directory, &read.tag, read.hour)?;
            let grants = grants_at(snapshot, directory, &read.tag, read.hour)?;
            found.push((slots, grants));
        }
        Ok(found)
    })?;
    let mut states = Vec::with_capacity(reads.len());
    for (read, (slots, grants)) in reads.into_iter().zip(read) {
        crate::metadata::check_cancel(cancelled)?;
        let mut history = ResearchHistory::default();
        for slot in slots.iter().filter(|slot| slot.available) {
            let mut state = slot.state.clone();
            let mut progress = slot.progress.clone().filter(|progress| {
                state.as_ref().is_some_and(|state| {
                    state.token.is_some()
                        && state.entry_first_observed_hour == progress.entry_first_observed_hour
                })
            });
            let supplying = slot.read_hour.unwrap_or(read.hour);
            let from = state
                .as_ref()
                .map_or(supplying, |state| state.first_hour.min(supplying));
            if let Some(state) = &mut state {
                state.last_hour = read.hour;
                state.entry_first_observed_hour = None;
            }
            if let Some(progress) = &mut progress {
                progress.first_hour = supplying;
                progress.last_hour = supplying;
                progress.entry_first_observed_hour = None;
            }
            history.slots.push(ResearchSlotHistory {
                index: slot.index,
                availability: vec![ResearchCoverage {
                    first_hour: from,
                    last_hour: read.hour,
                }],
                entries: Vec::new(),
                states: state.into_iter().collect(),
                progress: progress.into_iter().collect(),
            });
        }
        if history.coverage.is_empty()
            && let Some(first) = history
                .slots
                .iter()
                .flat_map(|slot| slot.availability.iter())
                .map(|span| span.first_hour)
                .min()
        {
            history.coverage.push(ResearchCoverage {
                first_hour: first,
                last_hour: read.hour,
            });
        }
        let state = source.state::<State>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let data = super::present(source, context, &mut state, history, 0, cancelled)?;
        let grants = match grants {
            Ok(inventory) => grants::localised(inventory, |token| {
                state.artwork.as_mut().and_then(|art| art.localise(token))
            }),
            Err(reason) => GrantInventory::unavailable(reason),
        };
        states.push(ResearchState {
            tag: read.tag,
            hour: read.hour,
            slots,
            data,
            preparing: Vec::new(),
            grants,
        });
    }
    let mut data = serde_json::to_value(ResearchStateReply {
        branch_id: branch_id.into(),
        states,
    })?;
    prefix.mark(&mut data)?;
    Ok(data)
}
