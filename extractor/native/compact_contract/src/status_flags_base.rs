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
pub const COMPACT_STATUS_SOURCE_EMPTY: u32 = 1 << 14;

pub const COMPACT_STATUS_BASELINE_CLEAR: u32 = 1 << 15;

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
    | COMPACT_STATUS_WAR_SEGMENT_ABSENT
    | COMPACT_STATUS_SOURCE_EMPTY
    | COMPACT_STATUS_BASELINE_CLEAR;

pub const fn compact_status_baseline_clear_is_contradictory(status_flags: u32) -> bool {
    (status_flags & COMPACT_STATUS_BASELINE_CLEAR) != 0
        && (status_flags & COMPACT_STATUS_DELTA_DATA) != 0
}

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
    fn v1_known_mask_is_exactly_bits_zero_through_fifteen() {
        assert_eq!(COMPACT_STATUS_V1_KNOWN_MASK, (1_u32 << 16) - 1);
        assert_eq!(compact_status_unknown_flags_v1(0), 0);
        assert_eq!(
            compact_status_unknown_flags_v1(COMPACT_STATUS_V1_KNOWN_MASK),
            0
        );
    }

    #[test]
    fn source_empty_is_an_assigned_bit_every_consumer_now_admits() {
        assert_eq!(
            compact_status_unknown_flags_v1(COMPACT_STATUS_SOURCE_EMPTY),
            0
        );
        assert_eq!(COMPACT_STATUS_SOURCE_EMPTY, 1 << 14);
        assert_eq!(
            COMPACT_STATUS_SOURCE_EMPTY & COMPACT_STATUS_SOURCE_UNAVAILABLE,
            0,
            "the measured-zero badge must not share a bit with the bit it \
             exists to be distinguishable from"
        );
    }

    #[test]
    fn no_lane_local_badge_collides_with_the_measured_empty_bit() {
        assert_eq!(
            DOMAIN51_LANE_LOCAL_STATUS_MASK & COMPACT_STATUS_SOURCE_EMPTY,
            0
        );
    }

    #[test]
    fn v1_unknown_helper_surfaces_every_unassigned_bit() {
        for bit in 16..32 {
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
    fn the_resync_bit_is_assigned_and_collides_with_nothing() {
        assert_eq!(COMPACT_STATUS_BASELINE_CLEAR, 1 << 15);
        assert_eq!(
            compact_status_unknown_flags_v1(COMPACT_STATUS_BASELINE_CLEAR),
            0
        );
        assert_eq!(
            DOMAIN51_LANE_LOCAL_STATUS_MASK & COMPACT_STATUS_BASELINE_CLEAR,
            0
        );
        assert_eq!(COMPACT_STATUS_BASELINE_CLEAR & COMPACT_STATUS_DELTA_DATA, 0);
        assert_eq!(
            COMPACT_STATUS_BASELINE_CLEAR & COMPACT_STATUS_TRANSITION_BASELINE_RESET,
            0,
            "d55's domain-local rebaseline badge and the global resync bit are different              claims and must not share a bit"
        );
    }

    #[test]
    fn a_patch_may_not_also_be_a_replacement() {
        assert!(compact_status_baseline_clear_is_contradictory(
            COMPACT_STATUS_BASELINE_CLEAR | COMPACT_STATUS_DELTA_DATA
        ));
        assert!(!compact_status_baseline_clear_is_contradictory(
            COMPACT_STATUS_BASELINE_CLEAR
        ));
        assert!(!compact_status_baseline_clear_is_contradictory(
            COMPACT_STATUS_DELTA_DATA
        ));
        assert!(!compact_status_baseline_clear_is_contradictory(0));
    }

    #[test]
    fn lane_local_legacy_badges_are_inside_the_v1_known_mask() {
        assert_eq!(
            DOMAIN51_LANE_LOCAL_STATUS_MASK & !COMPACT_STATUS_V1_KNOWN_MASK,
            0
        );
    }
}
