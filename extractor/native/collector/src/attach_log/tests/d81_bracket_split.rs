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
        provinces: Vec::new(),
        hub_simulation: None,
        observed_neighbors: 0,
        read_time_us: 1_234,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        first_failure: Some(refusal),
        map_reciprocity_record_is_province: 3,
        map_reciprocity_key_is_province_id: 4,
        phase_times: D81PhaseTimes::default(),
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
    // THE LABEL MISMATCH IS NOT LOCAL, and the split still does not skip it.
    // Lane EK gave the reason as "no worker has the frame's hour when its guard
    // closes"; that is false (see `LocalBracketVerdict`), and the real reason is
    // pinned by `a_label_mismatched_bracket_cannot_yet_skip_its_compute`.
    assert_eq!(
        local_bracket_verdict(&coherent),
        LocalBracketVerdict::CoherentHour,
        "a bracket that observed the wrong hour is still LOCALLY coherent"
    );
    assert_eq!(
        bracket_hour_verdict(&coherent, next_hour),
        BracketHourVerdict::LabelMismatch
    );

    // Now the production seam, through a real guard on a real tracker. The
    // fixture-less tracker cannot read a clock, so the guard publishes
    // `ClockUnreadable` and the compute must be skipped.
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let verdict = Cell::new(LocalBracketVerdict::CoherentHour);
    let observed_ran = Cell::new(false);
    {
        let _guard = ReaderClockBracketGuard::new_reporting(
            &lanes,
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            Some(&verdict),
        );
        observed_ran.set(true);
    }
    assert!(observed_ran.get(), "the observe half always runs");
    assert_eq!(
        verdict.get(),
        LocalBracketVerdict::ClockUnreadable,
        "a guard with no readable clock must publish its refusal, or the split \
         has nothing to branch on"
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
        split.contains("let out = if verdict.get() == LocalBracketVerdict::CoherentHour {")
            && split.contains("drop(observed);")
            && split.contains("rejected()"),
        "a bracket the guard refused must not reach the deferred compute, and \
         its copy must be dropped unread"
    );
    assert!(
        split.contains(
            "ReaderClockBracketGuard::new_reporting(lanes, tag, Some(&verdict))"
        ),
        "and the guard must be the one that publishes the verdict; a verdict \
         computed anywhere else is a second predicate that can drift"
    );

    // The rejected arm's own outcome refuses the hour rather than publishing an
    // empty one, and it does no work.
    let refused = compact_rail_replay_bracket_refused_read(0);
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

/// WHY THE SPLIT STILL COMPUTES ON A LABEL-MISMATCHED BRACKET, and it is not
/// because the worker cannot tell (lane FG, 2026-08-08).
///
/// t37 turned 211 of d81's 621 frames into `HourLabelMismatch` gaps, and every
/// one of them ran the whole post-bracket compute first — `lw_d81_post_us`
/// averages 6.4 ms a frame. Skipping it is worth having, and lane EK's stated
/// reason for not doing so is false: `game_hour_raw` is fixed by the scheduler
/// request before the fan-out opens and is in scope where the `FrameLaneTracker`
/// is built, so the guard could publish the full verdict.
///
/// THE REAL BLOCKER IS THE GAP PRECEDENCE, and this test is what a naive
/// attempt runs into. The chain, asserted below:
///
///  1. skipping means returning `compact_rail_replay_bracket_refused_read`,
///     which carries no `hub_simulation`, so the encoder produces no payload and
///     d81's record is a DOMAIN_GAP;
///  2. that gap's reason is `SourceUnavailable` — a claim that the engine could
///     not be read, which on a label-mismatched frame is simply false;
///  3. `replace_rejected_domain_records_with_gaps` DROPS a `HourLabelMismatch`
///     rejection when the domain already carries a gap, deliberately, because a
///     read failure is the more specific fact than an attribution failure.
///
/// So the skip would replace `HourLabelMismatch` with a fabricated
/// `SourceUnavailable` on 34% of d81's frames. Closing it needs d81's
/// bracket-refused read to be able to name the mismatch and the hour it
/// observed, so the encoder can mint the same gap reconciliation would have —
/// new wire vocabulary, and an owner decision.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: extending the rejected arm to cover a
/// label-mismatched bracket without also giving that read its own reason. The
/// assertions are the three links; whichever one a future change breaks is the
/// one it has to have fixed.
#[test]
fn a_label_mismatched_bracket_cannot_yet_skip_its_compute() {
    // LINK 1 -- the placeholder read cannot produce a payload, so its record is
    // a gap and not data.
    let mut refused = compact_rail_replay_bracket_refused_read(0);
    assert!(
        encode_compact_rail_replay_production(&mut refused, 0).is_none(),
        "the bracket-refused read has no hub snapshot, so d81's record for that \
         frame is a DOMAIN_GAP"
    );

    // LINK 2 -- and the reason that gap carries. `Torn` at stage `Worker` is
    // what the placeholder names, and the encoder maps it to
    // `SourceUnavailable`.
    assert_eq!(
        refused.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::Torn)
    );
    let source = collector_source_lf();
    assert!(
        source.contains(
            "Some(D81FailureReason::Torn | D81FailureReason::ClosureChanged) => {\n\
             \x20                   DomainGapReason::SourceUnavailable\n\
             \x20               }"
        ),
        "the placeholder's reason must still map to SourceUnavailable, or this \
         test is describing a chain that no longer exists"
    );

    // LINK 3 -- and the precedence that then discards the mismatch. A d81
    // bracket that read the wrong hour IS rejected under `Refuse` ...
    let hour = super::state_fixture::fixture_game_hour_raw();
    let mismatched = DomainClockBracket {
        tag: COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        opening: Some(ClockObservation {
            hour: hour + 1,
            ordinal: 1,
            at_us: 0,
        }),
        closing: Some(ClockObservation {
            hour: hour + 1,
            ordinal: 2,
            at_us: 1,
        }),
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
        .map(|&(reason, _, _)| reason),
        Some(compact_contract::domain_gap::DomainGapReason::HourLabelMismatch),
        "reconciliation does name the mismatch"
    );
    // ... and that naming is exactly what the yield rule removes when the
    // domain's own record is already a gap.
    assert!(
        source.contains(
            "*reason == compact_contract::domain_gap::DomainGapReason::HourLabelMismatch"
        ) && source.contains(
            "&& header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP\n\
             \x20       {\n\
             \x20           rejected.remove(&header.domain_id);"
        ),
        "a HourLabelMismatch rejection yields to an existing gap, which is what \
         makes link 1 fatal"
    );
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
    assert!(
        split.contains(
            "let observed = {\n            let _clock_guard = ReaderClockBracketGuard::new_reporting(lanes, tag, Some(&verdict));\n            observe()\n        };"
        ),
        "the guard must be scoped to the OBSERVE half alone; a guard that lives \
         to the end of the worker is the shape this lane exists to remove"
    );
    assert!(
        split.contains("compute(observed)"),
        "and the compute must run after that scope, on the same worker"
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
    let mut pending: Vec<D81ObserveOutcome> = (0..TRIALS)
        .map(|_| {
            D81ObserveOutcome::Deferred(Box::new(D81DeferredCompute {
                started: Instant::now(),
                status_flags: 0,
                phases: D81PhaseAccum::default(),
                admitted_record_is_province: 0,
                admitted_key_is_province_id: 0,
                provinces: mutual.clone(),
                observed_neighbors,
                hub: D81HubObservation {
                    foreign_tables: BTreeMap::new(),
                    povs: Vec::new(),
                    read_failure: None,
                    reread_refused: Some(false),
                },
                closure_failure: None,
                hour_stamp: None,
                snapshot_bytes: 0,
            }))
        })
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
        "d81 reciprocity fold + closure checks moved off the bracket \
         ({published} railed provinces, {observed_neighbors} directed edges):"
    );
    println!("  {}", percentiles_us(fold_samples));
}
