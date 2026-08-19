//!   nobody left to release it.
//! * [`MinHookLock`] makes the lock explicit. It is acquired BEFORE the caller
//!   `MH_LockAcquire`.
//! type-checked ([`MinHookLock`] is unforgeable and the C re-checks ownership);

#![cfg(windows)]

use std::ffi::c_void;
use std::fmt;

mod ffi;

/// is roughly "a couple of seconds of a lock nobody released". The work it waits
pub const DEFAULT_SPIN_BUDGET: usize = 2048;

/// [`MH_STATUS::MH_ERROR_LOCK_HELD`], never a deadlock.
#[derive(Debug)]
pub struct MinHookLock {
    _private: (),
}

impl MinHookLock {
    /// Acquire the lock and initialise the library under it.
    pub fn acquire() -> Result<Self, MH_STATUS> {
        Self::acquire_within(DEFAULT_SPIN_BUDGET)
    }

    /// Acquire with an explicit spin budget, refusing
    pub fn acquire_within(spin_budget: usize) -> Result<Self, MH_STATUS> {
        let acquired = unsafe { ffi::MH_LockAcquire(spin_budget) };
        if acquired != MH_STATUS::MH_OK {
            return Err(acquired);
        }
        match unsafe { ffi::MH_Initialize() } {
            MH_STATUS::MH_OK | MH_STATUS::MH_ERROR_ALREADY_INITIALIZED => Ok(Self { _private: () }),
            failed => {
                // Release before reporting: a lock left held by a failed
                let _ = unsafe { ffi::MH_LockRelease() };
                Err(failed)
            }
        }
    }
}

impl Drop for MinHookLock {
    fn drop(&mut self) {
        // A refusal here means the lock was released out from under this guard,
        let _ = unsafe { ffi::MH_LockRelease() };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchFootprint {
    pub patch: (usize, usize),
    pub trampoline: (usize, usize),
}

pub struct MinHook {}

impl MinHook {
    pub unsafe fn create_hook(
        target: *mut c_void,
        detour: *mut c_void,
    ) -> Result<*mut c_void, MH_STATUS> {
        let lock = MinHookLock::acquire()?;
        unsafe { Self::create_hook_locked(&lock, target, detour) }
    }

    pub unsafe fn create_hook_locked(
        _lock: &MinHookLock,
        target: *mut c_void,
        detour: *mut c_void,
    ) -> Result<*mut c_void, MH_STATUS> {
        let mut original: *mut c_void = std::ptr::null_mut();
        match unsafe { ffi::MH_CreateHook(target, detour, &mut original) } {
            MH_STATUS::MH_OK => Ok(original),
            status => Err(status),
        }
    }

    pub fn patch_footprint(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<PatchFootprint, MH_STATUS> {
        let mut patch_start: *mut c_void = std::ptr::null_mut();
        let mut patch_size: usize = 0;
        let mut trampoline: *mut c_void = std::ptr::null_mut();
        let mut trampoline_size: usize = 0;
        let status = unsafe {
            ffi::MH_PatchFootprint(
                target,
                &mut patch_start,
                &mut patch_size,
                &mut trampoline,
                &mut trampoline_size,
            )
        };
        if status != MH_STATUS::MH_OK {
            return Err(status);
        }
        let patch = patch_start as usize;
        let slot = trampoline as usize;
        Ok(PatchFootprint {
            patch: (patch, patch.saturating_add(patch_size)),
            trampoline: (slot, slot.saturating_add(trampoline_size)),
        })
    }

    pub fn hook_is_enabled(_lock: &MinHookLock, target: *mut c_void) -> Result<bool, MH_STATUS> {
        let mut enabled: i32 = 0;
        match unsafe { ffi::MH_HookIsEnabled(target, &mut enabled) } {
            MH_STATUS::MH_OK => Ok(enabled != 0),
            status => Err(status),
        }
    }

    pub fn hook_is_incoherent(_lock: &MinHookLock, target: *mut c_void) -> Result<bool, MH_STATUS> {
        let mut incoherent: i32 = 0;
        match unsafe { ffi::MH_HookIsIncoherent(target, &mut incoherent) } {
            MH_STATUS::MH_OK => Ok(incoherent != 0),
            status => Err(status),
        }
    }

    pub fn hook_count(_lock: &MinHookLock) -> Result<u32, MH_STATUS> {
        let mut count: u32 = 0;
        match unsafe { ffi::MH_HookCount(&mut count) } {
            MH_STATUS::MH_OK => Ok(count),
            status => Err(status),
        }
    }

    pub fn pending_footprint(
        _lock: &MinHookLock,
        ordinal: u32,
    ) -> Result<Option<(*mut c_void, PatchFootprint)>, MH_STATUS> {
        let mut target: *mut c_void = std::ptr::null_mut();
        let mut patch_start: *mut c_void = std::ptr::null_mut();
        let mut patch_size: usize = 0;
        let mut trampoline: *mut c_void = std::ptr::null_mut();
        let mut trampoline_size: usize = 0;
        let status = unsafe {
            ffi::MH_PendingPatchFootprint(
                ordinal,
                &mut target,
                &mut patch_start,
                &mut patch_size,
                &mut trampoline,
                &mut trampoline_size,
            )
        };
        match status {
            MH_STATUS::MH_OK => {
                let patch = patch_start as usize;
                let slot = trampoline as usize;
                Ok(Some((
                    target,
                    PatchFootprint {
                        patch: (patch, patch.saturating_add(patch_size)),
                        trampoline: (slot, slot.saturating_add(trampoline_size)),
                    },
                )))
            }
            MH_STATUS::MH_ERROR_NOT_CREATED => Ok(None),
            failed => Err(failed),
        }
    }

    pub unsafe fn enable_hook(_lock: &MinHookLock, target: *mut c_void) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_EnableHook(target) })
    }

    pub unsafe fn disable_hook(_lock: &MinHookLock, target: *mut c_void) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_DisableHook(target) })
    }

    pub unsafe fn remove_hook(_lock: &MinHookLock, target: *mut c_void) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_RemoveHook(target) })
    }

    pub unsafe fn queue_enable_hook(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_QueueEnableHook(target) })
    }

    pub unsafe fn queue_disable_hook(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_QueueDisableHook(target) })
    }

    pub unsafe fn apply_queued(_lock: &MinHookLock) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_ApplyQueued() })
    }

    pub unsafe fn uninitialize(_lock: &MinHookLock) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_Uninitialize() })
    }
}

#[cfg(feature = "test_support")]
pub struct EnableIncoherentFault;

#[cfg(feature = "test_support")]
#[must_use = "the fault disarms when this guard drops, so it must be bound"]
pub struct ArmedFault;

#[cfg(feature = "test_support")]
impl Drop for ArmedFault {
    fn drop(&mut self) {
        EnableIncoherentFault::disarm();
    }
}

#[cfg(feature = "test_support")]
impl EnableIncoherentFault {
    pub fn arm(target: *mut c_void) -> ArmedFault {
        unsafe { ffi::MH_H4_TestArmEnableIncoherent(target) };
        ArmedFault
    }

    pub fn disarm() {
        unsafe { ffi::MH_H4_TestArmEnableIncoherent(std::ptr::null_mut()) };
    }

    pub fn witness() -> Option<&'static str> {
        let witness = unsafe { ffi::MH_H4_TestFaultWitness() };
        if witness.is_null() {
            return None;
        }
        // SAFETY: the C stores a pointer to a static array of ASCII bytes.
        Some(
            unsafe { std::ffi::CStr::from_ptr(witness) }
                .to_str()
                .expect("the marker is ASCII"),
        )
    }

    pub fn armed() -> Option<*mut c_void> {
        let armed = unsafe { ffi::MH_H4_TestArmedEnableIncoherent() };
        (!armed.is_null()).then_some(armed)
    }

    pub fn sentinel() -> &'static str {
        // SAFETY: the C returns a pointer to a static string literal.
        unsafe { std::ffi::CStr::from_ptr(ffi::MH_H4_TestFaultSentinel()) }
            .to_str()
            .expect("the sentinel is ASCII")
    }
}

fn status_to_result(status: MH_STATUS) -> Result<(), MH_STATUS> {
    match status {
        MH_STATUS::MH_OK => Ok(()),
        other => Err(other),
    }
}

#[allow(non_camel_case_types)]
#[must_use]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MH_STATUS {
    MH_UNKNOWN = -1,
    MH_OK = 0,
    MH_ERROR_ALREADY_INITIALIZED,
    MH_ERROR_NOT_INITIALIZED,
    MH_ERROR_ALREADY_CREATED,
    MH_ERROR_NOT_CREATED,
    MH_ERROR_ENABLED,
    MH_ERROR_DISABLED,
    MH_ERROR_NOT_EXECUTABLE,
    MH_ERROR_UNSUPPORTED_FUNCTION,
    MH_ERROR_MEMORY_ALLOC,
    MH_ERROR_MEMORY_PROTECT,
    MH_ERROR_MODULE_NOT_FOUND,
    MH_ERROR_FUNCTION_NOT_FOUND,
    MH_ERROR_NOT_LOCKED,
    MH_ERROR_LOCK_HELD,
    MH_ERROR_LOCK_BUSY,
}

impl fmt::Display for MH_STATUS {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for MH_STATUS {}

#[cfg(test)]
mod tests {
    use super::*;

    /// What production change would make this fail? Making `MH_LockAcquire`
    #[test]
    fn a_second_acquisition_on_the_same_thread_is_refused() {
        let first = MinHookLock::acquire().expect("first acquisition");
        assert_eq!(
            MinHookLock::acquire().unwrap_err(),
            MH_STATUS::MH_ERROR_LOCK_HELD
        );
        drop(first);
        let again = MinHookLock::acquire().expect("re-acquisition after release");
        drop(again);
    }

    /// What production change would make this fail? Deleting the ownership
    #[test]
    fn every_operation_refuses_a_caller_that_does_not_hold_the_lock() {
        let target = 1usize as *mut c_void;
        assert_eq!(
            unsafe { ffi::MH_EnableHook(target) },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_DisableHook(target) },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_RemoveHook(target) },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_QueueEnableHook(target) },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_QueueDisableHook(target) },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_ApplyQueued() },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_Initialize() },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_Uninitialize() },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_CreateHook(target, target, std::ptr::null_mut()) },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(
            unsafe { ffi::MH_LockRelease() },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
    }

    /// `EnterSpinLock` inside `MH_LockAcquire`, which runs with the process
    #[test]
    fn the_c_cannot_suspend_a_thread_or_wait_outside_the_lock_acquisition() {
        let hook = include_str!("../vendor/src/hook.c").replace("\r\n", "\n");
        let buffer = include_str!("../vendor/src/buffer.c").replace("\r\n", "\n");
        let trampoline = include_str!("../vendor/src/trampoline.c").replace("\r\n", "\n");
        let source = format!("{hook}\n{buffer}\n{trampoline}");
        for banned in [
            concat!("Suspend", "Thread"),
            concat!("Resume", "Thread"),
            concat!("Open", "Thread"),
            concat!("GetThread", "Context"),
            concat!("SetThread", "Context"),
            concat!("CreateToolhelp32", "Snapshot"),
            concat!("Thread32", "First"),
            concat!("Thread32", "Next"),
            concat!("tlhelp", "32.h"),
            concat!("Wait", "ForSingleObject"),
        ] {
            assert!(
                !source.contains(banned),
                "the fork's C contains `{banned}`; the suspension and relocation \
                 this fork deleted are what made a frozen window unsafe"
            );
        }
        assert_eq!(
            source.matches(concat!("Sle", "ep(")).count(),
            2,
            "the only waits in this library are the two inside \
             `EnterSpinLock`'s contention path"
        );
        let contention = hook
            .split_once("static BOOL EnterSpinLock(SIZE_T spinBudget)")
            .expect("the bounded acquisition exists")
            .1;
        let contention = &contention[..contention.find("\n}").expect("a body")];
        assert_eq!(
            contention.matches(concat!("Sle", "ep(")).count(),
            2,
            "both waits must be inside the acquisition's contention path"
        );
        assert!(
            contention.contains("if (spinCount >= spinBudget)"),
            "the acquisition must refuse when its budget runs out"
        );
        assert_eq!(
            source.matches(concat!("EnterSpin", "Lock(spin")).count(),
            1,
            "exactly one call site may acquire the lock"
        );
        assert_eq!(
            source.matches(concat!("LeaveSpin", "Lock();")).count(),
            1,
            "exactly one call site may release the lock"
        );
        let acquire = source
            .split_once("MH_STATUS WINAPI MH_LockAcquire(SIZE_T spinBudget)")
            .expect("the lock API exists")
            .1;
        let acquire = &acquire[..acquire.find("\n}").expect("a body")];
        assert!(
            acquire.contains(concat!("EnterSpin", "Lock(spin")),
            "the only acquisition must be the one in MH_LockAcquire"
        );
    }

    ///   the roster [`every_exported_operation_checks_lock_ownership`] derives
    ///   and would have to carry the ownership check; more to the point, it
    #[test]
    fn the_test_support_block_adds_no_operation_and_touches_no_library_state() {
        let source = include_str!("../vendor/src/hook.c").replace("\r\n", "\n");
        let opens: Vec<_> = source
            .match_indices("#ifdef MINHOOK_H4_TEST_SUPPORT")
            .collect();
        assert_eq!(
            opens.len(),
            2,
            "exactly two guarded regions: the arming block and the one \
             statement inside `EnableHookLL`"
        );
        for (at, _) in opens {
            let rest = &source[at..];
            let region = &rest[..rest.find("\n#endif").expect("every region closes")];
            assert!(
                !region.contains("MH_STATUS WINAPI MH_"),
                "the test-support block must not declare an operation"
            );
            for banned in [
                "g_hooks",
                "g_hHeap",
                "LockedByCallingThread",
                "EnterSpinLock",
                concat!("Sle", "ep("),
                concat!("Suspend", "Thread"),
                concat!("Resume", "Thread"),
                "HeapAlloc",
                "VirtualProtect",
            ] {
                assert!(
                    !region.contains(banned),
                    "the test-support block contains `{banned}`, which is \
                     either library state or something that can wait inside a \
                     frozen window"
                );
            }
        }
        let build = include_str!("../build.rs").replace("\r\n", "\n");
        assert_eq!(
            build.matches("define(\"MINHOOK_H4_TEST_SUPPORT\"").count(),
            1,
            "exactly one definition site"
        );
        assert!(
            build.contains("CARGO_FEATURE_TEST_SUPPORT"),
            "the definition must come from cargo's feature environment, so an \
             ordinary build cannot reach it"
        );
        assert!(
            !source.contains("#define MINHOOK_H4_TEST_SUPPORT"),
            "the C must not define the guard for itself"
        );
    }

    /// Every entry point checks lock ownership. A new one that forgets is the
    /// `MH_*` entry point without the ownership check.
    #[test]
    fn every_exported_operation_checks_lock_ownership() {
        let source = include_str!("../vendor/src/hook.c").replace("\r\n", "\n");
        let exported: Vec<&str> = source
            .match_indices("MH_STATUS WINAPI MH_")
            .map(|(at, _)| {
                let rest = &source[at + "MH_STATUS WINAPI ".len()..];
                &rest[..rest.find('(').expect("an exported signature")]
            })
            .collect();
        let exempt = ["MH_LockAcquire"];
        for name in &exported {
            if exempt.contains(name) {
                continue;
            }
            let body = source
                .split_once(&format!("MH_STATUS WINAPI {name}("))
                .expect("the body follows the signature")
                .1;
            let body = &body[..body.find("\n}").expect("a body")];
            let delegated = ["EnableHook(", "QueueHook("].iter().find_map(|helper| {
                body.contains(&format!("return {helper}"))
                    .then_some(*helper)
            });
            let checked = match delegated {
                Some(helper) => {
                    let target = source
                        .split_once(&format!("static MH_STATUS {helper}"))
                        .expect("the delegate is defined")
                        .1;
                    target[..target
                        .find(
                            "
}",
                        )
                        .expect("a body")]
                        .contains("if (!LockedByCallingThread())")
                }
                None => body.contains("if (!LockedByCallingThread())"),
            };
            assert!(
                checked,
                "exported operation `{name}` does not refuse an unlocked caller"
            );
        }
        let checked = source.matches("if (!LockedByCallingThread())").count();
        // MH_LockRelease, MH_PatchFootprint, MH_HookIsEnabled,
        assert_eq!(
            checked, 13,
            "every entry point that touches library state must refuse an \
             unlocked caller; found {checked} checks"
        );
    }
}
