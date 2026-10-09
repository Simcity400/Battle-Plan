//! Current required-use operands for FuelPanel_BuildBreakdown@1410DE880.
//! This is a supplying-read rate, not preceding-day applied consumption.
//! Country roots reuse only the current quiescent callback's certified census.

use crate::{Memory, memory::Raw, read};

mod air;
mod capture;
mod demand;
pub use capture::{BufferRows, Buffers, capture};
pub use demand::demand;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Name {
    pub bytes: Option<Span>,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub country: u32,
    pub instance: Option<u32>,
    pub divisions: Span,
    pub army_reason: Option<&'static str>,
    pub air_reason: Option<&'static str>,
    pub sites: Span,
}

/// Native roster occurrences are retained, including a repeated unit handle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CachedUnit {
    pub country: u32,
    pub reference: u64,
    pub parent: u64,
    pub ordinal: u32,
    pub owners: Option<[u32; 3]>,
    pub rate: Option<i64>,
    pub reason: Option<&'static str>,
    pub name: Name,
    pub group: Option<u64>,
    pub group_reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fleet {
    pub country: u32,
    pub reference: u64,
    pub name: Name,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Site {
    pub list: u8,
    pub ordinal: u32,
    pub base: u64,
    pub container_owner: Option<u32>,
    pub capacity: Option<i64>,
    pub demilitarized: Option<u8>,
    pub wings: Span,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wing {
    pub reference: u64,
    pub owner: Option<u32>,
    pub country_override: Option<i32>,
    pub owner_reason: Option<&'static str>,
    pub pool_ordinal: u32,
    pub wing_ordinal: u32,
    pub name: Name,
    pub inputs: Option<AirInputs>,
    pub weather: Span,
    pub context: Option<usize>,
    pub reason: Option<&'static str>,
}

/// Effective modifier operands use the shared passive 140551820 reader.
/// Raw value/presence remain distinct from feature/clamp eligibility.
pub use crate::modifier_accumulator::RequestedOperand as Modifier;

/// Only reached operands are acquired. A skipped wing needs no fuel, ace,
/// mission-cost or weather reads. `reached` certifies that native gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AirInputs {
    pub reached: bool,
    pub mission_first: u32,
    pub mission_selected: u32,
    pub mask: u32,
    pub veto: u8,
    pub counter: i32,
    pub limit: i32,
    pub override_reference: u64,
    pub override_resolved: bool,
    pub planes: i32,
    pub capacity_threshold: i64,
    pub capacity_divisor: i32,
    pub active_count: i32,
    pub base_fuel: i64,
    pub country_modifier: Modifier,
    pub ace: u64,
    pub ace_alive: u8,
    pub ace_assigned_wing: u64,
    pub ace_assigned_planes: Option<i32>,
    pub ace_count: i64,
    pub ace_cap: i64,
    pub ace_modifier: Modifier,
    pub ace_country_modifier: Modifier,
    pub air_weight: i64,
    pub mission_cost: i64,
    pub period: u8,
    pub region: Option<u32>,
    /// Province count is sampled separately for each native slot average.
    pub context_counts: [i32; 24],
    pub context_thresholds: [i64; 24],
}

/// One ordered province occurrence at one slot, before native averaging.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weather {
    pub slot: u8,
    pub selector: i32,
    pub coefficients: [i64; 3],
    pub cell: [i64; 3],
    pub subtract: i64,
    pub scale: i64,
}

/// One current callback's context closure. Reuse is permitted only within
/// this pack; no native pointer or prior-read result survives reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub region: u32,
    pub pointer: usize,
    pub weather: Span,
    pub counts: [i32; 24],
    pub thresholds: [i64; 24],
    pub reason: Option<&'static str>,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("fuel demand address overflow")
}
fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("fuel demand operand unreadable")
}

/// 140F6B7B0's three native return arms; do not cancel fixed5 factors.
pub fn capacity_gate(planes: i32, divisor: i32, threshold: i64) -> bool {
    let denominator = i64::from(divisor).wrapping_mul(100_000);
    if denominator == 0 {
        return threshold < 100_001;
    }
    let planes = i64::from(planes);
    if planes.wrapping_mul(100_000) > -0x7fff_ffff_ffff_ffff {
        threshold
            <= planes
                .wrapping_mul(10_000_000_000)
                .wrapping_div(denominator)
    } else {
        threshold < 0x1_0000_0000
    }
}

/// 14060B790, including the sentinel arm before wrapping multiplication.
fn ace_scale(input: &AirInputs) -> i64 {
    let Some(planes) = input.ace_assigned_planes.filter(|value| *value != 0) else {
        return 100_000;
    };
    if (input.ace_count.wrapping_add(0x7fff_ffff_ffff_fffe) as u64) < 0xffff_ffff_ffff_fffd {
        input
            .ace_count
            .wrapping_mul(100_000)
            .wrapping_div(i64::from(planes).wrapping_mul(100_000))
            .min(input.ace_cap)
    } else {
        0xffff_ffff_i64.min(input.ace_cap)
    }
}

/// Current pool contribution@140F46340, context argument zero to 140F4A700.
/// Native wrapping/truncation is kept separate from widened reporting sums.
pub fn wing_rate(input: &AirInputs, weather: &[Weather]) -> Result<i64, &'static str> {
    if !input.reached {
        return Ok(0);
    }
    let sums = if input.region.is_some() && input.period != 2 {
        Some(context_sums(&input.context_counts, weather)?)
    } else {
        if !weather.is_empty() {
            return Err("fuel demand unreached context has occurrences");
        }
        None
    };
    wing_rate_with_context(input, sums.as_ref())
}

/// Ordered 140F04500 contributions accumulated exactly as 140E0CB70. Decode
/// performs this once per captured region, before its compact operands are
/// shared by the independently decodable country packs.
pub fn context_sums(counts: &[i32; 24], weather: &[Weather]) -> Result<[i64; 24], &'static str> {
    const ONE: i64 = 100_000;
    let mut sums = [0_i64; 24];
    let mut samples = weather.iter();
    for (slot, &count) in counts.iter().enumerate() {
        for _ in 0..count.max(0) {
            let sample = samples
                .next()
                .filter(|sample| usize::from(sample.slot) == slot)
                .ok_or("fuel demand context occurrence missing or out of order")?;
            // 140F04500 evaluates coefficient 0, then 2, then 1.
            let weighted = sample.coefficients[0]
                .wrapping_mul(sample.cell[0])
                .wrapping_div(ONE)
                .wrapping_add(
                    sample.coefficients[2]
                        .wrapping_mul(sample.cell[2])
                        .wrapping_div(ONE),
                )
                .wrapping_add(
                    sample.coefficients[1]
                        .wrapping_mul(sample.cell[1])
                        .wrapping_div(ONE),
                );
            sums[slot] = sums[slot].wrapping_add(
                weighted
                    .wrapping_sub(sample.subtract)
                    .wrapping_mul(sample.scale)
                    .wrapping_div(ONE)
                    .clamp(0, ONE),
            );
        }
    }
    if samples.next().is_some() {
        return Err("fuel demand context has extra occurrences");
    }
    Ok(sums)
}

/// The native slot sums retain wrapping before average, threshold and period
/// selection. They are not rounded averages or an inferred usage factor.
pub fn wing_rate_with_context(
    input: &AirInputs,
    sums: Option<&[i64; 24]>,
) -> Result<i64, &'static str> {
    const ONE: i64 = 100_000;
    if !input.reached {
        return Ok(0);
    }
    let mut factor = ONE;
    if input.ace_alive != 0 {
        let scaled = ace_scale(input)
            .wrapping_mul(input.ace_modifier.effective)
            .wrapping_div(ONE);
        factor = factor.wrapping_add(
            scaled
                .wrapping_mul(ONE.wrapping_add(input.ace_country_modifier.effective))
                .wrapping_div(ONE),
        );
    }
    factor = factor.wrapping_add(input.country_modifier.effective).max(0);
    let fuel = factor.wrapping_mul(input.base_fuel).wrapping_div(ONE);
    let count = if input.mission_selected != 0 {
        input.active_count
    } else {
        input.planes
    };
    let mut term = fuel
        .wrapping_mul(input.air_weight)
        .wrapping_div(ONE)
        .wrapping_mul(i64::from(count).wrapping_mul(input.mission_cost))
        .wrapping_div(ONE);
    if input.region.is_some() && input.period != 2 {
        let sums = sums.ok_or("fuel demand context accumulated operands unavailable")?;
        let mut included = 0_i64;
        for (slot, &sum) in sums.iter().enumerate() {
            let count = input.context_counts[slot];
            // 140E0CB70 does not simplify the product/divisor.
            let average = if count != 0
                && (sum.wrapping_add(0x7fff_ffff_ffff_fffe) as u64) < 0xffff_ffff_ffff_fffd
            {
                sum.wrapping_mul(ONE)
                    .wrapping_div(i64::from(count).wrapping_mul(ONE))
            } else {
                0xffff_ffff
            };
            included += i64::from(input.context_thresholds[slot] < average);
        }
        if input.period == 0 {
            included = 24 - included;
        }
        let fraction = included
            .wrapping_mul(10_000_000_000)
            .wrapping_div(2_400_000);
        term = fraction.wrapping_mul(term).wrapping_div(ONE);
    } else if sums.is_some() {
        return Err("fuel demand unreached context has accumulated operands");
    }
    Ok(term)
}

/// 1401C6B30, tooltip's 24-hour projection of a current fixed5 rate.
pub fn daily_required(rate: i64) -> i64 {
    rate.wrapping_mul(2_400_000).wrapping_div(100_000)
}

#[cfg(test)]
mod tests;
