//! Numerical strategic spotting graph, with branch-local source demands.
//! Inputs must come from this hour's original physical ledgers and receiver
//! observations. The air disruption producer and cache/lifetime admission are
//! separate requirements; unavailable reached inputs remain named refusals.

pub type Fact<T> = Result<T, &'static str>;
const P: i64 = 100_000;
const REFUSED: i64 = 0xffff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defines {
    pub base: i64,
    pub visibility_scale: i64,
    pub navy_scale: i64,
    pub navy_base: i64,
    pub radar_base: i64,
    pub decryption_base: i64,
    pub intel_minimum: i64,
    pub intel_maximum: i64,
}

impl Defines {
    /// Bounded fixed5 define loads apply the shown clamps; visibility and
    /// both NIntel thresholds have no loader clamp. No fallback values.
    pub fn read<E>(
        mut fixed5: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            base: fixed5("NNavy", "BASE_SPOTTING")?.clamp(0, 10_000_000),
            visibility_scale: fixed5("NNavy", "VISIBILITY_MULTIPLIER_FOR_SPOTTING")?,
            navy_scale: fixed5("NNavy", "NAVY_SPOTTER_DETECTION_FACTOR")?.max(0),
            navy_base: fixed5("NNavy", "BASE_SPOTTING_FROM_NAVY")?.clamp(0, 10_000_000),
            radar_base: fixed5("NNavy", "BASE_SPOTTING_FROM_RADAR")?.clamp(0, 10_000_000),
            decryption_base: fixed5("NNavy", "BASE_SPOTTING_FROM_DECRYPTION")?.clamp(0, 10_000_000),
            intel_minimum: fixed5(
                "NIntel",
                "NAVY_INTEL_BASE_SPOTTING_BONUS_MIN_INTEL_FOR_BONUS",
            )?,
            intel_maximum: fixed5(
                "NIntel",
                "NAVY_INTEL_BASE_SPOTTING_BONUS_MAX_INTEL_FOR_BONUS",
            )?,
        })
    }
}

pub trait Inputs {
    /// First nonzero unfiltered convoy-pool variant's visibility. None is no
    /// variant; a failed visibility word is a refusal.
    fn first_convoy_visibility(&mut self) -> Fact<Option<i64>>;
    fn defines(&mut self) -> Fact<Defines>;
    /// Detection for every original owner navy/region mission occurrence.
    /// Each mission and backlink is dereferenced; null occurrences are reached
    /// unsupported forms here, rather than skipped by a mission census.
    fn navy_detection(&mut self) -> Fact<Vec<i64>>;
    /// Air contribution with secondary visibility zero. Known empty regional
    /// registrations supply zero without demanding wing stat inputs.
    fn air_contribution(&mut self, visibility: i64) -> Fact<i64>;
    /// Raw radar count and strategic region's signed province count.
    fn radar_counts(&mut self) -> Fact<(i32, i32)>;
    /// Navy intel lane: own target amount minus reciprocal owner amount.
    fn navy_intel_difference(&mut self) -> Fact<i64>;
    /// Nullable weather modifier, effective key 0x2A.
    fn weather_detection(&mut self) -> Fact<Option<i64>>;
}

/// Numeric query without tooltip text. Stages follow the game's demand
/// order; zero visibility still reads navy, air, radar, intel and weather.
/// Only absence of the first convoy variant returns before those demands.
pub fn query(inputs: &mut impl Inputs) -> Fact<i64> {
    let Some(raw_visibility) = inputs.first_convoy_visibility()? else {
        return Ok(0);
    };
    let defines = inputs.defines()?;
    let visibility = raw_visibility.wrapping_mul(defines.visibility_scale) / P;
    let detection = inputs
        .navy_detection()?
        .into_iter()
        .fold(0_i64, i64::wrapping_add)
        .wrapping_mul(visibility)
        / P;
    let detection = detection.wrapping_mul(defines.navy_scale) / P;
    let navy = falloff(detection)?.wrapping_mul(defines.navy_base) / P;
    let air = inputs.air_contribution(visibility)?;
    let (covered, provinces) = inputs.radar_counts()?;
    let radar = radar(covered, provinces)?.wrapping_mul(defines.radar_base) / P;
    let decryption = decryption(inputs.navy_intel_difference()?, defines)?;
    let total = defines
        .base
        .wrapping_add(navy)
        .wrapping_add(air)
        .wrapping_add(radar)
        .wrapping_add(decryption);
    let weather = inputs
        .weather_detection()?
        .map_or(P, |raw| P.wrapping_add(raw))
        .max(0);
    Ok((weather.wrapping_mul(total) / P).max(0))
}

/// Task-force detection adds the ordered repeated carrier-fill sum times
/// its carrier detection stat to its surface detection stat, then multiplies
/// by the common detection value plus unity. The original ship list owns
/// fill observations; it is not rebuilt.
pub fn task_force_detection(
    detection_138: i64,
    carrier_detection_160: i64,
    carrier_fills: impl IntoIterator<Item = Fact<i64>>,
    combined_detection: i64,
) -> Fact<i64> {
    let mut fill = 0_i64;
    for value in carrier_fills {
        fill = fill.wrapping_add(value?);
    }
    let carrier = fill.wrapping_mul(carrier_detection_160) / P;
    Ok((detection_138
        .wrapping_add(carrier)
        .wrapping_mul(P.wrapping_add(combined_detection))
        / P)
        .max(0))
}

/// Power of two at the raw value, then 1 - 1/power. Raw >= 10 skips power.
/// This is not a floating-point exponential and has no independent lower clamp.
fn falloff(raw: i64) -> Fact<i64> {
    if raw >= 1_000_000 {
        return Ok(P);
    }
    let power = engine::fixed5::power(2 * P, raw)?;
    let inverse = if power == 0 {
        REFUSED
    } else {
        (P * P)
            .checked_div(power)
            .ok_or("selected transport spotting reciprocal division traps")?
    };
    Ok(P.wrapping_sub(inverse))
}

/// Both counts are signed; the second numerator multiplication wraps before
/// division. A zero denominator supplies the sentinel.
fn radar(covered: i32, provinces: i32) -> Fact<i64> {
    let denominator = i64::from(provinces) * P;
    if denominator == 0 {
        return Ok(REFUSED);
    }
    i64::from(covered)
        .wrapping_mul(P)
        .wrapping_mul(P)
        .checked_div(denominator)
        .ok_or("selected transport radar division traps")
}

/// Negation wraps, including i64::MIN. The threshold order does not sort
/// or clamp the two content defines.
pub fn decryption(difference: i64, defines: Defines) -> Fact<i64> {
    let magnitude = if difference < 0 {
        difference.wrapping_neg()
    } else {
        difference
    };
    let above = magnitude.wrapping_sub(defines.intel_minimum);
    if above < 0 {
        return Ok(0);
    }
    let width = defines.intel_maximum.wrapping_sub(defines.intel_minimum);
    if width == 0 {
        return Ok(0);
    }
    let amount = above.min(width).wrapping_mul(defines.decryption_base) / P;
    let ratio = if (amount as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        amount
            .wrapping_mul(P)
            .checked_div(width)
            .ok_or("selected transport decryption division traps")?
    } else {
        REFUSED
    };
    Ok(if difference < 1 {
        ratio.wrapping_neg()
    } else {
        ratio
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AirDefines {
    pub normalized_size: i32,
    pub detection_scale: i64,
    pub base: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisruptionDefines {
    pub defence: i64,
    pub attack: i64,
    pub speed: i64,
}

impl DisruptionDefines {
    /// Unclamped fixed5 content loads.
    pub fn read<E>(
        mut fixed5: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            defence: fixed5("NAir", "DISRUPTION_DEFENCE_DEFENCE_FACTOR")?,
            attack: fixed5("NAir", "DISRUPTION_DEFENCE_ATTACK_FACTOR")?,
            speed: fixed5("NAir", "DISRUPTION_DEFENCE_SPEED_FACTOR")?,
        })
    }
}

/// Raw mission effectiveness, signed aircraft count and current type.
/// Normalized attack/defence use the sampled province and current mission;
/// normalized speed is independent. Stored sortie/raid
/// values never stand in for those independently demanded producers.
pub fn disruption_factor(
    raw: Fact<i64>,
    aircraft: i32,
    kind: u32,
    normalized_combat: Fact<[i64; 2]>,
    normalized_speed: Fact<i64>,
    defines: Fact<DisruptionDefines>,
) -> Fact<i64> {
    disruption_factor_leaves(
        raw,
        aircraft,
        kind,
        normalized_combat.map(|value| value[0]),
        normalized_combat.map(|value| value[1]),
        normalized_speed,
        defines,
    )
}

/// Normalized defence is checked before normalized attack and speed are
/// demanded. Independent source failures retain the early-unity gate.
pub fn disruption_factor_leaves(
    raw: Fact<i64>,
    aircraft: i32,
    kind: u32,
    normalized_attack: Fact<i64>,
    normalized_defence: Fact<i64>,
    normalized_speed: Fact<i64>,
    defines: Fact<DisruptionDefines>,
) -> Fact<i64> {
    let raw = raw?;
    if raw == 0 || kind & 0x4400 != 0 || aircraft <= 0 {
        return Ok(P);
    }
    let defence = normalized_defence?;
    if defence < 1 {
        return Ok(P);
    }
    let attack = normalized_attack?;
    let speed = normalized_speed?;
    let defines = defines?;
    let defence = (defence.wrapping_mul(defines.defence) / P).wrapping_add(P);
    let attack = (attack.wrapping_mul(defines.attack) / P).wrapping_add(P);
    let speed = (speed.wrapping_mul(defines.speed) / P).wrapping_add(P);
    let denominator = ((defence.wrapping_mul(attack) / P).wrapping_mul(speed) / P)
        .wrapping_mul(i64::from(aircraft));
    let root = sqrt(guarded_fraction(raw, denominator)?)?;
    if (root as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc {
        return Ok(100);
    }
    // Signed high multiplication implements division by -316200.
    Ok(P.wrapping_add(root.wrapping_mul(P) / -316_200)
        .clamp(100, P))
}

fn guarded_fraction(numerator: i64, denominator: i64) -> Fact<i64> {
    if denominator == 0
        || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc
    {
        return Ok(REFUSED);
    }
    numerator
        .wrapping_mul(P)
        .checked_div(denominator)
        .ok_or("transport air disruption fixed5 division traps")
}

/// Scale reduction around the integer Newton loop. The bound refuses
/// nonconvergence, never returns an approximate result.
pub fn sqrt(mut raw: i64) -> Fact<i64> {
    if raw < 10_001 {
        return Ok(sqrt_inner(raw.wrapping_mul(100))?.wrapping_mul(10_000) / P);
    }
    let mut scales = 0_u32;
    while raw > 2_999_999_999 {
        raw = raw.wrapping_mul(1_000) / P;
        scales += 1;
        if scales == 65_536 {
            return Err("transport native air sqrt scaling did not converge");
        }
    }
    let mut result = sqrt_inner(raw)?;
    for _ in 0..scales {
        result = result.wrapping_mul(10);
    }
    Ok(result)
}

fn sqrt_inner(raw: i64) -> Fact<i64> {
    if raw < 2 {
        return Ok(0);
    }
    let mut result = if raw < i64::MAX {
        raw.wrapping_mul(P) / 200_000
    } else {
        REFUSED
    };
    let tolerance = raw.wrapping_mul(1_000) / P;
    for _ in 0..65_536 {
        let error = (result.wrapping_mul(result) / P).wrapping_sub(raw);
        let step = guarded_fraction(error, result.wrapping_mul(2))?;
        result = result.wrapping_sub(step);
        if !(tolerance < error || error < tolerance.wrapping_neg()) || step == 0 {
            return Ok(result);
        }
    }
    Err("transport native air sqrt Newton loop did not converge")
}

impl AirDefines {
    /// The integer plane scale clamps at 1, detection factor at 0, and
    /// base spotting at 0..100 fixed5.
    pub fn read<E>(
        mut fixed5: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
        mut int32: impl FnMut(&'static str, &'static str) -> Result<i32, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            normalized_size: int32("NNavy", "AIR_SPOTTER_NORMALIZED_AIRWING_SIZE")?.max(1),
            detection_scale: fixed5("NNavy", "AIR_SPOTTER_DETECTION_FACTOR")?.max(0),
            base: fixed5("NNavy", "BASE_SPOTTING_FROM_AIR")?.clamp(0, 10_000_000),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AirMission {
    pub current_type: Fact<u32>,
    /// Stored signed aircraft count; hourly eligibility gates may leave it stale.
    pub aircraft: Fact<i32>,
    /// Exact numerical combat result for this mission and region's hourly
    /// sampled province. It has a separate normalization producer;
    /// no stored efficiency, count, weather flag or raid score substitutes.
    pub disruption_factor: Fact<i64>,
    /// Current-type stats 13 and 14, in that order.
    pub detection: Fact<[i64; 2]>,
}

/// Selects an original physical root and its complete regional header domain.
/// Native root nulls are distinct from an authentic zero regional count.
/// A positive vector retains nullable occurrences for the native sample-
/// before-dereference demand order in `air_contribution`.
pub fn air_region(
    packet: &crate::rows::TransportAirInputs,
    hour: i32,
    owner_slot: u32,
    region: u32,
) -> Fact<(
    &crate::rows::TransportAirCountry,
    &crate::rows::TransportAirRegion,
)> {
    if packet.sample_hour != hour {
        return Err("transport original air graph has wrong sample hour");
    }
    let countries = packet
        .countries
        .as_deref()
        .ok_or("transport original air physical root domain refused")?;
    for (slot, country) in countries.iter().enumerate() {
        if country.slot as usize != slot || country.present != country.country.is_some() {
            return Err("transport original air physical root domain malformed");
        }
    }
    let country = countries
        .get(owner_slot as usize)
        .ok_or("transport original air owner outside physical root domain")?;
    if !country.present {
        return Err("transport reached original null strategic air root");
    }
    let regions = country
        .regions
        .as_deref()
        .ok_or("transport original air region header domain refused")?;
    if regions
        .iter()
        .enumerate()
        .any(|(slot, row)| row.region as usize != slot)
    {
        return Err("transport original air region header domain malformed");
    }
    let region = regions
        .get(region as usize)
        .ok_or("transport original air region outside header domain")?;
    let count = region
        .count
        .ok_or("transport original air regional count refused")?;
    if count < 0 {
        return Err("transport original air regional count negative");
    }
    if count > 0
        && region
            .missions
            .as_ref()
            .is_none_or(|missions| missions.len() != count as usize)
    {
        return Err("transport original air regional occurrence list refused");
    }
    Ok((country, region))
}

/// Caller supplies the complete original regional air-registration
/// occurrence list, including repeats, and the content stack's fixed5
/// AIR_MISSION_SPOTTING_FACTORS array. Empty lists and zero factors retain
/// the game's lazy leaf demands.
pub fn air_contribution(
    missions: &[AirMission],
    sampled_province: Fact<u32>,
    defines: Fact<AirDefines>,
    factors: Fact<&[i64]>,
    regional_factor: Fact<i64>,
    visibility: [i64; 2],
) -> Fact<[i64; 2]> {
    if missions.is_empty() {
        return Ok([0, 0]);
    }
    let defines = defines?;
    let denominator = i64::from(defines.normalized_size) * P;
    let normalized = if denominator == 0 {
        REFUSED
    } else {
        (P * P) / denominator
    };
    // Sample the province at `(hour - 43_800_000) % signed_count` before the
    // first mission, even when every mission or factor is zero.
    let _sampled_province = sampled_province?;
    let mut detection = [0_i64; 2];
    for mission in missions {
        let kind = mission.current_type?;
        if kind == 0 {
            continue;
        }
        // Arithmetic-right-shift to the highest set bit. Reached multibit or
        // sign-bit forms fail the single-bit assertion.
        if !kind.is_power_of_two() || kind as i32 <= 0 {
            return Err("selected transport air spotting mission is not one positive bit");
        }
        let factor = *factors?
            .get(kind.trailing_zeros() as usize)
            .ok_or("selected transport air mission missing from AIR_MISSION_SPOTTING_FACTORS")?;
        if factor == 0 {
            continue;
        }
        let aircraft = mission.aircraft?;
        let efficiency = if kind & 0x1f7fa == 0 {
            P
        } else {
            mission.disruption_factor?
        };
        let weight = (i64::from(aircraft).wrapping_mul(normalized).wrapping_mul(P) / P)
            .wrapping_mul(factor)
            / P;
        let weight = weight.wrapping_mul(efficiency) / P;
        let stats = mission.detection?;
        for (sum, stat) in detection.iter_mut().zip(stats) {
            *sum = sum.wrapping_add(weight.wrapping_mul(stat) / P);
        }
    }
    // Native sums both lanes before deciding whether to reach the manager
    // region factor, even when secondary visibility is zero.
    if detection[0].wrapping_add(detection[1]) == 0 {
        return Ok([0, 0]);
    }
    let weighted = (detection[0].wrapping_mul(visibility[0]) / P)
        .wrapping_add(detection[1].wrapping_mul(visibility[1]) / P);
    let raw =
        (weighted.wrapping_mul(regional_factor?) / P).wrapping_mul(defines.detection_scale) / P;
    Ok([raw, falloff(raw)?.wrapping_mul(defines.base) / P])
}

#[cfg(test)]
#[path = "transport_spotting_graph_tests.rs"]
mod tests;
