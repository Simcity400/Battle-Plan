//! Exact-a729 A34 decision-panel hook set.
//!
//! The detours are fully engineered, but installation remains fail-closed
//! behind the lane's two production gates. A scheduling epoch must bracket one
//! complete engine update: the panel materializer, both alert accumulators,
//! every family materializer, final grid insertion, and the shared exact text
//! setter. Only an opaque receipt minted by that epoch can admit a resolved
//! [`DecisionPanelSnapshot`](crate::DecisionPanelSnapshot).

use crate::{validate_snapshot, DecisionPanelSnapshot, ExactDecisionPanelSource, MAX_ROWS};
use core::ffi::c_void;
use minhook::MinHook;
use runtime_text_inputs_live_lane::a729_hook::{
    install_a729_runtime_text_hooks, register_post_setter_observer,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const PANEL_MATERIALIZER_RVA: u32 = 0x01c4_aa30;
pub const ORDERED_INSERT_RVA: u32 = 0x01c4_8550;
pub const CAN_TAKE_ACCUMULATOR_RVA: u32 = 0x0188_34b0;
pub const TIMEOUT_ACCUMULATOR_RVA: u32 = 0x0188_39d0;
pub const STANDARD_MATERIALIZER_RVA: u32 = 0x01d6_2e10;
pub const TARGETED_MATERIALIZER_RVA: u32 = 0x01d6_33b0;
pub const TIMED_MATERIALIZER_RVA: u32 = 0x01d6_35e0;
pub const ON_MAP_MATERIALIZER_RVA: u32 = 0x01d6_3320;
pub const UNTYPED_NAME_RESOLVER_RVA: u32 = 0x0072_09e0;
pub const TARGETED_NAME_RESOLVER_RVA: u32 = 0x0072_0d40;

pub const STANDARD_VTABLE_RVA: usize = 0x02a5_9d40;
pub const TIMED_VTABLE_RVA: usize = 0x02a5_9e98;
pub const TARGETED_VTABLE_RVA: usize = 0x02a5_9ff0;
pub const ON_MAP_VTABLE_RVA: usize = 0x02a5_a148;

const DECISION_DEF_TOKEN: usize = 0x120;
const RECORD_DEFINITION: usize = 0x10;
const TEXTBOX_FINAL_STRING: usize = 0x110;
const MAX_ENGINE_TEXT_BYTES: usize = 16_384;

const NAME_CALLERS: &[u32] = &[0x01d6_2ecd, 0x01d6_3448, 0x01d6_365a, 0x01d6_3390];
const COST_CALLERS: &[u32] = &[0x01d6_39b7, 0x01d6_3a3e, 0x01d6_4549, 0x01d6_45d1];
const TIMER_CALLERS: &[u32] = &[
    0x01d6_3acb,
    0x01d6_463a,
    0x01d6_5a94,
    0x01d6_4303,
    0x01d6_514f,
    0x01d6_5809,
];

const PANEL_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d,
    0xac,
];
const INSERT_SENTINEL: [u8; 16] = [
    0x48, 0x83, 0xec, 0x38, 0x4c, 0x8b, 0x51, 0x58, 0x45, 0x33, 0xdb, 0x41, 0x8b, 0xc3, 0x44,
    0x89,
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
const UNTYPED_NAME_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10,
    0x57,
];
const TARGETED_NAME_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10,
    0x55,
];

type PanelFn = unsafe extern "system" fn(usize);
type InsertFn = unsafe extern "system" fn(usize, usize);
type AlertFn = unsafe extern "system" fn(usize, usize) -> usize;
type PresenterFn = unsafe extern "system" fn(usize, usize, usize);
type NameResolverFn = unsafe extern "system" fn(usize, usize, usize) -> usize;

static ORIGINAL_PANEL: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_INSERT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CAN_TAKE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMEOUT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_STANDARD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGETED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ON_MAP: AtomicUsize = AtomicUsize::new(0);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenterFamily {
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
}

#[derive(Clone, Debug)]
struct WorkingPanel {
    controller: usize,
    presenters: BTreeMap<usize, WorkingPresenter>,
    ordered_presenters: Vec<usize>,
    error: Option<String>,
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
    pub decision_id: String,
    pub presentation_index: u32,
    pub name_text: String,
    pub cost_texts: Vec<String>,
    pub timer_texts: Vec<String>,
    pub alert_to_take: bool,
    pub alert_timeout: bool,
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
    rows: Vec<HookRowWitness>,
    nonce: u64,
}

/// Pointer-free fields resolved outside the presentation callbacks. Identity,
/// order, exact rendered text, and alert membership are intentionally absent:
/// those fields can only be copied from the opaque hook-minted receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDecisionRowSupplement {
    pub category_id: String,
    pub availability: crate::Availability,
    pub lifecycle: crate::Lifecycle,
    pub target: Option<crate::Target>,
    pub cost: Option<crate::Cost>,
    pub remaining_timer_days: Option<i32>,
    pub cooldown_remaining_days: Option<i32>,
    pub timer_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDecisionPanelSupplement {
    pub country_tag: String,
    pub rows: Vec<ResolvedDecisionRowSupplement>,
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

    let mut created = Vec::new();
    for spec in &specs {
        let original = unsafe { MinHook::create_hook(spec.target as *mut c_void, spec.detour) }
            .map_err(|status| format!("A34 {} hook creation failed: {status}", spec.name));
        let original = match original {
            Ok(original) => original,
            Err(error) => {
                rollback(&created);
                return Err(error);
            }
        };
        spec.original.store(original as usize, Ordering::Release);
        created.push(spec.target);
    }
    MODULE_BASE.store(base, Ordering::Release);
    for spec in &specs {
        if let Err(status) = unsafe { MinHook::enable_hook(spec.target as *mut c_void) } {
            rollback(&created);
            clear_originals();
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("A34 {} hook enable failed: {status}", spec.name));
        }
    }
    EVIDENCE_MODE.store(for_evidence, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    addresses(base)
}

pub fn uninstall() -> Result<(), String> {
    let needs_disable = installed();
    if !needs_disable && MODULE_BASE.load(Ordering::Acquire) == 0 {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let specs = hook_specs(base)?;
    let mut errors = Vec::new();
    if needs_disable {
        for spec in &specs {
            if let Err(status) = unsafe { MinHook::disable_hook(spec.target as *mut c_void) } {
                errors.push(format!("disable {}: {status}", spec.name));
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        INSTALLED.store(false, Ordering::Release);
    }
    wait_for_quiescence()?;
    for spec in &specs {
        if let Err(status) = unsafe { MinHook::remove_hook(spec.target as *mut c_void) } {
            errors.push(format!("remove {}: {status}", spec.name));
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
    require_active()?;
    let mut state = state().lock().map_err(|_| "A34 hook state mutex poisoned")?;
    let epoch = state.epoch.take().ok_or("A34 capture epoch is not open")?;
    if epoch.generation != generation {
        return fail(&mut state, "A34 capture epoch generation mismatch");
    }
    let receipt = finalize_epoch(epoch).map_err(|error| {
        state.current_error = Some(error.clone());
        error
    })?;
    state.current_error = None;
    state.pending_receipt = Some(receipt);
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

/// Admit the pointer-free row resolution only when it mechanically agrees
/// with the one-shot hook receipt. Category, lifecycle, target, and numeric
/// state may be resolved by the collector's existing static/D45 joins; row
/// identity, order, exact displayed text, and both alert memberships cannot.
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
        || snapshot.rows.len() != receipt.rows.len()
    {
        return fail(&mut state, "A34 resolved snapshot receipt identity/count mismatch");
    }
    for (row, witness) in snapshot.rows.iter().zip(&receipt.rows) {
        if row.presentation_index != witness.presentation_index
            || row.decision_id != witness.decision_id
            || row.name_text != witness.name_text
            || row.alert_to_take != witness.alert_to_take
            || row.alert_timeout != witness.alert_timeout
        {
            return fail(&mut state, "A34 resolved row disagrees with hook order/text/alert witness");
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

/// Consume the one-shot hook receipt and join only non-identity state. The
/// caller cannot provide decision ids, presentation indices, exact names, or
/// alert membership; those are reconstructed exclusively from hook witnesses.
pub fn admit_resolved_supplement(
    receipt: MaterializationReceipt,
    supplement: ResolvedDecisionPanelSupplement,
) -> Result<(), String> {
    if supplement.rows.len() != receipt.rows.len() {
        return Err("A34 resolved supplement/receipt row count mismatch".into());
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
    let rows = receipt
        .rows
        .iter()
        .zip(supplement.rows)
        .map(|(witness, resolved)| crate::DecisionRow {
            decision_id: witness.decision_id.clone(),
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
        })
        .collect();
    let snapshot = DecisionPanelSnapshot {
        source_sequence: receipt.source_sequence,
        producer_complete: true,
        country_tag: supplement.country_tag,
        available_alert_count,
        timeout_alert_count,
        rows,
    };
    admit_resolved_snapshot(receipt, snapshot)
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

unsafe extern "system" fn hook_panel(controller: usize) {
    let _callback = CallbackGuard::enter();
    let nested = ACTIVE_PANEL.with(|slot| slot.borrow().is_some());
    if nested {
        record_error("A34 nested decision-panel materializer callback");
    } else {
        ACTIVE_PANEL.with(|slot| {
            *slot.borrow_mut() = Some(WorkingPanel {
                controller,
                presenters: BTreeMap::new(),
                ordered_presenters: Vec::new(),
                error: None,
            });
        });
    }
    call_panel(ORIGINAL_PANEL.load(Ordering::Acquire), controller);
    if !nested {
        let panel = ACTIVE_PANEL.with(|slot| slot.borrow_mut().take());
        if let Some(panel) = panel {
            record_panel(panel);
        }
    }
}

unsafe extern "system" fn hook_insert(controller: usize, presenter: usize) {
    let _callback = CallbackGuard::enter();
    call_insert(ORIGINAL_INSERT.load(Ordering::Acquire), controller, presenter);
    ACTIVE_PANEL.with(|slot| {
        if let Some(panel) = slot.borrow_mut().as_mut() {
            if panel.controller != controller {
                panel.error = Some("A34 ordered insertion controller mismatch".into());
            } else {
                panel.ordered_presenters.push(presenter);
            }
        }
    });
    if let Ok(mut state) = state().lock() {
        state.insertion_calls = state.insertion_calls.saturating_add(1);
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

fn finalize_epoch(epoch: CaptureEpoch) -> Result<MaterializationReceipt, String> {
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
    if panel.ordered_presenters.len() > MAX_ROWS {
        return Err("A34 ordered presenter count exceeds bound".into());
    }
    if panel.presenters.len() != panel.ordered_presenters.len() {
        return Err("A34 family materializer/final insertion cardinality mismatch".into());
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
    for (index, presenter) in panel.ordered_presenters.iter().copied().enumerate() {
        if !seen.insert(presenter) {
            return Err("A34 final insertion order contains a duplicate presenter".into());
        }
        let raw = panel
            .presenters
            .get(&presenter)
            .ok_or("A34 inserted presenter lacks a family materializer witness")?;
        validate_presenter_vtable(raw)?;
        let decision_id = raw
            .decision_id
            .clone()
            .ok_or("A34 inserted presenter lacks an exact decision token")?;
        if raw.name_texts.len() != 1 {
            return Err("A34 inserted presenter lacks exactly one name setter witness".into());
        }
        let alert_to_take = can_take
            .members
            .iter()
            .any(|identity| identity_matches(identity, &decision_id, raw.record));
        let alert_timeout = timeout
            .members
            .iter()
            .any(|identity| identity_matches(identity, &decision_id, raw.record));
        rows.push(HookRowWitness {
            family: raw.family,
            decision_id,
            presentation_index: u32::try_from(index)
                .map_err(|_| "A34 presentation index overflow")?,
            name_text: raw.name_texts[0].clone(),
            cost_texts: dedupe_text(&raw.cost_texts),
            timer_texts: dedupe_text(&raw.timer_texts),
            alert_to_take,
            alert_timeout,
            record: raw.record,
        });
    }
    for identity in can_take.members.iter().chain(&timeout.members) {
        if rows
            .iter()
            .filter(|row| identity_matches(identity, &row.decision_id, row.record))
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
        rows,
        nonce: epoch.generation ^ (panel.controller as u64).rotate_left(17),
    })
}

fn validate_presenter_vtable(raw: &WorkingPresenter) -> Result<(), String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let expected = match raw.family {
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

fn rollback(targets: &[usize]) {
    for target in targets.iter().rev() {
        let _ = unsafe { MinHook::disable_hook(*target as *mut c_void) };
        let _ = unsafe { MinHook::remove_hook(*target as *mut c_void) };
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
unsafe fn call_insert(original: usize, controller: usize, presenter: usize) {
    if original != 0 {
        let original: InsertFn = unsafe { mem::transmute(original) };
        unsafe { original(controller, presenter) };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hook_topology_is_stable_and_dormant() {
        assert_eq!(PANEL_MATERIALIZER_RVA, 0x01c4_aa30);
        assert_eq!(ORDERED_INSERT_RVA, 0x01c4_8550);
        assert_eq!(CAN_TAKE_ACCUMULATOR_RVA, 0x0188_34b0);
        assert_eq!(TIMEOUT_ACCUMULATOR_RVA, 0x0188_39d0);
        assert!(!activation_enabled());
        assert!(install().unwrap_err().contains("dormant"));
    }

    #[test]
    fn exact20_sentinel_receipts_are_bound() {
        assert_eq!(&PANEL_SENTINEL[..4], &[0x40, 0x55, 0x53, 0x56]);
        assert_eq!(&INSERT_SENTINEL[..4], &[0x48, 0x83, 0xec, 0x38]);
        assert_eq!(&UNTYPED_NAME_SENTINEL[12..], &[0x54, 0x24, 0x10, 0x57]);
        assert_eq!(&TARGETED_NAME_SENTINEL[12..], &[0x54, 0x24, 0x10, 0x55]);
    }
}
