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

/// **CLEAR this domain's folded state, then apply this payload as its new whole
/// baseline.** The stream's deliberate resync primitive.
///
/// # What it is for
///
/// Every other whole payload on this stream is a keyframe, and a keyframe is NOT
/// a resync point: once a domain has a delta baseline the fold applies every
/// later record SPARSELY (`DELTA_WIRE_DESIGN.md` §2.3), so a whole payload is
/// merged and a key that vanished is retained until something tombstones it.
/// That is correct — absence is not deletion — but it leaves the producer with
/// no way to say "my baseline and yours have parted company; here is the whole
/// state; start again". This bit is that sentence, and it is the only one.
///
/// # The rules that ride with it
///
/// 1. **Whole payloads only.** `BASELINE_CLEAR | DELTA_DATA` is a contradiction —
///    a patch cannot be a replacement — and is rejected, not resolved. See
///    [`compact_status_baseline_clear_is_contradictory`].
/// 2. **Deliberate resync only, never a routine keyframe.** A keyframe forced by
///    a lost pin says nothing about whether the two sides disagree, and badging
///    those would make the bit mean "whole payload", which is what the ABSENCE
///    of `DELTA_DATA` already means.
/// 3. **Admissible per domain, fail-closed.** A domain may carry it only if a
///    record of that domain with no failure bit is a COMPLETE CLAIM ABOUT ITS
///    WHOLE EXTENT. `compact_baseline_clear_admissible` is the list, and a
///    record carrying the bit for a domain that is not on it is a hard
///    rejection. See [`crate::compact_baseline_clear_admissible`] for why d13 is
///    excluded — the case that makes the rule necessary rather than decorative.
/// 4. **Exactly one domain.** The fold clears the state of the domain the record
///    belongs to and nothing else. It is a resync, not a reset.
/// 5. **Not a failure bit**, and deliberately absent from
///    [`crate::COMPACT_STATUS_FAILURE_MASK`]: a resync is a successful read that
///    also carries an instruction.
///
/// # Old readers
///
/// Bit 15 was the next free global bit and this is the assignment §2.5 of
/// `DELTA_WIRE_DESIGN.md` anticipated. A consumer built against the old mask
/// sees an unassigned bit and HARD-REJECTS the record, which is the intended
/// outcome: a reader that would merge a resync record has misread the stream,
/// and refusing is strictly better than that.
pub const COMPACT_STATUS_BASELINE_CLEAR: u32 = 1 << 15;

/// Every status bit defined by compact-stream contract version 1.
///
/// Bits 16..=31 are unassigned. Consumers must reject them before applying
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
    | COMPACT_STATUS_SOURCE_EMPTY
    | COMPACT_STATUS_BASELINE_CLEAR;

/// A record that claims to be both a patch and a replacement.
///
/// Rule 1 of [`COMPACT_STATUS_BASELINE_CLEAR`], as a predicate rather than as
/// prose, so producer and consumer ask it the same way. Every consumer must
/// refuse such a record: resolving it either way invents a semantics neither
/// side agreed to.
pub const fn compact_status_baseline_clear_is_contradictory(status_flags: u32) -> bool {
    (status_flags & COMPACT_STATUS_BASELINE_CLEAR) != 0
        && (status_flags & COMPACT_STATUS_DELTA_DATA) != 0
}

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
        // Production change that makes this fail: assigning bit 15 to something
        // else, or to a lane-local badge. A lane-local badge is STRIPPED before
        // global interpretation, so a lane that reused bit 15 would silently
        // erase the resync instruction for its own domain and no other.
        assert_eq!(COMPACT_STATUS_BASELINE_CLEAR, 1 << 15);
        assert_eq!(compact_status_unknown_flags_v1(COMPACT_STATUS_BASELINE_CLEAR), 0);
        assert_eq!(DOMAIN51_LANE_LOCAL_STATUS_MASK & COMPACT_STATUS_BASELINE_CLEAR, 0);
        // And it is not a second spelling of an existing bit.
        assert_eq!(COMPACT_STATUS_BASELINE_CLEAR & COMPACT_STATUS_DELTA_DATA, 0);
        assert_eq!(
            COMPACT_STATUS_BASELINE_CLEAR & COMPACT_STATUS_TRANSITION_BASELINE_RESET,
            0,
            "d55's domain-local rebaseline badge and the global resync bit are different              claims and must not share a bit"
        );
    }

    #[test]
    fn a_patch_may_not_also_be_a_replacement() {
        // Production change that makes this fail: letting a producer set both,
        // or a consumer resolving the pair instead of refusing it. Resolving it
        // either way invents a semantics neither side agreed to -- apply the
        // patch after clearing, and the patch is against a state that no longer
        // exists; clear after applying, and the payload is discarded.
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
