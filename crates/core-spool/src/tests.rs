use super::*;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn folder() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = std::env::temp_dir().join("hoi4-core-spool-synthetic");
    fs::create_dir_all(&parent).unwrap();
    loop {
        let path = parent.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        if !path.exists() {
            return path;
        }
    }
}
fn config(layout: Layout, compression: Compression) -> Config {
    Config {
        block_bytes: 464,
        layout,
        compression,
    }
}
fn record(hour: i64, kind: Kind, field: u32, value: Value<'_>) -> Record<'_> {
    Record {
        hour,
        kind,
        table: 123,
        field,
        key: b"key",
        value,
    }
}
fn allowances() -> Allowances {
    Allowances {
        dictionary: 262_144,
        reasons: 65_536,
    }
}
fn budgets() -> (Budget, Budget) {
    (Budget::new(2_000_000), Budget::new(100_000_000))
}

#[test]
fn every_layout_preserves_hour_fragments_order_and_native_values() {
    for layout in [Layout::Rows, Layout::Columns] {
        for compression in [Compression::None, Compression::Lz4] {
            let path = folder();
            let (memory, disk) = budgets();
            let config = config(layout, compression);
            let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
            let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
            assert!(matches!(reader.next_block().unwrap(), Next::Pending));
            for i in 0..9 {
                writer
                    .append(record(10, Kind::Changed, 8 - i, Value::I64(-i64::from(i))))
                    .unwrap();
            }
            writer
                .append(record(12, Kind::Changed, 0, Value::OptionalU64(None)))
                .unwrap();
            writer.publish_hour().unwrap();
            let mut order = Vec::new();
            let mut fragments = Vec::new();
            loop {
                match reader.next_block().unwrap() {
                    Next::Block(block) => {
                        for hour in block.hours() {
                            fragments.push((hour.hour, hour.starts, hour.ends));
                            for (_, field, r) in hour.ordered() {
                                order.push((r.order, field, r.kind));
                            }
                            for lane in hour.lanes() {
                                if lane.value_type == Type::I64 {
                                    assert!(lane.typed::<u64>().is_err());
                                    for (order, kind, _, v) in lane.typed::<i64>().unwrap().iter() {
                                        assert_eq!(v, -(order as i64));
                                        assert_eq!(kind, Kind::Changed);
                                    }
                                } else {
                                    assert_eq!(
                                        lane.typed::<Option<u64>>()
                                            .unwrap()
                                            .iter()
                                            .next()
                                            .unwrap()
                                            .3,
                                        None
                                    );
                                }
                            }
                        }
                    }
                    Next::Pending => break,
                    Next::Finished => panic!("writer is still open"),
                }
            }
            assert_eq!(order.len(), 10);
            for (i, (order, field, _)) in order.iter().enumerate() {
                assert_eq!(*order, i as u64);
                assert_eq!(*field, if i < 9 { 8 - i as u32 } else { 0 });
            }
            assert_eq!(fragments, [(10, true, true), (12, true, true)]);
            let cursor = reader.cursor();
            reader.restart(0).unwrap();
            let location = reader.skip_block().unwrap().unwrap();
            assert_eq!((location.first, location.last), (10, 12));
            let block = reader.read_at(location).unwrap();
            assert_eq!(block.records, 10);
            reader.restart(cursor).unwrap();
            writer.finish().unwrap();
            assert!(matches!(reader.next_block().unwrap(), Next::Finished));
            assert!(memory.peak() <= memory.limit());
        }
    }
}

#[test]
fn unknown_refused_and_status_keep_reasons_hours_and_optional_evidence() {
    let path = folder();
    let (memory, disk) = budgets();
    let config = config(Layout::Columns, Compression::Lz4);
    let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
    for kind in [
        Kind::Unknown,
        Kind::Refused,
        Kind::Status,
        Kind::Lost,
        Kind::NotRecorded,
    ] {
        writer
            .append(record(
                30,
                kind,
                7,
                Value::Evidence(Evidence {
                    code: 9,
                    reason: b"named reason",
                    first: 20,
                    last: 30,
                    sample: Some(18),
                    missing: None,
                }),
            ))
            .unwrap();
    }
    writer.finish().unwrap();
    let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
    let mut payload = PayloadReader::open(&path, 3, &memory).unwrap();
    let mut kinds = Vec::new();
    loop {
        match reader.next_block().unwrap() {
            Next::Block(block) => {
                for hour in block.hours() {
                    for lane in hour.lanes() {
                        for (_, kind, _, e) in lane.typed::<EvidenceRef>().unwrap().iter() {
                            assert_eq!(
                                (e.first, e.last, e.sample, e.missing),
                                (20, 30, Some(18), None)
                            );
                            let mut reason = Vec::new();
                            payload
                                .for_block(block.location)
                                .reason(e.reason, |b| {
                                    reason.extend_from_slice(b);
                                    Ok(())
                                })
                                .unwrap();
                            assert_eq!(reason, b"named reason");
                            kinds.push(kind);
                        }
                    }
                }
            }
            Next::Finished => break,
            Next::Pending => panic!("sealed"),
        }
    }
    assert_eq!(
        kinds,
        [
            Kind::Unknown,
            Kind::Refused,
            Kind::Status,
            Kind::Lost,
            Kind::NotRecorded
        ]
    );
    let bad = folder();
    let mut writer = Writer::create(&bad, config, allowances(), &memory, &disk).unwrap();
    assert!(
        writer
            .append(record(0, Kind::Unknown, 0, Value::U32(0)))
            .is_err()
    );
    assert!(writer.finish().is_err());
}

#[test]
fn dictionary_spills_unbounded_keys_and_grows_without_country_or_id_caps() {
    let path = folder();
    let (memory, disk) = budgets();
    let config = Config {
        block_bytes: 16384,
        layout: Layout::Columns,
        compression: Compression::None,
    };
    let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
    let wide = vec![0x91; 20000];
    writer
        .append(Record {
            hour: 0,
            kind: Kind::Appeared,
            table: 7,
            field: 0,
            key: &wide,
            value: Value::U32(42),
        })
        .unwrap();
    for i in 0_u32..20000 {
        writer
            .append(Record {
                hour: 1,
                kind: Kind::Changed,
                table: 7,
                field: 0,
                key: &i.to_le_bytes(),
                value: Value::U32(i),
            })
            .unwrap();
    }
    writer
        .append(Record {
            hour: 2,
            kind: Kind::Changed,
            table: 7,
            field: 0,
            key: &wide,
            value: Value::U32(43),
        })
        .unwrap();
    writer.finish().unwrap();
    let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
    let mut payload = PayloadReader::open(&path, 31, &memory).unwrap();
    let mut wide_ids = Vec::new();
    let mut count = 0;
    loop {
        match reader.next_block().unwrap() {
            Next::Block(block) => {
                for hour in block.hours() {
                    for lane in hour.lanes() {
                        for (_, _, id, v) in lane.typed::<u32>().unwrap().iter() {
                            if hour.hour != 1 {
                                let mut key = Vec::new();
                                payload
                                    .for_block(block.location)
                                    .key(id, |b| {
                                        key.extend_from_slice(b);
                                        Ok(())
                                    })
                                    .unwrap();
                                assert_eq!(key, wide);
                                wide_ids.push(id.index());
                            } else {
                                assert_eq!(v, count);
                                count += 1;
                            }
                        }
                    }
                }
            }
            Next::Finished => break,
            Next::Pending => panic!("sealed"),
        }
    }
    assert_eq!(count, 20000);
    assert_eq!(wide_ids.len(), 2);
    assert_eq!(wide_ids[0], wide_ids[1]);
    assert!(memory.used() <= memory.limit());
}

#[test]
fn payloads_larger_than_blocks_and_pages_are_lossless_and_checked_before_delivery() {
    let path = folder();
    let (memory, disk) = budgets();
    let config = config(Layout::Rows, Compression::None);
    let bytes: Vec<u8> = (0..20000).map(|i| i as u8).collect();
    let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
    writer
        .append(record(100, Kind::Changed, 0, Value::Bytes(&bytes)))
        .unwrap();
    writer.finish().unwrap();
    let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
    let Next::Block(block) = reader.next_block().unwrap() else {
        panic!("block")
    };
    let blob = block
        .hours()
        .next()
        .unwrap()
        .lanes()
        .next()
        .unwrap()
        .typed::<Blob>()
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .3;
    let mut payload = PayloadReader::open(&path, 79, &memory).unwrap();
    let mut actual = Vec::new();
    payload
        .for_block(block.location)
        .blob(blob, |b| {
            actual.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
    assert_eq!(actual, bytes);
    let mut file = OpenOptions::new()
        .write(true)
        .open(path.join("arena"))
        .unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[0xff]).unwrap();
    drop(file);
    let mut damaged = PayloadReader::open(&path, 79, &memory).unwrap();
    let mut calls = 0;
    let error = damaged
        .for_block(block.location)
        .blob(blob, |_| {
            calls += 1;
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("hours 100..=100"));
    assert_eq!(calls, 0);
}

#[test]
fn checksums_name_the_exact_block_hours_and_damage_is_repeatable() {
    for compression in [Compression::None, Compression::Lz4] {
        let path = folder();
        let (memory, disk) = budgets();
        let config = config(Layout::Rows, compression);
        let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
        writer
            .append(record(71, Kind::Changed, 0, Value::U64(999)))
            .unwrap();
        writer
            .append(record(74, Kind::Gone, 0, Value::Unit))
            .unwrap();
        writer.finish().unwrap();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.join("blocks"))
            .unwrap();
        file.seek(SeekFrom::End(-1)).unwrap();
        let mut byte = [0];
        file.read_exact(&mut byte).unwrap();
        file.seek(SeekFrom::End(-1)).unwrap();
        file.write_all(&[byte[0] ^ 0x40]).unwrap();
        drop(file);
        let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
        for _ in 0..2 {
            let error = match reader.next_block() {
                Err(e) => e,
                Ok(_) => panic!("corruption accepted"),
            };
            assert!(error.to_string().contains("hours 71..=74"));
            assert_eq!(reader.cursor(), 0);
        }
    }
}

#[test]
fn all_native_types_and_variable_references_have_their_own_typed_lanes() {
    let path = folder();
    let (memory, disk) = budgets();
    let config = Config {
        block_bytes: 16384,
        layout: Layout::Columns,
        compression: Compression::Lz4,
    };
    let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
    let values = [
        Value::U8(1),
        Value::I8(-2),
        Value::U16(3),
        Value::I16(-4),
        Value::U32(5),
        Value::I32(-6),
        Value::U64(7),
        Value::I64(-8),
        Value::Bool(true),
        Value::OptionalI32(Some(-9)),
        Value::OptionalU32(None),
        Value::OptionalI64(None),
        Value::OptionalU64(Some(10)),
        Value::Text {
            id: 11,
            text: Some(b"text"),
        },
        Value::U32List(&[12, 13]),
        Value::U64List(&[14, 15]),
        Value::Bytes(b"bytes"),
    ];
    for (i, value) in values.into_iter().enumerate() {
        writer
            .append(record(1, Kind::Changed, i as u32, value))
            .unwrap();
    }
    writer.finish().unwrap();
    let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
    let Next::Block(block) = reader.next_block().unwrap() else {
        panic!("block")
    };
    let hour = block.hours().next().unwrap();
    let lanes: Vec<_> = hour.lanes().collect();
    assert_eq!(lanes.len(), 17);
    macro_rules! check {
        ($i:expr,$ty:ty,$expected:expr) => {
            assert_eq!(
                lanes[$i].typed::<$ty>().unwrap().iter().next().unwrap().3,
                $expected
            );
        };
    }
    check!(0, u8, 1);
    check!(1, i8, -2);
    check!(2, u16, 3);
    check!(3, i16, -4);
    check!(4, u32, 5);
    check!(5, i32, -6);
    check!(6, u64, 7);
    check!(7, i64, -8);
    check!(8, bool, true);
    check!(9, Option<i32>, Some(-9));
    check!(10, Option<u32>, None);
    check!(11, Option<i64>, None);
    check!(12, Option<u64>, Some(10));
    assert_eq!(
        lanes[13]
            .typed::<TextRef>()
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .3
            .id,
        11
    );
    let mut payload = PayloadReader::open(&path, 2, &memory).unwrap();
    let mut list = Vec::new();
    let b = lanes[14]
        .typed::<U32ListRef>()
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .3
        .0;
    payload
        .for_block(block.location)
        .blob(b, |b| {
            list.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
    assert_eq!(list, [12_u32.to_le_bytes(), 13_u32.to_le_bytes()].concat());
    assert_eq!(
        lanes[15]
            .typed::<U64ListRef>()
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .3
            .0
            .len,
        16
    );
    assert_eq!(
        lanes[16]
            .typed::<Blob>()
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .3
            .len,
        5
    );
}

#[test]
fn reservations_precede_memory_and_disk_growth() {
    let memory = Budget::new(100);
    let disk = Budget::new(100_000);
    let path = folder();
    assert!(
        Writer::create(
            &path,
            config(Layout::Rows, Compression::None),
            allowances(),
            &memory,
            &disk
        )
        .is_err()
    );
    assert!(!path.exists());
    assert_eq!(memory.used(), 0);
    let memory = Budget::new(2_000_000);
    let disk = Budget::new(100_000);
    let path = folder();
    let mut writer = Writer::create(
        &path,
        config(Layout::Rows, Compression::None),
        allowances(),
        &memory,
        &disk,
    )
    .unwrap();
    disk.grow_disk(disk.limit() - disk.used() - 24).unwrap();
    assert!(
        writer
            .append(record(
                0,
                Kind::Changed,
                0,
                Value::Bytes(b"exceeds disk reservation")
            ))
            .is_err()
    );
    assert_eq!(fs::metadata(path.join("keys")).unwrap().len(), 0);
    assert_eq!(disk.used(), disk.limit() - 24);
}

#[test]
fn empty_spools_seal_and_closed_hours_cannot_be_reopened() {
    let (memory, disk) = budgets();
    let path = folder();
    let config = config(Layout::Rows, Compression::None);
    Writer::create(&path, config, allowances(), &memory, &disk)
        .unwrap()
        .finish()
        .unwrap();
    let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
    assert!(matches!(reader.next_block().unwrap(), Next::Finished));
    let path = folder();
    let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
    writer
        .append(record(1, Kind::Gone, 0, Value::Unit))
        .unwrap();
    writer.publish_hour().unwrap();
    assert!(
        writer
            .append(record(1, Kind::Gone, 0, Value::Unit))
            .is_err()
    );
}

#[test]
fn dense_ids_and_shared_reasons_survive_hourly_publication() {
    let path = folder();
    let (memory, disk) = budgets();
    let config = config(Layout::Columns, Compression::None);
    let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
    let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
    for hour in 0..3 {
        for key in [b"second".as_slice(), b"first".as_slice()] {
            writer
                .append(Record {
                    hour,
                    kind: Kind::Unknown,
                    table: 1,
                    field: 0,
                    key,
                    value: Value::Evidence(Evidence {
                        code: 0,
                        reason: b"shared reason",
                        first: hour,
                        last: hour,
                        sample: None,
                        missing: None,
                    }),
                })
                .unwrap();
        }
        writer.publish_hour().unwrap();
        let Next::Block(block) = reader.next_block().unwrap() else {
            panic!("published hour");
        };
        let ids = block
            .hours()
            .next()
            .unwrap()
            .ordered()
            .map(|(_, _, r)| (r.key.index(), EvidenceRef::decode(r.value).reason))
            .collect::<Vec<_>>();
        assert_eq!(ids, [(0, 0), (1, 0)]);
        assert!(matches!(reader.next_block().unwrap(), Next::Pending));
    }
    writer.finish().unwrap();
    assert!(matches!(reader.next_block().unwrap(), Next::Finished));
    assert_eq!(fs::metadata(path.join("reasons-offsets")).unwrap().len(), 8);
    assert_eq!(fs::metadata(path.join("arena")).unwrap().len(), 0);
}

#[test]
fn block_fills_by_bytes_and_small_reader_buffer_is_a_usage_error() {
    let path = folder();
    let (memory, disk) = budgets();
    let config = config(Layout::Rows, Compression::None);
    let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
    for i in 0..100 {
        writer
            .append(record(1, Kind::Changed, 0, Value::U32(i)))
            .unwrap();
    }
    writer.finish().unwrap();
    let error = Reader::open(&path, config.block_bytes - 1, &memory)
        .err()
        .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("spool block bound 464"));
    let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
    let mut total = 0;
    let mut fragments = Vec::new();
    loop {
        match reader.next_block().unwrap() {
            Next::Block(block) => {
                assert!(block.raw_bytes() <= config.block_bytes);
                if total == 0 {
                    assert_eq!(block.records, 54);
                    assert_eq!(block.raw_bytes(), 464);
                }
                total += block.records;
                for hour in block.hours() {
                    fragments.push((hour.starts, hour.ends));
                }
            }
            Next::Finished => break,
            Next::Pending => panic!("sealed"),
        }
    }
    assert_eq!(total, 100);
    assert_eq!(fragments, [(true, false), (false, true)]);
}

#[test]
fn paged_dirty_eviction_and_sparse_growth_are_lossless_with_fixed_memory() {
    let path = folder();
    fs::create_dir(&path).unwrap();
    let memory = Budget::new(131072);
    let disk = Budget::new(10_000_000);
    let mut pages =
        paged::PagedFile::create(&path.join("pages"), memory.limit(), &memory, &disk).unwrap();
    for i in 0_u64..1000 {
        pages.write(i * 4096, &i.to_le_bytes()).unwrap();
    }
    for i in (0_u64..1000).rev() {
        let mut bytes = [0; 8];
        pages.read(i * 4096, &mut bytes).unwrap();
        assert_eq!(u64::from_le_bytes(bytes), i);
    }
    let mut zeros = [1; 8];
    pages.read(4096 + 16, &mut zeros).unwrap();
    assert_eq!(zeros, [0; 8]);
    pages.flush().unwrap();
    assert!(memory.peak() <= memory.limit());
    assert!(memory.paging()[Owner::Other as usize].2 > 0);
    assert_eq!(
        fs::metadata(path.join("pages")).unwrap().len(),
        999 * 4096 + 8
    );
}

#[test]
fn associative_pages_keep_conflicting_hot_pages_and_evict_the_least_recently_used() {
    let path = folder();
    fs::create_dir(&path).unwrap();
    let memory = Budget::new(13_000).owner(Owner::Dictionary);
    let disk = Budget::new(1_000_000);
    let mut pages =
        paged::PagedFile::create(&path.join("pages"), memory.limit(), &memory, &disk).unwrap();
    for i in [0_u64, 32, 64] {
        pages.write(i * 4096, &i.to_le_bytes()).unwrap();
    }
    let mut value = [0; 8];
    let before = memory.paging();
    for i in [0_u64, 32, 64, 0] {
        pages.read(i * 4096, &mut value).unwrap();
        assert_eq!(u64::from_le_bytes(value), i);
    }
    assert_eq!(memory.paging(), before);
    pages.write(96 * 4096, &96_u64.to_le_bytes()).unwrap();
    let before = memory.paging();
    for i in [0_u64, 64] {
        pages.read(i * 4096, &mut value).unwrap();
    }
    assert_eq!(memory.paging(), before);
    pages.read(32 * 4096, &mut value).unwrap();
    assert_eq!(u64::from_le_bytes(value), 32);
    let after = memory.paging()[Owner::Dictionary as usize];
    assert_eq!(after.0, before[Owner::Dictionary as usize].0 + 1);
    assert_eq!(after.1, 1);
    assert!(after.2 > 0);
    assert!(memory.peak() <= memory.limit());
}

#[test]
fn dictionary_hits_stay_in_memory_beyond_the_old_window() {
    let path = folder();
    fs::create_dir(&path).unwrap();
    let memory = Budget::new(16_000_000).owner(Owner::Dictionary);
    let disk = Budget::new(100_000_000);
    let mut dictionary = Dictionary::create(&path, memory.limit(), &memory, &disk).unwrap();
    for key in 0_u64..50_000 {
        assert_eq!(dictionary.intern(&key.to_le_bytes()).unwrap(), key);
    }
    let before = memory.paging();
    for i in 0_u64..50_000 {
        let key = i * 7919 % 50_000;
        assert_eq!(dictionary.intern(&key.to_le_bytes()).unwrap(), key);
    }
    assert_eq!(memory.paging(), before);
    assert_eq!(before[Owner::Dictionary as usize].1, 0);
    assert_eq!(before[Owner::Dictionary as usize].2, 0);
    assert!(memory.peak() <= memory.limit());
}

#[test]
fn compact_orders_expand_past_sixteen_bits_in_both_layouts() {
    for layout in [Layout::Rows, Layout::Columns] {
        let path = folder();
        let memory = Budget::new(64_000_000);
        let disk = Budget::new(100_000_000);
        let config = Config {
            block_bytes: 524288,
            layout,
            compression: Compression::None,
        };
        let mut writer = Writer::create(&path, config, allowances(), &memory, &disk).unwrap();
        for i in 0..70_000 {
            writer
                .append(record(1, Kind::Changed, i % 2, Value::Unit))
                .unwrap();
        }
        writer.finish().unwrap();
        let mut header = [0; 80];
        fs::File::open(path.join("blocks"))
            .unwrap()
            .read_exact(&mut header)
            .unwrap();
        assert_eq!(header[53], 3);
        let mut reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
        let Next::Block(block) = reader.next_block().unwrap() else {
            panic!("one wide-order block");
        };
        assert_eq!(block.raw_bytes(), 70_000 * 5 + 2 * 32);
        for (i, (_, field, record)) in block.hours().next().unwrap().ordered().enumerate() {
            assert_eq!((record.order, field), (i as u64, i as u32 % 2));
        }
    }
}

#[test]
fn resident_pages_charge_only_when_touched_and_stop_at_the_allowance() {
    let path = folder();
    fs::create_dir(&path).unwrap();
    let memory = Budget::new(1_000_000);
    let disk = Budget::new(1_000_000);
    let allowance = 65_536;
    let mut pages =
        paged::PagedFile::create(&path.join("pages"), allowance, &memory, &disk).unwrap();
    let opened = memory.used();
    assert!(opened < allowance / 8);
    pages.grow(4096 * 100).unwrap();
    assert_eq!(memory.used(), opened);
    pages.write(0, &[7]).unwrap();
    assert_eq!(memory.used(), opened + 4096);
    pages.write(4095, &[9]).unwrap();
    assert_eq!(memory.used(), opened + 4096);
    pages.write(4096, &[11]).unwrap();
    assert_eq!(memory.used(), opened + 8192);
    for page in 2..100 {
        pages.write(page * 4096, &[page as u8]).unwrap();
    }
    assert!(memory.peak() <= allowance);
    assert!(memory.paging()[Owner::Other as usize].2 > 0);
    let mut byte = [0];
    pages.read(0, &mut byte).unwrap();
    assert_eq!(byte, [7]);
    pages.read(4095, &mut byte).unwrap();
    assert_eq!(byte, [9]);
    drop(pages);
    assert_eq!(memory.used(), 0);
}

#[test]
fn eight_writers_and_readers_share_measured_admission_and_spill_independently() {
    let root = folder();
    fs::create_dir(&root).unwrap();
    let memory = Budget::new(8_000_000);
    let disk = Budget::new(100_000_000);
    let allowances = Allowances {
        dictionary: 65_536,
        reasons: 65_536,
    };
    let config = Config {
        block_bytes: 1024,
        layout: Layout::Columns,
        compression: Compression::None,
    };
    let writers = (0..8)
        .map(|i| {
            let path = root.join(i.to_string());
            let writer = Writer::create(&path, config, allowances, &memory, &disk).unwrap();
            let reader = Reader::open(&path, config.block_bytes, &memory).unwrap();
            (path, writer, reader)
        })
        .collect::<Vec<_>>();
    assert!(memory.owner_peaks()[Owner::Dictionary as usize] < 8 * allowances.dictionary / 2);
    std::thread::scope(|scope| {
        for (path, mut writer, mut reader) in writers {
            scope.spawn(move || {
                for key in 0_u64..2000 {
                    writer
                        .append(Record {
                            hour: 0,
                            kind: Kind::Changed,
                            table: 1,
                            field: 0,
                            key: &key.to_le_bytes(),
                            value: Value::U64(key),
                        })
                        .unwrap();
                }
                writer.publish_hour().unwrap();
                let mut count = 0;
                loop {
                    match reader.next_block().unwrap() {
                        Next::Block(block) => {
                            for hour in block.hours() {
                                for (_, _, record) in hour.ordered() {
                                    assert_eq!(u64::decode(record.value), count);
                                    count += 1;
                                }
                            }
                        }
                        Next::Pending => break,
                        Next::Finished => panic!("writer is still open"),
                    }
                }
                assert_eq!(count, 2000);
                writer.finish().unwrap();
                assert_eq!(fs::metadata(path.join("keys-index")).unwrap().len(), 0);
                assert!(matches!(reader.next_block().unwrap(), Next::Finished));
            });
        }
    });
    assert!(memory.peak() <= memory.limit());
    assert!(memory.owner_peaks()[Owner::Dictionary as usize] <= 8 * allowances.dictionary);
    assert!(memory.paging()[Owner::Dictionary as usize].2 > 0);
    assert_eq!(memory.used(), 0);
}

#[test]
fn dictionary_rehash_discards_dead_tables_without_spill_or_final_index_flush() {
    let path = folder();
    fs::create_dir(&path).unwrap();
    let memory = Budget::new(4_000_000).owner(Owner::Dictionary);
    let disk = Budget::new(20_000_000);
    let mut dictionary = Dictionary::create(&path, memory.limit(), &memory, &disk).unwrap();
    for key in 0_u64..8000 {
        assert_eq!(dictionary.intern(&key.to_le_bytes()).unwrap(), key);
    }
    dictionary.flush().unwrap();
    assert_eq!(fs::metadata(path.join("keys-index")).unwrap().len(), 0);
    let keys = fs::metadata(path.join("keys")).unwrap().len();
    let offsets = fs::metadata(path.join("keys-offsets")).unwrap().len();
    assert_eq!(disk.used(), keys + offsets + 16_384 * 16);
    assert_eq!(memory.paging()[Owner::Dictionary as usize].2, 0);
    for key in (0_u64..8000).rev() {
        assert_eq!(dictionary.intern(&key.to_le_bytes()).unwrap(), key);
    }
    assert_eq!(memory.paging()[Owner::Dictionary as usize].1, 0);
}

#[test]
fn flat_resident_prefix_and_paged_overflow_preserve_cells_across_the_boundary() {
    let path = folder();
    fs::create_dir(&path).unwrap();
    let memory = Budget::new(200_000).owner(Owner::CurrentValues);
    let disk = Budget::new(10_000_000);
    let mut flat = paged::FlatFile::create(&path.join("flat"), 65_536, &memory, &disk).unwrap();
    let opened = memory.used();
    flat.grow(1000 * 80).unwrap();
    assert_eq!(memory.used(), opened);
    for row in 0_u64..1000 {
        flat.write(row * 80, &row.to_le_bytes()).unwrap();
    }
    for row in (0_u64..1000).rev() {
        let mut bytes = [0; 8];
        flat.read(row * 80, &mut bytes).unwrap();
        assert_eq!(u64::from_le_bytes(bytes), row);
    }
    assert!(memory.peak() <= 65_536);
    assert!(memory.paging()[Owner::CurrentValues as usize].2 > 0);
}

#[test]
fn lz4_bound_fits_incompressible_and_near_full_blocks() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for len in [0, 1, 15, 464, 4096, 65_535, 65_536, 262_144] {
        let input: Vec<u8> = (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        let bound = crate::writer::compressed_bound(len).unwrap();
        assert!(bound >= lz4_flex::block::get_maximum_output_size(len));
        let mut output = vec![0; bound];
        lz4_flex::block::compress_into(&input, &mut output).unwrap();
    }
}
