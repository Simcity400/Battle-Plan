//!   EACH FOR THE LIFETIME IT HAD: the derivation's two `(hour, domain)` sets,
//!   cargo test --release -p recorder_host --bin collector_record \

use std::collections::{BTreeMap, BTreeSet};

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use recorder_host::gap_ledger;
use recorder_host::mem_account::{self, Charge, Component};
use serde_json::json;

use super::*;

const ALWAYS: &[u16] = &[
    7, 9, 11, 13, 17, 19, 21, 23, 27, 29, 31, 33, 37, 39, 41, 43, 47, 49, 51, 53,
];
const ALTERNATING: &[u16] = &[54, 55, 56, 57, 58, 59, 60, 61, 62, 63];
const OUTCOME: &[u16] = &[64, 65, 66, 67, 68, 69, 70, 71, 72, 73];
const HEALTHY: &[u16] = &[81, 82];

fn synthetic_summary(hours: i32) -> (CompactSummary, Vec<u16>) {
    let base = 60_832_235i32;
    let mut summary = CompactSummary::default();
    let mut expected: Vec<u16> = ALWAYS
        .iter()
        .chain(ALTERNATING)
        .chain(OUTCOME)
        .chain(HEALTHY)
        .copied()
        .collect();
    expected.sort_unstable();

    let mut next_ordinal: BTreeMap<u16, u64> = BTreeMap::new();
    let mut unique_hours: Vec<i32> = Vec::new();
    for step in 0..hours {
        if step % 26 == 25 {
            continue;
        }
        let hour = base + step;
        unique_hours.push(hour);
        summary.frame_end_hour_counts.insert(hour, 1);

        let perf = summary.per_hour_perf_domain_counts.entry(hour).or_default();
        for domain in HEALTHY {
            perf.insert(*domain, 1);
        }
        if step % 2 == 1 {
            for domain in ALTERNATING {
                perf.insert(*domain, 1);
            }
        }
        let data = summary.per_hour_data_domain_counts.entry(hour).or_default();
        for domain in HEALTHY {
            if compact_domain_requires_data_record(*domain) {
                data.insert(*domain, 1);
            }
        }
        if step % 2 == 1 {
            for domain in ALTERNATING {
                if compact_domain_requires_data_record(*domain) {
                    data.insert(*domain, 1);
                }
            }
        }
        summary
            .per_hour_domain_status_flags
            .entry(hour)
            .or_default()
            .insert(HEALTHY[0], COMPACT_STATUS_PARTIAL_DATA);

        for domain in OUTCOME {
            let ordinal = next_ordinal.entry(*domain).or_insert(0);
            summary.domain_gaps.push(ObservedDomainGap {
                affected_domain_id: *domain,
                header_hour_raw: hour,
                record: DomainGapRecordV1 {
                    part: DomainGapPart::Both,
                    reason: DomainGapReason::ReaderSilent,
                    target_hour_raw: hour,
                    range_start_hour_raw: hour,
                    range_end_hour_raw: hour,
                    tick_before: 0,
                    tick_after: 0,
                    queue_capacity: 0,
                    queue_depth: 0,
                    queue_dropped: 0,
                    outcome_ordinal: *ordinal,
                },
            });
            *ordinal += 1;
        }
    }
    summary.frame_end_record_count = unique_hours.len();
    summary.skipped_hours = skipped_hours(&unique_hours);
    summary.first_hour = unique_hours.first().copied();
    summary.last_hour = unique_hours.last().copied();
    summary.unique_hours = unique_hours;
    (summary, expected)
}

/// # Three lifetimes, not one sum
struct MaterializedExpansion {
    _encoded_seen: BTreeSet<Vec<u8>>,
    _projection_counts: BTreeMap<Vec<u8>, usize>,
    _bounded: Vec<GapRecord>,
    bytes: u64,
    _charge: Charge,
}

struct MaterializedRows {
    rows: Vec<GapRecord>,
    peak_bytes: std::cell::Cell<u64>,
    _charge: Charge,
}

fn encoded_key_bytes<'a>(keys: impl Iterator<Item = &'a Vec<u8>>) -> u64 {
    keys.map(|encoded| (encoded.len() + std::mem::size_of::<Vec<u8>>()) as u64)
        .sum()
}

impl MaterializedRows {
    fn derive(summary: &CompactSummary, expected: &[u16]) -> Self {
        let pair = std::mem::size_of::<(i32, u16)>() as u64;
        let (rows, derivation_bytes) = {
            let reference = reference_derive_gap_records(summary, expected);
            let bytes = gap_ledger::gap_records_resident_bytes(&reference.rows)
                + (reference.h4dg_accounted.len() as u64) * pair
                + (reference.coverage_shortfalls.len() as u64) * pair;
            let _phase = Charge::new(Component::ManifestGapLedger, bytes);
            mem_account::mark("profile.materialized_derivation");
            (reference.rows, bytes)
        };
        let row_bytes = gap_ledger::gap_records_resident_bytes(&rows);
        Self {
            rows,
            peak_bytes: std::cell::Cell::new(derivation_bytes.max(row_bytes)),
            _charge: Charge::new(Component::ManifestGapLedger, row_bytes),
        }
    }

    fn validation_phase(
        &self,
        summary: &CompactSummary,
        expected: &[u16],
    ) -> MaterializedExpansion {
        let mut encoded_seen: BTreeSet<Vec<u8>> = BTreeSet::new();
        let mut bounded: Vec<GapRecord> = Vec::new();
        let mut projection_counts: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        for observed in &summary.domain_gaps {
            if !expected.contains(&observed.affected_domain_id) {
                continue;
            }
            if let Ok(encoded) =
                compact_contract::encode_gap_record(&project_observed_domain_gap(observed))
            {
                *projection_counts.entry(encoded).or_default() += 1;
            }
        }
        let projection_bytes = encoded_key_bytes(projection_counts.keys());
        for record in &self.rows {
            if let Ok(encoded) = compact_contract::encode_gap_record(record) {
                match projection_counts
                    .get_mut(&encoded)
                    .filter(|left| **left != 0)
                {
                    Some(left) => *left -= 1,
                    None => bounded.push(record.clone()),
                }
                encoded_seen.insert(encoded);
            }
        }
        let bytes = gap_ledger::gap_records_resident_bytes(&self.rows)
            + gap_ledger::gap_records_resident_bytes(&bounded)
            + encoded_key_bytes(encoded_seen.iter())
            + projection_bytes;
        self.peak_bytes.set(self.peak_bytes.get().max(bytes));
        mem_account::mark("profile.materialized_validation");
        MaterializedExpansion {
            _encoded_seen: encoded_seen,
            _projection_counts: projection_counts,
            _bounded: bounded,
            bytes,
            _charge: Charge::new(
                Component::ManifestGapLedger,
                bytes - gap_ledger::gap_records_resident_bytes(&self.rows),
            ),
        }
    }

    fn charged_bytes(&self) -> u64 {
        self.peak_bytes.get()
    }
}

fn run_arm(hours: i32, arm: &str) -> serde_json::Value {
    let (summary, expected) = synthetic_summary(hours);
    mem_account::mark("profile.summary_built");
    mem_account::reset_peak_live_heap();
    let live_before = mem_account::live_heap_bytes();

    let materialized = if arm == "materialized" {
        Some(MaterializedRows::derive(&summary, &expected))
    } else {
        None
    };

    let ledger = DerivedGapLedger::derive(&summary, &expected);
    let class_bytes = ledger.all.resident_bytes() + ledger.host_derived.resident_bytes();
    let row_count = ledger.all.row_count();
    let class_count = ledger.all.class_count();
    let hour_run_count = ledger.all.hour_run_count();

    let coverage_errors = compact_domain_coverage_errors_with_gaps(&summary, &expected, &ledger);
    let hourly_problems = {
        let _validation = materialized
            .as_ref()
            .map(|rows| rows.validation_phase(&summary, &expected));
        compact_hourly_problems_with_gap_records(&summary, 0, &coverage_errors, &expected, &ledger)
    };
    let telemetry_problems =
        acceptance_telemetry_problems_for_run(&summary, &expected, None, &ledger);
    let manifest = compact_stream_manifest_json_for_run(
        &summary,
        &expected,
        &coverage_errors,
        &[],
        None,
        &ledger,
    );
    let manifest_json_bytes = serde_json::to_vec(&manifest["gap_ledger"])
        .map(|bytes| bytes.len())
        .unwrap_or(0);

    mem_account::mark("profile.validated");
    let peak_live = mem_account::peak_live_heap_bytes();
    let report = json!({
        "arm": arm,
        "game_hours": hours,
        "observed_hours": summary.unique_hours.len(),
        "gap_rows": row_count,
        "gap_classes": class_count,
        "gap_hour_runs": hour_run_count,
        "outcome_rows": summary.domain_gaps.len(),
        "gap_ledger_json_bytes": manifest_json_bytes,
        "class_table_bytes": class_bytes,
        "materialized_expansion_bytes": materialized
            .as_ref()
            .map(MaterializedRows::charged_bytes),
        "manifest_gap_record_count": manifest["gap_record_count"],
        "coverage_errors": coverage_errors.len(),
        "hourly_problems": hourly_problems,
        "telemetry_problems": telemetry_problems.len(),
        "live_heap_bytes_before_phase": live_before,
        "peak_live_heap_bytes_absolute": peak_live,
        "phase_peak_above_input_bytes": match (peak_live, live_before) {
            (Some(peak), Some(before)) => Some(peak.saturating_sub(before)),
            _ => None,
        },
        "memory_account": mem_account::report_json(),
    });
    drop(materialized);
    drop(ledger);
    report
}

#[test]
fn ge_gap_stream_profile() {
    let hours: i32 = std::env::var("GE_GAP_STREAM_HOURS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if hours > 0 {
        let arm = std::env::var("GE_GAP_STREAM_ARM").unwrap_or_else(|_| "streamed".to_string());
        assert!(
            arm == "streamed" || arm == "materialized",
            "GE_GAP_STREAM_ARM must be `streamed` or `materialized`, not {arm:?}"
        );
        let report = run_arm(hours, &arm);
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("the report serializes")
        );
        return;
    }

    let (summary, expected) = synthetic_summary(200);
    let rows = reference_derive_gap_records(&summary, &expected).rows;
    let ledger = DerivedGapLedger::derive(&summary, &expected);
    assert!(ledger.build_errors.is_empty(), "{:?}", ledger.build_errors);
    assert_eq!(
        ledger.all.row_count(),
        rows.len(),
        "the class table must stand for exactly the rows the derivation produced"
    );
    let mut from_table: Vec<GapRecord> = ledger.all.rows().collect();
    let mut from_walk = rows.clone();
    let key = |record: &GapRecord| {
        (
            record.hour_start,
            record.hour_end,
            record.reason,
            record.domains.clone(),
            format!("{:?}", record.witness),
        )
    };
    from_table.sort_by_key(key);
    from_walk.sort_by_key(key);
    assert_eq!(
        from_table, from_walk,
        "expanding the classes must give back the same multiset of rows"
    );
    assert!(
        ledger.all.resident_bytes() < gap_ledger::gap_records_resident_bytes(&rows),
        "the class table ({} B) must be smaller than the rows it stands for ({} B)",
        ledger.all.resident_bytes(),
        gap_ledger::gap_records_resident_bytes(&rows)
    );
}
