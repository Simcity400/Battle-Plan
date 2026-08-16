//! The C entry points this fork exposes, and only those.
//!
//! `MH_CreateHookApi`/`MH_CreateHookApiEx` are deliberately absent: nothing in
//! this workspace resolves a hook by module and export name, and a declared but
//! unreachable FFI entry is a bypass of the lock discipline waiting to be used.

use crate::MH_STATUS;
use std::ffi::c_void;

extern "system" {
    /// Initialize the library. FORK: requires the caller to hold the lock.
    pub fn MH_Initialize() -> MH_STATUS;

    /// Uninitialize the library. FORK: requires the caller to hold the lock.
    pub fn MH_Uninitialize() -> MH_STATUS;

    /// FORK ADDITION. Acquire the library-wide lock. THE ONLY CALL IN THIS
    /// LIBRARY THAT CAN WAIT, and therefore the only one that must never be
    /// made with the process frozen.
    pub fn MH_LockAcquire(spinBudget: usize) -> MH_STATUS;

    /// FORK ADDITION. Release the library-wide lock.
    pub fn MH_LockRelease() -> MH_STATUS;

    /// FORK ADDITION. The exact bytes an enable/disable of `pTarget` rewrites,
    /// and the trampoline slot a thread can be parked in.
    pub fn MH_PatchFootprint(
        pTarget: *mut c_void,
        ppPatchStart: *mut *mut c_void,
        pPatchSize: *mut usize,
        ppTrampoline: *mut *mut c_void,
        pTrampolineSize: *mut usize,
    ) -> MH_STATUS;

    /// FORK ADDITION. Is this target's detour live right now?
    pub fn MH_HookIsEnabled(pTarget: *mut c_void, pEnabled: *mut i32) -> MH_STATUS;

    /// FORK ADDITION. How many hooks exist, which bounds every pending set.
    pub fn MH_HookCount(pCount: *mut u32) -> MH_STATUS;

    /// FORK ADDITION. The footprint of the ordinal-th hook whose queued state
    /// diverges from its live state -- the exact set an apply will patch.
    pub fn MH_PendingPatchFootprint(
        ordinal: u32,
        ppTarget: *mut *mut c_void,
        ppPatchStart: *mut *mut c_void,
        pPatchSize: *mut usize,
        ppTrampoline: *mut *mut c_void,
        pTrampolineSize: *mut usize,
    ) -> MH_STATUS;

    /// Create a hook in the disabled state. Patches nothing.
    pub fn MH_CreateHook(
        pTarget: *mut c_void,
        pDetour: *mut c_void,
        ppOriginal: *mut *mut c_void,
    ) -> MH_STATUS;

    /// Remove a created hook, freeing its trampoline. FORK: does not suspend.
    pub fn MH_RemoveHook(pTarget: *mut c_void) -> MH_STATUS;

    /// Enable a created hook. FORK: does not suspend and does not relocate.
    pub fn MH_EnableHook(pTarget: *mut c_void) -> MH_STATUS;

    /// Disable an enabled hook. FORK: does not suspend and does not relocate.
    pub fn MH_DisableHook(pTarget: *mut c_void) -> MH_STATUS;

    /// Queue a hook for enabling.
    pub fn MH_QueueEnableHook(pTarget: *mut c_void) -> MH_STATUS;

    /// Queue a hook for disabling.
    pub fn MH_QueueDisableHook(pTarget: *mut c_void) -> MH_STATUS;

    /// Apply every queued change. FORK: does not suspend and does not relocate.
    pub fn MH_ApplyQueued() -> MH_STATUS;
}

// The test-support entry points, which exist only when the `test_support`
// feature compiled them into the C. See `crate::EnableIncoherentFault`.
#[cfg(feature = "test_support")]
extern "system" {
    /// Arm: the next completed enable/disable of `pTarget` reports the
    /// completion failure. `NULL` disarms.
    pub fn MH_H4_TestArmEnableIncoherent(pTarget: *mut c_void);

    /// What is armed right now -- `NULL` once the fault has been consumed.
    pub fn MH_H4_TestArmedEnableIncoherent() -> *mut c_void;

    /// The inertness marker.
    pub fn MH_H4_TestFaultSentinel() -> *const std::os::raw::c_char;

    /// The marker the injected branch itself stored, or NULL if it never ran.
    pub fn MH_H4_TestFaultWitness() -> *const std::os::raw::c_char;
}
