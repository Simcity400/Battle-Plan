//! What d81 stopped doing inside its clock bracket, and what must not have
//! changed because of it.
//!
//! `rail_replay_state_live` held the frame's critical lane in 342 of t32's 583
//! frames at ~38.6 ms p50, while the bulk copies its reads perform measured
//! 0.2-0.4 ms hot: the span was compute, not bandwidth. Lane EK split the
//! bracket so that only observation happens inside it — the pointer-chasing
//! discovery, the bulk copies, and the witnesses — and every phase that reads
//! nothing runs afterwards on an owned, hour-stamped copy.
//!
//! THE PROOF OBLIGATIONS THAT COMES WITH, and each has a test below:
//!
//!  * the wire cannot tell. Same rows, same order, same probe counters, over
//!    one observation both halves see identically.
//!  * FIRST-FAILURE ORDER survives. Reads and emissions used to interleave per
//!    POV; they no longer do, so the order has to be reconstructed rather than
//!    inherited.
//!  * the refusal vocabulary survives. The gate's verdict and the sort's
//!    strictness check now sit on opposite sides of the bracket close and must
//!    still refuse together, under the same name.
//!  * a refused bracket is never computed on.
//!  * the copy is BOUNDED, and exhausting the bound refuses under a reason of
//!    its own rather than one borrowed from an engine-supplied count.
//!  * NOTHING THE POST-BRACKET HALF RUNS CAN OBSERVE. This is the one that no
//!    behavioural test can see: a stray guarded read there publishes plausible
//!    values taken outside the span the witnesses vouch for.
//!
//! WHAT THESE TESTS DO NOT PROVE. d81 has no end-to-end fixture — see the head
//! of `d81_frame_cost.rs` for why — so the byte-identity comparison is made at
//! the seam the split introduced, over a hub observation rather than over a
//! staged `CGameState`. That is exactly the boundary the split moved, and it is
//! the boundary at which "identical observed bytes" can be made exact instead of
//! approximate: both sides are handed the same observation.

use super::d81_frame_cost::{d81_hub_foreign_source_for, percentiles_us, t31, T31Frame};
use super::*;

/// A hub observation at a small, fully hand-specified shape.
///
/// Every value here is a decode of bytes the bracket would have observed, and
/// nothing in it is an address anything dereferences — which is the property the
/// split rests on and the reason this fixture needs no staged memory at all.
fn ek_observation() -> D81HubObservation {
    ek_observation_with_overlap(false)
}

/// The same shape, optionally with a province that carries a route in BOTH
/// subtype lists.
///
/// `overlap` is what makes the cross-subtype query memo reachable (lane FG,
/// 2026-08-08): with it false no province is queried twice and the memo can
/// never hit, so a differential run on that fixture alone would say nothing
/// about the memo at all. Province 11 is the one duplicated, because it is held
/// by both related tables and so its answer is a real maximum rather than a
/// zero an incorrect memo could also produce.
fn ek_observation_with_overlap(overlap: bool) -> D81HubObservation {
    let table = |rows: &[(u32, i64)]| D81HashTableSnapshot {
        rows: rows.iter().copied().collect(),
        witnesses: Vec::new(),
    };
    let mut foreign_tables = BTreeMap::new();
    foreign_tables.insert(0x1000, table(&[(11, 700), (12, 250)]));
    foreign_tables.insert(0x2000, table(&[(11, 400), (13, 990)]));

    let route = |province_id: u32, node_index: u32| D81RoutePair {
        province_id,
        node_index,
    };
    let pov = |pov_country_id: u32| D81HubPovPlan {
        pov_country_id,
        subtype0_routes: vec![(route(11, 0), false), (route(12, 1), true)],
        subtype1_routes: if overlap {
            vec![(route(11, 0), false), (route(13, 1), false)]
        } else {
            vec![(route(13, 1), false)]
        },
        controller_by_province: [(11u32, (5u32, 5u32)), (12, (0, 0)), (13, (9, 9))]
            .into_iter()
            .collect(),
        node_states: [(0u32, (3u32, 1u8, -7i64, 9i64)), (1, (4, 0, 11, -3))]
            .into_iter()
            .collect(),
        foreign_related: vec![(5, 5, 0x1000), (9, 9, 0x2000)],
    };
    D81HubObservation {
        foreign_tables,
        povs: vec![pov(2), pov(7)],
        read_failure: None,
        reread_refused: Some(false),
    }
}

/// THE PRE-SPLIT EMISSION LOOP, transcribed, as a reference implementation.
///
/// This is the code that used to run inside the reader's clock bracket at the
/// end of each POV iteration, with one change: it reads a POV's inputs out of a
/// plan instead of out of the surrounding scope's locals. It is a SEPARATE
/// transcription and not a call into production on purpose — a golden test whose
/// two sides are the same function proves nothing.
fn ek_reference_emission(
    observation: &D81HubObservation,
    phases: &D81PhaseAccum,
) -> Result<Vec<RailHubSimulationState>, D81FirstFailure> {
    let failure = |reason| D81FirstFailure::new(D81FailureStage::HubRoot, reason);
    let mut rows: Vec<RailHubSimulationState> = Vec::new();
    for plan in &observation.povs {
        let pov_country_id = plan.pov_country_id;
        let foreign_source = d81_hub_foreign_source_for(plan, &observation.foreign_tables);
        for (subtype, route_class, selected_routes) in [
            (0u8, 2u8, &plan.subtype0_routes),
            (1u8, 1u8, &plan.subtype1_routes),
        ] {
            for &(route, fallback_record) in selected_routes {
                let &(controller_raw_handle, controller_country_id) = plan
                    .controller_by_province
                    .get(&route.province_id)
                    .ok_or_else(|| {
                        failure(D81FailureReason::IdentityMismatch).subject(route.province_id)
                    })?;
                let province_owner_country_id = if fallback_record {
                    pov_country_id
                } else if controller_country_id != 0 {
                    controller_country_id
                } else {
                    return Err(
                        failure(D81FailureReason::IdentityMismatch).subject(route.province_id)
                    );
                };
                let &(node_kind, node_flag, available_raw, consumed_raw) =
                    plan.node_states.get(&route.node_index).ok_or_else(|| {
                        failure(D81FailureReason::OutOfBounds).index(route.node_index as usize)
                    })?;
                rows.push(RailHubSimulationState {
                    pov_country_id,
                    province_id: route.province_id,
                    province_owner_country_id,
                    subtype,
                    route_class,
                    node_kind,
                    node_flag,
                    available_raw,
                    consumed_raw,
                    foreign_max_raw: foreign_source.maximum_excluding_controller(
                        phases,
                        route.province_id,
                        controller_raw_handle,
                        controller_country_id,
                    ),
                    foreign_route: fallback_record,
                    fallback_record,
                });
                if rows.len() > rail_replay::MAX_HUB_SIMULATION_ROWS {
                    return Err(failure(D81FailureReason::OutOfBounds).raw(rows.len() as u64));
                }
            }
        }
    }
    rows.sort_unstable_by_key(|row| (row.pov_country_id, row.province_id, row.subtype));
    Ok(rows)
}

/// The rows the split publishes are the rows the undivided reader published —
/// same values, same order — and so are the probe counters underneath them.
///
/// THIS IS THE SPLIT'S WHOLE OBLIGATION. Moving work across the bracket close is
/// only admissible if the wire cannot tell.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL:
///  * reordering the two subtype passes, or swapping `route_class` 2 and 1
///    between them;
///  * taking the fallback branch's owner from the controller instead of the POV;
///  * dropping the sort, or sorting on a different key;
///  * building the probe list in any order but the plan's — the published answer
///    is a maximum and so order-independent, but `hub_probe_hits` is not;
///  * charging the probe counters anywhere but inside the query.
///
/// MUTATION RUN: `subtype1_routes` given `route_class` 2 instead of 1 in
/// `d81_compute_rail_hub_rows`. Observed:
///
/// ```text
/// assertion `left == right` failed: the deferred emission must publish the
/// pre-split rows exactly
/// ```
///
/// Restored.
///
/// TWO MORE, ON THE MEMO (lane FG, 2026-08-08):
///
///  * the memo's gate `pass == 1` changed to `pass == 9`, i.e. the memo never
///    consulted — the shape a lane that quietly dropped the dedup would leave:
///
/// ```text
/// a province routed under both subtypes must be answered from the memo, and
/// that must show up as probes never walked
/// ```
///
///  * the memo made to answer `answered[at].1 + 1` — a memo that hits but
///    lies, which the counter assertions alone cannot see:
///
/// ```text
/// assertion `left == right` failed: the deferred emission must publish the
/// pre-split rows exactly (overlap true)
/// ... foreign_max_raw: 401 ... vs ... foreign_max_raw: 400 ...
/// ```
///
/// Both restored. The first fires only on the `overlap` fixture, which is why
/// that fixture exists.
#[test]
fn the_deferred_hub_emission_is_byte_identical_to_the_in_bracket_one() {
    // BOTH SHAPES (lane FG, 2026-08-08): without a province in both subtype
    // lists, where the cross-subtype memo cannot hit and every counter must
    // still equal the reference's; and with one, where it hits and the
    // accounting identity is what has to hold instead.
    for overlap in [false, true] {
        let observation = ek_observation_with_overlap(overlap);

        let reference_phases = D81PhaseAccum::default();
        let reference = ek_reference_emission(&observation, &reference_phases)
            .expect("the fixture is a complete observation");

        let split_phases = D81PhaseAccum::default();
        let split = d81_compute_rail_hub_rows(&observation, &split_phases)
            .expect("the fixture is a complete observation");

        assert_eq!(
            split, reference,
            "the deferred emission must publish the pre-split rows exactly \
             (overlap {overlap})"
        );
        assert!(
            !split.is_empty(),
            "a fixture that emits nothing would satisfy the comparison above \
             without exercising it"
        );

        let reference_snapshot = reference_phases.snapshot();
        let split_snapshot = split_phases.snapshot();
        // THE ACCOUNTING IDENTITY, which is what survived the memo. The
        // reference performs one query per emitted row and never memoises, so
        // its call count IS the row count; production's calls plus its memo
        // hits must be that same number.
        assert_eq!(
            reference_snapshot.hub_query_calls,
            split.len() as u64,
            "the reference queries once per row, which is what makes it the \
             denominator (overlap {overlap})"
        );
        assert_eq!(
            split_snapshot.hub_query_calls + split_snapshot.hub_query_memo_hits,
            reference_snapshot.hub_query_calls,
            "every emitted row is either a query performed or a memo hit \
             (overlap {overlap})"
        );
        assert!(
            split_snapshot.hub_query_probes <= reference_snapshot.hub_query_probes
                && split_snapshot.hub_probe_hits <= reference_snapshot.hub_probe_hits,
            "the memo can only remove probe work, never add it (overlap {overlap})"
        );
        assert!(
            split_snapshot.hub_query_calls > 0 && split_snapshot.hub_probe_hits > 0,
            "counters that are zero on both sides compare equal and prove nothing"
        );
        if overlap {
            assert!(
                split_snapshot.hub_query_memo_hits > 0
                    && split_snapshot.hub_query_probes < reference_snapshot.hub_query_probes,
                "a province routed under both subtypes must be answered from the \
                 memo, and that must show up as probes never walked"
            );
        } else {
            assert_eq!(
                (
                    split_snapshot.hub_query_calls,
                    split_snapshot.hub_query_probes,
                    split_snapshot.hub_probe_hits,
                    split_snapshot.hub_query_memo_hits,
                ),
                (
                    reference_snapshot.hub_query_calls,
                    reference_snapshot.hub_query_probes,
                    reference_snapshot.hub_probe_hits,
                    0,
                ),
                "the probe counters are wire units too, and moving the queries \
                 across the bracket close must not change what they count"
            );
        }
    }
}

/// A read that refused at POV k publishes that refusal AFTER every POV before k
/// has emitted, and BEFORE the closing gate — exactly where it was published
/// when the loop returned early.
///
/// WHY THIS IS THE HARD PART OF THE SPLIT. Before it, a POV's reads and that
/// POV's emission were adjacent, so "first failure" was just "first statement to
/// return". After it, every read happens before every emission, and an emission
/// failure in POV k-1 that used to precede POV k's read failure would otherwise
/// be published second — or not at all.
///
/// MUTATION RUN: the `if let Some(read_failure)` block hoisted above the `for
/// plan in &observation.povs` loop in `d81_compute_rail_hub_rows`. Observed:
///
/// ```text
/// assertion `left == right` failed: an emission failure in a POV BEFORE the
/// refusing one happened first and must be published
///   left: Unreadable
///  right: OutOfBounds
/// ```
///
/// Restored.
#[test]
fn a_read_refusal_is_published_after_the_povs_that_completed_before_it() {
    let phases = D81PhaseAccum::default();
    let refusal = D81FirstFailure::new(D81FailureStage::HubObject, D81FailureReason::Unreadable)
        .subject(4_242);

    // A read that refused at the POV after the two that completed.
    let mut observation = ek_observation();
    observation.read_failure = Some(refusal);
    observation.reread_refused = None;
    let outcome = d81_compute_rail_hub_rows(&observation, &phases)
        .expect_err("a refused read refuses the hour");
    assert_eq!(
        (outcome.stage, outcome.reason, outcome.subject_id),
        (
            D81FailureStage::HubObject,
            D81FailureReason::Unreadable,
            4_242
        ),
        "the read's own refusal is the one published when every earlier POV \
         emitted cleanly"
    );

    // Now break the FIRST POV's emission. It happened before the refusing POV's
    // reads, so it is the failure the undivided reader would have published.
    let mut earlier = ek_observation();
    earlier.read_failure = Some(refusal);
    earlier.reread_refused = None;
    earlier.povs[0].node_states.remove(&1);
    let outcome = d81_compute_rail_hub_rows(&earlier, &phases)
        .expect_err("a POV that cannot resolve a node refuses the hour");
    assert_eq!(
        outcome.reason,
        D81FailureReason::OutOfBounds,
        "an emission failure in a POV BEFORE the refusing one happened first \
         and must be published"
    );
}

/// The closing gate's verdict and the sort's strictness check still refuse
/// together, and under the same name, though they now sit on opposite sides of
/// the bracket close.
#[test]
fn the_gate_verdict_and_the_sort_check_still_refuse_under_one_name() {
    let phases = D81PhaseAccum::default();

    let mut refused = ek_observation();
    refused.reread_refused = Some(true);
    let outcome = d81_compute_rail_hub_rows(&refused, &phases)
        .expect_err("a gate that saw a witness move refuses the hour");
    assert_eq!(
        (outcome.stage, outcome.reason),
        (D81FailureStage::HubRoot, D81FailureReason::ClosureChanged),
        "the verdict must refuse with the name the one `||` chain refused with"
    );

    // The same name from the other disjunct: two POVs sharing an id make the
    // sort non-strict, which is what the strictness check exists to catch.
    let mut duplicated = ek_observation();
    duplicated.povs[1].pov_country_id = duplicated.povs[0].pov_country_id;
    let outcome = d81_compute_rail_hub_rows(&duplicated, &phases)
        .expect_err("a non-strict sort refuses the hour");
    assert_eq!(
        (outcome.stage, outcome.reason),
        (D81FailureStage::HubRoot, D81FailureReason::ClosureChanged),
        "and so must the strictness check, which is the disjunct that moved"
    );
}

/// A refused bracket never reaches the compute, and its copy is dropped unread.
///
/// `D81ObserveOutcome::Complete` is how the in-bracket half says "this hour is
/// already decided". `d81_finish_rail_replay_state` must hand it straight back:
/// a finish that ran the deferred phases over a refused read would be computing
/// on a snapshot the bracket declined to vouch for.
#[test]
fn a_refused_observation_is_returned_untouched_and_never_computed_on() {
    let refusal =
        D81FirstFailure::new(D81FailureStage::RailRoots, D81FailureReason::InvalidPointer)
            .subject(77)
            .raw(0xDEAD_BEEF);
    let read = CompactRailReplayRead {
        plan: crate::attach_log::d81_copy_bracket::D81PlanCounters::default(),
        equality: crate::attach_log::d81_copy_bracket::D81EqualityCounters::default(),
        provinces: Vec::new(),
        hub_simulation: None,
        observed_neighbors: 0,
        read_time_us: 1_234,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        first_failure: Some(refusal),
        map_reciprocity_record_is_province: 3,
        map_reciprocity_key_is_province_id: 4,
        phase_times: D81PhaseTimes::default(),
        bracket_observed_hour_raw: None,
    };
    let finished = d81_finish_rail_replay_state(D81ObserveOutcome::Complete(read));
    assert_eq!(finished.first_failure, Some(refusal));
    assert_eq!(
        finished.read_time_us, 1_234,
        "and it is not re-timed by the finish"
    );
    assert_eq!(
        (
            finished.map_reciprocity_record_is_province,
            finished.map_reciprocity_key_is_province_id
        ),
        (3, 4),
        "nor recomputed"
    );
    assert!(
        finished.hub_simulation.is_none(),
        "a refused hour publishes no hub snapshot, and the compute that would \
         have built one must not have run"
    );
}

/// A REJECTED CLOCK BRACKET NEVER REACHES THE DEFERRED COMPUTE.
///
/// FOUND BY REVIEW, NOT BY A TEST (Codex 019fdc30-62fb-7643-a49f-5834726132eb).
/// The first version of `spawn_gated_split` called `compute(observed)`
/// unconditionally: a bracket whose two clock observations straddled an hour
/// boundary, or whose clock could not be read at all, still ran d81's whole
/// post-bracket compute before reconciliation replaced its record with a gap. No
/// wrong bytes were published — the gap replaces the record either way — but the
/// worker and its reader permit were spent on an answer nothing could publish,
/// and the copy-then-compute contract says a refused hour's copy is dropped
/// unread. `a_refused_observation_is_returned_untouched_and_never_computed_on`
/// above did not catch it: that covers an IN-READER refusal, which is a
/// different thing from the BRACKET refusing.
///
/// Driven through the real guard: the two halves are recorded and the verdict
/// is what the guard published, so this exercises the production wiring rather
/// than a transcription of it.
///
/// MUTATION RUN: the verdict check in `spawn_gated_split` replaced by an
/// unconditional `compute(observed)` — the shape the review found. Observed:
///
/// ```text
/// assertion failed: !computed.get()
/// a bracket the guard refused must not reach the deferred compute
/// ```
///
/// Restored.
///
/// LANE FG (2026-08-08) LOOKED AT THE RESIDUAL THIS TEST PINS AND FOUND ITS
/// STATED CAUSE WRONG, and the residual real anyway. The frame's label IS
/// available to a worker; what blocks the skip is the gap-precedence rule. The
/// assertions below keep the correct half of lane EK's claim, and
/// `a_label_mismatched_bracket_cannot_yet_skip_its_compute` pins the part that
/// actually blocks it.
#[test]
fn a_rejected_bracket_never_reaches_the_compute() {
    // The two locally-decidable rejections a worker can see for itself. A
    // straddled pair needs a moving clock, which no fixture has; an unreadable
    // clock is what the fixture-less lane tracker produces, and it travels the
    // identical code path -- `LocalBracketVerdict` is one value and the branch
    // reads it, not its cause.
    let unreadable = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: None,
        closing: None,
        worker_panicked: false,
    };
    assert_eq!(
        local_bracket_verdict(&unreadable),
        LocalBracketVerdict::ClockUnreadable
    );
    // Real raw hours, because `frame_label_is_a_real_hour` fails closed on a
    // value outside the plausible calendar window: a bracket labelled 10 takes
    // the "no real label" arm and never reaches the mismatch arm at all, which
    // would make the residual below look closed when it is not.
    let hour = super::state_fixture::fixture_game_hour_raw();
    let next_hour = hour + 1;
    let straddled = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(ClockObservation {
            hour,
            ordinal: 1,
            at_us: 0,
        }),
        closing: Some(ClockObservation {
            hour: next_hour,
            ordinal: 2,
            at_us: 1,
        }),
        worker_panicked: false,
    };
    assert_eq!(
        local_bracket_verdict(&straddled),
        LocalBracketVerdict::Straddled
    );
    let coherent = DomainClockBracket {
        closing: straddled.opening,
        ..straddled
    };
    assert_eq!(
        local_bracket_verdict(&coherent),
        LocalBracketVerdict::CoherentHour
    );
    // The local half and the full verdict cannot drift: the full one is written
    // in terms of the local one, and these are the three it must inherit.
    assert_eq!(
        bracket_hour_verdict(&unreadable, hour),
        BracketHourVerdict::ClockUnreadable
    );
    assert_eq!(
        bracket_hour_verdict(&straddled, hour),
        BracketHourVerdict::Straddled
    );
    assert_eq!(
        bracket_hour_verdict(&coherent, hour),
        BracketHourVerdict::InFrameHour
    );
    // THE LABEL MISMATCH IS NOT LOCAL, and since lane FJ the split refuses it
    // too -- under its own name, with the observed hour attached. The local
    // verdict still cannot see it, which is why the sink carries the FULL
    // verdict and the guard is handed the frame's label.
    assert_eq!(
        local_bracket_verdict(&coherent),
        LocalBracketVerdict::CoherentHour,
        "a bracket that observed the wrong hour is still LOCALLY coherent"
    );
    assert_eq!(
        bracket_hour_verdict(&coherent, next_hour),
        BracketHourVerdict::LabelMismatch
    );
    assert_eq!(
        split_bracket_refusal(&coherent, hour, HOUR_LABEL_MISMATCH_POLICY),
        None,
        "a bracket that observed the frame's own hour admits the compute"
    );
    assert_eq!(
        split_bracket_refusal(&unreadable, hour, HOUR_LABEL_MISMATCH_POLICY),
        Some(SplitBracketRefusal::BracketIncoherent)
    );
    assert_eq!(
        split_bracket_refusal(&straddled, hour, HOUR_LABEL_MISMATCH_POLICY),
        Some(SplitBracketRefusal::BracketIncoherent)
    );
    assert_eq!(
        split_bracket_refusal(&coherent, next_hour, HourLabelMismatchPolicy::Refuse),
        Some(SplitBracketRefusal::LabelMismatch {
            observed_hour_raw: u64::try_from(hour).expect("fixture hour fits u64"),
        }),
        "and a mismatched bracket refuses under a DIFFERENT name, carrying the \
         hour it actually observed -- collapsing the two would put the skip back \
         on the SourceUnavailable path lane FG refused"
    );

    // Now the production seam, through a real guard on a real tracker. The
    // fixture-less tracker cannot read a clock, so the guard publishes
    // `BracketIncoherent` and the compute must be skipped.
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let verdict: Cell<BracketGuardOutcome> = Cell::new(BracketGuardOutcome::default());
    let observed_ran = Cell::new(false);
    {
        let _guard = ReaderClockBracketGuard::new_reporting(
            &lanes,
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            Some(BracketVerdictSink {
                sink: &verdict,
                frame_hour_raw: hour,
            }),
        );
        observed_ran.set(true);
    }
    assert!(observed_ran.get(), "the observe half always runs");
    assert_eq!(
        verdict.get().refusal,
        Some(SplitBracketRefusal::BracketIncoherent),
        "a guard with no readable clock must publish its refusal, or the split \
         has nothing to branch on"
    );
    assert_eq!(
        verdict.get().observed_hour_raw,
        None,
        "and it must publish NO observation. A clock it could not read is not an \
         hour it observed, and seeding one would put a fabricated witness on \
         d81's gap record -- which is the whole defect the stamp closes"
    );
    // AND THE REASON STILL REACHES THE WIRE. Skipping the compute must not
    // skip the RECORD of the bracket: the gap and its reason are minted at
    // reconciliation from `take_clock_brackets`, so a guard that published a
    // refusal to the worker and recorded nothing would silently turn a
    // refused domain-hour into a missing one.
    let recorded = lanes.take_clock_brackets();
    assert_eq!(
        recorded
            .iter()
            .map(|bracket| bracket.tag)
            .collect::<Vec<_>>(),
        vec![COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE],
        "the refusing guard must still record its bracket"
    );
    assert_eq!(
        rejected_bracket_domains(&recorded, hour, HOUR_LABEL_MISMATCH_POLICY)
            .get(&COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE)
            .map(|&(reason, _, _)| reason),
        Some(compact_contract::domain_gap::DomainGapReason::ClockUnreadable),
        "reconciliation must name the same cause the worker refused for"
    );

    // The branch `spawn_gated_split` takes on that verdict, asserted against
    // the source: reaching the real one needs a live game process, and the
    // defect is exactly a missing branch.
    let source = collector_source_lf();
    let split = source
        .split_once("fn spawn_gated_split<")
        .expect("the split spawn")
        .1
        .split_once("\nfn ")
        .expect("the end of the split spawn")
        .0;
    assert!(
        split.contains("let guard_outcome = verdict.get();")
            && split.contains("let out = match guard_outcome.refusal {")
            && split.contains("None => compute(observed),")
            && !split.contains("Some(_) => compute(")
            && split.contains("drop(observed);"),
        "a bracket the guard refused must not reach the deferred compute, and \
         an incoherent bracket's copy must be dropped unread"
    );
    assert!(
        split.contains("let out = stamp(out, guard_outcome.observed_hour_raw);")
            && split.matches("verdict.get()").count() == 1,
        "and EVERY outcome must leave the worker stamped with the observation \
         of the SAME closed bracket the branch was taken on -- a second \
         `verdict.get()` is a second value the guard could have rewritten \
         between the branch and the stamp, and an unstamped path is one whose \
         gap record falls back to the frame's label (lane FQ)"
    );
    assert!(
        split.contains("rejected(refusal, None)")
            && split.contains("rejected(refusal, Some(observed))"),
        "and the refusal each arm publishes under must be the one the guard \
         decided -- see \
         `a_read_failure_from_the_observe_half_survives_a_label_mismatch_skip` \
         for why exactly one of the two arms keeps its observation"
    );
    assert!(
        split.contains("ReaderClockBracketGuard::new_reporting(")
            && split.contains("sink: &verdict,")
            && split.contains("frame_hour_raw,"),
        "and the guard must be the one that publishes the verdict, against the \
         label the caller will publish the frame under; a verdict computed \
         anywhere else is a second predicate that can drift"
    );

    // The rejected arm's own outcome refuses the hour rather than publishing an
    // empty one, and it does no work.
    let refused =
        compact_rail_replay_bracket_refused_read(0, SplitBracketRefusal::BracketIncoherent);
    assert!(refused.hub_simulation.is_none());
    assert_eq!(
        refused.status_flags,
        COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA
    );
    assert_eq!(
        refused.first_failure.map(|failure| failure.stage),
        Some(D81FailureStage::Worker)
    );
    assert_eq!(refused.phase_times, D81PhaseTimes::default());
}

/// THE SKIP, AND THE THREE LINKS THAT MAKE IT TRUTHFUL (lane FJ, 2026-08-08).
///
/// t37 turned 211 of d81's 621 frames into `HourLabelMismatch` gaps, and every
/// one of them ran the whole post-bracket compute first — `lw_d81_post_us`
/// averages 6.4 ms a frame. Lane EK did not skip them because it believed the
/// frame's label was unavailable to a worker; that was false. Lane FG found the
/// real blocker and REVERTED its own skip for it, leaving
/// `a_label_mismatched_bracket_cannot_yet_skip_its_compute` — a guard test
/// asserting a chain that made the skip publish a lie:
///
///  1. skipping means returning `compact_rail_replay_bracket_refused_read`,
///     which carries no `hub_simulation`, so d81's record is a DOMAIN_GAP;
///  2. that gap's reason was `SourceUnavailable` — a claim that the ENGINE could
///     not be read, which on a label-mismatched frame is simply false;
///  3. `replace_rejected_domain_records_with_gaps` DROPS a `HourLabelMismatch`
///     rejection when the domain already carries a gap, so the false reason is
///     the one that ships.
///
/// The chain is unchanged. What changed is link 2: the refused read now names
/// [`D81FailureReason::HourLabelMismatch`] and carries the hour it observed, so
/// the gap link 3 preserves is the SAME gap reconciliation would have minted.
/// This test asserts each link individually, so a regression on any one of them
/// fails here rather than silently restoring the swallowed reason.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: pointing the label-mismatch refusal
/// back at `D81FailureReason::Torn` (link 2 fails — that reason maps to
/// `SourceUnavailable`); dropping the observed hour from the refusal (link 2's
/// witness assertion fails); or making the yield rule replace the domain's own
/// mismatch gap (link 3 fails, with the ordinal spent twice).
#[test]
fn a_label_mismatched_bracket_skips_its_compute_under_its_own_reason() {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed_hour = hour + 1;
    let observed = u64::try_from(observed_hour).expect("fixture hour fits u64");

    // LINK 1 -- unchanged. The placeholder read cannot produce a payload, so
    // d81's record for that frame is a gap and not data.
    let mut refused = compact_rail_replay_bracket_refused_read(
        0,
        SplitBracketRefusal::LabelMismatch {
            observed_hour_raw: observed,
        },
    );
    assert!(
        encode_compact_rail_replay_production(&mut refused, 0).is_none(),
        "the bracket-refused read has no hub snapshot, so d81's record for that \
         frame is a DOMAIN_GAP"
    );

    // LINK 2 -- CLOSED. The reason is the mismatch's own, not a borrowed
    // `Torn`, and it carries the observed hour as its witness.
    assert_eq!(
        refused.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::HourLabelMismatch),
        "the label-mismatch refusal must not borrow `Torn`: the encode site maps \
         `Torn` to SourceUnavailable, which is the false claim this closes"
    );
    assert_eq!(
        refused.first_failure.map(|failure| failure.raw_value),
        Some(observed),
        "and it must carry the hour actually observed -- the encode site has no \
         other source for it and may not invent one"
    );
    assert_eq!(
        d81_failure_reason_unit_name(D81FailureReason::HourLabelMismatch),
        "d81_failure_reason_24_hour_label_mismatch",
        "a NEW assigned code: sharing `Torn`'s name would make an attribution \
         failure indistinguishable from a graph tear in the sealed diagnostics"
    );
    let source = collector_source_lf();
    assert!(
        source.contains(
            "Some(D81FailureReason::Torn | D81FailureReason::ClosureChanged) => {\n\
             \x20                   DomainGapReason::SourceUnavailable\n\
             \x20               }"
        ),
        "`Torn` must still map to SourceUnavailable -- that mapping is exactly \
         why the mismatch needed a reason of its own"
    );
    assert!(
        source.contains("Some(D81FailureReason::HourLabelMismatch) => match d81_mismatch_witness {")
            && source.contains("Some(_) => DomainGapReason::HourLabelMismatch,"),
        "and the new reason must reach the wire as the mismatch, gated on its \
         witness being present"
    );
    // The payload that pairing produces is one the wire's self-proving grammar
    // ACCEPTS. Asserted by encoding it, not by reading the grammar: a witness
    // that cannot prove the claim fails `encode`, and at the production site
    // that is an `expect` on the publication path.
    let staged = DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::HourLabelMismatch,
        target_hour_raw: hour,
        range_start_hour_raw: hour,
        range_end_hour_raw: hour,
        tick_before: observed,
        tick_after: observed,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    };
    let staged_bytes = staged
        .encode()
        .expect("the skip's own payload must be one the wire can carry");

    // LINK 3 -- the yield rule still yields, and that is now the RIGHT answer.
    // The bracket rejection and the domain's own record are the same claim over
    // the same observation, so the record that ships is the one already in the
    // buffer, with its staged ordinal intact.
    let mismatched = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(clock_obs(observed_hour, 1)),
        closing: Some(clock_obs(observed_hour, 2)),
        worker_panicked: false,
    };
    assert_eq!(
        bracket_hour_verdict(&mismatched, hour),
        BracketHourVerdict::LabelMismatch
    );
    assert_eq!(
        rejected_bracket_domains(
            std::slice::from_ref(&mismatched),
            hour,
            HourLabelMismatchPolicy::Refuse
        )
        .get(&COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE)
        .copied(),
        Some((DomainGapReason::HourLabelMismatch, observed, observed)),
        "reconciliation names the mismatch and witnesses the same hour the \
         worker did -- one predicate, two callers"
    );
    // And the whole seam, over a real frame buffer.
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        hour,
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &staged_bytes,
    );
    let before = records.clone();
    let key = (
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        DomainGapPart::State.code(),
    );
    let mut ordinals = std::collections::BTreeMap::from([(key, 1u64)]);
    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        std::slice::from_ref(&mismatched),
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("a skipped mismatch frame reconciles");
    assert!(
        rejected.is_empty(),
        "the domain already published the mismatch, so nothing is replaced"
    );
    assert_eq!(
        records, before,
        "and the bytes that ship are the ones the skip wrote -- reason, \
         witnesses and outcome ordinal all its own"
    );
    assert_eq!(
        ordinals.get(&key),
        Some(&1),
        "one terminal outcome, one ordinal"
    );
}

/// The skipped frame publishes no `source_unavailable` claim about the engine.
///
/// The gap reason is the load-bearing statement, but it is not the only one a
/// consumer reads: `COMPACT_STATUS_SOURCE_UNAVAILABLE` drives
/// `compact_source_unavailable` in the host's problem list and its own row in
/// `report_raw_capture.py`. A mismatch shape that set it would put the same
/// false claim on a second channel.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: giving both refusal shapes the same
/// status word again.
#[test]
fn the_skipped_frame_publishes_no_source_unavailable_claim() {
    let mismatch = compact_rail_replay_bracket_refused_read(
        0,
        SplitBracketRefusal::LabelMismatch {
            observed_hour_raw: 7,
        },
    );
    assert_eq!(
        mismatch.status_flags, COMPACT_STATUS_PARTIAL_DATA,
        "the engine was read perfectly; only its attribution failed"
    );
    let incoherent =
        compact_rail_replay_bracket_refused_read(0, SplitBracketRefusal::BracketIncoherent);
    assert_eq!(
        incoherent.status_flags,
        COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        "a bracket that could not vouch for its observation is unchanged"
    );
    // Neither reaches the wire today -- both have `hub_simulation: None`, so the
    // gap branch publishes status 0 on probe 250 and never d81's own word. That
    // is a property of today's encode site, which is why the values above are
    // written truthfully rather than left inert.
    assert!(mismatch.hub_simulation.is_none() && incoherent.hub_simulation.is_none());
}

/// A mismatch gap and the bracket behind it must witness the SAME hour.
///
/// The yield rule keeps the domain's own record and discards the rejection, so
/// the rejection's witnesses never reach the wire — which means a skip that
/// published a different observed hour than reconciliation computed would ship
/// unchallenged, and nothing downstream could tell. Hour misattribution is the
/// project's stop condition, so the disagreement is a frame error.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: deleting the equality check from the
/// yield rule, or letting the skip's witness come from anywhere other than the
/// bracket the guard closed.
#[test]
fn a_mismatch_gap_and_its_bracket_must_witness_the_same_hour() {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed_hour = hour + 1;
    let observed = u64::try_from(observed_hour).expect("fixture hour fits u64");
    let mismatched = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(clock_obs(observed_hour, 1)),
        closing: Some(clock_obs(observed_hour, 2)),
        worker_panicked: false,
    };
    // A gap claiming a DIFFERENT observed hour than the bracket recorded. Still
    // a well-formed payload -- the grammar checks the witness against the target
    // hour, and this one is nonzero and not the label -- so nothing but this
    // rule can see it.
    let lying = DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::HourLabelMismatch,
        target_hour_raw: hour,
        range_start_hour_raw: hour,
        range_end_hour_raw: hour,
        tick_before: observed + 5,
        tick_after: observed + 5,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    }
    .encode()
    .expect("a disagreeing witness is still a legal payload, which is the point");
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        hour,
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &lying,
    );
    let mut ordinals = std::collections::BTreeMap::from([(
        (
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            DomainGapPart::State.code(),
        ),
        1u64,
    )]);
    let error = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        std::slice::from_ref(&mismatched),
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect_err("a witness disagreement must not reach the wire");
    assert!(
        error.contains("HourLabelMismatch gap witnessing hour"),
        "and it must say which two hours disagreed: {error}"
    );
}

/// Every OTHER existing gap still outranks a label mismatch, exactly as before.
///
/// The yield rule was made reason-aware; it was not made reason-selective. A
/// domain whose own record says the READ failed keeps saying that — the mismatch
/// names only the attribution, and a read failure is the more specific fact.
/// Symmetrically, a torn or unreadable bracket still REPLACES whatever the
/// domain published, because those say the read itself was invalid.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: making a mismatch rejection overwrite
/// an existing gap, or making the new mismatch handling swallow the tear
/// precedence on its way past.
#[test]
fn a_read_failure_still_outranks_a_label_mismatch() {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed_hour = hour + 1;
    let key = (
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        DomainGapPart::State.code(),
    );
    // THE TICK PAIR IS THE HOUR THE READ HAPPENED IN, which on a mismatched
    // frame is the OBSERVED hour and not the label. Written that way here on
    // purpose: d81's encode site currently fills a `SourceUnavailable` tick pair
    // from `game_hour_raw`, which is right on every in-hour frame and wrong on
    // this one, and a fixture that copied the label would quietly enshrine that
    // as the expected shape. What this test asserts is PRECEDENCE -- which
    // record survives -- and it must not double as an endorsement of the
    // witnesses that record carries.
    let source_unavailable = DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::SourceUnavailable,
        target_hour_raw: hour,
        range_start_hour_raw: hour,
        range_end_hour_raw: hour,
        tick_before: u64::try_from(observed_hour).expect("fixture hour fits u64"),
        tick_after: u64::try_from(observed_hour).expect("fixture hour fits u64"),
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    }
    .encode()
    .expect("a genuine read failure d81 staged for itself");

    let frame = |payload: &[u8]| {
        let mut records = Vec::new();
        append_compact_record(
            &mut records,
            1,
            hour,
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
            0,
            payload,
        );
        records
    };
    let reason_of = |records: &[u8]| {
        let header = compact_contract::parse_compact_record_header(records).expect("H4DG header");
        DomainGapRecordV1::decode(
            &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
        )
        .expect("H4DG payload")
        .reason
    };

    // A MISMATCHED bracket over a genuine read failure: the read failure stands.
    let mismatched = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(clock_obs(observed_hour, 1)),
        closing: Some(clock_obs(observed_hour, 2)),
        worker_panicked: false,
    };
    let mut records = frame(&source_unavailable);
    let mut ordinals = std::collections::BTreeMap::from([(key, 1u64)]);
    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        std::slice::from_ref(&mismatched),
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("reconciliation");
    assert!(rejected.is_empty());
    assert_eq!(
        reason_of(&records),
        DomainGapReason::SourceUnavailable,
        "the engine really could not be read; the attribution failure does not \
         overwrite that"
    );

    // A TORN bracket over a mismatch gap: the tear replaces it.
    let torn = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(clock_obs(hour, 1)),
        closing: Some(clock_obs(hour + 1, 2)),
        worker_panicked: false,
    };
    let staged_mismatch = DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::HourLabelMismatch,
        target_hour_raw: hour,
        range_start_hour_raw: hour,
        range_end_hour_raw: hour,
        tick_before: u64::try_from(observed_hour).expect("fixture hour fits u64"),
        tick_after: u64::try_from(observed_hour).expect("fixture hour fits u64"),
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    }
    .encode()
    .expect("a skipped mismatch frame");
    let mut records = frame(&staged_mismatch);
    let mut ordinals = std::collections::BTreeMap::from([(key, 1u64)]);
    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        std::slice::from_ref(&torn),
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("reconciliation");
    assert_eq!(
        rejected,
        std::collections::BTreeSet::from([COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE])
    );
    assert_eq!(
        reason_of(&records),
        DomainGapReason::Torn,
        "a bracket that tore says the read itself was invalid, which outranks \
         anything the reader thought it was reporting"
    );
}

/// A bracket whose observed hour cannot serve as a wire witness refuses the
/// domain-hour WITHOUT killing the frame.
///
/// `HourLabelMismatch` is the wire's one self-proving reason: its grammar
/// demands two equal, NONZERO ticks that differ from the target hour, because
/// the observed hour is the reason's entire content. `clock_tick_witness`
/// collapses every hour it cannot fit in a `u64` to 0, the wire's "no witness"
/// value — so a negative clock reading used to build a payload `encode` REFUSES,
/// which returned `Err` from reconciliation and took the whole frame, including
/// every genuine tear in it, down with it.
///
/// The domain-hour is still refused; the owner's ruling that a mismatched sweep
/// loses its hour visibly is untouched. It is refused under `ClockUnreadable`,
/// whose required zero ticks say exactly what such a reading supports.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: minting `HourLabelMismatch` from a
/// bracket whose witness `hour_label_mismatch_witness` rejects — the frame then
/// fails to reconcile at all, and the assertion below is an `expect`.
#[test]
fn an_unusable_observed_hour_refuses_without_killing_the_frame() {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let hour = super::state_fixture::fixture_game_hour_raw();
    // A negative engine reading: a real hour label, an observation that is not
    // one. `LabelMismatch` by verdict, unusable as a witness.
    let unusable = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(clock_obs(-5, 1)),
        closing: Some(clock_obs(-5, 2)),
        worker_panicked: false,
    };
    assert_eq!(
        bracket_hour_verdict(&unusable, hour),
        BracketHourVerdict::LabelMismatch,
        "the verdict is unchanged -- it is the WITNESS that cannot be built"
    );
    assert_eq!(hour_label_mismatch_witness(&unusable, hour), None);
    // THE CAUSE STAYS THE MISMATCH IN THE REJECTION MAP. That map is what the
    // precedence rule reads, and the cause has not changed just because the wire
    // cannot carry its witness. Downgrading here would make this rejection
    // outrank a genuine read failure the domain had already published, which is
    // the inversion `a_mismatch_without_a_witness_still_yields_to_a_read_failure`
    // exercises end to end.
    assert_eq!(
        rejected_bracket_domains(
            std::slice::from_ref(&unusable),
            hour,
            HourLabelMismatchPolicy::Refuse
        )
        .get(&COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE)
        .copied(),
        Some((DomainGapReason::HourLabelMismatch, 0, 0)),
        "still refused, still as a mismatch, with the wire's no-witness value"
    );
    assert_eq!(
        split_bracket_refusal(&unusable, hour, HourLabelMismatchPolicy::Refuse),
        None,
        "and the split does NOT skip: the compute runs as before, which costs \
         6.4 ms and lies about nothing"
    );

    // THE RECORD is what gives, at the mint site. d81 published DATA, so there
    // is no read failure to preserve and the rejection really does replace it --
    // under `ClockUnreadable`, whose grammar requires the same two zeros.
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        hour,
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    let healthy: u16 = 10;
    append_compact_record(
        &mut records,
        2,
        hour,
        healthy,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[9u8; 8],
    );
    let healthy_bracket = DomainClockBracket {
        tag: healthy,
        opening: Some(clock_obs(hour, 3)),
        closing: Some(clock_obs(hour, 4)),
        worker_panicked: false,
    };
    let mut ordinals = std::collections::BTreeMap::new();
    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        &[unusable, healthy_bracket],
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("an unusable mismatch witness must not fail the whole frame");
    assert_eq!(
        rejected,
        std::collections::BTreeSet::from([COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE]),
        "the domain-hour is still refused -- the owner's ruling is untouched"
    );
    // Walk the frame: the healthy domain keeps its DATA, and d81's replacement
    // is an encodable gap. Before the mint-site downgrade the call above
    // returned `Err` and this whole frame -- healthy domain included -- was lost.
    let mut offset = 0usize;
    let mut seen = Vec::new();
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..]).unwrap();
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP {
            let gap = DomainGapRecordV1::decode(
                &records[offset + compact_contract::COMPACT_RECORD_HEADER_LEN..offset + header.record_len()],
            )
            .expect("the replacement must satisfy the H4DG grammar");
            assert_eq!(gap.reason, DomainGapReason::ClockUnreadable);
            assert_eq!((gap.tick_before, gap.tick_after), (0, 0));
        }
        seen.push((header.domain_id, header.record_type));
        offset += header.record_len();
    }
    assert!(
        seen.contains(&(healthy, COMPACT_RECORD_TYPE_DOMAIN_DATA)),
        "the healthy domain's DATA record survives: {seen:?}"
    );
    assert!(
        seen.contains(&(
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP
        )),
        "and d81's DATA is replaced by a gap: {seen:?}"
    );

    // AND THE PRECEDENCE HOLDS WITHOUT A WITNESS. The same unusable bracket over
    // a domain that already published a genuine read failure leaves that failure
    // alone: the cause is an attribution failure either way, and a read failure
    // is the more specific fact.
    // Witnesses that are NOT a copy of the label, for the reason given in
    // `a_read_failure_still_outranks_a_label_mismatch`: this test is about which
    // record survives, and must not also stand as an expectation about the
    // witnesses that record carries. -5 is what this bracket observed.
    let source_unavailable = DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::SourceUnavailable,
        target_hour_raw: hour,
        range_start_hour_raw: hour,
        range_end_hour_raw: hour,
        tick_before: 7,
        tick_after: 7,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    }
    .encode()
    .expect("a genuine read failure d81 staged for itself");
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        hour,
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &source_unavailable,
    );
    let before = records.clone();
    let mut ordinals = std::collections::BTreeMap::from([(
        (
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            DomainGapPart::State.code(),
        ),
        1u64,
    )]);
    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        std::slice::from_ref(&unusable),
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("reconciliation");
    assert!(rejected.is_empty());
    assert_eq!(
        records, before,
        "the engine really could not be read; a mismatch that cannot even name \
         the hour it observed must not overwrite that with `clock_unreadable`"
    );
}

/// A READ FAILURE PRODUCED INSIDE THE BRACKET SURVIVES THE SKIP.
///
/// Found by review, not by a test (Codex 019fdeb9-2ff3-7700-b799-737eff4c4302,
/// finding 1). The first version of the skip dropped the observe half's outcome
/// unconditionally and returned the placeholder, exactly as the incoherent-
/// bracket arm does. That arm is right to: reconciliation replaces its record
/// whatever it says. The MISMATCH arm is not, because its record is the one that
/// ships — so a d81 whose reader had already failed inside the bracket
/// (`EmptySnapshot`, `InvalidPointer`, any of the nineteen) would have published
/// `HourLabelMismatch`: "the engine read fine, only the hour was wrong", about a
/// frame where the engine did not read at all. That is this lane's own defect
/// pointed the other way, and it is the same precedence
/// `replace_rejected_domain_records_with_gaps` states.
///
/// There is also nothing to save: `D81ObserveOutcome::Complete` is terminal, and
/// `d81_finish_rail_replay_state` returns it untouched, so no deferred compute
/// runs on it in the first place.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: returning the placeholder for a
/// `Complete` observation on the mismatch arm, or handing the observation to the
/// incoherent arm (whose copy must still be dropped unread).
#[test]
fn a_read_failure_from_the_observe_half_survives_a_label_mismatch_skip() {
    let observed_hour_raw = 42u64;
    let mismatch = SplitBracketRefusal::LabelMismatch { observed_hour_raw };

    // The observe half's own terminal answer, with a reason of its own.
    let already_failed = CompactRailReplayRead {
        plan: crate::attach_log::d81_copy_bracket::D81PlanCounters::default(),
        equality: crate::attach_log::d81_copy_bracket::D81EqualityCounters::default(),
        provinces: Vec::new(),
        hub_simulation: None,
        observed_neighbors: 0,
        read_time_us: 4_321,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        first_failure: Some(D81FirstFailure::new(
            D81FailureStage::HubObject,
            D81FailureReason::EmptySnapshot,
        )),
        map_reciprocity_record_is_province: 5,
        map_reciprocity_key_is_province_id: 6,
        phase_times: D81PhaseTimes::default(),
        bracket_observed_hour_raw: None,
    };
    let kept = d81_bracket_refused_outcome(
        mismatch,
        Some(D81ObserveOutcome::Complete(already_failed)),
        0,
    );
    assert_eq!(
        kept.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::EmptySnapshot),
        "a read that already failed keeps its OWN reason; the encode site turns \
         this into `SourceUnavailable`, which is true of this frame and is what \
         must outrank the attribution failure"
    );
    assert_eq!(
        kept.read_time_us, 4_321,
        "and the whole outcome is handed back untouched, not rebuilt"
    );
    assert_eq!(
        (
            kept.map_reciprocity_record_is_province,
            kept.map_reciprocity_key_is_province_id
        ),
        (5, 6)
    );

    // A DEFERRED observation on the same refusal IS the skip: nothing terminal
    // exists yet, so the copy is dropped UNCOMPUTED and the mismatch is named.
    // Driven through a real `Deferred`, not through `None`, because those are
    // different arms of the handler and only this one can prove the copy went
    // unread.
    // A three-province ring, so every directed edge has its reciprocal and the
    // deferred fold runs to completion rather than refusing part-way. An empty
    // province list would refuse `EmptySnapshot`, and then "the compute did not
    // run" and "the compute ran and refused" would look identical.
    let ring: Vec<RailProvinceState> = (1..=3u32)
        .map(|province_id| RailProvinceState {
            province_id,
            active_terminal: false,
            anchor_variant: 0,
            anchor_bits: [0; 3],
            neighbors: (1..=3u32)
                .filter(|neighbor| *neighbor != province_id)
                .map(|neighbor| RailNeighborState {
                    province_id: neighbor,
                    edge_level: 1,
                })
                .collect(),
        })
        .collect();
    let observed_neighbors = ring
        .iter()
        .map(|province| province.neighbors.len())
        .sum::<usize>();
    // SINCE THE CONVERSION THE COPY IS WHAT CROSSES THE BRACKET, not the rows.
    // The compute half decodes the plan's snapshot, so a fixture that handed it a
    // ready-made row list would be a fixture for a design that no longer exists —
    // and, worse, one that could not tell "the compute did not run" from "there
    // was nothing left to compute". `staged_deferred` carries a REAL copy of a
    // staged world whose neighbours are reciprocal, so the control below
    // genuinely publishes.
    let _ = (&ring, observed_neighbors);
    let deferred_copy =
        || D81ObserveOutcome::Deferred(Box::new(super::d81_authoritative::staged_deferred()));
    // THE CONTROL, and what makes the assertion below mean anything: this exact
    // copy DOES compute to a published snapshot when it is not refused.
    let computed = d81_finish_rail_replay_state(deferred_copy());
    assert!(
        computed.hub_simulation.is_some() && computed.first_failure.is_none(),
        "the fixture must be a copy the compute half would publish, or 'the \
         compute did not run' is indistinguishable from 'there was nothing to \
         run'"
    );

    let deferred = d81_bracket_refused_outcome(mismatch, Some(deferred_copy()), 0);
    assert!(
        deferred.hub_simulation.is_none(),
        "the deferred copy must be dropped UNREAD -- a hub snapshot here is the \
         compute having run on an hour the bracket refuses"
    );
    assert_eq!(
        deferred.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::HourLabelMismatch)
    );
    assert_eq!(
        deferred.first_failure.map(|failure| failure.raw_value),
        Some(observed_hour_raw)
    );

    // And the incoherent arm is untouched: it never receives an observation at
    // all, because `spawn_gated_split` has already dropped its copy unread.
    let incoherent =
        d81_bracket_refused_outcome(SplitBracketRefusal::BracketIncoherent, None, 0);
    assert_eq!(
        incoherent.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::Torn)
    );

    // The wiring, asserted against the source: reaching the real branch needs a
    // live game process, and the defect is exactly which arm gets the copy.
    let source = collector_source_lf();
    let split = source
        .split_once("fn spawn_gated_split<")
        .expect("the split spawn")
        .1
        .split_once("\nfn ")
        .expect("the end of the split spawn")
        .0;
    assert!(
        split.contains("Some(refusal @ SplitBracketRefusal::BracketIncoherent) => {")
            && split.contains("drop(observed);")
            && split.contains("rejected(refusal, None)"),
        "a bracket that cannot vouch for its observation still has its copy \
         dropped unread"
    );
    assert!(
        split.contains("Some(refusal @ SplitBracketRefusal::LabelMismatch { .. }) => {")
            && split.contains("rejected(refusal, Some(observed))"),
        "and a label-mismatched bracket hands its observation to the handler, \
         which is the only way a terminal read failure can survive the skip"
    );
}

/// `CountOnly` never skips the compute.
///
/// The policy switch's standing claim is that it changes only WHETHER a
/// mismatched domain is refused. Under `CountOnly` a mismatched bracket is
/// counted and its domain still publishes its rows — so a split that skipped
/// regardless would DELETE data the policy says to keep, which is a change of a
/// different kind entirely.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping the policy gate from
/// `split_bracket_refusal`, or reading `HOUR_LABEL_MISMATCH_POLICY` inside it
/// instead of taking it as an argument (the second makes this test impossible to
/// write, which is why the argument exists).
#[test]
fn the_count_only_policy_never_skips_the_compute() {
    let hour = super::state_fixture::fixture_game_hour_raw();
    let mismatched = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(clock_obs(hour + 1, 1)),
        closing: Some(clock_obs(hour + 1, 2)),
        worker_panicked: false,
    };
    assert_eq!(
        split_bracket_refusal(&mismatched, hour, HourLabelMismatchPolicy::CountOnly),
        None,
        "under CountOnly the domain still publishes, so its compute must run"
    );
    assert!(
        split_bracket_refusal(&mismatched, hour, HourLabelMismatchPolicy::Refuse).is_some(),
        "and under Refuse it must not"
    );
    // The two locally-decidable refusals are policy-INDEPENDENT: the bracket
    // could not vouch for its observation, which no mismatch policy makes true.
    let unreadable = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: None,
        closing: None,
        worker_panicked: false,
    };
    for policy in [
        HourLabelMismatchPolicy::Refuse,
        HourLabelMismatchPolicy::CountOnly,
    ] {
        assert_eq!(
            split_bracket_refusal(&unreadable, hour, policy),
            Some(SplitBracketRefusal::BracketIncoherent),
            "{policy:?}"
        );
    }
}

/// The owned copy is bounded, and exhausting the bound refuses the domain-hour
/// under its own assigned reason rather than under a borrowed one.
#[test]
fn the_snapshot_budget_refuses_under_its_own_assigned_reason() {
    let mut budget = D81SnapshotBudget::new(100);
    assert!(budget.charge(60).is_ok());
    assert!(budget.charge(40).is_ok(), "exactly at the limit is admitted");
    assert_eq!(
        budget.charge(1),
        Err(101),
        "and the refusal carries the footprint that broke it, so a capture can \
         say how far over the copy went"
    );

    assert_eq!(
        d81_failure_reason_unit_name(D81FailureReason::SnapshotBudgetExhausted),
        "d81_failure_reason_23_snapshot_budget_exhausted",
        "the reason is named on the wire; sharing `OutOfBounds` would make OUR \
         buffer running out indistinguishable from the engine handing us an \
         impossible count"
    );
    assert_ne!(
        d81_failure_reason_unit_name(D81FailureReason::SnapshotBudgetExhausted),
        d81_failure_reason_unit_name(D81FailureReason::OutOfBounds),
    );

    // The budget is charged where the copy grows, and both growth sites are
    // charged. Asserted against the source: reaching either from a fixture needs
    // a 64 MiB staged tree.
    let source = collector_source_lf();
    let read = source
        .split_once("fn d81_read_foreign_source(")
        .expect("the foreign read")
        .1
        .split_once("\nfn ")
        .expect("the end of the foreign read")
        .0;
    assert!(
        read.contains("if let Err(used) = budget.charge(bytes) {"),
        "every table that enters the owned cache must be charged, or the \
         largest thing the copy holds sits outside the bound"
    );
    let hub = source
        .split_once("fn read_compact_rail_hub_simulation(")
        .expect("the hub read")
        .1
        .split_once("\nfn ")
        .expect("the end of the hub read")
        .0;
    let charge_index = hub
        .find("if let Err(used) = budget.charge(plan.owned_bytes()) {")
        .expect("every retained POV plan must be charged");
    let push_index = hub.find("povs.push(plan);").expect("the plan push");
    assert!(
        charge_index < push_index,
        "and charged BEFORE it is retained; charging afterwards admits one \
         unbounded plan per frame"
    );
}

/// NOTHING THE POST-BRACKET HALF RUNS CAN OBSERVE.
///
/// The snapshot-law amendment legalises computing on a bounded, hour-stamped
/// copy after the bracket closes. It does not legalise reading the engine there,
/// and the difference is invisible to every behavioural test: a stray
/// `read_u64_stable` in the compute would publish plausible values taken outside
/// the span the witnesses vouch for and outside the hour label the record
/// carries.
///
/// So it is asserted against the source, over the whole post-bracket surface.
/// The primitive list is the one d81's reader actually uses; anything not on it
/// is reachable only through something that is.
///
/// MUTATION RUN: `read_u64_stable(0x1000);` inserted at the top of
/// `d81_compute_rail_hub_rows`. Observed:
///
/// ```text
/// the post-bracket half must not call `read_u64_stable`: it runs after the
/// clock bracket has closed, so anything it reads is outside the span the
/// witnesses vouch for (fn d81_compute_rail_hub_rows()
/// ```
///
/// Restored.
#[test]
fn the_post_bracket_half_performs_no_guarded_read() {
    const PRIMITIVES: [&str; 8] = [
        "read_u64_stable(",
        "read_u32_stable(",
        "read_u8_stable(",
        "read_bytes(",
        "read_into(",
        "read_u64(",
        "read_u32(",
        "d81_read_stable_bytes(",
    ];
    let source = collector_source_lf();
    for name in [
        "fn d81_compute_rail_hub_rows(",
        "fn d81_finish_rail_replay_state(",
        "fn d81_hub_foreign_source_into<",
        // Its only remaining caller in production is the compute, so the probe
        // that answers from a table is on the post-bracket surface too.
        "fn maximum_excluding_controller(",
        // And the table the probe reads: `D81RowTable::get` runs entirely after
        // the bracket closes, so a guarded read reintroduced there would be
        // exactly the defect this test exists for.
        "impl D81RowTable {",
    ] {
        let body = source
            .split_once(name)
            .unwrap_or_else(|| panic!("{name} must exist"))
            .1
            .split_once("\nfn ")
            .unwrap_or_else(|| panic!("the end of {name}"))
            .0;
        // Doc comments and prose name these primitives; only CODE matters, so
        // the comment lines are stripped rather than the search weakened.
        let code = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for primitive in PRIMITIVES {
            assert!(
                !code.contains(primitive),
                "the post-bracket half must not call `{}`: it runs after the \
                 clock bracket has closed, so anything it reads is outside the \
                 span the witnesses vouch for ({name})",
                primitive.trim_end_matches('(')
            );
        }
    }
}

/// The owned copy records the bracket opening it was observed under, the guard
/// is scoped to the OBSERVE half alone, and the copy is handed to the compute by
/// MOVE — so there is nothing left behind to be used in another hour.
///
/// `hour_stamp` is `None` only where the pre-split reader also had no hour to
/// place itself in (a read taken with no bracket at all — the serial and fixture
/// paths). It is deliberately NOT a refusal input: making it one would refuse
/// hours the undivided reader published, which is a completeness regression
/// dressed as a safety check.
#[test]
fn the_d81_deferred_copy_carries_the_hour_it_was_observed_in() {
    let source = collector_source_lf();
    let reader = source
        .split_once("fn read_compact_rail_replay_state(")
        .expect("the d81 reader")
        .1
        .split_once("\nfn ")
        .expect("the end of the d81 reader")
        .0;
    assert!(
        reader.contains("hour_stamp: active_reader_bracket_opening(),"),
        "the stamp must be taken while the bracket is still open; the guard \
         restores the ENCLOSING opening on drop, so a stamp taken afterwards \
         names the wrong bracket or none"
    );
    let deferred = source
        .split_once("struct D81DeferredCompute {")
        .expect("the owned copy")
        .1
        .split_once("\n}")
        .expect("the end of the owned copy")
        .0;
    assert!(
        !deferred.contains("derive"),
        "the copy must not be `Clone`: a clone is a second copy of one hour's \
         observation with nothing left to say which hour it belongs to"
    );
    assert!(
        source.contains("fn d81_finish_rail_replay_state(outcome: D81ObserveOutcome)"),
        "the post-bracket half must take the outcome BY VALUE; a borrow leaves \
         the copy alive in the caller after the hour it belongs to"
    );
    let split = source
        .split_once("fn spawn_gated_split<")
        .expect("the split spawn")
        .1
        .split_once("\nfn ")
        .expect("the end of the split spawn")
        .0;
    // The BRACKET SCOPE, read out of the source rather than matched as one
    // literal line. It used to be pinned as an exact one-line string, which made
    // the assertion hostage to `rustfmt` -- the guard grew a third argument and
    // the same correct code stopped matching. What matters is the SHAPE: the
    // guard and the observation are inside the scope, and the compute is not.
    let observed_scope = split
        .split_once("let observed = {")
        .expect("the observe scope")
        .1
        .split_once("\n        };")
        .expect("the end of the observe scope")
        .0;
    assert!(
        observed_scope.contains("let _clock_guard = ReaderClockBracketGuard::new_reporting(")
            && observed_scope.contains("observe()"),
        "the guard must be scoped to the OBSERVE half alone; a guard that lives \
         to the end of the worker is the shape this lane exists to remove"
    );
    assert!(
        !observed_scope.contains("compute(") && split.contains("compute(observed)"),
        "and the compute must run AFTER that scope, on the same worker -- \
         inside it, the deferred half would be back under the clock bracket \
         and the whole split would be undone"
    );
    let d81_spawn = source
        .split_once("let rail_replay_state_handle = rail_replay_state_live_enabled.then(|| {")
        .expect("the d81 spawn site")
        .1
        .split_once("});")
        .expect("the end of the d81 spawn site")
        .0;
    assert!(
        d81_spawn.contains("spawn_gated_split(")
            && d81_spawn.contains("d81_finish_rail_replay_state,"),
        "and production must actually spawn d81 through the split, or the two \
         halves exist and the bracket still covers both"
    );
}

/// The two appended lane-wall units decompose the split reader's lane span and
/// displace nothing.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: emitting the split units in place of
/// `lw_d{tag}_crit_us` rather than beside it, or naming them something an
/// existing offline formula already reads.
#[test]
fn the_split_units_are_appended_beside_the_existing_lane_wall_units() {
    let names = lw_domain_split_unit_names(81);
    assert_eq!(names, &["lw_d81_inb_us", "lw_d81_post_us"]);
    assert_ne!(names[0], lw_domain_crit_unit_name(81));
    assert_ne!(names[1], lw_domain_crit_unit_name(81));
    // Leaked once per tag, like every other lazily-built unit name beside it.
    assert_eq!(
        lw_domain_split_unit_names(81).as_ptr(),
        names.as_ptr(),
        "a name rebuilt per frame leaks per frame"
    );

    // A frame with no split reader emits neither unit, which is what makes the
    // pair additive rather than a schema change.
    let summary = summarize_lane_spans(&[]);
    assert!(summary.bracket_splits.is_empty());
}

/// Price the emission that left the bracket, at t31's measured shape.
///
/// WHY A BENCH AND NOT AN ASSERTION. `#[ignore]` on purpose, for the reason
/// `d81_foreign_fold_cost_model` gives: this is a measurement, and a wall-clock
/// gate in this tree flakes under lane load. Run it deliberately:
///
/// ```text
/// cargo test -p collector --release d81_bracket_split_cost_model -- --ignored --nocapture
/// ```
///
/// WHAT THE ARM IS, and what it is not. It runs the production emission over an
/// observation at t31's shape and reports its wall. That number is what the
/// pre-split reader billed to its clock bracket for this work and what the split
/// reader now bills to the compute half instead: the saving is a RELOCATION, not
/// a speedup, which is the only kind available when the output has to be
/// byte-identical. Reporting it as a speedup would be a lie the fixture cannot
/// support.
///
/// WHAT IS NOT IN THE ARM: the guarded reads, the witnesses, the hash-table
/// decode. Those are unchanged by the split and stay inside the bracket; the
/// fixture exists to isolate the part that moved.
#[test]
#[ignore]
fn d81_bracket_split_cost_model() {
    use std::time::Instant;

    const TRIALS: usize = 15;
    let mut seed = 0xD8_1E_C0_07u64;
    let frame = T31Frame::build(&mut seed);

    let mut foreign_tables = BTreeMap::new();
    for (index, (_, _, rows)) in frame.tables.iter().enumerate() {
        foreign_tables.insert(
            index,
            D81HashTableSnapshot {
                rows: rows.clone(),
                witnesses: Vec::new(),
            },
        );
    }
    let mut povs = Vec::with_capacity(frame.related.len());
    for pov in 0..frame.related.len() {
        let mut controller_by_province = BTreeMap::new();
        let mut subtype0_routes = Vec::new();
        let mut node_states = BTreeMap::new();
        node_states.insert(0u32, (1u32, 0u8, 5i64, 6i64));
        for &province_id in &frame.queries[pov] {
            let (raw_handle, equivalence_id) = T31Frame::controller(province_id);
            // The emission refuses a zero controller on the direct route, and a
            // refusal would cut the arm short instead of measuring it.
            let raw_handle = raw_handle.max(1);
            let equivalence_id = equivalence_id.max(1);
            controller_by_province.insert(province_id, (raw_handle, equivalence_id));
            subtype0_routes.push((
                D81RoutePair {
                    province_id,
                    node_index: 0,
                },
                false,
            ));
        }
        povs.push(D81HubPovPlan {
            pov_country_id: pov as u32,
            subtype0_routes,
            subtype1_routes: Vec::new(),
            controller_by_province,
            node_states,
            foreign_related: frame.related[pov]
                .iter()
                .map(|&country| {
                    let (raw_handle, equivalence_id, _) = &frame.tables[country];
                    (*raw_handle, *equivalence_id, country)
                })
                .collect(),
        });
    }
    let observation = D81HubObservation {
        foreign_tables,
        povs,
        read_failure: None,
        reread_refused: Some(false),
    };

    // THE FIXTURE IS T31's, OR THIS BENCH MEANS NOTHING.
    let close_to = |realised: u64, measured: u64, name: &str| {
        let error = (realised as f64 - measured as f64).abs() / measured as f64;
        assert!(
            error < 0.01,
            "the fixture must reproduce t31's {name}: realised {realised}, \
             measured {measured} ({:.2}% off)",
            error * 100.0
        );
    };
    close_to(
        observation.povs.len() as u64,
        t31::POV_COUNT,
        "hub_pov_count",
    );
    close_to(
        frame.related_visits(),
        t31::RELATED_VISITS,
        "hub_related_visits",
    );
    close_to(frame.query_calls(), t31::QUERY_CALLS, "hub_query_calls");
    close_to(frame.query_probes(), t31::QUERY_PROBES, "hub_query_probes");

    let phases = D81PhaseAccum::default();
    let mut samples = Vec::with_capacity(TRIALS);
    let mut rows_seen = 0usize;
    let mut checksum = 0i64;
    for _ in 0..TRIALS {
        let started = Instant::now();
        let rows = d81_compute_rail_hub_rows(&observation, &phases)
            .expect("the staged observation emits cleanly");
        samples.push(started.elapsed().as_nanos() as u64);
        rows_seen = rows.len();
        checksum = rows
            .iter()
            .fold(0i64, |acc, row| acc.wrapping_add(row.foreign_max_raw));
    }
    let snapshot = phases.snapshot();
    println!(
        "d81 emission moved off the bracket, t31 shape ({} POVs, {rows_seen} rows, \
         {} queries/trial, {} probes/trial, checksum {checksum}):",
        observation.povs.len(),
        snapshot.hub_query_calls / TRIALS as u64,
        snapshot.hub_query_probes / TRIALS as u64,
    );
    println!("  wall that WAS in-bracket and is now post-bracket:");
    println!("  {}", percentiles_us(samples));

    // ARM A2 -- THE CONTAINER, PRICED AGAINST THE ONE IT REPLACED (lane FG,
    // 2026-08-08).
    //
    // The probe loop is unchanged; what changed is the lookup inside it. This
    // arm walks the SAME per-POV probe sequence the emission above walks, over
    // the same tables held both ways, and reports each container's wall. It is
    // a container measurement and not a claim about the frame: what it prices
    // is `hub_emit_us`, which t37 put at 9.87 ms of d81's 26.05 ms `hub_us`.
    //
    // The checksums are printed and compared: two containers that disagree
    // about an answer are not comparable on speed.
    let map_tables: Vec<BTreeMap<u32, i64>> = frame
        .tables
        .iter()
        .map(|(_, _, rows)| rows.iter().collect())
        .collect();
    let probe_sequence = |lookup: &mut dyn FnMut(usize, u32) -> Option<i64>| -> i64 {
        let mut checksum = 0i64;
        for pov in 0..frame.related.len() {
            for &province_id in &frame.queries[pov] {
                let mut best = 0i64;
                for &country in &frame.related[pov] {
                    if let Some(value) = lookup(country, province_id) {
                        if value > best {
                            best = value;
                        }
                    }
                }
                checksum = checksum.wrapping_add(best);
            }
        }
        checksum
    };
    let mut map_samples = Vec::with_capacity(TRIALS);
    let mut map_checksum = 0i64;
    for _ in 0..TRIALS {
        let started = Instant::now();
        map_checksum = probe_sequence(&mut |country, province_id| {
            map_tables[country].get(&province_id).copied()
        });
        map_samples.push(started.elapsed().as_nanos() as u64);
    }
    let mut table_samples = Vec::with_capacity(TRIALS);
    let mut table_checksum = 0i64;
    for _ in 0..TRIALS {
        let started = Instant::now();
        table_checksum =
            probe_sequence(&mut |country, province_id| frame.tables[country].2.get(province_id));
        table_samples.push(started.elapsed().as_nanos() as u64);
    }
    assert_eq!(
        map_checksum, table_checksum,
        "the two containers must answer identically, or the timings compare \
         different computations"
    );
    println!(
        "  the same probe sequence, per container (checksum {map_checksum}):"
    );
    println!("    BTreeMap (deleted):  {}", percentiles_us(map_samples));
    println!("    D81RowTable (shipped): {}", percentiles_us(table_samples));

    // ARM B -- the second deferred phase. The neighbour-reciprocity fold plus
    // the empty/bounds checks, driven through the REAL post-bracket half with an
    // empty hub observation so that the hub's contribution is ~0 and what is
    // reported is the fold.
    //
    // THE SHAPE IS t26's `d81_phase_provinces_railed` (3,510), because that is
    // the railed count the phase block published; t31's constants above carry
    // `provinces_visited` and not `provinces_railed`. The fold runs over the
    // RAILED provinces, which are the only ones the walk pushes.
    const RAILED: u32 = 3_510;
    const NEIGHBOURS: u32 = 4;
    let mutual: Vec<RailProvinceState> = (0..RAILED)
        .map(|index| {
            // A ring, so every directed edge has its reciprocal and the fold
            // runs to completion instead of refusing part-way.
            let province_id = index + 1;
            let neighbors = (1..=NEIGHBOURS)
                .flat_map(|step| {
                    [
                        (index + step) % RAILED + 1,
                        (index + RAILED - step) % RAILED + 1,
                    ]
                })
                .filter(|&neighbor| neighbor != province_id)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|neighbor| RailNeighborState {
                    province_id: neighbor,
                    edge_level: 1,
                })
                .collect::<Vec<_>>();
            RailProvinceState {
                province_id,
                active_terminal: false,
                anchor_variant: 0,
                anchor_bits: [0; 3],
                neighbors,
            }
        })
        .collect();
    let observed_neighbors = mutual
        .iter()
        .map(|province| province.neighbors.len())
        .sum::<usize>();
    // Built up front, one per trial, because the finish CONSUMES its copy --
    // which is the ownership rule the split rests on, so the bench has to obey
    // it rather than work around it.
    // THE SHAPE IS THE CONVERSION GATE'S WORLD, not t26's 3,510 railed
    // provinces, and the substitution is forced rather than chosen (lane GI,
    // 2026-08-10). The post-bracket half no longer receives rows: it receives the
    // hour's COPY and walks it, so a bench that fed it a synthetic row list would
    // be timing a function signature that does not exist. Staging 3,510 provinces
    // into a `SyntheticMemory` — whose lookup is a linear scan over regions —
    // would spend its whole wall inside the fixture rather than inside the code
    // under test, so the shape is the gate's own and the number below is
    // "the post-bracket half at that shape", walk included.
    let _ = (&mutual, observed_neighbors);
    let mut pending: Vec<D81ObserveOutcome> = (0..TRIALS)
        .map(|_| D81ObserveOutcome::Deferred(Box::new(super::d81_authoritative::staged_deferred())))
        .collect();
    let mut fold_samples = Vec::with_capacity(TRIALS);
    let mut published = 0usize;
    for outcome in pending.drain(..) {
        let started = Instant::now();
        let read = d81_finish_rail_replay_state(outcome);
        fold_samples.push(started.elapsed().as_nanos() as u64);
        assert!(
            read.first_failure.is_none(),
            "the ring is reciprocal, so the fold must publish"
        );
        published = read.provinces.len();
    }
    println!(
        "d81 post-bracket half -- the authoritative walk over the hour's copy, \
         then the reciprocity fold and the closure checks \
         ({published} railed provinces):"
    );
    println!("  {}", percentiles_us(fold_samples));
}

// ---- Witness provenance (lane FQ, 2026-08-08) ------------------------------
//
// THE DEFECT. d81's gap record used to fill both tick witnesses from
// `game_hour_raw` -- the FRAME'S LABEL -- for every reason but the mismatch. On
// an in-hour frame that is right by coincidence; on a label-mismatched one it
// published `SourceUnavailable(target=H, ticks=H/H)` about a read that ran
// wholly in H+1, which contradicts that reason's own documented witness and
// forced `hour_label_audit` to exclude the whole reason as a tautology it could
// not tell from an observation.
//
// THE FIX has three seams and each is exercised below: the bracket publishes
// what it observed, the worker carries that observation out on EVERY outcome,
// and the emit site mints its witnesses from it and states on the wire where
// they came from.

/// What a bracket may claim to have observed, and the four shapes that may not
/// claim anything.
///
/// The refusals are the whole safety property: a witness fabricated from a
/// bracket that tore, panicked, or could not read its clock is worse than no
/// witness, because the wire's grammar cannot tell it from a real one.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: reading the opening observation
/// without consulting `local_bracket_verdict` (the panic, unreadable and
/// straddle cases start reporting an hour); dropping the nonzero filter (the
/// negative case starts claiming it observed hour 0, which is the wire's
/// no-witness value); or deriving the answer from `game_hour_raw` (the in-hour
/// and mismatched cases stop differing).
#[test]
fn a_bracket_reports_only_the_hour_it_can_prove_it_observed() {
    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed = u64::try_from(hour).expect("fixture hour fits u64");
    let bracket = |opening: Option<ClockObservation>,
                   closing: Option<ClockObservation>,
                   worker_panicked: bool| DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening,
        closing,
        worker_panicked,
    };

    // COHERENT, IN THE FRAME'S OWN HOUR. The observation exists and equals the
    // label -- and that equality is now EVIDENCE rather than arithmetic,
    // because the next case proves the two can differ.
    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(hour, 1)),
            Some(clock_obs(hour, 2)),
            false
        )),
        Some(observed)
    );

    // COHERENT, IN ANOTHER HOUR. THE CASE THE WHOLE LANE EXISTS FOR: the read
    // ran entirely in hour+1 and the frame will publish under `hour`.
    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(hour + 1, 1)),
            Some(clock_obs(hour + 1, 2)),
            false
        )),
        Some(observed + 1),
        "the observation must be what the bracket SAW, never the label the \
         frame will publish under"
    );

    // STRADDLED. Two different hours, and every reason d81 mints requires the
    // two ticks to be equal, so there is no admissible pair to publish.
    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(hour, 1)),
            Some(clock_obs(hour + 1, 2)),
            false
        )),
        None
    );
    // UNREADABLE CLOCK, on either end. "Could not observe" is never "observed".
    assert_eq!(
        bracket_observed_hour_raw(&bracket(None, Some(clock_obs(hour, 2)), false)),
        None
    );
    assert_eq!(
        bracket_observed_hour_raw(&bracket(Some(clock_obs(hour, 1)), None, false)),
        None
    );
    // PANICKED WORKER. Nothing the bracket says is trustworthy, even though
    // both reads succeeded and agree.
    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(hour, 1)),
            Some(clock_obs(hour, 2)),
            true
        )),
        None
    );
    // A NEGATIVE READING. `clock_tick_witness` collapses it to 0, which is the
    // wire's no-witness value; publishing it would claim an observation of
    // hour zero.
    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(-5, 1)),
            Some(clock_obs(-5, 2)),
            false
        )),
        None
    );
}

/// The guard publishes the observation and the refusal TOGETHER, off one closed
/// bracket.
///
/// The two are not the same fact and come apart in both directions: an ADMITTED
/// bracket still has an observation its outcome must carry, and a refused one
/// may have none. A guard that wrote only the refusal would leave every
/// successful d81 hour with no witness at all.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: writing the sink from a second pair
/// of clock reads instead of the recorded bracket (the observation stops
/// matching the bracket the tracker holds), or seeding the outcome with anything
/// but `None` (the no-clock fixture below starts reporting an hour it never
/// read).
#[test]
fn a_guard_publishes_its_observation_beside_its_refusal() {
    let hour = super::state_fixture::fixture_game_hour_raw();
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let verdict: Cell<BracketGuardOutcome> = Cell::new(BracketGuardOutcome::default());
    {
        let _guard = ReaderClockBracketGuard::new_reporting(
            &lanes,
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            Some(BracketVerdictSink {
                sink: &verdict,
                frame_hour_raw: hour,
            }),
        );
    }
    let recorded = lanes.take_clock_brackets();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        verdict.get().observed_hour_raw,
        bracket_observed_hour_raw(&recorded[0]),
        "the observation the worker branches on and the bracket reconciliation \
         later judges must be ONE closed bracket, not two"
    );
    assert_eq!(
        verdict.get(),
        BracketGuardOutcome {
            refusal: Some(SplitBracketRefusal::BracketIncoherent),
            observed_hour_raw: None,
        },
        "a fixture-less tracker cannot read a clock, so the guard must refuse \
         AND publish no observation; the two are written together off one \
         closed bracket"
    );
    assert_eq!(
        BracketGuardOutcome::default().observed_hour_raw,
        None,
        "and the SEEDED value must publish none either -- a guard that somehow \
         failed to write must leave the outcome unable to claim an hour, not \
         holding a plausible one"
    );
}

/// The refusal's own witness and the bracket's stamp are ONE observation.
///
/// Lane FJ's `LabelMismatch` refusal carries the observed hour on
/// `first_failure.raw_value`, and lane FQ's stamp carries it on the read. Both
/// are minted from the same closed bracket, and the emit site now takes the
/// witness from the STAMP -- so if the two could disagree, the diagnostic unit a
/// capture reads (`d81_failure_reason_24_hour_label_mismatch`'s raw value) and
/// the witness on the wire would name different hours.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: stamping the refused outcome from
/// anything but the guard that decided the refusal -- for instance from the
/// frame label, or from a bracket re-read after the close.
#[test]
fn the_refusals_own_witness_and_the_brackets_stamp_are_one_observation() {
    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed_hour = hour + 1;
    let mismatched = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(clock_obs(observed_hour, 1)),
        closing: Some(clock_obs(observed_hour, 2)),
        worker_panicked: false,
    };
    let refusal = split_bracket_refusal(&mismatched, hour, HourLabelMismatchPolicy::Refuse)
        .expect("a mismatched bracket with a usable witness refuses");
    let SplitBracketRefusal::LabelMismatch { observed_hour_raw } = refusal else {
        panic!("expected a label mismatch, got {refusal:?}");
    };
    let stamp = bracket_observed_hour_raw(&mismatched);
    assert_eq!(
        Some(observed_hour_raw),
        stamp,
        "the refusal's witness and the bracket's stamp are two spellings of one \
         reading and must not drift"
    );

    // And the placeholder read the refusal builds, once stamped, agrees with
    // itself: the emit site reads the reason from `first_failure` and the hour
    // from the stamp, so those two are what must not contradict.
    let refused =
        d81_stamp_bracket_observation(compact_rail_replay_bracket_refused_read(0, refusal), stamp);
    assert_eq!(
        refused.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::HourLabelMismatch)
    );
    assert_eq!(
        refused.first_failure.map(|failure| failure.raw_value),
        refused.bracket_observed_hour_raw,
    );
}

/// EVERY outcome leaves the worker carrying its bracket's observation.
///
/// Three shapes reach the join -- the computed one, the observe half's own
/// terminal answer kept across a label mismatch, and the placeholder for a
/// refused bracket -- and an unstamped one is a gap record whose witnesses fall
/// back to nothing. Each is built here through the SAME handler
/// `spawn_gated_split` calls, then through the same stamp, because the split's
/// own branch cannot be driven without a live game process.
///
/// The stamp must OVERWRITE: every constructor seeds the field absent (the
/// observe half's answers are built before the guard closes and therefore
/// before the observation exists), so a stamp that preserved a seed would drop
/// the observation on exactly the paths that need it most.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: applying the stamp on one branch of
/// `spawn_gated_split` instead of to the branch result; seeding a constructor
/// with a value of its own; or making the stamp merge instead of overwrite.
#[test]
fn every_d81_outcome_leaves_the_worker_carrying_its_brackets_observation() {
    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed = u64::try_from(hour + 1).expect("fixture hour fits u64");
    let mismatch = SplitBracketRefusal::LabelMismatch {
        observed_hour_raw: observed,
    };

    // 1. THE OBSERVE HALF'S OWN TERMINAL ANSWER, kept across a mismatch.
    let already_failed = CompactRailReplayRead {
        plan: crate::attach_log::d81_copy_bracket::D81PlanCounters::default(),
        equality: crate::attach_log::d81_copy_bracket::D81EqualityCounters::default(),
        provinces: Vec::new(),
        hub_simulation: None,
        observed_neighbors: 0,
        read_time_us: 4_321,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        first_failure: Some(D81FirstFailure::new(
            D81FailureStage::HubObject,
            D81FailureReason::EmptySnapshot,
        )),
        map_reciprocity_record_is_province: 5,
        map_reciprocity_key_is_province_id: 6,
        phase_times: D81PhaseTimes::default(),
        bracket_observed_hour_raw: None,
    };
    assert_eq!(
        already_failed.bracket_observed_hour_raw, None,
        "the observe half builds its answer before the bracket closes, so it \
         cannot know the observation and must not pretend to"
    );
    let kept = d81_stamp_bracket_observation(
        d81_bracket_refused_outcome(
            mismatch,
            Some(D81ObserveOutcome::Complete(already_failed)),
            0,
        ),
        Some(observed),
    );
    assert_eq!(kept.bracket_observed_hour_raw, Some(observed));
    assert_eq!(
        kept.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::EmptySnapshot),
        "and the stamp must not disturb the outcome it stamps"
    );

    // 2. THE PLACEHOLDER for a refused bracket, on both refusal shapes.
    for refusal in [mismatch, SplitBracketRefusal::BracketIncoherent] {
        let placeholder = d81_stamp_bracket_observation(
            d81_bracket_refused_outcome(refusal, None, 0),
            Some(observed),
        );
        assert_eq!(
            placeholder.bracket_observed_hour_raw,
            Some(observed),
            "{refusal:?}"
        );
    }

    // 3. THE COMPUTED OUTCOME. `d81_finish_rail_replay_state` returns a
    // `Complete` observation untouched, so the stamp is the only thing that can
    // fill it -- and it must, since this is the ordinary in-hour path that
    // supplies every honest `SourceUnavailable` witness in a capture.
    let computed = d81_stamp_bracket_observation(
        d81_finish_rail_replay_state(D81ObserveOutcome::Complete(CompactRailReplayRead {
        plan: crate::attach_log::d81_copy_bracket::D81PlanCounters::default(),
        equality: crate::attach_log::d81_copy_bracket::D81EqualityCounters::default(),
            provinces: Vec::new(),
            hub_simulation: None,
            observed_neighbors: 0,
            read_time_us: 7,
            status_flags: 0,
            first_failure: None,
            map_reciprocity_record_is_province: 0,
            map_reciprocity_key_is_province_id: 0,
            phase_times: D81PhaseTimes::default(),
            bracket_observed_hour_raw: None,
        })),
        Some(observed),
    );
    assert_eq!(computed.bracket_observed_hour_raw, Some(observed));

    // 4. AND AN ABSENT OBSERVATION STAYS ABSENT. A stamp that filled in a
    // default here would be the fabricated witness this lane removes.
    let unobserved = d81_stamp_bracket_observation(
        d81_bracket_refused_outcome(SplitBracketRefusal::BracketIncoherent, None, 0),
        None,
    );
    assert_eq!(unobserved.bracket_observed_hour_raw, None);

    // The worker-panic substitute is built at `join()`, outside the bracket, so
    // it never reaches the stamp at all and must carry no observation.
    assert_eq!(
        compact_rail_replay_worker_panic_read(0).bracket_observed_hour_raw,
        None,
        "a read substituted after the join holds no bracket and may not borrow \
         one"
    );
}

/// THE WITNESSES d81 PUBLISHES COME FROM ITS BRACKET, AND NEVER FROM THE LABEL.
///
/// Swept over every wire reason under three observations -- an hour equal to the
/// label, an hour that is NOT (the mismatched-bracket case), and none -- because
/// the defect was invisible in exactly the first of the three.
///
/// The check is made against `game_hour_raw` explicitly: for every reason that
/// carries a witness at all, a bracket that observed `label + 1` must publish
/// `label + 1`. Before this lane every one of them published `label`.
///
/// Each result is then ENCODED, so the payload grammar -- including the rule
/// that a provenance may not contradict its witnesses -- is what accepts it,
/// rather than this test's own opinion of the shape.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: restoring `_ => (tick, tick)` at the
/// emit site or passing `game_hour_raw` into the rule (the mismatched column
/// collapses onto the label); stamping `Unstated` instead of `Unobserved` on the
/// no-observation path (the provenance assertion fails); or claiming
/// `BracketObserved` on a zero witness (the encode fails).
#[test]
fn the_witnesses_a_d81_gap_publishes_come_from_its_bracket() {
    use compact_contract::domain_gap::{
        DomainGapPart, DomainGapReason, DomainGapRecordV1, DomainGapWitnessProvenance,
    };

    let label = super::state_fixture::fixture_game_hour_raw();
    let label_hour = u64::try_from(label).expect("fixture hour fits u64");
    let elsewhere = label_hour + 1;

    for reason in DomainGapReason::ALL {
        // The two reasons the wire requires zeros for state the absence
        // regardless of what the bracket saw; every other reason carries the
        // observation.
        let zero_witness_reason = matches!(
            reason,
            DomainGapReason::ReaderSilent | DomainGapReason::ClockUnreadable
        );
        // `HourLabelMismatch` takes its witness from the mismatch channel, and
        // its grammar forbids the label, so its "in-hour" column is not a
        // reachable state and is exercised only at `elsewhere`.
        let mismatch_reason = reason == DomainGapReason::HourLabelMismatch;

        for observation in [Some(label_hour), Some(elsewhere), None] {
            if mismatch_reason && observation != Some(elsewhere) {
                continue;
            }
            let mismatch_witness = mismatch_reason.then_some(elsewhere);
            let (tick_before, tick_after, provenance) =
                d81_gap_witnesses(reason, mismatch_witness, observation);
            assert_eq!(tick_before, tick_after, "{reason:?} {observation:?}");

            let expected = if zero_witness_reason {
                None
            } else if mismatch_reason {
                Some(elsewhere)
            } else {
                observation
            };
            match expected {
                Some(hour) => {
                    assert_eq!(tick_before, hour, "{reason:?} {observation:?}");
                    assert_eq!(
                        provenance,
                        DomainGapWitnessProvenance::BracketObserved,
                        "{reason:?} {observation:?}"
                    );
                }
                None => {
                    assert_eq!(tick_before, 0, "{reason:?} {observation:?}");
                    assert_eq!(
                        provenance,
                        DomainGapWitnessProvenance::Unobserved,
                        "{reason:?} {observation:?}: absence must be a POSITIVE \
                         claim, not the `Unstated` an unconverted producer writes"
                    );
                }
            }
            // THE CLAIM THIS TEST EXISTS FOR.
            if observation == Some(elsewhere) && !zero_witness_reason {
                assert_ne!(
                    tick_before, label_hour,
                    "{reason:?}: a read that ran outside the frame's hour must \
                     not publish the frame's label as its witness"
                );
            }

            // Only the single-hour reasons can be encoded at this shape;
            // `Overrun`/`Backpressure` carry a coalesced range and a queue
            // witness this emit site never builds, and `Torn` requires two
            // DIFFERENT ticks. d81 mints none of the three.
            if matches!(
                reason,
                DomainGapReason::Overrun | DomainGapReason::Backpressure | DomainGapReason::Torn
            ) {
                continue;
            }
            let encoded = DomainGapRecordV1 {
                part: DomainGapPart::State,
                reason,
                target_hour_raw: label,
                range_start_hour_raw: label,
                range_end_hour_raw: label,
                tick_before,
                tick_after,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: 0,
            }
            .encode_with_witness_provenance(provenance)
            .unwrap_or_else(|error| {
                panic!("{reason:?} {observation:?} must be encodable: {error}")
            });
            assert_eq!(
                DomainGapRecordV1::decode_witness_provenance(&encoded).unwrap(),
                provenance,
                "{reason:?} {observation:?}"
            );
        }
    }
}

/// The emit site mints its witnesses through the rule above, and holds no second
/// source for them.
///
/// A source pin, because `build_compact_extraction_gate_records` needs an
/// injected `hoi4.exe`. It is narrow on purpose: the BEHAVIOUR is
/// `the_witnesses_a_d81_gap_publishes_come_from_its_bracket`, and all this adds
/// is that the production block calls it with the bracket's stamp rather than
/// keeping a copy of the old label arithmetic.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: reintroducing a
/// `u64::try_from(game_hour_raw)` witness in the gap branch, or encoding the
/// payload without stating its provenance.
#[test]
fn the_d81_emit_site_has_no_witness_source_but_its_bracket() {
    let source = collector_source_lf();
    let gap_branch = source
        .split_once("if let Some(mut read) = rail_replay_state_read {")
        .expect("the d81 emit block")
        .1
        .split_once("use compact_contract::domain_gap::{")
        .expect("the gap branch")
        .1
        .split_once("let payload = DomainGapRecordV1 {")
        .expect("the gap payload is built")
        .0;
    assert!(
        gap_branch.contains("let d81_bracket_observation = read.bracket_observed_hour_raw;")
            && gap_branch.contains(
                "d81_gap_witnesses(reason, d81_mismatch_witness, d81_bracket_observation)"
            ),
        "the gap branch must take its witnesses from the bracket's stamp, \
         through the one rule that decides them"
    );
    assert_eq!(
        gap_branch.matches("u64::try_from(game_hour_raw)").count(),
        1,
        "the frame's label may appear in the gap branch exactly once -- in the \
         mismatch witness's `!=` check -- and never as a witness VALUE; the \
         defect this lane closes was `let tick = u64::try_from(game_hour_raw)` \
         feeding every reason's tick pair"
    );
}
