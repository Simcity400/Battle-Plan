//! Price `encode_domain_gap_outcomes`'s TRANSIENT STATE, before and after
//! task #179-1, in ONE process.
//!
//! # What this measures, and what it does not
//!
//! This measures the largest remaining term in the finalize gap phase. The term
//! is not the ledger it produces — that is
//! O(classes + hour runs) and did not change, which the byte-identity test in
//! `gap_ledger/tests.rs` pins. The term is the state the encoder holds WHILE
//! producing it, and the two arms here differ in nothing else:
//!
//! * `reference` — `reference_encode_domain_gap_outcomes`, the pre-#179-1
//!   encoder kept verbatim. It builds `lane_ordinal_firsts`' per-lane
//!   `(hour, ordinal)` vectors, a `per_hour` map holding one `(domain, ordinal)`
//!   per row, and a class map holding one `u64` ordinal per row that
//!   `lane_sequential` encoding then discards.
//! * `streamed` — the shipping encoder. One sorted row index (one `usize`) per
//!   row, an O(lanes) lane check, one hour's rows at a time, and a run-encoder
//!   accumulator per class.
//!
//! # THE BASIS: PER OUTCOME ROW
//!
//! Every ratio is stated per `summary.domain_gaps` row, which is what the
//! encoder is O(). Not per game hour and not per class: a real capture in this
//! tree (`runs/live_unlock/t49_pricing`) carries 179 OUTCOME classes against
//! its whole outcome table, so a per-class basis would understate the same
//! change by orders of magnitude. The row count is published in every report so
//! the basis can be checked rather than taken on trust.
//!
//! # WHAT THE FIXTURE IS, EXACTLY
//!
//! Ten outcome domains, half of them alternating with the scheduler, one
//! whole-clock skipped hour every 26 -- and, because the class key is the
//! (reason, part, queue, range, anchored ticks, domain set) tuple and this
//! fixture varies only the DOMAIN SET, it produces TWO outcome classes. It is
//! not a model of a real capture's 179; it is a fixture in which the class
//! table is small and constant, so that what the arms differ by is the per-row
//! state and not the table. The published report carries the class count so
//! this is checkable rather than asserted (Codex round 6, which caught an
//! earlier draft claiming seven -- that figure is lane GE's `gap_classes`, a
//! different table).
//!
//! # Why one process is legitimate here, when `ge_gap_stream` needed two
//!
//! `ge_gap_stream_profile` measures `mem_account` COMPONENT peaks, which are
//! process statics that never fall, so two arms in one process publish the
//! larger twice. This measures the ALLOCATOR's live-heap peak, which
//! `mem_account::reset_peak_live_heap` can lower to the current live figure —
//! so each arm's peak is its own high-water above the input both arms share.
//! That is the in-process A/B the working method asks for, and it removes the
//! cross-process comparison GE had to correct for.
//!
//! The input is built ONCE and both arms read it, so nothing separates them but
//! the encoder. The arms run in both orders and both orderings are published:
//! if an allocator's retained arenas made the second arm cheaper, the two
//! orderings would disagree.
//!
//! ```text
//! HN_OUTCOME_HOURS=68300 cargo test --release -p recorder_host \
//!   --bin collector_record --features mem-alloc-trace \
//!   hn_outcome_encoder_profile -- --exact --nocapture
//! ```
//!
//! With no `HN_OUTCOME_HOURS` it still runs, at a small session, and asserts the
//! two arms emit identical bytes. The instrument is therefore also a test.

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use recorder_host::gap_ledger::{self, DomainOutcome};
use recorder_host::mem_account;
use serde_json::json;

/// Domains whose absence the PRODUCER accounts for with an H4DG outcome, in the
/// proportion `runs/live_unlock/t44c_shadow` carries: ten of them, one outcome
/// per hour each, against a scheduler that drops some every other hour.
const OUTCOME_DOMAINS: &[u16] = &[64, 65, 66, 67, 68, 69, 70, 71, 72, 73];

/// The outcomes a session of `hours` produces, in PUBLICATION order.
///
/// Publication order is the point: `summary.domain_gaps` is ordered by the frame
/// that reported each observation, not by the hour it describes, which is the
/// whole reason the old encoder could not stream. The rotation at the end makes
/// that explicit rather than incidental.
fn synthetic_outcomes(hours: i32) -> Vec<DomainOutcome> {
    let base = 60_832_235i32;
    let mut outcomes = Vec::new();
    let mut next: std::collections::BTreeMap<u16, u64> = std::collections::BTreeMap::new();
    for step in 0..hours {
        // A whole-clock skipped hour every ~26, as t44c carries 26 over 683.
        if step % 26 == 25 {
            continue;
        }
        let hour = base + step;
        for (index, domain) in OUTCOME_DOMAINS.iter().enumerate() {
            // Half the set alternates with the scheduler, which is what opens a
            // new run every second hour and keeps the class table honest.
            if index % 2 == 1 && step % 2 == 1 {
                continue;
            }
            let ordinal = next.entry(*domain).or_default();
            outcomes.push((
                *domain,
                DomainGapRecordV1 {
                    reason: DomainGapReason::Overrun,
                    part: DomainGapPart::State,
                    target_hour_raw: hour,
                    range_start_hour_raw: hour,
                    range_end_hour_raw: hour,
                    outcome_ordinal: *ordinal,
                    queue_capacity: 64,
                    queue_depth: 1,
                    queue_dropped: 0,
                    // THE TICKS TRACK THE HOUR, and a first draft that left
                    // them at zero is why this matters. The class key stores
                    // ticks ANCHORED to the target hour (`Anchored::from_tick`
                    // records `tick - hour`), so a constant tick makes every
                    // hour its own class -- the ledger becomes O(hours), 5,000
                    // classes at 683 hours, and it dwarfs the very state this
                    // instrument exists to compare. A real capture's ticks
                    // advance with the clock, which is what collapses the table
                    // to the 7 classes lane GE measured at 68,300 hours.
                    tick_before: u64::try_from(hour).unwrap_or(0),
                    tick_after: u64::try_from(hour).unwrap_or(0) + 1,
                },
            ));
            *ordinal += 1;
        }
    }
    let split = outcomes.len() / 3;
    outcomes.rotate_left(split);
    outcomes
}

/// What one arm cost, split into the part that DIED and the part that stayed.
struct ArmCost {
    bytes: Vec<u8>,
    /// `peak - live_after_encode`: the state the encoder held and then dropped.
    /// THIS IS THE TERM #179-1 TARGETS, and it is the only figure the arms are
    /// compared on.
    transient: Option<u64>,
    /// `live_after_encode - live_before`: the ledger the arm produced. Common to
    /// both arms by construction — the byte-identity assertion below proves it —
    /// and published so a reader can see it is common rather than assume it.
    retained: Option<u64>,
    peak_above_input: Option<u64>,
}

/// One arm: reset the allocator's peak to the live figure, encode, read the peak
/// and the live figure back.
///
/// THE SERIALIZATION IS OUTSIDE THE WINDOW, and a first draft that left it
/// inside is why: `to_vec` allocates the whole JSON, which is the same document
/// for both arms and an order of magnitude larger than what separates them, so
/// it swamped the comparison and the 683-hour arm reported the streaming encoder
/// as the more expensive one. The peak is read before anything else touches the
/// heap.
fn measure(arm: &str, outcomes: &[DomainOutcome]) -> ArmCost {
    mem_account::reset_peak_live_heap();
    let live_before = mem_account::live_heap_bytes();
    let ledger = match arm {
        "streamed" => gap_ledger::encode_domain_gap_outcomes(&outcomes[..]),
        "reference" => {
            gap_ledger::reference_encode_domain_gap_outcomes(outcomes.iter().copied())
        }
        other => panic!("unknown arm {other}"),
    };
    let peak = mem_account::peak_live_heap_bytes();
    let live_after = mem_account::live_heap_bytes();
    let bytes = serde_json::to_vec(&ledger).expect("the ledger serializes");
    drop(ledger);
    let transient = match (peak, live_after) {
        (Some(peak), Some(after)) => Some(peak.saturating_sub(after)),
        _ => None,
    };
    let retained = match (live_after, live_before) {
        (Some(after), Some(before)) => Some(after.saturating_sub(before)),
        _ => None,
    };
    let peak_above_input = match (peak, live_before) {
        (Some(peak), Some(before)) => Some(peak.saturating_sub(before)),
        _ => None,
    };
    ArmCost {
        bytes,
        transient,
        retained,
        peak_above_input,
    }
}

/// The instrument, and the gate.
///
/// As an instrument it publishes the A/B; as a test it holds three properties.
/// The third is the one that matters and the first draft did not have it: the
/// inequality "reference holds more than streamed" is NOT a gate against
/// reintroducing per-row state, because putting one `u64` a row back into the
/// streaming encoder would still leave it below the reference (Codex round 6).
/// A per-row BOUND is, and that is what `MAX_STREAMED_BYTES_PER_ROW` is for.
///
/// kills:
///   * change either encoder's output -- the byte-equality assertion fails;
///   * accumulate `ClassAccumulator::ordinals` unconditionally, or hold any
///     other per-row structure in the streaming encoder -- eight more bytes a
///     row breaks the per-row bound;
///   * point the `streamed` arm at `reference_encode_domain_gap_outcomes` --
///     the bound and the direction both fail.
#[test]
fn hn_outcome_encoder_profile() {
    let hours: i32 = std::env::var("HN_OUTCOME_HOURS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(683);

    let outcomes = synthetic_outcomes(hours);
    let rows = outcomes.len();
    assert!(rows > 0, "the fixture produced no outcomes");

    // BOTH ORDERINGS. If an arm were cheaper only because the allocator had
    // already grown for the other, the two orderings would disagree.
    let streamed = measure("streamed", &outcomes);
    let reference = measure("reference", &outcomes);
    let reference_first = measure("reference", &outcomes);
    let streamed_second = measure("streamed", &outcomes);

    // THE EQUALITY, FIRST. A size measurement of an encoder that changed its
    // output would be worthless.
    assert_eq!(
        streamed.bytes, reference.bytes,
        "the two arms must emit identical bytes at {hours} hours"
    );

    let ratio = match (streamed.transient, reference.transient) {
        (Some(s), Some(r)) if s > 0 => Some(r as f64 / s as f64),
        _ => None,
    };
    let report = json!({
        "instrument": "hn_outcome_encoder_profile",
        "basis": "per outcome row (summary.domain_gaps), not per game hour and not per class",
        "compared_on": "transient = peak_live_heap - live_heap_after_encode, i.e. the state \
                        the encoder held and then dropped; the ledger it produced is `retained` \
                        and is identical in both arms",
        "game_hours": hours,
        "outcome_rows": rows,
        "ledger_json_bytes": streamed.bytes.len(),
        "outcome_classes": serde_json::from_slice::<serde_json::Value>(&streamed.bytes)
            .ok()
            .and_then(|v| v["class_count"].as_u64()),
        "sorted_index_bytes": rows * std::mem::size_of::<usize>(),
        "arms": {
            "streamed": {
                "transient_bytes": streamed.transient,
                "retained_bytes": streamed.retained,
                "peak_above_input_bytes": streamed.peak_above_input,
                "transient_bytes_second_ordering": streamed_second.transient,
            },
            "reference": {
                "transient_bytes": reference.transient,
                "retained_bytes": reference.retained,
                "peak_above_input_bytes": reference.peak_above_input,
                "transient_bytes_first_ordering": reference_first.transient,
            },
        },
        "reference_over_streamed": ratio,
        "transient_bytes_per_outcome_row": {
            "streamed": streamed.transient.map(|b| b as f64 / rows as f64),
            "reference": reference.transient.map(|b| b as f64 / rows as f64),
        },
        "heap_oracle_installed": mem_account::live_heap_bytes().is_some(),
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());

    // WITHOUT THE ORACLE THIS IS A CONFORMANCE TEST, NOT A MEASUREMENT, and it
    // says so rather than publishing zeros as if they were findings.
    if mem_account::live_heap_bytes().is_none() {
        println!(
            "heap oracle absent: build with --features mem-alloc-trace for the \
             memory arms; the equality above still holds"
        );
        return;
    }

    // The ledger is the same document, so the arms must also RETAIN the same
    // thing. If they did not, `transient` would be comparing two different
    // remainders.
    let (streamed_retained, reference_retained) = (
        streamed.retained.expect("oracle installed"),
        reference.retained.expect("oracle installed"),
    );
    let retained_gap = streamed_retained.abs_diff(reference_retained);
    assert!(
        retained_gap * 20 <= streamed_retained.max(reference_retained).max(1),
        "the arms retain different amounts ({streamed_retained} against \
         {reference_retained}), so the transient figures are not comparable"
    );

    let streamed_transient = streamed.transient.expect("oracle installed");
    let reference_transient = reference.transient.expect("oracle installed");

    // THE BOUND, and it is the gate. The streaming encoder's transient state is
    // the sorted index plus O(classes); the index is one `usize` a row, so the
    // per-row figure sits just above eight bytes at every measured length
    // (8.3, 8.6, 8.5). Sixteen leaves room for allocator rounding and for the
    // class table's share at a short session, and still fails the moment any
    // second per-row term comes back -- one `u64` a row would put it past 16.
    const MAX_STREAMED_BYTES_PER_ROW: f64 = 16.0;
    let streamed_per_row = streamed_transient as f64 / rows as f64;
    assert!(
        streamed_per_row < MAX_STREAMED_BYTES_PER_ROW,
        "the streaming encoder holds {streamed_per_row:.1} bytes per outcome row \
         against a bound of {MAX_STREAMED_BYTES_PER_ROW:.1}: something per-row \
         came back beside the sorted index"
    );
    assert!(
        reference_transient > streamed_transient,
        "the pre-change encoder must hold more than the streaming one: \
         reference {reference_transient}, streamed {streamed_transient} at {rows} rows"
    );
    // The orderings must agree on the direction, or the arms are measuring the
    // allocator rather than the encoders.
    assert!(
        reference_first.transient.expect("oracle installed")
            > streamed_second.transient.expect("oracle installed"),
        "the two orderings disagree, so this is an allocator measurement"
    );
}
