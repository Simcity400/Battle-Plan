//! Grouped block layout: section costs, anchors, patches, predictions and
//! exact round trips.

use super::*;
use crate::rows::{ArmyOrderRow, CountryRow, DivisionRow, LexicalTokenRow, ProvinceRow};

fn stored(mut frame: DecodedFrame) -> DecodedFrame {
    frame.metrics = frame.metrics.stored();
    frame
}

/// One session's records for these decoded hours, through the spool encoder.
fn session_records(frames: &[DecodedFrame]) -> Vec<Record> {
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, frames[0].hour)),
    ];
    for frame in frames {
        records.push(Record::Frame(encoder.encode(frame.clone(), false).unwrap()));
    }
    records
}

fn spool_frames(records: &[Record]) -> Vec<Frame> {
    records
        .iter()
        .filter_map(|record| match record {
            Record::Frame(frame) => Some(Frame {
                metrics: frame.metrics.stored(),
                ..frame.clone()
            }),
            _ => None,
        })
        .collect()
}

fn size(replay: &Replay, block: usize, section: Section) -> Option<SectionSize> {
    replay
        .block_sections(&replay.index[block])
        .unwrap()
        .expect("grouped block")
        .into_iter()
        .find(|size| size.section == section)
}

/// Every decoded hour of a replay, block by block.
fn decoded_hours(replay: &Replay) -> Vec<DecodedFrame> {
    let mut out = Vec::new();
    for block in &replay.index {
        let mut decoder = FrameDecoder::new();
        for frame in replay.read_block(block).unwrap() {
            out.push(decoder.advance(&frame).unwrap().clone());
        }
    }
    out
}

#[test]
fn grouped_frame_headers_cost_bytes_not_a_word_per_table_and_keep_read_hours() {
    let frames: Vec<DecodedFrame> = (1..=200)
        .map(|hour| {
            let mut frame = source(1, hour);
            frame.metrics.total_ns = 12_345_678 + hour as u64 * 17;
            frame.metrics.pool_bytes = (1 << 20) + hour as u64;
            // A read shorter than a microsecond is still a read.
            frame.metrics.table_ns[TableId::Countries.index()] = 1;
            frame.metrics.table_ns[TableId::Provinces.index()] = 2_000_000 + hour as u64 * 999;
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[session_records(&frames)]).unwrap();
    assert_eq!(replay.index.len(), 1);
    let header = size(&replay, 0, Section::Header).unwrap();
    let diagnostics = size(&replay, 0, Section::Diagnostics).unwrap();
    // A plain frame header is 37 bytes plus a u64 per table.
    let plain_header = 37 + 8 * TableId::ALL.len() as u64;
    // By owner decision, the raw allowance scales with registered tables. The
    // catalogue through RegionalConvoyEfficiency gets the original 100 B/frame.
    let baseline_tables = TableId::ALL
        .iter()
        .filter(|&&table| table <= TableId::RegionalConvoyEfficiency)
        .count();
    let raw_allowance = 100.0 / baseline_tables as f64 * TableId::ALL.len() as f64;
    assert!(
        (header.raw + diagnostics.raw) as f64 / (frames.len() as f64) < raw_allowance,
        "{header:?} {diagnostics:?} against {plain_header} plain bytes per frame"
    );
    assert!(
        (header.compressed + diagnostics.compressed) / 200 < 16,
        "{header:?} {diagnostics:?}"
    );
    for (decoded, expected) in decoded_hours(&replay).iter().zip(&frames) {
        assert_eq!(decoded.metrics.total_ns, expected.metrics.total_ns);
        assert_eq!(decoded.metrics.pool_bytes, expected.metrics.pool_bytes);
        assert_eq!(decoded.metrics, expected.metrics.stored());
        assert_eq!(
            decoded.metrics.table_ns[TableId::Countries.index()],
            crate::frame::READ_TIME_NOT_RECORDED
        );
        assert!(!decoded.metrics.table_times_recorded());
        for table in TableId::ALL {
            let read = expected.metrics.table_ns[table.index()] != 0;
            assert_eq!(
                decoded.tables[table.index()].sampled_at(),
                read.then_some(decoded.hour),
                "{table:?}"
            );
        }
    }
}

/// One block written directly, given its frames.
fn written_block(frames: &[Frame], layout: grouped::Layout) -> CompressedBlock {
    let mut block = grouped::BlockWriter::with_layout(&frames[0], layout);
    let mut decoder = FrameDecoder::new();
    let mut anchors = grouped::Anchors::default();
    let mut aliases = aliases::Encoder::default();
    let cores = pack::Cores::new(1);
    for frame in frames {
        let decoded = decoder.advance(frame).unwrap();
        let mut payload = Vec::new();
        let sizes = frame.encode_with_sizes(&mut payload).unwrap();
        let prepared = aliases.prepare(frame, decoded, &payload, &sizes).unwrap();
        block
            .push_marked(
                frame,
                &prepared.views,
                &prepared.payload,
                &prepared.sizes,
                &mut anchors,
                MAX_BLOCK_RAW_BYTES,
                &cores,
                &prepared.private,
            )
            .unwrap();
        aliases.commit(prepared);
    }
    block.compress(&cores).unwrap()
}

fn one_block_in_layout(replay: &Replay, frames: &[Frame], layout: grouped::Layout) -> Replay {
    let mut prefix = replay.bytes()[..replay.header_end].to_vec();
    let mut index = Vec::new();
    append_blocks(&mut prefix, &mut index, &[written_block(frames, layout)]).unwrap();
    Replay::open(rebuild(replay, prefix, &index)).unwrap()
}

/// The chosen later section's leading masks-column byte, without decoding
/// its rows or inferring the choice from compressed size.
fn later_row_coding(replay: &Replay, table: TableId) -> u8 {
    let (_, raw) = grouped::stored_section(replay, 0, Section::Later(table)).unwrap();
    let mut input = Decoder::new(&raw);
    let count = usize::try_from(input.varint().unwrap()).unwrap();
    let lengths: Vec<usize> = (0..count)
        .map(|_| usize::try_from(input.varint().unwrap()).unwrap())
        .collect();
    assert!(lengths[2] > 0, "missing row coding for {table:?}");
    input.take(lengths[0] + lengths[1]).unwrap();
    input.u8().unwrap()
}

#[test]
fn grouped_diagnostics_store_which_tables_were_read_not_their_times() {
    let diagnostics = |time: &dyn Fn(i32, usize) -> u64| {
        let frames: Vec<DecodedFrame> = (1..=50)
            .map(|hour| {
                let mut frame = source(1, hour);
                for table in TableId::ALL {
                    if table.index() % 3 == 0 {
                        frame.metrics.table_ns[table.index()] = time(hour, table.index());
                    }
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[session_records(&frames)]).unwrap();
        for decoded in decoded_hours(&replay) {
            // A retired table is not recorded, so neither is its read.
            for table in TableId::ALL
                .into_iter()
                .filter(|&table| crate::frame::current_absence(table).is_none())
            {
                assert_eq!(
                    decoded.metrics.table_ns[table.index()] != 0,
                    table.index() % 3 == 0
                );
            }
        }
        size(&replay, 0, Section::Diagnostics).unwrap().raw
    };
    // Read times that would take one byte or four as microseconds store
    // the same bytes: only the read set is recorded.
    assert_eq!(
        diagnostics(&|_, _| 1),
        diagnostics(&|hour, table| 123_456_789 + (hour as u64) * 7_777 + table as u64)
    );
}

#[test]
fn earlier_grouped_blocks_keep_their_microsecond_read_times() {
    let frames: Vec<DecodedFrame> = (1..=4)
        .map(|hour| {
            let mut frame = source(1, hour);
            frame.metrics.table_ns[TableId::Clock.index()] = 2_000_000 + hour as u64 * 1_000;
            frame.metrics.table_ns[TableId::Countries.index()] = 1;
            frame
        })
        .collect();
    let records = session_records(&frames);
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    let spool: Vec<Frame> = records
        .iter()
        .filter_map(|record| match record {
            Record::Frame(frame) => Some(frame.clone()),
            _ => None,
        })
        .collect();
    let mut prefix = replay.bytes()[..replay.header_end].to_vec();
    let mut index = Vec::new();
    append_blocks(
        &mut prefix,
        &mut index,
        &[written_block(&spool, grouped::Layout::Timed)],
    )
    .unwrap();
    let timed = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
    let read = timed.read_block(&timed.index[0]).unwrap();
    for (frame, expected) in read.iter().zip(&frames) {
        assert!(frame.metrics.table_times_recorded());
        assert_eq!(
            frame.metrics.table_ns[TableId::Clock.index()],
            expected.metrics.table_ns[TableId::Clock.index()]
        );
        assert_eq!(frame.metrics.table_ns[TableId::Countries.index()], 1_000);
        assert_eq!(frame.metrics.stored(), expected.metrics.stored());
    }
    // The current writer drops the times and keeps everything else.
    let current = replay.read_block(&replay.index[0]).unwrap();
    assert!(
        current
            .iter()
            .all(|frame| !frame.metrics.table_times_recorded())
    );
    for (timed, current) in read.iter().zip(&current) {
        assert_eq!(timed.tables, current.tables);
        assert_eq!(timed.metrics.stored(), current.metrics);
    }
}

fn countries(count: u32) -> TableRows {
    TableRows::Countries(
        (0..count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                command_power_raw: i64::from(id) * 7,
                tag: format!("T{id:04}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    )
}

fn provinces(owner_of_seven: u32) -> TableRows {
    TableRows::Provinces(
        (1..=3_000)
            .map(|id| ProvinceRow {
                id,
                state: id / 4,
                owner: if id == 7 { owner_of_seven } else { id % 60 },
                controller: id % 60,
                victory_points: (id % 5) as i32,
                ..Default::default()
            })
            .collect(),
    )
}

#[test]
fn keyframes_store_unchanged_tables_at_constant_cost_against_their_anchor() {
    let mut frames = Vec::new();
    for hour in [1, 2, 3, 721, 722, 1441, 1442, 2161, 2162] {
        let mut frame = source(1, hour);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(countries(5_000));
        // One province changes owner between each pair of keyframes.
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(provinces(hour as u32 / 720));
        frames.push(frame);
    }
    let records = session_records(&frames);
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    assert_eq!(replay.index.len(), 4);
    let first_countries = size(&replay, 0, Section::Keyframe(TableId::Countries)).unwrap();
    let first_provinces = size(&replay, 0, Section::Keyframe(TableId::Provinces)).unwrap();
    assert!(first_countries.compressed > 4_000, "{first_countries:?}");
    assert!(first_provinces.compressed > 640, "{first_provinces:?}");
    for block in 1..4 {
        let countries = size(&replay, block, Section::Keyframe(TableId::Countries)).unwrap();
        let provinces = size(&replay, block, Section::Keyframe(TableId::Provinces)).unwrap();
        assert!(countries.compressed < 64, "block {block}: {countries:?}");
        assert!(provinces.compressed < 64, "block {block}: {provinces:?}");
    }
    // Every keyframe still decodes to its whole tables, sequentially, in
    // parallel, selected and by seeking into a later block.
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    assert_eq!(decoded_hours(&replay), expected);
    let spool = spool_frames(&records);
    let mut at = 0;
    for block in &replay.index {
        let read = replay.read_block(block).unwrap();
        assert_eq!(read, spool[at..at + read.len()]);
        at += read.len();
        let parallel = Parallel {
            workers: 4,
            on_worker: &|| {},
        };
        assert_eq!(replay.read_block_parallel(block, &parallel).unwrap(), read);
        let selected = replay
            .read_block_selected(block, &[TableId::Countries])
            .unwrap();
        for (selected, full) in selected.iter().zip(&read) {
            assert_eq!(
                selected.tables[TableId::Countries.index()],
                full.tables[TableId::Countries.index()]
            );
            assert!(matches!(
                &selected.tables[TableId::Provinces.index()],
                TableUpdate::Unavailable { reason } if reason.contains("not selected")
            ));
        }
    }
    assert!(
        matches!(replay.seek(1, 1442).unwrap(), SeekResult::Frame(frame) if *frame == expected[6])
    );
    // A block whose anchor is missing is refused, never read as empty.
    let mut prefix = replay.bytes()[..replay.header_end].to_vec();
    let second = &replay.index[1];
    let mut index = vec![BlockIndex {
        offset: prefix.len() as u64,
        ..second.clone()
    }];
    prefix.extend_from_slice(&replay.block_bytes(second).unwrap());
    index[0].offset = replay.header_end as u64;
    let orphan = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
    assert!(
        orphan
            .read_block(&orphan.index[0])
            .unwrap_err()
            .to_string()
            .contains("anchor")
    );
}

#[test]
fn one_changed_part_of_a_byte_field_is_stored_as_a_small_patch() {
    let literal: Vec<u8> = (0..65_536).map(|at| (at * 31 % 251) as u8).collect();
    let path: Vec<u32> = (0..10_000).collect();
    let mut frames = Vec::new();
    for hour in 1..=40 {
        let mut frame = source(1, hour);
        let mut text = literal.clone();
        text[30_000] = hour as u8;
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 3,
                literal: text,
            }]));
        // A window that drops its oldest entry and appends one each hour.
        let start = hour as u32;
        *frame.tables[TableId::ArmyOrders.index()] =
            TableView::Complete(TableRows::ArmyOrders(vec![ArmyOrderRow {
                owner_group: 9,
                instance: 1,
                path: path.iter().map(|step| step + start).collect(),
                ..Default::default()
            }]));
        frames.push(frame);
    }
    let records = session_records(&frames);
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    assert_eq!(replay.index.len(), 1);
    let tokens = size(&replay, 0, Section::Later(TableId::LexicalTokens)).unwrap();
    let orders = size(&replay, 0, Section::Later(TableId::ArmyOrders)).unwrap();
    // A plain delta re-sends the whole 64 KiB literal and 40 KB path each hour.
    assert!(tokens.raw / 39 < 32, "{tokens:?}");
    assert!(orders.raw / 39 < 48, "{orders:?}");
    assert_eq!(
        replay.read_block(&replay.index[0]).unwrap(),
        spool_frames(&records)
    );
    assert_eq!(
        decoded_hours(&replay),
        frames.into_iter().map(stored).collect::<Vec<_>>()
    );
}

/// Every table's sample rows for 26 hours, with unavailable and truncated
/// tables, an empty one, and an unchanged one whose baseline is not the
/// previous hour.
fn every_update_kind_records() -> Vec<Record> {
    let mut frames = Vec::new();
    for hour in (1..=24).chain([30, 31]) {
        let mut frame = source(1, hour);
        for table in TableId::ALL {
            if let Some(rows) = crate::rows::sample_extended_table(table, hour as usize) {
                *frame.tables[table.index()] = TableView::Complete(rows);
            }
            frame.metrics.table_ns[table.index()] =
                (hour as u64 * 7 + table.index() as u64) % 5 * 1_499;
        }
        if hour % 6 == 0 {
            *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
                reason: "synthetic guarded read failure".into(),
            };
        }
        if hour == 9 {
            *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
                rows: TableRows::Provinces(vec![ProvinceRow {
                    id: 4,
                    ..Default::default()
                }]),
                missing: 3,
            };
        }
        *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(Vec::new()));
        frames.push(frame);
    }
    let mut records = session_records(&frames);
    // A baseline that names an hour other than the previous frame.
    if let Record::Frame(frame) = &mut records[5] {
        frame.tables[TableId::States.index()] = TableUpdate::Unchanged { baseline: -7 };
    }
    records
}

#[test]
fn grouped_blocks_round_trip_every_update_kind() {
    let records = every_update_kind_records();
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    assert_eq!(replay.index.len(), 1);
    let read = replay.read_block(&replay.index[0]).unwrap();
    assert_eq!(read, spool_frames(&records));
    let interleaved = interleaved_copy(&replay);
    assert_eq!(interleaved.read_block(&interleaved.index[0]).unwrap(), read);
    assert!(matches!(
        &read[3].tables[TableId::States.index()],
        TableUpdate::Unchanged { baseline: -7 }
    ));
    let parallel = Parallel {
        workers: 3,
        on_worker: &|| {},
    };
    assert_eq!(
        replay
            .read_block_parallel(&replay.index[0], &parallel)
            .unwrap(),
        read
    );
}

#[test]
fn interleaved_recordings_decode_and_write_again_to_the_same_hours() {
    let mut frames = Vec::new();
    for hour in [1, 2, 3, 721, 722, 900] {
        let mut frame = source(1, hour);
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(countries(100 + hour as u32 % 7));
        frame.metrics.table_ns[TableId::Countries.index()] = 3_000;
        frames.push(frame);
    }
    let grouped = Replay::from_records(CAMPAIGN.into(), &[session_records(&frames)]).unwrap();
    let interleaved = interleaved_copy(&grouped);
    assert!(
        interleaved
            .index
            .iter()
            .all(|block| interleaved.block_sections(block).unwrap().is_none())
    );
    let expected = decoded_hours(&grouped);
    assert_eq!(decoded_hours(&interleaved), expected);
    let again = interleaved.reencoded(|_, _, _| Ok(())).unwrap();
    assert!(
        again
            .index
            .iter()
            .all(|block| again.block_sections(block).unwrap().is_some())
    );
    assert_eq!(decoded_hours(&again), expected);
    assert_eq!(again.bytes(), grouped.bytes());
}

#[test]
fn hostile_grouped_blocks_are_refused() {
    let frames: Vec<DecodedFrame> = (1..=5).map(|hour| source(1, hour)).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[session_records(&frames)]).unwrap();
    let block = replay.index[0].clone();
    let read = |bytes: Vec<u8>| {
        Replay::open(bytes)
            .and_then(|replay| replay.read_block(&replay.index[0]).map(|_| ()))
            .unwrap_err()
            .to_string()
    };
    let mut bytes = replay.bytes().to_vec();
    bytes[block.offset as usize + 4] ^= 1;
    let error = read(bytes);
    assert!(error.contains("directory CRC"), "{error}");
    // A damaged section fails its zstd checksum.
    let mut bytes = replay.bytes().to_vec();
    bytes[(block.offset + block.length) as usize - 1] ^= 0x40;
    read(bytes);
    // Grouped blocks belong to grouped-format recordings only.
    let mut bytes = replay.bytes().to_vec();
    bytes[4..6].copy_from_slice(&101_u16.to_le_bytes());
    if let Ok(old) = Replay::open(bytes) {
        assert!(
            old.read_block(&old.index[0])
                .unwrap_err()
                .to_string()
                .contains("grouped block in a format 101")
        );
    }
}

/// Divisions whose organisation recovers and strength falls at constant
/// rates, one rate per division, in whole display steps (a tenth).
fn moving_divisions(hour: i32, count: u64) -> TableRows {
    TableRows::Divisions(
        (1..=count)
            .map(|id| DivisionRow {
                id,
                province: id as u32,
                organisation: 10_000 * (1_000 + 7 * i64::from(hour)),
                hit_points: 10_000 * (90_000 - id as i64 * i64::from(hour)),
                ..Default::default()
            })
            .collect(),
    )
}

fn moving_frames(hours: impl IntoIterator<Item = i32>) -> Vec<DecodedFrame> {
    hours
        .into_iter()
        .map(|hour| {
            let mut frame = source(1, hour);
            *frame.tables[TableId::Divisions.index()] =
                TableView::Complete(moving_divisions(hour, 200));
            frame
        })
        .collect()
}

#[test]
fn constant_rate_fields_cost_nothing_after_their_first_two_changes() {
    let later = |hours: i32| {
        let frames = moving_frames(1..=hours);
        let records = session_records(&frames);
        let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
        assert_eq!(replay.index.len(), 1);
        assert_eq!(
            replay.read_block(&replay.index[0]).unwrap(),
            spool_frames(&records)
        );
        assert_eq!(
            decoded_hours(&replay),
            frames.into_iter().map(stored).collect::<Vec<_>>()
        );
        let section = Section::Later(TableId::Divisions);
        let (_, raw) = grouped::stored_section(&replay, 0, section).unwrap();
        let mut input = Decoder::new(&raw);
        let count = usize::try_from(input.varint().unwrap()).unwrap();
        let lengths: Vec<_> = (0..count).map(|_| input.varint().unwrap()).collect();
        // Private marks have one empty-change byte per later hour, outside
        // the numeric predictor's rows, masks and value columns.
        let aliases = u64::try_from(*lengths.last().unwrap()).unwrap();
        assert_eq!(aliases, (hours - 1) as u64);
        (
            size(&replay, 0, section).unwrap().raw - aliases,
            size(&replay, 0, Section::Later(TableId::Clock)),
        )
    };
    let (short, short_clock) = later(300);
    let (long, long_clock) = later(400);
    // Hours 2 and 3 store each change; from hour 4 every change is the
    // predicted one, so a hundred more hours of 200 moving rows and of the
    // clock's hour add no entry.
    assert_eq!(long, short);
    assert_eq!(
        long_clock.map(|size| size.raw),
        short_clock.map(|size| size.raw)
    );
}

/// Divisions whose orders group counts the hours of the day, each from its
/// own start, and wraps at midnight.
fn wrapping(hour: i32, divisions: u64) -> TableRows {
    TableRows::Divisions(
        (1..=divisions)
            .map(|id| DivisionRow {
                id,
                orders_group: (hour as u64 + id) % 24,
                ..Default::default()
            })
            .collect(),
    )
}

#[test]
fn a_wrapping_clock_decodes_exactly_and_never_costs_more_than_plain_deltas() {
    let later = |hours: i32, layout| {
        let frames: Vec<DecodedFrame> = (1..=hours)
            .map(|hour| {
                let mut frame = source(1, hour);
                *frame.tables[TableId::Divisions.index()] =
                    TableView::Complete(wrapping(hour, 100));
                frame
            })
            .collect();
        let records = session_records(&frames);
        let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
        let spool = spool_frames(&records);
        let mut prefix = replay.bytes()[..replay.header_end].to_vec();
        let mut index = Vec::new();
        append_blocks(&mut prefix, &mut index, &[written_block(&spool, layout)]).unwrap();
        let replay = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
        assert_eq!(
            decoded_hours(&replay),
            frames.into_iter().map(stored).collect::<Vec<_>>()
        );
        size(&replay, 0, Section::Later(TableId::Divisions))
            .unwrap()
            .compressed
    };
    // Each division's count wraps from 23 to 0 once a day. The block keeps residual
    // entries only when they compress smaller than plain deltas.
    for hours in [48, 240] {
        assert!(
            later(hours, grouped::Layout::Predicted) <= later(hours, grouped::Layout::Unpredicted)
        );
    }
}

#[test]
fn predictions_restart_at_keyframes_and_branch_starts_and_seek_exactly() {
    // Two blocks of one branch, then a loaded branch of the same session.
    let mut frames = moving_frames((1..=730).chain(1_000..=1_010));
    for frame in &mut frames[730..] {
        frame.branch = 2;
    }
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for frame in &frames {
        if frame.hour == 1_000 {
            records.push(Record::BranchStart(BranchStart {
                id: 2,
                session: 1,
                loaded_hour: Some(999),
                first_hour: 1_000,
                parent: crate::branches::Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            }));
        }
        records.push(Record::Frame(encoder.encode(frame.clone(), false).unwrap()));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    assert_eq!(replay.index.len(), 3);
    assert_eq!(replay.index[1].first_hour, 721);
    assert_eq!(replay.index[2].branch, 2);
    // Each block stores the first two changes again and nothing after them:
    // the 720-hour block costs no more than the 10-hour ones.
    let later: Vec<SectionSize> = (0..3)
        .map(|block| size(&replay, block, Section::Later(TableId::Divisions)).unwrap())
        .collect();
    assert!(
        later
            .iter()
            .all(|size| later[0].compressed <= size.compressed + 16),
        "{later:?}"
    );
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    assert_eq!(decoded_hours(&replay), expected);
    let spool = spool_frames(&records);
    let mut at = 0;
    for block in &replay.index {
        let read = replay.read_block(block).unwrap();
        assert_eq!(read, spool[at..at + read.len()]);
        at += read.len();
        let parallel = Parallel {
            workers: 4,
            on_worker: &|| {},
        };
        assert_eq!(replay.read_block_parallel(block, &parallel).unwrap(), read);
        let selected = replay
            .read_block_selected(block, &[TableId::Divisions])
            .unwrap();
        for (selected, full) in selected.iter().zip(&read) {
            assert_eq!(
                selected.tables[TableId::Divisions.index()],
                full.tables[TableId::Divisions.index()]
            );
        }
    }
    for (branch, hour, position) in [(1, 4, 3), (1, 720, 719), (1, 725, 724), (2, 1_007, 737)] {
        assert!(
            matches!(replay.seek(branch, hour).unwrap(), SeekResult::Frame(frame) if *frame == expected[position]),
            "branch {branch} hour {hour}"
        );
    }
}

#[test]
fn unpredicted_grouped_blocks_still_decode_and_predicted_ones_need_their_layout() {
    let frames = moving_frames(1..=60);
    let records = session_records(&frames);
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    let spool = spool_frames(&records);
    let rewritten = |layout| {
        let mut prefix = replay.bytes()[..replay.header_end].to_vec();
        let mut index = Vec::new();
        append_blocks(&mut prefix, &mut index, &[written_block(&spool, layout)]).unwrap();
        Replay::open(rebuild(&replay, prefix, &index)).unwrap()
    };
    let chosen = rewritten(grouped::Layout::Chosen);
    assert_eq!(chosen.bytes(), replay.bytes());
    let advanced = rewritten(grouped::Layout::Advanced);
    let trimmed = rewritten(grouped::Layout::Trimmed);
    let modelled = rewritten(grouped::Layout::Modelled);
    let ramped = rewritten(grouped::Layout::Ramped);
    let keyed = rewritten(grouped::Layout::Keyed);
    let anchored = rewritten(grouped::Layout::Anchored);
    let predicted = rewritten(grouped::Layout::Predicted);
    let unpredicted = rewritten(grouped::Layout::Unpredicted);
    let magic = |replay: &Replay| replay.bytes()[replay.index[0].offset as usize..][..4].to_vec();
    assert_eq!(magic(&chosen), b"H4GC");
    assert_eq!(magic(&advanced), b"H4G9");
    assert_eq!(magic(&trimmed), b"H4G8");
    assert_eq!(magic(&modelled), b"H4G7");
    assert_eq!(magic(&ramped), b"H4G6");
    assert_eq!(magic(&keyed), b"H4G5");
    assert_eq!(magic(&anchored), b"H4G4");
    assert_eq!(magic(&predicted), b"H4G3");
    assert_eq!(magic(&unpredicted), b"H4G2");
    let expected = frames.into_iter().map(stored).collect::<Vec<_>>();
    assert_eq!(decoded_hours(&chosen), expected);
    assert_eq!(chosen.read_block(&chosen.index[0]).unwrap(), spool);
    assert_eq!(decoded_hours(&trimmed), expected);
    assert_eq!(trimmed.read_block(&trimmed.index[0]).unwrap(), spool);
    assert_eq!(decoded_hours(&modelled), expected);
    assert_eq!(modelled.read_block(&modelled.index[0]).unwrap(), spool);
    assert_eq!(decoded_hours(&ramped), expected);
    assert_eq!(ramped.read_block(&ramped.index[0]).unwrap(), spool);
    assert_eq!(decoded_hours(&keyed), expected);
    assert_eq!(keyed.read_block(&keyed.index[0]).unwrap(), spool);
    assert_eq!(decoded_hours(&anchored), expected);
    assert_eq!(anchored.read_block(&anchored.index[0]).unwrap(), spool);
    assert_eq!(decoded_hours(&predicted), expected);
    assert_eq!(predicted.read_block(&predicted.index[0]).unwrap(), spool);
    assert_eq!(decoded_hours(&unpredicted), expected);
    assert_eq!(
        unpredicted.read_block(&unpredicted.index[0]).unwrap(),
        spool
    );
    let stored_later =
        |replay: &Replay| size(replay, 0, Section::Later(TableId::Divisions)).unwrap();
    assert!(stored_later(&predicted).raw * 10 < stored_later(&unpredicted).raw);
    assert!(stored_later(&predicted).compressed < stored_later(&unpredicted).compressed);
    // A predicted block read as the earlier layout has residual entries
    // that layout cannot hold.
    let mut bytes = predicted.bytes().to_vec();
    let offset = predicted.index[0].offset as usize;
    bytes[offset..offset + 4].copy_from_slice(b"H4G2");
    let error = Replay::open(bytes)
        .and_then(|replay| replay.read_block(&replay.index[0]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid in its section"), "{error}");
}

/// 400 divisions with wide handles (the id in the high 32 bits), about a
/// third of which change their reveal byte each hour; five are inserted
/// between them at hour 40 and five deleted at hour 80.
fn sparse_frames() -> Vec<DecodedFrame> {
    let handle = |n: u64| (n << 32) | 0x33;
    let mut state: std::collections::BTreeMap<u64, u8> =
        (1..=400_u64).map(|n| (handle(n * 977), 0)).collect();
    (1..=120)
        .map(|hour: i32| {
            if hour == 40 {
                for n in 1..=5_u64 {
                    state.insert(handle(n * 977 + 500), 0);
                }
            }
            if hour == 80 {
                for n in 10..=14_u64 {
                    state.remove(&handle(n * 977));
                }
            }
            for (id, reveal) in state.iter_mut() {
                let n = *id >> 32;
                if (n.wrapping_mul(2_654_435_761) ^ (hour as u64).wrapping_mul(40_503)) % 10 < 3 {
                    *reveal = reveal.wrapping_add(1);
                }
            }
            let mut frame = source(1, hour);
            *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
                state
                    .iter()
                    .map(|(&id, &reveal_skip)| DivisionRow {
                        id,
                        reveal_skip,
                        ..Default::default()
                    })
                    .collect(),
            ));
            frame
        })
        .collect()
}

#[test]
fn keyed_blocks_name_changing_rows_by_position_and_masks_by_recency() {
    let frames = sparse_frames();
    let records = session_records(&frames);
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    assert_eq!(replay.index.len(), 1);
    let spool = spool_frames(&records);
    assert_eq!(replay.read_block(&replay.index[0]).unwrap(), spool);
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    assert_eq!(decoded_hours(&replay), expected);
    let selected = replay
        .read_block_selected(&replay.index[0], &[TableId::Divisions])
        .unwrap();
    for (selected, full) in selected.iter().zip(&spool) {
        assert_eq!(
            selected.tables[TableId::Divisions.index()],
            full.tables[TableId::Divisions.index()]
        );
    }
    let mut prefix = replay.bytes()[..replay.header_end].to_vec();
    let mut index = Vec::new();
    append_blocks(
        &mut prefix,
        &mut index,
        &[written_block(&spool, grouped::Layout::Anchored)],
    )
    .unwrap();
    let anchored = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
    assert_eq!(decoded_hours(&anchored), expected);
    // Both layouts at the level the writer chooses encodings with, and as
    // stored.
    let later = |replay: &Replay| {
        let (stored, raw) =
            grouped::stored_section(replay, 0, Section::Later(TableId::Divisions)).unwrap();
        (grouped::compress(&raw).unwrap().len(), stored.len())
    };
    let ((keyed, keyed_stored), (anchored, anchored_stored)) = (later(&replay), later(&anchored));
    // A third of 400 wide keys change each hour: each costs a few bits of
    // position instead of the varint of its key difference, and its
    // repeated mask a byte.
    assert!(
        keyed * 3 < anchored * 2,
        "keyed {keyed} against anchored {anchored}"
    );
    assert!(
        keyed_stored * 4 < anchored_stored * 3,
        "stored keyed {keyed_stored} against anchored {anchored_stored}"
    );
}

#[test]
fn every_layout_reads_every_update_kind_as_written() {
    let records = every_update_kind_records();
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    let spool = spool_frames(&records);
    for layout in [
        grouped::Layout::Chosen,
        grouped::Layout::Advanced,
        grouped::Layout::Trimmed,
        grouped::Layout::Modelled,
        grouped::Layout::Ramped,
        grouped::Layout::Keyed,
    ] {
        let mut prefix = replay.bytes()[..replay.header_end].to_vec();
        let mut index = Vec::new();
        append_blocks(&mut prefix, &mut index, &[written_block(&spool, layout)]).unwrap();
        let rewritten = Replay::open(rebuild(&replay, prefix, &index)).unwrap();
        assert_eq!(
            rewritten.read_block(&rewritten.index[0]).unwrap(),
            spool,
            "{layout:?}"
        );
    }
}

/// 400 divisions with wide handles that change their reveal byte by an
/// unpredictable amount in streaks: one that changed in an hour mostly
/// changes in the next, one that did not rarely starts. Five more are
/// inserted between them at hour 300 and deleted at hour 600.
fn streaky_frames(hours: std::ops::RangeInclusive<i32>) -> Vec<DecodedFrame> {
    let handle = |n: u64| (n << 32) | 0x33;
    let draw = |n: u64, hour: u64| {
        ((n.wrapping_mul(2_654_435_761) ^ hour.wrapping_mul(40_503))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            >> 32)
            % 100
    };
    let mut state: std::collections::BTreeMap<u64, (bool, u8)> = (1..=400_u64)
        .map(|n| (handle(n * 977), (false, 0)))
        .collect();
    hours
        .map(|hour: i32| {
            let inserted = (1..=5_u64).map(|n| handle(n * 977 + 500));
            if hour == 300 {
                state.extend(inserted.map(|id| (id, (false, 0))));
            } else if hour == 600 {
                for id in inserted {
                    state.remove(&id);
                }
            }
            for (id, (on, reveal)) in state.iter_mut() {
                let (n, hour) = (*id >> 32, hour as u64);
                let below = if *on { 85 } else { 10 };
                *on = draw(n, hour) < below;
                if *on {
                    *reveal = reveal.wrapping_add(1 + (draw(n, hour + 100_000) % 200) as u8);
                }
            }
            let mut frame = source(1, hour);
            *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
                state
                    .iter()
                    .map(|(&id, &(_, reveal_skip))| DivisionRow {
                        id,
                        reveal_skip,
                        ..Default::default()
                    })
                    .collect(),
            ));
            frame
        })
        .collect()
}

#[test]
fn modelled_blocks_decode_exactly_and_store_streaky_rows_smaller_than_keyed_ones() {
    let frames = streaky_frames(1..=800);
    let records = session_records(&frames);
    let written = |layout| {
        grouped::WRITTEN_LAYOUT.set(layout);
        let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records));
        grouped::WRITTEN_LAYOUT.set(grouped::Layout::Chosen);
        replay.unwrap()
    };
    let ramped = written(grouped::Layout::Ramped);
    let modelled = written(grouped::Layout::Modelled);
    let magic = |replay: &Replay, block: usize| {
        replay.bytes()[replay.index[block].offset as usize..][..4].to_vec()
    };
    // The second block starts again at its keyframe.
    assert_eq!((ramped.index.len(), modelled.index.len()), (2, 2));
    for block in 0..2 {
        assert_eq!(magic(&ramped, block), b"H4G6");
        assert_eq!(magic(&modelled, block), b"H4G7");
    }
    let spool = spool_frames(&records);
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    for replay in [&ramped, &modelled] {
        assert_eq!(decoded_hours(replay), expected);
        let mut at = 0;
        for block in &replay.index {
            let read = replay.read_block(block).unwrap();
            assert_eq!(read, spool[at..at + read.len()]);
            at += read.len();
            let selected = replay
                .read_block_selected(block, &[TableId::Divisions])
                .unwrap();
            for (selected, full) in selected.iter().zip(&read) {
                assert_eq!(
                    selected.tables[TableId::Divisions.index()],
                    full.tables[TableId::Divisions.index()]
                );
            }
        }
        assert_eq!(at, spool.len());
    }
    for hour in [1, 2, 299, 300, 301, 600, 720, 721, 722, 800] {
        assert!(
            matches!(modelled.seek(1, hour).unwrap(), SeekResult::Frame(frame) if *frame == expected[hour as usize - 1]),
            "hour {hour}"
        );
    }
    // Each block's Divisions keys and masks columns, each compressed alone.
    let rows = |replay: &Replay| {
        replay
            .index
            .iter()
            .map(|block| {
                let columns = replay.block_columns(block).unwrap().unwrap();
                let later = columns
                    .iter()
                    .find(|sizes| sizes.section == Section::Later(TableId::Divisions))
                    .unwrap();
                later.columns[1].alone + later.columns[2].alone
            })
            .sum::<u64>()
    };
    // A changing row mostly changes again the hour after: the model names
    // it in a fraction of a bit, the keyed layout by the rows it skips.
    let (ramped, modelled) = (rows(&ramped), rows(&modelled));
    assert!(
        modelled * 3 < ramped * 2,
        "modelled {modelled} against ramped {ramped}"
    );
}

/// 660 owners rotate through 22 phases, each read hour changing only when
/// its owner's phase is read. Each entry names rows the last two did not.
fn rotating_slice_frames() -> Vec<DecodedFrame> {
    use crate::rows::SliceReadRow;
    (1..=240)
        .map(|hour: i32| {
            let mut frame = source(1, hour);
            *frame.tables[TableId::SliceReads.index()] =
                TableView::Complete(TableRows::SliceReads(
                    (0..660_u32)
                        .map(|owner| SliceReadRow {
                            table: TableId::ResearchTechnologies.index() as u32,
                            owner,
                            sample_hour: hour - (hour - owner as i32).rem_euclid(22),
                        })
                        .collect(),
                ));
            frame
        })
        .collect()
}

#[test]
fn chosen_blocks_list_rotating_slices_and_decode_every_hour_in_every_view() {
    let frames = rotating_slice_frames();
    let records = session_records(&frames);
    let chosen = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    assert_eq!(chosen.index.len(), 1);
    assert_eq!(
        &chosen.bytes()[chosen.index[0].offset as usize..][..4],
        b"H4GC"
    );
    let spool = spool_frames(&records);
    let advanced = one_block_in_layout(&chosen, &spool, grouped::Layout::Advanced);
    assert_eq!(later_row_coding(&chosen, TableId::SliceReads), 1);
    let later = |replay: &Replay| {
        size(replay, 0, Section::Later(TableId::SliceReads))
            .unwrap()
            .compressed
    };
    assert!(
        later(&chosen) < later(&advanced),
        "chosen {} against advanced {}",
        later(&chosen),
        later(&advanced)
    );
    assert!(chosen.bytes().len() < advanced.bytes().len());
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    for replay in [&chosen, &advanced] {
        assert_eq!(decoded_hours(replay), expected);
        assert_eq!(replay.read_block(&replay.index[0]).unwrap(), spool);
    }
    let selected = chosen
        .read_block_selected(&chosen.index[0], &[TableId::SliceReads])
        .unwrap();
    let mut decoder = FrameDecoder::for_tables(&[TableId::SliceReads]);
    for (frame, expected) in selected.iter().zip(&expected) {
        assert_eq!(
            decoder.advance(frame).unwrap().tables[TableId::SliceReads.index()],
            expected.tables[TableId::SliceReads.index()]
        );
    }
    // Seeks cover every phase and the block's boundaries; sequential reads
    // above already decoded every hour.
    for hour in (1..=23).chain([44, 45, 121, 220, 239, 240]) {
        assert!(
            matches!(chosen.seek(1, hour).unwrap(), SeekResult::Frame(frame) if *frame == expected[hour as usize - 1]),
            "hour {hour}"
        );
        let SeekResult::Frame(frame) = chosen
            .seek_selected(1, hour, Some(&[TableId::SliceReads]))
            .unwrap()
        else {
            panic!("missing selected hour {hour}");
        };
        assert_eq!(
            frame.tables[TableId::SliceReads.index()],
            expected[hour as usize - 1].tables[TableId::SliceReads.index()],
            "selected hour {hour}"
        );
    }
}

#[test]
fn chosen_blocks_keep_modelled_streaky_divisions() {
    let frames = streaky_frames(1..=240);
    let records = session_records(&frames);
    let chosen = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    assert_eq!(chosen.index.len(), 1);
    assert_eq!(
        &chosen.bytes()[chosen.index[0].offset as usize..][..4],
        b"H4GC"
    );
    let spool = spool_frames(&records);
    assert_eq!(later_row_coding(&chosen, TableId::Divisions), 0);
    // Alias metadata can tip the compressed plain/residual choice. Compare
    // modelled sections with the same residual choice, not different winners.
    grouped::FORCE_RESIDUAL.set(true);
    let modelled = one_block_in_layout(&chosen, &spool, grouped::Layout::Chosen);
    let advanced = one_block_in_layout(&chosen, &spool, grouped::Layout::Advanced);
    grouped::FORCE_RESIDUAL.set(false);
    assert_eq!(later_row_coding(&modelled, TableId::Divisions), 0);
    // The coding byte and at most one more byte for its column's length.
    let later = |replay: &Replay| size(replay, 0, Section::Later(TableId::Divisions)).unwrap();
    assert!(
        later(&modelled).raw > later(&advanced).raw
            && later(&modelled).raw <= later(&advanced).raw + 2,
        "modelled {:?} against advanced {:?}",
        later(&modelled),
        later(&advanced)
    );
    assert!(later(&chosen).compressed <= later(&modelled).compressed);
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    for replay in [&chosen, &modelled, &advanced] {
        assert_eq!(replay.read_block(&replay.index[0]).unwrap(), spool);
        assert_eq!(decoded_hours(replay), expected);
    }
}

/// A literal holding a nested little-endian counter between fixed bytes.
fn counter_literal(counter: u64, tail: &[u8]) -> Vec<u8> {
    let mut literal = b"name:".to_vec();
    literal.extend_from_slice(&counter.to_le_bytes());
    literal.extend_from_slice(tail);
    literal
}

#[test]
fn nested_counters_at_a_constant_rate_cost_nothing_after_their_first_two_changes() {
    let frames = |hours: &[(i32, u64, &[u8])]| -> Vec<DecodedFrame> {
        hours
            .iter()
            .map(|&(hour, counter, tail)| {
                let mut frame = source(1, hour);
                *frame.tables[TableId::LexicalTokens.index()] =
                    TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                        token: 3,
                        literal: counter_literal(counter, tail),
                    }]));
                frame
            })
            .collect()
    };
    let later = |frames: Vec<DecodedFrame>| {
        let records = session_records(&frames);
        let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
        assert_eq!(replay.index.len(), 1);
        assert_eq!(
            replay.read_block(&replay.index[0]).unwrap(),
            spool_frames(&records)
        );
        assert_eq!(
            decoded_hours(&replay),
            frames.into_iter().map(stored).collect::<Vec<_>>()
        );
        size(&replay, 0, Section::Later(TableId::LexicalTokens)).map_or(0, |size| size.raw)
    };
    let steady = |hours: i32| -> Vec<(i32, u64, &'static [u8])> {
        (1..=hours)
            .map(|hour| (hour, 250 + 7 * hour as u64, &b":tail"[..]))
            .collect()
    };
    // The counter crosses a byte boundary at 256 and still costs nothing.
    assert_eq!(later(frames(&steady(200))), later(frames(&steady(300))));
    // A tail edited while the counter moves is a patch against the
    // prediction; a counter that stops, starts again or changes rate decodes
    // exactly.
    let mut varied = steady(20);
    varied[10].2 = b":TAIL";
    for (hour, counter, _) in &mut varied[14..] {
        *counter = 400 + (*hour as u64 % 3);
    }
    varied.extend((21..=30).map(|hour| (hour, 9_000 - 5 * hour as u64, &b":tail"[..])));
    later(frames(&varied));
}

/// Provinces whose groups 1..=`changed` of a hundred have owners that do not
/// compress away: each block's keyframe changes one more group, so its
/// change from the previous keyframe stays small while its change from the
/// first snapshot grows with every block.
fn drifting_provinces(changed: usize) -> TableRows {
    let mut rows = provinces(0);
    if let TableRows::Provinces(rows) = &mut rows {
        for row in rows.iter_mut() {
            if (1..=changed).contains(&(row.id as usize / 100)) {
                row.owner = (u64::from(row.id).wrapping_mul(2_654_435_761) % 1_000_003) as u32;
            }
        }
    }
    rows
}

/// One keyframe hour per block, each changing one more province group.
fn drifting_frames(blocks: usize) -> (Vec<i32>, Vec<DecodedFrame>) {
    let hours: Vec<i32> = (0..blocks as i32).map(|block| 1 + 720 * block).collect();
    let frames = hours
        .iter()
        .enumerate()
        .map(|(block, &hour)| {
            let mut frame = source(1, hour);
            *frame.tables[TableId::Provinces.index()] =
                TableView::Complete(drifting_provinces(block));
            frame
        })
        .collect();
    (hours, frames)
}

/// Every hour of `expected`, in order, seeks to exactly its frame.
fn assert_seeks(replay: &Replay, expected: &[DecodedFrame]) {
    for frame in expected {
        assert!(
            matches!(replay.seek(frame.branch, frame.hour).unwrap(), SeekResult::Frame(found) if *found == *frame),
            "branch {} hour {}",
            frame.branch,
            frame.hour
        );
    }
}

#[test]
fn earlier_predicted_recordings_keep_their_chains_and_take_anchored_appends() {
    let (hours, frames) = drifting_frames(8);
    let records = session_records(&frames);
    grouped::WRITTEN_LAYOUT.set(grouped::Layout::Predicted);
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records));
    grouped::WRITTEN_LAYOUT.set(grouped::Layout::Keyed);
    let replay = replay.unwrap();
    assert_eq!(replay.index.len(), hours.len());
    let magic = |replay: &Replay, block: usize| {
        replay.bytes()[replay.index[block].offset as usize..][..4].to_vec()
    };
    assert!((0..hours.len()).all(|block| magic(&replay, block) == b"H4G3"));
    let keyframe = |replay: &Replay, block: usize| {
        size(replay, block, Section::Keyframe(TableId::Provinces))
            .unwrap()
            .compressed
    };
    // Blocks 1-4 anchor to the keyframe before them, a chain of four; block
    // 5 starts again from the snapshot, and block 6 anchors to block 5.
    for block in 2..=4 {
        assert!(
            keyframe(&replay, block) * 2 < keyframe(&replay, 1) * 3,
            "block {block}"
        );
    }
    assert!(keyframe(&replay, 5) > keyframe(&replay, 4) * 2);
    assert!(keyframe(&replay, 6) * 2 < keyframe(&replay, 5));
    let mut expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    assert_eq!(decoded_hours(&replay), expected);
    assert_eq!(
        replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .collect::<Vec<_>>(),
        spool_frames(&records)
    );
    assert_seeks(&replay, &expected);
    let selected = replay
        .read_block_selected(&replay.index[4], &[TableId::Provinces])
        .unwrap();
    assert_eq!(
        selected[0].tables[TableId::Provinces.index()],
        replay.read_block(&replay.index[4]).unwrap()[0].tables[TableId::Provinces.index()]
    );
    // An earlier layout anchors only to snapshots.
    let mut bytes = replay.bytes().to_vec();
    let offset = replay.index[3].offset as usize;
    bytes[offset..offset + 4].copy_from_slice(b"H4G2");
    let error = Replay::open(bytes)
        .and_then(|replay| replay.read_block(&replay.index[3]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a snapshot"), "{error}");
    // A later session appended by the current writer anchors to the
    // earlier layout's last keyframe and decodes through its chain.
    let later: Vec<DecodedFrame> = [9_000, 9_001]
        .into_iter()
        .map(|hour| {
            let mut frame = source(2, hour);
            *frame.tables[TableId::Provinces.index()] = TableView::Complete(drifting_provinces(8));
            frame
        })
        .collect();
    let mut encoder = FrameEncoder::new();
    let mut appended_records = vec![
        Record::SessionStart(session(2)),
        Record::BranchStart(branch(2, 9_000)),
    ];
    for frame in &later {
        appended_records.push(Record::Frame(encoder.encode(frame.clone(), false).unwrap()));
    }
    let appended = replay.append(&[appended_records]);
    grouped::WRITTEN_LAYOUT.set(grouped::Layout::Chosen);
    let appended = appended.unwrap();
    let last = appended.index.len() - 1;
    assert_eq!(magic(&appended, last), b"H4G5");
    assert!(
        keyframe(&appended, last) * 2 < keyframe(&appended, 0),
        "{} against the snapshot's {}",
        keyframe(&appended, last),
        keyframe(&appended, 0)
    );
    expected.extend(later.into_iter().map(stored));
    assert_eq!(decoded_hours(&appended), expected);
    assert_seeks(&appended, &expected);
}

#[test]
fn anchored_keyframes_chain_past_four_blocks_within_their_size_bound_and_seek_exactly() {
    let (hours, frames) = drifting_frames(24);
    let records = session_records(&frames);
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    assert_eq!(replay.index.len(), hours.len());
    let keyframe = |block: usize| {
        size(&replay, block, Section::Keyframe(TableId::Provinces))
            .unwrap()
            .compressed
    };
    // Past the fourth block each keyframe still stores only its own
    // hundred provinces against the keyframe before it.
    for block in 2..=8 {
        assert!(
            keyframe(block) * 2 < keyframe(1) * 3,
            "block {block}: {} against {}",
            keyframe(block),
            keyframe(1)
        );
    }
    // Each chain reads at most twice its snapshot's raw bytes and rebuilds
    // the rows at most sixteen times, however many blocks back the snapshot
    // is, so chains end and start again.
    let mut lengths = Vec::new();
    for block in 0..hours.len() {
        let chain = grouped::keyframe_chain(&replay, block, TableId::Provinces).unwrap();
        let (&(snapshot, root), anchored) = chain.split_last().unwrap();
        let charge: usize = anchored.iter().map(|(_, raw)| raw + root / 16).sum();
        assert!(charge <= root, "block {block}: {chain:?}");
        assert!(anchored.len() <= 16 && snapshot <= block);
        lengths.push(anchored.len());
    }
    assert!(
        lengths[1..=8].windows(2).all(|pair| pair[1] == pair[0] + 1),
        "{lengths:?}"
    );
    assert!(
        lengths.windows(2).any(|pair| pair[1] <= pair[0]),
        "{lengths:?}"
    );
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    assert_eq!(decoded_hours(&replay), expected);
    assert_eq!(
        replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .collect::<Vec<_>>(),
        spool_frames(&records)
    );
    assert_seeks(&replay, &expected);
    let deepest = lengths
        .iter()
        .enumerate()
        .max_by_key(|(_, length)| **length)
        .unwrap()
        .0;
    let parallel = Parallel {
        workers: 4,
        on_worker: &|| {},
    };
    let read = replay.read_block(&replay.index[deepest]).unwrap();
    assert_eq!(
        replay
            .read_block_parallel(&replay.index[deepest], &parallel)
            .unwrap(),
        read
    );
    let selected = replay
        .read_block_selected(&replay.index[deepest], &[TableId::Provinces])
        .unwrap();
    assert_eq!(
        selected[0].tables[TableId::Provinces.index()],
        read[0].tables[TableId::Provinces.index()]
    );
}

#[test]
fn reused_compression_contexts_write_the_bytes_of_fresh_ones_at_every_level() {
    let (_, frames) = drifting_frames(1);
    let mut frames = frames;
    frames.extend(moving_frames(2..=240));
    let replay = Replay::from_records(CAMPAIGN.into(), &[session_records(&frames)]).unwrap();
    let sections: Vec<Vec<u8>> = replay
        .block_sections(&replay.index[0])
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|section| {
            grouped::stored_section(&replay, 0, section.section)
                .unwrap()
                .1
        })
        .collect();
    assert!(sections.len() > 1);
    // Levels the writer uses (3, 9 and the stored level) and the windowed
    // high level the size survey compares.
    for level in [3, 9, grouped::STORED_LEVEL, 22] {
        for raw in &sections {
            // The first call creates the context; later calls reuse it.
            let (fresh, failed) = grouped::fresh_and_failed_reuse(raw, level).unwrap();
            assert!(failed, "a one-byte buffer must fail mid-frame");
            assert_eq!(
                grouped::compress_at(raw, level).unwrap(),
                fresh,
                "level {level}"
            );
            assert_eq!(
                grouped::compress_at(raw, level).unwrap(),
                fresh,
                "level {level}"
            );
        }
    }
}

#[test]
fn sections_are_stored_at_the_stored_level_where_that_is_smaller() {
    let (_, frames) = drifting_frames(1);
    let mut frames = frames;
    frames.extend(moving_frames(2..=240));
    let replay = Replay::from_records(CAMPAIGN.into(), &[session_records(&frames)]).unwrap();
    let mut smaller = 0;
    for section in replay.block_sections(&replay.index[0]).unwrap().unwrap() {
        let (bytes, raw) = grouped::stored_section(&replay, 0, section.section).unwrap();
        let choosing = grouped::compress(&raw).unwrap();
        let level = grouped::compress_at(&raw, grouped::STORED_LEVEL).unwrap();
        // The smaller of two whole frames of the same plain bytes.
        let at_stored_level = level.len() < choosing.len();
        smaller += usize::from(at_stored_level);
        assert_eq!(
            bytes,
            if at_stored_level { level } else { choosing },
            "{section:?}"
        );
    }
    assert!(smaller > 0, "no section is smaller at the stored level");
    // The provinces' snapshot takes fewer bytes than at level 3.
    let (snapshot, raw) =
        grouped::stored_section(&replay, 0, Section::Keyframe(TableId::Provinces)).unwrap();
    let level_three = grouped::compress_at(&raw, 3).unwrap();
    assert!(
        snapshot.len() < level_three.len(),
        "{} against {}",
        snapshot.len(),
        level_three.len()
    );
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    assert_eq!(decoded_hours(&replay), expected);
}

fn tokens(count: u32) -> TableRows {
    TableRows::LexicalTokens(
        (1..=count)
            .map(|token| LexicalTokenRow {
                token,
                literal: format!("token_{:010}", u64::from(token).wrapping_mul(2_654_435_761))
                    .into_bytes(),
            })
            .collect(),
    )
}

#[test]
fn static_tables_are_stored_once_per_file_across_branches_and_appended_sessions() {
    let frame = |branch: u64, hour: i32| {
        let mut frame = source(branch, hour);
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(tokens(4_000));
        frame
    };
    // Two blocks of a session's first branch, then a loaded branch.
    let first: Vec<DecodedFrame> = [(1, 1), (1, 2), (1, 721), (1, 722), (2, 100), (2, 101)]
        .into_iter()
        .map(|(branch, hour)| frame(branch, hour))
        .collect();
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(branch(1, 1)),
    ];
    for decoded in &first {
        if decoded.hour == 100 {
            records.push(Record::BranchStart(BranchStart {
                id: 2,
                session: 1,
                loaded_hour: Some(99),
                first_hour: 100,
                parent: crate::branches::Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            }));
        }
        records.push(Record::Frame(
            encoder.encode(decoded.clone(), false).unwrap(),
        ));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    // A later process loads the campaign again; its session is appended.
    let second: Vec<DecodedFrame> = [5, 6, 725].into_iter().map(|hour| frame(3, hour)).collect();
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(3)),
        Record::BranchStart(branch(3, 5)),
    ];
    for decoded in &second {
        records.push(Record::Frame(
            encoder.encode(decoded.clone(), false).unwrap(),
        ));
    }
    let appended = Replay::open(replay.append(&[records]).unwrap().bytes().to_vec()).unwrap();
    assert_eq!(appended.index.len(), 5);
    // The tokens are stored once; every later keyframe, in another branch
    // or session too, names them in a few bytes.
    let tokens = |block: usize| {
        size(&appended, block, Section::Keyframe(TableId::LexicalTokens))
            .unwrap()
            .compressed
    };
    assert!(tokens(0) > 10_000, "{}", tokens(0));
    for block in 1..5 {
        assert!(tokens(block) < 64, "block {block}: {}", tokens(block));
    }
    let expected: Vec<DecodedFrame> = first.into_iter().chain(second).map(stored).collect();
    assert_eq!(decoded_hours(&appended), expected);
    assert_seeks(&appended, &expected);
    for block in &appended.index {
        let read = appended.read_block(block).unwrap();
        let selected = appended
            .read_block_selected(block, &[TableId::LexicalTokens])
            .unwrap();
        for (selected, full) in selected.iter().zip(&read) {
            assert_eq!(
                selected.tables[TableId::LexicalTokens.index()],
                full.tables[TableId::LexicalTokens.index()]
            );
        }
    }
}

#[test]
fn nested_counters_that_move_by_a_little_store_small_differences() {
    // A counter in a nested record moves by one to three each hour, never
    // twice by the same amount in a row: no prediction, but each patch's
    // in-place byte is a small difference instead of the counter's low byte.
    let mut counter = 1_000_u64;
    let frames: Vec<DecodedFrame> = (1..=300)
        .map(|hour| {
            counter += 1 + (hour as u64 * 7 + hour as u64 / 3) % 3;
            let mut frame = source(1, hour);
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 3,
                    literal: counter_literal(counter, b":tail"),
                }]));
            frame
        })
        .collect();
    let records = session_records(&frames);
    let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records)).unwrap();
    let spool = spool_frames(&records);
    let rewritten = |layout| {
        let mut prefix = replay.bytes()[..replay.header_end].to_vec();
        let mut index = Vec::new();
        append_blocks(&mut prefix, &mut index, &[written_block(&spool, layout)]).unwrap();
        Replay::open(rebuild(&replay, prefix, &index)).unwrap()
    };
    let predicted = rewritten(grouped::Layout::Predicted);
    let unpredicted = rewritten(grouped::Layout::Unpredicted);
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    assert_eq!(decoded_hours(&predicted), expected);
    assert_eq!(decoded_hours(&unpredicted), expected);
    let later = |replay: &Replay| {
        size(replay, 0, Section::Later(TableId::LexicalTokens))
            .unwrap()
            .compressed
    };
    assert!(
        later(&predicted) * 4 < later(&unpredicted) * 3,
        "{} {}",
        later(&predicted),
        later(&unpredicted)
    );
}

/// 300 divisions walking 14-province paths: each reaches its next province
/// every three hours, at its own phase, and the path loses its head while
/// the source becomes the province it leaves; every 40 hours, at its own
/// phase, it is given a new path from a new source.
fn walking_frames(hours: std::ops::RangeInclusive<i32>) -> Vec<DecodedFrame> {
    hours
        .map(|hour| {
            let mut frame = source(1, hour);
            *frame.tables[TableId::DivisionMovement.index()] =
                TableView::Complete(TableRows::DivisionMovement(
                    (1..=300_u64)
                        .map(|n| {
                            let since = hour as u64 + n;
                            let (age, start) = (since % 40, since / 40 * 1_000 + n);
                            let dropped = (age + n % 3) / 3;
                            // Provinces without a steady difference, which no
                            // rate predicts.
                            let province = |step: u64| {
                                ((start * 7 + step * 13).wrapping_mul(2_654_435_761) % 10_007)
                                    as u32
                            };
                            crate::rows::DivisionMovementRow {
                                division: (n << 32) | 0x35,
                                // The province it leaves: the head the path
                                // dropped before the latest.
                                source: dropped
                                    .checked_sub(2)
                                    .map_or((start * 7 + 5) as u32, province),
                                path: (dropped..14).map(province).collect(),
                                ..Default::default()
                            }
                        })
                        .collect(),
                ));
            frame
        })
        .collect()
}

#[test]
fn a_path_that_loses_its_head_is_one_tag_and_decodes_exactly() {
    let table = TableId::DivisionMovement;
    let frames = walking_frames(1..=200);
    let records = session_records(&frames);
    // Residual entries, which alone store the tag, whichever the plain
    // entries would cost.
    grouped::FORCE_RESIDUAL.set(true);
    let written = |layout| {
        grouped::WRITTEN_LAYOUT.set(layout);
        let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records));
        grouped::WRITTEN_LAYOUT.set(grouped::Layout::Chosen);
        replay.unwrap()
    };
    let advanced = written(grouped::Layout::Advanced);
    let trimmed = written(grouped::Layout::Trimmed);
    let modelled = written(grouped::Layout::Modelled);
    grouped::FORCE_RESIDUAL.set(false);
    // Each later path that lost its head costs its tag, and any other
    // changed path at most its tag and plain encoding (a count and its
    // elements).
    let paths = |frame: &DecodedFrame| match &*frame.tables[table.index()] {
        TableView::Complete(TableRows::DivisionMovement(rows)) => {
            rows.iter().map(|row| row.path.clone()).collect::<Vec<_>>()
        }
        _ => unreachable!("every hour holds the walkers"),
    };
    let mut bound = 0;
    for pair in frames.windows(2) {
        for (before, after) in paths(&pair[0]).iter().zip(paths(&pair[1])) {
            if before.get(1..) == Some(after.as_slice()) {
                bound += 1;
            } else if *before != after {
                bound += 1 + 4 + 4 * after.len() as u64;
            }
        }
    }
    let spool = spool_frames(&records);
    let expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    for replay in [&advanced, &trimmed, &modelled] {
        assert_eq!(replay.index.len(), 1);
        assert_eq!(replay.read_block(&replay.index[0]).unwrap(), spool);
        assert_eq!(decoded_hours(replay), expected);
    }
    for hour in [1, 2, 3, 40, 41, 199, 200] {
        assert!(
            matches!(advanced.seek(1, hour).unwrap(), SeekResult::Frame(frame) if *frame == expected[hour as usize - 1]),
            "hour {hour}"
        );
    }
    // The later entries' path column, after control, keys and masks:
    // about a hundred heads a frame, each a tag where earlier layouts store
    // a patch, which takes them past the bound.
    let path = 3
        + (0..table.schema().len())
            .find(|&field| table.field_name(field) == Some("path"))
            .unwrap();
    let path_bytes = |replay: &Replay| {
        replay
            .block_columns(&replay.index[0])
            .unwrap()
            .unwrap()
            .into_iter()
            .find(|sizes| sizes.section == Section::Later(table))
            .unwrap()
            .columns[path]
            .raw
    };
    assert!(
        path_bytes(&trimmed) <= bound,
        "{} {bound}",
        path_bytes(&trimmed)
    );
    assert!(
        path_bytes(&modelled) > bound,
        "{} {bound}",
        path_bytes(&modelled)
    );
    // The advance changes only the source, each head's one small
    // correction.
    assert_eq!(path_bytes(&advanced), path_bytes(&trimmed));
    let later = |replay: &Replay| size(replay, 0, Section::Later(table)).unwrap().raw;
    assert!(
        later(&advanced) < later(&trimmed),
        "{} {}",
        later(&advanced),
        later(&trimmed)
    );
}

/// Four hours a block from `first` (a block per 720 hours): 200 standing
/// divisions and 40 walking ones, which reach their next province every
/// three steps at their own phase (the path loses its head and the source
/// becomes the head it lost before) and take a new path every 40 steps. A
/// block's keyframe then stores only the walkers against the block before,
/// whose paths lost one head or two since.
fn anchored_walking_frames(branch: u64, first: i32, blocks: u64) -> Vec<DecodedFrame> {
    let province = |start: u64, step: u64| {
        ((start * 7 + step * 13).wrapping_mul(2_654_435_761) % 10_007) as u32
    };
    (0..blocks * 4)
        .map(|t| {
            let mut frame = source(branch, first + 720 * (t / 4) as i32 + (t % 4) as i32);
            let walkers = (1..=40_u64).map(|n| {
                let since = t + n;
                let (age, start) = (since % 40, since / 40 * 1_000 + n);
                let dropped = (age + n % 3) / 3;
                crate::rows::DivisionMovementRow {
                    division: (n << 32) | 0x35,
                    source: dropped
                        .checked_sub(2)
                        .map_or(province(start, 99), |step| province(start, step)),
                    path: (dropped..14).map(|step| province(start, step)).collect(),
                    ..Default::default()
                }
            });
            let standing = (1_000..1_200_u64).map(|n| crate::rows::DivisionMovementRow {
                division: (n << 32) | 0x35,
                source: n as u32,
                path: (0..10).map(|step| (n * 10 + step) as u32).collect(),
                ..Default::default()
            });
            *frame.tables[TableId::DivisionMovement.index()] = TableView::Complete(
                TableRows::DivisionMovement(walkers.chain(standing).collect()),
            );
            frame
        })
        .collect()
}

fn walking_records(frames: &[DecodedFrame], branch: u64) -> Vec<Record> {
    let mut encoder = FrameEncoder::new();
    let mut records = vec![
        Record::SessionStart(session(branch)),
        Record::BranchStart(super::branch(branch, frames[0].hour)),
    ];
    for frame in frames {
        records.push(Record::Frame(encoder.encode(frame.clone(), false).unwrap()));
    }
    records
}

#[test]
fn advances_decode_exactly_after_anchored_keyframes_seeks_and_appends() {
    let frames = anchored_walking_frames(1, 1, 12);
    let records = walking_records(&frames, 1);
    // Residual entries, whose source codes take the advance, whichever the
    // plain entries would cost.
    grouped::FORCE_RESIDUAL.set(true);
    let written = |workers| {
        pack::WORKERS.set(Some(workers));
        let replay = Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&records));
        pack::WORKERS.set(None);
        replay.unwrap()
    };
    let replay = written(1);
    assert_eq!(replay.bytes(), written(3).bytes());
    assert_eq!(replay.index.len(), 12);
    // The keyframes after the first store the walkers' change against the
    // keyframe before, whose paths lost one head or two since.
    let keyframe = |replay: &Replay, block| {
        size(replay, block, Section::Keyframe(TableId::DivisionMovement))
            .unwrap()
            .raw
    };
    for block in 1..12 {
        assert!(
            keyframe(&replay, block) * 2 < keyframe(&replay, 0),
            "block {block}"
        );
    }
    let mut expected: Vec<DecodedFrame> = frames.into_iter().map(stored).collect();
    assert_eq!(decoded_hours(&replay), expected);
    assert_eq!(
        replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .collect::<Vec<_>>(),
        spool_frames(&records)
    );
    assert_seeks(&replay, &expected);
    // A later session continues the walk, its first keyframe anchored to
    // the file's last.
    let later = anchored_walking_frames(2, 20_001, 4);
    let appended = replay.append(&[walking_records(&later, 2)]).unwrap();
    grouped::FORCE_RESIDUAL.set(false);
    assert!(keyframe(&appended, 12) * 2 < keyframe(&appended, 0));
    expected.extend(later.into_iter().map(stored));
    assert_eq!(decoded_hours(&appended), expected);
    assert_seeks(&appended, &expected);
}

#[test]
fn a_selected_read_checks_every_section_of_a_block_not_yet_verified() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1, 2, 3])]).unwrap();
    let block = &replay.index[0];
    let table = |section: &grouped::SectionSize| match section.section {
        grouped::Section::Keyframe(table) | grouped::Section::Later(table) => Some(table),
        _ => None,
    };
    let sections = replay.block_sections(block).unwrap().unwrap();
    let selected = sections.iter().find_map(table).unwrap();
    assert_ne!(table(sections.last().unwrap()), Some(selected));
    // Damage the last section's checksum, which the selected table's read
    // never inflates.
    let mut bytes = replay.bytes().to_vec();
    bytes[(block.offset + block.length - 1) as usize] ^= 1;
    let damaged = Replay::open(bytes).unwrap();
    let mut visited = 0;
    assert!(
        damaged
            .visit_block(&damaged.index[0], Some(&[selected]), |_| {
                visited += 1;
                Ok(true)
            })
            .is_err()
    );
    assert_eq!(
        visited, 3,
        "the selected frames are visited before the check fails"
    );
    assert!(!damaged.is_verified(&damaged.index[0]));
    // An intact block is checked once; later selected reads trust it.
    assert!(!replay.is_verified(block));
    replay
        .visit_block(block, Some(&[selected]), |_| Ok(true))
        .unwrap();
    assert!(replay.is_verified(block));
}

/// Isolate key stability with the same current numeric-lane representation on
/// both sides. This is not an attempt to reopen or transcode an older replay.
#[test]
fn grouped_combat_log_observer_stable_keys_keep_exact_ordered_hours_and_seeks() {
    use crate::rows::{CombatLogObserverKind as K, CombatLogObserverRow, tables};
    fn observations(hour: i32, stable: bool) -> Vec<CombatLogObserverRow> {
        let mut rows = Vec::new();
        let mut put = |label, kind: K, item, values| {
            let mut row =
                CombatLogObserverRow::from_fact(7 << 32 | 62, true, kind as u8, item, values)
                    .unwrap();
            row.ordinal = if stable { label } else { rows.len() as u64 };
            rows.push(row);
        };
        put(0, K::State, 0, [hour as u64 * 100, hour as u64, 3, 0, 0]);
        for first in (0..30).step_by(5) {
            put(1 + first, K::Counters, first, [hour as u64; 5]);
        }
        // Changing an early collection length formerly rekeyed every activity.
        for index in 0..hour as u64 % 5 {
            put(
                10_000 + index,
                K::Leader,
                index,
                [(index + 1) << 32 | 64, hour as u64, 0, 0, 0],
            );
        }
        for parent in 0..32 {
            let label = 100 + parent * 20;
            put(
                label,
                K::Activity,
                parent,
                [(parent + 1) << 32 | 53, 11, 12, 0, 0],
            );
            for index in 0..16 {
                put(
                    label + 1 + index,
                    K::ActivityDamage,
                    parent,
                    [
                        index,
                        (parent * 16 + index + 1) << 32 | 51,
                        if hour < 40 {
                            hour as u64 * 13 + index
                        } else {
                            (hour as u64 - 40) * 7 + index
                        },
                        0,
                        0,
                    ],
                );
            }
            put(
                label + 17,
                K::ActivityEquipment,
                parent,
                [u64::MAX, 0, 1, 0, 0],
            );
        }
        rows.sort_unstable_by_key(CombatLogObserverRow::key);
        rows
    }
    let frames = |stable| {
        (1..=96)
            .map(|hour| {
                let mut frame = source(1, hour);
                *frame.tables[TableId::CombatLogObservers.index()] = if hour == 48 {
                    TableView::Unavailable {
                        reason: "synthetic observer refusal".into(),
                    }
                } else {
                    TableView::Complete(TableRows::CombatLogObservers(observations(hour, stable)))
                };
                frame.metrics.table_ns[TableId::CombatLogObservers.index()] = 1;
                frame
            })
            .collect::<Vec<_>>()
    };
    let stable_frames = frames(true);
    let stable = Replay::from_records(CAMPAIGN.into(), &[session_records(&stable_frames)]).unwrap();
    let flattened =
        Replay::from_records(CAMPAIGN.into(), &[session_records(&frames(false))]).unwrap();
    let section = Section::Later(TableId::CombatLogObservers);
    let stable_bytes = size(&stable, 0, section).unwrap().compressed;
    let flattened_bytes = size(&flattened, 0, section).unwrap().compressed;
    println!(
        "CombatLogObservers synthetic 96-hour section: stable keys {stable_bytes} compressed bytes, global ordinals {flattened_bytes}; numeric lanes identical"
    );
    assert!(
        stable_bytes < flattened_bytes,
        "{stable_bytes} vs {flattened_bytes}"
    );
    for (decoded, expected) in decoded_hours(&stable).into_iter().zip(stable_frames) {
        let SeekResult::Frame(seek) = stable.seek(1, expected.hour).unwrap() else {
            panic!("observer hour missing")
        };
        assert_eq!(
            decoded.tables[TableId::CombatLogObservers.index()],
            expected.tables[TableId::CombatLogObservers.index()]
        );
        assert_eq!(
            seek.tables[TableId::CombatLogObservers.index()],
            decoded.tables[TableId::CombatLogObservers.index()]
        );
        if expected.hour != 48 {
            let ordered = |frame: &DecodedFrame| {
                let mut rows = frame
                    .complete::<tables::CombatLogObservers>()
                    .unwrap()
                    .to_vec();
                rows.sort_unstable_by_key(CombatLogObserverRow::logical_order);
                rows.iter()
                    .map(CombatLogObserverRow::fact)
                    .collect::<Vec<_>>()
            };
            assert_eq!(ordered(&seek), ordered(&expected));
        } else {
            assert!(seek.complete::<tables::CombatLogObservers>().is_err());
        }
    }
}
