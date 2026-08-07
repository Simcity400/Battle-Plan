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

use std::collections::BTreeMap;
use std::ops::Bound;

use super::{ConsumerId, IntervalId, Span};

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntervalRecord {
    pub id: IntervalId,
    /// The ORIGINAL address range. Kept for translation and for the closing
    /// witness, which re-copies these exact addresses.
    pub span: Span,
    /// Offset of the opening copy in the arena.
    pub opening_offset: usize,
    /// Offset of the closing copy, once the closing wave has run.
    pub closing_offset: Option<usize>,
    /// Consumers whose hour depends on these bytes, with how far into the
    /// interval each one actually reached.
    pub dependents: Vec<Dependent>,
    /// Intervals discovered THROUGH these bytes, each labelled with the
    /// consumer whose recipe walked that edge.
    ///
    /// The label is what keeps invalidation per consumer. Deduplication means
    /// one interval can be BOTH a discovered child of A's chain and a root of
    /// B's: an unlabelled edge would gap B when A's parent moved, even though
    /// B never went through that parent and B's own bytes were witnessed
    /// unchanged.
    pub children: Vec<(IntervalId, ConsumerId)>,
    /// Whether the closing wave must re-copy this interval.
    pub witnessed: bool,
}

impl IntervalRecord {
    pub fn depends(&self, consumer: ConsumerId) -> bool {
        self.dependents
            .iter()
            .any(|dependent| dependent.consumer == consumer)
    }

    /// Whether `consumer` may see `want`. Depending on the interval is not
    /// enough — the bytes must be inside what it reached for.
    pub fn reaches(&self, consumer: ConsumerId, want: &Span) -> bool {
        self.dependents
            .iter()
            .any(|dependent| dependent.consumer == consumer && dependent.reach.contains_span(want))
    }

    pub fn consumers(&self) -> impl Iterator<Item = ConsumerId> + '_ {
        self.dependents.iter().map(|dependent| dependent.consumer)
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

/// The observation-wide interval map.
#[derive(Debug, Default)]
pub struct OwnershipMap {
    records: Vec<IntervalRecord>,
    /// start address -> index into `records`. Keeps insertion `O(log n)` and
    /// overlap queries `O(log n + hits)`; a sorted `Vec` with mid-inserts is
    /// `O(n)` per insert and quadratic over a 100k-range observation.
    by_start: BTreeMap<u64, u32>,
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

    /// Ids in ascending address order.
    pub fn sorted_ids(&self) -> Vec<IntervalId> {
        self.by_start.values().map(|&i| IntervalId(i)).collect()
    }

    pub fn into_records(self) -> Vec<IntervalRecord> {
        self.records
    }

    /// Every owned interval overlapping `span`, in address order.
    pub fn overlapping(&self, span: Span) -> Vec<IntervalId> {
        let mut out = Vec::new();
        self.overlapping_into(span, &mut out);
        out
    }

    /// [`Self::overlapping`] into a buffer the caller owns and reuses. The
    /// executor calls this once per request per wave, so a fresh `Vec` here is
    /// a heap allocation per request.
    pub fn overlapping_into(&self, span: Span, out: &mut Vec<IntervalId>) {
        out.clear();
        if span.is_empty() {
            return;
        }
        // The interval that starts at or before `span.start` may still reach
        // into it, so the scan begins there when it exists.
        if let Some((_, &idx)) = self.by_start.range(..=span.start).next_back() {
            if self.records[idx as usize].span.overlaps(&span) {
                out.push(IntervalId(idx));
            }
        }
        // EXCLUDED, not `span.start..`: an interval starting exactly at
        // `span.start` is the one the lookup above already returned, and an
        // inclusive bound here reports it twice. That duplicate is invisible in
        // the dependency-attach path (both `add_dependent` and `add_edge`
        // deduplicate) and NOT invisible in `plan_into`, whose `existing` list
        // is what a caller counts.
        for (&start, &idx) in self
            .by_start
            .range((Bound::Excluded(span.start), Bound::Unbounded))
        {
            if u128::from(start) >= span.end_excl() {
                break;
            }
            out.push(IntervalId(idx));
        }
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
        plan.existing.clear();
        plan.holes.clear();
        if span.is_empty() {
            return;
        }
        let mut cursor = u128::from(span.start);
        let end = span.end_excl();
        self.overlapping_into(span, &mut plan.existing);
        for position in 0..plan.existing.len() {
            let owned = self.records[plan.existing[position].0 as usize].span;
            let owned_start = u128::from(owned.start);
            if owned_start > cursor {
                plan.holes.push(hole(cursor, owned_start.min(end)));
            }
            cursor = cursor.max(owned.end_excl());
            if cursor >= end {
                break;
            }
        }
        if cursor < end {
            plan.holes.push(hole(cursor, end));
        }
        plan.holes.retain(|h| !h.is_empty());
    }

    /// Take ownership of a previously unowned `span` copied at
    /// `opening_offset`.
    ///
    /// The caller is responsible for having asked [`Self::plan`] first: this
    /// does not check for overlap, because the only caller that can produce an
    /// arena offset is the one that just filled the hole.
    pub fn insert(&mut self, span: Span, opening_offset: usize) -> IntervalId {
        let id = IntervalId(self.records.len() as u32);
        self.records.push(IntervalRecord {
            id,
            span,
            opening_offset,
            closing_offset: None,
            dependents: Vec::new(),
            children: Vec::new(),
            witnessed: false,
        });
        self.by_start.insert(span.start, id.0);
        id
    }

    /// Record that `consumer`'s hour depends on the part of `id` that
    /// `requested` covers.
    ///
    /// `requested` is CLIPPED to the interval: a request that spans several
    /// intervals grants reach in each one only over its own share.
    pub fn add_dependent(&mut self, id: IntervalId, consumer: ConsumerId, requested: Span) {
        let record = &mut self.records[id.0 as usize];
        let clipped = clip(requested, record.span);
        if clipped.is_empty() {
            return;
        }
        match record
            .dependents
            .iter_mut()
            .find(|dependent| dependent.consumer == consumer)
        {
            Some(dependent) => dependent.reach = dependent.reach.hull(&clipped),
            None => record.dependents.push(Dependent {
                consumer,
                reach: clipped,
            }),
        }
    }

    /// Record that `consumer`'s recipe decoded `child`'s address out of
    /// `parent`'s bytes.
    pub fn add_edge(&mut self, parent: IntervalId, child: IntervalId, consumer: ConsumerId) {
        if parent == child {
            return;
        }
        let record = &mut self.records[parent.0 as usize];
        if !record.children.contains(&(child, consumer)) {
            record.children.push((child, consumer));
        }
    }

    /// The interval containing `addr`, if any.
    pub fn find(&self, addr: u64) -> Option<IntervalId> {
        let (_, &idx) = self.by_start.range(..=addr).next_back()?;
        if self.records[idx as usize].span.contains_addr(addr) {
            Some(IntervalId(idx))
        } else {
            None
        }
    }

    /// Read `len` bytes at `addr` out of the arena, if one interval holds all
    /// of them.
    pub fn read<'a>(
        &self,
        arena: &'a super::arena::Arena,
        addr: u64,
        len: usize,
    ) -> Option<&'a [u8]> {
        let id = self.find(addr)?;
        let record = &self.records[id.0 as usize];
        let want = Span::new(addr, len as u64);
        if !record.span.contains_span(&want) {
            return None;
        }
        let skew = (addr - record.span.start) as usize;
        Some(arena.slice(record.opening_offset + skew, len))
    }

    /// Mark `id` for the closing witness wave.
    pub fn enrol_witness(&mut self, id: IntervalId) {
        self.records[id.0 as usize].witnessed = true;
    }
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
