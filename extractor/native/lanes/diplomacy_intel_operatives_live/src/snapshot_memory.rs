use crate::extractor::{Domain53Memory, ExtractError};
use core::ptr;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
#[cfg(windows)]
use windows_sys::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};

pub const DOMAIN53_SNAPSHOT_PAGE_BYTES: usize = 4096;
pub const DOMAIN53_SNAPSHOT_LEGACY_MERGE_GAP_BYTES: u64 = 64;
/// Pages a plan may hold and the bytes they copy. From measured demand with
/// stated headroom: the P4 lower bound is 1,041 pages (runs/perf_w2_d53/
/// DEMAND_MEASUREMENT.json), the caps carry 3.9x over it, and the worst LEGAL
/// plan (both caps met in one plan) is measured on the production copy path
/// against the 16 ms bar in runs/perf_w2_arming/GATE_COST.json. A plan over
/// a cap is refused with its attempted demand, never truncated.
pub const DOMAIN53_SNAPSHOT_MAX_REGIONS: usize = 4_096;
pub const DOMAIN53_SNAPSHOT_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const DOMAIN53_SNAPSHOT_MAX_TRACE_READS: usize = 8 * 1024 * 1024;
/// Distinct operating-system regions a plan may span. Each costs one
/// VirtualQuery inside the bracket (the copy's guard-page gate), so the count
/// is taken on the plan-walk thread and refused above this; the bracket's
/// query budget is the same number, so a layout that fragmented after
/// planning refuses the copy instead of running past the bound.
pub const DOMAIN53_SNAPSHOT_MAX_OS_REGIONS: usize = 2_048;

pub const DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE: &str = "domain53_snapshot_region_cap";
pub const DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE: &str = "domain53_snapshot_byte_cap";
pub const DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE: &str = "domain53_snapshot_address_overflow";
pub const DOMAIN53_SNAPSHOT_REGION_MISS_CAUSE: &str = "domain53_snapshot_region_miss";
pub const DOMAIN53_SNAPSHOT_COPY_REFUSED_CAUSE: &str = "domain53_snapshot_copy_refused";
pub const DOMAIN53_SNAPSHOT_TRACE_CAP_CAUSE: &str = "domain53_snapshot_trace_cap";
pub const DOMAIN53_SNAPSHOT_OS_REGION_CAP_CAUSE: &str = "domain53_snapshot_os_region_cap";
pub const DOMAIN53_SNAPSHOT_GATE_BUDGET_CAUSE: &str = "domain53_snapshot_gate_budget";
pub const DOMAIN53_SNAPSHOT_ARENA_CAUSE: &str = "domain53_snapshot_arena_refused";

/// The plan's page buffers: ONE page-aligned, committed allocation holding
/// every planned page in plan order. Page-aligned, so a page buffer never
/// straddles two destination pages; committed and touched page by page on
/// the plan-walk thread, so the bracket copies into resident memory (a fresh
/// plan's first copy measured 28 ms for 32 MiB against 5 ms resident while
/// the buffers were lazily committed heap blocks). Taken from the operating
/// system directly rather than the global allocator, so the collector's
/// private heap and the test allocator share one allocation environment.
struct Domain53PageArena {
    base: *mut u8,
    pages: usize,
    #[cfg(not(windows))]
    backing: Vec<u8>,
}

// SAFETY: the arena owns its pages exclusively and hands them out only
// through `&self` / `&mut self` borrows.
unsafe impl Send for Domain53PageArena {}
unsafe impl Sync for Domain53PageArena {}

impl core::fmt::Debug for Domain53PageArena {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Domain53PageArena")
            .field("base", &self.base)
            .field("pages", &self.pages)
            .finish()
    }
}

impl Domain53PageArena {
    fn new(pages: usize) -> Option<Self> {
        let bytes = pages.checked_mul(DOMAIN53_SNAPSHOT_PAGE_BYTES)?;
        if pages == 0 {
            return Some(Self {
                base: ptr::null_mut(),
                pages: 0,
                #[cfg(not(windows))]
                backing: Vec::new(),
            });
        }
        #[cfg(windows)]
        let mut arena = {
            // SAFETY: plain Win32 call asking for a fresh private read-write
            // commit; the result is checked before use.
            let base = unsafe {
                VirtualAlloc(ptr::null(), bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE)
            } as *mut u8;
            if base.is_null() {
                return None;
            }
            Self { base, pages }
        };
        #[cfg(not(windows))]
        let mut arena = {
            let mut backing = vec![0u8; bytes + DOMAIN53_SNAPSHOT_PAGE_BYTES];
            let start = backing.as_mut_ptr() as usize;
            let aligned =
                (start + DOMAIN53_SNAPSHOT_PAGE_BYTES - 1) & !(DOMAIN53_SNAPSHOT_PAGE_BYTES - 1);
            Self {
                base: aligned as *mut u8,
                pages,
                backing,
            }
        };
        arena.touch();
        Some(arena)
    }

    /// Write one byte into every page so each is committed AND resident
    /// before the bracket copies into it. Volatile so the stores are kept.
    fn touch(&mut self) {
        for index in 0..self.pages {
            // SAFETY: every page of `[base, base + pages * PAGE)` is owned,
            // committed and writable.
            unsafe { ptr::write_volatile(self.base.add(index * DOMAIN53_SNAPSHOT_PAGE_BYTES), 0) };
        }
    }

    fn page(&self, index: usize) -> &[u8] {
        assert!(index < self.pages);
        // SAFETY: in bounds of the owned allocation (asserted above).
        unsafe {
            core::slice::from_raw_parts(
                self.base.add(index * DOMAIN53_SNAPSHOT_PAGE_BYTES),
                DOMAIN53_SNAPSHOT_PAGE_BYTES,
            )
        }
    }

    fn page_mut(&mut self, index: usize) -> &mut [u8] {
        assert!(index < self.pages);
        // SAFETY: in bounds of the owned allocation (asserted above); `&mut
        // self` makes this the only live borrow of the page.
        unsafe {
            core::slice::from_raw_parts_mut(
                self.base.add(index * DOMAIN53_SNAPSHOT_PAGE_BYTES),
                DOMAIN53_SNAPSHOT_PAGE_BYTES,
            )
        }
    }

    fn span(&self) -> (usize, usize) {
        (self.base as usize, self.pages * DOMAIN53_SNAPSHOT_PAGE_BYTES)
    }
}

#[cfg(windows)]
impl Drop for Domain53PageArena {
    fn drop(&mut self) {
        if !self.base.is_null() {
            // SAFETY: `base` is the VirtualAlloc reservation taken in `new`;
            // MEM_RELEASE with size 0 releases the whole reservation.
            unsafe { VirtualFree(self.base.cast(), 0, MEM_RELEASE) };
        }
    }
}

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
pub struct Domain53SnapshotPlanDemand {
    pub trace_read_count: u64,
    pub successful_read_count: u64,
    pub legacy_region_count: u64,
    pub legacy_byte_count: u64,
    pub page_region_count: u64,
    pub page_byte_count: u64,
    /// Pages the walk could not read or found uncommitted. They are planned
    /// as PROBE pages: copied when they have become readable so the plan
    /// serves the current value, tolerated when they are still unreadable so
    /// the historical answer is replayed without a layout miss.
    pub probe_region_count: u64,
    /// Distinct operating-system regions the plan's pages span, counted on
    /// the plan-walk thread (0 when never counted).
    pub os_region_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain53SnapshotPlanRefusal {
    pub cause: &'static str,
    pub demand: Domain53SnapshotPlanDemand,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain53SnapshotCaptureReceipt {
    pub demand: Domain53SnapshotPlanDemand,
    pub region_count: u32,
    pub byte_count: u64,
    pub copied_region_count: u32,
    pub copied_byte_count: u64,
    /// First DATA page that did not copy; a probe page never sets it.
    pub failure_region_ordinal: Option<u32>,
    pub probe_region_count: u32,
    pub probe_regions_unreadable: u32,
    /// Region queries the bracket's gate issued, and whether it ran out of
    /// budget (every page after that is a copy failure).
    pub gate_queries: u32,
    pub gate_budget_exhausted: bool,
}

#[derive(Debug)]
struct Domain53SnapshotRegion {
    address: u64,
    probe: bool,
    copied: bool,
    /// The operating-system region the copy's gate confirmed this page in.
    /// A read may only be served from pages of ONE region: that is the live
    /// reader's whole-range rule, and the snapshot keeps it.
    region_key: u64,
}

#[derive(Debug)]
pub struct Domain53SnapshotPlan {
    module_base: u64,
    regions: Vec<Domain53SnapshotRegion>,
    /// Every region's page buffer, one page per region in region order.
    arena: Domain53PageArena,
    known_uncommitted: BTreeSet<u64>,
    known_failed: BTreeSet<(u64, usize)>,
    byte_count: usize,
    probe_region_count: u32,
    demand: Domain53SnapshotPlanDemand,
}

impl Domain53SnapshotPlan {
    pub fn from_trace(
        module_base: u64,
        trace: Domain53AccessTrace,
    ) -> Result<Self, Domain53SnapshotPlanRefusal> {
        Self::from_trace_with_limits(
            module_base,
            trace,
            DOMAIN53_SNAPSHOT_MAX_REGIONS,
            DOMAIN53_SNAPSHOT_MAX_BYTES,
        )
    }

    pub fn from_trace_with_limits(
        module_base: u64,
        trace: Domain53AccessTrace,
        max_regions: usize,
        max_bytes: usize,
    ) -> Result<Self, Domain53SnapshotPlanRefusal> {
        let trace_read_count = trace.reads.len() as u64;
        if trace.trace_overflowed {
            return Err(Domain53SnapshotPlanRefusal {
                cause: DOMAIN53_SNAPSHOT_TRACE_CAP_CAUSE,
                demand: Domain53SnapshotPlanDemand {
                    trace_read_count,
                    ..Domain53SnapshotPlanDemand::default()
                },
            });
        }
        let refuse = |cause, demand| Domain53SnapshotPlanRefusal { cause, demand };
        let successful_read_count = trace
            .reads
            .iter()
            .filter(|read| read.succeeded && read.len != 0)
            .count() as u64;
        let initial_demand = Domain53SnapshotPlanDemand {
            trace_read_count,
            successful_read_count,
            ..Domain53SnapshotPlanDemand::default()
        };
        let mut ranges = trace
            .reads
            .iter()
            .filter(|read| read.succeeded && read.len != 0)
            .map(|read| {
                let len = u64::try_from(read.len).map_err(|_| {
                    refuse(DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE, initial_demand)
                })?;
                let end = read.address.checked_add(len).ok_or_else(|| {
                    refuse(DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE, initial_demand)
                })?;
                Ok((read.address, end))
            })
            .collect::<Result<Vec<_>, Domain53SnapshotPlanRefusal>>()?;
        ranges.sort_unstable();

        let mut legacy_merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
        for &(start, end) in &ranges {
            match legacy_merged.last_mut() {
                Some(last)
                    if start
                        <= last
                            .1
                            .saturating_add(DOMAIN53_SNAPSHOT_LEGACY_MERGE_GAP_BYTES) =>
                {
                    last.1 = last.1.max(end);
                }
                _ => legacy_merged.push((start, end)),
            }
        }
        let legacy_byte_count = legacy_merged
            .iter()
            .try_fold(0u64, |total, (start, end)| total.checked_add(end - start))
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE, initial_demand))?;

        let page_mask = !(DOMAIN53_SNAPSHOT_PAGE_BYTES as u64 - 1);
        let mut page_intervals: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            let page_start = start & page_mask;
            let last_page = (end - 1) & page_mask;
            let page_end = last_page
                .checked_add(DOMAIN53_SNAPSHOT_PAGE_BYTES as u64)
                .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE, initial_demand))?;
            match page_intervals.last_mut() {
                Some(last) if page_start <= last.1 => last.1 = last.1.max(page_end),
                _ => page_intervals.push((page_start, page_end)),
            }
        }

        let page_region_count = page_intervals.iter().try_fold(0u64, |total, (start, end)| {
            total.checked_add((end - start) / DOMAIN53_SNAPSHOT_PAGE_BYTES as u64)
        });
        let page_region_count = page_region_count
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE, initial_demand))?;
        let page_byte_count = page_region_count
            .checked_mul(DOMAIN53_SNAPSHOT_PAGE_BYTES as u64)
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE, initial_demand))?;

        // Probe pages: what the walk could not read or found uncommitted,
        // less anything a successful read already covers.
        let covered = |page: u64| {
            let index = page_intervals.partition_point(|(start, _)| *start <= page);
            index
                .checked_sub(1)
                .is_some_and(|i| page < page_intervals[i].1)
        };
        let mut probe_pages: BTreeSet<u64> = BTreeSet::new();
        for &(address, len) in &trace.known_failed {
            if len == 0 {
                continue;
            }
            let end = address
                .checked_add(len as u64)
                .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE, initial_demand))?;
            let last_page = (end - 1) & page_mask;
            let mut page = address & page_mask;
            loop {
                if !covered(page) {
                    probe_pages.insert(page);
                }
                if page == last_page {
                    break;
                }
                page += DOMAIN53_SNAPSHOT_PAGE_BYTES as u64;
            }
        }
        for &address in &trace.known_uncommitted {
            let page = address & page_mask;
            if !covered(page) {
                probe_pages.insert(page);
            }
        }
        let probe_region_count = probe_pages.len() as u64;
        let total_region_count = page_region_count
            .checked_add(probe_region_count)
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE, initial_demand))?;
        let total_byte_count = total_region_count
            .checked_mul(DOMAIN53_SNAPSHOT_PAGE_BYTES as u64)
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE, initial_demand))?;
        let demand = Domain53SnapshotPlanDemand {
            trace_read_count,
            successful_read_count,
            legacy_region_count: legacy_merged.len() as u64,
            legacy_byte_count,
            page_region_count,
            page_byte_count,
            probe_region_count,
            os_region_count: 0,
        };
        if total_region_count > max_regions as u64 {
            return Err(refuse(DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE, demand));
        }
        if total_byte_count > max_bytes as u64 {
            return Err(refuse(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE, demand));
        }

        let mut specs: Vec<(u64, bool)> = Vec::with_capacity(total_region_count as usize);
        for (start, end) in page_intervals {
            let mut address = start;
            while address < end {
                specs.push((address, false));
                address += DOMAIN53_SNAPSHOT_PAGE_BYTES as u64;
            }
        }
        specs.extend(probe_pages.into_iter().map(|page| (page, true)));
        specs.sort_unstable();

        // The page buffers are committed and touched HERE, on the plan-walk
        // thread, so the bracket's copy faults nothing in.
        let arena = Domain53PageArena::new(specs.len())
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_ARENA_CAUSE, demand))?;
        let regions = specs
            .into_iter()
            .map(|(address, probe)| Domain53SnapshotRegion {
                address,
                probe,
                copied: false,
                region_key: 0,
            })
            .collect();
        Ok(Self {
            module_base,
            regions,
            arena,
            known_uncommitted: trace.known_uncommitted,
            known_failed: trace.known_failed,
            byte_count: total_byte_count as usize,
            probe_region_count: probe_region_count.min(u32::MAX as u64) as u32,
            demand,
        })
    }

    pub fn region_specs(&self) -> impl ExactSizeIterator<Item = Domain53SnapshotRegionSpec> + '_ {
        self.regions
            .iter()
            .map(|region| Domain53SnapshotRegionSpec {
                address: region.address,
                len: DOMAIN53_SNAPSHOT_PAGE_BYTES,
            })
    }

    /// The page buffers' address span `(base, bytes)`: page-aligned, one
    /// page per region in region order, committed and touched when the plan
    /// was built.
    pub fn page_buffer_span(&self) -> (usize, usize) {
        self.arena.span()
    }

    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    pub fn byte_count(&self) -> usize {
        self.byte_count
    }

    pub fn demand(&self) -> Domain53SnapshotPlanDemand {
        self.demand
    }

    pub fn note_os_region_count(&mut self, count: u64) {
        self.demand.os_region_count = count;
    }

    /// Copy every page. `copy` answers `Some(region_key)` when the page was
    /// copied inside the operating-system region `region_key` (the gate's
    /// confirmed region base), `None` when it was not copied.
    pub fn capture_with(
        &mut self,
        mut copy: impl FnMut(u64, &mut [u8]) -> Option<u64>,
    ) -> Domain53SnapshotCaptureReceipt {
        let mut receipt = Domain53SnapshotCaptureReceipt {
            demand: self.demand,
            region_count: self.regions.len().min(u32::MAX as usize) as u32,
            byte_count: self.byte_count.min(u64::MAX as usize) as u64,
            probe_region_count: self.probe_region_count,
            ..Domain53SnapshotCaptureReceipt::default()
        };
        let Self { regions, arena, .. } = self;
        for (ordinal, region) in regions.iter_mut().enumerate() {
            let key = copy(region.address, arena.page_mut(ordinal));
            region.copied = key.is_some();
            region.region_key = key.unwrap_or(0);
            if region.copied {
                receipt.copied_region_count = receipt.copied_region_count.saturating_add(1);
                receipt.copied_byte_count = receipt
                    .copied_byte_count
                    .saturating_add(DOMAIN53_SNAPSHOT_PAGE_BYTES as u64);
            } else if region.probe {
                receipt.probe_regions_unreadable = receipt.probe_regions_unreadable.saturating_add(1);
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

    /// The copied page holding `address`, if any: its ordinal (its page in
    /// the arena) and its region.
    fn copied_page_at(&self, address: u64) -> Option<(usize, &Domain53SnapshotRegion)> {
        let index = self
            .plan
            .regions
            .partition_point(|region| region.address <= address)
            .checked_sub(1)?;
        let region = self.plan.regions.get(index)?;
        let region_end = region
            .address
            .saturating_add(DOMAIN53_SNAPSHOT_PAGE_BYTES as u64);
        (region.copied && address < region_end).then_some((index, region))
    }

    /// Serve `out` from copied pages of ONE operating-system region only;
    /// false when any byte is not on one, or the range crosses two regions
    /// (the live reader's whole-range gate refuses that read, so the snapshot
    /// must not answer it).
    fn copy_from_regions(&self, address: u64, out: &mut [u8]) -> bool {
        let len = out.len();
        if address.checked_add(len as u64).is_none() {
            return false;
        }
        let mut cursor = address;
        let mut copied = 0usize;
        let mut key = None;
        while copied < len {
            let Some((index, region)) = self.copied_page_at(cursor) else {
                return false;
            };
            if key.is_some_and(|key| key != region.region_key) {
                return false;
            }
            key = Some(region.region_key);
            let page = self.plan.arena.page(index);
            let offset = (cursor - region.address) as usize;
            let chunk_len = (len - copied).min(page.len() - offset);
            out[copied..copied + chunk_len].copy_from_slice(&page[offset..offset + chunk_len]);
            cursor += chunk_len as u64;
            copied += chunk_len;
        }
        true
    }

    fn read_into_snapshot(&self, address: u64, out: &mut [u8]) -> Result<(), ExtractError> {
        let len = out.len();
        if self.copy_from_regions(address, out) {
            return Ok(());
        }
        if self.plan.known_failed.contains(&(address, len)) {
            // The walk saw this read fail and its probe page is still
            // unreadable this hour: the same answer, not a layout miss. Once
            // the page copies, the branch above serves the current bytes.
            return Err(ExtractError::MemoryRead { address, len });
        }
        Err(self.miss(address, len))
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
        let mut ranges: Vec<(u64, usize)> = Vec::new();
        let mut last_key = None;
        for region in self.plan.regions.iter().filter(|region| region.copied) {
            let start = region.address.max(lo);
            let end = region
                .address
                .saturating_add(DOMAIN53_SNAPSHOT_PAGE_BYTES as u64)
                .min(hi);
            if end <= start {
                continue;
            }
            match ranges.last_mut() {
                Some((last_start, last_len))
                    if last_start.saturating_add(*last_len as u64) == start
                        && last_key == Some(region.region_key) =>
                {
                    *last_len += (end - start) as usize;
                }
                _ => ranges.push((start, (end - start) as usize)),
            }
            last_key = Some(region.region_key);
        }
        ranges
    }

    fn probe_not_committed(&self, address: u64) -> bool {
        let page_copied = self.copied_page_at(address).is_some();
        if self.plan.known_uncommitted.contains(&address) {
            // Uncommitted when the walk looked; committed now iff its probe
            // page copied this hour.
            return !page_copied;
        }
        if page_copied {
            return false;
        }
        // Never probed by the planning walk and on no copied page: the walk
        // diverged from the plan. Answer the refusing way and record the miss.
        self.miss(address, 0);
        true
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
        fn insert_page(&mut self, address: u64) {
            self.insert(address, &[0; DOMAIN53_SNAPSHOT_PAGE_BYTES]);
        }

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
        source.insert_page(0x1000);
        source.insert_page(0x7000);
        source.insert(0x1000, &0x7000u64.to_le_bytes());
        source.insert(0x7000, &[1, 2, 3, 4]);
        let traced = Domain53TracingMemory::new(&source);
        assert_eq!(traced.read_u64(0x1000).unwrap(), 0x7000);
        assert_eq!(traced.read_u32(0x7000).unwrap(), 0x0403_0201);
        let mut plan = Domain53SnapshotPlan::from_trace_with_limits(
            source.module_base,
            traced.into_trace(),
            2,
            2 * DOMAIN53_SNAPSHOT_PAGE_BYTES,
        )
        .unwrap();
        let receipt = plan.capture_with(|address, out| source.read_into(address, out).ok().map(|_| 1));
        assert_eq!(receipt.region_count, 2);
        assert_eq!(receipt.byte_count, 2 * DOMAIN53_SNAPSHOT_PAGE_BYTES as u64);
        assert_eq!(receipt.failure_region_ordinal, None);

        source.insert(0x7000, &[9, 9, 9, 9]);
        let snapshot = plan.view();
        assert_eq!(snapshot.read_u64(0x1000).unwrap(), 0x7000);
        assert_eq!(snapshot.read_u32(0x7000).unwrap(), 0x0403_0201);
        assert_eq!(snapshot.read_u8(0x7004).unwrap(), 0);
        assert!(snapshot.read_u8(0x8000).is_err());
    }

    /// Codex round 3, N1: the page buffers are one page-aligned arena in
    /// region order, so no buffer straddles two destination pages, and a
    /// plan with nothing to copy owns no arena at all.
    #[test]
    fn page_buffers_are_one_page_aligned_arena_in_region_order() {
        let mut source = FixtureMemory::default();
        for page in [0x1000u64, 0x5000, 0x9000] {
            source.insert_page(page);
            source.insert(page, &[(page >> 12) as u8; 8]);
        }
        let trace = Domain53AccessTrace {
            reads: [0x9000u64, 0x1000, 0x5000]
                .into_iter()
                .map(|address| Domain53ReadTrace {
                    address,
                    len: 8,
                    succeeded: true,
                })
                .collect(),
            ..Domain53AccessTrace::default()
        };
        let mut plan = Domain53SnapshotPlan::from_trace(1, trace).unwrap();
        let (base, bytes) = plan.page_buffer_span();
        assert_eq!(base % DOMAIN53_SNAPSHOT_PAGE_BYTES, 0, "page-aligned arena");
        assert_eq!(bytes, 3 * DOMAIN53_SNAPSHOT_PAGE_BYTES);
        assert_eq!(plan.region_count(), 3);
        // The bracket copies into page `ordinal` of the arena, in address
        // order.
        let mut destinations = Vec::new();
        let receipt = plan.capture_with(|address, out| {
            destinations.push((address, out.as_ptr() as usize));
            source.read_into(address, out).ok().map(|_| 1)
        });
        assert_eq!(receipt.copied_region_count, 3);
        assert_eq!(
            destinations,
            vec![(0x1000, base), (0x5000, base + 4096), (0x9000, base + 8192)]
        );
        let view = plan.view();
        assert_eq!(view.read_u8(0x1000).unwrap(), 1);
        assert_eq!(view.read_u8(0x5000).unwrap(), 5);
        assert_eq!(view.read_u8(0x9000).unwrap(), 9);

        let empty = Domain53SnapshotPlan::from_trace(1, Domain53AccessTrace::default()).unwrap();
        assert_eq!(empty.region_count(), 0);
        assert_eq!(empty.page_buffer_span(), (0, 0));
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
            Domain53SnapshotPlan::from_trace_with_limits(
                1,
                trace.clone(),
                1,
                2 * DOMAIN53_SNAPSHOT_PAGE_BYTES,
            )
            .unwrap_err(),
            Domain53SnapshotPlanRefusal {
                cause: DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE,
                demand: Domain53SnapshotPlanDemand {
                    trace_read_count: 2,
                    successful_read_count: 2,
                    legacy_region_count: 2,
                    legacy_byte_count: 16,
                    page_region_count: 2,
                    page_byte_count: 2 * DOMAIN53_SNAPSHOT_PAGE_BYTES as u64,
                    probe_region_count: 0,
                    os_region_count: 0,
                },
            }
        );
        assert_eq!(
            Domain53SnapshotPlan::from_trace_with_limits(
                1,
                trace,
                2,
                2 * DOMAIN53_SNAPSHOT_PAGE_BYTES - 1,
            )
            .unwrap_err()
            .cause,
            DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE,
        );
    }

    /// Codex round 1, F5: a read that failed when the walk ran must not stay
    /// failed for every later hour. Its page is planned as a probe: while it
    /// is still unreadable the failure is replayed with no layout miss; the
    /// hour it copies, the current bytes are served.
    #[test]
    fn a_failed_read_recovers_the_hour_its_probe_page_copies() {
        let mut source = FixtureMemory::default();
        let traced = Domain53TracingMemory::new(&source);
        assert!(traced.read_u64(0x4000).is_err());
        let mut plan = Domain53SnapshotPlan::from_trace(1, traced.into_trace()).unwrap();
        assert_eq!(plan.region_count(), 1);
        assert_eq!(plan.demand().probe_region_count, 1);
        assert_eq!(plan.demand().page_region_count, 0);

        let receipt = plan.capture_with(|address, out| source.read_into(address, out).ok().map(|_| 1));
        assert_eq!(receipt.probe_region_count, 1);
        assert_eq!(receipt.probe_regions_unreadable, 1);
        assert_eq!(receipt.failure_region_ordinal, None, "a probe page never refuses the copy");
        assert_eq!(receipt.copied_region_count, 0);
        {
            let view = plan.view();
            assert!(view.read_u64(0x4000).is_err());
            assert_eq!(view.first_miss(), None, "the replayed failure is not a miss");
            assert!(view.read_u64(0x4008).is_err());
            assert_eq!(view.first_miss(), Some((0x4008, 8)), "an unplanned read is");
        }

        source.insert_page(0x4000);
        source.insert(0x4000, &7u64.to_le_bytes());
        let receipt = plan.capture_with(|address, out| source.read_into(address, out).ok().map(|_| 1));
        assert_eq!(receipt.probe_regions_unreadable, 0);
        assert_eq!(receipt.copied_region_count, 1);
        let view = plan.view();
        assert_eq!(view.read_u64(0x4000).unwrap(), 7);
        assert_eq!(view.first_miss(), None);
    }

    /// The uncommitted twin of the test above: the walk's `probe_not_committed`
    /// answer is replayed until the page copies, then it reads as committed.
    /// A probe the walk never made is a layout miss, answered the refusing way.
    #[test]
    fn an_uncommitted_probe_reads_as_committed_the_hour_its_page_copies() {
        let mut source = FixtureMemory::default();
        source.uncommitted.insert(0x5010);
        let traced = Domain53TracingMemory::new(&source);
        assert!(traced.probe_not_committed(0x5010));
        let mut plan = Domain53SnapshotPlan::from_trace(1, traced.into_trace()).unwrap();
        assert_eq!(plan.demand().probe_region_count, 1);

        let receipt = plan.capture_with(|address, out| source.read_into(address, out).ok().map(|_| 1));
        assert_eq!(receipt.probe_regions_unreadable, 1);
        {
            let view = plan.view();
            assert!(view.probe_not_committed(0x5010));
            assert_eq!(view.first_miss(), None);
            assert!(view.probe_not_committed(0x9000));
            assert_eq!(view.first_miss(), Some((0x9000, 0)));
        }

        source.uncommitted.clear();
        source.insert_page(0x5000);
        plan.capture_with(|address, out| source.read_into(address, out).ok().map(|_| 1));
        let view = plan.view();
        assert!(!view.probe_not_committed(0x5010));
        assert_eq!(view.first_miss(), None);
    }

    /// Codex round 2, F5: the live reader refuses a read that crosses two
    /// operating-system regions even when every byte is readable. A snapshot
    /// serves a read only from pages the gate confirmed in ONE region, so a
    /// failed cross-region read is replayed as failed until the pages are one
    /// region -- and a read the walk never made across two regions is a miss.
    #[test]
    fn a_read_across_two_os_regions_is_refused_until_they_are_one() {
        let mut source = FixtureMemory::default();
        source.insert(0x1ff8, &[7; 8]);
        let traced = Domain53TracingMemory::new(&source);
        assert!(traced.read_bytes(0x1ff8, 16).is_err(), "the walk's read failed");
        let mut plan = Domain53SnapshotPlan::from_trace(1, traced.into_trace()).unwrap();
        assert_eq!(plan.demand().probe_region_count, 2, "both pages of the failed read");

        source.insert_page(0x1000);
        source.insert_page(0x2000);
        source.insert(0x1ff8, &[7; 8]);
        source.insert(0x2000, &[9; 8]);
        // Each page copies, but the gate confirmed them in different regions.
        let receipt = plan.capture_with(|address, out| {
            source.read_into(address, out).ok().map(|_| address >> 12)
        });
        assert_eq!(receipt.copied_region_count, 2);
        {
            let view = plan.view();
            assert!(view.read_bytes(0x1ff8, 16).is_err(), "two regions: still refused");
            assert_eq!(view.first_miss(), None, "the replayed failure is not a miss");
            assert_eq!(view.read_u64(0x1ff8).unwrap(), 0x0707_0707_0707_0707);
            assert_eq!(view.read_u64(0x2000).unwrap(), 0x0909_0909_0909_0909);
            assert_eq!(view.readable_ranges(0x1000, 0x3000), vec![(0x1000, 4096), (0x2000, 4096)]);
            assert!(view.read_bytes(0x1ffc, 8).is_err(), "an unplanned cross-region read");
            assert_eq!(view.first_miss(), Some((0x1ffc, 8)));
        }

        // The hour the pages are one region, the read recovers.
        let receipt = plan.capture_with(|address, out| source.read_into(address, out).ok().map(|_| 1));
        assert_eq!(receipt.copied_region_count, 2);
        let view = plan.view();
        assert_eq!(view.read_bytes(0x1ff8, 16).unwrap(), [7, 7, 7, 7, 7, 7, 7, 7, 9, 9, 9, 9, 9, 9, 9, 9]);
        assert_eq!(view.readable_ranges(0x1000, 0x3000), vec![(0x1000, 8192)]);
        assert_eq!(view.first_miss(), None);
    }

    #[test]
    fn probe_pages_count_against_the_caps_and_never_duplicate_a_data_page() {
        let mut source = FixtureMemory::default();
        // Eight readable bytes on the page and nothing else on it.
        source.insert(0x1000, &[1; 8]);
        let traced = Domain53TracingMemory::new(&source);
        assert!(traced.read_u64(0x1000).is_ok());
        assert!(traced.read_u64(0x1800).is_err(), "same page, unreadable bytes");
        assert!(traced.read_u64(0x3000).is_err());
        let trace = traced.into_trace();
        let plan = Domain53SnapshotPlan::from_trace(1, trace.clone()).unwrap();
        assert_eq!(plan.demand().page_region_count, 1);
        assert_eq!(plan.demand().probe_region_count, 1, "0x1000 is a data page already");
        assert_eq!(plan.region_count(), 2);
        let refused = Domain53SnapshotPlan::from_trace_with_limits(
            1,
            trace,
            1,
            2 * DOMAIN53_SNAPSHOT_PAGE_BYTES,
        )
        .unwrap_err();
        assert_eq!(refused.cause, DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE);
        assert_eq!(refused.demand.probe_region_count, 1);
    }

    #[test]
    fn reads_cross_adjacent_captured_pages() {
        let mut source = FixtureMemory::default();
        source.insert_page(0x1000);
        source.insert_page(0x2000);
        source.insert(0x1ffe, &[1, 2]);
        source.insert(0x2000, &[3, 4]);
        let trace = Domain53AccessTrace {
            reads: vec![Domain53ReadTrace {
                address: 0x1ffe,
                len: 4,
                succeeded: true,
            }],
            ..Domain53AccessTrace::default()
        };
        let mut plan = Domain53SnapshotPlan::from_trace(1, trace).unwrap();
        let receipt = plan.capture_with(|address, out| source.read_into(address, out).ok().map(|_| 1));

        assert_eq!(receipt.failure_region_ordinal, None);
        assert_eq!(plan.view().read_u32(0x1ffe).unwrap(), 0x0403_0201);
    }

    #[test]
    fn trace_overflow_refuses_the_plan() {
        let trace = Domain53AccessTrace {
            trace_overflowed: true,
            ..Domain53AccessTrace::default()
        };
        assert_eq!(
            Domain53SnapshotPlan::from_trace(1, trace)
                .unwrap_err()
                .cause,
            DOMAIN53_SNAPSHOT_TRACE_CAP_CAUSE
        );
    }

    #[test]
    fn production_lower_bound_exceeds_legacy_cap_but_fits_page_plan() {
        const P4_REFUSED_REGION_LOWER_BOUND: usize = 65_537;
        const MINIMUM_NON_MERGING_STRIDE: u64 = DOMAIN53_SNAPSHOT_LEGACY_MERGE_GAP_BYTES + 2;
        let trace = Domain53AccessTrace {
            reads: (0..P4_REFUSED_REGION_LOWER_BOUND)
                .map(|ordinal| Domain53ReadTrace {
                    address: 0x1000 + ordinal as u64 * MINIMUM_NON_MERGING_STRIDE,
                    len: 1,
                    succeeded: true,
                })
                .collect(),
            ..Domain53AccessTrace::default()
        };
        let plan = Domain53SnapshotPlan::from_trace(1, trace).unwrap();

        assert_eq!(plan.demand().legacy_region_count, 65_537);
        assert!(plan.demand().legacy_region_count > 65_536);
        assert_eq!(plan.region_count(), 1_057);
        assert_eq!(plan.byte_count(), 1_057 * DOMAIN53_SNAPSHOT_PAGE_BYTES);
    }
}
