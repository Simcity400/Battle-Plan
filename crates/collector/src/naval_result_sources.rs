//! Native result lineage. Constructor witnesses distinguish a short persistent
//! combat from a stack-local strike; no identity is invented for the latter.
//! All callbacks are allocation-free and never wait for another caller.
use crate::hook::Hook;
type Clock = engine_native::cgame_state::Clock<u64>;
use engine_native::Memory;
use engine_naval_history::naval_result_sources as native;
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub game_hour: i32,
    pub result: u64,
    pub combat: u64,
    pub merged: bool,
    pub transient: bool,
    pub combat_created_hour: Option<i32>,
}
impl Entry {
    pub fn into_row(self) -> replay_data::rows::NavalResultSourceRow {
        replay_data::rows::NavalResultSourceRow {
            id: self.id,
            game_hour: self.game_hour,
            result: self.result,
            combat: self.combat,
            merged: self.merged,
            transient: self.transient,
            combat_created_hour: self.combat_created_hour,
        }
    }
}

#[derive(Clone, Copy)]
struct Birth {
    hour: i32,
    kind: native::BirthKind,
}
struct Store {
    entries: Vec<Entry>,
    births: HashMap<usize, Birth>,
    next_id: u64,
    partial: Option<&'static str>,
    refusal: Option<&'static str>,
}
impl Store {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            births: HashMap::new(),
            next_id: 1,
            partial: Some("naval result sources first/branch interval is partial"),
            refusal: None,
        }
    }
    fn refuse(&mut self, reason: &'static str) {
        self.refusal.get_or_insert(reason);
        REFUSALS.fetch_add(1, Ordering::Relaxed);
        poison();
    }
    fn birth(&mut self, pointer: usize, birth: Birth) -> Result<(), &'static str> {
        if !self.births.contains_key(&pointer) && self.births.len() == self.births.capacity() {
            OVERFLOW.fetch_add(1, Ordering::Relaxed);
            return Err("naval result sources birth storage overflow");
        }
        self.births.insert(pointer, birth); // pre-reserved, including hash buckets
        Ok(())
    }
    fn append(
        &mut self,
        pointer: usize,
        input: native::Inputs,
        merged: bool,
    ) -> Result<(), &'static str> {
        let birth = self.births.remove(&pointer);
        let transient = !input.registered;
        if transient
            && !birth.is_some_and(|b| {
                b.kind == native::BirthKind::Transient
                    && b.hour == input.game_hour
                    && (input.port_strike || input.naval_strike)
            })
        {
            return Err("naval result sources unregistered source without transient witness");
        }
        if !transient && birth.is_some_and(|b| b.kind != native::BirthKind::Persistent) {
            return Err("naval result sources registered source has conflicting birth witness");
        }
        if self.entries.len() == self.entries.capacity() {
            OVERFLOW.fetch_add(1, Ordering::Relaxed);
            return Err("naval result sources event storage overflow");
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or("naval result sources event id overflow")?;
        self.entries.push(Entry {
            id: self.next_id,
            game_hour: input.game_hour,
            result: input.result,
            combat: input.combat,
            merged,
            transient,
            combat_created_hour: birth.map(|b| b.hour),
        });
        self.next_id = next;
        EVENTS.fetch_add(1, Ordering::Relaxed);
        if self.entries.len() > HOURLY_PEAK.load(Ordering::Relaxed) {
            HOURLY_PEAK_CAPACITY.store(self.entries.capacity(), Ordering::Relaxed);
            HOURLY_PEAK.store(self.entries.len(), Ordering::Relaxed);
        }
        Ok(())
    }
}
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
fn store() -> &'static Mutex<Store> {
    STORE.get_or_init(|| Mutex::new(Store::new()))
}
static CLOCK: Mutex<Option<Clock>> = Mutex::new(None);
static WORLD_GENERATION: AtomicU64 = AtomicU64::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
static BIRTH_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static OFFERED_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static EVENT_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static BIRTH_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static EVENT_DEMAND: AtomicUsize = AtomicUsize::new(0);
static BIRTH_DEMAND: AtomicUsize = AtomicUsize::new(0);
static HOURLY_PEAK: AtomicUsize = AtomicUsize::new(0);
static HOURLY_PEAK_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static GAME_THREAD: AtomicUsize = AtomicUsize::new(0);
static CALL_THREADS: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static THREAD_MISMATCHES: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
static RECOVERY_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static ORIGINAL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static INSTALLED: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];
static SITE_ENABLED: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];
static CALLS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static EVENTS: AtomicU64 = AtomicU64::new(0);
static OVERFLOW: AtomicU64 = AtomicU64::new(0);
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicU64 = AtomicU64::new(0);
static OVERLAPS: AtomicU64 = AtomicU64::new(0);
static PREPARE_FAILURES: AtomicU64 = AtomicU64::new(0);
static POPULATION_READ_FAILURES: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_CONSTRUCTORS: AtomicU64 = AtomicU64::new(0);
static UNWINDS: AtomicU64 = AtomicU64::new(0);
static UNWOUND: AtomicBool = AtomicBool::new(false);
pub fn active() -> usize {
    ACTIVE.iter().map(|v| v.load(Ordering::SeqCst)).sum()
}
pub fn capacity() -> usize {
    CAPACITY.load(Ordering::Acquire)
}
pub(crate) fn demand() -> usize {
    EVENT_DEMAND
        .load(Ordering::Acquire)
        .max(BIRTH_DEMAND.load(Ordering::Acquire))
}
/// Loader only, after all relays, callbacks, and spool readers are quiet.
pub fn release() -> Result<(), &'static str> {
    if ENABLED.load(Ordering::SeqCst)
        || active() != 0
        || SITE_ENABLED.iter().any(|site| site.load(Ordering::SeqCst))
    {
        return Err("naval result source release before callback quiescence");
    }
    let retained = {
        let mut current = store().lock().unwrap_or_else(|poison| poison.into_inner());
        std::mem::replace(&mut *current, Store::new())
    };
    let storage = {
        let mut swap = STORAGE_SWAP
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::mem::replace(
            &mut *swap,
            StorageSwap {
                next: None,
                retired: None,
            },
        )
    };
    CAPACITY.store(0, Ordering::Release);
    BIRTH_CAPACITY.store(0, Ordering::Release);
    OFFERED_CAPACITY.store(0, Ordering::Release);
    RETIRED.store(false, Ordering::Release);
    *CLOCK.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    drop((retained, storage));
    Ok(())
}
/// Bootstrap only, before installing hooks, under the retained replay leases.
pub fn restore_sequence(previous: u64) -> Result<(), &'static str> {
    if ENABLED.load(Ordering::Acquire) || active() != 0 {
        return Err("naval result source sequence restoration requires disabled hooks");
    }
    let next = previous
        .checked_add(1)
        .ok_or("naval result source event sequence exhausted")?;
    let mut current = store()
        .lock()
        .map_err(|_| "naval result source sequence storage poisoned")?;
    current.next_id = current.next_id.max(next);
    Ok(())
}
fn poison() {
    POISONED.store(true, Ordering::SeqCst);
}
pub fn lost_drain() {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LOST.fetch_add(1, Ordering::Relaxed);
    RECOVERY_PENDING.store(true, Ordering::SeqCst);
    poison();
}

/// Attach/sizing thread only. Population and observed demand determine storage; there
/// is no country/event cap. A burst that outruns growth refuses observation.
pub fn prepare(memory: &impl Memory, base: usize) {
    collect_retired();
    match native::population(memory, base) {
        Ok(population) => presize(population),
        Err("naval source game state is null") => (),
        Err(_) => {
            POPULATION_READ_FAILURES.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct Storage {
    entries: Vec<Entry>,
    births: HashMap<usize, Birth>,
}
struct StorageSwap {
    next: Option<Storage>,
    retired: Option<Storage>,
}
static STORAGE_SWAP: Mutex<StorageSwap> = Mutex::new(StorageSwap {
    next: None,
    retired: None,
});
static RETIRED: AtomicBool = AtomicBool::new(false);

fn collect_retired() {
    if !RETIRED.load(Ordering::Acquire) {
        return;
    }
    let retired = if let Ok(mut swap) = STORAGE_SWAP.try_lock() {
        let retired = swap.retired.take();
        RETIRED.store(false, Ordering::Release);
        retired
    } else {
        return;
    };
    drop(retired); // loader/spool only
}

fn presize(population: usize) {
    engine_allocation::advisory(|| presize_storage(population));
}

fn presize_storage(population: usize) {
    EVENT_DEMAND.fetch_max(population, Ordering::Release);
    let requested = population
        .max(EVENT_DEMAND.load(Ordering::Relaxed))
        .max(BIRTH_DEMAND.load(Ordering::Relaxed));
    if (requested <= capacity() && requested <= BIRTH_CAPACITY.load(Ordering::Acquire))
        || requested <= OFFERED_CAPACITY.load(Ordering::Acquire)
    {
        return;
    }
    let mut entries = Vec::new();
    let mut births = HashMap::new();
    if engine_allocation::reserve_exact(&mut entries, requested).is_err()
        || engine_allocation::reserve_map(&mut births, requested).is_err()
    {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let Ok(mut swap) = STORAGE_SWAP.try_lock() else {
        return;
    };
    if requested <= OFFERED_CAPACITY.load(Ordering::Acquire) {
        return;
    }
    swap.next = Some(Storage { entries, births });
    OFFERED_CAPACITY.store(requested, Ordering::Release);
}

/// Startup/hourly thread only, while empty and writer-free. The spool never
/// locks STORE; failed offer adoption is harmless and leaves the current log.
pub fn install_offered() {
    if let Ok(mut current) = store().try_lock() {
        install_into(&mut current);
    }
}
fn install_into(current: &mut Store) {
    if active() != 0 || !current.entries.is_empty() || !current.births.is_empty() {
        return;
    }
    let Ok(mut swap) = STORAGE_SWAP.try_lock() else {
        return;
    };
    if swap.retired.is_some() {
        return;
    }
    let Some(next) = swap.next.take() else {
        return;
    };
    let entries = std::mem::replace(&mut current.entries, next.entries);
    let births = std::mem::replace(&mut current.births, next.births);
    BIRTH_CAPACITY.store(OFFERED_CAPACITY.load(Ordering::Acquire), Ordering::Release);
    CAPACITY.store(current.entries.capacity(), Ordering::Release);
    OFFERED_CAPACITY.store(0, Ordering::Release);
    swap.retired = Some(Storage { entries, births });
    RETIRED.store(true, Ordering::Release);
}
pub fn boundary(clock: Option<Clock>) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    GAME_THREAD.store(
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() } as usize,
        Ordering::Relaxed,
    );
    let Some(clock) = clock else {
        lost_drain();
        return;
    };
    let Ok(mut previous) = CLOCK.try_lock() else {
        lost_drain();
        return;
    };
    let transition = WORLD_GENERATION.swap(crate::world_lifetime::generation(), Ordering::SeqCst)
        != crate::world_lifetime::generation()
        || !previous.is_some_and(|before| {
            before.hour.checked_add(1) == Some(clock.hour)
                && before.campaign_id == clock.campaign_id
                && before.play_session_started_at == clock.play_session_started_at
        });
    *previous = Some(clock);
    drop(previous);
    let Ok(mut current) = store().try_lock() else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
        return;
    };
    // Only birth witnesses from the just-finished interval can prove that an
    // unobserved combat began after its previous read. Events own their copy.
    current.births.clear();
    BIRTH_ATTEMPTS.store(0, Ordering::Relaxed);
    if transition {
        current.entries.clear();
        EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
        current.refusal = None;
        current.partial = Some("naval result sources first/branch interval is partial");
        RECOVERY_PENDING.store(false, Ordering::SeqCst);
        UNWOUND.store(false, Ordering::SeqCst);
        POISONED.store(false, Ordering::SeqCst);
    } else if RECOVERY_PENDING.swap(false, Ordering::SeqCst) {
        current.entries.clear();
        EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
        current.refusal = None;
        current.partial = Some("naval result sources recovery interval is partial");
        UNWOUND.store(false, Ordering::SeqCst);
        POISONED.store(false, Ordering::SeqCst);
    } else {
        current.partial = None;
        RECOVERY_PENDING.store(POISONED.load(Ordering::SeqCst), Ordering::SeqCst);
    }
    if active() != 0 {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
    }
    install_into(&mut current);
}
#[cfg(test)]
pub fn drain(output: &mut [Entry]) -> Result<usize, &'static str> {
    drain_buffer(engine_native::walk::Buffer::Fixed(output))
}
pub fn drain_growing(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    drain_buffer(engine_native::walk::Buffer::Growing(output))
}
fn drain_buffer(mut output: engine_native::walk::Buffer<'_, Entry>) -> Result<usize, &'static str> {
    #[cfg(test)]
    if !tests::owns_observation() {
        return Err("naval result sources hooks unavailable (see status unhooked)");
    }
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("naval result sources hooks unavailable (see status unhooked)");
    }
    let Ok(mut current) = store().try_lock() else {
        lost_drain();
        return Err("naval result sources writer overlaps hourly drain");
    };
    if POISONED.load(Ordering::SeqCst) {
        RECOVERY_PENDING.store(true, Ordering::SeqCst);
        if UNWOUND.load(Ordering::SeqCst) {
            return Err("naval result sources native writer unwound");
        }
        return Err(current
            .refusal
            .unwrap_or("naval result sources lost observation; interval unavailable"));
    }
    if let Some(reason) = current.partial {
        return Err(reason);
    }
    if active() != 0 {
        lost_drain();
        return Err("naval result sources active writer at drain");
    }
    let len = current.entries.len();
    output
        .ensure(len)
        .map_err(|_| "naval result sources drain allocation failed")?;
    if len > output.len() {
        lost_drain();
        return Err("naval result sources drain storage too small");
    }
    output[..len].copy_from_slice(&current.entries);
    if POISONED.load(Ordering::SeqCst) || active() != 0 {
        lost_drain();
        return Err("naval result sources writer overlaps hourly drain");
    }
    current.entries.clear();
    EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
    install_into(&mut current);
    Ok(len)
}
struct Call {
    site: usize,
    completed: bool,
}
impl Call {
    fn enter(site: usize) -> Self {
        ACTIVE[site].fetch_add(1, Ordering::SeqCst);
        CALLS[site].fetch_add(1, Ordering::Relaxed);
        Self {
            site,
            completed: false,
        }
    }
    // Only state-accessing paths call this, after acquiring STORE.
    fn observe(&self) {
        let thread =
            unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() } as usize;
        CALL_THREADS[self.site].store(thread, Ordering::Relaxed);
        let expected = GAME_THREAD.load(Ordering::Relaxed);
        if expected != 0 && expected != thread {
            THREAD_MISMATCHES.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn returned(mut self, value: usize) -> usize {
        self.completed = true;
        value
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        if !self.completed {
            UNWINDS.fetch_add(1, Ordering::Relaxed);
            UNWOUND.store(true, Ordering::SeqCst);
            poison();
        }
        ACTIVE[self.site].fetch_sub(1, Ordering::SeqCst);
    }
}
// Count every reached append, including calls skipped after an interval loss.
// This pressure sizes only the producer's next offer and resets with its log.
fn attempt(attempts: &AtomicUsize, demand: &AtomicUsize) {
    if ENABLED.load(Ordering::SeqCst) {
        let required = attempts.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        demand.fetch_max(required, Ordering::Release);
    }
}
fn observing() -> bool {
    ENABLED.load(Ordering::SeqCst)
        && !crate::world_lifetime::invalidating()
        && !POISONED.load(Ordering::SeqCst)
        && !crate::allocation::failed()
}

// A frameless tail jump passes the native return PC without changing RCX or
// RSP. The Rust body has ordinary unwind metadata, as main's aircraft shim.
#[unsafe(naked)]
unsafe extern "system" fn constructor_entry() {
    core::arch::naked_asm!("mov rdx, [rsp]", "jmp {body}", body = sym constructor);
}
unsafe extern "system-unwind" fn constructor(pointer: usize, caller: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::NavalResultSources);
    let call = Call::enter(2);
    let original: unsafe extern "system-unwind" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    let run = || timing.original(|| unsafe { original(pointer) });
    let base = BASE.load(Ordering::Relaxed);
    let kind = native::constructor_caller(base, caller);
    if kind == native::BirthKind::Unknown {
        UNKNOWN_CONSTRUCTORS.fetch_add(1, Ordering::Relaxed);
    }
    if kind == native::BirthKind::Loaded {
        return call.returned(run());
    }
    if kind != native::BirthKind::Unknown {
        attempt(&BIRTH_ATTEMPTS, &BIRTH_DEMAND);
    }
    if !observing() {
        return call.returned(run());
    }
    if kind == native::BirthKind::Unknown {
        let returned = run();
        if let Ok(mut current) = store().try_lock() {
            call.observe();
            current.refuse("naval result sources unknown constructor caller");
        } else {
            OVERLAPS.fetch_add(1, Ordering::Relaxed);
            poison();
        }
        return call.returned(returned);
    }
    let Ok(mut current) = store().try_lock() else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
        return call.returned(run());
    };
    call.observe();
    // A fresh allocator can reuse a receiver within the interval.
    current.births.remove(&pointer);
    let generation = crate::world_lifetime::generation();
    let returned = run();
    if generation != crate::world_lifetime::generation() {
        current.refuse("naval result source native world changed during constructor");
        return call.returned(returned);
    }
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let result = native::creation_hour(&memory, base)
        .and_then(|hour| current.birth(pointer, Birth { hour, kind }));
    if let Err(reason) = result {
        current.refuse(reason);
    }
    call.returned(returned)
}
unsafe extern "system-unwind" fn fill(result: usize, combat: usize) -> usize {
    unsafe { result_call(0, result, combat) }
}
unsafe extern "system-unwind" fn merge(result: usize, combat: usize) -> usize {
    unsafe { result_call(1, result, combat) }
}
unsafe fn result_call(site: usize, result: usize, combat: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::NavalResultSources);
    let call = Call::enter(site);
    let original: unsafe extern "system-unwind" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    let run = || timing.original(|| unsafe { original(result, combat) });
    attempt(&EVENT_ATTEMPTS, &EVENT_DEMAND);
    if !observing() {
        return call.returned(run());
    }
    let Ok(mut current) = store().try_lock() else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
        return call.returned(run());
    };
    call.observe();
    let generation = crate::world_lifetime::generation();
    let returned = run();
    if generation != crate::world_lifetime::generation() {
        current.refuse("naval result source native world changed during original");
        return call.returned(returned);
    }
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let input = native::read_inputs(&memory, BASE.load(Ordering::Relaxed), result, combat);
    if let Err(reason) = input.and_then(|input| current.append(combat, input, site == 1)) {
        current.refuse(reason);
    }
    call.returned(returned)
}

/// The verified bytes a hook owns at its site, padded to the hook's 16-byte original.
fn owned_span(code: &engine_sites::Code, span: usize) -> Result<[u8; 16], &'static str> {
    let mut out = [0; 16];
    match span {
        16 => out = code.sentinel::<16>()?,
        13 => out[..13].copy_from_slice(&code.sentinel::<13>()?),
        _ => return Err("naval result sources site span unsupported"),
    }
    Ok(out)
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, Ordering::Relaxed);
        let _ = store();
        let callbacks = [
            fill as *const () as usize,
            merge as *const () as usize,
            constructor_entry as *const () as usize,
        ];
        let install = || -> Result<Vec<Hook>, &'static str> {
            engine_sites::require(base, &native::CONSTRUCTOR_CALLERS)?;
            let mut located = [(0, [0u8; 16]); 3];
            for (site, slot) in native::SITES.into_iter().zip(&mut located) {
                *slot = engine_sites::hook(base, site.rva)
                    .and_then(|code| Ok((code.address, owned_span(&code, site.span)?)))
                    .inspect_err(|cause| {
                        crate::hook::installation::refused(
                            crate::hook::installation::Kind::Sentinel,
                            None,
                            cause,
                        )
                    })?;
                crate::hook::installation::site(slot.0);
            }
            let mut targets = [0; 2];
            for (target, key) in targets.iter_mut().zip(native::TARGETS) {
                *target = engine_sites::hook(base, key)?.address;
            }
            let mut hooks = Vec::new();
            for (index, (site, (at, original))) in
                native::SITES.into_iter().zip(located).enumerate()
            {
                let hook = if index < 2 {
                    unsafe { Hook::new_call_span(at, callbacks[index], original, site.span) }
                } else {
                    unsafe { Hook::new(at, callbacks[index], original) }
                }
                .map_err(|_| "naval result sources hook construction failed")?;
                ORIGINAL[index].store(
                    if index < 2 {
                        targets[index]
                    } else {
                        hook.trampoline()
                    },
                    Ordering::Release,
                );
                hooks.push(hook);
            }
            Ok(hooks)
        };
        match install() {
            Ok(hooks) => {
                for installed in &INSTALLED {
                    installed.store(true, Ordering::Release);
                }
                Self { hooks }
            }
            Err(reason) => {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    reason,
                );
                let _ = UNHOOKED.set(reason);
                Self { hooks: Vec::new() }
            }
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        for (index, hook) in self.hooks.iter_mut().enumerate() {
            if !hook
                .set_enabled(enabled)
                .map_err(|_| "naval result sources hook state change failed")?
            {
                return Ok(false);
            }
            SITE_ENABLED[index].store(enabled, Ordering::SeqCst);
        }
        ENABLED.store(
            enabled && self.hooks.len() == native::SITES.len(),
            Ordering::SeqCst,
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
pub fn status() -> impl serde::Serialize {
    let hooks = std::array::from_fn::<_, 3, _>(|i| {
        let site = native::SITES[i];
        crate::status::status!({
            "name": site.name, "rva": site.rva.0, "span": site.span,
            "installed": INSTALLED[i].load(Ordering::Acquire),
            "enabled": SITE_ENABLED[i].load(Ordering::SeqCst),
            "active": ACTIVE[i].load(Ordering::SeqCst),
            "calls": CALLS[i].load(Ordering::Relaxed),
            "last_thread": CALL_THREADS[i].load(Ordering::Relaxed),
        })
    });
    crate::status::status!({
        "hooks": crate::status::sequence(hooks.into_iter()),
        "unhooked": UNHOOKED.get(),
        "interval_unavailable": POISONED.load(Ordering::SeqCst),
        "events": EVENTS.load(Ordering::Relaxed),
        "overflow": OVERFLOW.load(Ordering::Relaxed),
        "refusals": REFUSALS.load(Ordering::Relaxed),
        "lost_drains": LOST.load(Ordering::Relaxed),
        "overlaps": OVERLAPS.load(Ordering::Relaxed),
        "event_capacity": capacity(),
        "birth_capacity": BIRTH_CAPACITY.load(Ordering::Acquire),
        "offered_capacity": OFFERED_CAPACITY.load(Ordering::Acquire),
        "event_demand": EVENT_DEMAND.load(Ordering::Relaxed),
        "birth_demand": BIRTH_DEMAND.load(Ordering::Relaxed),
        "hourly_peak_events": HOURLY_PEAK.load(Ordering::Relaxed),
        "hourly_peak_capacity": HOURLY_PEAK_CAPACITY.load(Ordering::Relaxed),
        "game_thread": GAME_THREAD.load(Ordering::Relaxed),
        "thread_mismatches": THREAD_MISMATCHES.load(Ordering::Relaxed),
        "prepare_failures": PREPARE_FAILURES.load(Ordering::Relaxed),
        "population_read_failures": POPULATION_READ_FAILURES.load(Ordering::Relaxed),
        "unknown_constructors": UNKNOWN_CONSTRUCTORS.load(Ordering::Relaxed),
        "native_unwinds": UNWINDS.load(Ordering::Relaxed),
        "scope": "cumulative since attach; first/branch and recovery intervals partial; skipped result creation invokes neither result tap",
    })
}

#[cfg(test)]
pub(crate) mod tests;
