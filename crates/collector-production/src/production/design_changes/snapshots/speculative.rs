//! A burst's likely full-decode misses walked ahead on the Variants lanes. Each lane owns
//! copies of its events' spans and runs only the pure walk; the ordered pass decides every
//! hit or miss and consumes a walk only for byte-identical spans.
use super::*;
use crate::raw::{RawLog, Recorded};
use collector_raw::raw::Span;

/// Fewer likely misses than this decode inline: the lanes' dispatch would cost more.
const MIN_BURST: usize = 64;

/// One lane's copied events and their walks.
pub struct Part {
    base: usize,
    hour: i32,
    jobs: Vec<Job>,
}
struct Job {
    index: usize,
    pointer: usize,
    spans: Vec<Span>,
    bytes: Vec<u8>,
    has_bulk: bool,
    prepared: Option<Prepared>,
}
/// A completed pure walk, with the inputs it read and its observation so far.
pub(super) struct Prepared {
    spans: Vec<Span>,
    bytes: Vec<u8>,
    walk: Box<super::super::RawWalk>,
    observed: witness::State,
}
impl Part {
    /// Lane work: only the copied spans are read, and nothing shared is touched.
    pub fn capture(&mut self) {
        let (base, hour) = (self.base, self.hour);
        for job in &mut self.jobs {
            // Optional work: an allocation refusal drops the speculation, never the hour.
            job.prepared = engine_allocation::advisory(|| job.walk(base, hour));
        }
    }
}
impl Job {
    fn walk(&mut self, base: usize, hour: i32) -> Option<Prepared> {
        let mut log = RawLog::<1>::new(self.spans.len(), self.bytes.len());
        if log.spans.capacity() < self.spans.len() || log.bytes.capacity() < self.bytes.len() {
            return None;
        }
        log.spans.extend_from_slice(&self.spans);
        log.bytes.extend_from_slice(&self.bytes);
        log.has_bulk = self.has_bulk;
        let recorded = Recorded::build_range(&log, 0..self.spans.len()).ok()?;
        let observed = witness::Observed::new(&recorded, self.spans.len());
        let mut walk = Box::new(super::super::RawWalk::vacant());
        super::super::walk_snapshot(&observed, base, hour, self.pointer, &mut walk).ok()?;
        let observed = observed.into_state();
        if !walk.complete() || observed.allocation_failed() {
            return None;
        }
        Some(Prepared {
            spans: std::mem::take(&mut self.spans),
            bytes: std::mem::take(&mut self.bytes),
            walk,
            observed,
        })
    }
}
impl Prepared {
    /// The ordered pass's spans are those this walk read: addresses, lengths, flags, bytes.
    pub(super) fn matches(&self, log: &RawLog<1>, range: std::ops::Range<usize>) -> bool {
        let Some(spans) = log.spans.get(range) else {
            return false;
        };
        spans.len() == self.spans.len()
            && spans.iter().zip(&self.spans).all(|(span, copy)| {
                span.addr == copy.addr
                    && span.len == copy.len
                    && span.ok == copy.ok
                    && span.mask == copy.mask
                    && span.bulk == copy.bulk
                    && (!span.ok
                        || self
                            .bytes
                            .get(copy.start as usize..)
                            .and_then(|bytes| bytes.get(..copy.len as usize))
                            == Some(payload(log, span)))
            })
    }
    pub(super) fn into_parts(self) -> (Box<super::super::RawWalk>, witness::State) {
        (self.walk, self.observed)
    }
}
/// Walks the batch's likely misses on the lanes: first occurrences of a pointer, and
/// occurrences whose span count differs from that pointer's previous one, except those
/// `reusable` (the retained cache's own hit test) reports. Returns the completed walks by
/// event index; empty when the burst is small or anything is refused.
pub(super) fn speculate(
    batch: &Batch,
    base: usize,
    hour: i32,
    speculation: &mut super::super::Speculation<'_>,
    reusable: &mut dyn FnMut(&Event) -> bool,
) -> Vec<Option<Box<Prepared>>> {
    engine_allocation::advisory(|| plan(batch, base, hour, speculation, reusable))
        .unwrap_or_default()
}
fn plan(
    batch: &Batch,
    base: usize,
    hour: i32,
    speculation: &mut super::super::Speculation<'_>,
    reusable: &mut dyn FnMut(&Event) -> bool,
) -> Option<Vec<Option<Box<Prepared>>>> {
    if speculation.lanes < 2 {
        return None;
    }
    let mut seen = HashMap::new();
    engine_allocation::reserve_map(&mut seen, batch.events.len()).ok()?;
    let mut candidates = engine_allocation::vec(batch.events.len()).ok()?;
    for (index, event) in batch.events.iter().enumerate() {
        if event.kind != Kind::Design {
            continue;
        }
        if event.removed {
            seen.remove(&event.pointer);
            continue;
        }
        if event.flags.is_some() {
            continue;
        }
        let len = event.end - event.start;
        if seen.insert(event.pointer, len) != Some(len) && !reusable(event) {
            candidates.push(index);
        }
    }
    if candidates.len() < MIN_BURST {
        return None;
    }
    let per_lane = candidates.len().div_ceil(speculation.lanes);
    let mut parts = engine_allocation::vec(speculation.lanes).ok()?;
    for chunk in candidates.chunks(per_lane) {
        let mut jobs = engine_allocation::vec(chunk.len()).ok()?;
        for &index in chunk {
            let event = &batch.events[index];
            let source = batch.log.spans.get(event.start..event.end)?;
            let len = source
                .iter()
                .filter(|span| span.ok)
                .try_fold(0_usize, |len, span| len.checked_add(span.len as usize))?;
            let mut spans = engine_allocation::vec(source.len()).ok()?;
            let mut bytes = engine_allocation::vec(len).ok()?;
            for span in source {
                let mut copy = *span;
                if span.ok {
                    copy.start = u32::try_from(bytes.len()).ok()?;
                    bytes.extend_from_slice(payload(&batch.log, span));
                }
                spans.push(copy);
            }
            jobs.push(Job {
                index,
                pointer: event.pointer,
                spans,
                bytes,
                has_bulk: batch.log.has_bulk,
                prepared: None,
            });
        }
        parts.push(Part { base, hour, jobs });
    }
    let parts = (speculation.run)(parts).ok()?;
    let mut table = Vec::new();
    engine_allocation::reserve(&mut table, batch.events.len()).ok()?;
    table.resize_with(batch.events.len(), || None);
    for part in parts {
        for job in part.jobs {
            if let Some(prepared) = job.prepared {
                table[job.index] = Some(Box::new(prepared));
            }
        }
    }
    Some(table)
}
