use super::*;
mod copied;
mod missions;
mod retained;
pub(super) use retained::{Retained, prune, release, retained};
mod witness;
use profile::Outcome as CacheOutcome;
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
    missions: Option<missions::Proof>,
    mission_next: Option<usize>,
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
    comparison: Vec<u8>,
    entries: Vec<Entry>,
    recent: HashMap<usize, (usize, Option<usize>)>,
    order: Vec<usize>,
    counts: [u64; profile::CACHE_FIELDS],
    full_decode: profile::Cost,
    miss: Option<CacheOutcome>,
}
impl Drop for Cache<'_> {
    fn drop(&mut self) {
        profile::add_cache(self.counts);
        profile::add_cost(profile::Phase::FullDecode, self.full_decode);
    }
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
impl<'a> Cache<'a> {
    pub fn with_retained(retained: &'a mut Retained) -> Self {
        let mut cache = Self::default();
        cache.retained = Some(retained);
        static NEXT_CHARGE: AtomicU64 = AtomicU64::new(1);
        cache.retained_charge = NEXT_CHARGE.fetch_add(1, Ordering::Relaxed);
        cache
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
    pub fn finish(&mut self, _batch: &Batch) {
        if let Some(retained) = &mut self.retained {
            retained.remember(std::mem::take(&mut self.entries), &self.source_heads);
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
        self.order.clear();
        // The cache is optional: an allocation failure only skips caching, never the hour.
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.order, range.len()))
            .is_err()
        {
            self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
            return Ok(false);
        }
        self.order.extend(range);
        self.order
            .sort_unstable_by_key(|index| log.spans[*index].addr);
        let mut covering: Option<&collector_raw::raw::Span> = None;
        // Reused decodes need single-valued bytes even when read cursors start afresh.
        for index in &self.order {
            let span = &log.spans[*index];
            if !span.ok {
                if report {
                    self.counts[CacheOutcome::UnreadableCopy as usize] += 1;
                }
                return Ok(false);
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
                        if report {
                            self.counts[CacheOutcome::InsertOverlap as usize] += 1;
                        }
                        return Ok(false);
                    }
                }
                if previous_end >= end {
                    continue;
                }
            }
            covering = Some(span);
        }
        Ok(true)
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
            self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
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
            self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
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
        let range = event.start..event.end;
        self.miss = None;
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
                self.counts[CacheOutcome::FullCopyHit as usize] += 1;
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
                self.counts[CacheOutcome::ReadWitnessHit as usize] += 1;
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
                    self.counts[CacheOutcome::ReadWitnessHit as usize] += 1;
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
            let matched = engine_allocation::advisory(|| {
                if !self.consistent_for(&batch.log, range.clone(), false).ok()? {
                    return None;
                }
                let memory = copied::MemoryView::new(&batch.log, range.clone(), &self.order);
                let entry = self
                    .retained
                    .as_ref()?
                    .find(&memory, event, &mut self.comparison)?;
                let charge = entry.charge(self.retained_charge);
                Some((Arc::clone(&entry.value), charge))
            });
            if let Some((value, charge)) = matched {
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
                    self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
                }
                shared.heap_bytes += charge;
                self.counts[CacheOutcome::ReadWitnessHit as usize] += 1;
                return Ok(value);
            }
        }
        if let Some((latest, previous)) = self.recent.get(&event.pointer).copied() {
            for index in std::iter::once(latest).chain(previous) {
                if equal(&batch.log, self.entries[index].range.clone(), range.clone()) {
                    let value = self.reuse(index)?;
                    self.counts[CacheOutcome::FullCopyHit as usize] += 1;
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
                    self.counts[CacheOutcome::ReadWitnessHit as usize] += 1;
                    self.remember(event.pointer, index, range.clone());
                    return Ok(value);
                }
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
        for index in shaped
            .into_iter()
            .chain(recent.into_iter().flat_map(|(latest, previous)| {
                std::iter::once(latest)
                    .chain(previous)
                    .filter(move |index| Some(*index) != shaped)
            }))
        {
            let entry = &self.entries[index];
            let Some(witness) = entry.witness.as_ref() else {
                continue;
            };
            if Some(index) == shaped
                && entry.pointer == event.pointer
                && witness.equal(&batch.log, entry.range.clone(), range.clone())
            {
                let value = self.reuse(index)?;
                self.counts[CacheOutcome::ReadWitnessHit as usize] += 1;
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
                    self.counts[CacheOutcome::ReadWitnessHit as usize] += 1;
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
                let candidate = self.mission_candidate(&memory, event.pointer);
                self.order = order;
                candidate
            });
            if let Some(index) = matched {
                let value = self.reuse(index)?;
                self.counts[CacheOutcome::ReadWitnessHit as usize] += 1;
                self.remember(event.pointer, index, range.clone());
                return Ok(value);
            }
        }
        let key = fingerprint(&batch.log, range.clone(), event.pointer);
        self.decode_key(batch, event, base, hour, shared, key)
    }
    fn mission_candidate(&mut self, memory: &impl Memory, pointer: usize) -> Option<usize> {
        let model_key = missions::model_key(memory, pointer)?;
        // A single model selects the invariant reads; only the complete key selects candidates.
        let model = &self.entries[*self.mission_models.get(&model_key)?];
        let key = model.missions.as_ref()?.key(
            memory,
            model.witness.as_ref()?,
            model_key,
            &mut self.comparison,
        )?;
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
                self.counts[CacheOutcome::FullCopyHit as usize] += 1;
                self.remember(event.pointer, index, range.clone());
                return Ok(value);
            }
            next = entry.next;
        }
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
        self.counts[CacheOutcome::FullDecode as usize] += 1;
        self.counts[miss as usize] += 1;
        let stamp = profile::Stamp::new();
        let result = (|| {
            let recorded = crate::raw::Recorded::build_range(&batch.log, range.clone())?;
            let memory = witness::Observed::new(&recorded, range.len());
            let value = Arc::new(decode_snapshot(&memory, base, hour, event.pointer, false)?);
            shared.heap_bytes += value.allocated_bytes();
            if !value
                .context
                .as_ref()
                .is_some_and(|context| context.reusable())
            {
                self.counts[CacheOutcome::ContextNotReusable as usize] += 1;
                return Ok(value);
            }
            if !self.consistent(&batch.log, range.clone())? {
                return Ok(value);
            }
            let (complete, allocation_failed, read_failed) = memory.diagnostics();
            let witness = memory.finish(&batch.log, range.clone(), &self.order, true);
            if allocation_failed || (complete && witness.is_none()) {
                self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
            }
            if read_failed {
                self.counts[CacheOutcome::WitnessReadFailed as usize] += 1;
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
                self.index_missions(&recorded, event.pointer, proof);
            }
            Ok(value)
        })();
        self.full_decode.add(stamp.finish());
        result
    }
    fn index_missions(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        proof: Option<missions::Proof>,
    ) {
        let Some(proof) = proof else {
            return;
        };
        let indexed = engine_allocation::advisory(|| {
            let witness = self.entries.last()?.witness.as_ref()?;
            let model_key = missions::model_key(memory, pointer)?;
            let mission_key = proof.key(memory, witness, model_key, &mut self.comparison)?;
            engine_allocation::reserve_map(&mut self.mission_heads, 1).ok()?;
            engine_allocation::reserve_map(&mut self.mission_models, 1).ok()?;
            Some((model_key, mission_key))
        });
        if let Some((model_key, mission_key)) = indexed {
            let index = self.entries.len() - 1;
            self.mission_models.insert(model_key, index);
            self.entries[index].missions = Some(proof);
            self.entries[index].mission_next = self.mission_heads.insert(mission_key, index);
        } else {
            self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
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
            self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
            return false;
        }
        if witness.is_some()
            && engine_allocation::advisory(|| {
                engine_allocation::reserve_map(&mut self.witness_heads, 1)?;
                engine_allocation::reserve_map(&mut self.shape_heads, 1)
            })
            .is_err()
        {
            self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
            witness = None;
        }
        let witness_key = witness
            .as_ref()
            .and_then(|witness| witness.key(log, range.clone(), range.clone(), pointer));
        let index = self.entries.len();
        if witness_key.is_some() {
            self.shape_heads
                .insert(shape_key(log, range.clone(), pointer), index);
        }
        self.entries.push(Entry {
            range: range.clone(),
            pointer,
            next: self.heads.insert(key, index),
            value: Arc::clone(value),
            witness,
            witness_next: witness_key.and_then(|key| self.witness_heads.insert(key, index)),
            missions: None,
            mission_next: None,
        });
        self.remember(pointer, index, range);
        true
    }
}

#[cfg(test)]
mod tests;
