//! Retour owns ordinary hook relocation. Verified patches are published while
//! existing process threads are suspended, without heap use in that interval.
//! The hooked site is the hourly update, the once-per-hour full tick on the
//! main thread.

use retour::RawDetour;
use std::{
    ffi::c_void,
    io,
    mem::{ManuallyDrop, size_of},
    ops::Range,
    ptr,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_BAD_LENGTH, ERROR_INSUFFICIENT_BUFFER,
        ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, ERROR_NOT_ENOUGH_MEMORY, ERROR_RETRY,
        GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    },
    System::{
        Diagnostics::{
            Debug::{
                CONTEXT, CONTEXT_CONTROL_AMD64, FlushInstructionCache, GetThreadContext,
                IMAGE_RUNTIME_FUNCTION_ENTRY, IMAGE_RUNTIME_FUNCTION_ENTRY_0, RtlAddFunctionTable,
                RtlDeleteFunctionTable,
            },
            ToolHelp::*,
        },
        Memory::*,
        Threading::*,
    },
};

pub mod installation;

/// An initial enable refused before writing any byte and with no owned member.
pub const BYTE_VERIFICATION_FAILURE: &str = "hook bytes differ from verified snapshot";

/// Covers a counted callback's native target and its uncounted entry/exit.
pub struct TapSafety {
    pub in_flight: &'static AtomicUsize,
    pub stub: Range<usize>,
}

pub struct Hook {
    relocation: ManuallyDrop<RawDetour>,
    relay: *mut c_void,
    site: usize,
    original: [u8; 16],
    sentinel_len: usize,
    patch: [u8; 16],
    patch_len: usize,
    enabled: bool,
    needs_flush: bool,
    diagnostic_failure: bool,
    page_protection: u32,
    tap: Option<TapSafety>,
    name: &'static str,
    publication_busy: AtomicBool,
}

impl Hook {
    /// # Safety
    /// Caller verified the executable, all 16 sentinel bytes, and that no
    /// native CALL return address falls in the five patched bytes: unwind
    /// epilogue decoding must never see our jump at a caller's return PC.
    pub unsafe fn new(site: usize, callback: usize, original: [u8; 16]) -> Result<Self, String> {
        unsafe { Self::new_span(site, callback, original, original.len()) }
    }

    /// Bind the native flight counter and complete callback range before publication.
    /// The owner must retain both on native escape and quiesce the DLL before unload.
    ///
    /// # Safety
    /// The requirements of [`Self::new`] apply. Retain the flight counter and stub until native calls have quiesced before unloading the DLL.
    pub unsafe fn new_with_safety(
        site: usize,
        callback: usize,
        original: [u8; 16],
        safety: TapSafety,
    ) -> Result<Self, String> {
        installation::construct(site, || {
            if safety.stub.is_empty() || !safety.stub.contains(&callback) {
                return Err("hook safety stub must contain its callback".into());
            }
            let mut hook = unsafe { Self::new(site, callback, original) }?;
            hook.tap = Some(safety);
            Ok(hook)
        })
    }

    unsafe fn new_span(
        site: usize,
        callback: usize,
        original: [u8; 16],
        sentinel_len: usize,
    ) -> Result<Self, String> {
        installation::construct(site, || {
            if !(5..=original.len()).contains(&sentinel_len) {
                return Err("hook sentinel must cover the patch within its storage".into());
            }
            if site.checked_add(sentinel_len).is_none() {
                return Err("hook sentinel address overflow".into());
            }
            // A relay in rel32 range lets retour relocate exactly the same five-byte
            // prologue that we patch. The known candidates have no hot-patch prefix.
            let aligned = site & !0xffff;
            let mut relay = ptr::null_mut();
            for distance in (0x10000..0x7000_0000_usize).step_by(0x10000) {
                for address in [aligned.checked_sub(distance), aligned.checked_add(distance)]
                    .into_iter()
                    .flatten()
                {
                    relay = unsafe {
                        VirtualAlloc(
                            address as *const c_void,
                            4096,
                            MEM_RESERVE | MEM_COMMIT,
                            PAGE_READWRITE,
                        )
                    };
                    if !relay.is_null() {
                        break;
                    }
                }
                if !relay.is_null() {
                    break;
                }
            }
            if relay.is_null() {
                return Err("cannot allocate a nearby detour relay".into());
            }
            let result = (|| {
                let mut code = [0_u8; 14];
                code[..6].copy_from_slice(&[0xff, 0x25, 0, 0, 0, 0]);
                code[6..].copy_from_slice(&(callback as u64).to_le_bytes());
                unsafe {
                    ptr::copy_nonoverlapping(code.as_ptr(), relay.cast(), code.len());
                }
                let mut previous = 0;
                if unsafe { VirtualProtect(relay, 4096, PAGE_EXECUTE_READ, &mut previous) } == 0
                    || unsafe { FlushInstructionCache(GetCurrentProcess(), relay, code.len()) } == 0
                {
                    return Err(io::Error::last_os_error().to_string());
                }
                let relocation = unsafe { RawDetour::new(site as *const (), relay.cast()) }
                    .map_err(|e| e.to_string())?;
                // Retour emits its trampoline after the relay; publish that code too.
                if unsafe { FlushInstructionCache(GetCurrentProcess(), ptr::null(), 0) } == 0 {
                    return Err(io::Error::last_os_error().to_string());
                }
                let mut region: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
                if unsafe {
                    VirtualQuery(
                        site as *const c_void,
                        &mut region,
                        size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                } == 0
                {
                    return Err(io::Error::last_os_error().to_string());
                }
                let relative = i32::try_from(relay as i64 - (site + 5) as i64)
                    .map_err(|_| "relay outside rel32 range")?;
                let mut patch = [0; 16];
                patch[0] = 0xe9;
                patch[1..5].copy_from_slice(&relative.to_le_bytes());
                Ok(Self {
                    relocation: ManuallyDrop::new(relocation),
                    relay,
                    site,
                    original,
                    sentinel_len,
                    patch,
                    patch_len: 5,
                    enabled: false,
                    needs_flush: false,
                    diagnostic_failure: false,
                    page_protection: region.Protect,
                    tap: None,
                    name: "native hook",
                    publication_busy: AtomicBool::new(false),
                })
            })();
            if result.is_err() {
                unsafe {
                    VirtualFree(relay, 0, MEM_RELEASE);
                }
            }
            result
        })
    }

    /// Whether the last tracked publication reached this site or tap while busy.
    pub fn publication_busy(&self) -> bool {
        self.publication_busy.load(Ordering::Relaxed)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled || self.needs_flush
    }

    /// A static family/site label retained for publication diagnostics.
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }
    /// Name the exact site whose authenticated bytes no longer match.
    pub fn snapshot_failure(&self) -> Option<&'static str> {
        (!self.bytes_match(self.enabled)).then_some(self.name)
    }

    pub fn trampoline(&self) -> usize {
        self.relocation.trampoline() as *const () as usize
    }

    /// Replace one verified five-byte CALL with a CALL to the relay. The callback invokes the
    /// original call target itself, then returns to return point after the five-byte CALL. Its
    /// native caller's frame and unwind information stay intact. The relocation trampoline is
    /// retained for ownership, never invoked.
    ///
    /// # Safety
    /// Authenticate the executable and sentinel, provide a callback with the native call ABI, and retain its code until restoration and quiescence complete.
    pub unsafe fn new_call(
        site: usize,
        callback: usize,
        original: [u8; 16],
    ) -> Result<Self, String> {
        unsafe { Self::new_call_span(site, callback, original, original.len()) }
    }

    /// Verify only the owned span: the two naval CALLs need 16 / 13 bytes;
    /// the latter ends at the next native function.
    ///
    /// # Safety
    /// The requirements of [`Self::new_call`] apply to the complete owned sentinel span.
    pub unsafe fn new_call_span(
        site: usize,
        callback: usize,
        original: [u8; 16],
        sentinel_len: usize,
    ) -> Result<Self, String> {
        installation::construct(site, || {
            if original[0] != 0xe8 {
                return Err("call hook requires a five-byte rel32 CALL".into());
            }
            let mut hook = unsafe { Self::new_span(site, callback, original, sentinel_len) }?;
            hook.patch[0] = 0xe8;
            Ok(hook)
        })
    }

    /// Keep a counted native CALL's entry, original and exit alive through restore.
    ///
    /// # Safety
    /// The requirements of [`Self::new_call`] apply. The supplied counter and stub must cover every native entry, target call, return and unwind until quiescence.
    pub unsafe fn new_call_with_safety(
        site: usize,
        callback: usize,
        original: [u8; 16],
        safety: TapSafety,
    ) -> Result<Self, String> {
        installation::construct(site, || {
            if safety.stub.is_empty() || !safety.stub.contains(&callback) {
                return Err("call tap stub must contain its callback".into());
            }
            let mut hook = unsafe { Self::new_call(site, callback, original) }?;
            hook.tap = Some(safety);
            Ok(hook)
        })
    }

    /// Replace the leader-action indirect CALLs, FF 90 disp32, with CALL relay; NOP. The counted
    /// stub forwards the live RAX-based target and native arguments, then returns to the return
    /// point after the five-byte CALL. Their caller has no unwind handler/chain; the five- and
    /// six-byte return points share the same body unwind state, after its ten-byte prologue. Its
    /// counter must cover the target until its return PC is no longer in use, including unwind.
    /// Rust cleanup must not clear it before Windows finishes traversing the stub. The stub range
    /// covers uncounted entry/exit instructions.
    ///
    /// # Safety
    /// Authenticate the executable and sentinel and verify the documented unwind state. The callback must preserve the native call ABI and the counter and stub must remain valid through every native return or unwind.
    pub unsafe fn new_indirect_call(
        site: usize,
        callback: usize,
        original: [u8; 16],
        safety: TapSafety,
    ) -> Result<Self, String> {
        installation::construct(site, || {
            if original[..2] != [0xff, 0x90] {
                return Err("indirect call tap requires a six-byte CALL [RAX+disp32]".into());
            }
            if safety.stub.is_empty() || !safety.stub.contains(&callback) {
                return Err("indirect call tap stub must contain its callback".into());
            }
            let mut hook = unsafe { Self::new(site, callback, original) }?;
            hook.patch[0] = 0xe8;
            hook.patch[5] = 0x90;
            hook.patch_len = 6;
            hook.tap = Some(safety);
            Ok(hook)
        })
    }

    pub fn executable_ranges(&self) -> [Range<usize>; 2] {
        let trampoline = self.trampoline();
        [
            self.relay as usize..self.relay as usize + 4096,
            trampoline & !0xfff..(trampoline & !0xfff) + 4096,
        ]
    }

    fn tap_busy_with(
        &self,
        enabled: bool,
        mut contains: impl FnMut(&[Range<usize>]) -> Result<bool, u32>,
    ) -> Result<bool, u32> {
        let [relay, trampoline] = self.executable_ranges();
        if let Some(tap) = &self.tap
            && (!enabled || !self.enabled)
        {
            // A native target can hold the five-byte return point on its stack while its current IP
            // is outside every collector range. Keep the flight check first.
            return Ok(tap.in_flight.load(Ordering::SeqCst) != 0
                || contains(&[relay, trampoline, tap.stub.clone()])?);
        }
        Ok(false)
    }

    fn family_ready<const TRACK_BUSY: bool>(
        hooks: &mut [&mut Self],
        enabled: bool,
        selected: impl Fn(usize) -> bool,
        ranges: &[Range<usize>],
        threads: &mut Threads,
        snapshot_refusal: &mut bool,
    ) -> Result<bool, u32> {
        if TRACK_BUSY {
            if let Some(index) = threads.colliding_range(ranges)? {
                hooks
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| selected(*index))
                    .nth(index)
                    .expect("range belongs to selected hook")
                    .1
                    .publication_busy
                    .store(true, Ordering::Relaxed);
                return Ok(false);
            }
        } else if threads.contains_ip(ranges)? {
            return Ok(false);
        }
        let unowned = enabled && hooks.iter().all(|hook| !hook.is_enabled());
        for (_, hook) in hooks
            .iter_mut()
            .enumerate()
            .filter(|(index, _)| selected(*index))
        {
            if hook
                .tap_busy_with(enabled, |ranges| threads.contains_ip(ranges))
                .inspect_err(|_| hook.diagnostic_failure = true)?
            {
                if TRACK_BUSY {
                    hook.publication_busy.store(true, Ordering::Relaxed);
                }
                return Ok(false);
            }
            if !hook.bytes_match(hook.enabled) {
                hook.diagnostic_failure = true;
                *snapshot_refusal = unowned;
                return Err(0);
            }
        }
        Ok(true)
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        Self::set_family_enabled(std::slice::from_mut(self), enabled)
    }

    fn bytes_match(&self, enabled: bool) -> bool {
        let mut expected = self.original;
        if enabled {
            expected[..self.patch_len].copy_from_slice(&self.patch[..self.patch_len]);
        }
        (unsafe { std::slice::from_raw_parts(self.site as *const u8, self.sentinel_len) })
            == &expected[..self.sentinel_len]
    }

    /// Recheck this hook's currently owned bytes for an installation diagnostic.
    /// Call only after publication has resumed threads; this does not grant a
    /// publication certificate or change ownership.
    pub fn matches_published_snapshot(&self) -> bool {
        self.bytes_match(self.enabled)
    }

    /// Prepare protection/ranges before holding threads, validate every site
    /// before any write, and publish the whole family under one suspension.
    pub fn set_family_enabled(hooks: &mut [Self], enabled: bool) -> Result<bool, String> {
        let mut references = publication_storage(hooks.len(), enabled)
            .map_err(|_| "hook family allocation failed")?;
        references.extend(hooks.iter_mut());
        Self::set_enabled_batch(&mut references, enabled)
    }

    pub fn set_enabled_batch(hooks: &mut [&mut Self], enabled: bool) -> Result<bool, String> {
        let result = Self::set_family_enabled_with_flush(hooks, enabled, shared::real_flush);
        Self::observe_installation_outcome(hooks.iter().map(|hook| &**hook), &result);
        result
    }

    /// Retain the first busy site for callers that report publication deadlines.
    pub fn set_enabled_batch_tracking_busy(
        hooks: &mut [&mut Self],
        enabled: bool,
    ) -> Result<bool, String> {
        let result = Self::set_family_enabled_with_flush_tracking::<true>(
            hooks,
            enabled,
            shared::real_flush,
        );
        Self::observe_installation_outcome(hooks.iter().map(|hook| &**hook), &result);
        result
    }

    fn observe_installation_outcome<'a>(
        hooks: impl Iterator<Item = &'a Self>,
        result: &Result<bool, String>,
    ) {
        if let Some(failure) = Self::installation_failure(hooks, result) {
            installation::refused(failure.kind, failure.site, &failure.cause);
        }
    }

    fn installation_failure<'a>(
        mut hooks: impl Iterator<Item = &'a Self>,
        result: &Result<bool, String>,
    ) -> Option<installation::Failure> {
        let Err(cause) = result else { return None };
        let first = hooks.next();
        let mut site = first.map(|hook| hook.site);
        let mut rollback_failed = first.is_some_and(|hook| hook.needs_flush);
        for hook in hooks {
            site = None;
            rollback_failed |= hook.needs_flush;
        }
        Some(installation::Failure {
            kind: if rollback_failed {
                installation::Kind::Rollback
            } else {
                installation::Kind::Publication
            },
            site,
            cause: cause.clone().into(),
        })
    }

    fn set_family_enabled_with_flush(
        hooks: &mut [&mut Self],
        enabled: bool,
        flush: impl FnMut() -> Result<(), u32>,
    ) -> Result<bool, String> {
        Self::set_family_enabled_with_flush_tracking::<false>(hooks, enabled, flush)
    }

    fn set_family_enabled_with_flush_tracking<const TRACK_BUSY: bool>(
        hooks: &mut [&mut Self],
        enabled: bool,
        mut flush: impl FnMut() -> Result<(), u32>,
    ) -> Result<bool, String> {
        for hook in hooks.iter_mut() {
            hook.diagnostic_failure = false;
        }
        if TRACK_BUSY {
            for hook in hooks.iter() {
                hook.publication_busy.store(false, Ordering::Relaxed);
            }
        }
        if hooks.is_empty() || !enabled && hooks.iter().all(|hook| !hook.is_enabled()) {
            return Ok(true);
        }
        // Taken before any protection change and released after the last
        // restore and resume; a panicking publisher cannot leave it unusable.
        let _publication = PUBLICATION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut threads = Threads::open()?;
        let mut previous = publication_storage(hooks.len(), enabled)
            .map_err(|_| "hook state allocation failed")?;
        previous.extend(hooks.iter().map(|hook| (hook.enabled, hook.is_enabled())));
        let selected = |index: usize| enabled || previous[index].1;
        let mut ranges = publication_storage(hooks.len(), enabled)
            .map_err(|_| "hook ranges allocation failed")?;
        ranges.extend(
            hooks
                .iter()
                .enumerate()
                .filter(|(index, _)| selected(*index))
                .map(|(_, hook)| hook.site..hook.site + hook.sentinel_len),
        );
        let mut protected = 0;
        let mut prepare_error = None;
        let mut publication_failed = false;
        let mut snapshot_refusal = false;
        for (_, hook) in hooks
            .iter_mut()
            .enumerate()
            .filter(|(index, _)| selected(*index))
        {
            let mut previous = 0;
            let ok = unsafe {
                VirtualProtect(
                    hook.site as *const c_void,
                    hook.sentinel_len,
                    PAGE_EXECUTE_READWRITE,
                    &mut previous,
                )
            } != 0;
            let error = if ok { 0 } else { unsafe { GetLastError() } };
            if !ok {
                hook.diagnostic_failure = true;
                prepare_error = Some(io::Error::from_raw_os_error(error as i32).to_string());
                break;
            }
            protected += 1;
        }
        // No allocation, formatting, locks or destruction while suspended.
        let outcome = if prepare_error.is_none() {
            (|| {
                threads.suspend()?;
                if !Self::family_ready::<TRACK_BUSY>(
                    hooks,
                    enabled,
                    selected,
                    &ranges,
                    &mut threads,
                    &mut snapshot_refusal,
                )? {
                    return Ok(false);
                }
                for (_, hook) in hooks
                    .iter_mut()
                    .enumerate()
                    .filter(|(index, _)| selected(*index))
                {
                    if hook.enabled == enabled {
                        continue;
                    }
                    let bytes = if enabled {
                        &hook.patch[..hook.patch_len]
                    } else {
                        &hook.original[..hook.patch_len]
                    };
                    unsafe {
                        ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            hook.site as *mut u8,
                            hook.patch_len,
                        );
                    }
                    hook.enabled = enabled;
                    hook.needs_flush = true;
                }
                if let Err(code) = flush() {
                    publication_failed = true;
                    // Restore the complete prior family before releasing threads.
                    // On a second flush failure retain executable allocations and
                    // let detach retry publication of the restored bytes.
                    for (index, hook) in hooks.iter_mut().enumerate() {
                        let was_enabled = previous[index].0;
                        if !selected(index) || hook.enabled == was_enabled && !hook.needs_flush {
                            continue;
                        }
                        let bytes = if was_enabled {
                            &hook.patch[..hook.patch_len]
                        } else {
                            &hook.original[..hook.patch_len]
                        };
                        unsafe {
                            ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                hook.site as *mut u8,
                                hook.patch_len,
                            );
                        }
                        hook.enabled = was_enabled;
                        hook.needs_flush = true;
                    }
                    if flush().is_ok()
                        && hooks
                            .iter()
                            .filter(|hook| hook.needs_flush)
                            .all(|hook| hook.bytes_match(hook.enabled))
                    {
                        for hook in hooks.iter_mut() {
                            hook.needs_flush = false;
                        }
                    }
                    return Err(code);
                }
                for (index, hook) in hooks.iter_mut().enumerate() {
                    if selected(index) && !hook.bytes_match(enabled) {
                        hook.diagnostic_failure = true;
                        return Err(0);
                    }
                }
                for hook in hooks.iter_mut() {
                    hook.needs_flush = false;
                }
                Ok(true)
            })()
        } else {
            Ok(false)
        };
        let mut protection_ok = true;
        for (_, hook) in hooks
            .iter_mut()
            .enumerate()
            .filter(|(index, _)| selected(*index))
            .take(protected)
        {
            let mut ignored = 0;
            let restored = unsafe {
                VirtualProtect(
                    hook.site as *const c_void,
                    hook.sentinel_len,
                    hook.page_protection,
                    &mut ignored,
                )
            } != 0;
            hook.diagnostic_failure |= !restored;
            protection_ok &= restored;
        }
        let census_refusal = threads.census_refusal();
        let resume = threads.resume();
        drop(threads);
        if !protection_ok {
            return Err("failed to restore hook-page protection".into());
        }
        resume.map_err(|code| format!("ResumeThread failed: {code}"))?;
        if let Some(reason) = prepare_error {
            return Err(reason);
        }
        outcome.map_err(|code| {
            let publication = if hooks.iter().any(|hook| hook.needs_flush) {
                "; restored bytes remain unflushed or unverified; executable storage retained for detach"
            } else {
                ""
            };
            if publication_failed {
                format!("instruction-cache publication failed (Windows error {code}); prior bytes restored{publication}")
            } else if snapshot_refusal {
                BYTE_VERIFICATION_FAILURE.into()
            } else if let Some(reason) = census_refusal {
                format!("{reason} (code {code:#x})")
            } else {
                format!("patch refused or verification failed (Windows error {code}){publication}")
            }
        })
    }
}

// Restore bookkeeping must remain possible after capture has stopped. These
// temporary hook-family vectors are prepared before suspension; failed reserves
// leave owned patches and executable storage intact for a later restore retry.
pub fn publication_storage<T>(capacity: usize, enabled: bool) -> io::Result<Vec<T>> {
    if enabled {
        engine_allocation::vec(capacity)
    } else {
        let mut storage = Vec::new();
        storage
            .try_reserve_exact(capacity)
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        Ok(storage)
    }
}

#[cfg(any(test, feature = "test-support"))]
std::thread_local! { static FAIL_FLUSHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(any(test, feature = "test-support"))]
pub fn fail_flushes(count: usize) {
    FAIL_FLUSHES.with(|remaining| remaining.set(count));
}
#[cfg(any(test, feature = "test-support"))]
pub static FAMILY_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    /// Thread suspensions this thread began ([`Threads::suspend`]).
    pub static SUSPENSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Held by one publication at a time, from its first page-protection change
/// to its last restore. Two publications whose sites share a code page would
/// otherwise race: one restores the page's original protection while the
/// other, already past its own `VirtualProtect`, writes its patch into a
/// page that is no longer writable (an access violation), or suspends the
/// other publisher mid-suspension. The runtime publishes from one thread, so
/// this lock is never contended in production; it makes the order a property
/// of the publication itself rather than of its callers.
pub static PUBLICATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Drop for Hook {
    fn drop(&mut self) {
        // The worker never drops an enabled hook or an in-use trampoline.
        if self.is_enabled() {
            // Retain the relay AND relocation on failed publication. Runtime
            // restores and drains callbacks before dropping an owned family.
            return;
        }
        unsafe {
            VirtualFree(self.relay, 0, MEM_RELEASE);
            ManuallyDrop::drop(&mut self.relocation);
        }
    }
}

// Off-suspension only: the snapshot API allocates and maps storage.
fn visit_threads(
    excluded: &[u32],
    mut visit: impl FnMut(u32) -> Result<(), u32>,
) -> Result<(), u32> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(unsafe { GetLastError() });
    }
    let result = (|| {
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut found = unsafe { Thread32First(snapshot, &mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == unsafe { GetCurrentProcessId() }
                && entry.th32ThreadID != unsafe { GetCurrentThreadId() }
                && !excluded.contains(&entry.th32ThreadID)
            {
                visit(entry.th32ThreadID)?;
            }
            found = unsafe { Thread32Next(snapshot, &mut entry) };
        }
        let error = unsafe { GetLastError() };
        if error == ERROR_NO_MORE_FILES {
            Ok(())
        } else {
            Err(error)
        }
    })();
    unsafe {
        CloseHandle(snapshot);
    }
    result
}

// NtQuerySystemInformation fills caller storage without user-mode allocation.
// SystemProcessInformation (5) writes caller storage; STATUS_INFO_LENGTH_MISMATCH
// reports the required length. The process and thread records are parsed
// below without following native pointers.
// https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntquerysysteminformation
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        buffer: *mut c_void,
        length: u32,
        required: *mut u32,
    ) -> i32;
}
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xc0000004_u32 as i32;

fn query_census(buffer: &mut [usize], required: &mut u32) -> Result<usize, u32> {
    let bytes = u32::try_from(std::mem::size_of_val(buffer)).map_err(|_| ERROR_BAD_LENGTH)?;
    let pointer = if buffer.is_empty() {
        ptr::null_mut()
    } else {
        buffer.as_mut_ptr().cast()
    };
    let status = unsafe { NtQuerySystemInformation(5, pointer, bytes, required) };
    if status == STATUS_INFO_LENGTH_MISMATCH {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    // Return NTSTATUS directly, without another user-mode helper while held.
    if status < 0 {
        return Err(status as u32);
    }
    if *required > bytes {
        return Err(ERROR_BAD_LENGTH);
    }
    Ok(*required as usize)
}

fn visit_census(
    bytes: &[u8],
    process: u32,
    current: u32,
    excluded: &[u32],
    mut visit: impl FnMut(u32) -> Result<(), u32>,
) -> Result<(), u32> {
    let mut offset = 0;
    loop {
        let record = bytes
            .get(offset..)
            .filter(|record| record.len() >= 0x100)
            .ok_or(ERROR_BAD_LENGTH)?;
        let next = u32::from_le_bytes(record[..4].try_into().unwrap()) as usize;
        let end = if next == 0 { record.len() } else { next };
        let record = record
            .get(..end)
            .filter(|record| record.len() >= 0x100)
            .ok_or(ERROR_BAD_LENGTH)?;
        let pid = u64::from_le_bytes(record[0x50..0x58].try_into().unwrap());
        if pid == u64::from(process) {
            let count = u32::from_le_bytes(record[4..8].try_into().unwrap()) as usize;
            let length = count.checked_mul(0x50).ok_or(ERROR_BAD_LENGTH)?;
            let threads = record
                .get(0x100..)
                .and_then(|tail| tail.get(..length))
                .ok_or(ERROR_BAD_LENGTH)?;
            for thread in threads.chunks_exact(0x50) {
                let id = u64::from_le_bytes(thread[0x30..0x38].try_into().unwrap());
                let id = u32::try_from(id).map_err(|_| ERROR_BAD_LENGTH)?;
                if id != current && !excluded.contains(&id) {
                    visit(id)?;
                }
            }
            return Ok(());
        }
        if next == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        offset = offset.checked_add(next).ok_or(ERROR_BAD_LENGTH)?;
    }
}

/// A refused `SuspendThread`: `Ok(false)` (not held) for a thread that has
/// exited or is exiting, the error otherwise. A thread whose termination
/// began (`NtTerminateThread`, after its DLL and TLS detach) refuses
/// suspension with `STATUS_THREAD_IS_TERMINATING`, which Win32 reports as
/// `ERROR_ACCESS_DENIED`, before its handle is signalled; it never runs
/// user code again, so leaving it unheld is safe. Every other refusal fails
/// the publication as before.
fn refused_suspension(error: u32, exited: bool) -> Result<bool, u32> {
    if exited || error == ERROR_ACCESS_DENIED {
        Ok(false)
    } else {
        Err(error)
    }
}

/// Opens a thread a census of this process listed, or `None` when that
/// thread has exited since. A thread id is unique only among live threads:
/// once the listed thread exits, the id can name a thread of another process
/// before this opens it. Such a thread is never ours to suspend, so a handle
/// whose owner is another process is closed, and an id this process may not
/// open (`ERROR_ACCESS_DENIED`, which no thread of this process returns for
/// these rights) is the same exit.
fn open_thread(id: u32) -> Result<Option<HANDLE>, u32> {
    let handle = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME
                | THREAD_GET_CONTEXT
                | THREAD_QUERY_LIMITED_INFORMATION
                | THREAD_SYNCHRONIZE,
            0,
            id,
        )
    };
    if !handle.is_null() {
        // An owned handle pins the id: its owner cannot change from here on.
        if unsafe { GetProcessIdOfThread(handle) } != unsafe { GetCurrentProcessId() } {
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }
        Ok(Some(handle))
    } else {
        let error = unsafe { GetLastError() };
        // The snapshot thread exited, and its id is free or another
        // process's.
        if error == ERROR_INVALID_PARAMETER || error == ERROR_ACCESS_DENIED {
            Ok(None)
        } else {
            Err(error)
        }
    }
}

// The successful call's start excludes the exit precheck. Refusal handling
// still probes exit after capturing the original suspension error.
fn suspension_with(
    handle: HANDLE,
    diagnostics: &publication::Attempt,
    mut exited: impl FnMut(HANDLE) -> bool,
    suspend: impl FnOnce(HANDLE) -> Result<(), u32>,
) -> Result<Option<u64>, u32> {
    if exited(handle) {
        return Ok(None);
    }
    let begin = diagnostics.now();
    match suspend(handle) {
        Ok(()) => Ok(Some(begin)),
        Err(error) => refused_suspension(error, exited(handle)).map(|_| None),
    }
}

fn acquire_context(handle: HANDLE, context: &mut AlignedContext) -> Result<(), u32> {
    #[cfg(test)]
    if let Some(result) = snapshot_tests::acquire(handle, context) {
        return result;
    }
    if unsafe { GetThreadContext(handle, &mut context.0) } == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

fn thread_exited(handle: HANDLE) -> bool {
    #[cfg(test)]
    if let Some(exited) = snapshot_tests::exited(handle) {
        return exited;
    }
    (unsafe { WaitForSingleObject(handle, 0) }) == 0
}

struct Thread {
    id: u32,
    handle: HANDLE,
    suspended: bool,
    ip: Option<usize>,
    exited: bool,
}

// The native x64 CONTEXT contract requires 16 bytes even though windows-sys
// 0.61's Rust fields only give the generated struct eight-byte alignment.
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

pub struct Threads {
    threads: Vec<Thread>,
    excluded: Vec<u32>,
    census: Vec<usize>,
    census_required: u32,
    census_short: bool,
    census_refusal: Option<&'static str>,
    context: AlignedContext,
    snapshot_valid: bool,
    diagnostics: publication::Attempt,
}

impl Threads {
    pub fn open() -> Result<Self, String> {
        Self::open_excluding(&[])
    }

    /// During callback quiescence registered collector workers may remain inside
    /// the DLL. Every excluded worker is joined before the final full-image check.
    /// Hook patching never excludes workers.
    pub fn open_excluding(excluded_threads: &[u32]) -> Result<Self, String> {
        let mut diagnostics = publication::Attempt::new();
        // Detach scratch must remain available after the capture allocation gate
        // stops admission. All growth still finishes before holding threads.
        let mut excluded = Vec::new();
        if excluded.try_reserve_exact(excluded_threads.len()).is_err() {
            diagnostics.finish();
            return Err("thread exclusion allocation failed".into());
        }
        excluded.extend_from_slice(excluded_threads);
        let mut result = Self {
            threads: Vec::new(),
            excluded,
            census: Vec::new(),
            census_required: 0,
            census_short: false,
            census_refusal: None,
            context: AlignedContext(unsafe { std::mem::zeroed() }),
            snapshot_valid: false,
            diagnostics,
        };
        let enumerated = visit_threads(&result.excluded, |id| {
            if let Some(handle) = open_thread(id)? {
                result.threads.push(Thread {
                    id,
                    handle,
                    suspended: false,
                    ip: None,
                    exited: false,
                });
            }
            Ok(())
        });
        enumerated.map_err(|code| format!("thread enumeration failed (Windows error {code})"))?;
        Ok(result)
    }

    pub fn suspend(&mut self) -> Result<(), u32> {
        #[cfg(any(test, feature = "test-support"))]
        SUSPENSIONS.with(|count| count.set(count.get() + 1));
        // Query sizing and prepare aligned caller storage immediately before
        // acquiring any hold. A resize retry always resumes before preparing.
        self.prepare_census()?;
        self.suspend_with(
            |handle, diagnostics| {
                suspension_with(handle, diagnostics, thread_exited, |handle| {
                    if unsafe { SuspendThread(handle) } == u32::MAX {
                        Err(unsafe { GetLastError() })
                    } else {
                        Ok(())
                    }
                })
            },
            Self::discover,
            |threads| {
                threads.resume()?;
                threads.prepare_census()
            },
        )
    }

    fn prepare_census(&mut self) -> Result<(), u32> {
        if self.threads.iter().any(|thread| thread.suspended) {
            self.census_refusal = Some("thread census resize requires resumed threads");
            return Err(ERROR_RETRY);
        }
        self.invalidate_snapshot();
        self.census_refusal = Some("thread census sizing failed");
        let mut required = 0;
        match query_census(&mut [], &mut required) {
            Ok(_) | Err(ERROR_INSUFFICIENT_BUFFER) => {}
            Err(code) => return Err(code),
        }
        let required = required.max(self.census_required) as usize;
        let bytes = required
            .checked_add(required / 2)
            .filter(|bytes| *bytes <= u32::MAX as usize)
            .ok_or(ERROR_BAD_LENGTH)?;
        let words = bytes.div_ceil(size_of::<usize>());
        let additional = words.saturating_sub(self.census.len());
        self.census
            .try_reserve_exact(additional)
            .map_err(|_| ERROR_NOT_ENOUGH_MEMORY)?;
        self.census.resize(words.max(self.census.len()), 0);
        // Headroom for threads created before every in-process creator is held.
        let headroom = self.threads.len() / 2 + 16;
        self.threads
            .try_reserve_exact(headroom)
            .map_err(|_| ERROR_NOT_ENOUGH_MEMORY)?;
        self.census_short = false;
        self.census_refusal = None;
        Ok(())
    }

    fn census_refusal(&self) -> Option<&'static str> {
        self.census_refusal
    }

    fn suspend_with(
        &mut self,
        mut suspend: impl FnMut(HANDLE, &publication::Attempt) -> Result<Option<u64>, u32>,
        mut discover: impl FnMut(&mut Self) -> Result<bool, u32>,
        mut resize: impl FnMut(&mut Self) -> Result<(), u32>,
    ) -> Result<(), u32> {
        // Exited newcomers can keep recycling prepared slots. Refuse churn
        // after a live-sized number of rounds instead of holding threads forever.
        for _ in 0..=self.threads.capacity() {
            self.snapshot_valid = false;
            for thread in self.threads.iter_mut().filter(|thread| !thread.suspended) {
                if let Some(begin) = suspend(thread.handle, &self.diagnostics)? {
                    thread.suspended = true;
                    self.diagnostics.acquired(begin);
                }
            }
            // A thread can be created between the first snapshot and suspension.
            // Game thread creation happens in-process; once all such callers are
            // held and enumeration adds nobody, they cannot create another
            // thread during the write. No heap use while held.
            match discover(self) {
                Ok(false) => return Ok(()),
                Ok(true) => {}
                Err(ERROR_INSUFFICIENT_BUFFER) if self.census_short => {
                    resize(self)?;
                    self.census_refusal =
                        Some("thread census buffer kept growing during suspension");
                }
                Err(code) => return Err(code),
            }
        }
        self.census_refusal
            .get_or_insert("thread census did not reach a stable set");
        Err(ERROR_RETRY)
    }

    fn discover(&mut self) -> Result<bool, u32> {
        // GetThreadContext synchronizes with suspension before the next census.
        // https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getthreadcontext
        self.discover_with(Self::synchronize, Self::discover_census)
    }

    fn discover_with(
        &mut self,
        synchronize: impl FnOnce(&mut Self) -> Result<(), u32>,
        census: impl FnOnce(&mut Self) -> Result<bool, u32>,
    ) -> Result<bool, u32> {
        synchronize(self)?;
        let result = census(self);
        self.snapshot_valid = result == Ok(false);
        result
    }

    fn discover_census(&mut self) -> Result<bool, u32> {
        let length = match query_census(&mut self.census, &mut self.census_required) {
            Ok(length) => length,
            Err(code) => {
                self.census_short = code == ERROR_INSUFFICIENT_BUFFER;
                self.census_refusal = Some(if self.census_short {
                    "thread census buffer too small during suspension"
                } else {
                    "NtQuerySystemInformation thread census failed"
                });
                return Err(code);
            }
        };
        self.census_refusal = None;
        self.remove_exited_with(
            |handle| unsafe { WaitForSingleObject(handle, 0) } == 0,
            |handle| {
                unsafe { CloseHandle(handle) };
            },
        );
        let mut added = false;
        let bytes =
            unsafe { std::slice::from_raw_parts(self.census.as_ptr().cast::<u8>(), length) };
        visit_census(
            bytes,
            unsafe { GetCurrentProcessId() },
            unsafe { GetCurrentThreadId() },
            &self.excluded,
            |id| {
                if self.threads.iter().any(|thread| {
                    thread.id == id && unsafe { WaitForSingleObject(thread.handle, 0) } != 0
                }) {
                    return Ok(());
                }
                if self.threads.len() == self.threads.capacity() {
                    // Never grow while threads are held: resume, grow and retry
                    // through the bounded census resize path.
                    self.census_short = true;
                    self.census_refusal = Some("prepared thread storage insufficient");
                    return Err(ERROR_INSUFFICIENT_BUFFER);
                }
                if let Some(handle) = open_thread(id)? {
                    self.threads.push(Thread {
                        id,
                        handle,
                        suspended: false,
                        ip: None,
                        exited: false,
                    });
                    added = true;
                }
                Ok(())
            },
        )?;
        Ok(added)
    }

    fn remove_exited_with(
        &mut self,
        mut exited: impl FnMut(HANDLE) -> bool,
        mut close: impl FnMut(HANDLE),
    ) {
        self.threads.retain(|thread| {
            if exited(thread.handle) {
                if thread.suspended && !thread.exited {
                    self.diagnostics.resolved();
                }
                close(thread.handle);
                false
            } else {
                true
            }
        });
    }

    fn invalidate_snapshot(&mut self) {
        self.snapshot_valid = false;
        for thread in &mut self.threads {
            thread.ip = None;
        }
    }

    fn synchronize(&mut self) -> Result<(), u32> {
        self.synchronize_with(acquire_context, thread_exited)
    }

    fn synchronize_with(
        &mut self,
        mut context: impl FnMut(HANDLE, &mut AlignedContext) -> Result<(), u32>,
        mut exited: impl FnMut(HANDLE) -> bool,
    ) -> Result<(), u32> {
        self.invalidate_snapshot();
        let begin = self.diagnostics.now();
        let result = (|| {
            for thread in self.threads.iter_mut().filter(|thread| thread.suspended) {
                self.context.0 = unsafe { std::mem::zeroed() };
                self.context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
                let started = self.diagnostics.now();
                let result = context(thread.handle, &mut self.context);
                self.diagnostics.context(started, thread.id);
                if let Err(error) = result {
                    if exited(thread.handle) {
                        if !thread.exited {
                            thread.exited = true;
                            self.diagnostics.resolved();
                        }
                        continue;
                    }
                    return Err(error);
                }
                thread.ip = Some(self.context.0.Rip as usize);
            }
            Ok(())
        })();
        let elapsed = self.diagnostics.elapsed(begin);
        self.diagnostics.sample.longest_synchronization_ns = self
            .diagnostics
            .sample
            .longest_synchronization_ns
            .max(elapsed);
        // Discovery still has to prove that this synchronized set is complete.
        result
    }

    pub fn contains_ip(&mut self, ranges: &[Range<usize>]) -> Result<bool, u32> {
        self.contains_ip_with(ranges, thread_exited)
    }

    fn contains_ip_with(
        &mut self,
        ranges: &[Range<usize>],
        exited: impl FnMut(HANDLE) -> bool,
    ) -> Result<bool, u32> {
        self.find_collision(
            |ip| ranges.iter().any(|range| range.contains(&ip)).then_some(()),
            exited,
        )
        .map(|collision| collision.is_some())
    }

    /// Return the first live collision from the ordinary suspended-thread scan.
    fn colliding_range(&mut self, ranges: &[Range<usize>]) -> Result<Option<usize>, u32> {
        self.find_collision(
            |ip| ranges.iter().position(|range| range.contains(&ip)),
            thread_exited,
        )
    }

    fn find_collision<T>(
        &mut self,
        mut locate: impl FnMut(usize) -> Option<T>,
        mut exited: impl FnMut(HANDLE) -> bool,
    ) -> Result<Option<T>, u32> {
        #[cfg(test)]
        IP_SCANS.with(|count| count.set(count.get() + 1));
        if !self.snapshot_valid && self.threads.iter().any(|thread| thread.suspended) {
            return Err(ERROR_RETRY);
        }
        for thread in self.threads.iter_mut().filter(|thread| thread.suspended) {
            let Some(ip) = thread.ip else {
                continue;
            };
            let Some(collision) = locate(ip) else {
                continue;
            };
            // Only a collision needs an exit probe; healthy ranges add no syscall.
            if exited(thread.handle) {
                thread.ip = None;
                thread.exited = true;
                self.diagnostics.resolved();
                continue;
            }
            return Ok(Some(collision));
        }
        Ok(None)
    }

    /// Return only after every suspension acquired by this owner is resolved.
    /// A failed OS resume is still reported after recovery; it never permits
    /// callers to format errors, destroy owners or close unresolved handles.
    /// Persistent uncertainty retains this controller and all caller storage
    /// in the resume-only loop, even when an outer detach budget has expired.
    pub fn resume(&mut self) -> Result<(), u32> {
        self.resume_with(
            |handle| {
                if unsafe { ResumeThread(handle) } == u32::MAX {
                    Err(unsafe { GetLastError() })
                } else {
                    Ok(())
                }
            },
            // open_excluding owns a stable thread handle with SYNCHRONIZE.
            // Only a signaled handle proves exit; timeout/WAIT_FAILED do not.
            |handle| unsafe { WaitForSingleObject(handle, 0) } == 0,
            || unsafe { Sleep(1) },
        )
    }

    fn resume_with(
        &mut self,
        mut resume: impl FnMut(HANDLE) -> Result<(), u32>,
        mut exited: impl FnMut(HANDLE) -> bool,
        mut retry: impl FnMut(),
    ) -> Result<(), u32> {
        self.invalidate_snapshot();
        let mut first_error = None;
        while self.threads.iter().any(|thread| thread.suspended) {
            for thread in self
                .threads
                .iter_mut()
                .rev()
                .filter(|thread| thread.suspended)
            {
                match resume(thread.handle) {
                    Ok(()) => {
                        thread.suspended = false;
                        if !thread.exited {
                            self.diagnostics.resolved();
                        }
                    }
                    Err(code) => {
                        first_error.get_or_insert(code);
                        if exited(thread.handle) {
                            thread.suspended = false;
                            if !thread.exited {
                                thread.exited = true;
                                self.diagnostics.resolved();
                            }
                        }
                    }
                }
            }
            if self.threads.iter().any(|thread| thread.suspended) {
                // No heap use, formatting, locks, destruction or Rust unwind.
                // Handles and all surrounding publication/runtime roots stay
                // owned until actual resume or actual thread exit is proved.
                retry();
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for Threads {
    fn drop(&mut self) {
        let _ = self.resume();
        for thread in &self.threads {
            unsafe {
                CloseHandle(thread.handle);
            }
        }
        self.diagnostics.finish();
    }
}

#[path = "hook/publication.rs"]
pub mod publication;

#[path = "hook/shared.rs"]
pub mod shared;
#[path = "hook/tap.rs"]
mod tap;
/// The bytes a [`Tap`] at a sentinel replaces, for the site-overlap test.
pub use tap::instruction_span as tap_span;
pub use tap::{Registers, Tap};

#[cfg(any(test, feature = "test-support"))]
pub struct TestCode(*mut c_void);
#[cfg(any(test, feature = "test-support"))]
impl Default for TestCode {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl TestCode {
    pub fn new() -> Self {
        let memory = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        };
        assert!(!memory.is_null());
        Self(memory)
    }
    // Five NOPs give retour a frameless, position-independent relocation;
    // the remaining indirect jump preserves every native argument and return.
    pub fn stub(&self, offset: usize, target: usize) -> (usize, [u8; 16]) {
        assert!(offset + 19 <= 4096);
        let mut code = [0x90; 19];
        code[5..11].copy_from_slice(&[0xff, 0x25, 0, 0, 0, 0]);
        code[11..].copy_from_slice(&(target as u64).to_le_bytes());
        let address = self.0 as usize + offset;
        unsafe {
            ptr::copy_nonoverlapping(code.as_ptr(), address as *mut u8, code.len());
        }
        assert_ne!(
            unsafe {
                FlushInstructionCache(GetCurrentProcess(), address as *const c_void, code.len())
            },
            0
        );
        (address, code[..16].try_into().unwrap())
    }
}
#[cfg(any(test, feature = "test-support"))]
impl Drop for TestCode {
    fn drop(&mut self) {
        unsafe {
            VirtualFree(self.0, 0, MEM_RELEASE);
        }
    }
}

#[cfg(test)]
#[path = "hook/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "hook/snapshot_tests.rs"]
mod snapshot_tests;

#[cfg(test)]
#[path = "hook/indirect_call_tests.rs"]
mod indirect_call_tests;

#[cfg(test)]
mod call_tap_tests {
    use super::*;
    core::arch::global_asm!(
        ".text", ".globl call_tap_test_caller", ".seh_proc call_tap_test_caller",
        "call_tap_test_caller:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
        ".globl call_tap_test_call_site", "call_tap_test_call_site:", "call {target}",
        ".rept 11", "nop", ".endr", "add rsp, 0x28", "ret", ".seh_endproc", target = sym target);
    unsafe extern "system" {
        fn call_tap_test_caller(value: usize) -> usize;
        fn call_tap_test_call_site();
    }
    extern "system" fn target(value: usize) -> usize {
        value + 1
    }
    extern "system" fn tapped(value: usize) -> usize {
        target(value) + 100
    }
    #[test]
    fn call_patch_returns_to_native_continuation_and_restores_exactly() {
        let _guard = FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let site = call_tap_test_call_site as *const () as usize;
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        assert_eq!(sentinel[0], 0xe8);
        assert_eq!(unsafe { call_tap_test_caller(41) }, 42);
        let mut hook =
            unsafe { Hook::new_call(site, tapped as *const () as usize, sentinel) }.unwrap();
        assert!(hook.set_enabled(true).unwrap());
        assert_eq!(unsafe { call_tap_test_caller(41) }, 142);
        assert!(hook.set_enabled(false).unwrap());
        assert_eq!(unsafe { call_tap_test_caller(41) }, 42);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
    }

    #[test]
    fn counted_call_restore_waits_for_the_native_return() {
        static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
        let _guard = FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let site = call_tap_test_call_site as *const () as usize;
        let callback = tapped as *const () as usize;
        let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        assert!(
            unsafe {
                Hook::new_call_with_safety(
                    site,
                    callback,
                    sentinel,
                    TapSafety {
                        in_flight: &IN_FLIGHT,
                        stub: site..site + 1,
                    },
                )
            }
            .is_err()
        );
        let mut hook = unsafe {
            Hook::new_call_with_safety(
                site,
                callback,
                sentinel,
                TapSafety {
                    in_flight: &IN_FLIGHT,
                    stub: callback..callback + 1,
                },
            )
        }
        .unwrap();
        assert!(hook.set_enabled(true).unwrap());
        IN_FLIGHT.store(1, Ordering::SeqCst);
        assert!(!hook.set_enabled(false).unwrap());
        assert!(hook.is_enabled());
        IN_FLIGHT.store(0, Ordering::SeqCst);
        assert!(hook.set_enabled(false).unwrap());
        assert_eq!(unsafe { call_tap_test_caller(41) }, 42);
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn ordinary_hook_refuses_callback_outside_its_stub_before_reading_native_code() {
        static FLIGHTS: AtomicUsize = AtomicUsize::new(0);
        for stub in [10..10, 10..11, 20..30] {
            let safety = TapSafety {
                in_flight: &FLIGHTS,
                stub,
            };
            assert!(unsafe { Hook::new_with_safety(0, 15, [0; 16], safety) }.is_err());
        }
    }

    #[test]
    fn suspension_refuses_repeated_newcomer_churn_within_prepared_capacity() {
        let mut threads = snapshot_tests::fixture(1, &[Ok(1)]);
        let limit = threads.threads.capacity() + 1;
        let mut rounds = 0;
        let result = threads.suspend_with(
            |_, diagnostics| Ok(Some(diagnostics.now())),
            |threads| {
                rounds += 1;
                assert!(rounds <= limit, "thread census did not refuse churn");
                // The held thread exited and a newcomer reuses its slot.
                threads.diagnostics.resolved();
                threads.threads[0] = Thread {
                    id: rounds as u32 + 1,
                    handle: (rounds + 1) as HANDLE,
                    suspended: false,
                    ip: None,
                    exited: false,
                };
                Ok(true)
            },
            |_| panic!("newcomer churn must not resize census storage"),
        );
        assert_eq!(result, Err(ERROR_RETRY));
        assert_eq!(rounds, limit);
        snapshot_tests::release(threads);
    }

    #[test]
    fn census_resize_resumes_every_hold_and_retries_within_the_existing_round_bound() {
        use snapshot_tests::{counts, fixture, release, set_ips, stable};
        for (keeps_growing, final_ip) in [
            (true, Ok(1)),
            (false, Ok(1)),
            (false, Ok(100)),
            (false, Err(87)),
        ] {
            let mut threads = fixture(2, &[Ok(100); 2]);
            let limit = threads.threads.capacity() + 1;
            let mut rounds = 0;
            let mut resizes = 0;
            let result = threads.suspend_with(
                |_, clock| Ok(Some(clock.now())),
                |threads| {
                    threads.discover_with(Threads::synchronize, |threads| {
                        rounds += 1;
                        if rounds == 1 || keeps_growing {
                            threads.census_short = true;
                            Err(ERROR_INSUFFICIENT_BUFFER)
                        } else {
                            Ok(false)
                        }
                    })
                },
                |threads| {
                    threads.resume_with(|_| Ok(()), |_| false, || unreachable!())?;
                    assert!(
                        threads
                            .threads
                            .iter()
                            .all(|thread| !thread.suspended && thread.ip.is_none())
                    );
                    threads.census.resize(threads.census.len() + 1, 0);
                    threads.census_short = false;
                    set_ips(&[final_ip; 2]);
                    resizes += 1;
                    Ok(())
                },
            );
            if keeps_growing {
                assert_eq!(result, Err(ERROR_RETRY));
                assert_eq!((rounds, resizes), (limit, limit));
                assert_eq!(counts(), [limit; 2]);
                assert_eq!(
                    threads.census_refusal(),
                    Some("thread census buffer kept growing during suspension")
                );
            } else {
                assert_eq!(result, final_ip.map(|_| ()));
                assert_eq!(resizes, 1);
                assert_eq!(rounds, if final_ip.is_ok() { 2 } else { 1 });
                assert_eq!(
                    threads.contains_ip(std::slice::from_ref(&(100..116))),
                    match final_ip {
                        Ok(ip) => Ok(ip == 100),
                        Err(_) => Err(ERROR_RETRY),
                    }
                );
                assert_eq!(counts(), if final_ip.is_ok() { [2, 2] } else { [2, 1] });
                if final_ip.is_err() {
                    assert!(threads.threads.iter().all(|thread| thread.ip.is_none()));
                }
                threads
                    .resume_with(|_| Ok(()), |_| false, || unreachable!())
                    .unwrap();
                set_ips(&[Ok(1); 2]);
                stable(&mut threads).unwrap();
                assert_eq!(
                    threads.contains_ip(std::slice::from_ref(&(100..116))),
                    Ok(false)
                );
                assert_eq!(counts(), if final_ip.is_ok() { [3, 3] } else { [3, 2] });
            }
            release(threads);
        }
    }

    #[test]
    fn prepared_system_census_reads_only_this_process_and_refuses_short_thread_records() {
        let mut bytes = vec![0_u8; 0x100 + 0x100 + 3 * 0x50];
        bytes[..4].copy_from_slice(&0x100_u32.to_le_bytes());
        bytes[0x50..0x58].copy_from_slice(&90_u64.to_le_bytes());
        bytes[0x104..0x108].copy_from_slice(&3_u32.to_le_bytes());
        bytes[0x150..0x158].copy_from_slice(&91_u64.to_le_bytes());
        for (index, id) in [10_u64, 11, 12].into_iter().enumerate() {
            let offset = 0x200 + index * 0x50 + 0x30;
            bytes[offset..offset + 8].copy_from_slice(&id.to_le_bytes());
        }
        let mut ids = Vec::new();
        visit_census(&bytes, 91, 10, &[12], |id| {
            ids.push(id);
            Ok(())
        })
        .unwrap();
        assert_eq!(ids, [11]);
        assert_eq!(
            visit_census(&bytes[..bytes.len() - 1], 91, 10, &[], |_| Ok(())),
            Err(ERROR_BAD_LENGTH)
        );
    }

    #[test]
    fn resume_recovery_keeps_failed_holds_and_skips_unacquired_or_resolved_ones() {
        let mut threads = ManuallyDrop::new(Threads {
            threads: (1..=3)
                .map(|handle| Thread {
                    id: handle as u32,
                    handle: handle as HANDLE,
                    suspended: handle != 2,
                    ip: None,
                    exited: false,
                })
                .collect(),
            excluded: Vec::new(),
            census: Vec::new(),
            census_required: 0,
            census_short: false,
            census_refusal: None,
            context: AlignedContext(unsafe { std::mem::zeroed() }),
            snapshot_valid: false,
            diagnostics: publication::Attempt::new(),
        });
        let mut calls = [0; 3];
        let mut waits = 0;
        let result = threads.resume_with(
            |handle| {
                let index = handle as usize - 1;
                calls[index] += 1;
                if index == 2 && calls[index] == 1 {
                    Err(5)
                } else {
                    Ok(())
                }
            },
            |_| false,
            || waits += 1,
        );
        assert_eq!(result, Err(5));
        assert_eq!(calls, [1, 0, 2]);
        assert_eq!(waits, 1);
        assert!(threads.threads.iter().all(|thread| !thread.suspended));
        // Synthetic handles must never reach the OS cleanup.
        threads.threads.clear();
        unsafe { ManuallyDrop::drop(&mut threads) };
    }
}

#[cfg(test)]
mod thread_resume_policy_tests {
    use super::*;

    // These labels never reach Windows. ManuallyDrop also prevents a failed
    // assertion from passing synthetic handles to the real Threads::drop.
    fn fixture(count: usize) -> ManuallyDrop<Threads> {
        ManuallyDrop::new(Threads {
            threads: (1..=count)
                .map(|label| Thread {
                    id: label as u32,
                    handle: label as HANDLE,
                    suspended: true,
                    ip: None,
                    exited: false,
                })
                .collect(),
            excluded: Vec::new(),
            census: Vec::new(),
            census_required: 0,
            census_short: false,
            census_refusal: None,
            context: AlignedContext(unsafe { std::mem::zeroed() }),
            snapshot_valid: false,
            diagnostics: publication::Attempt::new(),
        })
    }
    fn release(mut threads: ManuallyDrop<Threads>) {
        threads
            .resume_with(|_| Ok(()), |_| false, || unreachable!())
            .unwrap();
        threads.threads.clear();
        unsafe { ManuallyDrop::drop(&mut threads) };
    }

    #[test]
    fn failed_resume_retains_each_hold_until_recovered_and_preserves_first_error() {
        let mut threads = fixture(2);
        let mut calls = [0; 2];
        let mut waits = 0;
        let result = threads.resume_with(
            |handle| {
                let index = handle as usize - 1;
                calls[index] += 1;
                if calls[index] == 1 {
                    Err(40 + index as u32)
                } else {
                    Ok(())
                }
            },
            |_| false,
            || waits += 1,
        );
        assert_eq!(result, Err(41)); // reverse order: the first fault survives
        assert_eq!(calls, [2, 2]);
        assert_eq!(waits, 1);
        assert!(threads.threads.iter().all(|thread| !thread.suspended));
        release(threads);
    }

    #[test]
    fn unresolved_exit_probe_does_not_release_and_proved_exit_settles_only_its_hold() {
        let mut threads = fixture(2);
        let mut calls = [0; 2];
        let mut waits = 0;
        let result = threads.resume_with(
            |handle| {
                let index = handle as usize - 1;
                calls[index] += 1;
                if index == 0 && calls[index] == 2 {
                    Ok(())
                } else {
                    Err(6)
                }
            },
            |handle| handle as usize == 2,
            || waits += 1,
        );
        assert_eq!(result, Err(6));
        assert_eq!(calls, [2, 1]);
        assert_eq!(waits, 1);
        assert!(threads.threads.iter().all(|thread| !thread.suspended));
        release(threads);
    }

    #[test]
    fn resume_recovery_never_touches_unacquired_or_already_resolved_holds() {
        let mut threads = fixture(3);
        threads.threads[1].suspended = false;
        let mut calls = [0; 3];
        let mut waits = 0;
        let result = threads.resume_with(
            |handle| {
                let index = handle as usize - 1;
                calls[index] += 1;
                if index == 2 && calls[index] == 1 {
                    Err(5)
                } else {
                    Ok(())
                }
            },
            |_| false,
            || waits += 1,
        );
        assert_eq!(result, Err(5));
        assert_eq!(calls, [1, 0, 2]);
        assert_eq!(waits, 1);
        assert!(threads.threads.iter().all(|thread| !thread.suspended));
        release(threads);
    }
}

#[cfg(test)]
std::thread_local! { static IP_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
