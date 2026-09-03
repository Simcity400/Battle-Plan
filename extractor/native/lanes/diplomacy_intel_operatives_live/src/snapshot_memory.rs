use crate::extractor::{Domain53Memory, ExtractError};
use core::ptr;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
#[cfg(windows)]
use windows_sys::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};

pub const DOMAIN53_SNAPSHOT_PAGE_BYTES: usize = 4096;
/// Two successful reads no further apart than this become ONE planned range,
/// gap bytes included. Safe at any value below the page size: the gap cannot
/// reach a page that holds no read byte, so every byte of a merged range sits
/// on a page the walk read successfully.
pub const DOMAIN53_SNAPSHOT_MERGE_GAP_BYTES: u64 = 64;
/// Planned ranges and the bytes they copy. Measured demand, not a shape
/// guess: a real 1944 campaign of 439 countries asked for 393,117 ranges and
/// 26,609,589 bytes at its worst hour (runs/d53c_snapshot_region_cap/
/// DEMAND_MEASUREMENT.json, capture e2e_02), which copies in 7.7-9.7 ms
/// against the 16 ms bar. The caps are 1.33x / 1.26x that, which is what the
/// arena's own memory affords (32 MiB of bytes plus 40 bytes of plan per
/// range is 52 MiB); the worst LEGAL plan at them costs more than the bar,
/// and both figures are measured on the production copy path in
/// runs/d53c_snapshot_region_cap/GATE_COST.json. Caps low enough to hold the
/// worst legal plan under the bar would sit ON the measured demand and refuse
/// every larger campaign outright, which is more loss than a longer bracket:
/// an over-long bracket costs an hour only when it crosses the tick the clock
/// bracket already checks, a refusal costs every hour. A plan over a cap is
/// refused with its attempted demand, never truncated.
pub const DOMAIN53_SNAPSHOT_MAX_REGIONS: usize = 524_288;
pub const DOMAIN53_SNAPSHOT_MAX_BYTES: usize = 32 * 1024 * 1024;
pub const DOMAIN53_SNAPSHOT_MAX_TRACE_READS: usize = 8 * 1024 * 1024;
/// The service bar one bracket is measured against: one frame at 60 Hz.
pub const DOMAIN53_SNAPSHOT_SERVICE_BAR_NS: u64 = 16_000_000;
/// What the copy of the measured production demand costs the bracket:
/// 393,494 ranges and 26,757,592 bytes at 7,874,200 ns unloaded p50 on the
/// production copy path (runs/d53c_os_region_budget/GATE_COST.json,
/// measured_demand).
pub const DOMAIN53_SNAPSHOT_MEASURED_COPY_NS: u64 = 7_874_200;
/// What ONE gate query costs the bracket. Two measurements, and this is the
/// LARGER: interleaving a one-region and a many-region capture so both see
/// the same machine gives 514-784 ns, and subtracting the one-region shape's
/// own measurement window from the many-region shape's gives 1,052 ns. The
/// interleaved figure is the cleaner one and the standalone figure is the one
/// a bracket actually exhibited, so the budget is derived from the standalone
/// figure and is the smaller of the two derivations.
pub const DOMAIN53_SNAPSHOT_GATE_QUERY_NS: u64 = 1_052;
/// Gate queries one bracket may make, which is what is left of the service
/// bar after the measured copy, at the measured cost of one query. NOT a cap
/// on how fragmented the address space may be: the bracket queries a region
/// when the copy reaches a range outside the one it holds, so fragmentation
/// costs queries. Only a layout needing more queries than the bar affords
/// costs the hour, and it says so with the count it reached. Capture e2e_03
/// measured 2,319 regions at its worst hour, 3.3x under this; the cap this
/// replaces was a guessed 2,048 checked BEFORE the copy ran, which refused
/// 490 of that capture's 1,169 hours.
pub const DOMAIN53_SNAPSHOT_GATE_QUERY_BUDGET: u32 =
    ((DOMAIN53_SNAPSHOT_SERVICE_BAR_NS - DOMAIN53_SNAPSHOT_MEASURED_COPY_NS)
        / DOMAIN53_SNAPSHOT_GATE_QUERY_NS) as u32;

pub const DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE: &str = "domain53_snapshot_region_cap";
pub const DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE: &str = "domain53_snapshot_byte_cap";
pub const DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE: &str = "domain53_snapshot_address_overflow";
pub const DOMAIN53_SNAPSHOT_REGION_MISS_CAUSE: &str = "domain53_snapshot_region_miss";
pub const DOMAIN53_SNAPSHOT_COPY_REFUSED_CAUSE: &str = "domain53_snapshot_copy_refused";
pub const DOMAIN53_SNAPSHOT_TRACE_CAP_CAUSE: &str = "domain53_snapshot_trace_cap";
pub const DOMAIN53_SNAPSHOT_GATE_BUDGET_CAUSE: &str = "domain53_snapshot_gate_budget";
pub const DOMAIN53_SNAPSHOT_ARENA_CAUSE: &str = "domain53_snapshot_arena_refused";

/// The plan's destination bytes: ONE page-aligned, committed allocation
/// holding every planned range end to end in plan order. Committed and
/// touched page by page on the plan-walk thread, so the bracket copies into
/// resident memory (a fresh plan's first copy measured 28 ms for 32 MiB
/// against 5 ms resident while the buffers were lazily committed heap
/// blocks). Taken from the operating system directly rather than the global
/// allocator, so the collector's private heap and the test allocator share
/// one allocation environment.
struct Domain53ByteArena {
    base: *mut u8,
    bytes: usize,
    #[cfg(not(windows))]
    backing: Vec<u8>,
}

// SAFETY: the arena owns its bytes exclusively and hands them out only
// through `&self` / `&mut self` borrows.
unsafe impl Send for Domain53ByteArena {}
unsafe impl Sync for Domain53ByteArena {}

impl core::fmt::Debug for Domain53ByteArena {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Domain53ByteArena")
            .field("base", &self.base)
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl Domain53ByteArena {
    fn new(bytes: usize) -> Option<Self> {
        if bytes == 0 {
            return Some(Self {
                base: ptr::null_mut(),
                bytes: 0,
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
            Self { base, bytes }
        };
        #[cfg(not(windows))]
        let mut arena = {
            let mut backing = vec![0u8; bytes + DOMAIN53_SNAPSHOT_PAGE_BYTES];
            let start = backing.as_mut_ptr() as usize;
            let aligned =
                (start + DOMAIN53_SNAPSHOT_PAGE_BYTES - 1) & !(DOMAIN53_SNAPSHOT_PAGE_BYTES - 1);
            Self {
                base: aligned as *mut u8,
                bytes,
                backing,
            }
        };
        arena.touch();
        Some(arena)
    }

    /// Write one byte into every page so each is committed AND resident
    /// before the bracket copies into it. Volatile so the stores are kept.
    fn touch(&mut self) {
        let mut offset = 0usize;
        while offset < self.bytes {
            // SAFETY: `offset` is inside the owned, committed allocation.
            unsafe { ptr::write_volatile(self.base.add(offset), 0) };
            offset += DOMAIN53_SNAPSHOT_PAGE_BYTES;
        }
    }

    fn bytes(&self) -> &[u8] {
        if self.bytes == 0 {
            return &[];
        }
        // SAFETY: the whole owned allocation, committed and initialised by
        // `touch`.
        unsafe { core::slice::from_raw_parts(self.base, self.bytes) }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        if self.bytes == 0 {
            return &mut [];
        }
        // SAFETY: the whole owned allocation; `&mut self` makes this the only
        // live borrow of it.
        unsafe { core::slice::from_raw_parts_mut(self.base, self.bytes) }
    }

    fn span(&self) -> (usize, usize) {
        (self.base as usize, self.bytes)
    }
}

#[cfg(windows)]
impl Drop for Domain53ByteArena {
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

/// One planned range as the bracket's batch copy consumes it. The field
/// order and `repr(C)` ARE the collector's `SehCopySpec` ABI: the bracket
/// hands a subslice of these straight to the SEH shim.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain53SnapshotCopySpec {
    pub src: u64,
    pub dst_offset: u64,
    pub len: u64,
}

/// An operating-system region as the bracket's gate sees it: `[start, stop)`
/// and whether the live reader's own predicate accepts it (committed, not
/// no-access, not a guard page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain53SnapshotOsRegion {
    pub start: u64,
    pub stop: u64,
    pub readable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain53SnapshotPlanDemand {
    pub trace_read_count: u64,
    pub successful_read_count: u64,
    /// Merged ranges over the walk's successful reads, and their bytes.
    pub data_region_count: u64,
    pub data_byte_count: u64,
    /// Ranges the walk could not read or found uncommitted, and their bytes.
    /// They are planned as PROBE ranges: copied when they have become
    /// readable so the plan serves the current value, tolerated when they are
    /// still unreadable so the historical answer is replayed without a layout
    /// miss.
    pub probe_region_count: u64,
    pub probe_byte_count: u64,
}

impl Domain53SnapshotPlanDemand {
    pub fn region_count(&self) -> u64 {
        self.data_region_count.saturating_add(self.probe_region_count)
    }

    pub fn byte_count(&self) -> u64 {
        self.data_byte_count.saturating_add(self.probe_byte_count)
    }
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
    /// First DATA range that did not copy; a probe range never sets it.
    pub failure_region_ordinal: Option<u32>,
    pub probe_region_count: u32,
    pub probe_regions_unreadable: u32,
    pub probe_bytes_unreadable: u64,
    /// Region queries the bracket's gate issued, and whether it ran out of
    /// budget (every range after that is a copy failure).
    pub gate_queries: u32,
    pub gate_budget_exhausted: bool,
}

/// Per-range capture state, indexed exactly as `Domain53SnapshotPlan::specs`.
#[derive(Clone, Copy, Debug)]
struct Domain53SnapshotRegionState {
    /// The operating-system region the copy's gate confirmed this range in.
    /// A read may only be served from ranges of ONE region: that is the live
    /// reader's whole-range rule, and the snapshot keeps it.
    region_key: u64,
    probe: bool,
    copied: bool,
}

#[derive(Debug)]
pub struct Domain53SnapshotPlan {
    module_base: u64,
    /// Sorted by `src`, disjoint, and their `dst_offset`s are the running sum
    /// of `len` — so a run of consecutive specs owns a contiguous arena span.
    specs: Vec<Domain53SnapshotCopySpec>,
    state: Vec<Domain53SnapshotRegionState>,
    arena: Domain53ByteArena,
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

        let mut data: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            match data.last_mut() {
                Some(last)
                    if start <= last.1.saturating_add(DOMAIN53_SNAPSHOT_MERGE_GAP_BYTES) =>
                {
                    last.1 = last.1.max(end);
                }
                _ => data.push((start, end)),
            }
        }
        let data_byte_count = data
            .iter()
            .try_fold(0u64, |total, (start, end)| total.checked_add(end - start))
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE, initial_demand))?;

        // Probe ranges: what the walk could not read or found uncommitted,
        // less every byte a successful read already covers.
        let mut probe_raw: Vec<(u64, u64)> = Vec::new();
        for &(address, len) in &trace.known_failed {
            if len == 0 {
                continue;
            }
            let end = address
                .checked_add(len as u64)
                .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE, initial_demand))?;
            probe_raw.push((address, end));
        }
        for &address in &trace.known_uncommitted {
            let end = address
                .checked_add(1)
                .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_ADDRESS_OVERFLOW_CAUSE, initial_demand))?;
            probe_raw.push((address, end));
        }
        probe_raw.sort_unstable();
        let probe = subtract_sorted(coalesce_sorted(probe_raw), &data);
        let probe_region_count = probe.len() as u64;
        let probe_byte_count = probe
            .iter()
            .try_fold(0u64, |total, (start, end)| total.checked_add(end - start))
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE, initial_demand))?;

        let demand = Domain53SnapshotPlanDemand {
            trace_read_count,
            successful_read_count,
            data_region_count: data.len() as u64,
            data_byte_count,
            probe_region_count,
            probe_byte_count,
        };
        let total_region_count = demand.region_count();
        let total_byte_count = demand.byte_count();
        if total_region_count > max_regions as u64 {
            return Err(refuse(DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE, demand));
        }
        if total_byte_count > max_bytes as u64 {
            return Err(refuse(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE, demand));
        }
        let Ok(arena_bytes) = usize::try_from(total_byte_count) else {
            return Err(refuse(DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE, demand));
        };

        let mut ordered: Vec<(u64, u64, bool)> = Vec::with_capacity(total_region_count as usize);
        ordered.extend(data.into_iter().map(|(start, end)| (start, end, false)));
        ordered.extend(probe.into_iter().map(|(start, end)| (start, end, true)));
        ordered.sort_unstable();

        let mut specs = Vec::with_capacity(ordered.len());
        let mut state = Vec::with_capacity(ordered.len());
        let mut dst_offset = 0u64;
        for (start, end, probe) in ordered {
            let len = end - start;
            specs.push(Domain53SnapshotCopySpec {
                src: start,
                dst_offset,
                len,
            });
            state.push(Domain53SnapshotRegionState {
                region_key: 0,
                probe,
                copied: false,
            });
            dst_offset += len;
        }

        // The arena is committed and touched HERE, on the plan-walk thread,
        // so the bracket's copy faults nothing in.
        let arena = Domain53ByteArena::new(arena_bytes)
            .ok_or_else(|| refuse(DOMAIN53_SNAPSHOT_ARENA_CAUSE, demand))?;
        Ok(Self {
            module_base,
            specs,
            state,
            arena,
            known_uncommitted: trace.known_uncommitted,
            known_failed: trace.known_failed,
            byte_count: arena_bytes,
            probe_region_count: probe_region_count.min(u32::MAX as u64) as u32,
            demand,
        })
    }

    pub fn copy_specs(&self) -> &[Domain53SnapshotCopySpec] {
        &self.specs
    }

    /// The arena's address span `(base, bytes)`: page-aligned, every planned
    /// range end to end in plan order, committed and touched when the plan
    /// was built.
    pub fn arena_span(&self) -> (usize, usize) {
        self.arena.span()
    }

    pub fn region_count(&self) -> usize {
        self.specs.len()
    }

    pub fn byte_count(&self) -> usize {
        self.byte_count
    }

    pub fn demand(&self) -> Domain53SnapshotPlanDemand {
        self.demand
    }

    /// Copy every planned range. `query` answers the operating-system region
    /// holding an address (`None` when the query itself failed); `copy` takes
    /// a run of consecutive specs the gate confirmed in ONE readable region
    /// plus the whole arena, and answers how many LEADING specs it copied
    /// (`specs.len()` on success; a smaller number means the spec at that
    /// index faulted and no later spec of the run was copied).
    pub fn capture_with(
        &mut self,
        mut query: impl FnMut(u64) -> Option<Domain53SnapshotOsRegion>,
        mut copy: impl FnMut(&[Domain53SnapshotCopySpec], &mut [u8]) -> usize,
        query_budget: u32,
    ) -> Domain53SnapshotCaptureReceipt {
        let mut receipt = Domain53SnapshotCaptureReceipt {
            demand: self.demand,
            region_count: self.specs.len().min(u32::MAX as usize) as u32,
            byte_count: self.byte_count as u64,
            probe_region_count: self.probe_region_count,
            ..Domain53SnapshotCaptureReceipt::default()
        };
        let Self {
            specs,
            state,
            arena,
            ..
        } = self;
        for slot in state.iter_mut() {
            slot.copied = false;
            slot.region_key = 0;
        }
        let arena_len = arena.bytes;
        let arena = arena.bytes_mut();
        let mut region: Option<Domain53SnapshotOsRegion> = None;
        let mut index = 0usize;
        while index < specs.len() {
            let spec = specs[index];
            if !region.is_some_and(|held| covers(held, &spec)) {
                if receipt.gate_queries >= query_budget {
                    receipt.gate_budget_exhausted = true;
                    break;
                }
                receipt.gate_queries += 1;
                region = query(spec.src);
            }
            let Some(held) = region else {
                // The query itself failed: this range alone refuses, and the
                // next one queries again rather than inheriting a region that
                // was never answered.
                index += 1;
                continue;
            };
            if !held.readable || !covers(held, &spec) {
                // Rejected region, or a range straddling two regions: refused,
                // with the region kept so its other ranges cost no query.
                index += 1;
                continue;
            }
            let mut end = index + 1;
            while end < specs.len() && covers(held, &specs[end]) {
                end += 1;
            }
            let run = &specs[index..end];
            let last = run[run.len() - 1];
            let run_stop = last.dst_offset + last.len;
            let copied = if run_stop <= arena_len as u64 {
                copy(run, arena).min(run.len())
            } else {
                // Unreachable while the offsets are the running sum of the
                // lens; refuse rather than hand the copy an out-of-bounds run.
                0
            };
            for slot in &mut state[index..index + copied] {
                slot.copied = true;
                slot.region_key = held.start;
            }
            index += copied;
            if copied < run.len() {
                // The spec at `index` faulted, so the layout is not what the
                // gate confirmed: drop the held region and skip that spec. The
                // retry costs a query, which is what stops a region that went
                // away under the copy from charging one structured exception
                // per range it holds -- the budget ends it instead.
                region = None;
                index += 1;
            }
        }
        for (ordinal, (spec, slot)) in specs.iter().zip(state.iter()).enumerate() {
            if slot.copied {
                receipt.copied_region_count = receipt.copied_region_count.saturating_add(1);
                receipt.copied_byte_count = receipt.copied_byte_count.saturating_add(spec.len);
            } else if slot.probe {
                receipt.probe_regions_unreadable =
                    receipt.probe_regions_unreadable.saturating_add(1);
                receipt.probe_bytes_unreadable =
                    receipt.probe_bytes_unreadable.saturating_add(spec.len);
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

fn covers(region: Domain53SnapshotOsRegion, spec: &Domain53SnapshotCopySpec) -> bool {
    spec.src >= region.start && spec.src.saturating_add(spec.len) <= region.stop
}

/// Sorted half-open ranges with every overlap and touching pair fused.
fn coalesce_sorted(ranges: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// `ranges` minus `cut`, both sorted and disjoint.
fn subtract_sorted(ranges: Vec<(u64, u64)>, cut: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut next_cut = 0usize;
    for (mut start, end) in ranges {
        while next_cut < cut.len() && cut[next_cut].1 <= start {
            next_cut += 1;
        }
        let mut index = next_cut;
        while start < end && index < cut.len() && cut[index].0 < end {
            let (cut_start, cut_end) = cut[index];
            if cut_start > start {
                out.push((start, cut_start.min(end)));
            }
            start = start.max(cut_end);
            index += 1;
        }
        if start < end {
            out.push((start, end));
        }
    }
    out
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

    /// The copied range holding `address`, if any: its spec and its state.
    fn copied_range_at(
        &self,
        address: u64,
    ) -> Option<(
        &Domain53SnapshotCopySpec,
        &Domain53SnapshotRegionState,
    )> {
        let index = self
            .plan
            .specs
            .partition_point(|spec| spec.src <= address)
            .checked_sub(1)?;
        let spec = self.plan.specs.get(index)?;
        let state = self.plan.state.get(index)?;
        (state.copied && address < spec.src.saturating_add(spec.len)).then_some((spec, state))
    }

    /// Serve `out` from copied ranges of ONE operating-system region only;
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
            let Some((spec, state)) = self.copied_range_at(cursor) else {
                return false;
            };
            if key.is_some_and(|key| key != state.region_key) {
                return false;
            }
            key = Some(state.region_key);
            let offset = (cursor - spec.src) as usize;
            let available = spec.len as usize - offset;
            let chunk_len = (len - copied).min(available);
            let from = (spec.dst_offset as usize) + offset;
            out[copied..copied + chunk_len]
                .copy_from_slice(&self.plan.arena.bytes()[from..from + chunk_len]);
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
            // The walk saw this read fail and its probe range is still
            // unreadable this hour: the same answer, not a layout miss. Once
            // the range copies, the branch above serves the current bytes.
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
        for (spec, state) in self
            .plan
            .specs
            .iter()
            .zip(self.plan.state.iter())
            .filter(|(_, state)| state.copied)
        {
            let start = spec.src.max(lo);
            let end = spec.src.saturating_add(spec.len).min(hi);
            if end <= start {
                continue;
            }
            match ranges.last_mut() {
                Some((last_start, last_len))
                    if last_start.saturating_add(*last_len as u64) == start
                        && last_key == Some(state.region_key) =>
                {
                    *last_len += (end - start) as usize;
                }
                _ => ranges.push((start, (end - start) as usize)),
            }
            last_key = Some(state.region_key);
        }
        ranges
    }

    fn probe_not_committed(&self, address: u64) -> bool {
        let range_copied = self.copied_range_at(address).is_some();
        if self.plan.known_uncommitted.contains(&address) {
            // Uncommitted when the walk looked; committed now iff its probe
            // range copied this hour.
            return !range_copied;
        }
        if range_copied {
            return false;
        }
        // Never probed by the planning walk and on no copied range: the walk
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
        /// Address-space regions the fixture's gate answers, `(start, stop,
        /// readable)`. Empty means one region over everything.
        regions: Vec<(u64, u64, bool)>,
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

        fn region_at(&self, address: u64) -> Option<Domain53SnapshotOsRegion> {
            if self.regions.is_empty() {
                return Some(Domain53SnapshotOsRegion {
                    start: 0,
                    stop: u64::MAX,
                    readable: true,
                });
            }
            self.regions
                .iter()
                .find(|(start, stop, _)| address >= *start && address < *stop)
                .map(|&(start, stop, readable)| Domain53SnapshotOsRegion {
                    start,
                    stop,
                    readable,
                })
        }

        /// The whole bracket as the collector runs it: the plan's gate over
        /// this fixture's regions, and a batch copy that serves every byte it
        /// holds and stops at the first it does not.
        fn capture(&self, plan: &mut Domain53SnapshotPlan) -> Domain53SnapshotCaptureReceipt {
            self.capture_with_budget(plan, u32::MAX)
        }

        fn capture_with_budget(
            &self,
            plan: &mut Domain53SnapshotPlan,
            budget: u32,
        ) -> Domain53SnapshotCaptureReceipt {
            plan.capture_with(
                |address| self.region_at(address),
                |specs, arena| {
                    for (done, spec) in specs.iter().enumerate() {
                        let at = spec.dst_offset as usize;
                        for offset in 0..spec.len as usize {
                            let Some(byte) = self.bytes.get(&(spec.src + offset as u64)) else {
                                return done;
                            };
                            arena[at + offset] = *byte;
                        }
                    }
                    specs.len()
                },
                budget,
            )
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

    fn plan_from_reads(reads: &[(u64, usize)]) -> Domain53SnapshotPlan {
        let trace = Domain53AccessTrace {
            reads: reads
                .iter()
                .map(|&(address, len)| Domain53ReadTrace {
                    address,
                    len,
                    succeeded: true,
                })
                .collect(),
            ..Domain53AccessTrace::default()
        };
        Domain53SnapshotPlan::from_trace(1, trace).expect("a plan inside the caps")
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
            12,
        )
        .unwrap();
        let receipt = source.capture(&mut plan);
        assert_eq!(receipt.region_count, 2);
        assert_eq!(receipt.byte_count, 12, "8 pointer bytes plus 4 target bytes");
        assert_eq!(receipt.failure_region_ordinal, None);

        source.insert(0x7000, &[9, 9, 9, 9]);
        let snapshot = plan.view();
        assert_eq!(snapshot.read_u64(0x1000).unwrap(), 0x7000);
        assert_eq!(snapshot.read_u32(0x7000).unwrap(), 0x0403_0201);
        // The bytes AROUND a planned range are not in the snapshot: only what
        // the walk read is copied, so a read past the range is a miss.
        assert!(snapshot.read_u8(0x7004).is_err());
        assert!(snapshot.read_u8(0x8000).is_err());
    }

    /// The plan copies the merged READ ranges, not the pages holding them:
    /// scattered reads cost their own bytes, in one arena, in address order.
    #[test]
    fn the_arena_holds_the_merged_read_ranges_end_to_end_in_address_order() {
        let mut source = FixtureMemory::default();
        for page in [0x1000u64, 0x5000, 0x9000] {
            source.insert(page, &[(page >> 12) as u8; 8]);
        }
        let mut plan = plan_from_reads(&[(0x9000, 8), (0x1000, 8), (0x5000, 8)]);
        let (base, bytes) = plan.arena_span();
        assert_eq!(base % DOMAIN53_SNAPSHOT_PAGE_BYTES, 0, "page-aligned arena");
        assert_eq!(bytes, 24, "three 8-byte reads, not three 4 KiB pages");
        assert_eq!(plan.region_count(), 3);
        assert_eq!(
            plan.copy_specs(),
            [
                Domain53SnapshotCopySpec { src: 0x1000, dst_offset: 0, len: 8 },
                Domain53SnapshotCopySpec { src: 0x5000, dst_offset: 8, len: 8 },
                Domain53SnapshotCopySpec { src: 0x9000, dst_offset: 16, len: 8 },
            ]
        );
        let receipt = source.capture(&mut plan);
        assert_eq!(receipt.copied_region_count, 3);
        assert_eq!(receipt.copied_byte_count, 24);
        let view = plan.view();
        assert_eq!(view.read_u8(0x1000).unwrap(), 1);
        assert_eq!(view.read_u8(0x5000).unwrap(), 5);
        assert_eq!(view.read_u8(0x9000).unwrap(), 9);

        let empty = Domain53SnapshotPlan::from_trace(1, Domain53AccessTrace::default()).unwrap();
        assert_eq!(empty.region_count(), 0);
        assert_eq!(empty.arena_span(), (0, 0));
    }

    /// Reads within the merge gap become one range, gap bytes included; a
    /// read past the gap starts a new one.
    #[test]
    fn reads_inside_the_merge_gap_share_one_range() {
        let gap = DOMAIN53_SNAPSHOT_MERGE_GAP_BYTES;
        let plan = plan_from_reads(&[
            (0x1000, 8),
            (0x1008 + gap, 8),
            (0x1010 + 2 * gap + 1, 8),
        ]);
        assert_eq!(
            plan.copy_specs(),
            [
                Domain53SnapshotCopySpec {
                    src: 0x1000,
                    dst_offset: 0,
                    len: 16 + gap,
                },
                Domain53SnapshotCopySpec {
                    src: 0x1010 + 2 * gap + 1,
                    dst_offset: 16 + gap,
                    len: 8,
                },
            ]
        );
        assert_eq!(plan.demand().data_region_count, 2);
        assert_eq!(plan.demand().data_byte_count, 24 + gap);
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
            Domain53SnapshotPlan::from_trace_with_limits(1, trace.clone(), 1, 16)
                .unwrap_err(),
            Domain53SnapshotPlanRefusal {
                cause: DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE,
                demand: Domain53SnapshotPlanDemand {
                    trace_read_count: 2,
                    successful_read_count: 2,
                    data_region_count: 2,
                    data_byte_count: 16,
                    probe_region_count: 0,
                    probe_byte_count: 0,
                },
            }
        );
        assert_eq!(
            Domain53SnapshotPlan::from_trace_with_limits(1, trace, 2, 15)
                .unwrap_err()
                .cause,
            DOMAIN53_SNAPSHOT_BYTE_CAP_CAUSE,
        );
    }

    /// Codex round 1, F5: a read that failed when the walk ran must not stay
    /// failed for every later hour. Its bytes are planned as a probe range:
    /// while they are still unreadable the failure is replayed with no layout
    /// miss; the hour they copy, the current bytes are served.
    #[test]
    fn a_failed_read_recovers_the_hour_its_probe_range_copies() {
        let mut source = FixtureMemory::default();
        let traced = Domain53TracingMemory::new(&source);
        assert!(traced.read_u64(0x4000).is_err());
        let mut plan = Domain53SnapshotPlan::from_trace(1, traced.into_trace()).unwrap();
        assert_eq!(plan.region_count(), 1);
        assert_eq!(plan.demand().probe_region_count, 1);
        assert_eq!(plan.demand().probe_byte_count, 8);
        assert_eq!(plan.demand().data_region_count, 0);

        let receipt = source.capture(&mut plan);
        assert_eq!(receipt.probe_region_count, 1);
        assert_eq!(receipt.probe_regions_unreadable, 1);
        assert_eq!(
            receipt.failure_region_ordinal, None,
            "a probe range never refuses the copy"
        );
        assert_eq!(receipt.copied_region_count, 0);
        {
            let view = plan.view();
            assert!(view.read_u64(0x4000).is_err());
            assert_eq!(view.first_miss(), None, "the replayed failure is not a miss");
            assert!(view.read_u64(0x4008).is_err());
            assert_eq!(view.first_miss(), Some((0x4008, 8)), "an unplanned read is");
        }

        source.insert(0x4000, &7u64.to_le_bytes());
        let receipt = source.capture(&mut plan);
        assert_eq!(receipt.probe_regions_unreadable, 0);
        assert_eq!(receipt.copied_region_count, 1);
        let view = plan.view();
        assert_eq!(view.read_u64(0x4000).unwrap(), 7);
        assert_eq!(view.first_miss(), None);
    }

    /// The uncommitted twin of the test above: the walk's `probe_not_committed`
    /// answer is replayed until its byte copies, then it reads as committed.
    /// A probe the walk never made is a layout miss, answered the refusing way.
    #[test]
    fn an_uncommitted_probe_reads_as_committed_the_hour_its_byte_copies() {
        let mut source = FixtureMemory::default();
        source.uncommitted.insert(0x5010);
        let traced = Domain53TracingMemory::new(&source);
        assert!(traced.probe_not_committed(0x5010));
        let mut plan = Domain53SnapshotPlan::from_trace(1, traced.into_trace()).unwrap();
        assert_eq!(plan.demand().probe_region_count, 1);
        assert_eq!(plan.demand().probe_byte_count, 1);

        let receipt = source.capture(&mut plan);
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
        source.capture(&mut plan);
        let view = plan.view();
        assert!(!view.probe_not_committed(0x5010));
        assert_eq!(view.first_miss(), None);
    }

    /// Codex round 2, F5: the live reader refuses a read that crosses two
    /// operating-system regions even when every byte is readable. A snapshot
    /// serves a read only from ranges the gate confirmed in ONE region, so a
    /// failed cross-region read is replayed as failed until the ranges are one
    /// region -- and a read the walk never made across two regions is a miss.
    #[test]
    fn a_read_across_two_os_regions_is_refused_until_they_are_one() {
        let mut source = FixtureMemory::default();
        source.insert(0x1ff8, &[7; 8]);
        let traced = Domain53TracingMemory::new(&source);
        assert!(traced.read_bytes(0x1ff8, 16).is_err(), "the walk's read failed");
        assert!(traced.read_u64(0x1ff8).is_ok());
        assert!(traced.read_u64(0x2000).is_err());
        let mut plan = Domain53SnapshotPlan::from_trace(1, traced.into_trace()).unwrap();
        // The 8 bytes that read cover one data range; the 8 the failed reads
        // could not reach are the probe range beside it.
        assert_eq!(plan.demand().data_region_count, 1);
        assert_eq!(plan.demand().probe_region_count, 1);

        source.insert(0x2000, &[9; 8]);
        // Each range copies, but the gate confirmed them in different regions.
        source.regions = vec![(0, 0x2000, true), (0x2000, u64::MAX, true)];
        let receipt = source.capture(&mut plan);
        assert_eq!(receipt.copied_region_count, 2);
        {
            let view = plan.view();
            assert!(view.read_bytes(0x1ff8, 16).is_err(), "two regions: still refused");
            assert_eq!(view.first_miss(), None, "the replayed failure is not a miss");
            assert_eq!(view.read_u64(0x1ff8).unwrap(), 0x0707_0707_0707_0707);
            assert_eq!(view.read_u64(0x2000).unwrap(), 0x0909_0909_0909_0909);
            assert_eq!(
                view.readable_ranges(0x1000, 0x3000),
                vec![(0x1ff8, 8), (0x2000, 8)]
            );
            assert!(view.read_bytes(0x1ffc, 8).is_err(), "an unplanned cross-region read");
            assert_eq!(view.first_miss(), Some((0x1ffc, 8)));
        }

        // The hour the ranges are one region, the read recovers.
        source.regions.clear();
        let receipt = source.capture(&mut plan);
        assert_eq!(receipt.copied_region_count, 2);
        let view = plan.view();
        assert_eq!(
            view.read_bytes(0x1ff8, 16).unwrap(),
            [7, 7, 7, 7, 7, 7, 7, 7, 9, 9, 9, 9, 9, 9, 9, 9]
        );
        assert_eq!(view.readable_ranges(0x1000, 0x3000), vec![(0x1ff8, 16)]);
        assert_eq!(view.first_miss(), None);
    }

    /// A rejected operating-system region costs ONE query however many of the
    /// plan's ranges it holds, and a budget that runs out stops the bracket
    /// instead of letting it query past the bound.
    #[test]
    fn a_rejected_region_costs_one_query_and_an_exhausted_budget_stops_the_copy() {
        let mut source = FixtureMemory::default();
        for ordinal in 0..4u64 {
            source.insert(0x1000 + ordinal * 0x100, &[1; 8]);
        }
        source.insert(0x9000, &[2; 8]);
        let mut plan = plan_from_reads(&[
            (0x1000, 8),
            (0x1100, 8),
            (0x1200, 8),
            (0x1300, 8),
            (0x9000, 8),
        ]);
        source.regions = vec![(0, 0x8000, false), (0x8000, u64::MAX, true)];
        let receipt = source.capture(&mut plan);
        assert_eq!(receipt.gate_queries, 2, "one query per distinct region");
        assert!(!receipt.gate_budget_exhausted);
        assert_eq!(receipt.copied_region_count, 1);
        assert_eq!(receipt.failure_region_ordinal, Some(0));

        let receipt = source.capture_with_budget(&mut plan, 1);
        assert!(receipt.gate_budget_exhausted);
        assert_eq!(receipt.gate_queries, 1);
        assert_eq!(receipt.copied_region_count, 0);
    }

    /// A range that faults mid-batch loses only itself: the ranges before it
    /// keep their bytes and the ranges after it are copied by the retry, which
    /// re-queries -- so a region that went away under the copy is refused by
    /// the query budget instead of by one fault per range it holds.
    #[test]
    fn a_faulting_range_costs_only_itself_and_the_batch_resumes_after_it() {
        let mut source = FixtureMemory::default();
        for ordinal in 0..3u64 {
            source.insert(0x1000 + ordinal * 0x100, &[ordinal as u8 + 1; 8]);
        }
        let mut plan = plan_from_reads(&[(0x1000, 8), (0x1100, 8), (0x1200, 8)]);
        // The middle range's bytes are gone when the bracket runs.
        for offset in 0..8u64 {
            source.bytes.remove(&(0x1100 + offset));
        }
        let receipt = source.capture(&mut plan);
        assert_eq!(receipt.copied_region_count, 2);
        assert_eq!(receipt.failure_region_ordinal, Some(1));
        assert_eq!(receipt.gate_queries, 2, "the fault re-queries before retrying");
        let view = plan.view();
        assert_eq!(view.read_u64(0x1000).unwrap(), 0x0101_0101_0101_0101);
        assert!(view.read_u64(0x1100).is_err());
        assert_eq!(view.read_u64(0x1200).unwrap(), 0x0303_0303_0303_0303);

        // Every range gone: the retries spend the budget and stop, instead of
        // taking one fault per range.
        source.bytes.clear();
        let receipt = source.capture_with_budget(&mut plan, 2);
        assert!(receipt.gate_budget_exhausted);
        assert_eq!(receipt.gate_queries, 2);
        assert_eq!(receipt.copied_region_count, 0);
    }

    #[test]
    fn probe_ranges_count_against_the_caps_and_never_duplicate_a_data_range() {
        let mut source = FixtureMemory::default();
        // Eight readable bytes on the page and nothing else on it.
        source.insert(0x1000, &[1; 8]);
        let traced = Domain53TracingMemory::new(&source);
        assert!(traced.read_u64(0x1000).is_ok());
        assert!(traced.read_bytes(0x1000, 16).is_err(), "overlaps the data range");
        assert!(traced.read_u64(0x3000).is_err());
        let trace = traced.into_trace();
        let plan = Domain53SnapshotPlan::from_trace(1, trace.clone()).unwrap();
        assert_eq!(plan.demand().data_region_count, 1);
        assert_eq!(plan.demand().data_byte_count, 8);
        assert_eq!(
            plan.demand().probe_region_count,
            2,
            "the 8 bytes of the failed 16-byte read that the data range does not cover, and 0x3000"
        );
        assert_eq!(plan.demand().probe_byte_count, 16);
        assert_eq!(plan.region_count(), 3);
        let refused =
            Domain53SnapshotPlan::from_trace_with_limits(1, trace, 2, 4096).unwrap_err();
        assert_eq!(refused.cause, DOMAIN53_SNAPSHOT_REGION_CAP_CAUSE);
        assert_eq!(refused.demand.probe_region_count, 2);
    }

    #[test]
    fn reads_cross_adjacent_captured_ranges() {
        let mut source = FixtureMemory::default();
        source.insert(0x1ffe, &[1, 2]);
        source.insert(0x2000, &[3, 4]);
        let mut plan = plan_from_reads(&[(0x1ffe, 4)]);
        let receipt = source.capture(&mut plan);

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

    /// The demand a real 1944 campaign published, hour by hour, when the
    /// page-granular planner refused it: capture e2e_02, 439 countries,
    /// 263 refused hours (runs/d53c_snapshot_region_cap/DEMAND_MEASUREMENT.json).
    /// The worst hour's shape is planned here against the production caps.
    #[test]
    fn the_measured_production_demand_fits_the_caps() {
        const MEASURED_TRACE_READS: u64 = 544_830;
        const MEASURED_MERGED_RANGES: usize = 393_494;
        const MEASURED_MERGED_BYTES: u64 = 26_613_940;
        // The page-granular planner turned that demand into this, which is
        // what refused every hour of the capture.
        const MEASURED_PAGE_REGIONS: u64 = 125_083;
        const MEASURED_PAGE_BYTES: u64 =
            MEASURED_PAGE_REGIONS * DOMAIN53_SNAPSHOT_PAGE_BYTES as u64;
        assert_eq!(MEASURED_PAGE_BYTES, 512_339_968);
        assert!(
            MEASURED_PAGE_BYTES / MEASURED_MERGED_BYTES >= 19,
            "the page shape inflated the same demand past every affordable cap"
        );

        // The worst hour's ranges: the measured count and byte total, spaced
        // so none of them merges.
        let len = MEASURED_MERGED_BYTES / MEASURED_MERGED_RANGES as u64;
        let step = len + DOMAIN53_SNAPSHOT_MERGE_GAP_BYTES + 2;
        let mut reads: Vec<Domain53ReadTrace> = (0..MEASURED_MERGED_RANGES as u64)
            .map(|ordinal| Domain53ReadTrace {
                address: 0x1_0000_0000 + ordinal * step,
                len: len as usize,
                succeeded: true,
            })
            .collect();
        // The bytes the equal split leaves over ride on the LAST range, where
        // no later range can be swallowed by them.
        let short = MEASURED_MERGED_BYTES - len * MEASURED_MERGED_RANGES as u64;
        reads.last_mut().expect("ranges").len += short as usize;
        let plan = Domain53SnapshotPlan::from_trace(
            1,
            Domain53AccessTrace {
                reads,
                ..Domain53AccessTrace::default()
            },
        )
        .expect("the measured production demand plans inside the caps");
        assert_eq!(plan.region_count(), MEASURED_MERGED_RANGES);
        assert_eq!(plan.byte_count() as u64, MEASURED_MERGED_BYTES);
        assert!(plan.demand().trace_read_count <= MEASURED_TRACE_READS);
        assert!(
            (DOMAIN53_SNAPSHOT_MAX_REGIONS as f64) / (MEASURED_MERGED_RANGES as f64) >= 1.33,
            "the region cap keeps its stated headroom over the measured demand"
        );
        assert!(
            (DOMAIN53_SNAPSHOT_MAX_BYTES as f64) / (MEASURED_MERGED_BYTES as f64) >= 1.26,
            "the byte cap keeps its stated headroom over the measured demand"
        );
    }

    /// The gate's query budget is arithmetic over three measured numbers, and
    /// the operating-system regions a real campaign's plan spans sit under it
    /// with room. Capture e2e_03 refused 490 of 1,169 hours because the same
    /// quantity was a GUESSED cap of 2,048 checked before the copy ran.
    #[test]
    fn the_gate_query_budget_is_what_the_bar_affords_over_the_measured_demand() {
        const MEASURED_OS_REGIONS_MIN: u32 = 2_273;
        const MEASURED_OS_REGIONS_MAX: u32 = 2_319;
        const SUPERSEDED_OS_REGION_CAP: u32 = 2_048;
        assert!(
            MEASURED_OS_REGIONS_MIN > SUPERSEDED_OS_REGION_CAP,
            "every hour of e2e_03 was over the cap it was refused by, not a tail"
        );
        assert_eq!(
            DOMAIN53_SNAPSHOT_GATE_QUERY_BUDGET,
            ((DOMAIN53_SNAPSHOT_SERVICE_BAR_NS - DOMAIN53_SNAPSHOT_MEASURED_COPY_NS)
                / DOMAIN53_SNAPSHOT_GATE_QUERY_NS) as u32,
        );
        assert!(
            DOMAIN53_SNAPSHOT_MEASURED_COPY_NS < DOMAIN53_SNAPSHOT_SERVICE_BAR_NS,
            "the copy has to leave the bar something to buy queries with"
        );
        assert!(
            DOMAIN53_SNAPSHOT_GATE_QUERY_BUDGET >= MEASURED_OS_REGIONS_MAX * 3,
            "the budget carries at least 3x the measured region demand: {} against {MEASURED_OS_REGIONS_MAX}",
            DOMAIN53_SNAPSHOT_GATE_QUERY_BUDGET,
        );
        // The budget bounds the bracket, so spending all of it plus the
        // measured copy still fits the bar it was derived from.
        let spent = u64::from(DOMAIN53_SNAPSHOT_GATE_QUERY_BUDGET)
            * DOMAIN53_SNAPSHOT_GATE_QUERY_NS
            + DOMAIN53_SNAPSHOT_MEASURED_COPY_NS;
        assert!(spent <= DOMAIN53_SNAPSHOT_SERVICE_BAR_NS);
    }
}
