//! MinHook, forked so that this product owns the suspension and the lock.
//!
//! # What this fork is for
//!
//! Upstream MinHook does its own thread suspension inside every operation that
//! rewrites bytes. This product already freezes the process itself --
//! `inline_detour`'s freeze iterates to a fixed point, confirms a system roster,
//! checks the access the kernel granted on every handle, and fails closed --
//! and it holds that freeze ACROSS the MinHook call. Upstream's machinery then
//! runs a second time inside our stopped window, and three of its properties
//! become defects there:
//!
//! * ITS LOCK. Every entry point opens with a spin lock whose contention path
//!   calls `Sleep`. Entered with every other thread suspended, a lock held by
//!   one of those threads is a permanent hang of the player's game -- there is
//!   nobody left to release it.
//! * ITS RELOCATION. `ProcessThreadIPs` moved a thread parked in the bytes
//!   about to be rewritten, and returned silently when `GetThreadContext`
//!   failed, and ignored a failed `SetThreadContext`. The bytes were then
//!   patched anyway, so the thread resumed into a half-old, half-new
//!   instruction stream.
//! * ITS UNFREEZE. It reopened each thread by id and ignored both a failed
//!   `OpenThread` and a failed `ResumeThread`, so its own extra suspend count
//!   could outlive the call -- a thread suspended forever.
//!
//! # What it does instead
//!
//! The suspension and the relocation are DELETED from the C, not made optional:
//! `Freeze`, `Unfreeze`, `EnumerateThreads`, `ProcessThreadIPs`, `FindOldIP`
//! and `FindNewIP` are gone, with `tlhelp32.h` and the instruction-boundary
//! tables that only they read. There is no code path left that can suspend a
//! thread or write a thread's context, so no edit can bring the fail-open back
//! without adding it from scratch.
//!
//! In their place:
//!
//! * [`MinHookLock`] makes the lock explicit. It is acquired BEFORE the caller
//!   freezes anything, while every thread of the process is still running, and
//!   held across the whole frozen window. Every other entry point REFUSES with
//!   [`MH_STATUS::MH_ERROR_NOT_LOCKED`] unless the calling thread owns it, so a
//!   caller that forgets gets a named refusal instead of a race. Exactly one
//!   `EnterSpinLock` call survives in the library and it is inside
//!   `MH_LockAcquire`.
//! * [`MinHook::patch_footprint`] reports the exact bytes an enable or disable
//!   will rewrite, and the trampoline slot a thread can be parked in. The
//!   caller asks its freeze whether any held thread is stopped inside either
//!   range and REFUSES the whole transaction if one is, before a byte moves.
//!   That is strictly stronger than the relocation it replaces, which acted on
//!   a context it might have failed to read.
//!
//! # What the caller now owes
//!
//! Every function here that rewrites bytes -- enable, disable, remove, apply --
//! requires BOTH the lock and a process this caller has frozen. The lock is
//! type-checked ([`MinHookLock`] is unforgeable and the C re-checks ownership);
//! the freeze is not, and cannot be from here, because this crate has no
//! business knowing what a `ProcessFreeze` is. `inline_detour` is the only
//! crate in the workspace that calls them, which its own source pin enforces.

#![cfg(windows)]

use std::ffi::c_void;
use std::fmt;

mod ffi;

/// How long an acquisition spins before it refuses.
///
/// The unit is loop iterations, not time: the first 32 yield the rest of this
/// thread's slice (`Sleep(0)`) and the rest sleep a millisecond, so the budget
/// is roughly "a couple of seconds of a lock nobody released". The work it waits
/// on is one `create_hook` -- a trampoline allocation and a prologue disassembly
/// -- which is microseconds. Anything past this budget is a stall, not
/// contention.
pub const DEFAULT_SPIN_BUDGET: usize = 2048;

/// The library-wide lock, held for as long as this value lives.
///
/// # Why it is taken outside the freeze
///
/// Acquiring can WAIT: another thread may be inside `MH_CreateHook` building a
/// trampoline. Waiting is free while the process runs and fatal while it is
/// frozen, so the acquisition is hoisted here, to a point the caller reaches
/// before its first `SuspendThread`. Once held, no other thread can be inside
/// this library at all -- so freezing them cannot capture the lock, which is
/// the whole of the hang this fork exists to remove.
///
/// NOT REENTRANT. A second acquisition on the same thread is
/// [`MH_STATUS::MH_ERROR_LOCK_HELD`], never a deadlock.
#[derive(Debug)]
pub struct MinHookLock {
    /// Not constructible outside this crate.
    _private: (),
}

impl MinHookLock {
    /// Acquire the lock and initialise the library under it.
    ///
    /// Initialisation is INSIDE the lock deliberately: `MH_Initialize` creates
    /// the private heap every other operation allocates from, and this fork
    /// requires the lock for it like everything else. Doing it here also means
    /// no operation performs a one-time initialisation inside a frozen window.
    pub fn acquire() -> Result<Self, MH_STATUS> {
        Self::acquire_within(DEFAULT_SPIN_BUDGET)
    }

    /// Acquire with an explicit spin budget, refusing
    /// [`MH_STATUS::MH_ERROR_LOCK_BUSY`] rather than waiting past it.
    ///
    /// # Why the wait is bounded even though it is outside every frozen window
    ///
    /// Hoisting the acquisition out of the freeze is what stops it hanging a
    /// STOPPED game. It does not stop it hanging the ATTACH: an owner that
    /// stalled or died holding the lock would wedge the install, the teardown
    /// and every `create_hook` with nothing said. The budget turns that into a
    /// named refusal the caller can report, which is the product's rule --
    /// never wait unbounded, fail closed with a reason.
    pub fn acquire_within(spin_budget: usize) -> Result<Self, MH_STATUS> {
        let acquired = unsafe { ffi::MH_LockAcquire(spin_budget) };
        if acquired != MH_STATUS::MH_OK {
            return Err(acquired);
        }
        match unsafe { ffi::MH_Initialize() } {
            MH_STATUS::MH_OK | MH_STATUS::MH_ERROR_ALREADY_INITIALIZED => {
                Ok(Self { _private: () })
            }
            failed => {
                // Release before reporting: a lock left held by a failed
                // acquisition would wedge every later caller.
                let _ = unsafe { ffi::MH_LockRelease() };
                Err(failed)
            }
        }
    }
}

impl Drop for MinHookLock {
    fn drop(&mut self) {
        // A refusal here means the lock was released out from under this guard,
        // which cannot happen through this API. Nothing useful can be done
        // about it at drop time and panicking in a drop would be worse.
        let _ = unsafe { ffi::MH_LockRelease() };
    }
}

/// The exact memory an enable or disable of one target touches.
///
/// Both ranges are `[start, end)`. `patch` is the bytes the operation rewrites
/// -- `[target, target+5)` normally, and `[target-5, target+2)` for a hook that
/// uses the hot-patch area above the function. `trampoline` is the slot holding
/// the relocated prologue, which a disable leaves alone and a remove FREES.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchFootprint {
    /// The bytes an enable/disable rewrites.
    pub patch: (usize, usize),
    /// The trampoline slot for this hook.
    pub trampoline: (usize, usize),
}

/// The MinHook API.
pub struct MinHook {}

impl MinHook {
    /// Create a hook in the DISABLED state, taking the lock for the call.
    ///
    /// Patches nothing: it reads the target's prologue, allocates a trampoline
    /// and records a backup. It is the one operation that does not require the
    /// caller to have frozen the process, and the one that takes the lock
    /// itself -- so it must never be called from inside a frozen window, where
    /// the acquisition could wait forever.
    ///
    /// # Safety
    ///
    /// `target` and `detour` must be executable addresses, and `detour` must
    /// remain a valid callable for as long as the hook exists.
    pub unsafe fn create_hook(
        target: *mut c_void,
        detour: *mut c_void,
    ) -> Result<*mut c_void, MH_STATUS> {
        let lock = MinHookLock::acquire()?;
        unsafe { Self::create_hook_locked(&lock, target, detour) }
    }

    /// [`MinHook::create_hook`] for a caller that already holds the lock.
    ///
    /// # Safety
    ///
    /// As [`MinHook::create_hook`].
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

    /// The exact bytes an enable/disable of `target` rewrites, and its
    /// trampoline slot.
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

    /// Is this target's detour LIVE right now?
    ///
    /// For the one caller that has to restore MinHook's queue to the state it
    /// found: a queue call answers OK for an already-disabled target as readily
    /// as for a live one, so a recovery driven from queue statuses re-queues
    /// targets that were never live.
    pub fn hook_is_enabled(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<bool, MH_STATUS> {
        let mut enabled: i32 = 0;
        match unsafe { ffi::MH_HookIsEnabled(target, &mut enabled) } {
            MH_STATUS::MH_OK => Ok(enabled != 0),
            status => Err(status),
        }
    }

    /// How many hooks exist. Bounds every pending set.
    ///
    /// Asked BEFORE the caller freezes the process, so the buffer that
    /// receives [`MinHook::pending_footprint`] can be reserved outside the
    /// stopped window.
    pub fn hook_count(_lock: &MinHookLock) -> Result<u32, MH_STATUS> {
        let mut count: u32 = 0;
        match unsafe { ffi::MH_HookCount(&mut count) } {
            MH_STATUS::MH_OK => Ok(count),
            status => Err(status),
        }
    }

    /// The `ordinal`-th target whose queued state diverges from its live state,
    /// with its footprint -- the exact set [`MinHook::apply_queued`] will patch.
    ///
    /// `Ok(None)` once `ordinal` is past the end.
    ///
    /// # Why the PENDING set and not the caller's slice
    ///
    /// The queue is library-global. An apply patches every divergent entry it
    /// finds, including one some earlier caller left behind, so a batch that
    /// checked only its own targets would be checking the wrong set.
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

    /// Enable a created hook.
    ///
    /// # Safety
    ///
    /// The caller must hold a freeze over every other thread of this process
    /// and must have established that none of them is stopped inside this
    /// hook's [`PatchFootprint`]. This fork does not relocate instruction
    /// pointers and will not check for you.
    pub unsafe fn enable_hook(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_EnableHook(target) })
    }

    /// Disable an enabled hook, restoring the target's original bytes.
    ///
    /// # Safety
    ///
    /// As [`MinHook::enable_hook`].
    pub unsafe fn disable_hook(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_DisableHook(target) })
    }

    /// Remove a created hook, FREEING its trampoline.
    ///
    /// # Safety
    ///
    /// As [`MinHook::enable_hook`], and additionally: the trampoline slot is
    /// freed, so the caller must also have established that no held thread is
    /// stopped inside [`PatchFootprint::trampoline`].
    pub unsafe fn remove_hook(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_RemoveHook(target) })
    }

    /// Queue a hook for enabling. Patches nothing.
    ///
    /// # Safety
    ///
    /// The queue is library-global, so a queued target left unapplied is a
    /// patch some later `apply_queued` performs. The caller owns that.
    pub unsafe fn queue_enable_hook(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_QueueEnableHook(target) })
    }

    /// Queue a hook for disabling. Patches nothing.
    ///
    /// # Safety
    ///
    /// As [`MinHook::queue_enable_hook`].
    pub unsafe fn queue_disable_hook(
        _lock: &MinHookLock,
        target: *mut c_void,
    ) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_QueueDisableHook(target) })
    }

    /// Apply every queued change, in the library's own creation order.
    ///
    /// # Safety
    ///
    /// As [`MinHook::enable_hook`], for EVERY queued target rather than one.
    pub unsafe fn apply_queued(_lock: &MinHookLock) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_ApplyQueued() })
    }

    /// Uninitialise the library, disabling every hook it still holds.
    ///
    /// # Safety
    ///
    /// As [`MinHook::enable_hook`]: it disables hooks, which rewrites bytes.
    pub unsafe fn uninitialize(_lock: &MinHookLock) -> Result<(), MH_STATUS> {
        status_to_result(unsafe { ffi::MH_Uninitialize() })
    }
}

fn status_to_result(status: MH_STATUS) -> Result<(), MH_STATUS> {
    match status {
        MH_STATUS::MH_OK => Ok(()),
        other => Err(other),
    }
}

/// MinHook status codes. The discriminants are the C enum's, in its order.
#[allow(non_camel_case_types)]
#[must_use]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MH_STATUS {
    /// Unknown error. Should not be returned.
    MH_UNKNOWN = -1,
    /// Successful.
    MH_OK = 0,
    /// MinHook is already initialized.
    MH_ERROR_ALREADY_INITIALIZED,
    /// MinHook is not initialized yet, or already uninitialized.
    MH_ERROR_NOT_INITIALIZED,
    /// The hook for the specified target function is already created.
    MH_ERROR_ALREADY_CREATED,
    /// The hook for the specified target function is not created yet.
    MH_ERROR_NOT_CREATED,
    /// The hook for the specified target function is already enabled.
    MH_ERROR_ENABLED,
    /// The hook for the specified target function is not enabled yet, or
    /// already disabled.
    MH_ERROR_DISABLED,
    /// The specified pointer is invalid.
    MH_ERROR_NOT_EXECUTABLE,
    /// The specified target function cannot be hooked.
    MH_ERROR_UNSUPPORTED_FUNCTION,
    /// Failed to allocate memory.
    MH_ERROR_MEMORY_ALLOC,
    /// Failed to change the memory protection.
    MH_ERROR_MEMORY_PROTECT,
    /// The specified module is not loaded.
    MH_ERROR_MODULE_NOT_FOUND,
    /// The specified function is not found.
    MH_ERROR_FUNCTION_NOT_FOUND,
    /// FORK: the calling thread does not own the MinHook lock.
    MH_ERROR_NOT_LOCKED,
    /// FORK: the calling thread already owns the MinHook lock, which is not
    /// reentrant.
    MH_ERROR_LOCK_HELD,
    /// FORK: another thread still held the lock when the spin budget ran out.
    /// Nothing was touched.
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

    /// The lock is not reentrant, and says so instead of hanging.
    ///
    /// What production change would make this fail? Making `MH_LockAcquire`
    /// spin when the calling thread already owns the lock -- which is exactly
    /// the hang this fork removed, reintroduced one level up.
    #[test]
    fn a_second_acquisition_on_the_same_thread_is_refused() {
        let first = MinHookLock::acquire().expect("first acquisition");
        assert_eq!(
            MinHookLock::acquire().unwrap_err(),
            MH_STATUS::MH_ERROR_LOCK_HELD
        );
        drop(first);
        // And the lock is free again afterwards.
        let again = MinHookLock::acquire().expect("re-acquisition after release");
        drop(again);
    }

    /// Every operation refuses without the lock.
    ///
    /// The guard cannot be forged, so this drives the FFI directly -- which is
    /// what a future edit that reached past `MinHookLock` would do.
    ///
    /// What production change would make this fail? Deleting the ownership
    /// check from any entry point in `hook.c`, which would let an unlocked
    /// caller mutate `g_hooks` while a frozen window walks it.
    #[test]
    fn every_operation_refuses_a_caller_that_does_not_hold_the_lock() {
        // Nothing here is called with the lock held, and none of these touches
        // a target: each must refuse before it looks at one.
        let target = 1usize as *mut c_void;
        assert_eq!(unsafe { ffi::MH_EnableHook(target) }, MH_STATUS::MH_ERROR_NOT_LOCKED);
        assert_eq!(unsafe { ffi::MH_DisableHook(target) }, MH_STATUS::MH_ERROR_NOT_LOCKED);
        assert_eq!(unsafe { ffi::MH_RemoveHook(target) }, MH_STATUS::MH_ERROR_NOT_LOCKED);
        assert_eq!(unsafe { ffi::MH_QueueEnableHook(target) }, MH_STATUS::MH_ERROR_NOT_LOCKED);
        assert_eq!(unsafe { ffi::MH_QueueDisableHook(target) }, MH_STATUS::MH_ERROR_NOT_LOCKED);
        assert_eq!(unsafe { ffi::MH_ApplyQueued() }, MH_STATUS::MH_ERROR_NOT_LOCKED);
        assert_eq!(unsafe { ffi::MH_Initialize() }, MH_STATUS::MH_ERROR_NOT_LOCKED);
        assert_eq!(unsafe { ffi::MH_Uninitialize() }, MH_STATUS::MH_ERROR_NOT_LOCKED);
        assert_eq!(
            unsafe { ffi::MH_CreateHook(target, target, std::ptr::null_mut()) },
            MH_STATUS::MH_ERROR_NOT_LOCKED
        );
        assert_eq!(unsafe { ffi::MH_LockRelease() }, MH_STATUS::MH_ERROR_NOT_LOCKED);
    }

    /// THE PROPERTY THE FORK EXISTS FOR, asserted against the C source itself
    /// because no test can observe a hang that would never end.
    ///
    /// Nothing in this library may suspend a thread, resume one, read or write
    /// a thread's context, enumerate threads, or wait -- except the single
    /// `EnterSpinLock` inside `MH_LockAcquire`, which runs with the process
    /// still moving.
    ///
    /// What production change would make this fail? Restoring any part of
    /// upstream's `Freeze`/`Unfreeze`/`ProcessThreadIPs`, or adding a second
    /// lock acquisition to any operation.
    #[test]
    fn the_c_cannot_suspend_a_thread_or_wait_outside_the_lock_acquisition() {
        // EVERY TRANSLATION UNIT, not just `hook.c`. A wait added to the buffer
        // allocator or the trampoline builder runs inside the caller's frozen
        // window exactly as one in `hook.c` would (review round 1, MED).
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
        // AND `Sleep` ITSELF IS PINNED TO THE ACQUISITION. Counting the
        // acquisitions is not enough: a `Sleep` added anywhere else in the
        // library runs inside the caller's frozen window.
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
        // The contention path is BOUNDED: it gives up rather than spinning for
        // ever on an owner that stalled.
        assert!(
            contention.contains("if (spinCount >= spinBudget)"),
            "the acquisition must refuse when its budget runs out"
        );
        // `Sleep` survives only inside the contention path of the one
        // acquisition, and there is exactly one acquisition.
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

    /// Every entry point checks lock ownership. A new one that forgets is the
    /// defect this catches, and it is spelled as a count so an added function
    /// fails here rather than silently joining the unchecked set.
    ///
    /// What production change would make this fail? Adding an exported
    /// `MH_*` entry point without the ownership check.
    #[test]
    fn every_exported_operation_checks_lock_ownership() {
        let source = include_str!("../vendor/src/hook.c").replace("\r\n", "\n");
        // DERIVED FROM THE EXPORTED SURFACE, not pinned as a number. A count
        // alone passes when an UNCHECKED entry point is added beside a deleted
        // checked one (review round 1, MED). Every exported operation except
        // the lock acquisition itself -- which cannot require the lock it takes
        // -- and the pure status-string helper must carry the check, and so must
        // the two static helpers every enable/queue routes through.
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
            // A thin exported wrapper may DELEGATE to a static that carries
            // the check -- `MH_EnableHook` is `return EnableHook(t, TRUE);` --
            // and that helper is checked by this same loop's requirement that
            // the delegate exist and carry it.
            let delegated = ["EnableHook(", "QueueHook("].iter().find_map(|helper| {
                body.contains(&format!("return {helper}")).then_some(*helper)
            });
            let checked = match delegated {
                Some(helper) => {
                    let target = source
                        .split_once(&format!("static MH_STATUS {helper}"))
                        .expect("the delegate is defined")
                        .1;
                    target[..target.find("
}").expect("a body")]
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
        // MH_LockRelease, MH_PatchFootprint, MH_HookIsEnabled, MH_HookCount,
        // MH_PendingPatchFootprint, MH_Initialize, MH_Uninitialize,
        // MH_CreateHook, MH_RemoveHook, EnableHook, QueueHook, MH_ApplyQueued.
        assert_eq!(
            checked, 12,
            "every entry point that touches library state must refuse an \
             unlocked caller; found {checked} checks"
        );
    }
}
