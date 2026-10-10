//! The ETA walk, replayed from recorded operands with no memory access.
//!
//! Per-pair operands are leg-aligned vectors on the `DivisionEtaInputs` row.
//! The duration and speed evaluators are the engine's own pure functions.
//! The recording stores these operands, not a second independently acquired
//! duration, so replay alone cannot certify capture equivalence to the game.

use crate::rows::{DivisionEtaInputsRow, DivisionMovementRow, RailwayGunCounterInputRow};
use engine::carmy::{MovementSpeedInputs, NormalEdgeOperands, PreparedNormalSpeed};
use engine::cunit::{EtaInputs, EtaLeg, EtaSpeed, eta_remaining};

/// The progress divisor comes from the first route edge. The same
/// edge-distance rule drives this and the ETA walk: a sea-to-land embark
/// override, the first directed adjacency, or 100000 when matching content
/// confirms no adjacency exists. Refuse missing operands rather than
/// interpreting a later leg as the first edge; the ETA walk skips self edges.
pub fn movement_progress(
    movement: &DivisionMovementRow,
    inputs: Option<&DivisionEtaInputsRow>,
    location: Option<u32>,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Option<i64> {
    if movement.path_total == 0 {
        return movement.path.is_empty().then_some(0);
    }
    let first = *movement.path.first()?;
    let inputs = inputs.filter(|row| row.division == movement.division)?;
    unit_progress(
        inputs.class,
        movement.numerator,
        location?,
        first,
        province,
        distance,
        defines,
    )
}

/// The same first-edge fraction for any map unit with a raw movement numerator.
/// Class-13 railway guns use this with their own ordinary route and class, while
/// their visual route can independently choose the alternate path.
pub fn unit_progress(
    class: u32,
    numerator: i64,
    from: u32,
    first: u32,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Option<i64> {
    if from == 0 || first == 0 {
        return None;
    }
    let denominator = edge_distance(class, from, first, &province, &distance, defines)?;
    Some(native_progress_fraction(numerator, denominator))
}

/// Fixed-point arithmetic shared by division and task-force map movement,
/// including signed wrap and the zero-divisor/infinity sentinel. This is
/// display logic over recorded simulation operands.
pub fn native_progress_fraction(numerator: i64, denominator: i64) -> i64 {
    if denominator == 0
        || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
    {
        return 0xffff_ffff;
    }
    numerator
        .wrapping_mul(100_000)
        .checked_div(denominator)
        .unwrap_or(0xffff_ffff)
}

/// Authored movement parameters loaded from the replay's matching content.
/// These are pure operands loaded from matching authored content, never a
/// recorded table.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Defines {
    pub embark_distance: i64,
    pub transport_scale: i64,
    pub invasion_speed: i64,
    pub outer_scale: i64,
    pub river_crossing_two: i64,
    pub river_crossing_three: i64,
}

/// The destination-side province operands `normal_movement_edge` reads.
/// The river relation term is recorded per leg, so the destination
/// controller is not needed here.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProvinceOperands {
    /// Raw province_definition +0xd2; selects the transport branch.
    pub terrain_flags: u8,
    /// One-based terrain index (an i32); finds its `terrain_speed`.
    pub terrain_index: i32,
    pub fort_positive: bool,
    /// Owning state id, or zero for a province with no state.
    pub state: u32,
}

/// Class-0 divisions and class-13 railway guns use the authored embark cost
/// on a sea-to-land edge. Otherwise the first directed province adjacency
/// supplies its movement distance. The caller returns `Some(100000)` for a
/// known no-adjacency edge and `None` when matching geometry is unavailable.
fn edge_distance(
    class: u32,
    from: u32,
    to: u32,
    province: &impl Fn(u32) -> Option<ProvinceOperands>,
    distance: &impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Option<i64> {
    let source = province(from)?;
    let target = province(to)?;
    if matches!(class, 0 | 13) && source.terrain_flags & 3 == 0 && target.terrain_flags & 1 != 0 {
        Some(defines.embark_distance)
    } else {
        distance(from, to)
    }
}

/// An ordinary railway-gun edge uses its captured speed as the progress
/// divisor. Distance and embark cost come from matching content, as for divisions.
pub fn railway_gun_first_edge(
    gun: &RailwayGunCounterInputRow,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Option<i64> {
    let Some(&first) = gun.ordinary_path.first() else {
        return Some(0);
    };
    if gun.province == 0 || first == 0 {
        return None;
    }
    let operands = gun
        .eta
        .require("RailwayGunCounterInputs.eta")
        .ok()?
        .as_ref()?;
    let leg = EtaLeg {
        distance: if first == gun.province {
            0
        } else {
            edge_distance(13, gun.province, first, &province, &distance, defines)?
        },
        speed: EtaSpeed::Fixed(operands.current_speed),
    };
    let legs = if first == gun.province {
        &[][..]
    } else {
        std::slice::from_ref(&leg)
    };
    let input = EtaInputs {
        division: gun.gun,
        next: first,
        destination: first,
        path_total: 1,
        leg_len: legs.len(),
        next_leg_count: legs.len(),
        numerator: operands.numerator,
        current_speed: EtaSpeed::Fixed(operands.current_speed),
        ..Default::default()
    };
    eta_remaining(&input, legs).ok().map(|pair| pair.0)
}

/// Recompute `(next_remaining, destination_remaining)` for one moving division.
///
/// `location` is `Divisions.province`, the walk's start. `province` and
/// `state_infra` resolve this same hour's rows; a missing one is a refusal.
/// The caller validates the weather census before `snow` follows the native
/// predicate, where a province without a weather record has no snow. `None`
/// means the row cannot be derived exactly.
///
/// `distance` reads matching authored geometry. A known missing adjacency
/// returns the native 100000 fallback; unavailable geometry and missing
/// province operands refuse the row because the edge cost cannot be decided.
/// `crossing_kind` resolves the matching directed edge for a normal-speed
/// leg; a known clear/no-edge route returns zero, while unavailable geometry
/// returns `None` and refuses derivation.
#[allow(clippy::too_many_arguments)]
pub fn derive(
    inputs: &DivisionEtaInputsRow,
    movement: &DivisionMovementRow,
    location: u32,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    state_infra: impl Fn(u32) -> Option<i64>,
    snow: impl Fn(u32) -> bool,
    distance: impl Fn(u32, u32) -> Option<i64>,
    crossing_kind: impl Fn(u32, u32) -> Option<u8>,
    defines: &Defines,
) -> Option<(i64, i64)> {
    if location == 0 {
        return None;
    }
    // A route the recorder's arena truncated cannot be walked.
    if movement.path.len() != movement.path_total as usize {
        return None;
    }
    if movement.path.is_empty() {
        return Some((0, 0)); // cunit.rs:889-891
    }
    let mut legs: Vec<EtaLeg> = Vec::with_capacity(movement.path.len());
    let mut next_leg_count = 0;
    let mut from = location;
    for (index, &to) in movement.path.iter().enumerate() {
        if to == 0 {
            return None; // cunit.rs:632-635
        }
        if from != to {
            // Self edges emit no leg, so the leg vectors advance only here.
            let leg = legs.len();
            let length = edge_distance(inputs.class, from, to, &province, &distance, defines)?;
            let speed = edge_speed(
                inputs,
                movement,
                defines,
                &province,
                &state_infra,
                &snow,
                &crossing_kind,
                location,
                from,
                to,
                leg,
            )?;
            legs.push(EtaLeg {
                distance: length,
                speed: EtaSpeed::Fixed(speed),
            });
            if index == 0 {
                next_leg_count = 1; // cunit.rs:659-661
            }
            // `is_zero` only short-circuits a proof that the speed is positive
            // (carmy.rs:1600-1610), so the evaluated value decides identically.
            if speed == 0 {
                break; // cunit.rs:663-666
            }
        }
        from = to;
    }
    let input = EtaInputs {
        division: inputs.division,
        next: movement.path[0],
        destination: movement.path[movement.path.len() - 1],
        path_total: movement.path_total,
        leg_start: 0,
        leg_len: legs.len(),
        next_leg_count,
        numerator: movement.numerator,
        // Recorded directly, including the native null-destination branch
        // this call alone can take.
        current_speed: EtaSpeed::Fixed(inputs.current_speed),
        normal: None,
    };
    eta_remaining(&input, &legs).ok()
}

/// One leg's speed, in the native branch order of
/// `movement_speed_inputs_internal` (carmy.rs:2395-2504).
#[allow(clippy::too_many_arguments)]
fn edge_speed(
    inputs: &DivisionEtaInputsRow,
    movement: &DivisionMovementRow,
    defines: &Defines,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    state_infra: impl Fn(u32) -> Option<i64>,
    snow: impl Fn(u32) -> bool,
    crossing_kind: impl Fn(u32, u32) -> Option<u8>,
    location: u32,
    from: u32,
    to: u32,
    leg: usize,
) -> Option<i64> {
    // A branch vector may be empty only when no leg took it. An evaluated
    // branch requires its recorded operand, including an explicitly recorded zero.
    let recorded = |values: &[i64]| values.get(leg).copied();
    let source = province(from)?;
    let target = province(to)?;
    if movement.naval_transfer != 0
        || source.terrain_flags & 1 == 0
        || target.terrain_flags & 1 == 0
    {
        return Some(
            MovementSpeedInputs::Scaled {
                speed: recorded(&inputs.leg_transport_speed)?,
                scale: defines.transport_scale,
            }
            .evaluate(),
        );
    }
    if invasion_branch(inputs.class, movement, location, &province)? {
        return Some(defines.invasion_speed);
    }
    if movement.strategic_redeployment != 0 {
        // Rail movement is blocked when the country 0x1ea, current-state 0x1ea,
        // or friendly current-state 0x1eb effective modifier is nonzero. These are
        // shared simulation operands; the collector's branch result is not a replay field.
        let country = inputs.rail_country_modifier?;
        let blocked = country != 0
            || inputs.rail_state_modifier.is_some_and(|value| value != 0)
            || inputs
                .rail_friendly_state_modifier
                .is_some_and(|value| value != 0);
        if !blocked {
            return Some(
                MovementSpeedInputs::Scaled {
                    speed: recorded(&inputs.leg_rail_speed)?,
                    scale: defines.outer_scale,
                }
                .evaluate(),
            );
        }
    }
    // normal_movement_edge, carmy.rs:3030-3147. The sub-unit and leader terrain
    // terms are already composed into terrain_speed.
    let mut additive = inputs
        .terrain_speed
        .iter()
        .find(|entry| entry.terrain == target.terrain_index)?
        .modifier;
    if target.fort_positive {
        additive = additive.wrapping_add(inputs.fort_penalty);
    }
    if snow(to) {
        additive = additive.wrapping_add(inputs.snow_penalty);
    }
    if target.state != 0 {
        additive = additive.wrapping_add(state_infra(target.state)?);
    }
    let crossing = crossing_kind(from, to)?;
    let river = if (1..=3).contains(&crossing) {
        Some((
            recorded(&inputs.leg_river_combined)?.wrapping_add(inputs.river_leader),
            inputs.river_adjustment,
            match crossing {
                2 => defines.river_crossing_two,
                3 => defines.river_crossing_three,
                _ => 0,
            },
        ))
    } else {
        None
    };
    Some(
        PreparedNormalSpeed {
            modifier: inputs.speed_modifier,
            base_speed: inputs.speed_base,
            organisation: inputs.speed_org,
            fuel: inputs.speed_fuel,
            outer_scale: defines.outer_scale,
        }
        .evaluate(NormalEdgeOperands { additive, river }),
    )
}

/// `movement_land_inputs` (carmy.rs) tests the division's original province
/// and first stored route province, even while evaluating a later land leg.
/// Transport wins before this predicate, so the first sea-to-land leg itself
/// uses transport speed. The terrain flags are reconstructed from matching
/// content; the collector does not serialize its computed branch result.
fn invasion_branch(
    class: u32,
    movement: &DivisionMovementRow,
    location: u32,
    province: &impl Fn(u32) -> Option<ProvinceOperands>,
) -> Option<bool> {
    if movement.path_total == 0 || class != 0 {
        return Some(false);
    }
    let first = *movement.path.first()?;
    if first == 0 {
        return Some(false);
    }
    let source = province(location)?;
    if source.terrain_flags & 3 != 0 {
        return Some(false);
    }
    Some(province(first)?.terrain_flags & 1 != 0)
}

#[cfg(test)]
mod tests;
