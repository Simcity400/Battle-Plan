
use crate::codec::invalid_static as invalid;
pub use crate::rows::compatibility::DISPLAY_PRECISION_FORMAT;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    TowardZero,
    TowardZeroNonzero,
    HalfAwayFromZero,
    HalfAwayFromZeroNonzero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    raw: i64,
    scale: i64,
    rounding: Rounding,
    elements: u64,
}

const SENTINELS: [i64; 3] = [i64::MIN, i64::MIN + 1, i64::MAX];

const fn sentinel(value: i64) -> bool {
    value == SENTINELS[0] || value == SENTINELS[1] || value == SENTINELS[2]
}

pub const fn is_sentinel(value: i64) -> bool {
    sentinel(value)
}

impl Step {
    pub const fn fixed5(raw: i64, rounding: Rounding) -> Self {
        assert!(raw > 1 && (100_000 % raw == 0 || raw % 100_000 == 0));
        Self {
            raw,
            scale: 100_000,
            rounding,
            elements: u64::MAX,
        }
    }

    pub const fn only(self, elements: &[usize]) -> Self {
        let mut mask = 0;
        let mut index = 0;
        while index < elements.len() {
            assert!(elements[index] < 64);
            mask |= 1 << elements[index];
            index += 1;
        }
        Self {
            elements: mask,
            ..self
        }
    }

    pub const fn applies(self, index: usize) -> bool {
        index < 64 && self.elements & (1 << index) != 0
    }

    pub const fn whole(self) -> Self {
        Self {
            elements: u64::MAX,
            ..self
        }
    }

    pub const fn raw(self) -> i64 {
        self.raw
    }

    pub const fn rounding(self) -> Rounding {
        self.rounding
    }

    #[inline]
    pub const fn round(self, value: i64) -> i64 {
        if sentinel(value) {
            value
        } else {
            self.units(value) * self.raw
        }
    }

    pub const fn toward_zero_bounds(self, value: i64) -> [i64; 2] {
        if sentinel(value) || value % self.raw != 0 {
            return [value, value];
        }
        let span = self.raw - 1;
        if value > 0 {
            let high = value.saturating_add(span);
            [value, if high == i64::MAX { i64::MAX - 1 } else { high }]
        } else if value < 0 {
            let low = value.saturating_sub(span);
            [if low <= i64::MIN + 1 { i64::MIN + 2 } else { low }, value]
        } else {
            [-span, span]
        }
    }

    #[inline]
    pub const fn units(self, value: i64) -> i64 {
        if sentinel(value) {
            return value;
        }
        let raw = self.raw as u64;
        let magnitude = value.unsigned_abs();
        let truncated = magnitude / raw;
        let mut quotient = match self.rounding {
            Rounding::TowardZero => truncated,
            Rounding::TowardZeroNonzero => {
                if magnitude != 0 && magnitude < raw {
                    1
                } else {
                    truncated
                }
            }
            Rounding::HalfAwayFromZero => truncated + (2 * (magnitude % raw) >= raw) as u64,
            Rounding::HalfAwayFromZeroNonzero => {
                let rounded = truncated + (2 * (magnitude % raw) >= raw) as u64;
                if magnitude != 0 && rounded == 0 {
                    1
                } else {
                    rounded
                }
            }
        };
        if quotient * raw > i64::MAX as u64 {
            quotient -= 1;
        }
        let quotient = quotient as i64;
        if value < 0 { -quotient } else { quotient }
    }

    pub fn value(self, units: i64) -> io::Result<i64> {
        if sentinel(units) {
            return Ok(units);
        }
        units
            .checked_mul(self.raw)
            .filter(|value| !sentinel(*value))
            .ok_or_else(|| invalid("display step value exceeds native range"))
    }

    pub fn describe(self) -> String {
        let unit = match self.scale {
            100_000 => "fixed5",
            32_768 => "Q15",
            _ => "integer",
        };
        let rounding = match self.rounding {
            Rounding::TowardZero => "toward zero",
            Rounding::TowardZeroNonzero => "toward zero, nonzero kept",
            Rounding::HalfAwayFromZero => "half away from zero",
            Rounding::HalfAwayFromZeroNonzero => "half away from zero, nonzero kept",
        };
        let whole = self.raw / self.scale;
        let mut digits = String::new();
        let mut rest = self.raw % self.scale;
        if rest != 0 {
            digits.push('.');
            while rest != 0 {
                rest *= 10;
                digits.push(char::from(b'0' + (rest / self.scale) as u8));
                rest %= self.scale;
            }
        }
        let mut text = format!("{whole}{digits} ({unit}, {rounding})");
        if self.elements != u64::MAX {
            let elements: Vec<String> = (0..64)
                .filter(|&index| self.applies(index))
                .map(|index| index.to_string())
                .collect();
            text.push_str(&format!(" for elements {}", elements.join(", ")));
        }
        text
    }
}

pub const fn stepped_format(format: u16) -> bool {
    format >= DISPLAY_PRECISION_FORMAT
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENTH: Step = Step::fixed5(10_000, Rounding::TowardZero);
    const NONZERO_TENTH: Step = Step::fixed5(10_000, Rounding::TowardZeroNonzero);
    const HALF_TENTH: Step = Step::fixed5(10_000, Rounding::HalfAwayFromZero);

    #[test]
    fn toward_zero_matches_the_fixed5_formatter_at_every_boundary() {
        for (value, stored) in [
            (0, 0),
            (1, 0),
            (9_999, 0),
            (10_000, 10_000),
            (129_999, 120_000),
            (130_000, 130_000),
            (-1, 0),
            (-9_999, 0),
            (-10_000, -10_000),
            (-129_999, -120_000),
            (i64::MAX - 1, (i64::MAX - 1) / 10_000 * 10_000),
            (i64::MIN + 2, (i64::MIN + 2) / 10_000 * 10_000),
        ] {
            assert_eq!(TENTH.round(value), stored, "{value}");
            assert_eq!(TENTH.units(value) * 10_000, stored, "{value}");
            assert_eq!(TENTH.round(stored), stored, "idempotent at {value}");
        }
    }

    #[test]
    fn nonzero_values_keep_one_step_and_their_sign() {
        for (value, stored) in [
            (0, 0),
            (1, 10_000),
            (9_999, 10_000),
            (10_000, 10_000),
            (19_999, 10_000),
            (-1, -10_000),
            (-9_999, -10_000),
            (-19_999, -10_000),
            (-20_000, -20_000),
        ] {
            assert_eq!(NONZERO_TENTH.round(value), stored, "{value}");
            assert_eq!(NONZERO_TENTH.round(stored), stored, "idempotent at {value}");
        }
    }

    #[test]
    fn half_away_from_zero_rounds_halves_outward_and_stays_in_range() {
        for (value, stored) in [
            (0, 0),
            (4_999, 0),
            (5_000, 10_000),
            (-4_999, 0),
            (-5_000, -10_000),
            (14_999, 10_000),
            (15_000, 20_000),
            (-15_000, -20_000),
            (i64::MAX - 1, (i64::MAX - 1) / 10_000 * 10_000),
            (i64::MIN + 2, (i64::MIN + 2) / 10_000 * 10_000),
        ] {
            assert_eq!(HALF_TENTH.round(value), stored, "{value}");
        }
    }

    #[test]
    fn half_away_nonzero_keeps_one_step_below_half_a_step_and_its_sign() {
        const NONZERO_HUNDREDTH: Step = Step::fixed5(1_000, Rounding::HalfAwayFromZeroNonzero);
        for (value, stored) in [
            (0, 0),
            (1, 1_000),
            (499, 1_000),
            (500, 1_000),
            (1_499, 1_000),
            (1_500, 2_000),
            (-1, -1_000),
            (-499, -1_000),
            (-1_499, -1_000),
            (-1_500, -2_000),
            (i64::MAX - 1, (i64::MAX - 1) / 1_000 * 1_000),
            (i64::MIN + 2, (i64::MIN + 2) / 1_000 * 1_000),
        ] {
            assert_eq!(NONZERO_HUNDREDTH.round(value), stored, "{value}");
            assert_eq!(
                NONZERO_HUNDREDTH.round(stored),
                stored,
                "idempotent at {value}"
            );
        }
        assert_eq!(
            NONZERO_HUNDREDTH.describe(),
            "0.01 (fixed5, half away from zero, nonzero kept)"
        );
    }

    #[test]
    fn sentinel_words_pass_through_exactly() {
        for step in [TENTH, NONZERO_TENTH, HALF_TENTH] {
            for value in SENTINELS {
                assert_eq!(step.round(value), value);
                assert_eq!(step.units(value), value);
                assert_eq!(step.value(value).unwrap(), value);
            }
        }
    }

    #[test]
    fn units_multiply_back_and_refuse_overflow() {
        assert_eq!(TENTH.value(TENTH.units(-129_999)).unwrap(), -120_000);
        assert_eq!(
            TENTH.value(i64::MAX / 10_000).unwrap(),
            i64::MAX / 10_000 * 10_000
        );
        assert!(TENTH.value(i64::MAX / 10_000 + 1).is_err());
        assert!(TENTH.value(i64::MIN + 2).is_err());
    }

}
