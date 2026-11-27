use super::*;
use collector_hooks::hook::Hook;
use native::source_mutations::{BONUS_BODY, IDEA_SOURCES, MUTATIONS};
use std::ops::Range;
static ORIGINAL: [AtomicUsize; MUTATIONS.len()] = [const { AtomicUsize::new(0) }; MUTATIONS.len()];
static ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTRY_ENABLED: AtomicBool = AtomicBool::new(false);
static CONTENT_ENABLED: AtomicBool = AtomicBool::new(false);
fn content_enabled(enabled: bool) {
    CONTENT_ENABLED.store(enabled, Ordering::Release);
    super::update_enabled();
}
/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn content(
    _writer: collector_certificates::content_changes::Writer,
    [a, b, c, d]: [usize; 4],
    original: collector_certificates::content_changes::Original,
) -> usize {
    let mut scope = Scope {
        flight: DESIGN_GATE.enter(true),
        complete: false,
    };
    let forwarded = unsafe { forward::content_invalidation() };
    super::boundary(true);
    let result = unsafe { original(a, b, c, d) };
    if let Some(forwarded) = forwarded {
        unsafe { forwarded.finish() };
    }
    scope.complete = true;
    result
}
static CONTENT_FORWARDER: collector_certificates::content_changes::Forwarder =
    collector_certificates::content_changes::Forwarder {
        call: content,
        enabled: content_enabled,
    };

fn country_enabled(enabled: bool) {
    COUNTRY_ENABLED.store(enabled, Ordering::Release);
    super::update_enabled();
    if !enabled {
        unobserved!();
    }
}
/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn country(
    [a, b, c, d]: [usize; 4],
    original: collector_observers::country_input_changes::DesignSourceOriginal,
) -> usize {
    let mut scope = Scope {
        flight: DESIGN_GATE.enter(true),
        complete: false,
    };
    let forwarded = unsafe { forward::country_invalidation(a) };
    let result = unsafe { original(a, b, c, d) };
    if let Some(forwarded) = forwarded {
        unsafe { forwarded.finish() };
    }
    scope.complete = true;
    result
}
static COUNTRY_FORWARDER: collector_observers::country_input_changes::DesignSourceForwarder =
    collector_observers::country_input_changes::DesignSourceForwarder {
        call: country,
        enabled: country_enabled,
    };

pub(super) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
        && COUNTRY_ENABLED.load(Ordering::Acquire)
        && CONTENT_ENABLED.load(Ordering::Acquire)
}
fn clock(memory: &impl Memory, base: usize) -> Result<i32, &'static str> {
    let state = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .ok_or("design source clock unreadable")?;
    engine_native_memory::read(
        memory,
        state as usize + engine_native::cgame_state::CURRENT_HOUR,
    )
    .ok_or("design source hour unreadable")
}
fn direct(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(), &'static str> {
    let size = native::bonus_source_size(memory, base, pointer)?;
    let header = take(log, memory, pointer, size)?;
    array(log, memory, header + native::BONUS_STATS, 16)?;
    array(log, memory, header + native::BONUS_TERRAIN, 8)?;
    if size > native::BONUS_IDEA {
        let idea = u64_at(log, header + native::BONUS_IDEA) as usize;
        if idea != 0 {
            let header = take(log, memory, idea, engine_native_parts::cidea::TOKEN + 32)?;
            string(log, memory, header + engine_native_parts::cidea::TOKEN)?;
        }
    }
    Ok(())
}
fn provider(log: &mut RawLog<1>, memory: &impl Memory, pointer: usize) -> Result<(), &'static str> {
    let header = take(log, memory, pointer + native::COUNTRY_NAMED_BONUSES, 16)?;
    let (records, count) = array(log, memory, header, native::NAMED_BONUS_STRIDE)?;
    for ordinal in 0..count as usize {
        let record = records + ordinal * native::NAMED_BONUS_STRIDE;
        string(log, memory, record + native::NAMED_BONUS_NAME)?;
        string(log, memory, record + native::NAMED_BONUS_DESCRIPTION)?;
        array(log, memory, record + 8 + native::BONUS_STATS, 16)?;
        array(log, memory, record + 8 + native::BONUS_TERRAIN, 8)?;
    }
    Ok(())
}
pub(super) static WRITES: AtomicU64 = AtomicU64::new(0);
pub(super) static NOTES: AtomicU64 = AtomicU64::new(0);
fn observe_locked(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    key: sources::Key,
    retired: bool,
    cached_hour: &mut Option<i32>,
) {
    let Some(owner) = store.interests.changed(key) else {
        return;
    };
    NOTES.fetch_add(1, Ordering::Relaxed);
    if let Err(reason) = store.batch.room() {
        store.batch.failure = Some(reason);
        return;
    }
    let (pointer, kind) = match key {
        sources::Key::Direct(pointer) => (
            pointer,
            if retired {
                Kind::SourceRemoved
            } else {
                Kind::Source
            },
        ),
        sources::Key::Provider(pointer) => (
            pointer,
            if retired {
                Kind::ProviderReset(owner)
            } else {
                Kind::Provider(owner)
            },
        ),
    };
    let hour = match *cached_hour {
        Some(hour) => Ok(hour),
        None => clock(memory, base).inspect(|&hour| *cached_hour = Some(hour)),
    };
    match hour {
        Ok(hour) => {
            store.batch.events.push(Event {
                kind,
                sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
                hour,
                pointer,
                variant: 0,
                removed: false,
                start: 0,
                end: 0,
            });
        }
        Err(reason) => {
            store.batch.failure = Some(reason);
        }
    }
}
pub(super) fn observe(memory: &impl Memory, base: usize, key: sources::Key, retired: bool) {
    WRITES.fetch_add(1, Ordering::Relaxed);
    if matches!(key, sources::Key::Direct(0) | sources::Key::Provider(0)) {
        unknown_receiver!();
        return;
    }
    let Some(store) = STORE.get() else {
        return;
    };
    let mut store = callback_store(store);
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return;
    }
    let mut cached_hour = None;
    observe_locked(&mut store, memory, base, key, retired, &mut cached_hour);
}
pub(super) fn materialize(
    batch: &mut Batch,
    memory: &impl Memory,
    base: usize,
) -> Result<(), &'static str> {
    let key = |event: &Event| match event.kind {
        Kind::Source | Kind::SourceRemoved => Some((false, event.pointer)),
        Kind::Provider(_) | Kind::ProviderReset(_) => Some((true, event.pointer)),
        _ => None,
    };
    let mut last = HashMap::new();
    engine_allocation::reserve_map(&mut last, batch.events.len())
        .map_err(|_| "design source note allocation failed")?;
    for (index, event) in batch.events.iter().enumerate() {
        if let Some(key) = key(event) {
            last.insert(key, index);
        }
    }
    let mut index = 0;
    batch.events.retain(|event| {
        let keep = !matches!(event.kind, Kind::Source | Kind::Provider(_))
            || key(event).and_then(|key| last.get(&key)) == Some(&index);
        index += 1;
        keep
    });
    let mut log = std::mem::replace(&mut batch.log, RawLog::new(0, 0));
    log.clear();
    let copied = (|| {
        for event in &mut batch.events {
            let start = log.spans.len();
            match event.kind {
                Kind::Source => direct(&mut log, memory, base, event.pointer)?,
                Kind::Provider(owner) => {
                    let state = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
                        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
                        .ok_or("design provider world unreadable")? as usize;
                    let countries = engine_native::ccountry::Reader::new(memory, base, state)?;
                    let country = countries.resolve(owner)?;
                    if engine_native::cproduction_status::manager(memory, base, country)?
                        != event.pointer
                    {
                        return Err("design provider address reused outside its owner lifetime");
                    }
                    provider(&mut log, memory, event.pointer)?;
                }
                _ => continue,
            }
            (event.start, event.end) = (start, log.spans.len());
        }
        Ok(())
    })();
    copied?;
    batch.log = log;
    Ok(())
}
pub(super) fn reset_provider(memory: &impl Memory, base: usize, pointer: usize) {
    observe(memory, base, sources::Key::Provider(pointer), true);
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
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[SITE].load(Ordering::Acquire)) };
    unsafe { around::<SITE>(a, b, c, d, || original(a, b, c, d)) }
}
unsafe extern "system-unwind" fn append(a: usize, b: usize, c: usize, d: usize, e: usize) -> usize {
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[4].load(Ordering::Acquire)) };
    unsafe { around::<4>(a, b, c, d, || original(a, b, c, d, e)) }
}
/// # Safety
/// `original` invokes the matching native original with its ABI and the arguments it captured,
/// and the receiver `a` stays valid for the reads `SITE` makes.
unsafe fn around<const SITE: usize>(
    a: usize,
    _b: usize,
    _c: usize,
    _d: usize,
    original: impl FnOnce() -> usize,
) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::VariantChanges);
    let mut scope = Scope {
        flight: DESIGN_GATE.enter(true),
        complete: false,
    };
    let base = BASE.load(Ordering::Acquire);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let source = (SITE <= 1 || SITE == 3)
        .then(|| a.checked_sub(BONUS_BODY))
        .flatten();
    let retired = SITE == 3;
    if retired && let Some(source) = source {
        observe(&memory, base, sources::Key::Direct(source), true);
    }
    let provider = SITE >= 4;
    let forwarded = unsafe {
        if SITE == 2 {
            forward::idea_invalidation(a)
        } else {
            forward::invalidation(provider, if provider { a } else { source.unwrap_or(a) })
        }
    };
    let result = timing.original(original);
    if !retired {
        if let Some(source) = source {
            observe(&memory, base, sources::Key::Direct(source), false);
        } else if provider {
            observe(&memory, base, sources::Key::Provider(a), false);
        } else if SITE == 2 && SOURCE_BASELINE.load(Ordering::Acquire) {
            if let Some(roster) = Array::read(&memory, a + IDEA_SOURCES, 8) {
                if roster.count != 0 {
                    let mut store_guard = STORE.get().map(callback_store);
                    let current_epoch = EPOCH.load(Ordering::Acquire);
                    let mut cached_hour = None;
                    let mut handle_pointer = |pointer: usize| {
                        WRITES.fetch_add(1, Ordering::Relaxed);
                        if pointer == 0 {
                            unknown_receiver!();
                            return;
                        }
                        if let Some(store) = store_guard.as_mut()
                            && store.epoch == current_epoch
                        {
                            observe_locked(
                                store,
                                &memory,
                                base,
                                sources::Key::Direct(pointer),
                                false,
                                &mut cached_hour,
                            );
                        }
                    };
                    const SOURCE_BATCH: usize = 64;
                    let mut bytes = [0_u8; SOURCE_BATCH * 8];
                    let mut first = 0_u32;
                    while first < roster.count {
                        let count = ((roster.count - first) as usize).min(SOURCE_BATCH);
                        let batch_ok = roster
                            .slice(first, count as u32, 8)
                            .ok()
                            .is_some_and(|span| {
                                engine_native_memory::read_slice(
                                    &memory,
                                    span.data,
                                    &mut bytes[..count * 8],
                                )
                            });
                        if batch_ok {
                            for chunk in bytes[..count * 8].chunks_exact(8) {
                                let pointer = u64::from_le_bytes(chunk.try_into().unwrap()) as usize;
                                handle_pointer(pointer);
                            }
                        } else {
                            for ordinal in first..first + count as u32 {
                                if let Some(pointer) = roster
                                    .element(ordinal, 8)
                                    .and_then(|at| engine_native_memory::read::<u64>(&memory, at))
                                {
                                    handle_pointer(pointer as usize);
                                } else {
                                    unobserved!();
                                }
                            }
                        }
                        first += count as u32;
                    }
                }
            } else {
                unobserved!();
            }
        }
    }
    if let Some(forwarded) = forwarded {
        unsafe { forwarded.finish() };
    }
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
            callback::<0> as *const () as usize,
            callback::<1> as *const () as usize,
            callback::<2> as *const () as usize,
            callback::<3> as *const () as usize,
            append as *const () as usize,
            callback::<5> as *const () as usize,
        ];
        let mut hooks = Vec::new();
        if MUTATIONS.iter().all(|site| {
            engine_sites::hook(base, site.rva)
                .and_then(|code| code.sentinel::<16>())
                .is_ok()
        }) {
            for (index, site) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| match index {
                            0 => "design-source ReadAppliedSource hook construction failed",
                            1 => "design-source ResetAppliedSource hook construction failed",
                            2 => "design-source PostprocessIdeaSources hook construction failed",
                            3 => "design-source DestroyAppliedSource hook construction failed",
                            4 => "design-source AppendNamedSource hook construction failed",
                            _ => "design-source RemoveNamedSource hook construction failed",
                        })?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        collector_observers::country_input_changes::design_source_forwarder(Some(
            &COUNTRY_FORWARDER,
        ));
        collector_certificates::content_changes::forwarder(Some(&CONTENT_FORWARDER));
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::Release);
        }
        let changed = Hook::set_family_enabled_shared(&mut self.hooks,enabled).map_err(|_| "design-source hook publication failed (ReadAppliedSource, ResetAppliedSource, PostprocessIdeaSources, DestroyAppliedSource, AppendNamedSource, RemoveNamedSource)")?;
        if changed {
            ENABLED.store(
                enabled && self.hooks.len() == MUTATIONS.len(),
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
    fn named_append_preserves_its_fifth_stack_argument_and_return() {
        unsafe extern "system-unwind" fn original(
            a: usize,
            b: usize,
            c: usize,
            d: usize,
            e: usize,
        ) -> usize {
            a ^ b ^ c ^ d ^ e
        }
        let previous = ORIGINAL[4].swap(original as *const () as usize, Ordering::AcqRel);
        assert_eq!(unsafe { append(1, 2, 4, 8, 16) }, 31);
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(unsafe { append(1, 2, 4, 8, 16) }, 31)
        });
        ORIGINAL[4].store(previous, Ordering::Release);
    }
}
