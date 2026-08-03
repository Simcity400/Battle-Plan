//! Canonical base-row and computed-badge contract for compact domain 44.
//!
//! The collector, recorder authority, replay fold, and lane-local proof all
//! consume these constants and helpers. Keep wire vocabulary and the
//! mutually-exclusive convoy-pressure/foreign-homebase availability rule here;
//! re-spelling either beside an encoder or validator recreates a certification
//! split-brain.

pub const HEADER_LEN: usize = 48;
pub const ROW_LEN: usize = 40;

pub const ROW_KIND_PROVINCE_LOGISTICS: u32 = 1;
pub const ROW_KIND_DIVISION_SUPPLY: u32 = 2;
pub const ROW_KIND_COMPUTED_BADGE: u32 = 3;
pub const ROW_KIND_CSUPPLY_PROVINCE_INPUT: u32 = 4;
pub const ROW_KIND_COUNTRY_DISRUPTED_SUPPLY: u32 = 5;
pub const ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE: u32 = 6;

pub const ROW_FLAG_RAIL: u32 = 1;
pub const ROW_FLAG_SUPPLY_NODE: u32 = 1 << 1;
pub const ROW_FLAG_NAVAL_SUPPLY_HUB: u32 = 1 << 2;
pub const ROW_FLAG_INFRASTRUCTURE: u32 = 1 << 3;
pub const ROW_FLAG_LIVE_BUILDING_VECTOR: u32 = 1 << 4;
pub const ROW_FLAG_DIVISION_CURRENT_SUPPLY: u32 = 1 << 5;
pub const ROW_FLAG_DIVISION_MAX_SUPPLY: u32 = 1 << 6;
pub const ROW_FLAG_DIVISION_SUPPLY_GAIN: u32 = 1 << 7;
pub const ROW_FLAG_DIVISION_OWNER: u32 = 1 << 8;
pub const ROW_FLAG_EXACT_SUPPLY_WALL: u32 = 1 << 9;
pub const ROW_FLAG_ATTRITION_WALL: u32 = 1 << 10;
pub const ROW_FLAG_PROXY_VALUE: u32 = 1 << 11;
pub const ROW_FLAG_DIVISION_SUPPLY_DEFICIT: u32 = 1 << 12;
pub const ROW_FLAG_DIVISION_FUEL: u32 = 1 << 13;
pub const ROW_FLAG_MOTORIZATION_WALL: u32 = 1 << 14;
pub const ROW_FLAG_CONVOY_PRESSURE_WALL: u32 = 1 << 15;
pub const ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED: u32 = 1 << 16;
pub const ROW_FLAG_CSUPPLY_EFFICIENCY: u32 = 1 << 17;
pub const ROW_FLAG_CSUPPLY_PROVINCE_ARRAY: u32 = 1 << 18;
pub const ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT: u32 = 1 << 19;
pub const ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE: u32 = 1 << 20;
// Bits 21 and 22 are lane-local, explicitly non-production candidate flags.
pub const ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE: u32 = 1 << 21;
pub const ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT: u32 = 1 << 22;
// Bit 26 is the shipped exact FHB1 source-availability witness. It replaces,
// rather than accompanies, the legacy convoy-pressure wall.
pub const ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE: u32 = 1 << 26;
pub const ROW_ID_DELETED: u32 = 1 << 31;

pub const PRODUCTION_VALID_ROW_FLAGS: u32 = ROW_FLAG_RAIL
    | ROW_FLAG_SUPPLY_NODE
    | ROW_FLAG_NAVAL_SUPPLY_HUB
    | ROW_FLAG_INFRASTRUCTURE
    | ROW_FLAG_LIVE_BUILDING_VECTOR
    | ROW_FLAG_DIVISION_CURRENT_SUPPLY
    | ROW_FLAG_DIVISION_MAX_SUPPLY
    | ROW_FLAG_DIVISION_SUPPLY_GAIN
    | ROW_FLAG_DIVISION_OWNER
    | ROW_FLAG_EXACT_SUPPLY_WALL
    | ROW_FLAG_ATTRITION_WALL
    | ROW_FLAG_PROXY_VALUE
    | ROW_FLAG_DIVISION_SUPPLY_DEFICIT
    | ROW_FLAG_DIVISION_FUEL
    | ROW_FLAG_MOTORIZATION_WALL
    | ROW_FLAG_CONVOY_PRESSURE_WALL
    | ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED
    | ROW_FLAG_CSUPPLY_EFFICIENCY
    | ROW_FLAG_CSUPPLY_PROVINCE_ARRAY
    | ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT
    | ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE
    | ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE;

pub const LANE_LOCAL_VALID_ROW_FLAGS: u32 = PRODUCTION_VALID_ROW_FLAGS
    | ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE
    | ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT;

pub const BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT: i32 = 1;
pub const BADGE_ATTRITION_RUNTIME_COMPUTED: i32 = 2;
pub const BADGE_MOTORIZATION_SETTINGS_UNPINNED: i32 = 3;
pub const BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED: i32 = 4;
pub const BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE: i32 = 5;
pub const BADGE_COUNTRY_DISRUPTED_SOURCE_AVAILABLE: i32 = 6;
pub const BADGE_FOREIGN_HOMEBASE_SOURCE_AVAILABLE: i32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadgeSpec {
    pub flag: u32,
    pub code: i32,
    pub label: &'static str,
}

pub const EXACT_SUPPLY_WALL_BADGE: BadgeSpec = BadgeSpec {
    flag: ROW_FLAG_EXACT_SUPPLY_WALL,
    code: BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT,
    label: "exact-supply",
};
pub const ATTRITION_WALL_BADGE: BadgeSpec = BadgeSpec {
    flag: ROW_FLAG_ATTRITION_WALL,
    code: BADGE_ATTRITION_RUNTIME_COMPUTED,
    label: "attrition",
};
pub const MOTORIZATION_WALL_BADGE: BadgeSpec = BadgeSpec {
    flag: ROW_FLAG_MOTORIZATION_WALL,
    code: BADGE_MOTORIZATION_SETTINGS_UNPINNED,
    label: "motorization",
};
pub const CONVOY_PRESSURE_WALL_BADGE: BadgeSpec = BadgeSpec {
    flag: ROW_FLAG_CONVOY_PRESSURE_WALL,
    code: BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED,
    label: "convoy-pressure",
};
pub const CSUPPLY_CANDIDATE_REJECTED_BADGE: BadgeSpec = BadgeSpec {
    flag: ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED,
    code: BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE,
    label: "csupply-candidate",
};
pub const COUNTRY_DISRUPTED_SOURCE_AVAILABLE_BADGE: BadgeSpec = BadgeSpec {
    flag: ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE,
    code: BADGE_COUNTRY_DISRUPTED_SOURCE_AVAILABLE,
    label: "country-disrupted-source-available",
};
pub const FOREIGN_HOMEBASE_SOURCE_AVAILABLE_BADGE: BadgeSpec = BadgeSpec {
    flag: ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE,
    code: BADGE_FOREIGN_HOMEBASE_SOURCE_AVAILABLE,
    label: "foreign-homebase-source-available",
};

pub const ALWAYS_REQUIRED_BASELINE_BADGES: [BadgeSpec; 4] = [
    EXACT_SUPPLY_WALL_BADGE,
    ATTRITION_WALL_BADGE,
    MOTORIZATION_WALL_BADGE,
    CSUPPLY_CANDIDATE_REJECTED_BADGE,
];

pub const BADGE_FLAGS: u32 = ROW_FLAG_EXACT_SUPPLY_WALL
    | ROW_FLAG_ATTRITION_WALL
    | ROW_FLAG_MOTORIZATION_WALL
    | ROW_FLAG_CONVOY_PRESSURE_WALL
    | ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED
    | ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE
    | ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE;

pub fn badge_spec(flag: u32) -> Option<BadgeSpec> {
    [
        EXACT_SUPPLY_WALL_BADGE,
        ATTRITION_WALL_BADGE,
        MOTORIZATION_WALL_BADGE,
        CONVOY_PRESSURE_WALL_BADGE,
        CSUPPLY_CANDIDATE_REJECTED_BADGE,
        COUNTRY_DISRUPTED_SOURCE_AVAILABLE_BADGE,
        FOREIGN_HOMEBASE_SOURCE_AVAILABLE_BADGE,
    ]
    .into_iter()
    .find(|spec| spec.flag == flag)
}

pub fn validate_row_flags(
    offset: usize,
    row_index: usize,
    row_kind: u32,
    flags: u32,
    known_flags: u32,
    parse_errors: &mut Vec<String>,
) {
    if flags & !known_flags != 0 {
        parse_errors.push(format!(
            "offset {offset}: supply logistics row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE != 0
        && row_kind != ROW_KIND_COMPUTED_BADGE
    {
        parse_errors.push(format!(
            "offset {offset}: supply logistics row {row_index} carries foreign homebase source availability outside a badge row"
        ));
    }
}

/// Validate one computed-badge row using the sole D44 badge vocabulary.
/// Returns the selected badge when it has exactly one badge bit, even when an
/// additional non-badge bit is present, so callers can still diagnose both
/// independent corruptions.
pub fn validate_badge_row(
    offset: usize,
    row_index: usize,
    flags: u32,
    value_a_raw: i32,
    parse_errors: &mut Vec<String>,
) -> Option<BadgeSpec> {
    let badge_flags = flags & BADGE_FLAGS;
    if badge_flags.count_ones() != 1 {
        parse_errors.push(format!(
            "offset {offset}: supply logistics badge row {row_index} must carry exactly one badge flag"
        ));
    }
    if flags != badge_flags {
        parse_errors.push(format!(
            "offset {offset}: supply logistics badge row {row_index} has non-badge flags"
        ));
    }
    let spec = (badge_flags.count_ones() == 1)
        .then(|| badge_spec(badge_flags))
        .flatten();
    if let Some(spec) = spec {
        if value_a_raw != spec.code {
            parse_errors.push(format!(
                "offset {offset}: supply logistics {} badge row {row_index} has invalid code {value_a_raw}",
                spec.label
            ));
        }
    }
    spec
}

/// Validate the baseline badges whose presence is a frame-level invariant.
/// FHB1 is an availability witness even when its row count is zero: a coherent
/// read of an empty engine collection is exact EMPTY, not source failure.
pub fn validate_baseline_badge_coverage(
    offset: usize,
    seen_badge_flags: u32,
    foreign_homebase_extension_present: bool,
    parse_errors: &mut Vec<String>,
) {
    for (label, flag) in [
        ("exact supply wall", ROW_FLAG_EXACT_SUPPLY_WALL),
        ("attrition wall", ROW_FLAG_ATTRITION_WALL),
        ("motorization wall", ROW_FLAG_MOTORIZATION_WALL),
        (
            "csupply candidate rejection",
            ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED,
        ),
    ] {
        if seen_badge_flags & flag == 0 {
            parse_errors.push(format!(
                "offset {offset}: supply logistics missing {label} badge row"
            ));
        }
    }

    let required_pressure = if foreign_homebase_extension_present {
        FOREIGN_HOMEBASE_SOURCE_AVAILABLE_BADGE
    } else {
        CONVOY_PRESSURE_WALL_BADGE
    };
    if seen_badge_flags & required_pressure.flag == 0 {
        let label = if foreign_homebase_extension_present {
            "foreign homebase source availability"
        } else {
            "convoy pressure wall"
        };
        parse_errors.push(format!(
            "offset {offset}: supply logistics missing {label} badge row"
        ));
    }

    if foreign_homebase_extension_present {
        if seen_badge_flags & ROW_FLAG_CONVOY_PRESSURE_WALL != 0 {
            parse_errors.push(format!(
                "offset {offset}: supply logistics FHB1 source retained obsolete convoy pressure wall badge row"
            ));
        }
    } else if seen_badge_flags & ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE != 0 {
        parse_errors.push(format!(
            "offset {offset}: supply logistics foreign homebase source badge is present without FHB1"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_badges_are_mutually_exclusive_and_empty_fhb1_is_available() {
        let core = ALWAYS_REQUIRED_BASELINE_BADGES
            .iter()
            .fold(0, |flags, badge| flags | badge.flag);

        let mut errors = Vec::new();
        validate_baseline_badge_coverage(
            0,
            core | ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE,
            true,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");

        validate_baseline_badge_coverage(
            0,
            core | ROW_FLAG_CONVOY_PRESSURE_WALL,
            true,
            &mut errors,
        );
        assert!(errors
            .iter()
            .any(|error| error.contains("missing foreign homebase")));
        assert!(errors
            .iter()
            .any(|error| error.contains("obsolete convoy pressure")));
    }

    #[test]
    fn foreign_homebase_availability_cannot_ride_on_an_entity_row() {
        let mut errors = Vec::new();
        validate_row_flags(
            0,
            7,
            ROW_KIND_PROVINCE_LOGISTICS,
            ROW_FLAG_LIVE_BUILDING_VECTOR | ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE,
            PRODUCTION_VALID_ROW_FLAGS,
            &mut errors,
        );
        assert!(errors
            .iter()
            .any(|error| error.contains("outside a badge row")));
    }
}
