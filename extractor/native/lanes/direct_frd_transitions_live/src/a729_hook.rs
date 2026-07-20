//! Dormant a729 MinHook adapter for the authenticated D45 direct-event roots.
//!
//! Installation is deliberately refused while [`PRODUCTION_ACTIVATION`] is
//! false. The adapter contains no evidence bypass. It validates all five entry
//! sentinels before creating any hook, calls each trampoline exactly once, and
//! emits a row only after the original returns. Raw addresses exist only as
//! callback-local values and private identity-map keys.
//!
//! Row scalar contract:
//! - focus `input_bits`: low 32 bits are pre-copied `*R8`; high 32 bits are
//!   pre-copied `*(i32*)R9` bits;
//! - research emits only for `pre_progress < required && EDX >= required`;
//!   its `input_bits` low 32 bits are entry `EDX`;
//! - decision `input_bits`: zero because no scalar argument is authenticated;
//! - every emitted row has [`HOOK_VOID_RETURN_OBSERVED_BITS`] in `return_bits`
//!   and [`HOOK_SUMMARY_RETURN_OBSERVED`] added only to the post summary.

use core::ffi::c_void;
use minhook::MinHook;
use std::cell::Cell;
use std::collections::HashMap;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

use crate::{
    DirectEventCarrier, DirectEventKind, DirectEventRow, DirectEventStateSummary, OpaqueInstanceId,
    UnsequencedDirectEventRow, ROOT_RVA_FOCUS_COMPLETED_A729,
    ROOT_RVA_ORDINARY_DECISION_TAKEN_A729, ROOT_RVA_RESEARCH_COMPLETED_A729,
    ROOT_RVA_TARGETED_DECISION_TAKEN_A729, ROOT_RVA_TIMED_DECISION_ACTIVATED_A729,
};

/// Remains false until authenticated live entry/return observations close the
/// invocation gate. Unlike evidence-only hooks, there is no environment bypass.
pub const PRODUCTION_ACTIVATION: bool = crate::PRODUCTION_ACTIVATION;

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6e90;
const CURRENT_HOUR_OFFSET: usize = 0x468;
const RESEARCH_PROGRESS_OFFSET: usize = 0x174;
const RESEARCH_DEFINITION_OFFSET: usize = 0x160;
const RESEARCH_REQUIRED_OFFSET: usize = 0x3dc;
const MAX_POINTER_IDENTITIES: usize = 65_536;

pub const HOOK_VOID_RETURN_OBSERVED_BITS: u64 = 1;
pub const HOOK_SUMMARY_RETURN_OBSERVED: u64 = 1 << 63;
pub const HOOK_SUMMARY_RESEARCH_PROGRESS_PRESENT: u64 = 1;
pub const HOOK_SUMMARY_RESEARCH_REQUIRED_PRESENT: u64 = 1 << 1;

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

type FocusFn = unsafe extern "system" fn(usize, usize, usize, usize);
type ResearchFn = unsafe extern "system" fn(usize, i32);
type OrdinaryDecisionFn = unsafe extern "system" fn(usize, usize);
type TargetedDecisionFn = unsafe extern "system" fn(usize, usize, usize);
type TimedDecisionFn = unsafe extern "system" fn(usize, usize);

static ORIGINAL_FOCUS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RESEARCH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ORDINARY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGETED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMED: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

#[derive(Debug)]
struct HookState {
    carrier: DirectEventCarrier,
    pointer_ids: HashMap<usize, OpaqueInstanceId>,
    callback_count: u64,
    bound_thread_id: Option<u32>,
    last_game_hour_raw: Option<i32>,
    current_error: Option<String>,
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
    pub rvas: [u32; 5],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub installed: bool,
    pub callback_count: u64,
    pub pending_rows: usize,
    pub pointer_identity_count: usize,
    pub bound_thread_id: Option<u32>,
    pub last_game_hour_raw: Option<i32>,
    pub current_error: Option<String>,
}

#[derive(Clone, Copy)]
struct PreparedRow {
    kind: DirectEventKind,
    thread_id: u32,
    game_hour_raw: i32,
    owner_instance_id: OpaqueInstanceId,
    subject_instance_id: OpaqueInstanceId,
    argument_instance_id: OpaqueInstanceId,
    input_bits: u64,
    before: DirectEventStateSummary,
}

pub fn install() -> Result<InstalledRoots, String> {
    if !PRODUCTION_ACTIVATION {
        return Err(
            "D45 a729 direct hooks are dormant until the live invocation gate closes".into(),
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
    for (target, sentinel, name) in target_sentinels(&targets) {
        verify_sentinel(target, sentinel, name)?;
    }

    let state = STATE.get_or_init(|| Mutex::new(HookState::fresh()));
    *state
        .lock()
        .map_err(|_| "D45 direct hook state mutex poisoned before install")? = HookState::fresh();

    let focus = unsafe { MinHook::create_hook(targets[0] as *mut c_void, hook_focus as *mut c_void) }
        .map_err(|status| format!("D45 focus hook creation failed: {status}"))?;
    ORIGINAL_FOCUS.store(focus as usize, Ordering::Release);
    if let Err(error) = create_remaining_hooks(&targets) {
        rollback(&targets);
        return Err(error);
    }
    MODULE_BASE.store(base, Ordering::Release);
    for (target, name) in targets.into_iter().zip([
        "focus",
        "research",
        "ordinary decision",
        "targeted decision",
        "timed decision",
    ]) {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            rollback(&targets);
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("D45 {name} hook enable failed: {status}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(installed_roots())
}

pub fn uninstall() -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let targets = targets(base)?;
    let mut errors = Vec::new();
    for (target, name) in targets.into_iter().zip([
        "focus",
        "research",
        "ordinary decision",
        "targeted decision",
        "timed decision",
    ]) {
        if let Err(status) = unsafe { MinHook::disable_hook(target as *mut c_void) } {
            errors.push(format!("disable {name}: {status}"));
        }
        if let Err(status) = unsafe { MinHook::remove_hook(target as *mut c_void) } {
            errors.push(format!("remove {name}: {status}"));
        }
    }
    if errors.is_empty() {
        clear_originals();
        MODULE_BASE.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        if let Some(state) = STATE.get() {
            *state
                .lock()
                .map_err(|_| "D45 direct hook state mutex poisoned during uninstall")? =
                HookState::fresh();
        }
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub const fn activation_enabled() -> bool {
    PRODUCTION_ACTIVATION
}

pub fn drain_rows() -> Result<Vec<DirectEventRow>, String> {
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
    state
        .carrier
        .drain_rows()
        .map_err(|error| format!("D45 direct hook typed drain failed: {error}"))
}

pub fn drain_trailer() -> Result<Vec<u8>, String> {
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
    state
        .carrier
        .drain_trailer()
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
        pending_rows: state.carrier.pending_rows().len(),
        pointer_identity_count: state.pointer_ids.len(),
        bound_thread_id: state.bound_thread_id,
        last_game_hour_raw: state.last_game_hour_raw,
        current_error: state.current_error.clone(),
    }
}

unsafe extern "system" fn hook_focus(
    progress: usize,
    completed_focus: usize,
    country_key: usize,
    context40: usize,
) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let prepared = (!nested).then(|| prepare_focus(progress, completed_focus, country_key, context40));
    unsafe { call_original_focus(progress, completed_focus, country_key, context40) };
    if nested {
        record_nested_callback("focus");
    } else {
        publish(
            prepared
                .unwrap_or_else(|| Err("D45 focus preparation missing".into()))
                .map(Some),
        );
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_research(instance: usize, new_progress: i32) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let prepared = (!nested).then(|| prepare_research(instance, new_progress));
    unsafe { call_original_research(instance, new_progress) };
    if nested {
        record_nested_callback("research");
    } else {
        publish(prepared.unwrap_or_else(|| Err("D45 research preparation missing".into())));
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_ordinary(status: usize, decision: usize) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let prepared = (!nested).then(|| {
        prepare_pointer_row(
            DirectEventKind::OrdinaryDecisionTaken,
            status,
            decision,
            None,
        )
    });
    unsafe { call_original_ordinary(status, decision) };
    if nested {
        record_nested_callback("ordinary decision");
    } else {
        publish(
            prepared
                .unwrap_or_else(|| Err("D45 ordinary preparation missing".into()))
                .map(Some),
        );
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_targeted(status: usize, target_descriptor: usize, decision: usize) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let prepared = (!nested).then(|| {
        prepare_pointer_row(
            DirectEventKind::TargetedDecisionTaken,
            status,
            decision,
            Some(target_descriptor),
        )
    });
    unsafe { call_original_targeted(status, target_descriptor, decision) };
    if nested {
        record_nested_callback("targeted decision");
    } else {
        publish(
            prepared
                .unwrap_or_else(|| Err("D45 targeted preparation missing".into()))
                .map(Some),
        );
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_timed(status: usize, decision: usize) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let prepared = (!nested).then(|| {
        prepare_pointer_row(
            DirectEventKind::TimedDecisionActivated,
            status,
            decision,
            None,
        )
    });
    unsafe { call_original_timed(status, decision) };
    if nested {
        record_nested_callback("timed decision");
    } else {
        publish(
            prepared
                .unwrap_or_else(|| Err("D45 timed preparation missing".into()))
                .map(Some),
        );
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
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
    prepared.before = DirectEventStateSummary {
        state_bits: u64::from(country_key),
        progress_raw: context40,
        remaining_raw: 0,
    };
    Ok(prepared)
}

fn prepare_research(instance: usize, new_progress: i32) -> Result<Option<PreparedRow>, String> {
    let current = checked_read_i32(instance, RESEARCH_PROGRESS_OFFSET)
        .ok_or("D45 research hook could not pre-copy instance+0x174 progress")?;
    let required = checked_read_usize(instance, RESEARCH_DEFINITION_OFFSET)
        .and_then(|definition| checked_read_i32(definition, RESEARCH_REQUIRED_OFFSET))
        .ok_or("D45 research hook could not pre-copy required threshold")?;
    if !research_threshold_crossed(current, new_progress, required) {
        return Ok(None);
    }
    let mut prepared =
        prepare_pointer_row(DirectEventKind::ResearchCompleted, instance, instance, None)?;
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
        input_bits: prepared.input_bits,
        return_bits: HOOK_VOID_RETURN_OBSERVED_BITS,
        before: prepared.before,
        after,
    };
    if let Err(error) = state.carrier.capture(row) {
        state.current_error = Some(format!("D45 direct hook carrier rejected row: {error}"));
    } else {
        state.last_game_hour_raw = Some(prepared.game_hour_raw);
    }
}

fn record_nested_callback(kind: &str) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.callback_count = state.callback_count.saturating_add(1);
    if state.current_error.is_none() {
        state.current_error = Some(format!(
            "D45 nested {kind} callback observed; recursion suppression makes the direct-event stream incomplete"
        ));
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

fn targets(base: usize) -> Result<[usize; 5], String> {
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
    ])
}

fn target_sentinels(targets: &[usize; 5]) -> [(usize, &'static [u8; 16], &'static str); 5] {
    [
        (targets[0], &FOCUS_SENTINEL, "focus"),
        (targets[1], &RESEARCH_SENTINEL, "research"),
        (targets[2], &ORDINARY_DECISION_SENTINEL, "ordinary decision"),
        (targets[3], &TARGETED_DECISION_SENTINEL, "targeted decision"),
        (targets[4], &TIMED_DECISION_SENTINEL, "timed decision"),
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
        ],
    }
}

fn create_remaining_hooks(targets: &[usize; 5]) -> Result<(), String> {
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
    Ok(())
}

fn rollback(targets: &[usize; 5]) {
    for target in targets {
        let _ = unsafe { MinHook::disable_hook(*target as *mut c_void) };
        let _ = unsafe { MinHook::remove_hook(*target as *mut c_void) };
    }
    clear_originals();
}

fn clear_originals() {
    ORIGINAL_FOCUS.store(0, Ordering::Release);
    ORIGINAL_RESEARCH.store(0, Ordering::Release);
    ORIGINAL_ORDINARY.store(0, Ordering::Release);
    ORIGINAL_TARGETED.store(0, Ordering::Release);
    ORIGINAL_TIMED.store(0, Ordering::Release);
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

fn checked_read_usize(base: usize, offset: usize) -> Option<usize> {
    (base != 0)
        .then(|| base.checked_add(offset).and_then(read_usize))
        .flatten()
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
    fn dormant_gate_has_no_environment_bypass() {
        assert!(!PRODUCTION_ACTIVATION);
        assert!(!activation_enabled());
        assert!(install()
            .expect_err("dormant hook installation must fail")
            .contains("dormant"));
    }

    #[test]
    fn exact_roots_and_sentinels_are_bound() {
        assert_eq!(installed_roots().rvas.len(), 5);
        assert_eq!(FOCUS_SENTINEL.len(), 16);
        assert_eq!(RESEARCH_SENTINEL.len(), 16);
        assert_eq!(ORDINARY_DECISION_SENTINEL.len(), 16);
        assert_eq!(TARGETED_DECISION_SENTINEL.len(), 16);
        assert_eq!(TIMED_DECISION_SENTINEL.len(), 16);
    }

    #[test]
    fn research_only_emits_on_upward_threshold_crossing() {
        assert!(research_threshold_crossed(99, 100, 100));
        assert!(!research_threshold_crossed(100, 101, 100));
        assert!(!research_threshold_crossed(99, 99, 100));
        assert!(!research_threshold_crossed(101, 99, 100));
    }
}
