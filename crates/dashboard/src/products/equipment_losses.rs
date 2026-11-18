//! One country identity's equipment losses, from the equipment-loss writer's tables: a tree of
//! cause, equipment type and exact design, each leaf a running total.
//!
//! - `equipment-losses` answers a window: each leaf's total at the chart's bins (the declared
//!   `equipment-losses` series' summaries) and exactly at the hours asked, the sunk ships the
//!   `ship-losses` log listed, and the coverage the writer recorded (the drain's reasons, the
//!   records not read, ended lend-lease exchanges, Combat Log land coverage). It never names the
//!   playhead: a followed view asks it for the whole branch and paints the head's values from
//!   `equipment-loss-head`.
//! - `equipment-loss-head` (rows at an hour) names each leaf's total as a live cell, so the
//!   head's totals are painted from followed cells, never pushed.
//!
//! Causes and land modes are main's: destruction mode takes every drain cause, ships, convoys and
//! lend-lease; Combat Log mode takes the log's reasons and recovered equipment, the air causes,
//! and the two mixed causes where the recorded content proves a design land or air (stockpile
//! fractions: air or naval). A source no row names holds no leaves; a cause the content refuses
//! (the Combat Log's retention) is unknown by name.

pub mod content;
mod coverage;
#[cfg(test)]
mod tests;

use super::{
    LoadContext, Product, ReplyWarnings,
    at_hour::{self, At, AtHour, Keys, Owner, Read},
};
use crate::Source;
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

pub use coverage::{Coverage, CoverageSpan, DestructionCoverageSpan, UnknownReason, UnknownSpan};

/// Units per equipment of every amount (fixed5).
pub const AMOUNT_SCALE: i64 = 100_000;
/// The declared series the chart reads.
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

/// A country identity (canonical slot and raw id) the reply is about.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LossCountry {
    pub slot: u32,
    pub country_id: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LossesParams {
    pub branch_id: String,
    pub tag: String,
    #[serde(default)]
    pub land_source: LandSource,
    /// The identity; the tag's holder at `to` when none.
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional, as = "Option<_>"))]
    pub country: Option<LossCountry>,
    pub from: i32,
    pub to: i32,
    pub width: u32,
    /// Hours whose totals are read exactly.
    #[serde(default)]
    pub reads: Vec<i32>,
}

/// Where a cause's amounts come from.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", tag = "kind", content = "reason")]
pub enum LossSource {
    /// A Combat Log reason (0 to 3) of land equipment.
    CombatLog(u8),
    /// Land equipment recovered after combat.
    Recovered,
    SunkShips,
    SunkConvoys,
    LendLeaseSunk,
    LendLeaseSent,
    Destruction(#[cfg_attr(any(test, feature = "export-types"), ts(type = "EquipmentDestructionCause"))] Cause),
}

/// A destruction cause as replies name it: the recording's own name of it.
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
    /// The source a totals row's `source` code names; none for a code no source holds.
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

    /// The causes a land mode shows, in order.
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

/// A cause's writer domain; mixed or unrecognised callers are Other.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LossDomain {
    Land,
    Air,
    Naval,
    Other,
}

/// A cause's localisation: the key, its text without colour codes (may hold the game's `$NAME$`
/// placeholders), and the text when it needs no substitution.
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
    /// The Combat Log's attrition column, for reasons 1 to 3.
    pub group: Option<LossName>,
    /// False for recovered equipment and lend-lease sent.
    pub loss: bool,
    /// Why the amounts are unknown throughout; then no leaves.
    pub unknown: Option<String>,
    pub unknown_spans: Vec<UnknownSpan>,
    /// Sunk ships' leaves are made from [`LossesReply::ships`] at the hour shown.
    pub leaves: Vec<LossLeaf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossLeaf {
    /// Index into [`LossesReply::designs`]; null when the record names none.
    pub design: Option<u32>,
    /// The totals row's key, the live cell `equipment-loss-head` names it by.
    pub key: String,
    /// The running total at the end of each bin it changed in, and at each exact hour.
    pub points: Vec<LossPoint>,
    pub lower_bound_from: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossPoint {
    pub hour: i32,
    /// Exact decimal, in [`LossesReply::amount_scale`] units.
    pub total_raw: String,
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

/// A sunk ship as the `ship-losses` log listed it: the read that listed it so often first, its
/// sinking hour and design. The most `count` of a signature over the reads up to an hour is how
/// many such ships the country lost.
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

#[derive(Clone, Debug, Serialize)]
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
    /// Combat Log land coverage; empty in destruction mode.
    pub coverage: Vec<CoverageSpan>,
    pub destruction_coverage: Vec<DestructionCoverageSpan>,
    pub destruction_counted_from: Option<i32>,
    pub retained_months: Option<i32>,
    /// The window and bins the points were read on, and the hours read exactly.
    pub from: i32,
    pub to: i32,
    pub width: u32,
    pub exact_hours: Vec<i32>,
    /// The first hour of each stretch after hours the timeline does not hold, where totals restart.
    pub restarts: Vec<i32>,
    pub warnings: Vec<String>,
}

impl ReplyWarnings for LossesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

/// The totals table's keys of one identity: every source and design.
pub fn identity_range(slot: u32, id: u32) -> (u128, u128) {
    let low = u128::from(slot) << 73 | u128::from(id) << 41;
    (low, low | ((1u128 << 41) - 1))
}

fn coverage_key(slot: u32, id: u32) -> u128 {
    u128::from(slot) << 32 | u128::from(id)
}

const RUN_KEY: u128 = (EquipmentLossCoverageRow::RUN as u128) << 32 | EquipmentLossCoverageRow::RUN as u128;

/// A leaf's source and design from its totals key.
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

/// The identity the reply reads: the one asked, when it held the tag in the window, else the
/// tag's holder at `to`.
fn identity(query: &Query<'_>, params: &LossesParams) -> io::Result<LossCountry> {
    let runs = query.holders.within(&params.tag, params.from, params.to);
    if let Some(asked) = params.country {
        let held = runs.iter().any(|run| run.holder == Holder::Found { slot: asked.slot, id: asked.country_id });
        return if held { Ok(asked) } else { Err(dashboard_core::refused("The chosen country identity does not hold the tag in this window")) };
    }
    match query.holders.at(&params.tag, params.to).map(|run| run.holder) {
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

/// One leaf's totals across the window, as read.
#[derive(Default)]
struct Leaf {
    points: BTreeMap<i32, Option<i64>>,
    incomplete_from: Option<i32>,
}

/// Bin ends, then exact reads over them: a bin's first value at its first hour and its last at
/// its last hour, where they differ from the value before; a value not recorded ends nothing.
pub fn bin_points(bins: &[replay::lanes::Bin], first_bin_hour: i64, bin_hours: i64, from: i32, to: i32) -> Vec<(i32, i64)> {
    let mut out: Vec<(i32, i64)> = Vec::new();
    for (at, bin) in bins.iter().enumerate() {
        if bin.hours == 0 {
            continue;
        }
        let start = (first_bin_hour + at as i64 * bin_hours).max(i64::from(from)) as i32;
        let end = (first_bin_hour + (at as i64 + 1) * bin_hours - 1).min(i64::from(to)) as i32;
        if out.last().is_none_or(|&(_, value)| value != bin.first) {
            out.push((start, bin.first));
        }
        if bin.last != bin.first && end > start {
            out.push((end, bin.last));
        }
    }
    out
}

fn ship_log() -> io::Result<&'static dashboard_declare::Log> {
    dashboard_declare::LOGS.iter().find(|log| log.table == TableId::ShipLosses).ok_or_else(|| io::Error::other("The ship-losses log is not declared"))
}

/// A sunk ship's listing as read: its signature, the read's hour, its sinking hour, design handle
/// and count.
type Listed = (String, i32, i32, u64, u32);

/// The ships the `ship-losses` log listed for `who` from `first` through `last` of run `run`: only
/// the six-hour buckets its slot's counts name are read.
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

/// A totals row's key.
fn total_key(row: &EquipmentLossTotalRow) -> u128 {
    identity_range(row.slot, row.country).0 | u128::from(row.source) << 33 | row.variant.map_or(0, |variant| 1 << 32 | u128::from(variant))
}

/// The identity's totals at `hour`, by key, each not recorded or its value; none when the table
/// is not complete at the hour.
fn totals_at(query: &Query<'_>, hour: i32, (low, high): (u128, u128)) -> io::Result<Option<BTreeMap<u128, (Option<i64>, Option<i32>)>>> {
    let frame = at_hour::frame(query, hour, &[Read { table: TableId::EquipmentLossTotals, keys: Keys::Range(low, high) }], "Equipment losses")?;
    Ok(frame.complete::<tables::EquipmentLossTotals>().ok().map(|rows| rows.iter().map(|row| (total_key(row), (known(row.total), row.incomplete_from))).collect()))
}

/// The window's reply.
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
    // Ships from every listing of the timeline, each dated at its sinking; coverage from the run's start, so a span or a
    // reason begun before the window still holds in it.
    for stretch in query.timeline.within(query.timeline.first, query.timeline.last) {
        crate::metadata::check_cancel(cancelled)?;
        listed.extend(ships(&reader, stretch.run, who, stretch.first, stretch.last)?);
        if stretch.first > highest {
            continue;
        }
        if stretch.join == Join::Break && stretch.first > query.timeline.first {
            restarts.push(stretch.first);
        }
        let stretch = Stretch { last: stretch.last.min(highest), ..stretch };
        covered.read(recording, stretch.run, coverage_key(who.slot, who.country_id), RUN_KEY, stretch.first, stretch.last)?;
        if stretch.last < lowest {
            continue;
        }
        let last = totals_at(query, stretch.last, (low, high))?.unwrap_or_default();
        let keys: Vec<u128> = last.keys().copied().collect();
        for (&key, &(_, incomplete_from)) in &last {
            leaves.entry(key).or_default().incomplete_from = incomplete_from;
        }
        let (first, end) = (stretch.first.max(from), stretch.last.min(to));
        if !keys.is_empty() && first <= end {
            let charts = dashboard_query::series::charts_within(recording, stretch.run, (TableId::EquipmentLossTotals, column), Some(&keys[..]), (first, end), level, bin_hours)?;
            for (chart, (first, end)) in charts {
                for (at, &key) in chart.keys.iter().enumerate() {
                    let leaf = leaves.entry(key).or_default();
                    for (hour, value) in bin_points(chart.values(at), chart.first_bin_hour, chart.bin_hours, first, end) {
                        leaf.points.insert(hour, Some(value));
                    }
                }
            }
        }
        for &hour in reads.iter().filter(|hour| (stretch.first..=stretch.last).contains(*hour)) {
            crate::metadata::check_cancel(cancelled)?;
            let Some(found) = totals_at(query, hour, (low, high))? else { continue };
            for (key, leaf) in leaves.iter_mut() {
                leaf.points.insert(hour, found.get(key).map_or(Some(0), |&(total, _)| total));
            }
        }
    }
    reply(query, params, content, leaves, covered, listed, restarts, reads, (from, to))
}

#[allow(clippy::too_many_arguments, reason = "the reply joins every read of the window")]
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
    let destruction_lower = covered.incomplete_from().into_iter().chain(destruction_spans.iter().map(|span| span.first_hour)).min();
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
                        && let Some(first) = leaf.points.keys().next()
                    {
                        let at = domain_from.entry(source).or_insert(*first);
                        *at = (*at).min(*first);
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
                        points: leaf.points.iter().filter_map(|(&hour, total)| total.map(|total| LossPoint { hour, total_raw: total.to_string() })).collect(),
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
        tag: params.tag.clone(),
        land_source: params.land_source,
        land_lower_bound: None,
        amount_scale: AMOUNT_SCALE as u32,
        causes,
        designs,
        ships,
        coverage: if combat_log { covered.land() } else { Vec::new() },
        destruction_coverage: covered.destruction_coverage(),
        destruction_counted_from: covered.counted_from(),
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
            LossLeaf::decl(),
            LossCause::decl(),
            LossDesignName::decl(),
            LossDesign::decl(),
            ShipListing::decl(),
            Coverage::decl(),
            CoverageSpan::decl(),
            DestructionCoverageSpan::decl(),
            LossesReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::CountryParams { branch_id: params.branch_id.clone(), tag: params.tag.clone() }.validate()?;
        if params.from > params.to {
            return Err("Invalid equipment-loss window");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().map_err(|_| io::Error::other("Invalid branch"))?;
        let query = crate::answer::query(source, branch)?;
        let content = Content::load(source, context, params.land_source)?;
        load(&query, params, &content, cancelled)
    }
}

/// `equipment-loss-head`'s parameters: the tag, and the identity the widget shows when it chose
/// one.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LossHeadParams {
    pub branch_id: String,
    pub tag: String,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional, as = "Option<_>"))]
    pub country: Option<LossCountry>,
}

/// The leaves the identity holds at the hour; each one's total is the live cell `(key, "total")`.
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
    params.country.map(|country| (country.slot, country.country_id)).or(owner.map(|owner| (owner.slot, owner.id)))
}

impl AtHour for EquipmentLossHead {
    const KEY: &'static str = "equipment-loss-head";
    const TITLE: &'static str = "Equipment losses";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LossHeadParams;
    type Reply = LossHeadReply;
    type Context = ();

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::CountryParams { branch_id: params.branch_id.clone(), tag: params.tag.clone() }.validate()
    }

    fn owner(params: &Self::Params) -> Option<&str> {
        params.country.is_none().then_some(params.tag.as_str())
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
