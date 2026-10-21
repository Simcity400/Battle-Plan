//! Range-owned native reads and warm registrations; cold admission and row edits stay ordered.
use super::*;

#[derive(Default)]
pub(super) struct NativeReads {
    pub complete: bool,
    pub root_certificates: HashMap<(u8, u64), (u64, u64)>,
    pub root_slots: HashMap<(u8, u64), usize>,
    pub membership_observation: Option<((u64, u64), usize)>,
    pub state_certificates: HashMap<(u32, u32), (u64, u64)>,
    pub state_slots: HashMap<(u32, u32), usize>,
    pub variant_certificates: HashMap<(u64, usize), (u64, u64)>,
    pub variant_lifetimes: HashMap<(u64, usize), (u64, u64)>,
    roots: Vec<RootRead>,
    root_indices: HashMap<(u8, u64), Option<usize>>,
    roots_used: usize,
    memberships: Vec<MembershipRead>,
    membership_indices: HashMap<u64, usize>,
    states: HashMap<(u32, u32), Option<StateRead>>,
    variants: Vec<VariantRead>,
    variant_indices: HashMap<(u64, usize), Option<usize>>,
    variants_used: usize,
    variant_refresh: Option<bool>,
}
pub(super) struct RootRead {
    receiver: usize,
    accumulator: usize,
    certificate: Option<(u64, u64)>,
    pub rows: RootRows,
    pub result: Result<(), &'static str>,
    ready: bool,
}
impl Default for RootRead {
    fn default() -> Self {
        Self {
            receiver: 0,
            accumulator: 0,
            certificate: None,
            rows: RootRows::default(),
            result: Ok(()),
            ready: false,
        }
    }
}
/// Retained source rows are immutable between acquisitions. Every mutable
/// borrow revokes the nested total, including capacity growth and partial reads.
#[derive(Default)]
pub(super) struct RootRows {
    rows: Vec<SharedDivisionPanelSourceRow>,
    heap_bytes: OnceLock<usize>,
}
impl RootRows {
    fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<SharedDivisionPanelSourceRow>()
            + *self.heap_bytes.get_or_init(|| {
                use replay_data::rows::RowCodec;
                self.rows.iter().map(RowCodec::heap_bytes).sum()
            })
    }
}
impl std::ops::Deref for RootRows {
    type Target = Vec<SharedDivisionPanelSourceRow>;
    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}
impl std::ops::DerefMut for RootRows {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.heap_bytes.take();
        &mut self.rows
    }
}
impl<'a> IntoIterator for &'a RootRows {
    type Item = &'a SharedDivisionPanelSourceRow;
    type IntoIter = std::slice::Iter<'a, SharedDivisionPanelSourceRow>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter()
    }
}
pub(super) struct MembershipRead {
    army: usize,
    certificate: Option<(u64, u64)>,
    pub variants: Vec<(u32, u64, usize)>,
    pub result: Result<u32, &'static str>,
    ready: bool,
}
impl Default for MembershipRead {
    fn default() -> Self {
        Self {
            army: 0,
            certificate: None,
            variants: Vec::new(),
            result: Ok(0),
            ready: false,
        }
    }
}
pub(super) struct VariantRead {
    pointer: usize,
    lifetime: Option<(u64, u64)>,
    certificate: Option<(u64, u64)>,
    availability: Option<u64>,
    pub values: Vec<i64>,
    pub result: Result<u32, &'static str>,
    ready: bool,
}
impl Default for VariantRead {
    fn default() -> Self {
        Self {
            pointer: 0,
            lifetime: None,
            certificate: None,
            availability: None,
            values: Vec::new(),
            result: Ok(0),
            ready: false,
        }
    }
}
pub(super) struct StateRead {
    pointer: usize,
    certificate: Option<(u64, u64)>,
    pub values: [Result<i64, &'static str>; 3],
}
impl NativeReads {
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
    pub fn allocated_bytes(&self) -> usize {
        self.root_certificates.capacity() * std::mem::size_of::<((u8, u64), (u64, u64))>()
            + self.root_slots.capacity() * std::mem::size_of::<((u8, u64), usize)>()
            + self.state_certificates.capacity() * std::mem::size_of::<((u32, u32), (u64, u64))>()
            + self.state_slots.capacity() * std::mem::size_of::<((u32, u32), usize)>()
            + self.variant_certificates.capacity()
                * std::mem::size_of::<((u64, usize), (u64, u64))>()
            + self.variant_lifetimes.capacity() * std::mem::size_of::<((u64, usize), (u64, u64))>()
            + self.roots.capacity() * std::mem::size_of::<RootRead>()
            + self
                .roots
                .iter()
                .map(|root| root.rows.allocated_bytes())
                .sum::<usize>()
            + self.root_indices.capacity() * std::mem::size_of::<((u8, u64), Option<usize>)>()
            + self.memberships.capacity() * std::mem::size_of::<MembershipRead>()
            + self
                .memberships
                .iter()
                .map(|read| read.variants.capacity() * std::mem::size_of::<(u32, u64, usize)>())
                .sum::<usize>()
            + self.membership_indices.capacity() * std::mem::size_of::<(u64, usize)>()
            + self.states.capacity() * std::mem::size_of::<((u32, u32), Option<StateRead>)>()
            + self.variants.capacity() * std::mem::size_of::<VariantRead>()
            + self
                .variants
                .iter()
                .map(|read| read.values.capacity() * 8)
                .sum::<usize>()
            + self.variant_indices.capacity() * std::mem::size_of::<((u64, usize), Option<usize>)>()
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
        // A reset may retire every retained pointer. Selection observations are
        // independently authenticated, but held row/membership hints cannot carry.
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
        self.capture_membership(memory, base, store, seed);
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
        }) && let Some(pool) = store.pools.get(&seed.division)
        {
            for &(_, handle, pointer) in pool.variants.iter() {
                self.capture_variant(memory, base, store, handle, pointer, false);
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
                for country in [seed.roster_owner, seed.logical_owner]
                    .into_iter()
                    .flatten()
                {
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
        if self.root_indices.contains_key(&identity) {
            return;
        }
        self.root_indices.insert(identity, None);
        // Cold/full admissions stay in the original ordered slow path. Warm
        // receivers register and sample here; a slot hint never trusts a table pointer.
        let hint = store
            .roots
            .get(&identity)
            .map_or(usize::MAX, |held| held.slot);
        let acquired = changes::sequence_at(receiver, hint);
        let Some((certificate, slot)) = acquired else {
            self.complete = false;
            return;
        };
        self.root_certificates.insert(identity, certificate);
        self.root_slots.insert(identity, slot);
        let certificate = Some(certificate);
        if store.roots.get(&identity).is_some_and(|held| {
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
            // Staging shortage is not a native refusal. Before observing a
            // refusal, ordinary ordered acquisition can still consume the tree.
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
        self.root_indices.insert(identity, Some(index));
        self.roots_used += 1;
    }
    pub fn root(
        &self,
        kind: u8,
        owner: u64,
        receiver: usize,
        accumulator: usize,
        certificate: Option<(u64, u64)>,
    ) -> Option<&RootRead> {
        let read = self.roots.get((*self.root_indices.get(&(kind, owner))?)?)?;
        (read.ready
            && certificate.is_some()
            && read.certificate == certificate
            && read.receiver == receiver
            && read.accumulator == accumulator)
            .then_some(read)
    }
    fn capture_membership(
        &mut self,
        memory: &impl Memory,
        base: usize,
        store: &Store,
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
        let hint = store
            .pools
            .get(&seed.division)
            .map_or(usize::MAX, |held| held.slot);
        let acquired = operands::sequence_at(pool, hint);
        let Some((certificate, slot)) = acquired else {
            self.complete = false;
            return;
        };
        self.membership_observation = Some((certificate, slot));
        let certificate = Some(certificate);
        if store
            .pools
            .get(&seed.division)
            .is_some_and(|held| held.army == seed.pointer && held.certificate == certificate)
        {
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
            engine_native::division_panel_operands::held_variants(memory, base, seed, |entry| {
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
    pub fn membership(
        &self,
        seed: &DivisionSeed,
        certificate: Option<(u64, u64)>,
    ) -> Option<&MembershipRead> {
        let read = self
            .memberships
            .get(*self.membership_indices.get(&seed.division)?)?;
        (read.ready
            && certificate.is_some()
            && read.certificate == certificate
            && read.army == seed.pointer)
            .then_some(read)
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
        if self.states.contains_key(&(id, country)) {
            return;
        }
        self.states.insert((id, country), None);
        let hint = store
            .states
            .get(&(id, country))
            .map_or(usize::MAX, |held| held.slot);
        let acquired = operands::state_sequence_at(id, hint);
        let Some((certificate, slot)) = acquired else {
            self.complete = false;
            return;
        };
        self.state_certificates.insert((id, country), certificate);
        self.state_slots.insert((id, country), slot);
        let certificate = Some(certificate);
        if store
            .states
            .get(&(id, country))
            .is_some_and(|held| held.pointer == pointer && held.certificate == certificate)
        {
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
        self.states.insert(
            (id, country),
            Some(StateRead {
                pointer,
                certificate,
                values,
            }),
        );
    }
    pub fn state(
        &self,
        id: u32,
        pointer: usize,
        country: u32,
        certificate: Option<(u64, u64)>,
    ) -> Option<&StateRead> {
        let read = self.states.get(&(id, country))?.as_ref()?;
        (certificate.is_some() && read.certificate == certificate && read.pointer == pointer)
            .then_some(read)
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
        self.variant_indices.insert((handle, pointer), None);
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
        read.result = engine_native::division_panel_operands::terrain(
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
        self.variant_indices.insert((handle, pointer), Some(index));
        self.variants_used += 1;
    }
    pub fn has_variant(&self, handle: u64, pointer: usize) -> bool {
        self.variant_certificates.contains_key(&(handle, pointer))
    }
    pub fn root_certificate(&self, kind: u8, owner: u64) -> Option<(u64, u64)> {
        self.root_certificates.get(&(kind, owner)).copied()
    }
    pub fn variant_observed(
        &self,
        handle: u64,
        pointer: usize,
        certificate: Option<(u64, u64)>,
        availability: Option<u64>,
        owned: bool,
    ) -> Option<&VariantRead> {
        let read = self
            .variants
            .get((*self.variant_indices.get(&(handle, pointer))?)?)?;
        (read.ready
            && certificate.is_some()
            && read.certificate == certificate
            && read.pointer == pointer
            && read.availability == availability
            && (owned || read.lifetime == operands::observed_lifetime_sequence(pointer)))
        .then_some(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::memory::SliceMemory;
    use replay_data::rows::tables;

    struct Denied;
    impl Memory for Denied {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("settlement retried a prepared source observation")
        }
    }

    #[test]
    fn optional_source_staging_denial_keeps_the_first_ordered_allocation_failure() {
        let mut preparation = Preparation::default();
        assert!(!collector_test_support::with_allocations_denied(|| {
            preparation.begin(1)
        }));
        let seeds = [DivisionSeed {
            division: 7,
            ..Default::default()
        }];
        let mut serial = Store::default();
        let mut staged = Store::default();
        let mut serial_selected = Vec::new();
        let mut staged_selected = Vec::new();
        let serial_result = collector_test_support::with_allocations_denied(|| {
            serial.capture(&Denied, 0, 0, &seeds, &mut serial_selected)
        });
        let staged_result = collector_test_support::with_allocations_denied(|| {
            staged.capture_staged(
                &Denied,
                0,
                0,
                &seeds,
                &mut staged_selected,
                |_| None,
                |_| None,
                |_, _| None,
            )
        });
        assert_eq!(
            serial_result.err(),
            Some("division source selection allocation failed")
        );
        assert_eq!(
            staged_result.err(),
            Some("division source selection allocation failed")
        );
        assert_eq!(staged_selected, serial_selected);
        assert_eq!(staged.turn, serial.turn);
        assert!(staged.roots.is_empty());
        assert!(staged.pools.is_empty());
    }

    #[test]
    fn staged_root_prefix_faults_and_first_shared_admission_match_serial() {
        for fault in [false, true] {
            let mut bytes = vec![0u8; 0x1000];
            let put = |bytes: &mut [u8], at: usize, value: u64| {
                bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
            };
            let (root, receiver, owner) = (0x100, 0x600, 7);
            put(&mut bytes, root, 0x800);
            put(&mut bytes, root + 8, 1 | (1 << 32));
            put(&mut bytes, root + 0x48 + 24, 15);
            put(&mut bytes, 0x800, 57);
            put(&mut bytes, 0x808, 37);
            if fault {
                put(&mut bytes, root + 0x18, 0x900);
                put(&mut bytes, root + 0x20, 1 | (1 << 32));
            }
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let stamp = (1, 1);
            let certificate = Some(stamp);
            let mut serial = Store::default();
            serial
                .root_with_certificate(&memory, 0, owner, receiver, root, certificate)
                .unwrap();
            let expected = serial.rows.finish().unwrap();
            let mut read = RootRead {
                receiver,
                accumulator: root,
                certificate,
                ready: true,
                ..Default::default()
            };
            read.result = modifier_breakdown::source_nodes(&memory, root, |ordinal, node| {
                read.rows.push(Arc::new(DivisionPanelSourceRow {
                    kind: 0,
                    owner,
                    ordinal,
                    node: Some(node),
                    reason: Vec::new(),
                }));
                Ok(())
            });
            assert_eq!(read.result.is_err(), fault);
            assert_eq!(
                read.rows.len(),
                1,
                "complete prefix retained on a later fault"
            );
            let mut reads = NativeReads::default();
            reads.roots.push(read);
            reads.root_indices.insert((0, owner), Some(0));
            reads
                .root_certificates
                .insert((0, owner), stamp);
            let mut staged = Store {
                settling_owned: true,
                ..Store::default()
            };
            staged
                .root_prepared(&Denied, 0, owner, receiver, root, Some(&reads))
                .unwrap();
            let actual = staged.rows.finish().unwrap();
            assert_eq!(
                actual.complete::<tables::DivisionPanelSources>().unwrap(),
                expected.complete::<tables::DivisionPanelSources>().unwrap()
            );
            assert_eq!(actual.ranges, expected.ranges);
            assert_eq!(
                staged.roots[&(0, owner)].count,
                serial.roots[&(0, owner)].count
            );
            assert_eq!(staged.roots[&(0, owner)].certificate, certificate);
            reads.roots[0].rows.clear();
            reads.roots[0].result = if fault {
                Ok(())
            } else {
                Err("later observation refused")
            };
            staged
                .root_with_observation(&Denied, 0, owner, receiver, root, certificate, Some(&reads))
                .unwrap();
            assert!(
                Arc::ptr_eq(&actual, &staged.rows.finish().unwrap()),
                "the earlier shared success or refusal wins"
            );
            staged
                .root_with_observation(
                    &Denied,
                    0,
                    owner,
                    receiver + 1,
                    root + 1,
                    Some((stamp.0, stamp.1 + 1)),
                    Some(&reads),
                )
                .unwrap();
            assert!(
                Arc::ptr_eq(&actual, &staged.rows.finish().unwrap()),
                "a contradictory later alias cannot change first admission"
            );
            assert_eq!(staged.roots[&(0, owner)].receiver, receiver);
            assert_eq!(staged.roots[&(0, owner)].accumulator, root);
            assert_eq!(staged.roots[&(0, owner)].certificate, certificate);
        }
    }
}
