use super::*;
mod copied;
mod missions;
mod retained;
pub(super) use retained::{Retained, prune, release, retained};
pub(super) mod speculative;
mod witness;
#[cfg(test)]
const CACHE_FIELDS: usize = 20;
/// Why a design snapshot hit or decoded; tests count them as cache evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
// Test-only miss diagnostics construct the remaining outcomes.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) enum CacheOutcome {
    FullCopyHit,
    ReadWitnessHit,
    FullDecode,
    NoRecentEntry,
    SpanCount,
    SpanAddress,
    SpanMetadata,
    ReadBytes,
    Overlap,
    ContextNotReusable,
    AllocationSkipped,
    WitnessUnavailable,
    UnreadableCopy,
    WitnessReadFailed,
    InsertOverlap,
    DesignEvents,
    RetainedHits,
    RetainedCheckSpans,
    RetainedCandidateReads,
    RetainedRepeats,
}
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

pub(super) struct Decoded {
    pub value: rows::VariantSnapshot,
    pub definition: native::DefinitionRow<Arc<rows::VariantDesignState>>,
    pub context: Option<super::super::design_context::Event>,
}
impl Decoded {
    fn allocated_bytes(&self) -> usize {
        size_of::<Decoded>()
            + 2 * size_of::<usize>()
            + replay_data::value::Value::heap_bytes(&self.value)
            + rows::NativeValue::heap_bytes(self.definition.design.as_ref())
            + self
                .definition
                .source_links
                .as_ref()
                .map_or(0, |links| links.direct.capacity() * size_of::<usize>())
            + self
                .context
                .as_ref()
                .map_or(0, super::super::design_context::Event::allocated_bytes)
    }
}
pub(super) struct Occurrence {
    index: usize,
    pub sequence: u64,
    pub hour: i32,
    pub variant: u64,
    pub value: Arc<Decoded>,
}
#[derive(Clone, Copy)]
pub(super) enum Change<'a> {
    Snapshot(&'a Occurrence),
    Row(&'a rows::VariantChangeRow),
}
pub(super) struct View<'a> {
    pub variant: u64,
    pub flags: Option<[u8; 3]>,
    pub scalar: Option<&'a rows::VariantScalarChange>,
    pub sources: Option<&'a rows::VariantSourceReferences>,
    pub organisation: Option<&'a rows::VariantOrganisationChange>,
    pub removed: bool,
    pub snapshot: Option<&'a rows::VariantSnapshot>,
}
impl<'a> Change<'a> {
    pub fn view(self) -> View<'a> {
        match self {
            Self::Snapshot(snapshot) => View {
                variant: snapshot.variant,
                flags: None,
                scalar: None,
                sources: None,
                organisation: None,
                removed: false,
                snapshot: Some(&snapshot.value.value),
            },
            Self::Row(row) => View {
                variant: row.variant,
                flags: row.flags,
                scalar: row.scalar.as_ref(),
                sources: row.sources.as_ref(),
                organisation: row.organisation.as_ref(),
                removed: row.removed,
                snapshot: row.snapshot.as_ref(),
            },
        }
    }
    pub fn try_clone(self) -> std::io::Result<rows::VariantChangeRow> {
        let row = match self {
            Self::Snapshot(snapshot) => rows::VariantChangeRow {
                sequence: snapshot.sequence,
                hour: snapshot.hour,
                variant: snapshot.variant,
                snapshot: Some(replay_data::value::Value::try_clone(&snapshot.value.value)?),
                ..Default::default()
            },
            Self::Row(row) => return rows::RowCodec::try_clone(row),
        };
        rows::RowCodec::check_key_fields(&row)?;
        Ok(row)
    }
}
#[derive(Clone, Copy)]
pub(in crate::production) struct History<'a> {
    rows: &'a [rows::VariantChangeRow],
    shared: &'a Shared,
}
impl<'a> History<'a> {
    pub(super) fn new(rows: &'a [rows::VariantChangeRow], shared: &'a Shared) -> Self {
        Self { rows, shared }
    }
    #[cfg(test)]
    pub(super) fn from_rows(rows: &'a [rows::VariantChangeRow]) -> Self {
        static EMPTY: Shared = Shared {
            entries: Vec::new(),
            heap_bytes: 0,
        };
        Self {
            rows,
            shared: &EMPTY,
        }
    }
    pub(super) fn len(self) -> usize {
        self.rows.len() + self.shared.len()
    }
    pub(super) fn is_empty(self) -> bool {
        self.len() == 0
    }
    pub(super) fn iter(self) -> impl Iterator<Item = Result<Change<'a>, &'static str>> {
        self.shared.iter(self.rows)
    }
}
#[derive(Default)]
pub(super) struct Shared {
    entries: Vec<Occurrence>,
    heap_bytes: usize,
}
impl Shared {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.heap_bytes = 0;
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Occurrence>() + self.heap_bytes
    }
    pub fn reserve(&mut self, additional: usize) -> Result<(), &'static str> {
        engine_allocation::reserve(&mut self.entries, additional)
            .map_err(|_| "variant shared snapshot allocation failed")
    }
    pub fn push(
        &mut self,
        index: usize,
        sequence: u64,
        hour: i32,
        variant: u64,
        value: Arc<Decoded>,
    ) -> Result<(), &'static str> {
        self.reserve(1)?;
        self.entries.push(Occurrence {
            index,
            sequence,
            hour,
            variant,
            value,
        });
        Ok(())
    }
    pub fn iter<'a>(
        &'a self,
        source: &'a [rows::VariantChangeRow],
    ) -> impl Iterator<Item = Result<Change<'a>, &'static str>> {
        let mut entries = self.entries.iter().peekable();
        let mut rows = source.iter();
        (0..source.len() + self.entries.len()).map(move |index| {
            if entries.peek().is_some_and(|entry| entry.index == index) {
                Ok(Change::Snapshot(entries.next().unwrap()))
            } else {
                rows.next()
                    .map(Change::Row)
                    .ok_or("variant shared snapshot index invalid")
            }
        })
    }
    #[cfg(test)]
    pub fn materialize(
        &self,
        output: &mut Vec<rows::VariantChangeRow>,
    ) -> Result<(), &'static str> {
        if !self.entries.is_empty() {
            let mut rows = engine_allocation::vec(output.len() + self.len())
                .map_err(|_| "variant shared history allocation failed")?;
            for change in self.iter(output) {
                rows.push(
                    change?
                        .try_clone()
                        .map_err(|_| "variant shared history row invalid")?,
                );
            }
            *output = rows;
        }
        Ok(())
    }
}
struct Entry {
    range: std::ops::Range<usize>,
    pointer: usize,
    next: Option<usize>,
    value: Arc<Decoded>,
    witness: Option<witness::Witness>,
    witness_next: Option<usize>,
    shape_next: Option<usize>,
    missions: Option<missions::Proof>,
    mission_next: Option<usize>,
    mission_model_next: Option<usize>,
}
struct Verified {
    range: std::ops::Range<usize>,
    value: Arc<Decoded>,
    index: Option<usize>,
}
struct SourceHeads {
    pointer: usize,
    states: HashMap<u32, Verified>,
}
#[derive(Default)]
pub(super) struct Cache<'a> {
    retained: Option<&'a mut Retained>,
    retained_charge: u64,
    source_heads: HashMap<u64, SourceHeads>,
    #[cfg(test)]
    removal_visits: usize,
    heads: HashMap<u64, usize>,
    witness_heads: HashMap<u64, usize>,
    shape_heads: HashMap<u64, usize>,
    mission_heads: HashMap<u64, usize>,
    mission_models: HashMap<u64, usize>,
    #[cfg(test)]
    probes: [usize; 3],
    #[cfg(test)]
    mission_model_visits: [usize; 2],
    comparison: Vec<u8>,
    entries: Vec<Entry>,
    recent: HashMap<usize, (usize, Option<usize>)>,
    order: Vec<usize>,
    #[cfg(test)]
    counts: [u64; CACHE_FIELDS],
    #[cfg(test)]
    miss: Option<CacheOutcome>,
    /// Speculative walks and planned retained tests by event index, and the event being decoded.
    speculative: Vec<Option<Box<speculative::Prepared>>>,
    checks: Vec<Check>,
    scratch: Scratch,
    current: Option<usize>,
}
fn payload<'a>(log: &'a RawLog<1>, span: &collector_raw::raw::Span) -> &'a [u8] {
    if span.ok {
        &log.bytes[span.start as usize..][..span.len as usize]
    } else {
        &[]
    }
}
fn fingerprint(log: &RawLog<1>, range: std::ops::Range<usize>, pointer: usize) -> u64 {
    let mut hash = DefaultHasher::new();
    pointer.hash(&mut hash);
    range.len().hash(&mut hash);
    for span in &log.spans[range] {
        span.addr.hash(&mut hash);
        span.len.hash(&mut hash);
        span.ok.hash(&mut hash);
        span.bulk.hash(&mut hash);
        hash.write(payload(log, span));
    }
    hash.finish()
}
fn shape_key(log: &RawLog<1>, range: std::ops::Range<usize>, pointer: usize) -> u64 {
    let mut hash = DefaultHasher::new();
    pointer.hash(&mut hash);
    range.len().hash(&mut hash);
    for span in &log.spans[range] {
        (span.addr, span.len, span.ok, span.bulk).hash(&mut hash);
    }
    hash.finish()
}
fn equal(log: &RawLog<1>, left: std::ops::Range<usize>, right: std::ops::Range<usize>) -> bool {
    let (left, right) = (&log.spans[left], &log.spans[right]);
    if left.len() != right.len() {
        return false;
    }
    let Some((first_left, first_right)) = left.first().zip(right.first()) else {
        return true;
    };
    let (left_start, right_start) = (first_left.start as usize, first_right.start as usize);
    let (mut left_end, mut right_end) = (left_start, right_start);
    let mut contiguous = true;
    for (left, right) in left.iter().zip(right) {
        if left.addr != right.addr
            || left.len != right.len
            || left.ok != right.ok
            || left.bulk != right.bulk
        {
            return false;
        }
        contiguous &=
            left.ok && left.start as usize == left_end && right.start as usize == right_end;
        left_end = left.start as usize + left.len as usize;
        right_end = right.start as usize + right.len as usize;
    }
    if contiguous {
        // Staged contiguous payloads need one byte comparison, not one per small span.
        log.bytes[left_start..left_end] == log.bytes[right_start..right_end]
    } else {
        left.iter()
            .zip(right)
            .all(|(left, right)| payload(log, left) == payload(log, right))
    }
}
enum Overlaps {
    Consistent,
    Allocation,
    Unreadable,
    Inconsistent,
}
/// Orders `range`'s spans by address and checks that overlapping copies agree.
fn order_overlaps(
    log: &RawLog<1>,
    range: std::ops::Range<usize>,
    order: &mut Vec<usize>,
) -> Result<Overlaps, &'static str> {
    order.clear();
    // The cache is optional: an allocation failure only skips caching, never the hour.
    if engine_allocation::advisory(|| engine_allocation::reserve(order, range.len())).is_err() {
        return Ok(Overlaps::Allocation);
    }
    order.extend(range);
    order.sort_unstable_by_key(|index| log.spans[*index].addr);
    let mut covering: Option<&collector_raw::raw::Span> = None;
    // Reused decodes need single-valued bytes even when read cursors start afresh.
    for index in order.iter() {
        let span = &log.spans[*index];
        if !span.ok {
            return Ok(Overlaps::Unreadable);
        }
        let end = span
            .addr
            .checked_add(span.len as usize)
            .ok_or("variant snapshot witness address overflow")?;
        if let Some(previous) = covering {
            let previous_end = previous.addr + previous.len as usize;
            if previous_end > span.addr {
                let offset = span.addr - previous.addr;
                let len = (previous_end - span.addr).min(span.len as usize);
                if payload(log, previous)[offset..offset + len] != payload(log, span)[..len] {
                    return Ok(Overlaps::Inconsistent);
                }
            }
            if previous_end >= end {
                continue;
            }
        }
        covering = Some(span);
    }
    Ok(Overlaps::Consistent)
}
/// A retained state an occurrence reproduces: its native source count and value address.
pub(super) struct Verdict {
    count: u32,
    value: usize,
    repeat: bool,
}
/// A retained-cache test taken ahead of the ordered pass, which consumes it in its place.
#[derive(Default)]
pub(super) enum Check {
    #[default]
    Unchecked,
    Miss,
    Hit(Verdict),
}
#[derive(Default)]
pub(super) struct Scratch {
    order: Vec<usize>,
    comparison: Vec<u8>,
    spans: u64,
    candidate_reads: u64,
    allocation_skips: u64,
}
impl Scratch {
    pub(super) fn absorb(&mut self, other: &mut Self) {
        self.spans += std::mem::take(&mut other.spans);
        self.candidate_reads += std::mem::take(&mut other.candidate_reads);
        self.allocation_skips += std::mem::take(&mut other.allocation_skips);
    }
}
/// The ordered pass's retained-cache test for an occurrence of a retained design. Read-only,
/// so lanes may run it; until the pass finishes, retained states are only removed.
pub(super) fn check_retained(
    retained: &Retained,
    log: &RawLog<1>,
    event: &Event,
    scratch: &mut Scratch,
) -> Check {
    let range = event.start..event.end;
    scratch.spans += range.len() as u64;
    if let Some((count, _)) = copied::sources(log, range.clone(), event.pointer)
        && let Some(entry) = retained.find_repeat(log, range.clone(), event, count)
    {
        return Check::Hit(Verdict {
            count,
            value: Arc::as_ptr(&entry.value) as usize,
            repeat: true,
        });
    }
    engine_allocation::advisory(|| {
        match order_overlaps(log, range.clone(), &mut scratch.order).ok()? {
            Overlaps::Consistent => {}
            Overlaps::Allocation => {
                scratch.allocation_skips += 1;
                return None;
            }
            Overlaps::Unreadable | Overlaps::Inconsistent => return None,
        }
        let memory = copied::MemoryView::new(log, range.clone(), &scratch.order);
        let (count, entry) =
            retained.find(&memory, event, &mut scratch.comparison, &mut scratch.candidate_reads)?;
        Some(Verdict {
            count,
            value: Arc::as_ptr(&entry.value) as usize,
            repeat: false,
        })
    })
    .map_or(Check::Miss, Check::Hit)
}
impl<'a> Cache<'a> {
    pub fn with_retained(retained: &'a mut Retained) -> Self {
        let mut cache = Self::default();
        cache.retained = Some(retained);
        static NEXT_CHARGE: AtomicU64 = AtomicU64::new(1);
        cache.retained_charge = NEXT_CHARGE.fetch_add(1, Ordering::Relaxed);
        cache
    }
    /// Runs the burst's likely-miss walks on the lanes before the ordered pass.
    pub fn speculate(
        &mut self,
        batch: &Batch,
        base: usize,
        hour: i32,
        speculation: &mut super::Speculation<'_>,
    ) {
        let mut planned = speculative::speculate(
            batch,
            base,
            hour,
            speculation,
            self.retained.as_deref(),
        );
        self.speculative = planned.walks;
        self.checks = planned.checks;
        self.absorb(&mut planned.scratch);
    }
    fn absorb(&mut self, scratch: &mut Scratch) {
        let mut totals = Scratch::default();
        totals.absorb(scratch);
        self.count_by(CacheOutcome::RetainedCheckSpans, totals.spans);
        self.count_by(CacheOutcome::RetainedCandidateReads, totals.candidate_reads);
        self.count_by(CacheOutcome::AllocationSkipped, totals.allocation_skips);
    }
    #[inline(always)]
    fn count(&mut self, outcome: CacheOutcome) {
        self.count_by(outcome, 1);
    }
    #[inline(always)]
    fn count_by(&mut self, _outcome: CacheOutcome, _count: u64) {
        #[cfg(test)]
        {
            self.counts[_outcome as usize] += _count;
        }
    }
    /// The next [`Self::decode`] is the batch's event `index`.
    pub fn at(&mut self, index: usize) {
        self.current = Some(index);
    }
    /// The walk speculated for the current event, when its inputs are byte-identical.
    fn prepared(
        &mut self,
        log: &RawLog<1>,
        range: std::ops::Range<usize>,
    ) -> Option<Box<speculative::Prepared>> {
        let prepared = self.speculative.get_mut(self.current.take()?)?.take()?;
        prepared.matches(log, range).then_some(prepared)
    }
    pub fn remove(&mut self, variant: u64) {
        if let Some(heads) = self.source_heads.remove(&variant) {
            for _ in heads.states {
                #[cfg(test)]
                {
                    self.removal_visits += 1;
                }
            }
        }
        if let Some(retained) = &mut self.retained {
            retained.remove(variant);
        }
    }
    pub fn finish(&mut self, batch: &Batch) {
        if let Some(retained) = &mut self.retained {
            retained.remember(
                &batch.log,
                std::mem::take(&mut self.entries),
                &self.source_heads,
            );
        }
    }
    fn consistent(
        &mut self,
        log: &RawLog<1>,
        range: std::ops::Range<usize>,
    ) -> Result<bool, &'static str> {
        self.consistent_for(log, range, true)
    }
    fn consistent_for(
        &mut self,
        log: &RawLog<1>,
        range: std::ops::Range<usize>,
        report: bool,
    ) -> Result<bool, &'static str> {
        let outcome = match order_overlaps(log, range, &mut self.order)? {
            Overlaps::Consistent => return Ok(true),
            Overlaps::Allocation => CacheOutcome::AllocationSkipped,
            Overlaps::Unreadable if report => CacheOutcome::UnreadableCopy,
            Overlaps::Inconsistent if report => CacheOutcome::InsertOverlap,
            Overlaps::Unreadable | Overlaps::Inconsistent => return Ok(false),
        };
        self.count(outcome);
        Ok(false)
    }
    fn reuse(&self, index: usize) -> Result<Arc<Decoded>, &'static str> {
        Self::observe(&self.entries[index].value)
    }
    fn observe(value: &Arc<Decoded>) -> Result<Arc<Decoded>, &'static str> {
        sources::track(&value.definition)?;
        liveness::sample(&value.definition)?;
        value
            .context
            .as_ref()
            .ok_or("variant cached context observation missing")?
            .apply();
        Ok(Arc::clone(value))
    }
    fn source_heads(
        heads: &mut HashMap<u64, SourceHeads>,
        id: u64,
        pointer: usize,
    ) -> Option<&mut SourceHeads> {
        if !heads.contains_key(&id) {
            engine_allocation::reserve_map(heads, 1).ok()?;
        }
        let heads = heads.entry(id).or_insert_with(|| SourceHeads {
            pointer,
            states: HashMap::new(),
        });
        if heads.pointer != pointer {
            heads.pointer = pointer;
            heads.states.clear();
        }
        Some(heads)
    }
    fn verified(
        heads: &mut HashMap<u64, SourceHeads>,
        pointer: usize,
        range: std::ops::Range<usize>,
        value: &Arc<Decoded>,
        index: Option<usize>,
    ) -> bool {
        engine_allocation::advisory(|| {
            let heads = Self::source_heads(heads, value.definition.identity.id, pointer)?;
            let count = value.definition.design.bonus_sources.len() as u32;
            if let Some(previous) = heads.states.get_mut(&count) {
                previous.range = range;
                previous.index = index;
                if !Arc::ptr_eq(&previous.value, value) {
                    previous.value = Arc::clone(value);
                }
            } else {
                engine_allocation::reserve_map(&mut heads.states, 1).ok()?;
                heads.states.insert(
                    count,
                    Verified {
                        range,
                        value: Arc::clone(value),
                        index,
                    },
                );
            }
            Some(())
        })
        .is_some()
    }
    fn remember(&mut self, pointer: usize, index: usize, range: std::ops::Range<usize>) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        if !Self::verified(
            &mut self.source_heads,
            pointer,
            range.clone(),
            &entry.value,
            Some(index),
        ) {
            self.count(CacheOutcome::AllocationSkipped);
        }

        if let Some((latest, previous)) = self.recent.get_mut(&pointer) {
            if *latest != index {
                *previous = Some(*latest);
                *latest = index;
            }
            return;
        }
        if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.recent, 1))
            .is_ok()
        {
            self.recent.insert(pointer, (index, None));
        } else {
            self.count(CacheOutcome::AllocationSkipped);
        }
    }
    pub fn decode(
        &mut self,
        batch: &Batch,
        event: &Event,
        base: usize,
        hour: i32,
        shared: &mut Shared,
    ) -> Result<Arc<Decoded>, &'static str> {
        self.count(CacheOutcome::DesignEvents);
        let range = event.start..event.end;
        #[cfg(test)]
        {
            self.miss = None;
        }
        let source = copied::sources(&batch.log, range.clone(), event.pointer);
        // Capacity evidence must survive every optional cache allocation and early return.
        if let Some(retained) = &mut self.retained {
            retained.observe(
                event.variant,
                event.pointer,
                source.map(|(_, capacity)| capacity),
            );
        }
        if let Some((count, _)) = source
            && let Some(heads) = self
                .source_heads
                .get_mut(&event.variant)
                .filter(|heads| heads.pointer == event.pointer)
        {
            if let Some(head) = heads.states.get_mut(&count)
                && equal(&batch.log, head.range.clone(), range.clone())
            {
                // Equal complete copies inherit the verified occurrence's overlap consistency.
                let value = Self::observe(&head.value)?;
                head.range = range;
                self.count(CacheOutcome::FullCopyHit);
                return Ok(value);
            }
        }
        if let Some(index) = source.and_then(|(count, _)| {
            self.source_heads
                .get(&event.variant)
                .filter(|heads| heads.pointer == event.pointer)?
                .states
                .get(&count)?
                .index
        }) && self.entries[index].pointer == event.pointer
        {
            let entry = &self.entries[index];
            if entry.witness.as_ref().is_some_and(|witness| {
                witness.equal(&batch.log, entry.range.clone(), range.clone())
            }) {
                let value = self.reuse(index)?;
                self.count(CacheOutcome::ReadWitnessHit);
                self.remember(event.pointer, index, range.clone());
                return Ok(value);
            }
            if entry.missions.is_some() {
                let matched = engine_allocation::advisory(|| {
                    if !self.consistent_for(&batch.log, range.clone(), false).ok()? {
                        return None;
                    }
                    let memory = copied::MemoryView::new(&batch.log, range.clone(), &self.order);
                    let entry = &self.entries[index];
                    entry.missions.as_ref()?.equal(
                        &memory,
                        entry.witness.as_ref()?,
                        event.pointer,
                        &entry.value.definition.design.mission_stats,
                        &mut self.comparison,
                    )
                });
                if matched == Some(true) {
                    let value = self.reuse(index)?;
                    self.count(CacheOutcome::ReadWitnessHit);
                    self.remember(event.pointer, index, range.clone());
                    return Ok(value);
                }
            }
        }
        if self
            .retained
            .as_ref()
            .is_some_and(|retained| retained.contains(event))
        {
            // A planned test stands: until the pass finishes, states are only removed.
            let check = match self
                .current
                .and_then(|index| self.checks.get_mut(index))
                .map(std::mem::take)
            {
                Some(check @ (Check::Hit(_) | Check::Miss)) => check,
                _ => {
                    let mut scratch = std::mem::take(&mut self.scratch);
                    let check = self.retained.as_deref().map_or(Check::Miss, |retained| {
                        check_retained(retained, &batch.log, event, &mut scratch)
                    });
                    self.absorb(&mut scratch);
                    self.scratch = scratch;
                    check
                }
            };
            let matched = match check {
                Check::Hit(verdict) => self.retained.as_deref().and_then(|retained| {
                    let entry = retained.verified(event, verdict.count, verdict.value)?;
                    Some((
                        Arc::clone(&entry.value),
                        entry.charge(self.retained_charge),
                        verdict.repeat,
                    ))
                }),
                Check::Miss | Check::Unchecked => None,
            };
            if let Some((value, charge, repeat)) = matched {
                if repeat {
                    self.count(CacheOutcome::RetainedRepeats);
                }
                self.count(CacheOutcome::RetainedHits);
                let value = Self::observe(&value)?;
                // A retained hit supersedes any earlier local revision at this source count.
                if !Self::verified(
                    &mut self.source_heads,
                    event.pointer,
                    range.clone(),
                    &value,
                    None,
                ) {
                    self.source_heads.remove(&event.variant);
                    self.count(CacheOutcome::AllocationSkipped);
                }
                shared.heap_bytes += charge;
                self.count(CacheOutcome::ReadWitnessHit);
                return Ok(value);
            }
        }
        if let Some((latest, previous)) = self.recent.get(&event.pointer).copied() {
            for index in std::iter::once(latest).chain(previous) {
                if equal(&batch.log, self.entries[index].range.clone(), range.clone()) {
                    let value = self.reuse(index)?;
                    self.count(CacheOutcome::FullCopyHit);
                    self.remember(event.pointer, index, range.clone());
                    return Ok(value);
                }
            }
        }
        if let Some((latest, previous)) = self.recent.get(&event.pointer).copied() {
            for index in std::iter::once(latest).chain(previous) {
                let entry = &self.entries[index];
                let mismatch = entry.witness.as_ref().map(|witness| {
                    witness.mismatch(&batch.log, entry.range.clone(), range.clone())
                });
                if mismatch == Some(None) {
                    let value = self.reuse(index)?;
                    self.count(CacheOutcome::ReadWitnessHit);
                    self.remember(event.pointer, index, range.clone());
                    return Ok(value);
                }
                #[cfg(test)]
                if index == latest {
                    self.miss = Some(mismatch.flatten().unwrap_or_else(|| {
                        witness::shape_miss(
                            &batch.log.spans[entry.range.clone()],
                            &batch.log.spans[range.clone()],
                        )
                        .unwrap_or(CacheOutcome::WitnessUnavailable)
                    }));
                }
            }
        }
        // A modifier refresh can revisit an older source-list shape, beyond the last two decodes.
        let shaped = self
            .shape_heads
            .get(&shape_key(&batch.log, range.clone(), event.pointer))
            .copied();
        let recent = self.recent.get(&event.pointer).copied();
        let mut recent = recent.into_iter().flat_map(|(latest, previous)| {
            std::iter::once(latest)
                .chain(previous)
                .filter(move |index| Some(*index) != shaped)
        });
        let mut model = shaped;
        loop {
            let (index, shaped) = if let Some(index) = model {
                model = self.entries[index].shape_next;
                (index, true)
            } else if let Some(index) = recent.next() {
                (index, false)
            } else {
                break;
            };
            let entry = &self.entries[index];
            let Some(witness) = entry.witness.as_ref() else {
                continue;
            };
            if shaped
                && entry.pointer == event.pointer
                && witness.equal(&batch.log, entry.range.clone(), range.clone())
            {
                let value = self.reuse(index)?;
                self.count(CacheOutcome::ReadWitnessHit);
                self.remember(event.pointer, index, range.clone());
                return Ok(value);
            }
            let Some(key) = witness.key(
                &batch.log,
                entry.range.clone(),
                range.clone(),
                event.pointer,
            ) else {
                continue;
            };
            let mut next = self.witness_heads.get(&key).copied();
            while let Some(index) = next {
                #[cfg(test)]
                {
                    self.probes[1] += 1;
                }
                let entry = &self.entries[index];
                if entry.pointer == event.pointer
                    && entry.witness.as_ref().is_some_and(|witness| {
                        witness.equal(&batch.log, entry.range.clone(), range.clone())
                    })
                {
                    let value = self.reuse(index)?;
                    self.count(CacheOutcome::ReadWitnessHit);
                    self.remember(event.pointer, index, range.clone());
                    return Ok(value);
                }
                next = self.entries[index].witness_next;
            }
        }
        if !self.mission_heads.is_empty() && self.recent.contains_key(&event.pointer) {
            let matched = engine_allocation::advisory(|| {
                if !self.consistent_for(&batch.log, range.clone(), false).ok()? {
                    return None;
                }
                let order = std::mem::take(&mut self.order);
                let memory = copied::MemoryView::new(&batch.log, range.clone(), &order);
                let candidate =
                    self.mission_candidate(&memory, event.pointer, &batch.log.spans[range.clone()]);
                self.order = order;
                candidate
            });
            if let Some(index) = matched {
                let value = self.reuse(index)?;
                self.count(CacheOutcome::ReadWitnessHit);
                self.remember(event.pointer, index, range.clone());
                return Ok(value);
            }
        }
        let key = fingerprint(&batch.log, range.clone(), event.pointer);
        self.decode_key(batch, event, base, hour, shared, key)
    }
    fn mission_candidate(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        spans: &[collector_raw::raw::Span],
    ) -> Option<usize> {
        let model_key = missions::model_key(memory, pointer, spans)?;
        let mut model = self.mission_models.get(&model_key).copied();
        while let Some(index) = model {
            #[cfg(test)]
            {
                self.mission_model_visits[0] += 1;
            }
            let entry = &self.entries[index];
            model = entry.mission_model_next;
            let Some(key) = entry.missions.as_ref()?.key(
                memory,
                entry.witness.as_ref()?,
                model_key,
                &mut self.comparison,
            ) else {
                continue;
            };
            let mut next = self.mission_heads.get(&key).copied();
            while let Some(index) = next {
                #[cfg(test)]
                {
                    self.probes[2] += 1;
                }
                let entry = &self.entries[index];
                if entry.pointer == pointer
                    && entry.missions.as_ref()?.equal(
                        memory,
                        entry.witness.as_ref()?,
                        pointer,
                        &entry.value.definition.design.mission_stats,
                        &mut self.comparison,
                    )?
                {
                    return Some(index);
                }
                next = entry.mission_next;
            }
        }
        None
    }
    fn decode_key(
        &mut self,
        batch: &Batch,
        event: &Event,
        base: usize,
        hour: i32,
        shared: &mut Shared,
        key: u64,
    ) -> Result<Arc<Decoded>, &'static str> {
        let range = event.start..event.end;
        let mut next = self.heads.get(&key).copied();
        while let Some(index) = next {
            #[cfg(test)]
            {
                self.probes[0] += 1;
            }
            let entry = &self.entries[index];
            // A hash only selects candidates; every copied witness must still match.
            if entry.pointer == event.pointer
                && equal(&batch.log, entry.range.clone(), range.clone())
            {
                let value = self.reuse(index)?;
                self.count(CacheOutcome::FullCopyHit);
                self.remember(event.pointer, index, range.clone());
                return Ok(value);
            }
            next = entry.next;
        }
        #[cfg(test)]
        let miss = self.miss.take().unwrap_or_else(|| {
            self.recent
                .get(&event.pointer)
                .map_or(CacheOutcome::NoRecentEntry, |&(latest, _)| {
                    let entry = &self.entries[latest];
                    entry.witness.as_ref().map_or_else(
                        || {
                            witness::shape_miss(
                                &batch.log.spans[entry.range.clone()],
                                &batch.log.spans[range.clone()],
                            )
                            .unwrap_or(CacheOutcome::WitnessUnavailable)
                        },
                        |witness| {
                            witness
                                .mismatch(&batch.log, entry.range.clone(), range.clone())
                                .unwrap_or(CacheOutcome::WitnessUnavailable)
                        },
                    )
                })
        });
        self.count(CacheOutcome::FullDecode);
        #[cfg(test)]
        self.count(miss);
        (|| {
            let recorded = crate::raw::Recorded::build_range(&batch.log, range.clone())?;
            let (memory, value) = match self.prepared(&batch.log, range.clone()) {
                // The lane walked identical spans; the ordered step runs every effect here.
                Some(prepared) => {
                    let (mut walk, state) = prepared.into_parts();
                    let memory = witness::Observed::resume(&recorded, state);
                    let value = complete_snapshot(&memory, base, false, &mut walk)?;
                    (memory, value)
                }
                None => {
                    let memory = witness::Observed::new(&recorded, range.len());
                    let value = decode_snapshot(&memory, base, hour, event.pointer, false)?;
                    (memory, value)
                }
            };
            let value = Arc::new(value);
            shared.heap_bytes += value.allocated_bytes();
            if !value
                .context
                .as_ref()
                .is_some_and(|context| context.reusable())
            {
                self.count(CacheOutcome::ContextNotReusable);
                return Ok(value);
            }
            if !self.consistent(&batch.log, range.clone())? {
                return Ok(value);
            }
            #[cfg(test)]
            let (complete, allocation_failed, read_failed) = memory.diagnostics();
            let witness = memory.finish(&batch.log, range.clone(), &self.order, true);
            #[cfg(test)]
            {
                if allocation_failed || (complete && witness.is_none()) {
                    self.count(CacheOutcome::AllocationSkipped);
                }
                if read_failed {
                    self.count(CacheOutcome::WitnessReadFailed);
                }
            }
            let proof = witness.as_ref().and_then(|witness| {
                engine_allocation::advisory(|| {
                    missions::Proof::build(
                        &recorded,
                        witness,
                        event.pointer,
                        &value.definition.design.mission_stats,
                    )
                })
            });
            if self.insert(&batch.log, range, event.pointer, key, &value, witness) {
                self.index_missions(
                    &recorded,
                    event.pointer,
                    proof,
                    &batch.log.spans[event.start..event.end],
                );
            }
            Ok(value)
        })()
    }
    fn index_missions(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        proof: Option<missions::Proof>,
        spans: &[collector_raw::raw::Span],
    ) {
        let Some(proof) = proof else {
            return;
        };
        let indexed = engine_allocation::advisory(|| {
            let witness = self.entries.last()?.witness.as_ref()?;
            let model_key = missions::model_key(memory, pointer, spans)?;
            let mission_key = proof.key(memory, witness, model_key, &mut self.comparison)?;
            engine_allocation::reserve_map(&mut self.mission_heads, 1).ok()?;
            engine_allocation::reserve_map(&mut self.mission_models, 1).ok()?;
            Some((model_key, mission_key))
        });
        if let Some((model_key, mission_key)) = indexed {
            let index = self.entries.len() - 1;
            let witness = self.entries[index].witness.as_ref().unwrap();
            let mut model = self.mission_models.get(&model_key).copied();
            let mut known = false;
            while let Some(candidate) = model {
                #[cfg(test)]
                {
                    self.mission_model_visits[1] += 1;
                }
                let entry = &self.entries[candidate];
                if proof.same_reads(
                    witness,
                    entry.missions.as_ref().unwrap(),
                    entry.witness.as_ref().unwrap(),
                ) {
                    known = true;
                    break;
                }
                model = entry.mission_model_next;
            }
            if !known {
                self.entries[index].mission_model_next =
                    self.mission_models.insert(model_key, index);
            }
            self.entries[index].missions = Some(proof);
            self.entries[index].mission_next = self.mission_heads.insert(mission_key, index);
        } else {
            self.count(CacheOutcome::AllocationSkipped);
        }
    }
    fn insert(
        &mut self,
        log: &RawLog<1>,
        range: std::ops::Range<usize>,
        pointer: usize,
        key: u64,
        value: &Arc<Decoded>,
        mut witness: Option<witness::Witness>,
    ) -> bool {
        let reserved = engine_allocation::advisory(|| {
            engine_allocation::reserve_map(&mut self.heads, 1)?;
            engine_allocation::reserve(&mut self.entries, 1)
        });
        if reserved.is_err() {
            self.count(CacheOutcome::AllocationSkipped);
            return false;
        }
        if witness.is_some()
            && engine_allocation::advisory(|| {
                engine_allocation::reserve_map(&mut self.witness_heads, 1)?;
                engine_allocation::reserve_map(&mut self.shape_heads, 1)
            })
            .is_err()
        {
            self.count(CacheOutcome::AllocationSkipped);
            witness = None;
        }
        let witness_key = witness
            .as_ref()
            .and_then(|witness| witness.key(log, range.clone(), range.clone(), pointer));
        let index = self.entries.len();
        let mut shape_next = None;
        if let Some(witness) = witness.as_ref().filter(|_| witness_key.is_some()) {
            let key = shape_key(log, range.clone(), pointer);
            let mut model = self.shape_heads.get(&key).copied();
            let mut known = false;
            while let Some(candidate) = model {
                let entry = &self.entries[candidate];
                if entry.pointer == pointer
                    && witness::shape_miss(
                        &log.spans[entry.range.clone()],
                        &log.spans[range.clone()],
                    )
                    .is_none()
                    && entry
                        .witness
                        .as_ref()
                        .is_some_and(|other| witness.same_reads(other))
                {
                    known = true;
                    break;
                }
                model = entry.shape_next;
            }
            // Cold context lookups can add a read plan, but design revisions cannot grow this chain.
            if !known {
                shape_next = self.shape_heads.insert(key, index);
            }
        }
        self.entries.push(Entry {
            range: range.clone(),
            pointer,
            next: self.heads.insert(key, index),
            value: Arc::clone(value),
            witness,
            witness_next: witness_key.and_then(|key| self.witness_heads.insert(key, index)),
            shape_next,
            missions: None,
            mission_next: None,
            mission_model_next: None,
        });
        self.remember(pointer, index, range);
        true
    }
}

#[cfg(test)]
mod tests;
