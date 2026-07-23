//! Exact production a729 ability-list transition observer.
//!
//! This module records the exact engine leaves needed by the A36 human
//! transition matrix and captures the exact same-row disabled-reason
//! accumulator before its destruction when such text is evaluated. Runtime
//! pointers and aggregate scratch records remain process-local join identities.
//! Only `JoinedExactA729AbilitySource`,
//! produced after a complete pointer-erasing row join, implements
//! `ExactA729AbilityControllerSource`. Core rows do not require optional text.

use core::ffi::c_void;
use minhook::MinHook;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::{
    ReadProcessMemory, RtlCaptureStackBackTrace,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const ABILITY_REFRESH_RVA: u32 = 0x01D0_72F0;
pub const CONTEXT_INVALID_PROBE_RVA: u32 = 0x00B5_8CA0;
pub const ACTIVE_COOLDOWN_PROBE_RVA: u32 = 0x006F_3370;
pub const EFFECT_SCOPE_CASTER_RESOLVER_RVA: u32 = 0x0052_94F0;
pub const AVAILABILITY_EVALUATOR_RVA: u32 = 0x006D_0DE0;
pub const SPECIAL_CONTEXT_PROBE_RVA: u32 = 0x01D0_6480;
pub const TOOLTIP_BUILDER_RVA: u32 = 0x01D0_49C0;
pub const STRING_APPEND_RVA: u32 = 0x0012_9700;
pub const EFFECTIVE_COMMAND_POWER_COST_RVA: u32 = 0x00FF_4C30;
pub const ICON_RESOLVER_RVA: u32 = 0x0222_3EB0;
/// Return address immediately after the one append which copies the temporary
/// disabled-reason accumulator into the final tooltip. The accumulator is the
/// exact source argument at this call and is still alive until 0x141D063A3.
pub const DISABLED_REASON_APPEND_RETURN_RVA: u32 = 0x01D0_6057;
pub const EFFECTIVE_COST_RETURN_RVA: u32 = 0x01D0_4E38;
pub const ICON_RESOLVER_RETURN_RVA: u32 = 0x01D0_8BA6;

/// Static hook authority bound to the mechanically reconciled a729 receipt.
/// This opens diagnostic observer installation only; it does not promote ABL1
/// production output or schedule a compact domain.
pub const EXACT_A729_AUTHORITY_BOUND: bool = true;
pub const EXACT_A729_AUTHORITY_JOB_ID: &str =
    "20260720T020812Z-w3-a36-ability-list-feb98226";
pub const EXACT_A729_AUTHORITY_SHARD: &str = "shard-19";
pub const EXACT_A729_AUTHORITY_MANIFEST_SHA256: &str =
    "12768dfa9a8b47ceda4feae9cdc45b76811f602ecd557008aa936e95adf2db45";
/// Follow-up receipts which authenticate the corrected tooltip/timer laws and
/// the final selected-context root census used by this observer.  The original
/// exact20 receipt remains the complete hook-site census; these receipts are
/// additive authority and must travel with every live evidence receipt.
pub const FINAL_STATIC_AUDIT_JOB_ID: &str =
    "20260720T102325Z-w5-a36-final-tooltip-timer-audit-dc624c9b";
pub const FINAL_STATIC_AUDIT_SHARD: &str = "shard-06";
pub const FINAL_STATIC_AUDIT_MANIFEST_SHA256: &str =
    "6e3ce8c70d50d98256083b4a054d681b1302e373f1c299dba72ff0fd8406d386";
pub const FINAL_ROOTS_JOB_ID: &str =
    "20260720T134743Z-wave5-a36-selected-context-final-roots-f5a32492";
pub const FINAL_ROOTS_SHARD: &str = "shard-19";
pub const FINAL_ROOTS_MANIFEST_SHA256: &str =
    "504e0fda71c6d0071683425e45edc7cc8b22734aaadfaf05f00769b11a70b021";
pub const TERMINAL_ACTIVE_RECORD_JOB_ID: &str =
    "20260721T100154Z-terminal-a36-active-record-minimal-autho-a78da095";
pub const TERMINAL_ACTIVE_RECORD_SHARD: &str = "shard-08";
pub const TERMINAL_ACTIVE_RECORD_MANIFEST_SHA256: &str =
    "1847eb5ee55bb7358b20c8953f8d8b2969c81c306222ae6d7d08ff84922336d5";
pub const TERMINAL_ACTIVE_RECORD_AUTHORITY_BOUND: bool = true;
pub const EXACT_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const LIVE_TRANSITION_PROMOTION_BOUND: bool = true;
pub const COMPACT_DOMAIN_SCHEDULED: bool = true;

const REFRESH_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
];
const CONTEXT_PROBE_SENTINEL: [u8; 16] = [
    0x80, 0xb9, 0xfa, 0x03, 0x00, 0x00, 0x00, 0x74, 0x0c, 0x48, 0x8b, 0x89, 0x40, 0x02, 0x00, 0x00,
];
const ACTIVE_PROBE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
const EFFECT_SCOPE_CASTER_RESOLVER_SENTINEL: [u8; 16] = [
    0x48, 0x83, 0xec, 0x28, 0x48, 0x83, 0xc1, 0x50, 0x83, 0x39, 0x00, 0x75, 0x06, 0x83, 0x79, 0x04,
];
const AVAILABILITY_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x4c, 0x89, 0x48, 0x20, 0x48, 0x89, 0x50, 0x10, 0x48,
];
const SPECIAL_CONTEXT_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xb9, 0x39, 0x00, 0x00, 0x00, 0xe8, 0xad,
];
const TOOLTIP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
const STRING_APPEND_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xf9, 0x49, 0x8b, 0xf0,
];
const DISABLED_REASON_CALLSITE_SENTINEL: [u8; 16] = [
    0xe8, 0xa9, 0x36, 0x42, 0xfe, 0x48, 0x8b, 0x4e, 0x10, 0x48, 0x85, 0xc9, 0x74, 0x54, 0x4c, 0x8b,
];
const EFFECTIVE_COST_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
const EFFECTIVE_COST_CALLSITE_SENTINEL: [u8; 16] = [
    0xe8, 0xf8, 0xfd, 0x2e, 0xff, 0x48, 0x8b, 0x9b, 0xf0, 0x01, 0x00, 0x00, 0x45, 0x38, 0xa6, 0xe9,
];
const ICON_RESOLVER_SENTINEL: [u8; 16] = [
    0x48, 0x81, 0xc1, 0x00, 0x01, 0x00, 0x00, 0xe9, 0x14, 0x92, 0x80, 0xfe, 0xcc, 0xcc, 0xcc, 0xcc,
];
const ICON_RESOLVER_CALLSITE_SENTINEL: [u8; 16] = [
    0xe8, 0x0a, 0xb3, 0x51, 0x00, 0x48, 0x85, 0xc0, 0x0f, 0x85, 0x28, 0x01, 0x00, 0x00, 0x38, 0x05,
];

const MAX_ROWS: usize = 4096;
const MAX_OBSERVATIONS: usize = 128;
const MAX_TEXT_OBSERVATIONS: usize = 256;
const MAX_TEXT_BYTES: usize = 4096;
const MAX_TOTAL_FIELD_MARSHAL_LEADERS: usize = 65_536;
const FM_STATE_STRIDE: usize = 0x68;
const FM_LEADER_STATE_STRIDE: usize = 0x28;
const ACTIVE_RECORD_STRIDE: usize = 0x58;
const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const CGAMESTATE_CURRENT_DATE_OFF: usize = 0x468;
pub const HOOK_COUNT: usize = 10;

type RefreshFn = unsafe extern "system" fn(usize);
type ContextProbeFn = unsafe extern "system" fn(usize) -> u8;
type ActiveProbeFn = unsafe extern "system" fn(usize, usize, usize, *mut u8, *mut i32) -> usize;
type EffectScopeCasterResolverFn = unsafe extern "system" fn(usize) -> usize;
type AvailabilityFn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
type SpecialContextFn = unsafe extern "system" fn(usize) -> u8;
type TooltipFn = unsafe extern "system" fn(usize, usize, usize) -> usize;
type StringAppendFn = unsafe extern "system" fn(usize, usize, usize) -> usize;
type EffectiveCostFn = unsafe extern "system" fn(usize, *mut i64, usize) -> usize;
type IconResolverFn = unsafe extern "system" fn(usize, usize) -> usize;

static ORIGINAL_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CONTEXT_PROBE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ACTIVE_PROBE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_EFFECT_SCOPE_CASTER_RESOLVER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_AVAILABILITY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SPECIAL_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TOOLTIP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_STRING_APPEND: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_EFFECTIVE_COST: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ICON_RESOLVER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static INSTALL_BROKEN: AtomicBool = AtomicBool::new(false);
static DRAINING: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static STATE: OnceLock<Mutex<ObserverState>> = OnceLock::new();
static INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

thread_local! {
    static TRACE: RefCell<Option<OpenTrace>> = const { RefCell::new(None) };
    static ACTIVE_PROBE_TRACE: RefCell<Option<OpenActiveProbeTrace>> = const { RefCell::new(None) };
    static TOOLTIP_TRACE: RefCell<Option<OpenTooltipTrace>> = const { RefCell::new(None) };
}

struct CallbackGuard {
    capture: bool,
}

impl CallbackGuard {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self {
            capture: !DRAINING.load(Ordering::Acquire),
        }
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
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
        record: ActiveRecordWitness,
    },
    Availability {
        country_runtime: usize,
        definition_runtime: usize,
        scope_runtime: usize,
        available: bool,
    },
    SpecialContext { controller_runtime: usize, allowed: bool },
    IconResolved { icon: String, resolved: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenTrace {
    controller_runtime: usize,
    events: Vec<ProbeEvent>,
    nested_or_replaced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenActiveProbeTrace {
    country_runtime: usize,
    definition_runtime: usize,
    effect_scope_runtime: usize,
    resolved_caster_runtime: Option<usize>,
    duplicate_or_nested: bool,
}

/// Exact CCountry active-record identity consumed by `0x1406F3370`.
/// Runtime addresses are diagnostic join keys and never enter ABL1/ABA2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveRecordWitness {
    pub country_runtime: usize,
    pub effect_scope_runtime: usize,
    pub resolved_caster_runtime: usize,
    pub effect_group_target_runtime: usize,
    pub record_runtime: Option<usize>,
    pub record_caster_runtime: Option<usize>,
    pub record_group_target_runtime: Option<usize>,
    pub record_pass_active: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TooltipRuntimeIdentity {
    observed_game_hour_raw: i32,
    controller_sequence: u64,
    callback_owner_runtime: usize,
    hovered_widget_runtime: usize,
    tooltip_output_runtime: usize,
    controller_runtime: usize,
    context_mode: u32,
    selected_root_runtime: usize,
    selected_leader_runtime: usize,
    selected_army_runtime: usize,
    selected_army_group_runtime: usize,
    selected_other_runtime: usize,
    field_marshal_aggregate: bool,
    row_ordinal: u32,
    definition_runtime: usize,
    source_definition_runtime: Vec<usize>,
    row_owner_runtime: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenTooltipTrace {
    identity: TooltipRuntimeIdentity,
    disabled_reason_bytes: Option<Vec<u8>>,
    effective_command_power_cost_fixed: Option<i64>,
    duplicate_effective_cost: bool,
    accumulator_callsite_seen: bool,
    duplicate_accumulator_append: bool,
    nested_or_replaced: bool,
}

/// Diagnostic-only exact same-row text observation. Runtime addresses are
/// process-local join keys and are never serialized into ABL1 or ABA2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisabledReasonObservation {
    pub sequence: u64,
    pub observed_game_hour_raw: i32,
    pub controller_sequence: u64,
    pub controller_runtime: usize,
    pub callback_owner_runtime: usize,
    pub hovered_widget_runtime: usize,
    pub context_mode: u32,
    pub selected_root_runtime: usize,
    pub selected_leader_runtime: usize,
    pub selected_army_runtime: usize,
    pub selected_army_group_runtime: usize,
    pub selected_other_runtime: usize,
    pub field_marshal_aggregate: bool,
    pub row_ordinal: u32,
    pub definition_runtime: usize,
    pub source_definition_runtime: Vec<usize>,
    pub row_owner_runtime: Vec<usize>,
    pub final_tooltip: String,
    /// Exact evaluated accumulator bytes before the final-tooltip append. An
    /// empty string is a measured empty accumulator, not a missing value.
    pub disabled_reason: String,
    pub accumulator_callsite_seen: bool,
    /// Exact fixed-point result written by 0x140FF4C30 at the unique ordinary
    /// tooltip call returning at 0x141D04E38. Field-marshal rows use the
    /// already-retained aggregate cost record instead.
    pub effective_command_power_cost_fixed: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionPresentation {
    pub definition_runtime: usize,
    pub ability_id: String,
    pub resolved_icon: String,
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
    /// Exact elapsed value returned by the same ordered 0x1406F3370 call
    /// which produced this leader state. The 0x28 aggregate row does not
    /// retain it for indefinite abilities, so the observer joins it by the
    /// engine's leader iteration order before pointer erasure.
    pub activation_elapsed_hours: i32,
    pub command_power_cost_fixed: i64,
    pub activation_command_power_cost_fixed: i64,
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
    pub allocated_activation_command_power_cost_fixed: i64,
    pub allocated_daily_command_power_cost_fixed: i64,
    pub allocated_affected_division_count: u32,
    pub group_already_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerObservation {
    pub sequence: u64,
    pub observed_game_hour_raw: i32,
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
    pub row_owner_runtime: Vec<usize>,
    pub definitions: Vec<DefinitionLeaves>,
    pub definition_presentations: Vec<DefinitionPresentation>,
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
    pub completed_tooltips: u64,
    pub rejected_tooltips: u64,
    pub nested_tooltips: u64,
    pub retained_disabled_reasons: u64,
}

#[derive(Default)]
struct ObserverState {
    observations: VecDeque<ControllerObservation>,
    disabled_reasons: VecDeque<DisabledReasonObservation>,
    completed_refreshes: u64,
    rejected_refreshes: u64,
    nested_refreshes: u64,
    completed_tooltips: u64,
    rejected_tooltips: u64,
    nested_tooltips: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub refresh: usize,
    pub context_invalid_probe: usize,
    pub active_cooldown_probe: usize,
    pub effect_scope_caster_resolver: usize,
    pub availability_evaluator: usize,
    pub special_context_probe: usize,
    pub tooltip_builder: usize,
    pub string_append: usize,
    pub effective_command_power_cost: usize,
    pub icon_resolver: usize,
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
        completed_tooltips: state.completed_tooltips,
        rejected_tooltips: state.rejected_tooltips,
        nested_tooltips: state.nested_tooltips,
        retained_disabled_reasons: state.disabled_reasons.len() as u64,
    }
}

pub fn observations() -> Vec<ControllerObservation> {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map(|state| state.observations.iter().cloned().collect())
        .unwrap_or_default()
}

/// Re-authenticates the process-local controller identity at the collector
/// root-catalog boundary. The hook observation remains the sole state/text
/// producer, but a later root join may not reuse it after selection, source
/// order, aggregate membership, or row ownership has changed.
pub fn observation_identity_still_current(
    observation: &ControllerObservation,
) -> Result<(), String> {
    let current_game_hour_raw = current_game_hour_raw()?;
    if current_game_hour_raw != observation.observed_game_hour_raw {
        return Err("A36 retained refresh crossed a game-hour boundary before exact-root join".into());
    }
    let (meta, source_bytes, state_bytes, leader_bytes, row_widget_bytes) =
        stable_snapshot(observation.controller_runtime)?;
    let source = pointer_rows(&source_bytes)?;
    let leaders = pointer_rows(&leader_bytes)?;
    let row_owners = pointer_rows(&row_widget_bytes)?;
    if meta.context_mode != observation.context_mode
        || meta.selected_root != observation.selected_root_runtime
        || meta.selected_leader != observation.selected_leader_runtime
        || meta.selected_army != observation.selected_army_runtime
        || meta.selected_army_group != observation.selected_army_group_runtime
        || meta.selected_other != observation.selected_other_runtime
        || meta.field_marshal_aggregate != observation.field_marshal_aggregate
        || source != observation.source_definition_runtime
        || leaders != observation.aggregate_leader_runtime
        || row_owners != observation.row_owner_runtime
        || meta.row_widgets.count != observation.row_widget_count
    {
        return Err("A36 retained refresh identity changed before exact-root join".into());
    }
    if observation.field_marshal_aggregate {
        let current = parse_field_marshal_states(&state_bytes)?;
        if current.len() != observation.field_marshal_leaves.len()
            || !current.iter().zip(&observation.field_marshal_leaves).all(|(left, right)| {
                left.definition_runtime == right.definition_runtime
                    && left.leaders.iter().zip(&right.leaders).all(|(l, r)| {
                        l.leader_runtime == r.leader_runtime
                            && l.state_code == r.state_code
                            && l.remaining_hours == r.remaining_hours
                            && l.command_power_cost_fixed == r.command_power_cost_fixed
                            && l.activation_command_power_cost_fixed
                                == r.activation_command_power_cost_fixed
                            && l.will_activate == r.will_activate
                    })
                    && left.leaders.len() == right.leaders.len()
                    && left.available_count == right.available_count
                    && left.active_count == right.active_count
                    && left.cooldown_count == right.cooldown_count
                    && left.allocated_count == right.allocated_count
                    && left.candidate_leader_count == right.candidate_leader_count
                    && left.definition_allowed_leader_count
                        == right.definition_allowed_leader_count
                    && left.minimum_active_remaining_hours
                        == right.minimum_active_remaining_hours
                    && left.minimum_cooldown_remaining_hours
                        == right.minimum_cooldown_remaining_hours
                    && left.group_activation_elapsed_hours
                        == right.group_activation_elapsed_hours
                    && left.allocated_command_power_cost_fixed
                        == right.allocated_command_power_cost_fixed
                    && left.allocated_activation_command_power_cost_fixed
                        == right.allocated_activation_command_power_cost_fixed
                    && left.allocated_daily_command_power_cost_fixed
                        == right.allocated_daily_command_power_cost_fixed
                    && left.allocated_affected_division_count
                        == right.allocated_affected_division_count
                    && left.group_already_active == right.group_already_active
            })
        {
            return Err("A36 retained field-marshal state changed before exact-root join".into());
        }
    } else {
        for leaf in &observation.ordinary_leaves {
            let matching = observation.probe_events.iter().filter_map(|event| match event {
                ProbeEvent::ActiveCooldown {
                    definition_runtime,
                    active,
                    cooldown,
                    elapsed_since_activation_hours,
                    record,
                    ..
                } if *definition_runtime == leaf.definition_runtime => Some((
                    *active,
                    *cooldown,
                    *elapsed_since_activation_hours,
                    *record,
                )),
                _ => None,
            }).collect::<Vec<_>>();
            if matching.len() != 1 {
                return Err("A36 retained ordinary row lacks one active-record witness".into());
            }
            let (active, cooldown, elapsed, record) = matching[0];
            let refreshed = exact_active_record_witness(
                OpenActiveProbeTrace {
                    country_runtime: record.country_runtime,
                    definition_runtime: leaf.definition_runtime,
                    effect_scope_runtime: record.effect_scope_runtime,
                    resolved_caster_runtime: Some(record.resolved_caster_runtime),
                    duplicate_or_nested: false,
                },
                active,
                cooldown,
                elapsed,
            )?;
            if refreshed.record_runtime != record.record_runtime
                || refreshed.record_caster_runtime != record.record_caster_runtime
                || refreshed.record_group_target_runtime != record.record_group_target_runtime
                || refreshed.record_pass_active != record.record_pass_active
            {
                return Err("A36 retained ordinary active-record state changed before exact-root join".into());
            }
        }
    }
    Ok(())
}

pub fn take_observations() -> Vec<ControllerObservation> {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map(|mut state| state.observations.drain(..).collect())
        .unwrap_or_default()
}

pub fn disabled_reason_observations() -> Vec<DisabledReasonObservation> {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map(|state| state.disabled_reasons.iter().cloned().collect())
        .unwrap_or_default()
}

pub fn take_disabled_reason_observations() -> Vec<DisabledReasonObservation> {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map(|mut state| state.disabled_reasons.drain(..).collect())
        .unwrap_or_default()
}

/// Portable identity from the authenticated character/OOB root snapshot.
/// Runtime addresses are lookup keys only and never enter ABL1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortableContextIdentity {
    pub country_tag: String,
    pub kind: crate::SelectedContextKind,
    pub stable_id: u64,
}

/// One immutable same-epoch join catalog built by the exact root collectors.
/// Ability tokens and icons are deliberately absent: A36 observes those from
/// the CAbility definition and exact UI resolver callsite itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExactA729PortableIdentityCatalog {
    pub epoch: u64,
    pub contexts_by_runtime: BTreeMap<usize, PortableContextIdentity>,
    pub division_ids_by_leader_runtime: BTreeMap<usize, Vec<u32>>,
}

/// Frozen pointer-free source built from engine observations. No caller may
/// supply an AbilityFrame or any row field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinedExactA729AbilitySource {
    frame: crate::AbilityFrame,
    controller_sequence: u64,
    identity_catalog_epoch: u64,
    tooltip_sequences: Vec<u64>,
}

impl JoinedExactA729AbilitySource {
    pub fn bind_from_exact_root_catalog(
        frame_hour: u64,
        elapsed_wall_ms: u64,
        catalog: &ExactA729PortableIdentityCatalog,
        controller: &ControllerObservation,
        observations: &[DisabledReasonObservation],
    ) -> Result<Self, String> {
        let (frame, tooltip_sequences) = materialize_abl1_frame(
            frame_hour,
            elapsed_wall_ms,
            catalog,
            controller,
            observations,
        )?;
        Ok(Self {
            frame,
            controller_sequence: controller.sequence,
            identity_catalog_epoch: catalog.epoch,
            tooltip_sequences,
        })
    }

    pub fn controller_sequence(&self) -> u64 {
        self.controller_sequence
    }

    pub fn tooltip_sequences(&self) -> &[u64] {
        &self.tooltip_sequences
    }
}

impl crate::sealed::Sealed for JoinedExactA729AbilitySource {}

impl crate::ExactA729AbilityControllerSource for JoinedExactA729AbilitySource {
    fn is_exact_a729_ability_controller(&self) -> bool {
        EXACT_A729_AUTHORITY_BOUND
            && self.controller_sequence != 0
            && self.identity_catalog_epoch != 0
            && self.tooltip_sequences.iter().all(|sequence| *sequence != 0)
    }

    fn collect_frame(&self, frame_hour: u64) -> Result<crate::AbilityFrame, String> {
        if frame_hour != self.frame.frame_hour {
            return Err("A36 joined source requested at a different frame hour".into());
        }
        Ok(self.frame.clone())
    }
}

fn materialize_abl1_frame(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    catalog: &ExactA729PortableIdentityCatalog,
    controller: &ControllerObservation,
    observations: &[DisabledReasonObservation],
) -> Result<(crate::AbilityFrame, Vec<u64>), String> {
    validate_identity_catalog(catalog)?;
    if compact_contract::calendar::hour_index_from_raw(controller.observed_game_hour_raw)
        .and_then(|hour| u64::try_from(hour).ok())
        != Some(frame_hour)
    {
        return Err("A36 exact-root join hour differs from retained refresh hour".into());
    }
    if controller.sequence == 0
        || catalog.epoch != controller.sequence
        || controller.source_definition_runtime.len() != controller.row_widget_count as usize
        || controller.definition_presentations.len() != controller.source_definition_runtime.len()
        || controller.definitions.len() != controller.source_definition_runtime.len() {
        return Err("A36 ABL1 row count does not match retained refresh identity".into());
    }
    let selected_runtime = selected_context_runtime(controller)?;
    let context = catalog.contexts_by_runtime.get(&selected_runtime)
        .ok_or_else(|| "A36 selected context is absent from the exact root catalog".to_string())?;
    validate_context_identity(context, controller)?;
    let selected_context_id = context_wire_id(context);
    let mut rows = Vec::with_capacity(controller.source_definition_runtime.len());
    let mut tooltip_sequences = Vec::with_capacity(rows.capacity());
    for ordinal in 0..controller.source_definition_runtime.len() {
        let row_identity_matches = observations
            .iter()
            .filter(|observation| {
                observation.controller_runtime == controller.controller_runtime
                    && observation.row_ordinal as usize == ordinal
                    && observation.definition_runtime
                        == controller.source_definition_runtime[ordinal]
                    && observation.source_definition_runtime
                        == controller.source_definition_runtime
                    && observation.row_owner_runtime == controller.row_owner_runtime
                    && observation.context_mode == controller.context_mode
                    && observation.selected_root_runtime == controller.selected_root_runtime
                    && observation.selected_leader_runtime == controller.selected_leader_runtime
                    && observation.selected_army_runtime == controller.selected_army_runtime
                    && observation.selected_army_group_runtime
                        == controller.selected_army_group_runtime
                    && observation.selected_other_runtime == controller.selected_other_runtime
                    && observation.field_marshal_aggregate == controller.field_marshal_aggregate
            })
            .collect::<Vec<_>>();
        if row_identity_matches
            .iter()
            .any(|observation| observation.controller_sequence != controller.sequence)
        {
            return Err("A36 ABL1 row has a cross-refresh tooltip transaction".into());
        }
        let matches = row_identity_matches
            .into_iter()
            .filter(|observation| observation.controller_sequence == controller.sequence)
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err("A36 ABL1 row has ambiguous tooltip transactions".into());
        }
        let observation = matches.first().copied();
        if observation.is_some_and(|observation| {
            observation.sequence <= controller.sequence
                || observation.observed_game_hour_raw != controller.observed_game_hour_raw
                || observation.final_tooltip.is_empty()
                || !observation.accumulator_callsite_seen
        }) {
            return Err("A36 observed tooltip/disabled-reason transaction is incomplete".into());
        }
        // 0x141D06052 is unconditional: it appends the evaluated error
        // accumulator even when that accumulator has length zero. Therefore
        // callsite presence authenticates the observation, while the exact
        // captured length (not callsite presence) is the availability leaf.
        let has_disabled_reason = observation
            .is_some_and(|observation| !observation.disabled_reason.is_empty());
        let definition = &controller.definitions[ordinal];
        let presentation = &controller.definition_presentations[ordinal];
        if definition.definition_runtime != controller.source_definition_runtime[ordinal]
            || presentation.definition_runtime != definition.definition_runtime {
            return Err("A36 definition leaves/presentation/source identity disagree".into());
        }
        if !controller.field_marshal_aggregate {
            let ordinary = controller
                .ordinary_leaves
                .iter()
                .find(|row| row.definition_runtime == definition.definition_runtime)
                .ok_or_else(|| {
                    "A36 ordinary definition lacks its exact state record".to_string()
                })?;
            if observation.is_some()
                && ((ordinary.state == OrdinaryAbilityState::Available && has_disabled_reason)
                    || (ordinary.state == OrdinaryAbilityState::Unavailable
                        && !has_disabled_reason)) {
                return Err(
                    "A36 evaluated ordinary availability disagrees with the exact accumulator"
                        .into(),
                );
            }
        }
        let (active, activation_elapsed, cooldown_remaining, duration_remaining, cost, active_leaders) =
            materialize_state(controller, definition, observation)?;
        let available = ability_available(controller, definition)?;
        let activation_hour = if active {
            Some(frame_hour.checked_sub(u64::try_from(activation_elapsed)
                .map_err(|_| "A36 active row has a negative activation elapsed value")?)
                .ok_or_else(|| "A36 activation elapsed predates frame zero".to_string())?)
        } else { None };
        let mut affected_division_ids = BTreeSet::new();
        for leader in active_leaders {
            for division_id in catalog.division_ids_by_leader_runtime.get(&leader)
                .ok_or_else(|| "A36 active leader is absent from exact division membership roots".to_string())? {
                if *division_id == 0 || !affected_division_ids.insert(*division_id) {
                    return Err("A36 exact division membership contains zero or duplicate identity".into());
                }
            }
        }
        rows.push(crate::AbilityRow {
            ability_id: presentation.ability_id.clone(),
            available,
            active,
            activation_hour,
            cooldown_total: cooldown_total(definition.cooldown_hours)?,
            cooldown_remaining,
            cost_command_power_fixed: cost,
            target_kind: Some(context_kind_name(context.kind).into()),
            target_id: Some(selected_context_id.clone()),
            duration_total: duration_total(definition.duration_hours)?,
            duration_remaining,
            display_order: ordinal as u32,
            icon: presentation.resolved_icon.clone(),
            tooltip: observation.map(|observation| observation.final_tooltip.clone()),
            disabled_reason: observation.and_then(|observation| {
                (!observation.disabled_reason.is_empty())
                    .then(|| observation.disabled_reason.clone())
            }),
            affected_division_ids: affected_division_ids.into_iter()
                .map(|id| format!("division:{id}"))
                .collect(),
        });
        if let Some(observation) = observation {
            tooltip_sequences.push(observation.sequence);
        }
    }
    if rows.is_empty() && !controller.measured_empty {
        return Err("A36 empty source lacks the exact measured-empty witness".into());
    }
    let frame = crate::AbilityFrame {
        elapsed_wall_ms,
        status_flags: 0,
        frame_hour,
        country_tag: context.country_tag.clone(),
        selected_context_kind: context.kind,
        selected_context_id: selected_context_id.clone(),
        source_record_id: format!(
            "a729:a36:{frame_hour}:{}:{}:{selected_context_id}",
            context.country_tag,
            context.kind.wire()
        ),
        complete: true,
        rows,
    };
    crate::validate_frame(&frame).map_err(|error| format!("A36 joined ABL1 frame invalid: {error:?}"))?;
    Ok((frame, tooltip_sequences))
}

fn selected_context_runtime(controller: &ControllerObservation) -> Result<usize, String> {
    [
        controller.selected_leader_runtime,
        controller.selected_army_group_runtime,
        controller.selected_army_runtime,
        controller.selected_other_runtime,
    ]
    .into_iter()
    .find(|value| *value != 0)
    .ok_or_else(|| "A36 selected context lacks an exact runtime identity".into())
}

fn selected_runtime_from_meta(meta: &ControllerMeta) -> Result<usize, String> {
    [
        meta.selected_leader,
        meta.selected_army_group,
        meta.selected_army,
        meta.selected_other,
    ]
    .into_iter()
    .find(|value| *value != 0)
    .ok_or_else(|| "A36 selected controller has no exact runtime identity".into())
}

fn witness_targets_runtime(witness: &ActiveRecordWitness, expected: usize) -> bool {
    expected != 0
        && [
            witness.resolved_caster_runtime,
            witness.effect_group_target_runtime,
            witness.record_caster_runtime.unwrap_or(0),
            witness.record_group_target_runtime.unwrap_or(0),
        ]
        .contains(&expected)
}

fn validate_identity_catalog(catalog: &ExactA729PortableIdentityCatalog) -> Result<(), String> {
    if catalog.epoch == 0 {
        return Err("A36 exact root catalog has no snapshot epoch".into());
    }
    let mut context_ids = BTreeSet::new();
    for (runtime, context) in &catalog.contexts_by_runtime {
        if *runtime == 0
            || context.stable_id == 0
            || context.country_tag.len() != 3
            || !context.country_tag.bytes().all(|byte| byte.is_ascii_uppercase())
            || !context_ids.insert((context.country_tag.clone(), context.kind.wire(), context.stable_id))
        {
            return Err("A36 exact root catalog carries an invalid or duplicate context identity".into());
        }
    }
    for (leader_runtime, divisions) in &catalog.division_ids_by_leader_runtime {
        let mut division_ids = BTreeSet::new();
        if *leader_runtime == 0
            || divisions.iter().any(|division| *division == 0 || !division_ids.insert(*division))
        {
            return Err(
                "A36 exact division catalog carries a null or duplicate identity for one leader"
                    .into(),
            );
        }
    }
    Ok(())
}

fn validate_context_identity(
    context: &PortableContextIdentity,
    controller: &ControllerObservation,
) -> Result<(), String> {
    if context.stable_id == 0
        || context.country_tag.len() != 3
        || !context.country_tag.bytes().all(|byte| byte.is_ascii_uppercase())
    {
        return Err("A36 exact context catalog carries an invalid portable identity".into());
    }
    let expected = if controller.selected_leader_runtime != 0 {
        if controller.field_marshal_aggregate {
            crate::SelectedContextKind::FieldMarshal
        } else {
            crate::SelectedContextKind::ArmyLeader
        }
    } else if controller.selected_army_group_runtime != 0 {
        crate::SelectedContextKind::ArmyGroup
    } else if controller.selected_army_runtime != 0 {
        crate::SelectedContextKind::Army
    } else {
        crate::SelectedContextKind::Other(controller.context_mode)
    };
    if context.kind != expected {
        return Err("A36 root catalog context kind contradicts controller selection".into());
    }
    Ok(())
}

fn context_kind_name(kind: crate::SelectedContextKind) -> &'static str {
    match kind {
        crate::SelectedContextKind::ArmyLeader => "army_leader",
        crate::SelectedContextKind::FieldMarshal => "field_marshal",
        crate::SelectedContextKind::ArmyGroup => "army_group",
        crate::SelectedContextKind::Army => "army",
        crate::SelectedContextKind::Other(_) => "other",
    }
}

fn context_wire_id(context: &PortableContextIdentity) -> String {
    let prefix = match context.kind {
        crate::SelectedContextKind::ArmyLeader | crate::SelectedContextKind::FieldMarshal => "leader",
        crate::SelectedContextKind::ArmyGroup => "army_group",
        crate::SelectedContextKind::Army => "army",
        crate::SelectedContextKind::Other(_) => "other",
    };
    format!("{prefix}:{}", context.stable_id)
}

fn duration_total(raw: i32) -> Result<crate::TimerValue, String> {
    match raw {
        -1 => Ok(crate::TimerValue::Indefinite),
        value if value >= 0 => Ok(crate::TimerValue::Hours(value as u32)),
        _ => Err("A36 duration sentinel is outside the exact -1/nonnegative law".into()),
    }
}

fn cooldown_total(raw: i32) -> Result<crate::TimerValue, String> {
    match raw {
        -1 => Ok(crate::TimerValue::NotStarted),
        value if value >= 0 => Ok(crate::TimerValue::Hours(value as u32)),
        _ => Err("A36 cooldown sentinel is outside the exact -1/nonnegative law".into()),
    }
}

fn remaining_timer(raw: i32) -> Result<crate::TimerValue, String> {
    if raw < 0 {
        Err("A36 running timer has a negative remaining value".into())
    } else {
        Ok(crate::TimerValue::Hours(raw as u32))
    }
}

fn ability_available(
    controller: &ControllerObservation,
    definition: &DefinitionLeaves,
) -> Result<bool, String> {
    if controller.field_marshal_aggregate {
        let state = controller
            .field_marshal_leaves
            .iter()
            .find(|row| row.definition_runtime == definition.definition_runtime)
            .ok_or_else(|| {
                "A36 field-marshal definition lacks its exact state record".to_string()
            })?;
        Ok(state.available_count != 0)
    } else {
        let state = controller
            .ordinary_leaves
            .iter()
            .find(|row| row.definition_runtime == definition.definition_runtime)
            .ok_or_else(|| "A36 ordinary definition lacks its exact state record".to_string())?;
        Ok(state.state == OrdinaryAbilityState::Available)
    }
}

fn materialize_state(
    controller: &ControllerObservation,
    definition: &DefinitionLeaves,
    tooltip: Option<&DisabledReasonObservation>,
) -> Result<(bool, i32, crate::TimerValue, crate::TimerValue, Option<i64>, Vec<usize>), String> {
    if controller.field_marshal_aggregate {
        let state = controller.field_marshal_leaves.iter()
            .find(|row| row.definition_runtime == definition.definition_runtime)
            .ok_or_else(|| "A36 field-marshal definition lacks its exact state record".to_string())?;
        // The refresh paints the aggregate row active either when the
        // selected field-marshal scope is itself active (+0x64), or when all
        // candidate subordinate leaders are active (+0x24 == +0x30, nonzero).
        let all_candidates_active = state.candidate_leader_count != 0
            && state.active_count == state.candidate_leader_count;
        let active = state.group_already_active || all_candidates_active;
        let cooldown_remaining = if !active
            && state.available_count == 0
            && state.cooldown_count != 0
        {
            remaining_timer(state.minimum_cooldown_remaining_hours)?
        } else { crate::TimerValue::NotStarted };
        let duration_remaining = if active {
            if definition.duration_hours == -1 { crate::TimerValue::Indefinite }
            else if state.group_already_active {
                let remaining = i64::from(definition.duration_hours)
                    .checked_sub(i64::from(state.group_activation_elapsed_hours))
                    .ok_or_else(|| "A36 aggregate duration remaining calculation overflowed".to_string())?;
                remaining_timer(i32::try_from(remaining.max(0))
                    .map_err(|_| "A36 aggregate duration remaining exceeds i32")?)?
            } else {
                remaining_timer(state.minimum_active_remaining_hours)?
            }
        } else { crate::TimerValue::NotStarted };
        let (activation_elapsed, active_leaders) = if state.group_already_active {
            if controller.selected_leader_runtime == 0 {
                return Err("A36 active field-marshal scope has no selected leader identity".into());
            }
            (
                state.group_activation_elapsed_hours,
                vec![controller.selected_leader_runtime],
            )
        } else if all_candidates_active {
            let leaders = state.leaders.iter()
                .filter(|leader| leader.state_code == 1)
                .collect::<Vec<_>>();
            let elapsed = leaders.iter()
                .map(|leader| leader.activation_elapsed_hours)
                .max()
                .ok_or_else(|| "A36 active field-marshal row has no active leader elapsed leaf".to_string())?;
            (
                elapsed,
                leaders.into_iter().map(|leader| leader.leader_runtime).collect(),
            )
        } else {
            (state.group_activation_elapsed_hours, Vec::new())
        };
        if active && active_leaders.is_empty() {
            return Err("A36 active field-marshal row has no active leader membership".into());
        }
        Ok((active, activation_elapsed, cooldown_remaining,
            duration_remaining, Some(state.allocated_command_power_cost_fixed), active_leaders))
    } else {
        let state = controller.ordinary_leaves.iter()
            .find(|row| row.definition_runtime == definition.definition_runtime)
            .ok_or_else(|| "A36 ordinary definition lacks its exact state record".to_string())?;
        let active = state.state == OrdinaryAbilityState::Active;
        let cooldown_remaining = if state.state == OrdinaryAbilityState::Cooldown {
            let remaining = i64::from(definition.duration_hours)
                .checked_add(i64::from(definition.cooldown_hours))
                .and_then(|value| value.checked_sub(i64::from(state.activation_elapsed_hours)))
                .ok_or_else(|| "A36 cooldown remaining calculation overflowed".to_string())?;
            remaining_timer(i32::try_from(remaining.max(0))
                .map_err(|_| "A36 cooldown remaining exceeds i32")?)?
        } else { crate::TimerValue::NotStarted };
        let duration_remaining = if active {
            if definition.duration_hours == -1 { crate::TimerValue::Indefinite }
            else {
                let remaining = i64::from(definition.duration_hours)
                    .checked_sub(i64::from(state.activation_elapsed_hours))
                    .ok_or_else(|| "A36 duration remaining calculation overflowed".to_string())?;
                remaining_timer(i32::try_from(remaining.max(0))
                    .map_err(|_| "A36 duration remaining exceeds i32")?)?
            }
        } else { crate::TimerValue::NotStarted };
        let cost = tooltip.and_then(|observation| {
            observation.effective_command_power_cost_fixed
        });
        let leaders = if active && controller.selected_leader_runtime != 0 {
            vec![controller.selected_leader_runtime]
        } else {
            Vec::new()
        };
        Ok((active, state.activation_elapsed_hours, cooldown_remaining,
            duration_remaining, cost, leaders))
    }
}

pub fn install_a729_ability_transition_hook() -> Result<InstalledHookAddresses, String> {
    let _install_guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "A36 hook install lock poisoned".to_string())?;
    if !EXACT_A729_AUTHORITY_BOUND || !TERMINAL_ACTIVE_RECORD_AUTHORITY_BOUND {
        return Err("A36 exact-a729 static authority receipt is not bound".into());
    }
    if INSTALL_BROKEN.load(Ordering::Acquire) {
        return Err("A36 prior hook rollback was incomplete; process restart required".into());
    }
    if INSTALLED.load(Ordering::Acquire) {
        return Err("A36 observer is already installed by another evidence owner".into());
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
        ("effect-scope caster resolver", addresses.effect_scope_caster_resolver, EFFECT_SCOPE_CASTER_RESOLVER_SENTINEL.as_slice()),
        ("availability evaluator", addresses.availability_evaluator, AVAILABILITY_SENTINEL.as_slice()),
        ("special-context probe", addresses.special_context_probe, SPECIAL_CONTEXT_SENTINEL.as_slice()),
        ("ability tooltip", addresses.tooltip_builder, TOOLTIP_SENTINEL.as_slice()),
        ("string append", addresses.string_append, STRING_APPEND_SENTINEL.as_slice()),
        ("effective command-power cost", addresses.effective_command_power_cost, EFFECTIVE_COST_SENTINEL.as_slice()),
        ("ability icon resolver", addresses.icon_resolver, ICON_RESOLVER_SENTINEL.as_slice()),
        ("ordinary effective-cost callsite", base + (EFFECTIVE_COST_RETURN_RVA - 5) as usize, EFFECTIVE_COST_CALLSITE_SENTINEL.as_slice()),
        ("ability icon resolver callsite", base + (ICON_RESOLVER_RETURN_RVA - 5) as usize, ICON_RESOLVER_CALLSITE_SENTINEL.as_slice()),
        (
            "disabled-reason accumulator append callsite",
            base + (DISABLED_REASON_APPEND_RETURN_RVA - 5) as usize,
            DISABLED_REASON_CALLSITE_SENTINEL.as_slice(),
        ),
    ] {
        verify_sentinel(label, address, sentinel)?;
    }
    {
        let mut state = STATE
            .get_or_init(|| Mutex::new(ObserverState::default()))
            .lock()
            .map_err(|_| "A36 observer state lock poisoned".to_string())?;
        *state = ObserverState::default();
    }
    NEXT_SEQUENCE.store(1, Ordering::Release);
    TRACE.with(|trace| *trace.borrow_mut() = None);
    ACTIVE_PROBE_TRACE.with(|trace| *trace.borrow_mut() = None);
    TOOLTIP_TRACE.with(|trace| *trace.borrow_mut() = None);
    MODULE_BASE.store(base, Ordering::Release);
    DRAINING.store(false, Ordering::Release);
    let specs = hook_specs(addresses);
    let mut created = 0usize;
    for &(target, detour, original_slot) in &specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(original) => {
                original_slot.store(original as usize, Ordering::Release);
                created += 1;
            }
            Err(status) => {
                let rollback = rollback(&specs, created, 0);
                return Err(format!(
                    "A36 hook creation failed at 0x{target:X}: {status}{rollback}"
                ));
            }
        }
    }
    for (enabled, (target, _, _)) in specs.iter().enumerate() {
        if let Err(status) = unsafe { MinHook::enable_hook(*target as *mut c_void) } {
            let rollback = rollback(&specs, created, enabled);
            return Err(format!(
                "A36 hook enable failed at 0x{target:X}: {status}{rollback}"
            ));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(addresses)
}

type HookSpec = (usize, *mut c_void, &'static AtomicUsize);

fn hook_specs(addresses: InstalledHookAddresses) -> [HookSpec; HOOK_COUNT] {
    [
        (addresses.context_invalid_probe, hook_context_probe as *mut c_void, &ORIGINAL_CONTEXT_PROBE),
        (addresses.active_cooldown_probe, hook_active_probe as *mut c_void, &ORIGINAL_ACTIVE_PROBE),
        (addresses.effect_scope_caster_resolver, hook_effect_scope_caster_resolver as *mut c_void, &ORIGINAL_EFFECT_SCOPE_CASTER_RESOLVER),
        (addresses.availability_evaluator, hook_availability as *mut c_void, &ORIGINAL_AVAILABILITY),
        (addresses.special_context_probe, hook_special_context as *mut c_void, &ORIGINAL_SPECIAL_CONTEXT),
        (addresses.string_append, hook_string_append as *mut c_void, &ORIGINAL_STRING_APPEND),
        (addresses.effective_command_power_cost, hook_effective_cost as *mut c_void, &ORIGINAL_EFFECTIVE_COST),
        (addresses.icon_resolver, hook_icon_resolver as *mut c_void, &ORIGINAL_ICON_RESOLVER),
        (addresses.tooltip_builder, hook_tooltip as *mut c_void, &ORIGINAL_TOOLTIP),
        (addresses.refresh, hook_refresh as *mut c_void, &ORIGINAL_REFRESH),
    ]
}

/// Stops capture, disables every detour, waits for in-flight callbacks to
/// leave the observer, and only then removes the trampolines. A failure leaves
/// the lane draining and marks the process unusable for a second install; the
/// owner must retain the DLL and require a process restart.
pub fn uninstall_a729_ability_transition_hook() -> Result<(), String> {
    let _install_guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "A36 hook install lock poisoned".to_string())?;
    if INSTALL_BROKEN.load(Ordering::Acquire) {
        return Err("A36 hook lifecycle is broken; process restart required".into());
    }
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        INSTALL_BROKEN.store(true, Ordering::Release);
        DRAINING.store(true, Ordering::Release);
        return Err("A36 installed hook has no module base; process restart required".into());
    }
    DRAINING.store(true, Ordering::Release);
    let specs = hook_specs(installed_addresses(base));
    let mut disable_failures = Vec::new();
    for (target, _, _) in specs.iter().rev() {
        if let Err(status) = unsafe { MinHook::disable_hook(*target as *mut c_void) } {
            disable_failures.push(format!("0x{target:X}: {status}"));
        }
    }
    if !disable_failures.is_empty() {
        INSTALL_BROKEN.store(true, Ordering::Release);
        return Err(format!(
            "A36 hook disable was incomplete at {}; process restart required",
            disable_failures.join(", ")
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            INSTALL_BROKEN.store(true, Ordering::Release);
            return Err(format!(
                "A36 hook callbacks did not quiesce ({} active); process restart required",
                ACTIVE_CALLBACKS.load(Ordering::Acquire)
            ));
        }
        thread::yield_now();
    }
    let mut remove_failures = Vec::new();
    for (target, _, original) in specs.iter().rev() {
        match unsafe { MinHook::remove_hook(*target as *mut c_void) } {
            Ok(()) => original.store(0, Ordering::Release),
            Err(status) => remove_failures.push(format!("0x{target:X}: {status}")),
        }
    }
    if !remove_failures.is_empty() {
        INSTALL_BROKEN.store(true, Ordering::Release);
        return Err(format!(
            "A36 hook removal was incomplete at {}; process restart required",
            remove_failures.join(", ")
        ));
    }
    TRACE.with(|trace| *trace.borrow_mut() = None);
    ACTIVE_PROBE_TRACE.with(|trace| *trace.borrow_mut() = None);
    TOOLTIP_TRACE.with(|trace| *trace.borrow_mut() = None);
    MODULE_BASE.store(0, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    DRAINING.store(false, Ordering::Release);
    Ok(())
}

fn rollback(
    specs: &[HookSpec],
    created: usize,
    enabled: usize,
) -> String {
    DRAINING.store(true, Ordering::Release);
    let mut failures = Vec::new();
    for (target, _, _) in specs.iter().take(enabled).rev() {
        if let Err(status) = unsafe { MinHook::disable_hook(*target as *mut c_void) } {
            failures.push(format!("0x{target:X} disable: {status}"));
        }
    }
    if failures.is_empty() {
        let deadline = Instant::now() + Duration::from_secs(5);
        while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
            thread::yield_now();
        }
        let remaining = ACTIVE_CALLBACKS.load(Ordering::Acquire);
        if remaining != 0 {
            failures.push(format!("{remaining} callback(s) did not quiesce"));
        }
    }
    if failures.is_empty() {
        for (target, _, original) in specs.iter().take(created).rev() {
            match unsafe { MinHook::remove_hook(*target as *mut c_void) } {
                Ok(()) => original.store(0, Ordering::Release),
                Err(status) => failures.push(format!("0x{target:X}: {status}")),
            }
        }
    }
    if failures.is_empty() {
        MODULE_BASE.store(0, Ordering::Release);
        DRAINING.store(false, Ordering::Release);
        String::new()
    } else {
        INSTALL_BROKEN.store(true, Ordering::Release);
        format!("; rollback retained live trampolines at {}", failures.join(", "))
    }
}

fn installed_addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        refresh: base + ABILITY_REFRESH_RVA as usize,
        context_invalid_probe: base + CONTEXT_INVALID_PROBE_RVA as usize,
        active_cooldown_probe: base + ACTIVE_COOLDOWN_PROBE_RVA as usize,
        effect_scope_caster_resolver: base + EFFECT_SCOPE_CASTER_RESOLVER_RVA as usize,
        availability_evaluator: base + AVAILABILITY_EVALUATOR_RVA as usize,
        special_context_probe: base + SPECIAL_CONTEXT_PROBE_RVA as usize,
        tooltip_builder: base + TOOLTIP_BUILDER_RVA as usize,
        string_append: base + STRING_APPEND_RVA as usize,
        effective_command_power_cost: base + EFFECTIVE_COMMAND_POWER_COST_RVA as usize,
        icon_resolver: base + ICON_RESOLVER_RVA as usize,
    }
}

unsafe extern "system" fn hook_refresh(controller: usize) {
    let callback = CallbackGuard::enter();
    if !callback.capture {
        let original = ORIGINAL_REFRESH.load(Ordering::Acquire);
        if original != 0 {
            let original: RefreshFn = unsafe { mem::transmute(original) };
            unsafe { original(controller) };
        }
        return;
    }
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
    let callback = CallbackGuard::enter();
    let original: ContextProbeFn = unsafe {
        mem::transmute(ORIGINAL_CONTEXT_PROBE.load(Ordering::Acquire))
    };
    let result = unsafe { original(context) };
    if callback.capture {
        record_event(ProbeEvent::ContextInvalid { invalid: result != 0 });
    }
    result
}

unsafe extern "system" fn hook_active_probe(
    runtime: usize,
    definition: usize,
    scope: usize,
    cooldown_out: *mut u8,
    elapsed_out: *mut i32,
) -> usize {
    let callback = CallbackGuard::enter();
    let nested = if callback.capture {
        ACTIVE_PROBE_TRACE.with(|trace| {
            let mut trace = trace.borrow_mut();
            if let Some(open) = trace.as_mut() {
                open.duplicate_or_nested = true;
                true
            } else {
                *trace = Some(OpenActiveProbeTrace {
                    country_runtime: runtime,
                    definition_runtime: definition,
                    effect_scope_runtime: scope,
                    resolved_caster_runtime: None,
                    duplicate_or_nested: false,
                });
                false
            }
        })
    } else {
        false
    };
    let original: ActiveProbeFn = unsafe {
        mem::transmute(ORIGINAL_ACTIVE_PROBE.load(Ordering::Acquire))
    };
    let result = unsafe { original(runtime, definition, scope, cooldown_out, elapsed_out) };
    if !callback.capture || nested {
        return result;
    }
    let cooldown = read_local_u8(cooldown_out).is_some_and(|value| value != 0);
    let elapsed = read_local_i32(elapsed_out).unwrap_or(-1);
    let active = result as u8 != 0;
    let trace = ACTIVE_PROBE_TRACE.with(|trace| trace.borrow_mut().take());
    if let Some(record) = trace
        .filter(|trace| {
            !trace.duplicate_or_nested
                && trace.country_runtime == runtime
                && trace.definition_runtime == definition
                && trace.effect_scope_runtime == scope
        })
        .and_then(|trace| exact_active_record_witness(trace, active, cooldown, elapsed).ok())
    {
        record_event(ProbeEvent::ActiveCooldown {
            definition_runtime: definition,
            scope_runtime: scope,
            active,
            cooldown,
            elapsed_since_activation_hours: elapsed,
            record,
        });
    }
    result
}

unsafe extern "system" fn hook_effect_scope_caster_resolver(scope: usize) -> usize {
    let callback = CallbackGuard::enter();
    let original: EffectScopeCasterResolverFn = unsafe {
        mem::transmute(ORIGINAL_EFFECT_SCOPE_CASTER_RESOLVER.load(Ordering::Acquire))
    };
    let result = unsafe { original(scope) };
    if callback.capture {
        ACTIVE_PROBE_TRACE.with(|trace| {
            let mut trace = trace.borrow_mut();
            let Some(open) = trace.as_mut() else { return };
            if open.effect_scope_runtime != scope {
                open.duplicate_or_nested = true;
            } else if let Some(prior) = open.resolved_caster_runtime {
                if prior != result {
                    open.duplicate_or_nested = true;
                }
            } else {
                open.resolved_caster_runtime = Some(result);
            }
        });
    }
    result
}

fn exact_active_record_witness(
    trace: OpenActiveProbeTrace,
    active: bool,
    cooldown: bool,
    elapsed: i32,
) -> Result<ActiveRecordWitness, String> {
    let resolved_caster_runtime = trace
        .resolved_caster_runtime
        .ok_or_else(|| "active probe did not traverse its exact effect-scope caster resolver".to_string())?;
    let effect_group_target_runtime = read_usize(
        trace
            .effect_scope_runtime
            .checked_add(0x48)
            .ok_or("active effect-scope group-target address overflow")?,
    )?;
    let target_type = read_i32(
        trace
            .definition_runtime
            .checked_add(0x90)
            .ok_or("ability target-type address overflow")?,
    )?;
    let header_address = trace
        .country_runtime
        .checked_add(0x15b8)
        .ok_or("country active-record header address overflow")?;
    let header = read_header(header_address)?;
    validate_header(header, ACTIVE_RECORD_STRIDE, "country active ability")?;
    let first = read_vector(header, ACTIVE_RECORD_STRIDE)?;
    let second_header = read_header(header_address)?;
    let second = read_vector(second_header, ACTIVE_RECORD_STRIDE)?;
    if header != second_header || first != second {
        return Err("country active-ability records changed during exact probe join".into());
    }
    let mut matches = first
        .chunks_exact(ACTIVE_RECORD_STRIDE)
        .enumerate()
        .filter_map(|(index, record)| {
            let record_definition = le_usize(record, 0).ok()?;
            let record_caster = le_usize(record, 0x10).ok()?;
            let record_group_target = le_usize(record, 0x20).ok()?;
            let target_matches = target_type == 2
                || (effect_group_target_runtime != 0
                    && record_group_target == effect_group_target_runtime)
                || (resolved_caster_runtime != 0 && record_caster == resolved_caster_runtime);
            (record_definition == trace.definition_runtime && target_matches).then_some((
                header.data + index * ACTIVE_RECORD_STRIDE,
                record_caster,
                record_group_target,
                record[0x48] == 0,
            ))
        });
    let matched = matches.next();
    if matches.next().is_some() {
        return Err("active probe matched multiple CCountry active-ability records".into());
    }
    if let Some((_, _, _, record_pass_active)) = matched {
        if active != record_pass_active || cooldown == record_pass_active || elapsed < 0 {
            return Err("active probe result disagrees with its exact CCountry record".into());
        }
    } else if active || cooldown || elapsed != -1 {
        return Err("active probe result claims state without an exact CCountry record".into());
    }
    Ok(ActiveRecordWitness {
        country_runtime: trace.country_runtime,
        effect_scope_runtime: trace.effect_scope_runtime,
        resolved_caster_runtime,
        effect_group_target_runtime,
        record_runtime: matched.map(|row| row.0),
        record_caster_runtime: matched.map(|row| row.1),
        record_group_target_runtime: matched.map(|row| row.2),
        record_pass_active: matched.map(|row| row.3),
    })
}

unsafe extern "system" fn hook_availability(
    runtime: usize,
    definition: usize,
    scope: usize,
    resolved_text: usize,
) -> usize {
    let callback = CallbackGuard::enter();
    let original: AvailabilityFn = unsafe {
        mem::transmute(ORIGINAL_AVAILABILITY.load(Ordering::Acquire))
    };
    let result = unsafe { original(runtime, definition, scope, resolved_text) };
    if callback.capture {
        record_event(ProbeEvent::Availability {
            country_runtime: runtime,
            definition_runtime: definition,
            scope_runtime: scope,
            available: result as u8 != 0,
        });
    }
    result
}

unsafe extern "system" fn hook_special_context(controller: usize) -> u8 {
    let callback = CallbackGuard::enter();
    let original: SpecialContextFn = unsafe {
        mem::transmute(ORIGINAL_SPECIAL_CONTEXT.load(Ordering::Acquire))
    };
    let result = unsafe { original(controller) };
    if callback.capture {
        record_event(ProbeEvent::SpecialContext {
            controller_runtime: controller,
            allowed: result != 0,
        });
    }
    result
}

unsafe extern "system" fn hook_icon_resolver(registry: usize, icon_string: usize) -> usize {
    let callback = CallbackGuard::enter();
    let original: IconResolverFn = unsafe {
        mem::transmute(ORIGINAL_ICON_RESOLVER.load(Ordering::Acquire))
    };
    let result = unsafe { original(registry, icon_string) };
    if callback.capture && immediate_game_return_rva() == Some(ICON_RESOLVER_RETURN_RVA) {
        let icon = read_engine_string(icon_string)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default();
        record_event(ProbeEvent::IconResolved {
            icon,
            resolved: result != 0,
        });
    }
    result
}

unsafe extern "system" fn hook_effective_cost(
    definition: usize,
    cost_out: *mut i64,
    scope: usize,
) -> usize {
    let callback = CallbackGuard::enter();
    let original: EffectiveCostFn = unsafe {
        mem::transmute(ORIGINAL_EFFECTIVE_COST.load(Ordering::Acquire))
    };
    let result = unsafe { original(definition, cost_out, scope) };
    if callback.capture && immediate_game_return_rva() == Some(EFFECTIVE_COST_RETURN_RVA) {
        let value = rpm(cost_out as usize, 8).and_then(|bytes| le_i64(&bytes, 0).ok());
        TOOLTIP_TRACE.with(|trace| {
            let mut trace = trace.borrow_mut();
            let Some(open) = trace.as_mut() else { return };
            let Some(value) = value else {
                open.duplicate_effective_cost = true;
                return;
            };
            if open.identity.field_marshal_aggregate
                || definition != open.identity.definition_runtime
                || open.effective_command_power_cost_fixed.replace(value).is_some()
            {
                open.duplicate_effective_cost = true;
            }
        });
    }
    result
}

unsafe extern "system" fn hook_tooltip(
    callback_owner: usize,
    hovered_widget: usize,
    tooltip_output: usize,
) -> usize {
    let callback = CallbackGuard::enter();
    if !callback.capture {
        let original = ORIGINAL_TOOLTIP.load(Ordering::Acquire);
        return if original == 0 {
            0
        } else {
            let original: TooltipFn = unsafe { mem::transmute(original) };
            unsafe { original(callback_owner, hovered_widget, tooltip_output) }
        };
    }
    let identity = read_tooltip_identity(callback_owner, hovered_widget, tooltip_output).ok();
    let identity_present = identity.is_some();
    let nested = TOOLTIP_TRACE.with(|trace| {
        let mut trace = trace.borrow_mut();
        if let Some(open) = trace.as_mut() {
            open.nested_or_replaced = true;
            true
        } else if let Some(identity) = identity.clone() {
            *trace = Some(OpenTooltipTrace {
                identity,
                disabled_reason_bytes: None,
                effective_command_power_cost_fixed: None,
                duplicate_effective_cost: false,
                accumulator_callsite_seen: false,
                duplicate_accumulator_append: false,
                nested_or_replaced: false,
            });
            false
        } else {
            false
        }
    });
    if nested {
        reject_nested_tooltip();
    } else if !identity_present {
        reject_tooltip();
    }
    let original = ORIGINAL_TOOLTIP.load(Ordering::Acquire);
    let result = if original == 0 {
        0
    } else {
        let original: TooltipFn = unsafe { mem::transmute(original) };
        unsafe { original(callback_owner, hovered_widget, tooltip_output) }
    };
    if nested || !identity_present {
        return result;
    }
    let trace = TOOLTIP_TRACE.with(|trace| trace.borrow_mut().take());
    match trace.and_then(|trace| finish_tooltip_trace(trace, result).ok()) {
        Some(observation) => retain_disabled_reason(observation),
        None => reject_tooltip(),
    }
    result
}

unsafe extern "system" fn hook_string_append(
    destination: usize,
    source: usize,
    length: usize,
) -> usize {
    let callback = CallbackGuard::enter();
    let tooltip_open =
        callback.capture && TOOLTIP_TRACE.with(|trace| trace.borrow().is_some());
    if tooltip_open && immediate_game_return_rva() == Some(DISABLED_REASON_APPEND_RETURN_RVA) {
        TOOLTIP_TRACE.with(|trace| {
            let mut trace = trace.borrow_mut();
            let Some(open) = trace.as_mut() else {
                return;
            };
            open.accumulator_callsite_seen = true;
            if destination != open.identity.tooltip_output_runtime || length > MAX_TEXT_BYTES {
                open.duplicate_accumulator_append = true;
                return;
            }
            // The exact call is unconditional. A zero-length source is the
            // engine's measured available result, and must be retained as an
            // authenticated empty accumulator rather than rejected as a
            // missing disabled-reason observation.
            let bytes = if length == 0 {
                Vec::new()
            } else if let Some(bytes) = rpm(source, length) {
                bytes
            } else {
                open.duplicate_accumulator_append = true;
                return;
            };
            if open.disabled_reason_bytes.replace(bytes).is_some() {
                open.duplicate_accumulator_append = true;
            }
        });
    }
    let original = ORIGINAL_STRING_APPEND.load(Ordering::Acquire);
    if original == 0 {
        destination
    } else {
        let original: StringAppendFn = unsafe { mem::transmute(original) };
        unsafe { original(destination, source, length) }
    }
}

fn read_tooltip_identity(
    callback_owner: usize,
    hovered_widget: usize,
    tooltip_output: usize,
) -> Result<TooltipRuntimeIdentity, String> {
    let controller = callback_owner
        .checked_sub(0x28)
        .ok_or_else(|| "ability tooltip callback owner underflow".to_string())?;
    if hovered_widget == 0 || tooltip_output == 0 {
        return Err("ability tooltip has a null widget/output".into());
    }
    let observed_game_hour_raw = current_game_hour_raw()?;
    let before = read_meta(controller)?;
    validate_header(before.source, 8, "tooltip source")?;
    validate_header(before.row_widgets, 8, "tooltip row widget")?;
    if before.source.count == 0 || before.source.count != before.row_widgets.count {
        return Err("ability tooltip source/widget counts are empty or unequal".into());
    }
    let source_first = read_vector(before.source, 8)?;
    let rows_first = read_vector(before.row_widgets, 8)?;
    let source = read_vector(before.source, 8)?;
    let rows = read_vector(before.row_widgets, 8)?;
    let after = read_meta(controller)?;
    if before != after || source_first != source || rows_first != rows {
        return Err("ability tooltip row identity changed during snapshot".into());
    }
    let source_definition_runtime = pointer_rows(&source)?;
    let row_owner_runtime = pointer_rows(&rows)?;
    if source_definition_runtime.iter().any(|value| *value == 0)
        || row_owner_runtime.iter().any(|value| *value == 0)
    {
        return Err("ability tooltip row identity contains null".into());
    }
    let matching = row_owner_runtime
        .iter()
        .enumerate()
        .filter_map(|(ordinal, owner)| {
            owner
                .checked_add(0x30)
                .filter(|embedded_widget| *embedded_widget == hovered_widget)
                .map(|_| ordinal)
        })
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err("ability tooltip widget lacks one exact embedded-row owner".into());
    }
    let ordinal = matching[0];
    let definition_runtime = source_definition_runtime[ordinal];
    let mut identity = TooltipRuntimeIdentity {
        observed_game_hour_raw,
        controller_sequence: 0,
        callback_owner_runtime: callback_owner,
        hovered_widget_runtime: hovered_widget,
        tooltip_output_runtime: tooltip_output,
        controller_runtime: controller,
        context_mode: after.context_mode,
        selected_root_runtime: after.selected_root,
        selected_leader_runtime: after.selected_leader,
        selected_army_runtime: after.selected_army,
        selected_army_group_runtime: after.selected_army_group,
        selected_other_runtime: after.selected_other,
        field_marshal_aggregate: after.field_marshal_aggregate,
        row_ordinal: ordinal as u32,
        definition_runtime,
        source_definition_runtime,
        row_owner_runtime,
    };
    identity.controller_sequence = matching_controller_sequence(&identity)
        .ok_or_else(|| "ability tooltip lacks an exact retained refresh transaction".to_string())?;
    Ok(identity)
}

fn matching_controller_sequence(identity: &TooltipRuntimeIdentity) -> Option<u64> {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .and_then(|state| {
            state
                .observations
                .iter()
                .rev()
                .find(|observation| {
                    observation.controller_runtime == identity.controller_runtime
                        && observation.context_mode == identity.context_mode
                        && observation.selected_root_runtime == identity.selected_root_runtime
                        && observation.selected_leader_runtime == identity.selected_leader_runtime
                        && observation.selected_army_runtime == identity.selected_army_runtime
                        && observation.selected_army_group_runtime
                            == identity.selected_army_group_runtime
                        && observation.selected_other_runtime == identity.selected_other_runtime
                        && observation.field_marshal_aggregate == identity.field_marshal_aggregate
                        && observation.source_definition_runtime
                            == identity.source_definition_runtime
                        && observation.row_owner_runtime == identity.row_owner_runtime
                        && observation.observed_game_hour_raw == identity.observed_game_hour_raw
                })
                .map(|observation| observation.sequence)
        })
}

fn finish_tooltip_trace(
    trace: OpenTooltipTrace,
    tooltip_result: usize,
) -> Result<DisabledReasonObservation, String> {
    if tooltip_result == 0
        || trace.nested_or_replaced
        || trace.duplicate_accumulator_append
        || trace.duplicate_effective_cost
        || (!trace.identity.field_marshal_aggregate
            && trace.effective_command_power_cost_fixed.is_none())
    {
        return Err("ability tooltip transaction was unsuccessful or ambiguous".into());
    }
    let after = read_tooltip_identity(
        trace.identity.callback_owner_runtime,
        trace.identity.hovered_widget_runtime,
        trace.identity.tooltip_output_runtime,
    )?;
    if after != trace.identity {
        return Err("ability tooltip row identity changed across invocation".into());
    }
    if trace.accumulator_callsite_seen != trace.disabled_reason_bytes.is_some() {
        return Err("ability tooltip accumulator callsite/body disagreement".into());
    }
    let final_tooltip = String::from_utf8(
        read_engine_string(trace.identity.tooltip_output_runtime)
            .ok_or_else(|| "ability final tooltip string unreadable".to_string())?,
    )
    .map_err(|_| "ability final tooltip is not UTF-8".to_string())?;
    if final_tooltip.is_empty() {
        return Err("ability final tooltip is empty".into());
    }
    let disabled_reason = String::from_utf8(trace.disabled_reason_bytes.unwrap_or_default())
        .map_err(|_| "ability disabled-reason accumulator is not UTF-8".to_string())?;
    let identity = trace.identity;
    Ok(DisabledReasonObservation {
        sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        observed_game_hour_raw: identity.observed_game_hour_raw,
        controller_sequence: identity.controller_sequence,
        controller_runtime: identity.controller_runtime,
        callback_owner_runtime: identity.callback_owner_runtime,
        hovered_widget_runtime: identity.hovered_widget_runtime,
        context_mode: identity.context_mode,
        selected_root_runtime: identity.selected_root_runtime,
        selected_leader_runtime: identity.selected_leader_runtime,
        selected_army_runtime: identity.selected_army_runtime,
        selected_army_group_runtime: identity.selected_army_group_runtime,
        selected_other_runtime: identity.selected_other_runtime,
        field_marshal_aggregate: identity.field_marshal_aggregate,
        row_ordinal: identity.row_ordinal,
        definition_runtime: identity.definition_runtime,
        source_definition_runtime: identity.source_definition_runtime,
        row_owner_runtime: identity.row_owner_runtime,
        final_tooltip,
        disabled_reason,
        accumulator_callsite_seen: trace.accumulator_callsite_seen,
        effective_command_power_cost_fixed: trace.effective_command_power_cost_fixed,
    })
}

fn immediate_game_return_rva() -> Option<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let pe_offset = read_u32(base.checked_add(0x3c)?).ok()? as usize;
    let image_size = read_u32(base.checked_add(pe_offset)?.checked_add(0x50)?).ok()? as usize;
    let mut stack = [ptr::null_mut::<c_void>(); 8];
    let count = unsafe {
        RtlCaptureStackBackTrace(0, stack.len() as u32, stack.as_mut_ptr(), ptr::null_mut())
    } as usize;
    nearest_game_return_rva(
        base,
        image_size,
        &stack[..count]
            .iter()
            .map(|address| *address as usize)
            .collect::<Vec<_>>(),
    )
}

fn nearest_game_return_rva(base: usize, image_size: usize, stack: &[usize]) -> Option<u32> {
    let end = base.checked_add(image_size)?;
    let address = stack
        .iter()
        .copied()
        .find(|address| *address >= base && *address < end)?;
    u32::try_from(address.checked_sub(base)?).ok()
}

fn read_engine_string(address: usize) -> Option<Vec<u8>> {
    let length = read_usize(address.checked_add(0x10)?).ok()?;
    let capacity = read_usize(address.checked_add(0x18)?).ok()?;
    if length > capacity || length > MAX_TEXT_BYTES {
        return None;
    }
    if length == 0 {
        return Some(Vec::new());
    }
    let data = if capacity <= 15 {
        address
    } else {
        read_usize(address).ok()?
    };
    rpm(data, length)
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
    let game_hour_before = current_game_hour_raw()?;
    let (meta, source_bytes, state_bytes, leader_bytes, row_widget_bytes) =
        stable_snapshot(trace.controller_runtime)?;
    let observed_game_hour_raw = current_game_hour_raw()?;
    if game_hour_before != observed_game_hour_raw {
        return Err("ability refresh crossed a game-hour boundary during observation".into());
    }
    validate_header(meta.source, 8, "source")?;
    validate_header(meta.state, FM_STATE_STRIDE, "field-marshal state")?;
    validate_header(meta.leaders, 8, "field-marshal leader")?;
    validate_header(meta.row_widgets, 8, "row widget")?;
    if meta.row_widgets.count != meta.source.count {
        return Err("row widget count differs from exact source order".into());
    }
    let sources = pointer_rows(&source_bytes)?;
    let row_owners = pointer_rows(&row_widget_bytes)?;
    let aggregate_leaders = pointer_rows(&leader_bytes)?;
    if aggregate_leaders.iter().any(|pointer| *pointer == 0) {
        return Err("field-marshal leader vector contains a null identity".into());
    }
    if row_owners.iter().any(|pointer| *pointer == 0) {
        return Err("row widget vector contains a null owner".into());
    }
    let mut definitions = Vec::with_capacity(sources.len());
    for definition in &sources {
        definitions.push(read_definition(*definition)?);
    }
    let icon_events = trace
        .events
        .iter()
        .filter_map(|event| match event {
            ProbeEvent::IconResolved { icon, resolved } => Some((icon, *resolved)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if icon_events.len() != sources.len() {
        return Err("ability icon resolver observations differ from source row count".into());
    }
    let mut ability_ids = BTreeSet::new();
    let mut definition_presentations = Vec::with_capacity(sources.len());
    for (definition_runtime, (icon, resolved)) in sources.iter().zip(icon_events) {
        let ability_id = read_engine_string(*definition_runtime + 0x70)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|value| valid_portable_token(value))
            .ok_or_else(|| "ability definition intrinsic token is unreadable or non-portable".to_string())?;
        if !ability_ids.insert(ability_id.clone())
            || !resolved
            || !valid_portable_token(icon)
        {
            return Err("ability token/icon identity is duplicate, unresolved or non-portable".into());
        }
        definition_presentations.push(DefinitionPresentation {
            definition_runtime: *definition_runtime,
            ability_id,
            resolved_icon: icon.clone(),
        });
    }
    let mut field_marshal_leaves = parse_field_marshal_states(&state_bytes)?;
    let state_definitions = field_marshal_leaves
        .iter()
        .map(|row| row.definition_runtime)
        .collect::<Vec<_>>();
    if meta.field_marshal_aggregate {
        if state_definitions != sources {
            return Err("field-marshal state records differ from source order".into());
        }
        reconcile_field_marshal_active_probes(
            &mut field_marshal_leaves,
            &trace.events,
            meta.selected_leader,
        )?;
    } else if !state_definitions.is_empty() {
        return Err("ordinary refresh retained field-marshal state records".into());
    }
    let ordinary_leaves = if !meta.field_marshal_aggregate {
        reconcile_ordinary(
            trace.controller_runtime,
            selected_runtime_from_meta(&meta)?,
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
        observed_game_hour_raw,
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
        row_owner_runtime: row_owners,
        definitions,
        definition_presentations,
        ordinary_leaves,
        field_marshal_leaves,
        row_widget_count: meta.row_widgets.count,
        measured_empty,
        probe_events: trace.events,
    })
}

fn reconcile_field_marshal_active_probes(
    states: &mut [FieldMarshalAbilityLeaf],
    events: &[ProbeEvent],
    selected_leader_runtime: usize,
) -> Result<(), String> {
    let probes = events
        .iter()
        .filter_map(|event| match event {
            ProbeEvent::ActiveCooldown {
                definition_runtime,
                active,
                cooldown,
                elapsed_since_activation_hours,
                record,
                ..
            } => Some((
                *definition_runtime,
                *active,
                *cooldown,
                *elapsed_since_activation_hours,
                *record,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut cursor = 0usize;
    for state in states {
        let (definition, group_active, _group_cooldown, group_elapsed, group_record) = *probes
            .get(cursor)
            .ok_or_else(|| "field-marshal state lacks its selected-scope active probe".to_string())?;
        cursor += 1;
        if definition != state.definition_runtime
            || group_active != state.group_already_active
            || (group_active && group_elapsed != state.group_activation_elapsed_hours)
            || !witness_targets_runtime(&group_record, selected_leader_runtime)
        {
            return Err("field-marshal selected-scope probe disagrees with aggregate record".into());
        }
        if state.group_already_active {
            if !state.leaders.is_empty() {
                return Err("active field-marshal scope unexpectedly retained subordinate rows".into());
            }
            continue;
        }
        for leader in &mut state.leaders {
            let (definition, active, cooldown, elapsed, record) = *probes
                .get(cursor)
                .ok_or_else(|| "field-marshal leader row lacks its ordered active probe".to_string())?;
            cursor += 1;
            let expected_active = leader.state_code == 1;
            let expected_cooldown = leader.state_code == 2;
            if definition != state.definition_runtime
                || active != expected_active
                || cooldown != expected_cooldown
                || !witness_targets_runtime(&record, leader.leader_runtime)
            {
                return Err("field-marshal leader state disagrees with ordered active probe".into());
            }
            leader.activation_elapsed_hours = elapsed;
        }
    }
    if cursor != probes.len() {
        return Err("field-marshal refresh emitted extra active probes".into());
    }
    Ok(())
}

fn reconcile_ordinary(
    controller: usize,
    selected_runtime: usize,
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
            record,
        } => Some((
            *definition_runtime,
            *scope_runtime,
            *active,
            *cooldown,
            *elapsed_since_activation_hours,
            *record,
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
        let (observed, scope, active, cooldown, elapsed, record) = active_events
            .next()
            .ok_or_else(|| "ordinary row missing active/cooldown probe".to_string())?;
        if observed != *definition || !witness_targets_runtime(&record, selected_runtime) {
            return Err("ordinary active/cooldown probes differ from source order".into());
        }
        let matching_availability = events
            .iter()
            .filter_map(|event| match event {
                ProbeEvent::Availability {
                    country_runtime,
                    definition_runtime,
                    scope_runtime,
                    available,
                } if definition_runtime == definition => Some((*country_runtime, *scope_runtime, *available)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if matching_availability.len() > 1 {
            return Err("ordinary row emitted duplicate availability probes".into());
        }
        if matching_availability
            .first()
            .is_some_and(|(country, availability_scope, _)| {
                *country != record.country_runtime || *availability_scope != scope
            })
        {
            return Err("ordinary availability scope differs from active/cooldown scope".into());
        }
        let availability = matching_availability.first().map(|(_, _, available)| *available);
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

fn current_game_hour_raw() -> Result<i32, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("A36 module base is unavailable for game-hour authentication".into());
    }
    let game_state = read_usize(
        base.checked_add(CGAMESTATE_SINGLETON_RVA)
            .ok_or("A36 game-state singleton address overflow")?,
    )?;
    if game_state == 0 {
        return Err("A36 game-state singleton is null".into());
    }
    let hour = read_i32(
        game_state
            .checked_add(CGAMESTATE_CURRENT_DATE_OFF)
            .ok_or("A36 game-hour address overflow")?,
    )?;
    if hour <= 0 {
        return Err("A36 observed an implausible game hour".into());
    }
    Ok(hour)
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

fn valid_portable_token(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    !value.is_empty()
        && value.len() <= 1024
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':')
        })
        && !lower.starts_with("0x")
        && !lower.contains(":0x")
}

fn parse_field_marshal_states(bytes: &[u8]) -> Result<Vec<FieldMarshalAbilityLeaf>, String> {
    let mut out = Vec::new();
    let mut total_leader_count = 0usize;
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
        total_leader_count = total_leader_count
            .checked_add(leader_count)
            .ok_or_else(|| "field-marshal aggregate leader count overflowed".to_string())?;
        if total_leader_count > MAX_TOTAL_FIELD_MARSHAL_LEADERS {
            return Err("field-marshal aggregate leader count exceeds bound".into());
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
                activation_elapsed_hours: -1,
                command_power_cost_fixed: le_i64(row, 0x10)?,
                activation_command_power_cost_fixed: le_i64(row, 0x18)?,
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
            allocated_activation_command_power_cost_fixed: le_i64(record, 0x50)?,
            allocated_daily_command_power_cost_fixed: le_i64(record, 0x58)?,
            allocated_affected_division_count: le_u32(record, 0x60)?,
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
    let replace_last = state
        .observations
        .back()
        .is_some_and(|prior| {
            same_retention_state(prior, &observation)
                && !state
                    .disabled_reasons
                    .iter()
                    .any(|tooltip| tooltip.controller_sequence == prior.sequence)
        });
    if replace_last {
        state.observations.pop_back();
        state.observations.push_back(observation);
        return;
    }
    if state.observations.len() >= MAX_OBSERVATIONS {
        let removable = state.observations.iter().position(|candidate| {
            !state
                .disabled_reasons
                .iter()
                .any(|tooltip| tooltip.controller_sequence == candidate.sequence)
        });
        if let Some(index) = removable {
            state.observations.remove(index);
        } else {
            state.rejected_refreshes = state.rejected_refreshes.saturating_add(1);
            return;
        }
    }
    state.observations.push_back(observation);
}

fn same_retention_state(
    left: &ControllerObservation,
    right: &ControllerObservation,
) -> bool {
    left.controller_runtime == right.controller_runtime
        && left.context_mode == right.context_mode
        && left.selected_root_runtime == right.selected_root_runtime
        && left.selected_leader_runtime == right.selected_leader_runtime
        && left.selected_army_runtime == right.selected_army_runtime
        && left.selected_army_group_runtime == right.selected_army_group_runtime
        && left.selected_other_runtime == right.selected_other_runtime
        && left.field_marshal_aggregate == right.field_marshal_aggregate
        && left.aggregate_leader_runtime == right.aggregate_leader_runtime
        && left.source_definition_runtime == right.source_definition_runtime
        && left.row_owner_runtime == right.row_owner_runtime
        && left.measured_empty == right.measured_empty
        && left
            .ordinary_leaves
            .iter()
            .map(|row| {
                (
                    row.definition_runtime,
                    row.scope_runtime,
                    row.state,
                    row.availability_evaluated,
                    row.special_context_allowed,
                )
            })
            .eq(right.ordinary_leaves.iter().map(|row| {
                (
                    row.definition_runtime,
                    row.scope_runtime,
                    row.state,
                    row.availability_evaluated,
                    row.special_context_allowed,
                )
            }))
        && left
            .field_marshal_leaves
            .iter()
            .map(field_marshal_retention_key)
            .eq(right
                .field_marshal_leaves
                .iter()
                .map(field_marshal_retention_key))
}

fn field_marshal_retention_key(
    row: &FieldMarshalAbilityLeaf,
) -> (
    usize,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    bool,
    Vec<(usize, u8, bool)>,
) {
    (
        row.definition_runtime,
        row.available_count,
        row.active_count,
        row.cooldown_count,
        row.allocated_count,
        row.candidate_leader_count,
        row.definition_allowed_leader_count,
        row.group_already_active,
        row.leaders
            .iter()
            .map(|leader| {
                (
                    leader.leader_runtime,
                    leader.state_code,
                    leader.will_activate,
                )
            })
            .collect(),
    )
}

fn retain_disabled_reason(observation: DisabledReasonObservation) {
    let Ok(mut state) = STATE.get_or_init(|| Mutex::new(ObserverState::default())).lock() else {
        return;
    };
    state.completed_tooltips = state.completed_tooltips.saturating_add(1);
    if state.disabled_reasons.len() >= MAX_TEXT_OBSERVATIONS {
        // Never silently orphan an earlier controller/tooltip transaction.
        // The fixture regrader rejects any nonzero rejection counter, so a
        // saturated evidence buffer is explicit and cannot be promoted as a
        // complete four-card capture.
        state.rejected_tooltips = state.rejected_tooltips.saturating_add(1);
        return;
    }
    state.disabled_reasons.push_back(observation);
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

fn reject_tooltip() {
    if let Ok(mut state) = STATE.get_or_init(|| Mutex::new(ObserverState::default())).lock() {
        state.rejected_tooltips = state.rejected_tooltips.saturating_add(1);
    }
}

fn reject_nested_tooltip() {
    if let Ok(mut state) = STATE.get_or_init(|| Mutex::new(ObserverState::default())).lock() {
        state.nested_tooltips = state.nested_tooltips.saturating_add(1);
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

fn read_i32(address: usize) -> Result<i32, String> {
    let bytes = rpm(address, 4).ok_or_else(|| "i32 unreadable".to_string())?;
    le_i32(&bytes, 0)
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

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FRAME_HOUR: u64 = 1936 * 365 * 24 + 900;

    fn controller() -> ControllerObservation {
        ControllerObservation {
            sequence: 41,
            observed_game_hour_raw: compact_contract::calendar::DATE_EPOCH_CONST
                + TEST_FRAME_HOUR as i32,
            controller_runtime: 0x7ff6_1000_0000,
            context_mode: 1,
            selected_root_runtime: 0x7ff6_2000_0000,
            selected_leader_runtime: 0x7ff6_2000_0100,
            selected_army_runtime: 0,
            selected_army_group_runtime: 0,
            selected_other_runtime: 0,
            field_marshal_aggregate: false,
            aggregate_leader_runtime: Vec::new(),
            source_definition_runtime: vec![0x7ff6_3000_0000],
            row_owner_runtime: vec![0x7ff6_4000_0000],
            definitions: vec![DefinitionLeaves {
                definition_runtime: 0x7ff6_3000_0000,
                cancelable: false,
                special_context_required: false,
                duration_hours: 168,
                cooldown_hours: 504,
            }],
            definition_presentations: vec![DefinitionPresentation {
                definition_runtime: 0x7ff6_3000_0000,
                ability_id: "force_attack".into(),
                resolved_icon: "GFX_ability_force_attack".into(),
            }],
            ordinary_leaves: vec![OrdinaryAbilityLeaf {
                definition_runtime: 0x7ff6_3000_0000,
                scope_runtime: 0x7ff6_2000_0100,
                state: OrdinaryAbilityState::Unavailable,
                activation_elapsed_hours: -1,
                availability_evaluated: Some(false),
                special_context_allowed: None,
            }],
            field_marshal_leaves: Vec::new(),
            row_widget_count: 1,
            measured_empty: false,
            probe_events: Vec::new(),
        }
    }

    fn catalog() -> ExactA729PortableIdentityCatalog {
        ExactA729PortableIdentityCatalog {
            epoch: 41,
            contexts_by_runtime: BTreeMap::from([(
                0x7ff6_2000_0100,
                PortableContextIdentity {
                    country_tag: "GER".into(),
                    kind: crate::SelectedContextKind::ArmyLeader,
                    stable_id: 42,
                },
            )]),
            division_ids_by_leader_runtime: BTreeMap::from([(
                0x7ff6_2000_0100,
                vec![7],
            )]),
        }
    }

    fn observation() -> DisabledReasonObservation {
        DisabledReasonObservation {
            sequence: 42,
            observed_game_hour_raw: compact_contract::calendar::DATE_EPOCH_CONST
                + TEST_FRAME_HOUR as i32,
            controller_sequence: 41,
            controller_runtime: 0x7ff6_1000_0000,
            callback_owner_runtime: 0x7ff6_1000_0028,
            hovered_widget_runtime: 0x7ff6_4000_0030,
            context_mode: 1,
            selected_root_runtime: 0x7ff6_2000_0000,
            selected_leader_runtime: 0x7ff6_2000_0100,
            selected_army_runtime: 0,
            selected_army_group_runtime: 0,
            selected_other_runtime: 0,
            field_marshal_aggregate: false,
            row_ordinal: 0,
            definition_runtime: 0x7ff6_3000_0000,
            source_definition_runtime: vec![0x7ff6_3000_0000],
            row_owner_runtime: vec![0x7ff6_4000_0000],
            final_tooltip: "Force Attack\nCannot use while retreating".into(),
            disabled_reason: "Cannot use while retreating".into(),
            accumulator_callsite_seen: true,
            effective_command_power_cost_fixed: Some(125_000),
        }
    }

    #[test]
    fn exact_disabled_reason_transaction_materializes_pointer_free_abl1_and_aba2() {
        let source = JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            TEST_FRAME_HOUR,
            12,
            &catalog(),
            &controller(),
            &[observation()],
        )
        .expect("bind exact tooltip transaction");
        assert!(crate::ExactA729AbilityControllerSource::is_exact_a729_ability_controller(&source));
        assert_eq!(source.controller_sequence(), 41);
        assert_eq!(source.tooltip_sequences(), &[42]);
        assert_eq!(source.frame.rows[0].disabled_reason.as_deref(), Some("Cannot use while retreating"));
        let production = crate::production_carrier::ProductionAbilityFrame {
            elapsed_wall_ms: source.frame.elapsed_wall_ms,
            frame_hour: source.frame.frame_hour,
            status_flags: 0,
            authority: crate::production_carrier::AuthorityState::Exact20Receipt,
            snapshot: Some(source.frame.clone()),
        };
        crate::production_carrier::validate_frame(&production)
            .expect("joined frame is admissible to the dormant ABA2 carrier");
    }

    #[test]
    fn core_row_is_authoritative_without_an_optional_text_transaction() {
        let source = JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            TEST_FRAME_HOUR,
            7,
            &catalog(),
            &controller(),
            &[],
        )
        .expect("bind exact core row without tooltip");
        assert!(crate::ExactA729AbilityControllerSource::is_exact_a729_ability_controller(&source));
        assert!(source.tooltip_sequences().is_empty());
        assert_eq!(source.frame.rows[0].tooltip, None);
        assert_eq!(source.frame.rows[0].disabled_reason, None);
        assert_eq!(source.frame.rows[0].cost_command_power_fixed, None);
        assert!(crate::production_carrier::collect_and_encode(
            &source,
            TEST_FRAME_HOUR,
            7
        )
        .is_ok());
    }

    #[test]
    fn unconditional_empty_accumulator_materializes_available_row() {
        let mut available = controller();
        available.ordinary_leaves[0].state = OrdinaryAbilityState::Available;
        available.ordinary_leaves[0].availability_evaluated = Some(true);
        let mut tooltip = observation();
        tooltip.final_tooltip = "Force Attack".into();
        tooltip.disabled_reason.clear();
        tooltip.accumulator_callsite_seen = true;

        let source = JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            TEST_FRAME_HOUR,
            12,
            &catalog(),
            &available,
            &[tooltip],
        )
        .expect("bind exact empty error accumulator");
        assert!(source.frame.rows[0].available);
        assert_eq!(source.frame.rows[0].disabled_reason, None);
    }

    #[test]
    fn signed_effective_cost_is_preserved_without_an_invented_clamp() {
        let mut tooltip = observation();
        tooltip.effective_command_power_cost_fixed = Some(-125_000);
        let source = JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            TEST_FRAME_HOUR,
            12,
            &catalog(),
            &controller(),
            &[tooltip],
        )
        .expect("bind signed exact effective cost");
        assert_eq!(source.frame.rows[0].cost_command_power_fixed, Some(-125_000));
    }

    #[test]
    fn ordinary_accumulator_must_agree_with_evaluated_availability_state() {
        let mut available = controller();
        available.ordinary_leaves[0].state = OrdinaryAbilityState::Available;
        available.ordinary_leaves[0].availability_evaluated = Some(true);
        let mut tooltip = observation();
        tooltip.disabled_reason = "Cannot activate".into();
        assert!(JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            TEST_FRAME_HOUR,
            12,
            &catalog(),
            &available,
            &[tooltip],
        )
        .is_err());

        let mut unavailable = controller();
        unavailable.ordinary_leaves[0].state = OrdinaryAbilityState::Unavailable;
        unavailable.ordinary_leaves[0].availability_evaluated = Some(false);
        let mut empty = observation();
        empty.disabled_reason.clear();
        assert!(JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            TEST_FRAME_HOUR,
            12,
            &catalog(),
            &unavailable,
            &[empty],
        )
        .is_err());
    }

    #[test]
    fn division_identity_uniqueness_is_per_leader_membership() {
        let mut exact = catalog();
        exact
            .division_ids_by_leader_runtime
            .insert(0x7ff6_2000_0200, vec![7, 8]);
        validate_identity_catalog(&exact)
            .expect("a descendant division may appear in field-marshal and leader memberships");

        exact
            .division_ids_by_leader_runtime
            .insert(0x7ff6_2000_0200, vec![7, 7]);
        assert!(validate_identity_catalog(&exact).is_err());
    }

    #[test]
    fn retention_coalesces_timer_ticks_but_not_lifecycle_transitions() {
        let before = controller();
        let mut later_same_state = before.clone();
        later_same_state.sequence += 1;
        later_same_state.ordinary_leaves[0].activation_elapsed_hours = 12;
        assert!(same_retention_state(&before, &later_same_state));

        later_same_state.ordinary_leaves[0].state = OrdinaryAbilityState::Active;
        assert!(!same_retention_state(&before, &later_same_state));
    }

    #[test]
    fn field_marshal_all_candidates_active_uses_ordered_elapsed_and_minimum_remaining() {
        let mut controller = controller();
        controller.field_marshal_aggregate = true;
        controller.ordinary_leaves.clear();
        controller.field_marshal_leaves = vec![FieldMarshalAbilityLeaf {
            definition_runtime: controller.definitions[0].definition_runtime,
            leaders: vec![FieldMarshalLeaderLeaf {
                leader_runtime: 0x7ff6_2000_0100,
                state_code: 1,
                remaining_hours: 158,
                activation_elapsed_hours: 10,
                command_power_cost_fixed: 125_000,
                activation_command_power_cost_fixed: 125_000,
                will_activate: false,
            }],
            available_count: 0,
            active_count: 1,
            cooldown_count: 0,
            allocated_count: 0,
            candidate_leader_count: 1,
            definition_allowed_leader_count: 1,
            minimum_active_remaining_hours: 158,
            minimum_cooldown_remaining_hours: i32::MAX,
            group_activation_elapsed_hours: 0,
            allocated_command_power_cost_fixed: 0,
            allocated_activation_command_power_cost_fixed: 0,
            allocated_daily_command_power_cost_fixed: 0,
            allocated_affected_division_count: 0,
            group_already_active: false,
        }];
        let (active, elapsed, cooldown, duration, _, leaders) = materialize_state(
            &controller,
            &controller.definitions[0],
            Some(&observation()),
        )
        .expect("materialize exact all-leaders-active aggregate");
        assert!(active);
        assert_eq!(elapsed, 10);
        assert_eq!(cooldown, crate::TimerValue::NotStarted);
        assert_eq!(duration, crate::TimerValue::Hours(158));
        assert_eq!(leaders, vec![0x7ff6_2000_0100]);
    }

    #[test]
    fn duplicate_or_cross_refresh_tooltip_transactions_fail_closed() {
        let row = observation();
        assert!(JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            TEST_FRAME_HOUR,
            12,
            &catalog(),
            &controller(),
            &[row.clone(), row.clone()],
        )
        .is_err());
        let mut wrong_refresh = row;
        wrong_refresh.controller_sequence = 40;
        assert!(JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            TEST_FRAME_HOUR,
            12,
            &catalog(),
            &controller(),
            &[wrong_refresh],
        )
        .is_err());
    }

    #[test]
    fn callsite_filter_uses_nearest_game_frame_only() {
        let base = 0x1400_0000_0usize;
        let expected = base + DISABLED_REASON_APPEND_RETURN_RVA as usize;
        assert_eq!(
            nearest_game_return_rva(base, 0x4000_0000, &[0x7fff_0000, expected]),
            Some(DISABLED_REASON_APPEND_RETURN_RVA)
        );
        assert_eq!(
            nearest_game_return_rva(
                base,
                0x4000_0000,
                &[0x7fff_0000, base + 0x1234, expected],
            ),
            Some(0x1234)
        );
    }

    #[test]
    fn army_group_context_uses_engine_selection_precedence() {
        let mut controller = controller();
        controller.selected_leader_runtime = 0;
        controller.selected_army_group_runtime = 0x7ff6_2000_0200;
        controller.selected_army_runtime = 0x7ff6_2000_0300;
        assert_eq!(
            selected_context_runtime(&controller),
            Ok(0x7ff6_2000_0200)
        );
    }

    #[test]
    fn portable_token_filter_rejects_pointer_shapes_not_embedded_hex_text() {
        assert!(valid_portable_token("GFX_ability_model_0x_variant"));
        assert!(!valid_portable_token("0x7ff610000000"));
        assert!(!valid_portable_token("ability:0x7ff610000000"));
    }
}
