use crate::{
    hook::{Hook, Tap},
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{
    script_variable_mutations::MUTATIONS, script_variable_mutations::Patch,
    script_variable_mutations::Receiver, script_variable_mutations::store,
};
use engine_native_memory::Memory;
use std::{
    ffi::c_void,
    ops::Range,
    sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
const RELEASE: usize = 3;
const _: () = assert!(
    matches!(MUTATIONS[RELEASE].1, Receiver::Table),
    "the release names the store's table"
);
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static SCOPES: AtomicUsize = AtomicUsize::new(0);
static EMBEDDED_SCOPES: AtomicUsize = AtomicUsize::new(0);
static FACTION_SCOPES: AtomicUsize = AtomicUsize::new(0);

pub struct Watches {
    registry: WatchRegistry,
    structure: AtomicU64,
    exceptions: AtomicU64,
    calls: [AtomicU64; SITES],
    unread_contexts: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    roster: WatchRegistry,
}

pub struct Released<'a> {
    structure: &'a AtomicU64,
    _notification: Notification<'a>,
}

impl Drop for Released<'_> {
    fn drop(&mut self) {
        self.structure.fetch_add(1, Ordering::SeqCst);
    }
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub fn embedded_quiet(&self) -> Option<(u64, u64, u64)> {
        let structure = self.structure.load(Ordering::SeqCst);
        let (epoch, sequence) = self.registry.quiet()?;
        (structure == self.structure.load(Ordering::SeqCst)).then_some((epoch, sequence, structure))
    }

    pub fn embedded_sequence(&self, store: usize) -> Option<(u64, u64, u64)> {
        let structure = self.structure.load(Ordering::SeqCst);
        let (epoch, sequence) = self.registry.sequence(store)?;
        (structure == self.structure.load(Ordering::SeqCst)).then_some((epoch, sequence, structure))
    }
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::marking(),
            structure: AtomicU64::new(0),
            exceptions: AtomicU64::new(0),
            calls: [const { AtomicU64::new(0) }; SITES],
            unread_contexts: AtomicU64::new(0),
            #[cfg(any(test, feature = "test-support"))]
            roster: WatchRegistry::new(),
        }
    }

    pub fn notify(&self, store: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(store)
    }

    pub fn release(&self, store: usize) -> Released<'_> {
        self.calls[RELEASE].fetch_add(1, Ordering::Relaxed);
        let notification = self.registry.notify(store);
        self.structure.fetch_add(1, Ordering::SeqCst);
        Released {
            structure: &self.structure,
            _notification: notification,
        }
    }

    pub fn exception(&self) {
        self.exceptions.fetch_add(1, Ordering::Relaxed);
        self.structure.fetch_add(1, Ordering::SeqCst);
        collector_observers::project_variable_changes::invalidate();
    }

    fn unread_context(&self, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.unread_contexts.fetch_add(1, Ordering::Relaxed);
        self.registry.notify_every()
    }

    fn embedded_certificate(&self, store: usize) -> Option<(u64, u64, u64)> {
        let structure = self.structure.load(Ordering::SeqCst);
        let ((epoch, sequence), _) = self.registry.register(store)?;
        (self.structure.load(Ordering::SeqCst) == structure).then_some((epoch, sequence, structure))
    }

    fn take_marks(&self, visit: &mut dyn FnMut(usize)) -> Option<Taken> {
        let structure = self.structure.load(Ordering::SeqCst);
        let epoch = self.registry.take(visit)?;
        (self.structure.load(Ordering::SeqCst) == structure).then_some(Taken { epoch, structure })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(stores: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(stores);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches.roster.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.roster.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn roster_write(&self) {
        drop(self.roster.notify(0));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taken {
    pub epoch: u64,
    pub structure: u64,
}

pub trait Changes: Sync {
    fn register(&self, store: usize) -> Option<((u64, u64), bool)>;
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<Taken>;
    fn roster(&self) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn register(&self, store: usize) -> Option<((u64, u64), bool)> {
        self.registry.register(store)
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<Taken> {
        self.take_marks(visit)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn roster(&self) -> Option<(u64, u64)> {
        self.roster.quiet()
    }
    #[cfg(not(any(test, feature = "test-support")))]
    fn roster(&self) -> Option<(u64, u64)> {
        crate::roster_changes::roster()
    }
}

pub struct Observer;
impl Observer {
    pub fn embedded_quiet(&self) -> Option<(u64, u64, u64)> {
        WATCHES.embedded_quiet()
    }
    pub fn embedded_sequence(&self, store: usize) -> Option<(u64, u64, u64)> {
        WATCHES.embedded_sequence(store)
    }
    pub fn demand_embedded(&self, count: usize) {
        FACTION_SCOPES.fetch_max(count, Ordering::Relaxed);
    }
}
impl Changes for Observer {
    fn register(&self, store: usize) -> Option<((u64, u64), bool)> {
        WATCHES.register(store)
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<Taken> {
        WATCHES.take_marks(visit)
    }
    fn roster(&self) -> Option<(u64, u64)> {
        crate::roster_changes::roster()
    }
}

pub fn embedded_certificate(store: usize) -> Option<(u64, u64, u64)> {
    WATCHES.embedded_certificate(store)
}

pub fn active() -> usize {
    WATCHES.registry.active() + HANDLING.load(Ordering::SeqCst)
}

const CXX_EXCEPTION: u32 = 0xe06d_7363;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

#[repr(C)]
struct ExceptionRecord {
    code: u32,
}
#[repr(C)]
struct ExceptionPointers {
    record: *const ExceptionRecord,
    _context: *const c_void,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn AddVectoredExceptionHandler(
        first: u32,
        handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32,
    ) -> *mut c_void;
    fn RemoveVectoredExceptionHandler(handle: *mut c_void) -> u32;
}

static HANDLER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static HANDLING: AtomicUsize = AtomicUsize::new(0);
static HANDLER_FAILURES: AtomicU64 = AtomicU64::new(0);

unsafe extern "system" fn vectored(pointers: *mut ExceptionPointers) -> i32 {
    HANDLING.fetch_add(1, Ordering::SeqCst);
    let code = unsafe {
        pointers
            .as_ref()
            .and_then(|pointers| pointers.record.as_ref())
    }
    .map(|record| record.code);
    if code == Some(CXX_EXCEPTION) {
        WATCHES.exception();
        collector_observers::market_changes::variables_uncertified();
    }
    HANDLING.fetch_sub(1, Ordering::SeqCst);
    EXCEPTION_CONTINUE_SEARCH
}

fn install_handler() -> bool {
    if !HANDLER.load(Ordering::SeqCst).is_null() {
        return true;
    }
    // SAFETY: `vectored` is a function of this DLL, removed before the DLL
    // unloads ([`remove_handler`]), and only reads what the dispatcher
    // passes.
    let handle = unsafe { AddVectoredExceptionHandler(1, vectored) };
    if handle.is_null() {
        HANDLER_FAILURES.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    HANDLER.store(handle, Ordering::SeqCst);
    true
}

fn remove_handler() -> Result<(), &'static str> {
    let handle = HANDLER.load(Ordering::SeqCst);
    if handle.is_null() {
        return Ok(());
    }
    if unsafe { RemoveVectoredExceptionHandler(handle) } == 0 {
        return Err("variable exception handler removal failed");
    }
    HANDLER.store(std::ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

pub fn offer_scopes(scopes: usize) {
    SCOPES.fetch_max(scopes, Ordering::Relaxed);
    presize();
}

pub fn offer_embedded(scopes: usize) {
    EMBEDDED_SCOPES.fetch_max(scopes, Ordering::Relaxed);
    presize();
}

pub fn presize() {
    presize_watches(
        &WATCHES.registry,
        SCOPES.load(Ordering::Relaxed),
        EMBEDDED_SCOPES.load(Ordering::Relaxed),
        FACTION_SCOPES.load(Ordering::Relaxed),
    );
}

fn presize_watches(registry: &WatchRegistry, scopes: usize, embedded: usize, factions: usize) {
    let Some(scopes) = scopes
        .checked_add(embedded)
        .and_then(|scopes| scopes.checked_add(factions))
    else {
        let _ = engine_allocation::oom();
        return;
    };
    if scopes != 0 {
        registry.offer(scopes);
    }
}

pub fn collect() {
    WATCHES.registry.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
    pub quiet: bool,
    pub visited: u64,
}

pub fn record(counts: &Counts, scopes: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.reused_rows,
        counts.read_rows,
        u64::from(counts.quiet),
        counts.visited,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(scopes) = scopes {
        SCOPES.store(scopes, Ordering::Relaxed);
    }
}


static TLS_INDEX: AtomicUsize = AtomicUsize::new(usize::MAX);
static CLEAR: AtomicUsize = AtomicUsize::new(0);
static END_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static UNLOCATED_THREADS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static TEMPORARY: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationThread(
        thread: windows_sys::Win32::Foundation::HANDLE,
        class: u32,
        information: *mut std::ffi::c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}

#[inline(always)]
fn temporary() -> usize {
    let cached = TEMPORARY.get();
    if cached != 0 {
        return cached;
    }
    let store = locate_temporary().unwrap_or_else(|| {
        UNLOCATED_THREADS.fetch_add(1, Ordering::Relaxed);
        usize::MAX
    });
    TEMPORARY.set(store);
    store
}

#[cold]
#[inline(never)]
fn locate_temporary() -> Option<usize> {
    #[repr(C)]
    struct ThreadBasicInformation {
        exit_status: i32,
        teb: usize,
        client: [usize; 2],
        affinity: usize,
        priority: i32,
        base_priority: i32,
    }
    let index = TLS_INDEX.load(Ordering::Acquire);
    if index == usize::MAX {
        return None;
    }
    let mut information = std::mem::MaybeUninit::<ThreadBasicInformation>::zeroed();
    let status = unsafe {
        NtQueryInformationThread(
            windows_sys::Win32::System::Threading::GetCurrentThread(),
            0,
            information.as_mut_ptr().cast(),
            size_of::<ThreadBasicInformation>() as u32,
            std::ptr::null_mut(),
        )
    };
    if status < 0 {
        return None;
    }
    let teb = unsafe { information.assume_init() }.teb;
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let array = engine_native_memory::read::<u64>(&memory, teb.checked_add(0x58)?)?;
    let slot = usize::try_from(array)
        .ok()?
        .checked_add(index.checked_mul(8)?)?;
    let block = usize::try_from(engine_native_memory::read::<u64>(&memory, slot)?).ok()?;
    if block == 0 {
        return None;
    }
    block.checked_add(engine_mutations::script_variable_mutations::TEMPORARY_STORE)
}

static DELAYED_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
unsafe extern "system-unwind" fn delayed_pass(country: usize) -> usize {
    type Original = unsafe extern "system-unwind" fn(usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(DELAYED_ORIGINAL.load(Ordering::Acquire)) };
    collector_observers::script_event_observer::delayed_pass(country, || unsafe {
        original(country)
    })
}
pub struct Hooks {
    hooks: Vec<Hook>,
    taps: Vec<Tap>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A withheld site or an unreadable TLS index leaves the family
    /// uninstalled; ScriptVariables then reads every store afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, String> {
        use engine_mutations::{
            script_variable_mutations::CLEAR_RVA, script_variable_mutations::END_CONTEXT_RVA,
            script_variable_mutations::TLS_INDEX_RVA,
        };
        let mut hooks = Vec::new();
        if cfg!(feature = "variable_changes")
            && let Ok(tls_index) = engine_sites::resolve(base, TLS_INDEX_RVA)
            && let Some(index) = engine_native_memory::read::<u32>(memory, tls_index)
            && let Ok(clear) = engine_sites::resolve(base, CLEAR_RVA)
            && let Ok(end_context) = engine_sites::resolve(base, END_CONTEXT_RVA)
            && MUTATIONS
                .iter()
                .all(|(site, _, _)| crate::located(base, site.rva).is_ok())
        {
            TLS_INDEX.store(index as usize, Ordering::Release);
            CLEAR.store(clear, Ordering::Release);
            END_CONTEXT.store(end_context, Ordering::Release);
            let callbacks: [usize; SITES] = [
                get_mutable_value as *const () as usize,
                erase_variable as *const () as usize,
                read_variables as *const () as usize,
                release_variables as *const () as usize,
                reset_country_variables as *const () as usize,
                reset_state_variables as *const () as usize,
                reset_global_variables as *const () as usize,
                end_effect_command as *const () as usize,
                end_effect_command_run as *const () as usize,
                end_effect_execute as *const () as usize,
                end_country_daily as *const () as usize,
                end_delayed_events as *const () as usize,
                end_state_daily as *const () as usize,
                end_scripted_window as *const () as usize,
                end_fire_event as *const () as usize,
            ];
            for (index, (site, _, patch)) in MUTATIONS.iter().enumerate() {
                let code =
                    engine_sites::hook(base, site.rva).map_err(|_| site_failure(site.name))?;
                let original = code.sentinel::<16>().map_err(|_| site_failure(site.name))?;
                let hook = match patch {
                    Patch::Entry => unsafe { Hook::new(code.address, callbacks[index], original) },
                    Patch::Call | Patch::EndContext => unsafe {
                        Hook::new_call(code.address, callbacks[index], original)
                    },
                }
                .map_err(|_| site_failure(site.name))?
                .named(site_failure(site.name));
                if *patch == Patch::Entry {
                    ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                }
                hooks.push(hook);
            }
        }
        if cfg!(feature = "variable_changes") && hooks.is_empty() {
            crate::hook::installation::refused(
                crate::hook::installation::Kind::Construction,
                None,
                "script variable hook TLS index or native sites unavailable",
            );
        }
        let site = engine_events::script_event_sites::DELAYED_PASS;
        let mut taps = Vec::new();
        if !hooks.is_empty() {
            if crate::located(base, site.rva).is_err() {
                return Err(site.failure().into());
            }
            let code = engine_sites::hook(base, site.rva).map_err(|_| site.failure())?;
            let hook = unsafe {
                Hook::new(
                    code.address,
                    delayed_pass as *const () as usize,
                    code.sentinel::<16>().map_err(|_| site.failure())?,
                )
            }
            .map_err(|_| site.failure())?
            .named(site.failure());
            DELAYED_ORIGINAL.store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
            taps = unsafe {
                collector_observers::script_event_observer::delayed_pass_taps(memory, base)
            }?;
        }
        Ok(Self { hooks, taps })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        collector_observers::project_variable_changes::enabled(false);
        if !enabled && collector_observers::script_event_observer::active() != 0 {
            return Ok(false);
        }
        if !enabled {
            collector_observers::event_feed_observer::forwarder(
                collector_observers::event_feed_observer::Forwarder::DelayedPass,
                false,
            );
            WATCHES.registry.set_enabled(false);
            collector_observers::market_changes::observe_variables(false);
            remove_handler()?;
        }
        if !Hook::set_enabled_iter(
            self.hooks
                .iter_mut()
                .chain(self.taps.iter_mut().map(Tap::publication_hook)),
            enabled,
        )
        .map_err(|_| {
            self.hooks
                .iter_mut()
                .chain(self.taps.iter_mut().map(Tap::publication_hook))
                .find_map(|hook| hook.snapshot_failure())
                .unwrap_or("Script variables / delayed pass publication failed")
        })? {
            return Ok(false);
        }
        let observing = enabled && !self.hooks.is_empty() && install_handler();
        collector_observers::project_variable_changes::enabled(observing);
        WATCHES.registry.set_enabled(observing);
        collector_observers::market_changes::observe_variables(observing);
        collector_observers::event_feed_observer::forwarder(
            collector_observers::event_feed_observer::Forwarder::DelayedPass,
            enabled && !self.taps.is_empty(),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.taps.iter().flat_map(Tap::executable_ranges))
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        let _ = remove_handler();
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

macro_rules! callback {
    ($name:ident, $site:expr, $filter:expr, ($first:ident, $second:ident $(, $arg:ident)*)) => {
        unsafe extern "system" fn $name($first: usize, $second: usize $(, $arg: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::VariableChanges);
            let written = store(MUTATIONS[$site].1, $first, $second);
            let original: unsafe extern "system" fn($first: usize, $second: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            if $filter && written == temporary() {
                return timing.original(|| unsafe { original($first, $second $(, $arg)*) });
            }
            let _market=collector_observers::market_changes::variable_write(written);
            let _notification = WATCHES.notify(written, $site);
            let _project = collector_observers::project_variable_changes::writing(written);
            timing.original(|| unsafe { original($first, $second $(, $arg)*) })
        }
    };
}
callback!(
    get_mutable_value,
    0,
    true,
    (variable, store_, scope, fallback)
);
callback!(erase_variable, 1, true, (store_, key));
callback!(read_variables, 2, false, (store_, reader));

unsafe extern "system" fn release_variables(table: usize, unused: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::VariableChanges);
    let written = store(MUTATIONS[RELEASE].1, table, unused);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[RELEASE].load(Ordering::Acquire)) };
    let _market = collector_observers::market_changes::variable_write(written);
    let _released = WATCHES.release(written);
    let _project = collector_observers::project_variable_changes::writing(written);
    timing.original(|| unsafe { original(table, unused) })
}

macro_rules! call_site {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            store_: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::VariableChanges);
            let clear: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(CLEAR.load(Ordering::Acquire)) };
            let _market = collector_observers::market_changes::variable_write(store_);
            let _notification = WATCHES.notify(store_, $site);
            let _project = collector_observers::project_variable_changes::writing(store_);
            timing.original(|| unsafe { clear(store_, second, third, fourth) })
        }
    };
}
call_site!(reset_country_variables, 4);
call_site!(reset_state_variables, 5);
call_site!(reset_global_variables, 6);

macro_rules! context_site {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            context: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::VariableChanges);
            let end: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(END_CONTEXT.load(Ordering::Acquire)) };
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            let written =
                engine_mutations::script_variable_mutations::context_store(&memory, context);
            let _market = match written {
                Ok(Some(at)) => Some(collector_observers::market_changes::variable_write(at)),
                Ok(None) => None,
                Err(_) => {
                    collector_observers::market_changes::variables_uncertified();
                    None
                }
            };
            let _project = match written {
                Ok(Some(written)) => Some(collector_observers::project_variable_changes::writing(
                    written,
                )),
                Ok(None) => None,
                Err(_) => {
                    collector_observers::project_variable_changes::invalidate();
                    None
                }
            };
            let _notification = match written {
                Ok(Some(written)) => Some(WATCHES.notify(written, $site)),
                Ok(None) => None,
                Err(_) => Some(WATCHES.unread_context($site)),
            };
            timing.original(|| unsafe { end(context, second, third, fourth) })
        }
    };
}
context_site!(end_effect_command, 7);
context_site!(end_effect_command_run, 8);
context_site!(end_effect_execute, 9);
context_site!(end_country_daily, 10);
context_site!(end_delayed_events, 11);
context_site!(end_state_daily, 12);
context_site!(end_scripted_window, 13);
context_site!(end_fire_event, 14);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_scope_and_disjoint_embedded_demands_prepare_the_first_watch_table() {
        let watches = Watches::new();
        presize_watches(&watches.registry, 0, 3_000, 0);
        presize_watches(&watches.registry, 5_000, 3_000, 1_500);
        assert!(watches.registry.boundary(false).installed());
        watches.registry.set_enabled(true);
        assert_eq!(watches.registry.capacity(), 65_536);
        let stores: Vec<_> = (1..=9_500).map(|index| index * 0x100).collect();
        let certificates: Vec<_> = stores
            .iter()
            .map(|&store| {
                watches
                    .embedded_sequence(store)
                    .expect("initial scope registered")
            })
            .collect();
        let quiet = watches.embedded_quiet().unwrap();
        assert!(!watches.registry.boundary(false).installed());
        assert_eq!(watches.embedded_quiet(), Some(quiet));
        for (&store, &certificate) in stores.iter().zip(&certificates) {
            assert_eq!(watches.embedded_sequence(store), Some(certificate));
        }
        let changed = stores[5_000 + 3_000];
        drop(watches.notify(changed, 0));
        assert_ne!(
            watches.embedded_sequence(changed),
            Some(certificates[8_000])
        );
        assert_eq!(watches.embedded_sequence(stores[0]), Some(certificates[0]));
        let mut marked = Vec::new();
        watches.take(&mut |store| marked.push(store)).unwrap();
        assert_eq!(marked, [changed]);
    }

    #[test]
    fn embedded_certificates_do_not_consume_the_scope_dirty_stream_and_follow_lifetime() {
        let watches = Watches::enabled(16);
        let before = watches.embedded_sequence(8).unwrap();
        let other = watches.embedded_sequence(16).unwrap();
        drop(watches.notify(8, 0));
        let changed = watches.embedded_sequence(8).unwrap();
        assert_ne!(before, changed);
        assert_eq!(other, watches.embedded_sequence(16).unwrap());
        let mut marked = Vec::new();
        watches.take(&mut |store| marked.push(store)).unwrap();
        assert_eq!(marked, [8]);
        assert_eq!(changed, watches.embedded_sequence(8).unwrap());
        let quiet = watches.embedded_quiet().unwrap();
        watches
            .take(&mut |_| panic!("marks already drained"))
            .unwrap();
        assert_eq!(quiet, watches.embedded_quiet().unwrap());
        drop(watches.release(16));
        assert_ne!(changed, watches.embedded_sequence(8).unwrap());
        let before = watches.embedded_sequence(8).unwrap();
        watches.exception();
        assert_ne!(before, watches.embedded_sequence(8).unwrap());
    }

    #[test]
    fn the_vectored_handler_moves_the_structure_only_for_a_cxx_exception() {
        let call = |code: u32| {
            let record = ExceptionRecord { code };
            let mut pointers = ExceptionPointers {
                record: &record,
                _context: std::ptr::null(),
            };
            unsafe { vectored(&mut pointers) }
        };
        let token = || {
            (
                WATCHES.structure.load(Ordering::SeqCst),
                WATCHES.exceptions.load(Ordering::SeqCst),
            )
        };
        let before = token();
        assert_eq!(call(0xc000_0005), EXCEPTION_CONTINUE_SEARCH);
        assert_eq!(token(), before);
        assert_eq!(call(CXX_EXCEPTION), EXCEPTION_CONTINUE_SEARCH);
        assert_eq!(token(), (before.0 + 1, before.1 + 1));
        assert_eq!(HANDLING.load(Ordering::SeqCst), 0);
        let empty = unsafe { vectored(std::ptr::null_mut()) };
        assert_eq!(empty, EXCEPTION_CONTINUE_SEARCH);
        assert_eq!(token(), (before.0 + 1, before.1 + 1));
    }

    #[test]
    fn an_exception_moves_the_takes_structure_token() {
        let watches = Watches::enabled(16);
        let certificate = watches.register(8);
        let before = watches.take(&mut |_| {}).expect("observing");
        watches.exception();
        let after = watches.take(&mut |_| {}).expect("observing");
        assert_eq!(after.epoch, before.epoch);
        assert_ne!(after.structure, before.structure);
        assert_eq!(
            watches.register(8).map(|(certificate, _)| certificate),
            certificate.map(|(certificate, _)| certificate)
        );
    }
}

#[cfg(test)]
mod embedded_tests {
    use super::*;
    #[test]
    fn embedded_certificates_do_not_consume_marks_and_cover_release_exception_and_reset() {
        let watches = Watches::enabled(8);
        let store = 0x1230;
        let initial = watches.embedded_certificate(store).unwrap();
        let writer = watches.notify(store, 0);
        assert!(watches.embedded_certificate(store).is_none());
        drop(writer);
        let changed = watches.embedded_certificate(store).unwrap();
        assert_ne!(changed, initial);
        let mut marks = Vec::new();
        watches.take_marks(&mut |at| marks.push(at)).unwrap();
        assert!(marks.contains(&store));
        assert_eq!(watches.embedded_certificate(store), Some(changed));
        drop(watches.release(store));
        let released = watches.embedded_certificate(store).unwrap();
        assert_ne!(released, changed);
        watches.exception();
        let exception = watches.embedded_certificate(store).unwrap();
        assert_ne!(exception, released);
        watches.registry.boundary(true);
        assert_ne!(watches.embedded_certificate(store), Some(exception));
    }
}

fn site_failure(name: &str) -> &'static str {
    match name {
        "get_mutable_value" => "Script variables / get_mutable_value hook failed",
        "erase_variable" => "Script variables / erase_variable hook failed",
        "read_variables" => "Script variables / read_variables hook failed",
        "release_variables" => "Script variables / release_variables hook failed",
        "reset_country_variables" => "Script variables / reset_country_variables hook failed",
        "reset_state_variables" => "Script variables / reset_state_variables hook failed",
        "reset_global_variables" => "Script variables / reset_global_variables hook failed",
        "end_effect_command" => "Script variables / end_effect_command hook failed",
        "end_effect_command_run" => "Script variables / end_effect_command_run hook failed",
        "end_effect_execute" => "Script variables / end_effect_execute hook failed",
        "end_country_daily" => "Script variables / end_country_daily hook failed",
        "end_delayed_events" => "Script variables / end_delayed_events hook failed",
        "end_state_daily" => "Script variables / end_state_daily hook failed",
        "end_scripted_window" => "Script variables / end_scripted_window hook failed",
        "end_fire_event" => "Script variables / end_fire_event hook failed",
        _ => "Script variables / delayed pass hook failed",
    }
}
