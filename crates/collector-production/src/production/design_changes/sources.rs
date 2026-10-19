//! Independent provider observations. Native callbacks copy only a demanded
//! source or owner roster; workers update immutable references for dependants.
use super::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Key {
    Direct(usize),
    Provider(usize),
}
#[derive(Default)]
pub(super) struct Interests {
    entries: HashMap<Key, usize>,
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
            return Err("design source watch capacity exhausted");
        }
        self.entries.insert(key, owner as usize);
        Ok(())
    }
    pub fn prepare_organisations(&mut self, count: usize) -> Result<(), &'static str> {
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
            return Err("design liveness watch capacity exhausted");
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
                return Err("design liveness pointer capacity exhausted");
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
        Ok(())
    }
    pub fn get(&self, key: Key) -> Option<u32> {
        self.entries.get(&key).map(|owner| *owner as u32)
    }
}

#[derive(Clone)]
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
pub(super) fn seed(variants: &Variants, interests: &mut Interests) -> Result<(), &'static str> {
    let mut held = bindings();
    let count = variants.definitions.outcome.len();
    engine_allocation::reserve_map(&mut held, count)
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
) -> Result<(), &'static str> {
    let Some(links) = &row.source_links else {
        return held
            .contains_key(&row.identity.id)
            .then_some(())
            .ok_or("carried design lacks its source binding");
    };
    for source in &links.direct {
        interests.insert(Key::Direct(*source), 0)?;
    }
    if let Some((owner, provider)) = links.provider {
        interests.insert(Key::Provider(provider), owner)?;
    }
    let state = row.design.as_ref();
    held.insert(
        row.identity.id,
        Binding {
            author: state.design_team,
            direct: engine_allocation::copy_vec(&links.direct)
                .map_err(|_| "design source binding allocation failed")?,
            provider: links.provider,
            handles: engine_allocation::copy_vec(&state.named_bonuses)
                .map_err(|_| "design source binding allocation failed")?,
            references: rows::VariantSourceReferences {
                direct: engine_allocation::copy_vec(&state.bonus_sources)
                    .map_err(|_| "design source binding allocation failed")?,
                named: engine_allocation::copy_vec(&state.named_sources)
                    .map_err(|_| "design source binding allocation failed")?,
            },
        },
    );
    Ok(())
}
pub(super) fn track(
    row: &native::DefinitionRow<std::sync::Arc<rows::VariantDesignState>>,
) -> Result<(), &'static str> {
    let mut held = bindings();
    engine_allocation::reserve_map(&mut held, 1)
        .map_err(|_| "design source binding allocation failed")?;
    // Interests were already registered while staging the native occurrence.
    let mut ignored = Interests::default();
    ignored.prepare(
        row.source_links
            .as_ref()
            .map_or(0, |links| links.direct.len() + 1),
    )?;
    track_in(&mut held, &mut ignored, row)?;
    TRIM.store(true, Ordering::Release);
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
    super::liveness::retain(&authors);
    TRIM.store(false, Ordering::Release);
    Ok(())
}
pub(super) fn remove(variant: u64) {
    bindings().remove(&variant);
    TRIM.store(true, Ordering::Release);
}
pub(super) fn release() {
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
                        engine_native::read::<u32>(memory, pointer + native::NAMED_BONUS_HANDLE)
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
        Kind::Design | Kind::Scalar(_) | Kind::Organisation(_) => return Ok(()),
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
        bytes[..8].copy_from_slice(&(native::SPECIFIC_BONUS_VTABLE_RVA as u64).to_le_bytes());
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
            flags: None,
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
    fn repeated_source_writes_freeze_distinct_versions_and_preserve_duplicate_references() {
        let mut held = HashMap::from([(1, binding())]);
        let mut output = Vec::new();
        for value in [42, 43] {
            let bytes = direct(value);
            let memory = engine_native::memory::SliceMemory {
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
        let empty = engine_native::memory::SliceMemory {
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
            .copy_from_slice(&(native::NAMED_BONUS_VTABLE_RVA as u64).to_le_bytes());
        bytes[0x1008..0x1010]
            .copy_from_slice(&(native::SPECIFIC_BONUS_VTABLE_RVA as u64).to_le_bytes());
        for offset in [native::NAMED_BONUS_NAME, native::NAMED_BONUS_DESCRIPTION] {
            bytes[0x1000 + offset + 24..0x1000 + offset + 32]
                .copy_from_slice(&15_u64.to_le_bytes());
        }
        bytes[0x1000 + native::NAMED_BONUS_HANDLE..0x1000 + native::NAMED_BONUS_HANDLE + 4]
            .copy_from_slice(&7_u32.to_le_bytes());
        let memory = engine_native::memory::SliceMemory {
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
}
