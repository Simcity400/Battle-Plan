use super::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Key {
    Direct(usize),
    Provider(usize),
}
#[derive(Default)]
pub(super) struct Interests {
    pub(super) entries: HashMap<Key, (u32, bool)>,
    dirty_authors: std::collections::HashSet<u64>,
    pub(super) organisations: HashMap<u64, usize>,
    pub(super) organisation_pointers: HashMap<usize, u64>,
}
impl Interests {
    pub fn prepare(&mut self, entries: usize) -> Result<(), &'static str> {
        engine_allocation::reserve_map(&mut self.entries, entries)
            .map_err(|_| "design source watch allocation failed")
    }
    pub fn insert(&mut self, key: Key, owner: u32) -> Result<(), &'static str> {
        if !self.entries.contains_key(&key) && self.entries.len() == self.entries.capacity() {
            grow_map(&mut self.entries)?;
        }
        self.entries.entry(key).or_insert((owner, false)).0 = owner;
        Ok(())
    }
    pub fn prepare_organisations(&mut self, count: usize) -> Result<(), &'static str> {
        engine_allocation::reserve_set(&mut self.dirty_authors, count)
            .map_err(|_| "design liveness watch allocation failed")?;
        engine_allocation::reserve_map(&mut self.organisations, count)
            .map_err(|_| "design liveness watch allocation failed")?;
        engine_allocation::reserve_map(&mut self.organisation_pointers, count)
            .map_err(|_| "design liveness watch allocation failed")?;
        Ok(())
    }
    pub fn organisation(
        &mut self,
        handle: u64,
        pointer: Option<usize>,
    ) -> Result<(), &'static str> {
        if !self.organisations.contains_key(&handle)
            && self.organisations.len() == self.organisations.capacity()
        {
            grow_map(&mut self.organisations)?;
        }
        if let Some(old) = self.organisations.get(&handle).copied()
            && old != pointer.unwrap_or(0)
            && old != 0
        {
            self.organisation_pointers.remove(&old);
        }
        if let Some(pointer) = pointer {
            if !self.organisation_pointers.contains_key(&pointer)
                && self.organisation_pointers.len() == self.organisation_pointers.capacity()
            {
                grow_map(&mut self.organisation_pointers)?;
            }
            if self
                .organisation_pointers
                .get(&pointer)
                .is_some_and(|old| *old != handle)
            {
                return Err("design organisation address reused without retirement");
            }
            self.organisation_pointers.insert(pointer, handle);
        }
        self.organisations.insert(handle, pointer.unwrap_or(0));
        liveness::demand(true);
        Ok(())
    }
    pub(super) fn changed(&mut self, key: Key) -> Option<u32> {
        let (owner, dirty) = self.entries.get_mut(&key)?;
        *dirty = true;
        Some(*owner)
    }
    pub(super) fn author_changed(&mut self, handle: u64) {
        if !self.dirty_authors.contains(&handle)
            && self.dirty_authors.len() == self.dirty_authors.capacity()
            && engine_allocation::advisory(|| {
                engine_allocation::reserve_set(&mut self.dirty_authors, 1)
            })
            .is_err()
        {
            unobserved!();
            return;
        }
        self.dirty_authors.insert(handle);
    }
}

fn grow_map<K: Eq + std::hash::Hash, V>(map: &mut HashMap<K, V>) -> Result<(), &'static str> {
    engine_allocation::advisory(|| engine_allocation::reserve_map(map, 1))
        .map_err(|_| STORAGE_FAILED)
}

#[derive(Clone, Default)]
struct Binding {
    author: u64,
    direct: Vec<usize>,
    provider: Option<(u32, usize)>,
    handles: Vec<u32>,
    references: rows::VariantSourceReferences,
}
static BINDINGS: OnceLock<Mutex<HashMap<u64, Binding>>> = OnceLock::new();
fn bindings() -> std::sync::MutexGuard<'static, HashMap<u64, Binding>> {
    BINDINGS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(super) struct CertifiedBindings(std::sync::MutexGuard<'static, HashMap<u64, Binding>>);
impl CertifiedBindings {
    pub(super) fn unchanged(&self, id: u64, interests: &Interests) -> bool {
        let Some(binding) = self.0.get(&id) else {
            return false;
        };
        let clean = |key| interests.entries.get(&key).is_some_and(|(_, dirty)| !dirty);
        (binding.author == 0 || interests.organisations.contains_key(&binding.author))
            && !interests.dirty_authors.contains(&binding.author)
            && binding
                .direct
                .iter()
                .all(|pointer| clean(Key::Direct(*pointer)))
            && binding
                .provider
                .is_none_or(|(_, pointer)| clean(Key::Provider(pointer)))
    }
}
pub(super) fn certified_bindings() -> CertifiedBindings {
    CertifiedBindings(bindings())
}
pub(super) fn seed(variants: &Variants, interests: &mut Interests) -> Result<(), &'static str> {
    let mut held = bindings();
    let count = variants.definitions.outcome.len();
    let additional = count.saturating_sub(held.len());
    engine_allocation::reserve_map(&mut held, additional)
        .map_err(|_| "design source binding allocation failed")?;
    let demand = variants.definitions.rows[..count]
        .iter()
        .filter_map(|row| row.source_links.as_ref())
        .map(|links| links.direct.len() + usize::from(links.provider.is_some()))
        .sum::<usize>();
    interests.prepare(demand.saturating_mul(2).saturating_add(count))?;
    let mut live = std::collections::HashSet::new();
    engine_allocation::reserve_set(&mut live, count)
        .map_err(|_| "design source membership allocation failed")?;
    live.extend(
        variants.definitions.rows[..count]
            .iter()
            .map(|row| row.identity.id),
    );
    held.retain(|id, _| live.contains(id));
    interests.entries.clear();
    interests.dirty_authors.clear();
    let actual = held
        .values()
        .map(|binding| binding.direct.len() + usize::from(binding.provider.is_some()))
        .sum::<usize>();
    interests.prepare(actual.saturating_add(demand).saturating_add(count))?;
    for row in &variants.definitions.rows[..count] {
        track_in(&mut held, interests, row)?;
    }
    for binding in held.values() {
        for source in &binding.direct {
            interests.insert(Key::Direct(*source), 0)?;
        }
        if let Some((owner, provider)) = binding.provider {
            interests.insert(Key::Provider(provider), owner)?;
        }
    }
    Ok(())
}
fn track_in(
    held: &mut HashMap<u64, Binding>,
    interests: &mut Interests,
    row: &native::DefinitionRow<std::sync::Arc<rows::VariantDesignState>>,
) -> Result<bool, &'static str> {
    let Some(links) = &row.source_links else {
        return held
            .contains_key(&row.identity.id)
            .then_some(false)
            .ok_or("carried design lacks its source binding");
    };
    for source in &links.direct {
        interests.insert(Key::Direct(*source), 0)?;
    }
    if let Some((owner, provider)) = links.provider {
        interests.insert(Key::Provider(provider), owner)?;
    }
    bind(held, row)
}
fn bind(
    held: &mut HashMap<u64, Binding>,
    row: &native::DefinitionRow<std::sync::Arc<rows::VariantDesignState>>,
) -> Result<bool, &'static str> {
    if !held.contains_key(&row.identity.id)
        && let Some(binding) = suspended()
            .as_mut()
            .and_then(|suspended| suspended.remove(&row.identity.id))
    {
        engine_allocation::reserve_map(held, 1)
            .map_err(|_| "design source binding allocation failed")?;
        held.insert(row.identity.id, binding);
    }
    let Some(links) = &row.source_links else {
        return held
            .contains_key(&row.identity.id)
            .then_some(false)
            .ok_or("carried design lacks its source binding");
    };
    let state = row.design.as_ref();
    if held.get(&row.identity.id).is_some_and(|binding| {
        binding.author == state.design_team
            && binding.direct == links.direct
            && binding.provider == links.provider
            && binding.handles == state.named_bonuses
            && binding.references.direct == state.bonus_sources
            && binding.references.named == state.named_sources
    }) {
        return Ok(false);
    }
    let mut vacant = Binding::default();
    let binding = held.get_mut(&row.identity.id).unwrap_or(&mut vacant);
    fn prepare<T>(target: &mut Vec<T>, source: &[T]) -> Result<(), &'static str> {
        engine_allocation::reserve(target, source.len().saturating_sub(target.len()))
            .map_err(|_| "design source binding allocation failed")
    }
    prepare(&mut binding.direct, &links.direct)?;
    prepare(&mut binding.handles, &state.named_bonuses)?;
    prepare(&mut binding.references.direct, &state.bonus_sources)?;
    prepare(&mut binding.references.named, &state.named_sources)?;
    binding.author = state.design_team;
    binding.provider = links.provider;
    binding.direct.clear();
    binding.direct.extend_from_slice(&links.direct);
    binding.handles.clear();
    binding.handles.extend_from_slice(&state.named_bonuses);
    binding.references.direct.clear();
    binding
        .references
        .direct
        .extend_from_slice(&state.bonus_sources);
    binding.references.named.clear();
    binding
        .references
        .named
        .extend_from_slice(&state.named_sources);
    if !held.contains_key(&row.identity.id) {
        held.insert(row.identity.id, vacant);
    }
    Ok(true)
}
pub(super) fn track(
    row: &native::DefinitionRow<std::sync::Arc<rows::VariantDesignState>>,
) -> Result<(), &'static str> {
    let mut held = bindings();
    engine_allocation::reserve_map(&mut held, 1)
        .map_err(|_| "design source binding allocation failed")?;
    if bind(&mut held, row)? {
        TRIM.store(true, Ordering::Release);
    }
    Ok(())
}
pub(super) fn repair(
    row: &native::DefinitionRow<std::sync::Arc<rows::VariantDesignState>>,
    interests: &mut Interests,
) -> Result<(), &'static str> {
    interests.prepare(
        row.source_links
            .as_ref()
            .map_or(0, |links| links.direct.len() + 1),
    )?;
    let mut held = bindings();
    engine_allocation::reserve_map(&mut held, 1)
        .map_err(|_| "design source binding allocation failed")?;
    if track_in(&mut held, interests, row)? {
        TRIM.store(true, Ordering::Release);
    }
    Ok(())
}
static TRIM: AtomicBool = AtomicBool::new(false);
pub(super) fn prune_interests() -> Result<(), &'static str> {
    if !TRIM.load(Ordering::Acquire) {
        return Ok(());
    }
    let Some(store) = STORE.get() else {
        return Ok(());
    };
    let mut store = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if active() != 0
        || !store.batch.events.is_empty()
        || store.epoch != EPOCH.load(Ordering::Acquire)
    {
        return Ok(());
    }
    let held = bindings();
    let mut keys = std::collections::HashSet::new();
    let demand = held
        .values()
        .map(|binding| binding.direct.len() + usize::from(binding.provider.is_some()))
        .sum();
    engine_allocation::reserve_set(&mut keys, demand)
        .map_err(|_| "design source interest allocation failed")?;
    let mut authors = std::collections::HashSet::new();
    engine_allocation::reserve_set(&mut authors, held.len())
        .map_err(|_| "design author interest allocation failed")?;
    for binding in held.values() {
        keys.extend(binding.direct.iter().map(|pointer| Key::Direct(*pointer)));
        if let Some((_, pointer)) = binding.provider {
            keys.insert(Key::Provider(pointer));
        }
        if binding.author != 0 {
            authors.insert(binding.author);
        }
    }
    store.interests.entries.retain(|key, _| keys.contains(key));
    store
        .interests
        .organisations
        .retain(|handle, _| authors.contains(handle));
    store
        .interests
        .organisation_pointers
        .retain(|_, handle| authors.contains(handle));
    super::liveness::demand(!store.interests.organisations.is_empty());
    super::liveness::retain(&authors);
    TRIM.store(false, Ordering::Release);
    Ok(())
}
pub(super) fn author_designs(handle: u64) -> Result<Vec<u64>, &'static str> {
    let held = bindings();
    let mut designs = engine_allocation::vec(held.len())
        .map_err(|_| "variant author dependency allocation failed")?;
    designs.extend(
        held.iter()
            .filter(|(_, binding)| binding.author == handle)
            .map(|(id, _)| *id),
    );
    Ok(designs)
}
pub(super) fn remove(variant: u64) {
    if bindings().remove(&variant).is_some() {
        TRIM.store(true, Ordering::Release);
    }
}
static SUSPENDED: Mutex<Option<HashMap<u64, Binding>>> = Mutex::new(None);
fn suspended() -> std::sync::MutexGuard<'static, Option<HashMap<u64, Binding>>> {
    SUSPENDED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(super) fn suspend(variant: u64) -> Result<(), &'static str> {
    let mut held = bindings();
    let mut suspended = suspended();
    let suspended = suspended.get_or_insert_with(HashMap::new);
    engine_allocation::reserve_map(suspended, 1)
        .map_err(|_| "design source binding allocation failed")?;
    if let Some(binding) = held.remove(&variant) {
        suspended.insert(variant, binding);
    }
    Ok(())
}
pub(super) fn resume(variant: u64) -> Result<(), &'static str> {
    let mut held = bindings();
    let Some(binding) = suspended()
        .as_mut()
        .and_then(|suspended| suspended.remove(&variant))
    else {
        return Ok(());
    };
    engine_allocation::reserve_map(&mut held, 1)
        .map_err(|_| "design source binding allocation failed")?;
    held.insert(variant, binding);
    Ok(())
}
pub(super) fn drop_suspended() {
    if let Some(suspended) = suspended().as_mut()
        && !suspended.is_empty()
    {
        suspended.clear();
        TRIM.store(true, Ordering::Release);
    }
}
pub(super) fn release() {
    drop(suspended().take());
    bindings().clear();
}

pub(super) fn decode(
    event: &Event,
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<rows::VariantChangeRow>,
) -> Result<(), &'static str> {
    decode_in(&mut bindings(), event, memory, base, output)
}
fn decode_in(
    held: &mut HashMap<u64, Binding>,
    event: &Event,
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<rows::VariantChangeRow>,
) -> Result<(), &'static str> {
    let mut affected = Vec::new();
    match event.kind {
        Kind::Source => {
            let source = native::read_bonus_source(memory, base, event.pointer)?;
            let snapshot = super::super::bonus_snapshots::intern_source(&source)?;
            for (id, binding) in held.iter_mut() {
                let mut changed = false;
                for (pointer, reference) in
                    binding.direct.iter().zip(&mut binding.references.direct)
                {
                    if *pointer == event.pointer && *reference != snapshot {
                        *reference = snapshot;
                        changed = true;
                    }
                }
                if changed {
                    engine_allocation::reserve(&mut affected, 1)
                        .map_err(|_| "design source change allocation failed")?;
                    affected.push(*id);
                }
            }
        }
        Kind::SourceRemoved => {
            if held
                .values()
                .any(|binding| binding.direct.contains(&event.pointer))
            {
                return Err("applied design source destroyed while still referenced");
            }
        }
        Kind::Provider(owner) | Kind::ProviderReset(owner) => {
            let mut sources = HashMap::new();
            if matches!(event.kind, Kind::Provider(_)) {
                let roster = Array::read(
                    memory,
                    event.pointer + native::COUNTRY_NAMED_BONUSES,
                    native::NAMED_BONUS_STRIDE,
                )
                .ok_or("design source provider roster unreadable")?;
                engine_allocation::reserve_map(&mut sources, roster.count as usize)
                    .map_err(|_| "design source provider allocation failed")?;
                for ordinal in 0..roster.count {
                    let pointer = roster
                        .element(ordinal, native::NAMED_BONUS_STRIDE)
                        .ok_or("design source provider address overflow")?;
                    let handle =
                        engine_native_memory::read::<u32>(memory, pointer + native::NAMED_BONUS_HANDLE)
                            .ok_or("design source handle unreadable")?;
                    if sources.contains_key(&handle) {
                        continue;
                    }
                    let source =
                        native::read_named_bonus_source(memory, base, pointer, owner, handle)?;
                    sources.insert(handle, source);
                }
            }
            for (id, binding) in held.iter_mut().filter(|(_, binding)| {
                binding
                    .provider
                    .is_some_and(|(_, pointer)| pointer == event.pointer)
            }) {
                let mut named = engine_allocation::vec(binding.handles.len())
                    .map_err(|_| "design source reference allocation failed")?;
                for handle in &binding.handles {
                    if *handle != u32::MAX
                        && let Some(source) = sources.get(handle)
                    {
                        let mut source = rows::NativeValue::try_clone(source)
                            .map_err(|_| "design source clone allocation failed")?;
                        source.named.as_mut().unwrap().owner = binding.provider.unwrap().0;
                        named.push(super::super::bonus_snapshots::intern_source(&source)?);
                    }
                }
                if named != binding.references.named {
                    binding.references.named = named;
                    engine_allocation::reserve(&mut affected, 1)
                        .map_err(|_| "design source change allocation failed")?;
                    affected.push(*id);
                }
            }
        }
        Kind::Design | Kind::Organisation(_) => return Ok(()),
    }
    affected.sort_unstable();
    for id in affected {
        engine_allocation::reserve(output, 1)
            .map_err(|_| "design source event allocation failed")?;
        output.push(rows::VariantChangeRow {
            sequence: OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            hour: event.hour,
            variant: id,
            sources: Some(rows::VariantSourceReferences {
                direct: engine_allocation::copy_vec(&held[&id].references.direct)
                    .map_err(|_| "design source reference allocation failed")?,
                named: engine_allocation::copy_vec(&held[&id].references.named)
                    .map_err(|_| "design source reference allocation failed")?,
            }),
            ..Default::default()
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: usize = 0x4000;
    fn direct(value: i64) -> Vec<u8> {
        let mut bytes = vec![0; 0x100];
        bytes[..8].copy_from_slice(&(native::SPECIFIC_BONUS_VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[native::BONUS_STATS..native::BONUS_STATS + 8]
            .copy_from_slice(&((SOURCE + 0x80) as u64).to_le_bytes());
        bytes[native::BONUS_STATS + 8..native::BONUS_STATS + 12]
            .copy_from_slice(&1_u32.to_le_bytes());
        bytes[native::BONUS_STATS + 12..native::BONUS_STATS + 16]
            .copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x88..0x90].copy_from_slice(&value.to_le_bytes());
        bytes
    }
    fn event(kind: Kind, pointer: usize) -> Event {
        Event {
            kind,
            sequence: 1,
            hour: 7,
            pointer,
            variant: 0,
            removed: false,
            start: 0,
            end: 0,
        }
    }
    fn binding() -> Binding {
        Binding {
            author: 0,
            direct: vec![SOURCE, SOURCE],
            provider: Some((5_000, 0x1000)),
            handles: vec![u32::MAX, 7, 99, 7],
            references: rows::VariantSourceReferences {
                direct: vec![1, 1],
                named: vec![2, 2],
            },
        }
    }
    #[test]
    fn staged_bindings_reuse_storage_for_unchanged_and_replaced_values() {
        let binding = binding();
        let mut row = native::DefinitionRow::default();
        row.identity.id = 1;
        row.source_links = Some(native::DesignSourceLinks {
            direct: binding.direct.clone(),
            provider: binding.provider,
        });
        row.design = std::sync::Arc::new(rows::VariantDesignState {
            named_bonuses: binding.handles.clone(),
            bonus_sources: binding.references.direct.clone(),
            named_sources: binding.references.named.clone(),
            ..Default::default()
        });
        *bindings() = HashMap::from([(1, binding)]);
        collector_test_support::with_allocations_denied(|| track(&row)).unwrap();
        let state = std::sync::Arc::make_mut(&mut row.design);
        for value in &mut state.bonus_sources {
            *value += 1;
        }
        collector_test_support::with_allocations_denied(|| track(&row)).unwrap();
        assert_eq!(bindings()[&1].references.direct, row.design.bonus_sources);
    }
    #[test]
    fn source_occurrences_select_all_raw_owner_aliases() {
        let first = binding();
        let mut alias = first.clone();
        alias.provider.as_mut().unwrap().0 = 4_999;
        let mut unrelated = first.clone();
        unrelated.direct = vec![SOURCE + 1];
        unrelated.provider = Some((4_998, 0x2000));
        let mut held = HashMap::from([(1, first), (2, alias), (3, unrelated)]);
        let bytes = direct(42);
        let memory = engine_native_memory::memory::SliceMemory {
            base: SOURCE,
            bytes: &bytes,
        };
        let mut output = Vec::new();
        decode_in(
            &mut held,
            &event(Kind::Source, SOURCE),
            &memory,
            0,
            &mut output,
        )
        .unwrap();
        assert_eq!(
            output.iter().map(|row| row.variant).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(held[&3].references.direct, [1, 1]);
    }
    #[test]
    fn repeated_source_writes_freeze_distinct_versions_and_preserve_duplicate_references() {
        let mut held = HashMap::from([(1, binding())]);
        let mut output = Vec::new();
        for value in [42, 43] {
            let bytes = direct(value);
            let memory = engine_native_memory::memory::SliceMemory {
                base: SOURCE,
                bytes: &bytes,
            };
            let source = native::read_bonus_source(&memory, 0, SOURCE).unwrap();
            let id = rows::variant_bonus_snapshot_id(&source);
            decode_in(
                &mut held,
                &event(Kind::Source, SOURCE),
                &memory,
                0,
                &mut output,
            )
            .unwrap();
            assert_eq!(
                output.last().unwrap().sources.as_ref().unwrap().direct,
                [id, id]
            );
            assert_eq!(
                output.last().unwrap().sources.as_ref().unwrap().named,
                [2, 2]
            );
        }
        assert_eq!(output.len(), 2);
        assert_ne!(output[0].sources, output[1].sources);
        assert!(output[0].sequence < output[1].sequence);
        for row in output {
            rows::RowCodec::check_key_fields(&row).unwrap();
        }
    }
    #[test]
    fn destruction_refuses_a_surviving_dependency_and_never_reads_destroyed_storage() {
        struct Dead;
        impl Memory for Dead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("retired storage must not be read")
            }
        }
        let mut held = HashMap::from([(1, binding())]);
        let mut output = Vec::new();
        assert!(
            decode_in(
                &mut held,
                &event(Kind::SourceRemoved, SOURCE),
                &Dead,
                0,
                &mut output
            )
            .unwrap_err()
            .contains("still referenced")
        );
        held.clear();
        collector_test_support::with_allocations_denied(|| {
            decode_in(
                &mut held,
                &event(Kind::SourceRemoved, SOURCE),
                &Dead,
                0,
                &mut output,
            )
        })
        .unwrap();
        assert!(output.is_empty());
    }
    #[test]
    fn provider_reset_and_reused_handles_join_each_raw_owner_and_skip_unresolved_handles() {
        let mut one = binding();
        one.direct.clear();
        one.references.direct.clear();
        let mut alias = one.clone();
        alias.provider = Some((4_999, 0x1000));
        let mut held = HashMap::from([(1, one), (2, alias)]);
        let mut output = Vec::new();
        let empty = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        decode_in(
            &mut held,
            &event(Kind::ProviderReset(5_000), 0x1000),
            &empty,
            0,
            &mut output,
        )
        .unwrap();
        assert!(
            output
                .iter()
                .all(|row| row.sources.as_ref().unwrap().named.is_empty())
        );
        let mut bytes = vec![0; 0x1100];
        let header = native::COUNTRY_NAMED_BONUSES;
        bytes[header..header + 8].copy_from_slice(&0x2000_u64.to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x1000..0x1008]
            .copy_from_slice(&(native::NAMED_BONUS_VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x1008..0x1010]
            .copy_from_slice(&(native::SPECIFIC_BONUS_VTABLE_RVA.0 as u64).to_le_bytes());
        for offset in [native::NAMED_BONUS_NAME, native::NAMED_BONUS_DESCRIPTION] {
            bytes[0x1000 + offset + 24..0x1000 + offset + 32]
                .copy_from_slice(&15_u64.to_le_bytes());
        }
        bytes[0x1000 + native::NAMED_BONUS_HANDLE..0x1000 + native::NAMED_BONUS_HANDLE + 4]
            .copy_from_slice(&7_u32.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        decode_in(
            &mut held,
            &event(Kind::Provider(5_000), 0x1000),
            &memory,
            0,
            &mut output,
        )
        .unwrap();
        assert_eq!(output.len(), 4);
        for (row, owner) in output[2..].iter().zip([5_000, 4_999]) {
            let expected = native::read_named_bonus_source(&memory, 0, 0x2000, owner, 7).unwrap();
            let id = rows::variant_bonus_snapshot_id(&expected);
            assert_eq!(row.sources.as_ref().unwrap().named, [id, id]);
            rows::RowCodec::check_key_fields(row).unwrap();
        }
        assert_ne!(output[2].sources, output[3].sources);
    }
    #[test]
    fn prepared_hash_watches_keep_owner_lifetimes_without_native_reads_or_allocation() {
        let mut interests = Interests::default();
        interests.prepare_organisations(10_000).unwrap();
        collector_test_support::with_allocations_denied(|| {
            for handle in 1..=5_000 {
                interests
                    .organisation(handle, Some(handle as usize + 10_000))
                    .unwrap();
            }
            interests.organisation(2_500, Some(50_000)).unwrap();
            assert!(!interests.organisation_pointers.contains_key(&12_500));
            assert_eq!(interests.organisation_pointers[&50_000], 2_500);
            interests.organisation_pointers.remove(&50_000);
            interests.organisation(2_500, None).unwrap();
            interests.organisation(9_999, Some(50_000)).unwrap();
            assert_eq!(interests.organisation_pointers[&50_000], 9_999);
        });
    }
    #[test]
    fn variant_recovery_bindings_select_only_changed_sources_providers_and_authors() {
        let mut one = binding();
        one.author = 11;
        let mut two = binding();
        two.author = 22;
        two.direct = vec![SOURCE + 1];
        two.provider = Some((5_001, 0x2000));
        *bindings() = HashMap::from([(1, one), (2, two)]);
        let mut interests = Interests::default();
        interests.prepare(4).unwrap();
        interests.prepare_organisations(2).unwrap();
        interests.organisation(11, None).unwrap();
        interests.organisation(22, None).unwrap();
        for key in [
            Key::Direct(SOURCE),
            Key::Direct(SOURCE + 1),
            Key::Provider(0x1000),
            Key::Provider(0x2000),
        ] {
            interests.insert(key, 0).unwrap();
        }
        let proof = certified_bindings();
        collector_test_support::with_allocations_denied(|| {
            assert!(proof.unchanged(1, &interests) && proof.unchanged(2, &interests));
            assert!(!proof.unchanged(3, &interests));
            interests.changed(Key::Direct(SOURCE));
            interests.insert(Key::Direct(SOURCE), 0).unwrap();
            assert!(
                !proof.unchanged(1, &interests),
                "restaging must not erase an earlier dirty mark"
            );
            assert!(proof.unchanged(2, &interests));
            interests.author_changed(22);
            assert!(!proof.unchanged(2, &interests));
            interests.dirty_authors.clear();
            interests.changed(Key::Provider(0x2000));
            assert!(!proof.unchanged(2, &interests));
        });
    }
    #[test]
    fn unchanged_fresh_variant_source_bindings_keep_their_allocations() {
        let mut held = HashMap::from([(1, binding())]);
        let mut interests = Interests::default();
        interests.prepare(2).unwrap();
        let row = native::DefinitionRow {
            identity: native::Row {
                id: 1,
                ..Default::default()
            },
            design: std::sync::Arc::new(rows::VariantDesignState {
                named_bonuses: held[&1].handles.clone(),
                bonus_sources: held[&1].references.direct.clone(),
                named_sources: held[&1].references.named.clone(),
                ..Default::default()
            }),
            source_links: Some(native::DesignSourceLinks {
                direct: held[&1].direct.clone(),
                provider: held[&1].provider,
            }),
            ..Default::default()
        };
        let pointers = (
            held[&1].direct.as_ptr(),
            held[&1].handles.as_ptr(),
            held[&1].references.direct.as_ptr(),
        );
        collector_test_support::with_allocations_denied(|| {
            track_in(&mut held, &mut interests, &row)
        })
        .unwrap();
        assert_eq!(
            pointers,
            (
                held[&1].direct.as_ptr(),
                held[&1].handles.as_ptr(),
                held[&1].references.direct.as_ptr()
            )
        );
    }
}
