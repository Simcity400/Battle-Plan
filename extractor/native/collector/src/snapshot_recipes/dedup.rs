//! Global interval ownership: every copied byte is owned once.
//!    [`OwnershipMap::end_batch`] folds `FRESH` into it.
//!  * `BODY` — everything older. [`OwnershipMap::end_wave`] folds `PENDING`

//!  * [`OwnershipMap::insert`] asserts both bounds on the way in, which is

use std::sync::Arc;

use super::arena::{charged_push, charged_reserve, hash_table_bytes, vec_bytes, CustodyLedger};
use super::{ConsumerId, CustodyRefusal, FxSet, IntervalId, Span, DEFAULT_CUSTODY_BUDGET_BYTES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dependent {
    pub consumer: ConsumerId,
    pub reach: Span,
}

const LEN_BITS: u32 = 30;
const LEN_MASK: u32 = (1 << LEN_BITS) - 1;
const WITNESSED: u32 = 1 << 30;
/// [`OwnershipMap::spilled_edges`]. See [`OwnershipMap::add_edge`].
const EDGES_SPILLED: u32 = 1 << 31;

const NO_OFFSET: u32 = u32::MAX;

const _: () = assert!(
    DEFAULT_CUSTODY_BUDGET_BYTES <= LEN_MASK as u64,
    "an owned interval's length must fit IntervalRecord's 30 length bits",
);
const _: () = assert!(
    DEFAULT_CUSTODY_BUDGET_BYTES < NO_OFFSET as u64,
    "a custody offset must fit a u32 with NO_OFFSET left over as a sentinel",
);

/// per transaction, more than every ownership lookup in the substrate put
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalRecord {
    start: u64,
    len_and_flags: u32,
    opening_offset: u32,
    closing_offset: u32,
    dependents: u32,
    children: u32,
    id: u32,
}

impl IntervalRecord {
    pub fn id(&self) -> IntervalId {
        IntervalId(self.id)
    }

    #[inline]
    pub fn span(&self) -> Span {
        Span::new(self.start, u64::from(self.len_and_flags & LEN_MASK))
    }

    #[inline]
    pub fn start(&self) -> u64 {
        self.start
    }

    #[inline]
    pub fn len(&self) -> u64 {
        u64::from(self.len_and_flags & LEN_MASK)
    }

    #[inline]
    pub fn opening_offset(&self) -> usize {
        self.opening_offset as usize
    }

    #[inline]
    pub fn closing_offset(&self) -> Option<usize> {
        if self.closing_offset == NO_OFFSET {
            None
        } else {
            Some(self.closing_offset as usize)
        }
    }

    #[inline]
    pub fn witnessed(&self) -> bool {
        self.len_and_flags & WITNESSED != 0
    }

    fn edges_spilled(&self) -> bool {
        self.len_and_flags & EDGES_SPILLED != 0
    }

    pub(super) fn set_closing_offset(&mut self, offset: usize) {
        assert!(
            offset < NO_OFFSET as usize,
            "a closing offset at or past {NO_OFFSET} cannot be told from `no closing copy`; \
             the custody ceiling is what bounds it",
        );
        self.closing_offset = offset as u32;
    }
}

const NO_LINK: u32 = u32::MAX;

/// How long a parent's edge chain may get before [`OwnershipMap::add_edge`]
const EDGE_DEDUP_SCAN: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DependentNode {
    reach_start: u64,
    reach_len: u32,
    next: u32,
    consumer: ConsumerId,
}

impl DependentNode {
    #[inline]
    fn dependent(&self) -> Dependent {
        Dependent {
            consumer: self.consumer,
            reach: Span::new(self.reach_start, u64::from(self.reach_len)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChildNode {
    child: IntervalId,
    consumer: ConsumerId,
    next: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Links {
    dependents: Vec<DependentNode>,
    children: Vec<ChildNode>,
}

impl Links {
    pub fn dependents(&self, record: &IntervalRecord) -> DependentChain<'_> {
        DependentChain {
            nodes: &self.dependents,
            at: record.dependents,
        }
    }

    pub fn children(&self, record: &IntervalRecord) -> ChildChain<'_> {
        ChildChain {
            nodes: &self.children,
            at: record.children,
        }
    }

    pub fn depends(&self, record: &IntervalRecord, consumer: ConsumerId) -> bool {
        self.dependents(record)
            .any(|dependent| dependent.consumer == consumer)
    }

    pub fn reaches(&self, record: &IntervalRecord, consumer: ConsumerId, want: &Span) -> bool {
        self.reach_of(record, consumer)
            .is_some_and(|reach| reach.contains_span(want))
    }

    /// [`OwnershipMap::add_dependent`] walks the chain and HULLS into the node
    pub fn reach_of(&self, record: &IntervalRecord, consumer: ConsumerId) -> Option<Span> {
        self.dependents(record)
            .find(|dependent| dependent.consumer == consumer)
            .map(|dependent| dependent.reach)
    }

    pub fn consumers(&self, record: &IntervalRecord) -> impl Iterator<Item = ConsumerId> + '_ {
        self.dependents(record).map(|dependent| dependent.consumer)
    }

    pub fn has_children(&self, record: &IntervalRecord) -> bool {
        record.children != NO_LINK
    }
}

#[derive(Clone, Debug)]
pub struct DependentChain<'a> {
    nodes: &'a [DependentNode],
    at: u32,
}

impl Iterator for DependentChain<'_> {
    type Item = Dependent;

    fn next(&mut self) -> Option<Dependent> {
        if self.at == NO_LINK {
            return None;
        }
        let node = self.nodes[self.at as usize];
        self.at = node.next;
        Some(node.dependent())
    }
}

#[derive(Clone, Debug)]
pub struct ChildChain<'a> {
    nodes: &'a [ChildNode],
    at: u32,
}

impl Iterator for ChildChain<'_> {
    type Item = (IntervalId, ConsumerId);

    fn next(&mut self) -> Option<(IntervalId, ConsumerId)> {
        if self.at == NO_LINK {
            return None;
        }
        let node = self.nodes[self.at as usize];
        self.at = node.next;
        Some((node.child, node.consumer))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcquisitionPlan {
    pub existing: Vec<IntervalId>,
    pub holes: Vec<Span>,
}

impl AcquisitionPlan {
    pub fn capacity_bytes(&self) -> u64 {
        super::arena::vec_bytes(&self.existing) + super::arena::vec_bytes(&self.holes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    start: u64,
    last_delta: u32,
    id: u32,
}

impl Entry {
    #[inline]
    fn last(&self) -> u64 {
        self.start.wrapping_add(u64::from(self.last_delta))
    }
}

#[cfg(test)]
pub(super) const INDEX_ENTRY_BYTES: usize = std::mem::size_of::<Entry>();

const RUN_COUNT: usize = 3;
const BODY: usize = 0;
const PENDING: usize = 1;
const FRESH: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    at: [u32; RUN_COUNT],
}

fn last_addr(span: Span) -> Option<u64> {
    if span.len == 0 {
        return None;
    }
    Some(span.start.checked_add(span.len - 1).unwrap_or(u64::MAX))
}

#[derive(Debug)]
pub struct OwnershipMap {
    records: Vec<IntervalRecord>,
    links: Links,
    runs: [Vec<Entry>; RUN_COUNT],
    scratch: Vec<Entry>,
    spilled_edges: FxSet<(u32, u32, u32)>,
    spilled_edges_charged: u64,
    witnessed: Vec<IntervalId>,
    ledger: Arc<CustodyLedger>,
}

impl Default for OwnershipMap {
    fn default() -> Self {
        Self::on_ledger(CustodyLedger::new(DEFAULT_CUSTODY_BUDGET_BYTES))
    }
}

impl OwnershipMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// An ownership map charged to an existing observation's ledger — the
    pub fn on_ledger(ledger: Arc<CustodyLedger>) -> Self {
        Self {
            records: Vec::new(),
            links: Links::default(),
            runs: Default::default(),
            scratch: Vec::new(),
            spilled_edges: FxSet::default(),
            spilled_edges_charged: 0,
            witnessed: Vec::new(),
            ledger,
        }
    }

    pub fn table_bytes(&self) -> u64 {
        self.ledger.table_bytes()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn records(&self) -> &[IntervalRecord] {
        &self.records
    }

    pub fn record(&self, id: IntervalId) -> &IntervalRecord {
        &self.records[id.0 as usize]
    }

    pub fn record_mut(&mut self, id: IntervalId) -> &mut IntervalRecord {
        &mut self.records[id.0 as usize]
    }

    pub fn links(&self) -> &Links {
        &self.links
    }

    pub fn witnessed_ids(&self) -> &[IntervalId] {
        &self.witnessed
    }

    pub fn into_frozen(mut self) -> Result<(Vec<IntervalRecord>, Links), CustodyRefusal> {
        self.end_wave()?;
        let index = &self.runs[BODY];
        let already_ordered = index.len() == self.records.len()
            && index
                .iter()
                .enumerate()
                .all(|(position, entry)| entry.id as usize == position);
        let released = vec_bytes(&self.runs[BODY])
            + vec_bytes(&self.runs[PENDING])
            + vec_bytes(&self.runs[FRESH])
            + vec_bytes(&self.scratch)
            + vec_bytes(&self.witnessed)
            + self.spilled_edges_charged;
        self.ledger.release_tables(released);

        let mut records = self.records;
        if !already_ordered {
            records.sort_unstable_by_key(|record| record.start);
        }
        Ok((records, self.links))
    }

    pub fn sweep(&self) -> Cursor {
        Cursor::default()
    }

    pub fn overlapping(&self, span: Span) -> Vec<IntervalId> {
        let mut out = Vec::new();
        self.overlapping_into(span, &mut out);
        out
    }

    pub fn overlapping_into(&self, span: Span, out: &mut Vec<IntervalId>) {
        let mut cursor = Cursor::default();
        self.sweep_overlapping(&mut cursor, span, out);
    }

    pub fn sweep_overlapping(&self, cursor: &mut Cursor, span: Span, out: &mut Vec<IntervalId>) {
        out.clear();
        let last = match last_addr(span) {
            Some(last) => last,
            None => return,
        };
        self.advance(cursor, span.start);
        self.collect(cursor, last, out);
    }

    pub fn plan(&self, span: Span) -> AcquisitionPlan {
        let mut plan = AcquisitionPlan::default();
        self.plan_into(span, &mut plan);
        plan
    }

    pub fn plan_into(&self, span: Span, plan: &mut AcquisitionPlan) {
        let mut cursor = Cursor::default();
        self.sweep_plan(&mut cursor, span, plan);
    }

    pub fn sweep_plan(&self, cursor: &mut Cursor, span: Span, plan: &mut AcquisitionPlan) {
        plan.existing.clear();
        plan.holes.clear();
        if span.is_empty() {
            return;
        }
        let mut cursor_addr = u128::from(span.start);
        let end = span.end_excl();
        self.sweep_overlapping(cursor, span, &mut plan.existing);
        for position in 0..plan.existing.len() {
            let owned = self.records[plan.existing[position].0 as usize].span();
            let owned_start = u128::from(owned.start);
            if owned_start > cursor_addr {
                plan.holes.push(hole(cursor_addr, owned_start.min(end)));
            }
            cursor_addr = cursor_addr.max(owned.end_excl());
            if cursor_addr >= end {
                break;
            }
        }
        if cursor_addr < end {
            plan.holes.push(hole(cursor_addr, end));
        }
        plan.holes.retain(|h| !h.is_empty());
    }

    /// Take ownership of a previously unowned `span` copied at
    fn check_custody_bounds(span: Span, opening_offset: usize) {
        assert!(
            span.len <= LEN_MASK as u64,
            "an owned interval of {} B is past the {LEN_MASK} B the record can hold; \
             it can only exist if custody admitted it",
            span.len,
        );
        assert!(
            opening_offset < NO_OFFSET as usize,
            "a custody offset of {opening_offset} is past the {NO_OFFSET} the record can hold",
        );
    }

    /// ownership index, and this is the row that answers it.
    #[cfg(test)]
    pub(super) fn reserve_for(&mut self, intervals: usize) -> Result<(), CustodyRefusal> {
        charged_reserve(&mut self.records, &self.ledger, intervals)?;
        charged_reserve(&mut self.runs[FRESH], &self.ledger, intervals)
    }

    pub fn insert(
        &mut self,
        span: Span,
        opening_offset: usize,
    ) -> Result<IntervalId, CustodyRefusal> {
        Self::check_custody_bounds(span, opening_offset);
        let id = IntervalId(self.records.len() as u32);
        charged_push(
            &mut self.records,
            &self.ledger,
            IntervalRecord {
                start: span.start,
                len_and_flags: span.len as u32,
                opening_offset: opening_offset as u32,
                closing_offset: NO_OFFSET,
                dependents: NO_LINK,
                children: NO_LINK,
                id: id.0,
            },
        )?;
        if let Some(last) = last_addr(span) {
            let entry = Entry {
                start: span.start,
                last_delta: (last - span.start) as u32,
                id: id.0,
            };
            charged_reserve(&mut self.runs[FRESH], &self.ledger, 1)?;
            let fresh = &mut self.runs[FRESH];
            match fresh.last() {
                Some(tail) if tail.start > entry.start => {
                    let at = fresh.partition_point(|held| held.start < entry.start);
                    fresh.insert(at, entry);
                }
                _ => fresh.push(entry),
            }
        }
        Ok(id)
    }

    pub fn end_batch(&mut self) -> Result<(), CustodyRefusal> {
        self.fold(PENDING, FRESH)
    }

    pub fn end_wave(&mut self) -> Result<(), CustodyRefusal> {
        self.fold(PENDING, FRESH)?;
        self.fold(BODY, PENDING)
    }

    pub fn add_dependent(
        &mut self,
        id: IntervalId,
        consumer: ConsumerId,
        requested: Span,
    ) -> Result<(), CustodyRefusal> {
        let record = self.records[id.0 as usize];
        let clipped = clip(requested, record.span());
        if clipped.is_empty() {
            return Ok(());
        }
        let mut at = record.dependents;
        while at != NO_LINK {
            let node = &mut self.links.dependents[at as usize];
            if node.consumer == consumer {
                let widened = Span::new(node.reach_start, u64::from(node.reach_len)).hull(&clipped);
                debug_assert!(
                    widened.len <= LEN_MASK as u64,
                    "a reach is a sub-span of one owned interval, so the hull of two of \
                     them is bounded by the same custody ceiling the interval is",
                );
                node.reach_start = widened.start;
                node.reach_len = widened.len as u32;
                return Ok(());
            }
            at = node.next;
        }
        let node = self.links.dependents.len() as u32;
        debug_assert!(clipped.len <= LEN_MASK as u64);
        charged_push(
            &mut self.links.dependents,
            &self.ledger,
            DependentNode {
                reach_start: clipped.start,
                reach_len: clipped.len as u32,
                next: record.dependents,
                consumer,
            },
        )?;
        self.records[id.0 as usize].dependents = node;
        Ok(())
    }

    pub fn add_edge(
        &mut self,
        parent: IntervalId,
        child: IntervalId,
        consumer: ConsumerId,
    ) -> Result<(), CustodyRefusal> {
        if parent == child {
            return Ok(());
        }
        let record = self.records[parent.0 as usize];
        let head = record.children;
        if record.edges_spilled() {
            if !self.insert_spilled_edge(edge_key(parent, child, consumer))? {
                return Ok(());
            }
        } else {
            let mut at = head;
            let mut scanned = 0u32;
            let mut spilled = false;
            while at != NO_LINK {
                let node = self.links.children[at as usize];
                if node.child == child && node.consumer == consumer {
                    return Ok(());
                }
                at = node.next;
                scanned += 1;
                if scanned == EDGE_DEDUP_SCAN {
                    spilled = true;
                    break;
                }
            }
            if spilled {
                self.spill_edges(parent)?;
                if !self.insert_spilled_edge(edge_key(parent, child, consumer))? {
                    return Ok(());
                }
            }
        }
        let node = self.links.children.len() as u32;
        charged_push(
            &mut self.links.children,
            &self.ledger,
            ChildNode {
                child,
                consumer,
                next: head,
            },
        )?;
        self.records[parent.0 as usize].children = node;
        Ok(())
    }

    fn insert_spilled_edge(&mut self, key: (u32, u32, u32)) -> Result<bool, CustodyRefusal> {
        let capacity = self.spilled_edges.capacity();
        if self.spilled_edges.len() == capacity {
            let predicted = hash_table_bytes::<(u32, u32, u32)>(capacity.max(1) * 2)
                - self.spilled_edges_charged;
            self.ledger.charge_tables(predicted)?;
            self.spilled_edges_charged += predicted;
        }
        let fresh = self.spilled_edges.insert(key);
        let footprint = hash_table_bytes::<(u32, u32, u32)>(self.spilled_edges.capacity());
        if footprint > self.spilled_edges_charged {
            let overshoot = footprint - self.spilled_edges_charged;
            self.spilled_edges_charged = footprint;
            self.ledger.charge_tables_overshoot(overshoot)?;
        }
        Ok(fresh)
    }

    fn spill_edges(&mut self, parent: IntervalId) -> Result<(), CustodyRefusal> {
        let mut at = self.records[parent.0 as usize].children;
        while at != NO_LINK {
            let node = self.links.children[at as usize];
            self.insert_spilled_edge(edge_key(parent, node.child, node.consumer))?;
            at = node.next;
        }
        self.records[parent.0 as usize].len_and_flags |= EDGES_SPILLED;
        Ok(())
    }

    #[inline]
    pub fn find_hinted(&self, addr: u64, hint: IntervalId) -> Option<IntervalId> {
        if let Some(record) = self.records.get(hint.0 as usize) {
            if record.start() <= addr && addr - record.start() < record.len() {
                return Some(hint);
            }
        }
        self.find(addr)
    }

    pub fn find(&self, addr: u64) -> Option<IntervalId> {
        for run in &self.runs {
            let at = run.partition_point(|entry| entry.last() < addr);
            if let Some(entry) = run.get(at) {
                if entry.start <= addr {
                    return Some(IntervalId(entry.id));
                }
            }
        }
        None
    }

    pub fn read_at<'a>(
        &self,
        arena: &'a super::arena::Arena,
        addr: u64,
        len: usize,
    ) -> Option<(&'a [u8], IntervalId)> {
        let id = self.find(addr)?;
        let record = &self.records[id.0 as usize];
        let want = Span::new(addr, len as u64);
        if !record.span().contains_span(&want) {
            return None;
        }
        let skew = (addr - record.start) as usize;
        Some((arena.slice(record.opening_offset() + skew, len), id))
    }

    pub fn read<'a>(
        &self,
        arena: &'a super::arena::Arena,
        addr: u64,
        len: usize,
    ) -> Option<&'a [u8]> {
        self.read_at(arena, addr, len).map(|(bytes, _)| bytes)
    }

    pub fn enrol_witness(&mut self, id: IntervalId) -> Result<(), CustodyRefusal> {
        let record = &mut self.records[id.0 as usize];
        if record.witnessed() {
            return Ok(());
        }
        record.len_and_flags |= WITNESSED;
        charged_push(&mut self.witnessed, &self.ledger, id)
    }

    fn advance(&self, cursor: &mut Cursor, addr: u64) {
        for run_index in 0..RUN_COUNT {
            let run = &self.runs[run_index];
            let mut at = cursor.at[run_index] as usize;
            if at < run.len() && run[at].last() < addr {
                let mut step = 1usize;
                while at + step < run.len() && run[at + step].last() < addr {
                    at += step;
                    step *= 2;
                }
                let hi = at.saturating_add(step).saturating_add(1).min(run.len());
                at += run[at..hi].partition_point(|entry| entry.last() < addr);
            }
            cursor.at[run_index] = at as u32;
        }
    }

    fn collect(&self, cursor: &Cursor, last: u64, out: &mut Vec<IntervalId>) {
        let mut at = cursor.at;
        loop {
            let mut pick = RUN_COUNT;
            let mut best = 0u64;
            for run_index in 0..RUN_COUNT {
                let run = &self.runs[run_index];
                let position = at[run_index] as usize;
                if position < run.len() {
                    let start = run[position].start;
                    if pick == RUN_COUNT || start < best {
                        pick = run_index;
                        best = start;
                    }
                }
            }
            if pick == RUN_COUNT || best > last {
                return;
            }
            let entry = self.runs[pick][at[pick] as usize];
            at[pick] += 1;
            out.push(IntervalId(entry.id));
        }
    }

    fn fold(&mut self, dst: usize, src: usize) -> Result<(), CustodyRefusal> {
        if self.runs[src].is_empty() {
            return Ok(());
        }
        if self.runs[dst].is_empty() {
            self.runs.swap(dst, src);
            return Ok(());
        }
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        let wanted = self.runs[dst].len() + self.runs[src].len();
        if let Err(refusal) = charged_reserve(&mut out, &self.ledger, wanted) {
            self.scratch = out;
            return Err(refusal);
        }
        let (left, right) = (&self.runs[dst], &self.runs[src]);
        let (mut i, mut j) = (0usize, 0usize);
        while i < left.len() && j < right.len() {
            if left[i].start <= right[j].start {
                out.push(left[i]);
                i += 1;
            } else {
                out.push(right[j]);
                j += 1;
            }
        }
        out.extend_from_slice(&left[i..]);
        out.extend_from_slice(&right[j..]);
        std::mem::swap(&mut self.runs[dst], &mut out);
        out.clear();
        self.scratch = out;
        self.runs[src].clear();
        Ok(())
    }
}

fn edge_key(parent: IntervalId, child: IntervalId, consumer: ConsumerId) -> (u32, u32, u32) {
    (parent.0, child.0, u32::from(consumer.0))
}

fn clip(requested: Span, interval: Span) -> Span {
    let start = u128::from(requested.start).max(u128::from(interval.start));
    let end = requested.end_excl().min(interval.end_excl());
    if end <= start {
        return Span::new(0, 0);
    }
    hole(start, end)
}

fn hole(start: u128, end: u128) -> Span {
    Span::new(
        u64::try_from(start).unwrap_or(u64::MAX),
        u64::try_from(end.saturating_sub(start)).unwrap_or(u64::MAX),
    )
}
