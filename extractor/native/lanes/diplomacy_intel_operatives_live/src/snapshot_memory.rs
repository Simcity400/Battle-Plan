use crate::extractor::{Domain53Memory, ExtractError};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

pub const DOMAIN53_SNAPSHOT_MERGE_GAP_BYTES: u64 = 64;
pub const DOMAIN53_SNAPSHOT_MAX_REGIONS: usize = 65_536;
pub const DOMAIN53_SNAPSHOT_MAX_BYTES: usize = 128 * 1024 * 1024;
pub const DOMAIN53_SNAPSHOT_MAX_TRACE_READS: usize = 8 * 1024 * 1024;

pub const DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE: &str = "domain53_snapshot_region_cap";
pub const DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE: &str = "domain53_snapshot_byte_cap";
pub const DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE: &str =
    "domain53_snapshot_address_overflow";
pub const DOMAIN53_SNAPSHOT_REGION_MISS_CAUSE: &str = "domain53_snapshot_region_miss";
pub const DOMAIN53_SNAPSHOT_COPY_REFUSED_CAUSE: &str = "domain53_snapshot_copy_refused";
pub const DOMAIN53_SNAPSHOT_TRACE_CAP_CAUSE: &str = "domain53_snapshot_trace_cap";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain53ReadTrace {
    pub address: u64,
    pub len: usize,
    pub succeeded: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain53AccessTrace {
    pub reads: Vec<Domain53ReadTrace>,
    pub known_uncommitted: BTreeSet<u64>,
    pub known_failed: BTreeSet<(u64, usize)>,
    pub trace_overflowed: bool,
}

pub struct Domain53TracingMemory<'a, M: Domain53Memory + ?Sized> {
    inner: &'a M,
    trace: RefCell<Domain53AccessTrace>,
}

impl<'a, M: Domain53Memory + ?Sized> Domain53TracingMemory<'a, M> {
    pub fn new(inner: &'a M) -> Self {
        Self {
            inner,
            trace: RefCell::new(Domain53AccessTrace::default()),
        }
    }

    pub fn into_trace(self) -> Domain53AccessTrace {
        self.trace.into_inner()
    }

    fn note<T>(&self, address: u64, len: usize, result: &Result<T, ExtractError>) {
        let mut trace = self.trace.borrow_mut();
        if trace.reads.len() == DOMAIN53_SNAPSHOT_MAX_TRACE_READS {
            trace.trace_overflowed = true;
            return;
        }
        if trace.trace_overflowed {
            return;
        }
        trace.reads.push(Domain53ReadTrace {
            address,
            len,
            succeeded: result.is_ok(),
        });
        if result.is_err() {
            trace.known_failed.insert((address, len));
        }
    }
}

impl<M: Domain53Memory + ?Sized> Domain53Memory for Domain53TracingMemory<'_, M> {
    fn module_base(&self) -> u64 {
        self.inner.module_base()
    }

    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
        let result = self.inner.read_bytes(address, len);
        self.note(address, len, &result);
        result
    }

    fn read_into(&self, address: u64, buf: &mut [u8]) -> Result<(), ExtractError> {
        let result = self.inner.read_into(address, buf);
        self.note(address, buf.len(), &result);
        result
    }

    fn read_u8(&self, address: u64) -> Result<u8, ExtractError> {
        let result = self.inner.read_u8(address);
        self.note(address, 1, &result);
        result
    }

    fn read_u32(&self, address: u64) -> Result<u32, ExtractError> {
        let result = self.inner.read_u32(address);
        self.note(address, 4, &result);
        result
    }

    fn read_i32(&self, address: u64) -> Result<i32, ExtractError> {
        let result = self.inner.read_i32(address);
        self.note(address, 4, &result);
        result
    }

    fn read_u64(&self, address: u64) -> Result<u64, ExtractError> {
        let result = self.inner.read_u64(address);
        self.note(address, 8, &result);
        result
    }

    fn read_i64(&self, address: u64) -> Result<i64, ExtractError> {
        let result = self.inner.read_i64(address);
        self.note(address, 8, &result);
        result
    }

    fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
        self.inner.readable_ranges(lo, hi)
    }

    fn probe_not_committed(&self, address: u64) -> bool {
        let result = self.inner.probe_not_committed(address);
        if result && !self.trace.borrow().trace_overflowed {
            self.trace.borrow_mut().known_uncommitted.insert(address);
        }
        result
    }

    fn prefetch(&self, address: u64) {
        self.inner.prefetch(address);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain53SnapshotRegionSpec {
    pub address: u64,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain53SnapshotCaptureReceipt {
    pub region_count: u32,
    pub byte_count: u64,
    pub copied_region_count: u32,
    pub copied_byte_count: u64,
    pub failure_region_ordinal: Option<u32>,
}

#[derive(Debug)]
struct Domain53SnapshotRegion {
    address: u64,
    bytes: Box<[u8]>,
}

#[derive(Debug)]
pub struct Domain53SnapshotPlan {
    module_base: u64,
    regions: Vec<Domain53SnapshotRegion>,
    known_uncommitted: BTreeSet<u64>,
    known_failed: BTreeSet<(u64, usize)>,
    byte_count: usize,
}

impl Domain53SnapshotPlan {
    pub fn from_trace(
        module_base: u64,
        trace: Domain53AccessTrace,
    ) -> Result<Self, &'static str> {
        Self::from_trace_with_limits(
            module_base,
            trace,
            DOMAIN53_SNAPSHOT_MERGE_GAP_BYTES,
            DOMAIN53_SNAPSHOT_MAX_REGIONS,
            DOMAIN53_SNAPSHOT_MAX_BYTES,
        )
    }

    pub fn from_trace_with_limits(
        module_base: u64,
        trace: Domain53AccessTrace,
        merge_gap: u64,
        max_regions: usize,
        max_bytes: usize,
    ) -> Result<Self, &'static str> {
        if trace.trace_overflowed {
            return Err(DOMAIN53_SNAPSHOT_TRACE_CAP_CAUSE);
        }
        let mut ranges = trace
            .reads
            .into_iter()
            .filter(|read| read.succeeded && read.len != 0)
            .map(|read| {
                let len = u64::try_from(read.len)
                    .map_err(|_| DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE)?;
                let end = read
                    .address
                    .checked_add(len)
                    .ok_or(DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE)?;
                Ok((read.address, end))
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        ranges.sort_unstable();

        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            match merged.last_mut() {
                Some(last) if start <= last.1.saturating_add(merge_gap) => {
                    last.1 = last.1.max(end);
                }
                _ => merged.push((start, end)),
            }
        }
        if merged.len() > max_regions {
            return Err(DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE);
        }

        let mut byte_count = 0usize;
        let mut specs = Vec::with_capacity(merged.len());
        for (address, end) in merged {
            let len = usize::try_from(end - address)
                .map_err(|_| DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE)?;
            byte_count = byte_count
                .checked_add(len)
                .ok_or(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE)?;
            if byte_count > max_bytes {
                return Err(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE);
            }
            specs.push(Domain53SnapshotRegionSpec { address, len });
        }

        let regions = specs
            .into_iter()
            .map(|spec| Domain53SnapshotRegion {
                address: spec.address,
                bytes: vec![0; spec.len].into_boxed_slice(),
            })
            .collect();
        Ok(Self {
            module_base,
            regions,
            known_uncommitted: trace.known_uncommitted,
            known_failed: trace.known_failed,
            byte_count,
        })
    }

    pub fn region_specs(&self) -> impl ExactSizeIterator<Item = Domain53SnapshotRegionSpec> + '_ {
        self.regions.iter().map(|region| Domain53SnapshotRegionSpec {
            address: region.address,
            len: region.bytes.len(),
        })
    }

    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    pub fn byte_count(&self) -> usize {
        self.byte_count
    }

    pub fn capture_with(
        &mut self,
        mut copy: impl FnMut(u64, &mut [u8]) -> bool,
    ) -> Domain53SnapshotCaptureReceipt {
        let mut receipt = Domain53SnapshotCaptureReceipt {
            region_count: self.regions.len().min(u32::MAX as usize) as u32,
            byte_count: self.byte_count.min(u64::MAX as usize) as u64,
            ..Domain53SnapshotCaptureReceipt::default()
        };
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
        receipt
    }

    pub fn view(&self) -> Domain53SnapshotMemory<'_> {
        Domain53SnapshotMemory {
            plan: self,
            first_miss: Cell::new(None),
        }
    }
}

pub struct Domain53SnapshotMemory<'a> {
    plan: &'a Domain53SnapshotPlan,
    first_miss: Cell<Option<(u64, usize)>>,
}

impl Domain53SnapshotMemory<'_> {
    pub fn first_miss(&self) -> Option<(u64, usize)> {
        self.first_miss.get()
    }

    fn miss(&self, address: u64, len: usize) -> ExtractError {
        if self.first_miss.get().is_none() {
            self.first_miss.set(Some((address, len)));
        }
        ExtractError::MemoryRead { address, len }
    }

    fn read_into_snapshot(&self, address: u64, out: &mut [u8]) -> Result<(), ExtractError> {
        let len = out.len();
        if self.plan.known_failed.contains(&(address, len)) {
            return Err(ExtractError::MemoryRead { address, len });
        }
        let Some(end) = address.checked_add(len as u64) else {
            return Err(self.miss(address, len));
        };
        let index = self
            .plan
            .regions
            .partition_point(|region| region.address <= address);
        let Some(region) = index.checked_sub(1).and_then(|i| self.plan.regions.get(i)) else {
            return Err(self.miss(address, len));
        };
        let region_end = region.address.saturating_add(region.bytes.len() as u64);
        if end > region_end {
            return Err(self.miss(address, len));
        }
        let offset = (address - region.address) as usize;
        out.copy_from_slice(&region.bytes[offset..offset + len]);
        Ok(())
    }
}

impl Domain53Memory for Domain53SnapshotMemory<'_> {
    fn module_base(&self) -> u64 {
        self.plan.module_base
    }

    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
        let mut out = vec![0; len];
        self.read_into_snapshot(address, &mut out)?;
        Ok(out)
    }

    fn read_into(&self, address: u64, buf: &mut [u8]) -> Result<(), ExtractError> {
        self.read_into_snapshot(address, buf)
    }

    fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
        self.plan
            .regions
            .iter()
            .filter_map(|region| {
                let start = region.address.max(lo);
                let end = region
                    .address
                    .saturating_add(region.bytes.len() as u64)
                    .min(hi);
                (end > start).then_some((start, (end - start) as usize))
            })
            .collect()
    }

    fn probe_not_committed(&self, address: u64) -> bool {
        self.plan.known_uncommitted.contains(&address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FixtureMemory {
        module_base: u64,
        bytes: BTreeMap<u64, u8>,
        uncommitted: BTreeSet<u64>,
    }

    impl FixtureMemory {
        fn insert(&mut self, address: u64, bytes: &[u8]) {
            for (offset, byte) in bytes.iter().copied().enumerate() {
                self.bytes.insert(address + offset as u64, byte);
            }
        }
    }

    impl Domain53Memory for FixtureMemory {
        fn module_base(&self) -> u64 {
            self.module_base
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            (0..len)
                .map(|offset| {
                    self.bytes
                        .get(&(address + offset as u64))
                        .copied()
                        .ok_or(ExtractError::MemoryRead { address, len })
                })
                .collect()
        }

        fn probe_not_committed(&self, address: u64) -> bool {
            self.uncommitted.contains(&address)
        }
    }

    #[test]
    fn captured_pointers_are_opaque_and_reads_translate_only_through_regions() {
        let mut source = FixtureMemory {
            module_base: 0x1400_0000_0,
            ..FixtureMemory::default()
        };
        source.insert(0x1000, &0x7000u64.to_le_bytes());
        source.insert(0x7000, &[1, 2, 3, 4]);
        let traced = Domain53TracingMemory::new(&source);
        assert_eq!(traced.read_u64(0x1000).unwrap(), 0x7000);
        assert_eq!(traced.read_u32(0x7000).unwrap(), 0x0403_0201);
        let mut plan = Domain53SnapshotPlan::from_trace_with_limits(
            source.module_base,
            traced.into_trace(),
            0,
            2,
            12,
        )
        .unwrap();
        let receipt = plan.capture_with(|address, out| source.read_into(address, out).is_ok());
        assert_eq!(receipt.region_count, 2);
        assert_eq!(receipt.byte_count, 12);
        assert_eq!(receipt.failure_region_ordinal, None);

        source.insert(0x7000, &[9, 9, 9, 9]);
        let snapshot = plan.view();
        assert_eq!(snapshot.read_u64(0x1000).unwrap(), 0x7000);
        assert_eq!(snapshot.read_u32(0x7000).unwrap(), 0x0403_0201);
        assert!(snapshot.read_u8(0x7004).is_err());
    }

    #[test]
    fn plan_limits_refuse_instead_of_truncating() {
        let trace = Domain53AccessTrace {
            reads: vec![
                Domain53ReadTrace {
                    address: 0x1000,
                    len: 8,
                    succeeded: true,
                },
                Domain53ReadTrace {
                    address: 0x2000,
                    len: 8,
                    succeeded: true,
                },
            ],
            ..Domain53AccessTrace::default()
        };
        assert_eq!(
            Domain53SnapshotPlan::from_trace_with_limits(1, trace.clone(), 0, 1, 16)
                .unwrap_err(),
            DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE
        );
        assert_eq!(
            Domain53SnapshotPlan::from_trace_with_limits(1, trace, 0, 2, 15).unwrap_err(),
            DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE
        );
    }

    #[test]
    fn failed_source_reads_never_become_snapshot_regions() {
        let trace = Domain53AccessTrace {
            reads: vec![Domain53ReadTrace {
                address: 0x4000,
                len: 8,
                succeeded: false,
            }],
            ..Domain53AccessTrace::default()
        };
        let plan = Domain53SnapshotPlan::from_trace(1, trace).unwrap();
        assert_eq!(plan.region_count(), 0);
        assert!(plan.view().read_u64(0x4000).is_err());
    }

    #[test]
    fn trace_overflow_refuses_the_plan() {
        let trace = Domain53AccessTrace {
            trace_overflowed: true,
            ..Domain53AccessTrace::default()
        };
        assert_eq!(
            Domain53SnapshotPlan::from_trace(1, trace).unwrap_err(),
            DOMAIN53_SNAPSHOT_TRACE_CAP_CAUSE
        );
    }
}
