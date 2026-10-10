//! Selected transport interception for the supported build.
//! Every live leaf comes from the original hourly receiver observation. The
//! viewer's signed low32 convoy operand remains query-local.

pub type Fact<T> = Result<T, &'static str>;
const ONE: i64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub visibility: Fact<i64>,
    pub surface_detection: Fact<i64>,
    pub submarine_detection: Fact<i64>,
    pub speed: Fact<i64>,
}

/// Each callback is demanded in native order. A failed reached input refuses
/// the calculation; a native early return never demands the later callbacks.
pub trait Inputs {
    fn detection_multiplier(&mut self) -> Fact<i64>;
    fn combined_detection(&mut self) -> Fact<i64>;
    fn detection_stats(&mut self) -> Fact<[i64; 2]>;
    fn carrier_fill_sum(&mut self) -> Fact<i64>;
    fn first_variant(&mut self) -> Fact<Option<Variant>>;
    fn visibility_stats(&mut self) -> Fact<[i64; 2]>;
    fn detached_activity(&mut self) -> Fact<i32>;
    fn visibility_modifier(&mut self) -> Fact<i64>;
    fn actual_speed(&mut self) -> Fact<i64>;
    fn coordination(&mut self) -> Fact<i64>;
    fn spotting_modifier(&mut self) -> Fact<i64>;
    /// Roster owner's relation to the actual location controller.
    fn targeted_spotting_modifier(&mut self) -> Fact<i64>;
    /// Night factor at the admitted simulation hour, independent of display mode.
    fn simulation_night_factor(&mut self) -> Fact<i64>;
    fn night_spotting_modifier(&mut self) -> Fact<i64>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defines {
    pub base: i64,
    pub transfer_multiplier: i64,
    pub surface_multiplier: i64,
    pub submarine_multiplier: i64,
    pub convoy_count_exponent: i64,
    pub visibility_multiplier: i64,
    pub detection_base: i64,
    pub enemy_detection_multiplier: i64,
    pub transport_speed_scale: i64,
    pub catching_up_multiplier: i64,
    pub running_away_multiplier: i64,
    pub coordination_factor: i64,
    pub repair_visibility_multiplier: i64,
}

impl Defines {
    /// NNavy defines resolved through the recording's existing content stack.
    pub fn read<E>(
        mut fixed5: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            base: fixed5("NNavy", "BASE_UNIT_TRANSFER_SPOTTING_SPEED")?,
            transfer_multiplier: fixed5("NNavy", "UNIT_TRANSFER_SPOTTING_SPEED_MULT")?,
            surface_multiplier: fixed5("NNavy", "SPOTTING_MULTIPLIER_FOR_SURFACE")?,
            submarine_multiplier: fixed5("NNavy", "SPOTTING_MULTIPLIER_FOR_SUB")?,
            convoy_count_exponent: fixed5("NNavy", "SPOTTING_MOD_FOR_CONVOY_COUNT")?,
            visibility_multiplier: fixed5("NNavy", "VISIBILITY_MULTIPLIER_FOR_SPOTTING")?,
            detection_base: fixed5("NNavy", "DETECTION_CHANCE_MULT_BASE")?,
            enemy_detection_multiplier: fixed5(
                "NNavy",
                "SPOTTING_ENEMY_SPOTTING_MULTIPLIER_FOR_RUNNING_AWAY",
            )?,
            transport_speed_scale: fixed5("NNavy", "NAVAL_SPEED_MODIFIER")?,
            catching_up_multiplier: fixed5("NNavy", "SPOTTING_SPEED_MULT_FOR_CATCHING_UP")?,
            running_away_multiplier: fixed5("NNavy", "SPOTTING_SPEED_MULT_FOR_RUNNING_AWAY")?,
            coordination_factor: fixed5("NNavy", "COORDINATION_EFFECT_ON_PATROL_SPOTTING")?,
            repair_visibility_multiplier: fixed5(
                "NNavy",
                "NAVY_VISIBILITY_BONUS_ON_RETURN_FOR_REPAIR",
            )?,
        })
    }
}

fn mul(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right) / ONE
}

/// Detection retains the wrapping ordered carrier-fill sum. Its two stat
/// lanes are independent; a null or refused ship is never omitted.
pub fn detection(stats: [i64; 2], carrier_fill_sum: i64, combined: i64) -> i64 {
    mul(
        stats[0].wrapping_add(mul(carrier_fill_sum, stats[1])),
        combined.wrapping_add(ONE),
    )
    .max(0)
}

/// Visibility with ordered intermediate truncations and the final clamp.
pub fn visibility(stat: i64, modifier: i64, detached_activity: i32, defines: Defines) -> i64 {
    let repair = if detached_activity == 1 {
        defines.repair_visibility_multiplier
    } else {
        ONE
    };
    mul(
        mul(mul(stat, defines.visibility_multiplier), repair),
        modifier.wrapping_add(ONE),
    )
    .max(0)
}

/// Flag-zero selected-transfer interception speed, before the mission view's
/// independent weighting and Q15 conversion.
pub fn query(inputs: &mut impl Inputs, defines: Defines, convoys: i32) -> Fact<i64> {
    let mission_detection = inputs.detection_multiplier()?;
    let combined = inputs.combined_detection()?;
    let stats = inputs.detection_stats()?;
    let carrier_fill = inputs.carrier_fill_sum()?;
    let task_detection = detection(stats, carrier_fill, combined);
    let detection_multiplier = mul(task_detection, mission_detection);
    let surface_multiplier = mul(defines.transfer_multiplier, defines.surface_multiplier);
    let Some(variant) = inputs.first_variant()? else {
        return Ok(0);
    };
    // Sign-extend the low32 caller operand before multiplying it.
    let count = engine::fixed5::power(
        i64::from(convoys).wrapping_mul(ONE),
        defines.convoy_count_exponent,
    )?;
    let visible = mul(
        mul(variant.visibility?, defines.visibility_multiplier),
        count,
    )
    .max(0);
    let target = mul(visible, detection_multiplier).max(0);
    let own_stats = inputs.visibility_stats()?;
    let activity = inputs.detached_activity()?;
    let own_modifier = inputs.visibility_modifier()?;
    let own = own_stats.map(|stat| visibility(stat, own_modifier, activity, defines));
    let enemy_term = |stat: i64, own: i64| {
        // The *1000 happens before the division; do not move it outwards.
        let scaled = stat.wrapping_mul(defines.detection_base).wrapping_mul(1000) / ONE;
        mul(
            mul(own.max(0), scaled).max(0),
            defines.enemy_detection_multiplier,
        )
    };
    let surface_enemy = enemy_term(variant.surface_detection?, own[0]);
    let submarine_enemy = enemy_term(variant.submarine_detection?, own[1]);
    let surface = mul(target.wrapping_sub(surface_enemy), surface_multiplier);
    // Negation wraps, including i64::MIN; it is not absolute value.
    let submarine = mul(submarine_enemy.wrapping_neg(), defines.submarine_multiplier);
    let mut chance = defines
        .base
        .wrapping_add(surface.wrapping_add(submarine).wrapping_mul(100) / ONE);
    let delta = inputs
        .actual_speed()?
        .wrapping_sub(mul(variant.speed?, defines.transport_speed_scale));
    let speed_multiplier = if delta < 0 {
        defines.running_away_multiplier
    } else {
        defines.catching_up_multiplier
    };
    chance = chance.wrapping_add(mul(mul(delta, speed_multiplier), surface_multiplier));
    let coordination = mul(inputs.coordination()?, defines.coordination_factor).wrapping_add(ONE);
    chance = mul(chance, coordination);
    // The entry gate remains the original positive chance, even if the first
    // modifier multiplication makes it nonpositive before night.
    if chance > 0 {
        let spotting = inputs.spotting_modifier()?;
        let targeted = inputs.targeted_spotting_modifier()?;
        chance = mul(chance, targeted.wrapping_add(ONE).wrapping_add(spotting));
        if inputs.simulation_night_factor()? > 80_000 {
            chance = mul(chance, inputs.night_spotting_modifier()?.wrapping_add(ONE));
        }
    }
    Ok(chance)
}

/// Carrier fill ratio after the carrier predicate and base-null gates.
/// Planes are a signed32 wrapping sum, never effective planes.
pub fn carrier_fill(capacity: i32, planes: i32) -> i64 {
    let denominator = i64::from(capacity) * ONE;
    if denominator == 0 {
        return 0;
    }
    (i64::from(planes).wrapping_mul(10_000_000_000) / denominator).clamp(0, ONE)
}

#[cfg(test)]
#[path = "transport_damage_graph_tests.rs"]
mod tests;
