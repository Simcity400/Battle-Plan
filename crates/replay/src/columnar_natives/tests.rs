use super::*;
use crate::{
    campaign::Parallel,
    columnar_frames::{
        fixtures,
        synthetic::{Input, Sample},
    },
    format::FormatVersion,
    frame::{SharedView, TableView},
    rows::{CountryRow, TableId, WorldTensionRow},
    scope::CaptureMode,
};
use replay_columnar::envelope::UNIT_ORDINALS;
use std::sync::Arc;

/// Runs (branch, first hour, units): two of one branch one after the other, and another.
const RUNS: [(u64, i32, u32); 3] = [
    (1, 100, 3),
    (1, 100 + 3 * UNIT_ORDINALS as i32, 2),
    (2, 50_000, 2),
];
const TABLES: [TableId; 2] = [TableId::Countries, TableId::WorldTension];

/// Countries that come and go in many blocks, read in part for a while every few days, and
/// world tension that is sometimes unavailable, a restart ending every `restart_units`th unit.
fn wide(runs: &[(u64, i32, u32)], restart_units: u32) -> (Vec<u8>, Vec<fixtures::Seen>) {
    fixtures::written(
        b"wide synthetic",
        runs,
        restart_units,
        &TABLES,
        |synthetic, _, ordinal| {
            let rows: Vec<CountryRow> = (1..=30_u32)
                .filter(|&id| (id + ordinal / 40) % 7 != 0)
                .map(|id| CountryRow {
                    id,
                    tag: format!("tag {id} {}", (ordinal / 90 + id) % 5).into_bytes(),
                    stability_raw: i64::from(id) * 1_000 + i64::from(ordinal / 3),
                    political_power_raw: (id % 3 != 0).then_some(i64::from(ordinal % 11)),
                    ..Default::default()
                })
                .collect();
            let partial = ordinal % 120 >= 100;
            let read: Vec<CountryRow> = rows
                .iter()
                .filter(|row| !partial || row.id % 2 == 0)
                .cloned()
                .collect();
            synthetic
                .table(
                    TableId::Countries,
                    Input::Snapshot {
                        rows: &read,
                        partial: partial.then_some(3),
                        read: true,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap();
            let block = ordinal / 200;
            let tension = if block % 2 == 0 {
                synthetic.table(
                    TableId::WorldTension,
                    Input::Snapshot {
                        rows: &[WorldTensionRow {
                            total_raw: i64::from(ordinal / 7),
                            source_count: 1,
                            country_aggregate_count: 2,
                        }],
                        partial: None,
                        read: ordinal % 5 == 0,
                        sample: Sample::FromReadBit,
                    },
                )
            } else {
                synthetic.table::<WorldTensionRow>(
                    TableId::WorldTension,
                    Input::Unavailable {
                        reason: &format!("refused {}", block % 3),
                        read: false,
                        sample: Sample::FromReadBit,
                    },
                )
            };
            tension.unwrap();
        },
    )
}

/// `bytes`, a recording, read in place.
fn alone(bytes: Vec<u8>) -> Arc<Natives> {
    Arc::new(Natives::of(Recording::open(bytes).unwrap()).unwrap())
}

/// Every hour of `runs`, in their order, with the branch it is of.
fn hours(runs: &[(u64, i32, u32)]) -> Vec<(u64, i32)> {
    runs.iter()
        .flat_map(|&(branch, first, units)| {
            (0..(units * UNIT_ORDINALS) as i32).map(move |at| (branch, first + at))
        })
        .collect()
}

/// Frames read by a cursor at every hour, in order and then at hours far and near in either
/// direction, moved to without building and built later, alone and with views built on
/// several workers, are the frames the writer saw there: statuses, reasons, truncated views,
/// sample hours, rows that leave and come back, and metrics; whether restarts end every unit
/// or every other. A move stopped part way leaves the cursor where it stopped, and the next
/// move walks on from there to the same frame. An hour no run holds is no frame.
#[test]
fn frames_read_from_the_recording_are_the_writer_s_at_every_hour() {
    for restart_units in [1, 2] {
        for (bytes, expected, runs) in [
            {
                let (bytes, seen) = fixtures::churned(&RUNS, restart_units);
                (bytes, seen, &RUNS[..])
            },
            {
                let (bytes, seen) = wide(&RUNS[..2], restart_units);
                (bytes, seen, &RUNS[..2])
            },
        ] {
            let natives = alone(bytes);
            let hours = hours(runs);
            assert_eq!(hours.len(), expected.len());
            let alone = || {};
            let parallel = Parallel {
                workers: 3,
                on_worker: &alone,
            };
            for parallel in [None, Some(&parallel)] {
                let mut cursor = Cursor::new(
                    Arc::clone(&natives),
                    &TABLES,
                    FormatVersion::CURRENT,
                    CaptureMode::Full,
                )
                .unwrap();
                let check = |at: usize, cursor: &mut Cursor| {
                    let (branch, hour) = hours[at];
                    let frame = cursor.frame(branch, branch, hour, parallel).unwrap().unwrap();
                    let sampled: Vec<Option<i32>> =
                        frame.tables.iter().map(SharedView::sampled_at).collect();
                    assert!(frame == expected[at].0, "branch {branch} hour {hour}");
                    assert_eq!(sampled, expected[at].1, "branch {branch} hour {hour}");
                    let complete = matches!(
                        &*frame.tables[TableId::WorldTension.index()],
                        TableView::Complete(_)
                    );
                    assert_eq!(cursor.complete(TableId::WorldTension).unwrap(), complete);
                };
                for at in 0..hours.len() {
                    check(at, &mut cursor);
                }
                let mut seed = 9_u64;
                for step in 0..500 {
                    seed = seed
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    let at = (seed >> 33) as usize % hours.len();
                    match step % 4 {
                        0 => {
                            // Moves taken without a frame are built into the next one.
                            let (branch, hour) = hours[(at + 1) % hours.len()];
                            assert!(cursor.move_to(branch, branch, hour, &|| false).unwrap());
                        }
                        1 => {
                            // A move stopped part way is taken up by the next.
                            let (branch, hour) = hours[at];
                            let asked = std::cell::Cell::new(0);
                            let stop = || {
                                asked.set(asked.get() + 1);
                                asked.get() > 3
                            };
                            match cursor.move_to(branch, branch, hour, &stop) {
                                Ok(moved) => assert!(moved),
                                Err(error) => {
                                    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted)
                                }
                            }
                        }
                        _ => {}
                    }
                    check(at, &mut cursor);
                }
                assert!(cursor.frame(1, 1, 99, parallel).unwrap().is_none());
                assert!(cursor.frame(3, 3, 100, parallel).unwrap().is_none());
            }
        }
    }
}
