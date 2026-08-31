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
    fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
        let succeeded = self.inner.read_into(address, out);
        let mut trace = self.trace.borrow_mut();
        if trace.reads.len() == D81_SNAPSHOT_MAX_TRACE_READS {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain81SnapshotRegionSpec {
    pub address: u64,
    pub len: usize,
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
        let mut ranges = trace
            .reads
            .into_iter()
            .filter(|read| read.succeeded && read.len != 0)
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
                Some(last) if start <= last.1.saturating_add(merge_gap) => {
                    last.1 = last.1.max(end);
                }
                _ => merged.push((start, end)),
            }
        }
        if merged.len() > max_regions {
            return Err(D81_SNAPSHOT_REGION_CAP_CAUSE);
        }

        let mut byte_count = 0usize;
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
        })
    }

    pub fn region_specs(&self) -> impl ExactSizeIterator<Item = Domain81SnapshotRegionSpec> + '_ {
        self.regions
            .iter()
            .map(|region| Domain81SnapshotRegionSpec {
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
    ) -> Domain81SnapshotCaptureReceipt {
        let mut receipt = Domain81SnapshotCaptureReceipt {
            region_count: self.regions.len().min(u32::MAX as usize) as u32,
            byte_count: self.byte_count.min(u64::MAX as usize) as u64,
            ..Domain81SnapshotCaptureReceipt::default()
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
        if len == 0 || self.plan.known_failed.contains(&(address, len)) {
            return false;
        }
        let Some(end) = address.checked_add(len as u64) else {
            self.note_miss(address, len);
            return false;
        };
        let index = self
            .plan
            .regions
            .partition_point(|region| region.address <= address);
        let Some(region) = index
            .checked_sub(1)
            .and_then(|index| self.plan.regions.get(index))
        else {
            self.note_miss(address, len);
            return false;
        };
        let Some(region_end) = region.address.checked_add(region.bytes.len() as u64) else {
            self.note_miss(address, len);
            return false;
        };
        if end > region_end {
            self.note_miss(address, len);
            return false;
        }
        let offset = (address - region.address) as usize;
        out.copy_from_slice(&region.bytes[offset..offset + len]);
        true
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
