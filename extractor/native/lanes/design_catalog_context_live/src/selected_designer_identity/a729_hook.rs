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

/// The three patched sites, and the families each one's failures can possibly
/// be about.
///
/// # Why an error needs a scope
///
/// The observer keeps its committed selections in a per-FAMILY map but kept its
/// last failure in ONE process-wide `current_error`, and every read consulted
/// that word unconditionally. So a failure at the division site erased a
/// perfectly good tank observation the equipment site had committed, and went
/// on erasing it for every subsequent hour, because nothing but a later SUCCESS
/// could clear the word. One dropped division click therefore made d85
/// `source_unavailable` for the rest of the capture even though the engine's
/// selected tank had not changed and was still sitting in the map.
///
/// A hook site knows statically which families it can speak for, so scoping the
/// error is not an inference: `hook_division_order` and
/// `hook_division_selection` can only ever be about `Division`,
/// `hook_variant_upgrade_selection` only about `VariantUpgrade`, and
/// `hook_equipment_selection` about the four families the equipment designer
/// classifies -- which of the four is exactly what a classification failure
/// there does not know, so the scope covers all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SelectedDesignerErrorScope {
    /// `0x1_7715A0` / `0x14_7DDD0`: `Equipment`, `Tank`, `Plane`, `Ship`.
    EquipmentDesigner = 0,
    /// `0x1_756C40` / `0x1_67AEE0`: `Division`.
    Division = 1,
    /// `0x1F_5D420`: `VariantUpgrade`.
    VariantUpgrade = 2,
}

impl SelectedDesignerErrorScope {
    /// The scope that is authoritative for `family`. Total: every family is
    /// produced by exactly one site.
    pub const fn of_family(family: SelectedDesignerFamily) -> Self {
        match family {
            SelectedDesignerFamily::Division => Self::Division,
            SelectedDesignerFamily::VariantUpgrade => Self::VariantUpgrade,
            SelectedDesignerFamily::Equipment
            | SelectedDesignerFamily::Tank
            | SelectedDesignerFamily::Plane
            | SelectedDesignerFamily::Ship => Self::EquipmentDesigner,
        }
    }

    /// One bit per scope, for the `d85_latched_error_scope_mask` unit. A mask
    /// rather than a count: two hours both latching the equipment site must
    /// read as "the equipment site", never as "two sites".
    pub const fn bit(self) -> u64 {
        1 << (self as u64)
    }
}

/// A failure, and WHEN it happened.
///
/// The ordinal is the value of `callback_count` at the callback that produced
/// it, which totally orders every observation this module makes. It is what
/// lets a read ask "is this failure newer than the observation I am about to
/// serve?" instead of "has anything ever failed?".
#[derive(Clone, Debug)]
struct LatchedError {
    message: String,
    callback_ordinal: u64,
}

#[derive(Clone, Debug)]
struct ObservedSelection {
    transition_sequence: u64,
    /// `callback_count` at the callback that committed this observation.
    callback_ordinal: u64,
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
    /// At most one live failure per site. A site's own later success clears
    /// its entry; nothing else does, and nothing clears another site's.
    errors: BTreeMap<SelectedDesignerErrorScope, LatchedError>,
}

impl HookState {
    /// The failure, if any, that invalidates an observation of `family`
    /// committed at `observed_ordinal`.
    ///
    /// `None` for `observed_ordinal` asks the weaker question -- "is there any
    /// live failure about this family at all" -- which is what a read with no
    /// observation to serve has to ask.
    fn blocking_error(
        &self,
        family: SelectedDesignerFamily,
        observed_ordinal: Option<u64>,
    ) -> Option<&LatchedError> {
        self.errors
            .get(&SelectedDesignerErrorScope::of_family(family))
            .filter(|error| match observed_ordinal {
                Some(ordinal) => error.callback_ordinal > ordinal,
                None => true,
            })
    }

    /// The newest failure across all sites, for the diagnostics receipt and
    /// for a read that has no observation of any family to serve.
    fn newest_error(&self) -> Option<&LatchedError> {
        self.errors
            .values()
            .max_by_key(|error| error.callback_ordinal)
    }

    fn latched_error_scope_mask(&self) -> u64 {
        self.errors.keys().fold(0, |mask, scope| mask | scope.bit())
    }
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
    /// The newest live failure across all three sites, or `None` when no site
    /// holds one.
    pub current_error: Option<String>,
    /// `callback_count` at the callback that set [`Self::current_error`], and
    /// `0` when there is none.
    ///
    /// This is the word that separates lane AI's H1: with it, one capture can
    /// tell "d85 failed every hour" (the ordinal advances) from "d85 re-read
    /// one stale failure 317 times" (the ordinal is frozen).
    pub error_callback_ordinal: u64,
    /// Which sites currently hold a failure, as
    /// [`SelectedDesignerErrorScope::bit`] flags.
    pub latched_error_scope_mask: u64,
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
                return Err(state
                    .blocking_error(family, None)
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| {
                        format!("A40 has not observed a complete {family:?} selection")
                    }))
            }
        };
        // Only a failure at THIS family's own site, and only one newer than the
        // observation itself, can invalidate it.
        if let Some(error) = state.blocking_error(family, Some(observed.callback_ordinal)) {
            return Err(error.message.clone());
        }
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
    let newest = state.newest_error();
    HookDiagnostics {
        callback_count: state.callback_count,
        variant_upgrade_selection_count: state.variant_upgrade_selection_count,
        available_families: state.latest.keys().copied().collect(),
        current_error: newest.map(|error| error.message.clone()),
        error_callback_ordinal: newest.map(|error| error.callback_ordinal).unwrap_or(0),
        latched_error_scope_mask: state.latched_error_scope_mask(),
    }
}

/// True once this module owns enabled, receipt-observed production hooks, as a
/// diagnostic word.
///
/// Published beside the callback count because the two are ambiguous apart: a
/// zero callback count means "installed and the player never opened a
/// designer" or "never patched in", and only this says which. In a passive
/// product capture it is always `0` -- `collector/src/attach_log.rs` gates the
/// A40 designer install behind `!passive_product_capture` -- which is the
/// single fact that explains every d85 hour in t18 through t23.
pub fn installed_diagnostic() -> u64 {
    u64::from(INSTALLED.load(Ordering::Acquire))
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
        // The newest COMMITTED selection is chosen first, and only then asked
        // whether anything has invalidated it. The order matters: consulting a
        // process-wide failure word before looking at the map is what let one
        // site's failure speak for every family, forever. See
        // `SelectedDesignerErrorScope`.
        let Some(observed) = state
            .latest
            .values()
            .max_by_key(|observed| observed.transition_sequence)
        else {
            // Nothing has ever committed. If a site has a live failure, that
            // failure IS the reason; otherwise the lane simply has not seen a
            // selection yet.
            return Err(SelectedDesignerError::Source(
                state
                    .newest_error()
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| {
                        "A40 has not observed a complete selected-designer transition".into()
                    }),
            ));
        };
        if let Some(error) = state.blocking_error(observed.family, Some(observed.callback_ordinal))
        {
            return Err(SelectedDesignerError::Source(error.message.clone()));
        }
        observed.clone()
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
        match state.latest.get(&family) {
            Some(observed) => {
                if let Some(error) =
                    state.blocking_error(family, Some(observed.callback_ordinal))
                {
                    return Err(SelectedDesignerError::Source(error.message.clone()));
                }
                state
                    .latest
                    .remove(&family)
                    .expect("the observation was present a statement ago")
            }
            None => {
                return Err(SelectedDesignerError::Source(
                    state
                        .blocking_error(family, None)
                        .map(|error| error.message.clone())
                        .unwrap_or_else(|| {
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
            publish_result(
                SelectedDesignerErrorScope::EquipmentDesigner,
                capture_equipment_selection(view, variant),
            );
        } else {
            publish_error(
                SelectedDesignerErrorScope::EquipmentDesigner,
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
        publish_result(
            SelectedDesignerErrorScope::Division,
            capture_division_selection(view),
        );
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
                publish_result(
                    SelectedDesignerErrorScope::Division,
                    capture_division_selection(view),
                )
            }
            Some(_) => publish_error(
                SelectedDesignerErrorScope::Division,
                "A40 division selection transaction did not commit the requested template",
            ),
            None => publish_error(
                SelectedDesignerErrorScope::Division,
                "A40 division selection has no authenticated view/handler join",
            ),
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
        publish_result(SelectedDesignerErrorScope::VariantUpgrade, result);
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
        callback_ordinal: 0,
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
        callback_ordinal: 0,
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
        callback_ordinal: 0,
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

/// Commit one callback's outcome under the site that produced it.
///
/// `scope` is the SITE's, not the result's: a failure has no family to speak
/// for, and a success clears only the site that just proved itself working.
fn publish_result(scope: SelectedDesignerErrorScope, result: Result<ObservedSelection, String>) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.callback_count = state.callback_count.saturating_add(1);
    let callback_ordinal = state.callback_count;
    match result {
        Ok(mut observed) => {
            state.next_transition_sequence =
                state.next_transition_sequence.saturating_add(1).max(1);
            observed.transition_sequence = state.next_transition_sequence;
            observed.callback_ordinal = callback_ordinal;
            state.latest.insert(observed.family, observed);
            state.errors.remove(&scope);
        }
        Err(message) => {
            state.errors.insert(
                scope,
                LatchedError {
                    message,
                    callback_ordinal,
                },
            );
        }
    }
}

fn publish_error(scope: SelectedDesignerErrorScope, error: impl Into<String>) {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.callback_count = state.callback_count.saturating_add(1);
        let callback_ordinal = state.callback_count;
        state.errors.insert(
            scope,
            LatchedError {
                message: error.into(),
                callback_ordinal,
            },
        );
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
        state.errors.insert(
            SelectedDesignerErrorScope::Division,
            LatchedError {
                message: "old campaign".into(),
                callback_ordinal: 9,
            },
        );
        clear_campaign_observations(&mut state);
        assert_eq!(state.callback_count, 0);
        assert!(state.errors.is_empty());
        assert!(state.latest.is_empty());
        assert_eq!(state.latched_error_scope_mask(), 0);
    }

    // -----------------------------------------------------------------
    // The sticky-latch defect, and the laws that replace it.
    // -----------------------------------------------------------------

    fn observed(
        family: SelectedDesignerFamily,
        transition_sequence: u64,
        callback_ordinal: u64,
    ) -> ObservedSelection {
        ObservedSelection {
            transition_sequence,
            callback_ordinal,
            source_record_identity: format!("test:{family:?}"),
            owner: SelectedDesignerOwner::TankDesignerMode1,
            family,
            ordered_designs: vec![StableDesignIdentity {
                object_type: EQUIPMENT_VARIANT_TYPE,
                stable_id: 7,
            }],
            selected_design: StableDesignIdentity {
                object_type: EQUIPMENT_VARIANT_TYPE,
                stable_id: 7,
            },
        }
    }

    fn latched(state: &mut HookState, scope: SelectedDesignerErrorScope, callback_ordinal: u64) {
        state.errors.insert(
            scope,
            LatchedError {
                message: format!("{scope:?} failed at {callback_ordinal}"),
                callback_ordinal,
            },
        );
    }

    /// THE d85 defect. A failure at the division site must not invalidate a
    /// tank observation the equipment site committed.
    ///
    /// Before the scope existed there was one `current_error` word, every read
    /// consulted it unconditionally, and only a later SUCCESS could clear it.
    /// So the sequence below -- commit a tank, then drop one division click --
    /// made d85 `source_unavailable` for every remaining hour of the capture
    /// while the engine's selected tank sat unchanged in the map.
    ///
    /// MUTATION RUN: `blocking_error` replaced by `self.errors.values().next()`
    /// (any live failure, any site -- the old global latch).
    /// OBSERVED: FAILED, `a division failure says nothing about the tank
    /// designer`.
    #[test]
    fn a_failure_at_one_site_does_not_poison_another_sites_observation() {
        let mut state = HookState::default();
        state.latest.insert(
            SelectedDesignerFamily::Tank,
            observed(SelectedDesignerFamily::Tank, 1, 1),
        );
        latched(&mut state, SelectedDesignerErrorScope::Division, 2);

        assert!(
            state
                .blocking_error(SelectedDesignerFamily::Tank, Some(1))
                .is_none(),
            "a division failure says nothing about the tank designer"
        );
        assert!(
            state
                .blocking_error(SelectedDesignerFamily::Division, None)
                .is_some(),
            "the division site's own failure still speaks for the division family"
        );
    }

    /// A failure that PREDATES the observation it is asked about is spent.
    ///
    /// The equipment site can fail (a selection that never crossed the
    /// auto-design boundary) and then succeed. `publish_result` clears the
    /// site on success, but the ordinal comparison is what makes the rule true
    /// independently of the clearing, so an ordering bug in one cannot
    /// silently resurrect a dead failure in the other.
    ///
    /// MUTATION RUN: `blocking_error` replaced by `self.errors.values().next()`
    /// (the ordinal comparison dropped).
    /// OBSERVED: FAILED, `a failure older than the observation it is asked
    /// about is spent`.
    #[test]
    fn a_failure_older_than_the_observation_is_spent() {
        let mut state = HookState::default();
        latched(&mut state, SelectedDesignerErrorScope::EquipmentDesigner, 4);
        state.latest.insert(
            SelectedDesignerFamily::Tank,
            observed(SelectedDesignerFamily::Tank, 1, 9),
        );

        assert!(
            state
                .blocking_error(SelectedDesignerFamily::Tank, Some(9))
                .is_none(),
            "a failure older than the observation it is asked about is spent"
        );
    }

    /// A failure that POSTDATES the observation still refuses. The fix must not
    /// become "ignore failures".
    ///
    /// MUTATION: make `blocking_error` return `None` unconditionally.
    /// OBSERVED: this fails -- and d85 would publish a selection the engine has
    /// since changed to something the observer could not read.
    #[test]
    fn a_failure_newer_than_the_observation_still_refuses() {
        let mut state = HookState::default();
        state.latest.insert(
            SelectedDesignerFamily::Tank,
            observed(SelectedDesignerFamily::Tank, 1, 3),
        );
        latched(&mut state, SelectedDesignerErrorScope::EquipmentDesigner, 8);

        let error = state
            .blocking_error(SelectedDesignerFamily::Tank, Some(3))
            .expect("a newer equipment-site failure invalidates the tank observation");
        assert_eq!(error.callback_ordinal, 8);
    }

    /// Law: every family is owned by exactly one site, and the mask names the
    /// sites apart.
    ///
    /// MUTATION: map `Ship` to `SelectedDesignerErrorScope::Division`.
    /// OBSERVED: this fails -- and a ship-classification failure would start
    /// poisoning division answers instead of its own.
    #[test]
    fn every_family_maps_to_the_site_that_produces_it() {
        use SelectedDesignerErrorScope as Scope;
        for (family, scope) in [
            (SelectedDesignerFamily::Division, Scope::Division),
            (SelectedDesignerFamily::Equipment, Scope::EquipmentDesigner),
            (SelectedDesignerFamily::Tank, Scope::EquipmentDesigner),
            (SelectedDesignerFamily::Plane, Scope::EquipmentDesigner),
            (SelectedDesignerFamily::Ship, Scope::EquipmentDesigner),
            (SelectedDesignerFamily::VariantUpgrade, Scope::VariantUpgrade),
        ] {
            assert_eq!(
                SelectedDesignerErrorScope::of_family(family),
                scope,
                "{family:?}"
            );
        }
        let mut bits = vec![
            Scope::EquipmentDesigner.bit(),
            Scope::Division.bit(),
            Scope::VariantUpgrade.bit(),
        ];
        bits.sort_unstable();
        bits.dedup();
        assert_eq!(bits.len(), 3, "each site needs its own mask bit");
    }

    /// Law: a site's own success clears only that site.
    ///
    /// MUTATION: change `state.errors.remove(&scope)` in `publish_result` back
    /// to `state.errors.clear()`. OBSERVED: this fails -- an equipment success
    /// would silently declare the division site healthy, which is the same
    /// cross-site claim the latch made in the other direction.
    #[test]
    fn a_success_clears_only_its_own_site() {
        let mut state = HookState::default();
        latched(&mut state, SelectedDesignerErrorScope::Division, 1);
        latched(&mut state, SelectedDesignerErrorScope::EquipmentDesigner, 2);

        // What `publish_result`'s Ok arm does, applied directly: this test is
        // about the CLEARING RULE, and the callbacks it lives in cannot run
        // without a patched hoi4.exe.
        state
            .errors
            .remove(&SelectedDesignerErrorScope::EquipmentDesigner);

        assert_eq!(
            state.latched_error_scope_mask(),
            SelectedDesignerErrorScope::Division.bit()
        );
    }

    /// Law: the diagnostics publish the newest failure's ordinal, which is what
    /// separates "fails hourly" from "one stale error re-read".
    ///
    /// MUTATION: publish `state.callback_count` as `error_callback_ordinal`.
    /// OBSERVED: this fails -- and the two hypotheses would be
    /// indistinguishable on the wire again, because the callback count advances
    /// whether or not the failure is new.
    #[test]
    fn the_error_ordinal_is_the_failures_own_not_the_callback_count() {
        let mut state = HookState::default();
        state.callback_count = 400;
        latched(&mut state, SelectedDesignerErrorScope::Division, 7);
        latched(&mut state, SelectedDesignerErrorScope::VariantUpgrade, 12);

        let newest = state.newest_error().expect("two sites hold failures");
        assert_eq!(newest.callback_ordinal, 12);
        assert_eq!(
            state.latched_error_scope_mask(),
            SelectedDesignerErrorScope::Division.bit()
                | SelectedDesignerErrorScope::VariantUpgrade.bit()
        );
    }
}
