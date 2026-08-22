use super::d81_frame_cost::{d81_hub_foreign_source_for, percentiles_us, t31, T31Frame};
use super::*;

fn ek_observation() -> D81HubObservation {
    ek_observation_with_overlap(false)
}

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

#[test]
fn the_deferred_hub_emission_is_byte_identical_to_the_in_bracket_one() {
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

#[test]
fn a_read_refusal_is_published_after_the_povs_that_completed_before_it() {
    let phases = D81PhaseAccum::default();
    let refusal = D81FirstFailure::new(D81FailureStage::HubObject, D81FailureReason::Unreadable)
        .subject(4_242);

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
        diagnostics_provenance: D81DiagnosticsProvenance::Measured,
    };
    let finished = d81_finish_rail_replay_state(
        D81ObserveOutcome::Complete(read),
        &mut crate::attach_log::d81_geometry_gate::D81GeometryGate::default(),
    );
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

#[test]
fn a_rejected_bracket_never_reaches_the_compute() {
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
        split.contains("rejected(refusal, RefusedObservation::MeasurementOnly(measured))")
            && split.contains("rejected(refusal, RefusedObservation::Whole(observed))"),
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

    let refused = compact_rail_replay_bracket_refused_read(
        0,
        SplitBracketRefusal::BracketIncoherent,
        D81PreservedMeasurement::default(),
    );
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
    assert_eq!(
        refused.diagnostics_provenance,
        D81DiagnosticsProvenance::PreservedAfterRefusal,
        "and a refused record's block is published as PRESERVED, so a reader \
         cannot mistake this empty one for a measurement of a cheap frame",
    );
}

#[test]
fn a_label_mismatched_bracket_skips_its_compute_under_its_own_reason() {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed_hour = hour + 1;
    let observed = u64::try_from(observed_hour).expect("fixture hour fits u64");

    let mut refused = compact_rail_replay_bracket_refused_read(
        0,
        SplitBracketRefusal::LabelMismatch {
            observed_hour_raw: observed,
        },
        D81PreservedMeasurement::default(),
    );
    assert!(
        encode_compact_rail_replay_production(&mut refused, 0).is_none(),
        "the bracket-refused read has no hub snapshot, so d81's record for that \
         frame is a DOMAIN_GAP"
    );

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
        source
            .contains("Some(D81FailureReason::HourLabelMismatch) => match d81_mismatch_witness {")
            && source.contains("Some(_) => DomainGapReason::HourLabelMismatch,"),
        "and the new reason must reach the wire as the mismatch, gated on its \
         witness being present"
    );
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

#[test]
fn the_skipped_frame_publishes_no_source_unavailable_claim() {
    let mismatch = compact_rail_replay_bracket_refused_read(
        0,
        SplitBracketRefusal::LabelMismatch {
            observed_hour_raw: 7,
        },
        D81PreservedMeasurement::default(),
    );
    assert_eq!(
        mismatch.status_flags, COMPACT_STATUS_PARTIAL_DATA,
        "the engine was read perfectly; only its attribution failed"
    );
    let incoherent = compact_rail_replay_bracket_refused_read(
        0,
        SplitBracketRefusal::BracketIncoherent,
        D81PreservedMeasurement::default(),
    );
    assert_eq!(
        incoherent.status_flags,
        COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        "a bracket that could not vouch for its observation is unchanged"
    );
    assert!(mismatch.hub_simulation.is_none() && incoherent.hub_simulation.is_none());
}

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

#[test]
fn a_read_failure_still_outranks_a_label_mismatch() {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed_hour = hour + 1;
    let key = (
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        DomainGapPart::State.code(),
    );
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

#[test]
fn an_unusable_observed_hour_refuses_without_killing_the_frame() {
    use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let hour = super::state_fixture::fixture_game_hour_raw();
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
    let mut offset = 0usize;
    let mut seen = Vec::new();
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..]).unwrap();
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP {
            let gap = DomainGapRecordV1::decode(
                &records[offset + compact_contract::COMPACT_RECORD_HEADER_LEN
                    ..offset + header.record_len()],
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

#[test]
fn a_read_failure_from_the_observe_half_survives_a_label_mismatch_skip() {
    let observed_hour_raw = 42u64;
    let mismatch = SplitBracketRefusal::LabelMismatch { observed_hour_raw };

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
        diagnostics_provenance: D81DiagnosticsProvenance::Measured,
    };
    let kept = d81_bracket_refused_outcome(
        mismatch,
        RefusedObservation::Whole(D81ObserveOutcome::Complete(already_failed)),
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
    assert_eq!(
        kept.diagnostics_provenance,
        D81DiagnosticsProvenance::Measured,
    );

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
    let _ = (&ring, observed_neighbors);
    let deferred_copy =
        || D81ObserveOutcome::Deferred(Box::new(super::d81_authoritative::staged_deferred()));
    let computed = d81_finish_rail_replay_state(
        deferred_copy(),
        &mut crate::attach_log::d81_geometry_gate::D81GeometryGate::default(),
    );
    assert!(
        computed.hub_simulation.is_some() && computed.first_failure.is_none(),
        "the fixture must be a copy the compute half would publish, or 'the \
         compute did not run' is indistinguishable from 'there was nothing to \
         run'"
    );

    let deferred =
        d81_bracket_refused_outcome(mismatch, RefusedObservation::Whole(deferred_copy()), 0);
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

    let incoherent = d81_bracket_refused_outcome(
        SplitBracketRefusal::BracketIncoherent,
        RefusedObservation::MeasurementOnly(D81PreservedMeasurement::default()),
        0,
    );
    assert_eq!(
        incoherent.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::Torn)
    );

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
            && split.contains("let measured = preserve(&observed);")
            && split.contains("drop(observed);")
            && split.contains("rejected(refusal, RefusedObservation::MeasurementOnly(measured))"),
        "a bracket that cannot vouch for its observation still has its copy \
         dropped unread, and what leaves the arm is the MEASUREMENT taken \
         through a shared reference -- never the observation"
    );
    assert!(
        split.contains("Some(refusal @ SplitBracketRefusal::LabelMismatch { .. }) => {")
            && split.contains("rejected(refusal, RefusedObservation::Whole(observed))"),
        "and a label-mismatched bracket hands its observation to the handler, \
         which is the only way a terminal read failure can survive the skip"
    );
}

fn measured_plan_counters() -> crate::attach_log::d81_copy_bracket::D81PlanCounters {
    crate::attach_log::d81_copy_bracket::D81PlanCounters {
        ran: true,
        copies_issued: 84_256,
        intervals_closed: 21_063,
        plan_us: 4_012,
        custody_held_bytes: 7_654_321,
        ..crate::attach_log::d81_copy_bracket::D81PlanCounters::default()
    }
}

fn measured_phase_times() -> D81PhaseTimes {
    D81PhaseTimes {
        topology_us: 9_001,
        roots_us: 9_002,
        plan_us: 9_003,
        snapshot_bytes: 9_004,
        provinces_railed: 9_005,
        ..D81PhaseTimes::default()
    }
}

#[test]
fn a_refused_frames_measurements_survive_both_discard_sites() {
    let hour = super::state_fixture::fixture_game_hour_raw();
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let observed_read = CompactRailReplayRead {
        plan: measured_plan_counters(),
        equality: crate::attach_log::d81_copy_bracket::D81EqualityCounters::default(),
        provinces: Vec::new(),
        hub_simulation: Some(Vec::new()),
        observed_neighbors: 0,
        read_time_us: 97_700,
        status_flags: 0,
        first_failure: None,
        map_reciprocity_record_is_province: 11,
        map_reciprocity_key_is_province_id: 12,
        phase_times: measured_phase_times(),
        bracket_observed_hour_raw: None,
        diagnostics_provenance: D81DiagnosticsProvenance::Measured,
    };
    let torn = std::thread::scope(|scope| {
        crate::attach_log::spawn_gated_split(
            scope,
            None,
            &lanes,
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            hour,
            move |_bracket_opening| D81ObserveOutcome::Complete(observed_read),
            |outcome| {
                d81_finish_rail_replay_state(
                    outcome,
                    &mut crate::attach_log::d81_geometry_gate::D81GeometryGate::default(),
                )
            },
            d81_preserve_measurement,
            |refusal, observed| d81_bracket_refused_outcome(refusal, observed, 0),
            d81_stamp_bracket_observation,
        )
        .join()
        .expect("the worker must not panic")
    });
    assert_eq!(
        torn.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::Torn),
        "a fixture-less tracker has no clock, so this arm must be the \
         incoherent one",
    );
    assert!(
        torn.hub_simulation.is_none() && torn.provinces.is_empty(),
        "a torn bracket publishes no rows and no hub snapshot -- preservation \
         carries COUNTERS across the discard and nothing else",
    );
    assert_eq!(
        torn.status_flags,
        COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
    );
    assert_eq!(torn.plan, measured_plan_counters());
    assert_eq!(torn.phase_times, measured_phase_times());
    assert_eq!(torn.read_time_us, 97_700);
    assert_eq!(torn.map_reciprocity_record_is_province, 11);
    assert_eq!(torn.map_reciprocity_key_is_province_id, 12);
    assert_eq!(
        torn.diagnostics_provenance,
        D81DiagnosticsProvenance::PreservedAfterRefusal,
        "and the wire says they were PRESERVED, not measured by a frame that \
         ran to a published outcome",
    );

    let mut deferred = super::d81_authoritative::staged_deferred();
    deferred.plan = measured_plan_counters();
    deferred.snapshot_bytes = 9_004;
    deferred.phases.topology_ns.set(9_001_000);
    deferred.phases.roots_ns.set(9_002_000);
    deferred.phases.plan_ns.set(9_003_000);
    deferred.phases.provinces_railed.set(9_005);
    let mismatch = SplitBracketRefusal::LabelMismatch {
        observed_hour_raw: 42,
    };
    let handled = d81_bracket_refused_outcome(
        mismatch,
        RefusedObservation::Whole(D81ObserveOutcome::Deferred(Box::new(deferred))),
        0,
    );
    assert!(
        handled.hub_simulation.is_none() && handled.provinces.is_empty(),
        "the deferred copy must still be dropped unread",
    );
    assert_eq!(
        handled.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::HourLabelMismatch),
    );
    assert_eq!(handled.status_flags, COMPACT_STATUS_PARTIAL_DATA);
    assert_eq!(handled.plan, measured_plan_counters());
    assert_eq!(handled.phase_times, measured_phase_times());
    assert!(
        handled.read_time_us > 0,
        "the frame's own wall must be published rather than left at zero, or \
         `read_time_us - sum(phases)` reads negative on every refused frame",
    );
    assert_eq!(
        handled.diagnostics_provenance,
        D81DiagnosticsProvenance::PreservedAfterRefusal,
    );

    let panicked = compact_rail_replay_worker_panic_read(0);
    assert_eq!(
        panicked.diagnostics_provenance,
        D81DiagnosticsProvenance::Substituted,
    );
    assert_eq!(panicked.phase_times, D81PhaseTimes::default());
}

#[test]
fn each_d81_record_publishes_exactly_one_provenance_word() {
    let base = compact_rail_replay_bracket_refused_read(
        0,
        SplitBracketRefusal::BracketIncoherent,
        D81PreservedMeasurement::default(),
    );
    let mut names = Vec::new();
    for provenance in [
        D81DiagnosticsProvenance::Measured,
        D81DiagnosticsProvenance::PreservedAfterRefusal,
        D81DiagnosticsProvenance::Substituted,
    ] {
        let read = CompactRailReplayRead {
            diagnostics_provenance: provenance,
            ..compact_rail_replay_bracket_refused_read(
                0,
                SplitBracketRefusal::BracketIncoherent,
                D81PreservedMeasurement::default(),
            )
        };
        let units = d81_diagnostics_provenance_units(&read);
        assert_eq!(units.len(), 1, "{provenance:?}");
        assert_eq!(units[0].value, 1, "{provenance:?}");
        assert!(
            units[0].name.starts_with("d81_diagnostics_"),
            "the manifest aggregates by name, so the namespace is load-bearing: \
             {}",
            units[0].name,
        );
        assert!(
            !units[0].name.ends_with("_sample"),
            "a `_sample` suffix would route this to LastValue, and a count that \
             does not sum cannot be divided into `d81_plan_frames`: {}",
            units[0].name,
        );
        names.push(units[0].name);
    }
    let distinct: std::collections::BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        names.len(),
        "two provenances sharing a name would make a preserved block \
         indistinguishable from a measured one on the wire: {names:?}",
    );
    assert_eq!(
        d81_diagnostics_provenance_units(&base)[0].name,
        "d81_diagnostics_preserved_frames",
    );
}

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

#[test]
fn the_snapshot_budget_refuses_under_its_own_assigned_reason() {
    let mut budget = D81SnapshotBudget::new(100);
    assert!(budget.charge(60).is_ok());
    assert!(
        budget.charge(40).is_ok(),
        "exactly at the limit is admitted"
    );
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
        "fn maximum_excluding_controller(",
        "impl D81RowTable {",
    ] {
        let body = source
            .split_once(name)
            .unwrap_or_else(|| panic!("{name} must exist"))
            .1
            .split_once("\nfn ")
            .unwrap_or_else(|| panic!("the end of {name}"))
            .0;
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
        source.contains("fn d81_finish_rail_replay_state(
    outcome: D81ObserveOutcome,"),
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
    let observed_scope = split
        .split_once("let observed = {")
        .expect("the observe scope")
        .1
        .split_once("\n        };")
        .expect("the end of the observe scope")
        .0;
    assert!(
        observed_scope.contains("let clock_guard = ReaderClockBracketGuard::new_reporting(")
            && observed_scope.contains("observe(bracket_opening)"),
        "the guard must be scoped to the OBSERVE half alone; a guard that lives \
         to the end of the worker is the shape this lane exists to remove. \
         (Since lane HT the guard binding is named — its opening observation \
         is handed to the observe closure — and the observation is passed as \
         `observe(bracket_opening)`.)"
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
            && d81_spawn.contains("d81_finish_rail_replay_state(observation, d81_geometry_gate)"),
        "and production must actually spawn d81 through the split, or the two \
         halves exist and the bracket still covers both"
    );
}

#[test]
fn the_split_units_are_appended_beside_the_existing_lane_wall_units() {
    let names = lw_domain_split_unit_names(81);
    assert_eq!(names, &["lw_d81_inb_us", "lw_d81_post_us"]);
    assert_ne!(names[0], lw_domain_crit_unit_name(81));
    assert_ne!(names[1], lw_domain_crit_unit_name(81));
    assert_eq!(
        lw_domain_split_unit_names(81).as_ptr(),
        names.as_ptr(),
        "a name rebuilt per frame leaks per frame"
    );

    let summary = summarize_lane_spans(&[]);
    assert!(summary.bracket_splits.is_empty());
}

/// cargo test -p collector --release d81_bracket_split_cost_model -- --ignored --nocapture
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
    println!("  the same probe sequence, per container (checksum {map_checksum}):");
    println!("    BTreeMap (deleted):  {}", percentiles_us(map_samples));
    println!(
        "    D81RowTable (shipped): {}",
        percentiles_us(table_samples)
    );

    // the empty/bounds checks, driven through the REAL post-bracket half with an
    const RAILED: u32 = 3_510;
    const NEIGHBOURS: u32 = 4;
    let mutual: Vec<RailProvinceState> = (0..RAILED)
        .map(|index| {
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
    // which is the ownership rule the split rests on, so the bench has to obey
    let _ = (&mutual, observed_neighbors);
    let mut pending: Vec<D81ObserveOutcome> = (0..TRIALS)
        .map(|_| D81ObserveOutcome::Deferred(Box::new(super::d81_authoritative::staged_deferred())))
        .collect();
    let mut fold_samples = Vec::with_capacity(TRIALS);
    let mut published = 0usize;
    for outcome in pending.drain(..) {
        let started = Instant::now();
        let read = d81_finish_rail_replay_state(
            outcome,
            &mut crate::attach_log::d81_geometry_gate::D81GeometryGate::default(),
        );
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

    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(hour, 1)),
            Some(clock_obs(hour, 2)),
            false
        )),
        Some(observed)
    );

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

    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(hour, 1)),
            Some(clock_obs(hour + 1, 2)),
            false
        )),
        None
    );
    assert_eq!(
        bracket_observed_hour_raw(&bracket(None, Some(clock_obs(hour, 2)), false)),
        None
    );
    assert_eq!(
        bracket_observed_hour_raw(&bracket(Some(clock_obs(hour, 1)), None, false)),
        None
    );
    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(hour, 1)),
            Some(clock_obs(hour, 2)),
            true
        )),
        None
    );
    assert_eq!(
        bracket_observed_hour_raw(&bracket(
            Some(clock_obs(-5, 1)),
            Some(clock_obs(-5, 2)),
            false
        )),
        None
    );
}

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

    let refused = d81_stamp_bracket_observation(
        compact_rail_replay_bracket_refused_read(0, refusal, D81PreservedMeasurement::default()),
        stamp,
    );
    assert_eq!(
        refused.first_failure.map(|failure| failure.reason),
        Some(D81FailureReason::HourLabelMismatch)
    );
    assert_eq!(
        refused.first_failure.map(|failure| failure.raw_value),
        refused.bracket_observed_hour_raw,
    );
}

#[test]
fn every_d81_outcome_leaves_the_worker_carrying_its_brackets_observation() {
    let hour = super::state_fixture::fixture_game_hour_raw();
    let observed = u64::try_from(hour + 1).expect("fixture hour fits u64");
    let mismatch = SplitBracketRefusal::LabelMismatch {
        observed_hour_raw: observed,
    };

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
        diagnostics_provenance: D81DiagnosticsProvenance::Measured,
    };
    assert_eq!(
        already_failed.bracket_observed_hour_raw, None,
        "the observe half builds its answer before the bracket closes, so it \
         cannot know the observation and must not pretend to"
    );
    let kept = d81_stamp_bracket_observation(
        d81_bracket_refused_outcome(
            mismatch,
            RefusedObservation::Whole(D81ObserveOutcome::Complete(already_failed)),
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

    for refusal in [mismatch, SplitBracketRefusal::BracketIncoherent] {
        let placeholder = d81_stamp_bracket_observation(
            d81_bracket_refused_outcome(
                refusal,
                RefusedObservation::MeasurementOnly(D81PreservedMeasurement::default()),
                0,
            ),
            Some(observed),
        );
        assert_eq!(
            placeholder.bracket_observed_hour_raw,
            Some(observed),
            "{refusal:?}"
        );
    }

    let computed = d81_stamp_bracket_observation(
        d81_finish_rail_replay_state(
            D81ObserveOutcome::Complete(CompactRailReplayRead {
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
            diagnostics_provenance: D81DiagnosticsProvenance::Measured,
        }),
            &mut crate::attach_log::d81_geometry_gate::D81GeometryGate::default(),
        ),
        Some(observed),
    );
    assert_eq!(computed.bracket_observed_hour_raw, Some(observed));

    let unobserved = d81_stamp_bracket_observation(
        d81_bracket_refused_outcome(
            SplitBracketRefusal::BracketIncoherent,
            RefusedObservation::MeasurementOnly(D81PreservedMeasurement::default()),
            0,
        ),
        None,
    );
    assert_eq!(unobserved.bracket_observed_hour_raw, None);

    assert_eq!(
        compact_rail_replay_worker_panic_read(0).bracket_observed_hour_raw,
        None,
        "a read substituted after the join holds no bracket and may not borrow \
         one"
    );
}

#[test]
fn the_witnesses_a_d81_gap_publishes_come_from_its_bracket() {
    use compact_contract::domain_gap::{
        DomainGapPart, DomainGapReason, DomainGapRecordV1, DomainGapWitnessProvenance,
    };

    let label = super::state_fixture::fixture_game_hour_raw();
    let label_hour = u64::try_from(label).expect("fixture hour fits u64");
    let elsewhere = label_hour + 1;

    for reason in DomainGapReason::ALL {
        let zero_witness_reason = matches!(
            reason,
            DomainGapReason::ReaderSilent | DomainGapReason::ClockUnreadable
        );
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
            if observation == Some(elsewhere) && !zero_witness_reason {
                assert_ne!(
                    tick_before, label_hour,
                    "{reason:?}: a read that ran outside the frame's hour must \
                     not publish the frame's label as its witness"
                );
            }

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
