use crate::MH_STATUS;
use std::ffi::c_void;

extern "system" {
    pub fn MH_Initialize() -> MH_STATUS;

    pub fn MH_Uninitialize() -> MH_STATUS;

    /// FORK ADDITION. Acquire the library-wide lock. THE ONLY CALL IN THIS
    pub fn MH_LockAcquire(spinBudget: usize) -> MH_STATUS;

    /// FORK ADDITION. Release the library-wide lock.
    pub fn MH_LockRelease() -> MH_STATUS;

    pub fn MH_PatchFootprint(
        pTarget: *mut c_void,
        ppPatchStart: *mut *mut c_void,
        pPatchSize: *mut usize,
        ppTrampoline: *mut *mut c_void,
        pTrampolineSize: *mut usize,
    ) -> MH_STATUS;

    pub fn MH_HookIsEnabled(pTarget: *mut c_void, pEnabled: *mut i32) -> MH_STATUS;
    pub fn MH_HookIsIncoherent(pTarget: *mut c_void, pIncoherent: *mut i32) -> MH_STATUS;

    pub fn MH_HookCount(pCount: *mut u32) -> MH_STATUS;

    pub fn MH_PendingPatchFootprint(
        ordinal: u32,
        ppTarget: *mut *mut c_void,
        ppPatchStart: *mut *mut c_void,
        pPatchSize: *mut usize,
        ppTrampoline: *mut *mut c_void,
        pTrampolineSize: *mut usize,
    ) -> MH_STATUS;

    pub fn MH_CreateHook(
        pTarget: *mut c_void,
        pDetour: *mut c_void,
        ppOriginal: *mut *mut c_void,
    ) -> MH_STATUS;

    pub fn MH_RemoveHook(pTarget: *mut c_void) -> MH_STATUS;

    pub fn MH_EnableHook(pTarget: *mut c_void) -> MH_STATUS;

    pub fn MH_DisableHook(pTarget: *mut c_void) -> MH_STATUS;

    pub fn MH_QueueEnableHook(pTarget: *mut c_void) -> MH_STATUS;

    pub fn MH_QueueDisableHook(pTarget: *mut c_void) -> MH_STATUS;

    pub fn MH_ApplyQueued() -> MH_STATUS;
}

#[cfg(feature = "test_support")]
extern "system" {
    pub fn MH_H4_TestArmEnableIncoherent(pTarget: *mut c_void);

    pub fn MH_H4_TestArmedEnableIncoherent() -> *mut c_void;

    pub fn MH_H4_TestFaultSentinel() -> *const std::os::raw::c_char;

    pub fn MH_H4_TestFaultWitness() -> *const std::os::raw::c_char;
}
