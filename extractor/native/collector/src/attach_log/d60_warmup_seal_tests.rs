//! lane QX — d60 first-attach warmup seals clean (pw_gate_final defect).
//!
//! The closed defect: on the FIRST frames of an attach, background war-relation
//! discovery has not completed its first validated pass. The cold consume arm
//! used to route those frames through `capwar60_source_unavailable_read`, which
//! ORs `SOURCE_UNAVAILABLE|PARTIAL_DATA` into a published (empty) data record.
//! `status_flags_or` is an OR across the whole capture window, so ONE warmup
//! frame blocked pw_gate_final: d60's window word was 0x300 (hour 60830856,
//! origin sample 365713358724465886 == FNV-1a64 of
//! "capwar_discovery_pending" — see reproduce_d60_cause.py in
//! runs/qx_d60_seal/). The fix publishes the typed warmup miss as one named
//! DOMAIN_GAP and no PERF/DATA pair; these tests pin every seam by
//! construction.

use super::*;
use compact_contract::domain_gap::{
    DomainGapPart, DomainGapReason, DomainGapRecordV1, DomainGapWitnessProvenance,
};
use std::thread;

/// The exact static reason strings `capwar60_unavailable_reason` can return.
const WARMUP_REASONS: [&str; 2] = ["capwar_discovery_pending", "capwar_sweep_not_warmed"];

fn cold_tracker() -> CompactCapitulationWarStateLiveTracker {
    CompactCapitulationWarStateLiveTracker::default()
}

fn tracker_with_validation_in_flight() -> CompactCapitulationWarStateLiveTracker {
    let mut tracker = cold_tracker();
    // A live handle is all `capwar60_unavailable_reason` reads; the thread's
    // result value is never consumed (the tracker Drop joins it).
    tracker.validation_handle = Some((
        1,
        1,
        thread::spawn(|| -> CapWar60ValidationResult {
            Err(capwar60::extractor::ExtractError::InvalidRoot(
                "warmup-seal-test-handle",
            ))
        }),
    ));
    tracker
}

// kills: classifying any real refusal ("capwar_sweep_epoch_changed",
// "capwar_worker_panic", "capwar_memory_read", "capwar_discovery_degraded")
// as warmup would publish a named gap for a read that MUST refuse by name —
// the hour-misattribution hard stop.
#[test]
fn the_warmup_classifier_admits_exactly_the_two_cold_states() {
    for reason in WARMUP_REASONS {
        assert!(
            capwar60_discovery_warmup_reason(reason),
            "{reason} is a first-attach warmup state"
        );
    }
    for refusal in [
        "capwar_sweep_epoch_changed",
        "capwar_worker_panic",
        "capwar_memory_read",
        capwar60::extractor::CAPWAR_DISCOVERY_DEGRADED,
    ] {
        assert!(
            !capwar60_discovery_warmup_reason(refusal),
            "{refusal} is a REAL refusal and keeps its failure bits"
        );
    }
}

// kills: if the pre-spawn frames ever mapped to something other than
// "capwar_sweep_not_warmed", they would fall into the failure arm and re-poison
// the window word exactly as pw_gate_final hour 60830856 did.
#[test]
fn unavailable_reason_names_both_warmup_shapes_from_the_tracker_state() {
    assert_eq!(
        capwar60_unavailable_reason(&cold_tracker(), 0),
        "capwar_sweep_not_warmed",
        "no validation spawned yet (cold clock) is still warmup"
    );
    assert_eq!(
        capwar60_unavailable_reason(&tracker_with_validation_in_flight(), 1),
        "capwar_discovery_pending",
        "in-flight first pass is warmup, not source unavailability"
    );
}

// kills: a failed validation for the CURRENT generation must NOT read as
// warmup — a persistently failing discovery walk has to keep blocking, or a
// dead relation-root path would seal captures with silent permanent gaps.
// Covers BOTH reachable same-generation failure shapes: the post-failure
// frames before the backoff deadline re-arms (no handle), AND the
// retry-in-flight frames after it (same-generation retry spawns WITHOUT
// clearing the failure fields, so handle-Some alone must never downgrade a
// proven failure to warmup).
#[test]
fn a_recorded_validation_failure_is_not_warmup() {
    let failure = ("capwar_memory_read", 3u64);
    let mut after_failure = cold_tracker();
    after_failure.validation_failure_gen = Some(failure.1);
    after_failure.validation_failure_reason = Some(failure.0);
    after_failure.validation_failure_streak = 1;
    let reason = capwar60_unavailable_reason(&after_failure, failure.1);
    assert_eq!(reason, failure.0);
    assert!(!capwar60_discovery_warmup_reason(reason));

    let mut retry_in_flight = after_failure;
    retry_in_flight.validation_handle = Some((
        failure.1,
        1,
        thread::spawn(|| -> CapWar60ValidationResult {
            Err(capwar60::extractor::ExtractError::InvalidRoot(
                "warmup-seal-test-retry",
            ))
        }),
    ));
    let retried_reason =
        capwar60_unavailable_reason(&retry_in_flight, failure.1);
    assert_eq!(
        retried_reason, failure.0,
        "a retry of a proven failure keeps the true cause, not 'pending'"
    );
    assert!(!capwar60_discovery_warmup_reason(retried_reason));
}

// kills: mislabeling a PRIOR generation's error as current. The shared spawn
// gate deliberately clears the failure tuple when the sweep advances and a
// fresh attempt begins, and
// `capwar60_cold_diagnostic_retains_exact_worker_failure` (attach_log/tests.rs)
// pins that cross-generation NAMING for this exact helper. The NAME stays
// cold; whether such a frame publishes a warmup word or failure bits is
// decided separately by `validation_ever_failed`
// (see `a_tracker_that_has_ever_failed_never_publishes_benign_warmup_words`).
#[test]
fn a_prior_generations_failure_is_not_named_after_the_generation_advances() {
    let mut tracker = cold_tracker();
    tracker.validation_failure_gen = Some(3);
    tracker.validation_failure_reason = Some("capwar_memory_read");
    tracker.validation_failure_streak = 1;
    assert_eq!(
        capwar60_unavailable_reason(&tracker, 4),
        "capwar_sweep_not_warmed",
        "generation 4 has never been attempted; its state is cold, not failed"
    );
}

// kills: THE WINDOW-WORD POISON. If the warmup read reintroduces
// SOURCE_UNAVAILABLE|PARTIAL_DATA (or any FAILURE_MASK bit) into its status
// word, one warmup frame blocks the whole capture again — the exact mechanism
// that held pw_gate_final at 0x300.
#[test]
fn the_warmup_read_carries_no_failure_bits_into_any_window_or() {
    let tracker = cold_tracker();
    let inherited = COMPACT_STATUS_DELTA_DATA;
    let read = capwar60_warmup_gap_read(Instant::now(), inherited, &tracker, "capwar_sweep_not_warmed");
    assert!(read.warmup_gap);
    assert_eq!(
        read.status_flags,
        inherited,
        "warmup passes the incoming word through untouched"
    );
    assert_eq!(
        read.status_flags
            & (COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA),
        0,
        "nothing failed here; failure bits would poison the window-wide OR"
    );
    assert_eq!(read.observed_rows, 0);
    assert_eq!(read.emitted_country_rows, 0);
    assert_eq!(read.emitted_war_rows, 0);
    assert_eq!(read.failure_reason, Some("capwar_sweep_not_warmed"));
}

// kills: over-generalising the fix. Every REAL refusal must keep failing
// closed with su+pd and must never be marked as a gap-publishable warmup miss;
// only the encode site decides the wire shape, from this flag.
#[test]
fn real_refusals_still_fail_closed_with_their_failure_bits() {
    let tracker = cold_tracker();
    let read = capwar60_source_unavailable_read(
        Instant::now(),
        0,
        0,
        0,
        "capwar_sweep_epoch_changed",
        &tracker,
    );
    assert!(!read.warmup_gap);
    assert_ne!(
        read.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        0,
        "SOURCE_UNAVAILABLE must be present, not just any one failure bit"
    );
    assert_ne!(
        read.status_flags & COMPACT_STATUS_PARTIAL_DATA,
        0,
        "PARTIAL_DATA must be present alongside it"
    );
    assert_eq!(read.failure_reason, Some("capwar_sweep_epoch_changed"));
}

// kills: the production encoder and this test share ONE payload builder,
// [`capwar60_warmup_gap_payload`] — changing the reason, ticks, range, hour
// bounds, part or provenance in production changes what is proven here. The
// shape itself: `(0, 0)` ticks under `Unobserved` provenance, part Both (d60
// is a mixed domain), target == range == the frame hour; any drift from it is
// refused by the payload grammar AT DECODE on the recorder side, so proving it
// legal HERE is the construction proof that a warmup frame reaches the wire
// instead of a frame error. (What no payload-level test can pin: that no
// PERF/DATA record rides beside the gap — that invariant is enforced by the
// fold's `gap_accounting_invalid` rule on every capture and by the single
// `if read.warmup_gap` branch at the encode site; FIX.md's confirming capture
// closes it end-to-end.)
#[test]
fn the_encode_sites_gap_shape_encodes_decodes_and_validates() {
    fn hour(offset: i32) -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + 1944 * 365 * 24 + offset
    }
    let part = domain_gap_part(COMPACT_DOMAIN_CAPITULATION_WAR_STATE_LIVE);
    assert_eq!(part, DomainGapPart::Both);
    // A NONZERO sentinel for the ordinal: production passes its advancing
    // per-(domain,part) counter, and the stream validator requires those
    // ordinals contiguous from zero — a builder regression to a constant
    // would break real captures while a zero-sentinel assertion could not see
    // it.
    let sentinel_ordinal = 0x1234u64;
    let wire = capwar60_warmup_gap_payload(hour(0), sentinel_ordinal)
        .expect("the pinned d60 warmup gap shape encodes");
    compact_contract::validate_domain_gap_payload(&wire)
        .expect("the pinned d60 warmup gap shape satisfies the H4DG grammar");
    // The provenance is part of the claim, not just the reason: the read
    // consumed nothing, so it may not carry a bracket observation, and
    // `Unstated` (the every-unconverted-producer default) is a DIFFERENT wire
    // sentence that must fail this pin.
    assert_eq!(
        DomainGapRecordV1::decode_witness_provenance(&wire).unwrap(),
        DomainGapWitnessProvenance::Unobserved,
    );
    let decoded = DomainGapRecordV1::decode(&wire).expect("round-trips");
    assert_eq!(decoded.reason, DomainGapReason::SourceUnavailable);
    assert_eq!(decoded.part, DomainGapPart::Both);
    assert_eq!(decoded.target_hour_raw, hour(0));
    assert_eq!(
        decoded.outcome_ordinal, sentinel_ordinal,
        "the caller's advancing ordinal must reach the wire unchanged"
    );
}

// kills: THE ROUND-4 ERASURE HOLE. Bracket reconciliation may replace a
// failure frame's PERF/DATA records with a torn gap, and a generation advance
// clears the failure tuple — so neither mechanism alone guarantees a published
// failure word. `validation_ever_failed` survives both (it rides the
// rejected-bracket reset and is cleared only by adoption of a completed set),
// and this predicate is what turns that history into refusal: once any walk
// has failed, NO cold frame publishes a benign warmup word until recovery.
#[test]
fn a_tracker_that_has_ever_failed_never_publishes_benign_warmup_words() {
    let mut failed_once = cold_tracker();
    failed_once.validation_ever_failed = true;
    for reason in WARMUP_REASONS {
        assert!(
            !capwar60_cold_read_is_warmup(&failed_once, reason),
            "after any failed pass, {reason} must refuse closed with failure bits"
        );
    }
    // Recovery (adoption clears the flag) re-opens warmup words: nothing has
    // failed SINCE the last success.
    let mut recovered = failed_once;
    recovered.validation_ever_failed = false;
    for reason in WARMUP_REASONS {
        assert!(capwar60_cold_read_is_warmup(&recovered, reason));
    }
}

// kills: a reap arm "forgetting" any part of the failure bookkeeping —
// including the sticky history flag round 4 demanded. Both production arms
// route through this ONE recorder, so the assertions here hold for the error
// and panic joins alike.
#[test]
fn noting_a_validation_failure_sets_the_full_sticky_state() {
    let mut tracker = cold_tracker();
    tracker.validation_failure_streak = 2;
    let reason = capwar60_note_validation_failure(&mut tracker, 7, "capwar_memory_read");
    assert_eq!(reason, "capwar_memory_read");
    assert_eq!(tracker.validation_failure_gen, Some(7));
    assert_eq!(tracker.validation_failure_reason, Some("capwar_memory_read"));
    assert_eq!(tracker.validation_failure_streak, 3);
    assert!(
        tracker.validation_ever_failed,
        "the sticky history flag must be set by every recorded failure"
    );
    assert!(tracker.validation_retry_not_before.is_some());
    assert_eq!(tracker.stale_frames, 1);
}

// kills: a bracket-rejection reset silently dropping the sticky history. The
// take/restore pair is what carries d60's discovery state across a rejected
// frame's tracker rebuild; if `validation_ever_failed` stopped riding it, a
// torn bracket would launder every earlier failure and re-open benign warmup
// publication.
#[test]
fn the_sticky_history_survives_a_bracket_rejection_reset() {
    let mut before = cold_tracker();
    before.validation_ever_failed = true;
    before.attempted_gen = Some(4);
    before.validation_failure_gen = Some(4);
    before.validation_failure_reason = Some("capwar_memory_read");
    before.validation_failure_streak = 1;

    let carried = before.take_shared_sweep_discovery();
    let mut fresh = cold_tracker();
    fresh.restore_shared_sweep_discovery(carried);

    assert!(fresh.validation_ever_failed);
    assert!(!capwar60_cold_read_is_warmup(
        &fresh,
        "capwar_discovery_pending"
    ));
}
