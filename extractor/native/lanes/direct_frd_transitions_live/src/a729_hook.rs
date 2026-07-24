//! Production a729 MinHook adapter for the authenticated D45/X10 direct-event
//! roots.
//!
//! A named evidence-only environment gate remains available to the owner
//! fixture. Production scheduling remains disabled until that receipt passes. The
//! adapter validates all 12 entry sentinels before creating any hook, calls
//! each trampoline exactly once, and emits legacy focus/research/decision rows
//! only after the original returns.
//! X10 on-action and terminal-effect rows are emitted on entry because the
//! invocation itself is the transient fact that hourly state cannot recover.
//! Raw addresses exist only as callback-local values and private identity-map
//! keys.
//!
//! Row scalar contract:
//! - focus `input_bits`: low 32 bits are pre-copied `*R8`; high 32 bits are
//!   pre-copied `*(i32*)R9` bits;
//! - research emits only for `pre_progress < required && EDX >= required`;
//!   its `input_bits` low 32 bits are entry `EDX`;
//! - decision `input_bits`: zero because no scalar argument is authenticated;
//! - legacy post-return rows have [`HOOK_VOID_RETURN_OBSERVED_BITS`] in
//!   `return_bits` and [`HOOK_SUMMARY_RETURN_OBSERVED`] only in the post
//!   summary;
//! - X10 invocation rows have zero `return_bits` and
//!   [`HOOK_ENTRY_OBSERVED_BITS`] only in the entry summary.

use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::collections::HashMap;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

use crate::{
    DirectEventBatch, DirectEventCarrier, DirectEventKind, DirectEventRow,
    DirectEventStateSummary, OpaqueInstanceId, UnsequencedDirectEventRow,
    D45_V5_DIRECT_EVENT_REQUIRED_ROOT_MASK, ROOT_RVA_FOCUS_COMPLETED_A729,
    ROOT_RVA_DECISION_CANCEL_EFFECT_A729, ROOT_RVA_DECISION_REMOVE_EFFECT_A729,
    ROOT_RVA_DECISION_TIMEOUT_EFFECT_A729, ROOT_RVA_ON_ACTION_INVOKED_A729,
    ROOT_RVA_ORDINARY_DECISION_TAKEN_A729, ROOT_RVA_RESEARCH_COMPLETED_A729,
    ROOT_RVA_TARGETED_DECISION_CANCEL_EFFECT_A729,
    ROOT_RVA_TARGETED_DECISION_REMOVE_EFFECT_A729,
    ROOT_RVA_TARGETED_DECISION_TIMEOUT_EFFECT_A729,
    ROOT_RVA_TARGETED_DECISION_TAKEN_A729, ROOT_RVA_TIMED_DECISION_ACTIVATED_A729,
};

/// Bound to the crate-level exact-build production authority. The separate
/// environment gate is diagnostics-only.
pub const PRODUCTION_ACTIVATION: bool = crate::PRODUCTION_ACTIVATION;

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6e90;
const CURRENT_HOUR_OFFSET: usize = 0x468;
const RESEARCH_PROGRESS_OFFSET: usize = 0x174;
const RESEARCH_DEFINITION_OFFSET: usize = 0x160;
const RESEARCH_REQUIRED_OFFSET: usize = 0x3dc;
const TECHNOLOGY_DEFINITION_INDEX_OFFSET: usize = 0x08;
const COUNTRY_TAG_DB_ID_OFFSET: usize = 0x08;
const DECISION_STATUS_COUNTRY_OFFSET: usize = 0x08;
const FOCUS_TOKEN_OFFSET: usize = 0x18;
const DECISION_TOKEN_OFFSET: usize = 0x120;
// The MSVC string beginning at +0x20 is only valid for active entries.
const ON_ACTION_ACTIVE_OFFSET: usize = 0x18;
const ON_ACTION_TOKEN_OFFSET: usize = 0x20;
const MAX_PORTABLE_TOKEN_LEN: usize = 192;
const MAX_POINTER_IDENTITIES: usize = 65_536;

pub const HOOK_VOID_RETURN_OBSERVED_BITS: u64 = 1;
pub const HOOK_ENTRY_OBSERVED_BITS: u64 = 1 << 1;
pub const HOOK_SUMMARY_RETURN_OBSERVED: u64 = 1 << 63;
pub const HOOK_SUMMARY_RESEARCH_PROGRESS_PRESENT: u64 = 1;
pub const HOOK_SUMMARY_RESEARCH_REQUIRED_PRESENT: u64 = 1 << 1;
pub const LEGACY_OWNER_FIXTURE_ROOT_MASK: u64 = (1 << 5) - 1;

pub const FOCUS_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
    0x41,
];
pub const RESEARCH_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x81, 0x60, 0x01, 0x00, 0x00, 0x48, 0x8b,
    0xd9,
];
pub const ORDINARY_DECISION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x56, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf2,
    0x48,
];
pub const TARGETED_DECISION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
    0x57,
];
pub const TIMED_DECISION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20,
    0x48,
];
pub const ON_ACTION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18,
    0x55,
];
pub const DECISION_TERMINAL_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x81, 0xec, 0xf0,
    0x00,
];
pub const TARGETED_DECISION_TERMINAL_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
    0x55,
];
pub const TARGETED_DECISION_TIMEOUT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x55, 0x41,
    0x56,
];

type FocusFn = unsafe extern "system" fn(usize, usize, usize, usize);
type ResearchFn = unsafe extern "system" fn(usize, i32);
type OrdinaryDecisionFn = unsafe extern "system" fn(usize, usize);
type TargetedDecisionFn = unsafe extern "system" fn(usize, usize, usize);
type TimedDecisionFn = unsafe extern "system" fn(usize, usize);
type OnActionFn = unsafe extern "system" fn(usize, usize, usize, u8);
type DecisionTerminalFn = unsafe extern "system" fn(usize, usize);
type TargetedDecisionTerminalFn = unsafe extern "system" fn(usize, usize, usize);

static ORIGINAL_FOCUS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RESEARCH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ORDINARY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGETED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ON_ACTION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DECISION_CANCEL: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DECISION_REMOVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DECISION_TIMEOUT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGETED_DECISION_CANCEL: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGETED_DECISION_REMOVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGETED_DECISION_TIMEOUT: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static QUIESCED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicU64 = AtomicU64::new(0);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static EVIDENCE_FIXTURE: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

const OWNER_FIXTURE_ENV: &str = "BATTLE_PLAN_D45_X10_OWNER_FIXTURE";

#[derive(Debug)]
struct HookState {
    carrier: DirectEventCarrier,
    pointer_ids: HashMap<usize, OpaqueInstanceId>,
    callback_count: u64,
    bound_thread_id: Option<u32>,
    last_game_hour_raw: Option<i32>,
    current_error: Option<String>,
    drain_sequence: u64,
    kind_witness_counts: [u64; 12],
}

impl HookState {
    fn fresh() -> Self {
        Self {
            carrier: DirectEventCarrier::new(),
            pointer_ids: HashMap::new(),
            callback_count: 0,
            bound_thread_id: None,
            last_game_hour_raw: None,
            current_error: None,
            drain_sequence: 1,
            kind_witness_counts: [0; 12],
        }
    }

    fn intern(&mut self, pointer: usize, role: &str) -> Result<OpaqueInstanceId, String> {
        if pointer == 0 {
            return Err(format!("D45 direct hook {role} pointer is null"));
        }
        if let Some(id) = self.pointer_ids.get(&pointer) {
            return Ok(*id);
        }
        if self.pointer_ids.len() >= MAX_POINTER_IDENTITIES {
            return Err("D45 direct hook pointer identity table overflow".into());
        }
        let id = self
            .carrier
            .allocate_instance_id()
            .map_err(|error| format!("D45 direct hook identity allocation failed: {error}"))?;
        self.pointer_ids.insert(pointer, id);
        Ok(id)
    }

    fn ephemeral(&mut self, pointer: usize, role: &str) -> Result<OpaqueInstanceId, String> {
        if pointer == 0 {
            return Err(format!("D45 direct hook {role} pointer is null"));
        }
        self.carrier
            .allocate_instance_id()
            .map_err(|error| format!("D45 direct hook ephemeral ID allocation failed: {error}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledRoots {
    pub rvas: [u32; 12],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub installed: bool,
    pub callback_count: u64,
    pub active_callbacks: u64,
    pub pending_rows: usize,
    pub pointer_identity_count: usize,
    pub bound_thread_id: Option<u32>,
    pub last_game_hour_raw: Option<i32>,
    pub current_error: Option<String>,
    pub hook_epoch: u64,
    pub drain_sequence: u64,
    pub kind_witness_counts: [u64; 12],
    pub evidence_fixture: bool,
}

struct CallbackLease;

impl CallbackLease {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackLease {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl HookDiagnostics {
    /// True only after the evidence-only activation has observed every one of
    /// the five legacy focus/research/decision owner roots at least once.
    pub fn legacy_owner_fixture_complete(&self) -> bool {
        self.installed
            && self.evidence_fixture
            && self.hook_epoch != 0
            && self.active_callbacks == 0
            && self.current_error.is_none()
            && self.kind_witness_counts[..5].iter().all(|count| *count != 0)
    }
}

#[derive(Clone, Copy)]
struct PreparedRow {
    kind: DirectEventKind,
    thread_id: u32,
    game_hour_raw: i32,
    owner_instance_id: OpaqueInstanceId,
    subject_instance_id: OpaqueInstanceId,
    argument_instance_id: OpaqueInstanceId,
    owner_key: u64,
    subject_token_hash: u64,
    subject_key: u64,
    input_bits: u64,
    before: DirectEventStateSummary,
}

pub fn install() -> Result<InstalledRoots, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "D45 direct hook lifecycle mutex poisoned".to_string())?;
    let evidence_fixture = !PRODUCTION_ACTIVATION && evidence_activation_enabled();
    if !PRODUCTION_ACTIVATION && !evidence_fixture {
        return Err(
            "D45 a729 direct hooks are not authorized for this build".into(),
        );
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("D45 direct hooks GetModuleHandleW(NULL) returned null".into());
    }
    let targets = targets(base)?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(installed_roots());
    }
    if MODULE_BASE.load(Ordering::Acquire) != 0 {
        return Err("D45 prior hook set has not completed quiescent removal".into());
    }
    for (target, sentinel, name) in target_sentinels(&targets) {
        verify_sentinel(target, sentinel, name)?;
    }

    let hook_epoch = NEXT_HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    if hook_epoch == 0 {
        return Err("D45 direct hook epoch exhausted".into());
    }

    let state = STATE.get_or_init(|| Mutex::new(HookState::fresh()));
    *state
        .lock()
        .map_err(|_| "D45 direct hook state mutex poisoned before install")? = HookState::fresh();

    MODULE_BASE.store(base, Ordering::Release);
    let focus = match unsafe {
        MinHook::create_hook(targets[0] as *mut c_void, hook_focus as *mut c_void)
    } {
        Ok(focus) => focus,
        Err(status) => {
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("D45 focus hook creation failed: {status}"));
        }
    };
    ORIGINAL_FOCUS.store(focus as usize, Ordering::Release);
    if let Err(error) = create_remaining_hooks(&targets) {
        let cleanup = rollback(&targets)
            .err()
            .map(|cleanup| format!("; rollback failed: {cleanup}"))
            .unwrap_or_default();
        if cleanup.is_empty() {
            clear_hook_lifecycle_state();
        }
        return Err(format!("{error}{cleanup}"));
    }
    ACTIVE_HOOK_EPOCH.store(hook_epoch, Ordering::Release);
    EVIDENCE_FIXTURE.store(evidence_fixture, Ordering::Release);
    QUIESCED.store(false, Ordering::Release);
    for (target, name) in targets.into_iter().zip([
        "focus",
        "research",
        "ordinary decision",
        "targeted decision",
        "timed decision",
        "on action",
        "decision cancel effect",
        "decision remove effect",
        "decision timeout effect",
        "targeted decision cancel effect",
        "targeted decision remove effect",
        "targeted decision timeout effect",
    ]) {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            let cleanup = rollback(&targets)
                .err()
                .map(|cleanup| format!("; rollback failed: {cleanup}"))
                .unwrap_or_default();
            if cleanup.is_empty() {
                clear_hook_lifecycle_state();
            }
            return Err(format!("D45 {name} hook enable failed: {status}{cleanup}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(installed_roots())
}

pub fn uninstall() -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "D45 direct hook lifecycle mutex poisoned".to_string())?;
    if !INSTALLED.load(Ordering::Acquire) && MODULE_BASE.load(Ordering::Acquire) == 0 {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let targets = targets(base)?;
    let names = [
        "focus",
        "research",
        "ordinary decision",
        "targeted decision",
        "timed decision",
        "on action",
        "decision cancel effect",
        "decision remove effect",
        "decision timeout effect",
        "targeted decision cancel effect",
        "targeted decision remove effect",
        "targeted decision timeout effect",
    ];
    let mut disable_errors = Vec::new();
    for (target, name) in targets.into_iter().zip(names) {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(())
            | Err(MH_STATUS::MH_ERROR_DISABLED)
            | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => disable_errors.push(format!("disable {name}: {status}")),
        }
    }
    if !disable_errors.is_empty() {
        return Err(disable_errors.join("; "));
    }
    INSTALLED.store(false, Ordering::Release);
    QUIESCED.store(true, Ordering::Release);
    wait_for_callback_quiescence()?;
    let mut remove_errors = Vec::new();
    for (target, name) in targets.into_iter().zip(names) {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => remove_errors.push(format!("remove {name}: {status}")),
        }
    }
    if remove_errors.is_empty() {
        clear_hook_lifecycle_state();
        if let Some(state) = STATE.get() {
            *state
                .lock()
                .map_err(|_| "D45 direct hook state mutex poisoned during uninstall")? =
                HookState::fresh();
        }
        Ok(())
    } else {
        Err(remove_errors.join("; "))
    }
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub const fn activation_enabled() -> bool {
    PRODUCTION_ACTIVATION
}

pub fn evidence_activation_enabled() -> bool {
    std::env::var(OWNER_FIXTURE_ENV).ok().as_deref() == Some("1")
}

pub fn scheduling_enabled() -> bool {
    activation_enabled() || evidence_activation_enabled()
}

pub fn drain_batch() -> Result<DirectEventBatch, String> {
    if !installed() {
        return Err("D45 a729 direct hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("D45 direct hook state is not initialized")?
        .lock()
        .map_err(|_| "D45 direct hook state mutex poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    let rows = state
        .carrier
        .drain_rows()
        .map_err(|error| format!("D45 direct hook typed drain failed: {error}"))?;
    let sequence_cursor = state.carrier.next_sequence();
    let drain_sequence = state.drain_sequence;
    let observed_kind_mask = state
        .kind_witness_counts
        .iter()
        .enumerate()
        .fold(0u64, |mask, (index, count)| {
            if *count == 0 {
                mask
            } else {
                mask | (1u64 << index)
            }
        });
    state.drain_sequence = state
        .drain_sequence
        .checked_add(1)
        .ok_or("D45 direct hook drain sequence exhausted")?;
    Ok(DirectEventBatch {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        drain_sequence,
        installed_root_mask: D45_V5_DIRECT_EVENT_REQUIRED_ROOT_MASK,
        observed_kind_mask,
        sequence_cursor,
        producer_complete: true,
        evidence_fixture: EVIDENCE_FIXTURE.load(Ordering::Acquire),
        rows,
    })
}

pub fn drain_rows() -> Result<Vec<DirectEventRow>, String> {
    Ok(drain_batch()?.rows)
}

pub fn drain_trailer() -> Result<Vec<u8>, String> {
    if !installed() {
        return Err("D45 a729 direct hooks are not installed".into());
    }
    crate::encode_authoritative_trailer(&drain_batch()?)
        .map_err(|error| format!("D45 direct hook trailer drain failed: {error}"))
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics {
            installed: installed(),
            ..HookDiagnostics::default()
        };
    };
    HookDiagnostics {
        installed: installed(),
        callback_count: state.callback_count,
        active_callbacks: ACTIVE_CALLBACKS.load(Ordering::Acquire),
        pending_rows: state.carrier.pending_rows().len(),
        pointer_identity_count: state.pointer_ids.len(),
        bound_thread_id: state.bound_thread_id,
        last_game_hour_raw: state.last_game_hour_raw,
        current_error: state.current_error.clone(),
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        drain_sequence: state.drain_sequence,
        kind_witness_counts: state.kind_witness_counts,
        evidence_fixture: EVIDENCE_FIXTURE.load(Ordering::Acquire),
    }
}

unsafe extern "system" fn hook_focus(
    progress: usize,
    completed_focus: usize,
    country_key: usize,
    context40: usize,
) {
    let _callback_lease = CallbackLease::enter();
    // Cross-family recursion is part of the engine's real transition order.
    // Every callback prepares its own pointer-free row before entering the
    // original, so nested callbacks can be admitted without retaining raw
    // pointers or suppressing a real event.
    let prepared = prepare_focus(progress, completed_focus, country_key, context40);
    unsafe { call_original_focus(progress, completed_focus, country_key, context40) };
    publish(prepared.map(Some));
}

unsafe extern "system" fn hook_research(instance: usize, new_progress: i32) {
    let _callback_lease = CallbackLease::enter();
    let prepared = prepare_research(instance, new_progress);
    unsafe { call_original_research(instance, new_progress) };
    publish(prepared);
}

unsafe extern "system" fn hook_ordinary(status: usize, decision: usize) {
    let _callback_lease = CallbackLease::enter();
    let prepared = prepare_decision_row(
        DirectEventKind::OrdinaryDecisionTaken,
        status,
        decision,
        None,
    );
    unsafe { call_original_ordinary(status, decision) };
    publish(prepared.map(Some));
}

unsafe extern "system" fn hook_targeted(status: usize, target_descriptor: usize, decision: usize) {
    let _callback_lease = CallbackLease::enter();
    let prepared = prepare_decision_row(
        DirectEventKind::TargetedDecisionTaken,
        status,
        decision,
        Some(target_descriptor),
    );
    unsafe { call_original_targeted(status, target_descriptor, decision) };
    publish(prepared.map(Some));
}

unsafe extern "system" fn hook_timed(status: usize, decision: usize) {
    let _callback_lease = CallbackLease::enter();
    let prepared = prepare_decision_row(
        DirectEventKind::TimedDecisionActivated,
        status,
        decision,
        None,
    );
    unsafe { call_original_timed(status, decision) };
    publish(prepared.map(Some));
}

unsafe extern "system" fn hook_on_action(
    manager: usize,
    on_action: usize,
    scope: usize,
    ignore_running_check: u8,
) {
    let _callback_lease = CallbackLease::enter();
    let prepared = read_on_action_token_hash(on_action).and_then(|token_hash| {
        let Some(token_hash) = token_hash else {
            return Ok(None);
        };
        prepare_pointer_row(
            DirectEventKind::OnActionInvoked,
            scope,
            on_action,
            Some(manager),
        )
        .map(|mut row| {
            row.subject_token_hash = token_hash;
            row.input_bits = u64::from(ignore_running_check);
            Some(row)
        })
    });
    publish_optional_entry(prepared);
    unsafe { call_original_on_action(manager, on_action, scope, ignore_running_check) };
}

unsafe extern "system" fn hook_decision_cancel_effect(decision: usize, country: usize) {
    let _callback_lease = CallbackLease::enter();
    publish_entry(prepare_terminal_row(
        DirectEventKind::DecisionCancelEffectInvoked,
        decision,
        country,
        None,
    ));
    unsafe { call_original_decision_terminal(&ORIGINAL_DECISION_CANCEL, decision, country) };
}

unsafe extern "system" fn hook_decision_remove_effect(decision: usize, country: usize) {
    let _callback_lease = CallbackLease::enter();
    publish_entry(prepare_terminal_row(
        DirectEventKind::DecisionRemoveEffectInvoked,
        decision,
        country,
        None,
    ));
    unsafe { call_original_decision_terminal(&ORIGINAL_DECISION_REMOVE, decision, country) };
}

unsafe extern "system" fn hook_decision_timeout_effect(decision: usize, country: usize) {
    let _callback_lease = CallbackLease::enter();
    publish_entry(prepare_terminal_row(
        DirectEventKind::DecisionTimeoutEffectInvoked,
        decision,
        country,
        None,
    ));
    unsafe { call_original_decision_terminal(&ORIGINAL_DECISION_TIMEOUT, decision, country) };
}

unsafe extern "system" fn hook_targeted_decision_cancel_effect(
    decision: usize,
    country: usize,
    target: usize,
) {
    let _callback_lease = CallbackLease::enter();
    publish_entry(prepare_terminal_row(
        DirectEventKind::TargetedDecisionCancelEffectInvoked,
        decision,
        country,
        Some(target),
    ));
    unsafe {
        call_original_targeted_decision_terminal(
            &ORIGINAL_TARGETED_DECISION_CANCEL,
            decision,
            country,
            target,
        )
    };
}

unsafe extern "system" fn hook_targeted_decision_remove_effect(
    decision: usize,
    country: usize,
    target: usize,
) {
    let _callback_lease = CallbackLease::enter();
    publish_entry(prepare_terminal_row(
        DirectEventKind::TargetedDecisionRemoveEffectInvoked,
        decision,
        country,
        Some(target),
    ));
    unsafe {
        call_original_targeted_decision_terminal(
            &ORIGINAL_TARGETED_DECISION_REMOVE,
            decision,
            country,
            target,
        )
    };
}

unsafe extern "system" fn hook_targeted_decision_timeout_effect(
    decision: usize,
    country: usize,
    target: usize,
) {
    let _callback_lease = CallbackLease::enter();
    publish_entry(prepare_terminal_row(
        DirectEventKind::TargetedDecisionTimeoutEffectInvoked,
        decision,
        country,
        Some(target),
    ));
    unsafe {
        call_original_targeted_decision_terminal(
            &ORIGINAL_TARGETED_DECISION_TIMEOUT,
            decision,
            country,
            target,
        )
    };
}

fn prepare_focus(
    progress: usize,
    completed_focus: usize,
    country_key_pointer: usize,
    context40_pointer: usize,
) -> Result<PreparedRow, String> {
    let country_key = read_u32(country_key_pointer)
        .ok_or("D45 focus hook could not pre-copy *R8 country key")?;
    let context40 = read_i32(context40_pointer)
        .ok_or("D45 focus hook could not pre-copy *(i32*)R9 context40")?;
    let mut prepared = prepare_pointer_row(
        DirectEventKind::FocusCompleted,
        progress,
        completed_focus,
        None,
    )?;
    prepared.input_bits = u64::from(country_key) | (u64::from(context40 as u32) << 32);
    prepared.owner_key = u64::from(country_key);
    prepared.subject_token_hash = read_token_hash(completed_focus, FOCUS_TOKEN_OFFSET)
        .ok_or("D45 focus hook could not decode completed-focus token")?;
    prepared.before = DirectEventStateSummary {
        state_bits: u64::from(country_key),
        progress_raw: context40,
        remaining_raw: 0,
    };
    Ok(prepared)
}

fn prepare_decision_row(
    kind: DirectEventKind,
    status: usize,
    decision: usize,
    target: Option<usize>,
) -> Result<PreparedRow, String> {
    let mut prepared = prepare_pointer_row(kind, status, decision, target)?;
    prepared.owner_key = checked_read_usize(status, DECISION_STATUS_COUNTRY_OFFSET)
        .and_then(|country| checked_read_u32(country, COUNTRY_TAG_DB_ID_OFFSET))
        .map(u64::from)
        .ok_or("D45 decision hook could not decode owner country key")?;
    prepared.subject_token_hash = read_token_hash(decision, DECISION_TOKEN_OFFSET)
        .ok_or("D45 decision hook could not decode authored token")?;
    Ok(prepared)
}

fn prepare_terminal_row(
    kind: DirectEventKind,
    decision: usize,
    country: usize,
    target: Option<usize>,
) -> Result<PreparedRow, String> {
    let mut prepared = prepare_pointer_row(kind, country, decision, target)?;
    prepared.owner_key = checked_read_u32(country, COUNTRY_TAG_DB_ID_OFFSET)
        .map(u64::from)
        .ok_or("D45 decision terminal hook could not decode owner country key")?;
    prepared.subject_token_hash = read_token_hash(decision, DECISION_TOKEN_OFFSET)
        .ok_or("D45 decision terminal hook could not decode authored token")?;
    Ok(prepared)
}

fn prepare_research(instance: usize, new_progress: i32) -> Result<Option<PreparedRow>, String> {
    let current = checked_read_i32(instance, RESEARCH_PROGRESS_OFFSET)
        .ok_or("D45 research hook could not pre-copy instance+0x174 progress")?;
    let definition = checked_read_usize(instance, RESEARCH_DEFINITION_OFFSET)
        .ok_or("D45 research hook could not pre-copy technology definition")?;
    let required = checked_read_i32(definition, RESEARCH_REQUIRED_OFFSET)
        .ok_or("D45 research hook could not pre-copy required threshold")?;
    if !research_threshold_crossed(current, new_progress, required) {
        return Ok(None);
    }
    let mut prepared =
        prepare_pointer_row(DirectEventKind::ResearchCompleted, instance, definition, None)?;
    prepared.subject_key = checked_read_u32(definition, TECHNOLOGY_DEFINITION_INDEX_OFFSET)
        .map(u64::from)
        .filter(|key| *key != 0)
        .ok_or("D45 research hook could not decode CTechnology definition index")?;
    prepared.input_bits = u64::from(new_progress as u32);
    prepared.before = DirectEventStateSummary {
        state_bits: HOOK_SUMMARY_RESEARCH_PROGRESS_PRESENT
            | HOOK_SUMMARY_RESEARCH_REQUIRED_PRESENT,
        progress_raw: current,
        remaining_raw: required,
    };
    Ok(Some(prepared))
}

fn research_threshold_crossed(current: i32, new_progress: i32, required: i32) -> bool {
    current < required && new_progress >= required
}

fn prepare_pointer_row(
    kind: DirectEventKind,
    owner_pointer: usize,
    subject_pointer: usize,
    argument_pointer: Option<usize>,
) -> Result<PreparedRow, String> {
    let game_hour_raw = read_game_hour()?;
    let thread_id = unsafe { GetCurrentThreadId() };
    if thread_id == 0 {
        return Err("D45 direct hook GetCurrentThreadId returned zero".into());
    }
    let mut state = STATE
        .get()
        .ok_or("D45 direct hook state is not initialized")?
        .lock()
        .map_err(|_| "D45 direct hook state mutex poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    let owner_instance_id = state.intern(owner_pointer, "owner")?;
    let subject_instance_id = state.intern(subject_pointer, "subject")?;
    // Target RDX is assigned an opaque ID but is never entered in the retained
    // raw-pointer map. It disappears before the original call begins.
    let argument_instance_id = match argument_pointer {
        Some(pointer) => state.ephemeral(pointer, "target argument")?,
        None => OpaqueInstanceId::NONE,
    };
    Ok(PreparedRow {
        kind,
        thread_id,
        game_hour_raw,
        owner_instance_id,
        subject_instance_id,
        argument_instance_id,
        owner_key: 0,
        subject_token_hash: 0,
        subject_key: 0,
        input_bits: 0,
        before: DirectEventStateSummary::default(),
    })
}

fn publish(prepared: Result<Option<PreparedRow>, String>) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.callback_count = state.callback_count.saturating_add(1);
    if state.current_error.is_some() {
        return;
    }
    let prepared = match prepared {
        Ok(Some(prepared)) => prepared,
        Ok(None) => return,
        Err(error) => {
            state.current_error = Some(error);
            return;
        }
    };
    match state.bound_thread_id {
        Some(bound) if bound != prepared.thread_id => {
            state.current_error = Some(format!(
                "D45 direct hooks crossed callback threads ({bound} then {}); total sequence ordering is unauthenticated",
                prepared.thread_id
            ));
            return;
        }
        None => state.bound_thread_id = Some(prepared.thread_id),
        Some(_) => {}
    }
    if let Some(previous) = state.last_game_hour_raw {
        if prepared.game_hour_raw < previous {
            state.current_error = Some(format!(
                "D45 direct-hook raw hour regressed to {} after {previous}; ordered history is invalid",
                prepared.game_hour_raw
            ));
            return;
        }
    }
    let mut after = prepared.before;
    after.state_bits |= HOOK_SUMMARY_RETURN_OBSERVED;
    let row = UnsequencedDirectEventRow {
        kind: prepared.kind,
        thread_id: prepared.thread_id,
        game_hour_raw: prepared.game_hour_raw,
        root_rva: prepared.kind.expected_root_rva_a729(),
        owner_instance_id: prepared.owner_instance_id,
        subject_instance_id: prepared.subject_instance_id,
        argument_instance_id: prepared.argument_instance_id,
        owner_key: prepared.owner_key,
        subject_token_hash: prepared.subject_token_hash,
        subject_key: prepared.subject_key,
        input_bits: prepared.input_bits,
        return_bits: HOOK_VOID_RETURN_OBSERVED_BITS,
        before: prepared.before,
        after,
    };
    if let Err(error) = state.carrier.capture(row) {
        state.current_error = Some(format!("D45 direct hook carrier rejected row: {error}"));
    } else {
        state.last_game_hour_raw = Some(prepared.game_hour_raw);
        let index = prepared.kind as usize - 1;
        state.kind_witness_counts[index] = state.kind_witness_counts[index].saturating_add(1);
    }
}

fn publish_entry(prepared: Result<PreparedRow, String>) {
    publish_optional_entry(prepared.map(Some));
}

fn publish_optional_entry(prepared: Result<Option<PreparedRow>, String>) {
    let prepared = match prepared {
        Ok(Some(prepared)) => prepared,
        Ok(None) => {
            if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
                state.callback_count = state.callback_count.saturating_add(1);
            }
            return;
        }
        Err(error) => {
            if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
                state.callback_count = state.callback_count.saturating_add(1);
                if state.current_error.is_none() {
                    state.current_error = Some(error);
                }
            }
            return;
        }
    };
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.callback_count = state.callback_count.saturating_add(1);
    if state.current_error.is_some() {
        return;
    }
    match state.bound_thread_id {
        Some(bound) if bound != prepared.thread_id => {
            state.current_error = Some(format!(
                "D45 X10 entries crossed callback threads ({bound} then {})",
                prepared.thread_id
            ));
            return;
        }
        None => state.bound_thread_id = Some(prepared.thread_id),
        Some(_) => {}
    }
    if let Some(previous) = state.last_game_hour_raw {
        if prepared.game_hour_raw < previous {
            state.current_error = Some(format!(
                "D45 X10 entry raw hour regressed to {} after {previous}",
                prepared.game_hour_raw
            ));
            return;
        }
    }
    let row = UnsequencedDirectEventRow {
        kind: prepared.kind,
        thread_id: prepared.thread_id,
        game_hour_raw: prepared.game_hour_raw,
        root_rva: prepared.kind.expected_root_rva_a729(),
        owner_instance_id: prepared.owner_instance_id,
        subject_instance_id: prepared.subject_instance_id,
        argument_instance_id: prepared.argument_instance_id,
        owner_key: prepared.owner_key,
        subject_token_hash: prepared.subject_token_hash,
        subject_key: prepared.subject_key,
        input_bits: prepared.input_bits,
        return_bits: 0,
        before: DirectEventStateSummary::default(),
        after: DirectEventStateSummary {
            state_bits: HOOK_ENTRY_OBSERVED_BITS,
            ..DirectEventStateSummary::default()
        },
    };
    if let Err(error) = state.carrier.capture(row) {
        state.current_error = Some(format!("D45 X10 entry carrier rejected row: {error}"));
    } else {
        state.last_game_hour_raw = Some(prepared.game_hour_raw);
        let index = prepared.kind as usize - 1;
        state.kind_witness_counts[index] = state.kind_witness_counts[index].saturating_add(1);
    }
}

fn read_game_hour() -> Result<i32, String> {
    let base = module_base()?;
    let game_state = checked_read_usize(base, CGAMESTATE_SINGLETON_RVA)
        .ok_or("D45 direct hook CGameState singleton unreadable")?;
    checked_read_i32(game_state, CURRENT_HOUR_OFFSET)
        .ok_or_else(|| "D45 direct hook raw game hour unreadable".into())
}

fn module_base() -> Result<usize, String> {
    let stored = MODULE_BASE.load(Ordering::Acquire);
    if stored != 0 {
        return Ok(stored);
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        Err("D45 direct hook module base unavailable".into())
    } else {
        Ok(base)
    }
}

fn targets(base: usize) -> Result<[usize; 12], String> {
    if base == 0 {
        return Err("D45 direct hook module base is zero".into());
    }
    let target = |rva: u32| {
        base.checked_add(rva as usize)
            .ok_or_else(|| "D45 direct hook target address overflow".to_string())
    };
    Ok([
        target(ROOT_RVA_FOCUS_COMPLETED_A729)?,
        target(ROOT_RVA_RESEARCH_COMPLETED_A729)?,
        target(ROOT_RVA_ORDINARY_DECISION_TAKEN_A729)?,
        target(ROOT_RVA_TARGETED_DECISION_TAKEN_A729)?,
        target(ROOT_RVA_TIMED_DECISION_ACTIVATED_A729)?,
        target(ROOT_RVA_ON_ACTION_INVOKED_A729)?,
        target(ROOT_RVA_DECISION_CANCEL_EFFECT_A729)?,
        target(ROOT_RVA_DECISION_REMOVE_EFFECT_A729)?,
        target(ROOT_RVA_DECISION_TIMEOUT_EFFECT_A729)?,
        target(ROOT_RVA_TARGETED_DECISION_CANCEL_EFFECT_A729)?,
        target(ROOT_RVA_TARGETED_DECISION_REMOVE_EFFECT_A729)?,
        target(ROOT_RVA_TARGETED_DECISION_TIMEOUT_EFFECT_A729)?,
    ])
}

fn target_sentinels(
    targets: &[usize; 12],
) -> [(usize, &'static [u8; 16], &'static str); 12] {
    [
        (targets[0], &FOCUS_SENTINEL, "focus"),
        (targets[1], &RESEARCH_SENTINEL, "research"),
        (targets[2], &ORDINARY_DECISION_SENTINEL, "ordinary decision"),
        (targets[3], &TARGETED_DECISION_SENTINEL, "targeted decision"),
        (targets[4], &TIMED_DECISION_SENTINEL, "timed decision"),
        (targets[5], &ON_ACTION_SENTINEL, "on action"),
        (targets[6], &DECISION_TERMINAL_SENTINEL, "decision cancel effect"),
        (targets[7], &DECISION_TERMINAL_SENTINEL, "decision remove effect"),
        (targets[8], &DECISION_TERMINAL_SENTINEL, "decision timeout effect"),
        (
            targets[9],
            &TARGETED_DECISION_TERMINAL_SENTINEL,
            "targeted decision cancel effect",
        ),
        (
            targets[10],
            &TARGETED_DECISION_TERMINAL_SENTINEL,
            "targeted decision remove effect",
        ),
        (
            targets[11],
            &TARGETED_DECISION_TIMEOUT_SENTINEL,
            "targeted decision timeout effect",
        ),
    ]
}

fn installed_roots() -> InstalledRoots {
    InstalledRoots {
        rvas: [
            ROOT_RVA_FOCUS_COMPLETED_A729,
            ROOT_RVA_RESEARCH_COMPLETED_A729,
            ROOT_RVA_ORDINARY_DECISION_TAKEN_A729,
            ROOT_RVA_TARGETED_DECISION_TAKEN_A729,
            ROOT_RVA_TIMED_DECISION_ACTIVATED_A729,
            ROOT_RVA_ON_ACTION_INVOKED_A729,
            ROOT_RVA_DECISION_CANCEL_EFFECT_A729,
            ROOT_RVA_DECISION_REMOVE_EFFECT_A729,
            ROOT_RVA_DECISION_TIMEOUT_EFFECT_A729,
            ROOT_RVA_TARGETED_DECISION_CANCEL_EFFECT_A729,
            ROOT_RVA_TARGETED_DECISION_REMOVE_EFFECT_A729,
            ROOT_RVA_TARGETED_DECISION_TIMEOUT_EFFECT_A729,
        ],
    }
}

fn create_remaining_hooks(targets: &[usize; 12]) -> Result<(), String> {
    let research = unsafe {
        MinHook::create_hook(targets[1] as *mut c_void, hook_research as *mut c_void)
    }
    .map_err(|status| format!("D45 research hook creation failed: {status}"))?;
    ORIGINAL_RESEARCH.store(research as usize, Ordering::Release);
    let ordinary = unsafe {
        MinHook::create_hook(targets[2] as *mut c_void, hook_ordinary as *mut c_void)
    }
    .map_err(|status| format!("D45 ordinary-decision hook creation failed: {status}"))?;
    ORIGINAL_ORDINARY.store(ordinary as usize, Ordering::Release);
    let targeted = unsafe {
        MinHook::create_hook(targets[3] as *mut c_void, hook_targeted as *mut c_void)
    }
    .map_err(|status| format!("D45 targeted-decision hook creation failed: {status}"))?;
    ORIGINAL_TARGETED.store(targeted as usize, Ordering::Release);
    let timed = unsafe {
        MinHook::create_hook(targets[4] as *mut c_void, hook_timed as *mut c_void)
    }
    .map_err(|status| format!("D45 timed-decision hook creation failed: {status}"))?;
    ORIGINAL_TIMED.store(timed as usize, Ordering::Release);
    let on_action = unsafe {
        MinHook::create_hook(targets[5] as *mut c_void, hook_on_action as *mut c_void)
    }
    .map_err(|status| format!("D45 on-action hook creation failed: {status}"))?;
    ORIGINAL_ON_ACTION.store(on_action as usize, Ordering::Release);
    let decision_cancel = unsafe {
        MinHook::create_hook(
            targets[6] as *mut c_void,
            hook_decision_cancel_effect as *mut c_void,
        )
    }
    .map_err(|status| format!("D45 decision-cancel hook creation failed: {status}"))?;
    ORIGINAL_DECISION_CANCEL.store(decision_cancel as usize, Ordering::Release);
    let decision_remove = unsafe {
        MinHook::create_hook(
            targets[7] as *mut c_void,
            hook_decision_remove_effect as *mut c_void,
        )
    }
    .map_err(|status| format!("D45 decision-remove hook creation failed: {status}"))?;
    ORIGINAL_DECISION_REMOVE.store(decision_remove as usize, Ordering::Release);
    let decision_timeout = unsafe {
        MinHook::create_hook(
            targets[8] as *mut c_void,
            hook_decision_timeout_effect as *mut c_void,
        )
    }
    .map_err(|status| format!("D45 decision-timeout hook creation failed: {status}"))?;
    ORIGINAL_DECISION_TIMEOUT.store(decision_timeout as usize, Ordering::Release);
    let targeted_cancel = unsafe {
        MinHook::create_hook(
            targets[9] as *mut c_void,
            hook_targeted_decision_cancel_effect as *mut c_void,
        )
    }
    .map_err(|status| format!("D45 targeted-decision-cancel hook creation failed: {status}"))?;
    ORIGINAL_TARGETED_DECISION_CANCEL.store(targeted_cancel as usize, Ordering::Release);
    let targeted_remove = unsafe {
        MinHook::create_hook(
            targets[10] as *mut c_void,
            hook_targeted_decision_remove_effect as *mut c_void,
        )
    }
    .map_err(|status| format!("D45 targeted-decision-remove hook creation failed: {status}"))?;
    ORIGINAL_TARGETED_DECISION_REMOVE.store(targeted_remove as usize, Ordering::Release);
    let targeted_timeout = unsafe {
        MinHook::create_hook(
            targets[11] as *mut c_void,
            hook_targeted_decision_timeout_effect as *mut c_void,
        )
    }
    .map_err(|status| format!("D45 targeted-decision-timeout hook creation failed: {status}"))?;
    ORIGINAL_TARGETED_DECISION_TIMEOUT.store(targeted_timeout as usize, Ordering::Release);
    Ok(())
}

fn rollback(targets: &[usize; 12]) -> Result<(), String> {
    let mut errors = Vec::new();
    for target in targets {
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
    QUIESCED.store(true, Ordering::Release);
    wait_for_callback_quiescence()?;
    for target in targets.iter().rev() {
        match unsafe { MinHook::remove_hook(*target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("remove {target:#x}: {status}")),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    clear_originals();
    Ok(())
}

fn wait_for_callback_quiescence() -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err(format!(
                "D45 direct hooks still have {} callback(s) in flight after disable",
                ACTIVE_CALLBACKS.load(Ordering::Acquire)
            ));
        }
        std::thread::yield_now();
    }
    Ok(())
}

fn clear_hook_lifecycle_state() {
    clear_originals();
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    EVIDENCE_FIXTURE.store(false, Ordering::Release);
    QUIESCED.store(false, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
}

fn clear_originals() {
    ORIGINAL_FOCUS.store(0, Ordering::Release);
    ORIGINAL_RESEARCH.store(0, Ordering::Release);
    ORIGINAL_ORDINARY.store(0, Ordering::Release);
    ORIGINAL_TARGETED.store(0, Ordering::Release);
    ORIGINAL_TIMED.store(0, Ordering::Release);
    ORIGINAL_ON_ACTION.store(0, Ordering::Release);
    ORIGINAL_DECISION_CANCEL.store(0, Ordering::Release);
    ORIGINAL_DECISION_REMOVE.store(0, Ordering::Release);
    ORIGINAL_DECISION_TIMEOUT.store(0, Ordering::Release);
    ORIGINAL_TARGETED_DECISION_CANCEL.store(0, Ordering::Release);
    ORIGINAL_TARGETED_DECISION_REMOVE.store(0, Ordering::Release);
    ORIGINAL_TARGETED_DECISION_TIMEOUT.store(0, Ordering::Release);
}

fn verify_sentinel(address: usize, expected: &[u8; 16], name: &str) -> Result<(), String> {
    let mut actual = [0u8; 16];
    if !read_process(address, &mut actual) || &actual != expected {
        return Err(format!("D45 direct hook {name} sentinel mismatch"));
    }
    Ok(())
}

fn checked_read_i32(base: usize, offset: usize) -> Option<i32> {
    (base != 0)
        .then(|| base.checked_add(offset).and_then(read_i32))
        .flatten()
}

fn checked_read_u32(base: usize, offset: usize) -> Option<u32> {
    (base != 0)
        .then(|| base.checked_add(offset).and_then(read_u32))
        .flatten()
}

fn checked_read_usize(base: usize, offset: usize) -> Option<usize> {
    (base != 0)
        .then(|| base.checked_add(offset).and_then(read_usize))
        .flatten()
}

fn read_on_action_token_hash(on_action: usize) -> Result<Option<u64>, String> {
    let active = checked_read_u8(on_action, ON_ACTION_ACTIVE_OFFSET)
        .ok_or("D45 on-action hook could not read the authenticated active gate")?;
    if active == 0 {
        return Ok(None);
    }
    read_token_hash(on_action, ON_ACTION_TOKEN_OFFSET)
        .map(Some)
        .ok_or_else(|| "D45 on-action hook could not decode active authored token".into())
}

fn read_token_hash(object: usize, offset: usize) -> Option<u64> {
    let address = object.checked_add(offset)?;
    let mut header = [0u8; 32];
    if !read_process(address, &mut header) {
        return None;
    }
    let length = u64::from_le_bytes(header[16..24].try_into().ok()?) as usize;
    let capacity = u64::from_le_bytes(header[24..32].try_into().ok()?) as usize;
    if length == 0 || length > MAX_PORTABLE_TOKEN_LEN || capacity < length {
        return None;
    }
    let mut bytes = vec![0u8; length];
    if capacity <= 15 {
        bytes.copy_from_slice(&header[..length]);
    } else {
        let pointer = u64::from_le_bytes(header[..8].try_into().ok()?) as usize;
        if !read_process(pointer, &mut bytes) {
            return None;
        }
    }
    // Hash the exact engine-owned token bytes. The MSVC string length/capacity
    // checks and guarded read authenticate the storage; an ASCII punctuation
    // allow-list is not part of the engine ABI and rejected valid mod tokens.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(hash.max(1))
}

fn read_process(address: usize, output: &mut [u8]) -> bool {
    if address == 0 {
        return false;
    }
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            output.as_mut_ptr().cast(),
            output.len(),
            &mut read,
        )
    };
    ok != 0 && read == output.len()
}

fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0u8; 4];
    read_process(address, &mut bytes).then_some(u32::from_le_bytes(bytes))
}

fn read_u8(address: usize) -> Option<u8> {
    let mut bytes = [0u8; 1];
    read_process(address, &mut bytes).then_some(bytes[0])
}

fn checked_read_u8(base: usize, offset: usize) -> Option<u8> {
    (base != 0)
        .then(|| base.checked_add(offset).and_then(read_u8))
        .flatten()
}

fn read_i32(address: usize) -> Option<i32> {
    read_u32(address).map(|value| value as i32)
}

fn read_usize(address: usize) -> Option<usize> {
    let mut bytes = [0u8; mem::size_of::<usize>()];
    read_process(address, &mut bytes).then_some(usize::from_le_bytes(bytes))
}

unsafe fn call_original_focus(a: usize, b: usize, c: usize, d: usize) {
    let address = original_or_abort(&ORIGINAL_FOCUS);
    let original: FocusFn = unsafe { mem::transmute(address) };
    unsafe { original(a, b, c, d) };
}

unsafe fn call_original_research(a: usize, b: i32) {
    let address = original_or_abort(&ORIGINAL_RESEARCH);
    let original: ResearchFn = unsafe { mem::transmute(address) };
    unsafe { original(a, b) };
}

unsafe fn call_original_ordinary(a: usize, b: usize) {
    let address = original_or_abort(&ORIGINAL_ORDINARY);
    let original: OrdinaryDecisionFn = unsafe { mem::transmute(address) };
    unsafe { original(a, b) };
}

unsafe fn call_original_targeted(a: usize, b: usize, c: usize) {
    let address = original_or_abort(&ORIGINAL_TARGETED);
    let original: TargetedDecisionFn = unsafe { mem::transmute(address) };
    unsafe { original(a, b, c) };
}

unsafe fn call_original_timed(a: usize, b: usize) {
    let address = original_or_abort(&ORIGINAL_TIMED);
    let original: TimedDecisionFn = unsafe { mem::transmute(address) };
    unsafe { original(a, b) };
}

unsafe fn call_original_on_action(a: usize, b: usize, c: usize, d: u8) {
    let address = original_or_abort(&ORIGINAL_ON_ACTION);
    let original: OnActionFn = unsafe { mem::transmute(address) };
    unsafe { original(a, b, c, d) };
}

unsafe fn call_original_decision_terminal(slot: &AtomicUsize, a: usize, b: usize) {
    let address = original_or_abort(slot);
    let original: DecisionTerminalFn = unsafe { mem::transmute(address) };
    unsafe { original(a, b) };
}

unsafe fn call_original_targeted_decision_terminal(
    slot: &AtomicUsize,
    a: usize,
    b: usize,
    c: usize,
) {
    let address = original_or_abort(slot);
    let original: TargetedDecisionTerminalFn = unsafe { mem::transmute(address) };
    unsafe { original(a, b, c) };
}

fn original_or_abort(slot: &AtomicUsize) -> usize {
    let address = slot.load(Ordering::Acquire);
    if address == 0 {
        std::process::abort();
    }
    address
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_build_hook_is_scheduled_through_the_authenticated_runtime_seam() {
        assert!(PRODUCTION_ACTIVATION);
    }

    #[test]
    fn production_scheduling_does_not_bypass_target_address_gates() {
        assert!(targets(0)
            .expect_err("a zero module base must remain fail closed")
            .contains("module base is zero"));
        assert!(targets(usize::MAX)
            .expect_err("overflowing a production hook target must remain fail closed")
            .contains("target address overflow"));
    }

    #[test]
    fn exact_roots_and_sentinels_are_bound() {
        assert_eq!(installed_roots().rvas.len(), 12);
        assert_eq!(FOCUS_SENTINEL.len(), 16);
        assert_eq!(RESEARCH_SENTINEL.len(), 16);
        assert_eq!(ORDINARY_DECISION_SENTINEL.len(), 16);
        assert_eq!(TARGETED_DECISION_SENTINEL.len(), 16);
        assert_eq!(TIMED_DECISION_SENTINEL.len(), 16);
        assert_eq!(ON_ACTION_SENTINEL.len(), 16);
        assert_eq!(DECISION_TERMINAL_SENTINEL.len(), 16);
        assert_eq!(TARGETED_DECISION_TERMINAL_SENTINEL.len(), 16);
        assert_eq!(TARGETED_DECISION_TIMEOUT_SENTINEL.len(), 16);
    }

    #[test]
    fn research_only_emits_on_upward_threshold_crossing() {
        assert!(research_threshold_crossed(99, 100, 100));
        assert!(!research_threshold_crossed(100, 101, 100));
        assert!(!research_threshold_crossed(99, 99, 100));
        assert!(!research_threshold_crossed(101, 99, 100));
    }

    #[test]
    fn inactive_on_action_does_not_require_an_authored_token() {
        let object = [0u8; 64];
        assert_eq!(
            read_on_action_token_hash(object.as_ptr() as usize).unwrap(),
            None
        );
    }

    #[test]
    fn active_on_action_hashes_exact_engine_token_bytes_without_an_ascii_allowlist() {
        let mut object = [0u8; 64];
        object[ON_ACTION_ACTIVE_OFFSET] = 1;
        let token = b"on_mod/path+\xC2\xA7";
        object[ON_ACTION_TOKEN_OFFSET..ON_ACTION_TOKEN_OFFSET + token.len()]
            .copy_from_slice(token);
        object[ON_ACTION_TOKEN_OFFSET + 16..ON_ACTION_TOKEN_OFFSET + 24]
            .copy_from_slice(&(token.len() as u64).to_le_bytes());
        object[ON_ACTION_TOKEN_OFFSET + 24..ON_ACTION_TOKEN_OFFSET + 32]
            .copy_from_slice(&15u64.to_le_bytes());

        let actual = read_on_action_token_hash(object.as_ptr() as usize)
            .unwrap()
            .unwrap();
        let expected = token.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        assert_eq!(actual, expected.max(1));
    }
}
