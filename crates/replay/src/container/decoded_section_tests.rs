//! Section work shared by preparation, sampled queries and watched reads.
use super::*;
use crate::{
    container::{BlockWatch, Keep, Watching, test_work::table_decodes},
    rows::{
        CountryRow, DivisionRow, LexicalTokenRow, Recorded, ScriptVariableEntry, ScriptVariablesRow,
    },
};
use std::sync::Barrier;

fn fixture() -> Replay {
    test_recording::build(
        b"synthetic-shared-sections",
        &[1, 2, 4, 5, 721, 722],
        |hour, tables| {
            tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=4)
                    .filter(|id| *id != 4 || hour < 4)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("synthetic-{id}").into_bytes(),
                        command_power_raw: i64::from(hour),
                        ..Default::default()
                    })
                    .collect(),
            ));
            tables[TableId::Divisions.index()] =
                TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                    id: 1,
                    roster_owner: 1,
                    logical_owner: 1,
                    province: hour as u32,
                    hit_points: i64::from(hour) * 10_000,
                    organisation: i64::from(hour) * 10_000,
                    ..Default::default()
                }]));
            // Exercise both a same-frame organisation alias and its literal
            // fallback without making province membership or panel stats sources.
            tables[TableId::DivisionStatus.index()] =
                TableView::Complete(TableRows::DivisionStatus(vec![
                    crate::rows::DivisionStatusRow {
                        division: 1,
                        organisation_exact: (hour != 4).then_some(i64::from(hour) * 10_000).into(),
                        ..Default::default()
                    },
                ]));
            tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 1,
                    literal: format!("token-{hour}").into_bytes(),
                }]));
            tables[TableId::ScriptVariables.index()] =
                TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
                    scope_kind: 1,
                    scope_id: 1,
                    seed: Recorded::Value(Some(hour as u32)),
                    entries: vec![ScriptVariableEntry {
                        name: b"synthetic".to_vec(),
                        value_raw: i64::from(hour),
                    }],
                }]));
            let mut metrics = Metrics::default();
            for table in [
                TableId::Countries,
                TableId::LexicalTokens,
                TableId::ScriptVariables,
            ] {
                metrics.table_ns[table.index()] = u64::from(hour % 2 == 0);
            }
            metrics
        },
    )
}

fn encoded(frames: &[Frame]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .map(|frame| {
            let mut bytes = Vec::new();
            frame.encode(&mut bytes).unwrap();
            bytes
        })
        .collect()
}

fn checkpoint(frame: &DecodedFrame) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame.encode_checkpoint(&mut bytes).unwrap();
    bytes
}

#[test]
fn consumer_selections_share_one_decode_and_keep_identical_wire_and_reconstruction() {
    let oracle = fixture();
    let replay = Replay::open(oracle.bytes().to_vec()).unwrap();
    replay.cache_decoded_sections(true);
    let tables = [
        TableId::Campaign,
        TableId::Countries,
        TableId::Divisions,
        TableId::LexicalTokens,
        TableId::ScriptVariables,
    ];
    for index in &replay.index {
        let mut expected = Vec::new();
        oracle
            .read_block_alone(
                index,
                &tables,
                |_, frame| {
                    expected.push(checkpoint(frame));
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        let selections = [
            &tables[..],
            &[TableId::Countries, TableId::Divisions][..],
            &[TableId::LexicalTokens, TableId::ScriptVariables][..],
        ];
        let raw: Vec<_> = selections
            .iter()
            .map(|selection| oracle.read_block_selected(index, selection).unwrap())
            .collect();
        let ((watched, repeated), decodes) = table_decodes(|| {
            let mut watched = Vec::new();
            let mut kept = Vec::new();
            {
                let mut watches = [BlockWatch::new(Watching::Every(&tables), |step| {
                    let crate::container::Watched::Hour { frame, .. } = step else {
                        unreachable!()
                    };
                    watched.push(checkpoint(frame));
                    Ok(())
                })];
                replay
                    .read_block_watched_keeping(
                        index,
                        &mut watches,
                        Some(Keep {
                            tables: &tables,
                            frames: &mut |frames| kept = frames.to_vec(),
                        }),
                        || Ok(()),
                    )
                    .unwrap();
                watches.into_iter().next().unwrap().finish().unwrap();
            }
            for (kept, expected) in kept.iter().zip(&raw[0]) {
                assert_eq!(kept.metrics, expected.metrics);
                assert_eq!(kept.hour, expected.hour);
                for table in tables {
                    assert_eq!(kept.tables[table.index()], expected.tables[table.index()]);
                }
            }
            for (selection, expected) in selections.iter().zip(&raw) {
                let frames = replay.read_block_selected(index, selection).unwrap();
                assert_eq!(encoded(&frames), encoded(expected));
                let keyframe = replay.read_keyframe_selected(index, selection).unwrap();
                assert_eq!(encoded(&[keyframe]), encoded(&expected[..1]));
                let prefix = replay
                    .read_block_selected_through(index, Some(selection), index.first_hour + 1, None)
                    .unwrap();
                let count = expected.partition_point(|frame| frame.hour <= index.first_hour + 1);
                assert_eq!(encoded(&prefix), encoded(&expected[..count]));
            }
            let mut repeated = Vec::new();
            replay
                .read_block_alone(
                    index,
                    &tables,
                    |_, frame| {
                        repeated.push(checkpoint(frame));
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap();
            (watched, repeated)
        });
        assert_eq!(watched, expected);
        assert_eq!(repeated, expected);
        // Only the conditional organisation source is needed. Its absent exact
        // operand at one hour keeps the literal without opening unrelated sources.
        for table in tables.into_iter().chain([TableId::DivisionStatus]) {
            assert_eq!(
                decodes.iter().filter(|row| row.2 == table).count(),
                1,
                "{table:?} must decode once across the consumer group: {decodes:?}"
            );
        }
        assert_eq!(
            decodes.len(),
            tables.len() + 1,
            "unread sections must not decode"
        );
    }
}

#[test]
fn short_reads_reuse_their_prefix_without_reading_or_certifying_later_entries() {
    let replay = fixture();
    replay.cache_decoded_sections(true);
    let index = &replay.index[0];
    let tables = [TableId::Clock];
    let (_, decodes) = table_decodes(|| {
        for _ in 0..2 {
            let prefix = replay
                .read_block_selected_through(index, Some(&tables), 2, None)
                .unwrap();
            assert_eq!(
                prefix.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
                [1, 2]
            );
            assert!(!replay.is_verified(index));
            replay.read_keyframe_selected(index, &tables).unwrap();
        }
    });
    assert_eq!(decodes, vec![(1, 1, TableId::Clock, 2)]);
    let (_, decodes) = table_decodes(|| {
        replay.read_block_selected(index, &tables).unwrap();
        replay.read_keyframe_selected(index, &tables).unwrap();
        replay
            .read_block_selected_through(index, Some(&tables), 2, None)
            .unwrap();
    });
    assert_eq!(decodes, vec![(1, 1, TableId::Clock, 4)]);
    assert!(replay.is_verified(index));
}

#[test]
fn simultaneous_consumers_decode_each_section_once() {
    let replay = fixture();
    let tables = [
        TableId::Campaign,
        TableId::Countries,
        TableId::LexicalTokens,
    ];
    let expected = encoded(
        &replay
            .read_block_selected(&replay.index[0], &tables)
            .unwrap(),
    );
    replay.cache_decoded_sections(true);
    let gate = Barrier::new(8);
    let decodes: Vec<_> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    gate.wait();
                    let (frames, decodes) = table_decodes(|| {
                        replay
                            .read_block_selected(&replay.index[0], &tables)
                            .unwrap()
                    });
                    assert_eq!(encoded(&frames), expected);
                    decodes
                })
            })
            .collect();
        threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect()
    });
    assert_eq!(
        decodes.len(),
        tables.len(),
        "including Campaign: {decodes:?}"
    );
    for table in [
        TableId::Campaign,
        TableId::Countries,
        TableId::LexicalTokens,
    ] {
        assert_eq!(decodes.iter().filter(|row| row.2 == table).count(), 1);
    }
}

#[test]
fn keyframe_consumers_share_only_the_requested_keyframe_sections() {
    let oracle = fixture();
    let replay = Replay::open(oracle.bytes().to_vec()).unwrap();
    replay.cache_decoded_sections(true);
    let index = &replay.index[0];
    let ((first, second), decodes) = table_decodes(|| {
        (
            replay
                .read_keyframe_selected(index, &[TableId::ScriptVariables])
                .unwrap(),
            replay
                .read_keyframe_selected(index, &[TableId::ScriptVariables])
                .unwrap(),
        )
    });
    let expected = oracle
        .read_keyframe_selected(index, &[TableId::ScriptVariables])
        .unwrap();
    assert_eq!(
        encoded(&[first, second]),
        encoded(&[expected.clone(), expected])
    );
    assert_eq!(decodes, vec![(1, 1, TableId::ScriptVariables, 1)]);
    assert!(!replay.is_verified(index));
}

#[test]
fn immutable_readers_share_update_cells_without_copying_them() {
    let replay = fixture();
    replay.cache_decoded_sections(true);
    let index = &replay.index[0];
    let tables = [
        TableId::Countries,
        TableId::Divisions,
        TableId::LexicalTokens,
    ];
    let ((first, second), decodes) = table_decodes(|| {
        (
            replay.read_block_recorded_selected(index, &tables).unwrap(),
            replay.read_block_recorded_selected(index, &tables).unwrap(),
        )
    });
    for (first, second) in first.iter().zip(&second) {
        for table in tables {
            assert!(
                std::ptr::eq(&first.tables[table.index()], &second.tables[table.index()]),
                "{table:?}: a hit shares the update, not a deep copy"
            );
        }
    }
    for table in tables.into_iter().chain([TableId::DivisionStatus]) {
        assert_eq!(decodes.iter().filter(|row| row.2 == table).count(), 1);
    }
    assert_eq!(decodes.len(), tables.len() + 1);
}

thread_local! {
    static ADMISSIONS: std::cell::RefCell<Option<Vec<TableId>>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn admitted(table: TableId) {
    ADMISSIONS.with_borrow_mut(|counts| {
        if let Some(counts) = counts {
            counts.push(table);
        }
    });
}
fn admissions<T>(run: impl FnOnce() -> T) -> (T, Vec<TableId>) {
    struct Reset(Option<Vec<TableId>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ADMISSIONS.with_borrow_mut(|counts| *counts = self.0.take());
        }
    }
    let _reset = Reset(ADMISSIONS.with_borrow_mut(|counts| counts.replace(Vec::new())));
    let result = run();
    (
        result,
        ADMISSIONS.with_borrow_mut(|counts| counts.take().unwrap()),
    )
}
#[test]
fn each_reader_admits_each_table_once_and_cache_values_ignore_selection() {
    let replay = fixture();
    replay.cache_decoded_sections(true);
    let index = &replay.index[0];
    for selection in [
        &[TableId::Countries][..],
        &[][..],
        &[TableId::LexicalTokens, TableId::Countries][..],
    ] {
        let (_, counts) = admissions(|| {
            replay
                .read_block_recorded_selected(index, selection)
                .unwrap()
        });
        assert_eq!(
            counts,
            TableId::ALL,
            "ordinary selected and unselected admission"
        );
        let block = replay.block_bytes(index).unwrap();
        let (_, counts) = admissions(|| {
            read_selected_updates(&replay, index, &block, selection, &mut || Ok(())).unwrap()
        });
        assert_eq!(
            counts,
            TableId::ALL,
            "watched selected and unselected admission"
        );
    }
    let first = replay
        .read_block_recorded_selected(index, &[TableId::Countries])
        .unwrap();
    let second = replay
        .read_block_recorded_selected(index, &[TableId::LexicalTokens, TableId::Countries])
        .unwrap();
    for (first, second) in first.iter().zip(&second) {
        assert!(std::ptr::eq(
            &first.tables[TableId::Countries.index()],
            &second.tables[TableId::Countries.index()]
        ));
        assert!(
            matches!(&first.tables[TableId::LexicalTokens.index()], TableUpdate::Unavailable { reason }
            if reason == "Table not selected for reconstruction")
        );
        assert!(matches!(
            &second.tables[TableId::LexicalTokens.index()],
            TableUpdate::Snapshot(_) | TableUpdate::Delta { .. } | TableUpdate::Unchanged { .. }
        ));
    }
}

#[test]
fn selected_and_unselected_admission_preserve_absence_and_layout_gap_checks() {
    let mut checked = [false; 4];
    for version in crate::rows::compatibility::PROVEN_FROM..=FormatVersion::CURRENT.version() {
        let Ok(format) = FormatVersion::read(version) else {
            continue;
        };
        for mode in [CaptureMode::Full, CaptureMode::Map]
            .into_iter()
            .filter(|mode| {
                *mode == CaptureMode::Full
                    || version >= crate::rows::compatibility::CAPTURE_MODE_FORMAT
            })
        {
            for table in TableId::ALL {
                let change = &HISTORY[table.index()];
                let present = change.present(version);
                let recorded = scope::records(mode, table);
                let gap = change.table_gap(table, version);
                checked[if !present {
                    0
                } else if !recorded {
                    1
                } else if gap.is_some() {
                    2
                } else {
                    3
                }] = true;
                let mut directory = Directory {
                    frames: 1,
                    layout: Layout::Chosen,
                    sections: vec![None; SECTIONS],
                    chunks: vec![Vec::new(); SECTIONS],
                };
                for selection in [None, Some(&[][..]), Some(std::slice::from_ref(&table))] {
                    let (result, counts) =
                        admissions(|| keyframe_refusal(&directory, format, mode, selection, table));
                    assert_eq!(counts, [table]);
                    let result = result.unwrap();
                    if !present {
                        assert_eq!(
                            result,
                            Some(TableUpdate::Unavailable {
                                reason: gap.clone().unwrap()
                            })
                        );
                    } else if !recorded {
                        assert_eq!(result, Some(TableUpdate::NotRecorded { mode }));
                    } else if gap.is_none() && selection.is_some_and(|tables| tables.is_empty()) {
                        assert!(
                            matches!(result, Some(TableUpdate::Unavailable { reason }) if reason == "Table not selected for reconstruction")
                        );
                    } else {
                        assert!(
                            result.is_none(),
                            "{table:?} at {version} in {mode}: selected tables and present layout gaps must reach the selection-independent decoder"
                        );
                    }
                }
                if !present || !recorded {
                    for section in [keyframe_section(table), later_section(table)] {
                        directory.sections[section] = Some((0, 1, 1));
                        for selection in [None, Some(&[][..]), Some(std::slice::from_ref(&table))] {
                            assert!(
                                keyframe_refusal(&directory, format, mode, selection, table)
                                    .is_err(),
                                "stored absent {table:?} at {version} in {mode} must refuse even when omitted"
                            );
                        }
                        directory.sections[section] = None;
                    }
                }
            }
        }
    }
    assert_eq!(
        checked, [true; 4],
        "exercise absent, out-of-scope, present gap and current tables"
    );
}
