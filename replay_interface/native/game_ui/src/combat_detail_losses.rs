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
    LandCombatSideDetail, Leaf, NavalCombatRow, SunkShipEventExact, SunkShipTextUnavailable,
    ViewStatus, WarObjectRow,
    WarRelationRow, WorldFrame,
};
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
    /// Terminal lifecycle rows are a separate keyed collection and are never
    /// passed through the active-combat panel renderer.
    pub ended_land: BTreeMap<(i64, i64), EndedCombatRow>,
    pub ended_land_status: ViewStatus,
    pub naval: Vec<NavalCombatRow>,
    pub air_losses: Leaf<Vec<AirCombatLossRow>>,
    pub air_region_combat: Leaf<Vec<AirRegionCombatRow>>,
    pub sunk_convoys: Leaf<Vec<ConvoyLossRow>>,
    /// Exact, producer-ordered terminal convoy attacks from d90. This is not
    /// the monthly d70 aggregate and is never populated from that aggregate.
    pub convoy_terminal: Leaf<Vec<ConvoyTransportAttackExactRow>>,
    pub sunk_ships: Leaf<Vec<SunkShipEventExact>>,
    pub wars: Leaf<Vec<WarObjectRow>>,
    pub war_relations: Leaf<Vec<WarRelationRow>>,
}

impl Xui20CombatDetailLosses {
    /// True only when every stream required to render at least one XUI20
    /// surface is positively measured. Missing optional rows remain empty but
    /// unavailable producer leaves remain distinguishable.
    pub fn has_any_measured_surface(&self) -> bool {
        !self.land.is_empty()
            || !self.ended_land.is_empty()
            || !self.naval.is_empty()
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
            || row.owner_tag_db_id > 2_048
            || row.killer_tag_db_id == 0
            || row.killer_tag_db_id > 2_048
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

fn validate_war_pairs(rows: &Leaf<Vec<WarObjectRow>>) -> Result<usize, GateFailure> {
    let rows = required(rows, "wars")?;
    for (index, row) in rows.iter().enumerate() {
        if row.key.trim().is_empty() {
            return Err(invalid(format!("wars[{index}].key is empty")));
        }
        required(&row.first, &format!("wars[{index}].first"))?;
        required(&row.second, &format!("wars[{index}].second"))?;
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

/// Exactly one side of a D86 exact-text value/reason pair is populated. D86
/// admits a record whose text will not decode rather than refusing the record,
/// so a missing name with a reason beside it is a valid row; a missing name
/// with no reason, or a present-but-empty required name, is not.
fn valid_exact_text(
    text: &Option<String>,
    unavailable: &Option<SunkShipTextUnavailable>,
    required: bool,
) -> bool {
    match (text, unavailable) {
        (Some(text), None) => {
            text.len() <= 4_096 && (!required || !text.is_empty()) && !text.as_bytes().contains(&0)
        }
        (None, Some(_)) => true,
        _ => false,
    }
}

fn validate_sunk_ships(rows: &Leaf<Vec<SunkShipEventExact>>) -> Result<usize, GateFailure> {
    let rows = required(rows, "sunk_ships")?;
    // The producer numbers this history contiguously from 2. A merely
    // increasing order accepts [2, 4], which is what a dropped sinking leaves.
    let expected_sequence =
        |index: usize| u64::try_from(index).ok().and_then(|index| index.checked_add(2));
    for (index, row) in rows.iter().enumerate() {
        let combat_join_valid = if row.combat_join_present {
            row.combat_id != 0
                && row.combat_generation != 0
                && row.victim_member_battle_slot_index <= 4_096
        } else {
            row.combat_id == 0
                && row.combat_generation == 0
                && row.victim_member_battle_slot_index == 0
        };
        if row.event_sequence <= 1
            || !combat_join_valid
            || row.victim_country_id_raw == 0
            || row.attacker_country_id_raw == 0
            || row.sink_date_raw <= 0
            || row.killed == row.assisted_kill
            || (row.cause_record_flag && row.assisted_kill)
            || row.victim_ship_reference_raw == [0, 0]
            || !valid_exact_text(&row.ship_name, &row.ship_name_unavailable, true)
            || !valid_exact_text(
                &row.ship_class_name,
                &row.ship_class_name_unavailable,
                true,
            )
            || !valid_exact_text(
                &row.attribution_source_name,
                &row.attribution_source_name_unavailable,
                false,
            )
            || !valid_exact_text(
                &row.attribution_source_class_name,
                &row.attribution_source_class_name_unavailable,
                false,
            )
            || Some(row.event_sequence) != expected_sequence(index)
        {
            return Err(invalid(format!(
                "sunk_ships[{index}] exact writer identity/content is invalid"
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

/// Exact replay admission. Each producer family is gated independently and
/// no cross-family ordering key is constructed.
pub fn admit_xui20_replay(binding: &Xui20CombatDetailLosses) -> Xui20ReplayAdmission {
    let air = validate_air(&binding.air_losses);
    let air_region = validate_air_region(&binding.air_region_combat);
    let convoys = validate_convoys(&binding.sunk_convoys);
    let convoy_terminal = validate_convoy_terminal(&binding.convoy_terminal);
    let sunk_ships = validate_sunk_ships(&binding.sunk_ships);
    let war_pairs = validate_war_pairs(&binding.wars);
    let war_overview = validate_war_overview(&binding.wars);
    let family_rows = binding.land.len()
        + binding.ended_land.len()
        + binding.naval.len()
        + binding.air_losses.measured().map_or(0, Vec::len)
        + binding.air_region_combat.measured().map_or(0, Vec::len)
        + binding.sunk_convoys.measured().map_or(0, Vec::len)
        + binding.sunk_ships.measured().map_or(0, Vec::len)
        + binding.wars.measured().map_or(0, Vec::len)
        + binding.war_relations.measured().map_or(0, Vec::len);
    let land_active = state(binding.land.len(), validate_land_active(&binding.land));
    let land_side_detail = state(binding.land.len(), validate_land_side_detail(&binding.land));
    let land_ended_lifecycle = state(
        binding.ended_land.len(),
        validate_ended_land_surface(binding.ended_land_status, &binding.ended_land),
    );
    let naval_active = state(binding.naval.len(), validate_naval(&binding.naval));
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
        land_terminal: Xui20SurfaceState::ExternalInputRequired {
            reason: "terminal land record owning-vector join is not carried".into(),
        },
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
    }
}

fn land_panel(row: &CombatRow) -> LandCombatPanel {
    let (
        phase_event,
        detail_generation,
        detail_location_province,
        age_days,
        duration_days,
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
    Xui20CombatDetailLosses {
        land: frame.combats().iter().map(land_panel).collect(),
        ended_land: frame.ended_combats().clone(),
        ended_land_status: frame.ended_combats_status(),
        naval: naval_air.naval_combats.clone(),
        air_losses: naval_air.air_losses.clone(),
        air_region_combat: naval_air.air_region_combat.clone(),
        sunk_convoys: naval_air.sunk_convoys.clone(),
        convoy_terminal,
        sunk_ships,
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
