//! Fuel by branch's replies, read from its series publication.

use super::{
    FuelBranch, FuelBranchCountry, FuelBranchesParams, FuelBranchesReply,
    emit::{FAMILY, KNOWN, PRIORITY, READ, RECEIVED, REQUESTED, query_tags, reasons},
};
use crate::products::keyed::{EconomyRun, clip_text, holder_tags, tag_spans};
use dashboard_base::prepared::engine;
use dashboard_series::{PageRead, Snapshot, family::Directory};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};

fn text<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    through: i32,
) -> io::Result<Vec<EconomyRun>> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    let accepted = snapshot.publication().accepted;
    Ok(
        engine::runs(snapshot, key, accepted.first, accepted.last.min(through))?
            .into_iter()
            .map(|run| EconomyRun(run.span.first, run.span.last, run.value.to_string()))
            .collect(),
    )
}

pub fn unpublished(
    params: &FuelBranchesParams,
    through: Option<i32>,
    warnings: Vec<String>,
) -> FuelBranchesReply {
    FuelBranchesReply {
        branch_id: params.branch_id.clone(),
        last_hour: through,
        consumer_names: Vec::new(),
        scale: 100_000,
        known: Vec::new(),
        countries: Vec::new(),
        warnings,
    }
}

pub fn reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &FuelBranchesParams,
    through: i32,
    labels: [Option<String>; 3],
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<FuelBranchesReply> {
    let mut reply = unpublished(params, Some(through), Vec::new());
    reply.known = text(snapshot, directory.key(FAMILY, KNOWN, None), through)?;
    let mut holders = BTreeMap::new();
    for (actor, named) in directory.actors.iter().enumerate() {
        let actor = actor as u64;
        let slot: u32 = std::str::from_utf8(&named.name)
            .ok()
            .and_then(|name| name.parse().ok())
            .ok_or_else(|| dashboard_base::invalid("A prepared fuel branch holder is not a slot"))?;
        let tags = query_tags(directory, actor)?;
        if !tags.is_empty() {
            holders.insert(slot, (actor, tags));
        }
    }
    let mut indexes = BTreeSet::new();
    for (slot, (actor, tags)) in holders {
        let spans = if params.tags.is_empty() {
            vec![(i32::MIN, i32::MAX)]
        } else {
            tag_spans(&tags, &params.tags)
        };
        if spans.is_empty() {
            continue;
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(dashboard_base::invalid("Economy history request cancelled"));
        }
        let read = text(snapshot, directory.key(actor, READ, None), through)?;
        let mut country = FuelBranchCountry {
            holder: slot.to_string(),
            tags: holder_tags(&tags),
            read: clip_text(&read, &spans),
            branches: Vec::new(),
        };
        let mut branches = BTreeMap::<u32, FuelBranch>::new();
        for measure in [REQUESTED, RECEIVED, PRIORITY] {
            for &(index, key) in directory.dimensions(actor, measure) {
                let index = u32::try_from(index)
                    .map_err(|_| dashboard_base::invalid("A prepared fuel consumer index is not valid"))?;
                let runs = clip_text(&text(snapshot, Some(key), through)?, &spans);
                let branch = branches.entry(index).or_insert_with(|| FuelBranch {
                    index,
                    requested: Vec::new(),
                    received: Vec::new(),
                    priority: Vec::new(),
                });
                match measure {
                    REQUESTED => branch.requested = runs,
                    RECEIVED => branch.received = runs,
                    _ => branch.priority = runs,
                }
            }
        }
        branches.retain(|_, branch| !branch.requested.is_empty() || !branch.received.is_empty());
        if branches.is_empty() {
            continue;
        }
        indexes.extend(branches.keys().copied());
        country.branches = branches.into_values().collect();
        reply.countries.push(country);
    }
    let last = indexes
        .last()
        .map_or(0, |last| *last as usize + 1)
        .max(labels.len());
    reply.consumer_names = (0..last)
        .map(|index| labels.get(index).cloned().flatten())
        .collect();
    let beyond = indexes
        .iter()
        .filter(|&&index| index as usize >= labels.len())
        .count();
    if beyond > 0 {
        warnings.push(format!(
            "{beyond} fuel consumers beyond the fuel panel's three have no content label"
        ));
    }
    let refusals: BTreeMap<String, u64> = reasons(directory)?.into_iter().collect();
    warnings.extend(
        refusals
            .iter()
            .map(|(reason, hours)| format!("Fuel by branch: {reason} ({hours} observed hours)")),
    );
    reply.warnings = warnings;
    Ok(reply)
}
