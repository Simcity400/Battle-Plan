use super::*;

pub(super) struct Entry {
    pub value: Arc<Decoded>,
    pub witness: witness::Witness,
    pub(super) missions: Option<missions::Proof>,
    layout: Option<witness::Layout>,
    charged: AtomicU64,
}
impl Entry {
    pub(super) fn charge(&self, stamp: u64) -> usize {
        if self.charged.swap(stamp, Ordering::Relaxed) == stamp {
            0
        } else {
            self.value.allocated_bytes()
        }
    }
}
struct Design {
    pointer: usize,
    capacity: u32,
    states: HashMap<u32, Entry>,
}
#[derive(Default)]
pub(in crate::production::design_changes) struct Retained {
    epoch: Option<(u64, usize)>,
    designs: HashMap<u64, Design>,
    #[cfg(test)]
    capacity_visits: usize,
}
static RETAINED: OnceLock<Mutex<Retained>> = OnceLock::new();
pub(in crate::production::design_changes) fn retained(
    epoch: u64,
    base: usize,
) -> std::sync::MutexGuard<'static, Retained> {
    let mut retained = RETAINED
        .get_or_init(|| Mutex::new(Retained::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    retained.reset(epoch, base);
    retained
}
pub(in crate::production::design_changes) fn prune(watches: &HashMap<usize, Watch>) {
    if let Some(retained) = RETAINED.get() {
        retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prune(watches);
    }
}
pub(in crate::production::design_changes) fn release() {
    if let Some(retained) = RETAINED.get() {
        retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .release();
    }
}
impl Retained {
    pub(super) fn reset(&mut self, epoch: u64, base: usize) {
        if self.epoch != Some((epoch, base)) {
            self.release();
            self.epoch = Some((epoch, base));
        }
    }
    pub(super) fn prune(&mut self, watches: &HashMap<usize, Watch>) {
        self.designs.retain(|id, design| {
            watches
                .get(&design.pointer)
                .is_some_and(|watch| watch.id == *id)
        });
    }
    pub(super) fn release(&mut self) {
        *self = Self::default();
    }
    #[cfg(test)]
    pub(super) fn state_count(&self) -> usize {
        self.designs
            .values()
            .map(|design| design.states.len())
            .sum()
    }
    pub(super) fn remove(&mut self, variant: u64) {
        self.designs.remove(&variant);
    }
    pub(super) fn observe(&mut self, id: u64, pointer: usize, capacity: Option<u32>) {
        let Some(capacity) = capacity else {
            self.remove(id);
            return;
        };
        let Some(design) = self.designs.get_mut(&id) else {
            engine_allocation::advisory(|| {
                if engine_allocation::reserve_map(&mut self.designs, 1).is_ok() {
                    self.designs.insert(
                        id,
                        Design {
                            pointer,
                            capacity,
                            states: HashMap::new(),
                        },
                    );
                }
            });
            return;
        };
        if design.pointer != pointer {
            design.pointer = pointer;
            design.states.clear();
        } else if capacity < design.capacity {
            design.states.retain(|count, _| {
                #[cfg(test)]
                {
                    self.capacity_visits += 1;
                }
                *count <= capacity
            });
        }
        design.capacity = capacity;
    }
    pub(super) fn contains(&self, event: &Event) -> bool {
        self.designs
            .get(&event.variant)
            .is_some_and(|design| design.pointer == event.pointer && !design.states.is_empty())
    }
    /// The current state at `count` when this occurrence repeats its kept layout and read bytes.
    pub(super) fn find_repeat(
        &self,
        log: &RawLog<1>,
        range: std::ops::Range<usize>,
        event: &Event,
        count: u32,
    ) -> Option<&Entry> {
        let design = self.designs.get(&event.variant)?;
        if design.pointer != event.pointer {
            return None;
        }
        let entry = design.states.get(&count)?;
        entry
            .witness
            .repeats(entry.layout.as_ref()?, log, range)
            .then_some(entry)
    }
    pub(super) fn find(
        &self,
        memory: &impl Memory,
        event: &Event,
        buffer: &mut Vec<u8>,
        candidate_reads: &mut u64,
    ) -> Option<(u32, &Entry)> {
        let count =
            Array::read(memory, event.pointer.checked_add(native::BONUS_SOURCES)?, 8)?.count;
        let design = self.designs.get(&event.variant)?;
        if design.pointer != event.pointer {
            return None;
        }
        let entry = design.states.get(&count)?;
        *candidate_reads += entry.witness.requests.len() as u64;
        let matches = if let Some(proof) = &entry.missions {
            proof.equal(
                memory,
                &entry.witness,
                event.pointer,
                &entry.value.definition.design.mission_stats,
                buffer,
            )
        } else {
            entry.witness.matches(memory, buffer)
        };
        (matches == Some(true)).then_some((count, entry))
    }
    /// The state a verdict named, while it is still this design's state at `count`.
    pub(super) fn verified(&self, event: &Event, count: u32, value: usize) -> Option<&Entry> {
        let design = self.designs.get(&event.variant)?;
        let entry = design.states.get(&count)?;
        (design.pointer == event.pointer && Arc::as_ptr(&entry.value) as usize == value)
            .then_some(entry)
    }
    pub(super) fn remember(
        &mut self,
        log: &RawLog<1>,
        entries: Vec<super::Entry>,
        heads: &HashMap<u64, SourceHeads>,
    ) {
        engine_allocation::advisory(|| {
            for (index, entry) in entries.into_iter().enumerate() {
                let count = entry.value.definition.design.bonus_sources.len() as u32;
                let Some(heads) = heads
                    .get(&entry.value.definition.identity.id)
                    .filter(|heads| heads.pointer == entry.pointer)
                else {
                    continue;
                };
                // Optional candidate maps can disappear; only an observed design permits promotion.
                let Some(design) = self
                    .designs
                    .get_mut(&entry.value.definition.identity.id)
                    .filter(|design| design.pointer == entry.pointer)
                else {
                    continue;
                };
                if count > design.capacity
                    || !heads
                        .states
                        .get(&count)
                        .is_some_and(|head| head.index == Some(index))
                {
                    continue;
                }
                let Some(witness) = entry.witness else {
                    continue;
                };
                if witness.requests.is_empty() {
                    continue;
                }
                if !entry.value.definition.design.mission_stats.is_empty()
                    && entry.missions.is_none()
                {
                    continue;
                }
                // One current witness per native source count, never a history of design revisions.
                if engine_allocation::reserve_map(&mut design.states, 1).is_err() {
                    continue;
                }
                // Optional: without it, only the recorded reads prove a repeat.
                let layout = witness.layout(log, entry.range);
                design.states.insert(
                    count,
                    Entry {
                        value: entry.value,
                        witness,
                        missions: entry.missions,
                        layout,
                        charged: AtomicU64::new(0),
                    },
                );
            }
        });
    }
}

#[cfg(test)]
mod tests;
