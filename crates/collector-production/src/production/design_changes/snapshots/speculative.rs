//! The hour's marked designs, read live on the Variants lanes at the boundary: a design whose
//! witness reads back the same needs nothing more; any other is walked, its reads recorded, and
//! decoded. The drain runs every decoded walk's shared effects in design order.
use super::super::witness::{Recording, Witness};
use super::*;

/// Fewer marked designs than this are read inline: the lanes' dispatch would cost more.
const MIN_BURST: usize = 64;
/// Parts per lane: a part no worker has started runs on the parent, so smaller parts spread
/// a busy pool's leftovers across the parent and every worker that frees up.
const PARTS_PER_LANE: usize = 4;

/// A marked design's boundary read.
pub(in super::super) enum Read {
    Pending(Option<Witness>),
    /// Every span of its witness read back the same.
    Unchanged(Witness),
    /// A complete walk and its decode, and the witness of its reads where they could all be held.
    Walked(
        Box<super::super::RawWalk>,
        Box<super::Decoded>,
        Option<Witness>,
    ),
    /// Refused, consumed or undecodable: the drain reads it again in order, which reports why.
    Refused,
}

/// One lane's marked designs and the hour's consumed designs they must not read.
pub struct Part {
    base: usize,
    hour: i32,
    marked: Vec<(usize, u64, Read)>,
    removed: Vec<usize>,
    ns: u64,
}
impl Part {
    /// Lane work: live reads of the marked designs only; nothing shared is touched.
    pub fn capture(&mut self) {
        let begin = std::time::Instant::now();
        self.read();
        self.ns = u64::try_from(begin.elapsed().as_nanos()).unwrap_or(u64::MAX);
    }
    fn read(&mut self) {
        let (base, hour) = (self.base, self.hour);
        // SAFETY: lanes run while the hourly original is parked, as every acquisition read does.
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let mut scratch = Vec::new();
        // A design read for the first time presizes from the largest recording so far.
        let mut largest = (0, 0);
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
            let size = witness.as_ref().map_or(largest, Witness::size);
            if let Some(witness) = witness
                && witness.unchanged(&guard, &mut scratch)
                && !guard.consumed.get()
            {
                *read = Read::Unchanged(witness);
                continue;
            }
            guard.consumed.set(false);
            // Optional work: a refused or consumed walk is read again by the drain.
            *read = engine_allocation::advisory(|| {
                let recording = Recording::new(&guard, size);
                let mut walk = Box::new(super::super::RawWalk::vacant());
                super::super::walk_snapshot(&recording, base, hour, *pointer, &mut walk).ok()?;
                let recorded = recording.witness(*id, (0, false));
                if let Some(recorded) = &recorded {
                    let size = recorded.size();
                    largest = (largest.0.max(size.0), largest.1.max(size.1));
                }
                if guard.consumed.get() || !walk.complete() {
                    return None;
                }
                let decoded = Box::new(super::super::decode_walk(&mut walk).ok()?);
                Some(Read::Walked(walk, decoded, recorded))
            })
            .unwrap_or(Read::Refused);
        }
    }
}

/// Reads the hour's marked designs live, on the lanes when there are enough to share, by
/// pointer (the lanes return parts in any order).
pub(in super::super) fn read_marked(
    speculation: Option<&mut super::super::Speculation<'_>>,
    base: usize,
    hour: i32,
    watches: &[Watch],
    witnesses: Vec<Option<Witness>>,
    removed: &[usize],
) -> Result<HashMap<usize, Read>, &'static str> {
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
            let per_part = marked.len().div_ceil(speculation.lanes * PARTS_PER_LANE);
            let mut parts = engine_allocation::vec(marked.len().div_ceil(per_part))
                .map_err(|_| "variant boundary read allocation failed")?;
            let mut marked = marked.into_iter();
            for _ in 0..marked.len().div_ceil(per_part) {
                parts.push(Part {
                    base,
                    hour,
                    marked: engine_allocation::collect_vec(marked.by_ref().take(per_part))
                        .map_err(|_| "variant boundary read allocation failed")?,
                    removed: engine_allocation::copy_vec(&removed)
                        .map_err(|_| "variant boundary read allocation failed")?,
                    ns: 0,
                });
            }
            // A lane failure leaves every design to the drain's own read, in order.
            let Ok(parts) = (speculation.run)(parts) else {
                return Ok(HashMap::new());
            };
            parts
        }
        _ => {
            let mut part = Part {
                base,
                hour,
                marked,
                removed,
                ns: 0,
            };
            part.capture();
            let mut parts = engine_allocation::vec(1)
                .map_err(|_| "variant boundary read allocation failed")?;
            parts.push(part);
            parts
        }
    };
    let mut reads = HashMap::new();
    engine_allocation::reserve_map(&mut reads, watches.len())
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
        for (pointer, _, read) in part.marked.drain(..) {
            reads.insert(pointer, read);
        }
    }
    Ok(reads)
}
