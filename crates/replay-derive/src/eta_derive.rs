//! Movement timing replayed from recorded operands with no memory access.
//!
//! Progress follows the native first-edge fraction. Arrival times walk each division's
//! recorded route natively, each edge at the native land speed ([`edge_speed`]) of its
//! recorded stat-block base speed and the speed terms the hour records ([`SpeedTerms`]):
//! the common modifier ([`common_modifier`]), whose one a land combat's movement base
//! replaces ([`combat_base`]), of country, commander, armour, location state, priority
//! sub-unit and category, weather, supply (with the template's special-forces share and
//! the noncore-supply term), enemy air ([`air_term`]) and retreat terms, each edge's
//! terrain, fort, snow, commander terrain, river (with the owner's relation's river
//! crossing factor against the destination's controller) and infrastructure terms
//! ([`edge_operands`]), the low-organisation factor of a hostile move
//! ([`organisation_factor`]) and the fuel factor ([`fuel_factor`]). A move to or from the
//! sea or in naval transfer moves at the owner's transport speed times the transport
//! scale, a strategic redeployment that no modifier blocks at the road or rail speed
//! times the movement scale, and land edges past a landing at the invasion speed
//! ([`EdgeTerm`]).

use crate::rows::{DivisionMovementRow, RailwayGunCounterInputRow, RailwayGunEtaOperands};
use game_model::movement::army::{NormalEdgeOperands, PreparedNormalSpeed};
use game_model::movement::unit::{EtaInputs, EtaLeg, EtaSpeed, eta_remaining};

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
    /// The organisation term's defines; None when the content refused them.
    pub organisation: Option<OrganisationDefines>,
    /// The strategic redeployment speed's defines; None when the content refused them.
    pub rail: Option<RailDefines>,
    /// `NMilitary.NON_CORE_SUPPLY_SPEED` and `NMilitary.NON_CORE_SUPPLY_AIR_SPEED`
    /// (fixed5): the noncore-supply term of a ground-supplied and of an air-supplied
    /// division; each None refuses only the corresponding selected form.
    pub noncore_supply: [Option<i64>; 2],
    /// `NMilitary.COMBAT_MOVEMENT_SPEED` (fixed5), the land combat movement base's
    /// define; None when the content refused it.
    pub combat_movement: Option<i64>,
}

/// `NMilitary.STRATEGIC_SPEED_INFRA_BASE`, `STRATEGIC_SPEED_INFRA_MAX`,
/// `STRATEGIC_SPEED_RAIL_BASE` and `STRATEGIC_SPEED_RAIL_MAX` (fixed5), and the railway
/// building's maximum level, the rail percentage's denominator.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RailDefines {
    pub speed: [i64; 4],
    pub maximum_level: i32,
}

/// `NMilitary.LOW_ORG_FOR_ATTACK` and `NMilitary.ZERO_ORG_MOVEMENT_MODIFIER`, fixed5.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OrganisationDefines {
    /// The organisation share below which a hostile move slows.
    pub threshold: i64,
    /// The speed term at zero organisation.
    pub penalty: i64,
}

/// The recorded operands of the speed terms an arrival time applies beyond the base
/// speed.
pub struct SpeedTerms<'a> {
    /// The division's common movement modifier ([`common_modifier`]).
    pub modifier: i64,
    /// The speed term of a move `from -> to` ([`EdgeTerm`]); an error names the missing
    /// operand.
    pub edge: &'a dyn Fn(u32, u32) -> Result<EdgeTerm, String>,
    /// The division's organisation factor ([`organisation_factor`]), None where the
    /// game applies none.
    pub organisation: Option<i64>,
    /// The division's fuel factor ([`fuel_factor`]), None where the game applies none.
    pub fuel: Option<i64>,
    /// The moving division's base speed ([`DivisionMovementRow::base_speed`]), None when the
    /// frame cannot tell it.
    pub base_speed: Option<i64>,
}

/// How the game moves a division over one edge: the normal speed with the edge's
/// operands ([`edge_operands`]); a speed times a scale, without the common modifier,
/// organisation or fuel factors (the owner's transport speed times the content's
/// transport scale, or a strategic redeployment's road or rail speed times the outer
/// movement scale); or a fixed speed (a naval invasion's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeTerm {
    Normal(NormalEdgeOperands),
    Scaled { speed: i64, scale: i64 },
    Fixed(i64),
}

/// The effective modifier values the common term adds, each zero where the game finds
/// no entry: the country's and the selected commander block's army speed, their armour
/// speed (for a division with a commander whose priority sub-unit is armour), the
/// country's attack speed (added only for a hostile move), the location state's speed
/// and attack speed terms, the priority sub-unit's and its categories' speed entries,
/// the noncore-supply term, and the location's province and region weather speed and
/// attack speed entries.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommonModifiers {
    /// The land combat movement base ([`combat_base`]) of a division in combat that is
    /// not withdrawing, which replaces the modifier's one; None for any other division.
    pub combat_base: Option<i64>,
    pub country_speed: i64,
    pub leader_speed: i64,
    /// The country's and the commander block's armour speed, each added on its own.
    pub armour_speed: [i64; 2],
    pub country_attack_speed: i64,
    /// The location state's army speed plus its controller or enemy army speed entry,
    /// selected for the division's roster owner.
    pub state_speed: i64,
    /// The location state's attack speed entry (added only for a hostile move).
    pub state_attack_speed: i64,
    /// The owner country's entries for the template's priority sub-unit and each of its
    /// categories, summed in native order with repeats.
    pub unit_speed: i64,
    /// The noncore-supply term: the content's noncore supply speed for a division whose
    /// owner lacks core supply access at its location, zero otherwise.
    pub noncore: i64,
    /// Province speed, province attack speed, region speed, region attack speed.
    pub weather: [i64; 4],
}

/// The land combat movement base that replaces the common modifier's one for a division
/// in combat that is not withdrawing: the combat movement define plus the first land
/// combat's movement modifier (`entry`), in the native fixed5 arithmetic; zero where the
/// division's first matching combat is naval or none matches (`entry` None).
pub fn combat_base(entry: Option<i64>, combat_movement: i64) -> i64 {
    entry.map_or(0, |modifier| {
        movement_product(combat_movement, MOVEMENT_ONE).wrapping_add(modifier)
    })
}

/// The division's common movement modifier: one (or the combat base where set), plus
/// the country's and commander's army speed and armour speed, the location state's
/// speed term, the priority sub-unit and category speed entries, the noncore-supply
/// term, the country's and the location state's attack speed for a hostile move
/// (`attacking`), each weather entry times the acclimatization factor (the attack speed
/// entries only for a hostile move), the out-of-supply term ([`supply_term`]), the enemy
/// air term ([`air_term`]) and the retreat or withdrawal term ([`retreat_term`]), each
/// sum the native wrapping one.
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

/// The operands of the enemy air term over a division's location: the owner's
/// regional air superiority pair and movement impact (`RaidAirScores`), the air speed
/// define and the division's anti-air reduction
/// ([`engine_native::csub_unit_definition::movement_air_defense_value`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AirOperands {
    pub friendly: i64,
    pub enemy: i64,
    pub impact: i64,
    pub scale: i64,
    pub defense: i64,
}

/// The enemy air term: the impact times the enemy's share of the pair times the air
/// speed define, plus the anti-air reduction, capped at zero. Zero without operands
/// (an owner at no war or a location outside any strategic region) or without enemy
/// superiority, where the game adds nothing, not even the reduction.
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

/// The supply operands of the out-of-supply term.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SupplyOperands {
    /// The division's maximum and remaining grace and the received supply ratio.
    pub grace_maximum: i64,
    pub grace_remaining: i64,
    pub ratio: i64,
    /// Signed days out of supply.
    pub days: i32,
    /// The supply-days and supply-grace defines (signed 32-bit globals).
    pub supply_days: i32,
    pub supply_grace: i32,
    /// The out-of-supply speed penalty at full severity.
    pub penalty: i64,
}

/// The out-of-supply severity's multiplier operands: the country's and the commander
/// block's out-of-supply factor, and the special-forces factors with the template's
/// special-forces share, which the game reads only when that share is positive.
pub struct SupplyFactors<'a> {
    pub country: i64,
    pub leader: i64,
    pub special_forces: [i64; 2],
    /// The template's special-forces share; an error names why it is unknown.
    pub share: &'a dyn Fn() -> Result<i64, String>,
}

/// The out-of-supply severity: the unsupplied share of the larger of the received
/// ratio and the remaining grace's share of `SUPPLY_GRACE` hours (zero without a
/// positive maximum grace), times the days out of supply over the supply-days define,
/// scaled by one plus the out-of-supply factors (the special-forces factors weighted by
/// the template's share) floored at zero. The movement speed multiplies it by its
/// full-severity penalty and the attrition hover by its out-of-supply attrition.
pub fn supply_severity(
    operands: &SupplyOperands,
    factors: &SupplyFactors<'_>,
) -> Result<i64, String> {
    severity(available(operands)?, operands, factors)
}

/// The attrition hover's out-of-supply severity: [`supply_severity`] with the available
/// share first normalized by the army attrition supply threshold, `min(1, available /
/// threshold)`, one for a zero threshold.
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

/// The supplied share: the larger of the received ratio and the remaining grace's share
/// of `SUPPLY_GRACE` hours, clamped to [0, 1]; zero without a positive maximum grace.
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

/// The movement speed's out-of-supply term: the severity ([`supply_severity`]) times the
/// full-severity penalty; zero for an exiled division, which bypasses supply.
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

/// The retreat or withdrawal term: a retreating division adds the retreat define and
/// the country's retreat speed factor, otherwise a withdrawing one the withdrawal define.
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

/// The fuel factor: fuel over the capacity (the stat block's fuel word times the
/// capacity scale), clamped to [0, 1], raised from the out-of-fuel floor as
/// `(1 - floor) * ratio + floor` when below one; None for a division without capacity,
/// with a sentinel fuel word or with a full tank, where the game applies none.
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

/// The edge operands of a move into a province.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EdgeTerms {
    /// The stat block's movement for the destination's terrain plus the terrain's own
    /// unit adjuster movement.
    pub terrain: i64,
    /// The stat block's fort movement where the destination's fort sum is positive.
    pub fort: Option<i64>,
    /// The stat block's snow movement where the destination has snow.
    pub snow: Option<i64>,
    /// The direct commander's movement for the destination's terrain.
    pub leader_terrain: i64,
    /// The destination state's movement infrastructure term.
    pub infrastructure: i64,
    /// For a crossing of kind 1 to 3: the summed river crossing factors (the commander
    /// block's and the owner's relation's against the destination's controller), the
    /// stat block's river movement and the crossing's define (zero for kind 1).
    pub river: Option<(i64, i64, i64)>,
}

/// [`NormalEdgeOperands`] of an edge: the terrain, fort, snow, commander terrain and
/// infrastructure terms summed with the native wrapping addition, and the river terms.
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

/// A numerator the native division does not scale: within two of either signed
/// extreme.
fn exceptional(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
}

/// The native signed fraction with its sentinel and zero-divisor result.
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

/// The movement speed's organisation factor: a division whose recorded route ends at a
/// province controlled by a country its owner is at war with (`attacking`, evaluated only
/// when the share is below the threshold) and whose organisation share of its maximum,
/// clamped to [0, 1], is below `LOW_ORG_FOR_ATTACK` moves at `1 + (1 - share /
/// threshold) * ZERO_ORG_MOVEMENT_MODIFIER` of its speed. `operands` is the division's
/// recorded organisation and maximum. None where the game applies no factor; an operand
/// refusal is reached only when the factor could apply.
pub fn organisation_factor(
    operands: Result<(i64, i64), String>,
    attacking: impl FnOnce() -> Result<bool, String>,
    defines: &Defines,
) -> Result<Option<i64>, String> {
    let (Some(organisation_defines), Ok((organisation, maximum))) =
        (defines.organisation, &operands)
    else {
        // Without the share or the threshold only a non-hostile move is decided.
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

/// The destination-side province operands `normal_movement_edge` reads.
/// The river relation term is recorded per leg, so the destination
/// controller is not needed here.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProvinceOperands {
    /// Raw province-definition terrain flags; selects the transport branch.
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

/// [`railway_gun_first_edge`] from values: the gun, where it stands, its ordinary route's first
/// province and its recorded first-edge operands.
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

/// The native land speed of the edge `from -> to`: the stat-block base speed times the
/// common modifier plus the edge's terms, times the organisation and fuel factors,
/// floored at 1 km/h (100000) and then scaled by the content's movement scale, each
/// product the native wrapping fixed5 one; a scaled edge's speed times its scale, and a
/// fixed edge's speed.
pub fn edge_speed(
    base_speed: i64,
    from: u32,
    to: u32,
    terms: &SpeedTerms<'_>,
    defines: &Defines,
) -> Result<i64, String> {
    edge_speed_with(|| Ok(base_speed), from, to, terms, defines)
}

/// [`edge_speed`] with the base speed resolved only for a normal edge, the only edge
/// that reads it.
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

/// Estimated `(next_remaining, destination_remaining)` for one division, in
/// native signed fixed5 hours: [`eta_remaining`] over the recorded route from
/// `location` (`Divisions.province`), each edge and the first-edge progress
/// at its [`edge_speed`]. Each edge distance follows the native rule
/// for class 0 over matching content ([`edge_distance`]); self edges emit no leg.
/// The caller rounds hours as the game does ([`game_model::movement::unit::eta_hours`]).
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
        terms,
        defines,
    )
}

/// The estimated duration of a prospective route (`route` excludes the division's
/// location and ends at its target), as the counter preview words it: each edge at
/// its [`edge_speed`], and the division's first-edge progress subtracted only when the
/// route starts with the division's own recorded first edge. Zero for an empty route,
/// which needs no speed. The organisation factor is the division's own, decided by its
/// recorded route, as the game decides it.
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

/// The division's base speed; refused by name when its panel row does not tell it.
pub fn base_speed(terms: &SpeedTerms<'_>) -> Result<i64, String> {
    terms
        .base_speed
        .ok_or_else(|| "the division's base speed is not recorded".into())
}

/// [`eta_remaining`] over `route` from `location` with each edge at its
/// [`edge_speed`], subtracting `numerator` of progress at the first edge's speed.
#[allow(clippy::too_many_arguments)]
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
        // The division's current edge runs into the route's first province.
        current_speed: EtaSpeed::Fixed(speed(location, route[0])?),
        normal: None,
    };
    eta_remaining(&input, &legs).map_err(str::to_owned)
}

pub mod frame_terms;

#[cfg(test)]
mod tests;
