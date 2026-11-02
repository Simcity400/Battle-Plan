//! The lend-lease balance reply, read from the family's series publication through the hours
//! its gate prepared.

use super::{
    LendLeaseBalanceParams, LendLeaseBalanceReply, LendLeaseHistory, MEASURES,
    emit::{FAMILY, KNOWN, RECORD, REFUSALS, TAGS, TOTAL},
};
use crate::products::keyed::{EconomyRun, HolderTag, branch_of};
use dashboard_base::prepared::engine;
use dashboard_series::{PageRead, Snapshot, family::Directory};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// The reply before any history is read, and the holder it asks for.
fn empty(params: &LendLeaseBalanceParams, through: Option<i32>) -> io::Result<(LendLeaseBalanceReply, Option<u32>)> {
    branch_of(&params.branch_id)?;
    let holder = params
        .holder
        .as_deref()
        .map(str::parse::<u32>)
        .transpose()
        .map_err(|_| dashboard_base::invalid("Invalid lend-lease holder"))?;
    let reply = LendLeaseBalanceReply {
        branch_id: params.branch_id.clone(),
        last_hour: through,
        scale: 100_000,
        known: Vec::new(),
        histories: Vec::new(),
        holder: params.holder.clone(),
        warnings: Vec::new(),
    };
    Ok((reply, holder))
}

/// The reply before the family publishes the branch.
pub fn unpublished(params: &LendLeaseBalanceParams, through: Option<i32>) -> io::Result<LendLeaseBalanceReply> {
    empty(params, through).map(|(reply, _)| reply)
}

/// `key`'s runs through `through`, as exact decimal text.
fn text<P: PageRead>(snapshot: &mut Snapshot<'_, P>, key: u64, through: i32) -> io::Result<Vec<EconomyRun>> {
    let accepted = snapshot.publication().accepted;
    Ok(engine::runs(snapshot, key, accepted.first, through.min(accepted.last))?
        .into_iter()
        .map(|run| EconomyRun(run.span.first, run.span.last, run.value.to_string()))
        .collect())
}

/// The tags a read gave `slot`'s holder.
fn tags(directory: &Directory, slot: u32) -> Vec<HolderTag> {
    let Some(actor) = directory.actor(slot.to_string().as_bytes()) else {
        return Vec::new();
    };
    let Some(scalar) = TAGS.checked_add(actor).and_then(|id| directory.scalar(id)) else {
        return Vec::new();
    };
    scalar
        .list()
        .iter()
        .filter_map(|entry| match entry.list() {
            [hour, tag] => Some(HolderTag {
                hour: i32::try_from(hour.integer()?).ok()?,
                tag: String::from_utf8(tag.text()?.to_vec()).ok()?,
            }),
            _ => None,
        })
        .collect()
}

/// Every country's totals over its own records, or, for one holder, its record of each partner,
/// through `through`, the last hour the family's gate prepared.
pub fn reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &LendLeaseBalanceParams,
    through: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<LendLeaseBalanceReply> {
    let (mut reply, holder) = empty(params, through)?;
    let Some(through) = through else {
        return Ok(reply);
    };
    if let Some(key) = directory.key(FAMILY, KNOWN, None) {
        reply.known = text(snapshot, key, through)?;
    }
    // Each holder's keys by measure: every owner's totals, or one owner's records by partner.
    let mut wanted: Vec<(u32, Vec<(usize, u64)>)> = Vec::new();
    match holder {
        None => {
            for (actor, named) in directory.actors.iter().enumerate() {
                let keys = directory.dimensions(actor as u64, TOTAL);
                if keys.is_empty() {
                    continue;
                }
                let slot = std::str::from_utf8(&named.name)
                    .ok()
                    .and_then(|name| name.parse::<u32>().ok())
                    .ok_or_else(|| dashboard_base::invalid("A lend-lease holder is not a slot"))?;
                let keys = keys
                    .iter()
                    .map(|&(measure, key)| (measure as usize, key))
                    .collect();
                wanted.push((slot, keys));
            }
            wanted.sort_unstable_by_key(|&(slot, _)| slot);
        }
        Some(holder) => {
            if let Some(actor) = directory.actor(holder.to_string().as_bytes()) {
                for &(dimension, key) in directory.dimensions(actor, RECORD) {
                    let partner = u32::try_from(dimension >> 2)
                        .map_err(|_| dashboard_base::invalid("A lend-lease partner is not a slot"))?;
                    let measure = (dimension & 3) as usize;
                    match wanted.last_mut() {
                        Some((held, keys)) if *held == partner => keys.push((measure, key)),
                        _ => wanted.push((partner, vec![(measure, key)])),
                    }
                }
            }
        }
    }
    for (slot, keys) in wanted {
        if cancelled.load(Ordering::Acquire) {
            return Err(dashboard_base::invalid("Economy history request cancelled"));
        }
        let mut values = vec![Vec::new(); MEASURES];
        for (measure, key) in keys {
            if let Some(runs) = values.get_mut(measure) {
                *runs = text(snapshot, key, through)?;
            }
        }
        if values.iter().all(Vec::is_empty) {
            continue;
        }
        reply.histories.push(LendLeaseHistory {
            holder: slot.to_string(),
            tags: tags(directory, slot),
            values,
        });
    }
    if let Some(refusals) = directory.scalar(REFUSALS) {
        reply.warnings = refusals
            .list()
            .iter()
            .filter_map(|entry| match entry.list() {
                [reason, reads] => Some(format!(
                    "Lend-lease balance: {} ({} observed hours)",
                    String::from_utf8_lossy(reason.text()?),
                    reads.integer()?
                )),
                _ => None,
            })
            .collect();
    }
    Ok(reply)
}
