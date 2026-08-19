//! HN_OUTCOME_HOURS=68300 cargo test --release -p recorder_host \

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use recorder_host::gap_ledger::{self, DomainOutcome};
use recorder_host::mem_account;
use serde_json::json;

const OUTCOME_DOMAINS: &[u16] = &[64, 65, 66, 67, 68, 69, 70, 71, 72, 73];

fn synthetic_outcomes(hours: i32) -> Vec<DomainOutcome> {
    let base = 60_832_235i32;
    let mut outcomes = Vec::new();
    let mut next: std::collections::BTreeMap<u16, u64> = std::collections::BTreeMap::new();
    for step in 0..hours {
        if step % 26 == 25 {
            continue;
        }
        let hour = base + step;
        for (index, domain) in OUTCOME_DOMAINS.iter().enumerate() {
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

struct ArmCost {
    bytes: Vec<u8>,
    transient: Option<u64>,
    retained: Option<u64>,
    peak_above_input: Option<u64>,
}

fn measure(arm: &str, outcomes: &[DomainOutcome]) -> ArmCost {
    mem_account::reset_peak_live_heap();
    let live_before = mem_account::live_heap_bytes();
    let ledger = match arm {
        "streamed" => gap_ledger::encode_domain_gap_outcomes(&outcomes[..]),
        "reference" => gap_ledger::reference_encode_domain_gap_outcomes(outcomes.iter().copied()),
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

#[test]
fn hn_outcome_encoder_profile() {
    let hours: i32 = std::env::var("HN_OUTCOME_HOURS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(683);

    let outcomes = synthetic_outcomes(hours);
    let rows = outcomes.len();
    assert!(rows > 0, "the fixture produced no outcomes");

    let streamed = measure("streamed", &outcomes);
    let reference = measure("reference", &outcomes);
    let reference_first = measure("reference", &outcomes);
    let streamed_second = measure("streamed", &outcomes);

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

    if mem_account::live_heap_bytes().is_none() {
        println!(
            "heap oracle absent: build with --features mem-alloc-trace for the \
             memory arms; the equality above still holds"
        );
        return;
    }

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
    assert!(
        reference_first.transient.expect("oracle installed")
            > streamed_second.transient.expect("oracle installed"),
        "the two orderings disagree, so this is an allocator measurement"
    );
}
