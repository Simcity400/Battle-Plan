use super::*;
use crate::container::{ReadStep, ReaderExecution, ReaderSelection, SequentialReader};
use std::sync::Arc;

fn rebuilt(replay: &Replay, blocks: Vec<(BlockIndex, Vec<u8>)>) -> Replay {
    let mut bytes = replay.bytes[..replay.index[0].offset as usize].to_vec();
    let mut index = Vec::new();
    for (mut entry, block) in blocks {
        entry.offset = bytes.len() as u64;
        bytes.extend_from_slice(&block);
        index.push(entry);
    }
    super::super::finish_file(
        &mut bytes,
        &replay.sessions,
        &replay.branches,
        &replay.gaps,
        &replay.controls,
        &replay.recovery_notes,
        &index,
    )
    .unwrap();
    super::super::commit::stamp_slot(&mut bytes, replay.slot).unwrap();
    Replay::open(bytes).unwrap()
}

fn chunk_damage(replay: Replay, table: TableId, slot: usize, grammar: bool) -> Replay {
    let index = replay.index[0].clone();
    let directory = Directory::read(&replay, &index).unwrap();
    let (offset, stored, raw) = directory.chunk_extents(Section::Later(table))[slot];
    if grammar {
        let mut bytes =
            Directory::inflate(&replay.block_range(&index, offset, stored).unwrap(), raw).unwrap();
        reader_tests::append_control(&mut bytes, table);
        let changed =
            chunked::replace_chunks(&replay, &index, &[], &[(later_section(table), slot, bytes)])
                .unwrap();
        let mut blocks = vec![changed];
        for index in &replay.index[1..] {
            blocks.push((
                index.clone(),
                replay.block_bytes(index).unwrap().into_owned(),
            ));
        }
        rebuilt(&replay, blocks)
    } else {
        let mut replay = replay;
        replay.bytes[index.offset as usize + offset + stored - 1] ^= 1;
        replay
    }
}

#[test]
fn actual_multichunk_alias_marks_gaps_and_empty_windows_match_native_hours() {
    let hours: Vec<_> = (10..=220).chain([600, 650, 729]).chain(730..=940).collect();
    for workers in [1, 3] {
        let (replay, native) = direct_tests::alias_fixture_at(Layout::Chosen, &hours);
        let mut oracle = reader_reference::ReferenceDecoder::new();
        let mut application = reader_reference::ReferenceDecoder::new();
        for (actual, expected) in replay
            .index
            .iter()
            .flat_map(|index| replay.read_block(index).unwrap())
            .zip(&native)
        {
            reader_tests::assert_frame(
                application.advance(&actual).unwrap(),
                oracle.advance(expected).unwrap(),
            );
        }
        assert!(
            replay
                .section_chunks(&replay.index[0], Section::Later(TableId::DivisionMovement))
                .unwrap()
                .len()
                >= 4
        );
        reader_tests::compare(replay, &TableId::ALL, workers, workers > 1);
    }
}

#[test]
fn the_row_coding_marker_can_follow_empty_initial_windows() {
    let hours = [10, 600, 650, 729, 730, 900];
    for workers in [1, 3] {
        let (replay, _) = direct_tests::alias_fixture_at(Layout::Chosen, &hours);
        let reference = reader_tests::reference_copy(&replay);
        reader_tests::assert_seek(
            replay.seek(1, 10).unwrap(),
            reader_reference::seek(&reference, 1, 10, None, &mut Check::disabled()).unwrap(),
        );
        reader_tests::compare(replay, &TableId::ALL, workers, workers > 1);
    }
}

fn row_coding_fixture(modelled: bool) -> Replay {
    let (replay, _) = chunk_tests::fixture();
    let cores = Cores::new(1);
    let mut anchors = Anchors::default();
    let mut blocks = Vec::new();
    for index in &replay.index {
        let frames = replay.read_block(index).unwrap();
        let mut writer = BlockWriter::new(&frames[0]).unwrap();
        for table in &mut writer.tables {
            table.predicted = None;
            table.plain.columns.1[KEYS] = None;
            table.plain.columns.1[MASKS] = None;
            if !modelled {
                table.plain.model = None;
                table.plain.columns.0[MASKS][0] = LISTED_ROWS;
            }
        }
        let mut decoder = crate::frame::FrameDecoder::new();
        for frame in &frames {
            let mut payload = Vec::new();
            let sizes = frame.encode_with_sizes(&mut payload).unwrap();
            writer
                .push(
                    frame,
                    decoder.advance(frame).unwrap(),
                    &payload,
                    &sizes,
                    &mut anchors,
                    MAX_BLOCK_RAW_BYTES,
                    &cores,
                )
                .unwrap();
        }
        let block = writer.compress(&cores).unwrap();
        let mut index = index.clone();
        index.length = block.bytes.len() as u64;
        index.raw_length = block.raw_length;
        blocks.push((index, block.bytes));
    }
    rebuilt(&replay, blocks)
}

#[test]
fn listed_and_modelled_range_lookahead_survive_real_loader_replacements() {
    for modelled in [false, true] {
        for workers in [1, 3] {
            reader_tests::compare(
                row_coding_fixture(modelled),
                &TableId::ALL,
                workers,
                workers > 1,
            );
        }
    }
}

#[test]
fn the_previous_raw_owner_is_dead_before_the_next_chunk_is_admitted() {
    let (replay, _) = chunk_tests::fixture();
    let credits = Arc::new(chunks::Credits::default());
    let table = TableId::LexicalTokens;
    let mut selected = [false; TABLE_COUNT];
    selected[table.index()] = true;
    let mut block = chunks::BlockChunks::load(
        &replay,
        0,
        &selected,
        true,
        credits.acquire(1).unwrap(),
        &mut Check::disabled(),
    )
    .unwrap();
    let (mut cursor, _, _) = cursor::TableCursor::begin(
        &replay,
        &mut block,
        table,
        &mut anchors::TableAnchors::local(),
        &mut Check::disabled(),
    )
    .unwrap();
    let mut owner = cursor.raw_owner();
    let mut slot = 0;
    let mut retired = 0;
    for at in 1..block.hours.len() {
        let next = (block.hours[at] - block.hours[0] - 1) as usize / LATER_CHUNK_HOURS;
        let mut inspected = false;
        let mut inspect = || {
            if next != slot
                && super::super::check::step()
                    == Some(super::super::check::Step::Source(later_section(table)))
            {
                assert!(owner.upgrade().is_none());
                inspected = true;
            }
            Ok(())
        };
        cursor
            .prepare(&replay, &mut block, at, None, &mut Check::new(&mut inspect))
            .unwrap();
        if next != slot {
            assert!(inspected);
            assert!(owner.upgrade().is_none());
            owner = cursor.raw_owner();
            slot = next;
            retired += 1;
        }
        cursor
            .next(
                block.hours[at - 1],
                block.entered(table, at),
                &mut Check::disabled(),
            )
            .unwrap();
    }
    assert!(retired >= 2);
    cursor.finish().unwrap();
    drop(cursor);
    assert!(owner.upgrade().is_none());
}

#[test]
fn middle_and_final_chunk_damage_cannot_issue_a_receipt_or_verify_a_prefix() {
    for slot in [1, 4] {
        for grammar in [false, true] {
            for workers in [1, 3] {
                let (replay, _) = chunk_tests::fixture();
                let replay = Arc::new(chunk_damage(replay, TableId::LexicalTokens, slot, grammar));
                let mut reader = SequentialReader::for_block(
                    ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
                    0,
                    ReaderExecution {
                        workers,
                        read_ahead: false,
                    },
                )
                .unwrap();
                let mut seen = 0;
                loop {
                    match reader.advance(&replay, || Ok(())) {
                        Ok(ReadStep::Hour { .. }) => seen += 1,
                        Err(error) => {
                            assert!(super::super::recording_damage(&error));
                            break;
                        }
                        _ => panic!("damaged chunk reached a block receipt"),
                    }
                }
                assert!(seen > 0);
                assert!(!replay.is_verified(&replay.index[0]));
                assert_eq!(reader.cursor().next_block, 0);
                assert!(reader.current().is_none());
                assert_eq!(reader.census().raw_blocks, 0);
            }
        }
    }
}

#[test]
fn cancellation_at_chunk_replacement_drops_the_old_owner_and_poisoned_state() {
    let (replay, _) = chunk_tests::fixture();
    let replay = Arc::new(replay);
    let table = TableId::LexicalTokens;
    let mut reader = SequentialReader::for_block(
        ReaderSelection::new(&[table]).unwrap(),
        0,
        ReaderExecution {
            workers: 1,
            read_ahead: false,
        },
    )
    .unwrap();
    for _ in 0..169 {
        assert!(matches!(
            reader.advance(&replay, || Ok(())).unwrap(),
            ReadStep::Hour { .. }
        ));
    }
    let mut cancelled = false;
    let error = reader
        .advance(&replay, || {
            if super::super::check::step()
                == Some(super::super::check::Step::Source(later_section(table)))
            {
                cancelled = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            Ok(())
        })
        .err()
        .unwrap();
    assert!(cancelled);
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(reader.census().raw_blocks, 0);
    assert_eq!(reader.cursor().next_block, 0);
    assert!(reader.current().is_none());
}

#[test]
fn compatibility_first_hour_checks_future_chunk_integrity_not_future_grammar() {
    for grammar in [false, true] {
        let (replay, _) = chunk_tests::fixture();
        let replay = chunk_damage(replay, TableId::LexicalTokens, 1, grammar);
        let selected = &[TableId::LexicalTokens];
        let reference = reader_tests::reference_copy(&replay);
        let expected =
            reader_reference::seek(&reference, 1, 10, Some(selected), &mut Check::disabled())
                .unwrap();
        let actual = replay.seek_selected(1, 10, Some(selected)).unwrap();
        if grammar {
            assert!(matches!(&actual, super::super::SeekResult::Frame(_)));
        } else {
            assert!(matches!(&actual, super::super::SeekResult::Gap { .. }));
        }
        reader_tests::assert_seek(actual, expected);
        assert!(!replay.is_verified(&replay.index[0]));
    }
}

#[test]
fn standalone_first_hour_never_admits_a_future_damaged_chunk() {
    let (replay, _) = chunk_tests::fixture();
    let replay = Arc::new(chunk_damage(replay, TableId::LexicalTokens, 1, false));
    let mut point = crate::container::HourReader::new();
    point.with_hour(&replay, 1, 10, ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(), || Ok(()), |result| {
        assert!(matches!(result, crate::container::PointResult::Frame(frame) if frame.hour == 10));
    }).unwrap();
    assert!(!replay.is_verified(&replay.index[0]));
    assert_eq!(point.census().point_validation_raw_bytes, 0);
}

#[test]
fn compatibility_checks_only_alias_sources_the_point_actually_reads() {
    let hours: Vec<_> = (10..=729).collect();
    for marked in [false, true] {
        for grammar in [false, true] {
            let replay = test_recording::build(b"point-alias-envelope", &hours, |hour, tables| {
                let panel = 1_000_000 + i64::from(hour % 97);
                tables[TableId::DivisionPanelStats.index()] =
                    TableView::Complete(TableRows::DivisionPanelStats(vec![
                        crate::rows::DivisionPanelStatsRow {
                            division: 7,
                            panel_stats: std::array::from_fn(|_| Some(panel)),
                            ..Default::default()
                        },
                    ]));
                tables[TableId::DivisionMovement.index()] =
                    TableView::Complete(TableRows::DivisionMovement(vec![
                        crate::rows::DivisionMovementRow {
                            division: 7,
                            base_speed: Some(if marked { panel } else { 123 + i64::from(hour) })
                                .into(),
                            fuel: None.into(),
                            ..Default::default()
                        },
                    ]));
                Metrics::default()
            });
            let replay = chunk_damage(replay, TableId::DivisionPanelStats, 1, grammar);
            let reference = reader_tests::reference_copy(&replay);
            let points: &[Hour] = if marked { &[10, 150] } else { &[10, 150, 400] };
            for &hour in points {
                let selected = &[TableId::DivisionMovement];
                let expected = reader_reference::seek(
                    &reference,
                    1,
                    hour,
                    Some(selected),
                    &mut Check::disabled(),
                )
                .unwrap();
                assert_eq!(
                    matches!(&expected, super::super::SeekResult::Frame(_)),
                    !marked || grammar,
                    "marked {marked}, grammar {grammar}, hour {hour}",
                );
                reader_tests::assert_seek(
                    replay.seek_selected(1, hour, Some(selected)).unwrap(),
                    expected,
                );
                assert!(!replay.is_verified(&replay.index[0]));
            }
        }
    }
}

#[test]
fn dormant_chunk_integrity_failure_preserves_the_oracle_reason_and_boundary() {
    let source = TableId::DivisionPanelStats;
    let selected = &[TableId::DivisionMovement];
    for (last, section, slot) in [
        (10, Section::Keyframe(source), 0),
        (12, Section::Keyframe(source), 0),
        (12, Section::Later(source), 0),
        (350, Section::Later(source), 1),
    ] {
        for failure in 0..3 {
            let selected_tail = failure == 1;
            let earlier_integrity = failure == 2;
            let hours: Vec<_> = (10..=last).collect();
            let mut replay =
                test_recording::build(b"dormant-chunk-integrity", &hours, |hour, tables| {
                    // The second later chunk needs a residual to have bytes to corrupt.
                    let panel = 1_000_000 + i64::from(hour) + i64::from(hour == 200);
                    tables[source.index()] =
                        TableView::Complete(TableRows::DivisionPanelStats(vec![
                            crate::rows::DivisionPanelStatsRow {
                                division: 7,
                                panel_stats: std::array::from_fn(|_| Some(panel)),
                                ..Default::default()
                            },
                        ]));
                    tables[selected[0].index()] =
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
            if selected_tail {
                replay = reader_tests::rewrite(replay, |_, id, raw| {
                    let tail = if last == 10 {
                        keyframe_section(selected[0])
                    } else {
                        later_section(selected[0])
                    };
                    if id == tail {
                        reader_tests::append_control(raw, selected[0]);
                    }
                });
            }
            let index = &replay.index[0];
            let directory = Directory::read(&replay, index).unwrap();
            let (offset, stored, _) = directory.chunk_extents(section)[slot];
            assert!(stored > 0);
            replay.bytes[index.offset as usize + offset + stored - 1] ^= 1;
            if earlier_integrity {
                let (offset, _, _) = directory.sections[keyframe_section(TableId::Clock)].unwrap();
                assert!(keyframe_section(TableId::Clock) < keyframe_section(source));
                replay.bytes[index.offset as usize + offset] ^= 1;
            }
            let reference = reader_tests::reference_copy(&replay);
            let expected =
                reader_reference::seek(&reference, 1, last, Some(selected), &mut Check::disabled())
                    .unwrap();
            let super::super::SeekResult::Gap { reason, .. } = &expected else {
                panic!("damaged source issued an oracle frame");
            };
            assert!(!reason.contains("reader source prefix"));
            if selected_tail {
                assert_eq!(
                    reason,
                    "unreadable block: grouped section has unread column bytes"
                );
            } else if earlier_integrity {
                assert_eq!(
                    reason,
                    "unreadable block: grouped section is not a zstd frame"
                );
            } else {
                assert!(reason.contains("checksum"), "{reason}");
            }
            let integrity_reason = reason
                .strip_prefix("unreadable block: ")
                .unwrap()
                .to_owned();
            reader_tests::assert_seek(
                replay.seek_selected(1, last, Some(selected)).unwrap(),
                expected,
            );
            assert!(!replay.is_verified(index));
            if last != 10 {
                let expected = reader_reference::seek(
                    &reference,
                    1,
                    10,
                    Some(selected),
                    &mut Check::disabled(),
                )
                .unwrap();
                assert!(matches!(expected, super::super::SeekResult::Frame(_)));
                reader_tests::assert_seek(
                    replay.seek_selected(1, 10, Some(selected)).unwrap(),
                    expected,
                );
                assert!(!replay.is_verified(index));
            }
            if selected_tail {
                continue;
            }
            let replay = Arc::new(replay);
            for workers in [1, 3] {
                let mut reader = SequentialReader::new(
                    ReaderSelection::new(selected).unwrap(),
                    ReaderExecution {
                        workers,
                        read_ahead: false,
                    },
                )
                .unwrap();
                for &hour in &hours {
                    let ReadStep::Hour { frame, .. } = reader.advance(&replay, || Ok(())).unwrap()
                    else {
                        panic!("dormant source prevented a tentative hour");
                    };
                    assert_eq!(frame.hour, hour);
                }
                let error = reader.advance(&replay, || Ok(())).err().unwrap();
                assert_eq!(error.to_string(), integrity_reason);
                assert!(!replay.is_verified(&replay.index[0]));
                assert_eq!(reader.cursor().next_block, 0);
                assert!(reader.current().is_none());
                assert_eq!(reader.census().raw_blocks, 0);
                let error = reader.advance(&replay, || Ok(())).err().unwrap();
                assert_eq!(
                    error.to_string(),
                    "reader is poisoned; restore the committed checkpoint"
                );
            }
        }
    }
}

/// Every frame of `replay`'s blocks, read with every later section joined
/// or not, and this thread's later sections read a chunk at a time and
/// fallen back to their joined section meanwhile.
fn frames_read(replay: &Replay, joined: bool) -> (Vec<Frame>, [usize; 2]) {
    let replay = Replay::open(replay.bytes.to_vec()).unwrap();
    replay.cache_decoded_sections(false);
    JOINED.set(joined);
    CHUNKED_READS.set([0; 2]);
    let frames = replay
        .index
        .iter()
        .flat_map(|index| replay.read_block(index).unwrap())
        .collect();
    JOINED.set(false);
    (frames, CHUNKED_READS.get())
}

fn alias_hours() -> Vec<Hour> {
    (10..=220).chain([600, 650, 729]).chain(730..=940).collect()
}

/// Later sections read a chunk at a time, keeping only their alias marks,
/// decode every table at every hour as their joined sections do: across
/// empty chunks, gaps, alias marks and several blocks.
#[test]
fn later_sections_read_a_chunk_at_a_time_decode_as_their_joined_sections() {
    for (label, replay) in [
        ("chunked", super::chunk_tests::fixture().0),
        (
            "aliased",
            direct_tests::alias_fixture_at(Layout::Chosen, &alias_hours()).0,
        ),
    ] {
        let (chunked, reads) = frames_read(&replay, false);
        let (joined, none) = frames_read(&replay, true);
        assert_eq!(chunked, joined, "{label}");
        assert!(reads[0] > 0 && reads[1] == 0, "{label}: {reads:?}");
        assert_eq!(none, [0, 0], "{label}");
    }
}

/// A chunk whose columns end inside an hour holds the same joined bytes cut
/// elsewhere: its table falls back to the joined section and reads as the
/// unchanged recording does.
#[test]
fn a_chunk_cut_inside_an_hour_falls_back_to_the_joined_section() {
    let (replay, _) = direct_tests::alias_fixture_at(Layout::Chosen, &alias_hours());
    let table = TableId::DivisionMovement;
    let index = replay.index[0].clone();
    let directory = Directory::read(&replay, &index).unwrap();
    let raws: Vec<Vec<u8>> = directory
        .chunk_extents(Section::Later(table))
        .into_iter()
        .map(|(offset, stored, raw)| {
            if stored == 0 {
                Vec::new()
            } else {
                Directory::inflate(&replay.block_range(&index, offset, stored).unwrap(), raw)
                    .unwrap()
            }
        })
        .collect();
    let slot = (0..raws.len() - 1)
        .find(|&at| !raws[at].is_empty() && !raws[at + 1].is_empty())
        .unwrap();
    let columns = |raw: &[u8]| -> Vec<Vec<u8>> {
        chunked::columns(raw)
            .unwrap()
            .into_iter()
            .map(<[u8]>::to_vec)
            .collect()
    };
    let (mut before, mut after) = (columns(&raws[slot]), columns(&raws[slot + 1]));
    // A column every entry reads after its control byte: the next chunk's
    // first entry finds this chunk's control column read to its end.
    let moved = (CONTROL + 1..after.len() - 1)
        .find(|&column| !after[column].is_empty())
        .unwrap();
    let byte = after[moved].remove(0);
    before[moved].push(byte);
    let id = later_section(table);
    let changed = chunked::replace_chunks(
        &replay,
        &index,
        &[],
        &[
            (id, slot, chunked::frame(&before).unwrap()),
            (id, slot + 1, chunked::frame(&after).unwrap()),
        ],
    )
    .unwrap();
    let mut blocks = vec![changed];
    for index in &replay.index[1..] {
        blocks.push((
            index.clone(),
            replay.block_bytes(index).unwrap().into_owned(),
        ));
    }
    let cut = rebuilt(&replay, blocks);
    let (actual, reads) = frames_read(&cut, false);
    let (expected, _) = frames_read(&replay, true);
    assert_eq!(actual, expected);
    assert!(reads[1] > 0, "{reads:?}");
}

/// Every frame of `replay`'s blocks with each table's plain bytes bounded by
/// `budget`, or the read's refusal, and this thread's later sections read a
/// chunk at a time and fallen back to their joined section meanwhile.
fn budgeted(
    replay: &Replay,
    budget: usize,
    joined: bool,
) -> (Result<Vec<Frame>, String>, [usize; 2]) {
    let replay = Replay::open(replay.bytes.to_vec()).unwrap();
    replay.cache_decoded_sections(false);
    RAW_BUDGET.set(Some(budget));
    JOINED.set(joined);
    CHUNKED_READS.set([0; 2]);
    let mut frames = Vec::new();
    let mut read = Ok(());
    for index in &replay.index {
        match replay.read_block(index) {
            Ok(block) => frames.extend(block),
            Err(error) => {
                read = Err(error.to_string());
                break;
            }
        }
    }
    JOINED.set(false);
    RAW_BUDGET.set(None);
    (read.map(|()| frames), CHUNKED_READS.get())
}

/// A tag shrunk by a patch in the first chunk, then a new long tag every
/// hour of the later chunks, in the same column.
fn patch_budget_fixture() -> Replay {
    let hours: Vec<Hour> = (10..=400).collect();
    let tag = |seed: u64, length: usize| -> Vec<u8> {
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    };
    let replay =
        test_recording::build(b"synthetic-chunked-patch-budget", &hours, |hour, tables| {
            let tag = match hour {
                10 => tag(0, 20_000),
                11..178 => tag(0, 20_000)[..100].to_vec(),
                _ => tag(hour as u64, 2_000),
            };
            tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(vec![crate::rows::CountryRow {
                    id: 1,
                    canonical_slot: 1,
                    tag,
                    ..Default::default()
                }]));
            Metrics::default()
        });
    assert_eq!(replay.index.len(), 1);
    assert!(
        replay
            .section_chunks(&replay.index[0], Section::Later(TableId::Countries))
            .unwrap()
            .len()
            >= 3
    );
    replay
}

/// A patch's budget counts what its column holds in later chunks, as the
/// joined section's column does: a block the joined read refuses for it is
/// refused when read a chunk at a time too, though the chunk alone fits.
#[test]
fn a_patch_budgets_its_column_bytes_in_later_chunks() {
    let replay = patch_budget_fixture();
    // The least budget the joined read decodes the block in.
    let (mut refused, mut fits) = (0, MAX_BLOCK_RAW_BYTES);
    while fits - refused > 1 {
        let budget = refused + (fits - refused) / 2;
        if budgeted(&replay, budget, true).0.is_ok() {
            fits = budget;
        } else {
            refused = budget;
        }
    }
    let refusal = budgeted(&replay, refused, true).0.unwrap_err();
    CHUNK_LOCAL.set(true);
    let (local, _) = budgeted(&replay, refused, false);
    CHUNK_LOCAL.set(false);
    assert!(local.is_ok(), "{local:?}");
    let (chunked, reads) = budgeted(&replay, refused, false);
    assert_eq!(chunked, Err(refusal));
    assert!(reads[1] > 0, "{reads:?}");
    assert_eq!(
        budgeted(&replay, fits, false).0,
        budgeted(&replay, fits, true).0
    );
}
