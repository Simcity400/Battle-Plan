use core::ffi::c_void;
use minhook::MinHook;
use production_hook_receipt::rel5_roster::Rel5HookRoster;
use production_hook_receipt::{
    ExpectedRvaHook, InstallAbortDiscard, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    SiteDetour, MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::gate;
use crate::gate::ProvenMutatorSite;

pub const HOOK_RECEIPT_COUNT: usize = 8;

const RVAS: [u32; HOOK_RECEIPT_COUNT] = [
    0x0035_C330,
    0x0034_7DF0,
    0x00F8_A000,
    0x00F8_B8A0,
    0x00F8_BE70,
    0x00F8_D380,
    0x00F8_DF60,
    0x00F8_FF30,
];

const SENTINELS: [[u8; 16]; HOOK_RECEIPT_COUNT] = [
    [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
        0x55,
    ],
    [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
        0x57,
    ],
    [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0x1f, 0xde,
        0x9d,
    ],
    [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54,
        0x41,
    ],
    [
        0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08,
        0x55,
    ],
    [
        0x4c, 0x89, 0x4c, 0x24, 0x20, 0x44, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08,
        0x55,
    ],
    [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08,
        0x55,
    ],
    [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
        0x57,
    ],
];

const NAMES: [&str; HOOK_RECEIPT_COUNT] = [
    "gameplay.d47.advisor_erase_effect",
    "gameplay.d47.advisor_replace_effect",
    "gameplay.d47.role_tree_destructor",
    "gameplay.d47.advisor_bulk_copy",
    "gameplay.d47.role_tree_merge",
    "gameplay.d47.advisor_insert",
    "gameplay.d47.character_definition_build",
    "gameplay.d47.character_role_parser",
];

pub const PRODUCTION_HOOK_SPECS: [ExpectedRvaHook; HOOK_RECEIPT_COUNT] = [
    spec(0),
    spec(1),
    spec(2),
    spec(3),
    spec(4),
    spec(5),
    spec(6),
    spec(7),
];

const fn spec(index: usize) -> ExpectedRvaHook {
    ExpectedRvaHook {
        logical_name: NAMES[index],
        requested_rva: RVAS[index] as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &SENTINELS[index],
    }
}

static HOOK_RECEIPTS: Rel5HookRoster<HOOK_RECEIPT_COUNT> =
    Rel5HookRoster::new("D47CharacterChangeGate", PRODUCTION_HOOK_SPECS);
static ORIGINALS: [AtomicUsize; HOOK_RECEIPT_COUNT] =
    [const { AtomicUsize::new(0) }; HOOK_RECEIPT_COUNT];
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static RETAINED_LIVE: AtomicBool = AtomicBool::new(false);
static LIFECYCLE: Mutex<()> = Mutex::new(());

type Fn1Void = unsafe extern "system" fn(usize);
type Fn2Void = unsafe extern "system" fn(usize, usize);
type Fn3Void = unsafe extern "system" fn(usize, usize, usize);
type Fn4Void = unsafe extern "system" fn(usize, usize, usize, usize);
type Fn4Ret = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
type Fn3Ret = unsafe extern "system" fn(usize, usize, usize) -> usize;

unsafe extern "system" fn hook_0(a: usize, b: usize) {
    gate::mark_proven_mutator_enter(ProvenMutatorSite::AdvisorEraseEffect);
    let original: Fn2Void = unsafe { mem::transmute(ORIGINALS[0].load(Ordering::Acquire)) };
    unsafe { original(a, b) };
    gate::mark_exit();
}
unsafe extern "system" fn hook_1(a: usize, b: usize) {
    gate::mark_proven_mutator_enter(ProvenMutatorSite::AdvisorReplaceEffect);
    let original: Fn2Void = unsafe { mem::transmute(ORIGINALS[1].load(Ordering::Acquire)) };
    unsafe { original(a, b) };
    gate::mark_exit();
}
unsafe extern "system" fn hook_2(a: usize) {
    gate::mark_proven_mutator_enter(ProvenMutatorSite::RoleTreeDestructor);
    let original: Fn1Void = unsafe { mem::transmute(ORIGINALS[2].load(Ordering::Acquire)) };
    unsafe { original(a) };
    gate::mark_exit();
}
unsafe extern "system" fn hook_3(a: usize, b: usize, c: usize, d: usize) -> usize {
    gate::mark_proven_mutator_enter(ProvenMutatorSite::AdvisorBulkCopy);
    let original: Fn4Ret = unsafe { mem::transmute(ORIGINALS[3].load(Ordering::Acquire)) };
    let returned = unsafe { original(a, b, c, d) };
    gate::mark_exit();
    returned
}
unsafe extern "system" fn hook_4(a: usize, b: usize, c: usize) {
    gate::mark_proven_mutator_enter(ProvenMutatorSite::RoleTreeMerge);
    let original: Fn3Void = unsafe { mem::transmute(ORIGINALS[4].load(Ordering::Acquire)) };
    unsafe { original(a, b, c) };
    gate::mark_exit();
}
unsafe extern "system" fn hook_5(a: usize, b: usize, c: usize, d: usize) {
    gate::mark_proven_mutator_enter(ProvenMutatorSite::AdvisorInsert);
    let original: Fn4Void = unsafe { mem::transmute(ORIGINALS[5].load(Ordering::Acquire)) };
    unsafe { original(a, b, c, d) };
    gate::mark_exit();
}
unsafe extern "system" fn hook_6(a: usize, b: usize, c: usize) -> usize {
    gate::mark_proven_mutator_enter(ProvenMutatorSite::CharacterDefinitionBuild);
    let original: Fn3Ret = unsafe { mem::transmute(ORIGINALS[6].load(Ordering::Acquire)) };
    let returned = unsafe { original(a, b, c) };
    gate::mark_exit();
    returned
}
unsafe extern "system" fn hook_7(a: usize, b: usize, c: usize) {
    gate::mark_proven_mutator_enter(ProvenMutatorSite::CharacterRoleParser);
    let original: Fn3Void = unsafe { mem::transmute(ORIGINALS[7].load(Ordering::Acquire)) };
    unsafe { original(a, b, c) };
    gate::mark_exit();
}

fn detour_addresses() -> [usize; HOOK_RECEIPT_COUNT] {
    [
        hook_0 as *const () as usize,
        hook_1 as *const () as usize,
        hook_2 as *const () as usize,
        hook_3 as *const () as usize,
        hook_4 as *const () as usize,
        hook_5 as *const () as usize,
        hook_6 as *const () as usize,
        hook_7 as *const () as usize,
    ]
}

fn site_detours() -> [SiteDetour; HOOK_RECEIPT_COUNT] {
    let detours = detour_addresses();
    std::array::from_fn(|index| SiteDetour::new(NAMES[index], detours[index]))
}

fn hook_batch(targets: &[usize]) -> Vec<(&'static str, *mut c_void)> {
    targets
        .iter()
        .enumerate()
        .map(|(index, target)| (NAMES[index], *target as *mut c_void))
        .collect()
}

fn read_exact(address: usize, out: &mut [u8]) -> Result<(), String> {
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const _,
            out.as_mut_ptr().cast(),
            out.len(),
            ptr::null_mut(),
        )
    };
    (ok != 0)
        .then_some(())
        .ok_or_else(|| "bytes unreadable".into())
}

pub fn install() -> Result<(), String> {
    let _guard = LIFECYCLE
        .lock()
        .map_err(|_| "d47 character change gate lifecycle poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    if RETAINED_LIVE.load(Ordering::Acquire) {
        return Err("d47 character change gate retained a prior live detour".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("d47 character change gate module unavailable".into());
    }
    let mut targets = [0usize; HOOK_RECEIPT_COUNT];
    for index in 0..HOOK_RECEIPT_COUNT {
        let target = base
            .checked_add(RVAS[index] as usize)
            .ok_or_else(|| format!("d47 hook RVA {:#x} overflows", RVAS[index]))?;
        let mut actual = [0u8; 16];
        read_exact(target, &mut actual)
            .map_err(|error| format!("d47 hook RVA {:#x}: {error}", RVAS[index]))?;
        if actual != SENTINELS[index] {
            return Err(format!(
                "d47 hook sentinel mismatch at RVA {:#x}",
                RVAS[index]
            ));
        }
        targets[index] = target;
    }
    let mut created = Vec::with_capacity(HOOK_RECEIPT_COUNT);
    let detours = detour_addresses();
    for index in 0..HOOK_RECEIPT_COUNT {
        let original = match unsafe {
            MinHook::create_hook(targets[index] as *mut c_void, detours[index] as *mut c_void)
        } {
            Ok(original) => original,
            Err(status) => {
                rollback(&created);
                clear_originals();
                HOOK_RECEIPTS.discard_all_custody();
                return Err(format!(
                    "d47 hook creation failed at {}: {status}",
                    NAMES[index]
                ));
            }
        };
        ORIGINALS[index].store(original as usize, Ordering::Release);
        created.push(targets[index]);
    }
    let enabled = unsafe { inline_detour::enable_batch(&hook_batch(&created)) };
    if let Some((name, status)) = enabled.refusal() {
        if rollback(&created) {
            RETAINED_LIVE.store(true, Ordering::Release);
        } else {
            clear_originals();
            HOOK_RECEIPTS.discard_all_custody();
        }
        return Err(format!("d47 hook {name} enable failed: {status}"));
    }
    MODULE_BASE.store(base, Ordering::Release);
    if let Err(error) = HOOK_RECEIPTS.observe_installed(base, site_detours()) {
        if rollback(&created) {
            RETAINED_LIVE.store(true, Ordering::Release);
        } else {
            clear_originals();
            HOOK_RECEIPTS.discard_all_custody();
        }
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("d47 hook receipt refused: {error}"));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

fn rollback(created: &[usize]) -> bool {
    if created.is_empty() {
        return false;
    }
    let mut batch = hook_batch(created);
    batch.reverse();
    let disabled = unsafe { inline_detour::disable_batch(&batch) };
    if !disabled.all_succeeded() || gate::active_writers() != 0 {
        return true;
    }
    // QUIESCED: disable closed admission; a nonzero writer count retains every trampoline.
    !unsafe { inline_detour::remove_batch(&batch) }.all_succeeded()
}

fn clear_originals() {
    for original in &ORIGINALS {
        original.store(0, Ordering::Release);
    }
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn release_pending() -> bool {
    RETAINED_LIVE.load(Ordering::Acquire)
        || (!installed() && HOOK_RECEIPTS.may_still_hold_installed_custody())
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.installed_receipts()
}

pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String>
{
    HOOK_RECEIPTS.take_released_receipts()
}

pub fn abort_partial_install_and_uninstall() -> Result<InstallAbortDiscard, String> {
    uninstall().map_err(|error| format!("D47CharacterChangeGate install abort: {error}"))?;
    if release_pending() {
        return Err("D47CharacterChangeGate could not prove its detours were removed".into());
    }
    Ok(InstallAbortDiscard::ZERO)
}

pub fn uninstall() -> Result<(), String> {
    let _guard = LIFECYCLE
        .lock()
        .map_err(|_| "d47 character change gate lifecycle poisoned")?;
    if !INSTALLED.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let base = MODULE_BASE.swap(0, Ordering::AcqRel);
    let targets: [usize; HOOK_RECEIPT_COUNT] =
        std::array::from_fn(|index| base.wrapping_add(RVAS[index] as usize));
    let mut batch = hook_batch(&targets);
    batch.reverse();
    let disabled = unsafe { inline_detour::disable_batch(&batch) };
    let mut errors: Vec<String> = disabled
        .failures()
        .into_iter()
        .map(|(name, status)| format!("d47 hook {name} disable failed: {status}"))
        .collect();
    let mut retained = !disabled.all_succeeded();
    if !retained && gate::active_writers() != 0 {
        retained = true;
        errors.push(format!(
            "d47 hook teardown retained trampolines: {} writer callback(s) remain active",
            gate::active_writers()
        ));
    }
    if !retained {
        // QUIESCED: disable closed admission; a nonzero writer count retained every trampoline.
        errors.extend(
            unsafe { inline_detour::remove_batch(&batch) }
                .failures()
                .into_iter()
                .map(|(name, status)| format!("d47 hook {name} removal failed: {status}")),
        );
        retained = !errors.is_empty();
    }
    if retained {
        RETAINED_LIVE.store(true, Ordering::Release);
    } else {
        clear_originals();
        if let Err(error) = HOOK_RECEIPTS.publish_release(true) {
            errors.push(format!("d47 hook release receipt refused: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    static ORIGINAL_MUTATED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn mutating_original(_a: usize, _b: usize) {
        ORIGINAL_MUTATED.store(true, Ordering::Release);
    }

    #[test]
    fn a_proven_advisor_wrapper_signals_around_the_original_mutation() {
        ORIGINAL_MUTATED.store(false, Ordering::Release);
        let previous = ORIGINALS[1].swap(mutating_original as *const () as usize, Ordering::AcqRel);
        let baseline = gate::witness();
        unsafe { hook_1(1, 2) };
        ORIGINALS[1].store(previous, Ordering::Release);

        assert!(ORIGINAL_MUTATED.load(Ordering::Acquire));
        assert_eq!(gate::active_writers(), 0);
        assert_eq!(
            gate::open_hour(true, Some(baseline)),
            Err(crate::gate::WalkReason::Mutated)
        );
    }
}
