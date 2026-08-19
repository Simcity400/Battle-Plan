use super::{
    collect_fixture_and_encode, ExactSelectedDesignerIdentitySource, SelectedDesignerError,
    SelectedDesignerFamily, SelectedDesignerIdentityFrame, SelectedDesignerOwner,
    StableDesignIdentity, CLASSIFIER_MANIFEST_SHA256, DIVISION_CLICK_THUNK_ARTIFACT_SHA256,
    DIVISION_WRAPPER_ARTIFACT_SHA256, EQUIPMENT_VARIANT_TYPE, EXACT_A729_EXECUTABLE_SHA256,
    MAX_ORDERED_DESIGNS, PRODUCER_MANIFEST_SHA256, REFERENCED_DIVISION_TEMPLATE_TYPE,
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
pub const DIVISION_VIEW_HANDLER_REPLACE_RVA: u32 = 0x0174_F670;
pub const DIVISION_VIEW_DESTROY_RVA: u32 = 0x0174_8D60;
pub const VARIANT_UPGRADE_SELECTION_RVA: u32 = 0x01F5_D420;
pub const PLANE_CLASSIFIER_RVA: u32 = 0x00C8_7CE0;
pub const TANK_CLASSIFIER_RVA: u32 = 0x00C8_7EB0;
pub const SHIP_CLASSIFIER_RVA: u32 = 0x00C8_8440;
pub const EQUIPMENT_CLASSIFIER_RVA: u32 = 0x00C8_84B0;

const EQUIPMENT_SELECTION_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xD2, 0x0F, 0x84, 0x14, 0x03, 0x00, 0x00, 0x48, 0x89, 0x5C, 0x24, 0x08, 0x55, 0x56,
];
const EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL: [u8; 16] = [
    0x44, 0x88, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4C, 0x24, 0x08, 0x55,
];
pub const EQUIPMENT_AUTODESIGN_SUCCESS_MANIFEST_SHA256: &str =
    "44bb3647f7285c8b961aca7ba0d702289c1145ddf983421369bf9abdc90e0ce5";
pub const EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL_ARTIFACT_SHA256: &str =
    "618865df111fb54b0a6d6773b1832a5696c5c916a9e66dbe4c00aba186a5be80";
const DIVISION_ORDER_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xEC,
];
const DIVISION_SELECTION_SENTINEL: [u8; 16] = [
    0x4C, 0x8B, 0xC2, 0xC7, 0x01, 0x02, 0x00, 0x00, 0x00, 0x48, 0x83, 0xC2, 0x18, 0xE9, 0x7E, 0x12,
];
const VARIANT_UPGRADE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x54, 0x41, 0x56,
];
const DIVISION_VIEW_HANDLER_REPLACE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x48,
];
const DIVISION_VIEW_DESTROY_SENTINEL: [u8; 16] = [
    0x48, 0x8B, 0xC4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10, 0x48, 0x89, 0x78, 0x18, 0x55,
];

pub const HOOK_RECEIPT_COUNT: usize = 7;

pub const PRODUCTION_HOOK_SPECS: [ExpectedRvaHook; HOOK_RECEIPT_COUNT] = [
    ExpectedRvaHook {
        logical_name: "gameplay.d85.division_view_handler_replace",
        requested_rva: DIVISION_VIEW_HANDLER_REPLACE_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &DIVISION_VIEW_HANDLER_REPLACE_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d85.division_view_destroy",
        requested_rva: DIVISION_VIEW_DESTROY_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &DIVISION_VIEW_DESTROY_SENTINEL,
    },
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

fn detours() -> [SiteDetour; HOOK_RECEIPT_COUNT] {
    [
        SiteDetour::new(
            "gameplay.d85.division_view_handler_replace",
            hook_division_view_handler_replace as *const () as usize,
        ),
        SiteDetour::new(
            "gameplay.d85.division_view_destroy",
            hook_division_view_destroy as *const () as usize,
        ),
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
static ORIGINAL_DIVISION_VIEW_HANDLER_REPLACE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DIVISION_VIEW_DESTROY: AtomicUsize = AtomicUsize::new(0);
static DIVISION_VIEW_RETIREMENT_UNPROVED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());
static FAILED_INSTALL_REMOVAL_UNPROVED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
    static IN_EQUIPMENT_SELECTION: Cell<bool> = const { Cell::new(false) };
    static EQUIPMENT_AUTODESIGN_SUCCEEDED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SelectedDesignerErrorScope {
    EquipmentDesigner = 0,
    Division = 1,
    VariantUpgrade = 2,
}

impl SelectedDesignerErrorScope {
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

    pub const fn bit(self) -> u64 {
        1 << (self as u64)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectedDesignerFailure {
    ModuleBaseUnavailable,
    D78Controller(String),
    EquipmentNoActiveControllerSnapshot,
    EquipmentControllerViewDiffers,
    EquipmentViewModeUnreadable,
    EquipmentVariantDefinitionNull,
    EquipmentSelectionAbsentFromOrder,
    EquipmentAutodesignBoundaryNotCrossed,
    DivisionNullView,
    DivisionHandlerNull,
    DivisionRowPoolUsedUnreadable,
    DivisionRowPoolUsedOutOfBounds,
    DivisionRowPoolDataNull,
    DivisionRowNull(usize),
    DivisionRowTemplateNull(usize),
    DivisionSelectedTemplateNull,
    DivisionOrderChangedDuringCapture,
    DivisionSelectedAbsentFromOrder,
    DivisionTransactionDidNotCommitTemplate,
    DivisionNoViewHandlerJoin,
    DivisionStaleViewHandlerJoin,
    DivisionViewRetiredDuringCapture,
    VariantUpgradeNullWindow,
    VariantUpgradeNestedViewNull,
    VariantUpgradeTransactionMismatch,
    IdentityPointerNull,
    IdentityChangedDuringCapture,
    IdentityVtableGate,
    IdentityObjectTypeUnreadable,
    IdentityStableIdUnreadableOrZero,
    IdentityObjectTypeDisagreesWithFamily,
    ClassifierDisagreesWithMode(i32),
}

impl SelectedDesignerFailure {
    pub fn code(&self) -> u64 {
        match self {
            Self::ModuleBaseUnavailable => 101,
            Self::D78Controller(_) => 102,
            Self::EquipmentNoActiveControllerSnapshot => 201,
            Self::EquipmentControllerViewDiffers => 202,
            Self::EquipmentViewModeUnreadable => 203,
            Self::EquipmentVariantDefinitionNull => 204,
            Self::EquipmentSelectionAbsentFromOrder => 205,
            Self::EquipmentAutodesignBoundaryNotCrossed => 206,
            Self::DivisionNullView => 301,
            Self::DivisionHandlerNull => 302,
            Self::DivisionRowPoolUsedUnreadable => 303,
            Self::DivisionRowPoolUsedOutOfBounds => 304,
            Self::DivisionRowPoolDataNull => 305,
            Self::DivisionRowNull(_) => 306,
            Self::DivisionRowTemplateNull(_) => 307,
            Self::DivisionSelectedTemplateNull => 308,
            Self::DivisionOrderChangedDuringCapture => 309,
            Self::DivisionSelectedAbsentFromOrder => 310,
            Self::DivisionTransactionDidNotCommitTemplate => 311,
            Self::DivisionNoViewHandlerJoin => 312,
            Self::DivisionStaleViewHandlerJoin => 313,
            Self::DivisionViewRetiredDuringCapture => 314,
            Self::VariantUpgradeNullWindow => 401,
            Self::VariantUpgradeNestedViewNull => 402,
            Self::VariantUpgradeTransactionMismatch => 403,
            Self::IdentityPointerNull => 501,
            Self::IdentityChangedDuringCapture => 502,
            Self::IdentityVtableGate => 503,
            Self::IdentityObjectTypeUnreadable => 504,
            Self::IdentityStableIdUnreadableOrZero => 505,
            Self::IdentityObjectTypeDisagreesWithFamily => 506,
            Self::ClassifierDisagreesWithMode(_) => 507,
        }
    }
}

impl std::fmt::Display for SelectedDesignerFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModuleBaseUnavailable => {
                formatter.write_str("A40 exact-a729 module base is unavailable")
            }
            Self::D78Controller(message) => formatter.write_str(message),
            Self::EquipmentNoActiveControllerSnapshot => {
                formatter.write_str("A40 equipment selection has no active d78 controller snapshot")
            }
            Self::EquipmentControllerViewDiffers => {
                formatter.write_str("A40 equipment selection and d78 controller view differ")
            }
            Self::EquipmentViewModeUnreadable => {
                formatter.write_str("A40 equipment view mode unreadable")
            }
            Self::EquipmentVariantDefinitionNull => {
                formatter.write_str("A40 selected variant definition is null/unreadable")
            }
            Self::EquipmentSelectionAbsentFromOrder => {
                formatter.write_str("A40 selected variant is absent from the exact d78 order")
            }
            Self::EquipmentAutodesignBoundaryNotCrossed => formatter.write_str(
                "A40 equipment selection did not cross the exact successful auto-design boundary",
            ),
            Self::DivisionNullView => formatter.write_str("A40 division order supplied null view"),
            Self::DivisionHandlerNull => {
                formatter.write_str("A40 division handler is null/unreadable")
            }
            Self::DivisionRowPoolUsedUnreadable => {
                formatter.write_str("A40 division row-pool used count unreadable")
            }
            Self::DivisionRowPoolUsedOutOfBounds => {
                formatter.write_str("A40 division row-pool used count is empty/out of bounds")
            }
            Self::DivisionRowPoolDataNull => {
                formatter.write_str("A40 division row-pool data is null/unreadable")
            }
            Self::DivisionRowNull(index) => {
                write!(formatter, "A40 division row {index} is null/unreadable")
            }
            Self::DivisionRowTemplateNull(index) => {
                write!(
                    formatter,
                    "A40 division row {index} template is null/unreadable"
                )
            }
            Self::DivisionSelectedTemplateNull => {
                formatter.write_str("A40 division selected template is null/unreadable")
            }
            Self::DivisionOrderChangedDuringCapture => {
                formatter.write_str("A40 division order/selection changed during capture")
            }
            Self::DivisionSelectedAbsentFromOrder => formatter
                .write_str("A40 selected division template is absent from controller order"),
            Self::DivisionTransactionDidNotCommitTemplate => formatter.write_str(
                "A40 division selection transaction did not commit the requested template",
            ),
            Self::DivisionNoViewHandlerJoin => {
                formatter.write_str("A40 division selection has no authenticated view/handler join")
            }
            Self::DivisionStaleViewHandlerJoin => formatter
                .write_str("A40 division selection's joined view no longer names this handler"),
            Self::DivisionViewRetiredDuringCapture => formatter
                .write_str("A40 division designer view was retired while this capture read it"),
            Self::VariantUpgradeNullWindow => {
                formatter.write_str("A40 variant-upgrade selection supplied null window")
            }
            Self::VariantUpgradeNestedViewNull => {
                formatter.write_str("A40 variant-upgrade nested view is null/unreadable")
            }
            Self::VariantUpgradeTransactionMismatch => formatter.write_str(
                "A40 variant-upgrade transaction did not commit one identical source variant",
            ),
            Self::IdentityPointerNull => formatter.write_str("A40 selected object pointer is null"),
            Self::IdentityChangedDuringCapture => {
                formatter.write_str("A40 selected object identity changed during capture")
            }
            Self::IdentityVtableGate => {
                formatter.write_str("A40 selected object failed exact vtable gate")
            }
            Self::IdentityObjectTypeUnreadable => {
                formatter.write_str("A40 selected object type unreadable")
            }
            Self::IdentityStableIdUnreadableOrZero => {
                formatter.write_str("A40 selected object stable id unreadable/zero")
            }
            Self::IdentityObjectTypeDisagreesWithFamily => {
                formatter.write_str("A40 selected object type disagrees with owner family")
            }
            Self::ClassifierDisagreesWithMode(mode) => write!(
                formatter,
                "A40 mode {mode} and exact equipment-definition classifiers disagree"
            ),
        }
    }
}

#[derive(Clone, Debug)]
struct LatchedError {
    reason: SelectedDesignerFailure,
    callback_ordinal: u64,
}

impl LatchedError {
    fn message(&self) -> String {
        self.reason.to_string()
    }
}

#[derive(Clone, Debug)]
struct ObservedSelection {
    transition_sequence: u64,
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
    division_view_registrations: u64,
    division_view_retirements: u64,
    division_view_teardowns_in_flight: u32,
    division_view_handler_replacements: u64,
    division_view_destructions: u64,
    callback_count: u64,
    variant_upgrade_selection_count: u64,
    errors: BTreeMap<SelectedDesignerErrorScope, LatchedError>,
}

impl HookState {
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
    pub division_view_handler_replace: usize,
    pub division_view_destroy: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub callback_count: u64,
    pub variant_upgrade_selection_count: u64,
    pub division_view_registrations: u64,
    pub available_families: Vec<SelectedDesignerFamily>,
    pub current_error: Option<String>,
    pub error_callback_ordinal: u64,
    pub latched_error_scope_mask: u64,
    pub latched_error_reason_code: u64,
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
                    .map(LatchedError::message)
                    .unwrap_or_else(|| {
                        format!("A40 has not observed a complete {family:?} selection")
                    }))
            }
        };
        if let Some(error) = state.blocking_error(family, Some(observed.callback_ordinal)) {
            return Err(error.message());
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
        (
            DIVISION_VIEW_HANDLER_REPLACE_RVA,
            &DIVISION_VIEW_HANDLER_REPLACE_SENTINEL[..],
        ),
        (
            DIVISION_VIEW_DESTROY_RVA,
            &DIVISION_VIEW_DESTROY_SENTINEL[..],
        ),
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
    *state.lock().map_err(|_| "A40 hook state mutex poisoned")? = HookState::default();
    MODULE_BASE.store(base, Ordering::Release);
    let addresses = installed_addresses(base);
    let hooks = [
        (
            addresses.division_view_handler_replace,
            hook_division_view_handler_replace as *mut c_void,
            &ORIGINAL_DIVISION_VIEW_HANDLER_REPLACE,
        ),
        (
            addresses.division_view_destroy,
            hook_division_view_destroy as *mut c_void,
            &ORIGINAL_DIVISION_VIEW_DESTROY,
        ),
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
                        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) =>
                            {}
                        Err(error) => {
                            cleanup_failures.push(format!("remove {rollback:#x}: {error}"))
                        }
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
                return Err(format!("A40 hook creation at {target:#x} failed: {status}"));
            }
        };
        original_slot.store(original as usize, Ordering::Release);
        created.push(target);
    }
    let batch = named_batch(&created);
    let report = unsafe { inline_detour::enable_batch(&batch) };
    if let Some((name, status)) = report.refusal() {
        let mut cleanup_failures = release_batch_failures("disable", unsafe {
            inline_detour::disable_batch(&reversed_batch(&created))
        });
        if cleanup_failures.is_empty() {
            if let Err(error) = wait_for_quiescence() {
                cleanup_failures.push(error);
            }
        }
        if cleanup_failures.is_empty() {
            cleanup_failures = release_batch_failures("remove", unsafe {
                inline_detour::remove_batch(&reversed_batch(&created))
            });
        }
        if !cleanup_failures.is_empty() {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A40 hook enable at {name} failed: {status}; rollback failed: {}",
                cleanup_failures.join(", ")
            ));
        }
        clear_originals();
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("A40 hook enable at {name} failed: {status}"));
    }
    if let Err(error) = HOOK_RECEIPTS.observe_installed(base, detours()) {
        let cleanup = release_created_sites(&created);
        if !cleanup.is_empty() {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A40 selected-designer hook receipt refused: {error}; rollback failed: {}",
                cleanup.join(", ")
            ));
        }
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

fn release_created_sites(created: &[usize]) -> Vec<String> {
    let mut failures = release_batch_failures("disable", unsafe {
        inline_detour::disable_batch(&reversed_batch(created))
    });
    if !failures.is_empty() {
        return failures;
    }
    if let Err(error) = wait_for_quiescence() {
        failures.push(error);
        return failures;
    }
    release_batch_failures("remove", unsafe {
        inline_detour::remove_batch(&reversed_batch(created))
    })
}

fn named_batch(created: &[usize]) -> Vec<(&'static str, *mut c_void)> {
    created
        .iter()
        .enumerate()
        .map(|(index, target)| {
            let name = PRODUCTION_HOOK_SPECS
                .get(index)
                .map(|spec| spec.logical_name)
                .unwrap_or("gameplay.d85.unnamed_site");
            (name, *target as *mut c_void)
        })
        .collect()
}

/// [`named_batch`] in reverse, which is the release order this module has
/// release order is what an operator expects to read.
fn reversed_batch(created: &[usize]) -> Vec<(&'static str, *mut c_void)> {
    let mut batch = named_batch(created);
    batch.reverse();
    batch
}

fn selection_batch_report(
    batch: &[(&'static str, *mut c_void)],
) -> inline_detour::BatchReleaseReport {
    unsafe { inline_detour::disable_batch(batch) }
}

/// The first refused site of a release batch, rendered the way this module has
fn first_disable_failure(report: &inline_detour::BatchReleaseReport) -> Option<String> {
    report
        .first_failure()
        .map(|(name, status)| format!("A40 hook disable at {name} failed: {status}"))
}

/// Render a batched release report's refusals, named, with the operation that
fn release_batch_failures(
    operation: &str,
    report: inline_detour::BatchReleaseReport,
) -> Vec<String> {
    report
        .failures()
        .into_iter()
        .map(|(name, error)| format!("{operation} {name}: {error}"))
        .collect()
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.installed_receipts()
}

/// Consume the release receipts. One-shot: a second call after the registry has
/// taken custody reports them unavailable rather than minting a second release.
pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String>
{
    HOOK_RECEIPTS.take_released_receipts()
}

/// True while this module holds a release it could not PROVE.
/// disables succeed but publishes the release only after every removal also
pub fn release_pending() -> bool {
    FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire)
        || (!installed() && HOOK_RECEIPTS.may_still_hold_installed_custody())
}

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
    // left behind kept `release_pending()` true forever, which is what turned a
    let latched = FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire);
    let mut base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 && latched {
        base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    }
    if !needs_disable && base == 0 {
        return Ok(());
    }
    let addresses = installed_addresses(base);
    let selection_targets = [
        addresses.variant_upgrade_selection,
        addresses.division_selection,
        addresses.division_order,
        addresses.equipment_autodesign_success,
        addresses.equipment_selection,
    ];
    let destruction_targets = [
        addresses.division_view_destroy,
        addresses.division_view_handler_replace,
    ];
    let targets = [selection_targets.as_slice(), destruction_targets.as_slice()].concat();
    let install_order = [
        addresses.division_view_handler_replace,
        addresses.division_view_destroy,
        addresses.equipment_selection,
        addresses.equipment_autodesign_success,
        addresses.division_order,
        addresses.division_selection,
        addresses.variant_upgrade_selection,
    ];
    let release_order = reversed_batch(&install_order);
    if release_order
        .iter()
        .map(|(_, target)| *target as usize)
        .ne(targets.iter().copied())
    {
        return Err(
            "A40 selected-designer release order does not match the install-order roster".into(),
        );
    }
    let (selection_batch, destruction_batch) = release_order.split_at(selection_targets.len());
    let mut first_error = None;
    if needs_disable {
        first_error = first_disable_failure(&selection_batch_report(selection_batch));
        if let Some(error) = first_error.as_ref() {
            // The record is the ledger `inline_detour` keeps for every release
            return Err(format!(
                "{error}; A40 release attempts: {}",
                inline_detour::render_release_attempts_for(&targets)
            ));
        }
        wait_for_quiescence()?;
        first_error = first_disable_failure(&selection_batch_report(destruction_batch));
        if let Some(error) = first_error.as_ref() {
            return Err(format!(
                "{error}; A40 release attempts: {}",
                inline_detour::render_release_attempts_for(&targets)
            ));
        }
        INSTALLED.store(false, Ordering::Release);
    }
    wait_for_quiescence()?;
    let removal_batch = reversed_batch(&install_order);
    for (name, status) in unsafe { inline_detour::remove_batch(&removal_batch) }.failures() {
        first_error.get_or_insert_with(|| format!("A40 hook removal at {name} failed: {status}"));
    }
    if let Some(error) = first_error {
        return Err(format!(
            "{error}; A40 release attempts: {}",
            inline_detour::render_release_attempts_for(&targets)
        ));
    }
    // A release receipt may not be minted while any of this module's own sites
    let unproven = inline_detour::unproven_releases_among(&targets);
    if !unproven.is_empty() {
        return Err(format!(
            "A40 selected-designer release is unproven for {} site(s): {}",
            unproven.len(),
            inline_detour::render_release_attempts_for(&targets)
        ));
    }
    // Mint the release receipts only for a teardown that provably restored
    // returned clean; `observe_released_set` then independently re-reads each
    // receipts -- has nothing to release, and that is not an error.
    if HOOK_RECEIPTS.has_complete_installed_custody() {
        if let Err(error) = HOOK_RECEIPTS.publish_release(true) {
            return Err(format!(
                "A40 selected-designer hook release receipt refused: {error}"
            ));
        }
    }
    // `unproven_releases_among` independently agreed over the same targets.
    // leaving it would keep `release_pending()` true over sites nothing can
    // Without this, a lane that failed to install once answered "release
    if FAILED_INSTALL_REMOVAL_UNPROVED.swap(false, Ordering::AcqRel)
        && !HOOK_RECEIPTS.has_complete_installed_custody()
    {
        HOOK_RECEIPTS.discard_all_custody();
    }
    clear_originals();
    MODULE_BASE.store(0, Ordering::Release);
    if let Some(state) = STATE.get() {
        *state.lock().map_err(|_| "A40 hook state mutex poisoned")? = HookState::default();
    }
    Ok(())
}

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
        division_view_registrations: state.division_view_registrations,
        available_families: state.latest.keys().copied().collect(),
        current_error: newest.map(LatchedError::message),
        error_callback_ordinal: newest.map(|error| error.callback_ordinal).unwrap_or(0),
        latched_error_scope_mask: state.latched_error_scope_mask(),
        latched_error_reason_code: newest.map(|error| error.reason.code()).unwrap_or(0),
    }
}

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

pub fn encode_latest(
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, SelectedDesignerError> {
    if !super::PRODUCTION_ACTIVATION {
        return Err(SelectedDesignerError::ActivationBlocked);
    }
    if !INSTALLED.load(Ordering::Acquire) {
        return Err(SelectedDesignerError::HookNotInstalled);
    }
    let observed = {
        let state = STATE
            .get()
            .ok_or_else(|| {
                SelectedDesignerError::Source("A40 hook state is not initialized".into())
            })?
            .lock()
            .map_err(|_| SelectedDesignerError::Source("A40 hook state mutex poisoned".into()))?;
        let Some(observed) = state
            .latest
            .values()
            .max_by_key(|observed| observed.transition_sequence)
        else {
            return Err(SelectedDesignerError::Source(
                state
                    .newest_error()
                    .map(LatchedError::message)
                    .unwrap_or_else(|| {
                        "A40 has not observed a complete selected-designer transition".into()
                    }),
            ));
        };
        if let Some(error) = state.blocking_error(observed.family, Some(observed.callback_ordinal))
        {
            return Err(SelectedDesignerError::Source(error.message()));
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
    let retirements = state.division_view_retirements;
    let teardowns_in_flight = state.division_view_teardowns_in_flight;
    let handler_replacements = state.division_view_handler_replacements;
    let destructions = state.division_view_destructions;
    *state = HookState::default();
    state.division_view_retirements = retirements;
    state.division_view_teardowns_in_flight = teardowns_in_flight;
    state.division_view_handler_replacements = handler_replacements;
    state.division_view_destructions = destructions;
}

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
            .ok_or_else(|| {
                SelectedDesignerError::Source("A40 hook state is not initialized".into())
            })?
            .lock()
            .map_err(|_| SelectedDesignerError::Source("A40 hook state mutex poisoned".into()))?;
        match state.latest.get(&family) {
            Some(observed) => {
                if let Some(error) = state.blocking_error(family, Some(observed.callback_ordinal)) {
                    return Err(SelectedDesignerError::Source(error.message()));
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
                        .map(LatchedError::message)
                        .unwrap_or_else(|| format!("A40 has no undrained {family:?} selection")),
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
        equipment_autodesign_success: base + EQUIPMENT_AUTODESIGN_SUCCESS_RVA as usize,
        division_order: base + DIVISION_ORDER_RVA as usize,
        division_selection: base + DIVISION_SELECTION_RVA as usize,
        variant_upgrade_selection: base + VARIANT_UPGRADE_SELECTION_RVA as usize,
        division_view_handler_replace: base + DIVISION_VIEW_HANDLER_REPLACE_RVA as usize,
        division_view_destroy: base + DIVISION_VIEW_DESTROY_RVA as usize,
    }
}

fn clear_originals() {
    ORIGINAL_EQUIPMENT_SELECTION.store(0, Ordering::Release);
    ORIGINAL_EQUIPMENT_AUTODESIGN_SUCCESS.store(0, Ordering::Release);
    ORIGINAL_DIVISION_ORDER.store(0, Ordering::Release);
    ORIGINAL_DIVISION_SELECTION.store(0, Ordering::Release);
    ORIGINAL_VARIANT_UPGRADE.store(0, Ordering::Release);
    ORIGINAL_DIVISION_VIEW_HANDLER_REPLACE.store(0, Ordering::Release);
    ORIGINAL_DIVISION_VIEW_DESTROY.store(0, Ordering::Release);
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
                SelectedDesignerFailure::EquipmentAutodesignBoundaryNotCrossed,
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
        let epoch = observe_division_view_epoch();
        register_division_view(view, epoch);
        publish_result(
            SelectedDesignerErrorScope::Division,
            refuse_if_view_retired(epoch, capture_division_selection(view)),
        );
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

fn register_division_view(view: usize, before: Option<DivisionViewEpoch>) {
    if view == 0 {
        return;
    }
    let Some(before) = before.filter(|epoch| epoch.quiet()) else {
        return;
    };
    let Some(handler) = read_usize(view + DIVISION_HANDLER).filter(|pointer| *pointer != 0) else {
        return;
    };
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        if state.division_view_retirements != before.retirements
            || state.division_view_teardowns_in_flight != 0
        {
            return;
        }
        state.division_views_by_handler.insert(handler, view);
        state.division_view_registrations = state.division_view_registrations.saturating_add(1);
    }
}

/// The selection callback copies the view out of the map and releases the lock
fn forget_division_view(handler: usize, stale: usize) {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        if state.division_views_by_handler.get(&handler) == Some(&stale) {
            state.division_views_by_handler.remove(&handler);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DivisionViewEpoch {
    retirements: u64,
    teardowns_in_flight: u32,
}

impl DivisionViewEpoch {
    const fn quiet(self) -> bool {
        self.teardowns_in_flight == 0
    }
}

fn observe_division_view_epoch() -> Option<DivisionViewEpoch> {
    if DIVISION_VIEW_RETIREMENT_UNPROVED.load(Ordering::Acquire) {
        return None;
    }
    let state = STATE.get()?.lock().ok()?;
    Some(DivisionViewEpoch {
        retirements: state.division_view_retirements,
        teardowns_in_flight: state.division_view_teardowns_in_flight,
    })
}

fn join_division_view(handler: usize) -> (Option<DivisionViewEpoch>, Option<usize>) {
    if DIVISION_VIEW_RETIREMENT_UNPROVED.load(Ordering::Acquire) {
        return (None, None);
    }
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return (None, None);
    };
    (
        Some(DivisionViewEpoch {
            retirements: state.division_view_retirements,
            teardowns_in_flight: state.division_view_teardowns_in_flight,
        }),
        state.division_views_by_handler.get(&handler).copied(),
    )
}

fn refuse_if_view_retired(
    before: Option<DivisionViewEpoch>,
    captured: Result<ObservedSelection, SelectedDesignerFailure>,
) -> Result<ObservedSelection, SelectedDesignerFailure> {
    if captured.is_err() {
        return captured;
    }
    match (before, observe_division_view_epoch()) {
        (Some(before), Some(after)) if before == after && before.quiet() => captured,
        _ => Err(SelectedDesignerFailure::DivisionViewRetiredDuringCapture),
    }
}

struct DivisionViewTeardown {
    recorded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DivisionViewTeardownCause {
    HandlerReplaced,
    ViewDestroyed,
}

impl DivisionViewTeardown {
    fn open(view: usize, cause: DivisionViewTeardownCause) -> Self {
        let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
            DIVISION_VIEW_RETIREMENT_UNPROVED.store(true, Ordering::Release);
            return Self { recorded: false };
        };
        if view != 0 {
            state
                .division_views_by_handler
                .retain(|_, joined| *joined != view);
        }
        match cause {
            DivisionViewTeardownCause::HandlerReplaced => {
                state.division_view_handler_replacements =
                    state.division_view_handler_replacements.saturating_add(1);
            }
            DivisionViewTeardownCause::ViewDestroyed => {
                state.division_view_destructions =
                    state.division_view_destructions.saturating_add(1);
            }
        }
        let Some(in_flight) = state.division_view_teardowns_in_flight.checked_add(1) else {
            DIVISION_VIEW_RETIREMENT_UNPROVED.store(true, Ordering::Release);
            return Self { recorded: false };
        };
        state.division_view_teardowns_in_flight = in_flight;
        Self { recorded: true }
    }
}

impl Drop for DivisionViewTeardown {
    fn drop(&mut self) {
        if !self.recorded {
            return;
        }
        let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
            DIVISION_VIEW_RETIREMENT_UNPROVED.store(true, Ordering::Release);
            return;
        };
        match (
            state.division_view_teardowns_in_flight.checked_sub(1),
            state.division_view_retirements.checked_add(1),
        ) {
            (Some(in_flight), Some(retirements)) => {
                state.division_view_teardowns_in_flight = in_flight;
                state.division_view_retirements = retirements;
            }
            _ => DIVISION_VIEW_RETIREMENT_UNPROVED.store(true, Ordering::Release),
        }
    }
}

unsafe extern "system" fn hook_division_view_handler_replace(view: usize) {
    let _callback = CallbackGuard::enter();
    let _teardown = DivisionViewTeardown::open(view, DivisionViewTeardownCause::HandlerReplaced);
    call_one_arg_original(&ORIGINAL_DIVISION_VIEW_HANDLER_REPLACE, view);
}

unsafe extern "system" fn hook_division_view_destroy(view: usize) {
    let _callback = CallbackGuard::enter();
    let _teardown = DivisionViewTeardown::open(view, DivisionViewTeardownCause::ViewDestroyed);
    call_one_arg_original(&ORIGINAL_DIVISION_VIEW_DESTROY, view);
}

unsafe extern "system" fn hook_division_selection(handler: usize, template: usize) {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    call_two_arg_original(&ORIGINAL_DIVISION_SELECTION, handler, template);
    if !nested {
        let (epoch, view) = join_division_view(handler);
        let captured = match view {
            Some(view) if read_usize(view + DIVISION_HANDLER) != Some(handler) => {
                forget_division_view(handler, view);
                Err(SelectedDesignerFailure::DivisionStaleViewHandlerJoin)
            }
            Some(view) if read_usize(handler + DIVISION_HANDLER_SELECTED) == Some(template) => {
                capture_division_selection(view)
            }
            Some(_) => Err(SelectedDesignerFailure::DivisionTransactionDidNotCommitTemplate),
            None => Err(SelectedDesignerFailure::DivisionNoViewHandlerJoin),
        };
        publish_result(
            SelectedDesignerErrorScope::Division,
            refuse_if_view_retired(epoch, captured),
        );
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
) -> Result<ObservedSelection, SelectedDesignerFailure> {
    let base = module_base()?;
    let designer = crate::a729_hook::latest_active_role_root()
        .map_err(SelectedDesignerFailure::D78Controller)?
        .ok_or(SelectedDesignerFailure::EquipmentNoActiveControllerSnapshot)?;
    if designer.view_ptr != view {
        return Err(SelectedDesignerFailure::EquipmentControllerViewDiffers);
    }
    let selected_design = read_identity(
        variant,
        EQUIPMENT_VARIANT_VTABLE_RVA,
        EQUIPMENT_VARIANT_TYPE,
    )?;
    let mode =
        read_i32(view + VIEW_MODE).ok_or(SelectedDesignerFailure::EquipmentViewModeUnreadable)?;
    let definition = read_usize(variant + VARIANT_DEFINITION)
        .filter(|pointer| *pointer != 0)
        .ok_or(SelectedDesignerFailure::EquipmentVariantDefinitionNull)?;
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
        return Err(SelectedDesignerFailure::EquipmentSelectionAbsentFromOrder);
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

fn capture_division_selection(view: usize) -> Result<ObservedSelection, SelectedDesignerFailure> {
    module_base()?;
    if view == 0 {
        return Err(SelectedDesignerFailure::DivisionNullView);
    }
    let handler = read_usize(view + DIVISION_HANDLER)
        .filter(|pointer| *pointer != 0)
        .ok_or(SelectedDesignerFailure::DivisionHandlerNull)?;
    let used = read_u32(view + DIVISION_ROW_POOL_USED)
        .ok_or(SelectedDesignerFailure::DivisionRowPoolUsedUnreadable)? as usize;
    if used == 0 || used > MAX_ORDERED_DESIGNS {
        return Err(SelectedDesignerFailure::DivisionRowPoolUsedOutOfBounds);
    }
    let data = read_usize(view + DIVISION_ROW_POOL_DATA)
        .filter(|pointer| *pointer != 0)
        .ok_or(SelectedDesignerFailure::DivisionRowPoolDataNull)?;
    let mut ordered_designs = Vec::with_capacity(used);
    for index in 0..used {
        let row = read_usize(data + index * mem::size_of::<usize>())
            .filter(|pointer| *pointer != 0)
            .ok_or(SelectedDesignerFailure::DivisionRowNull(index))?;
        let template = read_usize(row + DIVISION_ROW_TEMPLATE)
            .filter(|pointer| *pointer != 0)
            .ok_or(SelectedDesignerFailure::DivisionRowTemplateNull(index))?;
        ordered_designs.push(read_identity(
            template,
            DIVISION_TEMPLATE_VTABLE_RVA,
            REFERENCED_DIVISION_TEMPLATE_TYPE,
        )?);
    }
    let selected = read_usize(handler + DIVISION_HANDLER_SELECTED)
        .filter(|pointer| *pointer != 0)
        .ok_or(SelectedDesignerFailure::DivisionSelectedTemplateNull)?;
    let selected_design = read_identity(
        selected,
        DIVISION_TEMPLATE_VTABLE_RVA,
        REFERENCED_DIVISION_TEMPLATE_TYPE,
    )?;
    if read_usize(view + DIVISION_ROW_POOL_DATA) != Some(data)
        || read_u32(view + DIVISION_ROW_POOL_USED) != Some(used as u32)
        || read_usize(handler + DIVISION_HANDLER_SELECTED) != Some(selected)
    {
        return Err(SelectedDesignerFailure::DivisionOrderChangedDuringCapture);
    }
    if !ordered_designs.contains(&selected_design) {
        return Err(SelectedDesignerFailure::DivisionSelectedAbsentFromOrder);
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
) -> Result<ObservedSelection, SelectedDesignerFailure> {
    if window == 0 {
        return Err(SelectedDesignerFailure::VariantUpgradeNullWindow);
    }
    let nested_view = read_usize(window + VARIANT_UPGRADE_NESTED_VIEW)
        .filter(|pointer| *pointer != 0)
        .ok_or(SelectedDesignerFailure::VariantUpgradeNestedViewNull)?;
    if read_usize(nested_view + VARIANT_UPGRADE_NESTED_SELECTED) != Some(variant)
        || read_usize(window + VARIANT_UPGRADE_WINDOW_SELECTED) != Some(variant)
    {
        return Err(SelectedDesignerFailure::VariantUpgradeTransactionMismatch);
    }
    let selected_design = read_identity(
        variant,
        EQUIPMENT_VARIANT_VTABLE_RVA,
        EQUIPMENT_VARIANT_TYPE,
    )?;
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
) -> Result<(SelectedDesignerOwner, SelectedDesignerFamily), SelectedDesignerFailure> {
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
        _ => Err(SelectedDesignerFailure::ClassifierDisagreesWithMode(mode)),
    }
}

fn publish_result(
    scope: SelectedDesignerErrorScope,
    result: Result<ObservedSelection, SelectedDesignerFailure>,
) {
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
        Err(reason) => {
            state.errors.insert(
                scope,
                LatchedError {
                    reason,
                    callback_ordinal,
                },
            );
        }
    }
}

fn publish_error(scope: SelectedDesignerErrorScope, reason: SelectedDesignerFailure) {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.callback_count = state.callback_count.saturating_add(1);
        let callback_ordinal = state.callback_count;
        state.errors.insert(
            scope,
            LatchedError {
                reason,
                callback_ordinal,
            },
        );
    }
}

fn module_base() -> Result<usize, SelectedDesignerFailure> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        Err(SelectedDesignerFailure::ModuleBaseUnavailable)
    } else {
        Ok(base)
    }
}

fn read_identity(
    pointer: usize,
    expected_vtable_rva: usize,
    expected_type: u32,
) -> Result<StableDesignIdentity, SelectedDesignerFailure> {
    let base = module_base()?;
    if pointer == 0 {
        return Err(SelectedDesignerFailure::IdentityPointerNull);
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
        return Err(SelectedDesignerFailure::IdentityChangedDuringCapture);
    }
    let (vtable, object_type, stable_id) = first;
    if vtable != Some(base + expected_vtable_rva) {
        return Err(SelectedDesignerFailure::IdentityVtableGate);
    }
    let object_type = object_type.ok_or(SelectedDesignerFailure::IdentityObjectTypeUnreadable)?;
    let stable_id = stable_id
        .filter(|id| *id != 0)
        .ok_or(SelectedDesignerFailure::IdentityStableIdUnreadableOrZero)?;
    if object_type != expected_type {
        return Err(SelectedDesignerFailure::IdentityObjectTypeDisagreesWithFamily);
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
        ) != 0
            && read == out.len()
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

    #[test]
    fn the_receipt_specs_match_the_installers_own_site_table() {
        let expected: [(u32, &[u8]); HOOK_RECEIPT_COUNT] = [
            (
                DIVISION_VIEW_HANDLER_REPLACE_RVA,
                &DIVISION_VIEW_HANDLER_REPLACE_SENTINEL,
            ),
            (DIVISION_VIEW_DESTROY_RVA, &DIVISION_VIEW_DESTROY_SENTINEL),
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
        assert_eq!(
            names.len(),
            HOOK_RECEIPT_COUNT,
            "receipt names must be distinct"
        );
        let detours = detours();
        assert!(detours.iter().all(|detour| detour.address != 0));
        for (index, spec) in PRODUCTION_HOOK_SPECS.iter().enumerate() {
            assert_eq!(detours[index].logical_name, spec.logical_name);
        }
        production_hook_receipt::bind_detours_by_identity("D85", &PRODUCTION_HOOK_SPECS, detours)
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
        assert_eq!(
            &EQUIPMENT_SELECTION_SENTINEL[..4],
            &[0x48, 0x85, 0xD2, 0x0F]
        );
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
                reason: SelectedDesignerFailure::DivisionNullView,
                callback_ordinal: 9,
            },
        );
        clear_campaign_observations(&mut state);
        assert_eq!(state.callback_count, 0);
        assert!(state.errors.is_empty());
        assert!(state.latest.is_empty());
        assert_eq!(state.latched_error_scope_mask(), 0);
    }

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
                reason: SelectedDesignerFailure::DivisionNullView,
                callback_ordinal,
            },
        );
    }

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

    #[test]
    fn every_family_maps_to_the_site_that_produces_it() {
        use SelectedDesignerErrorScope as Scope;
        for (family, scope) in [
            (SelectedDesignerFamily::Division, Scope::Division),
            (SelectedDesignerFamily::Equipment, Scope::EquipmentDesigner),
            (SelectedDesignerFamily::Tank, Scope::EquipmentDesigner),
            (SelectedDesignerFamily::Plane, Scope::EquipmentDesigner),
            (SelectedDesignerFamily::Ship, Scope::EquipmentDesigner),
            (
                SelectedDesignerFamily::VariantUpgrade,
                Scope::VariantUpgrade,
            ),
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

    static GLOBAL_HOOK_STATE_TESTS: Mutex<()> = Mutex::new(());

    fn lock_global_hook_state() -> std::sync::MutexGuard<'static, ()> {
        GLOBAL_HOOK_STATE_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn reset_global_hook_state() {
        DIVISION_VIEW_RETIREMENT_UNPROVED.store(false, Ordering::Release);
        let state = STATE.get_or_init(|| Mutex::new(HookState::default()));
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = HookState::default();
    }

    fn division_view_teardown_causes() -> (u64, u64) {
        STATE
            .get()
            .and_then(|state| state.lock().ok())
            .map(|state| {
                (
                    state.division_view_handler_replacements,
                    state.division_view_destructions,
                )
            })
            .expect("the hook state is initialized in tests")
    }

    fn mapped_division_view(handler: usize) -> Option<usize> {
        STATE
            .get()
            .and_then(|state| state.lock().ok())
            .and_then(|state| state.division_views_by_handler.get(&handler).copied())
    }

    #[test]
    fn a_success_clears_only_its_own_site() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        publish_error(
            SelectedDesignerErrorScope::Division,
            SelectedDesignerFailure::DivisionSelectedTemplateNull,
        );
        publish_error(
            SelectedDesignerErrorScope::EquipmentDesigner,
            SelectedDesignerFailure::EquipmentAutodesignBoundaryNotCrossed,
        );
        assert_eq!(
            diagnostics().latched_error_scope_mask,
            SelectedDesignerErrorScope::Division.bit()
                | SelectedDesignerErrorScope::EquipmentDesigner.bit(),
            "both sites hold a failure before the success"
        );

        publish_result(
            SelectedDesignerErrorScope::EquipmentDesigner,
            Ok(observed(SelectedDesignerFamily::Tank, 0, 0)),
        );

        let after = diagnostics();
        assert_eq!(
            after.latched_error_scope_mask,
            SelectedDesignerErrorScope::Division.bit(),
            "a success at one site must not declare another site healthy"
        );
        assert_eq!(
            after.latched_error_reason_code,
            SelectedDesignerFailure::DivisionSelectedTemplateNull.code(),
            "the division failure is still the live one, with its own reason"
        );
        assert_eq!(
            after.available_families,
            vec![SelectedDesignerFamily::Tank],
            "and the success committed its observation"
        );

        publish_result(
            SelectedDesignerErrorScope::Division,
            Ok(observed(SelectedDesignerFamily::Division, 0, 0)),
        );
        let cleared = diagnostics();
        assert_eq!(cleared.latched_error_scope_mask, 0);
        assert_eq!(cleared.latched_error_reason_code, 0);
        assert_eq!(cleared.current_error, None);

        reset_global_hook_state();
    }

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

    fn every_failure() -> Vec<SelectedDesignerFailure> {
        use SelectedDesignerFailure as Failure;
        vec![
            Failure::ModuleBaseUnavailable,
            Failure::D78Controller("d78 said so".into()),
            Failure::EquipmentNoActiveControllerSnapshot,
            Failure::EquipmentControllerViewDiffers,
            Failure::EquipmentViewModeUnreadable,
            Failure::EquipmentVariantDefinitionNull,
            Failure::EquipmentSelectionAbsentFromOrder,
            Failure::EquipmentAutodesignBoundaryNotCrossed,
            Failure::DivisionNullView,
            Failure::DivisionHandlerNull,
            Failure::DivisionRowPoolUsedUnreadable,
            Failure::DivisionRowPoolUsedOutOfBounds,
            Failure::DivisionRowPoolDataNull,
            Failure::DivisionRowNull(3),
            Failure::DivisionRowTemplateNull(3),
            Failure::DivisionSelectedTemplateNull,
            Failure::DivisionOrderChangedDuringCapture,
            Failure::DivisionSelectedAbsentFromOrder,
            Failure::DivisionTransactionDidNotCommitTemplate,
            Failure::DivisionNoViewHandlerJoin,
            Failure::DivisionStaleViewHandlerJoin,
            Failure::DivisionViewRetiredDuringCapture,
            Failure::VariantUpgradeNullWindow,
            Failure::VariantUpgradeNestedViewNull,
            Failure::VariantUpgradeTransactionMismatch,
            Failure::IdentityPointerNull,
            Failure::IdentityChangedDuringCapture,
            Failure::IdentityVtableGate,
            Failure::IdentityObjectTypeUnreadable,
            Failure::IdentityStableIdUnreadableOrZero,
            Failure::IdentityObjectTypeDisagreesWithFamily,
            Failure::ClassifierDisagreesWithMode(2),
        ]
    }

    #[test]
    fn every_failure_reason_has_a_distinct_nonzero_frozen_code() {
        let mut seen: BTreeMap<u64, String> = BTreeMap::new();
        for failure in every_failure() {
            let code = failure.code();
            assert_ne!(code, 0, "zero is reserved for no-live-failure: {failure:?}");
            if let Some(previous) = seen.insert(code, format!("{failure:?}")) {
                panic!("code {code} is used by two reasons: {previous} and {failure:?}");
            }
        }
        for (code, expected) in [
            (101u64, "A40 exact-a729 module base is unavailable"),
            (301, "A40 division order supplied null view"),
            (302, "A40 division handler is null/unreadable"),
            (
                304,
                "A40 division row-pool used count is empty/out of bounds",
            ),
            (305, "A40 division row-pool data is null/unreadable"),
            (308, "A40 division selected template is null/unreadable"),
            (
                310,
                "A40 selected division template is absent from controller order",
            ),
            (
                312,
                "A40 division selection has no authenticated view/handler join",
            ),
            (
                313,
                "A40 division selection's joined view no longer names this handler",
            ),
            (
                314,
                "A40 division designer view was retired while this capture read it",
            ),
            (503, "A40 selected object failed exact vtable gate"),
            (505, "A40 selected object stable id unreadable/zero"),
            (506, "A40 selected object type disagrees with owner family"),
        ] {
            let failure = every_failure()
                .into_iter()
                .find(|failure| failure.code() == code)
                .unwrap_or_else(|| panic!("code {code} must still exist"));
            assert_eq!(failure.to_string(), expected, "code {code}");
        }
    }

    struct DivisionFixture {
        words: Vec<u64>,
    }

    impl DivisionFixture {
        const VIEW: usize = 0x0000;
        const HANDLER: usize = 0x3000;
        const ROW_POOL: usize = 0x4000;
        const ROW0: usize = 0x4100;
        const ROW1: usize = 0x4800;
        const TEMPLATE0: usize = 0x4F00;
        const TEMPLATE1: usize = 0x4F40;
        const VTABLE: usize = 0x4F80;
        const OUTSIDER: usize = 0x5000;

        fn new() -> Self {
            let mut fixture = Self {
                words: vec![0u64; 0x6000 / 8],
            };
            let vtable = fixture.at(Self::VTABLE);
            let handler = fixture.at(Self::HANDLER);
            let pool = fixture.at(Self::ROW_POOL);
            let row0 = fixture.at(Self::ROW0);
            let row1 = fixture.at(Self::ROW1);
            let template0 = fixture.at(Self::TEMPLATE0);
            let template1 = fixture.at(Self::TEMPLATE1);

            fixture.write_usize(Self::VIEW + DIVISION_HANDLER, handler);
            fixture.write_usize(Self::VIEW + DIVISION_ROW_POOL_DATA, pool);
            fixture.write_u32(Self::VIEW + DIVISION_ROW_POOL_USED, 2);
            fixture.write_usize(Self::ROW_POOL, row0);
            fixture.write_usize(Self::ROW_POOL + 8, row1);
            fixture.write_usize(Self::ROW0 + DIVISION_ROW_TEMPLATE, template0);
            fixture.write_usize(Self::ROW1 + DIVISION_ROW_TEMPLATE, template1);
            for (template, stable_id) in [(Self::TEMPLATE0, 11u32), (Self::TEMPLATE1, 12)] {
                fixture.write_usize(template, vtable);
                fixture.write_u32(template + OBJECT_TYPE, REFERENCED_DIVISION_TEMPLATE_TYPE);
                fixture.write_u32(template + STABLE_ID, stable_id);
            }
            fixture.write_usize(Self::HANDLER + DIVISION_HANDLER_SELECTED, template0);
            fixture
        }

        fn at(&self, offset: usize) -> usize {
            self.words.as_ptr() as usize + offset
        }

        fn write_usize(&mut self, offset: usize, value: usize) {
            assert_eq!(offset % 8, 0, "staged pointers stay word aligned");
            self.words[offset / 8] = value as u64;
        }

        fn write_u32(&mut self, offset: usize, value: u32) {
            let word = offset / 8;
            let shift = (offset % 8) * 8;
            let mask = !(0xffff_ffffu64 << shift);
            self.words[word] = (self.words[word] & mask) | ((value as u64) << shift);
        }

        fn module_base(&self) -> usize {
            self.at(Self::VTABLE)
                .wrapping_sub(DIVISION_TEMPLATE_VTABLE_RVA)
        }

        fn view(&self) -> usize {
            self.at(Self::VIEW)
        }
    }

    #[test]
    fn every_division_capture_arm_reports_its_own_reason_code() {
        let _serial = lock_global_hook_state();
        let fixture = DivisionFixture::new();
        let previous_base = MODULE_BASE.swap(fixture.module_base(), Ordering::AcqRel);

        let captured = capture_division_selection(fixture.view())
            .expect("a fully staged division view is capturable");
        assert_eq!(captured.family, SelectedDesignerFamily::Division);
        assert_eq!(captured.ordered_designs.len(), 2);
        assert_eq!(captured.selected_design.stable_id, 11);

        let mut failures: Vec<(&str, u64)> = Vec::new();
        let mut case = |label: &'static str, edit: &dyn Fn(&mut DivisionFixture)| {
            let mut perturbed = DivisionFixture::new();
            edit(&mut perturbed);
            let base = MODULE_BASE.swap(perturbed.module_base(), Ordering::AcqRel);
            let outcome = capture_division_selection(perturbed.view());
            MODULE_BASE.store(base, Ordering::Release);
            failures.push((
                label,
                outcome.err().map(|failure| failure.code()).unwrap_or(0),
            ));
        };

        case("null handler", &|fixture| {
            fixture.write_usize(DivisionFixture::VIEW + DIVISION_HANDLER, 0)
        });
        case("empty row pool", &|fixture| {
            fixture.write_u32(DivisionFixture::VIEW + DIVISION_ROW_POOL_USED, 0)
        });
        case("row pool used out of bounds", &|fixture| {
            fixture.write_u32(
                DivisionFixture::VIEW + DIVISION_ROW_POOL_USED,
                MAX_ORDERED_DESIGNS as u32 + 1,
            )
        });
        case("null row pool data", &|fixture| {
            fixture.write_usize(DivisionFixture::VIEW + DIVISION_ROW_POOL_DATA, 0)
        });
        case("null row", &|fixture| {
            fixture.write_usize(DivisionFixture::ROW_POOL + 8, 0)
        });
        case("null row template", &|fixture| {
            fixture.write_usize(DivisionFixture::ROW1 + DIVISION_ROW_TEMPLATE, 0)
        });
        case("template fails the vtable gate", &|fixture| {
            let wrong = fixture.at(DivisionFixture::VTABLE) + 8;
            fixture.write_usize(DivisionFixture::TEMPLATE1, wrong)
        });
        case("template stable id is zero", &|fixture| {
            fixture.write_u32(DivisionFixture::TEMPLATE1 + STABLE_ID, 0)
        });
        case("template object type disagrees", &|fixture| {
            fixture.write_u32(
                DivisionFixture::TEMPLATE1 + OBJECT_TYPE,
                REFERENCED_DIVISION_TEMPLATE_TYPE + 1,
            )
        });
        case("nothing is selected", &|fixture| {
            fixture.write_usize(DivisionFixture::HANDLER + DIVISION_HANDLER_SELECTED, 0)
        });
        case("the selection is absent from the order", &|fixture| {
            let outsider = fixture.at(DivisionFixture::OUTSIDER);
            let vtable = fixture.at(DivisionFixture::VTABLE);
            fixture.write_usize(DivisionFixture::OUTSIDER, vtable);
            fixture.write_u32(
                DivisionFixture::OUTSIDER + OBJECT_TYPE,
                REFERENCED_DIVISION_TEMPLATE_TYPE,
            );
            fixture.write_u32(DivisionFixture::OUTSIDER + STABLE_ID, 99);
            fixture.write_usize(
                DivisionFixture::HANDLER + DIVISION_HANDLER_SELECTED,
                outsider,
            )
        });

        let null_view = capture_division_selection(0)
            .err()
            .map(|failure| failure.code());
        MODULE_BASE.store(0, Ordering::Release);
        let unbound = capture_division_selection(fixture.view())
            .err()
            .map(|failure| failure.code());
        MODULE_BASE.store(previous_base, Ordering::Release);

        assert_eq!(
            null_view,
            Some(SelectedDesignerFailure::DivisionNullView.code()),
            "a null view is its own reason, not a row-pool complaint"
        );
        assert_eq!(
            unbound,
            Some(SelectedDesignerFailure::ModuleBaseUnavailable.code()),
            "an unbound module base is its own reason, not a division defect"
        );

        let expected = [
            ("null handler", 302u64),
            ("empty row pool", 304),
            ("row pool used out of bounds", 304),
            ("null row pool data", 305),
            ("null row", 306),
            ("null row template", 307),
            ("template fails the vtable gate", 503),
            ("template stable id is zero", 505),
            ("template object type disagrees", 506),
            ("nothing is selected", 308),
            ("the selection is absent from the order", 310),
        ];
        assert_eq!(failures.len(), expected.len());
        for ((label, actual), (expected_label, expected_code)) in
            failures.iter().zip(expected.iter())
        {
            assert_eq!(label, expected_label);
            assert_eq!(
                actual, expected_code,
                "{label}: a null selected template must refuse under its own code"
            );
        }
    }

    #[test]
    fn an_order_callback_registers_the_view_before_a_selection_commits() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        let mut fixture = DivisionFixture::new();
        fixture.write_usize(DivisionFixture::HANDLER + DIVISION_HANDLER_SELECTED, 0);
        let previous_base = MODULE_BASE.swap(fixture.module_base(), Ordering::AcqRel);

        let handler = fixture.at(DivisionFixture::HANDLER);
        let template0 = fixture.at(DivisionFixture::TEMPLATE0);

        unsafe { hook_division_order(fixture.view()) };
        let after_order = diagnostics();
        assert_eq!(
            after_order.division_view_registrations, 1,
            "the order callback records the join key even with nothing selected"
        );
        assert_eq!(
            after_order.latched_error_reason_code,
            SelectedDesignerFailure::DivisionSelectedTemplateNull.code(),
            "the capture still refuses -- registration is not conditioned on it"
        );
        assert!(
            after_order.available_families.is_empty(),
            "and nothing is committed yet"
        );

        fixture.write_usize(
            DivisionFixture::HANDLER + DIVISION_HANDLER_SELECTED,
            template0,
        );
        unsafe { hook_division_selection(handler, template0) };

        let after_selection = diagnostics();
        assert_eq!(
            after_selection.available_families,
            vec![SelectedDesignerFamily::Division],
            "the selection JOINS the registered view and commits the observation"
        );
        assert_ne!(
            after_selection.latched_error_reason_code,
            SelectedDesignerFailure::DivisionNoViewHandlerJoin.code(),
            "the join no longer misses"
        );
        assert_eq!(
            after_selection.latched_error_scope_mask, 0,
            "the committing selection clears the division site's earlier refusal"
        );

        MODULE_BASE.store(previous_base, Ordering::Release);
        reset_global_hook_state();
    }

    #[test]
    fn a_stale_view_handler_join_refuses_instead_of_reading_the_wrong_view() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        let mut fixture = DivisionFixture::new();
        fixture.write_usize(DivisionFixture::HANDLER + DIVISION_HANDLER_SELECTED, 0);
        let previous_base = MODULE_BASE.swap(fixture.module_base(), Ordering::AcqRel);
        let handler = fixture.at(DivisionFixture::HANDLER);
        let template0 = fixture.at(DivisionFixture::TEMPLATE0);

        unsafe { hook_division_order(fixture.view()) };
        assert_eq!(
            diagnostics().division_view_registrations,
            1,
            "the join is registered before the engine retires the handler"
        );
        assert!(
            diagnostics().available_families.is_empty(),
            "the order callback committed nothing, so the assertions below are \
             about the stale join alone"
        );

        let replacement = fixture.at(DivisionFixture::OUTSIDER);
        fixture.write_usize(DivisionFixture::VIEW + DIVISION_HANDLER, replacement);
        fixture.write_usize(
            DivisionFixture::HANDLER + DIVISION_HANDLER_SELECTED,
            template0,
        );
        fixture.write_usize(
            DivisionFixture::OUTSIDER + DIVISION_HANDLER_SELECTED,
            template0,
        );

        unsafe { hook_division_selection(handler, template0) };

        let after = diagnostics();
        assert_eq!(
            after.latched_error_reason_code,
            SelectedDesignerFailure::DivisionStaleViewHandlerJoin.code(),
            "a stale join must refuse under its own code, not read the wrong view"
        );
        assert!(
            after.available_families.is_empty(),
            "and must commit NOTHING: a selection read from a retired view is a \
             confident wrong answer"
        );

        unsafe { hook_division_selection(handler, template0) };
        assert_eq!(
            diagnostics().latched_error_reason_code,
            SelectedDesignerFailure::DivisionNoViewHandlerJoin.code(),
            "the stale entry is forgotten, not left to be hit forever"
        );

        MODULE_BASE.store(previous_base, Ordering::Release);
        reset_global_hook_state();
    }

    /// `hook_division_selection` copies the view out of the map and releases
    #[test]
    fn a_newer_registration_survives_a_stale_removal() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        let handler = 0x1234_0000usize;
        let retired = 0x1111_0000usize;
        let newer = 0x2222_0000usize;

        let mapped = |handler: usize| -> Option<usize> {
            STATE
                .get()
                .and_then(|state| state.lock().ok())
                .and_then(|state| state.division_views_by_handler.get(&handler).copied())
        };

        if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
            state.division_views_by_handler.insert(handler, newer);
        }

        forget_division_view(handler, retired);
        assert_eq!(
            mapped(handler),
            Some(newer),
            "a newer registration must survive a stale removal aimed at the \
             view it replaced"
        );

        forget_division_view(handler, newer);
        assert_eq!(
            mapped(handler),
            None,
            "the entry the proof is actually about is removed"
        );

        reset_global_hook_state();
    }

    /// So the 313 guard passes on a view the engine has released and d85
    #[test]
    fn a_destroyed_view_is_unjoined_even_though_it_still_reads_perfectly() {
        for (label, destroy, expected_causes) in [
            (
                "handler replace",
                hook_division_view_handler_replace as unsafe extern "system" fn(usize),
                (1u64, 0u64),
            ),
            (
                "view destructor",
                hook_division_view_destroy as unsafe extern "system" fn(usize),
                (0u64, 1u64),
            ),
        ] {
            let _serial = lock_global_hook_state();
            reset_global_hook_state();

            let fixture = DivisionFixture::new();
            let previous_base = MODULE_BASE.swap(fixture.module_base(), Ordering::AcqRel);
            let handler = fixture.at(DivisionFixture::HANDLER);
            let template0 = fixture.at(DivisionFixture::TEMPLATE0);

            unsafe { hook_division_order(fixture.view()) };
            unsafe { hook_division_selection(handler, template0) };
            assert_eq!(
                diagnostics().available_families,
                vec![SelectedDesignerFamily::Division],
                "{label}: the join publishes while the view is live, so the \
                 refusal below is caused by the destruction and nothing else"
            );

            unsafe { destroy(fixture.view()) };

            assert_eq!(
                read_usize(fixture.view() + DIVISION_HANDLER),
                Some(handler),
                "{label}: the 313 staleness guard's own premise still holds \
                 after the destruction, so that guard alone would let this \
                 through -- this is the residual, stated as an assertion"
            );
            reset_latched_division_state();
            unsafe { hook_division_selection(handler, template0) };
            let after = diagnostics();
            assert!(
                after.available_families.is_empty(),
                "{label}: a selection joined through a retired view must \
                 refuse, not publish"
            );
            assert_eq!(
                after.latched_error_reason_code,
                SelectedDesignerFailure::DivisionNoViewHandlerJoin.code(),
                "{label}: and it refuses as an honest missing join"
            );
            assert_eq!(
                mapped_division_view(handler),
                None,
                "{label}: the join is retired while the memory is still live, \
                 which is the only moment at which it is answerable"
            );
            assert_eq!(
                division_view_teardown_causes(),
                expected_causes,
                "{label}: this detour ran its own body, not the other's"
            );

            unsafe { hook_division_order(fixture.view()) };
            unsafe { hook_division_selection(handler, template0) };
            assert_eq!(
                diagnostics().available_families,
                vec![SelectedDesignerFamily::Division],
                "{label}: d85 recovers once the engine re-establishes the join"
            );

            MODULE_BASE.store(previous_base, Ordering::Release);
            reset_global_hook_state();
        }
    }

    fn reset_latched_division_state() {
        if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
            state.latest.clear();
            state.errors.clear();
        }
    }

    #[test]
    fn a_capture_that_straddles_a_teardown_is_refused() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        let fixture = DivisionFixture::new();
        let observation = || {
            Ok(ObservedSelection {
                transition_sequence: 0,
                callback_ordinal: 0,
                source_record_identity: "test".into(),
                owner: SelectedDesignerOwner::CountryDivisionDesigner,
                family: SelectedDesignerFamily::Division,
                ordered_designs: Vec::new(),
                selected_design: StableDesignIdentity {
                    object_type: REFERENCED_DIVISION_TEMPLATE_TYPE,
                    stable_id: 1,
                },
            })
        };

        let quiet = observe_division_view_epoch();
        assert!(
            refuse_if_view_retired(quiet, observation()).is_ok(),
            "an undisturbed capture publishes"
        );

        let before = observe_division_view_epoch();
        drop(DivisionViewTeardown::open(
            fixture.view(),
            DivisionViewTeardownCause::ViewDestroyed,
        ));
        assert!(
            refuse_if_view_retired(before, observation()).is_err(),
            "a capture that straddled a completed teardown must refuse"
        );

        let in_flight =
            DivisionViewTeardown::open(fixture.view(), DivisionViewTeardownCause::ViewDestroyed);
        let during = observe_division_view_epoch();
        assert_eq!(
            refuse_if_view_retired(during, observation()).unwrap_err(),
            SelectedDesignerFailure::DivisionViewRetiredDuringCapture,
            "a capture that began while a teardown was already in flight must \
             refuse"
        );
        let straddling = observe_division_view_epoch();
        drop(in_flight);
        assert!(
            refuse_if_view_retired(straddling, observation()).is_err(),
            "a capture whose teardown was still running at both ends must refuse"
        );

        let before = observe_division_view_epoch();
        drop(DivisionViewTeardown::open(
            fixture.view(),
            DivisionViewTeardownCause::ViewDestroyed,
        ));
        assert_eq!(
            refuse_if_view_retired(
                before,
                Err(SelectedDesignerFailure::DivisionSelectedTemplateNull)
            )
            .unwrap_err(),
            SelectedDesignerFailure::DivisionSelectedTemplateNull,
            "an already-refused capture keeps the reason its own arm proved"
        );

        assert!(
            refuse_if_view_retired(None, observation()).is_err(),
            "a capture that cannot sample the epoch must refuse"
        );

        reset_global_hook_state();
    }

    #[test]
    fn a_registration_started_before_a_teardown_does_not_resurrect_the_join() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        let fixture = DivisionFixture::new();
        let previous_base = MODULE_BASE.swap(fixture.module_base(), Ordering::AcqRel);
        let handler = fixture.at(DivisionFixture::HANDLER);

        let before = observe_division_view_epoch();

        drop(DivisionViewTeardown::open(
            fixture.view(),
            DivisionViewTeardownCause::ViewDestroyed,
        ));
        assert_eq!(mapped_division_view(handler), None);

        register_division_view(fixture.view(), before);
        assert_eq!(
            mapped_division_view(handler),
            None,
            "a registration started before a teardown must not resurrect the \
             join it retired"
        );

        let in_flight =
            DivisionViewTeardown::open(fixture.view(), DivisionViewTeardownCause::ViewDestroyed);
        let during = observe_division_view_epoch();
        register_division_view(fixture.view(), during);
        assert_eq!(
            mapped_division_view(handler),
            None,
            "a registration started inside a teardown is dropped too"
        );
        drop(in_flight);

        let quiet = observe_division_view_epoch();
        register_division_view(fixture.view(), quiet);
        assert_eq!(
            mapped_division_view(handler),
            Some(fixture.view()),
            "an undisturbed registration still records the join"
        );

        MODULE_BASE.store(previous_base, Ordering::Release);
        reset_global_hook_state();
    }

    #[test]
    fn an_unrecordable_teardown_refuses_every_later_capture() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        assert!(
            observe_division_view_epoch().is_some(),
            "the epoch is observable to begin with"
        );

        DIVISION_VIEW_RETIREMENT_UNPROVED.store(true, Ordering::Release);
        assert!(
            observe_division_view_epoch().is_none(),
            "a teardown that could not be recorded must refuse every later \
             capture"
        );
        let (epoch, view) = join_division_view(0x1234_0000);
        assert!(
            epoch.is_none() && view.is_none(),
            "and the join lookup refuses with it, rather than answering from a \
             map that may name freed memory"
        );

        reset_global_hook_state();
        assert!(
            observe_division_view_epoch().is_some(),
            "the test harness clears the latch it just set"
        );
    }

    #[test]
    fn a_campaign_reset_does_not_rewind_the_division_view_epoch() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        let fixture = DivisionFixture::new();
        let previous_base = MODULE_BASE.swap(fixture.module_base(), Ordering::AcqRel);
        let handler = fixture.at(DivisionFixture::HANDLER);

        let before = observe_division_view_epoch();
        unsafe { hook_division_order(fixture.view()) };
        assert_eq!(mapped_division_view(handler), Some(fixture.view()));
        unsafe { hook_division_view_destroy(fixture.view()) };

        let mut state = STATE
            .get()
            .expect("hook state is initialized")
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear_campaign_observations(&mut state);
        drop(state);

        assert_eq!(
            observe_division_view_epoch(),
            Some(DivisionViewEpoch {
                retirements: 1,
                teardowns_in_flight: 0,
            }),
            "a campaign reset must not rewind the epoch a capture in flight is \
             comparing against"
        );
        assert!(
            refuse_if_view_retired(
                before,
                Ok(ObservedSelection {
                    transition_sequence: 0,
                    callback_ordinal: 0,
                    source_record_identity: "test".into(),
                    owner: SelectedDesignerOwner::CountryDivisionDesigner,
                    family: SelectedDesignerFamily::Division,
                    ordered_designs: Vec::new(),
                    selected_design: StableDesignIdentity {
                        object_type: REFERENCED_DIVISION_TEMPLATE_TYPE,
                        stable_id: 1,
                    },
                }),
            )
            .is_err(),
            "so the capture that straddled the teardown still refuses"
        );
        assert_eq!(
            mapped_division_view(handler),
            None,
            "the observations themselves are still cleared"
        );

        MODULE_BASE.store(previous_base, Ordering::Release);
        reset_global_hook_state();
    }

    #[test]
    fn an_uncountable_epoch_refuses_instead_of_carrying_on() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();

        let fixture = DivisionFixture::new();

        let open =
            DivisionViewTeardown::open(fixture.view(), DivisionViewTeardownCause::ViewDestroyed);
        if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
            state.division_view_teardowns_in_flight = 0;
        }
        drop(open);

        assert!(
            observe_division_view_epoch().is_none(),
            "an epoch that cannot be counted must refuse, not carry on"
        );

        reset_global_hook_state();

        if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
            state.division_view_teardowns_in_flight = u32::MAX;
        }
        let overflowed =
            DivisionViewTeardown::open(fixture.view(), DivisionViewTeardownCause::ViewDestroyed);
        assert!(
            observe_division_view_epoch().is_none(),
            "an in-flight count that cannot advance must refuse too"
        );
        drop(overflowed);

        reset_global_hook_state();
    }

    /// `selection_batch_report(&release_order)` over all seven sites.
    #[test]
    fn teardown_quiesces_the_readers_before_it_blinds_the_observers() {
        let source = include_str!("a729_hook.rs").replace("\r\n", "\n");
        let body = source
            .split_once("pub fn uninstall_fixture_hooks()")
            .expect("the teardown entry point is present")
            .1
            .split_once("\n}\n")
            .expect("the teardown body is delimited")
            .0;
        let readers = body
            .find("selection_batch_report(selection_batch)")
            .expect("the join-reading callbacks are disabled");
        let observers = body
            .find("selection_batch_report(destruction_batch)")
            .expect("the destruction observers are disabled");
        let wait = body[readers..observers]
            .find("wait_for_quiescence()?;")
            .map(|offset| readers + offset);
        assert!(
            readers < observers,
            "the join-reading callbacks must be disabled before the \
             destruction observers"
        );
        assert!(
            wait.is_some(),
            "the join-reading callbacks must be quiesced BEFORE the \
             destruction observers are disabled"
        );
        assert_ne!(
            readers, observers,
            "the two teardown phases must be two separate batches"
        );
    }

    #[test]
    fn the_latched_reason_code_reaches_the_published_diagnostics() {
        let _serial = lock_global_hook_state();
        reset_global_hook_state();
        assert_eq!(diagnostics().latched_error_reason_code, 0);

        publish_error(
            SelectedDesignerErrorScope::Division,
            SelectedDesignerFailure::DivisionSelectedTemplateNull,
        );
        let after = diagnostics();
        assert_eq!(after.latched_error_reason_code, 308);
        assert_eq!(
            after.current_error.as_deref(),
            Some("A40 division selected template is null/unreadable"),
            "the code and the sentence must describe the same failure"
        );
        assert_eq!(after.error_callback_ordinal, after.callback_count);

        publish_error(
            SelectedDesignerErrorScope::VariantUpgrade,
            SelectedDesignerFailure::VariantUpgradeNullWindow,
        );
        assert_eq!(diagnostics().latched_error_reason_code, 401);

        reset_global_hook_state();
        assert_eq!(
            diagnostics().latched_error_reason_code,
            0,
            "a campaign reset leaves nothing latched, and the unit says so"
        );
    }
}
