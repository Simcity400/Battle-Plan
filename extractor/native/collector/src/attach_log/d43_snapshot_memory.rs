use super::d81_snapshot_memory::Domain81Memory;
use std::cell::Cell;
use std::collections::BTreeSet;

// Caps, from `runs/perf_d43_market/SNAPSHOT_DEMAND.json` (the walk over a
// world at P4's cardinalities in two layouts: objects packed, and objects
// scattered with 64..4160 bytes between them so no two merge) and
// `GATE_COST.json` (the gated copy of the worst legal plan at each cap
// against the 16 ms bar). A cap refuses the plan, with the demand it refused
// on the wire, and the reader stays on its live walk; it never truncates a
// copy. The headroom each constant carries over the scattered measurement is
// stated on it.
pub(super) const D43_SNAPSHOT_MERGE_GAP_BYTES: u64 = 64;
pub(super) const D43_SNAPSHOT_MAX_REGIONS: usize = 262_144;
pub(super) const D43_SNAPSHOT_MAX_BYTES: usize = 16 << 20;
pub(super) const D43_SNAPSHOT_MAX_TRACE_READS: usize = 4 << 20;
pub(super) const D43_SNAPSHOT_TRACE_RESERVE_READS: usize = 1 << 20;
/// Distinct operating-system regions a plan may span: one VirtualQuery each
/// inside the bracket, so the count is taken at plan time and the bracket's
/// query budget is the same number.
pub(super) const D43_SNAPSHOT_MAX_OS_REGIONS: u32 = 4_096;

pub(super) const D43_SNAPSHOT_REGION_CAP_CAUSE: &str = "d43_snapshot_region_cap";
pub(super) const D43_SNAPSHOT_BYTE_CAP_CAUSE: &str = "d43_snapshot_byte_cap";
pub(super) const D43_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE: &str = "d43_snapshot_address_overflow";
pub(super) const D43_SNAPSHOT_TRACE_CAP_CAUSE: &str = "d43_snapshot_trace_cap";
pub(super) const D43_SNAPSHOT_OS_REGION_CAP_CAUSE: &str = "d43_snapshot_os_region_cap";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Domain43SnapshotLimits {
    pub merge_gap: u64,
    pub max_regions: usize,
    pub max_bytes: usize,
    pub max_trace_reads: usize,
    pub trace_reserve_reads: usize,
    pub max_os_regions: u32,
}

impl Default for Domain43SnapshotLimits {
    fn default() -> Self {
        Self {
            merge_gap: D43_SNAPSHOT_MERGE_GAP_BYTES,
            max_regions: D43_SNAPSHOT_MAX_REGIONS,
            max_bytes: D43_SNAPSHOT_MAX_BYTES,
            max_trace_reads: D43_SNAPSHOT_MAX_TRACE_READS,
            trace_reserve_reads: D43_SNAPSHOT_TRACE_RESERVE_READS,
            max_os_regions: D43_SNAPSHOT_MAX_OS_REGIONS,
        }
    }
}

/// A refused plan, with the demand it would have had: regions (data and
/// probe) and bytes, 0 when the trace overflowed and no plan was attempted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Domain43SnapshotPlanRefusal {
    pub cause: &'static str,
    pub region_count: u64,
    pub byte_count: u64,
}

/// The wire code a plan refusal publishes (`d43_snapshot_plan_refusal_cause`).
pub(super) fn plan_refusal_code(cause: &str) -> u64 {
    match cause {
        D43_SNAPSHOT_REGION_CAP_CAUSE => 1,
        D43_SNAPSHOT_BYTE_CAP_CAUSE => 2,
        D43_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE => 3,
        D43_SNAPSHOT_TRACE_CAP_CAUSE => 4,
        D43_SNAPSHOT_OS_REGION_CAP_CAUSE => 5,
        _ => u64::MAX,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Domain43ReadTrace {
    pub address: u64,
    pub len: u32,
    pub succeeded: bool,
}

/// Every read the traced walk made, failed ones included. `reads` never grows
/// past the capacity it was created with: the walk that fills it runs inside
/// the clock bracket, and a `push` past capacity would allocate there (the
/// failed reads become probe regions at plan time, outside the bracket). An
/// overrun sets `overflowed` and the plan is refused by name.
#[derive(Debug)]
pub(super) struct Domain43AccessTrace {
    reads: Vec<Domain43ReadTrace>,
    overflowed: bool,
}

impl Domain43AccessTrace {
    pub fn with_capacity(reads: usize) -> Self {
        Self {
            reads: Vec::with_capacity(reads),
            overflowed: false,
        }
    }

    pub fn capacity(&self) -> usize {
        self.reads.capacity()
    }

    pub fn reads(&self) -> &[Domain43ReadTrace] {
        &self.reads
    }

    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    pub fn push(&mut self, read: Domain43ReadTrace) {
        if self.overflowed || self.reads.len() == self.reads.capacity() {
            self.overflowed = true;
            return;
        }
        self.reads.push(read);
    }

    fn note(&mut self, address: u64, len: usize, succeeded: bool) {
        let Ok(len) = u32::try_from(len) else {
            self.overflowed = true;
            return;
        };
        self.push(Domain43ReadTrace {
            address,
            len,
            succeeded,
        });
    }
}

pub(super) struct Domain43TracingMemory<'a, M: Domain81Memory + ?Sized> {
    inner: &'a M,
    trace: std::cell::RefCell<Domain43AccessTrace>,
}

impl<'a, M: Domain81Memory + ?Sized> Domain43TracingMemory<'a, M> {
    pub fn new(inner: &'a M, trace: Domain43AccessTrace) -> Self {
        Self {
            inner,
            trace: std::cell::RefCell::new(trace),
        }
    }

    pub fn into_trace(self) -> Domain43AccessTrace {
        self.trace.into_inner()
    }
}

impl<M: Domain81Memory + ?Sized> Domain81Memory for Domain43TracingMemory<'_, M> {
    fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
        let succeeded = self.inner.read_into(address, out);
        self.trace.borrow_mut().note(address, out.len(), succeeded);
        succeeded
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Domain43SnapshotRegionSpec {
    pub address: u64,
    pub len: usize,
    /// A read the arming walk could not make. Copied when it has become
    /// readable so the current bytes are served; tolerated when it is still
    /// unreadable so the read is refused again, without a layout miss.
    pub probe: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Domain43SnapshotCaptureReceipt {
    pub region_count: u32,
    pub byte_count: u64,
    pub copied_region_count: u32,
    pub copied_byte_count: u64,
    /// First DATA region that did not copy; a probe region never sets it.
    pub failure_region_ordinal: Option<u32>,
    pub probe_region_count: u32,
    pub probe_regions_unreadable: u32,
}

/// The regions one traced walk touched, merged, sorted, and backed by ONE
/// preallocated arena. `capture_with` fills the arena in place; a view answers
/// reads out of it and out of nothing else.
#[derive(Debug)]
pub(super) struct Domain43SnapshotPlan {
    starts: Vec<u64>,
    ends: Vec<u64>,
    offsets: Vec<usize>,
    probe: Vec<bool>,
    /// Per region: the nonzero ordinal of the operating-system region the
    /// copy confirmed it inside, 0 where it did not copy.
    copied: Vec<u32>,
    arena: Box<[u8]>,
    byte_count: usize,
    probe_region_count: u32,
    os_region_count: u32,
}

impl Domain43SnapshotPlan {
    pub fn from_trace(
        trace: Domain43AccessTrace,
        limits: Domain43SnapshotLimits,
    ) -> Result<Self, Domain43SnapshotPlanRefusal> {
        let refuse = |cause, region_count: usize, byte_count: u64| Domain43SnapshotPlanRefusal {
            cause,
            region_count: region_count as u64,
            byte_count,
        };
        if trace.overflowed {
            return Err(refuse(D43_SNAPSHOT_TRACE_CAP_CAUSE, 0, 0));
        }
        let Domain43AccessTrace { reads, .. } = trace;
        let mut ranges = Vec::with_capacity(reads.len());
        let mut failed: BTreeSet<(u64, u64)> = BTreeSet::new();
        for read in reads {
            if read.len == 0 {
                continue;
            }
            let end = read
                .address
                .checked_add(u64::from(read.len))
                .ok_or_else(|| refuse(D43_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE, 0, 0))?;
            if read.succeeded {
                ranges.push((read.address, end));
            } else {
                failed.insert((read.address, end));
            }
        }
        ranges.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for (start, end) in ranges {
            match merged.last_mut() {
                Some(last) if start <= last.1.saturating_add(limits.merge_gap) => {
                    last.1 = last.1.max(end);
                }
                _ => merged.push((start, end)),
            }
        }
        // Probe regions: the failed reads that touch no data region. One that
        // overlaps a data region is left out; a read there is served from the
        // data region or misses, and either answer is the live reader's.
        let overlaps_data = |start: u64, end: u64| {
            let index = merged.partition_point(|(data_start, _)| *data_start < end);
            index
                .checked_sub(1)
                .is_some_and(|i| merged[i].1 > start)
        };
        let mut probes: Vec<(u64, u64)> = Vec::new();
        for (start, end) in failed {
            if overlaps_data(start, end) {
                continue;
            }
            match probes.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ => probes.push((start, end)),
            }
        }
        let mut regions: Vec<(u64, u64, bool)> = Vec::with_capacity(merged.len() + probes.len());
        regions.extend(merged.iter().map(|(start, end)| (*start, *end, false)));
        regions.extend(probes.iter().map(|(start, end)| (*start, *end, true)));
        regions.sort_unstable();
        // The demand first, whole, so a refusal names what was asked for.
        let demand_bytes = regions
            .iter()
            .fold(0u64, |total, (start, end, _)| total.saturating_add(end - start));
        if regions.len() > limits.max_regions {
            return Err(refuse(D43_SNAPSHOT_REGION_CAP_CAUSE, regions.len(), demand_bytes));
        }
        if demand_bytes > limits.max_bytes as u64 {
            return Err(refuse(D43_SNAPSHOT_BYTE_CAP_CAUSE, regions.len(), demand_bytes));
        }
        let mut starts = Vec::with_capacity(regions.len());
        let mut ends = Vec::with_capacity(regions.len());
        let mut offsets = Vec::with_capacity(regions.len());
        let mut probe = Vec::with_capacity(regions.len());
        let mut byte_count = 0usize;
        for (start, end, is_probe) in regions.iter().copied() {
            let len = usize::try_from(end - start).map_err(|_| {
                refuse(D43_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE, regions.len(), demand_bytes)
            })?;
            offsets.push(byte_count);
            byte_count = byte_count
                .checked_add(len)
                .ok_or_else(|| refuse(D43_SNAPSHOT_BYTE_CAP_CAUSE, regions.len(), demand_bytes))?;
            starts.push(start);
            ends.push(end);
            probe.push(is_probe);
        }
        let copied = vec![0u32; starts.len()];
        Ok(Self {
            starts,
            ends,
            offsets,
            probe,
            copied,
            arena: vec![0u8; byte_count].into_boxed_slice(),
            byte_count,
            probe_region_count: probes.len().min(u32::MAX as usize) as u32,
            os_region_count: 0,
        })
    }

    /// Split every region at the boundaries `boundary` names: for a region
    /// `[start, start + len)` it answers `Some(b)` with `start < b < end` to
    /// cut there (and is asked again about `[b, end)`), `None` to keep the
    /// region whole. The arena and the byte count are unchanged; the region
    /// cap is re-checked.
    pub fn split_regions_at(
        mut self,
        limits: Domain43SnapshotLimits,
        mut boundary: impl FnMut(u64, usize) -> Option<u64>,
    ) -> Result<Self, Domain43SnapshotPlanRefusal> {
        let mut starts = Vec::with_capacity(self.starts.len());
        let mut ends = Vec::with_capacity(self.ends.len());
        let mut probe = Vec::with_capacity(self.probe.len());
        for index in 0..self.starts.len() {
            let (start, end, is_probe) = (self.starts[index], self.ends[index], self.probe[index]);
            let mut cursor = start;
            loop {
                match boundary(cursor, (end - cursor) as usize) {
                    Some(cut) if cut > cursor && cut < end => {
                        starts.push(cursor);
                        ends.push(cut);
                        probe.push(is_probe);
                        cursor = cut;
                    }
                    _ => {
                        starts.push(cursor);
                        ends.push(end);
                        probe.push(is_probe);
                        break;
                    }
                }
            }
        }
        if starts.len() > limits.max_regions {
            return Err(Domain43SnapshotPlanRefusal {
                cause: D43_SNAPSHOT_REGION_CAP_CAUSE,
                region_count: starts.len() as u64,
                byte_count: self.byte_count as u64,
            });
        }
        let mut offsets = Vec::with_capacity(starts.len());
        let mut byte_count = 0usize;
        for (start, end) in starts.iter().zip(&ends) {
            offsets.push(byte_count);
            byte_count += (end - start) as usize;
        }
        debug_assert_eq!(byte_count, self.byte_count);
        self.copied = vec![0u32; starts.len()];
        self.starts = starts;
        self.ends = ends;
        self.offsets = offsets;
        self.probe = probe;
        Ok(self)
    }

    pub fn region_count(&self) -> usize {
        self.starts.len()
    }

    pub fn byte_count(&self) -> usize {
        self.byte_count
    }

    pub fn probe_region_count(&self) -> u32 {
        self.probe_region_count
    }

    pub fn os_region_count(&self) -> u32 {
        self.os_region_count
    }

    pub fn note_os_region_count(&mut self, count: u32) {
        self.os_region_count = count;
    }

    pub fn region_specs(&self) -> impl ExactSizeIterator<Item = Domain43SnapshotRegionSpec> + '_ {
        self.starts
            .iter()
            .zip(&self.ends)
            .zip(&self.probe)
            .map(|((start, end), probe)| Domain43SnapshotRegionSpec {
                address: *start,
                len: (end - start) as usize,
                probe: *probe,
            })
    }

    fn region_slice_mut(&mut self, index: usize) -> &mut [u8] {
        let offset = self.offsets[index];
        let len = (self.ends[index] - self.starts[index]) as usize;
        &mut self.arena[offset..offset + len]
    }

    /// Allocation-free: every region is copied into its own slice of the
    /// arena. `copy` answers `Some(region)` -- the nonzero ordinal of the
    /// operating-system region it confirmed the piece inside -- or `None`. A
    /// data region that does not copy is the receipt's failure; a probe region
    /// that does not copy is counted and tolerated.
    pub fn capture_with(
        &mut self,
        mut copy: impl FnMut(u64, &mut [u8]) -> Option<u32>,
    ) -> Domain43SnapshotCaptureReceipt {
        let mut receipt = Domain43SnapshotCaptureReceipt {
            region_count: self.starts.len().min(u32::MAX as usize) as u32,
            byte_count: self.byte_count as u64,
            probe_region_count: self.probe_region_count,
            ..Domain43SnapshotCaptureReceipt::default()
        };
        for index in 0..self.starts.len() {
            let address = self.starts[index];
            let is_probe = self.probe[index];
            let slice = self.region_slice_mut(index);
            let region = copy(address, slice);
            let len = slice.len() as u64;
            debug_assert!(region != Some(0), "region ordinals start at one");
            self.copied[index] = region.unwrap_or(0);
            if region.is_some() {
                receipt.copied_region_count = receipt.copied_region_count.saturating_add(1);
                receipt.copied_byte_count = receipt.copied_byte_count.saturating_add(len);
            } else if is_probe {
                receipt.probe_regions_unreadable = receipt.probe_regions_unreadable.saturating_add(1);
            } else if receipt.failure_region_ordinal.is_none() {
                receipt.failure_region_ordinal = Some(index.min(u32::MAX as usize) as u32);
            }
        }
        receipt
    }

    pub fn view(&self) -> Domain43SnapshotMemory<'_> {
        Domain43SnapshotMemory {
            plan: self,
            first_miss: Cell::new(None),
            misses: Cell::new(0),
            last_hit: Cell::new(0),
            served: Cell::new(0),
        }
    }
}

/// A read-only view over one captured arena. Addresses are the engine's own;
/// the only translation is through the plan's region table, and an address the
/// table does not cover is a miss, never a live read. A read across a
/// plan-time cut is one read: address-adjacent regions are arena-adjacent, so
/// it is served whole when every piece copied inside ONE operating-system
/// region at the copy instant, and refused without a miss otherwise -- the
/// live gate's rule (`range_confirmed_good`: a range straddling two regions is
/// refused even when both are readable).
pub(super) struct Domain43SnapshotMemory<'a> {
    plan: &'a Domain43SnapshotPlan,
    first_miss: Cell<Option<(u64, usize)>>,
    misses: Cell<u64>,
    last_hit: Cell<usize>,
    served: Cell<u64>,
}

thread_local! {
    /// Free-running across frames and never reset, so two nested samplers
    /// cannot clear each other's baseline; every reader of it takes a delta.
    static COPY_SHORT_READS: Cell<u64> = const { Cell::new(0) };
}

/// How many reads this thread's snapshot views have failed to answer out of a
/// copy, ever. The walk cannot reach the view it is running under, so a site
/// that needs to know whether its own refusal came from a short copy brackets
/// this instead.
pub(super) fn copy_short_reads() -> u64 {
    COPY_SHORT_READS.with(Cell::get)
}

impl Domain43SnapshotMemory<'_> {
    pub fn first_miss(&self) -> Option<(u64, usize)> {
        self.first_miss.get()
    }

    /// Reads this view could not answer because the plan does not cover them.
    /// A read the plan covers but the copy refused (a probe still unreadable,
    /// or a range that straddled two operating-system regions at the copy
    /// instant) is the live gate's own answer and is not counted here.
    pub fn misses(&self) -> u64 {
        self.misses.get()
    }

    pub fn reads_served(&self) -> u64 {
        self.served.get()
    }

    /// The consecutive regions covering `[address, address + len)`, first and
    /// last; `None` where the plan does not cover it.
    fn locate(&self, address: u64, len: usize) -> Option<(usize, usize)> {
        let end = address.checked_add(len as u64)?;
        let plan = self.plan;
        let last = self.last_hit.get();
        let first = if last < plan.starts.len()
            && plan.starts[last] <= address
            && address < plan.ends[last]
        {
            last
        } else {
            let index = plan.starts.partition_point(|start| *start <= address).checked_sub(1)?;
            if address >= plan.ends[index] {
                return None;
            }
            self.last_hit.set(index);
            index
        };
        let mut tail = first;
        while end > plan.ends[tail] {
            let next = tail + 1;
            if next >= plan.starts.len() || plan.starts[next] != plan.ends[tail] {
                return None;
            }
            debug_assert_eq!(
                plan.offsets[next],
                plan.offsets[tail] + (plan.ends[tail] - plan.starts[tail]) as usize
            );
            tail = next;
        }
        Some((first, tail))
    }
}

impl Domain81Memory for Domain43SnapshotMemory<'_> {
    fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
        let len = out.len();
        if len == 0 {
            return false;
        }
        let Some((first, last)) = self.locate(address, len) else {
            if self.first_miss.get().is_none() {
                self.first_miss.set(Some((address, len)));
            }
            self.misses.set(self.misses.get().saturating_add(1));
            COPY_SHORT_READS.with(|reads| reads.set(reads.get().wrapping_add(1)));
            return false;
        };
        // Refused without a miss, as the live gate refuses: a region this
        // hour's copy did not fill (a probe still unreadable), or a read across
        // a cut whose pieces were two operating-system regions when copied.
        let region = self.plan.copied[first];
        if region == 0
            || self.plan.copied[first + 1..=last]
                .iter()
                .any(|piece| *piece != region)
        {
            return false;
        }
        let offset = self.plan.offsets[first] + (address - self.plan.starts[first]) as usize;
        out.copy_from_slice(&self.plan.arena[offset..offset + len]);
        self.served.set(self.served.get().wrapping_add(1));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct FixtureMemory(BTreeMap<u64, u8>);

    impl FixtureMemory {
        fn spanning(ranges: &[(u64, usize)]) -> Self {
            let mut bytes = BTreeMap::new();
            for (start, len) in ranges {
                for offset in 0..*len as u64 {
                    bytes.insert(start + offset, (start + offset) as u8);
                }
            }
            Self(bytes)
        }
    }

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

    fn limits(merge_gap: u64, max_regions: usize, max_bytes: usize) -> Domain43SnapshotLimits {
        Domain43SnapshotLimits {
            merge_gap,
            max_regions,
            max_bytes,
            ..Domain43SnapshotLimits::default()
        }
    }

    fn spec(address: u64, len: usize, probe: bool) -> Domain43SnapshotRegionSpec {
        Domain43SnapshotRegionSpec {
            address,
            len,
            probe,
        }
    }

    #[test]
    fn reads_merge_within_the_gap_and_split_beyond_it() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x100), (0x3000, 0x100)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        let mut buf = [0u8; 8];
        assert!(tracing.read_into(0x1000, &mut buf));
        assert!(tracing.read_into(0x1040, &mut buf));
        assert!(tracing.read_into(0x1090, &mut buf));
        assert!(tracing.read_into(0x3000, &mut buf));
        let plan = Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(64, 16, 4096))
            .unwrap();
        let specs: Vec<_> = plan.region_specs().collect();
        assert_eq!(
            specs,
            vec![
                spec(0x1000, 0x48, false),
                spec(0x1090, 8, false),
                spec(0x3000, 8, false),
            ]
        );
        assert_eq!(plan.byte_count(), 0x48 + 8 + 8);
        assert_eq!(plan.probe_region_count(), 0);
    }

    #[test]
    fn a_view_answers_only_what_was_copied_and_names_the_first_miss() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x100)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        let mut buf = [0u8; 8];
        assert!(tracing.read_into(0x1000, &mut buf));
        let mut plan =
            Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(0, 16, 4096)).unwrap();
        let receipt = plan.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        assert_eq!(receipt.copied_region_count, 1);
        assert_eq!(receipt.copied_byte_count, 8);
        assert_eq!(receipt.failure_region_ordinal, None);
        let view = plan.view();
        let mut out = [0u8; 8];
        assert!(view.read_into(0x1000, &mut out));
        assert_eq!(out, [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
        // A read the plan never covered is the miss.
        assert!(!view.read_into(0x1004, &mut out));
        assert_eq!(view.first_miss(), Some((0x1004, 8)));
        assert_eq!(view.reads_served(), 1);
    }

    /// kills: replaying an arming-hour failure for ever. A read the arming
    /// walk could not make is a PROBE region: refused again while it is still
    /// unreadable (no miss, so the plan is kept), served the hour it copies.
    #[test]
    fn a_failed_read_is_a_probe_region_that_serves_once_it_copies() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x100)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        let mut buf = [0u8; 8];
        assert!(tracing.read_into(0x1000, &mut buf));
        assert!(!tracing.read_into(0x2000, &mut buf));
        assert!(!tracing.read_into(0x2000, &mut buf));
        // A failed read overlapping a data region is not a probe: the data
        // region answers for it, or the read misses.
        assert!(!tracing.read_into(0x1004, &mut [0u8; 0x100]));
        let mut plan =
            Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(0, 16, 4096)).unwrap();
        assert_eq!(
            plan.region_specs().collect::<Vec<_>>(),
            vec![spec(0x1000, 8, false), spec(0x2000, 8, true)]
        );
        assert_eq!(plan.probe_region_count(), 1);
        assert_eq!(plan.byte_count(), 16);

        let receipt = plan.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        assert_eq!(receipt.copied_region_count, 1);
        assert_eq!(receipt.failure_region_ordinal, None, "a probe failure is tolerated");
        assert_eq!(receipt.probe_region_count, 1);
        assert_eq!(receipt.probe_regions_unreadable, 1);
        {
            let view = plan.view();
            assert!(!view.read_into(0x2000, &mut buf));
            assert_eq!(view.first_miss(), None, "an unreadable probe is not a miss");
            assert!(!view.read_into(0x1004, &mut [0u8; 0x100]));
            assert_eq!(view.first_miss(), Some((0x1004, 0x100)));
        }

        let healed = FixtureMemory::spanning(&[(0x1000, 0x100), (0x2000, 0x10)]);
        let receipt = plan.capture_with(|address, out| healed.read_into(address, out).then_some(1));
        assert_eq!(receipt.copied_region_count, 2);
        assert_eq!(receipt.probe_regions_unreadable, 0);
        let view = plan.view();
        assert!(view.read_into(0x2000, &mut buf));
        assert_eq!(buf, [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
        assert_eq!(view.first_miss(), None);
    }

    #[test]
    fn caps_refuse_by_name_and_never_truncate() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x100), (0x3000, 0x100)]);
        let trace = || {
            let tracing =
                Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
            let mut buf = [0u8; 8];
            assert!(tracing.read_into(0x1000, &mut buf));
            assert!(tracing.read_into(0x3000, &mut buf));
            assert!(!tracing.read_into(0x5000, &mut buf));
            tracing.into_trace()
        };
        // A refusal carries the demand it refused: three regions (two data,
        // one probe), twenty-four bytes.
        assert_eq!(
            Domain43SnapshotPlan::from_trace(trace(), limits(0, 2, 4096)).unwrap_err(),
            Domain43SnapshotPlanRefusal {
                cause: D43_SNAPSHOT_REGION_CAP_CAUSE,
                region_count: 3,
                byte_count: 24,
            }
        );
        assert_eq!(
            Domain43SnapshotPlan::from_trace(trace(), limits(0, 3, 23)).unwrap_err(),
            Domain43SnapshotPlanRefusal {
                cause: D43_SNAPSHOT_BYTE_CAP_CAUSE,
                region_count: 3,
                byte_count: 24,
            }
        );
        assert!(Domain43SnapshotPlan::from_trace(trace(), limits(0, 3, 24)).is_ok());
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(1));
        let mut buf = [0u8; 8];
        assert!(tracing.read_into(0x1000, &mut buf));
        assert!(tracing.read_into(0x3000, &mut buf));
        let overflowed = tracing.into_trace();
        assert!(overflowed.overflowed());
        assert_eq!(overflowed.reads().len(), 1);
        assert_eq!(
            Domain43SnapshotPlan::from_trace(overflowed, limits(0, 16, 4096))
                .unwrap_err()
                .cause,
            D43_SNAPSHOT_TRACE_CAP_CAUSE
        );
        let mut wrapped = Domain43AccessTrace::with_capacity(4);
        wrapped.push(Domain43ReadTrace {
            address: u64::MAX - 2,
            len: 8,
            succeeded: true,
        });
        assert_eq!(
            Domain43SnapshotPlan::from_trace(wrapped, limits(0, 16, 4096))
                .unwrap_err()
                .cause,
            D43_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE
        );
        assert_eq!(plan_refusal_code(D43_SNAPSHOT_OS_REGION_CAP_CAUSE), 5);
    }

    #[test]
    fn a_failed_region_copy_is_reported_by_ordinal() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x100), (0x3000, 0x100)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        let mut buf = [0u8; 8];
        assert!(tracing.read_into(0x1000, &mut buf));
        assert!(tracing.read_into(0x3000, &mut buf));
        let mut plan =
            Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(0, 16, 4096)).unwrap();
        let gone = FixtureMemory::spanning(&[(0x1000, 0x100)]);
        let receipt = plan.capture_with(|address, out| gone.read_into(address, out).then_some(1));
        assert_eq!(receipt.region_count, 2);
        assert_eq!(receipt.copied_region_count, 1);
        assert_eq!(receipt.failure_region_ordinal, Some(1));
        // The view never serves a region the copy did not fill.
        let view = plan.view();
        assert!(!view.read_into(0x3000, &mut buf));
        assert_eq!(view.first_miss(), None);
    }

    #[test]
    fn a_region_is_split_where_its_boundaries_are_named_and_the_arena_is_kept() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x100)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        let mut buf = [0u8; 8];
        for offset in (0..0x40).step_by(8) {
            assert!(tracing.read_into(0x1000 + offset, &mut buf));
        }
        assert!(!tracing.read_into(0x4000, &mut buf));
        let plan =
            Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(64, 16, 4096)).unwrap();
        assert_eq!(plan.region_count(), 2);
        let mut asked = Vec::new();
        let mut plan = plan
            .split_regions_at(limits(64, 16, 4096), |start, len| {
                asked.push((start, len));
                // Two boundaries inside the data region, one outside it.
                [0x1010u64, 0x1030, 0x2000]
                    .into_iter()
                    .find(|cut| *cut > start && *cut < start + len as u64)
            })
            .unwrap();
        assert_eq!(
            asked,
            vec![(0x1000, 0x40), (0x1010, 0x30), (0x1030, 0x10), (0x4000, 8)]
        );
        assert_eq!(
            plan.region_specs().collect::<Vec<_>>(),
            vec![
                spec(0x1000, 0x10, false),
                spec(0x1010, 0x20, false),
                spec(0x1030, 0x10, false),
                spec(0x4000, 8, true),
            ]
        );
        assert_eq!(plan.byte_count(), 0x48);
        let receipt = plan.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        assert_eq!(receipt.copied_region_count, 3);
        assert_eq!(receipt.probe_regions_unreadable, 1);
        let view = plan.view();
        assert!(view.read_into(0x1028, &mut buf));
        assert_eq!(buf, [0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F]);
        // A read across the cut is one read of the arena: both sides copied
        // inside one operating-system region (the cut's boundary was gone by
        // the copy instant), so it is served, as the live gate would.
        assert!(view.read_into(0x100C, &mut buf));
        assert_eq!(buf, [0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13]);
        assert!(view.read_into(0x1000, &mut [0u8; 0x40]), "the region across both cuts");
        assert_eq!(view.first_miss(), None);
        // Past the last piece no region is adjacent: a miss.
        assert!(!view.read_into(0x103C, &mut buf));
        assert_eq!(view.first_miss(), Some((0x103C, 8)));
        drop(view);
        // Copied inside DIFFERENT operating-system regions: the live gate
        // refuses a range straddling two regions even when both are readable,
        // and so does the view -- without a miss. Within a piece, served.
        let receipt = plan.capture_with(|address, out| {
            memory
                .read_into(address, out)
                .then_some((address as u32 >> 4) + 1)
        });
        assert_eq!(receipt.copied_region_count, 3);
        let view = plan.view();
        assert!(!view.read_into(0x100C, &mut buf));
        assert!(!view.read_into(0x1000, &mut [0u8; 0x40]));
        assert_eq!(view.first_miss(), None, "the live gate's refusal is not a miss");
        assert!(view.read_into(0x1028, &mut buf));
        assert_eq!(buf, [0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F]);
        // The cap counts the split regions.
        let memory = FixtureMemory::spanning(&[(0x1000, 0x100)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        assert!(tracing.read_into(0x1000, &mut buf));
        let plan =
            Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(0, 1, 4096)).unwrap();
        assert_eq!(
            plan.split_regions_at(limits(0, 1, 4096), |start, _| Some(start + 4))
                .unwrap_err(),
            Domain43SnapshotPlanRefusal {
                cause: D43_SNAPSHOT_REGION_CAP_CAUSE,
                region_count: 2,
                byte_count: 8,
            }
        );
    }

    /// kills: a probe cut at an operating-system boundary whose replay
    /// misses -- the plan would be dropped every hour the failure persists.
    #[test]
    fn a_probe_cut_at_a_boundary_is_replayed_whole_and_serves_once_both_sides_copy() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x100)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        let mut buf = [0u8; 8];
        assert!(tracing.read_into(0x1000, &mut buf));
        assert!(!tracing.read_into(0x2FFC, &mut buf));
        let mut plan = Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(64, 16, 4096))
            .unwrap()
            .split_regions_at(limits(64, 16, 4096), |start, len| {
                (start < 0x3000 && start + len as u64 > 0x3000).then_some(0x3000)
            })
            .unwrap();
        assert_eq!(
            plan.region_specs().collect::<Vec<_>>(),
            vec![spec(0x1000, 8, false), spec(0x2FFC, 4, true), spec(0x3000, 4, true)]
        );
        let receipt = plan.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        assert_eq!(receipt.probe_regions_unreadable, 2);
        assert_eq!(receipt.failure_region_ordinal, None);
        {
            let view = plan.view();
            assert!(!view.read_into(0x2FFC, &mut buf), "unreadable on both sides");
            assert_eq!(view.first_miss(), None, "a persistent failure replays without a miss");
        }
        let half = FixtureMemory::spanning(&[(0x1000, 0x100), (0x2FFC, 4)]);
        let receipt = plan.capture_with(|address, out| half.read_into(address, out).then_some(1));
        assert_eq!(receipt.probe_regions_unreadable, 1);
        {
            let view = plan.view();
            assert!(!view.read_into(0x2FFC, &mut buf), "one side unreadable: the live read fails");
            assert_eq!(view.first_miss(), None);
        }
        let healed = FixtureMemory::spanning(&[(0x1000, 0x100), (0x2FFC, 8)]);
        // Both sides readable but still two operating-system regions: the
        // live gate refuses the straddling read, and so does the view.
        let receipt = plan.capture_with(|address, out| {
            healed
                .read_into(address, out)
                .then_some(if address < 0x3000 { 1 } else { 2 })
        });
        assert_eq!(receipt.probe_regions_unreadable, 0);
        {
            let view = plan.view();
            assert!(!view.read_into(0x2FFC, &mut buf), "two regions: refused");
            assert_eq!(view.first_miss(), None);
        }
        // One region now (the boundary between them gone): served.
        let receipt = plan.capture_with(|address, out| healed.read_into(address, out).then_some(1));
        assert_eq!(receipt.probe_regions_unreadable, 0);
        let view = plan.view();
        assert!(view.read_into(0x2FFC, &mut buf));
        assert_eq!(buf, [0xFC, 0xFD, 0xFE, 0xFF, 0x00, 0x01, 0x02, 0x03]);
        assert_eq!(view.first_miss(), None);
    }

    #[test]
    fn the_last_hit_cache_cannot_answer_past_its_own_region() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x10), (0x1100, 0x10)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        let mut buf = [0u8; 8];
        assert!(tracing.read_into(0x1000, &mut buf));
        assert!(tracing.read_into(0x1100, &mut buf));
        let mut plan =
            Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(0, 16, 4096)).unwrap();
        plan.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        let view = plan.view();
        assert!(view.read_into(0x1100, &mut buf));
        assert!(view.read_into(0x1000, &mut buf));
        assert!(!view.read_into(0x1008, &mut buf));
        assert_eq!(view.first_miss(), Some((0x1008, 8)));
    }
}
