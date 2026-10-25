use super::reader_reference::ReferenceDecoder;
use super::*;
use crate::container::{HourReader, PointResult, ReaderSelection, SeekResult};
use std::sync::Arc;

#[test]
fn first_middle_last_backward_and_changed_selection_match_uncached_reconstruction() {
    let (replay, _) = direct_tests::alias_fixture(Layout::Chosen);
    let replay = Arc::new(replay);
    let reference_replay = reader_tests::reference_copy(&replay);
    replay.cache_decoded_sections(false);
    let mut reader = HourReader::new();
    for tables in [
        &[TableId::Divisions][..],
        &[TableId::ProductionLines][..],
        &TableId::ALL[..],
        &[][..],
    ] {
        let selection = ReaderSelection::new(tables).unwrap();
        for hour in [10, 18, 25, 740, 731, 10] {
            let index = replay
                .index
                .iter()
                .find(|index| index.first_hour <= hour && hour <= index.last_hour)
                .unwrap();
            let mut reference = ReferenceDecoder::for_tables(selection.tables());
            let mut expected = None;
            for frame in reference_replay
                .read_block_selected(index, selection.tables())
                .unwrap()
            {
                reference.advance(&frame).unwrap();
                if frame.hour == hour {
                    expected = reference.current().cloned();
                    break;
                }
            }
            replay.log_section_decodes();
            reader
                .with_hour(
                    &replay,
                    1,
                    hour,
                    selection.clone(),
                    || Ok(()),
                    |result| {
                        let PointResult::Frame(frame) = result else {
                            panic!("missing point");
                        };
                        reader_tests::assert_frame(frame, expected.as_ref().unwrap());
                    },
                )
                .unwrap();
            assert!(reader.census().raw_blocks <= 1);
            assert!(reader.census().peak_raw_blocks <= 1);
            assert!(replay.section_decodes().is_empty());
        }
    }
}

#[test]
fn gaps_are_exact_and_owned_seek_transfers_its_frame() {
    let replay = test_recording::build(b"point-gaps", &[10, 12, 15], |_, _| Metrics::default());
    let replay = Arc::new(replay);
    let mut reader = HourReader::new();
    for (branch, hour, expected) in [
        (99, 10, "unknown branch"),
        (1, 9, "hour was not recorded"),
        (1, 11, "hour was not recorded"),
        (1, 14, "hour was not recorded"),
        (1, 16, "hour was not recorded"),
    ] {
        reader
            .with_hour(
                &replay,
                branch,
                hour,
                ReaderSelection::new(&[TableId::Clock]).unwrap(),
                || Ok(()),
                |result| {
                    let PointResult::Gap { reason, .. } = result else {
                        panic!("a gap became a nearby hour");
                    };
                    assert_eq!(reason, expected);
                },
            )
            .unwrap();
    }
    let result = reader
        .seek(
            &replay,
            1,
            12,
            Some(&[TableId::Clock]),
            &mut Check::disabled(),
        )
        .unwrap();
    assert!(matches!(result, SeekResult::Frame(frame) if frame.hour == 12));
    assert_eq!(reader.census().raw_blocks, 0);
}

#[test]
fn prefix_does_not_validate_future_column_grammar_or_certify_the_block() {
    let replay = test_recording::build(b"point-tail", &[10, 12, 15], |_, _| Metrics::default());
    let replay = reader_tests::rewrite(replay, |_, id, raw| {
        if id == later_section(TableId::Clock) {
            reader_tests::append_control(raw, TableId::Clock);
        }
    });
    for hour in [10, 12] {
        assert!(
            matches!(replay.seek_selected(1, hour, Some(&[TableId::Clock])).unwrap(), SeekResult::Frame(frame) if frame.hour == hour)
        );
        assert!(!replay.is_verified(&replay.index[0]));
    }
    assert!(
        matches!(replay.seek_selected(1, 15, Some(&[TableId::Clock])).unwrap(), SeekResult::Gap { reason, .. } if reason == "unreadable block: grouped section has unread column bytes")
    );
    assert!(!replay.is_verified(&replay.index[0]));
}

#[test]
fn point_cancellation_is_not_a_gap_and_drops_the_previous_hour() {
    let replay = test_recording::build(b"point-cancel", &[10, 11], |_, _| Metrics::default());
    let replay = Arc::new(replay);
    let mut reader = HourReader::new();
    reader
        .with_hour(
            &replay,
            1,
            10,
            ReaderSelection::new(&[]).unwrap(),
            || Ok(()),
            |_| (),
        )
        .unwrap();
    let error = reader
        .with_hour(
            &replay,
            1,
            11,
            ReaderSelection::new(&[]).unwrap(),
            || Err(io::ErrorKind::Interrupted.into()),
            |_| panic!("cancelled callback"),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn identical_and_forward_points_reuse_only_the_latest_hour_in_the_same_source() {
    let replay = Arc::new(test_recording::build(
        b"point-reuse",
        &[10, 11, 12, 13],
        |_, _| Metrics::default(),
    ));
    let mut reader = HourReader::new();
    let selection = ReaderSelection::new(&[TableId::Clock]).unwrap();
    let mut identity = None;
    reader
        .with_hour(
            &replay,
            1,
            11,
            selection.clone(),
            || Ok(()),
            |result| {
                let PointResult::Frame(frame) = result else {
                    panic!();
                };
                identity = Some(frame.tables[TableId::Clock.index()].identity());
            },
        )
        .unwrap();
    reader
        .with_hour(
            &replay,
            1,
            11,
            selection.clone(),
            || Ok(()),
            |result| {
                let PointResult::Frame(frame) = result else {
                    panic!();
                };
                assert!(
                    identity
                        .as_ref()
                        .unwrap()
                        .is(&frame.tables[TableId::Clock.index()])
                );
            },
        )
        .unwrap();
    reader
        .with_hour(
            &replay,
            1,
            12,
            selection,
            || Ok(()),
            |result| {
                let PointResult::Frame(frame) = result else {
                    panic!();
                };
                assert_eq!(frame.hour, 12);
            },
        )
        .unwrap();
    assert_eq!(reader.census().peak_raw_blocks, 1);
}

fn compare_legacy_seek(replay: Replay, hour: Hour) -> SeekResult {
    let reference = reader_tests::reference_copy(&replay);
    let expected = reader_reference::seek(
        &reference,
        1,
        hour,
        Some(&[TableId::Clock]),
        &mut Check::disabled(),
    )
    .unwrap();
    let actual = replay
        .seek_selected(1, hour, Some(&[TableId::Clock]))
        .unwrap();
    reader_tests::assert_seek(actual, expected);
    replay
        .seek_selected(1, hour, Some(&[TableId::Clock]))
        .unwrap()
}

#[test]
fn first_hour_compatibility_checks_checksum_column_directory_and_row_coding() {
    for damage in 0..3 {
        let replay =
            test_recording::build(b"first-hour-envelope", &[10, 12], |_, _| Metrics::default());
        let mut replay = reader_tests::rewrite(replay, |_, id, raw| {
            if id != later_section(TableId::Clock) {
                return;
            }
            match damage {
                1 => raw[0] = 0,
                2 => {
                    let input = ColumnReader::stored(
                        raw,
                        TableId::Clock.field_count(),
                        TableId::Clock,
                        FormatVersion::CURRENT.version(),
                    )
                    .unwrap();
                    let mut columns = Columns::new(TableId::Clock.field_count());
                    for (out, input) in columns.0.iter_mut().zip(input.0) {
                        out.extend_from_slice(input.rest());
                    }
                    assert!(!columns.0[MASKS].is_empty());
                    columns.0[MASKS][0] = 255;
                    *raw = columns.raw();
                }
                _ => {}
            }
        });
        if damage == 0 {
            let index = &replay.index[0];
            let directory = Directory::read(&replay, index).unwrap();
            let (offset, length, _) = directory.sections[later_section(TableId::Clock)].unwrap();
            // Flip only the zstd checksum; extents, directory CRC and keyframe stay valid.
            replay.bytes[index.offset as usize + offset + length - 1] ^= 1;
        }
        let reference = reader_tests::reference_copy(&replay);
        let expected = reader_reference::seek(
            &reference,
            1,
            10,
            Some(&[TableId::Clock]),
            &mut Check::disabled(),
        )
        .unwrap();
        assert!(
            matches!(&expected, SeekResult::Gap { reason, .. } if reason.starts_with("unreadable block: "))
        );
        reader_tests::assert_seek(
            replay
                .seek_selected(1, 10, Some(&[TableId::Clock]))
                .unwrap(),
            expected,
        );
        let mut point = HourReader::new();
        point
            .with_hour(
                &Arc::new(replay),
                1,
                10,
                ReaderSelection::new(&[TableId::Clock]).unwrap(),
                || Ok(()),
                |result| {
                    assert!(matches!(result, PointResult::Frame(frame) if frame.hour == 10));
                },
            )
            .unwrap();
    }
}

#[test]
fn point_peak_has_no_retained_anchor_copies_and_drops_previous_updates() {
    use super::super::reader::ownership::{Audit, Kind};
    let replay = Arc::new(reader_tests::ownership_fixture(64));
    let audit = Audit::new();
    let scope = audit.enter();
    let mut reader = HourReader::new();
    reader
        .with_hour(
            &replay,
            1,
            63,
            ReaderSelection::new(&[TableId::LexicalTokens]).unwrap(),
            || Ok(()),
            |result| {
                assert!(matches!(result, PointResult::Frame(frame) if frame.hour == 63));
                assert_eq!(audit.peak(Kind::Anchors), 0);
                assert_eq!(audit.live(Kind::Anchors), 0);
                assert!(audit.peak(Kind::Bootstrap) > 0);
                assert!(audit.peak_update_hours() <= 1);
                assert_eq!(audit.live_update_hours(), 1);
                audit.assert_unique_views();
            },
        )
        .unwrap();
    drop((reader, scope));
    assert_eq!(audit.live_update_hours(), 0);
    assert_eq!(audit.live_views(false), 0);
    audit.clear_dead_witnesses();
    assert_eq!(audit.live_total(), 0);
}

#[test]
fn hidden_dependency_allocations_and_worker_results_die_on_cancellation() {
    use super::super::reader::ownership::{Audit, Kind};
    let (replay, _) = direct_tests::alias_fixture(Layout::Chosen);
    let replay = Arc::new(replay);
    let audit = Audit::new();
    let scope = audit.enter();
    let mut reader = crate::container::SequentialReader::for_block(
        ReaderSelection::new(&[TableId::Divisions]).unwrap(),
        0,
        crate::container::ReaderExecution {
            workers: 3,
            read_ahead: true,
        },
    )
    .unwrap();
    assert!(matches!(
        reader.advance(&replay, || Ok(())).unwrap(),
        crate::container::ReadStep::Hour { .. }
    ));
    assert!(audit.peak(Kind::Hidden) > 0 && audit.live_views(true) > 0);
    assert!(audit.peak(Kind::Aliases) > 0);
    let error = reader
        .advance(&replay, || Err(io::ErrorKind::Interrupted.into()))
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(audit.live_views(true), 0);
    assert_eq!(audit.live_views(false), 0);
    assert_eq!(audit.live_update_hours(), 0);
    drop((reader, scope));
    audit.clear_dead_witnesses();
    assert_eq!(audit.live_total(), 0);
}
