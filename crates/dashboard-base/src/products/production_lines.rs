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

use super::{
    CountryParams, LoadContext, Product,
    equipment::{self, EQUIPMENT_DECIMALS, Tags, Texts, VariantCatalogue},
    equipment_icon::{Icon, IconContent, IconHour, variant_icon},
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{TaggedCountry, link_slot, slot_read, tagged_country, validate_campaign},
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{
        IndustrialOrganisationRow, LexicalTokenRow, ProductionLineOutputRow, ProductionLineRow,
        TableId, VariantDefinitionRow, tables,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;
mod breakdown;
pub use breakdown::{
    ProductionBreakdownParams, ProductionLineBreakdown, ProductionLineBreakdownReply,
    ProductionModifierGroup, ProductionModifierPart, ProductionModifierRun,
};

pub struct ProductionLines;
pub struct ProductionLineDetail;

const CACHED_COUNTRIES: usize = 8;

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
            engine::cequipment_production_line::KIND_MILITARY => Some(Self::Military),
            engine::cequipment_production_line::KIND_NAVAL => Some(Self::Naval),
            engine::cequipment_production_line::KIND_SHIP_REFIT => Some(Self::ShipRefit),
            engine::cequipment_production_line::KIND_RAILWAY_GUN => Some(Self::RailwayGun),
            _ => None,
        }
    }
}

/// One recorded read of a line, held over consecutive hours while it stays
/// unchanged, as the scan keeps it: numbers, which the replies send as exact
/// decimal strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawPart {
    pub variant: u64,
    pub mio: u64,
    pub units: i64,
    pub exact: bool,
}

/// A line's output at one read, held until the next, as the scan keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawOutput {
    pub first_hour: i32,
    pub last_hour: i32,
    pub parts: Vec<RawPart>,
    pub since_hour: i32,
    pub whole_life: bool,
}

/// One production line's life in the branch as the scan keeps it; the replies
/// send its steps ([`LineHistory`]) and a window of its reads ([`LineDetail`]).
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// The hours one `production-line-detail` window covers.
    pub detail_hours: i32,
    pub warnings: Vec<String>,
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
}

/// A variant's icon at a read: the icon, none, or why it is unknown.
pub type IconAt = (i32, Result<Option<Icon>, String>);

#[derive(Clone, Debug, Default)]
pub struct ScannedLines {
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

/// Pushes a version when it differs from the last, whatever hour it is from.
fn version<T: Clone + PartialEq>(versions: &mut Vec<T>, value: T, same: impl Fn(&T, &T) -> bool) {
    if versions.last().is_none_or(|last| !same(last, &value)) {
        versions.push(value);
    }
}

/// A line open at the last read: where it is in `lines`.
struct Open {
    index: usize,
}

/// Scans one branch for one country's production lines, their output, and their
/// variants' and organisations' naming operands; with `icons`, each produced
/// variant's icon at each read.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    icons: Option<&IconContent>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedLines> {
    scan_holder(replay, branch, tag, icons, None, cancelled)
}

fn scan_holder(
    replay: &Replay,
    branch: u64,
    tag: &str,
    icons: Option<&IconContent>,
    holder: Option<(u32, u32)>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedLines> {
    // The variant catalogue's tables include the countries and tokens read here too.
    let mut selected = vec![
        TableId::Campaign,
        TableId::ProductionLines,
        TableId::ProductionLineOutput,
        TableId::IndustrialOrganisations,
        TableId::CountryProductionModifiers,
        TableId::ProductionLineModifiers,
        TableId::ModifierDefinitions,
    ];
    selected.extend(VariantCatalogue::TABLES);
    if icons.is_some() {
        // The icon rule's country operands.
        selected.extend([
            TableId::PoliticalParties,
            TableId::Politics,
            TableId::Factions,
            TableId::FactionMembers,
            TableId::States,
        ]);
    }
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Production request cancelled");
    let mut result = ScannedLines::default();
    let recorded_modifier_language: Arc<str> = replay
        .identity
        .language
        .strip_prefix("l_")
        .unwrap_or(&replay.identity.language)
        .into();
    let mut last: Option<((u32, u32), i32)> = None;
    let mut open: BTreeMap<u64, Open> = BTreeMap::new();
    let mut carried = false;
    // Lines open when the recording broke, kept until the next read of the same identity.
    let mut broken: BTreeMap<u64, Open> = BTreeMap::new();
    let mut last_read_identity: Option<(u32, u32)> = None;
    let mut broken_identity: Option<(u32, u32)> = None;
    let mut first_read = true;
    let mut output_open = false;
    let mut organisations_open = false;
    // The census before this one, which bounds a line made and deleted between them.
    let mut last_census: Option<i32> = None;
    let mut appearances: BTreeMap<u64, u32> = BTreeMap::new();
    // Each line's last appearance, for output published after it ended.
    let mut latest: BTreeMap<u64, usize> = BTreeMap::new();
    // The latest definition read of each variant, for its icon.
    let mut definitions: BTreeMap<u64, VariantDefinitionRow> = BTreeMap::new();
    // Produced variants whose icon was unknown for want of their definition: looked
    // up again as soon as the catalogue knows them (a line made and deleted before a
    // census can name a variant whose definition is read only later).
    let mut pending: BTreeSet<u64> = BTreeSet::new();
    // The last hour an output read was due, read or not.
    let mut last_output_due: Option<i32> = None;
    let mut last_modifier_country: Option<Arc<replay::rows::CountryProductionModifiersRow>> = None;
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                // Hours the recording holds but this scan cannot read: an output read may lie
                // in them, so they are marked, never skipped silently.
                let lost = |result: &mut ScannedLines, first: i32, last: i32| {
                    result.unreadable_stretches += 1;
                    mark_span(&mut result.unread_output, first.min(last), last);
                };
                let Ok(frames) = frames else {
                    lost(&mut result, block.first_hour, block.last_hour);
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        lost(&mut result, frame.hour, block.last_hour);
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        lost(&mut result, frame.hour, block.last_hour);
                        break;
                    }
                    let hour = frame.hour;
                    let tables = decoded
                        .complete::<tables::Countries>()
                        .ok()
                        .zip(decoded.complete::<tables::CountryLinks>().ok());
                    let country = tables.map(|(countries, links)| {
                        let selected_tag = match holder {
                            None => Some(tag),
                            Some((slot, id)) => countries
                                .iter()
                                .find(|country| country.canonical_slot == slot && country.id == id)
                                .and_then(|country| std::str::from_utf8(&country.tag).ok()),
                        };
                        selected_tag.map_or(TaggedCountry::Absent, |tag| {
                            tagged_country(countries, links, tag)
                        })
                    });
                    let identity = country.and_then(|country| {
                        country.found().map(|(country, slot)| (slot, country.id))
                    });
                    let continues = identity.is_some_and(|identity| {
                        last.is_some_and(|(seen, at)| {
                            seen == identity && at.checked_add(1) == Some(hour)
                        })
                    });
                    last = identity.map(|identity| (identity, hour));
                    // A due output read this country's lines could not be read at (its tables,
                    // its census or the output incomplete): lines made and ended since the one
                    // before published their only rows here, so they may be missing. A country
                    // that does not exist then has no lines to miss.
                    if slot_read(
                        replay.format,
                        &decoded.metrics,
                        TableId::ProductionLineOutput,
                        decoded.hour,
                    ) {
                        let read = match country {
                            Some(TaggedCountry::Absent) => true,
                            Some(TaggedCountry::Found(..)) => {
                                slot_read(
                                    replay.format,
                                    &decoded.metrics,
                                    TableId::ProductionLines,
                                    decoded.hour,
                                ) && decoded.complete::<tables::ProductionLines>().is_ok()
                                    && decoded.complete::<tables::ProductionLineOutput>().is_ok()
                            }
                            Some(TaggedCountry::Unresolved) | None => false,
                        };
                        if !read {
                            result.unread_output_reads += 1;
                            let first = last_output_due
                                .and_then(|due| due.checked_add(1))
                                .unwrap_or(hour);
                            mark_span(&mut result.unread_output, first.min(hour), hour);
                        }
                        last_output_due = Some(hour);
                    }
                    let lexical = decoded.complete::<tables::LexicalTokens>().ok();
                    // Variant names, shared with every equipment product (`VariantCatalogue`);
                    // each variant's latest definition row, for its icon.
                    result.variants.observe(replay.format, decoded);
                    if slot_read(
                        replay.format,
                        &decoded.metrics,
                        TableId::VariantDefinitions,
                        decoded.hour,
                    ) && let Ok(rows) = decoded.complete::<tables::VariantDefinitions>()
                    {
                        // Icons to look up again at this read: a used variant whose definition
                        // changed (a new icon entry, creator or origin), and a pending one the
                        // catalogue now knows.
                        let mut refresh: BTreeSet<u64> = BTreeSet::new();
                        for definition in rows {
                            let changed = definitions
                                .insert(definition.id, definition.clone())
                                .is_some_and(|before| before != *definition);
                            if changed && result.icons.contains_key(&definition.id) {
                                refresh.insert(definition.id);
                            }
                        }
                        refresh.extend(
                            pending
                                .iter()
                                .copied()
                                .filter(|id| definitions.contains_key(id)),
                        );
                        refresh.retain(|id| result.variants.current(*id).is_some());
                        if let (Some(content), Some(lexical), false) =
                            (icons, lexical, refresh.is_empty())
                        {
                            let state = IconHour::read(decoded, content);
                            for id in refresh {
                                pending.remove(&id);
                                let definition = &definitions[&id];
                                let icon = match &state {
                                    Err(error) => Err(error.to_string()),
                                    Ok(state) => variant_icon(
                                        content,
                                        state,
                                        definition,
                                        &equipment::variant_token(definition, lexical),
                                    ),
                                };
                                version(
                                    result.icons.entry(id).or_default(),
                                    (hour, icon),
                                    |a, b| a.1 == b.1,
                                );
                            }
                        }
                    }
                    let rows = decoded.complete::<tables::ProductionLines>().ok();
                    if carried && (!continues || rows.is_none()) {
                        // The recording broke: what was carried is not known past here.
                        broken = std::mem::take(&mut open);
                        broken_identity = last_read_identity;
                        carried = false;
                        output_open = false;
                        organisations_open = false;
                    }
                    let (Some(identity), Some((countries, links)), Some(rows)) =
                        (identity, tables, rows)
                    else {
                        continue;
                    };
                    let slot = identity.0;
                    if !slot_read(
                        replay.format,
                        &decoded.metrics,
                        TableId::ProductionLines,
                        decoded.hour,
                    ) {
                        // Carried: the last read still holds, but only after a read of this identity.
                        if carried {
                            mark(&mut result.line_hours, hour);
                            // Output is carried only while its table stays readable: once a
                            // tick went unobserved it is unavailable for the rest of the branch.
                            output_open &=
                                decoded.complete::<tables::ProductionLineOutput>().is_ok();
                            organisations_open &= decoded
                                .complete::<tables::IndustrialOrganisations>()
                                .is_ok();
                            if organisations_open {
                                mark(&mut result.organisation_hours, hour);
                            }
                            let modifiers_carried = decoded
                                .complete::<tables::CountryProductionModifiers>()
                                .is_ok()
                                && decoded
                                    .complete::<tables::ProductionLineModifiers>()
                                    .is_ok();
                            for line in open.values() {
                                let history = &mut result.lines[line.index];
                                if modifiers_carried
                                    && let Some(modifiers) = result
                                        .modifiers
                                        .get_mut(&history.key)
                                        .and_then(|runs| runs.last_mut())
                                    && modifiers.last_hour.checked_add(1) == Some(hour)
                                {
                                    modifiers.last_hour = hour;
                                }
                                if let Some(run) = history.runs.last_mut()
                                    && run.last_hour.checked_add(1) == Some(hour)
                                {
                                    run.last_hour = hour;
                                }
                                if output_open
                                    && let Some(output) = history.output.last_mut()
                                    && output.last_hour.checked_add(1) == Some(hour)
                                {
                                    output.last_hour = hour;
                                }
                            }
                            if output_open {
                                mark(&mut result.output_hours, hour);
                            }
                        }
                        continue;
                    }
                    result.reads.push(hour);
                    mark(&mut result.line_hours, hour);
                    let previous_read = last_census;
                    last_census = Some(hour);
                    if broken_identity != Some(identity) {
                        broken.clear();
                    }
                    last_read_identity = Some(identity);
                    let mut still = BTreeMap::new();
                    // A row whose producer link does not resolve is this country's when it is
                    // one of its current lines (a native handle names one line object);
                    // otherwise whose it is is not known. Either way it is counted.
                    let ours = |country: u32, handle: u64| match link_slot(links, country) {
                        Some(linked) => linked == slot,
                        None => open.contains_key(&handle) || broken.contains_key(&handle),
                    };
                    result.unlinked_rows += rows
                        .iter()
                        .filter(|row| link_slot(links, row.country).is_none())
                        .count();
                    let mine: Vec<&ProductionLineRow> = rows
                        .iter()
                        .filter(|row| ours(row.country, row.handle))
                        .collect();
                    let naming_tags = Tags::new(
                        countries,
                        links,
                        decoded.complete::<tables::CountryAppearance>().ok(),
                    );
                    let modifier_inputs = (slot_read(
                        replay.format,
                        &decoded.metrics,
                        TableId::CountryProductionModifiers,
                        hour,
                    ) && slot_read(
                        replay.format,
                        &decoded.metrics,
                        TableId::ProductionLineModifiers,
                        hour,
                    ))
                    .then(|| {
                        decoded
                            .complete::<tables::CountryProductionModifiers>()
                            .ok()
                            .zip(decoded.complete::<tables::ProductionLineModifiers>().ok())
                    })
                    .flatten();
                    let modifier_inputs =
                        modifier_inputs.filter(|_| match replay::fixed_layout::validate_recording(
                            decoded,
                            replay::fixed_layout::PRODUCTION_MODIFIERS,
                        ) {
                            Ok(()) => true,
                            Err(reason) => {
                                *result.modifier_layout_refusals.entry(reason).or_default() += 1;
                                false
                            }
                        });
                    let naming_organisations =
                        decoded.complete::<tables::IndustrialOrganisations>().ok();
                    let modifier_country = modifier_inputs
                        .and_then(|(countries, _)| {
                            countries
                                .binary_search_by_key(&identity.1, |country| country.country)
                                .ok()
                                .map(|index| &countries[index])
                        })
                        .map(|country| {
                            if last_modifier_country.as_deref() != Some(country) {
                                last_modifier_country = Some(Arc::new(country.clone()));
                            }
                            Arc::clone(
                                last_modifier_country
                                    .as_ref()
                                    .expect("recorded modifier country"),
                            )
                        });
                    for row in &mine {
                        let Some(kind) = LineKind::of(row.kind) else {
                            result.unknown_kinds += 1;
                            continue;
                        };
                        let (index, starts_segment) = if let Some(line) = open.remove(&row.handle) {
                            (line.index, false)
                        } else if let Some(line) = broken.remove(&row.handle) {
                            (line.index, true)
                        } else {
                            let appearance = appearances.entry(row.handle).or_default();
                            *appearance += 1;
                            result.lines.push(ScannedLine {
                                key: format!("{}#{appearance}", row.handle),
                                handle: row.handle,
                                kind: Some(kind),
                                runs: Vec::new(),
                                output: Vec::new(),
                                final_output: None,
                                ended_by: None,
                                before_recording: first_read,
                                between_reads: None,
                                variants: Vec::new(),
                            });
                            (result.lines.len() - 1, true)
                        };
                        latest.insert(row.handle, index);
                        let values = RawRun {
                            first_hour: hour,
                            last_hour: hour,
                            starts_segment,
                            variant: row.variant,
                            mio: row.mio,
                            active_factories: row.active_factories,
                            requested_factories: row.requested_factories,
                            max_factories: row.max_factories,
                            cost_raw: row.cost_raw,
                            speed_raw: row.speed_raw,
                            efficiency_raw: row.efficiency_raw,
                            priority: row.priority,
                            amount: row.amount,
                        };
                        let line = &mut result.lines[index];
                        if let Some((_, lines)) = modifier_inputs
                            && let Some(country) = modifier_country
                                .as_ref()
                                .filter(|country| country.country == row.country)
                            && let Ok(position) =
                                lines.binary_search_by_key(&row.handle, |line| line.line)
                        {
                            let operands = &lines[position];
                            if (
                                operands.country,
                                operands.variant,
                                operands.organisation,
                                operands.kind,
                                operands.active_factories,
                            ) == (
                                row.country,
                                row.variant,
                                row.mio,
                                row.kind,
                                row.active_factories,
                            ) {
                                let catalogue = icons.and_then(|icons| icons.catalogue.as_deref());
                                let organisation = naming_organisations.and_then(|rows| {
                                    rows.binary_search_by_key(&row.mio, |organisation| {
                                        organisation.organisation
                                    })
                                    .ok()
                                    .map(|index| &rows[index])
                                });
                                let organisation_name = organisation.zip(lexical).and_then(
                                    |(organisation, lexical)| {
                                        let raw = raw_organisation(
                                            organisation,
                                            hour,
                                            &naming_tags,
                                            lexical,
                                        );
                                        organisation_name(
                                            &raw,
                                            catalogue.map(|catalogue| catalogue as &dyn Texts),
                                        )
                                    },
                                );
                                result.modifiers.entry(line.key.clone()).or_default().push(
                                    breakdown::RawModifierRun {
                                        first_hour: hour,
                                        last_hour: hour,
                                        country: Arc::clone(country),
                                        line: operands.clone(),
                                        run: values,
                                        organisation: organisation_name,
                                        localisation: catalogue
                                            .map(|catalogue| Arc::clone(&catalogue.localisation)),
                                        recorded_language: Arc::clone(&recorded_modifier_language),
                                    },
                                );
                            }
                        }
                        match line.runs.last_mut() {
                            Some(run)
                                if !starts_segment
                                    && run.last_hour.checked_add(1) == Some(hour)
                                    && same_values(run, &values) =>
                            {
                                run.last_hour = hour
                            }
                            _ => {
                                line.runs.push(values);
                            }
                        }
                        still.insert(row.handle, Open { index });
                    }
                    // Lines this read no longer lists ended since the one before.
                    for line in open.values().chain(broken.values()) {
                        result.lines[line.index].ended_by = Some(hour);
                    }
                    broken.clear();
                    open = still;
                    carried = true;
                    first_read = false;
                    // The output counted up to this read, by line.
                    output_open = false;
                    let mut produced_between: BTreeSet<u64> = BTreeSet::new();
                    if let Ok(output) = decoded.complete::<tables::ProductionLineOutput>() {
                        output_open = true;
                        mark(&mut result.output_hours, hour);
                        // An output row whose producer link does not resolve is this country's
                        // when it names one of its lines; its count is then not exact, since
                        // whose it is rests on the handle alone.
                        let mut by_line: BTreeMap<u64, Vec<(&ProductionLineOutputRow, bool)>> =
                            BTreeMap::new();
                        for row in output {
                            let linked = link_slot(links, row.country);
                            if linked.is_none() {
                                result.unlinked_rows += 1;
                            }
                            let mine = match linked {
                                Some(linked) => linked == slot,
                                None => latest.contains_key(&row.line),
                            };
                            if !mine {
                                continue;
                            }
                            by_line
                                .entry(row.line)
                                .or_default()
                                .push((row, linked.is_some()));
                            // A variant a line made only between two censuses is named and pictured too.
                            produced_between.insert(row.variant);
                        }
                        for (handle, rows) in by_line {
                            let index = match latest.get(&handle) {
                                Some(&index) => index,
                                // A line no census listed was made and deleted since the last
                                // one: its final rows name its variant and producer, so it is
                                // shown between the two censuses with what it made.
                                None if rows.iter().all(|(row, _)| row.ended) => {
                                    // Made after the census before only when one was complete and
                                    // the line was observed since its creation; otherwise it ran
                                    // before the observation began, from an unknown hour.
                                    let whole_life =
                                        rows.iter().all(|(row, _)| row.observed_since_creation);
                                    let since = rows
                                        .iter()
                                        .map(|(row, _)| row.since_hour)
                                        .min()
                                        .unwrap_or(hour);
                                    let (first_hour, before) = match previous_read {
                                        Some(read) if whole_life => (read + 1, false),
                                        _ => (since.min(hour), true),
                                    };
                                    let appearance = appearances.entry(handle).or_default();
                                    *appearance += 1;
                                    result.lines.push(ScannedLine {
                                        key: format!("{handle}#{appearance}"),
                                        handle,
                                        kind: None,
                                        runs: Vec::new(),
                                        output: Vec::new(),
                                        final_output: None,
                                        ended_by: Some(hour),
                                        before_recording: before,
                                        between_reads: Some(super::country_groups::HourSpan {
                                            first_hour,
                                            last_hour: hour,
                                        }),
                                        variants: rows.iter().map(|(row, _)| row.variant).collect(),
                                    });
                                    let index = result.lines.len() - 1;
                                    latest.insert(handle, index);
                                    produced_between
                                        .extend(rows.iter().map(|(row, _)| row.variant));
                                    index
                                }
                                None => {
                                    result.unlisted_output += 1;
                                    continue;
                                }
                            };
                            let run = output_of(&rows, hour);
                            let line = &mut result.lines[index];
                            if rows.iter().any(|(row, _)| row.ended) || !open.contains_key(&handle)
                            {
                                line.final_output = Some(run);
                            } else {
                                line.output.push(run);
                            }
                        }
                    }
                    // Organisations the lines name, and the produced variants' icons at this hour.
                    organisations_open = false;
                    if let (Ok(organisations), Some(lexical)) = (
                        decoded.complete::<tables::IndustrialOrganisations>(),
                        lexical,
                    ) {
                        organisations_open = true;
                        mark(&mut result.organisation_hours, hour);
                        let named: BTreeSet<u64> = mine
                            .iter()
                            .map(|row| row.mio)
                            .filter(|mio| *mio != 0)
                            .collect();
                        for row in organisations
                            .iter()
                            .filter(|row| named.contains(&row.organisation))
                        {
                            version(
                                result.organisations.entry(row.organisation).or_default(),
                                raw_organisation(row, hour, &naming_tags, lexical),
                                |a, b| {
                                    RawOrganisation {
                                        first_hour: 0,
                                        ..a.clone()
                                    } == RawOrganisation {
                                        first_hour: 0,
                                        ..b.clone()
                                    }
                                },
                            );
                        }
                    }
                    if let (Some(content), Some(lexical)) = (icons, lexical) {
                        let produced: BTreeSet<u64> = mine
                            .iter()
                            .map(|row| row.variant)
                            .chain(produced_between)
                            .collect();
                        let state = IconHour::read(decoded, content);
                        for id in produced {
                            let icon = match (&state, definitions.get(&id)) {
                                (Err(error), _) => Err(error.to_string()),
                                (Ok(_), None) => {
                                    pending.insert(id);
                                    Err("the variant's definition is not recorded yet".to_owned())
                                }
                                // A definition carried from before a gap may have changed since.
                                (Ok(_), Some(_)) if result.variants.current(id).is_none() => {
                                    pending.insert(id);
                                    Err("the variant's definition is not recorded at this time"
                                        .to_owned())
                                }
                                (Ok(state), Some(definition)) => variant_icon(
                                    content,
                                    state,
                                    definition,
                                    &equipment::variant_token(definition, lexical),
                                ),
                            };
                            version(result.icons.entry(id).or_default(), (hour, icon), |a, b| {
                                a.1 == b.1
                            });
                        }
                    }
                }
            }
            Ok(())
        },
    )?;
    Ok(result)
}

fn raw_organisation(
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

/// The produced variants' names and icons by hour, merged from the variant catalogue
/// (names, and where they are known) and the line reads (icons), each icon added to
/// `shown`. Names are known only from their first definition read onwards: an earlier
/// hour is left out (unknown), never given a later read's name; an hour the catalogue
/// no longer knows them is an entry with `known` false. Returns the entries, how many
/// produced variants were never defined, and the reasons icons were unknown.
pub fn variant_entries(
    scanned: &ScannedLines,
    texts: Option<&dyn Texts>,
    has_content: bool,
    shown: &mut BTreeMap<String, Icon>,
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
                    let (icon, icon_unknown) = match icon {
                        Some(Ok(Some(icon))) => {
                            let key = icon_key(icon);
                            shown.insert(key.clone(), icon.clone());
                            (Some(key), None)
                        }
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

/// The key an icon is shared under in the reply.
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
    equipment: Arc<installed_content::equipment::Database>,
) -> io::Result<(Vec<crate::findings::Finding>, Vec<installed_content::Diagnostic>)> {
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

/// A scan's identity: branch, original tag, native holder and icon content.
/// The content is the `Arc` itself, kept alive by the entry, so it can never match
/// other content that reuses its address.
type ScanKey<T = IconContent> = (u64, String, Option<Arc<T>>, Option<(u32, u32)>);

fn same_scan<T>(a: &ScanKey<T>, b: &ScanKey<T>) -> bool {
    a.0 == b.0
        && a.1 == b.1
        && a.3 == b.3
        && match (&a.2, &b.2) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
}

/// Decoded pictures by key, for one catalogue (held, so its identity stays unique).
type Images<T = Catalogue> = (Arc<T>, BTreeMap<String, Option<String>>);

/// A decoded picture of `owner`'s, if one is cached.
fn cached_image<T>(
    images: &Option<Images<T>>,
    owner: &Arc<T>,
    key: &str,
) -> Option<Option<String>> {
    images
        .as_ref()
        .filter(|(held, _)| Arc::ptr_eq(held, owner))
        .and_then(|(_, images)| images.get(key).cloned())
}

/// Keeps a picture decoded from `owner`, under that owner alone: another owner's
/// pictures are replaced, never mixed with these.
fn keep_image<T>(
    images: &mut Option<Images<T>>,
    owner: &Arc<T>,
    key: String,
    image: Option<String>,
) {
    match images {
        Some((held, images)) if Arc::ptr_eq(held, owner) => {
            images.insert(key, image);
        }
        _ => *images = Some((Arc::clone(owner), [(key, image)].into())),
    }
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(ScanKey, Arc<ScannedLines>)>,
    icons: Option<Arc<IconContent>>,
    /// Pictures of the catalogue they were decoded from: a request for another
    /// catalogue never reads them, and a decode finishing after the content changed
    /// is stored only under its own catalogue, never served to another.
    images: Option<Images>,
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
            let runs: Vec<LineRun> = line
                .runs
                .iter()
                .filter(|run| overlaps(run.first_hour, run.last_hour))
                .map(LineRun::from)
                .collect();
            let output: Vec<OutputRun> = line
                .output
                .iter()
                .filter(|output| overlaps(output.first_hour, output.last_hour))
                .map(OutputRun::from)
                .collect();
            (!runs.is_empty() || !output.is_empty()).then(|| LineDetail {
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

/// The catalogue, the icon rule's content (loaded once per catalogue) and the
/// country's scan (cached by branch, tag and icon content), shared by both products
/// so a detail window reuses the scan its history was answered from.
fn scanned_lines(
    source: &Source,
    params: &CountryParams,
    holder: Option<(u32, u32)>,
    context: &LoadContext,
    cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<(Option<Arc<Catalogue>>, Arc<ScannedLines>)> {
    let state = source.state::<Cache>();
    let catalogue = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
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
    let key: ScanKey = (params.branch(), params.tag.clone(), icons.clone(), holder);
    let cached = {
        let cache = state.lock().unwrap_or_else(|error| error.into_inner());
        cache
            .entries
            .iter()
            .find(|(cached, _)| same_scan(cached, &key))
            .map(|(_, scanned)| Arc::clone(scanned))
    };
    let scanned = match cached {
        Some(scanned) => scanned,
        None => {
            let scanned = Arc::new(scan_holder(
                &source.replay(),
                params.branch(),
                &params.tag,
                icons.as_deref(),
                holder,
                cancelled,
            )?);
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache.entries.retain(|(cached, _)| !same_scan(cached, &key));
            cache.entries.push_back((key, Arc::clone(&scanned)));
            while cache.entries.len() > CACHED_COUNTRIES {
                cache.entries.pop_front();
            }
            scanned
        }
    };
    warnings.extend(
        scanned
            .modifier_layout_refusals
            .iter()
            .map(|(reason, count)| format!("{reason} ({count} supplying reads)")),
    );
    Ok((catalogue, scanned))
}

impl Product for ProductionLines {
    const KEY: &'static str = "production-lines";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
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
            EquipmentIcon::decl(),
            ProductionLinesReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let state = source.state::<Cache>();
        let mut warnings = Vec::new();
        let (catalogue, scanned) =
            scanned_lines(source, params, None, context, cancelled, &mut warnings)?;
        let texts = catalogue
            .as_ref()
            .map(|catalogue| catalogue.as_ref() as &dyn Texts);
        let mut shown: BTreeMap<String, Icon> = BTreeMap::new();
        let (variants, unnamed, unknown_icons) =
            variant_entries(&scanned, texts, catalogue.is_some(), &mut shown);
        warnings.extend(scanned.variants.warnings());
        // Organisations.
        let mut organisations = Vec::new();
        for (id, versions) in &scanned.organisations {
            for raw in versions {
                let icon = catalogue
                    .as_ref()
                    .and_then(|catalogue| {
                        organisation_icon(raw, |name| {
                            catalogue.sprites.contains(name)
                                || catalogue.sprites.refusal(name).is_some()
                        })
                    })
                    .map(|name| {
                        let icon = Icon::Sprite(name);
                        let key = icon_key(&icon);
                        shown.insert(key.clone(), icon);
                        key
                    });
                organisations.push(OrganisationName {
                    id: id.to_string(),
                    first_hour: raw.first_hour,
                    token: raw.token.clone(),
                    name: organisation_name(raw, texts),
                    name_unknown: raw.owner_unknown.then(|| "The organisation's owner is unresolved at this read; its country-specific name is unknown.".to_owned()),
                    icon_unknown: (raw.owner_unknown && icon.is_none()).then(|| "The organisation's owner is unresolved at this read; its country-specific icon is unknown.".to_owned()),
                    icon,
                });
            }
        }
        // Pictures, decoded once per catalogue.
        let mut icons_out = Vec::new();
        if let Some(catalogue) = &catalogue {
            for (key, icon) in &shown {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancelled_error());
                }
                let cached = cached_image(
                    &state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .images,
                    catalogue,
                    key,
                );
                let image = match cached {
                    Some(image) => image,
                    None => {
                        let image = match icon_image(catalogue, icon) {
                            Ok(image) => Some(image),
                            Err(error) => {
                                warnings.push(format!("{key}: picture unavailable: {error}"));
                                None
                            }
                        };
                        keep_image(
                            &mut state
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .images,
                            catalogue,
                            key.clone(),
                            image.clone(),
                        );
                        image
                    }
                };
                icons_out.push(EquipmentIcon {
                    key: key.clone(),
                    image,
                });
            }
            if !icons_out.is_empty() {
                warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
            }
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
        if scanned.reads.is_empty() {
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
