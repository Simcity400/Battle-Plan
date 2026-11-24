use super::*;
use core_spool::Type;
use std::{
    io::Cursor,
    sync::atomic::{AtomicU64, Ordering},
};
fn writer_allowances() -> core_spool::Allowances {
    core_spool::Allowances {
        dictionary: 262_144,
        reasons: 65_536,
    }
}
fn output() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join("hoi4-core-spool-gate-synthetic");
    fs::create_dir_all(&root).unwrap();
    loop {
        let path = root.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        if !path.exists() {
            return path;
        }
    }
}
fn event(hour: i32, kind: u8, key: u128) -> Vec<u8> {
    let mut b = hour.to_le_bytes().to_vec();
    b.push(kind);
    b.extend_from_slice(&(replay_data::rows::TableId::Countries as u16).to_le_bytes());
    b.extend_from_slice(&key.to_le_bytes());
    b
}
fn frame(stream: &mut Vec<u8>, record: &[u8]) {
    stream.extend_from_slice(&(record.len() as u64).to_le_bytes());
    stream.extend_from_slice(record);
}
fn blob(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}
#[test]
fn copied_gate_decoder_and_typed_spool_agree_for_every_value_and_notification() {
    let mut stream = Vec::new();
    let values = [
        vec![0, 7],
        vec![1, 0xf8],
        vec![2, 9, 0],
        vec![3, 0xf6, 0xff],
        vec![4, 11, 0, 0, 0],
        vec![5, 0xf4, 0xff, 0xff, 0xff],
        [vec![6], 13_u64.to_le_bytes().to_vec()].concat(),
        [vec![7], (-14_i64).to_le_bytes().to_vec()].concat(),
        vec![8, 1],
        [vec![9, 1], (-15_i32).to_le_bytes().to_vec()].concat(),
        vec![10, 0],
        vec![11, 0],
        [vec![12, 1], 16_u64.to_le_bytes().to_vec()].concat(),
        {
            let mut v = vec![13];
            v.extend_from_slice(&17_u32.to_le_bytes());
            v.push(1);
            blob(&mut v, b"synthetic text");
            v
        },
        {
            let mut v = vec![14];
            v.extend_from_slice(&2_u64.to_le_bytes());
            v.extend_from_slice(&18_u32.to_le_bytes());
            v.extend_from_slice(&19_u32.to_le_bytes());
            v
        },
        {
            let mut v = vec![15];
            v.extend_from_slice(&1_u64.to_le_bytes());
            v.extend_from_slice(&20_u64.to_le_bytes());
            v
        },
        {
            let mut v = vec![16];
            blob(&mut v, b"synthetic opaque");
            v
        },
        {
            let mut v = vec![13];
            v.extend_from_slice(&999_u32.to_le_bytes());
            v.push(0);
            v
        },
    ];
    for (i, value) in values.iter().enumerate() {
        let mut record = event(
            100 + i as i32,
            if i % 2 == 0 { 1 } else { 3 },
            123 + i as u128,
        );
        record.extend_from_slice(&0_u16.to_le_bytes());
        record.push(u8::from(i < 16));
        record.extend_from_slice(value);
        frame(&mut stream, &record);
    }
    for status in 0..4 {
        let mut record = event(120, 0, 0);
        record.push(status);
        if status == 2 {
            record.extend_from_slice(&23_u32.to_le_bytes());
        }
        if status == 3 {
            blob(&mut record, b"synthetic refusal");
        }
        record.push(1);
        record.extend_from_slice(&119_i32.to_le_bytes());
        record.extend_from_slice(&2_u64.to_le_bytes());
        record.extend_from_slice(&123_u128.to_le_bytes());
        record.extend_from_slice(&456_u128.to_le_bytes());
        frame(&mut stream, &record);
    }
    for kind in [2, 4, 5, 6] {
        let mut record = event(121, kind, 789);
        if kind >= 5 {
            record.push(3);
        }
        frame(&mut stream, &record);
    }
    let mut kinds = Vec::new();
    let mut side = convert::SourceKinds::new(&mut kinds);
    gate::replay(Cursor::new(&stream), &mut side, &Budget::new(2_000_000)).unwrap();
    assert!(side.error.is_none());
    for layout in [Layout::Rows, Layout::Columns] {
        for compression in [Compression::None, Compression::Lz4] {
            let path = output();
            let memory = Budget::new(2_000_000);
            let disk = Budget::new(10_000_000);
            let config = Config {
                block_bytes: 464,
                layout,
                compression,
            };
            let mut writer =
                core_spool::Writer::create(&path, config, writer_allowances(), &memory, &disk)
                    .unwrap();
            let mut convert = convert::Convert::new(&mut writer);
            assert_eq!(
                gate::replay(Cursor::new(&stream), &mut convert, &memory)
                    .unwrap()
                    .0,
                26
            );
            assert!(convert.error.is_none());
            let expected = convert.hash.finish();
            writer.finish().unwrap();
            let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
            let mut payload = PayloadReader::open(&path, 7, &memory).unwrap();
            let mut actual = convert::Canonical::default();
            let mut source_kinds = kinds.iter();
            loop {
                match reader.next_block().unwrap() {
                    Next::Block(block) => {
                        for hour in block.hours() {
                            for (table, field, r) in hour.ordered() {
                                let source_kind = if r.kind == core_spool::Kind::Unknown {
                                    source_kinds.next().copied()
                                } else {
                                    None
                                };
                                actual
                                    .spool_record(
                                        hour.hour,
                                        table,
                                        field,
                                        r,
                                        &mut payload.for_block(block.location),
                                        source_kind,
                                    )
                                    .unwrap();
                            }
                        }
                    }
                    Next::Finished => break,
                    Next::Pending => panic!("sealed"),
                }
            }
            assert!(source_kinds.next().is_none());
            assert_eq!(actual.finish(), expected);
        }
    }
}
#[test]
fn copied_gate_decoder_rejects_truncation_bad_bits_and_unknown_kinds() {
    let mut record = event(1, 3, 5);
    record.extend_from_slice(&0_u16.to_le_bytes());
    record.extend_from_slice(&[1, 8, 2]);
    assert!(gate::decode(&record, &mut gate::Noop).is_err());
    assert!(gate::decode(&record[..10], &mut gate::Noop).is_err());
    record[4] = 99;
    assert!(gate::decode(&record, &mut gate::Noop).is_err());
}

#[test]
fn measured_consumer_carries_typed_cells_and_evidence_across_local_id_reuse() {
    use core_spool::{Evidence, Kind, Record, Value, Writer};
    let root = output();
    fs::create_dir(&root).unwrap();
    let memory = Budget::new(16_000_000);
    let disk = Budget::new(100_000_000);
    let config = Config {
        block_bytes: 4096,
        layout: Layout::Columns,
        compression: Compression::Lz4,
    };
    let table = replay_data::rows::TableId::Countries as u32;
    let mut state = consumer::Consumer::create(
        &root.join("state"),
        admission::consumer_allowances(memory.limit() / 2).unwrap(),
        &memory,
        &disk,
    )
    .unwrap();
    for segment in 0..3 {
        let path = root.join(format!("segment-{segment}"));
        let mut writer =
            Writer::create(&path, config, writer_allowances(), &memory, &disk).unwrap();
        let mut append = |key: u128, kind, field, value| {
            writer
                .append(Record {
                    hour: segment,
                    kind,
                    table,
                    field,
                    key: &key.to_le_bytes(),
                    value,
                })
                .unwrap()
        };
        match segment {
            0 => {
                append(11, Kind::Appeared, 0, Value::Unit);
                append(11, Kind::Appeared, 0, Value::U32(7));
                append(11, Kind::Appeared, 2, Value::Bytes(b"first payload"));
                append(
                    11,
                    Kind::Unknown,
                    1,
                    Value::Evidence(Evidence {
                        code: 1,
                        reason: b"first reason",
                        first: 0,
                        last: 0,
                        sample: None,
                        missing: None,
                    }),
                );
                append(
                    0,
                    Kind::Refused,
                    0,
                    Value::Evidence(Evidence {
                        code: 3,
                        reason: b"refusal",
                        first: 0,
                        last: 0,
                        sample: Some(-1),
                        missing: None,
                    }),
                );
            }
            1 => {
                append(22, Kind::Appeared, 0, Value::Unit);
                append(22, Kind::Appeared, 0, Value::U32(55));
                append(22, Kind::Appeared, 2, Value::Bytes(b"second payload"));
                append(
                    22,
                    Kind::Unknown,
                    1,
                    Value::Evidence(Evidence {
                        code: 1,
                        reason: b"new reason",
                        first: 1,
                        last: 1,
                        sample: None,
                        missing: None,
                    }),
                );
                append(11, Kind::Changed, 0, Value::U32(9));
                append(
                    0,
                    Kind::NotRecorded,
                    0,
                    Value::Evidence(Evidence {
                        code: 0,
                        reason: b"not recorded",
                        first: 1,
                        last: 1,
                        sample: None,
                        missing: None,
                    }),
                );
            }
            _ => {
                append(11, Kind::Gone, 0, Value::Unit);
                append(11, Kind::Appeared, 0, Value::Unit);
                append(11, Kind::Appeared, 1, Value::U32(12));
                append(
                    0,
                    Kind::Status,
                    0,
                    Value::Evidence(Evidence {
                        code: 2,
                        reason: b"truncated",
                        first: 2,
                        last: 2,
                        sample: Some(2),
                        missing: Some(4),
                    }),
                );
            }
        }
        writer.finish().unwrap();
        state.begin_segment().unwrap();
        let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
        let mut payload = PayloadReader::open(&path, 31, &memory).unwrap();
        drain(&mut reader, &mut payload, Some(&mut state)).unwrap();
        if segment < 2 {
            assert_eq!(state.bytes(table as usize, 2, 0).unwrap(), b"first payload");
            let cell = state.cell(table as usize, 0, 0).unwrap().unwrap();
            assert_eq!(cell.1, Type::U32);
            assert_eq!(
                u32::from_le_bytes(cell.2.try_into().unwrap()),
                if segment == 0 { 7 } else { 9 }
            );
            let unknown = state.cell(table as usize, 1, 0).unwrap().unwrap();
            assert_eq!((unknown.0, unknown.1), (Kind::Unknown, Type::Evidence));
            assert_eq!(u64::from_le_bytes(unknown.2[1..9].try_into().unwrap()), 0);
            assert_eq!(i64::from_le_bytes(unknown.2[9..17].try_into().unwrap()), 0);
        } else {
            assert!(state.cell(table as usize, 0, 0).unwrap().is_none());
            assert_eq!(
                state.cell(table as usize, 1, 0).unwrap().unwrap().2,
                12_u32.to_le_bytes()
            );
        }
        assert_eq!(
            state.table_state(table as usize).unwrap().0,
            [Kind::Refused, Kind::NotRecorded, Kind::Status][segment as usize].code()
        );
    }
    assert_eq!(
        state.cell(table as usize, 0, 2).unwrap().unwrap().2,
        55_u32.to_le_bytes()
    );
    assert_eq!(state.applied, 15);
    assert!(memory.peak() <= memory.limit());
}

#[test]
fn consumer_keeps_realistic_columns_resident_with_a_shallow_index() {
    use core_spool::{Kind, Owner, Record, Value, Writer};
    let root = output();
    fs::create_dir(&root).unwrap();
    let memory = Budget::new(64_000_000);
    let disk = Budget::new(200_000_000);
    let config = Config {
        block_bytes: 65536,
        layout: Layout::Columns,
        compression: Compression::None,
    };
    let table = replay_data::rows::TableId::Countries as u32;
    let mut state = consumer::Consumer::create(
        &root.join("state"),
        admission::consumer_allowances(memory.limit() / 2).unwrap(),
        &memory,
        &disk,
    )
    .unwrap();
    let path = root.join("segment");
    let mut writer = Writer::create(&path, config, writer_allowances(), &memory, &disk).unwrap();
    for key in 0_u128..20_000 {
        for (kind, value) in [
            (Kind::Appeared, Value::Unit),
            (Kind::Appeared, Value::U32(key as u32)),
        ] {
            writer
                .append(Record {
                    hour: 0,
                    kind,
                    table,
                    field: 0,
                    key: &key.to_le_bytes(),
                    value,
                })
                .unwrap();
        }
    }
    writer.finish().unwrap();
    let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
    let mut payload = PayloadReader::open(&path, 65536, &memory).unwrap();
    state.begin_segment().unwrap();
    drain(&mut reader, &mut payload, Some(&mut state)).unwrap();
    assert_eq!(state.storage_shape().0, 1);
    let before = memory.paging()[Owner::CurrentValues as usize];
    for i in 0_u64..20_000 {
        let key = i * 7919 % 20_000;
        assert_eq!(
            state.cell(table as usize, 0, key).unwrap().unwrap().2,
            (key as u32).to_le_bytes()
        );
    }
    assert_eq!(memory.paging()[Owner::CurrentValues as usize], before);
    assert_eq!((before.1, before.2), (0, 0));
    assert!(memory.owner_peaks()[Owner::CurrentValues as usize] <= memory.limit() / 2);
}

#[test]
fn overwritten_payloads_reuse_storage_and_long_reasons_survive_segment_reuse() {
    use core_spool::{Evidence, EvidenceRef, Kind, Native, Record, Value, Writer};
    let root = output();
    fs::create_dir(&root).unwrap();
    let memory = Budget::new(16_000_000);
    let disk = Budget::new(100_000_000);
    let config = Config {
        block_bytes: 4096,
        layout: Layout::Rows,
        compression: Compression::None,
    };
    let table = replay_data::rows::TableId::Countries as u32;
    let reason = vec![0x51; 8193];
    let value = vec![0x77; 257];
    let mut state = consumer::Consumer::create(
        &root.join("state"),
        admission::consumer_allowances(memory.limit() / 2).unwrap(),
        &memory,
        &disk,
    )
    .unwrap();
    let mut high_water = 0;
    for segment in 0..4 {
        let path = root.join(format!("segment-{segment}"));
        let mut writer =
            Writer::create(&path, config, writer_allowances(), &memory, &disk).unwrap();
        for hour in 0..1000 {
            writer
                .append(Record {
                    hour,
                    kind: Kind::Changed,
                    table,
                    field: 0,
                    key: &11_u128.to_le_bytes(),
                    value: Value::Bytes(&value),
                })
                .unwrap();
        }
        writer
            .append(Record {
                hour: 1000,
                kind: Kind::Unknown,
                table,
                field: 1,
                key: &11_u128.to_le_bytes(),
                value: Value::Evidence(Evidence {
                    code: 0,
                    reason: &reason,
                    first: 1000,
                    last: 1000,
                    sample: None,
                    missing: None,
                }),
            })
            .unwrap();
        writer.finish().unwrap();
        state.begin_segment().unwrap();
        if segment != 0 {
            assert_eq!(state.long_reason(table as usize, 1, 0).unwrap(), reason);
        }
        let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
        let mut payload = PayloadReader::open(&path, 31, &memory).unwrap();
        drain(&mut reader, &mut payload, Some(&mut state)).unwrap();
        assert_eq!(state.bytes(table as usize, 0, 0).unwrap(), value);
        let evidence = state.cell(table as usize, 1, 0).unwrap().unwrap();
        let evidence = EvidenceRef::decode(&evidence.2);
        assert_eq!(evidence.code, 0);
        if segment == 1 {
            high_water = state.storage_shape().1;
        }
        if segment > 1 {
            assert_eq!(state.storage_shape().1, high_water);
        }
    }
    assert!(high_water < 40_000);
}

#[test]
fn selection_requires_both_budgets_and_keeps_writer_cpu_at_full_weight() {
    use selection::{Candidate, Range};
    let range = |mean, spread| Range {
        mean,
        min: mean - spread,
        max: mean + spread,
    };
    let candidate = |serial, writer, bytes| Candidate {
        config: Config {
            block_bytes: 4096,
            layout: Layout::Rows,
            compression: Compression::None,
        },
        serial: range(serial, 0.1),
        writer: range(writer, 0.1),
        bytes,
        passes_budgets: selection::passes(range(serial, 0.1), range(writer, 0.1), 150.0),
    };
    let candidates = [
        candidate(20.0, 100.0, 1000),
        candidate(21.0, 94.0, 2000),
        candidate(30.0, 1.0, 1),
        candidate(1.0, 151.0, 1),
    ];
    assert_eq!(selection::choose(&candidates), Some((1, 1)));
    assert!(!selection::passes(range(29.6, 0.2), range(1.0, 0.0), 150.0));
    assert!(!selection::passes(range(1.0, 0.0), range(0.0, 0.1), 150.0));
    assert!(!selection::passes(
        range(29.0, 0.0),
        range(1840.0, 0.0),
        1860.0
    ));
    assert!(selection::choose(&candidates[2..]).is_none());
}

#[test]
fn selection_reports_overlapping_pass_ranges_as_a_tie_and_uses_bytes() {
    use selection::{Candidate, Range, Samples};
    let mut samples = Samples::default();
    for value in [10.0, 12.0, 11.0] {
        samples.push(value);
    }
    let range = samples.range();
    assert_eq!((range.mean, range.min, range.max), (11.0, 10.0, 12.0));
    let candidate = |writer: Range, bytes| Candidate {
        config: Config {
            block_bytes: 4096,
            layout: Layout::Columns,
            compression: Compression::None,
        },
        serial: range,
        writer,
        bytes,
        passes_budgets: true,
    };
    let candidates = [
        candidate(range, 2000),
        candidate(
            Range {
                mean: 12.0,
                min: 11.0,
                max: 13.0,
            },
            1000,
        ),
        candidate(
            Range {
                mean: 30.0,
                min: 29.0,
                max: 31.0,
            },
            1,
        ),
    ];
    assert_eq!(selection::choose(&candidates), Some((1, 2)));
}

#[test]
fn unknown_insert_then_change_in_one_hour_uses_only_bench_side_data() {
    let mut stream = Vec::new();
    for kind in [1, 3] {
        let mut record = event(1, kind, 123);
        record.extend_from_slice(&0_u16.to_le_bytes());
        record.extend_from_slice(&[0, 0, 7]);
        frame(&mut stream, &record);
    }
    let memory = Budget::new(2_000_000);
    let mut kinds = Vec::new();
    let mut side = convert::SourceKinds::new(&mut kinds);
    gate::replay(Cursor::new(&stream), &mut side, &memory).unwrap();
    assert!(side.error.is_none());
    assert_eq!(kinds, [1, 3]);
    assert_eq!(Type::Evidence.width(), 43);
    for layout in [Layout::Rows, Layout::Columns] {
        let path = output();
        let disk = Budget::new(10_000_000);
        let config = Config {
            block_bytes: 464,
            layout,
            compression: Compression::None,
        };
        let mut writer =
            core_spool::Writer::create(&path, config, writer_allowances(), &memory, &disk).unwrap();
        let mut convert = convert::Convert::new(&mut writer);
        gate::replay(Cursor::new(&stream), &mut convert, &memory).unwrap();
        assert!(convert.error.is_none());
        let expected = convert.hash.finish();
        writer.finish().unwrap();
        let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
        let mut payload = PayloadReader::open(&path, 4096, &memory).unwrap();
        let mut hash = convert::Canonical::default();
        let mut kinds = kinds.iter();
        loop {
            match reader.next_block().unwrap() {
                Next::Block(block) => {
                    for hour in block.hours() {
                        for (table, field, record) in hour.ordered() {
                            let source = if record.kind == core_spool::Kind::Unknown {
                                kinds.next().copied()
                            } else {
                                None
                            };
                            hash.spool_record(
                                hour.hour,
                                table,
                                field,
                                record,
                                &mut payload.for_block(block.location),
                                source,
                            )
                            .unwrap();
                        }
                    }
                }
                Next::Finished => break,
                Next::Pending => panic!("sealed"),
            }
        }
        assert_eq!(hash.finish(), expected);
        assert!(kinds.next().is_none());
    }
}
