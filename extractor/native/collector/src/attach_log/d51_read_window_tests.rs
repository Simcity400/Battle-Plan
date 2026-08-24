//! Lane SE's proof that cutting work out of the d51 read window left every
//! recorded byte where it was.
//!
//! Three claims, one test group each:
//!
//! 1. the encoder's skipped defensive sort is a no-op only where it is a
//!    no-op: canonical and shuffled inputs encode to the same bytes;
//! 2. `PayloadShape::RowsOnly` differs from `Encoded` in the payload field and
//!    in nothing else, and re-encoding a `RowsOnly` frame reproduces the
//!    `Encoded` payload byte for byte;
//! 3. the one-pass delta selection emits exactly what the two-pass shape it
//!    replaced emitted - the reference half is that shape, restated here
//!    statement for statement.
//!
//! All three are post-read computation over rows the reader had already
//! materialised, which is why this file can prove them by restating what the
//! old code did and comparing outputs. The lane's fourth and fifth cuts are
//! proved in the lane crate beside the code they changed; one of those (the
//! phase-loop pre-count) does stop issuing guarded reads, but only reads that
//! fed parse passes which could not have been accepted - see
//! `runs/se_flags_worker/MEASUREMENT.md` section 3.

use super::*;

use domain51::scripted_state_domain51_collector::compact as d51c;

// ---------------------------------------------------------------------
// the frame-shaping and delta cuts
// ---------------------------------------------------------------------

fn variable_row(
    scope_kind: u32,
    owner_id: u32,
    key: &str,
    value_raw: i64,
) -> d51c::ScriptedStateRow {
    d51c::ScriptedStateRow {
        scope_kind,
        owner_id,
        value_kind: d51c::VALUE_VARIABLE_FIXED_I64,
        key: Some(key.to_string()),
        token_id: None,
        value_raw: Some(value_raw),
        date_hour_raw: None,
        deleted: false,
        delayed_due_hours: None,
    }
}

fn token_flag_row(
    scope_kind: u32,
    owner_id: u32,
    token_id: u32,
    hour: i32,
) -> d51c::ScriptedStateRow {
    d51c::ScriptedStateRow {
        scope_kind,
        owner_id,
        value_kind: d51c::VALUE_FLAG_TOKEN_ONLY,
        key: None,
        token_id: Some(token_id),
        value_raw: Some(1),
        date_hour_raw: Some(hour),
        deleted: false,
        delayed_due_hours: None,
    }
}

fn mixed_rows() -> Vec<d51c::ScriptedStateRow> {
    vec![
        variable_row(d51c::SCOPE_GLOBAL, 0, "global_war_support", 550_000),
        variable_row(d51c::SCOPE_COUNTRY, 12, "alpha_counter", 1),
        variable_row(d51c::SCOPE_COUNTRY, 12, "beta_counter", -7),
        variable_row(d51c::SCOPE_COUNTRY, 40, "alpha_counter", 900_000),
        variable_row(d51c::SCOPE_STATE, 412, "state_var", 3),
        token_flag_row(d51c::SCOPE_COUNTRY, 12, 9001, 4_812_000),
        token_flag_row(d51c::SCOPE_STATE, 412, 9002, 4_812_001),
    ]
}

fn test_header(rows: &[d51c::ScriptedStateRow]) -> d51c::ScriptedStateHeader {
    d51c::ScriptedStateHeader {
        elapsed_wall_ms: 4,
        status_flags: 0,
        changed_rows: rows.len() as u32,
        observed_scopes: 8,
        measured_scopes: 8,
        rejected_scopes: 0,
    }
}

#[test]
fn the_encoder_produces_the_same_bytes_from_canonical_and_shuffled_rows() {
    // kills: making the skipped defensive sort unconditional. `encode_payload`
    // now sorts only when the input is not already non-decreasing under
    // `canonical_row_cmp`; if that check were replaced by `true` the shuffled
    // input below would serialise in caller order and these bytes would part.
    let mut canonical = mixed_rows();
    canonical.sort_by(d51c::canonical_row_cmp);
    let mut shuffled = canonical.clone();
    shuffled.reverse();
    shuffled.swap(0, 3);

    let header = test_header(&canonical);
    assert_eq!(
        d51c::encode_payload(&header, &canonical),
        d51c::encode_payload(&header, &shuffled)
    );
}

fn scope_pair(
    scope_kind: u32,
    owner_id: u32,
    variables: &[(&str, i64)],
    flags: &[(u32, i32)],
) -> (domain51::VariableScopeRead, domain51::FlagScopeRead) {
    let variable_status = if variables.is_empty() {
        domain51::ScopeReadStatus::MeasuredEmpty
    } else {
        domain51::ScopeReadStatus::Measured
    };
    let flag_status = if flags.is_empty() {
        domain51::ScopeReadStatus::MeasuredEmpty
    } else {
        domain51::ScopeReadStatus::Measured
    };
    (
        domain51::VariableScopeRead {
            scope_kind,
            owner_id,
            status: variable_status,
            entries: variables
                .iter()
                .map(|(key, value_raw)| domain51::VariableEntry {
                    key: (*key).to_string(),
                    value_raw: *value_raw,
                })
                .collect(),
        },
        domain51::FlagScopeRead {
            scope_kind,
            owner_id,
            status: flag_status,
            entries: flags
                .iter()
                .map(|(token_id, hour)| domain51::FlagEntry {
                    key: None,
                    token_id: Some(*token_id),
                    date_hour_raw: Some(*hour),
                    value: Some(1),
                })
                .collect(),
        },
    )
}

fn a13_input() -> domain51::A13FrameInput {
    let pairs = vec![
        scope_pair(
            d51c::SCOPE_GLOBAL,
            0,
            &[("global_war_support", 550_000)],
            &[],
        ),
        scope_pair(
            d51c::SCOPE_COUNTRY,
            12,
            &[("alpha_counter", 1), ("beta_counter", -7)],
            &[(9001, 4_812_000)],
        ),
        scope_pair(d51c::SCOPE_COUNTRY, 40, &[("alpha_counter", 900_000)], &[]),
        scope_pair(
            d51c::SCOPE_STATE,
            412,
            &[("state_var", 3)],
            &[(9002, 4_812_001)],
        ),
        scope_pair(d51c::SCOPE_STATE, 413, &[], &[]),
    ];
    domain51::A13FrameInput {
        elapsed_wall_ms: 4,
        executable_sha256:
            domain51::scripted_state_domain51_collector::EXACT_A729_EXECUTABLE_SHA256,
        game_hour_raw: 4_812_000,
        observed_game_hour_before: 4_812_000,
        observed_game_hour_after: 4_812_000,
        source_sequence: 9,
        variable_scopes: pairs.iter().map(|(v, _)| v.clone()).collect(),
        flag_scopes: pairs.iter().map(|(_, f)| f.clone()).collect(),
    }
}

#[test]
fn the_rows_only_shape_changes_the_payload_field_and_nothing_else() {
    let encoded =
        domain51::collect_a13_domain51_frame_shaped(a13_input(), domain51::PayloadShape::Encoded)
            .expect("the fixture roster is complete");
    let rows_only =
        domain51::collect_a13_domain51_frame_shaped(a13_input(), domain51::PayloadShape::RowsOnly)
            .expect("the fixture roster is complete");

    assert!(rows_only.payload.is_empty());
    assert!(!encoded.payload.is_empty());
    assert_eq!(rows_only.rows, encoded.rows);
    assert_eq!(rows_only.row_count, encoded.row_count);
    assert_eq!(rows_only.status_flags, encoded.status_flags);
    assert_eq!(rows_only.observed_scopes, encoded.observed_scopes);
    assert_eq!(rows_only.measured_scopes, encoded.measured_scopes);
    assert_eq!(rows_only.rejected_scopes, encoded.rejected_scopes);
    assert_eq!(rows_only.elapsed_wall_ms, encoded.elapsed_wall_ms);
    assert_eq!(rows_only.domain_id, encoded.domain_id);
    assert_eq!(
        rows_only.validated_scope_partial_failure,
        encoded.validated_scope_partial_failure
    );

    // The deferred path rebuilds through exactly this helper, and it has to
    // land on the bytes the eager encode would have produced.
    assert_eq!(domain51_payload_from_frame(&rows_only), encoded.payload);
}

/// The pre-lane-SE delta selection, restated statement for statement: build
/// the whole baseline map from cloned rows first, then walk the rows a second
/// time to pick the changed ones, then append the deleted keys.
fn reference_delta_selection(
    rows: &[d51c::ScriptedStateRow],
    last_emitted: &BTreeMap<Domain51RowKey, d51c::ScriptedStateRow>,
) -> (
    Vec<d51c::ScriptedStateRow>,
    BTreeMap<Domain51RowKey, d51c::ScriptedStateRow>,
) {
    let mut current = BTreeMap::new();
    for row in rows {
        if let Some(key) = domain51_row_key(row) {
            current.insert(key, row.clone());
        }
    }
    let mut emitted = Vec::new();
    for row in rows {
        if let Some(key) = domain51_row_key(row) {
            let changed = last_emitted.get(&key).map(|old| old != row).unwrap_or(true);
            if changed {
                emitted.push(row.clone());
            }
        } else if domain51_full_emit_in_delta(row) {
            emitted.push(row.clone());
        }
    }
    for key in last_emitted.keys() {
        if !current.contains_key(key) {
            emitted.push(domain51_deleted_row(key));
        }
    }
    emitted.sort_by(d51c::canonical_row_cmp);
    (emitted, current)
}

/// A collected frame in the shape the reader hands to the emit-mode step.
///
/// `payload_shape` MUST match the `payload_deferred` flag the test then
/// passes: `Encoded` is what the reader produces when it did not defer, and
/// its payload has to survive the priming path untouched, which is only
/// checkable if it is a real payload and not an empty `Vec`.
fn frame_from_rows(
    rows: Vec<d51c::ScriptedStateRow>,
    payload_shape: domain51::PayloadShape,
) -> domain51::Domain51CollectedFrame {
    let mut rows = rows;
    rows.sort_by(d51c::canonical_row_cmp);
    let row_count = rows.len() as u32;
    let mut frame = domain51::Domain51CollectedFrame {
        domain_id: d51c::DOMAIN_ID,
        elapsed_wall_ms: 4,
        payload: Vec::new(),
        rows,
        row_count,
        status_flags: 0,
        observed_scopes: 8,
        measured_scopes: 8,
        rejected_scopes: 0,
        validated_scope_partial_failure: false,
    };
    if payload_shape == domain51::PayloadShape::Encoded {
        frame.payload = domain51_payload_from_frame(&frame);
        assert!(!frame.payload.is_empty());
    }
    frame
}

#[test]
fn the_one_pass_delta_emits_what_the_two_pass_shape_emitted() {
    // Second frame against a seeded baseline, exercising all four outcomes at
    // once: an unchanged row (dropped), a changed row (emitted), a brand new
    // row (emitted), and a vanished row (emitted as a deletion).
    let mut baseline_rows = mixed_rows();
    baseline_rows.sort_by(d51c::canonical_row_cmp);
    let (_, baseline) = reference_delta_selection(&baseline_rows, &BTreeMap::new());

    let mut next_rows = mixed_rows();
    next_rows.retain(|row| row.key.as_deref() != Some("beta_counter"));
    for row in next_rows.iter_mut() {
        if row.key.as_deref() == Some("state_var") {
            row.value_raw = Some(99);
        }
    }
    next_rows.push(variable_row(d51c::SCOPE_COUNTRY, 40, "gamma_counter", 5));
    // The production reader hands the emit-mode step canonically sorted rows;
    // the reference half must see the same sequence.
    next_rows.sort_by(d51c::canonical_row_cmp);

    let (expected_emitted, expected_baseline) = reference_delta_selection(&next_rows, &baseline);

    let mut tracker = CompactScriptedStateTracker {
        topology: None,
        last_emitted: baseline,
    };
    let read = compact_scripted_state_apply_emit_mode(
        CompactScriptedStateRead {
            frame: frame_from_rows(next_rows.clone(), domain51::PayloadShape::RowsOnly),
            read_time_us: 0,
            status_flags: 0,
            country_roster_failure: None,
        },
        D51EmitMode::Delta,
        &mut tracker,
        true,
    );

    assert_eq!(read.frame.rows, expected_emitted);
    assert_eq!(read.frame.row_count, expected_emitted.len() as u32);
    assert_eq!(tracker.last_emitted, expected_baseline);
    assert!(read.status_flags & COMPACT_STATUS_DELTA_DATA != 0);
    assert!(read.frame.status_flags & COMPACT_STATUS_DELTA_DATA != 0);
    assert_eq!(read.frame.payload, domain51_payload_from_frame(&read.frame));
    assert!(
        expected_emitted
            .iter()
            .any(|row| row.value_kind & d51c::ROW_VALUE_KIND_DELETED != 0),
        "the fixture must produce a deletion or it proves nothing about that arm"
    );
}

#[test]
fn the_first_delta_frame_still_publishes_the_whole_table_and_seeds_the_baseline() {
    let rows = mixed_rows();
    let (_, expected_baseline) = reference_delta_selection(&rows, &BTreeMap::new());
    let frame = frame_from_rows(rows.clone(), domain51::PayloadShape::Encoded);
    let whole_table = frame.rows.clone();
    let whole_table_payload = frame.payload.clone();

    let mut tracker = CompactScriptedStateTracker::default();
    let read = compact_scripted_state_apply_emit_mode(
        CompactScriptedStateRead {
            frame,
            read_time_us: 0,
            status_flags: 0,
            country_roster_failure: None,
        },
        D51EmitMode::Delta,
        &mut tracker,
        false,
    );

    assert_eq!(read.frame.rows, whole_table);
    assert_eq!(read.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(tracker.last_emitted, expected_baseline);
    // kills: deferring the payload on the priming frame. This is the exit that
    // publishes the WHOLE TABLE and does not re-encode, so it is the one path
    // where `PayloadShape::RowsOnly` would reach a publish with nothing in it.
    // Asserting the exact bytes, not merely non-emptiness, also catches the
    // priming frame publishing something other than the eager encode.
    assert!(!read.frame.payload.is_empty());
    assert_eq!(read.frame.payload, whole_table_payload);
    assert_eq!(
        read.frame.payload,
        d51c::encode_payload(&test_header(&whole_table), &whole_table)
    );
}
