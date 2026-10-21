//! Display steps: drifting values recorded at the precision the game shows.
//!
//! A column with a [`Step`] holds whole multiples of the step, rounded the way
//! the game's own formatter rounds the value where it shows it, so a viewer
//! formatting the stored value with the same formatter prints the game's text.
//! The wire holds the value divided by the step (its *units*): snapshots keep
//! the column's fixed width and deltas its zigzag varint, so smaller numbers
//! compress better without a layout change. Decoders multiply back, and
//! consumers read native units. A recording before
//! [`DISPLAY_PRECISION_FORMAT`] holds exact values in these columns and
//! decodes them unscaled.
//!
//! Writers round twice: capture rounds each stepped field once the row is
//! converted ([`crate::rows::TableRows::round_to_display`]), so rows that
//! changed only below the step compare equal, and the encoder rounds again
//! (idempotently) before dividing, so every encoding path stores a multiple.
//!
//! The extreme words `i64::MIN`, `i64::MIN + 1` and `i64::MAX`, which native
//! code and the viewer use as sentinels, are stored exactly: their units are
//! the same words, which no rounded value reaches.
//!
//! Values that advance in fixed steps or count things (identities, handles,
//! counters, dates, seeds, route progress), values whose exact digits a
//! discrete decision reads (a glyph threshold, a content define comparison),
//! and values the game shows at full precision are never stepped. Derived
//! displays (combat progress, strength percentages, damage estimates) read the
//! stepped values and can differ from the game in their last digit at
//! boundaries, by owner decision 2026-10-11. The reasoning for each stepped
//! field sits beside its declaration.

use crate::codec::invalid_static as invalid;
pub use crate::rows::compatibility::DISPLAY_PRECISION_FORMAT;
use std::io;

/// How a value becomes a multiple of its step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// Drop the remainder: the fixed5 formatter prints the integer quotient and remainder
    /// of the magnitude, truncating discarded digits.
    TowardZero,
    /// [`Self::TowardZero`], except that a nonzero value below one step keeps
    /// one step with its sign, so the zero tests, signs and sign colours a
    /// consumer applies read the same; such a value shows that one step.
    TowardZeroNonzero,
    /// Round to the nearest multiple, a half step away from zero.
    HalfAwayFromZero,
    /// [`Self::HalfAwayFromZero`], except that a nonzero value below half a
    /// step keeps one step with its sign, so the zero tests and signs a
    /// consumer applies read the same; such a value shows that one step.
    HalfAwayFromZeroNonzero,
}

/// A column's display precision: `raw` native units per step, over `scale`
/// native units per displayed unit (100000 for fixed5, 32768 for Q15, 1 for
/// plain integers). A step can apply to some elements of an array only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    raw: i64,
    scale: i64,
    rounding: Rounding,
    /// Bit `i` set: element `i` of an array or list takes the step.
    elements: u64,
}

const SENTINELS: [i64; 3] = [i64::MIN, i64::MIN + 1, i64::MAX];

const fn sentinel(value: i64) -> bool {
    value == SENTINELS[0] || value == SENTINELS[1] || value == SENTINELS[2]
}

impl Step {
    /// A fixed5 step of `raw` hundred-thousandths.
    pub const fn fixed5(raw: i64, rounding: Rounding) -> Self {
        assert!(raw > 1 && (100_000 % raw == 0 || raw % 100_000 == 0));
        Self {
            raw,
            scale: 100_000,
            rounding,
            elements: u64::MAX,
        }
    }

    /// This step for the listed array elements only; the others stay exact.
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

    /// Whether element `index` of an array or list takes this step.
    pub const fn applies(self, index: usize) -> bool {
        index < 64 && self.elements & (1 << index) != 0
    }

    /// This step for every element of a nested value.
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

    /// The multiple of the step `value` is stored as; a sentinel word stays.
    #[inline]
    pub const fn round(self, value: i64) -> i64 {
        if sentinel(value) {
            value
        } else {
            self.units(value) * self.raw
        }
    }

    /// `value` rounded and divided by the step; a sentinel word stays itself.
    /// A multiple outside i64 keeps the next multiple toward zero.
    #[inline]
    pub const fn units(self, value: i64) -> i64 {
        if sentinel(value) {
            return value;
        }
        // Sentinel exclusion bounds the magnitude by i64::MAX - 1. A rounded
        // product is at most magnitude + raw, so even the overflow check fits
        // in u64; no wide division is needed for an ordinary display value.
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

    /// The native value of `units` steps; a product outside i64 is malformed.
    pub fn value(self, units: i64) -> io::Result<i64> {
        if sentinel(units) {
            return Ok(units);
        }
        units
            .checked_mul(self.raw)
            .filter(|value| !sentinel(*value))
            .ok_or_else(|| invalid("display step value exceeds native range"))
    }

    /// The precision as the report names it, e.g. `0.1 (fixed5, toward zero)`.
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

/// Whether a value decoded in `format` holds step units.
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
        // The fixed5 formatter prints the absolute integer part, then the remainder's
        // leading digits: 1.29999 shows "1.2", -1.29999 shows "-1.2".
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
        // The two-decimal formatter rounds half away from zero; a nonzero value it would
        // print as zero keeps one step instead.
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

    #[test]
    fn steps_name_their_precision_and_elements() {
        assert_eq!(TENTH.describe(), "0.1 (fixed5, toward zero)");
        assert_eq!(
            Step::fixed5(1_000, Rounding::HalfAwayFromZero).describe(),
            "0.01 (fixed5, half away from zero)"
        );
        assert_eq!(
            Step::fixed5(100_000, Rounding::TowardZero).describe(),
            "1 (fixed5, toward zero)"
        );
        let some = TENTH.only(&[0, 1, 3]);
        assert!(some.applies(0) && some.applies(3) && !some.applies(2));
        assert!(some.whole().applies(2));
        assert_eq!(
            some.describe(),
            "0.1 (fixed5, toward zero) for elements 0, 1, 3"
        );
    }
}
