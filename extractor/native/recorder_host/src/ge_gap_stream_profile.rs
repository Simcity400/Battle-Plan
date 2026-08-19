//! Price the WHOLE derived-gap path at a synthetic session length, one shape
//! per process.
//!
//! # What this measures, and why it lives here
//!
//! The derivation and its validators live in the recorder binary, so the
//! measurement stays beside them as a `#[test]` instead of a separate tool.
//!
//! # The two arms
//!
//! * `streamed` — the production path: the derivation is streamed into
//!   `gap_ledger::GapClassTableBuilder`, the flat row list never exists, and the
//!   validators read the class tables.
//! * `materialized` — the same run, plus the structures the pre-GE code held,
//!   EACH FOR THE LIFETIME IT HAD: the derivation's two `(hour, domain)` sets,
//!   which died when it returned; the flat `Vec<GapRecord>`, which outlived
//!   every validator and the manifest; and `derived_gap_ledger_is_valid`'s
//!   `BTreeSet<Vec<u8>>` of encoded rows, its projection multiset and its cloned
//!   bounded rows, which died when it returned. What that arm publishes is the
//!   LARGEST of those three moments, never their sum.
//!
//! The `materialized` arm is a RECONSTRUCTION of what was removed, in this
//! process, not a run of the previous binary. Its rows come from
//! `reference_derive_gap_records`, which is that walk kept verbatim, and the
//! structures are written out here rather than remembered so that what the
//! "before" number prices is readable.
//!
//! # Why one process per arm
//!
//! `mem_account`'s peaks are process statics and never fall, so two arms in one
//! process would publish the larger twice. The live-heap peak is reset at the
//! phase boundary (`mem_account::reset_peak_live_heap`, measurement builds only)
//! so that what it reports is the phase's own high-water above the summary it
//! was handed, rather than the cost of building that summary.
//!
//! ```text
//! GE_GAP_STREAM_HOURS=68300 GE_GAP_STREAM_ARM=streamed \
//!   cargo test --release -p recorder_host --bin collector_record \
//!   --features mem-alloc-trace ge_gap_stream_profile -- --exact --nocapture
//! ```
//!
//! With no `GE_GAP_STREAM_HOURS` the test still runs, at a small session, and
//! asserts that the class table gives back exactly the rows the reference walk
//! produced and is smaller than they are. The instrument is therefore also a
//! test.

use std::collections::{BTreeMap, BTreeSet};

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use recorder_host::gap_ledger;
use recorder_host::mem_account::{self, Charge, Component};
use serde_json::json;

use super::*;

/// Domains that shortfall every hour, as ~35 of 68 do on the real capture
/// `runs/live_unlock/t44c_shadow`.
const ALWAYS: &[u16] = &[
    7, 9, 11, 13, 17, 19, 21, 23, 27, 29, 31, 33, 37, 39, 41, 43, 47, 49, 51, 53,
];
/// Domains that come and go with the scheduler.
const ALTERNATING: &[u16] = &[54, 55, 56, 57, 58, 59, 60, 61, 62, 63];
/// Domains whose absence the PRODUCER accounts for with an H4DG outcome, so the
/// ledger carries a projection rather than a host-derived shortfall.
const OUTCOME: &[u16] = &[64, 65, 66, 67, 68, 69, 70, 71, 72, 73];
/// Domains that deliver every hour, one of them carrying a status word so the
/// status-only arm of the derivation is exercised too.
const HEALTHY: &[u16] = &[81, 82];

/// A summary with the shape a real capture has, at any session length.
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
        // A whole-clock skipped hour every ~26, as t44c carries 26 over 683.
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
        // One status word on a domain that DID deliver, so the record it raises
        // is a `StatusBits` one rather than a composite.
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
                    // `ReaderSilent` requires both ticks to be zero and projects
                    // to a `Coverage` witness whose expected DATA count is the
                    // range width.
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

/// What the pre-GE code held while the validators ran.
///
/// Constructed only by the `materialized` arm. Its rows come from
/// `reference_derive_gap_records`, which IS the walk this lane replaced.
///
/// # Three lifetimes, not one sum
///
/// The removed code did not hold everything at once, and a "before" arm that
/// does would publish a peak that never happened. What it did was:
///
///   1. `derive_gap_records` built `h4dg_accounted` and `coverage_shortfalls`
///      alongside the rows, and dropped BOTH SETS when it returned;
///   2. `derived_gap_ledger_is_valid` then built `encoded_seen`, the projection
///      multiset and the bounded rows, and dropped all three when IT returned --
///      which is inside the hourly-problems walk, before the manifest is built;
///   3. the ROWS outlived both and were still there for the manifest.
///
/// So this type is only phase two. Phase one lives in [`Self::derive`]'s own
/// scope, phase three is the `rows` the arm holds across everything, and the
/// number published is the LARGEST of the three, which is what a peak is.
struct MaterializedExpansion {
    /// `derived_gap_ledger_is_valid`'s duplicate check: one encoded copy of
    /// every row.
    _encoded_seen: BTreeSet<Vec<u8>>,
    /// Its projection multiset: one encoded copy of every H4DG projection.
    _projection_counts: BTreeMap<Vec<u8>, usize>,
    /// The rows it cloned out to hand to `validate_gap_records`.
    _bounded: Vec<GapRecord>,
    /// What this phase holds, including the rows it reads.
    bytes: u64,
    _charge: Charge,
}

/// The rows the derivation returned, held for as long as the pre-change
/// `gap_records` local was: across every validator and the manifest build.
struct MaterializedRows {
    rows: Vec<GapRecord>,
    /// The largest of the three phases seen so far.
    peak_bytes: std::cell::Cell<u64>,
    _charge: Charge,
}

/// A `BTreeSet`/`BTreeMap` of encoded records, priced as its keys plus their
/// `Vec` headers. A FLOOR: the node overhead of the tree itself is not counted,
/// which understates the "before" arm rather than flattering it.
fn encoded_key_bytes<'a>(keys: impl Iterator<Item = &'a Vec<u8>>) -> u64 {
    keys.map(|encoded| (encoded.len() + std::mem::size_of::<Vec<u8>>()) as u64)
        .sum()
}

impl MaterializedRows {
    /// PHASE ONE, then the rows alone.
    fn derive(summary: &CompactSummary, expected: &[u16]) -> Self {
        let pair = std::mem::size_of::<(i32, u16)>() as u64;
        let (rows, derivation_bytes) = {
            let reference = reference_derive_gap_records(summary, expected);
            let bytes = gap_ledger::gap_records_resident_bytes(&reference.rows)
                + (reference.h4dg_accounted.len() as u64) * pair
                + (reference.coverage_shortfalls.len() as u64) * pair;
            let _phase = Charge::new(Component::ManifestGapLedger, bytes);
            // Read while the charge and both sets are live, so the peak this
            // records is one the process really reached.
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

    /// PHASE TWO, for as long as the returned guard lives. The caller scopes it
    /// to the hourly-problems walk, which is where `derived_gap_ledger_is_valid`
    /// ran and where all three of these structures died.
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
        // Priced BEFORE the walk consumes the counts, and consumed IN PLACE:
        // the removed code decremented `h4dg_projection_counts` through
        // `get_mut`, so a second copy of it here would be a map the process
        // never held.
        let projection_bytes = encoded_key_bytes(projection_counts.keys());
        for record in &self.rows {
            if let Ok(encoded) = compact_contract::encode_gap_record(record) {
                match projection_counts.get_mut(&encoded).filter(|left| **left != 0) {
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
            // Only this phase's OWN extra bytes: the rows are already charged
            // by the guard this borrows from, and charging them twice would be
            // the over-count `accounting_consistent` exists to catch.
            _charge: Charge::new(
                Component::ManifestGapLedger,
                bytes - gap_ledger::gap_records_resident_bytes(&self.rows),
            ),
        }
    }

    /// The largest of the three phases the removed code went through.
    fn charged_bytes(&self) -> u64 {
        self.peak_bytes.get()
    }
}

/// Run one arm and return the report.
fn run_arm(hours: i32, arm: &str) -> serde_json::Value {
    let (summary, expected) = synthetic_summary(hours);
    mem_account::mark("profile.summary_built");
    // Everything above this line is the INPUT. Resetting here is what makes the
    // peak below the phase's own, rather than the cost of a session-length
    // summary that both arms need identically.
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

    // The four validators, exactly as finalize runs them.
    let coverage_errors = compact_domain_coverage_errors_with_gaps(&summary, &expected, &ledger);
    let hourly_problems = {
        // `derived_gap_ledger_is_valid` runs inside this call and its three
        // structures died when it returned, so the `materialized` arm holds
        // them for exactly this scope and no longer.
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
        // The allocator oracle. `reset_peak_live_heap` sets the peak to what is
        // live, it does not zero it, so the absolute reading below still carries
        // the synthetic summary the phase was handed. Both arms are handed an
        // identical summary, so the number the comparison is stated in is the
        // one ABOVE it -- published as its own field rather than left for a
        // reader to subtract. `null` unless built with `mem-alloc-trace`.
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

/// The instrument, and a test of the property it exists to demonstrate.
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

    // No session length asked for: run small, and assert what the instrument is
    // for rather than printing a number nobody asked for.
    //
    // The rows are taken from `reference_derive_gap_records` -- the walk this
    // lane replaced -- so this compares the class table against an INDEPENDENT
    // expansion. Taking them from `derive_gap_records` would put
    // `visit_gap_records` on both sides of the comparison, and a source dropped
    // from it would then satisfy the assertion.
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
