//! Exact signed equipment-line factory IC from the applied progress input,
//! independently of delivery or the later census cost. Instant progress is
//! recorded separately and excluded. See engine_production::line_output_sites for the
//! four callers and their instant predicates. Deleted lines retain their
//! last publication through the shared Equipment produced branch scan.
//! Rocket sites use their independent retained country/design observation.

pub use super::equipment::production::ProductionSpendingUnknown;
use super::{
    LoadContext, Product,
    equipment::{
        Domain, EQUIPMENT_DECIMALS, StockVariantName, Texts,
        production::{self, CostTotal, Entry, Named, Purpose, ScannedProduced, TagHistory},
    },
    validate_tag,
};
use crate::Source;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct ProductionSpending;
pub mod prepared;
use super::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;

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
    pub rocket: bool,
    /// In a reply split by purpose, what this input was for (one design can
    /// have a leaf per purpose); null otherwise.
    pub purpose: Option<Purpose>,
    /// The summed magnitudes of the input of production ticks whose purpose
    /// was not recorded, which counts in `runs` (as new production where the
    /// reply is not split by purpose): dated evidence that
    /// grows and never cancels; empty when there never was any.
    pub unproven: Vec<ProductionSpendingRun>,
    pub names: Vec<StockVariantName>,
    pub runs: Vec<ProductionSpendingRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProductionPresenceRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Any owned line or observed Rocket event; null cannot certify emptiness.
    pub present: Option<bool>,
    /// Supplying equipment census/output read, with independent Rocket absence.
    pub empty_census_hour: Option<i32>,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
    pub equipment_observations: Vec<i32>,
    pub rocket_observations: Vec<i32>,
    pub equipment_reads: Vec<i32>,
    pub rocket_reads: Vec<i32>,
    pub rocket_key: Option<String>,
    pub rocket_name: Option<String>,
    /// Production source → content archetype → variant uses these exact leaves.
    pub leaves: Vec<ProductionSpendingLeaf>,
    /// The country's total is independent of unresolved type/domain membership.
    pub total: Vec<ProductionSpendingRun>,
    /// The equipment source's own total, whatever Rocket coverage holds;
    /// only in a reply split by purpose, empty otherwise.
    pub equipment_total: Vec<ProductionSpendingRun>,
    pub rocket_total: Vec<ProductionSpendingRun>,
    pub production: Vec<ProductionPresenceRun>,
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

fn append(runs: &mut Vec<ProductionSpendingRun>, run: ProductionSpendingRun) {
    if let Some(last) = runs.last_mut()
        && last.last_hour.checked_add(1) == Some(run.first_hour)
        && last.total_raw == run.total_raw
        && last.reason == run.reason
    {
        last.last_hour = run.last_hour;
    } else {
        runs.push(run);
    }
}

/// Applied equipment-line IC at each read that changed it.
type CostSeries = Vec<(u32, CostTotal)>;

/// Which applied input a reply counts, and how it keys its leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purposes {
    /// New production and conversions, without ship refits, one leaf per design.
    WithoutRefits,
    /// Every purpose, one leaf per design.
    Combined,
    /// Every purpose, one leaf per design and purpose.
    Split,
}

impl Purposes {
    /// What a reply counts with refits included or not.
    fn counting(refits: bool) -> Self {
        if refits {
            Self::Combined
        } else {
            Self::WithoutRefits
        }
    }

    /// The leaf an entry's input belongs to, if the reply counts it.
    fn leaf(self, (variant, purpose): Entry) -> Option<Entry> {
        match self {
            Self::WithoutRefits => (purpose != Purpose::Refit).then_some((variant, Purpose::New)),
            Self::Combined => Some((variant, Purpose::New)),
            Self::Split => Some((variant, purpose)),
        }
    }
}

/// Groups sparse entry changes; reads with no applied input carry exact zero.
fn histories(
    track: &TagHistory,
    purposes: Purposes,
) -> io::Result<(BTreeMap<Entry, CostSeries>, CostSeries)> {
    let mut events: BTreeMap<u32, Vec<(Entry, CostTotal)>> = BTreeMap::new();
    for (entry, list) in &track.cost_changes {
        if purposes.leaf(*entry).is_none() {
            continue;
        }
        for &(sample, total) in list {
            events.entry(sample).or_default().push((*entry, total));
        }
    }
    let mut current = BTreeMap::new();
    let mut variants: BTreeMap<Entry, CostTotal> = BTreeMap::new();
    let mut grouped: BTreeMap<Entry, CostSeries> = BTreeMap::new();
    let mut totals = Vec::new();
    let mut sum = CostTotal::default();
    for (sample, changed) in events {
        let mut touched = BTreeSet::new();
        for (entry, total) in changed {
            let before: CostTotal = current.insert(entry, total).unwrap_or_default();
            let leaf = purposes.leaf(entry).expect("counted entry");
            let variant = variants.entry(leaf).or_default();
            for value in [&mut sum, variant] {
                value.raw = value
                    .raw
                    .checked_sub(before.raw)
                    .and_then(|sum| sum.checked_add(total.raw))
                    .ok_or_else(|| {
                        io::Error::other("Applied IC exceeds its exact integer range")
                    })?;
                value.reason = value.reason.or(total.reason);
                value.unproven_raw = value
                    .unproven_raw
                    .checked_sub(before.unproven_raw)
                    .and_then(|sum| sum.checked_add(total.unproven_raw))
                    .ok_or_else(|| {
                        io::Error::other("Applied IC exceeds its exact integer range")
                    })?;
            }
            touched.insert(leaf);
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

fn rocket_coverage(scanned: &ScannedProduced, track: &TagHistory) -> Vec<Coverage> {
    let first = track.identities.first();
    let restart = scanned.rocket.observations.get(1).copied();
    track
        .hours
        .iter()
        .enumerate()
        .map(|(sample, &hour)| Coverage {
            first: hour,
            last: track.last_hours[sample],
            reason: if restart.is_some_and(|from| hour >= from) {
                Some(ProductionSpendingUnknown::ObservationRestarted)
            } else if first.is_some_and(|first| {
                (first.0, first.1) != (track.identities[sample].0, track.identities[sample].1)
            }) {
                Some(ProductionSpendingUnknown::CountryChanged)
            } else {
                None
            },
        })
        .collect()
}

/// Both independent sources must cover a combined total.
fn combined(
    equipment: &[ProductionSpendingRun],
    rocket: &[ProductionSpendingRun],
    equipment_observations: &[i32],
    rocket_observations: &[i32],
) -> io::Result<Vec<ProductionSpendingRun>> {
    let boundaries: BTreeSet<i64> = equipment
        .iter()
        .chain(rocket)
        .flat_map(|run| [i64::from(run.first_hour), i64::from(run.last_hour) + 1])
        .collect();
    let boundaries: Vec<_> = boundaries.into_iter().collect();
    let mut result = Vec::new();
    let (mut eq, mut ro) = (0, 0);
    for pair in boundaries.windows(2) {
        let first = pair[0] as i32;
        while equipment.get(eq).is_some_and(|run| run.last_hour < first) {
            eq += 1;
        }
        while rocket.get(ro).is_some_and(|run| run.last_hour < first) {
            ro += 1;
        }
        let e = equipment.get(eq).filter(|run| run.first_hour <= first);
        let r = rocket.get(ro).filter(|run| run.first_hour <= first);
        if e.is_none() && r.is_none() {
            continue;
        }
        let mut reason = e
            .map_or(Some(ProductionSpendingUnknown::NotRecorded), |run| {
                run.reason
            })
            .or_else(|| {
                r.map_or(
                    Some(ProductionSpendingUnknown::RocketTableUnavailable),
                    |run| run.reason,
                )
            });
        let origin = |observations: &[i32]| {
            observations
                .partition_point(|since| *since <= first)
                .checked_sub(1)
                .map(|index| observations[index])
        };
        if let (Some(equipment), Some(rocket)) =
            (origin(equipment_observations), origin(rocket_observations))
            && equipment != rocket
        {
            reason = reason.or(Some(ProductionSpendingUnknown::ObservationMismatch));
        }
        let total_raw = if reason.is_none() {
            let e: i128 = e
                .and_then(|run| run.total_raw.as_ref())
                .expect("known equipment amount")
                .parse()
                .map_err(|_| io::Error::other("Equipment IC amount is malformed"))?;
            let r: i128 = r
                .and_then(|run| run.total_raw.as_ref())
                .expect("known Rocket amount")
                .parse()
                .map_err(|_| io::Error::other("Rocket IC amount is malformed"))?;
            Some(
                e.checked_add(r)
                    .ok_or_else(|| {
                        io::Error::other("Combined production IC exceeds its exact integer range")
                    })?
                    .to_string(),
            )
        } else {
            None
        };
        append(
            &mut result,
            ProductionSpendingRun {
                first_hour: first,
                last_hour: (pair[1] - 1) as i32,
                total_raw,
                reason,
            },
        );
    }
    Ok(result)
}

fn rocket_name(
    buildings: &installed_content_core::buildings::Buildings,
    texts: Option<&dyn Texts>,
) -> Result<(String, Option<String>), String> {
    let keys: Vec<_> = buildings
        .declaration_order
        .iter()
        .filter(|key| {
            buildings
                .factory_flags
                .get(*key)
                .is_some_and(|flags| flags.rocket())
                || buildings.ambiguous_factory_types.contains(*key)
        })
        .collect();
    let [key] = keys.as_slice() else {
        return Err(format!(
            "{} content building definitions can classify Rocket production; the source name is not known",
            keys.len()
        ));
    };
    if buildings.ambiguous_factory_types.contains(*key)
        || buildings.repeated_types.contains_key(*key)
    {
        return Err(format!(
            "{key}: repeated Rocket building definition is ambiguous"
        ));
    }
    Ok(((*key).clone(), texts.and_then(|texts| texts.text(key))))
}

type RocketName = Result<(String, Option<String>), String>;
#[derive(Default)]
struct RocketNameCache {
    content: Option<(Arc<crate::catalogue::Catalogue>, RocketName)>,
}

fn cached_rocket_name(source: &Source, catalogue: &Arc<crate::catalogue::Catalogue>) -> RocketName {
    let state = source.state::<RocketNameCache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if !cache
        .content
        .as_ref()
        .is_some_and(|(cached, _)| Arc::ptr_eq(cached, catalogue))
    {
        let name = installed_content_core::buildings::read(&catalogue.content.layers)
            .map_err(|error| error.to_string())
            .and_then(|buildings| rocket_name(&buildings, Some(&**catalogue)));
        cache.content = Some((Arc::clone(catalogue), name));
    }
    cache
        .content
        .as_ref()
        .expect("Rocket source name cached")
        .1
        .clone()
}

fn production_presence(
    equipment: Option<&TagHistory>,
    rocket: Option<&TagHistory>,
) -> Vec<ProductionPresenceRun> {
    let boundaries: Vec<_> = equipment
        .into_iter()
        .chain(rocket)
        .flat_map(|track| {
            track
                .hours
                .iter()
                .zip(&track.last_hours)
                .flat_map(|(first, last)| [i64::from(*first), i64::from(*last) + 1])
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    fn at(track: Option<&TagHistory>, hour: i32) -> Option<(&TagHistory, usize)> {
        let track = track?;
        let index = track
            .hours
            .partition_point(|read| *read <= hour)
            .checked_sub(1)?;
        (track.last_hours[index] >= hour).then_some((track, index))
    }
    let mut result: Vec<ProductionPresenceRun> = Vec::new();
    for pair in boundaries.windows(2) {
        let hour = pair[0] as i32;
        let equipment_at = at(equipment, hour);
        let presence = |sample: Option<(&TagHistory, usize)>| {
            sample.and_then(|(track, index)| track.presence.get(index).copied().flatten())
        };
        let present = match (presence(equipment_at), presence(at(rocket, hour))) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };
        let empty_census_hour = equipment_at.and_then(|(track, index)| {
            // False presence already requires a complete equipment census/output read.
            (present == Some(false)).then_some(track.hours[index])
        });
        let last = (pair[1] - 1) as i32;
        if let Some(previous) = result.last_mut()
            && previous.last_hour.checked_add(1) == Some(hour)
            && previous.present == present
            && previous.empty_census_hour == empty_census_hour
        {
            previous.last_hour = last;
        } else {
            result.push(ProductionPresenceRun {
                first_hour: hour,
                last_hour: last,
                present,
                empty_census_hour,
            });
        }
    }
    result
}

fn runs(
    track: &TagHistory,
    coverage: &[Coverage],
    values: &[(u32, CostTotal)],
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
            );
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

/// A leaf's input from lines whose purpose is not recorded, as runs over the
/// same coverage as its amounts; none when it never had any.
fn unproven_runs(
    track: &TagHistory,
    coverage: &[Coverage],
    values: &[(u32, CostTotal)],
    cancel: &AtomicBool,
) -> io::Result<Vec<ProductionSpendingRun>> {
    if values.iter().all(|(_, value)| value.unproven_raw == 0) {
        return Ok(Vec::new());
    }
    let unproven: Vec<_> = values
        .iter()
        .map(|&(sample, value)| {
            (
                sample,
                CostTotal {
                    raw: value.unproven_raw,
                    ..value
                },
            )
        })
        .collect();
    runs(track, coverage, &unproven, cancel)
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

type PreparedOperands<'a> = (
    &'a BTreeMap<u64, crate::prepared_production::Classification>,
    &'a BTreeMap<i32, Option<ProductionSpendingUnknown>>,
    &'a BTreeMap<i32, Option<ProductionSpendingUnknown>>,
);

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn reply(
    scanned: &ScannedProduced,
    params: &ProductionSpendingParams,
    named: Named,
    warnings: Vec<String>,
    cancel: &AtomicBool,
) -> io::Result<ProductionSpendingReply> {
    reply_inner(
        scanned,
        params,
        named,
        warnings,
        cancel,
        None,
        Purposes::counting(params.refits),
    )
}

pub fn reply_prepared(
    scanned: &ScannedProduced,
    params: &ProductionSpendingParams,
    operands: PreparedOperands<'_>,
    warnings: Vec<String>,
    cancel: &AtomicBool,
    purposes: Purposes,
) -> io::Result<ProductionSpendingReply> {
    reply_inner(
        scanned,
        params,
        Named::default(),
        warnings,
        cancel,
        Some(operands),
        purposes,
    )
}

fn reply_inner(
    scanned: &ScannedProduced,
    params: &ProductionSpendingParams,
    named: Named,
    mut warnings: Vec<String>,
    cancel: &AtomicBool,
    prepared: Option<PreparedOperands<'_>>,
    purposes: Purposes,
) -> io::Result<ProductionSpendingReply> {
    warnings.extend(scanned.variants.warnings());
    let track = scanned.tags.get(&params.tag);
    let entries: BTreeSet<Entry> = track
        .into_iter()
        .flat_map(|track| track.changes.keys().chain(track.cost_changes.keys()))
        .filter_map(|entry| purposes.leaf(*entry))
        .collect();
    let variants: BTreeSet<u64> = entries.iter().map(|(variant, _)| *variant).collect();
    let classify = |variants: Vec<u64>| {
        if let Some((classes, _, _)) = prepared {
            let types = variants
                .iter()
                .filter_map(|variant| {
                    classes.get(variant).map(|kind| {
                        (
                            *variant,
                            production::VariantType {
                                key: kind.key.clone(),
                                ordinal: kind.ordinal,
                                domain: kind.domain,
                            },
                        )
                    })
                })
                .collect();
            let reasons = variants
                .iter()
                .filter_map(|variant| classes.get(variant)?.reason.clone())
                .collect();
            let unresolved = variants
                .iter()
                .filter(|variant| {
                    classes
                        .get(variant)
                        .is_none_or(|kind| kind.ordinal.is_none())
                })
                .count();
            (types, reasons, unresolved)
        } else {
            production::variant_types_for(&scanned.variants, variants, named)
        }
    };
    let type_name = |variant, key: &str| {
        prepared
            .and_then(|(classes, _, _)| classes.get(&variant))
            .map_or_else(|| named.type_name(key), |kind| kind.name.clone())
    };
    let names = |variant| {
        prepared
            .and_then(|(classes, _, _)| classes.get(&variant))
            .map_or_else(
                || scanned.variants.names(variant, named.texts),
                |kind| kind.names.clone(),
            )
    };
    let scope = |track: &TagHistory, rocket: bool| {
        if let Some((_, equipment, rockets)) = prepared {
            let reasons = if rocket { rockets } else { equipment };
            track
                .hours
                .iter()
                .enumerate()
                .map(|(index, &first)| Coverage {
                    first,
                    last: track.last_hours[index],
                    reason: reasons.get(&first).copied().flatten(),
                })
                .collect()
        } else if rocket {
            rocket_coverage(scanned, track)
        } else {
            coverage(scanned, track)
        }
    };
    let (types, reasons, unresolved) = classify(variants.iter().copied().collect());
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
        warnings.push("A recording gap, missing spending suffix or missed output publication ends exact equipment-line IC; ended lines publish their final totals only once.".into());
    }
    if scanned.observations.len() > 1 {
        warnings.push("The output observation restarted; applied IC across the restart is not known, so the campaign total remains Not known.".into());
    }
    if scanned.unlisted > 0 {
        warnings.push(format!("{} lines produced in ticks whose purpose was not recorded; as Equipment produced does, they count as new production.", scanned.unlisted));
    }
    warnings
        .push("Instant progress is recorded separately and excluded from factory spending.".into());
    let mut leaves = Vec::new();
    let mut total = Vec::new();
    let mut size = Counter { bytes: 0, cancel };
    if let Some(track) = track {
        let coverage = scope(track, false);
        let (history, total_history) = histories(track, purposes)?;
        let mut entries: Vec<_> = entries.into_iter().collect();
        entries.sort_by_key(|entry| {
            types
                .get(&entry.0)
                .map(|kind| (kind.ordinal.unwrap_or(usize::MAX), kind.key.clone(), *entry))
        });
        for entry @ (variant, purpose) in entries {
            cancelled(cancel)?;
            let Some(kind) = types.get(&variant) else {
                continue;
            };
            let values = history.get(&entry).map(Vec::as_slice).unwrap_or(&[]);
            let leaf = ProductionSpendingLeaf {
                variant: variant.to_string(),
                type_key: kind.key.clone(),
                name: type_name(variant, &kind.key),
                domain: kind.domain,
                rocket: false,
                purpose: (purposes == Purposes::Split).then_some(purpose),
                unproven: unproven_runs(track, &coverage, values, cancel)?,
                names: names(variant),
                runs: runs(track, &coverage, values, cancel)?,
            };
            serde_json::to_writer(&mut size, &leaf)
                .map_err(|error| io::Error::other(error.to_string()))?;
            leaves.push(leaf);
        }
        total = runs(track, &coverage, &total_history, cancel)?;
    } else {
        warnings.push(format!(
            "{}: no readable country production history in this branch",
            params.tag
        ));
    }
    let mut rocket_total = Vec::new();
    if let Some(rocket) = scanned.rocket.tags.get(&params.tag) {
        let coverage = scope(rocket, true);
        let (history, totals) = histories(rocket, Purposes::Combined)?;
        let variants: Vec<_> = rocket
            .cost_changes
            .keys()
            .map(|entry| entry.0)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|variant| *variant != 0)
            .collect();
        let (types, reasons, _) = classify(variants.clone());
        warnings.extend(
            reasons
                .into_iter()
                .map(|reason| format!("A Rocket equipment type cannot be resolved: {reason}.")),
        );
        for variant in variants {
            cancelled(cancel)?;
            let kind = &types[&variant];
            let leaf = ProductionSpendingLeaf {
                variant: variant.to_string(),
                type_key: kind.key.clone(),
                name: type_name(variant, &kind.key),
                domain: kind.domain,
                rocket: true,
                purpose: None,
                unproven: Vec::new(),
                names: names(variant),
                runs: runs(
                    rocket,
                    &coverage,
                    history
                        .get(&(variant, Purpose::New))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    cancel,
                )?,
            };
            serde_json::to_writer(&mut size, &leaf)
                .map_err(|error| io::Error::other(error.to_string()))?;
            leaves.push(leaf);
        }
        rocket_total = runs(rocket, &coverage, &totals, cancel)?;
    }
    if scanned.rocket.reads.is_empty() {
        warnings.push(
            "Rocket production spending is not known: no independently readable Rocket coverage."
                .into(),
        );
    } else if scanned.rocket.unread > 0 {
        warnings.push(format!(
            "{} Rocket publications have no independent coverage; those hours are not known.",
            scanned.rocket.unread
        ));
    }
    if scanned.rocket.unattributed > 0 {
        warnings.push(format!("{} retained Rocket rows have no identified producing country; their amounts are not assigned to another country.", scanned.rocket.unattributed));
    }
    if scanned.rocket.inconsistent > 0 {
        warnings.push(format!("{} retained Rocket rows regressed counters or changed amounts without new events; the affected scope's measure stays unknown.", scanned.rocket.inconsistent));
    }
    // Only a reply split by purpose reads the equipment source on its own;
    // the others keep their budget for the combined total.
    let equipment_total = if purposes == Purposes::Split {
        total.clone()
    } else {
        Vec::new()
    };
    let total = combined(
        &total,
        &rocket_total,
        &scanned.observations,
        &scanned.rocket.observations,
    )?;
    let reads = scanned
        .reads
        .iter()
        .chain(&scanned.rocket.reads)
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let observations = scanned
        .observations
        .iter()
        .chain(&scanned.rocket.observations)
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let result = ProductionSpendingReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        decimals: EQUIPMENT_DECIMALS,
        reads,
        observations,
        equipment_observations: scanned.observations.clone(),
        rocket_observations: scanned.rocket.observations.clone(),
        equipment_reads: scanned.reads.clone(),
        rocket_reads: scanned.rocket.reads.clone(),
        rocket_key: None,
        rocket_name: None,
        leaves,
        total,
        equipment_total,
        rocket_total,
        production: production_presence(track, scanned.rocket.tags.get(&params.tag)),
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
            Purpose::decl(),
            ProductionSpendingRun::decl(),
            ProductionSpendingLeaf::decl(),
            ProductionPresenceRun::decl(),
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
    #[cfg(any(test, feature = "test-support"))]
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
        let mut reply = reply(
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
        )?;
        if let Some(catalogue) = catalogue {
            if reply.leaves.iter().any(|leaf| leaf.rocket) {
                match cached_rocket_name(source, &catalogue) {
                    Ok((key, name)) => {
                        reply.rocket_key = Some(key);
                        reply.rocket_name = name;
                    }
                    Err(reason) => reply.warnings.push(format!(
                        "Rocket production source name is unavailable: {reason}."
                    )),
                }
            }
            reply.warnings.extend(catalogue.refusal_lines());
        }
        serde_json::to_writer(Counter { bytes: 0, cancel }, &reply)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(reply)
    }
}

#[cfg(test)]
pub(crate) mod tests;
