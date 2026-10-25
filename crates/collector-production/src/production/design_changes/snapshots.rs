use super::*;
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
}
#[derive(Default)]
pub(super) struct Cache {
    heads: HashMap<u64, usize>,
    witness_heads: HashMap<u64, usize>,
    shape_heads: HashMap<u64, usize>,
    entries: Vec<Entry>,
    recent: HashMap<usize, (usize, Option<usize>)>,
    order: Vec<usize>,
    counts: [u64; profile::CACHE_FIELDS],
    full_decode: profile::Cost,
    miss: Option<CacheOutcome>,
}
impl Drop for Cache {
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
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.addr == right.addr
                && left.len == right.len
                && left.ok == right.ok
                && left.bulk == right.bulk
                && payload(log, left) == payload(log, right)
        })
}
impl Cache {
    fn consistent(
        &mut self,
        log: &RawLog<1>,
        range: std::ops::Range<usize>,
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
                self.counts[CacheOutcome::UnreadableCopy as usize] += 1;
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
                        self.counts[CacheOutcome::InsertOverlap as usize] += 1;
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
        let value = &self.entries[index].value;
        sources::track(&value.definition)?;
        liveness::sample(&value.definition)?;
        value
            .context
            .as_ref()
            .ok_or("variant cached context observation missing")?
            .apply();
        Ok(Arc::clone(value))
    }
    fn remember(&mut self, pointer: usize, index: usize) {
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
        if let Some((latest, previous)) = self.recent.get(&event.pointer).copied() {
            for index in std::iter::once(latest).chain(previous) {
                if equal(&batch.log, self.entries[index].range.clone(), range.clone()) {
                    let value = self.reuse(index)?;
                    self.counts[CacheOutcome::FullCopyHit as usize] += 1;
                    self.remember(event.pointer, index);
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
                    self.remember(event.pointer, index);
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
                self.remember(event.pointer, index);
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
                let entry = &self.entries[index];
                if entry.pointer == event.pointer
                    && entry.witness.as_ref().is_some_and(|witness| {
                        witness.equal(&batch.log, entry.range.clone(), range.clone())
                    })
                {
                    let value = self.reuse(index)?;
                    self.counts[CacheOutcome::ReadWitnessHit as usize] += 1;
                    self.remember(event.pointer, index);
                    return Ok(value);
                }
                next = self.entries[index].witness_next;
            }
        }
        let key = fingerprint(&batch.log, range.clone(), event.pointer);
        self.decode_key(batch, event, base, hour, shared, key)
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
            let entry = &self.entries[index];
            // A hash only selects candidates; every copied witness must still match.
            if entry.pointer == event.pointer
                && equal(&batch.log, entry.range.clone(), range.clone())
            {
                let value = self.reuse(index)?;
                self.counts[CacheOutcome::FullCopyHit as usize] += 1;
                self.remember(event.pointer, index);
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
            let memory = crate::raw::Recorded::build_range(&batch.log, range.clone())?;
            let memory = witness::Observed::new(&memory, range.len());
            let value = Arc::new(decode_snapshot(&memory, base, hour, event.pointer, false)?);
            shared.heap_bytes += size_of::<Decoded>()
                + 2 * size_of::<usize>()
                + replay_data::value::Value::heap_bytes(&value.value)
                + rows::NativeValue::heap_bytes(value.definition.design.as_ref())
                + value
                    .definition
                    .source_links
                    .as_ref()
                    .map_or(0, |links| links.direct.capacity() * size_of::<usize>())
                + value
                    .context
                    .as_ref()
                    .map_or(0, super::super::design_context::Event::allocated_bytes);
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
            let witness = memory.finish(&batch.log, range.clone(), &self.order);
            if allocation_failed || (complete && witness.is_none()) {
                self.counts[CacheOutcome::AllocationSkipped as usize] += 1;
            }
            if read_failed {
                self.counts[CacheOutcome::WitnessReadFailed as usize] += 1;
            }
            self.insert(&batch.log, range, event.pointer, key, &value, witness);
            Ok(value)
        })();
        self.full_decode.add(stamp.finish());
        result
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
            range,
            pointer,
            next: self.heads.insert(key, index),
            value: Arc::clone(value),
            witness,
            witness_next: witness_key.and_then(|key| self.witness_heads.insert(key, index)),
        });
        self.remember(pointer, index);
        true
    }
}

#[cfg(test)]
mod tests;
