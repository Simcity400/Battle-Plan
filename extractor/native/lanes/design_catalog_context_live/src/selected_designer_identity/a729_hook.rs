//! Dormant exact-a729 A40 selected-designer observer.
//!
//! The hook is installable only through an explicit fixture API. Domain 85 is
//! still registry-reserved and the production collector never calls this
//! module. All process pointers are consumed in the callback and reduced to
//! `(object_type, stable_id)` before entering shared state.

use super::{
    collect_fixture_and_encode, ExactSelectedDesignerIdentitySource,
    SelectedDesignerFamily, SelectedDesignerIdentityFrame, SelectedDesignerOwner,
    SelectedDesignerError, StableDesignIdentity, CLASSIFIER_MANIFEST_SHA256,
    DIVISION_CLICK_THUNK_ARTIFACT_SHA256, DIVISION_WRAPPER_ARTIFACT_SHA256,
    EQUIPMENT_VARIANT_TYPE, EXACT_A729_EXECUTABLE_SHA256, MAX_ORDERED_DESIGNS,
    PRODUCER_MANIFEST_SHA256, REFERENCED_DIVISION_TEMPLATE_TYPE,
};
use core::ffi::c_void;
use minhook::MinHook;
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

const VIEW_MODE: usize = 0x3790;
const DIVISION_HANDLER: usize = 0x2B58;
const DIVISION_ROW_POOL_DATA: usize = 0x2BB0;
const DIVISION_ROW_POOL_USED: usize = 0x2BBC;
const DIVISION_ROW_TEMPLATE: usize = 0x528;
const DIVISION_HANDLER_SELECTED: usize = 0x8D0;
const VARIANT_DEFINITION: usize = 0x3F0;
const OBJECT_TYPE: usize = 0x08;
const STABLE_ID: usize = 0x0C;
const DIVISION_TEMPLATE_VTABLE_RVA: usize = 0x0293_8F70;
const EQUIPMENT_VARIANT_VTABLE_RVA: usize = 0x0293_AFB8;

type TwoArgFn = unsafe extern "system" fn(usize, usize);
type OneArgFn = unsafe extern "system" fn(usize);
type PredicateFn = unsafe extern "system" fn(usize) -> u8;

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_EQUIPMENT_SELECTION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DIVISION_ORDER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DIVISION_SELECTION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_VARIANT_UPGRADE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
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
        if let Some(error) = &state.current_error {
            return Err(error.clone());
        }
        let observed = state
            .latest
            .get(&family)
            .ok_or_else(|| format!("A40 has not observed a complete {family:?} selection"))?;
        Ok(frame_from_observed(frame_hour, observed.clone()))
    }
}

pub fn install_for_fixture() -> Result<InstalledHookAddresses, String> {
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
                for rollback in created.iter().rev() {
                    let _ = unsafe { MinHook::remove_hook(*rollback as *mut c_void) };
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
        if let Err(status) = unsafe { MinHook::enable_hook(*target as *mut c_void) } {
            for rollback in created.iter().rev() {
                let _ = unsafe { MinHook::disable_hook(*rollback as *mut c_void) };
                let _ = unsafe { MinHook::remove_hook(*rollback as *mut c_void) };
            }
            clear_originals();
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("A40 hook enable at {target:#x} failed: {status}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(addresses)
}

pub fn uninstall_fixture_hooks() -> Result<(), String> {
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
        addresses.equipment_selection,
    ];
    let mut first_error = None;
    if needs_disable {
        for target in &targets {
            if let Err(status) = unsafe { MinHook::disable_hook(*target as *mut c_void) } {
                first_error.get_or_insert_with(|| format!("A40 hook disable at {target:#x} failed: {status}"));
            }
        }
        if let Some(error) = first_error.as_ref() {
            return Err(error.clone());
        }
        INSTALLED.store(false, Ordering::Release);
    }
    wait_for_quiescence()?;
    for target in targets {
        if let Err(status) = unsafe { MinHook::remove_hook(target as *mut c_void) } {
            first_error.get_or_insert_with(|| format!("A40 hook removal at {target:#x} failed: {status}"));
        }
    }
    clear_originals();
    MODULE_BASE.store(0, Ordering::Release);
    if let Some(error) = first_error {
        Err(error)
    } else {
        if let Some(state) = STATE.get() {
            *state
                .lock()
                .map_err(|_| "A40 hook state mutex poisoned")? = HookState::default();
        }
        Ok(())
    }
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
        if let Some(error) = &state.current_error {
            return Err(SelectedDesignerError::Source(error.clone()));
        }
        state
            .latest
            .remove(&family)
            .ok_or_else(|| SelectedDesignerError::Source(format!("A40 has no undrained {family:?} selection")))?
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
        division_order: base + DIVISION_ORDER_RVA as usize,
        division_selection: base + DIVISION_SELECTION_RVA as usize,
        variant_upgrade_selection: base + VARIANT_UPGRADE_SELECTION_RVA as usize,
    }
}

fn clear_originals() {
    ORIGINAL_EQUIPMENT_SELECTION.store(0, Ordering::Release);
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
    call_two_arg_original(&ORIGINAL_EQUIPMENT_SELECTION, view, variant);
    if !nested {
        publish_result(capture_equipment_selection(view, variant));
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
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
            Some(view) => publish_result(capture_division_selection(view)),
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

fn capture_equipment_selection(view: usize, variant: usize) -> Result<ObservedSelection, String> {
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
    if pointer == 0 || read_usize(pointer) != Some(base + expected_vtable_rva) {
        return Err("A40 selected object failed exact vtable gate".into());
    }
    let object_type = read_u32(pointer + OBJECT_TYPE)
        .ok_or("A40 selected object type unreadable")?;
    let stable_id = read_u32(pointer + STABLE_ID)
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

    #[test]
    fn exact_hook_rvas_and_sentinels_are_frozen() {
        assert_eq!(EQUIPMENT_SELECTION_RVA, 0x0177_15A0);
        assert_eq!(DIVISION_ORDER_RVA, 0x0175_6C40);
        assert_eq!(DIVISION_SELECTION_RVA, 0x0167_AEE0);
        assert_eq!(VARIANT_UPGRADE_SELECTION_RVA, 0x01F5_D420);
        assert_eq!(&EQUIPMENT_SELECTION_SENTINEL[..4], &[0x48, 0x85, 0xD2, 0x0F]);
        assert_eq!(&DIVISION_ORDER_SENTINEL[..4], &[0x40, 0x53, 0x55, 0x56]);
    }

    #[test]
    fn variant_upgrade_starts_unavailable_until_exact_input_callback() {
        let state = HookState::default();
        assert!(!state
            .latest
            .contains_key(&SelectedDesignerFamily::VariantUpgrade));
    }
}
