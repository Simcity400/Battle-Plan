//! Scrubbable world model over a [`ReplaySource`].
//!
//! [`WorldTimeline`] owns the source and a small frame cache; [`WorldFrame`]
//! is one hour of world state with the raw domain map plus typed views for
//! the surfaces the replay UI consumes (map control, divisions, combats,
//! country core, provenance). Typed views never coerce: a wrong-typed value
//! becomes [`Leaf::Unobtained`] plus a [`WorldFrame::coverage_warnings`]
//! entry, never a guessed value and never a silent drop.

use crate::source::{RawFrame, ReplaySource, SourceError};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// Default frames kept hot for scrubbing (see [`TimelineConfig`]). Small on
/// purpose: a frame is rebuilt from the verified source on miss, so eviction
/// only costs a re-read. DERIVED: the pre-F2 fixed cap, proven bounded by the
/// F3 soak harness (`replay_app/tests/soak.rs`).
const FRAME_CACHE_CAP: usize = 16;

/// The no-fabrication trichotomy, preserved end to end:
///
/// * `Absent`     — the key was never captured (absent from the JSON).
/// * `Unobtained` — the key is present with an explicit JSON `null`
///   (captured-but-unobtained), OR present with a wrong-typed value. A
///   wrong-typed value additionally appends to
///   [`WorldFrame::coverage_warnings`] — it is never coerced and never
///   dropped silently.
/// * `Measured(T)` — a typed value, including a measured `0` / `0.0`.
///
/// Consumers must render all three distinctly.
#[derive(Debug, Clone, PartialEq)]
pub enum Leaf<T> {
    Absent,
    Unobtained,
    Measured(T),
}

impl<T> Leaf<T> {
    pub fn is_measured(&self) -> bool {
        matches!(self, Leaf::Measured(_))
    }
    pub fn measured(&self) -> Option<&T> {
        match self {
            Leaf::Measured(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum WorldError {
    Source(SourceError),
    /// The recording's hour series is unusable (empty or out of order).
    Timeline(String),
    /// [`WorldTimeline::frame`] is exact-hour; this hour has no frame.
    NoSuchHour(i64),
    /// [`WorldTimeline::state_at`] before the first recorded frame.
    BeforeFirst {
        hour: i64,
        first: i64,
    },
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorldError::Source(e) => write!(f, "source: {e}"),
            WorldError::Timeline(m) => write!(f, "timeline: {m}"),
            WorldError::NoSuchHour(h) => write!(
                f,
                "no frame at hour_index {h} (frame() is exact-hour; state_at() scrubs)"
            ),
            WorldError::BeforeFirst { hour, first } => write!(
                f,
                "hour_index {hour} is before the first recorded frame ({first})"
            ),
        }
    }
}
impl std::error::Error for WorldError {}
impl From<SourceError> for WorldError {
    fn from(e: SourceError) -> Self {
        WorldError::Source(e)
    }
}

/// Where a map-control view came from (recorded so a consumer can tell a
/// writer-provided mirror from a fallback walk from a missing layer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapLayerSource {
    /// The `map.owner_by_state` / `map.control_by_province` mirror.
    MapMirror,
    /// Fallback walk of `states.*.owner` / `provinces.*.controller`.
    DomainWalk,
    /// Neither the mirror nor the domain existed; the view is empty.
    Missing,
}

/// Domain-level availability for a typed view.
///
/// [`Leaf`] carries the trichotomy for individual values. This status carries
/// the same honesty at the view root so consumers can distinguish "this
/// recording never carried the domain" from "the domain exists, but the value
/// for this hour was null or malformed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewStatus {
    DomainAbsent,
    DomainUnavailable,
    Present,
}

/// Completeness of the d53 faction roster. Unlike a generic view status,
/// measured-empty is positive knowledge: only a FACS completeness witness (or
/// the explicit legacy fallback for pre-d53 recordings) may produce it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FactionRosterStatus {
    #[default]
    Unavailable,
    MeasuredEmpty,
    MeasuredNonEmpty,
}

impl FactionRosterStatus {
    pub fn is_measured(self) -> bool {
        !matches!(self, Self::Unavailable)
    }
}

/// One division row (REPLAY_SCHEMA.md §6).
#[derive(Debug, Clone, PartialEq)]
pub struct DivisionRow {
    /// `"div_id:div_type"`, falling back to bare `"div_id"` when `div_type`
    /// is absent/null (schema §6 identity rule).
    pub div_uid: String,
    pub owner: Leaf<String>,
    pub location_province: Leaf<u32>,
    pub strength: Leaf<f64>,
    pub organisation: Leaf<f64>,
    pub in_combat: Leaf<bool>,
}

/// One active combat row (REPLAY_SCHEMA.md §7).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatRow {
    pub combat_id: Leaf<i64>,
    pub kind: Leaf<String>,
    pub location_province: Leaf<u32>,
    /// Participant division ids (bare `div_id`s per §7).
    pub attackers: Leaf<Vec<i64>>,
    pub defenders: Leaf<Vec<i64>>,
}

/// Equipment composition row shared by divisions, task forces, and air wings.
#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentAmount {
    pub archetype: Leaf<String>,
    pub equip_id: Leaf<i64>,
    pub equip_type: Leaf<i64>,
    pub amount: Leaf<f64>,
}

/// Full land-unit view for item 3: position, movement, template role, and
/// equipment composition at this hour.
#[derive(Debug, Clone, PartialEq)]
pub struct DivisionUnitRow {
    pub div_uid: String,
    pub owner: Leaf<String>,
    pub location_province: Leaf<u32>,
    pub state_id: Leaf<u32>,
    pub template_id: Leaf<i64>,
    pub template_type: Leaf<i64>,
    pub template_name: Leaf<String>,
    pub template_role: Leaf<String>,
    pub strength: Leaf<f64>,
    pub organisation: Leaf<f64>,
    pub moving: Leaf<bool>,
    pub move_target_province: Leaf<u32>,
    pub move_progress: Leaf<f64>,
    pub in_combat: Leaf<bool>,
    pub equipment: Leaf<Vec<EquipmentAmount>>,
}

/// Task-force / fleet presence view for item 8.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetUnitRow {
    pub task_force_uid: String,
    pub fleet_uid: Leaf<String>,
    pub owner: Leaf<String>,
    pub location_province: Leaf<u32>,
    pub mission: Leaf<String>,
    pub ship_count: Leaf<i64>,
    pub dominant_hull_class: Leaf<String>,
    pub ships: Leaf<Vec<ShipCompositionRow>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShipCompositionRow {
    pub ship_uid: Leaf<String>,
    pub definition: Leaf<String>,
    pub organisation: Leaf<f64>,
    pub strength: Leaf<f64>,
    pub experience: Leaf<f64>,
    pub name: Leaf<String>,
    pub equipment: Leaf<Vec<EquipmentAmount>>,
}

/// Air-wing presence view for item 8.
#[derive(Debug, Clone, PartialEq)]
pub struct WingUnitRow {
    pub wing_uid: String,
    pub owner: Leaf<String>,
    pub name: Leaf<String>,
    pub plane_count: Leaf<i64>,
    pub experience: Leaf<f64>,
    pub manpower: Leaf<i64>,
    pub mission_kind: Leaf<String>,
    pub strategic_region_id: Leaf<u32>,
    pub air_base: Leaf<String>,
    pub state_id: Leaf<u32>,
    pub plane_definition: Leaf<String>,
    pub effective_planes: Leaf<i64>,
    pub equipment: Leaf<Vec<EquipmentAmount>>,
}

/// Session metadata needed for playback pacing and player-country POVs.
///
/// TODO(A3/A4): REPLAY_SCHEMA.md has not pinned a first-class session event
/// payload for speed/pause changes yet. This view consumes a conservative
/// internal `session` object when present, with fallback only to already
/// schema-shaped `world.scenario` leaves for DLC/language. Missing fields stay
/// absent/unavailable rather than inferred.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMetadataView {
    pub status: ViewStatus,
    pub player_countries: Leaf<Vec<PlayerCountry>>,
    pub speed_events: Leaf<Vec<SpeedEvent>>,
    pub active_dlcs: Leaf<Vec<String>>,
    pub localisation_language: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerCountry {
    pub tag: Leaf<String>,
    pub player_name: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeedEvent {
    pub hour_index: Leaf<i64>,
    pub speed: Leaf<i64>,
    pub paused: Leaf<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavalSupremacyRow {
    pub controller: Leaf<String>,
    pub state: Leaf<String>,
    /// Region naval supremacy magnitude.
    ///
    /// UNIT — **UNKNOWN (extractor must state)**: the fixture generator
    /// (`replay_fixture_gen::fx_naval_air_activity`) emits this NORMALISED in
    /// `0.0..=1.0` (`0.55 + tick*0.02`), but the engine's own naval-region
    /// supremacy is accumulated in raw points and only normalised for the
    /// map-mode tint against `NAVAL_REGION_TRANSPARENCY_RANGE` (=2000, the
    /// full-supremacy point ceiling). Whether a real recording carries the
    /// normalised fraction or the raw point total is NOT pinned by the schema
    /// today (documented gap D56, c6 lane). The view therefore surfaces the
    /// number VERBATIM and never rescales; consumers must not assume a unit
    /// until `REPLAY_SCHEMA.md` states it. See `EXTRACTION_NEEDS.md §A`.
    pub supremacy: Leaf<f64>,
}

/// Per-strategic-region air-superiority contest (requirements item 5),
/// distinct from [`NavalSupremacyRow`]'s naval region control. Sourced from
/// `naval_air.air_superiority.<region>` in the recording schema.
#[derive(Debug, Clone, PartialEq)]
pub struct AirSuperiorityRow {
    pub controller: Leaf<String>,
    pub mission_mask: Leaf<i64>,
    pub wing_count: Leaf<i64>,
    pub plane_count: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherRegionRow {
    pub weather_kind: Leaf<String>,
    pub intensity: Leaf<f64>,
    pub snow_level_0_255: Leaf<u32>,
    pub mud_level_0_255: Leaf<u32>,
    pub snow_mud_fade: Leaf<f64>,
    pub snow_mud_fade_from: Leaf<f64>,
    pub snow_mud_fade_to: Leaf<f64>,
}

/// One per-province accumulated SnowMudData cell (requirements item 6): the
/// engine's ground-cover texture is province-addressed and evolves
/// independently from the strategic-region weather phenomenon.
#[derive(Debug, Clone, PartialEq)]
pub struct SnowMudCell {
    pub region_id: Leaf<u32>,
    pub snow_level_0_255: Leaf<u32>,
    pub mud_level_0_255: Leaf<u32>,
}

/// Accumulated snow/mud ground-cover view (requirements item 6): per-province
/// levels plus the fade phase/window that drives the visual blend. Distinct
/// from [`WeatherRegionRow`], which carries the region-level phenomenon.
#[derive(Debug, Clone, PartialEq)]
pub struct SnowMudView {
    pub status: ViewStatus,
    pub by_province: BTreeMap<u32, SnowMudCell>,
    pub fade_phase: Leaf<f64>,
    pub fade_0_hour: Leaf<i64>,
    pub fade_1_hour: Leaf<i64>,
}

impl SnowMudView {
    fn with_status(status: ViewStatus) -> SnowMudView {
        let (phase, h0, h1) = match status {
            ViewStatus::DomainAbsent => (Leaf::Absent, Leaf::Absent, Leaf::Absent),
            _ => (Leaf::Unobtained, Leaf::Unobtained, Leaf::Unobtained),
        };
        SnowMudView {
            status,
            by_province: BTreeMap::new(),
            fade_phase: phase,
            fade_0_hour: h0,
            fade_1_hour: h1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CountryIdentity {
    pub cosmetic_tag: Leaf<String>,
    pub ruling_ideology: Leaf<String>,
}

impl CountryIdentity {
    fn all_unobtained() -> CountryIdentity {
        CountryIdentity {
            cosmetic_tag: Leaf::Unobtained,
            ruling_ideology: Leaf::Unobtained,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CountryDiplomacy {
    pub faction_name: Leaf<String>,
    pub faction_is_leader: Leaf<bool>,
    pub capital_state: Leaf<u32>,
}

impl CountryDiplomacy {
    fn all_unobtained() -> CountryDiplomacy {
        CountryDiplomacy {
            faction_name: Leaf::Unobtained,
            faction_is_leader: Leaf::Unobtained,
            capital_state: Leaf::Unobtained,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiplomaticPairState {
    pub at_war: Leaf<bool>,
    pub same_faction: Leaf<bool>,
    pub non_aggression_pact: Leaf<bool>,
    pub military_access: Leaf<bool>,
    pub guarantee: Leaf<bool>,
    pub embargo: Leaf<bool>,
    pub subject: Leaf<bool>,
}

impl DiplomaticPairState {
    fn all_absent() -> DiplomaticPairState {
        DiplomaticPairState {
            at_war: Leaf::Absent,
            same_faction: Leaf::Absent,
            non_aggression_pact: Leaf::Absent,
            military_access: Leaf::Absent,
            guarantee: Leaf::Absent,
            embargo: Leaf::Absent,
            subject: Leaf::Absent,
        }
    }

    fn all_unobtained() -> DiplomaticPairState {
        DiplomaticPairState {
            at_war: Leaf::Unobtained,
            same_faction: Leaf::Unobtained,
            non_aggression_pact: Leaf::Unobtained,
            military_access: Leaf::Unobtained,
            guarantee: Leaf::Unobtained,
            embargo: Leaf::Unobtained,
            subject: Leaf::Unobtained,
        }
    }
}

/// One live war-relation row from `diplomacy_live.war_relations` (requirements
/// item 8b): the ordered belligerent pair plus the running casualty tallies.
/// This is a LIVE feed distinct from the boolean `pairs` matrix (which folds
/// `wars[]`/relation lists into directional `at_war`); it carries the casualty
/// magnitudes the boolean matrix cannot. It is deliberately NOT folded into
/// `pairs` so the certified directional war matrix is preserved verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct WarRelationRow {
    pub first: Leaf<String>,
    pub second: Leaf<String>,
    pub first_casualties: Leaf<i64>,
    pub second_casualties: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiplomacyView {
    pub status: ViewStatus,
    pub pairs: BTreeMap<(String, String), DiplomaticPairState>,
    pub countries: BTreeMap<String, CountryDiplomacy>,
    /// Live war-relation rows from `diplomacy_live.war_relations`
    /// (`Absent` when the key never appears, `Unobtained` when explicit-null).
    pub war_relations: Leaf<Vec<WarRelationRow>>,
    /// Faction membership from witnessed d53 rows, falling back to the
    /// recording's `diplomacy_live` faction id → member TAG map when the d53
    /// status predates the recording. Source absence/null remains distinct.
    pub faction_members: Leaf<BTreeMap<String, Vec<String>>>,
    /// Per-owner relation-flag arrays from
    /// `countries.<owner>.diplomacy.relation_flags` (`{other_tag: [flag,...]}`).
    /// Surfaced verbatim (never coerced into the boolean `pairs` matrix, whose
    /// directional semantics are certified): the entry is `Measured` for an
    /// object, `Unobtained` for explicit-null; an owner with no relation_flags
    /// key is simply absent from the map.
    pub relation_flags: BTreeMap<String, Leaf<BTreeMap<String, Vec<String>>>>,
}

impl DiplomacyView {
    /// Recorder d53 faction-roster completeness. The existing faction_members
    /// Leaf is the atomic carrier: `Unobtained` is unavailable, while a
    /// measured map distinguishes empty from nonempty. Absence is never a
    /// completeness witness and therefore fails closed.
    pub fn faction_roster_status(&self) -> FactionRosterStatus {
        match &self.faction_members {
            Leaf::Measured(rows) if rows.is_empty() => FactionRosterStatus::MeasuredEmpty,
            Leaf::Measured(_) => FactionRosterStatus::MeasuredNonEmpty,
            Leaf::Unobtained | Leaf::Absent => FactionRosterStatus::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TopbarScalars {
    pub political_power: Leaf<f64>,
    pub stability: Leaf<f64>,
    pub war_support: Leaf<f64>,
    pub command_power: Leaf<f64>,
    pub manpower_available: Leaf<i64>,
    pub xp_army: Leaf<f64>,
    pub xp_navy: Leaf<f64>,
    pub xp_air: Leaf<f64>,
    pub factories_civ: Leaf<i64>,
    pub factories_mil: Leaf<i64>,
    pub factories_dockyard: Leaf<i64>,
    pub fuel_current: Leaf<f64>,
    pub fuel_max: Leaf<f64>,
    pub convoys: Leaf<f64>,
    pub nukes_total: Leaf<i64>,
    pub nukes_ready: Leaf<i64>,
    pub world_tension: Leaf<f64>,
    pub supply_value: Leaf<f64>,
    pub supply_ratio: Leaf<f64>,
}

impl TopbarScalars {
    fn all_unobtained() -> TopbarScalars {
        TopbarScalars {
            political_power: Leaf::Unobtained,
            stability: Leaf::Unobtained,
            war_support: Leaf::Unobtained,
            command_power: Leaf::Unobtained,
            manpower_available: Leaf::Unobtained,
            xp_army: Leaf::Unobtained,
            xp_navy: Leaf::Unobtained,
            xp_air: Leaf::Unobtained,
            factories_civ: Leaf::Unobtained,
            factories_mil: Leaf::Unobtained,
            factories_dockyard: Leaf::Unobtained,
            fuel_current: Leaf::Unobtained,
            fuel_max: Leaf::Unobtained,
            convoys: Leaf::Unobtained,
            nukes_total: Leaf::Unobtained,
            nukes_ready: Leaf::Unobtained,
            world_tension: Leaf::Unobtained,
            supply_value: Leaf::Unobtained,
            supply_ratio: Leaf::Unobtained,
        }
    }
}

/// Per-POV topbar ratio bars (requirements item 9b): the derived
/// industrial / convoy / fuel / xp fractions the HUD renders as fill bars.
/// Sourced from `countries.<tag>.scalars.*` (the topbar scalar sweep bag);
/// each field independently degrades to `Absent`/`Unobtained`.
#[derive(Debug, Clone, PartialEq)]
pub struct TopbarRatios {
    pub industrial_ratio: Leaf<f64>,
    pub convoy_ratio: Leaf<f64>,
    pub fuel_ratio: Leaf<f64>,
    pub xp_ratio: Leaf<f64>,
}

impl TopbarRatios {
    fn all_unobtained() -> TopbarRatios {
        TopbarRatios {
            industrial_ratio: Leaf::Unobtained,
            convoy_ratio: Leaf::Unobtained,
            fuel_ratio: Leaf::Unobtained,
            xp_ratio: Leaf::Unobtained,
        }
    }
}

/// Per-POV topbar alert / notification counts (requirements item 9b).
/// Sourced from `countries.<tag>.scalars.alerts.*`. Only the fields the
/// schema oracle (fixture generator) carries are modeled; each degrades
/// independently. (The topbar also shows prototype/decision-timeout alerts;
/// those are not yet in the recording schema — see `EXTRACTION_NEEDS.md §A`.)
#[derive(Debug, Clone, PartialEq)]
pub struct TopbarAlerts {
    pub initiative: Leaf<i64>,
    pub decisions_available: Leaf<i64>,
    pub intel: Leaf<i64>,
    pub market_contracts: Leaf<i64>,
}

impl TopbarAlerts {
    fn all_unobtained() -> TopbarAlerts {
        TopbarAlerts {
            initiative: Leaf::Unobtained,
            decisions_available: Leaf::Unobtained,
            intel: Leaf::Unobtained,
            market_contracts: Leaf::Unobtained,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildingStatus {
    pub level: Leaf<f64>,
    pub healthy_level: Leaf<f64>,
    pub damaged: Leaf<bool>,
    pub damage_fraction: Leaf<f64>,
    pub repair_progress: Leaf<f64>,
    pub under_construction: Leaf<bool>,
}

impl BuildingStatus {
    fn all_absent() -> BuildingStatus {
        BuildingStatus {
            level: Leaf::Absent,
            healthy_level: Leaf::Absent,
            damaged: Leaf::Absent,
            damage_fraction: Leaf::Absent,
            repair_progress: Leaf::Absent,
            under_construction: Leaf::Absent,
        }
    }

    fn level_only(level: f64) -> BuildingStatus {
        BuildingStatus {
            level: Leaf::Measured(level),
            ..BuildingStatus::all_absent()
        }
    }

    fn all_unobtained() -> BuildingStatus {
        BuildingStatus {
            level: Leaf::Unobtained,
            healthy_level: Leaf::Unobtained,
            damaged: Leaf::Unobtained,
            damage_fraction: Leaf::Unobtained,
            repair_progress: Leaf::Unobtained,
            under_construction: Leaf::Unobtained,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateBuildings {
    pub status: ViewStatus,
    pub state_buildings: BTreeMap<String, BuildingStatus>,
    pub province_buildings: BTreeMap<u32, BTreeMap<String, BuildingStatus>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProvinceVisibility {
    pub level: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpottedUnitsView {
    pub status: ViewStatus,
    pub divisions: Leaf<Vec<String>>,
    pub fleets: Leaf<Vec<String>>,
    pub wings: Leaf<Vec<String>>,
}

impl SpottedUnitsView {
    fn with_status(status: ViewStatus) -> SpottedUnitsView {
        let leaf = match status {
            ViewStatus::DomainAbsent => Leaf::Absent,
            ViewStatus::DomainUnavailable => Leaf::Unobtained,
            ViewStatus::Present => Leaf::Absent,
        };
        SpottedUnitsView {
            status,
            divisions: leaf.clone(),
            fleets: leaf.clone(),
            wings: leaf,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PovVisibilityView {
    pub status: ViewStatus,
    pub provinces: BTreeMap<u32, Leaf<ProvinceVisibility>>,
    pub spotted_units: SpottedUnitsView,
}

impl PovVisibilityView {
    fn with_status(status: ViewStatus) -> PovVisibilityView {
        PovVisibilityView {
            status,
            provinces: BTreeMap::new(),
            spotted_units: SpottedUnitsView::with_status(status),
        }
    }
}

/// Per-TAG country core values for the topbar surface (REPLAY_SCHEMA.md
/// §3.1 fuel, §3.2 division_count, §3.3 politics).
#[derive(Debug, Clone, PartialEq)]
pub struct CountryCore {
    pub stability: Leaf<f64>,
    pub war_support: Leaf<f64>,
    pub political_power: Leaf<f64>,
    pub ruling_party: Leaf<String>,
    pub fuel_current: Leaf<f64>,
    pub fuel_max: Leaf<f64>,
    pub division_count: Leaf<i64>,
}

impl CountryCore {
    /// A present-but-null (or wrong-typed) country entry: every leaf is
    /// captured-but-unobtained, distinct from an absent country.
    fn all_unobtained() -> CountryCore {
        CountryCore {
            stability: Leaf::Unobtained,
            war_support: Leaf::Unobtained,
            political_power: Leaf::Unobtained,
            ruling_party: Leaf::Unobtained,
            fuel_current: Leaf::Unobtained,
            fuel_max: Leaf::Unobtained,
            division_count: Leaf::Unobtained,
        }
    }
}

/// Coverage of one provenance channel (REPLAY_SCHEMA.md §2).
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelCoverage {
    pub captured: Leaf<bool>,
    pub domains: Vec<String>,
    pub problems: Vec<DomainProblem>,
}

/// One recorder-reported problem tied to a numeric compact domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainProblem {
    pub domain: i64,
    pub problem: String,
}

/// Per-frame provenance: which domains each channel (collector/memory/save)
/// captured, plus the writer's free-form notes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProvenanceView {
    pub channels: BTreeMap<String, ChannelCoverage>,
    pub notes: Vec<String>,
}

/// World state at one canonical hour: the raw domain map plus typed views.
pub struct WorldFrame {
    hour_index: i64,
    date: Option<String>,
    save_version: Option<String>,
    raw: Map<String, Value>,
    owner_by_state: BTreeMap<u32, Leaf<String>>,
    owner_by_state_source: MapLayerSource,
    owner_by_state_status: ViewStatus,
    control_by_state: BTreeMap<u32, Leaf<String>>,
    control_by_state_source: MapLayerSource,
    control_by_state_status: ViewStatus,
    control_by_province: BTreeMap<u32, Leaf<String>>,
    control_by_province_source: MapLayerSource,
    control_by_province_status: ViewStatus,
    divisions: Vec<DivisionRow>,
    division_units: Vec<DivisionUnitRow>,
    division_units_status: ViewStatus,
    fleets: Vec<FleetUnitRow>,
    fleets_status: ViewStatus,
    wings: Vec<WingUnitRow>,
    wings_status: ViewStatus,
    combats: Vec<CombatRow>,
    countries: BTreeMap<String, CountryCore>,
    countries_status: ViewStatus,
    session_metadata: SessionMetadataView,
    naval_supremacy_by_region: BTreeMap<u32, NavalSupremacyRow>,
    naval_supremacy_status: ViewStatus,
    air_superiority_by_region: BTreeMap<u32, AirSuperiorityRow>,
    air_superiority_status: ViewStatus,
    weather_by_region: BTreeMap<u32, WeatherRegionRow>,
    weather_status: ViewStatus,
    snow_mud: SnowMudView,
    country_identities: BTreeMap<String, CountryIdentity>,
    country_identities_status: ViewStatus,
    diplomacy: DiplomacyView,
    topbar_scalars_by_country: BTreeMap<String, TopbarScalars>,
    topbar_status: ViewStatus,
    topbar_ratios_by_country: BTreeMap<String, TopbarRatios>,
    topbar_alerts_by_country: BTreeMap<String, TopbarAlerts>,
    topbar_extras_status: ViewStatus,
    buildings_by_state: BTreeMap<u32, StateBuildings>,
    buildings_status: ViewStatus,
    visibility_by_pov: BTreeMap<String, PovVisibilityView>,
    visibility_status: ViewStatus,
    // --- fx-streams lane: the 12 UI-inventory data streams (UI_COVERAGE_MATRIX §9) ---
    research_by_country: BTreeMap<String, ResearchView>,
    research_status: ViewStatus,
    focus_by_country: BTreeMap<String, FocusView>,
    focus_status: ViewStatus,
    politics_detail_by_country: BTreeMap<String, PoliticsDetailView>,
    politics_detail_status: ViewStatus,
    production_by_country: BTreeMap<String, ProductionView>,
    production_status: ViewStatus,
    intel_by_country: BTreeMap<String, IntelView>,
    intel_status: ViewStatus,
    trade_by_country: BTreeMap<String, TradeView>,
    trade_status: ViewStatus,
    leaders_by_country: BTreeMap<String, LeaderRosterView>,
    leaders_status: ViewStatus,
    occupation_by_state: BTreeMap<u32, OccupationRow>,
    occupation_status: ViewStatus,
    power_balance: PowerBalanceView,
    designs: DesignsView,
    naval_air_combat: NavalAirCombatView,
    scripted_gui: ScriptedGuiView,
    raids: RaidStateView,
    provenance: ProvenanceView,
    coverage_warnings: Vec<String>,
}

impl WorldFrame {
    pub fn hour_index(&self) -> i64 {
        self.hour_index
    }
    pub fn date(&self) -> Option<&str> {
        self.date.as_deref()
    }
    pub fn save_version(&self) -> Option<&str> {
        self.save_version.as_deref()
    }
    /// Every top-level domain of the snapshot, raw and untouched (unknown
    /// schema growth stays reachable here).
    pub fn raw(&self) -> &Map<String, Value> {
        &self.raw
    }
    pub fn domain(&self, key: &str) -> Option<&Value> {
        self.raw.get(key)
    }
    pub fn owner_by_state(&self) -> &BTreeMap<u32, Leaf<String>> {
        &self.owner_by_state
    }
    pub fn owner_by_state_source(&self) -> MapLayerSource {
        self.owner_by_state_source
    }
    pub fn owner_by_state_status(&self) -> ViewStatus {
        self.owner_by_state_status
    }
    /// Per-state controller (occupier when it differs from the owner).
    /// Schema §8 defines no `map.control_by_state` mirror today, so this
    /// normally comes from the `states.*.controller` domain walk; a future
    /// writer-provided mirror under that key is honored if present.
    pub fn control_by_state(&self) -> &BTreeMap<u32, Leaf<String>> {
        &self.control_by_state
    }
    pub fn control_by_state_source(&self) -> MapLayerSource {
        self.control_by_state_source
    }
    pub fn control_by_state_status(&self) -> ViewStatus {
        self.control_by_state_status
    }
    pub fn control_by_province(&self) -> &BTreeMap<u32, Leaf<String>> {
        &self.control_by_province
    }
    pub fn control_by_province_source(&self) -> MapLayerSource {
        self.control_by_province_source
    }
    pub fn control_by_province_status(&self) -> ViewStatus {
        self.control_by_province_status
    }
    pub fn divisions(&self) -> &[DivisionRow] {
        &self.divisions
    }
    pub fn division_units(&self) -> &[DivisionUnitRow] {
        &self.division_units
    }
    pub fn division_units_status(&self) -> ViewStatus {
        self.division_units_status
    }
    pub fn fleets(&self) -> &[FleetUnitRow] {
        &self.fleets
    }
    pub fn fleets_status(&self) -> ViewStatus {
        self.fleets_status
    }
    pub fn wings(&self) -> &[WingUnitRow] {
        &self.wings
    }
    pub fn wings_status(&self) -> ViewStatus {
        self.wings_status
    }
    pub fn combats(&self) -> &[CombatRow] {
        &self.combats
    }
    pub fn countries(&self) -> &BTreeMap<String, CountryCore> {
        &self.countries
    }
    pub fn countries_status(&self) -> ViewStatus {
        self.countries_status
    }
    pub fn country(&self, tag: &str) -> Option<&CountryCore> {
        self.countries.get(tag)
    }
    pub fn session_metadata(&self) -> &SessionMetadataView {
        &self.session_metadata
    }
    pub fn naval_supremacy_by_region(&self) -> &BTreeMap<u32, NavalSupremacyRow> {
        &self.naval_supremacy_by_region
    }
    pub fn naval_supremacy_status(&self) -> ViewStatus {
        self.naval_supremacy_status
    }
    /// Per-strategic-region air-superiority contest (requirements item 5),
    /// sourced from `naval_air.air_superiority`. Distinct from
    /// [`WorldFrame::naval_supremacy_by_region`] (naval region control).
    pub fn air_superiority_by_region(&self) -> &BTreeMap<u32, AirSuperiorityRow> {
        &self.air_superiority_by_region
    }
    pub fn air_superiority(&self, region_id: u32) -> Option<&AirSuperiorityRow> {
        self.air_superiority_by_region.get(&region_id)
    }
    pub fn air_superiority_status(&self) -> ViewStatus {
        self.air_superiority_status
    }
    pub fn weather_by_region(&self) -> &BTreeMap<u32, WeatherRegionRow> {
        &self.weather_by_region
    }
    pub fn weather_status(&self) -> ViewStatus {
        self.weather_status
    }
    /// Accumulated per-province snow/mud ground cover and its fade window
    /// (requirements item 6), independent of the region weather phenomenon.
    pub fn snow_mud(&self) -> &SnowMudView {
        &self.snow_mud
    }
    pub fn snow_mud_status(&self) -> ViewStatus {
        self.snow_mud.status
    }
    pub fn country_identities(&self) -> &BTreeMap<String, CountryIdentity> {
        &self.country_identities
    }
    pub fn country_identity(&self, tag: &str) -> Option<&CountryIdentity> {
        self.country_identities.get(tag)
    }
    pub fn country_identities_status(&self) -> ViewStatus {
        self.country_identities_status
    }
    pub fn diplomacy(&self) -> &DiplomacyView {
        &self.diplomacy
    }
    pub fn diplomatic_pair_state(&self, owner: &str, viewed: &str) -> Option<&DiplomaticPairState> {
        self.diplomacy
            .pairs
            .get(&(owner.to_string(), viewed.to_string()))
    }
    pub fn topbar_scalars_by_country(&self) -> &BTreeMap<String, TopbarScalars> {
        &self.topbar_scalars_by_country
    }
    pub fn topbar_scalars(&self, tag: &str) -> Option<&TopbarScalars> {
        self.topbar_scalars_by_country.get(tag)
    }
    pub fn topbar_status(&self) -> ViewStatus {
        self.topbar_status
    }
    /// Per-POV topbar ratio bars (requirements item 9b), sourced from the
    /// `countries.<tag>.scalars` bag. Additive companion to
    /// [`WorldFrame::topbar_scalars_by_country`].
    pub fn topbar_ratios_by_country(&self) -> &BTreeMap<String, TopbarRatios> {
        &self.topbar_ratios_by_country
    }
    pub fn topbar_ratios(&self, tag: &str) -> Option<&TopbarRatios> {
        self.topbar_ratios_by_country.get(tag)
    }
    /// Per-POV topbar alert / notification counts (requirements item 9b),
    /// sourced from `countries.<tag>.scalars.alerts`.
    pub fn topbar_alerts_by_country(&self) -> &BTreeMap<String, TopbarAlerts> {
        &self.topbar_alerts_by_country
    }
    pub fn topbar_alerts(&self, tag: &str) -> Option<&TopbarAlerts> {
        self.topbar_alerts_by_country.get(tag)
    }
    /// Domain status for the ratio-bar / alert surfaces (mirrors
    /// [`WorldFrame::topbar_status`]; keyed off the same `countries` domain).
    pub fn topbar_extras_status(&self) -> ViewStatus {
        self.topbar_extras_status
    }
    pub fn buildings_by_state(&self) -> &BTreeMap<u32, StateBuildings> {
        &self.buildings_by_state
    }
    pub fn buildings_status(&self) -> ViewStatus {
        self.buildings_status
    }
    /// Per-POV visibility. Borrows the frame's view when the POV exists
    /// (F2: the province map can be large and cloning it per call was
    /// per-seek garbage); a POV the frame does not carry synthesizes an
    /// allocation-free fail-closed view (empty map, Absent/Unobtained leaves
    /// — never fabricated visibility).
    pub fn visibility_for(&self, pov_tag: &str) -> Cow<'_, PovVisibilityView> {
        match self.visibility_by_pov.get(pov_tag) {
            Some(view) => Cow::Borrowed(view),
            None => {
                let status = match self.visibility_status {
                    ViewStatus::Present => ViewStatus::DomainAbsent,
                    other => other,
                };
                Cow::Owned(PovVisibilityView::with_status(status))
            }
        }
    }
    pub fn spotted_units_for(&self, pov_tag: &str) -> SpottedUnitsView {
        match self.visibility_for(pov_tag) {
            Cow::Borrowed(view) => view.spotted_units.clone(),
            Cow::Owned(view) => view.spotted_units,
        }
    }
    pub fn visibility_status(&self) -> ViewStatus {
        self.visibility_status
    }
    pub fn visibility_povs(&self) -> impl Iterator<Item = &str> {
        self.visibility_by_pov.keys().map(String::as_str)
    }
    pub fn provenance(&self) -> &ProvenanceView {
        &self.provenance
    }
    /// Present-but-wrong-typed values and other typed-view findings. Each
    /// wrong-typed leaf is `Unobtained` in the view AND listed here.
    pub fn coverage_warnings(&self) -> &[String] {
        &self.coverage_warnings
    }

    fn from_raw(frame: RawFrame) -> WorldFrame {
        let mut warnings: Vec<String> = Vec::new();
        let (owner_by_state, owner_by_state_source, owner_by_state_status) = build_tag_map(
            &frame.rest,
            "owner_by_state",
            "states",
            "owner",
            &mut warnings,
        );
        let (control_by_state, control_by_state_source, control_by_state_status) = build_tag_map(
            &frame.rest,
            "control_by_state",
            "states",
            "controller",
            &mut warnings,
        );
        let (control_by_province, control_by_province_source, control_by_province_status) =
            build_tag_map(
                &frame.rest,
                "control_by_province",
                "provinces",
                "controller",
                &mut warnings,
            );
        let (division_units, mut division_units_status) =
            build_division_units(&frame.rest, &mut warnings);
        let (fleets, mut fleets_status) = build_fleets(&frame.rest, &mut warnings);
        let (wings, wings_status) = build_wings(&frame.rest, &mut warnings);
        let divisions = build_divisions(&frame.rest, &mut warnings);
        let combats = build_combats(&frame.rest, &mut warnings);
        let (countries, countries_status) = build_countries(&frame.rest, &mut warnings);
        let session_metadata = build_session_metadata(&frame.rest, &mut warnings);
        let (naval_supremacy_by_region, naval_supremacy_status) =
            build_naval_supremacy(&frame.rest, &mut warnings);
        let (air_superiority_by_region, air_superiority_status) =
            build_air_superiority(&frame.rest, &mut warnings);
        let (weather_by_region, weather_status) = build_weather(&frame.rest, &mut warnings);
        let snow_mud = build_snow_mud(&frame.rest, &mut warnings);
        let (country_identities, country_identities_status) =
            build_country_identities(&frame.rest, &mut warnings);
        let mut diplomacy = build_diplomacy(&frame.rest, &mut warnings);
        let (topbar_scalars_by_country, topbar_status) =
            build_topbar_scalars(&frame.rest, &mut warnings);
        let (topbar_ratios_by_country, topbar_alerts_by_country, topbar_extras_status) =
            build_topbar_extras(&frame.rest, &mut warnings);
        let (buildings_by_state, buildings_status) = build_buildings(&frame.rest, &mut warnings);
        let (visibility_by_pov, visibility_status) = build_visibility(&frame.rest, &mut warnings);
        let (research_by_country, research_status) = build_research(&frame.rest, &mut warnings);
        let (focus_by_country, mut focus_status) = build_focus(&frame.rest, &mut warnings);
        let (politics_detail_by_country, politics_detail_status) =
            build_politics_detail(&frame.rest, &mut warnings);
        let (production_by_country, mut production_status) =
            build_production(&frame.rest, &mut warnings);
        let (intel_by_country, intel_status) = build_intel(&frame.rest, &mut warnings);
        let (trade_by_country, trade_status) = build_trade(&frame.rest, &mut warnings);
        let (leaders_by_country, mut leaders_status) = build_leaders(&frame.rest, &mut warnings);
        let (occupation_by_state, occupation_status) = build_occupation(&frame.rest, &mut warnings);
        let power_balance = build_power_balance(&frame.rest, &mut warnings);
        let designs = build_designs(&frame.rest, &mut warnings);
        let naval_air_combat = build_naval_air_combat(&frame.rest, &mut warnings);
        let scripted_gui = build_scripted_gui(&frame.rest, &mut warnings);
        let raids = build_raids(&frame.rest, &mut warnings);
        let provenance = build_provenance(&frame.rest, &mut warnings);
        reconcile_domain_problem_statuses(
            &frame.rest,
            &mut production_status,
            &mut division_units_status,
            &mut fleets_status,
            &mut focus_status,
            &mut leaders_status,
            &mut diplomacy.status,
        );
        WorldFrame {
            hour_index: frame.hour_index,
            date: frame.date,
            save_version: frame.save_version,
            raw: frame.rest,
            owner_by_state,
            owner_by_state_source,
            owner_by_state_status,
            control_by_state,
            control_by_state_source,
            control_by_state_status,
            control_by_province,
            control_by_province_source,
            control_by_province_status,
            divisions,
            division_units,
            division_units_status,
            fleets,
            fleets_status,
            wings,
            wings_status,
            combats,
            countries,
            countries_status,
            session_metadata,
            naval_supremacy_by_region,
            naval_supremacy_status,
            air_superiority_by_region,
            air_superiority_status,
            weather_by_region,
            weather_status,
            snow_mud,
            country_identities,
            country_identities_status,
            diplomacy,
            topbar_scalars_by_country,
            topbar_status,
            topbar_ratios_by_country,
            topbar_alerts_by_country,
            topbar_extras_status,
            buildings_by_state,
            buildings_status,
            visibility_by_pov,
            visibility_status,
            research_by_country,
            research_status,
            focus_by_country,
            focus_status,
            politics_detail_by_country,
            politics_detail_status,
            production_by_country,
            production_status,
            intel_by_country,
            intel_status,
            trade_by_country,
            trade_status,
            leaders_by_country,
            leaders_status,
            occupation_by_state,
            occupation_status,
            power_balance,
            designs,
            naval_air_combat,
            scripted_gui,
            raids,
            provenance,
            coverage_warnings: warnings,
        }
    }
}

/// Open-time memory budget for a [`WorldTimeline`] (plan F2: the replay must
/// play multi-year recordings on weak PCs).
///
/// The decoded-frame LRU cache is the only tunable O(1)-per-timeline
/// structure whose size is a budget; everything else the timeline retains is
/// either the per-open hour-index spine (8 B per recorded hour — see
/// [`WorldTimeline::open`]) or the frames the cache itself owns. The budget
/// is expressed in ENTRIES, not bytes: a decoded frame's heap size depends on
/// recording content and `replay_core` will not estimate it (an estimated
/// byte figure would be a fabricated value). The operator-facing byte budget
/// is `entries x observed frame size`, measured by the soak harnesses
/// (`replay_app/tests/soak.rs`, `soak_f2.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineConfig {
    /// Maximum decoded frames kept hot (LRU). Default [`FRAME_CACHE_CAP`]
    /// (16, DERIVED: the pre-F2 fixed cap, soak-proven bounded). `0` disables
    /// caching entirely: every access re-decodes from the verified source —
    /// still correct (`seek == play_to` holds), minimum working set.
    pub frame_cache_entries: usize,
}

impl Default for TimelineConfig {
    fn default() -> TimelineConfig {
        TimelineConfig {
            frame_cache_entries: FRAME_CACHE_CAP,
        }
    }
}

/// Ordered, scrubbable series of world frames over one recording.
pub struct WorldTimeline {
    source: Box<dyn ReplaySource>,
    hours: Vec<i64>,
    config: TimelineConfig,
    cache: Mutex<FrameCache>,
}

/// Result of resolving an arbitrary requested hour against the snapshot spine.
///
/// This is the A4 seam: today it resolves to the nearest snapshot at-or-before
/// the request. When snapshot+delta reconstruction lands, `frame_hour` remains
/// the anchor snapshot and the delta window can hang off this lookup without
/// changing callers that already test `seek(h) == play_to(h)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineLookup {
    pub requested_hour: i64,
    pub frame_hour: i64,
    pub exact_match: bool,
    pub next_hour: Option<i64>,
}

impl WorldTimeline {
    /// Wrap a source, validating its hour series (non-empty, strictly
    /// increasing — the source contract, revalidated here because everything
    /// downstream scrubs by binary search over it). Uses the default memory
    /// budget ([`TimelineConfig::default`], 16 cached frames).
    pub fn open(source: Box<dyn ReplaySource>) -> Result<WorldTimeline, WorldError> {
        WorldTimeline::open_with_config(source, TimelineConfig::default())
    }

    /// As [`WorldTimeline::open`] with an explicit memory budget (F2): a
    /// weak-PC profile shrinks `frame_cache_entries`; the default stays 16.
    ///
    /// F2 audit note (justified O(recorded-hours) floor): this `to_vec()` is
    /// a defensive copy of the source's validated hour spine — 8 B per
    /// recorded hour (210,240 B for a 3-year hourly recording), the second of
    /// exactly two such spines (the source keeps its own). Index spines are
    /// NOT frame payloads and are the documented floor of the timeline's
    /// working set; the frame cache above it is the only budgeted structure.
    pub fn open_with_config(
        source: Box<dyn ReplaySource>,
        config: TimelineConfig,
    ) -> Result<WorldTimeline, WorldError> {
        let hours = source.hours().to_vec();
        if hours.is_empty() {
            return Err(WorldError::Timeline(format!(
                "recording {:?} has no playable hours",
                source.game_id()
            )));
        }
        for pair in hours.windows(2) {
            if pair[0] >= pair[1] {
                return Err(WorldError::Timeline(format!(
                    "hour series is not strictly increasing ({} then {})",
                    pair[0], pair[1]
                )));
            }
        }
        Ok(WorldTimeline {
            source,
            hours,
            config,
            cache: Mutex::new(FrameCache::new(config.frame_cache_entries)),
        })
    }

    /// The frame-cache entry budget this timeline was opened with.
    pub fn frame_cache_entries(&self) -> usize {
        self.config.frame_cache_entries
    }

    pub fn game_id(&self) -> &str {
        self.source.game_id()
    }
    pub fn hours(&self) -> &[i64] {
        &self.hours
    }
    pub fn first_hour(&self) -> i64 {
        self.hours[0]
    }
    pub fn last_hour(&self) -> i64 {
        self.hours[self.hours.len() - 1]
    }
    /// Ingest warnings from the source (skipped index entries, duplicates).
    pub fn warnings(&self) -> &[String] {
        self.source.warnings()
    }

    pub fn lookup_hour(&self, hour_index: i64) -> Result<TimelineLookup, WorldError> {
        let first = self.first_hour();
        if hour_index < first {
            return Err(WorldError::BeforeFirst {
                hour: hour_index,
                first,
            });
        }
        match self.hours.binary_search(&hour_index) {
            Ok(i) => Ok(TimelineLookup {
                requested_hour: hour_index,
                frame_hour: self.hours[i],
                exact_match: true,
                next_hour: self.hours.get(i + 1).copied(),
            }),
            Err(i) => Ok(TimelineLookup {
                requested_hour: hour_index,
                frame_hour: self.hours[i - 1],
                exact_match: false,
                next_hour: self.hours.get(i).copied(),
            }),
        }
    }

    /// The frame at exactly `hour_index`; [`WorldError::NoSuchHour`] otherwise.
    pub fn frame(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError> {
        if self.hours.binary_search(&hour_index).is_err() {
            return Err(WorldError::NoSuchHour(hour_index));
        }
        self.load_cached(hour_index)
    }

    /// The nearest frame at-or-before `hour_index` (the scrub primitive);
    /// [`WorldError::BeforeFirst`] before the first recorded frame.
    pub fn state_at(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError> {
        let lookup = self.lookup_hour(hour_index)?;
        self.load_cached(lookup.frame_hour)
    }

    /// Random-access seek. This is equivalent to [`WorldTimeline::state_at`]
    /// until compact delta replay is wired behind [`TimelineLookup`].
    pub fn seek(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError> {
        self.state_at(hour_index)
    }

    /// Linear play-to hook for A4 equality tests. It deliberately resolves
    /// through the same index today; a future delta walker should preserve
    /// `seek(h) == play_to(h)` for all typed views.
    pub fn play_to(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError> {
        self.state_at(hour_index)
    }

    fn load_cached(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError> {
        if let Some(frame) = self.cache.lock().expect("frame cache lock").get(hour_index) {
            return Ok(frame);
        }
        let raw = self.source.frame(hour_index)?;
        let frame = Arc::new(WorldFrame::from_raw(raw));
        self.cache
            .lock()
            .expect("frame cache lock")
            .insert(hour_index, frame.clone());
        Ok(frame)
    }
}

/// Tiny LRU (oldest first); linear scans are fine at the configured cap
/// (a budget-sized constant — [`TimelineConfig::frame_cache_entries`],
/// default 16 — never timeline-sized). A cap of 0 disables caching: every
/// insert is immediately evicted, correctness is unchanged.
struct FrameCache {
    cap: usize,
    entries: Vec<(i64, Arc<WorldFrame>)>,
}

impl FrameCache {
    fn new(cap: usize) -> FrameCache {
        FrameCache {
            cap,
            entries: Vec::with_capacity(cap),
        }
    }
    fn get(&mut self, hour: i64) -> Option<Arc<WorldFrame>> {
        let pos = self.entries.iter().position(|(h, _)| *h == hour)?;
        let entry = self.entries.remove(pos);
        let frame = entry.1.clone();
        self.entries.push(entry);
        Some(frame)
    }
    fn insert(&mut self, hour: i64, frame: Arc<WorldFrame>) {
        if let Some(pos) = self.entries.iter().position(|(h, _)| *h == hour) {
            self.entries.remove(pos);
        }
        self.entries.push((hour, frame));
        if self.entries.len() > self.cap {
            self.entries.remove(0);
        }
    }
}

// ---------------------------------------------------------------------------
// typed-view builders (no coercion; wrong types -> Unobtained + warning)
// ---------------------------------------------------------------------------

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Walk `path` under `obj`. `Absent` if any key on the path is absent;
/// `Unobtained` for an explicit null anywhere on the path (an unobtained
/// group makes its leaves unobtained, not absent); `Measured(&Value)` for a
/// non-null leaf. A wrong-typed intermediate is `Unobtained` + a warning.
fn walk_path<'a>(
    mut obj: &'a Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<&'a Value> {
    for (i, seg) in path.iter().enumerate() {
        let v = match obj.get(*seg) {
            None => return Leaf::Absent,
            Some(v) => v,
        };
        if i + 1 == path.len() {
            return if v.is_null() {
                Leaf::Unobtained
            } else {
                Leaf::Measured(v)
            };
        }
        match v {
            Value::Null => return Leaf::Unobtained,
            Value::Object(m) => obj = m,
            other => {
                warnings.push(format!(
                    "{ctx}.{}: expected object, found {}; treated as unobtained",
                    path[..=i].join("."),
                    json_type_name(other)
                ));
                return Leaf::Unobtained;
            }
        }
    }
    Leaf::Absent
}

fn typed_leaf<'a, T, F>(
    obj: &'a Map<String, Value>,
    path: &[&str],
    want: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
    convert: F,
) -> Leaf<T>
where
    F: FnOnce(&'a Value) -> Option<T>,
{
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(v) => match convert(v) {
            Some(t) => Leaf::Measured(t),
            None => {
                warnings.push(format!(
                    "{ctx}.{}: present but wrong-typed (expected {want}, found {}); \
                     treated as unobtained",
                    path.join("."),
                    json_type_name(v)
                ));
                Leaf::Unobtained
            }
        },
    }
}

fn leaf_f64(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<f64> {
    typed_leaf(obj, path, "number", ctx, warnings, Value::as_f64)
}

fn leaf_i64(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<i64> {
    typed_leaf(obj, path, "integer", ctx, warnings, Value::as_i64)
}

fn leaf_u32(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<u32> {
    typed_leaf(obj, path, "non-negative integer", ctx, warnings, |v| {
        v.as_u64().and_then(|n| u32::try_from(n).ok())
    })
}

fn leaf_bool(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<bool> {
    typed_leaf(obj, path, "bool", ctx, warnings, Value::as_bool)
}

fn leaf_string(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<String> {
    typed_leaf(obj, path, "string", ctx, warnings, |v| {
        v.as_str().map(str::to_string)
    })
}

fn leaf_id_string(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<String> {
    typed_leaf(
        obj,
        path,
        "string or integer id",
        ctx,
        warnings,
        value_to_id_string,
    )
}

fn value_to_id_string(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        Some(s.to_string())
    } else {
        v.as_i64().map(|id| id.to_string())
    }
}

fn combine_status(a: ViewStatus, b: ViewStatus) -> ViewStatus {
    match (a, b) {
        (ViewStatus::Present, _) | (_, ViewStatus::Present) => ViewStatus::Present,
        (ViewStatus::DomainUnavailable, _) | (_, ViewStatus::DomainUnavailable) => {
            ViewStatus::DomainUnavailable
        }
        _ => ViewStatus::DomainAbsent,
    }
}

fn object_at_path<'a>(
    rest: &'a Map<String, Value>,
    path: &[&str],
    label: &str,
    warnings: &mut Vec<String>,
) -> (Option<&'a Map<String, Value>>, ViewStatus) {
    match walk_path(rest, path, "snapshot", warnings) {
        Leaf::Absent => (None, ViewStatus::DomainAbsent),
        Leaf::Unobtained => (None, ViewStatus::DomainUnavailable),
        Leaf::Measured(Value::Object(obj)) => (Some(obj), ViewStatus::Present),
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{label}: present but wrong-typed (expected object, found {}); \
                 view is unavailable",
                json_type_name(other)
            ));
            (None, ViewStatus::DomainUnavailable)
        }
    }
}

fn array_at_path<'a>(
    rest: &'a Map<String, Value>,
    path: &[&str],
    label: &str,
    warnings: &mut Vec<String>,
) -> (Option<&'a Vec<Value>>, ViewStatus) {
    match walk_path(rest, path, "snapshot", warnings) {
        Leaf::Absent => (None, ViewStatus::DomainAbsent),
        Leaf::Unobtained => (None, ViewStatus::DomainUnavailable),
        Leaf::Measured(Value::Array(items)) => (Some(items), ViewStatus::Present),
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{label}: present but wrong-typed (expected array, found {}); \
                 view is unavailable",
                json_type_name(other)
            ));
            (None, ViewStatus::DomainUnavailable)
        }
    }
}

fn leaf_string_list(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<String>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match value_to_id_string(item) {
                    Some(s) => out.push(s),
                    None => {
                        warnings.push(format!(
                            "{ctx}.{}[{i}]: expected string/integer id, found {}; \
                             whole list treated as unobtained",
                            path.join("."),
                            json_type_name(item)
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(out)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn leaf_bool_alias(
    obj: &Map<String, Value>,
    keys: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<bool> {
    let mut saw_unobtained = false;
    for key in keys {
        match leaf_bool(obj, &[*key], ctx, warnings) {
            Leaf::Measured(v) => return Leaf::Measured(v),
            Leaf::Unobtained => saw_unobtained = true,
            Leaf::Absent => {}
        }
    }
    if saw_unobtained {
        Leaf::Unobtained
    } else {
        Leaf::Absent
    }
}

fn first_available_f64(leaves: impl IntoIterator<Item = Leaf<f64>>) -> Leaf<f64> {
    let mut saw_unobtained = false;
    for leaf in leaves {
        match leaf {
            Leaf::Measured(v) => return Leaf::Measured(v),
            Leaf::Unobtained => saw_unobtained = true,
            Leaf::Absent => {}
        }
    }
    if saw_unobtained {
        Leaf::Unobtained
    } else {
        Leaf::Absent
    }
}

fn first_available_i64(leaves: impl IntoIterator<Item = Leaf<i64>>) -> Leaf<i64> {
    let mut saw_unobtained = false;
    for leaf in leaves {
        match leaf {
            Leaf::Measured(v) => return Leaf::Measured(v),
            Leaf::Unobtained => saw_unobtained = true,
            Leaf::Absent => {}
        }
    }
    if saw_unobtained {
        Leaf::Unobtained
    } else {
        Leaf::Absent
    }
}

/// A `{id-string: TAG|null}` layer -> typed map. Used for the `map.*`
/// mirrors and for the fallback domain walk.
fn tag_map_from_object(
    layer: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> BTreeMap<u32, Leaf<String>> {
    let mut out = BTreeMap::new();
    for (key, v) in layer {
        let id: u32 = match key.parse() {
            Ok(id) => id,
            Err(_) => {
                warnings.push(format!("{ctx}: non-numeric id key {key:?} skipped"));
                continue;
            }
        };
        let leaf = match v {
            Value::Null => Leaf::Unobtained,
            Value::String(tag) => Leaf::Measured(tag.clone()),
            other => {
                warnings.push(format!(
                    "{ctx}.{key}: present but wrong-typed (expected string TAG, found {}); \
                     treated as unobtained",
                    json_type_name(other)
                ));
                Leaf::Unobtained
            }
        };
        out.insert(id, leaf);
    }
    out
}

/// Map-control view: prefer the `map.<mirror_key>` mirror; when it is absent
/// or null (or wrong-typed, with a warning), fall back to walking
/// `<domain>.*.<field>`. Returns the view and which source produced it.
fn build_tag_map(
    rest: &Map<String, Value>,
    mirror_key: &str,
    domain: &str,
    field: &str,
    warnings: &mut Vec<String>,
) -> (BTreeMap<u32, Leaf<String>>, MapLayerSource, ViewStatus) {
    let mirror = match rest.get("map") {
        Some(Value::Object(map)) => map.get(mirror_key),
        Some(Value::Null) | None => None,
        Some(other) => {
            warnings.push(format!(
                "map: present but wrong-typed (expected object, found {}); \
                 falling back to walking {domain}.*.{field}",
                json_type_name(other)
            ));
            None
        }
    };
    match mirror {
        Some(Value::Object(layer)) => {
            let ctx = format!("map.{mirror_key}");
            return (
                tag_map_from_object(layer, &ctx, warnings),
                MapLayerSource::MapMirror,
                ViewStatus::Present,
            );
        }
        Some(Value::Null) => {
            if !matches!(rest.get(domain), Some(Value::Object(_))) {
                return (
                    BTreeMap::new(),
                    MapLayerSource::Missing,
                    ViewStatus::DomainUnavailable,
                );
            }
        }
        None => {}
        Some(other) => {
            warnings.push(format!(
                "map.{mirror_key}: present but wrong-typed (expected object, found {}); \
                 falling back to walking {domain}.*.{field}",
                json_type_name(other)
            ));
            if !matches!(rest.get(domain), Some(Value::Object(_))) {
                return (
                    BTreeMap::new(),
                    MapLayerSource::Missing,
                    ViewStatus::DomainUnavailable,
                );
            }
        }
    }
    // Fallback: walk the full domain objects.
    let entries = match rest.get(domain) {
        Some(Value::Object(entries)) => entries,
        Some(Value::Null) => {
            return (
                BTreeMap::new(),
                MapLayerSource::Missing,
                ViewStatus::DomainUnavailable,
            )
        }
        None => {
            return (
                BTreeMap::new(),
                MapLayerSource::Missing,
                ViewStatus::DomainAbsent,
            )
        }
        Some(other) => {
            warnings.push(format!(
                "{domain}: present but wrong-typed (expected object, found {}); \
                 map view is empty",
                json_type_name(other)
            ));
            return (
                BTreeMap::new(),
                MapLayerSource::Missing,
                ViewStatus::DomainUnavailable,
            );
        }
    };
    let mut out = BTreeMap::new();
    for (key, entry) in entries {
        let id: u32 = match key.parse() {
            Ok(id) => id,
            Err(_) => {
                warnings.push(format!("{domain}: non-numeric id key {key:?} skipped"));
                continue;
            }
        };
        let leaf = match entry {
            Value::Null => Leaf::Unobtained,
            Value::Object(obj) => {
                let ctx = format!("{domain}.{key}");
                leaf_string(obj, &[field], &ctx, warnings)
            }
            other => {
                warnings.push(format!(
                    "{domain}.{key}: expected object, found {}; treated as unobtained",
                    json_type_name(other)
                ));
                Leaf::Unobtained
            }
        };
        out.insert(id, leaf);
    }
    (out, MapLayerSource::DomainWalk, ViewStatus::Present)
}

fn build_division_units(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (Vec<DivisionUnitRow>, ViewStatus) {
    let (items, status) = array_at_path(rest, &["divisions"], "divisions", warnings);
    let Some(items) = items else {
        return (Vec::new(), status);
    };
    let mut rows = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let obj = match item.as_object() {
            Some(obj) => obj,
            None => {
                warnings.push(format!(
                    "divisions[{i}]: expected object, found {}; row skipped",
                    json_type_name(item)
                ));
                continue;
            }
        };
        let div_id = match obj.get("div_id").and_then(Value::as_i64) {
            Some(id) => id,
            None => {
                warnings.push(format!(
                    "divisions[{i}]: missing or non-integer div_id; row skipped"
                ));
                continue;
            }
        };
        let div_uid = division_uid_from_obj(obj, div_id, i, warnings);
        let ctx = format!("divisions[{i}]");
        rows.push(DivisionUnitRow {
            div_uid,
            owner: leaf_string(obj, &["owner"], &ctx, warnings),
            location_province: leaf_u32(obj, &["location_province"], &ctx, warnings),
            state_id: leaf_u32(obj, &["state_id"], &ctx, warnings),
            template_id: leaf_i64(obj, &["template_id"], &ctx, warnings),
            template_type: leaf_i64(obj, &["template_type"], &ctx, warnings),
            template_name: leaf_string(obj, &["template_name"], &ctx, warnings),
            template_role: leaf_string(obj, &["template_role"], &ctx, warnings),
            strength: leaf_f64(obj, &["strength"], &ctx, warnings),
            organisation: leaf_f64(obj, &["organisation"], &ctx, warnings),
            moving: leaf_bool(obj, &["moving"], &ctx, warnings),
            move_target_province: leaf_u32(obj, &["move_target_province"], &ctx, warnings),
            move_progress: leaf_f64(obj, &["move_progress"], &ctx, warnings),
            in_combat: leaf_bool(obj, &["in_combat"], &ctx, warnings),
            equipment: leaf_equipment_list(obj, &["equipment"], &ctx, warnings),
        });
    }
    (rows, ViewStatus::Present)
}

fn division_uid_from_obj(
    obj: &Map<String, Value>,
    div_id: i64,
    _index: usize,
    _warnings: &mut Vec<String>,
) -> String {
    match obj.get("div_type") {
        None => format!("{div_id}"),
        Some(Value::Null) => format!("{div_id}"),
        Some(v) => match v.as_i64() {
            Some(t) => format!("{div_id}:{t}"),
            None => format!("{div_id}"),
        },
    }
}

fn build_fleets(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (Vec<FleetUnitRow>, ViewStatus) {
    let (fleets, status) = array_at_path(
        rest,
        &["order_of_battle", "fleets"],
        "order_of_battle.fleets",
        warnings,
    );
    let Some(fleets) = fleets else {
        return (Vec::new(), status);
    };
    let mut rows = Vec::new();
    for (fleet_i, fleet) in fleets.iter().enumerate() {
        let fleet_obj = match fleet.as_object() {
            Some(obj) => obj,
            None => {
                warnings.push(format!(
                    "order_of_battle.fleets[{fleet_i}]: expected object, found {}; row skipped",
                    json_type_name(fleet)
                ));
                continue;
            }
        };
        let fleet_ctx = format!("order_of_battle.fleets[{fleet_i}]");
        let fleet_uid = leaf_id_string(fleet_obj, &["id"], &fleet_ctx, warnings);
        let task_forces = match fleet_obj.get("task_forces") {
            Some(Value::Array(items)) => items,
            Some(Value::Null) | None => continue,
            Some(other) => {
                warnings.push(format!(
                    "{fleet_ctx}.task_forces: present but wrong-typed (expected array, found {}); skipped",
                    json_type_name(other)
                ));
                continue;
            }
        };
        for (tf_i, task_force) in task_forces.iter().enumerate() {
            let obj = match task_force.as_object() {
                Some(obj) => obj,
                None => {
                    warnings.push(format!(
                        "{fleet_ctx}.task_forces[{tf_i}]: expected object, found {}; row skipped",
                        json_type_name(task_force)
                    ));
                    continue;
                }
            };
            let ctx = format!("{fleet_ctx}.task_forces[{tf_i}]");
            let task_force_uid = match obj.get("id").and_then(value_to_id_string) {
                Some(id) => id,
                None => {
                    warnings.push(format!("{ctx}: missing or wrong-typed id; row skipped"));
                    continue;
                }
            };
            let ships = leaf_ships(obj, &ctx, warnings);
            let ship_count = match leaf_i64(obj, &["ship_count"], &ctx, warnings) {
                Leaf::Absent => match &ships {
                    Leaf::Measured(rows) => Leaf::Measured(rows.len() as i64),
                    Leaf::Unobtained => Leaf::Unobtained,
                    Leaf::Absent => Leaf::Absent,
                },
                other => other,
            };
            rows.push(FleetUnitRow {
                task_force_uid,
                fleet_uid: fleet_uid.clone(),
                owner: first_available_string([
                    leaf_string(obj, &["owner"], &ctx, warnings),
                    leaf_string(fleet_obj, &["owner"], &fleet_ctx, warnings),
                ]),
                location_province: leaf_u32(obj, &["location_province"], &ctx, warnings),
                mission: leaf_string(obj, &["mission"], &ctx, warnings),
                ship_count,
                dominant_hull_class: first_available_string([
                    leaf_string(obj, &["dominant_hull_class"], &ctx, warnings),
                    leaf_string(obj, &["dominant_hull"], &ctx, warnings),
                ]),
                ships,
            });
        }
    }
    (rows, ViewStatus::Present)
}

fn build_wings(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (Vec<WingUnitRow>, ViewStatus) {
    let (items, status) = array_at_path(
        rest,
        &["order_of_battle", "air_wings"],
        "order_of_battle.air_wings",
        warnings,
    );
    let Some(items) = items else {
        return (Vec::new(), status);
    };
    let mut rows = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let obj = match item.as_object() {
            Some(obj) => obj,
            None => {
                warnings.push(format!(
                    "order_of_battle.air_wings[{i}]: expected object, found {}; row skipped",
                    json_type_name(item)
                ));
                continue;
            }
        };
        let ctx = format!("order_of_battle.air_wings[{i}]");
        let wing_uid = match obj.get("id").and_then(value_to_id_string) {
            Some(id) => id,
            None => {
                warnings.push(format!("{ctx}: missing or wrong-typed id; row skipped"));
                continue;
            }
        };
        rows.push(WingUnitRow {
            wing_uid,
            owner: leaf_string(obj, &["owner"], &ctx, warnings),
            name: leaf_string(obj, &["name"], &ctx, warnings),
            plane_count: first_available_i64([
                leaf_i64(obj, &["count"], &ctx, warnings),
                leaf_i64(obj, &["plane_count"], &ctx, warnings),
            ]),
            experience: leaf_f64(obj, &["experience"], &ctx, warnings),
            manpower: leaf_i64(obj, &["manpower"], &ctx, warnings),
            mission_kind: first_available_string([
                leaf_string(obj, &["mission"], &ctx, warnings),
                leaf_string(obj, &["mission_type"], &ctx, warnings),
            ]),
            strategic_region_id: leaf_u32(obj, &["strategic_region"], &ctx, warnings),
            air_base: leaf_id_string(obj, &["air_base"], &ctx, warnings),
            state_id: leaf_u32(obj, &["state_id"], &ctx, warnings),
            plane_definition: leaf_string(obj, &["plane_definition"], &ctx, warnings),
            effective_planes: leaf_i64(obj, &["effective_planes"], &ctx, warnings),
            equipment: leaf_equipment_list(obj, &["equipment"], &ctx, warnings),
        });
    }
    (rows, ViewStatus::Present)
}

fn first_available_string(leaves: impl IntoIterator<Item = Leaf<String>>) -> Leaf<String> {
    let mut saw_unobtained = false;
    for leaf in leaves {
        match leaf {
            Leaf::Measured(v) => return Leaf::Measured(v),
            Leaf::Unobtained => saw_unobtained = true,
            Leaf::Absent => {}
        }
    }
    if saw_unobtained {
        Leaf::Unobtained
    } else {
        Leaf::Absent
    }
}

fn leaf_equipment_list(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<EquipmentAmount>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut rows = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::Object(equip) => {
                        let ectx = format!("{ctx}.{}[{i}]", path.join("."));
                        rows.push(EquipmentAmount {
                            archetype: leaf_string(equip, &["archetype"], &ectx, warnings),
                            equip_id: first_available_i64([
                                leaf_i64(equip, &["equip_id"], &ectx, warnings),
                                leaf_i64(equip, &["id"], &ectx, warnings),
                            ]),
                            equip_type: first_available_i64([
                                leaf_i64(equip, &["equip_type"], &ectx, warnings),
                                leaf_i64(equip, &["type"], &ectx, warnings),
                            ]),
                            amount: leaf_f64(equip, &["amount"], &ectx, warnings),
                        });
                    }
                    Value::Array(parts) if parts.len() >= 3 => {
                        rows.push(EquipmentAmount {
                            archetype: Leaf::Absent,
                            equip_id: parts[0]
                                .as_i64()
                                .map(Leaf::Measured)
                                .unwrap_or(Leaf::Unobtained),
                            equip_type: parts[1]
                                .as_i64()
                                .map(Leaf::Measured)
                                .unwrap_or(Leaf::Unobtained),
                            amount: parts[2]
                                .as_f64()
                                .map(Leaf::Measured)
                                .unwrap_or(Leaf::Unobtained),
                        });
                    }
                    other => {
                        warnings.push(format!(
                            "{ctx}.{}[{i}]: expected object or [id,type,amount], found {}; \
                             whole list treated as unobtained",
                            path.join("."),
                            json_type_name(other)
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(rows)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn leaf_ships(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<ShipCompositionRow>> {
    match walk_path(obj, &["ships"], ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut rows = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let ship = match item.as_object() {
                    Some(ship) => ship,
                    None => {
                        warnings.push(format!(
                            "{ctx}.ships[{i}]: expected object, found {}; whole list treated as unobtained",
                            json_type_name(item)
                        ));
                        return Leaf::Unobtained;
                    }
                };
                let sctx = format!("{ctx}.ships[{i}]");
                rows.push(ShipCompositionRow {
                    ship_uid: leaf_id_string(ship, &["id"], &sctx, warnings),
                    definition: leaf_string(ship, &["definition"], &sctx, warnings),
                    organisation: leaf_f64(ship, &["organisation"], &sctx, warnings),
                    strength: leaf_f64(ship, &["strength"], &sctx, warnings),
                    experience: leaf_f64(ship, &["experience"], &sctx, warnings),
                    name: leaf_string(ship, &["name"], &sctx, warnings),
                    equipment: leaf_equipment_list(ship, &["equipment"], &sctx, warnings),
                });
            }
            Leaf::Measured(rows)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.ships: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn build_divisions(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> Vec<DivisionRow> {
    let items = match rest.get("divisions") {
        Some(Value::Array(items)) => items,
        Some(Value::Null) | None => return Vec::new(),
        Some(other) => {
            warnings.push(format!(
                "divisions: present but wrong-typed (expected array, found {}); \
                 division view is empty",
                json_type_name(other)
            ));
            return Vec::new();
        }
    };
    let mut rows = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let obj = match item.as_object() {
            Some(obj) => obj,
            None => {
                warnings.push(format!(
                    "divisions[{i}]: expected object, found {}; row skipped",
                    json_type_name(item)
                ));
                continue;
            }
        };
        // Identity is required (schema §6): a row without an integer div_id
        // cannot be keyed and is skipped with a warning, never invented.
        let div_id = match obj.get("div_id").and_then(Value::as_i64) {
            Some(id) => id,
            None => {
                warnings.push(format!(
                    "divisions[{i}]: missing or non-integer div_id; row skipped"
                ));
                continue;
            }
        };
        let div_uid = match obj.get("div_type") {
            // absent div_type is the schema-sanctioned bare-id identity
            None => format!("{div_id}"),
            // null is captured-but-unobtained, not schema-absent: identity
            // still degrades to the bare id (§6 untyped-channel matching),
            // but the degradation is surfaced, never silent
            Some(Value::Null) => {
                warnings.push(format!(
                    "divisions[{i}].div_type is null (unobtained); falling back to \
                     bare div_id identity"
                ));
                format!("{div_id}")
            }
            Some(v) => match v.as_i64() {
                Some(t) => format!("{div_id}:{t}"),
                None => {
                    warnings.push(format!(
                        "divisions[{i}].div_type: present but wrong-typed (expected \
                         integer, found {}); falling back to bare div_id identity",
                        json_type_name(v)
                    ));
                    format!("{div_id}")
                }
            },
        };
        let ctx = format!("divisions[{i}]");
        rows.push(DivisionRow {
            div_uid,
            owner: leaf_string(obj, &["owner"], &ctx, warnings),
            location_province: leaf_u32(obj, &["location_province"], &ctx, warnings),
            strength: leaf_f64(obj, &["strength"], &ctx, warnings),
            organisation: leaf_f64(obj, &["organisation"], &ctx, warnings),
            in_combat: leaf_bool(obj, &["in_combat"], &ctx, warnings),
        });
    }
    rows
}

fn leaf_id_list(
    obj: &Map<String, Value>,
    key: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<i64>> {
    match obj.get(key) {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::Array(items)) => {
            let mut ids = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item.as_i64() {
                    Some(id) => ids.push(id),
                    None => {
                        // A partial participant list would misrepresent the
                        // engagement as Measured; one malformed element
                        // poisons the whole leaf (fail-closed, with warning).
                        warnings.push(format!(
                            "{ctx}.{key}[{i}]: expected integer division id, found {}; \
                             whole list treated as unobtained",
                            json_type_name(item)
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(ids)
        }
        Some(other) => {
            warnings.push(format!(
                "{ctx}.{key}: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn build_combats(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> Vec<CombatRow> {
    let items = match rest.get("combats") {
        Some(Value::Array(items)) => items,
        Some(Value::Null) | None => return Vec::new(),
        Some(other) => {
            warnings.push(format!(
                "combats: present but wrong-typed (expected array, found {}); \
                 combat view is empty",
                json_type_name(other)
            ));
            return Vec::new();
        }
    };
    let mut rows = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let obj = match item.as_object() {
            Some(obj) => obj,
            None => {
                warnings.push(format!(
                    "combats[{i}]: expected object, found {}; row skipped",
                    json_type_name(item)
                ));
                continue;
            }
        };
        let ctx = format!("combats[{i}]");
        rows.push(CombatRow {
            combat_id: leaf_i64(obj, &["combat_id"], &ctx, warnings),
            kind: leaf_string(obj, &["kind"], &ctx, warnings),
            location_province: leaf_u32(obj, &["location_province"], &ctx, warnings),
            attackers: leaf_id_list(obj, "attackers", &ctx, warnings),
            defenders: leaf_id_list(obj, "defenders", &ctx, warnings),
        });
    }
    rows
}

fn build_session_metadata(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> SessionMetadataView {
    let (session, session_status) = object_at_path(rest, &["session"], "session", warnings);
    let (scenario, scenario_status) =
        object_at_path(rest, &["world", "scenario"], "world.scenario", warnings);
    let status = combine_status(session_status, scenario_status);
    // Players live under `world.scenario.players` in the schema/fixtures; the
    // internal `session.player_countries` path is honored first when a writer
    // provides it, then we fall through to the scenario location. Genuine
    // absence (neither location carries a list) stays Absent.
    let player_countries = {
        let from_session = match session {
            Some(obj) => leaf_player_countries(obj, "player_countries", "session", warnings),
            None => Leaf::Absent,
        };
        match from_session {
            Leaf::Absent => match scenario {
                Some(obj) => leaf_player_countries(obj, "players", "world.scenario", warnings),
                None if scenario_status == ViewStatus::DomainUnavailable => Leaf::Unobtained,
                None => match status {
                    ViewStatus::DomainUnavailable => Leaf::Unobtained,
                    _ => Leaf::Absent,
                },
            },
            other => other,
        }
    };
    let speed_events = match session {
        Some(obj) => leaf_speed_events(obj, warnings),
        None => match session_status {
            ViewStatus::DomainUnavailable => Leaf::Unobtained,
            _ => Leaf::Absent,
        },
    };
    let active_dlcs = match session {
        Some(obj) => first_available_string_list([
            leaf_string_list(obj, &["active_dlcs"], "session", warnings),
            leaf_string_list(obj, &["dlcs"], "session", warnings),
        ]),
        None => Leaf::Absent,
    };
    let active_dlcs = match active_dlcs {
        Leaf::Absent => match scenario {
            Some(obj) => leaf_string_list(obj, &["dlcs"], "world.scenario", warnings),
            None if scenario_status == ViewStatus::DomainUnavailable => Leaf::Unobtained,
            None => Leaf::Absent,
        },
        other => other,
    };
    let localisation_language = match session {
        Some(obj) => first_available_string([
            leaf_string(obj, &["localisation_language"], "session", warnings),
            leaf_string(obj, &["language"], "session", warnings),
        ]),
        None => Leaf::Absent,
    };
    let localisation_language = match localisation_language {
        Leaf::Absent => match scenario {
            Some(obj) => first_available_string([
                leaf_string(obj, &["localisation_language"], "world.scenario", warnings),
                leaf_string(obj, &["language"], "world.scenario", warnings),
            ]),
            None if scenario_status == ViewStatus::DomainUnavailable => Leaf::Unobtained,
            None => Leaf::Absent,
        },
        other => other,
    };
    SessionMetadataView {
        status,
        player_countries,
        speed_events,
        active_dlcs,
        localisation_language,
    }
}

fn first_available_string_list(
    leaves: impl IntoIterator<Item = Leaf<Vec<String>>>,
) -> Leaf<Vec<String>> {
    let mut saw_unobtained = false;
    for leaf in leaves {
        match leaf {
            Leaf::Measured(v) => return Leaf::Measured(v),
            Leaf::Unobtained => saw_unobtained = true,
            Leaf::Absent => {}
        }
    }
    if saw_unobtained {
        Leaf::Unobtained
    } else {
        Leaf::Absent
    }
}

/// Parse a player-country list from `obj[key]`. Shared by the internal
/// `session.player_countries` path and the schema-shaped
/// `world.scenario.players` path (`base` names the enclosing object for
/// warnings, `key` the list field). Trichotomy is preserved: a missing key is
/// [`Leaf::Absent`] so the caller can fall through to the other location.
fn leaf_player_countries(
    obj: &Map<String, Value>,
    key: &str,
    base: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<PlayerCountry>> {
    match walk_path(obj, &[key], base, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut players = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::String(tag) => players.push(PlayerCountry {
                        tag: Leaf::Measured(tag.clone()),
                        player_name: Leaf::Absent,
                    }),
                    Value::Object(entry) => {
                        let ctx = format!("{base}.{key}[{i}]");
                        players.push(PlayerCountry {
                            tag: leaf_string(entry, &["tag"], &ctx, warnings),
                            player_name: first_available_string([
                                leaf_string(entry, &["player"], &ctx, warnings),
                                leaf_string(entry, &["player_name"], &ctx, warnings),
                                leaf_string(entry, &["human_name"], &ctx, warnings),
                                leaf_string(entry, &["label"], &ctx, warnings),
                            ]),
                        });
                    }
                    other => {
                        warnings.push(format!(
                            "{base}.{key}[{i}]: expected string or object, found {}; \
                             whole list treated as unobtained",
                            json_type_name(other)
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(players)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{base}.{key}: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn leaf_speed_events(
    session: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<SpeedEvent>> {
    match walk_path(session, &["speed_events"], "session", warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut events = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let obj = match item.as_object() {
                    Some(obj) => obj,
                    None => {
                        warnings.push(format!(
                            "session.speed_events[{i}]: expected object, found {}; whole list treated as unobtained",
                            json_type_name(item)
                        ));
                        return Leaf::Unobtained;
                    }
                };
                let ctx = format!("session.speed_events[{i}]");
                events.push(SpeedEvent {
                    hour_index: leaf_i64(obj, &["hour_index"], &ctx, warnings),
                    speed: leaf_i64(obj, &["speed"], &ctx, warnings),
                    paused: leaf_bool(obj, &["paused"], &ctx, warnings),
                });
            }
            Leaf::Measured(events)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "session.speed_events: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn build_naval_supremacy(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<u32, NavalSupremacyRow>, ViewStatus) {
    if let (Some(obj), status) =
        object_at_path(rest, &["naval_supremacy"], "naval_supremacy", warnings)
    {
        return (
            parse_naval_supremacy_map(obj, "naval_supremacy", warnings),
            status,
        );
    } else {
        let (_, status) = object_at_path(rest, &["naval_supremacy"], "naval_supremacy", warnings);
        if status == ViewStatus::DomainUnavailable {
            return (BTreeMap::new(), status);
        }
    }

    let (region_control, status) = object_at_path(
        rest,
        &["naval_air", "region_control"],
        "naval_air.region_control",
        warnings,
    );
    let Some(region_control) = region_control else {
        return (BTreeMap::new(), status);
    };
    let mut out = BTreeMap::new();
    for (key, value) in region_control {
        let id = match key.parse::<u32>() {
            Ok(id) => id,
            Err(_) => {
                warnings.push(format!(
                    "naval_air.region_control: non-numeric region id {key:?} skipped"
                ));
                continue;
            }
        };
        let row_obj = match value {
            Value::Array(items) => items.iter().find_map(Value::as_object),
            Value::Object(obj) => Some(obj),
            Value::Null => {
                out.insert(
                    id,
                    NavalSupremacyRow {
                        controller: Leaf::Unobtained,
                        state: Leaf::Unobtained,
                        supremacy: Leaf::Unobtained,
                    },
                );
                continue;
            }
            other => {
                warnings.push(format!(
                    "naval_air.region_control.{key}: expected object/array, found {}; treated as unobtained",
                    json_type_name(other)
                ));
                out.insert(
                    id,
                    NavalSupremacyRow {
                        controller: Leaf::Unobtained,
                        state: Leaf::Unobtained,
                        supremacy: Leaf::Unobtained,
                    },
                );
                continue;
            }
        };
        if let Some(obj) = row_obj {
            let ctx = format!("naval_air.region_control.{key}");
            out.insert(
                id,
                NavalSupremacyRow {
                    controller: leaf_string(obj, &["controller"], &ctx, warnings),
                    state: first_available_string([
                        leaf_string(obj, &["state"], &ctx, warnings),
                        leaf_string(obj, &["status"], &ctx, warnings),
                    ]),
                    supremacy: first_available_f64([
                        leaf_f64(obj, &["supremacy"], &ctx, warnings),
                        leaf_f64(obj, &["value"], &ctx, warnings),
                    ]),
                },
            );
        }
    }
    (out, ViewStatus::Present)
}

fn parse_naval_supremacy_map(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> BTreeMap<u32, NavalSupremacyRow> {
    let mut out = BTreeMap::new();
    for (key, value) in obj {
        let id = match key.parse::<u32>() {
            Ok(id) => id,
            Err(_) => {
                warnings.push(format!("{ctx}: non-numeric region id {key:?} skipped"));
                continue;
            }
        };
        match value {
            Value::Object(row) => {
                let rctx = format!("{ctx}.{key}");
                out.insert(
                    id,
                    NavalSupremacyRow {
                        controller: leaf_string(row, &["controller"], &rctx, warnings),
                        state: first_available_string([
                            leaf_string(row, &["state"], &rctx, warnings),
                            leaf_string(row, &["status"], &rctx, warnings),
                        ]),
                        supremacy: first_available_f64([
                            leaf_f64(row, &["supremacy"], &rctx, warnings),
                            leaf_f64(row, &["value"], &rctx, warnings),
                        ]),
                    },
                );
            }
            Value::Null => {
                out.insert(
                    id,
                    NavalSupremacyRow {
                        controller: Leaf::Unobtained,
                        state: Leaf::Unobtained,
                        supremacy: Leaf::Unobtained,
                    },
                );
            }
            other => {
                warnings.push(format!(
                    "{ctx}.{key}: expected object, found {}; treated as unobtained",
                    json_type_name(other)
                ));
                out.insert(
                    id,
                    NavalSupremacyRow {
                        controller: Leaf::Unobtained,
                        state: Leaf::Unobtained,
                        supremacy: Leaf::Unobtained,
                    },
                );
            }
        }
    }
    out
}

/// Air-superiority view (requirements item 5): `naval_air.air_superiority` is
/// a `{region_id: {controller, mission_mask, wing_count, plane_count}}` map.
/// A present-but-null domain is `DomainUnavailable`; a null/wrong-typed row is
/// captured-but-unobtained (never coerced).
fn build_air_superiority(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<u32, AirSuperiorityRow>, ViewStatus) {
    let (region_map, status) = object_at_path(
        rest,
        &["naval_air", "air_superiority"],
        "naval_air.air_superiority",
        warnings,
    );
    let Some(region_map) = region_map else {
        return (BTreeMap::new(), status);
    };
    let mut out = BTreeMap::new();
    for (key, value) in region_map {
        let id = match key.parse::<u32>() {
            Ok(id) => id,
            Err(_) => {
                warnings.push(format!(
                    "naval_air.air_superiority: non-numeric region id {key:?} skipped"
                ));
                continue;
            }
        };
        match value {
            Value::Object(row) => {
                let ctx = format!("naval_air.air_superiority.{key}");
                out.insert(
                    id,
                    AirSuperiorityRow {
                        controller: leaf_string(row, &["controller"], &ctx, warnings),
                        mission_mask: leaf_i64(row, &["mission_mask"], &ctx, warnings),
                        wing_count: leaf_i64(row, &["wing_count"], &ctx, warnings),
                        plane_count: leaf_i64(row, &["plane_count"], &ctx, warnings),
                    },
                );
            }
            Value::Null => {
                out.insert(
                    id,
                    AirSuperiorityRow {
                        controller: Leaf::Unobtained,
                        mission_mask: Leaf::Unobtained,
                        wing_count: Leaf::Unobtained,
                        plane_count: Leaf::Unobtained,
                    },
                );
            }
            other => {
                warnings.push(format!(
                    "naval_air.air_superiority.{key}: expected object, found {}; treated as unobtained",
                    json_type_name(other)
                ));
                out.insert(
                    id,
                    AirSuperiorityRow {
                        controller: Leaf::Unobtained,
                        mission_mask: Leaf::Unobtained,
                        wing_count: Leaf::Unobtained,
                        plane_count: Leaf::Unobtained,
                    },
                );
            }
        }
    }
    (out, ViewStatus::Present)
}

fn build_weather(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<u32, WeatherRegionRow>, ViewStatus) {
    let (weather, status) = object_at_path(rest, &["weather"], "weather", warnings);
    let Some(weather) = weather else {
        return (BTreeMap::new(), status);
    };
    let region_map = match weather.get("by_region").or_else(|| weather.get("regions")) {
        Some(Value::Object(obj)) => obj,
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        Some(other) => {
            warnings.push(format!(
                "weather.by_region: present but wrong-typed (expected object, found {}); view unavailable",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
        None => weather,
    };
    let mut out = BTreeMap::new();
    for (key, value) in region_map {
        let id = match key.parse::<u32>() {
            Ok(id) => id,
            Err(_) => continue,
        };
        match value {
            Value::Object(row) => {
                let ctx = format!("weather.by_region.{key}");
                out.insert(
                    id,
                    WeatherRegionRow {
                        weather_kind: first_available_string([
                            leaf_string(row, &["weather_kind"], &ctx, warnings),
                            leaf_string(row, &["kind"], &ctx, warnings),
                            leaf_string(row, &["phenomenon"], &ctx, warnings),
                            // A3 round-trip: the strategic-region weather rows
                            // (requirements item 6) name the phenomenon under
                            // `phenom_name`; honor it as a weather_kind alias.
                            leaf_string(row, &["phenom_name"], &ctx, warnings),
                        ]),
                        intensity: leaf_f64(row, &["intensity"], &ctx, warnings),
                        snow_level_0_255: leaf_u32(row, &["snow_level_0_255"], &ctx, warnings),
                        mud_level_0_255: leaf_u32(row, &["mud_level_0_255"], &ctx, warnings),
                        snow_mud_fade: leaf_f64(row, &["snow_mud_fade"], &ctx, warnings),
                        snow_mud_fade_from: leaf_f64(row, &["snow_mud_fade_from"], &ctx, warnings),
                        snow_mud_fade_to: leaf_f64(row, &["snow_mud_fade_to"], &ctx, warnings),
                    },
                );
            }
            Value::Null => {
                out.insert(
                    id,
                    WeatherRegionRow {
                        weather_kind: Leaf::Unobtained,
                        intensity: Leaf::Unobtained,
                        snow_level_0_255: Leaf::Unobtained,
                        mud_level_0_255: Leaf::Unobtained,
                        snow_mud_fade: Leaf::Unobtained,
                        snow_mud_fade_from: Leaf::Unobtained,
                        snow_mud_fade_to: Leaf::Unobtained,
                    },
                );
            }
            other => {
                warnings.push(format!(
                    "weather.by_region.{key}: expected object, found {}; treated as unobtained",
                    json_type_name(other)
                ));
                out.insert(
                    id,
                    WeatherRegionRow {
                        weather_kind: Leaf::Unobtained,
                        intensity: Leaf::Unobtained,
                        snow_level_0_255: Leaf::Unobtained,
                        mud_level_0_255: Leaf::Unobtained,
                        snow_mud_fade: Leaf::Unobtained,
                        snow_mud_fade_from: Leaf::Unobtained,
                        snow_mud_fade_to: Leaf::Unobtained,
                    },
                );
            }
        }
    }
    (out, ViewStatus::Present)
}

/// Accumulated snow/mud ground-cover view (requirements item 6). The engine's
/// SnowMudData texture is province-addressed and lives under
/// `weather.snow_mud` (`texture` = per-province levels, `fade` = the blend
/// window), separate from the region-level phenomenon in `weather.regions`.
fn build_snow_mud(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> SnowMudView {
    let (weather, weather_status) = object_at_path(rest, &["weather"], "weather", warnings);
    let Some(weather) = weather else {
        return SnowMudView::with_status(weather_status);
    };
    let snow_mud = match weather.get("snow_mud") {
        Some(Value::Object(obj)) => obj,
        Some(Value::Null) => return SnowMudView::with_status(ViewStatus::DomainUnavailable),
        None => return SnowMudView::with_status(ViewStatus::DomainAbsent),
        Some(other) => {
            warnings.push(format!(
                "weather.snow_mud: present but wrong-typed (expected object, found {}); view unavailable",
                json_type_name(other)
            ));
            return SnowMudView::with_status(ViewStatus::DomainUnavailable);
        }
    };
    let mut by_province = BTreeMap::new();
    match snow_mud.get("texture") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                let row = match item.as_object() {
                    Some(row) => row,
                    None => {
                        warnings.push(format!(
                            "weather.snow_mud.texture[{i}]: expected object, found {}; cell skipped",
                            json_type_name(item)
                        ));
                        continue;
                    }
                };
                let ctx = format!("weather.snow_mud.texture[{i}]");
                let province_id = match row.get("province_id").and_then(Value::as_u64) {
                    Some(pid) if u32::try_from(pid).is_ok() => pid as u32,
                    _ => {
                        warnings.push(format!(
                            "{ctx}: missing or invalid province_id; cell skipped"
                        ));
                        continue;
                    }
                };
                by_province.insert(
                    province_id,
                    SnowMudCell {
                        region_id: leaf_u32(row, &["region_id"], &ctx, warnings),
                        snow_level_0_255: leaf_u32(row, &["snow_level_0_255"], &ctx, warnings),
                        mud_level_0_255: leaf_u32(row, &["mud_level_0_255"], &ctx, warnings),
                    },
                );
            }
        }
        Some(other) => warnings.push(format!(
            "weather.snow_mud.texture: present but wrong-typed (expected array, found {}); ignored",
            json_type_name(other)
        )),
    }
    let (fade_phase, fade_0_hour, fade_1_hour) = match snow_mud.get("fade") {
        Some(Value::Object(fade)) => {
            let ctx = "weather.snow_mud.fade";
            (
                leaf_f64(fade, &["phase"], ctx, warnings),
                leaf_i64(fade, &["fade_0_hour"], ctx, warnings),
                leaf_i64(fade, &["fade_1_hour"], ctx, warnings),
            )
        }
        Some(Value::Null) => (Leaf::Unobtained, Leaf::Unobtained, Leaf::Unobtained),
        None => (Leaf::Absent, Leaf::Absent, Leaf::Absent),
        Some(other) => {
            warnings.push(format!(
                "weather.snow_mud.fade: present but wrong-typed (expected object, found {}); treated as unobtained",
                json_type_name(other)
            ));
            (Leaf::Unobtained, Leaf::Unobtained, Leaf::Unobtained)
        }
    };
    SnowMudView {
        status: ViewStatus::Present,
        by_province,
        fade_phase,
        fade_0_hour,
        fade_1_hour,
    }
}

fn build_countries(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, CountryCore>, ViewStatus) {
    let entries = match rest.get("countries") {
        Some(Value::Object(entries)) => entries,
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        None => return (BTreeMap::new(), ViewStatus::DomainAbsent),
        Some(other) => {
            warnings.push(format!(
                "countries: present but wrong-typed (expected object, found {}); \
                 country view is empty",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
    };
    let mut out = BTreeMap::new();
    for (tag, entry) in entries {
        let obj = match entry {
            Value::Object(obj) => obj,
            // A present-but-null country entry means its leaves are
            // captured-but-unobtained; the tag must not vanish from the view
            // (that would be indistinguishable from an absent country).
            Value::Null => {
                out.insert(tag.clone(), CountryCore::all_unobtained());
                continue;
            }
            other => {
                warnings.push(format!(
                    "countries.{tag}: expected object, found {}; leaves treated as \
                     unobtained",
                    json_type_name(other)
                ));
                out.insert(tag.clone(), CountryCore::all_unobtained());
                continue;
            }
        };
        let ctx = format!("countries.{tag}");
        out.insert(
            tag.clone(),
            CountryCore {
                stability: leaf_f64(obj, &["politics", "stability"], &ctx, warnings),
                war_support: leaf_f64(obj, &["politics", "war_support"], &ctx, warnings),
                political_power: leaf_f64(obj, &["politics", "political_power"], &ctx, warnings),
                ruling_party: leaf_string(obj, &["politics", "ruling_party"], &ctx, warnings),
                fuel_current: leaf_f64(obj, &["economy", "fuel", "current"], &ctx, warnings),
                fuel_max: leaf_f64(obj, &["economy", "fuel", "max"], &ctx, warnings),
                division_count: leaf_i64(obj, &["military", "division_count"], &ctx, warnings),
            },
        );
    }
    (out, ViewStatus::Present)
}

fn build_country_identities(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, CountryIdentity>, ViewStatus) {
    let entries = match rest.get("countries") {
        Some(Value::Object(entries)) => entries,
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        None => return (BTreeMap::new(), ViewStatus::DomainAbsent),
        Some(other) => {
            warnings.push(format!(
                "countries: present but wrong-typed (expected object, found {}); \
                 country identity view is unavailable",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
    };
    let mut out = BTreeMap::new();
    for (tag, entry) in entries {
        match entry {
            Value::Object(obj) => {
                let ctx = format!("countries.{tag}");
                out.insert(
                    tag.clone(),
                    CountryIdentity {
                        // A3 round-trip: the cosmetic tag (requirements item 7b)
                        // is mirrored under both `extra` and `misc`; accept
                        // either so a country that carries only one still keys.
                        cosmetic_tag: first_available_string([
                            leaf_string(obj, &["extra", "cosmetic_tag"], &ctx, warnings),
                            leaf_string(obj, &["misc", "cosmetic_tag"], &ctx, warnings),
                        ]),
                        ruling_ideology: leaf_string(
                            obj,
                            &["politics", "ruling_party"],
                            &ctx,
                            warnings,
                        ),
                    },
                );
            }
            Value::Null => {
                out.insert(tag.clone(), CountryIdentity::all_unobtained());
            }
            other => {
                warnings.push(format!(
                    "countries.{tag}: expected object, found {}; identity leaves treated as unobtained",
                    json_type_name(other)
                ));
                out.insert(tag.clone(), CountryIdentity::all_unobtained());
            }
        }
    }
    (out, ViewStatus::Present)
}

fn build_diplomacy(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> DiplomacyView {
    let mut countries = BTreeMap::new();
    let mut pairs: BTreeMap<(String, String), DiplomaticPairState> = BTreeMap::new();
    let mut relation_flags: BTreeMap<String, Leaf<BTreeMap<String, Vec<String>>>> = BTreeMap::new();
    let mut status = ViewStatus::DomainAbsent;

    match rest.get("countries") {
        Some(Value::Object(entries)) => {
            status = ViewStatus::Present;
            for (tag, entry) in entries {
                match entry {
                    Value::Object(obj) => {
                        let ctx = format!("countries.{tag}");
                        countries.insert(
                            tag.clone(),
                            CountryDiplomacy {
                                faction_name: leaf_string(
                                    obj,
                                    &["politics", "faction", "name"],
                                    &ctx,
                                    warnings,
                                ),
                                faction_is_leader: leaf_bool(
                                    obj,
                                    &["politics", "faction", "is_leader"],
                                    &ctx,
                                    warnings,
                                ),
                                capital_state: leaf_u32(
                                    obj,
                                    &["politics", "capital_state"],
                                    &ctx,
                                    warnings,
                                ),
                            },
                        );
                        fold_country_diplomacy_pairs(tag, obj, &mut pairs, warnings);
                        if let Some(flags) = read_relation_flags(obj, &ctx, warnings) {
                            relation_flags.insert(tag.clone(), flags);
                        }
                    }
                    Value::Null => {
                        countries.insert(tag.clone(), CountryDiplomacy::all_unobtained());
                    }
                    other => {
                        warnings.push(format!(
                            "countries.{tag}: expected object, found {}; diplomacy leaves treated as unobtained",
                            json_type_name(other)
                        ));
                        countries.insert(tag.clone(), CountryDiplomacy::all_unobtained());
                    }
                }
            }
            fold_same_faction_pairs(&countries, &mut pairs);
        }
        Some(Value::Null) => status = combine_status(status, ViewStatus::DomainUnavailable),
        None => {}
        Some(other) => {
            warnings.push(format!(
                "countries: present but wrong-typed (expected object, found {}); diplomacy country view unavailable",
                json_type_name(other)
            ));
            status = combine_status(status, ViewStatus::DomainUnavailable);
        }
    }

    match rest.get("diplomacy_pairs") {
        Some(Value::Object(obj)) => {
            status = ViewStatus::Present;
            fold_explicit_diplomacy_pairs(obj, &mut pairs, warnings);
        }
        Some(Value::Null) => status = combine_status(status, ViewStatus::DomainUnavailable),
        None => {}
        Some(other) => {
            warnings.push(format!(
                "diplomacy_pairs: present but wrong-typed (expected object, found {}); pair view unavailable",
                json_type_name(other)
            ));
            status = combine_status(status, ViewStatus::DomainUnavailable);
        }
    }

    // War casualties retain the diplomacy_live path. Factions prefer the
    // recorder's witnessed d53 roster, with diplomacy_live used only when the
    // d53 status is absent.
    let (war_relations, war_rel_status) = read_war_relations(rest, warnings);
    let (faction_members, faction_status) = match read_domain53_faction_roster(rest, warnings) {
        Some((_, modern_members, modern_status)) => (modern_members, modern_status),
        None => match read_faction_members(rest, warnings) {
            // No modern witness plus no legacy value is unavailable, not
            // positive knowledge of an empty roster.
            (Leaf::Absent, _) => (Leaf::Unobtained, ViewStatus::DomainUnavailable),
            legacy => legacy,
        },
    };
    status = combine_status(combine_status(status, war_rel_status), faction_status);

    DiplomacyView {
        status,
        pairs,
        countries,
        war_relations,
        faction_members,
        relation_flags,
    }
}

/// Read `countries.<owner>.diplomacy.relation_flags` (`{other_tag: [flag,…]}`).
/// Returns `None` when the key is absent (owner omitted from the flags map),
/// `Some(Unobtained)` for explicit-null / wrong-typed, `Some(Measured(map))`
/// for a well-formed object. Flag strings are surfaced verbatim.
fn read_relation_flags(
    country: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Option<Leaf<BTreeMap<String, Vec<String>>>> {
    match walk_path(country, &["diplomacy", "relation_flags"], ctx, warnings) {
        Leaf::Absent => None,
        Leaf::Unobtained => Some(Leaf::Unobtained),
        Leaf::Measured(Value::Object(map)) => {
            let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (other, flags) in map {
                match flags {
                    Value::Array(items) => {
                        let mut list = Vec::with_capacity(items.len());
                        let mut ok = true;
                        for (i, item) in items.iter().enumerate() {
                            match item.as_str() {
                                Some(s) => list.push(s.to_string()),
                                None => {
                                    warnings.push(format!(
                                        "{ctx}.diplomacy.relation_flags.{other}[{i}]: expected flag string, found {}; entry treated as unobtained",
                                        json_type_name(item)
                                    ));
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok {
                            out.insert(other.clone(), list);
                        }
                    }
                    other_v => warnings.push(format!(
                        "{ctx}.diplomacy.relation_flags.{other}: expected array of flag strings, found {}; entry skipped",
                        json_type_name(other_v)
                    )),
                }
            }
            Some(Leaf::Measured(out))
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.diplomacy.relation_flags: present but wrong-typed (expected object, found {}); treated as unobtained",
                json_type_name(other)
            ));
            Some(Leaf::Unobtained)
        }
    }
}

/// Read `diplomacy_live.war_relations` (an array of live belligerent rows with
/// casualty tallies). Returns the typed leaf plus the domain status the
/// presence of `diplomacy_live` contributes.
fn read_war_relations(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (Leaf<Vec<WarRelationRow>>, ViewStatus) {
    match walk_path(
        rest,
        &["diplomacy_live", "war_relations"],
        "snapshot",
        warnings,
    ) {
        Leaf::Absent => (Leaf::Absent, ViewStatus::DomainAbsent),
        Leaf::Unobtained => (Leaf::Unobtained, ViewStatus::DomainUnavailable),
        Leaf::Measured(Value::Array(items)) => {
            let mut rows = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let ctx = format!("diplomacy_live.war_relations[{i}]");
                match item {
                    Value::Object(row) => rows.push(WarRelationRow {
                        first: leaf_string(row, &["first"], &ctx, warnings),
                        second: leaf_string(row, &["second"], &ctx, warnings),
                        first_casualties: leaf_i64(row, &["first_casualties"], &ctx, warnings),
                        second_casualties: leaf_i64(row, &["second_casualties"], &ctx, warnings),
                    }),
                    other => warnings.push(format!(
                        "{ctx}: expected object, found {}; row skipped",
                        json_type_name(other)
                    )),
                }
            }
            (Leaf::Measured(rows), ViewStatus::Present)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "diplomacy_live.war_relations: present but wrong-typed (expected array, found {}); treated as unobtained",
                json_type_name(other)
            ));
            (Leaf::Unobtained, ViewStatus::DomainUnavailable)
        }
    }
}

/// Read `diplomacy_live.faction_members` (`{faction_id: [TAG,…]}`). Returns the
/// typed leaf plus the domain status contribution.
fn unavailable_faction_roster() -> (
    FactionRosterStatus,
    Leaf<BTreeMap<String, Vec<String>>>,
    ViewStatus,
) {
    (
        FactionRosterStatus::Unavailable,
        Leaf::Unobtained,
        ViewStatus::DomainUnavailable,
    )
}

fn parse_domain53_faction_rows(
    extra: &Map<String, Value>,
    expect_nonempty: bool,
    warnings: &mut Vec<String>,
) -> Option<BTreeMap<String, Vec<String>>> {
    let rows = match extra.get("faction_rows") {
        Some(Value::Array(rows)) => rows,
        Some(other) => {
            warnings.push(format!(
                "diplomacy_world.domain53_extra.faction_rows: expected array, found {}; roster treated as unavailable",
                json_type_name(other)
            ));
            return None;
        }
        None => {
            warnings.push(
                "diplomacy_world.domain53_extra.faction_rows: missing beside measured status; roster treated as unavailable"
                    .to_string(),
            );
            return None;
        }
    };
    if rows.is_empty() == expect_nonempty {
        warnings.push(
            "diplomacy_world.domain53_extra: faction status disagrees with faction_rows emptiness; roster treated as unavailable"
                .to_string(),
        );
        return None;
    }

    let mut by_faction = BTreeMap::new();
    let mut faction_by_member = BTreeMap::<String, String>::new();
    for (index, value) in rows.iter().enumerate() {
        let Value::Object(row) = value else {
            warnings.push(format!(
                "diplomacy_world.domain53_extra.faction_rows[{index}]: expected object; roster treated as unavailable"
            ));
            return None;
        };
        let faction_id = match row.get("faction_id").and_then(Value::as_u64) {
            Some(value) if value > 0 && value <= u32::MAX as u64 => value.to_string(),
            _ => {
                warnings.push(format!(
                    "diplomacy_world.domain53_extra.faction_rows[{index}].faction_id: invalid; roster treated as unavailable"
                ));
                return None;
            }
        };
        let members = match row.get("members") {
            Some(Value::Array(items)) if !items.is_empty() => {
                let mut members = Vec::with_capacity(items.len());
                for (member_index, item) in items.iter().enumerate() {
                    let Some(member) = item.as_str().filter(|member| !member.is_empty()) else {
                        warnings.push(format!(
                            "diplomacy_world.domain53_extra.faction_rows[{index}].members[{member_index}]: invalid TAG; roster treated as unavailable"
                        ));
                        return None;
                    };
                    if members.iter().any(|prior| prior == member) {
                        warnings.push(format!(
                            "diplomacy_world.domain53_extra.faction_rows[{index}].members: duplicate TAG {member}; roster treated as unavailable"
                        ));
                        return None;
                    }
                    members.push(member.to_string());
                }
                members
            }
            _ => {
                warnings.push(format!(
                    "diplomacy_world.domain53_extra.faction_rows[{index}].members: expected nonempty TAG array; roster treated as unavailable"
                ));
                return None;
            }
        };
        if row.get("member_count").and_then(Value::as_u64) != Some(members.len() as u64) {
            warnings.push(format!(
                "diplomacy_world.domain53_extra.faction_rows[{index}].member_count: disagrees with members; roster treated as unavailable"
            ));
            return None;
        }
        let Some(leader) = row.get("leader").and_then(Value::as_str) else {
            warnings.push(format!(
                "diplomacy_world.domain53_extra.faction_rows[{index}].leader: invalid; roster treated as unavailable"
            ));
            return None;
        };
        if !members.iter().any(|member| member == leader) {
            warnings.push(format!(
                "diplomacy_world.domain53_extra.faction_rows[{index}].leader: not present in members; roster treated as unavailable"
            ));
            return None;
        }
        for member in &members {
            if let Some(prior_faction) =
                faction_by_member.insert(member.clone(), faction_id.clone())
            {
                if prior_faction != faction_id {
                    warnings.push(format!(
                        "diplomacy_world.domain53_extra.faction_rows: member {member} appears in multiple factions; roster treated as unavailable"
                    ));
                    return None;
                }
            }
        }
        if by_faction.insert(faction_id, members).is_some() {
            warnings.push(format!(
                "diplomacy_world.domain53_extra.faction_rows[{index}].faction_id: duplicate; roster treated as unavailable"
            ));
            return None;
        }
    }
    Some(by_faction)
}

/// Prefer the recorder d53 completeness witness when present. Returning None
/// means this recording predates the field and activates the legacy fallback;
/// any malformed or ambiguous present value instead returns Unavailable.
fn read_domain53_faction_roster(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Option<(
    FactionRosterStatus,
    Leaf<BTreeMap<String, Vec<String>>>,
    ViewStatus,
)> {
    let world = match rest.get("diplomacy_world") {
        Some(Value::Object(world)) => world,
        None | Some(Value::Null) => return None,
        Some(other) => {
            warnings.push(format!(
                "diplomacy_world: expected object, found {}; faction roster treated as unavailable",
                json_type_name(other)
            ));
            return Some(unavailable_faction_roster());
        }
    };
    let extra = match world.get("domain53_extra") {
        Some(Value::Object(extra)) => extra,
        None => return None,
        Some(other) => {
            warnings.push(format!(
                "diplomacy_world.domain53_extra: expected object, found {}; faction roster treated as unavailable",
                json_type_name(other)
            ));
            return Some(unavailable_faction_roster());
        }
    };
    let status = match extra.get("faction_roster_status") {
        Some(Value::String(status)) => status.as_str(),
        None => return None,
        Some(other) => {
            warnings.push(format!(
                "diplomacy_world.domain53_extra.faction_roster_status: expected string, found {}; roster treated as unavailable",
                json_type_name(other)
            ));
            return Some(unavailable_faction_roster());
        }
    };
    match status {
        "unavailable" => Some(unavailable_faction_roster()),
        "measured_empty" | "measured_nonempty" => {
            let expect_nonempty = status == "measured_nonempty";
            match parse_domain53_faction_rows(extra, expect_nonempty, warnings) {
                Some(rows) => Some((
                    if expect_nonempty {
                        FactionRosterStatus::MeasuredNonEmpty
                    } else {
                        FactionRosterStatus::MeasuredEmpty
                    },
                    Leaf::Measured(rows),
                    ViewStatus::Present,
                )),
                None => Some(unavailable_faction_roster()),
            }
        }
        "ambiguous_member_exclusivity" => {
            warnings.push(
                "diplomacy_world.domain53_extra.faction_roster_status: ambiguous member exclusivity; roster treated as unavailable"
                    .to_string(),
            );
            Some(unavailable_faction_roster())
        }
        other => {
            warnings.push(format!(
                "diplomacy_world.domain53_extra.faction_roster_status: unknown value {other}; roster treated as unavailable"
            ));
            Some(unavailable_faction_roster())
        }
    }
}

fn read_faction_members(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (Leaf<BTreeMap<String, Vec<String>>>, ViewStatus) {
    match walk_path(
        rest,
        &["diplomacy_live", "faction_members"],
        "snapshot",
        warnings,
    ) {
        Leaf::Absent => (Leaf::Absent, ViewStatus::DomainAbsent),
        Leaf::Unobtained => (Leaf::Unobtained, ViewStatus::DomainUnavailable),
        Leaf::Measured(Value::Object(map)) => {
            if map.is_empty() {
                warnings.push(
                    "diplomacy_live.faction_members: empty legacy object has no completeness witness; treated as unobtained"
                        .to_string(),
                );
                return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
            }
            let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut faction_by_member: BTreeMap<String, String> = BTreeMap::new();
            for (faction_id, members) in map {
                if faction_id.is_empty() {
                    warnings.push(
                        "diplomacy_live.faction_members: empty faction id; whole roster treated as unobtained"
                            .to_string(),
                    );
                    return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
                }
                let Value::Array(items) = members else {
                    warnings.push(format!(
                        "diplomacy_live.faction_members.{faction_id}: expected array of TAG strings, found {}; whole roster treated as unobtained",
                        json_type_name(members)
                    ));
                    return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
                };
                if items.is_empty() {
                    warnings.push(format!(
                        "diplomacy_live.faction_members.{faction_id}: empty member array has no valid v1 roster row; whole roster treated as unobtained"
                    ));
                    return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
                }
                let mut list = Vec::with_capacity(items.len());
                let mut seen_members = BTreeSet::new();
                for (i, item) in items.iter().enumerate() {
                    let Some(member) = item.as_str().filter(|member| !member.is_empty()) else {
                        warnings.push(format!(
                            "diplomacy_live.faction_members.{faction_id}[{i}]: expected nonempty TAG string, found {}; whole roster treated as unobtained",
                            json_type_name(item)
                        ));
                        return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
                    };
                    if !seen_members.insert(member) {
                        warnings.push(format!(
                            "diplomacy_live.faction_members.{faction_id}: duplicate member {member}; whole roster treated as unobtained"
                        ));
                        return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
                    }
                    if let Some(prior_faction) =
                        faction_by_member.insert(member.to_string(), faction_id.clone())
                    {
                        warnings.push(format!(
                            "diplomacy_live.faction_members: member {member} appears in factions {prior_faction} and {faction_id}; whole roster treated as unobtained"
                        ));
                        return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
                    }
                    list.push(member.to_string());
                }
                out.insert(faction_id.clone(), list);
            }
            (Leaf::Measured(out), ViewStatus::Present)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "diplomacy_live.faction_members: present but wrong-typed (expected object, found {}); treated as unobtained",
                json_type_name(other)
            ));
            (Leaf::Unobtained, ViewStatus::DomainUnavailable)
        }
    }
}

fn pair_entry<'a>(
    pairs: &'a mut BTreeMap<(String, String), DiplomaticPairState>,
    a: &str,
    b: &str,
) -> &'a mut DiplomaticPairState {
    pairs
        .entry((a.to_string(), b.to_string()))
        .or_insert_with(DiplomaticPairState::all_absent)
}

fn fold_country_diplomacy_pairs(
    tag: &str,
    country: &Map<String, Value>,
    pairs: &mut BTreeMap<(String, String), DiplomaticPairState>,
    warnings: &mut Vec<String>,
) {
    let ctx = format!("countries.{tag}");
    let diplomacy = match walk_path(country, &["diplomacy"], &ctx, warnings) {
        Leaf::Measured(Value::Object(obj)) => Some(obj),
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.diplomacy: present but wrong-typed (expected object, found {}); relation arrays skipped",
                json_type_name(other)
            ));
            None
        }
        _ => None,
    };
    if let Some(diplomacy) = diplomacy {
        for other in tags_from_relation_list(diplomacy, "wars", "against", &ctx, warnings) {
            pair_entry(pairs, tag, &other).at_war = Leaf::Measured(true);
        }
        for other in tags_from_simple_list(diplomacy, "non_aggression", &ctx, warnings) {
            pair_entry(pairs, tag, &other).non_aggression_pact = Leaf::Measured(true);
        }
        for other in tags_from_simple_list(diplomacy, "guarantees", &ctx, warnings) {
            pair_entry(pairs, tag, &other).guarantee = Leaf::Measured(true);
        }
        for other in tags_from_simple_list(diplomacy, "embargoes", &ctx, warnings) {
            pair_entry(pairs, tag, &other).embargo = Leaf::Measured(true);
        }
        for other in tags_from_simple_list(diplomacy, "military_access", &ctx, warnings) {
            pair_entry(pairs, tag, &other).military_access = Leaf::Measured(true);
        }
    }
    match leaf_string(
        country,
        &["politics", "autonomy", "overlord"],
        &ctx,
        warnings,
    ) {
        Leaf::Measured(overlord) => {
            pair_entry(pairs, tag, &overlord).subject = Leaf::Measured(true)
        }
        Leaf::Unobtained | Leaf::Absent => {}
    }
}

fn tags_from_relation_list(
    obj: &Map<String, Value>,
    key: &str,
    nested_key: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    match obj.get(key) {
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::Object(row) => match row.get(nested_key) {
                        Some(Value::Array(tags)) => {
                            for (j, tag) in tags.iter().enumerate() {
                                match tag.as_str() {
                                    Some(tag) => out.push(tag.to_string()),
                                    None => warnings.push(format!(
                                        "{ctx}.diplomacy.{key}[{i}].{nested_key}[{j}]: expected TAG string, found {}; skipped",
                                        json_type_name(tag)
                                    )),
                                }
                            }
                        }
                        Some(Value::String(tag)) => out.push(tag.clone()),
                        Some(Value::Null) | None => {}
                        Some(other) => warnings.push(format!(
                            "{ctx}.diplomacy.{key}[{i}].{nested_key}: expected array/string, found {}; skipped",
                            json_type_name(other)
                        )),
                    },
                    Value::String(tag) => out.push(tag.clone()),
                    other => warnings.push(format!(
                        "{ctx}.diplomacy.{key}[{i}]: expected object/string, found {}; skipped",
                        json_type_name(other)
                    )),
                }
            }
        }
        Some(Value::Null) | None => {}
        Some(other) => warnings.push(format!(
            "{ctx}.diplomacy.{key}: present but wrong-typed (expected array, found {}); skipped",
            json_type_name(other)
        )),
    }
    out
}

fn tags_from_simple_list(
    obj: &Map<String, Value>,
    key: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    match obj.get(key) {
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::String(tag) => out.push(tag.clone()),
                    Value::Object(row) => {
                        if let Some(tag) = row
                            .get("tag")
                            .or_else(|| row.get("target"))
                            .or_else(|| row.get("other"))
                            .and_then(Value::as_str)
                        {
                            out.push(tag.to_string());
                        } else {
                            warnings.push(format!(
                                "{ctx}.diplomacy.{key}[{i}]: object has no tag/target/other; skipped"
                            ));
                        }
                    }
                    other => warnings.push(format!(
                        "{ctx}.diplomacy.{key}[{i}]: expected string/object, found {}; skipped",
                        json_type_name(other)
                    )),
                }
            }
        }
        Some(Value::Null) | None => {}
        Some(other) => warnings.push(format!(
            "{ctx}.diplomacy.{key}: present but wrong-typed (expected array, found {}); skipped",
            json_type_name(other)
        )),
    }
    out
}

fn fold_same_faction_pairs(
    countries: &BTreeMap<String, CountryDiplomacy>,
    pairs: &mut BTreeMap<(String, String), DiplomaticPairState>,
) {
    let measured: Vec<(&str, &str)> = countries
        .iter()
        .filter_map(|(tag, dip)| {
            dip.faction_name
                .measured()
                .map(|name| (tag.as_str(), name.as_str()))
        })
        .collect();
    for (a, faction_a) in &measured {
        for (b, faction_b) in &measured {
            if a != b && faction_a == faction_b {
                pair_entry(pairs, a, b).same_faction = Leaf::Measured(true);
            }
        }
    }
}

fn fold_explicit_diplomacy_pairs(
    obj: &Map<String, Value>,
    pairs: &mut BTreeMap<(String, String), DiplomaticPairState>,
    warnings: &mut Vec<String>,
) {
    for (key, value) in obj {
        let (owner, viewed, row) = match value {
            Value::Object(row) => {
                let parsed = parse_pair_key(key).or_else(|| {
                    let owner = row
                        .get("owner")
                        .or_else(|| row.get("a"))
                        .and_then(Value::as_str)?;
                    let viewed = row
                        .get("viewed")
                        .or_else(|| row.get("b"))
                        .and_then(Value::as_str)?;
                    Some((owner.to_string(), viewed.to_string()))
                });
                match parsed {
                    Some((owner, viewed)) => (owner, viewed, row),
                    None => {
                        warnings.push(format!(
                            "diplomacy_pairs.{key}: cannot determine owner/viewed pair; skipped"
                        ));
                        continue;
                    }
                }
            }
            Value::Null => {
                if let Some((owner, viewed)) = parse_pair_key(key) {
                    pairs.insert((owner, viewed), DiplomaticPairState::all_unobtained());
                }
                continue;
            }
            other => {
                warnings.push(format!(
                    "diplomacy_pairs.{key}: expected object, found {}; treated as unobtained when key is parseable",
                    json_type_name(other)
                ));
                if let Some((owner, viewed)) = parse_pair_key(key) {
                    pairs.insert((owner, viewed), DiplomaticPairState::all_unobtained());
                }
                continue;
            }
        };
        let ctx = format!("diplomacy_pairs.{key}");
        let entry = pair_entry(pairs, &owner, &viewed);
        entry.at_war = leaf_bool(row, &["at_war"], &ctx, warnings);
        entry.same_faction = leaf_bool(row, &["same_faction"], &ctx, warnings);
        entry.non_aggression_pact =
            leaf_bool_alias(row, &["non_aggression_pact", "nap"], &ctx, warnings);
        entry.military_access =
            leaf_bool_alias(row, &["military_access", "access"], &ctx, warnings);
        entry.guarantee = leaf_bool(row, &["guarantee"], &ctx, warnings);
        entry.embargo = leaf_bool(row, &["embargo"], &ctx, warnings);
        entry.subject = leaf_bool(row, &["subject"], &ctx, warnings);
    }
}

fn parse_pair_key(key: &str) -> Option<(String, String)> {
    for sep in ["|", "->", ":"] {
        if let Some((a, b)) = key.split_once(sep) {
            if !a.is_empty() && !b.is_empty() {
                return Some((a.to_string(), b.to_string()));
            }
        }
    }
    None
}

fn build_topbar_scalars(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, TopbarScalars>, ViewStatus) {
    let entries = match rest.get("countries") {
        Some(Value::Object(entries)) => entries,
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        None => return (BTreeMap::new(), ViewStatus::DomainAbsent),
        Some(other) => {
            warnings.push(format!(
                "countries: present but wrong-typed (expected object, found {}); topbar view unavailable",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
    };
    let world_tension = topbar_world_tension(rest, warnings);
    let mut out = BTreeMap::new();
    for (tag, entry) in entries {
        match entry {
            Value::Object(obj) => {
                let ctx = format!("countries.{tag}");
                out.insert(
                    tag.clone(),
                    TopbarScalars {
                        political_power: leaf_f64(
                            obj,
                            &["politics", "political_power"],
                            &ctx,
                            warnings,
                        ),
                        stability: leaf_f64(obj, &["politics", "stability"], &ctx, warnings),
                        war_support: leaf_f64(obj, &["politics", "war_support"], &ctx, warnings),
                        command_power: first_available_f64([
                            leaf_f64(obj, &["politics", "command_power"], &ctx, warnings),
                            leaf_f64(obj, &["military", "command_power"], &ctx, warnings),
                        ]),
                        manpower_available: leaf_i64(
                            obj,
                            &["military", "manpower", "available"],
                            &ctx,
                            warnings,
                        ),
                        xp_army: leaf_f64(obj, &["military", "xp", "army"], &ctx, warnings),
                        xp_navy: leaf_f64(obj, &["military", "xp", "navy"], &ctx, warnings),
                        xp_air: leaf_f64(obj, &["military", "xp", "air"], &ctx, warnings),
                        factories_civ: leaf_i64(
                            obj,
                            &["economy", "factories", "civ"],
                            &ctx,
                            warnings,
                        ),
                        factories_mil: leaf_i64(
                            obj,
                            &["economy", "factories", "mil"],
                            &ctx,
                            warnings,
                        ),
                        factories_dockyard: leaf_i64(
                            obj,
                            &["economy", "factories", "dockyard"],
                            &ctx,
                            warnings,
                        ),
                        fuel_current: leaf_f64(
                            obj,
                            &["economy", "fuel", "current"],
                            &ctx,
                            warnings,
                        ),
                        fuel_max: leaf_f64(obj, &["economy", "fuel", "max"], &ctx, warnings),
                        convoys: leaf_convoys(obj, &ctx, warnings),
                        nukes_total: leaf_i64(obj, &["extra", "nukes_total"], &ctx, warnings),
                        nukes_ready: leaf_i64(obj, &["extra", "nukes_ready_total"], &ctx, warnings),
                        world_tension: world_tension.clone(),
                        // A3 round-trip: the canonical supply figures live
                        // under economy.logistics, but the topbar scalar sweep
                        // (requirements item 9b) reports them under the derived
                        // `scalars` bag; accept either, canonical first.
                        supply_value: first_available_f64([
                            leaf_f64(
                                obj,
                                &["economy", "logistics", "supply_value"],
                                &ctx,
                                warnings,
                            ),
                            leaf_f64(obj, &["scalars", "supply_value"], &ctx, warnings),
                        ]),
                        supply_ratio: first_available_f64([
                            leaf_f64(
                                obj,
                                &["economy", "logistics", "supply_ratio"],
                                &ctx,
                                warnings,
                            ),
                            leaf_f64(obj, &["scalars", "supply_ratio"], &ctx, warnings),
                        ]),
                    },
                );
            }
            Value::Null => {
                out.insert(tag.clone(), TopbarScalars::all_unobtained());
            }
            other => {
                warnings.push(format!(
                    "countries.{tag}: expected object, found {}; topbar leaves treated as unobtained",
                    json_type_name(other)
                ));
                out.insert(tag.clone(), TopbarScalars::all_unobtained());
            }
        }
    }
    (out, ViewStatus::Present)
}

/// Per-POV topbar ratio bars + alert counts (requirements item 9b). Both live
/// in the derived `countries.<tag>.scalars` bag (the topbar scalar sweep);
/// mirrors [`build_topbar_scalars`]'s domain handling so the trichotomy and
/// status are consistent across the topbar surfaces.
fn build_topbar_extras(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (
    BTreeMap<String, TopbarRatios>,
    BTreeMap<String, TopbarAlerts>,
    ViewStatus,
) {
    let entries = match rest.get("countries") {
        Some(Value::Object(entries)) => entries,
        Some(Value::Null) => {
            return (
                BTreeMap::new(),
                BTreeMap::new(),
                ViewStatus::DomainUnavailable,
            )
        }
        None => return (BTreeMap::new(), BTreeMap::new(), ViewStatus::DomainAbsent),
        Some(other) => {
            warnings.push(format!(
                "countries: present but wrong-typed (expected object, found {}); topbar ratio/alert view unavailable",
                json_type_name(other)
            ));
            return (
                BTreeMap::new(),
                BTreeMap::new(),
                ViewStatus::DomainUnavailable,
            );
        }
    };
    let mut ratios = BTreeMap::new();
    let mut alerts = BTreeMap::new();
    for (tag, entry) in entries {
        match entry {
            Value::Object(obj) => {
                let ctx = format!("countries.{tag}");
                ratios.insert(
                    tag.clone(),
                    TopbarRatios {
                        industrial_ratio: leaf_f64(
                            obj,
                            &["scalars", "industrial_ratio"],
                            &ctx,
                            warnings,
                        ),
                        convoy_ratio: leaf_f64(obj, &["scalars", "convoy_ratio"], &ctx, warnings),
                        fuel_ratio: leaf_f64(obj, &["scalars", "fuel_ratio"], &ctx, warnings),
                        xp_ratio: leaf_f64(obj, &["scalars", "xp_ratio"], &ctx, warnings),
                    },
                );
                alerts.insert(
                    tag.clone(),
                    TopbarAlerts {
                        initiative: leaf_i64(
                            obj,
                            &["scalars", "alerts", "initiative"],
                            &ctx,
                            warnings,
                        ),
                        decisions_available: leaf_i64(
                            obj,
                            &["scalars", "alerts", "decisions_available"],
                            &ctx,
                            warnings,
                        ),
                        intel: leaf_i64(obj, &["scalars", "alerts", "intel"], &ctx, warnings),
                        market_contracts: leaf_i64(
                            obj,
                            &["scalars", "alerts", "market_contracts"],
                            &ctx,
                            warnings,
                        ),
                    },
                );
            }
            Value::Null => {
                ratios.insert(tag.clone(), TopbarRatios::all_unobtained());
                alerts.insert(tag.clone(), TopbarAlerts::all_unobtained());
            }
            other => {
                warnings.push(format!(
                    "countries.{tag}: expected object, found {}; topbar ratio/alert leaves treated as unobtained",
                    json_type_name(other)
                ));
                ratios.insert(tag.clone(), TopbarRatios::all_unobtained());
                alerts.insert(tag.clone(), TopbarAlerts::all_unobtained());
            }
        }
    }
    (ratios, alerts, ViewStatus::Present)
}

fn topbar_world_tension(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> Leaf<f64> {
    let mut saw_unobtained = false;
    for path in [
        &["world", "world_tension"][..],
        &["world", "tension"][..],
        &["world_events", "world_tension"][..],
    ] {
        let leaf = match walk_path(rest, path, "snapshot", warnings) {
            Leaf::Absent => Leaf::Absent,
            Leaf::Unobtained => Leaf::Unobtained,
            Leaf::Measured(v) => match v.as_f64() {
                Some(v) => Leaf::Measured(v),
                None => {
                    warnings.push(format!(
                        "snapshot.{}: present but wrong-typed (expected number, found {}); treated as unobtained",
                        path.join("."),
                        json_type_name(v)
                    ));
                    Leaf::Unobtained
                }
            },
        };
        match leaf {
            Leaf::Measured(v) => return Leaf::Measured(v),
            Leaf::Unobtained => saw_unobtained = true,
            Leaf::Absent => {}
        }
    }
    if saw_unobtained {
        Leaf::Unobtained
    } else {
        Leaf::Absent
    }
}

fn leaf_convoys(obj: &Map<String, Value>, ctx: &str, warnings: &mut Vec<String>) -> Leaf<f64> {
    match leaf_f64(obj, &["military", "convoy_count"], ctx, warnings) {
        Leaf::Measured(v) => return Leaf::Measured(v),
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Absent => {}
    }
    // A3 round-trip: the topbar scalar sweep (requirements item 4/9b) reports
    // the convoy stockpile as the scalar `military.convoys_count`; prefer it
    // over summing the per-type `military.convoys` array when present.
    match leaf_f64(obj, &["military", "convoys_count"], ctx, warnings) {
        Leaf::Measured(v) => return Leaf::Measured(v),
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Absent => {}
    }
    match walk_path(obj, &["military", "convoys"], ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut total = 0.0;
            for (i, item) in items.iter().enumerate() {
                let row = match item.as_object() {
                    Some(row) => row,
                    None => {
                        warnings.push(format!(
                            "{ctx}.military.convoys[{i}]: expected object, found {}; count treated as unobtained",
                            json_type_name(item)
                        ));
                        return Leaf::Unobtained;
                    }
                };
                match row.get("amount").and_then(Value::as_f64) {
                    Some(amount) => total += amount,
                    None => {
                        warnings.push(format!(
                            "{ctx}.military.convoys[{i}].amount missing or non-number; count treated as unobtained"
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(total)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.military.convoys: present but wrong-typed (expected array, found {}); treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn build_buildings(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<u32, StateBuildings>, ViewStatus) {
    let states = match rest.get("states") {
        Some(Value::Object(states)) => states,
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        None => return (BTreeMap::new(), ViewStatus::DomainAbsent),
        Some(other) => {
            warnings.push(format!(
                "states: present but wrong-typed (expected object, found {}); building view unavailable",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
    };
    let mut out = BTreeMap::new();
    let mut status = ViewStatus::DomainAbsent;
    for (state_key, state_value) in states {
        let state_id = match state_key.parse::<u32>() {
            Ok(id) => id,
            Err(_) => {
                warnings.push(format!(
                    "states: non-numeric state id {state_key:?} skipped"
                ));
                continue;
            }
        };
        let state = match state_value {
            Value::Object(state) => state,
            Value::Null => {
                status = combine_status(status, ViewStatus::DomainUnavailable);
                out.insert(
                    state_id,
                    StateBuildings {
                        status: ViewStatus::DomainUnavailable,
                        state_buildings: BTreeMap::new(),
                        province_buildings: BTreeMap::new(),
                    },
                );
                continue;
            }
            other => {
                warnings.push(format!(
                    "states.{state_key}: expected object, found {}; building leaves treated as unobtained",
                    json_type_name(other)
                ));
                status = combine_status(status, ViewStatus::DomainUnavailable);
                out.insert(
                    state_id,
                    StateBuildings {
                        status: ViewStatus::DomainUnavailable,
                        state_buildings: BTreeMap::new(),
                        province_buildings: BTreeMap::new(),
                    },
                );
                continue;
            }
        };
        let (mut state_buildings, state_status) = parse_building_map(
            state.get("buildings"),
            &format!("states.{state_key}.buildings"),
            warnings,
        );
        let (mut province_buildings, province_status) = parse_province_buildings(
            state.get("province_buildings"),
            &format!("states.{state_key}.province_buildings"),
            warnings,
        );
        // A3 round-trip: building damage / repair / construction (requirements
        // item 10) arrives as a per-state `building_damage` array of scoped
        // rows; overlay each row onto the level-only building maps so a
        // consumer sees damaged/healthy_level/repair without a second lookup.
        let damage_status = fold_building_damage(
            state.get("building_damage"),
            &mut state_buildings,
            &mut province_buildings,
            &format!("states.{state_key}.building_damage"),
            warnings,
        );
        let row_status =
            combine_status(combine_status(state_status, province_status), damage_status);
        status = combine_status(status, row_status);
        if row_status != ViewStatus::DomainAbsent {
            out.insert(
                state_id,
                StateBuildings {
                    status: row_status,
                    state_buildings,
                    province_buildings,
                },
            );
        }
    }
    (out, status)
}

fn parse_building_map(
    value: Option<&Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, BuildingStatus>, ViewStatus) {
    let Some(value) = value else {
        return (BTreeMap::new(), ViewStatus::DomainAbsent);
    };
    match value {
        Value::Object(obj) => {
            let mut out = BTreeMap::new();
            for (building, value) in obj {
                out.insert(
                    building.clone(),
                    parse_building_status(value, &format!("{ctx}.{building}"), warnings),
                );
            }
            (out, ViewStatus::Present)
        }
        Value::Null => (BTreeMap::new(), ViewStatus::DomainUnavailable),
        other => {
            warnings.push(format!(
                "{ctx}: present but wrong-typed (expected object, found {}); treated as unavailable",
                json_type_name(other)
            ));
            (BTreeMap::new(), ViewStatus::DomainUnavailable)
        }
    }
}

fn parse_province_buildings(
    value: Option<&Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> (BTreeMap<u32, BTreeMap<String, BuildingStatus>>, ViewStatus) {
    let Some(value) = value else {
        return (BTreeMap::new(), ViewStatus::DomainAbsent);
    };
    match value {
        Value::Object(provinces) => {
            let mut out = BTreeMap::new();
            for (province_key, value) in provinces {
                let province_id = match province_key.parse::<u32>() {
                    Ok(id) => id,
                    Err(_) => {
                        warnings.push(format!(
                            "{ctx}: non-numeric province id {province_key:?} skipped"
                        ));
                        continue;
                    }
                };
                let (buildings, _) =
                    parse_building_map(Some(value), &format!("{ctx}.{province_key}"), warnings);
                out.insert(province_id, buildings);
            }
            (out, ViewStatus::Present)
        }
        Value::Null => (BTreeMap::new(), ViewStatus::DomainUnavailable),
        other => {
            warnings.push(format!(
                "{ctx}: present but wrong-typed (expected object, found {}); treated as unavailable",
                json_type_name(other)
            ));
            (BTreeMap::new(), ViewStatus::DomainUnavailable)
        }
    }
}

fn parse_building_status(value: &Value, ctx: &str, warnings: &mut Vec<String>) -> BuildingStatus {
    match value {
        Value::Number(n) => n
            .as_f64()
            .map(BuildingStatus::level_only)
            .unwrap_or_else(BuildingStatus::all_unobtained),
        Value::Null => BuildingStatus::all_unobtained(),
        Value::Object(obj) => BuildingStatus {
            level: leaf_f64(obj, &["level"], ctx, warnings),
            healthy_level: leaf_f64(obj, &["healthy_level"], ctx, warnings),
            damaged: leaf_bool(obj, &["damaged"], ctx, warnings),
            damage_fraction: first_available_f64([
                leaf_f64(obj, &["damage_fraction"], ctx, warnings),
                leaf_f64(obj, &["damage"], ctx, warnings),
            ]),
            repair_progress: leaf_f64(obj, &["repair_progress"], ctx, warnings),
            under_construction: leaf_bool(obj, &["under_construction"], ctx, warnings),
        },
        other => {
            warnings.push(format!(
                "{ctx}: expected number/object/null building row, found {}; treated as unobtained",
                json_type_name(other)
            ));
            BuildingStatus::all_unobtained()
        }
    }
}

/// Overlay a per-state `building_damage` array (requirements item 10) onto the
/// level-only building maps. Each row carries `scope` (`"state"`/`"province"`),
/// `scope_id`, `building_type`, and the same status fields a building object
/// would; a state-scope row updates `state_buildings[building_type]`, a
/// province-scope row updates `province_buildings[scope_id][building_type]`.
/// Returns `Present` if any row was folded, else `DomainAbsent`.
fn fold_building_damage(
    value: Option<&Value>,
    state_buildings: &mut BTreeMap<String, BuildingStatus>,
    province_buildings: &mut BTreeMap<u32, BTreeMap<String, BuildingStatus>>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> ViewStatus {
    let items = match value {
        None => return ViewStatus::DomainAbsent,
        Some(Value::Null) => return ViewStatus::DomainUnavailable,
        Some(Value::Array(items)) => items,
        Some(other) => {
            warnings.push(format!(
                "{ctx}: present but wrong-typed (expected array, found {}); building damage ignored",
                json_type_name(other)
            ));
            return ViewStatus::DomainUnavailable;
        }
    };
    let mut status = ViewStatus::DomainAbsent;
    for (i, item) in items.iter().enumerate() {
        let row = match item.as_object() {
            Some(row) => row,
            None => {
                warnings.push(format!(
                    "{ctx}[{i}]: expected object, found {}; row skipped",
                    json_type_name(item)
                ));
                continue;
            }
        };
        let building_type = match row.get("building_type").and_then(Value::as_str) {
            Some(bt) => bt.to_string(),
            None => {
                warnings.push(format!(
                    "{ctx}[{i}]: missing or non-string building_type; row skipped"
                ));
                continue;
            }
        };
        let rctx = format!("{ctx}[{i}]");
        let damaged = parse_building_status(item, &rctx, warnings);
        match row.get("scope").and_then(Value::as_str) {
            Some("state") | None => {
                state_buildings.insert(building_type, damaged);
                status = ViewStatus::Present;
            }
            Some("province") => match row.get("scope_id").and_then(Value::as_u64) {
                Some(pid) if u32::try_from(pid).is_ok() => {
                    province_buildings
                        .entry(pid as u32)
                        .or_default()
                        .insert(building_type, damaged);
                    status = ViewStatus::Present;
                }
                _ => warnings.push(format!(
                    "{rctx}: province-scope row has missing/invalid scope_id; row skipped"
                )),
            },
            Some(other) => warnings.push(format!(
                "{rctx}: unknown building_damage scope {other:?}; row skipped"
            )),
        }
    }
    status
}

fn build_visibility(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, PovVisibilityView>, ViewStatus) {
    let (visibility, status) = object_at_path(rest, &["visibility"], "visibility", warnings);
    let Some(visibility) = visibility else {
        return (BTreeMap::new(), status);
    };
    // A3 round-trip: per-POV visibility (requirements item 13) is nested under
    // `by_pov`, or under `povs` in the recorder's own shape; accept either.
    let by_pov = match visibility.get("by_pov").or_else(|| visibility.get("povs")) {
        Some(Value::Object(obj)) => obj,
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        Some(other) => {
            warnings.push(format!(
                "visibility.by_pov: present but wrong-typed (expected object, found {}); view unavailable",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
        None => visibility,
    };
    let mut out = BTreeMap::new();
    for (pov, value) in by_pov {
        match value {
            Value::Object(obj) => {
                out.insert(pov.clone(), parse_pov_visibility(obj, pov, warnings));
            }
            Value::Null => {
                out.insert(
                    pov.clone(),
                    PovVisibilityView::with_status(ViewStatus::DomainUnavailable),
                );
            }
            other => {
                warnings.push(format!(
                    "visibility.{pov}: expected object, found {}; POV view unavailable",
                    json_type_name(other)
                ));
                out.insert(
                    pov.clone(),
                    PovVisibilityView::with_status(ViewStatus::DomainUnavailable),
                );
            }
        }
    }
    (out, ViewStatus::Present)
}

fn parse_pov_visibility(
    obj: &Map<String, Value>,
    pov: &str,
    warnings: &mut Vec<String>,
) -> PovVisibilityView {
    let province_value = obj
        .get("provinces")
        .or_else(|| obj.get("province_visibility"));
    let mut status = ViewStatus::DomainAbsent;
    let mut provinces = BTreeMap::new();
    match province_value {
        Some(Value::Object(map)) => {
            status = ViewStatus::Present;
            for (key, value) in map {
                let province_id = match key.parse::<u32>() {
                    Ok(id) => id,
                    Err(_) => {
                        warnings.push(format!(
                            "visibility.{pov}.provinces: non-numeric province id {key:?} skipped"
                        ));
                        continue;
                    }
                };
                provinces.insert(
                    province_id,
                    match value {
                        Value::Bool(true) => Leaf::Measured(ProvinceVisibility {
                            level: "visible".to_string(),
                        }),
                        Value::Bool(false) => Leaf::Measured(ProvinceVisibility {
                            level: "fogged".to_string(),
                        }),
                        Value::String(level) => Leaf::Measured(ProvinceVisibility {
                            level: level.clone(),
                        }),
                        Value::Null => Leaf::Unobtained,
                        Value::Object(row) => match leaf_string(
                            row,
                            &["level"],
                            &format!("visibility.{pov}.provinces.{key}"),
                            warnings,
                        ) {
                            Leaf::Measured(level) => Leaf::Measured(ProvinceVisibility { level }),
                            Leaf::Unobtained => Leaf::Unobtained,
                            Leaf::Absent => Leaf::Unobtained,
                        },
                        other => {
                            warnings.push(format!(
                                "visibility.{pov}.provinces.{key}: expected bool/string/object/null, found {}; treated as unobtained",
                                json_type_name(other)
                            ));
                            Leaf::Unobtained
                        }
                    },
                );
            }
        }
        Some(Value::Null) => status = combine_status(status, ViewStatus::DomainUnavailable),
        Some(other) => {
            warnings.push(format!(
                "visibility.{pov}.provinces: expected object, found {}; treated as unavailable",
                json_type_name(other)
            ));
            status = combine_status(status, ViewStatus::DomainUnavailable);
        }
        None => {}
    }
    // A3 round-trip: spotted units are `spotted_units`, or `visible_foreign_units`
    // in the recorder's own per-POV shape (requirements item 13).
    let spotted_units = parse_spotted_units(
        obj.get("spotted_units")
            .or_else(|| obj.get("visible_foreign_units")),
        pov,
        warnings,
    );
    status = combine_status(status, spotted_units.status);
    PovVisibilityView {
        status,
        provinces,
        spotted_units,
    }
}

fn parse_spotted_units(
    value: Option<&Value>,
    pov: &str,
    warnings: &mut Vec<String>,
) -> SpottedUnitsView {
    let Some(value) = value else {
        return SpottedUnitsView::with_status(ViewStatus::DomainAbsent);
    };
    match value {
        Value::Object(obj) => {
            let ctx = format!("visibility.{pov}.spotted_units");
            SpottedUnitsView {
                status: ViewStatus::Present,
                divisions: leaf_string_list(obj, &["divisions"], &ctx, warnings),
                fleets: leaf_string_list(obj, &["fleets"], &ctx, warnings),
                wings: leaf_string_list(obj, &["wings"], &ctx, warnings),
            }
        }
        Value::Null => SpottedUnitsView::with_status(ViewStatus::DomainUnavailable),
        other => {
            warnings.push(format!(
                "visibility.{pov}.spotted_units: expected object, found {}; treated as unavailable",
                json_type_name(other)
            ));
            SpottedUnitsView::with_status(ViewStatus::DomainUnavailable)
        }
    }
}

fn build_provenance(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> ProvenanceView {
    let prov = match rest.get("provenance") {
        Some(Value::Object(prov)) => prov,
        None => return ProvenanceView::default(),
        Some(Value::Null) => {
            warnings.push(
                "provenance: present but null; problem-backed typed views unavailable".to_string(),
            );
            return ProvenanceView::default();
        }
        Some(other) => {
            warnings.push(format!(
                "provenance: present but wrong-typed (expected object, found {}); \
                 provenance view is empty and problem-backed typed views are unavailable",
                json_type_name(other)
            ));
            return ProvenanceView::default();
        }
    };
    let mut channels = BTreeMap::new();
    for channel in ["collector", "memory", "save"] {
        match prov.get(channel) {
            None => {}
            Some(Value::Null) => {
                if channel == "collector" {
                    warnings.push(
                        "provenance.collector: present but null; problem-backed typed views unavailable"
                            .to_string(),
                    );
                }
                channels.insert(
                    channel.to_string(),
                    ChannelCoverage {
                        captured: Leaf::Unobtained,
                        domains: Vec::new(),
                        problems: Vec::new(),
                    },
                );
            }
            Some(Value::Object(obj)) => {
                let ctx = format!("provenance.{channel}");
                let mut domains = Vec::new();
                match obj.get("domains") {
                    None | Some(Value::Null) => {}
                    Some(Value::Array(items)) => {
                        for (i, item) in items.iter().enumerate() {
                            match item.as_str() {
                                Some(s) => domains.push(s.to_string()),
                                None => warnings.push(format!(
                                    "{ctx}.domains[{i}]: expected string, found {}; \
                                     element skipped",
                                    json_type_name(item)
                                )),
                            }
                        }
                    }
                    Some(other) => warnings.push(format!(
                        "{ctx}.domains: present but wrong-typed (expected array, found {}); \
                         treated as empty",
                        json_type_name(other)
                    )),
                }
                let problems = build_domain_problems(obj, &ctx, warnings);
                channels.insert(
                    channel.to_string(),
                    ChannelCoverage {
                        captured: leaf_bool(obj, &["captured"], &ctx, warnings),
                        domains,
                        problems,
                    },
                );
            }
            Some(other) => {
                let consequence = if channel == "collector" {
                    "treated as unobtained and problem-backed typed views are unavailable"
                } else {
                    "treated as unobtained"
                };
                warnings.push(format!(
                    "provenance.{channel}: present but wrong-typed (expected object, \
                     found {}); {consequence}",
                    json_type_name(other),
                ));
                channels.insert(
                    channel.to_string(),
                    ChannelCoverage {
                        captured: Leaf::Unobtained,
                        domains: Vec::new(),
                        problems: Vec::new(),
                    },
                );
            }
        }
    }
    let mut notes = Vec::new();
    match prov.get("notes") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(s) => notes.push(s.to_string()),
                    None => warnings.push(format!(
                        "provenance.notes[{i}]: expected string, found {}; element skipped",
                        json_type_name(item)
                    )),
                }
            }
        }
        Some(other) => warnings.push(format!(
            "provenance.notes: present but wrong-typed (expected array, found {}); \
             treated as empty",
            json_type_name(other)
        )),
    }
    ProvenanceView { channels, notes }
}

fn build_domain_problems(
    channel: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Vec<DomainProblem> {
    match parse_domain_problems(channel.get("domain_problems")) {
        Ok(None) => Vec::new(),
        Ok(Some(problems)) => problems,
        Err(detail) => {
            warnings.push(format!(
                "{ctx}.domain_problems: {detail}; problem-backed typed views unavailable"
            ));
            Vec::new()
        }
    }
}

/// Parse the recorder's coverage-integrity channel as one atomic value. One
/// malformed element invalidates the whole array because a skipped element
/// could name any problem-backed domain.
fn parse_domain_problems(value: Option<&Value>) -> Result<Option<Vec<DomainProblem>>, String> {
    let items = match value {
        None => return Ok(None),
        Some(Value::Array(items)) => items,
        Some(Value::Null) => return Err("present but null".to_string()),
        Some(other) => {
            return Err(format!(
                "present but wrong-typed (expected array, found {})",
                json_type_name(other)
            ));
        }
    };

    let mut problems = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let entry = item.as_object().ok_or_else(|| {
            format!(
                "element [{i}] is wrong-typed (expected object, found {})",
                json_type_name(item)
            )
        })?;
        let domain_value = entry
            .get("domain_id")
            .ok_or_else(|| format!("element [{i}].domain_id is absent"))?;
        let domain = domain_value.as_i64().ok_or_else(|| {
            format!(
                "element [{i}].domain_id is not an integer (found {})",
                json_type_name(domain_value)
            )
        })?;
        if !(0..=i64::from(u16::MAX)).contains(&domain) {
            return Err(format!(
                "element [{i}].domain_id {domain} is outside the recorder u16 range"
            ));
        }
        let problem_value = entry
            .get("problem")
            .ok_or_else(|| format!("element [{i}].problem is absent"))?;
        let problem = problem_value.as_str().ok_or_else(|| {
            format!(
                "element [{i}].problem is not a string (found {})",
                json_type_name(problem_value)
            )
        })?;
        if problem.trim().is_empty() {
            return Err(format!("element [{i}].problem is blank"));
        }
        problems.push(DomainProblem {
            domain,
            problem: problem.to_string(),
        });
    }
    Ok(Some(problems))
}

#[derive(Clone, Copy)]
enum ProblemBackedView {
    Production,
    DivisionUnits,
    Fleets,
    Focus,
    Leaders,
    Diplomacy,
}

const DOMAIN_PROBLEM_VIEW_MAP: &[(i64, ProblemBackedView)] = &[
    (11, ProblemBackedView::Production),
    (26, ProblemBackedView::DivisionUnits),
    (28, ProblemBackedView::Fleets),
    (45, ProblemBackedView::Focus),
    (47, ProblemBackedView::Leaders),
    (53, ProblemBackedView::Diplomacy),
];

fn reconcile_domain_problem_statuses(
    rest: &Map<String, Value>,
    production_status: &mut ViewStatus,
    division_units_status: &mut ViewStatus,
    fleets_status: &mut ViewStatus,
    focus_status: &mut ViewStatus,
    leaders_status: &mut ViewStatus,
    diplomacy_status: &mut ViewStatus,
) {
    // This enforcing consumer deliberately re-derives coverage from raw JSON;
    // it does not trust build_provenance's presentation-oriented problem list.
    let problems = match rederive_collector_domain_problems(rest) {
        Ok(None) => return,
        Ok(Some(problems)) => problems,
        Err(()) => {
            for status in [
                production_status,
                division_units_status,
                fleets_status,
                focus_status,
                leaders_status,
                diplomacy_status,
            ] {
                downgrade_present_status(status);
            }
            return;
        }
    };
    for problem in &problems {
        for (_, view) in DOMAIN_PROBLEM_VIEW_MAP
            .iter()
            .filter(|(domain, _)| *domain == problem.domain)
        {
            match view {
                ProblemBackedView::Production => downgrade_present_status(production_status),
                ProblemBackedView::DivisionUnits => downgrade_present_status(division_units_status),
                ProblemBackedView::Fleets => downgrade_present_status(fleets_status),
                ProblemBackedView::Focus => downgrade_present_status(focus_status),
                ProblemBackedView::Leaders => downgrade_present_status(leaders_status),
                ProblemBackedView::Diplomacy => downgrade_present_status(diplomacy_status),
            }
        }
    }
}

fn rederive_collector_domain_problems(
    rest: &Map<String, Value>,
) -> Result<Option<Vec<DomainProblem>>, ()> {
    let provenance = match rest.get("provenance") {
        None => return Ok(None),
        Some(Value::Object(provenance)) => provenance,
        Some(_) => return Err(()),
    };
    let collector = match provenance.get("collector") {
        None => return Ok(None),
        Some(Value::Object(collector)) => collector,
        Some(_) => return Err(()),
    };
    parse_domain_problems(collector.get("domain_problems")).map_err(|_| ())
}

fn downgrade_present_status(status: &mut ViewStatus) {
    if *status == ViewStatus::Present {
        *status = ViewStatus::DomainUnavailable;
    }
}

// ===========================================================================
// fx-streams lane: the 12 UI-inventory data streams (UI_COVERAGE_MATRIX §9).
//
// Additive typed views for the deep-panel data the UI wave gates against.
// Each mirrors the no-coercion trichotomy of the existing views and is sourced
// from the REPLAY_SCHEMA.md field names the extractor emits (see
// REPLAY_DATA_REQUIREMENTS.md §14). The country-scoped views are built through
// the shared `country_map` pass; state/world-scoped views have bespoke passes.
// ===========================================================================

// --- 1. research state (REPLAY_SCHEMA §3.4) ---
#[derive(Debug, Clone, PartialEq)]
pub struct ResearchProgressRow {
    pub name: Leaf<String>,
    pub progress_pct: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResearchTechnologyRow {
    pub name: Leaf<String>,
    pub level: Leaf<i64>,
    pub date: Leaf<String>,
    pub research_points: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DoctrineCompletedRow {
    pub id: Leaf<String>,
    pub level: Leaf<i64>,
    pub date: Leaf<String>,
    pub research_points: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DoctrineProgressRow {
    pub id: Leaf<String>,
    pub progress: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DoctrineView {
    pub completed: Leaf<Vec<DoctrineCompletedRow>>,
    pub completed_count: Leaf<i64>,
    pub in_progress: Leaf<Vec<DoctrineProgressRow>>,
    pub source: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResearchView {
    pub tech_count: Leaf<i64>,
    pub research_slots: Leaf<i64>,
    pub in_progress: Leaf<Vec<ResearchProgressRow>>,
    pub technologies: Leaf<Vec<ResearchTechnologyRow>>,
    pub doctrine: Leaf<DoctrineView>,
}

impl ResearchView {
    fn all_unobtained() -> ResearchView {
        ResearchView {
            tech_count: Leaf::Unobtained,
            research_slots: Leaf::Unobtained,
            in_progress: Leaf::Unobtained,
            technologies: Leaf::Unobtained,
            doctrine: Leaf::Unobtained,
        }
    }
}

// --- 2. national-focus state (REPLAY_SCHEMA §3.5) ---
#[derive(Debug, Clone, PartialEq)]
pub struct FocusView {
    pub focus_tree: Leaf<String>,
    pub current: Leaf<String>,
    pub completed_count: Leaf<i64>,
    pub progress: Leaf<f64>,
    pub completed: Leaf<Vec<String>>,
}

impl FocusView {
    fn all_unobtained() -> FocusView {
        FocusView {
            focus_tree: Leaf::Unobtained,
            current: Leaf::Unobtained,
            completed_count: Leaf::Unobtained,
            progress: Leaf::Unobtained,
            completed: Leaf::Unobtained,
        }
    }
}

// --- 3. politics detail (REPLAY_SCHEMA §3.3) + command abilities (abilities) ---
#[derive(Debug, Clone, PartialEq)]
pub struct PoliticsDetailView {
    pub party_popularities: Leaf<BTreeMap<String, f64>>,
    pub ideas: Leaf<Vec<String>>,
    pub national_spirits: Leaf<Vec<String>>,
    pub law_economy: Leaf<String>,
    pub law_trade: Leaf<String>,
    pub law_conscription: Leaf<String>,
    pub appointed_advisors: Leaf<Vec<String>>,
    /// Active command abilities (ability tokens) from `countries.<tag>.abilities`.
    pub command_abilities: Leaf<Vec<String>>,
}

impl PoliticsDetailView {
    fn all_unobtained() -> PoliticsDetailView {
        PoliticsDetailView {
            party_popularities: Leaf::Unobtained,
            ideas: Leaf::Unobtained,
            national_spirits: Leaf::Unobtained,
            law_economy: Leaf::Unobtained,
            law_trade: Leaf::Unobtained,
            law_conscription: Leaf::Unobtained,
            appointed_advisors: Leaf::Unobtained,
            command_abilities: Leaf::Unobtained,
        }
    }
}

// --- 4. production queue + stockpile (REPLAY_SCHEMA §3.6 / §3.2) ---
#[derive(Debug, Clone, PartialEq)]
pub struct ProductionLineRow {
    pub id: Leaf<i64>,
    pub active_factories: Leaf<i64>,
    pub produced: Leaf<f64>,
    pub cost: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentStock {
    pub archetype: Leaf<String>,
    pub amount: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductionView {
    pub stockpile_total: Leaf<f64>,
    pub stockpile: Leaf<BTreeMap<String, f64>>,
    pub construction_queue: Leaf<Vec<ProductionLineRow>>,
    pub equipment_stockpile: Leaf<Vec<EquipmentStock>>,
}

impl ProductionView {
    fn all_unobtained() -> ProductionView {
        ProductionView {
            stockpile_total: Leaf::Unobtained,
            stockpile: Leaf::Unobtained,
            construction_queue: Leaf::Unobtained,
            equipment_stockpile: Leaf::Unobtained,
        }
    }
}

// --- 5. intelligence / operatives (REPLAY_SCHEMA §3.9) ---
#[derive(Debug, Clone, PartialEq)]
pub struct OperativeRow {
    pub op_id: Leaf<i64>,
    pub name: Leaf<String>,
    pub state: Leaf<String>,
    pub captured: Leaf<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationRow {
    pub op_id: Leaf<i64>,
    pub op_type: Leaf<String>,
    pub target: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntelView {
    pub agency_name: Leaf<String>,
    pub agency_created: Leaf<bool>,
    pub agency_upgrade_progress: Leaf<f64>,
    pub operatives: Leaf<Vec<OperativeRow>>,
    pub operations_running: Leaf<Vec<OperationRow>>,
}

impl IntelView {
    fn all_unobtained() -> IntelView {
        IntelView {
            agency_name: Leaf::Unobtained,
            agency_created: Leaf::Unobtained,
            agency_upgrade_progress: Leaf::Unobtained,
            operatives: Leaf::Unobtained,
            operations_running: Leaf::Unobtained,
        }
    }
}

// --- 6. trade state (REPLAY_SCHEMA §3.1 resources + §3.8 lend_lease) ---
#[derive(Debug, Clone, PartialEq)]
pub struct TradeContractRow {
    pub counterpart: Leaf<String>,
    pub direction: Leaf<String>,
    pub convoys_assigned: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TradeView {
    pub produced: Leaf<BTreeMap<String, f64>>,
    pub exported: Leaf<BTreeMap<String, f64>>,
    pub to_export: Leaf<BTreeMap<String, f64>>,
    pub contracts: Leaf<Vec<TradeContractRow>>,
}

impl TradeView {
    fn all_unobtained() -> TradeView {
        TradeView {
            produced: Leaf::Unobtained,
            exported: Leaf::Unobtained,
            to_export: Leaf::Unobtained,
            contracts: Leaf::Unobtained,
        }
    }
}

// --- 7. occupation resistance/compliance (REPLAY_SCHEMA §4, state-scoped) ---
#[derive(Debug, Clone, PartialEq)]
pub struct OccupationRow {
    pub resistance: Leaf<f64>,
    pub resistance_speed: Leaf<f64>,
    pub compliance: Leaf<f64>,
    pub compliance_speed: Leaf<f64>,
    pub occupied_country_tag: Leaf<String>,
    pub operational_status: Leaf<String>,
    pub occupation_law: Leaf<String>,
}

// --- 8. leader roster + traits (REPLAY_SCHEMA §3.10) ---
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderRow {
    pub name: Leaf<String>,
    pub token: Leaf<String>,
    pub level: Leaf<i64>,
    pub traits: Leaf<Vec<String>>,
    /// Formation the leader commands (assignment.name); `Unobtained` = reserve.
    pub assignment: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LeaderRosterView {
    pub country_leader_name: Leaf<String>,
    pub field_marshals: Leaf<Vec<LeaderRow>>,
    pub generals: Leaf<Vec<LeaderRow>>,
    pub admirals: Leaf<Vec<LeaderRow>>,
    pub roster_size: Leaf<i64>,
}

impl LeaderRosterView {
    fn all_unobtained() -> LeaderRosterView {
        LeaderRosterView {
            country_leader_name: Leaf::Unobtained,
            field_marshals: Leaf::Unobtained,
            generals: Leaf::Unobtained,
            admirals: Leaf::Unobtained,
            roster_size: Leaf::Unobtained,
        }
    }
}

// --- 9. power-balance state (REPLAY_SCHEMA §18, world-scoped) ---
#[derive(Debug, Clone, PartialEq)]
pub struct PowerBalanceRow {
    pub template: Leaf<String>,
    pub value: Leaf<f64>,
    pub left_side: Leaf<String>,
    pub right_side: Leaf<String>,
    pub trending_side: Leaf<String>,
    pub countries: Leaf<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PowerBalanceView {
    pub status: ViewStatus,
    pub balances: Vec<PowerBalanceRow>,
}

impl PowerBalanceView {
    fn with_status(status: ViewStatus) -> PowerBalanceView {
        PowerBalanceView {
            status,
            balances: Vec::new(),
        }
    }
}

// --- 10. templates + equipment variants (REPLAY_SCHEMA §11, world-scoped) ---
#[derive(Debug, Clone, PartialEq)]
pub struct DivisionTemplateRow {
    pub template_uid: String,
    pub country: Leaf<String>,
    pub name: Leaf<String>,
    pub role: Leaf<String>,
    pub n_regiments: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentVariantRow {
    pub archetype: Leaf<String>,
    pub family: Leaf<String>,
    pub name: Leaf<String>,
    pub obsolete: Leaf<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DesignsView {
    pub status: ViewStatus,
    pub division_templates: Vec<DivisionTemplateRow>,
    pub equipment_variants: BTreeMap<String, EquipmentVariantRow>,
}

impl DesignsView {
    fn with_status(status: ViewStatus) -> DesignsView {
        DesignsView {
            status,
            division_templates: Vec::new(),
            equipment_variants: BTreeMap::new(),
        }
    }
}

// --- 11. naval/air combat detail + losses (REPLAY_SCHEMA §7/§17) ---
#[derive(Debug, Clone, PartialEq)]
pub struct NavalCombatRow {
    pub combat_id: Leaf<i64>,
    pub location_province: Leaf<u32>,
    pub attacker: Leaf<String>,
    pub defender: Leaf<String>,
    pub result_type: Leaf<String>,
    pub progress: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConvoyLossRow {
    pub killer: Leaf<String>,
    pub owner: Leaf<String>,
    pub convoys: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavalAirCombatView {
    pub status: ViewStatus,
    pub naval_combats: Vec<NavalCombatRow>,
    pub sunk_convoys: Leaf<Vec<ConvoyLossRow>>,
}

// --- 12. scripted-GUI variable state (REPLAY_SCHEMA §3 script_variables / §18) ---
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptedGuiView {
    pub status: ViewStatus,
    /// Per-country dynamic script variables (`countries.<tag>.script_variables`).
    pub by_country: BTreeMap<String, Leaf<BTreeMap<String, f64>>>,
    pub world_variables: Leaf<BTreeMap<String, f64>>,
    /// Active global flag names (`world.flags` keys), sorted.
    pub world_flags: Leaf<Vec<String>>,
}

// --- small: strategic raid state (REPLAY_SCHEMA §16) ---
#[derive(Debug, Clone, PartialEq)]
pub struct RaidStateView {
    pub status: ViewStatus,
    pub priorities: BTreeMap<String, i64>,
    pub next_target: Leaf<i64>,
}

impl RaidStateView {
    fn with_status(status: ViewStatus) -> RaidStateView {
        RaidStateView {
            status,
            priorities: BTreeMap::new(),
            next_target: match status {
                ViewStatus::DomainAbsent => Leaf::Absent,
                _ => Leaf::Unobtained,
            },
        }
    }
}

impl WorldFrame {
    pub fn research_by_country(&self) -> &BTreeMap<String, ResearchView> {
        &self.research_by_country
    }
    pub fn research(&self, tag: &str) -> Option<&ResearchView> {
        self.research_by_country.get(tag)
    }
    pub fn research_status(&self) -> ViewStatus {
        self.research_status
    }
    pub fn focus_by_country(&self) -> &BTreeMap<String, FocusView> {
        &self.focus_by_country
    }
    pub fn focus(&self, tag: &str) -> Option<&FocusView> {
        self.focus_by_country.get(tag)
    }
    pub fn focus_status(&self) -> ViewStatus {
        self.focus_status
    }
    pub fn politics_detail_by_country(&self) -> &BTreeMap<String, PoliticsDetailView> {
        &self.politics_detail_by_country
    }
    pub fn politics_detail(&self, tag: &str) -> Option<&PoliticsDetailView> {
        self.politics_detail_by_country.get(tag)
    }
    pub fn politics_detail_status(&self) -> ViewStatus {
        self.politics_detail_status
    }
    pub fn production_by_country(&self) -> &BTreeMap<String, ProductionView> {
        &self.production_by_country
    }
    pub fn production(&self, tag: &str) -> Option<&ProductionView> {
        self.production_by_country.get(tag)
    }
    pub fn production_status(&self) -> ViewStatus {
        self.production_status
    }
    pub fn intel_by_country(&self) -> &BTreeMap<String, IntelView> {
        &self.intel_by_country
    }
    pub fn intel(&self, tag: &str) -> Option<&IntelView> {
        self.intel_by_country.get(tag)
    }
    pub fn intel_status(&self) -> ViewStatus {
        self.intel_status
    }
    pub fn trade_by_country(&self) -> &BTreeMap<String, TradeView> {
        &self.trade_by_country
    }
    pub fn trade(&self, tag: &str) -> Option<&TradeView> {
        self.trade_by_country.get(tag)
    }
    pub fn trade_status(&self) -> ViewStatus {
        self.trade_status
    }
    pub fn leaders_by_country(&self) -> &BTreeMap<String, LeaderRosterView> {
        &self.leaders_by_country
    }
    pub fn leaders(&self, tag: &str) -> Option<&LeaderRosterView> {
        self.leaders_by_country.get(tag)
    }
    pub fn leaders_status(&self) -> ViewStatus {
        self.leaders_status
    }
    pub fn occupation_by_state(&self) -> &BTreeMap<u32, OccupationRow> {
        &self.occupation_by_state
    }
    pub fn occupation(&self, state_id: u32) -> Option<&OccupationRow> {
        self.occupation_by_state.get(&state_id)
    }
    pub fn occupation_status(&self) -> ViewStatus {
        self.occupation_status
    }
    pub fn power_balance(&self) -> &PowerBalanceView {
        &self.power_balance
    }
    pub fn power_balance_status(&self) -> ViewStatus {
        self.power_balance.status
    }
    pub fn designs(&self) -> &DesignsView {
        &self.designs
    }
    pub fn designs_status(&self) -> ViewStatus {
        self.designs.status
    }
    pub fn naval_air_combat(&self) -> &NavalAirCombatView {
        &self.naval_air_combat
    }
    pub fn naval_air_combat_status(&self) -> ViewStatus {
        self.naval_air_combat.status
    }
    pub fn scripted_gui(&self) -> &ScriptedGuiView {
        &self.scripted_gui
    }
    pub fn scripted_gui_status(&self) -> ViewStatus {
        self.scripted_gui.status
    }
    pub fn raids(&self) -> &RaidStateView {
        &self.raids
    }
    pub fn raids_status(&self) -> ViewStatus {
        self.raids.status
    }
}

// ---------------------------------------------------------------------------
// fx-streams shared helpers
// ---------------------------------------------------------------------------

/// Iterate `countries` once, building one typed view per TAG. Mirrors the
/// domain handling of `build_topbar_scalars`: a present-but-null / wrong-typed
/// country entry yields the caller's `unobtained()` value (never dropped).
fn country_map<T>(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
    unobtained: fn() -> T,
    build: impl Fn(&Map<String, Value>, &str, &mut Vec<String>) -> T,
) -> (BTreeMap<String, T>, ViewStatus) {
    let entries = match rest.get("countries") {
        Some(Value::Object(entries)) => entries,
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        None => return (BTreeMap::new(), ViewStatus::DomainAbsent),
        Some(other) => {
            warnings.push(format!(
                "countries: present but wrong-typed (expected object, found {}); \
                 country view unavailable",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
    };
    let mut out = BTreeMap::new();
    for (tag, entry) in entries {
        match entry {
            Value::Object(obj) => {
                let ctx = format!("countries.{tag}");
                out.insert(tag.clone(), build(obj, &ctx, warnings));
            }
            Value::Null => {
                out.insert(tag.clone(), unobtained());
            }
            other => {
                warnings.push(format!(
                    "countries.{tag}: expected object, found {}; leaves treated as unobtained",
                    json_type_name(other)
                ));
                out.insert(tag.clone(), unobtained());
            }
        }
    }
    (out, ViewStatus::Present)
}

/// `{key: number}` object at `path` -> typed f64 map (sorted). One non-number
/// value poisons the whole leaf (fail-closed, with warning).
fn leaf_f64_map(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<BTreeMap<String, f64>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Object(map)) => {
            let mut out = BTreeMap::new();
            for (k, v) in map {
                match v.as_f64() {
                    Some(n) => {
                        out.insert(k.clone(), n);
                    }
                    None => {
                        warnings.push(format!(
                            "{ctx}.{}.{k}: expected number, found {}; whole map treated as unobtained",
                            path.join("."),
                            json_type_name(v)
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(out)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: present but wrong-typed (expected object, found {}); \
                 treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

/// Object keys at `path` -> sorted string list (e.g. `world.flags` names).
fn object_keys(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<String>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Object(map)) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            Leaf::Measured(keys)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: present but wrong-typed (expected object, found {}); \
                 treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

/// Array-of-objects at `path` -> `Leaf<Vec<T>>` via a per-row builder. A
/// non-object element poisons the whole leaf (fail-closed, with warning).
fn leaf_rows<T>(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
    mut row: impl FnMut(&Map<String, Value>, &str, &mut Vec<String>) -> T,
) -> Leaf<Vec<T>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item.as_object() {
                    Some(o) => {
                        let rctx = format!("{ctx}.{}[{i}]", path.join("."));
                        out.push(row(o, &rctx, warnings));
                    }
                    None => {
                        warnings.push(format!(
                            "{ctx}.{}[{i}]: expected object, found {}; whole list treated as unobtained",
                            path.join("."),
                            json_type_name(item)
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(out)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

/// Extract a single string `field` from each object in the array at `path` ->
/// `Leaf<Vec<String>>` (e.g. `abilities[].ability`).
fn leaf_obj_field_list(
    obj: &Map<String, Value>,
    path: &[&str],
    field: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<String>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item
                    .as_object()
                    .and_then(|o| o.get(field))
                    .and_then(Value::as_str)
                {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        warnings.push(format!(
                            "{ctx}.{}[{i}].{field}: expected string, found non-string/absent; \
                             whole list treated as unobtained",
                            path.join(".")
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(out)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

// ---------------------------------------------------------------------------
// fx-streams builders
// ---------------------------------------------------------------------------

fn build_research(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, ResearchView>, ViewStatus) {
    country_map(
        rest,
        warnings,
        ResearchView::all_unobtained,
        |obj, ctx, w| ResearchView {
            tech_count: leaf_i64(obj, &["research", "tech_count"], ctx, w),
            research_slots: leaf_i64(obj, &["research", "research_slots"], ctx, w),
            in_progress: leaf_rows(obj, &["research", "in_progress"], ctx, w, |o, c, w| {
                ResearchProgressRow {
                    name: leaf_string(o, &["name"], c, w),
                    progress_pct: leaf_f64(o, &["progress_pct"], c, w),
                }
            }),
            technologies: leaf_rows(obj, &["research", "technologies"], ctx, w, |o, c, w| {
                ResearchTechnologyRow {
                    name: leaf_string(o, &["name"], c, w),
                    level: leaf_i64(o, &["level"], c, w),
                    date: leaf_string(o, &["date"], c, w),
                    research_points: leaf_f64(o, &["research_points"], c, w),
                }
            }),
            doctrine: leaf_doctrine(obj, ctx, w),
        },
    )
}

fn leaf_doctrine(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<DoctrineView> {
    match walk_path(obj, &["research", "doctrine"], ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Object(doctrine)) => {
            let dctx = format!("{ctx}.research.doctrine");
            Leaf::Measured(DoctrineView {
                completed: leaf_rows(doctrine, &["completed"], &dctx, warnings, |o, c, w| {
                    DoctrineCompletedRow {
                        id: leaf_string(o, &["id"], c, w),
                        level: leaf_i64(o, &["level"], c, w),
                        date: leaf_string(o, &["date"], c, w),
                        research_points: leaf_f64(o, &["research_points"], c, w),
                    }
                }),
                completed_count: leaf_i64(doctrine, &["completed_count"], &dctx, warnings),
                in_progress: leaf_rows(doctrine, &["in_progress"], &dctx, warnings, |o, c, w| {
                    DoctrineProgressRow {
                        id: leaf_string(o, &["id"], c, w),
                        progress: leaf_f64(o, &["progress"], c, w),
                    }
                }),
                source: leaf_string(doctrine, &["source"], &dctx, warnings),
            })
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.research.doctrine: present but wrong-typed (expected object, found {}); treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn build_focus(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, FocusView>, ViewStatus) {
    country_map(rest, warnings, FocusView::all_unobtained, |obj, ctx, w| {
        FocusView {
            focus_tree: leaf_string(obj, &["focus", "focus_tree"], ctx, w),
            current: leaf_string(obj, &["focus", "current"], ctx, w),
            completed_count: leaf_i64(obj, &["focus", "completed_count"], ctx, w),
            progress: leaf_f64(obj, &["focus", "progress"], ctx, w),
            completed: leaf_string_list(obj, &["focus", "completed"], ctx, w),
        }
    })
}

fn build_politics_detail(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, PoliticsDetailView>, ViewStatus) {
    country_map(
        rest,
        warnings,
        PoliticsDetailView::all_unobtained,
        |obj, ctx, w| PoliticsDetailView {
            party_popularities: leaf_f64_map(obj, &["politics", "party_popularities"], ctx, w),
            ideas: leaf_string_list(obj, &["politics", "ideas"], ctx, w),
            national_spirits: leaf_string_list(obj, &["politics", "national_spirits"], ctx, w),
            law_economy: leaf_string(obj, &["politics", "laws", "economy"], ctx, w),
            law_trade: leaf_string(obj, &["politics", "laws", "trade"], ctx, w),
            law_conscription: leaf_string(obj, &["politics", "laws", "conscription"], ctx, w),
            appointed_advisors: leaf_string_list(obj, &["politics", "appointed_advisors"], ctx, w),
            command_abilities: leaf_obj_field_list(obj, &["abilities"], "ability", ctx, w),
        },
    )
}

fn build_production(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, ProductionView>, ViewStatus) {
    country_map(
        rest,
        warnings,
        ProductionView::all_unobtained,
        |obj, ctx, w| ProductionView {
            stockpile_total: leaf_f64(obj, &["production", "stockpile_total"], ctx, w),
            stockpile: leaf_f64_map(obj, &["production", "stockpile"], ctx, w),
            construction_queue: leaf_rows(
                obj,
                &["production", "construction_queue"],
                ctx,
                w,
                |o, c, w| ProductionLineRow {
                    id: leaf_i64(o, &["id"], c, w),
                    active_factories: leaf_i64(o, &["active_factories"], c, w),
                    produced: leaf_f64(o, &["produced"], c, w),
                    cost: leaf_f64(o, &["cost"], c, w),
                },
            ),
            equipment_stockpile: leaf_rows(
                obj,
                &["military", "equipment_stockpile"],
                ctx,
                w,
                |o, c, w| EquipmentStock {
                    archetype: leaf_string(o, &["archetype"], c, w),
                    amount: leaf_f64(o, &["amount"], c, w),
                },
            ),
        },
    )
}

fn build_intel(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, IntelView>, ViewStatus) {
    country_map(rest, warnings, IntelView::all_unobtained, |obj, ctx, w| {
        // `intel.operations` is an object in the deepened schema, but legacy
        // base rows park it as an empty array; only read `.running` when it is
        // genuinely an object (an array/absent yields Absent, not a warning).
        let operations_running = match obj
            .get("intel")
            .and_then(Value::as_object)
            .and_then(|i| i.get("operations"))
        {
            Some(Value::Object(_)) => leaf_rows(
                obj,
                &["intel", "operations", "running"],
                ctx,
                w,
                |o, c, w| OperationRow {
                    op_id: leaf_i64(o, &["op_id"], c, w),
                    op_type: leaf_string(o, &["type"], c, w),
                    target: leaf_string(o, &["target"], c, w),
                },
            ),
            Some(Value::Null) => Leaf::Unobtained,
            _ => Leaf::Absent,
        };
        IntelView {
            agency_name: leaf_string(obj, &["intel", "agency", "name"], ctx, w),
            agency_created: leaf_bool(obj, &["intel", "agency", "is_created"], ctx, w),
            agency_upgrade_progress: leaf_f64(
                obj,
                &["intel", "agency", "upgrade_progress"],
                ctx,
                w,
            ),
            operatives: leaf_rows(obj, &["intel", "operatives"], ctx, w, |o, c, w| {
                OperativeRow {
                    op_id: leaf_i64(o, &["op_id"], c, w),
                    name: leaf_string(o, &["name"], c, w),
                    state: leaf_string(o, &["state"], c, w),
                    captured: leaf_bool(o, &["captured"], c, w),
                }
            }),
            operations_running,
        }
    })
}

fn build_trade(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, TradeView>, ViewStatus) {
    country_map(rest, warnings, TradeView::all_unobtained, |obj, ctx, w| {
        TradeView {
            produced: leaf_f64_map(obj, &["economy", "resources", "produced"], ctx, w),
            exported: leaf_f64_map(obj, &["economy", "resources", "exported"], ctx, w),
            to_export: leaf_f64_map(obj, &["economy", "resources", "to_export"], ctx, w),
            contracts: leaf_rows(obj, &["diplomacy", "lend_lease"], ctx, w, |o, c, w| {
                TradeContractRow {
                    counterpart: leaf_string(o, &["counterpart"], c, w),
                    direction: leaf_string(o, &["direction"], c, w),
                    convoys_assigned: leaf_i64(o, &["convoys", "assigned"], c, w),
                }
            }),
        }
    })
}

fn build_leaders(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, LeaderRosterView>, ViewStatus) {
    country_map(
        rest,
        warnings,
        LeaderRosterView::all_unobtained,
        |obj, ctx, w| {
            let leader_row = |o: &Map<String, Value>, c: &str, w: &mut Vec<String>| LeaderRow {
                name: leaf_string(o, &["name"], c, w),
                token: leaf_string(o, &["token"], c, w),
                level: leaf_i64(o, &["level"], c, w),
                traits: leaf_string_list(o, &["traits"], c, w),
                assignment: leaf_string(o, &["assignment", "name"], c, w),
            };
            LeaderRosterView {
                country_leader_name: leaf_string(obj, &["commanders", "leader", "name"], ctx, w),
                field_marshals: leaf_rows(
                    obj,
                    &["commanders", "field_marshals"],
                    ctx,
                    w,
                    leader_row,
                ),
                generals: leaf_rows(obj, &["commanders", "generals"], ctx, w, leader_row),
                admirals: leaf_rows(obj, &["commanders", "admirals"], ctx, w, leader_row),
                roster_size: leaf_i64(obj, &["commanders", "counts", "roster_size"], ctx, w),
            }
        },
    )
}

fn build_occupation(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<u32, OccupationRow>, ViewStatus) {
    let states = match rest.get("states") {
        Some(Value::Object(states)) => states,
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        None => return (BTreeMap::new(), ViewStatus::DomainAbsent),
        Some(other) => {
            warnings.push(format!(
                "states: present but wrong-typed (expected object, found {}); \
                 occupation view unavailable",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
    };
    let mut out = BTreeMap::new();
    let mut status = ViewStatus::DomainAbsent;
    for (key, value) in states {
        let id = match key.parse::<u32>() {
            Ok(id) => id,
            Err(_) => continue,
        };
        let Some(obj) = value.as_object() else {
            continue;
        };
        match obj.get("occupation") {
            Some(Value::Object(occ)) => {
                let ctx = format!("states.{key}.occupation");
                out.insert(
                    id,
                    OccupationRow {
                        resistance: leaf_f64(occ, &["resistance"], &ctx, warnings),
                        resistance_speed: leaf_f64(occ, &["resistance_speed"], &ctx, warnings),
                        compliance: leaf_f64(occ, &["compliance"], &ctx, warnings),
                        compliance_speed: leaf_f64(occ, &["compliance_speed"], &ctx, warnings),
                        occupied_country_tag: leaf_string(
                            occ,
                            &["occupied_country_tag"],
                            &ctx,
                            warnings,
                        ),
                        operational_status: leaf_string(
                            occ,
                            &["operational_status"],
                            &ctx,
                            warnings,
                        ),
                        occupation_law: leaf_string(
                            obj,
                            &["occupation_law"],
                            &format!("states.{key}"),
                            warnings,
                        ),
                    },
                );
                status = ViewStatus::Present;
            }
            // Absent or explicit-null occupation: no active occupation on this
            // state; skip the row rather than flood the map with unobtained rows.
            Some(Value::Null) | None => {}
            Some(other) => warnings.push(format!(
                "states.{key}.occupation: expected object, found {}; row skipped",
                json_type_name(other)
            )),
        }
    }
    (out, status)
}

fn build_power_balance(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> PowerBalanceView {
    let (items, status) = array_at_path(
        rest,
        &["world", "power_balances"],
        "world.power_balances",
        warnings,
    );
    let Some(items) = items else {
        return PowerBalanceView::with_status(status);
    };
    let mut balances = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(o) = item.as_object() else {
            warnings.push(format!(
                "world.power_balances[{i}]: expected object, found {}; row skipped",
                json_type_name(item)
            ));
            continue;
        };
        let ctx = format!("world.power_balances[{i}]");
        balances.push(PowerBalanceRow {
            template: leaf_string(o, &["template"], &ctx, warnings),
            value: leaf_f64(o, &["value"], &ctx, warnings),
            left_side: leaf_string(o, &["left_side"], &ctx, warnings),
            right_side: leaf_string(o, &["right_side"], &ctx, warnings),
            trending_side: leaf_string(o, &["trending_side"], &ctx, warnings),
            countries: leaf_string_list(o, &["countries"], &ctx, warnings),
        });
    }
    PowerBalanceView {
        status: ViewStatus::Present,
        balances,
    }
}

fn build_designs(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> DesignsView {
    let (designs, status) = object_at_path(rest, &["designs"], "designs", warnings);
    let Some(designs) = designs else {
        return DesignsView::with_status(status);
    };
    let mut division_templates = Vec::new();
    match designs.get("division_templates") {
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                let Some(o) = item.as_object() else {
                    warnings.push(format!(
                        "designs.division_templates[{i}]: expected object, found {}; row skipped",
                        json_type_name(item)
                    ));
                    continue;
                };
                let ctx = format!("designs.division_templates[{i}]");
                let template_uid = o
                    .get("id")
                    .and_then(value_to_id_string)
                    .unwrap_or_else(|| format!("template_{i}"));
                division_templates.push(DivisionTemplateRow {
                    template_uid,
                    country: leaf_string(o, &["country"], &ctx, warnings),
                    name: leaf_string(o, &["name"], &ctx, warnings),
                    role: leaf_string(o, &["role"], &ctx, warnings),
                    n_regiments: leaf_i64(o, &["n_regiments"], &ctx, warnings),
                });
            }
        }
        Some(Value::Null) | None => {}
        Some(other) => warnings.push(format!(
            "designs.division_templates: expected array, found {}; ignored",
            json_type_name(other)
        )),
    }
    let mut equipment_variants = BTreeMap::new();
    match designs.get("equipment_variants") {
        Some(Value::Object(map)) => {
            for (key, value) in map {
                let Some(o) = value.as_object() else {
                    warnings.push(format!(
                        "designs.equipment_variants.{key}: expected object, found {}; row skipped",
                        json_type_name(value)
                    ));
                    continue;
                };
                let ctx = format!("designs.equipment_variants.{key}");
                equipment_variants.insert(
                    key.clone(),
                    EquipmentVariantRow {
                        archetype: leaf_string(o, &["archetype"], &ctx, warnings),
                        family: leaf_string(o, &["family"], &ctx, warnings),
                        name: leaf_string(o, &["name"], &ctx, warnings),
                        obsolete: leaf_bool(o, &["obsolete"], &ctx, warnings),
                    },
                );
            }
        }
        Some(Value::Null) | None => {}
        Some(other) => warnings.push(format!(
            "designs.equipment_variants: expected object, found {}; ignored",
            json_type_name(other)
        )),
    }
    DesignsView {
        status: ViewStatus::Present,
        division_templates,
        equipment_variants,
    }
}

fn build_naval_air_combat(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> NavalAirCombatView {
    let (items, combat_status) = array_at_path(rest, &["naval_combats"], "naval_combats", warnings);
    let mut naval_combats = Vec::new();
    if let Some(items) = items {
        for (i, item) in items.iter().enumerate() {
            let Some(o) = item.as_object() else {
                warnings.push(format!(
                    "naval_combats[{i}]: expected object, found {}; row skipped",
                    json_type_name(item)
                ));
                continue;
            };
            let ctx = format!("naval_combats[{i}]");
            naval_combats.push(NavalCombatRow {
                combat_id: leaf_i64(o, &["naval_combat_id"], &ctx, warnings),
                location_province: leaf_u32(o, &["location_province"], &ctx, warnings),
                attacker: leaf_string(o, &["attacker"], &ctx, warnings),
                defender: leaf_string(o, &["defender"], &ctx, warnings),
                result_type: leaf_string(o, &["result_type"], &ctx, warnings),
                progress: leaf_f64(o, &["progress"], &ctx, warnings),
            });
        }
    }
    let sunk_convoys = leaf_rows(
        rest,
        &["world_events", "sunk_convoys"],
        "snapshot",
        warnings,
        |o, c, w| ConvoyLossRow {
            killer: leaf_string(o, &["killer_country"], c, w),
            owner: leaf_string(o, &["owner"], c, w),
            convoys: leaf_i64(o, &["convoys"], c, w),
        },
    );
    let sunk_status = match &sunk_convoys {
        Leaf::Measured(_) => ViewStatus::Present,
        Leaf::Unobtained => ViewStatus::DomainUnavailable,
        Leaf::Absent => ViewStatus::DomainAbsent,
    };
    NavalAirCombatView {
        status: combine_status(combat_status, sunk_status),
        naval_combats,
        sunk_convoys,
    }
}

fn build_scripted_gui(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> ScriptedGuiView {
    let (by_country, cstatus) = country_map(
        rest,
        warnings,
        || Leaf::Unobtained,
        |obj, ctx, w| leaf_f64_map(obj, &["script_variables"], ctx, w),
    );
    let world_variables = leaf_f64_map(rest, &["world", "variables"], "snapshot", warnings);
    let world_flags = object_keys(rest, &["world", "flags"], "world.flags", warnings);
    let world_status = match (&world_variables, &world_flags) {
        (Leaf::Measured(_), _) | (_, Leaf::Measured(_)) => ViewStatus::Present,
        (Leaf::Unobtained, _) | (_, Leaf::Unobtained) => ViewStatus::DomainUnavailable,
        _ => ViewStatus::DomainAbsent,
    };
    ScriptedGuiView {
        status: combine_status(cstatus, world_status),
        by_country,
        world_variables,
        world_flags,
    }
}

fn build_raids(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> RaidStateView {
    let (raids, status) = object_at_path(rest, &["raids"], "raids", warnings);
    let Some(raids) = raids else {
        return RaidStateView::with_status(status);
    };
    let mut priorities = BTreeMap::new();
    if let Some(Value::Array(items)) = raids.get("countries") {
        for item in items {
            if let Some(o) = item.as_object() {
                if let (Some(tag), Some(pri)) = (
                    o.get("tag").and_then(Value::as_str),
                    o.get("priority").and_then(Value::as_i64),
                ) {
                    priorities.insert(tag.to_string(), pri);
                }
            }
        }
    }
    let next_target = match raids.get("counters") {
        Some(Value::Object(c)) => leaf_i64(c, &["next_target"], "raids.counters", warnings),
        Some(Value::Null) => Leaf::Unobtained,
        _ => Leaf::Absent,
    };
    RaidStateView {
        status: ViewStatus::Present,
        priorities,
        next_target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    const RECORD_MAGIC: &[u8; 4] = b"H4CR";
    const RECORD_VERSION: u16 = 1;
    const RECORD_HEADER_LEN: u16 = 32;
    const RECORD_TYPE_FRAME_END: u16 = 2;
    const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
    const DATE_EPOCH_CONST: i32 = 43_799_999;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> TestDir {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "replay_core_{label}_{}_{}",
                std::process::id(),
                unique
            ));
            fs::create_dir_all(&path).expect("create test directory");
            TestDir(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn push_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) {
        out.extend_from_slice(RECORD_MAGIC);
        out.extend_from_slice(&RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&RECORD_HEADER_LEN.to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn frame_end_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn push_compact_string(payload: &mut Vec<u8>, value: &str) {
        payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    }

    fn focus_depth_payload(tag_db_id: u32) -> Vec<u8> {
        const HEADER_LEN: usize = 48;
        const FOCUS_COMPLETED_COUNT_INCREASED: u32 = 1;
        const FOCUS_CURRENT_STARTED: u32 = 2;

        let mut payload = vec![0u8; HEADER_LEN];
        payload[8..12].copy_from_slice(&1u32.to_le_bytes()); // depth_status_flags
        payload[12..16].copy_from_slice(&2u32.to_le_bytes()); // focus_count
        payload[28..32].copy_from_slice(&2u32.to_le_bytes()); // observed_focus_count

        for value in [tag_db_id, FOCUS_COMPLETED_COUNT_INCREASED, 0, 0, 3, 4] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&500i32.to_le_bytes());
        payload.extend_from_slice(&750i32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        push_compact_string(&mut payload, "");
        push_compact_string(&mut payload, "");
        push_compact_string(&mut payload, "focus_done");

        for value in [tag_db_id, FOCUS_CURRENT_STARTED, 0, 0, 4, 4] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&0i32.to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        push_compact_string(&mut payload, "");
        push_compact_string(&mut payload, "focus_current");
        push_compact_string(&mut payload, "");

        payload
    }

    fn country_identity_payload(tag_db_id: u32, tag: &str) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&tag_db_id.to_le_bytes());
        payload.extend_from_slice(&(tag.len() as u16).to_le_bytes());
        payload.extend_from_slice(tag.as_bytes());
        payload.push(1); // cosmetic tag is measured null
        payload
    }

    fn production_payload(declared_country_count: u32, keyed_tag: Option<u32>) -> Vec<u8> {
        let keyed_count = u32::from(keyed_tag.is_some());
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&declared_country_count.to_le_bytes());
        payload.extend_from_slice(&declared_country_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&100_000u32.to_le_bytes());
        payload.extend_from_slice(&keyed_count.to_le_bytes());
        payload.extend_from_slice(&keyed_count.to_le_bytes());
        payload.extend_from_slice(&keyed_count.to_le_bytes());
        payload.extend_from_slice(&keyed_count.to_le_bytes());

        if let Some(tag) = keyed_tag {
            payload.extend_from_slice(&tag.to_le_bytes());
            payload.extend_from_slice(&(10_000 + tag).to_le_bytes());
            payload.extend_from_slice(&100_000i64.to_le_bytes());

            for kind in [1u32, 2u32] {
                payload.extend_from_slice(&tag.to_le_bytes());
                payload.extend_from_slice(&kind.to_le_bytes());
                payload.extend_from_slice(&(tag as i32).to_le_bytes());
                payload.extend_from_slice(&1i32.to_le_bytes());
                payload.extend_from_slice(&0i32.to_le_bytes());
                payload.extend_from_slice(&0i32.to_le_bytes());
                payload.extend_from_slice(&0i32.to_le_bytes());
                payload.extend_from_slice(&0i32.to_le_bytes());
            }

            payload.extend_from_slice(&tag.to_le_bytes());
            payload.extend_from_slice(&(20_000 + tag).to_le_bytes());
            payload.extend_from_slice(&(30_000 + tag).to_le_bytes());
            payload.extend_from_slice(&1i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn raw_frame_from_snapshot(path: &Path) -> RawFrame {
        let mut root = serde_json::from_slice::<Value>(&fs::read(path).expect("read snapshot"))
            .expect("snapshot json")
            .as_object()
            .expect("snapshot root object")
            .clone();
        RawFrame {
            hour_index: root
                .remove("hour_index")
                .and_then(|value| value.as_i64())
                .expect("snapshot hour_index"),
            date: root
                .remove("date")
                .and_then(|value| value.as_str().map(str::to_string)),
            save_version: root
                .remove("save_version")
                .and_then(|value| value.as_str().map(str::to_string)),
            schema_version: root.remove("schema_version"),
            rest: root,
        }
    }

    fn world_frame_from_json(root: Value) -> WorldFrame {
        let encoded = serde_json::to_vec(&root).expect("encode JSON frame");
        let mut root = serde_json::from_slice::<Value>(&encoded)
            .expect("parse JSON frame")
            .as_object()
            .expect("JSON frame root object")
            .clone();
        WorldFrame::from_raw(RawFrame {
            hour_index: root
                .remove("hour_index")
                .and_then(|value| value.as_i64())
                .expect("JSON frame hour_index"),
            date: root
                .remove("date")
                .and_then(|value| value.as_str().map(str::to_string)),
            save_version: root
                .remove("save_version")
                .and_then(|value| value.as_str().map(str::to_string)),
            schema_version: root.remove("schema_version"),
            rest: root,
        })
    }

    fn carried_problem_backed_json(domain_problems: Option<Value>) -> Value {
        let mut collector = serde_json::Map::from_iter([
            ("captured".to_string(), Value::Bool(true)),
            (
                "domains".to_string(),
                serde_json::json!([
                    "production_live",
                    "division_equipment_live",
                    "oob_command_live",
                    "characters_leaders_cabinet_live",
                    "diplomacy_intel_operatives_live"
                ]),
            ),
        ]);
        if let Some(domain_problems) = domain_problems {
            collector.insert("domain_problems".to_string(), domain_problems);
        }
        serde_json::json!({
            "hour_index": 100,
            "countries": {
                "GER": {
                    "production": {
                        "stockpile_total": 120.0,
                        "stockpile": {"1001": 120.0}
                    },
                    "focus": {
                        "current": "carried_focus",
                        "completed_count": 4
                    },
                    "commanders": {
                        "leader": {"name": "Carried Leader"},
                        "counts": {"roster_size": 3}
                    },
                    "politics": {
                        "faction": {"name": "Axis", "is_leader": true}
                    },
                    "diplomacy": {
                        "guarantees": ["ITA"],
                        "relation_flags": {"ITA": ["guarantee"]}
                    }
                }
            },
            "divisions": [{
                "div_id": 7,
                "owner": "GER",
                "equipment": [{
                    "archetype": "infantry_equipment",
                    "equip_id": 1001,
                    "amount": 120.0
                }]
            }],
            "order_of_battle": {
                "fleets": [{
                    "id": 3,
                    "owner": "GER",
                    "task_forces": [{
                        "id": 31,
                        "owner": "GER",
                        "location_province": 42,
                        "ships": []
                    }]
                }]
            },
            "diplomacy_live": {
                "war_relations": [],
                "faction_members": {"1": ["GER", "ITA"]}
            },
            "provenance": {
                "collector": Value::Object(collector)
            }
        })
    }

    #[test]
    fn research_view_keeps_completed_technologies_and_doctrine_subsets_typed() {
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 7,
            "countries": {
                "GER": {
                    "research": {
                        "tech_count": 1,
                        "research_slots": 4,
                        "in_progress": [],
                        "technologies": [{
                            "name": "infantry_weapons",
                            "level": 1,
                            "date": "1936.1.1.0",
                            "research_points": 10.0
                        }],
                        "doctrine": {
                            "completed": [{
                                "id": "superior_firepower",
                                "level": 1,
                                "date": "1936.2.1.0",
                                "research_points": 100.0
                            }],
                            "completed_count": 1,
                            "in_progress": [{"id": "delay", "progress": 0.25}],
                            "source": "S"
                        }
                    }
                }
            }
        }));
        let research = frame.research("GER").expect("typed research");
        assert_eq!(
            research.technologies,
            Leaf::Measured(vec![ResearchTechnologyRow {
                name: Leaf::Measured("infantry_weapons".to_string()),
                level: Leaf::Measured(1),
                date: Leaf::Measured("1936.1.1.0".to_string()),
                research_points: Leaf::Measured(10.0),
            }])
        );
        assert_eq!(
            research.doctrine,
            Leaf::Measured(DoctrineView {
                completed: Leaf::Measured(vec![DoctrineCompletedRow {
                    id: Leaf::Measured("superior_firepower".to_string()),
                    level: Leaf::Measured(1),
                    date: Leaf::Measured("1936.2.1.0".to_string()),
                    research_points: Leaf::Measured(100.0),
                }]),
                completed_count: Leaf::Measured(1),
                in_progress: Leaf::Measured(vec![DoctrineProgressRow {
                    id: Leaf::Measured("delay".to_string()),
                    progress: Leaf::Measured(0.25),
                }]),
                source: Leaf::Measured("S".to_string()),
            })
        );
    }

    fn assert_all_problem_backed_views_unavailable(label: &str, frame: &WorldFrame) {
        assert_eq!(
            frame.production_status(),
            ViewStatus::DomainUnavailable,
            "{label}: production must fail closed"
        );
        assert_eq!(
            frame.division_units_status(),
            ViewStatus::DomainUnavailable,
            "{label}: division equipment must fail closed"
        );
        assert_eq!(
            frame.fleets_status(),
            ViewStatus::DomainUnavailable,
            "{label}: fleets must fail closed"
        );
        assert_eq!(
            frame.focus_status(),
            ViewStatus::DomainUnavailable,
            "{label}: focus must fail closed"
        );
        assert_eq!(
            frame.leaders_status(),
            ViewStatus::DomainUnavailable,
            "{label}: leaders must fail closed"
        );
        assert_eq!(
            frame.diplomacy().status,
            ViewStatus::DomainUnavailable,
            "{label}: diplomacy must fail closed"
        );
    }

    #[test]
    fn provenance_parses_well_formed_domain_problems() {
        let rest = serde_json::json!({
            "provenance": {
                "collector": {
                    "captured": true,
                    "domains": ["production_live"],
                    "domain_problems": [
                        {
                            "domain_id": 11,
                            "domain": "production_live",
                            "problem": "production_count_uncovered"
                        },
                        {
                            "domain_id": 11,
                            "domain": "production_live",
                            "problem": "compact_status_failure"
                        }
                    ]
                }
            }
        });
        let mut warnings = Vec::new();
        let provenance = build_provenance(
            rest.as_object().expect("provenance fixture object"),
            &mut warnings,
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let collector = provenance
            .channels
            .get("collector")
            .expect("collector coverage");
        assert_eq!(collector.captured, Leaf::Measured(true));
        assert_eq!(collector.domains, vec!["production_live"]);
        assert_eq!(
            collector.problems,
            vec![
                DomainProblem {
                    domain: 11,
                    problem: "production_count_uncovered".to_string(),
                },
                DomainProblem {
                    domain: 11,
                    problem: "compact_status_failure".to_string(),
                },
            ]
        );
    }

    #[test]
    fn domain_problems_malformed_fail_closed_for_carried_production() {
        let clean = world_frame_from_json(carried_problem_backed_json(None));
        assert_eq!(clean.production_status(), ViewStatus::Present);
        let carried = clean.raw()["countries"]["GER"]["production"].clone();
        let malformed = [
            (
                "wrong outer type",
                serde_json::json!({"domain_id": 11, "problem": "partial_data"}),
            ),
            ("null", Value::Null),
            (
                "one malformed element",
                serde_json::json!([
                    {"domain_id": 11, "problem": "partial_data"},
                    "not-an-object"
                ]),
            ),
            (
                "missing problem string",
                serde_json::json!([{"domain_id": 11}]),
            ),
            (
                "wrong-typed problem string",
                serde_json::json!([{"domain_id": 11, "problem": 7}]),
            ),
            (
                "blank problem string",
                serde_json::json!([{"domain_id": 11, "problem": " \t"}]),
            ),
            (
                "negative domain id",
                serde_json::json!([{"domain_id": -1, "problem": "partial_data"}]),
            ),
            (
                "out-of-u16 domain id",
                serde_json::json!([{"domain_id": 65_536, "problem": "partial_data"}]),
            ),
            (
                "non-integer domain id",
                serde_json::json!([{"domain_id": 11.5, "problem": "partial_data"}]),
            ),
        ];

        for (label, domain_problems) in malformed {
            let frame = world_frame_from_json(carried_problem_backed_json(Some(domain_problems)));
            assert_eq!(
                frame.raw()["countries"]["GER"]["production"],
                carried,
                "{label}: raw carried production must remain available"
            );
            assert_eq!(
                frame.production_status(),
                ViewStatus::DomainUnavailable,
                "{label}: malformed coverage must fail closed"
            );
            assert_eq!(
                frame.division_units_status(),
                ViewStatus::DomainUnavailable,
                "{label}: malformed coverage must poison division equipment"
            );
            assert_eq!(
                frame.fleets_status(),
                ViewStatus::DomainUnavailable,
                "{label}: malformed coverage must poison fleets"
            );
            assert_eq!(
                frame.leaders_status(),
                ViewStatus::DomainUnavailable,
                "{label}: malformed coverage must poison leaders"
            );
            assert_eq!(
                frame.diplomacy().status,
                ViewStatus::DomainUnavailable,
                "{label}: malformed coverage must poison diplomacy"
            );
            assert_eq!(
                frame.focus_status(),
                ViewStatus::DomainUnavailable,
                "{label}: malformed coverage must poison focus"
            );
            assert!(
                frame
                    .coverage_warnings()
                    .iter()
                    .any(|warning| warning.contains("domain_problems")),
                "{label}: malformed coverage must retain a warning breadcrumb"
            );
        }
    }

    #[test]
    fn coverage_metadata_absent_at_each_level_keeps_carried_views_present() {
        let mut absent_provenance = carried_problem_backed_json(None);
        absent_provenance
            .as_object_mut()
            .expect("frame object")
            .remove("provenance");
        let mut absent_collector = carried_problem_backed_json(None);
        absent_collector["provenance"]
            .as_object_mut()
            .expect("provenance object")
            .remove("collector");

        for (label, root) in [
            ("provenance absent", absent_provenance),
            ("collector absent", absent_collector),
            ("domain_problems absent", carried_problem_backed_json(None)),
        ] {
            let frame = world_frame_from_json(root);
            assert_eq!(frame.production_status(), ViewStatus::Present, "{label}");
            assert_eq!(
                frame.division_units_status(),
                ViewStatus::Present,
                "{label}"
            );
            assert_eq!(frame.fleets_status(), ViewStatus::Present, "{label}");
            assert_eq!(frame.focus_status(), ViewStatus::Present, "{label}");
            assert_eq!(frame.leaders_status(), ViewStatus::Present, "{label}");
            assert_eq!(frame.diplomacy().status, ViewStatus::Present, "{label}");
            assert_eq!(
                frame
                    .production("GER")
                    .expect("typed production")
                    .stockpile_total,
                Leaf::Measured(120.0),
                "{label}"
            );
        }
    }

    #[test]
    fn provenance_envelope_present_but_malformed_fails_closed() {
        for (label, provenance) in [
            ("provenance null", Value::Null),
            (
                "provenance wrong type",
                Value::String("not-an-object".to_string()),
            ),
        ] {
            let mut root = carried_problem_backed_json(Some(serde_json::json!([])));
            root["provenance"] = provenance;
            let frame = world_frame_from_json(root);

            assert_all_problem_backed_views_unavailable(label, &frame);
            assert_eq!(
                frame.raw()["countries"]["GER"]["focus"]["current"],
                "carried_focus",
                "{label}: raw carried focus must remain diagnostic"
            );
            assert!(
                frame
                    .coverage_warnings()
                    .iter()
                    .any(|warning| warning.contains("provenance")),
                "{label}: malformed envelope needs a warning breadcrumb"
            );
        }
    }

    #[test]
    fn collector_envelope_present_but_malformed_fails_closed() {
        for (label, collector) in [
            ("collector null", Value::Null),
            (
                "collector wrong type refutation",
                serde_json::json!(["not-an-object"]),
            ),
        ] {
            let mut root = carried_problem_backed_json(Some(serde_json::json!([])));
            root["provenance"]["collector"] = collector;
            let frame = world_frame_from_json(root);

            assert_all_problem_backed_views_unavailable(label, &frame);
            assert_eq!(
                frame.raw()["countries"]["GER"]["focus"]["completed_count"],
                4,
                "{label}: raw carried focus must remain diagnostic"
            );
            assert!(
                frame
                    .coverage_warnings()
                    .iter()
                    .any(|warning| warning.contains("provenance.collector")),
                "{label}: malformed envelope needs a warning breadcrumb"
            );
        }
    }

    #[test]
    fn domain_problems_well_formed_downgrade_all_problem_backed_typed_views() {
        let clean = world_frame_from_json(carried_problem_backed_json(Some(serde_json::json!([]))));
        assert_eq!(clean.production_status(), ViewStatus::Present);
        assert_eq!(clean.division_units_status(), ViewStatus::Present);
        assert_eq!(clean.fleets_status(), ViewStatus::Present);
        assert_eq!(clean.leaders_status(), ViewStatus::Present);
        assert_eq!(clean.diplomacy().status, ViewStatus::Present);
        assert_eq!(clean.division_units().len(), 1);
        assert_eq!(clean.fleets().len(), 1);
        assert_eq!(
            clean
                .leaders("GER")
                .expect("typed leaders")
                .country_leader_name,
            Leaf::Measured("Carried Leader".to_string())
        );
        assert_eq!(
            clean.diplomacy().countries["GER"].faction_is_leader,
            Leaf::Measured(true)
        );
        assert_eq!(
            clean.diplomacy().faction_members,
            Leaf::Measured(BTreeMap::from([(
                "1".to_string(),
                vec!["GER".to_string(), "ITA".to_string()]
            )]))
        );

        let division_equipment = world_frame_from_json(carried_problem_backed_json(Some(
            serde_json::json!([{"domain_id": 26, "problem": "partial_data"}]),
        )));
        assert_eq!(
            division_equipment.division_units_status(),
            ViewStatus::DomainUnavailable
        );
        assert_eq!(
            division_equipment.raw()["divisions"],
            clean.raw()["divisions"],
            "domain 26 raw carried divisions/equipment must remain available"
        );

        let fleets = world_frame_from_json(carried_problem_backed_json(Some(
            serde_json::json!([{"domain_id": 28, "problem": "partial_data"}]),
        )));
        assert_eq!(fleets.fleets_status(), ViewStatus::DomainUnavailable);
        assert_eq!(
            fleets.raw()["order_of_battle"]["fleets"],
            clean.raw()["order_of_battle"]["fleets"],
            "domain 28 raw carried fleets must remain available"
        );

        let leaders = world_frame_from_json(carried_problem_backed_json(Some(
            serde_json::json!([{"domain_id": 47, "problem": "failure_status"}]),
        )));
        assert_eq!(leaders.leaders_status(), ViewStatus::DomainUnavailable);
        assert_eq!(
            leaders.raw()["countries"]["GER"]["commanders"],
            clean.raw()["countries"]["GER"]["commanders"],
            "domain 47 raw carried leaders must remain available"
        );

        let diplomacy = world_frame_from_json(carried_problem_backed_json(Some(
            serde_json::json!([{"domain_id": 53, "problem": "partial_data"}]),
        )));
        assert_eq!(diplomacy.diplomacy().status, ViewStatus::DomainUnavailable);
        assert_eq!(
            diplomacy.diplomacy().countries["GER"].faction_is_leader,
            Leaf::Measured(true),
            "domain 53 status guards the retained country-diplomacy projection"
        );
        assert_eq!(
            diplomacy.diplomacy().faction_members,
            clean.diplomacy().faction_members,
            "domain 53 status guards the retained faction projection"
        );
        assert_eq!(
            diplomacy.raw()["diplomacy_live"],
            clean.raw()["diplomacy_live"],
            "domain 53 raw carried diplomacy must remain available"
        );

        let focus = world_frame_from_json(carried_problem_backed_json(Some(
            serde_json::json!([{"domain_id": 45, "problem": "malformed_payload"}]),
        )));
        assert_eq!(focus.focus_status(), ViewStatus::DomainUnavailable);
        assert_eq!(
            focus.focus("GER").expect("typed focus").completed_count,
            Leaf::Measured(4),
            "domain 45 status guards the retained focus projection"
        );
        assert_eq!(
            focus.raw()["countries"]["GER"]["focus"],
            clean.raw()["countries"]["GER"]["focus"],
            "domain 45 raw carried focus must remain available"
        );
    }

    // OWNER RULING 2026-07-14 — ZERO DATA LOSS (per-domain degrade). A malformed single-domain
    // DOMAIN_DATA payload degrades ONLY that domain (records a "malformed_payload" problem, marks
    // focus DomainUnavailable, retains the carried raw metrics for diagnostics — the payload is
    // never applied) rather than failing the whole fold. The stream-boundary ingest gate
    // (stream_fold.rs drive_fold) now degrades instead of returning InvalidData; framing / magic /
    // version / checksum / zstd corruption still fails closed. This test is the acceptance oracle
    // for that degrade behavior (was briefly parked during the boundary-auth × foldwave2 merge).
    #[test]
    fn folded_malformed_d45_without_d12_is_unavailable_to_typed_consumer() {
        const HOUR_A: i64 = 16_959_360;
        const HOUR_B: i64 = HOUR_A + 1;
        let raw_a = DATE_EPOCH_CONST + HOUR_A as i32;
        let raw_b = DATE_EPOCH_CONST + HOUR_B as i32;
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            raw_a,
            18,
            RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(1, "GER"),
        );
        push_record(
            &mut stream,
            2,
            raw_a,
            45,
            RECORD_TYPE_DOMAIN_DATA,
            &focus_depth_payload(1),
        );
        push_record(
            &mut stream,
            3,
            raw_a,
            1000,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        push_record(
            &mut stream,
            4,
            raw_b,
            45,
            RECORD_TYPE_DOMAIN_DATA,
            b"malformed-d45",
        );
        push_record(
            &mut stream,
            5,
            raw_b,
            1000,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );

        let temp = TestDir::new("folded_malformed_d45");
        let stream_path = temp.0.join("collector_stream.bin");
        let snapshots_path = temp.0.join("snapshots");
        fs::write(&stream_path, stream).expect("write compact stream");
        fs::create_dir_all(&snapshots_path).expect("create snapshot directory");

        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("repository root");
        let extractor_manifest = repo_root.join("extractor/native/Cargo.toml");
        let output = Command::new(env!("CARGO"))
            .arg("run")
            .arg("--quiet")
            .arg("--manifest-path")
            .arg(extractor_manifest)
            .arg("-p")
            .arg("recorder_host")
            .arg("--bin")
            .arg("stream_fold")
            .arg("--")
            .arg(&stream_path)
            .arg(&snapshots_path)
            .arg("--game-id")
            .arg("typed_d45_status_regression")
            .arg("--workers")
            .arg("1")
            .output()
            .expect("run d45 stream fold");
        assert!(
            output.status.success(),
            "stream_fold failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let current = WorldFrame::from_raw(raw_frame_from_snapshot(
            &snapshots_path.join(format!("{HOUR_A:012}.snapshot.json")),
        ));
        let malformed = WorldFrame::from_raw(raw_frame_from_snapshot(
            &snapshots_path.join(format!("{HOUR_B:012}.snapshot.json")),
        ));

        assert_eq!(current.focus_status(), ViewStatus::Present);
        assert_eq!(
            current.focus("GER").expect("current typed focus").current,
            Leaf::Measured("focus_current".to_string())
        );
        assert_eq!(
            current
                .focus("GER")
                .expect("current typed focus")
                .completed_count,
            Leaf::Measured(4)
        );
        assert!(malformed.provenance().channels["collector"]
            .problems
            .iter()
            .any(|problem| problem.domain == 45 && problem.problem == "malformed_payload"));
        assert_eq!(
            malformed.focus_status(),
            ViewStatus::DomainUnavailable,
            "carried D45 focus with a malformed D45 problem must not remain Present"
        );
        for key in [
            "current",
            "completed_count",
            "_r2_depth_current",
            "_r2_depth_completed_count",
        ] {
            assert_eq!(
                malformed.raw()["countries"]["GER"]["focus"][key],
                current.raw()["countries"]["GER"]["focus"][key],
                "malformed D45 must retain carried raw focus metric {key} for diagnostics"
            );
        }
    }

    #[test]
    fn folded_undercovered_production_is_unavailable_to_typed_consumer() {
        const HOUR_A: i64 = 16_959_360;
        const HOUR_B: i64 = HOUR_A + 1;
        let raw_a = DATE_EPOCH_CONST + HOUR_A as i32;
        let raw_b = DATE_EPOCH_CONST + HOUR_B as i32;
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            raw_a,
            18,
            RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(1, "GER"),
        );
        push_record(
            &mut stream,
            2,
            raw_a,
            11,
            RECORD_TYPE_DOMAIN_DATA,
            &production_payload(1, Some(1)),
        );
        push_record(
            &mut stream,
            3,
            raw_a,
            1000,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        push_record(
            &mut stream,
            4,
            raw_b,
            11,
            RECORD_TYPE_DOMAIN_DATA,
            &production_payload(1, None),
        );
        push_record(
            &mut stream,
            5,
            raw_b,
            1000,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );

        let temp = TestDir::new("folded_undercovered_production");
        let stream_path = temp.0.join("collector_stream.bin");
        let snapshots_path = temp.0.join("snapshots");
        fs::write(&stream_path, stream).expect("write compact stream");
        fs::create_dir_all(&snapshots_path).expect("create snapshot directory");

        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("repository root");
        let extractor_manifest = repo_root.join("extractor/native/Cargo.toml");
        let output = Command::new(env!("CARGO"))
            .arg("run")
            .arg("--quiet")
            .arg("--manifest-path")
            .arg(extractor_manifest)
            .arg("-p")
            .arg("recorder_host")
            .arg("--bin")
            .arg("stream_fold")
            .arg("--")
            .arg(&stream_path)
            .arg(&snapshots_path)
            .arg("--game-id")
            .arg("typed_status_regression")
            .arg("--workers")
            .arg("1")
            .output()
            .expect("run production stream fold");
        assert!(
            output.status.success(),
            "stream_fold failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let current = WorldFrame::from_raw(raw_frame_from_snapshot(
            &snapshots_path.join(format!("{HOUR_A:012}.snapshot.json")),
        ));
        let undercovered = WorldFrame::from_raw(raw_frame_from_snapshot(
            &snapshots_path.join(format!("{HOUR_B:012}.snapshot.json")),
        ));

        assert_eq!(current.production_status(), ViewStatus::Present);
        assert!(
            undercovered.provenance().channels["collector"]
                .problems
                .iter()
                .any(|problem| problem.domain == 11
                    && problem.problem == "production_count_uncovered")
        );
        assert_eq!(
            undercovered.production_status(),
            ViewStatus::DomainUnavailable,
            "carried production with a d11 domain_problem must not remain Present"
        );
        assert_eq!(
            undercovered.raw()["countries"]["GER"]["production"],
            current.raw()["countries"]["GER"]["production"],
            "raw carried production remains available for diagnostics"
        );
    }
}
