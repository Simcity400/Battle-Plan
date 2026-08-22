//! Domain 44 torn-read retry: the closed defect class is a TRANSIENT live
//! read tear voiding the whole hour. On `pw_gate_final`, 11 frames published
//! su|pd because the division body walk came back short against its own
//! census (first-failure reasons 7 = LOCATION, 3 = UNSTABLE -- both
//! mid-write tears), and 1 frame published su because the country
//! disrupted-supply walk failed its substage-609 validation reread. Every
//! refusal was honest; every one was also healable by re-reading the same
//! bytes a moment later. The retry wrappers below give both walks the same
//! bounded budget the province sampler has always spent
//! (`sample_compact_supply_logistics_province_rows_with_retry`): whole walks,
//! `thread::yield_now()` between attempts (never a wait, never a pause),
//! worker-panic arms still fatal and un-retried, and an exhausted budget
//! refusing by name exactly as the single-shot form did.

use super::*;

/// kills: deleting the division tear-retry arm outright. The first walk of
/// `pw_gate_final` hour 60830904-class tears returned a roster short of its
/// census and poisoned the frame with su|pd; a healed second attempt must
/// return that exact roster with NO poison flags. A single-shot regression
/// publishes after attempt one and this test fails on the call count.
#[test]
fn division_retry_heals_a_transient_short_roster() {
    let mut calls = 0usize;
    let observed = 5u32;
    let rows = compact_supply_division_rows_with_tear_retry(
        COMPACT_SUPPLY_DIVISION_ROSTER_READ_ATTEMPTS,
        observed,
        || {
            calls += 1;
            if calls == 1 {
                Ok(vec![1u32, 2, 3]) // torn: three of five bodies read clean
            } else {
                Ok(vec![1, 2, 3, 4, 5]) // next pass: all five read clean
            }
        },
    )
    .expect("a transient tear heals within the budget");
    assert_eq!(calls, 2, "one retry, not zero (single-shot) and not N (burn)");
    assert_eq!(rows.len(), 5, "the HEALED roster is what gets published");
}

/// kills: an unbounded retry loop (the engine-wait law) and a loop that keeps
/// walking past the budget. A tear that survives every attempt is real, not
/// transient: exactly `attempts` whole walks run, and the FINAL attempt's
/// payload is the one returned so the caller poisons via
/// `compact_supply_division_roster_status` on this frame's own truth.
#[test]
fn division_retry_is_bounded_and_publishes_the_final_attempt() {
    let mut calls = 0usize;
    let rows = compact_supply_division_rows_with_tear_retry(
        COMPACT_SUPPLY_DIVISION_ROSTER_READ_ATTEMPTS,
        4u32,
        || {
            calls += 1;
            // Distinguishable per-attempt rosters: a wrapper that runs three
            // walks but returns an EARLIER one fails this assertion.
            Ok(match calls {
                1 => vec![10u32, 11, 12],
                2 => vec![20u32, 21, 22],
                _ => vec![30u32, 31, 32],
            })
        },
    )
    .expect("short is not an error; the predicate poisons it");
    assert_eq!(calls, COMPACT_SUPPLY_DIVISION_ROSTER_READ_ATTEMPTS);
    assert_eq!(
        rows,
        vec![30u32, 31, 32],
        "the LAST attempt's roster, byte for byte"
    );
}

/// kills: routing the shard-panic arm through the tear-retry path. A panicked
/// worker shard was fatal-and-immediate before this change and must stay so:
/// one call, Err out, no yield-and-rewalk papering over a broken shard.
#[test]
fn division_worker_panic_stays_fatal_and_unretried() {
    let mut calls = 0usize;
    let panic = compact_supply_division_rows_with_tear_retry::<u32>(
        COMPACT_SUPPLY_DIVISION_ROSTER_READ_ATTEMPTS,
        1u32,
        || {
            calls += 1;
            Err(ShardWorkerPanic { shard_index: 2 })
        },
    );
    assert!(matches!(panic, Err(ShardWorkerPanic { shard_index: 2 })));
    assert_eq!(calls, 1, "panics are never retried");
}

/// kills: an "always spend the full budget" regression that rewalks even when
/// the first walk was already exact -- wasted reads on the hottest D44 path.
#[test]
fn division_exact_first_walk_is_not_rewalked() {
    let mut calls = 0usize;
    compact_supply_division_rows_with_tear_retry(
        COMPACT_SUPPLY_DIVISION_ROSTER_READ_ATTEMPTS,
        2u32,
        || {
            calls += 1;
            Ok(vec![9u32, 8])
        },
    )
    .expect("exact on the first attempt");
    assert_eq!(calls, 1);
}

/// kills: removing the `.max(1)` clamp and underflowing the remaining-attempt
/// counter into an infinite or zero-walk state. Zero attempts means the old
/// single-shot behaviour: one walk, its truth published whatever it is.
#[test]
fn division_zero_attempts_clamps_to_single_shot() {
    let mut calls = 0usize;
    let rows = compact_supply_division_rows_with_tear_retry(0, 3u32, || {
        calls += 1;
        Ok(vec![1u32])
    })
    .expect("clamped to one attempt");
    assert_eq!(calls, 1);
    assert_eq!(rows.len(), 1);
}

/// kills: deleting the country-side retry arm. The pw_gate_final frame that
/// died in the substage-609 validation reread published bare SOURCE_UNAVAILABLE
/// for the hour; its tables were mid-mutation for one instant only. The healed
/// second attempt must return the rows.
#[test]
fn country_retry_heals_a_transient_validation_reread_failure() {
    let mut calls = 0usize;
    let rows = sample_compact_supply_country_disrupted_rows_with_tear_retry(
        COMPACT_SUPPLY_COUNTRY_ROWS_READ_ATTEMPTS,
        || {
            calls += 1;
            if calls == 1 {
                None // header/blob/string snapshots disagreed: torn
            } else {
                Some(41u32)
            }
        },
    );
    assert_eq!(calls, 2);
    assert_eq!(rows, Some(41));
}

/// kills: an unbounded country retry loop, and a loop that reports failure
/// while attempts remain. Exhaustion returns None after EXACTLY the budget --
/// the frame then refuses by name (bare SOURCE_UNAVAILABLE), unchanged from
/// the single-shot contract.
#[test]
fn country_retry_exhausts_to_none_after_exactly_the_budget() {
    let mut calls = 0usize;
    let rows: Option<u32> = sample_compact_supply_country_disrupted_rows_with_tear_retry(
        COMPACT_SUPPLY_COUNTRY_ROWS_READ_ATTEMPTS,
        || {
            calls += 1;
            None
        },
    );
    assert_eq!(calls, COMPACT_SUPPLY_COUNTRY_ROWS_READ_ATTEMPTS);
    assert_eq!(rows, None);
}

/// kills: a success-shaped regression where the wrapper drops a good read or
/// invents one: Some passes through untouched, on any attempt.
#[test]
fn country_success_on_any_attempt_passes_through() {
    for good_on in 1..=COMPACT_SUPPLY_COUNTRY_ROWS_READ_ATTEMPTS {
        let mut calls = 0usize;
        let rows = sample_compact_supply_country_disrupted_rows_with_tear_retry(
            COMPACT_SUPPLY_COUNTRY_ROWS_READ_ATTEMPTS,
            || {
                calls += 1;
                (calls == good_on).then_some(good_on * 10)
            },
        );
        assert_eq!(calls, good_on);
        assert_eq!(rows, Some(good_on * 10));
    }
}

// ---- structural pins -------------------------------------------------------
//
// The functional tests above prove the wrappers' behaviour; these pins prove
// the d44 FRAME actually routes both walks through them (deleting the wiring
// while keeping the wrappers would otherwise be invisible to unit tests).
// Source scans normalise CRLF first: this checkout lands attach_log.rs with
// CRLF line endings and multi-line needles must match regardless.

fn normalized_attach_log_source() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/attach_log.rs"))
        .expect("attach_log.rs readable")
        .replace("\r\n", "\n")
}

fn source_between<'a>(source: &'a str, start_marker: &str, close: &str) -> &'a str {
    let start = source.find(start_marker).unwrap_or_else(|| {
        panic!("marker not found: {start_marker}")
    });
    let end = source[start..].find(close).expect("closing marker");
    &source[start..start + end + close.len()]
}

/// kills: rewiring `read_compact_supply_logistics_attrition` back to the
/// single-shot walks (the exact pre-fix code shape) while leaving the new
/// wrappers as dead code.
#[test]
fn d44_frame_routes_both_walks_through_the_tear_retry_wrappers() {
    let source = normalized_attach_log_source();
    let frame = source_between(
        &source,
        "fn read_compact_supply_logistics_attrition(",
        "\n}\n",
    );
    assert!(
        frame.contains("compact_supply_division_rows_with_tear_retry(\n        COMPACT_SUPPLY_DIVISION_ROSTER_READ_ATTEMPTS,"),
        "the division walk must run under the bounded tear-retry budget"
    );
    assert!(
        frame.contains("sample_compact_supply_country_disrupted_rows_with_tear_retry(\n        COMPACT_SUPPLY_COUNTRY_ROWS_READ_ATTEMPTS,"),
        "the country disrupted-supply walk must run under the bounded tear-retry budget"
    );
    assert!(
        frame.contains("compact_supply_logistics_shard_panic_read("),
        "the worker-panic early return stays at the division call site"
    );
    assert!(
        frame.contains("compact_supply_division_roster_status("),
        "the exhausted-budget poison still flows through the roster predicate"
    );
}

/// kills: turning a retry into a wait -- a sleep/park/spin/block between
/// attempts would hold the collector against the running game, violating the
/// owner law the province sampler already obeys (`yield_now` only). The scan
/// pins concrete wait CALL shapes, not prose: any of these tokens inside a
/// wrapper body is a waiting retry.
#[test]
fn retries_yield_never_wait() {
    let source = normalized_attach_log_source();
    const WAIT_SHAPES: [&str; 5] = [
        "sleep(",
        "park(",
        "spin_loop(",
        ".wait(",
        "recv_timeout(",
    ];
    // Prefix markers, no trailing paren: the wrappers are generic (`<T>`),
    // and a marker that pinned the exact parameter list would desync from any
    // future signature change.
    for wrapper in [
        "fn compact_supply_division_rows_with_tear_retry",
        "fn sample_compact_supply_country_disrupted_rows_with_tear_retry",
    ] {
        let body = source_between(&source, wrapper, "\n}\n");
        assert!(
            body.contains("thread::yield_now()"),
            "{wrapper} must yield between attempts"
        );
        for shape in WAIT_SHAPES {
            assert!(
                !body.contains(shape),
                "{wrapper} must never wait: found `{shape}` in its body"
            );
        }
    }
}

/// kills: re-scattering the diagnostic resets back into the deep sub-walks.
/// The retry wrapper re-enters `compact_supply_logistics_attrition_country_disrupted_supply_rows`
/// once per attempt; only an ENTRY-level reset of EVERY publishable atomic
/// makes a refusal name its OWN attempt. If even one diagnostic is reset
/// deeper than the entry, attempt two failing early would publish attempt
/// one's stale substage/counter as the refusal's name (Codex round 1,
/// finding 1).
#[test]
fn country_entry_resets_every_diagnostic_it_can_publish() {
    let source = normalized_attach_log_source();
    let entry = source_between(
        &source,
        "fn compact_supply_logistics_attrition_country_disrupted_supply_rows(",
        "\n}\n",
    );
    for diagnostic in [
        "DOMAIN44_COUNTRY_SUPPLY_FAILURE_STAGE",
        "DOMAIN44_COUNTRY_SUPPLY_FAILURE_BRANCH",
        "DOMAIN44_COUNTRY_SUPPLY_SLOT_INDEX",
        "DOMAIN44_COUNTRY_SUPPLY_TAG_DB_ID",
        "DOMAIN44_DISRUPTED_TABLE_FAILURE_SUBSTAGE",
        "DOMAIN44_DISRUPTED_TABLE_OCCUPIED_COUNT",
        "DOMAIN44_DISRUPTED_TABLE_MASK",
        "DOMAIN44_DISRUPTED_TABLE_MAX_DISTANCE",
        "DOMAIN44_DISRUPTED_TABLE_OFFENDING_MARKER",
        "DOMAIN44_DISRUPTED_TABLE_OFFENDING_VALUE_NEGATIVE",
        "DOMAIN44_DISRUPTED_TABLE_OFFENDING_VALUE_ABS",
        "DOMAIN44_FOREIGN_HOMEBASE_FAILURE_SUBSTAGE",
        "DOMAIN44_FOREIGN_HOMEBASE_CAPACITY",
        "DOMAIN44_FOREIGN_HOMEBASE_COUNT",
        "DOMAIN44_FOREIGN_HOMEBASE_BAD_PROVINCE",
        "DOMAIN44_FOREIGN_HOMEBASE_BAD_BASE_RAW",
    ] {
        assert!(
            entry.contains(&format!("{diagnostic}.store(")),
            "{diagnostic} must be reset at the walk entry so each retry attempt names its own truth"
        );
    }
}

/// kills: softening `compact_supply_division_roster_status` (e.g. treating a
/// short roster as deletions) once retries make refusals rare. The poison arm
/// IS the honest-refusal contract for a roster that is genuinely short or
/// ambiguous; retries only make honest refusals rare, never silent.
#[test]
fn exhausted_budget_still_poisons_through_the_roster_predicate() {
    let source = normalized_attach_log_source();
    let predicate = source_between(
        &source,
        "fn compact_supply_division_roster_status(",
        "\n}\n",
    );
    assert!(
        predicate.contains("status_flags | COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE"),
        "a roster that is still short/ambiguous after the final attempt must refuse by name"
    );
}
