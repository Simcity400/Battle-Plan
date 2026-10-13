//! Movement timing replayed from recorded operands with no memory access.
//!
//! Progress follows the native first-edge fraction. Arrival times are estimates: the
//! native walk over each division's recorded route with every edge at one speed, the
//! native land speed of its recorded stat-block base speed ([`estimated_speed`]). The
//! game evaluates each edge with its own terrain, infrastructure, weather, river, supply,
//! air, organisation and fuel terms, which the recording does not hold, so an estimate
//! drifts from the game wherever those terms apply.

use crate::rows::{DivisionMovementRow, RailwayGunCounterInputRow, Recorded};
use engine::cunit::{EtaInputs, EtaLeg, EtaSpeed, eta_remaining};

const MOVEMENT_ONE: i64 = 100_000;

/// Movement progress for a division (class 0): its numerator over the first route edge.
/// Zero for an empty route; None where the route, location or matching content cannot
/// decide the edge.
pub fn division_progress(
    movement: &DivisionMovementRow,
    location: Option<u32>,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Option<i64> {
    if movement.path_total == 0 {
        return movement.path.is_empty().then_some(0);
    }
    let first = *movement.path.first()?;
    unit_progress(
        0,
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

/// The speed an estimate assumes on every edge: the land tail of Movement-speed
/// calculation applied to the stat-block base speed alone, floored at 1 km/h (100000) and
/// then scaled by the content's movement scale with the native wrapping product. With no
/// destination and full organisation and fuel the game computes the same value; otherwise
/// the unrecorded edge modifier, organisation and fuel factors make it differ.
pub fn estimated_speed(base_speed: i64, defines: &Defines) -> i64 {
    base_speed
        .max(MOVEMENT_ONE)
        .wrapping_mul(defines.outer_scale)
        / MOVEMENT_ONE
}

/// Estimated `(next_remaining, destination_remaining)` for one division, in
/// native signed fixed5 hours: [`eta_remaining`] over the recorded route from
/// `location` (`Divisions.province`), every edge and the first-edge progress
/// at [`estimated_speed`]. Each edge distance follows the native rule for
/// class 0 over matching content ([`edge_distance`]); self edges emit no leg.
/// The caller rounds hours as the game does ([`engine::cunit::eta_hours`]).
pub fn estimate(
    movement: &DivisionMovementRow,
    location: u32,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Result<(i64, i64), String> {
    if location == 0 {
        return Err("the division's location is not recorded".into());
    }
    // A route the recorder's arena truncated cannot be measured.
    if movement.path.len() != movement.path_total as usize {
        return Err("the division's recorded route is truncated".into());
    }
    if movement.path.is_empty() {
        return Ok((0, 0));
    }
    walk(
        movement,
        location,
        &movement.path,
        movement.numerator,
        province,
        distance,
        defines,
    )
}

/// The estimated duration of a prospective route (`route` excludes the division's
/// location and ends at its target), as the counter preview words it: every edge at
/// [`estimated_speed`], and the division's first-edge progress subtracted only when the
/// route starts with the division's own recorded first edge. Zero for an empty route,
/// which needs no speed.
pub fn estimate_route(
    movement: &DivisionMovementRow,
    location: u32,
    route: &[u32],
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Result<i64, String> {
    if location == 0 {
        return Err("the division's location is not recorded".into());
    }
    if route.is_empty() {
        return Ok(0);
    }
    let numerator = if route.first() == movement.path.first() {
        movement.numerator
    } else {
        0
    };
    walk(
        movement, location, route, numerator, province, distance, defines,
    )
    .map(|(_, destination)| destination)
}

/// [`eta_remaining`] over `route` from `location` at the division's
/// estimated speed, subtracting `numerator` of progress.
fn walk(
    movement: &DivisionMovementRow,
    location: u32,
    route: &[u32],
    numerator: i64,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Result<(i64, i64), String> {
    let base_speed = match movement.base_speed {
        Recorded::NotRecorded => {
            return Err("DivisionMovement.base_speed is not recorded in this format".into());
        }
        Recorded::Value(None) => return Err("the division's base speed was refused".into()),
        Recorded::Value(Some(speed)) => speed,
    };
    let path_total =
        u32::try_from(route.len()).map_err(|_| "the route exceeds the native count".to_owned())?;
    let speed = estimated_speed(base_speed, defines);
    let mut legs = Vec::with_capacity(route.len());
    let mut next_leg_count = 0;
    let mut from = location;
    for (index, &to) in route.iter().enumerate() {
        if to == 0 {
            return Err("the division's route holds a null province".into());
        }
        if from != to {
            let length = edge_distance(0, from, to, &province, &distance, defines)
                .ok_or_else(|| format!("route edge {from}->{to} has no content geometry"))?;
            legs.push(EtaLeg {
                distance: length,
                speed: EtaSpeed::Fixed(speed),
            });
            if index == 0 {
                next_leg_count = 1;
            }
        }
        from = to;
    }
    let input = EtaInputs {
        division: movement.division,
        next: route[0],
        destination: route[route.len() - 1],
        path_total,
        leg_start: 0,
        leg_len: legs.len(),
        next_leg_count,
        numerator,
        current_speed: EtaSpeed::Fixed(speed),
        normal: None,
    };
    eta_remaining(&input, &legs).map_err(str::to_owned)
}

#[cfg(test)]
mod tests;
