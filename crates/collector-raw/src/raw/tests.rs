use super::*;
use engine_native_memory::memory::SliceMemory;

#[test]
fn carried_splice_keeps_order_offsets_aliases_masks_and_bulk_without_read_charges() {
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[17; 16],
    };
    let bytes = [1, 2, 3, 4, 5, 6, 7, 8];
    let spans = [
        Span {
            addr: 0x2000,
            len: 4,
            ok: true,
            mask: 1,
            start: 4,
            bulk: false,
        },
        Span {
            addr: 0x3000,
            len: 8,
            ok: true,
            mask: 2,
            start: 0,
            bulk: true,
        },
        Span {
            addr: 0x2000,
            len: 4,
            ok: true,
            mask: 3,
            start: 0,
            bulk: false,
        },
    ];
    let mut log = RawLog::<2>::new(1, 4);
    assert_eq!(log.take(&memory, 0x1000, 4, 1), Some(0));
    let reads = log.raw_bytes;
    let mut expected = log.spans.clone();
    assert!(log.splice(&spans, &bytes));
    assert_eq!(log.raw_bytes, reads);
    expected.extend(spans.into_iter().map(|span| Span {
        start: 4 + span.start,
        ..span
    }));
    assert_eq!(log.spans, expected);
    assert_eq!(
        log.bytes.as_ref(),
        &[17, 17, 17, 17, 1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert!(log.has_bulk);
    assert!(!log.overflowed);
    assert_eq!(log.fault, [false; 2]);
}

#[test]
fn growth_preserves_raw_prefixes_and_first_fault_without_retry() {
    struct Once {
        reads: std::cell::RefCell<Vec<usize>>,
    }
    impl Memory for Once {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            assert!(
                !self.reads.borrow().contains(&address),
                "native read repeated"
            );
            self.reads.borrow_mut().push(address);
            if address == 0x9000 {
                return false;
            }
            destination.fill(address as u8);
            true
        }
    }
    let memory = Once {
        reads: Default::default(),
    };
    let mut log = RawLog::<1>::new(1, 1);
    for ordinal in 0..5000 {
        assert_eq!(log.take(&memory, 0x1000 + ordinal, 4, 1), Some(ordinal * 4));
    }
    assert_eq!(log.take(&memory, 0x9000, 4, 1), None);
    assert!(!log.overflowed);
    assert!(log.fault[0]);
    let reader = Recorded::build(&log).unwrap();
    for ordinal in 0..5000 {
        let mut bytes = [0; 4];
        assert!(reader.copy(0x1000 + ordinal, &mut bytes));
        assert_eq!(bytes, [(0x1000 + ordinal) as u8; 4]);
    }
    assert!(!reader.copy(0x9000, &mut [0; 4]));
    assert_eq!(memory.reads.borrow().len(), 5001);
}

#[test]
fn growth_prepares_scattered_and_record_storage_before_native_reads() {
    let bytes: Vec<u8> = (0..256).map(|value| value as u8).collect();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::default();
    assert_eq!(
        log.take_scattered(&memory, [0x1000, 0x1004], [4, 4], [1, 1]),
        Some(([0, 4], 2))
    );
    let fields = [engine_native_memory::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 4,
    }];
    let layout = engine_native_memory::memory::RecordLayout::new(&fields, 4).unwrap();
    assert_eq!(
        log.take_records(
            &memory,
            engine_native_memory::memory::RecordSources::Strided {
                base: 0x1008,
                stride: 4,
                count: 32
            },
            &layout,
            1
        ),
        Some((8, 32))
    );
    assert_eq!(
        log.take_array_entries(&memory, 0x1088, 4, 8, 1),
        Some((136, 8))
    );
    assert!(!log.overflowed);
    assert_eq!(&log.bytes[..168], &bytes[..168]);
}

fn immutable_view_positions(reader: &Recorded<'_>) -> (Vec<usize>, usize, usize) {
    (
        reader
            .index
            .slots
            .iter()
            .map(std::cell::Cell::get)
            .collect(),
        reader.next_exact.get(),
        reader.last_bulk.get(),
    )
}

fn immutable_view_log() -> RawLog<1> {
    let mut log = RawLog::new(16, 512);
    log.has_bulk = true;
    let bytes: Vec<u8> = (0..64).collect();
    assert_eq!(
        log.take_bulk(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0x1000,
            64,
            1
        ),
        Some(0)
    );
    log
}

#[test]
fn immutable_view_borrows_current_bytes_without_changing_independent_cursors() {
    let mut log = immutable_view_log();
    for value in [71, 93] {
        assert!(
            log.take(
                &SliceMemory {
                    base: 0x3000,
                    bytes: &[value]
                },
                0x3000,
                1,
                1
            )
            .is_some()
        );
    }
    log.spans.push(Span {
        addr: 0x1010,
        len: 0,
        ok: false,
        mask: 1,
        start: 0,
        bulk: false,
    });
    {
        let first = Recorded::build(&log).unwrap();
        let second = Recorded::build(&log).unwrap();
        let mut outside = [0];
        assert!(first.copy(0x3000, &mut outside));
        assert_eq!(outside, [71]);
        let before = immutable_view_positions(&first);
        let view = first.immutable_view(0x1001, 62).unwrap();
        assert_eq!(view.base, 0x1001);
        assert!(std::ptr::eq(view.bytes.as_ptr(), log.bytes[1..].as_ptr()));
        for (address, len) in [(0x1001, 1), (0x1007, 9), (0x103d, 2)] {
            let mut direct = vec![0; len];
            let mut replay = vec![0; len];
            assert!(view.copy(address, &mut direct));
            assert!(second.copy(address, &mut replay));
            assert_eq!(direct, replay);
        }
        assert_eq!(immutable_view_positions(&first), before);
        assert!(first.immutable_view(0x1000, 64).is_none());
        assert_eq!(immutable_view_positions(&first), before);
        assert!(first.copy(0x3000, &mut outside));
        assert_eq!(outside, [93]);
        assert!(second.copy(0x3000, &mut outside));
        assert_eq!(outside, [71]);
        assert!(second.copy(0x3000, &mut outside));
        assert_eq!(outside, [93]);
        assert!(first.copy(0x3000, &mut outside));
        assert_eq!(outside, [93]);
    }
    log.bytes.extend_from_slice(&[0xa7; 64]);
    let current_start = log.bytes.len() - 64;
    log.spans[0].start = current_start as u32;
    log.spans[0].mask = 0;
    let reader = Recorded::build(&log).unwrap();
    assert_eq!(
        reader.immutable_view(0x1001, 62).unwrap().bytes,
        &[0xa7; 62]
    );
}

#[test]
fn immutable_view_rejects_every_competing_span_without_advancing_replay() {
    for (address, len, ok, bulk) in [
        (0x1008, 4, true, false),
        (0x1008, 4, false, false),
        (0x1000, 64, false, false),
        (0x1000, 64, true, false),
        (0x1008, 16, true, true),
        (0x0ff0, 96, true, true),
        (0x1000, 32, true, true),
        (0x1000, 96, true, true),
        (0x1000, 64, true, true),
        (0x0ff0, 24, true, true),
        (0x1030, 32, true, true),
    ] {
        let mut log = immutable_view_log();
        log.bytes.extend_from_slice(&[0xe1; 96]);
        log.spans.push(Span {
            addr: address,
            len,
            ok,
            mask: 1,
            start: 64,
            bulk,
        });
        let reader = Recorded::build(&log).unwrap();
        let before = immutable_view_positions(&reader);
        assert!(
            reader.immutable_view(0x1004, 56).is_none(),
            "{address:x}/{len}/{ok}/{bulk}"
        );
        assert_eq!(immutable_view_positions(&reader), before);
    }
    let mut log = immutable_view_log();
    for value in [Some(11), None, Some(29)] {
        match value {
            Some(value) => {
                assert!(
                    log.take(
                        &SliceMemory {
                            base: 0x1008,
                            bytes: &[value; 4]
                        },
                        0x1008,
                        4,
                        1
                    )
                    .is_some()
                );
            }
            None => log.note_fail(0x1008, 4, 1),
        }
    }
    let reader = Recorded::build(&log).unwrap();
    for value in [Some(11), None, Some(29), Some(29)] {
        let before = immutable_view_positions(&reader);
        assert!(reader.immutable_view(0x1004, 56).is_none());
        assert_eq!(immutable_view_positions(&reader), before);
        let mut bytes = [0xcc; 4];
        assert_eq!(reader.copy(0x1008, &mut bytes), value.is_some());
        assert_eq!(bytes, [value.unwrap_or(0xcc); 4]);
    }
    assert!(reader.immutable_view(0x1020, 16).is_some());
}

#[test]
fn immutable_view_declines_empty_full_invalid_ranges_and_unpublished_storage() {
    let log = immutable_view_log();
    let reader = Recorded::build(&log).unwrap();
    for (address, len) in [
        (0x1000, 0),
        (0, 1),
        (0x1000, 64),
        (0x0fff, 2),
        (0x103f, 2),
        (usize::MAX - 1, 4),
        (0x0000_8000_0000_0000, 1),
        (0x0000_7fff_ffff_ffff, 2),
        (0x1000, u32::MAX as usize + 1),
    ] {
        let before = immutable_view_positions(&reader);
        assert!(
            reader.immutable_view(address, len).is_none(),
            "{address:x}/{len}"
        );
        assert_eq!(immutable_view_positions(&reader), before);
    }
    drop(reader);
    for mode in 0..6 {
        let mut log = immutable_view_log();
        match mode {
            0 => log.has_bulk = false,
            1 => log.spans[0].ok = false,
            2 => log.spans[0].bulk = false,
            3 => log.spans[0].start = u32::MAX,
            4 => {
                log.bytes.clear();
                log.bytes.extend_from_slice(&[0xb5; 63]);
            }
            5 => log.spans.push(Span {
                addr: usize::MAX,
                len: 2,
                ok: false,
                mask: 1,
                start: 0,
                bulk: false,
            }),
            _ => unreachable!(),
        }
        let reader = Recorded::build(&log).unwrap();
        assert!(reader.immutable_view(0x1001, 62).is_none(), "mode {mode}");
    }
}

#[test]
fn capacity_accounting_follows_raw_growth_clear_and_independent_reader_lifetimes() {
    use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
    static LOCAL: Tracker = Tracker::new();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[7; 32],
    };
    let mut log = RawLog::<1>::new(4, 8);
    log.capacity_charge = Charge::empty_on(&LOCAL);
    log.update_capacity_charge();
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 0);
    drop(Recorded::build(&log).unwrap());
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 0);
    let initial = LOCAL.snapshot().current;
    assert_eq!(log.bytes.storage.len(), log.bytes.capacity());
    assert!(log.bytes.is_empty());
    assert!(log.bytes.storage.iter().all(|&byte| byte == 0));
    assert_eq!(
        initial.payload_vectors,
        vector_bytes(&log.spans) + log.bytes.capacity() as u64
    );
    assert_eq!(log.take(&memory, 0x1000, 16, 1), Some(0));
    let grown = LOCAL.snapshot().current;
    assert_eq!(log.bytes.storage.len(), log.bytes.capacity());
    assert_eq!(log.bytes, [7; 16]);
    assert!(log.bytes.storage[16..].iter().all(|&byte| byte == 0));
    assert!(grown.payload_vectors > initial.payload_vectors);
    assert_eq!(
        grown.payload_vectors,
        vector_bytes(&log.spans) + log.bytes.capacity() as u64
    );
    log.clear();
    assert_eq!(LOCAL.snapshot().current, grown);
    assert_eq!(log.take(&memory, 0x1000, 4, 1), Some(0));
    assert_eq!(log.take(&memory, 0x1000, 4, 1), Some(4));
    assert_eq!(log.take_bulk(&memory, 0x1008, 8, 1), Some(8));

    let first = Recorded::build(&log).unwrap();
    let one_index = LOCAL.snapshot().current;
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
    assert_eq!(
        one_index.index_vectors,
        vector_bytes(&first.index.slots)
            + vector_bytes(&first.index.next)
            + vector_bytes(&first.index.repeated)
            + vector_bytes(&first.index.shape)
            + vector_bytes(&first.index.bulk)
    );
    assert_eq!(one_index.index_map_entries_lower_bound, 0);
    assert_eq!(one_index.payload_vectors, grown.payload_vectors);
    let second = Recorded::build(&log).unwrap();
    let two_indexes = LOCAL.snapshot().current;
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 2);
    assert_eq!(two_indexes.index_vectors, one_index.index_vectors * 2);
    assert_eq!(
        two_indexes.index_map_entries_lower_bound,
        one_index.index_map_entries_lower_bound * 2
    );
    drop(first);
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 2);
    assert_eq!(LOCAL.snapshot().current, two_indexes);
    drop(second);
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
    assert_eq!(LOCAL.snapshot().current, one_index);
    let reused = Recorded::build(&log).unwrap();
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
    assert_eq!(LOCAL.snapshot().current, one_index);
    drop(reused);
    log.clear();
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
    assert_eq!(LOCAL.snapshot().current, one_index);
    drop(Recorded::build(&log).unwrap());
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
    assert_eq!(LOCAL.snapshot().current, one_index);
    let peak = LOCAL.snapshot().peak_total_lower_bound;
    let moved = Some(log);
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
    assert_eq!(LOCAL.snapshot().current, one_index);
    drop(moved);
    assert_eq!(LOCAL.snapshot().allocated_index_owners, 0);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
    assert_eq!(LOCAL.snapshot().peak_total_lower_bound, peak);
}

#[test]
fn initialized_arena_clear_growth_and_equality_expose_only_the_current_prefix() {
    let mut log = RawLog::<1>::new(8, 16);
    let mut other = RawLog::<1>::new(8, 32);
    log.bytes.storage.fill(0xa7);
    other.bytes.storage.fill(0x3e);
    assert!(log.logs_equal(&other));
    assert_eq!(format!("{:?}", log.bytes), "[]");
    assert_eq!(log.bytes.first(), None);
    let initial_pointer = log.bytes.as_ptr();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[9, 8, 7, 6],
    };
    assert_eq!(log.take(&memory, 0x1000, 4, 1), Some(0));
    assert_eq!(other.take(&memory, 0x1000, 4, 1), Some(0));
    assert!(log.logs_equal(&other));
    assert_eq!(format!("{:?}", log.bytes), "[9, 8, 7, 6]");
    assert!(log.bytes.get(4).is_none());
    log.clear();
    assert_eq!(log.bytes.as_ptr(), initial_pointer);
    assert!(log.bytes.is_empty());
    assert_eq!(&log.bytes.storage[..4], &[9, 8, 7, 6]);
    assert!(!Recorded::build(&log).unwrap().copy(0x1000, &mut [0; 4]));

    log.bytes.extend_from_slice(&[1, 2, 3]);
    let capacity = log.bytes.capacity();
    let old_storage = log.bytes.storage.clone();
    log.required_bytes = capacity + 1;
    log.overflowed = true;
    log.grow();
    assert!(log.bytes.capacity() > capacity);
    assert_eq!(log.bytes.storage.len(), log.bytes.capacity());
    assert_eq!(&log.bytes.storage[..capacity], old_storage);
    assert!(log.bytes.storage[capacity..].iter().all(|&byte| byte == 0));
    assert_eq!(log.bytes, [1, 2, 3]);
    assert!(log.bytes.get(3).is_none());
    log.clear();
    assert!(log.bytes.is_empty());
    assert!(log.bytes.storage.iter().any(|&byte| byte != 0));
    assert_eq!(log.take_bulk(&memory, 0x1000, 4, 1), Some(0));
    assert_eq!(log.bytes, [9, 8, 7, 6]);
}

#[test]
fn initialized_arena_gathers_hide_dirty_failed_fields_and_unread_suffixes() {
    struct PartialSource {
        fail: usize,
        partial: usize,
        calls: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for PartialSource {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            let complete = address != self.fail;
            let len = if complete {
                destination.len()
            } else {
                self.partial.min(destination.len())
            };
            destination[..len].fill((address / 8) as u8);
            complete
        }
    }
    for completed in 0..=4 {
        for partial in [0, 3, 7] {
            let mut log = RawLog::<2>::new(8, 64);
            log.bytes.storage.fill(0xda);
            log.bytes.extend_from_slice(&[0xb3, 0x91, 0x7e]);
            let source = PartialSource {
                fail: 0x1000 + completed * 8,
                partial,
                calls: RefCell::default(),
            };
            assert_eq!(
                log.take_array_chunk(&source, 0x1000, 8, 4, 2),
                Some((3, completed))
            );
            let mut expected = vec![0xb3, 0x91, 0x7e];
            for index in 0..completed {
                expected.extend_from_slice(&[(0x1000 / 8 + index) as u8; 8]);
            }
            assert_eq!(log.bytes, expected);
            assert!(log.bytes.get(expected.len()).is_none());
            assert_eq!(source.calls.borrow().len(), (completed + 1).min(4));
            assert_eq!(log.raw_bytes, [0, (completed * 8) as u64]);
            assert_eq!(log.fault, [false, completed != 4]);
            if completed != 4 {
                let reader = Recorded::build(&log).unwrap();
                let mut failed = [0xe7; 8];
                assert!(!reader.copy(source.fail, &mut failed));
                assert_eq!(failed, [0xe7; 8]);
            }
            let current = SliceMemory {
                base: 0x2000,
                bytes: &[31; 4],
            };
            assert_eq!(log.take(&current, 0x2000, 4, 1), Some(expected.len()));
            expected.extend_from_slice(&[31; 4]);
            assert_eq!(log.bytes, expected);
        }
    }
    for partial in [0, 2, 3] {
        let mut log = RawLog::<2>::new(8, 32);
        log.bytes.storage.fill(0xe3);
        log.bytes.extend_from_slice(&[0xb3, 0x91, 0x7e]);
        let source = PartialSource {
            fail: 0x1008,
            partial,
            calls: RefCell::default(),
        };
        assert_eq!(
            log.take_scattered(
                &source,
                [0x1000, usize::MAX, 0x1008, 0x1010],
                [8, 0, 4, 4],
                [1, 1, 2, 1],
            ),
            Some(([3, 11, 11, 15], 2))
        );
        assert_eq!(*source.calls.borrow(), vec![(0x1000, 8), (0x1008, 4)]);
        assert_eq!(log.bytes, [0xb3, 0x91, 0x7e, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(log.bytes.get(11).is_none());
        assert_eq!(log.raw_bytes, [8, 0]);
        assert_eq!(log.fault, [false, true]);
        let reader = Recorded::build(&log).unwrap();
        assert!(!reader.copy(0x1008, &mut [0; 4]));
        assert!(!reader.copy(0x1010, &mut [0; 4]));
    }
}

fn raw_specialization_frozen_scattered<const M: usize>(
    log: &mut RawLog<3>,
    memory: &impl Memory,
    addresses: [usize; M],
    lengths: [usize; M],
    masks: [u32; M],
) -> Option<([usize; M], usize)> {
    if log.overflowed {
        return None;
    }
    let start = log.bytes.len();
    let mut end = start;
    let mut starts = [0; M];
    for (index, len) in lengths.into_iter().enumerate() {
        starts[index] = end;
        if len > u32::MAX as usize {
            log.refuse_torn(masks[index]);
            return None;
        }
        let Some(next) = end.checked_add(len) else {
            log.overflowed = true;
            return None;
        };
        end = next;
    }
    let spans = lengths.iter().filter(|&&len| len != 0).count();
    if !log.ensure_storage(spans, end) {
        return None;
    }
    let completed = {
        let mut remaining = &mut log.bytes.storage[start..end];
        let mut destinations: [&mut [u8]; M] = std::array::from_fn(|index| {
            let (field, rest) = std::mem::take(&mut remaining).split_at_mut(lengths[index]);
            remaining = rest;
            field
        });
        let guarded = std::array::from_fn(|index| {
            if is_canonical_range(addresses[index], lengths[index]) {
                addresses[index]
            } else {
                0
            }
        });
        memory.copy_scattered(guarded, &mut destinations)
    };
    debug_assert!(completed <= M);
    log.bytes.used = if completed == M {
        end
    } else {
        starts[completed]
    };
    for index in 0..completed {
        let len = lengths[index];
        if len != 0 {
            log.spans.push(Span {
                addr: addresses[index],
                len: len as u32,
                ok: true,
                mask: masks[index],
                start: starts[index] as u32,
                bulk: false,
            });
            log.raw_bytes[lowest_table(masks[index])] += len as u64;
        }
    }
    if completed != M {
        log.note_fail(
            addresses[completed],
            lengths[completed] as u32,
            masks[completed],
        );
    }
    Some((starts, completed))
}

fn raw_specialization_frozen_array(
    log: &mut RawLog<3>,
    memory: &impl Memory,
    address: usize,
    width: usize,
    count: usize,
    mask: u32,
) -> Option<(usize, usize)> {
    if log.overflowed || count > 64 || width == 0 || width > u32::MAX as usize {
        return None;
    }
    let start = log.bytes.len();
    if count == 0 {
        return Some((start, 0));
    }
    let size = count.checked_mul(width)?;
    let end = start.checked_add(size)?;
    if !log.ensure_storage(2, end) {
        return None;
    }
    let addresses: [usize; 64] =
        std::array::from_fn(|index| address.checked_add(index * width).unwrap_or(0));
    let guarded = addresses.map(|address| {
        if is_canonical_range(address, width) {
            address
        } else {
            0
        }
    });
    let fields = [engine_native_memory::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: width,
    }];
    let completed = memory.copy_records(
        engine_native_memory::memory::RecordSources::Pointers(&guarded[..count]),
        &engine_native_memory::memory::RecordLayout::new(&fields, width).unwrap(),
        &mut log.bytes.storage[start..end],
    );
    debug_assert!(completed <= count);
    log.bytes.used = start + completed * width;
    if completed != 0 {
        log.spans.push(Span {
            addr: address,
            len: (completed * width) as u32,
            ok: true,
            mask,
            start: start as u32,
            bulk: true,
        });
        log.has_bulk = true;
        log.raw_bytes[lowest_table(mask)] += (completed * width) as u64;
    }
    if completed != count {
        log.note_fail(addresses[completed], width as u32, mask);
    }
    Some((start, completed))
}

#[derive(Default)]
struct RawSpecializationSource {
    fail_call: Option<usize>,
    calls: RefCell<Vec<(usize, usize)>>,
}

impl Memory for RawSpecializationSource {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let call = self.calls.borrow().len();
        self.calls.borrow_mut().push((address, destination.len()));
        let ok = self.fail_call != Some(call);
        let copied = if ok {
            destination.len()
        } else {
            destination.len() / 2
        };
        for (offset, byte) in destination[..copied].iter_mut().enumerate() {
            *byte = (address as u8)
                .wrapping_add((call as u8).wrapping_mul(17))
                .wrapping_add(offset as u8);
        }
        ok
    }
}

fn raw_specialization_log(span_capacity: usize, byte_capacity: usize) -> RawLog<3> {
    let mut log = RawLog::new(span_capacity, byte_capacity);
    log.bytes.storage.fill(0xd7);
    if byte_capacity >= 3 {
        log.bytes.extend_from_slice(&[0x91, 0x72, 0x53]);
    }
    log
}

fn assert_raw_specialization_equal(actual: &RawLog<3>, expected: &RawLog<3>) {
    assert_eq!(actual.spans, expected.spans);
    assert_eq!(actual.bytes, expected.bytes);
    assert_eq!(actual.bytes.storage, expected.bytes.storage);
    assert_eq!(actual.fault, expected.fault);
    assert_eq!(actual.raw_bytes, expected.raw_bytes);
    assert_eq!(actual.has_bulk, expected.has_bulk);
    assert_eq!(actual.overflowed, expected.overflowed);
    assert_eq!(actual.refusal(), expected.refusal());
    assert_eq!(actual.span_exhausted, expected.span_exhausted);
    assert_eq!(actual.required_bytes, expected.required_bytes);
    assert_eq!(actual.spans.capacity(), expected.spans.capacity());
    assert_eq!(actual.bytes.capacity(), expected.bytes.capacity());
    assert_eq!(actual.grow_for_next(), expected.grow_for_next());
}

#[test]
fn raw_specialization_array_prefix_matches_frozen_reads_and_faults() {
    for count in [0, 1, 63, 64] {
        for address in [
            0x1000,
            0,
            0x0000_7fff_ffff_fff0,
            0x0000_8000_0000_0000,
            usize::MAX - 15,
            usize::MAX - 3,
        ] {
            for fail in 0..=count {
                let fail_call = (fail < count).then_some(fail);
                let actual_source = RawSpecializationSource {
                    fail_call,
                    ..Default::default()
                };
                let expected_source = RawSpecializationSource {
                    fail_call,
                    ..Default::default()
                };
                let mut actual = raw_specialization_log(8, 1024);
                let mut expected = raw_specialization_log(8, 1024);
                assert_eq!(
                    actual.take_array_chunk(&actual_source, address, 8, count, 6),
                    raw_specialization_frozen_array(
                        &mut expected,
                        &expected_source,
                        address,
                        8,
                        count,
                        6,
                    ),
                    "count={count}, address={address:#x}, fail={fail}"
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert_eq!(
                    *actual_source.calls.borrow(),
                    *expected_source.calls.borrow()
                );
                assert!(actual_source.calls.borrow().len() <= count);
                if count == 0 {
                    assert!(actual_source.calls.borrow().is_empty());
                    assert!(actual.spans.is_empty());
                }
                let actual_resume = RawSpecializationSource::default();
                let expected_resume = RawSpecializationSource::default();
                assert_eq!(
                    actual.take(&actual_resume, 0x4000, 4, 1),
                    expected.take(&expected_resume, 0x4000, 4, 1)
                );
                assert_raw_specialization_equal(&actual, &expected);
            }
        }
    }
}

#[test]
fn raw_specialization_array_admission_and_growth_match_frozen() {
    for count in [0, 1, 63, 64, 65] {
        for (span_capacity, byte_capacity) in [
            (0, 0),
            (1, 1024),
            (2, 0),
            (2, 3),
            (2, 10),
            (2, 11),
            (2, 514),
            (2, 515),
        ] {
            let mut actual = raw_specialization_log(span_capacity, byte_capacity);
            let mut expected = raw_specialization_log(span_capacity, byte_capacity);
            let actual_source = RawSpecializationSource::default();
            let expected_source = RawSpecializationSource::default();
            assert_eq!(
                actual.take_array_chunk(&actual_source, 0x1000, 8, count, 1),
                raw_specialization_frozen_array(
                    &mut expected,
                    &expected_source,
                    0x1000,
                    8,
                    count,
                    1,
                )
            );
            assert_raw_specialization_equal(&actual, &expected);
            assert_eq!(
                *actual_source.calls.borrow(),
                *expected_source.calls.borrow()
            );
            assert!(!actual.overflowed);
        }
    }
    for width in [0, u32::MAX as usize, u32::MAX as usize + 1] {
        for count in [0, 1, 64] {
            let mut actual = raw_specialization_log(2, 16);
            let mut expected = raw_specialization_log(2, 16);
            let actual_source = RawSpecializationSource::default();
            let expected_source = RawSpecializationSource::default();
            assert_eq!(
                actual.take_array_chunk(&actual_source, usize::MAX - 3, width, count, 1),
                raw_specialization_frozen_array(
                    &mut expected,
                    &expected_source,
                    usize::MAX - 3,
                    width,
                    count,
                    1,
                )
            );
            assert_raw_specialization_equal(&actual, &expected);
            assert!(actual_source.calls.borrow().is_empty());
            assert!(expected_source.calls.borrow().is_empty());
        }
    }
}

#[test]
fn array_entries_record_the_per_entry_takes_log() {
    for count in [0, 1, 2, 63, 64] {
        for address in [
            0x1000,
            0,
            0x0000_7fff_ffff_fff0,
            0x0000_8000_0000_0000,
            usize::MAX - 15,
            usize::MAX - 3,
        ] {
            for fail in 0..=count {
                let fail_call = (fail < count).then_some(fail);
                let actual_source = RawSpecializationSource {
                    fail_call,
                    ..Default::default()
                };
                let expected_source = RawSpecializationSource {
                    fail_call,
                    ..Default::default()
                };
                let mut actual = raw_specialization_log(80, 1024);
                let mut expected = raw_specialization_log(80, 1024);
                let start = expected.bytes.len();
                let completed = (0..count)
                    .take_while(|&index| {
                        expected
                            .take(&expected_source, address.wrapping_add(index * 8), 8, 6)
                            .is_some()
                    })
                    .count();
                assert_eq!(
                    actual.take_array_entries(&actual_source, address, 8, count, 6),
                    Some((start, completed)),
                    "count={count}, address={address:#x}, fail={fail}"
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert!(!actual.has_bulk);
                assert_eq!(
                    *actual_source.calls.borrow(),
                    *expected_source.calls.borrow()
                );
            }
        }
    }
}

#[test]
fn array_entries_grow_every_span_before_copying() {
    let mut log = raw_specialization_log(64, 1024);
    let source = RawSpecializationSource::default();
    assert_eq!(
        log.take_array_entries(&source, 0x1000, 8, 64, 1),
        Some((3, 64))
    );
    assert!(!log.overflowed && log.spans.capacity() > 64);
    assert_eq!(log.spans.len(), 64);
}

#[test]
fn raw_specialization_scattered_split_matches_frozen_mixed_and_empty_fields() {
    let lengths = [0, 1, 4, 0, 8, 1, 32, 4, 0];
    let masks = [1, 1, 2, 4, 3, 1, 2, 4, 1];
    let original = [
        usize::MAX,
        0x1000,
        0x1004,
        0,
        0x1010,
        0x1000,
        0x1020,
        0x1004,
        usize::MAX,
    ];
    for replaced in 0..=lengths.len() {
        for replacement in [0, 0x0000_8000_0000_0000, usize::MAX - 3] {
            let mut addresses = original;
            if replaced < addresses.len() {
                addresses[replaced] = replacement;
            }
            for fail in 0..=6 {
                let fail_call = (fail < 6).then_some(fail);
                let actual_source = RawSpecializationSource {
                    fail_call,
                    ..Default::default()
                };
                let expected_source = RawSpecializationSource {
                    fail_call,
                    ..Default::default()
                };
                let mut actual = raw_specialization_log(16, 128);
                let mut expected = raw_specialization_log(16, 128);
                assert_eq!(
                    actual.take_scattered(&actual_source, addresses, lengths, masks),
                    raw_specialization_frozen_scattered(
                        &mut expected,
                        &expected_source,
                        addresses,
                        lengths,
                        masks,
                    )
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert_eq!(
                    *actual_source.calls.borrow(),
                    *expected_source.calls.borrow()
                );
            }
        }
    }
    for span_capacity in 0..=9 {
        for byte_capacity in [0, 3, 4, 8, 16, 52, 53, 128] {
            let mut actual = raw_specialization_log(span_capacity, byte_capacity);
            let mut expected = raw_specialization_log(span_capacity, byte_capacity);
            let actual_source = RawSpecializationSource::default();
            let expected_source = RawSpecializationSource::default();
            assert_eq!(
                actual.take_scattered(&actual_source, original, lengths, masks),
                raw_specialization_frozen_scattered(
                    &mut expected,
                    &expected_source,
                    original,
                    lengths,
                    masks,
                )
            );
            assert_raw_specialization_equal(&actual, &expected);
            assert_eq!(
                *actual_source.calls.borrow(),
                *expected_source.calls.borrow()
            );
        }
    }
    let mut actual = raw_specialization_log(0, 0);
    let mut expected = raw_specialization_log(0, 0);
    let actual_source = RawSpecializationSource::default();
    let expected_source = RawSpecializationSource::default();
    assert_eq!(
        actual.take_scattered(&actual_source, [usize::MAX; 9], [0; 9], [1; 9]),
        raw_specialization_frozen_scattered(
            &mut expected,
            &expected_source,
            [usize::MAX; 9],
            [0; 9],
            [1; 9],
        )
    );
    assert_raw_specialization_equal(&actual, &expected);
    assert_eq!(
        actual.take_scattered(&actual_source, [], [], []),
        raw_specialization_frozen_scattered(&mut expected, &expected_source, [], [], [])
    );
    assert_raw_specialization_equal(&actual, &expected);
    assert!(actual_source.calls.borrow().is_empty());
    assert!(expected_source.calls.borrow().is_empty());
}

#[test]
fn prefetched_scattered_matches_frozen_faults_masks_duplicates_and_dirty_suffixes() {
    let lengths = [0, 8, 12, 8, 1, 32, 0, 8];
    let masks = [0, 3, 2, 4, 0, 5, 1, 2];
    let original = [
        usize::MAX,
        0x1000,
        0x1010,
        0x1020,
        0x1030,
        0x1040,
        0,
        0x1000,
    ];
    for replaced in 0..=original.len() {
        for replacement in [0, 0x0000_8000_0000_0000, usize::MAX - 3] {
            let mut addresses = original;
            if replaced < addresses.len() {
                addresses[replaced] = replacement;
            }
            for fail in 0..=6 {
                let fail_call = (fail < 6).then_some(fail);
                let source = RawSpecializationSource {
                    fail_call,
                    ..Default::default()
                };
                let reference = RawSpecializationSource {
                    fail_call,
                    ..Default::default()
                };
                let mut actual = raw_specialization_log(16, 128);
                let mut expected = raw_specialization_log(16, 128);
                assert_eq!(
                    actual.take_scattered_prefetched(&source, addresses, lengths, masks),
                    raw_specialization_frozen_scattered(
                        &mut expected,
                        &reference,
                        addresses,
                        lengths,
                        masks,
                    )
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
                assert!(actual.bytes.get(actual.bytes.len()).is_none());
                let resumed = RawSpecializationSource::default();
                let resumed_reference = RawSpecializationSource::default();
                assert_eq!(
                    actual.take(&resumed, 0x2000, 4, 1),
                    expected.take(&resumed_reference, 0x2000, 4, 1)
                );
                assert_raw_specialization_equal(&actual, &expected);
            }
        }
    }
}

#[test]
fn prefetched_scattered_preserves_capacity_empty_and_length_admission() {
    for span_capacity in 0..=6 {
        for byte_capacity in 0..=65 {
            let mut actual = raw_specialization_log(span_capacity, byte_capacity);
            let mut expected = raw_specialization_log(span_capacity, byte_capacity);
            let source = RawSpecializationSource::default();
            let reference = RawSpecializationSource::default();
            assert_eq!(
                actual.take_scattered_prefetched(
                    &source,
                    [0x1000, 0x1010, 0x1020, 0x1030, 0x1040],
                    [8, 12, 8, 1, 32],
                    [1; 5],
                ),
                expected.take_scattered(
                    &reference,
                    [0x1000, 0x1010, 0x1020, 0x1030, 0x1040],
                    [8, 12, 8, 1, 32],
                    [1; 5],
                )
            );
            assert_raw_specialization_equal(&actual, &expected);
            assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
        }
    }
    for lengths in [
        [0, 0],
        [1, u32::MAX as usize],
        [u32::MAX as usize + 1, 0],
        [8, usize::MAX],
    ] {
        for overflowed in [false, true] {
            let mut actual = raw_specialization_log(2, 16);
            let mut expected = raw_specialization_log(2, 16);
            actual.overflowed = overflowed;
            expected.overflowed = overflowed;
            let source = RawSpecializationSource::default();
            assert_eq!(
                actual.take_scattered_prefetched(&source, [usize::MAX; 2], lengths, [0, 3]),
                expected.take_scattered(&source, [usize::MAX; 2], lengths, [0, 3])
            );
            assert_raw_specialization_equal(&actual, &expected);
            assert!(source.calls.borrow().is_empty());
        }
    }
    let mut empty = RawLog::<1>::default();
    let source = RawSpecializationSource::default();
    assert_eq!(
        empty.take_scattered_prefetched(&source, [], [], []),
        Some(([], 0))
    );
    assert_eq!(
        empty.take_scattered_prefetched(&source, [usize::MAX; 2], [0; 2], [1; 2]),
        Some(([0; 2], 2))
    );
    assert!(empty.spans.is_empty());
    assert!(empty.bytes.is_empty());
    assert!(!empty.overflowed);
    assert!(source.calls.borrow().is_empty());
}

#[test]
fn prefetched_scattered_selects_only_the_requested_provider_entry() {
    #[derive(Default)]
    struct Provider {
        ordinary: std::cell::Cell<usize>,
        prefetched: std::cell::Cell<usize>,
    }
    impl Memory for Provider {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("raw scatter must use the provider's grouped operation")
        }
        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.ordinary.set(self.ordinary.get() + 1);
            SliceMemory {
                base: 0x1000,
                bytes: &[7; 8],
            }
            .copy_scattered(addresses, destinations)
        }
        fn copy_scattered_prefetched<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.prefetched.set(self.prefetched.get() + 1);
            SliceMemory {
                base: 0x1000,
                bytes: &[7; 8],
            }
            .copy_scattered(addresses, destinations)
        }
    }
    let provider = Provider::default();
    let mut log = RawLog::<1>::new(4, 16);
    assert_eq!(
        log.take_scattered(&provider, [0x1000, 0x1004], [4; 2], [1; 2]),
        Some(([0, 4], 2))
    );
    assert_eq!((provider.ordinary.get(), provider.prefetched.get()), (1, 0));
    assert_eq!(
        log.take_scattered_prefetched(&provider, [0x1000, 0x1004], [4; 2], [1; 2]),
        Some(([8, 12], 2))
    );
    assert_eq!((provider.ordinary.get(), provider.prefetched.get()), (1, 1));
    assert_eq!(log.raw_bytes, [16]);
}

#[test]
fn lowest_table_preserves_zero_single_bits_and_mixed_mask_anchors() {
    assert_eq!(lowest_table(0), 0);
    for bit in 0..32 {
        assert_eq!(lowest_table(1u32 << bit), bit as usize);
        for other in 0..32 {
            assert_eq!(
                lowest_table((1u32 << bit) | (1u32 << other)),
                bit.min(other) as usize
            );
        }
    }
    for (mask, expected) in [
        (u32::MAX, 0),
        (0x8000_003f, 0),
        (0xffff_ff00, 8),
        (0xaaaa_aaa0, 5),
        (0x5555_5500, 8),
        (0xf000_0000, 28),
    ] {
        assert_eq!(lowest_table(mask), expected);
    }
}

#[test]
fn array_chunk_coalesces_exact_success_prefix_without_retry() {
    struct Source<'a> {
        memory: SliceMemory<'a>,
        calls: std::cell::Cell<usize>,
    }
    impl Memory for Source<'_> {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("expected one record gather")
        }
        fn copy_records(
            &self,
            sources: engine_native_memory::memory::RecordSources<'_>,
            layout: &engine_native_memory::memory::RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.calls.set(self.calls.get() + 1);
            self.memory.copy_records(sources, layout, destination)
        }
    }
    for readable in 0..=4 {
        let bytes: Vec<u8> = (0..32).collect();
        let memory = Source {
            memory: SliceMemory {
                base: 0x1000,
                bytes: &bytes[..readable * 8],
            },
            calls: std::cell::Cell::new(0),
        };
        let mut log = RawLog::<2>::new(2, 32);
        assert_eq!(
            log.take_array_chunk(&memory, 0x1000, 8, 4, 2),
            Some((0, readable))
        );
        assert_eq!(memory.calls.get(), 1);
        assert_eq!(log.bytes, bytes[..readable * 8]);
        assert_eq!(log.raw_bytes, [0, (readable * 8) as u64]);
        assert_eq!(log.fault, [false, readable != 4]);
        assert_eq!(
            log.spans.len(),
            usize::from(readable != 0) + usize::from(readable != 4)
        );
        let recorded = Recorded::build(&log).unwrap();
        for index in 0..4 {
            let mut field = [0; 8];
            assert_eq!(
                recorded.copy(0x1000 + index * 8, &mut field),
                index < readable
            );
            if index < readable {
                assert_eq!(field, bytes[index * 8..index * 8 + 8]);
            }
        }
    }
}

#[test]
fn array_chunk_grows_short_storage_before_reading_and_empty_chunk_succeeds() {
    let mut log = RawLog::<1>::new(2, 8);
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[3; 16],
    };
    assert_eq!(log.take_array_chunk(&memory, 0, 8, 0, 1), Some((0, 0)));
    assert_eq!(log.take_array_chunk(&memory, 0x1000, 8, 2, 1), Some((0, 2)));
    assert!(!log.overflowed);
    assert_eq!(log.spans.len(), 1);
    assert_eq!(log.bytes, [3; 16]);
}

#[test]
fn raw_growth_stops_when_offsets_cannot_represent_the_required_arena() {
    let mut log = RawLog::<1> {
        overflowed: true,
        required_bytes: u32::MAX as usize,
        ..Default::default()
    };
    assert!(log.grow_for_next());
    log.required_bytes += 1;
    assert!(!log.grow_for_next());
}

#[test]
fn scattered_copy_uses_one_gather_and_keeps_failed_field_masks() {
    struct Scatter<'a> {
        memory: SliceMemory<'a>,
        gathers: std::cell::Cell<usize>,
    }
    impl Memory for Scatter<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.memory.copy(address, destination)
        }
        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.gathers.set(self.gathers.get() + 1);
            self.memory.copy_scattered(addresses, destinations)
        }
    }
    let memory = Scatter {
        memory: SliceMemory {
            base: 0x1000,
            bytes: &[31; 8],
        },
        gathers: std::cell::Cell::new(0),
    };
    let mut log = RawLog::<2>::new(4, 32);
    assert_eq!(
        log.take_scattered(
            &memory,
            [0x1000, 0, 0x1008, 0x1000],
            [8, 0, 4, 4],
            [1, 1, 2, 1]
        ),
        Some(([0, 8, 8, 12], 2))
    );
    assert_eq!(memory.gathers.get(), 1);
    assert_eq!(log.spans.len(), 2);
    assert_eq!(log.bytes.len(), 8);
    assert_eq!(log.fault, [false, true]);
    assert_eq!(log.raw_bytes, [8, 0]);
    assert_eq!(log.spans[1].mask, 2);
    assert!(!log.spans[1].ok);
    let recorded = Recorded::build(&log).unwrap();
    assert!(recorded.copy(0x1000, &mut [0; 8]));
    assert!(!recorded.copy(0x1008, &mut [0; 4]));
    assert!(!recorded.copy(0x1000, &mut [0; 4]));
}

#[test]
fn scattered_copy_grows_short_storage_before_its_gather() {
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[7; 8],
    };
    let mut log = RawLog::<1>::new(1, 4);
    assert_eq!(
        log.take_scattered(&memory, [0x1000, 0x1004], [4, 4], [1, 1]),
        Some(([0, 4], 2))
    );
    assert!(!log.overflowed);
    assert_eq!(log.spans.len(), 2);
    assert_eq!(log.bytes, [7; 8]);
}

#[test]
fn reused_index_reads_only_current_bytes_and_resets_duplicate_cursors() {
    let mut log = RawLog::<1>::new(4, 32);
    for (first, second) in [(7, 11), (23, 47)] {
        log.clear();
        for byte in [first, second] {
            let memory = SliceMemory {
                base: 0x1000,
                bytes: &[byte; 8],
            };
            log.take(&memory, 0x1000, 8, 1).unwrap();
        }
        let recorded = Recorded::build(&log).unwrap();
        let mut bytes = [0; 8];
        assert!(recorded.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [first; 8]);
        assert!(recorded.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [second; 8]);
        assert!(recorded.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [second; 8]);
    }
    assert!(log.index.borrow().is_some());
}

#[test]
fn cached_index_rebuilds_when_address_or_failure_changes() {
    let mut log = RawLog::<1>::new(4, 32);
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[9; 16],
    };
    log.take(&memory, 0x1000, 8, 1).unwrap();
    assert!(Recorded::build(&log).unwrap().copy(0x1000, &mut [0; 8]));
    log.clear();
    log.take(&memory, 0x1008, 8, 1).unwrap();
    {
        let recorded = Recorded::build(&log).unwrap();
        assert!(!recorded.copy(0x1000, &mut [0; 8]));
        assert!(recorded.copy(0x1008, &mut [0; 8]));
    }
    log.clear();
    assert!(log.take(&FailAll, 0x1008, 8, 1).is_none());
    assert!(!Recorded::build(&log).unwrap().copy(0x1008, &mut [0; 8]));
}

#[test]
fn cached_bulk_index_uses_current_pool_offsets_and_bulk_gate() {
    let mut log = RawLog::<1>::new(4, 32);
    log.has_bulk = true;
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[9; 16],
    };
    log.take_bulk(&memory, 0x1000, 16, 1).unwrap();
    assert!(Recorded::build(&log).unwrap().copy(0x1004, &mut [0; 8]));
    let original = log.bytes.to_vec();
    log.bytes.clear();
    log.bytes.extend_from_slice(&[71; 8]);
    log.bytes.extend_from_slice(&original);
    log.spans[0].start += 8;
    let mut bytes = [0; 8];
    assert!(Recorded::build(&log).unwrap().copy(0x1004, &mut bytes));
    assert_eq!(bytes, [9; 8]);
    log.has_bulk = false;
    assert!(!Recorded::build(&log).unwrap().copy(0x1004, &mut bytes));
    {
        let range = Recorded::build_range(&log, 0..1).unwrap();
        assert!(!range.copy(0x1004, &mut bytes));
        assert!(range.immutable_view(0x1004, bytes.len()).is_none());
        assert!(range.copy(0x1000, &mut [0; 16]));
    }
    log.has_bulk = true;
    let range = Recorded::build_range(&log, 0..1).unwrap();
    assert!(range.copy(0x1004, &mut bytes));
    assert_eq!(bytes, [9; 8]);
    assert_eq!(
        range.immutable_view(0x1004, bytes.len()).unwrap().bytes,
        &bytes[..]
    );
}

#[test]
fn simultaneous_recorded_views_have_independent_cursors() {
    let mut log = RawLog::<1>::new(4, 32);
    for byte in [7, 19] {
        log.take(
            &SliceMemory {
                base: 0x1000,
                bytes: &[byte; 8],
            },
            0x1000,
            8,
            1,
        )
        .unwrap();
    }
    let first = Recorded::build(&log).unwrap();
    let second = Recorded::build(&log).unwrap();
    let mut bytes = [0; 8];
    assert!(first.copy(0x1000, &mut bytes));
    assert!(first.copy(0x1000, &mut bytes));
    assert_eq!(bytes, [19; 8]);
    assert!(second.copy(0x1000, &mut bytes));
    assert_eq!(bytes, [7; 8]);
    drop(first);
    assert!(Recorded::build(&log).unwrap().copy(0x1000, &mut bytes));
    assert_eq!(bytes, [7; 8]);
    assert!(second.copy(0x1000, &mut bytes));
    assert_eq!(bytes, [19; 8]);
}

fn set_index_fixture(log: &mut RawLog<1>, reads: &[(usize, u32, Option<u8>, bool)]) {
    log.clear();
    for &(addr, len, value, bulk) in reads {
        let start = log.bytes.len() as u32;
        if let Some(value) = value {
            log.bytes.resize(log.bytes.len() + len as usize, value);
        }
        log.spans.push(Span {
            addr,
            len,
            ok: value.is_some(),
            mask: 1,
            start,
            bulk,
        });
        log.has_bulk |= bulk;
    }
}

fn assert_recorded_query(
    log: &RawLog<1>,
    recorded: &Recorded<'_>,
    observations: &mut std::collections::BTreeMap<(usize, u32), usize>,
    (address, len): (usize, u32),
) {
    let mut exact = log
        .spans
        .iter()
        .filter(|span| span.addr == address && span.len == len);
    let source = if let Some(first) = exact.next() {
        let observation = observations.entry((address, len)).or_default();
        let mut selected = first;
        for span in exact.take(*observation) {
            selected = span;
        }
        *observation += 1;
        selected.ok.then_some((selected, 0))
    } else {
        log.spans
            .iter()
            .enumerate()
            .filter(|(_, span)| {
                log.has_bulk
                    && span.ok
                    && span.bulk
                    && span.addr <= address
                    && address + len as usize <= span.addr + span.len as usize
            })
            .max_by_key(|(index, span)| (span.addr, span.addr + span.len as usize, *index))
            .map(|(_, span)| (span, address - span.addr))
    };
    let mut bytes = vec![0xee; len as usize];
    assert_eq!(recorded.copy(address, &mut bytes), source.is_some());
    if let Some((span, offset)) = source {
        let start = span.start as usize + offset;
        assert_eq!(bytes, log.bytes[start..start + len as usize]);
    } else {
        assert!(bytes.iter().all(|byte| *byte == 0xee));
    }
}

fn assert_index_fixture(log: &RawLog<1>) {
    let recorded = Recorded::build(log).unwrap();
    let mut observations = std::collections::BTreeMap::<(usize, u32), usize>::new();
    for _ in 0..3 {
        for span in &log.spans {
            for (address, len) in [
                (span.addr, span.len),
                (span.addr + 1, span.len.saturating_sub(1).max(1)),
                (span.addr, span.len.saturating_add(1)),
            ] {
                assert_recorded_query(log, &recorded, &mut observations, (address, len));
            }
        }
    }
}

#[test]
fn exact_length_mask_absent_width_keeps_bulk_winners_and_unique_hint() {
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[
            (0x5000, 4, Some(3), false),
            (0x1000, 128, Some(7), true),
            (0x1020, 32, Some(11), true),
            (0x1020, 32, Some(19), true),
            (0x5004, 8, Some(23), false),
        ],
    );
    let recorded = Recorded::build(&log).unwrap();
    assert!(!recorded.index.may_have_exact_length(12));
    let mut observations = std::collections::BTreeMap::new();
    assert_recorded_query(&log, &recorded, &mut observations, (0x5000, 4));
    assert_eq!(recorded.next_exact.get(), 1);
    for address in [0x1003, 0x1024, 0x1040, 0x1025, 0x1078, 0x1004] {
        assert_recorded_query(&log, &recorded, &mut observations, (address, 12));
        assert_eq!(recorded.next_exact.get(), 1);
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x1020, 32).unwrap().get(),
            2
        );
    }
    assert_recorded_query(&log, &recorded, &mut observations, (0x1000, 128));
    assert_eq!(recorded.next_exact.get(), 2);
    assert_recorded_query(&log, &recorded, &mut observations, (0x5004, 8));
    assert_eq!(recorded.next_exact.get(), 5);
}

#[test]
fn exact_length_mask_includes_failures_and_preserves_exact_over_bulk() {
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[
            (0x1000, 128, Some(7), true),
            (0x1008, 12, None, false),
            (0x1008, 12, Some(19), false),
            (0x1008, 12, None, false),
            (0x2000, 13, None, true),
        ],
    );
    let recorded = Recorded::build(&log).unwrap();
    for len in [12, 13, 128] {
        assert!(recorded.index.may_have_exact_length(len));
    }
    let mut observations = std::collections::BTreeMap::new();
    for query in [
        (0x1008, 12),
        (0x1009, 12),
        (0x1008, 12),
        (0x1008, 12),
        (0x1008, 12),
        (0x2000, 13),
        (0x1004, 13),
    ] {
        assert_recorded_query(&log, &recorded, &mut observations, query);
    }
    assert_eq!(
        recorded.index.cursor(&log.spans, 0x1008, 12).unwrap().get(),
        3
    );
}

#[test]
fn exact_length_mask_large_width_bucket_is_conservative() {
    let widths = [
        0,
        1,
        12,
        63,
        64,
        127,
        128,
        191,
        192,
        254,
        255,
        256,
        u32::MAX,
    ];
    for marked in widths {
        let mut index = RecordedIndex::default();
        index.mark_exact_length(marked);
        for queried in widths {
            assert_eq!(
                index.may_have_exact_length(queried),
                marked.min(255) == queried.min(255),
                "marked {marked}, queried {queried}"
            );
        }
    }
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[
            (0x1000, 1024, Some(7), true),
            (0x1100, 255, Some(11), false),
            (0x1200, 256, None, false),
            (0x4000, u32::MAX, None, false),
        ],
    );
    let recorded = Recorded::build(&log).unwrap();
    assert!(recorded.index.may_have_exact_length(300));
    assert!(recorded.index.may_have_exact_length(u32::MAX));
    let mut observations = std::collections::BTreeMap::new();
    for query in [
        (0x1001, 300),
        (0x1100, 255),
        (0x1200, 256),
        (0x1201, 256),
        (0x1002, 254),
    ] {
        assert_recorded_query(&log, &recorded, &mut observations, query);
    }
}

#[test]
fn exact_length_mask_reuse_rebuild_empty_and_bulk_gate_follow_current_shape() {
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[(0x1000, 64, Some(7), true), (0x2000, 12, Some(11), false)],
    );
    let mut index = RecordedIndex::default();
    assert!(!index.prepare(&log.spans, log.has_bulk).unwrap());
    let mask = index.exact_lengths;
    assert!(index.may_have_exact_length(12));
    assert!(index.prepare(&log.spans, log.has_bulk).unwrap());
    assert_index_fixture(&log);

    log.bytes.clear();
    log.bytes.resize(13, 0xa5);
    for (position, span) in log.spans.iter_mut().enumerate() {
        span.start = log.bytes.len() as u32;
        log.bytes
            .resize(log.bytes.len() + span.len as usize, 31 + position as u8);
    }
    assert!(index.prepare(&log.spans, log.has_bulk).unwrap());
    assert_eq!(index.exact_lengths, mask);
    assert_index_fixture(&log);

    set_index_fixture(&mut log, &[(0x1000, 64, Some(41), true)]);
    assert!(!index.prepare(&log.spans, log.has_bulk).unwrap());
    assert!(!index.may_have_exact_length(12));
    assert_index_fixture(&log);
    log.spans[0].ok = false;
    assert!(!index.prepare(&log.spans, log.has_bulk).unwrap());
    assert!(index.may_have_exact_length(64));
    assert_index_fixture(&log);
    log.spans[0].ok = true;
    log.spans[0].bulk = false;
    assert!(!index.prepare(&log.spans, log.has_bulk).unwrap());
    assert_index_fixture(&log);
    log.has_bulk = false;
    assert!(!index.prepare(&log.spans, log.has_bulk).unwrap());
    assert!(index.may_have_exact_length(64));
    assert_index_fixture(&log);

    log.clear();
    assert!(!index.prepare(&log.spans, log.has_bulk).unwrap());
    assert_eq!(index.exact_lengths, [0; 4]);
    assert!(index.prepare(&log.spans, log.has_bulk).unwrap());
    assert_index_fixture(&log);
    assert_eq!(log.index.borrow().as_ref().unwrap().exact_lengths, [0; 4]);
}

#[test]
fn exact_length_mask_bulk_subranges_leave_repeated_cursors_independent() {
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[
            (0x1000, 64, Some(7), true),
            (0x1000, 64, None, true),
            (0x1000, 64, Some(23), true),
            (0x3000, 4, Some(31), false),
        ],
    );
    let first = Recorded::build(&log).unwrap();
    let second = Recorded::build(&log).unwrap();
    for recorded in [&first, &second] {
        assert!(!recorded.index.may_have_exact_length(12));
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x1000, 64).unwrap().get(),
            0
        );
        let mut observations = std::collections::BTreeMap::new();
        for expected_cursor in [1, 2, 2, 2] {
            assert_recorded_query(&log, recorded, &mut observations, (0x1000, 64));
            let next_exact = recorded.next_exact.get();
            for address in [0x1001, 0x1010, 0x1038, 0x1002] {
                assert_recorded_query(&log, recorded, &mut observations, (address, 12));
                assert_eq!(recorded.next_exact.get(), next_exact);
                assert_eq!(
                    recorded.index.cursor(&log.spans, 0x1000, 64).unwrap().get(),
                    expected_cursor
                );
            }
        }
    }
    drop(first);
    drop(second);
    let rebuilt = Recorded::build(&log).unwrap();
    assert_eq!(
        rebuilt.index.cursor(&log.spans, 0x1000, 64).unwrap().get(),
        0
    );
    let mut observations = std::collections::BTreeMap::new();
    assert_recorded_query(&log, &rebuilt, &mut observations, (0x1000, 64));
}

#[test]
fn compact_map_preserves_reset_roots_and_terminal_duplicate_faults() {
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[
            (0x1000, 4, Some(7), false),
            (0x2000, 4, Some(11), false),
            (0x3000, 8, None, false),
            (0x2000, 4, None, false),
            (0x4000, 4, Some(19), false),
            (0x3000, 8, Some(23), false),
            (0x2000, 4, None, false),
            (0x3000, 8, Some(31), false),
        ],
    );
    for _ in 0..3 {
        let recorded = Recorded::build(&log).unwrap();
        assert_eq!(
            recorded.index.repeated,
            [
                (recorded.index.slot(&log.spans, 0x2000, 4).unwrap(), 1),
                (recorded.index.slot(&log.spans, 0x3000, 8).unwrap(), 2),
            ]
        );
        assert_eq!(
            recorded.index.next,
            [usize::MAX, 3, 5, 6, usize::MAX, 7, 6, 7]
        );
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x2000, 4).unwrap().get(),
            1
        );
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x3000, 8).unwrap().get(),
            2
        );
        let mut observations = std::collections::BTreeMap::new();
        for query in [
            (0x1000, 4),
            (0x2000, 4),
            (0x3000, 8),
            (0x4000, 4),
            (0x2000, 4),
            (0x3000, 8),
            (0x2000, 4),
            (0x3000, 8),
            (0x2000, 4),
            (0x3000, 8),
            (0x1000, 4),
            (0x4000, 4),
        ] {
            assert_recorded_query(&log, &recorded, &mut observations, query);
        }
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x2000, 4).unwrap().get(),
            6
        );
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x3000, 8).unwrap().get(),
            7
        );
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x1000, 4).unwrap().get(),
            0
        );
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x4000, 4).unwrap().get(),
            4
        );
    }
}

#[test]
fn unique_exact_hint_matches_linear_reads_in_sequential_and_arbitrary_order() {
    let reads: Vec<_> = (0..64)
        .map(|index| {
            (
                0x1000 + index * 32,
                4 + index as u32 % 3,
                (index % 7 != 0).then_some(index as u8),
                false,
            )
        })
        .collect();
    let mut log = RawLog::<1>::default();
    set_index_fixture(&mut log, &reads);
    let recorded = Recorded::build(&log).unwrap();
    assert!(recorded.index.next.iter().all(|&next| next == usize::MAX));
    let mut observations = std::collections::BTreeMap::new();
    for (index, &(address, len, _, _)) in reads.iter().enumerate() {
        assert_recorded_query(&log, &recorded, &mut observations, (address, len));
        assert_eq!(recorded.next_exact.get(), index + 1);
    }
    for index in (0..reads.len())
        .rev()
        .chain((0..256).map(|index| index * 37 % 64))
    {
        let (address, len, _, _) = reads[index];
        assert_recorded_query(&log, &recorded, &mut observations, (address, len));
        assert_eq!(recorded.next_exact.get(), index + 1);
        assert_recorded_query(&log, &recorded, &mut observations, (address, len + 9));
    }
}

#[test]
fn unique_exact_hint_never_skips_an_earlier_duplicate_observation() {
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[
            (0x1000, 4, Some(7), false),
            (0x2000, 4, Some(11), false),
            (0x1000, 4, None, false),
            (0x3000, 4, Some(19), false),
            (0x1000, 4, Some(23), false),
            (0x4000, 8, Some(31), false),
        ],
    );
    let recorded = Recorded::build(&log).unwrap();
    assert_eq!(
        recorded.index.next,
        [2, usize::MAX, 4, usize::MAX, 4, usize::MAX]
    );
    let mut observations = std::collections::BTreeMap::new();
    for query in [
        (0x2000, 4),
        (0x1000, 4),
        (0x3000, 4),
        (0x1000, 4),
        (0x3000, 4),
        (0x1000, 4),
        (0x2000, 4),
        (0x1000, 4),
        (0x4000, 8),
        (0x1000, 4),
    ] {
        assert_recorded_query(&log, &recorded, &mut observations, query);
    }
}

#[test]
fn unique_exact_hint_preserves_fault_priority_bulk_ties_and_independent_readers() {
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[
            (0x1000, 64, Some(7), true),
            (0x1010, 16, Some(11), true),
            (0x1010, 16, Some(19), true),
            (0x1014, 4, None, false),
            (0x2000, 8, Some(23), false),
            (0x1014, 4, Some(31), false),
            (0x1008, 4, None, false),
        ],
    );
    let first = Recorded::build(&log).unwrap();
    let second = Recorded::build(&log).unwrap();
    let mut first_observations = std::collections::BTreeMap::new();
    let mut second_observations = std::collections::BTreeMap::new();
    let queries = [
        (0x1000, 64),
        (0x1011, 4),
        (0x1010, 16),
        (0x2000, 8),
        (0x1014, 4),
        (0x1014, 4),
        (0x1011, 24),
        (0x1010, 16),
        (0x1010, 16),
        (0x1008, 4),
        (0x1008, 4),
        (0x1040, 1),
    ];
    for (index, &query) in queries.iter().enumerate() {
        assert_recorded_query(&log, &first, &mut first_observations, query);
        assert_recorded_query(
            &log,
            &second,
            &mut second_observations,
            queries[queries.len() - index - 1],
        );
    }
    drop(first);
    let third = Recorded::build(&log).unwrap();
    assert_eq!(third.next_exact.get(), 0);
    let mut third_observations = std::collections::BTreeMap::new();
    for query in queries {
        assert_recorded_query(&log, &third, &mut third_observations, query);
    }
    assert_recorded_query(&log, &second, &mut second_observations, (0x1014, 4));
}

#[test]
fn unique_exact_hint_uses_current_bytes_and_offsets_after_shape_reuse() {
    let mut log = RawLog::<1>::default();
    let reads = [
        (0x1000, 4, Some(7), false),
        (0x2000, 8, Some(11), false),
        (0x3000, 4, None, false),
        (0x4000, 4, Some(19), false),
    ];
    set_index_fixture(&mut log, &reads);
    assert_index_fixture(&log);
    let original_slots = log.index.borrow().as_ref().unwrap().slots.capacity();
    log.bytes.clear();
    log.bytes.resize(13, 0xa5);
    for (index, span) in log.spans.iter_mut().enumerate() {
        if span.ok {
            span.start = log.bytes.len() as u32;
            log.bytes
                .resize(log.bytes.len() + span.len as usize, 41 + index as u8);
        }
    }
    assert_index_fixture(&log);
    assert_eq!(
        log.index.borrow().as_ref().unwrap().slots.capacity(),
        original_slots
    );
    for tail in [Some((0x1000, 4, Some(59), false)), None] {
        let mut changed = reads.to_vec();
        changed.extend(tail);
        set_index_fixture(&mut log, &changed);
        assert_index_fixture(&log);
        let cached = log.index.borrow();
        let next = &cached.as_ref().unwrap().next;
        assert_eq!(next[0] == usize::MAX, tail.is_none());
        if tail.is_some() {
            assert_eq!(next[4], 4);
        }
    }
}

#[test]
fn index_refresh_handles_insertions_deletions_reorders_and_duplicate_faults() {
    let a = (0x1000, 4, Some(7), false);
    let b = (0x2000, 8, Some(11), false);
    let c = (0x3000, 4, Some(19), false);
    let d = (0x4000, 4, Some(31), false);
    let failed_a = (0x1000, 4, None, false);
    let later_a = (0x1000, 4, Some(47), false);
    let mut log = RawLog::<1>::default();
    for reads in [
        vec![a, b, later_a, c],
        vec![a, b, failed_a, later_a, c],
        vec![d, a, b, failed_a, later_a, c],
        vec![d, a, failed_a, later_a, c],
        vec![d, a, failed_a, later_a, c, b],
        vec![c, failed_a, d, later_a, a, b],
        vec![c, (0x1000, 8, Some(59), false), d, a, b],
        vec![],
        vec![failed_a, later_a, a],
    ] {
        set_index_fixture(&mut log, &reads);
        assert_index_fixture(&log);
        let cached = log.index.borrow();
        let index = cached.as_ref().unwrap();
        let keys: std::collections::BTreeSet<_> =
            reads.iter().map(|read| (read.0, read.1)).collect();
        assert_eq!(index.distinct, keys.len());
        assert_eq!(index.next.len(), reads.len());
        assert_eq!(index.shape.len(), reads.len());
        assert!(
            keys.iter()
                .all(|key| index.cursor(&log.spans, key.0, key.1).is_some())
        );
    }
}

#[test]
fn index_refresh_handles_bulk_flags_faults_and_current_pool_offsets() {
    let mut log = RawLog::<1>::default();
    for (parent, child, exact, bulk_gate) in [
        (Some(7), Some(11), Some(19), true),
        (Some(23), Some(31), None, true),
        (Some(37), None, Some(41), true),
        (None, Some(43), Some(47), true),
        (Some(53), Some(59), Some(61), false),
        (Some(67), Some(71), Some(73), true),
    ] {
        set_index_fixture(
            &mut log,
            &[
                (0x1000, 32, parent, bulk_gate),
                (0x1008, 8, child, true),
                (0x1009, 4, exact, false),
            ],
        );
        log.has_bulk = bulk_gate;
        assert_index_fixture(&log);
    }
}

#[test]
fn index_refresh_drops_inactive_keys_and_retains_only_peak_capacity() {
    let mut log = RawLog::<1>::default();
    let mut index = RecordedIndex::default();
    let mut peak_count = 0;
    let mut peak_capacity = 0;
    for refresh in 0..64 {
        let count = if refresh == 12 { 17 } else { 4 };
        let reads: Vec<_> = (0..count)
            .map(|row| (0x1000 + refresh * 0x1000 + row * 16, 4, Some(7), false))
            .collect();
        set_index_fixture(&mut log, &reads);
        index.prepare(&log.spans, false).unwrap();
        assert_eq!(index.distinct, count);
        if count > peak_count {
            peak_count = count;
            peak_capacity = index.slots.capacity();
        }
        assert_eq!(index.slots.capacity(), peak_capacity);
        assert_eq!(index.next.len(), count);
        assert_eq!(index.shape.len(), count);
        assert!(index.repeated.is_empty());
        assert!(
            reads
                .iter()
                .all(|read| index.cursor(&log.spans, read.0, read.1).is_some())
        );
    }
}

#[test]
fn compact_index_collisions_wrap_and_keep_each_observation_stream() {
    let addresses: Vec<_> = (1..=4096)
        .map(|index| 0x1000 + index * 16)
        .filter(|&address| RecordedIndex::hash(address, 4) & 7 == 7)
        .take(5)
        .collect();
    assert_eq!(addresses.len(), 5);
    let mut log = RawLog::<1>::default();
    set_index_fixture(
        &mut log,
        &[
            (addresses[0], 4, Some(7), false),
            (addresses[1], 4, Some(11), false),
            (addresses[2], 4, None, false),
            (addresses[3], 4, Some(19), false),
            (addresses[0], 4, None, false),
            (addresses[2], 4, Some(23), false),
            (addresses[0], 4, Some(31), false),
        ],
    );
    for _ in 0..3 {
        let recorded = Recorded::build(&log).unwrap();
        assert_eq!(recorded.index.slots.len(), 8);
        assert_eq!(recorded.index.distinct, 4);
        for (index, &address) in addresses[..4].iter().enumerate() {
            assert_eq!(
                recorded.index.slot(&log.spans, address, 4),
                Some((7 + index) & 7)
            );
        }
        assert_eq!(recorded.index.slot(&log.spans, addresses[4], 4), Some(3));
        assert!(recorded.index.cursor(&log.spans, addresses[4], 4).is_none());
        let mut observations = std::collections::BTreeMap::new();
        for _ in 0..4 {
            for &address in addresses.iter().rev() {
                assert_recorded_query(&log, &recorded, &mut observations, (address, 4));
                assert_recorded_query(&log, &recorded, &mut observations, (address, 8));
            }
        }
    }
}

#[test]
fn compact_index_resolves_early_duplicate_roots_after_all_rehashes() {
    let first = (1..=4096)
        .map(|index| 0x1000 + index * 16)
        .find(|&address| (32..64).contains(&(RecordedIndex::hash(address, 4) & 255)))
        .unwrap();
    let second = first + 16;
    let mut reads = vec![
        (first, 4, Some(7), false),
        (second, 4, Some(11), false),
        (first, 4, None, false),
        (second, 4, Some(19), false),
    ];
    reads.extend((0..70).map(|index| (first + 0x1000 + index * 16, 4, Some(37), false)));
    reads.extend([(first, 4, Some(23), false), (second, 4, None, false)]);
    let mut log = RawLog::<1>::default();
    set_index_fixture(&mut log, &reads);
    for _ in 0..3 {
        assert_index_fixture(&log);
        let recorded = Recorded::build(&log).unwrap();
        assert_eq!(recorded.index.slots.len(), 256);
        assert_eq!(recorded.index.distinct, 72);
        let first_slot = recorded.index.slot(&log.spans, first, 4).unwrap();
        let second_slot = recorded.index.slot(&log.spans, second, 4).unwrap();
        assert_ne!(first_slot, RecordedIndex::hash(first, 4) & 7);
        assert_eq!(recorded.index.repeated, [(first_slot, 0), (second_slot, 1)]);
        assert_eq!(recorded.index.slots[first_slot].get(), 0);
        assert_eq!(recorded.index.slots[second_slot].get(), 1);
    }
}

#[test]
fn compact_index_capacity_depends_on_distinct_keys_not_observations() {
    let reads: Vec<_> = (0..2048)
        .map(|index| {
            (
                0x1000,
                if index % 2 == 0 { 4 } else { 8 },
                (index % 7 != 0).then_some((index % 251) as u8),
                false,
            )
        })
        .collect();
    let mut log = RawLog::<1>::default();
    set_index_fixture(&mut log, &reads);
    for _ in 0..3 {
        let recorded = Recorded::build(&log).unwrap();
        assert_eq!(recorded.index.distinct, 2);
        assert_eq!(recorded.index.slots.len(), 8);
        assert_eq!(recorded.index.slots.capacity(), 8);
        assert_eq!(recorded.index.repeated.len(), 2);
        for &(address, len, value, _) in &reads {
            let mut bytes = [0xee; 8];
            assert_eq!(
                recorded.copy(address, &mut bytes[..len as usize]),
                value.is_some()
            );
            let expected = [value.unwrap_or(0xee); 8];
            assert_eq!(bytes[..len as usize], expected[..len as usize]);
        }
        for &(address, len, value, _) in &reads[reads.len() - 2..] {
            let mut bytes = [0xee; 8];
            assert_eq!(
                recorded.copy(address, &mut bytes[..len as usize]),
                value.is_some()
            );
            let expected = [value.unwrap_or(0xee); 8];
            assert_eq!(bytes[..len as usize], expected[..len as usize]);
        }
    }
}

#[test]
fn compact_index_shrink_clears_exhausted_cursors_before_using_new_spans() {
    let mut reads: Vec<_> = (0..70)
        .map(|index| (0x1000 + index * 16, 4, Some(7), false))
        .collect();
    reads.extend((0..70).map(|index| (0x1000 + index * 16, 4, None, false)));
    let mut log = RawLog::<1>::default();
    set_index_fixture(&mut log, &reads);
    assert_index_fixture(&log);
    let capacity = log.index.borrow().as_ref().unwrap().slots.capacity();
    {
        let cached = log.index.borrow();
        let index = cached.as_ref().unwrap();
        assert_eq!(index.cursor(&log.spans, 0x1000, 4).unwrap().get(), 70);
    }
    set_index_fixture(
        &mut log,
        &[(0x1000, 4, Some(201), false), (0x1000, 4, None, false)],
    );
    {
        let recorded = Recorded::build(&log).unwrap();
        assert_eq!(recorded.index.slots.capacity(), capacity);
        assert_eq!(recorded.index.distinct, 1);
        let mut observations = std::collections::BTreeMap::new();
        for query in [
            (0x1000, 4),
            (0x1000, 4),
            (0x1000, 4),
            (0x1010, 4),
            (0x1450, 4),
        ] {
            assert_recorded_query(&log, &recorded, &mut observations, query);
        }
    }
    set_index_fixture(&mut log, &[]);
    {
        let recorded = Recorded::build(&log).unwrap();
        assert_eq!(recorded.index.slots.capacity(), capacity);
        assert_eq!(recorded.index.distinct, 0);
        assert!(
            recorded
                .index
                .slots
                .iter()
                .all(|slot| slot.get() == usize::MAX)
        );
        assert!(recorded.index.repeated.is_empty());
        assert!(!recorded.copy(0x1000, &mut [0; 4]));
    }
    set_index_fixture(&mut log, &[(0x1000, 8, Some(211), true)]);
    assert_index_fixture(&log);
    let recorded = Recorded::build(&log).unwrap();
    assert_eq!(recorded.index.slots.capacity(), capacity);
    let mut bytes = [0; 4];
    assert!(recorded.copy(0x1002, &mut bytes));
    assert_eq!(bytes, [211; 4]);
}

#[test]
fn edited_indexes_keep_simultaneous_recorded_views_independent() {
    let mut log = RawLog::<1>::default();
    set_index_fixture(&mut log, &[(0x1000, 4, Some(7), false)]);
    assert_index_fixture(&log);
    set_index_fixture(
        &mut log,
        &[
            (0x2000, 4, Some(11), false),
            (0x1000, 4, Some(19), false),
            (0x1000, 4, None, false),
            (0x1000, 4, Some(23), false),
        ],
    );
    let first = Recorded::build(&log).unwrap();
    let second = Recorded::build(&log).unwrap();
    for view in [&first, &second] {
        let mut bytes = [0; 4];
        assert!(view.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [19; 4]);
        assert!(!view.copy(0x1000, &mut bytes));
    }
    drop(first);
    let third = Recorded::build(&log).unwrap();
    let mut bytes = [0; 4];
    assert!(third.copy(0x1000, &mut bytes));
    assert_eq!(bytes, [19; 4]);
    assert!(second.copy(0x1000, &mut bytes));
    assert_eq!(bytes, [23; 4]);
}

struct FailAll;

impl Memory for FailAll {
    fn copy(&self, _address: usize, _destination: &mut [u8]) -> bool {
        false
    }
}

#[test]
fn record_gathers_stage_each_field_and_replay_to_the_same_fault() {
    use engine_native_memory::memory::{
        CountedMemory,
        ReadCounts,
        RecordField,
        RecordLayout,
        RecordSources,
    };
    struct Fault<'a>(SliceMemory<'a>);
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            address != 0x1110 && self.0.copy(address, destination)
        }
    }
    let bytes: Vec<u8> = (0..0x200).map(|at| at as u8).collect();
    let memory = CountedMemory::new(Fault(SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    }));
    let fields = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 4,
        },
        RecordField {
            source_offset: 0x10,
            destination_offset: 4,
            len: 2,
        },
    ];
    let layout = RecordLayout::new(&fields, 6).unwrap();
    let sources = RecordSources::Pointers(&[0x1000, 0x1100, 0x1180]);
    let mut log = RawLog::<2>::new(8, 64);
    ReadCounts::take_current();
    let (start, completed) = log.take_records(&memory, sources, &layout, 0x02).unwrap();
    assert_eq!(ReadCounts::take_current().record_calls, 1);
    assert_eq!((start, completed), (0, 3));
    let shape: Vec<_> = log
        .spans
        .iter()
        .map(|s| (s.addr, s.len, s.ok, s.start))
        .collect();
    assert_eq!(
        shape,
        [
            (0x1000, 4, true, 0),
            (0x1010, 2, true, 4),
            (0x1100, 4, true, 6),
            (0x1110, 2, false, 0)
        ]
    );
    assert_eq!(log.fault, [false, true]);
    assert_eq!(log.raw_bytes, [0, 10]);
    let recorded = Recorded::build(&log).unwrap();
    let mut direct = [0; 18];
    let mut replayed = [0; 18];
    assert_eq!(memory.copy_records(sources, &layout, &mut direct), 3);
    assert_eq!(recorded.copy_records(sources, &layout, &mut replayed), 3);
    assert_eq!(direct[..10], replayed[..10]);
    drop(recorded);
    let null = RecordSources::Pointers(&[0x1000, 0, 0x1180]);
    let mut log = RawLog::<2>::new(8, 64);
    ReadCounts::take_current();
    assert_eq!(log.take_records(&memory, null, &layout, 0x02), Some((0, 2)));
    let shape: Vec<_> = log.spans.iter().map(|s| (s.addr, s.len, s.ok)).collect();
    assert_eq!(shape, [(0x1000, 4, true), (0x1010, 2, true), (0, 4, false)]);
    let recorded = Recorded::build(&log).unwrap();
    let mut direct = [0; 18];
    let mut replayed = [0; 18];
    assert_eq!(memory.copy_records(null, &layout, &mut direct), 2);
    assert_eq!(recorded.copy_records(null, &layout, &mut replayed), 2);
    assert_eq!(direct[..6], replayed[..6]);
}

#[test]
fn take_records_bytes_and_failures_with_table_masks() {
    let bytes = [9u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<3>::new(8, 64);
    let start = log.take(&memory, 0x1000, 4, 0x01).expect("copy");
    assert_eq!(&log.bytes[start..start + 4], &[9, 9, 9, 9]);
    assert_eq!(log.raw_bytes, [4, 0, 0]);
    assert_eq!(log.take(&memory, 0xdead, 0, 0x02), Some(4));
    assert_eq!(log.spans.len(), 1);
    assert!(log.take(&memory, 0, 4, 0x02).is_none());
    assert!(log.fault[1]);
    assert!(!log.fault[0]);
    assert!(log.take(&FailAll, 0x1010, 4, 0x06).is_none());
    assert_eq!(log.fault, [false, true, true]);
}

#[test]
fn initialized_arena_keeps_exact_scalar_bulk_prefixes_after_partial_faults() {
    struct Traced {
        partial: usize,
        calls: RefCell<Vec<(usize, usize)>>,
        initial: RefCell<Vec<Vec<u8>>>,
    }
    impl Memory for Traced {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.initial.borrow_mut().push(destination.to_vec());
            self.calls.borrow_mut().push((address, destination.len()));
            let completed = if address == 0x2000 {
                self.partial.min(destination.len())
            } else {
                destination.len()
            };
            for (index, byte) in destination[..completed].iter_mut().enumerate() {
                *byte = (address.wrapping_add(index * 29) as u8) ^ 0xa5;
            }
            address != 0x2000
        }
    }

    for len in [0_usize, 1, 2, 3, 4, 7, 8, 15, 16, 24, 26, 31, 32, 33, 64] {
        for partial in [0, len / 2, len.saturating_sub(1)] {
            let scalar_memory = Traced {
                partial,
                calls: RefCell::default(),
                initial: RefCell::default(),
            };
            let generic_memory = Traced {
                partial,
                calls: RefCell::default(),
                initial: RefCell::default(),
            };
            let mut scalar = RawLog::<2>::new(8, 256);
            let mut generic = RawLog::<2>::new(8, 256);
            let prefix = [0xb3, 0, 0xff, 0x71];
            for log in [&mut scalar, &mut generic] {
                log.bytes.resize(log.bytes.capacity(), 0xcd);
                log.bytes.truncate(prefix.len());
                log.bytes.copy_from_slice(&prefix);
            }
            let mut expected = prefix.to_vec();
            for (address, mask) in [
                (0x1000, 1),
                (0x2000, 2),
                (0x1100, 1),
                (0, 2),
                (0x0072_6f74_6361_665f, 2),
                (usize::MAX, 2),
            ] {
                let scalar_result = scalar.take(&scalar_memory, address, len, mask);
                let generic_result = generic.take_bulk(&generic_memory, address, len, mask);
                if generic_result.is_some() && len != 0 {
                    generic.spans.last_mut().unwrap().bulk = false;
                }
                assert_eq!(scalar_result, generic_result);
                assert!(scalar.logs_equal(&generic));
                assert_eq!(scalar.fault, generic.fault);
                assert_eq!(scalar.raw_bytes, generic.raw_bytes);
                assert_eq!(scalar.overflowed, generic.overflowed);
                assert_eq!(scalar.span_exhausted, generic.span_exhausted);
                assert_eq!(scalar.required_bytes, generic.required_bytes);
                assert_eq!(scalar_memory.calls, generic_memory.calls);
                assert_eq!(&scalar.bytes[..prefix.len()], &prefix);
                if len != 0 && matches!(address, 0x1000 | 0x1100) {
                    expected.extend(
                        (0..len).map(|index| (address.wrapping_add(index * 29) as u8) ^ 0xa5),
                    );
                }
                assert_eq!(scalar.bytes, expected);
                assert!(
                    scalar
                        .bytes
                        .get(expected.len()..expected.len() + 1)
                        .is_none()
                );
            }
            let expected_calls = if len == 0 {
                Vec::new()
            } else {
                vec![(0x1000, len), (0x2000, len), (0x1100, len)]
            };
            assert_eq!(*scalar_memory.calls.borrow(), expected_calls);
            assert_eq!(scalar.raw_bytes, [(len * 2) as u64, 0]);
            if len != 0 {
                assert_eq!(scalar_memory.initial.borrow()[0], vec![0xcd; len]);
                assert_eq!(scalar_memory.initial, generic_memory.initial);
                let reader = Recorded::build(&scalar).unwrap();
                let mut rejected = vec![0xe7; len];
                assert!(!reader.copy(0x2000, &mut rejected));
                assert_eq!(rejected, vec![0xe7; len]);
            }
        }
    }
}

#[test]
fn recorded_serves_exact_spans_and_fails_everything_else() {
    let bytes = [7u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<2>::new(8, 64);
    log.take(&memory, 0x1000, 8, 0x01);
    log.take(&memory, 0x1020, 4, 0x02);
    let recorded = Recorded::build(&log).unwrap();
    let mut out = [0u8; 8];
    assert!(recorded.copy(0x1000, &mut out));
    assert_eq!(out, [7; 8]);
    assert!(!recorded.copy(0x1000, &mut out[..4]));
    assert!(!recorded.copy(0x1004, &mut out));
    assert!(!recorded.copy(0x2000, &mut out));
    assert!(recorded.copy(0x2000, &mut []));
    assert!(!recorded.copy(0, &mut out));
}

#[test]
fn short_storage_grows_in_the_pass_and_a_torn_length_fails_only_its_read() {
    let bytes = [1u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::new(1, 4);
    assert_eq!(log.take(&memory, 0x1000, 4, 0x01), Some(0));
    assert_eq!(log.take(&memory, 0x1010, 4, 0x01), Some(4));
    assert!(!log.overflowed);
    assert_eq!(log.bytes, [1; 8]);
    assert_eq!(log.take(&memory, 0x1000, u32::MAX as usize + 1, 0x01), None);
    assert!(!log.overflowed);
    assert!(!log.grow_for_next());
    assert_eq!(log.refusal(), Some(TORN_READ));
    assert_eq!(log.take(&memory, 0x1000, 1, 0x01), Some(8));
    log.clear();
    assert_eq!(
        (log.overflowed, log.refusal(), log.fault),
        (false, None, [false])
    );
    assert_eq!(log.take(&memory, 0x1000, 4, 0x01), Some(0));
}

#[test]
fn a_read_longer_than_any_game_span_is_unreadable_and_claims_no_growth() {
    let bytes = [1u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<2>::new(4, 64);
    assert_eq!(log.take(&memory, 0x1000, u32::MAX as usize + 1, 0b10), None);
    assert!(
        !log.overflowed,
        "a torn length is an unreadable read, not storage"
    );
    assert_eq!(log.fault, [false, true]);
    assert!(!log.grow_for_next());
    assert!(log.spans.is_empty());
    assert_eq!(log.refusal(), Some(TORN_READ));
    assert_eq!(log.take(&memory, 0x1000, 4, 0b01), Some(0));
    assert_eq!(log.fault, [false, true]);
    log.clear();
    assert_eq!(log.refusal(), None);
}

#[test]
fn logs_compare_by_spans_and_bytes() {
    let bytes = [3u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut first = RawLog::<1>::new(8, 64);
    let mut second = RawLog::<1>::new(8, 64);
    first.take(&memory, 0x1000, 4, 0x01);
    second.take(&memory, 0x1000, 4, 0x01);
    assert!(first.logs_equal(&second));
    second.take(&memory, 0x1010, 4, 0x01);
    assert!(!first.logs_equal(&second));
}

#[test]
fn span_pressure_does_not_grow_the_byte_arena() {
    let bytes = [1; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::new(1, 64);
    log.take(&memory, 0x1000, 4, 1);
    log.take(&memory, 0x1004, 4, 1);
    assert!(!log.overflowed);
    assert!(log.spans.capacity() >= 2);
    assert_eq!(log.bytes.capacity(), 64);
}

#[test]
fn byte_pressure_does_not_grow_the_span_arena() {
    let bytes = [1; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::new(8, 4);
    log.take_bulk(&memory, 0x1000, 32, 1);
    assert!(!log.overflowed);
    assert_eq!(log.spans.capacity(), 8);
    assert!(log.bytes.capacity() >= 32);
}

#[test]
fn repeated_takes_serve_in_record_order_then_repeat_the_last() {
    let first_bytes = [3u8; 64];
    let first = SliceMemory {
        base: 0x1000,
        bytes: &first_bytes,
    };
    let mut second_bytes = [3u8; 64];
    second_bytes[0] = 9;
    let second = SliceMemory {
        base: 0x1000,
        bytes: &second_bytes,
    };
    let mut log = RawLog::<1>::new(8, 64);
    log.take(&first, 0x1000, 4, 0x01);
    log.take(&second, 0x1000, 4, 0x01);
    let recorded = Recorded::build(&log).unwrap();
    let mut out = [0u8; 4];
    assert!(recorded.copy(0x1000, &mut out));
    assert_eq!(out, [3, 3, 3, 3]);
    assert!(recorded.copy(0x1000, &mut out));
    assert_eq!(out, [9, 3, 3, 3]);
    assert!(recorded.copy(0x1000, &mut out));
    assert_eq!(out, [9, 3, 3, 3]);
    assert!(!recorded.copy(0x2000, &mut out));
}

#[test]
fn bulk_spans_serve_enclosed_unaligned_sub_ranges() {
    let mut bytes = [0u8; 128];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index & 0xff) as u8;
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::new(8, 128);
    log.has_bulk = true;
    log.take_bulk(&memory, 0x1000, 64, 0x01);
    log.take(&memory, 0x1060, 8, 0x01);
    let recorded = Recorded::build(&log).unwrap();
    let mut out = [0u8; 8];
    assert!(recorded.copy(0x1008, &mut out));
    assert_eq!(out, bytes[8..16]);
    let mut unaligned = [0u8; 7];
    assert!(recorded.copy(0x1003, &mut unaligned));
    assert_eq!(unaligned, bytes[3..10]);
    let mut tail = [0u8; 1];
    assert!(recorded.copy(0x103f, &mut tail));
    assert_eq!(tail, [bytes[63]]);
    let mut straddle = [0u8; 8];
    assert!(!recorded.copy(0x103c, &mut straddle));
    let mut exact = [0u8; 8];
    assert!(recorded.copy(0x1060, &mut exact));
    let mut plain = RawLog::<1>::new(8, 128);
    plain.take(&memory, 0x1000, 64, 0x01);
    let plain_recorded = Recorded::build(&plain).unwrap();
    assert!(!plain_recorded.copy(0x1008, &mut out));
}

#[test]
fn take_inner_rejects_noncanonical_addresses_as_faults_pre_copy() {
    use std::cell::Cell;
    struct Traced<'a> {
        inner: SliceMemory<'a>,
        calls: Cell<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.inner.copy(address, destination)
        }
    }
    let bytes = [9u8; 64];
    let memory = Traced {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        calls: Cell::new(0),
    };
    let mut log = RawLog::<2>::new(8, 64);
    for noncanonical in [0x0072_6f74_6361_665fusize, 0x6f74_6361_665f_7374usize] {
        assert!(
            log.take(&memory, noncanonical, 8, 0x01).is_none(),
            "noncanonical {noncanonical:x} must fail pre-copy"
        );
        assert!(log.fault[0]);
        assert!(!log.overflowed);
    }
    assert_eq!(
        memory.calls.get(),
        0,
        "noncanonical takes must not touch memory"
    );
    assert!(log.take(&memory, 0x1000, 8, 0x02).is_some());
    assert_eq!(memory.calls.get(), 1);
    let recorded = Recorded::build(&log).unwrap();
    let mut out = [0u8; 8];
    assert!(!recorded.copy(0x0072_6f74_6361_665f, &mut out));
    assert!(!recorded.copy(0x6f74_6361_665f_7374, &mut out));
    assert!(recorded.copy(0x1000, &mut out));
}

#[test]
fn nested_bulk_arrays_do_not_hide_later_parent_object_fields() {
    let bytes: Vec<u8> = (0..128).collect();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::new(8, 256);
    log.has_bulk = true;
    log.take_bulk(&memory, 0x1000, 128, 1);
    log.take_bulk(&memory, 0x1010, 16, 1);
    log.take_bulk(&memory, 0x1040, 8, 1);
    let recorded = Recorded::build(&log).unwrap();
    let mut out = [0; 24];
    assert!(recorded.copy(0x1028, &mut out));
    assert_eq!(out, bytes[40..64]);
    assert!(recorded.copy(0x1044, &mut out));
    assert_eq!(out, bytes[68..92]);
    assert!(!recorded.copy(0x1070, &mut out));
}

#[test]
fn bulk_cache_keeps_nested_winners_parent_fallback_and_exact_faults() {
    let mut log = RawLog::<1>::new(8, 256);
    log.has_bulk = true;
    for (base, bytes) in [
        (0x1000, &[1; 128][..]),
        (0x1010, &[2; 16][..]),
        (0x1040, &[3; 8][..]),
    ] {
        log.take_bulk(&SliceMemory { base, bytes }, base, bytes.len(), 1)
            .unwrap();
    }
    log.take(&FailAll, 0x1018, 4, 1);
    let recorded = Recorded::build(&log).unwrap();
    for (address, len, expected) in [
        (0x1001, 4, Some(1)),
        (0x1005, 4, Some(1)),
        (0x1011, 4, Some(2)),
        (0x1012, 24, Some(1)),
        (0x1013, 4, Some(2)),
        (0x1018, 4, None),
        (0x1041, 4, Some(3)),
        (0x1044, 12, Some(1)),
        (0x107f, 2, None),
    ] {
        let mut bytes = vec![0xa5; len];
        assert_eq!(recorded.copy(address, &mut bytes), expected.is_some());
        assert_eq!(bytes, vec![expected.unwrap_or(0xa5); len]);
    }
    for address in (0x0ffc..0x1084).rev() {
        for len in 1..12 {
            let expected = if address == 0x1018 && len == 4 {
                None
            } else {
                recorded
                    .index
                    .bulk
                    .iter()
                    .rev()
                    .find(|span| span.0 <= address && address + len <= span.1)
                    .map(|span| log.bytes[log.spans[span.2].start as usize])
            };
            let mut bytes = vec![0xa5; len];
            assert_eq!(
                recorded.copy(address, &mut bytes),
                expected.is_some(),
                "{address:x}/{len}"
            );
            assert_eq!(
                bytes,
                vec![expected.unwrap_or(0xa5); len],
                "{address:x}/{len}"
            );
        }
    }
}

#[test]
fn canonical_gate_includes_bit_47_in_the_sign_extension() {
    assert!(!is_canonical_range(0x0000_8000_0000_0000, 8));
    assert!(!is_canonical_range(0xFFFF_0000_0000_0000, 8));
    assert!(is_canonical_range(0x0000_7FFF_FFFF_FFF8, 8));
    assert!(is_canonical_range(0xFFFF_8000_0000_0000, 8));
}

#[test]
fn repeated_failed_reads_keep_their_place_and_do_not_fall_back_to_bulk() {
    let bytes = [7; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::new(8, 128);
    log.has_bulk = true;
    log.take_bulk(&memory, 0x1000, 64, 1);
    log.take(&memory, 0x1010, 8, 1);
    log.take(&FailAll, 0x1010, 8, 1);
    log.take(&memory, 0x1010, 8, 1);
    log.take(&FailAll, 0x1020, 8, 1);
    let recorded = Recorded::build(&log).unwrap();
    let mut out = [0; 8];
    assert!(recorded.copy(0x1010, &mut out));
    assert!(!recorded.copy(0x1010, &mut out));
    assert!(recorded.copy(0x1010, &mut out));
    assert!(recorded.copy(0x1010, &mut out));
    assert!(!recorded.copy(0x1020, &mut out));
    assert!(!recorded.copy(0x1020, &mut out));
}

#[test]
fn canonical_gate_matches_engine_vectors_and_keeps_valid_data() {
    assert!(!is_canonical_range(0x0072_6f74_6361_665f, 8));
    assert!(!is_canonical_range(0x6f74_6361_665f_7374, 8));
    assert!(is_canonical_range(0x1_4000_0000, 8));
    assert!(is_canonical_range(0xFFFF_8000_0000_0000, 8));
    assert!(!is_canonical_range(0x0000_7FFF_FFFF_FFF0, 32));
    assert!(is_canonical_range(0x2000, 0));
    assert!(!is_canonical_range(usize::MAX - 3, 8));
}

#[test]
fn recorded_ranges_keep_later_success_and_bulk_outside_a_failed_prefix() {
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[9; 64],
    };
    let mut log = RawLog::<1>::new(8, 128);
    assert!(log.take(&FailAll, 0x1010, 4, 1).is_none());
    let boundary = log.spans.len();
    assert!(log.take(&memory, 0x1010, 4, 0).is_some());
    assert!(log.take_bulk(&memory, 0x1000, 64, 0).is_some());
    {
        let prefix = Recorded::build_range(&log, 0..boundary).unwrap();
        let mut output = [0xa5; 4];
        assert!(!prefix.copy(0x1010, &mut output));
        assert_eq!(output, [0xa5; 4]);
        assert!(!prefix.copy(0x1004, &mut output));
    }
    {
        let suffix = Recorded::build_range(&log, boundary..log.spans.len()).unwrap();
        let mut output = [0; 4];
        assert!(suffix.copy(0x1010, &mut output));
        assert_eq!(output, [9; 4]);
        assert!(suffix.copy(0x1004, &mut output));
        assert_eq!(output, [9; 4]);
    }
    let prefix = Recorded::build_range(&log, 0..boundary).unwrap();
    assert!(!prefix.copy(0x1010, &mut [0; 4]));
}

#[test]
fn recorded_range_bounds_fail_before_taking_the_pooled_index() {
    let log = immutable_view_log();
    drop(Recorded::build_range(&log, 0..log.spans.len()).unwrap());
    assert!(log.index.borrow().is_some());
    assert!(Recorded::build_range(&log, std::ops::Range { start: 1, end: 0 }).is_err());
    assert!(log.index.borrow().is_some());
    assert!(Recorded::build_range(&log, 0..log.spans.len() + 1).is_err());
    assert!(log.index.borrow().is_some());
    let empty = Recorded::build_range(&log, 0..0).unwrap();
    assert!(!empty.copy(0x1000, &mut [0; 4]));
}

#[test]
fn a_refused_index_allocation_is_named_not_an_empty_log() {
    let bytes = [7_u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::new(4, 64);
    assert!(log.take(&memory, 0x1000, 8, 1).is_some());
    let refused =
        collector_test_support::with_allocations_denied(|| Recorded::build(&log).err());
    assert_eq!(refused, Some("recorded span index allocation failed"));
    let mut word = [0; 8];
    assert!(Recorded::build(&log).unwrap().copy(0x1000, &mut word));
    assert_eq!(word, [7; 8]);
}
