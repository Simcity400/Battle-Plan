use super::reader_reference::ReferenceDecoder;
use super::*;
use crate::container::{ReadStep, ReaderExecution, ReaderSelection, SeekResult, SequentialReader};
use std::sync::Arc;
use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
};

pub(super) fn assert_frame(actual: &DecodedFrame, expected: &DecodedFrame) {
    assert_eq!(actual, expected);
    for table in TableId::ALL {
        assert_eq!(
            actual.tables[table.index()].sampled_at(),
            expected.tables[table.index()].sampled_at(),
            "{table:?} at {}",
            actual.hour
        );
    }
}

pub(super) fn reference_copy(replay: &Replay) -> Replay {
    let mut reference = Replay::open(replay.bytes.clone()).unwrap();
    reference.index = replay.index.clone();
    reference.ordinals = replay.ordinals.clone();
    reference.verified = replay
        .index
        .iter()
        .map(|_| std::sync::atomic::AtomicBool::new(false))
        .collect();
    reference.cache_decoded_sections(false);
    reference
}

pub(super) fn compare(
    replay: Replay,
    tables: &[TableId],
    workers: usize,
    ahead: bool,
) -> crate::container::ReaderCensus {
    let replay = Arc::new(replay);
    let reference_replay = reference_copy(&replay);
    replay
        .verified
        .iter()
        .for_each(|flag| flag.store(false, Ordering::Release));
    replay.cache_decoded_sections(false);
    let selection = ReaderSelection::new(tables).unwrap();
    let mut reader = SequentialReader::new(
        selection.clone(),
        ReaderExecution {
            workers,
            read_ahead: ahead,
        },
    )
    .unwrap();
    let mut decoder = ReferenceDecoder::for_tables(selection.tables());
    let mut seen = 0;
    for (position, index) in replay.index.iter().enumerate() {
        // The old reader retains its original offset loop and alias collector.
        let reference = reference_replay
            .read_block_selected(index, selection.tables())
            .unwrap();
        replay.log_section_decodes();
        for raw in &reference {
            let expected = decoder.advance(raw).unwrap();
            let ReadStep::Hour {
                frame,
                updates,
                ordinal,
                ..
            } = reader.advance(&replay, || Ok(())).unwrap()
            else {
                panic!("missing hour");
            };
            assert_eq!(ordinal, replay.ordinals[position]);
            assert_frame(frame, expected);
            assert_eq!(updates.metrics, raw.metrics);
            for table in selection.tables() {
                assert_eq!(
                    updates.tables[table.index()],
                    raw.tables[table.index()],
                    "{table:?}, hour {}",
                    raw.hour
                );
            }
            seen += 1;
            let census = reader.census();
            assert!(census.raw_blocks <= 2 && census.peak_raw_blocks <= 2);
            assert!(census.anchor_states <= TABLE_COUNT * 2);
            assert_eq!(reader.cursor().next_block, position);
        }
        let ReadStep::BlockEnd { receipt } = reader.advance(&replay, || Ok(())).unwrap() else {
            panic!("no block receipt");
        };
        let receipt = receipt.clone();
        assert_eq!(reader.cursor().next_block, position);
        reader.accept_block(&receipt).unwrap();
        assert!(reader.accept_block(&receipt).is_err());
        assert!(
            replay.section_decodes().is_empty(),
            "streaming must bypass decoded sections"
        );
        assert!(replay.wire_decodes().is_empty());
    }
    assert!(seen != 0);
    assert!(matches!(
        reader.advance(&replay, || Ok(())).unwrap(),
        ReadStep::Tail
    ));
    reader.census()
}

#[test]
fn every_hour_matches_original_for_all_layouts_and_parallelism() {
    for layout in direct_tests::LAYOUTS {
        for (workers, ahead) in [(1, false), (3, true)] {
            let (replay, _) = direct_tests::fixture(layout);
            compare(replay, &TableId::ALL, workers, ahead);
        }
    }
}

#[test]
fn every_private_alias_and_transitive_source_matches_at_every_hour() {
    for layout in direct_tests::LAYOUTS {
        for workers in [1, 3] {
            let (replay, native) = direct_tests::alias_fixture(layout);
            let mut oracle = ReferenceDecoder::new();
            let mut legacy = ReferenceDecoder::new();
            for (actual, expected) in replay
                .index
                .iter()
                .flat_map(|index| replay.read_block(index).unwrap())
                .zip(&native)
            {
                assert_frame(
                    legacy.advance(&actual).unwrap(),
                    oracle.advance(expected).unwrap(),
                );
            }
            compare(replay, private_tables(), workers, workers > 1);
        }
    }
}

fn private_tables() -> &'static [TableId] {
    super::super::aliases::private::TABLES
}

#[test]
fn branches_gaps_empty_truncated_unavailable_and_fifteen_hundred_countries() {
    let build = || {
        test_recording::build_branches(
            b"stream-population",
            &[(1, &[10, 12, 14, 15, 16], &[14]), (2, &[3, 4, 9], &[])],
            |branch, hour, tables| {
                let rows = TableRows::Countries(
                    (0..1500)
                        .filter(|id| hour != 15 || id % 3 != 0)
                        .map(|id| crate::rows::CountryRow {
                            id,
                            canonical_slot: id,
                            tag: format!("mod-{id}").into_bytes(),
                            command_power_raw: branch as i64 * 1000 + i64::from(hour),
                            ..Default::default()
                        })
                        .collect(),
                );
                tables[TableId::Countries.index()] = match hour {
                    12 => TableView::Truncated { rows, missing: 2 },
                    15 => TableView::Unavailable {
                        reason: "synthetic unavailable".into(),
                    },
                    16 => TableView::Complete(TableRows::Countries(Vec::new())),
                    _ => TableView::Complete(rows),
                };
                let mut metrics = Metrics::default();
                if hour % 2 == 0 {
                    metrics.table_ns[TableId::Countries.index()] = 5000;
                }
                metrics
            },
        )
    };
    for workers in [1, 4] {
        compare(
            build(),
            &[TableId::Countries, TableId::Clock],
            workers,
            true,
        );
    }
}

pub(crate) fn rewrite(
    replay: Replay,
    mut change: impl FnMut(usize, usize, &mut Vec<u8>),
) -> Replay {
    let mut bytes = replay.bytes[..replay.index[0].offset as usize].to_vec();
    let mut indexes = Vec::new();
    for (position, index) in replay.index.iter().enumerate() {
        let directory = Directory::read(&replay, index).unwrap();
        let mut replacements = Vec::new();
        for id in 0..SECTIONS {
            if let Some(mut raw) = directory.read_raw(&replay, index, id).unwrap() {
                let before = raw.clone();
                change(position, id, &mut raw);
                if raw != before {
                    replacements.push((id, raw));
                }
            }
        }
        let (mut index, block) = replace_sections(&replay, index, &replacements).unwrap();
        index.offset = bytes.len() as u64;
        bytes.extend_from_slice(&block);
        indexes.push(index);
    }
    super::super::finish_file(
        &mut bytes,
        &replay.sessions,
        &replay.branches,
        &replay.gaps,
        &replay.controls,
        &replay.recovery_notes,
        &indexes,
    )
    .unwrap();
    super::super::commit::stamp_slot(&mut bytes, replay.slot).unwrap();
    let ordinals = replay.ordinals;
    let mut rebuilt = Replay::open(bytes).unwrap();
    rebuilt.ordinals = ordinals;
    rebuilt
}

pub(super) fn append_control(raw: &mut Vec<u8>, table: TableId) {
    let input = ColumnReader::stored(
        raw,
        table.field_count(),
        table,
        FormatVersion::CURRENT.version(),
    )
    .unwrap();
    let mut columns = Columns::new(table.field_count());
    for (out, input) in columns.0.iter_mut().zip(&input.0) {
        out.extend_from_slice(input.rest());
    }
    columns.0[CONTROL].push(255);
    if let Some(marks) = input.1 {
        columns.marks(table, marks).unwrap();
    }
    *raw = columns.raw();
}

#[test]
fn bad_tail_poisoning_never_accepts_tentative_hours() {
    let replay = test_recording::build(b"bad-tail", &[10, 11, 12], |_, _| Metrics::default());
    let replay = Arc::new(rewrite(replay, |_, id, raw| {
        if id == later_section(TableId::Clock) {
            append_control(raw, TableId::Clock);
        }
    }));
    let mut reader = SequentialReader::new(
        ReaderSelection::new(&[TableId::Clock]).unwrap(),
        ReaderExecution {
            workers: 2,
            read_ahead: true,
        },
    )
    .unwrap();
    for _ in 0..3 {
        assert!(matches!(
            reader.advance(&replay, || Ok(())).unwrap(),
            ReadStep::Hour { .. }
        ));
    }
    let error = reader.advance(&replay, || Ok(())).err().unwrap();
    assert_eq!(error.to_string(), "grouped section has unread column bytes");
    assert_eq!(reader.cursor().next_block, 0);
    assert_eq!(reader.census().raw_blocks, 0);
    assert!(reader.current().is_none());
    assert!(reader.advance(&replay, || Ok(())).is_err());
    assert!(!replay.is_verified(&replay.index[0]));
}

#[test]
fn cancellation_releases_workers_and_both_credits_without_accepting() {
    let (replay, _) = direct_tests::fixture(Layout::Chosen);
    let replay = Arc::new(replay);
    let mut reader = SequentialReader::new(
        ReaderSelection::new(&TableId::ALL).unwrap(),
        ReaderExecution {
            workers: 3,
            read_ahead: true,
        },
    )
    .unwrap();
    assert!(matches!(
        reader.advance(&replay, || Ok(())).unwrap(),
        ReadStep::Hour { .. }
    ));
    let error = reader
        .advance(&replay, || Err(io::ErrorKind::Interrupted.into()))
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(reader.cursor().next_block, 0);
    assert_eq!(reader.census().raw_blocks, 0);
    assert!(reader.current().is_none());
}

#[test]
fn credit_is_reserved_before_the_producer_can_allocate_a_third_block() {
    let credits = Arc::new(chunks::Credits::default());
    let current = credits.acquire(2).unwrap();
    let ahead = credits.acquire(2).unwrap();
    assert!(credits.acquire(2).is_err());
    assert_eq!(credits.counts(), (2, 2));
    drop(current);
    let successor = credits.acquire(2).unwrap();
    assert_eq!(credits.counts(), (2, 2));
    drop((ahead, successor));
    assert_eq!(credits.counts(), (0, 2));
}

fn lexical_rows(phase: u8) -> TableRows {
    TableRows::LexicalTokens(
        (0..512)
            .map(|token| crate::rows::LexicalTokenRow {
                token,
                literal: (0..96)
                    .map(|at| {
                        (token as u8)
                            .wrapping_mul(17)
                            .wrapping_add(at)
                            .wrapping_add(if token % 11 == 0 { phase } else { 0 })
                    })
                    .collect(),
            })
            .collect(),
    )
}

pub(crate) fn forced_keyframe(rows: &TableRows, base: Option<(&TableRows, usize)>) -> Vec<u8> {
    let table = TableId::LexicalTokens;
    let mut plain = vec![SNAPSHOT];
    plain.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    rows.encode_rows(&mut plain).unwrap();
    let mut shadow = Shadow::new();
    let mode = if let Some((base, back)) = base {
        let mut snapshot = vec![SNAPSHOT];
        snapshot.extend_from_slice(&(base.len() as u32).to_le_bytes());
        base.encode_rows(&mut snapshot).unwrap();
        advance_shadow(
            table,
            FormatVersion::CURRENT.version(),
            &snapshot,
            &mut shadow,
        )
        .unwrap();
        plain = vec![DELTA];
        plain.extend_from_slice(&1_i32.to_le_bytes());
        if !rows
            .encode_changes(base, &mut plain, &mut Vec::new())
            .unwrap()
        {
            plain.truncate(1);
            plain[0] = UNCHANGED;
            plain.extend_from_slice(&1_i32.to_le_bytes());
        }
        Mode::Back { blocks: back }
    } else {
        Mode::Plain
    };
    let mut columns = Columns::new(table.field_count());
    put_entry(
        table,
        FormatVersion::CURRENT.version(),
        &plain,
        mode,
        &shadow,
        &mut columns,
        None,
    )
    .unwrap();
    columns.raw()
}

fn anchor_fixture() -> Replay {
    let hours: Vec<_> = (0..5).map(|n| 1 + n * 720).collect();
    let replay = test_recording::build(b"forced-anchor", &hours, |_, tables| {
        tables[TableId::LexicalTokens.index()] = TableView::Complete(lexical_rows(0));
        Metrics::default()
    });
    rewrite(replay, |position, id, raw| {
        if id != keyframe_section(TableId::LexicalTokens) {
            return;
        }
        *raw = match position {
            0 => forced_keyframe(&lexical_rows(0), None),
            1 => forced_keyframe(&lexical_rows(1), Some((&lexical_rows(0), 1))),
            2 => forced_keyframe(&lexical_rows(2), Some((&lexical_rows(0), 2))),
            3 => {
                let mut plain = vec![UNAVAILABLE];
                crate::codec::put_string(&mut plain, "synthetic keyframe unavailable").unwrap();
                let mut columns = Columns::new(TableId::LexicalTokens.field_count());
                put_entry(
                    TableId::LexicalTokens,
                    FormatVersion::CURRENT.version(),
                    &plain,
                    Mode::Plain,
                    &Shadow::new(),
                    &mut columns,
                    None,
                )
                .unwrap();
                columns.raw()
            }
            4 => forced_keyframe(&lexical_rows(2), Some((&lexical_rows(2), 2))),
            _ => unreachable!(),
        };
    })
}

#[test]
fn snapshot_and_latest_survive_unavailable_keyframes_without_historical_reads() {
    for workers in [1, 3] {
        compare(anchor_fixture(), &[TableId::LexicalTokens], workers, true);
    }
}

#[test]
fn checkpoint_moves_the_frame_and_both_anchor_candidates_at_an_accepted_boundary() {
    let replay = Arc::new(anchor_fixture());
    let selection = ReaderSelection::new(&[TableId::LexicalTokens]).unwrap();
    let execution = ReaderExecution {
        workers: 2,
        read_ahead: true,
    };
    let mut reader = SequentialReader::new(selection.clone(), execution).unwrap();
    for block in 0..replay.index.len() {
        let raw = replay
            .read_block_selected(&replay.index[block], selection.tables())
            .unwrap();
        let mut decoder = ReferenceDecoder::for_tables(selection.tables());
        let expected = decoder.advance(&raw[0]).unwrap();
        let ReadStep::Hour { frame, .. } = reader.advance(&replay, || Ok(())).unwrap() else {
            panic!("missing hour");
        };
        assert_frame(frame, expected);
        let ReadStep::BlockEnd { receipt } = reader.advance(&replay, || Ok(())).unwrap() else {
            panic!("missing boundary");
        };
        let receipt = receipt.clone();
        reader.accept_block(&receipt).unwrap();
        let (checkpoint, frame) = reader.into_checkpoint().unwrap();
        reader = SequentialReader::resume(selection.clone(), checkpoint, frame, execution).unwrap();
        assert_eq!(reader.cursor().next_block, block + 1);
    }
}

#[test]
fn size_splits_follow_the_actual_index_instead_of_a_fixed_hour_count() {
    let hours: Vec<_> = (1..=14).collect();
    let replay = test_recording::build_branches_limit(
        b"stream-size-split",
        &[(1, &hours, &[])],
        |_, hour, tables| {
            tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![
                    crate::rows::LexicalTokenRow {
                        token: 0,
                        literal: (0..5000).map(|at| (at * 71 + hour * 13) as u8).collect(),
                    },
                ]));
            Metrics::default()
        },
        32 * 1024,
    );
    assert!(replay.index.len() > 1);
    assert!(
        replay
            .index
            .iter()
            .any(|index| index.first_hour != 1 && index.first_hour < 720)
    );
    compare(replay, &[TableId::LexicalTokens, TableId::Clock], 3, true);
}

#[test]
fn unused_malformed_alias_dependency_does_not_refuse_literal_rows() {
    let replay = test_recording::build(b"dormant-source", &[1, 2, 3], |hour, tables| {
        tables[TableId::DivisionPanelStats.index()] =
            TableView::Complete(TableRows::DivisionPanelStats(vec![
                crate::rows::DivisionPanelStatsRow {
                    division: 7,
                    panel_stats: std::array::from_fn(|_| Some(999)),
                    ..Default::default()
                },
            ]));
        tables[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(vec![
                crate::rows::DivisionMovementRow {
                    division: 7,
                    base_speed: Some(123 + i64::from(hour)).into(),
                    fuel: None.into(),
                    ..Default::default()
                },
            ]));
        Metrics::default()
    });
    let replay = rewrite(replay, |_, id, raw| {
        if id == keyframe_section(TableId::DivisionPanelStats) {
            append_control(raw, TableId::DivisionPanelStats);
        }
    });
    assert!(
        replay
            .read_block_selected(&replay.index[0], &[TableId::DivisionPanelStats])
            .is_err()
    );
    compare(replay, &[TableId::DivisionMovement], 3, true);
}

#[test]
fn a_withheld_index_does_not_renumber_physical_anchor_references() {
    let mut replay = anchor_fixture();
    replay.index.remove(1);
    replay.ordinals.remove(1);
    replay.verified.remove(1);
    let replay = Arc::new(replay);
    let mut reader = SequentialReader::for_block(
        ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
        2,
        ReaderExecution {
            workers: 1,
            read_ahead: false,
        },
    )
    .unwrap();
    assert_eq!(
        reader
            .advance(&replay, || Ok(()))
            .err()
            .unwrap()
            .to_string(),
        "keyframe anchor is not the latest snapshot or latest keyframe"
    );
}

#[test]
fn missing_anchor_ordinal_is_refused_instead_of_using_another_block() {
    let mut replay = anchor_fixture();
    replay.index.remove(0);
    replay.ordinals.remove(0);
    replay.verified.remove(0);
    let replay = Arc::new(replay);
    let mut reader = SequentialReader::new(
        ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
        ReaderExecution {
            workers: 1,
            read_ahead: false,
        },
    )
    .unwrap();
    let error = reader.advance(&replay, || Ok(())).err().unwrap();
    assert_eq!(
        error.to_string(),
        "keyframe anchor names no earlier block of its file"
    );
    assert_eq!(reader.cursor().next_block, 0);
}

#[test]
fn cancellation_during_each_initial_stage_never_publishes_an_hour() {
    let (replay, _) = direct_tests::alias_fixture(Layout::Chosen);
    let replay = Arc::new(replay);
    for stop_at in [1, 2, 8, 32, 128, 256] {
        let mut reader = SequentialReader::new(
            ReaderSelection::new(&TableId::ALL).unwrap(),
            ReaderExecution {
                workers: 1,
                read_ahead: false,
            },
        )
        .unwrap();
        let mut calls = 0;
        let result = reader.advance(&replay, || {
            calls += 1;
            if calls == stop_at {
                Err(io::ErrorKind::Interrupted.into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::Interrupted);
        assert_eq!(calls, stop_at);
        assert_eq!(reader.cursor().next_block, 0);
        assert_eq!(reader.census().raw_blocks, 0);
    }
}

#[test]
fn append_resume_keeps_candidates_and_changed_old_bytes_poison_the_reader() {
    let full = Arc::new(anchor_fixture());
    let prefix = Replay::open(file_bytes(&full, 2)).unwrap();
    let prefix = Arc::new(prefix);
    let selection = ReaderSelection::new(&[TableId::LexicalTokens]).unwrap();
    let mut reader = SequentialReader::new(
        selection,
        ReaderExecution {
            workers: 2,
            read_ahead: true,
        },
    )
    .unwrap();
    for replay in [&prefix, &full] {
        loop {
            match reader.advance(replay, || Ok(())).unwrap() {
                ReadStep::Hour { .. } => {}
                ReadStep::BlockEnd { receipt } => {
                    let receipt = receipt.clone();
                    reader.accept_block(&receipt).unwrap();
                }
                ReadStep::Tail => break,
            }
        }
    }
    assert_eq!(reader.cursor().next_block, full.index.len());
    let mut changed = anchor_fixture();
    changed.bytes[changed.index[0].offset as usize + 8] ^= 1;
    let error = reader.advance(&Arc::new(changed), || Ok(())).err().unwrap();
    assert_eq!(
        error.to_string(),
        "recording differs from reader source prefix"
    );
    assert_eq!(reader.cursor().next_block, full.index.len());
}

#[test]
fn a_checkpoint_cannot_be_reused_with_a_different_selection() {
    let reader = SequentialReader::new(
        ReaderSelection::new(&[TableId::Clock]).unwrap(),
        ReaderExecution {
            workers: 1,
            read_ahead: false,
        },
    )
    .unwrap();
    let (checkpoint, frame) = reader.into_checkpoint().unwrap();
    let error = SequentialReader::resume(
        ReaderSelection::new(&[TableId::Countries]).unwrap(),
        checkpoint,
        frame,
        ReaderExecution {
            workers: 1,
            read_ahead: false,
        },
    )
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "reader checkpoint selection mismatch");
}

#[test]
fn map_capture_keeps_out_of_scope_tables_distinct_from_known_empty_tables() {
    let replay = test_recording::build_branches_mode(
        b"stream-map-mode",
        &[(1, &[1, 2, 3], &[])],
        |_, _, tables| {
            tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(Vec::new()));
            Metrics::default()
        },
        MAX_BLOCK_RAW_BYTES,
        CaptureMode::Map,
    );
    compare(replay, &TableId::ALL, 3, true);
}

pub(super) fn assert_seek(actual: SeekResult, expected: SeekResult) {
    match (actual, expected) {
        (SeekResult::Frame(a), SeekResult::Frame(b)) => assert_frame(&a, &b),
        (
            SeekResult::Gap {
                branch: a,
                hour: ah,
                reason: ar,
            },
            SeekResult::Gap {
                branch: b,
                hour: bh,
                reason: br,
            },
        ) => assert_eq!((a, ah, ar), (b, bh, br)),
        _ => panic!("seek result kind differs from the legacy oracle"),
    }
}

pub(crate) fn file_bytes(replay: &Replay, blocks: usize) -> Vec<u8> {
    let last = &replay.index[blocks - 1];
    let mut bytes = replay.bytes[..(last.offset + last.length) as usize].to_vec();
    super::super::finish_file(
        &mut bytes,
        &replay.sessions,
        &replay.branches,
        &replay.gaps,
        &replay.controls,
        &replay.recovery_notes,
        &replay.index[..blocks],
    )
    .unwrap();
    super::super::commit::stamp_slot(&mut bytes, replay.slot).unwrap();
    bytes
}

pub(crate) fn rewrite_fixture(byte: u8) -> Replay {
    let rows = TableRows::LexicalTokens(vec![crate::rows::LexicalTokenRow {
        token: 0,
        literal: vec![byte; 64],
    }]);
    let replay = test_recording::build(b"rewritten-prefix", &[1, 721], |_, tables| {
        tables[TableId::LexicalTokens.index()] = TableView::Complete(rows.clone());
        Metrics::default()
    });
    rewrite(replay, |position, id, raw| {
        if id == keyframe_section(TableId::LexicalTokens) {
            *raw = forced_keyframe(&rows, (position != 0).then_some((&rows, position)));
        }
    })
}

#[test]
#[cfg(windows)]
fn file_rewrite_with_equal_extents_invalidates_live_and_checkpoint_anchors() {
    let original = rewrite_fixture(b'a');
    let changed = rewrite_fixture(b'b');
    assert_eq!(original.index, changed.index);
    assert_ne!(file_bytes(&original, 1), file_bytes(&changed, 1));
    for (case, (checkpoint, initial_blocks, reopen)) in [
        (false, 1, true),
        (true, 1, true),
        (false, 2, true),
        (false, 2, false),
        (true, 1, false),
    ]
    .into_iter()
    .enumerate()
    {
        let path = std::env::temp_dir().join(format!(
            "hoi4-w2-rewrite-{}-{case}-{}.replay",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(&file_bytes(&original, initial_blocks))
            .unwrap();
        file.flush().unwrap();
        let held = Arc::new(Replay::open_path(&path).unwrap());
        let selection = ReaderSelection::new(&[TableId::LexicalTokens]).unwrap();
        let execution = ReaderExecution {
            workers: 2,
            read_ahead: true,
        };
        let mut reader = SequentialReader::new(selection.clone(), execution).unwrap();
        assert!(matches!(
            reader.advance(&held, || Ok(())).unwrap(),
            ReadStep::Hour { .. }
        ));
        let ReadStep::BlockEnd { receipt } = reader.advance(&held, || Ok(())).unwrap() else {
            panic!()
        };
        let receipt = receipt.clone();
        reader.accept_block(&receipt).unwrap();
        assert!(reader.census().anchor_states > 0);
        if checkpoint {
            let (state, frame) = reader.into_checkpoint().unwrap();
            reader = SequentialReader::resume(selection, state, frame, execution).unwrap();
        }
        file.seek(SeekFrom::Start(0)).unwrap();
        let replacement = file_bytes(&changed, 2);
        if !checkpoint {
            assert_eq!(
                file.write_all(&replacement).unwrap_err().raw_os_error(),
                Some(33)
            );
            let (state, frame) = reader.into_checkpoint().unwrap();
            reader = SequentialReader::resume(
                ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
                state,
                frame,
                execution,
            )
            .unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
        }
        file.write_all(&replacement).unwrap();
        file.set_len(replacement.len() as u64).unwrap();
        file.flush().unwrap();
        let reopened = Arc::new(Replay::open_path(&path).unwrap());
        assert!(matches!(
            reopened
                .seek_selected(1, 721, Some(&[TableId::LexicalTokens]))
                .unwrap(),
            SeekResult::Frame(_)
        ));
        let source = if reopen { &reopened } else { &held };
        let error = reader.advance(source, || Ok(())).err().unwrap();
        assert_eq!(
            error.to_string(),
            "recording differs from reader source prefix"
        );
        assert_eq!(reader.census().anchor_states, 0);
        assert_eq!(reader.census().raw_blocks, 0);
        assert!(reader.current().is_none());
        drop((reader, reopened, held, file));
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn arbitrary_original_block_streams_updates_then_releases_without_accepting_history() {
    let replay = test_recording::build(
        b"block-cold",
        &[1, 2, 721, 722, 1441, 1442, 2161, 2162],
        |hour, tables| {
            tables[TableId::LexicalTokens.index()] =
                TableView::Complete(lexical_rows(((hour - 1) % 720) as u8));
            Metrics::default()
        },
    );
    let mut replay = rewrite(replay, |position, id, raw| {
        if id == keyframe_section(TableId::LexicalTokens) {
            let base = lexical_rows(0);
            *raw = forced_keyframe(
                &base,
                if position >= 3 {
                    Some((&base, position - 2))
                } else {
                    None
                },
            );
        }
    });
    replay.index.remove(1);
    replay.ordinals.remove(1);
    replay.verified.remove(1);
    // An earlier later section is damaged; cold bootstrap must never load it.
    let mut damaged = false;
    let replay = rewrite(replay, |position, id, raw| {
        if position == 0 && id == later_section(TableId::LexicalTokens) {
            raw.clear();
            raw.push(255);
            damaged = true;
        }
    });
    assert!(damaged);
    let replay = Arc::new(replay);
    let selection = ReaderSelection::new(&[TableId::LexicalTokens]).unwrap();
    let mut reader = SequentialReader::for_block(
        selection.clone(),
        3,
        ReaderExecution {
            workers: 3,
            read_ahead: true,
        },
    )
    .unwrap();
    let reference = reference_copy(&replay);
    let expected = reference
        .read_block_selected(&reference.index[2], selection.tables())
        .unwrap();
    let mut decoder = ReferenceDecoder::for_tables(selection.tables());
    for raw in &expected {
        let ReadStep::Hour {
            ordinal,
            frame,
            updates,
            ..
        } = reader.advance(&replay, || Ok(())).unwrap()
        else {
            panic!()
        };
        assert_eq!(ordinal, 3);
        assert_frame(frame, decoder.advance(raw).unwrap());
        for table in selection.tables() {
            assert_eq!(updates.tables[table.index()], raw.tables[table.index()]);
        }
    }
    let ReadStep::BlockEnd { receipt } = reader.advance(&replay, || Ok(())).unwrap() else {
        panic!()
    };
    let receipt = receipt.clone();
    assert_eq!(receipt.ordinal(), 3);
    assert_eq!(reader.cursor().next_block, 0);
    assert!(reader.cursor().previous.is_none());
    let census = reader.census();
    assert_eq!(census.anchor_states, 0);
    assert!(census.anchor_bootstrap_reads > 0);
    assert!(census.anchor_bootstrap_raw_bytes > 0 && census.anchor_bootstrap_stored_bytes > 0);
    assert_eq!(census.peak_raw_blocks, 1);
    reader.accept_block(&receipt).unwrap();
    assert_eq!(reader.cursor().next_block, 0);
    assert!(reader.current().is_none());
    assert_eq!(reader.census().raw_blocks, 0);
    assert!(matches!(
        reader.advance(&replay, || Ok(())).unwrap(),
        ReadStep::Tail
    ));
    assert!(reader.into_checkpoint().is_err());
}

pub(super) fn ownership_fixture(hours: i32) -> Replay {
    let times: Vec<_> = (1..=hours).collect();
    test_recording::build(b"live-ownership", &times, |hour, tables| {
        tables[TableId::LexicalTokens.index()] = TableView::Complete(lexical_rows(hour as u8));
        Metrics::default()
    })
}

#[test]
fn live_allocations_and_weak_owners_bound_current_and_worker_updates() {
    use super::super::reader::ownership::{Audit, Kind};
    for workers in [1, 3] {
        let mut peaks = Vec::new();
        for hours in [8, 64] {
            let replay = Arc::new(ownership_fixture(hours));
            let audit = Audit::new();
            let scope = audit.enter();
            let mut reader = SequentialReader::new(
                ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
                ReaderExecution {
                    workers,
                    read_ahead: true,
                },
            )
            .unwrap();
            for _ in 0..hours {
                assert!(matches!(
                    reader.advance(&replay, || Ok(())).unwrap(),
                    ReadStep::Hour { .. }
                ));
                audit.assert_unique_views();
                assert!(audit.live_update_hours() <= 2);
                audit.clear_dead_witnesses();
            }
            assert!(audit.peak_update_hours() <= 2);
            assert!(
                audit.peak(Kind::Raw) > 0
                    && audit.peak(Kind::Frame) > 0
                    && audit.peak(Kind::Anchors) > 0
            );
            if workers > 1 {
                assert!(audit.peak(Kind::Jobs) > 0 && audit.peak(Kind::Results) > 0);
            }
            let measured = (
                audit.peak(Kind::Frame),
                audit.peak(Kind::Updates),
                audit.peak_total(),
                audit.peak(Kind::Raw),
            );
            let error = reader
                .advance(&replay, || Err(io::ErrorKind::Interrupted.into()))
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            assert_eq!(audit.live_update_hours(), 0);
            assert_eq!(audit.live_views(false), 0);
            drop(reader);
            drop(scope);
            audit.clear_dead_witnesses();
            assert_eq!(audit.live_total(), 0);
            assert!(audit.peak_total() > 0);
            peaks.push(measured);
        }
        assert!(
            peaks[1].2 <= peaks[0].2 * 2 + peaks[1].3.saturating_sub(peaks[0].3) * 2,
            "hidden allocations grow with consumed history: {peaks:?}"
        );
        assert!(
            peaks[1].0 <= peaks[0].0 * 2,
            "frame allocations grow with consumed history: {peaks:?}"
        );
        assert!(
            peaks[1].1 <= peaks[0].1 * 2,
            "update allocations grow with consumed history: {peaks:?}"
        );
    }
}

#[test]
fn ownership_witnesses_detect_consumer_retention_of_old_frames_and_updates() {
    use super::super::reader::ownership::Audit;
    let replay = Arc::new(ownership_fixture(4));
    let audit = Audit::new();
    let scope = audit.enter();
    let mut reader = SequentialReader::for_block(
        ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
        0,
        ReaderExecution {
            workers: 1,
            read_ahead: false,
        },
    )
    .unwrap();
    let mut retained = Vec::new();
    let mut old_frame = None;
    let mut old_identity = None;
    for hour in 1..=3 {
        let ReadStep::Hour { frame, updates, .. } = reader.advance(&replay, || Ok(())).unwrap()
        else {
            panic!()
        };
        if hour == 1 {
            old_frame = Some(frame.clone());
            old_identity = Some(frame.tables[TableId::LexicalTokens.index()].identity());
        }
        retained.push(updates.clone());
    }
    assert_eq!(audit.live_update_hours(), 3);
    assert!(audit.peak_update_hours() >= 3);
    assert!(old_identity.as_ref().unwrap().is_alive());
    drop((retained, old_frame));
    assert!(!old_identity.as_ref().unwrap().is_alive());
    assert_eq!(audit.live_update_hours(), 1);
    drop((old_identity, reader, scope));
    audit.clear_dead_witnesses();
    assert_eq!(audit.live_total(), 0);
}

#[cfg(windows)]
pub(crate) fn many_blocks_fixture(blocks: i32) -> Replay {
    let hours: Vec<_> = (0..blocks).map(|block| 1 + block * 720).collect();
    test_recording::build(b"linear-source-proof", &hours, |_, _| Metrics::default())
}

#[test]
fn obsolete_successful_keyframes_are_not_eligible_anchor_targets() {
    let replay = rewrite(anchor_fixture(), |position, id, raw| {
        if position == 4 && id == keyframe_section(TableId::LexicalTokens) {
            *raw = forced_keyframe(&lexical_rows(3), Some((&lexical_rows(1), 3)));
        }
    });
    let replay = Arc::new(replay);
    let selection = ReaderSelection::new(&[TableId::LexicalTokens]).unwrap();
    for local in [false, true] {
        let execution = ReaderExecution {
            workers: 1,
            read_ahead: false,
        };
        let mut reader = if local {
            SequentialReader::for_block(selection.clone(), 4, execution).unwrap()
        } else {
            SequentialReader::new(selection.clone(), execution).unwrap()
        };
        if !local {
            for _ in 0..4 {
                assert!(matches!(
                    reader.advance(&replay, || Ok(())).unwrap(),
                    ReadStep::Hour { .. }
                ));
                let ReadStep::BlockEnd { receipt } = reader.advance(&replay, || Ok(())).unwrap()
                else {
                    panic!()
                };
                let receipt = receipt.clone();
                reader.accept_block(&receipt).unwrap();
            }
        }
        assert_eq!(
            reader
                .advance(&replay, || Ok(()))
                .err()
                .unwrap()
                .to_string(),
            "keyframe anchor is not the latest snapshot or latest keyframe"
        );
    }
}

pub(in crate::container) fn multi_chunk_fixture() -> Replay {
    chunk_tests::fixture().0
}
