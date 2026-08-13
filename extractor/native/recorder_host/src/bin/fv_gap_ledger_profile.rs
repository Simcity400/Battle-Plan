//! Price the manifest gap ledger at a synthetic session length, one shape per run.
//!
//! # Why a separate process per arm
//!
//! [`recorder_host::mem_account`] keeps its counters in process statics and its
//! peaks never fall, which is the whole point of a peak. Running both the flat
//! and the compact arm in one process would therefore publish `max(flat,
//! compact)` for both, i.e. the flat number twice. Each invocation measures
//! exactly ONE arm and prints the registry's own report, so the two numbers come
//! from the same instrument and are comparable.
//!
//! # What the synthetic session models
//!
//! The shape a real capture has, taken from `runs/live_unlock/t44c_shadow`
//! (683 game hours, 40,139 gap rows, 39,637 outcome rows): a stable set of
//! domains failing coverage every hour, a second set alternating with the
//! scheduler, occasional whole-clock skipped hours, and one H4DG outcome per
//! affected domain-hour. It is NOT the t44c stream — it is a generator with the
//! same per-hour repetition, so the session length can be turned up past any
//! capture that exists.
//!
//! ```text
//! cargo run --release -p recorder_host --bin fv_gap_ledger_profile -- <hours> flat|compact
//! ```

use std::collections::BTreeMap;

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use compact_contract::{GapCounter, GapReason, GapRecord, GapWitness};
use recorder_host::gap_ledger;
use recorder_host::mem_account::{self, Charge, Component};

/// Domains that shortfall every hour, as ~35 of 68 do on t44c.
const ALWAYS: &[u16] = &[
    7, 9, 11, 13, 17, 19, 21, 23, 27, 29, 31, 33, 37, 39, 41, 43, 47, 49, 51, 53,
];
/// Domains that come and go with the scheduler.
const ALTERNATING: &[u16] = &[54, 55, 56, 57, 58, 59, 60, 61, 62, 63];

fn synthetic_session(hours: i32) -> (Vec<GapRecord>, Vec<gap_ledger::DomainOutcome>) {
    let base = 60_832_235i32;
    let mut gaps = Vec::new();
    let mut outcomes = Vec::new();
    let mut next_ordinal: BTreeMap<u16, u64> = BTreeMap::new();
    for step in 0..hours {
        let hour = base + step;
        for domain in ALWAYS.iter().copied() {
            gaps.push(GapRecord {
                hour_start: hour,
                hour_end: hour,
                reason: GapReason::CoverageShortfall,
                domains: vec![domain],
                witness: GapWitness::Coverage {
                    expected_perf_records: 1,
                    observed_perf_records: 0,
                    expected_data_records: 1,
                    observed_data_records: 0,
                },
            });
            let ordinal = next_ordinal.entry(domain).or_insert(0);
            outcomes.push((
                domain,
                DomainGapRecordV1 {
                    part: DomainGapPart::Both,
                    reason: DomainGapReason::HourLabelMismatch,
                    target_hour_raw: hour,
                    range_start_hour_raw: hour,
                    range_end_hour_raw: hour,
                    tick_before: u64::try_from(hour).unwrap_or(0) + 1,
                    tick_after: u64::try_from(hour).unwrap_or(0) + 1,
                    queue_capacity: 0,
                    queue_depth: 0,
                    queue_dropped: 0,
                    outcome_ordinal: *ordinal,
                },
            ));
            *ordinal += 1;
        }
        if step % 2 == 0 {
            for domain in ALTERNATING.iter().copied() {
                gaps.push(GapRecord {
                    hour_start: hour,
                    hour_end: hour,
                    reason: GapReason::Backpressure,
                    domains: vec![domain],
                    witness: GapWitness::Counter {
                        counter: GapCounter::SinkBackpressureHours,
                        observed: 1,
                    },
                });
                let ordinal = next_ordinal.entry(domain).or_insert(0);
                outcomes.push((
                    domain,
                    DomainGapRecordV1 {
                        part: DomainGapPart::Both,
                        reason: DomainGapReason::Backpressure,
                        target_hour_raw: hour,
                        range_start_hour_raw: hour,
                        range_end_hour_raw: hour,
                        tick_before: u64::try_from(hour).unwrap_or(0),
                        tick_after: u64::try_from(hour).unwrap_or(0),
                        queue_capacity: 1,
                        queue_depth: 1,
                        queue_dropped: 1,
                        outcome_ordinal: *ordinal,
                    },
                ));
                *ordinal += 1;
            }
        }
        // A whole-clock skipped hour every ~26 hours, as t44c carries 26 over 683.
        if step % 26 == 25 {
            gaps.push(GapRecord {
                hour_start: hour,
                hour_end: hour,
                reason: GapReason::SkippedHour,
                domains: Vec::new(),
                witness: GapWitness::HourDiscontinuity {
                    previous_hour: hour - 1,
                    next_hour: hour + 1,
                },
            });
        }
    }
    (gaps, outcomes)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let hours: i32 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(683);
    let arm = args.next().unwrap_or_else(|| "compact".to_string());

    mem_account::mark("profile.before_build");
    let (gaps, outcomes) = synthetic_session(hours);
    mem_account::mark("profile.session_built");

    // Charged exactly as production charges it: the rows plus the EXACT
    // serialized length of the JSON they become. The flat arm is priced by the
    // same function so the two numbers mean the same thing.
    let (json_bytes, class_counts) = match arm.as_str() {
        "flat" => {
            let flat = serde_json::json!({
                "gaps": gap_ledger::reference_flat_gaps_json(&gaps),
                "domain_gap_outcomes": gap_ledger::reference_flat_domain_gap_outcomes_json(&outcomes),
            });
            // The flat arm's rows ARE the flat list, so it is priced by the
            // flat form of the same function (lane GE moved the production
            // pricing onto the class table, which this arm does not build).
            let bytes = gap_ledger::flat_ledger_manifest_bytes(&flat, &gaps, outcomes.len());
            let charge = Charge::new(Component::ManifestGapLedger, bytes);
            let serialized = serde_json::to_vec(&flat).map(|v| v.len()).unwrap_or(0);
            mem_account::mark("profile.ledger_built");
            drop(charge);
            (serialized, (0usize, 0usize))
        }
        "compact" => {
            // Through the class table, as production now does it: what the
            // compact arm holds is the classes, not the rows.
            let table = gap_ledger::GapClassTable::from_records(&gaps);
            let ledger =
                gap_ledger::encode_gap_ledger_from_table(&table, &outcomes[..]);
            // The rows are charged too: THIS BIN still holds them (they are the
            // synthetic session it generated), even though production no longer
            // does. Charging only the table would publish a compact arm that is
            // smaller than the process it ran in.
            // The LARGER of two moments, never their sum: while the table is
            // being built this arm holds the rows plus `from_records`' sort
            // index plus the growing classes, and once it is built it holds the
            // rows plus the classes plus the ledger JSON plus the outcomes. The
            // sort index is gone by the time the second moment exists, so adding
            // it there would publish a peak no instant reached. Production
            // builds neither — it streams into the class builder.
            let rows_bytes = gap_ledger::gap_records_resident_bytes(&gaps);
            let building = rows_bytes
                + table.resident_bytes()
                + gap_ledger::GapClassTable::from_records_transient_bytes(gaps.len());
            let built = rows_bytes
                + table.resident_bytes()
                + gap_ledger::ledger_manifest_bytes(&ledger, outcomes.len());
            let bytes = building.max(built);
            let charge = Charge::new(Component::ManifestGapLedger, bytes);
            let serialized = serde_json::to_vec(&ledger).map(|v| v.len()).unwrap_or(0);
            let classes = (
                ledger["gaps"]["classes"].as_array().map_or(0, Vec::len),
                ledger["domain_gap_outcomes"]["classes"]
                    .as_array()
                    .map_or(0, Vec::len),
            );
            mem_account::mark("profile.ledger_built");
            // Proven, not asserted: the arm that publishes a smaller number also
            // has to give every row back. A size measurement of a lossy encoder
            // would be worthless.
            let (back_gaps, back_outcomes) = gap_ledger::read_compact_stream_gap_ledger(
                &serde_json::json!({ "gap_ledger": ledger }),
            )
            .expect("the ledger this arm measured must expand");
            assert_eq!(back_gaps.len(), gaps.len(), "gap rows lost");
            assert_eq!(back_outcomes.len(), outcomes.len(), "outcome rows lost");
            drop(charge);
            (serialized, classes)
        }
        other => {
            eprintln!("unknown arm {other:?}; expected `flat` or `compact`");
            std::process::exit(2);
        }
    };

    let report = serde_json::json!({
        "arm": arm,
        "game_hours": hours,
        "gap_rows": gaps.len(),
        "outcome_rows": outcomes.len(),
        "gap_classes": class_counts.0,
        "outcome_classes": class_counts.1,
        "manifest_json_bytes": json_bytes,
        // The copy that no longer happens. Production used to collect the
        // summary's outcomes into a second `Vec` before encoding; the encoder
        // now streams them. Those bytes were never charged to any component --
        // they landed in `unaccounted_bytes` -- so this is the only place the
        // saving is visible, as the size of the allocation that is gone.
        "outcome_row_bytes": std::mem::size_of::<gap_ledger::DomainOutcome>(),
        "removed_outcome_copy_bytes":
            std::mem::size_of::<gap_ledger::DomainOutcome>() * outcomes.len(),
        "manifest_json_bytes_per_game_hour": if hours > 0 {
            json_bytes as f64 / f64::from(hours)
        } else {
            0.0
        },
        "memory_account": mem_account::report_json(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("report serializes")
    );
}
