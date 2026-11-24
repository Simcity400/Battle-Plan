
use crate::rows::{DivisionMovementRow, RailwayGunCounterInputRow, RailwayGunEtaOperands};
use game_model::movement::army::{NormalEdgeOperands, PreparedNormalSpeed};
use game_model::movement::unit::{EtaInputs, EtaLeg, EtaSpeed, eta_remaining};

const MOVEMENT_ONE: i64 = 100_000;

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Defines {
    pub embark_distance: i64,
    pub transport_scale: i64,
    pub invasion_speed: i64,
    pub outer_scale: i64,
    pub river_crossing_two: i64,
    pub river_crossing_three: i64,
    pub organisation: Option<OrganisationDefines>,
    pub rail: Option<RailDefines>,
    pub noncore_supply: [Option<i64>; 2],
    pub combat_movement: Option<i64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RailDefines {
    pub speed: [i64; 4],
    pub maximum_level: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OrganisationDefines {
    pub threshold: i64,
    pub penalty: i64,
}

pub struct SpeedTerms<'a> {
    pub modifier: i64,
    pub edge: &'a dyn Fn(u32, u32) -> Result<EdgeTerm, String>,
    pub organisation: Option<i64>,
    pub fuel: Option<i64>,
    pub base_speed: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeTerm {
    Normal(NormalEdgeOperands),
    Scaled { speed: i64, scale: i64 },
    Fixed(i64),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommonModifiers {
    pub combat_base: Option<i64>,
    pub country_speed: i64,
    pub leader_speed: i64,
    pub armour_speed: [i64; 2],
    pub country_attack_speed: i64,
    pub state_speed: i64,
    pub state_attack_speed: i64,
    pub unit_speed: i64,
    pub noncore: i64,
    pub weather: [i64; 4],
}

pub fn combat_base(entry: Option<i64>, combat_movement: i64) -> i64 {
    entry.map_or(0, |modifier| {
        movement_product(combat_movement, MOVEMENT_ONE).wrapping_add(modifier)
    })
}

pub fn common_modifier(
    modifiers: &CommonModifiers,
    attacking: bool,
    acclimatization: i64,
    supply: i64,
    air: i64,
    retreat: i64,
) -> i64 {
    let mut value = modifiers
        .combat_base
        .unwrap_or(MOVEMENT_ONE)
        .wrapping_add(modifiers.country_speed)
        .wrapping_add(modifiers.leader_speed)
        .wrapping_add(modifiers.armour_speed[0])
        .wrapping_add(modifiers.armour_speed[1])
        .wrapping_add(modifiers.state_speed)
        .wrapping_add(modifiers.unit_speed)
        .wrapping_add(modifiers.noncore);
    if attacking {
        value = value
            .wrapping_add(modifiers.country_attack_speed)
            .wrapping_add(modifiers.state_attack_speed);
    }
    for (index, entry) in modifiers.weather.into_iter().enumerate() {
        if index % 2 == 0 || attacking {
            value = value.wrapping_add(movement_product(acclimatization, entry));
        }
    }
    value
        .wrapping_add(supply)
        .wrapping_add(air)
        .wrapping_add(retreat)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AirOperands {
    pub friendly: i64,
    pub enemy: i64,
    pub impact: i64,
    pub scale: i64,
    pub defense: i64,
}

pub fn air_term(operands: Option<&AirOperands>) -> Result<i64, String> {
    let Some(air) = operands.filter(|air| air.enemy > 0) else {
        return Ok(0);
    };
    let total = air.friendly.wrapping_add(air.enemy);
    let fraction = if total > 0 {
        movement_fraction(air.enemy, total)?
    } else {
        0
    };
    Ok(
        movement_product(movement_product(air.impact, fraction), air.scale)
            .wrapping_add(air.defense)
            .min(0),
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SupplyOperands {
    pub grace_maximum: i64,
    pub grace_remaining: i64,
    pub ratio: i64,
    pub days: i32,
    pub supply_days: i32,
    pub supply_grace: i32,
    pub penalty: i64,
}

pub struct SupplyFactors<'a> {
    pub country: i64,
    pub leader: i64,
    pub special_forces: [i64; 2],
    pub share: &'a dyn Fn() -> Result<i64, String>,
}

pub fn supply_severity(
    operands: &SupplyOperands,
    factors: &SupplyFactors<'_>,
) -> Result<i64, String> {
    severity(available(operands)?, operands, factors)
}

pub fn attrition_supply_severity(
    operands: &SupplyOperands,
    threshold: i64,
    factors: &SupplyFactors<'_>,
) -> Result<i64, String> {
    let normalized = if threshold == 0 {
        MOVEMENT_ONE
    } else {
        movement_fraction(available(operands)?, threshold)?
    }
    .min(MOVEMENT_ONE);
    severity(normalized, operands, factors)
}

fn available(operands: &SupplyOperands) -> Result<i64, String> {
    if operands.grace_maximum < 1 {
        return Ok(0);
    }
    let hours = i64::from(operands.supply_grace).wrapping_mul(MOVEMENT_ONE);
    Ok(movement_fraction(operands.grace_remaining, hours)?
        .max(operands.ratio)
        .clamp(0, MOVEMENT_ONE))
}

fn severity(
    available: i64,
    operands: &SupplyOperands,
    factors: &SupplyFactors<'_>,
) -> Result<i64, String> {
    let days = i64::from(operands.days).wrapping_mul(MOVEMENT_ONE);
    let limit = i64::from(operands.supply_days).wrapping_mul(MOVEMENT_ONE);
    let severity = movement_product(
        MOVEMENT_ONE.wrapping_sub(available).clamp(0, MOVEMENT_ONE),
        movement_fraction(days, limit)?,
    );
    if severity == 0 {
        return Ok(0);
    }
    let mut multiplier = MOVEMENT_ONE
        .wrapping_add(factors.country)
        .wrapping_add(factors.leader);
    if factors.special_forces != [0, 0] {
        let share = (factors.share)()?;
        if share > 0 {
            for factor in factors.special_forces {
                multiplier = multiplier.wrapping_add(movement_product(share, factor));
            }
        }
    }
    Ok(movement_product(multiplier.max(0), severity))
}

pub fn supply_term(
    exiled: bool,
    operands: &SupplyOperands,
    factors: &SupplyFactors<'_>,
) -> Result<i64, String> {
    if exiled {
        return Ok(0);
    }
    Ok(movement_product(
        operands.penalty,
        supply_severity(operands, factors)?,
    ))
}

pub fn retreat_term(
    retreating: bool,
    withdrawing: bool,
    retreat_speed: i64,
    country_retreat_speed: i64,
    withdrawal_speed: i64,
) -> i64 {
    if retreating {
        retreat_speed.wrapping_add(country_retreat_speed)
    } else if withdrawing {
        withdrawal_speed
    } else {
        0
    }
}

pub fn fuel_factor(
    fuel: i64,
    fuel_rate: i64,
    capacity_scale: i64,
    floor: i64,
) -> Result<Option<i64>, String> {
    let capacity = movement_product(fuel_rate, capacity_scale);
    if capacity <= 0 || exceptional(fuel) {
        return Ok(None);
    }
    let ratio = movement_fraction(fuel, capacity)?.clamp(0, MOVEMENT_ONE);
    Ok((ratio < MOVEMENT_ONE)
        .then(|| movement_product(MOVEMENT_ONE.wrapping_sub(floor), ratio).wrapping_add(floor)))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EdgeTerms {
    pub terrain: i64,
    pub fort: Option<i64>,
    pub snow: Option<i64>,
    pub leader_terrain: i64,
    pub infrastructure: i64,
    pub river: Option<(i64, i64, i64)>,
}

pub fn edge_operands(terms: &EdgeTerms) -> NormalEdgeOperands {
    NormalEdgeOperands {
        additive: terms
            .terrain
            .wrapping_add(terms.fort.unwrap_or(0))
            .wrapping_add(terms.snow.unwrap_or(0))
            .wrapping_add(terms.leader_terrain)
            .wrapping_add(terms.infrastructure),
        river: terms.river,
    }
}

fn exceptional(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
}

fn movement_fraction(numerator: i64, denominator: i64) -> Result<i64, String> {
    if denominator == 0
        || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
    {
        return Ok(0xffff_ffff);
    }
    numerator
        .wrapping_mul(MOVEMENT_ONE)
        .checked_div(denominator)
        .ok_or_else(|| "a movement fraction overflows the native division".to_owned())
}

fn movement_product(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / MOVEMENT_ONE
}

pub fn organisation_factor(
    operands: Result<(i64, i64), String>,
    attacking: impl FnOnce() -> Result<bool, String>,
    defines: &Defines,
) -> Result<Option<i64>, String> {
    let (Some(organisation_defines), Ok((organisation, maximum))) =
        (defines.organisation, &operands)
    else {
        if !attacking()? {
            return Ok(None);
        }
        return Err(match operands {
            Err(reason) => reason,
            Ok(_) => "the content's organisation speed defines failed to load".into(),
        });
    };
    let share =
        if (*organisation as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
            MOVEMENT_ONE
        } else {
            movement_fraction(*organisation, *maximum)?.clamp(0, MOVEMENT_ONE)
        };
    if share >= organisation_defines.threshold || !attacking()? {
        return Ok(None);
    }
    let ratio = movement_fraction(share, organisation_defines.threshold)?;
    Ok(Some(MOVEMENT_ONE.wrapping_add(movement_product(
        MOVEMENT_ONE.wrapping_sub(ratio),
        organisation_defines.penalty,
    ))))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProvinceOperands {
    pub terrain_flags: u8,
    pub terrain_index: i32,
    pub fort_positive: bool,
    pub state: u32,
}

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

pub fn railway_gun_first_edge(
    gun: &RailwayGunCounterInputRow,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Option<i64> {
    railway_gun_first_edge_of(
        gun.gun,
        gun.province,
        gun.ordinary_path.first().copied(),
        || gun.eta.require("RailwayGunCounterInputs.eta").ok().copied().flatten(),
        province,
        distance,
        defines,
    )
}

pub fn railway_gun_first_edge_of(
    gun: u64,
    at: u32,
    first: Option<u32>,
    eta: impl FnOnce() -> Option<RailwayGunEtaOperands>,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    defines: &Defines,
) -> Option<i64> {
    let Some(first) = first else {
        return Some(0);
    };
    if at == 0 || first == 0 {
        return None;
    }
    let operands = eta()?;
    let leg = EtaLeg {
        distance: if first == at {
            0
        } else {
            edge_distance(13, at, first, &province, &distance, defines)?
        },
        speed: EtaSpeed::Fixed(operands.current_speed),
    };
    let legs = if first == at {
        &[][..]
    } else {
        std::slice::from_ref(&leg)
    };
    let input = EtaInputs {
        division: gun,
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

pub fn edge_speed(
    base_speed: i64,
    from: u32,
    to: u32,
    terms: &SpeedTerms<'_>,
    defines: &Defines,
) -> Result<i64, String> {
    edge_speed_with(|| Ok(base_speed), from, to, terms, defines)
}

pub fn edge_speed_with(
    base_speed: impl FnOnce() -> Result<i64, String>,
    from: u32,
    to: u32,
    terms: &SpeedTerms<'_>,
    defines: &Defines,
) -> Result<i64, String> {
    Ok(match (terms.edge)(from, to)? {
        EdgeTerm::Normal(edge) => PreparedNormalSpeed {
            modifier: terms.modifier,
            base_speed: base_speed()?,
            organisation: terms.organisation,
            fuel: terms.fuel,
            outer_scale: defines.outer_scale,
        }
        .evaluate(edge),
        EdgeTerm::Scaled { speed, scale } => movement_product(speed, scale),
        EdgeTerm::Fixed(speed) => speed,
    })
}

pub fn estimate(
    movement: &DivisionMovementRow,
    location: u32,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    terms: &SpeedTerms<'_>,
    defines: &Defines,
) -> Result<(i64, i64), String> {
    if location == 0 {
        return Err("the division's location is not recorded".into());
    }
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
        terms,
        defines,
    )
}

pub fn estimate_route(
    movement: &DivisionMovementRow,
    location: u32,
    route: &[u32],
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    terms: &SpeedTerms<'_>,
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
        movement, location, route, numerator, province, distance, terms, defines,
    )
    .map(|(_, destination)| destination)
}

pub fn base_speed(terms: &SpeedTerms<'_>) -> Result<i64, String> {
    terms
        .base_speed
        .ok_or_else(|| "the division's base speed is not recorded".into())
}

fn walk(
    movement: &DivisionMovementRow,
    location: u32,
    route: &[u32],
    numerator: i64,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    distance: impl Fn(u32, u32) -> Option<i64>,
    terms: &SpeedTerms<'_>,
    defines: &Defines,
) -> Result<(i64, i64), String> {
    let path_total =
        u32::try_from(route.len()).map_err(|_| "the route exceeds the native count".to_owned())?;
    let speed =
        |from: u32, to: u32| edge_speed_with(|| base_speed(terms), from, to, terms, defines);
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
                speed: EtaSpeed::Fixed(speed(from, to)?),
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
        current_speed: EtaSpeed::Fixed(speed(location, route[0])?),
        normal: None,
    };
    eta_remaining(&input, &legs).map_err(str::to_owned)
}

pub mod frame_terms;

#[cfg(test)]
mod tests;
