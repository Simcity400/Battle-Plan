//! One country's production lines over a branch, for the Production lines widget:
//! each line's recorded state at every read, the units it completed, and each
//! produced variant's name and icon and each organisation's name and icon as the
//! game's production view shows them.
//!
//! A request asks for only what the view draws. `production-lines` answers each
//! line's life: its factory steps (which change only when factories are assigned or
//! the variant switches), its final count, and the names, icons and hours recorded.
//! Values that change at nearly every read (efficiency, output per week, units made,
//! units ordered) come from `production-line-detail`, one fixed window of
//! [`DETAIL_HOURS`] at a time. `production-line-breakdown` sends one line's selected
//! modifier factor across the branch; a detail window carries no modifier trees,
//! which hold every source of every factor at each read and would make a month of
//! a large country's lines too big for one reply. Full histories grow with the
//! campaign; every reply is measured and refused past [`MAX_REPLY_BYTES`].
//!
//! Preparation observes every recorded country; requests read its published pages.
//!
//! Ordinary line values are recorded `ProductionLines` fields, compared with
//! the save by the fresh-save oracle (`recorder::oracle::production`):
//!
//! - `cost_raw` is the IC one unit costs, fixed5, after the organisation's
//!   cost bonus.
//! - `speed_raw` is the IC added at each production tick; units per week are
//!   the game's weekly formula `speed × 700000 ÷ cost` (`equipment::weekly_units_raw`,
//!   which says when the game itself shows that unit), sent as `weekly_raw`.
//! - `efficiency_raw` is the line's current efficiency in Q15 (100% is 3276800),
//!   the average of its first min(active, maximum) per-factory efficiencies; the
//!   row shows `(raw + 0x4000) >> 15` percent. `max_factories` is that maximum.
//! - `priority` is the line's place in the queue from 0; the row shows it
//!   plus one. `amount` is the units still ordered, −1 for no limit.
//!
//! **Units completed** come from `ProductionLineOutput`, the collector's count of
//! every production tick per line (the game keeps none per line; `produced_raw` is
//! progress toward the next unit): a line's total is the sum of its rows' units.
//! A row that is not exact leaves a tick out, so the total is not a number then. A
//! line first seen before the branch's first complete census counts from
//! `since_hour`. An ended line's final rows are published once, at the census that
//! no longer lists it, and kept here as its final output.
//!
//! A line switching variant is edited in place, so it keeps its handle and total.
//!
//! Production lines, their output and the organisations are read once a day in one
//! slot and carried in between (`metadata::slot_read`). Values start only at a
//! read, are carried over the following hours of the same country identity, and
//! end at a gap, an incomplete table or a change of identity. A line missing from a
//! read of its country ended before it.
//!
//! Variant names use `equipment::variant_names`; icons use `equipment_icon`,
//! evaluated at each read with that hour's countries.
//! Organisations are named as the game names them: the name-key override, else the
//! script key, localised preferring `<owner tag>_<key>`; their icon is the stored
//! sprite, else `GFX_<owner tag>_<script key>`, `GFX_<script key>`, or the game's
//! `GFX_idea_unknown` (replay README, IndustrialOrganisations).

mod binary;

use super::{
    CountryParams, LoadContext, Product,
    equipment::{self, EQUIPMENT_DECIMALS, Tags, Texts, VariantCatalogue},
    equipment_icon::{Icon, IconChoice, IconContent, IconHour, icon_choice},
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{TaggedCountry, link_slot, slot_reads},
};
use replay::{container::BlockIndex, container::Replay};
use replay_data::{
    rows::IndustrialOrganisationDetailsRow, rows::IndustrialOrganisationRow, rows::LexicalTokenRow,
    rows::ProductionLineOutputRow, rows::ProductionLineRow, rows::TableId,
    rows::VariantDefinitionRow, rows::tables,
};
#[cfg(test)]
use replay_frame::frame::FrameDecoder;
use replay_frame::{frame::DecodedFrame, frame::ViewIdentity};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

mod breakdown;
mod input;
mod pages;
pub mod prepared;
mod private;
#[cfg(test)]
mod reference;
mod saved;
pub use breakdown::{
    ProductionBreakdownParams, ProductionLineBreakdown, ProductionLineBreakdownReply,
    ProductionModifierGroup, ProductionModifierPart, ProductionModifierRun, factory_output,
};
pub use saved::{SavedLines, SavedModifierRun};

pub const PREPARED_PRODUCTS: &[&str] = &[
    "production-lines",
    "production-line-detail",
    "production-line-breakdown",
];

pub struct ProductionLines;
pub struct ProductionLineDetail;

/// The hours one detail reply covers: 30 days, so playback asks about once a month.
pub const DETAIL_HOURS: i32 = 720;
/// Each reply's bound, inside the transport's.
pub use super::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
/// Longest side of an equipment or organisation icon.
const ICON_EDGE: u32 = 96;
/// The fallback sprite for an organisation without its own.
const UNKNOWN_ORGANISATION_ICON: &str = "GFX_idea_unknown";

/// The kind of line, by its native class: military, naval, ship refit or railway gun.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LineKind {
    Military,
    Naval,
    ShipRefit,
    RailwayGun,
}

impl LineKind {
    fn of(kind: u8) -> Option<Self> {
        match kind {
            engine_native::cequipment_production_line::KIND_MILITARY => Some(Self::Military),
            engine_native::cequipment_production_line::KIND_NAVAL => Some(Self::Naval),
            engine_native::cequipment_production_line::KIND_SHIP_REFIT => Some(Self::ShipRefit),
            engine_native::cequipment_production_line::KIND_RAILWAY_GUN => Some(Self::RailwayGun),
            _ => None,
        }
    }
}

/// One recorded read of a line, held over consecutive hours while it stays
/// unchanged, as the scan keeps it: numbers, which the replies send as exact
/// decimal strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct RawRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub variant: u64,
    pub mio: u64,
    pub active_factories: i32,
    pub requested_factories: i32,
    pub max_factories: i32,
    pub cost_raw: i64,
    pub speed_raw: i64,
    pub efficiency_raw: i64,
    pub priority: i32,
    pub amount: i32,
}

/// One (variant, organisation) part of a line's output, as the scan keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RawPart {
    pub variant: u64,
    pub mio: u64,
    pub units: i64,
    pub exact: bool,
}

/// A line's output at one read, held until the next, as the scan keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RawOutput {
    pub first_hour: i32,
    pub last_hour: i32,
    pub parts: Vec<RawPart>,
    pub since_hour: i32,
    pub whole_life: bool,
}

/// One production line's life in the branch as the scan keeps it; the replies
/// send its steps ([`LineHistory`]) and a window of its reads ([`LineDetail`]).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ScannedLine {
    pub key: String,
    pub handle: u64,
    pub kind: Option<LineKind>,
    pub runs: Vec<RawRun>,
    pub output: Vec<RawOutput>,
    pub final_output: Option<RawOutput>,
    pub ended_by: Option<i32>,
    pub before_recording: bool,
    pub between_reads: Option<super::country_groups::HourSpan>,
    pub variants: Vec<u64>,
}

/// One recorded read of a line, held over consecutive hours while it stays
/// unchanged. Raw values are exact decimal strings with [`EQUIPMENT_DECIMALS`]
/// places (efficiency is Q15); references are decimal strings.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LineRun {
    /// Frozen design authorship, independently observed from the line assignment.
    pub design_author: Option<String>,
    /// Supplying VariantStats read, independent of the production-line census.
    pub design_read_hour: Option<i32>,
    pub design_trait_count: Option<i32>,
    pub design_bonus_count: Option<u32>,
    pub first_hour: i32,
    pub last_hour: i32,
    /// The run follows a gap, a missing read or another country identity.
    pub starts_segment: bool,
    /// The produced variant (`VariantDefinitions.id`).
    pub variant: String,
    /// The organisation's reference (`IndustrialOrganisations.organisation`); "0" for none.
    pub mio: String,
    pub active_factories: i32,
    pub requested_factories: i32,
    /// The most factories the line takes (the efficiency average's clamp).
    pub max_factories: i32,
    /// IC per unit, after the organisation's cost bonus.
    pub cost_raw: String,
    /// IC added at each production tick.
    pub speed_raw: String,
    /// Units per week by the game's weekly formula (`equipment::weekly_units_raw`),
    /// fixed5; null without a positive cost.
    pub weekly_raw: Option<String>,
    /// The line's efficiency, Q15: 100% is 3276800.
    pub efficiency_raw: String,
    /// Place in the country's production queue, from 0.
    pub priority: i32,
    /// Units still ordered; −1 for no limit.
    pub amount: i32,
}

impl From<&RawRun> for LineRun {
    fn from(run: &RawRun) -> Self {
        Self {
            design_author: None,
            design_read_hour: None,
            design_trait_count: None,
            design_bonus_count: None,
            first_hour: run.first_hour,
            last_hour: run.last_hour,
            starts_segment: run.starts_segment,
            variant: run.variant.to_string(),
            mio: run.mio.to_string(),
            active_factories: run.active_factories,
            requested_factories: run.requested_factories,
            max_factories: run.max_factories,
            cost_raw: run.cost_raw.to_string(),
            speed_raw: run.speed_raw.to_string(),
            weekly_raw: equipment::weekly_units_raw(run.speed_raw, run.cost_raw)
                .map(|weekly| weekly.to_string()),
            efficiency_raw: run.efficiency_raw.to_string(),
            priority: run.priority,
            amount: run.amount,
        }
    }
}

/// One (variant, organisation) part of a line's output.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OutputPart {
    pub variant: String,
    pub mio: String,
    /// Units delivered, as a decimal string.
    pub units: String,
    /// False when a tick could not be counted: `units` then leave it out.
    pub exact: bool,
}

/// A line's output as one read of `ProductionLineOutput` counts it, held until the
/// next read.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OutputRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Its parts, in first-seen order; the total is their sum, exact only when every part is.
    pub parts: Vec<OutputPart>,
    /// The sample that began the branch's observation.
    pub since_hour: i32,
    /// The line was first seen after the branch's first complete census, so the
    /// totals cover its whole life; otherwise they count from `since_hour`.
    pub whole_life: bool,
}

impl From<&RawOutput> for OutputRun {
    fn from(output: &RawOutput) -> Self {
        Self {
            first_hour: output.first_hour,
            last_hour: output.last_hour,
            parts: output
                .parts
                .iter()
                .map(|part| OutputPart {
                    variant: part.variant.to_string(),
                    mio: part.mio.to_string(),
                    units: part.units.to_string(),
                    exact: part.exact,
                })
                .collect(),
            since_hour: output.since_hour,
            whole_life: output.whole_life,
        }
    }
}

/// A stretch of a line's reads with the same variant and factories: what its
/// history lane draws. A new step starts where either changes or the reads break.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LineStep {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The step follows a gap, a missing read or another country identity.
    pub starts_segment: bool,
    /// The produced variant (`VariantDefinitions.id`).
    pub variant: String,
    pub active_factories: i32,
    pub requested_factories: i32,
}

/// One production line's life in the branch: from the read it was first seen to
/// the read that no longer listed it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LineHistory {
    /// Unique in the reply: the handle, and which appearance of it this is.
    pub key: String,
    /// The line's native handle.
    pub handle: String,
    /// Its native class; null for a line no census listed.
    pub kind: Option<LineKind>,
    /// Its variant and factories over its reads; empty for a line made and deleted
    /// between two censuses. Every other value is in `production-line-detail`.
    pub steps: Vec<LineStep>,
    /// The output published when the line ended, counting every unit it made.
    pub final_output: Option<OutputRun>,
    /// The first read of its country that no longer listed it: it ended before.
    pub ended_by: Option<i32>,
    /// Listed at the first read of its country in the branch: it began before recording.
    pub before_recording: bool,
    /// For a line no census listed: the hours within which it ran and was deleted,
    /// up to the census that published its final output. From the census before when
    /// it was observed since its creation; else (`before_recording`) from when the
    /// observation began, since it ran before and its start is not known.
    pub between_reads: Option<super::country_groups::HourSpan>,
    /// For such a line, the variants its output names (it has no runs to name them).
    pub variants: Vec<String>,
}

/// A line's reads within one detail window: every recorded line value, cut to the
/// window. Its modifier factors come from `production-line-breakdown`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LineDetail {
    pub organisations: Vec<OrganisationStateRead>,
    /// The line's key in `production-lines`.
    pub key: String,
    pub runs: Vec<LineRun>,
    /// Its output at each read (the same reads as `runs`), where recorded.
    pub output: Vec<OutputRun>,
}

/// A produced variant's names and icon as the game's production row shows them;
/// a new entry wherever a read changes them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct VariantName {
    pub id: String,
    /// The read these values are from; they hold until the next entry of this id.
    pub first_hour: i32,
    /// False from an hour the variant's names and icon stopped being known (a gap, or
    /// a variant table not read or carried: `VariantCatalogue`) until the next entry;
    /// its other fields are then empty.
    pub known: bool,
    /// The equipment type's script key.
    pub token: String,
    /// The row's variant display name; null when content does not name it.
    pub name: Option<String>,
    /// The row's equipment type line; null when content does not name it.
    pub type_name: Option<String>,
    /// The icon's key in `icons`; null when the row shows none or it is unknown.
    pub icon: Option<String>,
    /// Why the icon is unknown, when it is.
    pub icon_unknown: Option<String>,
}

/// A military-industrial organisation as the game names and pictures it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationName {
    pub id: String,
    pub first_hour: i32,
    /// The script key.
    pub token: String,
    /// The localised name; null when content does not name it without runtime scope.
    pub name: Option<String>,
    /// Why the localised name cannot be determined from this read.
    pub name_unknown: Option<String>,
    /// The icon's key in `icons`.
    pub icon: Option<String>,
    /// Why the icon cannot be determined from this read.
    pub icon_unknown: Option<String>,
}

/// MIO states demanded by a line's bounded detail window.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationStateRead {
    pub id: String,
    pub first_hour: i32,
    pub state: Option<OrganisationState>,
}

/// Stored MIO operands at a daily census; absent state remains unknown.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationState {
    pub read_hour: Option<i32>,
    pub size: i32,
    pub funds_raw: String,
    pub unspent_points: i32,
    pub base_task_capacity: i32,
    pub active_tasks: u32,
    pub task_slots: u32,
    pub unlocked: Vec<String>,
    pub planned: Vec<String>,
    pub queue_index: i32,
    pub policy_token: Option<String>,
    pub policy_name: Option<String>,
    pub cooldown_hour: Option<i32>,
    pub auto_update_designs: bool,
}

/// One picture, shared by every variant or organisation showing it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentIcon {
    pub key: String,
    /// A PNG data URL; null when the texture could not be read.
    pub image: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProductionLinesReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    /// Every line of the country in the branch, in the order first seen.
    pub lines: Vec<LineHistory>,
    /// The names and icons of every variant a line produced.
    pub variants: Vec<VariantName>,
    /// The organisations the lines were assigned to.
    pub organisations: Vec<OrganisationName>,
    pub icons: Vec<EquipmentIcon>,
    /// Hours the country's lines were read, ascending.
    pub reads: Vec<i32>,
    /// Hours whose output is recorded, ascending spans.
    pub output_hours: Vec<super::country_groups::HourSpan>,
    /// Hours whose lines are recorded (a complete census, read or carried),
    /// ascending spans: outside them the lines are not known, which is not none.
    pub line_hours: Vec<super::country_groups::HourSpan>,
    /// Hours whose organisations are recorded, ascending spans: outside them an
    /// organisation's name and icon are not known (never an earlier read's).
    pub organisation_hours: Vec<super::country_groups::HourSpan>,
    /// Hours whose lines made and ended there may be missing: a due read of the
    /// output, the only place such a line's rows are published, could not be read.
    pub unread_output: Vec<super::country_groups::HourSpan>,
    /// Hours beyond the published prefix that are still being prepared.
    pub approximate: Vec<super::country_groups::HourSpan>,
    /// The hours one `production-line-detail` window covers.
    pub detail_hours: i32,
    pub warnings: Vec<String>,
}

/// A request for one country's production lines over a branch.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionLinesParams {
    pub branch_id: String,
    pub tag: String,
    /// Wait for the branch's complete prepared history.
    #[serde(default)]
    #[ts(optional)]
    pub exact: Option<bool>,
}

impl ProductionLinesParams {
    fn country(&self) -> CountryParams {
        CountryParams {
            branch_id: self.branch_id.clone(),
            tag: self.tag.clone(),
        }
    }
}

/// A request for one window of a country's line reads: hours
/// `window × DETAIL_HOURS` to the hour before the next window.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionDetailParams {
    pub branch_id: String,
    pub tag: String,
    pub window: i32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProductionLineDetailReply {
    pub warnings: Vec<String>,
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    pub window: i32,
    /// The window's hours, both included.
    pub first_hour: i32,
    pub last_hour: i32,
    /// Every line with a read in the window, in the order first seen.
    pub lines: Vec<LineDetail>,
}

/// An organisation's naming operands at a read.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawOrganisation {
    pub first_hour: i32,
    pub token: String,
    pub name_key: String,
    pub icon: String,
    pub owner_tag: Option<String>,
    /// A positive owner reference the read cannot resolve, rather than no owner.
    pub owner_unknown: bool,
    pub state: Option<OrganisationState>,
}

/// A variant's icon at a read, with its artwork questions left open for the reply's
/// presentation ([`IconChoice`]), or why it is unknown.
pub type IconAt = (i32, IconChoice);

/// A demanded design's own observation, including explicit unavailable intervals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct DesignRead {
    pub first_hour: i32,
    pub read_hour: Option<i32>,
    pub author: Option<u64>,
    pub trait_count: Option<i32>,
    pub bonus_count: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct ScannedLines {
    pub designs: BTreeMap<u64, Vec<DesignRead>>,
    pub lines: Vec<ScannedLine>,
    pub modifiers: BTreeMap<String, Vec<breakdown::RawModifierRun>>,
    /// Counts supplying reads whose fixed modifier names did not authenticate.
    pub modifier_layout_refusals: BTreeMap<String, usize>,
    /// Every variant's names over the branch, with where they are known.
    pub variants: VariantCatalogue,
    pub icons: BTreeMap<u64, Vec<IconAt>>,
    pub organisations: BTreeMap<u64, Vec<RawOrganisation>>,
    pub reads: Vec<i32>,
    pub output_hours: Vec<super::country_groups::HourSpan>,
    pub line_hours: Vec<super::country_groups::HourSpan>,
    pub organisation_hours: Vec<super::country_groups::HourSpan>,
    /// Lines of an unknown native class.
    pub unknown_kinds: usize,
    /// Output rows of this country naming a line no census listed.
    pub unlisted_output: usize,
    /// Due output reads of this country that could not be read.
    pub unread_output_reads: usize,
    /// Stretches of the recording (a block, or its frames from one that did not
    /// decode) this scan could not read at all.
    pub unreadable_stretches: usize,
    /// Line or output rows whose producer is not a linked country: whose they are is
    /// not known, so they are counted, not dropped silently.
    pub unlinked_rows: usize,
    /// The hours those reads covered: lines made and ended within them may be missing.
    pub unread_output: Vec<super::country_groups::HourSpan>,
}

/// The same recorded values, whatever hours they cover.
fn same_values(a: &RawRun, b: &RawRun) -> bool {
    let bare = |run: &RawRun| RawRun {
        first_hour: 0,
        last_hour: 0,
        starts_segment: false,
        ..*run
    };
    bare(a) == bare(b)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn token_of(lexical: &[LexicalTokenRow], token: u32) -> String {
    lexical
        .binary_search_by_key(&token, |row| row.token)
        .ok()
        .map(|index| text(&lexical[index].literal))
        .unwrap_or_default()
}

/// Adds hours `first` to `last`, joined to the last span where they touch it.
fn mark_span(spans: &mut Vec<super::country_groups::HourSpan>, first: i32, last: i32) {
    match spans.last_mut() {
        Some(span)
            if span
                .last_hour
                .checked_add(1)
                .is_some_and(|next| next >= first) =>
        {
            span.last_hour = span.last_hour.max(last)
        }
        _ => spans.push(super::country_groups::HourSpan {
            first_hour: first,
            last_hour: last,
        }),
    }
}

fn mark(spans: &mut Vec<super::country_groups::HourSpan>, hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) == Some(hour) => span.last_hour = hour,
        Some(span) if span.last_hour >= hour => {}
        _ => spans.push(super::country_groups::HourSpan {
            first_hour: hour,
            last_hour: hour,
        }),
    }
}

/// A line's output from this read's rows naming it, each with whether its producer
/// link resolved: a part whose did not is not exact.
fn output_of(rows: &[(&ProductionLineOutputRow, bool)], hour: i32) -> RawOutput {
    RawOutput {
        first_hour: hour,
        last_hour: hour,
        parts: rows
            .iter()
            .map(|(row, linked)| RawPart {
                variant: row.variant,
                mio: row.mio,
                units: row.units,
                exact: row.exact && *linked,
            })
            .collect(),
        since_hour: rows
            .iter()
            .map(|(row, _)| row.since_hour)
            .min()
            .unwrap_or(hour),
        whole_life: rows.iter().all(|(row, _)| row.observed_since_creation),
    }
}

/// Pushes a version when it differs from the last, whatever hour it is from; whether it did.
fn version<T: Clone + PartialEq>(
    versions: &mut Vec<T>,
    value: T,
    same: impl Fn(&T, &T) -> bool,
) -> bool {
    let differs = versions.last().is_none_or(|last| !same(last, &value));
    if differs {
        versions.push(value);
    }
    differs
}

/// Overlay the design's independent versions only in the requested line detail.
/// The underlying line runs and branch-wide factory steps keep their own cadence.
fn append_design_runs(
    result: &mut Vec<LineRun>,
    run: &RawRun,
    versions: &[DesignRead],
    window_first: i32,
    window_last: i32,
) {
    let start = run.first_hour.max(window_first);
    let end = run.last_hour.min(window_last);
    if start > end {
        return;
    }
    let mut next = versions.partition_point(|read| read.first_hour <= start);
    let mut first = next.checked_sub(1).map_or(run.first_hour, |index| {
        run.first_hour.max(versions[index].first_hour)
    });
    loop {
        let last = versions.get(next).map_or(run.last_hour, |read| {
            run.last_hour.min(read.first_hour.saturating_sub(1))
        });
        let mut value = LineRun::from(run);
        value.first_hour = first;
        value.last_hour = last;
        value.starts_segment &= first == run.first_hour;
        if let Some(read) = next.checked_sub(1).and_then(|index| versions.get(index)) {
            value.design_author = read.author.map(|id| id.to_string());
            value.design_read_hour = read.read_hour;
            value.design_trait_count = read.trait_count;
            value.design_bonus_count = read.bonus_count;
        }
        result.push(value);
        if last >= end {
            break;
        }
        first = last + 1;
        next += 1;
    }
}

/// A line open at the last read: where it is in `lines`.
#[derive(Clone, Serialize, Deserialize)]
struct Open {
    index: usize,
}

#[cfg(test)]
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    icons: Option<&IconContent>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedLines> {
    scan_holder(replay, branch, tag, icons, None, cancelled)
}
#[cfg(test)]
fn scan_holder(
    replay: &Replay,
    branch: u64,
    tag: &str,
    icons: Option<&IconContent>,
    holder: Option<(u32, u32)>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedLines> {
    prepared::fixtures::scan(
        replay,
        &branch_blocks(replay, branch),
        tag,
        icons,
        holder,
        cancelled,
        true,
    )
}

/// A branch's blocks, in recorded order.
fn branch_blocks(replay: &Replay, branch: u64) -> Vec<&BlockIndex> {
    replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect()
}

/// The recording's language as modifier runs name it: without its `l_` prefix.
pub fn recorded_language(replay: &Replay) -> Arc<str> {
    replay
        .identity
        .language
        .strip_prefix("l_")
        .unwrap_or(&replay.identity.language)
        .into()
}

/// The country a scan reads, as one hour's countries and links name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Country {
    /// No country holds the tag.
    Absent,
    /// Countries hold it, but the links resolve none of them.
    Unresolved,
    /// Its canonical slot and native id.
    Found(u32, u32),
}

impl Country {
    fn found(self) -> Option<(u32, u32)> {
        match self {
            Self::Found(slot, id) => Some((slot, id)),
            Self::Absent | Self::Unresolved => None,
        }
    }
}

/// Extend each open line through consecutive hours with equal availability.
fn carry_lines(
    result: &mut ScannedLines,
    open: &BTreeMap<u64, Open>,
    span: (i32, i32),
    modifiers_carried: bool,
    output_open: bool,
) {
    let (hour, through) = span;
    for line in open.values() {
        crate::prepared::prepass::note("production-lines.carried_line");
        let history = &mut result.lines[line.index];
        if modifiers_carried
            && let Some(modifiers) = result
                .modifiers
                .get_mut(&history.key)
                .and_then(|runs| runs.last_mut())
            && modifiers.last_hour.checked_add(1) == Some(hour)
        {
            modifiers.last_hour = through;
        }
        if let Some(run) = history.runs.last_mut()
            && run.last_hour.checked_add(1) == Some(hour)
        {
            run.last_hour = through;
        }
        if output_open
            && let Some(output) = history.output.last_mut()
            && output.last_hour.checked_add(1) == Some(hour)
        {
            output.last_hour = through;
        }
    }
}

pub fn raw_organisation(
    row: &IndustrialOrganisationRow,
    hour: i32,
    tags: &Tags,
    lexical: &[LexicalTokenRow],
) -> RawOrganisation {
    let owner_tag = tags.tag_of(row.owner);
    RawOrganisation {
        first_hour: hour,
        token: token_of(lexical, row.token),
        name_key: text(&row.name),
        icon: text(&row.icon),
        owner_unknown: (row.owner as i32) > 0 && owner_tag.is_none(),
        owner_tag,
        state: None,
    }
}

/// Native constructor and save-writer sentinel for no selected policy.
const UNSELECTED_POLICY: u32 = 0x4c17;

pub fn organisation_state(
    row: &IndustrialOrganisationDetailsRow,
    read_hour: Option<i32>,
    lexical: &[LexicalTokenRow],
) -> OrganisationState {
    OrganisationState {
        read_hour,
        size: row.state.size,
        funds_raw: row.state.funds_raw.to_string(),
        unspent_points: row.state.points,
        base_task_capacity: row.state.task_capacity,
        active_tasks: row
            .tasks
            .iter()
            .filter(|task| task.line.is_some() || task.technology.is_some())
            .count() as u32,
        task_slots: row.tasks.len() as u32,
        unlocked: row
            .unlocked
            .iter()
            .map(|&token| token_of(lexical, token))
            .collect(),
        planned: row
            .queued
            .iter()
            .map(|&token| token_of(lexical, token))
            .collect(),
        queue_index: row.state.queue_index,
        policy_token: (row.state.policy != UNSELECTED_POLICY)
            .then(|| token_of(lexical, row.state.policy)),
        policy_name: None,
        cooldown_hour: row.state.cooldown_hour,
        auto_update_designs: row.state.upgrades != 0,
    }
}

/// An organisation's name as the game shows it; unknown without its owner or
/// content, or where it needs runtime scope.
pub fn organisation_name(raw: &RawOrganisation, texts: Option<&dyn Texts>) -> Option<String> {
    if raw.owner_unknown {
        return None;
    }
    let key = if raw.name_key.is_empty() {
        &raw.token
    } else {
        &raw.name_key
    };
    texts.and_then(|texts| equipment::localise_for(texts, raw.owner_tag.as_deref(), key))
}

/// An organisation's icon: its stored sprite, else `GFX_<owner>_<key>`, `GFX_<key>`,
/// then the game's unknown-organisation sprite; the first the content declares.
pub fn organisation_icon(raw: &RawOrganisation, exists: impl Fn(&str) -> bool) -> Option<String> {
    organisation_icon_with_token(raw, true, exists)
        .ok()
        .flatten()
}

/// The same icon selector, with a token-read witness. An explicit stored
/// sprite needs neither that token nor the owner's identity.
pub fn organisation_icon_with_token(
    raw: &RawOrganisation,
    token_known: bool,
    exists: impl Fn(&str) -> bool,
) -> Result<Option<String>, &'static str> {
    if !raw.icon.is_empty() && exists(&raw.icon) {
        return Ok(Some(raw.icon.clone()));
    }
    if raw.owner_unknown {
        return Err("the recorded owner identity is unavailable for fallback icon selection");
    }
    if !token_known {
        return Err(
            "LexicalTokens does not spell the recorded organisation token for fallback icon selection",
        );
    }
    let tagged = raw
        .owner_tag
        .as_ref()
        .map(|tag| format!("GFX_{tag}_{}", raw.token));
    Ok([
        tagged,
        Some(format!("GFX_{}", raw.token)),
        Some(UNKNOWN_ORGANISATION_ICON.to_owned()),
    ]
    .into_iter()
    .flatten()
    .find(|name| exists(name)))
}

/// An organisation's icon as one operation through `shown`, the catalogue its reply shows
/// ([`catalogue::draw`]): the sprite [`organisation_icon`] chooses by what the drawing catalogue
/// declares and its picture at `edge`, with the picture's key ([`picture_key`]), kept with it. When the shown catalogue cannot draw
/// its choice, the recording's own choice is drawn, named. `None` when no candidate is declared.
pub fn organisation_picture(
    shown: &Catalogue,
    raw: &RawOrganisation,
    edge: u32,
) -> catalogue::Drawn<Result<Option<(String, String)>, String>> {
    catalogue::draw(
        || format!("organisation {} icon", raw.token),
        shown,
        |cat| {
            let Some(name) = organisation_icon(raw, declared_in(cat)) else {
                return Ok(None);
            };
            let icon = Icon::Sprite(name);
            let image = super::equipment_icon::image(cat, &icon, edge)
                .map_err(|error| error.to_string())?;
            Ok(Some((picture_key(&icon_key(&icon), Some(&image)), image)))
        },
    )
}

/// The asset key of the sprite `shown` chooses for an organisation, drawn or not.
pub fn organisation_asset(shown: &Catalogue, raw: &RawOrganisation) -> Option<String> {
    organisation_icon(raw, declared_in(shown)).map(|name| icon_key(&Icon::Sprite(name)))
}

/// Whether `cat` declares a sprite, refused declarations included, as the icon selector asks.
fn declared_in(cat: &Catalogue) -> impl Fn(&str) -> bool + '_ {
    move |name| cat.sprites.contains(name) || cat.sprites.refusal(name).is_some()
}

/// The produced variants' names and icons by hour, merged from the variant catalogue
/// (names, and where they are known) and the line reads (icons), each icon's key the one
/// `shown` gives its choice (choosing and drawing it), or why it has none. Names are known
/// only from their first definition read onwards: an earlier hour is left out (unknown),
/// never given a later read's name; an hour the catalogue no longer knows them is an entry
/// with `known` false. Returns the entries, how many produced variants were never defined,
/// and the reasons icons were unknown.
pub fn variant_entries(
    scanned: &ScannedLines,
    texts: Option<&dyn Texts>,
    has_content: bool,
    shown: &mut dyn FnMut(&IconChoice) -> Result<Option<String>, String>,
) -> (Vec<VariantName>, usize, BTreeSet<String>) {
    let used: BTreeSet<u64> = scanned
        .lines
        .iter()
        .flat_map(|line| {
            let parts = line
                .output
                .iter()
                .chain(&line.final_output)
                .flat_map(|output| output.parts.iter().map(|part| part.variant));
            line.runs
                .iter()
                .map(|run| run.variant)
                .chain(line.variants.iter().copied())
                .chain(parts)
        })
        .collect();
    let mut variants = Vec::new();
    let mut unnamed = 0;
    let mut unknown_icons = BTreeSet::new();
    for id in &used {
        let versions = scanned
            .variants
            .versions
            .get(id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if versions.is_empty() {
            unnamed += 1;
            continue;
        }
        let icons_at: &[IconAt] = scanned.icons.get(id).map_or(&[], Vec::as_slice);
        let mut hours: Vec<i32> = versions
            .iter()
            .map(|version| version.first_hour)
            .chain(icons_at.iter().map(|(hour, _)| *hour))
            .collect();
        hours.sort_unstable();
        hours.dedup();
        let token = scanned.variants.token(*id);
        let mut previous: Option<VariantName> = None;
        for hour in hours {
            // Nothing before the first read: unknown, never a later read's name.
            let Some(version) = versions
                .iter()
                .rev()
                .find(|version| version.first_hour <= hour)
            else {
                continue;
            };
            let entry = match &version.raw {
                None => VariantName {
                    id: id.to_string(),
                    first_hour: hour,
                    known: false,
                    token: token.to_owned(),
                    name: None,
                    type_name: None,
                    icon: None,
                    icon_unknown: Some("The variant is not recorded at this time.".to_owned()),
                },
                Some(raw) => {
                    let (name, type_name) = equipment::variant_names(raw, texts);
                    let icon = icons_at
                        .iter()
                        .rev()
                        .find(|(at, _)| *at <= hour)
                        .map(|(_, icon)| icon);
                    let (icon, icon_unknown) = match icon.map(|choice| shown(choice)) {
                        Some(Ok(Some(key))) => (Some(key), None),
                        Some(Ok(None)) => (None, None),
                        Some(Err(reason)) => {
                            unknown_icons.insert(reason.clone());
                            (None, Some(reason.clone()))
                        }
                        None if !has_content => (
                            None,
                            Some("Equipment icons need the game folders.".to_owned()),
                        ),
                        None => (None, Some("Not read at this time.".to_owned())),
                    };
                    VariantName {
                        id: id.to_string(),
                        first_hour: hour,
                        known: true,
                        token: raw.token.clone(),
                        name,
                        type_name,
                        icon,
                        icon_unknown,
                    }
                }
            };
            if previous.as_ref().is_some_and(|previous| {
                VariantName {
                    first_hour: hour,
                    ..previous.clone()
                } == entry
            }) {
                continue;
            }
            previous = Some(entry.clone());
            variants.push(entry);
        }
    }
    (variants, unnamed, unknown_icons)
}

/// The variants' icons drawn for one selection, by their choices, with each icon's key and
/// picture; one per kind of catalogue ([`catalogue::ByCatalogue`]).
#[derive(Default)]
struct VariantArt(Option<VariantIcons>);

type VariantIcons = catalogue::Redrawn<IconChoice, Option<(String, String)>>;

/// The key of the icon a variant's `choice` shows in the selection `drawn` is bound to, its
/// picture kept in `images` under that key: the choice's questions answered and its icon drawn
/// as one operation ([`catalogue::present`]), so where the selection cannot draw it, the
/// recording's own choice is drawn and named.
fn draw_variant_icon(
    drawn: &mut VariantIcons,
    images: &mut BTreeMap<String, Option<String>>,
    choice: &IconChoice,
) -> Result<Option<String>, String> {
    let shown = drawn.get(
        choice.clone(),
        || "production line icon".to_owned(),
        |catalogue| {
            let Some(icon) = choice.shown(catalogue)? else {
                return Ok(None);
            };
            let asset = icon_key(&icon);
            let image = icon_image(catalogue, &icon)
                .map_err(|error| format!("{asset}: picture unavailable: {error}"))?;
            Ok(Some((picture_key(&asset, Some(&image)), image)))
        },
    );
    match shown {
        Ok(Some((key, image))) => {
            images
                .entry(key.clone())
                .or_insert_with(|| Some(image.clone()));
            Ok(Some(key.clone()))
        }
        Ok(None) => Ok(None),
        Err(reason) => Err(reason.clone()),
    }
}

/// An organisation's picture key and picture as `catalogue` shows it ([`organisation_shown`]),
/// kept per catalogue in the product's cache by what the choice reads, so a memo hit names what
/// its first draw named; a picture neither the selection nor the recording can draw is warned
/// when it is first drawn.
fn kept_organisation_picture(
    state: &std::sync::Mutex<Cache>,
    catalogue: &Arc<Catalogue>,
    raw: &RawOrganisation,
    warnings: &mut Vec<String>,
) -> Option<(String, Option<String>)> {
    let key = OrganisationIcon::of(raw);
    let kept = state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .organisations
        .as_ref()
        .filter(|(held, _)| Arc::ptr_eq(held, catalogue))
        .and_then(|(_, drawn)| drawn.get(&key).cloned());
    let (shown, first) = match kept {
        Some(drawn) => (organisation_shown(&drawn, catalogue, raw), false),
        None => {
            let drawn = organisation_picture(catalogue, raw, ICON_EDGE);
            let shown = organisation_shown(&drawn, catalogue, raw);
            let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
            match &mut state.organisations {
                Some((held, kept)) if Arc::ptr_eq(held, catalogue) => {
                    kept.insert(key, drawn);
                }
                slot => *slot = Some((Arc::clone(catalogue), [(key, drawn)].into())),
            }
            (shown, true)
        }
    };
    let (key, image) = shown?;
    Some((
        key.clone(),
        image
            .map_err(|error| {
                if first {
                    warnings.push(format!("{key}: picture unavailable: {error}"));
                }
            })
            .ok(),
    ))
}

/// What an organisation icon [`organisation_picture`] drew shows, its fallbacks credited to the
/// reply: its picture's key and the picture; when neither the selection nor the recording can
/// draw it, the key of the shown choice without a picture (`{asset}#none`) and why. `None` when
/// no candidate is declared.
pub fn organisation_shown(
    drawn: &catalogue::Drawn<Result<Option<(String, String)>, String>>,
    shown: &Catalogue,
    raw: &RawOrganisation,
) -> Option<(String, Result<String, String>)> {
    match drawn.get() {
        Ok(Some((key, image))) => Some((key.clone(), Ok(image.clone()))),
        Ok(None) => None,
        Err(error) => organisation_asset(shown, raw)
            .map(|asset| (picture_key(&asset, None), Err(error.clone()))),
    }
}

/// The key a reply's picture is shared under: the asset it shows (`icon_key`) and a digest of
/// its pixels, so two elements share a key only when they show the same picture. The same
/// asset drawn from the selection and, where the selection could not draw a choice, from the
/// recording are two pictures under two keys.
pub fn picture_key(asset: &str, image: Option<&str>) -> String {
    match image {
        Some(image) => format!("{asset}#{}", blake3::hash(image.as_bytes()).to_hex()),
        None => format!("{asset}#none"),
    }
}

/// The key an icon's asset is known by: its kind and name.
fn icon_key(icon: &Icon) -> String {
    match icon {
        Icon::Sprite(name) => format!("sprite:{name}"),
        Icon::Path(path) => format!("path:{path}"),
    }
}

fn icon_image(catalogue: &Catalogue, icon: &Icon) -> io::Result<String> {
    super::equipment_icon::image(catalogue, icon, ICON_EDGE)
}

/// Checks the presentation inputs the product can enumerate without selecting a
/// recorded variant or organisation. Their runtime operands stay explicit.
pub fn check_content(
    catalogue: Arc<Catalogue>,
    equipment: Arc<installed_content_core::equipment::Database>,
) -> io::Result<(
    Vec<crate::findings::Finding>,
    Vec<installed_content_core::Diagnostic>,
)> {
    use crate::findings::{Finding, Kind};
    let content = IconContent::with_equipment(Arc::clone(&catalogue), Some(equipment))?;
    let diagnostics = content.preparation_warnings();
    let mut findings = Vec::new();
    for prepared in content.authored_icons()? {
        let subject = format!("production equipment {}", prepared.token);
        if equipment::localise_for(catalogue.as_ref(), None, &prepared.token).is_none() {
            findings.push(Finding::grouped(
                Kind::Fallback,
                &subject,
                "production-line equipment has no usable base name; its token is shown",
                format!(
                    "{}: production-line equipment has no usable base name; the token is shown",
                    prepared.token
                ),
            ));
        }
        for reason in prepared.refusals {
            findings.push(Finding::new(Kind::Refused, &subject, reason));
        }
        for icon in prepared.icons {
            if let Err(error) = icon_image(&catalogue, &icon) {
                findings.push(Finding::grouped(
                    Kind::Fallback,
                    &subject,
                    "production-line picture is unavailable; a placeholder is shown",
                    format!(
                        "{}: picture unavailable: {error}; a placeholder is shown",
                        icon_key(&icon)
                    ),
                ));
            }
        }
        findings.extend(catalogue.take_findings().into_iter().map(|mut finding| {
            finding.subject = subject.clone();
            finding
        }));
        findings.push(Finding::new(Kind::Offline, subject,
            "production variant names and selected icons need recorded variant/country/appearance/faction/capital operands; unavailable offline"));
    }
    findings.push(Finding::new(Kind::Offline, "industrial organisations",
        "organisation name keys, stored sprites and owners come from recorded IndustrialOrganisations rows; their inventory is unavailable offline"));
    Ok((findings, diagnostics))
}

/// What an organisation's icon choice reads ([`organisation_icon`]): its stored sprite, its
/// owner and its token.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OrganisationIcon {
    icon: String,
    owner_tag: Option<String>,
    owner_unknown: bool,
    token: String,
}

impl OrganisationIcon {
    fn of(raw: &RawOrganisation) -> Self {
        Self {
            icon: raw.icon.clone(),
            owner_tag: raw.owner_tag.clone(),
            owner_unknown: raw.owner_unknown,
            token: raw.token.clone(),
        }
    }
}

/// Organisation pictures drawn through one catalogue (held, so its identity stays unique), by
/// what their choice reads.
type OrganisationPictures = (
    Arc<Catalogue>,
    BTreeMap<OrganisationIcon, catalogue::Drawn<Result<Option<(String, String)>, String>>>,
);

#[derive(Default)]
struct Cache {
    icons: Option<Arc<IconContent>>,
    /// Organisation pictures of the catalogue they were drawn through: a request for another
    /// catalogue never reads them, and a draw finishing after the content changed is stored
    /// only under its own catalogue, never served to another.
    organisations: Option<OrganisationPictures>,
}

/// A line's steps: its runs joined while the variant and factories stay the same
/// over consecutive hours of one stretch.
pub fn line_steps(runs: &[RawRun]) -> Vec<LineStep> {
    let mut steps: Vec<LineStep> = Vec::new();
    let mut previous: Option<&RawRun> = None;
    for run in runs {
        let joins = previous.is_some_and(|previous| {
            !run.starts_segment
                && previous.last_hour.checked_add(1) == Some(run.first_hour)
                && previous.variant == run.variant
                && previous.active_factories == run.active_factories
                && previous.requested_factories == run.requested_factories
        });
        match steps.last_mut() {
            Some(step) if joins => step.last_hour = run.last_hour,
            _ => steps.push(LineStep {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                starts_segment: run.starts_segment,
                variant: run.variant.to_string(),
                active_factories: run.active_factories,
                requested_factories: run.requested_factories,
            }),
        }
        previous = Some(run);
    }
    steps
}

/// What `production-lines` sends of a line: its life, steps and final count.
pub fn line_history(line: &ScannedLine) -> LineHistory {
    LineHistory {
        key: line.key.clone(),
        handle: line.handle.to_string(),
        kind: line.kind,
        steps: line_steps(&line.runs),
        final_output: line.final_output.as_ref().map(OutputRun::from),
        ended_by: line.ended_by,
        before_recording: line.before_recording,
        between_reads: line.between_reads,
        variants: line.variants.iter().map(u64::to_string).collect(),
    }
}

/// The hours of a detail window, both included; none past the hours an `i32` holds.
pub fn detail_window(window: i32) -> Option<(i32, i32)> {
    let first = window.checked_mul(DETAIL_HOURS)?;
    Some((first, first.checked_add(DETAIL_HOURS - 1)?))
}

/// Every line's runs and output reads that hold any hour from `first` to `last`,
/// whole (a read before the window that holds into it is sent as it was read).
/// Modifier trees stay out: they grow with every source of every factor.
pub fn detail_lines(scanned: &ScannedLines, first: i32, last: i32) -> Vec<LineDetail> {
    let overlaps = |from: i32, to: i32| from <= last && to >= first;
    scanned
        .lines
        .iter()
        .filter_map(|line| {
            let mut runs = Vec::new();
            for run in &line.runs {
                if overlaps(run.first_hour, run.last_hour) {
                    append_design_runs(
                        &mut runs,
                        run,
                        scanned
                            .designs
                            .get(&run.variant)
                            .map(Vec::as_slice)
                            .unwrap_or_default(),
                        first,
                        last,
                    );
                }
            }
            let output: Vec<OutputRun> = line
                .output
                .iter()
                .filter(|output| overlaps(output.first_hour, output.last_hour))
                .map(OutputRun::from)
                .collect();
            let demanded: BTreeSet<_> = line
                .runs
                .iter()
                .filter(|run| overlaps(run.first_hour, run.last_hour))
                .map(|run| run.mio)
                .filter(|&id| id != 0)
                .collect();
            let organisations = demanded
                .into_iter()
                .flat_map(|id| {
                    let versions = scanned
                        .organisations
                        .get(&id)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    versions
                        .iter()
                        .enumerate()
                        .filter(move |(index, raw)| {
                            raw.first_hour <= last
                                && versions
                                    .get(index + 1)
                                    .is_none_or(|next| next.first_hour > first)
                        })
                        .map(move |(_, raw)| OrganisationStateRead {
                            id: id.to_string(),
                            first_hour: raw.first_hour,
                            state: raw.state.clone(),
                        })
                })
                .collect();
            (!runs.is_empty() || !output.is_empty()).then(|| LineDetail {
                organisations,
                key: line.key.clone(),
                runs,
                output,
            })
        })
        .collect()
}

/// Refuses a reply past [`MAX_REPLY_BYTES`], measured before it is sent, with what
/// to do instead.
fn bounded<T: Serialize>(reply: T, what: &str, instead: &str) -> io::Result<T> {
    super::check_reply_budget(&reply).map_err(|_| {
        io::Error::other(format!(
            "{what} takes more than one reply holds ({} MiB); {instead}",
            MAX_REPLY_BYTES / (1024 * 1024)
        ))
    })?;
    Ok(reply)
}

fn cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Production request cancelled")
}

/// The catalogue a scan names with, and the icon rule's content (loaded once per
/// catalogue), with their warnings.
fn scan_content(
    source: &Source,
    context: &LoadContext,
    warnings: &mut Vec<String>,
) -> (Option<Arc<Catalogue>>, Option<Arc<IconContent>>) {
    let state = source.state::<Cache>();
    let catalogue = match context
        .content()
        .and_then(|content| catalogue::recorded_catalogue(source, content))
    {
        Ok(catalogue) => {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            Some(catalogue)
        }
        Err(error) => {
            warnings.push(format!(
                "Equipment names and icons are unavailable: {error}"
            ));
            None
        }
    };
    let icons: Option<Arc<IconContent>> = match &catalogue {
        None => None,
        Some(catalogue) => {
            let cached = {
                let cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.icons.clone().filter(|icons| {
                    icons
                        .catalogue
                        .as_ref()
                        .is_some_and(|cached| Arc::ptr_eq(cached, catalogue))
                })
            };
            match cached {
                Some(icons) => Some(icons),
                None => match IconContent::load(Arc::clone(catalogue)) {
                    Ok(icons) => {
                        let icons = Arc::new(icons);
                        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                        cache.icons = Some(Arc::clone(&icons));
                        Some(icons)
                    }
                    Err(error) => {
                        warnings.push(format!("Equipment icons are unavailable: {error}"));
                        None
                    }
                },
            }
        }
    };
    if let Some(icons) = &icons {
        warnings.extend(icons.warnings());
    }
    (catalogue, icons)
}

pub fn preparation_icons(source: &Source, context: &LoadContext) -> Option<Arc<IconContent>> {
    scan_content(source, context, &mut Vec::new()).1
}

/// Whether the prepared icon choices are those the icon content would make now: icon
/// content loaded alike, with the same inputs outside the recorded content's hash.
/// Which sprites and files exist is not among them: each reply answers that
/// ([`IconChoice::shown`]).
fn pictures_hold_with(icons: Option<&IconContent>, pictured: bool, visual: Option<&str>) -> bool {
    match icons {
        None => !pictured,
        Some(icons) => pictured && visual == Some(icons.visual_inputs()),
    }
}

/// A branch's last block: its first and last hour and its offset.
fn last_block(replay: &Replay, branch: u64) -> Option<(i32, i32, u64)> {
    branch_blocks(replay, branch)
        .last()
        .map(|block| (block.first_hour, block.last_hour, block.offset))
}

fn scanned_lines(
    source: &Source,
    params: &CountryParams,
    holder: Option<prepared::NativeCountry>,
    context: &LoadContext,
    cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<(Option<Arc<Catalogue>>, Arc<ScannedLines>)> {
    let (catalogue, scanned, approximate) =
        scanned_view(source, params, holder, context, cancelled, warnings)?;
    if let Some(span) = approximate.first() {
        warnings.push(format!(
            "Production history from hour {} is still being prepared; missing reads are unknown.",
            span.first_hour
        ));
    }
    Ok((catalogue, scanned))
}

type ScannedView = (
    Option<Arc<Catalogue>>,
    Arc<ScannedLines>,
    Vec<super::country_groups::HourSpan>,
);

fn scanned_view(
    source: &Source,
    params: &CountryParams,
    holder: Option<prepared::NativeCountry>,
    context: &LoadContext,
    cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<ScannedView> {
    let (catalogue, icons) = scan_content(source, context, warnings);
    #[cfg(test)]
    let reference = reference::selected(source, holder.as_ref());
    #[cfg(not(test))]
    let reference: Option<ScannedLines> = None;
    let (scanned, coverage) = match reference {
        Some(scanned) => (
            scanned,
            crate::prepared::coverage::Coverage {
                schema: crate::prepared::coverage::SCHEMA,
                first_hour: branch_blocks(source.replay_ref(), params.branch())
                    .first()
                    .unwrap()
                    .first_hour,
                last_hour: branch_blocks(source.replay_ref(), params.branch())
                    .last()
                    .unwrap()
                    .last_hour,
                complete: true,
            },
        ),
        None => prepared::query(
            source,
            context,
            params.branch(),
            &params.tag,
            holder,
            icons.as_deref(),
            cancelled,
        )?,
    };
    let approximate = if coverage.complete {
        Vec::new()
    } else {
        let last = branch_blocks(source.replay_ref(), params.branch())
            .last()
            .map_or(coverage.last_hour, |block| block.last_hour);
        coverage
            .last_hour
            .checked_add(1)
            .filter(|first| *first <= last)
            .map(|first_hour| {
                vec![super::country_groups::HourSpan {
                    first_hour,
                    last_hour: last,
                }]
            })
            .unwrap_or_default()
    };
    warnings.extend(
        scanned
            .modifier_layout_refusals
            .iter()
            .map(|(reason, count)| format!("{reason} ({count} supplying reads)")),
    );
    Ok((catalogue, Arc::new(scanned), approximate))
}

impl Product for ProductionLines {
    const KEY: &'static str = "production-lines";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionLinesParams;
    type Reply = ProductionLinesReply;

    fn types() -> Vec<String> {
        vec![
            LineKind::decl(),
            LineStep::decl(),
            OutputPart::decl(),
            OutputRun::decl(),
            LineHistory::decl(),
            VariantName::decl(),
            OrganisationName::decl(),
            OrganisationState::decl(),
            EquipmentIcon::decl(),
            ProductionLinesParams::decl(),
            ProductionLinesReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.country().validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let mut warnings = Vec::new();
        let (catalogue, scanned, approximate) = scanned_view(
            source,
            &params.country(),
            None,
            context,
            cancelled,
            &mut warnings,
        )?;
        if params.exact == Some(true) && !approximate.is_empty() {
            return Err(crate::invalid("Production lines are still being prepared"));
        }
        if !approximate.is_empty() {
            warnings.push(format!(
                "{}'s production history after the published prefix is still being prepared.",
                params.tag
            ));
        }
        let texts = catalogue
            .as_ref()
            .map(|catalogue| catalogue.as_ref() as &dyn Texts);
        // The scan keeps each variant's icon as a choice with its artwork questions open
        // (kept with prepared scans too); this reply answers them, and draws the icon, as the
        // request's selection shows it.
        let pictured = catalogue.as_ref().map(|_| {
            context
                .content()
                .and_then(|content| catalogue::catalogue(source, content))
        });
        let mut images: BTreeMap<String, Option<String>> = BTreeMap::new();
        let (variants, unnamed, unknown_icons) = {
            let art = source.state::<catalogue::ByCatalogue<VariantArt>>();
            let mut art = art.lock().unwrap_or_else(|error| error.into_inner());
            let mut drawn = match (&pictured, &catalogue) {
                (Some(Ok(admitted)), Some(recorded)) => Ok(catalogue::Redrawn::bind(
                    &mut art.of(admitted).0,
                    admitted,
                    recorded,
                )),
                (Some(Err(error)), _) => {
                    Err(format!("Equipment pictures are unavailable: {error}"))
                }
                _ => Err("Equipment icons need the game folders.".to_owned()),
            };
            variant_entries(
                &scanned,
                texts,
                catalogue.is_some(),
                &mut |choice: &IconChoice| {
                    if cancelled.load(Ordering::Acquire) {
                        return Err("cancelled".to_owned());
                    }
                    let drawn = drawn.as_mut().map_err(|why| why.clone())?;
                    draw_variant_icon(drawn, &mut images, choice)
                },
            )
        };
        if cancelled.load(Ordering::Acquire) {
            return Err(cancelled_error());
        }
        warnings.extend(scanned.variants.warnings());
        let pictured = pictured.and_then(Result::ok);
        // Organisations: each one's choice of sprite and its picture drawn as one operation from
        // the selection, kept per catalogue, its picture under the picture's key.
        let mut organisations = Vec::new();
        let state = source.state::<Cache>();
        for (id, versions) in &scanned.organisations {
            let mut previous: Option<OrganisationName> = None;
            for raw in versions {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancelled_error());
                }
                let icon = pictured.as_ref().and_then(|catalogue| {
                    let (key, image) =
                        kept_organisation_picture(&state, catalogue, raw, &mut warnings)?;
                    images.entry(key.clone()).or_insert(image);
                    Some(key)
                });
                let entry = OrganisationName {
                    id: id.to_string(),
                    first_hour: raw.first_hour,
                    token: raw.token.clone(),
                    name: organisation_name(raw, texts),
                    name_unknown: raw.owner_unknown.then(|| "The organisation's owner is unresolved at this read; its country-specific name is unknown.".to_owned()),
                    icon_unknown: (raw.owner_unknown && icon.is_none()).then(|| "The organisation's owner is unresolved at this read; its country-specific icon is unknown.".to_owned()),
                    icon,
                };
                if previous.as_ref().is_none_or(|old| {
                    OrganisationName {
                        first_hour: entry.first_hour,
                        ..old.clone()
                    } != entry
                }) {
                    previous = Some(entry.clone());
                    organisations.push(entry);
                }
            }
        }
        let icons_out: Vec<EquipmentIcon> = images
            .into_iter()
            .map(|(key, image)| EquipmentIcon { key, image })
            .collect();
        if !icons_out.is_empty() {
            warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
        }
        for reason in unknown_icons {
            warnings.push(format!("Some equipment icons are unknown: {reason}"));
        }
        if unnamed > 0 {
            warnings.push(format!("{unnamed} produced variants have no recorded definition in this branch; they are shown by id."));
        }
        if scanned.unknown_kinds > 0 {
            warnings.push(format!(
                "{} recorded line reads are of an unknown native class; they are left out.",
                scanned.unknown_kinds
            ));
        }
        if scanned.unlisted_output > 0 {
            warnings.push(format!(
                "{} output reads name a line no census listed without its final count; they are left out.",
                scanned.unlisted_output
            ));
        }
        if scanned.unread_output_reads > 0 || scanned.unreadable_stretches > 0 {
            let stretches = match scanned.unreadable_stretches {
                0 => String::new(),
                count => format!(", nor {count} stretches of the recording"),
            };
            warnings.push(format!(
                "{} daily production output reads of {} could not be read{stretches}, so lines made and ended then may be missing and their final counts are not recorded; the history marks those hours.",
                scanned.unread_output_reads, params.tag
            ));
        }
        if scanned.unlinked_rows > 0 {
            warnings.push(format!(
                "{} production rows name a producer the recording does not link to a country; those naming a line of {} are kept with counts that are not exact, the others are left out.",
                scanned.unlinked_rows, params.tag
            ));
        }
        // A first view without reads says nothing of the hours before it.
        if scanned.reads.is_empty() && approximate.is_empty() {
            warnings.push(format!(
                "{}: no recorded production lines in this branch",
                params.tag
            ));
        }
        if let Some(catalogue) = &catalogue {
            warnings.extend(catalogue.refusal_lines());
        }
        bounded(
            ProductionLinesReply {
                branch_id: params.branch_id.clone(),
                tag: params.tag.clone(),
                decimals: EQUIPMENT_DECIMALS,
                lines: scanned.lines.iter().map(line_history).collect(),
                variants,
                organisations,
                icons: icons_out,
                reads: scanned.reads.clone(),
                output_hours: scanned.output_hours.clone(),
                line_hours: scanned.line_hours.clone(),
                organisation_hours: scanned.organisation_hours.clone(),
                unread_output: scanned.unread_output.clone(),
                approximate,
                detail_hours: DETAIL_HOURS,
                warnings,
            },
            "This country's production history",
            "its lines cannot be shown.",
        )
    }
}

impl Product for ProductionLineDetail {
    const KEY: &'static str = "production-line-detail";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionDetailParams;
    type Reply = ProductionLineDetailReply;

    fn types() -> Vec<String> {
        vec![
            LineRun::decl(),
            LineDetail::decl(),
            OrganisationStateRead::decl(),
            ProductionDetailParams::decl(),
            ProductionLineDetailReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if detail_window(params.window).is_none() {
            return Err("Invalid production window");
        }
        CountryParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
        }
        .validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let country = CountryParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
        };
        let mut warnings = Vec::new();
        let (_, scanned) =
            scanned_lines(source, &country, None, context, cancelled, &mut warnings)?;
        let (first_hour, last_hour) = detail_window(params.window).expect("validated window");
        let lines = detail_lines(&scanned, first_hour, last_hour);
        warnings.sort();
        warnings.dedup();
        bounded(
            ProductionLineDetailReply {
                warnings,
                branch_id: params.branch_id.clone(),
                tag: params.tag.clone(),
                decimals: EQUIPMENT_DECIMALS,
                window: params.window,
                first_hour,
                last_hour,
                lines,
            },
            "This month of production lines",
            "its values cannot be shown.",
        )
    }
}

#[cfg(test)]
mod tests;
