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
/// The read reached its source, the source answered, and the answer was empty.
///
/// The counterpart to [`COMPACT_STATUS_SOURCE_UNAVAILABLE`], which has always
/// meant "we did not get through the read". Before this bit existed the two
/// were the same word: a domain that legitimately observed nothing published
/// exactly what a domain whose reader was broken published, so "is this capture
/// missing data?" was not answerable from the stream.
///
/// NOT a failure bit, and deliberately absent from
/// [`crate::COMPACT_STATUS_FAILURE_MASK`]: a measured zero is an ANSWER. It is
/// also not the same claim as a clean zero-row record — a delta domain with no
/// changes and an emit-once domain with nothing new both publish zero rows and
/// a clean word, and neither of them is measuring that the source is empty.
///
/// A producer may only set it with a completeness witness for the emptiness
/// itself: the walk reached the whole extent and rejected nothing on the way.
/// A zero that survives a rejected row is not a measured zero, and stays
/// [`COMPACT_STATUS_SOURCE_UNAVAILABLE`].
pub const COMPACT_STATUS_SOURCE_EMPTY: u32 = 1 << 14;

/// Every status bit defined by compact-stream contract version 1.
///
/// Bits 15..=31 are unassigned. Consumers must reject them before applying
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
    | COMPACT_STATUS_WAR_SEGMENT_ABSENT
    | COMPACT_STATUS_SOURCE_EMPTY;

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
    fn v1_known_mask_is_exactly_bits_zero_through_fourteen() {
        assert_eq!(COMPACT_STATUS_V1_KNOWN_MASK, (1_u32 << 15) - 1);
        assert_eq!(compact_status_unknown_flags_v1(0), 0);
        assert_eq!(
            compact_status_unknown_flags_v1(COMPACT_STATUS_V1_KNOWN_MASK),
            0
        );
    }

    #[test]
    fn source_empty_is_an_assigned_bit_every_consumer_now_admits() {
        // Production change that makes this fail: dropping SOURCE_EMPTY from
        // the known mask. Every gate in the workspace rejects an unassigned
        // bit, so a producer that published this one against the old mask
        // would fail the replay gate rather than report a measured zero.
        assert_eq!(compact_status_unknown_flags_v1(COMPACT_STATUS_SOURCE_EMPTY), 0);
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
        // A lane-local badge is stripped before global interpretation, so a
        // lane that reused bit 14 would erase the global measured-zero claim
        // for its own domain and for nobody else's -- exactly the kind of
        // producer/validator disagreement this contract keeps hitting.
        assert_eq!(
            DOMAIN51_LANE_LOCAL_STATUS_MASK & COMPACT_STATUS_SOURCE_EMPTY,
            0
        );
    }

    #[test]
    fn v1_unknown_helper_surfaces_every_unassigned_bit() {
        for bit in 15..32 {
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
