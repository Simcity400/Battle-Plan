//! Dockyard spending reads production's prepared endpoints and spent series and
//! the prepared repair rate, through the hours both families have prepared.
use super::*;
use crate::Source;
use crate::prepared::series::Run;
use crate::prepared_production::ProductionNaming;
use crate::prepared_production::{self as pages, Classification};
use crate::products::dockyard_repairs::{RepairPart, RepairReads};
use crate::products::equipment::production::variant_types_for;
use crate::products::equipment::{Domain, UNRESOLVED_MARK};
use crate::products::production_spending::{
    Purposes,
    prepared::{Answered, Params as SpendingParams, query_with},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    tag: String,
    view: DockyardView,
    from: Option<i32>,
    hour: Option<i32>,
    end: Option<i32>,
}

fn cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Dockyard spending request cancelled",
        ))
    } else {
        Ok(())
    }
}

pub fn query(
    source: &Source,
    params: Value,
    content: Option<&super::super::ContentOptions>,
    cancel: &AtomicBool,
) -> io::Result<Option<Value>> {
    let input: Params = serde_json::from_value(params)?;
    let spending = SpendingParams {
        branch_id: input.branch_id.clone(),
        tag: input.tag.clone(),
        refits: true,
        from: input.from,
        hour: input.hour,
        end: input.end,
    };
    let (mut data, answered) = query_with(
        source,
        spending,
        content,
        DockyardSpending::KEY,
        Purposes::Split,
        cancel,
    )?;
    let (store, key) = crate::prepared::files(source, content)?;
    let mut repairs = RepairReads::new(&store, &key, answered.branch, cancel)?;
    let mismatches = repairs.allocation_mismatches();
    if mismatches > 0 {
        data["warnings"]
            .as_array_mut()
            .ok_or_else(|| crate::invalid("Dockyard spending warnings are not a list"))?
            .push(json!(format!(
                "{mismatches} repair reads had naval base assignments that did not sum to their country's dockyards in use; repairs there are not split by design."
            )));
    }
    match input.view {
        DockyardView::Circle => {
            keep_naval(&mut data)?;
            // A whole campaign's repairs start where the country's output is
            // first known, named when that is after the recording's start.
            let origin = repairs.origin(&input.tag)?;
            let start = answered.from.or(origin);
            let since =
                origin.filter(|&origin| answered.from.is_none() && origin > answered.bounds.0);
            let ends = [answered.hour, answered.end];
            let [hour, end] =
                window_integrals(start, ends, |spans| repairs.integrals(&input.tag, spans))?;
            let parts = repair_parts(source, content, &mut repairs, &input.tag, start, ends)?;
            data["repairs"] = serde_json::to_value(DockyardRepairs {
                denominator: REPAIR_DENOMINATOR,
                hour: DockyardAmount::of(hour),
                end: DockyardAmount::of(end),
                since,
                parts,
            })?;
            data["months"] = json!([]);
        }
        DockyardView::Months => {
            let months = month_columns(&store, &key, &input.tag, &answered, &mut repairs, cancel)?;
            data["leaves"] = json!([]);
            data["peaks"] = json!({});
            data["repairs"] = serde_json::to_value(DockyardRepairs {
                denominator: REPAIR_DENOMINATOR,
                ..Default::default()
            })?;
            data["months"] = serde_json::to_value(months)?;
        }
    }
    Ok(Some(data))
}

pub(super) type Integral = Result<i128, ProductionSpendingUnknown>;

/// Repair integrals from `start` through each of `ends`, from `integrals` of
/// the nonempty spans. A window ending before the rate's origin, or without
/// one, is not known: no output was recorded there, which is not zero.
pub(super) fn window_integrals(
    start: Option<i32>,
    ends: [i32; 2],
    integrals: impl FnOnce(&[(i32, i32)]) -> io::Result<Vec<Integral>>,
) -> io::Result<[Integral; 2]> {
    let Some(start) = start else {
        return Ok([Err(ProductionSpendingUnknown::NotRecorded); 2]);
    };
    let spans: Vec<(i32, i32)> = ends
        .iter()
        .filter(|&&end| end >= start)
        .map(|&end| (start, end))
        .collect();
    let mut integrals = integrals(&spans)?.into_iter();
    Ok(ends.map(|end| {
        if end < start {
            Err(ProductionSpendingUnknown::NotRecorded)
        } else {
            integrals
                .next()
                .unwrap_or(Err(ProductionSpendingUnknown::NotRecorded))
        }
    }))
}

/// Each repaired design's share of the window's repairs, named and typed
/// through the recorded content stack, and the unattributed rest.
fn repair_parts(
    source: &Source,
    content: Option<&super::super::ContentOptions>,
    repairs: &mut RepairReads<'_>,
    tag: &str,
    start: Option<i32>,
    ends: [i32; 2],
) -> io::Result<Vec<DockyardRepairPart>> {
    let parts = repairs.parts(tag)?;
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    let naming = ProductionNaming::load(
        source,
        &super::super::LoadContext {
            content: content.cloned(),
        },
    );
    let named = naming.named();
    let designs = parts.iter().filter_map(|part| match part {
        RepairPart::Design(variant) => Some(*variant),
        RepairPart::Unattributed => None,
    });
    let (types, _, _) = variant_types_for(repairs.variants(), designs, named);
    let mut result = Vec::with_capacity(parts.len());
    for part in parts {
        let described = match part {
            RepairPart::Design(variant) => {
                let kind = types.get(&variant).expect("every repaired design is typed");
                DockyardRepairPart {
                    variant: Some(variant.to_string()),
                    type_key: kind.key.clone(),
                    name: named.type_name(&kind.key),
                    domain: kind.domain,
                    names: repairs.variants().names(variant, named.texts),
                    ..Default::default()
                }
            }
            RepairPart::Unattributed => DockyardRepairPart::default(),
        };
        let [hour, end] = window_integrals(start, ends, |spans| {
            repairs.part_integrals(tag, part, spans)
        })?;
        result.push(DockyardRepairPart {
            hour: DockyardAmount::of(hour),
            end: DockyardAmount::of(end),
            ..described
        });
    }
    Ok(result)
}

/// Whether a leaf's design could be naval: its type is naval, or content
/// cannot resolve it.
fn possibly_naval(leaf: &Value) -> bool {
    leaf["rocket"] != true
        && (leaf["domain"] == json!(Domain::Naval)
            || leaf["typeKey"]
                .as_str()
                .is_some_and(|key| key.starts_with(UNRESOLVED_MARK)))
}

fn keep_naval(data: &mut Value) -> io::Result<()> {
    data["leaves"]
        .as_array_mut()
        .ok_or_else(|| crate::invalid("Production spending leaves are not a list"))?
        .retain(possibly_naval);
    Ok(())
}

/// A design's spent series through the reply's hour: its new production's
/// and its combined new and refit input.
pub(super) struct Design {
    pub unresolved: bool,
    pub new: Vec<Run>,
    pub combined: Vec<Run>,
    /// Each purpose's cumulative input of lines whose purpose is not
    /// recorded (new, then refit); empty when it never had any.
    pub unproven: [Vec<Run>; 2],
}

fn covering(runs: &[Run], hour: i32) -> Option<&Run> {
    let index = runs.partition_point(|run| run.last_hour < hour);
    runs.get(index).filter(|run| run.first_hour <= hour)
}

/// Where a cumulative series' change over `first..=last` is measured from:
/// None before the design had any input, `Some(None)` from zero (its first
/// input in that span, or its observation's start), `Some(Some(hour))` from its
/// value at `hour`. Err when an hour between cannot be told.
fn baseline(
    runs: &[Run],
    first: i32,
    last: i32,
    origins: &[i32],
) -> Result<Option<Option<i32>>, ProductionSpendingUnknown> {
    let index = runs.partition_point(|run| run.last_hour < last);
    match runs.get(index) {
        Some(run) if run.first_hour <= last => {}
        // Input that has not begun is none yet; one that ended is unknown.
        _ if runs.first().is_none_or(|run| run.first_hour > last) => return Ok(None),
        _ => return Err(ProductionSpendingUnknown::SpendingUnavailable),
    }
    let mut at = index;
    loop {
        let run = &runs[at];
        if run.first_hour < first {
            return Ok(Some(Some(first - 1)));
        }
        if run.break_before {
            return if at == 0 && origins.contains(&run.first_hour) {
                Ok(Some(None))
            } else {
                Err(ProductionSpendingUnknown::CountryChanged)
            };
        }
        if at == 0 {
            return Ok(Some(None));
        }
        if runs[at - 1].last_hour.checked_add(1) != Some(run.first_hour) {
            return Err(ProductionSpendingUnknown::SpendingUnavailable);
        }
        at -= 1;
    }
}

/// A cumulative series' value at a baseline, zero where it holds none: a
/// design's new production has no series until its first new input.
fn value(runs: &[Run], at: Option<i32>) -> i128 {
    at.and_then(|hour| covering(runs, hour))
        .map_or(0, |run| run.value)
}

/// One design's new and refit input over `first..=last`.
pub(super) fn design_change(
    design: &Design,
    first: i32,
    last: i32,
    origins: &[i32],
) -> Result<(i128, i128), ProductionSpendingUnknown> {
    let Some(from) = baseline(&design.combined, first, last, origins)? else {
        return Ok((0, 0));
    };
    let overflow = || ProductionSpendingUnknown::SpendingUnavailable;
    let combined = value(&design.combined, Some(last))
        .checked_sub(value(&design.combined, from))
        .ok_or_else(overflow)?;
    let new = value(&design.new, Some(last))
        .checked_sub(value(&design.new, from))
        .ok_or_else(overflow)?;
    Ok((new, combined.checked_sub(new).ok_or_else(overflow)?))
}

/// Whether input of lines whose purpose is not recorded joined the design
/// in `first..=last`, read against the same baseline as its amounts.
pub(super) fn unproven_in(
    design: &Design,
    first: i32,
    last: i32,
    origins: &[i32],
) -> Result<bool, ProductionSpendingUnknown> {
    let Some(from) = baseline(&design.combined, first, last, origins)? else {
        return Ok(false);
    };
    Ok(design
        .unproven
        .iter()
        .any(|runs| value(runs, Some(last)) != value(runs, from)))
}

/// When the equipment source's spending is known: from its first read until
/// the first taint of its observation or its restart.
pub(super) struct SourceCoverage {
    first_read: Option<i32>,
    tainted: Option<(i32, ProductionSpendingUnknown)>,
    restart: Option<i32>,
}

impl SourceCoverage {
    pub(super) fn of(scanned: &crate::products::equipment::production::ScannedProduced) -> Self {
        Self {
            first_read: scanned.reads.first().copied(),
            tainted: scanned
                .spending_tainted
                .values()
                .min_by_key(|(at, _)| *at)
                .copied(),
            restart: scanned.observations.get(1).copied(),
        }
    }

    /// Whether a month ending at `last` lies where spending is known. A
    /// month the recording starts in counts from its first read.
    pub(super) fn month(&self, last: i32) -> Result<(), ProductionSpendingUnknown> {
        if self.first_read.is_none_or(|read| read > last) {
            return Err(ProductionSpendingUnknown::NotRecorded);
        }
        if let Some((_, reason)) = self.tainted.filter(|(at, _)| *at <= last) {
            return Err(reason);
        }
        if self.restart.is_some_and(|restart| restart <= last) {
            return Err(ProductionSpendingUnknown::ObservationRestarted);
        }
        Ok(())
    }
}

/// The calendar months from the window's first hour through its hour, each
/// with exact new, refit and repair amounts or the reason one is unknown.
fn month_columns(
    store: &store::Store,
    key: &store::SourceKey,
    tag: &str,
    answered: &Answered,
    repairs: &mut RepairReads<'_>,
    cancel: &AtomicBool,
) -> io::Result<Vec<DockyardMonth>> {
    let first = answered
        .from
        .unwrap_or(answered.bounds.0)
        .max(answered.bounds.0);
    let last = answered.hour.min(answered.bounds.1);
    if first > last {
        return Ok(Vec::new());
    }
    let count = pages::endpoint_count(store, key, answered.branch)?;
    let mut reads = pages::Endpoints::new(store, key, answered.branch, count, cancel);
    let Some(catalogue) = reads.catalogue()? else {
        return Ok(Vec::new());
    };
    let scanned = catalogue.spending.scanned(tag);
    let coverage = SourceCoverage::of(&scanned);
    let origins = scanned.observations;
    let classes: BTreeMap<u64, Classification> = catalogue.classifications.clone();
    let mut keys: BTreeMap<u64, [Option<u64>; 2]> = BTreeMap::new();
    for (token, &id) in &catalogue.spent_keys {
        let (owner, rocket, refits, variant): (String, bool, bool, u64) =
            serde_json::from_str(token)?;
        if owner == tag && !rocket {
            keys.entry(variant).or_default()[usize::from(refits)] = Some(id);
        }
    }
    let mut unproven: BTreeMap<u64, [Option<u64>; 2]> = BTreeMap::new();
    for (token, &id) in &catalogue.unproven_keys {
        let (owner, variant, refit): (String, u64, bool) = serde_json::from_str(token)?;
        if owner == tag {
            unproven.entry(variant).or_default()[usize::from(refit)] = Some(id);
        }
    }
    let reader = reads.reader()?;
    let mut designs = Vec::new();
    for (variant, [new, combined]) in keys {
        cancelled(cancel)?;
        let class = classes.get(&variant);
        let unresolved = class.is_none_or(|class| class.ordinal.is_none());
        if !unresolved && class.and_then(|class| class.domain) != Some(Domain::Naval) {
            continue;
        }
        let Some(combined) = combined else { continue };
        let mut read = |id: Option<u64>| match id {
            Some(id) => reader.runs(id, answered.bounds.0, last),
            None => Ok(Vec::new()),
        };
        let [unproven_new, unproven_refit] = unproven.get(&variant).copied().unwrap_or_default();
        designs.push(Design {
            unresolved,
            new: read(new)?,
            combined: read(Some(combined))?,
            unproven: [read(unproven_new)?, read(unproven_refit)?],
        });
    }
    let spans = months(first, last);
    let repaired = repairs.integrals(tag, &spans)?;
    let mut result = Vec::with_capacity(spans.len());
    for (&(month_first, month_last), repair) in spans.iter().zip(repaired) {
        cancelled(cancel)?;
        // A month no read covers, or after a taint or restart, is not known,
        // whatever its designs' series hold.
        let mut sums = coverage.month(month_last).map(|()| (0_i128, 0_i128));
        let mut unresolved = false;
        let mut purpose_unproven = false;
        for design in &designs {
            // Unproven input of a design that could be naval, in this month.
            purpose_unproven |=
                unproven_in(design, month_first, month_last, &origins).unwrap_or(false);
            let change = design_change(design, month_first, month_last, &origins);
            if design.unresolved {
                unresolved |= change != Ok((0, 0));
                continue;
            }
            sums = match (sums, change) {
                (Ok((new, refit)), Ok((more_new, more_refit))) => new
                    .checked_add(more_new)
                    .zip(refit.checked_add(more_refit))
                    .ok_or(ProductionSpendingUnknown::SpendingUnavailable),
                (Err(reason), _) | (_, Err(reason)) => Err(reason),
            };
        }
        result.push(DockyardMonth {
            first_hour: month_first,
            last_hour: month_last,
            new_ships: DockyardAmount::of(sums.map(|(new, _)| new)),
            refits: DockyardAmount::of(sums.map(|(_, refit)| refit)),
            repairs: DockyardAmount::of(repair),
            purpose_unproven,
            unresolved,
        });
    }
    Ok(result)
}
