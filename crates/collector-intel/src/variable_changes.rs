//! Allocation-free notifications from the native writers of every script
//! variable store ([`engine::script_variable_mutations`]). A detour only
//! notifies the shared watch registry ([`WatchRegistry`]) for the store its
//! arguments name, counts its site and calls the original; it never changes
//! game behaviour. Receivers are the `CVariables` addresses a ScriptVariables
//! capture registered; a notification never infers a scope. Anything
//! unobserved (disabled, poisoned, a refused registration, an epoch change, a
//! running writer) makes the capture read that store afresh.
//!
//! The registry marks each notified store ([`WatchRegistry::marking`]), so a
//! capture finds the stores writers touched with one take an hour instead of
//! looking up every store. The release, which destroys a store, also moves
//! the domain's structure token, so a capture walks every scope after one.
//! The ends of the scripted contexts that may rewrite a store's seed notify
//! that store.
//!
//! An exception unwinding one of those contexts ends it unobserved: the
//! seven functions that begin one name the context-ending helper in their
//! C++ unwind maps. While the family observes, a vectored exception handler
//! moves the structure token at every first-chance C++ exception in the
//! Process  (`0xE06D7363`, the collector's own caught panics included), so the
//! next capture walks every scope and reads every seed. A seed-changing
//! context must begin on the main thread, enforced by an assertion. The
//! hourly callback parks that thread, so the raise and its unwinding both
//! fall on one side of a
//! capture's take. The handler never handles an exception.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine::{
    Memory,
    script_variable_mutations::{MUTATIONS, Patch, Receiver, store},
};
use std::{
    ffi::c_void,
    ops::Range,
    sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
/// The variable release site's index in [`MUTATIONS`].
const RELEASE: usize = 3;
const _: () = assert!(
    matches!(MUTATIONS[RELEASE].1, Receiver::Table),
    "the release names the store's table"
);
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read, verified and mismatched stores; reused and read rows;
/// quiet hours and the stores they visited.
static READS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried ScriptVariables rows differ from a fresh read",
]);
/// Stores the latest complete capture walked, for off-thread sizing.
static SCOPES: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched stores, the structure token and each
/// writer site's calls.
pub struct Watches {
    registry: WatchRegistry,
    /// Moves on entry to and return from every release, and at every C++
    /// exception.
    structure: AtomicU64,
    /// First-chance C++ exceptions the installed handler saw.
    exceptions: AtomicU64,
    calls: [AtomicU64; SITES],
    /// Context ends whose context did not read: each revoked every
    /// certificate.
    unread_contexts: AtomicU64,
    /// Tests: stands in for the roster domain ([`crate::roster_changes`]).
    #[cfg(any(test, feature = "test-support"))]
    roster: WatchRegistry,
}

/// A running release; its drop moves the structure token again before the
/// notification returns.
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

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `store`. Dropping the guard notifies its return.
    pub fn notify(&self, store: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(store)
    }

    /// Hook path, allocation-free: the release is about to destroy `store`.
    /// It moves the structure token while its notification runs, on entry
    /// and on return.
    pub fn release(&self, store: usize) -> Released<'_> {
        self.calls[RELEASE].fetch_add(1, Ordering::Relaxed);
        let notification = self.registry.notify(store);
        self.structure.fetch_add(1, Ordering::SeqCst);
        Released {
            structure: &self.structure,
            _notification: notification,
        }
    }

    /// Allocation-free, on any thread: a C++ exception was raised, and its
    /// unwinding may end a seed-changing context unobserved. Moving the
    /// structure token makes the next capture walk every scope.
    pub fn exception(&self) {
        self.exceptions.fetch_add(1, Ordering::Relaxed);
        self.structure.fetch_add(1, Ordering::SeqCst);
    }

    /// Hook path, allocation-free: a context end of `site` whose context did
    /// not read is about to run; no certificate survives it.
    fn unread_context(&self, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.unread_contexts.fetch_add(1, Ordering::Relaxed);
        self.registry.notify_every()
    }

    fn take_marks(&self, visit: &mut dyn FnMut(usize)) -> Option<Taken> {
        let structure = self.structure.load(Ordering::SeqCst);
        let epoch = self.registry.take(visit)?;
        (self.structure.load(Ordering::SeqCst) == structure).then_some(Taken { epoch, structure })
    }

    /// An enabled domain with a table for `stores` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(stores: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(stores);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches.roster.set_enabled(true);
        watches
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.roster.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }

    /// Tests: a roster writer runs and returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn roster_write(&self) {
        drop(self.roster.notify(0));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn roster_poisoned(&self) -> bool {
        self.roster.poisoned()
    }
}

/// The take before an hour's walk: the marks' epoch and the structure token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taken {
    pub epoch: u64,
    pub structure: u64,
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, sequence)` of one store, registering it, and whether this
    /// call registered it; `None` when it cannot be observed. Equal values
    /// across hours prove that no observed writer touched it since the
    /// first.
    fn register(&self, store: usize) -> Option<((u64, u64), bool)>;
    /// A fresh read disagreed with carried rows: disable reuse for the
    /// attachment.
    fn mismatch(&self);
    /// Before any read of the hour: visits every store a hooked writer
    /// touched since the previous take, clearing its mark, and returns the
    /// marks' epoch with the structure token. `None` when nothing taken
    /// certifies anything (observation off, a writer or release running, the
    /// epoch or structure moving during the take).
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<Taken>;
    /// The character roster's token ([`crate::roster_changes::roster`]):
    /// equal values prove the roster's occurrences, order and ids unchanged.
    fn roster(&self) -> Option<(u64, u64)>;
    /// A held scope list disagreed with a fresh walk: disable reuse of the
    /// variable and roster domains for the attachment.
    fn scope_mismatch(&self);
}

impl Changes for Watches {
    fn register(&self, store: usize) -> Option<((u64, u64), bool)> {
        self.registry.register(store)
    }
    fn mismatch(&self) {
        self.registry.poison();
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
    #[cfg(any(test, feature = "test-support"))]
    fn scope_mismatch(&self) {
        self.registry.poison();
        self.roster.poison();
    }
    #[cfg(not(any(test, feature = "test-support")))]
    fn scope_mismatch(&self) {
        self.registry.poison();
        crate::roster_changes::mismatch();
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn register(&self, store: usize) -> Option<((u64, u64), bool)> {
        WATCHES.register(store)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<Taken> {
        WATCHES.take_marks(visit)
    }
    fn roster(&self) -> Option<(u64, u64)> {
        crate::roster_changes::roster()
    }
    fn scope_mismatch(&self) {
        WATCHES.mismatch();
        crate::roster_changes::mismatch();
    }
}

/// Whether captures read every carryable store afresh and compare, rather
/// than the hour's sampled slice ([`crate::sampled_verify`]); such an hour
/// is never quiet, and one whose quiet key matched is compared whole with
/// the held walk.
pub const VERIFY: bool = cfg!(feature = "variable_verify");

/// Notifications and exception-handler calls running now.
pub fn active() -> usize {
    WATCHES.registry.active() + HANDLING.load(Ordering::SeqCst)
}

/// `EXCEPTION_RECORD.ExceptionCode` of an MSVC C++ throw
/// (`_CxxThrowException`, `'msc' | 0xE0000000`).
const CXX_EXCEPTION: u32 = 0xe06d_7363;
/// A vectored handler's verdict that passes the exception on unchanged.
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

/// The prefixes of `EXCEPTION_RECORD` (`ExceptionCode` first) and
/// `EXCEPTION_POINTERS` the handler reads.
#[repr(C)]
struct ExceptionRecord {
    code: u32,
}
#[repr(C)]
struct ExceptionPointers {
    record: *const ExceptionRecord,
    _context: *const c_void,
}

// https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-addvectoredexceptionhandler
// https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-removevectoredexceptionhandler
#[link(name = "kernel32")]
unsafe extern "system" {
    fn AddVectoredExceptionHandler(
        first: u32,
        handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32,
    ) -> *mut c_void;
    fn RemoveVectoredExceptionHandler(handle: *mut c_void) -> u32;
}

/// The installed handler's handle; null while none is installed.
static HANDLER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Handler calls running now: detach waits for them ([`active`]); the DLL
/// image's IP check covers the instructions around the count.
static HANDLING: AtomicUsize = AtomicUsize::new(0);
/// Installations the system refused: the family then observes nothing.
static HANDLER_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Every exception dispatched in the process, first, before any frame
/// handler; allocation-free and lock-free. A C++ exception moves the
/// structure token; nothing is ever handled.
unsafe extern "system" fn vectored(pointers: *mut ExceptionPointers) -> i32 {
    HANDLING.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the dispatcher passes its live exception pointers.
    let code = unsafe {
        pointers
            .as_ref()
            .and_then(|pointers| pointers.record.as_ref())
    }
    .map(|record| record.code);
    if code == Some(CXX_EXCEPTION) {
        WATCHES.exception();
    }
    HANDLING.fetch_sub(1, Ordering::SeqCst);
    EXCEPTION_CONTINUE_SEARCH
}

/// Installs the handler, first in the vectored chain; `false` when the
/// system refuses it.
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

/// Removes the installed handler; calls already running finish under
/// [`HANDLING`].
fn remove_handler() -> Result<(), &'static str> {
    let handle = HANDLER.load(Ordering::SeqCst);
    if handle.is_null() {
        return Ok(());
    }
    // SAFETY: the handle is the one installation returned.
    if unsafe { RemoveVectoredExceptionHandler(handle) } == 0 {
        return Err("variable exception handler removal failed");
    }
    HANDLER.store(std::ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

/// Off the game thread: size the watches for the stores the latest capture
/// walked. Nothing is offered before a capture walked them: a later larger
/// table waits for an invalidating hour, while the first installs at once.
pub fn presize() {
    let scopes = SCOPES.load(Ordering::Relaxed);
    if scopes != 0 {
        WATCHES.registry.offer(scopes);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's stores and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
    /// The capture published the held walk without walking a scope.
    pub quiet: bool,
    /// Stores a quiet hour visited: marked, sampled or unobserved.
    pub visited: u64,
}

/// A worker publishes one capture's counts and, when it captured every
/// scope, their number for sizing.
pub fn record(counts: &Counts, scopes: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        counts.reused_rows,
        counts.read_rows,
        u64::from(counts.quiet),
        counts.visited,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    if let Some(scopes) = scopes {
        SCOPES.store(scopes, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|((site, _, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let (taken, refused_takes) = WATCHES.registry.takes();
    crate::status::status!({
        "configured": cfg!(feature = "variable_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_stores": reads[0],
        "read_stores": reads[1],
        "verified_stores": reads[2],
        "mismatches": reads[3],
        "reused_rows": reads[4],
        "read_rows": reads[5],
        "quiet_hours": reads[6],
        "visited_stores": reads[7],
        "taken_stores": taken,
        "refused_takes": refused_takes,
        "structure_token": WATCHES.structure.load(Ordering::Relaxed),
        "cxx_exceptions": WATCHES.exceptions.load(Ordering::Relaxed),
        "exception_handler": !HANDLER.load(Ordering::Relaxed).is_null(),
        "exception_handler_failures": HANDLER_FAILURES.load(Ordering::Relaxed),
        "events": events,
        "unread_contexts": WATCHES.unread_contexts.load(Ordering::Relaxed),
        "unlocated_threads": UNLOCATED_THREADS.load(Ordering::Relaxed),
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a quiet hour (quiet_hours) publishes the held walk's rows and reads only the stores a hooked writer marked since the previous hour's take, its slice's stores and unobserved ones (visited_stores), when the take's epoch and structure token (every release and every first-chance C++ exception, cxx_exceptions, moves it), the character roster's token, the global root, the country census and the state slots all equal the held walk's; any other hour reads every scope's identity and every store's vtable, header and seed, and carries a store's entries and keys when it is the store held for the same scope with the same hash header and no hooked writer touched it since its read; when no release or exception moved the structure token since the held walk, a store no writer touched whose header or seed moved is a missed write; the handler (exception_handler, installed with the family, never handling an exception) moves the token so that the hour after an exception walks every scope; every hour also reads the carryable stores of its slice afresh (one in rotation_hours by scope kind and id; every one in verify mode, which also compares each hour whose quiet key matched whole with the held walk instead of carrying it), publishes the fresh rows and disables reuse at the first mismatch (sampled_*); events count notified calls, get_mutable_value and erase_variable calls on the calling thread's temporary store run without one, and a context end (end_*) notifies only a context that may change its store's seed (hook_timing counts every call)"
    })
}

/// The game's static TLS index, read at installation; `usize::MAX` before.
static TLS_INDEX: AtomicUsize = AtomicUsize::new(usize::MAX);
/// The variable-store clear function the callsites invoke.
static CLEAR: AtomicUsize = AtomicUsize::new(0);
/// The context-ending helper invoked by the context callsites.
static END_CONTEXT: AtomicUsize = AtomicUsize::new(0);
/// Threads whose temporary store could not be located: their calls all
/// notify.
static UNLOCATED_THREADS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// This thread's temporary store; zero until located, `usize::MAX` when
    /// it cannot be.
    static TEMPORARY: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Installed ntdll NtQueryInformationThread dispatches directly to its system
// call without user-mode allocation. ThreadBasicInformation (0) writes the
// caller's THREAD_BASIC_INFORMATION (x64: TebBaseAddress@8, 48 bytes).
// https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntqueryinformationthread
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

/// Hook path: the calling thread's temporary store
/// (thread-local variable store lookup: `[[TEB+0x58] + index * 8] +
/// 0x20`), located once per thread; `usize::MAX` when it cannot be.
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
    // SAFETY: the pseudo-handle names the calling thread; the buffer is the
    // documented structure's size.
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
    // SAFETY: the query succeeded, so it wrote the structure.
    let teb = unsafe { information.assume_init() }.teb;
    // SAFETY: guarded reads of the calling thread's own TEB and TLS array.
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let array = engine::read::<u64>(&memory, teb.checked_add(0x58)?)?;
    let slot = usize::try_from(array)
        .ok()?
        .checked_add(index.checked_mul(8)?)?;
    let block = usize::try_from(engine::read::<u64>(&memory, slot)?).ok()?;
    if block == 0 {
        return None;
    }
    block.checked_add(engine::script_variable_mutations::TEMPORARY_STORE)
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel or an unreadable TLS index leaves the family
    /// uninstalled; ScriptVariables then reads every store afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        use engine::script_variable_mutations::{CLEAR_RVA, END_CONTEXT_RVA, TLS_INDEX_RVA};
        let mut hooks = Vec::new();
        if cfg!(feature = "variable_changes")
            && let Some(index) = engine::read::<u32>(memory, base + TLS_INDEX_RVA)
            && MUTATIONS.iter().all(|(site, _, _)| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            TLS_INDEX.store(index as usize, Ordering::Release);
            CLEAR.store(base + CLEAR_RVA, Ordering::Release);
            END_CONTEXT.store(base + END_CONTEXT_RVA, Ordering::Release);
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
                let hook = match patch {
                    Patch::Entry => unsafe {
                        Hook::new(base + site.rva, callbacks[index], site.sentinel)
                    },
                    Patch::Call | Patch::EndContext => unsafe {
                        Hook::new_call(base + site.rva, callbacks[index], site.sentinel)
                    },
                }
                .map_err(|_| "native hook construction failed")?;
                if *patch == Patch::Entry {
                    ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                }
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.registry.set_enabled(false);
            remove_handler()?;
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        // The exception handler is live before observation starts; without
        // it nothing is observed and every store is read afresh.
        let observing = enabled && !self.hooks.is_empty() && install_handler();
        WATCHES.registry.set_enabled(observing);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // Normally already removed with the family.
        let _ = remove_handler();
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

// Each entry site takes at most four integer or pointer arguments (RCX..R9)
// and returns RAX unchanged from its original; the first two name the store
// as its [`engine::script_variable_mutations::Receiver`] says. The
// notification covers the original in flight. A `$filter` site also serves
// every thread's temporary store, which no scope records: a call on the
// calling thread's own temporary store runs the original without one.
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
            let _notification = WATCHES.notify(written, $site);
            timing.original(|| unsafe { original($first, $second $(, $arg)*) })
        }
    };
}
// Get mutable value (variable, store, scope, fallback).
callback!(
    get_mutable_value,
    0,
    true,
    (variable, store_, scope, fallback)
);
// Erase variable (store, key).
callback!(erase_variable, 1, true, (store_, key));
// Read variables (store, reader).
callback!(read_variables, 2, false, (store_, reader));

// The release (store + 0x10); RDX passes through unused. It moves
// the structure token around the original as well.
unsafe extern "system" fn release_variables(table: usize, unused: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::VariableChanges);
    let written = store(MUTATIONS[RELEASE].1, table, unused);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[RELEASE].load(Ordering::Acquire)) };
    let _released = WATCHES.release(written);
    timing.original(|| unsafe { original(table, unused) })
}

// A replaced CALL of the clear (store): the detour calls the clear
// itself, which returns to the site's next instruction through the relay.
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
            let _notification = WATCHES.notify(store_, $site);
            timing.original(|| unsafe { clear(store_, second, third, fourth) })
        }
    };
}
// Country appearance reset clears its variables.
call_site!(reset_country_variables, 4);
// State reset clears its variables.
call_site!(reset_state_variables, 5);
// Runtime state reset clears global variables.
call_site!(reset_global_variables, 6);

// A replaced context-ending call: the detour reads the context passed by
// the call, as the original does, and notifies the store whose seed it may
// rewrite around the original. A context that cannot change a seed runs
// the original without a notification; an unreadable context revokes every
// certificate.
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
            // SAFETY: a guarded read of the context the native call passes.
            let memory = unsafe { engine::memory::LiveMemory::new() };
            let _notification =
                match engine::script_variable_mutations::context_store(&memory, context) {
                    Ok(Some(written)) => Some(WATCHES.notify(written, $site)),
                    Ok(None) => None,
                    Err(_) => Some(WATCHES.unread_context($site)),
                };
            timing.original(|| unsafe { end(context, second, third, fourth) })
        }
    };
}
// The scripted-effect console command ends two seed-changing contexts.
context_site!(end_effect_command, 7);
context_site!(end_effect_command_run, 8);
// Validated effect execution ends its seed-changing context.
context_site!(end_effect_execute, 9);
// Daily country update ends its seed-changing context.
context_site!(end_country_daily, 10);
// Delayed country events end their seed-changing context.
context_site!(end_delayed_events, 11);
// Daily state update ends its seed-changing context.
context_site!(end_state_daily, 12);
// The scripted window effect ends its seed-changing context.
context_site!(end_scripted_window, 13);
// Event effect execution ends its seed-changing context.
context_site!(end_fire_event, 14);

#[cfg(test)]
mod tests {
    use super::*;

    /// The handler moves the hooks' structure token and counts the
    /// exception only for a C++ throw, and passes every exception on.
    #[test]
    fn the_vectored_handler_moves_the_structure_only_for_a_cxx_exception() {
        let call = |code: u32| {
            let record = ExceptionRecord { code };
            let mut pointers = ExceptionPointers {
                record: &record,
                _context: std::ptr::null(),
            };
            // SAFETY: the pointers name a live record.
            unsafe { vectored(&mut pointers) }
        };
        let token = || {
            (
                WATCHES.structure.load(Ordering::SeqCst),
                WATCHES.exceptions.load(Ordering::SeqCst),
            )
        };
        let before = token();
        // An access violation, as a guarded read's.
        assert_eq!(call(0xc000_0005), EXCEPTION_CONTINUE_SEARCH);
        assert_eq!(token(), before);
        assert_eq!(call(CXX_EXCEPTION), EXCEPTION_CONTINUE_SEARCH);
        assert_eq!(token(), (before.0 + 1, before.1 + 1));
        assert_eq!(HANDLING.load(Ordering::SeqCst), 0);
        // SAFETY: a null record pointer reads nothing.
        let empty = unsafe { vectored(std::ptr::null_mut()) };
        assert_eq!(empty, EXCEPTION_CONTINUE_SEARCH);
        assert_eq!(token(), (before.0 + 1, before.1 + 1));
    }

    /// An exception moves the structure token the next take returns, and
    /// leaves the marks' epoch and every certificate alone.
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
