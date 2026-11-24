
pub mod content;
mod coverage;
#[cfg(test)]
mod tests;

use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, Product, ReplyWarnings, at_hour::{self, At, AtHour, Keys, Owner, Read}};
use dashboard_base::Source;
use content::Content;
use coverage::Held as Covered;
use dashboard_follow::live::Source as Cell;
use dashboard_query::{
    Query,
    identity::Holder,
    timeline::{Join, Stretch},
};
use replay_data::rows::{EquipmentDestructionCause, EquipmentLossCoverageRow, EquipmentLossTotalRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub use coverage::{Coverage, CoverageSpan, DestructionCoverageSpan, DestructionRun, UnknownReason, UnknownSpan};

pub const AMOUNT_SCALE: i64 = 100_000;
pub const SERIES: &str = "equipment-losses";
const TOTAL: usize = 0;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LandSource {
    #[default]
    Destruction,
    CombatLog,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LossCountry {
    pub slot: u32,
    pub country_id: u32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LossesParams {
    pub branch_id: String,
    pub country: Country,
    #[serde(default)]
    pub land_source: LandSource,
    #[doc = " The identity; the country's holder at `to` when none."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional, as = "Option<_>"))]
    pub identity: Option<LossCountry>,
    pub from: i32,
    pub to: i32,
    pub width: u32,
    #[doc = " Hours whose totals are read exactly."]
    #[serde(default)]
    pub reads: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", tag = "kind", content = "reason")]
pub enum LossSource {
    CombatLog(u8),
    Recovered,
    SunkShips,
    SunkConvoys,
    LendLeaseSunk,
    LendLeaseSent,
    Destruction(#[cfg_attr(any(test, feature = "export-types"), ts(type = "EquipmentDestructionCause"))] Cause),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cause(pub EquipmentDestructionCause);

impl Serialize for Cause {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for Cause {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        EquipmentDestructionCause::ALL
            .into_iter()
            .find(|cause| cause.as_str() == name)
            .map(Cause)
            .ok_or_else(|| serde::de::Error::custom(format!("Unknown equipment destruction cause: {name}")))
    }
}

#[cfg(any(test, feature = "export-types"))]
fn destruction_cause_typescript() -> String {
    let names: Vec<String> = EquipmentDestructionCause::ALL.into_iter().map(|cause| format!("\"{}\"", cause.as_str())).collect();
    format!("type EquipmentDestructionCause = {};", names.join(" | "))
}

impl LossSource {
    pub fn of(code: u8) -> Option<Self> {
        match code {
            0..=3 => Some(Self::CombatLog(code)),
            4 => Some(Self::Recovered),
            EquipmentLossTotalRow::SUNK_CONVOYS => Some(Self::SunkConvoys),
            code if code == EquipmentLossTotalRow::LEND_LEASE + engine_resources::clend_lease_exchange::SUNK_KIND => Some(Self::LendLeaseSunk),
            code if code == EquipmentLossTotalRow::LEND_LEASE + engine_resources::clend_lease_exchange::DELIVERED_KIND => Some(Self::LendLeaseSent),
            code if code >= EquipmentLossTotalRow::DESTRUCTION => EquipmentDestructionCause::ALL.into_iter().find(|cause| *cause as u8 == code - EquipmentLossTotalRow::DESTRUCTION).map(|cause| Self::Destruction(Cause(cause))),
            _ => None,
        }
    }

    pub fn domain(self) -> LossDomain {
        match self {
            Self::CombatLog(_) | Self::Recovered => LossDomain::Land,
            Self::SunkShips | Self::SunkConvoys => LossDomain::Naval,
            Self::LendLeaseSunk | Self::LendLeaseSent => LossDomain::Other,
            Self::Destruction(Cause(EquipmentDestructionCause::Unknown | EquipmentDestructionCause::ScriptedStockpileFraction)) => LossDomain::Other,
            Self::Destruction(Cause(cause)) if cause.is_air() => LossDomain::Air,
            Self::Destruction(_) => LossDomain::Land,
        }
    }

    fn mixed(self) -> bool {
        matches!(self, Self::Destruction(Cause(EquipmentDestructionCause::Unknown | EquipmentDestructionCause::ScriptedStockpileFraction)))
    }

    pub fn shown(land: LandSource) -> Vec<Self> {
        let mut sources: BTreeSet<Self> = [Self::SunkShips, Self::SunkConvoys, Self::LendLeaseSunk, Self::LendLeaseSent].into();
        let combat_log = land == LandSource::CombatLog;
        if combat_log {
            sources.extend((0..=3).map(Self::CombatLog));
            sources.insert(Self::Recovered);
        }
        sources.extend(EquipmentDestructionCause::ALL.into_iter().map(|cause| Self::Destruction(Cause(cause))).filter(|source| !combat_log || source.domain() == LossDomain::Air || source.mixed()));
        sources.into_iter().collect()
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LossDomain {
    Land,
    Air,
    Naval,
    Other,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossName {
    pub key: Option<String>,
    pub template: Option<String>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossCause {
    pub source: LossSource,
    pub domain: LossDomain,
    pub name: LossName,
    #[doc = " The Combat Log's attrition column, for reasons 1 to 3."]
    pub group: Option<LossName>,
    #[doc = " False for recovered equipment and lend-lease sent."]
    pub loss: bool,
    #[doc = " Why the amounts are unknown throughout; then no leaves."]
    pub unknown: Option<String>,
    pub unknown_spans: Vec<UnknownSpan>,
    #[doc = " Sunk ships' leaves are made from [`LossesReply::ships`] at the hour shown."]
    pub leaves: Vec<LossLeaf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossLeaf {
    #[doc = " Index into [`LossesReply::designs`]; null when the record names none."]
    pub design: Option<u32>,
    #[doc = " The totals row's key, the live cell `equipment-loss-head` names it by."]
    pub key: String,
    #[doc = " The running total at the end of each bin it changed in."]
    pub points: Vec<LossPoint>,
    #[doc = " Exact running totals read at requested hours; null when absent or unrecorded."]
    pub exact: Vec<ExactLossPoint>,
    pub lower_bound_from: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossPoint {
    pub hour: i32,
    #[doc = " Exact decimal, in [`LossesReply::amount_scale`] units."]
    pub total_raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExactLossPoint {
    pub hour: i32,
    #[doc = " Exact decimal, in [`LossesReply::amount_scale`] units, or null when absent or unrecorded."]
    pub total_raw: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossDesignName {
    pub first_hour: i32,
    pub name: Option<String>,
    pub known: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossDesign {
    pub variant: u32,
    pub names: Vec<LossDesignName>,
    pub equipment: Option<String>,
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ShipListing {
    pub signature: String,
    pub listed_hour: i32,
    pub sink_hour: i32,
    pub design: Option<u32>,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossesReply {
    pub branch_id: String,
    pub tag: String,
    pub land_source: LandSource,
    pub land_lower_bound: Option<String>,
    pub amount_scale: u32,
    pub causes: Vec<LossCause>,
    pub designs: Vec<LossDesign>,
    pub ships: Vec<ShipListing>,
    #[doc = " Combat Log land coverage; empty in destruction mode."]
    pub coverage: Vec<CoverageSpan>,
    pub destruction_coverage: Vec<DestructionCoverageSpan>,
    pub destruction_runs: Vec<DestructionRun>,
    pub retained_months: Option<i32>,
    #[doc = " The window and bins the points were read on, and the hours read exactly."]
    pub from: i32,
    pub to: i32,
    pub width: u32,
    pub exact_hours: Vec<i32>,
    #[doc = " The first hour of each stretch after hours the timeline does not hold, where totals restart."]
    pub restarts: Vec<i32>,
    pub warnings: Vec<String>,
}

impl ReplyWarnings for LossesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

pub fn identity_range(slot: u32, id: u32) -> (u128, u128) {
    let low = u128::from(slot) << 73 | u128::from(id) << 41;
    (low, low | ((1u128 << 41) - 1))
}

fn coverage_key(slot: u32, id: u32) -> u128 {
    u128::from(slot) << 32 | u128::from(id)
}

const RUN_KEY: u128 = (EquipmentLossCoverageRow::RUN as u128) << 32 | EquipmentLossCoverageRow::RUN as u128;

pub fn leaf_of(key: u128) -> Option<(LossSource, Option<u32>)> {
    let source = LossSource::of((key >> 33) as u8)?;
    let variant = ((key >> 32) & 1 == 1).then_some(key as u32);
    Some((source, variant))
}

fn known<T: Copy>(value: Recorded<T>) -> Option<T> {
    match value {
        Recorded::Value(value) => Some(value),
        Recorded::NotRecorded => None,
    }
}

fn identity(query: &Query<'_>, params: &LossesParams) -> io::Result<LossCountry> {
    let runs = query.holders.within(&params.country, params.from, params.to);
    if let Some(asked) = params.identity {
        let held = runs.iter().any(|run| run.holder == Holder::Found { slot: asked.slot, id: asked.country_id });
        return if held { Ok(asked) } else { Err(dashboard_core::refused("The chosen country identity does not hold the tag in this window")) };
    }
    match query.holders.at(&params.country, params.to).map(|run| run.holder) {
        Some(Holder::Found { slot, id }) => Ok(LossCountry { slot, country_id: id }),
        _ => runs
            .iter()
            .rev()
            .find_map(|run| match run.holder {
                Holder::Found { slot, id } => Some(LossCountry { slot, country_id: id }),
                Holder::Unresolved => None,
            })
            .ok_or_else(|| dashboard_core::refused("Equipment losses country is unavailable in this window")),
    }
}

#[derive(Default)]
struct Leaf {
    points: BTreeMap<i32, i64>,
    exact: BTreeMap<i32, Option<i64>>,
    incomplete_from: Option<i32>,
}

fn has_exact_in(exact_hours: &[i32], low: i32, high: i32) -> bool {
    if low > high {
        return false;
    }
    let at = exact_hours.partition_point(|&hour| hour < low);
    exact_hours.get(at).is_some_and(|&hour| hour <= high)
}

pub fn bin_points(bins: &[replay_lanes::lanes::Bin], first_bin_hour: i64, bin_hours: i64, from: i32, to: i32) -> Vec<(i32, i64)> {
    bin_points_for(bins, first_bin_hour, bin_hours, from, to, &[])
}

fn bin_points_for(bins: &[replay_lanes::lanes::Bin], first_bin_hour: i64, bin_hours: i64, from: i32, to: i32, exact_hours: &[i32]) -> Vec<(i32, i64)> {
    let mut out: Vec<(i32, i64)> = Vec::new();
    for (at, bin) in bins.iter().enumerate() {
        if bin.hours == 0 {
            continue;
        }
        let start = (first_bin_hour + at as i64 * bin_hours).max(i64::from(from)) as i32;
        let end = (first_bin_hour + (at as i64 + 1) * bin_hours - 1).min(i64::from(to)) as i32;
        if start > end {
            continue;
        }
        let crossed_exact = out.last().is_some_and(|&(prev, _)| has_exact_in(exact_hours, prev, start.saturating_sub(1)));
        if out.last().is_none_or(|&(_, value)| value != bin.first) || crossed_exact {
            out.push((start, bin.first));
        }
        if end > start && (bin.last != bin.first || has_exact_in(exact_hours, start, end - 1)) {
            out.push((end, bin.last));
        }
    }
    out
}

fn ship_log() -> io::Result<&'static dashboard_declare::Log> {
    dashboard_declare::LOGS.iter().find(|log| log.table == TableId::ShipLosses).ok_or_else(|| io::Error::other("The ship-losses log is not declared"))
}

type Listed = (String, i32, i32, u64, u32);

fn ships(reader: &dashboard_query::log::Reader<'_, '_>, run: u32, who: LossCountry, first: i32, last: i32) -> io::Result<Vec<Listed>> {
    use dashboard_query::log::{Cell as Logged, Pattern, facet_shift};
    let six = replay_columnar::summary::bucket_hours(0);
    let pattern = Pattern { run, first, last, mask: u128::from(u32::MAX) << facet_shift(0), value: u128::from(who.slot) << facet_shift(0), group: 0 };
    let bins = reader.bins(&[pattern], first, last, six)?;
    let ranges: Vec<(i32, i32)> = bins
        .first()
        .into_iter()
        .flatten()
        .filter(|(_, count)| *count > 0)
        .map(|&(start, _)| (start.max(i64::from(first)) as i32, (start + six - 1).min(i64::from(last)) as i32))
        .collect();
    if ranges.is_empty() {
        return Ok(Vec::new());
    }
    let number = |cell: Option<&Logged>| match cell {
        Some(Logged::Number(value)) => Some(*value),
        _ => None,
    };
    let mut out = Vec::new();
    for (_, row) in reader.rows(run, &[pattern], &ranges)? {
        if (row.key >> 80) as u32 & replay_data::rows::ShipLossRow::FIELD != who.country_id {
            continue;
        }
        let (Some(sink), Some(variant), Some(count), Some(check)) = (number(row.cells.first()), number(row.cells.get(1)), number(row.cells.get(2)), number(row.cells.get(3))) else { continue };
        let signature = row.key as u64 & replay_data::rows::ShipLossRow::SIGNATURE;
        out.push((format!("{signature:x}:{:x}", check as u64), row.hour, sink as i32, variant as u64, count as u32));
    }
    Ok(out)
}

fn total_key(row: &EquipmentLossTotalRow) -> u128 {
    identity_range(row.slot, row.country).0 | u128::from(row.source) << 33 | row.variant.map_or(0, |variant| 1 << 32 | u128::from(variant))
}

fn totals_at(query: &Query<'_>, hour: i32, (low, high): (u128, u128)) -> io::Result<Option<BTreeMap<u128, (Option<i64>, Option<i32>)>>> {
    let frame = at_hour::frame(query, hour, &[Read { table: TableId::EquipmentLossTotals, keys: Keys::Range(low, high) }], "Equipment losses")?;
    Ok(frame.complete::<tables::EquipmentLossTotals>().ok().map(|rows| rows.iter().map(|row| (total_key(row), (known(row.total), row.incomplete_from))).collect()))
}

pub fn load(query: &Query<'_>, params: &LossesParams, content: &Content, cancelled: &AtomicBool) -> io::Result<LossesReply> {
    let who = identity(query, params)?;
    let (from, to) = (params.from.max(query.timeline.first), params.to.min(query.timeline.last));
    if from > to {
        return Err(dashboard_core::refused("Equipment losses hour is not recorded"));
    }
    let recording = query.replay.recording();
    let (low, high) = identity_range(who.slot, who.country_id);
    let mut leaves: BTreeMap<u128, Leaf> = BTreeMap::new();
    let mut covered = Covered::default();
    let mut restarts = Vec::new();
    let mut listed = Vec::new();
    let log = ship_log()?;
    let reader = dashboard_query::log::Reader::new(query, log)?;
    let series = dashboard_declare::SERIES.iter().find(|series| series.kind == SERIES).ok_or_else(|| io::Error::other("The equipment-losses series is not declared"))?;
    let column = series.measure("total").map(|_| TOTAL as u32).ok_or_else(|| io::Error::other("The equipment-losses series has no total"))?;
    let mut reads: Vec<i32> = params.reads.iter().copied().filter(|hour| (query.timeline.first..=query.timeline.last).contains(hour)).collect();
    reads.sort_unstable();
    reads.dedup();
    let (lowest, highest) = (reads.first().map_or(from, |&first| first.min(from)), reads.last().map_or(to, |&last| last.max(to)));
    let (level, bin_hours) = dashboard_query::series::grid(i64::from(to) - i64::from(from) + 1, params.width.max(1));
    for stretch in query.timeline.within(query.timeline.first, query.timeline.last) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        listed.extend(ships(&reader, stretch.run, who, stretch.first, stretch.last)?);
        if stretch.first > highest {
            continue;
        }
        if stretch.join == Join::Break && stretch.first > query.timeline.first {
            restarts.push(stretch.first);
        }
        let stretch = Stretch { last: stretch.last.min(highest), ..stretch };
        let before_run = covered.run.len();
        covered.read(recording, stretch.run, coverage_key(who.slot, who.country_id), RUN_KEY, stretch.first, stretch.last)?;
        let stretch_cells = &covered.run[before_run..];
        let stretch_counted = stretch_cells.iter().filter_map(|(_, _, cells)| cells.as_ref()?.counted_from).min();
        let stretch_withheld = stretch_cells.iter().filter_map(|(_, _, cells)| cells.as_ref()?.withheld_from).min();
        let stretch_complete_through = |target: i32| -> bool {
            let Some(counted) = stretch_counted else { return false };
            if target < counted || stretch_withheld.is_some_and(|withheld| target >= withheld) {
                return false;
            }
            let mut cursor = counted;
            for (first, last, cells) in stretch_cells {
                if *last < counted {
                    continue;
                }
                if *first > target {
                    break;
                }
                if *first > cursor {
                    return false;
                }
                let Some(cells) = cells.as_ref() else { return false };
                if !coverage::is_complete_drain(cells, covered.legacy_partial_text)
                    || cells.incomplete_from.is_some_and(|incomplete| incomplete <= target)
                    || cells.withheld_from.is_some_and(|withheld| withheld <= target)
                {
                    return false;
                }
                cursor = (*last).min(target).saturating_add(1);
            }
            cursor > target
        };
        if stretch.last < lowest {
            continue;
        }
        let last = totals_at(query, stretch.last, (low, high))?.unwrap_or_default();
        let mut stretch_keys: BTreeSet<u128> = last.keys().copied().collect();
        let stretch_reads: Vec<i32> = reads.iter().copied().filter(|hour| (stretch.first..=stretch.last).contains(hour)).collect();
        let mut stretch_found = Vec::with_capacity(stretch_reads.len());
        for &hour in &stretch_reads {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let found = totals_at(query, hour, (low, high))?;
            if let Some(found) = &found {
                for (&key, &(_, incomplete_from)) in found {
                    stretch_keys.insert(key);
                    leaves.entry(key).or_default().incomplete_from = incomplete_from;
                }
            }
            stretch_found.push((hour, found));
        }
        for (&key, &(_, incomplete_from)) in &last {
            leaves.entry(key).or_default().incomplete_from = incomplete_from;
        }
        let (first, end) = (stretch.first.max(from), stretch.last.min(to));
        if first <= end {
            let lives = dashboard_query::cost::lifetimes(recording, stretch.run, TableId::EquipmentLossTotals)?;
            let start = lives.partition_point(|&(key, ..)| key < low);
            for &(key, lived_first, lived_last) in lives[start..].iter().take_while(|&&(key, ..)| key <= high) {
                if lived_last >= i64::from(first) && lived_first <= i64::from(end) {
                    stretch_keys.insert(key);
                }
            }
        }
        let keys: Vec<u128> = stretch_keys.iter().copied().collect();
        if !keys.is_empty() && first <= end {
            let charts = dashboard_query::series::charts_within(recording, stretch.run, (TableId::EquipmentLossTotals, column), Some(&keys[..]), (first, end), level, bin_hours)?;
            for (chart, (first, end)) in charts {
                for (at, &key) in chart.keys.iter().enumerate() {
                    let is_destruction = matches!(leaf_of(key), Some((LossSource::Destruction(_), _)));
                    let (leaf_first, leaf_end) = if is_destruction {
                        let Some(counted) = stretch_counted else { continue };
                        let clipped_end = stretch_withheld.map_or(end, |withheld| end.min(withheld.saturating_sub(1)));
                        (first.max(counted), clipped_end)
                    } else {
                        (first, end)
                    };
                    if leaf_first > leaf_end {
                        continue;
                    }
                    let points = bin_points_for(chart.values(at), chart.first_bin_hour, chart.bin_hours, leaf_first, leaf_end, &stretch_reads);
                    if points.is_empty() {
                        continue;
                    }
                    let leaf = leaves.entry(key).or_default();
                    for (hour, value) in points {
                        leaf.points.insert(hour, value);
                    }
                }
            }
        }
        for (hour, found) in stretch_found {
            let Some(found) = found else {
                for leaf in leaves.values_mut() {
                    leaf.exact.insert(hour, None);
                }
                continue;
            };
            let destruction_complete = stretch_complete_through(hour);
            for (key, leaf) in leaves.iter_mut() {
                let is_destruction = matches!(leaf_of(*key), Some((LossSource::Destruction(_), _)));
                let value = if is_destruction {
                    if stretch_counted.is_none_or(|counted| hour < counted) {
                        None
                    } else {
                        match found.get(key) {
                            Some(&(total, _)) => total,
                            None if stretch_keys.contains(key) && destruction_complete => Some(0),
                            None => None,
                        }
                    }
                } else {
                    found.get(key).map_or(Some(0), |&(total, _)| total)
                };
                leaf.exact.insert(hour, value);
            }
        }
    }
    reply(query, params, content, leaves, covered, listed, restarts, reads, (from, to))
}

fn reply(
    query: &Query<'_>,
    params: &LossesParams,
    content: &Content,
    leaves: BTreeMap<u128, Leaf>,
    covered: Covered,
    listed: Vec<Listed>,
    restarts: Vec<i32>,
    exact_hours: Vec<i32>,
    (from, to): (i32, i32),
) -> io::Result<LossesReply> {
    let combat_log = params.land_source == LandSource::CombatLog;
    let mut warnings = content.warnings.clone();
    let mut variants: BTreeSet<u32> = leaves.keys().filter_map(|&key| leaf_of(key).and_then(|(_, variant)| variant)).collect();
    variants.extend(listed.iter().filter_map(|(_, _, _, handle, _)| EquipmentLossTotalRow::variant_id(*handle)));
    let named = content.designs(query, to, &variants, &mut warnings)?;
    let mut designs: Vec<LossDesign> = Vec::new();
    let mut index: BTreeMap<u32, u32> = BTreeMap::new();
    let mut design = |variant: u32| -> u32 {
        *index.entry(variant).or_insert_with(|| {
            designs.push(named.get(&variant).cloned().unwrap_or(LossDesign { variant, names: vec![LossDesignName { first_hour: from, name: None, known: false }], ..LossDesign::default() }));
            (designs.len() - 1) as u32
        })
    };
    let land_unknown = combat_log.then(|| content.retention_refusal()).flatten();
    let last_hour = to;
    let destruction_spans = covered.destruction_unknown(last_hour);
    let active_start = covered.active_run_start();
    let destruction_lower = covered
        .incomplete_from()
        .into_iter()
        .chain(destruction_spans.iter().filter(|span| active_start.is_none_or(|start| span.last_hour >= start)).map(|span| span.first_hour))
        .min();
    let mut by_source: BTreeMap<LossSource, Vec<(Option<u32>, u128, &Leaf)>> = BTreeMap::new();
    for (&key, leaf) in &leaves {
        if let Some((source, variant)) = leaf_of(key) {
            by_source.entry(source).or_default().push((variant, key, leaf));
        }
    }
    let mut domain_from: BTreeMap<LossSource, i32> = BTreeMap::new();
    let mut causes = Vec::new();
    for source in LossSource::shown(params.land_source) {
        let unknown = land_unknown.clone().filter(|_| matches!(source, LossSource::CombatLog(_) | LossSource::Recovered));
        let mut kept = Vec::new();
        for &(variant, key, leaf) in by_source.get(&source).map(Vec::as_slice).unwrap_or_default() {
            if combat_log && source.mixed() {
                let domain = variant.and_then(|variant| named.get(&variant)).and_then(|design| design.archetype.as_deref()).and_then(|archetype| content.domain(archetype));
                let stockpile = source == LossSource::Destruction(Cause(EquipmentDestructionCause::ScriptedStockpileFraction));
                let proven = domain == Some(LossDomain::Air) || domain == Some(if stockpile { LossDomain::Naval } else { LossDomain::Land });
                if !proven {
                    if !(stockpile && domain == Some(LossDomain::Land))
                        && let Some(first) = leaf.points.keys().copied().chain(leaf.exact.iter().filter_map(|(&hour, total)| total.map(|_| hour))).min()
                    {
                        let at = domain_from.entry(source).or_insert(first);
                        *at = (*at).min(first);
                    }
                    continue;
                }
            }
            kept.push((variant, key, leaf));
        }
        let mut spans = match source {
            LossSource::Destruction(_) => destruction_spans.clone(),
            LossSource::CombatLog(_) | LossSource::Recovered => covered.unread(EquipmentLossCoverageRow::UNREAD_COMBAT_LOG, last_hour),
            LossSource::SunkShips => covered.unread(EquipmentLossCoverageRow::UNREAD_SHIPS, last_hour),
            LossSource::SunkConvoys => covered.unread(EquipmentLossCoverageRow::UNREAD_CONVOYS, last_hour),
            LossSource::LendLeaseSunk | LossSource::LendLeaseSent => {
                let mut spans = covered.unread(EquipmentLossCoverageRow::UNREAD_LEND_LEASE, last_hour);
                spans.extend(covered.ended(last_hour));
                spans
            }
        };
        if let Some(&first_hour) = domain_from.get(&source) {
            spans.push(UnknownSpan {
                first_hour,
                last_hour: last_hour.max(first_hour),
                reason: UnknownReason::DestructionDomain,
                detail: Some("The recorded content does not resolve this destruction equipment's domain; its contribution in Combat Log mode is not known".to_owned()),
            });
        }
        spans.sort_unstable();
        let lower_bound = |leaf: &Leaf| match source {
            LossSource::Destruction(_) => destruction_lower.into_iter().chain(domain_from.get(&source).copied()).min(),
            LossSource::LendLeaseSunk | LossSource::LendLeaseSent => leaf.incomplete_from,
            _ => None,
        };
        causes.push(LossCause {
            source,
            domain: source.domain(),
            name: content.cause_name(source),
            group: matches!(source, LossSource::CombatLog(1..=3)).then(|| content.attrition_name()),
            loss: !matches!(source, LossSource::Recovered | LossSource::LendLeaseSent),
            unknown_spans: spans,
            leaves: match unknown {
                Some(_) => Vec::new(),
                None => kept
                    .into_iter()
                    .map(|(variant, key, leaf)| LossLeaf {
                        design: variant.map(&mut design),
                        key: key.to_string(),
                        points: leaf.points.iter().map(|(&hour, &total)| LossPoint { hour, total_raw: total.to_string() }).collect(),
                        exact: leaf.exact.iter().map(|(&hour, &total)| ExactLossPoint { hour, total_raw: total.map(|total| total.to_string()) }).collect(),
                        lower_bound_from: lower_bound(leaf),
                    })
                    .collect(),
            },
            unknown,
        });
    }
    let ships: Vec<ShipListing> = listed
        .into_iter()
        .map(|(signature, listed_hour, sink_hour, handle, count)| ShipListing {
            signature,
            listed_hour,
            sink_hour,
            design: EquipmentLossTotalRow::variant_id(handle).map(&mut design),
            count,
        })
        .collect();
    warnings.extend(covered.warnings(combat_log));
    Ok(LossesReply {
        branch_id: params.branch_id.clone(),
        tag: query.holders.name(&params.country, query.timeline.last),
        land_source: params.land_source,
        land_lower_bound: None,
        amount_scale: AMOUNT_SCALE as u32,
        causes,
        designs,
        ships,
        coverage: if combat_log { covered.land() } else { Vec::new() },
        destruction_coverage: covered.destruction_coverage(),
        destruction_runs: covered.destruction_runs(),
        retained_months: content.retained_months.filter(|_| combat_log),
        from,
        to,
        width: params.width,
        exact_hours,
        restarts,
        warnings,
    })
}

pub struct EquipmentLosses;

impl Product for EquipmentLosses {
    const KEY: &'static str = "equipment-losses";
    const CONTENT_DEPENDENT: bool = true;
    type Params = LossesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, landSource, identity, from: Start, to: End, width, "reads[]": Read, "identity.countryId", "identity.slot"];
    type Reply = LossesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            LandSource::decl(),
            LossCountry::decl(),
            LossesParams::decl(),
            destruction_cause_typescript(),
            LossSource::decl(),
            LossDomain::decl(),
            LossName::decl(),
            UnknownReason::decl(),
            UnknownSpan::decl(),
            LossPoint::decl(),
            ExactLossPoint::decl(),
            LossLeaf::decl(),
            LossCause::decl(),
            LossDesignName::decl(),
            LossDesign::decl(),
            ShipListing::decl(),
            Coverage::decl(),
            CoverageSpan::decl(),
            DestructionCoverageSpan::decl(),
            DestructionRun::decl(),
            LossesReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        dashboard_base::products::CountryParams { branch_id: params.branch_id.clone(), country: params.country.clone() }.validate()?;
        if params.from > params.to {
            return Err("Invalid equipment-loss window");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| io::Error::other("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let content = Content::load(source, context, params.land_source)?;
        load(&query, params, &content, cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LossHeadParams {
    pub branch_id: String,
    pub country: Country,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional, as = "Option<_>"))]
    pub identity: Option<LossCountry>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossHeadReply {
    pub leaves: Vec<String>,
    pub warnings: Vec<String>,
}

impl ReplyWarnings for LossHeadReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

pub struct EquipmentLossHead;

fn head_identity(params: &LossHeadParams, owner: Option<Owner>) -> Option<(u32, u32)> {
    params.identity.map(|country| (country.slot, country.country_id)).or(owner.map(|owner| (owner.slot, owner.id)))
}

impl AtHour for EquipmentLossHead {
    const KEY: &'static str = "equipment-loss-head";
    const TITLE: &'static str = "Equipment losses";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LossHeadParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, identity, "identity.countryId", "identity.slot"];
    type Reply = LossHeadReply;
    type Context = ();

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        dashboard_base::products::CountryParams { branch_id: params.branch_id.clone(), country: params.country.clone() }.validate()
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        params.identity.is_none().then_some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::EquipmentLossTotals]
    }

    fn reads(params: &Self::Params, owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match (step, head_identity(params, owner)) {
            (0, Some((slot, id))) => {
                let (low, high) = identity_range(slot, id);
                vec![Read { table: TableId::EquipmentLossTotals, keys: Keys::Range(low, high) }]
            }
            _ => Vec::new(),
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let Some((slot, id)) = head_identity(params, at.owner) else { return Err(dashboard_core::refused("Equipment losses country is unavailable at this hour")) };
        let (low, high) = identity_range(slot, id);
        let rows = frame.complete::<tables::EquipmentLossTotals>()?;
        let mut leaves = Vec::new();
        for row in rows {
            let key = identity_range(row.slot, row.country).0 | (u128::from(row.source) << 33) | row.variant.map_or(0, |variant| 1 << 32 | u128::from(variant));
            if !(low..=high).contains(&key) {
                continue;
            }
            let name = key.to_string();
            at.live.value(&name, "total", Cell { table: TableId::EquipmentLossTotals, column: TOTAL, key }, 1.0);
            leaves.push(name);
        }
        Ok(LossHeadReply { leaves, warnings: Vec::new() })
    }
}
