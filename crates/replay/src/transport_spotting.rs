//! Native numerical terms used by selected transport spotting.
//! Caller supplies same-frame original mission cache and raw supply inputs.
//! These terms do not reconstruct full strategic spotting or damage.

pub type Fact<T> = Result<T, &'static str>;
const ONE: i64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defines {
    pub base: i64,
    pub radar_bonus: i64,
    pub air_superiority_bonus: i64,
}

impl Defines {
    /// Resolve define names in order through installed_content::Defines::fixed5 using
    /// this callback and the recording's resolved vanilla/DLC/mod stack.
    pub fn read<E>(
        mut fixed5: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            base: fixed5("NNavy", "DETECTION_CHANCE_MULT_BASE")?,
            radar_bonus: fixed5("NNavy", "DETECTION_CHANCE_MULT_RADAR_BONUS")?,
            air_superiority_bonus: fixed5("NNavy", "DETECTION_CHANCE_MULT_AIR_SUPERIORITY_BONUS")?,
        })
    }
}

/// Numeric detection multiplier only. Cache entries retain the stored
/// mission ratio and bonus; the later text-only weather lookup is separate.
/// Interception independently uses full combined NAVAL_DETECTION.
/// Supply Ok(None) means authentic null mission backlink; Err is unavailable.
/// Embedded authenticated task-force missions normally supply Ok(Some(raw ratio)).
pub fn detection_multiplier(
    cache: [i64; 2],
    defines: Defines,
    in_regions: bool,
    received_supply: Fact<Option<i64>>,
) -> Fact<i64> {
    let radar = cache[0].wrapping_mul(defines.radar_bonus) / ONE;
    let air = cache[1].wrapping_mul(defines.air_superiority_bonus) / ONE;
    let total = defines
        .base
        .wrapping_add(radar)
        .wrapping_add(air)
        .wrapping_mul(1000);
    if !in_regions {
        return Ok(total);
    }
    let supply = received_supply?.unwrap_or(0);
    Ok(if supply < ONE {
        supply.wrapping_mul(total) / ONE
    } else {
        total
    })
}

#[cfg(test)]
#[path = "spotting_law_tests.rs"]
mod tests;
