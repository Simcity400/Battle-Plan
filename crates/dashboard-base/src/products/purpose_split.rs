//! What the spending widgets split by purpose share: keeping a reply's leaves
//! to one domain, and calendar-month amounts of one country's applied
//! equipment input per purpose, read from production's prepared spent series.
//! Dockyard spending and Military factory spending each keep the designs of
//! their domain and designs whose type content cannot resolve, which could be
//! in it.
//!
//! A month's amount is its end's cumulative input less the hour before its
//! start, inside one uninterrupted observation. Per design, new production is
//! the input without refits less its converted and unrecorded-purpose input,
//! refits are the whole input less the input without refits, and converted
//! and unrecorded-purpose input are their own series' change: every amount is
//! measured against the same baseline.

use crate::prepared::series::Run;
use crate::prepared_production::{self as pages, Classification, SERIES_PURPOSES};
use crate::products::equipment::{Domain, UNRESOLVED_MARK};
use crate::products::equipment::production::{ProductionSpendingUnknown, Purpose};
use crate::products::production_spending::prepared::Answered;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// The first hour of the engine-calendar month holding `hour`, and of the next.
pub fn month_of(hour: i32) -> (i32, i32) {
    let (start, next) = month_bounds(hour);
    let clamp = |value: i64| i32::try_from(value).unwrap_or(i32::MAX);
    (clamp(start), clamp(next))
}

/// [`month_of`], unclamped: a month can run past the last representable hour.
fn month_bounds(hour: i32) -> (i64, i64) {
    use engine_native::cdate::{HOUR_EPOCH, MONTH_LENGTHS};
    const YEAR: i64 = 365 * 24;
    let elapsed = i64::from(hour) - i64::from(HOUR_EPOCH);
    let year_start = i64::from(HOUR_EPOCH) + elapsed.div_euclid(YEAR) * YEAR;
    let mut start = year_start;
    for days in MONTH_LENGTHS {
        let next = start + i64::from(days) * 24;
        if i64::from(hour) < next {
            return (start, next);
        }
        start = next;
    }
    unreachable!("a year's months cover it")
}

/// The calendar months overlapping `first..=last`, each clipped to it.
pub fn months(first: i32, last: i32) -> Vec<(i32, i32)> {
    let mut result = Vec::new();
    let mut at = first;
    while at <= last {
        let (_, next) = month_bounds(at);
        let end = i32::try_from(next - 1).map_or(last, |end| end.min(last));
        result.push((at, end));
        if end == last {
            break;
        }
        at = end + 1;
    }
    result
}

/// Whether a domain showing `shown` has a part of its own for input of
/// `purpose`: one it shows, or one that was not recorded. Any other purpose
/// is its Other purposes remainder.
fn possible(shown: &[Purpose], purpose: Purpose) -> bool {
    purpose == Purpose::Unknown || shown.contains(&purpose)
}

/// Keeps a split reply's leaves to designs that could be in `domain` (its
/// own, or a type content cannot resolve), never Rocket's, with every
/// purpose: input of a purpose the domain never shows is still recorded as
/// its own, so no purpose rules an unresolved design out. Returns how many of
/// the domain's own leaves had a purpose it never shows, which the reply warns
/// about.
pub(crate) fn keep_domain(
    data: &mut Value,
    domain: Domain,
    shown: &[Purpose],
) -> io::Result<usize> {
    let leaves = data["leaves"]
        .as_array_mut()
        .ok_or_else(|| crate::invalid("Production spending leaves are not a list"))?;
    let mut other = 0;
    leaves.retain(|leaf| {
        let equipment = leaf["rocket"] != true;
        let own = equipment && leaf["domain"] == serde_json::json!(domain);
        let unresolved = equipment
            && leaf["typeKey"]
                .as_str()
                .is_some_and(|key| key.starts_with(UNRESOLVED_MARK));
        let shown = serde_json::from_value::<Purpose>(leaf["purpose"].clone())
            .is_ok_and(|purpose| possible(shown, purpose));
        other += usize::from(own && !shown);
        own || unresolved
    });
    Ok(other)
}

/// Per-purpose amounts, by [`Purpose::index`].
pub(crate) type Amounts = [i128; Purpose::ALL.len()];

/// A design's spent series through the reply's hour.
pub(crate) struct Design {
    pub unresolved: bool,
    /// Its input of every purpose but refits.
    pub without_refits: Vec<Run>,
    /// Its input of every purpose.
    pub combined: Vec<Run>,
    /// Its converted and unrecorded-purpose input, in [`SERIES_PURPOSES`]
    /// order; empty when it never had any.
    pub separate: [Vec<Run>; SERIES_PURPOSES.len()],
    /// The cumulative magnitudes of its unrecorded-purpose input; empty when
    /// it never had any.
    pub unproven: Vec<Run>,
}

fn covering(runs: &[Run], hour: i32) -> Option<&Run> {
    let index = runs.partition_point(|run| run.last_hour < hour);
    runs.get(index).filter(|run| run.first_hour <= hour)
}

/// Where a cumulative series' change over `first..=last` is measured from:
/// None before the design had any input, `Some(None)` from zero (its first
/// input in that span), `Some(Some(hour))` from its value at `hour`. Err when
/// an hour between cannot be told.
///
/// `runs` are the branch's: a design's first run is its first input since its
/// country's observation began. That run starts with a break, as every
/// series' first does; a restart or holder change is the month's coverage's
/// refusal ([`SourceCoverage`]). A later break is another holder's series.
fn baseline(
    runs: &[Run],
    first: i32,
    last: i32,
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
        if at == 0 {
            return Ok(Some(None));
        }
        if run.break_before {
            return Err(ProductionSpendingUnknown::CountryChanged);
        }
        if runs[at - 1].last_hour.checked_add(1) != Some(run.first_hour) {
            return Err(ProductionSpendingUnknown::SpendingUnavailable);
        }
        at -= 1;
    }
}

/// A cumulative series' value at a baseline, zero where it holds none: a
/// design's input of one purpose has no series until its first such input.
fn value(runs: &[Run], at: Option<i32>) -> i128 {
    at.and_then(|hour| covering(runs, hour))
        .map_or(0, |run| run.value)
}

/// One design's input of each purpose over `first..=last`.
pub(crate) fn design_change(
    design: &Design,
    first: i32,
    last: i32,
) -> Result<Amounts, ProductionSpendingUnknown> {
    let Some(from) = baseline(&design.combined, first, last)? else {
        return Ok([0; Purpose::ALL.len()]);
    };
    let overflow = || ProductionSpendingUnknown::SpendingUnavailable;
    let change = |runs: &[Run]| {
        value(runs, Some(last))
            .checked_sub(value(runs, from))
            .ok_or_else(overflow)
    };
    let combined = change(&design.combined)?;
    let without_refits = change(&design.without_refits)?;
    let mut amounts = [0; Purpose::ALL.len()];
    let mut new = without_refits;
    for (purpose, runs) in SERIES_PURPOSES.into_iter().zip(&design.separate) {
        let amount = change(runs)?;
        amounts[purpose.index()] = amount;
        new = new.checked_sub(amount).ok_or_else(overflow)?;
    }
    amounts[Purpose::New.index()] = new;
    amounts[Purpose::Refit.index()] = combined.checked_sub(without_refits).ok_or_else(overflow)?;
    Ok(amounts)
}

/// Whether input of production ticks whose purpose was not recorded joined
/// the design in `first..=last`, read against the same baseline as its amounts.
pub(crate) fn unproven_in(
    design: &Design,
    first: i32,
    last: i32,
) -> Result<bool, ProductionSpendingUnknown> {
    let Some(from) = baseline(&design.combined, first, last)? else {
        return Ok(false);
    };
    Ok(value(&design.unproven, Some(last)) != value(&design.unproven, from))
}

/// Whether one country's equipment spending is known through a month's last
/// hour: from the source's first read until the first taint of its
/// observation, its restart or a change of the country's holder, as
/// production spending's coverage. The prepared catalogue keeps only the
/// latest read and observation, so each month reads them from the endpoint
/// page holding its last hour.
pub(crate) struct SourceCoverage {
    /// The equipment source was read by then.
    read: bool,
    /// The first taint of the source's spending, and its hour.
    tainted: Option<(i32, ProductionSpendingUnknown)>,
    /// The source's observation restarted by then.
    restarted: bool,
    /// The country's own refusal then: a taint, restart or holder change.
    country: Option<ProductionSpendingUnknown>,
}

impl SourceCoverage {
    /// The coverage `page` gives `tag`: the endpoint page holding the month's
    /// last hour, None when no page holds it.
    pub(crate) fn at(
        page: Option<&pages::EndpointPage>,
        tag: &str,
        tainted: Option<(i32, ProductionSpendingUnknown)>,
    ) -> Self {
        let country = page
            .and_then(|page| {
                let index = page
                    .countries
                    .binary_search_by(|row| row.tag.as_str().cmp(tag))
                    .ok()?;
                page.countries[index].equipment.as_ref()
            })
            .and_then(|source| source.campaign_spending_reason);
        Self {
            read: page.is_some_and(|page| page.equipment_read.is_some()),
            tainted,
            restarted: page.is_some_and(|page| page.equipment_restart.is_some()),
            country,
        }
    }

    /// Whether the month ending at `last` lies where spending is known. A
    /// month the recording starts in counts from its first read.
    pub(crate) fn month(&self, last: i32) -> Result<(), ProductionSpendingUnknown> {
        if !self.read {
            return Err(ProductionSpendingUnknown::NotRecorded);
        }
        if let Some((_, reason)) = self.tainted.filter(|(at, _)| *at <= last) {
            return Err(reason);
        }
        if self.restarted {
            return Err(ProductionSpendingUnknown::ObservationRestarted);
        }
        self.country.map_or(Ok(()), Err)
    }
}

/// One calendar month of a domain's applied input.
pub(crate) struct PurposeMonth {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Each purpose's input of the domain's resolved designs, or why it is
    /// not known.
    pub amounts: Result<Amounts, ProductionSpendingUnknown>,
    /// Input of a production tick whose purpose was not recorded joined it.
    pub purpose_unproven: bool,
    /// A design whose type content cannot resolve spent IC this month, of
    /// any purpose, so the domain's amounts may lack it.
    pub unresolved: bool,
}

impl PurposeMonth {
    /// One purpose's input, or why it is not known.
    pub(crate) fn part(&self, purpose: Purpose) -> Result<i128, ProductionSpendingUnknown> {
        self.amounts.map(|amounts| amounts[purpose.index()])
    }

    /// The input of purposes a domain showing `shown` never has: a remainder
    /// counted in the whole, never folded into a shown purpose.
    pub(crate) fn other(&self, shown: &[Purpose]) -> Result<i128, ProductionSpendingUnknown> {
        let amounts = self.amounts?;
        Purpose::ALL
            .into_iter()
            .filter(|&purpose| !possible(shown, purpose))
            .try_fold(0_i128, |sum, purpose| sum.checked_add(amounts[purpose.index()]))
            .ok_or(ProductionSpendingUnknown::SpendingUnavailable)
    }
}

fn add(sums: Amounts, more: Amounts) -> Result<Amounts, ProductionSpendingUnknown> {
    let mut next = [0; Purpose::ALL.len()];
    for ((next, sum), more) in next.iter_mut().zip(sums).zip(more) {
        *next = sum
            .checked_add(more)
            .ok_or(ProductionSpendingUnknown::SpendingUnavailable)?;
    }
    Ok(next)
}

fn cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Spending by month request cancelled",
        ))
    } else {
        Ok(())
    }
}

/// The calendar months from the window's first hour through its hour, each
/// with the exact input of `domain`'s designs per purpose or the reason it is
/// unknown. Designs content cannot type mark a month with input of any
/// purpose: a domain counts every purpose of its own designs.
pub(crate) fn purpose_months(
    store: &store::Store,
    key: &store::SourceKey,
    tag: &str,
    answered: &Answered,
    domain: Domain,
    cancel: &AtomicBool,
) -> io::Result<Vec<PurposeMonth>> {
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
    let tainted = catalogue
        .spending
        .scanned(tag)
        .spending_tainted
        .values()
        .min_by_key(|(at, _)| *at)
        .copied();
    let classes: BTreeMap<u64, Classification> = catalogue.classifications.clone();
    /// A design's series keys: without refits, combined, each separate
    /// purpose's, and its unproven input.
    #[derive(Default)]
    struct Keys {
        spent: [Option<u64>; 2],
        separate: [Option<u64>; SERIES_PURPOSES.len()],
        unproven: Option<u64>,
    }
    let mut keys: BTreeMap<u64, Keys> = BTreeMap::new();
    for (token, &id) in &catalogue.spent_keys {
        let (owner, rocket, refits, variant): (String, bool, bool, u64) =
            serde_json::from_str(token)?;
        if owner == tag && !rocket {
            keys.entry(variant).or_default().spent[usize::from(refits)] = Some(id);
        }
    }
    for (token, &id) in &catalogue.purpose_keys {
        let (owner, variant, purpose): (String, u64, Purpose) = serde_json::from_str(token)?;
        let Some(index) = SERIES_PURPOSES.iter().position(|&known| known == purpose) else {
            return Err(crate::invalid("Prepared purpose series has another purpose"));
        };
        if owner == tag {
            keys.entry(variant).or_default().separate[index] = Some(id);
        }
    }
    for (token, &id) in &catalogue.unproven_keys {
        let (owner, variant, purpose): (String, u64, Purpose) = serde_json::from_str(token)?;
        if owner == tag && purpose == Purpose::Unknown {
            keys.entry(variant).or_default().unproven = Some(id);
        }
    }
    let reader = reads.reader()?;
    let mut designs = Vec::new();
    for (variant, keys) in keys {
        cancelled(cancel)?;
        let class = classes.get(&variant);
        let unresolved = class.is_none_or(|class| class.ordinal.is_none());
        if !unresolved && class.and_then(|class| class.domain) != Some(domain) {
            continue;
        }
        let Some(combined) = keys.spent[1] else {
            continue;
        };
        let mut read = |id: Option<u64>| match id {
            Some(id) => reader.runs(id, answered.bounds.0, last),
            None => Ok(Vec::new()),
        };
        let [converted, unknown] = keys.separate;
        designs.push(Design {
            unresolved,
            without_refits: read(keys.spent[0])?,
            combined: read(Some(combined))?,
            separate: [read(converted)?, read(unknown)?],
            unproven: read(keys.unproven)?,
        });
    }
    let mut result = Vec::new();
    for (month_first, month_last) in months(first, last) {
        cancelled(cancel)?;
        // A month no read covers, or after a taint, restart or holder change,
        // is not known, whatever its designs' series hold.
        let page = reads.page(month_last, pages::Leaves::Sources)?;
        let coverage = SourceCoverage::at(page.as_ref(), tag, tainted);
        let mut amounts = coverage.month(month_last).map(|()| [0_i128; Purpose::ALL.len()]);
        let mut unresolved = false;
        let mut purpose_unproven = false;
        for design in &designs {
            purpose_unproven |= unproven_in(design, month_first, month_last).unwrap_or(false);
            let change = design_change(design, month_first, month_last);
            if design.unresolved {
                unresolved |= change != Ok([0; Purpose::ALL.len()]);
                continue;
            }
            amounts = match (amounts, change) {
                (Ok(sums), Ok(more)) => add(sums, more),
                (Err(reason), _) | (_, Err(reason)) => Err(reason),
            };
        }
        result.push(PurposeMonth {
            first_hour: month_first,
            last_hour: month_last,
            amounts,
            purpose_unproven,
            unresolved,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
