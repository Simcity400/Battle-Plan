
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Defines {
    pub low_half_range_percent: i32,
    pub low_min_ships: i32,
    pub low_min_capitals: i32,
    pub medium_half_range_percent: i32,
    pub medium_min_ships: i32,
    pub medium_min_capitals: i32,
    pub port_detail: Option<[i64; 5]>,
}

impl Defines {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
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
            port_detail: defines
                .fixed5_array("NIntel", "NAVY_MAPICON_NAVAL_PORT_VISIBILITY_DETAIL_THRESHOLDS")
                .ok()
                .and_then(|values| values.try_into().ok()),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const FIXTURE: Self = Self {
        low_half_range_percent: 14,
        low_min_ships: 5,
        low_min_capitals: 2,
        medium_half_range_percent: 7,
        medium_min_ships: 2,
        medium_min_capitals: 2,
        port_detail: Some([0, 15_000, 35_000, 65_000, 85_000]),
    };
}

impl Defines {
    pub fn port_detail_level(&self, ratio: i64) -> Option<u32> {
        let thresholds = self.port_detail?;
        Some((0..5).rev().find(|&at| thresholds[at] <= ratio).map_or(0, |at| at as u32))
    }
}

const COUNT_INTEL: u16 = 100;

pub fn format_count(
    total: i32,
    first_force_handle: u64,
    visibility: u16,
    capital: bool,
    total_ships: i32,
    defines: &Defines,
) -> String {
    if visibility < COUNT_INTEL {
        return "?".to_owned();
    }
    if visibility >= 250 {
        return total.to_string();
    }
    let medium = visibility >= 150;
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
    let seed = (first_force_handle >> 32) as u32;
    let (lower, upper) = count_range(total, percent, minimum, seed);
    if lower == upper {
        lower.to_string()
    } else if upper < 100 {
        format!("{lower}-{upper}")
    } else {
        ">100".to_owned()
    }
}

pub fn format_recorded_count(
    total: Option<i32>,
    first_force_handle: u64,
    visibility: u16,
    capital: bool,
    total_ships: Option<i32>,
    defines: &Defines,
) -> Option<String> {
    if visibility < COUNT_INTEL {
        return Some("?".to_owned());
    }
    Some(format_count(
        total?,
        first_force_handle,
        visibility,
        capital,
        total_ships?,
        defines,
    ))
}

pub fn count_range(total: i32, percent: i32, minimum: i32, seed: u32) -> (i32, i32) {
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

    fn fixture() -> Defines {
        Defines::FIXTURE
    }

    const FORCE: u64 = (7_u64 << 32) | 61;

    fn range_text(total: i32, percent: i32, minimum: i32, handle: u64) -> String {
        let (lower, upper) = count_range(total, percent, minimum, (handle >> 32) as u32);
        if lower == upper {
            lower.to_string()
        } else {
            format!("{lower}-{upper}")
        }
    }

    #[test]
    fn visibility_boundaries_choose_unknown_low_medium_and_exact() {
        let defines = fixture();
        let low = range_text(
            15,
            defines.low_half_range_percent,
            defines.low_min_ships,
            FORCE,
        );
        let medium = range_text(
            15,
            defines.medium_half_range_percent,
            defines.medium_min_ships,
            FORCE,
        );
        assert_ne!(low, medium);
        assert_eq!(format_count(15, FORCE, 99, false, 15, &defines), "?");
        assert_eq!(format_count(15, FORCE, 100, false, 15, &defines), low);
        assert_eq!(format_count(15, FORCE, 149, false, 15, &defines), low);
        assert_eq!(format_count(15, FORCE, 150, false, 15, &defines), medium);
        assert_eq!(format_count(15, FORCE, 249, false, 15, &defines), medium);
        assert_eq!(format_count(15, FORCE, 250, false, 15, &defines), "15");
    }

    #[test]
    fn unrecorded_counts_are_unknown_only_where_the_text_reads_them() {
        let defines = fixture();
        let text = |total, visibility, ships| {
            format_recorded_count(total, FORCE, visibility, false, ships, &defines)
        };
        assert_eq!(text(None, 99, None).as_deref(), Some("?"));
        for visibility in [100, 150, 250] {
            assert_eq!(text(None, visibility, Some(15)), None);
            assert_eq!(text(Some(15), visibility, None), None);
            assert_eq!(
                text(Some(15), visibility, Some(15)),
                Some(format_count(15, FORCE, visibility, false, 15, &defines))
            );
        }
    }

    #[test]
    fn capital_minimum_switches_only_when_total_ships_exceed_three() {
        let defines = fixture();
        let low_cap = range_text(
            2,
            defines.low_half_range_percent,
            defines.low_min_capitals,
            FORCE,
        );
        let med_cap = range_text(
            2,
            defines.medium_half_range_percent,
            defines.medium_min_capitals,
            FORCE,
        );
        assert_eq!(format_count(2, FORCE, 149, true, 3, &defines), low_cap);
        assert_eq!(format_count(2, FORCE, 150, true, 3, &defines), "2");
        assert_eq!(format_count(2, FORCE, 150, true, 4, &defines), med_cap);
    }

    #[test]
    fn range_hash_uses_handle_id_with_wrapping_unsigned_multiplication() {
        let defines = fixture();
        assert_eq!(
            format_count(
                15,
                (u64::from(u32::MAX) << 32) | 61,
                100,
                false,
                15,
                &defines
            ),
            "11-21"
        );
        assert_eq!(format_count(99, FORCE, 100, false, 99, &defines), ">100");
    }

    #[test]
    fn matching_content_defines_control_range_without_capture_operands() {
        let mut defines = fixture();
        defines.low_half_range_percent = 0;
        defines.low_min_ships = 0;
        assert_eq!(format_count(15, FORCE, 100, false, 15, &defines), "15");
        defines.low_min_ships = 4;
        assert_eq!(format_count(15, FORCE, 100, false, 15, &defines), "11-19");
    }

    #[test]
    fn every_count_words_a_text_as_the_game_formats_it() {
        let defines = fixture();
        assert_eq!(format_count(-1, FORCE, 250, false, 0, &defines), "-1");
        assert_eq!(format_count(5, FORCE, 250, true, 4, &defines), "5");
        assert_eq!(
            format_count(1, FORCE, 100, false, -1, &defines),
            format_count(1, FORCE, 100, false, 15, &defines)
        );
        let mut modded = fixture();
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
        let _ = count_range(i32::MAX, 100, i32::MAX / 2 + 2, 7);
    }
}
