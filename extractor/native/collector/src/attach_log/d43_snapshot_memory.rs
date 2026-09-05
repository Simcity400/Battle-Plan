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
/// Bytes the widened plan aims at. Half the byte cap, so widening can never
/// push a plan the unwidened walk would have carried over `max_bytes` and
/// refuse it; the slack is whatever fits in the gap between the traced
/// demand and this number.
pub(super) const D43_SNAPSHOT_WIDEN_TARGET_BYTES: usize = 8 << 20;
/// The widest neighbourhood one traced range may claim on each side. A read
/// further out than this from every byte the walk touched last hour is a new
/// allocation, not the next element of something already planned, and
/// `misses_in_span` is what counts those.
pub(super) const D43_SNAPSHOT_MAX_WIDEN_SLACK_BYTES: u64 = 512;

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
    pub widen_target_bytes: usize,
    pub max_widen_slack: u64,
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
            widen_target_bytes: D43_SNAPSHOT_WIDEN_TARGET_BYTES,
            max_widen_slack: D43_SNAPSHOT_MAX_WIDEN_SLACK_BYTES,
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
    fn plan_reads(&self, base: u64, fields: &[(usize, usize)]) {
        let mut bytes = [0u8; 8];
        for &(offset, len) in fields {
            let mut trace = self.trace.borrow_mut();
            if trace.overflowed || trace.reads.len() == trace.capacity() {
                trace.overflowed = true;
                break;
            }
            let Some(address) = base.checked_add(offset as u64) else {
                trace.overflowed = true;
                break;
            };
            let Some(out) = bytes.get_mut(..len) else {
                trace.overflowed = true;
                break;
            };
            let succeeded = self.inner.read_into(address, out);
            trace.note(address, len, succeeded);
        }
    }

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
    /// The distinct operating-system regions the plan lies inside, in address
    /// order. A missed read inside one of these is a read a wider plan over
    /// the same regions could have carried; a missed read outside all of them
    /// is in memory this plan never touched.
    os_spans: Vec<(u64, u64)>,
    /// Bytes claimed on each side of every traced range by
    /// [`Self::fit_to_os_regions`].
    widen_slack: u64,
    /// 64-byte line -> region index, direct-mapped, owned here so no frame
    /// allocates it inside the bracket; every entry is verified against the
    /// region bounds before it answers.
    line_cache: std::cell::RefCell<Vec<(u64, u32)>>,
}

const D43_VIEW_LINE_CACHE_WAYS: usize = 1024;

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
        // A failed read may overlap a successful scalar without being wholly
        // covered. Retain each uncovered byte as a probe, never as measured data.
        let mut uncovered = Vec::new();
        for (start, end) in failed {
            let mut cursor = start;
            let first = merged.partition_point(|(_, data_end)| *data_end <= start);
            for &(data_start, data_end) in &merged[first..] {
                if data_start >= end { break; }
                if cursor < data_start { uncovered.push((cursor, data_start.min(end))); }
                cursor = cursor.max(data_end);
                if cursor >= end { break; }
            }
            if cursor < end { uncovered.push((cursor, end)); }
        }
        uncovered.sort_unstable();
        let mut probes: Vec<(u64, u64)> = Vec::new();
        for (start, end) in uncovered {
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
            os_spans: Vec::new(),
            widen_slack: 0,
            line_cache: std::cell::RefCell::new(vec![(u64::MAX, 0); D43_VIEW_LINE_CACHE_WAYS]),
        })
    }

    /// Cut every region at the operating-system boundaries `region_of` names,
    /// then give each DATA piece as much of the memory around it as the plan's
    /// unspent byte budget affords.
    ///
    /// `region_of(address)` answers the committed operating-system region
    /// `[start, end)` that holds `address`, or `None` where it cannot be
    /// queried. It is asked once per region and once more per boundary
    /// crossed; consecutive regions inside one operating-system region reuse
    /// the previous answer, so the plan spends one query per region it spans
    /// rather than one per range it holds.
    ///
    /// **Why widen at all.** The plan is armed from one hour's addresses and
    /// served to the next, and the walk that meets it reads one element past
    /// the end of a roster, a wing the vector grew by, a table slot no wing
    /// indexed last hour. Those addresses are inside memory the plan already
    /// spans and a byte-exact region table refuses them. On
    /// `captures/e2e_05` the traced demand is 2.15 MB against a 16 MiB cap
    /// and 8 reads of 1,035,107 miss.
    ///
    /// The slack is clipped to the operating-system region the piece was cut
    /// inside and to its neighbours' own bounds, so widening never crosses a
    /// region boundary (which the copy's gate would refuse), never reaches a
    /// page the arming walk did not prove committed, and never overlaps
    /// another region. A probe region -- a read the arming walk could NOT
    /// make -- is never widened and never merged into a data region: the two
    /// carry different answers on a copy that comes up short.
    pub fn fit_to_os_regions(
        mut self,
        limits: Domain43SnapshotLimits,
        mut region_of: impl FnMut(u64) -> Option<(u64, u64)>,
    ) -> Result<Self, Domain43SnapshotPlanRefusal> {
        // Cut at the boundaries first: after this every piece lies inside one
        // operating-system region (or inside none, where the query failed).
        let mut pieces: Vec<(u64, u64, bool, Option<(u64, u64)>)> =
            Vec::with_capacity(self.starts.len());
        let mut cached: Option<(u64, u64)> = None;
        let mut span_of = |address: u64| -> Option<(u64, u64)> {
            if let Some((start, end)) = cached {
                if start <= address && address < end {
                    return cached;
                }
            }
            cached = region_of(address);
            cached.filter(|(start, end)| *start <= address && address < *end)
        };
        for index in 0..self.starts.len() {
            let (start, end, is_probe) = (self.starts[index], self.ends[index], self.probe[index]);
            let mut cursor = start;
            loop {
                let span = span_of(cursor);
                let cut = span.map(|(_, span_end)| span_end).filter(|cut| *cut < end);
                match cut {
                    Some(cut) => {
                        pieces.push((cursor, cut, is_probe, span));
                        cursor = cut;
                    }
                    None => {
                        pieces.push((cursor, end, is_probe, span));
                        break;
                    }
                }
            }
        }
        if pieces.len() > limits.max_regions {
            return Err(Domain43SnapshotPlanRefusal {
                cause: D43_SNAPSHOT_REGION_CAP_CAUSE,
                region_count: pieces.len() as u64,
                byte_count: self.byte_count as u64,
            });
        }
        // The slack every data piece may claim on each side, from the gap
        // between the traced demand and the widen target. Computed once,
        // against the piece count the widening is actually applied to, so the
        // widened plan cannot pass the target however the pieces fall.
        let slack = limits
            .widen_target_bytes
            .saturating_sub(self.byte_count)
            .checked_div(pieces.len().saturating_mul(2))
            .unwrap_or(0) as u64;
        let slack = slack.min(limits.max_widen_slack);
        if slack > 0 {
            for index in 0..pieces.len() {
                let (start, end, is_probe, span) = pieces[index];
                let Some((span_start, span_end)) = span else {
                    continue;
                };
                if is_probe {
                    continue;
                }
                let floor = index
                    .checked_sub(1)
                    .map_or(span_start, |prev| pieces[prev].1.max(span_start));
                let ceiling = pieces
                    .get(index + 1)
                    .map_or(span_end, |next| next.0.min(span_end));
                pieces[index].0 = start.saturating_sub(slack).max(floor);
                pieces[index].1 = end.saturating_add(slack).min(ceiling).max(end);
            }
        }
        let mut starts: Vec<u64> = Vec::with_capacity(pieces.len());
        let mut ends: Vec<u64> = Vec::with_capacity(pieces.len());
        let mut probe: Vec<bool> = Vec::with_capacity(pieces.len());
        let mut os_spans: Vec<(u64, u64)> = Vec::new();
        let mut last_span: Option<(u64, u64)> = None;
        for (start, end, is_probe, span) in pieces {
            if let Some(span) = span {
                if os_spans.last() != Some(&span) {
                    os_spans.push(span);
                }
            }
            // Merged only inside ONE operating-system region and only between
            // pieces of the same kind, so a merged region still answers the
            // copy gate's question with one query and a probe never hides
            // inside a data region.
            let mergeable = last_span == span
                && probe.last() == Some(&is_probe)
                && ends.last().is_some_and(|last_end| start <= *last_end);
            if mergeable {
                if let Some(last_end) = ends.last_mut() {
                    *last_end = (*last_end).max(end);
                    continue;
                }
            }
            starts.push(start);
            ends.push(end);
            probe.push(is_probe);
            last_span = span;
        }
        let mut offsets = Vec::with_capacity(starts.len());
        let mut byte_count = 0usize;
        for (start, end) in starts.iter().zip(&ends) {
            offsets.push(byte_count);
            byte_count = byte_count
                .checked_add((end - start) as usize)
                .ok_or(Domain43SnapshotPlanRefusal {
                    cause: D43_SNAPSHOT_BYTE_CAP_CAUSE,
                    region_count: starts.len() as u64,
                    byte_count: u64::MAX,
                })?;
        }
        if byte_count > limits.max_bytes {
            return Err(Domain43SnapshotPlanRefusal {
                cause: D43_SNAPSHOT_BYTE_CAP_CAUSE,
                region_count: starts.len() as u64,
                byte_count: byte_count as u64,
            });
        }
        debug_assert!(byte_count >= self.byte_count);
        self.copied = vec![0u32; starts.len()];
        self.arena = vec![0u8; byte_count].into_boxed_slice();
        self.byte_count = byte_count;
        self.starts = starts;
        self.ends = ends;
        self.offsets = offsets;
        self.probe_region_count = probe.iter().filter(|is_probe| **is_probe).count() as u32;
        self.probe = probe;
        self.line_cache.borrow_mut().fill((u64::MAX, 0));
        self.os_spans = os_spans;
        self.widen_slack = slack;
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

    pub fn widen_slack(&self) -> u64 {
        self.widen_slack
    }

    /// Whether `[address, address + len)` lies wholly inside one of the
    /// operating-system regions this plan spans.
    fn inside_one_os_span(&self, address: u64, len: usize) -> bool {
        let Some(end) = address.checked_add(len as u64) else {
            return false;
        };
        let Some(index) = self
            .os_spans
            .partition_point(|(start, _)| *start <= address)
            .checked_sub(1)
        else {
            return false;
        };
        end <= self.os_spans[index].1
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
            misses_in_span: Cell::new(0),
            last_hit: Cell::new(0),
            line_cache: &self.line_cache,
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
    misses_in_span: Cell<u64>,
    last_hit: Cell<usize>,
    line_cache: &'a std::cell::RefCell<Vec<(u64, u32)>>,
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

    /// The subset of [`Self::misses`] whose address lies inside an
    /// operating-system region the plan already spans. Those are reads a
    /// wider plan over the same regions could have carried; the remainder are
    /// in memory this plan never touched, and no widening reaches them.
    pub fn misses_in_span(&self) -> u64 {
        self.misses_in_span.get()
    }

    pub fn reads_served(&self) -> u64 {
        self.served.get()
    }

    /// The consecutive regions covering `[address, address + len)`, first and
    /// last; `None` where the plan does not cover it.
    fn locate(&self, address: u64, len: usize) -> Option<(usize, usize)> {
        let end = address.checked_add(len as u64)?;
        let plan = self.plan;
        let covers = |index: usize| {
            index < plan.starts.len() && plan.starts[index] <= address && address < plan.ends[index]
        };
        let last = self.last_hit.get();
        let first = if covers(last) {
            last
        } else {
            let line = address >> 6;
            let slot = (line as usize) & (D43_VIEW_LINE_CACHE_WAYS - 1);
            let cached = self.line_cache.borrow()[slot];
            let index = if cached.0 == line && covers(cached.1 as usize) {
                cached.1 as usize
            } else {
                let index =
                    plan.starts.partition_point(|start| *start <= address).checked_sub(1)?;
                if address >= plan.ends[index] {
                    return None;
                }
                self.line_cache.borrow_mut()[slot] = (line, index as u32);
                index
            };
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
            if self.plan.inside_one_os_span(address, len) {
                self.misses_in_span
                    .set(self.misses_in_span.get().saturating_add(1));
            }
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

    /// Slack zero: the split's own behaviour, with no widening on top.
    fn limits(merge_gap: u64, max_regions: usize, max_bytes: usize) -> Domain43SnapshotLimits {
        Domain43SnapshotLimits {
            merge_gap,
            max_regions,
            max_bytes,
            widen_target_bytes: 0,
            ..Domain43SnapshotLimits::default()
        }
    }

    fn widening_limits(
        merge_gap: u64,
        max_regions: usize,
        max_bytes: usize,
        widen_target_bytes: usize,
        max_widen_slack: u64,
    ) -> Domain43SnapshotLimits {
        Domain43SnapshotLimits {
            merge_gap,
            max_regions,
            max_bytes,
            widen_target_bytes,
            max_widen_slack,
            ..Domain43SnapshotLimits::default()
        }
    }

    /// One operating-system region per entry, in address order; the last runs
    /// to the top of the space.
    fn spans(bounds: &[u64]) -> impl FnMut(u64) -> Option<(u64, u64)> + '_ {
        move |address| {
            let mut start = 0u64;
            for bound in bounds {
                if address < *bound {
                    return Some((start, *bound));
                }
                start = *bound;
            }
            Some((start, u64::MAX))
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
    fn planned_branch_reads_obey_trace_capacity_and_use_the_copy_hour() {
        let old = FixtureMemory::spanning(&[(0x1000, 8), (0x1170, 4)]);
        let trace = Domain43TracingMemory::new(&old, Domain43AccessTrace::with_capacity(2));
        trace.plan_reads(0x1000, &[(0, 8), (0x170, 4)]);
        let mut plan = Domain43SnapshotPlan::from_trace(trace.into_trace(), limits(0, 2, 12)).unwrap();
        let mut fresh = FixtureMemory::spanning(&[(0x1000, 8), (0x1170, 4)]);
        for offset in 0..4 {
            fresh.0.insert(0x1170 + offset, 0xA5);
        }
        plan.capture_with(|address, out| fresh.read_into(address, out).then_some(1));
        let mut out = [0u8; 4];
        assert!(plan.view().read_into(0x1170, &mut out));
        assert_eq!(out, [0xA5; 4]);
        let trace = Domain43TracingMemory::new(&old, Domain43AccessTrace::with_capacity(1));
        trace.plan_reads(0x1000, &[(0, 8), (0x170, 4)]);
        assert_eq!(Domain43SnapshotPlan::from_trace(trace.into_trace(), limits(0, 2, 12))
            .unwrap_err().cause, D43_SNAPSHOT_TRACE_CAP_CAUSE);
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
        // The uncovered tail remains a probe even though the read overlaps data.
        assert!(!tracing.read_into(0x1004, &mut [0u8; 0x100]));
        let mut plan =
            Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(0, 16, 4096)).unwrap();
        assert_eq!(
            plan.region_specs().collect::<Vec<_>>(),
            vec![spec(0x1000, 8, false), spec(0x1008, 0xFC, true), spec(0x2000, 8, true)]
        );
        assert_eq!(plan.probe_region_count(), 2);
        assert_eq!(plan.byte_count(), 268);

        let receipt = plan.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        assert_eq!(receipt.copied_region_count, 1);
        assert_eq!(receipt.failure_region_ordinal, None, "a probe failure is tolerated");
        assert_eq!(receipt.probe_region_count, 2);
        assert_eq!(receipt.probe_regions_unreadable, 2);
        {
            let view = plan.view();
            assert!(!view.read_into(0x2000, &mut buf));
            assert_eq!(view.first_miss(), None, "an unreadable probe is not a miss");
            assert!(!view.read_into(0x1004, &mut [0u8; 0x100]));
            assert_eq!(view.first_miss(), None);
        }

        let healed = FixtureMemory::spanning(&[(0x1000, 0x104), (0x2000, 0x10)]);
        let receipt = plan.capture_with(|address, out| healed.read_into(address, out).then_some(1));
        assert_eq!(receipt.copied_region_count, 3);
        assert_eq!(receipt.probe_regions_unreadable, 0);
        let view = plan.view();
        assert!(view.read_into(0x2000, &mut buf));
        assert_eq!(buf, [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
        assert_eq!(view.first_miss(), None);
    }

    #[test]
    fn failed_read_overlap_keeps_its_uncovered_probe_tail() {
        let mut memory = FixtureMemory::spanning(&[(0x1000, 8)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(8));
        assert!(tracing.read_into(0x1000, &mut [0; 8]));
        assert!(!tracing.read_into(0x1000, &mut [0; 24]));
        let mut plan = Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits(0, 8, 1024)).unwrap();
        assert_eq!(plan.region_specs().collect::<Vec<_>>(), [spec(0x1000, 8, false), spec(0x1008, 16, true)]);
        let receipt = plan.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        assert_eq!(receipt.failure_region_ordinal, None);
        assert_eq!(receipt.probe_regions_unreadable, 1);
        let view = plan.view();
        assert!(!view.read_into(0x1000, &mut [0; 24]));
        assert_eq!(view.first_miss(), None);
        drop(view);
        memory = FixtureMemory::spanning(&[(0x1000, 24)]);
        assert_eq!(plan.capture_with(|address, out| memory.read_into(address, out).then_some(1)).copied_byte_count, 24);
        let mut bytes = [0; 24];
        assert!(plan.view().read_into(0x1000, &mut bytes));
        assert_eq!(bytes, core::array::from_fn(|i| i as u8));
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
        // Two boundaries inside the data region, one outside it.
        let mut bounds = spans(&[0x1010, 0x1030, 0x2000]);
        let mut plan = plan
            .fit_to_os_regions(limits(64, 16, 4096), |address| {
                asked.push(address);
                bounds(address)
            })
            .unwrap();
        // One query per region the plan enters, not one per range it holds.
        assert_eq!(asked, vec![0x1000, 0x1010, 0x1030, 0x4000]);
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
            plan.fit_to_os_regions(limits(0, 1, 4096), spans(&[0x1004]))
                .unwrap_err(),
            Domain43SnapshotPlanRefusal {
                cause: D43_SNAPSHOT_REGION_CAP_CAUSE,
                region_count: 2,
                byte_count: 8,
            }
        );
    }

    /// kills: the widening dropped, or spent past its own target. The plan is
    /// armed from a walk that read four slots of a roster; the next hour's
    /// walk reads the fifth and the object it points at, neither of which the
    /// traced walk touched. With no slack both are misses, both typed as
    /// inside a region the plan spans; with slack both are served out of the
    /// copy, and the byte count stays under the target.
    #[test]
    fn the_next_element_of_a_grown_roster_is_served_out_of_the_plans_slack() {
        let memory = FixtureMemory::spanning(&[(0x1000, 0x400)]);
        let trace = || {
            let tracing =
                Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
            let mut buf = [0u8; 8];
            for slot in 0..4u64 {
                assert!(tracing.read_into(0x1000 + slot * 8, &mut buf));
            }
            tracing.into_trace()
        };
        let mut buf = [0u8; 8];
        // Slack zero: the fifth slot and the object are outside the plan.
        let mut narrow = Domain43SnapshotPlan::from_trace(trace(), limits(64, 16, 4096))
            .unwrap()
            .fit_to_os_regions(limits(64, 16, 4096), spans(&[0x1000]))
            .unwrap();
        assert_eq!(narrow.widen_slack(), 0);
        assert_eq!(narrow.byte_count(), 32);
        narrow.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        let view = narrow.view();
        assert!(!view.read_into(0x1020, &mut buf), "the fifth slot");
        assert!(!view.read_into(0x1100, &mut buf), "the object it points at");
        // A read in memory the plan never entered is a miss no widening
        // reaches, and is typed apart from the two above.
        assert!(!view.read_into(0x800, &mut buf), "below every region the plan spans");
        assert_eq!(view.misses(), 3);
        assert_eq!(
            view.misses_in_span(),
            2,
            "two are inside the operating-system region the plan spans"
        );
        drop(view);

        let wide_limits = widening_limits(64, 16, 4096, 1024, 512);
        let mut wide = Domain43SnapshotPlan::from_trace(trace(), wide_limits)
            .unwrap()
            .fit_to_os_regions(wide_limits, spans(&[0x1000]))
            .unwrap();
        // (1024 - 32) / (2 * 1) = 496, under the 512 ceiling; the region's own
        // lower edge stops the slack below the traced range.
        assert_eq!(wide.widen_slack(), 496);
        assert_eq!(
            wide.region_specs().collect::<Vec<_>>(),
            vec![spec(0x1000, 32 + 496, false)]
        );
        assert!(wide.byte_count() <= wide_limits.widen_target_bytes);
        wide.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        let view = wide.view();
        assert!(view.read_into(0x1020, &mut buf), "the fifth slot");
        assert_eq!(buf, [0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27]);
        assert!(view.read_into(0x1100, &mut buf), "the object it points at");
        assert_eq!(view.misses(), 0);
        assert_eq!(view.misses_in_span(), 0);
    }

    /// kills: slack that crosses an operating-system boundary (the copy's gate
    /// refuses such a region and the whole frame is lost), slack over memory
    /// no region answers for, slack that swallows a probe region into a data
    /// one, and slack that makes two regions overlap.
    #[test]
    fn slack_stops_at_the_regions_edge_at_a_probe_and_where_no_region_answers() {
        let memory =
            FixtureMemory::spanning(&[(0x1000, 0x1000), (0x2000, 0x400), (0x2408, 0x600)]);
        let tracing = Domain43TracingMemory::new(&memory, Domain43AccessTrace::with_capacity(16));
        let mut buf = [0u8; 8];
        assert!(tracing.read_into(0x1FF8, &mut buf), "the last bytes of one region");
        assert!(tracing.read_into(0x2000, &mut buf), "the first bytes of the next");
        assert!(!tracing.read_into(0x2400, &mut buf), "a probe between them");
        assert!(tracing.read_into(0x2500, &mut buf), "data past the probe");
        assert!(tracing.read_into(0x2900, &mut buf), "data no region answers for");
        let limits = widening_limits(0, 16, 4096, 4096, 256);
        let mut asked = Vec::new();
        let plan = Domain43SnapshotPlan::from_trace(tracing.into_trace(), limits)
            .unwrap()
            .fit_to_os_regions(limits, |address| {
                asked.push(address);
                match address {
                    a if a < 0x2000 => Some((0x1000, 0x2000)),
                    a if a < 0x2800 => Some((0x2000, 0x2800)),
                    _ => None,
                }
            })
            .unwrap();
        assert_eq!(
            plan.region_specs().collect::<Vec<_>>(),
            vec![
                // Clipped at its own region's top edge, never across it.
                spec(0x1EF8, 0x108, false),
                // Clipped at the probe above it.
                spec(0x2000, 0x108, false),
                // A probe is never widened and never merged into data.
                spec(0x2400, 8, true),
                // Clipped at the probe below it.
                spec(0x2408, 0x200, false),
                // No region answers here, so nothing is claimed around it.
                spec(0x2900, 8, false),
            ]
        );
        // One query per region entered, and one for the range above them all.
        assert_eq!(asked, vec![0x1FF8, 0x2000, 0x2900]);
        let mut previous: Option<(u64, u64)> = None;
        for region in plan.region_specs() {
            if let Some((start, end)) = previous {
                assert!(start < region.address, "regions stay ordered");
                assert!(end <= region.address, "and disjoint");
            }
            previous = Some((region.address, region.address + region.len as u64));
        }
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
            .fit_to_os_regions(limits(64, 16, 4096), spans(&[0x3000]))
            .unwrap();
        assert_eq!(
            plan.region_specs().collect::<Vec<_>>(),
            vec![spec(0x1000, 8, false), spec(0x2FFC, 4, true), spec(0x3000, 4, true)]
        );
        let receipt = plan.capture_with(|address, out| memory.read_into(address, out).then_some(1));
        assert_eq!(receipt.probe_region_count, 2);
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

    fn word_at(address: u64) -> u64 {
        let mut bytes = [0u8; 8];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (address + i as u64) as u8;
        }
        u64::from_le_bytes(bytes)
    }

    /// Production-shape timing witness: 33k regions of 244 bytes over a sparse
    /// space and 350k reads in object-local runs that keep jumping back to a
    /// dozen hot objects, the oil walk's shape. Release only:
    /// `cargo test -p collector --release --lib -- --ignored --nocapture
    /// d43_production_shape_view_reads_time`.
    #[test]
    #[ignore]
    fn d43_production_shape_view_reads_time() {
        const REGIONS: u64 = 33_000;
        let mut trace = Domain43AccessTrace::with_capacity(REGIONS as usize);
        let mut starts = Vec::with_capacity(REGIONS as usize);
        for k in 0..REGIONS {
            let start = 0x1_0000_0000 + k * 2_000 + (k * 7_919) % 1_400;
            starts.push(start);
            trace.push(Domain43ReadTrace {
                address: start,
                len: 244,
                succeeded: true,
            });
        }
        let mut plan =
            Domain43SnapshotPlan::from_trace(trace, limits(64, 262_144, 16 << 20)).unwrap();
        plan.capture_with(|address, out| {
            for (i, b) in out.iter_mut().enumerate() {
                *b = (address + i as u64) as u8;
            }
            Some(1)
        });
        let hot: Vec<u64> = (0..15usize)
            .map(|i| starts[(i * 2_749) % starts.len()])
            .collect();
        let mut sequence: Vec<u64> = Vec::new();
        for arm in 0..2usize {
            for root in 0..440usize {
                let base = starts[(root * 61 + arm) % starts.len()];
                for off in 0..30u64 {
                    sequence.push(base + off * 8);
                }
                for slot in 0..24u64 {
                    for (i, h) in hot.iter().enumerate() {
                        sequence.push(h + ((i as u64 + slot) % 3) * 8);
                    }
                }
                for unit in 0..3usize {
                    let base = starts[(root * 7 + unit * 13 + 1) % starts.len()];
                    for off in 0..4u64 {
                        sequence.push(base + off * 16);
                    }
                }
            }
        }
        let expected: u64 = sequence.iter().map(|a| word_at(*a)).fold(0, u64::wrapping_add);

        let view = plan.view();
        let mut buf = [0u8; 8];
        let mut best = u128::MAX;
        let mut checksum = 0u64;
        for _ in 0..5 {
            checksum = 0;
            let started = std::time::Instant::now();
            for address in &sequence {
                assert!(view.read_into(*address, &mut buf));
                checksum = checksum.wrapping_add(u64::from_le_bytes(buf));
            }
            best = best.min(started.elapsed().as_nanos());
        }
        assert_eq!(checksum, expected);
        assert_eq!(view.misses(), 0);
        eprintln!(
            "d43 view: {} reads, best of 5 {} us, {:.1} ns/read",
            sequence.len(),
            best / 1_000,
            best as f64 / sequence.len() as f64
        );
    }
}
