use super::*;

#[derive(Default)]
pub(super) struct ReferenceReads {
    pub complete: bool,
    pub root_certificates: Map<(u8, u64), (u64, u64)>,
    pub root_slots: Map<(u8, u64), usize>,
    pub membership_observation: Option<((u64, u64), usize)>,
    pub state_certificates: Map<(u32, u32), (u64, u64)>,
    pub state_slots: Map<(u32, u32), usize>,
    pub variant_certificates: Map<(u64, usize), (u64, u64)>,
    pub variant_lifetimes: Map<(u64, usize), (u64, u64)>,
    roots: Vec<RootRead>,
    root_indices: Map<(u8, u64), Option<usize>>,
    roots_used: usize,
    memberships: Vec<MembershipRead>,
    membership_indices: Map<u64, usize>,
    states: Map<(u32, u32), Option<StateRead>>,
    variants: Vec<VariantRead>,
    variant_indices: Map<(u64, usize), Option<usize>>,
    variants_used: usize,
    variant_refresh: Option<bool>,
    dirty_read: bool,
}
impl ReferenceReads {
    pub fn begin(&mut self, divisions: usize) -> std::io::Result<()> {
        self.complete = true;
        self.root_certificates.clear();
        self.root_slots.clear();
        self.membership_observation = None;
        self.state_certificates.clear();
        self.state_slots.clear();
        self.variant_certificates.clear();
        self.variant_lifetimes.clear();
        self.root_indices.clear();
        self.roots_used = 0;
        self.variants_used = 0;
        self.variant_refresh = None;
        self.dirty_read = false;
        self.membership_indices.clear();
        self.states.clear();
        self.variant_indices.clear();
        engine_allocation::reserve_map(&mut self.root_certificates, divisions.saturating_mul(2))?;
        engine_allocation::reserve_map(&mut self.root_slots, divisions.saturating_mul(2))?;
        engine_allocation::reserve_map(&mut self.state_certificates, divisions.saturating_mul(2))?;
        engine_allocation::reserve_map(&mut self.state_slots, divisions.saturating_mul(2))?;
        engine_allocation::reserve_map(&mut self.variant_certificates, divisions)?;
        engine_allocation::reserve_map(&mut self.variant_lifetimes, divisions)?;
        engine_allocation::reserve_map(&mut self.root_indices, divisions.saturating_mul(2))?;
        engine_allocation::reserve_map(&mut self.membership_indices, divisions)?;
        engine_allocation::reserve_map(&mut self.states, divisions.saturating_mul(2))?;
        engine_allocation::reserve_map(&mut self.variant_indices, divisions)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        store: &Store,
        seed: &DivisionSeed,
        selection: Selection,
        block: Option<(usize, bool)>,
    ) {
        self.membership_observation = None;
        if !store.can_settle_owned() {
            self.complete = false;
        }
        if store.world_epoch != operands::world_epoch() {
            self.complete = false;
            return;
        }
        if let Some((_, Ok(country))) = selection.country {
            if let Some(accumulator) = country.pointer.checked_add(ccountry::MODIFIER_ACCUMULATOR) {
                self.capture_root(
                    memory,
                    store,
                    0,
                    country.id as u64,
                    country.pointer,
                    accumulator,
                );
            }
            if let (Some((receiver, Ok(handle))), Some((accumulator, hq))) =
                (selection.commander, block)
                && handle as u32 == engine_native::cunit_leader::REFERENCE_TYPE
            {
                self.capture_root(
                    memory,
                    store,
                    if hq { 2 } else { 1 },
                    handle,
                    receiver,
                    accumulator,
                );
            }
        }
        let pool = store.pools.get(&seed.division);
        self.capture_membership(memory, base, pool, seed);
        if let Some(&index) = self.membership_indices.get(&seed.division)
            && self.memberships[index].ready
            && self.memberships[index].result.is_ok()
        {
            for entry in 0..self.memberships[index].variants.len() {
                let (_, handle, pointer) = self.memberships[index].variants[entry];
                self.capture_variant(memory, base, store, handle, pointer, true);
            }
        } else if *self.variant_refresh.get_or_insert_with(|| {
            let stamp = operands::observed_variant_stamp();
            stamp.is_some() && store.variant_observation != stamp
        }) {
            if store.variant_epoch != store.boundary.variant_epoch {
                if let Some(pool) = pool {
                    for &(_, handle, pointer) in pool.variants.iter() {
                        self.capture_variant(memory, base, store, handle, pointer, false);
                    }
                }
            } else if !std::mem::replace(&mut self.dirty_read, true) {
                for &pointer in &store.boundary.dirty {
                    if let Some(&handle) = store.variant_handles.get(&pointer) {
                        self.capture_variant(memory, base, store, handle, pointer, false);
                    }
                }
            }
        }
        if let Ok(province) = selection.province {
            let context = store
                .provinces
                .get(&province)
                .copied()
                .map(Ok)
                .or(selection.context)
                .and_then(Result::ok)
                .flatten();
            if let Some((id, pointer)) = context {
                for country in state_countries(seed) {
                    self.capture_state(memory, base, state, store, id, pointer, country);
                }
            }
        }
    }
    fn capture_root(
        &mut self,
        memory: &impl Memory,
        store: &Store,
        kind: u8,
        owner: u64,
        receiver: usize,
        accumulator: usize,
    ) {
        let identity = (kind, owner);
        let index_slot = match self.root_indices.entry(identity) {
            std::collections::hash_map::Entry::Occupied(_) => return,
            std::collections::hash_map::Entry::Vacant(slot) => slot.insert(None),
        };
        let held = store.roots.get(&identity);
        let hint = held.map_or(usize::MAX, |held| held.slot);
        let acquired = changes::sequence_at(receiver, hint);
        let Some((certificate, slot)) = acquired else {
            self.complete = false;
            return;
        };
        self.root_certificates.insert(identity, certificate);
        self.root_slots.insert(identity, slot);
        let certificate = Some(certificate);
        if held.is_some_and(|held| {
            held.receiver == receiver
                && held.accumulator == accumulator
                && held.certificate == certificate
        }) {
            return;
        }
        let index = self.roots_used;
        if index == self.roots.len() {
            if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.roots, 1))
                .is_err()
            {
                self.complete = false;
                return;
            }
            self.roots.push(RootRead::default());
        }
        let read = &mut self.roots[index];
        read.rows.clear();
        read.receiver = receiver;
        read.accumulator = accumulator;
        read.certificate = certificate;
        read.ready = true;
        read.result = modifier_breakdown::source_nodes(memory, accumulator, |ordinal, node| {
            if engine_allocation::advisory(|| engine_allocation::reserve(&mut read.rows, 1))
                .is_err()
            {
                read.ready = false;
                return Err("optional source staging unavailable");
            }
            ordinal
                .checked_add(1)
                .ok_or("division modifier source ordinal overflow")?;
            read.rows.push(Arc::new(DivisionPanelSourceRow {
                kind,
                owner,
                ordinal,
                node: Some(node),
                reason: Vec::new(),
            }));
            Ok(())
        });
        self.complete &= read.ready;
        *index_slot = Some(index);
        self.roots_used += 1;
    }
    fn capture_membership(
        &mut self,
        memory: &impl Memory,
        base: usize,
        held: Option<&Pool>,
        seed: &DivisionSeed,
    ) {
        if self.membership_indices.contains_key(&seed.division) {
            self.complete = false;
            return;
        }
        let Some(pool) = seed
            .pointer
            .checked_add(engine_native::carmy::EQUIPMENT_POOL)
        else {
            self.complete = false;
            return;
        };
        let hint = held.map_or(usize::MAX, |held| held.slot);
        let acquired = operands::sequence_at(pool, hint);
        let Some((certificate, slot)) = acquired else {
            self.complete = false;
            return;
        };
        self.membership_observation = Some((certificate, slot));
        let certificate = Some(certificate);
        if held.is_some_and(|held| held.army == seed.pointer && held.certificate == certificate) {
            return;
        }
        let index = self.membership_indices.len();
        if index == self.memberships.len() {
            if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.memberships, 1))
                .is_err()
            {
                self.complete = false;
                return;
            }
            self.memberships.push(MembershipRead::default());
        }
        let read = &mut self.memberships[index];
        read.army = seed.pointer;
        read.certificate = certificate;
        read.variants.clear();
        read.ready = true;
        read.result =
            engine_native_land::division_panel_operands::held_variants(memory, base, seed, |entry| {
                if engine_allocation::advisory(|| engine_allocation::reserve(&mut read.variants, 1))
                    .is_err()
                {
                    read.ready = false;
                    return Err("optional membership staging unavailable");
                }
                read.variants.push((
                    entry.source_ordinal,
                    entry.variant_handle,
                    entry.variant_pointer,
                ));
                Ok(())
            })
            .map(|count| count + 1);
        self.complete &= read.ready;
        self.membership_indices.insert(seed.division, index);
    }
    #[allow(clippy::too_many_arguments)]
    fn capture_state(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        store: &Store,
        id: u32,
        pointer: usize,
        country: u32,
    ) {
        let state_slot = match self.states.entry((id, country)) {
            std::collections::hash_map::Entry::Occupied(_) => return,
            std::collections::hash_map::Entry::Vacant(slot) => slot.insert(None),
        };
        let held = store.states.get(&(id, country));
        let hint = held.map_or(usize::MAX, |held| held.slot);
        let acquired = operands::state_sequence_at(id, hint);
        let Some((certificate, slot)) = acquired else {
            self.complete = false;
            return;
        };
        self.state_certificates.insert((id, country), certificate);
        self.state_slots.insert((id, country), slot);
        let certificate = Some(certificate);
        if held.is_some_and(|held| held.pointer == pointer && held.certificate == certificate) {
            return;
        }
        let mut values = [Err("state attrition unavailable"); 3];
        for (index, key) in [
            engine_executable::modifier_keys::ATTRITION,
            engine_executable::modifier_keys::ATTRITION_FOR_CONTROLLER,
            engine_executable::modifier_keys::ENEMY_ATTRITION,
        ]
        .into_iter()
        .enumerate()
        {
            values[index] = engine_native::cstate::movement_modifier(
                memory, base, state, pointer, country, key,
            );
            if values[index].is_err() {
                break;
            }
        }
        *state_slot = Some(StateRead {
            pointer,
            certificate,
            values,
        });
    }
    fn capture_variant(
        &mut self,
        memory: &impl Memory,
        base: usize,
        store: &Store,
        handle: u64,
        pointer: usize,
        fresh_membership: bool,
    ) {
        if self.variant_indices.contains_key(&(handle, pointer)) {
            return;
        }
        if engine_allocation::advisory(|| {
            engine_allocation::reserve_map(&mut self.variant_indices, 1)
        })
        .is_err()
        {
            self.complete = false;
            return;
        }
        let index_slot = self
            .variant_indices
            .entry((handle, pointer))
            .or_insert(None);
        let certificate = operands::observed_variant_sequence(pointer)
            .and_then(|_| operands::variant_sequence(pointer));
        let availability = operands::variant_capability();
        let lifetime = operands::observed_lifetime_sequence(pointer);
        let held = store.variants.get(&handle);
        if let (Some(certificate), Some(lifetime)) = (certificate, lifetime) {
            if engine_allocation::advisory(|| {
                engine_allocation::reserve_map(&mut self.variant_certificates, 1)?;
                engine_allocation::reserve_map(&mut self.variant_lifetimes, 1)
            })
            .is_err()
            {
                self.complete = false;
                return;
            }
            self.variant_certificates
                .insert((handle, pointer), certificate);
            self.variant_lifetimes.insert((handle, pointer), lifetime);
        } else {
            self.complete = false;
        }
        if !fresh_membership
            && !held.is_some_and(|held| {
                held.pointer == pointer && lifetime.is_some() && held.lifetime == lifetime
            })
        {
            self.complete = false;
            return;
        }
        if certificate.is_none()
            || held.is_some_and(|held| {
                held.pointer == pointer
                    && held.certificate == certificate
                    && held.availability == availability
            })
        {
            return;
        }
        let index = self.variants_used;
        if index == self.variants.len() {
            if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.variants, 1))
                .is_err()
            {
                self.complete = false;
                return;
            }
            self.variants.push(VariantRead::default());
        }
        let read = &mut self.variants[index];
        read.pointer = pointer;
        read.lifetime = lifetime;
        read.certificate = certificate;
        read.availability = availability;
        read.values.clear();
        read.ready = true;
        read.result = engine_native_land::division_panel_operands::terrain(
            memory,
            base,
            pointer,
            handle,
            |ordinal, raw| {
                if engine_allocation::advisory(|| engine_allocation::reserve(&mut read.values, 1))
                    .is_err()
                {
                    read.ready = false;
                    return Err("optional terrain staging unavailable");
                }
                debug_assert_eq!(ordinal as usize, read.values.len());
                read.values.push(raw);
                Ok(())
            },
        );
        if read.result.is_ok()
            && (operands::variant_sequence(pointer) != certificate
                || !availability.is_some_and(operands::variant_capability_is_current))
        {
            read.result = Err(VARIANT_WITHDRAWN);
        }
        self.complete &= read.ready;
        *index_slot = Some(index);
        self.variants_used += 1;
    }
}

fn assert_paired<K: Eq + std::hash::Hash, A, B, V>(
    observed: &Map<K, V>,
    first: &Map<K, A>,
    second: &Map<K, B>,
) {
    assert_eq!(first.len(), second.len());
    assert!(
        first
            .keys()
            .all(|key| second.contains_key(key) && observed.contains_key(key))
    );
}

impl ReferenceReads {
    pub fn into_current(self) -> NativeReads {
        assert_paired(
            &self.root_indices,
            &self.root_certificates,
            &self.root_slots,
        );
        assert_paired(&self.states, &self.state_certificates, &self.state_slots);
        assert_paired(
            &self.variant_indices,
            &self.variant_certificates,
            &self.variant_lifetimes,
        );
        let mut roots = Map::default();
        for (key, read) in self.root_indices {
            let sequence =
                (self.root_certificates.get(&key).copied()).zip(self.root_slots.get(&key).copied());
            roots.insert(key, RootObservation { sequence, read });
        }
        let mut states = Map::default();
        for (key, read) in self.states {
            let sequence = (self.state_certificates.get(&key).copied())
                .zip(self.state_slots.get(&key).copied());
            states.insert(key, StateObservation { sequence, read });
        }
        let mut variants = Map::default();
        for (key, read) in self.variant_indices {
            let sequence = (self.variant_certificates.get(&key).copied())
                .zip(self.variant_lifetimes.get(&key).copied());
            variants.insert(key, VariantObservation { sequence, read });
        }
        NativeReads {
            complete: self.complete,
            membership_observation: self.membership_observation,
            roots: self.roots,
            root_indices: roots,
            roots_used: self.roots_used,
            memberships: self.memberships,
            membership_indices: self.membership_indices,
            states,
            variants: self.variants,
            variant_indices: variants,
            variants_used: self.variants_used,
            variant_refresh: self.variant_refresh,
            dirty_read: self.dirty_read,
        }
    }
}
