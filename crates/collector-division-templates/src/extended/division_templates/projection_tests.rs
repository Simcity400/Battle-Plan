use super::*;
use collector_projection::projection::ProjectionScratch;
use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};

#[test]
fn template_dictionary_projection_keeps_frame_bytes_and_same_frame_names_at_scale() {
    for force in [false, true] {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 5_000,
            icon_rows: 5_000,
            bytes: 6,
            name_bytes: 6,
            ..Default::default()
        });
        capture.sample_hour = Some(0);
        capture.rows.outcome = Outcome::Rows { len: 5_000 };
        capture.icon_rows.outcome = Outcome::Rows { len: 5_000 };
        capture.country_rows.outcome = Outcome::Rows { len: 0 };
        for arena in [&mut capture.bytes, &mut capture.name_bytes] {
            arena.first[..6].copy_from_slice(b"A\0\xffk\0B");
            arena.len = 6;
        }
        let key = native::Span { start: 3, len: 2 };
        for index in 0..5_000 {
            let name = native::Span {
                start: 0,
                len: if index < 2_500 { 3 } else { 0 },
            };
            capture.rows.rows[index] = native::Row {
                template: index as u64 + 1,
                owner: index as u32 + 1,
                name,
                name_key: key,
                unit_type: index as i32,
            };
            capture.icon_rows.rows[index] = native::IconRow {
                template: index as u64 + 1,
                display_name: Some(native::DisplayName {
                    literal: name,
                    fallback_key: (name.len == 0).then_some(key),
                }),
                ..Default::default()
            };
        }
        let scratch = ProjectionScratch::default();
        let mut optimized = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        for hour in 0..=10 {
            match hour {
                2 => capture.rows.rows[0].name = native::Span { start: 5, len: 1 },
                3 => {
                    capture.icon_rows.rows[0]
                        .display_name
                        .as_mut()
                        .unwrap()
                        .literal = native::Span { start: 5, len: 1 }
                }
                4 => capture.rows.rows[0].name_key = native::Span { start: 5, len: 1 },
                5 => {
                    capture.rows.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 4_999,
                    }
                }
                6 => {
                    capture.rows.outcome = Outcome::Rows { len: 5_000 };
                    capture.rows.rows[0].name = native::Span {
                        start: usize::MAX,
                        len: 1,
                    };
                }
                7 => capture.rows.outcome = Outcome::Rows { len: 5_001 },
                8 => {
                    capture.rows.outcome = Outcome::Unavailable {
                        reason: "fixture refusal",
                    }
                }
                9 => {
                    capture.rows.outcome = Outcome::Rows { len: 5_000 };
                    capture.rows.rows[0].name = native::Span { start: 5, len: 1 };
                }
                10 => {
                    capture.icon_rows.rows[0].priority_unit_name = native::Span {
                        start: usize::MAX,
                        len: 1,
                    }
                }
                _ => {}
            }
            let mut projection = Projection::new(&optimized, 1, hour, force, &scratch);
            let actual = capture.projected_inputs(Some(&mut projection));
            if !force && hour == 1 {
                assert!(matches!(actual[0], TableInput::ComparedEqual(_)));
                assert!(matches!(actual[1], TableInput::ComparedEqual(_)));
            }
            if !force && hour == 2 {
                let TableInput::ComparedChanges { indices, .. } = &actual[0] else {
                    panic!("only changed dictionary rows must reach the encoder");
                };
                assert_eq!(indices, &[0]);
            }
            let expected = capture.table_inputs();
            let frame = |inputs: [TableInput; N]| {
                let mut frame = FrameInput {
                    branch: 1,
                    hour,
                    tables: std::array::from_fn(|_| unavailable("fixture table").into()),
                    metrics: Metrics::default(),
                };
                for (table, input) in TABLES.into_iter().zip(inputs) {
                    frame.tables[table.index()] = input;
                }
                frame
            };
            assert_eq!(
                optimized.encode_input(frame(actual), force).unwrap(),
                reference.encode_input(frame(expected), force).unwrap(),
                "hour {hour}, force {force}"
            );
        }
    }
}

#[test]
fn a_witnessed_template_hour_compares_equal_unprojected_even_after_a_change_hour() {
    let mut capture = Captured::with_capacities(&Capacities {
        rows: 2,
        icon_rows: 2,
        bytes: 6,
        name_bytes: 6,
        ..Default::default()
    });
    capture.sample_hour = Some(0);
    capture.rows.outcome = Outcome::Rows { len: 2 };
    capture.icon_rows.outcome = Outcome::Rows { len: 2 };
    capture.country_rows.outcome = Outcome::Rows { len: 0 };
    for arena in [&mut capture.bytes, &mut capture.name_bytes] {
        arena.first[..6].copy_from_slice(b"A\0\xffk\0B");
        arena.len = 6;
    }
    let name = native::Span { start: 0, len: 3 };
    let key = native::Span { start: 3, len: 2 };
    for index in 0..2 {
        capture.rows.rows[index] = native::Row {
            template: index as u64 + 1,
            owner: 1,
            name,
            name_key: key,
            unit_type: 0,
        };
        capture.icon_rows.rows[index] = native::IconRow {
            template: index as u64 + 1,
            display_name: Some(native::DisplayName {
                literal: name,
                fallback_key: None,
            }),
            ..Default::default()
        };
    }
    let scratch = ProjectionScratch::default();
    let mut optimized = FrameEncoder::new();
    let mut reference = FrameEncoder::new();
    for (hour, held, republished) in [
        (0, true, 0),
        (1, true, 1),
        (2, false, 1),
        (3, true, 1),
        (4, true, 2),
    ] {
        if hour == 2 {
            capture.rows.rows[0].name = native::Span { start: 5, len: 1 };
        }
        capture.witness = held.then(|| Witness::new([1, 2]));
        let projection = Projection::new(&optimized, 1, hour, false, &scratch);
        let actual = capture.projected_inputs(Some(&mut projection.clone()));
        for table in TABLES {
            assert_eq!(
                scratch.republish_counts(table)[0],
                republished,
                "hour {hour}"
            );
        }
        let compared = actual
            .iter()
            .filter(|input| matches!(input, TableInput::ComparedEqual(_)))
            .count();
        match hour {
            1 | 4 => assert_eq!(compared, N, "hour {hour}"),
            2 => {
                let TableInput::ComparedChanges { indices, .. } = &actual[0] else {
                    panic!("a change hour keeps sparse row replacements");
                };
                assert_eq!(indices, &[0]);
            }
            3 => assert_eq!(compared, 2),
            _ => {}
        }
        let frame = |inputs: [TableInput; N]| {
            let mut frame = FrameInput {
                branch: 1,
                hour,
                tables: std::array::from_fn(|_| unavailable("fixture table").into()),
                metrics: Metrics::default(),
            };
            for (table, input) in TABLES.into_iter().zip(inputs) {
                frame.tables[table.index()] = input;
            }
            frame
        };
        assert_eq!(
            optimized.encode_input(frame(actual), false).unwrap(),
            reference
                .encode_input(frame(capture.table_inputs()), false)
                .unwrap(),
            "hour {hour}"
        );
    }
}
