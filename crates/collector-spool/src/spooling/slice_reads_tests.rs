use super::*;
use collector_certificates::slicing::SliceSelection;
use replay_data::rows::{ClockRow, SliceReadRow};
use replay_frame::frame::{FrameDecoder, TableUpdate};

fn original_merge(
    previous: &[(u32, i32)],
    owners: &[u32],
    selection: &SliceSelection,
    hour: i32,
) -> Vec<(u32, i32)> {
    let mut reads = Vec::new();
    for &owner in owners {
        let read = if selection.force_full || selection.contains(owner) {
            Some(hour)
        } else {
            previous
                .binary_search_by_key(&owner, |entry| entry.0)
                .ok()
                .map(|index| previous[index].1)
        };
        if let Some(read) = read {
            reads.push((owner, read));
        }
    }
    for owner in selection.read_owners() {
        if owners.binary_search(&owner).is_err() {
            reads.push((owner, hour));
        }
    }
    for &(owner, read) in previous {
        if !selection.force_full
            && !selection.contains(owner)
            && selection.live(owner)
            && owners.binary_search(&owner).is_err()
        {
            reads.push((owner, read));
        }
    }
    reads.sort_unstable_by_key(|entry| entry.0);
    reads.dedup_by_key(|entry| entry.0);
    reads
}

fn original_rows(
    held: &ReadHours,
    frame_reads: &[Option<Vec<(u32, i32)>>; TABLE_COUNT],
) -> TableRows {
    let mut reads = Vec::new();
    for (table, (view, base)) in frame_reads.iter().zip(&held.0).enumerate() {
        reads.extend(
            view.as_ref()
                .unwrap_or(base)
                .iter()
                .map(|&(owner, sample_hour)| SliceReadRow {
                    table: table as u32,
                    owner,
                    sample_hour,
                }),
        );
    }
    TableRows::SliceReads(reads)
}

fn frame(branch: BranchId, hour: i32, input: TableInput) -> FrameInput {
    let mut tables = TableId::ALL.map(|_| {
        TableView::Unavailable {
            reason: "outside slice provenance fixture".into(),
        }
        .into()
    });
    tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
        hour,
        ..Default::default()
    }]))
    .into();
    tables[TableId::SliceReads.index()] = input;
    let mut metrics = Metrics::default();
    metrics.table_ns[TableId::SliceReads.index()] = 1;
    FrameInput {
        branch,
        hour,
        tables,
        metrics,
    }
}

#[test]
fn ordered_slice_provenance_matches_original_for_all_small_owner_sets() {
    let table = TableId::ResearchInputs;
    let owners = [1, 2, 25, u32::MAX];
    let subset = |mask: u32| {
        owners
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, owner)| (mask & (1 << index) != 0).then_some(owner))
            .collect::<Vec<_>>()
    };
    let mut held = ReadHours::new();
    for hour in [1, 2] {
        for full in [false, true] {
            for census in 0..=16 {
                for early in 0..16 {
                    let mut selection = SliceSelection::for_test(hour, full, &subset(early));
                    if census != 16 {
                        selection = selection.with_census(&subset(census));
                    }
                    if early & 1 != 0 {
                        selection.track_reads();
                        for owner in subset(early & 10) {
                            selection.mark_read(owner);
                        }
                    }
                    for returned in 0..16 {
                        let returned = subset(returned);
                        for previous in 0..16 {
                            let previous: Vec<_> = subset(previous)
                                .into_iter()
                                .enumerate()
                                .map(|(index, owner)| (owner, -7 + index as i32))
                                .collect();
                            let expected = original_merge(&previous, &returned, &selection, hour);
                            held.0[table.index()] = previous;
                            held.merge(table, &returned, &selection, hour).unwrap();
                            assert_eq!(held.0[table.index()], expected);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn unordered_or_duplicate_returned_owners_normalize_before_provenance_merge() {
    let table = TableId::ResearchInputs;
    let mut held = ReadHours::new();
    for returned in [
        &[25, 1, 2][..],
        &[1, 1, 25],
        &[u32::MAX, 2, u32::MAX, 1],
        &[2, 1, 25, 2, 1],
    ] {
        let mut ordered = returned.to_vec();
        ordered.sort_unstable();
        ordered.dedup();
        for hour in [-3, 1, 2] {
            for full in [false, true] {
                for census in [false, true] {
                    for tracked in [false, true] {
                        let mut selection = SliceSelection::for_test(hour, full, &[1, 25]);
                        if census {
                            selection = selection.with_census(&[1, 2, 25, 26, u32::MAX]);
                        }
                        if tracked {
                            selection.track_reads();
                            selection.mark_read(1);
                        }
                        let previous = vec![(1, -7), (2, -6), (25, -5), (26, -4)];
                        let expected = original_merge(&previous, &ordered, &selection, hour);
                        held.0[table.index()] = previous;
                        held.merge(table, returned, &selection, hour).unwrap();
                        assert_eq!(held.0[table.index()], expected);
                    }
                }
            }
        }
    }
}

#[test]
fn sparse_slice_reads_match_complete_spool_bytes_through_keyframes() {
    let tables = [
        TableId::ResearchInputs,
        TableId::Projects,
        TableId::StateResources,
    ];
    let mut owners: Vec<_> = (1..=60).collect();
    let mut held = ReadHours::new();
    let mut original = ReadHours::new();
    let mut encoder = FrameEncoder::new();
    let mut reference = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut saw = [false; 3];
    for hour in 0..=721 {
        match hour {
            40 => {
                owners.retain(|&owner| owner != 25);
                owners.push(61);
            }
            50 => {
                owners.pop();
            }
            60 => owners.push(62),
            _ => {}
        }
        for table in tables {
            let full = hour == 0 || hour == 48;
            let early = if hour == 40 { &[61][..] } else { &[] };
            let mut selection = SliceSelection::for_test(hour, full, early);
            if hour % 17 == 0 && hour % 24 != 0 {
                selection.reset_daily(hour, false);
            }
            if hour % 11 != 0 || full {
                selection = selection.with_census(&owners);
            }
            let returned: Vec<_> = owners
                .iter()
                .copied()
                .filter(|&owner| selection.contains(owner) && hour % 19 != 0)
                .collect();
            original.0[table.index()] =
                original_merge(&original.0[table.index()], &returned, &selection, hour);
            held.merge(table, &returned, &selection, hour).unwrap();
            assert_eq!(held.0[table.index()], original.0[table.index()]);
        }
        let mut frame_reads = std::array::from_fn(|_| None);
        if hour % 31 == 0 {
            let table = tables[0].index();
            let mut partial = original.0[table].clone();
            if let Some(row) = partial.first_mut() {
                row.1 = hour;
            }
            frame_reads[table] = Some(partial);
            frame_reads[tables[1].index()] = Some(Vec::new());
        }
        let expected = original_rows(&original, &frame_reads);
        let input = held.input(&frame_reads, &encoder, 1, hour, false).unwrap();
        if hour == 40 {
            assert_eq!(
                encoder.committed::<tables::SliceReads>().unwrap().1.len(),
                expected.len()
            );
            assert!(matches!(input, TableInput::Sampled(TableView::Complete(_))));
        }
        match &input {
            TableInput::Sampled(TableView::Complete(_)) => saw[0] = true,
            TableInput::ComparedEqual(_) => saw[1] = true,
            TableInput::ComparedChanges { indices, rows, .. } => {
                assert_eq!(indices.len(), rows.len());
                saw[2] = true;
            }
            _ => panic!("unexpected SliceReads input"),
        }
        if hour == 0 || hour == 720 {
            assert!(matches!(input, TableInput::Sampled(TableView::Complete(_))));
        }
        let actual = encoder.encode_bytes(frame(1, hour, input), false).unwrap();
        let ordinary = reference
            .encode_bytes(
                frame(
                    1,
                    hour,
                    TableInput::SharedComplete(Arc::new(expected.clone())),
                ),
                false,
            )
            .unwrap();
        let decoded = actual.decode().unwrap();
        assert_eq!(decoded.keyframe, hour == 0 || hour == 720);
        let reconstructed = decoder.advance(&decoded).unwrap();
        assert_eq!(
            *reconstructed.tables[TableId::SliceReads.index()],
            TableView::Complete(expected),
            "hour {hour}"
        );
        assert_eq!(
            actual.into_spool_record().unwrap(),
            ordinary.into_spool_record().unwrap(),
            "hour {hour}"
        );
        assert!(encoder.shared_baseline(TableId::SliceReads).is_none());
    }
    assert_eq!(saw, [true; 3]);
}

#[test]
fn slice_reads_recovery_gaps_branches_and_dropped_publications_keep_complete_fallback() {
    let mut held = ReadHours::new();
    held.0[TableId::ResearchInputs.index()] = vec![(1, -2), (25, -2)];
    let frame_reads = std::array::from_fn(|_| None);
    let mut encoder = FrameEncoder::new();
    let mut reference = FrameEncoder::new();
    for (branch, hour, force, publish, sparse) in [
        (1, 0, false, true, false),
        (1, 1, false, false, true),
        (1, 2, false, true, false),
        (1, 3, true, true, false),
        (1, 4, false, true, true),
        (2, 5, false, true, false),
        (2, 7, false, true, false),
    ] {
        held.0[TableId::ResearchInputs.index()][0].1 = hour;
        let input = held
            .input(&frame_reads, &encoder, branch, hour, force)
            .unwrap();
        assert_eq!(matches!(input, TableInput::ComparedChanges { .. }), sparse);
        let expected = original_rows(&held, &frame_reads);
        let mut actual = encoder
            .prepare_bytes(frame(branch, hour, input), force)
            .unwrap();
        let mut ordinary = reference
            .prepare_bytes(
                frame(branch, hour, TableInput::SharedComplete(Arc::new(expected))),
                force,
            )
            .unwrap();
        assert_eq!(
            actual.take_frame().into_spool_record().unwrap(),
            ordinary.take_frame().into_spool_record().unwrap()
        );
        if publish {
            encoder.commit(actual).unwrap();
            reference.commit(ordinary).unwrap();
        }
    }
    encoder
        .encode_bytes(
            frame(2, 8, unavailable("slice provenance fixture refusal").into()),
            false,
        )
        .unwrap();
    let fresh = held.input(&frame_reads, &encoder, 2, 9, false).unwrap();
    assert!(matches!(fresh, TableInput::Sampled(TableView::Complete(_))));
    let encoded = encoder.encode_bytes(frame(2, 9, fresh), false).unwrap();
    assert!(matches!(
        encoded.decode().unwrap().tables[TableId::SliceReads.index()],
        TableUpdate::Snapshot(_)
    ));
    encoder = FrameEncoder::new();
    assert!(matches!(
        held.input(&frame_reads, &encoder, 2, 10, true).unwrap(),
        TableInput::Sampled(TableView::Complete(_))
    ));
}

#[test]
fn slice_reads_allocation_failures_preserve_held_and_committed_provenance() {
    let table = TableId::ResearchInputs;
    let mut held = ReadHours::new();
    held.0[table.index()] = vec![(1, 0), (2, 0)];
    let selection = SliceSelection::for_test(1, false, &[1]).with_census(&[1, 2]);
    let failed =
        collector_test_support::with_allocations_denied(|| held.merge(table, &[1], &selection, 1));
    assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
    assert_eq!(held.0[table.index()], [(1, 0), (2, 0)]);
    for ordinal in [1, 2] {
        let failed = {
            let _denied = collector_test_support::NullGuard::new(ordinal);
            held.merge(table, &[2, 1, 2], &selection, 1)
        };
        assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(held.0[table.index()], [(1, 0), (2, 0)]);
    }
    let frame_reads = std::array::from_fn(|_| None);
    let mut encoder = FrameEncoder::new();
    let failed = collector_test_support::with_allocations_denied(|| {
        held.input(&frame_reads, &encoder, 1, 0, false)
    });
    assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
    let initial = held.input(&frame_reads, &encoder, 1, 0, false).unwrap();
    encoder.encode_bytes(frame(1, 0, initial), false).unwrap();
    let quiet = collector_test_support::with_allocations_denied(|| {
        held.input(&frame_reads, &encoder, 1, 1, false)
    })
    .unwrap();
    assert!(matches!(quiet, TableInput::ComparedEqual(_)));
    held.merge(table, &[1], &selection, 1).unwrap();
    for ordinal in [1, 2] {
        let failed = {
            let _denied = collector_test_support::NullGuard::new(ordinal);
            held.input(&frame_reads, &encoder, 1, 1, false)
        };
        assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(encoder.committed::<tables::SliceReads>().unwrap().0, 0);
        assert_eq!(
            encoder.committed::<tables::SliceReads>().unwrap().1[0].sample_hour,
            0
        );
    }
    let retry = held.input(&frame_reads, &encoder, 1, 1, false).unwrap();
    assert!(
        matches!(&retry, TableInput::ComparedChanges { indices, rows, .. }
        if indices == &[0] && rows.len() == 1)
    );
    encoder.encode_bytes(frame(1, 1, retry), false).unwrap();
    assert_eq!(
        encoder.committed::<tables::SliceReads>().unwrap().1[0].sample_hour,
        1
    );
}
