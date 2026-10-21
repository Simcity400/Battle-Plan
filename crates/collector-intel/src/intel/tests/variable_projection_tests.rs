use super::*;
use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};

#[test]
fn variable_scope_updates_keep_frame_bytes_at_five_thousand_scopes() {
    let frame = |hour| FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| unavailable("fixture table").into()),
        metrics: Metrics::default(),
    };
    for force in [false, true] {
        let mut capacity = Capacities::default();
        capacity.raise_for_script_variables(VariableDemand {
            scopes: 5_000,
            entries: 10_000,
            bytes: 8,
        });
        let mut capture = Captured::with_capacities(&capacity);
        capture.variables.outcome = Outcome::Rows { len: 5_000 };
        for index in 0..5_000 {
            capture.variables.rows[index] = cvariables::ScopeRow {
                scope: cvariables::Scope::Country(index as u32 + 1),
                entries_start: index * 2,
                entries_len: 2,
                seed: Some(index as u32),
            };
            capture.variable_entries.first[index * 2] = cvariables::Row {
                name_start: 0,
                name_len: 4,
                value_raw: index as i64,
            };
            capture.variable_entries.first[index * 2 + 1] = cvariables::Row {
                name_start: 4,
                name_len: 2,
                value_raw: -(index as i64),
            };
        }
        capture.variable_entries.len = 10_000;
        capture.variable_entries.required = 10_000;
        capture.variable_names.first[..6].copy_from_slice(b"zetaA\0");
        capture.variable_names.len = 6;
        capture.variable_names.required = 6;
        let mut optimized = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        let mut scratch = ScriptVariableComparison::default();
        for hour in 0..=10 {
            match hour {
                2 => capture.variables.rows[..5_000].reverse(),
                3 => capture.variable_entries.first[0].value_raw = i64::MIN,
                4 => capture.variables.rows[0].seed = None,
                5 => capture.variable_names.first[0] = b'Z',
                6 => capture.variable_entries.first[1] = capture.variable_entries.first[0],
                7 => {
                    capture.variables.outcome = Outcome::Unavailable {
                        reason: "fixture refusal",
                    }
                }
                8 => {
                    capture.variables.outcome = Outcome::Truncated {
                        len: 3,
                        unread_rows: 4_997,
                    }
                }
                9 => {
                    capture.variables.outcome = Outcome::Rows { len: 5_000 };
                    capture.variable_names.len = usize::MAX;
                }
                10 => {
                    capture.variables.outcome = Outcome::Rows {
                        len: capture.variables.rows.len() + 1,
                    }
                }
                _ => {}
            }
            let baseline = optimized.comparison_baseline::<tables::ScriptVariables>(1, hour, force);
            let actual = if !force && hour == 2 {
                // The first successful comparison sized its bitsets at hour 1.
                collector_test_support::with_allocations_denied(|| {
                    capture.variables_input(capture.variables.outcome, baseline, &mut scratch)
                })
            } else {
                capture.variables_input(capture.variables.outcome, baseline, &mut scratch)
            };
            if !force && matches!(hour, 1 | 2) {
                assert!(matches!(actual, TableInput::ComparedEqual(_)));
            }
            if !force && matches!(hour, 3 | 4) {
                let TableInput::ComparedChanges { indices, rows, .. } = &actual else {
                    panic!("one changed scope must not convert all scopes");
                };
                assert_eq!(indices.len(), 1);
                assert_eq!(rows.len(), 1);
            }
            let mut next = frame(hour);
            let mut control = frame(hour);
            next.tables[TableId::ScriptVariables.index()] = actual;
            control.tables[TableId::ScriptVariables.index()] =
                capture.variables_table(capture.variables.outcome).into();
            assert_eq!(
                optimized.encode_input(next, force).unwrap(),
                reference.encode_input(control, force).unwrap(),
                "hour {hour}, force {force}"
            );
        }
    }
}
