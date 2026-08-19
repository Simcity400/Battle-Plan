#![cfg(windows)]

use core::ffi::c_void;
use core::ptr;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

type Bool = i32;
type Dword = u32;
type Handle = *mut c_void;
const MEM_COMMIT: Dword = 0x1000;
const MEM_RESERVE: Dword = 0x2000;
const MEM_RELEASE: Dword = 0x8000;
const PAGE_EXECUTE_READWRITE: Dword = 0x40;
const ABS_JUMP_LEN: usize = 14;
pub const PRODUCTION_SEMANTIC_HOOK_PRIMITIVE_ID: &str =
    "fired_event_popups.a729_inline_hook.production_semantic.v1";

#[repr(C)]
struct RuntimeFunction {
    begin_address: Dword,
    end_address: Dword,
    unwind_data: Dword,
}

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        allocation_type: Dword,
        protect: Dword,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: Dword) -> Bool;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: Dword,
        old_protect: *mut Dword,
    ) -> Bool;
    fn FlushInstructionCache(process: Handle, address: *const c_void, size: usize) -> Bool;
    fn GetCurrentProcess() -> Handle;
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlLookupFunctionEntry(
        control_pc: u64,
        image_base: *mut u64,
        history_table: *mut c_void,
    ) -> *const RuntimeFunction;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteKind {
    Fire,
    Materialise,
    SelectedDescription,
    PictureSelector,
    SpriteLookup,
    PlayerChoice,
    Timeout,
    OptionCommand,
    OptionAdmission,
}

#[derive(Clone, Copy)]
pub struct HookSite {
    pub kind: SiteKind,
    pub rva: usize,
    pub expected: &'static [u8],
}

pub const FIRE: HookSite = HookSite {
    kind: SiteKind::Fire,
    rva: 0x0A028A0,
    expected: &[
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18,
    ],
};
pub const MATERIALISE: HookSite = HookSite {
    kind: SiteKind::Materialise,
    rva: 0x1226000,
    expected: &[
        0x48, 0x8b, 0xc4, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
    ],
};
pub const SELECTED_DESCRIPTION: HookSite = HookSite {
    kind: SiteKind::SelectedDescription,
    rva: 0x116C180,
    expected: &[
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54,
    ],
};
pub const PICTURE_SELECTOR: HookSite = HookSite {
    kind: SiteKind::PictureSelector,
    rva: 0x1225130,
    expected: &[
        0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x18, 0x55, 0x57, 0x41, 0x54,
    ],
};
pub const SPRITE_LOOKUP: HookSite = HookSite {
    kind: SiteKind::SpriteLookup,
    rva: 0x0A2D0D0,
    expected: &[
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
    ],
};
pub const PLAYER_CHOICE: HookSite = HookSite {
    kind: SiteKind::PlayerChoice,
    rva: 0x12249F0,
    expected: &[
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30,
    ],
};
pub const TIMEOUT: HookSite = HookSite {
    kind: SiteKind::Timeout,
    rva: 0x1227360,
    expected: &[
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
        0x57,
    ],
};
pub const OPTION_COMMAND: HookSite = HookSite {
    kind: SiteKind::OptionCommand,
    rva: 0x1527B30,
    expected: &[
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x48, 0x83,
        0xec, 0x30,
    ],
};
pub const OPTION_ADMISSION: HookSite = HookSite {
    kind: SiteKind::OptionAdmission,
    rva: 0x15279A0,
    expected: &[
        0x80, 0xb9, 0xf5, 0x01, 0x00, 0x00, 0x00, 0x74, 0x0c, 0x44, 0x38, 0x81, 0xf4, 0x01, 0x00,
        0x00,
    ],
};
/// widget ownership, text setters, sprite lookup, or player-choice UI code.
pub const PRODUCTION_SEMANTIC_SITES: &[HookSite] = &[FIRE, OPTION_COMMAND, TIMEOUT];

#[cfg(test)]
pub const FULL_EVIDENCE_SITES: &[HookSite] = &[
    FIRE,
    MATERIALISE,
    SELECTED_DESCRIPTION,
    PICTURE_SELECTOR,
    SPRITE_LOOKUP,
    PLAYER_CHOICE,
    TIMEOUT,
    OPTION_COMMAND,
    OPTION_ADMISSION,
];

#[derive(Clone, Copy)]
struct RoleProducerSite {
    label: &'static str,
    rva: usize,
    expected: &'static [u8],
}

const ROLE_PRODUCER_SITES: &[RoleProducerSite] = &[
    RoleProducerSite {
        label: "title text setter",
        rva: 0x1226334,
        expected: &[0xE8, 0xA7, 0xF6, 0x08, 0x01],
    },
    RoleProducerSite {
        label: "description text setter",
        rva: 0x1226D84,
        expected: &[0xE8, 0x57, 0xEC, 0x08, 0x01],
    },
    RoleProducerSite {
        label: "option text setter",
        rva: 0x1223DEB,
        expected: &[0xE8, 0xF0, 0x1B, 0x09, 0x01],
    },
];

#[derive(Debug, PartialEq, Eq)]
pub enum InstallError {
    NullImageBase,
    NullCallback(SiteKind),
    ByteMismatch(SiteKind),
    RoleProducerByteMismatch(&'static str),
    Allocation(SiteKind),
    Protect(SiteKind),
    Flush(SiteKind),
    FreezeRefused(inline_detour::FreezeRefusal),
    CallbackRegion,
    CallbacksInFlight(usize),
    InstructionPointerInHook,
    MissingOriginalSlot(SiteKind),
    Receipt(String),
    RecoveryCustodyRetained {
        install: Box<InstallError>,
        cleanup: Box<InstallError>,
    },
    RecoveryCustodyPending(Box<InstallError>),
}

static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static CALLBACKS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

pub struct CallbackGuard;

impl CallbackGuard {
    pub fn enter() -> Self {
        CALLBACKS_IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }

    pub fn semantic_admission_open(&self) -> bool {
        CALLBACKS_OPEN.load(Ordering::Acquire)
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        CALLBACKS_IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
pub(crate) struct TestSemanticAdmissionGuard {
    _install_guard: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for TestSemanticAdmissionGuard {
    fn drop(&mut self) {
        CALLBACKS_OPEN.store(false, Ordering::Release);
    }
}

#[cfg(test)]
pub(crate) fn open_test_semantic_admission() -> TestSemanticAdmissionGuard {
    let install_guard = INSTALL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    CALLBACKS_OPEN.store(true, Ordering::Release);
    TestSemanticAdmissionGuard {
        _install_guard: install_guard,
    }
}

pub(crate) struct InstalledHook {
    site: HookSite,
    target: *mut u8,
    trampoline: *mut u8,
    original: Vec<u8>,
    installed_receipt: Option<InstalledProductionSemanticHookReceipt>,
    original_page_protection: Dword,
    page_protection_restored: bool,
    target_patched: bool,
}

unsafe impl Send for InstalledHook {}

struct HookInstallFailure {
    install_error: InstallError,
    cleanup_error: Option<InstallError>,
    custody: Option<InstalledHook>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledExactRvaOutcome {
    pub rva: u64,
    pub patch_len: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledProductionSemanticHookReceipt {
    logical_name: &'static str,
    rva: u64,
    patch_len: u16,
    original_prefix_sha256: [u8; 32],
    destination_binding_sha256: [u8; 32],
    teardown_owner_token: u64,
}

impl InstalledProductionSemanticHookReceipt {
    pub fn logical_name(&self) -> &'static str {
        self.logical_name
    }
    pub fn rva(&self) -> u64 {
        self.rva
    }
    pub fn patch_len(&self) -> u16 {
        self.patch_len
    }
    pub fn original_prefix_sha256(&self) -> [u8; 32] {
        self.original_prefix_sha256
    }
    pub fn destination_binding_sha256(&self) -> [u8; 32] {
        self.destination_binding_sha256
    }
    pub fn teardown_owner_token(&self) -> u64 {
        self.teardown_owner_token
    }
    pub fn custody_active(&self) -> bool {
        self.teardown_owner_token != 0
            && self.original_prefix_sha256 != [0; 32]
            && self.destination_binding_sha256 != [0; 32]
    }
}

/// had its trampoline released by the same unique custody owner that installed
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleasedProductionSemanticHookReceipt {
    logical_name: &'static str,
    rva: u64,
    patch_len: u16,
    original_prefix_sha256: [u8; 32],
    destination_binding_sha256: [u8; 32],
    restored_prefix_sha256: [u8; 32],
    teardown_owner_token: u64,
    callbacks_quiesced: bool,
    hook_removed: bool,
    trampoline_freed: bool,
}

impl ReleasedProductionSemanticHookReceipt {
    pub fn logical_name(&self) -> &'static str {
        self.logical_name
    }
    pub fn rva(&self) -> u64 {
        self.rva
    }
    pub fn patch_len(&self) -> u16 {
        self.patch_len
    }
    pub fn teardown_owner_token(&self) -> u64 {
        self.teardown_owner_token
    }
    pub fn original_prefix_sha256(&self) -> [u8; 32] {
        self.original_prefix_sha256
    }
    pub fn restored_prefix_sha256(&self) -> [u8; 32] {
        self.restored_prefix_sha256
    }
    pub fn destination_binding_sha256(&self) -> [u8; 32] {
        self.destination_binding_sha256
    }
    pub fn custody_complete(&self) -> bool {
        self.teardown_owner_token != 0
            && self.callbacks_quiesced
            && self.hook_removed
            && self.trampoline_freed
            && self.original_prefix_sha256 == self.restored_prefix_sha256
    }
}

impl InstalledHook {
    pub fn kind(&self) -> SiteKind {
        self.site.kind
    }
    pub fn trampoline(&self) -> *const c_void {
        self.trampoline.cast()
    }
    fn target_patched(&self) -> bool {
        self.target_patched
    }

    unsafe fn install(
        image_base: usize,
        site: HookSite,
        callback: *const c_void,
        teardown_owner_token: Option<u64>,
    ) -> Result<Self, HookInstallFailure> {
        if callback.is_null() {
            return Err(HookInstallFailure::clean(InstallError::NullCallback(
                site.kind,
            )));
        }
        let target = image_base
            .checked_add(site.rva)
            .ok_or_else(|| HookInstallFailure::clean(InstallError::NullImageBase))?
            as *mut u8;
        let current = unsafe { std::slice::from_raw_parts(target, site.expected.len()) };
        if current != site.expected {
            return Err(HookInstallFailure::clean(InstallError::ByteMismatch(
                site.kind,
            )));
        }
        let production_logical_name = teardown_owner_token
            .map(|_| match site.kind {
                SiteKind::Fire => Ok("gameplay.d75.common_fire"),
                SiteKind::Timeout => Ok("gameplay.d75.timeout_resolution"),
                SiteKind::OptionCommand => Ok("gameplay.d75.option_command"),
                _ => Err(InstallError::Receipt(
                    "D75 production receipt encountered a non-semantic site".into(),
                )),
            })
            .transpose()
            .map_err(HookInstallFailure::clean)?;
        let trampoline_len = site.expected.len() + ABS_JUMP_LEN;
        let trampoline = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                trampoline_len,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        if trampoline.is_null() {
            return Err(HookInstallFailure::clean(InstallError::Allocation(
                site.kind,
            )));
        }
        let original = site.expected.to_vec();
        let mut hook = Self {
            site,
            target,
            trampoline,
            original,
            installed_receipt: None,
            original_page_protection: 0,
            page_protection_restored: false,
            target_patched: false,
        };
        unsafe {
            ptr::copy_nonoverlapping(site.expected.as_ptr(), trampoline, site.expected.len());
        }
        unsafe {
            write_abs_jump(
                trampoline.add(site.expected.len()),
                target.add(site.expected.len()).cast(),
            );
        }
        if unsafe { FlushInstructionCache(GetCurrentProcess(), trampoline.cast(), trampoline_len) }
            == 0
        {
            return Err(hook.rollback_failed_install(InstallError::Flush(site.kind)));
        }
        let mut old = 0;
        if unsafe {
            VirtualProtect(
                target.cast(),
                site.expected.len(),
                PAGE_EXECUTE_READWRITE,
                &mut old,
            )
        } == 0
        {
            return Err(hook.rollback_failed_install(InstallError::Protect(site.kind)));
        }
        hook.original_page_protection = old;
        unsafe {
            write_abs_jump(target, callback);
            for i in ABS_JUMP_LEN..site.expected.len() {
                target.add(i).write(0x90);
            }
        }
        hook.target_patched = true;
        let mut ignored = 0;
        if unsafe { VirtualProtect(target.cast(), site.expected.len(), old, &mut ignored) } == 0 {
            return Err(hook.rollback_failed_install(InstallError::Protect(site.kind)));
        }
        hook.page_protection_restored = true;
        if unsafe { FlushInstructionCache(GetCurrentProcess(), target.cast(), site.expected.len()) }
            == 0
        {
            return Err(hook.rollback_failed_install(InstallError::Flush(site.kind)));
        }
        let patched = unsafe { std::slice::from_raw_parts(target, site.expected.len()) };
        let mut observed_destination = [0u8; 8];
        if patched.len() >= ABS_JUMP_LEN {
            observed_destination.copy_from_slice(&patched[6..14]);
        }
        if patched.len() < ABS_JUMP_LEN
            || patched[..6] != [0xff, 0x25, 0, 0, 0, 0]
            || patched[ABS_JUMP_LEN..].iter().any(|byte| *byte != 0x90)
            || usize::try_from(u64::from_le_bytes(observed_destination)).ok()
                != Some(callback as usize)
        {
            return Err(hook.rollback_failed_install(InstallError::Receipt(
                "D75 custom detour destination binding mismatch".into(),
            )));
        }
        hook.installed_receipt = teardown_owner_token.zip(production_logical_name).map(
            |(teardown_owner_token, logical_name)| {
                let original_prefix_sha256: [u8; 32] = Sha256::digest(&hook.original).into();
                let mut binding = Sha256::new();
                binding.update(b"battle-plan.d75.custom_detour_binding.v1\0");
                binding.update(logical_name.as_bytes());
                binding.update((site.rva as u64).to_le_bytes());
                binding.update((site.expected.len() as u16).to_le_bytes());
                binding.update((callback as usize).to_le_bytes());
                binding.update(teardown_owner_token.to_le_bytes());
                InstalledProductionSemanticHookReceipt {
                    logical_name,
                    rva: site.rva as u64,
                    patch_len: site.expected.len() as u16,
                    original_prefix_sha256,
                    destination_binding_sha256: binding.finalize().into(),
                    teardown_owner_token,
                }
            },
        );
        Ok(hook)
    }

    unsafe fn uninstall(&mut self) -> Result<(), InstallError> {
        if self.trampoline.is_null() && !self.target_patched {
            return Ok(());
        }
        if self.target_patched {
            unsafe {
                restore_with_expected_protection(
                    self.site.kind,
                    self.target,
                    &self.original,
                    self.original_page_protection,
                )?
            };
            self.target_patched = false;
            self.page_protection_restored = true;
        }
        if !self.trampoline.is_null() {
            if unsafe { VirtualFree(self.trampoline.cast(), 0, MEM_RELEASE) } == 0 {
                return Err(InstallError::Allocation(self.site.kind));
            }
            self.trampoline = ptr::null_mut();
        }
        Ok(())
    }

    fn rollback_failed_install(mut self, install_error: InstallError) -> HookInstallFailure {
        match unsafe { self.uninstall() } {
            Ok(()) => HookInstallFailure::clean(install_error),
            Err(cleanup_error) => HookInstallFailure {
                install_error,
                cleanup_error: Some(cleanup_error),
                custody: Some(self),
            },
        }
    }

    fn released_receipt(
        &self,
        callbacks_quiesced: bool,
    ) -> Result<Option<ReleasedProductionSemanticHookReceipt>, InstallError> {
        let Some(installed_receipt) = self.installed_receipt.as_ref() else {
            return Ok(None);
        };
        if !self.trampoline.is_null() {
            return Err(InstallError::Receipt(
                "D75 trampoline remains allocated after hook restoration".into(),
            ));
        }
        if !self.page_protection_restored {
            return Err(InstallError::Receipt(
                "D75 target page protection was not restored".into(),
            ));
        }
        let restored =
            unsafe { std::slice::from_raw_parts(self.target.cast_const(), self.original.len()) };
        let restored_prefix_sha256: [u8; 32] = Sha256::digest(restored).into();
        if restored != self.original.as_slice()
            || restored_prefix_sha256 != installed_receipt.original_prefix_sha256
            || installed_receipt.teardown_owner_token == 0
        {
            return Err(InstallError::Receipt(
                "D75 restored prefix or teardown owner does not match installation custody".into(),
            ));
        }
        Ok(Some(ReleasedProductionSemanticHookReceipt {
            logical_name: installed_receipt.logical_name,
            rva: installed_receipt.rva,
            patch_len: installed_receipt.patch_len,
            original_prefix_sha256: installed_receipt.original_prefix_sha256,
            destination_binding_sha256: installed_receipt.destination_binding_sha256,
            restored_prefix_sha256,
            teardown_owner_token: installed_receipt.teardown_owner_token,
            callbacks_quiesced,
            hook_removed: true,
            trampoline_freed: true,
        }))
    }
}

impl Drop for InstalledHook {
    fn drop(&mut self) {
        if self.target_patched || !self.trampoline.is_null() {
            unsafe {
                let _ = self.uninstall();
            }
        }
    }
}

impl HookInstallFailure {
    fn clean(install_error: InstallError) -> Self {
        Self {
            install_error,
            cleanup_error: None,
            custody: None,
        }
    }
}

pub(crate) struct InstalledHookSet {
    install_guard: Option<MutexGuard<'static, ()>>,
    publication_suspend: Option<inline_detour::ProcessFreeze>,
    hooks: Vec<InstalledHook>,
    original_slots: Vec<&'static AtomicUsize>,
    callback_regions: Vec<(usize, usize)>,
    released_receipts: Vec<ReleasedProductionSemanticHookReceipt>,
    execution_ranges: Vec<(usize, usize)>,
}

static FAILED_INSTALL_CUSTODY: Mutex<Vec<InstalledHookSet>> = Mutex::new(Vec::new());

pub(crate) struct CallbackBinding {
    pub(crate) kind: SiteKind,
    pub(crate) callback: *const c_void,
    pub(crate) original_slot: Option<&'static AtomicUsize>,
}
unsafe impl Send for InstalledHookSet {}
impl InstalledHookSet {
    pub(crate) fn publish(&mut self) {
        CALLBACKS_OPEN.store(true, Ordering::Release);
        self.publication_suspend.take();
    }
    pub(crate) fn trampoline(&self, kind: SiteKind) -> Option<*const c_void> {
        if kind == SiteKind::OptionAdmission {
            return None;
        }
        self.hooks
            .iter()
            .find(|h| h.kind() == kind)
            .map(InstalledHook::trampoline)
    }
    pub(crate) fn len(&self) -> usize {
        self.hooks.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
    pub(crate) fn installed_exact_rva_outcomes(&self) -> Vec<InstalledExactRvaOutcome> {
        let mut outcomes = self
            .hooks
            .iter()
            .map(|hook| InstalledExactRvaOutcome {
                rva: hook.site.rva as u64,
                patch_len: hook.site.expected.len() as u16,
            })
            .collect::<Vec<_>>();
        outcomes.sort_unstable_by_key(|outcome| (outcome.rva, outcome.patch_len));
        outcomes
    }

    pub(crate) fn installed_production_receipts(
        &self,
    ) -> Vec<InstalledProductionSemanticHookReceipt> {
        let mut receipts = self
            .hooks
            .iter()
            .filter_map(|hook| hook.installed_receipt.clone())
            .collect::<Vec<_>>();
        receipts.sort_unstable_by_key(|receipt| (receipt.rva, receipt.patch_len));
        receipts
    }

    pub(crate) fn released_receipts(&self) -> &[ReleasedProductionSemanticHookReceipt] {
        &self.released_receipts
    }

    fn fill_execution_ranges(&mut self) {
        self.execution_ranges.clear();
        for hook in &self.hooks {
            self.execution_ranges.push((
                hook.target as usize,
                hook.target as usize + hook.site.expected.len(),
            ));
            self.execution_ranges.push((
                hook.trampoline as usize,
                hook.trampoline as usize + hook.site.expected.len() + ABS_JUMP_LEN,
            ));
        }
        for region in &self.callback_regions {
            self.execution_ranges.push(*region);
        }
    }

    fn execution_range_capacity(sites: usize, callback_regions: usize) -> usize {
        sites * 2 + callback_regions
    }

    pub(crate) fn try_uninstall(&mut self) -> Result<(), InstallError> {
        CALLBACKS_OPEN.store(false, Ordering::Release);
        self.fill_execution_ranges();
        // The freeze used to be acquired under a second name inside the `else`
        let took_fresh_freeze = self.publication_suspend.is_none();
        let suspended = match self.publication_suspend.take() {
            Some(carried) => carried,
            None => suspend_all_other_threads()?,
        };
        if took_fresh_freeze {
            let in_flight = CALLBACKS_IN_FLIGHT.load(Ordering::Acquire);
            if in_flight != 0 {
                drop(suspended);
                return Err(InstallError::CallbacksInFlight(in_flight));
            }
            if any_instruction_pointer_in(&suspended, &self.execution_ranges)? {
                drop(suspended);
                return Err(InstallError::InstructionPointerInHook);
            }
        }
        for hook in self.hooks.iter_mut().rev() {
            unsafe { hook.uninstall()? };
        }
        for slot in &self.original_slots {
            slot.store(0, Ordering::Release);
        }
        drop(suspended);
        // ==== AND WHAT DOES NOT: THE RELEASE RECEIPTS ====
        let mut released = self
            .hooks
            .iter()
            .map(|hook| hook.released_receipt(true))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        released.sort_unstable_by_key(|receipt| (receipt.rva, receipt.patch_len));
        let production_receipts_required = self
            .hooks
            .iter()
            .any(|hook| hook.installed_receipt.is_some());
        if production_receipts_required
            && (released.len() != 3 || released.iter().any(|receipt| !receipt.custody_complete()))
        {
            return Err(InstallError::Receipt(
                "D75 exact three-site release custody is incomplete".into(),
            ));
        }
        self.released_receipts = released;
        self.hooks.clear();
        Ok(())
    }

    /// No release receipt is produced because an incomplete roster was never a
    fn try_rollback_partial_install(&mut self) -> Result<(), InstallError> {
        CALLBACKS_OPEN.store(false, Ordering::Release);
        self.fill_execution_ranges();
        let took_fresh_freeze = self.publication_suspend.is_none();
        let suspended = match self.publication_suspend.take() {
            Some(carried) => carried,
            None => suspend_all_other_threads()?,
        };
        if took_fresh_freeze {
            let in_flight = CALLBACKS_IN_FLIGHT.load(Ordering::Acquire);
            if in_flight != 0 {
                drop(suspended);
                return Err(InstallError::CallbacksInFlight(in_flight));
            }
            if any_instruction_pointer_in(&suspended, &self.execution_ranges)? {
                drop(suspended);
                return Err(InstallError::InstructionPointerInHook);
            }
        }
        for hook in self.hooks.iter_mut().rev() {
            unsafe { hook.uninstall()? };
        }
        for slot in &self.original_slots {
            slot.store(0, Ordering::Release);
        }
        drop(suspended);
        self.hooks.clear();
        Ok(())
    }
}

impl Drop for InstalledHookSet {
    fn drop(&mut self) {
        if self.hooks.is_empty() {
            return;
        }
        CALLBACKS_OPEN.store(false, Ordering::Release);
        self.fill_execution_ranges();
        let suspended = match self.publication_suspend.take() {
            Some(suspended) => Ok(suspended),
            None => suspend_all_other_threads(),
        };
        if let Ok(suspended) = suspended {
            if CALLBACKS_IN_FLIGHT.load(Ordering::Acquire) != 0
                || any_instruction_pointer_in(&suspended, &self.execution_ranges).unwrap_or(true)
            {
                drop(suspended);
                for hook in self.hooks.drain(..) {
                    core::mem::forget(hook);
                }
                if let Some(guard) = self.install_guard.take() {
                    core::mem::forget(guard);
                }
                return;
            }
            let mut failed = false;
            for hook in self.hooks.iter_mut().rev() {
                if unsafe { hook.uninstall() }.is_err() {
                    failed = true;
                    break;
                }
            }
            if failed {
                for hook in self.hooks.drain(..) {
                    core::mem::forget(hook);
                }
                if let Some(guard) = self.install_guard.take() {
                    core::mem::forget(guard);
                }
                return;
            }
            for slot in &self.original_slots {
                slot.store(0, Ordering::Release);
            }
            drop(suspended);
            self.hooks.clear();
        } else {
            for hook in self.hooks.drain(..) {
                core::mem::forget(hook);
            }
            if let Some(guard) = self.install_guard.take() {
                core::mem::forget(guard);
            }
        }
    }
}

pub(crate) unsafe fn install_production_semantic(
    image_base: usize,
    callbacks: &[CallbackBinding],
) -> Result<InstalledHookSet, InstallError> {
    unsafe { install_sites(image_base, callbacks, PRODUCTION_SEMANTIC_SITES, false) }
}

#[cfg(test)]
pub(crate) unsafe fn install_full_evidence_unshipped(
    image_base: usize,
    callbacks: &[CallbackBinding],
) -> Result<InstalledHookSet, InstallError> {
    unsafe { install_sites(image_base, callbacks, FULL_EVIDENCE_SITES, true) }
}

unsafe fn install_sites(
    image_base: usize,
    callbacks: &[CallbackBinding],
    sites: &[HookSite],
    validate_role_producers: bool,
) -> Result<InstalledHookSet, InstallError> {
    if image_base == 0 {
        return Err(InstallError::NullImageBase);
    }
    let guard = INSTALL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    retry_failed_install_cleanup_locked()
        .map_err(|error| InstallError::RecoveryCustodyPending(Box::new(error)))?;
    CALLBACKS_OPEN.store(false, Ordering::Release);

    let target_ranges = sites
        .iter()
        .map(|site| {
            image_base
                .checked_add(site.rva)
                .and_then(|start| Some((start, start.checked_add(site.expected.len())?)))
                .ok_or(InstallError::NullImageBase)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut callback_regions = Vec::new();
    for binding in callbacks
        .iter()
        .filter(|binding| sites.iter().any(|site| site.kind == binding.kind))
    {
        let region = unsafe { callback_function_range(binding.callback)? };
        if !callback_regions.contains(&region) {
            callback_regions.push(region);
        }
    }
    for site in sites {
        let binding = callbacks.iter().find(|binding| binding.kind == site.kind);
        if site.kind != SiteKind::OptionAdmission
            && binding.and_then(|binding| binding.original_slot).is_none()
        {
            return Err(InstallError::MissingOriginalSlot(site.kind));
        }
    }
    let mut original_slots: Vec<&'static AtomicUsize> = sites
        .iter()
        .filter_map(|site| {
            callbacks
                .iter()
                .find(|binding| binding.kind == site.kind)
                .and_then(|binding| binding.original_slot)
        })
        .collect();
    let teardown_owner_token = if !validate_role_producers && sites.len() == 3 {
        Some(production_hook_receipt::next_teardown_owner_token().map_err(InstallError::Receipt)?)
    } else {
        None
    };
    let mut hooks = Vec::with_capacity(sites.len());
    let mut recovery_original_slots: Vec<(&'static AtomicUsize, usize)> =
        Vec::with_capacity(sites.len());
    let execution_ranges = Vec::with_capacity(InstalledHookSet::execution_range_capacity(
        sites.len(),
        callback_regions.len(),
    ));

    let suspended = suspend_all_other_threads()?;
    if any_instruction_pointer_in(&suspended, &target_ranges)? {
        return Err(InstallError::InstructionPointerInHook);
    }
    if validate_role_producers {
        for site in ROLE_PRODUCER_SITES {
            let address = image_base
                .checked_add(site.rva)
                .ok_or(InstallError::NullImageBase)? as *const u8;
            let current = unsafe { std::slice::from_raw_parts(address, site.expected.len()) };
            if current != site.expected {
                return Err(InstallError::RoleProducerByteMismatch(site.label));
            }
        }
    }
    for site in sites {
        let binding = callbacks.iter().find(|binding| binding.kind == site.kind);
        let callback = binding
            .map(|binding| binding.callback)
            .unwrap_or(ptr::null());
        match unsafe { InstalledHook::install(image_base, *site, callback, teardown_owner_token) } {
            Ok(hook) => hooks.push(hook),
            Err(failure) => {
                let HookInstallFailure {
                    install_error,
                    cleanup_error: _initial_cleanup_error,
                    custody,
                } = failure;
                if let Some(hook) = custody {
                    hooks.push(hook);
                }
                recovery_original_slots.clear();
                for hook in hooks.iter().filter(|hook| hook.target_patched()) {
                    let Some(slot) = callbacks
                        .iter()
                        .find(|binding| binding.kind == hook.kind())
                        .and_then(|binding| binding.original_slot)
                    else {
                        continue;
                    };
                    recovery_original_slots.push((slot, hook.trampoline() as usize));
                }
                for (slot, trampoline) in &recovery_original_slots {
                    slot.store(*trampoline, Ordering::Release);
                }
                original_slots.clear();
                for (slot, _) in &recovery_original_slots {
                    original_slots.push(*slot);
                }
                let mut partial = InstalledHookSet {
                    install_guard: Some(guard),
                    publication_suspend: Some(suspended),
                    hooks,
                    original_slots,
                    callback_regions,
                    released_receipts: Vec::new(),
                    execution_ranges,
                };
                match partial.try_rollback_partial_install() {
                    Ok(()) => return Err(install_error),
                    Err(cleanup_error) => {
                        // THE FREEZE IS RELEASED FIRST, and only then is
                        let held_suspend = partial.publication_suspend.take();
                        let held_install_guard = partial.install_guard.take();
                        drop(held_suspend);
                        let mut recovery = FAILED_INSTALL_CUSTODY
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        recovery.push(partial);
                        drop(recovery);
                        drop(held_install_guard);
                        return Err(InstallError::RecoveryCustodyRetained {
                            install: Box::new(install_error),
                            cleanup: Box::new(cleanup_error),
                        });
                    }
                }
            }
        }
    }
    for hook in &hooks {
        if let Some(slot) = callbacks
            .iter()
            .find(|binding| binding.kind == hook.kind())
            .and_then(|binding| binding.original_slot)
        {
            slot.store(hook.trampoline() as usize, Ordering::Release);
        }
    }
    Ok(InstalledHookSet {
        install_guard: Some(guard),
        publication_suspend: Some(suspended),
        hooks,
        original_slots,
        callback_regions,
        released_receipts: Vec::new(),
        execution_ranges,
    })
}

pub fn retry_failed_install_cleanup() -> Result<(), InstallError> {
    let _guard = INSTALL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    retry_failed_install_cleanup_locked()
}

pub fn failed_install_cleanup_pending() -> bool {
    !FAILED_INSTALL_CUSTODY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty()
}

fn retry_failed_install_cleanup_locked() -> Result<(), InstallError> {
    let mut recovery = FAILED_INSTALL_CUSTODY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while let Some(custody) = recovery.first_mut() {
        custody.try_rollback_partial_install()?;
        let completed = recovery.remove(0);
        drop(completed);
    }
    Ok(())
}

unsafe fn callback_function_range(callback: *const c_void) -> Result<(usize, usize), InstallError> {
    if callback.is_null() {
        return Err(InstallError::CallbackRegion);
    }
    let mut image_base = 0u64;
    let entry =
        unsafe { RtlLookupFunctionEntry(callback as u64, &mut image_base, ptr::null_mut()) };
    if entry.is_null() || image_base == 0 {
        return Err(InstallError::CallbackRegion);
    }
    let entry = unsafe { &*entry };
    let start = (image_base as usize)
        .checked_add(entry.begin_address as usize)
        .ok_or(InstallError::CallbackRegion)?;
    let end = (image_base as usize)
        .checked_add(entry.end_address as usize)
        .ok_or(InstallError::CallbackRegion)?;
    if start >= end || (callback as usize) < start || (callback as usize) >= end {
        return Err(InstallError::CallbackRegion);
    }
    Ok((start, end))
}

fn suspend_all_other_threads() -> Result<inline_detour::ProcessFreeze, InstallError> {
    inline_detour::freeze_process().map_err(InstallError::FreezeRefused)
}

/// inside a callback body that is about to be released?
fn any_instruction_pointer_in(
    suspended: &inline_detour::ProcessFreeze,
    ranges: &[(usize, usize)],
) -> Result<bool, InstallError> {
    suspended
        .any_instruction_pointer_in(ranges)
        .map_err(InstallError::FreezeRefused)
}

unsafe fn write_abs_jump(at: *mut u8, destination: *const c_void) {
    unsafe {
        at.write(0xFF);
        at.add(1).write(0x25);
        at.add(2).write(0);
        at.add(3).write(0);
        at.add(4).write(0);
        at.add(5).write(0);
        ptr::copy_nonoverlapping(&(destination as u64).to_le_bytes()[0], at.add(6), 8);
    }
}
unsafe fn restore_with_expected_protection(
    kind: SiteKind,
    target: *mut u8,
    original: &[u8],
    expected_protection: Dword,
) -> Result<(), InstallError> {
    if expected_protection == 0 {
        return Err(InstallError::Protect(kind));
    }
    let mut prior = 0;
    if unsafe {
        VirtualProtect(
            target.cast(),
            original.len(),
            PAGE_EXECUTE_READWRITE,
            &mut prior,
        )
    } == 0
    {
        return Err(InstallError::Protect(kind));
    }
    unsafe {
        ptr::copy_nonoverlapping(original.as_ptr(), target, original.len());
    }
    let mut replaced = 0;
    if unsafe {
        VirtualProtect(
            target.cast(),
            original.len(),
            expected_protection,
            &mut replaced,
        )
    } == 0
    {
        return Err(InstallError::Protect(kind));
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), target.cast(), original.len()) } == 0 {
        return Err(InstallError::Flush(kind));
    }
    let restored = unsafe { std::slice::from_raw_parts(target, original.len()) };
    if restored != original {
        return Err(InstallError::ByteMismatch(kind));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) unsafe fn seed_exact_test_image(base: *mut u8) {
    for site in FULL_EVIDENCE_SITES {
        unsafe {
            ptr::copy_nonoverlapping(
                site.expected.as_ptr(),
                base.add(site.rva),
                site.expected.len(),
            );
        }
    }
    for site in ROLE_PRODUCER_SITES {
        unsafe {
            ptr::copy_nonoverlapping(
                site.expected.as_ptr(),
                base.add(site.rva),
                site.expected.len(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn production_code() -> String {
        let source = include_str!("a729_inline_hook.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("this module has a test module")
            .0
            .to_string()
    }

    #[test]
    fn an_installation_freezes_then_asks_where_threads_are_then_writes() {
        let production = production_code();
        let body = production
            .split_once("unsafe fn install_sites(")
            .expect("install_sites is declared in production")
            .1;
        let freeze = body
            .find("suspend_all_other_threads()?")
            .expect("install_sites must freeze the process");
        let query = body
            .find("any_instruction_pointer_in(&suspended, &target_ranges)?")
            .expect("install_sites must ask where every frozen thread is stopped");
        let write = body
            .find("InstalledHook::install(image_base,")
            .expect("install_sites must still install the hooks");
        assert!(
            freeze < query && query < write,
            "the order must be freeze, then query, then write (freeze at {freeze}, \
             query at {query}, write at {write})"
        );
    }

    /// The publication freeze is released immediately after it is observed.
    #[test]
    fn installing_and_releasing_take_the_shared_process_freeze() {
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { seed_exact_test_image(base) };

        let before = inline_detour::freeze_attempts_on_this_thread();
        let mut installed = unsafe { install_production_semantic(base as usize, &bindings()) }
            .expect("three semantic sites install atomically");
        let after_install = inline_detour::freeze_attempts_on_this_thread();
        // Release the publication freeze before asserting anything else: every
        installed.publish();
        assert_eq!(
            after_install,
            before + 1,
            "an installation must freeze the process exactly once, and hold that \
             freeze until publication"
        );

        let before = inline_detour::freeze_attempts_on_this_thread();
        installed
            .try_uninstall()
            .expect("explicit quiescent teardown restores every hook site");
        assert_eq!(
            inline_detour::freeze_attempts_on_this_thread(),
            before + 1,
            "a teardown that restores original bytes must freeze the process too"
        );
        for site in PRODUCTION_SEMANTIC_SITES {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected
            );
        }
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    #[test]
    fn a_freeze_refusal_is_reported_with_the_shared_vocabulary() {
        let refusal = InstallError::FreezeRefused(inline_detour::FreezeRefusal::Enumeration(1450));
        let rendered = format!("{refusal:?}");
        assert!(rendered.contains("Enumeration"), "{rendered}");
        assert!(rendered.contains("1450"), "{rendered}");
        assert_ne!(
            refusal,
            InstallError::FreezeRefused(inline_detour::FreezeRefusal::ThreadSuspend {
                thread: 7,
                os_error: 5,
            }),
            "a truncated enumeration and an unstoppable thread are different facts"
        );
    }

    #[inline(never)]
    unsafe extern "system" fn dummy() {
        std::hint::black_box(unsafe { GetCurrentProcess() });
    }
    static FIRE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static MATERIALISE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static DESCRIPTION_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static PICTURE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static SPRITE_LOOKUP_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static PLAYER_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static TIMEOUT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static COMMAND_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

    fn bindings() -> Vec<CallbackBinding> {
        [
            (SiteKind::Fire, Some(&FIRE_ORIGINAL)),
            (SiteKind::Materialise, Some(&MATERIALISE_ORIGINAL)),
            (SiteKind::SelectedDescription, Some(&DESCRIPTION_ORIGINAL)),
            (SiteKind::PictureSelector, Some(&PICTURE_ORIGINAL)),
            (SiteKind::SpriteLookup, Some(&SPRITE_LOOKUP_ORIGINAL)),
            (SiteKind::PlayerChoice, Some(&PLAYER_ORIGINAL)),
            (SiteKind::Timeout, Some(&TIMEOUT_ORIGINAL)),
            (SiteKind::OptionCommand, Some(&COMMAND_ORIGINAL)),
            (SiteKind::OptionAdmission, None),
        ]
        .into_iter()
        .map(|(kind, original_slot)| CallbackBinding {
            kind,
            callback: dummy as *const c_void,
            original_slot,
        })
        .collect()
    }

    #[test]
    fn sites_are_instruction_aligned_and_large_enough() {
        assert_eq!(
            PRODUCTION_SEMANTIC_HOOK_PRIMITIVE_ID,
            "fired_event_popups.a729_inline_hook.production_semantic.v1"
        );
        assert_eq!(FULL_EVIDENCE_SITES.len(), 9);
        assert_eq!(
            PRODUCTION_SEMANTIC_SITES
                .iter()
                .map(|site| (site.rva, site.expected.len()))
                .collect::<Vec<_>>(),
            vec![(0x0A028A0, 15), (0x1527B30, 17), (0x1227360, 16)]
        );
        for s in FULL_EVIDENCE_SITES {
            assert!(s.expected.len() >= ABS_JUMP_LEN);
            assert!(s.rva > 0)
        }
    }

    #[test]
    fn absolute_jump_is_register_preserving_rip_indirect_encoding() {
        let destination = 0x0123_4567_89ab_cdefusize as *const c_void;
        let mut bytes = [0u8; ABS_JUMP_LEN];
        unsafe { write_abs_jump(bytes.as_mut_ptr(), destination) };
        assert_eq!(&bytes[..6], &[0xff, 0x25, 0, 0, 0, 0]);
        assert_eq!(
            &bytes[6..],
            &(destination as u64).to_le_bytes(),
            "the destination lives in the RIP-relative literal, not a GPR"
        );
    }

    #[test]
    fn installs_all_sites_and_restores_every_original_byte() {
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { seed_exact_test_image(base) };
        let mut installed = unsafe { install_full_evidence_unshipped(base as usize, &bindings()) }
            .expect("all exact prologues install");
        assert_eq!(installed.len(), FULL_EVIDENCE_SITES.len());
        for site in FULL_EVIDENCE_SITES {
            assert_eq!(
                unsafe { *base.add(site.rva) },
                0xff,
                "site {:?} patched with absolute jump",
                site.kind
            );
        }
        installed
            .try_uninstall()
            .expect("explicit quiescent teardown restores every hook site");
        assert!(installed.is_empty());
        drop(installed);
        for site in FULL_EVIDENCE_SITES {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected
            );
        }
        unsafe {
            VirtualFree(base.cast(), 0, MEM_RELEASE);
        }
    }

    #[test]
    fn production_installs_only_three_semantic_sites_and_reports_exact_outcomes() {
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { seed_exact_test_image(base) };
        let mut installed = unsafe { install_production_semantic(base as usize, &bindings()) }
            .expect("three semantic sites install atomically");
        assert_eq!(
            installed.installed_exact_rva_outcomes(),
            vec![
                InstalledExactRvaOutcome {
                    rva: 0x0A028A0,
                    patch_len: 15,
                },
                InstalledExactRvaOutcome {
                    rva: 0x1227360,
                    patch_len: 16,
                },
                InstalledExactRvaOutcome {
                    rva: 0x1527B30,
                    patch_len: 17,
                },
            ]
        );
        for site in PRODUCTION_SEMANTIC_SITES {
            assert_eq!(unsafe { *base.add(site.rva) }, 0xff);
        }
        for site in [
            MATERIALISE,
            SELECTED_DESCRIPTION,
            PICTURE_SELECTOR,
            SPRITE_LOOKUP,
            PLAYER_CHOICE,
            OPTION_ADMISSION,
        ] {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected,
                "production must not patch UI site {:?}",
                site.kind
            );
        }
        installed.try_uninstall().unwrap();
        for site in PRODUCTION_SEMANTIC_SITES {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected
            );
        }
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    #[test]
    fn production_semantic_mismatch_rolls_back_prior_site() {
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { seed_exact_test_image(base) };
        unsafe { *base.add(OPTION_COMMAND.rva) ^= 0xff };
        let error = match unsafe { install_production_semantic(base as usize, &bindings()) } {
            Ok(_) => panic!("mismatched semantic prologue must reject installation"),
            Err(error) => error,
        };
        assert_eq!(error, InstallError::ByteMismatch(SiteKind::OptionCommand));
        assert_eq!(
            unsafe { std::slice::from_raw_parts(base.add(FIRE.rva), FIRE.expected.len()) },
            FIRE.expected
        );
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    #[test]
    fn production_neither_validates_nor_patches_ui_sites() {
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { seed_exact_test_image(base) };
        let ui_sites = [
            MATERIALISE,
            SELECTED_DESCRIPTION,
            PICTURE_SELECTOR,
            SPRITE_LOOKUP,
            PLAYER_CHOICE,
            OPTION_ADMISSION,
        ];
        for site in ui_sites {
            unsafe { *base.add(site.rva) ^= 0x5a };
        }
        for site in ROLE_PRODUCER_SITES {
            unsafe { *base.add(site.rva) ^= 0xa5 };
        }
        let ui_first_bytes = ui_sites
            .iter()
            .map(|site| unsafe { *base.add(site.rva) })
            .collect::<Vec<_>>();
        let role_first_bytes = ROLE_PRODUCER_SITES
            .iter()
            .map(|site| unsafe { *base.add(site.rva) })
            .collect::<Vec<_>>();
        let mut installed = unsafe { install_production_semantic(base as usize, &bindings()) }
            .expect("UI drift is outside the semantic production roster");
        for (site, byte) in ui_sites.iter().zip(ui_first_bytes) {
            assert_eq!(unsafe { *base.add(site.rva) }, byte);
        }
        for (site, byte) in ROLE_PRODUCER_SITES.iter().zip(role_first_bytes) {
            assert_eq!(unsafe { *base.add(site.rva) }, byte);
        }
        installed.try_uninstall().unwrap();
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    #[test]
    fn mismatch_rolls_back_every_earlier_site() {
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { seed_exact_test_image(base) };
        unsafe { *base.add(PLAYER_CHOICE.rva) ^= 0xff };
        let error = match unsafe { install_full_evidence_unshipped(base as usize, &bindings()) } {
            Ok(_) => panic!("mismatched prologue must reject installation"),
            Err(error) => error,
        };
        assert_eq!(error, InstallError::ByteMismatch(SiteKind::PlayerChoice));
        for site in [FIRE, MATERIALISE] {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected,
                "earlier {:?} patch rolled back",
                site.kind
            );
        }
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }
}

#[cfg(test)]
mod frozen_window_allocation_tests {
    use super::*;

    #[test]
    fn the_allocation_probe_is_this_test_binarys_allocator() {
        assert!(
            freeze_window_audit::the_probe_is_installed(),
            "the #[global_allocator] in lib.rs is not in force, so every \
             allocation count in this module is vacuously zero"
        );
    }

    fn reserved_set(sites: usize, regions: usize) -> InstalledHookSet {
        let mut callback_regions = Vec::with_capacity(regions);
        for index in 0..regions {
            callback_regions.push((0x1000 * (index + 1), 0x1000 * (index + 1) + 0x40));
        }
        let mut hooks = Vec::with_capacity(sites);
        for index in 0..sites {
            hooks.push(InstalledHook {
                site: PRODUCTION_SEMANTIC_SITES[index % PRODUCTION_SEMANTIC_SITES.len()],
                target: (0x10_0000 + index * 0x100) as *mut u8,
                trampoline: ptr::null_mut(),
                original: Vec::new(),
                installed_receipt: None,
                original_page_protection: 0,
                page_protection_restored: false,
                target_patched: false,
            });
        }
        InstalledHookSet {
            install_guard: None,
            publication_suspend: None,
            hooks,
            original_slots: Vec::new(),
            callback_regions,
            released_receipts: Vec::new(),
            execution_ranges: Vec::with_capacity(InstalledHookSet::execution_range_capacity(
                sites, regions,
            )),
        }
    }

    #[test]
    fn refilling_the_execution_ranges_allocates_nothing() {
        let mut set = reserved_set(3, 2);
        let ((), allocations) = freeze_window_audit::observed(|| set.fill_execution_ranges());
        assert_eq!(
            set.execution_ranges.len(),
            3 * 2 + 2,
            "two ranges per hook -- target and trampoline -- plus each region"
        );
        assert_eq!(allocations, 0, "the first refill reached the allocator");

        let ((), allocations) = freeze_window_audit::observed(|| {
            for _ in 0..8 {
                set.fill_execution_ranges();
            }
        });
        assert_eq!(
            set.execution_ranges.len(),
            3 * 2 + 2,
            "clear-then-push, not append"
        );
        assert_eq!(allocations, 0, "a repeated refill reached the allocator");
    }

    #[test]
    fn the_reservation_covers_every_range_the_fill_pushes() {
        for sites in 0..6usize {
            for regions in 0..4usize {
                let mut set = reserved_set(sites, regions);
                let reserved = set.execution_ranges.capacity();
                set.fill_execution_ranges();
                assert!(
                    set.execution_ranges.len() <= reserved,
                    "{sites} sites and {regions} regions filled {} into {reserved}",
                    set.execution_ranges.len()
                );
            }
        }
    }
}
