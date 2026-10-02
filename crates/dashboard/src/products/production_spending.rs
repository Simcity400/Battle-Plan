//! Delivered IC at read cost: each new whole unit in ProductionLineOutput is
//! weighted by the matching ProductionLines cost at that read (fixed5).
//!
//! Speed is not exact factory output: ProduceEquipment@140F596C0 and naval
//! ProduceEquipment@140F59ED0 replace it on the instant-production paths.
//! Cost is not a delivery-tick price: SetEquipmentVariant@141926570 can replace
//! variant and cost between passes; RefreshProductionState@141926DB0 writes the
//! MIO-adjusted cost. Ordinary delivery divides by cost at 141921B80/141921C10.
//! This product therefore names its read-cost valuation explicitly. It never
//! reprices earlier deliveries or counts progress still in flight.
//!
//! A new delivery without its matching producer/design/MIO census cost is
//! unknown, including an ended refit removed by 140F5CA10. A final publication
//! with no new units needs no price and keeps its earlier exact value.

pub use super::equipment::production::ProductionSpendingUnknown;
use super::{
    LoadContext, Product,
    equipment::{
        Domain, EQUIPMENT_DECIMALS, StockVariantName, Texts,
        production::{self, CostTotal, Named, ScannedProduced, TagHistory},
    },
    validate_tag,
};
use crate::Source;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct ProductionSpending;
const MAX_REPLY_BYTES: usize = 24 * 1024 * 1024;
const MAX_RUNS: usize = 300_000;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionSpendingParams {
    pub branch_id: String,
    pub tag: String,
    pub refits: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Decimal string of the fixed5 total; null is unknown, never a partial sum.
    pub total_raw: Option<String>,
    pub reason: Option<ProductionSpendingUnknown>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingLeaf {
    pub variant: String,
    pub type_key: String,
    pub name: Option<String>,
    pub domain: Option<Domain>,
    pub names: Vec<StockVariantName>,
    pub runs: Vec<ProductionSpendingRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
    /// Domain → content archetype → variant is built from these exact leaves.
    pub leaves: Vec<ProductionSpendingLeaf>,
    /// The country's total is independent of unresolved type/domain membership.
    pub total: Vec<ProductionSpendingRun>,
    pub warnings: Vec<String>,
}

fn cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Production spending request cancelled",
        ))
    } else {
        Ok(())
    }
}

fn budget() -> io::Error {
    io::Error::other("The production spending reply exceeds its memory budget")
}

fn append(
    runs: &mut Vec<ProductionSpendingRun>,
    run: ProductionSpendingRun,
    count: &mut usize,
) -> io::Result<()> {
    if let Some(last) = runs.last_mut()
        && last.last_hour.checked_add(1) == Some(run.first_hour)
        && last.total_raw == run.total_raw
        && last.reason == run.reason
    {
        last.last_hour = run.last_hour;
    } else {
        *count += 1;
        if *count > MAX_RUNS {
            return Err(budget());
        }
        runs.push(run);
    }
    Ok(())
}

/// Groups sparse entry changes; reads with no delivery still carry exact zero.
fn histories(
    track: &TagHistory,
    refits: bool,
) -> io::Result<(BTreeMap<u64, Vec<(u32, CostTotal)>>, Vec<(u32, CostTotal)>)> {
    let mut events: BTreeMap<u32, Vec<((u64, bool), CostTotal)>> = BTreeMap::new();
    let mut count = 0;
    for (entry, list) in &track.cost_changes {
        if !refits && entry.1 {
            continue;
        }
        count += list.len();
        if count > MAX_RUNS {
            return Err(budget());
        }
        for &(sample, total) in list {
            events.entry(sample).or_default().push((*entry, total));
        }
    }
    let mut current = BTreeMap::new();
    let mut variants: BTreeMap<u64, CostTotal> = BTreeMap::new();
    let mut grouped: BTreeMap<u64, Vec<(u32, CostTotal)>> = BTreeMap::new();
    let mut totals = Vec::new();
    let mut sum = CostTotal::default();
    for (sample, changed) in events {
        let mut touched = BTreeSet::new();
        for (entry, total) in changed {
            let before: CostTotal = current.insert(entry, total).unwrap_or_default();
            let variant = variants.entry(entry.0).or_default();
            for value in [&mut sum, variant] {
                value.raw = value
                    .raw
                    .checked_sub(before.raw)
                    .and_then(|sum| sum.checked_add(total.raw))
                    .ok_or_else(|| {
                        io::Error::other("Delivered IC exceeds its exact integer range")
                    })?;
                value.reason = value.reason.or(total.reason);
            }
            touched.insert(entry.0);
        }
        for variant in touched {
            grouped
                .entry(variant)
                .or_default()
                .push((sample, variants[&variant]));
        }
        totals.push((sample, sum));
    }
    Ok((grouped, totals))
}

#[derive(Clone, Copy)]
struct Coverage {
    first: i32,
    last: i32,
    reason: Option<ProductionSpendingUnknown>,
}

fn coverage(scanned: &ScannedProduced, track: &TagHistory) -> Vec<Coverage> {
    let first_identity = track.identities.first().copied();
    let tainted = scanned
        .spending_tainted
        .values()
        .min_by_key(|(at, _)| *at)
        .copied();
    let restart = scanned.observations.get(1).copied();
    let mut sticky = None;
    let mut spans: Vec<Coverage> = Vec::new();
    for (sample, &hour) in track.hours.iter().enumerate() {
        let identity = track.identities[sample];
        if tainted.is_some_and(|(from, _)| hour >= from) {
            sticky = sticky.or(tainted.map(|(_, reason)| reason));
        }
        if restart.is_some_and(|from| hour >= from) {
            sticky = sticky.or(Some(ProductionSpendingUnknown::ObservationRestarted));
        }
        if first_identity.is_some_and(|first| (first.0, first.1) != (identity.0, identity.1)) {
            sticky = sticky.or(Some(ProductionSpendingUnknown::CountryChanged));
        }
        let last = track.last_hours[sample];
        if let Some(previous) = spans.last_mut()
            && previous.last.checked_add(1) == Some(hour)
            && previous.reason == sticky
        {
            previous.last = last;
        } else {
            spans.push(Coverage {
                first: hour,
                last,
                reason: sticky,
            });
        }
    }
    spans
}

fn runs(
    track: &TagHistory,
    coverage: &[Coverage],
    values: &[(u32, CostTotal)],
    count: &mut usize,
    cancel: &AtomicBool,
) -> io::Result<Vec<ProductionSpendingRun>> {
    let mut result = Vec::new();
    let mut value = CostTotal::default();
    let mut next = 0;
    let mut sticky = None;
    for span in coverage {
        cancelled(cancel)?;
        while values
            .get(next)
            .is_some_and(|(at, _)| track.hours[*at as usize] <= span.first)
        {
            value = values[next].1;
            sticky = sticky.or(value.reason);
            next += 1;
        }
        let mut first = span.first;
        loop {
            let changed = values
                .get(next)
                .map(|(at, _)| track.hours[*at as usize])
                .filter(|&hour| hour <= span.last);
            let last = changed.map_or(span.last, |hour| hour - 1);
            let reason = span.reason.or(sticky);
            append(
                &mut result,
                ProductionSpendingRun {
                    first_hour: first,
                    last_hour: last,
                    total_raw: reason.is_none().then(|| value.raw.to_string()),
                    reason,
                },
                count,
            )?;
            let Some(hour) = changed else {
                break;
            };
            first = hour;
            value = values[next].1;
            sticky = sticky.or(value.reason);
            next += 1;
        }
    }
    Ok(result)
}

struct Counter<'a> {
    bytes: usize,
    cancel: &'a AtomicBool,
}
impl Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        cancelled(self.cancel)?;
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > MAX_REPLY_BYTES {
            return Err(budget());
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn reply(
    scanned: &ScannedProduced,
    params: &ProductionSpendingParams,
    named: Named,
    mut warnings: Vec<String>,
    cancel: &AtomicBool,
) -> io::Result<ProductionSpendingReply> {
    let track = scanned.tags.get(&params.tag);
    let variants: BTreeSet<u64> = track
        .into_iter()
        .flat_map(|track| track.changes.keys().chain(track.cost_changes.keys()))
        .filter(|(_, refit)| params.refits || !*refit)
        .map(|(variant, _)| *variant)
        .collect();
    let (types, reasons, unresolved) =
        production::variant_types_for(&scanned.variants, variants.iter().copied(), named);
    warnings.extend(super::equipment::marked_keys_warning(
        types
            .values()
            .filter(|kind| kind.ordinal.is_some())
            .map(|kind| kind.key.as_str()),
    ));
    if unresolved > 0 {
        warnings.push(format!("{unresolved} produced variants have types the recorded content cannot resolve; each is kept under its recorded token in the unresolved arm."));
    }
    for reason in reasons.iter().take(8) {
        warnings.push(format!("An equipment type cannot be resolved: {reason}."));
    }
    if scanned.reads.is_empty() {
        warnings.push(format!(
            "Production spending is not known: no readable ProductionLineOutput{}.",
            scanned
                .unread_reason
                .as_ref()
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default()
        ));
    }
    if !scanned.spending_tainted.is_empty() {
        warnings.push("A recording gap or missed output publication ends exact delivered IC; ended lines publish their final count only once.".into());
    }
    if scanned.observations.len() > 1 {
        warnings.push("The output observation restarted; deliveries across the restart are not known, so the campaign total remains Not known.".into());
    }
    if scanned.unlisted > 0 {
        warnings.push(format!("{} lines have no recorded census kind; as Equipment produced does, they count as new production. New deliveries without a matching read cost remain Not known.", scanned.unlisted));
    }
    let mut leaves = Vec::new();
    let mut total = Vec::new();
    let mut count = 0;
    let mut size = Counter { bytes: 0, cancel };
    if let Some(track) = track {
        let coverage = coverage(scanned, track);
        let (history, total_history) = histories(track, params.refits)?;
        let mut variants: Vec<_> = variants.into_iter().collect();
        variants.sort_by_key(|variant| {
            types.get(variant).map(|kind| {
                (
                    kind.ordinal.unwrap_or(usize::MAX),
                    kind.key.clone(),
                    *variant,
                )
            })
        });
        for variant in variants {
            cancelled(cancel)?;
            if scanned
                .variants
                .versions
                .get(&variant)
                .is_some_and(|versions| versions.len() > MAX_RUNS)
            {
                return Err(budget());
            }
            let Some(kind) = types.get(&variant) else {
                continue;
            };
            let leaf = ProductionSpendingLeaf {
                variant: variant.to_string(),
                type_key: kind.key.clone(),
                name: named.type_name(&kind.key),
                domain: kind.domain,
                names: scanned.variants.names(variant, named.texts),
                runs: runs(
                    track,
                    &coverage,
                    history.get(&variant).map(Vec::as_slice).unwrap_or(&[]),
                    &mut count,
                    cancel,
                )?,
            };
            serde_json::to_writer(&mut size, &leaf)
                .map_err(|error| io::Error::other(error.to_string()))?;
            leaves.push(leaf);
        }
        total = runs(track, &coverage, &total_history, &mut count, cancel)?;
    } else {
        warnings.push(format!(
            "{}: no readable country production history in this branch",
            params.tag
        ));
    }
    let result = ProductionSpendingReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        decimals: EQUIPMENT_DECIMALS,
        reads: scanned.reads.clone(),
        observations: scanned.observations.clone(),
        leaves,
        total,
        warnings,
    };
    serde_json::to_writer(Counter { bytes: 0, cancel }, &result)
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(result)
}

impl Product for ProductionSpending {
    const KEY: &'static str = "production-spending";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionSpendingParams;
    type Reply = ProductionSpendingReply;

    fn types() -> Vec<String> {
        vec![
            ProductionSpendingParams::decl(),
            ProductionSpendingUnknown::decl(),
            ProductionSpendingRun::decl(),
            ProductionSpendingLeaf::decl(),
            ProductionSpendingReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        validate_tag(&params.tag)
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancel: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().expect("validated branch");
        let scanned = production::scanned(source, branch, cancel)?;
        let mut warnings = Vec::new();
        let (catalogue, units) = production::content(source, context, &mut warnings);
        reply(
            &scanned,
            params,
            Named {
                units: units.as_deref(),
                texts: catalogue
                    .as_ref()
                    .map(|catalogue| &**catalogue as &dyn Texts),
            },
            warnings,
            cancel,
        )
    }
}

#[cfg(test)]
mod tests;
