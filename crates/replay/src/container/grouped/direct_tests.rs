//! Fixed row/byte expectations and independently encoded grouped fixtures.
use super::*;
use crate::rows::{
    ArmyOrderRow, CountryRow, DivisionMovementRow, DivisionRow, Kind, LexicalTokenRow,
};

pub(super) const LAYOUTS: [Layout; 10] = [
    Layout::Timed,
    Layout::Unpredicted,
    Layout::Predicted,
    Layout::Anchored,
    Layout::Keyed,
    Layout::Ramped,
    Layout::Modelled,
    Layout::Trimmed,
    Layout::Advanced,
    Layout::Chosen,
];

fn plain_delta(table: TableId, mask: u64, values: &[u8]) -> Vec<u8> {
    let mut bytes = vec![DELTA];
    bytes.extend_from_slice(&9_i32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    put_varint(&mut bytes, 0);
    bytes.extend_from_slice(&mask.to_le_bytes()[..table.mask_width()]);
    bytes.extend_from_slice(values);
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes
}

pub(super) fn encoded_delta(table: TableId, update: &TableUpdate) -> Vec<u8> {
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = update
    else {
        panic!("not a delta")
    };
    let mut out = vec![DELTA];
    out.extend_from_slice(&baseline.to_le_bytes());
    out.extend_from_slice(&(changed.len() as u32).to_le_bytes());
    let mut previous = 0;
    for row in changed {
        crate::rows::encode_delta(table, row, previous, &mut out).unwrap();
        previous = row.key;
    }
    out.extend_from_slice(&(deleted.len() as u32).to_le_bytes());
    let mut previous = 0;
    for &key in deleted {
        table.encode_key(key, previous, &mut out).unwrap();
        previous = key;
    }
    out
}

fn change(kind: Kind, round: u32) -> Vec<u8> {
    match kind {
        Kind::Bytes | Kind::Opaque => {
            let mut bytes = 3_u32.to_le_bytes().to_vec();
            bytes.extend_from_slice(&[round as u8, 1, 255]);
            bytes
        }
        Kind::U32List | Kind::U64List => {
            let mut bytes = 2_u32.to_le_bytes().to_vec();
            for value in [round + 1, round + 2] {
                if matches!(kind, Kind::U32List) {
                    bytes.extend(value.to_le_bytes());
                } else {
                    bytes.extend(u64::from(value).to_le_bytes());
                }
            }
            bytes
        }
        Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64 => {
            if round == 0 { vec![0] } else { vec![1, 2] }
        }
        _ => vec![if round == 0 { 1 } else { 2 }], // exact -1 / +1 differences
    }
}

#[test]
fn direct_delta_every_registered_field_matches_update_and_reencoded_bytes() {
    let format = FormatVersion::CURRENT.version();
    let mut checked = 0;
    for table in TableId::ALL {
        if !table.last_change().present(format)
            || table.last_change().table_gap(table, format).is_some()
        {
            continue;
        }
        for (field, &kind) in table.schema().iter().enumerate() {
            for round in 0..2 {
                let values = change(kind, round);
                let plain = plain_delta(table, 1 << field, &values);
                let expected = TableUpdate::Delta {
                    baseline: 9,
                    changed: vec![crate::rows::DeltaRow {
                        format,
                        key: 0,
                        mask: 1 << field,
                        recorded_fields: table.field_count(),
                        values: values.clone(),
                    }],
                    deleted: Vec::new(),
                };
                let mut shadow = Shadow::new();
                let mut output =
                    direct::Delta::new(table, table.field_count(), format, 9, &mut shadow);
                let pointer = values.as_ptr();
                output.row(0, 1 << field, values).unwrap();
                output.deleted(Vec::new()).unwrap();
                assert_eq!(output.len(), plain.len());
                let actual = output.finish();
                assert_eq!(actual, expected, "{table:?}, field {field}, round {round}");
                let TableUpdate::Delta { changed, .. } = &actual else {
                    unreachable!()
                };
                assert_eq!(
                    changed[0].values.as_ptr(),
                    pointer,
                    "the row buffer must transfer, not copy"
                );
                assert_eq!(encoded_delta(table, &actual), plain);
                let mut expected_shadow = Shadow::new();
                let TableUpdate::Delta { changed, .. } = &expected else {
                    unreachable!()
                };
                if table == TableId::DivisionMovement && field == 0 {
                    // A -1 insertion has no valid u32 head; +1 seeds it.
                    let fields = expected_shadow.entry(0).or_default();
                    if round == 1 {
                        fields.insert(0, 1_u32.to_le_bytes().to_vec());
                    }
                } else if table.byte_field(field) {
                    expected_shadow
                        .entry(0)
                        .or_default()
                        .insert(field, changed[0].values.clone());
                }
                assert_eq!(shadow, expected_shadow, "{table:?}, field {field}");
                checked += 1;
            }
        }
    }
    assert!(
        checked > TableId::ALL.len(),
        "the fixture must cover fields, not just table refusals"
    );
}

fn fixture(layout: Layout) -> (Replay, Vec<Frame>) {
    let old_layout = WRITTEN_LAYOUT.replace(layout);
    let old_residual = FORCE_RESIDUAL.replace(true);
    let hours: Vec<_> = (10..42).chain(730..735).collect();
    let mut encoder = crate::frame::FrameEncoder::new();
    let mut expected = Vec::new();
    let replay = test_recording::build(b"direct-delta-parity", &hours, |hour, tables| {
        let owners: &[u32] = if hour < 21 { &[1, 3, 5] } else { &[2, 3, 7] };
        tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            owners
                .iter()
                .map(|&id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("tag-{id}-{}", hour / 4).into_bytes(),
                    command_power_raw: i64::from(hour) * 100_000,
                    ..Default::default()
                })
                .collect(),
        ));
        tables[TableId::Divisions.index()] = if hour == 24 {
            TableView::Unavailable {
                reason: "synthetic read refusal".into(),
            }
        } else if hour == 26 {
            TableView::Truncated {
                rows: TableRows::Divisions(Vec::new()),
                missing: 2,
            }
        } else {
            TableView::Complete(TableRows::Divisions(
                owners
                    .iter()
                    .map(|&id| DivisionRow {
                        id: u64::from(id),
                        province: hour as u32,
                        organisation: i64::from(hour * hour) * 10_000,
                        ..Default::default()
                    })
                    .collect(),
            ))
        };
        let phase = (hour as u32 - 10) % 14;
        tables[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: 1,
                source: 100 + phase.saturating_sub(2),
                path: (phase..14).map(|p| 100 + p).collect(),
                numerator: i64::from(hour) * 100_000,
                ..Default::default()
            }]));
        tables[TableId::ArmyOrders.index()] =
            TableView::Complete(TableRows::ArmyOrders(vec![ArmyOrderRow {
                owner_group: 1,
                instance: 1,
                path: (hour as u32..44).collect(),
                ..Default::default()
            }]));
        tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: format!("literal-{hour}").into_bytes(),
            }]));
        let frame = crate::frame::retire_absent(DecodedFrame {
            branch: 1,
            hour,
            tables: std::array::from_fn(|at| tables[at].clone().into()),
            metrics: Metrics::default(),
        });
        // Ordinary frame encoding is independent of grouped reconstruction.
        expected.push(encoder.encode(frame, false).unwrap());
        Metrics::default()
    });
    WRITTEN_LAYOUT.set(old_layout);
    FORCE_RESIDUAL.set(old_residual);
    (replay, expected)
}

fn compare(replay: &Replay, expected: &[Frame], index: &BlockIndex, bytes: &[u8], through: usize) {
    let directory = Directory::parse(bytes, index).unwrap();
    let present = Present::new(replay.format.version(), replay.capture_mode);
    let header = directory.raw(bytes, HEADER).unwrap().unwrap();
    let (hours, entries) = read_header(&header, index, directory.frames, &present).unwrap();
    let memo: Vec<_> = TableId::ALL.iter().map(|_| OnceLock::new()).collect();
    let context = Context {
        replay,
        index,
        position: replay.block_position(index),
        block: bytes,
        directory: &directory,
        format: replay.format,
        present: &present,
        hours: &hours[..through],
        entries: &entries,
        selected: None,
        complete: through == hours.len(),
        memo: &memo,
        group: &OnceLock::new(),
    };
    let expected: Vec<_> = expected
        .iter()
        .filter(|frame| frame.hour >= index.first_hour && frame.hour <= index.last_hour)
        .take(through)
        .collect();
    assert_eq!(expected.len(), through);
    let mut actual: Vec<Frame> = expected.iter().map(|frame| (**frame).clone()).collect();
    for &table in &present.tables {
        if keyframe_refusal(&directory, replay.format, replay.capture_mode, None, table)
            .unwrap()
            .is_some()
        {
            continue;
        }
        let updates = table_updates_inner(&context, table).unwrap();
        assert_eq!(updates.len(), through);
        let admitted = table_updates_admitted(&context, table).unwrap();
        assert_eq!(admitted.len(), through);
        for (at, update) in updates.iter().enumerate() {
            assert_eq!(**admitted.get(at), *update);
        }
        let memoized = table_updates_admitted(&context, table).unwrap();
        assert!(Arc::ptr_eq(admitted.get(0), memoized.get(0)));
        for ((actual, expected), update) in actual.iter_mut().zip(&expected).zip(updates) {
            assert_eq!(
                update,
                expected.tables[table.index()],
                "{:?}, {table:?}, hour {}, prefix {through}",
                directory.layout,
                expected.hour
            );
            actual.tables[table.index()] = update;
        }
    }
    for (expected, actual) in expected.iter().zip(&actual) {
        let mut a = Vec::new();
        let mut b = Vec::new();
        expected.encode(&mut a).unwrap();
        actual.encode(&mut b).unwrap();
        assert_eq!(a, b, "whole-frame re-encoded bytes at {}", expected.hour);
    }
}

#[test]
fn direct_delta_all_admitted_grouped_layouts_and_prefixes_match_every_update_and_byte() {
    for layout in LAYOUTS {
        let (replay, expected) = fixture(layout);
        assert!(
            replay.index.len() >= 2,
            "parity includes keyframe restarts/anchors"
        );
        for index in &replay.index {
            let bytes = replay.block_bytes(index).unwrap();
            let count = Directory::parse(&bytes, index).unwrap().frames;
            for through in [1, 3, 11, count].into_iter().filter(|n| *n <= count) {
                compare(&replay, &expected, index, &bytes, through);
            }
        }
    }
}

#[test]
fn direct_delta_malformed_field_encodings_and_keys_are_never_admitted() {
    let format = FormatVersion::CURRENT.version();
    for table in TableId::ALL {
        if !table.last_change().present(format)
            || table.last_change().table_gap(table, format).is_some()
        {
            continue;
        }
        for (field, &kind) in table.schema().iter().enumerate() {
            let values = change(kind, 1);
            // Every truncation of every kind, including option/length/list framing.
            for length in 0..values.len() {
                let mut shadow = Shadow::new();
                let mut output =
                    direct::Delta::new(table, table.field_count(), format, 9, &mut shadow);
                let actual = output
                    .row(0, 1 << field, values[..length].to_vec())
                    .and_then(|()| output.deleted(Vec::new()));
                let error = actual.expect_err("a truncated field cannot be admitted");
                assert_eq!(
                    error.kind(),
                    io::ErrorKind::InvalidData,
                    "{table:?}, field {field}, cut {length}"
                );
            }
        }
        for mask in [0, !crate::rows::full_mask(table.field_count())] {
            if mask == 0 || table.field_count() < 64 {
                let mut shadow = Shadow::new();
                let mut output =
                    direct::Delta::new(table, table.field_count(), format, 9, &mut shadow);
                assert!(output.row(0, mask, Vec::new()).is_err());
            }
        }
        let mut shadow = Shadow::new();
        let mut output = direct::Delta::new(table, table.field_count(), format, 9, &mut shadow);
        let values = change(table.schema()[0], 1);
        output.row(0, 1, values.clone()).unwrap();
        let error = output.row(0, 1, values).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            if table.key_bits() == 0 {
                "singleton table cannot contain multiple rows"
            } else {
                "delta keys must be unique and ascending"
            },
            "duplicate {table:?}"
        );
        assert!(
            output.deleted(vec![0]).is_err(),
            "changed and deleted {table:?}"
        );
    }
}

#[test]
fn direct_delta_changed_count_refusal_matches_the_row_coding_before_reading_rows() {
    let table = TableId::Divisions;
    let fields = table.field_count();
    for layout in LAYOUTS {
        for modelled in [false, true] {
            if !layout.chooses_rows() && modelled != layout.modelled() {
                continue;
            }
            let mut input = ColumnReader::empty(fields);
            input.0[CONTROL] = Decoder::new(&[1]);
            // Missing keys, masks and fields: the count refusal comes first.
            let keys = KeySet::default();
            let mut masks = MaskList::default();
            let mut model = modelled.then(|| Model::new(fields));
            let coding = layout.keyed().then_some(RowCoding {
                keys: &keys,
                masks: &mut masks,
                model: model.as_mut(),
                changes: None,
            });
            let error = direct::take_delta(
                table,
                fields,
                FormatVersion::CURRENT.version(),
                9,
                &mut input,
                &mut Shadow::new(),
                &Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES)),
                layout,
                coding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                error.to_string(),
                if modelled {
                    "grouped row count exceeds its rows"
                } else {
                    "grouped row count exceeds its keys"
                },
                "{layout:?}, modelled {modelled}"
            );
        }
    }
}

#[test]
fn direct_delta_singleton_changed_count_precedes_duplicate_keys() {
    let table = TableId::Clock;
    assert_eq!(table.key_bits(), 0);
    assert!(!table.byte_field(0));
    let fields = table.field_count();
    let values = change(table.schema()[0], 1).repeat(2);
    for layout in LAYOUTS.into_iter().filter(|layout| !layout.keyed()) {
        let mut input = ColumnReader::empty(fields);
        input.0[CONTROL] = Decoder::new(&[2, 0]);
        input.0[KEYS] = Decoder::new(&[0, 0]);
        let masks = 1_u64.to_le_bytes()[..table.mask_width()].repeat(2);
        input.0[MASKS] = Decoder::new(&masks);
        input.0[FIELDS] = Decoder::new(&values);
        let error = direct::take_delta(
            table,
            fields,
            FormatVersion::CURRENT.version(),
            9,
            &mut input,
            &mut Shadow::new(),
            &Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES)),
            layout,
            None,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "singleton table cannot contain multiple rows",
            "{layout:?}"
        );
    }
}

/// Exercise the section retry boundary, not just a direct row validator:
/// body framing owns precedence over wire checks and uses frame-before bases.
#[test]
fn direct_delta_section_refusals_keep_original_framing_and_wire_precedence() {
    let mut checked = 0;
    for layout in LAYOUTS.into_iter().filter(|layout| !layout.keyed()) {
        let old_layout = WRITTEN_LAYOUT.replace(layout);
        let old_residual = FORCE_RESIDUAL.replace(false);
        let replay =
            test_recording::build(b"direct-refusal-precedence", &[10, 11], |hour, tables| {
                // Key zero has no Countries patch base at the frame before.
                tables[TableId::Countries.index()] =
                    TableView::Complete(TableRows::Countries(vec![CountryRow {
                        id: 1,
                        tag: format!("held-{hour}").into_bytes(),
                        ..Default::default()
                    }]));
                tables[TableId::Divisions.index()] =
                    TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                        id: 1,
                        province: hour as u32,
                        ..Default::default()
                    }]));
                Metrics::default()
            });
        WRITTEN_LAYOUT.set(old_layout);
        FORCE_RESIDUAL.set(old_residual);
        let clock_value = change(TableId::Clock.schema()[0], 1);
        let division_value = change(TableId::Divisions.schema()[0], 1);
        let cases = [
            (
                "singleton first key is out of width",
                TableId::Clock,
                vec![2, 0],
                vec![1_u128, 0],
                vec![1_u64, 1],
                0,
                clock_value.repeat(2),
                "singleton table cannot contain multiple rows",
            ),
            (
                "singleton has a truncated deleted count",
                TableId::Clock,
                vec![2],
                vec![0, 0],
                vec![1, 1],
                0,
                clock_value.repeat(2),
                "truncated field",
            ),
            (
                "duplicate changed key before a missing deleted key",
                TableId::Divisions,
                vec![2, 1],
                vec![0, 0],
                vec![1, 1],
                0,
                division_value.repeat(2),
                "grouped row count exceeds its keys",
            ),
            (
                "wide first key before a zero mask",
                TableId::Divisions,
                vec![2, 0],
                vec![1_u128 << 64, 1],
                vec![1, 0],
                0,
                division_value.clone(),
                "invalid recorded delta mask",
            ),
            (
                "plain then patch uses frame-before bases",
                TableId::Countries,
                vec![2, 0],
                vec![0, 0],
                vec![2, 2],
                1,
                vec![0, 1, 0, 0, 0, b'a', 1, 1, 4, 1, 1, b'b'],
                "grouped patch has no base",
            ),
            (
                "deleted ordering before changed/deleted overlap",
                TableId::Divisions,
                vec![1, 2],
                vec![0, 0, 0],
                vec![1],
                0,
                division_value.clone(),
                "delta keys must be unique and ascending",
            ),
            (
                "deleted width before later accumulation overflow",
                TableId::Divisions,
                vec![0, 2],
                vec![1_u128 << 64, u128::MAX],
                vec![],
                0,
                vec![],
                "row key exceeds native width",
            ),
            (
                "singleton deleted count before key width and overflow",
                TableId::Clock,
                vec![0, 2],
                vec![1_u128 << 64, u128::MAX],
                vec![],
                0,
                vec![],
                "singleton table cannot contain multiple rows",
            ),
        ];
        for (name, table, control, differences, masks, field, values, expected) in cases {
            let mut columns = Columns::new(table.field_count());
            columns.0[CONTROL] = vec![DELTA, 0]; // baseline is the preceding hour
            columns.0[CONTROL].extend(control);
            for key in differences {
                put_varint(&mut columns.0[KEYS], key);
            }
            for mask in masks {
                columns.0[MASKS].extend_from_slice(&mask.to_le_bytes()[..table.mask_width()]);
            }
            columns.0[FIELDS + field] = values;
            columns.marks(table, &[]).unwrap();
            let (index, bytes) = replace_columns(&replay, table, &columns.raw());
            let directory = Directory::parse(&bytes, &index).unwrap();
            let present = Present::new(replay.format.version(), replay.capture_mode);
            let header = directory.raw(&bytes, HEADER).unwrap().unwrap();
            let (hours, entries) =
                read_header(&header, &index, directory.frames, &present).unwrap();
            assert!(get(
                &entries[present.width()..],
                present.ordinal[table.index()]
            ));
            let memo: Vec<_> = TableId::ALL.iter().map(|_| OnceLock::new()).collect();
            let context = Context {
                replay: &replay,
                index: &index,
                position: Some(0),
                block: &bytes,
                directory: &directory,
                format: replay.format,
                present: &present,
                hours: &hours,
                entries: &entries,
                selected: None,
                complete: true,
                memo: &memo,
                group: &OnceLock::new(),
            };
            let original = table_updates_body(&context, table, false).unwrap_err();
            assert_eq!(
                original.kind(),
                io::ErrorKind::InvalidData,
                "{layout:?}: {name}"
            );
            assert_eq!(original.to_string(), expected, "{layout:?}: {name}");
            // A failed fast attempt must retry from the section's initial state.
            assert!(
                table_updates_body(&context, table, true).is_err(),
                "{layout:?}: {name}"
            );
            let actual = table_updates_inner(&context, table).unwrap_err();
            assert_eq!(actual.kind(), original.kind(), "{layout:?}: {name}");
            assert_eq!(actual.to_string(), expected, "{layout:?}: {name}");
            checked += 1;
        }
    }
    assert_eq!(checked, 32);
}

fn replace_columns(replay: &Replay, table: TableId, raw: &[u8]) -> (BlockIndex, Vec<u8>) {
    let original = &replay.index[0];
    let bytes = replay.block_bytes(original).unwrap();
    let directory = Directory::parse(&bytes, original).unwrap();
    let mut sections = Vec::new();
    for (id, section) in directory.sections.iter().enumerate() {
        let Some(&(offset, length, raw_length)) = section.as_ref() else {
            continue;
        };
        if id == later_section(table) {
            sections.push((id, raw.len(), compress(raw).unwrap()));
        } else {
            sections.push((id, raw_length, bytes[offset..offset + length].to_vec()));
        }
    }
    let mut header = Vec::new();
    put_varint(&mut header, directory.frames as u128);
    put_varint(&mut header, sections.len() as u128);
    for (id, raw, stored) in &sections {
        put_varint(&mut header, *id as u128);
        put_varint(&mut header, *raw as u128);
        put_varint(&mut header, stored.len() as u128);
    }
    let mut bytes = directory.layout.magic().to_vec();
    bytes.extend_from_slice(&crc32fast::hash(&header).to_le_bytes());
    bytes.extend_from_slice(&header);
    let raw_length = sections.iter().map(|(_, raw, _)| *raw as u64).sum();
    for (_, _, stored) in sections {
        bytes.extend(stored);
    }
    let mut index = original.clone();
    index.length = bytes.len() as u64;
    index.raw_length = raw_length;
    (index, bytes)
}

/// Oversized frame counts must not wrap into a valid signed-hour range.
#[test]
fn a_frame_count_beyond_any_hour_range_is_refused_before_decoding() {
    let (replay, _) = fixture(LAYOUTS[0]);
    let original = &replay.index[0];
    let bytes = replay.block_bytes(original).unwrap();
    let directory = Directory::parse(&bytes, original).unwrap();
    let sections: Vec<_> = directory
        .sections
        .iter()
        .enumerate()
        .filter_map(|(id, section)| section.as_ref().map(|&section| (id, section)))
        .collect();
    let mut header = Vec::new();
    put_varint(&mut header, 1_u128 << 63);
    put_varint(&mut header, sections.len() as u128);
    for &(id, (_, length, raw)) in &sections {
        put_varint(&mut header, id as u128);
        put_varint(&mut header, raw as u128);
        put_varint(&mut header, length as u128);
    }
    let mut crafted = directory.layout.magic().to_vec();
    crafted.extend_from_slice(&crc32fast::hash(&header).to_le_bytes());
    crafted.extend_from_slice(&header);
    for &(_, (offset, length, _)) in &sections {
        crafted.extend_from_slice(&bytes[offset..offset + length]);
    }
    let mut index = original.clone();
    index.length = crafted.len() as u64;
    for last_hour in [index.first_hour, index.last_hour] {
        index.last_hour = last_hour;
        let error = Directory::parse(&crafted, &index).err().unwrap();
        assert_eq!(
            error.to_string(),
            "grouped frame count disagrees with its hour range"
        );
    }
}

#[test]
fn direct_delta_malformed_columns_have_fixed_refusals_in_every_layout() {
    let mut rejected = 0;
    for layout in LAYOUTS {
        let (replay, _) = fixture(layout);
        for table in [TableId::Divisions, TableId::ArmyOrders, TableId::Countries] {
            let original = &replay.index[0];
            let bytes = replay.block_bytes(original).unwrap();
            let directory = Directory::parse(&bytes, original).unwrap();
            let raw = directory
                .raw(&bytes, later_section(table))
                .unwrap()
                .unwrap();
            let reader =
                ColumnReader::stored(&raw, table.field_count(), table, replay.format.version())
                    .unwrap();
            let marks = reader.1;
            let columns: Vec<Vec<u8>> = reader
                .0
                .into_iter()
                .map(|column| column.rest().to_vec())
                .collect();
            for column in 0..columns.len() {
                // Appending cannot change a valid prefix's operations. It
                // must fail at the first complete-section finish, even for
                // an originally empty field column.
                let mut changed = Columns::new(table.field_count());
                changed.0.clone_from(&columns);
                changed.0[column].push(255);
                changed.marks(table, marks.unwrap_or(&[])).unwrap();
                let (index, bytes) = replace_columns(&replay, table, &changed.raw());
                let directory = Directory::parse(&bytes, &index).unwrap();
                let present = Present::new(replay.format.version(), replay.capture_mode);
                let header = directory.raw(&bytes, HEADER).unwrap().unwrap();
                let (hours, entries) =
                    read_header(&header, &index, directory.frames, &present).unwrap();
                let memo: Vec<_> = TableId::ALL.iter().map(|_| OnceLock::new()).collect();
                let context = Context {
                    replay: &replay,
                    index: &index,
                    position: Some(0),
                    block: &bytes,
                    directory: &directory,
                    format: replay.format,
                    present: &present,
                    hours: &hours,
                    entries: &entries,
                    selected: None,
                    complete: true,
                    memo: &memo,
                    group: &OnceLock::new(),
                };
                let error = table_updates_inner(&context, table).unwrap_err();
                rejected += 1;
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert_eq!(
                    error.to_string(),
                    "grouped section has unread column bytes",
                    "{layout:?}, {table:?}, column {column}"
                );
            }
        }
    }
    assert!(rejected > 100, "every layout must exercise real refusals");
}

/// The alias fixture over two blocks whose first hours hold every row.
pub(super) fn alias_fixture(layout: Layout) -> (Replay, Vec<Frame>) {
    let hours: Vec<_> = (10..26).chain(730..742).collect();
    alias_fixture_at(layout, &hours)
}

/// Expected frames are independently encoded native values; hours ending in 7 have no alias rows.
fn alias_fixture_at(layout: Layout, hours: &[Hour]) -> (Replay, Vec<Frame>) {
    use crate::rows::*;
    let old_layout = WRITTEN_LAYOUT.replace(layout);
    let old_residual = FORCE_RESIDUAL.replace(true);
    let mut encoder = crate::frame::FrameEncoder::new();
    let mut expected = Vec::new();
    let replay = test_recording::build(b"direct-alias-parity", hours, |hour, tables| {
        let value = 100_000 + i64::from(hour) * 10_000;
        let panel = DivisionPanelStatsRow {
            division: 7,
            panel_stats: std::array::from_fn(|_| Some(value)),
            suppression: Some(value).into(),
        };
        let mut status = DivisionStatusRow {
            division: 7,
            maximum_hit_points: DivisionStatusRow::MAXIMUM_STEP.round(value),
            maximum_organisation: DivisionStatusRow::MAXIMUM_STEP.round(value),
            supply_ratio: DivisionStatusRow::SUPPLY_RATIO_STEP.round(value + 123),
            supply_ratio_exact: Some(value + 123).into(),
            supply_grace_remaining: DivisionStatusRow::displayed_grace(value, Some(143)).into(),
            supply_grace_exact: Some(value).into(),
            organisation_exact: Some(value).into(),
            strength_factor: value,
            movement_stats: Some(DivisionMovementStats {
                air_defense: value,
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        let mut movement = DivisionMovementRow {
            division: 7,
            source: 100 + hour as u32,
            path: vec![101 + hour as u32, 102 + hour as u32],
            path_total: 2,
            base_speed: Some(value).into(),
            fuel: Some(value).into(),
            ..Default::default()
        };
        let mut member = LandCombatDivisionRow {
            combat: 17,
            division: 7,
            stat_values: std::array::from_fn(|at| {
                (if at == 2 {
                    value
                } else {
                    LandCombatDivisionRow::STAT_STEP.round(value)
                })
                .into()
            }),
            armour: value,
            piercing: value,
            reconnaissance: value,
            initiative: value,
            strength_combat_factor: value,
            air_attack: value,
            fuel_inputs: [value.into(), 99_999.into()],
            ..Default::default()
        };
        let mut division = DivisionRow {
            id: 7,
            province: hour as u32,
            organisation: DivisionRow::ORGANISATION_STEP.round(value),
            ..Default::default()
        };
        let mut line = ProductionLineRow {
            handle: 71,
            country: 7,
            active_factories: 2,
            max_factories: 2,
            efficiency_raw: value,
            ..Default::default()
        };
        let damage = i64::MAX.wrapping_add(i64::from(hour));
        let mut observers = vec![
            CombatLogObserverRow {
                combat: 17,
                ordinal: 1,
                kind: CombatLogObserverKind::State as u8,
                a: damage.wrapping_add(value),
                ..Default::default()
            },
            CombatLogObserverRow {
                combat: 17,
                ordinal: 2,
                kind: CombatLogObserverKind::Damage as u8,
                a: damage,
                ..Default::default()
            },
            CombatLogObserverRow {
                combat: 17,
                ordinal: 3,
                kind: CombatLogObserverKind::Damage as u8,
                a: value,
                ..Default::default()
            },
        ];
        let mut naval = NavalCombatDetailsRow {
            combat: 17,
            custom_sea_error: 1,
            ..Default::default()
        };
        naval.sides[0].damage_history[0][0] = damage;
        naval.sides[0].total_damage = damage;
        naval.sides[1].damage_history[0][0] = value;
        naval.sides[1].total_damage = value;
        let mut day = FuelAwardDay {
            own_gain_raw: damage.wrapping_add(value),
            gains_raw: [damage, value, 0, 0],
            received_raw: [0, damage, 0],
            recipients: vec![FuelAwardDayRecipient {
                kind: 1,
                amount_raw: damage,
                ..Default::default()
            }],
            ..Default::default()
        };
        // Switch marks without changing any source contract. Literals, absent
        // optional operands, incomplete sources and row deletion all recur.
        if hour % 8 == 3 {
            movement.path_total = 17;
            movement.base_speed = Some(value + 1).into();
            status.maximum_hit_points += 10_000;
            member.armour += 1;
            division.province += 1;
            line.efficiency_raw += 1;
            observers[0].a = observers[0].a.wrapping_add(1);
            naval.sides[0].total_damage = damage.wrapping_add(1);
            day.own_gain_raw = day.own_gain_raw.wrapping_add(1);
        } else if hour % 8 == 4 {
            movement.base_speed = None.into();
            status.supply_ratio_exact = None.into();
            status.organisation_exact = None.into();
        }
        for rows in [
            TableRows::DivisionPanelStats(vec![panel]),
            TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                movement_defines: MovementDefines {
                    supply_grace: Some(143),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }]),
            TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
                province: hour as u32,
                mixed: vec![7],
                ..Default::default()
            }]),
            TableRows::ProductionLineModifiers(vec![ProductionLineModifiersRow {
                line: 71,
                country: 7,
                active_factories: 2,
                factory_efficiencies: vec![value - 1, value + 1, i64::MAX],
                ..Default::default()
            }]),
            TableRows::DivisionStatus(vec![status]),
            TableRows::DivisionMovement(vec![movement]),
            TableRows::LandCombatDivisions(vec![member]),
            TableRows::LandCombatDivisionModifiers(vec![LandCombatDivisionModifierRow {
                combat: 17,
                division: 7,
                ..Default::default()
            }]),
            TableRows::Divisions(vec![division]),
            TableRows::ProductionLines(vec![line]),
            TableRows::CombatLogObservers(observers),
            TableRows::NavalCombatDetails(vec![naval]),
            TableRows::CountryFuel(vec![CountryFuelRow {
                country: 7,
                award_day: day.into(),
                ..Default::default()
            }]),
        ] {
            let table = rows.table();
            tables[table.index()] = TableView::Complete(rows);
        }
        if hour % 8 == 5 {
            tables[TableId::DivisionPanelStats.index()] = TableView::Unavailable {
                reason: "independent source refusal".into(),
            };
        } else if hour % 8 == 6 {
            let TableView::Complete(rows) = &tables[TableId::DivisionPanelStats.index()] else {
                unreachable!()
            };
            tables[TableId::DivisionPanelStats.index()] = TableView::Truncated {
                rows: rows.clone(),
                missing: 1,
            };
        } else if hour % 8 == 7 {
            for &table in super::super::aliases::private::TABLES {
                tables[table.index()] = TableView::Complete(TableRows::empty(table));
            }
        }
        expected.push(
            encoder
                .encode(
                    crate::frame::retire_absent(DecodedFrame {
                        branch: 1,
                        hour,
                        tables: std::array::from_fn(|at| tables[at].clone().into()),
                        metrics: Metrics::default(),
                    }),
                    false,
                )
                .unwrap(),
        );
        Metrics::default()
    });
    WRITTEN_LAYOUT.set(old_layout);
    FORCE_RESIDUAL.set(old_residual);
    (replay, expected)
}

#[test]
fn direct_delta_alias_rows_resolve_every_table_in_all_layouts_and_prefixes() {
    let targets = super::super::aliases::private::TABLES;
    // Deliberately fixed: a new alias table must extend this oracle fixture.
    assert_eq!(targets.len(), 9);
    for layout in LAYOUTS {
        let (replay, expected) = alias_fixture(layout);
        assert!(replay.index.len() >= 2);
        for index in &replay.index {
            let bytes = replay.block_bytes(index).unwrap();
            let directory = Directory::parse(&bytes, index).unwrap();
            for through in [1, 3, 7, directory.frames] {
                compare(&replay, &expected, index, &bytes, through);
            }
        }
        // The native oracle changes even when all aliased scalars remain
        // storage placeholders; selected reads must load their dependencies.
        let index = &replay.index[0];
        let mut decoder = crate::frame::FrameDecoder::for_tables(targets);
        let mut oracle = crate::frame::FrameDecoder::new();
        for (frame, native) in replay
            .read_block_selected(index, targets)
            .unwrap()
            .iter()
            .zip(&expected)
        {
            assert_eq!(frame.hour, native.hour);
            let actual = decoder.advance(frame).unwrap();
            let oracle = oracle.advance(native).unwrap();
            for &table in targets {
                assert_eq!(
                    actual.tables[table.index()],
                    oracle.tables[table.index()],
                    "{layout:?}, {table:?}, hour {}",
                    frame.hour
                );
            }
        }
    }
}

#[test]
fn direct_delta_alias_keyframe_memos_and_cached_prefixes_hold_only_resolved_updates() {
    let targets = super::super::aliases::private::TABLES;
    for layout in LAYOUTS {
        let (replay, expected) = alias_fixture(layout);
        let mut anchors = 0;
        for cache in [false, true] {
            replay.cache_decoded_sections(cache);
            for index in replay.index.iter().rev() {
                let bytes = replay.block_bytes(index).unwrap();
                let directory = Directory::parse(&bytes, index).unwrap();
                let from = From {
                    position: replay.block_position(index),
                    index,
                    layout,
                };
                let memo: Vec<_> = TableId::ALL.iter().map(|_| OnceLock::new()).collect();
                let first = expected
                    .iter()
                    .find(|frame| frame.hour == index.first_hour)
                    .unwrap();
                // These joins share sources; no placeholder may enter either
                // the per-read memo or the cross-reader section cache.
                for &table in targets.iter().rev() {
                    let raw = directory
                        .raw(&bytes, keyframe_section(table))
                        .unwrap()
                        .unwrap();
                    let stored = keyframe_update(
                        &replay,
                        &from,
                        table,
                        &raw,
                        &Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES)),
                    )
                    .unwrap();
                    anchors += usize::from(stored.anchored);
                    if index.first_hour == 10 {
                        assert!(stored.marks.is_some(), "{table:?}");
                        assert_ne!(
                            stored.update,
                            first.tables[table.index()],
                            "the fixture must actually alias {table:?}"
                        );
                    }
                    let admitted =
                        keyframe_admitted(&replay, &from, &directory, table, &memo).unwrap();
                    assert_eq!(
                        **admitted.get(0),
                        first.tables[table.index()],
                        "{layout:?}, {table:?}"
                    );
                    let again =
                        keyframe_admitted(&replay, &from, &directory, table, &memo).unwrap();
                    assert!(Arc::ptr_eq(admitted.get(0), again.get(0)));
                    assert_eq!(
                        **memo[table.index()].get().unwrap().get(0),
                        first.tables[table.index()]
                    );
                }
                // Extending a cached keyframe to hot deltas, then shortening a
                // cached section, must both agree with independent plain rows.
                compare(&replay, &expected, index, &bytes, directory.frames);
                compare(&replay, &expected, index, &bytes, 3);
            }
        }
        if layout.anchors_back() {
            assert!(anchors != 0, "{layout:?} must exercise raw anchored rows");
        }
    }
}

/// A checked allocation failure on the direct route is not a refusal: it propagates instead of being
/// retried through the original reader with a fresh budget, which would turn it into success.
#[test]
fn direct_delta_allocation_failures_propagate_instead_of_retrying() {
    const CHILD: &str = "BATTLE_PLAN_GROUPED_DIRECT_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "container::grouped::direct_tests::direct_delta_allocation_failures_propagate_instead_of_retrying",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    thread_local! {
        static ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static FAIL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    }
    fn fail_allocation() -> bool {
        let attempt = ATTEMPTS.get() + 1;
        ATTEMPTS.set(attempt);
        FAIL_AT.get() == Some(attempt)
    }
    let (replay, _) = fixture(Layout::Chosen);
    let mut checked = 0;
    for index in &replay.index {
        let bytes = replay.block_bytes(index).unwrap();
        let directory = Directory::parse(&bytes, index).unwrap();
        let present = Present::new(replay.format.version(), replay.capture_mode);
        let header = directory.raw(&bytes, HEADER).unwrap().unwrap();
        let (hours, entries) = read_header(&header, index, directory.frames, &present).unwrap();
        let memo: Vec<_> = TableId::ALL.iter().map(|_| OnceLock::new()).collect();
        let context = Context {
            replay: &replay,
            index,
            position: replay.block_position(index),
            block: &bytes,
            directory: &directory,
            format: replay.format,
            present: &present,
            hours: &hours,
            entries: &entries,
            selected: None,
            complete: true,
            memo: &memo,
            group: &OnceLock::new(),
        };
        for &table in &present.tables {
            if keyframe_refusal(&directory, replay.format, replay.capture_mode, None, table)
                .unwrap()
                .is_some()
            {
                continue;
            }
            // Warm the alias sources' shared views and memo before counting checked growth.
            direct::table_updates(&context, table).unwrap();
            ATTEMPTS.set(0);
            FAIL_AT.set(None);
            engine_allocation::set_stop_hook(Some(fail_allocation));
            let healthy = direct::table_updates(&context, table);
            engine_allocation::set_stop_hook(None);
            healthy.unwrap();
            let attempts = ATTEMPTS.get();
            for ordinal in 1..=attempts {
                ATTEMPTS.set(0);
                FAIL_AT.set(Some(ordinal));
                engine_allocation::set_stop_hook(Some(fail_allocation));
                let result = direct::table_updates(&context, table);
                engine_allocation::set_stop_hook(None);
                assert_eq!(
                    result.unwrap_err().kind(),
                    io::ErrorKind::OutOfMemory,
                    "{table:?} checked allocation {ordinal}"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked > 0,
        "the fixture must reach checked allocations on the direct route"
    );
}

/// Deny span growth on plain and residual routes; neither may retry allocation failures.
#[test]
fn span_buffer_growth_failures_propagate_on_both_routes() {
    const CHILD: &str = "BATTLE_PLAN_GROUPED_SPAN_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "container::grouped::direct_tests::span_buffer_growth_failures_propagate_on_both_routes",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    thread_local! {
        static ROUTES: std::cell::RefCell<Vec<bool>> = const { std::cell::RefCell::new(Vec::new()) };
        static FAIL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    }
    // Counts only span buffer growth, noting whether its row is residual.
    fn fail_allocation() -> bool {
        let Some(residual) = direct::SPAN_ROUTE.get() else {
            return false;
        };
        let attempt = ROUTES.with_borrow_mut(|routes| {
            routes.push(residual);
            routes.len()
        });
        FAIL_AT.get() == Some(attempt)
    }
    let mut reached = [false; 2];
    let mut checked = 0;
    for layout in [Layout::Unpredicted, Layout::Chosen] {
        let (replay, _) = fixture(layout);
        for index in &replay.index {
            let bytes = replay.block_bytes(index).unwrap();
            let directory = Directory::parse(&bytes, index).unwrap();
            let present = Present::new(replay.format.version(), replay.capture_mode);
            let header = directory.raw(&bytes, HEADER).unwrap().unwrap();
            let (hours, entries) = read_header(&header, index, directory.frames, &present).unwrap();
            let memo: Vec<_> = TableId::ALL.iter().map(|_| OnceLock::new()).collect();
            let context = Context {
                replay: &replay,
                index,
                position: replay.block_position(index),
                block: &bytes,
                directory: &directory,
                format: replay.format,
                present: &present,
                hours: &hours,
                entries: &entries,
                selected: None,
                complete: true,
                memo: &memo,
                group: &OnceLock::new(),
            };
            let read = |table: TableId, fail_at: Option<usize>| {
                ROUTES.with_borrow_mut(Vec::clear);
                FAIL_AT.set(fail_at);
                engine_allocation::set_stop_hook(Some(fail_allocation));
                let result = direct::table_updates(&context, table);
                engine_allocation::set_stop_hook(None);
                (result, ROUTES.with_borrow_mut(std::mem::take))
            };
            for &table in &present.tables {
                if keyframe_refusal(&directory, replay.format, replay.capture_mode, None, table)
                    .unwrap()
                    .is_some()
                {
                    continue;
                }
                // Warm the alias sources' memo, so every read repeats the same growths.
                direct::table_updates(&context, table).unwrap();
                let (healthy, routes) = read(table, None);
                healthy.unwrap();
                for (ordinal, &residual) in (1..).zip(&routes) {
                    reached[usize::from(residual)] = true;
                    let (result, _) = read(table, Some(ordinal));
                    assert_eq!(
                        result.unwrap_err().kind(),
                        io::ErrorKind::OutOfMemory,
                        "{layout:?} {table:?} span growth {ordinal}, residual {residual}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 0);
    assert_eq!(
        reached, [true; 2],
        "unpredicted and residual rows must both grow a span buffer"
    );
}

/// Deny every candidate step, both alone and grouped, with sources loaded afresh.
#[test]
fn candidate_hour_allocation_failures_propagate_instead_of_retrying() {
    use super::super::aliases::private::{Resolution, Step, TABLES, hooks};
    const CHILD: &str = "BATTLE_PLAN_GROUPED_CANDIDATE_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "container::grouped::direct_tests::candidate_hour_allocation_failures_propagate_instead_of_retrying",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    thread_local! {
        static STEPS: std::cell::RefCell<Vec<Step>> = const { std::cell::RefCell::new(Vec::new()) };
        static FAIL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    }
    // Counts only checked growth in candidate-hour work, noting its step.
    fn fail_allocation() -> bool {
        let Some(step) = hooks::step() else {
            return false;
        };
        let attempt = STEPS.with_borrow_mut(|steps| {
            steps.push(step);
            steps.len()
        });
        FAIL_AT.get() == Some(attempt)
    }
    hooks::with(|hooks| hooks.resolution = Some(Resolution::Candidates));
    let hours: Vec<_> = (15..31).chain(735..751).collect();
    let (replay, _) = alias_fixture_at(Layout::Chosen, &hours);
    replay.cache_decoded_sections(false);
    let index = &replay.index[0];
    assert_eq!(
        index.first_hour % 8,
        7,
        "the block starts with empty alias tables"
    );
    let bytes = replay.block_bytes(index).unwrap();
    let directory = Directory::parse(&bytes, index).unwrap();
    let present = Present::new(replay.format.version(), replay.capture_mode);
    let header = directory.raw(&bytes, HEADER).unwrap().unwrap();
    let (block_hours, entries) = read_header(&header, index, directory.frames, &present).unwrap();
    let read = |table: TableId, alone: bool, fail_at: Option<usize>| {
        hooks::with(|hooks| hooks.group_old = Some(alone));
        let selected = [table];
        let memo: Vec<_> = TableId::ALL.iter().map(|_| OnceLock::new()).collect();
        let context = Context {
            replay: &replay,
            index,
            position: replay.block_position(index),
            block: &bytes,
            directory: &directory,
            format: replay.format,
            present: &present,
            hours: &block_hours,
            entries: &entries,
            selected: (!alone).then_some(&selected[..]),
            complete: true,
            memo: &memo,
            group: &OnceLock::new(),
        };
        STEPS.with_borrow_mut(Vec::clear);
        FAIL_AT.set(fail_at);
        engine_allocation::set_stop_hook(Some(fail_allocation));
        let result = if alone {
            direct::table_updates(&context, table).map(|updates| updates.len())
        } else {
            table_updates_admitted(&context, table).map(|updates| updates.len())
        };
        engine_allocation::set_stop_hook(None);
        (result, STEPS.with_borrow_mut(std::mem::take))
    };
    for alone in [true, false] {
        let mut reached = std::collections::BTreeSet::new();
        let mut checked = 0;
        for &table in TABLES {
            let (healthy, steps) = read(table, alone, None);
            healthy.unwrap();
            reached.extend(steps.iter().copied());
            for (ordinal, step) in (1..).zip(&steps) {
                let (result, _) = read(table, alone, Some(ordinal));
                assert_eq!(
                    result.unwrap_err().kind(),
                    io::ErrorKind::OutOfMemory,
                    "{table:?} checked allocation {ordinal}, {step:?}, alone {alone}"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "alone {alone}");
        // Resolving copied candidate rows makes no checked allocation to deny.
        for step in [
            Step::Marks,
            Step::Keys,
            Step::CatchUp,
            Step::Load,
            Step::Rows,
            Step::Encode,
            Step::Merge,
        ] {
            assert!(
                reached.contains(&step),
                "no checked allocation denied in {step:?}, alone {alone}"
            );
        }
    }
}

/// Compare scheduled reads with serial reads, warm caches and refused workers included.
#[test]
fn scheduled_parallel_reads_return_the_serial_read() {
    let parallel = Parallel {
        workers: 8,
        on_worker: &|| {},
    };
    for (replay, _) in [fixture(Layout::Chosen), alias_fixture(Layout::Chosen)] {
        let serial: Vec<_> = replay
            .index
            .iter()
            .map(|index| replay.read_block(index).unwrap())
            .collect();
        for cache in [false, true] {
            replay.cache_decoded_sections(cache);
            for (index, serial) in replay.index.iter().zip(&serial) {
                assert_eq!(
                    &replay.read_block_parallel(index, &parallel).unwrap(),
                    serial
                );
            }
        }
        replay.cache_decoded_sections(false);
        for (index, serial) in replay.index.iter().zip(&serial) {
            super::super::REFUSED_WORKERS.set(parallel.workers);
            assert_eq!(
                &replay.read_block_parallel(index, &parallel).unwrap(),
                serial
            );
            assert_eq!(super::super::REFUSED_WORKERS.get(), 0);
        }
    }
}

/// Independently encoded frames and fixed malformed-column errors cover the row fast path.
#[test]
fn direct_rows_match_every_update_byte_and_fixed_refusal() {
    direct_delta_all_admitted_grouped_layouts_and_prefixes_match_every_update_and_byte();
    direct_delta_alias_rows_resolve_every_table_in_all_layouts_and_prefixes();
    direct_delta_section_refusals_keep_original_framing_and_wire_precedence();
    direct_delta_malformed_columns_have_fixed_refusals_in_every_layout();
}

/// Compare span shortcuts with a full field parse, including refusals and patch bases.
#[test]
fn direct_row_spans_refuse_what_reading_every_span_refuses() {
    let format = FormatVersion::CURRENT.version();
    let check = |table: TableId, mask: u64, bytes: Vec<u8>, spans: &[direct::Span], case: &str| {
        let (mut all_shadow, mut span_shadow) = (Shadow::new(), Shadow::new());
        let mut all = direct::Delta::new(table, table.field_count(), format, 9, &mut all_shadow);
        let mut parts = direct::Delta::new(table, table.field_count(), format, 9, &mut span_shadow);
        let read_all = all
            .row(0, mask, bytes.clone())
            .map_err(|error| (error.kind(), error.to_string()));
        let read_spans = parts
            .row_spans(0, mask, bytes, spans)
            .map_err(|error| (error.kind(), error.to_string()));
        assert_eq!(read_all, read_spans, "{table:?}, {case}");
        assert_eq!(all.finish(), parts.finish(), "{table:?}, {case}");
        assert_eq!(all_shadow, span_shadow, "{table:?}, {case}");
    };
    // Every cut of `values`, and one trailing byte past it.
    let cuts = |values: &[u8]| -> Vec<Vec<u8>> {
        let mut cuts: Vec<Vec<u8>> = (0..=values.len())
            .map(|length| values[..length].to_vec())
            .collect();
        let mut longer = values.to_vec();
        longer.push(0);
        cuts.push(longer);
        cuts
    };
    for table in TableId::ALL {
        if !table.last_change().present(format)
            || table.last_change().table_gap(table, format).is_some()
        {
            continue;
        }
        let schema = table.schema();
        for (field, &kind) in schema.iter().enumerate() {
            let valid = change(kind, 1);
            check(
                table,
                1 << field,
                valid.clone(),
                &[direct::Span {
                    field,
                    end: valid.len(),
                    parsed: true,
                }],
                &format!("parsed field {field}"),
            );
            for bytes in cuts(&valid) {
                let span = direct::Span {
                    field,
                    end: bytes.len(),
                    parsed: false,
                };
                check(
                    table,
                    1 << field,
                    bytes.clone(),
                    &[span],
                    &format!("field {field}, {} bytes", bytes.len()),
                );
            }
            // An earlier inexact span can displace the cursor before a parsed field.
            let Some(&next_kind) = schema.get(field + 1) else {
                continue;
            };
            let next = change(next_kind, 2);
            for first in cuts(&change(kind, 1)) {
                let end = first.len();
                let bytes = [first, next.clone()].concat();
                let spans = [
                    direct::Span {
                        field,
                        end,
                        parsed: false,
                    },
                    direct::Span {
                        field: field + 1,
                        end: bytes.len(),
                        parsed: true,
                    },
                ];
                check(
                    table,
                    0b11 << field,
                    bytes,
                    &spans,
                    &format!("fields {field}+1, first {end} bytes"),
                );
            }
        }
    }
}

/// Every direct delta owns exactly its value bytes, including residual rows.
#[test]
fn direct_rows_copy_each_row_buffer_exactly() {
    let mut checked = 0;
    for layout in LAYOUTS {
        let (replay, expected) = fixture(layout);
        for index in &replay.index {
            let bytes = replay.block_bytes(index).unwrap();
            let frames = read(&replay, index, &bytes, None, None).unwrap();
            let expected: Vec<_> = expected
                .iter()
                .filter(|frame| index.first_hour <= frame.hour && frame.hour <= index.last_hour)
                .collect();
            assert_eq!(frames.len(), expected.len(), "{layout:?}");
            for (frame, expected) in frames.iter().zip(expected) {
                assert_eq!(frame, expected, "{layout:?}, hour {}", frame.hour);
                for update in &frame.tables {
                    if let TableUpdate::Delta { changed, .. } = update {
                        for row in changed {
                            assert_eq!(
                                row.values.capacity(),
                                row.values.len(),
                                "{layout:?}, hour {}",
                                frame.hour
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 0, "the fixture must decode delta rows");
}

/// Candidate verification retains whole-table resolution as its independent reference.
#[test]
fn direct_alias_oracles_pass_with_candidate_rows_verified_and_with_whole_tables() {
    use super::super::aliases::private::{Resolution, hooks};
    for mode in [Resolution::Verify, Resolution::Whole] {
        hooks::with(|hooks| {
            hooks.resolution = Some(mode);
            hooks.candidate_hours = 0;
        });
        direct_delta_alias_rows_resolve_every_table_in_all_layouts_and_prefixes();
        direct_delta_alias_keyframe_memos_and_cached_prefixes_hold_only_resolved_updates();
        let candidate_hours = hooks::with(|hooks| {
            hooks.resolution = None;
            hooks.candidate_hours
        });
        if mode == Resolution::Whole {
            assert_eq!(candidate_hours, 0);
        } else {
            assert!(
                candidate_hours > 0,
                "the oracles must reach candidate hours"
            );
        }
    }
}
