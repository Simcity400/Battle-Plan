//! Global interval ownership: every copied byte is owned once.
//!
//! t35 measured 115.906 MiB summed over brackets against a 58.752 MiB union,
//! so independently copied per-domain snapshots surrender close to a factor of
//! two. The re-pricing made deduplication an architectural requirement rather
//! than an optimisation, with one condition attached: "shared extents carry a
//! dependency list so a failed witness gaps only affected domains".
//!
//! This map is that structure.
//!
//! # Invariants
//!
//! * Owned intervals are pairwise DISJOINT and sorted by address. A request
//!   that partly overlaps an existing interval is split: the already-owned
//!   part is not re-copied, the holes become new intervals.
//! * Every interval carries a dependency list of [`ConsumerId`]s. A consumer
//!   that arrives second at a shared extent pays no copy and is added to the
//!   list.
//! * Every interval carries the discovery edges OUT of it: the intervals whose
//!   addresses were decoded from its bytes. That edge set is what lets a
//!   changed closing witness invalidate exactly its dependent subtree.
//!
//! # What deduplication does NOT do
//!
//! It does not merge across recipes. Two recipes' requests that abut still
//! produce two intervals if they arrived separately; only exact overlap is
//! removed. Merging is [`super::policy`]'s job and is per-recipe, because the
//! policy is per-recipe data.
//!
//! # The address index is swept, not probed
//!
//! The first version of this map kept `BTreeMap<u64, u32>` from start address
//! to record. That is the obvious structure and it is the wrong one: the
//! executor issues FIVE ordered-map traversals per copy transaction (two to
//! plan the acquisition, one to insert, two to attach dependents and edges),
//! and at a hundred thousand owned intervals those tree walks were three
//! quarters of the substrate's entire per-transaction cost.
//!
//! What replaces them exploits the shape of the work. A wave's copies come out
//! of [`super::policy::plan_copies_into`] ALREADY sorted by address and
//! pairwise disjoint, and its requests come with a sorted index alongside. Two
//! sorted sequences meet in a merge, not in a search: the executor takes one
//! [`Cursor`], walks a batch of queries through it in ascending address order,
//! and the cursor walks the owned set once. Per query the cost is the overlaps
//! it actually reports plus a constant, with no lookup at all.
//!
//! The owned set is therefore three address-sorted arrays of [`Entry`] rather
//! than one, because a batch's new intervals land in the middle of the address
//! space and re-merging the whole index after every batch would cost more than
//! the probes it replaced:
//!
//!  * `FRESH` — the intervals this batch has inserted, appended in ascending
//!    order as the sweep discovers holes.
//!  * `PENDING` — the intervals earlier batches of this wave inserted.
//!    [`OwnershipMap::end_batch`] folds `FRESH` into it.
//!  * `BODY` — everything older. [`OwnershipMap::end_wave`] folds `PENDING`
//!    into it, so the whole index is re-merged once per wave (fourteen times
//!    per observation) rather than once per copy transaction.
//!
//! A sweep therefore carries three positions and picks the smallest start
//! among them. Three is a fixed constant, so the query stays O(1) plus its own
//! output, and the folds are linear passes whose total cost over an
//! observation is bounded by the wave count times the interval count.
//!
//! Correctness does not depend on which run an interval is in: all three are
//! address-sorted, and every owned interval is disjoint from every other one
//! in every run, so "sorted by start" is a total order over their union.

use super::{ConsumerId, FxSet, IntervalId, Span};

/// One consumer's claim on an interval.
///
/// `reach` is the hull of the sub-spans that consumer actually asked for
/// INSIDE this interval, not the interval itself. Deduplication and gap
/// coalescing both make an owned interval wider than any one request: a second
/// consumer that overlaps the first consumer's object by a byte would
/// otherwise be able to read the whole object. `reach` is bounded by the
/// consumer's own requests, so it cannot reach another consumer's bytes.
///
/// It is a hull rather than an exact set, so a consumer that makes two
/// disjoint requests into one interval can also read between them. Those bytes
/// are inside its own footprint, never another consumer's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dependent {
    pub consumer: ConsumerId,
    pub reach: Span,
}

/// One owned, copied interval.
///
/// The dependency list and the discovery edges are NOT stored here. Both are
/// one-or-two element lists on almost every interval, and a `Vec` per list is
/// a heap allocation per copied interval — measured at about fifty nanoseconds
/// per transaction, more than every ownership lookup in the substrate put
/// together. They live in [`Links`] instead, as chains through two shared
/// arenas, which also takes the record itself from 96 bytes to 56.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalRecord {
    pub id: IntervalId,
    /// The ORIGINAL address range. Kept for translation and for the closing
    /// witness, which re-copies these exact addresses.
    pub span: Span,
    /// Offset of the opening copy in the arena.
    pub opening_offset: usize,
    /// Offset of the closing copy, once the closing wave has run.
    pub closing_offset: Option<usize>,
    /// Whether the closing wave must re-copy this interval.
    pub witnessed: bool,
    /// Head of this interval's dependency chain in [`Links`], or [`NO_LINK`].
    dependents: u32,
    /// Head of this interval's discovery-edge chain in [`Links`].
    children: u32,
    /// Whether this interval's edge membership has spilled into
    /// [`OwnershipMap::spilled_edges`]. See [`OwnershipMap::add_edge`].
    ///
    /// Free: the record has three bytes of padding either way.
    edges_spilled: bool,
}

/// End of a chain in [`Links`].
const NO_LINK: u32 = u32::MAX;

/// How long a parent's edge chain may get before [`OwnershipMap::add_edge`]
/// stops walking it and answers from a set instead.
const EDGE_DEDUP_SCAN: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DependentNode {
    dependent: Dependent,
    next: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChildNode {
    child: IntervalId,
    consumer: ConsumerId,
    next: u32,
}

/// The dependency lists and discovery edges of every interval, in two shared
/// arenas.
///
/// Chains are built by prepending, so a chain reads back in reverse arrival
/// order. Nothing depends on the order: every consumer of a chain either asks
/// whether it contains something, or walks all of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Links {
    dependents: Vec<DependentNode>,
    children: Vec<ChildNode>,
}

impl Links {
    /// The consumers depending on `record`, with how far each one reached.
    pub fn dependents(&self, record: &IntervalRecord) -> DependentChain<'_> {
        DependentChain {
            nodes: &self.dependents,
            at: record.dependents,
        }
    }

    /// The intervals discovered THROUGH `record`'s bytes, each labelled with
    /// the consumer whose recipe walked that edge.
    ///
    /// The label is what keeps invalidation per consumer. Deduplication means
    /// one interval can be BOTH a discovered child of A's chain and a root of
    /// B's: an unlabelled edge would gap B when A's parent moved, even though
    /// B never went through that parent and B's own bytes were witnessed
    /// unchanged.
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

    /// Whether `consumer` may see `want` through `record`. Depending on the
    /// interval is not enough — the bytes must be inside what it reached for.
    pub fn reaches(&self, record: &IntervalRecord, consumer: ConsumerId, want: &Span) -> bool {
        self.dependents(record)
            .any(|dependent| dependent.consumer == consumer && dependent.reach.contains_span(want))
    }

    pub fn consumers(&self, record: &IntervalRecord) -> impl Iterator<Item = ConsumerId> + '_ {
        self.dependents(record).map(|dependent| dependent.consumer)
    }

    pub fn has_children(&self, record: &IntervalRecord) -> bool {
        record.children != NO_LINK
    }
}

/// Walk of one interval's dependency chain.
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
        Some(node.dependent)
    }
}

/// Walk of one interval's discovery-edge chain.
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

/// What an acquisition needs: which parts of a span are already owned, and
/// which are holes that must be copied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcquisitionPlan {
    /// Already-owned intervals overlapping the request.
    pub existing: Vec<IntervalId>,
    /// Sub-spans of the request nobody owns yet, in address order.
    pub holes: Vec<Span>,
}

/// One owned interval, projected into the address index.
///
/// `last` is the INCLUSIVE last address rather than an exclusive end. An
/// interval that runs to the top of the address space has an exclusive end of
/// `u64::MAX + 1`, which is why [`Span::end_excl`] is computed in `u128`;
/// carrying the inclusive end keeps every comparison in the sweep a plain
/// `u64` compare without reintroducing that truncation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    start: u64,
    last: u64,
    id: u32,
}

/// Address-sorted runs of the index. See the module header.
const RUN_COUNT: usize = 3;
/// Everything folded in before the current wave.
const BODY: usize = 0;
/// Intervals inserted by earlier batches of the current wave.
const PENDING: usize = 1;
/// Intervals inserted by the current batch.
const FRESH: usize = 2;

/// Position of one monotone sweep inside every index run.
///
/// A cursor is only meaningful against the map it was made from, and only for
/// queries issued in ascending `start` order. Both facts are the point: it is
/// what turns a batch of per-transaction lookups into one pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    at: [u32; RUN_COUNT],
}

/// The inclusive last address of a non-empty span, or `None` when it is empty.
///
/// Saturates at the top of the address space. Only a span whose exclusive end
/// is past `2^64` can reach that arm, and no interval can be owned there, so
/// clipping is equivalent to [`Span::overlaps`]'s `u128` comparison.
fn last_addr(span: Span) -> Option<u64> {
    if span.len == 0 {
        return None;
    }
    Some(span.start.checked_add(span.len - 1).unwrap_or(u64::MAX))
}

/// The observation-wide interval map.
#[derive(Debug, Default)]
pub struct OwnershipMap {
    records: Vec<IntervalRecord>,
    links: Links,
    /// The address index, as three sorted runs. See the module header.
    runs: [Vec<Entry>; RUN_COUNT],
    /// Merge buffer for the folds, held so a fold does not allocate.
    scratch: Vec<Entry>,
    /// Exact edge membership for the parents whose chains outgrew
    /// [`EDGE_DEDUP_SCAN`], keyed `(parent, child, consumer)`. See
    /// [`Self::add_edge`].
    spilled_edges: FxSet<(u32, u32, u32)>,
    /// Intervals enrolled in the closing wave, in enrolment order. Kept as a
    /// list so the closing wave does not re-scan every record to find the
    /// witnessed ones.
    witnessed: Vec<IntervalId>,
}

impl OwnershipMap {
    pub fn new() -> Self {
        Self::default()
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

    /// Intervals enrolled in the closing witness wave, in enrolment order.
    pub fn witnessed_ids(&self) -> &[IntervalId] {
        &self.witnessed
    }

    /// The records permuted into ascending address order, with the link
    /// arenas they index into.
    ///
    /// The index already knows that permutation, so the common case — records
    /// inserted in ascending order, which is what a sorted batch sweep
    /// produces — costs one pass over the index and no record moves at all.
    /// The links come along untouched: a record's chain heads are arena
    /// indices, so reordering the records does not disturb them.
    pub fn into_frozen(mut self) -> (Vec<IntervalRecord>, Links) {
        self.end_wave();
        let index = &self.runs[BODY];
        let already_ordered = index.len() == self.records.len()
            && index
                .iter()
                .enumerate()
                .all(|(position, entry)| entry.id as usize == position);
        let mut records = self.records;
        if !already_ordered {
            records.sort_unstable_by_key(|record| record.span.start);
        }
        (records, self.links)
    }

    /// A cursor positioned before every owned interval.
    pub fn sweep(&self) -> Cursor {
        Cursor::default()
    }

    /// Every owned interval overlapping `span`, in address order.
    pub fn overlapping(&self, span: Span) -> Vec<IntervalId> {
        let mut out = Vec::new();
        self.overlapping_into(span, &mut out);
        out
    }

    /// [`Self::overlapping`] into a buffer the caller owns and reuses, without
    /// a sweep cursor: the index is entered by bisection every time.
    ///
    /// This is the entry point for callers whose queries do not arrive in
    /// address order. The executor's wave paths use
    /// [`Self::sweep_overlapping`] instead.
    pub fn overlapping_into(&self, span: Span, out: &mut Vec<IntervalId>) {
        let mut cursor = Cursor::default();
        self.sweep_overlapping(&mut cursor, span, out);
    }

    /// [`Self::overlapping_into`] for a batch of queries that arrive in
    /// ascending `start` order, sharing one [`Cursor`].
    ///
    /// The cursor may be advanced past an interval only when no LATER query in
    /// the batch can overlap it, which is exactly "it ends before this query
    /// starts": a query that starts later still cannot reach back. Intervals
    /// the cursor has not passed are re-reported for as many queries as
    /// overlap them, so overlapping queries in one batch are as correct as
    /// disjoint ones.
    pub fn sweep_overlapping(&self, cursor: &mut Cursor, span: Span, out: &mut Vec<IntervalId>) {
        out.clear();
        let last = match last_addr(span) {
            Some(last) => last,
            None => return,
        };
        self.advance(cursor, span.start);
        self.collect(cursor, last, out);
    }

    /// Split `span` into the parts already owned and the parts that must be
    /// copied.
    pub fn plan(&self, span: Span) -> AcquisitionPlan {
        let mut plan = AcquisitionPlan::default();
        self.plan_into(span, &mut plan);
        plan
    }

    /// [`Self::plan`] into a buffer the caller owns and reuses.
    pub fn plan_into(&self, span: Span, plan: &mut AcquisitionPlan) {
        let mut cursor = Cursor::default();
        self.sweep_plan(&mut cursor, span, plan);
    }

    /// [`Self::plan_into`] sharing one [`Cursor`] across a batch of spans that
    /// arrive in ascending `start` order.
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
            let owned = self.records[plan.existing[position].0 as usize].span;
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
    /// `opening_offset`.
    ///
    /// The caller is responsible for having asked [`Self::plan`] first: this
    /// does not check for overlap, because the only caller that can produce an
    /// arena offset is the one that just filled the hole.
    ///
    /// A sweep discovers holes in ascending order, so the index entry appends;
    /// the bisecting arm keeps the run sorted for callers that insert out of
    /// order. It is what makes the folds a COST decision rather than a
    /// correctness one — an unfolded run is slower to insert into and answers
    /// exactly the same queries.
    ///
    /// A live [`Cursor`] survives either arm. The bisecting arm shifts
    /// entries, but never one the cursor has already passed: every entry
    /// before the cursor ends before the current query starts, and the new
    /// entry is a HOLE of that query, so it starts at or after it. The
    /// insertion point is therefore never earlier than the cursor, and an
    /// entry that lands exactly on the cursor is one the cursor should be
    /// reporting.
    pub fn insert(&mut self, span: Span, opening_offset: usize) -> IntervalId {
        let id = IntervalId(self.records.len() as u32);
        self.records.push(IntervalRecord {
            id,
            span,
            opening_offset,
            closing_offset: None,
            witnessed: false,
            dependents: NO_LINK,
            children: NO_LINK,
            edges_spilled: false,
        });
        // An empty span overlaps nothing (`Span::overlaps` is false for it and
        // `contains_addr` cannot be true), so it is deliberately not indexed:
        // an index entry for it would be reported by a sweep that a direct
        // `Span` comparison refuses.
        if let Some(last) = last_addr(span) {
            let entry = Entry {
                start: span.start,
                last,
                id: id.0,
            };
            let fresh = &mut self.runs[FRESH];
            match fresh.last() {
                Some(tail) if tail.start > entry.start => {
                    let at = fresh.partition_point(|held| held.start < entry.start);
                    fresh.insert(at, entry);
                }
                _ => fresh.push(entry),
            }
        }
        id
    }

    /// Fold this batch's index entries into the wave's pending run.
    ///
    /// Call it between batches. A batch is one recipe's copies in one wave —
    /// the unit whose spans are sorted and disjoint, which is what makes the
    /// batch's inserts an ascending append.
    pub fn end_batch(&mut self) {
        self.fold(PENDING, FRESH);
    }

    /// Fold the wave's index entries into the body.
    pub fn end_wave(&mut self) {
        self.fold(PENDING, FRESH);
        self.fold(BODY, PENDING);
    }

    /// Record that `consumer`'s hour depends on the part of `id` that
    /// `requested` covers.
    ///
    /// `requested` is CLIPPED to the interval: a request that spans several
    /// intervals grants reach in each one only over its own share.
    pub fn add_dependent(&mut self, id: IntervalId, consumer: ConsumerId, requested: Span) {
        let record = self.records[id.0 as usize];
        let clipped = clip(requested, record.span);
        if clipped.is_empty() {
            return;
        }
        let mut at = record.dependents;
        while at != NO_LINK {
            let node = &mut self.links.dependents[at as usize];
            if node.dependent.consumer == consumer {
                node.dependent.reach = node.dependent.reach.hull(&clipped);
                return;
            }
            at = node.next;
        }
        let node = self.links.dependents.len() as u32;
        self.links.dependents.push(DependentNode {
            dependent: Dependent {
                consumer,
                reach: clipped,
            },
            next: record.dependents,
        });
        self.records[id.0 as usize].dependents = node;
    }

    /// Record that `consumer`'s recipe decoded `child`'s address out of
    /// `parent`'s bytes.
    ///
    /// An edge already recorded is never recorded twice.
    ///
    /// # How the answer is reached, and why it is not one walk
    ///
    /// Almost every parent has a handful of edges, and for those a walk of the
    /// chain is a few comparisons in cache and the cheapest possible answer.
    /// But a parent can have as many edges as a decoded container has
    /// elements — every element's `DecodePointer` reads its field out of the
    /// one array interval — and walking the whole chain per edge is quadratic
    /// in that count: ten thousand elements is fifty million comparisons for
    /// one parent.
    ///
    /// So the walk is bounded at [`EDGE_DEDUP_SCAN`], and a parent that
    /// reaches the bound SPILLS: its whole chain goes into `spilled_edges`
    /// once, and every later question about that parent is one hash lookup.
    /// The spill is what keeps the answer exact — a bound with nothing behind
    /// it would let an edge deeper than the bound be stored a second time, and
    /// `Links::children` is public, so that count is observable.
    ///
    /// A parent that never spills pays nothing for the mechanism: the flag is
    /// on the record, so an unspilled parent never touches the set.
    pub fn add_edge(&mut self, parent: IntervalId, child: IntervalId, consumer: ConsumerId) {
        if parent == child {
            return;
        }
        let record = self.records[parent.0 as usize];
        let head = record.children;
        if record.edges_spilled {
            if !self.spilled_edges.insert(edge_key(parent, child, consumer)) {
                return;
            }
        } else {
            let mut at = head;
            let mut scanned = 0u32;
            let mut spilled = false;
            while at != NO_LINK {
                let node = self.links.children[at as usize];
                if node.child == child && node.consumer == consumer {
                    return;
                }
                at = node.next;
                scanned += 1;
                if scanned == EDGE_DEDUP_SCAN {
                    spilled = true;
                    break;
                }
            }
            if spilled {
                self.spill_edges(parent);
                if !self.spilled_edges.insert(edge_key(parent, child, consumer)) {
                    return;
                }
            }
        }
        let node = self.links.children.len() as u32;
        self.links.children.push(ChildNode {
            child,
            consumer,
            next: head,
        });
        self.records[parent.0 as usize].children = node;
    }

    /// Copy `parent`'s whole edge chain into `spilled_edges` and mark it, so
    /// that from here on its membership question is answered by the set alone.
    fn spill_edges(&mut self, parent: IntervalId) {
        let mut at = self.records[parent.0 as usize].children;
        while at != NO_LINK {
            let node = self.links.children[at as usize];
            self.spilled_edges
                .insert(edge_key(parent, node.child, node.consumer));
            at = node.next;
        }
        self.records[parent.0 as usize].edges_spilled = true;
    }

    /// The interval containing `addr`, if any.
    pub fn find(&self, addr: u64) -> Option<IntervalId> {
        for run in &self.runs {
            let at = run.partition_point(|entry| entry.last < addr);
            if let Some(entry) = run.get(at) {
                // Owned intervals are disjoint, so at most one run can hold
                // `addr` and the first hit is the only hit.
                if entry.start <= addr {
                    return Some(IntervalId(entry.id));
                }
            }
        }
        None
    }

    /// Read `len` bytes at `addr` out of the arena, if one interval holds all
    /// of them, with the interval that held them.
    pub fn read_at<'a>(
        &self,
        arena: &'a super::arena::Arena,
        addr: u64,
        len: usize,
    ) -> Option<(&'a [u8], IntervalId)> {
        let id = self.find(addr)?;
        let record = &self.records[id.0 as usize];
        let want = Span::new(addr, len as u64);
        if !record.span.contains_span(&want) {
            return None;
        }
        let skew = (addr - record.span.start) as usize;
        Some((arena.slice(record.opening_offset + skew, len), id))
    }

    /// Read `len` bytes at `addr` out of the arena, if one interval holds all
    /// of them.
    pub fn read<'a>(
        &self,
        arena: &'a super::arena::Arena,
        addr: u64,
        len: usize,
    ) -> Option<&'a [u8]> {
        self.read_at(arena, addr, len).map(|(bytes, _)| bytes)
    }

    /// Mark `id` for the closing witness wave.
    pub fn enrol_witness(&mut self, id: IntervalId) {
        let record = &mut self.records[id.0 as usize];
        if !record.witnessed {
            record.witnessed = true;
            self.witnessed.push(id);
        }
    }

    /// Advance `cursor` past every indexed interval that ends before `addr`.
    ///
    /// Galloping rather than stepping: a batch whose next query is far past
    /// the last one must not pay for the owned intervals in between, or a
    /// sweep over a sparse batch would cost the size of the whole index.
    fn advance(&self, cursor: &mut Cursor, addr: u64) {
        for run_index in 0..RUN_COUNT {
            let run = &self.runs[run_index];
            let mut at = cursor.at[run_index] as usize;
            if at < run.len() && run[at].last < addr {
                let mut step = 1usize;
                while at + step < run.len() && run[at + step].last < addr {
                    at += step;
                    step *= 2;
                }
                // `run[at]` is still short of `addr` and `run[at + step]` is
                // not (or does not exist), so the answer is in `(at, at+step]`.
                let hi = at.saturating_add(step).saturating_add(1).min(run.len());
                at += run[at..hi].partition_point(|entry| entry.last < addr);
            }
            cursor.at[run_index] = at as u32;
        }
    }

    /// Report every indexed interval starting at or before `last`, in address
    /// order, from an already-advanced `cursor`.
    ///
    /// The cursor guarantees the first entry of every run ends at or after the
    /// query's start; intervals are disjoint, so every later entry ends later
    /// still. Combined with the `start <= last` test below, everything emitted
    /// overlaps the query and nothing that overlaps it is missed.
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

    /// Merge run `src` into run `dst` and empty `src`. Both are address-sorted
    /// and their spans are disjoint, so this is a plain two-way merge.
    fn fold(&mut self, dst: usize, src: usize) {
        if self.runs[src].is_empty() {
            return;
        }
        if self.runs[dst].is_empty() {
            self.runs.swap(dst, src);
            return;
        }
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        out.reserve(self.runs[dst].len() + self.runs[src].len());
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
    }
}

/// A spilled edge's key. `consumer` widens to `u32` so the tuple is three
/// words; a `ConsumerId` is a `u16`, so no real edge can collide with another.
fn edge_key(parent: IntervalId, child: IntervalId, consumer: ConsumerId) -> (u32, u32, u32) {
    (parent.0, child.0, u32::from(consumer.0))
}

/// The part of `requested` that lies inside `interval`.
fn clip(requested: Span, interval: Span) -> Span {
    let start = u128::from(requested.start).max(u128::from(interval.start));
    let end = requested.end_excl().min(interval.end_excl());
    if end <= start {
        return Span::new(0, 0);
    }
    hole(start, end)
}

fn hole(start: u128, end: u128) -> Span {
    // Both ends come from `Span` arithmetic on real intervals, so the
    // conversion cannot lose: an observation cannot own a span wider than the
    // address space.
    Span::new(
        u64::try_from(start).unwrap_or(u64::MAX),
        u64::try_from(end.saturating_sub(start)).unwrap_or(u64::MAX),
    )
}
