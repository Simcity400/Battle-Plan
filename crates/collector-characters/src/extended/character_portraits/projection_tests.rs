use super::*;
use collector_projection::projection::ProjectionScratch;
use replay_data::rows::{CharacterPortrait, TableRows};
use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};

#[test]
fn retained_portrait_order_keeps_frame_bytes() {
    for force in [false, true] {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 11,
            arena: 11,
        });
        for (part, share) in capture.parts.iter_mut().enumerate() {
            share.rows.outcome = Outcome::Rows { len: 10 };
            share.arena.len = 10;
            for index in 0..10 {
                share.rows.rows[index] = ccharacter::PortraitRow {
                    character: (index * PARTS + part + 1) as u32,
                    portraits: [index, 1],
                };
                let entry = &mut share.arena.first[index];
                entry.category = 1;
                entry.size = 2;
                entry.kind = 1;
                entry.is_set = 255;
                entry.identity.bytes = vec![0xff, 0, index as u8];
                entry.identity.len = 3;
            }
        }
        let mut optimized = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        let scratch = ProjectionScratch::default();
        let mut allocation = None;
        for hour in 0..=8 {
            match hour {
                2 => {
                    for part in &mut capture.parts {
                        part.rows.rows[..10].reverse();
                    }
                }
                3 => capture.parts[0].arena.first[0].identity.bytes[0] ^= 1,
                4 => {
                    capture.parts[1].rows.rows[10] = capture.parts[0].rows.rows[9];
                    capture.parts[1].rows.rows[10].portraits = [0, 0];
                    capture.parts[0].rows.outcome = Outcome::Rows { len: 9 };
                    capture.parts[1].rows.outcome = Outcome::Rows { len: 11 };
                }
                5 => {
                    capture.parts[0].rows.rows[0].character =
                        capture.parts[1].rows.rows[0].character
                }
                6 => capture.parts[0].rows.rows[0].portraits = [usize::MAX, 1],
                7 => {
                    capture.parts[0].rows.outcome = Outcome::Unavailable {
                        reason: "fixture refusal",
                    }
                }
                8 => capture.parts[0].rows.outcome = Outcome::Rows { len: 12 },
                _ => {}
            }
            let mut projection = Projection::new(&optimized, 1, hour, force, &scratch);
            let [actual] = if !force && hour == 1 {
                collector_test_support::with_allocations_denied(|| {
                    capture.table_inputs(Some(&mut projection))
                })
            } else {
                capture.table_inputs(Some(&mut projection))
            };
            let storage = capture.ordering.borrow();
            let current = (storage.as_ptr(), storage.capacity());
            if hour < 8 && let Some(previous) = allocation {
                assert_eq!(current, previous, "merge scratch must not grow each hour");
            }
            allocation = Some(current);
            drop(storage);
            if !force && matches!(hour, 1 | 2) {
                assert!(matches!(actual, TableInput::ComparedEqual(_)));
            }
            let expected = match hour {
                5 => unavailable("duplicate character portrait identity"),
                6 => unavailable("character portraits exceed captured arena"),
                7 => unavailable("fixture refusal"),
                8 => unavailable("walker row count exceeds allocated buffer"),
                _ => {
                    let mut rows = Vec::new();
                    for part in &capture.parts {
                        for row in &part.rows.rows[..part.rows.outcome.len()] {
                            rows.push(CharacterPortraitRow {
                                character: row.character,
                                portraits: part.arena.first
                                    [row.portraits[0]..row.portraits[0] + row.portraits[1]]
                                    .iter()
                                    .map(|entry| CharacterPortrait {
                                        category: entry.category,
                                        size: entry.size,
                                        is_set: entry.is_set,
                                        kind: entry.kind,
                                        identity: entry.identity.as_bytes().to_vec(),
                                    })
                                    .collect(),
                            });
                        }
                    }
                    rows.sort_unstable_by_key(|row| row.character);
                    TableView::Complete(TableRows::CharacterPortraits(rows))
                }
            };
            let frame = |input| {
                let mut frame = FrameInput {
                    branch: 1,
                    hour,
                    tables: std::array::from_fn(|_| unavailable("fixture table").into()),
                    metrics: Metrics::default(),
                };
                frame.tables[TABLES[0].index()] = input;
                frame
            };
            assert_eq!(
                optimized.encode_input(frame(actual), force).unwrap(),
                reference
                    .encode_input(frame(expected.into()), force)
                    .unwrap(),
                "hour {hour}, force {force}"
            );
        }
    }
}
