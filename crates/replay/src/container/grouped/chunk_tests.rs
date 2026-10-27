use super::*;
use crate::{
    frame::FrameDecoder,
    rows::{CountryRow, DivisionMovementRow, DivisionRow, LexicalTokenRow},
};

fn patch_literal(token: u32, hour: Hour) -> Vec<u8> {
    let mut state = u64::from(token) + 0x9e37_79b9;
    (0..512)
        .map(|at| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let byte = state as u8;
            if at < 96 {
                byte.wrapping_add(hour as u8)
            } else {
                byte
            }
        })
        .collect()
}

pub(super) fn fixture() -> (Replay, Vec<DecodedFrame>) {
    let hours: Vec<_> = (10..=220)
        .chain([600, 650, 729, 730, 731, 1070, 1400, 1449])
        .collect();
    let mut expected = Vec::new();
    let old = FORCE_RESIDUAL.replace(true);
    let replay = test_recording::build(b"synthetic-chunked-columns", &hours, |hour, tables| {
        let owners: Vec<_> = (1..=48_u32)
            .filter(|id| hour % 37 != 0 || id % 5 != 0)
            .collect();
        tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            owners
                .iter()
                .map(|&id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("synthetic-long-country-tag-{id}-{}", hour / 17).into_bytes(),
                    command_power_raw: i64::from(hour) * 100_000,
                    ..Default::default()
                })
                .collect(),
        ));
        tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
            owners
                .iter()
                .map(|&id| DivisionRow {
                    id: u64::from(id),
                    province: hour as u32,
                    organisation: i64::from(hour) * 10_000,
                    ..Default::default()
                })
                .collect(),
        ));
        tables[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(
                owners
                    .iter()
                    .map(|&id| {
                        let path: Vec<_> = (hour as u32 % 15..20).collect();
                        DivisionMovementRow {
                            division: u64::from(id),
                            source: hour as u32,
                            path_total: path.len() as u32 + u32::from(hour % 11 == 0),
                            path,
                            numerator: i64::from(hour) * 100_000,
                            ..Default::default()
                        }
                    })
                    .collect(),
            ));
        tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
            (1..=16)
                .map(|token| LexicalTokenRow {
                    token,
                    literal: patch_literal(token, hour),
                })
                .collect(),
        ));
        expected.push(crate::frame::retire_absent(DecodedFrame {
            branch: 1,
            hour,
            tables: std::array::from_fn(|at| tables[at].clone().into()),
            metrics: Metrics::default(),
        }));
        Metrics::default()
    });
    FORCE_RESIDUAL.set(old);
    (replay, expected)
}

#[test]
fn chunked_sparse_multiblock_rows_equal_every_recorded_input_hour() {
    let (replay, expected) = fixture();
    assert!(replay.index.len() >= 2);
    let mut decoder = FrameDecoder::new();
    let mut at = 0;
    for index in &replay.index {
        for frame in replay.read_block(index).unwrap() {
            let actual = decoder.advance(&frame).unwrap();
            assert_eq!(actual.hour, expected[at].hour);
            assert_eq!(actual.tables, expected[at].tables, "hour {}", actual.hour);
            at += 1;
        }
    }
    assert_eq!(at, expected.len());
}

#[test]
fn on_demand_chunks_join_to_the_exact_original_columns() {
    let (replay, _) = fixture();
    let mut saw_empty = false;
    let mut saw_many = false;
    let mut buffer = Vec::new();
    for index in &replay.index {
        let block = replay.block_bytes(index).unwrap();
        let directory = Directory::parse(&block, index).unwrap();
        for table in [
            TableId::Countries,
            TableId::Divisions,
            TableId::DivisionMovement,
            TableId::LexicalTokens,
            TableId::Clock,
        ] {
            let section = Section::Later(table);
            let mut joined: Vec<Vec<u8>> = Vec::new();
            let chunks = replay.section_chunks(index, section).unwrap();
            saw_many |= chunks.len() > 1;
            for (slot, chunk) in chunks.enumerate() {
                assert_eq!(
                    i64::from(chunk.first_hour),
                    i64::from(index.first_hour) + 1 + (slot * LATER_CHUNK_HOURS) as i64
                );
                assert!(chunk.last_hour <= index.last_hour);
                assert!(
                    i64::from(chunk.last_hour) - i64::from(chunk.first_hour)
                        < LATER_CHUNK_HOURS as i64
                );
                chunk.inflate_into(&mut buffer).unwrap();
                assert_eq!(buffer.len() as u64, chunk.raw_length);
                if chunk.stored_length == 0 {
                    saw_empty = true;
                    assert!(buffer.is_empty());
                    continue;
                }
                let columns = chunked::columns(&buffer).unwrap();
                if joined.is_empty() {
                    joined.resize_with(columns.len(), Vec::new);
                }
                for (output, column) in joined.iter_mut().zip(columns) {
                    output.extend_from_slice(column);
                }
            }
            if let Some(raw) = directory.raw(&block, later_section(table)).unwrap() {
                assert_eq!(chunked::frame(&joined).unwrap(), raw);
            } else {
                assert!(joined.is_empty());
            }
        }
    }
    assert!(saw_empty && saw_many);
}

#[test]
fn chosen_column_slices_join_to_the_prechunk_writer_bytes() {
    let table = TableId::LexicalTokens;
    let mut later = Later::new(table, table.field_count(), Layout::Chosen);
    let value = |hour| {
        let mut bytes = 512_u32.to_le_bytes().to_vec();
        bytes.extend(patch_literal(1, hour));
        bytes
    };
    let mut base = value(0);
    for hour in 1..=400 {
        later.columns.0[CONTROL].push(UNCHANGED);
        let next = value(hour);
        put_byte_field(
            Some(&next),
            Some(&base),
            None,
            &mut later.columns,
            FIELDS,
            None,
        )
        .unwrap();
        base = next;
        later.entries += 1;
        later.checkpoint((hour as usize - 1) / LATER_CHUNK_HOURS);
    }
    let differences = later.columns.1[FIELDS].clone().unwrap();
    assert_ne!(later.columns.0[FIELDS], differences);
    let alternate_ends: Vec<_> = later.alternate_cuts.iter().map(|cut| cut[FIELDS]).collect();
    later.compress(Compression::Low).unwrap();
    assert_eq!(later.columns.0[FIELDS], differences);
    assert_eq!(
        later.cuts.iter().map(|cut| cut[FIELDS]).collect::<Vec<_>>(),
        alternate_ends
    );
    let before = later.columns.raw();
    let chunks = chunked::split(later_section(table), &later).unwrap();
    let joined = chunked::join(
        chunks
            .iter()
            .map(|(_, raw, stored)| Directory::inflate(stored, *raw)),
    )
    .unwrap();
    assert_eq!(joined, before);
}

#[test]
fn corrupt_chunk_index_and_checksums_are_named_errors() {
    let (replay, _) = fixture();
    let index = &replay.index[0];
    let block = replay.block_bytes(index).unwrap();
    let directory = Directory::parse(&block, index).unwrap();
    let end = directory
        .sections
        .iter()
        .flatten()
        .map(|entry| entry.0)
        .min()
        .unwrap();
    let mut damaged = block.to_vec();
    damaged[4] ^= 1;
    assert!(
        Directory::parse(&damaged, index)
            .err()
            .unwrap()
            .to_string()
            .contains("directory CRC")
    );
    let mut input = Decoder::new(&block[8..end]);
    input.varint().unwrap();
    let count = varint_usize(&mut input).unwrap();
    for _ in 0..count {
        let id = varint_usize(&mut input).unwrap();
        input.varint().unwrap();
        input.varint().unwrap();
        if id >= 3 && id % 2 == 1 {
            let at = end - input.remaining();
            damaged = block.to_vec();
            damaged[at] ^= 1;
            let crc = crc32fast::hash(&damaged[8..end]);
            damaged[4..8].copy_from_slice(&crc.to_le_bytes());
            assert!(
                Directory::parse(&damaged, index)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("chunk index")
            );
            break;
        }
    }
    let &(offset, stored, raw) = directory.chunks[later_section(TableId::Clock)]
        .iter()
        .find(|entry| entry.1 != 0)
        .unwrap();
    let mut frame = block[offset..offset + stored].to_vec();
    *frame.last_mut().unwrap() ^= 1;
    let error = Directory::inflate(&frame, raw).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error.to_string().to_lowercase().contains("checksum"),
        "{error}"
    );
}

#[test]
fn an_older_anchored_keyframe_is_not_a_candidate() {
    let replay = test_recording::build(
        b"synthetic-anchor-candidates",
        &[1, 721, 1441, 2161],
        |_, tables| {
            tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=256)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("synthetic-country-{id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ));
            Metrics::default()
        },
    );
    let table = TableId::Countries;
    let fields = table.field_count();
    assert_eq!(
        anchor_candidates(&replay, 3, table, fields, replay.format.version()).unwrap(),
        (Some(0), Some(2))
    );
    let from = From {
        position: Some(3),
        index: &replay.index[3],
        layout: Layout::Chosen,
    };
    let error = anchored_keyframe(
        &replay,
        &from,
        table,
        fields,
        false,
        AnchorRef::Back(2),
        0,
        replay.format,
        false,
    )
    .err()
    .unwrap();
    assert!(
        error
            .to_string()
            .contains("not the latest snapshot or latest keyframe")
    );
}

#[test]
fn real_table_row_coders_end_each_chunk_at_the_decoder_column_cursors() {
    let (replay, _) = fixture();
    let frames = replay.read_block(&replay.index[0]).unwrap();
    let version = replay.format.version();
    for modelled in [false, true] {
        let mut block = BlockWriter::new(&frames[0]).unwrap();
        for writer in &mut block.tables {
            let later = &mut writer.plain;
            later.columns.1[KEYS] = None;
            later.columns.1[MASKS] = None;
            if !modelled {
                later.model = None;
                later.columns.0[MASKS][0] = LISTED_ROWS;
            }
        }
        let mut anchors = Anchors::default();
        let cores = Cores::new(1);
        let mut decoder = FrameDecoder::new();
        let mut payloads = Vec::new();
        for frame in &frames {
            let mut payload = Vec::new();
            let sizes = frame.encode_with_sizes(&mut payload).unwrap();
            let decoded = decoder.advance(frame).unwrap();
            block
                .push(
                    frame,
                    decoded,
                    &payload,
                    &sizes,
                    &mut anchors,
                    MAX_BLOCK_RAW_BYTES,
                    &cores,
                )
                .unwrap();
            payloads.push((payload, sizes));
        }
        let mut saw_differences = false;
        for writer in block.tables.into_iter().filter(|writer| {
            [
                TableId::Countries,
                TableId::Divisions,
                TableId::DivisionMovement,
                TableId::LexicalTokens,
            ]
            .contains(&writer.table)
        }) {
            let table = writer.table;
            let fields = writer.fields;
            let plains: Vec<_> = payloads
                .iter()
                .map(|(payload, sizes)| {
                    let start = payload.len() - sizes[table.index()..].iter().sum::<usize>();
                    payload[start..start + sizes[table.index()]].to_vec()
                })
                .collect();
            let mut later = writer.plain;
            let variants: Vec<_> = later
                .columns
                .1
                .iter()
                .enumerate()
                .skip(FIELDS)
                .filter_map(|(column, alternate)| {
                    alternate
                        .as_ref()
                        .filter(|alternate| **alternate != later.columns.0[column])
                        .map(|alternate| (column, alternate.clone()))
                })
                .collect();
            later.compress(Compression::Low).unwrap();
            let chosen: Vec<_> = variants
                .iter()
                .filter(|(column, alternate)| later.columns.0[*column] == *alternate)
                .map(|(column, _)| *column)
                .collect();
            if table == TableId::LexicalTokens {
                assert!(
                    !chosen.is_empty(),
                    "the fixture must choose a differences field column"
                );
            }
            saw_differences |= !chosen.is_empty();
            let original = later.columns.raw();
            let chunks = chunked::split(later_section(table), &later).unwrap();
            let mut ends = Vec::new();
            let mut total = vec![0; later.columns.0.len()];
            for (_, raw, stored) in &chunks {
                if *raw != 0 {
                    let raw = Directory::inflate(stored, *raw).unwrap();
                    for (total, column) in total.iter_mut().zip(chunked::columns(&raw).unwrap()) {
                        *total += column.len();
                    }
                }
                ends.push(total.clone());
            }
            let mut input = ColumnReader::stored(&original, fields, table, version).unwrap();
            assert_eq!(
                input.modelled_rows(table, Layout::Chosen).unwrap(),
                modelled
            );
            let mut model = modelled.then(|| Model::new(fields));
            let mut masks = MaskList::default();
            let mut keys = KeySet::default();
            keys.advance(table, version, &plains[0]).unwrap();
            let mut shadow = Shadow::new();
            advance_shadow(table, version, &plains[0], &mut shadow).unwrap();
            if let crate::frame::TableUpdate::Snapshot(rows) = &frames[0].tables[table.index()] {
                settle_advance(table, Layout::Chosen, &mut shadow, Some(rows));
            }
            let budget = Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES));
            let mut changes = KeyChanges::default();
            for at in 1..frames.len() {
                if later.present[at] {
                    let mut plain = Vec::new();
                    let head = take_head(
                        &mut input,
                        Some(frames[at - 1].hour),
                        frames[0].hour,
                        &mut plain,
                        Layout::Chosen,
                    )
                    .unwrap();
                    take_body(
                        table,
                        fields,
                        head.status,
                        &mut input,
                        &shadow,
                        &mut plain,
                        &budget,
                        Layout::Chosen,
                        Some(RowCoding {
                            keys: &keys,
                            masks: &mut masks,
                            model: model.as_mut(),
                            changes: Some(&mut changes),
                        }),
                    )
                    .unwrap();
                    assert_eq!(plain, plains[at]);
                    advance_shadow(table, version, &plain, &mut shadow).unwrap();
                    if head.status == DELTA {
                        keys.apply(&mut changes).unwrap();
                    } else {
                        keys.advance(table, version, &plain).unwrap();
                    }
                }
                let slot = (frames[at].hour - frames[0].hour - 1) as usize / LATER_CHUNK_HOURS;
                let next_slot = frames
                    .get(at + 1)
                    .map(|frame| (frame.hour - frames[0].hour - 1) as usize / LATER_CHUNK_HOURS);
                if next_slot != Some(slot) {
                    for (column, decoder) in input.0.iter().enumerate() {
                        assert_eq!(
                            later.columns.0[column].len() - decoder.remaining(),
                            ends[slot][column],
                            "{table:?}, modelled {modelled}, slot {slot}, column {column}"
                        );
                    }
                }
            }
            input.finish().unwrap();
        }
        assert!(saw_differences, "modelled {modelled}");
    }
}

#[test]
fn candidate_history_is_cached_and_later_known_candidates_survive_older_gaps() {
    let mut replay = test_recording::build(
        b"synthetic-cached-candidates",
        &[1, 721, 1441, 2161],
        |_, tables| {
            tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=256)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("synthetic-cached-country-{id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ));
            Metrics::default()
        },
    );
    let table = TableId::Countries;
    let candidates = || {
        anchor_candidates(
            &replay,
            3,
            table,
            table.field_count(),
            replay.format.version(),
        )
        .unwrap()
    };
    assert_eq!(candidates(), (Some(0), Some(2)));
    assert_eq!(candidates(), (Some(0), Some(2)));
    assert_eq!(
        replay.anchor_candidates.tables[table.index()]
            .lock()
            .unwrap()
            .steps
            .len(),
        3
    );
    replay.ordinals[0] = 0;
    replay.ordinals[1] = 2;
    replay.ordinals[2] = 3;
    replay.ordinals[3] = 4;
    assert_eq!(
        anchor_candidates(
            &replay,
            3,
            table,
            table.field_count(),
            replay.format.version()
        )
        .unwrap(),
        (None, Some(2))
    );
}

#[test]
fn nonempty_chunks_require_column_bytes_and_zstd_checksums() {
    let raw = chunked::frame(&vec![Vec::<u8>::new(); FIELDS]).unwrap();
    assert!(
        chunked::join([Ok(raw)])
            .unwrap_err()
            .to_string()
            .contains("no column bytes")
    );
    let bytes = b"synthetic checksummed chunk";
    let unchecked = zstd::stream::encode_all(&bytes[..], LEVEL).unwrap();
    assert!(
        Directory::inflate(&unchecked, bytes.len())
            .unwrap_err()
            .to_string()
            .contains("missing its zstd checksum")
    );
}

#[test]
fn unknown_intervening_history_refuses_an_unprovable_snapshot_candidate() {
    enum Damage {
        MissingOrdinal,
        BadChecksum,
        MissingKeyframe,
    }
    for damage in [
        Damage::MissingOrdinal,
        Damage::BadChecksum,
        Damage::MissingKeyframe,
    ] {
        let mut replay = test_recording::build(
            b"synthetic-unknown-candidates",
            &[1, 721, 1441, 2161],
            |_, tables| {
                tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                    (1..=256)
                        .map(|id| CountryRow {
                            id,
                            canonical_slot: id,
                            tag: format!("synthetic-unknown-country-{id}").into_bytes(),
                            ..Default::default()
                        })
                        .collect(),
                ));
                Metrics::default()
            },
        );
        let table = TableId::Countries;
        let fields = table.field_count();
        assert_eq!(
            anchor_candidates(&replay, 3, table, fields, replay.format.version()).unwrap(),
            (Some(0), Some(2))
        );
        match damage {
            Damage::MissingOrdinal => {
                replay.index.remove(2);
                replay.ordinals.remove(2);
            }
            Damage::BadChecksum => {
                let index = &replay.index[2];
                let bytes = replay.block_bytes(index).unwrap();
                let directory = Directory::parse(&bytes, index).unwrap();
                let (offset, stored, _) = directory.sections[keyframe_section(table)].unwrap();
                replay.bytes[index.offset as usize + offset + stored - 1] ^= 1;
                replay.anchor_candidates = CandidateCache::default();
            }
            Damage::MissingKeyframe => {
                let original = &replay.index[2];
                let block = replay.block_bytes(original).unwrap();
                let directory = Directory::parse(&block, original).unwrap();
                assert_eq!(directory.frames, 1);
                assert!(
                    keyframe_refusal(&directory, replay.format, replay.capture_mode, None, table)
                        .unwrap()
                        .is_none()
                );
                let entries: Vec<_> = directory
                    .sections
                    .iter()
                    .enumerate()
                    .filter(|(id, _)| *id != keyframe_section(table))
                    .filter_map(|(id, entry)| entry.map(|entry| (id, entry)))
                    .collect();
                let mut header = Vec::new();
                put_varint(&mut header, directory.frames as u128);
                put_varint(&mut header, entries.len() as u128);
                let mut payload = Vec::new();
                let mut raw_length = 0;
                for (id, (offset, stored, raw)) in entries {
                    assert!(directory.chunks[id].is_empty());
                    put_varint(&mut header, id as u128);
                    put_varint(&mut header, raw as u128);
                    put_varint(&mut header, stored as u128);
                    payload.extend_from_slice(&block[offset..offset + stored]);
                    raw_length += raw as u64;
                }
                let mut bytes = CHOSEN_MAGIC.to_vec();
                bytes.extend_from_slice(&crc32fast::hash(&header).to_le_bytes());
                bytes.extend(header);
                bytes.extend(payload);
                let mut index = original.clone();
                index.offset = replay.bytes.len() as u64;
                index.length = bytes.len() as u64;
                index.raw_length = raw_length;
                let directory = Directory::parse(&bytes, &index).unwrap();
                assert!(directory.sections[keyframe_section(table)].is_none());
                replay.bytes.extend(bytes);
                replay.index[2] = index;
                assert_eq!(
                    anchor_candidates(&replay, 3, table, fields, replay.format.version()).unwrap(),
                    (None, None)
                );
                replay.anchor_candidates = CandidateCache::default();
                assert_eq!(
                    anchor_candidates(&replay, 3, table, fields, replay.format.version()).unwrap(),
                    (None, None)
                );
            }
        }
        let position = replay.index.len() - 1;
        let from = From {
            position: Some(position),
            index: &replay.index[position],
            layout: Layout::Chosen,
        };
        let error = anchored_keyframe(
            &replay,
            &from,
            table,
            fields,
            false,
            AnchorRef::Back(3),
            0,
            replay.format,
            false,
        )
        .err()
        .unwrap();
        assert!(
            error
                .to_string()
                .contains("not the latest snapshot or latest keyframe"),
            "{error}"
        );
    }
}

#[test]
fn legitimately_absent_keyframes_preserve_candidate_history() {
    let replay = test_recording::build(
        b"synthetic-legitimate-absence",
        &[1, 721, 1441],
        |_, tables| {
            tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=256)
                    .map(|id| CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("synthetic-kept-country-{id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ));
            Metrics::default()
        },
    );
    let omitted = scope::omitted(replay.capture_mode)
        .find(|table| HISTORY[table.index()].present(replay.format.version()))
        .expect("the capture mode must legitimately leave out a current table");
    for index in &replay.index {
        let directory = Directory::read(&replay, index).unwrap();
        assert!(directory.sections[keyframe_section(omitted)].is_none());
        assert_eq!(
            keyframe_refusal(
                &directory,
                replay.format,
                replay.capture_mode,
                None,
                omitted
            )
            .unwrap(),
            Some(TableUpdate::NotRecorded {
                mode: replay.capture_mode
            })
        );
    }
    assert_eq!(
        anchor_candidates(
            &replay,
            2,
            omitted,
            omitted.field_count(),
            replay.format.version()
        )
        .unwrap(),
        (None, None)
    );
    // Seed prior state to distinguish preserving legitimate absence from clearing candidates.
    {
        let mut history = replay.anchor_candidates.tables[omitted.index()]
            .lock()
            .unwrap();
        history.steps.truncate(1);
        history.steps[0].snapshot = Some(0);
        history.steps[0].latest = Some(0);
    }
    assert_eq!(
        anchor_candidates(
            &replay,
            2,
            omitted,
            omitted.field_count(),
            replay.format.version()
        )
        .unwrap(),
        (Some(0), Some(0))
    );
    assert_eq!(
        anchor_candidates(
            &replay,
            2,
            TableId::Countries,
            TableId::Countries.field_count(),
            replay.format.version()
        )
        .unwrap(),
        (Some(0), Some(1))
    );
    let frames = replay
        .read_block_selected(&replay.index[2], &[TableId::Countries, omitted])
        .unwrap();
    assert!(
        matches!(&frames[0].tables[TableId::Countries.index()], TableUpdate::Snapshot(rows) if rows.len() == 256)
    );
    assert_eq!(
        frames[0].tables[omitted.index()],
        TableUpdate::NotRecorded {
            mode: replay.capture_mode
        }
    );
}
