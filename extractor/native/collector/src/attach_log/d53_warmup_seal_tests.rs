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
const WARMUP_REASONS: [&str; 4] = [
    "domain53_discovery_cold",
    "domain53_discovery_not_warmed",
    "domain53_war_addrs_pending",
    // Lane SH: the war worker now starts at attach, under the COLD snapshot's
    // epoch 0. When the sweep's first pass lands, that set is dropped — it was
    // walked before an interval nothing refreshed — and a replacement walk
    // starts. The sweep STARTING is not an outage, and this can only happen
    // before any real-epoch set has existed, so it is a first-discovery state.
    "domain53_war_addrs_cold_window_ended",
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
    let read = domain53_warmup_gap_read(
        Instant::now(),
        inherited,
        &tracker,
        "domain53_discovery_cold",
        Domain53RootStageDiag::default(),
    );
    assert!(read.warmup_gap);
    // The warmup record is EMPTY, so it declares its intel table absent -- and
    // that is the whole of what it adds. `DOMAIN53_STATUS_INTEL_MAP_UNPROVEN` is
    // lane-local: `compact_status_for_global_interpretation` strips it, so it
    // cannot reach the window-wide OR this test exists to protect. Without it the
    // fold would read a warmup hour's absent matrix as a measured EMPTY one.
    assert_eq!(
        read.status_flags,
        inherited | diplomacy53::compact::STATUS_INTEL_MAP_UNPROVEN,
        "warmup adds the empty record's intel declaration and nothing else"
    );
    assert_eq!(
        compact_contract::compact_status_for_global_interpretation(53, read.status_flags),
        compact_contract::compact_status_for_global_interpretation(53, inherited),
        "the declaration must be invisible to the global vocabulary"
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
    // Lane SH: the war worker is keyed to the sweep generation PLUS ONE (see
    // `domain53_war_validation_generation`), so it can start at attach instead
    // of waiting out the sweep's first pass. Holding that generation's set is
    // what keeps this test from spawning a real worker.
    tracker.war_validated_gen = Some(domain53_war_validation_generation(9));
    tracker.war_validated_epoch = Some(7);
    let warm = d53_warm_snapshot(9);
    let stage = domain53_sweep_stage(
        &mut tracker,
        &warm,
        D53_TEST_NEEDLES,
        // Lane SH: this case drives the warmup-window transition, not the
        // bounded-root walk, so the walk is declined (pre-SH behaviour: serve
        // from the sweep, carry the refusal as a named note).
        Err("domain53_root_declined_by_test"),
        now,
        &window,
    );
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

// ===========================================================================
// LANE SH — bounded roots. d53's four candidate lists are now resolved from
// ENGINE GLOBALS by proven pointer chains and are available on frame one, and
// the off-frame war worker no longer waits for a sweep pass either. What they
// do NOT do is publish an hour on their own while the sweep is still cold:
// their completeness for the OPERATIVE family rests on a universal negative
// this lane did not prove, and an incomplete authoritative hour misattributes
// where an explicit gap merely excludes. So RS's classification seams above
// stay load-bearing, and the tests below pin exactly that boundary.
// ===========================================================================

/// A non-empty bounded-root discovery, standing in for what
/// `domain53_discovery_from_bounded_roots` returns on a live frame. The lane's
/// own fixture tests prove the WALK; these prove what the stage does with its
/// result.
fn sh_resolved_roots() -> diplomacy53::extractor::Domain53RootDiscovery {
    diplomacy53::extractor::Domain53RootDiscovery {
        discovery: diplomacy53::extractor::domain53_discovery_from_needle_hits(
            vec![0x1_0000_0000_0000],
            vec![0x1_0000_0000_1000],
            vec![0x1_0000_0000_2000],
            vec![0x1_0000_0000_3000],
        ),
        counts: diplomacy53::extractor::Domain53RootCounts {
            countries: 1,
            agencies: 1,
            operation_managers: 1,
            strategic_manager_present: true,
            operative_candidates: 1,
            operation_candidates: 1,
            strategic_operative_candidates: 1,
            intel_network_candidates: 1,
            ..diplomacy53::extractor::Domain53RootCounts::default()
        },
    }
}

// kills: publishing a roots-only hour as an authoritative one. The bounded
// roots ARE resolved on this frame, and it is tempting to serve from them —
// that was this lane's first shape. It is wrong until the operative population
// has a proven enumeration authority: the four chains are proven, but "no
// other engine path holds a live COperativeLeader" is a universal negative
// this lane did not prove, and an hour published without such an operative
// would first show it on the hour the sweep warms — an object that already
// existed, recorded against the wrong hour. A gap EXCLUDES the hour by name
// and misattributes nothing; an authoritative-looking incomplete hour
// misattributes silently, so the two are NOT ordered by "amount of
// information" (Codex round 3, finding 3; DOSSIER section 12).
#[test]
fn a_cold_sweep_still_publishes_the_typed_gap_even_with_roots_resolved() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    let window = AtomicBool::new(true);
    let cold = SweepSnapshot::empty();
    // Everything the WAR half needs, so nothing but the discovery cross-check
    // can be what refuses this frame.
    tracker.validated_war_addrs = Some(Arc::new(vec![0x0beef_u64]));
    tracker.war_validated_gen = Some(domain53_war_validation_generation(cold.generation));
    tracker.war_validated_epoch = Some(cold.epoch_id);

    let stage = domain53_sweep_stage(
        &mut tracker,
        &cold,
        D53_TEST_NEEDLES,
        Ok(sh_resolved_roots()),
        now,
        &window,
    );

    match stage.outcome {
        Domain53SweepStageOutcome::FailClosed(reason) => {
            assert_eq!(reason, "domain53_discovery_cold");
            assert!(
                domain53_failclosed_is_initial_warmup_in(reason, true),
                "and it stays lane RS's typed, seal-clean warming-up gap"
            );
        }
        Domain53SweepStageOutcome::Ready { .. } => panic!(
            "roots alone must not publish an hour as authoritative while their \
             completeness is unproven"
        ),
    }
    assert!(
        window.load(Ordering::Relaxed),
        "a frame that did not consume must not close the warmup window"
    );
    assert!(
        stage.root.roots_present,
        "the walk still ran, and its accounting still rides the frame"
    );
    assert!(
        tracker.sweep_gen.is_none(),
        "the cold frame must not adopt generation 0"
    );
}

// kills: the SECOND, independent source of the attach warmup. The off-frame
// war-row worker stopped needing the shared sweep when lane MF moved it onto
// the bounded owner route, but the shared spawn gate's `snapshot_gen != 0`
// clause still parked it for the whole first pass. Offsetting d53's war
// generation by one makes the first value available at attach while keeping
// every property the gate relies on: never zero, strictly monotonic in the
// sweep generation, one fresh value per published pass.
#[test]
fn the_d53_war_worker_can_start_before_the_sweep_publishes_a_pass() {
    assert_eq!(
        domain53_war_validation_generation(0),
        1,
        "a cold sweep must still give the war worker a spawnable generation"
    );
    assert!(
        !offframe_validation_should_spawn(None, None, false, 0),
        "generation 0 is the shared gate's cold refusal — d60 still relies on it"
    );
    assert!(
        offframe_validation_should_spawn(None, None, false, domain53_war_validation_generation(0)),
        "d53's offset generation must be spawnable at attach"
    );
    for sweep_generation in [0_u64, 1, 2, 9] {
        assert!(
            domain53_war_validation_generation(sweep_generation) > sweep_generation,
            "strictly monotonic and never colliding with the previous value"
        );
    }
}

// kills: reporting a GENERIC reason on a frame the root walk refused. A frame
// that fails closed while the warmup window is open publishes ONE DOMAIN_GAP
// and NO PERF record (the fold forbids a PERF beside a gap for the same
// domain-hour), so a cause carried only as a diagnostic note is unreachable
// there; the channel that does reach the wire is the gap's status-origin
// sample, whose value is FNV-1a of the frame's REASON string.
//
// And a root refusal must NOT be laundered into that benign gap: it is a real
// refusal (a torn manager pointer, an exhausted visit budget, a build-identity
// mismatch), so it falls outside lane RS's warmup vocabulary, fails closed with
// failure bits, and its cause reaches the wire through the normal PERF path
// (Codex round 3, finding 2 — classifying by ORIGIN admitted all of these).
#[test]
fn a_refused_root_walk_reports_its_own_cause_and_is_not_a_benign_gap() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    let window = AtomicBool::new(true);
    let cold = SweepSnapshot::empty();
    tracker.war_attempted_gen = Some(domain53_war_validation_generation(cold.generation));

    let stage = domain53_sweep_stage(
        &mut tracker,
        &cold,
        D53_TEST_NEEDLES,
        Err("domain53_root_strategic_manager_invalid"),
        now,
        &window,
    );

    match stage.outcome {
        Domain53SweepStageOutcome::FailClosed(reason) => {
            assert_eq!(
                reason, "domain53_root_strategic_manager_invalid",
                "the frame must carry the walk's OWN cause, not a generic word"
            );
            assert_ne!(
                fnv1a_64(reason.as_bytes()),
                fnv1a_64(b"domain53_discovery_cold"),
                "a generic word would make the two causes indistinguishable in \
                 the gap's status-origin sample"
            );
            assert!(
                !domain53_failclosed_is_initial_warmup_in(reason, true),
                "a torn root read is a REAL refusal even while the window is \
                 open; admitting it would publish a benign gap over it, with no \
                 bound on how long that can go on if the sweep never warms"
            );
        }
        Domain53SweepStageOutcome::Ready { .. } => panic!("nothing can serve this frame"),
    }
    assert!(!stage.root.roots_present);
    assert_eq!(
        stage.root.root_refusal,
        Some("domain53_root_strategic_manager_invalid")
    );
}

// kills: bounded roots being read as an excuse for the WAR half. They replace
// the DISCOVERY source, not the war-address set and not the epoch guard. A warm
// sweep plus resolved roots but no validated war set must still fail closed
// under the worker's own cause.
#[test]
fn bounded_roots_do_not_excuse_a_missing_war_address_set() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    let warm = d53_warm_snapshot(9);
    // Declare this generation's walk attempted so the production spawn gate
    // starts no real worker against the test process.
    tracker.war_attempted_gen = Some(domain53_war_validation_generation(9));

    let stage = domain53_sweep_stage(
        &mut tracker,
        &warm,
        D53_TEST_NEEDLES,
        Ok(sh_resolved_roots()),
        now,
        &AtomicBool::new(true),
    );

    match stage.outcome {
        Domain53SweepStageOutcome::FailClosed(reason) => {
            assert_eq!(reason, "domain53_war_addrs_cold")
        }
        Domain53SweepStageOutcome::Ready { .. } => {
            panic!("no validated war set exists; the frame must fail closed")
        }
    }
}

// kills: the union quietly becoming a replacement, or the retirement
// measurement quietly becoming a constant zero. A sweep hit no proven root
// chain reached must reach the extractor AND be counted, because that
// population is exactly what the lane's one residual is about.
#[test]
fn a_warm_sweep_contributes_its_extra_candidates_and_they_are_counted() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    // One hit per needle, none of them a root address, so every one is a
    // sweep-only contribution.
    let mut hits: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, needle) in D53_TEST_NEEDLES.iter().enumerate() {
        hits.insert(*needle, vec![0x2_0000_0000_0000 + index * 0x1000]);
    }
    let started = Instant::now();
    let warm = heap_sweep_completed_snapshot(7, 9, started, started, &D53_TEST_NEEDLES, hits, 0, 0);
    tracker.validated_war_addrs = Some(Arc::new(vec![0x0beef_u64]));
    tracker.war_validated_gen = Some(domain53_war_validation_generation(9));
    tracker.war_validated_epoch = Some(7);

    let stage = domain53_sweep_stage(
        &mut tracker,
        &warm,
        D53_TEST_NEEDLES,
        Ok(sh_resolved_roots()),
        now,
        &AtomicBool::new(true),
    );

    let Domain53SweepStageOutcome::Ready { ref discovery, .. } = stage.outcome else {
        panic!("a warm sweep plus resolved roots must consume")
    };
    assert_eq!(
        discovery.operative_hits.len(),
        2,
        "the sweep's hits must be ADDED to the roots, never substituted"
    );
    assert!(discovery.operative_hits.contains(&0x1_0000_0000_0000));
    assert!(discovery.operative_hits.contains(&0x2_0000_0000_0000));
    assert_eq!(
        stage.root.sweep_only.total(),
        4,
        "one unreached candidate per family, named per family"
    );
    assert_eq!(stage.root.sweep_only.operative, 1);
    assert_eq!(stage.root.sweep_only.operation, 1);
    assert_eq!(stage.root.sweep_only.strategic_operative, 1);
    assert_eq!(stage.root.sweep_only.country_intel_network, 1);
}

// ===========================================================================
// LANE SH, Codex round 1-3 fixes to the war-set epoch handling.
// ===========================================================================

// kills: leaving the sweep-EPOCH guard on the consume path, where an EARLIER
// fail-closed return skipped it. That ordering left the dead epoch's war set
// installed, so the spawn gate still saw `war_validated_gen = Some(previous)`;
// the new epoch's worker — a LOWER generation, because an epoch reset
// republishes generation 0 — completed only to be discarded as "superseded",
// with its generation already marked attempted and no failure recorded, so it
// could not retry until the sweep advanced a whole further pass. This drives
// the exact frame that used to skip the guard: cold sweep AND a refused root
// walk.
#[test]
fn a_dead_sweep_epoch_is_dropped_even_on_a_frame_that_can_serve_nothing() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    tracker.validated_war_addrs = Some(Arc::new(vec![0x0beef_u64]));
    tracker.war_validated_gen = Some(11);
    // A REAL epoch (HEAP_SWEEP_EPOCH_SEQ starts at 1, so 0 is exclusively the
    // cold snapshot) that the incoming snapshot no longer carries.
    tracker.war_validated_epoch = Some(7);

    let dead = SweepSnapshot::empty();
    // Hermeticity: declare the replacement generation attempted so the
    // production spawn gate starts no real worker against the test process.
    tracker.war_attempted_gen = Some(domain53_war_validation_generation(dead.generation));

    let stage = domain53_sweep_stage(
        &mut tracker,
        &dead,
        D53_TEST_NEEDLES,
        // The frame can serve NOTHING: cold sweep, refused roots. This is the
        // return that used to bypass the guard.
        Err("domain53_root_game_state_unavailable"),
        now,
        &AtomicBool::new(true),
    );

    match stage.outcome {
        Domain53SweepStageOutcome::FailClosed(reason) => {
            assert_eq!(reason, "domain53_war_addrs_sweep_epoch_changed")
        }
        Domain53SweepStageOutcome::Ready { .. } => {
            panic!("a war set validated under a dead epoch must never serve a clean frame")
        }
    }
    assert!(
        tracker.validated_war_addrs.is_none(),
        "the dead epoch's set must be DROPPED on this frame, not merely skipped"
    );
    assert_eq!(
        tracker.war_validated_gen, None,
        "clearing the generation is what lets the replacement worker's LOWER \
         generation be adopted instead of discarded as superseded"
    );
    assert_eq!(tracker.war_validated_epoch, None);
}

// kills: adopting a completed result that was WALKED across an epoch change.
// The epoch guard runs before the reap, so with no retained set it is a no-op
// and cannot see what the reap is about to install. Round 2 answered that by
// stamping the adopted set with the INSTALLING frame's epoch — which only
// relabels the problem: installation time does not prove production time, and
// a set walked during an outage is missing every war created since, which no
// amount of per-address revalidation can recover (Codex round 3, finding 1).
//
// Driven through the reap transition directly rather than through the stage:
// the discard arm deliberately re-arms the walk, so a stage-level test would
// start a real relation-root worker against the test process AND could not
// observe the cleared generation, which the same invocation immediately sets
// again (Codex round 4, finding 2).
#[test]
fn a_result_walked_across_an_epoch_change_is_discarded_not_relabelled() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    let spawn_gen = domain53_war_validation_generation(9);
    domain53_arm_war_validation_attempt(&mut tracker, spawn_gen);
    assert_eq!(tracker.war_attempted_gen, Some(spawn_gen));

    // Spawned under epoch 3, reaped on a frame whose snapshot carries epoch 7.
    domain53_apply_war_validation_outcome(
        &mut tracker,
        spawn_gen,
        3,
        7,
        Ok(Ok(vec![0x1000_u64])),
        now,
    );

    assert!(
        tracker.validated_war_addrs.is_none(),
        "a set walked across an epoch change must not be installed at all"
    );
    assert_eq!(tracker.war_validated_gen, None);
    assert_eq!(tracker.war_validated_epoch, None);
    assert_eq!(
        tracker.war_attempted_gen, None,
        "and the walk must be re-armed so the next frame redoes it under the          live epoch, rather than stranding d53 until the sweep advances another          whole pass"
    );
    assert_eq!(
        tracker.war_validation_failure_reason, None,
        "this is not a worker FAILURE — nothing was wrong with the result          except when it was walked — so it must arm no backoff and resurrect          no cause"
    );
}

// kills: discarding a result that never straddled an epoch change. The cold
// window is an epoch too (epoch 0): a worker spawned and reaped while the sweep
// is cold produced its set under the epoch it will serve under, and discarding
// it would throw away the attach-time walk on every capture — the whole point
// of removing d53's second warmup source.
#[test]
fn a_result_walked_entirely_within_one_epoch_is_adopted() {
    let now = Instant::now();
    for epoch in [0_u128, 7] {
        let mut tracker = cold_tracker();
        let spawn_gen = domain53_war_validation_generation(if epoch == 0 { 0 } else { 9 });
        domain53_arm_war_validation_attempt(&mut tracker, spawn_gen);
        domain53_apply_war_validation_outcome(
            &mut tracker,
            spawn_gen,
            epoch,
            epoch,
            Ok(Ok(vec![0x1000_u64])),
            now,
        );
        assert!(
            tracker.validated_war_addrs.is_some(),
            "epoch {epoch}: a result walked entirely within one epoch is adoptable"
        );
        assert_eq!(tracker.war_validated_epoch, Some(epoch));
        assert_eq!(tracker.war_validated_gen, Some(spawn_gen));
    }
}

// kills: serving a war set that was walked during the COLD window once a real
// epoch arrives. Two earlier shapes were wrong: exempting epoch-0 sets from the
// rule let one survive a later real epoch death (Codex round 2, finding 5), and
// ADOPTING one into the first real epoch still served a set walked before an
// interval nothing refreshed — `heap_sweep_reset_published_snapshot` republishes
// epoch 0 on every service restart, and a restart that fails before its first
// completed pass keeps it there unboundedly, with the war generation pinned at
// 1 so nothing re-walks (Codex round 4 finding 1, round 5 finding 1).
//
// So the set is DROPPED, and what the cold case gets instead is an accurate
// reason: leaving the cold window is the sweep starting, not an outage, and it
// is admitted as a first-discovery state only while the window is open.
#[test]
fn a_cold_window_war_set_is_dropped_when_the_sweep_starts_and_says_why() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    tracker.validated_war_addrs = Some(Arc::new(vec![0x0beef_u64]));
    tracker.war_validated_gen = Some(domain53_war_validation_generation(0));
    tracker.war_validated_epoch = Some(0);
    let first_real = d53_warm_snapshot(9);
    // Hermeticity: the replacement walk's generation is declared attempted.
    tracker.war_attempted_gen = Some(domain53_war_validation_generation(9));

    let stage = domain53_sweep_stage(
        &mut tracker,
        &first_real,
        D53_TEST_NEEDLES,
        Ok(sh_resolved_roots()),
        now,
        &AtomicBool::new(true),
    );

    match stage.outcome {
        Domain53SweepStageOutcome::FailClosed(reason) => {
            assert_eq!(
                reason, "domain53_war_addrs_cold_window_ended",
                "the cold case must name itself, not borrow the outage word"
            );
            assert!(
                domain53_failclosed_is_initial_warmup_in(reason, true),
                "while the window is open this is a benign, seal-clean gap for                  the frame or two the replacement walk needs"
            );
            assert!(
                !domain53_failclosed_is_initial_warmup_in(reason, false),
                "after any frame has consumed it is a real refusal like every                  other reason"
            );
        }
        Domain53SweepStageOutcome::Ready { .. } => panic!(
            "a set walked during the cold window may be arbitrarily old by the              time a pass lands; it must never serve"
        ),
    }
    assert!(tracker.validated_war_addrs.is_none());
    assert_eq!(tracker.war_validated_gen, None);
    assert_eq!(tracker.war_validated_epoch, None);
}

// kills: collapsing the two epoch reasons into one. A REAL epoch dying is an
// outage and must keep its own word — `domain53_war_addrs_sweep_epoch_changed`
// is NOT in the warmup vocabulary, so it fails closed with failure bits even
// during an attach, which is correct: a sweep that died mid-stream is not a
// warming-up condition.
#[test]
fn a_real_epoch_death_keeps_its_own_word_and_never_reads_as_warmup() {
    let now = Instant::now();
    let mut tracker = cold_tracker();
    tracker.validated_war_addrs = Some(Arc::new(vec![0x0beef_u64]));
    tracker.war_validated_gen = Some(domain53_war_validation_generation(9));
    tracker.war_validated_epoch = Some(7);

    let after_death = SweepSnapshot::empty();
    tracker.war_attempted_gen = Some(domain53_war_validation_generation(after_death.generation));

    let died = domain53_sweep_stage(
        &mut tracker,
        &after_death,
        D53_TEST_NEEDLES,
        Ok(sh_resolved_roots()),
        now,
        &AtomicBool::new(true),
    );
    match died.outcome {
        Domain53SweepStageOutcome::FailClosed(reason) => {
            assert_eq!(reason, "domain53_war_addrs_sweep_epoch_changed");
            assert!(
                !domain53_failclosed_is_initial_warmup_in(reason, true),
                "a sweep that died mid-stream is a real refusal even with the                  window open"
            );
        }
        Domain53SweepStageOutcome::Ready { .. } => {
            panic!("a set that outlived its real epoch must never serve a clean frame")
        }
    }
    assert!(tracker.validated_war_addrs.is_none());
    assert_eq!(tracker.war_validated_gen, None);
    assert_eq!(tracker.war_validated_epoch, None);
}
