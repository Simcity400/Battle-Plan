//! lane RS — d53 first-attach warmup seals clean (pw_gate_final residual §5).
//!
//! The closed defect: on the FIRST frames of an attach, the shared heap sweep
//! has not completed its first multi-needle pass, and the off-frame war-row
//! worker has not reaped yet. `domain53_sweep_stage` fail-closed those frames
//! through `domain53_source_unavailable_read`, which ORs
//! `SOURCE_UNAVAILABLE|PARTIAL_DATA` into a published (empty) record; status
//! OR is window-wide, so 50 such frames held pw_gate_final's d53 word at 0x300
//! (`so_d53_source_unavailable`, hours 60830856–60830912 ≈ 7 s wall).
//!
//! The fix has two halves. (1) The typed warmup miss: while NO frame of this
//! attach has consumed yet, a first-discovery fail-closed verdict publishes
//! one named DOMAIN_GAP and no PERF/DATA pair (mirror of lane QX's d60
//! warmup), so nothing reaches any window-wide OR and the gap shows in the
//! report by name. (2) The optimised first pass itself lives in
//! `heap_sweep_run`/`heap_sweep_multi_needle_pass`; these tests pin the
//! classification seams of (1).

use super::*;
use super::tests::{D53_TEST_NEEDLES, d53_warm_snapshot};
use compact_contract::domain_gap::{
    DomainGapPart, DomainGapReason, DomainGapRecordV1, DomainGapWitnessProvenance,
};

/// The exact static reasons `domain53_warmup_reason` admits: the three shapes
/// "the FIRST discovery of this attach is not finished" can take.
const WARMUP_REASONS: [&str; 3] = [
    "domain53_discovery_cold",
    "domain53_discovery_not_warmed",
    "domain53_war_addrs_pending",
];

fn cold_tracker() -> CompactDiplomacyIntelOperativesLiveTracker {
    CompactDiplomacyIntelOperativesLiveTracker::default()
}

// kills: classifying any real refusal as warmup would publish a named gap for
// a read that MUST refuse by name — the hour-misattribution hard stop.
// `domain53_war_addrs_cold` names a generation whose walk never completed (a
// post-failure/anomaly state); every other string here is a typed refusal.
#[test]
fn the_warmup_classifier_admits_exactly_the_first_discovery_states() {
    for reason in WARMUP_REASONS {
        assert!(
            domain53_warmup_reason(reason),
            "{reason} is a first-attach warmup state"
        );
    }
    for refusal in [
        "domain53_war_addrs_cold",
        "domain53_war_addrs_sweep_epoch_changed",
        DOMAIN53_WAR_WORKER_PANIC,
        "domain53_war_worker_memory_read",
        "domain53_frame_degraded",
        "domain53_extract_error",
    ] {
        assert!(
            !domain53_warmup_reason(refusal),
            "{refusal} is a REAL refusal and keeps its failure bits"
        );
    }
}

// kills: THE WINDOW-WORD POISON. If the warmup read reintroduced
// SOURCE_UNAVAILABLE|PARTIAL_DATA (or any failure bit) into its status word,
// one warmup frame blocks the whole capture again — the exact mechanism that
// held pw_gate_final at 0x300 for d53.
#[test]
fn the_warmup_read_carries_no_failure_bits_into_any_window_or() {
    let tracker = cold_tracker();
    let inherited = COMPACT_STATUS_DELTA_DATA;
    let read =
        domain53_warmup_gap_read(Instant::now(), inherited, &tracker, "domain53_discovery_cold");
    assert!(read.warmup_gap);
    assert_eq!(
        read.status_flags, inherited,
        "warmup passes the incoming word through untouched"
    );
    assert_eq!(
        read.status_flags & (COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA),
        0,
        "nothing failed here; failure bits would poison the window-wide OR"
    );
    assert_eq!(read.observed_rows, 0);
    assert_eq!(read.emitted_rows, 0);
    assert_eq!(read.stale_dropped_total, 0);
    assert_eq!(read.diag.failure_reason, Some("domain53_discovery_cold"));
}

// kills: THE OWNER KILL TEST. A `domain53_discovery_cold` verdict AFTER the
// first completed pass was consumed must still be a FAILURE — it means the
// sweep died or changed epoch mid-attach and hours are being lost. The window
// flag is what makes that verdict refuse closed with failure bits instead of a
// benign warming-up word.
#[test]
fn a_discovery_cold_verdict_after_the_first_consumed_pass_is_a_failure() {
    for reason in WARMUP_REASONS {
        assert!(
            domain53_failclosed_is_initial_warmup_in(reason, true),
            "with the window open, {reason} is warmup"
        );
        assert!(
            !domain53_failclosed_is_initial_warmup_in(reason, false),
            "after the first consumed frame (window closed), {reason} must fail closed"
        );
    }
}

// kills: the production transition that closes the warmup window. Driving the
// stage to Ready through the synthetic-snapshot harness against a LOCAL window
// flag must close it; if the assignment moved or dropped, every later cold
// frame would read as warmup again.
#[test]
fn a_ready_frame_closes_the_warmup_window_in_the_stage() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    let window = AtomicBool::new(true);
    // A validated war set under the snapshot's epoch (7) so the stage can
    // reach Ready without spawning a real worker.
    tracker.validated_war_addrs = Some(Arc::new(vec![0x0beef_u64]));
    tracker.war_validated_gen = Some(9);
    tracker.war_validated_epoch = Some(7);
    let warm = d53_warm_snapshot(9);
    let stage = domain53_sweep_stage(&mut tracker, &warm, D53_TEST_NEEDLES, now, &window);
    assert!(matches!(stage.outcome, Domain53SweepStageOutcome::Ready { .. }));
    assert!(
        !window.load(Ordering::Relaxed),
        "the first Ready frame must close the typed warmup window"
    );
}

// kills: the ONLY legitimate reopen is a fresh LOGICAL stream (rollover):
// nothing has consumed there, its gap ordinals restart legally. A same-stream
// transition — bracket reset, refused scheduler frame, uncertain-boundary
// retry — must never reopen it; those paths cannot reach the window at all
// now that it lives in a process static outside every tracker reset.
#[test]
fn only_a_stream_rollover_reopens_the_window() {
    let window = AtomicBool::new(true);
    domain53_note_ready_consumed_in(&window);
    assert!(!domain53_failclosed_is_initial_warmup_in(
        "domain53_discovery_cold",
        window.load(Ordering::Relaxed)
    ));
    // Simulating the rollover arm's single store:
    window.store(true, Ordering::Relaxed);
    for reason in WARMUP_REASONS {
        assert!(domain53_failclosed_is_initial_warmup_in(reason, true));
    }
}

// kills: over-generalising the fix. Every REAL refusal must keep failing
// closed with su+pd and must never be marked gap-publishable; only the encode
// site decides the wire shape, from this flag.
#[test]
fn real_refusals_still_fail_closed_with_their_failure_bits() {
    let now = Instant::now();
    let diag = Domain53Diag {
        failure_reason: Some("domain53_war_addrs_sweep_epoch_changed"),
        ..Domain53Diag::default()
    };
    let read = domain53_source_unavailable_read(now, 0, 0, diag);
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
    assert_eq!(
        read.diag.failure_reason,
        Some("domain53_war_addrs_sweep_epoch_changed")
    );
}

// kills: the production encoder and this test share ONE payload builder,
// [`domain53_warmup_gap_payload`] — changing the reason, ticks, range, hour
// bounds, part or provenance in production changes what is proven here. The
// shape itself: `(0, 0)` ticks under `Unobserved` provenance, part Both (d53
// is a mixed domain), target == range == the frame hour; any drift from it is
// refused by the payload grammar AT DECODE on the recorder side. (What no
// payload-level test can pin: that no PERF/DATA record rides beside the gap —
// enforced by the fold's `gap_accounting_invalid` rule on every capture and
// by the single `if read.warmup_gap` branch at the encode site.)
#[test]
fn the_encode_sites_gap_shape_encodes_decodes_and_validates() {
    fn hour(offset: i32) -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + 1944 * 365 * 24 + offset
    }
    let part = domain_gap_part(COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE);
    assert_eq!(part, DomainGapPart::Both);
    // NONZERO sentinel: production passes its advancing per-(domain,part)
    // counter, and the stream validator requires those ordinals contiguous
    // from zero — a builder regression to a constant would break real
    // captures while a zero-sentinel assertion could not see it.
    let sentinel_ordinal = 0x5678u64;
    let wire = domain53_warmup_gap_payload(hour(0), sentinel_ordinal)
        .expect("the pinned d53 warmup gap shape encodes");
    compact_contract::validate_domain_gap_payload(&wire)
        .expect("the pinned d53 warmup gap shape satisfies the H4DG grammar");
    assert_eq!(
        DomainGapRecordV1::decode_witness_provenance(&wire).unwrap(),
        DomainGapWitnessProvenance::Unobserved,
        "the read consumed nothing, so it may not claim a bracket observation"
    );
    let decoded = DomainGapRecordV1::decode(&wire).expect("round-trips");
    assert_eq!(decoded.reason, DomainGapReason::SourceUnavailable);
    assert_eq!(decoded.part, DomainGapPart::Both);
    assert_eq!(decoded.target_hour_raw, hour(0));
    assert_eq!(decoded.range_start_hour_raw, hour(0));
    assert_eq!(decoded.range_end_hour_raw, hour(0));
    assert_eq!(
        decoded.outcome_ordinal, sentinel_ordinal,
        "the caller's advancing ordinal must reach the wire unchanged"
    );
}

// kills: a reset silently dropping the window state. The window lives in a
// process static OUTSIDE every tracker (the staged-tracker drop paths cannot
// touch it), and its ONLY two transitions are the injected stores proven here:
// Ready closes, rollover reopens. A rejected-bracket take/restore carries no
// window state at all, so it can change nothing.
#[test]
fn the_window_has_exactly_two_transitions_and_no_tracker_residence() {
    let mut before = cold_tracker();
    before.sweep_gen = Some(9);

    // The bracket carry moves discovery state but no window state.
    let carried = before.take_shared_sweep_discovery();
    let mut fresh = cold_tracker();
    fresh.restore_shared_sweep_discovery(carried);
    let window = AtomicBool::new(false);

    assert!(
        !domain53_failclosed_is_initial_warmup_in(
            "domain53_discovery_cold",
            window.load(Ordering::Relaxed)
        ),
        "a closed window stays closed across a bracket reset"
    );
}
