//! Exact-a729 A40 selected-designer passive observer.
//!
//! The production collector installs this hook only when domain 85 is
//! scheduled. All process pointers are consumed in the callback and reduced
//! to `(object_type, stable_id)` before entering shared state. Missing, torn or
//! incomplete observations are reported as typed unavailability by the
//! collector and are never replayed as an empty designer catalog.

use super::{
    collect_fixture_and_encode, ExactSelectedDesignerIdentitySource,
    SelectedDesignerFamily, SelectedDesignerIdentityFrame, SelectedDesignerOwner,
    SelectedDesignerError, StableDesignIdentity, CLASSIFIER_MANIFEST_SHA256,
    DIVISION_CLICK_THUNK_ARTIFACT_SHA256, DIVISION_WRAPPER_ARTIFACT_SHA256,
    EQUIPMENT_VARIANT_TYPE, EXACT_A729_EXECUTABLE_SHA256, MAX_ORDERED_DESIGNS,
    PRODUCER_MANIFEST_SHA256, REFERENCED_DIVISION_TEMPLATE_TYPE,
};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::rel5_roster::Rel5HookRoster;
use production_hook_receipt::{
    ExpectedRvaHook, InstalledRvaHookReceipt, ReleasedRvaHookReceipt, SiteDetour,
    MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const EQUIPMENT_SELECTION_RVA: u32 = 0x0177_15A0;
pub const EQUIPMENT_AUTODESIGN_SUCCESS_RVA: u32 = 0x0147_DDD0;
pub const DIVISION_ORDER_RVA: u32 = 0x0175_6C40;
pub const DIVISION_SELECTION_RVA: u32 = 0x0167_AEE0;
pub const VARIANT_UPGRADE_SELECTION_RVA: u32 = 0x01F5_D420;
pub const PLANE_CLASSIFIER_RVA: u32 = 0x00C8_7CE0;
pub const TANK_CLASSIFIER_RVA: u32 = 0x00C8_7EB0;
pub const SHIP_CLASSIFIER_RVA: u32 = 0x00C8_8440;
pub const EQUIPMENT_CLASSIFIER_RVA: u32 = 0x00C8_84B0;

const EQUIPMENT_SELECTION_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xD2, 0x0F, 0x84, 0x14, 0x03, 0x00, 0x00, 0x48, 0x89, 0x5C, 0x24, 0x08,
    0x55, 0x56,
];
const EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL: [u8; 16] = [
    0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4C, 0x24,
    0x08, 0x55,
];
pub const EQUIPMENT_AUTODESIGN_SUCCESS_MANIFEST_SHA256: &str =
    "44bb3647f7285c8b961aca7ba0d702289c1145ddf983421369bf9abdc90e0ce5";
pub const EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL_ARTIFACT_SHA256: &str =
    "618865df111fb54b0a6d6773b1832a5696c5c916a9e66dbe4c00aba186a5be80";
const DIVISION_ORDER_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
    0x83, 0xEC,
];
const DIVISION_SELECTION_SENTINEL: [u8; 16] = [
    0x4C, 0x8B, 0xC2, 0xC7, 0x01, 0x02, 0x00, 0x00, 0x00, 0x48, 0x83, 0xC2, 0x18, 0xE9,
    0x7E, 0x12,
];
const VARIANT_UPGRADE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x54,
    0x41, 0x56,
];

/// The five selected-designer sites this module patches. D85's install unit
/// also owns the d78 designer-refresh prerequisite in the parent module, so the
/// group's roster is these five plus that one.
pub const HOOK_RECEIPT_COUNT: usize = 5;

/// The exact production-hook specifications this module may install, in the
/// same order as the `specs`/`hooks` tables `install_for_fixture` walks and as
/// the detour list `observe_installed` is handed.
///
/// Each `original_prefix` is the SAME constant the installer's own
/// `verify_sentinel` loop reads, so the byte gate and the receipt cannot end up
/// describing different functions.
pub const PRODUCTION_HOOK_SPECS: [ExpectedRvaHook; HOOK_RECEIPT_COUNT] = [
    ExpectedRvaHook {
        logical_name: "gameplay.d85.equipment_selection",
        requested_rva: EQUIPMENT_SELECTION_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &EQUIPMENT_SELECTION_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d85.equipment_autodesign_success",
        requested_rva: EQUIPMENT_AUTODESIGN_SUCCESS_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d85.division_order",
        requested_rva: DIVISION_ORDER_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &DIVISION_ORDER_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d85.division_selection",
        requested_rva: DIVISION_SELECTION_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &DIVISION_SELECTION_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d85.variant_upgrade_selection",
        requested_rva: VARIANT_UPGRADE_SELECTION_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &VARIANT_UPGRADE_SENTINEL,
    },
];

static HOOK_RECEIPTS: Rel5HookRoster<HOOK_RECEIPT_COUNT> =
    Rel5HookRoster::new("D85", PRODUCTION_HOOK_SPECS);

/// The detour addresses, in `PRODUCTION_HOOK_SPECS` order. A function rather
/// than a `const` because a function item's address is not an integer at
/// compile time.
fn detours() -> [SiteDetour; HOOK_RECEIPT_COUNT] {
    [
        SiteDetour::new(
            "gameplay.d85.equipment_selection",
            hook_equipment_selection as *const () as usize,
        ),
        SiteDetour::new(
            "gameplay.d85.equipment_autodesign_success",
            hook_equipment_autodesign_success as *const () as usize,
        ),
        SiteDetour::new(
            "gameplay.d85.division_order",
            hook_division_order as *const () as usize,
        ),
        SiteDetour::new(
            "gameplay.d85.division_selection",
            hook_division_selection as *const () as usize,
        ),
        SiteDetour::new(
            "gameplay.d85.variant_upgrade_selection",
            hook_variant_upgrade_selection as *const () as usize,
        ),
    ]
}

const VIEW_MODE: usize = 0x3790;
const DIVISION_HANDLER: usize = 0x2B58;
const DIVISION_ROW_POOL_DATA: usize = 0x2BB0;
const DIVISION_ROW_POOL_USED: usize = 0x2BBC;
const DIVISION_ROW_TEMPLATE: usize = 0x528;
const DIVISION_HANDLER_SELECTED: usize = 0x8D0;
const VARIANT_UPGRADE_NESTED_VIEW: usize = 0x1BB0;
const VARIANT_UPGRADE_NESTED_SELECTED: usize = 0xA48;
const VARIANT_UPGRADE_WINDOW_SELECTED: usize = 0x1CE0;
const VARIANT_DEFINITION: usize = 0x3F0;
const OBJECT_TYPE: usize = 0x08;
const STABLE_ID: usize = 0x0C;
const DIVISION_TEMPLATE_VTABLE_RVA: usize = 0x0293_8F70;
const EQUIPMENT_VARIANT_VTABLE_RVA: usize = 0x0293_AFB8;

type TwoArgFn = unsafe extern "system" fn(usize, usize);
type OneArgFn = unsafe extern "system" fn(usize);
type PredicateFn = unsafe extern "system" fn(usize) -> u8;
type ThreeArgReturnFn = unsafe extern "system" fn(usize, usize, u8) -> u64;

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_EQUIPMENT_SELECTION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_EQUIPMENT_AUTODESIGN_SUCCESS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DIVISION_ORDER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DIVISION_SELECTION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_VARIANT_UPGRADE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());
/// A failed installation could not PROVE it removed its detours.
///
/// `INSTALLED` is set only after the WHOLE roster is enabled, so an enable
/// failure whose rollback also failed leaves every other predicate answering
/// "nothing here" over a live patch. One-way, like D94's twin: there is no
/// later state in which a patch this module could not remove becomes provably
/// gone.
static FAILED_INSTALL_REMOVAL_UNPROVED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
    static IN_EQUIPMENT_SELECTION: Cell<bool> = const { Cell::new(false) };
    static EQUIPMENT_AUTODESIGN_SUCCEEDED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Debug)]
struct ObservedSelection {
    transition_sequence: u64,
    source_record_identity: String,
    owner: SelectedDesignerOwner,
    family: SelectedDesignerFamily,
    ordered_designs: Vec<StableDesignIdentity>,
    selected_design: StableDesignIdentity,
}

#[derive(Debug, Default)]
struct HookState {
    next_transition_sequence: u64,
    latest: BTreeMap<SelectedDesignerFamily, ObservedSelection>,
    division_views_by_handler: BTreeMap<usize, usize>,
    callback_count: u64,
    variant_upgrade_selection_count: u64,
    current_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub equipment_selection: usize,
    pub equipment_autodesign_success: usize,
    pub division_order: usize,
    pub division_selection: usize,
    pub variant_upgrade_selection: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub callback_count: u64,
    pub variant_upgrade_selection_count: u64,
    pub available_families: Vec<SelectedDesignerFamily>,
    pub current_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExactA729SelectedDesignerSource;

impl ExactSelectedDesignerIdentitySource for ExactA729SelectedDesignerSource {
    fn is_exact_a729_selected_designer_source(&self) -> bool {
        INSTALLED.load(Ordering::Acquire)
    }

    fn producer_manifest_sha256(&self) -> [u8; 32] {
        PRODUCER_MANIFEST_SHA256
    }

    fn classifier_manifest_sha256(&self) -> [u8; 32] {
        CLASSIFIER_MANIFEST_SHA256
    }

    fn collect_frame(
        &self,
        frame_hour: u64,
        family: SelectedDesignerFamily,
    ) -> Result<SelectedDesignerIdentityFrame, String> {
        let state = STATE
            .get()
            .ok_or("A40 hook state is not initialized")?
            .lock()
            .map_err(|_| "A40 hook state mutex poisoned")?;
        let observed = match state.latest.get(&family) {
            Some(observed) => observed,
            None => {
                return Err(state.current_error.clone().unwrap_or_else(|| {
                    format!("A40 has not observed a complete {family:?} selection")
                }))
            }
        };
        Ok(frame_from_observed(frame_hour, observed.clone()))
    }
}

pub fn install_for_fixture() -> Result<InstalledHookAddresses, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "A40 hook lifecycle mutex poisoned".to_string())?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(installed_addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    if MODULE_BASE.load(Ordering::Acquire) != 0 {
        return Err("A40 prior hook set has not completed quiescent removal".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let specs = [
        (EQUIPMENT_SELECTION_RVA, &EQUIPMENT_SELECTION_SENTINEL[..]),
        (
            EQUIPMENT_AUTODESIGN_SUCCESS_RVA,
            &EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL[..],
        ),
        (DIVISION_ORDER_RVA, &DIVISION_ORDER_SENTINEL[..]),
        (DIVISION_SELECTION_RVA, &DIVISION_SELECTION_SENTINEL[..]),
        (VARIANT_UPGRADE_SELECTION_RVA, &VARIANT_UPGRADE_SENTINEL[..]),
    ];
    for (rva, sentinel) in specs {
        verify_sentinel(base + rva as usize, sentinel)?;
    }
    let state = STATE.get_or_init(|| Mutex::new(HookState::default()));
    *state
        .lock()
        .map_err(|_| "A40 hook state mutex poisoned")? = HookState::default();
    // Publish the pinned base before enabling the first detour: another game
    // thread may enter a newly enabled callback immediately.
    MODULE_BASE.store(base, Ordering::Release);
    let addresses = installed_addresses(base);
    let hooks = [
        (
            addresses.equipment_selection,
            hook_equipment_selection as *mut c_void,
            &ORIGINAL_EQUIPMENT_SELECTION,
        ),
        (
            addresses.equipment_autodesign_success,
            hook_equipment_autodesign_success as *mut c_void,
            &ORIGINAL_EQUIPMENT_AUTODESIGN_SUCCESS,
        ),
        (
            addresses.division_order,
            hook_division_order as *mut c_void,
            &ORIGINAL_DIVISION_ORDER,
        ),
        (
            addresses.division_selection,
            hook_division_selection as *mut c_void,
            &ORIGINAL_DIVISION_SELECTION,
        ),
        (
            addresses.variant_upgrade_selection,
            hook_variant_upgrade_selection as *mut c_void,
            &ORIGINAL_VARIANT_UPGRADE,
        ),
    ];
    let mut created = Vec::new();
    for (target, detour, original_slot) in hooks {
        let original = match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(original) => original,
            Err(status) => {
                let mut cleanup_failures = Vec::new();
                for rollback in created.iter().rev() {
                    match unsafe { inline_detour::remove(*rollback as *mut c_void) } {
                        Ok(())
                        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                        Err(error) => cleanup_failures.push(format!(
                            "remove {rollback:#x}: {error}"
                        )),
                    }
                }
                if !cleanup_failures.is_empty() {
                    return Err(format!(
                        "A40 hook creation at {target:#x} failed: {status}; rollback failed: {}",
                        cleanup_failures.join(", ")
                    ));
                }
                clear_originals();
                MODULE_BASE.store(0, Ordering::Release);
                return Err(format!(
                    "A40 hook creation at {target:#x} failed: {status}"
                ));
            }
        };
        original_slot.store(original as usize, Ordering::Release);
        created.push(target);
    }
    for target in &created {
        if let Err(status) = unsafe { inline_detour::enable(*target as *mut c_void) } {
            let mut cleanup_failures = Vec::new();
            for rollback in created.iter().rev() {
                match unsafe { inline_detour::disable(*rollback as *mut c_void) } {
                    Ok(())
                    | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
                    | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                    Err(error) => cleanup_failures.push(format!(
                        "disable {rollback:#x}: {error}"
                    )),
                }
            }
            if cleanup_failures.is_empty() {
                if let Err(error) = wait_for_quiescence() {
                    cleanup_failures.push(error);
                }
            }
            if cleanup_failures.is_empty() {
                for rollback in created.iter().rev() {
                    match unsafe { inline_detour::remove(*rollback as *mut c_void) } {
                        Ok(())
                        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                        Err(error) => cleanup_failures.push(format!(
                            "remove {rollback:#x}: {error}"
                        )),
                    }
                }
            }
            if !cleanup_failures.is_empty() {
                // The enable failed and the rollback could not finish, so a
                // target may still be patched with `INSTALLED` never set. Latch
                // it: this is the only remaining record.
                FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
                return Err(format!(
                    "A40 hook enable at {target:#x} failed: {status}; rollback failed: {}",
                    cleanup_failures.join(", ")
                ));
            }
            clear_originals();
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("A40 hook enable at {target:#x} failed: {status}"));
        }
    }
    // Fail closed on identity, exactly as D94 does. The observation re-derives
    // the module from the running process, refuses anything that is not an
    // exact-a729 `hoi4.exe`, re-checks each admitted original prefix against
    // that file's own bytes at the site's RVA, and binds each installed relay
    // to the detour this module actually passed to MinHook. An installation
    // that cannot be attested is rolled back rather than shipped.
    if let Err(error) = HOOK_RECEIPTS.observe_installed(base, detours()) {
        let cleanup = release_created_sites(&created);
        if !cleanup.is_empty() {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A40 selected-designer hook receipt refused: {error}; rollback failed: {}",
                cleanup.join(", ")
            ));
        }
        // Only once the detours are provably gone. Discarding custody while a
        // site is still patched would erase the one record that a patch exists.
        HOOK_RECEIPTS.discard_all_custody();
        clear_originals();
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!(
            "A40 selected-designer hook receipt refused: {error}"
        ));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(addresses)
}

/// Disable, quiesce and remove every site this install created, in reverse
/// order. Returns the failures; an empty result means every target's original
/// bytes are back and nothing is patched.
///
/// The ordering matches the enable-failure arm above and is load-bearing: the
/// quiescence wait only terminates once nothing is admitting callbacks, so a
/// disable that failed must stop the sequence rather than let the wait spin
/// against a live detour.
fn release_created_sites(created: &[usize]) -> Vec<String> {
    let mut failures = Vec::new();
    for target in created.iter().rev() {
        match unsafe { inline_detour::disable(*target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(error) => failures.push(format!("disable {target:#x}: {error}")),
        }
    }
    if !failures.is_empty() {
        return failures;
    }
    if let Err(error) = wait_for_quiescence() {
        failures.push(error);
        return failures;
    }
    for target in created.iter().rev() {
        match unsafe { inline_detour::remove(*target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(error) => failures.push(format!("remove {target:#x}: {error}")),
        }
    }
    failures
}

/// True once this module owns enabled, receipt-observed production hooks.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// The exact enabled-byte receipts for the five selected-designer sites.
pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.installed_receipts()
}

/// Consume the release receipts. One-shot: a second call after the registry has
/// taken custody reports them unavailable rather than minting a second release.
pub fn take_released_hook_receipts(
) -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.take_released_receipts()
}

/// True while this module holds a release it could not PROVE.
///
/// Two independent ways that happens, and `installed()` reports false for both:
/// a failed install whose rollback could not finish
/// ([`FAILED_INSTALL_REMOVAL_UNPROVED`]), and receipt custody still on the
/// installed side -- `uninstall_fixture_hooks` clears `INSTALLED` as soon as the
/// disables succeed but publishes the release only after every removal also
/// came back clean.
pub fn release_pending() -> bool {
    FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire)
        || (!installed() && HOOK_RECEIPTS.may_still_hold_installed_custody())
}

/// Normal production installer. It shares the exact sentinel-gated hook owner
/// with the optional fixture API; no fixture receipt or client-driving action
/// is consulted.
pub fn install() -> Result<InstalledHookAddresses, String> {
    if !super::PRODUCTION_ACTIVATION {
        return Err("A40 selected-designer production authority is not bound".into());
    }
    install_for_fixture()
}

pub fn uninstall_fixture_hooks() -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "A40 hook lifecycle mutex poisoned".to_string())?;
    let needs_disable = INSTALLED.load(Ordering::Acquire);
    if !needs_disable && MODULE_BASE.load(Ordering::Acquire) == 0 {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let addresses = installed_addresses(base);
    let targets = [
        addresses.variant_upgrade_selection,
        addresses.division_selection,
        addresses.division_order,
        addresses.equipment_autodesign_success,
        addresses.equipment_selection,
    ];
    let mut first_error = None;
    if needs_disable {
        for target in &targets {
            match unsafe { inline_detour::disable(*target as *mut c_void) } {
                Ok(())
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                Err(status) => {
                    first_error.get_or_insert_with(|| format!("A40 hook disable at {target:#x} failed: {status}"));
                }
            }
        }
        if let Some(error) = first_error.as_ref() {
            return Err(error.clone());
        }
        INSTALLED.store(false, Ordering::Release);
    }
    wait_for_quiescence()?;
    for target in targets {
        match unsafe { inline_detour::remove(target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                first_error.get_or_insert_with(|| format!("A40 hook removal at {target:#x} failed: {status}"));
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    // Mint the release receipts only for a teardown that provably restored
    // every target. Reaching here means every disable AND every removal
    // returned clean; `observe_released_set` then independently re-reads each
    // site's bytes and refuses unless the admitted original prefix is back.
    //
    // `relay_freed: true` is the truth of this path: it calls
    // `inline_detour::remove`, which is MinHook's `MH_RemoveHook` and frees the
    // trampoline.
    //
    // A module that never observed receipts -- a fixture install predating this
    // custody, or a second uninstall after the registry already took the
    // receipts -- has nothing to release, and that is not an error.
    if HOOK_RECEIPTS.has_complete_installed_custody() {
        if let Err(error) = HOOK_RECEIPTS.publish_release(true) {
            return Err(format!(
                "A40 selected-designer hook release receipt refused: {error}"
            ));
        }
    }
    clear_originals();
    MODULE_BASE.store(0, Ordering::Release);
    if let Some(state) = STATE.get() {
        *state
            .lock()
            .map_err(|_| "A40 hook state mutex poisoned")? = HookState::default();
    }
    Ok(())
}

/// Normal production teardown. Quiescence remains mandatory before the DLL
/// can unload.
pub fn uninstall() -> Result<(), String> {
    uninstall_fixture_hooks()
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics::default();
    };
    HookDiagnostics {
        callback_count: state.callback_count,
        variant_upgrade_selection_count: state.variant_upgrade_selection_count,
        available_families: state.latest.keys().copied().collect(),
        current_error: state.current_error.clone(),
    }
}

pub fn encode_fixture_latest(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    family: SelectedDesignerFamily,
) -> Result<Vec<u8>, SelectedDesignerError> {
    collect_fixture_and_encode(
        &ExactA729SelectedDesignerSource,
        frame_hour,
        elapsed_wall_ms,
        family,
    )
}

/// Encodes the most recently committed selection across all six families.
/// This is a passive current-observation read: if no complete callback has
/// committed, or the newest callback failed validation, the call fails and the
/// collector emits a typed source-unavailable SDI1 frame for that hour.
pub fn encode_latest(
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, SelectedDesignerError> {
    // Two structurally different answers, deliberately not one. The combined
    // `||` published `ActivationBlocked` for both, and because
    // `PRODUCTION_ACTIVATION` is a compile-time `true` on this build, the
    // reason d85 actually carries on every capture -- "the producer detour was
    // never patched in" -- was reported as "the lane is switched off". Naming
    // the installation state is what makes d85's failure the same actionable
    // sentence d79 and d95 already publish.
    if !super::PRODUCTION_ACTIVATION {
        return Err(SelectedDesignerError::ActivationBlocked);
    }
    if !INSTALLED.load(Ordering::Acquire) {
        return Err(SelectedDesignerError::HookNotInstalled);
    }
    let observed = {
        let state = STATE
            .get()
            .ok_or_else(|| SelectedDesignerError::Source("A40 hook state is not initialized".into()))?
            .lock()
            .map_err(|_| SelectedDesignerError::Source("A40 hook state mutex poisoned".into()))?;
        if let Some(error) = state.current_error.as_ref() {
            return Err(SelectedDesignerError::Source(error.clone()));
        }
        state
            .latest
            .values()
            .max_by_key(|observed| observed.transition_sequence)
            .cloned()
            .ok_or_else(|| {
                SelectedDesignerError::Source(
                    "A40 has not observed a complete selected-designer transition".into(),
                )
            })?
    };
    let snapshot = frame_from_observed(frame_hour, observed);
    super::encode_payload(&super::SelectedDesignerTransportFrame {
        elapsed_wall_ms,
        status_flags: 0,
        frame_hour,
        snapshot: Some(snapshot),
    })
}

/// Clear process-local selections at an authenticated campaign boundary while
/// retaining the exact detours. The next frame remains typed unavailable until
/// a new campaign selection callback commits a complete observation.
pub fn reset_campaign_observations() -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("A40 selected-designer hook is not installed".into());
    }
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "A40 hook state mutex poisoned".to_string())?;
    clear_campaign_observations(&mut state);
    Ok(())
}

fn clear_campaign_observations(state: &mut HookState) {
    *state = HookState::default();
}

/// Destructively drain one hook-observed family into SDI1 evidence bytes. The
/// caller chooses only the capture hour/family; stable object identity and
/// controller order are removed from lane-owned hook state and cannot be
/// supplied through this API.
pub fn drain_fixture_payload(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    family: SelectedDesignerFamily,
) -> Result<Vec<u8>, SelectedDesignerError> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err(SelectedDesignerError::UnauthenticatedSource);
    }
    let observed = {
        let mut state = STATE
            .get()
            .ok_or_else(|| SelectedDesignerError::Source("A40 hook state is not initialized".into()))?
            .lock()
            .map_err(|_| SelectedDesignerError::Source("A40 hook state mutex poisoned".into()))?;
        match state.latest.remove(&family) {
            Some(observed) => observed,
            None => {
                return Err(SelectedDesignerError::Source(
                    state.current_error.clone().unwrap_or_else(|| {
                        format!("A40 has no undrained {family:?} selection")
                    }),
                ))
            }
        }
    };
    let snapshot = frame_from_observed(frame_hour, observed);
    super::encode_payload(&super::SelectedDesignerTransportFrame {
        elapsed_wall_ms,
        status_flags: 0,
        frame_hour,
        snapshot: Some(snapshot),
    })
}

fn frame_from_observed(
    frame_hour: u64,
    observed: ObservedSelection,
) -> SelectedDesignerIdentityFrame {
    SelectedDesignerIdentityFrame {
        frame_hour,
        transition_sequence: observed.transition_sequence,
        executable_sha256: EXACT_A729_EXECUTABLE_SHA256,
        producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
        classifier_manifest_sha256: CLASSIFIER_MANIFEST_SHA256,
        division_click_thunk_artifact_sha256: DIVISION_CLICK_THUNK_ARTIFACT_SHA256,
        division_wrapper_artifact_sha256: DIVISION_WRAPPER_ARTIFACT_SHA256,
        source_record_identity: observed.source_record_identity,
        owner: observed.owner,
        family: observed.family,
        expected_ordered_design_count: observed.ordered_designs.len() as u32,
        ordered_designs: observed.ordered_designs,
        selected_design: observed.selected_design,
        complete: true,
    }
}

fn installed_addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        equipment_selection: base + EQUIPMENT_SELECTION_RVA as usize,
        equipment_autodesign_success:
            base + EQUIPMENT_AUTODESIGN_SUCCESS_RVA as usize,
        division_order: base + DIVISION_ORDER_RVA as usize,
        division_selection: base + DIVISION_SELECTION_RVA as usize,
        variant_upgrade_selection: base + VARIANT_UPGRADE_SELECTION_RVA as usize,
    }
}

fn clear_originals() {
    ORIGINAL_EQUIPMENT_SELECTION.store(0, Ordering::Release);
    ORIGINAL_EQUIPMENT_AUTODESIGN_SUCCESS.store(0, Ordering::Release);
    ORIGINAL_DIVISION_ORDER.store(0, Ordering::Release);
    ORIGINAL_DIVISION_SELECTION.store(0, Ordering::Release);
    ORIGINAL_VARIANT_UPGRADE.store(0, Ordering::Release);
}

struct CallbackGuard;

impl CallbackGuard {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn wait_for_quiescence() -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) != 0 {
        if std::time::Instant::now() >= deadline {
            return Err("A40 hook callbacks did not quiesce after disable".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

unsafe extern "system" fn hook_equipment_selection(view: usize, variant: usize) {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    if !nested {
        EQUIPMENT_AUTODESIGN_SUCCEEDED.with(|value| value.set(false));
        IN_EQUIPMENT_SELECTION.with(|value| value.set(true));
    }
    call_two_arg_original(&ORIGINAL_EQUIPMENT_SELECTION, view, variant);
    if !nested {
        IN_EQUIPMENT_SELECTION.with(|value| value.set(false));
        if EQUIPMENT_AUTODESIGN_SUCCEEDED.with(Cell::get) {
            publish_result(capture_equipment_selection(view, variant));
        } else {
            publish_error(
                "A40 equipment selection did not cross the exact successful auto-design boundary",
            );
        }
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_equipment_autodesign_success(
    designer_state: usize,
    variant: usize,
    automatic: u8,
) -> u64 {
    let _callback = CallbackGuard::enter();
    let original = ORIGINAL_EQUIPMENT_AUTODESIGN_SUCCESS.load(Ordering::Acquire);
    let result = if original == 0 {
        0
    } else {
        let original: ThreeArgReturnFn = unsafe { mem::transmute(original) };
        unsafe { original(designer_state, variant, automatic) }
    };
    if automatic != 0 && result != 0 && IN_EQUIPMENT_SELECTION.with(Cell::get) {
        EQUIPMENT_AUTODESIGN_SUCCEEDED.with(|value| value.set(true));
    }
    result
}

unsafe extern "system" fn hook_division_order(view: usize) {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    call_one_arg_original(&ORIGINAL_DIVISION_ORDER, view);
    if !nested {
        publish_result(capture_division_selection(view));
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_division_selection(handler: usize, template: usize) {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    call_two_arg_original(&ORIGINAL_DIVISION_SELECTION, handler, template);
    if !nested {
        let view = STATE
            .get()
            .and_then(|state| state.lock().ok())
            .and_then(|state| state.division_views_by_handler.get(&handler).copied());
        match view {
            Some(view)
                if read_usize(handler + DIVISION_HANDLER_SELECTED) == Some(template) =>
            {
                publish_result(capture_division_selection(view))
            }
            Some(_) => publish_error(
                "A40 division selection transaction did not commit the requested template",
            ),
            None => publish_error("A40 division selection has no authenticated view/handler join"),
        }
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_variant_upgrade_selection(window: usize, variant: usize) {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    call_two_arg_original(&ORIGINAL_VARIANT_UPGRADE, window, variant);
    if !nested {
        let result = capture_variant_upgrade_selection(window, variant);
        if result.is_ok() {
            if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
                state.variant_upgrade_selection_count =
                    state.variant_upgrade_selection_count.saturating_add(1);
            }
        }
        publish_result(result);
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe fn call_two_arg_original(slot: &AtomicUsize, a: usize, b: usize) {
    let original = slot.load(Ordering::Acquire);
    if original != 0 {
        let original: TwoArgFn = mem::transmute(original);
        original(a, b);
    }
}

unsafe fn call_one_arg_original(slot: &AtomicUsize, a: usize) {
    let original = slot.load(Ordering::Acquire);
    if original != 0 {
        let original: OneArgFn = mem::transmute(original);
        original(a);
    }
}

fn capture_equipment_selection(
    view: usize,
    variant: usize,
) -> Result<ObservedSelection, String> {
    let base = module_base()?;
    let designer = crate::a729_hook::latest_active_role_root()?
        .ok_or("A40 equipment selection has no active d78 controller snapshot")?;
    if designer.view_ptr != view {
        return Err("A40 equipment selection and d78 controller view differ".into());
    }
    let selected_design = read_identity(
        variant,
        EQUIPMENT_VARIANT_VTABLE_RVA,
        EQUIPMENT_VARIANT_TYPE,
    )?;
    let mode = read_i32(view + VIEW_MODE).ok_or("A40 equipment view mode unreadable")?;
    let definition = read_usize(variant + VARIANT_DEFINITION)
        .filter(|pointer| *pointer != 0)
        .ok_or("A40 selected variant definition is null/unreadable")?;
    let (owner, family) = classify_variant(base, mode, definition)?;
    let ordered_designs = designer
        .context
        .ordered_design_ids
        .iter()
        .map(|stable_id| StableDesignIdentity {
            object_type: EQUIPMENT_VARIANT_TYPE,
            stable_id: *stable_id,
        })
        .collect::<Vec<_>>();
    if !ordered_designs.contains(&selected_design) {
        return Err("A40 selected variant is absent from the exact d78 order".into());
    }
    Ok(ObservedSelection {
        transition_sequence: 0,
        source_record_identity: format!(
            "a40:a729:equipment_selection_rva_{EQUIPMENT_SELECTION_RVA:08X}:d78_transition={}",
            designer.context.transition_sequence
        ),
        owner,
        family,
        ordered_designs,
        selected_design,
    })
}

fn capture_division_selection(view: usize) -> Result<ObservedSelection, String> {
    module_base()?;
    if view == 0 {
        return Err("A40 division order supplied null view".into());
    }
    let handler = read_usize(view + DIVISION_HANDLER)
        .filter(|pointer| *pointer != 0)
        .ok_or("A40 division handler is null/unreadable")?;
    let used = read_u32(view + DIVISION_ROW_POOL_USED)
        .ok_or("A40 division row-pool used count unreadable")? as usize;
    if used == 0 || used > MAX_ORDERED_DESIGNS {
        return Err("A40 division row-pool used count is empty/out of bounds".into());
    }
    let data = read_usize(view + DIVISION_ROW_POOL_DATA)
        .filter(|pointer| *pointer != 0)
        .ok_or("A40 division row-pool data is null/unreadable")?;
    let mut ordered_designs = Vec::with_capacity(used);
    for index in 0..used {
        let row = read_usize(data + index * mem::size_of::<usize>())
            .filter(|pointer| *pointer != 0)
            .ok_or_else(|| format!("A40 division row {index} is null/unreadable"))?;
        let template = read_usize(row + DIVISION_ROW_TEMPLATE)
            .filter(|pointer| *pointer != 0)
            .ok_or_else(|| format!("A40 division row {index} template is null/unreadable"))?;
        ordered_designs.push(read_identity(
            template,
            DIVISION_TEMPLATE_VTABLE_RVA,
            REFERENCED_DIVISION_TEMPLATE_TYPE,
        )?);
    }
    let selected = read_usize(handler + DIVISION_HANDLER_SELECTED)
        .filter(|pointer| *pointer != 0)
        .ok_or("A40 division selected template is null/unreadable")?;
    let selected_design = read_identity(
        selected,
        DIVISION_TEMPLATE_VTABLE_RVA,
        REFERENCED_DIVISION_TEMPLATE_TYPE,
    )?;
    if read_usize(view + DIVISION_ROW_POOL_DATA) != Some(data)
        || read_u32(view + DIVISION_ROW_POOL_USED) != Some(used as u32)
        || read_usize(handler + DIVISION_HANDLER_SELECTED) != Some(selected)
    {
        return Err("A40 division order/selection changed during capture".into());
    }
    if !ordered_designs.contains(&selected_design) {
        return Err("A40 selected division template is absent from controller order".into());
    }
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.division_views_by_handler.insert(handler, view);
    }
    Ok(ObservedSelection {
        transition_sequence: 0,
        source_record_identity: format!(
            "a40:a729:division_order_rva_{DIVISION_ORDER_RVA:08X}:rows={used}"
        ),
        owner: SelectedDesignerOwner::CountryDivisionDesigner,
        family: SelectedDesignerFamily::Division,
        ordered_designs,
        selected_design,
    })
}

fn capture_variant_upgrade_selection(
    window: usize,
    variant: usize,
) -> Result<ObservedSelection, String> {
    if window == 0 {
        return Err("A40 variant-upgrade selection supplied null window".into());
    }
    let nested_view = read_usize(window + VARIANT_UPGRADE_NESTED_VIEW)
        .filter(|pointer| *pointer != 0)
        .ok_or("A40 variant-upgrade nested view is null/unreadable")?;
    if read_usize(nested_view + VARIANT_UPGRADE_NESTED_SELECTED) != Some(variant)
        || read_usize(window + VARIANT_UPGRADE_WINDOW_SELECTED) != Some(variant)
    {
        return Err(
            "A40 variant-upgrade transaction did not commit one identical source variant"
                .into(),
        );
    }
    let selected_design = read_identity(
        variant,
        EQUIPMENT_VARIANT_VTABLE_RVA,
        EQUIPMENT_VARIANT_TYPE,
    )?;
    // CEquipmentUpgradeDesignerWindow is opened for exactly one source
    // variant. 0x141F5D420 forwards that RDX unchanged to 0x141F5CF50,
    // which commits it at nested +0xA48, and mirrors it at window +0x1CE0.
    // Its GUI grids are module/stat controls, not a second variant catalog.
    // Thus the controller-published design order for this view is exactly the
    // one input element, not an inferred slice of the global d59 catalog.
    Ok(ObservedSelection {
        transition_sequence: 0,
        source_record_identity: format!(
            "a40:a729:variant_upgrade_selection_rva_{VARIANT_UPGRADE_SELECTION_RVA:08X}"
        ),
        owner: SelectedDesignerOwner::EquipmentUpgradeDesignerWindow,
        family: SelectedDesignerFamily::VariantUpgrade,
        ordered_designs: vec![selected_design],
        selected_design,
    })
}

fn classify_variant(
    base: usize,
    mode: i32,
    definition: usize,
) -> Result<(SelectedDesignerOwner, SelectedDesignerFamily), String> {
    let predicate = |rva: u32| unsafe {
        let function: PredicateFn = mem::transmute(base + rva as usize);
        function(definition) != 0
    };
    match mode {
        0 if predicate(SHIP_CLASSIFIER_RVA) => Ok((
            SelectedDesignerOwner::CountryEquipmentDesignerMode0,
            SelectedDesignerFamily::Ship,
        )),
        0 if predicate(EQUIPMENT_CLASSIFIER_RVA) => Ok((
            SelectedDesignerOwner::CountryEquipmentDesignerMode0,
            SelectedDesignerFamily::Equipment,
        )),
        1 if predicate(TANK_CLASSIFIER_RVA) => Ok((
            SelectedDesignerOwner::TankDesignerMode1,
            SelectedDesignerFamily::Tank,
        )),
        2 if predicate(PLANE_CLASSIFIER_RVA) => Ok((
            SelectedDesignerOwner::PlaneDesignerMode2,
            SelectedDesignerFamily::Plane,
        )),
        _ => Err(format!(
            "A40 mode {mode} and exact equipment-definition classifiers disagree"
        )),
    }
}

fn publish_result(result: Result<ObservedSelection, String>) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.callback_count = state.callback_count.saturating_add(1);
    match result {
        Ok(mut observed) => {
            state.next_transition_sequence =
                state.next_transition_sequence.saturating_add(1).max(1);
            observed.transition_sequence = state.next_transition_sequence;
            state.latest.insert(observed.family, observed);
            state.current_error = None;
        }
        Err(error) => state.current_error = Some(error),
    }
}

fn publish_error(error: impl Into<String>) {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.callback_count = state.callback_count.saturating_add(1);
        state.current_error = Some(error.into());
    }
}

fn module_base() -> Result<usize, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        Err("A40 exact-a729 module base is unavailable".into())
    } else {
        Ok(base)
    }
}

fn read_identity(
    pointer: usize,
    expected_vtable_rva: usize,
    expected_type: u32,
) -> Result<StableDesignIdentity, String> {
    let base = module_base()?;
    if pointer == 0 {
        return Err("A40 selected object pointer is null".into());
    }
    let first = (
        read_usize(pointer),
        read_u32(pointer + OBJECT_TYPE),
        read_u32(pointer + STABLE_ID),
    );
    let second = (
        read_usize(pointer),
        read_u32(pointer + OBJECT_TYPE),
        read_u32(pointer + STABLE_ID),
    );
    if first != second {
        return Err("A40 selected object identity changed during capture".into());
    }
    let (vtable, object_type, stable_id) = first;
    if vtable != Some(base + expected_vtable_rva) {
        return Err("A40 selected object failed exact vtable gate".into());
    }
    let object_type = object_type.ok_or("A40 selected object type unreadable")?;
    let stable_id = stable_id
        .filter(|id| *id != 0)
        .ok_or("A40 selected object stable id unreadable/zero")?;
    if object_type != expected_type {
        return Err("A40 selected object type disagrees with owner family".into());
    }
    Ok(StableDesignIdentity {
        object_type,
        stable_id,
    })
}

fn verify_sentinel(address: usize, expected: &[u8]) -> Result<(), String> {
    let mut actual = vec![0; expected.len()];
    if !read_exact(address, &mut actual) {
        return Err(format!("A40 hook sentinel at {address:#x} unreadable"));
    }
    if actual != expected {
        return Err(format!("A40 hook sentinel mismatch at {address:#x}"));
    }
    Ok(())
}

fn read_exact(address: usize, out: &mut [u8]) -> bool {
    if address == 0 || out.is_empty() {
        return false;
    }
    let mut read = 0usize;
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            out.len(),
            &mut read,
        ) != 0 && read == out.len()
    }
}

fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0; 4];
    read_exact(address, &mut bytes).then(|| u32::from_le_bytes(bytes))
}

fn read_i32(address: usize) -> Option<i32> {
    read_u32(address).map(|value| value as i32)
}

fn read_usize(address: usize) -> Option<usize> {
    let mut bytes = [0; mem::size_of::<usize>()];
    read_exact(address, &mut bytes).then(|| usize::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Law: the receipt roster describes the sites the installer actually
    /// patches, in the order the installer patches them. See the D97 twin in
    /// `a40_peace_evidence.rs` for why positional agreement is load-bearing.
    ///
    /// Mutation proof: swap two entries of `PRODUCTION_HOOK_SPECS`, or point
    /// one `original_prefix` at a different sentinel, and this fails.
    #[test]
    fn the_receipt_specs_match_the_installers_own_site_table() {
        let expected: [(u32, &[u8]); HOOK_RECEIPT_COUNT] = [
            (EQUIPMENT_SELECTION_RVA, &EQUIPMENT_SELECTION_SENTINEL),
            (
                EQUIPMENT_AUTODESIGN_SUCCESS_RVA,
                &EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL,
            ),
            (DIVISION_ORDER_RVA, &DIVISION_ORDER_SENTINEL),
            (DIVISION_SELECTION_RVA, &DIVISION_SELECTION_SENTINEL),
            (VARIANT_UPGRADE_SELECTION_RVA, &VARIANT_UPGRADE_SENTINEL),
        ];
        for (index, (rva, sentinel)) in expected.iter().enumerate() {
            let spec = PRODUCTION_HOOK_SPECS[index];
            assert_eq!(spec.requested_rva, u64::from(*rva), "{}", spec.logical_name);
            assert_eq!(spec.original_prefix, *sentinel, "{}", spec.logical_name);
            assert_eq!(spec.target_patch_len, 5);
            assert_eq!(spec.primitive_id, MINHOOK_NORMAL_REL5_PRIMITIVE_ID);
            assert!(spec.logical_name.starts_with("gameplay.d85."));
        }
        let mut names = PRODUCTION_HOOK_SPECS
            .iter()
            .map(|spec| spec.logical_name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), HOOK_RECEIPT_COUNT, "receipt names must be distinct");
        let detours = detours();
        assert!(detours.iter().all(|detour| detour.address != 0));
        for (index, spec) in PRODUCTION_HOOK_SPECS.iter().enumerate() {
            assert_eq!(detours[index].logical_name, spec.logical_name);
        }
        production_hook_receipt::bind_detours_by_identity(
            "D85",
            &PRODUCTION_HOOK_SPECS,
            detours,
        )
        .expect("the selected-designer detour roster must bind to its specs by name");
        let mut distinct = detours.iter().map(|d| d.address).collect::<Vec<_>>();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            HOOK_RECEIPT_COUNT,
            "each selected-designer site needs its own detour"
        );
    }

    #[test]
    fn exact_hook_rvas_and_sentinels_are_frozen() {
        assert_eq!(EQUIPMENT_SELECTION_RVA, 0x0177_15A0);
        assert_eq!(EQUIPMENT_AUTODESIGN_SUCCESS_RVA, 0x0147_DDD0);
        assert_eq!(DIVISION_ORDER_RVA, 0x0175_6C40);
        assert_eq!(DIVISION_SELECTION_RVA, 0x0167_AEE0);
        assert_eq!(VARIANT_UPGRADE_SELECTION_RVA, 0x01F5_D420);
        assert_eq!(VARIANT_UPGRADE_NESTED_VIEW, 0x1BB0);
        assert_eq!(VARIANT_UPGRADE_NESTED_SELECTED, 0xA48);
        assert_eq!(VARIANT_UPGRADE_WINDOW_SELECTED, 0x1CE0);
        assert_eq!(&EQUIPMENT_SELECTION_SENTINEL[..4], &[0x48, 0x85, 0xD2, 0x0F]);
        assert_eq!(
            &EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL[..4],
            &[0x44, 0x88, 0x44, 0x24]
        );
        assert_eq!(&DIVISION_ORDER_SENTINEL[..4], &[0x40, 0x53, 0x55, 0x56]);
    }

    #[test]
    fn variant_upgrade_starts_unavailable_until_exact_input_callback() {
        let state = HookState::default();
        assert!(!state
            .latest
            .contains_key(&SelectedDesignerFamily::VariantUpgrade));
    }

    /// Law: an activated lane whose detour is not patched in says so, rather
    /// than reporting itself switched off.
    ///
    /// `PRODUCTION_ACTIVATION` is a compile-time `true`, so with `INSTALLED`
    /// false -- the state every passive production capture runs in, because
    /// the A40 install site sits behind `!passive_product_capture` in
    /// `collector/src/attach_log.rs` -- the ONLY reachable answer is
    /// "not installed". Restore the original
    /// `if !PRODUCTION_ACTIVATION || !INSTALLED.load(..)` guard and this fails
    /// with `ActivationBlocked`, which is exactly the reason all 396 d85
    /// frames in `runs/live_unlock/t15` carry today.
    #[test]
    fn an_activated_lane_with_no_detour_reports_the_detour_not_the_activation() {
        assert!(
            super::super::PRODUCTION_ACTIVATION,
            "this build activates the lane; if that changes the law below is \
             about a different state"
        );
        assert!(
            !INSTALLED.load(Ordering::Acquire),
            "the test process never patches hoi4.exe, so the hook is not installed"
        );
        assert_eq!(
            encode_latest(1, 0),
            Err(SelectedDesignerError::HookNotInstalled)
        );
    }

    #[test]
    fn campaign_reset_discards_prior_selection_errors_and_counters() {
        let mut state = HookState::default();
        state.callback_count = 9;
        state.current_error = Some("old campaign".into());
        clear_campaign_observations(&mut state);
        assert_eq!(state.callback_count, 0);
        assert!(state.current_error.is_none());
        assert!(state.latest.is_empty());
    }
}
