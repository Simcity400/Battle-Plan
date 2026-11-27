use super::super::witness::{MAX_WITNESS_BYTES, Recording, Witness};
use super::*;

const MIN_BURST: usize = 64;
const PARTS_PER_LANE: usize = 2;
const MIN_PER_PART: usize = 32;

pub(in super::super) struct WalkSummary {
    pub details: bool,
    pub stats_len: usize,
    pub modules: Vec<engine_equipment::equipment_module_container::Row>,
    pub first_upgrade: Option<engine_equipment::cequipment_upgrades_instance::Row>,
}

pub(in super::super) enum Read {
    Pending(Option<Witness>),
    Unchanged(Witness),
    Walked(WalkSummary, Box<super::Decoded>, Option<Witness>),
    Refused,
}

struct PartScratch {
    scratch: Vec<u8>,
    walk: super::super::RawWalk,
    largest: (usize, usize),
    rec_spans: Vec<(usize, u32, bool)>,
    rec_bytes: Vec<u8>,
}

impl PartScratch {
    fn vacant() -> Self {
        Self {
            scratch: Vec::new(),
            walk: super::super::RawWalk::vacant(),
            largest: (0, 0),
            rec_spans: Vec::new(),
            rec_bytes: Vec::new(),
        }
    }
}

static SCRATCH_POOL: Mutex<Vec<PartScratch>> = Mutex::new(Vec::new());

pub(in super::super) fn release() {
    SCRATCH_POOL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

pub struct Part {
    ordinal: usize,
    base: usize,
    hour: i32,
    marked: Vec<(usize, u64, Read)>,
    removed: Vec<usize>,
    witness_budget: usize,
    ns: u64,
}
impl Part {
    pub fn capture(&mut self) {
        let begin = std::time::Instant::now();
        self.read();
        self.ns = u64::try_from(begin.elapsed().as_nanos()).unwrap_or(u64::MAX);
    }
    fn read(&mut self) {
        let (base, hour) = (self.base, self.hour);
        // SAFETY: lanes run inside the capture's write boundary, as every acquisition read does:
        // the game is held, or every writer of the design data these lanes read waits at the
        // capture gate.
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let mut pooled = SCRATCH_POOL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .unwrap_or_else(PartScratch::vacant);
        let PartScratch {
            ref mut scratch,
            ref mut walk,
            ref mut largest,
            ref mut rec_spans,
            ref mut rec_bytes,
        } = pooled;
        let mut recorded_bytes = 0usize;
        for (pointer, id, read) in &mut self.marked {
            let Read::Pending(witness) = std::mem::replace(read, Read::Refused) else {
                continue;
            };
            let guard = super::super::repair::Surviving {
                source: &memory,
                removed: &self.removed,
                receiver: *pointer,
                consumed: std::cell::Cell::new(false),
            };
            let size = witness.as_ref().map_or(*largest, Witness::size);
            if let Some(witness) = witness
                && witness.unchanged(&guard, scratch)
                && !guard.consumed.get()
            {
                recorded_bytes = recorded_bytes.saturating_add(witness.retained_bytes());
                *read = Read::Unchanged(witness);
                continue;
            }
            guard.consumed.set(false);
            *read = engine_allocation::advisory(|| {
                walk.reset();
                let recorded = if recorded_bytes < self.witness_budget {
                    let recording = Recording::with_scratch(
                        &guard,
                        std::mem::take(rec_spans),
                        std::mem::take(rec_bytes),
                        size,
                    );
                    let walked =
                        super::super::walk_snapshot(&recording, base, hour, *pointer, walk)
                            .is_ok();
                    let (recorded, spans_back, bytes_back) = recording.finish(*id, (0, false));
                    *rec_spans = spans_back;
                    *rec_bytes = bytes_back;
                    if !walked {
                        return None;
                    }
                    if let Some(recorded) = &recorded {
                        let size = recorded.size();
                        *largest = (largest.0.max(size.0), largest.1.max(size.1));
                        recorded_bytes = recorded_bytes.saturating_add(recorded.retained_bytes());
                    }
                    recorded
                } else {
                    super::super::walk_snapshot(&guard, base, hour, *pointer, walk).ok()?;
                    None
                };
                if guard.consumed.get() || !walk.complete() {
                    return None;
                }
                let decoded = Box::new(super::super::decode_walk(walk).ok()?);
                let modules = engine_allocation::copy_vec(
                    &walk.modules.rows[..walk.modules.outcome.len()],
                )
                .ok()?;
                let summary = WalkSummary {
                    details: walk.details,
                    stats_len: walk.stats.outcome.len(),
                    modules,
                    first_upgrade: walk.upgrades.rows[..walk.upgrades.outcome.len()]
                        .first()
                        .copied(),
                };
                Some(Read::Walked(summary, decoded, recorded))
            })
            .unwrap_or(Read::Refused);
        }
        let mut pool = SCRATCH_POOL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pool.len() < 32
            && engine_allocation::advisory(|| engine_allocation::reserve(&mut *pool, 1)).is_ok()
        {
            pool.push(pooled);
        }
    }
}

fn fallback_reads(watches: &[Watch]) -> Result<Vec<(usize, u64, Read)>, &'static str> {
    let mut fallback = engine_allocation::vec(watches.len())
        .map_err(|_| "variant boundary read allocation failed")?;
    for watch in watches {
        fallback.push((watch.pointer, watch.id, Read::Refused));
    }
    Ok(fallback)
}

pub(in super::super) fn read_marked(
    speculation: Option<&mut super::super::Speculation<'_>>,
    base: usize,
    hour: i32,
    watches: &[Watch],
    witnesses: Vec<Option<Witness>>,
    removed: &[usize],
) -> Result<Vec<(usize, u64, Read)>, &'static str> {
    let mut witnesses = witnesses.into_iter();
    let mut marked = engine_allocation::vec(watches.len())
        .map_err(|_| "variant boundary read allocation failed")?;
    for watch in watches {
        marked.push((
            watch.pointer,
            watch.id,
            Read::Pending(witnesses.next().flatten()),
        ));
    }
    let removed = engine_allocation::copy_vec(removed)
        .map_err(|_| "variant boundary read allocation failed")?;
    let mut parts = match speculation {
        Some(speculation) if speculation.lanes >= 2 && marked.len() >= MIN_BURST => {
            let per_part = marked
                .len()
                .div_ceil(speculation.lanes * PARTS_PER_LANE)
                .max(MIN_PER_PART);
            let part_count = marked.len().div_ceil(per_part);
            let witness_budget = MAX_WITNESS_BYTES.div_ceil(part_count.max(1));
            let mut parts = engine_allocation::vec(part_count)
                .map_err(|_| "variant boundary read allocation failed")?;
            let mut marked = marked.into_iter();
            for ordinal in 0..part_count {
                parts.push(Part {
                    ordinal,
                    base,
                    hour,
                    marked: engine_allocation::collect_vec(marked.by_ref().take(per_part))
                        .map_err(|_| "variant boundary read allocation failed")?,
                    removed: engine_allocation::copy_vec(&removed)
                        .map_err(|_| "variant boundary read allocation failed")?,
                    witness_budget,
                    ns: 0,
                });
            }
            let Ok(mut parts) = (speculation.run)(parts) else {
                return fallback_reads(watches);
            };
            parts.sort_unstable_by_key(|part| part.ordinal);
            if parts.len() != part_count
                || parts
                    .iter()
                    .enumerate()
                    .any(|(expected, part)| part.ordinal != expected)
            {
                return fallback_reads(watches);
            }
            parts
        }
        _ => {
            let mut part = Part {
                ordinal: 0,
                base,
                hour,
                marked,
                removed,
                witness_budget: MAX_WITNESS_BYTES,
                ns: 0,
            };
            part.capture();
            let mut parts = engine_allocation::vec(1)
                .map_err(|_| "variant boundary read allocation failed")?;
            parts.push(part);
            parts
        }
    };
    let mut reads = engine_allocation::vec(watches.len())
        .map_err(|_| "variant boundary read allocation failed")?;
    {
        let mut profile = super::super::drain_profile_mut();
        profile.lanes = parts.len() as u32;
        profile.lane_ns = [
            parts.iter().map(|part| part.ns).sum(),
            parts.iter().map(|part| part.ns).max().unwrap_or(0),
        ];
    }
    for part in &mut parts {
        for item in part.marked.drain(..) {
            if reads.len() == watches.len() {
                return fallback_reads(watches);
            }
            reads.push(item);
        }
    }
    if reads.len() != watches.len() {
        return fallback_reads(watches);
    }
    Ok(reads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_worker_parts_restore_watch_order_and_identity() {
        let watches: Vec<Watch> = (0..MIN_BURST)
            .map(|index| Watch {
                pointer: (index + 1) * 0x1000,
                id: (index as u64) + 100,
                flags: [0; 3],
                dirty: true,
            })
            .collect();
        let witnesses: Vec<Option<Witness>> =
            std::iter::repeat_with(|| None).take(watches.len()).collect();
        let mut run = |mut parts: Vec<Part>| {
            assert!(parts.len() >= 2);
            for part in &mut parts {
                for (_, _, read) in &mut part.marked {
                    *read = Read::Refused;
                }
            }
            parts.reverse();
            Ok(parts)
        };
        let mut speculation = super::super::Speculation {
            lanes: 2,
            run: &mut run,
        };
        let reads = read_marked(Some(&mut speculation), 0, 1, &watches, witnesses, &[]).unwrap();
        assert_eq!(reads.len(), watches.len());
        for (watch, (pointer, id, read)) in watches.iter().zip(reads) {
            assert_eq!((pointer, id), (watch.pointer, watch.id));
            assert!(matches!(read, Read::Refused));
        }

        let mut drop_one = |mut parts: Vec<Part>| {
            parts.pop();
            Ok(parts)
        };
        let mut short_speculation = super::super::Speculation {
            lanes: 2,
            run: &mut drop_one,
        };
        let fallback = read_marked(
            Some(&mut short_speculation),
            0,
            1,
            &watches,
            std::iter::repeat_with(|| None).take(watches.len()).collect(),
            &[],
        )
        .unwrap();
        assert_eq!(fallback.len(), watches.len());
        for (watch, (pointer, id, read)) in watches.iter().zip(fallback) {
            assert_eq!((pointer, id), (watch.pointer, watch.id));
            assert!(matches!(read, Read::Refused));
        }
    }
}
