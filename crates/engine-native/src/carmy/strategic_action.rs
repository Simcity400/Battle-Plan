//! Shared strategic-action reader for glyphs and ETA rail inputs.

use super::*;

/// Effective operands; nested absence means the native read was skipped.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct StrategicRedeploymentInputs {
    pub country_modifier: i64,
    pub state_modifier: Option<i64>,
    pub friendly_state_modifier: Option<i64>,
}

impl StrategicRedeploymentInputs {
    pub fn blocked(self) -> bool {
        self.country_modifier != 0
            || self.state_modifier.is_some_and(|value| value != 0)
            || self.friendly_state_modifier.is_some_and(|value| value != 0)
    }
}

/// The failed native query, independently of the movement row's availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum StrategicActionFailure {
    Owner,
    CountryModifier,
    CurrentState,
    StateModifier,
    Friendly,
    FriendlyStateModifier,
}

impl StrategicActionFailure {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Owner => "strategic action owner-country input unavailable",
            Self::CountryModifier => "strategic action owner-country modifier unavailable",
            Self::CurrentState => "strategic action current-state input unavailable",
            Self::StateModifier => "strategic action roster-state modifier unavailable",
            Self::Friendly => "strategic action friendly-controller input unavailable",
            Self::FriendlyStateModifier => "strategic action friendly-state modifier unavailable",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum StrategicAction {
    #[default]
    Unreached,
    Available(StrategicRedeploymentInputs),
    Withheld(StrategicActionFailure),
}

/// Action-code selection reaches this reader only for class zero.
#[allow(clippy::too_many_arguments)]
pub(super) fn capture(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    unit: usize,
    path_total: u32,
    target: usize,
    retreat: u8,
    strategic: u8,
    seed: Option<&DivisionSeed>,
) -> StrategicAction {
    if (path_total as i32) <= 0 || target != 0 || retreat != 0 || strategic == 0 {
        return StrategicAction::Unreached;
    }
    let acquire = || {
        let expeditionary = field(
            memory,
            unit,
            cunit::EXPEDITIONARY_OWNER,
            seed.map(|s| s.expeditionary_owner),
        )
        .map_err(|_| StrategicActionFailure::Owner)?;
        let roster = || {
            field(
                memory,
                unit,
                cunit::ROSTER_OWNER,
                seed.map(|s| s.roster_owner),
            )
        };
        let owner_raw = if (expeditionary as i32) > 0 {
            expeditionary
        } else {
            roster().map_err(|_| StrategicActionFailure::Owner)?
        };
        let owner = ccountry::Reader::new(memory, base, game_state)
            .and_then(|reader| reader.resolve(owner_raw))
            .map_err(|_| StrategicActionFailure::Owner)?;
        read_inputs(
            memory,
            base,
            game_state,
            owner_raw,
            owner.pointer,
            roster,
            || {
                let province = field::<u64>(
                    memory,
                    unit,
                    cunit::LOCATION,
                    seed.map(|s| s.location.map(|p| p as u64)),
                )? as usize;
                authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
                field::<u64>(memory, province, cprovince::STATE, None).map(|p| p as usize)
            },
        )
    };
    match acquire() {
        Ok(inputs) => StrategicAction::Available(inputs),
        Err(reason) => StrategicAction::Withheld(reason),
    }
}

/// Read country, roster state, then friendly roster state. Closures preserve
/// the caller's acquired roots and skipped reads.
#[allow(clippy::too_many_arguments)]
pub fn read_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    owner_raw: u32,
    owner: usize,
    roster: impl FnOnce() -> Result<u32, &'static str>,
    current_state: impl FnOnce() -> Result<usize, &'static str>,
) -> Result<StrategicRedeploymentInputs, StrategicActionFailure> {
    let country_modifier = movement_address(owner, ccountry::MODIFIER_ACCUMULATOR)
        .and_then(|at| {
            crate::modifier_accumulator::requested_effective_at(
                memory,
                base,
                at,
                keys::DISABLE_STRATEGIC_REDEPLOYMENT,
            )
        })
        .map_err(|_| StrategicActionFailure::CountryModifier)?;
    let mut inputs = StrategicRedeploymentInputs {
        country_modifier,
        ..Default::default()
    };
    if country_modifier != 0 {
        return Ok(inputs);
    }
    let state = current_state().map_err(|_| StrategicActionFailure::CurrentState)?;
    if state == 0 {
        return Ok(inputs);
    }
    let roster = roster().map_err(|_| StrategicActionFailure::StateModifier)?;
    let query = |key| {
        authenticate(memory, base, state, crate::cstate::VTABLE_RVA)?;
        let accumulator =
            crate::cstate::country_modifier_accumulator(memory, game_state, state, roster)?;
        crate::modifier_accumulator::requested_effective_at(memory, base, accumulator, key)
    };
    let state_modifier = query(keys::DISABLE_STRATEGIC_REDEPLOYMENT)
        .map_err(|_| StrategicActionFailure::StateModifier)?;
    inputs.state_modifier = Some(state_modifier);
    if state_modifier != 0 {
        return Ok(inputs);
    }
    let friendly = field(memory, state, crate::cstate::CONTROLLER, None)
        .and_then(|controller| {
            ccountry::movement_friendly(memory, base, game_state, owner_raw, controller)
        })
        .map_err(|_| StrategicActionFailure::Friendly)?;
    if friendly {
        inputs.friendly_state_modifier = Some(
            query(keys::DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER)
                .map_err(|_| StrategicActionFailure::FriendlyStateModifier)?,
        );
    }
    Ok(inputs)
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    seeded: Option<Option<T>>,
) -> Result<T, &'static str> {
    match seeded {
        Some(value) => value,
        None => pointer.checked_add(offset).and_then(|p| read(memory, p)),
    }
    .ok_or("strategic action operand unreadable")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    vtable: usize,
) -> Result<(), &'static str> {
    if pointer != 0 && base.checked_add(vtable).map(|p| p as u64) == read(memory, pointer) {
        Ok(())
    } else {
        Err("strategic action object identity mismatch")
    }
}

#[cfg(test)]
mod tests;
