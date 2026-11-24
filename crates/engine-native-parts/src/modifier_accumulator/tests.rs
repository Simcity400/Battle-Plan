use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::{Cell, RefCell};

#[test]
fn requested_operand_keeps_lazy_disabled_presence_separate_from_raw_and_clamped_zero() {
    use crate::modifier_definition as definition;
    use std::collections::BTreeMap;
    struct Bytes(BTreeMap<usize, u8>);
    impl Bytes {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.0.extend(
                bytes
                    .iter()
                    .enumerate()
                    .map(|(index, byte)| (address + index, *byte)),
            );
        }
    }
    impl Memory for Bytes {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            for (index, byte) in output.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    let mut memory = Bytes(BTreeMap::new());
    memory.put(0x1000, &0x2000_u64.to_le_bytes());
    memory.put(0x1000 + COUNT, &1_i32.to_le_bytes());
    memory.put(0x2000, &1_u32.to_le_bytes());
    memory.put(definition::TABLE_RVA.0, &0x4000_u64.to_le_bytes());
    let metadata = 0x4000 + definition::STRIDE;
    memory.put(
        metadata + definition::FEATURE_SELECTOR,
        &4_u32.to_le_bytes(),
    );
    memory.put(definition::ELIGIBILITY_CALLBACK_RVA.0, &0_u64.to_le_bytes());
    assert_eq!(
        requested_operand_at(&memory, 0, 0x1000, 1),
        Ok(RequestedOperand {
            raw: None,
            present: true,
            effective: 0
        })
    );
    assert_eq!(
        requested_effective_memo(
            &memory,
            0,
            0x1000,
            1,
            &definition::DefinitionMemo::default()
        ),
        Ok(0)
    );
    memory.put(
        metadata + definition::FEATURE_SELECTOR,
        &0_u32.to_le_bytes(),
    );
    memory.put(
        metadata + definition::FLAGS,
        &definition::CLAMP_MAXIMUM.to_le_bytes(),
    );
    memory.put(0x2000 + modifier_entry::VALUE, &(-1_i64).to_le_bytes());
    assert_eq!(
        requested_operand_at(&memory, 0, 0x1000, 1),
        Ok(RequestedOperand {
            raw: Some(-1),
            present: true,
            effective: 0
        })
    );
    assert_eq!(
        requested_effective_memo(
            &memory,
            0,
            0x1000,
            1,
            &definition::DefinitionMemo::default()
        ),
        Ok(0)
    );
    assert_eq!(
        requested_operand_at(&memory, 0, 0x1000, 2),
        Ok(RequestedOperand::default())
    );
}

#[test]
fn checked_snapshot_growth_reports_oom_and_preserves_previous_storage() {
    let capacity = SnapshotCapacity {
        nodes: 1,
        entries: 1,
    };
    let mut arena = SnapshotArena::try_new(capacity).unwrap();
    let error = arena
        .try_grow(SnapshotCapacity {
            nodes: usize::MAX,
            entries: 1,
        })
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::OutOfMemory);
    assert_eq!(arena.capacity(), capacity);
    assert_eq!(
        SnapshotArena::vacant().capacity(),
        SnapshotCapacity::default()
    );
}

struct SnapshotMemory {
    bytes: RefCell<Vec<u8>>,
    calls: Cell<usize>,
    failure: Option<usize>,
    failure_calls: Cell<usize>,
}

impl SnapshotMemory {
    fn new(count: u32, failure: Option<usize>) -> Self {
        let mut bytes = vec![0; 0x600 + count as usize * 16];
        let header = &mut bytes[engine_native_memory::country_identity::MODIFIER_ACCUMULATOR..][..16];
        header[..8].copy_from_slice(&0x1600_u64.to_le_bytes());
        header[8..12].copy_from_slice(&count.to_le_bytes());
        header[12..].copy_from_slice(&count.to_le_bytes());
        for (index, entry) in bytes[0x600..].chunks_exact_mut(16).enumerate() {
            entry[..4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            entry[8..].copy_from_slice(&(-(index as i64) * 15).to_le_bytes());
        }
        Self {
            bytes: RefCell::new(bytes),
            calls: Cell::new(0),
            failure,
            failure_calls: Cell::new(0),
        }
    }
}

impl Memory for SnapshotMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        if self.failure == Some(address) {
            destination.fill(0xff);
            self.failure_calls.set(self.failure_calls.get() + 1);
            return false;
        }
        SliceMemory {
            base: 0x1000,
            bytes: &self.bytes.borrow(),
        }
        .copy(address, destination)
    }
}

#[test]
fn arena_reuses_full_census_and_expires_observations_after_clear() {
    let memory = SnapshotMemory::new(70, None);
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 1,
        entries: 70,
    });
    let allocated = arena.allocated_bytes();
    let present = arena.capture(&memory, 0x1000, 70).unwrap();
    assert_eq!(
        arena.value(present),
        Ok(Value {
            raw: -1035,
            present: true,
        })
    );
    let calls = memory.calls.get();
    assert_eq!(calls, 71);
    assert_eq!(
        arena.entries.as_flattened(),
        &memory.bytes.borrow()[0x600..]
    );
    let absent = arena.capture(&memory, 0x1000, 71).unwrap();
    assert_eq!(arena.value(absent), Ok(Value::default()));
    assert!(arena.capture(&memory, 0x1000, 0).is_err());
    assert_eq!(memory.calls.get(), calls);
    memory.bytes.borrow_mut()[0x600 + 69 * 16 + 8..][..8].copy_from_slice(&17_i64.to_le_bytes());
    assert_eq!(arena.value(present).unwrap().raw, -1035);
    assert_eq!(memory.calls.get(), calls);
    assert_eq!(arena.allocated_bytes(), allocated);
    arena.clear();
    assert_eq!(
        arena.value(present),
        Err("modifier snapshot observation expired")
    );
    let current = arena.capture(&memory, 0x1000, 70).unwrap();
    assert_eq!(arena.value(current).unwrap().raw, 17);
    assert_eq!(memory.calls.get(), calls * 2);
    assert_eq!(arena.allocated_bytes(), allocated);
}

#[test]
fn explicit_address_capture_needs_no_country_offset_and_shares_country_nodes() {
    let mut bytes = [0u8; 64];
    bytes[..8].copy_from_slice(&0x30u64.to_le_bytes());
    bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
    bytes[32..36].copy_from_slice(&1u32.to_le_bytes());
    bytes[40..48].copy_from_slice(&i64::MIN.to_le_bytes());
    bytes[48..52].copy_from_slice(&3u32.to_le_bytes());
    bytes[56..64].copy_from_slice(&i64::MAX.to_le_bytes());
    let direct = SliceMemory {
        base: 0x10,
        bytes: &bytes,
    };
    let capacity = SnapshotCapacity {
        nodes: 2,
        entries: 4,
    };
    let mut arena = SnapshotArena::new(capacity);
    let observation = arena.capture_at(&direct, 0x10, 3).unwrap();
    assert_eq!(
        arena.value(observation),
        Ok(Value {
            raw: i64::MAX,
            present: true
        })
    );
    arena.clear();
    let memory = SnapshotMemory::new(2, None);
    arena.capture(&memory, 0x1000, 1).unwrap();
    let reads = memory.calls.get();
    assert_eq!(reads, 3);
    let observation = arena
        .capture_at(&memory, 0x1000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 2)
        .unwrap();
    assert_eq!(
        arena.value(observation),
        Ok(Value {
            raw: -15,
            present: true
        })
    );
    assert_eq!(memory.calls.get(), reads);
    assert_eq!(
        arena.required_capacity(),
        SnapshotCapacity {
            nodes: 1,
            entries: 2
        }
    );
    assert_eq!(
        arena.capture_at(&memory, 0x10, 0),
        Err("modifier lookup keys or output invalid")
    );
    assert_eq!(
        arena.capture(&memory, usize::MAX, 1),
        Err("modifier accumulator address overflow")
    );
}

#[test]
fn explicit_address_failure_keeps_its_prefix_and_allows_a_later_source() {
    let memory = SnapshotMemory::new(70, Some(0x1600 + 66 * 16));
    {
        let mut bytes = memory.bytes.borrow_mut();
        bytes[0x20..0x28].copy_from_slice(&0x1600u64.to_le_bytes());
        bytes[0x28..0x2c].copy_from_slice(&2u32.to_le_bytes());
        bytes[0x2c..0x30].copy_from_slice(&2u32.to_le_bytes());
    }
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 2,
        entries: 72,
    });
    assert_eq!(
        arena.capture_at(&memory, 0x1000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 1),
        Err("modifier accumulator entry unreadable")
    );
    assert_eq!(arena.entries.len(), 66);
    assert_eq!(memory.calls.get(), 68);
    let later = arena.capture_at(&memory, 0x1020, 2).unwrap();
    assert_eq!(
        arena.value(later),
        Ok(Value {
            raw: -15,
            present: true,
        })
    );
    assert_eq!(arena.entries.len(), 68);
    assert_eq!(memory.calls.get(), 71);
    assert_eq!(
        memory.failure_calls.get(),
        1,
        "failed bytes are never retried"
    );
    let reads = memory.calls.get();
    assert_eq!(
        arena.capture_at(&memory, 0x1000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 2),
        Err("modifier accumulator entry unreadable")
    );
    assert_eq!(memory.calls.get(), reads);
}

#[test]
fn arena_retains_successful_prefix_and_never_retries_failed_entry() {
    let memory = SnapshotMemory::new(70, Some(0x1600 + 66 * 16));
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 1,
        entries: 70,
    });
    assert_eq!(
        arena.capture(&memory, 0x1000, 1),
        Err("modifier accumulator entry unreadable")
    );
    assert_eq!(arena.entries.len(), 66);
    let calls = memory.calls.get();
    assert_eq!(calls, 68);
    assert_eq!(
        arena.entries.as_flattened(),
        &memory.bytes.borrow()[0x600..0x600 + 66 * 16]
    );
    assert!(arena.capture(&memory, 0x1000, 65).is_err());
    assert_eq!(memory.calls.get(), calls);
    assert_eq!(memory.failure_calls.get(), 1);
}

#[test]
fn arena_malformed_unused_suffix_precedes_later_batch_fault() {
    let memory = SnapshotMemory::new(6, Some(0x1600 + 5 * 16));
    memory.bytes.borrow_mut()[0x600 + 3 * 16..][..4].copy_from_slice(&3_u32.to_le_bytes());
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 1,
        entries: 6,
    });
    assert_eq!(
        arena.capture(&memory, 0x1000, 1),
        Err("modifier accumulator keys not positive and increasing")
    );
    assert_eq!(arena.entries.len(), 5);
    assert_eq!(memory.calls.get(), 7);
    assert_eq!(memory.failure_calls.get(), 1);
    assert_eq!(
        arena.entries.as_flattened(),
        &memory.bytes.borrow()[0x600..0x600 + 5 * 16]
    );
    assert_eq!(
        arena.capture(&memory, 0x1000, 2),
        Err("modifier accumulator keys not positive and increasing")
    );
    assert_eq!(memory.calls.get(), 7);
}

#[test]
fn short_arena_grows_in_the_capture_and_try_grow_expires_its_observations() {
    let memory = SnapshotMemory::new(2, None);
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 1,
        entries: 1,
    });
    let present = arena.capture(&memory, 0x1000, 2).unwrap();
    assert_eq!(
        arena.value(present),
        Ok(Value {
            raw: -15,
            present: true,
        })
    );
    assert_eq!(memory.calls.get(), 3);
    assert_eq!(
        arena.capture(&memory, 0x2000, 1),
        Err("modifier accumulator header unreadable or invalid")
    );
    assert_eq!(memory.calls.get(), 4);
    let required = SnapshotCapacity {
        nodes: 2,
        entries: 2,
    };
    assert_eq!(arena.required_capacity(), required);
    assert_eq!(arena.capacity(), required);
    assert_eq!(arena.value(present).unwrap().raw, -15);
    arena.grow(required);
    assert_eq!(
        arena.value(present),
        Err("modifier snapshot observation expired")
    );
    assert_eq!(memory.calls.get(), 4);
}

#[test]
fn arena_caches_invalid_and_unreadable_header_failures_until_clear() {
    let memory = SnapshotMemory::new(2, None);
    memory.bytes.borrow_mut()[engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 8..][..4]
        .copy_from_slice(&1_u32.to_le_bytes());
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 1,
        entries: 2,
    });
    assert_eq!(
        arena.capture(&memory, 0x1000, 1),
        Err("modifier accumulator header unreadable or invalid")
    );
    memory.bytes.borrow_mut()[engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 8..][..4]
        .copy_from_slice(&2_u32.to_le_bytes());
    assert_eq!(
        arena.capture(&memory, 0x1000, 2),
        Err("modifier accumulator header unreadable or invalid")
    );
    assert_eq!(memory.calls.get(), 1);
    assert!(arena.entries.is_empty());
    arena.clear();
    let current = arena.capture(&memory, 0x1000, 2).unwrap();
    assert_eq!(arena.value(current).unwrap().raw, -15);
    assert_eq!(memory.calls.get(), 4);
    arena.clear();
    let memory = SnapshotMemory::new(2, Some(0x1000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR));
    for key in [1, 2] {
        assert_eq!(
            arena.capture(&memory, 0x1000, key),
            Err("modifier accumulator header unreadable or invalid")
        );
    }
    assert!(arena.entries.is_empty());
    assert_eq!(memory.calls.get(), 1);
    assert_eq!(memory.failure_calls.get(), 1);
}

#[test]
fn owned_snapshot_binary_lookup_matches_full_live_census() {
    let mut bytes = [0_u8; 96];
    bytes[..8].copy_from_slice(&0x1020_u64.to_le_bytes());
    bytes[8..12].copy_from_slice(&4_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&4_u32.to_le_bytes());
    for (index, (key, raw)) in [(1_u32, i64::MIN), (3, 0), (7, -17), (u32::MAX, i64::MAX)]
        .into_iter()
        .enumerate()
    {
        let at = 32 + index * 16;
        bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
    }
    let snapshot = &bytes[32..];
    assert_eq!(validate_snapshot(snapshot), Ok(()));
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    for key in [1, 2, 3, 4, 7, 8, u32::MAX - 1, u32::MAX] {
        assert_eq!(
            lookup_snapshot(snapshot, key),
            lookup_at(&memory, 0x1000, key)
        );
    }
    assert_eq!(validate_snapshot(&[]), Ok(()));
    assert_eq!(lookup_snapshot(&[], 1), Ok(Value::default()));
    assert!(lookup_snapshot(snapshot, 0).is_err());
    assert!(lookup_snapshot(&snapshot[..63], 1).is_err());
}

#[test]
fn owned_snapshot_rejects_invalid_unused_suffix_and_partial_entries() {
    let mut bytes = [0_u8; 48];
    bytes[..4].copy_from_slice(&2_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&5_u32.to_le_bytes());
    for suffix in [0_u32, 1, 5] {
        bytes[32..36].copy_from_slice(&suffix.to_le_bytes());
        assert!(validate_snapshot(&bytes).is_err());
    }
    bytes[32..36].copy_from_slice(&9_u32.to_le_bytes());
    assert_eq!(validate_snapshot(&bytes), Ok(()));
    for len in [1, 15, 17, 47] {
        assert!(validate_snapshot(&bytes[..len]).is_err());
    }
}

#[test]
fn optional_snapshot_hook_keeps_key_errors_presence_and_metadata_separate() {
    struct Hook;
    impl Memory for Hook {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
        fn modifier_accumulator_value(
            &self,
            address: usize,
            key: u32,
        ) -> Option<Result<Value, &'static str>> {
            assert_eq!(address, 0x1000);
            assert_ne!(key, 0, "invalid keys must not reach the hook");
            Some(match key {
                1 => Ok(Value {
                    raw: 123,
                    present: true,
                }),
                2 => Ok(Value::default()),
                _ => Err("snapshot fault"),
            })
        }
    }
    assert!(lookup_at(&Hook, 0x1000, 0).is_err());
    assert_eq!(
        lookup_at(&Hook, 0x1000, 1),
        Ok(Value {
            raw: 123,
            present: true
        })
    );
    assert_eq!(lookup_at(&Hook, 0x1000, 3), Err("snapshot fault"));
    assert_eq!(
        lookup_effective_at(&Hook, usize::MAX, 0x1000, 2),
        Ok(EffectiveValue::default())
    );
    assert!(lookup_effective_at(&Hook, usize::MAX, 0x1000, 1).is_err());
}

struct LookupBatches<'a, M> {
    source: &'a M,
    batches: RefCell<Vec<(usize, usize)>>,
}

impl<'a, M> LookupBatches<'a, M> {
    fn new(source: &'a M) -> Self {
        Self {
            source,
            batches: RefCell::new(Vec::new()),
        }
    }
}

impl<M: Memory> Memory for LookupBatches<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.source.copy(address, destination)
    }

    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let engine_native_memory::memory::RecordSources::Strided {
            base,
            stride,
            count,
        } = sources
        else {
            panic!("modifier entries must retain their exact array layout");
        };
        assert!((1..=SNAPSHOT_BATCH).contains(&count));
        assert_eq!(stride, 16);
        assert_eq!(layout.stride(), 16);
        assert_eq!(layout.field_count(), 1);
        let field = &layout.fields()[0];
        assert_eq!(
            (field.source_offset, field.destination_offset, field.len),
            (0, 0, 16)
        );
        assert_eq!(destination.len(), count * 16);
        self.batches.borrow_mut().push((base, count));
        self.source.copy_records(sources, layout, destination)
    }
}

#[test]
fn raw_lookup_batches_keep_signed_values_absence_and_cross_batch_matches() {
    let source = SnapshotMemory::new(130, None);
    {
        let mut bytes = source.bytes.borrow_mut();
        for (index, entry) in bytes[0x600..].chunks_exact_mut(16).enumerate() {
            entry[..4].copy_from_slice(&(index as u32 * 2 + 1).to_le_bytes());
        }
        bytes[0x608..0x610].copy_from_slice(&i64::MIN.to_le_bytes());
        let last = &mut bytes[0x600 + 129 * 16..][..16];
        last[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        last[8..].copy_from_slice(&i64::MAX.to_le_bytes());
    }
    let memory = LookupBatches::new(&source);
    let mut output = [Value::default(); 7];
    lookup_many(
        &memory,
        0x1000,
        &[1, 2, 127, 128, 129, 259, u32::MAX],
        &mut output,
    )
    .unwrap();
    assert_eq!(
        output.map(|value| (value.present, value.raw)),
        [
            (true, i64::MIN),
            (false, 0),
            (true, -945),
            (false, 0),
            (true, -960),
            (false, 0),
            (true, i64::MAX)
        ]
    );
    assert_eq!(source.calls.get(), 131);
    assert_eq!(memory.batches.borrow().len(), 3);
}

#[test]
fn raw_lookup_batches_validate_positive_order_across_boundaries_and_unused_suffixes() {
    for (index, key) in [
        (63, 63_u32),
        (64, 64),
        (64, 63),
        (65, 65),
        (129, 129),
        (129, 1),
    ] {
        let source = SnapshotMemory::new(130, None);
        source.bytes.borrow_mut()[0x600 + index * 16..][..4].copy_from_slice(&key.to_le_bytes());
        let memory = LookupBatches::new(&source);
        let mut output = [Value::default()];
        assert_eq!(
            lookup_many(&memory, 0x1000, &[1], &mut output),
            Err("modifier accumulator keys not positive and increasing")
        );
        assert_eq!(
            output,
            [Value {
                raw: 0,
                present: true
            }]
        );
        let batch_end = ((index / SNAPSHOT_BATCH + 1) * SNAPSHOT_BATCH).min(130);
        assert_eq!(source.calls.get(), 1 + batch_end);
    }
}

#[test]
fn raw_lookup_batches_preserve_the_first_error_and_only_its_good_output_prefix() {
    for (bad, failed) in [
        (0, None),
        (1, Some(5)),
        (5, Some(1)),
        (63, Some(64)),
        (64, Some(63)),
        (64, Some(64)),
        (65, Some(66)),
        (129, None),
    ] {
        let source = SnapshotMemory::new(130, failed.map(|index| 0x1600 + index * 16));
        source.bytes.borrow_mut()[0x600 + bad * 16..][..4].fill(0);
        let keys = [1_u32, 2, 64, 65, 66, 129, 130, u32::MAX];
        let mut output = [Value {
            raw: 77,
            present: true,
        }; 8];
        let first_error = failed.map_or(bad, |index| index.min(bad));
        let reason = if failed.is_none_or(|index| bad < index) {
            "modifier accumulator keys not positive and increasing"
        } else {
            "modifier accumulator entry unreadable"
        };
        assert_eq!(
            lookup_many(&source, 0x1000, &keys, &mut output),
            Err(reason)
        );
        for (key, value) in keys.into_iter().zip(output) {
            assert_eq!(
                value,
                if (key as usize - 1) < first_error {
                    Value {
                        raw: -(i64::from(key) - 1) * 15,
                        present: true,
                    }
                } else {
                    Value::default()
                }
            );
        }
        let batch_end = ((first_error / SNAPSHOT_BATCH + 1) * SNAPSHOT_BATCH).min(130);
        let attempted = failed.map_or(batch_end, |index| (index + 1).min(batch_end));
        assert_eq!(source.calls.get(), 1 + attempted);
        assert_eq!(
            source.failure_calls.get(),
            usize::from(failed.is_some_and(|index| index < batch_end))
        );
    }
}

#[test]
fn raw_lookup_batches_ignore_partial_failed_entries_and_never_retry_them() {
    struct Partial<'a> {
        source: &'a SnapshotMemory,
        failed: usize,
        written: usize,
        reads: RefCell<Vec<usize>>,
    }
    impl Memory for Partial<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            if address == self.failed {
                if self.written != 0 {
                    assert!(self.source.copy(address, &mut destination[..self.written]));
                }
                return false;
            }
            self.source.copy(address, destination)
        }
    }
    for failed in [0, 63, 64, 129] {
        for written in [0, 2, 16] {
            let source = SnapshotMemory::new(130, None);
            let partial = Partial {
                source: &source,
                failed: 0x1600 + failed * 16,
                written,
                reads: RefCell::new(Vec::new()),
            };
            let memory = LookupBatches::new(&partial);
            let keys = [1_u32, 64, 65, 130];
            let mut output = [Value::default(); 4];
            assert_eq!(
                lookup_many(&memory, 0x1000, &keys, &mut output),
                Err("modifier accumulator entry unreadable")
            );
            for (key, value) in keys.into_iter().zip(output) {
                assert_eq!(value.present, key as usize <= failed);
                assert_eq!(
                    value.raw,
                    if value.present {
                        -(i64::from(key) - 1) * 15
                    } else {
                        0
                    }
                );
            }
            let mut expected = vec![0x1000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR];
            expected.extend((0..=failed).map(|index| 0x1600 + index * 16));
            assert_eq!(*partial.reads.borrow(), expected);
        }
    }
}

#[test]
fn raw_lookup_batches_keep_ordered_budget_faults() {
    struct Limited<'a> {
        source: &'a SnapshotMemory,
        remaining: Cell<usize>,
        refused: Cell<bool>,
    }
    impl Memory for Limited<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if self.remaining.get() == 0 {
                self.refused.set(true);
                return false;
            }
            self.remaining.set(self.remaining.get() - 1);
            self.source.copy(address, destination)
        }
    }
    for (limit, bad, failed, reason, good, refused, calls) in [
        (
            0,
            None,
            None,
            "modifier accumulator header unreadable or invalid",
            0,
            true,
            0,
        ),
        (
            6,
            Some(1),
            None,
            "modifier accumulator keys not positive and increasing",
            1,
            true,
            6,
        ),
        (
            6,
            None,
            Some(2),
            "modifier accumulator entry unreadable",
            2,
            false,
            4,
        ),
        (
            65,
            None,
            None,
            "modifier accumulator entry unreadable",
            64,
            true,
            65,
        ),
    ] {
        let source = SnapshotMemory::new(70, failed.map(|index| 0x1600 + index * 16));
        if let Some(index) = bad {
            source.bytes.borrow_mut()[0x600 + index * 16..][..4].fill(0);
        }
        let limited = Limited {
            source: &source,
            remaining: Cell::new(limit),
            refused: Cell::new(false),
        };
        let memory = LookupBatches::new(&limited);
        let keys = [1_u32, 2, 65, 70];
        let mut output = [Value::default(); 4];
        assert_eq!(
            lookup_many(&memory, 0x1000, &keys, &mut output),
            Err(reason)
        );
        for (key, value) in keys.into_iter().zip(output) {
            assert_eq!(value.present, key as usize <= good);
        }
        assert_eq!(limited.refused.get(), refused);
        assert_eq!(source.calls.get(), calls);
    }
}

#[test]
fn raw_lookup_batches_reject_invalid_extents_before_entries_and_leave_invalid_outputs_alone() {
    for (data, capacity, count) in [
        (0_u64, 1_u32, 1_u32),
        (0x1600, 1, 2),
        (0x1600, u32::MAX, 1),
        (u64::MAX - 15, 2, 2),
    ] {
        let source = SnapshotMemory::new(2, None);
        {
            let mut bytes = source.bytes.borrow_mut();
            let header = &mut bytes[engine_native_memory::country_identity::MODIFIER_ACCUMULATOR..][..16];
            header[..8].copy_from_slice(&data.to_le_bytes());
            header[8..12].copy_from_slice(&capacity.to_le_bytes());
            header[12..].copy_from_slice(&count.to_le_bytes());
        }
        let memory = LookupBatches::new(&source);
        let mut output = [Value {
            raw: 77,
            present: true,
        }];
        assert_eq!(
            lookup_many(&memory, 0x1000, &[1], &mut output),
            Err("modifier accumulator header unreadable or invalid")
        );
        assert_eq!(output, [Value::default()]);
        assert_eq!(source.calls.get(), 1);
        assert!(memory.batches.borrow().is_empty());
    }
    let source = SnapshotMemory::new(2, None);
    let marker = Value {
        raw: 77,
        present: true,
    };
    for keys in [vec![0], vec![2, 1], vec![1, 1]] {
        let mut output = vec![marker; keys.len()];
        assert_eq!(
            lookup_many_at(&source, 0x1000, &keys, &mut output),
            Err("modifier lookup keys or output invalid")
        );
        assert!(output.iter().all(|value| *value == marker));
    }
    assert_eq!(
        lookup_many_at(&source, 0x1000, &[1], &mut []),
        Err("modifier lookup keys or output invalid")
    );
    assert_eq!(lookup_many_at(&source, usize::MAX, &[], &mut []), Ok(()));
    assert_eq!(source.calls.get(), 0);
}

#[test]
fn resolves_multiple_keys_exactly_and_checks_the_unselected_suffix() {
    let mut bytes = vec![0; 0x700];
    bytes[engine_native_memory::country_identity::MODIFIER_ACCUMULATOR..engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 8]
        .copy_from_slice(&0x1600_u64.to_le_bytes());
    bytes[engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 8..engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 12]
        .copy_from_slice(&3_u32.to_le_bytes());
    bytes[engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 12..engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 16]
        .copy_from_slice(&3_u32.to_le_bytes());
    for (index, (key, raw)) in [(2_u32, i64::MIN), (5, 0), (9, i64::MAX)]
        .into_iter()
        .enumerate()
    {
        let at = 0x600 + index * 16;
        bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
    }
    let mut output = [Value::default(); 4];
    lookup_many(
        &SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        0x1000,
        &[2, 4, 5, 9],
        &mut output,
    )
    .unwrap();
    assert_eq!(
        output,
        [
            Value {
                raw: i64::MIN,
                present: true
            },
            Value::default(),
            Value {
                raw: 0,
                present: true
            },
            Value {
                raw: i64::MAX,
                present: true
            }
        ]
    );
    bytes[0x620..0x624].copy_from_slice(&5_u32.to_le_bytes());
    assert!(
        lookup(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0x1000,
            2
        )
        .is_err()
    );
}

#[test]
fn arbitrary_accumulator_preserves_presence_and_absence_without_metadata() {
    let mut bytes = [0_u8; 64];
    bytes[..8].copy_from_slice(&0x1020_u64.to_le_bytes());
    bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&2_u32.to_le_bytes());
    bytes[32..36].copy_from_slice(&2_u32.to_le_bytes());
    bytes[40..48].copy_from_slice(&i64::MIN.to_le_bytes());
    bytes[48..52].copy_from_slice(&5_u32.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        lookup_at(&memory, 0x1000, 2),
        Ok(Value {
            raw: i64::MIN,
            present: true
        })
    );
    assert_eq!(
        lookup_at(&memory, 0x1000, 5),
        Ok(Value {
            raw: 0,
            present: true
        })
    );
    assert_eq!(lookup_at(&memory, 0x1000, 3), Ok(Value::default()));
    assert_eq!(
        lookup_effective_at(&memory, usize::MAX, 0x1000, 3),
        Ok(EffectiveValue::default())
    );
    assert!(lookup_effective_at(&memory, usize::MAX, 0x1000, 5).is_err());
    assert_eq!(lookup_many_at(&memory, usize::MAX, &[], &mut []), Ok(()));
    for keys in [vec![0], vec![5, 2], vec![2, 2]] {
        assert!(
            lookup_many_at(
                &memory,
                0x1000,
                &keys,
                &mut vec![Value::default(); keys.len()]
            )
            .is_err()
        );
    }
}

mod read_only;

#[test]
fn snapshot_population_demand_excludes_allocator_spare_capacity() {
    let memory = SnapshotMemory::new(5_001, None);
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 1,
        entries: 0,
    });
    engine_allocation::reserve_exact(&mut arena.entries, 30_000).unwrap();
    arena.capture(&memory, 0x1000, 1).unwrap();
    assert_eq!(arena.capacity().entries, 5_001);
    assert_eq!(arena.required_capacity().entries, 5_001);
    let demand = arena.capacity();
    for _ in 0..20 {
        arena.try_grow(demand).unwrap();
        assert_eq!(arena.capacity(), demand);
    }
}

#[test]
fn demand_counts_entries_read_not_a_header_count_nothing_backs() {
    let memory = SnapshotMemory::new(70, None);
    let claimed = i32::MAX as u32;
    memory.bytes.borrow_mut()[engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 8..][..8]
        .copy_from_slice(&(u64::from(claimed) | u64::from(claimed) << 32).to_le_bytes());
    let mut arena = SnapshotArena::new(SnapshotCapacity {
        nodes: 1,
        entries: 0,
    });
    assert_eq!(
        arena.capture(&memory, 0x1000, 1),
        Err("modifier accumulator entry unreadable")
    );
    assert_eq!(arena.required_capacity().entries, 70);
    assert_eq!(arena.capacity().entries, 70);
}

#[test]
fn requested_lookup_does_not_read_unselected_values_or_discard_sibling_keys() {
    struct Selected;
    impl engine_native_memory::Memory for Selected {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            let bytes = match (at, out.len()) {
                (0x100, 8) => 0x200_u64.to_le_bytes().to_vec(),
                (0x10c, 4) => 2_i32.to_le_bytes().to_vec(),
                (0x200, 4) => 10_u32.to_le_bytes().to_vec(),
                (0x210, 4) => 20_u32.to_le_bytes().to_vec(),
                (0x208, 8) => 123_i64.to_le_bytes().to_vec(),
                _ => return false,
            };
            out.copy_from_slice(&bytes);
            true
        }
    }
    assert_eq!(
        super::lookup_requested_at(&Selected, 0x100, 10),
        Ok(super::Value {
            present: true,
            raw: 123
        })
    );
    assert!(super::lookup_requested_at(&Selected, 0x100, 20).is_err());
    assert_eq!(
        super::lookup_requested_at(&Selected, 0x100, 15),
        Ok(super::Value::default())
    );
}

#[test]
fn requested_lookup_does_not_require_the_capacity_word() {
    struct WithoutCapacity(i32);
    impl engine_native_memory::Memory for WithoutCapacity {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            match (at, out.len()) {
                (0x100, 8) => out.copy_from_slice(&0x200_u64.to_le_bytes()),
                (0x10c, 4) => out.copy_from_slice(&self.0.to_le_bytes()),
                (0x200, 4) => out.copy_from_slice(&7_u32.to_le_bytes()),
                (0x208, 8) => out.copy_from_slice(&123_i64.to_le_bytes()),
                _ => return false,
            }
            true
        }
    }
    assert_eq!(
        super::lookup_requested_at(&WithoutCapacity(0), 0x100, 7),
        Ok(super::Value::default())
    );
    assert_eq!(
        super::lookup_requested_at(&WithoutCapacity(1), 0x100, 7),
        Ok(super::Value {
            present: true,
            raw: 123
        })
    );
}
