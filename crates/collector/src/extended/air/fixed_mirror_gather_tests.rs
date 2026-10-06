use super::*;
use std::cell::{Cell, RefCell};

struct TraceMemory<'a> {
    bytes: &'a [u8],
    blocked: std::ops::Range<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
    gathers: Cell<usize>,
    changing_address: Option<usize>,
}

impl<'a> TraceMemory<'a> {
    fn new(bytes: &'a [u8], blocked: std::ops::Range<usize>) -> Self {
        Self {
            bytes,
            blocked,
            reads: RefCell::new(Vec::new()),
            gathers: Cell::new(0),
            changing_address: None,
        }
    }
}

struct ScalarFields<'a, 'b>(&'a TraceMemory<'b>);

impl Memory for ScalarFields<'_, '_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.0.copy(address, destination)
    }
}

impl Memory for TraceMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        let Some(end) = address.checked_add(destination.len()) else {
            return false;
        };
        if address < self.blocked.end && end > self.blocked.start {
            destination.fill(0xa5); // A failed field may have copied bytes.
            return false;
        }
        let copied = engine::memory::SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination);
        if copied && self.changing_address == Some(address) && !destination.is_empty() {
            destination[0] = destination[0].wrapping_add(self.reads.borrow().len() as u8);
        }
        copied
    }

    fn copy_scattered<const M: usize>(
        &self,
        addresses: [usize; M],
        destinations: &mut [&mut [u8]; M],
    ) -> usize {
        self.gathers.set(self.gathers.get() + 1);
        ScalarFields(self).copy_scattered(addresses, destinations)
    }
}

fn assert_logs_and_growth<const M: usize>(mut actual: RawLog<M>, mut scalar: RawLog<M>) {
    assert_eq!(actual.spans, scalar.spans);
    assert_eq!(actual.bytes, scalar.bytes);
    assert_eq!(actual.fault, scalar.fault);
    assert_eq!(actual.raw_bytes, scalar.raw_bytes);
    assert_eq!(actual.overflowed, scalar.overflowed);
    assert_eq!(actual.has_bulk, scalar.has_bulk);
    assert_eq!(actual.grow_for_next(), scalar.grow_for_next());
    assert_eq!(actual.spans.capacity(), scalar.spans.capacity());
    assert_eq!(actual.bytes.capacity(), scalar.bytes.capacity());
    // Exercise private growth requests through their existing public
    // effect before clear discards them.
    actual.grow();
    scalar.grow();
    assert_eq!(actual.spans.capacity(), scalar.spans.capacity());
    assert_eq!(actual.bytes.capacity(), scalar.bytes.capacity());
    let capacity = (actual.spans.capacity(), actual.bytes.capacity());
    actual.clear();
    scalar.clear();
    assert_eq!(actual.spans, scalar.spans);
    assert_eq!(actual.bytes, scalar.bytes);
    assert_eq!(actual.fault, scalar.fault);
    assert_eq!(actual.raw_bytes, scalar.raw_bytes);
    assert_eq!(actual.overflowed, scalar.overflowed);
    assert_eq!(actual.has_bulk, scalar.has_bulk);
    assert_eq!(actual.grow_for_next(), scalar.grow_for_next());
    assert_eq!((actual.spans.capacity(), actual.bytes.capacity()), capacity);
    assert_eq!((scalar.spans.capacity(), scalar.bytes.capacity()), capacity);
}

fn scalar_stats_channels(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl Memory,
    values: Array,
) -> bool {
    // Independent copy of the original scalar loop, not the gather layout.
    for (len, index) in [
        (32, 0_usize),
        (8, 10),
        (8, 17),
        (8, 24),
        (72, 36),
        (8, 51),
        (8, 58),
        (8, 65),
        (40, 77),
    ] {
        let Some(address) = index
            .checked_mul(engine::loop_history_record_fields::VALUE_STRIDE)
            .and_then(|offset| values.data.checked_add(offset))
        else {
            return false;
        };
        if sspan(log, memory, Some(address), len, STATS_MASK).is_none() {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    true
}

#[test]
fn stats_gather_matches_scalar_spans_reads_and_every_field_fault() {
    let bytes: Vec<_> = (0..0x2000).map(|index| (index % 251) as u8).collect();
    let values = Array {
        data: 0x1000,
        count: 89,
    };
    let fields = [
        (0, 32),
        (80, 8),
        (136, 8),
        (192, 8),
        (288, 72),
        (408, 8),
        (464, 8),
        (520, 8),
        (616, 40),
    ];
    let faults = std::iter::once(0..0).chain(fields.into_iter().flat_map(|(offset, len)| {
        (0..len).map(move |byte| values.data + offset + byte..values.data + offset + byte + 1)
    }));
    for fault in faults {
        let mut actual_memory = TraceMemory::new(&bytes, fault.clone());
        let mut scalar_memory = TraceMemory::new(&bytes, fault.clone());
        actual_memory.changing_address = Some(values.data);
        scalar_memory.changing_address = Some(values.data);
        let mut actual = RawLog::new(32, 512);
        let mut scalar = RawLog::new(32, 512);
        // Repeating the same address must retain both observations, even
        // after a source fault. No field or earlier fault is cached.
        for _ in 0..2 {
            assert_eq!(
                copy_stats_channels(&mut actual, &actual_memory, values),
                scalar_stats_channels(&mut scalar, &scalar_memory, values),
                "fault {fault:x?}"
            );
        }
        assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
        assert_eq!(actual_memory.gathers.get(), 2);
        if fault.is_empty() {
            let expected: Vec<_> = fields
                .into_iter()
                .cycle()
                .take(18)
                .map(|(offset, len)| (values.data + offset, len))
                .collect();
            assert_eq!(*actual_memory.reads.borrow(), expected);
            assert_eq!(actual.raw_bytes, [384]);
            assert_ne!(actual.bytes[0], actual.bytes[192]);
        }
        assert_logs_and_growth(actual, scalar);
    }
}

#[test]
fn stats_gather_preserves_scalar_capacity_admission_and_growth() {
    let bytes = vec![7; 0x2000];
    let values = Array {
        data: 0x1000,
        count: 89,
    };
    for spans in 0..=10 {
        for capacity in [
            0, 31, 32, 39, 40, 47, 48, 55, 56, 127, 128, 135, 136, 143, 144, 151, 152, 191, 192,
            193,
        ] {
            for prefill in [false, true] {
                let actual_memory = TraceMemory::new(&bytes, 0..0);
                let scalar_memory = TraceMemory::new(&bytes, 0..0);
                let mut actual = RawLog::new(spans, capacity);
                let mut scalar = RawLog::new(spans, capacity);
                if prefill {
                    assert_eq!(
                        actual.take(&actual_memory, 0x80, 1, STATS_MASK),
                        scalar.take(&scalar_memory, 0x80, 1, STATS_MASK)
                    );
                }
                assert_eq!(
                    copy_stats_channels(&mut actual, &actual_memory, values),
                    scalar_stats_channels(&mut scalar, &scalar_memory, values)
                );
                assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
                assert_logs_and_growth(actual, scalar);
            }
        }
    }
}

#[test]
fn history_owned_pass_preserves_every_record_fault_and_damage_continuation() {
    use super::air_history_test_kit as kit;
    let record_fields = [
        (kit::REC0, 26),
        (kit::REC0 + sair_wing_combat_data::ATTRIBUTED_COUNTRY, 4),
        (kit::REC0 + sair_wing_combat_data::COPIED_FLAG, 1),
    ];
    let faults: Vec<_> = std::iter::once(0..0)
        .chain(record_fields.into_iter().flat_map(|(address, len)| {
            (0..len).map(move |byte| address + byte..address + byte + 1)
        }))
        .chain([
            kit::REC0 + 0x20..kit::REC0 + 0x21,
            kit::REC0 + 0x38..kit::REC0 + 0x39,
            kit::E0..kit::E0 + 1,
            kit::E0 + 0x10..kit::E0 + 0x11,
        ])
        .collect();
    for fault in faults {
        for invalid_damage_header in [false, true] {
            let mut bytes = kit::fixture();
            if invalid_damage_header {
                let at = kit::REC0 + sair_wing_combat_data::FIRST_DAMAGE_CAPACITY;
                bytes[at..at + 4].fill(0);
            }
            let mut actual_memory = TraceMemory::new(&bytes, fault.clone());
            let mut direct_memory = TraceMemory::new(&bytes, fault.clone());
            actual_memory.changing_address = Some(kit::REC0 + 0x50);
            direct_memory.changing_address = Some(kit::REC0 + 0x50);
            let mut actual = AirHistoryJob::new();
            let mut previous_attribution = None;
            for _ in 0..2 {
                let direct = kit::direct(&direct_memory);
                acquire_air_history_pass(
                    &mut actual,
                    &actual_memory,
                    kit::BASE,
                    kit::GAME_STATE,
                    8192,
                    16_384,
                );
                assert!(actual.acquired);
                assert_eq!(actual.history_result, direct.history_outcome);
                assert_eq!(actual.damage_result, direct.damage_outcome);
                assert_eq!(
                    actual.history[..actual.history_result.len()],
                    direct.history[..direct.history_outcome.len()]
                );
                assert_eq!(
                    actual.damage[..actual.damage_result.len()],
                    direct.damage[..direct.damage_outcome.len()]
                );
                assert_eq!(*actual_memory.reads.borrow(), *direct_memory.reads.borrow());
                assert_eq!(actual_memory.gathers.get(), direct_memory.gathers.get());
                if fault.is_empty() {
                    let attribution = actual.history[0].attributed_country;
                    if let Some(previous) = previous_attribution {
                        assert_ne!(
                            attribution, previous,
                            "each pass must acquire fresh leaf values"
                        );
                    }
                    previous_attribution = Some(attribution);
                }
            }
            if fault.start == kit::E0 && !invalid_damage_header {
                assert!(
                    !actual_memory
                        .reads
                        .borrow()
                        .iter()
                        .any(|&(address, _)| address == kit::E0 + sdamage_entry::AMOUNT),
                    "failed damage head must not retry or acquire its uncompleted amount"
                );
            }
        }
    }
}

#[test]
fn history_owned_pass_preserves_native_capacity_prefixes() {
    use super::air_history_test_kit as kit;
    let bytes = kit::fixture();
    for history_cap in 0..=4 {
        for damage_cap in 0..=4 {
            let actual_memory = TraceMemory::new(&bytes, 0..0);
            let direct_memory = TraceMemory::new(&bytes, 0..0);
            let direct = kit::direct_with(&direct_memory, history_cap, damage_cap);
            let mut actual = AirHistoryJob::new();
            let capacity = (actual.history.capacity(), actual.damage.capacity());
            acquire_air_history_pass(
                &mut actual,
                &actual_memory,
                kit::BASE,
                kit::GAME_STATE,
                history_cap,
                damage_cap,
            );
            assert!(actual.acquired);
            assert_eq!(actual.history_result, direct.history_outcome);
            assert_eq!(actual.damage_result, direct.damage_outcome);
            assert_eq!(
                actual.history[..actual.history_result.len()],
                direct.history[..direct.history_outcome.len()]
            );
            assert_eq!(
                actual.damage[..actual.damage_result.len()],
                direct.damage[..direct.damage_outcome.len()]
            );
            assert_eq!(*actual_memory.reads.borrow(), *direct_memory.reads.borrow());
            assert_eq!(
                (actual.history.capacity(), actual.damage.capacity()),
                capacity,
                "no callback growth"
            );
        }
    }
}

#[test]
fn stats_mirror_gathers_preserve_invalid_addresses() {
    let bytes = vec![0; 1024];
    for address in [0, usize::MAX - 32, usize::MAX, 0x0000_8000_0000_0000] {
        let actual_memory = TraceMemory::new(&bytes, 0..0);
        let scalar_memory = TraceMemory::new(&bytes, 0..0);
        let mut actual = RawLog::new(32, 512);
        let mut scalar = RawLog::new(32, 512);
        let values = Array {
            data: address,
            count: 89,
        };
        assert_eq!(
            copy_stats_channels(&mut actual, &actual_memory, values),
            scalar_stats_channels(&mut scalar, &scalar_memory, values)
        );
        assert_eq!(*actual_memory.reads.borrow(), *scalar_memory.reads.borrow());
        assert_logs_and_growth(actual, scalar);
    }
}

#[test]
fn history_owned_pass_preserves_invalid_record_vector_addresses() {
    use super::air_history_test_kit as kit;
    for address in [0_u64, u64::MAX - 32, u64::MAX, 0x0000_8000_0000_0000] {
        let mut bytes = kit::fixture();
        let at = kit::R0 + cair_region_combat_data::FIRST_RECORDS;
        bytes[at..at + 8].copy_from_slice(&address.to_le_bytes());
        let actual_memory = TraceMemory::new(&bytes, 0..0);
        let direct_memory = TraceMemory::new(&bytes, 0..0);
        let direct = kit::direct(&direct_memory);
        let mut actual = AirHistoryJob::new();
        acquire_air_history_pass(
            &mut actual,
            &actual_memory,
            kit::BASE,
            kit::GAME_STATE,
            8192,
            16_384,
        );
        assert!(actual.acquired);
        assert_eq!(actual.history_result, direct.history_outcome);
        assert_eq!(actual.damage_result, direct.damage_outcome);
        assert!(matches!(actual.history_result, Outcome::Unavailable { .. }));
        assert_eq!(*actual_memory.reads.borrow(), *direct_memory.reads.borrow());
    }
}
