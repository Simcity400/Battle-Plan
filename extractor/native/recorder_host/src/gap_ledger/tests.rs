use super::*;
use compact_contract::{GapCounter, GapReason, GapRecord, GapWitness};
use std::collections::BTreeMap;

fn coverage_row(hour: i32, domain: CompactDomainId, observed_perf: u32) -> GapRecord {
    GapRecord {
        hour_start: hour,
        hour_end: hour,
        reason: GapReason::CoverageShortfall,
        domains: vec![domain],
        witness: GapWitness::Coverage {
            expected_perf_records: 1,
            observed_perf_records: observed_perf,
            expected_data_records: 1,
            observed_data_records: 0,
        },
    }
}

fn status_row(hour: i32, domain: CompactDomainId, status_bits: u32) -> GapRecord {
    GapRecord {
        hour_start: hour,
        hour_end: hour,
        reason: GapReason::WorkerPanic,
        domains: vec![domain],
        witness: GapWitness::StatusBits { status_bits },
    }
}

fn outcome(
    domain: CompactDomainId,
    target: i32,
    ordinal: u64,
    reason: DomainGapReason,
) -> DomainOutcome {
    (
        domain,
        DomainGapRecordV1 {
            part: DomainGapPart::Both,
            reason,
            target_hour_raw: target,
            range_start_hour_raw: target,
            range_end_hour_raw: target,
            tick_before: u64::try_from(target).unwrap_or(0),
            tick_after: u64::try_from(target).unwrap_or(0) + 1,
            queue_capacity: 1,
            queue_depth: 1,
            queue_dropped: 3,
            outcome_ordinal: ordinal,
        },
    )
}

fn mixed_session() -> Vec<GapRecord> {
    let mut records = Vec::new();
    for hour in 100..140 {
        for domain in [7u16, 9, 11] {
            records.push(coverage_row(hour, domain, 0));
        }
        if hour % 3 == 0 {
            records.push(status_row(hour, 21, 0x100));
        }
    }
    records.push(GapRecord {
        hour_start: 141,
        hour_end: 149,
        reason: GapReason::SkippedHour,
        domains: Vec::new(),
        witness: GapWitness::HourDiscontinuity {
            previous_hour: 140,
            next_hour: 150,
        },
    });
    records.push(GapRecord {
        hour_start: 152,
        hour_end: 152,
        reason: GapReason::TornSweep,
        domains: vec![47],
        witness: GapWitness::TornSweep {
            tick_start: 152,
            tick_end: 153,
        },
    });
    records.push(GapRecord {
        hour_start: 153,
        hour_end: 164,
        reason: GapReason::Backpressure,
        domains: vec![10],
        witness: GapWitness::Counter {
            counter: GapCounter::SinkBackpressureHours,
            observed: 12,
        },
    });
    records.push(GapRecord {
        hour_start: 170,
        hour_end: 170,
        reason: GapReason::PartialData,
        domains: vec![5, 6, 8],
        witness: GapWitness::CoverageAndStatus {
            expected_perf_records: 1,
            observed_perf_records: 0,
            expected_data_records: 1,
            observed_data_records: 0,
            status_bits: 0x200,
        },
    });
    records
}

fn mixed_outcomes() -> Vec<DomainOutcome> {
    let mut outcomes = Vec::new();
    let mut next: BTreeMap<CompactDomainId, u64> = BTreeMap::new();
    for hour in 100..160 {
        for domain in [10u16, 11, 12] {
            let ordinal = next.entry(domain).or_insert(0);
            let reason = if hour % 4 == 0 {
                DomainGapReason::Backpressure
            } else {
                DomainGapReason::HourLabelMismatch
            };
            outcomes.push(outcome(domain, hour, *ordinal, reason));
            *ordinal += 1;
        }
    }
    outcomes
}

fn sorted(mut records: Vec<GapRecord>) -> Vec<GapRecord> {
    records.sort_by(|left, right| {
        (
            left.hour_start,
            left.hour_end,
            left.reason,
            left.domains.clone(),
        )
            .cmp(&(
                right.hour_start,
                right.hour_end,
                right.reason,
                right.domains.clone(),
            ))
            .then_with(|| format!("{:?}", left.witness).cmp(&format!("{:?}", right.witness)))
    });
    records
}

fn fact_set(records: &[GapRecord]) -> BTreeMap<(i32, i64), Vec<String>> {
    let mut facts: BTreeMap<(i32, i64), Vec<String>> = BTreeMap::new();
    for record in records {
        for hour in record.hour_start..=record.hour_end {
            if record.domains.is_empty() {
                facts
                    .entry((hour, -1))
                    .or_default()
                    .push(format!("{:?}/{:?}", record.reason, record.witness));
            }
            for domain in &record.domains {
                facts
                    .entry((hour, i64::from(*domain)))
                    .or_default()
                    .push(format!("{:?}/{:?}", record.reason, record.witness));
            }
        }
    }
    for value in facts.values_mut() {
        value.sort();
    }
    facts
}

fn expand(ledger: &Value) -> (Vec<GapRecord>, Vec<DomainOutcome>) {
    read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })).expect("ledger expands")
}

#[test]
fn compacted_ledger_round_trips_every_row() {
    let gaps = mixed_session();
    let outcomes = mixed_outcomes();
    let ledger = encode_gap_ledger(&gaps, &outcomes[..]);
    let (back_gaps, back_outcomes) = expand(&ledger);
    assert_eq!(sorted(back_gaps), sorted(gaps));
    assert_eq!(back_outcomes.len(), outcomes.len());
    let mut expected = outcomes.clone();
    expected.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
    let mut got = back_outcomes;
    got.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
    assert_eq!(got, expected);
}

#[test]
fn a_manifest_carrying_only_the_old_flat_arrays_is_refused() {
    let compact_stream = json!({
        "gaps": reference_flat_gaps_json(&mixed_session()),
        "domain_gap_outcomes": reference_flat_domain_gap_outcomes_json(&mixed_outcomes()),
    });
    assert_eq!(
        read_compact_stream_gap_ledger(&compact_stream),
        Err(GapLedgerError::MissingLedger)
    );
}

#[test]
fn every_hour_and_domain_fact_survives_compaction() {
    let gaps = mixed_session();
    let ledger = encode_gap_ledger(&gaps, &[][..]);
    let (back, _) = expand(&ledger);
    assert_eq!(fact_set(&back), fact_set(&gaps));
}

#[test]
fn a_domain_set_that_changes_hour_to_hour_is_not_a_cartesian_product() {
    let gaps = vec![coverage_row(5, 10, 0), coverage_row(6, 11, 0)];
    let ledger = encode_gap_ledger(&gaps, &[][..]);
    let (back, _) = expand(&ledger);
    assert_eq!(back.len(), 2, "expansion invented rows: {back:?}");
    assert_eq!(sorted(back), sorted(gaps));
}

#[test]
fn duplicate_rows_at_one_hour_survive_the_run_encoding() {
    let gaps = vec![coverage_row(5, 10, 0), coverage_row(5, 10, 0)];
    let ledger = encode_gap_ledger(&gaps, &[][..]);
    let (back, _) = expand(&ledger);
    assert_eq!(back.len(), 2);
    assert_eq!(sorted(back), sorted(gaps));
}

#[test]
fn shared_and_per_domain_rows_do_not_collapse_into_each_other() {
    let shared = vec![GapRecord {
        hour_start: 5,
        hour_end: 5,
        reason: GapReason::CoverageShortfall,
        domains: vec![10, 11],
        witness: GapWitness::Coverage {
            expected_perf_records: 1,
            observed_perf_records: 0,
            expected_data_records: 1,
            observed_data_records: 0,
        },
    }];
    let separate = vec![coverage_row(5, 10, 0), coverage_row(5, 11, 0)];
    let shared_ledger = encode_gap_ledger(&shared, &[][..]);
    let separate_ledger = encode_gap_ledger(&separate, &[][..]);
    assert_ne!(shared_ledger, separate_ledger);
    assert_eq!(sorted(expand(&shared_ledger).0), sorted(shared));
    assert_eq!(sorted(expand(&separate_ledger).0), sorted(separate));
}

#[test]
fn repeated_shared_rows_keep_their_multiplicity() {
    let row = GapRecord {
        hour_start: 5,
        hour_end: 5,
        reason: GapReason::SkippedHour,
        domains: Vec::new(),
        witness: GapWitness::HourDiscontinuity {
            previous_hour: 4,
            next_hour: 6,
        },
    };
    let gaps = vec![row.clone(), row.clone(), row];
    let ledger = encode_gap_ledger(&gaps, &[][..]);
    let (back, _) = expand(&ledger);
    assert_eq!(back.len(), 3);
    assert_eq!(sorted(back), sorted(gaps));
}

#[test]
fn a_tick_offset_too_wide_for_i64_is_stated_absolutely() {
    let gaps = vec![GapRecord {
        hour_start: i32::MIN,
        hour_end: i32::MIN,
        reason: GapReason::TornSweep,
        domains: vec![3],
        witness: GapWitness::TornSweep {
            tick_start: u64::MAX,
            tick_end: u64::MAX,
        },
    }];
    let outcomes = vec![(
        4u16,
        DomainGapRecordV1 {
            part: DomainGapPart::State,
            reason: DomainGapReason::Torn,
            target_hour_raw: i32::MIN,
            range_start_hour_raw: i32::MIN,
            range_end_hour_raw: i32::MIN,
            tick_before: u64::MAX,
            tick_after: u64::MAX - 1,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        },
    )];
    let ledger = encode_gap_ledger(&gaps, &outcomes[..]);
    let text = serde_json::to_string(&ledger).expect("the ledger serializes");
    assert!(text.contains("\"abs\""), "no absolute spelling in {text}");
    let (back_gaps, back_outcomes) = expand(&ledger);
    assert_eq!(sorted(back_gaps), sorted(gaps));
    assert_eq!(back_outcomes, outcomes);
}

#[test]
fn a_record_spanning_the_whole_hour_space_round_trips() {
    let gaps = vec![GapRecord {
        hour_start: i32::MIN,
        hour_end: i32::MAX,
        reason: GapReason::SkippedHour,
        domains: Vec::new(),
        witness: GapWitness::HourDiscontinuity {
            previous_hour: i32::MIN,
            next_hour: i32::MAX,
        },
    }];
    let ledger = encode_gap_ledger(&gaps, &[][..]);
    let (back, _) = expand(&ledger);
    assert_eq!(sorted(back), sorted(gaps));
}

#[test]
fn an_unknown_schema_or_version_is_refused_rather_than_guessed() {
    let mut ledger = encode_gap_ledger(&mixed_session(), &[][..]);
    ledger["version"] = json!(GAP_LEDGER_VERSION + 1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::UnknownVersion { .. })
    ));

    let mut renamed = encode_gap_ledger(&mixed_session(), &[][..]);
    renamed["schema"] = json!("some-other-ledger");
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": renamed })),
        Err(GapLedgerError::UnknownVersion { .. })
    ));
}

#[test]
fn a_manifest_with_no_ledger_at_all_is_an_error() {
    assert_eq!(
        read_compact_stream_gap_ledger(&json!({ "record_count": 5 })),
        Err(GapLedgerError::MissingLedger)
    );
}

#[test]
fn the_declared_row_count_is_checked_against_the_expansion() {
    let gaps = mixed_session();
    let mut ledger = encode_gap_ledger(&gaps, &[][..]);
    let classes = ledger["gaps"]["classes"].as_array_mut().expect("classes");
    classes.pop();
    let remaining = classes.len();
    ledger["gaps"]["class_count"] = json!(remaining);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::RowCountMismatch { .. })
    ));
}

#[test]
fn a_zero_length_hour_run_is_refused() {
    let mut ledger = encode_gap_ledger(&mixed_session(), &[][..]);
    ledger["gaps"]["classes"][0]["hour_runs"] = json!([[100, 0]]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn a_class_with_no_hour_runs_at_all_is_refused() {
    let mut ledger = encode_gap_ledger(&mixed_session(), &mixed_outcomes()[..]);
    ledger["gaps"]["classes"][0]["hour_runs"] = json!([]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut outcomes_ledger = encode_gap_ledger(&[], &mixed_outcomes()[..]);
    outcomes_ledger["domain_gap_outcomes"]["classes"][0]["hour_runs"] = json!([]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": outcomes_ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn a_class_that_names_no_domain_is_refused_except_where_empty_means_all() {
    let mut gaps = encode_gap_ledger(&mixed_session(), &[][..]);
    let per_domain = gaps["gaps"]["classes"]
        .as_array()
        .expect("classes")
        .iter()
        .position(|class| class["grouping"] == json!("per_domain"))
        .expect("the session carries a per-domain class");
    gaps["gaps"]["classes"][per_domain]["domains"] = json!([]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": gaps })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut outcomes = encode_gap_ledger(&[], &mixed_outcomes()[..]);
    outcomes["domain_gap_outcomes"]["classes"][0]["domains"] = json!([]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": outcomes })),
        Err(GapLedgerError::Malformed(_))
    ));

    let clean = encode_gap_ledger(&mixed_session(), &[][..]);
    assert!(clean["gaps"]["classes"]
        .as_array()
        .expect("classes")
        .iter()
        .any(|class| class["grouping"] == json!("shared") && class["domains"] == json!([])));
    assert!(read_compact_stream_gap_ledger(&json!({ "gap_ledger": clean })).is_ok());
}

#[test]
fn a_non_canonical_run_list_or_class_count_is_refused() {
    let mut split = encode_gap_ledger(&mixed_session(), &[][..]);
    {
        let classes = split["gaps"]["classes"].as_array_mut().expect("classes");
        let mut done = false;
        for class in classes.iter_mut() {
            let runs = class["hour_runs"].as_array_mut().expect("runs");
            if let Some(index) = runs
                .iter()
                .position(|run| run[1].as_u64().unwrap_or(0) >= 2)
            {
                let first = runs[index][0].as_i64().expect("first");
                let count = runs[index][1].as_u64().expect("count");
                runs.splice(
                    index..=index,
                    [json!([first, 1]), json!([first + 1, count - 1])],
                );
                done = true;
                break;
            }
        }
        assert!(done, "the session has no multi-hour run to split");
    }
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": split })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut miscounted = encode_gap_ledger(&mixed_session(), &[][..]);
    miscounted["gaps"]["class_count"] = json!(999);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": miscounted })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn a_field_this_format_does_not_define_is_refused_everywhere() {
    let gaps = mixed_session();
    let outcomes = mixed_outcomes();

    let mut extra_gap_field = encode_gap_ledger(&gaps, &[][..]);
    extra_gap_field["gaps"]["classes"][0]["novel_field"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_gap_field })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut extra_witness_field = encode_gap_ledger(&gaps, &[][..]);
    extra_witness_field["gaps"]["classes"][0]["witness"]["novel"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_witness_field })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut extra_outcome_field = encode_gap_ledger(&[], &outcomes[..]);
    extra_outcome_field["domain_gap_outcomes"]["classes"][0]["repeat"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_outcome_field })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut extra_tick_field = encode_gap_ledger(&[], &outcomes[..]);
    extra_tick_field["domain_gap_outcomes"]["classes"][0]["tick_before"]["novel"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_tick_field })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut extra_lane_field = encode_gap_ledger(&[], &outcomes[..]);
    extra_lane_field["domain_gap_outcomes"]["lane_ordinal_first"][0]["novel"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_lane_field })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn a_table_field_this_format_does_not_define_is_refused() {
    for table in ["gaps", "domain_gap_outcomes"] {
        let mut ledger = encode_gap_ledger(&mixed_session(), &mixed_outcomes()[..]);
        ledger[table]["novel_table_field"] = json!(1);
        assert!(
            matches!(
                read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
                Err(GapLedgerError::Malformed(_))
            ),
            "a field on the {table} table must be refused"
        );
    }

    let mut ledger = encode_gap_ledger(&mixed_session(), &mixed_outcomes()[..]);
    ledger["novel_ledger_field"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn the_ledger_note_must_be_a_string() {
    for bad in [json!({ "novel_nested_field": 1 }), json!([1, 2]), json!(7)] {
        let mut ledger = encode_gap_ledger(&mixed_session(), &mixed_outcomes()[..]);
        ledger["note"] = bad;
        assert!(matches!(
            read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
            Err(GapLedgerError::Malformed(_))
        ));
    }
    let mut ledger = encode_gap_ledger(&mixed_session(), &mixed_outcomes()[..]);
    ledger["note"] = json!("anything at all");
    assert!(read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })).is_ok());
}

#[test]
fn a_singleton_shared_class_and_an_unbounded_repeat_are_both_refused() {
    let mut singleton = encode_gap_ledger(&mixed_session(), &[][..]);
    let shared = singleton["gaps"]["classes"]
        .as_array()
        .expect("classes")
        .iter()
        .position(|class| class["grouping"] == json!("shared"))
        .expect("the session carries a shared class");
    singleton["gaps"]["classes"][shared]["domains"] = json!([12]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": singleton })),
        Err(GapLedgerError::Malformed(_))
    ));

    let huge_repeat = json!({ "gap_ledger": {
        "schema": GAP_LEDGER_SCHEMA,
        "version": GAP_LEDGER_VERSION,
        "gaps": { "row_count": 1, "class_count": 1, "classes": [{
            "reason": "skipped_hour", "grouping": "shared", "domains": [],
            "repeat": u64::MAX, "span_hours": 1,
            "witness": { "kind": "hour_discontinuity",
                         "previous_hour_rel": -1, "next_hour_rel": 1 },
            "hour_runs": [[0, 1]] }] },
        "domain_gap_outcomes": { "row_count": 0, "class_count": 0,
                                 "ordinal_encoding": "explicit", "classes": [] },
    }});
    assert!(matches!(
        read_compact_stream_gap_ledger(&huge_repeat),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn a_run_that_walks_past_the_last_hour_is_refused() {
    let mut ledger = encode_gap_ledger(&mixed_session(), &[][..]);
    ledger["gaps"]["classes"][0]["hour_runs"] = json!([[i32::MAX, 4u32]]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::OutOfRange(_))
    ));
}

#[test]
fn extreme_decoded_offsets_are_refused_rather_than_wrapped() {
    let mut ledger = encode_gap_ledger(&mixed_session(), &[][..]);
    ledger["gaps"]["classes"][0]["span_hours"] = json!(i64::MAX);
    ledger["gaps"]["classes"][0]["hour_runs"] = json!([[i32::MAX, 1]]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::OutOfRange(_))
    ));

    let gaps = vec![GapRecord {
        hour_start: 5,
        hour_end: 5,
        reason: GapReason::SkippedHour,
        domains: Vec::new(),
        witness: GapWitness::HourDiscontinuity {
            previous_hour: 4,
            next_hour: 6,
        },
    }];
    let mut witness_ledger = encode_gap_ledger(&gaps, &[][..]);
    witness_ledger["gaps"]["classes"][0]["witness"]["next_hour_rel"] = json!(i64::MAX);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": witness_ledger })),
        Err(GapLedgerError::OutOfRange(_))
    ));
}

#[test]
fn an_anchored_field_with_both_spellings_is_refused() {
    let gaps = vec![GapRecord {
        hour_start: 5,
        hour_end: 5,
        reason: GapReason::TornSweep,
        domains: vec![3],
        witness: GapWitness::TornSweep {
            tick_start: 5,
            tick_end: 6,
        },
    }];
    let mut ledger = encode_gap_ledger(&gaps, &[][..]);
    ledger["gaps"]["classes"][0]["witness"]["tick_start"] = json!({ "rel": 0, "abs": 5 });
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn lane_sequential_ordinals_are_derived_and_correct() {
    let outcomes = mixed_outcomes();
    let ledger = encode_gap_ledger(&[], &outcomes[..]);
    assert_eq!(
        ledger["domain_gap_outcomes"]["ordinal_encoding"],
        json!("lane_sequential")
    );
    let (_, back) = expand(&ledger);
    let mut expected = outcomes.clone();
    expected.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
    let mut got = back;
    got.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
    assert_eq!(got, expected);
}

#[test]
fn ordinals_that_hour_order_cannot_explain_are_written_out() {
    let outcomes = vec![
        outcome(10, 100, 7, DomainGapReason::Backpressure),
        outcome(10, 101, 3, DomainGapReason::Backpressure),
        outcome(10, 102, 91, DomainGapReason::Backpressure),
    ];
    let ledger = encode_gap_ledger(&[], &outcomes[..]);
    assert_eq!(
        ledger["domain_gap_outcomes"]["ordinal_encoding"],
        json!("explicit")
    );
    let (_, back) = expand(&ledger);
    let mut got = back;
    got.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
    assert_eq!(got, outcomes);
}

#[test]
fn two_outcomes_in_one_lane_at_one_hour_force_explicit_ordinals() {
    let outcomes = vec![
        outcome(10, 100, 0, DomainGapReason::Backpressure),
        outcome(10, 100, 1, DomainGapReason::Torn),
    ];
    let ledger = encode_gap_ledger(&[], &outcomes[..]);
    assert_eq!(
        ledger["domain_gap_outcomes"]["ordinal_encoding"],
        json!("explicit")
    );
    let (_, back) = expand(&ledger);
    let mut got = back;
    got.sort_by_key(|(_, record)| record.outcome_ordinal);
    assert_eq!(got, outcomes);
}

#[test]
fn an_unused_or_duplicated_lane_entry_is_refused() {
    let outcomes = mixed_outcomes();

    let mut unused = encode_gap_ledger(&[], &outcomes[..]);
    unused["domain_gap_outcomes"]["lane_ordinal_first"]
        .as_array_mut()
        .expect("lane table")
        .push(json!({ "domain_id": 999, "part": "both", "first": 0 }));
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": unused })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut duplicated = encode_gap_ledger(&[], &outcomes[..]);
    let first = duplicated["domain_gap_outcomes"]["lane_ordinal_first"][0].clone();
    duplicated["domain_gap_outcomes"]["lane_ordinal_first"]
        .as_array_mut()
        .expect("lane table")
        .push(first);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": duplicated })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn the_two_ordinal_encodings_do_not_carry_each_others_fields() {
    let outcomes = mixed_outcomes();

    let mut sequential = encode_gap_ledger(&[], &outcomes[..]);
    assert_eq!(
        sequential["domain_gap_outcomes"]["ordinal_encoding"],
        json!("lane_sequential")
    );
    sequential["domain_gap_outcomes"]["classes"][0]["ordinals"] = json!([999]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": sequential })),
        Err(GapLedgerError::Malformed(_))
    ));

    let ambiguous = vec![
        outcome(10, 100, 0, DomainGapReason::Backpressure),
        outcome(10, 100, 1, DomainGapReason::Torn),
    ];
    let mut explicit = encode_gap_ledger(&[], &ambiguous[..]);
    assert_eq!(
        explicit["domain_gap_outcomes"]["ordinal_encoding"],
        json!("explicit")
    );
    explicit["domain_gap_outcomes"]
        .as_object_mut()
        .expect("outcome table")
        .insert(
            "lane_ordinal_first".into(),
            json!([{ "domain_id": 10, "part": "both", "first": 0 }]),
        );
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": explicit })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn a_lane_sequential_ledger_over_ambiguous_rows_is_refused() {
    let outcomes = vec![
        outcome(10, 100, 0, DomainGapReason::Backpressure),
        outcome(10, 100, 1, DomainGapReason::Torn),
    ];
    let mut ledger = encode_gap_ledger(&[], &outcomes[..]);
    let table = ledger["domain_gap_outcomes"]
        .as_object_mut()
        .expect("outcome table");
    table.insert("ordinal_encoding".into(), json!("lane_sequential"));
    table.insert(
        "lane_ordinal_first".into(),
        json!([{ "domain_id": 10, "part": "both", "first": 0 }]),
    );
    for class in table["classes"].as_array_mut().expect("classes") {
        class.as_object_mut().expect("class").remove("ordinals");
    }
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

#[test]
fn the_compact_shape_is_far_smaller_than_the_flat_one_on_a_repetitive_session() {
    let mut gaps = Vec::new();
    let mut outcomes = Vec::new();
    let mut next: BTreeMap<CompactDomainId, u64> = BTreeMap::new();
    for hour in 0..2_000 {
        for domain in 1u16..=30 {
            gaps.push(coverage_row(hour, domain, 0));
            let ordinal = next.entry(domain).or_insert(0);
            outcomes.push(outcome(
                domain,
                hour,
                *ordinal,
                DomainGapReason::ReaderSilent,
            ));
            *ordinal += 1;
        }
    }
    let flat = serde_json::to_vec(&json!({
        "gaps": reference_flat_gaps_json(&gaps),
        "domain_gap_outcomes": reference_flat_domain_gap_outcomes_json(&outcomes),
    }))
    .expect("flat serializes")
    .len();
    let compact = serde_json::to_vec(&encode_gap_ledger(&gaps, &outcomes[..]))
        .expect("compact serializes")
        .len();
    assert!(
        compact * 10 < flat,
        "compact {compact} B is not far below flat {flat} B"
    );
    let (back_gaps, back_outcomes) = read_compact_stream_gap_ledger(
        &json!({ "gap_ledger": encode_gap_ledger(&gaps, &outcomes[..]) }),
    )
    .expect("expands");
    assert_eq!(fact_set(&back_gaps), fact_set(&gaps));
    assert_eq!(back_outcomes.len(), outcomes.len());
}

const CONFORMANCE_VECTOR: &str = "../scripts/gap_ledger_conformance.json";

fn canonically_ordered(mut rows: Vec<Value>) -> Vec<Value> {
    rows.sort_by_key(|row| serde_json::to_string(row).unwrap_or_default());
    rows
}

/// expander before the vector may be regenerated.
#[test]
fn the_cross_language_conformance_vector_matches_this_encoder() {
    let gaps = mixed_session();
    let outcomes = mixed_outcomes();
    let expected_gaps = canonically_ordered(
        reference_flat_gaps_json(&gaps)
            .as_array()
            .expect("flat gaps")
            .iter()
            .map(|row| {
                let mut row = row.clone();
                row.as_object_mut().expect("row").remove("h4gp_hex");
                row
            })
            .collect(),
    );
    let expected_outcomes = canonically_ordered(
        reference_flat_domain_gap_outcomes_json(&outcomes)
            .as_array()
            .expect("flat outcomes")
            .clone(),
    );
    let vector = json!({
        "note": "one encoded gap ledger and the rows it must expand to. Produced and \
                 asserted by recorder_host::gap_ledger's tests, consumed by \
                 scripts/test_gap_ledger.py, so the Rust and Python expanders cannot \
                 drift apart. Rows are sorted by their own serialization; `h4gp_hex` is \
                 absent because the ledger does not carry it.",
        "gap_ledger": encode_gap_ledger(&gaps, &outcomes[..]),
        "expected_gaps": expected_gaps,
        "expected_domain_gap_outcomes": expected_outcomes,
    });

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONFORMANCE_VECTOR);
    if std::env::var_os("FV_WRITE_GAP_LEDGER_VECTOR").is_some() {
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&vector).expect("vector serializes"),
        )
        .expect("write the conformance vector");
        return;
    }
    let on_disk: Value = serde_json::from_slice(
        &std::fs::read(&path).expect("the conformance vector is checked in"),
    )
    .expect("the conformance vector parses");
    assert_eq!(
        on_disk["gap_ledger"], vector["gap_ledger"],
        "the encoder no longer produces the checked-in vector; update the Python \
         expander first, then regenerate with FV_WRITE_GAP_LEDGER_VECTOR=1"
    );
    assert_eq!(on_disk["expected_gaps"], vector["expected_gaps"]);
    assert_eq!(
        on_disk["expected_domain_gap_outcomes"],
        vector["expected_domain_gap_outcomes"]
    );
}

#[test]
fn no_ledger_key_contains_a_dot() {
    fn walk(value: &Value, path: &str) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    assert!(!key.contains('.'), "key {key:?} at {path} contains a dot");
                    walk(child, &format!("{path}.{key}"));
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, path)),
            _ => {}
        }
    }
    walk(
        &encode_gap_ledger(&mixed_session(), &mixed_outcomes()[..]),
        "gap_ledger",
    );
}

#[test]
fn the_ordered_builder_and_from_records_agree() {
    let mut records = mixed_session();
    records.reverse();

    let unordered = GapClassTable::from_records(&records);

    let mut hour_ordered = records.clone();
    hour_ordered.sort_by_key(|record| record.hour_start);
    let mut builder = GapClassTableBuilder::default();
    for record in &hour_ordered {
        builder
            .push(record)
            .expect("hour ordered input is accepted");
    }
    let streamed = builder.finish();

    assert_eq!(streamed.to_json(), unordered.to_json());
    assert_eq!(streamed.row_count(), records.len());
    assert_eq!(sorted(streamed.rows().collect()), sorted(records));
}

#[test]
fn the_builder_refuses_a_record_for_a_closed_hour() {
    let mut builder = GapClassTableBuilder::default();
    builder.push(&coverage_row(200, 7, 0)).unwrap();
    builder.push(&coverage_row(201, 7, 0)).unwrap();
    assert_eq!(
        builder.push(&coverage_row(200, 9, 0)),
        Err(GapLedgerError::OutOfOrder {
            open_hour: 201,
            record_hour: 200,
        })
    );
}

#[test]
fn a_point_query_finds_the_interior_of_a_multi_hour_record() {
    let table = GapClassTable::from_records(&mixed_session());
    for hour in 141..=149 {
        assert!(
            table.any_covering(hour, 7, |record| record.reason == GapReason::SkippedHour),
            "the skipped range must cover hour {hour} for every domain"
        );
    }
    assert!(!table.any_covering(150, 7, |record| record.reason == GapReason::SkippedHour));
    for hour in 153..=164 {
        assert!(
            table.any_covering(hour, 10, |record| record.reason == GapReason::Backpressure),
            "the backpressure range must cover hour {hour} for domain 10"
        );
    }
    assert!(!table.any_covering(160, 11, |record| record.reason == GapReason::Backpressure));
}

#[test]
fn the_point_query_agrees_with_a_scan_of_every_row() {
    let records = mixed_session();
    let table = GapClassTable::from_records(&records);
    let mut covered = 0usize;
    for hour in 95..175 {
        for domain in [5u16, 6, 7, 8, 9, 10, 11, 21, 47, 99] {
            let scanned = records.iter().any(|record| record.covers(hour, domain));
            let indexed = table.any_covering(hour, domain, |_| true);
            assert_eq!(scanned, indexed, "hour {hour} domain {domain}");
            covered += usize::from(scanned);
        }
    }
    assert!(
        covered > 0,
        "the fixture must actually cover some domain-hours, or this proves nothing"
    );
}

#[test]
fn the_hour_ordered_merge_is_ascending_and_complete() {
    let records = mixed_session();
    let table = GapClassTable::from_records(&records);
    let merged: Vec<GapRecord> = table.rows_hour_ordered().collect();
    assert_eq!(merged.len(), records.len());
    assert!(
        merged
            .windows(2)
            .all(|pair| pair[0].hour_start <= pair[1].hour_start),
        "the merge must not go backwards"
    );
    assert_eq!(sorted(merged), sorted(records));
}

#[test]
fn duplicate_rows_are_visible_on_the_classes() {
    let per_domain_duplicate = vec![coverage_row(300, 7, 0), coverage_row(300, 7, 0)];
    assert!(GapClassTable::from_records(&per_domain_duplicate).has_duplicate_rows());

    let shared = GapRecord {
        hour_start: 300,
        hour_end: 300,
        reason: GapReason::PartialData,
        domains: vec![5, 6, 8],
        witness: GapWitness::CoverageAndStatus {
            expected_perf_records: 1,
            observed_perf_records: 0,
            expected_data_records: 1,
            observed_data_records: 0,
            status_bits: 0x200,
        },
    };
    let shared_duplicate = vec![shared.clone(), shared];
    assert!(GapClassTable::from_records(&shared_duplicate).has_duplicate_rows());

    assert!(!GapClassTable::from_records(&mixed_session()).has_duplicate_rows());
}

#[test]
fn the_class_table_is_smaller_than_its_rows_and_the_saving_grows() {
    let session = |hours: i32| {
        let mut records = Vec::new();
        for hour in 0..hours {
            for domain in [7u16, 9, 11, 13, 17] {
                records.push(coverage_row(hour, domain, 0));
            }
        }
        records
    };
    let mut previous = 0.0f64;
    for hours in [100i32, 1_000, 10_000] {
        let records = session(hours);
        let table = GapClassTable::from_records(&records);
        let rows_bytes = gap_records_resident_bytes(&records) as f64;
        let table_bytes = table.resident_bytes().max(1) as f64;
        let ratio = rows_bytes / table_bytes;
        assert!(
            ratio > previous,
            "the saving must grow with the session: {hours} hours gave {ratio:.1}x, \
             the shorter session gave {previous:.1}x"
        );
        previous = ratio;
    }
    assert!(
        previous > 100.0,
        "10,000 hours of one stable domain set should compact by far more than 100x, \
         got {previous:.1}x"
    );
}

fn out_of_order_outcomes(hours: i32) -> Vec<DomainOutcome> {
    let mut outcomes = Vec::new();
    let mut next: BTreeMap<CompactDomainId, u64> = BTreeMap::new();
    for step in 0..hours {
        let hour = 60_832_235 + step;
        for domain in [21u16, 33, 47] {
            if domain == 33 && step % 2 == 1 {
                continue;
            }
            if domain == 47 && step % 5 != 0 {
                continue;
            }
            let ordinal = next.entry(domain).or_default();
            outcomes.push((
                domain,
                DomainGapRecordV1 {
                    reason: if step % 7 == 3 {
                        DomainGapReason::Backpressure
                    } else {
                        DomainGapReason::Overrun
                    },
                    part: if domain == 47 {
                        DomainGapPart::Event
                    } else {
                        DomainGapPart::State
                    },
                    target_hour_raw: hour,
                    range_start_hour_raw: hour,
                    range_end_hour_raw: hour,
                    outcome_ordinal: *ordinal,
                    queue_capacity: 64,
                    queue_depth: 1,
                    queue_dropped: 0,
                    tick_before: 0,
                    tick_after: 0,
                },
            ));
            *ordinal += 1;
        }
    }
    let split = outcomes.len() / 3;
    outcomes.rotate_left(split);
    outcomes
}

#[test]
fn the_streaming_encoder_is_byte_identical_to_the_walk_it_replaced() {
    for hours in [1i32, 2, 7, 60, 400] {
        let outcomes = out_of_order_outcomes(hours);
        let streamed = serde_json::to_vec(&encode_domain_gap_outcomes(&outcomes[..])).unwrap();
        let reference = serde_json::to_vec(&reference_encode_domain_gap_outcomes(
            outcomes.iter().copied(),
        ))
        .unwrap();
        assert_eq!(
            streamed,
            reference,
            "the two encoders disagree at {hours} hours:\n  streamed  {}\n  reference {}",
            String::from_utf8_lossy(&streamed),
            String::from_utf8_lossy(&reference),
        );
        let ledger = encode_domain_gap_outcomes(&outcomes[..]);
        assert_eq!(ledger["ordinal_encoding"], "lane_sequential");
        assert_eq!(
            ledger["row_count"].as_u64(),
            Some(outcomes.len() as u64),
            "every row must be accounted for at {hours} hours"
        );
    }

    let mut ambiguous = out_of_order_outcomes(40);
    let mut twin = ambiguous[0];
    twin.1.outcome_ordinal = u64::MAX / 2;
    ambiguous.push(twin);
    let streamed = serde_json::to_vec(&encode_domain_gap_outcomes(&ambiguous[..])).unwrap();
    let reference = serde_json::to_vec(&reference_encode_domain_gap_outcomes(
        ambiguous.iter().copied(),
    ))
    .unwrap();
    assert_eq!(
        streamed, reference,
        "the two encoders disagree on the explicit arm"
    );
    assert_eq!(
        encode_domain_gap_outcomes(&ambiguous[..])["ordinal_encoding"],
        "explicit",
        "this fixture must exercise the arm it was built for"
    );
}

#[test]
fn the_streaming_encoder_expands_back_to_its_own_input() {
    for hours in [1i32, 7, 400] {
        let outcomes = out_of_order_outcomes(hours);
        let ledger = encode_gap_ledger(&[], &outcomes[..]);
        let (_, back) =
            read_compact_stream_gap_ledger(&serde_json::json!({ "gap_ledger": ledger }))
                .expect("the ledger must expand");
        let mut expected = outcomes.clone();
        expected.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
        let mut observed = back;
        observed.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
        assert_eq!(
            observed, expected,
            "a row was lost or invented at {hours} hours"
        );
    }
}

#[test]
fn the_streaming_encoder_keeps_no_ordinals_under_lane_sequential() {
    let outcomes = out_of_order_outcomes(500);
    let ledger = encode_domain_gap_outcomes(&outcomes[..]);
    assert_eq!(ledger["ordinal_encoding"], "lane_sequential");
    assert_eq!(ledger["row_count"].as_u64(), Some(outcomes.len() as u64));
    let classes = ledger["classes"].as_array().expect("classes");
    assert!(!classes.is_empty());
    for class in classes {
        assert!(
            class.get("ordinals").is_none(),
            "a lane_sequential class carries no ordinals: {class}"
        );
    }

    let mut ambiguous = outcomes;
    let mut twin = ambiguous[0];
    twin.1.outcome_ordinal = u64::MAX / 2;
    ambiguous.push(twin);
    let explicit = encode_domain_gap_outcomes(&ambiguous[..]);
    assert_eq!(explicit["ordinal_encoding"], "explicit");
    assert!(
        explicit["classes"]
            .as_array()
            .expect("classes")
            .iter()
            .any(|class| class.get("ordinals").is_some()),
        "the explicit arm must write the ordinals out"
    );
}
