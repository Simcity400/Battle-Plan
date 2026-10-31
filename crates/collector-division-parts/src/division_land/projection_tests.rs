use super::*;
use collector_projection::projection::ProjectionScratch;
use replay_data::rows::TableId;
use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};

#[test]
fn land_projection_keeps_frame_bytes_without_cloning_unchanged_contributors() {
    let frame = |hour| FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            crate::table_conversion::unavailable("fixture table").into()
        }),
        metrics: Metrics::default(),
    };
    for force in [false, true] {
        let mut capture = Captured::new(5_000);
        capture.rows.outcome = Outcome::Rows { len: 5_000 };
        capture.used = 10_000;
        for index in 0..5_000 {
            capture.rows.rows[index] = native::Row {
                division: index as u64 + 1,
                start: index * 2,
                len: 2,
                naval_transfer: index % 2 == 0,
            };
            capture.contributors[index * 2] = native::Contributor {
                country: 1,
                amount: -3,
            };
            capture.contributors[index * 2 + 1] = native::Contributor {
                country: 1,
                amount: 7,
            };
        }
        let scratch = ProjectionScratch::default();
        let mut optimized = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        for hour in 0..=9 {
            match hour {
                2 => capture.rows.rows.reverse(),
                3 => capture.contributors[0].amount = i32::MAX,
                4 => capture.rows.outcome = Outcome::Rows { len: 4_999 },
                5 => capture.rows.rows[1] = capture.rows.rows[0],
                6 => capture.rows.rows[0].start = usize::MAX,
                7 => {
                    capture.rows.outcome = Outcome::Unavailable {
                        reason: "fixture refusal",
                    }
                }
                8 => {
                    capture.rows.outcome = Outcome::Truncated {
                        len: 2,
                        unread_rows: 4_998,
                    }
                }
                9 => capture.rows.outcome = Outcome::Rows { len: 5_001 },
                _ => {}
            }
            let mut projection = Projection::new(&optimized, 1, hour, force, &scratch);
            let actual = if !force && hour == 1 {
                collector_test_support::with_allocations_denied(|| {
                    capture.table_input(Some(&mut projection))
                })
            } else {
                capture.table_input(Some(&mut projection))
            };
            if !force && matches!(hour, 1 | 2) {
                assert!(matches!(actual, TableInput::ComparedEqual(_)));
            }
            if !force && hour == 3 {
                let TableInput::SliceUpdate { update, .. } = &actual else {
                    panic!("one contributor change must not clone the census");
                };
                assert_eq!(update.keys.len(), 1);
                assert_eq!(update.rows().len(), 1);
            }
            if !force && hour == 4 {
                // A removed division is one deleted key, not a converted census.
                let TableInput::SliceUpdate { update, .. } = &actual else {
                    panic!("a removed division must not convert the census");
                };
                assert_eq!(update.keys.len(), 1);
                assert_eq!(update.rows().len(), 0);
            }
            let mut next = frame(hour);
            let mut control = frame(hour);
            next.tables[TableId::DivisionLandInputs.index()] = actual;
            control.tables[TableId::DivisionLandInputs.index()] = capture.table().into();
            assert_eq!(
                optimized.encode_input(next, force).unwrap(),
                reference.encode_input(control, force).unwrap(),
                "hour {hour}, force {force}"
            );
        }
    }
}
