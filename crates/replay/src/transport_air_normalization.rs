//! Normalized air attack, defence and speed.
//! Every define is resolved through the recording's content stack. Raw
//! observations retain the original producer's branch-local failures.
use crate::rows::{
    TransportAirAce as Ace, TransportAirModifiers as Modifiers, TransportAirNormalization as Source,
};
pub type Fact<T> = Result<T, &'static str>;
const P: i64 = 100_000;
const SENTINEL: i64 = 0xffff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defines {
    pub max_attack: i64,
    pub max_defence: i64,
    pub max_speed: i64,
    pub ace_wing_size: i64,
    pub ace_maximum: i64,
    pub night_threshold: i64,
}
impl Defines {
    /// Unclamped content-defined fixed5 operands.
    pub fn read<E>(
        mut fixed5: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            max_attack: fixed5("NAir", "AIR_WING_MAX_STATS_ATTACK")?,
            max_defence: fixed5("NAir", "AIR_WING_MAX_STATS_DEFENCE")?,
            max_speed: fixed5("NAir", "AIR_WING_MAX_STATS_SPEED")?,
            ace_wing_size: fixed5("NAir", "ACE_WING_SIZE")?,
            ace_maximum: fixed5("NAir", "ACE_WING_SIZE_MAX_BONUS")?,
            night_threshold: fixed5("NAir", "DAY_NIGHT_COVERAGE_FACTOR")?,
        })
    }
}
fn required<T>(value: Option<T>, reason: &'static str) -> Fact<T> {
    value.ok_or(reason)
}
fn times(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right) / P
}
fn modifier(source: &Modifiers, index: usize) -> Fact<i64> {
    required(
        source.effective[index],
        "transport normalized air modifier refused",
    )
}
fn ace(source: &Source) -> Fact<Option<&Ace>> {
    required(
        source.ace.as_ref(),
        "transport normalized air ace identity refused",
    )
    .map(Option::as_ref)
}
/// Unresolved assignment or exactly zero signed count supplies unity.
/// Negative aircraft counts remain signed division operands.
fn ace_scale(ace: &Ace, defines: Defines) -> Fact<i64> {
    let assigned = required(
        ace.assigned_aircraft,
        "transport normalized air ace assignment refused",
    )?;
    let Some(count) = assigned.filter(|&count| count != 0) else {
        return Ok(P);
    };
    let denominator = i64::from(count).wrapping_mul(P);
    let numerator = defines.ace_wing_size;
    let ratio = if denominator == 0 || !ordinary(numerator) {
        SENTINEL
    } else {
        numerator
            .wrapping_mul(P)
            .checked_div(denominator)
            .ok_or("transport normalized ace scale division traps")?
    };
    Ok(ratio.min(defines.ace_maximum))
}
fn scaled_ace(ace: Option<&Ace>, index: usize, defines: Defines) -> Fact<i64> {
    let Some(ace) = ace else {
        return Ok(0);
    };
    let modifiers = required(
        ace.modifiers.as_ref(),
        "transport normalized ace modifiers refused",
    )?;
    let raw = modifier(modifiers, index)?;
    let raw = times(raw, ace_scale(ace, defines)?);
    Ok(times(
        raw,
        P.wrapping_add(required(
            ace.owner_effectiveness,
            "transport normalized ace owner effectiveness refused",
        )?),
    ))
}
fn ordinary(raw: i64) -> bool {
    (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}
fn normalize(raw: i64, maximum: i64) -> Fact<i64> {
    if maximum == 0 || !ordinary(raw) {
        return Ok(SENTINEL);
    }
    raw.wrapping_mul(P)
        .checked_div(maximum)
        .ok_or("transport normalized air stat division traps")
}
fn mission_bonus(country: &Modifiers, kind: u32) -> Fact<i64> {
    if kind & 4 != 0 {
        modifier(country, 2)
    } else if kind & 1 != 0 {
        modifier(country, 1)
    } else if kind & 2 != 0 {
        modifier(country, 3)
    } else if kind & 0x110 != 0 {
        modifier(country, 4)
    } else {
        Ok(0)
    }
}

/// Two independent normalized combat leaves: an unrelated attack failure
/// cannot replace a known nonpositive defence with a refusal.
pub fn combat(source: &Source, current_type: u32, defines: Defines) -> [Fact<i64>; 2] {
    [
        combat_lane(source, current_type, defines, 0),
        combat_lane(source, current_type, defines, 1),
    ]
}
fn combat_lane(source: &Source, kind: u32, defines: Defines, lane: usize) -> Fact<i64> {
    let base = required(
        if lane == 0 {
            source.attack
        } else {
            source.defence
        },
        "transport original current-mission combat stat refused",
    )?;
    let wing = required(
        source.wing.as_ref(),
        "transport normalized wing modifier graph refused",
    )?;
    let country = required(
        source.country.as_ref(),
        "transport normalized country modifier graph refused",
    )?;
    let mut factor = P.wrapping_add(required(
        wing.direct[lane],
        "transport normalized wing direct combat scan refused",
    )?);
    factor = factor.wrapping_add(required(
        country.direct[lane],
        "transport normalized country direct combat scan refused",
    )?);
    let ace = ace(source)?;
    if let Some(ace) = ace {
        let scale = ace_scale(ace, defines)?;
        let entries = required(
            ace.combat.as_ref(),
            "transport normalized ordered ace combat scan refused",
        )?;
        for entry in entries {
            if entry.lane > 1 {
                return Err("transport normalized ace combat lane unsupported");
            }
            if entry.lane as usize == lane {
                factor = factor.wrapping_add(times(
                    scale,
                    required(
                        entry.raw,
                        "transport normalized ordered ace combat word refused",
                    )?,
                ));
            }
        }
        // A live ace always demands this effective lookup and type vector.
        let bonus = modifier(country, 7)?;
        let kind = required(
            ace.type_modifiers.as_ref(),
            "transport normalized ace type modifier graph refused",
        )?;
        let count = required(
            kind.positive[lane],
            "transport normalized ace type combat scan refused",
        )?;
        factor = factor.wrapping_add(bonus.wrapping_mul(i64::from(count)));
    }
    let environment = required(
        source.environment.as_ref(),
        "transport normalized air environment demand refused",
    )?;
    let mission = if let Some(environment) = environment {
        let mission = mission_bonus(country, kind)?;
        // Current kind is an explicit override; only override 8 receives the
        // strategic bomber defence modifier.
        if lane == 1 && kind == 8 {
            factor = factor.wrapping_add(modifier(country, 10)?);
        }
        let weather = P
            .wrapping_add(modifier(country, 13)?)
            .wrapping_add(modifier(wing, 13)?)
            .wrapping_add(scaled_ace(ace, 13, defines)?)
            .clamp(0, 200_000);
        factor = factor.wrapping_add(times(
            required(
                environment.weather[lane],
                "transport original air weather modifier refused",
            )?,
            weather,
        ));
        if required(
            environment.night_factor,
            "transport original simulation night factor refused",
        )? > defines.night_threshold
        {
            let mut night = P
                .wrapping_add(modifier(country, 12)?)
                .wrapping_add(modifier(wing, 12)?)
                .wrapping_add(required(
                    source.night_stat,
                    "transport original current-mission night stat refused",
                )?)
                .wrapping_add(scaled_ace(ace, 12, defines)?);
            if source.enabled_missions & 8 != 0 {
                night = night.wrapping_add(modifier(country, 11)?);
            }
            if required(
                source.carrier_mission,
                "transport original carrier night mission refused",
            )?
            .is_some_and(|mission| mission != 0)
            {
                night = night.wrapping_sub(modifier(country, 14)?);
            }
            factor = factor.wrapping_add(times(
                required(
                    environment.night[lane],
                    "transport original air night modifier refused",
                )?,
                night.clamp(0, 200_000),
            ));
        }
        let target = required(
            environment.target_region,
            "transport sampled province region refused",
        )?;
        let base = required(
            source.base_region,
            "transport original airbase region refused",
        )?;
        if target != 0 && base != 0 {
            factor = factor.wrapping_add(modifier(country, if target == base { 8 } else { 9 })?);
        }
        mission
    } else {
        0
    };
    normalize(
        times(factor.wrapping_add(mission), base),
        if lane == 0 {
            defines.max_attack
        } else {
            defines.max_defence
        },
    )
}
/// Normalize ace-modified default speed, never the current mission speed
/// slot; there is no weather/night/location branch.
pub fn speed(source: &Source, defines: Defines) -> Fact<i64> {
    let base = required(source.speed, "transport original default air speed refused")?;
    let ace = ace(source)?;
    let mut factor = P.wrapping_add(scaled_ace(ace, 0, defines)?);
    if let Some(ace) = ace {
        let kind = required(
            ace.type_modifiers.as_ref(),
            "transport normalized ace speed type refused",
        )?;
        if modifier(kind, 0)? != 0 {
            factor = factor.wrapping_add(modifier(
                required(
                    source.country.as_ref(),
                    "transport normalized ace speed country refused",
                )?,
                7,
            )?);
        }
    }
    normalize(times(factor, base), defines.max_speed)
}

#[cfg(test)]
#[path = "transport_air_normalization_tests.rs"]
mod tests;
