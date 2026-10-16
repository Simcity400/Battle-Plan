//! One country's equipment losses over a branch, for the Equipment losses
//! widget: a tree of cause, equipment type and exact design, each leaf a
//! running total over the branch's hours. `landSource` selects destruction
//! writers (default) or the game's Combat Log. Background preparation observes
//! every native holder once; queries select the land source and shared air and
//! non-land records. Complete
//! replies are bounded by the shared transport budget. Air uses destruction.
//!
//! Causes and their records:
//!
//! - Destruction (`EquipmentDestruction`): signed pre-minus-post fixed5
//!   removals, per raw country through CountryLinks, stable cause and full
//!   variant handle, dated by every hourly drain, including equal consecutive
//!   drains. These cover every country, divisions in or outside armies and
//!   garrisons without player/AI or orders-group gates, including deleted
//!   divisions and paradrop disruption. Negative amounts are native gain
//!   corrections. Supplemental-unavailable markers keep missing writers
//!   explicit; an incomplete signed net is a recorded subtotal, not a bound.
//!   Historical absent coverage keeps the recorded amounts and names the unknown
//!   supplemental writers, including in an empty historical drain.
//! - Combat Log land (`CombatLogEquipment`), when selected: per stored
//!   reason (0 in combat; 1, 2 and 3 the attrition the game's tooltip lists
//!   as other, in combat and in training) and the equipment recovered after
//!   combat, which is not a loss. The log keeps one entry per orders group,
//!   reason and calendar month; each signed (group, reason,
//!   month, variant) keeps the last value recorded, including after the row
//!   leaves the table (a closed month, a pruned entry, a deleted group's log),
//!   and a leaf's total is the sum of those values. A leaf changes at the hour
//!   its country's rows were read.
//! - Ships (`SunkShips`): one ship of its design per retained sunk-ship entry
//!   whose victim is the country, dated at its sinking hour. Entries have no
//!   identity; an entry counts as often as it appeared at once in one read.
//! - Convoys (`SunkConvoys`): the convoys the country lost, per month and
//!   killer; the design of a convoy is not recorded.
//! - Lend-lease (`LendLeaseEquipment`, the country as giver): equipment sunk
//!   on the way (a loss) and equipment delivered (a transfer, not a loss).
//! - Air, in both land modes: the destruction writers' nuclear, accident,
//!   strategic, mission, land-combat CAS, raid and naval-AA causes.
//! - An unrecognised destruction caller stays its own cause. Destruction
//!   mode includes all its removals. Combat Log mode includes designs the
//!   recorded content proves land or air (equipment::archetype_of): an unknown
//!   caller is not a proven Combat Log writer. Unresolved or unsupported
//!   domains remain named unknown spans and never enter the sum. This
//!   ambiguity affects only the mixed-domain cause, not known air causes.
//!   Scripted stockpile fractions use content-proven air/naval designs in log
//!   mode; their land designs are excluded with other known land writers.
//!
//! No destruction amount overlaps ship, convoy or lend-lease records. Army
//! damage removes division equipment; naval transport removes that equipment
//! separately from convoys. Attrition observes its dedicated caller rather
//! than every generic pool removal. Aircraft and occupation removals have
//! separate pools. Sunk-ship records book ships and sunk-convoy records integer
//! counts. Lend-lease moves transit pools outside these hooks, even though it
//! uses the same generic removal. Unknown callers retain the hooked pool scopes.
//!
//! The collector marks every first, load and discontinuity interval partial.
//! The first subsequent complete drain is this branch's destruction_counted_from:
//! no amount is claimed before it, and startup does not make later totals unknown.
//! From that start, a complete empty drain proves no hooked removals. Overflow,
//! refusal, lost drain or truncation withholds the table for the rest of the branch
//! with its recorded reason. Gaps after counting starts and unread frames also
//! remain unknown by name; later drains cannot recover missing running totals.
//!
//! Complete Countries and CountryLinks distinguish an absent tag from unresolved
//! identity. An absent tag cannot own a removal and adds no unknown span. Countries
//! lists allocated native objects, including dormant tags, rather than all content
//! tags (engine::ccountry::walk); release or annexation need not remove a row.
//! For a present tag, any unresolved raw owner might own its removals, so the total
//! stays an incomplete subtotal through branch end. Persistent spans merge by reason and distinct
//! count-free detail, using a keyed index. Earliest incomplete hours
//! are computed once per reply. Known signed amounts remain incomplete-subtotal leaves.
//! destructionCoverage describes each actual drain independently of earlier gaps;
//! the four Combat Log coverage states apply only in Combat Log mode.
//!
//! The Combat Log writer requires an orders group and admits its country
//! when the recorded gate operand is below 1 or its country matches a live
//! human roster record. The predicate does not inspect that record's flags.
//! CombatLogState explains missing game records; it never selects capture.
//! Each hour's land coverage is `logged`, `notLogged` (the game did not keep
//! this country's land losses: unknown, not zero), `partial { ungrouped: N }`
//! (it kept losses of divisions in armies, not of N divisions outside them),
//! or `unknown` (unread census, unresolved owners or missing writer inputs).
//! DivisionCensus is hourly: equal successful reads preserve counts, while
//! unavailable reads are unknown. Partial hours merge only for equal N.
//! Equipment loss logging also requires feature 3, checked from the recorded
//! runtime mask in SimulationFeatures. `pruneUnobserved` marks hours whose
//! daily prune may have deleted a log's last changes unseen.
//!
//! Every cause also lists the spans of hours its record was not read
//! completely (`unread`), and lend-lease the exchanges that ended after their
//! last read (`endedAfterLastRead`: the game deleted what they moved since, so
//! the total, and the leaves of their designs, are incomplete subtotals from then).
//! A retention define below 1 makes the game delete entries still changing,
//! so land causes are then unknown, named by the define.
//!
//! Names come from the recorded content stack: cause names use the game's
//! localisation keys where it names one, otherwise the key the content's
//! interface declares for the displayed text ([`InterfaceKeys`]). Writer
//! classifications the game does not name use short app text; no air-history
//! channel or chance text is guessed to name a destruction cause. Equipment
//! types come from content archetypes, and designs from recorded names or
//! their equipment's localisation. A name that does not resolve stays null
//! for the widget to show a fallback.

use dashboard_base::products::equipment::{
    Domain, StockVariantName, Texts, VariantCatalogue, archetype_of,
};
use dashboard_base::products::{CountryParams, Product};
use dashboard_base::{catalogue, source::Source};
use dashboard_core::config::LoadContext;
#[cfg(test)]
use dashboard_text::metadata::{slot_read, tagged_country};
use dashboard_text::{
    catalogue::View,
    metadata::{TaggedCountry, branch_bounds, link_slot, mapped_slot, validate_campaign},
};
#[cfg(test)]
use replay::frame::FrameDecoder;
use replay::{
    container::Replay,
    frame::DecodedFrame,
    rows::{
        CountryLinkRow, EQUIPMENT_DESTRUCTION_PARTIAL_REASON, EquipmentDestructionCause, RowCodec,
        TableId, tables,
    },
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::Ordering},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

mod census;
#[cfg(test)]
mod oracle;
pub mod prepared;
#[cfg(test)]
pub use oracle::scan;
#[cfg(test)]
use oracle::{observe, selected, selected_country};

pub struct EquipmentLosses;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LandSource {
    #[default]
    Destruction,
    CombatLog,
}

/// Recorded native holder and the date at which it was selected.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LossCountry {
    pub slot: u32,
    pub country_id: u32,
    pub origin_hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LossesParams {
    pub branch_id: String,
    pub tag: String,
    #[serde(default)]
    pub land_source: LandSource,
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub country: Option<LossCountry>,
}

impl LossesParams {
    fn country_params(&self) -> CountryParams {
        CountryParams {
            branch_id: self.branch_id.clone(),
            tag: self.tag.clone(),
        }
    }

    fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }
}

/// App labels (user decision, 6 Oct) for causes whose game text is a sentence
/// or a template with fill-ins rather than a name. The game text stays in the
/// reply's `template` for the tooltip.
fn app_label(source: LossSource) -> Option<&'static str> {
    match source {
        LossSource::SunkShips => Some("Sunk ships"),
        LossSource::SunkConvoys => Some("Sunk convoys"),
        LossSource::LendLeaseSunk => Some("Lend-lease sunk in transit"),
        LossSource::CombatLog(1) => Some("Attrition: other"),
        LossSource::CombatLog(2) => Some("Attrition: in combat"),
        LossSource::CombatLog(3) => Some("Attrition: in training"),
        LossSource::Destruction(cause) => Some(destruction_label(cause)),
        _ => None,
    }
}

/// App text for writer classifications the recorded content does not name.
fn destruction_label(cause: EquipmentDestructionCause) -> &'static str {
    use EquipmentDestructionCause::*;
    match cause {
        Unknown => "Unrecognised destruction caller",
        LandCombat => "Land combat losses",
        NavalTransport => "Naval transport losses",
        Immediate => "Immediate army damage",
        Attrition => "Attrition",
        Garrison => "Resistance attacks on garrisons",
        AirNuclear => "Nuclear air losses",
        AirAccident => "Air accidents",
        AirStrategic => "Strategic air losses",
        AirMissionAa => "Air mission losses",
        AirLandCombatCas => "Land combat anti-air",
        AirRaid => "Raid air losses",
        AirNavalAa => "Naval anti-air",
        DivisionDestroyed => "Divisions destroyed in combat",
        DivisionOverrun => "Divisions overrun",
        DivisionDeleted => "Deleted divisions",
        DivisionDisbanded => "Disconnected disbands",
        DivisionCapitulation => "Divisions lost at capitulation",
        DivisionScripted => "Scripted division removals",
        DivisionTemplateRemoved => "Removed division templates",
        ParadropDisruption => "Paradrop disruption",
        DivisionMerged => "Unreturned equipment from division merges",
        ExileEquipment => "Exile equipment reductions",
        VolunteerReturnEquipment => "Volunteer return equipment reductions",
        CapitulationEscapeEquipment => "Capitulation escape equipment reductions",
        ScriptedEquipmentFraction => "Scripted equipment reductions",
        ScriptedStockpileFraction => "Scripted stockpile reductions",
    }
}

/// Units per equipment of every amount (fixed5).
pub const AMOUNT_SCALE: i64 = 100_000;
#[cfg(test)]
const CACHED_SCANS: usize = 8;

/// Where a cause's amounts come from.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase", tag = "kind", content = "reason")]
pub enum LossSource {
    /// A Combat Log reason (0 to 3) of land equipment.
    CombatLog(u8),
    /// Land equipment recovered after combat (the Combat Log's recovered array).
    Recovered,
    SunkShips,
    SunkConvoys,
    LendLeaseSunk,
    LendLeaseSent,
    #[serde(with = "destruction_cause_export")]
    Destruction(#[ts(type = "EquipmentDestructionCause")] EquipmentDestructionCause),
}

// The recording enum stays independent of JSON/TypeScript. Its names have one
// authority: as_str() supplies both the serialized reply and generated union.
mod destruction_cause_export {
    use super::EquipmentDestructionCause;

    pub fn serialize<S: serde::Serializer>(
        cause: &EquipmentDestructionCause,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(cause.as_str())
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<EquipmentDestructionCause, D::Error> {
        let name = <String as serde::Deserialize>::deserialize(deserializer)?;
        EquipmentDestructionCause::ALL
            .into_iter()
            .find(|cause| cause.as_str() == name)
            .ok_or_else(|| {
                serde::de::Error::custom(format!("Unknown equipment destruction cause: {name}"))
            })
    }
}

fn destruction_cause_typescript() -> String {
    let names: Vec<_> = EquipmentDestructionCause::ALL
        .into_iter()
        .map(|cause| format!("\"{}\"", cause.as_str()))
        .collect();
    format!("type EquipmentDestructionCause = {};", names.join(" | "))
}

/// A cause's writer domain; mixed or unrecognised callers are Other.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LossDomain {
    Land,
    Air,
    Naval,
    Other,
}

/// A cause's localisation: the key (null when the content does not name one),
/// its text with colour codes removed (may hold the game's `$NAME$` amount
/// placeholders), and the text when it needs no substitution.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossName {
    pub key: Option<String>,
    pub template: Option<String>,
    pub text: Option<String>,
}

/// Why a cause's amounts are unknown over a span of hours.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
#[serde(rename_all = "camelCase")]
pub enum UnknownReason {
    /// The cause's record was not read completely at these hours (not
    /// recorded in this branch, a failed or truncated read, or the country
    /// not resolved): what changed then is unknown, and the running total
    /// may lag until the record is read again.
    Unread,
    /// A lend-lease exchange ended after its last read, and the game deleted
    /// it with what it moved since: the signed total is incomplete from here on.
    EndedAfterLastRead,
    DestructionWithheld,
    DestructionGap,
    DestructionUnresolved,
    DestructionDomain,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnknownSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: UnknownReason,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossCause {
    pub source: LossSource,
    pub domain: LossDomain,
    pub name: LossName,
    /// The group this cause belongs to in the game's own view (the Combat
    /// Log's attrition column for reasons 1 to 3).
    pub group: Option<LossName>,
    /// False for what is not a loss: recovered equipment and lend-lease sent.
    pub loss: bool,
    /// Why the amounts are unknown throughout, when they are (for example a
    /// retention setting that deletes entries still changing): no leaves.
    pub unknown: Option<String>,
    /// Spans of hours whose amounts are unknown, in hour order.
    pub unknown_spans: Vec<UnknownSpan>,
    pub leaves: Vec<LossLeaf>,
}

/// One design (or, when the record has none, no design) under a cause.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossLeaf {
    /// Index into [`LossesReply::designs`]; null when the record names none.
    pub design: Option<u32>,
    /// The running total at each hour it changed, in hour order.
    pub points: Vec<LossPoint>,
    /// From this hour missing destruction events or
    /// an ended lend-lease exchange make this signed total incomplete.
    pub lower_bound_from: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossPoint {
    pub hour: i32,
    /// Exact decimal string, in [`LossesReply::amount_scale`] units.
    pub total_raw: String,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossDesign {
    /// The variant's reference id.
    pub variant: u32,
    /// The design's names at their recorded reads, including where they
    /// stopped being known. Never use a later name at an earlier hour.
    pub names: Vec<StockVariantName>,
    /// Its equipment type's script token (VariantDefinitions, LexicalTokens).
    pub equipment: Option<String>,
    /// The equipment's archetype from the content stack: the type level.
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum Coverage {
    Logged,
    NotLogged,
    Partial { ungrouped: u32 },
    Unknown,
    PruneUnobserved,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoverageSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub coverage: Coverage,
}

/// Actual drain availability at an hour; earlier missing events do not recover.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DestructionCoverageSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Null is a complete drain, including an empty one.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossesReply {
    pub branch_id: String,
    pub tag: String,
    pub land_source: LandSource,
    /// Optional persistent content/capture exclusion; no exclusion with the full writer group.
    pub land_lower_bound: Option<String>,
    pub amount_scale: u32,
    pub causes: Vec<LossCause>,
    pub designs: Vec<LossDesign>,
    /// Combat Log land coverage; empty when destruction land is selected.
    pub coverage: Vec<CoverageSpan>,
    pub destruction_coverage: Vec<DestructionCoverageSpan>,
    /// The first complete drain on this branch. Startup hours before it
    /// make no claim about hooked removals and are not unknown spans.
    pub destruction_counted_from: Option<i32>,
    /// Months the game keeps a Combat Log entry (the content's define), which
    /// bounds the history a branch's first read can hold.
    pub retained_months: Option<i32>,
    pub warnings: Vec<String>,
}

/// A leaf: its cause and its design, when the record names one.
pub type LeafKey = (LossSource, Option<u32>);

/// The recorded table a cause's amounts come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LossRecord {
    CombatLog,
    SunkShips,
    SunkConvoys,
    LendLease,
}

impl LossSource {
    fn domain(self) -> LossDomain {
        match self {
            Self::CombatLog(_) | Self::Recovered => LossDomain::Land,
            Self::SunkShips | Self::SunkConvoys => LossDomain::Naval,
            Self::LendLeaseSunk | Self::LendLeaseSent => LossDomain::Other,
            Self::Destruction(
                EquipmentDestructionCause::Unknown
                | EquipmentDestructionCause::ScriptedStockpileFraction,
            ) => LossDomain::Other,
            Self::Destruction(cause) if cause.is_air() => LossDomain::Air,
            Self::Destruction(_) => LossDomain::Land,
        }
    }

    fn record(self) -> Option<LossRecord> {
        match self {
            LossSource::CombatLog(_) | LossSource::Recovered => Some(LossRecord::CombatLog),
            LossSource::SunkShips => Some(LossRecord::SunkShips),
            LossSource::SunkConvoys => Some(LossRecord::SunkConvoys),
            LossSource::LendLeaseSunk | LossSource::LendLeaseSent => Some(LossRecord::LendLease),
            LossSource::Destruction(_) => None,
        }
    }
}

/// A lend-lease exchange of the country that ended after its last read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndedExchange {
    /// The first hour after the last read that showed it.
    pub from: i32,
    /// The (kind, variant) pools that read showed.
    pub designs: BTreeSet<(u8, u32)>,
}

/// What one branch recorded of one country, before naming.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scanned {
    /// The recording's format, which dates its daily tables
    /// ([`crate::metadata::slot_read`]).
    #[serde(skip)]
    pub(crate) format: replay::format::FormatVersion,
    /// (source, variant) to (hour, delta) events, in scan order.
    #[serde(with = "dashboard_core::map_pairs")]
    pub events: BTreeMap<LeafKey, Vec<(i32, i128)>>,
    pub coverage: Vec<CoverageSpan>,
    pub destruction_coverage: Vec<DestructionCoverageSpan>,
    pub destruction_unknown: Vec<UnknownSpan>,
    #[serde(with = "dashboard_core::map_pairs")]
    destruction_unknown_index: HashMap<(UnknownReason, String), usize>,
    pub destruction_counted_from: Option<i32>,
    /// Hours each record was not read completely, as [`UnknownReason::Unread`] spans.
    pub unread: BTreeMap<LossRecord, Vec<UnknownSpan>>,
    unread_active: BTreeSet<LossRecord>,
    destruction_active: BTreeSet<usize>,
    pub ended: Vec<EndedExchange>,
    /// The branch's last recorded hour.
    pub last_hour: Option<i32>,
    /// SunkConvoys entries whose count fell, each counted as a new entry.
    pub convoy_drops: usize,
    /// Shared read-dated names and design identities of every variant.
    #[serde(skip)]
    pub variants: VariantCatalogue,
    /// Raw country references of loss rows the recorded links did not resolve.
    pub unresolved: BTreeSet<u32>,
    combat_unresolved: BTreeSet<u32>,
}

/// A (group, kind, month, variant) key of the Combat Log.
type LogKey = (u32, u8, u16, u32);
type ShipKey = (Vec<u8>, i32, u64, u64, u64, u8);

#[derive(Default, Serialize, Deserialize)]
struct State {
    land_source: LandSource,
    #[cfg(test)]
    country: Option<LossCountry>,
    #[serde(with = "dashboard_core::map_pairs")]
    log: BTreeMap<LogKey, i64>,
    #[serde(with = "dashboard_core::map_pairs")]
    convoys: BTreeMap<(u32, i32, u32), i32>,
    #[serde(with = "dashboard_core::map_pairs")]
    lend_lease: BTreeMap<(u32, u8, u32), i64>,
    /// Each exchange of the country the last complete read showed: that
    /// hour and its (kind, variant) pools.
    exchanges: BTreeMap<u32, (i32, BTreeSet<(u8, u32)>)>,
    exchanges_read: Option<i32>,
    #[serde(with = "dashboard_core::map_pairs")]
    ships: BTreeMap<ShipKey, usize>,
    #[serde(skip)]
    #[cfg(test)]
    division_census: census::Census,
    include_all_destruction: bool,
    destruction_withheld: Option<String>,
    destruction_coverage_not_recorded: bool,
}

impl Scanned {
    /// Materialise the open coverage only through authenticated, unchanged hours.
    /// Closed record spans and old loss operands are never visited.
    fn advance(&mut self, hour: i32) {
        if self.last_hour.is_none_or(|last| hour <= last) {
            return;
        }
        if let Some(last) = self.coverage.last_mut() {
            last.last_hour = hour;
        }
        if let Some(last) = self.destruction_coverage.last_mut() {
            last.last_hour = hour;
        }
        for record in &self.unread_active {
            self.unread
                .get_mut(record)
                .unwrap()
                .last_mut()
                .unwrap()
                .last_hour = hour;
        }
        for &index in &self.destruction_active {
            self.destruction_unknown[index].last_hour = hour;
        }
        self.last_hour = Some(hour);
    }

    fn event(&mut self, key: LeafKey, hour: i32, delta: impl Into<i128>) {
        let delta = delta.into();
        if delta != 0 {
            self.events.entry(key).or_default().push((hour, delta));
        }
    }

    /// Whether `record` was read completely at `hour`; an unread hour joins
    /// the record's unread spans.
    fn read(&mut self, record: LossRecord, hour: i32, complete: bool) {
        if complete {
            self.unread_active.remove(&record);
            return;
        }
        self.unread_range(record, hour, hour);
    }

    fn unread_range(&mut self, record: LossRecord, first: i32, last_hour: i32) {
        self.unread_active.insert(record);
        let spans = self.unread.entry(record).or_default();
        if let Some(last) = spans.last_mut()
            && i64::from(last.last_hour) + 1 >= i64::from(first)
        {
            last.last_hour = last.last_hour.max(last_hour);
            return;
        }
        spans.push(UnknownSpan {
            first_hour: first,
            last_hour,
            reason: UnknownReason::Unread,
            detail: None,
        });
    }

    fn cover(&mut self, hour: i32, coverage: Coverage) {
        self.cover_range(hour, hour, coverage);
    }

    fn cover_range(&mut self, first: i32, last_hour: i32, coverage: Coverage) {
        if let Some(last) = self.coverage.last_mut()
            && last.coverage == coverage
            && i64::from(last.last_hour) + 1 >= i64::from(first)
        {
            last.last_hour = last.last_hour.max(last_hour);
            return;
        }
        self.coverage.push(CoverageSpan {
            first_hour: first,
            last_hour,
            coverage,
        });
    }

    fn missing(&mut self, first: i32, last: i32, land_source: LandSource) {
        if first > last {
            return;
        }
        self.variants.forget(first);
        for record in [
            LossRecord::SunkShips,
            LossRecord::SunkConvoys,
            LossRecord::LendLease,
        ] {
            self.unread_range(record, first, last);
        }
        if land_source == LandSource::CombatLog {
            self.unread_range(LossRecord::CombatLog, first, last);
            self.cover_range(first, last, Coverage::Unknown);
        }
        if self.destruction_counted_from.is_some() {
            let reason = "Equipment destruction drain is missing in a recorded gap or unread frame";
            self.destruction_cover(first, last, Some(reason.to_owned()));
            self.destruction_unknown(first, UnknownReason::DestructionGap, reason.to_owned());
        }
        self.last_hour = Some(last);
        // The missing interval's uncertainty is dated at its first hour.
        self.destruction_active.clear();
    }

    fn destruction_cover(&mut self, first: i32, last_hour: i32, reason: Option<String>) {
        if let Some(last) = self.destruction_coverage.last_mut()
            && last.reason == reason
            && i64::from(last.last_hour) + 1 >= i64::from(first)
        {
            last.last_hour = last.last_hour.max(last_hour);
            return;
        }
        self.destruction_coverage.push(DestructionCoverageSpan {
            first_hour: first,
            last_hour,
            reason,
        });
    }

    fn destruction_unknown(&mut self, hour: i32, reason: UnknownReason, detail: String) {
        // Equal reason/details share one persistent uncertainty span. Counts
        // never enter these keys; supplemental-hook diagnostics keep their names.
        let key = (reason, detail.clone());
        if let Some(&index) = self.destruction_unknown_index.get(&key) {
            // Each missing destruction event affects the remainder of this
            // branch, so equal reason/details overlap regardless of recovery hours.
            self.destruction_unknown[index].last_hour = hour;
            self.destruction_active.insert(index);
            return;
        }
        self.destruction_active
            .insert(self.destruction_unknown.len());
        self.destruction_unknown_index
            .insert(key, self.destruction_unknown.len());
        self.destruction_unknown.push(UnknownSpan {
            first_hour: hour,
            last_hour: hour,
            reason,
            detail: Some(detail),
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn country_metadata(
    scanned: &mut Scanned,
    state: &mut State,
    decoded: &DecodedFrame,
    links: Option<&[CountryLinkRow]>,
    country: TaggedCountry<'_>,
    land: bool,
    land_coverage: Option<Coverage>,
    inputs: &DestructionInputs,
) -> io::Result<()> {
    let hour = decoded.hour;
    scanned.destruction_active.clear();
    destruction_from(scanned, state, decoded, links, country, inputs)?;
    if state.destruction_withheld.is_some() {
        scanned.destruction_active.clear();
    }
    let found = country.found().is_some() && links.is_some();
    if land {
        let complete = found && land_read(decoded);
        scanned.read(LossRecord::CombatLog, hour, complete);
        scanned.cover(
            hour,
            if complete {
                land_coverage.unwrap_or(Coverage::Unknown)
            } else {
                Coverage::Unknown
            },
        );
    }
    scanned.read(
        LossRecord::SunkShips,
        hour,
        found && matches!(decoded.complete::<tables::SunkShips>(), Ok([_])),
    );
    scanned.read(
        LossRecord::SunkConvoys,
        hour,
        found && decoded.complete::<tables::SunkConvoys>().is_ok(),
    );
    scanned.read(
        LossRecord::LendLease,
        hour,
        found && decoded.complete::<tables::LendLeaseEquipment>().is_ok(),
    );
    scanned.advance(hour);
    scanned.last_hour = Some(hour);
    Ok(())
}

fn ship_rows<'a>(
    scanned: &mut Scanned,
    state: &mut State,
    entries: impl Iterator<Item = &'a replay::rows::SunkShipEntry>,
) {
    let mut counts: BTreeMap<ShipKey, (usize, i32, u64)> = BTreeMap::new();
    for entry in entries {
        let key = (
            entry.ship_name.clone(),
            entry.sink_hour,
            entry.victim_equipment_variant,
            entry.battle,
            entry.damage_source_reference,
            entry.cause_record,
        );
        counts
            .entry(key)
            .or_insert((0, entry.sink_hour, entry.victim_equipment_variant))
            .0 += 1;
    }
    for (key, (count, sunk, variant)) in counts {
        let seen = state.ships.entry(key).or_default();
        if count > *seen {
            let added = (count - *seen) as i64;
            *seen = count;
            let variant = (variant as u32 == VARIANT_TYPE).then_some((variant >> 32) as u32);
            scanned.event((LossSource::SunkShips, variant), sunk, added * AMOUNT_SCALE);
        }
    }
}

fn convoy_rows<'a>(
    scanned: &mut Scanned,
    state: &mut State,
    hour: i32,
    rows: impl Iterator<Item = &'a replay::rows::SunkConvoyRow>,
) {
    for row in rows {
        let last = state
            .convoys
            .entry((row.owner_country, row.month, row.killer_country))
            .or_default();
        let delta = if row.convoys < *last {
            scanned.convoy_drops += 1;
            i64::from(row.convoys)
        } else {
            i64::from(row.convoys) - i64::from(*last)
        };
        *last = row.convoys;
        scanned.event((LossSource::SunkConvoys, None), hour, delta * AMOUNT_SCALE);
    }
}

fn lend_lease_rows<'a>(
    scanned: &mut Scanned,
    state: &mut State,
    hour: i32,
    rows: impl Iterator<Item = &'a replay::rows::LendLeaseEquipmentRow> + Clone,
) {
    // The game deletes an ended exchange with its totals: an exchange
    // the last complete read showed and this one does not ended after
    // that read, and what it moved since is lost.
    let mut present: BTreeMap<u32, BTreeSet<(u8, u32)>> = BTreeMap::new();
    for row in rows.clone() {
        present
            .entry(row.exchange)
            .or_default()
            .insert((row.kind, row.variant));
    }
    let ended: Vec<u32> = state
        .exchanges
        .keys()
        .filter(|exchange| !present.contains_key(exchange))
        .copied()
        .collect();
    for exchange in ended {
        if let Some((seen, designs)) = state.exchanges.remove(&exchange) {
            scanned.ended.push(EndedExchange {
                from: state
                    .exchanges_read
                    .map_or(seen, |read| seen.max(read))
                    .saturating_add(1),
                designs,
            });
        }
    }
    for (exchange, designs) in present {
        state.exchanges.insert(exchange, (hour, designs));
    }
    for row in rows {
        let last = state
            .lend_lease
            .entry((row.exchange, row.kind, row.variant))
            .or_default();
        let delta = i128::from(row.amount_raw) - i128::from(*last);
        *last = row.amount_raw;
        let source = if row.kind == engine::clend_lease_exchange::SUNK_KIND {
            LossSource::LendLeaseSunk
        } else {
            LossSource::LendLeaseSent
        };
        scanned.event((source, Some(row.variant)), hour, delta);
    }
    state.exchanges_read = Some(hour);
}

/// The variant reference type (low word of a variant handle).
const VARIANT_TYPE: u32 = engine::cequipment_variant::REFERENCE_TYPE;

const DESTRUCTION_TRUNCATED: &str = "Equipment destruction drain is truncated; branch unavailable";

#[derive(Default)]
struct DestructionInputs {
    supplemental: Option<String>,
    unresolved: BTreeSet<u32>,
    by_slot: BTreeMap<u32, Vec<usize>>,
    nonempty: bool,
    #[cfg(test)]
    row_visits: usize,
}

/// The native drain's owner and availability operands, shared with the oracle.
fn destruction_inputs(
    decoded: &DecodedFrame,
    links: Option<&[CountryLinkRow]>,
    format: replay::format::FormatVersion,
) -> io::Result<DestructionInputs> {
    let Ok(rows) = decoded.complete::<tables::EquipmentDestruction>() else {
        return Ok(DestructionInputs::default());
    };
    let marker = rows.first().filter(|row| {
        row.coverage
            == replay::rows::Recorded::Value(
                replay::rows::EquipmentDestructionCoverage::SupplementalUnavailable,
            )
    });
    #[cfg(test)]
    let mut row_visits = 0;
    let supplemental = if format
        .field_gaps(TableId::EquipmentDestruction)
        .next()
        .is_some()
        || rows.iter().any(|row| {
            #[cfg(test)]
            {
                row_visits += 1;
            }
            row.not_recorded_fields() != 0
        }) {
        Some("EquipmentDestruction supplemental coverage not recorded".to_owned())
    } else if let Some(marker) = marker {
        Some(
            marker
                .unavailable_reason
                .require("EquipmentDestruction.unavailable_reason")
                .map_err(io::Error::other)?
                .clone(),
        )
    } else {
        None
    };
    let mut inputs = DestructionInputs {
        supplemental,
        nonempty: rows.len() > usize::from(marker.is_some()),
        ..Default::default()
    };
    for (index, row) in rows.iter().enumerate().skip(usize::from(marker.is_some())) {
        #[cfg(test)]
        {
            row_visits += 1;
        }
        if let Some(slot) = links.and_then(|links| link_slot(links, row.country)) {
            inputs.by_slot.entry(slot).or_default().push(index);
        } else {
            inputs.unresolved.insert(row.country);
        }
    }
    #[cfg(test)]
    {
        inputs.row_visits = row_visits;
    }
    Ok(inputs)
}

#[cfg(test)]
fn destruction(
    scanned: &mut Scanned,
    state: &mut State,
    decoded: &DecodedFrame,
    links: Option<&[CountryLinkRow]>,
    country: TaggedCountry<'_>,
) -> io::Result<()> {
    let inputs = if country == TaggedCountry::Absent || state.destruction_withheld.is_some() {
        DestructionInputs::default()
    } else {
        destruction_inputs(decoded, links, scanned.format)?
    };
    destruction_from(scanned, state, decoded, links, country, &inputs)
}

/// Each complete frame is a fresh hourly drain, even when the wire was Unchanged.
fn destruction_from(
    scanned: &mut Scanned,
    state: &mut State,
    decoded: &DecodedFrame,
    links: Option<&[CountryLinkRow]>,
    country: TaggedCountry<'_>,
    inputs: &DestructionInputs,
) -> io::Result<()> {
    let hour = decoded.hour;
    let table = &*decoded.tables[TableId::EquipmentDestruction.index()];
    if let Some(reason) = &state.destruction_withheld {
        scanned.destruction_cover(hour, hour, Some(reason.clone()));
        return Ok(());
    }
    let rows = match table {
        replay::frame::TableView::Complete(rows) => {
            scanned.destruction_counted_from.get_or_insert(hour);
            rows.rows::<tables::EquipmentDestruction>().ok_or_else(|| {
                io::Error::other("Equipment destruction table has the wrong row type")
            })?
        }
        replay::frame::TableView::Unavailable { reason } => {
            let partial = reason == EQUIPMENT_DESTRUCTION_PARTIAL_REASON;
            // A deliberately partial startup interval precedes counting.
            // A later partial interval has lost an already counted drain.
            if partial && scanned.destruction_counted_from.is_none() {
                return Ok(());
            }
            let kind = if partial {
                UnknownReason::DestructionGap
            } else {
                UnknownReason::DestructionWithheld
            };
            let detail = if reason.starts_with(replay::frame::TRUNCATED_KEYFRAME_REASON) {
                DESTRUCTION_TRUNCATED.to_owned()
            } else {
                reason.clone()
            };
            scanned.destruction_cover(hour, hour, Some(detail.clone()));
            scanned.destruction_unknown(hour, kind, detail.clone());
            if !partial {
                state.destruction_withheld = Some(detail);
            }
            return Ok(());
        }
        replay::frame::TableView::Truncated { .. } => {
            let reason = DESTRUCTION_TRUNCATED.to_owned();
            scanned.destruction_cover(hour, hour, Some(reason.clone()));
            scanned.destruction_unknown(hour, UnknownReason::DestructionWithheld, reason.clone());
            state.destruction_withheld = Some(reason);
            return Ok(());
        }
    };
    // A country absent from complete identity tables cannot own a removal,
    // including one missed by an unavailable supplemental writer.
    if country == TaggedCountry::Absent {
        scanned.destruction_cover(hour, hour, None);
        return Ok(());
    }
    let supplemental_reason = if state.destruction_coverage_not_recorded {
        Some("EquipmentDestruction supplemental coverage not recorded".to_owned())
    } else {
        inputs.supplemental.clone()
    };
    if let Some(reason) = &supplemental_reason {
        scanned.destruction_unknown(hour, UnknownReason::DestructionWithheld, reason.clone());
    }

    // An empty complete drain proves no hooked removals for any country;
    // there is no raw owner whose identity could make that zero uncertain.
    if !inputs.nonempty {
        scanned.destruction_cover(hour, hour, supplemental_reason);
        return Ok(());
    }
    let (Some(_), TaggedCountry::Found(_, slot)) = (links, country) else {
        let reason = "Equipment destruction ownership is unknown: country identity or CountryLinks unavailable".to_owned();
        scanned.destruction_cover(hour, hour, Some(reason.clone()));
        scanned.destruction_unknown(hour, UnknownReason::DestructionUnresolved, reason);
        return Ok(());
    };
    let unresolved = inputs.unresolved.clone();
    let owner_reason = (!unresolved.is_empty()).then(|| {
        "Equipment destruction ownership is unknown: raw country references have no CountryLink"
            .to_owned()
    });
    let reason = match (owner_reason.clone(), supplemental_reason) {
        (Some(owner), Some(supplemental)) => Some(format!("{owner}; {supplemental}")),
        (owner, supplemental) => owner.or(supplemental),
    };
    if let Some(reason) = owner_reason {
        scanned.destruction_unknown(hour, UnknownReason::DestructionUnresolved, reason);
    }
    scanned.destruction_cover(hour, hour, reason);
    scanned.unresolved.extend(unresolved);
    for &index in inputs.by_slot.get(&slot).into_iter().flatten() {
        let row = &rows[index];
        if state.land_source == LandSource::CombatLog
            && !state.include_all_destruction
            && !row.cause.is_air()
            && !matches!(
                row.cause,
                EquipmentDestructionCause::Unknown
                    | EquipmentDestructionCause::ScriptedStockpileFraction
            )
        {
            continue;
        }
        if row.variant as u32 != VARIANT_TYPE {
            return Err(io::Error::other(format!(
                "Equipment destruction design {:#x} has an unsupported reference type",
                row.variant
            )));
        }
        if row.amount_raw == 0 {
            return Err(io::Error::other(
                "Equipment destruction contains a zero event amount",
            ));
        }
        let source = LossSource::Destruction(row.cause);
        scanned.event(
            (source, Some((row.variant >> 32) as u32)),
            hour,
            row.amount_raw,
        );
    }
    Ok(())
}

/// Explains whether the game kept this country's land losses, never whether
/// to capture them. The country gate alone is insufficient: damage logging
/// skips armies without an orders group, then chooses a positive expeditionary
/// owner or the roster owner. Mixed group membership has incomplete coverage.
/// Logging also requires feature 3 in the runtime mask recorded by
/// SimulationFeatures. Missing inputs stay unknown.
#[derive(Default, PartialEq, Eq)]
struct CoverageInputs {
    gate_known: bool,
    all: bool,
    players: BTreeSet<u32>,
    unresolved: bool,
    features: Option<bool>,
    prune_observed: bool,
}
impl CoverageInputs {
    fn country(&self, slot: u32, membership: Option<census::Membership>) -> Coverage {
        if !self.gate_known {
            return Coverage::Unknown;
        }
        if !self.all && !self.players.contains(&slot) {
            return if self.unresolved {
                Coverage::Unknown
            } else {
                Coverage::NotLogged
            };
        }
        match self.features {
            None => Coverage::Unknown,
            Some(false) => Coverage::NotLogged,
            Some(true) => membership.map_or(Coverage::Unknown, |membership| {
                membership.coverage(self.prune_observed)
            }),
        }
    }
}
fn coverage_inputs(decoded: &DecodedFrame, links: &[CountryLinkRow]) -> CoverageInputs {
    let Ok([state]) = decoded
        .complete::<tables::CombatLogState>()
        .map(<[_]>::as_ref)
    else {
        return CoverageInputs::default();
    };
    let all = state.viewing_country_raw < 1;
    let mut inputs = CoverageInputs {
        all,
        prune_observed: state.prune_observed != 0,
        ..Default::default()
    };
    if !all {
        let Ok(players) = decoded.complete::<tables::Players>() else {
            return inputs;
        };
        for player in players {
            if let Some(slot) = link_slot(links, player.country) {
                inputs.players.insert(slot);
            } else {
                inputs.unresolved = true;
            }
        }
    }
    inputs.gate_known = true;
    if let Ok([features]) = decoded
        .complete::<tables::SimulationFeatures>()
        .map(<[_]>::as_ref)
    {
        inputs.features =
            Some(features.dlc_mask & engine::dlc_feature_state::EQUIPMENT_LOSSES_MASK != 0);
    }
    inputs
}
#[cfg(test)]
fn coverage(
    decoded: &DecodedFrame,
    links: &[CountryLinkRow],
    slot: u32,
    membership: Option<census::Membership>,
) -> Coverage {
    coverage_inputs(decoded, links).country(slot, membership)
}

/// Whether the Combat Log was read completely at this hour: its rows, and the
/// slice reads that say which countries were read now.
fn land_read(decoded: &DecodedFrame) -> bool {
    decoded.complete::<tables::SliceReads>().is_ok()
        && matches!(
            &*decoded.tables[TableId::CombatLogEquipment.index()],
            replay::frame::TableView::Complete(_)
        )
}

/// The country's Combat Log rows read at this hour: each (group, kind, month,
/// variant) keeps its last value, and each change is an event at this hour.
/// Equipment losses add signed pre-minus-post pool differences. A fall within
/// the same monthly key is a correction; a new month is already a different
/// key. Never count the falling value anew.
fn combat_rows<'a>(
    scanned: &mut Scanned,
    state: &mut State,
    hour: i32,
    rows: impl Iterator<Item = &'a replay::rows::CombatLogEquipmentRow>,
) {
    for row in rows {
        let key = (row.group, row.kind, row.month, row.variant);
        let last = state.log.entry(key).or_default();
        let delta = i128::from(row.amount_raw) - i128::from(*last);
        *last = row.amount_raw;
        let source = if row.kind == engine::ncombat_log_cmanager::RECOVERED_KIND {
            LossSource::Recovered
        } else {
            LossSource::CombatLog(row.kind)
        };
        scanned.event((source, Some(row.variant)), hour, delta);
    }
}

/// Running totals from events: sorted by hour, one point per changed hour.
fn points(events: &[(i32, i128)]) -> Vec<LossPoint> {
    let mut sorted = events.to_vec();
    sorted.sort_by_key(|(hour, _)| *hour);
    let mut total: i128 = 0;
    let mut points: Vec<LossPoint> = Vec::new();
    for (hour, delta) in sorted {
        total += delta;
        match points.last_mut() {
            Some(last) if last.hour == hour => last.total_raw = total.to_string(),
            _ => points.push(LossPoint {
                hour,
                total_raw: total.to_string(),
            }),
        }
    }
    points
}

/// The localisation of a key: its text without colour codes, and the text
/// when it resolves without substitution.
fn name(catalogue: Option<&catalogue::Catalogue>, key: Option<&str>) -> LossName {
    let (Some(catalogue), Some(key)) = (catalogue, key) else {
        return LossName {
            key: key.map(str::to_owned),
            ..LossName::default()
        };
    };
    let template = catalogue.localisation.get(key).and_then(|text| {
        catalogue::plain_text(text)
            .inspect_err(|reason| catalogue.refuse(reason, key))
            .ok()
    });
    // The localiser fills an unknown `$NAME$` with nothing, so a template
    // with any fill-in has no complete text.
    let text = template
        .as_ref()
        .filter(|template| !template.contains('$'))
        .and_then(|_| catalogue.localised(key, View::Plain));
    LossName {
        key: Some(key.to_owned()),
        template,
        text,
    }
}

/// The Combat Log equipment tab's column titles: reason 0's, the recovered
/// column's, and the attrition column reasons 1 to 3 add up in. These are
/// static text boxes whose authored declarations supply their localisation keys.
const COMBAT_LOG_TAB: &str = "combat_log_equipment_tab";
const COMBAT_LOG_TITLES: [(&str, &str); 3] = [
    ("title_lost_in_combat", "text"),
    ("title_recovered", "text"),
    ("title_lost_by_attrition", "text"),
];
/// The naval losses window and the tooltip of its total_ships text box,
/// selected by their authored names.
const NAVAL_LOSSES_WINDOW: &str = "navallosseswindow";
const NAVAL_LOSSES_TOTAL: [(&str, &str); 1] = [("total_ships", "pdx_tooltip")];
/// The cause names only the content's interface files author, read through
/// the content stack ([`installed_content::gui::native::window_texts`]). A key that does not
/// resolve is null, and why is a warning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InterfaceKeys {
    pub combat: Option<String>,
    pub recovered: Option<String>,
    pub attrition: Option<String>,
    pub ships_lost: Option<String>,
}

impl InterfaceKeys {
    pub fn read(guis: &installed_content::gui::native::Guis, warnings: &mut Vec<String>) -> Self {
        let mut resolve = |window: &str, texts: &[(&str, &str)]| -> Vec<Option<String>> {
            match installed_content::gui::native::window_texts(guis, window, texts, warnings) {
                Ok(keys) => keys
                    .into_iter()
                    .map(|key| {
                        key.map_err(|reason| {
                            warnings.push(format!("A loss cause is unnamed: {reason}"))
                        })
                        .ok()
                    })
                    .collect(),
                Err(error) => {
                    warnings.push(format!("Loss causes from {window} are unnamed: {error}"));
                    vec![None; texts.len()]
                }
            }
        };
        let mut titles = resolve(COMBAT_LOG_TAB, &COMBAT_LOG_TITLES).into_iter();
        let mut total = resolve(NAVAL_LOSSES_WINDOW, &NAVAL_LOSSES_TOTAL).into_iter();
        Self {
            combat: titles.next().flatten(),
            recovered: titles.next().flatten(),
            attrition: titles.next().flatten(),
            ships_lost: total.next().flatten(),
        }
    }
}

/// The localisation key the game shows each cause under, resolved from the
/// content's interface definitions.
fn cause_key(source: LossSource, keys: &InterfaceKeys) -> Option<&str> {
    match source {
        LossSource::Destruction(EquipmentDestructionCause::LandCombat) => keys.combat.as_deref(),
        LossSource::Destruction(EquipmentDestructionCause::Attrition) => keys.attrition.as_deref(),
        LossSource::Destruction(_) => None,
        LossSource::CombatLog(0) => keys.combat.as_deref(),
        // Combat Log attrition reasons 1, 2 and 3 use separate content keys.
        LossSource::CombatLog(1) => Some("COMBAT_LOG_EQ_LOST_ATTR_OTHER"),
        LossSource::CombatLog(2) => Some("COMBAT_LOG_EQ_LOST_ATTR_COMBAT"),
        LossSource::CombatLog(_) => Some("COMBAT_LOG_EQ_LOST_ATTR_TRAINING"),
        LossSource::Recovered => keys.recovered.as_deref(),
        LossSource::SunkShips => keys.ships_lost.as_deref(),
        // Civilian intelligence uses this content key.
        LossSource::SunkConvoys => Some("LEDGER_CHART_DAMAGED_CONVOYS"),
        // The lend-lease details tooltip uses this content key.
        LossSource::LendLeaseSunk => Some("LEND_LEASE_DETAILS_EQUIPMENT_SUNK"),
        // The logistics equipment tooltip uses this content key.
        LossSource::LendLeaseSent => Some("LOGISTICS_NEED_LEND_LEASE"),
    }
}

fn cause_name(
    source: LossSource,
    catalogue: Option<&catalogue::Catalogue>,
    keys: &InterfaceKeys,
) -> LossName {
    let key = cause_key(source, keys);
    let mut named = name(catalogue, key);
    // Destruction causes the content names keep the game's name.
    let named_by_game = matches!(source, LossSource::Destruction(_)) && named.text.is_some();
    if let Some(label) = app_label(source).filter(|_| !named_by_game) {
        named.text = Some(label.to_owned());
    }
    named
}

const NAMED_SOURCES: [LossSource; 9] = [
    LossSource::CombatLog(0),
    LossSource::CombatLog(1),
    LossSource::CombatLog(2),
    LossSource::CombatLog(3),
    LossSource::Recovered,
    LossSource::SunkShips,
    LossSource::SunkConvoys,
    LossSource::LendLeaseSunk,
    LossSource::LendLeaseSent,
];

fn equipment_name(catalogue: &catalogue::Catalogue, key: &str) -> Option<String> {
    catalogue.localisation.get(key)?;
    catalogue.localised(key, View::Plain)
}

fn archetype_token(database: &installed_content::equipment::Database, token: &str) -> Option<String> {
    let archetype = database.named(token)?.archetype?;
    database
        .equipment
        .get(archetype)?
        .name_str()
        .ok()
        .map(str::to_owned)
}

/// NDefines.NGame.COMBAT_LOG_MAX_MONTHS, used by daily pruning.
fn retained_months(catalogue: &catalogue::Catalogue) -> io::Result<i32> {
    installed_content::defines::read(&catalogue.content.layers)?.int32("NGame", "COMBAT_LOG_MAX_MONTHS")
}

/// Checks authored inputs through the reply's naming and retention functions;
/// loss/design inventories themselves only exist in a recording.
pub fn check_content(
    catalogue: &catalogue::Catalogue,
    database: &installed_content::equipment::Database,
) -> io::Result<Vec<dashboard_core::findings::Finding>> {
    use dashboard_core::findings::{Finding, Kind};
    let mut warnings = Vec::new();
    let keys = InterfaceKeys::read(catalogue.guis()?, &mut warnings);
    let mut findings: Vec<_> = warnings
        .into_iter()
        .map(|message| Finding::new(Kind::Warning, "equipment loss interface", message))
        .collect();
    for source in NAMED_SOURCES {
        let key = cause_key(source, &keys);
        let named = name(Some(catalogue), key);
        if named.text.is_none() {
            findings.push(Finding::grouped(
                Kind::Fallback,
                format!("loss cause {source:?}"),
                "loss cause name is unavailable; its reply name is null",
                format!(
                    "loss cause name {} is unavailable; its reply name is null",
                    key.unwrap_or("<undeclared interface key>")
                ),
            ));
        }
    }
    name(Some(catalogue), keys.attrition.as_deref());
    for equipment in &database.equipment {
        let token = equipment
            .name_str()
            .map_err(|why| dashboard_core::invalid(why.to_string()))?;
        let subject = format!("loss equipment {token}");
        if equipment_name(catalogue, token).is_none() {
            findings.push(Finding::grouped(Kind::Fallback, &subject,
                "equipment-losses has no usable design base name; its reply name is null",
                format!("{token}: equipment-losses has no usable design base name; its reply name is null")));
        }
        if let Some(archetype) = archetype_token(database, token) {
            equipment_name(catalogue, &archetype);
        }
    }
    match retained_months(catalogue) {
        Ok(months) if months < 1 => findings.push(Finding::new(
            Kind::Refused,
            "Combat Log retention",
            retention_unknown(months),
        )),
        Ok(_) => {}
        Err(error) => findings.push(Finding::new(
            Kind::Refused,
            "Combat Log retention",
            format!("The Combat Log's retention is unavailable: {error}"),
        )),
    }
    Ok(findings)
}

/// Why land totals cannot be kept under a retention of `months` below 1.
/// The daily prune searches backwards for the first entry whose date plus
/// NGame.COMBAT_LOG_MAX_MONTHS is at or before the day, then deletes it and
/// every earlier entry. A writer adds only to an entry of the current month.
/// For a setting of 1 or more, adding months lands in a later month, so no
/// entry that can still change is deleted. For 0 or less it lands at or before
/// the entry's own date, so every entry, the current month's included, is
/// deleted each day with the losses written after its last read.
fn retention_unknown(months: i32) -> String {
    format!(
        "NGame.COMBAT_LOG_MAX_MONTHS is {months}: the game deletes every Combat Log entry each day, the current month's included, so land losses written after an entry's last read are lost and no total can be kept."
    )
}

/// What naming and retention the content gives a reply.
#[derive(Clone, Copy, Default)]
pub struct Content<'a> {
    pub catalogue: Option<&'a catalogue::Catalogue>,
    pub archetypes: Option<&'a installed_content::equipment::Database>,
    pub keys: Option<&'a InterfaceKeys>,
    /// `NGame.COMBAT_LOG_MAX_MONTHS`.
    pub retained_months: Option<i32>,
}

/// Builds the reply from a scan, naming through the content when it is known.
pub fn reply(
    params: &LossesParams,
    scanned: &Scanned,
    content: Content<'_>,
    mut warnings: Vec<String>,
) -> LossesReply {
    let Content {
        catalogue,
        archetypes,
        keys,
        retained_months,
    } = content;
    let no_keys = InterfaceKeys::default();
    let keys = keys.unwrap_or(&no_keys);
    let combat_log = params.land_source == LandSource::CombatLog;
    // A retention that deletes entries still changing makes land totals
    // unknowable: refused by name.
    let land_unknown = retained_months
        .filter(|_| combat_log)
        .filter(|&months| months < 1)
        .map(retention_unknown);
    if combat_log && retained_months.is_none() {
        warnings.push("The Combat Log's retention (NGame.COMBAT_LOG_MAX_MONTHS) is unknown; land totals assume a setting of at least 1, under which the game deletes no entry that can still change.".to_owned());
    }
    if scanned.convoy_drops != 0 {
        warnings.push(format!(
            "{} recorded SunkConvoys counts fell in value; each was counted as a new entry.",
            scanned.convoy_drops
        ));
    }
    let last_hour = scanned.last_hour.unwrap_or(i32::MIN);
    let ended_spans: Vec<UnknownSpan> = scanned
        .ended
        .iter()
        .map(|ended| UnknownSpan {
            first_hour: ended.from,
            last_hour: last_hour.max(ended.from),
            reason: UnknownReason::EndedAfterLastRead,
            detail: None,
        })
        .collect();
    // Unknown callers retain the prior land/air scope. Stockpile fractions have
    // a content-defined equipment domain: land comes from the selected log,
    // while air/naval fractions remain distinct from ship/convoy sinkings.
    let mut known_domain = BTreeSet::new();
    let mut domain_from = BTreeMap::new();
    let mixed_domain = |source| {
        matches!(
            source,
            LossSource::Destruction(
                EquipmentDestructionCause::Unknown
                    | EquipmentDestructionCause::ScriptedStockpileFraction
            )
        )
    };
    if combat_log {
        for (&(source, variant), events) in &scanned.events {
            if !mixed_domain(source) {
                continue;
            }
            let domain = variant.and_then(|variant| {
                let handle = u64::from(variant) << 32 | u64::from(VARIANT_TYPE);
                archetype_of(archetypes?, scanned.variants.token(handle))
                    .ok()
                    .map(|(_, _, domain)| domain)
            });
            let stockpile = source
                == LossSource::Destruction(EquipmentDestructionCause::ScriptedStockpileFraction);
            if matches!(domain, Some(Domain::Air))
                || domain
                    == Some(if stockpile {
                        Domain::Naval
                    } else {
                        Domain::Land
                    })
            {
                known_domain.insert((source, variant));
            } else if !(stockpile && domain == Some(Domain::Land))
                && let Some(&(first_hour, _)) = events.first()
            {
                domain_from
                    .entry(source)
                    .and_modify(|before: &mut i32| *before = (*before).min(first_hour))
                    .or_insert(first_hour);
            }
        }
    }
    let mut destruction_spans: Vec<_> = scanned
        .destruction_unknown
        .iter()
        .cloned()
        .map(|mut span| {
            span.last_hour = last_hour.max(span.first_hour);
            span
        })
        .collect();
    destruction_spans.sort_unstable();
    let destruction_lower_bound = destruction_spans.iter().map(|span| span.first_hour).min();
    let mut ended_lower_bounds = BTreeMap::new();
    for ended in &scanned.ended {
        for &design in &ended.designs {
            ended_lower_bounds
                .entry(design)
                .and_modify(|from: &mut i32| *from = (*from).min(ended.from))
                .or_insert(ended.from);
        }
    }
    let unknown_spans = |source: LossSource| -> Vec<UnknownSpan> {
        if matches!(source, LossSource::Destruction(_)) {
            let mut spans = destruction_spans.clone();
            if let Some(&first_hour) = domain_from.get(&source) {
                spans.push(UnknownSpan {
                    first_hour, last_hour: last_hour.max(first_hour), reason: UnknownReason::DestructionDomain,
                    detail: Some("The recorded content does not resolve this destruction equipment's domain; its contribution in Combat Log mode is not known".to_owned()),
                });
                spans.sort_unstable();
            }
            return spans;
        }
        let mut spans: Vec<UnknownSpan> = source
            .record()
            .and_then(|record| scanned.unread.get(&record))
            .cloned()
            .unwrap_or_default();
        if source.record() == Some(LossRecord::LendLease) {
            spans.extend(ended_spans.iter().cloned());
        }
        spans.sort_unstable();
        spans
    };
    let lower_bound_from = |source: LossSource, variant: Option<u32>| -> Option<i32> {
        if matches!(source, LossSource::Destruction(_)) {
            return destruction_lower_bound
                .into_iter()
                .chain(domain_from.get(&source).copied())
                .min();
        }
        let kind = match source {
            LossSource::LendLeaseSunk => engine::clend_lease_exchange::SUNK_KIND,
            LossSource::LendLeaseSent => engine::clend_lease_exchange::DELIVERED_KIND,
            _ => return None,
        };
        variant.and_then(|variant| ended_lower_bounds.get(&(kind, variant)).copied())
    };
    let mut designs: Vec<LossDesign> = Vec::new();
    let mut design_index: BTreeMap<u32, u32> = BTreeMap::new();
    let mut design = |variant: u32| -> u32 {
        *design_index.entry(variant).or_insert_with(|| {
            let handle = (u64::from(variant) << 32) | u64::from(VARIANT_TYPE);
            let token = scanned.variants.token(handle);
            let equipment = (!token.is_empty()).then(|| token.to_owned());
            let archetype = equipment
                .as_deref()
                .and_then(|equipment| archetype_token(archetypes?, equipment));
            let localised =
                |key: &str| catalogue.and_then(|catalogue| equipment_name(catalogue, key));
            designs.push(LossDesign {
                variant,
                names: scanned
                    .variants
                    .names(handle, catalogue.map(|texts| texts as &dyn Texts)),
                archetype_name: archetype.as_deref().and_then(localised),
                equipment,
                archetype,
            });
            (designs.len() - 1) as u32
        })
    };
    let mut sources: BTreeSet<LossSource> = [
        LossSource::SunkShips,
        LossSource::SunkConvoys,
        LossSource::LendLeaseSunk,
        LossSource::LendLeaseSent,
    ]
    .into();
    if combat_log {
        sources.extend((0..=3).map(LossSource::CombatLog));
        sources.insert(LossSource::Recovered);
    }
    sources.extend(
        EquipmentDestructionCause::ALL
            .into_iter()
            .filter(|cause| {
                !combat_log
                    || cause.is_air()
                    || matches!(
                        cause,
                        EquipmentDestructionCause::Unknown
                            | EquipmentDestructionCause::ScriptedStockpileFraction
                    )
            })
            .map(LossSource::Destruction),
    );
    let causes: Vec<LossCause> = sources
        .into_iter()
        .map(|source| {
            let unknown = land_unknown
                .clone()
                .filter(|_| source.record() == Some(LossRecord::CombatLog));
            LossCause {
                source,
                domain: source.domain(),
                name: cause_name(source, catalogue, keys),
                group: matches!(source, LossSource::CombatLog(1..=3))
                    .then(|| name(catalogue, keys.attrition.as_deref())),
                loss: !matches!(source, LossSource::Recovered | LossSource::LendLeaseSent),
                unknown_spans: unknown_spans(source),
                leaves: match unknown {
                    Some(_) => Vec::new(),
                    None => scanned
                        .events
                        .range((source, None)..=(source, Some(u32::MAX)))
                        .filter(|&(&(_, variant), _)| {
                            !combat_log
                                || !mixed_domain(source)
                                || known_domain.contains(&(source, variant))
                        })
                        .map(|(&(_, variant), events)| LossLeaf {
                            design: variant.map(&mut design),
                            points: points(events),
                            lower_bound_from: lower_bound_from(source, variant),
                        })
                        .collect(),
                },
                unknown,
            }
        })
        .collect();
    if combat_log
        && scanned
            .coverage
            .iter()
            .any(|span| span.coverage == Coverage::NotLogged)
    {
        warnings.push("The game did not keep this country's land losses at some hours (the country or feature gate was closed, or its divisions had no orders groups); those losses are unknown, not zero.".to_owned());
    }
    if combat_log
        && scanned
            .coverage
            .iter()
            .any(|span| span.coverage == Coverage::Unknown)
    {
        warnings.push("Some hours have recorded gaps, failed blocks/frames, unread division census, unresolved owners or missing writer inputs; land-loss coverage is unknown there.".to_owned());
    }
    let unresolved = scanned
        .unresolved
        .union(&scanned.combat_unresolved)
        .filter(|raw| combat_log || scanned.unresolved.contains(*raw))
        .count();
    if unresolved != 0 {
        warnings.push(format!(
            "Recorded loss data name {} country references the recorded links do not resolve; they are left out.",
            unresolved
        ));
    }
    if let Some(catalogue) = catalogue {
        warnings.extend(catalogue.refusal_lines());
    }
    LossesReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        land_source: params.land_source,
        land_lower_bound: None,
        amount_scale: AMOUNT_SCALE as u32,
        causes,
        designs,
        coverage: if combat_log {
            scanned.coverage.clone()
        } else {
            Vec::new()
        },
        destruction_coverage: scanned.destruction_coverage.clone(),
        destruction_counted_from: scanned.destruction_counted_from,
        retained_months: retained_months.filter(|_| combat_log),
        warnings,
    }
}

#[cfg(test)]
type ScanKey = (u64, String, LandSource, Option<LossCountry>);

#[derive(Default)]
#[cfg(test)]
struct Cache {
    entries: VecDeque<(ScanKey, Arc<Scanned>)>,
}

impl Product for EquipmentLosses {
    const KEY: &'static str = "equipment-losses";
    const CONTENT_DEPENDENT: bool = true;
    type Params = LossesParams;
    type Reply = LossesReply;

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
            StockVariantName::decl(),
            LossDesign::decl(),
            Coverage::decl(),
            CoverageSpan::decl(),
            DestructionCoverageSpan::decl(),
            LossesReply::decl(),
            prepared::PreparedLossesReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.country_params().validate()
    }

    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        if let Some(country) = params.country {
            let histories = dashboard_base::products::country_history::branch_histories(
                source,
                params.branch(),
                false,
                &[],
                cancelled,
            )?;
            let witnessed = histories.countries.get(&params.tag).is_some_and(|history| {
                history.identities.iter().any(|identity| {
                    identity.first_hour <= country.origin_hour
                        && country.origin_hour <= identity.last_hour
                        && identity.slot == country.slot
                        && identity.country_id == country.country_id
                })
            });
            if !witnessed {
                return Err(io::Error::other(
                    "The selected native country identity is not recorded at its origin",
                ));
            }
        }
        let key = (
            params.branch(),
            params.tag.clone(),
            params.land_source,
            params.country,
        );
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == key)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(
                    &source.replay(),
                    params.branch(),
                    &params.tag,
                    params.land_source,
                    params.country,
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_SCANS {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        reply_with_content(source, params, context, &scanned)
    }
}

fn reply_with_content(
    source: &Source,
    params: &LossesParams,
    context: &LoadContext,
    scanned: &Scanned,
) -> io::Result<LossesReply> {
    let mut warnings = Vec::new();
    let catalogue = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(catalogue) => Some(catalogue),
        Err(error) => {
            warnings.push(format!(
                "Cause, type and design names are unavailable: {error}"
            ));
            None
        }
    };
    let archetypes = catalogue.as_ref().and_then(|catalogue| {
        installed_content::equipment::Database::load_complete(&catalogue.content.layers)
            .inspect(|database| warnings.extend(database.misreads.iter().cloned()))
            .map_err(|error| warnings.push(format!("Equipment types are unavailable: {error}")))
            .ok()
    });
    // NDefines.NGame.COMBAT_LOG_MAX_MONTHS controls daily pruning.
    let retained_months = (params.land_source == LandSource::CombatLog)
        .then_some(catalogue.as_ref())
        .flatten()
        .and_then(|catalogue| {
            installed_content::defines::read(&catalogue.content.layers)
                .and_then(|defines| defines.int32("NGame", "COMBAT_LOG_MAX_MONTHS"))
                .map_err(|error| {
                    warnings.push(format!(
                        "The Combat Log's retention is unavailable: {error}"
                    ))
                })
                .ok()
        });
    let keys = catalogue.as_ref().and_then(|catalogue| {
        catalogue
            .guis()
            .map(|guis| InterfaceKeys::read(guis, &mut warnings))
            .map_err(|error| {
                warnings.push(format!(
                    "Equipment loss interface names are unavailable: {error}"
                ))
            })
            .ok()
    });
    if let Some(catalogue) = &catalogue {
        warnings.extend(catalogue.verification.iter().cloned());
    }
    Ok(reply(
        params,
        scanned,
        Content {
            catalogue: catalogue.as_deref(),
            archetypes: archetypes.as_ref(),
            keys: keys.as_ref(),
            retained_months,
        },
        warnings,
    ))
}

#[cfg(test)]
mod tests;
