//! The military air ledger's replies, read from the family's series publication
//! ([`super::emit`]): the requested family's census rebuilt from its runs, counts, leaves and
//! sums through the publication's accepted hours, then answered as the row ledger answers it.

use super::{
    FamilyScan, LeafKey, MilitaryAirFamily, MilitaryAirLedger, MilitaryAirParams, Scan,
    emit::{FAMILY, INCOMPLETE, OBSERVED, PRESENT, SUMS, WARNINGS, country_of, leaves},
};
use dashboard_base::{
    prepared::engine,
    products::{Product, country_groups::HourSpan},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span, codec::Binary,
    family::Directory, query::EventView,
};
use dashboard_base::products::built::Built;
use serde_json::Value;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

/// `key`'s runs through the accepted hours, as spans.
fn spans<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
) -> io::Result<Vec<HourSpan>> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    let accepted = snapshot.publication().accepted;
    Ok(engine::runs(snapshot, key, accepted.first, accepted.last)?
        .into_iter()
        .map(|run| HourSpan {
            first_hour: run.span.first,
            last_hour: run.span.last,
        })
        .collect())
}

/// `family`'s whole census as the publication holds it: a reply keeps the sums it asks for
/// ([`super::family_reply`]).
fn census<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    family: usize,
    cancelled: &AtomicBool,
) -> io::Result<FamilyScan> {
    let mut scan = FamilyScan {
        observed: spans(
            snapshot,
            directory.key(FAMILY, OBSERVED + family as u64, None),
        )?,
        incomplete: spans(
            snapshot,
            directory.key(FAMILY, INCOMPLETE + family as u64, None),
        )?,
        ..FamilyScan::default()
    };
    scan.available = !scan.observed.is_empty();
    for pair in directory
        .scalar(WARNINGS + family as u64)
        .map_or(&[][..], |scalar| scalar.list())
    {
        if let [reason, count] = pair.list()
            && let (Some(reason), Some(count)) = (reason.text(), count.integer())
        {
            scan.warnings.insert(
                String::from_utf8_lossy(reason).into_owned(),
                usize::try_from(count)
                    .map_err(|_| invalid("Prepared military air count is not valid"))?,
            );
        }
    }
    for (actor, described) in directory.actors.iter().enumerate() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let Some(country) = country_of(&described.name) else {
            continue;
        };
        let key = directory.key(actor as u64, PRESENT + family as u64, None);
        if key.is_some() {
            let present = spans(snapshot, key)?;
            if !present.is_empty() {
                scan.countries.insert(country, present);
            }
        }
    }
    let mut named = Vec::new();
    while let Some(scalar) = directory.scalar(leaves(family) + named.len() as u64) {
        let leaf: LeafKey = serde_json::from_slice(
            scalar
                .text()
                .ok_or_else(|| invalid("A prepared military air leaf is not valid"))?,
        )?;
        scan.leaves.insert(leaf.clone(), BTreeMap::new());
        named.push(leaf);
    }
    let Some(key) = directory.key(FAMILY, SUMS + family as u64, None) else {
        return Ok(scan);
    };
    let accepted = snapshot.publication().accepted;
    let native = Span::new(i32::MIN, accepted.last)?;
    let selection = [Selection {
        key,
        membership: None,
    }];
    let malformed = || invalid("A prepared military air sum is not valid");
    let mut cursor = None;
    loop {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut events = Vec::new();
        let page = snapshot.event_page(
            EventQuery {
                selection: &selection,
                native,
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
            if event.fields.len != 2 * Cell::SIZE as u64 {
                return Err(malformed());
            }
            let (Cell::Unsigned(leaf), Cell::Integer(sum)) = (
                snapshot.field(event.fields, 0)?,
                snapshot.field(event.fields, 1)?,
            ) else {
                return Err(malformed());
            };
            let leaf = named
                .get(usize::try_from(leaf).map_err(|_| malformed())?)
                .cloned()
                .ok_or_else(malformed)?;
            let read = event.read_hour.ok_or_else(malformed)?;
            scan.add(leaf, event.native_hour, read, sum)?;
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    Ok(scan)
}

/// A `military-air-ledger` request answered from the publication `opened`, whose census of each
/// family is read once; `availability` asks only whether the family read any hour.
pub fn reply(
    source: &Source,
    context: &LoadContext,
    params: Value,
    opened: &dashboard_base::prepared::published::Opened,
    cancelled: &AtomicBool,
) -> io::Result<Built> {
    respond(
        source,
        context,
        params,
        cancelled,
        |family| {
            opened.read(cancelled, |snapshot, directory| {
                spans(snapshot, directory.key(FAMILY, OBSERVED + family as u64, None))
            })
        },
        |family| {
            opened.derived(
                ["air ledger census: air losses", "air ledger census: bombing"][family],
                cancelled,
                |snapshot, directory| census(snapshot, directory, family, cancelled),
            )
        },
    )
}

/// [`reply`] read from `snapshot`, its census read afresh.
#[cfg(test)]
pub(super) fn reply_of<P: PageRead>(
    source: &Source,
    context: &LoadContext,
    params: Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let snapshot = std::cell::RefCell::new(snapshot);
    respond(
        source,
        context,
        params,
        cancelled,
        |family| {
            spans(
                &mut snapshot.borrow_mut(),
                directory.key(FAMILY, OBSERVED + family as u64, None),
            )
        },
        |family| {
            census(&mut snapshot.borrow_mut(), directory, family, cancelled)
                .map(std::sync::Arc::new)
        },
    )?
    .into_value()
}

/// `params`' reply: whether its family was `observed` at any hour, or the reply from its `census`.
fn respond(
    source: &Source,
    context: &LoadContext,
    mut params: Value,
    cancelled: &AtomicBool,
    observed: impl FnOnce(usize) -> io::Result<Vec<HourSpan>>,
    census: impl FnOnce(usize) -> io::Result<std::sync::Arc<FamilyScan>>,
) -> io::Result<Built> {
    let availability = params
        .as_object_mut()
        .and_then(|params| params.remove("availability"))
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| invalid("Invalid ledger availability request"))
        })
        .transpose()?
        .unwrap_or(false);
    let params: MilitaryAirParams = serde_json::from_value(params)?;
    MilitaryAirLedger::validate(&params).map_err(dashboard_core::invalid)?;
    let family = match params.family {
        MilitaryAirFamily::AirLosses => 0,
        MilitaryAirFamily::Bombing => 1,
    };
    if availability {
        let observed = observed(family)?;
        return Built::of(
            &serde_json::json!({ "available": !observed.is_empty() }),
            cancelled,
        );
    }
    let branch: u64 = params.branch_id.parse().expect("validated branch");
    let declaration = source
        .replay_ref()
        .branches
        .iter()
        .find(|row| row.id == branch)
        .ok_or_else(|| invalid("Unknown branch"))?;
    let (first, last) = dashboard_text::metadata::branch_bounds(source.replay_ref(), declaration);
    let held = census(family)?;
    super::built(
        &super::family_reply(&held, (first, last), source, &params, context, cancelled)?,
        cancelled,
    )
}

/// A cold ledger: no hour read, every recorded hour pending.
pub fn empty(
    source: &Source,
    context: &LoadContext,
    mut params: Value,
    cancelled: &AtomicBool,
) -> io::Result<Built> {
    if let Some(value) = params
        .as_object_mut()
        .and_then(|params| params.remove("availability"))
    {
        let asked = value
            .as_bool()
            .ok_or_else(|| invalid("Invalid ledger availability request"))?;
        if asked {
            return Built::of(&serde_json::json!({ "available": false }), cancelled);
        }
    }
    let params: MilitaryAirParams = serde_json::from_value(params)?;
    MilitaryAirLedger::validate(&params).map_err(dashboard_core::invalid)?;
    let branch: u64 = params.branch_id.parse().expect("validated branch");
    let declaration = source
        .replay_ref()
        .branches
        .iter()
        .find(|row| row.id == branch)
        .ok_or_else(|| invalid("Unknown branch"))?;
    let (first, last) = dashboard_text::metadata::branch_bounds(source.replay_ref(), declaration);
    super::built(
        &Scan {
            first,
            last,
            ..Scan::default()
        }
        .reply(source, &params, context, cancelled)?,
        cancelled,
    )
}
