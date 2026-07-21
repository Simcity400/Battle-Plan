//! Exact-a729 A34 decision-panel hook set.
//!
//! The detours are fully engineered, but installation remains fail-closed
//! behind the lane's two production gates. A scheduling epoch must bracket one
//! complete engine update: the `CCountryDecisionView` materializer, both alert
//! accumulators, every family materializer, final country-grid insertion, and
//! the shared exact text setter. Only an opaque receipt minted by that epoch
//! can admit a resolved [`DecisionPanelSnapshot`](crate::DecisionPanelSnapshot).
//!
//! The country grid is heterogeneous: category, fixed-header, event, and
//! decision children all pass through the generic grid insert. DPN1 v2 carries
//! every accepted child in exact insertion order with geometry and structural
//! state; an unknown presenter family still rejects the complete epoch.

use crate::{
    validate_snapshot, DecisionGridState, DecisionPanelChild, DecisionPanelSnapshot,
    ExactDecisionPanelSource, PanelChildKind, SelectedPresentStamp, MAX_ROWS,
};
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use runtime_text_inputs_live_lane::a729_hook::{
    install_a729_runtime_text_hooks, register_post_setter_observer,
    unregister_post_setter_observer,
};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

pub const PANEL_MATERIALIZER_RVA: u32 = 0x0171_6270;
pub const ORDERED_INSERT_RVA: u32 = 0x002d_e580;
pub const CAN_TAKE_ACCUMULATOR_RVA: u32 = 0x0188_34b0;
pub const TIMEOUT_ACCUMULATOR_RVA: u32 = 0x0188_39d0;
pub const STANDARD_MATERIALIZER_RVA: u32 = 0x01d6_2e10;
pub const TARGETED_MATERIALIZER_RVA: u32 = 0x01d6_33b0;
pub const TIMED_MATERIALIZER_RVA: u32 = 0x01d6_35e0;
pub const ON_MAP_MATERIALIZER_RVA: u32 = 0x01d6_3320;
pub const CATEGORY_MATERIALIZER_RVA: u32 = 0x01d6_29c0;
pub const EVENT_MATERIALIZER_RVA: u32 = 0x01d6_2fb0;
pub const CATEGORY_OPEN_RESOLVER_RVA: u32 = 0x0171_58b0;
pub const UNTYPED_NAME_RESOLVER_RVA: u32 = 0x0072_09e0;
pub const TARGETED_NAME_RESOLVER_RVA: u32 = 0x0072_0d40;

pub const STANDARD_VTABLE_RVA: usize = 0x02a5_9d40;
pub const TIMED_VTABLE_RVA: usize = 0x02a5_9e98;
pub const TARGETED_VTABLE_RVA: usize = 0x02a5_9ff0;
pub const ON_MAP_VTABLE_RVA: usize = 0x02a5_a148;
pub const CATEGORY_VTABLE_RVA: usize = 0x02a5_97f8;
pub const FIXED_HEADER_VTABLE_RVA: usize = 0x02a5_99b8;
pub const EVENT_VTABLE_RVA: usize = 0x02a5_9ae0;

const DECISION_DEF_TOKEN: usize = 0x120;
const RECORD_DEFINITION: usize = 0x10;
const TEXTBOX_FINAL_STRING: usize = 0x110;
const COUNTRY_VIEW_FROM_UPDATE_THIS: usize = 0x30;
const GAME_STATE_SINGLETON_RVA: usize = 0x331_6e90;
const ACTIVE_COUNTRY_PRIMARY: usize = 0x520;
const ACTIVE_COUNTRY_FALLBACK: usize = 0x524;
const COUNTRY_VIEW_GRID: usize = 0x588;
const COUNTRY_VIEW_CONTAINER: usize = 0x580;
const GUI_CLIP_OWNER: usize = 0x140;
const GUI_CONTENT_ORIGIN: usize = 0x160;
const GUI_CONTENT_EXTENT: usize = 0x168;
const CONTAINER_SCROLL_MIN: usize = 0x1558;
const CONTAINER_SCROLL_MAX: usize = 0x1560;
const CONTAINER_SCROLL_TARGET: usize = 0x15f8;
const CONTAINER_SCROLL_CURRENT: usize = 0x1600;
const MAX_ENGINE_TEXT_BYTES: usize = 16_384;

const NAME_CALLERS: &[u32] = &[
    0x01d6_2b22,
    0x01d6_3174,
    0x01d6_2ecd,
    0x01d6_3448,
    0x01d6_365a,
    0x01d6_3390,
];
const COST_CALLERS: &[u32] = &[0x01d6_39b7, 0x01d6_3a3e, 0x01d6_4549, 0x01d6_45d1];
const TIMER_CALLERS: &[u32] = &[
    0x01d6_3acb,
    0x01d6_463a,
    0x01d6_5a94,
    0x01d6_4303,
    0x01d6_514f,
    0x01d6_5809,
    0x01d6_32e1,
];

const PANEL_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54,
    0x41,
];
const INSERT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
    0x57,
];
const CAN_TAKE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
    0x57,
];
const TIMEOUT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54,
    0x41,
];
const STANDARD_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x56, 0x48,
    0x83,
];
const TARGETED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
    0x57,
];
const TIMED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54, 0x41,
    0x56,
];
const ON_MAP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x81, 0xec, 0xf0, 0x00, 0x00, 0x00, 0x49, 0x8b,
    0xf8,
];
const CATEGORY_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
    0x56,
];
const EVENT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x68, 0x20, 0x57, 0x41, 0x54, 0x41,
    0x55,
];
const UNTYPED_NAME_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10,
    0x57,
];
const TARGETED_NAME_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10,
    0x55,
];

type PanelFn = unsafe extern "system" fn(usize);
type InsertFn = unsafe extern "system" fn(usize, usize, usize, usize, usize);
type AlertFn = unsafe extern "system" fn(usize, usize) -> usize;
type PresenterFn = unsafe extern "system" fn(usize, usize, usize);
type StructuralPresenterFn = unsafe extern "system" fn(usize, usize);
type CategoryOpenFn = unsafe extern "system" fn(usize, usize) -> u8;
type NameResolverFn = unsafe extern "system" fn(usize, usize, usize) -> usize;

static ORIGINAL_PANEL: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_INSERT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CAN_TAKE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMEOUT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_STANDARD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGETED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ON_MAP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CATEGORY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_EVENT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_UNTYPED_NAME: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGETED_NAME: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static EVIDENCE_MODE: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

thread_local! {
    static ACTIVE_PANEL: RefCell<Option<WorkingPanel>> = const { RefCell::new(None) };
    static ACTIVE_PRESENTER: Cell<usize> = const { Cell::new(0) };
    static ACTIVE_ALERT: RefCell<Option<WorkingAlert>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenterFamily {
    Category,
    FixedHeader,
    Event,
    Standard,
    Targeted,
    Timed,
    OnMap,
}

#[derive(Clone, Debug)]
struct WorkingPresenter {
    family: PresenterFamily,
    presenter: usize,
    definition: usize,
    record: usize,
    decision_id: Option<String>,
    name_texts: Vec<String>,
    cost_texts: Vec<String>,
    timer_texts: Vec<String>,
    expanded: Option<bool>,
}

#[derive(Clone, Debug)]
struct WorkingPanel {
    controller: usize,
    active_country_tag_db_id: u32,
    container: usize,
    grid: usize,
    presenters: BTreeMap<usize, WorkingPresenter>,
    ordered_presenters: Vec<WorkingGridChild>,
    materialization_qpc_ticks: u64,
    qpc_frequency_hz: u64,
    grid_state: Option<DecisionGridState>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct WorkingGridChild {
    presenter: usize,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AlertKind {
    CanTake,
    Timeout,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RawIdentity {
    decision_id: String,
    record: usize,
}

#[derive(Clone, Debug)]
struct WorkingAlert {
    kind: AlertKind,
    members: Vec<RawIdentity>,
}

#[derive(Clone, Debug)]
struct CompletedAlert {
    count: u32,
    members: BTreeSet<RawIdentity>,
}

#[derive(Clone, Debug)]
struct CaptureEpoch {
    frame_hour: u64,
    generation: u64,
    bound_thread_id: Option<u32>,
    panel: Option<WorkingPanel>,
    can_take: Option<CompletedAlert>,
    timeout: Option<CompletedAlert>,
    error: Option<String>,
}

#[derive(Default)]
struct HookState {
    next_generation: u64,
    epoch: Option<CaptureEpoch>,
    pending_receipt: Option<MaterializationReceipt>,
    outstanding_receipt_nonce: Option<u64>,
    latest_snapshot: Option<DecisionPanelSnapshot>,
    latest_frame_hour: Option<u64>,
    materializer_calls: u64,
    insertion_calls: u64,
    text_events: u64,
    can_take_calls: u64,
    timeout_calls: u64,
    current_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookRowWitness {
    pub family: PresenterFamily,
    pub identity: String,
    pub presentation_index: u32,
    pub name_text: String,
    pub cost_texts: Vec<String>,
    pub timer_texts: Vec<String>,
    pub alert_to_take: bool,
    pub alert_timeout: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub expanded: Option<bool>,
    record: usize,
}

/// Opaque, one-shot proof that all producer callbacks occurred in one
/// scheduler-owned epoch. Pointer-bearing witnesses are process-local and are
/// never serialized; only a validated [`DecisionPanelSnapshot`] can cross d87.
#[derive(Clone, Debug)]
pub struct MaterializationReceipt {
    frame_hour: u64,
    source_sequence: u64,
    controller: usize,
    active_country_tag_db_id: u32,
    rows: Vec<HookRowWitness>,
    grid: DecisionGridState,
    materialization_qpc_ticks: u64,
    selected_present: SelectedPresentStamp,
    nonce: u64,
}

/// Pointer-free decision semantics resolved outside the presentation callbacks.
/// The decision id is repeated only as a mechanical join key. Final order,
/// exact rendered text, geometry, and alert membership can only be copied from
/// the opaque hook-minted receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDecisionRowSupplement {
    pub decision_id: String,
    pub category_id: String,
    pub availability: crate::Availability,
    pub lifecycle: crate::Lifecycle,
    pub target: Option<crate::Target>,
    pub cost: Option<crate::Cost>,
    pub remaining_timer_days: Option<i32>,
    pub cooldown_remaining_days: Option<i32>,
    pub timer_text: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDecisionPanelSupplement {
    pub country_tag_db_id: u32,
    pub country_tag: String,
    pub decision_catalog_sha256: String,
    pub children: Vec<ResolvedPanelChildSupplement>,
    pub rows: Vec<ResolvedDecisionRowSupplement>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPanelChildSupplement {
    pub family: PresenterFamily,
    pub identity: String,
    pub name_text: Option<String>,
    pub secondary_text: Option<String>,
    pub expanded: Option<bool>,
}

impl MaterializationReceipt {
    pub fn frame_hour(&self) -> u64 {
        self.frame_hour
    }
    pub fn source_sequence(&self) -> u64 {
        self.source_sequence
    }
    pub fn controller(&self) -> usize {
        self.controller
    }
    pub fn active_country_tag_db_id(&self) -> u32 {
        self.active_country_tag_db_id
    }
    pub fn rows(&self) -> &[HookRowWitness] {
        &self.rows
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub installed: bool,
    pub epoch_open: bool,
    pub pending_receipt: bool,
    pub authenticated_snapshot: bool,
    pub materializer_calls: u64,
    pub insertion_calls: u64,
    pub text_events: u64,
    pub can_take_calls: u64,
    pub timeout_calls: u64,
    pub current_error: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub panel_materializer: usize,
    pub ordered_insert: usize,
    pub can_take_accumulator: usize,
    pub timeout_accumulator: usize,
    pub presenter_materializers: [usize; 4],
    pub structural_materializers: [usize; 2],
    pub alert_name_resolvers: [usize; 2],
}

pub struct A729DecisionPanelHookSource;

impl ExactDecisionPanelSource for A729DecisionPanelHookSource {
    fn is_exact_a729_materializer_source(&self) -> bool {
        activation_enabled() && installed() && !evidence_mode()
    }

    fn read_decision_panel(&self, frame_hour: u64) -> Result<DecisionPanelSnapshot, String> {
        if !self.is_exact_a729_materializer_source() {
            return Err("A34 exact a729 hook set is not production-active".into());
        }
        let state = state().lock().map_err(|_| "A34 hook state mutex poisoned")?;
        if let Some(error) = &state.current_error {
            return Err(error.clone());
        }
        let snapshot = state
            .latest_snapshot
            .clone()
            .ok_or("A34 hook has no authenticated complete panel")?;
        if snapshot.source_sequence == 0 || state.latest_frame_hour != Some(frame_hour) {
            return Err("A34 snapshot does not bind to the requested frame hour".into());
        }
        Ok(snapshot)
    }
}

pub const fn activation_enabled() -> bool {
    crate::PRODUCTION_SCHEDULING_ENABLED && crate::LIVE_FIXTURE_ACCEPTED
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn evidence_mode() -> bool {
    EVIDENCE_MODE.load(Ordering::Acquire)
}

pub fn install() -> Result<InstalledHookAddresses, String> {
    if !activation_enabled() {
        return Err(
            "A34 a729 hook installation is dormant until scheduling and live-fixture gates close"
                .into(),
        );
    }
    install_hook_set(false)
}

/// Explicit harness-only install path. This bypasses only the production
/// scheduling latch; sentinel verification and the exact hook graph are
/// identical. Evidence-mode frames remain inaccessible through the production
/// source trait.
pub fn install_for_evidence() -> Result<InstalledHookAddresses, String> {
    install_hook_set(true)
}

fn install_hook_set(for_evidence: bool) -> Result<InstalledHookAddresses, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "A34 hook lifecycle mutex poisoned".to_string())?;
    if installed() {
        if evidence_mode() != for_evidence {
            return Err("A34 hook set is already installed in a different activation mode".into());
        }
        return addresses(MODULE_BASE.load(Ordering::Acquire));
    }
    if MODULE_BASE.load(Ordering::Acquire) != 0 {
        return Err("A34 prior hook set has not completed quiescent removal".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A34 GetModuleHandleW(NULL) returned null".into());
    }
    let specs = hook_specs(base)?;
    for spec in &specs {
        verify_sentinel(spec.target, spec.sentinel, spec.name)?;
    }
    install_a729_runtime_text_hooks()?;
    register_post_setter_observer(post_setter_observer)?;
    *state().lock().map_err(|_| "A34 hook state mutex poisoned")? = HookState::default();
    MODULE_BASE.store(base, Ordering::Release);

    let mut created = Vec::new();
    for spec in &specs {
        let original = unsafe { MinHook::create_hook(spec.target as *mut c_void, spec.detour) }
            .map_err(|status| format!("A34 {} hook creation failed: {status}", spec.name));
        let original = match original {
            Ok(original) => original,
            Err(error) => {
                unregister_post_setter_observer(post_setter_observer);
                let cleanup = rollback(&created)
                    .err()
                    .map(|cleanup| format!("; rollback failed: {cleanup}"))
                    .unwrap_or_default();
                if cleanup.is_empty() {
                    clear_originals();
                    MODULE_BASE.store(0, Ordering::Release);
                }
                return Err(format!("{error}{cleanup}"));
            }
        };
        spec.original.store(original as usize, Ordering::Release);
        created.push(spec.target);
    }
    for spec in &specs {
        if let Err(status) = unsafe { MinHook::enable_hook(spec.target as *mut c_void) } {
            unregister_post_setter_observer(post_setter_observer);
            let cleanup = rollback(&created)
                .err()
                .map(|cleanup| format!("; rollback failed: {cleanup}"))
                .unwrap_or_default();
            if cleanup.is_empty() {
                clear_originals();
                MODULE_BASE.store(0, Ordering::Release);
            }
            return Err(format!(
                "A34 {} hook enable failed: {status}{cleanup}",
                spec.name
            ));
        }
    }
    EVIDENCE_MODE.store(for_evidence, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    addresses(base)
}

pub fn uninstall() -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "A34 hook lifecycle mutex poisoned".to_string())?;
    let needs_disable = installed();
    if !needs_disable && MODULE_BASE.load(Ordering::Acquire) == 0 {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let specs = hook_specs(base)?;
    let mut errors = Vec::new();
    if needs_disable {
        for spec in &specs {
            match unsafe { MinHook::disable_hook(spec.target as *mut c_void) } {
                Ok(())
                | Err(MH_STATUS::MH_ERROR_DISABLED)
                | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                Err(status) => errors.push(format!("disable {}: {status}", spec.name)),
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        INSTALLED.store(false, Ordering::Release);
    }
    unregister_post_setter_observer(post_setter_observer);
    wait_for_quiescence()?;
    for spec in &specs {
        match unsafe { MinHook::remove_hook(spec.target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("remove {}: {status}", spec.name)),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    clear_originals();
    MODULE_BASE.store(0, Ordering::Release);
    EVIDENCE_MODE.store(false, Ordering::Release);
    *state().lock().map_err(|_| "A34 hook state mutex poisoned")? = HookState::default();
    Ok(())
}

/// Open the scheduler-owned collection epoch immediately before the engine UI
/// update. An already-open epoch is an error; partial epochs are never reused.
pub fn begin_capture_epoch(frame_hour: u64) -> Result<u64, String> {
    require_active()?;
    if frame_hour == 0 {
        return Err("A34 capture epoch frame hour is zero".into());
    }
    let mut state = state().lock().map_err(|_| "A34 hook state mutex poisoned")?;
    if state.epoch.is_some() {
        return Err("A34 capture epoch is already open".into());
    }
    state.next_generation = state.next_generation.saturating_add(1).max(1);
    let generation = state.next_generation;
    state.pending_receipt = None;
    state.outstanding_receipt_nonce = None;
    state.epoch = Some(CaptureEpoch {
        frame_hour,
        generation,
        bound_thread_id: None,
        panel: None,
        can_take: None,
        timeout: None,
        error: None,
    });
    Ok(generation)
}

/// Seal only after the engine update returns. Missing panel, alert, insertion,
/// text, family, or thread evidence rejects the complete epoch.
pub fn seal_capture_epoch(generation: u64) -> Result<(), String> {
    Err(format!(
        "A34 seal_capture_epoch({generation}) requires the selected-Present stamp; call seal_capture_epoch_at_selected_present"
    ))
}

pub fn seal_capture_epoch_at_selected_present(
    generation: u64,
    selected_present: SelectedPresentStamp,
) -> Result<(), String> {
    require_active()?;
    let mut state = state().lock().map_err(|_| "A34 hook state mutex poisoned")?;
    let epoch = state.epoch.take().ok_or("A34 capture epoch is not open")?;
    if epoch.generation != generation {
        return fail(&mut state, "A34 capture epoch generation mismatch");
    }
    let receipt = finalize_epoch(epoch, selected_present).map_err(|error| {
        state.current_error = Some(error.clone());
        error
    })?;
    state.current_error = None;
    state.pending_receipt = Some(receipt);
    Ok(())
}

/// Abort one armed evidence epoch without manufacturing a selected-Present
/// stamp or partial receipt. Owner-runnable harnesses use this only to
/// quiescently remove the hook set after an environment/driver failure.
pub fn abort_capture_epoch(generation: u64, reason: &str) -> Result<(), String> {
    require_active()?;
    if reason.trim().is_empty() {
        return Err("A34 capture-epoch abort reason is blank".into());
    }
    let mut state = state().lock().map_err(|_| "A34 hook state mutex poisoned")?;
    let epoch = state
        .epoch
        .take()
        .ok_or("A34 capture epoch is not open")?;
    if epoch.generation != generation {
        state.epoch = Some(epoch);
        return Err("A34 capture-epoch abort generation mismatch".into());
    }
    state.pending_receipt = None;
    state.outstanding_receipt_nonce = None;
    state.current_error = Some(format!("A34 capture epoch aborted: {reason}"));
    Ok(())
}

pub fn take_materialization_receipt() -> Result<MaterializationReceipt, String> {
    require_active()?;
    let mut state = state().lock().map_err(|_| "A34 hook state mutex poisoned")?;
    let receipt = state
        .pending_receipt
        .take()
        .ok_or("A34 has no complete materialization receipt")?;
    state.outstanding_receipt_nonce = Some(receipt.nonce);
    Ok(receipt)
}

/// Admit the pointer-free resolution only when it mechanically agrees with the
/// one-shot hook receipt. Category, lifecycle, target, numeric state, and the
/// on-map portable identity may be resolved by the collector's static/D45
/// joins; order, geometry, exact displayed text, and alert membership cannot.
fn admit_resolved_snapshot(
    receipt: MaterializationReceipt,
    snapshot: DecisionPanelSnapshot,
) -> Result<(), String> {
    require_active()?;
    let mut state = state().lock().map_err(|_| "A34 hook state mutex poisoned")?;
    if state.outstanding_receipt_nonce.take() != Some(receipt.nonce) {
        return fail(&mut state, "A34 materialization receipt is stale or not hook-minted");
    }
    validate_snapshot(&snapshot).map_err(|error| format!("A34 resolved snapshot invalid: {error:?}"))?;
    if snapshot.source_sequence != receipt.source_sequence
        || snapshot.children.len() != receipt.rows.len()
        || snapshot.grid != receipt.grid
        || snapshot.materialization_qpc_ticks != receipt.materialization_qpc_ticks
        || snapshot.selected_present != receipt.selected_present
    {
        return fail(&mut state, "A34 resolved snapshot receipt identity/count mismatch");
    }
    for (child, witness) in snapshot.children.iter().zip(&receipt.rows) {
        if child.presentation_index != witness.presentation_index
            || child.kind != panel_child_kind(witness.family)
            || (!witness.identity.is_empty() && child.identity != witness.identity)
            || child.x != witness.x
            || child.y != witness.y
            || child.width != witness.width
            || child.height != witness.height
            || (!witness.name_text.is_empty()
                && child.name_text.as_deref() != Some(witness.name_text.as_str()))
            || (witness.expanded.is_some() && child.expanded != witness.expanded)
        {
            return fail(&mut state, "A34 resolved child disagrees with hook family/order/geometry/text");
        }
        if !is_decision_family(witness.family) {
            continue;
        }
        let row_index = child
            .decision_row_index
            .ok_or_else(|| "A34 decision child lacks resolved row index".to_string())?;
        let row = snapshot
            .rows
            .get(row_index as usize)
            .ok_or_else(|| "A34 decision child row index is out of range".to_string())?;
        if row.presentation_index != witness.presentation_index
            || row.decision_id != child.identity
            || (!witness.identity.is_empty() && row.decision_id != witness.identity)
            || row.name_text != witness.name_text
            || row.alert_to_take != witness.alert_to_take
            || row.alert_timeout != witness.alert_timeout
        {
            return fail(&mut state, "A34 resolved decision disagrees with hook identity/text/alert witness");
        }
        if let Some(text) = row.cost.as_ref().and_then(|cost| cost.formatted_text.as_ref()) {
            if !witness.cost_texts.contains(text) {
                return fail(&mut state, "A34 resolved cost text lacks a matching setter witness");
            }
        }
        if let Some(text) = row.timer_text.as_ref() {
            if !witness.timer_texts.contains(text) {
                return fail(&mut state, "A34 resolved timer text lacks a matching setter witness");
            }
        }
    }
    state.latest_snapshot = Some(snapshot);
    state.latest_frame_hour = Some(receipt.frame_hour);
    state.current_error = None;
    Ok(())
}

/// Consume the one-shot hook receipt and join portable identities/state. All
/// family, order, geometry, rendered text, and alert fields are constrained by
/// the callback witness. On-map identity is supplied by the exact static/D45
/// join because its presenter source is not a `CDecisionDef`.
pub fn admit_resolved_supplement(
    receipt: MaterializationReceipt,
    supplement: ResolvedDecisionPanelSupplement,
) -> Result<(), String> {
    if supplement.country_tag_db_id == 0
        || supplement.country_tag_db_id != receipt.active_country_tag_db_id
    {
        return Err(
            "A34 resolved supplement country identity disagrees with the hook-owned active country"
                .into(),
        );
    }
    let decision_count = receipt
        .rows
        .iter()
        .filter(|row| is_decision_family(row.family))
        .count();
    if supplement.children.len() != receipt.rows.len() || supplement.rows.len() != decision_count {
        return Err("A34 resolved supplement/receipt child or decision count mismatch".into());
    }
    let available_alert_count: u32 = receipt
        .rows
        .iter()
        .filter(|row| row.alert_to_take)
        .count()
        .try_into()
        .map_err(|_| "A34 available-alert count overflow")?;
    let timeout_alert_count: u32 = receipt
        .rows
        .iter()
        .filter(|row| row.alert_timeout)
        .count()
        .try_into()
        .map_err(|_| "A34 timeout-alert count overflow")?;
    let mut resolved_rows = supplement.rows.into_iter();
    let mut rows = Vec::with_capacity(decision_count);
    let mut children = Vec::with_capacity(receipt.rows.len());
    for (witness, resolved_child) in receipt.rows.iter().zip(supplement.children) {
        if resolved_child.family != witness.family
            || (!witness.identity.is_empty() && resolved_child.identity != witness.identity)
            || (!witness.name_text.is_empty()
                && resolved_child.name_text.as_deref() != Some(witness.name_text.as_str()))
            || (witness.expanded.is_some() && resolved_child.expanded != witness.expanded)
        {
            return Err("A34 structural supplement disagrees with hook witness".into());
        }
        let decision_row_index = if is_decision_family(witness.family) {
            let resolved = resolved_rows
                .next()
                .ok_or("A34 decision supplement exhausted before child order")?;
            if resolved.decision_id.is_empty()
                || (!witness.identity.is_empty() && resolved.decision_id != witness.identity)
            {
                return Err("A34 decision supplement identity disagrees with hook witness".into());
            }
            let row_index = u32::try_from(rows.len())
                .map_err(|_| "A34 decision row index overflow")?;
            rows.push(crate::DecisionRow {
                decision_id: resolved.decision_id.clone(),
                category_id: resolved.category_id,
                presentation_index: witness.presentation_index,
                availability: resolved.availability,
                lifecycle: resolved.lifecycle,
                target: resolved.target,
                cost: resolved.cost,
                remaining_timer_days: resolved.remaining_timer_days,
                cooldown_remaining_days: resolved.cooldown_remaining_days,
                name_text: witness.name_text.clone(),
                timer_text: resolved.timer_text,
                alert_to_take: witness.alert_to_take,
                alert_timeout: witness.alert_timeout,
            });
            Some(row_index)
        } else {
            None
        };
        let identity = if witness.identity.is_empty() {
            resolved_child.identity
        } else {
            witness.identity.clone()
        };
        let name_text = if witness.name_text.is_empty() {
            resolved_child.name_text
        } else {
            Some(witness.name_text.clone())
        };
        let secondary_text = if witness.timer_texts.is_empty() {
            resolved_child.secondary_text
        } else if witness.timer_texts.len() == 1 {
            Some(witness.timer_texts[0].clone())
        } else {
            return Err("A34 structural child has ambiguous secondary text witnesses".into());
        };
        children.push(DecisionPanelChild {
            presentation_index: witness.presentation_index,
            kind: panel_child_kind(witness.family),
            decision_row_index,
            identity,
            name_text,
            secondary_text,
            expanded: witness.expanded.or(resolved_child.expanded),
            x: witness.x,
            y: witness.y,
            width: witness.width,
            height: witness.height,
        });
    }
    if resolved_rows.next().is_some() {
        return Err("A34 decision supplement contains trailing rows".into());
    }
    let snapshot = DecisionPanelSnapshot {
        source_sequence: receipt.source_sequence,
        producer_complete: true,
        country_tag: supplement.country_tag,
        decision_catalog_sha256: supplement.decision_catalog_sha256,
        materialization_qpc_ticks: receipt.materialization_qpc_ticks,
        selected_present: receipt.selected_present.clone(),
        grid: receipt.grid.clone(),
        children,
        available_alert_count,
        timeout_alert_count,
        rows,
    };
    admit_resolved_snapshot(receipt, snapshot)
}

fn panel_child_kind(family: PresenterFamily) -> PanelChildKind {
    match family {
        PresenterFamily::Category => PanelChildKind::Category,
        PresenterFamily::FixedHeader => PanelChildKind::FixedHeader,
        PresenterFamily::Event => PanelChildKind::Event,
        PresenterFamily::Standard
        | PresenterFamily::Targeted
        | PresenterFamily::Timed
        | PresenterFamily::OnMap => PanelChildKind::Decision,
    }
}

/// Evidence-only destructive drain. It bypasses no hook authority: the
/// snapshot must already have consumed a one-shot receipt and is removed from
/// process state before the DPN1 bytes are returned.
pub fn drain_evidence_payload(
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, crate::CarrierError> {
    if !installed() || !evidence_mode() {
        return Err(crate::CarrierError::Source(
            "A34 evidence hook set is not installed".into(),
        ));
    }
    let snapshot = {
        let mut state = state()
            .lock()
            .map_err(|_| crate::CarrierError::Source("A34 hook state mutex poisoned".into()))?;
        if let Some(error) = &state.current_error {
            return Err(crate::CarrierError::Source(error.clone()));
        }
        if state.latest_frame_hour != Some(frame_hour) {
            return Err(crate::CarrierError::Source(
                "A34 evidence snapshot does not bind to the requested frame hour".into(),
            ));
        }
        state.latest_frame_hour = None;
        state
            .latest_snapshot
            .take()
            .ok_or_else(|| crate::CarrierError::Source("A34 has no authenticated evidence snapshot".into()))?
    };
    crate::encode_payload(&crate::DecisionPanelFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags: 0,
        authority: crate::AuthorityState::Exact20Receipt,
        snapshot: Some(snapshot),
    })
}

pub fn diagnostics() -> HookDiagnostics {
    let Ok(state) = state().lock() else {
        return HookDiagnostics {
            installed: installed(),
            current_error: true,
            ..HookDiagnostics::default()
        };
    };
    HookDiagnostics {
        installed: installed(),
        epoch_open: state.epoch.is_some(),
        pending_receipt: state.pending_receipt.is_some(),
        authenticated_snapshot: state.latest_snapshot.is_some(),
        materializer_calls: state.materializer_calls,
        insertion_calls: state.insertion_calls,
        text_events: state.text_events,
        can_take_calls: state.can_take_calls,
        timeout_calls: state.timeout_calls,
        current_error: state.current_error.is_some(),
    }
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
            return Err("A34 hook callbacks did not quiesce after disable".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

unsafe extern "system" fn hook_panel(update_this: usize) {
    let _callback = CallbackGuard::enter();
    let nested = ACTIVE_PANEL.with(|slot| slot.borrow().is_some());
    if nested {
        record_error("A34 nested decision-panel materializer callback");
    } else {
        let controller = update_this.checked_sub(COUNTRY_VIEW_FROM_UPDATE_THIS).unwrap_or(0);
        let container = read_usize(controller.saturating_add(COUNTRY_VIEW_CONTAINER)).unwrap_or(0);
        let grid = read_usize(controller.saturating_add(COUNTRY_VIEW_GRID)).unwrap_or(0);
        let active_country_tag_db_id = read_active_country_tag_db_id().unwrap_or(0);
        ACTIVE_PANEL.with(|slot| {
            *slot.borrow_mut() = Some(WorkingPanel {
                controller,
                active_country_tag_db_id,
                container,
                grid,
                presenters: BTreeMap::new(),
                ordered_presenters: Vec::new(),
                materialization_qpc_ticks: 0,
                qpc_frequency_hz: 0,
                grid_state: None,
                error: (controller == 0
                    || container == 0
                    || grid == 0
                    || active_country_tag_db_id == 0)
                    .then(|| "A34 country-decision view/grid owner is unreadable".into()),
            });
        });
    }
    call_panel(ORIGINAL_PANEL.load(Ordering::Acquire), update_this);
    if !nested {
        ACTIVE_PANEL.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(panel) = slot.as_mut() {
                match (performance_counter(), performance_frequency(), read_grid_state(panel)) {
                    (Some(ticks), Some(frequency), Ok(grid_state)) => {
                        panel.materialization_qpc_ticks = ticks;
                        panel.qpc_frequency_hz = frequency;
                        panel.grid_state = Some(grid_state);
                    }
                    _ => panel.error = Some(
                        "A34 post-materialization QPC/grid state is unreadable".into(),
                    ),
                }
            }
        });
        let panel = ACTIVE_PANEL.with(|slot| slot.borrow_mut().take());
        if let Some(panel) = panel {
            record_panel(panel);
        }
    }
}

unsafe extern "system" fn hook_insert(
    grid: usize,
    presenter: usize,
    coordinate: usize,
    insert_mode: usize,
    insert_flags: usize,
) {
    let _callback = CallbackGuard::enter();
    call_insert(
        ORIGINAL_INSERT.load(Ordering::Acquire),
        grid,
        presenter,
        coordinate,
        insert_mode,
        insert_flags,
    );
    let mut matched_country_grid = false;
    ACTIVE_PANEL.with(|slot| {
        if let Some(panel) = slot.borrow_mut().as_mut() {
            if panel.grid == grid {
                matched_country_grid = true;
                let x = read_i32(coordinate).unwrap_or(i32::MIN);
                let y = read_i32(coordinate.saturating_add(4)).unwrap_or(i32::MIN);
                let width = read_i32(presenter.saturating_add(GUI_CONTENT_EXTENT))
                    .unwrap_or(i32::MIN);
                let height = read_i32(presenter.saturating_add(GUI_CONTENT_EXTENT + 4))
                    .unwrap_or(i32::MIN);
                if x == i32::MIN || y == i32::MIN || width <= 0 || height <= 0 {
                    panel.error = Some("A34 inserted child geometry is unreadable".into());
                }
                panel.ordered_presenters.push(WorkingGridChild {
                    presenter,
                    x,
                    y,
                    width,
                    height,
                });
            }
        }
    });
    if matched_country_grid {
        if let Ok(mut state) = state().lock() {
            state.insertion_calls = state.insertion_calls.saturating_add(1);
        }
    }
}

unsafe extern "system" fn hook_standard(presenter: usize, definition: usize, context: usize) {
    let _callback = CallbackGuard::enter();
    hook_presenter(
        PresenterFamily::Standard,
        presenter,
        definition,
        0,
        context,
        ORIGINAL_STANDARD.load(Ordering::Acquire),
    );
}

unsafe extern "system" fn hook_targeted(presenter: usize, record: usize, context: usize) {
    let _callback = CallbackGuard::enter();
    hook_presenter(
        PresenterFamily::Targeted,
        presenter,
        read_usize(record.saturating_add(RECORD_DEFINITION)).unwrap_or(0),
        record,
        context,
        ORIGINAL_TARGETED.load(Ordering::Acquire),
    );
}

unsafe extern "system" fn hook_timed(presenter: usize, record: usize, context: usize) {
    let _callback = CallbackGuard::enter();
    hook_presenter(
        PresenterFamily::Timed,
        presenter,
        read_usize(record.saturating_add(RECORD_DEFINITION)).unwrap_or(0),
        record,
        context,
        ORIGINAL_TIMED.load(Ordering::Acquire),
    );
}

unsafe extern "system" fn hook_on_map(presenter: usize, source: usize, context: usize) {
    let _callback = CallbackGuard::enter();
    // The on-map source is deliberately retained as a record, not guessed to
    // be a CDecisionDef. An epoch containing this family remains unavailable
    // unless later exact evidence authenticates its stable definition join.
    hook_presenter(
        PresenterFamily::OnMap,
        presenter,
        0,
        source,
        context,
        ORIGINAL_ON_MAP.load(Ordering::Acquire),
    );
}

unsafe extern "system" fn hook_category(presenter: usize, source: usize) {
    let _callback = CallbackGuard::enter();
    hook_structural_presenter(
        PresenterFamily::Category,
        presenter,
        source,
        read_engine_string(source.saturating_add(0x18))
            .and_then(|bytes| String::from_utf8(bytes).ok()),
        ORIGINAL_CATEGORY.load(Ordering::Acquire),
    );
    let category_view = read_usize(presenter.saturating_add(0x50)).unwrap_or(0);
    let expanded = call_category_collapsed(category_view, source).map(|collapsed| !collapsed);
    ACTIVE_PANEL.with(|slot| {
        if let Some(raw) = slot
            .borrow_mut()
            .as_mut()
            .and_then(|panel| panel.presenters.get_mut(&presenter))
        {
            raw.expanded = expanded;
        }
    });
}

unsafe extern "system" fn hook_event(presenter: usize, source: usize) {
    let _callback = CallbackGuard::enter();
    hook_structural_presenter(
        PresenterFamily::Event,
        presenter,
        source,
        read_u32(source).map(|event_id| format!("event_{event_id}")),
        ORIGINAL_EVENT.load(Ordering::Acquire),
    );
}

unsafe fn hook_structural_presenter(
    family: PresenterFamily,
    presenter: usize,
    source: usize,
    identity: Option<String>,
    original: usize,
) {
    ACTIVE_PANEL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(panel) = slot.as_mut() else { return; };
        if presenter == 0 || source == 0 || identity.as_deref().is_none_or(str::is_empty) {
            panel.error = Some("A34 structural presenter identity is unreadable".into());
            return;
        }
        if panel
            .presenters
            .insert(
                presenter,
                WorkingPresenter {
                    family,
                    presenter,
                    definition: source,
                    record: source,
                    decision_id: identity,
                    name_texts: Vec::new(),
                    cost_texts: Vec::new(),
                    timer_texts: Vec::new(),
                    expanded: None,
                },
            )
            .is_some()
        {
            panel.error = Some("A34 duplicate structural presenter materialization".into());
        }
    });
    ACTIVE_PRESENTER.with(|active| active.set(presenter));
    call_structural_presenter(original, presenter, source);
    ACTIVE_PRESENTER.with(|active| active.set(0));
}

unsafe fn hook_presenter(
    family: PresenterFamily,
    presenter: usize,
    definition: usize,
    record: usize,
    context: usize,
    original: usize,
) {
    ACTIVE_PANEL.with(|slot| {
        if let Some(panel) = slot.borrow_mut().as_mut() {
            if panel.presenters.contains_key(&presenter) {
                panel.error = Some("A34 presenter materialized more than once in one epoch".into());
            } else {
                panel.presenters.insert(
                    presenter,
                    WorkingPresenter {
                        family,
                        presenter,
                        definition,
                        record,
                        decision_id: decision_token(definition),
                        name_texts: Vec::new(),
                        cost_texts: Vec::new(),
                        timer_texts: Vec::new(),
                        expanded: None,
                    },
                );
            }
        }
    });
    let previous = ACTIVE_PRESENTER.with(|active| active.replace(presenter));
    call_presenter(original, presenter, if record == 0 { definition } else { record }, context);
    ACTIVE_PRESENTER.with(|active| active.set(previous));
}

unsafe extern "system" fn hook_can_take(status: usize, output: usize) -> usize {
    let _callback = CallbackGuard::enter();
    hook_alert(
        AlertKind::CanTake,
        status,
        output,
        ORIGINAL_CAN_TAKE.load(Ordering::Acquire),
    )
}

unsafe extern "system" fn hook_timeout(status: usize, output: usize) -> usize {
    let _callback = CallbackGuard::enter();
    hook_alert(
        AlertKind::Timeout,
        status,
        output,
        ORIGINAL_TIMEOUT.load(Ordering::Acquire),
    )
}

unsafe fn hook_alert(kind: AlertKind, status: usize, output: usize, original: usize) -> usize {
    let nested = ACTIVE_ALERT.with(|slot| slot.borrow().is_some());
    if nested {
        record_error("A34 nested alert accumulator callback");
    } else {
        ACTIVE_ALERT.with(|slot| {
            *slot.borrow_mut() = Some(WorkingAlert {
                kind,
                members: Vec::new(),
            })
        });
    }
    let count = call_alert(original, status, output);
    if !nested {
        if let Some(alert) = ACTIVE_ALERT.with(|slot| slot.borrow_mut().take()) {
            record_alert(alert, count);
        }
    }
    count
}

unsafe extern "system" fn hook_untyped_name(definition: usize, output: usize, context: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let result = call_name(
        ORIGINAL_UNTYPED_NAME.load(Ordering::Acquire),
        definition,
        output,
        context,
    );
    record_alert_member(definition, 0);
    result
}

unsafe extern "system" fn hook_targeted_name(record: usize, output: usize, context: usize) -> usize {
    let _callback = CallbackGuard::enter();
    let result = call_name(
        ORIGINAL_TARGETED_NAME.load(Ordering::Acquire),
        record,
        output,
        context,
    );
    let definition = read_usize(record.saturating_add(RECORD_DEFINITION)).unwrap_or(0);
    record_alert_member(definition, record);
    result
}

unsafe extern "system" fn post_setter_observer(_: usize, setter_widget: usize, caller_rva: u32) {
    let presenter = ACTIVE_PRESENTER.with(Cell::get);
    if presenter == 0 {
        return;
    }
    let Some(bytes) = read_engine_string(setter_widget.saturating_add(TEXTBOX_FINAL_STRING)) else {
        mark_panel_error("A34 presenter text setter output is unreadable");
        return;
    };
    let Ok(text) = String::from_utf8(bytes) else {
        mark_panel_error("A34 presenter text setter output is not UTF-8");
        return;
    };
    if text.is_empty() || text.len() > MAX_ENGINE_TEXT_BYTES || text.as_bytes().contains(&0) {
        mark_panel_error("A34 presenter text setter output violates text bounds");
        return;
    }
    ACTIVE_PANEL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(panel) = slot.as_mut() else {
            return;
        };
        let Some(row) = panel.presenters.get_mut(&presenter) else {
            panel.error = Some("A34 text setter presenter was not materializer-registered".into());
            return;
        };
        if NAME_CALLERS.contains(&caller_rva) {
            row.name_texts.push(text);
        } else if COST_CALLERS.contains(&caller_rva) {
            row.cost_texts.push(text);
        } else if TIMER_CALLERS.contains(&caller_rva) {
            row.timer_texts.push(text);
        }
    });
    if let Ok(mut state) = state().lock() {
        state.text_events = state.text_events.saturating_add(1);
    }
}

fn finalize_epoch(
    epoch: CaptureEpoch,
    selected_present: SelectedPresentStamp,
) -> Result<MaterializationReceipt, String> {
    if let Some(error) = epoch.error.as_ref() {
        return Err(error.clone());
    }
    let thread_id = epoch
        .bound_thread_id
        .ok_or("A34 epoch observed no authenticated callback thread")?;
    if thread_id == 0 {
        return Err("A34 epoch callback thread is zero".into());
    }
    let panel = epoch.panel.ok_or("A34 epoch omitted the panel materializer")?;
    if let Some(error) = panel.error.as_ref() {
        return Err(error.clone());
    }
    if panel.controller == 0 {
        return Err("A34 panel materializer controller is null".into());
    }
    if panel.grid == 0 {
        return Err("A34 panel materializer country grid is null".into());
    }
    if panel.container == 0
        || panel.materialization_qpc_ticks == 0
        || panel.qpc_frequency_hz == 0
        || panel.qpc_frequency_hz != selected_present.qpc_frequency_hz
        || selected_present.frame_hour != epoch.frame_hour
        || selected_present.selected_present_qpc_ticks == 0
        || panel.materialization_qpc_ticks > selected_present.selected_present_qpc_ticks
        || !lower_hex_sha256(&selected_present.selected_present_receipt_sha256)
    {
        return Err("A34 materialization/selected-Present receipt join rejected".into());
    }
    if panel.ordered_presenters.len() > MAX_ROWS {
        return Err("A34 ordered presenter count exceeds bound".into());
    }
    let can_take = epoch
        .can_take
        .ok_or("A34 epoch omitted the can-take accumulator")?;
    let timeout = epoch
        .timeout
        .ok_or("A34 epoch omitted the timeout accumulator")?;
    if can_take.count as usize != can_take.members.len()
        || timeout.count as usize != timeout.members.len()
    {
        return Err("A34 alert accumulator count/member mismatch".into());
    }
    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(panel.ordered_presenters.len());
    for (index, child) in panel.ordered_presenters.iter().enumerate() {
        if !seen.insert(child.presenter) {
            return Err("A34 final insertion order contains a duplicate presenter".into());
        }
        let family = presenter_family(child.presenter)
            .ok_or("A34 inserted child has an unknown presenter vtable")?;
        let raw = panel.presenters.get(&child.presenter);
        if let Some(raw) = raw {
            validate_presenter_vtable(raw)?;
            if raw.family != family {
                return Err("A34 inserted child family disagrees with materializer".into());
            }
        } else if family != PresenterFamily::FixedHeader {
            return Err("A34 inserted child lacks its family materializer witness".into());
        }
        let identity = raw
            .and_then(|value| value.decision_id.clone())
            .unwrap_or_else(|| {
                if family == PresenterFamily::FixedHeader {
                    "event_header".into()
                } else {
                    String::new()
                }
            });
        let decision_family = is_decision_family(family);
        if decision_family && family != PresenterFamily::OnMap && identity.is_empty() {
            return Err("A34 inserted decision presenter lacks an exact decision token".into());
        }
        if raw.is_some_and(|value| {
            (decision_family || matches!(family, PresenterFamily::Category | PresenterFamily::Event))
                && value.name_texts.len() != 1
        }) {
            return Err("A34 inserted presenter lacks exactly one name setter witness".into());
        }
        let record = raw.map_or(0, |value| value.record);
        let alert_to_take = can_take
            .members
            .iter()
            .any(|member| !identity.is_empty() && identity_matches(member, &identity, record));
        let alert_timeout = timeout
            .members
            .iter()
            .any(|member| !identity.is_empty() && identity_matches(member, &identity, record));
        rows.push(HookRowWitness {
            family,
            identity,
            presentation_index: u32::try_from(index)
                .map_err(|_| "A34 presentation index overflow")?,
            name_text: raw
                .and_then(|value| value.name_texts.first())
                .cloned()
                .unwrap_or_default(),
            cost_texts: raw.map_or_else(Vec::new, |value| dedupe_text(&value.cost_texts)),
            timer_texts: raw.map_or_else(Vec::new, |value| dedupe_text(&value.timer_texts)),
            alert_to_take,
            alert_timeout,
            x: child.x,
            y: child.y,
            width: child.width,
            height: child.height,
            expanded: raw.and_then(|value| value.expanded),
            record,
        });
    }
    for identity in can_take.members.iter().chain(&timeout.members) {
        if rows
            .iter()
            .filter(|row| identity_matches(identity, &row.identity, row.record))
            .count()
            != 1
        {
            return Err("A34 alert member does not uniquely join final insertion order".into());
        }
    }
    Ok(MaterializationReceipt {
        frame_hour: epoch.frame_hour,
        source_sequence: epoch.generation,
        controller: panel.controller,
        active_country_tag_db_id: panel.active_country_tag_db_id,
        rows,
        grid: panel
            .grid_state
            .ok_or("A34 post-materialization grid state is absent")?,
        materialization_qpc_ticks: panel.materialization_qpc_ticks,
        selected_present,
        nonce: epoch.generation ^ (panel.controller as u64).rotate_left(17),
    })
}

fn validate_presenter_vtable(raw: &WorkingPresenter) -> Result<(), String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let expected = match raw.family {
        PresenterFamily::Category => CATEGORY_VTABLE_RVA,
        PresenterFamily::FixedHeader => FIXED_HEADER_VTABLE_RVA,
        PresenterFamily::Event => EVENT_VTABLE_RVA,
        PresenterFamily::Standard => STANDARD_VTABLE_RVA,
        PresenterFamily::Targeted => TARGETED_VTABLE_RVA,
        PresenterFamily::Timed => TIMED_VTABLE_RVA,
        PresenterFamily::OnMap => ON_MAP_VTABLE_RVA,
    };
    if raw.presenter == 0 || read_usize(raw.presenter) != base.checked_add(expected) {
        return Err("A34 presenter failed exact family-vtable validation".into());
    }
    Ok(())
}

fn presenter_family(presenter: usize) -> Option<PresenterFamily> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let vtable = read_usize(presenter)?;
    [
        (CATEGORY_VTABLE_RVA, PresenterFamily::Category),
        (FIXED_HEADER_VTABLE_RVA, PresenterFamily::FixedHeader),
        (EVENT_VTABLE_RVA, PresenterFamily::Event),
        (STANDARD_VTABLE_RVA, PresenterFamily::Standard),
        (TARGETED_VTABLE_RVA, PresenterFamily::Targeted),
        (TIMED_VTABLE_RVA, PresenterFamily::Timed),
        (ON_MAP_VTABLE_RVA, PresenterFamily::OnMap),
    ]
    .into_iter()
    .find_map(|(rva, family)| (vtable == base.checked_add(rva)?).then_some(family))
}

fn is_decision_family(family: PresenterFamily) -> bool {
    matches!(
        family,
        PresenterFamily::Standard
            | PresenterFamily::Targeted
            | PresenterFamily::Timed
            | PresenterFamily::OnMap
    )
}

fn record_panel(panel: WorkingPanel) {
    if let Ok(mut state) = state().lock() {
        state.materializer_calls = state.materializer_calls.saturating_add(1);
        if let Some(epoch) = state.epoch.as_mut() {
            if !bind_epoch_thread(epoch) {
                epoch.error = Some("A34 materializer crossed scheduler thread".into());
            } else if epoch.panel.replace(panel).is_some() {
                epoch.error = Some("A34 epoch observed multiple panel materializers".into());
            }
        }
    }
}

fn record_alert(alert: WorkingAlert, count: usize) {
    let kind = alert.kind;
    let completed = (|| {
        let count = u32::try_from(count).map_err(|_| "A34 alert count exceeds u32")?;
        let members = alert.members.into_iter().collect::<BTreeSet<_>>();
        Ok::<_, String>(CompletedAlert { count, members })
    })();
    if let Ok(mut state) = state().lock() {
        match kind {
            AlertKind::CanTake => state.can_take_calls = state.can_take_calls.saturating_add(1),
            AlertKind::Timeout => state.timeout_calls = state.timeout_calls.saturating_add(1),
        }
        let Some(epoch) = state.epoch.as_mut() else {
            return;
        };
        if !bind_epoch_thread(epoch) {
            epoch.error = Some("A34 alert accumulator crossed scheduler thread".into());
            return;
        }
        match completed {
            Ok(completed) => match kind {
                AlertKind::CanTake => {
                    if epoch.can_take.replace(completed).is_some() {
                        epoch.error = Some("A34 epoch observed multiple can-take accumulators".into());
                    }
                }
                AlertKind::Timeout => {
                    if epoch.timeout.replace(completed).is_some() {
                        epoch.error = Some("A34 epoch observed multiple timeout accumulators".into());
                    }
                }
            },
            Err(error) => epoch.error = Some(error),
        }
    }
}

fn record_alert_member(definition: usize, record: usize) {
    let Some(decision_id) = decision_token(definition) else {
        ACTIVE_ALERT.with(|slot| {
            if slot.borrow().is_some() {
                record_error("A34 alert member lacks an exact decision token");
            }
        });
        return;
    };
    ACTIVE_ALERT.with(|slot| {
        if let Some(alert) = slot.borrow_mut().as_mut() {
            alert.members.push(RawIdentity { decision_id, record });
        }
    });
}

fn bind_epoch_thread(epoch: &mut CaptureEpoch) -> bool {
    let current = unsafe { GetCurrentThreadId() };
    match epoch.bound_thread_id {
        Some(bound) => bound == current,
        None => {
            epoch.bound_thread_id = Some(current);
            true
        }
    }
}

fn dedupe_text(values: &[String]) -> Vec<String> {
    values.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect()
}

fn identity_matches(identity: &RawIdentity, decision_id: &str, record: usize) -> bool {
    identity.decision_id == decision_id && (identity.record == 0 || identity.record == record)
}

fn decision_token(definition: usize) -> Option<String> {
    if definition == 0 {
        return None;
    }
    let bytes = read_engine_string(definition.checked_add(DECISION_DEF_TOKEN)?)?;
    let token = String::from_utf8(bytes).ok()?;
    (!token.is_empty()
        && token.len() <= MAX_ENGINE_TEXT_BYTES
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')))
    .then_some(token)
}

fn read_engine_string(address: usize) -> Option<Vec<u8>> {
    let size = read_usize(address.checked_add(0x10)?)?;
    let capacity = read_usize(address.checked_add(0x18)?)?;
    if size > MAX_ENGINE_TEXT_BYTES || capacity < size {
        return None;
    }
    let chars = if capacity < 16 {
        address
    } else {
        read_usize(address)?
    };
    let mut bytes = vec![0; size];
    if size != 0 && !read_exact(chars, &mut bytes) {
        return None;
    }
    Some(bytes)
}

fn mark_panel_error(message: &str) {
    ACTIVE_PANEL.with(|slot| {
        if let Some(panel) = slot.borrow_mut().as_mut() {
            panel.error = Some(message.to_string());
        }
    });
}

fn record_error(message: &str) {
    if let Ok(mut state) = state().lock() {
        if let Some(epoch) = state.epoch.as_mut() {
            epoch.error = Some(message.to_string());
        } else {
            state.current_error = Some(message.to_string());
        }
    }
}

fn require_active() -> Result<(), String> {
    if !activation_enabled() && !evidence_mode() {
        return Err("A34 production activation/live-fixture gate is false and evidence mode is not installed".into());
    }
    if !installed() {
        return Err("A34 exact a729 hook set is not installed".into());
    }
    Ok(())
}

fn fail<T>(state: &mut HookState, message: &str) -> Result<T, String> {
    state.current_error = Some(message.to_string());
    Err(message.to_string())
}

fn state() -> &'static Mutex<HookState> {
    STATE.get_or_init(|| Mutex::new(HookState::default()))
}

struct HookSpec {
    target: usize,
    sentinel: &'static [u8; 16],
    detour: *mut c_void,
    original: &'static AtomicUsize,
    name: &'static str,
}

fn hook_specs(base: usize) -> Result<Vec<HookSpec>, String> {
    if base == 0 {
        return Err("A34 module base is zero".into());
    }
    Ok(vec![
        spec(base, PANEL_MATERIALIZER_RVA, &PANEL_SENTINEL, hook_panel as *mut c_void, &ORIGINAL_PANEL, "panel materializer"),
        spec(base, ORDERED_INSERT_RVA, &INSERT_SENTINEL, hook_insert as *mut c_void, &ORIGINAL_INSERT, "ordered insertion"),
        spec(base, CAN_TAKE_ACCUMULATOR_RVA, &CAN_TAKE_SENTINEL, hook_can_take as *mut c_void, &ORIGINAL_CAN_TAKE, "can-take accumulator"),
        spec(base, TIMEOUT_ACCUMULATOR_RVA, &TIMEOUT_SENTINEL, hook_timeout as *mut c_void, &ORIGINAL_TIMEOUT, "timeout accumulator"),
        spec(base, STANDARD_MATERIALIZER_RVA, &STANDARD_SENTINEL, hook_standard as *mut c_void, &ORIGINAL_STANDARD, "standard presenter"),
        spec(base, TARGETED_MATERIALIZER_RVA, &TARGETED_SENTINEL, hook_targeted as *mut c_void, &ORIGINAL_TARGETED, "targeted presenter"),
        spec(base, TIMED_MATERIALIZER_RVA, &TIMED_SENTINEL, hook_timed as *mut c_void, &ORIGINAL_TIMED, "timed presenter"),
        spec(base, ON_MAP_MATERIALIZER_RVA, &ON_MAP_SENTINEL, hook_on_map as *mut c_void, &ORIGINAL_ON_MAP, "on-map presenter"),
        spec(base, CATEGORY_MATERIALIZER_RVA, &CATEGORY_SENTINEL, hook_category as *mut c_void, &ORIGINAL_CATEGORY, "category presenter"),
        spec(base, EVENT_MATERIALIZER_RVA, &EVENT_SENTINEL, hook_event as *mut c_void, &ORIGINAL_EVENT, "event presenter"),
        spec(base, UNTYPED_NAME_RESOLVER_RVA, &UNTYPED_NAME_SENTINEL, hook_untyped_name as *mut c_void, &ORIGINAL_UNTYPED_NAME, "untargeted alert member"),
        spec(base, TARGETED_NAME_RESOLVER_RVA, &TARGETED_NAME_SENTINEL, hook_targeted_name as *mut c_void, &ORIGINAL_TARGETED_NAME, "targeted alert member"),
    ])
}

fn spec(
    base: usize,
    rva: u32,
    sentinel: &'static [u8; 16],
    detour: *mut c_void,
    original: &'static AtomicUsize,
    name: &'static str,
) -> HookSpec {
    HookSpec {
        target: base + rva as usize,
        sentinel,
        detour,
        original,
        name,
    }
}

fn addresses(base: usize) -> Result<InstalledHookAddresses, String> {
    if base == 0 {
        return Err("A34 installed module base is zero".into());
    }
    Ok(InstalledHookAddresses {
        panel_materializer: base + PANEL_MATERIALIZER_RVA as usize,
        ordered_insert: base + ORDERED_INSERT_RVA as usize,
        can_take_accumulator: base + CAN_TAKE_ACCUMULATOR_RVA as usize,
        timeout_accumulator: base + TIMEOUT_ACCUMULATOR_RVA as usize,
        presenter_materializers: [
            base + STANDARD_MATERIALIZER_RVA as usize,
            base + TARGETED_MATERIALIZER_RVA as usize,
            base + TIMED_MATERIALIZER_RVA as usize,
            base + ON_MAP_MATERIALIZER_RVA as usize,
        ],
        structural_materializers: [
            base + CATEGORY_MATERIALIZER_RVA as usize,
            base + EVENT_MATERIALIZER_RVA as usize,
        ],
        alert_name_resolvers: [
            base + UNTYPED_NAME_RESOLVER_RVA as usize,
            base + TARGETED_NAME_RESOLVER_RVA as usize,
        ],
    })
}

fn verify_sentinel(address: usize, expected: &[u8; 16], name: &str) -> Result<(), String> {
    let mut actual = [0; 16];
    if !read_exact(address, &mut actual) {
        return Err(format!("A34 {name} sentinel unreadable"));
    }
    if &actual != expected {
        return Err(format!("A34 {name} sentinel mismatch"));
    }
    Ok(())
}

fn rollback(targets: &[usize]) -> Result<(), String> {
    let mut errors = Vec::new();
    for target in targets.iter().rev() {
        match unsafe { MinHook::disable_hook(*target as *mut c_void) } {
            Ok(())
            | Err(MH_STATUS::MH_ERROR_DISABLED)
            | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("disable {target:#x}: {status}")),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    wait_for_quiescence()?;
    for target in targets.iter().rev() {
        match unsafe { MinHook::remove_hook(*target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("remove {target:#x}: {status}")),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn clear_originals() {
    for original in [
        &ORIGINAL_PANEL,
        &ORIGINAL_INSERT,
        &ORIGINAL_CAN_TAKE,
        &ORIGINAL_TIMEOUT,
        &ORIGINAL_STANDARD,
        &ORIGINAL_TARGETED,
        &ORIGINAL_TIMED,
        &ORIGINAL_ON_MAP,
        &ORIGINAL_CATEGORY,
        &ORIGINAL_EVENT,
        &ORIGINAL_UNTYPED_NAME,
        &ORIGINAL_TARGETED_NAME,
    ] {
        original.store(0, Ordering::Release);
    }
}

unsafe fn call_panel(original: usize, controller: usize) {
    if original != 0 {
        let original: PanelFn = unsafe { mem::transmute(original) };
        unsafe { original(controller) };
    }
}
unsafe fn call_insert(
    original: usize,
    grid: usize,
    presenter: usize,
    coordinate: usize,
    insert_mode: usize,
    insert_flags: usize,
) {
    if original != 0 {
        let original: InsertFn = unsafe { mem::transmute(original) };
        unsafe { original(grid, presenter, coordinate, insert_mode, insert_flags) };
    }
}
unsafe fn call_alert(original: usize, status: usize, output: usize) -> usize {
    if original == 0 {
        return 0;
    }
    let original: AlertFn = unsafe { mem::transmute(original) };
    unsafe { original(status, output) }
}
unsafe fn call_presenter(original: usize, presenter: usize, source: usize, context: usize) {
    if original != 0 {
        let original: PresenterFn = unsafe { mem::transmute(original) };
        unsafe { original(presenter, source, context) };
    }
}
unsafe fn call_structural_presenter(original: usize, presenter: usize, source: usize) {
    if original != 0 {
        let original: StructuralPresenterFn = unsafe { mem::transmute(original) };
        unsafe { original(presenter, source) };
    }
}
fn call_category_collapsed(category_view: usize, category: usize) -> Option<bool> {
    if category_view == 0 || category == 0 {
        return None;
    }
    let target = MODULE_BASE
        .load(Ordering::Acquire)
        .checked_add(CATEGORY_OPEN_RESOLVER_RVA as usize)?;
    let resolver: CategoryOpenFn = unsafe { mem::transmute(target) };
    Some(unsafe { resolver(category_view, category) != 0 })
}
unsafe fn call_name(original: usize, source: usize, output: usize, context: usize) -> usize {
    if original == 0 {
        return 0;
    }
    let original: NameResolverFn = unsafe { mem::transmute(original) };
    unsafe { original(source, output, context) }
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

fn read_usize(address: usize) -> Option<usize> {
    let mut bytes = [0; mem::size_of::<usize>()];
    read_exact(address, &mut bytes).then(|| usize::from_le_bytes(bytes))
}

fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0; 4];
    read_exact(address, &mut bytes).then(|| u32::from_le_bytes(bytes))
}

fn read_i32(address: usize) -> Option<i32> {
    let mut bytes = [0; 4];
    read_exact(address, &mut bytes).then(|| i32::from_le_bytes(bytes))
}

/// The exact country-view update reads the active country database id from
/// game-state +0x520, falling back to +0x524 when the primary value is not
/// positive (exact-a729 function 0x141716270). Capture that same identity
/// inside the materialization callback so a resolver cannot relabel the panel.
fn read_active_country_tag_db_id() -> Option<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let game_state = read_usize(base.checked_add(GAME_STATE_SINGLETON_RVA)?)?;
    if game_state == 0 {
        return None;
    }
    let primary = read_i32(game_state.checked_add(ACTIVE_COUNTRY_PRIMARY)?)?;
    let selected = if primary > 0 {
        primary
    } else {
        read_i32(game_state.checked_add(ACTIVE_COUNTRY_FALLBACK)?)?
    };
    u32::try_from(selected).ok().filter(|value| *value != 0)
}

fn read_i64(address: usize) -> Option<i64> {
    let mut bytes = [0; 8];
    read_exact(address, &mut bytes).then(|| i64::from_le_bytes(bytes))
}

fn performance_counter() -> Option<u64> {
    let mut value = 0i64;
    (unsafe { QueryPerformanceCounter(&mut value) } != 0 && value > 0).then_some(value as u64)
}

fn performance_frequency() -> Option<u64> {
    let mut value = 0i64;
    (unsafe { QueryPerformanceFrequency(&mut value) } != 0 && value > 0).then_some(value as u64)
}

fn read_grid_state(panel: &WorkingPanel) -> Result<DecisionGridState, String> {
    let clip_owner = read_usize(panel.grid.saturating_add(GUI_CLIP_OWNER))
        .ok_or("A34 country grid clip owner is unreadable")?;
    let clip_left = read_i32(clip_owner.saturating_add(GUI_CONTENT_ORIGIN))
        .ok_or("A34 country grid clip X is unreadable")?;
    let clip_top = read_i32(clip_owner.saturating_add(GUI_CONTENT_ORIGIN + 4))
        .ok_or("A34 country grid clip Y is unreadable")?;
    let clip_width = read_i32(clip_owner.saturating_add(GUI_CONTENT_EXTENT))
        .ok_or("A34 country grid clip width is unreadable")?;
    let clip_height = read_i32(clip_owner.saturating_add(GUI_CONTENT_EXTENT + 4))
        .ok_or("A34 country grid clip height is unreadable")?;
    let clip_right = clip_left
        .checked_add(clip_width)
        .ok_or("A34 country grid clip right overflow")?;
    let clip_bottom = clip_top
        .checked_add(clip_height)
        .ok_or("A34 country grid clip bottom overflow")?;
    Ok(DecisionGridState {
        clip_left,
        clip_top,
        clip_right,
        clip_bottom,
        content_origin_x: read_i32(panel.grid.saturating_add(GUI_CONTENT_ORIGIN))
            .ok_or("A34 country grid content X is unreadable")?,
        content_origin_y: read_i32(panel.grid.saturating_add(GUI_CONTENT_ORIGIN + 4))
            .ok_or("A34 country grid content Y is unreadable")?,
        content_extent_width: read_i32(panel.grid.saturating_add(GUI_CONTENT_EXTENT))
            .ok_or("A34 country grid content width is unreadable")?,
        content_extent_height: read_i32(panel.grid.saturating_add(GUI_CONTENT_EXTENT + 4))
            .ok_or("A34 country grid content height is unreadable")?,
        scroll_min: read_i64(panel.container.saturating_add(CONTAINER_SCROLL_MIN))
            .ok_or("A34 country grid scroll minimum is unreadable")?,
        scroll_max: read_i64(panel.container.saturating_add(CONTAINER_SCROLL_MAX))
            .ok_or("A34 country grid scroll maximum is unreadable")?,
        scroll_target: read_i64(panel.container.saturating_add(CONTAINER_SCROLL_TARGET))
            .ok_or("A34 country grid scroll target is unreadable")?,
        scroll_current: read_i64(panel.container.saturating_add(CONTAINER_SCROLL_CURRENT))
            .ok_or("A34 country grid scroll current is unreadable")?,
    })
}

fn lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hook_topology_is_stable_and_dormant() {
        assert_eq!(PANEL_MATERIALIZER_RVA, 0x0171_6270);
        assert_eq!(ORDERED_INSERT_RVA, 0x002d_e580);
        assert_eq!(CAN_TAKE_ACCUMULATOR_RVA, 0x0188_34b0);
        assert_eq!(TIMEOUT_ACCUMULATOR_RVA, 0x0188_39d0);
        assert!(!activation_enabled());
        assert!(install().unwrap_err().contains("dormant"));
    }

    #[test]
    fn exact20_sentinel_receipts_are_bound() {
        assert_eq!(&PANEL_SENTINEL[..4], &[0x48, 0x89, 0x5c, 0x24]);
        assert_eq!(&INSERT_SENTINEL[..4], &[0x48, 0x89, 0x5c, 0x24]);
        assert_eq!(&UNTYPED_NAME_SENTINEL[12..], &[0x54, 0x24, 0x10, 0x57]);
        assert_eq!(&TARGETED_NAME_SENTINEL[12..], &[0x54, 0x24, 0x10, 0x55]);
    }
}
