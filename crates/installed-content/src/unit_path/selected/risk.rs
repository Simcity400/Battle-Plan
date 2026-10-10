//! Ordered transport-risk visitor. Use the query's mode-0 forced-transfer
//! route with region checks, rather than the unit's recorded ordinary route.
//! Each reached source is supplied independently by the current world.
use super::super::Fact;
use super::transport::Unit;
const ONE: i64 = 100_000;
const Q15: i64 = 0x8000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExposureDefines {
    pub embark_distance: i64,
    pub speed_scale: i64,
}
impl ExposureDefines {
    pub fn read(defines: &crate::defines::Defines) -> std::io::Result<Self> {
        // Read exposure defines from recorded content without default-value fallback.
        Ok(Self {
            embark_distance: defines.fixed5("NMilitary", "AMPHIBIOUS_INVADE_MOVEMENT_COST")?,
            speed_scale: defines.fixed5("NNavy", "NAVAL_SPEED_MODIFIER")?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChanceDefines {
    pub spotting_weight: i64,
    pub damage_weight: i64,
    pub base: i64,
}
impl ChanceDefines {
    pub fn read(defines: &crate::defines::Defines) -> std::io::Result<Self> {
        // Read mission-chance weights and base chance from recorded content.
        Ok(Self {
            spotting_weight: defines.fixed5(
                "NNavy",
                "BASE_SPOTTING_EFFECT_FOR_INITIAL_UNIT_TRANSFER_SPOTTING",
            )?,
            damage_weight: defines.fixed5(
                "NNavy",
                "SPOTTING_SPEED_EFFECT_FOR_INITIAL_UNIT_TRANSFER_SPOTTING",
            )?,
            base: defines.fixed5("NNavy", "UNIT_TRANSFER_DETECTION_CHANCE_BASE")?,
        })
    }
}

pub trait Inputs {
    /// Search failure is None; successful equal endpoints are Some(empty).
    fn route(&mut self, unit: Unit, source: u32, target: u32) -> Fact<Option<Vec<u32>>>;
    fn region(&mut self, province: u32) -> Fact<Option<u32>>;
    /// A route ID may resolve to a null province pointer.
    fn province_present(&mut self, province: u32) -> Fact<bool>;
    fn enemies(&mut self, unit: Unit) -> Fact<Vec<u32>>;
    fn ledger(&mut self, enemy_raw: u32, region: u32) -> Fact<Vec<u64>>;
    /// Zero occurrence was already skipped; authentic mission gates may
    /// supply zero, but an unreadable reached spotting/damage input may not.
    fn mission_chance(&mut self, receiver: u64, unit: Unit, convoys: i32) -> Fact<i64>;
    fn distance(&mut self, unit: Unit, previous: Option<u32>, current: u32) -> Fact<i64>;
    /// The selected logical owner's naval speed in the previous strategic
    /// region, including the content-derived speed scale.
    fn speed(&mut self, unit: Unit, previous_region: Option<u32>) -> Fact<i64>;
}

/// Signed fixed5 to Q15 conversion, with wrapping arithmetic.
pub fn fixed5_to_q15(raw: i64) -> i64 {
    ((raw % ONE).wrapping_mul(0x4000_0000) / 0xc350_0000)
        .wrapping_add((raw / ONE).wrapping_mul(Q15))
}
fn normalize(raw: i64) -> i64 {
    let shifted = raw.wrapping_shl(15);
    let high = ((i128::from(shifted) * i128::from(-0x5c28_f5c2_8f5c_28f5_i64)) >> 64) as i64;
    let value = shifted.wrapping_add(high) >> 21;
    value.wrapping_add(((value as u64) >> 63) as i64)
}
/// All products and additions wrap before signed division. Runtime define
/// values come from recorded content, with no defaults.
pub fn mission_chance(
    spotting: i64,
    damage: i64,
    spotting_weight: i64,
    damage_weight: i64,
    base: i64,
) -> i64 {
    let weighted = (spotting.wrapping_mul(spotting_weight) / ONE)
        .wrapping_add(damage.wrapping_mul(damage_weight) / ONE);
    (normalize(fixed5_to_q15(weighted)).wrapping_mul(normalize(fixed5_to_q15(base))) >> 15)
        .clamp(0, Q15)
}
fn risk(survival: i64) -> i64 {
    // Bounded Q15 survival permits direct half-up rounding to fixed5 risk.
    ((Q15 - survival) * ONE + 0x4000) >> 15
}

pub fn query(
    unit: Unit,
    source: u32,
    target: u32,
    convoys: i32,
    inputs: &mut impl Inputs,
) -> Fact<i64> {
    let Some(route) = inputs.route(unit, source, target)? else {
        return Ok(0);
    };
    if route.is_empty() {
        return Ok(0);
    }
    let mut previous_region = inputs.region(source)?;
    let enemies = inputs.enemies(unit)?;
    let mut previous = Some(source);
    let mut current_region = None;
    let mut region_survival = Q15;
    let mut survival = Q15;
    let mut remainder = 0_i64;
    for province in route {
        if inputs.province_present(province)? {
            let region = inputs.region(province)?;
            if region != current_region {
                current_region = region;
                region_survival = Q15;
                for &enemy in &enemies {
                    let region =
                        region.ok_or("transport risk reached a null strategic-region receiver")?;
                    for receiver in inputs.ledger(enemy, region)? {
                        if receiver == 0 {
                            continue;
                        }
                        let chance = inputs.mission_chance(receiver, unit, convoys)?;
                        if !(0..=Q15).contains(&chance) {
                            return Err("transport mission chance outside its clamp".into());
                        }
                        region_survival = region_survival.wrapping_mul(Q15 - chance) >> 15;
                    }
                }
            }
            if region_survival < Q15 {
                let distance = inputs.distance(unit, previous, province)?;
                let speed = inputs.speed(unit, previous_region)?;
                if speed == 0 {
                    return Ok(ONE);
                }
                let exposure = if (distance as u64).wrapping_add(0x7fff_ffff_ffff_fffe)
                    < 0xffff_ffff_ffff_fffd
                {
                    distance
                        .wrapping_mul(ONE)
                        .checked_div(speed)
                        .ok_or("transport exposure division overflows")?
                } else {
                    0xffff_ffff
                };
                remainder = remainder.wrapping_add(exposure);
                while remainder > 99_999 {
                    survival = survival.wrapping_mul(region_survival) >> 15;
                    remainder = remainder.wrapping_sub(ONE);
                    if survival == 0 {
                        return Ok(ONE);
                    }
                }
            } else {
                remainder = 0;
            }
            previous = Some(province);
        } else {
            previous = None;
        }
        previous_region = current_region;
    }
    Ok(risk(survival))
}

#[cfg(test)]
#[path = "risk_tests.rs"]
mod tests;
