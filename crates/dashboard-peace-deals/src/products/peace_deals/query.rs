//! Peace deals from the family's series publication ([`super::emit`]): the continuation its
//! pinned segment ends with, joined with every deal and finalizer outcome published before it.

use super::{
    PeaceDeals, PeaceDealsParams, PeaceDealsReply, PeaceTransferGraph, PeaceUnobserved,
    check_cancel,
    emit::{CONTINUATION, EVENTS, FAMILY, SCHEMA, kind},
    prepared::{self, Event},
    present,
};
use dashboard_base::{
    Source,
    prepared::published,
    products::{ContentOptions, LoadContext, Product},
};
use dashboard_series::{
    Blob, Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, family::Directory,
    query::EventView,
};
use dashboard_text::metadata::branch_bounds;
use std::{io, sync::atomic::AtomicBool};

fn malformed() -> io::Error {
    dashboard_core::invalid("A peace-deals publication event is malformed")
}

/// The bytes of `fields`' one text cell.
fn text<P: PageRead>(snapshot: &mut Snapshot<'_, P>, fields: Blob) -> io::Result<Vec<u8>> {
    let Cell::Text(blob) = snapshot.field(fields, 0)? else {
        return Err(malformed());
    };
    let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| malformed())?];
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok(bytes)
}

/// The reply through `last` from the publication `snapshot` pins: the branch's last hour, or a
/// prepared prefix's (`prefix`), which ends at the publication's last hour. None before its
/// first cut.
pub fn reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    last: i32,
    prefix: bool,
    cancelled: &AtomicBool,
) -> io::Result<Option<PeaceDealsReply>> {
    let accepted = snapshot.publication().accepted;
    let Some(key) = directory.key(FAMILY, CONTINUATION, None) else {
        return Ok(None);
    };
    let selection = Selection {
        key,
        membership: None,
    };
    let Some(EventView::Family(held)) = snapshot.latest_event(selection, accepted.last, None)?
    else {
        return Ok(None);
    };
    if held.kind != kind::CONTINUATION {
        return Err(malformed());
    }
    let continuation = text(snapshot, held.fields)?;
    let mut rows = Vec::new();
    if let Some(key) = directory.key(FAMILY, EVENTS, None) {
        let selection = [Selection {
            key,
            membership: None,
        }];
        let mut cursor = None;
        loop {
            check_cancel(cancelled)?;
            let mut page = Vec::new();
            let result = snapshot.event_page(
                EventQuery {
                    selection: &selection,
                    native: accepted,
                    as_of: None,
                    order: EventOrder::Native,
                },
                cursor,
                1 << 20,
                |event| {
                    page.push(event);
                    Ok(())
                },
            )?;
            for event in page {
                let EventView::Family(event) = event else {
                    return Err(malformed());
                };
                if !matches!(event.kind, kind::DEAL | kind::FINALIZER) {
                    return Err(malformed());
                }
                let bytes = text(snapshot, event.fields)?;
                rows.push((event.native_hour, event.sequence, bytes));
            }
            cursor = result.next;
            if cursor.is_none() {
                break;
            }
        }
    }
    // In the order the observer left them.
    rows.sort_unstable_by_key(|&(hour, sequence, _)| (hour, sequence));
    let events = rows
        .into_iter()
        .map(|(_, _, bytes)| serde_json::from_slice::<Event>(&bytes))
        .collect::<Result<Vec<_>, _>>()?;
    let last = if prefix { last.min(accepted.last) } else { last };
    prepared::reply(&continuation, events, last, prefix).map(Some)
}

/// The peace-deals reply to `params`, read through `through`, the last hour of the prepared
/// prefix, while preparation has not reached the branch's end (None once it has): deals its
/// later hours may still settle are marked `preparing`, and those hours are not called
/// unobserved.
pub fn query(
    source: &Source,
    content: Option<&ContentOptions>,
    params: serde_json::Value,
    through: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let params: PeaceDealsParams =
        serde_json::from_value(serde_json::json!({ "branchId": params.get("branchId") }))?;
    PeaceDeals::validate(&params).map_err(dashboard_core::invalid)?;
    let branch = params.branch_id.parse().expect("validated peace branch");
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Peace-deals branch is not in this recording"))?;
    let (_, last) = branch_bounds(source.replay_ref(), start);
    let last = through.map_or(last, |through| through.min(last));
    let opened = published::open_through(source, content, "peace", SCHEMA, branch, through)?;
    let reply = match opened {
        Some(opened) => opened.read(cancelled, |snapshot, directory| {
            reply(snapshot, directory, last, through.is_some(), cancelled)
        })?,
        None => None,
    };
    let Some(reply) = reply else {
        return serde_json::to_value(PeaceDealsReply {
            branch_id: params.branch_id,
            derived: true,
            deals: Vec::new(),
            transfers: PeaceTransferGraph::default(),
            unobserved: PeaceUnobserved::default(),
            warnings: Vec::new(),
        })
        .map_err(Into::into);
    };
    let context = LoadContext {
        content: content.cloned(),
    };
    serde_json::to_value(present(source, &context, cancelled, reply)?).map_err(Into::into)
}
