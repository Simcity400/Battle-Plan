//! XUI20 combat-detail/losses population seam.
//!
//! This module consumes only typed `replay_core` views. It preserves producer
//! order within every authenticated stream and never invents a cross-domain
//! total order: land participant events, naval members, air loss deltas, air
//! regional damage rows, d70 convoy aggregates, d90 terminal convoy events,
//! and D60 war rows remain separate.

use replay_core::{
    AirCombatLossRow, AirRegionCombatRow, CombatParticipantEvent, CombatRow, ConvoyLossRow,
    ConvoyD70MutationKindExact, ConvoyRouteEndpointKindExact, ConvoyRouteProducerKindExact,
    ConvoyTransportAttackExactRow, ConvoyTransportAttackExactView, DiplomacyView, EndedCombatRow,
    LandCombatSideDetail, Leaf, NavalCombatRow, SunkShipEventExact, SunkShipHistoryCompleteness,
    SunkShipTextUnavailable,
    ViewStatus, WarObjectRow,
    WarRelationRow, WorldFrame,
};
use crate::waroverview_binding::WarResidualAdmission;
// `LandCombatTerminalRecord` is public through `replay_core::world` but is not
// in the crate-root re-export list, so use its canonical module path.
use replay_core::world::LandCombatTerminalRecord;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct LandCombatPanel {
    pub combat_id: Leaf<i64>,
    pub location_province: Leaf<u32>,
    pub attackers: Leaf<Vec<i64>>,
    pub defenders: Leaf<Vec<i64>>,
    pub combat_width: Leaf<f64>,
    pub terrain: Leaf<String>,
    pub progress: Leaf<f64>,
    pub phase_event: Leaf<String>,
    pub detail_generation: Leaf<i64>,
    pub detail_location_province: Leaf<u32>,
    pub age_days: Leaf<u32>,
    pub duration_days: Leaf<u32>,
    pub terminal_record: Leaf<LandCombatTerminalRecord>,
    pub terminal_record_status: Leaf<String>,
    pub side_detail_completeness: Leaf<String>,
    pub participant_events: Leaf<Vec<CombatParticipantEvent>>,
    pub attacker: Leaf<LandCombatSideDetail>,
    pub defender: Leaf<LandCombatSideDetail>,
    pub attacker_leader: Leaf<Option<i64>>,
    pub defender_leader: Leaf<Option<i64>>,
    pub attacker_displayed_modifiers: Leaf<Vec<String>>,
    pub defender_displayed_modifiers: Leaf<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xui20CombatDetailLosses {
    pub land: Vec<LandCombatPanel>,
    /// Availability of the active land-combat collection itself
    /// ([`replay_core::WorldFrame::combats_status`]) — the same status
    /// `combat_binding` consumes for the combat panel. `Present` with an empty
    /// `land` is the measured "no land combats were running"; any other status
    /// means the collection was not readable this hour and the land surface
    /// must not be treated as measured, however many rows are retained.
    pub land_status: ViewStatus,
    /// Terminal lifecycle rows are a separate keyed collection and are never
    /// passed through the active-combat panel renderer.
    pub ended_land: BTreeMap<(i64, i64), EndedCombatRow>,
    pub ended_land_status: ViewStatus,
    pub naval: Vec<NavalCombatRow>,
    /// Availability of the `naval_combats` collection itself
    /// ([`replay_core::NavalAirCombatView::naval_combats_status`]). `Present`
    /// with an empty `naval` is the measured "no naval combats"; any other
    /// status means the collection was not readable this hour and the naval
    /// surface must not be treated as measured.
    pub naval_status: ViewStatus,
    pub air_losses: Leaf<Vec<AirCombatLossRow>>,
    pub air_region_combat: Leaf<Vec<AirRegionCombatRow>>,
    pub sunk_convoys: Leaf<Vec<ConvoyLossRow>>,
    /// Exact, producer-ordered terminal convoy attacks from d90. This is not
    /// the monthly d70 aggregate and is never populated from that aggregate.
    pub convoy_terminal: Leaf<Vec<ConvoyTransportAttackExactRow>>,
    pub sunk_ships: Leaf<Vec<SunkShipEventExact>>,
    /// Whether `sunk_ships` is the WHOLE record of sinkings for this hour, and
    /// when it is not, exactly what D86's producer lost. A partial history is
    /// admitted and its rows are carried (owner ruling 2026-08-06), so the
    /// surface that renders them has to be able to say so; `Leaf::Measured` on
    /// `sunk_ships` alone cannot, because a partial and a complete delivery are
    /// both measured.
    pub sunk_ships_completeness: SunkShipHistoryCompleteness,
    pub wars: Leaf<Vec<WarObjectRow>>,
    pub war_relations: Leaf<Vec<WarRelationRow>>,
}

impl Xui20CombatDetailLosses {
    /// True only when every stream required to render at least one XUI20
    /// surface is positively measured. Missing optional rows remain empty but
    /// unavailable producer leaves remain distinguishable.
    ///
    /// Every status-owning collection is read through its status first: rows
    /// retained under a non-`Present` collection are not a measured surface,
    /// so a non-empty vector alone can never make this true.
    pub fn has_any_measured_surface(&self) -> bool {
        (self.land_status == ViewStatus::Present && !self.land.is_empty())
            || (self.ended_land_status == ViewStatus::Present && !self.ended_land.is_empty())
            || (self.naval_status == ViewStatus::Present && !self.naval.is_empty())
            || self.air_losses.measured().is_some()
            || self.air_region_combat.measured().is_some()
            || self.sunk_convoys.measured().is_some()
            || self.convoy_terminal.measured().is_some()
            || self.sunk_ships.measured().is_some()
            || self.wars.measured().is_some()
            || self.war_relations.measured().is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xui20SurfaceState {
    Admitted { row_count: usize },
    ExternalInputRequired { reason: String },
    Rejected { reason: String },
    NotApplicable { reason: String },
}

/// The replay frame hour D86's calendar admission is evaluated against.
///
/// The gate cannot invent this value. `sink_date_raw` is only admissible
/// relative to the hour the frame claims to be, so a binding presented without
/// a usable hour is refused rather than admitted on the hour-independent half
/// alone -- which is what this surface did before, and is how `game_ui` and
/// `replay_core` could reach opposite verdicts on the same D86 row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xui20FrameHour {
    /// The frame's `hour_index`, non-negative by construction here.
    Measured(u64),
    /// The frame carries no hour this gate can evaluate against. The reason
    /// travels with it so the refusal stays distinguishable from a rejected
    /// row.
    Unavailable { reason: String },
}

impl Xui20FrameHour {
    /// The ONLY way a caller obtains a frame hour for this gate: it comes from
    /// authoritative replay state (`WorldFrame::hour_index`) or it does not
    /// exist. There is no default and no zero.
    ///
    /// `hour_index` is read from the recording's index as a signed value and is
    /// never clamped on the way in, so a negative hour is reachable from a
    /// corrupt or crafted file. `replay_core` refuses to bind D86 to one
    /// (`build_sunk_ship_history_exact`); this refuses for the same reason.
    pub fn from_frame(frame: &WorldFrame) -> Self {
        match u64::try_from(frame.hour_index()) {
            Ok(hour) => Self::Measured(hour),
            Err(_) => Self::Unavailable {
                reason: format!(
                    "replay frame hour_index {} is negative; D86's calendar admission \
                     has no hour to bind to",
                    frame.hour_index()
                ),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xui20ReplayAdmission {
    pub land_active: Xui20SurfaceState,
    pub land_side_detail: Xui20SurfaceState,
    pub land_ended_lifecycle: Xui20SurfaceState,
    pub land_terminal: Xui20SurfaceState,
    pub naval_active: Xui20SurfaceState,
    pub naval_terminal: Xui20SurfaceState,
    pub air_history: Xui20SurfaceState,
    pub air_region_history: Xui20SurfaceState,
    pub convoy_history: Xui20SurfaceState,
    pub convoy_terminal: Xui20SurfaceState,
    pub combat_log_families: Xui20SurfaceState,
    pub global_cross_family_order: Xui20SurfaceState,
    pub war_pair_history: Xui20SurfaceState,
    pub war_overview_exact: Xui20SurfaceState,
    /// EIR-NE-011. The exact-a729 H-B5 war-threat/instigator/hostility tuple
    /// carried on the same D60 war rows this packet already admits, gated
    /// independently of [`Self::war_overview_exact`] so a pre-v2 recording
    /// keeps its war overview and loses only the residual surface.
    pub war_threat_residuals_exact: Xui20SurfaceState,
}

enum GateFailure {
    External(String),
    Invalid(String),
}

fn external(field: impl Into<String>) -> GateFailure {
    GateFailure::External(field.into())
}

fn invalid(reason: impl Into<String>) -> GateFailure {
    GateFailure::Invalid(reason.into())
}

fn required<'a, T>(leaf: &'a Leaf<T>, field: &str) -> Result<&'a T, GateFailure> {
    leaf.measured().ok_or_else(|| external(field))
}

fn require_unobtained<T>(leaf: &Leaf<T>, field: &str) -> Result<(), GateFailure> {
    match leaf {
        Leaf::Unobtained => Ok(()),
        Leaf::Absent => Err(external(field)),
        Leaf::Measured(_) => Err(invalid(format!(
            "{field} must be explicitly unobtained for this member kind"
        ))),
    }
}

fn exact_fex_state_name(raw: i64) -> Option<&'static str> {
    match raw {
        0 => Some("FEX_STATE_UNINITIALIZED"),
        1 => Some("FEX_STATE_WAITING"),
        2 => Some("FEX_STATE_FIGHTING"),
        4 => Some("FEX_STATE_ON_THE_WAY"),
        8 => Some("FEX_STATE_ESCAPING"),
        0x10 => Some("FEX_STATE_RUN_AWAY"),
        0x20 => Some("FEX_STATE_SUNK"),
        _ => None,
    }
}

fn state(count: usize, result: Result<(), GateFailure>) -> Xui20SurfaceState {
    match result {
        Ok(()) => Xui20SurfaceState::Admitted { row_count: count },
        Err(GateFailure::External(reason)) => Xui20SurfaceState::ExternalInputRequired { reason },
        Err(GateFailure::Invalid(reason)) => Xui20SurfaceState::Rejected { reason },
    }
}

fn family_state(count: usize, states: &[&Xui20SurfaceState]) -> Xui20SurfaceState {
    if let Some(state) = states
        .iter()
        .copied()
        .find(|state| matches!(state, Xui20SurfaceState::Rejected { .. }))
    {
        return state.clone();
    }
    if let Some(state) = states
        .iter()
        .copied()
        .find(|state| matches!(state, Xui20SurfaceState::ExternalInputRequired { .. }))
    {
        return state.clone();
    }
    Xui20SurfaceState::Admitted { row_count: count }
}

/// The active-land surface is gated on the collection's own status first
/// (mirroring [`validate_ended_land_surface`] and [`validate_naval_surface`]):
/// a non-`Present` collection is an external-input condition regardless of any
/// retained rows, never an admitted (or measured-empty) surface.
fn validate_land_active_surface(
    status: ViewStatus,
    rows: &[LandCombatPanel],
) -> Result<(), GateFailure> {
    match status {
        ViewStatus::Present => validate_land_active(rows),
        ViewStatus::DomainAbsent => Err(external("combats field absent")),
        ViewStatus::DomainUnavailable => Err(external("combats unavailable")),
    }
}

/// Side detail is a projection of the same active-land collection, so it is
/// gated on the same status: an unreadable collection cannot yield a measured
/// side-detail surface.
fn validate_land_side_detail_surface(
    status: ViewStatus,
    rows: &[LandCombatPanel],
) -> Result<(), GateFailure> {
    match status {
        ViewStatus::Present => validate_land_side_detail(rows),
        ViewStatus::DomainAbsent => Err(external("combats field absent")),
        ViewStatus::DomainUnavailable => Err(external("combats unavailable")),
    }
}

fn validate_land_active(rows: &[LandCombatPanel]) -> Result<(), GateFailure> {
    for (index, row) in rows.iter().enumerate() {
        let prefix = format!("land[{index}]");
        if *required(&row.combat_id, &format!("{prefix}.combat_id"))? < 0 {
            return Err(invalid(format!("{prefix}.combat_id is negative")));
        }
        if *required(
            &row.location_province,
            &format!("{prefix}.location_province"),
        )? == 0
        {
            return Err(invalid(format!("{prefix}.location_province is zero")));
        }
        let attackers = required(&row.attackers, &format!("{prefix}.attackers"))?;
        let defenders = required(&row.defenders, &format!("{prefix}.defenders"))?;
        if attackers.is_empty() || defenders.is_empty() {
            return Err(invalid(format!("{prefix} has an empty active side")));
        }
        let terrain = required(&row.terrain, &format!("{prefix}.terrain"))?;
        if terrain.trim().is_empty() {
            return Err(invalid(format!("{prefix}.terrain is empty")));
        }
        let width = *required(&row.combat_width, &format!("{prefix}.combat_width"))?;
        if !width.is_finite() || width <= 0.0 {
            return Err(invalid(format!("{prefix}.combat_width is invalid")));
        }
        let progress = *required(&row.progress, &format!("{prefix}.progress"))?;
        if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
            return Err(invalid(format!("{prefix}.progress is outside 0..1")));
        }
    }
    Ok(())
}

fn validate_ended_land(rows: &BTreeMap<(i64, i64), EndedCombatRow>) -> Result<(), GateFailure> {
    for ((combat_id, generation), row) in rows {
        let prefix = format!("ended_land[{combat_id},{generation}]");
        if *combat_id != row.combat_id || *generation != row.generation {
            return Err(invalid(format!("{prefix} key/row identity mismatch")));
        }
        if *combat_id <= 0 || *generation <= 0 {
            return Err(invalid(format!("{prefix} identity is non-positive")));
        }
        if required(&row.lifecycle, &format!("{prefix}.lifecycle"))? != "ended" {
            return Err(invalid(format!("{prefix}.lifecycle is not ended")));
        }
        if required(&row.kind, &format!("{prefix}.kind"))? != "land" {
            return Err(invalid(format!("{prefix}.kind is not land")));
        }
        if let Leaf::Measured(detail) = &row.detail {
            if required(&detail.phase_event, &format!("{prefix}.phase_event"))? != "ended" {
                return Err(invalid(format!("{prefix}.phase_event is not ended")));
            }
            if detail.generation.measured().copied() != Some(*generation) {
                return Err(invalid(format!("{prefix}.detail generation mismatch")));
            }
        }
    }
    Ok(())
}

fn validate_ended_land_surface(
    status: ViewStatus,
    rows: &BTreeMap<(i64, i64), EndedCombatRow>,
) -> Result<(), GateFailure> {
    match status {
        ViewStatus::Present => validate_ended_land(rows),
        ViewStatus::DomainAbsent => Err(external("ended_combats field absent")),
        ViewStatus::DomainUnavailable => Err(external("ended_combats unavailable")),
    }
}

fn validate_land_terminal_record(
    record: &LandCombatTerminalRecord,
    owning_location_province: u32,
    prefix: &str,
) -> Result<(), GateFailure> {
    let record_index = *required(&record.record_index, &format!("{prefix}.record_index"))?;
    if record_index < 0 {
        return Err(invalid(format!("{prefix}.record_index is negative")));
    }

    let writer = required(&record.writer, &format!("{prefix}.writer"))?;
    if writer != "normal" && writer != "overrun" {
        return Err(invalid(format!(
            "{prefix}.writer is unknown: {writer:?}"
        )));
    }

    required(
        &record.attacker_manpower_loss_raw,
        &format!("{prefix}.attacker_manpower_loss_raw"),
    )?;
    required(
        &record.defender_manpower_loss_raw,
        &format!("{prefix}.defender_manpower_loss_raw"),
    )?;

    let end_date_raw = *required(&record.end_date_raw, &format!("{prefix}.end_date_raw"))?;
    if end_date_raw < 0 {
        return Err(invalid(format!("{prefix}.end_date_raw is negative")));
    }

    let terminal_location = *required(
        &record.location_province,
        &format!("{prefix}.location_province"),
    )?;
    if terminal_location == 0 {
        return Err(invalid(format!(
            "{prefix}.location_province is zero on terminal side"
        )));
    }
    if owning_location_province == 0 {
        return Err(invalid(format!(
            "{prefix}.owning_location_province is zero on owning row side"
        )));
    }
    if terminal_location != owning_location_province {
        return Err(invalid(format!(
            "{prefix}.location_province owning-vector join mismatch: terminal {terminal_location}, owning row {owning_location_province}"
        )));
    }

    required(
        &record.winner_side_raw,
        &format!("{prefix}.winner_side_raw"),
    )?;
    required(&record.winter, &format!("{prefix}.winter"))?;
    let result_selector_raw = *required(
        &record.result_selector_raw,
        &format!("{prefix}.result_selector_raw"),
    )?;
    required(
        &record.layout_side_raw,
        &format!("{prefix}.layout_side_raw"),
    )?;

    let source = required(&record.source, &format!("{prefix}.source"))?;
    if source != "authenticated_a729_terminal_writer" {
        return Err(invalid(format!(
            "{prefix}.source is not authenticated_a729_terminal_writer: {source:?}"
        )));
    }

    if !matches!(
        (writer.as_str(), result_selector_raw),
        ("overrun", 0) | ("normal", 1)
    ) {
        return Err(invalid(format!(
            "{prefix}.writer/result_selector_raw pair is invalid: ({writer:?}, {result_selector_raw})"
        )));
    }
    Ok(())
}

fn validate_land_terminal_slot(
    status: &Leaf<String>,
    record: &Leaf<LandCombatTerminalRecord>,
    owning_location: &Leaf<u32>,
    prefix: &str,
) -> Result<(), GateFailure> {
    let status = required(status, &format!("{prefix}.terminal_record_status"))?;
    match status.as_str() {
        "exact" => {
            let record = required(record, &format!("{prefix}.terminal_record"))?;
            let owning_location =
                *required(owning_location, &format!("{prefix}.owning_location_province"))?;
            validate_land_terminal_record(record, owning_location, &format!("{prefix}.terminal_record"))
        }
        "unavailable" => {
            if record.measured().is_some() {
                Err(invalid(format!(
                    "{prefix}.terminal_record is measured under unavailable terminal_record_status"
                )))
            } else {
                Err(external(format!(
                    "{prefix} terminal record is unavailable"
                )))
            }
        }
        unknown => Err(invalid(format!(
            "{prefix}.terminal_record_status is unknown: {unknown:?}"
        ))),
    }
}

/// Validate and count the exact terminal records carried by the two producer
/// collections.
///
/// `extractor/native/recorder_host/src/world_fold.rs:6689`-`:6691`:
///
/// > Double-emission across the two fields is impossible: an orphan is by
/// > definition a key that did NOT match any active combat's generation, so
/// > a key emitted inside an active record above is never re-emitted here.
///
/// Therefore a shared identity is recording corruption, never an overlap to
/// deduplicate.
fn validate_land_terminal(
    active_rows: &[LandCombatPanel],
    ended_rows: &BTreeMap<(i64, i64), EndedCombatRow>,
) -> Result<usize, GateFailure> {
    let mut validated_records = 0;
    for ((combat_id, generation), row) in ended_rows {
        if *combat_id != row.combat_id || *generation != row.generation {
            return Err(invalid(format!(
                "ended_land key/row identity mismatch: key ({combat_id}, {generation}), row ({}, {})",
                row.combat_id, row.generation
            )));
        }
        let prefix = format!("ended_land[{},{}]", row.combat_id, row.generation);
        match &row.detail {
            Leaf::Measured(detail) => validate_land_terminal_slot(
                &detail.terminal_record_status,
                &detail.terminal_record,
                &row.location_province,
                &prefix,
            )?,
            Leaf::Unobtained | Leaf::Absent => {
                return Err(external(format!("{prefix}.terminal_record_status")));
            }
        }
        validated_records += 1;
    }

    for (index, row) in active_rows.iter().enumerate() {
        if let (Some(combat_id), Some(generation)) =
            (row.combat_id.measured(), row.detail_generation.measured())
        {
            if ended_rows.values().any(|ended| {
                ended.combat_id == *combat_id && ended.generation == *generation
            }) {
                return Err(invalid(format!(
                    "impossible double-emission for identity ({combat_id}, {generation}) in combats[] and ended_combats[]"
                )));
            }
        }

        // `world_fold.rs:6798`-`:6803` emits this status for a record or ended phase,
        // but `replay_core/src/world.rs:4496`-`:4504` can parse it as non-`Measured`.
        let carries_terminal_data = row.terminal_record.measured().is_some()
            || row.terminal_record_status.measured().is_some();
        if !carries_terminal_data {
            if matches!(&row.phase_event, Leaf::Measured(event) if event == "ended") {
                return Err(external(format!(
                    "land[{index}].terminal_record_status"
                )));
            }
            continue;
        }
        let combat_id = required(&row.combat_id, &format!("land[{index}].combat_id"))?;
        let generation = required(
            &row.detail_generation,
            &format!("land[{index}].detail_generation"),
        )?;
        validate_land_terminal_slot(
            &row.terminal_record_status,
            &row.terminal_record,
            &row.location_province,
            &format!("land[{combat_id},{generation}]"),
        )?;
        validated_records += 1;
    }
    Ok(validated_records)
}

/// The terminal-land surface reads both combat collections, so neither source
/// can be treated as measured unless its own collection status is `Present`.
fn validate_land_terminal_surface(
    ended_status: ViewStatus,
    land_status: ViewStatus,
    active_rows: &[LandCombatPanel],
    ended_rows: &BTreeMap<(i64, i64), EndedCombatRow>,
) -> Result<usize, GateFailure> {
    match ended_status {
        ViewStatus::Present => Ok(()),
        ViewStatus::DomainAbsent => Err(external("ended_combats field absent")),
        ViewStatus::DomainUnavailable => Err(external("ended_combats unavailable")),
    }?;
    match land_status {
        ViewStatus::Present => Ok(()),
        ViewStatus::DomainAbsent => Err(external("combats field absent")),
        ViewStatus::DomainUnavailable => Err(external("combats unavailable")),
    }?;
    validate_land_terminal(active_rows, ended_rows)
}

fn validate_land_side(side: &LandCombatSideDetail, prefix: &str) -> Result<(), GateFailure> {
    for (leaf, field) in [
        (&side.units, "units"),
        (&side.committed, "committed"),
        (&side.reserve, "reserve"),
        (&side.air_kills, "air_kills"),
        (&side.org_loss_summary_index, "org_loss_summary_index"),
        (&side.num_org_losses, "num_org_losses"),
        (&side.progress_raw, "progress_raw"),
    ] {
        required(leaf, &format!("{prefix}.{field}"))?;
    }
    for (leaf, field) in [
        (&side.anti_air_attack, "anti_air_attack"),
        (&side.air_damage_str, "air_damage_str"),
        (&side.air_damage_org, "air_damage_org"),
        (&side.ground_damage_str, "ground_damage_str"),
        (&side.ground_damage_org, "ground_damage_org"),
        (&side.prevented_damage_str, "prevented_damage_str"),
        (&side.prevented_damage_org, "prevented_damage_org"),
        (&side.progress, "progress"),
    ] {
        let value = *required(leaf, &format!("{prefix}.{field}"))?;
        if !value.is_finite() {
            return Err(invalid(format!("{prefix}.{field} is invalid")));
        }
    }
    required(&side.tactic_status, &format!("{prefix}.tactic_status"))?;
    let losses_status = required(&side.losses_status, &format!("{prefix}.losses_status"))?;
    match losses_status.as_str() {
        "exact_current_epoch_writer" => {
            let observations = required(
                &side.current_loss_observations,
                &format!("{prefix}.current_loss_observations"),
            )?;
            if observations.is_empty() {
                return Err(invalid(format!(
                    "{prefix}.current_loss_observations is empty for exact status"
                )));
            }
            let mut sequences = std::collections::BTreeSet::new();
            for (observation_index, observation) in observations.iter().enumerate() {
                let observation_prefix =
                    format!("{prefix}.current_loss_observations[{observation_index}]");
                let sequence = *required(
                    &observation.capture_sequence,
                    &format!("{observation_prefix}.capture_sequence"),
                )?;
                if sequence == 0 || !sequences.insert(sequence) {
                    return Err(invalid(format!(
                        "{observation_prefix}.capture_sequence is zero or duplicate"
                    )));
                }
                let manpower_before = *required(
                    &observation.manpower_before_raw,
                    &format!("{observation_prefix}.manpower_before_raw"),
                )?;
                let manpower_after = *required(
                    &observation.manpower_after_raw,
                    &format!("{observation_prefix}.manpower_after_raw"),
                )?;
                if manpower_before == manpower_after {
                    return Err(invalid(format!(
                        "{observation_prefix} has no manpower delta"
                    )));
                }
                let source = required(
                    &observation.source,
                    &format!("{observation_prefix}.source"),
                )?;
                if !matches!(
                    source.as_str(),
                    "authenticated_a729_active_division_damage_140c7bfc0"
                        | "authenticated_a729_participant_lifecycle_141299850"
                ) {
                    return Err(invalid(format!(
                        "{observation_prefix}.source is not an authenticated X2 producer"
                    )));
                }
                let equipment = required(
                    &observation.equipment,
                    &format!("{observation_prefix}.equipment"),
                )?;
                if equipment.is_empty() {
                    return Err(invalid(format!(
                        "{observation_prefix}.equipment is empty"
                    )));
                }
                let mut equipment_ids = std::collections::BTreeSet::new();
                for (ordinal, delta) in equipment.iter().enumerate() {
                    let delta_prefix = format!("{observation_prefix}.equipment[{ordinal}]");
                    let actual_ordinal = *required(
                        &delta.ordinal,
                        &format!("{delta_prefix}.ordinal"),
                    )?;
                    let equipment_id = *required(
                        &delta.equipment_id,
                        &format!("{delta_prefix}.equipment_id"),
                    )?;
                    let before = *required(
                        &delta.before_raw,
                        &format!("{delta_prefix}.before_raw"),
                    )?;
                    let after = *required(
                        &delta.after_raw,
                        &format!("{delta_prefix}.after_raw"),
                    )?;
                    if actual_ordinal != ordinal as i64
                        || equipment_id <= 0
                        || !equipment_ids.insert(equipment_id)
                        || before == after
                    {
                        return Err(invalid(format!(
                            "{delta_prefix} has invalid order, identity, or delta"
                        )));
                    }
                }
            }
        }
        "exact_no_writer_call_this_hour" => {
            if side.current_loss_observations.measured().is_some() {
                return Err(invalid(format!(
                    "{prefix}.current_loss_observations conflicts with exact-empty status"
                )));
            }
        }
        "unavailable_no_authenticated_live_carrier" => {
            if side.current_loss_observations.measured().is_some() {
                return Err(invalid(format!(
                    "{prefix}.current_loss_observations conflicts with unavailable status"
                )));
            }
        }
        _ => return Err(invalid(format!("{prefix}.losses_status is unknown"))),
    }
    let width_status = required(&side.width_status, &format!("{prefix}.width_status"))?;
    match width_status.as_str() {
        "exact_synchronous_call" => {
            let observations = required(
                &side.width_observations,
                &format!("{prefix}.width_observations"),
            )?;
            if observations.is_empty() {
                return Err(invalid(format!(
                    "{prefix}.width_observations is empty for exact status"
                )));
            }
            let committed = *required(&side.committed, &format!("{prefix}.committed"))?;
            let counts_incomplete =
                *required(&side.counts_incomplete, &format!("{prefix}.counts_incomplete"))?;
            let mut sequences = BTreeSet::new();
            for (observation_index, observation) in observations.iter().enumerate() {
                let observation_prefix =
                    format!("{prefix}.width_observations[{observation_index}]");
                let sequence = *required(
                    &observation.capture_sequence,
                    &format!("{observation_prefix}.capture_sequence"),
                )?;
                if sequence == 0 || !sequences.insert(sequence) {
                    return Err(invalid(format!(
                        "{observation_prefix}.capture_sequence is zero or duplicate"
                    )));
                }
                if required(&observation.source, &format!("{observation_prefix}.source"))?
                    != "authenticated_a729_141299610_call"
                {
                    return Err(invalid(format!(
                        "{observation_prefix}.source is not the authenticated X2 width producer"
                    )));
                }
                let final_width_raw = *required(
                    &observation.final_width_raw,
                    &format!("{observation_prefix}.final_width_raw"),
                )?;
                let wrappers = required(
                    &observation.wrappers,
                    &format!("{observation_prefix}.wrappers"),
                )?;
                if wrappers.is_empty() || (!counts_incomplete && committed != wrappers.len() as i64)
                {
                    return Err(invalid(format!(
                        "{observation_prefix}.wrappers is empty or disagrees with committed count"
                    )));
                }
                let mut division_ids = BTreeSet::new();
                let mut sum = 0i64;
                for (ordinal, wrapper) in wrappers.iter().enumerate() {
                    let wrapper_prefix = format!("{observation_prefix}.wrappers[{ordinal}]");
                    let actual_ordinal =
                        *required(&wrapper.ordinal, &format!("{wrapper_prefix}.ordinal"))?;
                    let division_id = *required(
                        &wrapper.division_id,
                        &format!("{wrapper_prefix}.division_id"),
                    )?;
                    let width_raw =
                        *required(&wrapper.width_raw, &format!("{wrapper_prefix}.width_raw"))?;
                    if actual_ordinal != ordinal as i64
                        || division_id <= 0
                        || !division_ids.insert(division_id)
                    {
                        return Err(invalid(format!(
                            "{wrapper_prefix} has invalid order or division identity"
                        )));
                    }
                    sum = sum.checked_add(width_raw).ok_or_else(|| {
                        invalid(format!("{observation_prefix}.wrapper width sum overflows"))
                    })?;
                }
                if sum != final_width_raw {
                    return Err(invalid(format!(
                        "{observation_prefix}.wrapper sum does not equal final width"
                    )));
                }
            }
        }
        "exact_no_synchronous_call_this_hour" => {
            if side.width_observations.measured().is_some() {
                return Err(invalid(format!(
                    "{prefix}.width_observations conflicts with exact-empty status"
                )));
            }
        }
        "unavailable_pending_x2_control" => {
            if side.width_observations.measured().is_some() {
                return Err(invalid(format!(
                    "{prefix}.width_observations conflicts with unavailable status"
                )));
            }
        }
        _ => return Err(invalid(format!("{prefix}.width_status is unknown"))),
    }
    let leader_status = required(&side.leader_status, &format!("{prefix}.leader_status"))?;
    match leader_status.as_str() {
        "exact" => match required(&side.leader_id, &format!("{prefix}.leader_id"))? {
            Some(id) if *id > 0 => {}
            _ => return Err(invalid(format!("{prefix}.leader_id is not an exact positive id"))),
        },
        "exact_none" => {
            if required(&side.leader_id, &format!("{prefix}.leader_id"))?.is_some() {
                return Err(invalid(format!("{prefix}.leader_id conflicts with exact_none")));
            }
        }
        "unavailable_panel_not_observed" => {
            if side.leader_id.measured().is_some() {
                return Err(invalid(format!(
                    "{prefix}.leader_id conflicts with unavailable panel status"
                )));
            }
        }
        _ => return Err(invalid(format!("{prefix}.leader_status is unknown"))),
    }
    let modifier_status = required(
        &side.effective_displayed_modifiers_status,
        &format!("{prefix}.effective_displayed_modifiers_status"),
    )?;
    match modifier_status.as_str() {
        "exact" => {
            if !matches!(leader_status.as_str(), "exact" | "exact_none") {
                return Err(invalid(format!(
                    "{prefix} panel leader/modifier authority is split"
                )));
            }
            let modifiers = required(
                &side.effective_displayed_modifiers,
                &format!("{prefix}.effective_displayed_modifiers"),
            )?;
            const ORDER: [&str; 10] = [
                "BORDER_CONFLICT_MODIFIER",
                "BM_AIR_SUPPORT",
                "BM_ENEMY_AIR_SUPERIORITY",
                "BM_FORT_MODIFIER",
                "BM_SHORE_BOMBARD",
                "BM_RAILWAY_GUN_BOMBARD",
                "BM_STACKING",
                "BM_MULTIPLE_DIRECTIONS",
                "BM_DECRYPTION",
                "BM_ACTIVE_CODE_CRACKING_BONUSES",
            ];
            let mut next_rank = 0usize;
            for token in modifiers {
                let Some(rank) = ORDER.iter().position(|candidate| *candidate == token) else {
                    return Err(invalid(format!(
                        "{prefix}.effective_displayed_modifiers has unknown token"
                    )));
                };
                if rank < next_rank {
                    return Err(invalid(format!(
                        "{prefix}.effective_displayed_modifiers is duplicate or out of engine order"
                    )));
                }
                next_rank = rank + 1;
            }
            if *required(
                &side.panel_capture_sequence,
                &format!("{prefix}.panel_capture_sequence"),
            )? == 0
                || required(
                    &side.panel_observation_source,
                    &format!("{prefix}.panel_observation_source"),
                )? != "authenticated_a729_combatview_materializer_1415c9860"
            {
                return Err(invalid(format!(
                    "{prefix} panel observation lacks exact sequence/source authority"
                )));
            }
        }
        "unavailable_panel_not_observed" => {
            if leader_status != "unavailable_panel_not_observed"
                || side.effective_displayed_modifiers.measured().is_some()
                || side.panel_capture_sequence.measured().is_some()
                || side.panel_observation_source.measured().is_some()
            {
                return Err(invalid(format!(
                    "{prefix} unavailable panel status conflicts with measured panel data"
                )));
            }
        }
        _ => {
            return Err(invalid(format!(
                "{prefix}.effective_displayed_modifiers_status is unknown"
            )))
        }
    }
    required(
        &side.counts_incomplete,
        &format!("{prefix}.counts_incomplete"),
    )?;
    let reserve_ids = required(
        &side.reserve_division_ids,
        &format!("{prefix}.reserve_division_ids"),
    )?;
    let reserve_status = required(
        &side.reserve_members_status,
        &format!("{prefix}.reserve_members_status"),
    )?;
    match reserve_status.as_str() {
        "exact" => {
            if matches!(side.counts_incomplete, Leaf::Measured(false))
                && side.reserve.measured().copied() != Some(reserve_ids.len() as i64)
            {
                return Err(invalid(format!(
                    "{prefix}.reserve_division_ids count mismatch"
                )));
            }
        }
        "exact_counts_incomplete" if matches!(side.counts_incomplete, Leaf::Measured(true)) => {}
        "unavailable" => {
            return Err(external(format!("{prefix}.reserve members unavailable")));
        }
        _ => {
            return Err(invalid(format!(
                "{prefix}.reserve_members_status is invalid"
            )))
        }
    }
    let mut seen_reserves = std::collections::BTreeSet::new();
    if reserve_ids
        .iter()
        .any(|id| *id <= 0 || !seen_reserves.insert(*id))
    {
        return Err(invalid(format!(
            "{prefix}.reserve_division_ids contains invalid/duplicate identity"
        )));
    }
    required(
        &side.damage_attribution,
        &format!("{prefix}.damage_attribution"),
    )?;
    if let (Leaf::Measured(raw), Leaf::Measured(scaled)) = (&side.progress_raw, &side.progress) {
        if !(-100_000..=100_000).contains(raw)
            || !scaled.is_finite()
            || (*scaled - *raw as f64 / 100_000.0).abs() > 1e-9
        {
            return Err(invalid(format!("{prefix}.progress fixed-point mismatch")));
        }
    }
    if let Leaf::Measured(status) = &side.tactic_status {
        match status.as_str() {
            "exact" => {
                let Leaf::Measured(Some(tactic)) = &side.tactic else {
                    return Err(invalid(format!("{prefix}.tactic exact without identity")));
                };
                required(&tactic.id, &format!("{prefix}.tactic.id"))?;
                for (leaf, field) in [(&tactic.token, "token"), (&tactic.name, "name")] {
                    if required(leaf, &format!("{prefix}.tactic.{field}"))?
                        .trim()
                        .is_empty()
                    {
                        return Err(invalid(format!("{prefix}.tactic.{field} is empty")));
                    }
                }
            }
            "exact_none" if !matches!(side.tactic, Leaf::Measured(None)) => {
                return Err(invalid(format!("{prefix}.tactic exact_none mismatch")));
            }
            "exact_none" => {}
            "unavailable_legacy_stream" => {
                return Err(external(format!("{prefix}.tactic legacy stream")));
            }
            _ => return Err(invalid(format!("{prefix}.tactic_status is unknown"))),
        }
    }
    if matches!(side.counts_incomplete, Leaf::Measured(false)) {
        if let (Leaf::Measured(units), Leaf::Measured(committed), Leaf::Measured(reserve)) =
            (&side.units, &side.committed, &side.reserve)
        {
            if committed.saturating_add(*reserve) != *units {
                return Err(invalid(format!("{prefix} complete counts disagree")));
            }
        }
    }
    for (index, row) in required(
        &side.damage_attribution,
        &format!("{prefix}.damage_attribution"),
    )?
    .iter()
    .enumerate()
    {
        let item = format!("{prefix}.damage_attribution[{index}]");
        required(&row.dealer_tag_db_id, &format!("{item}.dealer_tag_db_id"))?;
        required(&row.victim_tag_db_id, &format!("{item}.victim_tag_db_id"))?;
        let damage = *required(&row.damage, &format!("{item}.damage"))?;
        required(&row.damage_raw, &format!("{item}.damage_raw"))?;
        if !damage.is_finite() {
            return Err(invalid(format!("{item}.damage is invalid")));
        }
    }
    Ok(())
}

fn validate_land_side_detail(rows: &[LandCombatPanel]) -> Result<(), GateFailure> {
    for (index, row) in rows.iter().enumerate() {
        let prefix = format!("land[{index}]");
        let phase = required(&row.phase_event, &format!("{prefix}.phase_event"))?;
        if phase.trim().is_empty() {
            return Err(invalid(format!("{prefix}.phase_event is empty")));
        }
        let generation = *required(
            &row.detail_generation,
            &format!("{prefix}.detail_generation"),
        )?;
        if generation < 0 {
            return Err(invalid(format!("{prefix}.detail_generation is negative")));
        }
        let detail_location = *required(
            &row.detail_location_province,
            &format!("{prefix}.detail_location_province"),
        )?;
        if Some(&detail_location) != row.location_province.measured() {
            return Err(invalid(format!("{prefix} detail location mismatch")));
        }
        required(&row.age_days, &format!("{prefix}.age_days"))?;
        required(&row.duration_days, &format!("{prefix}.duration_days"))?;
        if required(
            &row.side_detail_completeness,
            &format!("{prefix}.side_detail_completeness"),
        )? != "both"
        {
            return Err(external(format!(
                "{prefix}.side_detail_completeness is not both"
            )));
        }
        for (event_index, event) in required(
            &row.participant_events,
            &format!("{prefix}.participant_events"),
        )?
        .iter()
        .enumerate()
        {
            let ep = format!("{prefix}.participant_events[{event_index}]");
            for (leaf, field) in [
                (&event.event, "event"),
                (&event.side, "side"),
                (&event.semantic, "semantic"),
            ] {
                if required(leaf, &format!("{ep}.{field}"))?.trim().is_empty() {
                    return Err(invalid(format!("{ep}.{field} is empty")));
                }
            }
            let side = required(&event.side, &format!("{ep}.side"))?;
            if side != "attacker" && side != "defender" {
                return Err(invalid(format!("{ep}.side is invalid")));
            }
            for (leaf, field) in [
                (&event.generation, "generation"),
                (&event.division_id, "division_id"),
                (&event.old_side_count, "old_side_count"),
                (&event.new_side_count, "new_side_count"),
            ] {
                if *required(leaf, &format!("{ep}.{field}"))? < 0 {
                    return Err(invalid(format!("{ep}.{field} is negative")));
                }
            }
        }
        let attacker = required(&row.attacker, &format!("land[{index}].attacker"))?;
        let defender = required(&row.defender, &format!("land[{index}].defender"))?;
        validate_land_side(attacker, &format!("land[{index}].attacker"))?;
        validate_land_side(defender, &format!("land[{index}].defender"))?;
    }
    Ok(())
}

/// The naval-active surface is gated on the collection's own status first
/// (mirroring [`validate_ended_land_surface`]): a non-`Present` collection is
/// an external-input condition regardless of any retained rows, never an
/// admitted (or measured-empty) surface.
fn validate_naval_surface(
    status: ViewStatus,
    rows: &[NavalCombatRow],
) -> Result<(), GateFailure> {
    match status {
        ViewStatus::Present => validate_naval(rows),
        ViewStatus::DomainAbsent => Err(external("naval_combats field absent")),
        ViewStatus::DomainUnavailable => Err(external("naval_combats unavailable")),
    }
}

fn validate_naval(rows: &[NavalCombatRow]) -> Result<(), GateFailure> {
    for (index, row) in rows.iter().enumerate() {
        let prefix = format!("naval[{index}]");
        for (leaf, field) in [
            (&row.combat_id, "combat_id"),
            (&row.generation, "generation"),
        ] {
            if *required(leaf, &format!("{prefix}.{field}"))? < 0 {
                return Err(invalid(format!("{prefix}.{field} is negative")));
            }
        }
        if *required(
            &row.location_province,
            &format!("{prefix}.location_province"),
        )? == 0
            || *required(&row.region_id, &format!("{prefix}.region_id"))? == 0
        {
            return Err(invalid(format!("{prefix} location identity is zero")));
        }
        let attacker_count = *required(
            &row.attacker_member_count,
            &format!("{prefix}.attacker_member_count"),
        )? as usize;
        let defender_count = *required(
            &row.defender_member_count,
            &format!("{prefix}.defender_member_count"),
        )? as usize;
        for (raw_leaf, value_leaf, side) in [
            (
                &row.attacker_positioning_raw,
                &row.attacker_positioning,
                "attacker",
            ),
            (
                &row.defender_positioning_raw,
                &row.defender_positioning,
                "defender",
            ),
        ] {
            let raw = *required(raw_leaf, &format!("{prefix}.{side}_positioning_raw"))?;
            let value = *required(value_leaf, &format!("{prefix}.{side}_positioning"))?;
            if !(0..=100_000).contains(&raw)
                || !value.is_finite()
                || (value - raw as f64 / 100_000.0).abs() > 1e-9
            {
                return Err(invalid(format!(
                    "{prefix}.{side}_positioning exact raw/normalized pair is invalid"
                )));
            }
        }
        for (raw_leaf, value_leaf, side) in [
            (
                &row.attacker_anti_air_raw,
                &row.attacker_anti_air,
                "attacker",
            ),
            (
                &row.defender_anti_air_raw,
                &row.defender_anti_air,
                "defender",
            ),
        ] {
            let raw = *required(raw_leaf, &format!("{prefix}.{side}_anti_air_raw"))?;
            let value = *required(value_leaf, &format!("{prefix}.{side}_anti_air"))?;
            if !value.is_finite() || (value - raw as f64 / 100_000.0).abs() > 1e-9 {
                return Err(invalid(format!(
                    "{prefix}.{side}_anti_air exact raw/normalized pair is invalid"
                )));
            }
        }
        required(
            &row.attacker_task_force_ids,
            &format!("{prefix}.attacker_task_force_ids"),
        )?;
        required(
            &row.defender_task_force_ids,
            &format!("{prefix}.defender_task_force_ids"),
        )?;
        let attacker_countries = required(
            &row.attacker_country_tag_db_ids,
            &format!("{prefix}.attacker_country_tag_db_ids"),
        )?;
        let defender_countries = required(
            &row.defender_country_tag_db_ids,
            &format!("{prefix}.defender_country_tag_db_ids"),
        )?;
        let members = required(&row.members, &format!("{prefix}.members"))?;
        let mut previous: Option<(&str, i64)> = None;
        let mut measured_attackers = Vec::new();
        let mut measured_defenders = Vec::new();
        for (member_index, member) in members.iter().enumerate() {
            let mp = format!("{prefix}.members[{member_index}]");
            let side = required(&member.side, &format!("{mp}.side"))?;
            if side != "attacker" && side != "defender" {
                return Err(invalid(format!("{mp}.side is invalid")));
            }
            let group = *required(&member.group_index, &format!("{mp}.group_index"))?;
            let ordinal = *required(&member.member_index, &format!("{mp}.member_index"))?;
            if group < 0 || ordinal < 0 {
                return Err(invalid(format!("{mp} has a negative source ordinal")));
            }
            let is_convoy = *required(&member.is_convoy, &format!("{mp}.is_convoy"))?;
            let cship_join_validated = *required(
                &member.cship_join_validated,
                &format!("{mp}.cship_join_validated"),
            )?;
            let owner = *required(&member.owner_tag_db_id, &format!("{mp}.owner_tag_db_id"))?;
            if owner < 0 {
                return Err(invalid(format!("{mp}.owner_tag_db_id is negative")));
            }
            let battle_slot = *required(
                &member.battle_slot_index,
                &format!("{mp}.battle_slot_index"),
            )?;
            if battle_slot < 0 {
                return Err(invalid(format!("{mp}.battle_slot_index is negative")));
            }
            if let Some(prior) = previous {
                if (side.as_str(), battle_slot) <= prior {
                    return Err(invalid(format!(
                        "{mp} is not in exact side/battle-slot identity order"
                    )));
                }
            }
            previous = Some((side, battle_slot));
            let state_bitmask =
                *required(&member.state_bitmask, &format!("{mp}.state_bitmask"))?;
            let state_raw = *required(&member.state_raw, &format!("{mp}.state_raw"))?;
            let state_name = required(&member.state_name, &format!("{mp}.state_name"))?;
            let Some(expected_state_name) = exact_fex_state_name(state_raw) else {
                return Err(invalid(format!("{mp}.state_raw is not an exact CFEX state")));
            };
            if state_bitmask != state_raw || state_name.as_str() != expected_state_name {
                return Err(invalid(format!(
                    "{mp} exact CFEX state raw/name pair is inconsistent"
                )));
            }

            if is_convoy {
                if cship_join_validated {
                    return Err(invalid(format!(
                        "{mp} convoy member cannot carry a live CShip join"
                    )));
                }
                for (leaf, field) in [
                    (&member.ship_instance_id, "ship_instance_id"),
                    (&member.task_force_id, "task_force_id"),
                    (&member.fleet_id, "fleet_id"),
                    (&member.sunk_convoys, "sunk_convoys"),
                ] {
                    require_unobtained(leaf, &format!("{mp}.{field}"))?;
                }
                for (leaf, field) in [
                    (&member.strength, "strength"),
                    (&member.organisation, "organisation"),
                ] {
                    require_unobtained(leaf, &format!("{mp}.{field}"))?;
                }
            } else {
                let ship_instance_id = *required(
                    &member.ship_instance_id,
                    &format!("{mp}.ship_instance_id"),
                )?;
                if ship_instance_id <= 0 {
                    return Err(invalid(format!("{mp}.ship_instance_id is not positive")));
                }
                let terminal_sunk = state_raw == 0x20;
                if !cship_join_validated && !terminal_sunk {
                    return Err(invalid(format!(
                        "{mp} non-terminal ship lacks its required live CShip join"
                    )));
                }
                if cship_join_validated {
                    for (leaf, field) in [
                        (&member.task_force_id, "task_force_id"),
                        (&member.fleet_id, "fleet_id"),
                    ] {
                        if *required(leaf, &format!("{mp}.{field}"))? <= 0 {
                            return Err(invalid(format!("{mp}.{field} is not positive")));
                        }
                    }
                    for (leaf, field) in [
                        (&member.strength, "strength"),
                        (&member.organisation, "organisation"),
                    ] {
                        let value = *required(leaf, &format!("{mp}.{field}"))?;
                        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                            return Err(invalid(format!("{mp}.{field} is outside 0..1")));
                        }
                    }
                    if *required(&member.sunk_convoys, &format!("{mp}.sunk_convoys"))? < 0 {
                        return Err(invalid(format!("{mp}.sunk_convoys is negative")));
                    }
                } else {
                    for (leaf, field) in [
                        (&member.task_force_id, "task_force_id"),
                        (&member.fleet_id, "fleet_id"),
                        (&member.sunk_convoys, "sunk_convoys"),
                    ] {
                        require_unobtained(leaf, &format!("{mp}.{field}"))?;
                    }
                    for (leaf, field) in [
                        (&member.strength, "strength"),
                        (&member.organisation, "organisation"),
                    ] {
                        require_unobtained(leaf, &format!("{mp}.{field}"))?;
                    }
                }
            }
            if side == "attacker" {
                measured_attackers.push(owner);
            } else {
                measured_defenders.push(owner);
            }
        }
        if measured_attackers.len() != attacker_count || measured_defenders.len() != defender_count
        {
            return Err(invalid(format!("{prefix} member counts disagree")));
        }
        measured_attackers.sort_unstable();
        measured_attackers.dedup();
        measured_defenders.sort_unstable();
        measured_defenders.dedup();
        if &measured_attackers != attacker_countries || &measured_defenders != defender_countries {
            return Err(invalid(format!(
                "{prefix} country sets disagree with members"
            )));
        }
    }
    Ok(())
}

fn validate_air(rows: &Leaf<Vec<AirCombatLossRow>>) -> Result<usize, GateFailure> {
    let rows = required(rows, "air_losses")?;
    for (index, row) in rows.iter().enumerate() {
        let prefix = format!("air_losses[{index}]");
        if required(&row.owner, &format!("{prefix}.owner"))?
            .trim()
            .is_empty()
        {
            return Err(invalid(format!("{prefix}.owner is empty")));
        }
        for (leaf, field) in [
            (&row.owner_tag_db_id, "owner_tag_db_id"),
            (&row.wing_id, "wing_id"),
            (&row.generation, "generation"),
            (&row.mission_mask, "mission_mask"),
            (&row.flags, "flags"),
            (&row.air_accidents, "air_accidents"),
        ] {
            required(leaf, &format!("{prefix}.{field}"))?;
        }
        required(&row.mission_region, &format!("{prefix}.mission_region"))?;
        let old_count = *required(&row.old_count, &format!("{prefix}.old_count"))?;
        let new_count = *required(&row.new_count, &format!("{prefix}.new_count"))?;
        let count_delta = *required(&row.count_delta, &format!("{prefix}.count_delta"))?;
        if old_count < 0 || new_count < 0 || new_count - old_count != count_delta {
            return Err(invalid(format!("{prefix} count delta mismatch")));
        }
        let old_amount = *required(
            &row.old_equipment_amount_sum,
            &format!("{prefix}.old_equipment_amount_sum"),
        )?;
        let new_amount = *required(
            &row.new_equipment_amount_sum,
            &format!("{prefix}.new_equipment_amount_sum"),
        )?;
        let amount_delta = *required(
            &row.equipment_amount_delta,
            &format!("{prefix}.equipment_amount_delta"),
        )?;
        let experience = *required(&row.experience, &format!("{prefix}.experience"))?;
        if !old_amount.is_finite()
            || !new_amount.is_finite()
            || !amount_delta.is_finite()
            || !experience.is_finite()
            || ((new_amount - old_amount) - amount_delta).abs() > 1e-9
        {
            return Err(invalid(format!("{prefix} equipment delta mismatch")));
        }
    }
    Ok(rows.len())
}

fn validate_air_region(rows: &Leaf<Vec<AirRegionCombatRow>>) -> Result<usize, GateFailure> {
    let rows = required(rows, "air_region_combat")?;
    for (index, row) in rows.iter().enumerate() {
        let prefix = format!("air_region_combat[{index}]");
        for (leaf, field) in [
            (&row.owner, "owner"),
            (&row.attributed_country, "attributed_country"),
        ] {
            if required(leaf, &format!("{prefix}.{field}"))?
                .trim()
                .is_empty()
            {
                return Err(invalid(format!("{prefix}.{field} is empty")));
            }
        }
        for (leaf, field) in [
            (&row.owner_tag_db_id, "owner_tag_db_id"),
            (&row.side_vector, "side_vector"),
            (&row.selection_flag, "selection_flag"),
            (&row.classification_flag, "classification_flag"),
            (&row.copied_flag, "copied_flag"),
            (&row.wing_id, "wing_id"),
            (&row.date_raw, "date_raw"),
            (&row.plane_count, "plane_count"),
            (&row.mission_mask, "mission_mask"),
            (
                &row.attributed_country_tag_db_id,
                "attributed_country_tag_db_id",
            ),
        ] {
            required(leaf, &format!("{prefix}.{field}"))?;
        }
        if *required(&row.region_id, &format!("{prefix}.region_id"))? == 0 {
            return Err(invalid(format!("{prefix}.region_id is zero")));
        }
        if *required(&row.plane_count, &format!("{prefix}.plane_count"))? < 0 {
            return Err(invalid(format!("{prefix}.plane_count is negative")));
        }
        for (damage_index, damage) in required(&row.damage, &format!("{prefix}.damage"))?
            .iter()
            .enumerate()
        {
            let dp = format!("{prefix}.damage[{damage_index}]");
            for (leaf, field) in [(&damage.channel, "channel"), (&damage.cause, "cause")] {
                if required(leaf, &format!("{dp}.{field}"))?.trim().is_empty() {
                    return Err(invalid(format!("{dp}.{field} is empty")));
                }
            }
            required(&damage.cause_type, &format!("{dp}.cause_type"))?;
            if !required(&damage.amount, &format!("{dp}.amount"))?.is_finite() {
                return Err(invalid(format!("{dp}.amount is invalid")));
            }
        }
    }
    Ok(rows.len())
}

fn validate_convoys(rows: &Leaf<Vec<ConvoyLossRow>>) -> Result<usize, GateFailure> {
    let rows = required(rows, "sunk_convoys")?;
    for (index, row) in rows.iter().enumerate() {
        let prefix = format!("sunk_convoys[{index}]");
        required(&row.killer, &format!("{prefix}.killer"))?;
        required(&row.owner, &format!("{prefix}.owner"))?;
        if *required(&row.convoys, &format!("{prefix}.convoys"))? < 0 {
            return Err(invalid(format!("{prefix}.convoys is negative")));
        }
    }
    Ok(rows.len())
}

fn convoy_terminal_from_exact_view(
    view: &ConvoyTransportAttackExactView,
) -> Leaf<Vec<ConvoyTransportAttackExactRow>> {
    match view.status {
        ViewStatus::Present => match &view.events {
            Leaf::Measured(rows) => Leaf::Measured(rows.clone()),
            Leaf::Absent | Leaf::Unobtained => Leaf::Unobtained,
        },
        ViewStatus::DomainAbsent => Leaf::Absent,
        ViewStatus::DomainUnavailable => Leaf::Unobtained,
    }
}

fn validate_convoy_terminal(
    rows: &Leaf<Vec<ConvoyTransportAttackExactRow>>,
) -> Result<usize, GateFailure> {
    let rows = required(rows, "convoy_transport_attacks exact event ledger")?;
    let mut expected_sequence = 1u64;
    let mut d70_rows = rows.first().map(|row| row.d70_before_rows);
    let mut d70_by_ordinal: BTreeMap<u32, (u32, u32, u32)> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let prefix = format!("convoy_terminal[{index}]");
        if row.producer_sequence != expected_sequence {
            return Err(invalid(format!(
                "{prefix}.producer_sequence is not the gap-free cumulative ordinal"
            )));
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| invalid(format!("{prefix}.producer_sequence overflows")))?;
        if row.game_hour_raw <= 0
            || row.combat_id == 0
            || row.combat_generation == 0
            || row.convoy_id == 0
            || row.convoy_type_tag != 4_713
            || row.owner_tag_db_id == 0
            || row.owner_tag_db_id > replay_core::MAX_COMPACT_OWNER_TAG_DB_ID
            || row.killer_tag_db_id == 0
            || row.killer_tag_db_id > replay_core::MAX_COMPACT_OWNER_TAG_DB_ID
            || row.client_id == 0
            || row.client_type_tag != 4_713
            || row.member_state != 32
            || row.route_key_id != row.client_id
            || row.route_key_type_tag != row.client_type_tag
            || row.route_source_id == 0
            || row.route_destination_id == 0
        {
            return Err(invalid(format!(
                "{prefix} has invalid exact producer identity or route content"
            )));
        }
        match (
            row.route_producer_kind,
            row.route_source_kind,
            row.route_destination_kind,
        ) {
            (
                ConvoyRouteProducerKindExact::NavalTransport,
                ConvoyRouteEndpointKindExact::Province,
                ConvoyRouteEndpointKindExact::Province,
            )
            | (
                ConvoyRouteProducerKindExact::LendLease,
                ConvoyRouteEndpointKindExact::State,
                ConvoyRouteEndpointKindExact::State,
            ) => {}
            (ConvoyRouteProducerKindExact::Export, _, _) => {
                return Err(invalid(format!(
                    "{prefix}.route_producer_kind export lacks an authenticated endpoint contract"
                )));
            }
            _ => {
                return Err(invalid(format!(
                    "{prefix} route producer and endpoint kinds disagree"
                )));
            }
        }
        match row.d70_mutation_kind {
            ConvoyD70MutationKindExact::IncrementExisting
                if row.d70_before_rows != 0
                    && row.d70_after_rows == row.d70_before_rows
                    && row.d70_ordinal < row.d70_before_rows
                    && row.d70_before_count != 0
                    && row.d70_before_count.checked_add(1) == Some(row.d70_after_count) => {}
            ConvoyD70MutationKindExact::AppendFirstSeen
                if row.d70_before_rows.checked_add(1) == Some(row.d70_after_rows)
                    && row.d70_ordinal == row.d70_before_rows
                    && row.d70_before_count == 0
                    && row.d70_after_count == 1 => {}
            _ => {
                return Err(invalid(format!(
                    "{prefix} has an invalid d70 mutation witness"
                )));
            }
        }
        if d70_rows != Some(row.d70_before_rows) {
            return Err(invalid(format!(
                "{prefix} breaks the d70 cardinality chain"
            )));
        }
        if let Some((owner, killer, count)) = d70_by_ordinal.get(&row.d70_ordinal) {
            if (*owner, *killer, *count)
                != (
                    row.owner_tag_db_id,
                    row.killer_tag_db_id,
                    row.d70_before_count,
                )
            {
                return Err(invalid(format!(
                    "{prefix} breaks the d70 identity or count chain"
                )));
            }
            if matches!(
                row.d70_mutation_kind,
                ConvoyD70MutationKindExact::AppendFirstSeen
            ) {
                return Err(invalid(format!(
                    "{prefix} appends an existing d70 identity"
                )));
            }
        }
        d70_rows = Some(row.d70_after_rows);
        d70_by_ordinal.insert(
            row.d70_ordinal,
            (
                row.owner_tag_db_id,
                row.killer_tag_db_id,
                row.d70_after_count,
            ),
        );
    }
    Ok(rows.len())
}

/// The D60 belligerent-pair identity. `first`/`second` are what orients every
/// pair-scoped value on the row — including the H-B5 instigator/defender tuple
/// admitted by [`validate_war_threat_residuals`] — so a pair that names one
/// country twice, carries a blank tag, or repeats another row's pair would
/// attribute measured values to a relationship the producer never measured.
/// Requiring the two tags to be *measured* was never enough for that.
fn validate_war_pairs(rows: &Leaf<Vec<WarObjectRow>>) -> Result<usize, GateFailure> {
    let rows = required(rows, "wars")?;
    let mut pairs = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        if row.key.trim().is_empty() {
            return Err(invalid(format!("wars[{index}].key is empty")));
        }
        let first = required(&row.first, &format!("wars[{index}].first"))?;
        let second = required(&row.second, &format!("wars[{index}].second"))?;
        if first.trim().is_empty() || second.trim().is_empty() {
            return Err(invalid(format!("wars[{index}] pair has a blank party")));
        }
        if first == second {
            return Err(invalid(format!(
                "wars[{index}] names {first} as both parties"
            )));
        }
        // Order-insensitive: the producer emits one row per relationship, and
        // which side is `first` is the engine's orientation, not a licence to
        // carry the same pair twice.
        let pair = if first <= second {
            (first.clone(), second.clone())
        } else {
            (second.clone(), first.clone())
        };
        if !pairs.insert(pair) {
            return Err(invalid(format!(
                "wars[{index}] repeats the belligerent pair {first}/{second}"
            )));
        }
    }
    Ok(rows.len())
}

fn validate_war_participants(
    tags: &Leaf<Vec<String>>,
    participants: &Leaf<Vec<replay_core::WarParticipantOverviewRow>>,
    prefix: &str,
) -> Result<(), GateFailure> {
    let tags = required(tags, &format!("{prefix}.tags"))?;
    let participants = required(participants, &format!("{prefix}.participants"))?;
    if tags.is_empty() {
        return Err(invalid(format!("{prefix} is empty")));
    }
    if tags.len() != participants.len() {
        return Err(invalid(format!("{prefix} tag/participant count mismatch")));
    }
    for (index, (tag, row)) in tags.iter().zip(participants).enumerate() {
        if tag.trim().is_empty() {
            return Err(invalid(format!("{prefix}.tags[{index}] is empty")));
        }
        if required(&row.tag, &format!("{prefix}.participants[{index}].tag"))? != tag {
            return Err(invalid(format!(
                "{prefix}.participants[{index}] order mismatch"
            )));
        }
        required(
            &row.tag_db_id,
            &format!("{prefix}.participants[{index}].tag_db_id"),
        )?;
        let participation = *required(
            &row.participation_raw,
            &format!("{prefix}.participants[{index}].participation_raw"),
        )?;
        let capitulation = *required(
            &row.capitulation_progress_raw,
            &format!("{prefix}.participants[{index}].capitulation_progress_raw"),
        )?;
        if !(0..=100_000).contains(&participation) || !(0..=100_000).contains(&capitulation) {
            return Err(invalid(format!(
                "{prefix}.participants[{index}] fixed-point bounds"
            )));
        }
        let breakdown = required(
            &row.breakdown,
            &format!("{prefix}.participants[{index}].participation_breakdown"),
        )?;
        for (leaf, field) in [
            (&breakdown.land_damage, "land_damage"),
            (&breakdown.occupation, "occupation"),
            (&breakdown.air_damage, "air_damage"),
            (&breakdown.strategic_bombing, "strategic_bombing"),
            (&breakdown.sunk_ships, "sunk_ships"),
            (&breakdown.sunk_convoys, "sunk_convoys"),
            (&breakdown.casualties, "casualties"),
            (&breakdown.lend_lease_given, "lend_lease_given"),
            (&breakdown.lend_lease_received, "lend_lease_received"),
        ] {
            required(
                leaf,
                &format!("{prefix}.participants[{index}].breakdown.{field}"),
            )?;
        }
    }
    Ok(())
}

/// D86's per-row admission, applied from `replay_core`'s ONE copy.
///
/// This gate used to carry its own transcription of the whole rule -- the
/// value/reason pairing, the combat-join shape, the contiguous-from-2 sequence
/// -- minus `replay_core`'s calendar check. A rule written twice is a rule that
/// drifts: a fix landed in `replay_core` alone would have been undone here,
/// which is exactly how D93's and D86's earlier losses happened.
///
/// The calendar half -- `sink_date_raw` converts through the shared 1850..=2200
/// window and lands at or before the frame's hour -- is applied here too, from
/// `replay_core`'s [`replay_core::sunk_ship_event_sink_hour_is_admissible`],
/// which converts through `portable_admission`. Both halves are therefore
/// spelled once and evaluated identically here and where the view is built.
/// This gate used to apply only the first half, so a row `replay_core` had
/// refused on its sink date could still be admitted by the binding gate.
fn validate_sunk_ships(
    rows: &Leaf<Vec<SunkShipEventExact>>,
    completeness: SunkShipHistoryCompleteness,
    frame_hour: &Xui20FrameHour,
) -> Result<usize, GateFailure> {
    let rows = required(rows, "sunk_ships")?;
    // Reaching past `required` means the leaf is MEASURED, and a measured leaf
    // under `NoHistory` is a contradiction at the leaf level that the row
    // predicate cannot see: a bare slice carries no measured/unobtained state,
    // so an empty one is indistinguishable from a delivery that legitimately
    // has no rows. Admitting this pairing would report `Admitted { row_count:
    // 0 }` for a domain that was absent or refused -- collapsing "unavailable"
    // into "measured zero sinkings", which is the one distinction this binding
    // exists to keep. Production never builds it: `NoHistory` is paired with
    // `Leaf::Absent` or `Leaf::Unobtained` by the constructor below.
    if matches!(completeness, SunkShipHistoryCompleteness::NoHistory) {
        return Err(invalid(
            "sunk_ships is measured under a declaration that no history reached this view"
                .to_string(),
        ));
    }
    // Refused, never admitted on half the rule: without the frame hour the
    // calendar half cannot be evaluated at all, and admitting the rows anyway
    // is exactly the disagreement this threading exists to close.
    let expected_hour = match frame_hour {
        Xui20FrameHour::Measured(hour) => *hour,
        Xui20FrameHour::Unavailable { reason } => {
            return Err(external(format!(
                "sunk_ships calendar admission requires the replay frame hour: {reason}"
            )))
        }
    };
    // The hour-independent half, over the whole history at once: the sequence
    // budget a producer-declared partial licenses is not decidable one row at a
    // time, and the completeness state is what licenses it. Passing the gate's
    // own guess here instead of the view's verdict would be the transcription
    // this call exists to avoid.
    if let Err(refusal) = replay_core::sunk_ship_history_rows_are_admissible(rows, completeness) {
        return Err(invalid(format!("sunk_ships {}", refusal.describe())));
    }
    for (index, row) in rows.iter().enumerate() {
        if !replay_core::sunk_ship_event_sink_hour_is_admissible(row, expected_hour) {
            return Err(invalid(format!(
                "sunk_ships[{index}] sink date is outside the shared calendar window \
                 or later than the frame hour"
            )));
        }
    }
    Ok(rows.len())
}

fn validate_war_overview(rows: &Leaf<Vec<WarObjectRow>>) -> Result<usize, GateFailure> {
    let rows = required(rows, "wars")?;
    let mut war_ids = BTreeSet::new();
    let mut producer_ordinals = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let prefix = format!("wars[{index}]");
        if row.key.trim().is_empty() {
            return Err(invalid(format!("{prefix}.key is empty")));
        }
        if required(&row.war_name, &format!("{prefix}.war_name"))?
            .trim()
            .is_empty()
        {
            return Err(invalid(format!("{prefix}.war_name is empty")));
        }
        if let Leaf::Measured(front) = &row.front {
            if front.trim().is_empty() {
                return Err(invalid(format!("{prefix}.front is empty")));
            }
        }
        let war_id = *required(&row.war_id, &format!("{prefix}.war_id"))?;
        let producer_ordinal =
            *required(&row.producer_ordinal, &format!("{prefix}.producer_ordinal"))?;
        required(
            &row.aggregate_summary_flag,
            &format!("{prefix}.aggregate_summary_flag"),
        )?;
        if war_id <= 0
            || producer_ordinal < 0
            || !war_ids.insert(war_id)
            || !producer_ordinals.insert(producer_ordinal)
        {
            return Err(invalid(format!("{prefix} identity/order is invalid")));
        }
        if *required(&row.start_hour, &format!("{prefix}.start_hour"))? < 0 {
            return Err(invalid(format!("{prefix}.start_hour is negative")));
        }
        validate_war_participants(
            &row.attackers,
            &row.attacker_participants,
            &format!("wars[{index}].attackers"),
        )?;
        validate_war_participants(
            &row.defenders,
            &row.defender_participants,
            &format!("wars[{index}].defenders"),
        )?;
        let attackers = required(&row.attackers, &format!("{prefix}.attackers"))?;
        let defenders = required(&row.defenders, &format!("{prefix}.defenders"))?;
        let mut unique = BTreeSet::new();
        for tag in attackers.iter().chain(defenders) {
            if !unique.insert(tag) {
                return Err(invalid(format!("{prefix} has duplicate participant {tag}")));
            }
        }
        let progress = *required(
            &row.overall_progress_raw,
            &format!("wars[{index}].overall_progress_raw"),
        )?;
        if !(-100_000..=100_000).contains(&progress) {
            return Err(invalid(format!(
                "wars[{index}].overall_progress_raw bounds"
            )));
        }
    }
    if producer_ordinals != (0..rows.len() as i64).collect() {
        return Err(invalid("wars producer order is not contiguous"));
    }
    Ok(rows.len())
}

/// EIR-NE-011: admit the exact-a729 H-B5 residual tuple across the complete
/// current D60 war set.
///
/// The disposition is derived from the frame's own measured leaves — nothing
/// here is hard-coded to a build, a schema version or a producer flag. The
/// consequences of that:
///
/// * every war row is admitted through the SAME typed law the war-overview
///   binding applies ([`crate::waroverview_binding::admit_war_object_residuals`]),
///   so this admission and the typed consumer can never reach opposite
///   verdicts on one row;
/// * the pair gate runs first: the tuple is oriented by `first`/`second`, so a
///   blank, self, or repeated pair makes the residual unattributable;
/// * coverage is all-or-none across the war set. A set where some rows carry
///   the tuple (wholly or partly) and others carry nothing at all is a producer
///   contract break, not a partially populated panel, because the D60 producer
///   sets its residual badge only when every row carries it. Every row is
///   classified before the disposition is chosen, so this outranks whatever
///   else is wrong inside the carried rows;
/// * inside a fully-carrying set, a contradictory tuple is a REJECTION and a
///   missing leaf is an EXTERNAL gap naming that leaf — the producer supplying
///   something wrong and the producer supplying nothing are different facts;
/// * a war set where NO row carries the tuple is an external gap (a pre-v2
///   recording), never a measured zero threat;
/// * a measured-empty war set (no wars this hour) admits with `row_count` 0 and
///   stays distinct from all of the above.
fn validate_war_threat_residuals(rows: &Leaf<Vec<WarObjectRow>>) -> Result<usize, GateFailure> {
    // The tuple is oriented by the pair identity, so an unusable pair makes the
    // residual meaningless even when all seven leaves are present. Reusing the
    // pair gate rather than repeating it keeps the two from drifting apart.
    validate_war_pairs(rows)?;
    let rows = required(rows, "wars")?;
    // Classify EVERY row before choosing the aggregate disposition. Returning on
    // the first non-measured row would report an incomplete tuple as a plain
    // external gap even when the rest of the set proves the coverage is mixed,
    // which is the producer contract break and outranks it.
    let mut measured = 0usize;
    let mut absent = 0usize;
    let mut first_incomplete: Option<String> = None;
    let mut first_inconsistent: Option<String> = None;
    for (index, row) in rows.iter().enumerate() {
        match crate::waroverview_binding::admit_war_object_residuals(row) {
            WarResidualAdmission::Measured(_) => measured += 1,
            WarResidualAdmission::Absent => absent += 1,
            WarResidualAdmission::Incomplete { field } => {
                first_incomplete.get_or_insert_with(|| format!("wars[{index}].{field}"));
            }
            WarResidualAdmission::Inconsistent { reason } => {
                first_inconsistent
                    .get_or_insert_with(|| format!("wars[{index}] H-B5 residual: {reason}"));
            }
        }
    }
    let carried = measured + first_incomplete.iter().count() + first_inconsistent.iter().count();
    // Mixed coverage first: a set where some rows carry the tuple and others do
    // not contradicts the producer's own all-or-none badge, whatever else is
    // wrong inside the carried rows.
    if carried > 0 && absent > 0 {
        return Err(invalid(format!(
            "H-B5 residual coverage is partial: {} of {} war rows carry the tuple",
            rows.len() - absent,
            rows.len()
        )));
    }
    if let Some(reason) = first_inconsistent {
        return Err(invalid(reason));
    }
    if let Some(field) = first_incomplete {
        return Err(external(field));
    }
    if absent > 0 {
        return Err(external("wars[*].threat_fixed5_raw"));
    }
    Ok(measured)
}

/// Exact replay admission. Each producer family is gated independently and
/// no cross-family ordering key is constructed.
///
/// `frame_hour` is the hour the binding was taken at, obtained from
/// authoritative replay state via [`Xui20FrameHour::from_frame`]. D86's
/// calendar admission is meaningless without it, so it is a parameter rather
/// than something this function could default: a caller that has no hour
/// passes [`Xui20FrameHour::Unavailable`] and the D86 surface refuses.
pub fn admit_xui20_replay(
    binding: &Xui20CombatDetailLosses,
    frame_hour: &Xui20FrameHour,
) -> Xui20ReplayAdmission {
    let air = validate_air(&binding.air_losses);
    let air_region = validate_air_region(&binding.air_region_combat);
    let convoys = validate_convoys(&binding.sunk_convoys);
    let convoy_terminal = validate_convoy_terminal(&binding.convoy_terminal);
    let sunk_ships = validate_sunk_ships(
        &binding.sunk_ships,
        binding.sunk_ships_completeness,
        frame_hour,
    );
    let war_pairs = validate_war_pairs(&binding.wars);
    let war_overview = validate_war_overview(&binding.wars);
    let war_threat_residuals = validate_war_threat_residuals(&binding.wars);
    let family_rows = binding.land.len()
        + binding.ended_land.len()
        + binding.naval.len()
        + binding.air_losses.measured().map_or(0, Vec::len)
        + binding.air_region_combat.measured().map_or(0, Vec::len)
        + binding.sunk_convoys.measured().map_or(0, Vec::len)
        + binding.sunk_ships.measured().map_or(0, Vec::len)
        + binding.wars.measured().map_or(0, Vec::len)
        + binding.war_relations.measured().map_or(0, Vec::len);
    let land_active = state(
        binding.land.len(),
        validate_land_active_surface(binding.land_status, &binding.land),
    );
    let land_side_detail = state(
        binding.land.len(),
        validate_land_side_detail_surface(binding.land_status, &binding.land),
    );
    let land_ended_lifecycle = state(
        binding.ended_land.len(),
        validate_ended_land_surface(binding.ended_land_status, &binding.ended_land),
    );
    let land_terminal_result = validate_land_terminal_surface(
        binding.ended_land_status,
        binding.land_status,
        &binding.land,
        &binding.ended_land,
    );
    let land_terminal = state(
        land_terminal_result.as_ref().copied().unwrap_or(0),
        land_terminal_result.map(|_| ()),
    );
    let naval_active = state(
        binding.naval.len(),
        validate_naval_surface(binding.naval_status, &binding.naval),
    );
    let air_history = state(air.as_ref().map_or(0, |count| *count), air.map(|_| ()));
    let air_region_history = state(
        air_region.as_ref().map_or(0, |count| *count),
        air_region.map(|_| ()),
    );
    let convoy_history = state(
        convoys.as_ref().map_or(0, |count| *count),
        convoys.map(|_| ()),
    );
    let convoy_terminal = state(
        convoy_terminal.as_ref().map_or(0, |count| *count),
        convoy_terminal.map(|_| ()),
    );
    let naval_terminal = state(
        sunk_ships.as_ref().map_or(0, |count| *count),
        sunk_ships.map(|_| ()),
    );
    let war_pair_history = state(
        war_pairs.as_ref().map_or(0, |count| *count),
        war_pairs.map(|_| ()),
    );
    let combat_log_families = family_state(
        family_rows,
        &[
            &land_active,
            &land_side_detail,
            &land_ended_lifecycle,
            &naval_active,
            &air_history,
            &air_region_history,
            &convoy_history,
            &naval_terminal,
            &war_pair_history,
        ],
    );
    Xui20ReplayAdmission {
        land_active,
        land_side_detail,
        land_ended_lifecycle,
        land_terminal,
        naval_active,
        naval_terminal,
        air_history,
        air_region_history,
        convoy_history,
        convoy_terminal,
        combat_log_families,
        global_cross_family_order: Xui20SurfaceState::NotApplicable {
            reason: "COrdersGroupLogs persists separate family vectors; no global order exists"
                .into(),
        },
        war_pair_history,
        war_overview_exact: state(
            war_overview.as_ref().map_or(0, |count| *count),
            war_overview.map(|_| ()),
        ),
        war_threat_residuals_exact: state(
            war_threat_residuals.as_ref().map_or(0, |count| *count),
            war_threat_residuals.map(|_| ()),
        ),
    }
}

fn land_panel(row: &CombatRow) -> LandCombatPanel {
    let (
        phase_event,
        detail_generation,
        detail_location_province,
        age_days,
        duration_days,
        terminal_record,
        terminal_record_status,
        side_detail_completeness,
        participant_events,
        attacker,
        defender,
    ) = match row.detail.measured() {
        Some(detail) => (
            detail.phase_event.clone(),
            detail.generation.clone(),
            detail.location_province.clone(),
            detail.age_days.clone(),
            detail.duration_days.clone(),
            detail.terminal_record.clone(),
            detail.terminal_record_status.clone(),
            detail.side_detail_completeness.clone(),
            detail.participant_events.clone(),
            detail.attacker.clone(),
            detail.defender.clone(),
        ),
        None if matches!(&row.detail, Leaf::Unobtained) => (
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
            Leaf::Unobtained,
        ),
        None => (
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Absent,
        ),
    };
    let attacker_leader = attacker.measured().map_or_else(
        || {
            if matches!(&attacker, Leaf::Unobtained) {
                Leaf::Unobtained
            } else {
                Leaf::Absent
            }
        },
        |side| side.leader_id.clone(),
    );
    let defender_leader = defender.measured().map_or_else(
        || {
            if matches!(&defender, Leaf::Unobtained) {
                Leaf::Unobtained
            } else {
                Leaf::Absent
            }
        },
        |side| side.leader_id.clone(),
    );
    let attacker_displayed_modifiers = attacker
        .measured()
        .map_or(Leaf::Absent, |side| side.effective_displayed_modifiers.clone());
    let defender_displayed_modifiers = defender
        .measured()
        .map_or(Leaf::Absent, |side| side.effective_displayed_modifiers.clone());
    LandCombatPanel {
        combat_id: row.combat_id.clone(),
        location_province: row.location_province.clone(),
        attackers: row.attackers.clone(),
        defenders: row.defenders.clone(),
        combat_width: row.combat_width.clone(),
        terrain: row.terrain.clone(),
        progress: row.progress.clone(),
        phase_event,
        detail_generation,
        detail_location_province,
        age_days,
        duration_days,
        terminal_record,
        terminal_record_status,
        side_detail_completeness,
        participant_events,
        attacker,
        defender,
        attacker_leader,
        defender_leader,
        attacker_displayed_modifiers,
        defender_displayed_modifiers,
    }
}

/// Bind the exact producer-owned XUI20 surfaces for one replay hour.
pub fn combat_detail_losses_from_frame(frame: &WorldFrame) -> Xui20CombatDetailLosses {
    let naval_air = frame.naval_air_combat();
    let diplomacy: &DiplomacyView = frame.diplomacy();
    let convoy_terminal = convoy_terminal_from_exact_view(frame.convoy_transport_attacks());
    let sunk_ships = match frame.sunk_ship_history_exact().status {
        ViewStatus::Present => frame
            .sunk_ship_history_exact()
            .history
            .as_ref()
            .map(|history| Leaf::Measured(history.events.clone()))
            .unwrap_or(Leaf::Unobtained),
        ViewStatus::DomainAbsent => Leaf::Absent,
        ViewStatus::DomainUnavailable => Leaf::Unobtained,
    };
    // Carried through from `replay_core`'s admission verdict, never re-derived
    // here: the rows and the statement about what is missing beside them come
    // from the same view, so the surface cannot show one without the other.
    let sunk_ships_completeness = frame.sunk_ship_history_exact().completeness;
    let land_status = frame.combats_status();
    let ended_land_status = frame.ended_combats_status();
    Xui20CombatDetailLosses {
        // Fail-closed, exactly as the naval sibling below: a non-Present
        // collection binds no rows even if the frame retained state; the
        // status names which non-measured fact it is.
        land: if land_status == ViewStatus::Present {
            frame.combats().iter().map(land_panel).collect()
        } else {
            Vec::new()
        },
        land_status,
        ended_land: if ended_land_status == ViewStatus::Present {
            frame.ended_combats().clone()
        } else {
            BTreeMap::new()
        },
        ended_land_status,
        // Fail-closed: a non-Present collection binds no rows even if the
        // frame retained state; the status names which non-measured fact it is.
        naval: if naval_air.naval_combats_status == ViewStatus::Present {
            naval_air.naval_combats.clone()
        } else {
            Vec::new()
        },
        naval_status: naval_air.naval_combats_status,
        air_losses: naval_air.air_losses.clone(),
        air_region_combat: naval_air.air_region_combat.clone(),
        sunk_convoys: naval_air.sunk_convoys.clone(),
        convoy_terminal,
        sunk_ships,
        sunk_ships_completeness,
        wars: diplomacy.wars.clone(),
        war_relations: diplomacy.war_relations.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        LandCombatCurrentLossObservation, LandCombatDetail, LandCombatEquipmentLossDelta,
        LandCombatSideDetail, LandCombatWidthObservation, LandCombatWidthWrapper,
    };

    fn combat_row_with_detail(detail: Leaf<LandCombatDetail>) -> CombatRow {
        CombatRow {
            combat_id: Leaf::Measured(8),
            generation: Leaf::Measured(0),
            kind: Leaf::Measured("land".into()),
            location_province: Leaf::Measured(42),
            attackers: Leaf::Measured(vec![1]),
            defenders: Leaf::Measured(vec![2]),
            combat_width: Leaf::Measured(80.0),
            terrain: Leaf::Measured("plains".into()),
            progress: Leaf::Measured(0.5),
            detail,
        }
    }

    #[test]
    fn land_panel_preserves_terminal_leaf_four_state_fan_out() {
        let unobtained = land_panel(&combat_row_with_detail(Leaf::Unobtained));
        assert_eq!(unobtained.terminal_record, Leaf::Unobtained);
        assert_eq!(unobtained.terminal_record_status, Leaf::Unobtained);

        let absent = land_panel(&combat_row_with_detail(Leaf::Absent));
        assert_eq!(absent.terminal_record, Leaf::Absent);
        assert_eq!(absent.terminal_record_status, Leaf::Absent);

        let record = LandCombatTerminalRecord {
            record_index: Leaf::Measured(3),
            writer: Leaf::Measured("overrun".into()),
            attacker_manpower_loss_raw: Leaf::Measured(0),
            defender_manpower_loss_raw: Leaf::Measured(0),
            end_date_raw: Leaf::Measured(9),
            location_province: Leaf::Measured(42),
            winner_side_raw: Leaf::Measured(7),
            winter: Leaf::Measured(true),
            result_selector_raw: Leaf::Measured(0),
            layout_side_raw: Leaf::Measured(8),
            source: Leaf::Measured("authenticated_a729_terminal_writer".into()),
        };
        let measured = land_panel(&combat_row_with_detail(Leaf::Measured(LandCombatDetail {
            phase_event: Leaf::Measured("ended".into()),
            generation: Leaf::Measured(1),
            location_province: Leaf::Measured(42),
            age_days: Leaf::Measured(1),
            duration_days: Leaf::Measured(1),
            terminal_record: Leaf::Measured(record.clone()),
            terminal_record_status: Leaf::Measured("exact".into()),
            participant_events: Leaf::Measured(Vec::new()),
            side_detail_completeness: Leaf::Unobtained,
            attacker: Leaf::Unobtained,
            defender: Leaf::Unobtained,
        })));
        assert_eq!(measured.terminal_record, Leaf::Measured(record));
        assert_eq!(
            measured.terminal_record_status,
            Leaf::Measured("exact".into())
        );
    }

    fn empty_side() -> LandCombatSideDetail {
        LandCombatSideDetail {
            units: Leaf::Measured(3),
            committed: Leaf::Measured(2),
            reserve: Leaf::Measured(1),
            air_kills: Leaf::Measured(4),
            org_loss_summary_index: Leaf::Measured(0),
            num_org_losses: Leaf::Measured(1),
            anti_air_attack: Leaf::Measured(0.25),
            air_damage_str: Leaf::Measured(0.1),
            air_damage_org: Leaf::Measured(0.2),
            ground_damage_str: Leaf::Measured(0.3),
            ground_damage_org: Leaf::Measured(0.4),
            prevented_damage_str: Leaf::Measured(0.05),
            prevented_damage_org: Leaf::Measured(0.06),
            progress: Leaf::Measured(0.53),
            progress_raw: Leaf::Measured(53_000),
            tactic: Leaf::Measured(Some(replay_core::LandCombatTacticIdentity {
                id: Leaf::Measured(7),
                token: Leaf::Measured("tactic_basic_attack".into()),
                name: Leaf::Measured("TACTIC_BASIC_ATTACK".into()),
            })),
            tactic_status: Leaf::Measured("exact".into()),
            losses_status: Leaf::Measured("unavailable_no_authenticated_live_carrier".into()),
            width_status: Leaf::Measured("unavailable_pending_x2_control".into()),
            width_observations: Leaf::Absent,
            current_loss_observations: Leaf::Absent,
            leader_id: Leaf::Absent,
            leader_status: Leaf::Measured("unavailable_panel_not_observed".into()),
            effective_displayed_modifiers: Leaf::Absent,
            effective_displayed_modifiers_status: Leaf::Measured(
                "unavailable_panel_not_observed".into(),
            ),
            panel_capture_sequence: Leaf::Absent,
            panel_observation_source: Leaf::Absent,
            counts_incomplete: Leaf::Measured(false),
            reserve_division_ids: Leaf::Measured(vec![30]),
            reserve_members_status: Leaf::Measured("exact".into()),
            damage_attribution: Leaf::Measured(Vec::new()),
        }
    }

    #[test]
    fn land_binding_preserves_measured_detail_and_fails_closed_for_unowned_fields() {
        let detail = LandCombatDetail {
            phase_event: Leaf::Measured("updated".into()),
            generation: Leaf::Measured(2),
            location_province: Leaf::Measured(42),
            age_days: Leaf::Measured(1),
            duration_days: Leaf::Measured(1),
            terminal_record: Leaf::Absent,
            terminal_record_status: Leaf::Absent,
            participant_events: Leaf::Measured(Vec::new()),
            side_detail_completeness: Leaf::Measured("both".into()),
            attacker: Leaf::Measured(empty_side()),
            defender: Leaf::Measured(empty_side()),
        };
        let row = CombatRow {
            combat_id: Leaf::Measured(7),
            generation: Leaf::Measured(0),
            kind: Leaf::Measured("land".into()),
            location_province: Leaf::Measured(42),
            attackers: Leaf::Measured(vec![10, 11]),
            defenders: Leaf::Measured(vec![20]),
            combat_width: Leaf::Unobtained,
            terrain: Leaf::Unobtained,
            progress: Leaf::Unobtained,
            detail: Leaf::Measured(detail),
        };
        let panel = land_panel(&row);
        assert_eq!(panel.phase_event, Leaf::Measured("updated".into()));
        assert_eq!(
            panel.attacker.measured().unwrap().reserve,
            Leaf::Measured(1)
        );
        assert_eq!(
            panel.attacker.measured().unwrap().progress_raw,
            Leaf::Measured(53_000)
        );
        assert_eq!(
            panel.attacker.measured().unwrap().tactic_status,
            Leaf::Measured("exact".into())
        );
        assert_eq!(panel.attacker_leader, Leaf::Absent);
        assert_eq!(panel.attacker_displayed_modifiers, Leaf::Absent);
    }

    #[test]
    fn unavailable_detail_does_not_become_empty_or_zero() {
        let row = CombatRow {
            combat_id: Leaf::Measured(8),
            generation: Leaf::Measured(0),
            kind: Leaf::Measured("land".into()),
            location_province: Leaf::Measured(9),
            attackers: Leaf::Measured(Vec::new()),
            defenders: Leaf::Measured(Vec::new()),
            combat_width: Leaf::Unobtained,
            terrain: Leaf::Unobtained,
            progress: Leaf::Unobtained,
            detail: Leaf::Unobtained,
        };
        let panel = land_panel(&row);
        assert_eq!(panel.attacker, Leaf::Unobtained);
        assert_eq!(panel.participant_events, Leaf::Unobtained);
        assert_eq!(panel.combat_width, Leaf::Unobtained);
    }

    #[test]
    fn exact_current_loss_requires_one_authenticated_producer_and_exact_deltas() {
        let mut side = empty_side();
        side.losses_status = Leaf::Measured("exact_current_epoch_writer".into());
        side.current_loss_observations = Leaf::Measured(vec![
            LandCombatCurrentLossObservation {
                capture_sequence: Leaf::Measured(91),
                manpower_before_raw: Leaf::Measured(300),
                manpower_after_raw: Leaf::Measured(287),
                equipment: Leaf::Measured(vec![LandCombatEquipmentLossDelta {
                    ordinal: Leaf::Measured(0),
                    equipment_id: Leaf::Measured(4_120),
                    before_raw: Leaf::Measured(2_000_000),
                    after_raw: Leaf::Measured(1_900_000),
                }]),
                source: Leaf::Measured(
                    "authenticated_a729_active_division_damage_140c7bfc0".into(),
                ),
            },
        ]);
        assert!(validate_land_side(&side, "attacker").is_ok());

        let Leaf::Measured(observations) = &mut side.current_loss_observations else {
            unreachable!();
        };
        observations[0].source = Leaf::Measured(
            "authenticated_a729_participant_lifecycle_141299850".into(),
        );
        assert!(validate_land_side(&side, "attacker").is_ok());

        let Leaf::Measured(observations) = &mut side.current_loss_observations else {
            unreachable!();
        };
        observations[0].source = Leaf::Measured("ambiguous_current_loss_writer".into());
        assert!(validate_land_side(&side, "attacker").is_err());
    }

    #[test]
    fn exact_width_requires_authenticated_ordered_exact_sum() {
        let mut side = empty_side();
        side.width_status = Leaf::Measured("exact_synchronous_call".into());
        side.width_observations = Leaf::Measured(vec![LandCombatWidthObservation {
            capture_sequence: Leaf::Measured(91),
            final_width_raw: Leaf::Measured(4_000_000),
            wrappers: Leaf::Measured(vec![
                LandCombatWidthWrapper {
                    ordinal: Leaf::Measured(0),
                    division_id: Leaf::Measured(8_001),
                    width_raw: Leaf::Measured(2_000_000),
                },
                LandCombatWidthWrapper {
                    ordinal: Leaf::Measured(1),
                    division_id: Leaf::Measured(8_002),
                    width_raw: Leaf::Measured(2_000_000),
                },
            ]),
            source: Leaf::Measured("authenticated_a729_141299610_call".into()),
        }]);
        assert!(validate_land_side(&side, "attacker").is_ok());

        let Leaf::Measured(observations) = &mut side.width_observations else {
            unreachable!();
        };
        observations[0].final_width_raw = Leaf::Measured(3_999_999);
        assert!(matches!(
            validate_land_side(&side, "attacker"),
            Err(GateFailure::Invalid(_))
        ));

        let Leaf::Measured(observations) = &mut side.width_observations else {
            unreachable!();
        };
        observations[0].final_width_raw = Leaf::Measured(4_000_000);
        let Leaf::Measured(wrappers) = &mut observations[0].wrappers else {
            unreachable!();
        };
        wrappers[1].division_id = Leaf::Measured(8_001);
        assert!(matches!(
            validate_land_side(&side, "attacker"),
            Err(GateFailure::Invalid(_))
        ));

        side.width_status = Leaf::Measured("unavailable_pending_x2_control".into());
        assert!(matches!(
            validate_land_side(&side, "attacker"),
            Err(GateFailure::Invalid(_))
        ));
    }

    #[test]
    fn unavailable_current_loss_rejects_measured_rows() {
        let mut side = empty_side();
        side.current_loss_observations = Leaf::Measured(Vec::new());
        assert!(validate_land_side(&side, "defender").is_err());
    }

    #[test]
    fn exact_panel_observation_requires_joined_leader_modifier_order_and_source() {
        let mut side = empty_side();
        side.leader_id = Leaf::Measured(Some(4_713));
        side.leader_status = Leaf::Measured("exact".into());
        side.effective_displayed_modifiers = Leaf::Measured(vec![
            "BM_AIR_SUPPORT".into(),
            "BM_STACKING".into(),
            "BM_DECRYPTION".into(),
        ]);
        side.effective_displayed_modifiers_status = Leaf::Measured("exact".into());
        side.panel_capture_sequence = Leaf::Measured(51);
        side.panel_observation_source =
            Leaf::Measured("authenticated_a729_combatview_materializer_1415c9860".into());
        assert!(validate_land_side(&side, "attacker").is_ok());

        side.effective_displayed_modifiers = Leaf::Measured(vec![
            "BM_STACKING".into(),
            "BM_AIR_SUPPORT".into(),
        ]);
        assert!(matches!(
            validate_land_side(&side, "attacker"),
            Err(GateFailure::Invalid(_))
        ));
    }

    fn convoy_terminal_row(
        producer_sequence: u64,
        mutation: ConvoyD70MutationKindExact,
    ) -> ConvoyTransportAttackExactRow {
        let (before_count, after_count) = match mutation {
            ConvoyD70MutationKindExact::AppendFirstSeen => (0, 1),
            ConvoyD70MutationKindExact::IncrementExisting => (1, 2),
        };
        ConvoyTransportAttackExactRow {
            producer_sequence,
            game_hour_raw: 61_000_000,
            combat_id: 21,
            combat_generation: 4,
            convoy_id: 158_065,
            convoy_type_tag: 4_713,
            owner_tag_db_id: 2,
            killer_tag_db_id: 51,
            client_id: 106_381,
            client_type_tag: 4_713,
            convoy_index: 8,
            member_state: 32,
            battle_slot: 8,
            route_key_id: 106_381,
            route_key_type_tag: 4_713,
            route_producer_kind: ConvoyRouteProducerKindExact::LendLease,
            route_source_kind: ConvoyRouteEndpointKindExact::State,
            route_source_id: 282,
            route_destination_kind: ConvoyRouteEndpointKindExact::State,
            route_destination_id: 64,
            d70_mutation_kind: mutation,
            d70_ordinal: 0,
            d70_before_rows: if producer_sequence == 1 { 0 } else { 1 },
            d70_after_rows: 1,
            d70_before_count: before_count,
            d70_after_count: after_count,
        }
    }

    /// The convoy-terminal owner/killer ceiling is the shared compact identity
    /// bound, at its exact value: the ceiling is admitted and one past it is
    /// not. This gate carried a hand-typed copy of the number, three layers
    /// away from where the producer applies it. Spelling the boundary in the
    /// test as the constant it guards would pass for whatever the constant
    /// happened to be, so the ceiling is pinned to the id space's structural
    /// width (`i32::MAX - 1`, lane GV).
    #[test]
    fn convoy_terminal_identity_ceiling_is_the_shared_compact_bound() {
        assert_eq!(replay_core::MAX_COMPACT_OWNER_TAG_DB_ID, 2_147_483_646);
        let admitted = |owner: u32, killer: u32| {
            let mut row =
                convoy_terminal_row(1, ConvoyD70MutationKindExact::AppendFirstSeen);
            row.owner_tag_db_id = owner;
            row.killer_tag_db_id = killer;
            validate_convoy_terminal(&Leaf::Measured(vec![row])).is_ok()
        };
        let max = replay_core::MAX_COMPACT_OWNER_TAG_DB_ID as u32;
        // Boundary walk: the old ceiling and one past it are ordinary admitted
        // ids now; the structural ceiling is admitted exactly; one past it is
        // refused on either side of the pair.
        assert!(admitted(2_048, 2_048));
        assert!(admitted(2_049, 2_048));
        assert!(admitted(2_048, 2_049));
        assert!(admitted(max, max));
        assert!(!admitted(max + 1, max));
        assert!(!admitted(max, max + 1));
        assert!(!admitted(0, max));
        assert!(!admitted(max, 0));
    }

    #[test]
    fn convoy_terminal_requires_present_exact_replay_view() {
        let events = vec![
            convoy_terminal_row(1, ConvoyD70MutationKindExact::AppendFirstSeen),
            convoy_terminal_row(2, ConvoyD70MutationKindExact::IncrementExisting),
        ];
        let exact = ConvoyTransportAttackExactView {
            status: ViewStatus::Present,
            events: Leaf::Measured(events.clone()),
        };
        let bound = convoy_terminal_from_exact_view(&exact);
        assert_eq!(bound, Leaf::Measured(events));
        assert!(matches!(validate_convoy_terminal(&bound), Ok(2)));

        let poisoned = ConvoyTransportAttackExactView {
            status: ViewStatus::DomainUnavailable,
            events: exact.events,
        };
        let bound = convoy_terminal_from_exact_view(&poisoned);
        assert_eq!(bound, Leaf::Unobtained);
        assert!(matches!(
            validate_convoy_terminal(&bound),
            Err(GateFailure::External(_))
        ));
    }

    #[test]
    fn convoy_terminal_rejects_gap_or_unsupported_route_contract() {
        let mut row = convoy_terminal_row(2, ConvoyD70MutationKindExact::AppendFirstSeen);
        let rows = Leaf::Measured(vec![row.clone()]);
        assert!(matches!(
            validate_convoy_terminal(&rows),
            Err(GateFailure::Invalid(_))
        ));

        row.producer_sequence = 1;
        row.route_producer_kind = ConvoyRouteProducerKindExact::Export;
        let rows = Leaf::Measured(vec![row]);
        assert!(matches!(
            validate_convoy_terminal(&rows),
            Err(GateFailure::Invalid(_))
        ));
    }
}
