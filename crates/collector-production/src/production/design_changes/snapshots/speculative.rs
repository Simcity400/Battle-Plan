//! A burst's retained-cache tests and likely full-decode misses, run ahead on the Variants
//! lanes. Test lanes read the batch and the retained cache in place; walk lanes own copies of
//! their events' spans and run only the pure walk. The ordered pass takes every shared effect
//! and consumes a walk only for byte-identical spans.
use super::*;
use crate::raw::{RawLog, Recorded};
use collector_raw::raw::Span;

/// Fewer likely misses than this decode inline: the lanes' dispatch would cost more.
const MIN_BURST: usize = 64;
/// Fewer retained tests than this run inline.
const MIN_CHECKS: usize = 64;

/// One lane's copied events and their walks, or its share of the retained tests.
pub struct Part {
    base: usize,
    hour: i32,
    jobs: Vec<Job>,
    checks: Option<Checks>,
}
/// Retained tests of the batch's events at `indices`, read in place.
struct Checks {
    batch: *const Batch,
    retained: *const Retained,
    indices: Vec<usize>,
    results: Vec<Check>,
    scratch: Scratch,
}
// SAFETY: the planning thread holds the batch and the retained guard, and blocks in
// `Speculation::run` until every part has returned; lanes only read through these pointers.
unsafe impl Send for Checks {}
impl Checks {
    fn capture(&mut self) {
        let _phase = profile::Timer::new(profile::Phase::SpeculateRetained);
        // SAFETY: see `Send`; both outlive this part and nothing writes them meanwhile.
        let (batch, retained) = unsafe { (&*self.batch, &*self.retained) };
        // Optional work: a refusal leaves these events to the ordered pass.
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.results, self.indices.len()))
            .is_err()
        {
            return;
        }
        for &index in &self.indices {
            self.results.push(check_retained(
                retained,
                &batch.log,
                &batch.events[index],
                &mut self.scratch,
            ));
        }
    }
}
/// The plan's walks and retained tests by event index, and its tests' counters.
#[derive(Default)]
pub(super) struct Planned {
    pub walks: Vec<Option<Box<Prepared>>>,
    pub checks: Vec<Check>,
    pub scratch: Scratch,
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
        if let Some(checks) = &mut self.checks {
            checks.capture();
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
/// Tests the burst's first occurrences of each pointer, and those whose span count differs from
/// that pointer's previous one, against the retained cache; then walks those it cannot reuse on
/// the lanes. Returns both by event index; an empty table where the burst is small or anything is
/// refused leaves that work to the ordered pass.
pub(super) fn speculate(
    batch: &Batch,
    base: usize,
    hour: i32,
    speculation: &mut super::super::Speculation<'_>,
    retained: Option<&Retained>,
) -> Planned {
    let mut planned = Planned::default();
    engine_allocation::advisory(|| plan(batch, base, hour, speculation, retained, &mut planned));
    planned
}
fn plan(
    batch: &Batch,
    base: usize,
    hour: i32,
    speculation: &mut super::super::Speculation<'_>,
    retained: Option<&Retained>,
    planned: &mut Planned,
) -> Option<()> {
    if speculation.lanes < 2 {
        return None;
    }
    let mut seen = HashMap::new();
    engine_allocation::reserve_map(&mut seen, batch.events.len()).ok()?;
    let mut first = engine_allocation::vec(batch.events.len()).ok()?;
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
        if seen.insert(event.pointer, len) != Some(len) {
            first.push(index);
        }
    }
    if let Some(retained) = retained {
        let _ = check(batch, base, hour, speculation, retained, &first, planned);
    }
    let mut candidates = engine_allocation::vec(first.len()).ok()?;
    candidates.extend(
        first
            .into_iter()
            .filter(|&index| !matches!(planned.checks.get(index), Some(Check::Hit(_)))),
    );
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
        parts.push(Part {
            base,
            hour,
            jobs,
            checks: None,
        });
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
    planned.walks = table;
    Some(())
}
/// The retained tests of `first`'s retained designs, on the lanes for a large burst.
fn check(
    batch: &Batch,
    base: usize,
    hour: i32,
    speculation: &mut super::super::Speculation<'_>,
    retained: &Retained,
    first: &[usize],
    planned: &mut Planned,
) -> Option<()> {
    let mut indices = engine_allocation::vec(first.len()).ok()?;
    indices.extend(
        first
            .iter()
            .copied()
            .filter(|&index| retained.contains(&batch.events[index])),
    );
    let mut checks = Vec::new();
    engine_allocation::reserve(&mut checks, batch.events.len()).ok()?;
    checks.resize_with(batch.events.len(), Check::default);
    if indices.len() < MIN_CHECKS {
        let _phase = profile::Timer::new(profile::Phase::SpeculateRetained);
        for &index in &indices {
            checks[index] =
                check_retained(retained, &batch.log, &batch.events[index], &mut planned.scratch);
        }
        planned.checks = checks;
        return Some(());
    }
    let per_lane = indices.len().div_ceil(speculation.lanes);
    let mut parts = engine_allocation::vec(speculation.lanes).ok()?;
    for chunk in indices.chunks(per_lane) {
        parts.push(Part {
            base,
            hour,
            jobs: Vec::new(),
            checks: Some(Checks {
                batch,
                retained,
                indices: engine_allocation::copy_vec(chunk).ok()?,
                results: Vec::new(),
                scratch: Scratch::default(),
            }),
        });
    }
    // Every part returns before `run` does, so no lane still reads the batch or the cache.
    let parts = (speculation.run)(parts).ok()?;
    for part in parts {
        let Some(mut part) = part.checks else {
            continue;
        };
        for (index, check) in part.indices.iter().zip(part.results) {
            checks[*index] = check;
        }
        planned.scratch.absorb(&mut part.scratch);
    }
    planned.checks = checks;
    Some(())
}
