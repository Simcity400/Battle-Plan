//! `CNavalMissionUnitItem` ship and capital-count text at partial naval intel.
//!
//! `14197EAC0` chooses the low/medium NNavy defines from the *first* grouped
//! task force's viewer-derived visibility byte. `14197A4C0` formats the bounds
//! produced by `14197A330`; its deterministic jitter uses that task force's
//! simulation handle ID, not a saved display string or recorder-local RNG.

use std::io;

/// The `NNavy` intel-level count ranges; each native loader clamps to 0..=100.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Defines {
    pub low_half_range_percent: i32,
    pub low_min_ships: i32,
    pub low_min_capitals: i32,
    pub medium_half_range_percent: i32,
    pub medium_min_ships: i32,
    pub medium_min_capitals: i32,
}

impl Defines {
    pub fn read(defines: &installed_content::defines::Defines) -> io::Result<Self> {
        let value = |name| {
            defines
                .int32("NNavy", name)
                .map(|value| value.clamp(0, 100))
        };
        Ok(Self {
            low_half_range_percent: value("INTEL_LEVEL_LOW_HALF_RANGE_PERCENTAGE")?,
            low_min_ships: value("INTEL_LEVEL_LOW_HALF_RANGE_MIN_SHIPS")?,
            low_min_capitals: value("INTEL_LEVEL_LOW_HALF_RANGE_MIN_CAPITALS")?,
            medium_half_range_percent: value("INTEL_LEVEL_MEDIUM_HALF_RANGE_PERCENTAGE")?,
            medium_min_ships: value("INTEL_LEVEL_MEDIUM_HALF_RANGE_MIN_SHIPS")?,
            medium_min_capitals: value("INTEL_LEVEL_MEDIUM_HALF_RANGE_MIN_CAPITALS")?,
        })
    }

    /// The shipped values, for tests only.
    #[cfg(test)]
    pub const VANILLA: Self = Self {
        low_half_range_percent: 10,
        low_min_ships: 3,
        low_min_capitals: 1,
        medium_half_range_percent: 5,
        medium_min_ships: 1,
        medium_min_capitals: 1,
    };
}

/// Native count text for one naval mission counter child (`14197EAC0`). `total_ships` is the
/// complete task-force ship-roster sum (`BuildNavalCounterData@14197B4F0` `+0x50`), including
/// partly visible members; the `capital` child's `total` is the capital sum (`+0x54`). The game
/// formats whatever its signed arithmetic gives, so every input words a text.
pub(crate) fn format_count(
    total: i32,
    first_force_handle: u64,
    visibility: u16,
    capital: bool,
    total_ships: i32,
    defines: &Defines,
) -> String {
    if visibility < 100 {
        return "?".to_owned();
    }
    if visibility >= 250 {
        // `CInstantTextBox::SetInt32Text@1422B5AC0`.
        return total.to_string();
    }
    let medium = visibility >= 150; // `0x95 < byte` in `14197EAC0`.
    let (percent, minimum) = match (medium, capital) {
        (false, false) => (defines.low_half_range_percent, defines.low_min_ships),
        (false, true) => (defines.low_half_range_percent, defines.low_min_capitals),
        (true, false) => (defines.medium_half_range_percent, defines.medium_min_ships),
        (true, true) => (
            defines.medium_half_range_percent,
            if total_ships > 3 {
                defines.medium_min_capitals
            } else {
                0
            },
        ),
    };
    let seed = (first_force_handle >> 32) as u32; // `CTaskForce/CUnit+0x1c`.
    let (lower, upper) = count_range(total, percent, minimum, seed);
    // `FormatApproximateNavalIntelCount@14197A4C0`: equal bounds, a range below 100, or ">100";
    // it formats reversed bounds as they are.
    if lower == upper {
        lower.to_string()
    } else if upper < 100 {
        format!("{lower}-{upper}")
    } else {
        ">100".to_owned()
    }
}

/// `FormatNavalIntelCountRange@14197A330`: signed 32-bit count arithmetic, which wraps, and a
/// wrapping 32-bit seed hash. The jitter modulus is a signed `int` (`2h + 1` for an even half
/// range `h`, `2h` for an odd one), never zero; a half range past `2^30` makes it negative, and
/// the game's signed remainder takes it as it is.
pub(crate) fn count_range(total: i32, percent: i32, minimum: i32, seed: u32) -> (i32, i32) {
    let half_range = (total.wrapping_mul(percent) / 100).max(minimum);
    let jitter = if half_range > 0 {
        let modulus = half_range
            .wrapping_mul(2)
            .wrapping_add(i32::from(half_range & 1 == 0));
        let a = seed.wrapping_mul(0x7eb8_da44);
        let a = a ^ (a >> 8);
        let a = mix(a.wrapping_add(0x10d8_365e)).wrapping_mul(0x6d22_f31b);
        let b = seed.wrapping_mul(0x205a_d647);
        let b = b ^ (b >> 8);
        let b = mix(b.wrapping_add(0xb4f3_c315)).wrapping_mul(0x387a_9f3a);
        let combined = (a ^ (a >> 8)).wrapping_sub((b ^ (b >> 8)).wrapping_mul(0x4ad6_85b3));
        // Both hashes are below 2^31, so the signed remainder cannot overflow.
        let first = (jitter_hash(combined) as i32) % modulus;
        let second = (jitter_hash(combined.wrapping_add(0xb529_7a4d)) as i32) % modulus;
        first
            .wrapping_add(second)
            .wrapping_sub(modulus)
            .wrapping_add(1)
            / 2
    } else {
        0
    };
    let center = total.wrapping_add(jitter).max(0);
    let lower = center.wrapping_sub(half_range).max(0);
    let upper = center.wrapping_add(half_range).max(minimum.wrapping_mul(2));
    (lower, upper)
}

fn mix(value: u32) -> u32 {
    (value << 8) ^ value
}

fn jitter_hash(mut value: u32) -> u32 {
    value = (value ^ (value >> 8)).wrapping_add(0x68e3_1da4);
    value = mix(value).wrapping_mul(0x1b56_c4e9);
    value = (value ^ (value >> 8)).wrapping_mul(0x92d6_8ca2);
    (value ^ (value >> 8)) & 0x7fff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla() -> Defines {
        Defines::VANILLA
    }

    const FORCE: u64 = (7_u64 << 32) | 61;

    #[test]
    fn visibility_boundaries_choose_unknown_low_medium_and_exact() {
        let defines = vanilla();
        assert_eq!(format_count(15, FORCE, 99, false, 15, &defines), "?");
        assert_eq!(format_count(15, FORCE, 100, false, 15, &defines), "12-18");
        assert_eq!(format_count(15, FORCE, 149, false, 15, &defines), "12-18");
        assert_eq!(format_count(15, FORCE, 150, false, 15, &defines), "14-16");
        assert_eq!(format_count(15, FORCE, 249, false, 15, &defines), "14-16");
        assert_eq!(format_count(15, FORCE, 250, false, 15, &defines), "15");
    }

    #[test]
    fn capital_minimum_switches_only_when_total_ships_exceed_three() {
        let defines = vanilla();
        assert_eq!(format_count(2, FORCE, 149, true, 3, &defines), "1-3");
        assert_eq!(format_count(2, FORCE, 150, true, 3, &defines), "2");
        assert_eq!(format_count(2, FORCE, 150, true, 4, &defines), "1-3");
    }

    #[test]
    fn range_hash_uses_handle_id_with_wrapping_unsigned_multiplication() {
        let defines = vanilla();
        assert_eq!(
            format_count(
                15,
                (u64::from(u32::MAX) << 32) | 61,
                100,
                false,
                15,
                &defines
            ),
            "10-16"
        );
        assert_eq!(format_count(99, FORCE, 100, false, 99, &defines), ">100");
    }

    #[test]
    fn matching_content_defines_control_range_without_capture_operands() {
        let mut defines = vanilla();
        defines.low_half_range_percent = 0;
        defines.low_min_ships = 0;
        assert_eq!(format_count(15, FORCE, 100, false, 15, &defines), "15");
        defines.low_min_ships = 4;
        assert_eq!(format_count(15, FORCE, 100, false, 15, &defines), "11-19");
    }

    /// The game words whatever its arithmetic gives: a negative exact count, more capitals
    /// than ships, a negative ship total, bounds the wrapping arithmetic reverses and a
    /// negative jitter modulus are all formatted, none refused.
    #[test]
    fn every_count_words_a_text_as_the_game_formats_it() {
        let defines = vanilla();
        assert_eq!(format_count(-1, FORCE, 250, false, 0, &defines), "-1");
        assert_eq!(format_count(5, FORCE, 250, true, 4, &defines), "5");
        assert_eq!(
            format_count(1, FORCE, 100, false, -1, &defines),
            format_count(1, FORCE, 100, false, 15, &defines)
        );
        // `center + h` wraps negative, so the upper bound falls to twice the minimum (80)
        // while the lower stays near `i32::MAX`: reversed, and formatted as a range.
        let mut modded = vanilla();
        modded.low_half_range_percent = 0;
        modded.low_min_ships = 40;
        let total = i32::MAX - 10;
        let (seed, (lower, upper)) = (0..256_u32)
            .map(|seed| (seed, count_range(total, 0, 40, seed)))
            .find(|(_, (lower, upper))| lower > upper)
            .expect("a seed whose jitter keeps the centre high");
        assert_eq!(upper, 80);
        assert_eq!(
            format_count(total, u64::from(seed) << 32, 100, false, total, &modded),
            format!("{lower}-80")
        );
        // A half range past 2^30 makes the signed modulus negative; the game's remainder takes
        // it as it is.
        let _ = count_range(i32::MAX, 100, i32::MAX / 2 + 2, 7);
    }
}
