use super::*;
use engine_events::event_feed_sites::CountryIdentity;
use engine_events::{event_feed_sites::provenance::World, script_event_registry as native};
use replay_data::event_feed_rows::EventFeedScriptChange as Change;

#[derive(Clone, Copy, Default)]
pub(super) struct Slot {
    pub pointer: usize,
    pub id: u32,
    pub country: u32,
}
#[derive(Clone, Copy, Default)]
pub(super) struct Country {
    pub pointer: usize,
    pub id: u32,
    pub ready: bool,
    pub refused: bool,
}
#[derive(Default)]
pub(super) struct Index {
    slots: Vec<Slot>,
    pub(super) countries: Vec<Country>,
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
    fn needs_room(&self) -> bool {
        self.live >= self.slots.len() / 2
    }
    pub fn covers(&self, other: &Self) -> bool {
        self.slots.len() >= other.slots.len() && self.countries.len() >= other.countries.len()
    }
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }
    pub fn revoke_retained(&mut self) {
        self.revoked = true;
    }
    pub fn reset(&mut self, generation: u64) {
        let world = self.generation != Some(generation);
        if world || self.invalidated || self.revoked {
            super::recovery_needed();
        }
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
    fn insert(&mut self, pointer: usize, country: u32) -> Result<u32, &'static str> {
        if pointer == 0 {
            return Err("delayed-event certificate has a null object");
        }
        let (found, vacant) = self.search(pointer);
        if let Some(index) = found {
            if self.slots[index].country != country {
                return Err("delayed-event native owner certificate differs");
            }
            return Ok(self.slots[index].id);
        }
        self.demand = self.demand.max(self.live.saturating_add(1));
        if self.live >= self.slots.len() / 2 {
            return Err("delayed-event pointer index needs prepared storage");
        }
        let vacant = vacant.ok_or("delayed-event pointer index needs prepared storage")?;
        let id =
            u32::try_from(self.next).map_err(|_| "delayed-event compact identity exhausted")?;
        self.slots[vacant] = Slot {
            pointer,
            id,
            country,
        };
        self.next += 1;
        self.live += 1;
        Ok(id)
    }
    fn remove(&mut self, pointer: usize) -> Option<Slot> {
        let index = self.search(pointer).0?;
        let slot = self.slots[index];
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
    pub(super) fn seed_prepared_delayed(&mut self, generation: u64) {
        self.delayed.generation = Some(generation);
        self.delayed.next = 1;
    }
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
    ) -> Result<(), &'static str> {
        self.observation(generation)?;
        if self.delayed.needs_room() {
            self.prepare_delayed(
                self.delayed
                    .live
                    .saturating_add(1)
                    .max(self.delayed.slots.len()),
            )
            .map_err(|_| "delayed-event identity allocation failed")?;
        }
        let id = self.delayed.insert(row.pointer, owner.id)?;
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
        self.script_known(memory, base, generation, hour, fact)
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
        pass: Option<(usize, u64)>,
        visited: bool,
    ) -> Result<(), &'static str> {
        self.observation(generation)?;
        let Some(slot) = self.delayed.remove(pointer) else {
            let Some((owner, anchored)) = pass else {
                return Ok(());
            };
            if anchored != generation {
                return Err(UNCERTIFIED_RETIREMENT);
            }
            let (record, identity) = world.country_record(owner)?;
            let slot = record.canonical_slot as usize;
            let entry = self.delayed.countries.get_mut(slot);
            match entry {
                Some(entry) if entry.pointer == owner && entry.id == identity.id => {
                    if entry.ready {
                        return Err(CERTIFIED_RETIREMENT_MISSING);
                    }
                }
                Some(entry) => {
                    *entry = Country {
                        pointer: owner,
                        id: identity.id,
                        ..Country::default()
                    };
                }
                None => {}
            }
            let hour = world.hour()?;
            return self.refuse_delayed_owner(
                memory,
                base,
                generation,
                hour,
                identity,
                Some(slot),
                UNCERTIFIED_RETIREMENT,
                true,
            );
        };
        let mut fact = super::registry::marker(
            world,
            memory,
            Change::DelayedRemoved {
                iteration: pass.is_some(),
                visited,
            },
            Some(slot.id),
        )?;
        fact.target = world.country(slot.country)?;
        if fact.target.id != slot.country {
            return Err("delayed-event native owner certificate differs");
        }
        self.script_known(memory, base, generation, world.hour()?, fact)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "The refused owner, its census slot and the reason travel with the publication operands."
    )]
    pub(crate) fn refuse_delayed_owner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        owner: CountryIdentity,
        slot: Option<usize>,
        reason: &'static str,
        count: bool,
    ) -> Result<(), &'static str> {
        let entry = slot
            .and_then(|slot| self.delayed.countries.get_mut(slot))
            .filter(|entry| entry.id == owner.id && entry.pointer != 0);
        super::recovery_needed();
        match entry {
            Some(entry) => {
                entry.ready = false;
                if entry.refused {
                    super::repeated_owner_refusal(super::OwnerRegistry::Delayed);
                    return Ok(());
                }
                entry.refused = true;
            }
            None => {}
        }
        if count {
            let _ = self.advance();
            self.count_rejected(1);
        }
        let mut marker = super::registry::marker_for(memory, Change::DelayedRefused, None);
        marker.target = owner;
        self.script_text(memory, base, generation, hour, marker, reason)
    }
    pub(crate) fn recover_delayed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        state: usize,
        hour: i32,
        country: engine_native_memory::country_identity::Country,
    ) {
        if self.failure.is_some() || self.observation(generation).is_err() {
            return;
        }
        let slot = country.canonical_slot as usize;
        let seen = self
            .delayed
            .countries
            .get(slot)
            .filter(|seen| seen.pointer == country.pointer && seen.id == country.id)
            .copied();
        if seen.is_some_and(|seen| seen.ready) {
            return;
        }
        let recorded = slot < self.delayed.countries.len()
            || self.prepare_delayed(slot.saturating_add(1)).is_ok();
        if recorded {
            self.delayed.countries[slot] = Country {
                pointer: country.pointer,
                id: country.id,
                ready: false,
                refused: seen.is_some_and(|seen| seen.refused),
            };
        }
        let identity = || {
            let instance = country
                .pointer
                .checked_add(engine_native::ccountry::SURRENDER_TARGET)
                .and_then(|at| engine_native_memory::read::<u32>(memory, at))
                .ok_or("delayed-event country surrender target unreadable")?;
            Ok::<_, &'static str>(CountryIdentity {
                reference: country.id,
                id: country.id,
                instance,
            })
        };
        let mut named = None;
        let result = self.owner_scope(super::OwnerRegistry::Delayed, |store| {
            let cut = store.sample()?;
            let owner = identity()?;
            named = Some(owner);
            if !recorded {
                store.delayed.demand = store.delayed.demand.max(slot.saturating_add(1));
                return Err("delayed-event census table allocation failed");
            }
            store.delayed_owner_baseline(memory, base, generation, state, hour, country, owner, cut)
        });
        let refusal = match result {
            Ok(()) => {
                let entry = &mut self.delayed.countries[slot];
                entry.ready = true;
                entry.refused = false;
                return;
            }
            Err(refusal) => refusal,
        };
        self.owner_refusals[super::OwnerRegistry::Delayed as usize] = Some(refusal);
        if self.preparing_owner {
            return;
        }
        if named.is_none() && refusal.retry {
            named = identity().ok();
        }
        match named {
            Some(owner) => {
                let _ = self.refuse_delayed_owner(
                    memory,
                    base,
                    generation,
                    hour,
                    owner,
                    recorded.then_some(slot),
                    refusal.reason,
                    !refusal.counted,
                );
            }
            None if refusal.retry => {}
            None => {
                self.reject(refusal.reason);
            }
        }
        if refusal.retry {
            super::recovery_needed();
        }
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "The census owner's identity is read once by its visit."
    )]
    fn delayed_owner_baseline(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        state: usize,
        hour: i32,
        country: engine_native_memory::country_identity::Country,
        owner: CountryIdentity,
        cut: super::Cut,
    ) -> Result<(), &'static str> {
        if self.registry_state != Some(state) {
            return Err("delayed-event census world certificate differs");
        }
        let array = native::delayed_array(memory, country.pointer)?;
        if array.count == 0 {
            Self::certifies(cut)?;
            let mut marker = super::registry::marker_for(memory, Change::DelayedEmpty, None);
            marker.target = owner;
            return self.script_known(memory, base, generation, hour, marker);
        }
        let world = World::new(memory, base)?;
        let mut marker = super::registry::marker_for(memory, Change::DelayedBaseline, None);
        marker.target = owner;
        self.script_known(memory, base, generation, hour, marker)?;
        for ordinal in 0..array.count {
            self.delayed_stored(
                &world,
                memory,
                base,
                generation,
                hour,
                owner,
                native::delayed_at(memory, base, array, ordinal)?,
            )?;
        }
        if native::delayed_array(memory, country.pointer)? != array {
            return Err("delayed-event baseline changed during acquisition");
        }
        Self::certifies(cut)?;
        let mut marker = super::registry::marker_for(memory, Change::DelayedReady, None);
        marker.target = owner;
        self.script_known(memory, base, generation, hour, marker)
    }
    pub fn invalidate_delayed_country(&mut self, pointer: usize) {
        for country in &mut self.delayed.countries {
            if country.pointer == pointer {
                *country = Country::default();
            }
        }
        super::recovery_needed();
    }
    pub(super) fn prepared_delayed_demand(
        &self,
        source: &Self,
        country: engine_native_memory::country_identity::Country,
        completed: bool,
    ) -> usize {
        self.delayed
            .live
            .saturating_add(if completed { source.delayed.live } else { 0 })
            .max((country.canonical_slot as usize).saturating_add(1))
    }
    pub(super) fn prepared_delayed_labels(&self) -> usize {
        self.delayed.next as usize
    }
    pub(super) fn merge_delayed_owner(
        &mut self,
        source: &Self,
        country: engine_native_memory::country_identity::Country,
        labels: &mut [u32],
    ) -> Result<(), &'static str> {
        for slot in source.delayed.slots.iter().filter(|slot| slot.pointer != 0) {
            labels[slot.id as usize] = self.delayed.insert(slot.pointer, slot.country)?;
        }
        self.delayed.countries[country.canonical_slot as usize] = source.delayed.countries[0];
        Ok(())
    }
    pub(super) fn prepared_delayed_failure(
        &mut self,
        country: engine_native_memory::country_identity::Country,
    ) -> bool {
        if let Some(entry) = self
            .delayed
            .countries
            .get_mut(country.canonical_slot as usize)
        {
            let refused =
                entry.refused && entry.pointer == country.pointer && entry.id == country.id;
            *entry = Country {
                pointer: country.pointer,
                id: country.id,
                refused,
                ..Default::default()
            };
            return refused;
        }
        false
    }
}

pub(crate) const UNCERTIFIED_RETIREMENT: &str =
    "native delayed-event retirement has no owner certificate";
pub(crate) const CERTIFIED_RETIREMENT_MISSING: &str =
    "native delayed-event retirement is missing from its certified owner queue";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delayed_colliding_certificates_survive_wrapped_retirement_and_storage_pressure() {
        let _globals = crate::event_feed_observer::test_globals();
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
                assert_eq!(index.insert(pointer, owner.id).unwrap(), n as u32 + 1);
            }
            assert_eq!(
                index.insert(pointers[4], owner.id),
                Err("delayed-event pointer index needs prepared storage")
            );
            assert_eq!(index.demand, 5);
            assert_eq!(index.remove(pointers[1]).unwrap().id, 2);
            assert_eq!(index.remove(pointers[0]).unwrap().id, 1);
            assert_eq!(index.insert(pointers[2], owner.id).unwrap(), 3);
            assert_eq!(index.insert(pointers[3], owner.id).unwrap(), 4);
            assert_eq!(index.insert(pointers[4], owner.id).unwrap(), 5);
            for (n, pointer) in pointers[2..].iter().copied().enumerate() {
                assert_eq!(index.remove(pointer).unwrap().id, n as u32 + 3);
                assert!(index.search(pointer).0.is_none());
            }
            assert_eq!(index.live, 0);
        });
    }
    #[test]
    fn delayed_pointer_certificates_need_no_callback_allocation() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(20).unwrap();
        index.reset(7);
        let owner = CountryIdentity {
            reference: 1,
            id: 1,
            instance: 4,
        };
        crate::test_support::with_allocations_denied(|| {
            for n in 0..20 {
                assert_eq!(
                    index.insert(0x10000 + n * 0xc8, owner.id).unwrap(),
                    n as u32 + 1
                );
            }
            for n in 0..20 {
                assert_eq!(
                    index.insert(0x10000 + n * 0xc8, owner.id).unwrap(),
                    n as u32 + 1
                );
            }
            for n in 0..20 {
                assert_eq!(index.remove(0x10000 + n * 0xc8).unwrap().id, n as u32 + 1);
            }
            assert_eq!(index.live, 0);
            assert_eq!(index.insert(0x10000, owner.id).unwrap(), 21);
        });
        let mut spare = Index::default();
        spare.prepare(40).unwrap();
        crate::test_support::with_allocations_denied(|| index.install(&mut spare));
        assert_eq!(index.remove(0x10000).unwrap().id, 21);
        index.reset(8);
        assert_eq!(index.insert(0x10000, owner.id).unwrap(), 1);
    }
    #[test]
    fn delayed_labels_outlive_a_same_world_loss_and_restart_only_for_a_new_world() {
        let _globals = crate::event_feed_observer::test_globals();
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
            refused: false,
        };
        let attempted = Country {
            pointer: 0x20,
            id: 2,
            ready: false,
            refused: true,
        };
        crate::test_support::with_allocations_denied(|| {
            assert_eq!(index.insert(0x1000, owner.id).unwrap(), 1);
            index.countries[..2].copy_from_slice(&[certified, attempted]);
            index.invalidate();
            index.install(&mut spare);
            index.reset(1);
            assert_eq!(index.live, 0);
            assert!(index.search(0x1000).0.is_none());
            assert!(index.countries.iter().all(|country| country.pointer == 0));
            assert_eq!(index.insert(0x1000, owner.id).unwrap(), 2);
            assert_eq!(index.insert(0x2000, owner.id).unwrap(), 3);
            index.reset(1);
            assert_eq!(index.insert(0x1000, owner.id).unwrap(), 2);
            index.countries[..2].copy_from_slice(&[certified, attempted]);
            index.revoke_retained();
            index.reset(1);
            assert_eq!(index.countries[0].pointer, 0);
            assert_eq!(index.countries[1].pointer, attempted.pointer);
            assert!(!index.countries[1].ready && index.countries[1].refused);
            assert_eq!(index.insert(0x2000, owner.id).unwrap(), 4);
            index.reset(2);
            assert_eq!(index.live, 0);
            assert!(index.countries.iter().all(|country| country.pointer == 0));
            assert_eq!(index.insert(0x2000, owner.id).unwrap(), 1);
        });
    }
    #[test]
    fn delayed_label_exhaustion_stays_checked_across_a_same_world_loss() {
        let _globals = crate::event_feed_observer::test_globals();
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
            assert_eq!(index.insert(0x1000, owner.id).unwrap(), u32::MAX);
            let exhausted = Err("delayed-event compact identity exhausted");
            assert_eq!(index.insert(0x2000, owner.id), exhausted);
            index.invalidate();
            index.reset(7);
            assert_eq!(index.insert(0x2000, owner.id), exhausted);
            index.reset(8);
            assert_eq!(index.insert(0x2000, owner.id).unwrap(), 1);
        });
    }
    #[test]
    fn surrender_target_changes_keep_timer_labels_but_country_reuse_does_not() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(1500).unwrap();
        index.reset(1);
        let owner = CountryIdentity {
            reference: 24,
            id: 24,
            instance: 0,
        };
        let label = index.insert(100, owner.id).unwrap();
        let changed = CountryIdentity {
            reference: 500,
            instance: 1,
            ..owner
        };
        assert_eq!(index.insert(100, changed.id).unwrap(), label);
        let reused = CountryIdentity {
            reference: 1501,
            id: 1501,
            ..changed
        };
        assert_eq!(
            index.insert(100, reused.id),
            Err("delayed-event native owner certificate differs")
        );
        index.remove(100).unwrap();
        assert_ne!(index.insert(100, reused.id).unwrap(), label);
    }
}
