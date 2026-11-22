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
            destination.fill(0xa5);
            return false;
        }
        let copied = super::air_bases_test_kit::memory(self.bytes).copy(address, destination);
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

fn registration_wing_fields() -> [(usize, usize); 4] {
    use super::air_bases_test_kit as kit;
    [
        (kit::W0, 32),
        (kit::W0 + cair_wing::POOL, 8),
        (kit::W1, 32),
        (kit::W1 + cair_wing::POOL, 8),
    ]
}

fn registration_reads(memory: &TraceMemory<'_>) -> Vec<(usize, usize)> {
    let fields = registration_wing_fields();
    memory
        .reads
        .borrow()
        .iter()
        .copied()
        .filter(|read| fields.contains(read))
        .collect()
}

#[test]
fn registration_mirror_retains_every_wing_field_fault_prefix_without_retry() {
    use super::air_bases_test_kit as kit;
    let bytes = kit::fixture();
    let fields = registration_wing_fields();
    let faults =
        std::iter::once((fields.len(), 0..0)).chain(fields.into_iter().enumerate().flat_map(
            |(index, (address, size))| {
                (0..size).map(move |byte| (index, address + byte..address + byte + 1))
            },
        ));
    for (field_index, fault) in faults {
        let actual_memory = TraceMemory::new(&bytes, fault.clone());
        let direct_memory = TraceMemory::new(&bytes, fault.clone());
        let direct = kit::direct(&direct_memory);
        let (actual, report, job) = kit::split(&actual_memory);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&actual, &direct);
        let end = (field_index + 1).min(fields.len());
        assert_eq!(registration_reads(&actual_memory), fields[..end]);
        assert_eq!(registration_reads(&direct_memory), fields[..end]);
        let spans: Vec<_> = job
            .log
            .spans
            .iter()
            .filter(|span| fields.contains(&(span.addr, span.len as usize)))
            .map(|span| (span.addr, span.len as usize, span.ok))
            .collect();
        let expected: Vec<_> = fields[..end]
            .iter()
            .enumerate()
            .map(|(index, &(address, size))| (address, size, index != field_index))
            .collect();
        assert_eq!(spans, expected, "fault {fault:x?}");
        assert_eq!(job.log.fault, [false, !fault.is_empty(), false]);
        kit::assert_no_mandatory_growth(&job);
    }
}

#[test]
fn registration_mirror_copies_the_chunk_before_semantic_admission() {
    use super::air_bases_test_kit as kit;
    let fields = registration_wing_fields();
    for (address, value, reason) in [
        (kit::W0, 0_u64, "air pool wing vtable mismatch"),
        (
            kit::W0 + cair_wing::SECONDARY,
            0,
            "air pool wing vtable mismatch",
        ),
        (
            kit::W0 + cair_wing::POOL,
            kit::POOL0 as u64 + 0x100,
            "registered air wing backlink differs from containing pool",
        ),
        (
            kit::W0 + cair_wing::REFERENCE_TYPE,
            0,
            "registered air wing has a null reference handle",
        ),
    ] {
        let mut bytes = kit::fixture();
        kit::put64(&mut bytes, address, value);
        for fault in [
            0..0,
            fields[1].0..fields[1].0 + 1,
            fields[3].0..fields[3].0 + 1,
        ] {
            let actual_memory = TraceMemory::new(&bytes, fault.clone());
            let direct_memory = TraceMemory::new(&bytes, fault.clone());
            let direct = kit::direct(&direct_memory);
            let (actual, report, _) = kit::split(&actual_memory);
            assert!(report.raw_ok);
            kit::assert_matches_direct(&actual, &direct);
            assert_eq!(
                registration_reads(&actual_memory),
                registration_reads(&direct_memory)
            );
            assert_eq!(
                actual.regs.outcome,
                Outcome::Unavailable {
                    reason: if fault.start == fields[1].0 {
                        "air pool wing identity unreadable"
                    } else {
                        reason
                    }
                }
            );
        }
    }
    let mut bytes = kit::fixture();
    let header = kit::POOL0 + cair_wing_pool::WINGS;
    let vector = u64::from_le_bytes(bytes[header..header + 8].try_into().unwrap()) as usize;
    kit::put64(&mut bytes, vector + 8, 0);
    let actual_memory = TraceMemory::new(&bytes, 0..0);
    let direct_memory = TraceMemory::new(&bytes, 0..0);
    let direct = kit::direct(&direct_memory);
    let (actual, _, _) = kit::split(&actual_memory);
    kit::assert_matches_direct(&actual, &direct);
    assert_eq!(
        actual.regs.outcome,
        Outcome::Unavailable {
            reason: "air pool wing pointer unreadable or null"
        }
    );
    assert!(registration_reads(&actual_memory).is_empty());
    assert!(registration_reads(&direct_memory).is_empty());
}

#[test]
#[ignore = "run alone: the allocator-refusal harness owns process-global controls"]
fn registration_backlink_acquisition_uses_only_prepared_storage() {
    use super::air_bases_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let mut job = AirBasesJob::fixture();
    copy_air_bases_pass(
        &mut job,
        &memory,
        kit::BASE,
        kit::GAME_STATE,
        false,
        &mut [0; 3],
    );
    let spans = job.log.spans.clone();
    let payload = job.log.bytes.to_vec();
    collector_test_support::with_allocations_denied(|| {
        copy_air_bases_pass(
            &mut job,
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            false,
            &mut [0; 3],
        );
    });
    assert!(!job.log.overflowed && !job.log.fault.iter().any(|fault| *fault));
    assert_eq!(job.log.spans, spans);
    assert_eq!(job.log.bytes, payload);
    assert_eq!(job.registration_demand, Some([2, 2]));
}

fn scalar_stats_channels(
    log: &mut RawLog<AIR_STATS_N>,
    memory: &impl Memory,
    values: Array,
) -> bool {
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
            .checked_mul(engine_air_history::loop_history_record_fields::VALUE_STRIDE)
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
