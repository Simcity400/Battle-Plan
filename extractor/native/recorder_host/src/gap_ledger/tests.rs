//! What the gap ledger has to be right about.
//!
//! Every test below names the production change that makes it fail, because a
//! compaction whose tests cannot fail is worse than no compaction: it makes the
//! loss look proven.

use super::*;
use compact_contract::{GapCounter, GapReason, GapRecord, GapWitness};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

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

/// A session with every witness kind, several domains, a multi-hour span, a
/// global (domains-empty) record and a multi-domain record.
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

/// Records sorted into a canonical order so a comparison is about content
/// rather than about the order two encodings happened to produce.
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

/// The fact set the ledger exists to carry: for every hour a record covers and
/// every domain it affects, the reason and the exact witness.
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

// ---------------------------------------------------------------------------
// Round trips
// ---------------------------------------------------------------------------

/// The whole claim, on a session using every witness kind and both groupings.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: any encoder grouping that merges two
/// rows the decoder cannot separate again, or any witness field dropped from the
/// class.
#[test]
fn compacted_ledger_round_trips_every_row() {
    let gaps = mixed_session();
    let outcomes = mixed_outcomes();
    let ledger = encode_gap_ledger(&gaps, outcomes.iter().copied());
    let (back_gaps, back_outcomes) = expand(&ledger);
    assert_eq!(sorted(back_gaps), sorted(gaps));
    assert_eq!(back_outcomes.len(), outcomes.len());
    let mut expected = outcomes.clone();
    expected.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
    let mut got = back_outcomes;
    got.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
    assert_eq!(got, expected);
}

/// There is ONE format. A manifest carrying only the flat arrays this replaced
/// is refused, not silently read.
///
/// The old shape is deliberately unsupported (owner ruling 2026-08-09: captures
/// sealed before this are disposable test data). What matters is that the
/// refusal is LOUD: a reader that quietly returned nothing here would report a
/// capture with no gaps at all, which is the strongest possible false claim this
/// ledger can make.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: reintroducing a fallback that reads
/// `compact_stream.gaps`, or defaulting a missing ledger to empty.
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

/// Hour attribution is the owner's stop condition, so it gets its own assertion
/// rather than riding on record equality: the same (hour, domain) facts, no more
/// and no fewer.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: expanding a class as the Cartesian
/// product of its domain list with its hour range, or an off-by-one in the run
/// walk.
#[test]
fn every_hour_and_domain_fact_survives_compaction() {
    let gaps = mixed_session();
    let ledger = encode_gap_ledger(&gaps, []);
    let (back, _) = expand(&ledger);
    assert_eq!(fact_set(&back), fact_set(&gaps));
}

// ---------------------------------------------------------------------------
// The failure modes the shape is exposed to
// ---------------------------------------------------------------------------

/// The exact-set rule. Domain 10 gapped at hour 5 and domain 11 at hour 6 must
/// stay two rows: a class that multiplied its domain list by its hour range
/// would also claim (10,6) and (11,5), which is hour misattribution by
/// construction.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: forming a class's domain list across
/// hours instead of independently at each hour.
#[test]
fn a_domain_set_that_changes_hour_to_hour_is_not_a_cartesian_product() {
    let gaps = vec![coverage_row(5, 10, 0), coverage_row(6, 11, 0)];
    let ledger = encode_gap_ledger(&gaps, []);
    let (back, _) = expand(&ledger);
    assert_eq!(back.len(), 2, "expansion invented rows: {back:?}");
    assert_eq!(sorted(back), sorted(gaps));
}

/// Two identical rows at one hour are two absences, not one.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: run-encoding hours through a set, or
/// extending a run on an hour equal to (rather than one past) its end.
#[test]
fn duplicate_rows_at_one_hour_survive_the_run_encoding() {
    let gaps = vec![coverage_row(5, 10, 0), coverage_row(5, 10, 0)];
    let ledger = encode_gap_ledger(&gaps, []);
    let (back, _) = expand(&ledger);
    assert_eq!(back.len(), 2);
    assert_eq!(sorted(back), sorted(gaps));
}

/// One record naming two domains and two records naming one each are different
/// claims -- one absence witnessed once, versus two. The grouping tag is what
/// keeps them apart.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping `grouping`, or folding a
/// multi-domain record into the per-domain path.
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
    let shared_ledger = encode_gap_ledger(&shared, []);
    let separate_ledger = encode_gap_ledger(&separate, []);
    assert_ne!(shared_ledger, separate_ledger);
    assert_eq!(sorted(expand(&shared_ledger).0), sorted(shared));
    assert_eq!(sorted(expand(&separate_ledger).0), sorted(separate));
}

/// Two shared records with the same domains at the same hour are two rows.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping `repeat`, or defaulting it
/// to 1 when absent.
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
    let ledger = encode_gap_ledger(&gaps, []);
    let (back, _) = expand(&ledger);
    assert_eq!(back.len(), 3);
    assert_eq!(sorted(back), sorted(gaps));
}

/// A tick that cannot be stated as an offset from its hour must be stated
/// absolutely rather than aborting the finalize.
///
/// `target_hour_raw = i32::MIN` with `tick_before = u64::MAX` is a delta of
/// 18,446,744,075,857,035,263: a valid `i128`, and neither an `i64` nor a `u64`,
/// so it is not a JSON number at all.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: computing the offset in `i64`, or
/// dropping the absolute spelling.
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
    let ledger = encode_gap_ledger(&gaps, outcomes.iter().copied());
    let text = serde_json::to_string(&ledger).expect("the ledger serializes");
    assert!(text.contains("\"abs\""), "no absolute spelling in {text}");
    let (back_gaps, back_outcomes) = expand(&ledger);
    assert_eq!(sorted(back_gaps), sorted(gaps));
    assert_eq!(back_outcomes, outcomes);
}

/// The widest legal record -- one covering every hour there is -- must survive.
/// Its inclusive width is 4,294,967,296, which is not an `i32`.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: computing `span_hours` in `i32`, or
/// expanding `hour_end` without a checked conversion.
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
    let ledger = encode_gap_ledger(&gaps, []);
    let (back, _) = expand(&ledger);
    assert_eq!(sorted(back), sorted(gaps));
}

// ---------------------------------------------------------------------------
// Reading a manifest
// ---------------------------------------------------------------------------

/// A version this build does not define is refused, not interpreted.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: accepting any `gap_ledger` object
/// with a `classes` array regardless of what it says it is.
#[test]
fn an_unknown_schema_or_version_is_refused_rather_than_guessed() {
    let mut ledger = encode_gap_ledger(&mixed_session(), []);
    ledger["version"] = json!(GAP_LEDGER_VERSION + 1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::UnknownVersion { .. })
    ));

    let mut renamed = encode_gap_ledger(&mixed_session(), []);
    renamed["schema"] = json!("some-other-ledger");
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": renamed })),
        Err(GapLedgerError::UnknownVersion { .. })
    ));
}

/// "No ledger" and "an empty ledger" are different claims. A manifest carrying
/// neither must not read as a capture with no gaps.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: defaulting a missing ledger to empty.
#[test]
fn a_manifest_with_no_ledger_at_all_is_an_error() {
    assert_eq!(
        read_compact_stream_gap_ledger(&json!({ "record_count": 5 })),
        Err(GapLedgerError::MissingLedger)
    );
}

/// The declared row count is checked against the expansion, so a truncated class
/// list cannot read as a shorter session.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: using `row_count` only to size the
/// output vector.
#[test]
fn the_declared_row_count_is_checked_against_the_expansion() {
    let gaps = mixed_session();
    let mut ledger = encode_gap_ledger(&gaps, []);
    let classes = ledger["gaps"]["classes"].as_array_mut().expect("classes");
    classes.pop();
    let remaining = classes.len();
    // `class_count` REPAIRED to the truncated list, so the refusal below can only
    // come from the row count and not from the class count noticing first. The
    // assertion names `RowCountMismatch` for the same reason.
    ledger["gaps"]["class_count"] = json!(remaining);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::RowCountMismatch { .. })
    ));
}

/// A run naming no hour is not "nothing happened here", it is a run that should
/// not have been written; admitting it would let two class lists mean one
/// expansion.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping the zero-count check.
#[test]
fn a_zero_length_hour_run_is_refused() {
    let mut ledger = encode_gap_ledger(&mixed_session(), []);
    ledger["gaps"]["classes"][0]["hour_runs"] = json!([[100, 0]]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

/// A class that names no hour expands to nothing, so any number of them would
/// mean exactly what the empty table means — two different documents with one
/// expansion.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: checking only that each RUN is
/// non-empty and not that the run LIST is.
#[test]
fn a_class_with_no_hour_runs_at_all_is_refused() {
    let mut ledger = encode_gap_ledger(&mixed_session(), mixed_outcomes());
    ledger["gaps"]["classes"][0]["hour_runs"] = json!([]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut outcomes_ledger = encode_gap_ledger(&[], mixed_outcomes());
    outcomes_ledger["domain_gap_outcomes"]["classes"][0]["hour_runs"] = json!([]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": outcomes_ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

/// A class that names no DOMAIN expands to nothing for exactly the same reason
/// an empty run list does, and is refused for the same reason: any number of
/// them would mean what the empty table means.
///
/// The one exception is a `shared` gap class, where an empty list is the
/// contract's "every domain" and names a real record.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping either check, or extending
/// the refusal to `shared`, which would refuse every whole-clock skipped hour.
#[test]
fn a_class_that_names_no_domain_is_refused_except_where_empty_means_all() {
    let mut gaps = encode_gap_ledger(&mixed_session(), []);
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

    let mut outcomes = encode_gap_ledger(&[], mixed_outcomes());
    outcomes["domain_gap_outcomes"]["classes"][0]["domains"] = json!([]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": outcomes })),
        Err(GapLedgerError::Malformed(_))
    ));

    // And the exception still reads: `mixed_session` carries a whole-clock
    // skipped hour, which is a `shared` class with an empty domain list.
    let clean = encode_gap_ledger(&mixed_session(), []);
    assert!(clean["gaps"]["classes"]
        .as_array()
        .expect("classes")
        .iter()
        .any(|class| class["grouping"] == json!("shared")
            && class["domains"] == json!([])));
    assert!(read_compact_stream_gap_ledger(&json!({ "gap_ledger": clean })).is_ok());
}

/// `[[101,1],[102,2]]` expands to exactly what `[[101,3]]` does, so admitting
/// both would let two documents mean one thing. And `class_count` is published
/// beside the class list, so a count that disagrees is a second claim about it.
///
/// The split leaves every hour and every row unchanged, so nothing but the
/// canonical-form check can refuse it.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: walking the runs without requiring
/// them ascending and maximal, or ignoring `class_count`.
#[test]
fn a_non_canonical_run_list_or_class_count_is_refused() {
    let mut split = encode_gap_ledger(&mixed_session(), []);
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

    let mut miscounted = encode_gap_ledger(&mixed_session(), []);
    miscounted["gaps"]["class_count"] = json!(999);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": miscounted })),
        Err(GapLedgerError::Malformed(_))
    ));
}

/// THE STRUCTURAL CLOSE on the "ignored field" family: a class, a witness, an
/// anchored tick and a lane entry each carry an EXACT field set.
///
/// A document carrying a field no reader uses expands to exactly what the
/// document without it does — and, worse, an unread field still counts toward a
/// class's IDENTITY, so adding one splits an aggregation key and walks straight
/// through the same-hour check.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: adding a field to the encoder without
/// adding it to the reader's allowed set. That is the point: the reader has to
/// be told, and a lane that forgets finds out here rather than on a capture.
#[test]
fn a_field_this_format_does_not_define_is_refused_everywhere() {
    let gaps = mixed_session();
    let outcomes = mixed_outcomes();

    let mut extra_gap_field = encode_gap_ledger(&gaps, []);
    extra_gap_field["gaps"]["classes"][0]["novel_field"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_gap_field })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut extra_witness_field = encode_gap_ledger(&gaps, []);
    extra_witness_field["gaps"]["classes"][0]["witness"]["novel"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_witness_field })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut extra_outcome_field = encode_gap_ledger(&[], outcomes.iter().copied());
    extra_outcome_field["domain_gap_outcomes"]["classes"][0]["repeat"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_outcome_field })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut extra_tick_field = encode_gap_ledger(&[], outcomes.iter().copied());
    extra_tick_field["domain_gap_outcomes"]["classes"][0]["tick_before"]["novel"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_tick_field })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut extra_lane_field = encode_gap_ledger(&[], outcomes.iter().copied());
    extra_lane_field["domain_gap_outcomes"]["lane_ordinal_first"][0]["novel"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": extra_lane_field })),
        Err(GapLedgerError::Malformed(_))
    ));
}

/// The TABLE is closed, not only its classes. Closing the classes but not their
/// container left the same escape one level up, and the container is exactly
/// where a future field would be added.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: adding a table-level field to the
/// encoder without adding it to the reader's allowed set.
#[test]
fn a_table_field_this_format_does_not_define_is_refused() {
    for table in ["gaps", "domain_gap_outcomes"] {
        let mut ledger = encode_gap_ledger(&mixed_session(), mixed_outcomes().iter().copied());
        ledger[table]["novel_table_field"] = json!(1);
        assert!(
            matches!(
                read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
                Err(GapLedgerError::Malformed(_))
            ),
            "a field on the {table} table must be refused"
        );
    }

    // AND THE LEDGER ITSELF, the last container.
    let mut ledger = encode_gap_ledger(&mixed_session(), mixed_outcomes().iter().copied());
    ledger["novel_ledger_field"] = json!(1);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

/// Allowlisting `note` BY NAME left an object-shaped hole: a name allowlist says
/// nothing about the value under it. The TYPE is required; the exact prose
/// deliberately is not, because the note is documentation, part of no class
/// identity, and cannot change a row -- while an object could carry structure.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: allowlisting a field's name without
/// saying what may be under it.
#[test]
fn the_ledger_note_must_be_a_string() {
    for bad in [json!({ "novel_nested_field": 1 }), json!([1, 2]), json!(7)] {
        let mut ledger = encode_gap_ledger(&mixed_session(), mixed_outcomes().iter().copied());
        ledger["note"] = bad;
        assert!(matches!(
            read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
            Err(GapLedgerError::Malformed(_))
        ));
    }
    // A DIFFERENT string is still fine: the prose is not load-bearing.
    let mut ledger = encode_gap_ledger(&mixed_session(), mixed_outcomes().iter().copied());
    ledger["note"] = json!("anything at all");
    assert!(read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })).is_ok());
}

/// A one-domain record is always `per_domain` on the encoder's side, so a
/// `shared` class naming exactly one domain is a second spelling of the same
/// rows. And the work bound counts the ROWS a class emits, not the hours it
/// names: a shared class multiplies every hour by `repeat`, so an hour bound
/// alone admitted `u64::MAX` and the loop under it.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: letting `shared` take any domain
/// list, or bounding hours instead of rows.
#[test]
fn a_singleton_shared_class_and_an_unbounded_repeat_are_both_refused() {
    let mut singleton = encode_gap_ledger(&mixed_session(), []);
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

/// A run that walks off the top of the hour space is refused at the decoder
/// rather than wrapping into hours that never existed.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: expanding a run with `as i32`.
#[test]
fn a_run_that_walks_past_the_last_hour_is_refused() {
    let mut ledger = encode_gap_ledger(&mixed_session(), []);
    ledger["gaps"]["classes"][0]["hour_runs"] = json!([[i32::MAX, 4u32]]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::OutOfRange(_))
    ));
}

/// A decoded offset is an ARBITRARY number, not one this encoder produced, and
/// the arithmetic that resolves it has to survive the extremes before the
/// checked narrowing can reject them.
///
/// `span_hours = i64::MAX` with a run at `i32::MAX` overflowed `i64::from(hour)
/// + span` — a panic under overflow checks, a wrapped hour without them, and a
/// wrapped hour is the ledger inventing an hour, which is the one outcome it
/// exists to prevent. Same for a witness offset of `i64::MAX`.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: doing the offset arithmetic in `i64`
/// instead of `i128`.
#[test]
fn extreme_decoded_offsets_are_refused_rather_than_wrapped() {
    let mut ledger = encode_gap_ledger(&mixed_session(), []);
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
    let mut witness_ledger = encode_gap_ledger(&gaps, []);
    witness_ledger["gaps"]["classes"][0]["witness"]["next_hour_rel"] = json!(i64::MAX);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": witness_ledger })),
        Err(GapLedgerError::OutOfRange(_))
    ));
}

/// An anchored field carrying both spellings, or neither, has no single meaning.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: preferring one spelling when both are
/// present.
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
    let mut ledger = encode_gap_ledger(&gaps, []);
    ledger["gaps"]["classes"][0]["witness"]["tick_start"] = json!({ "rel": 0, "abs": 5 });
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": ledger })),
        Err(GapLedgerError::Malformed(_))
    ));
}

// ---------------------------------------------------------------------------
// Ordinals
// ---------------------------------------------------------------------------

/// The normal case: ordinals are derived from hour order and the ledger says so.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: assigning derived ordinals in an
/// order the encoder did not verify.
#[test]
fn lane_sequential_ordinals_are_derived_and_correct() {
    let outcomes = mixed_outcomes();
    let ledger = encode_gap_ledger(&[], outcomes.iter().copied());
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

/// Ordinals that hour order cannot explain are written out instead of derived.
/// Here one lane's ordinals are shuffled relative to its hours, so the
/// derivation is false and the encoder must notice.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: deriving ordinals without checking
/// the derivation against the rows -- which would publish a different ordinal
/// for a row than the producer emitted.
#[test]
fn ordinals_that_hour_order_cannot_explain_are_written_out() {
    let outcomes = vec![
        outcome(10, 100, 7, DomainGapReason::Backpressure),
        outcome(10, 101, 3, DomainGapReason::Backpressure),
        outcome(10, 102, 91, DomainGapReason::Backpressure),
    ];
    let ledger = encode_gap_ledger(&[], outcomes.iter().copied());
    assert_eq!(
        ledger["domain_gap_outcomes"]["ordinal_encoding"],
        json!("explicit")
    );
    let (_, back) = expand(&ledger);
    let mut got = back;
    got.sort_by_key(|(domain, record)| (record.target_hour_raw, *domain));
    assert_eq!(got, outcomes);
}

/// Two outcomes in one lane at one hour make hour order ambiguous, so the
/// derivation must not be used.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: deriving ordinals from a sort with
/// ties, where which row gets which ordinal is whatever the sort happened to do.
#[test]
fn two_outcomes_in_one_lane_at_one_hour_force_explicit_ordinals() {
    let outcomes = vec![
        outcome(10, 100, 0, DomainGapReason::Backpressure),
        outcome(10, 100, 1, DomainGapReason::Torn),
    ];
    let ledger = encode_gap_ledger(&[], outcomes.iter().copied());
    assert_eq!(
        ledger["domain_gap_outcomes"]["ordinal_encoding"],
        json!("explicit")
    );
    let (_, back) = expand(&ledger);
    let mut got = back;
    got.sort_by_key(|(_, record)| record.outcome_ordinal);
    assert_eq!(got, outcomes);
}

/// A lane entry that no class expands to, and a lane named twice, both leave the
/// expansion unchanged — so a table carrying either means exactly what the table
/// without it means. Same non-injectivity as a class that names no row.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: loading the lane table into a map and
/// only checking that every EXPANDED lane has a base, which is what it did.
#[test]
fn an_unused_or_duplicated_lane_entry_is_refused() {
    let outcomes = mixed_outcomes();

    let mut unused = encode_gap_ledger(&[], outcomes.iter().copied());
    unused["domain_gap_outcomes"]["lane_ordinal_first"]
        .as_array_mut()
        .expect("lane table")
        .push(json!({ "domain_id": 999, "part": "both", "first": 0 }));
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": unused })),
        Err(GapLedgerError::Malformed(_))
    ));

    let mut duplicated = encode_gap_ledger(&[], outcomes.iter().copied());
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

/// The two ordinal encodings must not carry each other's fields. A
/// `lane_sequential` table IGNORES a class's `ordinals` and an `explicit` one
/// IGNORES `lane_ordinal_first`, so a document carrying the ignored field means
/// exactly what the document without it means — two documents, one expansion.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: reading the field the branch uses and
/// saying nothing about the one it does not.
#[test]
fn the_two_ordinal_encodings_do_not_carry_each_others_fields() {
    let outcomes = mixed_outcomes();

    let mut sequential = encode_gap_ledger(&[], outcomes.iter().copied());
    assert_eq!(
        sequential["domain_gap_outcomes"]["ordinal_encoding"],
        json!("lane_sequential")
    );
    sequential["domain_gap_outcomes"]["classes"][0]["ordinals"] = json!([999]);
    assert!(matches!(
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": sequential })),
        Err(GapLedgerError::Malformed(_))
    ));

    // An `explicit` table, built from rows whose ordinals hour order cannot
    // explain, must not carry a lane table either.
    let ambiguous = vec![
        outcome(10, 100, 0, DomainGapReason::Backpressure),
        outcome(10, 100, 1, DomainGapReason::Torn),
    ];
    let mut explicit = encode_gap_ledger(&[], ambiguous.iter().copied());
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

/// A ledger claiming derived ordinals over ambiguous rows is refused, because
/// the reader would otherwise be guessing which row the producer numbered.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping the decoder's own ambiguity
/// check and trusting the encoder's word.
#[test]
fn a_lane_sequential_ledger_over_ambiguous_rows_is_refused() {
    let outcomes = vec![
        outcome(10, 100, 0, DomainGapReason::Backpressure),
        outcome(10, 100, 1, DomainGapReason::Torn),
    ];
    let mut ledger = encode_gap_ledger(&[], outcomes.iter().copied());
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

// ---------------------------------------------------------------------------
// The size claim
// ---------------------------------------------------------------------------

/// The point of the whole exercise, asserted rather than asserted-about.
///
/// The session below is the shape a real capture has -- a stable set of domains
/// failing the same way hour after hour -- and it is where the flat shape costs
/// its 33.5 KB per game hour. The bound is deliberately loose (10x) so that
/// ordinary encoder changes do not trip it; the measured ratio on the real t44c
/// capture is ~400x.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: reintroducing a per-row object --
/// per-row `h4gp_hex`, per-row hours, an explicit ordinal fallback that is
/// always taken.
#[test]
fn the_compact_shape_is_far_smaller_than_the_flat_one_on_a_repetitive_session() {
    let mut gaps = Vec::new();
    let mut outcomes = Vec::new();
    let mut next: BTreeMap<CompactDomainId, u64> = BTreeMap::new();
    for hour in 0..2_000 {
        for domain in 1u16..=30 {
            gaps.push(coverage_row(hour, domain, 0));
            let ordinal = next.entry(domain).or_insert(0);
            outcomes.push(outcome(domain, hour, *ordinal, DomainGapReason::ReaderSilent));
            *ordinal += 1;
        }
    }
    let flat = serde_json::to_vec(&json!({
        "gaps": reference_flat_gaps_json(&gaps),
        "domain_gap_outcomes": reference_flat_domain_gap_outcomes_json(&outcomes),
    }))
    .expect("flat serializes")
    .len();
    let compact = serde_json::to_vec(&encode_gap_ledger(&gaps, outcomes.iter().copied()))
        .expect("compact serializes")
        .len();
    assert!(
        compact * 10 < flat,
        "compact {compact} B is not far below flat {flat} B"
    );
    // And it still says everything the flat shape did.
    let (back_gaps, back_outcomes) =
        read_compact_stream_gap_ledger(&json!({ "gap_ledger": encode_gap_ledger(&gaps, outcomes.iter().copied()) }))
            .expect("expands");
    assert_eq!(fact_set(&back_gaps), fact_set(&gaps));
    assert_eq!(back_outcomes.len(), outcomes.len());
}

// ---------------------------------------------------------------------------
// Cross-language conformance
// ---------------------------------------------------------------------------

/// Where the vector the Python expander is tested against lives.
const CONFORMANCE_VECTOR: &str = "../scripts/gap_ledger_conformance.json";

/// Rows in one total order both languages can produce, so the vector compares
/// content rather than whichever order an encoder happened to emit.
fn canonically_ordered(mut rows: Vec<Value>) -> Vec<Value> {
    rows.sort_by_key(|row| serde_json::to_string(row).unwrap_or_default());
    rows
}

/// The manifest is read by Rust AND by Python (`scripts/gap_ledger.py`, which
/// the capture scorecard and the sink verifier both go through). Two expanders
/// written from one description agree until the first time either learns
/// something the other did not, and the failure mode is a capture graded on
/// hours nobody recorded.
///
/// So there is ONE vector, checked in, and both sides are held to it: this test
/// owns its content, and `scripts/test_gap_ledger.py` expands the same object
/// and must reach the same rows. Regenerate deliberately with
/// `FV_WRITE_GAP_LEDGER_VECTOR=1`, never by editing the file.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: any change to the encoded shape --
/// which is the point, because that same change has to reach the Python
/// expander before the vector may be regenerated.
#[test]
fn the_cross_language_conformance_vector_matches_this_encoder() {
    let gaps = mixed_session();
    let outcomes = mixed_outcomes();
    // `h4gp_hex` is deliberately absent from the expected rows: it is a pure
    // function of the record that only the binary codec consumes, and the
    // compacted ledger does not carry it for exactly that reason.
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
        "gap_ledger": encode_gap_ledger(&gaps, outcomes.iter().copied()),
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

/// The ledger's own field lookup takes the last dotted segment of a path, which
/// is only sound while no key contains a dot.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: adding a key like `queue.depth`,
/// which the reader would then look for under `depth`.
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
        &encode_gap_ledger(&mixed_session(), mixed_outcomes()),
        "gap_ledger",
    );
}

// ---------------------------------------------------------------------------
// The typed class table (lane GE)
// ---------------------------------------------------------------------------

/// The two builders answer the same thing.
///
/// Production streams its derivation into `GapClassTableBuilder` in hour order
/// and keeps only the current hour's tallies; `from_records` buffers a list and
/// sorts it. If the streaming path ever grouped differently -- an hour closed
/// too early, a tally flushed into the wrong class -- the two would disagree on
/// the classes, and this compares the whole published table, not a count.
#[test]
fn the_ordered_builder_and_from_records_agree() {
    let mut records = mixed_session();
    // Deliberately not in hour order, so `from_records` has to do its own
    // sorting and the comparison is not against an already-sorted input.
    records.reverse();

    let unordered = GapClassTable::from_records(&records);

    let mut hour_ordered = records.clone();
    hour_ordered.sort_by_key(|record| record.hour_start);
    let mut builder = GapClassTableBuilder::default();
    for record in &hour_ordered {
        builder.push(record).expect("hour ordered input is accepted");
    }
    let streamed = builder.finish();

    assert_eq!(streamed.to_json(), unordered.to_json());
    assert_eq!(streamed.row_count(), records.len());
    assert_eq!(sorted(streamed.rows().collect()), sorted(records));
}

/// A record for an hour the builder has already closed is REFUSED.
///
/// The builder's whole saving is that it holds one hour of tallies; folding a
/// late record into a closed run would move a row to an hour nobody recorded it
/// at, which is the hour misattribution this ledger exists to make impossible.
/// Make `push` re-open the previous hour instead and this fails.
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

/// The point query has to see the INTERIOR of a multi-hour record.
///
/// `hour_runs` holds the hours records START at, not every hour they cover, so
/// a lookup that binary searched for the queried hour directly would answer
/// "no gap here" in the middle of a nine-hour skipped range -- and the coverage
/// gate would then raise a second record for an hour that was already
/// explained. The empty-domain row below covers every domain, the backpressure
/// row only its own.
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

/// `any_covering` must find exactly what a scan of the flat rows would.
///
/// The class-level filters are an INDEX; an index that disagrees with the rows
/// is a wrong answer, so this compares the two over every (hour, domain) the
/// fixture touches.
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

/// The hour-ordered merge yields every row, once, hours ascending.
///
/// The bounds-and-overlap sweep on the finalize path depends on both halves: a
/// row out of order makes the sweep refuse the ledger, and a row dropped makes
/// it miss an overlap.
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

/// Duplicate rows are visible on the classes, exactly.
///
/// `derived_gap_ledger_is_valid` used to hold one encoded copy of every row to
/// answer this. The claim that replaced it is that a duplicate can only appear
/// as a repeated domain in one class-hour tally or as a `repeat` above one, so
/// both spellings are checked here -- and a clean session is checked with them,
/// which a `has_duplicate_rows` that always answered `true` would fail.
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

/// The class table costs less than the rows it stands for, and the saving grows
/// with the session.
///
/// This is lane GE's whole claim, stated where it can fail: a table that kept
/// one entry per row would hold the ratio at one.
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
