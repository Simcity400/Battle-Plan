use collector_hooks::hook::Hook;
type Clock = engine_native::cgame_state::Clock<u64>;
use collector_primitives::spill_stack::SpillStack;
use engine_native_memory::Memory;
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

const STORAGE_FAILED: &str = "naval result sources storage allocation failed";

#[derive(Clone, Copy)]
struct Birth {
    hour: i32,
    kind: native::BirthKind,
}

enum Event {
    Birth {
        pointer: usize,
        birth: Result<Birth, &'static str>,
    },
    Result {
        combat: usize,
        input: Result<native::Inputs, &'static str>,
        merged: bool,
    },
    Boundary {
        transition: bool,
    },
    Lost,
}
static LOG: SpillStack<Event> = SpillStack::new();

fn log(event: Event) {
    if LOG.push(event).is_err() {
        STORAGE_REFUSALS.fetch_add(1, Ordering::SeqCst);
    }
}

struct Store {
    entries: Vec<Entry>,
    births: HashMap<usize, Birth>,
    next_id: u64,
    partial: Option<&'static str>,
    refusal: Option<&'static str>,
    poisoned: bool,
    recovery: bool,
    unwound: bool,
    charged: (u64, u64),
}
impl Store {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            births: HashMap::new(),
            next_id: 1,
            partial: Some("naval result sources first/branch interval is partial"),
            refusal: None,
            poisoned: false,
            recovery: false,
            unwound: false,
            charged: hook_failures(),
        }
    }
    fn refuse(&mut self, reason: &'static str) {
        self.refusal.get_or_insert(reason);
        REFUSALS.fetch_add(1, Ordering::Relaxed);
        self.poisoned = true;
    }
    fn fold(&mut self) {
        let before = hook_failures();
        let taken = LOG.take();
        let after = hook_failures();
        for event in taken {
            self.apply(event);
        }
        if after.0 > self.charged.0 {
            self.unwound = true;
            self.poisoned = true;
        }
        if after.1 > self.charged.1 {
            self.refuse(STORAGE_FAILED);
        }
        self.charged = before;
    }
    fn apply(&mut self, event: Event) {
        match event {
            Event::Boundary { transition } => self.boundary(transition),
            Event::Lost => {
                self.recovery = true;
                self.poisoned = true;
            }
            _ if self.poisoned => {}
            Event::Birth { pointer, birth } => {
                self.births.remove(&pointer);
                if let Err(reason) = birth.and_then(|birth| self.birth(pointer, birth)) {
                    self.refuse(reason);
                }
            }
            Event::Result {
                combat,
                input,
                merged,
            } => {
                if let Err(reason) = input.and_then(|input| self.append(combat, input, merged)) {
                    self.refuse(reason);
                }
            }
        }
    }
    fn boundary(&mut self, transition: bool) {
        self.births.clear();
        if transition {
            self.entries.clear();
            EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
            self.refusal = None;
            self.partial = Some("naval result sources first/branch interval is partial");
            self.recovery = false;
            self.unwound = false;
            self.poisoned = false;
        } else if std::mem::take(&mut self.recovery) {
            self.entries.clear();
            EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
            self.refusal = None;
            self.partial = Some("naval result sources recovery interval is partial");
            self.unwound = false;
            self.poisoned = false;
        } else {
            self.partial = None;
            self.recovery = self.poisoned;
        }
    }
    fn birth(&mut self, pointer: usize, birth: Birth) -> Result<(), &'static str> {
        if !self.births.contains_key(&pointer) && self.births.len() == self.births.capacity() {
            engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.births, 1))
                .map_err(|_| STORAGE_FAILED)?;
        }
        self.births.insert(pointer, birth);
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
            engine_allocation::advisory(|| engine_allocation::reserve(&mut self.entries, 1))
                .map_err(|_| STORAGE_FAILED)?;
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
static ACTIVE: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static ORIGINAL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static INSTALLED: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];
static SITE_ENABLED: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];
static CALLS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static EVENTS: AtomicU64 = AtomicU64::new(0);
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicU64 = AtomicU64::new(0);
static STORAGE_REFUSALS: AtomicU64 = AtomicU64::new(0);
static POPULATION_READ_FAILURES: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_CONSTRUCTORS: AtomicU64 = AtomicU64::new(0);
static UNWINDS: AtomicU64 = AtomicU64::new(0);
fn hook_failures() -> (u64, u64) {
    (
        UNWINDS.load(Ordering::SeqCst),
        STORAGE_REFUSALS.load(Ordering::SeqCst),
    )
}
pub fn active() -> usize {
    ACTIVE.iter().map(|v| v.load(Ordering::SeqCst)).sum()
}
pub fn demand() -> usize {
    EVENT_DEMAND
        .load(Ordering::Acquire)
        .max(BIRTH_DEMAND.load(Ordering::Acquire))
}
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
    *CLOCK.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    drop((retained, LOG.take()));
    Ok(())
}
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
pub fn lost_drain() {
    LOST.fetch_add(1, Ordering::Relaxed);
    log(Event::Lost);
}

pub fn prepare(memory: &impl Memory, base: usize) {
    match native::population(memory, base) {
        Ok(population) => {
            EVENT_DEMAND.fetch_max(population, Ordering::Release);
        }
        Err("naval source game state is null") => (),
        Err(_) => {
            POPULATION_READ_FAILURES.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn boundary(clock: Option<Clock>) {
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
    let transition = WORLD_GENERATION.swap(
        collector_lifetime::world_lifetime::generation(),
        Ordering::SeqCst,
    ) != collector_lifetime::world_lifetime::generation()
        || !previous.is_some_and(|before| {
            before.hour.checked_add(1) == Some(clock.hour)
                && before.campaign_id == clock.campaign_id
                && before.play_session_started_at == clock.play_session_started_at
        });
    *previous = Some(clock);
    drop(previous);
    BIRTH_ATTEMPTS.store(0, Ordering::Relaxed);
    log(Event::Boundary { transition });
}
pub fn drain(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    drain_buffer(engine_native_memory::walk::Buffer::Growing(output))
}
fn drain_buffer(mut output: engine_native_memory::walk::Buffer<'_, Entry>) -> Result<usize, &'static str> {
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("naval result sources hooks unavailable (see status unhooked)");
    }
    let mut current = store().lock().unwrap_or_else(|poison| poison.into_inner());
    current.fold();
    if current.poisoned {
        current.recovery = true;
        if current.unwound {
            return Err("naval result sources native writer unwound");
        }
        return Err(current
            .refusal
            .unwrap_or("naval result sources lost observation; interval unavailable"));
    }
    if let Some(reason) = current.partial {
        return Err(reason);
    }
    let len = current.entries.len();
    output
        .ensure(len)
        .map_err(|_| "naval result sources drain allocation failed")?;
    if len > output.len() {
        drop(current);
        lost_drain();
        return Err("naval result sources drain storage too small");
    }
    output[..len].copy_from_slice(&current.entries);
    current.entries.clear();
    EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
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
            UNWINDS.fetch_add(1, Ordering::SeqCst);
        }
        ACTIVE[self.site].fetch_sub(1, Ordering::SeqCst);
    }
}
fn attempt(attempts: &AtomicUsize, demand: &AtomicUsize) {
    if ENABLED.load(Ordering::SeqCst) {
        let required = attempts.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        demand.fetch_max(required, Ordering::Release);
    }
}
fn observing() -> bool {
    ENABLED.load(Ordering::SeqCst)
        && !collector_lifetime::world_lifetime::invalidating()
        && !collector_memory::terminal::failed()
}

#[unsafe(naked)]
unsafe extern "system" fn constructor_entry() {
    core::arch::naked_asm!("mov rdx, [rsp]", "jmp {body}", body = sym constructor);
}
unsafe extern "system-unwind" fn constructor(pointer: usize, caller: usize) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::NavalResultSources,
    );
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
    let generation = collector_lifetime::world_lifetime::generation();
    let returned = run();
    call.observe();
    let birth = if kind == native::BirthKind::Unknown {
        Err("naval result sources unknown constructor caller")
    } else if generation != collector_lifetime::world_lifetime::generation() {
        Err("naval result source native world changed during constructor")
    } else {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        native::creation_hour(&memory, base).map(|hour| Birth { hour, kind })
    };
    log(Event::Birth { pointer, birth });
    call.returned(returned)
}
unsafe extern "system-unwind" fn fill(result: usize, combat: usize) -> usize {
    unsafe { result_call(0, result, combat) }
}
unsafe extern "system-unwind" fn merge(result: usize, combat: usize) -> usize {
    unsafe { result_call(1, result, combat) }
}
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn result_call(site: usize, result: usize, combat: usize) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::NavalResultSources,
    );
    let call = Call::enter(site);
    let original: unsafe extern "system-unwind" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    let run = || timing.original(|| unsafe { original(result, combat) });
    attempt(&EVENT_ATTEMPTS, &EVENT_DEMAND);
    if !observing() {
        return call.returned(run());
    }
    let generation = collector_lifetime::world_lifetime::generation();
    let returned = run();
    call.observe();
    let input = if generation != collector_lifetime::world_lifetime::generation() {
        Err("naval result source native world changed during original")
    } else {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        native::read_inputs(&memory, BASE.load(Ordering::Relaxed), result, combat)
    };
    log(Event::Result {
        combat,
        input,
        merged: site == 1,
    });
    call.returned(returned)
}

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
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
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
                        collector_hooks::hook::installation::refused(
                            collector_hooks::hook::installation::Kind::Sentinel,
                            None,
                            cause,
                        )
                    })?;
                collector_hooks::hook::installation::site(slot.0);
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
                collector_hooks::hook::installation::refused(
                    collector_hooks::hook::installation::Kind::Construction,
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

#[cfg(any(test, feature = "test-support"))]
pub mod tests;
