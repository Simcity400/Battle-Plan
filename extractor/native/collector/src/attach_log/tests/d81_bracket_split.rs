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
                // A LINEAR SCAN, DELIBERATELY (lane RY3). Production binary
                // searches this vector; scanning here keeps the reference an
                // INDEPENDENT check of that search instead of a copy of it. A
                // plan that reached the emission unsorted would still be found
                // by this scan and missed by the search, and the row comparison
                // this function feeds would fail — which is the outcome wanted.
                let &(_, (controller_raw_handle, controller_country_id)) = plan
                    .controller_by_province
                    .iter()
                    .find(|&&(province_id, _)| province_id == route.province_id)
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
                let &(_, (node_kind, node_flag, available_raw, consumed_raw)) = plan
                    .node_states
                    .iter()
                    .find(|&&(node_index, _)| node_index == route.node_index)
                    .ok_or_else(|| {
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
    earlier.povs[0]
        .node_states
        .retain(|&(node_index, _)| node_index != 1);
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
    let (torn, ()) = std::thread::scope(|scope| {
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
            |_read| (),
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

/// THE POST-BRACKET HALF MAY READ ONLY WHAT THE BRACKET COPIED.
///
/// Perf wave 1 changed the SHAPE this law is about and not the law. The bracket
/// used to carry the whole guarded read; it now carries only the bulk copy
/// (`capture_compact_rail_replay_state_snapshot`), and the decode runs after it
/// against the copied regions, under `with_d81_memory_override`. What the law
/// means in that shape is this: a decode read that leaves the copied regions
/// must refuse the WHOLE FRAME, because after the bracket closes nothing is
/// inside the span the clock witnesses vouch for.
///
/// It is not the override that this watches. An installed override already
/// fails such a read rather than falling through to the engine - that is wave
/// 1's, and its own tests own it (Codex round 43). What is left, and what this
/// owns, is what the frame does with that failed read: a read outside the copy
/// must end the frame as `SnapshotRegionMiss` with the short plan evicted and
/// nothing published, rather than being carried as one more absent field into a
/// frame that files itself under the hour anyway.
///
/// This drives it rather than reading it. The plan below copies memory the
/// decode never asks for, so the decode's first read is a miss, and the refusal
/// must be `SnapshotRegionMiss` with the plan dropped - not a frame assembled
/// from whatever the live process happened to hold.
///
/// Wave 1 owns the two halves either side of this one: the view's own bounds
/// (`d81_snapshot_memory::tests`) and the lexical scoping of the decode calls
/// (`d81_snapshot::the_outside_bracket_decoder_is_scoped_to_snapshot_memory`).
/// Nothing owned the WIRING between them, which is where a miss becomes a
/// refusal, and that is this.
///
/// kills: dropping the `view.first_miss()` check, or answering with `decoded`
/// anyway, either of which publishes a frame partly built outside the bracket.
#[test]
fn a_post_bracket_decode_that_leaves_the_copied_regions_refuses_instead_of_answering() {
    use crate::attach_log::d81_snapshot_memory::{
        Domain81Memory, Domain81SnapshotPlan, Domain81TracingMemory,
    };

    /// Sixteen bytes at an address no decode reads. A low page is never a
    /// map-control heap pointer, so nothing the fixture stages can collide.
    struct Elsewhere;
    const ELSEWHERE: u64 = 0x1000;
    impl Domain81Memory for Elsewhere {
        fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
            if address != ELSEWHERE || out.len() != 16 {
                return false;
            }
            out.fill(0x5a);
            true
        }
    }

    page_cache_new_frame();
    let fixture = super::shared_map_topology::staged_map_fixture();
    let mut state = fixture.collector_state();
    state.daily_match = true;
    state.long_string_match = true;
    let cache = super::shared_map_topology::cache_for(
        &state,
        super::state_fixture::fixture_game_hour_raw(),
    );

    let tracing = Domain81TracingMemory::new(&Elsewhere);
    let mut probe = [0u8; 16];
    assert!(
        tracing.read_into(ELSEWHERE, &mut probe),
        "the plan must be built from a read that succeeded, or it holds no region at all"
    );
    let plan = Domain81SnapshotPlan::from_trace(tracing.into_trace()).expect("a bounded plan");
    let tracker = CompactD81SnapshotTracker::default();
    tracker.state().plan = Some(plan);

    let capture = D81SnapshotCapture {
        receipt: crate::attach_log::d81_snapshot_memory::Domain81SnapshotCaptureReceipt {
            region_count: 1,
            byte_count: 16,
            copied_region_count: 1,
            copied_byte_count: 16,
            failure_region_ordinal: None,
        },
        // The shipped cause while the mutator census is open, so this is the
        // arm production actually takes.
        cause: D81SnapshotCaptureCause::MutatorCoverageIncomplete,
        opening: None,
        quiet_epoch: 7,
        plan_refusal: None,
        arm_us: 0,
        arm_trace_reads: 0,
    };
    let read = finish_compact_rail_replay_state_snapshot(
        capture,
        &tracker,
        &state,
        0,
        &cache,
        false,
        &mut crate::attach_log::d81_geometry_gate::D81GeometryGate::default(),
    );

    let failure = read
        .first_failure
        .expect("a decode outside the copied regions must refuse");
    assert_eq!(
        failure.reason,
        D81FailureReason::SnapshotRegionMiss,
        "the post-bracket decode answered from something other than the copy"
    );
    assert_eq!(
        read.plan.bulk_snapshot_region_miss, 1,
        "the miss must be counted where a reader of the wire can see it"
    );
    assert_ne!(failure.read_address, 0);
    assert_ne!(failure.read_width, 0);
    assert_eq!(failure.read_success_mask, 0);
    let units = d81_failure_diagnostic_units(Some(failure));
    for (name, expected) in [
        ("d81_failure_read_address_sample", failure.read_address),
        ("d81_failure_read_width_sample", u64::from(failure.read_width)),
    ] {
        assert_eq!(units.iter().find(|unit| unit.name == name).unwrap().value, expected);
    }
    assert!(
        read.hub_simulation.is_none() && read.provinces.is_empty(),
        "a refused snapshot must publish no rows at all"
    );
    assert!(
        tracker.state().plan.is_none(),
        "a plan that missed must be dropped, or the next hour decodes against the same short copy"
    );
    assert_eq!(
        read.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
        "the refusal must reach the wire as source-unavailable"
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
        // Wave 1's post-bracket members. The decode they run reads through the
        // snapshot override, which is the whole point of the copy; what these
        // four must not do is reach the engine THEMSELVES, outside the override
        // and outside the bracket.
        "fn finish_compact_rail_replay_state_snapshot(",
        "fn d81_apply_outer_snapshot_receipt(",
        "fn d81_snapshot_failure(",
        "fn d81_snapshot_bracket_refused(",
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
        source.contains(
            "fn d81_finish_rail_replay_state(
    outcome: D81ObserveOutcome,"
        ),
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
            && d81_spawn.contains("capture_compact_rail_replay_state_snapshot(")
            && d81_spawn.contains("finish_compact_rail_replay_state_snapshot("),
        "and production must actually spawn d81 through the split, or the two \
         halves exist and the bracket still covers both"
    );
}

#[test]
fn the_split_units_are_appended_beside_the_existing_lane_wall_units() {
    // BY SUFFIX, from the contract's pairing of each value's TYPE to the
    // family that means it: the two names were a positional array here, and
    // exchanging the emitter's two slots published each half of the split as
    // the other with every name assertion still true (Codex round 40).
    let [(inb_suffix, inb), (post_suffix, post)] =
        compact_contract::diagnostic_channel::domain_bracket_units(
            compact_contract::diagnostic_channel::InBracketMicros(10),
            compact_contract::diagnostic_channel::PostBracketMicros(90),
        );
    assert_eq!((inb, post), (10, 90));
    assert_eq!(lw_domain_unit_name(81, inb_suffix), "lw_d81_inb_us");
    assert_eq!(lw_domain_unit_name(81, post_suffix), "lw_d81_post_us");
    assert_ne!(lw_domain_unit_name(81, inb_suffix), lw_domain_crit_unit_name(81));
    assert_ne!(lw_domain_unit_name(81, post_suffix), lw_domain_crit_unit_name(81));
    assert!(
        std::ptr::eq(
            lw_domain_unit_name(81, inb_suffix),
            lw_domain_unit_name(81, inb_suffix)
        ),
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
        let mut controller_by_province: Vec<(u32, (u32, u32))> = Vec::new();
        let mut subtype0_routes = Vec::new();
        let mut node_states: Vec<(u32, (u32, u8, i64, i64))> = Vec::new();
        node_states.push((0u32, (1u32, 0u8, 5i64, 6i64)));
        for &province_id in &frame.queries[pov] {
            let (raw_handle, equivalence_id) = T31Frame::controller(province_id);
            let raw_handle = raw_handle.max(1);
            let equivalence_id = equivalence_id.max(1);
            controller_by_province.push((province_id, (raw_handle, equivalence_id)));
            subtype0_routes.push((
                D81RoutePair {
                    province_id,
                    node_index: 0,
                },
                false,
            ));
        }
        // THE PLAN'S INVARIANT IS THE CONSTRUCTOR'S JOB (lane RY3). This
        // fixture builds a `D81HubPovPlan` directly rather than through the
        // hub's builder, so it owes the same sorted-by-key ordering the builder
        // establishes; `frame.queries[pov]` is not ordered by construction.
        // A `BTreeMap` used to supply this for free.
        controller_by_province.sort_unstable_by_key(|&(province_id, _)| province_id);
        controller_by_province.dedup_by_key(|&mut (province_id, _)| province_id);
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

/// THE CONSTRUCTOR ESTABLISHES THE LOOKUP INVARIANT (lane RY3).
///
/// `D81HubPovPlan::controller_by_province` and `node_states` were `BTreeMap`s
/// and are now sorted vectors that the emission BINARY SEARCHES. Sortedness is
/// load-bearing in a quiet way: an unsorted plan does not fail loudly, it fails
/// to FIND an entry that is present and refuses the frame with
/// `IdentityMismatch` for a province the plan holds.
///
/// THIS TEST DRIVES `D81HubPovPlan::new` WITH UNSORTED INPUT, which is the only
/// form that can kill the production sort. An earlier version asserted
/// sortedness on `ek_observation_with_overlap`, whose plans are sorted literals
/// that never reach the constructor, so deleting the sort left it passing
/// (Codex r1 finding 3). The sort now lives in `new`, production builds through
/// `new`, and this asks `new` the question directly.
///
/// kills: deleting either `sort_unstable_by_key` in `D81HubPovPlan::new`.
#[test]
fn the_plan_constructor_sorts_its_lookup_vectors() {
    let plan = D81HubPovPlan::new(
        7,
        Vec::new(),
        Vec::new(),
        vec![(30u32, (3u32, 3u32)), (10, (1, 1)), (20, (2, 2))],
        vec![(5u32, (5u32, 1u8, 5i64, 5i64)), (1, (1, 0, 1, 1))],
        Vec::new(),
    );
    assert_eq!(
        plan.controller_by_province,
        vec![(10u32, (1u32, 1u32)), (20, (2, 2)), (30, (3, 3))],
        "the constructor must sort controller_by_province by province id",
    );
    assert_eq!(
        plan.node_states,
        vec![(1u32, (1u32, 0u8, 1i64, 1i64)), (5, (5, 1, 5, 5))],
        "the constructor must sort node_states by node index",
    );
}

/// AND THE EMISSION REALLY DOES DEPEND ON IT (lane RY3).
///
/// The invariant only earns the sort if breaking it changes an answer. This
/// reverses a plan's key order and shows the two emission forms then DISAGREE,
/// which is at once the proof that sortedness is load-bearing and the proof
/// that `ek_reference_emission`'s linear scan is a genuinely independent check
/// of the binary search rather than a copy of it.
///
/// kills: "simplifying" the reference back to the same lookup production uses,
/// which would make the byte-identity test above compare a thing with itself.
#[test]
fn the_emission_depends_on_the_plans_key_order() {
    let observation = ek_observation_with_overlap(true);
    for plan in &observation.povs {
        assert!(
            plan.controller_by_province.len() > 1 && plan.node_states.len() > 1,
            "a single-entry plan is sorted trivially and would not exercise this",
        );
        assert!(
            plan.controller_by_province
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "the fixture's plans must start sorted and unique by province id",
        );
        assert!(
            plan.node_states
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "the fixture's plans must start sorted and unique by node index",
        );
    }

    let mut broken = ek_observation_with_overlap(true);
    for plan in &mut broken.povs {
        plan.controller_by_province.reverse();
    }
    let reference = ek_reference_emission(&broken, &D81PhaseAccum::default())
        .expect("a linear scan finds every entry whatever order they are in");
    let searched = d81_compute_rail_hub_rows(&broken, &D81PhaseAccum::default());
    assert_ne!(
        searched.ok(),
        Some(reference),
        "reversing the plan's key order must change what the binary search \
         finds; if it does not, neither the sort nor this test is doing \
         anything",
    );
}

/// The two supply words D81 records are the two the engine's own supply provider
/// reads off a node, at the same offsets.
///
/// 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad) returns
/// `max(node+0x38, foreign)` and `node+0x38 - node+0x40`, and the replay side
/// derives XMAP16's supply-hub visibility class from exactly those two terms out
/// of `available_raw` and `consumed_raw`
/// (`portable_engine_evidence::xmap16_hub_visibility`,
/// `map_viewer::xmap16_hub_supply_terms`).
///
/// That binding is THIS side's arithmetic: the window base plus the slice offset
/// the decode reads. A replay-side test may not open a path under `extractor/` -
/// `game_ui/tests/replay_dependency_boundary.rs` fails on that reach - so the
/// guard has to live here, and without it the producer could move either offset
/// and leave the whole replay-side class law silently classifying the wrong pair
/// (Codex review of lane WK, round 6).
///
/// It drives `d81_decode_node_state`, the function PRODUCTION calls, over the
/// constants production reads with. An earlier version asserted test-local
/// constants and called the read helpers itself, which is a duplicate decoder
/// and would have stayed green through production moving either offset (round 7).
///
/// kills: changing the window base or either slice offset without noticing that
/// a consumer in the other tree is pinned to what they add up to.
#[test]
fn the_recorded_supply_words_are_the_engine_providers_two_node_fields() {
    // EVERY OFFSET IS ASSERTED AS A NUMBER, and the fixture below is written at
    // LITERAL offsets rather than through the constants.
    //
    // Writing the sentinel through the same constant the decode reads is
    // circular: change `D81_NODE_KIND_SLICE_OFF` from 0x00 to 0x08 and both
    // sides move together, so production reads `node+0x10` and the test still
    // passes (Codex round 15). Numbers on the left, numbers in the fixture.
    assert_eq!(D81_NODE_WINDOW_OFF, 0x08);
    assert_eq!(D81_NODE_WINDOW_LEN, 0x40);
    assert_eq!(D81_NODE_STRIDE, 0xE8);
    assert_eq!(D81_NODE_HEADER_BASE_OFF, 0x00);
    assert_eq!(D81_NODE_KIND_SLICE_OFF, 0x00);
    assert_eq!(D81_NODE_FLAG_SLICE_OFF, 0x04);
    assert_eq!(D81_NODE_AVAILABLE_SLICE_OFF, 0x30);
    assert_eq!(D81_NODE_CONSUMED_SLICE_OFF, 0x38);

    // What those offsets ARE in the node's own frame. This is the whole
    // cross-tree claim: the replay side reads these as the provider's +0x38 and
    // +0x40.
    assert_eq!(D81_NODE_WINDOW_OFF + D81_NODE_AVAILABLE_SLICE_OFF, 0x38);
    assert_eq!(D81_NODE_WINDOW_OFF + D81_NODE_CONSUMED_SLICE_OFF, 0x40);
    // Both i64s are inside the window the decode actually copies.
    assert!(D81_NODE_CONSUMED_SLICE_OFF + 8 <= D81_NODE_WINDOW_LEN);

    // And PRODUCTION's decode reads them THERE: a window whose fields are
    // distinct sentinels, written at the literal offsets, decodes to those
    // sentinels in that order.
    let mut node = vec![0u8; 0x40];
    node[0x00..0x04].copy_from_slice(&7u32.to_le_bytes());
    node[0x04] = 3;
    node[0x30..0x38].copy_from_slice(&1_234_500_000i64.to_le_bytes());
    node[0x38..0x40].copy_from_slice(&(-567_800_000i64).to_le_bytes());
    assert_eq!(
        d81_decode_node_state(&node),
        Some((7u32, 3u8, 1_234_500_000i64, -567_800_000i64))
    );

    // The two are not interchangeable: swapping the sentinels IN THE WINDOW
    // swaps what production decodes, so a producer reading the other pair could
    // not pass this by accident.
    let mut swapped = node.clone();
    swapped[0x30..0x38].copy_from_slice(&(-567_800_000i64).to_le_bytes());
    swapped[0x38..0x40].copy_from_slice(&1_234_500_000i64.to_le_bytes());
    assert_eq!(
        d81_decode_node_state(&swapped),
        Some((7u32, 3u8, -567_800_000i64, 1_234_500_000i64))
    );

    // A window too short for the last read decodes to nothing rather than to a
    // short answer.
    assert_eq!(d81_decode_node_state(&node[..0x3F]), None);

    // The offsets are DISTINCT, so a mutation collapsing the two i64 reads onto
    // one field fails here rather than silently halving the pair.
    assert_ne!(D81_NODE_AVAILABLE_SLICE_OFF, D81_NODE_CONSUMED_SLICE_OFF);
}

/// The window the hub reads is at the node's own base plus the window offset,
/// so the two i64 slice offsets ARE the provider's `node+0x38` and `node+0x40`.
///
/// The decode guard proves what the four offsets mean inside a window; this
/// proves where the window starts. Together they are the whole cross-tree
/// identity, and this half is the one an adjustment after the arithmetic - a
/// `wrapping_add(8)` on the formed offset - would break silently (Codex review
/// of lane WK, round 9).
///
/// kills: changing the stride, the window base, or the order of the two checked
/// steps; losing either overflow's own reason.
#[test]
fn the_window_the_hub_reads_starts_at_the_nodes_own_base() {
    // The first node's window is the node base plus the window offset, and the
    // n-th is n strides further on.
    assert_eq!(d81_node_window_offset(0), Ok(0x08));
    assert_eq!(d81_node_window_offset(1), Ok(0xE8 + 0x08));
    assert_eq!(d81_node_window_offset(37), Ok(37 * 0xE8 + 0x08));

    // WHICH IS THE CROSS-TREE CLAIM: the two i64s land at the provider's
    // +0x38 and +0x40 of the node they belong to, for every node.
    for node_index in [0u32, 1, 2, 3, 4, 5, 37, 1_000] {
        let base = (node_index as usize) * 0xE8;
        let window = d81_node_window_offset(node_index).expect("window offset");
        // Literal offsets, not the constants: writing both sides through the
        // same constant proves nothing about either (Codex round 15).
        assert_eq!(window + 0x30, base + 0x38);
        assert_eq!(window + 0x38, base + 0x40);
    }

    // Consecutive windows are one stride apart and never overlap, so no node's
    // recorded pair can be another node's.
    for node_index in [0u32, 1, 36, 999, u32::MAX - 1] {
        let here = d81_node_window_offset(node_index).expect("window offset");
        let next = d81_node_window_offset(node_index + 1).expect("next window offset");
        assert_eq!(next - here, 0xE8);
        assert!(here + D81_NODE_WINDOW_LEN <= next);
    }
}

/// The array base is the engine's pointer as read, with no arithmetic on it.
///
/// The base is the terminus of the address chain, so it is the one value that
/// must be provably unadjusted. Driven with a sentinel: whatever pointer the
/// header window carries is what the readers use.
///
/// kills: folding an adjustment into the base read; reading the pointer at any
/// other offset in the header window.
#[test]
fn the_array_base_is_the_pointer_the_header_carries() {
    let mut header = vec![0u8; 16];
    header[..8].copy_from_slice(&0x0000_1F2E_3D4C_5B6Au64.to_le_bytes());
    // The rest of the header is the capacity and the count; neither may reach
    // the base.
    header[8..].copy_from_slice(&0xDEAD_BEEF_FEED_FACEu64.to_le_bytes());
    assert_eq!(d81_node_array_base(&header), Ok(0x0000_1F2E_3D4C_5B6A));

    // A different pointer gives a different base, unshifted, including zero.
    header[..8].copy_from_slice(&0u64.to_le_bytes());
    assert_eq!(d81_node_array_base(&header), Ok(0));
    header[..8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(d81_node_array_base(&header), Ok(u64::MAX as usize));

    // A header too short for the pointer refuses under the read's own reason.
    assert_eq!(
        d81_node_array_base(&header[..7]),
        Err(D81FailureReason::Unreadable)
    );
}

/// Every production source in this crate, comment lines dropped.
///
/// The tree is WALKED rather than named, so a file added tomorrow is covered
/// the day it lands - `attach_log`'s child modules see its private items
/// through `super::*`, so a second call site can be added from any of them
/// (Codex round 15).
///
/// AND A FILE IS EXCLUDED ONLY WHEN THE CRATE ITSELF SAYS IT IS A TEST. Skipping
/// by filename was a hole: a module called `wk_mutator_tests` declared WITHOUT
/// `#[cfg(test)]` compiles into the recorder and was invisible to every census
/// (Codex round 21). So the declarations are read instead - `mod x;` with
/// `#[cfg(test)]` on it - and a file is skipped only if its own module, or a
/// module it sits under, is declared that way.
fn crate_production_sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut all: Vec<(String, String)> = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the crate's own src tree") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if !name.ends_with(".rs") {
                continue;
            }
            let shown = path.to_string_lossy().replace('\\', "/");
            let text = std::fs::read_to_string(&path).expect("a source file");
            let stripped = text
                .replace("\r\n", "\n")
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            all.push((shown, stripped));
        }
    }

    // Which file-backed modules the crate declares under `#[cfg(test)]`.
    let mut test_gated = std::collections::BTreeSet::new();
    for (_, text) in &all {
        let lines = text.lines().collect::<Vec<_>>();
        for (at, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let head = trimmed.trim_start_matches("#[cfg(test)]").trim_start();
            let declares_file_module = head.ends_with(';')
                && head
                    .trim_start_matches("pub(crate) ")
                    .trim_start_matches("pub(super) ")
                    .trim_start_matches("pub ")
                    .starts_with("mod ");
            if !declares_file_module {
                continue;
            }
            let module = head.trim_end_matches(';').rsplit(' ').next().unwrap_or("");
            if trimmed.starts_with("#[cfg(test)]") {
                test_gated.insert(module.to_string());
                continue;
            }
            // Otherwise the attribute is on the nearest preceding line that is
            // not blank (comments are already gone).
            let mut back = at;
            while back > 0 {
                back -= 1;
                let previous = lines[back].trim();
                if previous.is_empty() {
                    continue;
                }
                if previous == "#[cfg(test)]" {
                    test_gated.insert(module.to_string());
                }
                break;
            }
        }
    }

    all.retain(|(shown, _)| {
        let relative = shown.rsplit("/src/").next().unwrap_or(shown);
        relative
            .trim_end_matches(".rs")
            .split('/')
            .all(|part| !test_gated.contains(part))
    });

    assert!(
        test_gated.contains("tests"),
        "no module in this crate is declared under #[cfg(test)], which cannot be right"
    );
    // A floor, only so a walk that found nothing cannot make every census
    // below vacuous. Not a count of the crate: files come and go.
    assert!(
        all.len() >= 25,
        "the crate walk found only {} production sources",
        all.len()
    );
    assert!(
        all.iter().any(|(path, _)| path.ends_with("/src/attach_log.rs")),
        "the crate walk did not reach the file this guard is about"
    );
    all
}

/// One pinned region's span in `attach_log.rs`'s comment-stripped text.
///
/// A SPAN and not a string, because membership is a question about WHERE a line
/// is, not about whether its text occurs somewhere. Accepting any line whose
/// text appears in some region let a mutation outside every region be admitted
/// on the strength of a line that reads the same (Codex round 21).
fn pinned_span(code: &str, opener: &str, closer: &str) -> (usize, usize) {
    assert_eq!(
        code.matches(opener).count(),
        1,
        "`{opener}` occurs {} times, so pinning one of them proves nothing",
        code.matches(opener).count()
    );
    let found = code
        .find(opener)
        .unwrap_or_else(|| panic!("the read path no longer contains `{opener}`"));
    // WHAT STANDS ABOVE A PINNED OPENER IS PART OF THE REGION - and this stops
    // trying to recognise it.
    //
    // The question is one thing: `#[cfg(test)]` above the POV loop compiles the
    // whole read out of production while the opener is untouched. Six rules were
    // tried that all worked by IDENTIFYING an attribute, and every one had a
    // layout that slipped past it: a line-prefix rule missed a multi-line
    // attribute (Codex round 23); starting at the nearest `;`, `{` or `}` was
    // defeated by a comment containing a brace (round 24); a bracket balance by
    // a comment containing a bracket (round 25); a naive comment cut by a slash
    // pair inside a string (round 26); counting brackets inside strings by a
    // bracket inside one (round 27); and a string scanner by a RAW string whose
    // contents contain a quote (round 28). Each fix was correct and each was
    // one lexical form behind.
    //
    // So the region starts a FIXED NUMBER OF LINES above the opener, and
    // whatever is written there is compared like the rest of it. Nothing is
    // recognised, so no lexical form matters: any insertion above the opener
    // shifts those lines and changes the text. The cost is that a change to the
    // code just above a pinned opener updates the baseline - which is the same
    // cost the region pin already has, and the same intent.
    const LINES_ABOVE_A_PINNED_OPENER: usize = 3;
    let line_start = |at: usize| code[..at].rfind('\n').map_or(0, |line| line + 1);
    let mut at = line_start(found);
    for _ in 0..LINES_ABOVE_A_PINNED_OPENER {
        if at == 0 {
            break;
        }
        at = line_start(at - 1);
    }

    let block_end = |from: usize| -> usize {
        // From the OPENER, so an attribute above it cannot supply the brace.
        let from = from.max(found);
        let bytes = code.as_bytes();
        let mut depth = 0usize;
        let mut i = code[from..]
            .find('{')
            .unwrap_or_else(|| panic!("`{opener}` opens no block"))
            + from;
        loop {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    };
    let end = if closer.is_empty() {
        block_end(found)
    } else {
        let m = found
            + code[found..]
                .find(closer)
                .unwrap_or_else(|| panic!("`{opener}` no longer reaches `{closer}`"));
        if closer.ends_with('{') {
            block_end(m)
        } else {
            m + closer.len()
        }
    };
    (at, end)
}

/// Runs of whitespace collapsed, so indentation is not the thing being pinned.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The recorded node states are named only where this guard watches.
///
/// The regions pin their own text, but nothing stopped a NEW function
/// elsewhere in the crate from taking the plan and rewriting the pair. This
/// closes that by counting: `node_states` - the vector that carries every
/// recorded (kind, flag, available, consumed) from the decode to the wire row -
/// may not be named in the crate's production sources anywhere outside the
/// pinned regions.
///
/// kills: adding a reader or writer of the recorded node states anywhere the
/// pinned text does not cover.
/// Nothing outside this guard's regions may name what D81 records.
///
/// One census, five names, and it is the instrument the last three rounds kept
/// getting past:
///
///   * counting the CARRIER (`node_states`) stops at the wire row, where the
///     row's own fields can be rewritten (Codex round 18);
///   * counting the row's FIELDS cannot see `*row = source;`, which names none
///     of them (round 19);
///   * counting the row VECTOR (`hub_simulation`) with a `contains` exemption
///     for the unrelated `read_compact_rail_hub_simulation` lets a mutation
///     name its own variable so the exemption swallows the line (round 20).
///
/// So: occurrences are matched as WHOLE IDENTIFIERS, never substrings, and the
/// exemptions are EXACT LINES rather than fragments - lines that cannot forge a
/// value, being a refusal to `None`, a field declaration, an import, or the two
/// lines of an unrelated function that merely reads alike. Everything else must
/// sit inside a pinned region.
///
/// kills: any production code anywhere in this crate that reads, copies,
/// rewrites or re-carries the recorded pair outside the pinned path, whatever
/// it calls itself and whether or not it names a field.
#[test]
fn nothing_outside_the_pinned_path_names_what_d81_records() {
    // (the identifier, the exact lines that may name it outside a region)
    const CENSUS: &[(&str, &[&str])] = &[
        ("node_states", &[]),
        ("available_raw", &[]),
        ("consumed_raw", &[]),
        (
            "hub_simulation",
            &[
                "hub_simulation: None,",
                "hub_simulation: Option<Vec<RailHubSimulationState>>,",
            ],
        ),
        (
            "RailHubSimulationState",
            &[
                "self, RailHubSimulationState, RailNeighborState, RailProvinceState,",
                "hub_simulation: Option<Vec<RailHubSimulationState>>,",
                ") -> Result<Vec<RailHubSimulationState>, D81FirstFailure> {",
            ],
        ),
    ];

    let sources = crate_production_sources();
    // The regions all live in one file, and membership is by POSITION in it.
    let (pinned_path, pinned_code) = sources
        .iter()
        .find(|(path, _)| path.ends_with("/src/attach_log.rs"))
        .expect("the file the regions live in");
    let spans = READ_PATH_REGIONS
        .iter()
        .map(|(_, opener, closer, _)| pinned_span(pinned_code, opener, closer))
        .collect::<Vec<_>>();

    // Whole identifiers only: `read_compact_rail_hub_simulation` is not
    // `hub_simulation`, and neither is a variable named to look like it.
    let names_it = |line: &str, name: &str| -> bool {
        let bytes = line.as_bytes();
        line.match_indices(name).any(|(at, _)| {
            let before = at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
            let after_at = at + name.len();
            let after = after_at == bytes.len()
                || !(bytes[after_at].is_ascii_alphanumeric() || bytes[after_at] == b'_');
            before && after
        })
    };

    for (name, allowed) in CENSUS {
        let mut seen = 0usize;
        for (path, text) in &sources {
            let mut offset = 0usize;
            for line in text.split('\n') {
                let start = offset;
                offset += line.len() + 1;
                if !names_it(line, name) {
                    continue;
                }
                seen += 1;
                let trimmed = line.trim();
                // EXACT lines, not fragments: an exemption that is a substring
                // test can be satisfied by naming a variable after it.
                if allowed.iter().any(|form| trimmed == *form) {
                    continue;
                }
                // AND MEMBERSHIP IS BY POSITION. A line in another file is
                // outside by definition; a line in this one must fall inside a
                // region's span.
                let inside = path == pinned_path
                    && spans.iter().any(|(from, to)| start >= *from && start < *to);
                assert!(
                    inside,
                    "{path} names {name} outside every pinned region: {trimmed}"
                );
            }
        }
        assert!(seen > 0, "no production source names {name} at all");
    }
}


/// EVERY PLACE THE RECORDED NODE FIELDS PASS THROUGH, as contiguous regions of
/// production's own text.
///
/// The region used to stop where the read stopped, which left everything the
/// read HANDS ITS RESULT TO free: `D81HubPovPlan::new` can rewrite the pair on
/// its way into the plan - `*available_raw = *consumed_raw;` before the sort -
/// and the guard saw nothing, because the enumeration only watched names
/// beginning `d81_` and the constructor's own test uses equal sentinels (Codex
/// round 17). So the pin follows the VALUE, not the read: the POV body, the
/// five functions that decide what is read at a node, the plan that carries the
/// states, everything that plan does with them, and the emission that turns one
/// into a wire row.
///
/// `the_recorded_node_states_are_named_only_where_this_guard_watches` makes
/// that closure checkable rather than asserted: `node_states` may not be named
/// anywhere in the crate's production sources except inside these regions.
///
/// The baseline is the text Codex certified: rounds 8 through 17 each probed
/// this path and each confirmed current production reads the correct window at
/// the correct address through the tested decoder. What the pin does is make
/// any later change to it deliberate. It is not a semantic proof; the
/// sentinel-driven tests above are that half.
///
/// Each row is (what the region is, its opener, its closer - "" for the
/// brace-balanced block the opener starts, or a closer ending in `{` for the
/// block that closer starts - and the text it must have).
const READ_PATH_REGIONS: &[(&str, &str, &str, &str)] = &[
    (
        "the whole POV read, from the companion's header window to the plan it hands on",
        "for (&pov_country_id, authority) in &country_authority {",
        "",
        "let mut controller_witnesses: Vec<D81BytesWitness> = Vec::new(); D81PhaseAccum::bump(&phases.hub_pov_count, country_authority.len() as u64); for (&pov_country_id, authority) in &country_authority { let country = authority.country; let country_system = authority.country_system; let companion = authority.companion; let raw_handle = authority.raw_handle; let mut pov_witnesses = vec![ D81BytesWitness::new(country, authority.country_window.clone()), D81BytesWitness::new(country_system, authority.country_system_window.clone()), ]; d81_admit_pov_country_windows( pov_country_id, authority, expected_country_vtable, expected_vtable0, expected_vtable1, supply_system, equivalence_id(raw_handle), )?; let node_header_addr = companion .checked_add(NODE_DATA_OFF) .ok_or_else(|| failure(D81FailureReason::AddressOverflow))?; let node_header = d81_read_stable_bytes(node_header_addr, 16, D81FailureStage::HubHeader, &mut scratch)?; let node_data = d81_node_array_base(&node_header).map_err(|reason| failure(reason))?; let node_capacity = read_u32_from_slice(&node_header, NODE_CAPACITY_OFF - NODE_DATA_OFF) .ok_or_else(|| failure(D81FailureReason::Unreadable).index(1))?; let node_count = read_u32_from_slice(&node_header, NODE_COUNT_OFF - NODE_DATA_OFF) .ok_or_else(|| failure(D81FailureReason::Unreadable).index(2))?; pov_witnesses.push(D81BytesWitness::new(node_header_addr, node_header)); if node_count > node_capacity || node_count as usize > rail_replay::MAX_HUB_SIMULATION_ROWS || (node_count != 0 && !is_map_control_heap_pointer(node_data)) { return Err(failure(D81FailureReason::HeaderInvalid).subject(pov_country_id)); } D81PhaseAccum::bump(&phases.hub_node_slots, u64::from(node_count)); let routes_mark = Instant::now(); let (mut route_witnesses, subtype0_direct) = d81_read_route_map(companion, ROUTE_SUBTYPE0_OFF, node_count, &mut scratch)?; let (subtype1_witnesses, subtype1_direct) = d81_read_route_map(companion, ROUTE_SUBTYPE1_OFF, node_count, &mut scratch)?; let (fallback_witnesses, fallback) = d81_read_route_map(companion, ROUTE_FALLBACK_OFF, node_count, &mut scratch)?; route_witnesses.extend(subtype1_witnesses); route_witnesses.extend(fallback_witnesses); D81PhaseAccum::bump( &phases.hub_route_pairs, (subtype0_direct.len() + subtype1_direct.len() + fallback.len()) as u64, ); let relationship_ptr_addr = companion .checked_add(0xD0) .ok_or_else(|| failure(D81FailureReason::AddressOverflow))?; let relationship_ptr = read_u64_stable(relationship_ptr_addr) .ok_or_else(|| failure(D81FailureReason::Unreadable))? as usize; if !is_map_control_heap_pointer(relationship_ptr) { return Err(failure(D81FailureReason::InvalidPointer) .subject(pov_country_id) .raw(relationship_ptr as u64)); } let relationship = d81_read_stable_bytes( relationship_ptr, country_count as usize, D81FailureStage::HubClosure, &mut scratch, )?; pov_witnesses.push(D81BytesWitness::new( relationship_ptr_addr, (relationship_ptr as u64).to_le_bytes().to_vec(), )); pov_witnesses.push(D81BytesWitness::new(relationship_ptr, relationship.clone())); let candidate_provinces = subtype0_direct .iter() .chain(&subtype1_direct) .chain(&fallback) .map(|route| route.province_id) .collect::<BTreeSet<_>>(); let fallback_provinces = fallback .iter() .map(|route| route.province_id) .collect::<BTreeSet<_>>(); let mut controller_by_province: Vec<(u32, (u32, u32))> = Vec::with_capacity(candidate_provinces.len()); let mut related_by_province = BTreeMap::new(); let mut fallback_eligible_by_province = BTreeMap::new(); let mut fallback_gate_by_controller = BTreeMap::new(); D81PhaseAccum::bump( &phases.hub_candidate_provinces, candidate_provinces.len() as u64, ); phases.close(&phases.hub_routes_ns, routes_mark); let candidates_mark = Instant::now(); for province_id in candidate_provinces { let province_ptr = topology.province_ptr(province_id).ok_or_else(|| { failure(D81FailureReason::MissingEndpoint) .subject(province_id) .related(pov_country_id) })?; let (controller_raw_handle, controller_country_id) = d81_resolve_province_controller( province_ptr, province_id, &related_authority, &mut controller_by_province_frame, &mut controller_witnesses, phases, )?; let related = relationship .get(controller_country_id as usize) .copied() .ok_or_else(|| { failure(D81FailureReason::OutOfBounds) .subject(province_id) .raw(u64::from(controller_country_id)) })? != 0; controller_by_province.push((province_id, (controller_raw_handle, controller_country_id))); related_by_province.insert(province_id, related); let fallback_eligible = if related || !fallback_provinces.contains(&province_id) { false } else { let gate = if let Some(gate) = fallback_gate_by_controller.get(&controller_country_id) { *gate } else { let controller_country = *country_object_by_canonical .get(&controller_country_id) .ok_or_else(|| { failure(D81FailureReason::IdentityMismatch) .subject(province_id) .raw(u64::from(controller_country_id)) })?; let gate = d81_read_fallback_gate(controller_country, pov_country_id)?; fallback_gate_by_controller.insert(controller_country_id, gate); gate }; gate.resolved != 0 }; fallback_eligible_by_province.insert(province_id, fallback_eligible); } phases.close(&phases.hub_candidates_ns, candidates_mark); let subtype0_routes = d81_select_routes( &subtype0_direct, &fallback, &related_by_province, &fallback_eligible_by_province, ) .ok_or_else(|| failure(D81FailureReason::IdentityMismatch))?; let subtype1_routes = d81_select_routes( &subtype1_direct, &fallback, &related_by_province, &fallback_eligible_by_province, ) .ok_or_else(|| failure(D81FailureReason::IdentityMismatch))?; let needed_node_indices = subtype0_routes .iter() .chain(&subtype1_routes) .map(|(route, _)| route.node_index) .collect::<BTreeSet<_>>(); let mut node_states: Vec<(u32, (u32, u8, i64, i64))> = Vec::with_capacity(needed_node_indices.len()); let mut node_witnesses = Vec::new(); D81PhaseAccum::bump(&phases.hub_node_reads, needed_node_indices.len() as u64); if let (Some(&low), Some(&high)) = ( needed_node_indices.iter().next(), needed_node_indices.iter().next_back(), ) { D81PhaseAccum::bump( &phases.hub_node_index_span, u64::from(high - low).saturating_add(1), ); } let nodes_mark = Instant::now(); D81PhaseAccum::bump(&phases.hub_node_arrays_considered, 1); let node_array = match d81_node_array_admission( node_count, needed_node_indices.len(), node_array_retained_bytes, ) { D81NodeArrayAdmission::Empty => { D81PhaseAccum::bump(&phases.hub_node_arrays_empty, 1); None } D81NodeArrayAdmission::Sparse => { D81PhaseAccum::bump(&phases.hub_node_arrays_sparse, 1); None } D81NodeArrayAdmission::OverBudget => { D81PhaseAccum::bump(&phases.hub_node_arrays_over_budget, 1); None } D81NodeArrayAdmission::Copy => { let copied = d81_copy_node_array(node_data, node_count, &mut scratch); if let Some(bytes) = copied.as_ref() { node_array_retained_bytes = node_array_retained_bytes.saturating_add(bytes.len()); D81PhaseAccum::bump(&phases.hub_node_arrays, 1); D81PhaseAccum::bump(&phases.hub_node_array_bytes, bytes.len() as u64); } copied } }; let mut node_array_consumed: Vec<u32> = Vec::new(); for node_index in needed_node_indices { let node_offset = d81_node_window_offset(node_index).map_err(|reason| failure(reason))?; let node_address = node_data .checked_add(node_offset) .ok_or_else(|| failure(D81FailureReason::AddressOverflow))?; let served = node_array .as_ref() .and_then(|bytes| bytes.get(node_offset..node_offset + D81_NODE_WINDOW_LEN)); let node = match served { Some(window) => window.to_vec(), None => d81_read_stable_bytes( node_address, D81_NODE_WINDOW_LEN, D81FailureStage::HubObject, &mut scratch, )?, }; let node_state = d81_decode_node_state(&node) .ok_or_else(|| failure(D81FailureReason::Unreadable))?; if served.is_some() { node_array_consumed.push(node_offset as u32); } else { node_witnesses.push(D81BytesWitness::new(node_address, node)); } node_states.push((node_index, node_state)); } let pov_node_array = match node_array { Some(bytes) if !node_array_consumed.is_empty() => Some(D81NodeArrayWitness { base: node_data, bytes, consumed: node_array_consumed, }), _ => None, }; phases.close(&phases.hub_nodes_ns, nodes_mark); let foreign_mark = Instant::now(); let foreign_source = d81_read_foreign_source( supply_system, pov_country_id, country_count, &related_authority, foreign_table_cache, budget, &mut scratch, phases, ); phases.close(&phases.hub_foreign_ns, foreign_mark); let mut foreign_plan = foreign_source?; let plan = D81HubPovPlan::new( pov_country_id, subtype0_routes, subtype1_routes, controller_by_province, node_states, std::mem::take(&mut foreign_plan.related), ); if let Err(used) = budget.charge(plan.owned_bytes()) { return Err(failure(D81FailureReason::SnapshotBudgetExhausted) .subject(pov_country_id) .raw(used)); } povs.push(plan); pov_witnesses.extend(route_witnesses); pov_witnesses.extend(node_witnesses); final_node_arrays.extend(pov_node_array); pov_witnesses.extend(foreign_plan.witnesses); final_witnesses.extend(pov_witnesses); final_fallback_gates.extend( fallback_gate_by_controller .into_values() .map(|gate| (pov_country_id, gate)), ); }",
    ),
    (
        "the array base it reads out of that header",
        "fn d81_node_array_base(",
        "",
        "const D81_NODE_HEADER_BASE_OFF: usize = 0x00; fn d81_node_array_base(node_header: &[u8]) -> Result<usize, D81FailureReason> { read_u64_from_slice(node_header, D81_NODE_HEADER_BASE_OFF) .map(|base| base as usize) .ok_or(D81FailureReason::Unreadable) }",
    ),
    (
        "the window offset every node is read at",
        "fn d81_node_window_offset(",
        "",
        ".ok_or(D81FailureReason::Unreadable) } fn d81_node_window_offset(node_index: u32) -> Result<usize, D81FailureReason> { (node_index as usize) .checked_mul(D81_NODE_STRIDE) .ok_or(D81FailureReason::LengthOverflow)? .checked_add(D81_NODE_WINDOW_OFF) .ok_or(D81FailureReason::AddressOverflow) }",
    ),
    (
        "the decode of one window's four fields",
        "fn d81_decode_node_state(",
        "",
        "const D81_PROVINCE_CONTROLLER_OFF: usize = 0x188; const D81_NODE_WINDOW_LEN: usize = 0x40; fn d81_decode_node_state(node: &[u8]) -> Option<(u32, u8, i64, i64)> { Some(( read_u32_from_slice(node, D81_NODE_KIND_SLICE_OFF)?, *node.get(D81_NODE_FLAG_SLICE_OFF)?, read_i64_from_slice(node, D81_NODE_AVAILABLE_SLICE_OFF)?, read_i64_from_slice(node, D81_NODE_CONSUMED_SLICE_OFF)?, )) }",
    ),
    (
        "the choice between the bulk copy and the live reads",
        "fn d81_node_array_admission(",
        "",
        "OverBudget, } fn d81_node_array_admission( node_count: u32, needed: usize, retained_bytes: usize, ) -> D81NodeArrayAdmission { let slots = u64::from(node_count); if slots == 0 || needed == 0 { return D81NodeArrayAdmission::Empty; } if (needed as u64).saturating_mul(D81_NODE_ARRAY_MIN_DENSITY_DEN) < slots.saturating_mul(D81_NODE_ARRAY_MIN_DENSITY_NUM) { return D81NodeArrayAdmission::Sparse; } match (node_count as usize).checked_mul(D81_NODE_STRIDE) { None => D81NodeArrayAdmission::Copy, Some(len) if len > D81_BATCH_MAX_EXTENT_BYTES => D81NodeArrayAdmission::Copy, Some(len) if retained_bytes.saturating_add(len) > D81_NODE_ARRAY_FRAME_BUDGET_BYTES => { D81NodeArrayAdmission::OverBudget } Some(_) => D81NodeArrayAdmission::Copy, } }",
    ),
    (
        "the bulk copy itself",
        "fn d81_copy_node_array(",
        "",
        "const D81_NODE_ARRAY_MIN_DENSITY_NUM: u64 = 1; const D81_NODE_ARRAY_MIN_DENSITY_DEN: u64 = 3; fn d81_copy_node_array( node_data: usize, node_count: u32, scratch: &mut Vec<u8>, ) -> Option<Vec<u8>> { let len = (node_count as usize).checked_mul(D81_NODE_STRIDE)?; if len == 0 || len > D81_BATCH_MAX_EXTENT_BYTES { return None; } node_data.checked_add(len)?; let first = read_bytes(node_data, len)?; if scratch.len() < len { scratch.resize(len, 0); } let second = &mut scratch[..len]; if !read_into(node_data, second) || first.as_slice() != &*second { return None; } Some(first) }",
    ),
    (
        "the plan the read hands its node states to",
        "struct D81HubPovPlan {",
        "",
        "} #[derive(Clone, Debug, Default, PartialEq, Eq)] struct D81HubPovPlan { pov_country_id: u32, subtype0_routes: Vec<(D81RoutePair, bool)>, subtype1_routes: Vec<(D81RoutePair, bool)>, controller_by_province: Vec<(u32, (u32, u32))>, node_states: Vec<(u32, (u32, u8, i64, i64))>, foreign_related: Vec<(u32, u32, usize)>, }",
    ),
    (
        "everything that plan does with them",
        "impl D81HubPovPlan {",
        "",
        "foreign_related: Vec<(u32, u32, usize)>, } impl D81HubPovPlan { fn new( pov_country_id: u32, subtype0_routes: Vec<(D81RoutePair, bool)>, subtype1_routes: Vec<(D81RoutePair, bool)>, mut controller_by_province: Vec<(u32, (u32, u32))>, mut node_states: Vec<(u32, (u32, u8, i64, i64))>, foreign_related: Vec<(u32, u32, usize)>, ) -> Self { controller_by_province.sort_unstable_by_key(|&(province_id, _)| province_id); node_states.sort_unstable_by_key(|&(node_index, _)| node_index); Self { pov_country_id, subtype0_routes, subtype1_routes, controller_by_province, node_states, foreign_related, } } fn owned_bytes(&self) -> usize { self.subtype0_routes .len() .saturating_add(self.subtype1_routes.len()) .saturating_add(self.foreign_related.len()) .saturating_mul(D81_SNAPSHOT_VEC_ENTRY_BYTES) .saturating_add( self.controller_by_province .len() .saturating_add(self.node_states.len()) .saturating_mul(D81_SNAPSHOT_MAP_ENTRY_BYTES), ) } }",
    ),
    (
        "the emission that turns a node state into a wire row",
        "fn d81_compute_rail_hub_rows(",
        "",
        ")); } fn d81_compute_rail_hub_rows( observation: &D81HubObservation, phases: &D81PhaseAccum, ) -> Result<Vec<RailHubSimulationState>, D81FirstFailure> { let failure = |reason| D81FirstFailure::new(D81FailureStage::HubRoot, reason); let expected_rows = observation .povs .iter() .map(|plan| plan.subtype0_routes.len() + plan.subtype1_routes.len()) .sum::<usize>() .min(rail_replay::MAX_HUB_SIMULATION_ROWS + 1); let mut rows: Vec<RailHubSimulationState> = Vec::with_capacity(expected_rows); let mut foreign_source = D81ForeignSource::default(); let mut answered: Vec<(u32, i64)> = Vec::new(); for plan in &observation.povs { let pov_country_id = plan.pov_country_id; d81_hub_foreign_source_into( plan, &observation.foreign_tables, &mut foreign_source.related, ); answered.clear(); let emit_mark = Instant::now(); for (pass, (subtype, route_class, selected_routes)) in [ (0u8, 2u8, &plan.subtype0_routes), (1u8, 1u8, &plan.subtype1_routes), ] .into_iter() .enumerate() { if pass == 1 { answered.sort_unstable_by_key(|&(province_id, _)| province_id); } for &(route, fallback_record) in selected_routes { let (controller_raw_handle, controller_country_id) = plan .controller_by_province .binary_search_by_key(&route.province_id, |&(province_id, _)| province_id) .map(|at| plan.controller_by_province[at].1) .map_err(|_| { failure(D81FailureReason::IdentityMismatch).subject(route.province_id) })?; let province_owner_country_id = if fallback_record { pov_country_id } else if controller_country_id != 0 { controller_country_id } else { return Err( failure(D81FailureReason::IdentityMismatch).subject(route.province_id) ); }; let (node_kind, node_flag, available_raw, consumed_raw) = plan .node_states .binary_search_by_key(&route.node_index, |&(node_index, _)| node_index) .map(|at| plan.node_states[at].1) .map_err(|_| { failure(D81FailureReason::OutOfBounds).index(route.node_index as usize) })?; let memoised = (pass == 1) .then(|| { answered .binary_search_by_key(&route.province_id, |&(province_id, _)| { province_id }) .ok() .map(|at| answered[at].1) }) .flatten(); let foreign_max_raw = match memoised { Some(answer) => { D81PhaseAccum::bump(&phases.hub_query_memo_hits, 1); answer } None => { let answer = foreign_source.maximum_excluding_controller( phases, route.province_id, controller_raw_handle, controller_country_id, ); if pass == 0 { answered.push((route.province_id, answer)); } answer } }; rows.push(RailHubSimulationState { pov_country_id, province_id: route.province_id, province_owner_country_id, subtype, route_class, node_kind, node_flag, available_raw, consumed_raw, foreign_max_raw, foreign_route: fallback_record, fallback_record, }); if rows.len() > rail_replay::MAX_HUB_SIMULATION_ROWS { return Err(failure(D81FailureReason::OutOfBounds).raw(rows.len() as u64)); } } } phases.close(&phases.hub_emit_ns, emit_mark); } if let Some(read_failure) = observation.read_failure { return Err(read_failure); } rows.sort_unstable_by_key(|row| (row.pov_country_id, row.province_id, row.subtype)); let closure_refused = rows.windows(2).any(|pair| { (pair[0].pov_country_id, pair[0].province_id, pair[0].subtype) >= (pair[1].pov_country_id, pair[1].province_id, pair[1].subtype) }) || observation.reread_refused.unwrap_or(false); if closure_refused { return Err(failure(D81FailureReason::ClosureChanged)); } Ok(rows) }",
    ),
    (
        "the emission's result on its way into the read it is published from",
        "let hub_result = d81_compute_rail_hub_rows(&hub, &phases);",
        "observed_neighbors: observed_neighbors as u32,",
        "drop(by_id); phases.close(&phases.reciprocity_ns, reciprocity_mark); let hub_mark = Instant::now(); let hub_result = d81_compute_rail_hub_rows(&hub, &phases); let hub_closed = Instant::now(); phases.add(&phases.hub_ns, hub_mark, hub_closed); phases.add(&phases.hub_compute_ns, hub_mark, hub_closed); if provinces.is_empty() { return fail( status_flags, D81FirstFailure::new( D81FailureStage::FullClosure, D81FailureReason::EmptySnapshot, ), ); } if provinces.len() > rail_replay::MAX_PROVINCES || observed_neighbors > u32::MAX as usize { return fail( status_flags, D81FirstFailure::new(D81FailureStage::FullClosure, D81FailureReason::OutOfBounds).raw( ((provinces.len().min(u32::MAX as usize) as u64) << 32) | observed_neighbors.min(u32::MAX as usize) as u64, ), ); } if let Some(failure) = closure_failure { return fail(status_flags, failure); } let (hub_simulation, first_failure) = match hub_result { Ok(frames) => (Some(frames), None), Err(failure) => (None, Some(failure)), }; CompactRailReplayRead { plan, equality, provinces, hub_simulation, observed_neighbors: observed_neighbors as u32,",
    ),
    (
        "the encode that hands those rows to the wire crate: the pure payload half, the refusal it applies, and the inline wrapper that composes them (lane PT split the encode so d81's worker can run it; every mention of the hub rows between the three is inside this one region)",
        "fn encode_compact_rail_replay_production_payload(",
        "fn encode_compact_rail_replay_production(\n    read: &mut CompactRailReplayRead,\n    elapsed_wall_ms: u64,\n) -> Option<Vec<u8>> {",
        "EncodeRejected, } fn encode_compact_rail_replay_production_payload( read: &CompactRailReplayRead, elapsed_wall_ms: u64, ) -> Result<Vec<u8>, D81EncodeRefusal> { let observed_provinces = read.provinces.len().min(u32::MAX as usize) as u32; let Some(hub_simulation) = read.hub_simulation.as_deref() else { return Err(D81EncodeRefusal::HubSimulationAbsent); }; rail_replay::encode_payload_with_hub_simulation( &read.provinces, hub_simulation, elapsed_wall_ms, read.status_flags, observed_provinces, read.observed_neighbors, ) .map_err(|_| D81EncodeRefusal::EncodeRejected) } fn apply_d81_encode_refusal(read: &mut CompactRailReplayRead, refusal: D81EncodeRefusal) { match refusal { D81EncodeRefusal::HubSimulationAbsent => { read.status_flags |= COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE; } D81EncodeRefusal::EncodeRejected => { read.first_failure.get_or_insert_with(|| { D81FirstFailure::new(D81FailureStage::Encode, D81FailureReason::EncodeRejected) }); read.provinces.clear(); read.hub_simulation = None; read.observed_neighbors = 0; read.status_flags |= COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE; } } } fn encode_compact_rail_replay_production( read: &mut CompactRailReplayRead, elapsed_wall_ms: u64, ) -> Option<Vec<u8>> { match encode_compact_rail_replay_production_payload(read, elapsed_wall_ms) { Ok(payload) => Some(payload), Err(refusal) => { apply_d81_encode_refusal(read, refusal); None } } }",
    ),
];

/// Everything the read region calls out to that this project owns, and what
/// each one is.
///
/// `d81_*` AND `D81*`: the constructor that rewrote the pair was reached
/// through a type name, which a `d81_`-only filter never saw (Codex round 17).
const READ_REGION_CALLS: &[(&str, &str)] = &[
    ("D81BytesWitness::new", "holds the bytes the closing gate re-reads"),
    ("D81HubPovPlan::new", "pinned whole below"),
    ("D81PhaseAccum::bump", "counts the read; touches no recorded field"),
    ("d81_admit_pov_country_windows", "establishes the POV object itself; at or above `companion`"),
    ("d81_copy_node_array", "pinned whole below"),
    ("d81_decode_node_state", "pinned whole below"),
    ("d81_node_array_admission", "pinned whole below"),
    ("d81_node_array_base", "pinned whole below"),
    ("d81_node_window_offset", "pinned whole below"),
    ("d81_read_fallback_gate", "selects WHICH node indices are needed, not what is read at one"),
    ("d81_read_foreign_source", "reads the foreign supply table, a different field of the row"),
    ("d81_read_route_map", "selects WHICH node indices are needed, not what is read at one"),
    ("d81_read_stable_bytes", "the recorder's shared guarded read; not this claim's subject"),
    ("d81_resolve_province_controller", "selects WHICH node indices are needed, not what is read at one"),
    ("d81_select_routes", "selects WHICH node indices are needed, not what is read at one"),
];

/// The declarations the whole claim rests on, as they must read.
///
/// Asserting the VALUES inside a test says what the test sees, and a constant
/// can be made to say one thing under `cfg!(test)` and another in production -
/// `if cfg!(test) { 0x08 } else { 0x10 }` reads every node sixteen bytes in
/// while every fixture passes (Codex round 16). So the declarations themselves
/// are pinned, and each name may be declared only once in the crate.
const OFFSET_DECLARATIONS: &[&str] = &[
    "const D81_NODE_STRIDE: usize = 0xE8;",
    "const D81_NODE_WINDOW_OFF: usize = 0x08;",
    "const D81_NODE_WINDOW_LEN: usize = 0x40;",
    "const D81_NODE_HEADER_BASE_OFF: usize = 0x00;",
    "const D81_NODE_KIND_SLICE_OFF: usize = 0x00;",
    "const D81_NODE_FLAG_SLICE_OFF: usize = 0x04;",
    "const D81_NODE_AVAILABLE_SLICE_OFF: usize = 0x30;",
    "const D81_NODE_CONSUMED_SLICE_OFF: usize = 0x38;",
];

/// And PRODUCTION'S HUB DECODE IS THE TESTED DECODER, at the address this
/// guard's arithmetic describes.
///
/// The test above proves what `d81_decode_node_state` reads. It does NOT prove
/// the hub decode calls it: replacing the call site with four inline reads of
/// some other pair leaves the function and its constants intact, and every
/// assertion above stays green (Codex review of lane WK, round 8). The whole
/// cross-tree claim - that XMAP16's supply-hub class is derived from the engine
/// provider's `node+0x38` and `node+0x40` - rests on the call site, so the call
/// site is what has to be bound.
///
/// The hub decode itself is not drivable from a test: it sits inside the live
/// per-POV read, behind process reads, a budget and a witness ledger. So this
/// binds it structurally instead, over production's own source: the decoder has
/// exactly one call site, nothing else in the file produces a `node_state`, the
/// window address is formed exactly once out of the stride and the window base,
/// and the four slice offsets are named exactly where they are defined and
/// where the decoder reads them.
///
/// kills: bypassing, duplicating or re-implementing the node decode; reading a
/// window at any other address; adding a second reader at those offsets.
#[test]
fn the_hub_decode_reads_its_node_through_the_tested_decoder() {
    // Production's own text. Comment lines are dropped so that prose naming a
    // constant cannot satisfy - or break - a count over the code.
    const PRODUCTION: &str = include_str!("../../attach_log.rs");
    let code = PRODUCTION
        .replace("\r\n", "\n")
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    // One definition, one call, and the call is the hub decode's, verbatim.
    assert_eq!(
        code.matches("fn d81_decode_node_state(node: &[u8]) -> Option<(u32, u8, i64, i64)> {")
            .count(),
        1,
        "the tested decoder is not defined once"
    );
    assert_eq!(
        code.matches("d81_decode_node_state").count(),
        2,
        "the decoder has other mentions than its definition and its one call site"
    );

    // Nothing else in the file produces one, so an inline duplicate of the
    // decode cannot stand beside the call and pass.
    assert_eq!(
        code.matches("node_state =").count(),
        1,
        "something other than the tested decoder assigns a node state"
    );
    assert_eq!(
        code.matches("node_states.push((node_index, node_state));").count(),
        1,
        "the decoded state reaches the plan by more than one path"
    );

    // The ADDRESS is bound the same way the decode is: one function, one call,
    // one assignment, and both consumers naming that assignment. An adjustment
    // of the offset after it is formed - `node_offset.wrapping_add(8)`, which
    // shifts every field this frame records - is a second assignment and fails
    // here (Codex round 9).
    assert_eq!(
        code.matches("fn d81_node_window_offset(node_index: u32) -> Result<usize, D81FailureReason> {")
            .count(),
        1,
        "the tested window-offset function is not defined once"
    );
    assert_eq!(
        code.matches("d81_node_window_offset").count(),
        2,
        "the window offset has other mentions than its definition and its one call site"
    );
    assert_eq!(
        code.matches("node_offset =").count(),
        1,
        "the window offset is assigned somewhere other than the tested function"
    );

    // AND THE READ PATH IS PINNED AS WHOLE REGIONS, not as statements.
    //
    // A statement pin binds what each statement says and nothing about what
    // stands between them; Rust shadowing turns an inserted statement into a
    // rebinding, so a rotation of the window slid in front of the decode leaves
    // every statement intact (Codex round 12). Contiguous text has no gaps to
    // slide into.
    let region = |opener: &str, closer: &str| -> String {
        let (from, to) = pinned_span(&code, opener, closer);
        collapse(&code[from..to])
    };
    for (what, opener, closer, expected) in READ_PATH_REGIONS {
        assert_eq!(
            &region(opener, closer),
            expected,
            "{what}: this region is not the text this guard is about"
        );
    }
    // The read itself is the first row; the rest are the functions it calls.
    let region_text = READ_PATH_REGIONS[0].3;

    // THE ONE-CALL CLAIM IS ABOUT THE WHOLE CRATE, not about this file.
    //
    // `attach_log`'s child modules see its private items through `super::*`, so
    // a second call site can be added from another production file without
    // touching the scanned text at all (Codex round 15). So the counts below
    // are taken over every production source in the crate - the tree walked
    // here rather than named, so a new file is covered the day it is added.
    let crate_sources = crate_production_sources();
    // THE OFFSETS ARE PINNED AS DECLARATIONS, not just asserted as values: a
    // value assertion reports what the TEST sees, and a `cfg!(test)` constant
    // says one thing to the test and another to production (Codex round 16).
    for declaration in OFFSET_DECLARATIONS {
        let name = declaration
            .trim_start_matches("const ")
            .split(':')
            .next()
            .expect("a declaration names a constant");
        let declared = crate_sources
            .iter()
            .map(|(_, text)| text.matches(*declaration).count())
            .sum::<usize>();
        assert_eq!(
            declared, 1,
            "{declaration} is not the crate's one declaration of that offset"
        );
        let any = crate_sources
            .iter()
            .map(|(_, text)| text.matches(&format!("const {name}")).count())
            .sum::<usize>();
        assert_eq!(any, 1, "{name} is declared more than once in the crate");
    }


    // EVERYTHING THE REGION CALLS OUT TO IS ACCOUNTED FOR, and each of the five
    // that decide what is read at a node has exactly one definition and one
    // call - so the pinned body is the one production reaches.
    let mut calls = std::collections::BTreeSet::new();
    let bytes = region_text.as_bytes();
    for (i, _) in region_text.match_indices('(') {
        let mut start = i;
        while start > 0
            && (bytes[start - 1].is_ascii_alphanumeric()
                || bytes[start - 1] == b'_'
                || bytes[start - 1] == b':')
        {
            start -= 1;
        }
        // Paths too, not bare identifiers: the constructor that rewrote
        // the pair was reached as `D81HubPovPlan::new`, which a walk that
        // stopped at `:` recorded as `new` (Codex round 17).
        let name = region_text[start..i].trim_start_matches(':');
        if name.starts_with("d81_") || name.starts_with("D81") {
            calls.insert(name);
        }
    }
    let named = READ_REGION_CALLS
        .iter()
        .map(|(name, _)| *name)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        calls, named,
        "the read region calls something this guard has not accounted for"
    );
    // The emission is not called from the POV region, so the enumeration above
    // never saw it; it is nonetheless a function whose one call site carries the
    // recorded pair, and its result is pinned on its way into the read.
    assert_eq!(
        crate_sources
            .iter()
            .map(|(_, text)| text.matches("d81_compute_rail_hub_rows").count())
            .sum::<usize>(),
        2,
        "the emission has other mentions than its definition and its one call site"
    );

    for (name, disposition) in READ_REGION_CALLS {
        if *disposition == "pinned whole below" {
            // A free function is pinned by its own `fn` block; an associated
            // one - `D81HubPovPlan::new` - by the block that defines its type.
            let owner = name.split("::").next().expect("a call names something");
            assert!(
                READ_PATH_REGIONS
                    .iter()
                    .any(|(_, opener, _, _)| opener.contains(owner)),
                "{name} is filed as pinned and no region pins it"
            );
            if name.contains("::") {
                // Its one definition lives inside a pinned block, and that
                // block's text is compared in full; counting `Type::method`
                // across the crate would count call sites only.
                continue;
            }
            let across = crate_sources
                .iter()
                .map(|(_, text)| text.matches(*name).count())
                .sum::<usize>();
            assert_eq!(
                across, 2,
                "{name} has other mentions across the crate than its definition and \
                 its one call site"
            );
        }
    }

    // The bulk copy takes its two adjacent reads at that same base.
    assert_eq!(
        code.matches("let first = read_bytes(node_data, len)?;").count(),
        1,
        "the array copy's first read is not taken at the base"
    );
    assert_eq!(
        code.matches("if !read_into(node_data, second)").count(),
        1,
        "the array copy's second read is not taken at the base"
    );

    // AND THE BASE THOSE OFFSETS ARE MEASURED FROM. Both readers take the window
    // out of `node_data`, so shifting THAT shifts every recorded field just as
    // surely as shifting the offset does, and neither the offset function nor
    // the predicates above see it (Codex round 10).
    //
    // This is where the chain terminates, and it terminates because the base is
    // not computed: it is the engine's own pointer field, read once out of the
    // companion's header window at +0xB8 and never adjusted. A mutation past
    // this point is no longer "read the array at the wrong address" but "read a
    // different object", which is what the POV's authority and vtable checks
    // above are for and is not an offset question.
    assert_eq!(
        code.matches("const NODE_DATA_OFF: usize = 0xB8;").count(),
        1,
        "the header the array base is read from is no longer at the named offset"
    );
    assert_eq!(
        code.matches("node_header_addr =").count(),
        1,
        "the header address is assigned more than once"
    );
    assert_eq!(
        code.matches("node_header =").count(),
        1,
        "the header window is assigned more than once"
    );
    assert_eq!(
        code.matches("node_data =").count(),
        1,
        "the array base is adjusted or re-assigned after it is read"
    );

    // And the four offsets are named exactly twice each: where they are defined
    // and where the decoder reads them. A second reader at those offsets - the
    // shape this whole guard exists to catch - is a third mention.
    for name in [
        "D81_NODE_KIND_SLICE_OFF",
        "D81_NODE_FLAG_SLICE_OFF",
        "D81_NODE_AVAILABLE_SLICE_OFF",
        "D81_NODE_CONSUMED_SLICE_OFF",
    ] {
        assert_eq!(
            code.matches(name).count(),
            2,
            "{name} is read somewhere other than the tested decoder"
        );
    }
}
