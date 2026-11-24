use crate::rows::ClockRow;

const SCALE: i64 = 100_000;
const Q15: i64 = 1 << 15;
const DIVISION_FALLBACK: i64 = u32::MAX as i64;
const JITTER_MINIMUM: i64 = 20_000;
const HASH_ORIGIN: u32 = 0x5ea6_ba9f;
const HASH_STEP: u32 = 0xb529_7a4d;
const HASH_OFFSET: u32 = 0x68e3_1da4;
const HASH_MIX: u32 = 0x1b56_c4e9;
const HASH_FINAL: u32 = 0x92d6_8ca2;

fn finite(value: i64) -> bool {
    value != i64::MIN && value != i64::MIN + 1 && value != i64::MAX
}
fn divide(value: i64, denominator: i64) -> Result<i64, &'static str> {
    value
        .checked_div(denominator)
        .ok_or("intel estimate signed division overflow")
}
pub fn normalized(value: i64, maximum: i64) -> Result<i64, &'static str> {
    if maximum == 0 || !finite(value) {
        Ok(DIVISION_FALLBACK)
    } else {
        divide(value.wrapping_mul(SCALE), maximum)
    }
}
fn fixed_to_q15(value: i64) -> i64 {
    (value % SCALE).wrapping_mul(Q15) / SCALE + (value / SCALE).wrapping_mul(Q15)
}
fn square_root(value: i64) -> Result<i64, &'static str> {
    if value < 2 {
        return Ok(0);
    }
    let mut guess = divide(value.wrapping_mul(SCALE), 2 * SCALE)?;
    let tolerance = divide(value.wrapping_mul(1000), SCALE)?;
    loop {
        let residual = divide(guess.wrapping_mul(guess), SCALE)?.wrapping_sub(value);
        let denominator = guess.wrapping_mul(2);
        let adjustment = if denominator == 0 || !finite(residual) {
            DIVISION_FALLBACK
        } else {
            divide(residual.wrapping_mul(SCALE), denominator)?
        };
        guess = guess.wrapping_sub(adjustment);
        if adjustment == 0 || (-tolerance..=tolerance).contains(&residual) {
            return Ok(guess);
        }
    }
}
fn month(clock: &ClockRow) -> i32 {
    let elapsed = clock.hour.wrapping_sub(game_model::calendar::HOUR_EPOCH);
    let mut day = (elapsed / 24) % 365;
    let mut month = 0;
    for length in game_model::calendar::MONTH_LENGTHS {
        day -= length;
        if day < 0 {
            break;
        }
        month += 1;
    }
    (elapsed / (365 * 24)).wrapping_mul(12).wrapping_add(month)
}
fn draw(seed: u32, upper: bool) -> i64 {
    let value = HASH_ORIGIN
        .wrapping_add(seed.wrapping_mul(HASH_STEP).wrapping_sub(seed))
        .wrapping_add(if upper { HASH_STEP } else { 0 });
    let value = (value ^ (value >> 8)).wrapping_add(HASH_OFFSET);
    let value = (value ^ value.wrapping_shl(8)).wrapping_mul(HASH_MIX);
    let value = (value ^ (value >> 8)).wrapping_mul(HASH_FINAL);
    i64::from(((value ^ (value >> 8)) & 0x7fff_ffff) % SCALE as u32)
}
fn jitter(seed: u32, upper: bool) -> Result<i64, &'static str> {
    let root = square_root(draw(seed, upper))?;
    Ok(fixed_to_q15(
        divide((SCALE - root).wrapping_mul(JITTER_MINIMUM), SCALE)?.wrapping_add(root),
    ))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameters {
    pub minimum_intel: i64,
    pub exact_intel: i64,
    pub lowest_spread: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub available: bool,
    pub lower_q15: i64,
    pub upper_q15: i64,
}
pub fn needs_jitter(intel: i64, minimum: i64, maximum: i64) -> Result<bool, &'static str> {
    let numerator = intel.wrapping_sub(minimum);
    let denominator = maximum.wrapping_sub(minimum);
    Ok(denominator != 0
        && finite(numerator)
        && divide(numerator.wrapping_mul(SCALE), denominator)? < SCALE)
}
pub fn range(
    value_q15: i64,
    intel: i64,
    parameters: Parameters,
    clock: &ClockRow,
    kind_salt: i32,
) -> Result<Range, &'static str> {
    let available = parameters.minimum_intel <= intel;
    let exact = |value| Range {
        available,
        lower_q15: value,
        upper_q15: value,
    };
    if value_q15 <= 0 {
        return Ok(exact(0));
    }
    let numerator = intel.wrapping_sub(parameters.minimum_intel);
    let denominator = parameters
        .exact_intel
        .wrapping_sub(parameters.minimum_intel);
    if denominator == 0 || !finite(numerator) {
        return Ok(exact(value_q15));
    }
    let progress = divide(numerator.wrapping_mul(SCALE), denominator)?;
    if progress >= SCALE {
        return Ok(exact(value_q15));
    }
    let fraction = divide(
        (SCALE - progress.max(0)).wrapping_mul(parameters.lowest_spread),
        SCALE,
    )?;
    let spread = (value_q15.wrapping_mul(fixed_to_q15(fraction)) >> 15).wrapping_abs();
    let seed = clock
        .seed
        .wrapping_add(month(clock))
        .wrapping_add((value_q15 >> 15) as i32)
        .wrapping_add(kind_salt) as u32;
    let lower = value_q15
        .wrapping_sub(spread.wrapping_mul(jitter(seed, false)?) >> 15)
        .min(value_q15);
    let upper = value_q15
        .wrapping_add(spread.wrapping_mul(jitter(seed, true)?) >> 15)
        .max(value_q15)
        .max(lower);
    Ok(Range {
        available,
        lower_q15: lower.max(Q15),
        upper_q15: upper.max(Q15),
    })
}
pub fn count_bounds(range: Range) -> (i32, i32) {
    (
        (range.lower_q15.wrapping_add(Q15 / 2) >> 15) as i32,
        (range.upper_q15.wrapping_add(Q15 / 2) >> 15) as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_retains_positive_wide_fallback_and_signed_finite_arithmetic() {
        for value in [0, 1, -1, i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(normalized(value, 0), Ok(4294967295));
        }
        for value in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(normalized(value, 100_000), Ok(4294967295));
        }
        assert_eq!(normalized(50_000, -100_000), Ok(-50_000));
        assert_eq!(
            normalized(i64::MAX - 1, 70_000),
            Ok((i64::MAX - 1).wrapping_mul(SCALE) / 70_000)
        );
    }
    #[test]
    fn ranges_use_recorded_seed_month_content_and_native_zero_and_rounding_rules() {
        let p = Parameters {
            minimum_intel: 25_000,
            exact_intel: 75_000,
            lowest_spread: 100_000,
        };
        let clock = ClockRow {
            hour: game_model::calendar::HOUR_EPOCH + 1936 * 8760,
            seed: 123,
            ..Default::default()
        };
        assert_eq!(
            range(100 * Q15, 75_000, p, &clock, 1).unwrap(),
            Range {
                available: true,
                lower_q15: 100 * Q15,
                upper_q15: 100 * Q15
            }
        );
        assert!(!range(0, 24_999, p, &clock, 1).unwrap().available);
        assert!(range(0, 25_000, p, &clock, 1).unwrap().available);
        let first = range(100 * Q15, 25_000, p, &clock, 1).unwrap();
        assert_eq!(first, range(100 * Q15, 25_000, p, &clock, 1).unwrap());
        assert!(first.lower_q15 < 100 * Q15 && first.upper_q15 > 100 * Q15);
        assert_ne!(
            first,
            range(
                100 * Q15,
                25_000,
                p,
                &ClockRow {
                    seed: 999,
                    ..clock
                },
                1
            )
            .unwrap()
        );
        assert_ne!(
            first,
            range(
                100 * Q15,
                25_000,
                p,
                &ClockRow {
                    hour: clock.hour + 31 * 24,
                    ..clock
                },
                1
            )
            .unwrap()
        );
        assert_eq!(
            first,
            range(
                100 * Q15,
                25_000,
                p,
                &ClockRow {
                    hour: clock.hour + 24,
                    ..clock
                },
                1
            )
            .unwrap()
        );
        assert_eq!(
            count_bounds(Range {
                available: true,
                lower_q15: Q15 + Q15 / 2,
                upper_q15: 2 * Q15
            }),
            (2, 2)
        );
        assert_eq!(range(Q15 / 2, 25_000, p, &clock, 1).unwrap().lower_q15, Q15);
        assert_eq!(range(0, -1, p, &clock, 1).unwrap().lower_q15, 0);
        assert_eq!(range(-Q15, 100_000, p, &clock, 1).unwrap().upper_q15, 0);
        assert_eq!(month(&clock), 1936 * 12);
    }
}
