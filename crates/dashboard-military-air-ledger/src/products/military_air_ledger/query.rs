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

/// `family`'s census as the publication holds it, with its sums natively dated from `from`.
fn census<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    family: usize,
    from: Option<i32>,
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
    let native = Span::new(from.unwrap_or(i32::MIN).min(accepted.last), accepted.last)?;
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
            if from.is_none_or(|from| event.native_hour >= from) {
                scan.add(leaf, event.native_hour, read, sum)?;
            }
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    Ok(scan)
}

/// A `military-air-ledger` request answered from the publication; `availability` asks only
/// whether the family read any hour.
pub fn reply<P: PageRead>(
    source: &Source,
    context: &LoadContext,
    mut params: Value,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
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
        let observed = spans(
            snapshot,
            directory.key(FAMILY, OBSERVED + family as u64, None),
        )?;
        return Ok(serde_json::json!({ "available": !observed.is_empty() }));
    }
    let branch: u64 = params.branch_id.parse().expect("validated branch");
    let declaration = source
        .replay_ref()
        .branches
        .iter()
        .find(|row| row.id == branch)
        .ok_or_else(|| invalid("Unknown branch"))?;
    let (first, last) = dashboard_text::metadata::branch_bounds(source.replay_ref(), declaration);
    let held = census(snapshot, directory, family, params.from_hour, cancelled)?;
    let mut scan = Scan {
        first,
        last,
        ..Scan::default()
    };
    match params.family {
        MilitaryAirFamily::AirLosses => scan.air = held,
        MilitaryAirFamily::Bombing => scan.bombing = held,
    }
    Ok(serde_json::to_value(
        scan.reply(source, &params, context, cancelled)?,
    )?)
}

/// A cold ledger: no hour read, every recorded hour pending.
pub fn empty(
    source: &Source,
    context: &LoadContext,
    mut params: Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    if let Some(value) = params
        .as_object_mut()
        .and_then(|params| params.remove("availability"))
    {
        let asked = value
            .as_bool()
            .ok_or_else(|| invalid("Invalid ledger availability request"))?;
        if asked {
            return Ok(serde_json::json!({ "available": false }));
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
    Ok(serde_json::to_value(
        Scan {
            first,
            last,
            ..Scan::default()
        }
        .reply(source, &params, context, cancelled)?,
    )?)
}
