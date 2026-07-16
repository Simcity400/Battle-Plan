//! Dependency-free base compact status flags and domain-51 lane-local badges.
//!
//! Kept self-contained for standalone validators that also include lane payload
//! modules requiring the canonical status constants.

pub const COMPACT_STATUS_STATE_UNAVAILABLE: u32 = 1 << 0;
pub const COMPACT_STATUS_PROFILE_MISMATCH: u32 = 1 << 1;
pub const COMPACT_STATUS_NO_CLOCK: u32 = 1 << 2;
pub const COMPACT_STATUS_NO_SCALAR_SOURCE: u32 = 1 << 3;
pub const COMPACT_STATUS_WAR_BRIDGE_APPLIED: u32 = 1 << 4;
pub const COMPACT_STATUS_EXTRACTION_PERF_GATE: u32 = 1 << 5;
pub const COMPACT_STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const COMPACT_STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const COMPACT_STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const COMPACT_STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const COMPACT_STATUS_DELTA_DATA: u32 = 1 << 10;
pub const COMPACT_STATUS_TRANSITION_BASELINE_RESET: u32 = 1 << 11;
pub const COMPACT_STATUS_CONTROL_INVALID: u32 = 1 << 12;
pub const COMPACT_STATUS_WAR_SEGMENT_ABSENT: u32 = 1 << 13;

/// Every status bit defined by compact-stream contract version 1.
///
/// Bits 14..=31 are unassigned. Consumers must reject them before applying
/// failure/informational classification; an unassigned bit is not evidence of
/// a future informational status.
pub const COMPACT_STATUS_V1_KNOWN_MASK: u32 = COMPACT_STATUS_STATE_UNAVAILABLE
    | COMPACT_STATUS_PROFILE_MISMATCH
    | COMPACT_STATUS_NO_CLOCK
    | COMPACT_STATUS_NO_SCALAR_SOURCE
    | COMPACT_STATUS_WAR_BRIDGE_APPLIED
    | COMPACT_STATUS_EXTRACTION_PERF_GATE
    | COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_DELTA_DATA
    | COMPACT_STATUS_TRANSITION_BASELINE_RESET
    | COMPACT_STATUS_CONTROL_INVALID
    | COMPACT_STATUS_WAR_SEGMENT_ABSENT;

/// Returns status bits not assigned by compact-stream contract version 1.
///
/// A non-zero result is a contract error and must be rejected before calling
/// helpers that classify known statuses as failure or informational.
pub const fn compact_status_unknown_flags_v1(status_flags: u32) -> u32 {
    status_flags & !COMPACT_STATUS_V1_KNOWN_MASK
}

pub const DOMAIN51_STATUS_FLAG_NAMES_PARTIAL: u32 = 1 << 11;
pub const DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED: u32 = 1 << 12;
pub const DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED: u32 = 1 << 13;
pub const DOMAIN51_LANE_LOCAL_STATUS_MASK: u32 = DOMAIN51_STATUS_FLAG_NAMES_PARTIAL
    | DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
    | DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_known_mask_is_exactly_bits_zero_through_thirteen() {
        assert_eq!(COMPACT_STATUS_V1_KNOWN_MASK, (1_u32 << 14) - 1);
        assert_eq!(compact_status_unknown_flags_v1(0), 0);
        assert_eq!(
            compact_status_unknown_flags_v1(COMPACT_STATUS_V1_KNOWN_MASK),
            0
        );
    }

    #[test]
    fn v1_unknown_helper_surfaces_every_unassigned_bit() {
        for bit in 14..32 {
            let flag = 1_u32 << bit;
            assert_eq!(
                compact_status_unknown_flags_v1(flag),
                flag,
                "bit {bit} must be rejected, not classified as informational"
            );
        }
        assert_eq!(
            compact_status_unknown_flags_v1(
                COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_DELTA_DATA | (1 << 31),
            ),
            1 << 31
        );
    }

    #[test]
    fn lane_local_legacy_badges_are_inside_the_v1_known_mask() {
        assert_eq!(
            DOMAIN51_LANE_LOCAL_STATUS_MASK & !COMPACT_STATUS_V1_KNOWN_MASK,
            0
        );
    }
}
