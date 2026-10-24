use super::*;

fn fixture() -> Replay {
    let replay = test_recording::build(b"synthetic-through", &[10, 12, 13], |hour, _| {
        let mut table_ns = [0; TABLE_COUNT];
        table_ns[TableId::Clock.index()] = 1;
        Metrics {
            total_ns: hour as u64,
            table_ns,
            ..Default::default()
        }
    });
    assert_eq!(replay.index.len(), 1);
    replay
}

#[test]
fn selected_through_includes_equal_hours_and_stops_inside_gaps() {
    for selected in [None, Some(&[TableId::Clock][..]), Some(&[][..])] {
        let replay = fixture();
        let index = &replay.index[0];
        for (through, expected_hours) in [
            (9, &[][..]),
            (10, &[10][..]),
            (11, &[10][..]),
            (12, &[10, 12][..]),
        ] {
            let prefix = replay
                .read_block_selected_through(index, selected, through, None)
                .unwrap();
            assert_eq!(
                prefix.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
                expected_hours
            );
            assert!(
                !replay.is_verified(index),
                "a prefix must not certify later column bytes"
            );
        }
        let expected = match selected {
            Some(tables) => replay.read_block_selected(index, tables).unwrap(),
            None => replay.read_block(index).unwrap(),
        };
        for through in [9, 10, 11, 12, 13, 14] {
            let actual = replay
                .read_block_selected_through(index, selected, through, None)
                .unwrap();
            let prefix: Vec<_> = expected
                .iter()
                .filter(|frame| frame.hour <= through)
                .cloned()
                .collect();
            assert_eq!(actual, prefix);
            let workers = Parallel {
                workers: 2,
                on_worker: &|| {},
            };
            let parallel = replay
                .read_block_selected_through(index, selected, through, Some(&workers))
                .unwrap();
            assert_eq!(
                parallel, prefix,
                "parallel tables must decode the same prefix"
            );
        }
        let fresh = fixture();
        fresh
            .read_block_selected_through(&fresh.index[0], selected, 13, None)
            .unwrap();
        assert!(
            fresh.is_verified(&fresh.index[0]),
            "a whole read retains full verification"
        );
    }
}

/// Preserve section framing/checksums while making its unused column tail invalid.
fn with_extra_control(replay: &Replay) -> (BlockIndex, Vec<u8>) {
    let original = &replay.index[0];
    let bytes = replay.block_bytes(original).unwrap();
    let directory = Directory::parse(&bytes, original).unwrap();
    let target = later_section(TableId::Clock);
    assert!(directory.sections[target].is_some());
    let raw = directory.raw(&bytes, target).unwrap().unwrap();
    let reader = ColumnReader::stored(
        &raw,
        TableId::Clock.field_count(),
        TableId::Clock,
        replay.format.version(),
    )
    .unwrap();
    let mut columns = Columns::new(TableId::Clock.field_count());
    for (out, mut input) in columns.0.iter_mut().zip(reader.0) {
        out.extend_from_slice(input.take(input.remaining()).unwrap());
    }
    if let Some(marks) = reader.1 {
        columns.marks(TableId::Clock, marks).unwrap();
    }
    columns.0[CONTROL].push(255);
    replace_sections(replay, original, &[(target, columns.raw())]).unwrap()
}

#[test]
fn cached_prefix_does_not_admit_a_malformed_extension_or_lose_its_valid_prefix() {
    let replay = fixture();
    replay.cache_decoded_sections(true);
    let (index, bytes) = with_extra_control(&replay);
    let ((first, second), decodes) = super::super::test_work::table_decodes(|| {
        let prefix = || {
            read_through(
                &replay,
                &index,
                &bytes,
                Some(&[TableId::Clock]),
                None,
                Some(12),
            )
            .unwrap()
        };
        let first = prefix();
        assert!(
            read_through(
                &replay,
                &index,
                &bytes,
                Some(&[TableId::Clock]),
                None,
                Some(13)
            )
            .is_err()
        );
        (first, prefix())
    });
    assert_eq!(first, second);
    assert_eq!(
        decodes,
        [(1, 10, TableId::Clock, 2), (1, 10, TableId::Clock, 3)]
    );
    assert!(!replay.is_verified(&index));
}

#[test]
fn prefix_does_not_verify_later_grammar_but_whole_read_does() {
    let replay = fixture();
    let (index, bytes) = with_extra_control(&replay);
    verify(&bytes, &index).unwrap();
    let prefix = read_through(
        &replay,
        &index,
        &bytes,
        Some(&[TableId::Clock]),
        None,
        Some(12),
    )
    .unwrap();
    assert_eq!(
        prefix.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
        [10, 12]
    );
    assert!(
        read_through(
            &replay,
            &index,
            &bytes,
            Some(&[TableId::Clock]),
            None,
            Some(13)
        )
        .is_err()
    );
    // A section checksum failure remains a refusal, even when its unread tail
    // is where the damage lives.
    let directory = Directory::parse(&bytes, &index).unwrap();
    let (offset, length, _) = directory.sections[later_section(TableId::Clock)].unwrap();
    let mut damaged = bytes;
    damaged[offset + length - 1] ^= 1;
    assert!(
        read_through(
            &replay,
            &index,
            &damaged,
            Some(&[TableId::Clock]),
            None,
            Some(12)
        )
        .is_err()
    );
}
