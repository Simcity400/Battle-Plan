#![allow(clippy::too_many_arguments)]
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
pub(crate) static POWER_EPOCH: AtomicU64 = AtomicU64::new(1);
use engine_decisions::decision_runtime as native;
use engine_events::event_feed_sites::{CountryIdentity, provenance::World};
use replay_data::event_feed_rows::{
    EventFeedScriptChange as Change,
    decisions::{DecisionChange, Operation},
};
#[derive(Clone, Copy, Default)]
pub(crate) struct Slot {
    pub pointer: usize,
    pub id: u32,
    pub owner: usize,
    pub container: u8,
    pub epoch: u64,
    pub previous: usize,
    pub next: usize,
}
#[derive(Clone, Copy, Default)]
struct Owner {
    pointer: usize,
    identity: CountryIdentity,
    epoch: u64,
    certified: Option<u64>,
    refused: bool,
    head: usize,
    power: Option<i64>,
    power_epoch: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recovery {
    Certified,
    Baseline,
}
#[derive(Default)]
pub(crate) struct Index {
    slots: Vec<Slot>,
    owners: Vec<Owner>,
    generation: Option<u64>,
    invalidated: bool,
    revoked: u64,
    next: u64,
    live: usize,
    owner_count: usize,
    pub demand: usize,
    pub owner_demand: usize,
}
impl Index {
    fn locate<T>(values: &[T], pointer: usize, key: impl Fn(&T) -> usize) -> Result<usize, usize> {
        if values.is_empty() {
            return Err(0);
        }
        let first = collector_primitives::table_hash::slot(pointer as u64, values.len());
        for n in 0..values.len() {
            let at = (first + n) & (values.len() - 1);
            let found = key(&values[at]);
            if found == pointer && pointer != 0 {
                return Ok(at);
            }
            if found == 0 {
                return Err(at);
            }
        }
        Err(values.len())
    }
    pub fn capacity(&self) -> usize {
        self.slots.len() / 2
    }
    pub fn owner_capacity(&self) -> usize {
        self.owners.len() / 2
    }
    fn needs_room(&self) -> bool {
        self.live >= self.capacity() || self.owner_count >= self.owners.len() / 2
    }
    pub fn bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.slots) + crate::raw_memory::vector_bytes(&self.owners)
    }
    pub fn covers(&self, other: &Self) -> bool {
        self.slots.len() >= other.slots.len() && self.owners.len() >= other.owners.len()
    }
    pub fn prepare(&mut self, capacity: usize) -> io::Result<()> {
        self.prepare_sized(capacity, capacity)
    }
    pub fn prepare_sized(&mut self, slot_capacity: usize, owner_capacity: usize) -> io::Result<()> {
        let slot_size = slot_capacity
            .checked_mul(2)
            .and_then(|n| n.max(2).checked_next_power_of_two())
            .ok_or_else(allocation::oom)?;
        if slot_size > self.slots.len() {
            let mut slots = allocation::filled(slot_size, Slot::default())?;
            for value in &self.slots {
                if value.pointer != 0 {
                    let at = Self::locate(&slots, value.pointer, |s| s.pointer).unwrap_err();
                    slots[at] = *value;
                }
            }
            self.slots = slots;
        }
        let owner_size = owner_capacity
            .checked_mul(2)
            .and_then(|n| n.max(2).checked_next_power_of_two())
            .ok_or_else(allocation::oom)?;
        if owner_size > self.owners.len() {
            let mut owners = allocation::filled(owner_size, Owner::default())?;
            for value in &self.owners {
                if value.pointer != 0 {
                    let at = Self::locate(&owners, value.pointer, |s| s.pointer).unwrap_err();
                    owners[at] = *value;
                }
            }
            self.owners = owners;
        }
        Ok(())
    }
    pub fn install(&mut self, spare: &mut Self) {
        spare.slots.fill(Slot::default());
        spare.owners.fill(Owner::default());
        for value in &self.slots {
            if value.pointer != 0 {
                let at = Self::locate(&spare.slots, value.pointer, |s| s.pointer).unwrap_err();
                spare.slots[at] = *value;
            }
        }
        for value in &self.owners {
            if value.pointer != 0 {
                let at = Self::locate(&spare.owners, value.pointer, |s| s.pointer).unwrap_err();
                spare.owners[at] = *value;
            }
        }
        spare.generation = self.generation;
        spare.invalidated = self.invalidated;
        spare.revoked = self.revoked;
        spare.next = self.next;
        spare.live = self.live;
        spare.owner_count = self.owner_count;
        spare.demand = self.demand;
        spare.owner_demand = self.owner_demand;
        std::mem::swap(self, spare);
    }
    pub fn invalidate(&mut self) {
        self.invalidated = true;
        super::recovery_needed();
    }
    pub fn revoke(&mut self) {
        self.revoked = self.revoked.wrapping_add(1);
        super::recovery_needed();
    }
    pub fn matches(&self, generation: u64) -> bool {
        self.generation == Some(generation) && !self.invalidated
    }
    pub fn reset(&mut self, generation: u64) {
        let world = self.generation != Some(generation);
        if world || self.invalidated {
            self.slots.fill(Slot::default());
            self.owners.fill(Owner::default());
            self.live = 0;
            self.owner_count = 0;
            if world {
                self.generation = Some(generation);
                self.next = 1;
            }
            self.invalidated = false;
            super::recovery_needed();
        }
    }
    fn is_certified(&self, owner: &Owner) -> bool {
        owner.certified == Some(self.revoked)
    }
    pub fn certified(&self, pointer: usize) -> bool {
        Self::locate(&self.owners, pointer, |s| s.pointer)
            .is_ok_and(|at| self.is_certified(&self.owners[at]))
    }
    pub(super) fn census(&mut self, country: engine_native_memory::country_identity::Country) {
        let Ok(at) = Self::locate(&self.owners, country.pointer, |owner| owner.pointer) else {
            return;
        };
        let owner = &mut self.owners[at];
        if owner.identity.id != country.id {
            owner.certified = None;
            owner.refused = false;
            owner.power = None;
            owner.power_epoch = 0;
        }
    }
    pub fn recovery(&self, pointer: usize) -> Recovery {
        match Self::locate(&self.owners, pointer, |s| s.pointer) {
            Ok(at) if self.is_certified(&self.owners[at]) => Recovery::Certified,
            _ => Recovery::Baseline,
        }
    }
    pub(super) fn certify(
        &mut self,
        pointer: usize,
        identity: CountryIdentity,
    ) -> Result<(), &'static str> {
        let revoked = self.revoked;
        let owner = self.owner(pointer, identity)?;
        owner.certified = Some(revoked);
        owner.refused = false;
        Ok(())
    }
    fn refuse(&mut self, pointer: usize) -> Option<bool> {
        let Ok(at) = Self::locate(&self.owners, pointer, |s| s.pointer) else {
            return Some(true);
        };
        let owner = &mut self.owners[at];
        owner.certified = None;
        if std::mem::replace(&mut owner.refused, true) {
            return None;
        }
        Some(true)
    }
    fn owner(
        &mut self,
        pointer: usize,
        identity: CountryIdentity,
    ) -> Result<&mut Owner, &'static str> {
        let at = match Self::locate(&self.owners, pointer, |s| s.pointer) {
            Ok(at) => at,
            Err(at) if at < self.owners.len() && self.owner_count < self.owners.len() / 2 => {
                self.owners[at] = Owner {
                    pointer,
                    identity,
                    epoch: 1,
                    certified: None,
                    refused: false,
                    head: 0,
                    power: None,
                    power_epoch: 0,
                };
                self.owner_count += 1;
                self.owner_demand = self.owner_demand.max(self.owner_count);
                at
            }
            _ => {
                self.owner_demand = self
                    .owner_demand
                    .max(self.owner_capacity().saturating_add(1));
                return Err("decision owner certificates need prepared capacity");
            }
        };
        if !self.owners[at].identity.same_country(identity) {
            self.owners[at] = Owner {
                pointer,
                identity,
                epoch: self.owners[at].epoch.wrapping_add(1),
                certified: None,
                refused: false,
                head: self.owners[at].head,
                power: None,
                power_epoch: 0,
            };
        }
        self.owners[at].identity = identity;
        Ok(&mut self.owners[at])
    }
    pub fn find(&self, pointer: usize) -> Option<Slot> {
        let slot = self.slot(pointer)?;
        let owner = &self.owners[Self::locate(&self.owners, slot.owner, |s| s.pointer).ok()?];
        (owner.epoch == slot.epoch && self.is_certified(owner)).then_some(slot)
    }
    pub fn slot(&self, pointer: usize) -> Option<Slot> {
        if self.generation.is_none() || self.invalidated {
            return None;
        }
        let at = Self::locate(&self.slots, pointer, |s| s.pointer).ok()?;
        Some(self.slots[at])
    }
    pub fn uncertified(&self, pointer: usize) -> Option<Slot> {
        let slot = self.slot(pointer)?;
        let owner = &self.owners[Self::locate(&self.owners, slot.owner, |s| s.pointer).ok()?];
        (owner.epoch == slot.epoch && !self.is_certified(owner)).then_some(slot)
    }
    fn put(
        &mut self,
        pointer: usize,
        owner: usize,
        identity: CountryIdentity,
        container: u8,
    ) -> Result<(Slot, bool), &'static str> {
        let epoch = self.owner(owner, identity)?.epoch;
        if let Ok(at) = Self::locate(&self.slots, pointer, |s| s.pointer) {
            let slot = self.slots[at];
            if slot.owner == owner && slot.epoch == epoch {
                return Ok((slot, false));
            }
            self.remove(pointer);
        }
        self.demand = self.demand.max(self.live.saturating_add(1));
        if self.live >= self.capacity() {
            return Err("decision pointer certificates need prepared capacity");
        }
        let at = Self::locate(&self.slots, pointer, |s| s.pointer).unwrap_or_else(|at| at);
        if at >= self.slots.len() {
            return Err("decision pointer certificates are full");
        }
        let id = u32::try_from(self.next).map_err(|_| "decision compact identity exhausted")?;
        self.next += 1;
        let head = self.owner(owner, identity)?.head;
        let slot = Slot {
            pointer,
            id,
            owner,
            container,
            epoch,
            previous: 0,
            next: head,
        };
        if head != 0 {
            let next = Self::locate(&self.slots, head, |s| s.pointer)
                .map_err(|_| "decision certificate chain differs")?;
            self.slots[next].previous = pointer;
        }
        self.owner(owner, identity)?.head = pointer;
        if self.slots[at].pointer == 0 {
            self.live += 1;
        }
        self.slots[at] = slot;
        Ok((slot, true))
    }
    fn remove(&mut self, pointer: usize) -> Option<Slot> {
        let at = Self::locate(&self.slots, pointer, |s| s.pointer).ok()?;
        let old = self.slots[at];
        if old.previous == 0 {
            if let Ok(owner) = Self::locate(&self.owners, old.owner, |s| s.pointer) {
                self.owners[owner].head = old.next;
            }
        } else if let Ok(previous) = Self::locate(&self.slots, old.previous, |s| s.pointer) {
            self.slots[previous].next = old.next;
        }
        if old.next != 0
            && let Ok(next) = Self::locate(&self.slots, old.next, |s| s.pointer)
        {
            self.slots[next].previous = old.previous;
        }
        let mask = self.slots.len() - 1;
        let mut hole = at;
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
        Some(old)
    }
    fn mark(&mut self, pointer: usize, identity: CountryIdentity) -> Result<bool, &'static str> {
        let revoked = self.revoked;
        let owner = self.owner(pointer, identity)?;
        owner.power = None;
        owner.power_epoch = 0;
        owner.refused = false;
        owner.epoch = owner
            .epoch
            .checked_add(1)
            .ok_or("decision owner epoch exhausted")?;
        let certified = owner.certified.take() == Some(revoked);
        super::recovery_needed();
        Ok(certified)
    }
    fn clear_owner(
        &mut self,
        pointer: usize,
        identity: CountryIdentity,
    ) -> Result<(), &'static str> {
        loop {
            let head = self.owner(pointer, identity)?.head;
            if head == 0 {
                break;
            }
            self.remove(head)
                .ok_or("decision certificate chain missing")?;
        }
        Ok(())
    }
    pub fn power_update(
        &mut self,
        pointer: usize,
        reference: u32,
        _surrender_target: u32,
        before: i64,
        after: i64,
        epoch: u64,
    ) {
        let Ok(at) = Self::locate(&self.owners, pointer, |s| s.pointer) else {
            return;
        };
        let owner = &mut self.owners[at];
        if owner.identity.id != reference {
            return;
        }
        if owner.power_epoch == epoch && owner.power == Some(before) {
            owner.power = Some(after);
        } else {
            owner.power = None;
            owner.power_epoch = 0;
        }
    }
    fn power(&self, pointer: usize, epoch: u64) -> Option<i64> {
        let at = Self::locate(&self.owners, pointer, |s| s.pointer).ok()?;
        let owner = &self.owners[at];
        (self.is_certified(owner) && owner.power_epoch == epoch)
            .then_some(owner.power)
            .flatten()
    }
    fn power_baseline(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
    ) -> Result<(), &'static str> {
        let at = Self::locate(&self.owners, pointer, |s| s.pointer)
            .map_err(|_| "political balance has no demanded decision owner")?;
        let epoch = POWER_EPOCH.load(Ordering::Acquire);
        if self.owners[at].power_epoch == epoch {
            return Ok(());
        }
        let native_owner = engine_events::politics_sites::country_owner(memory, base, pointer)?;
        let power = native_owner
            .politics
            .map(|status| {
                engine_native_memory::read::<i64>(
                    memory,
                    status + engine_native_parts::cpolitical_status_layout::POLITICAL_POWER,
                )
                .ok_or("decision political balance baseline unreadable")
            })
            .transpose()?;
        self.owners[at].power = power;
        self.owners[at].power_epoch = epoch;
        Ok(())
    }
    fn fresh_id(&mut self) -> Result<u32, &'static str> {
        let id = u32::try_from(self.next).map_err(|_| "decision compact identity exhausted")?;
        self.next += 1;
        Ok(id)
    }
}
fn fact(
    memory: &impl Memory,
    owner: CountryIdentity,
    change: DecisionChange,
    instance: Option<u32>,
) -> ScriptFact {
    let mut fact = super::registry::marker_for(memory, Change::Decision(change), instance);
    fact.actor = owner;
    fact
}
impl Store {
    pub(super) fn seed_prepared_decisions(&mut self, generation: u64) {
        self.decisions.generation = Some(generation);
        self.decisions.next = 1;
    }
    pub fn prepare_decisions(&mut self, capacity: usize) -> io::Result<()> {
        self.decisions.prepare(capacity)?;
        self.prepare_staged(capacity)?;
        self.charge_storage();
        Ok(())
    }
    pub(crate) fn decision_marker<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        country: usize,
        mut change: DecisionChange,
    ) -> Result<(), &'static str> {
        if change.operation == Operation::Rebuild && change.container != 3 {
            return Ok(());
        }
        self.observation(generation)?;

        let identity = world.country_object(country)?;
        self.decisions.owner(country, identity)?;
        if change.operation == Operation::Invalidate && !self.decisions.mark(country, identity)? {
            return Ok(());
        }
        if change.operation == Operation::Rebuild {
            change.political_power = self
                .decisions
                .power(country, POWER_EPOCH.load(Ordering::Acquire));
        }
        let mut value = fact(memory, identity, change, None);
        if change.operation == Operation::Rebuild {
            value.script.days = Some(
                engine_native_memory::read::<i32>(
                    memory,
                    world.state_pointer() + engine_native::cgame_state::CAMPAIGN_SEED,
                )
                .ok_or("decision rebuild campaign seed unreadable")?,
            );
            value.script.lifecycle = Some(i32::from(
                engine_sites::at(base, native::ADMISSION_OVERRIDE_RVA)
                    .and_then(|at| engine_native_memory::read::<u8>(memory, at))
                    .ok_or("decision rebuild admission override unreadable")?
                    != 0,
            ));
        }
        self.script_known(memory, base, generation, world.hour()?, value)
    }
    pub(crate) fn decision_selected<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        country: usize,
        pointer: usize,
        change: DecisionChange,
        baseline: bool,
        results: (Option<i32>, Option<i32>),
    ) -> Result<(), &'static str> {
        self.decision_selected_source(
            world, memory, base, generation, country, pointer, change, baseline, results, None,
        )
    }
    pub(crate) fn decision_selected_source<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        country: usize,
        pointer: usize,
        mut change: DecisionChange,
        baseline: bool,
        results: (Option<i32>, Option<i32>),
        source: Option<EventFeedSource>,
    ) -> Result<(), &'static str> {
        self.observation(generation)?;

        let owner = world.country_object(country)?;
        let supplied_update = change.operation == Operation::Update;
        let old = self.decisions.find(pointer);
        let row = native::record(memory, base, pointer, change.container)?;
        if change.container != 0 && self.decisions.needs_room() {
            self.prepare_decisions(
                (self.decisions.live + 1)
                    .max(self.decisions.owner_count + 1)
                    .max(self.decisions.capacity().saturating_mul(2)),
            )
            .map_err(|_| "decision certificate allocation failed")?;
        }
        let (mut slot, new) = if change.container == 0 {
            (
                Slot {
                    pointer,
                    id: self.decisions.fresh_id()?,
                    owner: country,
                    container: 0,
                    ..Default::default()
                },
                true,
            )
        } else {
            self.decisions
                .put(pointer, country, owner, change.container)?
        };
        if change.operation == Operation::Store && !new && change.container != 0 {
            if slot.container != change.container {
                if old.is_some() {
                    change.flags |= replay_data::event_feed_rows::decisions::MOVED;
                }
                let at = Index::locate(&self.decisions.slots, pointer, |s| s.pointer).unwrap();
                self.decisions.slots[at].container = change.container;
                slot.container = change.container;
            } else if old.is_some() {
                change.operation = Operation::Update;
            }
        }
        if change.operation != Operation::Store
            && change.container != 0
            && old.is_none()
            && !baseline
        {
            return Err("decision writer has no retained instance certificate");
        }
        let value = record_fact(
            world,
            memory,
            owner,
            change,
            slot.id,
            row,
            results,
            supplied_update,
            source,
        )?;
        self.script_known(memory, base, generation, world.hour()?, value)
    }
    pub(crate) fn decision_result<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        pointer: usize,
        results: (Option<i32>, Option<i32>),
        source: EventFeedSource,
        reason: &'static str,
    ) -> Result<bool, &'static str> {
        if let Some(slot) = self.decisions.find(pointer) {
            self.decision_selected_source(
                world,
                memory,
                base,
                generation,
                slot.owner,
                pointer,
                DecisionChange::new(Operation::Update, slot.container),
                false,
                results,
                Some(source),
            )?;
            return Ok(true);
        }
        let Some(slot) = self.decisions.uncertified(pointer) else {
            return Ok(false);
        };
        self.observation(generation)?;
        let owner = world.country_object(slot.owner)?;
        let hour = world.hour()?;
        let row = native::record(memory, base, pointer, slot.container)?;
        let value = record_fact(
            world,
            memory,
            owner,
            DecisionChange::new(Operation::Update, slot.container),
            slot.id,
            row,
            results,
            true,
            Some(source),
        )?;
        self.refuse_decision_owner(
            memory, base, generation, hour, slot.owner, owner, reason, true,
        )?;
        self.script_known(memory, base, generation, hour, value)?;
        Ok(true)
    }
    pub(crate) fn refuse_decision_owner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        pointer: usize,
        owner: CountryIdentity,
        reason: &'static str,
        count: bool,
    ) -> Result<(), &'static str> {
        super::recovery_needed();
        let Some(_) = self.decisions.refuse(pointer) else {
            super::repeated_owner_refusal(super::OwnerRegistry::Decisions);
            return Ok(());
        };
        if count {
            let _ = self.advance();
            self.count_rejected(1);
        }
        let marker = fact(
            memory,
            owner,
            DecisionChange::new(Operation::Refused, 0),
            None,
        );
        self.script_text(memory, base, generation, hour, marker, reason)
    }
    pub(crate) fn recover_decisions(
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
        self.decisions.reset(generation);
        self.decisions.census(country);
        let pointer = country.pointer;
        let recovery = self.decisions.recovery(pointer);
        let result = match recovery {
            Recovery::Certified => self.owner_scope(super::OwnerRegistry::Decisions, |store| {
                store.watch();
                store.decisions.power_baseline(memory, base, pointer)
            }),
            Recovery::Baseline => self.owner_scope(super::OwnerRegistry::Decisions, |store| {
                store.decision_baseline(memory, base, generation, state, hour, country)
            }),
        };
        let Err(refusal) = result else {
            return;
        };
        self.owner_refusals[super::OwnerRegistry::Decisions as usize] = Some(refusal);
        if self.preparing_owner {
            return;
        }
        let identity = pointer
            .checked_add(engine_native::ccountry::SURRENDER_TARGET)
            .and_then(|at| engine_native_memory::read::<u32>(memory, at))
            .map(|instance| CountryIdentity {
                reference: country.id,
                id: country.id,
                instance,
            })
            .ok_or("decision country surrender target unreadable");
        match identity {
            Ok(owner) => {
                let _ = self.refuse_decision_owner(
                    memory,
                    base,
                    generation,
                    hour,
                    pointer,
                    owner,
                    refusal.reason,
                    !refusal.counted,
                );
            }
            Err(_) => {
                self.reject(refusal.reason);
            }
        }
        if refusal.retry {
            super::recovery_needed();
        }
    }
    pub(crate) fn decision_retire<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        pointer: usize,
        swap: bool,
        visited: bool,
    ) -> Result<(), &'static str> {
        let Some(slot) = self.decisions.find(pointer) else {
            if self.decisions.uncertified(pointer).is_some() {
                self.decisions.remove(pointer);
            }
            return Ok(());
        };
        if visited {
            let cut = DecisionChange::new(Operation::PassCut, slot.container);
            self.decision_selected(
                world,
                memory,
                base,
                generation,
                slot.owner,
                pointer,
                cut,
                false,
                (None, None),
            )?;
        }
        let owner = world.country_object(slot.owner)?;
        let mut change = DecisionChange::new(Operation::Erase, slot.container);
        if visited {
            change.flags |= replay_data::event_feed_rows::decisions::ITERATION;
        }
        if swap {
            change.flags |= replay_data::event_feed_rows::decisions::SWAP;
        }
        self.decisions.remove(pointer);
        self.script_known(
            memory,
            base,
            generation,
            world.hour()?,
            fact(memory, owner, change, Some(slot.id)),
        )
    }
    pub(crate) fn decision_random<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        country: usize,
        definition: usize,
        target: u32,
        count: Option<i32>,
    ) -> Result<(), &'static str> {
        let mut change = DecisionChange::new(
            if count.is_some() {
                Operation::RandomSet
            } else {
                Operation::RandomIncrement
            },
            0,
        );
        change.ordinal = target;
        let mut value = fact(memory, world.country_object(country)?, change, None);
        value.token = engine_native_memory::msvc_string::Descriptor::read(
            memory,
            definition
                .checked_add(engine_decisions::cdecision::TOKEN)
                .ok_or("decision random token overflow")?,
        )?;
        value.script.days = count;
        self.script_known(memory, base, generation, world.hour()?, value)
    }
    pub(crate) fn decision_baseline(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        state: usize,
        hour: i32,
        country: engine_native_memory::country_identity::Country,
    ) -> Result<(), &'static str> {
        self.observation(generation)?;
        self.decisions.reset(generation);
        if self.decisions.certified(country.pointer) {
            self.watch();
            return self.decisions.power_baseline(memory, base, country.pointer);
        }
        let cut = self.sample()?;
        let world = World::new(memory, base)?;
        if world.state_pointer() != state || world.hour() != Ok(hour) {
            return Err("decision baseline world/hour differs");
        }
        let identity = world.country_object(country.pointer)?;
        self.decisions.owner(country.pointer, identity)?;
        self.decisions
            .power_baseline(memory, base, country.pointer)?;
        self.decisions.clear_owner(country.pointer, identity)?;
        let status = country
            .pointer
            .checked_add(engine_native::ccountry::DECISION_STATUS)
            .and_then(|p| engine_native_memory::read::<u64>(memory, p))
            .ok_or("decision baseline status unreadable")? as usize;
        if engine_sites::at(base, engine_decisions::cdecision_status::VTABLE_RVA)
            .is_none_or(|vtable| engine_native_memory::read::<u64>(memory, status) != Some(vtable as u64))
        {
            return Err("decision baseline status identity differs");
        }
        self.script_known(
            memory,
            base,
            generation,
            hour,
            fact(
                memory,
                identity,
                DecisionChange::new(Operation::Baseline, 0),
                None,
            ),
        )?;
        for container in 0..6 {
            let array = native::array(memory, status, container)?;
            for ordinal in 0..array.count {
                let at = array
                    .element(ordinal, 8)
                    .ok_or("decision baseline ordinal differs")?;
                let pointer = engine_native_memory::read::<u64>(memory, at)
                    .ok_or("decision baseline pointer unreadable")?
                    as usize;
                let mut change = DecisionChange::new(Operation::Store, container);
                change.ordinal = ordinal;
                self.decision_selected(
                    &world,
                    memory,
                    base,
                    generation,
                    country.pointer,
                    pointer,
                    change,
                    true,
                    (None, None),
                )?;
            }
            if native::array(memory, status, container)? != array {
                return Err("decision baseline changed during acquisition");
            }
        }
        let random = engine_native_memory::cpdx_array::Array::read(
            memory,
            status + engine_decisions::cdecision_status::RANDOM_ITEMS,
            engine_decisions::cdecision_status::RANDOM_ITEM_STRIDE,
        )
        .ok_or("decision random baseline header unreadable")?;
        for n in 0..random.count {
            let at = random
                .element(n, engine_decisions::cdecision_status::RANDOM_ITEM_STRIDE)
                .ok_or("decision random baseline ordinal differs")?;
            let definition = engine_native_memory::read::<u64>(
                memory,
                at + engine_decisions::cdecision_status::RANDOM_ITEM_DEFINITION,
            )
            .ok_or("decision random definition unreadable")? as usize;
            let target = engine_native_memory::read::<u32>(
                memory,
                at + engine_decisions::cdecision_status::RANDOM_ITEM_TARGET,
            )
            .ok_or("decision random target unreadable")?;
            let count = engine_native_memory::read::<i32>(
                memory,
                at + engine_decisions::cdecision_status::RANDOM_ITEM_SEED,
            )
            .ok_or("decision random count unreadable")?;
            self.decision_random(
                &world,
                memory,
                base,
                generation,
                country.pointer,
                definition,
                target,
                Some(count),
            )?;
        }
        if engine_native_memory::cpdx_array::Array::read(
            memory,
            status + engine_decisions::cdecision_status::RANDOM_ITEMS,
            engine_decisions::cdecision_status::RANDOM_ITEM_STRIDE,
        ) != Some(random)
        {
            return Err("decision random baseline changed during acquisition");
        }
        self.decision_roster_baseline(&world, memory, base, generation, country.pointer, status)?;
        Self::certifies(cut)?;
        self.script_known(
            memory,
            base,
            generation,
            hour,
            fact(
                memory,
                identity,
                DecisionChange::new(Operation::Ready, 0),
                None,
            ),
        )?;
        self.decisions.certify(country.pointer, identity)
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "The owner, member, native record and supplied results form one occurrence."
)]
fn record_fact<M: Memory>(
    world: &World<'_, M>,
    memory: &M,
    owner: CountryIdentity,
    mut change: DecisionChange,
    id: u32,
    row: native::Record,
    results: (Option<i32>, Option<i32>),
    supplied_update: bool,
    source: Option<EventFeedSource>,
) -> Result<ScriptFact, &'static str> {
    if change.operation != Operation::Ignore {
        change.flags |= row.flags;
    }
    let mut value = fact(memory, owner, change, Some(id));
    if let Some(source) = source {
        value.source = source;
        value.cause = crate::war_provenance::inside();
    }
    value.token = engine_native_memory::msvc_string::Descriptor::read(
        memory,
        row.definition
            .checked_add(engine_decisions::cdecision::TOKEN)
            .ok_or("decision token overflow")?,
    )?;
    value.script.days = results.1.or_else(|| (!supplied_update).then_some(row.days));
    value.script.lifecycle = results
        .0
        .or_else(|| (!supplied_update).then_some(row.lifecycle));
    value.script.country_target = row.country_target;
    value.script.numeric_target = row.numeric_target;
    if let Some(target) = row.country_target.filter(|target| *target as i32 > 0) {
        value.target = world.country(target)?;
    }
    Ok(value)
}

impl Store {
    pub(super) fn prepared_decision_demand(&self, source: &Self, completed: bool) -> usize {
        self.decisions
            .live
            .saturating_add(if completed { source.decisions.live } else { 0 })
            .max(self.decisions.owner_count.saturating_add(1))
    }
    pub(super) fn merge_decision_owner(
        &mut self,
        source: &Self,
        pointer: usize,
        labels: &mut [u32],
    ) -> Result<(), &'static str> {
        let at = Index::locate(&source.decisions.owners, pointer, |s| s.pointer)
            .map_err(|_| "prepared decision owner missing")?;
        let owner = source.decisions.owners[at];
        self.decisions.clear_owner(pointer, owner.identity)?;
        let target = self.decisions.owner(pointer, owner.identity)?;
        target.power = owner.power;
        target.power_epoch = owner.power_epoch;
        for slot in source
            .decisions
            .slots
            .iter()
            .filter(|slot| slot.pointer != 0)
        {
            let (installed, _) =
                self.decisions
                    .put(slot.pointer, pointer, owner.identity, slot.container)?;
            labels[slot.id as usize] = installed.id;
        }
        self.decisions.certify(pointer, owner.identity)
    }
    pub(super) fn prepared_decision_labels(&self) -> usize {
        self.decisions.next as usize
    }
    pub(super) fn prepared_decision_failure(
        &mut self,
        pointer: usize,
        identity: CountryIdentity,
    ) -> bool {
        let Ok(owner) = self.decisions.owner(pointer, identity) else {
            return false;
        };
        owner.refused
    }
    pub(super) fn fresh_decision_label(&mut self) -> Result<u32, &'static str> {
        self.decisions.fresh_id()
    }
    pub(crate) fn decision_key<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        country: usize,
        change: DecisionChange,
        token: engine_native_memory::msvc_string::Descriptor,
        root: Option<u32>,
    ) -> Result<(), &'static str> {
        let owner = world.country_object(country)?;
        let hour = world.hour()?;
        self.decision_key_resolved(world, memory, base, generation, owner, hour, change, token, root)
    }
    pub(crate) fn decision_key_resolved<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        owner: CountryIdentity,
        hour: i32,
        change: DecisionChange,
        token: engine_native_memory::msvc_string::Descriptor,
        root: Option<u32>,
    ) -> Result<(), &'static str> {
        let mut value = fact(memory, owner, change, None);
        value.token = token;
        value.script.country_target = root;
        if change.operation == Operation::Candidate
            && matches!(change.container, 0 | 2)
            && change.flags & replay_data::event_feed_rows::decisions::ALL == 0
        {
            value.target = world.country(change.ordinal)?;
        }
        self.script_known(memory, base, generation, hour, value)
    }
    fn decision_roster_baseline<M: Memory>(
        &mut self,
        world: &World<'_, M>,
        memory: &M,
        base: usize,
        generation: u64,
        country: usize,
        status: usize,
    ) -> Result<(), &'static str> {
        for (kind, offset) in native::CANDIDATE_HEADERS.into_iter().enumerate() {
            let array = engine_native_memory::cpdx_array::Array::read(
                memory,
                status + offset,
                native::CANDIDATE_STRIDE,
            )
            .ok_or("decision candidate baseline header unreadable")?;
            for ordinal in 0..array.count {
                let at = array
                    .element(ordinal, native::CANDIDATE_STRIDE)
                    .ok_or("decision candidate baseline ordinal differs")?;
                let definition =
                    engine_native_memory::read::<u64>(memory, at + native::CANDIDATE_DEFINITION)
                        .ok_or("decision candidate definition unreadable")?
                        as usize;
                let target = engine_native_memory::read::<u32>(memory, at)
                    .ok_or("decision candidate target unreadable")?;
                let root = engine_native_memory::read::<u32>(memory, at + native::CANDIDATE_ROOT)
                    .ok_or("decision candidate root unreadable")?;
                world.country(root)?;
                let token = engine_native_memory::msvc_string::Descriptor::read(
                    memory,
                    definition + engine_decisions::cdecision::TOKEN,
                )?;
                self.decision_key(
                    world,
                    memory,
                    base,
                    generation,
                    country,
                    DecisionChange {
                        operation: Operation::Candidate,
                        container: kind as u8,
                        flags: 0,
                        ordinal: target,
                        political_power: None,
                    },
                    token,
                    Some(root),
                )?;
            }
            if engine_native_memory::cpdx_array::Array::read(
                memory,
                status + offset,
                native::CANDIDATE_STRIDE,
            ) != Some(array)
            {
                return Err("decision candidate baseline changed during acquisition");
            }
        }
        let ignored =
            engine_native_memory::cpdx_array::Array::read(memory, country + native::IGNORED_TOKENS, 40)
                .ok_or("decision ignored-token baseline unreadable")?;
        for ordinal in 0..ignored.count {
            let at = ignored
                .element(ordinal, 40)
                .ok_or("decision ignored-token ordinal differs")?;
            self.decision_key(
                world,
                memory,
                base,
                generation,
                country,
                DecisionChange {
                    operation: Operation::Ignore,
                    container: 0,
                    flags: replay_data::event_feed_rows::decisions::IGNORED,
                    ordinal: 0,
                    political_power: None,
                },
                engine_native_memory::msvc_string::Descriptor::read(memory, at)?,
                None,
            )?;
        }
        if engine_native_memory::cpdx_array::Array::read(memory, country + native::IGNORED_TOKENS, 40)
            != Some(ignored)
        {
            return Err("decision ignored-token baseline changed during acquisition");
        }
        Ok(())
    }
}

#[cfg(test)]
impl Index {
    pub(super) fn certify_quiet(&mut self, pointer: usize, identity: CountryIdentity) {
        self.certify(pointer, identity).unwrap();
        let at = Self::locate(&self.owners, pointer, |s| s.pointer).unwrap();
        self.owners[at].power_epoch = POWER_EPOCH.load(Ordering::Acquire);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(id: u32) -> CountryIdentity {
        CountryIdentity {
            reference: id,
            id,
            instance: 1,
        }
    }
    #[test]
    fn owner_certificates_reset_only_the_marked_chain() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(40).unwrap();
        index.reset(7);
        for id in 1..=20 {
            let owner = id as usize * 0x1000;
            let pointer = owner + 8;
            index.put(pointer, owner, identity(id), 3).unwrap();
            index.certify(owner, identity(id)).unwrap();
        }
        assert!(index.mark(0x1000, identity(1)).unwrap());
        assert!(!index.mark(0x1000, identity(1)).unwrap());
        assert!(index.find(0x1008).is_none());
        assert!(index.find(0x2008).is_some());
        index.clear_owner(0x1000, identity(1)).unwrap();
        assert_eq!(index.live, 19);
        let (_, created) = index.put(0x1008, 0x1000, identity(1), 3).unwrap();
        assert!(created);
        index.reset(8);
        assert!(index.find(0x2008).is_none());
        assert_eq!(index.live, 0);
    }
    #[test]
    fn prepared_growth_keeps_owner_links_and_reused_addresses_get_new_id() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(4).unwrap();
        index.reset(1);
        let (one, _) = index.put(17, 100, identity(1), 3).unwrap();
        index.put(18, 100, identity(1), 4).unwrap();
        index.certify(100, identity(1)).unwrap();
        let mut spare = Index::default();
        spare.prepare(5000).unwrap();
        index.install(&mut spare);
        assert_eq!(index.find(17).unwrap().id, one.id);
        index.remove(17);
        let (new, _) = index.put(17, 200, identity(2), 5).unwrap();
        assert_ne!(new.id, one.id);
        index.clear_owner(100, identity(1)).unwrap();
        assert!(index.find(18).is_none());
        index.certify(200, identity(2)).unwrap();
        assert!(index.find(17).is_some());
    }
    #[test]
    fn political_balance_forwards_only_demanded_matching_certificates() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(5000).unwrap();
        index.reset(7);
        index.certify(100, identity(1)).unwrap();
        let owner = index.owner(100, identity(1)).unwrap();
        owner.power = Some(100 * 100_000);
        owner.power_epoch = 7;
        index.power_update(999, 9, 1, 0, 1, 7);
        assert_eq!(index.owner_count, 1);
        index.power_update(100, 2, 1, 100 * 100_000, 0, 7);
        assert_eq!(index.power(100, 7), Some(100 * 100_000));
        index.power_update(100, 1, 1, 100 * 100_000, 90 * 100_000, 7);
        assert_eq!(index.power(100, 7), Some(90 * 100_000));
        assert_eq!(index.power(100, 8), None);
        index.power_update(100, 1, 1, 80 * 100_000, 70 * 100_000, 7);
        assert_eq!(index.power(100, 7), None);
        assert_eq!(
            index
                .owners
                .iter()
                .find(|o| o.pointer == 100)
                .unwrap()
                .power_epoch,
            0
        );
        let owner = index.owner(100, identity(1)).unwrap();
        owner.power = Some(50);
        owner.power_epoch = 7;
        index.mark(100, identity(1)).unwrap();
        assert_eq!(index.power(100, 7), None);
    }
    #[test]
    fn a_balance_written_while_revoked_is_carried_into_the_next_certificate() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(8).unwrap();
        index.reset(7);
        index.certify(100, identity(1)).unwrap();
        let owner = index.owner(100, identity(1)).unwrap();
        owner.power = Some(100 * 100_000);
        owner.power_epoch = 7;
        index.revoke();
        assert_eq!(index.power(100, 7), None);
        index.power_update(100, 1, 1, 100 * 100_000, 90 * 100_000, 7);
        index.certify(100, identity(1)).unwrap();
        assert_eq!(index.power(100, 7), Some(90 * 100_000));
    }
    #[test]
    fn decision_labels_outlive_a_same_world_loss_and_restart_only_for_a_new_world() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(8).unwrap();
        index.reset(7);
        let mut spare = Index::default();
        spare.prepare(16).unwrap();
        crate::test_support::with_allocations_denied(|| {
            assert_eq!(index.put(0x100, 0x1000, identity(1), 3).unwrap().0.id, 1);
            index.certify(0x1000, identity(1)).unwrap();
            index.invalidate();
            assert!(!index.matches(7));
            assert!(index.slot(0x100).is_none());
            index.install(&mut spare);
            index.reset(7);
            assert!(index.matches(7));
            assert!(index.slot(0x100).is_none());
            assert!(!index.certified(0x1000));
            assert_eq!(index.put(0x100, 0x1000, identity(1), 3).unwrap().0.id, 2);
            index.reset(8);
            assert_eq!(index.put(0x100, 0x1000, identity(1), 3).unwrap().0.id, 1);
        });
    }
    #[test]
    fn revocation_keeps_exact_slots_and_retryable_owners_while_a_loss_clears_them() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(8).unwrap();
        index.reset(7);
        let (member, _) = index.put(0x100, 0x1000, identity(1), 3).unwrap();
        index.certify(0x1000, identity(1)).unwrap();
        index.owner(0x2000, identity(2)).unwrap();
        index.refuse(0x2000);
        assert_eq!(index.recovery(0x1000), Recovery::Certified);
        assert_eq!(index.recovery(0x2000), Recovery::Baseline);
        crate::test_support::with_allocations_denied(|| index.revoke());
        assert!(index.find(0x100).is_none());
        assert_eq!(index.uncertified(0x100).unwrap().id, member.id);
        assert_eq!(index.recovery(0x1000), Recovery::Baseline);
        assert_eq!(index.recovery(0x2000), Recovery::Baseline);
        index.reset(7);
        assert_eq!(index.recovery(0x2000), Recovery::Baseline);
        index.invalidate();
        index.reset(7);
        assert_eq!(index.recovery(0x2000), Recovery::Baseline);
        assert!(index.uncertified(0x100).is_none());
    }
    #[test]
    fn every_status_reset_ends_member_attribution_whatever_the_owner_state() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(8).unwrap();
        index.reset(7);
        index.put(0x100, 0x1000, identity(1), 3).unwrap();
        assert_eq!(index.refuse(0x1000), Some(true));
        assert!(index.uncertified(0x100).is_some());
        assert!(!index.mark(0x1000, identity(1)).unwrap());
        assert!(index.uncertified(0x100).is_none());
        index.certify(0x2000, identity(2)).unwrap();
        assert!(index.mark(0x2000, identity(2)).unwrap());
        index.put(0x200, 0x2000, identity(2), 3).unwrap();
        assert!(index.uncertified(0x200).is_some());
        assert!(!index.mark(0x2000, identity(2)).unwrap());
        assert!(index.uncertified(0x200).is_none());
    }
    #[test]
    fn a_certified_owner_missing_a_member_is_not_an_uncertified_slot() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(8).unwrap();
        index.reset(7);
        index.put(0x100, 0x1000, identity(1), 3).unwrap();
        index.owner(0x1000, identity(9)).unwrap();
        index.certify(0x1000, identity(9)).unwrap();
        assert!(index.find(0x100).is_none());
        assert!(index.slot(0x100).is_some());
        assert!(index.uncertified(0x100).is_none());
    }
    #[test]
    fn surrender_target_changes_preserve_decisions_and_reused_links_recover() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut index = Index::default();
        index.prepare(1500).unwrap();
        index.reset(1);
        let owner = identity(24);
        let (member, _) = index.put(100, 200, owner, 3).unwrap();
        index.certify(200, owner).unwrap();
        let changed = CountryIdentity {
            reference: 500,
            instance: 1,
            ..owner
        };
        assert_eq!(index.owner(200, changed).unwrap().identity, changed);
        assert_eq!(index.find(100).unwrap().id, member.id);
        let reused = CountryIdentity {
            reference: 1501,
            id: 1501,
            ..changed
        };
        index.owner(200, changed).unwrap();
        index.refuse(200);
        assert_eq!(index.recovery(200), Recovery::Baseline);
        index.census(engine_native_memory::country_identity::Country {
            pointer: 200,
            canonical_slot: 24,
            id: reused.id,
        });
        assert_eq!(index.recovery(200), Recovery::Baseline);
        index.certify(200, reused).unwrap();
        assert_eq!(index.recovery(200), Recovery::Certified);
        assert!(index.find(100).is_none());
    }
}
