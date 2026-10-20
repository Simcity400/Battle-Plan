//! Native pointer certificates are acquisition-only. Replay uses compact IDs.
use super::*;
use engine_events::event_feed_sites::CountryIdentity;
use engine_events::{event_feed_sites::provenance::World, script_event_registry as native};
use replay_data::event_feed_rows::EventFeedScriptChange as Change;

#[derive(Clone, Copy, Default)]
pub(super) struct Slot {
    pub pointer: usize,
    pub id: u32,
    pub owner: CountryIdentity,
}
#[derive(Clone, Copy, Default)]
struct Country {
    pointer: usize,
    id: u32,
    ready: bool,
}
#[derive(Default)]
pub(super) struct Index {
    slots: Vec<Slot>,
    countries: Vec<Country>,
    /// The world these labels belong to. An observation loss keeps it, so a
    /// later timer of the same world never reuses an earlier timer's label.
    generation: Option<u64>,
    invalidated: bool,
    revoked: bool,
    next: u64,
    live: usize,
    pub demand: usize,
}
impl Index {
    pub fn bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.slots)
            + crate::raw_memory::vector_bytes(&self.countries)
    }
    pub fn capacity(&self) -> usize {
        (self.slots.len() / 2).max(self.countries.len())
    }
    pub fn covers(&self, other: &Self) -> bool {
        self.slots.len() >= other.slots.len() && self.countries.len() >= other.countries.len()
    }
    /// Every certificate retires at the next boundary; the labels do not.
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }
    pub fn revoke_retained(&mut self) {
        // Callback-side revocation is constant work. The boundary worker
        // retires ready certificates without erasing failed owner attempts.
        self.revoked = true;
    }
    pub fn reset(&mut self, generation: u64) {
        let world = self.generation != Some(generation);
        if world || self.invalidated {
            self.slots.fill(Slot::default());
            self.countries.fill(Country::default());
            self.live = 0;
        } else if self.revoked {
            self.slots.fill(Slot::default());
            for country in &mut self.countries {
                if country.ready {
                    *country = Country::default();
                }
            }
            self.live = 0;
        }
        // Only a new world restarts its labels.
        if world {
            self.generation = Some(generation);
            self.next = 1;
        }
        self.invalidated = false;
        self.revoked = false;
    }
    fn search(&self, pointer: usize) -> (Option<usize>, Option<usize>) {
        if self.slots.is_empty() {
            return (None, None);
        }
        let first = collector_primitives::table_hash::slot(pointer as u64, self.slots.len());
        for offset in 0..self.slots.len() {
            let index = (first + offset) & (self.slots.len() - 1);
            let slot = self.slots[index];
            if slot.pointer == pointer && pointer != 0 {
                return (Some(index), None);
            }
            if slot.pointer == 0 {
                return (None, Some(index));
            }
        }
        (None, None)
    }
    fn put(&mut self, slot: Slot) {
        let (_, vacant) = self.search(slot.pointer);
        self.slots[vacant.expect("prepared timer index has a vacant slot")] = slot;
    }
    pub fn prepare(&mut self, capacity: usize) -> io::Result<()> {
        let slots = capacity
            .checked_mul(2)
            .and_then(|n| n.max(2).checked_next_power_of_two())
            .ok_or_else(allocation::oom)?;
        if slots > self.slots.len() {
            let mut next = Self {
                slots: allocation::filled(slots, Slot::default())?,
                ..Self::default()
            };
            for slot in self.slots.iter().copied().filter(|slot| slot.pointer != 0) {
                next.put(slot);
            }
            std::mem::swap(&mut self.slots, &mut next.slots);
        }
        if capacity > self.countries.len() {
            let additional = capacity - self.countries.len();
            allocation::reserve_exact(&mut self.countries, additional)?;
            self.countries.resize(capacity, Country::default());
        }
        Ok(())
    }
    pub fn install(&mut self, spare: &mut Self) {
        debug_assert!(spare.covers(self));
        spare.slots.fill(Slot::default());
        for slot in self.slots.iter().copied().filter(|slot| slot.pointer != 0) {
            spare.put(slot);
        }
        spare.countries[..self.countries.len()].copy_from_slice(&self.countries);
        spare.generation = self.generation;
        spare.invalidated = self.invalidated;
        spare.revoked = self.revoked;
        spare.next = self.next;
        spare.live = self.live;
        spare.demand = self.demand;
        std::mem::swap(self, spare);
    }
    fn insert(&mut self, pointer: usize, owner: CountryIdentity) -> Result<u32, &'static str> {
        if pointer == 0 {
            return Err("delayed-event certificate has a null object");
        }
        let (found, vacant) = self.search(pointer);
        if let Some(index) = found {
            if self.slots[index].owner != owner {
                return Err("delayed-event native owner certificate differs");
            }
            return Ok(self.slots[index].id);
        }
        self.demand = self.demand.max(self.live.saturating_add(1));
        // Prepared storage maintains an empty suffix for every probe chain.
        // Bursts request growth off-thread rather than filling the table.
        if self.live >= self.slots.len() / 2 {
            return Err("delayed-event pointer index needs prepared storage");
        }
        let vacant = vacant.ok_or("delayed-event pointer index needs prepared storage")?;
        let id =
            u32::try_from(self.next).map_err(|_| "delayed-event compact identity exhausted")?;
        self.slots[vacant] = Slot { pointer, id, owner };
        self.next += 1;
        self.live += 1;
        Ok(id)
    }
    fn remove(&mut self, pointer: usize) -> Option<Slot> {
        let index = self.search(pointer).0?;
        let slot = self.slots[index];
        // Close only this probe chain. Tombstones would accumulate across a
        // campaign and make later callbacks search historical capacity even
        // when every timer has retired. No census or maintenance scan is needed.
        let mask = self.slots.len() - 1;
        let mut hole = index;
        let mut next = (hole + 1) & mask;
        while self.slots[next].pointer != 0 {
            let home = collector_primitives::table_hash::slot(
                self.slots[next].pointer as u64,
                self.slots.len(),
            );
            if (next.wrapping_sub(home) & mask) >= (hole.wrapping_sub(home) & mask) {
                self.slots[hole] = self.slots[next];
                hole = next;
            }
            next = (next + 1) & mask;
        }
        self.slots[hole] = Slot::default();
        self.live -= 1;
        Some(slot)
    }
}

pub(crate) fn step<M: Memory>(
    world: &World<'_, M>,
    memory: &M,
    country: usize,
    count: u32,
    finished: bool,
) -> Result<ScriptFact, &'static str> {
    let mut fact = super::registry::marker(
        world,
        memory,
        Change::DelayedAdvance { count, finished },
        None,
    )?;
    fact.target = world.country_object(country)?;
    Ok(fact)
}
impl Store {
    pub(crate) fn has_delayed(&self, pointer: usize) -> bool {
        self.delayed.search(pointer).0.is_some()
    }
    pub fn prepare_delayed(&mut self, capacity: usize) -> io::Result<()> {
        self.delayed.prepare(capacity)?;
        self.charge_storage();
        Ok(())
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Keep the certified owner, native row and publication boundary separate from the borrowed world."
    )]
    pub(crate) fn delayed_stored<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        hour: i32,
        owner: CountryIdentity,
        row: native::Delayed,
        growing: bool,
    ) -> Result<(), &'static str> {
        self.observation(generation)?;
        if growing && self.delayed.live.saturating_add(1) > self.delayed.capacity() {
            self.prepare_delayed(
                self.delayed
                    .live
                    .saturating_add(1)
                    .max(self.delayed.capacity().saturating_mul(2)),
            )
            .map_err(|_| "delayed-event baseline identity allocation failed")?;
        }
        let id = self.delayed.insert(row.pointer, owner)?;
        let (native_event, token) =
            crate::script_event_observer::definition(memory, base, row.definition)?;
        let fact = ScriptFact {
            source: EventFeedSource::ScriptState,
            actor: super::registry::country_operand(world, row.originator)?,
            target: owner,
            cause: Default::default(),
            token,
            scope: Some(row.scope),
            dispatch_frame: 0,
            dispatch_definition: 0,
            script: EventFeedScript {
                instance: Some(id),
                native_event: Some(native_event),
                change: Some(Change::DelayedStored {
                    remaining: row.remaining,
                }),
                ..Default::default()
            },
        };
        self.script_mode(memory, base, generation, hour, fact, growing)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Retirement forwards the native iteration facts with the existing world and publication certificate."
    )]
    pub(crate) fn delayed_removed<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        pointer: usize,
        iteration: bool,
        visited: bool,
    ) -> Result<(), &'static str> {
        self.observation(generation)?;
        let Some(slot) = self.delayed.remove(pointer) else {
            return if iteration {
                Err("native delayed-event retirement has no owner certificate")
            } else {
                Ok(())
            };
        };
        let mut fact = super::registry::marker(
            world,
            memory,
            Change::DelayedRemoved { iteration, visited },
            Some(slot.id),
        )?;
        fact.target = slot.owner;
        self.script_known(memory, base, generation, world.hour()?, fact)
    }
    pub(crate) fn delayed_baseline(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        state: usize,
        hour: i32,
        country: engine_native::ccountry::Country,
    ) -> Result<(), &'static str> {
        self.observation(generation)?;
        if self.registry_state != Some(state) {
            return Err("delayed-event census world certificate differs");
        }
        let slot = country.canonical_slot as usize;
        if self
            .delayed
            .countries
            .get(slot)
            .is_some_and(|seen| seen.pointer == country.pointer && seen.id == country.id)
        {
            return Ok(());
        }
        if slot >= self.delayed.countries.len() {
            self.prepare_delayed(
                slot.checked_add(1)
                    .ok_or("delayed-event country slot overflow")?,
            )
            .map_err(|_| "delayed-event country baseline allocation failed")?;
        }
        // One attempt per owner certificate, not periodic retries of queues.
        self.delayed.countries[slot] = Country {
            pointer: country.pointer,
            id: country.id,
            ready: false,
        };
        let rejected = self.counts.rejected;
        let array = native::delayed_array(memory, country.pointer)?;
        // Country's pointer/id/vtable came from this existing census. Read
        // only its additional identity word; no second country traversal.
        let instance = country
            .pointer
            .checked_add(engine_native::ccountry::ID_INSTANCE)
            .and_then(|at| engine_native::read::<u32>(memory, at))
            .ok_or("delayed-event country identity word unreadable")?;
        let owner = CountryIdentity {
            reference: country.id,
            id: country.id,
            instance,
        };
        if array.count == 0 {
            let mut marker = super::registry::marker_for(memory, Change::DelayedEmpty, None);
            marker.target = owner;
            self.script_growing(memory, base, generation, hour, marker)?;
        } else {
            let world = World::new(memory, base)?;
            let mut marker = super::registry::marker_for(memory, Change::DelayedBaseline, None);
            marker.target = owner;
            self.script_growing(memory, base, generation, hour, marker)?;
            for ordinal in 0..array.count {
                self.delayed_stored(
                    &world,
                    memory,
                    base,
                    generation,
                    hour,
                    owner,
                    native::delayed_at(memory, base, array, ordinal)?,
                    true,
                )?;
            }
            if native::delayed_array(memory, country.pointer)? != array {
                return Err("delayed-event baseline changed during acquisition");
            }
            let mut marker = super::registry::marker_for(memory, Change::DelayedReady, None);
            marker.target = owner;
            self.script_growing(memory, base, generation, hour, marker)?;
        }
        self.delayed.countries[slot].ready = self.counts.rejected == rejected;
        Ok(())
    }
    pub fn invalidate_delayed_country(&mut self, pointer: usize) {
        for country in &mut self.delayed.countries {
            if country.pointer == pointer {
                *country = Country::default();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delayed_colliding_certificates_survive_wrapped_retirement_and_storage_pressure() {
        let mut index = Index::default();
        index.prepare(4).unwrap();
        index.reset(7);
        let pointers: Vec<_> = (1_usize..)
            .map(|n| n * 16)
            .filter(|pointer| {
                collector_primitives::table_hash::slot(*pointer as u64, index.slots.len())
                    == index.slots.len() - 1
            })
            .take(5)
            .collect();
        let owner = CountryIdentity {
            reference: 1,
            id: 1,
            instance: 9,
        };
        crate::test_support::with_allocations_denied(|| {
            for (n, pointer) in pointers[..4].iter().copied().enumerate() {
                assert_eq!(index.insert(pointer, owner).unwrap(), n as u32 + 1);
            }
            assert_eq!(
                index.insert(pointers[4], owner),
                Err("delayed-event pointer index needs prepared storage")
            );
            assert_eq!(index.demand, 5);
            assert_eq!(index.remove(pointers[1]).unwrap().id, 2);
            assert_eq!(index.remove(pointers[0]).unwrap().id, 1);
            assert_eq!(index.insert(pointers[2], owner).unwrap(), 3);
            assert_eq!(index.insert(pointers[3], owner).unwrap(), 4);
            assert_eq!(index.insert(pointers[4], owner).unwrap(), 5);
            for (n, pointer) in pointers[2..].iter().copied().enumerate() {
                assert_eq!(index.remove(pointer).unwrap().id, n as u32 + 3);
                assert!(index.search(pointer).0.is_none());
            }
            assert_eq!(index.live, 0);
        });
    }
    #[test]
    fn delayed_pointer_certificates_scale_to_five_thousand_without_callback_allocation() {
        let mut index = Index::default();
        index.prepare(5000).unwrap();
        index.reset(7);
        let owner = CountryIdentity {
            reference: 1,
            id: 1,
            instance: 4,
        };
        crate::test_support::with_allocations_denied(|| {
            for n in 0..5000 {
                assert_eq!(
                    index.insert(0x10000 + n * 0xc8, owner).unwrap(),
                    n as u32 + 1
                );
            }
            for n in 0..5000 {
                assert_eq!(
                    index.insert(0x10000 + n * 0xc8, owner).unwrap(),
                    n as u32 + 1
                );
            }
            for n in 0..5000 {
                assert_eq!(index.remove(0x10000 + n * 0xc8).unwrap().id, n as u32 + 1);
            }
            assert_eq!(index.live, 0);
            assert_eq!(index.insert(0x10000, owner).unwrap(), 5001);
        });
        let mut spare = Index::default();
        spare.prepare(10000).unwrap();
        crate::test_support::with_allocations_denied(|| index.install(&mut spare));
        assert_eq!(index.remove(0x10000).unwrap().id, 5001);
        index.reset(8);
        assert_eq!(index.insert(0x10000, owner).unwrap(), 1);
    }
    #[test]
    fn delayed_labels_outlive_a_same_world_loss_and_restart_only_for_a_new_world() {
        let owner = CountryIdentity {
            reference: 3,
            id: 3,
            instance: 0,
        };
        let mut index = Index::default();
        index.prepare(4).unwrap();
        index.reset(1);
        let mut spare = Index::default();
        spare.prepare(8).unwrap();
        let certified = Country {
            pointer: 0x10,
            id: 1,
            ready: true,
        };
        let attempted = Country {
            pointer: 0x20,
            id: 2,
            ready: false,
        };
        crate::test_support::with_allocations_denied(|| {
            assert_eq!(index.insert(0x1000, owner).unwrap(), 1);
            index.countries[..2].copy_from_slice(&[certified, attempted]);
            // An observation overlap retires every certificate. The prepared
            // handoff before the next boundary must keep that distinction.
            index.invalidate();
            index.install(&mut spare);
            index.reset(1);
            assert_eq!(index.live, 0);
            assert!(index.search(0x1000).0.is_none());
            assert!(index.countries.iter().all(|country| country.pointer == 0));
            // The game may free the first timer unobserved and reuse its
            // address for another one: that timer must get a new label.
            assert_eq!(index.insert(0x1000, owner).unwrap(), 2);
            assert_eq!(index.insert(0x2000, owner).unwrap(), 3);
            // A boundary without a loss carries the certificates.
            index.reset(1);
            assert_eq!(index.insert(0x1000, owner).unwrap(), 2);
            // A revocation retires timers and certified owners, keeps failed
            // owner attempts, and keeps the labels too.
            index.countries[..2].copy_from_slice(&[certified, attempted]);
            index.revoke_retained();
            index.reset(1);
            assert_eq!(index.countries[0].pointer, 0);
            assert_eq!(index.countries[1].pointer, attempted.pointer);
            assert_eq!(index.insert(0x2000, owner).unwrap(), 4);
            index.reset(2);
            assert_eq!(index.live, 0);
            assert!(index.countries.iter().all(|country| country.pointer == 0));
            assert_eq!(index.insert(0x2000, owner).unwrap(), 1);
        });
    }
    #[test]
    fn delayed_label_exhaustion_stays_checked_across_a_same_world_loss() {
        let owner = CountryIdentity {
            reference: 1,
            id: 1,
            instance: 4,
        };
        let mut index = Index::default();
        index.prepare(4).unwrap();
        index.reset(7);
        index.next = u64::from(u32::MAX);
        crate::test_support::with_allocations_denied(|| {
            assert_eq!(index.insert(0x1000, owner).unwrap(), u32::MAX);
            let exhausted = Err("delayed-event compact identity exhausted");
            assert_eq!(index.insert(0x2000, owner), exhausted);
            index.invalidate();
            index.reset(7);
            assert_eq!(index.insert(0x2000, owner), exhausted);
            index.reset(8);
            assert_eq!(index.insert(0x2000, owner).unwrap(), 1);
        });
    }
}
