//! freeze takes it before its first `SuspendThread` and releases it after its
//! is scoped by a process handle, so OWNERSHIP IS BY CONSTRUCTION. The machinery
//! `--release`. Re-measured in both profiles on the same machine in the same
//!   a clean freeze     DEBUG 14.6-19.2 ms      RELEASE 1.85-2.13 ms
//!   14 in a row*       DEBUG   222-233 ms      RELEASE  27.4-38.3 ms
//!   one toolhelp pass  DEBUG 45.9-55.7 ms      RELEASE 43.4-50.0 ms
//! OF WHICH THE GAME IS STOPPED FOR 1.19-3.55 ms (release, measured at the
//! `guard_lifetime_raw.txt`): one production frozen window is 30.6-80.6 ms, of
//! REFERENCES one thread, RELEASES the lock, and copies that thread's record
//! reacquired lock; both insertion and unlinking of that field require the SAME
//!   RELEASE, ~730 KB / ~700 processes:        1.3-4.3 ms, settling to ~1.3 ms
//! 1.85-2.13 ms in the shipped release build (14.6-19.2 ms in debug), against
//! ceiling is [`MAX_PASSES`] reads, ARITHMETICALLY ~21-69 ms at the release
//!    release build and 14.6-19.2 ms in debug
//! module could deadlock on, and none of them makes an explicit TIMED wait --

use crate::freeze_census::{FreezeEntryPoint, FreezeTiming};
use core::ffi::c_void;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;
#[cfg(test)]
use windows_sys::Win32::Foundation::SetLastError;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::Debug::{
    GetThreadContext, CONTEXT, CONTEXT_CONTROL_AMD64,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetThreadId, ResumeThread,
    SuspendThread, WaitForSingleObject, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
    THREAD_SET_CONTEXT, THREAD_SUSPEND_RESUME,
};

const SYNCHRONIZE: u32 = 0x0010_0000;

/// refusal reports. MEASURED on Windows 10.0.26200 for the ownership read this
pub(crate) const FREEZE_THREAD_ACCESS: u32 = THREAD_SUSPEND_RESUME
    | THREAD_GET_CONTEXT
    | THREAD_QUERY_INFORMATION
    | THREAD_SET_CONTEXT
    | SYNCHRONIZE;

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const MINHOOK_THREAD_ACCESS: u32 =
    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION | THREAD_SET_CONTEXT;

const MAXIMUM_ALLOWED: u32 = 0x0200_0000;

const OBJECT_BASIC_INFORMATION: u32 = 0;

const STATUS_NO_MORE_ENTRIES: i32 = 0x8000_001Au32 as i32;

const MAXIMUM_SUSPEND_COUNT: u32 = 127;

const SUSPENSIONS_STILL_TO_COME: u32 = 1;

pub(crate) const MAX_PASSES: u32 = 16;

static FREEZE_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static FREEZE_REFUSALS: AtomicU64 = AtomicU64::new(0);
static SUSPEND_FAILURES_EXCUSED_AS_TERMINATED: AtomicU64 = AtomicU64::new(0);
static SUSPEND_FAILURES_DEFERRED_TO_A_LATER_PASS: AtomicU64 = AtomicU64::new(0);
static FREEZE_PASSES_SPENT: AtomicU64 = AtomicU64::new(0);

std::thread_local! {
    static THREAD_FREEZE_ATTEMPTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    static SUSPENDED_ANY_THIS_FREEZE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn freeze_attempts() -> u64 {
    FREEZE_ATTEMPTS.load(Ordering::Acquire)
}

pub fn freeze_attempts_on_this_thread() -> u64 {
    THREAD_FREEZE_ATTEMPTS.with(|count| count.get())
}

pub fn resume_failures() -> u64 {
    RESUME_FAILURES.load(Ordering::Acquire)
}

pub fn freeze_refusals() -> u64 {
    FREEZE_REFUSALS.load(Ordering::Acquire)
}

pub fn suspend_failures_excused_as_terminated() -> u64 {
    SUSPEND_FAILURES_EXCUSED_AS_TERMINATED.load(Ordering::Acquire)
}

pub fn suspend_failures_deferred_to_a_later_pass() -> u64 {
    SUSPEND_FAILURES_DEFERRED_TO_A_LATER_PASS.load(Ordering::Acquire)
}

pub fn freeze_passes_spent() -> u64 {
    FREEZE_PASSES_SPENT.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) static OS_SUSPEND_STATE_TESTS: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn os_suspend_state_guard() -> MutexGuard<'static, ()> {
    OS_SUSPEND_STATE_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeRefusal {
    EnumeratorUnavailable {
        os_error: u32,
    },
    Enumeration(u32),
    ThreadRights {
        thread: u32,
        granted: u32,
    },
    ThreadSuspend {
        thread: u32,
        os_error: u32,
    },
    SuspendCeiling {
        thread: u32,
        previous_count: u32,
    },
    RosterUnavailable {
        status: i32,
    },
    RosterUnreadable,
    RosterUnheld {
        thread: u32,
        passes: u32,
    },
    NotQuiescent {
        passes: u32,
        unaccounted: u32,
    },
    ThreadContext {
        thread: u32,
        os_error: u32,
    },
    ThreadRightsUnreadable {
        status: u32,
    },
    ThreadIdentity {
        os_error: u32,
    },
    CapacityExceeded {
        buffer: &'static str,
        capacity: usize,
    },
    /// nobody left to release it.
    MinHookLock(minhook::MH_STATUS),
    TargetExecuting {
        ranges: usize,
    },
    MinHookQuery(minhook::MH_STATUS),
}

impl fmt::Display for FreezeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnumeratorUnavailable { os_error } => write!(
                f,
                "detour patch refused: the kernel thread walk (ntdll \
                 NtGetNextThread / NtQueryObject) could not be resolved (os \
                 error {os_error}), so this process's thread list could not be \
                 read; no target byte was touched"
            ),
            Self::Enumeration(code) => write!(
                f,
                "detour patch refused: the thread walk stopped with status \
                 {code:#010x} rather than reaching the end of the list, so the \
                 threads past the stop were never seen; no target byte was touched"
            ),
            Self::ThreadRights { thread, granted } => write!(
                f,
                "detour patch refused: a thread of this process (id {thread}, 0 \
                 when the same denial hid its id) granted only {granted:#010x}, \
                 which is short of the access this freeze needs to suspend it \
                 and leave MinHook room to suspend it again; no target byte was \
                 touched"
            ),
            Self::ThreadSuspend { thread, os_error } => write!(
                f,
                "detour patch refused: thread {thread} could not be suspended \
                 (os error {os_error}) and was still not proven terminated when \
                 the freeze ran out of passes to re-derive it, so it may be \
                 executing inside the patch site; no target byte was touched"
            ),
            Self::SuspendCeiling {
                thread,
                previous_count,
            } => write!(
                f,
                "detour patch refused: thread {thread} was already suspended \
                 {previous_count} times, leaving no room for MinHook's own \
                 suspension on top of ours -- which would fail, and MinHook \
                 patches anyway when it does; no target byte was touched"
            ),
            Self::RosterUnavailable { status } => write!(
                f,
                "detour patch refused: the system thread roster could not be \
                 read (NTSTATUS {status:#x}), and a walk alone cannot prove the \
                 thread set closed; no target byte was touched"
            ),
            Self::RosterUnreadable => write!(
                f,
                "detour patch refused: the system thread roster does not list \
                 this process's own running thread, so its layout is not what \
                 this module reads; no target byte was touched"
            ),
            Self::RosterUnheld { thread, passes } => write!(
                f,
                "detour patch refused: on the last of {passes} passes the system \
                 roster listed thread {thread} in this process and the thread \
                 walk had not returned it, so it could not be suspended -- \
                 either this process cannot open it at all, or it is being \
                 recreated faster than the walk reaches it; no target byte was \
                 touched"
            ),
            Self::NotQuiescent { passes, unaccounted } => write!(
                f,
                "detour patch refused: the last of {passes} passes still had \
                 {unaccounted} thread(s) to account for, so the process never \
                 went quiet; no target byte was touched"
            ),
            Self::ThreadContext { thread, os_error } => write!(
                f,
                "detour patch refused: the context of held thread {thread} could \
                 not be read (os error {os_error}), so neither its suspension \
                 could be proven complete nor its instruction pointer proven \
                 outside the patch site; no target byte was touched"
            ),
            Self::ThreadRightsUnreadable { status } => write!(
                f,
                "detour patch refused: the access granted on a thread of this                  process could not be read (NtQueryObject status {status:#010x}),                  so it could not be established that this freeze can stop it; no                  target byte was touched"
            ),
            Self::ThreadIdentity { os_error } => write!(
                f,
                "detour patch refused: a thread of this process could not be \
                 identified (GetThreadId, os error {os_error}), so it could not \
                 be recorded as held or named; no target byte was touched"
            ),
            Self::CapacityExceeded { buffer, capacity } => write!(
                f,
                "detour patch refused: the {buffer} buffer reserved before the \
                 freeze ({capacity} entries) was exhausted while threads were \
                 held, and growing it would take the allocator's lock that a \
                 suspended thread may hold; no target byte was touched"
            ),
            Self::MinHookLock(status) => write!(
                f,
                "detour patch refused: MinHook's library lock could not be taken                  before the freeze ({status:?}); it is taken with the process                  still running precisely so that no suspended thread can be                  holding it, and no target byte was touched"
            ),
            Self::TargetExecuting { ranges } => write!(
                f,
                "detour patch refused: a thread of this process is stopped \
                 inside one of the {ranges} byte ranges this operation would \
                 rewrite or free, so patching would resume it into a half-old, \
                 half-new instruction stream; no target byte was touched and \
                 the attempt can be repeated once the game has run on"
            ),
            Self::MinHookQuery(status) => write!(
                f,
                "detour patch refused: MinHook could not report which bytes the \
                 operation would rewrite ({status:?}), so the check that no \
                 thread is stopped inside them could not be made; no target \
                 byte was touched"
            ),
        }
    }
}

impl std::error::Error for FreezeRefusal {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThreadDisposition {
    Gone,
    Refuse,
}

pub(crate) fn granted_access_covers_the_freeze(granted: u32) -> bool {
    granted & FREEZE_THREAD_ACCESS == FREEZE_THREAD_ACCESS
}

pub(crate) fn walk_stop_is_end_of_list(status: i32) -> bool {
    status == STATUS_NO_MORE_ENTRIES
}

pub(crate) fn leaves_room_for_minhooks_suspension(previous_count: u32) -> bool {
    previous_count
        .saturating_add(1)
        .saturating_add(SUSPENSIONS_STILL_TO_COME)
        <= MAXIMUM_SUSPEND_COUNT
}

pub(crate) fn pass_is_quiet(newly_suspended: u32, vanished: u32, unsettled: u32) -> bool {
    newly_suspended == 0 && vanished == 0 && unsettled == 0
}

pub(crate) fn classify_suspend_failure(signalled: bool) -> ThreadDisposition {
    if signalled {
        ThreadDisposition::Gone
    } else {
        ThreadDisposition::Refuse
    }
}

/// Two threads freezing concurrently deadlock with certainty, not by
static FREEZE_LOCK: Mutex<()> = Mutex::new(());

static RESUME_FAILURES: AtomicU64 = AtomicU64::new(0);

unsafe fn resume_or_count(handle: HANDLE) {
    let mut attempts = RESUME_ATTEMPTS;
    while resume_attempt_is_refused() || unsafe { ResumeThread(handle) } == u32::MAX {
        attempts -= 1;
        if attempts == 0 {
            if unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0 {
                RESUME_FAILURES.fetch_add(1, Ordering::AcqRel);
            }
            break;
        }
    }
}

const RESUME_ATTEMPTS: u32 = 3;

/// channel receive, a lock another thread holds -- deadlocks the process with
pub struct ProcessFreeze {
    accounted: Vec<AccountedThread>,
    roster: Vec<u8>,
    /// Held for the lifetime of the freeze, and released only once the buffers
    _serialisation: MutexGuard<'static, ()>,
    footprints: Vec<(usize, usize)>,
    /// MinHook's library lock, taken BEFORE the first suspension and released
    /// `Drop::drop` has run, so this is released after the buffers are freed
    /// and after `_serialisation`: acquire MinHook then the freeze lock, release
    _minhook_lock: minhook::MinHookLock,
    passes: u32,
    timing: Option<FreezeTiming>,
}

struct AccountedThread {
    id: u32,
    handle: HANDLE,
    suspended: bool,
}

#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

const _: () = assert!(core::mem::align_of::<AlignedContext>() == 16);

impl ProcessFreeze {
    pub fn frozen_thread_count(&self) -> usize {
        self.accounted
            .iter()
            .filter(|entry| entry.suspended)
            .count()
    }

    pub fn passes(&self) -> u32 {
        self.passes
    }

    /// freeze that owns the lock, and no way to acquire it a second time --
    pub(crate) fn minhook_lock(&self) -> &minhook::MinHookLock {
        &self._minhook_lock
    }

    pub(crate) fn note_entry(
        &mut self,
        entry_point: FreezeEntryPoint,
        label: &'static str,
        target_count: u32,
    ) {
        if let Some(timing) = self.timing.as_mut() {
            timing.note_entry(entry_point, label, target_count);
        }
    }

    pub(crate) fn time_minhook<T>(&mut self, call: impl FnOnce(&minhook::MinHookLock) -> T) -> T {
        let started = Instant::now();
        let outcome = call(&self._minhook_lock);
        let span = started.elapsed();
        if let Some(timing) = self.timing.as_mut() {
            timing.note_minhook(span);
        }
        outcome
    }

    pub fn any_instruction_pointer_in(
        &self,
        ranges: &[(usize, usize)],
    ) -> Result<bool, FreezeRefusal> {
        let mut found = false;
        self.read_held_contexts(|rip| {
            if ranges
                .iter()
                .any(|(start, end)| rip >= *start && rip < *end)
            {
                found = true;
            }
        })?;
        Ok(found)
    }

    pub(crate) fn refuse_if_a_thread_is_in_a_queued_patch(&mut self) -> Result<(), FreezeRefusal> {
        let mut ordinal = 0u32;
        let mut ranges = core::mem::take(&mut self.footprints);
        ranges.clear();
        let filled = loop {
            match minhook::MinHook::pending_footprint(&self._minhook_lock, ordinal) {
                Ok(None) => break Ok(()),
                Ok(Some((_target, footprint))) => {
                    if ranges.len() == ranges.capacity() {
                        break Err(FreezeRefusal::CapacityExceeded {
                            buffer: "patch footprint table",
                            capacity: ranges.capacity(),
                        });
                    }
                    ranges.push(footprint.patch);
                    ordinal = ordinal.saturating_add(1);
                }
                Err(status) => break Err(FreezeRefusal::MinHookQuery(status)),
            }
        };
        let answer = filled.and_then(|()| self.refuse_if_a_thread_is_in_any_of(&ranges));
        self.footprints = ranges;
        answer
    }

    /// slot is a use-after-free the moment it resumes.
    pub(crate) fn refuse_if_a_thread_is_in(
        &mut self,
        targets: impl Iterator<Item = *mut c_void>,
        include_trampolines: bool,
    ) -> Result<(), FreezeRefusal> {
        let mut ranges = core::mem::take(&mut self.footprints);
        ranges.clear();
        let filled = (|| {
            for target in targets {
                let footprint = match minhook::MinHook::patch_footprint(&self._minhook_lock, target)
                {
                    Ok(footprint) => footprint,
                    // (`MH_ERROR_NOT_CREATED`, which the release paths read as
                    // already-released), and turning that into a freeze refusal
                    Err(minhook::MH_STATUS::MH_ERROR_NOT_CREATED) => continue,
                    Err(status) => return Err(FreezeRefusal::MinHookQuery(status)),
                };
                let wanted = if include_trampolines { 2 } else { 1 };
                if ranges.capacity() - ranges.len() < wanted {
                    return Err(FreezeRefusal::CapacityExceeded {
                        buffer: "patch footprint table",
                        capacity: ranges.capacity(),
                    });
                }
                ranges.push(footprint.patch);
                if include_trampolines {
                    ranges.push(footprint.trampoline);
                }
            }
            Ok(())
        })();
        let answer = filled.and_then(|()| self.refuse_if_a_thread_is_in_any_of(&ranges));
        self.footprints = ranges;
        answer
    }

    #[cfg(test)]
    pub(crate) fn refuse_if_a_thread_is_in_ranges(
        &self,
        ranges: &[(usize, usize)],
    ) -> Result<(), FreezeRefusal> {
        self.refuse_if_a_thread_is_in_any_of(ranges)
    }

    fn refuse_if_a_thread_is_in_any_of(
        &self,
        ranges: &[(usize, usize)],
    ) -> Result<(), FreezeRefusal> {
        if ranges.is_empty() {
            return self.settle_suspensions();
        }
        #[cfg(test)]
        let _range_check = faults::RangeCheckPhase::enter();
        if self.any_instruction_pointer_in(ranges)? {
            return Err(FreezeRefusal::TargetExecuting {
                ranges: ranges.len(),
            });
        }
        Ok(())
    }

    pub fn settle_suspensions(&self) -> Result<(), FreezeRefusal> {
        self.read_held_contexts(|_| {})
    }

    fn what_the_roster_names_that_we_do_not_hold(
        &mut self,
        walk: &ThreadWalk,
        current_thread_id: u32,
        our_process_id: usize,
    ) -> Result<Option<u32>, FreezeRefusal> {
        let filled = walk.read_roster(&mut self.roster)?;
        roster_names_an_unheld_thread(
            &self.roster[..filled],
            our_process_id,
            current_thread_id,
            |id| self.accounted.iter().any(|entry| entry.id == id),
        )
    }

    fn read_held_contexts(&self, mut visit: impl FnMut(usize)) -> Result<(), FreezeRefusal> {
        for entry in self.accounted.iter().filter(|entry| entry.suspended) {
            let mut context: AlignedContext = unsafe { std::mem::zeroed() };
            context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
            let refused = context_read_is_refused_for(entry.id);
            if refused || unsafe { GetThreadContext(entry.handle, &mut context.0) } == 0 {
                return Err(FreezeRefusal::ThreadContext {
                    thread: entry.id,
                    os_error: if refused {
                        FAULT_OS_ERROR
                    } else {
                        unsafe { GetLastError() }
                    },
                });
            }
            visit(context.0.Rip as usize);
        }
        Ok(())
    }
}

impl Drop for ProcessFreeze {
    fn drop(&mut self) {
        for entry in self.accounted.drain(..).rev() {
            unsafe {
                if entry.suspended {
                    resume_or_count(entry.handle);
                }
                CloseHandle(entry.handle);
            }
        }
        if let Some(timing) = self.timing.take() {
            crate::freeze_census::record(timing.close(Instant::now(), self.passes));
        }
    }
}

pub(crate) fn frozen_window_buffer_bound(thread_count: usize) -> usize {
    thread_count.saturating_add(thread_count.saturating_mul(4).saturating_add(64))
}

type NtGetNextThreadFn =
    unsafe extern "system" fn(HANDLE, HANDLE, u32, u32, u32, *mut HANDLE) -> i32;

type NtQueryObjectFn = unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

#[repr(C)]
struct ObjectBasicInformation {
    attributes: u32,
    granted_access: u32,
    handle_count: u32,
    pointer_count: u32,
    reserved: [u32; 10],
}

type NtQuerySystemInformationFn = unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;

const SYSTEM_PROCESS_INFORMATION: u32 = 5;

const STATUS_INFO_LENGTH_MISMATCH: i32 = -1_073_741_820;

const ROSTER_PROCESS_ENTRY_BYTES: usize = 0x100;
const ROSTER_THREAD_ENTRY_BYTES: usize = 0x50;
const ROSTER_NEXT_ENTRY_OFFSET: usize = 0x00;
const ROSTER_THREAD_COUNT_OFFSET: usize = 0x04;
const ROSTER_PROCESS_ID_OFFSET: usize = 0x50;
const ROSTER_THREAD_PROCESS_OFFSET: usize = 0x28;
const ROSTER_THREAD_ID_OFFSET: usize = 0x30;

#[derive(Clone, Copy)]
pub(crate) struct ThreadWalk {
    next: NtGetNextThreadFn,
    query: NtQueryObjectFn,
    roster: NtQuerySystemInformationFn,
    process: HANDLE,
}

impl ThreadWalk {
    fn resolve() -> Result<Self, FreezeRefusal> {
        let ntdll: [u16; 10] = [
            b'n' as u16,
            b't' as u16,
            b'd' as u16,
            b'l' as u16,
            b'l' as u16,
            b'.' as u16,
            b'd' as u16,
            b'l' as u16,
            b'l' as u16,
            0,
        ];
        let module = unsafe { GetModuleHandleW(ntdll.as_ptr()) };
        if module.is_null() {
            return Err(FreezeRefusal::EnumeratorUnavailable {
                os_error: unsafe { GetLastError() },
            });
        }
        let next = unsafe { GetProcAddress(module, c"NtGetNextThread".as_ptr().cast()) };
        let Some(next) = next else {
            return Err(FreezeRefusal::EnumeratorUnavailable {
                os_error: unsafe { GetLastError() },
            });
        };
        let query = unsafe { GetProcAddress(module, c"NtQueryObject".as_ptr().cast()) };
        let Some(query) = query else {
            return Err(FreezeRefusal::EnumeratorUnavailable {
                os_error: unsafe { GetLastError() },
            });
        };
        let roster = unsafe { GetProcAddress(module, c"NtQuerySystemInformation".as_ptr().cast()) };
        let Some(roster) = roster else {
            return Err(FreezeRefusal::EnumeratorUnavailable {
                os_error: unsafe { GetLastError() },
            });
        };
        Ok(Self {
            next: unsafe {
                core::mem::transmute::<unsafe extern "system" fn() -> isize, NtGetNextThreadFn>(
                    next,
                )
            },
            query: unsafe {
                core::mem::transmute::<unsafe extern "system" fn() -> isize, NtQueryObjectFn>(query)
            },
            roster: unsafe {
                core::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    NtQuerySystemInformationFn,
                >(roster)
            },
            process: unsafe { GetCurrentProcess() },
        })
    }

    /// # Ownership needs no check any more
    fn next_thread(&self, cursor: HANDLE) -> Result<Option<HANDLE>, FreezeRefusal> {
        let mut handle: HANDLE = core::ptr::null_mut();
        let status =
            unsafe { (self.next)(self.process, cursor, MAXIMUM_ALLOWED, 0, 0, &mut handle) };
        if status == 0 {
            return Ok(Some(handle));
        }
        if walk_stop_is_end_of_list(status) {
            return Ok(None);
        }
        Err(FreezeRefusal::Enumeration(status as u32))
    }

    fn granted_access(&self, handle: HANDLE) -> Result<u32, FreezeRefusal> {
        let mut information: ObjectBasicInformation = unsafe { core::mem::zeroed() };
        let mut written = 0u32;
        let status = unsafe {
            (self.query)(
                handle,
                OBJECT_BASIC_INFORMATION,
                (&mut information) as *mut ObjectBasicInformation as *mut c_void,
                core::mem::size_of::<ObjectBasicInformation>() as u32,
                &mut written,
            )
        };
        if status != 0 {
            return Err(FreezeRefusal::ThreadRightsUnreadable {
                status: status as u32,
            });
        }
        Ok(information.granted_access)
    }

    fn roster_bytes_needed(&self) -> Result<usize, FreezeRefusal> {
        let mut needed = 0u32;
        let status = unsafe {
            (self.roster)(
                SYSTEM_PROCESS_INFORMATION,
                core::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        if status != STATUS_INFO_LENGTH_MISMATCH && status < 0 {
            return Err(FreezeRefusal::RosterUnavailable { status });
        }
        Ok(needed as usize)
    }

    fn read_roster(&self, buffer: &mut [u8]) -> Result<usize, FreezeRefusal> {
        let mut written = 0u32;
        let status = unsafe {
            (self.roster)(
                SYSTEM_PROCESS_INFORMATION,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut written,
            )
        };
        if status < 0 {
            return Err(FreezeRefusal::RosterUnavailable { status });
        }
        let written = written as usize;
        if written > buffer.len() {
            return Err(FreezeRefusal::RosterUnreadable);
        }
        Ok(written)
    }

    fn thread_count(&self) -> Result<usize, FreezeRefusal> {
        let mut count = 0usize;
        let mut cursor: HANDLE = core::ptr::null_mut();
        loop {
            let next = self.next_thread(cursor);
            if !cursor.is_null() {
                unsafe { CloseHandle(cursor) };
            }
            match next? {
                None => return Ok(count),
                Some(handle) => {
                    count = count.saturating_add(1);
                    cursor = handle;
                }
            }
        }
    }
}

fn roster_names_an_unheld_thread(
    roster: &[u8],
    our_process_id: usize,
    current_thread_id: u32,
    is_held: impl Fn(u32) -> bool,
) -> Result<Option<u32>, FreezeRefusal> {
    let mut answer: Option<Option<u32>> = None;
    let mut at = 0usize;
    loop {
        if at.saturating_add(ROSTER_PROCESS_ENTRY_BYTES) > roster.len() {
            return Err(FreezeRefusal::RosterUnreadable);
        }
        let process_id = read_usize(roster, at + ROSTER_PROCESS_ID_OFFSET);
        let next = read_u32(roster, at + ROSTER_NEXT_ENTRY_OFFSET) as usize;
        if next != 0
            && (next < ROSTER_PROCESS_ENTRY_BYTES
                || next % core::mem::size_of::<usize>() != 0
                || at.saturating_add(next) > roster.len())
        {
            return Err(FreezeRefusal::RosterUnreadable);
        }
        let listed = read_u32(roster, at + ROSTER_THREAD_COUNT_OFFSET) as usize;
        let threads = at + ROSTER_PROCESS_ENTRY_BYTES;
        let entry_end = if next == 0 {
            roster.len()
        } else {
            at.saturating_add(next)
        };
        if threads.saturating_add(listed.saturating_mul(ROSTER_THREAD_ENTRY_BYTES)) > entry_end {
            return Err(FreezeRefusal::RosterUnreadable);
        }
        for index in 0..listed {
            let record = threads + index * ROSTER_THREAD_ENTRY_BYTES;
            if read_usize(roster, record + ROSTER_THREAD_PROCESS_OFFSET) != process_id {
                return Err(FreezeRefusal::RosterUnreadable);
            }
            let id = read_usize(roster, record + ROSTER_THREAD_ID_OFFSET);
            if u32::try_from(id).is_err() {
                return Err(FreezeRefusal::RosterUnreadable);
            }
            let idle_processors = process_id == 0 && id == 0;
            if !idle_processors {
                for earlier in 0..index {
                    let seen = threads + earlier * ROSTER_THREAD_ENTRY_BYTES;
                    if read_usize(roster, seen + ROSTER_THREAD_ID_OFFSET) == id {
                        return Err(FreezeRefusal::RosterUnreadable);
                    }
                }
            }
        }
        if process_id == our_process_id {
            if answer.is_some() {
                return Err(FreezeRefusal::RosterUnreadable);
            }
            let mut lists_this_thread = false;
            let mut unheld = None;
            for index in 0..listed {
                let entry = threads + index * ROSTER_THREAD_ENTRY_BYTES;
                let Ok(id) = u32::try_from(read_usize(roster, entry + ROSTER_THREAD_ID_OFFSET))
                else {
                    return Err(FreezeRefusal::RosterUnreadable);
                };
                if id == current_thread_id {
                    lists_this_thread = true;
                    continue;
                }
                if unheld.is_none() && !is_held(id) {
                    unheld = Some(id);
                }
            }
            if !lists_this_thread {
                return Err(FreezeRefusal::RosterUnreadable);
            }
            answer = Some(unheld);
        }
        if next == 0 {
            break;
        }
        at = at.saturating_add(next);
    }
    answer.ok_or(FreezeRefusal::RosterUnreadable)
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_usize(bytes: &[u8], at: usize) -> usize {
    let mut raw = [0u8; core::mem::size_of::<usize>()];
    raw.copy_from_slice(&bytes[at..at + core::mem::size_of::<usize>()]);
    usize::from_ne_bytes(raw)
}

pub(crate) static ROSTER_CONFIRMATIONS_REFUSED: AtomicU64 = AtomicU64::new(0);

fn thread_has_terminated(handle: HANDLE) -> bool {
    unsafe { WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 }
}

/// take a lock here is the deadlock this whole module is shaped to avoid; and
#[cfg(test)]
pub(crate) mod faults {
    use std::sync::atomic::{AtomicU32, Ordering};

    pub(crate) static SUSPEND_FAULT_THREAD: AtomicU32 = AtomicU32::new(0);
    pub(crate) static SUSPEND_FAULT_HITS: AtomicU32 = AtomicU32::new(0);
    pub(crate) static CONTEXT_FAULT_THREAD: AtomicU32 = AtomicU32::new(0);
    pub(crate) static CONTEXT_FAULT_MODE: AtomicU32 = AtomicU32::new(0);
    pub(crate) static CONTEXT_FAULT_HITS: AtomicU32 = AtomicU32::new(0);
    pub(crate) static IN_RANGE_CHECK: AtomicU32 = AtomicU32::new(0);
    pub(crate) static RESUME_FAULT_ARMED: AtomicU32 = AtomicU32::new(0);
    pub(crate) static RESUME_FAULT_HITS: AtomicU32 = AtomicU32::new(0);

    pub(crate) struct RangeCheckPhase;

    impl RangeCheckPhase {
        pub(crate) fn enter() -> Self {
            IN_RANGE_CHECK.fetch_add(1, Ordering::AcqRel);
            Self
        }
    }

    impl Drop for RangeCheckPhase {
        fn drop(&mut self) {
            IN_RANGE_CHECK.fetch_sub(1, Ordering::AcqRel);
        }
    }

    pub(crate) struct Armed;

    impl Drop for Armed {
        fn drop(&mut self) {
            SUSPEND_FAULT_THREAD.store(0, Ordering::Release);
            CONTEXT_FAULT_THREAD.store(0, Ordering::Release);
            CONTEXT_FAULT_MODE.store(0, Ordering::Release);
            RESUME_FAULT_ARMED.store(0, Ordering::Release);
        }
    }

    pub(crate) fn refuse_one_resume_attempt() -> Armed {
        RESUME_FAULT_HITS.store(0, Ordering::Release);
        RESUME_FAULT_ARMED.store(1, Ordering::Release);
        Armed
    }

    pub(crate) fn refuse_suspension_of(thread: u32) -> Armed {
        SUSPEND_FAULT_HITS.store(0, Ordering::Release);
        SUSPEND_FAULT_THREAD.store(thread, Ordering::Release);
        Armed
    }

    pub(crate) fn refuse_context_of(thread: u32, mode: u32) -> Armed {
        CONTEXT_FAULT_HITS.store(0, Ordering::Release);
        CONTEXT_FAULT_MODE.store(mode, Ordering::Release);
        CONTEXT_FAULT_THREAD.store(thread, Ordering::Release);
        Armed
    }
}

const FAULT_OS_ERROR: u32 = 5;

#[cfg(not(test))]
#[inline(always)]
fn suspension_is_refused_for(_thread_id: u32) -> bool {
    false
}

#[cfg(test)]
fn suspension_is_refused_for(thread_id: u32) -> bool {
    if thread_id == 0 || faults::SUSPEND_FAULT_THREAD.load(Ordering::Acquire) != thread_id {
        return false;
    }
    unsafe { SetLastError(FAULT_OS_ERROR) };
    faults::SUSPEND_FAULT_HITS.fetch_add(1, Ordering::AcqRel);
    true
}

#[cfg(not(test))]
#[inline(always)]
fn context_read_is_refused_for(_thread_id: u32) -> bool {
    false
}

#[cfg(not(test))]
#[inline(always)]
fn resume_attempt_is_refused() -> bool {
    false
}

#[cfg(test)]
fn resume_attempt_is_refused() -> bool {
    if faults::RESUME_FAULT_ARMED.swap(0, Ordering::AcqRel) == 0 {
        return false;
    }
    faults::RESUME_FAULT_HITS.fetch_add(1, Ordering::AcqRel);
    true
}

#[cfg(test)]
fn context_read_is_refused_for(thread_id: u32) -> bool {
    if thread_id == 0 || faults::CONTEXT_FAULT_THREAD.load(Ordering::Acquire) != thread_id {
        return false;
    }
    let mode = faults::CONTEXT_FAULT_MODE.load(Ordering::Acquire);
    if mode == 2 && faults::IN_RANGE_CHECK.load(Ordering::Acquire) == 0 {
        return false;
    }
    faults::CONTEXT_FAULT_HITS.fetch_add(1, Ordering::AcqRel);
    true
}

pub fn freeze_process() -> Result<ProcessFreeze, FreezeRefusal> {
    freeze_other_threads()
}

/// Held suspensions must be released promptly. The returned guard does that on
pub(crate) fn freeze_other_threads() -> Result<ProcessFreeze, FreezeRefusal> {
    let ordinal = FREEZE_ATTEMPTS
        .fetch_add(1, Ordering::AcqRel)
        .saturating_add(1);
    THREAD_FREEZE_ATTEMPTS.with(|count| count.set(count.get().saturating_add(1)));
    // on how long the game was stopped and `acquire_us` is an upper bound on the
    let started = Instant::now();
    SUSPENDED_ANY_THIS_FREEZE.with(|flag| flag.set(false));
    match freeze_other_threads_inner() {
        Ok(mut freeze) => {
            freeze.timing = Some(FreezeTiming::new(ordinal, started, Instant::now()));
            Ok(freeze)
        }
        Err(refusal) => {
            FREEZE_REFUSALS.fetch_add(1, Ordering::AcqRel);
            if SUSPENDED_ANY_THIS_FREEZE.with(|flag| flag.get()) {
                crate::freeze_census::record(crate::freeze_census::refused_window(
                    ordinal,
                    started,
                    Instant::now(),
                ));
            }
            Err(refusal)
        }
    }
}

fn account_for(
    freeze: &mut ProcessFreeze,
    id: u32,
    handle: HANDLE,
    suspended: bool,
) -> Result<(), FreezeRefusal> {
    if freeze.accounted.len() == freeze.accounted.capacity() {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "accounted thread table",
            capacity: freeze.accounted.capacity(),
        });
    }
    freeze.accounted.push(AccountedThread {
        id,
        handle,
        suspended,
    });
    Ok(())
}

fn freeze_other_threads_inner() -> Result<ProcessFreeze, FreezeRefusal> {
    // let two freezers deadlock against each other. It is also the reason this
    let minhook_lock = minhook::MinHookLock::acquire().map_err(FreezeRefusal::MinHookLock)?;
    let serialisation = FREEZE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let walk = ThreadWalk::resolve()?;

    let thread_count = walk.thread_count()?;

    let bound = frozen_window_buffer_bound(thread_count);
    let mut accounted: Vec<AccountedThread> = Vec::new();
    if accounted.try_reserve(bound).is_err() {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "accounted thread table",
            capacity: accounted.capacity(),
        });
    }
    let roster_needed = walk.roster_bytes_needed()?;
    let roster_bytes = roster_needed
        .saturating_add(roster_needed / 2)
        .saturating_add(1024 * 1024);
    let mut roster: Vec<u8> = Vec::new();
    if roster.try_reserve(roster_bytes).is_err() {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "system thread roster",
            capacity: roster.capacity(),
        });
    }
    roster.resize(roster_bytes, 0);
    let hooks_held =
        minhook::MinHook::hook_count(&minhook_lock).map_err(FreezeRefusal::MinHookLock)? as usize;
    let mut footprints: Vec<(usize, usize)> = Vec::new();
    if footprints
        .try_reserve(hooks_held.saturating_mul(2).saturating_add(8))
        .is_err()
    {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "patch footprint table",
            capacity: footprints.capacity(),
        });
    }

    let mut freeze = ProcessFreeze {
        accounted,
        roster,
        _serialisation: serialisation,
        footprints,
        _minhook_lock: minhook_lock,
        passes: 0,
        timing: None,
    };

    let current_thread_id = unsafe { GetCurrentThreadId() };
    let our_process_id = unsafe { GetCurrentProcessId() } as usize;
    let mut unsettled_suspend: Option<(u32, u32)> = None;
    let mut unaccounted = 0u32;
    let mut roster_unheld: Option<u32> = None;
    for pass in 1..=MAX_PASSES {
        freeze.passes = pass;
        unsettled_suspend = None;
        FREEZE_PASSES_SPENT.fetch_add(1, Ordering::AcqRel);
        roster_unheld = None;
        let mut newly_suspended = 0u32;
        let mut vanished = 0u32;
        let mut unsettled = 0u32;
        let mut cursor: HANDLE = core::ptr::null_mut();
        let mut cursor_is_kept = false;
        loop {
            let stepped = walk.next_thread(cursor);
            if !cursor.is_null() && !cursor_is_kept {
                unsafe { CloseHandle(cursor) };
            }
            cursor = core::ptr::null_mut();
            cursor_is_kept = false;
            let Some(handle) = stepped? else { break };
            cursor = handle;
            let granted = match walk.granted_access(handle) {
                Ok(granted) => granted,
                Err(refusal) => {
                    unsafe { CloseHandle(handle) };
                    return Err(refusal);
                }
            };
            if !granted_access_covers_the_freeze(granted) {
                let thread = unsafe { GetThreadId(handle) };
                unsafe { CloseHandle(handle) };
                return Err(FreezeRefusal::ThreadRights { thread, granted });
            }
            let thread_id = unsafe { GetThreadId(handle) };
            if thread_id == 0 {
                let os_error = unsafe { GetLastError() };
                unsafe { CloseHandle(handle) };
                return Err(FreezeRefusal::ThreadIdentity { os_error });
            }
            if thread_id == current_thread_id {
                continue;
            }
            if freeze.accounted.iter().any(|entry| entry.id == thread_id) {
                continue;
            }
            let previous_count = if suspension_is_refused_for(thread_id) {
                u32::MAX
            } else {
                unsafe { SuspendThread(handle) }
            };
            if previous_count != u32::MAX {
                SUSPENDED_ANY_THIS_FREEZE.with(|flag| flag.set(true));
            }
            if previous_count != u32::MAX && !leaves_room_for_minhooks_suspension(previous_count) {
                unsafe {
                    resume_or_count(handle);
                    CloseHandle(handle);
                }
                return Err(FreezeRefusal::SuspendCeiling {
                    thread: thread_id,
                    previous_count,
                });
            }
            if previous_count == u32::MAX {
                let os_error = unsafe { GetLastError() };
                let disposition = classify_suspend_failure(thread_has_terminated(handle));
                match disposition {
                    ThreadDisposition::Gone => {
                        SUSPEND_FAILURES_EXCUSED_AS_TERMINATED.fetch_add(1, Ordering::AcqRel);
                        if let Err(refusal) = account_for(&mut freeze, thread_id, handle, false) {
                            unsafe { CloseHandle(handle) };
                            return Err(refusal);
                        }
                        cursor_is_kept = true;
                        vanished += 1;
                        continue;
                    }
                    ThreadDisposition::Refuse => {
                        SUSPEND_FAILURES_DEFERRED_TO_A_LATER_PASS.fetch_add(1, Ordering::AcqRel);
                        unsettled_suspend = Some((thread_id, os_error));
                        unsettled += 1;
                        continue;
                    }
                }
            }
            if let Err(refusal) = account_for(&mut freeze, thread_id, handle, true) {
                unsafe {
                    resume_or_count(handle);
                    CloseHandle(handle);
                }
                return Err(refusal);
            }
            cursor_is_kept = true;
            newly_suspended += 1;
        }
        unaccounted = newly_suspended
            .saturating_add(vanished)
            .saturating_add(unsettled);
        if pass_is_quiet(newly_suspended, vanished, unsettled) {
            // RELEASES it, and copies that thread's record outside the lock --
            // `+0x578.Flink` re-read under the reacquired lock, and both insert
            match freeze.what_the_roster_names_that_we_do_not_hold(
                &walk,
                current_thread_id,
                our_process_id,
            )? {
                None => return Ok(freeze),
                Some(thread) => {
                    ROSTER_CONFIRMATIONS_REFUSED.fetch_add(1, Ordering::AcqRel);
                    unsettled_suspend = None;
                    unaccounted = 1;
                    roster_unheld = Some(thread);
                }
            }
        }
        if newly_suspended > 0 && pass < MAX_PASSES {
            freeze.settle_suspensions()?;
        }
    }

    if let Some((thread, os_error)) = unsettled_suspend {
        return Err(FreezeRefusal::ThreadSuspend { thread, os_error });
    }
    if let Some(thread) = roster_unheld {
        return Err(FreezeRefusal::RosterUnheld {
            thread,
            passes: MAX_PASSES,
        });
    }
    Err(FreezeRefusal::NotQuiescent {
        passes: MAX_PASSES,
        unaccounted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
    use std::sync::Arc;

    #[test]
    fn the_freeze_access_mask_covers_everything_minhook_asks_for() {
        assert_eq!(
            MINHOOK_THREAD_ACCESS & FREEZE_THREAD_ACCESS,
            MINHOOK_THREAD_ACCESS,
            "the freeze mask must be a superset of MinHook's THREAD_ACCESS"
        );
        assert_ne!(
            FREEZE_THREAD_ACCESS & SYNCHRONIZE,
            0,
            "SYNCHRONIZE is required for the termination proof"
        );
    }

    #[test]
    fn a_handle_short_of_the_freeze_access_is_never_good_enough() {
        assert!(granted_access_covers_the_freeze(FREEZE_THREAD_ACCESS));
        assert!(granted_access_covers_the_freeze(0xFFFF_FFFF));
        assert!(!granted_access_covers_the_freeze(0));
        assert!(!granted_access_covers_the_freeze(0x0006_0000));
        for missing in [
            THREAD_SUSPEND_RESUME,
            THREAD_GET_CONTEXT,
            THREAD_QUERY_INFORMATION,
            THREAD_SET_CONTEXT,
            SYNCHRONIZE,
        ] {
            assert!(
                !granted_access_covers_the_freeze(FREEZE_THREAD_ACCESS & !missing),
                "dropping {missing:#010x} from the granted access must not still \
                 count as covering the freeze"
            );
        }
    }

    #[test]
    fn only_the_end_of_the_thread_list_ends_a_walk() {
        assert!(walk_stop_is_end_of_list(STATUS_NO_MORE_ENTRIES));
        for status in [
            0i32,
            0xC000_0061u32 as i32,
            0xC000_0022u32 as i32,
            0xC000_000Du32 as i32,
            0x8000_001Bu32 as i32,
            1,
        ] {
            assert!(
                !walk_stop_is_end_of_list(status),
                "status {status:#010x} is a broken walk, not the end of one"
            );
        }
    }

    #[test]
    fn the_kernel_walk_lists_every_thread_the_system_snapshot_lists() {
        const ROOM: usize = 4096;

        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicU32::new(0));
        let workers: Vec<_> = (0..3)
            .map(|_| {
                let stop = Arc::clone(&stop);
                let started = Arc::clone(&started);
                std::thread::spawn(move || {
                    started.fetch_add(1, Ordering::AcqRel);
                    while !stop.load(Ordering::Acquire) {
                        std::hint::spin_loop();
                    }
                })
            })
            .collect();
        while started.load(Ordering::Acquire) < 3 {
            std::thread::yield_now();
        }

        let mut walked: Vec<u32> = Vec::with_capacity(ROOM);
        let mut snapshotted: Vec<u32> = Vec::with_capacity(ROOM);
        let resolved = ThreadWalk::resolve();
        let outcome = match resolved {
            Err(_) => None,
            Ok(walker) => {
                let freeze = freeze_other_threads();
                match freeze {
                    Ok(freeze) => {
                        let walk = fill_with_walked_thread_ids(&walker, &mut walked);
                        let snapshot = fill_with_snapshotted_thread_ids(&mut snapshotted);
                        drop(freeze);
                        Some((walk, snapshot))
                    }
                    Err(_) => None,
                }
            }
        };

        stop.store(true, Ordering::Release);
        for worker in workers {
            worker.join().expect("worker joins");
        }

        let (walk, snapshot) =
            outcome.expect("a test process must be freezable for this comparison");
        walk.expect("the kernel walk answers");
        assert!(snapshot, "the system snapshot answers");
        assert!(
            walked.len() < ROOM && snapshotted.len() < ROOM,
            "one of the enumerations filled its buffer ({} walked, {} \
             snapshotted, room for {ROOM}), so the comparison below is partial",
            walked.len(),
            snapshotted.len()
        );
        assert!(
            snapshotted.len() > 1,
            "this process has one thread, so the comparison below would prove \
             nothing"
        );
        for id in &snapshotted {
            assert!(
                walked.contains(id),
                "thread {id} is in this process's system snapshot and NOT in the \
                 kernel walk; the walk skips threads it cannot open at the \
                 access it asks for, so a narrowed mask hides exactly the thread \
                 the freeze cannot stop"
            );
        }
    }

    #[test]
    fn a_thread_this_process_cannot_open_at_the_freeze_rights_refuses_by_name() {
        use std::os::windows::io::AsRawHandle;
        let _serialised = os_suspend_state_guard();

        let stop = Arc::new(AtomicBool::new(false));
        let denied = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let denied = Arc::clone(&denied);
            std::thread::spawn(move || {
                denied.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        while denied.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }
        let denied_id = denied.load(Ordering::Acquire);

        let set = deny_every_access_to(worker.as_raw_handle() as HANDLE);
        let outcome = if set == 0 {
            Some(freeze_other_threads().map(|freeze| freeze.frozen_thread_count()))
        } else {
            None
        };

        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");

        let outcome = outcome.unwrap_or_else(|| {
            panic!(
                "NtSetSecurityObject refused the empty DACL ({set:#010x}), so \
                    this test could not manufacture the thread it exists to test"
            )
        });
        match outcome {
            Err(FreezeRefusal::ThreadRights { thread, granted }) => {
                assert!(
                    thread == denied_id || thread == 0,
                    "the refusal named thread {thread}, which is neither the \
                     thread whose access was denied ({denied_id}) nor the 0 that \
                     means its id could not be read either"
                );
                assert!(
                    !granted_access_covers_the_freeze(granted),
                    "the refusal reported granted access {granted:#010x}, which \
                     DOES cover the freeze, so it refused for some other reason"
                );
            }
            Err(other) => panic!(
                "a thread this process cannot open at the freeze's access must \
                 refuse as ThreadRights, not as {other:?}"
            ),
            Ok(held) => panic!(
                "the freeze SUCCEEDED holding {held} thread(s) while thread \
                 {denied_id} could not be opened at the access needed to suspend \
                 it; that thread was still running when the patch would have \
                 landed"
            ),
        }
    }

    fn fill_with_walked_thread_ids(
        walk: &ThreadWalk,
        out: &mut Vec<u32>,
    ) -> Result<(), FreezeRefusal> {
        let mut cursor: HANDLE = core::ptr::null_mut();
        loop {
            let stepped = walk.next_thread(cursor);
            if !cursor.is_null() {
                unsafe { CloseHandle(cursor) };
            }
            match stepped? {
                None => return Ok(()),
                Some(handle) => {
                    if out.len() < out.capacity() {
                        out.push(unsafe { GetThreadId(handle) });
                    }
                    cursor = handle;
                }
            }
        }
    }

    fn fill_with_snapshotted_thread_ids(out: &mut Vec<u32>) -> bool {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;

        let process_id = unsafe { GetCurrentProcessId() };
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
        if unsafe { Thread32First(snapshot, &mut entry) } != 0 {
            loop {
                if entry.th32OwnerProcessID == process_id && out.len() < out.capacity() {
                    out.push(entry.th32ThreadID);
                }
                if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
                    break;
                }
            }
        }
        unsafe { CloseHandle(snapshot) };
        true
    }

    #[repr(C)]
    struct AclHeader {
        revision: u8,
        sbz1: u8,
        size: u16,
        count: u16,
        sbz2: u16,
    }

    fn set_thread_dacl(thread: HANDLE, dacl: *mut c_void) -> i32 {
        #[repr(C)]
        struct AbsoluteSecurityDescriptor {
            revision: u8,
            sbz1: u8,
            control: u16,
            owner: *mut c_void,
            group: *mut c_void,
            sacl: *mut c_void,
            dacl: *mut c_void,
        }
        const DACL_SECURITY_INFORMATION: u32 = 4;
        const SE_DACL_PRESENT: u16 = 0x0004;
        type NtSetSecurityObjectFn = unsafe extern "system" fn(HANDLE, u32, *const c_void) -> i32;

        let descriptor = AbsoluteSecurityDescriptor {
            revision: 1,
            sbz1: 0,
            control: SE_DACL_PRESENT,
            owner: core::ptr::null_mut(),
            group: core::ptr::null_mut(),
            sacl: core::ptr::null_mut(),
            dacl,
        };
        let ntdll: [u16; 10] = [
            b'n' as u16,
            b't' as u16,
            b'd' as u16,
            b'l' as u16,
            b'l' as u16,
            b'.' as u16,
            b'd' as u16,
            b'l' as u16,
            b'l' as u16,
            0,
        ];
        let module = unsafe { GetModuleHandleW(ntdll.as_ptr()) };
        let set = unsafe { GetProcAddress(module, c"NtSetSecurityObject".as_ptr().cast()) }
            .expect("ntdll exports NtSetSecurityObject");
        let set: NtSetSecurityObjectFn = unsafe {
            core::mem::transmute::<unsafe extern "system" fn() -> isize, NtSetSecurityObjectFn>(set)
        };
        unsafe {
            set(
                thread,
                DACL_SECURITY_INFORMATION,
                (&descriptor) as *const AbsoluteSecurityDescriptor as *const c_void,
            )
        }
    }

    fn deny_every_access_to(thread: HANDLE) -> i32 {
        let mut empty_dacl = AclHeader {
            revision: 2,
            sbz1: 0,
            size: core::mem::size_of::<AclHeader>() as u16,
            count: 0,
            sbz2: 0,
        };
        set_thread_dacl(thread, (&mut empty_dacl) as *mut AclHeader as *mut c_void)
    }

    fn deny_every_access_including_the_owner_s_to(thread: HANDLE) -> i32 {
        const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
        const CREATOR_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 3];
        const OWNER_RIGHTS_RID: u32 = 4;

        #[repr(C, align(4))]
        struct OwnerRightsDacl {
            revision: u8,
            sbz1: u8,
            size: u16,
            count: u16,
            sbz2: u16,
            ace_type: u8,
            ace_flags: u8,
            ace_size: u16,
            mask: u32,
            sid_revision: u8,
            sid_sub_authority_count: u8,
            sid_identifier_authority: [u8; 6],
            sid_sub_authority: [u32; 1],
        }
        const _: () = assert!(core::mem::size_of::<OwnerRightsDacl>() == 28);

        let mut dacl = OwnerRightsDacl {
            revision: 2,
            sbz1: 0,
            size: core::mem::size_of::<OwnerRightsDacl>() as u16,
            count: 1,
            sbz2: 0,
            ace_type: ACCESS_ALLOWED_ACE_TYPE,
            ace_flags: 0,
            ace_size: (core::mem::size_of::<OwnerRightsDacl>() - core::mem::size_of::<AclHeader>())
                as u16,
            mask: 0,
            sid_revision: 1,
            sid_sub_authority_count: 1,
            sid_identifier_authority: CREATOR_AUTHORITY,
            sid_sub_authority: [OWNER_RIGHTS_RID],
        };
        set_thread_dacl(thread, (&mut dacl) as *mut OwnerRightsDacl as *mut c_void)
    }

    fn what_maximum_allowed_gets(walk: &ThreadWalk, thread_id: u32) -> Option<u32> {
        use windows_sys::Win32::System::Threading::OpenThread;
        let handle = unsafe { OpenThread(MAXIMUM_ALLOWED, 0, thread_id) };
        if handle.is_null() {
            return None;
        }
        let granted = walk.granted_access(handle).ok();
        unsafe { CloseHandle(handle) };
        Some(granted.unwrap_or(u32::MAX))
    }

    #[test]
    fn only_a_signalled_thread_excuses_a_suspend_failure() {
        assert_eq!(classify_suspend_failure(true), ThreadDisposition::Gone);
        assert_eq!(classify_suspend_failure(false), ThreadDisposition::Refuse);
    }

    #[test]
    fn the_termination_proof_answers_without_waiting() {
        const NO_WAIT_CEILING_MS: u128 = 25;

        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let live_thread_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let live_thread_id = Arc::clone(&live_thread_id);
            std::thread::spawn(move || {
                live_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        let mut thread_id = 0u32;
        while thread_id == 0 {
            thread_id = live_thread_id.load(Ordering::Acquire);
            std::thread::yield_now();
        }
        const PROBES: u32 = 8;
        let handle = {
            use std::os::windows::io::AsRawHandle;
            worker.as_raw_handle() as HANDLE
        };
        let identified = unsafe { GetThreadId(handle) } == thread_id;
        let mut verdict = true;
        let mut elapsed = std::time::Duration::MAX;
        for _ in 0..PROBES {
            let started = std::time::Instant::now();
            verdict = thread_has_terminated(handle);
            elapsed = elapsed.min(started.elapsed());
        }
        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");

        assert!(
            identified,
            "the handle probed must be the worker thread's own"
        );
        assert!(
            !verdict,
            "a thread that is still running is not proof of termination"
        );
        assert!(
            elapsed.as_millis() < NO_WAIT_CEILING_MS,
            "the fastest of {PROBES} termination proofs took {elapsed:?}, over \
             the {NO_WAIT_CEILING_MS} ms ceiling, so it is waiting on a thread \
             object again; it runs with the rest of the process suspended, and \
             an unsettled probe is deferred to the next pass rather than paid for"
        );
    }

    /// which is the deadlock the module header warns about -- fail here.
    #[test]
    fn only_the_freeze_lock_blocks_in_this_module() {
        let production = code_only(&production_code());
        assert_eq!(
            production.matches("WaitForSingleObject").count(),
            3,
            "this module names WaitForSingleObject somewhere other than its \
             import and its two zero-timeout probes; another wait blocks the \
             freezing thread with every other thread already suspended"
        );
        assert!(
            production.contains("WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0"),
            "the resume path's probe must be a ZERO-timeout wait too: it runs \
             while every thread after it in the resume loop is still suspended"
        );
        assert!(
            production.contains("WaitForSingleObject(handle, 0) == WAIT_OBJECT_0"),
            "the termination probe must be a ZERO-timeout wait; a timeout here \
             is a thread of this process blocked on a thread that is dying, with \
             the rest of the process already suspended behind it"
        );
        assert_eq!(
            production.matches(concat!(".lo", "ck()")).count(),
            2,
            "this module takes a lock beyond FREEZE_LOCK and the test-only \
             suspend-state guard; the freeze holds every other thread suspended, \
             so a new lock is one a suspended thread may already own with nobody \
             left to release it"
        );
        assert!(
            production.contains(concat!("FREEZE_LO", "CK.lock()")),
            "the one lock production takes must be FREEZE_LOCK, acquired before \
             the first suspension so that two freezers cannot interleave"
        );
        assert!(
            production.contains(concat!("OS_SUSPEND_STATE_TE", "STS.lock()")),
            "the second lock must still be the cfg(test) suspend-state guard; if \
             it is not, the count above is passing over a lock nobody named"
        );
        for blocking in [
            concat!("Sle", "ep("),
            concat!("sle", "ep("),
            concat!("yield_", "now"),
            concat!("park", "("),
            concat!("SwitchTo", "Thread"),
            concat!("WaitForMultiple", "Objects"),
            concat!("Sleep", "Ex"),
            concat!("Duration::", "from"),
            concat!("spin_", "loop"),
        ] {
            assert!(
                !production.contains(blocking),
                "this module's production code contains `{blocking}`; the freeze \
                 never sleeps and never spins, because everything it holds up is \
                 held up with the rest of the process suspended"
            );
        }
        assert_eq!(
            production.matches(concat!("Instant::", "now")).count(),
            5,
            "the freeze census takes exactly five clock reads in this module -- \
             two bracketing the acquisition, one closing a REFUSED attempt, one \
             at the end of the resume loop, one opening the caller's MinHook \
             span; a sixth is a read nobody accounted for"
        );
        assert_eq!(
            production.matches(concat!("elap", "sed(")).count(),
            1,
            "exactly one span is closed with `elapsed()`, the MinHook call's; \
             the others are closed by subtracting two named instants, which is \
             what lets a row say WHICH part of the window each one is"
        );
        for census_read in [
            "let started = Instant::now();",
            "Some(FreezeTiming::new(ordinal, started, Instant::now()))",
            "timing.close(Instant::now(), self.passes)",
        ] {
            assert!(
                production.contains(census_read),
                "the freeze census clock read `{census_read}` is not where this \
                 module says it is; a read that moved is a window measured over \
                 a different span than the row claims"
            );
        }
        assert!(
            production.contains(
                "pub(crate) fn time_minhook<T>(&mut self, call: impl FnOnce(&minhook::MinHookLock) -> T) -> T \
                 { let started = Instant::now(); let outcome = call(&self._minhook_lock); \
                 let span = started.elapsed(); if let Some(timing) = self.timing.as_mut() \
                 { timing.note_minhook(span); } outcome }"
            ),
            "`time_minhook` is no longer exactly the four statements this module \
             pins. Everything in it runs with the whole process suspended, so a \
             statement added there is a statement added to the stopped window -- \
             and a wait added there is the deleted bounded wait, whatever it is \
             built out of"
        );
        let census = code_only(&census_production_code());
        for blocking in [
            concat!("Sle", "ep("),
            concat!("sle", "ep("),
            concat!("yield_", "now"),
            concat!("park", "("),
            concat!("SwitchTo", "Thread"),
            concat!("WaitForSingle", "Object"),
            concat!("WaitForMultiple", "Objects"),
            concat!("Sleep", "Ex"),
            concat!("Duration::", "from"),
            concat!("spin_", "loop"),
            concat!(".lo", "ck()"),
        ] {
            assert!(
                !census.contains(blocking),
                "the freeze census module contains `{blocking}`; two of its \
                 functions are called with every other thread in the process \
                 suspended, so a wait or a lock in there is a wait or a lock in \
                 the frozen window"
            );
        }
        for inside_the_window in [
            "pub(crate) fn note_entry(&mut self, entry_point: FreezeEntryPoint, \
             label: &'static str, target_count: u32) { self.entry_point = entry_point; \
             self.label = label; self.target_count = target_count; }",
            "pub(crate) fn note_minhook(&mut self, span: std::time::Duration) \
             { self.minhook_us = saturating_micros(span); }",
            "fn saturating_micros(span: std::time::Duration) -> u32 \
             { u32::try_from(span.as_micros()).unwrap_or(u32::MAX) }",
        ] {
            assert!(
                census.contains(inside_the_window),
                "a freeze-census function called from inside the frozen window \
                 is no longer exactly what this module pins: `{inside_the_window}`"
            );
        }
        assert!(
            production.contains(
                "impl Drop for ProcessFreeze { fn drop(&mut self) \
                 { for entry in self.accounted.drain(..).rev() { unsafe \
                 { if entry.suspended { resume_or_count(entry.handle); } \
                 CloseHandle(entry.handle); } } if let Some(timing) = self.timing.take() \
                 { crate::freeze_census::record(timing.close(Instant::now(), self.passes)); } } }"
            ),
            "`ProcessFreeze::drop` is no longer exactly the resume loop followed \
             by the census row. Everything before that loop finishes runs with \
             the process still stopped, and reordering the row into it would \
             measure the measurement"
        );
    }

    #[test]
    fn the_frozen_window_handle_table_leaves_room_for_a_burst() {
        for count in [0usize, 1, 9, 69, 1_024] {
            let bound = frozen_window_buffer_bound(count);
            assert!(
                bound >= count,
                "the bound must at least hold the threads already walked"
            );
            assert!(
                bound > count,
                "and leave room for a burst racing the first pass"
            );
        }
        assert_eq!(frozen_window_buffer_bound(usize::MAX), usize::MAX);
    }

    fn raw_string_prefix(characters: &[char], index: usize, code: &str) -> Option<usize> {
        if code
            .chars()
            .next_back()
            .is_some_and(|previous| previous.is_alphanumeric() || previous == '_')
        {
            return None;
        }
        let prefix = match characters.get(index) {
            Some('r') => 1usize,
            Some('b') | Some('c') if characters.get(index + 1) == Some(&'r') => 2usize,
            _ => return None,
        };
        let mut at = index + prefix;
        while characters.get(at) == Some(&'#') {
            at += 1;
        }
        (characters.get(at) == Some(&'"')).then_some(prefix)
    }

    fn code_only(source: &str) -> String {
        // ONE pass, three states. Two separate passes could not be made
        // correct in either order, and both orders were tried and shipped:
        // blocks-first let a `/*` inside a `//` comment open a block that never
        // closed and silently discarded the rest of the function;
        // lines-first truncated `/* see https://example */` at its `//`,
        // deleting the closing `*/` and doing the same thing. A comment's
        // meaning depends on the state it starts in, so the state has to be
        // carried rather than approximated.
        let mut code = String::with_capacity(source.len());
        let characters: Vec<char> = source.chars().collect();
        let mut block_depth = 0usize;
        let mut in_line_comment = false;
        let mut index = 0usize;
        while index < characters.len() {
            let current = characters[index];
            let next = characters.get(index + 1).copied();
            if in_line_comment {
                // Only the newline ends it. `*/` inside it is text.
                if current == '\n' {
                    in_line_comment = false;
                    code.push('\n');
                }
                index += 1;
                continue;
            }
            if block_depth > 0 {
                // Rust's block comments NEST, and `//` inside one is text.
                if current == '/' && next == Some('*') {
                    block_depth += 1;
                    index += 2;
                    continue;
                }
                if current == '*' && next == Some('/') {
                    block_depth -= 1;
                    if block_depth == 0 {
                        // A block comment SEPARATES tokens. Deleting it without
                        // leaving whitespace turns the valid
                        // `let/* why */bound = ...` into `letbound = ...`, and
                        // the count below then failed a correct build.
                        code.push(' ');
                    }
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }
            if current == '/' && next == Some('/') {
                in_line_comment = true;
                index += 2;
                continue;
            }
            if current == '/' && next == Some('*') {
                block_depth += 1;
                index += 2;
                continue;
            }
            // STRING LITERALS ARE EMPTIED, and this is not tidiness. Every
            // structural pin below matches code SHAPES, and a literal can spell
            // any shape: deleting the settling guard and adding
            // `let _escape = "if newly_suspended > 0 && .. { .. }";` in its
            // place satisfied `block_body` and every `contains` while removing
            // the operation they exist to enforce (adversarial review, round
            // 21). The quotes are KEPT so the token structure survives; only
            // what is between them goes.
            //
            // Raw strings are handled by their own hash count, because `\"`
            // does not end one.
            //
            // CHAR LITERALS ARE EMPTIED TOO, in their two unambiguous shapes.
            // Leaving them was justified by "no needle fits in one", which
            // missed that the pins also COUNT BRACES: moving
            // `freeze.settle_suspensions()?` out of its guard and putting
            // `let _ = '{';` inside made `block_body` swallow the unconditional
            // call and the pin passed (adversarial review, round 22). `'` also
            // opens a LIFETIME, which carries no brace, so only `'x'` and
            // `'\x'` are treated as literals and a lifetime is left as text.
            if current == '\''
                && (characters.get(index + 2) == Some(&'\'')
                    || (characters.get(index + 1) == Some(&'\\')
                        && characters.get(index + 3) == Some(&'\'')))
            {
                code.push('\'');
                code.push('\'');
                index += if characters.get(index + 2) == Some(&'\'') {
                    3
                } else {
                    4
                };
                continue;
            }
            let raw_prefix = raw_string_prefix(&characters, index, &code);
            if current == '"' || raw_prefix.is_some() {
                let prefix = raw_prefix.unwrap_or(0);
                let hashes = if prefix > 0 {
                    let mut count = 0usize;
                    while characters.get(index + prefix + count) == Some(&'#') {
                        count += 1;
                    }
                    count
                } else {
                    0
                };
                let raw = prefix > 0;
                code.push('"');
                index += if raw { prefix + hashes + 1 } else { 1 };
                loop {
                    let Some(&character) = characters.get(index) else {
                        break;
                    };
                    if !raw && character == '\\' {
                        index += 2;
                        continue;
                    }
                    if character == '"' {
                        let closed = (0..hashes)
                            .all(|offset| characters.get(index + 1 + offset) == Some(&'#'));
                        if closed {
                            index += 1 + hashes;
                            break;
                        }
                    }
                    index += 1;
                }
                code.push('"');
                continue;
            }
            code.push(current);
            index += 1;
        }
        assert_eq!(
            block_depth, 0,
            "the scanned source has an unterminated block comment, so this scan \
             saw less of it than the compiler did"
        );
        // Finally, layout. Collapsing whitespace alone was NOT enough to make a
        // match survive rustfmt at a narrower width: wrapping a call across
        // lines also inserts a trailing comma before the `)`, so the collapsed
        // text still differed. Normalising the punctuation is what actually
        // makes these assertions independent of where the formatter broke.
        let collapsed = code.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(",)", ")")
            .replace(" ,", ",")
            // Method chains: rustfmt at a narrow width breaks BEFORE the dot,
            // so `ids.try_reserve(..)` becomes `ids\n    .try_reserve(..)` and
            // collapsed to `ids .try_reserve(..)`. Same statement, and the
            // assertions must not care where the line broke.
            .replace(" .", ".")
            .replace(". ", ".")
            // A block comment may sit between ANY two tokens, and the space
            // this filter leaves in its place must not become the difference
            // between two spellings of the same call. `foo/* why */(x)` and
            // `Vec/* why */::new()` are both valid and both rustfmt-stable.
            .replace(" (", "(")
            .replace(" ::", "::")
            .replace(":: ", "::")
    }

    fn production_code() -> String {
        let source = include_str!("freeze.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("freeze.rs has a test module")
            .0
            .to_string()
    }

    /// Every `fn` this module DEFINES, by name, from its normalised production
    /// text.
    ///
    /// Names only: the call closure needs somewhere to start, and a name is what
    /// a call site spells. Generic parameters and `impl` blocks are irrelevant
    /// here because the closure is keyed on the name appearing followed by `(`.
    fn production_fn_names(production: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = production;
        while let Some(at) = rest.find("fn ") {
            let after = &rest[at + 3..];
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            let name = &after[..end];
            if !name.is_empty()
                && after[end..].starts_with(['(', '<'])
                && !names.iter().any(|seen: &String| seen == name)
            {
                names.push(name.to_string());
            }
            rest = after;
        }
        names
    }

    /// One named `fn`'s body from the normalised production text, brace-matched.
    ///
    /// `None` when the name is not defined here, which is how a call to
    /// something this module does not own -- a Win32 import, a `std` method --
    /// leaves the closure rather than being followed into text that is not there.
    fn production_fn_body(production: &str, name: &str) -> Option<String> {
        // BOTH SPELLINGS. `fn helper(` and `fn helper<T>(` are the same
        // definition to a caller, and searching only the first left every
        // generic helper unscanned -- review round 7. A generic helper is
        // exactly where somebody would put a `Box::new` and not notice.
        let at = production
            .find(&format!("fn {name}("))
            .or_else(|| production.find(&format!("fn {name}<")))?;
        let open = production[at..].find('{')? + at;
        let mut depth = 0usize;
        for (offset, character) in production[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(production[open + 1..open + offset].to_string());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The census module's production half, scanned by the same blocking guard.
    ///
    /// IT IS NOT THIS FILE, AND THAT IS THE POINT. The closing review found the
    /// guard defeatable a third time: `FreezeTiming::note_minhook` is CALLED
    /// from inside the frozen window but DEFINED in `freeze_census.rs`, which
    /// `production_code()` does not reach -- so `for _ in 0..1 {
    /// std::hint::spin_loop(); }` added there would spin inside every
    /// MinHook-timed window while every assertion in this file still passed.
    ///
    /// A guard on the frozen window has to follow the window, not the file.
    fn census_production_code() -> String {
        let source = include_str!("freeze_census.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("freeze_census.rs has a test module")
            .0
            .to_string()
    }

    /// Falsifiable, and the reason the classifiers above are not decorative:
    /// fails if a fail-open decision is taken anywhere but through them.
    ///
    /// A mutation battery found the classifier tests survive a reversion at the
    /// CALL SITE -- put `newly_suspended == 0` back in the fixed point, or drop
    /// the granted-access check from the loop, and the classifiers stay green
    /// while the freeze fails open again. Neither decision can be driven from a
    /// test in every one of its forms: the vanish-with-a-successor interleaving
    /// is not schedulable, and a walk cannot be made to break on demand. So the
    /// call sites are pinned by text as well.
    ///
    /// The ACCESS arm is pinned here for the sharpest reason of the three, and
    /// it is a MEASURED one. The walk skips a thread it cannot open at the
    /// access it asks for and still reports a clean end
    /// (`runs/ix_batch_freeze/enumeration_cost_measured.txt`), so two properties
    /// have to hold together and neither implies the other: the walk asks for
    /// `MAXIMUM_ALLOWED`, and what it was actually granted is checked before the
    /// thread is suspended. `a_thread_this_process_cannot_open_at_the_freeze_
    /// rights_refuses_by_name` drives the pair end to end; this pins the shape
    /// so a refactor cannot keep the test green by accident.
    #[test]
    fn the_fail_open_decisions_are_taken_only_through_their_classifiers() {
        let production = production_code();
        let walk = production
            .split_once("fn next_thread(")
            .expect("next_thread is declared in production")
            .1
            // To the closing brace at column 4 (it is a method), and no further.
            .split_once("\n    }\n")
            .expect("next_thread has a closing brace")
            .0;
        let walk = code_only(walk);
        let walk = walk.as_str();
        assert!(
            walk.contains("MAXIMUM_ALLOWED"),
            "the walk must ask for MAXIMUM_ALLOWED; asking for the access this \
             module happens to need makes the kernel SKIP a thread it cannot \
             open at that access and still report a clean end, which hides \
             exactly the thread the freeze cannot stop"
        );
        assert!(
            !walk.contains("FREEZE_THREAD_ACCESS"),
            "the walk asks for FREEZE_THREAD_ACCESS; a thread that cannot be \
             opened at it is then silently absent from the enumeration instead \
             of refusing"
        );
        assert!(
            walk.contains("walk_stop_is_end_of_list(status)"),
            "next_thread must classify the status it stopped on; ending the walk \
             on any stop certifies a TRUNCATED thread list"
        );
        assert!(
            !walk.contains("Ok(None)")
                || walk.find("walk_stop_is_end_of_list(status)") < walk.find("Ok(None)"),
            "next_thread answers `end of list` before it has classified the stop"
        );

        // The function BODY, and its CODE. Both narrowings are load-bearing for
        // the counting assertions further down, and neither was there when they
        // were written:
        //
        //  * unbounded, `inner` ran to the end of the production source, so a
        //    count of "how many times does this function do X" was really a
        //    count over every function after it as well;
        //  * uncommented, a count includes prose. A comment that merely NAMES
        //    the thing being counted -- exactly the comment a careful maintainer
        //    writes next to it -- would have failed a correct build. A gate that
        //    punishes documenting the thing it guards does not survive contact
        //    with maintenance.
        let inner_body = production
            .split_once("fn freeze_other_threads_inner(")
            .expect("freeze_other_threads_inner is declared in production")
            .1
            .split_once("\n}\n")
            .expect("freeze_other_threads_inner has a closing brace")
            .0;
        let inner = code_only(inner_body);
        let inner = inner.as_str();
        // THE SCAN EMPTIES STRING LITERALS, and that is what every structural
        // pin below rests on: a literal can spell any code shape, so without it
        // the settling guard could be DELETED and re-spelled inside a `let _ =
        // "..";` that satisfies `block_body` and every `contains` here
        // (adversarial review, round 21). Proven on this file rather than
        // asserted: the production text contains string literals, and none of
        // their contents survive into what is scanned.
        assert!(
            code_only(&production_code()).contains("buffer: \"\""),
            "the scan must EMPTY string literals; `CapacityExceeded` names its \
             buffer with one, and if that text survives then so does any code \
             shape a literal is made to spell"
        );
        assert!(
            inner.contains("if pass_is_quiet(newly_suspended, vanished, unsettled) {"),
            "the fixed point must ask pass_is_quiet with ALL THREE counts; \
             asking about newly_suspended alone lets a pass that saw a thread \
             vanish be the last one, and dropping `unsettled` lets a pass that \
             could not settle a suspend failure be the last one -- which is the \
             fail-open the bounded wait was covering for"
        );
        fn after_block<'a>(source: &'a str, header: &str) -> &'a str {
            let start = source
                .find(header)
                .unwrap_or_else(|| panic!("the freeze must contain `{header}`"))
                + header.len();
            let mut depth = 1usize;
            for (offset, character) in source[start..].char_indices() {
                match character {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return &source[start + offset + 1..];
                        }
                    }
                    _ => {}
                }
            }
            panic!("`{header}` has no closing brace");
        }

        fn block_body<'a>(source: &'a str, header: &str) -> &'a str {
            let start = source
                .find(header)
                .unwrap_or_else(|| panic!("the freeze must contain `{header}`"))
                + header.len();
            let mut depth = 1usize;
            for (offset, character) in source[start..].char_indices() {
                match character {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return &source[start..start + offset];
                        }
                    }
                    _ => {}
                }
            }
            panic!("`{header}` has no closing brace");
        }

        let loop_body = block_body(inner, "for pass in 1..=MAX_PASSES {");
        let settle = loop_body.find("freeze.settle_suspensions()?;").expect(
            "every pass that suspends must force those suspensions to COMPLETE \
             INSIDE the loop, before the next walk; SuspendThread returns before \
             the kernel has stopped the thread, and a walk taken in that window \
             can miss a thread that did not exist yet",
        );
        let quiet_body = block_body(
            loop_body,
            "if pass_is_quiet(newly_suspended, vanished, unsettled) {",
        );
        assert!(
            quiet_body.contains(
                "match freeze.what_the_roster_names_that_we_do_not_hold(&walk, current_thread_id, our_process_id)? {"
            ),
            "a quiet pass must be CONFIRMED against the materialised roster \
             before it returns; a walk alone has no instant at which it knew the \
             whole thread list, so a thread created behind its cursor by one \
             reaped ahead of it is invisible to every count it keeps"
        );
        assert!(
            quiet_body.contains("None => return Ok(freeze),"),
            "the freeze may be returned only on the roster's `None` -- the \
             answer that says it listed nothing this freeze does not hold"
        );
        assert_eq!(
            inner.matches("return Ok(freeze)").count(),
            1,
            "there is more than one way out of this function with a freeze in \
             hand; whichever one is not the confirmed quiet pass returns an \
             unproven set"
        );
        assert_eq!(
            inner
                .matches("what_the_roster_names_that_we_do_not_hold(")
                .count(),
            1,
            "the confirmation must be reached from exactly one place, or one of \
             them is a path that skips it"
        );
        let quiet_return = loop_body
            .find("None => return Ok(freeze),")
            .expect("the quiet pass must return the freeze");
        assert!(
            quiet_return < settle,
            "the settle must come AFTER the quiet-return check; a pass that \
             settles and then returns on the walk it took BEFORE settling proves \
             nothing about the set being closed"
        );
        let settle_guard = block_body(loop_body, "if newly_suspended > 0 && pass < MAX_PASSES {");
        assert!(
            settle_guard.contains("freeze.settle_suspensions()?;"),
            "the settle must sit INSIDE the `newly_suspended > 0 && pass < \
             MAX_PASSES` guard: inside, because a pass that suspended must \
             complete those suspensions before the next walk; conditional, \
             because making it unconditional spends a context read per held \
             thread on passes that added nothing, and on the LAST pass it can \
             refuse with ThreadContext over the answer the freeze had already \
             reached"
        );
        let walk = loop_body
            .find("let stepped = walk.next_thread(cursor);")
            .expect("the pass must walk this process's thread list");
        assert!(
            walk < settle,
            "the settle must follow the walk whose suspensions it completes, and \
             so precede the next one"
        );
        assert_eq!(
            inner.matches("vanished += 1;").count(),
            1,
            "the one accounted-for-but-not-suspended arm must record the vanish: \
             a suspend refused by a thread that is PROVEN terminated"
        );
        assert!(
            inner.contains("!leaves_room_for_minhooks_suspension(previous_count)"),
            "a suspension that lands on the ceiling must refuse; MinHook's own \
             suspension is the one that then fails, and MinHook patches anyway"
        );

        let granted = inner
            .find("walk.granted_access(handle)")
            .expect("every walked thread must be asked what it was granted");
        let classified = inner
            .find("!granted_access_covers_the_freeze(granted)")
            .expect("the granted access must be the disposition");
        let suspended = inner
            .find("SuspendThread(handle)")
            .expect("the freeze loop must suspend the threads it walked");
        assert!(
            granted < classified && classified < suspended,
            "the granted access must be read and classified BEFORE the suspend; \
             a thread short of the freeze's rights is one this module cannot \
             account for, and suspending first has already acted on it"
        );
        let arm_end = inner
            .find("let thread_id = unsafe { GetThreadId(handle) };")
            .expect("the freeze loop must read each walked thread's id");
        let arm_body = &inner[granted..arm_end];
        assert!(
            arm_body.contains("return Err(FreezeRefusal::ThreadRights"),
            "a granted-access shortfall must REFUSE and name the thread; \
             skipping it is what asking the walk for a narrower mask does by \
             itself, and it is the fail-open this arm exists to close"
        );
        assert!(
            !arm_body.contains("continue"),
            "the granted-access arm continues past a thread it could not \
             establish it can stop"
        );

        let suspend_failed = inner
            .find("if previous_count == u32::MAX {")
            .expect("the freeze loop must handle a failed suspend explicitly");
        let suspend_arm_end = inner[suspend_failed..]
            .find("account_for(&mut freeze, thread_id, handle, true)")
            .expect("the failed-suspend arm is followed by the suspended record")
            + suspend_failed;
        let suspend_arm = &inner[suspend_failed..suspend_arm_end];
        assert_eq!(
            suspend_arm.matches("let disposition").count(),
            1,
            "the failed-suspend arm binds `disposition` more than once, so which \
             binding the match reads is no longer decided here"
        );
        assert_eq!(
            suspend_arm.matches("classify_suspend_failure").count(),
            1,
            "the failed-suspend arm calls classify_suspend_failure more than \
             once; a second call can decide the match while the first satisfies \
             the pin below"
        );
        assert!(
            suspend_arm.contains("match disposition {"),
            "the failed-suspend arm must match on the `disposition` it bound; \
             matching on anything else leaves that binding computed and unused"
        );
        assert!(
            suspend_arm.contains(
                "let disposition = classify_suspend_failure(thread_has_terminated(handle));"
            ),
            "a failed suspend must be settled by the thread object being \
             signalled; the error-code allowlist this replaced excused only \
             ERROR_INVALID_HANDLE and ERROR_INVALID_PARAMETER, neither of which \
             the OS returns for a suspend, so an ordinary thread exit refused an \
             entire hook installation"
        );
        for code_test in [
            "os_error ==",
            "== os_error",
            "os_error !=",
            "match os_error",
            "contains(&os_error)",
            "matches!(os_error",
        ] {
            assert!(
                !suspend_arm.contains(code_test),
                "the failed-suspend arm consults the error code (`{code_test}`); \
                 which code Windows chose is not the question, whether the thread \
                 can still execute is, and only the thread object answers that"
            );
        }
        assert!(
            suspend_arm.contains("unsettled += 1;"),
            "the failed-suspend arm must record an unsettled probe so the pass \
             cannot be the last one; without it the freeze either refuses a \
             thread that was merely mid-exit or certifies a set it never closed"
        );
        assert!(
            suspend_arm.contains("unsettled_suspend = Some((thread_id, os_error));"),
            "an unsettled probe must carry the thread and its OS error to the \
             end of the pass budget; a refusal that cannot name the thread it \
             could not settle sends an operator to the wrong place"
        );
        let unsettled_arm = &suspend_arm[suspend_arm
            .find("ThreadDisposition::Refuse =>")
            .expect("the failed-suspend arm must handle the unsettled probe")..];
        assert!(
            !unsettled_arm.contains("return Err"),
            "the unsettled half of the failed-suspend arm refuses immediately; a \
             probe that settled nothing has to be deferred to the next pass, and \
             only the exhausted deferral allowance may turn it into a refusal"
        );
        assert!(
            inner.contains("for pass in 1..=MAX_PASSES {"),
            "the pass loop must be bounded; without a bound a thread that will \
             never settle holds the process forever rather than refusing"
        );
        assert!(
            inner.contains("if let Some((thread, os_error)) = unsettled_suspend {"),
            "the exhausted pass budget must refuse by NAME when the last pass was \
             still unsettled"
        );
        for disposition in ["ThreadDisposition::Gone", "ThreadDisposition::Refuse"] {
            for occurrence in suspend_arm.match_indices(disposition) {
                let tail = &suspend_arm[occurrence.0 + disposition.len()..];
                assert!(
                    tail.trim_start().starts_with("=>"),
                    "the failed-suspend arm CONSTRUCTS {disposition} instead of \
                     only matching on it, so the disposition has a source other \
                     than the thread object"
                );
            }
        }

        let frozen_region = inner
            .split_once("for pass in 1..=MAX_PASSES {")
            .expect("the freeze iterates to a fixed point")
            .1;
        for allocating in [
            concat!("MinHookLock::", "acquire"),
            concat!("Vec::", "new"),
            concat!("Default::", "default"),
            concat!("reser", "ve("),
            concat!("with_cap", "acity"),
            concat!("coll", "ect"),
            concat!("form", "at!"),
            concat!("to_st", "ring"),
            concat!("to_o", "wned"),
            concat!("to_v", "ec"),
            concat!(".clo", "ne("),
            concat!("Box::", "new"),
            concat!("String::", "from"),
            concat!("Vec::", "from"),
            concat!("vec", "!["),
            concat!("print", "ln!"),
            concat!("lo", "ck("),
        ] {
            assert!(
                !frozen_region.contains(allocating),
                "the frozen window contains `{allocating}`, which can allocate or \
                 block while every other thread in the process is suspended; the \
                 allocator's lock may be held by one of them and there is nobody \
                 left to release it"
            );
        }
        let production_all = code_only(&production_code());
        let mut reachable: Vec<String> = Vec::new();
        let mut frontier = vec![frozen_region.to_string()];
        while let Some(body) = frontier.pop() {
            for candidate in production_fn_names(&production_all) {
                if reachable.iter().any(|seen| *seen == candidate) {
                    continue;
                }
                if !body.contains(&format!("{candidate}("))
                    && !body.contains(&format!("{candidate}::<"))
                {
                    continue;
                }
                let Some(callee) = production_fn_body(&production_all, &candidate) else {
                    continue;
                };
                reachable.push(candidate);
                frontier.push(callee);
            }
        }
        assert!(
            reachable.iter().any(|name| name == "account_for"),
            "the call closure did not reach `account_for`, which the frozen \
             window calls twice; a closure that reaches nothing is a guard that \
             asserts nothing"
        );
        for helper in &reachable {
            let body =
                production_fn_body(&production_all, helper).expect("a reachable helper has a body");
            for allocating in [
                concat!("MinHookLock::", "acquire"),
                concat!("Vec::", "new"),
                concat!("Default::", "default"),
                concat!("reser", "ve("),
                concat!("with_cap", "acity"),
                concat!("coll", "ect"),
                concat!("form", "at!"),
                concat!("to_st", "ring"),
                concat!("to_o", "wned"),
                concat!("to_v", "ec"),
                concat!(".clo", "ne("),
                concat!("Box::", "new"),
                concat!("String::", "from"),
                concat!("Vec::", "from"),
                concat!("vec", "!["),
                concat!("print", "ln!"),
                concat!("lo", "ck("),
            ] {
                assert!(
                    !body.contains(allocating),
                    "`{helper}` is reachable from inside the frozen window and \
                     contains `{allocating}`, which can allocate or block while \
                     every other thread in the process is suspended"
                );
            }
        }
        assert!(
            !frozen_region.contains("ThreadWalk::resolve"),
            "the frozen window resolves the walk's entry points; GetProcAddress \
             takes the loader lock and a suspended thread may hold it"
        );
        assert!(
            inner.contains("let walk = ThreadWalk::resolve()?;"),
            "the walk must be resolved once, before the first suspension"
        );

        assert!(
            inner.contains("let bound = frozen_window_buffer_bound(thread_count);"),
            "the frozen-window bound must be derived once, from the thread count \
             seen before the first suspension"
        );
        assert_eq!(
            inner.matches("let bound").count(),
            1,
            "the frozen-window bound is rebound, so the reservation may take a \
             different value while still naming `bound`"
        );
        assert!(
            inner.contains("accounted.try_reserve(bound)"),
            "the accounted thread table must be reserved to the shared bound, \
             not to the headroom alone"
        );
        assert!(
            inner.contains("account_for(&mut freeze, thread_id, handle, false)"),
            "a thread proven terminated must be RECORDED as accounted for; \
             counting it afresh on every pass makes the fixed point unreachable \
             for as long as any handle to it is open"
        );
        assert!(
            inner.contains("account_for(&mut freeze, thread_id, handle, true)"),
            "a thread this freeze suspended must be recorded as suspended, or it \
             is never resumed"
        );
        assert_eq!(
            inner.matches(" accounted = ").count(),
            0,
            "the accounted thread table is reassigned after it was reserved; \
             whatever replaces it did not come from the shared bound"
        );

        let cursor_close = block_body(inner, "if !cursor.is_null() && !cursor_is_kept {");
        assert!(
            cursor_close.contains("CloseHandle(cursor)"),
            "the pass must CLOSE the cursor handles it did not keep, not merely \
             test for them; a handle per thread per pass is leaked otherwise, \
             and an exhausted handle table is exactly what makes MinHook's \
             second open fail into its fail-open branch"
        );
        for suspended in ["false", "true"] {
            let header = format!(
                "if let Err(refusal) = account_for(&mut freeze, thread_id, handle, {suspended}) {{"
            );
            let after = after_block(inner, &header).trim_start();
            assert!(
                after.starts_with("cursor_is_kept = true;"),
                "the `{suspended}` transfer to the accounted table is not \
                 followed by `cursor_is_kept = true;` -- the table owns that \
                 handle now, and the next step of the walk closes it"
            );
            let arm = after
                .split_once("let stepped = walk.next_thread(cursor);")
                .map(|(before, _)| before)
                .unwrap_or(after);
            let arm = arm.split_once("continue; }").map(|(a, _)| a).unwrap_or(arm);
            assert!(
                !arm.contains("CloseHandle(handle)"),
                "the `{suspended}` transfer hands the handle to the accounted \
                 table and then closes it; the table is left holding a closed \
                 handle, the thread object can be deleted, and its id can come \
                 back on a different thread that this freeze would then skip"
            );
        }
        assert_eq!(
            inner.matches("cursor_is_kept =").count(),
            4,
            "the walk's kept-handle flag is written {} times, not four (declared, \
             reset once per step, set once per transfer); an extra write can \
             only be undoing one of them",
            inner.matches("cursor_is_kept =").count()
        );
        assert_eq!(
            inner.matches("cursor_is_kept = true;").count(),
            2,
            "the kept-handle flag is set true somewhere other than the two \
             transfers to the accounted table"
        );
    }

    #[test]
    fn the_roster_names_a_live_thread_the_walk_did_not_account_for() {
        use std::os::windows::io::AsRawHandle;
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let worker_id = Arc::clone(&worker_id);
            std::thread::spawn(move || {
                worker_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        while worker_id.load(Ordering::Acquire) == 0 {
            std::hint::spin_loop();
        }
        let worker_thread_id = worker_id.load(Ordering::Acquire);
        let denied = deny_every_access_to(worker.as_raw_handle() as HANDLE);

        let walk = ThreadWalk::resolve().expect("the walk resolves on this OS");
        let current_thread_id = unsafe { GetCurrentThreadId() };
        let our_process_id = unsafe { GetCurrentProcessId() } as usize;

        let mut buffer = vec![0u8; walk.roster_bytes_needed().expect("the roster sizes") * 2];
        let filled = walk.read_roster(&mut buffer).expect("the roster reads");

        let everything_held = roster_names_an_unheld_thread(
            &buffer[..filled],
            our_process_id,
            current_thread_id,
            |_| true,
        )
        .expect("the roster lists this process and its running thread");
        let one_withheld = roster_names_an_unheld_thread(
            &buffer[..filled],
            our_process_id,
            current_thread_id,
            |id| id != worker_thread_id,
        )
        .expect("the roster lists this process and its running thread");

        stop.store(true, Ordering::Release);
        worker.join().expect("the spinning thread joins");

        assert_eq!(
            denied, 0,
            "the empty DACL was refused ({denied:#010x}), so this test could not \
             manufacture the thread it exists to look for"
        );
        assert_eq!(
            everything_held, None,
            "with every thread held, nothing may be named; a name here is the \
             confirmation refusing sets that are in fact closed"
        );
        assert_eq!(
            one_withheld,
            Some(worker_thread_id),
            "thread {worker_thread_id} is live in this process and is the ONE \
             thread the held set denies, so the roster must name it and name \
             nothing else; a parse that cannot see it cannot see the thread a \
             walk missed either"
        );
    }

    #[test]
    fn a_thread_the_owner_rights_sid_hides_from_the_walk_is_still_named_by_the_roster() {
        use std::os::windows::io::AsRawHandle;
        const ROOM: usize = 4096;

        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let worker_id = Arc::clone(&worker_id);
            std::thread::spawn(move || {
                worker_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        while worker_id.load(Ordering::Acquire) == 0 {
            std::hint::spin_loop();
        }
        let hidden_id = worker_id.load(Ordering::Acquire);

        let walk = ThreadWalk::resolve().expect("the walk resolves on this OS");

        let before = what_maximum_allowed_gets(&walk, hidden_id);
        let set = deny_every_access_including_the_owner_s_to(worker.as_raw_handle() as HANDLE);
        let after = what_maximum_allowed_gets(&walk, hidden_id);
        let current_thread_id = unsafe { GetCurrentThreadId() };
        let our_process_id = unsafe { GetCurrentProcessId() } as usize;
        let mut buffer = vec![0u8; walk.roster_bytes_needed().expect("the roster sizes") * 2];
        let mut walked: Vec<u32> = Vec::with_capacity(ROOM);
        let mut snapshotted: Vec<u32> = Vec::with_capacity(ROOM);

        let walk_answered = fill_with_walked_thread_ids(&walk, &mut walked);
        let snapshot_answered = fill_with_snapshotted_thread_ids(&mut snapshotted);
        let filled = walk.read_roster(&mut buffer).expect("the roster reads");
        let named = roster_names_an_unheld_thread(
            &buffer[..filled],
            our_process_id,
            current_thread_id,
            |id| id != hidden_id,
        );

        let refused_confirmations_before = ROSTER_CONFIRMATIONS_REFUSED.load(Ordering::Acquire);
        let frozen = freeze_other_threads().map(|freeze| freeze.frozen_thread_count());
        let refused_confirmations_after = ROSTER_CONFIRMATIONS_REFUSED.load(Ordering::Acquire);

        stop.store(true, Ordering::Release);
        worker.join().expect("the spinning thread joins");

        assert_eq!(
            set, 0,
            "NtSetSecurityObject refused the OWNER RIGHTS DACL ({set:#010x}), so \
             this test could not manufacture the thread it exists to test"
        );
        assert!(
            before.is_some_and(granted_access_covers_the_freeze),
            "before the ACE this process could not open its own thread at the \
             freeze's access ({before:?}), so nothing below is a change this test \
             caused"
        );
        assert_eq!(
            after, None,
            "after the ACE, OpenThread(MAXIMUM_ALLOWED) still returned a handle \
             granting {after:?}: the OWNER RIGHTS ACE did not take, and the \
             thread below is not the invisible one this test needs"
        );
        walk_answered.expect("the kernel walk answers");
        assert!(
            walked.len() < ROOM,
            "the walk filled its buffer ({} of {ROOM}), so 'the walk did not \
             return the hidden thread' below would be an artefact of capacity",
            walked.len()
        );
        assert!(
            walked.len() > 1,
            "the walk returned {} thread(s), so this process has nothing for the \
             comparison below to be about",
            walked.len()
        );
        assert!(snapshot_answered, "the system snapshot answers");
        assert!(
            snapshotted.len() < ROOM,
            "the snapshot filled its buffer ({} of {ROOM})",
            snapshotted.len()
        );
        assert!(
            snapshotted.contains(&hidden_id),
            "thread {hidden_id} is not in this process's system snapshot, so it \
             is not live and this test is about nothing"
        );
        assert!(
            !walked.contains(&hidden_id),
            "the MAXIMUM_ALLOWED walk named thread {hidden_id}, so the OWNER \
             RIGHTS ACE did not deny this process every access to it"
        );
        assert_eq!(
            named,
            Ok(Some(hidden_id)),
            "thread {hidden_id} is live in this process and INVISIBLE to the \
             walk, so the roster is the only thing that can name it; it did not"
        );
        match frozen {
            Err(FreezeRefusal::RosterUnheld { thread, passes }) => {
                assert_eq!(
                    thread, hidden_id,
                    "the freeze refused for a thread other than the hidden one"
                );
                assert_eq!(
                    passes, MAX_PASSES,
                    "a thread the walk can never reach must cost the whole pass \
                     budget before the freeze gives up on it"
                );
                assert!(
                    refused_confirmations_after > refused_confirmations_before,
                    "the freeze refused as RosterUnheld without any roster \
                     confirmation having refused, which means the refusal was \
                     reached by some path other than the roster naming the thread"
                );
            }
            Err(FreezeRefusal::ThreadRights { thread, granted }) => panic!(
                "the freeze refused as ThreadRights (thread {thread}, granted \
                 {granted:#010x}), which is the EMPTY-DACL outcome: the walk saw \
                 the thread. The OWNER RIGHTS construction did not reach the \
                 hole this test exists for"
            ),
            Err(other) => panic!(
                "a live thread of this process that the walk cannot see at all \
                 must refuse the freeze as RosterUnheld, not as {other:?}"
            ),
            Ok(held) => panic!(
                "the freeze SUCCEEDED holding {held} thread(s) while thread \
                 {hidden_id} was live, unsuspended, and invisible to the walk; \
                 that thread was still running when the patch would have landed"
            ),
        }
    }

    #[test]
    fn a_roster_that_makes_no_sense_refuses_instead_of_confirming() {
        const OUR_PID: usize = 0x1234;
        const OUR_TID: u32 = 0x5678;

        fn entry(next: u32, threads: u32, pid: usize) -> Vec<u8> {
            let mut bytes = vec![0u8; ROSTER_PROCESS_ENTRY_BYTES];
            bytes[ROSTER_NEXT_ENTRY_OFFSET..ROSTER_NEXT_ENTRY_OFFSET + 4]
                .copy_from_slice(&next.to_ne_bytes());
            bytes[ROSTER_THREAD_COUNT_OFFSET..ROSTER_THREAD_COUNT_OFFSET + 4]
                .copy_from_slice(&threads.to_ne_bytes());
            bytes[ROSTER_PROCESS_ID_OFFSET..ROSTER_PROCESS_ID_OFFSET + 8]
                .copy_from_slice(&pid.to_ne_bytes());
            bytes
        }
        fn thread_of(pid: usize, id: u32) -> Vec<u8> {
            let mut bytes = vec![0u8; ROSTER_THREAD_ENTRY_BYTES];
            bytes[ROSTER_THREAD_PROCESS_OFFSET..ROSTER_THREAD_PROCESS_OFFSET + 8]
                .copy_from_slice(&pid.to_ne_bytes());
            bytes[ROSTER_THREAD_ID_OFFSET..ROSTER_THREAD_ID_OFFSET + 8]
                .copy_from_slice(&(id as usize).to_ne_bytes());
            bytes
        }
        let thread = |id: u32| thread_of(OUR_PID, id);
        let held_everything = |_: u32| true;

        let mut good = entry(0, 1, OUR_PID);
        good.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&good, OUR_PID, OUR_TID, held_everything),
            Ok(None),
            "a well-formed roster listing only the running thread must confirm"
        );

        let short = entry(0, 1, OUR_PID);
        assert_eq!(
            roster_names_an_unheld_thread(&short, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "an entry whose thread array runs past the buffer must refuse"
        );

        let mut stranger = entry(0, 1, OUR_PID);
        stranger.extend(thread(OUR_TID.wrapping_add(1)));
        assert_eq!(
            roster_names_an_unheld_thread(&stranger, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a roster that does not list this process's own running thread must \
             refuse; confirming there would confirm from a misread buffer"
        );

        let mut overrun = entry(ROSTER_PROCESS_ENTRY_BYTES as u32, 1, OUR_PID);
        overrun.extend(entry(0, 1, OUR_PID + 1));
        overrun.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&overrun, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a thread array that runs past its own entry must refuse; the bytes \
             it would read belong to another process's entry"
        );

        let mut shifted = entry(1, 0, OUR_PID + 1);
        shifted.extend(vec![0u8; ROSTER_PROCESS_ENTRY_BYTES]);
        assert_eq!(
            roster_names_an_unheld_thread(&shifted, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a next-entry offset shorter than an entry, or unaligned, must \
             refuse rather than be followed"
        );

        let absent = entry(0, 0, OUR_PID + 1);
        assert_eq!(
            roster_names_an_unheld_thread(&absent, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a roster without this process in it must refuse"
        );

        let mut foreign_overrun = entry(ROSTER_PROCESS_ENTRY_BYTES as u32, 1, OUR_PID + 1);
        foreign_overrun.extend(entry(0, 1, OUR_PID));
        foreign_overrun.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&foreign_overrun, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a FOREIGN entry whose thread array runs past its own end must \
             refuse; the bytes after it are read as the next entry"
        );

        let listed_twice = (ROSTER_PROCESS_ENTRY_BYTES + ROSTER_THREAD_ENTRY_BYTES) as u32;
        let mut duplicated = entry(listed_twice, 1, OUR_PID);
        duplicated.extend(thread(OUR_TID));
        duplicated.extend(entry(0, 1, OUR_PID));
        duplicated.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&duplicated, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "two entries for this process id must refuse rather than answering \
             from whichever came first"
        );

        let mut foreign_record = entry(0, 2, OUR_PID);
        foreign_record.extend(thread(OUR_TID));
        foreign_record.extend(thread_of(OUR_PID + 1, OUR_TID.wrapping_add(9)));
        assert_eq!(
            roster_names_an_unheld_thread(&foreign_record, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a thread record naming another process inside this process's entry              must refuse; its id means nothing here"
        );

        let mut foreign_entry_lies = entry(
            (ROSTER_PROCESS_ENTRY_BYTES + ROSTER_THREAD_ENTRY_BYTES) as u32,
            1,
            OUR_PID + 1,
        );
        foreign_entry_lies.extend(thread_of(OUR_PID + 2, OUR_TID.wrapping_add(4)));
        foreign_entry_lies.extend(entry(0, 1, OUR_PID));
        foreign_entry_lies.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&foreign_entry_lies, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a FOREIGN entry whose records name a third process must refuse; a \
             roster that inconsistent cannot be confirmed from, whatever the \
             target entry after it says"
        );

        let mut repeated = entry(0, 2, OUR_PID);
        repeated.extend(thread(OUR_TID));
        repeated.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&repeated, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a roster listing the same thread id twice for one process must              refuse; identities are unique and these bytes are not describing              this process"
        );
        let mut repeated_held = entry(0, 3, OUR_PID);
        repeated_held.extend(thread(OUR_TID));
        repeated_held.extend(thread(OUR_TID.wrapping_add(3)));
        repeated_held.extend(thread(OUR_TID.wrapping_add(3)));
        assert_eq!(
            roster_names_an_unheld_thread(&repeated_held, OUR_PID, OUR_TID, |id| id
                == OUR_TID.wrapping_add(3)),
            Err(FreezeRefusal::RosterUnreadable),
            "a repeated HELD id must refuse too; one of the two records could be              carrying an unheld thread's real identity"
        );

        let idle_span = (ROSTER_PROCESS_ENTRY_BYTES + 2 * ROSTER_THREAD_ENTRY_BYTES) as u32;
        let mut idle = entry(idle_span, 2, 0);
        idle.extend(thread_of(0, 0));
        idle.extend(thread_of(0, 0));
        idle.extend(entry(0, 1, OUR_PID));
        idle.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&idle, OUR_PID, OUR_TID, held_everything),
            Ok(None),
            "pid 0's repeated zero ids are the Idle process's real shape and must \
             be exempt; refusing them refuses every freeze on this machine"
        );

        let mut foreign_duplicate = entry(idle_span, 2, OUR_PID + 1);
        foreign_duplicate.extend(thread_of(OUR_PID + 1, 77));
        foreign_duplicate.extend(thread_of(OUR_PID + 1, 77));
        foreign_duplicate.extend(entry(0, 1, OUR_PID));
        foreign_duplicate.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&foreign_duplicate, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "an ordinary foreign process repeating a thread id must refuse; the \
             exemption is pid 0's zero ids and nothing else"
        );
        let mut idle_nonzero = entry(idle_span, 2, 0);
        idle_nonzero.extend(thread_of(0, 5));
        idle_nonzero.extend(thread_of(0, 5));
        idle_nonzero.extend(entry(0, 1, OUR_PID));
        idle_nonzero.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&idle_nonzero, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "the exemption covers pid 0's ZERO ids; a repeated nonzero id there \
             is as impossible as anywhere else"
        );

        let mut two = entry(0, 2, OUR_PID);
        two.extend(thread(OUR_TID));
        two.extend(thread(OUR_TID.wrapping_add(9)));
        assert_eq!(
            roster_names_an_unheld_thread(&two, OUR_PID, OUR_TID, |id| id
                != OUR_TID.wrapping_add(9)),
            Ok(Some(OUR_TID.wrapping_add(9))),
            "the one listed thread the held set denies must be named"
        );
    }

    #[test]
    fn the_source_scan_shows_the_compiler_braces_and_no_others() {
        let braces = |source: &str| -> usize {
            code_only(source)
                .chars()
                .filter(|character| *character == '{' || *character == '}')
                .count()
        };
        assert_eq!(
            braces("fn f() { let _ = \"if x { y } else { z }\"; }"),
            2,
            "a string literal spelling a whole guard must contribute no braces; \
             with it intact, a deleted guard can be re-spelled where a pin looks"
        );
        assert_eq!(
            braces("fn f() { let _ = '{'; }"),
            2,
            "a char literal holding a brace must contribute none; with it \
             intact, a call can be moved out of a guard and the guard's body \
             made to look as if it still held it"
        );
        assert_eq!(
            braces("fn f() { let _ = br#\"}\"#; }"),
            2,
            "a raw BYTE string must contribute no braces"
        );
        assert_eq!(
            braces("fn f() { let _ = cr#\"{\"#; }"),
            2,
            "a raw C string must contribute no braces"
        );
        assert_eq!(
            braces("fn f() { let _ = \"\\\"}\"; }"),
            2,
            "an escaped quote must not end the literal early and let the rest of \
             it back into the scan"
        );
        assert_eq!(
            braces("fn f() { if a { b() } }"),
            4,
            "the scan must keep every brace the compiler sees"
        );
    }

    #[test]
    fn a_suspension_must_leave_minhook_a_slot_of_its_own() {
        assert!(leaves_room_for_minhooks_suspension(0));
        assert!(
            leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT - 2),
            "125 -> ours makes 126, MinHook's makes 127, which is the maximum"
        );
        assert!(
            !leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT - 1),
            "126 -> ours makes 127 and MinHook's would be 128, which fails"
        );
        assert!(!leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT));
        assert!(!leaves_room_for_minhooks_suspension(u32::MAX - 1));
    }

    #[test]
    fn a_pass_that_saw_a_thread_vanish_is_never_the_last_one() {
        assert!(
            pass_is_quiet(0, 0, 0),
            "nothing outside the set is the fixed point"
        );
        assert!(
            !pass_is_quiet(0, 1, 0),
            "a thread newly proved terminated may have started a successor this              pass has not walked"
        );
        assert!(
            !pass_is_quiet(1, 0, 0),
            "a newly suspended thread is a changed set"
        );
        assert!(
            !pass_is_quiet(0, 0, 1),
            "an unsettled suspend failure is not a closed set; the next pass has \
             to re-derive that id, which is what replaced the bounded wait"
        );
        assert!(!pass_is_quiet(3, 2, 1));
    }

    #[test]
    fn a_thread_that_cannot_be_suspended_refuses_by_name_when_the_passes_run_out() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let live_thread_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let live_thread_id = Arc::clone(&live_thread_id);
            std::thread::spawn(move || {
                live_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        let mut worker_id = 0u32;
        while worker_id == 0 {
            worker_id = live_thread_id.load(Ordering::Acquire);
            std::thread::yield_now();
        }
        let handle = {
            use std::os::windows::io::AsRawHandle;
            worker.as_raw_handle() as HANDLE
        };
        let identified = unsafe { GetThreadId(handle) } == worker_id;

        let mut raised = 0u32;
        let mut previous_counts_were_sequential = true;
        while raised < MAXIMUM_SUSPEND_COUNT {
            let previous = unsafe { SuspendThread(handle) };
            previous_counts_were_sequential &= previous == raised;
            raised += 1;
        }

        let deferred_before = suspend_failures_deferred_to_a_later_pass();
        let passes_before = freeze_passes_spent();
        let refusals_before = freeze_refusals();
        let started = std::time::Instant::now();
        let outcome = freeze_other_threads();
        let elapsed = started.elapsed();
        let deferred = suspend_failures_deferred_to_a_later_pass() - deferred_before;
        let passes = freeze_passes_spent() - passes_before;

        while raised > 0 {
            unsafe { ResumeThread(handle) };
            raised -= 1;
        }
        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");

        assert!(
            identified,
            "the handle suspended must be the worker thread's own"
        );
        assert!(
            previous_counts_were_sequential,
            "SuspendThread returns the PREVIOUS count and the setup did not see              0, 1, 2, ...; something else is suspending this thread, so it was              never raised to exactly MAXIMUM_SUSPEND_COUNT and the branch under              test may not have been the one taken"
        );
        match outcome {
            Ok(freeze) => {
                let held = freeze.frozen_thread_count();
                drop(freeze);
                panic!(
                    "the freeze declared the thread set closed ({held} thread(s) \
                     held) while one enumerated thread was neither suspended nor \
                     proven terminated; an unsettled probe must never end the \
                     pass loop"
                );
            }
            Err(FreezeRefusal::ThreadSuspend { thread, .. }) => assert_eq!(
                thread, worker_id,
                "the refusal must name the thread whose suspend could not be \
                 settled"
            ),
            Err(other) => panic!(
                "an unsuspendable thread must refuse as ThreadSuspend and name \
                 itself; got {other}"
            ),
        }
        assert_eq!(
            freeze_refusals() - refusals_before,
            1,
            "the refusal must be counted exactly once"
        );
        assert!(
            deferred > 1,
            "the unsettled probe was deferred {deferred} time(s), so the pass \
             loop is not re-deriving it at all; refusing on the first unsettled \
             probe is the behaviour the deleted 50 ms wait existed to paper over"
        );
        assert_eq!(
            passes,
            u64::from(MAX_PASSES),
            "the refusal spent {passes} pass(es) rather than the full \
             {MAX_PASSES}-pass budget; every pass is a walk of this process's \
             thread list taken with the rest of the process suspended, and a \
             thread that will never settle must be given all of them before it \
             is refused"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(30),
            "the refusal took {elapsed:?}, which is not a bounded pass loop at all"
        );
    }

    #[test]
    fn a_held_freeze_stops_other_threads_and_dropping_it_starts_them_again() {
        let _serialised = os_suspend_state_guard();
        let counter = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let counter = Arc::clone(&counter);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    counter.fetch_add(1, Ordering::AcqRel);
                    std::hint::spin_loop();
                }
            })
        };
        while counter.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }

        let freeze = freeze_other_threads().expect("a test process must be freezable");
        assert!(
            freeze.frozen_thread_count() >= 1,
            "the spinning worker must have been suspended"
        );
        let first = counter.load(Ordering::Acquire);
        std::thread::sleep(std::time::Duration::from_millis(40));
        let second = counter.load(Ordering::Acquire);
        assert_eq!(
            first, second,
            "the counter advanced while the process was supposed to be frozen"
        );
        assert!(
            freeze.passes() >= 2,
            "the fixed point needs a verification pass that found nothing new"
        );
        drop(freeze);

        let resumed_from = counter.load(Ordering::Acquire);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while counter.load(Ordering::Acquire) == resumed_from {
            assert!(
                std::time::Instant::now() < deadline,
                "the worker was never resumed after the freeze was dropped"
            );
            std::thread::yield_now();
        }
        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");
    }

    #[test]
    fn every_freeze_attempt_is_counted() {
        let _serialised = os_suspend_state_guard();
        let before = freeze_attempts();
        let freeze = freeze_other_threads().expect("a test process must be freezable");
        drop(freeze);
        assert!(
            freeze_attempts() > before,
            "the attempt counter must move on every call"
        );
    }

    #[test]
    #[ignore = "evidence tier: waits for a rare OS interleaving; 1.7-176.8 s measured over 13 runs"]
    fn a_deferred_suspend_failure_is_observed_resolving_against_the_os() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let churn = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut producers = Vec::new();
                for _ in 0..4 {
                    let stop = Arc::clone(&stop);
                    producers.push(std::thread::spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            let workers: Vec<_> = (0..16)
                                .map(|_| std::thread::spawn(std::thread::yield_now))
                                .collect();
                            for worker in workers {
                                let _ = worker.join();
                            }
                        }
                    }));
                }
                for producer in producers {
                    let _ = producer.join();
                }
            })
        };

        const MAX_ROUNDS: u32 = 20_000;
        let excused_before = suspend_failures_excused_as_terminated();
        let deferred_before = suspend_failures_deferred_to_a_later_pass();
        let mut rounds = 0u32;
        let outcome = loop {
            rounds += 1;
            match freeze_other_threads() {
                Ok(freeze) => drop(freeze),
                Err(FreezeRefusal::NotQuiescent { .. }) => {}
                Err(refusal) => break Err((rounds, refusal)),
            }
            let excused = suspend_failures_excused_as_terminated() - excused_before;
            let deferred = suspend_failures_deferred_to_a_later_pass() - deferred_before;
            if excused > 0 && deferred > 0 {
                break Ok((rounds, deferred));
            }
            if rounds >= MAX_ROUNDS {
                break Ok((rounds, 0));
            }
        };
        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");
        match outcome {
            Err((round, refusal)) => panic!(
                "round {round}: a thread exiting mid-suspend must be deferred to \
                 the next pass, not refuse the freeze: {refusal}"
            ),
            Ok((rounds, 0)) => panic!(
                "{rounds} freezes under churn never hit both shapes of a thread \
                 exiting mid-suspend, so this test proved nothing; make the churn \
                 harder rather than letting it pass vacuously"
            ),
            Ok((_, deferred)) => assert!(deferred > 0),
        }
    }

    #[test]
    fn a_thread_exiting_mid_enumeration_is_excused_by_its_signalled_thread_object() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let churn = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut producers = Vec::new();
                for _ in 0..4 {
                    let stop = Arc::clone(&stop);
                    producers.push(std::thread::spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            let workers: Vec<_> = (0..16)
                                .map(|_| std::thread::spawn(std::thread::yield_now))
                                .collect();
                            for worker in workers {
                                let _ = worker.join();
                            }
                        }
                    }));
                }
                for producer in producers {
                    let _ = producer.join();
                }
            })
        };

        let budget = std::time::Duration::from_secs(30);
        let deadline = std::time::Instant::now() + budget;
        let excused_before = suspend_failures_excused_as_terminated();
        let mut rounds = 0u32;
        let outcome = loop {
            rounds += 1;
            match freeze_other_threads() {
                Ok(freeze) => drop(freeze),
                Err(FreezeRefusal::NotQuiescent { .. }) => {}
                Err(refusal) => break Err((rounds, refusal)),
            }
            let excused = suspend_failures_excused_as_terminated() - excused_before;
            if excused > 0 {
                break Ok((rounds, excused));
            }
            if std::time::Instant::now() >= deadline {
                break Ok((rounds, 0));
            }
        };
        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");
        match outcome {
            Err((round, refusal)) => panic!(
                "round {round}: a thread exiting mid-enumeration must not refuse \
                 the freeze: {refusal}"
            ),
            Ok((rounds, 0)) => panic!(
                "{rounds} freezes under churn in {budget:?} never caught a thread \
                 exiting mid-suspend, so this test proved nothing; measured \
                 arrival on this machine was 0.6-2.9 s, so either the churn has \
                 stopped producing exits or the excusal path is no longer \
                 reached -- make the churn harder rather than letting it pass \
                 vacuously"
            ),
            Ok((_, excused)) => assert!(excused > 0),
        }
    }

    #[test]
    fn a_frozen_thread_is_found_where_it_is_stopped_and_nowhere_else() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };

        let freeze = freeze_other_threads().expect("a test process must be freezable");
        let frozen = freeze.frozen_thread_count();
        let everywhere = freeze.any_instruction_pointer_in(&[(0, usize::MAX)]);
        let nowhere = freeze.any_instruction_pointer_in(&[(0x1000, 0x1000 + 16)]);
        let empty = freeze.any_instruction_pointer_in(&[]);
        drop(freeze);

        assert!(frozen >= 1, "the spinning worker must have been suspended");
        assert_eq!(
            everywhere,
            Ok(true),
            "every frozen thread is stopped somewhere, so a range covering the \
             whole address space must find one"
        );
        assert_eq!(
            nowhere,
            Ok(false),
            "no thread can be stopped in the reserved null page"
        );
        assert_eq!(empty, Ok(false), "no ranges is nothing to be inside");

        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");
    }

    #[test]
    fn threads_churning_during_the_freeze_are_all_accounted_for() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let churn = {
            let stop = Arc::clone(&stop);
            let progress = Arc::clone(&progress);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let handles: Vec<_> = (0..8)
                        .map(|_| {
                            let progress = Arc::clone(&progress);
                            std::thread::spawn(move || {
                                for _ in 0..2_048 {
                                    progress.fetch_add(1, Ordering::AcqRel);
                                    std::hint::spin_loop();
                                }
                            })
                        })
                        .collect();
                    for handle in handles {
                        let _ = handle.join();
                    }
                }
            })
        };

        let started = progress.load(Ordering::Acquire);
        while progress.load(Ordering::Acquire) == started {
            std::hint::spin_loop();
        }

        let mut failure: Option<String> = None;
        let mut successful_held_freezes = 0u32;
        let mut tolerated_access_denied_refusals = 0u32;
        for round in 0..25 {
            match freeze_other_threads() {
                Err(FreezeRefusal::ThreadSuspend { os_error, .. })
                    if os_error == windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED =>
                {
                    tolerated_access_denied_refusals += 1;
                    continue;
                }
                Err(refusal) => {
                    failure = Some(format!(
                        "round {round}: threads exiting mid-enumeration must not                          refuse the freeze: {refusal}"
                    ));
                    break;
                }
                Ok(freeze) => {
                    if let Err(refusal) = freeze.settle_suspensions() {
                        failure = Some(format!(
                            "round {round}: a held thread's context could not be                              read, so the suspensions could not be completed:                              {refusal}"
                        ));
                        drop(freeze);
                        break;
                    }
                    let held_at_start = progress.load(Ordering::Acquire);
                    let held_until =
                        std::time::Instant::now() + std::time::Duration::from_millis(2);
                    let mut held_at_end = held_at_start;
                    while std::time::Instant::now() < held_until {
                        held_at_end = progress.load(Ordering::Acquire);
                        if held_at_end != held_at_start {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                    let passes = freeze.passes();
                    drop(freeze);
                    if held_at_end != held_at_start {
                        failure = Some(format!(
                            "round {round}: the progress counter advanced from                              {held_at_start} to {held_at_end} while the freeze was                              HELD, so at least one churn thread was still running                              in a process this module had declared stopped"
                        ));
                        break;
                    }
                    if passes > MAX_PASSES {
                        failure = Some(format!(
                            "round {round}: the freeze reported {passes} passes,                              over the {MAX_PASSES} bound"
                        ));
                        break;
                    }
                    successful_held_freezes += 1;
                }
            }
        }
        let after = progress.load(Ordering::Acquire);
        let resume_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut resumed = false;
        while std::time::Instant::now() < resume_deadline {
            if progress.load(Ordering::Acquire) != after {
                resumed = true;
                break;
            }
            std::thread::yield_now();
        }

        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");

        if let Some(failure) = failure {
            panic!("{failure}");
        }
        assert!(
            successful_held_freezes > 0,
            "all 25 attempts ended in the documented access-denied refusal              ({tolerated_access_denied_refusals} tolerated); no successful freeze              was held, so this run did not exercise the property"
        );
        assert!(
            resumed,
            "the churn never advanced again after the freezes were dropped, so              the held-window observation above proved nothing"
        );
    }

    #[test]
    fn every_resume_in_this_module_is_the_counted_one() {
        let production = code_only(&production_code());
        assert_eq!(
            production.matches("ResumeThread(").count(),
            1,
            "exactly one call, and it is the helper's own"
        );
        let helper = production
            .split_once("unsafe fn resume_or_count(handle: HANDLE)")
            .expect("the counted resume exists")
            .1;
        let helper = &helper[..helper.find("} /").unwrap_or(helper.len().min(600))];
        assert!(
            helper.contains("ResumeThread(handle)"),
            "the one call must be the helper's own"
        );
        assert_eq!(
            production.matches("resume_or_count(").count(),
            4,
            "the helper is defined once and called from the three places that \
             resume: `Drop`, the suspend-ceiling rollback and the capacity \
             rollback"
        );
    }

    #[test]
    fn an_ordinary_freeze_resumes_everything_it_suspended() {
        let _suspend = os_suspend_state_guard();
        let before = resume_failures();
        for _ in 0..3 {
            let frozen = freeze_process().expect("freeze");
            assert!(frozen.frozen_thread_count() >= 1, "the harness has threads");
            drop(frozen);
        }
        assert_eq!(
            resume_failures(),
            before,
            "a freeze that resumed everything it suspended must not count a              resume failure"
        );
    }

    /// offline test can observe the deadlock it prevents.
    /// * MinHook's lock is acquired BEFORE `FREEZE_LOCK`. One global order, so
    ///   other -- which is a deadlock with the process stopped, not a slow path.
    /// * It is acquired ONCE, in the freeze, and nowhere else in this module. A
    #[test]
    fn minhooks_lock_is_taken_once_and_before_the_freeze_lock() {
        let production = code_only(&production_code());
        assert_eq!(
            production.matches("MinHookLock::acquire()").count(),
            1,
            "exactly one acquisition, and it belongs to the freeze"
        );
        let acquire = production
            .find("MinHookLock::acquire()")
            .expect("the freeze acquires MinHook's lock");
        let freeze_lock = production
            .find("FREEZE_LOCK.lock()")
            .expect("the freeze takes its own serialisation lock");
        assert!(
            acquire < freeze_lock,
            "MinHook's lock must be taken BEFORE the freeze lock; one global              order is what keeps two freezers from deadlocking"
        );
        let first_suspend = production
            .find("SuspendThread(")
            .expect("the freeze suspends threads");
        assert!(
            acquire < first_suspend,
            "MinHook's lock must be taken before the first suspension: a lock              taken after it may already be held by a thread that can no longer run"
        );
    }

    /// nobody left to run and release it. The lock is now taken before the
    /// first `SuspendThread` and released after the last resume.
    #[test]
    fn the_freeze_holds_minhooks_lock_for_the_whole_window() {
        let _suspend = os_suspend_state_guard();
        let probe = minhook::MinHookLock::acquire().expect("the lock is free");
        drop(probe);
        let frozen = freeze_process().expect("freeze");
        assert_eq!(
            minhook::MinHookLock::acquire().unwrap_err(),
            minhook::MH_STATUS::MH_ERROR_LOCK_HELD,
            "the freeze must hold MinHook's lock while it holds the process"
        );
        drop(frozen);
        // And released again with the resume, so the next caller is not wedged.
        let after = minhook::MinHookLock::acquire().expect("the lock is free again");
        drop(after);
    }

    #[test]
    fn a_thread_inside_the_bytes_a_patch_would_rewrite_refuses_it() {
        let _suspend = os_suspend_state_guard();
        let frozen = freeze_process().expect("freeze");
        let refusal = frozen
            .refuse_if_a_thread_is_in_ranges(&[(0, usize::MAX)])
            .expect_err("every held thread's instruction pointer is in this range");
        assert!(
            matches!(refusal, FreezeRefusal::TargetExecuting { ranges: 1 }),
            "{refusal:?}"
        );
        let said = refusal.to_string();
        assert!(said.contains("no target byte was touched"), "{said}");
        assert!(frozen.refuse_if_a_thread_is_in_ranges(&[]).is_ok());
        drop(frozen);
    }

    #[test]
    fn a_range_no_thread_is_inside_does_not_refuse() {
        let _suspend = os_suspend_state_guard();
        let frozen = freeze_process().expect("freeze");
        assert!(frozen.refuse_if_a_thread_is_in_ranges(&[(1, 2)]).is_ok());
        drop(frozen);
    }
}
