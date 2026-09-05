use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

pub const D81_SNAPSHOT_MERGE_GAP_BYTES: u64 = 64;
pub const D81_SNAPSHOT_MAX_REGIONS: usize = 65_536;
pub const D81_SNAPSHOT_MAX_BYTES: usize = 128 * 1024 * 1024;
pub const D81_SNAPSHOT_MAX_TRACE_READS: usize = 8 * 1024 * 1024;

pub const D81_SNAPSHOT_REGION_CAP_CAUSE: &str = "d81_snapshot_region_cap";
pub const D81_SNAPSHOT_BYTE_CAP_CAUSE: &str = "d81_snapshot_byte_cap";
pub const D81_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE: &str = "d81_snapshot_address_overflow";
pub const D81_SNAPSHOT_REGION_MISS_CAUSE: &str = "d81_snapshot_region_miss";
pub const D81_SNAPSHOT_COPY_REFUSED_CAUSE: &str = "d81_snapshot_copy_refused";
pub const D81_SNAPSHOT_TRACE_CAP_CAUSE: &str = "d81_snapshot_trace_cap";
pub const D81_MUTATOR_COVERAGE_INCOMPLETE_CAUSE: &str = "d81_mutator_coverage_incomplete";
pub const D81_WRITER_IN_FLIGHT_CAUSE: &str = "d81_writer_in_flight";

pub trait Domain81Memory {
    fn read_into(&self, address: u64, out: &mut [u8]) -> bool;

    fn plan_reads(&self, _base: u64, _fields: &[(usize, usize)]) {}

    fn plan_indirect(&self, _read: Domain81IndirectRead) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Domain81IndirectRead {
    pub pointer_address: u64,
    pub address: u64,
    pub len: usize,
    pub count_address: Option<u64>,
    pub stride: usize,
    pub capacity: usize,
}

#[derive(Debug)]
struct Domain81IndirectRegion {
    read: Domain81IndirectRead,
    bytes: Box<[u8]>,
    address: u64,
    copied_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain81ReadTrace {
    pub address: u64,
    pub len: usize,
    pub succeeded: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain81AccessTrace {
    pub reads: Vec<Domain81ReadTrace>,
    pub known_failed: BTreeSet<(u64, usize)>,
    pub trace_overflowed: bool,
    pub indirect: Vec<Domain81IndirectRead>,
}

pub struct Domain81TracingMemory<'a, M: Domain81Memory + ?Sized> {
    inner: &'a M,
    trace: RefCell<Domain81AccessTrace>,
}

impl<'a, M: Domain81Memory + ?Sized> Domain81TracingMemory<'a, M> {
    pub fn new(inner: &'a M) -> Self {
        Self {
            inner,
            trace: RefCell::new(Domain81AccessTrace::default()),
        }
    }

    pub fn into_trace(self) -> Domain81AccessTrace {
        self.trace.into_inner()
    }
}

impl<M: Domain81Memory + ?Sized> Domain81Memory for Domain81TracingMemory<'_, M> {
    fn plan_indirect(&self, read: Domain81IndirectRead) {
        let mut trace = self.trace.borrow_mut();
        if trace.reads.len().saturating_add(trace.indirect.len()) >= D81_SNAPSHOT_MAX_TRACE_READS
            || trace.indirect.len() >= D81_SNAPSHOT_MAX_REGIONS
        {
            trace.trace_overflowed = true;
            return;
        }
        trace.indirect.push(read);
    }

    fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
        let succeeded = self.inner.read_into(address, out);
        let mut trace = self.trace.borrow_mut();
        if trace.reads.len().saturating_add(trace.indirect.len()) >= D81_SNAPSHOT_MAX_TRACE_READS {
            trace.trace_overflowed = true;
            return succeeded;
        }
        if trace.trace_overflowed {
            return succeeded;
        }
        trace.reads.push(Domain81ReadTrace {
            address,
            len: out.len(),
            succeeded,
        });
        if !succeeded {
            trace.known_failed.insert((address, out.len()));
        }
        succeeded
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain81SnapshotCaptureReceipt {
    pub region_count: u32,
    pub byte_count: u64,
    pub copied_region_count: u32,
    pub copied_byte_count: u64,
    pub failure_region_ordinal: Option<u32>,
}

#[derive(Debug)]
struct Domain81SnapshotRegion {
    address: u64,
    bytes: Box<[u8]>,
}

#[derive(Debug)]
pub struct Domain81SnapshotPlan {
    regions: Vec<Domain81SnapshotRegion>,
    known_failed: BTreeSet<(u64, usize)>,
    byte_count: usize,
    indirect: Vec<Domain81IndirectRegion>,
    indirect_order: Vec<usize>,
    max_indirect_bytes: usize,
    captured: bool,
}

impl Domain81SnapshotPlan {
    pub fn from_trace(trace: Domain81AccessTrace) -> Result<Self, &'static str> {
        Self::from_trace_with_limits(
            trace,
            D81_SNAPSHOT_MERGE_GAP_BYTES,
            D81_SNAPSHOT_MAX_REGIONS,
            D81_SNAPSHOT_MAX_BYTES,
        )
    }

    pub fn from_trace_with_limits(
        trace: Domain81AccessTrace,
        merge_gap: u64,
        max_regions: usize,
        max_bytes: usize,
    ) -> Result<Self, &'static str> {
        if trace.trace_overflowed {
            return Err(D81_SNAPSHOT_TRACE_CAP_CAUSE);
        }
        let mut recipes = trace.indirect;
        recipes.sort_unstable();
        recipes.dedup();
        let mut excluded = Vec::with_capacity(recipes.len());
        let mut indirect = Vec::with_capacity(recipes.len());
        let mut reserved = 0usize;
        for read in recipes {
            let end = read
                .address
                .checked_add(read.len as u64)
                .ok_or(D81_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE)?;
            let bytes = read
                .capacity
                .checked_mul(read.stride)
                .ok_or(D81_SNAPSHOT_BYTE_CAP_CAUSE)?;
            if read.stride == 0 || read.len > bytes {
                return Err(D81_SNAPSHOT_BYTE_CAP_CAUSE);
            }
            reserved = reserved
                .checked_add(bytes)
                .ok_or(D81_SNAPSHOT_BYTE_CAP_CAUSE)?;
            if reserved > max_bytes || indirect.len() >= max_regions {
                return Err(if reserved > max_bytes {
                    D81_SNAPSHOT_BYTE_CAP_CAUSE
                } else {
                    D81_SNAPSHOT_REGION_CAP_CAUSE
                });
            }
            excluded.push((read.address, end));
            indirect.push(Domain81IndirectRegion {
                read,
                bytes: vec![0; bytes].into_boxed_slice(),
                address: 0,
                copied_len: 0,
            });
        }
        excluded.sort_unstable();
        let mut covered: Vec<(u64, u64)> = Vec::with_capacity(excluded.len());
        for (start, end) in excluded {
            if let Some(last) = covered.last_mut().filter(|last| start <= last.1) {
                last.1 = last.1.max(end);
            } else {
                covered.push((start, end));
            }
        }
        // Replaced targets must not remain in a merged static copy after the engine frees them.
        let is_indirect = |address: u64, len: usize| {
            covered
                .partition_point(|(start, _)| *start <= address)
                .checked_sub(1)
                .is_some_and(|i| {
                    address
                        .checked_add(len as u64)
                        .is_some_and(|end| end <= covered[i].1)
                })
        };
        let mut ranges = trace
            .reads
            .into_iter()
            .filter(|read| read.succeeded && read.len != 0 && !is_indirect(read.address, read.len))
            .map(|read| {
                let len =
                    u64::try_from(read.len).map_err(|_| D81_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE)?;
                let end = read
                    .address
                    .checked_add(len)
                    .ok_or(D81_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE)?;
                Ok((read.address, end))
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        ranges.sort_unstable();

        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            match merged.last_mut() {
                Some(last)
                    if start <= last.1
                        || (start <= last.1.saturating_add(merge_gap)
                            && !covered
                                .partition_point(|(a, _)| *a < start)
                                .checked_sub(1)
                                .is_some_and(|i| covered[i].1 > last.1)) =>
                {
                    last.1 = last.1.max(end);
                }
                _ => merged.push((start, end)),
            }
        }
        if merged.len().saturating_add(indirect.len()) > max_regions {
            return Err(D81_SNAPSHOT_REGION_CAP_CAUSE);
        }

        let mut byte_count = reserved;
        let mut regions = Vec::with_capacity(merged.len());
        for (address, end) in merged {
            let len =
                usize::try_from(end - address).map_err(|_| D81_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE)?;
            byte_count = byte_count
                .checked_add(len)
                .ok_or(D81_SNAPSHOT_BYTE_CAP_CAUSE)?;
            if byte_count > max_bytes {
                return Err(D81_SNAPSHOT_BYTE_CAP_CAUSE);
            }
            regions.push(Domain81SnapshotRegion {
                address,
                bytes: vec![0; len].into_boxed_slice(),
            });
        }

        Ok(Self {
            regions,
            known_failed: trace.known_failed,
            byte_count,
            max_indirect_bytes: indirect
                .iter()
                .map(|region| region.bytes.len())
                .max()
                .unwrap_or(0),
            indirect_order: (0..indirect.len()).collect(),
            captured: false,
            indirect,
        })
    }

    pub fn region_count(&self) -> usize {
        self.regions.len() + self.indirect.len()
    }

    pub fn byte_count(&self) -> usize {
        self.byte_count
    }

    fn read_static(&self, address: u64, out: &mut [u8]) -> bool {
        let Some(end) = address.checked_add(out.len() as u64) else {
            return false;
        };
        let index = self
            .regions
            .partition_point(|region| region.address <= address);
        if let Some(region) = index.checked_sub(1).and_then(|i| self.regions.get(i)) {
            if end <= region.address.saturating_add(region.bytes.len() as u64) {
                let offset = (address - region.address) as usize;
                out.copy_from_slice(&region.bytes[offset..offset + out.len()]);
                return true;
            }
        }
        false
    }

    pub fn capture_with(
        &mut self,
        mut copy: impl FnMut(u64, &mut [u8]) -> bool,
    ) -> Domain81SnapshotCaptureReceipt {
        self.captured = false;
        let mut receipt = Domain81SnapshotCaptureReceipt {
            region_count: self.region_count().min(u32::MAX as usize) as u32,
            byte_count: self
                .regions
                .iter()
                .map(|region| region.bytes.len() as u64)
                .sum(),
            ..Domain81SnapshotCaptureReceipt::default()
        };
        for region in &mut self.indirect {
            region.copied_len = 0;
        }
        for (ordinal, region) in self.regions.iter_mut().enumerate() {
            if copy(region.address, &mut region.bytes) {
                receipt.copied_region_count = receipt.copied_region_count.saturating_add(1);
                receipt.copied_byte_count = receipt
                    .copied_byte_count
                    .saturating_add(region.bytes.len() as u64);
            } else if receipt.failure_region_ordinal.is_none() {
                receipt.failure_region_ordinal = Some(ordinal.min(u32::MAX as usize) as u32);
            }
        }
        if receipt.failure_region_ordinal.is_some() {
            return receipt;
        }
        for index in 0..self.indirect.len() {
            self.indirect[index].copied_len = 0;
            let read = self.indirect[index].read;
            let mut pointer = [0u8; 8];
            let mut count = [0u8; 4];
            // Target identity and extent belong to this capture's copied header.
            let pointer_ok = self.read_static(read.pointer_address, &mut pointer);
            let count_ok = read
                .count_address
                .map_or(true, |address| self.read_static(address, &mut count));
            let elements = if read.count_address.is_some() {
                u32::from_le_bytes(count) as usize
            } else {
                read.capacity
            };
            let address = u64::from_le_bytes(pointer);
            let len = elements.checked_mul(read.stride);
            receipt.byte_count = receipt
                .byte_count
                .saturating_add(len.unwrap_or(u64::MAX as usize) as u64);
            let region = &mut self.indirect[index];
            region.address = address;
            let succeeded = pointer_ok
                && count_ok
                && elements <= read.capacity
                && len.is_some_and(|len| {
                    address.checked_add(len as u64).is_some()
                        && (len == 0 || copy(address, &mut region.bytes[..len]))
                });
            if succeeded {
                region.copied_len = len.unwrap();
                receipt.copied_region_count += 1;
                receipt.copied_byte_count += region.copied_len as u64;
            } else if receipt.failure_region_ordinal.is_none() {
                receipt.failure_region_ordinal = Some((self.regions.len() + index) as u32);
            }
        }
        self.indirect_order
            .sort_unstable_by_key(|index| self.indirect[*index].address);
        let mut covering: Option<usize> = None;
        for &index in &self.indirect_order {
            let region = &self.indirect[index];
            if region.copied_len == 0 {
                continue;
            }
            let end = region.address + region.copied_len as u64;
            let first = self.regions.partition_point(|other| {
                other.address.saturating_add(other.bytes.len() as u64) <= region.address
            });
            let mut agrees = true;
            for other in &self.regions[first..] {
                if other.address >= end {
                    break;
                }
                let start = region.address.max(other.address);
                let stop = end.min(other.address + other.bytes.len() as u64);
                let a = (start - region.address) as usize;
                let b = (start - other.address) as usize;
                let len = (stop - start) as usize;
                agrees &= region.bytes[a..a + len] == other.bytes[b..b + len];
            }
            if let Some(previous) = covering {
                let other = &self.indirect[previous];
                let stop = end.min(other.address + other.copied_len as u64);
                if stop > region.address {
                    let a = (region.address - other.address) as usize;
                    let len = (stop - region.address) as usize;
                    agrees &= region.bytes[..len] == other.bytes[a..a + len];
                }
            }
            if covering.is_none_or(|previous| {
                let other = &self.indirect[previous];
                end > other.address + other.copied_len as u64
            }) {
                covering = Some(index);
            }
            if !agrees && receipt.failure_region_ordinal.is_none() {
                receipt.failure_region_ordinal = Some((self.regions.len() + index) as u32);
            }
        }
        self.captured = receipt.failure_region_ordinal.is_none();
        receipt
    }

    pub fn view(&self) -> Domain81SnapshotMemory<'_> {
        Domain81SnapshotMemory {
            plan: self,
            first_miss: Cell::new(None),
        }
    }
}

pub struct Domain81SnapshotMemory<'a> {
    plan: &'a Domain81SnapshotPlan,
    first_miss: Cell<Option<(u64, usize)>>,
}

impl Domain81SnapshotMemory<'_> {
    pub fn first_miss(&self) -> Option<(u64, usize)> {
        self.first_miss.get()
    }

    fn note_miss(&self, address: u64, len: usize) {
        if self.first_miss.get().is_none() {
            self.first_miss.set(Some((address, len)));
        }
    }
}

impl Domain81Memory for Domain81SnapshotMemory<'_> {
    fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
        let len = out.len();
        if len == 0 {
            return false;
        }
        let Some(end) = address.checked_add(len as u64) else {
            self.note_miss(address, len);
            return false;
        };
        if !self.plan.captured {
            self.note_miss(address, len);
            return false;
        }
        if self.plan.read_static(address, out) {
            return true;
        }
        let tail = self
            .plan
            .indirect_order
            .partition_point(|index| self.plan.indirect[*index].address <= address);
        for &index in self.plan.indirect_order[..tail].iter().rev() {
            let region = &self.plan.indirect[index];
            if address - region.address >= self.plan.max_indirect_bytes as u64 {
                break;
            }
            if end <= region.address.saturating_add(region.copied_len as u64) {
                let offset = (address - region.address) as usize;
                out.copy_from_slice(&region.bytes[offset..offset + len]);
                return true;
            }
        }
        if !self.plan.known_failed.contains(&(address, len)) {
            self.note_miss(address, len);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct FixtureMemory(BTreeMap<u64, u8>);

    impl Domain81Memory for FixtureMemory {
        fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
            for (offset, slot) in out.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + offset as u64)) else {
                    return false;
                };
                *slot = *value;
            }
            true
        }
    }

    fn indirect_fixture(count: u32, capacity: usize) -> (FixtureMemory, Domain81AccessTrace) {
        let mut memory = BTreeMap::new();
        for (address, bytes) in [
            (0x1000u64, 0x2000u64.to_le_bytes().to_vec()),
            (0x1008, count.to_le_bytes().to_vec()),
            (0x2000, vec![7; count as usize * 8]),
        ] {
            for (offset, value) in bytes.into_iter().enumerate() {
                memory.insert(address + offset as u64, value);
            }
        }
        let fixture = FixtureMemory(memory);
        let tracing = Domain81TracingMemory::new(&fixture);
        assert!(tracing.read_into(0x1000, &mut [0; 12]));
        assert!(tracing.read_into(0x2000, &mut vec![0; count as usize * 8]));
        tracing.plan_indirect(Domain81IndirectRead {
            pointer_address: 0x1000,
            address: 0x2000,
            len: count as usize * 8,
            count_address: Some(0x1008),
            stride: 8,
            capacity,
        });
        let trace = tracing.into_trace();
        (fixture, trace)
    }

    #[test]
    fn indirect_capture_follows_copied_pointer_and_count_with_bounded_growth() {
        let (mut fixture, trace) = indirect_fixture(1, 3);
        let mut plan = Domain81SnapshotPlan::from_trace_with_limits(trace, 64, 2, 36).unwrap();
        assert_eq!(plan.byte_count(), 36);
        assert!(!plan.view().read_into(0x1000, &mut [0; 8]));
        fixture.0.retain(|address, _| *address < 0x2000);
        for (address, bytes) in [
            (0x1000u64, 0x3000u64.to_le_bytes().to_vec()),
            (0x1008, 3u32.to_le_bytes().to_vec()),
            (0x3000, vec![19; 24]),
        ] {
            for (offset, value) in bytes.into_iter().enumerate() {
                fixture.0.insert(address + offset as u64, value);
            }
        }
        let mut addresses = Vec::new();
        let receipt = plan.capture_with(|address, out| {
            addresses.push(address);
            fixture.read_into(address, out)
        });
        assert_eq!(receipt.failure_region_ordinal, None);
        assert_eq!(receipt.byte_count, 36);
        assert_eq!(receipt.copied_byte_count, 36);
        assert!(
            !addresses.contains(&0x2000),
            "the freed target is not copied"
        );
        fixture.0.insert(0x3000, 99);
        let mut bytes = [0; 24];
        assert!(plan.view().read_into(0x3000, &mut bytes));
        assert_eq!(bytes, [19; 24]);
        fixture.0.insert(0x1008, 4);
        let receipt = plan.capture_with(|address, out| fixture.read_into(address, out));
        assert_eq!(receipt.failure_region_ordinal, Some(1));
        assert!(
            !plan.view().read_into(0x3000, &mut bytes),
            "a refused recopy cannot serve the previous image"
        );
        let (_, trace) = indirect_fixture(1, 3);
        assert_eq!(
            Domain81SnapshotPlan::from_trace_with_limits(trace, 64, 2, 35).unwrap_err(),
            D81_SNAPSHOT_BYTE_CAP_CAUSE
        );
    }

    #[test]
    fn indirect_copy_refuses_disagreement_with_an_overlapping_static_image() {
        let (mut fixture, mut trace) = indirect_fixture(1, 1);
        trace.reads.push(Domain81ReadTrace {
            address: 0x3000,
            len: 8,
            succeeded: true,
        });
        for (offset, byte) in 0x3000u64.to_le_bytes().into_iter().enumerate() {
            fixture.0.insert(0x1000 + offset as u64, byte);
        }
        for offset in 0..8 {
            fixture.0.insert(0x3000 + offset, 11);
        }
        let mut plan = Domain81SnapshotPlan::from_trace(trace).unwrap();
        let mut copies = 0;
        let receipt = plan.capture_with(|address, out| {
            let succeeded = fixture.read_into(address, out);
            if address == 0x3000 {
                copies += 1;
                if copies == 2 {
                    out.fill(12);
                }
            }
            succeeded
        });
        assert_eq!(copies, 2);
        assert!(receipt.failure_region_ordinal.is_some());
        assert!(!plan.view().read_into(0x3000, &mut [0; 8]));
    }

    #[test]
    fn capture_is_bounded_and_decode_cannot_escape_the_regions() {
        let fixture = FixtureMemory((0x1000..0x1100).map(|at| (at, at as u8)).collect());
        let tracing = Domain81TracingMemory::new(&fixture);
        let mut first = [0; 8];
        let mut second = [0; 4];
        assert!(tracing.read_into(0x1010, &mut first));
        assert!(tracing.read_into(0x1020, &mut second));
        let mut plan = Domain81SnapshotPlan::from_trace(tracing.into_trace()).unwrap();
        assert_eq!(plan.region_count(), 1);
        assert_eq!(plan.byte_count(), 20);

        let receipt = plan.capture_with(|address, out| fixture.read_into(address, out));
        assert_eq!(receipt.copied_byte_count, 20);
        let view = plan.view();
        let mut replayed = [0; 8];
        assert!(view.read_into(0x1010, &mut replayed));
        assert_eq!(replayed, first);
        assert!(!view.read_into(0x100f, &mut replayed));
        assert_eq!(view.first_miss(), Some((0x100f, 8)));
    }

    #[test]
    fn failed_live_reads_remain_failed_without_becoming_region_misses() {
        let fixture = FixtureMemory((0x2000..0x2010).map(|at| (at, at as u8)).collect());
        let tracing = Domain81TracingMemory::new(&fixture);
        let mut absent = [0; 4];
        assert!(!tracing.read_into(0x3000, &mut absent));
        let mut plan = Domain81SnapshotPlan::from_trace(tracing.into_trace()).unwrap();
        let receipt = plan.capture_with(|address, out| fixture.read_into(address, out));
        assert_eq!(receipt.failure_region_ordinal, None);
        let view = plan.view();
        assert!(!view.read_into(0x3000, &mut absent));
        assert_eq!(view.first_miss(), None);
    }

    #[test]
    fn trace_and_byte_caps_fail_closed_by_name() {
        let overflowed = Domain81AccessTrace {
            trace_overflowed: true,
            ..Domain81AccessTrace::default()
        };
        assert_eq!(
            Domain81SnapshotPlan::from_trace(overflowed).unwrap_err(),
            D81_SNAPSHOT_TRACE_CAP_CAUSE
        );
        let trace = Domain81AccessTrace {
            reads: vec![Domain81ReadTrace {
                address: 0x4000,
                len: 9,
                succeeded: true,
            }],
            ..Domain81AccessTrace::default()
        };
        assert_eq!(
            Domain81SnapshotPlan::from_trace_with_limits(trace, 0, 1, 8).unwrap_err(),
            D81_SNAPSHOT_BYTE_CAP_CAUSE
        );
    }
}
