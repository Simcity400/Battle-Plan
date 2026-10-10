//! The native wing-range law shared by raid gates and battle-plan labels.
//! Callers retain their own lazy table admission and current-frame joins.

use replay::rows::{AceRangeInputRow, AirWingRow, CountryCommandPowerRow, RaidCountryInputRow};

#[derive(Clone, Copy)]
pub(super) struct Defines {
    pub map_scale: i64,
    pub ace_wing_size: i64,
    pub ace_wing_size_max_bonus: i64,
}

pub(super) trait Sources {
    fn slot(&self, raw: u32) -> Option<u32>;
    fn country(&self, slot: u32) -> Result<&RaidCountryInputRow, String>;
    fn command_power(&self, slot: u32) -> Result<&CountryCommandPowerRow, String>;
    fn wing(&self, handle: u64) -> Result<Option<&AirWingRow>, String>;
    fn ace(&self, handle: u64) -> Result<Option<&AceRangeInputRow>, String>;
}

/// `CAirWing::GetModifiedMapRange@140F49440`.
pub(super) fn wing_range(
    wing: &AirWingRow,
    defines: Defines,
    sources: &impl Sources,
) -> Result<i64, String> {
    let base = to_map(wing.base_range_raw, defines.map_scale);
    let raw = if wing.country_override > 0 {
        wing.country_override as u32
    } else {
        wing.owner
    };
    let slot = sources
        .slot(raw)
        .ok_or("a wing country reference does not resolve")?;
    let country = sources.country(slot)?;
    let mut factor = 100_000_i64;
    if wing.ace != 0 {
        let ace = sources
            .ace(wing.ace)?
            .ok_or("a wing's ace has no AceRangeInputs row")?;
        if ace.alive != 0 {
            // `CAce::scaled_modifier@14060DDF0(ace, 5)`. The ace country
            // modifiers are CountryCommandPower's, read hourly.
            let owner = sources.command_power(ace.canonical_slot)?;
            let scaled = ace.range.wrapping_mul(ace_scale(ace, defines, sources)?) / 100_000;
            let scaled = registered(owner.ace_effectiveness_factor, "ace_effectiveness_factor")?
                .wrapping_add(100_000)
                .wrapping_mul(scaled)
                / 100_000;
            factor = scaled.wrapping_add(100_000);
            if ace.type_range != 0 {
                factor = factor.wrapping_add(registered(
                    sources.command_power(slot)?.air_ace_bonuses_factor,
                    "air_ace_bonuses_factor",
                )?);
            }
        }
    }
    let air = registered(country.air_range_factor, "air_range_factor")?;
    Ok(base.wrapping_mul(factor.wrapping_add(air)) / 100_000)
}

/// The ace's wing is its `CAce+0xE8`, as of the country's daily read.
pub(super) fn ace_scale(
    ace: &AceRangeInputRow,
    defines: Defines,
    sources: &impl Sources,
) -> Result<i64, String> {
    let Some(wing) = sources.wing(ace.wing)?.filter(|_| ace.wing != 0) else {
        return Ok(100_000);
    };
    let planes = i64::from(wing.planes);
    if planes == 0 {
        return Ok(100_000);
    }
    Ok(if finite(defines.ace_wing_size) {
        (defines.ace_wing_size.wrapping_mul(100_000) / planes.wrapping_mul(100_000))
            .min(defines.ace_wing_size_max_bonus)
    } else {
        defines.ace_wing_size_max_bonus.min(0xffff_ffff)
    })
}

/// `x + 0x7FFF_FFFF_FFFF_FFFE < 0xFFFF_FFFF_FFFF_FFFD`.
pub(super) fn finite(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

/// `x * 100000 / MAP_SCALE_PIXEL_TO_KM`, or `0xFFFFFFFF`.
pub(super) fn to_map(value: i64, scale: i64) -> i64 {
    if scale == 0 || !finite(value) {
        0xffff_ffff
    } else {
        value.wrapping_mul(100_000) / scale
    }
}

pub(super) fn registered(value: Option<i64>, name: &str) -> Result<i64, String> {
    value.ok_or_else(|| format!("{name} is not a registered modifier"))
}

#[cfg(test)]
mod tests;
