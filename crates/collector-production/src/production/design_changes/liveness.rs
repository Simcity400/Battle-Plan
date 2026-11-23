use super::*;
use collector_hooks::hook::Hook;
use engine_production::nindustrial_organisation_corganisation as organisation;
use native::source_mutations::ORGANISATIONS;
use std::{ops::Range, sync::Arc};
static ORIGINAL: [AtomicUsize; ORGANISATIONS.len()] =
    [const { AtomicUsize::new(0) }; ORGANISATIONS.len()];
static ENABLED: AtomicBool = AtomicBool::new(false);
static DEMANDED: AtomicBool = AtomicBool::new(false);
pub(super) fn demand(demanded: bool) {
    DEMANDED.store(demanded, Ordering::Release);
}
pub(in crate::production) fn available() -> bool {
    super::enabled()
        && LIVENESS_BASELINE.load(Ordering::Acquire)
        && !RECOVER.load(Ordering::Acquire)
        && !MISSED.load(Ordering::Acquire)
        && !super::repair::pending()
}
pub(super) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}
#[derive(Default)]
struct Cache {
    entries: HashMap<u64, bool>,
    snapshot: Option<Arc<rows::TableRows>>,
    dirty: bool,
}
static ORDER: AtomicU64 = AtomicU64::new(1);
#[derive(Default)]
pub(in crate::production) struct Observation {
    pub order: u64,
    pub snapshot: Option<Arc<rows::TableRows>>,
    pub admitted: bool,
}
impl Observation {
    pub(in crate::production) fn refused() -> Self {
        Self {
            order: ORDER.fetch_add(1, Ordering::Relaxed),
            snapshot: None,
            admitted: false,
        }
    }
    pub(in crate::production) fn refuse(&mut self) {
        self.order = ORDER.fetch_add(1, Ordering::Relaxed);
        self.admitted = false;
    }
    pub(in crate::production) fn snapshot(&self) -> &Option<Arc<rows::TableRows>> {
        if self.admitted { &self.snapshot } else { &None }
    }
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE
        .get_or_init(|| Mutex::new(Cache::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(super) fn change(handle: u64, live: bool) -> Result<(), &'static str> {
    if handle == 0 {
        return Ok(());
    }
    let mut cache = cache();
    if cache.entries.get(&handle) != Some(&live) {
        engine_allocation::reserve_map(&mut cache.entries, 1)
            .map_err(|_| "design liveness allocation failed")?;
        cache.entries.insert(handle, live);
        cache.dirty = true;
    }
    Ok(())
}
pub(super) fn sample(
    row: &native::DefinitionRow<Arc<rows::VariantDesignState>>,
) -> Result<(), &'static str> {
    change(row.design.design_team, row.design.design_team_serialized)
}
pub(super) fn seed(
    variants: &Variants,
    interests: &mut sources::Interests,
) -> Result<(), &'static str> {
    let count = variants.definitions.outcome.len();
    interests.prepare_organisations(count.saturating_mul(2).max(1))?;
    for row in &variants.definitions.rows[..count] {
        seed_row(row, interests)?;
    }
    Ok(())
}
pub(super) fn seed_one(
    row: &native::DefinitionRow<Arc<rows::VariantDesignState>>,
    interests: &mut sources::Interests,
) -> Result<(), &'static str> {
    interests.prepare_organisations(2)?;
    seed_row(row, interests)
}
fn seed_row(
    row: &native::DefinitionRow<Arc<rows::VariantDesignState>>,
    interests: &mut sources::Interests,
) -> Result<(), &'static str> {
    let handle = row.design.design_team;
    if handle == 0 {
        return Ok(());
    }
    let present = cache().entries.contains_key(&handle);
    if row.source_links.is_some() || !present {
        sample(row)?;
        interests.organisation(handle, row.organisation_pointer)?;
    }
    Ok(())
}
pub(in crate::production) fn observation() -> Result<Observation, &'static str> {
    if !available() {
        return Err("design organisation liveness certificate unavailable");
    }
    let mut cache = cache();
    if cache.dirty || cache.snapshot.is_none() {
        let mut rows = engine_allocation::vec(cache.entries.len())
            .map_err(|_| "design liveness publication allocation failed")?;
        for (organisation, live) in &cache.entries {
            rows.push(rows::DesignOrganisationLivenessRow {
                organisation: *organisation,
                live: *live,
            });
        }
        rows.sort_unstable_by_key(|row| row.organisation);
        cache.snapshot = Some(Arc::new(rows::TableRows::DesignOrganisationLiveness(rows)));
        cache.dirty = false;
    }
    if !available() {
        return Err("design organisation liveness certificate changed during observation");
    }
    Ok(Observation {
        order: ORDER.fetch_add(1, Ordering::Relaxed),
        snapshot: cache.snapshot.clone(),
        admitted: true,
    })
}
pub(super) fn retain(authors: &std::collections::HashSet<u64>) {
    let mut cache = cache();
    let before = cache.entries.len();
    cache.entries.retain(|handle, _| authors.contains(handle));
    cache.dirty |= before != cache.entries.len();
}
pub(super) fn release() {
    demand(false);
    *cache() = Cache::default();
}
fn event(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    handle: u64,
    pointer: usize,
    live: bool,
) -> Result<(), &'static str> {
    store.batch.room()?;
    let state = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .ok_or("design liveness clock unreadable")?;
    let hour = engine_native_memory::read(
        memory,
        state as usize + engine_native::cgame_state::CURRENT_HOUR,
    )
    .ok_or("design liveness hour unreadable")?;
    store.batch.events.push(Event {
        kind: Kind::Organisation(live),
        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
        hour,
        pointer,
        variant: handle,
        removed: false,
        start: 0,
        end: 0,
    });
    Ok(())
}
fn record_registered(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
) {
    if !store.interests.organisations.contains_key(&handle) {
        return;
    }
    let result = (|| {
        store.interests.author_changed(handle);
        let vtable = engine_sites::at(base, organisation::VTABLE_RVA)
            .ok_or("design organisation liveness identity mismatch")?;
        if engine_native::read::<u64>(memory, pointer) != Some(vtable as u64) {
            return Err("design organisation liveness identity mismatch");
        }
        let resolved = engine_native::creference_registry::resolve(memory, base, handle)?;
        store.interests.organisation(handle, resolved)?;
        event(store, memory, base, handle, pointer, resolved.is_some())
    })();
    if let Err(reason) = result {
        store.batch.failure = Some(reason);
    }
}
pub(super) fn registered(memory: &impl Memory, base: usize, pointer: usize) {
    if !DEMANDED.load(Ordering::Acquire) {
        return;
    }
    let Some(store) = STORE.get() else {
        return;
    };
    let Some(handle) = engine_native_memory::read::<u64>(memory, pointer + organisation::REFERENCE) else {
        unobserved!();
        return;
    };
    let mut store = callback_store(store);
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return;
    }
    record_registered(&mut store, memory, base, pointer, handle);
}
pub(super) fn retired(memory: &impl Memory, base: usize, pointer: usize) {
    if !DEMANDED.load(Ordering::Acquire) {
        return;
    }
    let Some(store) = STORE.get() else {
        return;
    };
    let mut store = callback_store(store);
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return;
    }
    let Some(handle) = store.interests.organisation_pointers.remove(&pointer) else {
        return;
    };
    store.interests.author_changed(handle);
    store.interests.organisations.insert(handle, 0);
    if let Err(reason) = event(&mut store, memory, base, handle, pointer, false) {
        store.batch.failure = Some(reason);
    }
}
pub(super) fn roster(memory: &impl Memory, base: usize, header: usize) {
    if !LIVENESS_BASELINE.load(Ordering::Acquire) || !DEMANDED.load(Ordering::Acquire) {
        return;
    }
    let Some(store) = STORE.get() else {
        return;
    };
    let Some(array) = Array::read(memory, header, 8) else {
        unobserved!();
        return;
    };
    if array.count == 0 {
        return;
    }
    let mut store = callback_store(store);
    if store.epoch != EPOCH.load(Ordering::Acquire) || store.interests.organisations.is_empty() {
        return;
    }
    const ROSTER_BATCH: usize = 64;
    const REFERENCE_FIELDS: [engine_native_memory::memory::RecordField; 1] =
        [engine_native_memory::memory::RecordField {
            source_offset: organisation::REFERENCE,
            destination_offset: 0,
            len: 8,
        }];
    const REFERENCE_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
        engine_native_memory::memory::RecordLayout::new(&REFERENCE_FIELDS, 8)
            .expect("valid record layout");
    let mut pointer_bytes = [0_u8; ROSTER_BATCH * 8];
    let mut pointers = [0_usize; ROSTER_BATCH];
    let mut handle_bytes = [0_u8; ROSTER_BATCH * 8];
    let mut first = 0_u32;
    while first < array.count {
        let count = ((array.count - first) as usize).min(ROSTER_BATCH);
        let batch_ok = array
            .slice(first, count as u32, 8)
            .ok()
            .is_some_and(|span| engine_native_memory::read_slice(memory, span.data, &mut pointer_bytes[..count * 8]));
        if !batch_ok {
            for ordinal in first..first + count as u32 {
                let pointer = array
                    .element(ordinal, 8)
                    .and_then(|at| engine_native_memory::read::<u64>(memory, at));
                let Some(pointer) = pointer else {
                    unobserved!();
                    continue;
                };
                let Some(handle) =
                    engine_native_memory::read::<u64>(memory, pointer as usize + organisation::REFERENCE)
                else {
                    unobserved!();
                    continue;
                };
                record_registered(&mut store, memory, base, pointer as usize, handle);
            }
            first += count as u32;
            continue;
        }
        for (slot, chunk) in pointers[..count]
            .iter_mut()
            .zip(pointer_bytes[..count * 8].chunks_exact(8))
        {
            *slot = u64::from_le_bytes(chunk.try_into().unwrap()) as usize;
        }
        let copied = memory.copy_records(
            engine_native::memory::RecordSources::Pointers(&pointers[..count]),
            &REFERENCE_LAYOUT,
            &mut handle_bytes[..count * 8],
        );
        for index in 0..copied {
            let handle = u64::from_le_bytes(handle_bytes[index * 8..(index + 1) * 8].try_into().unwrap());
            record_registered(&mut store, memory, base, pointers[index], handle);
        }
        if copied < count {
            for &pointer in &pointers[copied..count] {
                let Some(handle) =
                    engine_native::read::<u64>(memory, pointer + organisation::REFERENCE)
                else {
                    unobserved!();
                    continue;
                };
                record_registered(&mut store, memory, base, pointer, handle);
            }
        }
        first += count as u32;
    }
}
pub(super) fn refresh_context(
    memory: &impl Memory,
    base: usize,
    events: &[Event],
) -> Result<(), &'static str> {
    let mut handles = std::collections::HashSet::new();
    for event in events {
        if matches!(event.kind, Kind::Organisation(_)) {
            engine_allocation::reserve_set(&mut handles, 1)
                .map_err(|_| "variant author context allocation failed")?;
            handles.insert(event.variant);
        }
    }
    for handle in &handles {
        let resolved = engine_native_parts::creference_registry::resolve(memory, base, *handle)?;
        let designs = sources::author_designs(*handle)?;
        super::super::design_context::author(memory, base, *handle, resolved, &designs)?;
    }
    Ok(())
}
struct Scope {
    flight: collector_primitives::asymmetric::FlightGuard<'static>,
    complete: bool,
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.flight.mark_dirty();
        if !self.complete {
            unobserved!();
        }
    }
}
unsafe extern "system-unwind" fn callback<const SITE: usize>(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::VariantChanges);
    let mut scope = Scope {
        flight: DESIGN_GATE.enter(true),
        complete: false,
    };
    let base = BASE.load(Ordering::Acquire);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    if SITE == 3 {
        retired(&memory, base, a);
    }
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[SITE].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d) });
    if SITE == 1 {
        roster(
            &memory,
            base,
            a + engine_native::cproduction_status::ORGANISATIONS,
        );
    }
    if SITE == 2 {
        registered(&memory, base, a);
    }
    scope.complete = true;
    result
}
unsafe extern "system-unwind" fn reconcile(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::VariantChanges);
    let mut scope = Scope {
        flight: DESIGN_GATE.enter(true),
        complete: false,
    };
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d, e) });
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    roster(&memory, BASE.load(Ordering::Acquire), e);
    scope.complete = true;
    result
}
pub(super) struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let callbacks = [
            reconcile as *const () as usize,
            callback::<1> as *const () as usize,
            callback::<2> as *const () as usize,
            callback::<3> as *const () as usize,
        ];
        let mut hooks = Vec::new();
        if ORGANISATIONS.iter().all(|site| {
            engine_sites::hook(base, site.rva)
                .and_then(|code| code.sentinel::<16>())
                .is_ok()
        }) {
            for (index, site) in ORGANISATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook = unsafe {
                    Hook::new(code.address, callbacks[index], code.sentinel::<16>()?)
                }
                .map_err(|_| match index {
                    0 => "design-liveness ReconcileOrganisations hook construction failed",
                    1 => "design-liveness InitializeOrganisations hook construction failed",
                    2 => "design-liveness RepairOrganisationReference hook construction failed",
                    _ => "design-liveness DestroyOrganisation hook construction failed",
                })?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::Release);
        }
        let changed=Hook::set_family_enabled_shared(&mut self.hooks,enabled).map_err(|_|"design-liveness hook publication failed (ReconcileOrganisations, InitializeOrganisations, RepairOrganisationReference, DestroyOrganisation)")?;
        if changed {
            ENABLED.store(
                enabled && self.hooks.len() == ORGANISATIONS.len(),
                Ordering::Release,
            );
        }
        Ok(changed)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconciliation_preserves_the_owning_array_in_its_fifth_stack_argument() {
        unsafe extern "system-unwind" fn original(
            a: usize,
            b: usize,
            c: usize,
            d: usize,
            e: usize,
        ) -> usize {
            a ^ b ^ c ^ d ^ e
        }
        let previous = ORIGINAL[0].swap(original as *const () as usize, Ordering::AcqRel);
        assert_eq!(unsafe { reconcile(1, 2, 4, 8, 16) }, 31);
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(unsafe { reconcile(1, 2, 4, 8, 16) }, 31)
        });
        ORIGINAL[0].store(previous, Ordering::Release);
    }
}
