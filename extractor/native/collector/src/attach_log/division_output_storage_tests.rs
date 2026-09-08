use super::*;
use compact_contract::division_routes::{
    CombatClass, CombatIdentity, CountryWordsObservation, FirstCombatCountriesObservation,
};

fn row(id: u32, words: &[i32]) -> DivisionMovementSample {
    let mut row = deleted_division_sample(id, DIVISION_TYPE);
    row.deleted = false;
    row.location = 1;
    row.raw_max_organisation = Some(100_000);
    row.combat_ref_count = 1;
    row.in_combat = true;
    let combat = CombatIdentity {
        class: CombatClass::Land,
        kind: 62,
        id: 7,
    };
    row.movement_scalars.first_combat = Some(Some(combat));
    row.first_combat_countries = FirstCombatCountriesObservation::Present {
        combat,
        countries: CountryWordsObservation::Measured(words.to_vec()),
    };
    row
}

fn bytes(row: &DivisionMovementSample) -> &[i32] {
    match &row.first_combat_countries {
        FirstCombatCountriesObservation::Present {
            countries: CountryWordsObservation::Measured(words),
            ..
        } => words,
        _ => panic!("expected measured country words"),
    }
}

#[test]
fn complete_membership_clone_requires_exact_four_bytes_per_word() {
    let sample = row(1, &[i32::MIN, -1, -1, 0, i32::MAX]);
    let budget = country_roster_readset::Budget::test_limits(0, 20);
    let copied = try_clone_division_movement_sample(&sample, &budget).unwrap();
    assert_eq!(copied, sample);
    assert_ne!(bytes(&sample).as_ptr(), bytes(&copied).as_ptr());
    assert_eq!(budget.retained_bytes(), 20);
    assert_eq!(bytes(&copied).len(), 5);
    if let FirstCombatCountriesObservation::Present {
        countries: CountryWordsObservation::Measured(words),
        ..
    } = &copied.first_combat_countries
    {
        assert_eq!(words.capacity(), 5);
    }
    let refused = country_roster_readset::Budget::test_limits(0, 19);
    assert_eq!(
        try_clone_division_movement_sample(&sample, &refused),
        Err(DivisionSampleFailure::OutputStorageUnavailable)
    );
    assert_eq!(bytes(&sample), [i32::MIN, -1, -1, 0, i32::MAX]);
}

#[test]
fn fresh_frame_quota_accounts_for_retained_pin_capacity() {
    let mut sample = row(1, &[]);
    let mut words = Vec::with_capacity(32);
    words.extend_from_slice(&[-7, 5]);
    let retained = words.capacity() * std::mem::size_of::<i32>();
    let FirstCombatCountriesObservation::Present { countries, .. } =
        &mut sample.first_combat_countries
    else {
        unreachable!()
    };
    *countries = CountryWordsObservation::Measured(words);
    let mut tracker: FrameJournalled<BTreeMap<u32, DivisionMovementSample>> = FrameJournalled::default();
    tracker.advance(BTreeMap::from([(1, sample)]));
    tracker.close_frame_transaction();
    let short = country_roster_readset::Budget::test_limits(0, retained - 1);
    assert_eq!(
        division_output_storage::retain_baseline_membership(&tracker, &short),
        Err(DivisionSampleFailure::OutputStorageUnavailable)
    );
    let exact = country_roster_readset::Budget::test_limits(0, retained);
    division_output_storage::retain_baseline_membership(&tracker, &exact).unwrap();
    assert_eq!(exact.retained_bytes(), retained);
}

#[test]
fn empty_and_unavailable_membership_copy_without_manufacturing_words() {
    let mut sample = row(1, &[]);
    let quota = country_roster_readset::Budget::test_limits(0, 0);
    assert_eq!(
        try_clone_division_movement_sample(&sample, &quota).unwrap(),
        sample
    );
    sample.first_combat_countries = FirstCombatCountriesObservation::NoFirstCombat;
    sample.movement_scalars.first_combat = Some(None);
    sample.combat_ref_count = 0;
    sample.in_combat = false;
    assert_eq!(
        try_clone_division_movement_sample(&sample, &quota).unwrap(),
        sample
    );
    sample.first_combat_countries =
        FirstCombatCountriesObservation::Unavailable(Default::default());
    sample.movement_scalars.first_combat = None;
    assert_eq!(
        try_clone_division_movement_sample(&sample, &quota).unwrap(),
        sample
    );
    assert_eq!(quota.retained_bytes(), 0);
}

#[test]
fn later_row_copy_refusal_leaves_entire_baseline_and_journal_unchanged() {
    let old = row(9, &[8, 8]);
    let mut tracker: FrameJournalled<BTreeMap<u32, DivisionMovementSample>> = FrameJournalled::default();
    tracker.advance(BTreeMap::from([(9, old.clone())]));
    tracker.close_frame_transaction();
    let emitted_slots = 3 * std::mem::size_of::<DivisionMovementSample>();
    // Old pin 8B, first copied row 4B, second copied row needs another 8B.
    let budget = country_roster_readset::Budget::test_limits(0, 8 + emitted_slots + 4 + 7);
    division_output_storage::retain_baseline_membership(&tracker, &budget).unwrap();
    let result = compact_division_emitted_rows(
        vec![row(1, &[1]), row(2, &[2, 3])],
        2,
        0,
        &mut tracker,
        &budget,
    );
    assert_eq!(result, Err(DivisionSampleFailure::OutputStorageUnavailable));
    assert_eq!(tracker.current, BTreeMap::from([(9, old)]));
    assert!(!tracker.has_frame_entry());
}

#[test]
fn unchanged_rows_move_their_lists_and_only_retain_the_old_pin() {
    let sample = row(1, &[9, -7]);
    let mut tracker: FrameJournalled<BTreeMap<u32, DivisionMovementSample>> = FrameJournalled::default();
    tracker.advance(BTreeMap::from([(1, sample.clone())]));
    tracker.close_frame_transaction();
    let current_ptr = bytes(&sample).as_ptr();
    let budget = country_roster_readset::Budget::test_limits(0, 16);
    division_output_storage::retain_baseline_membership(&tracker, &budget).unwrap();
    budget.retain(8).unwrap(); // This hour's acquired list already exists.
    let (emitted, changed, flags) =
        compact_division_emitted_rows(vec![sample], 1, 0, &mut tracker, &budget).unwrap();
    assert!(emitted.is_empty());
    assert_eq!(changed, 0);
    assert_ne!(flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(budget.retained_bytes(), 16);
    assert_eq!(bytes(tracker.get(&1).unwrap()).as_ptr(), current_ptr);
    assert!(tracker.has_frame_entry());
}

#[test]
fn published_storage_failure_recovers_full_then_empty_delta_and_sink_refusal_restores() {
    let sample = row(1, &[i32::MIN, 5, 5]);
    let mut tracker = FrameJournalled::default();
    compact_division_emitted_rows(
        vec![sample.clone()],
        1,
        0,
        &mut tracker,
        &country_roster_readset::Budget::default(),
    )
    .unwrap();
    tracker.close_frame_transaction();
    let failure = CompactDivisionRead {
        rows: Vec::new(),
        read_time_us: 0,
        changed_rows: 0,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        root_rows: 1,
        shard_panic_index: None,
        output_storage_unavailable: true,
        frame_identity_refusal: None,
    };
    stage_division_publication_baseline(&failure, &mut tracker);
    assert!(tracker.is_empty());
    tracker.restore_frame_entry();
    assert_eq!(tracker.get(&1), Some(&sample));
    stage_division_publication_baseline(&failure, &mut tracker);
    tracker.close_frame_transaction();
    let (full, changed, flags) = compact_division_emitted_rows(
        vec![sample.clone()],
        1,
        0,
        &mut tracker,
        &country_roster_readset::Budget::default(),
    )
    .unwrap();
    assert_eq!(full, [sample.clone()]);
    assert_eq!(changed, 1);
    assert_eq!(flags & COMPACT_STATUS_DELTA_DATA, 0);
    tracker.close_frame_transaction();
    let (delta, changed, flags) = compact_division_emitted_rows(
        vec![sample],
        1,
        0,
        &mut tracker,
        &country_roster_readset::Budget::default(),
    )
    .unwrap();
    assert!(delta.is_empty());
    assert_eq!(changed, 0);
    assert_ne!(flags & COMPACT_STATUS_DELTA_DATA, 0);
}

#[test]
fn payload_funds_complete_prefix_and_membership_before_allocating() {
    let sample = row(1, &[i32::MIN, 0, 0, i32::MAX]);
    let rows = [sample];
    let generous = country_roster_readset::Budget::default();
    let expected =
        compact_divisions_data_payload(&rows, 0, 0, 1, 1, None, None, &generous).unwrap();
    assert_eq!(generous.retained_bytes(), expected.len());
    let exact = country_roster_readset::Budget::test_limits(0, expected.len());
    let payload = compact_divisions_data_payload(&rows, 0, 0, 1, 1, None, None, &exact).unwrap();
    assert_eq!(payload, expected);
    assert_eq!(payload.capacity(), expected.len());
    let parsed = compact_contract::division_routes::DivisionRoutes::parse(&payload).unwrap();
    assert_eq!(
        parsed.first_combat_countries(0).unwrap(),
        rows[0].first_combat_countries
    );
    let short = country_roster_readset::Budget::test_limits(0, expected.len() - 1);
    assert!(compact_divisions_data_payload(&rows, 0, 0, 1, 1, None, None, &short).is_err());
}

#[test]
fn final_copy_funds_old_and_new_frame_backing_and_refusal_preserves_prefix() {
    let prefix = [4u8, 7, 2, 9];
    let mut frame = prefix.to_vec();
    let original_pointer = frame.as_ptr();
    let original_capacity = frame.capacity();
    let additional = usize::from(COMPACT_RECORD_HEADER_LEN) * 2 + 40 + 100;
    let quota = country_roster_readset::Budget::test_limits(0, frame.len() + additional - 1);
    assert!(division_output_storage::reserve_publication(&mut frame, 100, &[], &quota).is_err());
    assert_eq!(frame, prefix);
    assert_eq!(frame.as_ptr(), original_pointer);
    assert_eq!(frame.capacity(), original_capacity);
    let quota = country_roster_readset::Budget::test_limits(0, frame.len() + additional);
    division_output_storage::reserve_publication(&mut frame, 100, &[], &quota).unwrap();
    assert_eq!(frame, prefix);
    assert_eq!(frame.capacity(), frame.len() + additional);
    assert_eq!(quota.retained_bytes(), frame.len() + additional);
}

#[test]
fn precharged_gap_survives_exhausted_semantic_quota_as_one_terminal() {
    let bytes =
        usize::from(COMPACT_RECORD_HEADER_LEN) + compact_contract::COMPACT_DOMAIN_GAP_PAYLOAD_LEN;
    let prefix = [1, 2, 3, 4];
    let mut out = prefix.to_vec();
    assert_eq!(out.len(), out.capacity());
    let budget = Arc::new(country_roster_readset::Budget::test_limits(
        0,
        prefix.len() + bytes * 4,
    ));
    let hour = 60_759_359;
    let reservation =
        division_output_storage::reserve_storage_gap(&mut out, &budget, hour).unwrap();
    assert_eq!(budget.retained_bytes(), prefix.len() + bytes * 4);
    let weak = Arc::downgrade(&budget);
    let pointer = out.as_ptr();
    let capacity = out.capacity();
    assert!(budget.retain(1).is_err());
    assert!(division_output_storage::reserve_publication(&mut out, 100, &[], &budget).is_err());
    let copies = division_output_storage::append_storage_gap(&mut out, 1, hour, 0, reservation);
    assert_eq!(out.as_ptr(), pointer);
    assert_eq!(out.capacity(), capacity);
    assert_eq!(capacity, prefix.len() + bytes);
    assert_eq!(budget.retained_bytes(), prefix.len() + bytes * 4 + 1);
    assert_eq!(&out[..prefix.len()], &prefix);
    let header = compact_contract::parse_compact_record_header(&out[prefix.len()..]).unwrap();
    assert_eq!(header.domain_id, COMPACT_DOMAIN_DIVISIONS);
    assert_eq!(
        header.record_type,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP
    );
    assert_eq!(header.record_len(), out.len() - prefix.len());
    let payload = &out[prefix.len() + usize::from(COMPACT_RECORD_HEADER_LEN)..];
    let gap = compact_contract::domain_gap::DomainGapRecordV1::decode(payload).unwrap();
    assert_eq!(
        gap.reason,
        compact_contract::domain_gap::DomainGapReason::SourceUnavailable
    );
    assert_eq!(gap.target_hour_raw, hour);
    assert_eq!(
        compact_contract::domain_gap::DomainGapRecordV1::decode_witness_provenance(payload)
            .unwrap(),
        compact_contract::domain_gap::DomainGapWitnessProvenance::Unobserved
    );
    drop(budget);
    assert!(weak.upgrade().is_some(), "GAP keeps its copy quota alive");
    drop(out);
    drop(copies);
    assert!(weak.upgrade().is_none());
}

#[test]
fn control_setup_refusal_preserves_existing_bytes_and_backing() {
    let bytes =
        usize::from(COMPACT_RECORD_HEADER_LEN) + compact_contract::COMPACT_DOMAIN_GAP_PAYLOAD_LEN;
    let mut out = vec![2, 4, 6, 8];
    let pointer = out.as_ptr();
    let capacity = out.capacity();
    let budget = Arc::new(country_roster_readset::Budget::test_limits(
        0,
        out.len() + bytes * 4 - 1,
    ));
    assert!(division_output_storage::reserve_storage_gap(&mut out, &budget, 60_759_359).is_err());
    assert_eq!(out, [2, 4, 6, 8]);
    assert_eq!(out.as_ptr(), pointer);
    assert_eq!(out.capacity(), capacity);
}

#[test]
fn stack_perf_encoding_matches_existing_diagnostic_wire() {
    let read = CompactDivisionRead {
        rows: Vec::new(),
        read_time_us: 37,
        changed_rows: 2,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        root_rows: 5,
        shard_panic_index: Some(3),
        output_storage_unavailable: true,
        frame_identity_refusal: Some(Default::default()),
    };
    let (units, count) = division_output_storage::diagnostics(&read);
    let units = &units[..count];
    assert_eq!(count, 5);
    let perf = division_output_storage::encode_perf(&read, 17, units).unwrap();
    let mut actual = Vec::new();
    division_output_storage::reserve_publication(
        &mut actual,
        0,
        units,
        &country_roster_readset::Budget::default(),
    )
    .unwrap();
    division_output_storage::append_perf(&mut actual, 3, 60_759_359, &perf);
    let mut expected = Vec::new();
    append_compact_domain_perf_record_with_diagnostics(
        &mut expected,
        3,
        60_759_359,
        CompactDomainPerfRecord {
            domain_id: COMPACT_DOMAIN_DIVISIONS,
            read_time_us: 37,
            row_count: 5,
            changed_rows: 2,
            unit_count: 5,
            changed_unit_count: 2,
            status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        },
        17,
        units,
    );
    assert_eq!(actual, expected);
}

#[test]
fn frame_copy_capacity_reservation_survives_pending_and_staged_ownership() {
    let read = CompactDivisionRead {
        rows: Vec::new(),
        read_time_us: 0,
        changed_rows: 0,
        status_flags: 0,
        root_rows: 0,
        shard_panic_index: None,
        output_storage_unavailable: false,
        frame_identity_refusal: None,
    };
    let perf = division_output_storage::encode_perf(&read, 0, &[]).unwrap();
    let mut payload = Vec::with_capacity(64);
    payload.push(0);
    let contribution = payload.capacity() + 40 + usize::from(COMPACT_RECORD_HEADER_LEN) * 2;
    let reserved = contribution * (division_output_storage::MAX_LIVE_FRAME_COPIES - 1);
    let refused = Arc::new(country_roster_readset::Budget::test_limits(0, reserved - 1));
    assert!(division_output_storage::reserve_frame_copies(
        payload.capacity(),
        &perf,
        60_759_359,
        &refused
    )
    .is_err());
    let budget = Arc::new(country_roster_readset::Budget::test_limits(0, reserved));
    let weak = Arc::downgrade(&budget);
    let copies = division_output_storage::reserve_frame_copies(
        payload.capacity(),
        &perf,
        60_759_359,
        &budget,
    )
    .unwrap();
    assert_eq!(budget.retained_bytes(), reserved);
    let timing = CompactFramePhaseTiming {
        division_copy_reservation: Some(copies),
        ..Default::default()
    };
    drop(budget);
    assert!(
        weak.upgrade().is_some(),
        "pending timing owns the same acquisition quota"
    );
    let staged = StagedPublication {
        division_copy_reservation: timing.division_copy_reservation.clone(),
        bytes: payload,
        staged_after_frame: 1,
        kind: StagedPublicationKind::ControlGap { group_len: 1 },
    };
    drop(timing);
    assert!(
        weak.upgrade().is_some(),
        "staging preserves the receipt after timing is retired"
    );
    drop(staged);
    assert!(weak.upgrade().is_none());
}

#[test]
fn downstream_copy_graph_changes_require_reauditing_the_four_buffer_bound() {
    #[derive(Default)]
    struct Census([usize; 4]);
    impl<'ast> syn::visit::Visit<'ast> for Census {
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            for (index, name) in [(0, "clone"), (1, "to_vec"), (3, "splice")] {
                if call.method == name {
                    self.0[index] += 1;
                }
            }
            syn::visit::visit_expr_method_call(self, call);
        }
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = &*call.func {
                let mut segments = path.path.segments.iter().rev();
                if segments
                    .next()
                    .is_some_and(|part| part.ident == "with_capacity")
                    && segments.next().is_some_and(|part| part.ident == "Vec")
                {
                    self.0[2] += 1;
                }
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    // The review proves ownership/aliasing; this syntax census makes any new
    // copy/allocation site in that graph require a new review of the bound.
    let source = syn::parse_file(include_str!("../attach_log.rs")).unwrap();
    for (name, expected) in [
        ("replace_terminal_hook_domain_records", [0, 0, 1, 0]),
        ("merge_terminal_d45_records", [1, 0, 0, 0]),
        ("merge_terminal_d55_records", [0, 0, 0, 0]),
        ("merge_terminal_d67_records", [0, 1, 0, 0]),
        ("merge_terminal_d75_d64_records", [0, 0, 0, 0]),
        ("merge_terminal_d75_d64_payloads", [1, 1, 1, 0]),
        ("insert_a30_payloads_before_frame_end", [0, 0, 0, 1]),
        ("insert_a38_payload_before_frame_end", [0, 0, 0, 1]),
        (
            "insert_production_render_payload_before_frame_end",
            [0, 0, 0, 1],
        ),
        (
            "insert_production_render_record_before_frame_end",
            [0, 0, 0, 0],
        ),
        (
            "insert_compact_scheduler_probe_before_frame_end",
            [0, 0, 0, 1],
        ),
        (
            "insert_frame_degraded_notice_before_frame_end",
            [0, 0, 0, 1],
        ),
    ] {
        let function = source
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(function) if function.sig.ident == name => Some(function),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing reviewed copy-graph function {name}"));
        let mut census = Census::default();
        syn::visit::Visit::visit_item_fn(&mut census, function);
        assert_eq!(
            census.0, expected,
            "{name}: re-audit before changing MAX_LIVE_FRAME_COPIES"
        );
    }
    assert_eq!(division_output_storage::MAX_LIVE_FRAME_COPIES, 4);
}
