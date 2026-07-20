//! Discovery-complete, promotion-gated a729 ability-list transition observer.
//!
//! This module records the exact engine leaves needed by the A36 human
//! transition matrix.  It deliberately does not implement
//! `ExactA729AbilityControllerSource`: runtime pointers and aggregate scratch
//! records are diagnostic identities, not portable replay data.  Promotion is
//! additionally separated from live/compact promotion until the required human
//! transitions are captured.

use core::ffi::c_void;
use minhook::MinHook;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const ABILITY_REFRESH_RVA: u32 = 0x01D0_72F0;
pub const CONTEXT_INVALID_PROBE_RVA: u32 = 0x00B5_8CA0;
pub const ACTIVE_COOLDOWN_PROBE_RVA: u32 = 0x006F_3370;
pub const AVAILABILITY_EVALUATOR_RVA: u32 = 0x006D_0DE0;
pub const SPECIAL_CONTEXT_PROBE_RVA: u32 = 0x01D0_6480;

/// Static hook authority bound to the mechanically reconciled a729 receipt.
/// This opens diagnostic observer installation only; it does not promote ABL1
/// production output or schedule a compact domain.
pub const EXACT_A729_AUTHORITY_BOUND: bool = true;
pub const EXACT_A729_AUTHORITY_JOB_ID: &str =
    "20260720T020812Z-w3-a36-ability-list-feb98226";
pub const EXACT_A729_AUTHORITY_SHARD: &str = "shard-19";
pub const EXACT_A729_AUTHORITY_MANIFEST_SHA256: &str =
    "12768dfa9a8b47ceda4feae9cdc45b76811f602ecd557008aa936e95adf2db45";
pub const LIVE_TRANSITION_PROMOTION_BOUND: bool = false;
pub const COMPACT_DOMAIN_SCHEDULED: bool = false;

const REFRESH_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
];
const CONTEXT_PROBE_SENTINEL: [u8; 16] = [
    0x80, 0xb9, 0xfa, 0x03, 0x00, 0x00, 0x00, 0x74, 0x0c, 0x48, 0x8b, 0x89, 0x40, 0x02, 0x00, 0x00,
];
const ACTIVE_PROBE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
const AVAILABILITY_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x4c, 0x89, 0x48, 0x20, 0x48, 0x89, 0x50, 0x10, 0x48,
];
const SPECIAL_CONTEXT_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xb9, 0x39, 0x00, 0x00, 0x00, 0xe8, 0xad,
];

const MAX_ROWS: usize = 4096;
const MAX_OBSERVATIONS: usize = 128;
const FM_STATE_STRIDE: usize = 0x68;
const FM_LEADER_STATE_STRIDE: usize = 0x28;

type RefreshFn = unsafe extern "system" fn(usize);
type ContextProbeFn = unsafe extern "system" fn(usize) -> u8;
type ActiveProbeFn = unsafe extern "system" fn(usize, usize, usize, *mut u8, *mut i32) -> usize;
type AvailabilityFn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
type SpecialContextFn = unsafe extern "system" fn(usize) -> u8;

static ORIGINAL_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CONTEXT_PROBE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ACTIVE_PROBE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_AVAILABILITY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SPECIAL_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static STATE: OnceLock<Mutex<ObserverState>> = OnceLock::new();

thread_local! {
    static TRACE: RefCell<Option<OpenTrace>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VectorHeader {
    data: usize,
    capacity: u32,
    count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ControllerMeta {
    source: VectorHeader,
    state: VectorHeader,
    leaders: VectorHeader,
    row_widgets: VectorHeader,
    context_mode: u32,
    selected_root: usize,
    selected_leader: usize,
    selected_army: usize,
    selected_army_group: usize,
    selected_other: usize,
    field_marshal_aggregate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeEvent {
    ContextInvalid { invalid: bool },
    ActiveCooldown {
        definition_runtime: usize,
        scope_runtime: usize,
        active: bool,
        cooldown: bool,
        elapsed_since_activation_hours: i32,
    },
    Availability {
        definition_runtime: usize,
        scope_runtime: usize,
        available: bool,
    },
    SpecialContext { controller_runtime: usize, allowed: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenTrace {
    controller_runtime: usize,
    events: Vec<ProbeEvent>,
    nested_or_replaced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefinitionLeaves {
    pub definition_runtime: usize,
    pub cancelable: bool,
    pub special_context_required: bool,
    pub duration_hours: i32,
    pub cooldown_hours: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrdinaryAbilityState {
    Available,
    Active,
    Cooldown,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrdinaryAbilityLeaf {
    pub definition_runtime: usize,
    pub scope_runtime: usize,
    pub state: OrdinaryAbilityState,
    pub activation_elapsed_hours: i32,
    pub availability_evaluated: Option<bool>,
    pub special_context_allowed: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldMarshalLeaderLeaf {
    pub leader_runtime: usize,
    /// Exact engine enum: 0 available, 1 active, 2 cooldown, 3 cannot-use,
    /// 4 leader-unavailable.
    pub state_code: u8,
    pub remaining_hours: i32,
    pub command_power_cost_fixed: i64,
    pub daily_command_power_cost_fixed: i64,
    pub will_activate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldMarshalAbilityLeaf {
    pub definition_runtime: usize,
    pub leaders: Vec<FieldMarshalLeaderLeaf>,
    pub available_count: u32,
    pub active_count: u32,
    pub cooldown_count: u32,
    pub allocated_count: u32,
    pub candidate_leader_count: u32,
    pub definition_allowed_leader_count: u32,
    pub minimum_active_remaining_hours: i32,
    pub minimum_cooldown_remaining_hours: i32,
    pub group_activation_elapsed_hours: i32,
    pub allocated_command_power_cost_fixed: i64,
    pub allocated_daily_command_power_cost_fixed: i64,
    pub allocated_effect_value_fixed: i64,
    pub affected_division_count: u32,
    pub group_already_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerObservation {
    pub sequence: u64,
    pub controller_runtime: usize,
    pub context_mode: u32,
    pub selected_root_runtime: usize,
    pub selected_leader_runtime: usize,
    pub selected_army_runtime: usize,
    pub selected_army_group_runtime: usize,
    pub selected_other_runtime: usize,
    pub field_marshal_aggregate: bool,
    pub aggregate_leader_runtime: Vec<usize>,
    pub source_definition_runtime: Vec<usize>,
    pub definitions: Vec<DefinitionLeaves>,
    pub ordinary_leaves: Vec<OrdinaryAbilityLeaf>,
    pub field_marshal_leaves: Vec<FieldMarshalAbilityLeaf>,
    pub row_widget_count: u32,
    pub measured_empty: bool,
    pub probe_events: Vec<ProbeEvent>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub completed_refreshes: u64,
    pub rejected_refreshes: u64,
    pub nested_refreshes: u64,
    pub retained_observations: u64,
}

#[derive(Default)]
struct ObserverState {
    observations: VecDeque<ControllerObservation>,
    completed_refreshes: u64,
    rejected_refreshes: u64,
    nested_refreshes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub refresh: usize,
    pub context_invalid_probe: usize,
    pub active_cooldown_probe: usize,
    pub availability_evaluator: usize,
    pub special_context_probe: usize,
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics::default();
    };
    HookDiagnostics {
        completed_refreshes: state.completed_refreshes,
        rejected_refreshes: state.rejected_refreshes,
        nested_refreshes: state.nested_refreshes,
        retained_observations: state.observations.len() as u64,
    }
}

pub fn observations() -> Vec<ControllerObservation> {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map(|state| state.observations.iter().cloned().collect())
        .unwrap_or_default()
}

pub fn take_observations() -> Vec<ControllerObservation> {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map(|mut state| state.observations.drain(..).collect())
        .unwrap_or_default()
}

pub fn install_a729_ability_transition_hook() -> Result<InstalledHookAddresses, String> {
    if !EXACT_A729_AUTHORITY_BOUND {
        return Err("A36 exact-a729 static authority receipt is not bound".into());
    }
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(installed_addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let addresses = installed_addresses(base);
    for (label, address, sentinel) in [
        ("ability refresh", addresses.refresh, REFRESH_SENTINEL.as_slice()),
        ("context-invalid probe", addresses.context_invalid_probe, CONTEXT_PROBE_SENTINEL.as_slice()),
        ("active/cooldown probe", addresses.active_cooldown_probe, ACTIVE_PROBE_SENTINEL.as_slice()),
        ("availability evaluator", addresses.availability_evaluator, AVAILABILITY_SENTINEL.as_slice()),
        ("special-context probe", addresses.special_context_probe, SPECIAL_CONTEXT_SENTINEL.as_slice()),
    ] {
        verify_sentinel(label, address, sentinel)?;
    }
    STATE.get_or_init(|| Mutex::new(ObserverState::default()));
    let specs = [
        (addresses.context_invalid_probe, hook_context_probe as *mut c_void, &ORIGINAL_CONTEXT_PROBE),
        (addresses.active_cooldown_probe, hook_active_probe as *mut c_void, &ORIGINAL_ACTIVE_PROBE),
        (addresses.availability_evaluator, hook_availability as *mut c_void, &ORIGINAL_AVAILABILITY),
        (addresses.special_context_probe, hook_special_context as *mut c_void, &ORIGINAL_SPECIAL_CONTEXT),
        (addresses.refresh, hook_refresh as *mut c_void, &ORIGINAL_REFRESH),
    ];
    let mut created = 0usize;
    for &(target, detour, original_slot) in &specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(original) => {
                original_slot.store(original as usize, Ordering::Release);
                created += 1;
            }
            Err(status) => {
                rollback(&specs, created);
                return Err(format!("A36 hook creation failed at 0x{target:X}: {status}"));
            }
        }
    }
    for (enabled, (target, _, _)) in specs.iter().enumerate() {
        if let Err(status) = unsafe { MinHook::enable_hook(*target as *mut c_void) } {
            for (prior, _, _) in specs.iter().take(enabled) {
                let _ = unsafe { MinHook::disable_hook(*prior as *mut c_void) };
            }
            rollback(&specs, created);
            return Err(format!("A36 hook enable failed at 0x{target:X}: {status}"));
        }
    }
    MODULE_BASE.store(base, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(addresses)
}

fn rollback(specs: &[(usize, *mut c_void, &AtomicUsize); 5], created: usize) {
    for (target, _, original) in specs.iter().take(created).rev() {
        let _ = unsafe { MinHook::remove_hook(*target as *mut c_void) };
        original.store(0, Ordering::Release);
    }
}

fn installed_addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        refresh: base + ABILITY_REFRESH_RVA as usize,
        context_invalid_probe: base + CONTEXT_INVALID_PROBE_RVA as usize,
        active_cooldown_probe: base + ACTIVE_COOLDOWN_PROBE_RVA as usize,
        availability_evaluator: base + AVAILABILITY_EVALUATOR_RVA as usize,
        special_context_probe: base + SPECIAL_CONTEXT_PROBE_RVA as usize,
    }
}

unsafe extern "system" fn hook_refresh(controller: usize) {
    let nested = TRACE.with(|trace| {
        let mut trace = trace.borrow_mut();
        if let Some(open) = trace.as_mut() {
            open.nested_or_replaced = true;
            true
        } else {
            *trace = Some(OpenTrace {
                controller_runtime: controller,
                events: Vec::new(),
                nested_or_replaced: false,
            });
            false
        }
    });
    if nested {
        reject_nested();
    }
    let original = ORIGINAL_REFRESH.load(Ordering::Acquire);
    if original != 0 {
        let original: RefreshFn = unsafe { mem::transmute(original) };
        unsafe { original(controller) };
    }
    if nested {
        return;
    }
    let trace = TRACE.with(|trace| trace.borrow_mut().take());
    match trace.and_then(|trace| build_observation(trace).ok()) {
        Some(observation) => retain(observation),
        None => reject_refresh(),
    }
}

unsafe extern "system" fn hook_context_probe(context: usize) -> u8 {
    let original: ContextProbeFn = unsafe {
        mem::transmute(ORIGINAL_CONTEXT_PROBE.load(Ordering::Acquire))
    };
    let result = unsafe { original(context) };
    record_event(ProbeEvent::ContextInvalid { invalid: result != 0 });
    result
}

unsafe extern "system" fn hook_active_probe(
    runtime: usize,
    definition: usize,
    scope: usize,
    cooldown_out: *mut u8,
    elapsed_out: *mut i32,
) -> usize {
    let original: ActiveProbeFn = unsafe {
        mem::transmute(ORIGINAL_ACTIVE_PROBE.load(Ordering::Acquire))
    };
    let result = unsafe { original(runtime, definition, scope, cooldown_out, elapsed_out) };
    let cooldown = read_local_u8(cooldown_out).is_some_and(|value| value != 0);
    let elapsed = read_local_i32(elapsed_out).unwrap_or(-1);
    record_event(ProbeEvent::ActiveCooldown {
        definition_runtime: definition,
        scope_runtime: scope,
        active: result as u8 != 0,
        cooldown,
        elapsed_since_activation_hours: elapsed,
    });
    result
}

unsafe extern "system" fn hook_availability(
    runtime: usize,
    definition: usize,
    scope: usize,
    resolved_text: usize,
) -> usize {
    let original: AvailabilityFn = unsafe {
        mem::transmute(ORIGINAL_AVAILABILITY.load(Ordering::Acquire))
    };
    let result = unsafe { original(runtime, definition, scope, resolved_text) };
    record_event(ProbeEvent::Availability {
        definition_runtime: definition,
        scope_runtime: scope,
        available: result as u8 != 0,
    });
    result
}

unsafe extern "system" fn hook_special_context(controller: usize) -> u8 {
    let original: SpecialContextFn = unsafe {
        mem::transmute(ORIGINAL_SPECIAL_CONTEXT.load(Ordering::Acquire))
    };
    let result = unsafe { original(controller) };
    record_event(ProbeEvent::SpecialContext {
        controller_runtime: controller,
        allowed: result != 0,
    });
    result
}

fn record_event(event: ProbeEvent) {
    TRACE.with(|trace| {
        if let Some(open) = trace.borrow_mut().as_mut() {
            open.events.push(event);
        }
    });
}

fn build_observation(trace: OpenTrace) -> Result<ControllerObservation, String> {
    if trace.controller_runtime == 0 || trace.nested_or_replaced {
        return Err("null/nested ability refresh".into());
    }
    if !matches!(trace.events.first(), Some(ProbeEvent::ContextInvalid { invalid: false })) {
        return Err("ability refresh did not pass its exact context-validity gate".into());
    }
    let (meta, source_bytes, state_bytes, leader_bytes, row_widget_bytes) =
        stable_snapshot(trace.controller_runtime)?;
    validate_header(meta.source, 8, "source")?;
    validate_header(meta.state, FM_STATE_STRIDE, "field-marshal state")?;
    validate_header(meta.leaders, 8, "field-marshal leader")?;
    validate_header(meta.row_widgets, 8, "row widget")?;
    if meta.row_widgets.count != meta.source.count {
        return Err("row widget count differs from exact source order".into());
    }
    let sources = pointer_rows(&source_bytes)?;
    let aggregate_leaders = pointer_rows(&leader_bytes)?;
    if aggregate_leaders.iter().any(|pointer| *pointer == 0) {
        return Err("field-marshal leader vector contains a null identity".into());
    }
    if pointer_rows(&row_widget_bytes)?.iter().any(|pointer| *pointer == 0) {
        return Err("row widget vector contains a null owner".into());
    }
    let mut definitions = Vec::with_capacity(sources.len());
    for definition in &sources {
        definitions.push(read_definition(*definition)?);
    }
    let field_marshal_leaves = parse_field_marshal_states(&state_bytes)?;
    let state_definitions = field_marshal_leaves
        .iter()
        .map(|row| row.definition_runtime)
        .collect::<Vec<_>>();
    if meta.field_marshal_aggregate {
        if state_definitions != sources {
            return Err("field-marshal state records differ from source order".into());
        }
    } else if !state_definitions.is_empty() {
        return Err("ordinary refresh retained field-marshal state records".into());
    }
    let ordinary_leaves = if !meta.field_marshal_aggregate {
        reconcile_ordinary(
            trace.controller_runtime,
            &sources,
            &definitions,
            &trace.events,
        )?
    } else {
        Vec::new()
    };
    let has_selected_identity = meta.selected_leader != 0
        || meta.selected_army != 0
        || meta.selected_army_group != 0
        || meta.selected_other != 0;
    let measured_empty = meta.context_mode != 7
        && has_selected_identity
        && meta.source.count == 0
        && meta.state.count == 0
        && meta.row_widgets.count == 0;
    Ok(ControllerObservation {
        sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        controller_runtime: trace.controller_runtime,
        context_mode: meta.context_mode,
        selected_root_runtime: meta.selected_root,
        selected_leader_runtime: meta.selected_leader,
        selected_army_runtime: meta.selected_army,
        selected_army_group_runtime: meta.selected_army_group,
        selected_other_runtime: meta.selected_other,
        field_marshal_aggregate: meta.field_marshal_aggregate,
        aggregate_leader_runtime: aggregate_leaders,
        source_definition_runtime: sources,
        definitions,
        ordinary_leaves,
        field_marshal_leaves,
        row_widget_count: meta.row_widgets.count,
        measured_empty,
        probe_events: trace.events,
    })
}

fn reconcile_ordinary(
    controller: usize,
    sources: &[usize],
    definitions: &[DefinitionLeaves],
    events: &[ProbeEvent],
) -> Result<Vec<OrdinaryAbilityLeaf>, String> {
    let mut active_events = events.iter().filter_map(|event| match event {
        ProbeEvent::ActiveCooldown {
            definition_runtime,
            scope_runtime,
            active,
            cooldown,
            elapsed_since_activation_hours,
        } => Some((
            *definition_runtime,
            *scope_runtime,
            *active,
            *cooldown,
            *elapsed_since_activation_hours,
        )),
        _ => None,
    });
    let special_results = events
        .iter()
        .filter_map(|event| match event {
            ProbeEvent::SpecialContext {
                controller_runtime,
                allowed,
            } => Some((*controller_runtime, *allowed)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if special_results
        .iter()
        .any(|(observed_controller, _)| *observed_controller != controller)
    {
        return Err("special-context probe used a different controller".into());
    }
    let special_result = special_results.first().map(|(_, allowed)| *allowed);
    if special_results
        .iter()
        .any(|(_, allowed)| Some(*allowed) != special_result)
    {
        return Err("special-context probes disagreed within one refresh".into());
    }
    for event in events {
        if let ProbeEvent::Availability {
            definition_runtime,
            ..
        } = event
        {
            if !sources.contains(definition_runtime) {
                return Err("availability probe referenced a definition outside the source vector".into());
            }
        }
    }
    let mut out = Vec::with_capacity(sources.len());
    for (definition, leaves) in sources.iter().zip(definitions) {
        let (observed, scope, active, cooldown, elapsed) = active_events
            .next()
            .ok_or_else(|| "ordinary row missing active/cooldown probe".to_string())?;
        if observed != *definition {
            return Err("ordinary active/cooldown probes differ from source order".into());
        }
        let matching_availability = events
            .iter()
            .filter_map(|event| match event {
                ProbeEvent::Availability {
                    definition_runtime,
                    scope_runtime,
                    available,
                } if definition_runtime == definition => Some((*scope_runtime, *available)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if matching_availability.len() > 1 {
            return Err("ordinary row emitted duplicate availability probes".into());
        }
        if matching_availability
            .first()
            .is_some_and(|(availability_scope, _)| *availability_scope != scope)
        {
            return Err("ordinary availability scope differs from active/cooldown scope".into());
        }
        let availability = matching_availability.first().map(|(_, available)| *available);
        let special = leaves.special_context_required.then_some(special_result).flatten();
        if !active && !cooldown && leaves.special_context_required && special.is_none() {
            return Err("ordinary inactive row lacks its required special-context leaf".into());
        }
        let state = if active {
            OrdinaryAbilityState::Active
        } else if cooldown {
            OrdinaryAbilityState::Cooldown
        } else if special == Some(false) || availability == Some(false) {
            OrdinaryAbilityState::Unavailable
        } else if availability == Some(true) {
            OrdinaryAbilityState::Available
        } else {
            return Err("ordinary inactive row lacks an exact availability leaf".into());
        };
        out.push(OrdinaryAbilityLeaf {
            definition_runtime: *definition,
            scope_runtime: scope,
            state,
            activation_elapsed_hours: elapsed,
            availability_evaluated: availability,
            special_context_allowed: special,
        });
    }
    if active_events.next().is_some() {
        return Err("ordinary refresh emitted extra active/cooldown probes".into());
    }
    Ok(out)
}

fn stable_snapshot(
    controller: usize,
) -> Result<(ControllerMeta, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>), String> {
    let before = read_meta(controller)?;
    let source_first = read_vector(before.source, 8)?;
    let state_first = read_vector(before.state, FM_STATE_STRIDE)?;
    let leaders_first = read_vector(before.leaders, 8)?;
    let rows_first = read_vector(before.row_widgets, 8)?;
    let source = read_vector(before.source, 8)?;
    let state = read_vector(before.state, FM_STATE_STRIDE)?;
    let leaders = read_vector(before.leaders, 8)?;
    let rows = read_vector(before.row_widgets, 8)?;
    let after = read_meta(controller)?;
    if before != after
        || source_first != source
        || state_first != state
        || leaders_first != leaders
        || rows_first != rows
    {
        return Err("ability controller changed during snapshot".into());
    }
    Ok((after, source, state, leaders, rows))
}

fn read_meta(controller: usize) -> Result<ControllerMeta, String> {
    Ok(ControllerMeta {
        source: read_header(controller + 0x718)?,
        state: read_header(controller + 0x790)?,
        leaders: read_header(controller + 0x778)?,
        row_widgets: read_header(controller + 0x70)?,
        context_mode: read_u32(controller + 0x730)?,
        selected_root: read_usize(controller + 0x738)?,
        selected_leader: read_usize(controller + 0x748)?,
        selected_army: read_usize(controller + 0x750)?,
        selected_army_group: read_usize(controller + 0x758)?,
        selected_other: read_usize(controller + 0x760)?,
        field_marshal_aggregate: read_u8(controller + 0x770)? != 0,
    })
}

fn read_header(address: usize) -> Result<VectorHeader, String> {
    let bytes = rpm(address, 16).ok_or_else(|| "ability vector header unreadable".to_string())?;
    Ok(VectorHeader {
        data: le_usize(&bytes, 0)?,
        capacity: le_u32(&bytes, 8)?,
        count: le_u32(&bytes, 12)?,
    })
}

fn validate_header(header: VectorHeader, stride: usize, label: &str) -> Result<(), String> {
    if header.count as usize > MAX_ROWS || header.capacity < header.count {
        return Err(format!("{label} vector count/capacity invalid"));
    }
    if header.count != 0 && header.data == 0 {
        return Err(format!("{label} vector has null data"));
    }
    (header.count as usize)
        .checked_mul(stride)
        .ok_or_else(|| format!("{label} vector size overflow"))?;
    Ok(())
}

fn read_vector(header: VectorHeader, stride: usize) -> Result<Vec<u8>, String> {
    validate_header(header, stride, "snapshot")?;
    let len = header.count as usize * stride;
    if len == 0 {
        return Ok(Vec::new());
    }
    rpm(header.data, len).ok_or_else(|| "ability vector body unreadable".into())
}

fn pointer_rows(bytes: &[u8]) -> Result<Vec<usize>, String> {
    bytes.chunks_exact(8).map(|row| le_usize(row, 0)).collect()
}

fn read_definition(definition: usize) -> Result<DefinitionLeaves, String> {
    if definition == 0 {
        return Err("null ability definition".into());
    }
    let bytes = rpm(definition + 0xa8, 0x242)
        .ok_or_else(|| "ability definition leaves unreadable".to_string())?;
    Ok(DefinitionLeaves {
        definition_runtime: definition,
        duration_hours: le_i32(&bytes, 0)?,
        cooldown_hours: le_i32(&bytes, 4)?,
        cancelable: *bytes.get(0x240).ok_or("ability cancelable leaf missing")? != 0,
        special_context_required: *bytes.get(0x241).ok_or("ability special-context leaf missing")? != 0,
    })
}

fn parse_field_marshal_states(bytes: &[u8]) -> Result<Vec<FieldMarshalAbilityLeaf>, String> {
    let mut out = Vec::new();
    for record in bytes.chunks_exact(FM_STATE_STRIDE) {
        let definition = le_usize(record, 0)?;
        let leader_data = le_usize(record, 8)?;
        let leader_capacity = le_u32(record, 0x10)? as usize;
        let leader_count = le_u32(record, 0x14)? as usize;
        if definition == 0
            || leader_count > MAX_ROWS
            || leader_capacity < leader_count
            || (leader_count != 0 && leader_data == 0)
        {
            return Err("invalid field-marshal state identity/vector".into());
        }
        let leader_bytes = if leader_count == 0 {
            Vec::new()
        } else {
            let len = leader_count * FM_LEADER_STATE_STRIDE;
            let first = rpm(leader_data, len)
                .ok_or_else(|| "field-marshal leader states unreadable".to_string())?;
            let second = rpm(leader_data, len)
                .ok_or_else(|| "field-marshal leader states unreadable".to_string())?;
            if first != second {
                return Err("field-marshal leader states changed during snapshot".into());
            }
            second
        };
        let mut leaders = Vec::with_capacity(leader_count);
        for row in leader_bytes.chunks_exact(FM_LEADER_STATE_STRIDE) {
            let state_code = *row.get(8).ok_or("field-marshal state code missing")?;
            if state_code > 4 {
                return Err("field-marshal state code outside 0..4".into());
            }
            let leader_runtime = le_usize(row, 0)?;
            if leader_runtime == 0 {
                return Err("field-marshal leader state has a null identity".into());
            }
            leaders.push(FieldMarshalLeaderLeaf {
                leader_runtime,
                state_code,
                remaining_hours: le_i32(row, 0x0c)?,
                command_power_cost_fixed: le_i64(row, 0x10)?,
                daily_command_power_cost_fixed: le_i64(row, 0x18)?,
                will_activate: *row.get(0x20).ok_or("field-marshal allocation leaf missing")? != 0,
            });
        }
        out.push(FieldMarshalAbilityLeaf {
            definition_runtime: definition,
            leaders,
            available_count: le_u32(record, 0x20)?,
            active_count: le_u32(record, 0x24)?,
            cooldown_count: le_u32(record, 0x28)?,
            allocated_count: le_u32(record, 0x2c)?,
            candidate_leader_count: le_u32(record, 0x30)?,
            definition_allowed_leader_count: le_u32(record, 0x34)?,
            minimum_active_remaining_hours: le_i32(record, 0x38)?,
            minimum_cooldown_remaining_hours: le_i32(record, 0x3c)?,
            group_activation_elapsed_hours: le_i32(record, 0x40)?,
            allocated_command_power_cost_fixed: le_i64(record, 0x48)?,
            allocated_daily_command_power_cost_fixed: le_i64(record, 0x50)?,
            allocated_effect_value_fixed: le_i64(record, 0x58)?,
            affected_division_count: le_u32(record, 0x60)?,
            group_already_active: *record.get(0x64).ok_or("field-marshal group-active leaf missing")? != 0,
        });
    }
    Ok(out)
}

fn retain(observation: ControllerObservation) {
    let Ok(mut state) = STATE.get_or_init(|| Mutex::new(ObserverState::default())).lock() else {
        return;
    };
    state.completed_refreshes = state.completed_refreshes.saturating_add(1);
    if state.observations.len() >= MAX_OBSERVATIONS {
        state.observations.pop_front();
    }
    state.observations.push_back(observation);
}

fn reject_refresh() {
    if let Ok(mut state) = STATE.get_or_init(|| Mutex::new(ObserverState::default())).lock() {
        state.rejected_refreshes = state.rejected_refreshes.saturating_add(1);
    }
}

fn reject_nested() {
    if let Ok(mut state) = STATE.get_or_init(|| Mutex::new(ObserverState::default())).lock() {
        state.nested_refreshes = state.nested_refreshes.saturating_add(1);
    }
}

fn verify_sentinel(label: &str, address: usize, expected: &[u8]) -> Result<(), String> {
    let actual = rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn read_local_u8(address: *mut u8) -> Option<u8> {
    rpm(address as usize, 1).and_then(|bytes| bytes.first().copied())
}

fn read_local_i32(address: *mut i32) -> Option<i32> {
    rpm(address as usize, 4).and_then(|bytes| le_i32(&bytes, 0).ok())
}

fn read_u8(address: usize) -> Result<u8, String> {
    rpm(address, 1).and_then(|bytes| bytes.first().copied()).ok_or_else(|| "u8 unreadable".into())
}

fn read_u32(address: usize) -> Result<u32, String> {
    let bytes = rpm(address, 4).ok_or_else(|| "u32 unreadable".to_string())?;
    le_u32(&bytes, 0)
}

fn read_usize(address: usize) -> Result<usize, String> {
    let bytes = rpm(address, 8).ok_or_else(|| "pointer unreadable".to_string())?;
    le_usize(&bytes, 0)
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let raw: [u8; 4] = bytes.get(offset..offset + 4).ok_or("u32 truncated")?.try_into().map_err(|_| "u32 width")?;
    Ok(u32::from_le_bytes(raw))
}

fn le_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    Ok(le_u32(bytes, offset)? as i32)
}

fn le_i64(bytes: &[u8], offset: usize) -> Result<i64, String> {
    let raw: [u8; 8] = bytes.get(offset..offset + 8).ok_or("i64 truncated")?.try_into().map_err(|_| "i64 width")?;
    Ok(i64::from_le_bytes(raw))
}

fn le_usize(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let raw: [u8; 8] = bytes.get(offset..offset + 8).ok_or("pointer truncated")?.try_into().map_err(|_| "pointer width")?;
    Ok(u64::from_le_bytes(raw) as usize)
}

fn rpm(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 || len == 0 {
        return (len == 0).then(Vec::new);
    }
    let mut out = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(out)
}
