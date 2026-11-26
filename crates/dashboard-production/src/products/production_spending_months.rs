use crate::products::production_spending::{
    self, Content, Domain, LeafKey, ProductionPurpose, ProductionSpendingUnknown, REPAIR_DENOMINATOR, RepairRead, SpanRead, UNRESOLVED_MARK,
    runs::{Amount, Piece, read_at},
};
use dashboard_base::Source;
use dashboard_base::catalogue::Catalogue;
use dashboard_base::products::{Country, LoadContext, Product, check_reply_budget, validate_country};
use dashboard_query::Query;
use replay_data::rows::OutputPurpose;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ProductionSpendingMonths;

const HOUR_EPOCH: i64 = 43_800_000;
const MONTH_START_DAYS: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const DECIMALS: u8 = 5;
const SCALE: f64 = 100_000.0;

const DOCKYARD_PURPOSE_NOTE: &str = "Whether some of it was refits was not recorded.";
const FACTORY_PURPOSE_NOTE: &str = "Whether some of it was converted was not recorded.";
const TYPE_NOTE: &str = "Some designs\u{2019} types are unknown.";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(crate) fn month_of_hour(hour: i32) -> Option<i32> {
    let elapsed = i64::from(hour) - HOUR_EPOCH;
    if elapsed < 0 {
        return None;
    }
    let days = elapsed / 24;
    let year = days / 365;
    let day = days - year * 365;
    let month = MONTH_START_DAYS.partition_point(|&start| start <= day) - 1;
    i32::try_from(year * 12 + month as i64).ok()
}

pub(crate) fn first_hour_of_month(month: i32) -> i32 {
    let m = i64::from(month);
    let year = m.div_euclid(12);
    let idx = m.rem_euclid(12) as usize;
    (HOUR_EPOCH + (year * 365 + MONTH_START_DAYS[idx]) * 24) as i32
}

pub(crate) fn year_month(month: i32) -> (i32, u8) {
    let m = i64::from(month);
    (m.div_euclid(12) as i32, (m.rem_euclid(12) + 1) as u8)
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionSpendingMonthsParams {
    pub branch_id: String,
    pub country: Country,
    pub domain: Domain,
    pub repairs: bool,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendingMonthBandSpec {
    pub id: String,
    pub label: String,
    pub remainder: bool,
    pub denominator: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendingMonthPart {
    pub raw: Option<String>,
    pub value: Option<f64>,
    pub reason: Option<String>,
    pub uncertain: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SpendingMonthBar {
    pub year: i32,
    pub month: u8,
    pub first_hour: i32,
    pub last_hour: i32,
    pub month_first_hour: i32,
    pub month_last_hour: i32,
    pub incomplete: bool,
    pub parts: Vec<SpendingMonthPart>,
    pub total_raw: Option<String>,
    pub total_denominator: u32,
    pub total_value: Option<f64>,
    pub total_reason: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionSpendingMonthsReply {
    pub branch_id: String,
    pub tag: String,
    pub domain: Domain,
    pub decimals: u8,
    pub recorded: bool,
    pub bands: Vec<SpendingMonthBandSpec>,
    pub months: Vec<SpendingMonthBar>,
    pub next_change_hour: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ProductionSpendingMonthsReply);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CachedRun {
    pub(crate) first_hour: i32,
    pub(crate) last_hour: i32,
    pub(crate) amount: Amount,
    pub(crate) break_before: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LeafClassification {
    pub(crate) first_hour: i32,
    pub(crate) last_hour: i32,
    pub(crate) domain: Option<Domain>,
    pub(crate) unresolved: bool,
    pub(crate) classified_from: Option<i32>,
}

#[derive(Clone, Debug)]
pub(crate) struct CachedLeaf {
    pub(crate) classifications: Vec<LeafClassification>,
    pub(crate) purpose: ProductionPurpose,
    pub(crate) runs: Vec<CachedRun>,
    pub(crate) unproven: Vec<CachedRun>,
}

impl CachedLeaf {
    fn classification_at(&self, hour: i32) -> (Option<Domain>, bool) {
        let found = self
            .classifications
            .iter()
            .rev()
            .find(|entry| entry.first_hour <= hour && hour <= entry.last_hour)
            .or_else(|| self.classifications.last());
        let Some(entry) = found else {
            return (None, true);
        };
        let unresolved = entry.unresolved || entry.domain.is_none() || entry.classified_from.is_some_and(|at| hour < at);
        (entry.domain, unresolved)
    }

    fn touches_domain(&self, domain: Domain) -> bool {
        self.classifications
            .iter()
            .any(|entry| entry.domain == Some(domain) || entry.unresolved || entry.domain.is_none() || entry.classified_from.is_some())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BranchSpending {
    pub(crate) branch_first: i32,
    pub(crate) branch_last: i32,
    pub(crate) tag: String,
    pub(crate) equipment_observations: Vec<i32>,
    pub(crate) equipment_coverage: Vec<CachedRun>,
    pub(crate) equipment_total: Vec<CachedRun>,
    pub(crate) leaves: Vec<CachedLeaf>,
    pub(crate) repairs_total: Option<Vec<CachedRun>>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Default)]
struct SpendingMonthsCache {
    entries: HashMap<(u64, String, bool), (Option<Arc<Catalogue>>, Arc<BranchSpending>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MonthUnknown {
    Spending(ProductionSpendingUnknown),
    OutsideRead,
    ObservationChanged,
    Unresolved,
}

fn unknown_label(why: MonthUnknown) -> &'static str {
    match why {
        MonthUnknown::Spending(ProductionSpendingUnknown::NotRecorded) => "Spending not recorded.",
        MonthUnknown::Spending(ProductionSpendingUnknown::SpendingUnavailable) => "Spending unavailable at this time.",
        MonthUnknown::Spending(ProductionSpendingUnknown::LostOutput) => "Final total missing.",
        MonthUnknown::Spending(ProductionSpendingUnknown::MissedPublication) => "Spending unavailable at this time.",
        MonthUnknown::Spending(ProductionSpendingUnknown::RecordingGap) => "Hours were not recorded.",
        MonthUnknown::Spending(ProductionSpendingUnknown::ObservationRestarted) => "Campaign total unavailable.",
        MonthUnknown::Spending(ProductionSpendingUnknown::ObservationMismatch) => "Production totals start on different dates.",
        MonthUnknown::Spending(ProductionSpendingUnknown::CountryChanged) => "Country changed during this period.",
        MonthUnknown::Spending(ProductionSpendingUnknown::RocketTableUnavailable) => "Rocket spending not recorded.",
        MonthUnknown::Spending(ProductionSpendingUnknown::RocketInputInexact) => "Rocket spending total unknown.",
        MonthUnknown::Spending(ProductionSpendingUnknown::RocketOwnerUnreadable) => "Rocket producer unknown.",
        MonthUnknown::Spending(ProductionSpendingUnknown::RocketDesignUnreadable) => "Rocket design unknown.",
        MonthUnknown::OutsideRead => "Spending unavailable at this time.",
        MonthUnknown::ObservationChanged => "Campaign total unavailable.",
        MonthUnknown::Unresolved => TYPE_NOTE,
    }
}

fn append_cached(runs: &mut Vec<CachedRun>, pieces: &[Piece<Amount>], breaks: bool) {
    for (index, &(first_hour, last_hour, amount)) in pieces.iter().enumerate() {
        let break_before = index == 0 && breaks;
        match runs.last_mut() {
            Some(open) if !break_before && open.last_hour.checked_add(1) == Some(first_hour) && open.amount == amount => {
                open.last_hour = last_hour;
            }
            _ => runs.push(CachedRun { first_hour, last_hour, amount, break_before }),
        }
    }
}

fn cached_runs_of(spans: &[SpanRead], mut each: impl FnMut(&SpanRead) -> Vec<Piece<Amount>>) -> Vec<CachedRun> {
    let mut out = Vec::new();
    for span in spans {
        append_cached(&mut out, &each(span), span.span.breaks);
    }
    out
}

fn build_branch(
    source: &Source,
    query: &Query<'_>,
    branch: u64,
    country: &Country,
    repairs: bool,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Arc<BranchSpending>> {
    let mut warnings = Vec::new();
    let content = Content::read(source, context, &mut warnings);
    let key = (branch, format!("{country:?}"), repairs);
    let state = source.state::<SpendingMonthsCache>();
    {
        let held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached_cat, entry)) = held.entries.get(&key) {
            let same_cat = match (cached_cat, &content.catalogue) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if same_cat {
                return Ok(Arc::clone(entry));
            }
        }
    }

    let mut read = Vec::new();
    for span in production_spending::spans(query, country) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        read.push(SpanRead::read(query, span, repairs)?);
    }
    let mut notes = BTreeSet::new();
    let mut named = Vec::with_capacity(read.len());
    for span in &read {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let variants: BTreeMap<u64, u32> = span
            .rows
            .keys()
            .filter(|&&(rocket, _, _)| !rocket)
            .map(|&(_, variant, _)| (variant, span.archetypes.get(&variant).copied().unwrap_or(0)))
            .collect();
        named.push(production_spending::designs(query, &span.span, &variants, &content, &mut notes)?);
    }
    warnings.extend(notes);

    let equipment_coverage = cached_runs_of(&read, SpanRead::equipment_coverage);
    let equipment_total = cached_runs_of(&read, |span| span.equipment(true));
    let equipment_observations: Vec<i32> = read
        .iter()
        .flat_map(|span| span.equipment_observations.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let keys: BTreeSet<LeafKey> = read
        .iter()
        .flat_map(|span| span.rows.keys().copied())
        .filter(|&(rocket, _, _)| !rocket)
        .collect();
    let leaves: Vec<CachedLeaf> = keys
        .into_iter()
        .map(|leaf_key| {
            let (_, variant, purpose_raw) = leaf_key;
            let mut classifications = Vec::new();
            for (span, span_named) in read.iter().zip(&named) {
                if !span.rows.contains_key(&leaf_key) {
                    continue;
                }
                let (domain, unresolved, classified_from) = match span_named.get(&variant) {
                    Some(found) => (
                        found.domain,
                        found.type_key.starts_with(UNRESOLVED_MARK),
                        found
                            .domain
                            .and_then(|_| span.archetype_since.get(&variant).copied().into_iter().chain(found.typed_since).min()),
                    ),
                    None => (None, true, None),
                };
                classifications.push(LeafClassification {
                    first_hour: span.span.first,
                    last_hour: span.span.last,
                    domain,
                    unresolved,
                    classified_from,
                });
            }
            let purpose = purpose_raw.map_or(ProductionPurpose::Unknown, ProductionPurpose::of);
            let runs = cached_runs_of(&read, |span| span.amounts(&leaf_key, |row| row.spent));
            let unproven = if purpose_raw == Some(OutputPurpose::Unknown.raw()) {
                let candidate = cached_runs_of(&read, |span| span.amounts(&leaf_key, |row| row.unproven));
                if candidate.iter().any(|run| matches!(run.amount, Amount::Known(v) if v != 0)) {
                    candidate
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            CachedLeaf { classifications, purpose, runs, unproven }
        })
        .collect();

    let repairs_total = repairs.then(|| {
        cached_runs_of(&read, |span| {
            span.repairs
                .as_ref()
                .map_or_else(|| vec![(span.span.first, span.span.last, Amount::Unknown(ProductionSpendingUnknown::NotRecorded))], RepairRead::total)
        })
    });

    let built = Arc::new(BranchSpending {
        branch_first: query.timeline.first,
        branch_last: query.timeline.last,
        tag: query.holders.name(country, query.timeline.last),
        equipment_observations,
        equipment_coverage,
        equipment_total,
        leaves,
        repairs_total,
        warnings,
    });
    let mut held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    held.entries.insert(key, (content.catalogue.clone(), Arc::clone(&built)));
    Ok(built)
}

fn run_index(runs: &[CachedRun], hour: i32) -> Option<usize> {
    let at = runs.partition_point(|run| run.last_hour < hour);
    runs.get(at).filter(|run| run.first_hour <= hour && hour <= run.last_hour).map(|_| at)
}

fn same_stretch(runs: &[CachedRun], a_idx: usize, b_idx: usize) -> bool {
    if a_idx > b_idx {
        return false;
    }
    for i in (a_idx + 1)..=b_idx {
        if runs[i - 1].last_hour.checked_add(1) != Some(runs[i].first_hour) || runs[i].break_before {
            return false;
        }
    }
    true
}

fn amount_in(runs: &[CachedRun], from: Option<i32>, hour: i32, observations: &[i32]) -> Result<i128, MonthUnknown> {
    let Some(b_idx) = run_index(runs, hour) else {
        return Err(MonthUnknown::OutsideRead);
    };
    let end_val = match runs[b_idx].amount {
        Amount::Known(v) => v,
        Amount::Unknown(why) => return Err(MonthUnknown::Spending(why)),
    };
    let Some(from) = from else {
        return Ok(end_val);
    };
    let obs = read_at(observations, hour);
    if obs.is_some_and(|o| from < o) {
        return Err(MonthUnknown::ObservationChanged);
    }
    if obs == Some(from) {
        return Ok(end_val);
    }
    let Some(prev_hour) = from.checked_sub(1) else {
        return Err(MonthUnknown::OutsideRead);
    };
    let Some(a_idx) = run_index(runs, prev_hour) else {
        return Err(MonthUnknown::OutsideRead);
    };
    let start_val = match runs[a_idx].amount {
        Amount::Known(v) => v,
        Amount::Unknown(why) => return Err(MonthUnknown::Spending(why)),
    };
    if !same_stretch(runs, a_idx, b_idx) {
        return Err(MonthUnknown::OutsideRead);
    }
    Ok(end_val - start_val)
}

fn rate_sum(runs: &[CachedRun], first: i32, last: i32) -> Result<i128, MonthUnknown> {
    if last < first {
        return Ok(0);
    }
    let Some(a_idx) = run_index(runs, first) else {
        return Err(MonthUnknown::OutsideRead);
    };
    let Some(b_idx) = run_index(runs, last) else {
        return Err(MonthUnknown::OutsideRead);
    };
    let mut sum = 0i128;
    for i in a_idx..=b_idx {
        if i > a_idx && runs[i - 1].last_hour.checked_add(1) != Some(runs[i].first_hour) {
            return Err(MonthUnknown::OutsideRead);
        }
        match runs[i].amount {
            Amount::Known(rate) => {
                let lo = runs[i].first_hour.max(first);
                let hi = runs[i].last_hour.min(last);
                sum = sum.saturating_add(rate.saturating_mul(i128::from(hi - lo + 1)));
            }
            Amount::Unknown(why) => return Err(MonthUnknown::Spending(why)),
        }
    }
    Ok(sum)
}

fn next_run_boundary(runs: &[CachedRun], after: i32) -> Option<i32> {
    let at = runs.partition_point(|run| run.last_hour < after);
    match runs.get(at) {
        Some(run) if run.first_hour <= after && after <= run.last_hour => run.last_hour.checked_add(1).filter(|&h| h > after),
        Some(run) => Some(run.first_hour),
        None => None,
    }
}

fn next_repair_change(runs: &[CachedRun], after: i32) -> Option<i32> {
    let next_hour = after.checked_add(1)?;
    if let Some(idx) = run_index(runs, next_hour)
        && matches!(runs[idx].amount, Amount::Known(v) if v != 0)
    {
        return Some(next_hour);
    }
    next_run_boundary(runs, after)
}

fn equipment_recorded_through(runs: &[CachedRun], until: i32) -> bool {
    runs.iter().take_while(|run| run.first_hour <= until).any(|run| match run.amount {
        Amount::Known(_) => true,
        Amount::Unknown(ProductionSpendingUnknown::NotRecorded | ProductionSpendingUnknown::RocketTableUnavailable) => false,
        Amount::Unknown(_) => true,
    })
}

fn repairs_recorded_through(runs: &[CachedRun], until: i32) -> bool {
    runs.iter().take_while(|run| run.first_hour <= until).any(|run| matches!(run.amount, Amount::Known(_)))
}

fn part_of(res: Result<i128, MonthUnknown>, denominator: u32, uncertain: bool) -> SpendingMonthPart {
    match res {
        Ok(raw) => SpendingMonthPart {
            raw: Some(raw.to_string()),
            value: Some(raw as f64 / (f64::from(denominator) * SCALE)),
            reason: None,
            uncertain,
        },
        Err(why) => SpendingMonthPart {
            raw: None,
            value: None,
            reason: Some(unknown_label(why).to_owned()),
            uncertain,
        },
    }
}

pub(crate) fn compute_months(
    branch: &BranchSpending,
    branch_id: &str,
    domain: Domain,
    repairs: bool,
    req_first: i32,
    req_last: i32,
) -> ProductionSpendingMonthsReply {
    let first = req_first.max(branch.branch_first);
    let last = req_last.min(branch.branch_last);
    let use_repairs = repairs && domain == Domain::Naval;

    let bands: Vec<SpendingMonthBandSpec> = if domain == Domain::Naval {
        vec![
            SpendingMonthBandSpec { id: "new".into(), label: "New ships".into(), remainder: false, denominator: 1 },
            SpendingMonthBandSpec { id: "refit".into(), label: "Refits".into(), remainder: false, denominator: 1 },
            SpendingMonthBandSpec { id: "repair".into(), label: "Repairs".into(), remainder: false, denominator: REPAIR_DENOMINATOR },
            SpendingMonthBandSpec { id: "purpose-unrecorded".into(), label: "Purpose not recorded".into(), remainder: true, denominator: 1 },
            SpendingMonthBandSpec { id: "other-purpose".into(), label: "Other purposes".into(), remainder: true, denominator: 1 },
        ]
    } else {
        vec![
            SpendingMonthBandSpec { id: "new".into(), label: "New".into(), remainder: false, denominator: 1 },
            SpendingMonthBandSpec { id: "converted".into(), label: "Converted".into(), remainder: false, denominator: 1 },
            SpendingMonthBandSpec { id: "purpose-unrecorded".into(), label: "Purpose not recorded".into(), remainder: true, denominator: 1 },
            SpendingMonthBandSpec { id: "other-purpose".into(), label: "Other purposes".into(), remainder: true, denominator: 1 },
        ]
    };

    let relevant_leaves: Vec<&CachedLeaf> = branch
        .leaves
        .iter()
        .filter(|leaf| leaf.touches_domain(domain))
        .collect();

    let recorded = equipment_recorded_through(&branch.equipment_coverage, last)
        || equipment_recorded_through(&branch.equipment_total, last)
        || (use_repairs && branch.repairs_total.as_deref().is_some_and(|runs| repairs_recorded_through(runs, last)));

    let mut months = Vec::new();
    if first <= last
        && let (Some(first_m), Some(last_m)) = (month_of_hour(first), month_of_hour(last))
    {
        for m in first_m..=last_m {
            let month_first_hour = first_hour_of_month(m);
            let month_last_hour = first_hour_of_month(m + 1) - 1;
            let m_first = month_first_hour.max(first);
            let m_last = month_last_hour.min(last);
            if m_first > m_last {
                continue;
            }
            let (year, month) = year_month(m);
            let incomplete = m_last < month_last_hour
                || (m_first > month_first_hour && !(m_first == branch.branch_first && m_first - month_first_hour < 24));
            let from = (m_first > branch.branch_first).then_some(m_first);
            let coverage = amount_in(&branch.equipment_coverage, from, m_last, &branch.equipment_observations);
            let mut slot_reasons = [coverage.err(); 4];
            let mut sums = [0i128; 4];
            let mut purpose_unproven = false;
            let mut unresolved = false;
            for leaf in &relevant_leaves {
                let (leaf_domain, is_unresolved) = leaf.classification_at(m_last);
                if !is_unresolved && leaf_domain != Some(domain) {
                    continue;
                }
                if !leaf.unproven.is_empty() && amount_in(&leaf.unproven, from, m_last, &branch.equipment_observations) != Ok(0) {
                    purpose_unproven = true;
                }
                let change = amount_in(&leaf.runs, from, m_last, &branch.equipment_observations);
                if is_unresolved {
                    if change != Ok(0) {
                        unresolved = true;
                    }
                    continue;
                }
                let slot = match leaf.purpose {
                    ProductionPurpose::New => 0,
                    ProductionPurpose::Refit => 1,
                    ProductionPurpose::Converted => 2,
                    ProductionPurpose::Unknown => 3,
                };
                match change {
                    Ok(v) => sums[slot] += v,
                    Err(why) => {
                        slot_reasons[slot].get_or_insert(why);
                    }
                }
            }
            let spent_of = |slot: usize| -> Result<i128, MonthUnknown> {
                match slot_reasons[slot] {
                    Some(why) => Err(why),
                    None => Ok(sums[slot]),
                }
            };
            let purposes_uncertain = purpose_unproven || unresolved;
            let (parts, total_denom, total_res, note) = if domain == Domain::Naval {
                let repair_res = if use_repairs {
                    branch
                        .repairs_total
                        .as_deref()
                        .map_or(Err(MonthUnknown::Spending(ProductionSpendingUnknown::NotRecorded)), |runs| rate_sum(runs, m_first, m_last))
                } else {
                    Err(MonthUnknown::Spending(ProductionSpendingUnknown::NotRecorded))
                };
                let band_results = [spent_of(0), spent_of(1), repair_res, spent_of(3), spent_of(2)];
                let parts = vec![
                    part_of(band_results[0], 1, purposes_uncertain),
                    part_of(band_results[1], 1, purposes_uncertain),
                    part_of(band_results[2], REPAIR_DENOMINATOR, false),
                    part_of(band_results[3], 1, purpose_unproven),
                    part_of(band_results[4], 1, false),
                ];
                let total_res = if unresolved {
                    Err(MonthUnknown::Unresolved)
                } else {
                    let denom = i128::from(REPAIR_DENOMINATOR);
                    band_results[0]
                        .and_then(|a| band_results[1].map(|b| a + b))
                        .and_then(|ab| band_results[3].map(|u| ab + u))
                        .and_then(|abu| band_results[4].map(|c| (abu + c) * denom))
                        .and_then(|eq_scaled| band_results[2].map(|rep| eq_scaled + rep))
                };
                let note = if purpose_unproven {
                    Some(DOCKYARD_PURPOSE_NOTE.to_owned())
                } else if unresolved {
                    Some(TYPE_NOTE.to_owned())
                } else {
                    None
                };
                (parts, REPAIR_DENOMINATOR, total_res, note)
            } else {
                let band_results = [spent_of(0), spent_of(2), spent_of(3), spent_of(1)];
                let parts = vec![
                    part_of(band_results[0], 1, purposes_uncertain),
                    part_of(band_results[1], 1, purposes_uncertain),
                    part_of(band_results[2], 1, purpose_unproven),
                    part_of(band_results[3], 1, false),
                ];
                let total_res = if unresolved {
                    Err(MonthUnknown::Unresolved)
                } else {
                    band_results[0]
                        .and_then(|a| band_results[1].map(|b| a + b))
                        .and_then(|ab| band_results[2].map(|u| ab + u))
                        .and_then(|abu| band_results[3].map(|r| abu + r))
                };
                let note = if purpose_unproven {
                    Some(FACTORY_PURPOSE_NOTE.to_owned())
                } else if unresolved {
                    Some(TYPE_NOTE.to_owned())
                } else {
                    None
                };
                (parts, 1, total_res, note)
            };
            let (total_raw, total_value, total_reason) = match total_res {
                Ok(raw) => (Some(raw.to_string()), Some(raw as f64 / (f64::from(total_denom) * SCALE)), None),
                Err(why) => (None, None, Some(unknown_label(why).to_owned())),
            };
            months.push(SpendingMonthBar {
                year,
                month,
                first_hour: m_first,
                last_hour: m_last,
                month_first_hour,
                month_last_hour,
                incomplete,
                parts,
                total_raw,
                total_denominator: total_denom,
                total_value,
                total_reason,
                note,
            });
        }
    }

    let mut next_change_hour: Option<i32> = None;
    let mut consider = |candidate: Option<i32>| {
        if let Some(h) = candidate
            && h > last
            && h <= branch.branch_last
        {
            next_change_hour = Some(next_change_hour.map_or(h, |cur| cur.min(h)));
        }
    };
    if last < branch.branch_last {
        if let Some(cur_m) = month_of_hour(last) {
            consider(Some(first_hour_of_month(cur_m + 1)));
        }
        consider(next_run_boundary(&branch.equipment_coverage, last));
        consider(next_run_boundary(&branch.equipment_total, last));
        for leaf in &relevant_leaves {
            consider(next_run_boundary(&leaf.runs, last));
            for entry in &leaf.classifications {
                consider(entry.classified_from);
            }
            if !leaf.unproven.is_empty() {
                consider(next_run_boundary(&leaf.unproven, last));
            }
        }
        if use_repairs && let Some(rep_runs) = &branch.repairs_total {
            consider(next_repair_change(rep_runs, last));
        }
    }

    ProductionSpendingMonthsReply {
        branch_id: branch_id.to_owned(),
        tag: branch.tag.clone(),
        domain,
        decimals: DECIMALS,
        recorded,
        bands,
        months,
        next_change_hour,
        warnings: branch.warnings.clone(),
    }
}

impl Product for ProductionSpendingMonths {
    const KEY: &'static str = "production-spending-months";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ProductionSpendingMonthsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, domain, repairs, firstHour: Start, lastHour: End];
    type Reply = ProductionSpendingMonthsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.first_hour > params.last_hour {
            return Err("Invalid production spending time range");
        }
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ProductionSpendingMonthsParams::decl(),
            SpendingMonthBandSpec::decl(),
            SpendingMonthPart::decl(),
            SpendingMonthBar::decl(),
            ProductionSpendingMonthsReply::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let held = build_branch(source, &query, branch, &params.country, params.repairs, context, cancelled)?;
        let reply = compute_months(&held, &params.branch_id, params.domain, params.repairs, params.first_hour, params.last_hour);
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(first_hour: i32, last_hour: i32, amount: Amount) -> CachedRun {
        CachedRun { first_hour, last_hour, amount, break_before: false }
    }

    fn classified(first_hour: i32, last_hour: i32, domain: Option<Domain>, classified_from: Option<i32>) -> Vec<LeafClassification> {
        vec![LeafClassification { first_hour, last_hour, domain, unresolved: domain.is_none(), classified_from }]
    }

    #[test]
    fn bins_dockyard_months_reports_incomplete_partial_missing_and_next_change() {
        let jan1 = first_hour_of_month(1936 * 12);
        let feb1 = first_hour_of_month(1936 * 12 + 1);
        let mar1 = first_hour_of_month(1936 * 12 + 2);
        let mar15 = mar1 + 14 * 24;
        let mar31 = first_hour_of_month(1936 * 12 + 3) - 1;

        let branch = BranchSpending {
            branch_first: jan1,
            branch_last: mar31,
            tag: "GER".into(),
            equipment_observations: vec![jan1],
            equipment_coverage: vec![
                run(jan1, feb1 - 1, Amount::Known(0)),
                run(feb1, mar1 - 1, Amount::Unknown(ProductionSpendingUnknown::SpendingUnavailable)),
                run(mar1, mar31, Amount::Known(0)),
            ],
            equipment_total: vec![
                run(jan1, feb1 - 1, Amount::Known(3_000_000)),
                run(feb1, mar1 - 1, Amount::Unknown(ProductionSpendingUnknown::SpendingUnavailable)),
                run(mar1, mar31, Amount::Known(5_000_000)),
            ],
            leaves: vec![
                CachedLeaf {
                    classifications: classified(jan1, mar31, Some(Domain::Naval), None),
                    purpose: ProductionPurpose::New,
                    runs: vec![
                        run(jan1, feb1 - 1, Amount::Known(2_000_000)),
                        run(feb1, mar1 - 1, Amount::Unknown(ProductionSpendingUnknown::SpendingUnavailable)),
                        run(mar1, mar31, Amount::Known(3_500_000)),
                    ],
                    unproven: Vec::new(),
                },
                CachedLeaf {
                    classifications: classified(jan1, mar31, Some(Domain::Naval), None),
                    purpose: ProductionPurpose::Unknown,
                    runs: vec![
                        run(jan1, feb1 - 1, Amount::Known(1_000_000)),
                        run(feb1, mar1 - 1, Amount::Unknown(ProductionSpendingUnknown::SpendingUnavailable)),
                        run(mar1, mar31, Amount::Known(1_500_000)),
                    ],
                    unproven: Vec::new(),
                },
            ],
            repairs_total: Some(vec![
                run(jan1, feb1 - 1, Amount::Known(0)),
                run(feb1, mar1 - 1, Amount::Known(240_000)),
                run(mar1, mar31, Amount::Known(0)),
            ]),
            warnings: Vec::new(),
        };

        let reply = compute_months(&branch, "1", Domain::Naval, true, jan1, mar15);
        assert!(reply.recorded);
        assert_eq!(reply.months.len(), 3);

        let jan = &reply.months[0];
        assert!(!jan.incomplete);
        assert_eq!(jan.parts[0].raw.as_deref(), Some("2000000"));
        assert_eq!(jan.parts[2].raw.as_deref(), Some("0"));
        assert_eq!(jan.parts[3].raw.as_deref(), Some("1000000"));
        assert_eq!(jan.total_raw.as_deref(), Some("72000000"));
        assert_eq!(jan.total_denominator, 24);

        let feb = &reply.months[1];
        assert!(!feb.incomplete);
        assert!(feb.parts[0].raw.is_none());
        assert_eq!(feb.parts[0].reason.as_deref(), Some("Spending unavailable at this time."));
        let expected_feb_repairs = (28 * 24 * 240_000i64).to_string();
        assert_eq!(feb.parts[2].raw.as_deref(), Some(expected_feb_repairs.as_str()));
        assert!(feb.total_raw.is_none());

        let mar = &reply.months[2];
        assert!(mar.incomplete);
        assert_eq!(mar.last_hour, mar15);
        assert!(mar.parts[0].raw.is_none());

        assert_eq!(reply.next_change_hour, None);

        let mid_feb = feb1 + 5 * 24;
        let feb_reply = compute_months(&branch, "1", Domain::Naval, true, jan1, mid_feb);
        assert_eq!(feb_reply.next_change_hour, Some(mid_feb + 1));
    }

    #[test]
    fn bins_military_factory_air_months_without_repairs() {
        let jan1 = first_hour_of_month(1936 * 12);
        let feb1 = first_hour_of_month(1936 * 12 + 1);
        let feb28 = first_hour_of_month(1936 * 12 + 2) - 1;

        let branch = BranchSpending {
            branch_first: jan1,
            branch_last: feb28,
            tag: "GER".into(),
            equipment_observations: vec![jan1],
            equipment_coverage: vec![run(jan1, feb28, Amount::Known(0))],
            equipment_total: vec![
                run(jan1, feb1 - 1, Amount::Known(1_000_000)),
                run(feb1, feb28, Amount::Known(2_500_000)),
            ],
            leaves: vec![
                CachedLeaf {
                    classifications: classified(jan1, feb28, Some(Domain::Air), None),
                    purpose: ProductionPurpose::New,
                    runs: vec![
                        run(jan1, feb1 - 1, Amount::Known(1_000_000)),
                        run(feb1, feb28, Amount::Known(2_000_000)),
                    ],
                    unproven: Vec::new(),
                },
                CachedLeaf {
                    classifications: classified(jan1, feb28, Some(Domain::Air), None),
                    purpose: ProductionPurpose::Converted,
                    runs: vec![
                        run(jan1, feb1 - 1, Amount::Known(0)),
                        run(feb1, feb28, Amount::Known(500_000)),
                    ],
                    unproven: Vec::new(),
                },
            ],
            repairs_total: None,
            warnings: Vec::new(),
        };

        let reply = compute_months(&branch, "1", Domain::Air, false, jan1, feb28);
        assert!(reply.recorded);
        assert_eq!(reply.bands.len(), 4);
        assert_eq!(reply.months.len(), 2);
        assert_eq!(reply.months[0].parts[0].raw.as_deref(), Some("1000000"));
        assert_eq!(reply.months[0].parts[1].raw.as_deref(), Some("0"));
        assert_eq!(reply.months[1].parts[0].raw.as_deref(), Some("1000000"));
        assert_eq!(reply.months[1].parts[1].raw.as_deref(), Some("500000"));
        assert_eq!(reply.months[1].total_raw.as_deref(), Some("1500000"));
        assert_eq!(reply.months[1].total_denominator, 1);
    }

    #[test]
    fn unknown_purpose_leaf_keeps_other_known_purposes_and_withholds_total() {
        let jan1 = first_hour_of_month(1936 * 12);
        let feb1 = first_hour_of_month(1936 * 12 + 1);
        let feb28 = first_hour_of_month(1936 * 12 + 2) - 1;
        let branch = BranchSpending {
            branch_first: jan1,
            branch_last: feb28,
            tag: "GER".into(),
            equipment_observations: vec![jan1],
            equipment_coverage: vec![run(jan1, feb28, Amount::Known(0))],
            equipment_total: vec![
                run(jan1, feb1 - 1, Amount::Unknown(ProductionSpendingUnknown::SpendingUnavailable)),
                run(feb1, feb28, Amount::Known(2_000_000)),
            ],
            leaves: vec![
                CachedLeaf {
                    classifications: classified(jan1, feb28, Some(Domain::Naval), None),
                    purpose: ProductionPurpose::New,
                    runs: vec![run(jan1, feb28, Amount::Known(1_500_000))],
                    unproven: Vec::new(),
                },
                CachedLeaf {
                    classifications: classified(jan1, feb28, Some(Domain::Naval), None),
                    purpose: ProductionPurpose::Refit,
                    runs: vec![run(jan1, feb1 - 1, Amount::Unknown(ProductionSpendingUnknown::LostOutput)), run(feb1, feb28, Amount::Known(0))],
                    unproven: Vec::new(),
                },
                CachedLeaf {
                    classifications: classified(jan1, feb28, Some(Domain::Air), Some(feb1)),
                    purpose: ProductionPurpose::New,
                    runs: vec![run(jan1, feb28, Amount::Known(500_000))],
                    unproven: Vec::new(),
                },
            ],
            repairs_total: Some(vec![run(jan1, feb28, Amount::Known(0))]),
            warnings: Vec::new(),
        };
        let reply = compute_months(&branch, "1", Domain::Naval, true, jan1, feb1 - 1);
        let jan = &reply.months[0];
        assert_eq!(jan.parts[0].raw.as_deref(), Some("1500000"), "known New ships stay visible even when lost refits make country total unknown");
        assert!(jan.parts[1].raw.is_none(), "unknown Refits stay unrecorded");
        assert_eq!(jan.parts[1].reason.as_deref(), Some("Final total missing."));
        assert!(jan.total_raw.is_none(), "month total is withheld when Refits are unknown or an unclassified design spent IC");
        assert_eq!(jan.total_reason.as_deref(), Some(TYPE_NOTE));
        assert_eq!(reply.next_change_hour, Some(feb1), "wakes when the unclassified design is classified");

        let only_refit_missing = BranchSpending { leaves: branch.leaves[..2].to_vec(), ..branch.clone() };
        let jan_only = compute_months(&only_refit_missing, "1", Domain::Naval, true, jan1, feb1 - 1);
        assert_eq!(jan_only.months[0].parts[0].raw.as_deref(), Some("1500000"));
        assert_eq!(jan_only.months[0].total_raw, None);
        assert_eq!(jan_only.months[0].total_reason.as_deref(), Some("Final total missing."));

        let cancelled_unproven = BranchSpending {
            leaves: vec![
                branch.leaves[0].clone(),
                CachedLeaf {
                    classifications: classified(jan1, feb28, None, None),
                    purpose: ProductionPurpose::Unknown,
                    runs: vec![run(jan1, feb28, Amount::Known(0))],
                    unproven: vec![run(jan1, feb28, Amount::Known(100_000))],
                },
            ],
            ..branch
        };
        let jan_unproven = compute_months(&cancelled_unproven, "1", Domain::Naval, true, jan1, feb1 - 1);
        assert_eq!(jan_unproven.months[0].note.as_deref(), Some(DOCKYARD_PURPOSE_NOTE), "unclassified design with zero net change still flags unproven purpose");
    }
}
