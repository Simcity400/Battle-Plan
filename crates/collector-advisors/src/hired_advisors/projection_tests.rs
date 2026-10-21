use super::*;
use collector_projection::projection::ProjectionScratch;
use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};

#[test]
fn advisor_projection_keeps_frame_bytes_and_shared_tokens_at_five_thousand_owners() {
    for force in [false, true] {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 5_000,
            bytes: 8,
            active: 5_000,
            retired: 5_000,
            tokens: 2,
            token_bytes: 8,
            ..Default::default()
        });
        capture.rows.outcome = Outcome::Rows { len: 5_000 };
        capture.active.outcome = Outcome::Rows { len: 5_000 };
        capture.retired.outcome = Outcome::Rows { len: 5_000 };
        capture.bytes.first[..5].copy_from_slice(b"i\0\xffs\0");
        capture.bytes.len = 5;
        for index in 0..5_000 {
            capture.rows.rows[index] = native::Row {
                country: index as u32 + 1,
                ordinal: 0,
                character: index as u32,
                idea_start: 0,
                idea_len: 3,
                slot_start: 3,
                slot_len: 2,
            };
            let row = native::ActiveRow {
                country: index as u32 + 1,
                ordinal: 0,
                character: (index % 2 == 0).then_some(index as u32),
            };
            capture.active.rows[index] = row;
            capture.retired.rows[index] = row;
        }
        capture.tokens.outcome = Outcome::Rows { len: 2 };
        capture.token_bytes.first[..5].copy_from_slice(b"z\0a\0\xff");
        capture.token_bytes.len = 5;
        capture.tokens.rows[0] = advisor_tokens::Row { start: 0, len: 2 };
        capture.tokens.rows[1] = advisor_tokens::Row { start: 2, len: 3 };
        let scratch = ProjectionScratch::default();
        let mut optimized = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        for hour in 0..=10 {
            match hour {
                2 => {
                    capture.rows.rows.reverse();
                    capture.active.rows.reverse();
                    capture.retired.rows.reverse();
                    capture.tokens.rows.swap(0, 1);
                }
                3 => capture.rows.rows[0].character ^= 1,
                4 => capture.token_bytes.first[0] = b'y',
                5 => capture.tokens.rows[1] = capture.tokens.rows[0],
                6 => capture.tokens.rows[0].start = usize::MAX,
                7 => capture.rows.rows[0].idea_start = usize::MAX,
                8 => {
                    capture.active.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 4_999,
                    }
                }
                9 => {
                    capture.retired.outcome = Outcome::Unavailable {
                        reason: "fixture refusal",
                    }
                }
                10 => capture.rows.outcome = Outcome::Rows { len: 5_001 },
                _ => {}
            }
            let mut projection = Projection::new(&optimized, 1, hour, force, &scratch);
            let actual = capture.table_inputs(Some(&mut projection));
            if hour == 1 {
                let TableInput::SharedComplete(rows) = &actual[2] else {
                    panic!("unchanged tokens must retain complete shared ownership");
                };
                assert!(std::sync::Arc::ptr_eq(
                    rows,
                    optimized.shared_baseline(TABLES[2]).unwrap()
                ));
                if !force {
                    assert!(matches!(actual[0], TableInput::ComparedEqual(_)));
                    assert!(matches!(actual[1], TableInput::ComparedEqual(_)));
                    assert!(matches!(actual[3], TableInput::ComparedEqual(_)));
                }
            }
            let expected = capture.tables().map(TableInput::Sampled);
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
