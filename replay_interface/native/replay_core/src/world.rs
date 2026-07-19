//! Scrubbable world model over a [`ReplaySource`].
//!
//! [`WorldTimeline`] owns the source and a small frame cache; [`WorldFrame`]
//! is one hour of world state with the raw domain map plus typed views for
//! the surfaces the replay UI consumes (map control, divisions, combats,
//! country core, provenance). Typed views never coerce: a wrong-typed value
//! becomes [`Leaf::Unobtained`] plus a [`WorldFrame::coverage_warnings`]
//! entry, never a guessed value and never a silent drop.

use crate::design_catalog::{
    CurrentDesignCatalog, DesignCatalogJoinError, DesignKind, DisplayOrderSnapshot,
    FieldCompleteness, PortableDesignCatalog,
};
use crate::source::{RawFrame, ReplaySource, SourceError};
use portable_render_stream::ProductionRenderFrame;
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
    /// Producer-owned display inputs. These remain `Unobtained` for the
    /// current d21/d55 fold where the recorder deliberately emits null.
    pub combat_width: Leaf<f64>,
    pub terrain: Leaf<String>,
    pub progress: Leaf<f64>,
    /// Authenticated d55 lifecycle/current-side enrichment.
    pub detail: Leaf<LandCombatDetail>,
}

/// One terminal land-combat lifecycle row from top-level `ended_combats[]`.
///
/// This is deliberately a different type and collection from [`CombatRow`]:
/// ended rows must never be rendered as live battles. Identity is the complete
/// `(combat_id, generation)` pair because combat ids can be reused.
#[derive(Debug, Clone, PartialEq)]
pub struct EndedCombatRow {
    pub combat_id: i64,
    pub generation: i64,
    pub kind: Leaf<String>,
    pub lifecycle: Leaf<String>,
    pub location_province: Leaf<u32>,
    pub attackers: Leaf<Vec<i64>>,
    pub defenders: Leaf<Vec<i64>>,
    pub combat_width: Leaf<f64>,
    pub terrain: Leaf<String>,
    pub progress: Leaf<f64>,
    pub side_detail_availability: Leaf<String>,
    pub detail: Leaf<LandCombatDetail>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CombatParticipantEvent {
    pub event: Leaf<String>,
    pub generation: Leaf<i64>,
    pub side: Leaf<String>,
    pub division_id: Leaf<i64>,
    pub old_side_count: Leaf<i64>,
    pub new_side_count: Leaf<i64>,
    pub semantic: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LandDamageAttributionRow {
    pub dealer_tag_db_id: Leaf<i64>,
    pub victim_tag_db_id: Leaf<i64>,
    pub damage: Leaf<f64>,
    pub damage_raw: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LandCombatTacticIdentity {
    pub id: Leaf<i64>,
    pub token: Leaf<String>,
    pub name: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LandCombatSideDetail {
    pub units: Leaf<i64>,
    pub committed: Leaf<i64>,
    pub reserve: Leaf<i64>,
    pub air_kills: Leaf<i64>,
    pub org_loss_summary_index: Leaf<i64>,
    pub num_org_losses: Leaf<i64>,
    pub anti_air_attack: Leaf<f64>,
    pub air_damage_str: Leaf<f64>,
    pub air_damage_org: Leaf<f64>,
    pub ground_damage_str: Leaf<f64>,
    pub ground_damage_org: Leaf<f64>,
    pub prevented_damage_str: Leaf<f64>,
    pub prevented_damage_org: Leaf<f64>,
    pub progress: Leaf<f64>,
    pub progress_raw: Leaf<i64>,
    /// `Measured(None)` preserves a null tactic payload. `tactic_status`
    /// distinguishes exact v4 no-tactic from a legacy unavailable stream;
    /// `Unobtained` is reserved for a malformed non-object payload.
    pub tactic: Leaf<Option<LandCombatTacticIdentity>>,
    pub tactic_status: Leaf<String>,
    pub losses_status: Leaf<String>,
    pub counts_incomplete: Leaf<bool>,
    /// Exact reserve-vector identities in engine order. Null/unobtained is
    /// distinct from a measured empty vector.
    pub reserve_division_ids: Leaf<Vec<i64>>,
    pub reserve_members_status: Leaf<String>,
    pub damage_attribution: Leaf<Vec<LandDamageAttributionRow>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LandCombatDetail {
    pub phase_event: Leaf<String>,
    pub generation: Leaf<i64>,
    pub location_province: Leaf<u32>,
    pub age_days: Leaf<u32>,
    pub duration_days: Leaf<u32>,
    pub participant_events: Leaf<Vec<CombatParticipantEvent>>,
    pub side_detail_completeness: Leaf<String>,
    pub attacker: Leaf<LandCombatSideDetail>,
    pub defender: Leaf<LandCombatSideDetail>,
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
/// A3 pins `session.speed_events[*].speed` to the canonical raw engine slot
/// `0..=4`; [`SpeedEvent::displayed_speed`] derives the UI label `1..=5`.
/// Current extractor frames provide exact hourly speed/pause anchors. X12's
/// separate sub-hour transition producer remains required before the list can
/// claim gap-free transition history. Fallback is limited to already
/// schema-shaped `world.scenario` leaves for DLC/language; missing fields stay
/// absent/unavailable rather than inferred.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMetadataView {
    pub status: ViewStatus,
    pub player_countries: Leaf<Vec<PlayerCountry>>,
    pub speed_events: Leaf<Vec<SpeedEvent>>,
    pub active_dlcs: Leaf<Vec<String>>,
    pub localisation_language: Leaf<String>,
    pub display_variant: Leaf<DisplayVariantMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayVariantMetadata {
    pub physical_width: u32,
    pub physical_height: u32,
    pub min_gui_width: u32,
    pub min_gui_height: u32,
    pub gui_scale_f32_bits: u32,
    pub refresh_rate_hz: u32,
    pub max_refresh_rate_hz: Option<u32>,
    pub fullscreen: bool,
    pub borderless: bool,
    pub display_mode: String,
    pub settings_file_sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerCountry {
    pub tag: Leaf<String>,
    pub player_name: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeedEvent {
    pub hour_index: Leaf<i64>,
    /// Canonical engine speed slot, exactly as captured (`0..=4`).
    pub speed: Leaf<i64>,
    pub paused: Leaf<bool>,
}

impl SpeedEvent {
    /// UI-visible speed label (`1..=5`) derived losslessly from the canonical
    /// engine slot. No second speed value is stored in the recording.
    pub fn displayed_speed(&self) -> Leaf<i64> {
        match &self.speed {
            Leaf::Absent => Leaf::Absent,
            Leaf::Unobtained => Leaf::Unobtained,
            Leaf::Measured(raw) if (0..=4).contains(raw) => Leaf::Measured(*raw + 1),
            Leaf::Measured(_) => Leaf::Unobtained,
        }
    }
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

/// Exact W1B contribution members from the a729 war-participation record.
#[derive(Debug, Clone, PartialEq)]
pub struct WarParticipationBreakdown {
    pub land_damage: Leaf<i64>,
    pub occupation: Leaf<i64>,
    pub air_damage: Leaf<i64>,
    pub strategic_bombing: Leaf<i64>,
    pub sunk_ships: Leaf<i64>,
    pub sunk_convoys: Leaf<i64>,
    pub casualties: Leaf<i64>,
    pub lend_lease_given: Leaf<i64>,
    pub lend_lease_received: Leaf<i64>,
}

/// One ordered participant row. Fixed-point producer values are retained as
/// raw 1e5 integers; replay presentation must not recompute engine inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct WarParticipantOverviewRow {
    pub tag: Leaf<String>,
    pub tag_db_id: Leaf<i64>,
    pub participation_raw: Leaf<i64>,
    pub capitulation_progress_raw: Leaf<i64>,
    pub breakdown: Leaf<WarParticipationBreakdown>,
}

/// One authenticated D60 war-object row. The current producer is pair-scoped;
/// it does not claim alliance-side grouping or per-participant warscore/IC.
#[derive(Debug, Clone, PartialEq)]
pub struct WarObjectRow {
    pub key: String,
    /// Exact ordered attacker/defender side tags when the producer carries a
    /// complete war object. Pair-scoped D60 rows leave these absent.
    pub attackers: Leaf<Vec<String>>,
    pub defenders: Leaf<Vec<String>>,
    /// Engine-resolved display name. The source object key is identity only
    /// and must never be substituted for a missing display name.
    pub war_name: Leaf<String>,
    pub front: Leaf<String>,
    pub start_hour: Leaf<i64>,
    pub attacker_participants: Leaf<Vec<WarParticipantOverviewRow>>,
    pub defender_participants: Leaf<Vec<WarParticipantOverviewRow>>,
    /// Signed aggressor-minus-defender fixed-1e5 progress.
    pub overall_progress_raw: Leaf<i64>,
    pub first: Leaf<String>,
    pub second: Leaf<String>,
    pub first_tag_db_id: Leaf<i64>,
    pub second_tag_db_id: Leaf<i64>,
    pub start_date: Leaf<String>,
    pub first_casualties: Leaf<i64>,
    pub second_casualties: Leaf<i64>,
    pub casualties_semantics: Leaf<String>,
    pub casualties_is_save_snapshot: Leaf<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiplomacyView {
    pub status: ViewStatus,
    pub pairs: BTreeMap<(String, String), DiplomaticPairState>,
    pub countries: BTreeMap<String, CountryDiplomacy>,
    /// Live war-relation rows from `diplomacy_live.war_relations`
    /// (`Absent` when the key never appears, `Unobtained` when explicit-null).
    pub war_relations: Leaf<Vec<WarRelationRow>>,
    pub wars: Leaf<Vec<WarObjectRow>>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicCollectionRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicCollectionSourceItem {
    pub stable_item_id: String,
    pub source_ordinal: u32,
    pub included: bool,
    pub filter_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicCollectionGroup {
    pub group_id: String,
    pub start_index: u32,
    pub item_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicCollectionMaterializedSlot {
    pub slot_id: String,
    pub bound_item_id: String,
    pub row_template_id: String,
    pub rect: DynamicCollectionRect,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicCollectionTraceRow {
    pub collection_instance_id: String,
    pub frame_hour: u64,
    pub source_items: Vec<DynamicCollectionSourceItem>,
    pub ordered_item_ids: Vec<String>,
    pub group_boundaries: Vec<DynamicCollectionGroup>,
    pub row_template_id: String,
    pub materialized_slots: Vec<DynamicCollectionMaterializedSlot>,
    pub clip_rect: DynamicCollectionRect,
    pub content_extent_pixels: [i32; 2],
    pub viewport_extent_pixels: [i32; 2],
    pub scroll_min_fixed: i64,
    pub scroll_max_fixed: i64,
    pub scroll_current_fixed: i64,
    pub scroll_target_fixed: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicCollectionTraceView {
    pub status: ViewStatus,
    pub frame_hour: Option<u64>,
    pub collections: Vec<DynamicCollectionTraceRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignCatalogContext {
    pub schema_version: u32,
    pub authority: String,
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub completeness_record_identity: String,
    pub transition_sequence: u64,
    pub active: bool,
    pub mode: i32,
    pub family: String,
    pub our_filter_enabled: bool,
    pub foreign_filter_enabled: bool,
    pub obsolete_filter_enabled: bool,
    pub current_country_tag_id: u32,
    pub selected_country_tag_ids: Vec<u32>,
    pub ordered_design_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignCatalogContextView {
    pub status: ViewStatus,
    pub context: Option<DesignCatalogContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HudSurfaceActivationKind {
    Topbar,
    Minimap,
    Mapmodes,
    TheatreSelector,
    DesignerCountryEquipment,
    DesignerTank,
    DesignerPlane,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudSurfaceActivationRoot {
    pub surface: HudSurfaceActivationKind,
    pub runtime_root_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudSurfaceActivation {
    pub schema_version: u32,
    pub authority: String,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub observation_sequence: u64,
    pub active_surfaces: Vec<HudSurfaceActivationRoot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudSurfaceActivationView {
    pub status: ViewStatus,
    pub activation: Option<HudSurfaceActivation>,
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
    ended_combats: BTreeMap<(i64, i64), EndedCombatRow>,
    ended_combats_status: ViewStatus,
    countries: BTreeMap<String, CountryCore>,
    countries_status: ViewStatus,
    session_metadata: SessionMetadataView,
    naval_supremacy_by_region: BTreeMap<u32, NavalSupremacyRow>,
    naval_supremacy_status: ViewStatus,
    air_superiority_by_region: BTreeMap<u32, AirSuperiorityRow>,
    air_superiority_status: ViewStatus,
    weather_by_region: BTreeMap<u32, WeatherRegionRow>,
    weather_status: ViewStatus,
    production_render: Option<ProductionRenderFrame>,
    production_render_status: ViewStatus,
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
    character_rosters_exact: BTreeMap<String, CharacterRosterExactView>,
    character_rosters_exact_status: ViewStatus,
    ace_rosters_exact: BTreeMap<String, AceRosterExactView>,
    ace_rosters_exact_status: ViewStatus,
    occupation_by_state: BTreeMap<u32, OccupationRow>,
    occupation_status: ViewStatus,
    occupation_history: Leaf<Vec<OccupationHistoryExactRow>>,
    occupation_history_complete: Leaf<bool>,
    power_balance: PowerBalanceView,
    designs: DesignsView,
    naval_air_combat: NavalAirCombatView,
    scripted_gui: ScriptedGuiView,
    dynamic_collection_trace: DynamicCollectionTraceView,
    design_catalog_context: DesignCatalogContextView,
    hud_surface_activation: HudSurfaceActivationView,
    design_catalog: Option<Arc<PortableDesignCatalog>>,
    raids: RaidStateView,
    fired_events: FiredEventsView,
    fired_events_status: ViewStatus,
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
    /// Terminal lifecycle rows keyed by `(combat_id, generation)`.
    pub fn ended_combats(&self) -> &BTreeMap<(i64, i64), EndedCombatRow> {
        &self.ended_combats
    }
    pub fn ended_combats_status(&self) -> ViewStatus {
        self.ended_combats_status
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
    /// Exact post-simulation/post-evaluation non-UI submissions. Gameplay rows
    /// are never adapted into this packet-native carrier.
    pub fn production_render(&self) -> Option<&ProductionRenderFrame> {
        self.production_render.as_ref()
    }
    pub fn production_render_status(&self) -> ViewStatus {
        self.production_render_status
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
        Self::from_raw_with_design_catalog(frame, None)
    }

    fn from_raw_with_design_catalog(
        frame: RawFrame,
        design_catalog: Option<Arc<PortableDesignCatalog>>,
    ) -> WorldFrame {
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
        let (ended_combats, ended_combats_status) = build_ended_combats(&frame.rest, &mut warnings);
        let (countries, countries_status) = build_countries(&frame.rest, &mut warnings);
        let session_metadata = build_session_metadata(&frame.rest, &mut warnings);
        let (naval_supremacy_by_region, naval_supremacy_status) =
            build_naval_supremacy(&frame.rest, &mut warnings);
        let (air_superiority_by_region, air_superiority_status) =
            build_air_superiority(&frame.rest, &mut warnings);
        let (weather_by_region, weather_status) = build_weather(&frame.rest, &mut warnings);
        let (production_render, production_render_status) =
            build_production_render(&frame.rest, frame.hour_index, &mut warnings);
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
        let (
            character_rosters_exact,
            character_rosters_exact_status,
            mut ace_rosters_exact,
            mut ace_rosters_exact_status,
        ) = build_xui17_exact_rosters(&frame.rest, &mut warnings);
        let (occupation_by_state, occupation_status) = build_occupation(&frame.rest, &mut warnings);
        let (occupation_history, occupation_history_complete) =
            build_occupation_history(&frame.rest, &mut warnings);
        let power_balance = build_power_balance(&frame.rest, &mut warnings);
        let designs = build_designs(&frame.rest, &mut warnings);
        let naval_air_combat = build_naval_air_combat(&frame.rest, &mut warnings);
        let scripted_gui = build_scripted_gui(&frame.rest, &mut warnings);
        let dynamic_collection_trace = build_dynamic_collection_trace(&frame.rest, &mut warnings);
        let design_catalog_context = build_design_catalog_context(&frame.rest, &mut warnings);
        let hud_surface_activation =
            build_hud_surface_activation(&frame.rest, frame.hour_index, &mut warnings);
        let raids = build_raids(&frame.rest, &mut warnings);
        let (fired_events, fired_events_status) = build_fired_events(&frame.rest, &mut warnings);
        let provenance = build_provenance(&frame.rest, &mut warnings);
        reconcile_domain_problem_statuses(
            &frame.rest,
            &mut production_status,
            &mut division_units_status,
            &mut fleets_status,
            &mut focus_status,
            &mut leaders_status,
            &mut diplomacy.status,
            &mut ace_rosters_exact,
            &mut ace_rosters_exact_status,
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
            ended_combats,
            ended_combats_status,
            countries,
            countries_status,
            session_metadata,
            naval_supremacy_by_region,
            naval_supremacy_status,
            air_superiority_by_region,
            air_superiority_status,
            weather_by_region,
            weather_status,
            production_render,
            production_render_status,
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
            character_rosters_exact,
            character_rosters_exact_status,
            ace_rosters_exact,
            ace_rosters_exact_status,
            occupation_by_state,
            occupation_status,
            occupation_history,
            occupation_history_complete,
            power_balance,
            designs,
            naval_air_combat,
            scripted_gui,
            dynamic_collection_trace,
            design_catalog_context,
            hud_surface_activation,
            design_catalog,
            raids,
            fired_events,
            fired_events_status,
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
    design_catalog: Option<Arc<PortableDesignCatalog>>,
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
        let design_catalog = source.design_catalog().cloned();
        Ok(WorldTimeline {
            source,
            hours,
            config,
            cache: Mutex::new(FrameCache::new(config.frame_cache_entries)),
            design_catalog,
        })
    }

    /// The frame-cache entry budget this timeline was opened with.
    pub fn frame_cache_entries(&self) -> usize {
        self.config.frame_cache_entries
    }

    /// Available before the first frame decode, so first-launch VFS admission
    /// gates construction of every production renderer/catalog.
    pub fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.source.content_manifest()
    }

    pub fn design_catalog(&self) -> Option<&PortableDesignCatalog> {
        self.design_catalog.as_deref()
    }

    pub fn admit_playback_content(
        &self,
        inventory: &portable_content_manifest::PlaybackContentInventory,
    ) -> Result<
        portable_content_manifest::ValidatedContentAdmission,
        portable_content_manifest::ContentValidationReport,
    > {
        let Some(manifest) = self.content_manifest() else {
            return Err(portable_content_manifest::ContentValidationReport {
                compatible: false,
                verified_required_content: 0,
                issues: vec![portable_content_manifest::ContentIssue {
                    code: portable_content_manifest::ContentIssueCode::InvalidManifest,
                    subject: "manifest.content_manifest".into(),
                    message: "recording has no portable used-content manifest".into(),
                }],
            });
        };
        manifest.admit_playback(inventory)
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
        let frame = Arc::new(WorldFrame::from_raw_with_design_catalog(
            raw,
            self.design_catalog.clone(),
        ));
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

fn leaf_u64(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<u64> {
    typed_leaf(
        obj,
        path,
        "non-negative integer",
        ctx,
        warnings,
        Value::as_u64,
    )
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

fn leaf_u32_prefer(
    obj: &Map<String, Value>,
    primary: &[&str],
    fallback: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<u32> {
    match leaf_u32(obj, primary, ctx, warnings) {
        Leaf::Absent => leaf_u32(obj, fallback, ctx, warnings),
        value => value,
    }
}

fn leaf_i64_prefer(
    obj: &Map<String, Value>,
    primary: &[&str],
    fallback: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<i64> {
    match leaf_i64(obj, primary, ctx, warnings) {
        Leaf::Absent => leaf_i64(obj, fallback, ctx, warnings),
        value => value,
    }
}

fn leaf_f64_prefer(
    obj: &Map<String, Value>,
    primary: &[&str],
    fallback: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<f64> {
    match leaf_f64(obj, primary, ctx, warnings) {
        Leaf::Absent => leaf_f64(obj, fallback, ctx, warnings),
        value => value,
    }
}

fn leaf_string_prefer(
    obj: &Map<String, Value>,
    primary: &[&str],
    fallback: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<String> {
    match leaf_string(obj, primary, ctx, warnings) {
        Leaf::Absent => leaf_string(obj, fallback, ctx, warnings),
        value => value,
    }
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

fn leaf_i64_list(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<i64>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item.as_i64() {
                    Some(value) => out.push(value),
                    None => {
                        warnings.push(format!(
                            "{ctx}.{}[{i}]: expected integer, found {}; whole list treated as unobtained",
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
                "{ctx}.{}: present but wrong-typed (expected integer array, found {}); treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn leaf_string_map(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<BTreeMap<String, String>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Object(map)) => {
            let mut out = BTreeMap::new();
            for (key, value) in map {
                match value_to_id_string(value) {
                    Some(value) => {
                        out.insert(key.clone(), value);
                    }
                    None => {
                        warnings.push(format!(
                            "{ctx}.{}.{key}: expected string/integer id, found {}; whole map treated as unobtained",
                            path.join("."),
                            json_type_name(value)
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(out)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: present but wrong-typed (expected object, found {}); treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn leaf_string_list_map(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<BTreeMap<String, Vec<String>>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Object(map)) => {
            let mut out = BTreeMap::new();
            for (key, value) in map {
                let Value::Array(items) = value else {
                    warnings.push(format!(
                        "{ctx}.{}.{key}: expected array, found {}; whole map treated as unobtained",
                        path.join("."),
                        json_type_name(value)
                    ));
                    return Leaf::Unobtained;
                };
                let mut values = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    match value_to_id_string(item) {
                        Some(value) => values.push(value),
                        None => {
                            warnings.push(format!(
                                "{ctx}.{}.{key}[{i}]: expected string/integer id, found {}; whole map treated as unobtained",
                                path.join("."),
                                json_type_name(item)
                            ));
                            return Leaf::Unobtained;
                        }
                    }
                }
                out.insert(key.clone(), values);
            }
            Leaf::Measured(out)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: present but wrong-typed (expected object, found {}); treated as unobtained",
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

/// Shared division identity derivation for `divisions()` and
/// `division_units()`. Both views derive identity from the same rows into the
/// same per-frame warnings vec, so each degradation warning is pushed once per
/// frame (the messages embed the row index, so the duplicate guard only
/// collapses the double emission across the two views — distinct rows still
/// warn individually).
fn division_uid_from_obj(
    obj: &Map<String, Value>,
    div_id: i64,
    index: usize,
    warnings: &mut Vec<String>,
) -> String {
    let mut warn_once = |message: String| {
        if !warnings.contains(&message) {
            warnings.push(message);
        }
    };
    match obj.get("div_type") {
        // absent div_type is the schema-sanctioned bare-id identity
        None => format!("{div_id}"),
        // null is captured-but-unobtained, not schema-absent: identity
        // still degrades to the bare id (§6 untyped-channel matching),
        // but the degradation is surfaced, never silent
        Some(Value::Null) => {
            warn_once(format!(
                "divisions[{index}].div_type is null (unobtained); falling back to \
                 bare div_id identity"
            ));
            format!("{div_id}")
        }
        Some(v) => match v.as_i64() {
            Some(t) => format!("{div_id}:{t}"),
            None => {
                warn_once(format!(
                    "divisions[{index}].div_type: present but wrong-typed (expected \
                     integer, found {}); falling back to bare div_id identity",
                    json_type_name(v)
                ));
                format!("{div_id}")
            }
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
        let div_uid = division_uid_from_obj(obj, div_id, i, warnings);
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
            combat_width: leaf_f64(obj, &["combat_width"], &ctx, warnings),
            terrain: leaf_string(obj, &["terrain"], &ctx, warnings),
            progress: leaf_f64(obj, &["progress"], &ctx, warnings),
            detail: parse_land_combat_detail(obj, &ctx, warnings),
        });
    }
    rows
}

fn build_ended_combats(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<(i64, i64), EndedCombatRow>, ViewStatus) {
    let items = match rest.get("ended_combats") {
        None => return (BTreeMap::new(), ViewStatus::DomainAbsent),
        Some(Value::Null) => return (BTreeMap::new(), ViewStatus::DomainUnavailable),
        Some(Value::Array(items)) => items,
        Some(other) => {
            warnings.push(format!(
                "ended_combats: present but wrong-typed (expected array, found {}); \
                 terminal combat view is unavailable",
                json_type_name(other)
            ));
            return (BTreeMap::new(), ViewStatus::DomainUnavailable);
        }
    };

    let mut rows = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            warnings.push(format!(
                "ended_combats[{i}]: expected object, found {}; row skipped",
                json_type_name(item)
            ));
            continue;
        };
        let ctx = format!("ended_combats[{i}]");
        let Some(combat_id) = obj.get("combat_id").and_then(Value::as_i64) else {
            warnings.push(format!(
                "{ctx}: missing or non-integer combat_id; row skipped"
            ));
            continue;
        };
        let Some(generation) = obj.get("generation").and_then(Value::as_i64) else {
            warnings.push(format!(
                "{ctx}: missing or non-integer generation; row skipped"
            ));
            continue;
        };
        if combat_id <= 0 || generation <= 0 {
            warnings.push(format!(
                "{ctx}: non-positive terminal identity; row skipped"
            ));
            continue;
        }
        if obj.get("lifecycle").and_then(Value::as_str) != Some("ended") {
            warnings.push(format!(
                "{ctx}: lifecycle is not exactly 'ended'; row skipped rather than exposed as terminal"
            ));
            continue;
        }
        let key = (combat_id, generation);
        if rows.contains_key(&key) {
            warnings.push(format!(
                "{ctx}: duplicate terminal combat identity ({combat_id},{generation}); later row skipped"
            ));
            continue;
        }
        rows.insert(
            key,
            EndedCombatRow {
                combat_id,
                generation,
                kind: leaf_string(obj, &["kind"], &ctx, warnings),
                lifecycle: leaf_string(obj, &["lifecycle"], &ctx, warnings),
                location_province: leaf_u32(obj, &["location_province"], &ctx, warnings),
                attackers: leaf_id_list(obj, "attackers", &ctx, warnings),
                defenders: leaf_id_list(obj, "defenders", &ctx, warnings),
                combat_width: leaf_f64(obj, &["combat_width"], &ctx, warnings),
                terrain: leaf_string(obj, &["terrain"], &ctx, warnings),
                progress: leaf_f64(obj, &["progress"], &ctx, warnings),
                side_detail_availability: leaf_string(
                    obj,
                    &["side_detail_availability"],
                    &ctx,
                    warnings,
                ),
                detail: parse_land_combat_detail(obj, &ctx, warnings),
            },
        );
    }
    (rows, ViewStatus::Present)
}

fn parse_land_combat_detail(
    combat: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<LandCombatDetail> {
    let key = "land_combat_detail_events_live";
    let obj = match combat.get(key) {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Unobtained,
        Some(Value::Object(obj)) => obj,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.{key}: expected object, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let dctx = format!("{ctx}.{key}");
    Leaf::Measured(LandCombatDetail {
        phase_event: leaf_string(obj, &["phase_event"], &dctx, warnings),
        generation: leaf_i64(obj, &["generation"], &dctx, warnings),
        location_province: leaf_u32(obj, &["location_province"], &dctx, warnings),
        age_days: leaf_u32(obj, &["age_days"], &dctx, warnings),
        duration_days: leaf_u32(obj, &["duration_days"], &dctx, warnings),
        participant_events: parse_participant_events(obj, &dctx, warnings),
        side_detail_completeness: leaf_string(obj, &["side_detail_completeness"], &dctx, warnings),
        attacker: parse_land_side(obj, "attacker", &dctx, warnings),
        defender: parse_land_side(obj, "defender", &dctx, warnings),
    })
}

fn parse_participant_events(
    detail: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<CombatParticipantEvent>> {
    let items = match detail.get("participant_events") {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Unobtained,
        Some(Value::Array(items)) => items,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.participant_events: expected array, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            warnings.push(format!(
                "{ctx}.participant_events[{index}]: expected object; whole list unobtained"
            ));
            return Leaf::Unobtained;
        };
        let rctx = format!("{ctx}.participant_events[{index}]");
        rows.push(CombatParticipantEvent {
            event: leaf_string(obj, &["event"], &rctx, warnings),
            generation: leaf_i64(obj, &["generation"], &rctx, warnings),
            side: leaf_string(obj, &["side"], &rctx, warnings),
            division_id: leaf_i64(obj, &["division_id"], &rctx, warnings),
            old_side_count: leaf_i64(obj, &["old_side_count"], &rctx, warnings),
            new_side_count: leaf_i64(obj, &["new_side_count"], &rctx, warnings),
            semantic: leaf_string(obj, &["semantic"], &rctx, warnings),
        });
    }
    Leaf::Measured(rows)
}

fn parse_land_side(
    detail: &Map<String, Value>,
    key: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<LandCombatSideDetail> {
    let obj = match detail.get(key) {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Unobtained,
        Some(Value::Object(obj)) => obj,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.{key}: expected object, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let sctx = format!("{ctx}.{key}");
    Leaf::Measured(LandCombatSideDetail {
        units: leaf_i64(obj, &["units"], &sctx, warnings),
        committed: leaf_i64(obj, &["committed"], &sctx, warnings),
        reserve: leaf_i64(obj, &["reserve"], &sctx, warnings),
        air_kills: leaf_i64(obj, &["air_kills"], &sctx, warnings),
        org_loss_summary_index: leaf_i64(obj, &["org_loss_summary_index"], &sctx, warnings),
        num_org_losses: leaf_i64(obj, &["num_org_losses"], &sctx, warnings),
        anti_air_attack: leaf_f64(obj, &["anti_air_attack"], &sctx, warnings),
        air_damage_str: leaf_f64(obj, &["air_damage_str"], &sctx, warnings),
        air_damage_org: leaf_f64(obj, &["air_damage_org"], &sctx, warnings),
        ground_damage_str: leaf_f64(obj, &["ground_damage_str"], &sctx, warnings),
        ground_damage_org: leaf_f64(obj, &["ground_damage_org"], &sctx, warnings),
        prevented_damage_str: leaf_f64(obj, &["prevented_damage_str"], &sctx, warnings),
        prevented_damage_org: leaf_f64(obj, &["prevented_damage_org"], &sctx, warnings),
        progress: leaf_f64(obj, &["progress"], &sctx, warnings),
        progress_raw: leaf_i64(obj, &["progress_raw"], &sctx, warnings),
        tactic: parse_land_tactic(obj, &sctx, warnings),
        tactic_status: leaf_string(obj, &["tactic_status"], &sctx, warnings),
        losses_status: leaf_string(obj, &["losses_status"], &sctx, warnings),
        counts_incomplete: leaf_bool(obj, &["counts_incomplete"], &sctx, warnings),
        reserve_division_ids: leaf_i64_list(obj, &["reserve_division_ids"], &sctx, warnings),
        reserve_members_status: leaf_string(obj, &["reserve_members_status"], &sctx, warnings),
        damage_attribution: parse_damage_attribution(obj, &sctx, warnings),
    })
}

fn parse_land_tactic(
    side: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Option<LandCombatTacticIdentity>> {
    let obj = match side.get("tactic") {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Measured(None),
        Some(Value::Object(obj)) => obj,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.tactic: expected object or null, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let tctx = format!("{ctx}.tactic");
    Leaf::Measured(Some(LandCombatTacticIdentity {
        id: leaf_i64(obj, &["id"], &tctx, warnings),
        token: leaf_string(obj, &["token"], &tctx, warnings),
        name: leaf_string(obj, &["name"], &tctx, warnings),
    }))
}

fn parse_damage_attribution(
    side: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<LandDamageAttributionRow>> {
    let items = match side.get("damage_attribution") {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Unobtained,
        Some(Value::Array(items)) => items,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.damage_attribution: expected array, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            warnings.push(format!(
                "{ctx}.damage_attribution[{index}]: expected object; whole list unobtained"
            ));
            return Leaf::Unobtained;
        };
        let rctx = format!("{ctx}.damage_attribution[{index}]");
        rows.push(LandDamageAttributionRow {
            dealer_tag_db_id: leaf_i64(obj, &["dealer_tag_db_id"], &rctx, warnings),
            victim_tag_db_id: leaf_i64(obj, &["victim_tag_db_id"], &rctx, warnings),
            damage: leaf_f64(obj, &["damage"], &rctx, warnings),
            damage_raw: leaf_i64(obj, &["damage_raw"], &rctx, warnings),
        });
    }
    Leaf::Measured(rows)
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
    let display_variant = match session {
        Some(obj) => leaf_display_variant(obj, warnings),
        None if session_status == ViewStatus::DomainUnavailable => Leaf::Unobtained,
        None => Leaf::Absent,
    };
    SessionMetadataView {
        status,
        player_countries,
        speed_events,
        active_dlcs,
        localisation_language,
        display_variant,
    }
}

fn leaf_display_variant(
    session: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Leaf<DisplayVariantMetadata> {
    let Some(value) = session.get("display_variant") else {
        return Leaf::Absent;
    };
    let display: DisplayVariantMetadata = match serde::Deserialize::deserialize(value) {
        Ok(display) => display,
        Err(error) => {
            warnings.push(format!(
                "session.display_variant: invalid exact launch-settings object ({error}); treated as unobtained"
            ));
            return Leaf::Unobtained;
        }
    };
    let scale = f32::from_bits(display.gui_scale_f32_bits);
    let expected_mode = match (display.fullscreen, display.borderless) {
        (true, false) => "fullscreen",
        (false, true) => "borderless_fullscreen",
        (false, false) => "windowed",
        (true, true) => {
            warnings.push("session.display_variant: ambiguous fullscreen/borderless flags; treated as unobtained".into());
            return Leaf::Unobtained;
        }
    };
    let sha_ok = display.settings_file_sha256.len() == 64
        && display
            .settings_file_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if display.physical_width == 0
        || display.physical_height == 0
        || display.min_gui_width == 0
        || display.min_gui_height == 0
        || display.refresh_rate_hz == 0
        || !scale.is_finite()
        || scale <= 0.0
        || display.display_mode != expected_mode
        || !sha_ok
    {
        warnings.push("session.display_variant: values violate exact launch-settings contract; treated as unobtained".into());
        Leaf::Unobtained
    } else {
        Leaf::Measured(display)
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
                let speed = leaf_i64(obj, &["speed"], &ctx, warnings);
                if matches!(&speed, Leaf::Measured(raw) if !(0..=4).contains(raw)) {
                    warnings.push(format!(
                        "{ctx}.speed: raw engine slot outside 0..=4; whole list treated as unobtained"
                    ));
                    return Leaf::Unobtained;
                }
                events.push(SpeedEvent {
                    hour_index: leaf_i64(obj, &["hour_index"], &ctx, warnings),
                    speed,
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
    let (obj, status) = object_at_path(rest, &["naval_supremacy"], "naval_supremacy", warnings);
    match obj {
        Some(obj) => {
            return (
                parse_naval_supremacy_map(obj, "naval_supremacy", warnings),
                status,
            )
        }
        None if status == ViewStatus::DomainUnavailable => return (BTreeMap::new(), status),
        // DomainAbsent: fall through to the naval_air.region_control fallback.
        None => {}
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
            Value::Array(items) => {
                let object_count = items.iter().filter(|item| item.is_object()).count();
                if object_count > 1 {
                    warnings.push(format!(
                        "naval_air.region_control.{key}: array has {object_count} object elements; first used, extras ignored"
                    ));
                }
                items.iter().find_map(Value::as_object)
            }
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
        } else {
            // Array-shaped row with no object element: surfaced as an
            // all-unobtained placeholder, never silently dropped.
            warnings.push(format!(
                "naval_air.region_control.{key}: array contains no object element; treated as unobtained"
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

fn build_production_render(
    rest: &Map<String, Value>,
    frame_hour: i64,
    warnings: &mut Vec<String>,
) -> (Option<ProductionRenderFrame>, ViewStatus) {
    let Some(value) = rest.get("production_render") else {
        return (None, ViewStatus::DomainAbsent);
    };
    if value.is_null() {
        return (None, ViewStatus::DomainUnavailable);
    }
    match ProductionRenderFrame::decode_value(value) {
        Ok(frame) => {
            if let Some(pass) = &frame.authenticated_ui_pass {
                let runtime_text =
                    match crate::runtime_text::admit_runtime_text(rest.get("runtime_text")) {
                        Ok(runtime_text) => runtime_text,
                        Err(error) => {
                            warnings.push(format!("production_render runtime-text join: {error}"));
                            return (None, ViewStatus::DomainUnavailable);
                        }
                    };
                if let Err(error) = crate::runtime_text::validate_ui_pass_runtime_text_bindings(
                    pass,
                    &runtime_text,
                    frame_hour,
                ) {
                    warnings.push(format!("production_render runtime-text join: {error}"));
                    return (None, ViewStatus::DomainUnavailable);
                }
            }
            (Some(frame), ViewStatus::Present)
        }
        Err(error) => {
            warnings.push(format!("production_render: {error}"));
            (None, ViewStatus::DomainUnavailable)
        }
    }
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
                        // A3 round-trip: legacy/save snapshots mirror the
                        // cosmetic tag under `extra` or `misc`, while domain 18
                        // compact folding writes the canonical root leaf.
                        cosmetic_tag: first_available_string([
                            leaf_string(obj, &["extra", "cosmetic_tag"], &ctx, warnings),
                            leaf_string(obj, &["misc", "cosmetic_tag"], &ctx, warnings),
                            leaf_string(obj, &["cosmetic_tag"], &ctx, warnings),
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
    let (wars, wars_status) = read_war_objects(rest, warnings);
    let (faction_members, faction_status) = match read_domain53_faction_roster(rest, warnings) {
        Some((_, modern_members, modern_status)) => (modern_members, modern_status),
        None => match read_faction_members(rest, warnings) {
            // No modern witness plus no legacy value is unavailable, not
            // positive knowledge of an empty roster.
            (Leaf::Absent, _) => (Leaf::Unobtained, ViewStatus::DomainUnavailable),
            legacy => legacy,
        },
    };
    status = combine_status(
        combine_status(combine_status(status, war_rel_status), wars_status),
        faction_status,
    );

    DiplomacyView {
        status,
        pairs,
        countries,
        war_relations,
        wars,
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

fn war_participation_breakdown(
    participant: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<WarParticipationBreakdown> {
    let breakdown = match participant.get("participation_breakdown") {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Unobtained,
        Some(Value::Object(value)) => value,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.participation_breakdown: expected object, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let bctx = format!("{ctx}.participation_breakdown");
    Leaf::Measured(WarParticipationBreakdown {
        land_damage: leaf_i64(breakdown, &["land_damage"], &bctx, warnings),
        occupation: leaf_i64(breakdown, &["occupation"], &bctx, warnings),
        air_damage: leaf_i64(breakdown, &["air_damage"], &bctx, warnings),
        strategic_bombing: leaf_i64(breakdown, &["strategic_bombing"], &bctx, warnings),
        sunk_ships: leaf_i64(breakdown, &["sunk_ships"], &bctx, warnings),
        sunk_convoys: leaf_i64(breakdown, &["sunk_convoys"], &bctx, warnings),
        casualties: leaf_i64(breakdown, &["casualties"], &bctx, warnings),
        lend_lease_given: leaf_i64(breakdown, &["lend_lease_given"], &bctx, warnings),
        lend_lease_received: leaf_i64(breakdown, &["lend_lease_received"], &bctx, warnings),
    })
}

fn war_participants(
    war: &Map<String, Value>,
    key: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<WarParticipantOverviewRow>> {
    let rows = match war.get(key) {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Unobtained,
        Some(Value::Array(rows)) => rows,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.{key}: expected array, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let mut out = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let Some(row) = row.as_object() else {
            warnings.push(format!(
                "{ctx}.{key}[{index}]: expected object; whole participant list treated as unobtained"
            ));
            return Leaf::Unobtained;
        };
        let rctx = format!("{ctx}.{key}[{index}]");
        out.push(WarParticipantOverviewRow {
            tag: leaf_string(row, &["tag"], &rctx, warnings),
            tag_db_id: leaf_i64(row, &["tag_db_id"], &rctx, warnings),
            participation_raw: leaf_i64(row, &["participation_raw"], &rctx, warnings),
            capitulation_progress_raw: leaf_i64(
                row,
                &["capitulation_progress_raw"],
                &rctx,
                warnings,
            ),
            breakdown: war_participation_breakdown(row, &rctx, warnings),
        });
    }
    Leaf::Measured(out)
}

fn read_war_objects(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (Leaf<Vec<WarObjectRow>>, ViewStatus) {
    let world = match rest.get("diplomacy_world") {
        None => return (Leaf::Absent, ViewStatus::DomainAbsent),
        Some(Value::Null) => return (Leaf::Unobtained, ViewStatus::DomainUnavailable),
        Some(Value::Object(world)) => world,
        Some(other) => {
            warnings.push(format!(
                "diplomacy_world: expected object, found {}; wars unavailable",
                json_type_name(other)
            ));
            return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
        }
    };
    let entries = match world.get("wars") {
        None => return (Leaf::Absent, ViewStatus::DomainAbsent),
        Some(Value::Null) => return (Leaf::Unobtained, ViewStatus::DomainUnavailable),
        Some(Value::Object(entries)) => entries,
        Some(other) => {
            warnings.push(format!(
                "diplomacy_world.wars: expected object, found {}; wars unavailable",
                json_type_name(other)
            ));
            return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
        }
    };
    let mut rows = Vec::with_capacity(entries.len());
    for (key, item) in entries {
        let Some(obj) = item.as_object() else {
            warnings.push(format!(
                "diplomacy_world.wars.{key}: expected object; wars unavailable"
            ));
            return (Leaf::Unobtained, ViewStatus::DomainUnavailable);
        };
        let ctx = format!("diplomacy_world.wars.{key}");
        rows.push(WarObjectRow {
            key: key.clone(),
            attackers: leaf_string_list(obj, &["attackers"], &ctx, warnings),
            defenders: leaf_string_list(obj, &["defenders"], &ctx, warnings),
            war_name: leaf_string_prefer(obj, &["war_name"], &["name"], &ctx, warnings),
            front: leaf_string(obj, &["front"], &ctx, warnings),
            start_hour: leaf_i64(obj, &["start_hour"], &ctx, warnings),
            attacker_participants: war_participants(obj, "attacker_participants", &ctx, warnings),
            defender_participants: war_participants(obj, "defender_participants", &ctx, warnings),
            overall_progress_raw: leaf_i64(obj, &["overall_progress_raw"], &ctx, warnings),
            first: leaf_string(obj, &["first"], &ctx, warnings),
            second: leaf_string(obj, &["second"], &ctx, warnings),
            first_tag_db_id: leaf_i64(obj, &["first_tag_db_id"], &ctx, warnings),
            second_tag_db_id: leaf_i64(obj, &["second_tag_db_id"], &ctx, warnings),
            start_date: leaf_string(obj, &["start_date"], &ctx, warnings),
            first_casualties: leaf_i64(obj, &["first_casualties"], &ctx, warnings),
            second_casualties: leaf_i64(obj, &["second_casualties"], &ctx, warnings),
            casualties_semantics: leaf_string(obj, &["casualties_semantics"], &ctx, warnings),
            casualties_is_save_snapshot: leaf_bool(
                obj,
                &["casualties_is_save_snapshot"],
                &ctx,
                warnings,
            ),
        });
    }
    (Leaf::Measured(rows), ViewStatus::Present)
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
    AceRosters,
}

const DOMAIN_PROBLEM_VIEW_MAP: &[(i64, ProblemBackedView)] = &[
    (11, ProblemBackedView::Production),
    (26, ProblemBackedView::DivisionUnits),
    (28, ProblemBackedView::Fleets),
    (45, ProblemBackedView::Focus),
    (46, ProblemBackedView::AceRosters),
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
    ace_rosters: &mut BTreeMap<String, AceRosterExactView>,
    ace_rosters_status: &mut ViewStatus,
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
            downgrade_ace_rosters(ace_rosters, ace_rosters_status);
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
                ProblemBackedView::AceRosters => {
                    downgrade_ace_rosters(ace_rosters, ace_rosters_status)
                }
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

fn downgrade_ace_rosters(
    rosters: &mut BTreeMap<String, AceRosterExactView>,
    status: &mut ViewStatus,
) {
    downgrade_present_status(status);
    for roster in rosters.values_mut() {
        downgrade_present_status(&mut roster.status);
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
    pub points_factor: Leaf<f64>,
    pub progress_points: Leaf<f64>,
    pub required_points_base: Leaf<f64>,
    pub remaining_points_base: Leaf<f64>,
    pub progress_pct: Leaf<f64>,
    pub eta_days: Leaf<i64>,
    pub progress_basis: Leaf<String>,
    pub progress_unavailable_reason: Leaf<String>,
    pub eta_unavailable_reason: Leaf<String>,
    pub ahead_of_time_years: Leaf<f64>,
    pub ahead_of_time_penalty_pct: Leaf<f64>,
    pub ahead_of_time_reduction_years: Leaf<f64>,
    pub ahead_of_time_reduced_penalty_pct: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResearchLiveProgressRow {
    pub slot_index: Leaf<i64>,
    pub tech_handle_raw: Leaf<i64>,
    pub tech_def_index_raw: Leaf<i64>,
    pub tech_name: Leaf<String>,
    pub progress_raw: Leaf<i64>,
    pub progress_units: Leaf<f64>,
    pub live_progress_raw: Leaf<i64>,
    pub live_progress_units: Leaf<f64>,
    pub base_required_points_raw: Leaf<i64>,
    pub base_required_points_units: Leaf<f64>,
    pub points_factor: Leaf<f64>,
    pub points_factor_elapsed_hours: Leaf<i64>,
    pub required_points_base: Leaf<f64>,
    pub remaining_points_base: Leaf<f64>,
    pub progress_pct: Leaf<f64>,
    pub eta_days: Leaf<i64>,
    pub progress_basis: Leaf<String>,
    pub progress_unavailable_reason: Leaf<String>,
    pub eta_unavailable_reason: Leaf<String>,
    pub ahead_of_time_years: Leaf<f64>,
    pub ahead_of_time_penalty_pct: Leaf<f64>,
    pub ahead_of_time_reduction_years: Leaf<f64>,
    pub ahead_of_time_reduced_penalty_pct: Leaf<f64>,
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
    pub in_progress_live: Leaf<Vec<ResearchLiveProgressRow>>,
    pub technologies: Leaf<Vec<ResearchTechnologyRow>>,
    pub doctrine: Leaf<DoctrineView>,
}

impl ResearchView {
    fn all_unobtained() -> ResearchView {
        ResearchView {
            tech_count: Leaf::Unobtained,
            research_slots: Leaf::Unobtained,
            in_progress: Leaf::Unobtained,
            in_progress_live: Leaf::Unobtained,
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
    pub current_set: Leaf<Vec<String>>,
    pub current_continuous: Leaf<String>,
    pub current_continuous_set: Leaf<Vec<String>>,
    pub completed_count: Leaf<i64>,
    pub progress: Leaf<f64>,
    pub completed: Leaf<Vec<String>>,
    pub available: Leaf<Vec<String>>,
    pub visible_order: Leaf<Vec<String>>,
    pub paused: Leaf<bool>,
}

impl FocusView {
    fn all_unobtained() -> FocusView {
        FocusView {
            focus_tree: Leaf::Unobtained,
            current: Leaf::Unobtained,
            current_set: Leaf::Unobtained,
            current_continuous: Leaf::Unobtained,
            current_continuous_set: Leaf::Unobtained,
            completed_count: Leaf::Unobtained,
            progress: Leaf::Unobtained,
            completed: Leaf::Unobtained,
            available: Leaf::Unobtained,
            visible_order: Leaf::Unobtained,
            paused: Leaf::Unobtained,
        }
    }
}

// --- 3. politics detail (REPLAY_SCHEMA §3.3) + command abilities (abilities) ---
#[derive(Debug, Clone, PartialEq)]
pub struct AppointedAdvisorRow {
    pub idea_token: Leaf<String>,
    pub slot: Leaf<String>,
    pub ledger: Leaf<String>,
    pub character: Leaf<String>,
}

/// One frame-authoritative command-ability entry.  Every display-facing value
/// remains a tri-state leaf so authored defaults never masquerade as the
/// selected leader's effective runtime state.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandAbilityRow {
    pub ability_id: Leaf<String>,
    pub available: Leaf<bool>,
    pub active: Leaf<bool>,
    pub activation_hour: Leaf<i64>,
    pub cooldown_hours: Leaf<i64>,
    pub cooldown_remaining_hours: Leaf<i64>,
    pub cost_command_power: Leaf<f64>,
    pub target_kind: Leaf<String>,
    pub target_id: Leaf<String>,
    pub duration_hours: Leaf<i64>,
    pub duration_remaining_hours: Leaf<i64>,
    pub display_order: Leaf<i64>,
    pub icon: Leaf<String>,
    pub tooltip: Leaf<String>,
    pub disabled_reason: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoliticsDetailView {
    pub stability: Leaf<f64>,
    pub war_support: Leaf<f64>,
    pub political_power: Leaf<f64>,
    pub command_power: Leaf<f64>,
    pub ruling_party: Leaf<String>,
    pub party_popularities: Leaf<BTreeMap<String, f64>>,
    pub ideas: Leaf<Vec<String>>,
    pub ideas_all: Leaf<Vec<String>>,
    pub ideas_other: Leaf<Vec<String>>,
    pub national_spirits: Leaf<Vec<String>>,
    pub idea_groups: Leaf<BTreeMap<String, Vec<String>>>,
    pub advisor_ideas: Leaf<BTreeMap<String, Vec<String>>>,
    /// The complete typed law object, preserving current live group names such
    /// as `trade_laws` and `mobilization_laws` as well as save aliases.
    pub laws: Leaf<BTreeMap<String, String>>,
    pub law_economy: Leaf<String>,
    pub law_trade: Leaf<String>,
    pub law_conscription: Leaf<String>,
    pub idea_count: Leaf<i64>,
    pub party_count: Leaf<i64>,
    pub election_frequency: Leaf<i64>,
    pub last_election: Leaf<String>,
    pub elections_allowed: Leaf<bool>,
    pub low_confidence: Leaf<bool>,
    pub appointed_advisors: Leaf<Vec<String>>,
    pub appointed_advisor_details: Leaf<Vec<AppointedAdvisorRow>>,
    pub appointed_advisor_slots: Leaf<BTreeMap<String, Vec<String>>>,
    pub appointed_advisor_ledgers: Leaf<BTreeMap<String, Vec<String>>>,
    /// Active command abilities (ability tokens) from `countries.<tag>.abilities`.
    pub command_abilities: Leaf<Vec<String>>,
    /// Exact selected-context ability rows from `countries.<tag>.active_abilities.rows`.
    pub command_ability_rows: Leaf<Vec<CommandAbilityRow>>,
    /// Distinguishes a proven-empty ability bar from an unreadable producer.
    pub command_abilities_complete: Leaf<bool>,
}

impl PoliticsDetailView {
    fn all_unobtained() -> PoliticsDetailView {
        PoliticsDetailView {
            stability: Leaf::Unobtained,
            war_support: Leaf::Unobtained,
            political_power: Leaf::Unobtained,
            command_power: Leaf::Unobtained,
            ruling_party: Leaf::Unobtained,
            party_popularities: Leaf::Unobtained,
            ideas: Leaf::Unobtained,
            ideas_all: Leaf::Unobtained,
            ideas_other: Leaf::Unobtained,
            national_spirits: Leaf::Unobtained,
            idea_groups: Leaf::Unobtained,
            advisor_ideas: Leaf::Unobtained,
            laws: Leaf::Unobtained,
            law_economy: Leaf::Unobtained,
            law_trade: Leaf::Unobtained,
            law_conscription: Leaf::Unobtained,
            idea_count: Leaf::Unobtained,
            party_count: Leaf::Unobtained,
            election_frequency: Leaf::Unobtained,
            last_election: Leaf::Unobtained,
            elections_allowed: Leaf::Unobtained,
            low_confidence: Leaf::Unobtained,
            appointed_advisors: Leaf::Unobtained,
            appointed_advisor_details: Leaf::Unobtained,
            appointed_advisor_slots: Leaf::Unobtained,
            appointed_advisor_ledgers: Leaf::Unobtained,
            command_abilities: Leaf::Unobtained,
            command_ability_rows: Leaf::Unobtained,
            command_abilities_complete: Leaf::Unobtained,
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

/// One military/naval production line. Every display-facing field is a
/// tri-state leaf: legacy d11 rows measure only the fields they actually
/// carry, while successor schemas can fill the exact identity, efficiency,
/// completeness and projection seams without replay-side guessing.
#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentProductionLineRow {
    pub id: Leaf<i64>,
    pub priority: Leaf<i64>,
    pub line_subtype: Leaf<String>,
    pub equipment_variant_id: Leaf<String>,
    pub active_factories: Leaf<i64>,
    pub requested_factories: Leaf<i64>,
    pub efficiency: Leaf<f64>,
    pub efficiency_cap: Leaf<f64>,
    pub efficiency_growth: Leaf<f64>,
    pub cost: Leaf<f64>,
    pub speed: Leaf<f64>,
    pub produced: Leaf<f64>,
    pub amount: Leaf<i64>,
    pub progress_ratio: Leaf<f64>,
    pub daily_output: Leaf<f64>,
    pub days_to_next_unit: Leaf<f64>,
    pub mio_id: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentStock {
    pub archetype: Leaf<String>,
    pub amount: Leaf<f64>,
}

/// B2 exact country convoy-bank row. `amount_raw` is the serializer-proven
/// fixed-5 value; `amount` is its lossless replay projection at scale 100000.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvoyVariantAmount {
    pub archetype: Leaf<String>,
    pub equip_id: Leaf<i64>,
    pub equip_type: Leaf<i64>,
    pub amount_raw: Leaf<i64>,
    pub amount: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConvoyPriorityReservation {
    pub priority: Leaf<i64>,
    pub count: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductionView {
    pub stockpile_total: Leaf<f64>,
    pub stockpile: Leaf<BTreeMap<String, f64>>,
    pub stockpile_complete: Leaf<bool>,
    pub military_lines: Leaf<Vec<EquipmentProductionLineRow>>,
    pub military_lines_complete: Leaf<bool>,
    pub naval_lines: Leaf<Vec<EquipmentProductionLineRow>>,
    pub naval_lines_complete: Leaf<bool>,
    pub construction_queue: Leaf<Vec<ProductionLineRow>>,
    pub equipment_stockpile: Leaf<Vec<EquipmentStock>>,
    pub convoys_complete: Leaf<bool>,
    pub convoys_owned_raw: Leaf<i64>,
    pub convoys_owned: Leaf<f64>,
    pub convoys_assigned: Leaf<i64>,
    pub convoys_available: Leaf<i64>,
    pub convoys_cache_valid: Leaf<bool>,
    pub convoy_variants: Leaf<Vec<ConvoyVariantAmount>>,
    pub convoy_priority_reservations_complete: Leaf<bool>,
    pub convoy_priority_reservations: Leaf<Vec<ConvoyPriorityReservation>>,
}

impl ProductionView {
    fn all_unobtained() -> ProductionView {
        ProductionView {
            stockpile_total: Leaf::Unobtained,
            stockpile: Leaf::Unobtained,
            stockpile_complete: Leaf::Unobtained,
            military_lines: Leaf::Unobtained,
            military_lines_complete: Leaf::Unobtained,
            naval_lines: Leaf::Unobtained,
            naval_lines_complete: Leaf::Unobtained,
            construction_queue: Leaf::Unobtained,
            equipment_stockpile: Leaf::Unobtained,
            convoys_complete: Leaf::Unobtained,
            convoys_owned_raw: Leaf::Unobtained,
            convoys_owned: Leaf::Unobtained,
            convoys_assigned: Leaf::Unobtained,
            convoys_available: Leaf::Unobtained,
            convoys_cache_valid: Leaf::Unobtained,
            convoy_variants: Leaf::Unobtained,
            convoy_priority_reservations_complete: Leaf::Unobtained,
            convoy_priority_reservations: Leaf::Unobtained,
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

/// Exact raw D53 agency leaves that are already production-carried. The
/// unresolved names are intentional: their final UI semantics remain A30 RE
/// inputs and must not be inferred from the raw offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct Domain53AgencyScalarsRaw {
    pub max_operatives_unresolved: Leaf<i64>,
    pub usable_slots_unresolved: Leaf<i64>,
    pub operative_count: Leaf<i64>,
    pub cryptology_target_count: Leaf<i64>,
    /// Legacy carrier key `defense_raw`; neutralized until A30 reconciles the
    /// +0x128 serialized representation and scale.
    pub value_0x128_unvalidated: Leaf<i64>,
    /// Legacy carrier key `upgrade_progress_raw`; neutralized until A30
    /// reconciles the +0xC8 optional/object serialization path.
    pub value_0xc8_unvalidated: Leaf<i64>,
    pub name_hash: Leaf<u64>,
    pub icon_hash: Leaf<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Domain53AgencyCapacityRaw {
    pub value_0xe4: Leaf<i64>,
    pub value_0xf0: Leaf<i64>,
    pub value_0xf4: Leaf<i64>,
    pub proof_hash: Leaf<u64>,
    pub semantic_status: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Domain53OperativeLifecycleRawRow {
    pub op_id: Leaf<i64>,
    pub detected: Leaf<bool>,
    pub captured_unvalidated: Leaf<bool>,
    pub skill: Leaf<i64>,
    pub experience_raw: Leaf<i64>,
    pub name_hash: Leaf<u64>,
    pub state_hash: Leaf<u64>,
    pub lifecycle_flag: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Domain53CryptologyTargetRawRow {
    pub target_tag_db_id: Leaf<i64>,
    pub amount_raw: Leaf<i64>,
    pub decrypting: Leaf<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntelView {
    pub agency_name: Leaf<String>,
    pub agency_created: Leaf<bool>,
    pub agency_upgrade_progress: Leaf<f64>,
    pub operatives: Leaf<Vec<OperativeRow>>,
    pub operations_running: Leaf<Vec<OperationRow>>,
    /// Typed access to the already-extracted, deliberately non-semantic D53
    /// sidecar. These are reusable A30 inputs, not a complete UI frame.
    pub domain53_agency_scalars_raw: Leaf<Domain53AgencyScalarsRaw>,
    pub domain53_agency_capacity_raw: Leaf<Domain53AgencyCapacityRaw>,
    pub domain53_operative_lifecycle_raw: Leaf<Vec<Domain53OperativeLifecycleRawRow>>,
    pub domain53_cryptology_targets_raw: Leaf<Vec<Domain53CryptologyTargetRawRow>>,
}

impl IntelView {
    fn all_unobtained() -> IntelView {
        IntelView {
            agency_name: Leaf::Unobtained,
            agency_created: Leaf::Unobtained,
            agency_upgrade_progress: Leaf::Unobtained,
            operatives: Leaf::Unobtained,
            operations_running: Leaf::Unobtained,
            domain53_agency_scalars_raw: Leaf::Unobtained,
            domain53_agency_capacity_raw: Leaf::Unobtained,
            domain53_operative_lifecycle_raw: Leaf::Unobtained,
            domain53_cryptology_targets_raw: Leaf::Unobtained,
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

/// One controller-materialized resource cell in `countrytradeview`.
///
/// Array order is preserved and `visible_order` authenticates that order.  The
/// replay never reconstructs these display roles from a balance formula: the
/// engine supplies each semantic value independently.
#[derive(Debug, Clone, PartialEq)]
pub struct TradeResourceRow {
    pub resource: Leaf<String>,
    pub visible_order: Leaf<i64>,
    pub balance: Leaf<f64>,
    pub produced: Leaf<f64>,
    pub imported: Leaf<f64>,
    pub exported: Leaf<f64>,
    pub special_project: Leaf<f64>,
    pub production_line: Leaf<f64>,
    pub needed: Leaf<f64>,
    /// Oil-only first line written to the `needed` widget. Unobtained for
    /// non-oil resources; retained separately because the engine writes two
    /// numeric lines for oil.
    pub needed_current: Leaf<f64>,
    /// Oil-only second line written to the `needed` widget.
    pub needed_max: Leaf<f64>,
    pub needed_type: Leaf<String>,
}

/// One classic resource-trade deal (not an equipment-market purchase
/// contract).  All amounts and costs are recorded semantic inputs; no replay
/// trade simulation or route estimation is permitted.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceTradeDealRow {
    pub deal_id: Leaf<i64>,
    pub counterpart: Leaf<String>,
    pub direction: Leaf<String>,
    pub resource: Leaf<String>,
    pub requested: Leaf<f64>,
    pub delivered: Leaf<f64>,
    pub available_export: Leaf<f64>,
    pub civilian_factories: Leaf<i64>,
    pub factory_cost_per_resource: Leaf<f64>,
    pub convoys_assigned: Leaf<i64>,
    pub convoys_required: Leaf<i64>,
    pub route_kind: Leaf<String>,
    pub delivery_efficiency: Leaf<f64>,
    /// Exact route+0xE8 flag. Optional in older recordings.
    pub no_trade_influence: Leaf<bool>,
    /// Exact route+0xE9 flag. Optional in older recordings.
    pub no_resources: Leaf<bool>,
    pub visible_order: Leaf<i64>,
}

const TRADE_RESOURCE_ORDER: [&str; 7] = [
    "oil",
    "aluminium",
    "rubber",
    "tungsten",
    "steel",
    "chromium",
    "coal",
];

#[derive(Debug, Clone, PartialEq)]
pub struct TradeView {
    pub resource_pools_complete: Leaf<bool>,
    pub produced: Leaf<BTreeMap<String, f64>>,
    pub imported: Leaf<BTreeMap<String, f64>>,
    pub used: Leaf<BTreeMap<String, f64>>,
    pub exported: Leaf<BTreeMap<String, f64>>,
    pub to_export: Leaf<BTreeMap<String, f64>>,
    pub base_export: Leaf<BTreeMap<String, f64>>,
    pub available_derived: Leaf<BTreeMap<String, f64>>,
    pub resource_rows: Leaf<Vec<TradeResourceRow>>,
    pub resource_rows_complete: Leaf<bool>,
    pub resource_deals: Leaf<Vec<ResourceTradeDealRow>>,
    pub resource_deals_complete: Leaf<bool>,
    pub factories_total: Leaf<i64>,
    pub factories_used: Leaf<i64>,
    pub factories_available: Leaf<i64>,
    pub contracts: Leaf<Vec<TradeContractRow>>,
}

impl TradeView {
    fn all_unobtained() -> TradeView {
        TradeView {
            resource_pools_complete: Leaf::Unobtained,
            produced: Leaf::Unobtained,
            imported: Leaf::Unobtained,
            used: Leaf::Unobtained,
            exported: Leaf::Unobtained,
            to_export: Leaf::Unobtained,
            base_export: Leaf::Unobtained,
            available_derived: Leaf::Unobtained,
            resource_rows: Leaf::Unobtained,
            resource_rows_complete: Leaf::Unobtained,
            resource_deals: Leaf::Unobtained,
            resource_deals_complete: Leaf::Unobtained,
            factories_total: Leaf::Unobtained,
            factories_used: Leaf::Unobtained,
            factories_available: Leaf::Unobtained,
            contracts: Leaf::Unobtained,
        }
    }

    /// Total imported resources displayed by the engine.  This is available
    /// only for a complete measured resource-row set.
    pub fn computed_imported_total(&self) -> Leaf<f64> {
        self.sum_complete_resource_rows(|row| &row.imported)
    }

    /// Total exported resources displayed by the engine.  This is available
    /// only for a complete measured resource-row set.
    pub fn computed_exported_total(&self) -> Leaf<f64> {
        self.sum_complete_resource_rows(|row| &row.exported)
    }

    pub fn computed_convoys_assigned_total(&self) -> Leaf<i64> {
        self.sum_complete_deal_i64(|row| &row.convoys_assigned)
    }

    pub fn computed_convoys_required_total(&self) -> Leaf<i64> {
        self.sum_complete_deal_i64(|row| &row.convoys_required)
    }

    pub fn computed_requested_total(&self) -> Leaf<f64> {
        self.sum_complete_deal_f64(|row| &row.requested)
    }

    pub fn computed_delivered_total(&self) -> Leaf<f64> {
        self.sum_complete_deal_f64(|row| &row.delivered)
    }

    pub fn computed_civilian_factory_total(&self) -> Leaf<i64> {
        self.sum_complete_deal_i64(|row| &row.civilian_factories)
    }

    /// Unique partners in measured visible deal order.  No alphabetical or
    /// country-id sort is introduced by replay.
    pub fn computed_partners(&self) -> Leaf<Vec<String>> {
        match (&self.resource_deals_complete, &self.resource_deals) {
            (Leaf::Measured(true), Leaf::Measured(rows)) => {
                let mut seen = BTreeSet::new();
                let mut partners = Vec::new();
                for row in rows {
                    let Leaf::Measured(counterpart) = &row.counterpart else {
                        return Leaf::Unobtained;
                    };
                    if seen.insert(counterpart.clone()) {
                        partners.push(counterpart.clone());
                    }
                }
                Leaf::Measured(partners)
            }
            (Leaf::Absent, _) | (_, Leaf::Absent) => Leaf::Absent,
            _ => Leaf::Unobtained,
        }
    }

    fn sum_complete_resource_rows(
        &self,
        select: impl Fn(&TradeResourceRow) -> &Leaf<f64>,
    ) -> Leaf<f64> {
        match (&self.resource_rows_complete, &self.resource_rows) {
            (Leaf::Measured(true), Leaf::Measured(rows)) => {
                let mut total = 0.0;
                for row in rows {
                    let Leaf::Measured(value) = select(row) else {
                        return Leaf::Unobtained;
                    };
                    total += value;
                }
                Leaf::Measured(total)
            }
            (Leaf::Absent, _) | (_, Leaf::Absent) => Leaf::Absent,
            _ => Leaf::Unobtained,
        }
    }

    fn sum_complete_deal_i64(
        &self,
        select: impl Fn(&ResourceTradeDealRow) -> &Leaf<i64>,
    ) -> Leaf<i64> {
        match (&self.resource_deals_complete, &self.resource_deals) {
            (Leaf::Measured(true), Leaf::Measured(rows)) => {
                let mut total = 0i64;
                for row in rows {
                    let Leaf::Measured(value) = select(row) else {
                        return Leaf::Unobtained;
                    };
                    let Some(next) = total.checked_add(*value) else {
                        return Leaf::Unobtained;
                    };
                    total = next;
                }
                Leaf::Measured(total)
            }
            (Leaf::Absent, _) | (_, Leaf::Absent) => Leaf::Absent,
            _ => Leaf::Unobtained,
        }
    }

    fn sum_complete_deal_f64(
        &self,
        select: impl Fn(&ResourceTradeDealRow) -> &Leaf<f64>,
    ) -> Leaf<f64> {
        match (&self.resource_deals_complete, &self.resource_deals) {
            (Leaf::Measured(true), Leaf::Measured(rows)) => {
                let mut total = 0.0;
                for row in rows {
                    let Leaf::Measured(value) = select(row) else {
                        return Leaf::Unobtained;
                    };
                    total += value;
                }
                Leaf::Measured(total)
            }
            (Leaf::Absent, _) | (_, Leaf::Absent) => Leaf::Absent,
            _ => Leaf::Unobtained,
        }
    }
}

// --- 7. occupation resistance/compliance (REPLAY_SCHEMA §4, state-scoped) ---
#[derive(Debug, Clone, PartialEq)]
pub struct OccupationEquipmentExactRow {
    pub source_ordinal: Leaf<i64>,
    pub equipment_id: Leaf<u32>,
    pub equipment_type: Leaf<u32>,
    pub equipment_token: Leaf<String>,
    pub amount: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OccupationManpowerExactRow {
    pub source_ordinal: Leaf<i64>,
    pub tag: Leaf<String>,
    pub value: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GarrisonReinforcementExactRow {
    pub source_ordinal: Leaf<i64>,
    pub total_progress: Leaf<f64>,
    pub need: Leaf<BTreeMap<String, f64>>,
    pub date: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OccupationHistoryExactRow {
    pub stable_event_id: Leaf<String>,
    pub source_ordinal: Leaf<i64>,
    pub controller_tag: Leaf<String>,
    pub state_id: Leaf<u32>,
    pub occupied_country_tag: Leaf<String>,
    pub event_hour_or_engine_date: Leaf<String>,
    pub attack_blocked: Leaf<bool>,
    pub resistance_activity: Leaf<String>,
    pub lost_manpower: Leaf<i64>,
    pub equipment_loss_rows: Leaf<Vec<OccupationEquipmentExactRow>>,
    pub equipment_loss_children_complete: Leaf<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OccupationRow {
    pub resistance: Leaf<f64>,
    pub resistance_speed: Leaf<f64>,
    pub compliance: Leaf<f64>,
    pub compliance_speed: Leaf<f64>,
    pub occupier_tag: Leaf<String>,
    pub occupied_country_tag: Leaf<String>,
    pub operational_status: Leaf<String>,
    pub occupation_law: Leaf<String>,
    /// Exact FNV-1a-32 law token from domain 66 when present, otherwise the
    /// domain-52 per-state/default token retained in its authenticated sidecar.
    pub occupation_law_token_id: Leaf<u32>,
    pub occupation_law_token_encoding: Leaf<String>,
    pub occupation_law_token_source: Leaf<String>,
    pub occupation_law_status: Leaf<String>,
    pub garrison_template_id: Leaf<u32>,
    pub garrison_army_manpower_need: Leaf<i64>,
    pub garrison_strength_ratio: Leaf<f64>,
    pub garrison_required: Leaf<f64>,
    pub garrison_signals_status: Leaf<String>,
    pub garrison_manpower_source_tag_db_id: Leaf<u32>,
    pub occupation_controller_tag_db_id: Leaf<u32>,
    pub occupied_tag_db_id: Leaf<u32>,
    pub garrison_manpower_pool: Leaf<Vec<OccupationManpowerExactRow>>,
    pub garrison_equipment: Leaf<Vec<OccupationEquipmentExactRow>>,
    pub garrison_equipment_complete: Leaf<bool>,
    pub garrison_equipment_need: Leaf<BTreeMap<String, f64>>,
    pub garrison_reinforcement_requests: Leaf<Vec<GarrisonReinforcementExactRow>>,
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

// --- 8b. lossless live character/leader/ace sidecars (XUI17) ---
//
// These views expose only fields already authenticated by the domain-47 and
// domain-46 recorder folds. `producer_ordinal` preserves the full-snapshot
// array order for diagnostics; it is deliberately not named or treated as a
// UI-visible ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterIdentityExactRow {
    pub producer_ordinal: usize,
    pub char_id: u64,
    pub name: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryLeaderRoleExactRow {
    pub producer_ordinal: usize,
    pub char_id: u64,
    pub name: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRulerExactRow {
    pub producer_ordinal: usize,
    pub char_id: u64,
    pub ruling_party: String,
    pub name: String,
    pub token: String,
    pub leader_vec_size: u64,
    pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitLeaderAssignmentKind {
    ArmyGroup,
    Army,
    Fleet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitLeaderAssignmentExact {
    pub kind: UnitLeaderAssignmentKind,
    pub formation_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitLeaderExactRow {
    pub producer_ordinal: usize,
    pub unit_leader_id: u64,
    pub char_id: u64,
    pub leader_kind: String,
    pub leader_kind_raw: u64,
    pub skill: i64,
    pub experience_raw: i64,
    pub attack: i64,
    pub defense: i64,
    pub planning: i64,
    pub logistics: i64,
    pub maneuvering: i64,
    pub coordination: i64,
    /// Exact CUnitLeader trait-vector order, reconstructed from `ordinal`.
    pub traits: Vec<String>,
    /// Exact d28 OOB join by `unit_leader_id`. `None` means reserve only when
    /// the containing view's `command_assignments_complete` is true.
    pub assignment: Option<UnitLeaderAssignmentExact>,
    pub token: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRosterExactView {
    pub status: ViewStatus,
    /// All three d28 command arrays were present, well typed, and unique.
    /// When false, `UnitLeaderExactRow::assignment == None` is unavailable,
    /// not evidence that the leader is in reserve.
    pub command_assignments_complete: bool,
    pub characters: Vec<CharacterIdentityExactRow>,
    pub unit_leaders: Vec<UnitLeaderExactRow>,
    pub country_leader_roles: Vec<CountryLeaderRoleExactRow>,
    pub active_rulers: Vec<ActiveRulerExactRow>,
}

impl CharacterRosterExactView {
    fn unavailable() -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            command_assignments_complete: false,
            characters: Vec::new(),
            unit_leaders: Vec::new(),
            country_leader_roles: Vec::new(),
            active_rulers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AceLifecycleState {
    Alive,
    DeadUnhandled,
    DeadHandled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AceExactRow {
    pub ace_id: u64,
    /// Exact per-country roster-vector index authenticated by domain 46.
    pub country_ace_index: u64,
    pub name: String,
    pub surname: String,
    pub callsign: String,
    pub alive: bool,
    /// Serializer-owned `CAceEntry+0x16C`; absent on trailer-v1 captures.
    pub handled: Option<bool>,
    /// Serializer-owned raw `CAceEntry+0x15C`; intentionally not relabelled
    /// as an enum until more than raw value 1 is authenticated.
    pub kill_type_raw: Option<u64>,
    /// Mechanical projection of (`alive`, `handled`), never visual inference.
    pub lifecycle: Option<AceLifecycleState>,
    pub wing_id: u64,
    pub killer_ace_id: u64,
    pub killer_country_tag_db_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AceRosterExactView {
    pub status: ViewStatus,
    pub wing_assignment_complete: bool,
    pub aces: Vec<AceExactRow>,
}

impl AceRosterExactView {
    fn unavailable() -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            wing_assignment_complete: false,
            aces: Vec::new(),
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
    pub generation: Leaf<i64>,
    pub location_province: Leaf<u32>,
    pub region_id: Leaf<u32>,
    pub attacker: Leaf<String>,
    pub defender: Leaf<String>,
    pub result_type: Leaf<String>,
    pub progress: Leaf<f64>,
    pub attacker_member_count: Leaf<u32>,
    pub defender_member_count: Leaf<u32>,
    pub attacker_positioning: Leaf<f64>,
    pub defender_positioning: Leaf<f64>,
    pub attacker_task_force_ids: Leaf<Vec<i64>>,
    pub defender_task_force_ids: Leaf<Vec<i64>>,
    /// Exact sorted/deduplicated owner ids derived only from authenticated D50
    /// member rows. A missing/unknown side or owner fails the affected set
    /// closed; it is never inferred from task-force membership or diplomacy.
    pub attacker_country_tag_db_ids: Leaf<Vec<i64>>,
    pub defender_country_tag_db_ids: Leaf<Vec<i64>>,
    pub members: Leaf<Vec<NavalCombatMemberRow>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavalCombatMemberRow {
    pub side: Leaf<String>,
    pub group_index: Leaf<i64>,
    pub member_index: Leaf<i64>,
    pub is_convoy: Leaf<bool>,
    pub ship_instance_id: Leaf<i64>,
    pub cship_join_validated: Leaf<bool>,
    pub owner_tag_db_id: Leaf<i64>,
    pub task_force_id: Leaf<i64>,
    pub fleet_id: Leaf<i64>,
    pub battle_slot_index: Leaf<i64>,
    pub state_bitmask: Leaf<i64>,
    pub strength: Leaf<f64>,
    pub organisation: Leaf<f64>,
    pub sunk_convoys: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConvoyLossRow {
    pub killer: Leaf<String>,
    pub owner: Leaf<String>,
    pub convoys: Leaf<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AirCombatLossRow {
    pub owner: Leaf<String>,
    pub owner_tag_db_id: Leaf<i64>,
    pub wing_id: Leaf<i64>,
    pub generation: Leaf<i64>,
    pub mission_mask: Leaf<i64>,
    pub mission_region: Leaf<u32>,
    pub old_count: Leaf<i64>,
    pub new_count: Leaf<i64>,
    pub count_delta: Leaf<i64>,
    pub old_equipment_amount_sum: Leaf<f64>,
    pub new_equipment_amount_sum: Leaf<f64>,
    pub equipment_amount_delta: Leaf<f64>,
    pub flags: Leaf<i64>,
    pub air_accidents: Leaf<i64>,
    pub experience: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AirRegionDamageRow {
    pub channel: Leaf<String>,
    pub cause_type: Leaf<i64>,
    pub cause: Leaf<String>,
    pub amount: Leaf<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AirRegionCombatRow {
    pub owner: Leaf<String>,
    pub owner_tag_db_id: Leaf<i64>,
    pub region_id: Leaf<u32>,
    pub side_vector: Leaf<i64>,
    pub selection_flag: Leaf<i64>,
    pub classification_flag: Leaf<i64>,
    pub copied_flag: Leaf<i64>,
    pub wing_id: Leaf<i64>,
    pub date_raw: Leaf<i64>,
    pub plane_count: Leaf<i64>,
    pub mission_mask: Leaf<i64>,
    pub attributed_country: Leaf<String>,
    pub attributed_country_tag_db_id: Leaf<i64>,
    pub damage: Leaf<Vec<AirRegionDamageRow>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavalAirCombatView {
    pub status: ViewStatus,
    pub naval_combats: Vec<NavalCombatRow>,
    pub sunk_convoys: Leaf<Vec<ConvoyLossRow>>,
    pub air_losses: Leaf<Vec<AirCombatLossRow>>,
    pub air_region_combat: Leaf<Vec<AirRegionCombatRow>>,
}

// --- 12. scripted-GUI variable state (REPLAY_SCHEMA §3 script_variables / §18) ---
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptedGuiView {
    pub status: ViewStatus,
    /// Per-country dynamic script variables (`countries.<tag>.script_variables`).
    pub by_country: BTreeMap<String, Leaf<BTreeMap<String, f64>>>,
    /// Per-country named flags from the production d51 fold
    /// (`countries.<tag>.extra.script_flags`).
    pub country_flags: BTreeMap<String, Leaf<Vec<String>>>,
    /// Engine-resolved inputs for scripted constructs that cannot be evaluated
    /// from the recorded variable/flag subset. Keyed by country then GUI id.
    pub runtime_by_country: BTreeMap<String, Leaf<BTreeMap<String, ScriptedGuiRuntimeInputs>>>,
    pub world_variables: Leaf<BTreeMap<String, f64>>,
    /// Active global flag names (`world.flags` keys), sorted.
    pub world_flags: Leaf<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptedGuiRuntimeInputs {
    pub visible: Leaf<bool>,
    pub enabled: Leaf<bool>,
    pub triggers: BTreeMap<String, bool>,
    /// Flattened `<element>.<property>` keys with exact engine-resolved values.
    pub properties: BTreeMap<String, String>,
    pub effect_enabled: BTreeMap<String, bool>,
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

// --- XUI21: fired-event history and popup producer inputs -----------------
//
// The save-era `world_events.fired_events = ["namespace.id", ...]` ledger is
// intentionally still accepted.  Such legacy rows expose only `event_id`; all
// live presentation leaves remain `Absent`, so production admission cannot
// mistake a historical id set for an ordered fired-event stream.
#[derive(Debug, Clone, PartialEq)]
pub struct FiredEventOptionTextContextRow {
    pub definition_ordinal: Leaf<i64>,
    pub context_id: Leaf<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FiredEventRow {
    pub event_id: Leaf<String>,
    pub recipient_tag: Leaf<String>,
    pub fire_date: Leaf<String>,
    pub fire_hour: Leaf<i64>,
    /// Absolute producer tick.  Replay code never converts hours to ticks.
    pub fire_tick: Leaf<i64>,
    /// Monotone producer ordinal; array order must agree with this value.
    pub event_sequence: Leaf<i64>,
    /// Definition option ordinals visible when the popup was materialised.
    pub visible_option_ordinals: Leaf<Vec<i64>>,
    /// Definition option ordinal selected by the human/AI, if resolved.
    pub chosen_option_ordinal: Leaf<i64>,
    pub selected_desc_key: Leaf<String>,
    pub shown_picture_gfx: Leaf<String>,
    pub popup_subtype: Leaf<String>,
    pub effective_timeout_hours: Leaf<i64>,
    pub timeout_tick: Leaf<i64>,
    pub resolved_tick: Leaf<i64>,
    pub resolution_reason: Leaf<String>,
    /// Independent joins into the generic XUI06/A25 runtime-text carrier.
    /// One singular context cannot authenticate multiple textbox source ids.
    pub runtime_title_text_context_id: Leaf<String>,
    pub runtime_description_text_context_id: Leaf<String>,
    pub runtime_option_text_contexts: Leaf<Vec<FiredEventOptionTextContextRow>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FiredEventsView {
    pub events: Leaf<Vec<FiredEventRow>>,
    /// A measured-empty list is authoritative only when this witness is true.
    pub history_complete: Leaf<bool>,
}

impl FiredEventsView {
    fn with_status(status: ViewStatus) -> Self {
        let leaf = match status {
            ViewStatus::DomainAbsent => Leaf::Absent,
            _ => Leaf::Unobtained,
        };
        Self {
            events: leaf,
            history_complete: match status {
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
    pub fn character_rosters_exact(&self) -> &BTreeMap<String, CharacterRosterExactView> {
        &self.character_rosters_exact
    }
    pub fn character_roster_exact(&self, tag: &str) -> Option<&CharacterRosterExactView> {
        self.character_rosters_exact.get(tag)
    }
    pub fn character_rosters_exact_status(&self) -> ViewStatus {
        self.character_rosters_exact_status
    }
    pub fn ace_rosters_exact(&self) -> &BTreeMap<String, AceRosterExactView> {
        &self.ace_rosters_exact
    }
    pub fn ace_roster_exact(&self, tag: &str) -> Option<&AceRosterExactView> {
        self.ace_rosters_exact.get(tag)
    }
    pub fn ace_rosters_exact_status(&self) -> ViewStatus {
        self.ace_rosters_exact_status
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
    pub fn occupation_history(&self) -> &Leaf<Vec<OccupationHistoryExactRow>> {
        &self.occupation_history
    }
    pub fn occupation_history_complete(&self) -> &Leaf<bool> {
        &self.occupation_history_complete
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
    pub fn dynamic_collection_trace(&self) -> &DynamicCollectionTraceView {
        &self.dynamic_collection_trace
    }
    pub fn dynamic_collection_trace_status(&self) -> ViewStatus {
        self.dynamic_collection_trace.status
    }
    pub fn design_catalog_context(&self) -> &DesignCatalogContextView {
        &self.design_catalog_context
    }
    pub fn design_catalog_context_status(&self) -> ViewStatus {
        self.design_catalog_context.status
    }
    pub fn hud_surface_activation(&self) -> &HudSurfaceActivationView {
        &self.hud_surface_activation
    }
    pub fn hud_surface_activation_status(&self) -> ViewStatus {
        self.hud_surface_activation.status
    }
    /// Complete pointer-free A27 live catalog carried once by the replay.
    /// Its vector order is captured heap-scan provenance, not UI display order.
    pub fn design_catalog(&self) -> Option<&PortableDesignCatalog> {
        self.design_catalog.as_deref()
    }
    /// Exact current equipment-designer rows: record identities come from the
    /// A27 live catalog and order comes only from d78's displayed row vector.
    /// Domain 59 authored-definition order and A27 heap-scan order are never
    /// substituted for UI order.
    pub fn current_design_catalog(&self) -> Result<CurrentDesignCatalog, DesignCatalogJoinError> {
        let catalog = self
            .design_catalog()
            .ok_or(DesignCatalogJoinError::CatalogUnavailable)?;
        if self.design_catalog_context.status != ViewStatus::Present {
            return Err(DesignCatalogJoinError::ContextUnavailable);
        }
        let context = self
            .design_catalog_context
            .context
            .as_ref()
            .ok_or(DesignCatalogJoinError::ContextUnavailable)?;
        let mut seen = BTreeSet::new();
        let mut records = Vec::with_capacity(context.ordered_design_ids.len());
        let mut ordered_design_ids = Vec::with_capacity(context.ordered_design_ids.len());
        for id in &context.ordered_design_ids {
            if !seen.insert(*id) {
                return Err(DesignCatalogJoinError::DuplicateContextId(*id));
            }
            let stable_id = format!("{id}:70");
            let record = catalog
                .get(&stable_id)
                .ok_or(DesignCatalogJoinError::UnknownContextId(*id))?;
            if record.kind == DesignKind::DivisionTemplate {
                return Err(DesignCatalogJoinError::ContextSelectedDivisionTemplate(*id));
            }
            ordered_design_ids.push(stable_id);
            records.push(record.clone());
        }
        Ok(CurrentDesignCatalog {
            records,
            display_order: DisplayOrderSnapshot {
                source_record_id: context.completeness_record_identity.clone(),
                completeness: FieldCompleteness::Complete,
                ordered_design_ids,
            },
        })
    }
    pub fn raids(&self) -> &RaidStateView {
        &self.raids
    }
    pub fn raids_status(&self) -> ViewStatus {
        self.raids.status
    }
    pub fn fired_events(&self) -> &FiredEventsView {
        &self.fired_events
    }
    pub fn fired_events_status(&self) -> ViewStatus {
        self.fired_events_status
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

fn bool_map(
    obj: &Map<String, Value>,
    key: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Option<BTreeMap<String, bool>> {
    let value = obj.get(key)?;
    let Value::Object(map) = value else {
        warnings.push(format!(
            "{ctx}.{key}: expected object; scripted-GUI runtime input treated as unobtained"
        ));
        return None;
    };
    let mut out = BTreeMap::new();
    for (name, value) in map {
        let Some(value) = value.as_bool() else {
            warnings.push(format!(
                "{ctx}.{key}.{name}: expected bool; scripted-GUI runtime input treated as unobtained"
            ));
            return None;
        };
        out.insert(name.clone(), value);
    }
    Some(out)
}

fn property_string_map(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Option<BTreeMap<String, String>> {
    let Some(value) = obj.get("properties") else {
        return Some(BTreeMap::new());
    };
    let Value::Object(elements) = value else {
        warnings.push(format!(
            "{ctx}.properties: expected object; scripted-GUI runtime input treated as unobtained"
        ));
        return None;
    };
    let mut out = BTreeMap::new();
    for (element, value) in elements {
        let Value::Object(properties) = value else {
            warnings.push(format!(
                "{ctx}.properties.{element}: expected object; scripted-GUI runtime input treated as unobtained"
            ));
            return None;
        };
        for (property, value) in properties {
            let Some(value) = value.as_str() else {
                warnings.push(format!(
                    "{ctx}.properties.{element}.{property}: expected string; scripted-GUI runtime input treated as unobtained"
                ));
                return None;
            };
            out.insert(format!("{element}.{property}"), value.to_string());
        }
    }
    Some(out)
}

fn leaf_scripted_gui_runtime_inputs(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<BTreeMap<String, ScriptedGuiRuntimeInputs>> {
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Object(guis)) => {
            let mut out = BTreeMap::new();
            for (gui, value) in guis {
                let Value::Object(input) = value else {
                    warnings.push(format!(
                        "{ctx}.{}.{gui}: expected object; whole scripted-GUI runtime map treated as unobtained",
                        path.join(".")
                    ));
                    return Leaf::Unobtained;
                };
                let required = [
                    "visible",
                    "enabled",
                    "triggers",
                    "properties",
                    "effect_enabled",
                ];
                if input.len() != required.len()
                    || required.iter().any(|key| !input.contains_key(*key))
                {
                    warnings.push(format!(
                        "{ctx}.{}.{gui}: scripted-GUI runtime input has unknown or missing fields; whole map treated as unobtained",
                        path.join(".")
                    ));
                    return Leaf::Unobtained;
                }
                let rctx = format!("{ctx}.{}.{gui}", path.join("."));
                // The exact-keys check above guarantees "triggers" and
                // "effect_enabled" are present, so a None from bool_map always
                // means malformed, never missing.
                let Some(triggers) = bool_map(input, "triggers", &rctx, warnings) else {
                    return Leaf::Unobtained;
                };
                let Some(effect_enabled) = bool_map(input, "effect_enabled", &rctx, warnings)
                else {
                    return Leaf::Unobtained;
                };
                let Some(properties) = property_string_map(input, &rctx, warnings) else {
                    return Leaf::Unobtained;
                };
                out.insert(
                    gui.clone(),
                    ScriptedGuiRuntimeInputs {
                        visible: leaf_bool(input, &["visible"], &rctx, warnings),
                        enabled: leaf_bool(input, &["enabled"], &rctx, warnings),
                        triggers,
                        properties,
                        effect_enabled,
                    },
                );
            }
            Leaf::Measured(out)
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: expected object, found {}; treated as unobtained",
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
                    points_factor: leaf_f64(o, &["points_factor"], c, w),
                    progress_points: leaf_f64(o, &["progress_points"], c, w),
                    required_points_base: leaf_f64(o, &["required_points_base"], c, w),
                    remaining_points_base: leaf_f64(o, &["remaining_points_base"], c, w),
                    progress_pct: leaf_f64(o, &["progress_pct"], c, w),
                    eta_days: leaf_i64(o, &["eta_days"], c, w),
                    progress_basis: leaf_string(o, &["progress_basis"], c, w),
                    progress_unavailable_reason: leaf_string(
                        o,
                        &["progress_unavailable_reason"],
                        c,
                        w,
                    ),
                    eta_unavailable_reason: leaf_string(o, &["eta_unavailable_reason"], c, w),
                    ahead_of_time_years: leaf_f64(o, &["ahead_of_time_years"], c, w),
                    ahead_of_time_penalty_pct: leaf_f64(o, &["ahead_of_time_penalty_pct"], c, w),
                    ahead_of_time_reduction_years: leaf_f64(
                        o,
                        &["ahead_of_time_reduction_years"],
                        c,
                        w,
                    ),
                    ahead_of_time_reduced_penalty_pct: leaf_f64(
                        o,
                        &["ahead_of_time_reduced_penalty_pct"],
                        c,
                        w,
                    ),
                }
            }),
            in_progress_live: leaf_rows(
                obj,
                &["research", "in_progress_live"],
                ctx,
                w,
                |o, c, w| ResearchLiveProgressRow {
                    slot_index: leaf_i64(o, &["slot_index"], c, w),
                    tech_handle_raw: leaf_i64(o, &["tech_handle_raw"], c, w),
                    tech_def_index_raw: leaf_i64(o, &["tech_def_index_raw"], c, w),
                    tech_name: leaf_string(o, &["tech_name"], c, w),
                    progress_raw: leaf_i64(o, &["progress_raw"], c, w),
                    progress_units: leaf_f64(o, &["progress_units"], c, w),
                    live_progress_raw: leaf_i64(o, &["live_progress_raw"], c, w),
                    live_progress_units: leaf_f64(o, &["live_progress_units"], c, w),
                    base_required_points_raw: leaf_i64(o, &["base_required_points_raw"], c, w),
                    base_required_points_units: leaf_f64(o, &["base_required_points_units"], c, w),
                    points_factor: leaf_f64(o, &["points_factor"], c, w),
                    points_factor_elapsed_hours: leaf_i64(
                        o,
                        &["points_factor_elapsed_hours"],
                        c,
                        w,
                    ),
                    required_points_base: leaf_f64(o, &["required_points_base"], c, w),
                    remaining_points_base: leaf_f64(o, &["remaining_points_base"], c, w),
                    progress_pct: leaf_f64(o, &["progress_pct"], c, w),
                    eta_days: leaf_i64(o, &["eta_days"], c, w),
                    progress_basis: leaf_string(o, &["progress_basis"], c, w),
                    progress_unavailable_reason: leaf_string(
                        o,
                        &["progress_unavailable_reason"],
                        c,
                        w,
                    ),
                    eta_unavailable_reason: leaf_string(o, &["eta_unavailable_reason"], c, w),
                    ahead_of_time_years: leaf_f64(o, &["ahead_of_time_years"], c, w),
                    ahead_of_time_penalty_pct: leaf_f64(o, &["ahead_of_time_penalty_pct"], c, w),
                    ahead_of_time_reduction_years: leaf_f64(
                        o,
                        &["ahead_of_time_reduction_years"],
                        c,
                        w,
                    ),
                    ahead_of_time_reduced_penalty_pct: leaf_f64(
                        o,
                        &["ahead_of_time_reduced_penalty_pct"],
                        c,
                        w,
                    ),
                },
            ),
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
            current_set: leaf_string_list(obj, &["focus", "current_set"], ctx, w),
            current_continuous: leaf_string(obj, &["focus", "current_continuous"], ctx, w),
            current_continuous_set: leaf_string_list(
                obj,
                &["focus", "current_continuous_set"],
                ctx,
                w,
            ),
            completed_count: leaf_i64(obj, &["focus", "completed_count"], ctx, w),
            progress: leaf_f64(obj, &["focus", "progress"], ctx, w),
            completed: leaf_string_list(obj, &["focus", "completed"], ctx, w),
            available: leaf_string_list(obj, &["focus", "available"], ctx, w),
            visible_order: leaf_string_list(obj, &["focus", "visible_order"], ctx, w),
            paused: leaf_bool(obj, &["focus", "paused"], ctx, w),
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
            stability: leaf_f64(obj, &["politics", "stability"], ctx, w),
            war_support: leaf_f64(obj, &["politics", "war_support"], ctx, w),
            political_power: leaf_f64(obj, &["politics", "political_power"], ctx, w),
            command_power: leaf_f64(obj, &["politics", "command_power"], ctx, w),
            ruling_party: leaf_string(obj, &["politics", "ruling_party"], ctx, w),
            party_popularities: leaf_f64_map(obj, &["politics", "party_popularities"], ctx, w),
            ideas: leaf_string_list(obj, &["politics", "ideas"], ctx, w),
            ideas_all: leaf_string_list(obj, &["politics", "ideas_all"], ctx, w),
            ideas_other: leaf_string_list(obj, &["politics", "ideas_other"], ctx, w),
            national_spirits: leaf_string_list(obj, &["politics", "national_spirits"], ctx, w),
            idea_groups: leaf_string_list_map(obj, &["politics", "idea_groups"], ctx, w),
            advisor_ideas: leaf_string_list_map(obj, &["politics", "advisor_ideas"], ctx, w),
            laws: leaf_string_map(obj, &["politics", "laws"], ctx, w),
            law_economy: leaf_string(obj, &["politics", "laws", "economy"], ctx, w),
            law_trade: leaf_string(obj, &["politics", "laws", "trade"], ctx, w),
            law_conscription: leaf_string(obj, &["politics", "laws", "conscription"], ctx, w),
            idea_count: leaf_i64(obj, &["politics", "idea_count"], ctx, w),
            party_count: leaf_i64(obj, &["politics", "party_count"], ctx, w),
            election_frequency: leaf_i64(obj, &["politics", "election_frequency"], ctx, w),
            last_election: leaf_string(obj, &["politics", "last_election"], ctx, w),
            elections_allowed: leaf_bool(obj, &["politics", "elections_allowed"], ctx, w),
            low_confidence: leaf_bool(obj, &["politics", "low_confidence"], ctx, w),
            appointed_advisors: leaf_string_list(obj, &["politics", "appointed_advisors"], ctx, w),
            appointed_advisor_details: leaf_rows(
                obj,
                &["politics", "appointed_advisor_details"],
                ctx,
                w,
                |row, row_ctx, row_warnings| AppointedAdvisorRow {
                    idea_token: leaf_string(row, &["idea_token"], row_ctx, row_warnings),
                    slot: leaf_string(row, &["slot"], row_ctx, row_warnings),
                    ledger: leaf_string(row, &["ledger"], row_ctx, row_warnings),
                    character: leaf_string(row, &["character"], row_ctx, row_warnings),
                },
            ),
            appointed_advisor_slots: leaf_string_list_map(
                obj,
                &["politics", "appointed_advisor_slots"],
                ctx,
                w,
            ),
            appointed_advisor_ledgers: leaf_string_list_map(
                obj,
                &["politics", "appointed_advisor_ledgers"],
                ctx,
                w,
            ),
            command_abilities: leaf_obj_field_list(obj, &["abilities"], "ability", ctx, w),
            command_ability_rows: leaf_rows(
                obj,
                &["active_abilities", "rows"],
                ctx,
                w,
                |row, row_ctx, row_warnings| CommandAbilityRow {
                    ability_id: leaf_id_string(row, &["ability_id"], row_ctx, row_warnings),
                    available: leaf_bool(row, &["available"], row_ctx, row_warnings),
                    active: leaf_bool(row, &["active"], row_ctx, row_warnings),
                    activation_hour: leaf_i64(row, &["activation_hour"], row_ctx, row_warnings),
                    cooldown_hours: leaf_i64(row, &["cooldown_hours"], row_ctx, row_warnings),
                    cooldown_remaining_hours: leaf_i64(
                        row,
                        &["cooldown_remaining_hours"],
                        row_ctx,
                        row_warnings,
                    ),
                    cost_command_power: leaf_f64(
                        row,
                        &["cost_command_power"],
                        row_ctx,
                        row_warnings,
                    ),
                    target_kind: leaf_string(row, &["target_kind"], row_ctx, row_warnings),
                    target_id: leaf_id_string(row, &["target_id"], row_ctx, row_warnings),
                    duration_hours: leaf_i64(row, &["duration_hours"], row_ctx, row_warnings),
                    duration_remaining_hours: leaf_i64(
                        row,
                        &["duration_remaining_hours"],
                        row_ctx,
                        row_warnings,
                    ),
                    display_order: leaf_i64(row, &["display_order"], row_ctx, row_warnings),
                    icon: leaf_string(row, &["icon"], row_ctx, row_warnings),
                    tooltip: leaf_string(row, &["tooltip"], row_ctx, row_warnings),
                    disabled_reason: leaf_string(row, &["disabled_reason"], row_ctx, row_warnings),
                },
            ),
            command_abilities_complete: leaf_bool(obj, &["active_abilities", "complete"], ctx, w),
        },
    )
}

fn equipment_production_line_row(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> EquipmentProductionLineRow {
    EquipmentProductionLineRow {
        id: leaf_i64(obj, &["id"], ctx, warnings),
        priority: leaf_i64(obj, &["priority"], ctx, warnings),
        line_subtype: leaf_string(obj, &["line_subtype"], ctx, warnings),
        equipment_variant_id: leaf_id_string(obj, &["equipment_variant_id"], ctx, warnings),
        active_factories: leaf_i64(obj, &["active_factories"], ctx, warnings),
        requested_factories: leaf_i64(obj, &["requested_factories"], ctx, warnings),
        efficiency: leaf_f64(obj, &["efficiency"], ctx, warnings),
        efficiency_cap: leaf_f64(obj, &["efficiency_cap"], ctx, warnings),
        efficiency_growth: leaf_f64(obj, &["efficiency_growth"], ctx, warnings),
        cost: leaf_f64(obj, &["cost"], ctx, warnings),
        speed: leaf_f64(obj, &["speed"], ctx, warnings),
        produced: leaf_f64(obj, &["produced"], ctx, warnings),
        amount: leaf_i64(obj, &["amount"], ctx, warnings),
        progress_ratio: leaf_f64(obj, &["progress_ratio"], ctx, warnings),
        daily_output: leaf_f64(obj, &["daily_output"], ctx, warnings),
        days_to_next_unit: leaf_f64(obj, &["days_to_next_unit"], ctx, warnings),
        mio_id: leaf_id_string(obj, &["mio_id"], ctx, warnings),
    }
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
            stockpile_complete: leaf_bool(obj, &["production", "stockpile_complete"], ctx, w),
            military_lines: leaf_rows(
                obj,
                &["production", "military_lines"],
                ctx,
                w,
                equipment_production_line_row,
            ),
            military_lines_complete: leaf_bool(
                obj,
                &["production", "military_lines_complete"],
                ctx,
                w,
            ),
            naval_lines: leaf_rows(
                obj,
                &["production", "naval_lines"],
                ctx,
                w,
                equipment_production_line_row,
            ),
            naval_lines_complete: leaf_bool(obj, &["production", "naval_lines_complete"], ctx, w),
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
            convoys_complete: leaf_bool(obj, &["military", "convoys_complete"], ctx, w),
            convoys_owned_raw: leaf_i64(obj, &["military", "convoys_owned_raw"], ctx, w),
            convoys_owned: leaf_f64(obj, &["military", "convoys_count"], ctx, w),
            convoys_assigned: leaf_i64(obj, &["military", "convoys_assigned"], ctx, w),
            convoys_available: leaf_i64(obj, &["military", "convoys_available"], ctx, w),
            convoys_cache_valid: leaf_bool(obj, &["military", "convoys_cache_valid"], ctx, w),
            convoy_variants: leaf_rows(obj, &["military", "convoys"], ctx, w, |o, c, w| {
                ConvoyVariantAmount {
                    archetype: leaf_string(o, &["archetype"], c, w),
                    equip_id: leaf_i64(o, &["equip_id"], c, w),
                    equip_type: leaf_i64(o, &["equip_type"], c, w),
                    amount_raw: leaf_i64(o, &["amount_raw"], c, w),
                    amount: leaf_f64(o, &["amount"], c, w),
                }
            }),
            convoy_priority_reservations_complete: leaf_bool(
                obj,
                &["military", "convoy_priority_reservations_complete"],
                ctx,
                w,
            ),
            convoy_priority_reservations: leaf_rows(
                obj,
                &["military", "convoy_priority_reservations"],
                ctx,
                w,
                |o, c, w| ConvoyPriorityReservation {
                    priority: leaf_i64(o, &["priority"], c, w),
                    count: leaf_i64(o, &["count"], c, w),
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
            domain53_agency_scalars_raw: leaf_domain53_agency_scalars_raw(obj, ctx, w),
            domain53_agency_capacity_raw: leaf_domain53_agency_capacity_raw(obj, ctx, w),
            domain53_operative_lifecycle_raw: leaf_rows(
                obj,
                &["intel", "domain53_extra", "operative_lifecycle_rows"],
                ctx,
                w,
                |o, c, w| Domain53OperativeLifecycleRawRow {
                    op_id: leaf_i64(o, &["op_id"], c, w),
                    detected: leaf_bool(o, &["detected"], c, w),
                    captured_unvalidated: leaf_bool(o, &["captured_unvalidated"], c, w),
                    skill: leaf_i64(o, &["skill"], c, w),
                    experience_raw: leaf_i64(o, &["experience_raw"], c, w),
                    name_hash: leaf_u64(o, &["name_hash"], c, w),
                    state_hash: leaf_u64(o, &["state_hash"], c, w),
                    lifecycle_flag: leaf_i64(o, &["lifecycle_flag"], c, w),
                },
            ),
            domain53_cryptology_targets_raw: leaf_rows(
                obj,
                &["intel", "domain53_extra", "cryptology_targets"],
                ctx,
                w,
                |o, c, w| Domain53CryptologyTargetRawRow {
                    target_tag_db_id: leaf_i64(o, &["target_tag_db_id"], c, w),
                    amount_raw: leaf_i64(o, &["amount_raw"], c, w),
                    decrypting: leaf_bool(o, &["decrypting"], c, w),
                },
            ),
        }
    })
}

fn leaf_domain53_agency_scalars_raw(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Domain53AgencyScalarsRaw> {
    let path = &["intel", "domain53_extra", "agency_scalars"];
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Object(o)) => {
            let c = format!("{ctx}.{}", path.join("."));
            Leaf::Measured(Domain53AgencyScalarsRaw {
                max_operatives_unresolved: leaf_i64(
                    o,
                    &["max_operatives_unresolved"],
                    &c,
                    warnings,
                ),
                usable_slots_unresolved: leaf_i64(o, &["usable_slots_unresolved"], &c, warnings),
                operative_count: leaf_i64(o, &["operative_count"], &c, warnings),
                cryptology_target_count: leaf_i64(o, &["cryptology_target_count"], &c, warnings),
                value_0x128_unvalidated: leaf_i64(o, &["defense_raw"], &c, warnings),
                value_0xc8_unvalidated: leaf_i64(o, &["upgrade_progress_raw"], &c, warnings),
                name_hash: leaf_u64(o, &["name_hash"], &c, warnings),
                icon_hash: leaf_u64(o, &["icon_hash"], &c, warnings),
            })
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: expected object, found {}; treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn leaf_domain53_agency_capacity_raw(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Domain53AgencyCapacityRaw> {
    let path = &["intel", "domain53_extra", "raw_capacity_cluster"];
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(Value::Object(o)) => {
            let c = format!("{ctx}.{}", path.join("."));
            Leaf::Measured(Domain53AgencyCapacityRaw {
                value_0xe4: leaf_i64(o, &["value_0xe4"], &c, warnings),
                value_0xf0: leaf_i64(o, &["value_0xf0"], &c, warnings),
                value_0xf4: leaf_i64(o, &["value_0xf4"], &c, warnings),
                proof_hash: leaf_u64(o, &["proof_hash"], &c, warnings),
                semantic_status: leaf_string(o, &["semantic_status"], &c, warnings),
            })
        }
        Leaf::Measured(other) => {
            warnings.push(format!(
                "{ctx}.{}: expected object, found {}; treated as unobtained",
                path.join("."),
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn build_trade(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, TradeView>, ViewStatus) {
    country_map(rest, warnings, TradeView::all_unobtained, |obj, ctx, w| {
        let resource_rows = validated_trade_resource_rows(obj, ctx, w);
        let resource_deals = validated_resource_trade_deals(obj, ctx, w);
        TradeView {
            resource_pools_complete: leaf_bool(obj, &["economy", "resources", "complete"], ctx, w),
            produced: leaf_f64_map(obj, &["economy", "resources", "produced"], ctx, w),
            imported: leaf_f64_map(obj, &["economy", "resources", "imported"], ctx, w),
            used: leaf_f64_map(obj, &["economy", "resources", "used"], ctx, w),
            exported: leaf_f64_map(obj, &["economy", "resources", "exported"], ctx, w),
            to_export: leaf_f64_map(obj, &["economy", "resources", "to_export"], ctx, w),
            base_export: leaf_f64_map(obj, &["economy", "resources", "base_export"], ctx, w),
            available_derived: leaf_f64_map(
                obj,
                &["economy", "resources", "available_derived"],
                ctx,
                w,
            ),
            resource_rows,
            resource_rows_complete: leaf_bool(
                obj,
                &["economy", "resource_trade", "resource_rows_complete"],
                ctx,
                w,
            ),
            resource_deals,
            resource_deals_complete: leaf_bool(
                obj,
                &["economy", "resource_trade", "deals_complete"],
                ctx,
                w,
            ),
            factories_total: leaf_i64(
                obj,
                &["economy", "resource_trade", "factories", "total"],
                ctx,
                w,
            ),
            factories_used: leaf_i64(
                obj,
                &["economy", "resource_trade", "factories", "used"],
                ctx,
                w,
            ),
            factories_available: leaf_i64(
                obj,
                &["economy", "resource_trade", "factories", "available"],
                ctx,
                w,
            ),
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

fn validated_trade_resource_rows(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<TradeResourceRow>> {
    let rows = leaf_rows(
        obj,
        &["economy", "resource_trade", "resource_rows"],
        ctx,
        warnings,
        |o, c, w| TradeResourceRow {
            resource: leaf_string(o, &["resource"], c, w),
            visible_order: leaf_i64(o, &["visible_order"], c, w),
            balance: leaf_f64(o, &["balance"], c, w),
            produced: leaf_f64(o, &["produced"], c, w),
            imported: leaf_f64(o, &["imported"], c, w),
            exported: leaf_f64(o, &["exported"], c, w),
            special_project: leaf_f64(o, &["special_project"], c, w),
            production_line: leaf_f64(o, &["production_line"], c, w),
            needed: leaf_f64(o, &["needed"], c, w),
            needed_current: leaf_f64(o, &["needed_current"], c, w),
            needed_max: leaf_f64(o, &["needed_max"], c, w),
            needed_type: leaf_string(o, &["needed_type"], c, w),
        },
    );
    let rows = validate_visible_order(
        rows,
        ctx,
        "economy.resource_trade.resource_rows",
        warnings,
        |row| &row.visible_order,
    );
    validate_trade_resource_order(rows, ctx, warnings)
}

fn validate_trade_resource_order(
    rows: Leaf<Vec<TradeResourceRow>>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<TradeResourceRow>> {
    let Leaf::Measured(items) = rows else {
        return rows;
    };
    if items.len() != TRADE_RESOURCE_ORDER.len() {
        warnings.push(format!(
            "{ctx}.economy.resource_trade.resource_rows: expected exact seven-resource list, found {}; whole list treated as unobtained",
            items.len()
        ));
        return Leaf::Unobtained;
    }
    for (index, (item, expected)) in items.iter().zip(TRADE_RESOURCE_ORDER).enumerate() {
        match &item.resource {
            Leaf::Measured(resource) if resource == expected => {}
            Leaf::Measured(resource) => {
                warnings.push(format!(
                    "{ctx}.economy.resource_trade.resource_rows[{index}].resource: expected {expected}, found {resource}; whole list treated as unobtained"
                ));
                return Leaf::Unobtained;
            }
            _ => {
                warnings.push(format!(
                    "{ctx}.economy.resource_trade.resource_rows[{index}].resource: not measured; whole list treated as unobtained"
                ));
                return Leaf::Unobtained;
            }
        }
    }
    Leaf::Measured(items)
}

fn validated_resource_trade_deals(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<ResourceTradeDealRow>> {
    let rows = leaf_rows(
        obj,
        &["economy", "resource_trade", "deals"],
        ctx,
        warnings,
        |o, c, w| ResourceTradeDealRow {
            deal_id: leaf_i64(o, &["deal_id"], c, w),
            counterpart: leaf_string(o, &["counterpart"], c, w),
            direction: leaf_string(o, &["direction"], c, w),
            resource: leaf_string(o, &["resource"], c, w),
            requested: leaf_f64(o, &["requested"], c, w),
            delivered: leaf_f64(o, &["delivered"], c, w),
            available_export: leaf_f64(o, &["available_export"], c, w),
            civilian_factories: leaf_i64(o, &["civilian_factories"], c, w),
            factory_cost_per_resource: leaf_f64(o, &["factory_cost_per_resource"], c, w),
            convoys_assigned: leaf_i64(o, &["convoys", "assigned"], c, w),
            convoys_required: leaf_i64(o, &["convoys", "required"], c, w),
            route_kind: leaf_string(o, &["route_kind"], c, w),
            delivery_efficiency: leaf_f64(o, &["delivery_efficiency"], c, w),
            no_trade_influence: leaf_bool(o, &["no_trade_influence"], c, w),
            no_resources: leaf_bool(o, &["no_resources"], c, w),
            visible_order: leaf_i64(o, &["visible_order"], c, w),
        },
    );
    validate_visible_order(rows, ctx, "economy.resource_trade.deals", warnings, |row| {
        &row.visible_order
    })
}

/// A materialized visible list must carry contiguous zero-based ordinals in
/// its serialized array order.  Missing/wrong-typed ordinals poison the whole
/// collection rather than allowing replay to invent an order.
fn validate_visible_order<T>(
    rows: Leaf<Vec<T>>,
    ctx: &str,
    path: &str,
    warnings: &mut Vec<String>,
    ordinal: impl Fn(&T) -> &Leaf<i64>,
) -> Leaf<Vec<T>> {
    let Leaf::Measured(items) = rows else {
        return rows;
    };
    for (index, item) in items.iter().enumerate() {
        match ordinal(item) {
            Leaf::Measured(value) if *value == index as i64 => {}
            Leaf::Measured(value) => {
                warnings.push(format!(
                    "{ctx}.{path}[{index}].visible_order: expected {index}, found {value}; whole list treated as unobtained"
                ));
                return Leaf::Unobtained;
            }
            _ => {
                warnings.push(format!(
                    "{ctx}.{path}[{index}].visible_order: not measured; whole list treated as unobtained"
                ));
                return Leaf::Unobtained;
            }
        }
    }
    Leaf::Measured(items)
}

fn xui17_object<'a>(
    value: &'a Value,
    ctx: &str,
) -> std::result::Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{ctx}: expected object, found {}", json_type_name(value)))
}

fn xui17_array<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
    ctx: &str,
    required: bool,
) -> std::result::Result<&'a [Value], String> {
    match obj.get(key) {
        Some(Value::Array(rows)) => Ok(rows),
        Some(value) => Err(format!(
            "{ctx}.{key}: expected array, found {}",
            json_type_name(value)
        )),
        None if required => Err(format!("{ctx}.{key}: required exact array is absent")),
        None => Ok(&[]),
    }
}

fn xui17_u64(obj: &Map<String, Value>, key: &str, ctx: &str) -> std::result::Result<u64, String> {
    obj.get(key).and_then(Value::as_u64).ok_or_else(|| {
        format!("{ctx}.{key}: required non-negative integer is absent or wrong-typed")
    })
}

fn xui17_i64(obj: &Map<String, Value>, key: &str, ctx: &str) -> std::result::Result<i64, String> {
    obj.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{ctx}.{key}: required integer is absent or wrong-typed"))
}

fn xui17_bool(obj: &Map<String, Value>, key: &str, ctx: &str) -> std::result::Result<bool, String> {
    obj.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{ctx}.{key}: required bool is absent or wrong-typed"))
}

fn xui17_optional_u64(
    obj: &Map<String, Value>,
    key: &str,
    ctx: &str,
) -> std::result::Result<Option<u64>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{ctx}.{key}: expected non-negative integer or null")),
    }
}

fn xui17_optional_bool(
    obj: &Map<String, Value>,
    key: &str,
    ctx: &str,
) -> std::result::Result<Option<bool>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("{ctx}.{key}: expected bool or null")),
    }
}

fn xui17_string(
    obj: &Map<String, Value>,
    key: &str,
    ctx: &str,
) -> std::result::Result<String, String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{ctx}.{key}: required string is absent or wrong-typed"))
}

fn xui17_oob_assignments(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (
    bool,
    BTreeMap<String, BTreeMap<u64, UnitLeaderAssignmentExact>>,
) {
    let Some(Value::Object(oob)) = rest.get("order_of_battle") else {
        return (false, BTreeMap::new());
    };
    let specs = [
        ("army_groups", UnitLeaderAssignmentKind::ArmyGroup),
        ("armies", UnitLeaderAssignmentKind::Army),
        ("fleets", UnitLeaderAssignmentKind::Fleet),
    ];
    let mut out: BTreeMap<String, BTreeMap<u64, UnitLeaderAssignmentExact>> = BTreeMap::new();
    let mut global_leaders = BTreeSet::new();
    for (array_name, kind) in specs {
        let Some(Value::Array(rows)) = oob.get(array_name) else {
            return (false, BTreeMap::new());
        };
        for (ordinal, value) in rows.iter().enumerate() {
            let ctx = format!("order_of_battle.{array_name}[{ordinal}]");
            let Some(row) = value.as_object() else {
                warnings.push(format!(
                    "{ctx}: malformed d28 command row; XUI17 assignments unavailable"
                ));
                return (false, BTreeMap::new());
            };
            let Some(formation_id) = row.get("id").and_then(Value::as_u64) else {
                warnings.push(format!(
                    "{ctx}.id: missing integer; XUI17 assignments unavailable"
                ));
                return (false, BTreeMap::new());
            };
            let Some(owner) = row.get("owner").and_then(Value::as_str) else {
                warnings.push(format!(
                    "{ctx}.owner: missing string; XUI17 assignments unavailable"
                ));
                return (false, BTreeMap::new());
            };
            let leader_id = match row.get("leader") {
                Some(Value::Null) | None => continue,
                Some(value) => match value.as_u64() {
                    Some(id) => id,
                    None => {
                        warnings.push(format!(
                            "{ctx}.leader: wrong typed; XUI17 assignments unavailable"
                        ));
                        return (false, BTreeMap::new());
                    }
                },
            };
            if !global_leaders.insert(leader_id) {
                warnings.push(format!(
                    "{ctx}.leader: duplicate d28 assignment for unit_leader_id {leader_id}; XUI17 assignments unavailable"
                ));
                return (false, BTreeMap::new());
            }
            out.entry(owner.to_string()).or_default().insert(
                leader_id,
                UnitLeaderAssignmentExact {
                    kind: kind.clone(),
                    formation_id,
                },
            );
        }
    }
    (true, out)
}

fn parse_xui17_character_roster(
    sidecar: &Map<String, Value>,
    ctx: &str,
    command_assignments_complete: bool,
    assignments: Option<&BTreeMap<u64, UnitLeaderAssignmentExact>>,
) -> std::result::Result<CharacterRosterExactView, String> {
    let mut characters = Vec::new();
    let mut character_ids = BTreeSet::new();
    for (ordinal, value) in xui17_array(sidecar, "characters", ctx, true)?
        .iter()
        .enumerate()
    {
        let rctx = format!("{ctx}.characters[{ordinal}]");
        let row = xui17_object(value, &rctx)?;
        let char_id = xui17_u64(row, "char_id", &rctx)?;
        if !character_ids.insert(char_id) {
            return Err(format!("{rctx}.char_id: duplicate {char_id}"));
        }
        characters.push(CharacterIdentityExactRow {
            producer_ordinal: ordinal,
            char_id,
            name: xui17_string(row, "name", &rctx)?,
            token: xui17_string(row, "token", &rctx)?,
        });
    }

    let mut traits_by_leader: BTreeMap<u64, Vec<(u64, String)>> = BTreeMap::new();
    for (ordinal, value) in xui17_array(sidecar, "unit_leader_traits", ctx, false)?
        .iter()
        .enumerate()
    {
        let rctx = format!("{ctx}.unit_leader_traits[{ordinal}]");
        let row = xui17_object(value, &rctx)?;
        let leader_id = xui17_u64(row, "unit_leader_id", &rctx)?;
        let trait_ordinal = xui17_u64(row, "ordinal", &rctx)?;
        let token = xui17_string(row, "trait_token", &rctx)?;
        if token.is_empty() {
            return Err(format!("{rctx}.trait_token: empty exact token"));
        }
        traits_by_leader
            .entry(leader_id)
            .or_default()
            .push((trait_ordinal, token));
    }

    let mut unit_leaders = Vec::new();
    let mut unit_leader_ids = BTreeSet::new();
    for (ordinal, value) in xui17_array(sidecar, "unit_leaders", ctx, false)?
        .iter()
        .enumerate()
    {
        let rctx = format!("{ctx}.unit_leaders[{ordinal}]");
        let row = xui17_object(value, &rctx)?;
        let unit_leader_id = xui17_u64(row, "unit_leader_id", &rctx)?;
        if !unit_leader_ids.insert(unit_leader_id) {
            return Err(format!("{rctx}.unit_leader_id: duplicate {unit_leader_id}"));
        }
        let char_id = xui17_u64(row, "char_id", &rctx)?;
        if !character_ids.contains(&char_id) {
            return Err(format!("{rctx}.char_id: orphan character {char_id}"));
        }
        let leader_kind = xui17_string(row, "leader_kind", &rctx)?;
        let leader_kind_raw = xui17_u64(row, "leader_kind_raw", &rctx)?;
        let expected_kind = match leader_kind_raw {
            0 => "army",
            1 => "navy",
            other => return Err(format!("{rctx}.leader_kind_raw: unsupported {other}")),
        };
        if leader_kind != expected_kind {
            return Err(format!(
                "{rctx}.leader_kind: {leader_kind:?} contradicts raw {leader_kind_raw}"
            ));
        }
        let assignment = assignments
            .and_then(|rows| rows.get(&unit_leader_id))
            .cloned();
        if let Some(assignment) = &assignment {
            let assignment_matches_kind = matches!(
                (&leader_kind[..], &assignment.kind),
                (
                    "army",
                    UnitLeaderAssignmentKind::Army | UnitLeaderAssignmentKind::ArmyGroup
                ) | ("navy", UnitLeaderAssignmentKind::Fleet)
            );
            if !assignment_matches_kind {
                return Err(format!(
                    "{rctx}: d28 {:?} assignment contradicts {leader_kind} leader kind",
                    assignment.kind
                ));
            }
        }
        let mut ordered_traits = traits_by_leader.remove(&unit_leader_id).unwrap_or_default();
        ordered_traits.sort_by_key(|(trait_ordinal, _)| *trait_ordinal);
        let mut traits = Vec::with_capacity(ordered_traits.len());
        for (expected, (actual, token)) in ordered_traits.into_iter().enumerate() {
            if actual != expected as u64 {
                return Err(format!(
                    "{ctx}.unit_leader_traits: leader {unit_leader_id} expected ordinal {expected}, found {actual}"
                ));
            }
            traits.push(token);
        }
        unit_leaders.push(UnitLeaderExactRow {
            producer_ordinal: ordinal,
            unit_leader_id,
            char_id,
            leader_kind,
            leader_kind_raw,
            skill: xui17_i64(row, "skill", &rctx)?,
            experience_raw: xui17_i64(row, "experience_raw", &rctx)?,
            attack: xui17_i64(row, "attack", &rctx)?,
            defense: xui17_i64(row, "defense", &rctx)?,
            planning: xui17_i64(row, "planning", &rctx)?,
            logistics: xui17_i64(row, "logistics", &rctx)?,
            maneuvering: xui17_i64(row, "maneuvering", &rctx)?,
            coordination: xui17_i64(row, "coordination", &rctx)?,
            traits,
            assignment,
            token: xui17_string(row, "token", &rctx)?,
            name: xui17_string(row, "name", &rctx)?,
        });
    }
    if let Some((&orphan, _)) = traits_by_leader.iter().next() {
        return Err(format!(
            "{ctx}.unit_leader_traits: orphan unit_leader_id {orphan}"
        ));
    }
    if let Some(assignments) = assignments {
        if let Some(orphan) = assignments
            .keys()
            .find(|leader_id| !unit_leader_ids.contains(leader_id))
        {
            return Err(format!(
                "{ctx}: d28 assignment references orphan unit_leader_id {orphan}"
            ));
        }
    }

    let mut country_leader_roles = Vec::new();
    for (ordinal, value) in xui17_array(sidecar, "country_leader_roles", ctx, false)?
        .iter()
        .enumerate()
    {
        let rctx = format!("{ctx}.country_leader_roles[{ordinal}]");
        let row = xui17_object(value, &rctx)?;
        let char_id = xui17_u64(row, "char_id", &rctx)?;
        if !character_ids.contains(&char_id) {
            return Err(format!("{rctx}.char_id: orphan character {char_id}"));
        }
        country_leader_roles.push(CountryLeaderRoleExactRow {
            producer_ordinal: ordinal,
            char_id,
            name: xui17_string(row, "name", &rctx)?,
            token: xui17_string(row, "token", &rctx)?,
        });
    }

    let mut active_rulers = Vec::new();
    for (ordinal, value) in xui17_array(sidecar, "active_rulers", ctx, false)?
        .iter()
        .enumerate()
    {
        let rctx = format!("{ctx}.active_rulers[{ordinal}]");
        let row = xui17_object(value, &rctx)?;
        let char_id = xui17_u64(row, "char_id", &rctx)?;
        if !character_ids.contains(&char_id) {
            return Err(format!("{rctx}.char_id: orphan character {char_id}"));
        }
        active_rulers.push(ActiveRulerExactRow {
            producer_ordinal: ordinal,
            char_id,
            ruling_party: xui17_string(row, "ruling_party", &rctx)?,
            name: xui17_string(row, "name", &rctx)?,
            token: xui17_string(row, "token", &rctx)?,
            leader_vec_size: xui17_u64(row, "leader_vec_size", &rctx)?,
            ambiguous: xui17_bool(row, "ambiguous", &rctx)?,
        });
    }

    Ok(CharacterRosterExactView {
        status: ViewStatus::Present,
        command_assignments_complete,
        characters,
        unit_leaders,
        country_leader_roles,
        active_rulers,
    })
}

fn parse_xui17_ace_roster(
    sidecar: &Map<String, Value>,
    ctx: &str,
) -> std::result::Result<AceRosterExactView, String> {
    let blocked = match sidecar.get("wing_assignment_blocked") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(value) => {
            return Err(format!(
                "{ctx}.wing_assignment_blocked: expected bool, found {}",
                json_type_name(value)
            ))
        }
    };
    let rows = xui17_array(sidecar, "aces", ctx, true)?;
    if rows.is_empty() {
        return Err(format!(
            "{ctx}.aces: empty sidecar is not an authenticated roster"
        ));
    }
    let mut ids = BTreeSet::new();
    let mut assigned_wings = BTreeSet::new();
    let mut aces = Vec::with_capacity(rows.len());
    for (ordinal, value) in rows.iter().enumerate() {
        let rctx = format!("{ctx}.aces[{ordinal}]");
        let row = xui17_object(value, &rctx)?;
        let ace_id = xui17_u64(row, "ace_id", &rctx)?;
        if !ids.insert(ace_id) {
            return Err(format!("{rctx}.ace_id: duplicate {ace_id}"));
        }
        let country_ace_index = xui17_u64(row, "country_ace_index", &rctx)?;
        if country_ace_index != ordinal as u64 {
            return Err(format!(
                "{rctx}.country_ace_index: expected {ordinal}, found {country_ace_index}"
            ));
        }
        let wing_id = xui17_optional_u64(row, "wing_id", &rctx)?.unwrap_or(0);
        if wing_id != 0 && !assigned_wings.insert(wing_id) {
            return Err(format!(
                "{rctx}.wing_id: duplicate nonzero assignment {wing_id}"
            ));
        }
        let alive = xui17_bool(row, "alive", &rctx)?;
        let handled = xui17_optional_bool(row, "handled", &rctx)?;
        let kill_type_raw = xui17_optional_u64(row, "kill_type_raw", &rctx)?;
        if handled.is_some() != kill_type_raw.is_some() {
            return Err(format!(
                "{rctx}: handled and kill_type_raw must be jointly present or absent"
            ));
        }
        let lifecycle = match (alive, handled) {
            (_, None) => None,
            (true, Some(false)) => Some(AceLifecycleState::Alive),
            (false, Some(false)) => Some(AceLifecycleState::DeadUnhandled),
            (false, Some(true)) => Some(AceLifecycleState::DeadHandled),
            (true, Some(true)) => {
                return Err(format!(
                    "{rctx}: contradictory lifecycle tuple alive=true handled=true"
                ))
            }
        };
        if let Some(value) = row.get("lifecycle") {
            let claimed = match value {
                Value::Null if lifecycle.is_none() => None,
                Value::String(value) if value == "alive" => Some(AceLifecycleState::Alive),
                Value::String(value) if value == "dead_unhandled" => {
                    Some(AceLifecycleState::DeadUnhandled)
                }
                Value::String(value) if value == "dead_handled" => {
                    Some(AceLifecycleState::DeadHandled)
                }
                _ => {
                    return Err(format!(
                        "{rctx}.lifecycle: expected exact derived lifecycle or null"
                    ))
                }
            };
            if claimed != lifecycle {
                return Err(format!(
                    "{rctx}.lifecycle: does not match alive/handled tuple"
                ));
            }
        }
        aces.push(AceExactRow {
            ace_id,
            country_ace_index,
            name: xui17_string(row, "name", &rctx)?,
            surname: xui17_string(row, "surname", &rctx)?,
            callsign: xui17_string(row, "callsign", &rctx)?,
            alive,
            handled,
            kill_type_raw,
            lifecycle,
            wing_id,
            killer_ace_id: xui17_optional_u64(row, "killer_ace_id", &rctx)?.unwrap_or(0),
            killer_country_tag_db_id: xui17_optional_u64(row, "killer_country_tag_db_id", &rctx)?
                .unwrap_or(0),
        });
    }
    Ok(AceRosterExactView {
        status: ViewStatus::Present,
        wing_assignment_complete: !blocked,
        aces,
    })
}

fn xui17_merge_status(status: &mut ViewStatus, candidate: ViewStatus) {
    *status = match (*status, candidate) {
        (ViewStatus::DomainUnavailable, _) | (_, ViewStatus::DomainUnavailable) => {
            ViewStatus::DomainUnavailable
        }
        (ViewStatus::Present, _) | (_, ViewStatus::Present) => ViewStatus::Present,
        _ => ViewStatus::DomainAbsent,
    };
}

fn build_xui17_exact_rosters(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (
    BTreeMap<String, CharacterRosterExactView>,
    ViewStatus,
    BTreeMap<String, AceRosterExactView>,
    ViewStatus,
) {
    let (command_assignments_complete, command_assignments) = xui17_oob_assignments(rest, warnings);
    let countries = match rest.get("countries") {
        Some(Value::Object(countries)) => countries,
        Some(Value::Null) => {
            return (
                BTreeMap::new(),
                ViewStatus::DomainUnavailable,
                BTreeMap::new(),
                ViewStatus::DomainUnavailable,
            )
        }
        None => {
            return (
                BTreeMap::new(),
                ViewStatus::DomainAbsent,
                BTreeMap::new(),
                ViewStatus::DomainAbsent,
            )
        }
        Some(other) => {
            warnings.push(format!(
                "countries: expected object for XUI17 exact rosters, found {}",
                json_type_name(other)
            ));
            return (
                BTreeMap::new(),
                ViewStatus::DomainUnavailable,
                BTreeMap::new(),
                ViewStatus::DomainUnavailable,
            );
        }
    };

    let mut characters = BTreeMap::new();
    let mut aces = BTreeMap::new();
    let mut character_status = ViewStatus::DomainAbsent;
    let mut ace_status = ViewStatus::DomainAbsent;
    for (tag, country_value) in countries {
        let country = match country_value.as_object() {
            Some(country) => country,
            None => continue,
        };
        let Some(extra_value) = country.get("extra") else {
            continue;
        };
        let Some(extra) = extra_value.as_object() else {
            warnings.push(format!(
                "countries.{tag}.extra: expected object for XUI17 exact rosters, found {}",
                json_type_name(extra_value)
            ));
            characters.insert(tag.clone(), CharacterRosterExactView::unavailable());
            aces.insert(tag.clone(), AceRosterExactView::unavailable());
            xui17_merge_status(&mut character_status, ViewStatus::DomainUnavailable);
            xui17_merge_status(&mut ace_status, ViewStatus::DomainUnavailable);
            continue;
        };

        if let Some(value) = extra.get("_r2_characters_leaders_cabinet_live") {
            let ctx = format!("countries.{tag}.extra._r2_characters_leaders_cabinet_live");
            match xui17_object(value, &ctx).and_then(|row| {
                parse_xui17_character_roster(
                    row,
                    &ctx,
                    command_assignments_complete,
                    command_assignments.get(tag),
                )
            }) {
                Ok(view) => {
                    characters.insert(tag.clone(), view);
                    xui17_merge_status(&mut character_status, ViewStatus::Present);
                }
                Err(error) => {
                    warnings.push(error);
                    characters.insert(tag.clone(), CharacterRosterExactView::unavailable());
                    xui17_merge_status(&mut character_status, ViewStatus::DomainUnavailable);
                }
            }
        }

        if let Some(value) = extra.get("_r2_air_ace_roster_live") {
            let ctx = format!("countries.{tag}.extra._r2_air_ace_roster_live");
            match xui17_object(value, &ctx).and_then(|row| parse_xui17_ace_roster(row, &ctx)) {
                Ok(view) => {
                    aces.insert(tag.clone(), view);
                    xui17_merge_status(&mut ace_status, ViewStatus::Present);
                }
                Err(error) => {
                    warnings.push(error);
                    aces.insert(tag.clone(), AceRosterExactView::unavailable());
                    xui17_merge_status(&mut ace_status, ViewStatus::DomainUnavailable);
                }
            }
        }
    }
    (characters, character_status, aces, ace_status)
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

fn occupation_equipment_exact_row(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> OccupationEquipmentExactRow {
    OccupationEquipmentExactRow {
        source_ordinal: leaf_i64(obj, &["source_ordinal"], ctx, warnings),
        equipment_id: leaf_u32(obj, &["equipment_id"], ctx, warnings),
        equipment_type: leaf_u32(obj, &["equipment_type"], ctx, warnings),
        equipment_token: leaf_string(obj, &["equipment_token"], ctx, warnings),
        amount: leaf_f64(obj, &["amount"], ctx, warnings),
    }
}

fn occupation_manpower_exact_row(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> OccupationManpowerExactRow {
    OccupationManpowerExactRow {
        source_ordinal: leaf_i64(obj, &["source_ordinal"], ctx, warnings),
        tag: leaf_string(obj, &["tag"], ctx, warnings),
        value: leaf_f64(obj, &["value"], ctx, warnings),
    }
}

fn garrison_reinforcement_exact_row(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> GarrisonReinforcementExactRow {
    GarrisonReinforcementExactRow {
        source_ordinal: leaf_i64(obj, &["source_ordinal"], ctx, warnings),
        total_progress: leaf_f64(obj, &["total_progress"], ctx, warnings),
        need: leaf_f64_map(obj, &["need"], ctx, warnings),
        date: leaf_string(obj, &["date"], ctx, warnings),
    }
}

fn build_occupation_history(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (Leaf<Vec<OccupationHistoryExactRow>>, Leaf<bool>) {
    let rows = leaf_rows(
        rest,
        &["occupation_history"],
        "snapshot",
        warnings,
        |obj, ctx, row_warnings| OccupationHistoryExactRow {
            stable_event_id: leaf_string(obj, &["stable_event_id"], ctx, row_warnings),
            source_ordinal: leaf_i64(obj, &["source_ordinal"], ctx, row_warnings),
            controller_tag: leaf_string(obj, &["controller_tag"], ctx, row_warnings),
            state_id: leaf_u32(obj, &["state_id"], ctx, row_warnings),
            occupied_country_tag: leaf_string(obj, &["occupied_country_tag"], ctx, row_warnings),
            event_hour_or_engine_date: leaf_string(
                obj,
                &["event_hour_or_engine_date"],
                ctx,
                row_warnings,
            ),
            attack_blocked: leaf_bool(obj, &["attack_blocked"], ctx, row_warnings),
            resistance_activity: leaf_string(obj, &["resistance_activity"], ctx, row_warnings),
            lost_manpower: leaf_i64(obj, &["lost_manpower"], ctx, row_warnings),
            equipment_loss_rows: leaf_rows(
                obj,
                &["equipment_loss_rows"],
                ctx,
                row_warnings,
                occupation_equipment_exact_row,
            ),
            equipment_loss_children_complete: leaf_bool(
                obj,
                &["equipment_loss_children_complete"],
                ctx,
                row_warnings,
            ),
        },
    );
    let complete = leaf_bool(rest, &["occupation_history_complete"], "snapshot", warnings);
    (rows, complete)
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
                        occupier_tag: leaf_string(occ, &["occupier_tag"], &ctx, warnings),
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
                        occupation_law_token_id: leaf_u32_prefer(
                            occ,
                            &["occupation_law_token_id"],
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "occupation_law_token_id",
                            ],
                            &ctx,
                            warnings,
                        ),
                        occupation_law_token_encoding: leaf_string_prefer(
                            occ,
                            &["occupation_law_token_encoding"],
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "occupation_law_token_encoding",
                            ],
                            &ctx,
                            warnings,
                        ),
                        occupation_law_token_source: leaf_string(
                            occ,
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "occupation_law_token_source",
                            ],
                            &ctx,
                            warnings,
                        ),
                        occupation_law_status: leaf_string(
                            occ,
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "occupation_law_status",
                            ],
                            &ctx,
                            warnings,
                        ),
                        garrison_template_id: leaf_u32_prefer(
                            occ,
                            &["garrison_template_id"],
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "garrison_template_id",
                            ],
                            &ctx,
                            warnings,
                        ),
                        garrison_army_manpower_need: leaf_i64_prefer(
                            occ,
                            &["garrison_army_manpower_need"],
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "garrison_army_manpower_need",
                            ],
                            &ctx,
                            warnings,
                        ),
                        garrison_strength_ratio: leaf_f64_prefer(
                            occ,
                            &["garrison_strength_ratio"],
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "garrison_strength_ratio",
                            ],
                            &ctx,
                            warnings,
                        ),
                        garrison_required: leaf_f64_prefer(
                            occ,
                            &["garrison_required"],
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "garrison_required",
                            ],
                            &ctx,
                            warnings,
                        ),
                        garrison_signals_status: leaf_string(
                            occ,
                            &[
                                "_r2_occupation_resistance_compliance_live",
                                "garrison_signals_status",
                            ],
                            &ctx,
                            warnings,
                        ),
                        garrison_manpower_source_tag_db_id: leaf_u32(
                            occ,
                            &["garrison_manpower_source_tag_db_id"],
                            &ctx,
                            warnings,
                        ),
                        occupation_controller_tag_db_id: leaf_u32(
                            occ,
                            &["occupation_controller_tag_db_id"],
                            &ctx,
                            warnings,
                        ),
                        occupied_tag_db_id: leaf_u32(occ, &["occupied_tag_db_id"], &ctx, warnings),
                        garrison_manpower_pool: leaf_rows(
                            occ,
                            &["garrison_manpower_pool"],
                            &ctx,
                            warnings,
                            occupation_manpower_exact_row,
                        ),
                        garrison_equipment: leaf_rows(
                            occ,
                            &["garrison_equipment"],
                            &ctx,
                            warnings,
                            occupation_equipment_exact_row,
                        ),
                        garrison_equipment_complete: leaf_bool(
                            occ,
                            &["garrison_equipment_complete"],
                            &ctx,
                            warnings,
                        ),
                        garrison_equipment_need: leaf_f64_map(
                            occ,
                            &["garrison_equipment_need"],
                            &ctx,
                            warnings,
                        ),
                        garrison_reinforcement_requests: leaf_rows(
                            occ,
                            &["garrison_reinforcement_requests"],
                            &ctx,
                            warnings,
                            garrison_reinforcement_exact_row,
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
    let mut naval_items: Vec<(String, &Value)> = Vec::new();
    let combat_status = match rest.get("naval_combats") {
        None => ViewStatus::DomainAbsent,
        Some(Value::Null) => ViewStatus::DomainUnavailable,
        Some(Value::Array(items)) => {
            naval_items.extend(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| (format!("naval_combats[{i}]"), item)),
            );
            ViewStatus::Present
        }
        Some(Value::Object(items)) => {
            naval_items.extend(
                items
                    .iter()
                    .map(|(key, item)| (format!("naval_combats.{key}"), item)),
            );
            ViewStatus::Present
        }
        Some(other) => {
            warnings.push(format!(
                "naval_combats: expected array or object, found {}; treated as unavailable",
                json_type_name(other)
            ));
            ViewStatus::DomainUnavailable
        }
    };
    let mut naval_combats = Vec::new();
    for (ctx, item) in naval_items {
        let Some(o) = item.as_object() else {
            warnings.push(format!(
                "{ctx}: expected object, found {}; row skipped",
                json_type_name(item)
            ));
            continue;
        };
        let members = parse_naval_members(o, &ctx, warnings);
        let (attacker_country_tag_db_ids, defender_country_tag_db_ids) =
            naval_combat_country_sets(&members);
        naval_combats.push(NavalCombatRow {
            combat_id: first_available_i64([
                leaf_i64(o, &["naval_combat_id"], &ctx, warnings),
                leaf_i64(o, &["combat_id"], &ctx, warnings),
            ]),
            generation: leaf_i64(o, &["generation"], &ctx, warnings),
            location_province: leaf_u32(o, &["location_province"], &ctx, warnings),
            region_id: leaf_u32(o, &["region_id"], &ctx, warnings),
            attacker: leaf_string(o, &["attacker"], &ctx, warnings),
            defender: leaf_string(o, &["defender"], &ctx, warnings),
            result_type: leaf_string(o, &["result_type"], &ctx, warnings),
            progress: leaf_f64(o, &["progress"], &ctx, warnings),
            attacker_member_count: leaf_u32(o, &["attacker_member_count"], &ctx, warnings),
            defender_member_count: leaf_u32(o, &["defender_member_count"], &ctx, warnings),
            attacker_positioning: leaf_f64(o, &["attacker_positioning"], &ctx, warnings),
            defender_positioning: leaf_f64(o, &["defender_positioning"], &ctx, warnings),
            attacker_task_force_ids: leaf_id_list(o, "attacker_task_force_ids", &ctx, warnings),
            defender_task_force_ids: leaf_id_list(o, "defender_task_force_ids", &ctx, warnings),
            attacker_country_tag_db_ids,
            defender_country_tag_db_ids,
            members,
        });
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
    let air_losses = parse_air_losses(rest, warnings);
    let air_region_combat = parse_air_region_combat(rest, warnings);
    let air_status = match (&air_losses, &air_region_combat) {
        (Leaf::Measured(_), _) | (_, Leaf::Measured(_)) => ViewStatus::Present,
        (Leaf::Unobtained, _) | (_, Leaf::Unobtained) => ViewStatus::DomainUnavailable,
        _ => ViewStatus::DomainAbsent,
    };
    NavalAirCombatView {
        status: combine_status(combine_status(combat_status, sunk_status), air_status),
        naval_combats,
        sunk_convoys,
        air_losses,
        air_region_combat,
    }
}

fn parse_naval_members(
    combat: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<NavalCombatMemberRow>> {
    let items = match combat.get("members") {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Unobtained,
        Some(Value::Array(items)) => items,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.members: expected array, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            warnings.push(format!(
                "{ctx}.members[{index}]: expected object; whole list unobtained"
            ));
            return Leaf::Unobtained;
        };
        let rctx = format!("{ctx}.members[{index}]");
        rows.push(NavalCombatMemberRow {
            side: leaf_string(obj, &["side"], &rctx, warnings),
            group_index: leaf_i64(obj, &["group_index"], &rctx, warnings),
            member_index: leaf_i64(obj, &["member_index"], &rctx, warnings),
            is_convoy: leaf_bool(obj, &["is_convoy"], &rctx, warnings),
            ship_instance_id: leaf_i64(obj, &["ship_instance_id"], &rctx, warnings),
            cship_join_validated: leaf_bool(obj, &["cship_join_validated"], &rctx, warnings),
            owner_tag_db_id: leaf_i64(obj, &["owner_tag_db_id"], &rctx, warnings),
            task_force_id: leaf_i64(obj, &["task_force_id"], &rctx, warnings),
            fleet_id: leaf_i64(obj, &["fleet_id"], &rctx, warnings),
            battle_slot_index: leaf_i64(obj, &["battle_slot_index"], &rctx, warnings),
            state_bitmask: leaf_i64(obj, &["state_bitmask"], &rctx, warnings),
            strength: leaf_f64(obj, &["strength"], &rctx, warnings),
            organisation: leaf_f64(obj, &["organisation"], &rctx, warnings),
            sunk_convoys: leaf_i64(obj, &["sunk_convoys"], &rctx, warnings),
        });
    }
    Leaf::Measured(rows)
}

fn naval_combat_country_sets(
    members: &Leaf<Vec<NavalCombatMemberRow>>,
) -> (Leaf<Vec<i64>>, Leaf<Vec<i64>>) {
    let rows = match members {
        Leaf::Absent => return (Leaf::Absent, Leaf::Absent),
        Leaf::Unobtained => return (Leaf::Unobtained, Leaf::Unobtained),
        Leaf::Measured(rows) => rows,
    };
    let mut attacker = BTreeSet::new();
    let mut defender = BTreeSet::new();
    let mut attacker_exact = true;
    let mut defender_exact = true;
    for row in rows {
        let side = match &row.side {
            Leaf::Measured(side) => side.as_str(),
            Leaf::Absent | Leaf::Unobtained => {
                attacker_exact = false;
                defender_exact = false;
                continue;
            }
        };
        let owner = match &row.owner_tag_db_id {
            Leaf::Measured(owner) if *owner > 0 => *owner,
            Leaf::Measured(_) | Leaf::Absent | Leaf::Unobtained => {
                match side {
                    "attacker" => attacker_exact = false,
                    "defender" => defender_exact = false,
                    _ => {
                        attacker_exact = false;
                        defender_exact = false;
                    }
                }
                continue;
            }
        };
        match side {
            "attacker" => {
                attacker.insert(owner);
            }
            "defender" => {
                defender.insert(owner);
            }
            _ => {
                attacker_exact = false;
                defender_exact = false;
            }
        }
    }
    let attacker = if attacker_exact {
        Leaf::Measured(attacker.into_iter().collect())
    } else {
        Leaf::Unobtained
    };
    let defender = if defender_exact {
        Leaf::Measured(defender.into_iter().collect())
    } else {
        Leaf::Unobtained
    };
    (attacker, defender)
}

fn nested_array<'a>(
    rest: &'a Map<String, Value>,
    domain: &str,
    key: &str,
    warnings: &mut Vec<String>,
) -> Leaf<&'a Vec<Value>> {
    match rest.get(domain) {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::Object(obj)) => match obj.get(key) {
            None => Leaf::Absent,
            Some(Value::Null) => Leaf::Unobtained,
            Some(Value::Array(items)) => Leaf::Measured(items),
            Some(other) => {
                warnings.push(format!(
                    "{domain}.{key}: expected array, found {}; treated as unobtained",
                    json_type_name(other)
                ));
                Leaf::Unobtained
            }
        },
        Some(other) => {
            warnings.push(format!(
                "{domain}: expected object, found {}; treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn parse_air_losses(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<AirCombatLossRow>> {
    let items = match nested_array(rest, "naval_air", "_r2_air_combat_losses_live", warnings) {
        Leaf::Absent => return Leaf::Absent,
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Measured(items) => items,
    };
    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            warnings.push(format!(
                "naval_air._r2_air_combat_losses_live[{index}]: expected object, found {}; whole list treated as unobtained",
                json_type_name(item)
            ));
            return Leaf::Unobtained;
        };
        let ctx = format!("naval_air._r2_air_combat_losses_live[{index}]");
        rows.push(AirCombatLossRow {
            owner: leaf_string(obj, &["owner"], &ctx, warnings),
            owner_tag_db_id: leaf_i64(obj, &["owner_tag_db_id"], &ctx, warnings),
            wing_id: leaf_i64(obj, &["wing_id"], &ctx, warnings),
            generation: leaf_i64(obj, &["generation"], &ctx, warnings),
            mission_mask: leaf_i64(obj, &["mission_mask"], &ctx, warnings),
            mission_region: leaf_u32(obj, &["mission_region"], &ctx, warnings),
            old_count: leaf_i64(obj, &["old_count"], &ctx, warnings),
            new_count: leaf_i64(obj, &["new_count"], &ctx, warnings),
            count_delta: leaf_i64(obj, &["count_delta"], &ctx, warnings),
            old_equipment_amount_sum: leaf_f64(obj, &["old_equipment_amount_sum"], &ctx, warnings),
            new_equipment_amount_sum: leaf_f64(obj, &["new_equipment_amount_sum"], &ctx, warnings),
            equipment_amount_delta: leaf_f64(obj, &["equipment_amount_delta"], &ctx, warnings),
            flags: leaf_i64(obj, &["flags"], &ctx, warnings),
            air_accidents: leaf_i64(obj, &["air_accidents"], &ctx, warnings),
            experience: leaf_f64(obj, &["experience"], &ctx, warnings),
        });
    }
    Leaf::Measured(rows)
}

fn parse_air_region_combat(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<AirRegionCombatRow>> {
    let items = match nested_array(rest, "naval_air", "_r2_air_region_combat_stats", warnings) {
        Leaf::Absent => return Leaf::Absent,
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Measured(items) => items,
    };
    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            warnings.push(format!(
                "naval_air._r2_air_region_combat_stats[{index}]: expected object, found {}; whole list treated as unobtained",
                json_type_name(item)
            ));
            return Leaf::Unobtained;
        };
        let ctx = format!("naval_air._r2_air_region_combat_stats[{index}]");
        rows.push(AirRegionCombatRow {
            owner: leaf_string(obj, &["owner"], &ctx, warnings),
            owner_tag_db_id: leaf_i64(obj, &["owner_tag_db_id"], &ctx, warnings),
            region_id: leaf_u32(obj, &["region_id"], &ctx, warnings),
            side_vector: leaf_i64(obj, &["side_vector"], &ctx, warnings),
            selection_flag: leaf_i64(obj, &["selection_flag"], &ctx, warnings),
            classification_flag: leaf_i64(obj, &["classification_flag"], &ctx, warnings),
            copied_flag: leaf_i64(obj, &["copied_flag"], &ctx, warnings),
            wing_id: leaf_i64(obj, &["wing_id"], &ctx, warnings),
            date_raw: leaf_i64(obj, &["date_raw"], &ctx, warnings),
            plane_count: leaf_i64(obj, &["plane_count"], &ctx, warnings),
            mission_mask: leaf_i64(obj, &["mission_mask"], &ctx, warnings),
            attributed_country: leaf_string(obj, &["attributed_country"], &ctx, warnings),
            attributed_country_tag_db_id: leaf_i64(
                obj,
                &["attributed_country_tag_db_id"],
                &ctx,
                warnings,
            ),
            damage: parse_air_damage(obj, &ctx, warnings),
        });
    }
    Leaf::Measured(rows)
}

fn parse_air_damage(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<AirRegionDamageRow>> {
    let items = match obj.get("damage") {
        None => return Leaf::Absent,
        Some(Value::Null) => return Leaf::Unobtained,
        Some(Value::Array(items)) => items,
        Some(other) => {
            warnings.push(format!(
                "{ctx}.damage: expected array, found {}; treated as unobtained",
                json_type_name(other)
            ));
            return Leaf::Unobtained;
        }
    };
    let mut rows = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(row) = item.as_object() else {
            warnings.push(format!(
                "{ctx}.damage[{index}]: expected object, found {}; whole list treated as unobtained",
                json_type_name(item)
            ));
            return Leaf::Unobtained;
        };
        let rctx = format!("{ctx}.damage[{index}]");
        rows.push(AirRegionDamageRow {
            channel: leaf_string(row, &["channel"], &rctx, warnings),
            cause_type: leaf_i64(row, &["cause_type"], &rctx, warnings),
            cause: leaf_string(row, &["cause"], &rctx, warnings),
            amount: leaf_f64(row, &["amount"], &rctx, warnings),
        });
    }
    Leaf::Measured(rows)
}

fn build_dynamic_collection_trace(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> DynamicCollectionTraceView {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Root {
        schema_version: u32,
        authority: String,
        elapsed_wall_ms: u64,
        frame_hour: u64,
        collections: Vec<DynamicCollectionTraceRow>,
    }

    let Some(value) = rest.get("_r2_dynamic_collection_trace_live") else {
        return DynamicCollectionTraceView {
            status: ViewStatus::DomainAbsent,
            frame_hour: None,
            collections: Vec::new(),
        };
    };
    if value.is_null() {
        return DynamicCollectionTraceView {
            status: ViewStatus::DomainUnavailable,
            frame_hour: None,
            collections: Vec::new(),
        };
    }
    let root: Root = match serde::Deserialize::deserialize(value) {
        Ok(root) => root,
        Err(error) => {
            warnings.push(format!(
                "_r2_dynamic_collection_trace_live malformed: {error}; treated as unavailable"
            ));
            return DynamicCollectionTraceView {
                status: ViewStatus::DomainUnavailable,
                frame_hour: None,
                collections: Vec::new(),
            };
        }
    };
    let _elapsed_wall_ms = root.elapsed_wall_ms;
    let mut ids = BTreeSet::new();
    if root.schema_version != 1
        || root.authority != "exact_a729_controller"
        || root.collections.iter().any(|row| {
            row.frame_hour != root.frame_hour || !ids.insert(row.collection_instance_id.as_str())
        })
    {
        warnings.push("_r2_dynamic_collection_trace_live authority/frame/identity rejected".into());
        return DynamicCollectionTraceView {
            status: ViewStatus::DomainUnavailable,
            frame_hour: None,
            collections: Vec::new(),
        };
    }
    DynamicCollectionTraceView {
        status: ViewStatus::Present,
        frame_hour: Some(root.frame_hour),
        collections: root.collections,
    }
}

fn build_design_catalog_context(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> DesignCatalogContextView {
    let key = "_r2_design_catalog_context_live";
    let Some(value) = rest.get(key) else {
        return DesignCatalogContextView {
            status: ViewStatus::DomainAbsent,
            context: None,
        };
    };
    if value.is_null() {
        return DesignCatalogContextView {
            status: ViewStatus::DomainUnavailable,
            context: None,
        };
    }
    let context: DesignCatalogContext = match serde::Deserialize::deserialize(value) {
        Ok(context) => context,
        Err(error) => {
            warnings.push(format!("{key} malformed: {error}; treated as unavailable"));
            return DesignCatalogContextView {
                status: ViewStatus::DomainUnavailable,
                context: None,
            };
        }
    };
    let family_matches = matches!(
        (context.mode, context.family.as_str()),
        (0, "countryequipmentdesignerview")
            | (1, "tank_designer_view")
            | (2, "plane_designer_view")
    );
    let selected = context
        .selected_country_tag_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let designs = context
        .ordered_design_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if context.schema_version != 1
        || context.authority != "exact_a729_designer_view"
        || context.completeness_record_identity.trim().is_empty()
        || context.transition_sequence == 0
        || context.current_country_tag_id == 0
        || !family_matches
        || selected.len() != context.selected_country_tag_ids.len()
        || selected.contains(&0)
        || designs.len() != context.ordered_design_ids.len()
        || designs.contains(&0)
        || (!context.active && !context.ordered_design_ids.is_empty())
    {
        warnings.push(format!("{key} authority/context identity rejected"));
        return DesignCatalogContextView {
            status: ViewStatus::DomainUnavailable,
            context: None,
        };
    }
    DesignCatalogContextView {
        status: ViewStatus::Present,
        context: Some(context),
    }
}

fn build_hud_surface_activation(
    rest: &Map<String, Value>,
    expected_hour: i64,
    warnings: &mut Vec<String>,
) -> HudSurfaceActivationView {
    let key = "_r2_hud_surface_activation_live";
    let Some(value) = rest.get(key) else {
        return HudSurfaceActivationView {
            status: ViewStatus::DomainAbsent,
            activation: None,
        };
    };
    if value.is_null() {
        return HudSurfaceActivationView {
            status: ViewStatus::DomainUnavailable,
            activation: None,
        };
    }
    let activation: HudSurfaceActivation = match serde::Deserialize::deserialize(value) {
        Ok(activation) => activation,
        Err(error) => {
            warnings.push(format!("{key} malformed: {error}; treated as unavailable"));
            return HudSurfaceActivationView {
                status: ViewStatus::DomainUnavailable,
                activation: None,
            };
        }
    };
    let required = [
        HudSurfaceActivationKind::Topbar,
        HudSurfaceActivationKind::Minimap,
        HudSurfaceActivationKind::Mapmodes,
        HudSurfaceActivationKind::TheatreSelector,
    ];
    let designer = |surface| {
        matches!(
            surface,
            HudSurfaceActivationKind::DesignerCountryEquipment
                | HudSurfaceActivationKind::DesignerTank
                | HudSurfaceActivationKind::DesignerPlane
        )
    };
    let mut seen = BTreeSet::new();
    let mut prior = None;
    let mut designer_count = 0usize;
    let rows_valid = activation.active_surfaces.iter().all(|row| {
        let canonical = prior.map_or(true, |old| old < row.surface);
        prior = Some(row.surface);
        if designer(row.surface) {
            designer_count += 1;
        }
        canonical
            && seen.insert(row.surface)
            && !row.runtime_root_name.trim().is_empty()
            && row.runtime_root_name.len() <= 512
            && !row.runtime_root_name.contains('/')
            && !row.runtime_root_name.contains('\\')
            && !row.runtime_root_name.chars().any(char::is_control)
    });
    let required_present = required.into_iter().all(|surface| seen.contains(&surface));
    let expected_len = 4 + designer_count;
    let expected_hour = u64::try_from(expected_hour).ok();
    if activation.schema_version != 1
        || activation.authority != "exact_a729_hud_controller_roots"
        || Some(activation.frame_hour) != expected_hour
        || activation.observation_sequence == 0
        || !rows_valid
        || !required_present
        || designer_count > 1
        || activation.active_surfaces.len() != expected_len
    {
        warnings.push(format!("{key} authority/hour/root partition rejected"));
        return HudSurfaceActivationView {
            status: ViewStatus::DomainUnavailable,
            activation: None,
        };
    }
    HudSurfaceActivationView {
        status: ViewStatus::Present,
        activation: Some(activation),
    }
}

fn build_scripted_gui(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> ScriptedGuiView {
    let (by_country, cstatus) = country_map(
        rest,
        warnings,
        || Leaf::Unobtained,
        |obj, ctx, w| match leaf_f64_map(obj, &["extra", "_r2_script_variables"], ctx, w) {
            Leaf::Absent => leaf_f64_map(obj, &["script_variables"], ctx, w),
            production => production,
        },
    );
    let (country_flags, flag_status) = country_map(
        rest,
        warnings,
        || Leaf::Unobtained,
        |obj, ctx, w| object_keys(obj, &["extra", "script_flags"], ctx, w),
    );
    let (runtime_by_country, runtime_status) = country_map(
        rest,
        warnings,
        || Leaf::Unobtained,
        |obj, ctx, w| {
            leaf_scripted_gui_runtime_inputs(obj, &["extra", "scripted_gui_runtime"], ctx, w)
        },
    );
    let world_variables = leaf_f64_map(rest, &["world", "variables"], "snapshot", warnings);
    let world_flags = object_keys(rest, &["world", "flags"], "world.flags", warnings);
    let world_status = match (&world_variables, &world_flags) {
        (Leaf::Measured(_), _) | (_, Leaf::Measured(_)) => ViewStatus::Present,
        (Leaf::Unobtained, _) | (_, Leaf::Unobtained) => ViewStatus::DomainUnavailable,
        _ => ViewStatus::DomainAbsent,
    };
    ScriptedGuiView {
        status: combine_status(
            combine_status(cstatus, flag_status),
            combine_status(runtime_status, world_status),
        ),
        by_country,
        country_flags,
        runtime_by_country,
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

fn legacy_fired_event(event_id: String) -> FiredEventRow {
    FiredEventRow {
        event_id: Leaf::Measured(event_id),
        recipient_tag: Leaf::Absent,
        fire_date: Leaf::Absent,
        fire_hour: Leaf::Absent,
        fire_tick: Leaf::Absent,
        event_sequence: Leaf::Absent,
        visible_option_ordinals: Leaf::Absent,
        chosen_option_ordinal: Leaf::Absent,
        selected_desc_key: Leaf::Absent,
        shown_picture_gfx: Leaf::Absent,
        popup_subtype: Leaf::Absent,
        effective_timeout_hours: Leaf::Absent,
        timeout_tick: Leaf::Absent,
        resolved_tick: Leaf::Absent,
        resolution_reason: Leaf::Absent,
        runtime_title_text_context_id: Leaf::Absent,
        runtime_description_text_context_id: Leaf::Absent,
        runtime_option_text_contexts: Leaf::Absent,
    }
}

fn leaf_fired_event_option_text_contexts(
    obj: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<FiredEventOptionTextContextRow>> {
    match obj.get("runtime_option_text_contexts") {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::Array(items)) => {
            let mut rows = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let Some(item) = item.as_object() else {
                    warnings.push(format!(
                        "{ctx}.runtime_option_text_contexts[{index}]: expected object, found {}; list treated as unobtained",
                        json_type_name(item)
                    ));
                    return Leaf::Unobtained;
                };
                let item_ctx = format!("{ctx}.runtime_option_text_contexts[{index}]");
                rows.push(FiredEventOptionTextContextRow {
                    definition_ordinal: leaf_i64(
                        item,
                        &["definition_ordinal"],
                        &item_ctx,
                        warnings,
                    ),
                    context_id: leaf_string(item, &["context_id"], &item_ctx, warnings),
                });
            }
            Leaf::Measured(rows)
        }
        Some(other) => {
            warnings.push(format!(
                "{ctx}.runtime_option_text_contexts: expected array, found {}; treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn build_fired_events(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> (FiredEventsView, ViewStatus) {
    let (world_events, status) = object_at_path(rest, &["world_events"], "world_events", warnings);
    let Some(world_events) = world_events else {
        return (FiredEventsView::with_status(status), status);
    };
    let history_complete = leaf_bool(
        world_events,
        &["fired_events_complete"],
        "world_events",
        warnings,
    );
    let events = match world_events.get("fired_events") {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::Array(items)) => {
            let mut rows = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                if let Some(id) = value_to_id_string(item) {
                    rows.push(legacy_fired_event(id));
                    continue;
                }
                let Some(obj) = item.as_object() else {
                    warnings.push(format!(
                        "world_events.fired_events[{index}]: expected event id or object, found {}; whole list treated as unobtained",
                        json_type_name(item)
                    ));
                    return (
                        FiredEventsView {
                            events: Leaf::Unobtained,
                            history_complete,
                        },
                        ViewStatus::DomainUnavailable,
                    );
                };
                let ctx = format!("world_events.fired_events[{index}]");
                rows.push(FiredEventRow {
                    event_id: leaf_id_string(obj, &["event_id"], &ctx, warnings),
                    recipient_tag: leaf_string(obj, &["recipient_tag"], &ctx, warnings),
                    fire_date: leaf_string(obj, &["fire_date"], &ctx, warnings),
                    fire_hour: leaf_i64(obj, &["fire_hour"], &ctx, warnings),
                    fire_tick: leaf_i64(obj, &["fire_tick"], &ctx, warnings),
                    event_sequence: leaf_i64(obj, &["event_sequence"], &ctx, warnings),
                    visible_option_ordinals: leaf_i64_list(
                        obj,
                        &["visible_option_ordinals"],
                        &ctx,
                        warnings,
                    ),
                    chosen_option_ordinal: leaf_i64(
                        obj,
                        &["chosen_option_ordinal"],
                        &ctx,
                        warnings,
                    ),
                    selected_desc_key: leaf_string(obj, &["selected_desc_key"], &ctx, warnings),
                    shown_picture_gfx: leaf_string(obj, &["shown_picture_gfx"], &ctx, warnings),
                    popup_subtype: leaf_string(obj, &["popup_subtype"], &ctx, warnings),
                    effective_timeout_hours: leaf_i64(
                        obj,
                        &["effective_timeout_hours"],
                        &ctx,
                        warnings,
                    ),
                    timeout_tick: leaf_i64(obj, &["timeout_tick"], &ctx, warnings),
                    resolved_tick: leaf_i64(obj, &["resolved_tick"], &ctx, warnings),
                    resolution_reason: leaf_string(obj, &["resolution_reason"], &ctx, warnings),
                    runtime_title_text_context_id: leaf_string(
                        obj,
                        &["runtime_title_text_context_id"],
                        &ctx,
                        warnings,
                    ),
                    runtime_description_text_context_id: leaf_string(
                        obj,
                        &["runtime_description_text_context_id"],
                        &ctx,
                        warnings,
                    ),
                    runtime_option_text_contexts: leaf_fired_event_option_text_contexts(
                        obj, &ctx, warnings,
                    ),
                });
            }
            Leaf::Measured(rows)
        }
        Some(other) => {
            warnings.push(format!(
                "world_events.fired_events: expected array, found {}; treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    };
    let status = if matches!(events, Leaf::Unobtained) {
        ViewStatus::DomainUnavailable
    } else {
        ViewStatus::Present
    };
    (
        FiredEventsView {
            events,
            history_complete,
        },
        status,
    )
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

    #[test]
    fn a3_speed_events_keep_raw_canonical_and_reject_out_of_range() {
        let valid = serde_json::json!({
            "speed_events": [{"hour_index": 100, "speed": 4, "paused": false}]
        });
        let mut warnings = Vec::new();
        let parsed = leaf_speed_events(valid.as_object().unwrap(), &mut warnings);
        let rows = parsed.measured().expect("valid raw speed rows").clone();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(rows[0].speed, Leaf::Measured(4));
        assert_eq!(rows[0].displayed_speed(), Leaf::Measured(5));

        let invalid = serde_json::json!({
            "speed_events": [{"hour_index": 100, "speed": 5, "paused": false}]
        });
        let mut warnings = Vec::new();
        assert_eq!(
            leaf_speed_events(invalid.as_object().unwrap(), &mut warnings),
            Leaf::Unobtained
        );
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("outside 0..=4")));
    }

    #[test]
    fn country_identity_accepts_domain18_root_cosmetic_tag() {
        let value = serde_json::json!({
            "countries": {
                "GER": {
                    "cosmetic_tag": "GER_reich",
                    "politics": {"ruling_party": "fascism"}
                }
            }
        });
        let mut warnings = Vec::new();
        let (countries, status) =
            build_country_identities(value.as_object().unwrap(), &mut warnings);

        assert_eq!(status, ViewStatus::Present);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(
            countries["GER"].cosmetic_tag,
            Leaf::Measured("GER_reich".to_string())
        );
        assert_eq!(
            countries["GER"].ruling_ideology,
            Leaf::Measured("fascism".to_string())
        );
    }

    #[test]
    fn fired_event_view_preserves_rich_history_order_and_legacy_ids() {
        let value = serde_json::json!({
            "world_events": {
                "fired_events_complete": true,
                "fired_events": [
                    {
                        "event_id": "test.1",
                        "recipient_tag": "GER",
                        "fire_date": "1936.1.2.3",
                        "fire_hour": 27,
                        "fire_tick": 27004,
                        "event_sequence": 8,
                        "visible_option_ordinals": [0, 2],
                        "chosen_option_ordinal": 2,
                        "selected_desc_key": "test.1.d2",
                        "shown_picture_gfx": "GFX_report_event_test",
                        "popup_subtype": "country",
                        "effective_timeout_hours": 312,
                        "timeout_tick": 28000,
                        "resolved_tick": 27009,
                        "resolution_reason": "selected",
                        "runtime_title_text_context_id": "event:title:8",
                        "runtime_description_text_context_id": "event:description:8",
                        "runtime_option_text_contexts": [
                            {"definition_ordinal": 2, "context_id": "event:option:8:2"}
                        ]
                    },
                    "legacy.2"
                ]
            }
        });
        let mut warnings = Vec::new();
        let (view, status) = build_fired_events(value.as_object().unwrap(), &mut warnings);
        assert_eq!(status, ViewStatus::Present);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(view.history_complete, Leaf::Measured(true));
        let rows = view.events.measured().expect("fired event rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].event_id, Leaf::Measured("test.1".into()));
        assert_eq!(rows[0].visible_option_ordinals, Leaf::Measured(vec![0, 2]));
        assert_eq!(rows[0].event_sequence, Leaf::Measured(8));
        assert_eq!(
            rows[0].runtime_title_text_context_id,
            Leaf::Measured("event:title:8".into())
        );
        assert_eq!(
            rows[0]
                .runtime_option_text_contexts
                .measured()
                .expect("option contexts")[0]
                .definition_ordinal,
            Leaf::Measured(2)
        );
        assert_eq!(rows[1].event_id, Leaf::Measured("legacy.2".into()));
        assert_eq!(rows[1].recipient_tag, Leaf::Absent);
        assert_eq!(rows[1].event_sequence, Leaf::Absent);
    }

    #[test]
    fn fired_event_view_wrong_typed_member_fails_closed() {
        let value = serde_json::json!({
            "world_events": {"fired_events_complete": true, "fired_events": [false]}
        });
        let mut warnings = Vec::new();
        let (view, status) = build_fired_events(value.as_object().unwrap(), &mut warnings);
        assert_eq!(status, ViewStatus::DomainUnavailable);
        assert_eq!(view.events, Leaf::Unobtained);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn occupation_view_exposes_domain52_and_domain66_garrison_carriers() {
        let value = serde_json::json!({
            "occupation_history_complete": true,
            "occupation_history": [{
                "stable_event_id": "GER|1942.8.15.1|891|0",
                "source_ordinal": 0,
                "controller_tag": "GER",
                "state_id": 891,
                "occupied_country_tag": "FRA",
                "event_hour_or_engine_date": "1942.8.15.1",
                "attack_blocked": true,
                "resistance_activity": null,
                "lost_manpower": 33,
                "equipment_loss_rows": [{
                    "source_ordinal": 0,
                    "equipment_id": 438,
                    "equipment_type": 70,
                    "equipment_token": "support_equipment_1",
                    "amount": 1.0
                }],
                "equipment_loss_children_complete": true
            }],
            "states": {
                "891": {
                    "occupation_law": null,
                    "occupation": {
                        "resistance": 22.3,
                        "resistance_speed": 0.025,
                        "compliance": 65.0,
                        "compliance_speed": -0.012,
                        "occupier_tag": "47",
                        "occupied_country_tag": "180",
                        "occupation_law_token_id": 304778569,
                        "occupation_law_token_encoding": "fnv1a32_of_law_token",
                        "occupation_controller_tag_db_id": 47,
                        "occupied_tag_db_id": 180,
                        "garrison_manpower_source_tag_db_id": 47,
                        "garrison_template_id": 4183,
                        "garrison_army_manpower_need": 13869,
                        "garrison_strength_ratio": 0.9375,
                        "garrison_required": 0.55477,
                        "garrison_manpower_pool": [{
                            "source_ordinal": 0,
                            "tag": "GER",
                            "value": 13869.0
                        }],
                        "garrison_equipment": [{
                            "source_ordinal": 0,
                            "equipment_id": 90,
                            "equipment_type": 70,
                            "equipment_token": "infantry_equipment_0",
                            "amount": 1664.31
                        }],
                        "garrison_equipment_complete": true,
                        "garrison_equipment_need": {
                            "infantry_equipment": 1664.31
                        },
                        "garrison_reinforcement_requests": [{
                            "source_ordinal": 0,
                            "total_progress": 7.57643,
                            "need": {"infantry_equipment": 661.31},
                            "date": "1942.8.18.2"
                        }],
                        "_r2_occupation_resistance_compliance_live": {
                            "occupation_law_token_id": 1626184329,
                            "occupation_law_token_encoding": "fnv1a32_name_hash",
                            "occupation_law_token_source": "controller_country_default",
                            "garrison_template_id": 4183,
                            "garrison_army_manpower_need": 14000,
                            "garrison_strength_ratio": 0.875,
                            "garrison_required": 2.5
                        }
                    }
                }
            }
        });
        let mut warnings = Vec::new();
        let (rows, status) = build_occupation(value.as_object().unwrap(), &mut warnings);
        assert_eq!(status, ViewStatus::Present);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let row = rows.get(&891).expect("state 891 occupation row");
        assert_eq!(row.occupier_tag, Leaf::Measured("47".into()));
        assert_eq!(row.occupation_law_token_id, Leaf::Measured(304_778_569));
        assert_eq!(
            row.occupation_law_token_encoding,
            Leaf::Measured("fnv1a32_of_law_token".into())
        );
        assert_eq!(row.garrison_template_id, Leaf::Measured(4183));
        assert_eq!(row.garrison_army_manpower_need, Leaf::Measured(13_869));
        assert_eq!(row.garrison_strength_ratio, Leaf::Measured(0.9375));
        assert_eq!(row.garrison_required, Leaf::Measured(0.55477));
        assert_eq!(row.garrison_manpower_source_tag_db_id, Leaf::Measured(47));
        let equipment = row
            .garrison_equipment
            .measured()
            .expect("garrison equipment");
        assert_eq!(equipment.len(), 1);
        assert_eq!(
            equipment[0].equipment_token,
            Leaf::Measured("infantry_equipment_0".into())
        );
        assert_eq!(row.garrison_equipment_complete, Leaf::Measured(true));
        assert_eq!(
            row.garrison_equipment_need
                .measured()
                .and_then(|need| need.get("infantry_equipment"))
                .copied(),
            Some(1664.31)
        );
        let requests = row
            .garrison_reinforcement_requests
            .measured()
            .expect("garrison reinforcement requests");
        assert_eq!(requests[0].total_progress, Leaf::Measured(7.57643));

        let (history, history_complete) =
            build_occupation_history(value.as_object().unwrap(), &mut warnings);
        assert_eq!(history_complete, Leaf::Measured(true));
        let history = history.measured().expect("occupation history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].attack_blocked, Leaf::Measured(true));
        assert_eq!(history[0].lost_manpower, Leaf::Measured(33));
        assert_eq!(
            history[0]
                .equipment_loss_rows
                .measured()
                .expect("equipment losses")[0]
                .equipment_token,
            Leaf::Measured("support_equipment_1".into())
        );
    }

    #[test]
    fn politics_detail_exposes_complete_live_classification_and_advisor_contract() {
        let value: Value = serde_json::from_str(
            r#"{
              "countries": {"GER": {
                "politics": {
                  "stability": 0.75,
                  "war_support": 0.6,
                  "political_power": 42.5,
                  "command_power": 18.0,
                  "ruling_party": "fascism",
                  "party_popularities": {"fascism": 60.0, "democratic": 20.0},
                  "ideas": ["war_economy", "GER_advisor"],
                  "ideas_all": ["war_economy", "GER_advisor"],
                  "ideas_other": ["GER_advisor"],
                  "national_spirits": [],
                  "idea_groups": {"economy": ["war_economy"], "political_advisor": ["GER_advisor"]},
                  "advisor_ideas": {"political_advisor": ["GER_advisor"]},
                  "laws": {"economy": "war_economy", "trade_laws": "export_focus", "mobilization_laws": "limited_conscription"},
                  "idea_count": 2,
                  "party_count": 4,
                  "election_frequency": 48,
                  "last_election": "1936.1.1.1",
                  "elections_allowed": false,
                  "low_confidence": false,
                  "appointed_advisors": ["GER_advisor"],
                  "appointed_advisor_details": [{"idea_token": "GER_advisor", "slot": "political_advisor", "ledger": "political", "character": "GER_character"}],
                  "appointed_advisor_slots": {"political_advisor": ["GER_advisor"]},
                  "appointed_advisor_ledgers": {"political": ["GER_advisor"]}
                },
                "abilities": [{"ability": "force_attack"}],
                "active_abilities": {
                  "complete": true,
                  "rows": [{
                    "ability_id": "force_attack",
                    "available": false,
                    "active": true,
                    "activation_hour": 17,
                    "cooldown_hours": 504,
                    "cooldown_remaining_hours": 336,
                    "cost_command_power": 22.0,
                    "target_kind": "army_leader",
                    "target_id": 73,
                    "duration_hours": 168,
                    "duration_remaining_hours": 120,
                    "display_order": 0,
                    "icon": "GFX_ability_force_attack",
                    "tooltip": "Force Attack",
                    "disabled_reason": "Already active"
                  }]
                }
              }}
            }"#,
        )
        .expect("politics fixture json");
        let rest = value.as_object().expect("politics fixture object");
        let mut warnings = Vec::new();
        let (views, status) = build_politics_detail(rest, &mut warnings);
        assert_eq!(status, ViewStatus::Present);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let view = views.get("GER").expect("GER politics detail");
        assert_eq!(view.ruling_party, Leaf::Measured("fascism".to_string()));
        assert_eq!(view.idea_count, Leaf::Measured(2));
        assert_eq!(view.elections_allowed, Leaf::Measured(false));
        assert_eq!(
            view.laws,
            Leaf::Measured(BTreeMap::from([
                ("economy".to_string(), "war_economy".to_string()),
                (
                    "mobilization_laws".to_string(),
                    "limited_conscription".to_string()
                ),
                ("trade_laws".to_string(), "export_focus".to_string()),
            ]))
        );
        assert_eq!(
            view.appointed_advisor_details,
            Leaf::Measured(vec![AppointedAdvisorRow {
                idea_token: Leaf::Measured("GER_advisor".to_string()),
                slot: Leaf::Measured("political_advisor".to_string()),
                ledger: Leaf::Measured("political".to_string()),
                character: Leaf::Measured("GER_character".to_string()),
            }])
        );
        assert_eq!(
            view.command_abilities,
            Leaf::Measured(vec!["force_attack".to_string()])
        );
        assert_eq!(view.command_abilities_complete, Leaf::Measured(true));
        let Leaf::Measured(rows) = &view.command_ability_rows else {
            panic!("exact command-ability rows must be measured");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].ability_id,
            Leaf::Measured("force_attack".to_string())
        );
        assert_eq!(rows[0].available, Leaf::Measured(false));
        assert_eq!(rows[0].active, Leaf::Measured(true));
        assert_eq!(rows[0].target_id, Leaf::Measured("73".to_string()));
        assert_eq!(rows[0].display_order, Leaf::Measured(0));
        assert_eq!(
            rows[0].disabled_reason,
            Leaf::Measured("Already active".to_string())
        );
    }

    #[test]
    fn command_ability_rows_fail_closed_without_inference() {
        let value = serde_json::json!({
            "countries": {"GER": {
                "active_abilities": {
                    "complete": null,
                    "rows": [{
                        "ability_id": "last_stand",
                        "available": "yes",
                        "active": false
                    }]
                }
            }}
        });
        let rest = value.as_object().expect("ability fixture object");
        let mut warnings = Vec::new();
        let (views, status) = build_politics_detail(rest, &mut warnings);
        assert_eq!(status, ViewStatus::Present);
        let view = views.get("GER").expect("GER ability view");
        assert_eq!(view.command_abilities_complete, Leaf::Unobtained);
        let Leaf::Measured(rows) = &view.command_ability_rows else {
            panic!("row container should remain measured");
        };
        assert_eq!(rows[0].ability_id, Leaf::Measured("last_stand".to_string()));
        assert_eq!(rows[0].available, Leaf::Unobtained);
        assert_eq!(rows[0].active, Leaf::Measured(false));
        assert_eq!(rows[0].cooldown_remaining_hours, Leaf::Absent);
        assert_eq!(rows[0].tooltip, Leaf::Absent);
        assert!(warnings.iter().any(|warning| warning.contains("available")));
    }

    #[test]
    fn intel_view_types_existing_domain53_raw_slice_without_relabeling() {
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 9,
            "countries": {"GER": {"intel": {"domain53_extra": {
                "agency_scalars": {
                    "max_operatives_unresolved": 3,
                    "usable_slots_unresolved": 2,
                    "operative_count": 2,
                    "cryptology_target_count": 1,
                    "defense_raw": 500000,
                    "upgrade_progress_raw": 25000,
                    "name_hash": 12345678901234u64,
                    "icon_hash": 987654321u64
                },
                "raw_capacity_cluster": {
                    "value_0xe4": 3,
                    "value_0xf0": 2,
                    "value_0xf4": 3,
                    "proof_hash": 51966,
                    "semantic_status": "raw_offsets_unresolved"
                },
                "operative_lifecycle_rows": [{
                    "op_id": 7,
                    "detected": false,
                    "captured_unvalidated": false,
                    "skill": 2,
                    "experience_raw": 30000,
                    "name_hash": 444u64,
                    "state_hash": 555,
                    "lifecycle_flag": 47
                }],
                "cryptology_targets": [{
                    "target_tag_db_id": 11,
                    "amount_raw": 960000,
                    "decrypting": true
                }]
            }}}}
        }));
        let intel = frame.intel("GER").expect("GER intel");
        let agency = intel
            .domain53_agency_scalars_raw
            .measured()
            .expect("typed raw agency scalars");
        assert_eq!(agency.value_0x128_unvalidated, Leaf::Measured(500000));
        assert_eq!(agency.value_0xc8_unvalidated, Leaf::Measured(25000));
        assert_eq!(agency.name_hash, Leaf::Measured(12345678901234));
        let capacity = intel
            .domain53_agency_capacity_raw
            .measured()
            .expect("typed raw capacity");
        assert_eq!(
            capacity.semantic_status,
            Leaf::Measured("raw_offsets_unresolved".to_string())
        );
        let operatives = intel
            .domain53_operative_lifecycle_raw
            .measured()
            .expect("typed raw operatives");
        assert_eq!(operatives[0].captured_unvalidated, Leaf::Measured(false));
        let cryptology = intel
            .domain53_cryptology_targets_raw
            .measured()
            .expect("typed raw cryptology");
        assert_eq!(cryptology[0].amount_raw, Leaf::Measured(960000));
    }

    #[test]
    fn trade_view_preserves_resource_semantics_order_and_complete_totals() {
        let pool = |oil: f64, steel: f64| {
            BTreeMap::from([
                ("oil", oil),
                ("aluminium", 0.0),
                ("rubber", 0.0),
                ("tungsten", 0.0),
                ("steel", steel),
                ("chromium", 0.0),
                ("coal", 0.0),
            ])
        };
        let resource_rows: Vec<Value> = TRADE_RESOURCE_ORDER
            .iter()
            .enumerate()
            .map(|(visible_order, resource)| {
                let oil = *resource == "oil";
                let steel = *resource == "steel";
                serde_json::json!({
                    "resource": resource,
                    "visible_order": visible_order,
                    "balance": if oil || steel { 5.0 } else { 0.0 },
                    "produced": if oil { 10.0 } else if steel { 20.0 } else { 0.0 },
                    "imported": if oil { 4.0 } else if steel { 2.0 } else { 0.0 },
                    "exported": if oil { 1.0 } else if steel { 4.0 } else { 0.0 },
                    "special_project": 0.0,
                    "production_line": if oil { 8.0 } else if steel { 13.0 } else { 0.0 },
                    "needed": if oil { -12.0 } else { 0.0 },
                    "needed_current": if oil { Some(-12.0) } else { None },
                    "needed_max": if oil { Some(-20.0) } else { None },
                    "needed_type": if oil { "current_max" } else { "surplus" }
                })
            })
            .collect();
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 7,
            "countries": {
                "GER": {
                    "economy": {
                        "resources": {
                            "complete": true,
                            "produced": pool(10.0, 20.0),
                            "imported": pool(4.0, 2.0),
                            "used": pool(8.0, 13.0),
                            "to_export": pool(2.0, 5.0),
                            "base_export": pool(3.0, 6.0),
                            "exported": pool(1.0, 4.0),
                            "available_derived": pool(9.0, 16.0)
                        },
                        "resource_trade": {
                            "resource_rows_complete": true,
                            "resource_rows": resource_rows,
                            "deals_complete": true,
                            "deals": [
                                {
                                    "deal_id": 41, "counterpart": "SWE",
                                    "direction": "import", "resource": "steel",
                                    "requested": 8.0, "delivered": 6.0,
                                    "available_export": 12.0,
                                    "civilian_factories": 1,
                                    "factory_cost_per_resource": 0.125,
                                    "convoys": {"assigned": 2, "required": 3},
                                    "route_kind": "sea", "delivery_efficiency": 0.75,
                                    "no_trade_influence": false, "no_resources": false,
                                    "visible_order": 0
                                },
                                {
                                    "deal_id": 42, "counterpart": "SOV",
                                    "direction": "import", "resource": "oil",
                                    "requested": 4.0, "delivered": 4.0,
                                    "available_export": 30.0,
                                    "civilian_factories": 1,
                                    "factory_cost_per_resource": 0.25,
                                    "convoys": {"assigned": 0, "required": 0},
                                    "route_kind": "land", "delivery_efficiency": 1.0,
                                    "visible_order": 1
                                },
                                {
                                    "deal_id": 43, "counterpart": "SWE",
                                    "direction": "import", "resource": "oil",
                                    "requested": 2.0, "delivered": 1.0,
                                    "available_export": 12.0,
                                    "civilian_factories": 1,
                                    "factory_cost_per_resource": 0.5,
                                    "convoys": {"assigned": 1, "required": 2},
                                    "route_kind": "sea", "delivery_efficiency": 0.5,
                                    "visible_order": 2
                                }
                            ],
                            "factories": {"total": 25, "used": 11, "available": 14}
                        }
                    },
                    "diplomacy": {"lend_lease": []}
                }
            },
            "order_of_battle": {
                "army_groups": [
                    {"id": 500, "owner": "GER", "leader": 100}
                ],
                "armies": [],
                "fleets": [
                    {"id": 600, "owner": "GER", "leader": 101}
                ]
            }
        }));
        let view = frame.trade("GER").expect("GER trade");
        assert_eq!(view.resource_pools_complete, Leaf::Measured(true));
        assert_eq!(
            view.imported,
            Leaf::Measured(BTreeMap::from([
                ("aluminium".to_string(), 0.0),
                ("chromium".to_string(), 0.0),
                ("coal".to_string(), 0.0),
                ("oil".to_string(), 4.0),
                ("rubber".to_string(), 0.0),
                ("steel".to_string(), 2.0),
                ("tungsten".to_string(), 0.0),
            ]))
        );
        assert_eq!(view.factories_total, Leaf::Measured(25));
        assert_eq!(view.factories_used, Leaf::Measured(11));
        assert_eq!(view.factories_available, Leaf::Measured(14));
        assert_eq!(view.computed_imported_total(), Leaf::Measured(6.0));
        assert_eq!(view.computed_exported_total(), Leaf::Measured(5.0));
        assert_eq!(view.computed_convoys_assigned_total(), Leaf::Measured(3));
        assert_eq!(view.computed_convoys_required_total(), Leaf::Measured(5));
        assert_eq!(view.computed_requested_total(), Leaf::Measured(14.0));
        assert_eq!(view.computed_delivered_total(), Leaf::Measured(11.0));
        assert_eq!(view.computed_civilian_factory_total(), Leaf::Measured(3));
        assert_eq!(
            view.computed_partners(),
            Leaf::Measured(vec!["SWE".to_string(), "SOV".to_string()])
        );
        let Leaf::Measured(rows) = &view.resource_rows else {
            panic!("resource rows must be measured");
        };
        assert_eq!(rows[0].resource, Leaf::Measured("oil".to_string()));
        assert_eq!(rows[0].needed_current, Leaf::Measured(-12.0));
        assert_eq!(rows[0].needed_max, Leaf::Measured(-20.0));
        assert_eq!(rows[4].resource, Leaf::Measured("steel".to_string()));
        let Leaf::Measured(deals) = &view.resource_deals else {
            panic!("resource deals must be measured");
        };
        assert_eq!(deals[0].no_trade_influence, Leaf::Measured(false));
        assert_eq!(deals[0].no_resources, Leaf::Measured(false));
        assert_eq!(deals[1].no_resources, Leaf::Absent);
    }

    #[test]
    fn trade_view_fails_closed_on_unproven_order_or_incomplete_totals() {
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 8,
            "countries": {"GER": {"economy": {"resource_trade": {
                "resource_rows_complete": false,
                "resource_rows": [{
                    "resource": "oil", "visible_order": 1,
                    "balance": 0, "produced": 0, "imported": 0, "exported": 0,
                    "special_project": 0, "production_line": 0, "needed": 0,
                    "needed_type": "surplus"
                }],
                "deals_complete": false,
                "deals": []
            }}}}
        }));
        let view = frame.trade("GER").expect("GER trade");
        assert_eq!(view.resource_rows, Leaf::Unobtained);
        assert_eq!(view.computed_imported_total(), Leaf::Unobtained);
        assert_eq!(view.computed_convoys_assigned_total(), Leaf::Unobtained);
        assert!(frame.coverage_warnings().iter().any(|warning| {
            warning.contains("resource_rows[0].visible_order")
                && warning.contains("whole list treated as unobtained")
        }));
    }

    #[test]
    fn trade_view_rejects_complete_noncanonical_resource_sequence() {
        let rows: Vec<Value> = [
            "oil",
            "steel",
            "rubber",
            "tungsten",
            "aluminium",
            "chromium",
            "coal",
        ]
        .into_iter()
        .enumerate()
        .map(|(visible_order, resource)| {
            serde_json::json!({
                "resource": resource, "visible_order": visible_order,
                "balance": 0, "produced": 0, "imported": 0, "exported": 0,
                "special_project": 0, "production_line": 0, "needed": 0,
                "needed_current": if resource == "oil" { Some(0) } else { None },
                "needed_max": if resource == "oil" { Some(0) } else { None },
                "needed_type": if resource == "oil" { "current_max" } else { "surplus" }
            })
        })
        .collect();
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 9,
            "countries": {"GER": {"economy": {"resource_trade": {
                "resource_rows_complete": true,
                "resource_rows": rows,
                "deals_complete": true,
                "deals": []
            }}}}
        }));
        let view = frame.trade("GER").expect("GER trade");
        assert_eq!(view.resource_rows, Leaf::Unobtained);
        assert!(frame.coverage_warnings().iter().any(|warning| {
            warning.contains("resource_rows[1].resource")
                && warning.contains("expected aluminium, found steel")
        }));
    }

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

    fn xui17_exact_root() -> Value {
        serde_json::json!({
            "hour_index": 7,
            "countries": {
                "GER": {
                    "extra": {
                        "_r2_characters_leaders_cabinet_live": {
                            "characters": [
                                {"char_id": 10, "name": "Heinz Guderian", "token": "GER_heinz_guderian"},
                                {"char_id": 11, "name": "Karl Donitz", "token": "GER_karl_donitz"}
                            ],
                            "unit_leaders": [
                                {
                                    "unit_leader_id": 100, "char_id": 10,
                                    "leader_kind": "army", "leader_kind_raw": 0,
                                    "skill": 5, "experience_raw": 12345,
                                    "attack": 6, "defense": 4, "planning": 5, "logistics": 3,
                                    "maneuvering": 0, "coordination": 0,
                                    "token": "GER_heinz_guderian", "name": "Heinz Guderian"
                                },
                                {
                                    "unit_leader_id": 101, "char_id": 11,
                                    "leader_kind": "navy", "leader_kind_raw": 1,
                                    "skill": 4, "experience_raw": -20,
                                    "attack": 0, "defense": 0, "planning": 0, "logistics": 0,
                                    "maneuvering": 5, "coordination": 6,
                                    "token": "GER_karl_donitz", "name": "Karl Donitz"
                                }
                            ],
                            "unit_leader_traits": [
                                {"unit_leader_id": 100, "ordinal": 1, "trait_token": "panzer_leader"},
                                {"unit_leader_id": 100, "ordinal": 0, "trait_token": "armor_officer"}
                            ],
                            "country_leader_roles": [],
                            "active_rulers": [
                                {"char_id": 10, "ruling_party": "fascism", "name": "Heinz Guderian", "token": "GER_heinz_guderian", "leader_vec_size": 1, "ambiguous": false}
                            ]
                        },
                        "_r2_air_ace_roster_live": {
                            "aces": [
                                {"ace_id": 900, "country_ace_index": 0, "name": "Max", "surname": "Mustermann", "callsign": "Falke", "alive": true, "handled": false, "kill_type_raw": 0, "lifecycle": "alive", "wing_id": 700, "killer_ace_id": 0, "killer_country_tag_db_id": 0},
                                {"ace_id": 901, "country_ace_index": 1, "name": "Erika", "surname": "Musterfrau", "callsign": "Adler", "alive": false, "handled": true, "kill_type_raw": 1, "lifecycle": "dead_handled", "wing_id": 0, "killer_ace_id": 42, "killer_country_tag_db_id": 3}
                            ]
                        }
                    }
                }
            },
            "order_of_battle": {
                "army_groups": [
                    {"id": 500, "owner": "GER", "leader": 100}
                ],
                "armies": [],
                "fleets": [
                    {"id": 600, "owner": "GER", "leader": 101}
                ]
            }
        })
    }

    #[test]
    fn xui17_exact_sidecars_are_typed_and_joined_fail_closed() {
        let frame = world_frame_from_json(xui17_exact_root());
        assert_eq!(frame.character_rosters_exact_status(), ViewStatus::Present);
        let roster = frame
            .character_roster_exact("GER")
            .expect("GER exact roster");
        assert_eq!(roster.status, ViewStatus::Present);
        assert_eq!(roster.characters.len(), 2);
        assert_eq!(roster.unit_leaders[0].unit_leader_id, 100);
        assert_eq!(
            roster.unit_leaders[0].traits,
            vec!["armor_officer", "panzer_leader"]
        );
        assert_eq!(roster.unit_leaders[1].experience_raw, -20);
        assert!(roster.command_assignments_complete);
        assert_eq!(
            roster.unit_leaders[0].assignment,
            Some(UnitLeaderAssignmentExact {
                kind: UnitLeaderAssignmentKind::ArmyGroup,
                formation_id: 500,
            })
        );
        assert_eq!(
            roster.unit_leaders[1].assignment,
            Some(UnitLeaderAssignmentExact {
                kind: UnitLeaderAssignmentKind::Fleet,
                formation_id: 600,
            })
        );
        assert_eq!(roster.active_rulers[0].char_id, 10);

        assert_eq!(frame.ace_rosters_exact_status(), ViewStatus::Present);
        let aces = frame.ace_roster_exact("GER").expect("GER exact aces");
        assert!(aces.wing_assignment_complete);
        assert_eq!(aces.aces[1].country_ace_index, 1);
        assert_eq!(aces.aces[0].lifecycle, Some(AceLifecycleState::Alive));
        assert_eq!(aces.aces[1].handled, Some(true));
        assert_eq!(aces.aces[1].kill_type_raw, Some(1));
        assert_eq!(aces.aces[1].lifecycle, Some(AceLifecycleState::DeadHandled));
        assert_eq!(aces.aces[1].killer_ace_id, 42);
    }

    #[test]
    fn xui17_never_labels_missing_or_duplicate_oob_as_reserve() {
        let mut missing = xui17_exact_root();
        missing
            .as_object_mut()
            .expect("root")
            .remove("order_of_battle");
        let frame = world_frame_from_json(missing);
        let roster = frame.character_roster_exact("GER").expect("GER roster");
        assert!(!roster.command_assignments_complete);
        assert!(roster
            .unit_leaders
            .iter()
            .all(|row| row.assignment.is_none()));

        let mut duplicate = xui17_exact_root();
        duplicate["order_of_battle"]["armies"] = serde_json::json!([
            {"id": 501, "owner": "GER", "leader": 100}
        ]);
        let frame = world_frame_from_json(duplicate);
        let roster = frame.character_roster_exact("GER").expect("GER roster");
        assert!(!roster.command_assignments_complete);
        assert!(roster
            .unit_leaders
            .iter()
            .all(|row| row.assignment.is_none()));
        assert!(frame.coverage_warnings().iter().any(|warning| {
            warning.contains("duplicate d28 assignment for unit_leader_id 100")
        }));
    }

    #[test]
    fn xui17_rejects_orphan_traits_and_noncontiguous_ace_order() {
        let mut root = xui17_exact_root();
        root["countries"]["GER"]["extra"]["_r2_characters_leaders_cabinet_live"]
            ["unit_leader_traits"][0]["unit_leader_id"] = serde_json::json!(999);
        root["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]["aces"][1]
            ["country_ace_index"] = serde_json::json!(7);
        let frame = world_frame_from_json(root);
        assert_eq!(
            frame.character_rosters_exact_status(),
            ViewStatus::DomainUnavailable
        );
        assert_eq!(
            frame
                .character_roster_exact("GER")
                .expect("failed roster")
                .status,
            ViewStatus::DomainUnavailable
        );
        assert_eq!(
            frame.ace_rosters_exact_status(),
            ViewStatus::DomainUnavailable
        );
        assert!(frame
            .coverage_warnings()
            .iter()
            .any(|warning| warning.contains("orphan unit_leader_id 999")));
        assert!(frame
            .coverage_warnings()
            .iter()
            .any(|warning| warning.contains("expected 1, found 7")));
    }

    #[test]
    fn xui17_rejects_claimed_ace_lifecycle_that_disagrees_with_raw_tuple() {
        let mut root = xui17_exact_root();
        root["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]["aces"][1]["lifecycle"] =
            serde_json::json!("alive");
        let frame = world_frame_from_json(root);
        assert_eq!(
            frame.ace_rosters_exact_status(),
            ViewStatus::DomainUnavailable
        );
        assert!(frame
            .coverage_warnings()
            .iter()
            .any(|warning| warning.contains("does not match alive/handled tuple")));
    }

    #[test]
    fn xui17_domain46_problem_downgrades_retained_ace_sidecar() {
        let mut root = xui17_exact_root();
        root["provenance"] = serde_json::json!({
            "collector": {
                "domain_problems": [
                    {"domain_id": 46, "problem": "ace_roster_partial"}
                ]
            }
        });

        let frame = world_frame_from_json(root);
        assert_eq!(
            frame.ace_rosters_exact_status(),
            ViewStatus::DomainUnavailable
        );
        assert_eq!(
            frame
                .ace_roster_exact("GER")
                .expect("retained sidecar stays available for diagnostics")
                .status,
            ViewStatus::DomainUnavailable
        );
        assert_eq!(
            frame
                .ace_roster_exact("GER")
                .expect("retained diagnostic rows")
                .aces
                .len(),
            2,
            "raw retained rows remain diagnostic but are not Present"
        );
    }

    #[test]
    fn xui17_accepts_v1_ace_rows_without_fabricating_v2_lifecycle() {
        let mut root = xui17_exact_root();
        let rows = root["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]["aces"]
            .as_array_mut()
            .expect("ace rows");
        for row in rows {
            let row = row.as_object_mut().expect("ace row");
            row.remove("handled");
            row.remove("kill_type_raw");
            row.remove("lifecycle");
        }

        let frame = world_frame_from_json(root);
        assert_eq!(frame.ace_rosters_exact_status(), ViewStatus::Present);
        let aces = frame.ace_roster_exact("GER").expect("v1 ace rows");
        assert!(aces.aces.iter().all(|row| row.handled.is_none()
            && row.kill_type_raw.is_none()
            && row.lifecycle.is_none()));
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
    fn production_lines_expose_complete_future_contract_without_derivation() {
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 1,
            "countries": {
                "GER": {
                    "production": {
                        "stockpile": {"7316": 125.5},
                        "stockpile_total": 125.5,
                        "stockpile_complete": true,
                        "military_lines_complete": true,
                        "naval_lines_complete": true,
                        "military_lines": [{
                            "id": 41,
                            "priority": 0,
                            "line_subtype": "military",
                            "equipment_variant_id": 7316,
                            "active_factories": 7,
                            "requested_factories": 10,
                            "efficiency": 0.72,
                            "efficiency_cap": 0.85,
                            "efficiency_growth": 0.003,
                            "cost": 4.5,
                            "speed": 18.0,
                            "produced": 2.25,
                            "amount": -1,
                            "progress_ratio": 0.5,
                            "daily_output": 4.0,
                            "days_to_next_unit": 0.125,
                            "mio_id": "german_tank_organization"
                        }],
                        "naval_lines": []
                    },
                    "military": {"equipment_stockpile": []}
                }
            }
        }));
        let production = frame.production("GER").expect("GER production");
        assert_eq!(production.stockpile_complete, Leaf::Measured(true));
        assert_eq!(production.military_lines_complete, Leaf::Measured(true));
        assert_eq!(production.naval_lines_complete, Leaf::Measured(true));
        assert_eq!(production.naval_lines, Leaf::Measured(Vec::new()));
        let Leaf::Measured(lines) = &production.military_lines else {
            panic!("future production lines must be measured");
        };
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line.id, Leaf::Measured(41));
        assert_eq!(line.priority, Leaf::Measured(0));
        assert_eq!(line.line_subtype, Leaf::Measured("military".into()));
        assert_eq!(line.equipment_variant_id, Leaf::Measured("7316".into()));
        assert_eq!(line.active_factories, Leaf::Measured(7));
        assert_eq!(line.requested_factories, Leaf::Measured(10));
        assert_eq!(line.efficiency, Leaf::Measured(0.72));
        assert_eq!(line.efficiency_cap, Leaf::Measured(0.85));
        assert_eq!(line.efficiency_growth, Leaf::Measured(0.003));
        assert_eq!(line.cost, Leaf::Measured(4.5));
        assert_eq!(line.speed, Leaf::Measured(18.0));
        assert_eq!(line.produced, Leaf::Measured(2.25));
        assert_eq!(line.amount, Leaf::Measured(-1));
        assert_eq!(line.progress_ratio, Leaf::Measured(0.5));
        assert_eq!(line.daily_output, Leaf::Measured(4.0));
        assert_eq!(line.days_to_next_unit, Leaf::Measured(0.125));
        assert_eq!(
            line.mio_id,
            Leaf::Measured("german_tank_organization".into())
        );
        assert!(frame.coverage_warnings().is_empty());
    }

    #[test]
    fn b2_exact_convoy_bank_is_typed_without_stockpile_substitution() {
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 1,
            "countries": {"GER": {"military": {
                "convoys_complete": true,
                "convoys_owned_raw": 36900000,
                "convoys_count": 369.0,
                "convoys_assigned": 21,
                "convoys_available": 348,
                "convoys_cache_valid": true,
                "convoy_priority_reservations_complete": true,
                "convoys": [{
                    "archetype": "convoy_1",
                    "equip_id": 7316,
                    "equip_type": 70,
                    "amount_raw": 36900000,
                    "amount": 369.0
                }],
                "convoy_priority_reservations": [{"priority": 10, "count": 21}]
            }}}
        }));
        let production = frame.production("GER").expect("GER production view");
        assert_eq!(production.convoys_complete, Leaf::Measured(true));
        assert_eq!(production.convoys_owned_raw, Leaf::Measured(36_900_000));
        assert_eq!(production.convoys_owned, Leaf::Measured(369.0));
        assert_eq!(production.convoys_assigned, Leaf::Measured(21));
        assert_eq!(production.convoys_available, Leaf::Measured(348));
        let variants = production.convoy_variants.measured().expect("variants");
        assert_eq!(variants[0].archetype, Leaf::Measured("convoy_1".into()));
        assert_eq!(variants[0].equip_id, Leaf::Measured(7316));
        assert_eq!(variants[0].equip_type, Leaf::Measured(70));
        assert_eq!(variants[0].amount_raw, Leaf::Measured(36_900_000));
        assert_eq!(
            production
                .convoy_priority_reservations
                .measured()
                .expect("reservations")[0]
                .priority,
            Leaf::Measured(10)
        );
        assert_eq!(
            frame.topbar_scalars("GER").unwrap().convoys,
            Leaf::Measured(369.0)
        );
        assert!(frame.coverage_warnings().is_empty());
    }

    #[test]
    fn production_lines_keep_legacy_values_and_fail_closed_missing_seams() {
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 1,
            "countries": {
                "GER": {
                    "production": {
                        "military_lines": [{
                            "id": 41,
                            "active_factories": 7,
                            "cost": 4.5,
                            "speed": 18.0,
                            "produced": 2.25,
                            "amount": -1
                        }],
                        "naval_lines": null,
                        "stockpile": null,
                        "stockpile_total": null
                    }
                }
            }
        }));
        let production = frame.production("GER").expect("GER production");
        assert_eq!(production.stockpile, Leaf::Unobtained);
        assert_eq!(production.naval_lines, Leaf::Unobtained);
        assert_eq!(production.stockpile_complete, Leaf::Absent);
        assert_eq!(production.military_lines_complete, Leaf::Absent);
        let Leaf::Measured(lines) = &production.military_lines else {
            panic!("legacy production line must remain available");
        };
        let line = &lines[0];
        assert_eq!(line.id, Leaf::Measured(41));
        assert_eq!(line.active_factories, Leaf::Measured(7));
        assert_eq!(line.cost, Leaf::Measured(4.5));
        assert_eq!(line.speed, Leaf::Measured(18.0));
        assert_eq!(line.produced, Leaf::Measured(2.25));
        assert_eq!(line.amount, Leaf::Measured(-1));
        for seam in [&line.priority, &line.requested_factories] {
            assert_eq!(*seam, Leaf::Absent);
        }
        assert_eq!(line.line_subtype, Leaf::Absent);
        assert_eq!(line.equipment_variant_id, Leaf::Absent);
        assert_eq!(line.efficiency, Leaf::Absent);
        assert_eq!(line.efficiency_cap, Leaf::Absent);
        assert_eq!(line.efficiency_growth, Leaf::Absent);
        assert_eq!(line.progress_ratio, Leaf::Absent);
        assert_eq!(line.daily_output, Leaf::Absent);
        assert_eq!(line.days_to_next_unit, Leaf::Absent);
        assert_eq!(line.mio_id, Leaf::Absent);
    }

    #[test]
    fn production_line_wrong_types_are_unobtained_with_warnings() {
        let frame = world_frame_from_json(serde_json::json!({
            "hour_index": 1,
            "countries": {
                "GER": {
                    "production": {
                        "stockpile_complete": "yes",
                        "military_lines_complete": 1,
                        "military_lines": [{
                            "id": "wrong",
                            "equipment_variant_id": [],
                            "requested_factories": 3.5,
                            "efficiency": "wrong",
                            "progress_ratio": false,
                            "mio_id": {}
                        }],
                        "naval_lines": {}
                    }
                }
            }
        }));
        let production = frame.production("GER").expect("GER production");
        assert_eq!(production.stockpile_complete, Leaf::Unobtained);
        assert_eq!(production.military_lines_complete, Leaf::Unobtained);
        assert_eq!(production.naval_lines, Leaf::Unobtained);
        let Leaf::Measured(lines) = &production.military_lines else {
            panic!("well-shaped row list must remain measured");
        };
        let line = &lines[0];
        assert_eq!(line.id, Leaf::Unobtained);
        assert_eq!(line.equipment_variant_id, Leaf::Unobtained);
        assert_eq!(line.requested_factories, Leaf::Unobtained);
        assert_eq!(line.efficiency, Leaf::Unobtained);
        assert_eq!(line.progress_ratio, Leaf::Unobtained);
        assert_eq!(line.mio_id, Leaf::Unobtained);
        assert!(frame.coverage_warnings().len() >= 9);
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

    #[test]
    fn xui20_land_detail_parser_preserves_side_rows_and_event_order() {
        let rest = serde_json::json!({"combats":[{
            "combat_id":77,"kind":"land","location_province":12,"attackers":[1,2],"defenders":[3],
            "combat_width":null,"terrain":null,"progress":null,
            "land_combat_detail_events_live":{"phase_event":"updated","generation":5,
                "participant_events":[
                    {"event":"participant_joined","generation":5,"side":"attacker","division_id":2,"old_side_count":1,"new_side_count":2},
                    {"event":"participant_left_unclassified","generation":5,"side":"defender","division_id":4,"old_side_count":2,"new_side_count":1,"semantic":"left_unclassified"}],
                "side_detail_completeness":"both",
                "attacker":{"units":2,"committed":1,"reserve":1,"air_kills":3,"counts_incomplete":false,"progress":0.53,"progress_raw":53000,"tactic":{"id":7,"token":"tactic_basic_attack","name":"TACTIC_BASIC_ATTACK"},"tactic_status":"exact","losses_status":"unavailable_no_authenticated_live_carrier","reserve_division_ids":[102],"reserve_members_status":"exact"},
                "defender":{"units":1,"committed":1,"reserve":0,"air_kills":0,"counts_incomplete":false,"progress":-0.175,"progress_raw":-17500,"tactic":null,"tactic_status":"exact_none","losses_status":"unavailable_no_authenticated_live_carrier","reserve_division_ids":[],"reserve_members_status":"exact"}}
        }]}).as_object().unwrap().clone();
        let mut warnings = Vec::new();
        let rows = build_combats(&rest, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        let detail = rows[0].detail.measured().unwrap();
        assert_eq!(
            detail.attacker.measured().unwrap().reserve,
            Leaf::Measured(1)
        );
        let attacker = detail.attacker.measured().unwrap();
        assert_eq!(attacker.progress_raw, Leaf::Measured(53_000));
        assert_eq!(attacker.reserve_division_ids, Leaf::Measured(vec![102]));
        assert_eq!(
            attacker.reserve_members_status,
            Leaf::Measured("exact".into())
        );
        let tactic = attacker.tactic.measured().unwrap().as_ref().unwrap();
        assert_eq!(tactic.id, Leaf::Measured(7));
        assert_eq!(tactic.token, Leaf::Measured("tactic_basic_attack".into()));
        assert_eq!(
            detail.defender.measured().unwrap().tactic,
            Leaf::Measured(None)
        );
        let events = detail.participant_events.measured().unwrap();
        assert_eq!(events[0].division_id, Leaf::Measured(2));
        assert_eq!(
            events[1].semantic,
            Leaf::Measured("left_unclassified".into())
        );
        assert_eq!(rows[0].combat_width, Leaf::Unobtained);
    }

    #[test]
    fn xui20_ended_combats_are_separate_and_keyed_by_id_generation() {
        let rest = serde_json::json!({
            "combats":[{"combat_id":77,"kind":"land","location_province":12,
                "attackers":[1],"defenders":[2]}],
            "ended_combats":[
                {"combat_id":77,"generation":4,"kind":"land","lifecycle":"ended",
                 "location_province":12,"side_detail_availability":"unavailable",
                 "land_combat_detail_events_live":{"phase_event":"ended","generation":4}},
                {"combat_id":77,"generation":3,"kind":"land","lifecycle":"ended",
                 "location_province":11,"side_detail_availability":"present",
                 "land_combat_detail_events_live":{"phase_event":"ended","generation":3}}
            ]
        })
        .as_object()
        .unwrap()
        .clone();
        let mut warnings = Vec::new();
        let active = build_combats(&rest, &mut warnings);
        let (ended, status) = build_ended_combats(&rest, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(active.len(), 1);
        assert_eq!(status, ViewStatus::Present);
        assert_eq!(ended.len(), 2);
        assert_eq!(ended[&(77, 4)].lifecycle, Leaf::Measured("ended".into()));
        assert_eq!(ended[&(77, 3)].location_province, Leaf::Measured(11));
    }

    #[test]
    fn xui20_ended_combats_reject_nonterminal_and_duplicate_rows() {
        let rest = serde_json::json!({"ended_combats":[
            {"combat_id":5,"generation":2,"kind":"land","lifecycle":"active"},
            {"combat_id":5,"generation":3,"kind":"land","lifecycle":"ended"},
            {"combat_id":5,"generation":3,"kind":"land","lifecycle":"ended"}
        ]})
        .as_object()
        .unwrap()
        .clone();
        let mut warnings = Vec::new();
        let (ended, status) = build_ended_combats(&rest, &mut warnings);
        assert_eq!(status, ViewStatus::Present);
        assert_eq!(ended.keys().copied().collect::<Vec<_>>(), vec![(5, 3)]);
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn xui20_war_overview_preserves_ordered_sides_and_exact_metric_raws() {
        let rest = serde_json::json!({"diplomacy_world":{"wars":{"war:7":{
            "war_name":"THE_GREAT_WAR",
            "attackers":["GER","ITA"],
            "defenders":["ENG","FRA"],
            "front":"western",
            "start_hour":42,
            "overall_progress_raw":-12500,
            "attacker_participants":[{
                "tag":"GER","tag_db_id":1,"participation_raw":60000,
                "capitulation_progress_raw":10000,
                "participation_breakdown":{
                    "land_damage":11,"occupation":12,"air_damage":13,
                    "strategic_bombing":14,"sunk_ships":15,"sunk_convoys":16,
                    "casualties":17,"lend_lease_given":18,"lend_lease_received":19
                }
            },{
                "tag":"ITA","tag_db_id":2,"participation_raw":40000,
                "capitulation_progress_raw":20000
            }],
            "defender_participants":[{
                "tag":"ENG","tag_db_id":3,"participation_raw":100000,
                "capitulation_progress_raw":0
            }]
        }}}})
        .as_object()
        .unwrap()
        .clone();
        let mut warnings = Vec::new();
        let (Leaf::Measured(wars), status) = read_war_objects(&rest, &mut warnings) else {
            panic!("war overview must be measured");
        };
        assert_eq!(status, ViewStatus::Present);
        assert!(warnings.is_empty(), "{warnings:?}");
        let war = &wars[0];
        assert_eq!(war.key, "war:7");
        assert_eq!(
            war.attackers,
            Leaf::Measured(vec!["GER".into(), "ITA".into()])
        );
        assert_eq!(
            war.defenders,
            Leaf::Measured(vec!["ENG".into(), "FRA".into()])
        );
        assert_eq!(war.overall_progress_raw, Leaf::Measured(-12_500));
        let attackers = war.attacker_participants.measured().expect("attackers");
        assert_eq!(attackers[0].tag, Leaf::Measured("GER".into()));
        assert_eq!(attackers[1].tag, Leaf::Measured("ITA".into()));
        assert_eq!(attackers[0].participation_raw, Leaf::Measured(60_000));
        assert_eq!(
            attackers[0]
                .breakdown
                .measured()
                .expect("breakdown")
                .lend_lease_received,
            Leaf::Measured(19)
        );
        assert_eq!(attackers[1].breakdown, Leaf::Absent);
        assert_eq!(war.war_name, Leaf::Measured("THE_GREAT_WAR".into()));
    }

    #[test]
    fn xui20_war_overview_wrong_participant_shape_fails_whole_side_closed() {
        let rest = serde_json::json!({"diplomacy_world":{"wars":{"war:bad":{
            "attackers":["GER"],
            "defenders":["ENG"],
            "attacker_participants":["GER"]
        }}}})
        .as_object()
        .unwrap()
        .clone();
        let mut warnings = Vec::new();
        let (Leaf::Measured(wars), _) = read_war_objects(&rest, &mut warnings) else {
            panic!("war object container remains measured");
        };
        assert_eq!(wars[0].attacker_participants, Leaf::Unobtained);
        assert_eq!(wars[0].defender_participants, Leaf::Absent);
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("whole participant list treated as unobtained")));
    }

    #[test]
    fn xui20_production_naval_object_and_air_loss_arrays_are_typed() {
        let rest = serde_json::json!({
            "naval_combats":{"9:2":{"combat_id":9,"generation":2,"kind":"naval","region_id":77,"attacker_member_count":2,"defender_member_count":1,"attacker_task_force_ids":[10],"defender_task_force_ids":[20],"members":[{"side":"attacker","group_index":0,"member_index":0,"is_convoy":false,"ship_instance_id":100,"cship_join_validated":true,"owner_tag_db_id":3,"task_force_id":10,"fleet_id":11,"battle_slot_index":0,"state_bitmask":3,"strength":0.75,"organisation":0.5,"sunk_convoys":0},{"side":"attacker","group_index":0,"member_index":1,"is_convoy":false,"ship_instance_id":101,"cship_join_validated":true,"owner_tag_db_id":1,"task_force_id":10,"fleet_id":11,"battle_slot_index":1,"state_bitmask":3,"strength":0.5,"organisation":0.4,"sunk_convoys":0},{"side":"defender","group_index":0,"member_index":0,"is_convoy":true,"cship_join_validated":false,"owner_tag_db_id":2,"battle_slot_index":0,"state_bitmask":1,"strength":1.0,"organisation":1.0,"sunk_convoys":0}]}},
            "naval_air":{"_r2_air_combat_losses_live":[{"owner":"ENG","owner_tag_db_id":1,"wing_id":5,"generation":3,"mission_mask":4,"mission_region":77,"old_count":10,"new_count":7,"count_delta":-3,"old_equipment_amount_sum":10.0,"new_equipment_amount_sum":7.0,"equipment_amount_delta":-3.0,"flags":25}],"_r2_air_region_combat_stats":[{"owner":"ENG","owner_tag_db_id":1,"region_id":77,"side_vector":0,"selection_flag":1,"classification_flag":0,"copied_flag":0,"wing_id":5,"date_raw":1,"plane_count":7,"mission_mask":4,"attributed_country":"GER","attributed_country_tag_db_id":2,"damage":[{"channel":"losses_disruption","cause_type":4,"cause":"planes_shot_down_by_enemy_planes","amount":3.0}]}]},
            "world_events":{"sunk_convoys":[]}
        }).as_object().unwrap().clone();
        let mut warnings = Vec::new();
        let view = build_naval_air_combat(&rest, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(view.naval_combats[0].generation, Leaf::Measured(2));
        assert_eq!(
            view.naval_combats[0].attacker_country_tag_db_ids,
            Leaf::Measured(vec![1, 3])
        );
        assert_eq!(
            view.naval_combats[0].defender_country_tag_db_ids,
            Leaf::Measured(vec![2])
        );
        assert_eq!(
            view.naval_combats[0].members.measured().unwrap()[0].ship_instance_id,
            Leaf::Measured(100)
        );
        assert_eq!(
            view.air_losses.measured().unwrap()[0].count_delta,
            Leaf::Measured(-3)
        );
        assert_eq!(
            view.air_region_combat.measured().unwrap()[0]
                .damage
                .measured()
                .unwrap()[0]
                .cause_type,
            Leaf::Measured(4)
        );
    }

    #[test]
    fn xui20_naval_country_sets_fail_closed_per_side() {
        let rest = serde_json::json!({
            "naval_combats": [{
                "combat_id": 9,
                "generation": 2,
                "members": [
                    {"side":"attacker","owner_tag_db_id":1},
                    {"side":"attacker","owner_tag_db_id":null},
                    {"side":"defender","owner_tag_db_id":2}
                ]
            }]
        })
        .as_object()
        .unwrap()
        .clone();
        let mut warnings = Vec::new();
        let view = build_naval_air_combat(&rest, &mut warnings);
        assert_eq!(
            view.naval_combats[0].attacker_country_tag_db_ids,
            Leaf::Unobtained
        );
        assert_eq!(
            view.naval_combats[0].defender_country_tag_db_ids,
            Leaf::Measured(vec![2])
        );
    }

    #[test]
    fn a28_dynamic_collection_sidecar_is_typed_and_fail_closed() {
        let value = serde_json::json!({
            "_r2_dynamic_collection_trace_live": {
                "schema_version": 1,
                "authority": "exact_a729_controller",
                "elapsed_wall_ms": 7,
                "frame_hour": 900,
                "collections": [{
                    "collection_instance_id": "interface/test.gui#grid@GER",
                    "frame_hour": 900,
                    "source_items": [{"stable_item_id":"item:a","source_ordinal":0,"included":true,"filter_reason":null}],
                    "ordered_item_ids": ["item:a"],
                    "group_boundaries": [{"group_id":"all","start_index":0,"item_count":1}],
                    "row_template_id": "row",
                    "materialized_slots": [{"slot_id":"slot:0","bound_item_id":"item:a","row_template_id":"row","rect":{"x":1,"y":2,"width":30,"height":10}}],
                    "clip_rect": {"x":0,"y":0,"width":100,"height":100},
                    "content_extent_pixels": [100,10],
                    "viewport_extent_pixels": [100,100],
                    "scroll_min_fixed": 0,
                    "scroll_max_fixed": 100000,
                    "scroll_current_fixed": 0,
                    "scroll_target_fixed": 0
                }]
            }
        });
        let rest = value.as_object().unwrap().clone();
        let mut warnings = Vec::new();
        let view = build_dynamic_collection_trace(&rest, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(view.status, ViewStatus::Present);
        assert_eq!(view.frame_hour, Some(900));
        assert_eq!(view.collections[0].ordered_item_ids, vec!["item:a"]);

        let mut wrong = rest;
        wrong["_r2_dynamic_collection_trace_live"]["authority"] =
            Value::String("screen_inferred".into());
        let view = build_dynamic_collection_trace(&wrong, &mut warnings);
        assert_eq!(view.status, ViewStatus::DomainUnavailable);
        assert!(view.collections.is_empty());
    }

    #[test]
    fn a27_design_catalog_context_is_typed_ordered_and_fail_closed() {
        let value = serde_json::json!({
            "_r2_design_catalog_context_live": {
                "schema_version": 1,
                "authority": "exact_a729_designer_view",
                "elapsed_wall_ms": 7,
                "frame_hour": 900,
                "completeness_record_identity": "a27:a729:transition:4",
                "transition_sequence": 4,
                "active": true,
                "mode": 2,
                "family": "plane_designer_view",
                "our_filter_enabled": true,
                "foreign_filter_enabled": false,
                "obsolete_filter_enabled": true,
                "current_country_tag_id": 17,
                "selected_country_tag_ids": [17, 42],
                "ordered_design_ids": [100, 7, 55]
            }
        });
        let rest = value.as_object().unwrap().clone();
        let mut warnings = Vec::new();
        let view = build_design_catalog_context(&rest, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(view.status, ViewStatus::Present);
        assert_eq!(
            view.context.as_ref().unwrap().ordered_design_ids,
            [100, 7, 55]
        );

        let mut wrong = rest;
        wrong["_r2_design_catalog_context_live"]["family"] =
            Value::String("tank_designer_view".into());
        let view = build_design_catalog_context(&wrong, &mut warnings);
        assert_eq!(view.status, ViewStatus::DomainUnavailable);
        assert!(view.context.is_none());
    }

    #[test]
    fn d79_hud_surface_activation_is_typed_hour_bound_and_fail_closed() {
        let value = serde_json::json!({
            "_r2_hud_surface_activation_live": {
                "schema_version": 1,
                "authority": "exact_a729_hud_controller_roots",
                "frame_hour": 900,
                "status_flags": 0,
                "observation_sequence": 9,
                "active_surfaces": [
                    {"surface":"topbar","runtime_root_name":"topbar_root"},
                    {"surface":"minimap","runtime_root_name":"minimap_root"},
                    {"surface":"mapmodes","runtime_root_name":"mapmodes_root"},
                    {"surface":"theatre_selector","runtime_root_name":"theatre_root"}
                ]
            }
        });
        let rest = value.as_object().unwrap().clone();
        let mut warnings = Vec::new();
        let view = build_hud_surface_activation(&rest, 900, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(view.status, ViewStatus::Present);
        assert_eq!(view.activation.unwrap().active_surfaces.len(), 4);

        let view = build_hud_surface_activation(&rest, 901, &mut warnings);
        assert_eq!(view.status, ViewStatus::DomainUnavailable);
        assert!(view.activation.is_none());
    }
}
