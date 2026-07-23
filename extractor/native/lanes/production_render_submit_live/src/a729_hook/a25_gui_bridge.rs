//! A25 GUI list/owner/final-D3D bridge.
//!
//! The exact GUI-list and virtual-render hooks are installed as part of the
//! authenticated render lifecycle. Optional pointer-bearing diagnostics remain
//! behind an explicit output environment variable; production ownership never
//! depends on that variable or on persisted diagnostic bytes.

use super::*;
use portable_content_manifest::{
    normalize_game_relative_path, ContentLayerKind, UsedContentKey,
};
use portable_render_stream::{PreparedRenderTarget, RenderTargetDesc};
use runtime_text_inputs_live_lane::RuntimeTextSourceIdentity;
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub const EVIDENCE_OUT_DIR_ENV: &str = "BATTLE_PLAN_A25_GUI_BRIDGE_EVIDENCE_OUT_DIR";
pub const GUI_LIST_CONSUMER_RVA: usize = 0x0222_8C20;
pub const INSTANT_TEXT_RENDER_RVA: usize = 0x022D_8350;
pub const UNITS_STACK_UPDATE_RVA: usize = 0x018F_FE60;
pub const GENERIC_SPRITE_RENDER_RVA: usize = 0x022E_95C0;
pub const PRODUCT_PROMOTION_ENABLED: bool = true;

const GUI_LIST_CONSUMER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xEC, 0x58,
    0x48,
];
const INSTANT_TEXT_RENDER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x6C, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
    0x57,
];
const UNITS_STACK_UPDATE_SENTINEL: [u8; 16] = [
    0x44, 0x88, 0x4C, 0x24, 0x20, 0x44, 0x88, 0x44, 0x24, 0x18, 0x55, 0x53, 0x41, 0x55, 0x41,
    0x56,
];
const GENERIC_SPRITE_RENDER_SENTINEL: [u8; 16] = [
    0x48, 0x8B, 0xC4, 0x48, 0x89, 0x68, 0x18, 0x56, 0x57, 0x41, 0x56, 0x48, 0x81, 0xEC, 0xB0,
    0x00,
];
const MAX_LIST_NODES: usize = 1_048_576;
const MAX_CAPTURED_NODES_PER_PRESENT: usize = 262_144;
const MAX_LISTS_PER_PRESENT: usize = 16_384;
const MAX_DRAWS_PER_OWNER_RUN: usize = 65_536;
const MAX_COUNTER_OWNER_PATHS: usize = 65_536;
const MAX_SELECTED_PRESENT_GUI_ROOTS: usize = 4_096;
const MAX_GRAPHICAL_ANCESTRY_DEPTH: usize = 64;

static ORIGINAL_GUI_LIST_CONSUMER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_INSTANT_TEXT_RENDER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_UNITS_STACK_UPDATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_GENERIC_SPRITE_RENDER: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_LIST_INVOCATION: AtomicU64 = AtomicU64::new(1);
static NEXT_DIAGNOSTIC_DRAW_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static STATE: OnceLock<Mutex<BridgeState>> = OnceLock::new();
static TARGETS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
static COUNTER_OWNER_PATHS: OnceLock<Mutex<BTreeMap<usize, CounterOwnerPath>>> = OnceLock::new();

thread_local! {
    static LIST_STACK: RefCell<Vec<ActiveListTls>> = const { RefCell::new(Vec::new()) };
    static ACTIVE_OWNER_RUN: RefCell<Option<ActiveOwnerRun>> = const { RefCell::new(None) };
}

type GuiListConsumerFn = unsafe extern "system" fn(usize, usize);
type InstantTextRenderFn = unsafe extern "system" fn(usize, usize, usize, usize);
type UnitsStackUpdateFn = unsafe extern "system" fn(usize, usize, u8, u8);
type GenericSpriteRenderFn =
    unsafe extern "system" fn(usize, usize, usize, usize, u32, u32, usize);

struct CallbackFlight;

impl CallbackFlight {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackFlight {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Default)]
struct BridgeState {
    module_base: usize,
    output_dir: Option<PathBuf>,
    current_candidate: Option<(u64, u64)>,
    next_paint_ordinal: u32,
    mapicons_gui_definition: Option<UsedContentKey>,
    gui_definitions: BTreeMap<String, UsedContentKey>,
    selected_present_gui_roots:
        BTreeMap<(u64, u64, usize), SelectedPresentGuiRootRegistration>,
    selected_present_gui_draws: Vec<SelectedPresentGuiDrawCapture>,
    active_lists: BTreeMap<u64, DiagnosticGuiList>,
    completed_lists: VecDeque<DiagnosticGuiList>,
    captured_node_count: usize,
    rejected_list_count: u64,
    rejected_owner_run_count: u64,
    rejected_draw_count: u64,
    queue_overflow_count: u64,
}

#[derive(Clone)]
struct ActiveListTls {
    invocation_ordinal: u64,
    nodes: Vec<DiagnosticGuiNode>,
}

struct ActiveOwnerRun {
    list_invocation_ordinal: u64,
    node_ordinal: u32,
    thread_id: u32,
    layout_object_ptr: usize,
    graphics_ptr: usize,
    render_context_ptr: usize,
    start_qpc_ticks: u64,
    game_hour_raw: Option<i32>,
    common_gui_operation:
        Option<crate::a25_runtime_text_present::CommonGuiOperationIdentity>,
    is_generic_sprite_owner: bool,
    owner: Option<DiagnosticOwnerJoin>,
    counter_owner: Option<DiagnosticCounterOwnerJoin>,
    transform_f32_bits: Option<[u32; 16]>,
    object_rect_candidate_i32: Option<[i32; 4]>,
    object_clip_candidate_i32: Option<[i32; 4]>,
    draws: Vec<DiagnosticDrawFingerprint>,
    static_d3d_captures: Vec<RawStaticD3dCapture>,
    capture_failure_count: u32,
}

#[derive(Clone)]
struct CounterOwnerPath {
    owner: usize,
    gui_root: usize,
    wrapper: usize,
    paint_root: usize,
    grid_container: usize,
    owner_kind: i32,
}

struct RuntimeAuthoredIdentity {
    authored_type_ptr: usize,
    runtime_widget_vtable_rva_hex: String,
    authored_type_vtable_rva_hex: String,
    selected_gui_definition: UsedContentKey,
    source_path: String,
    symbol: String,
    source_line: u32,
}

/// Process-local identity for an exact common-GUI traversal root. Pointer
/// fields are erased before any portable replay carrier is produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPresentGuiRootRegistration {
    pub candidate_submit_index: u64,
    pub candidate_frame_sequence: u64,
    pub authored_surface_identity: String,
    pub runtime_root_identity: String,
    pub effective_interface_winner: String,
    pub callback_qpc_ticks: u64,
    pub game_hour_raw: i32,
    pub controller_ptr: usize,
    pub runtime_root_ptr: usize,
    pub runtime_graphical_subobject_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPresentGuiDrawCapture {
    pub candidate_submit_index: u64,
    pub candidate_frame_sequence: u64,
    pub list_invocation_ordinal: u64,
    pub node_ordinal: u32,
    pub owner_begin_qpc_ticks: u64,
    pub owner_game_hour_raw: Option<i32>,
    /// Process-local join key only; consumers must erase it before publication.
    pub graphical_node_ptr: usize,
    /// Process-local join key only; consumers must erase it before publication.
    pub runtime_widget_ptr: usize,
    pub operation: crate::a25_runtime_text_present::CommonGuiOperationIdentity,
    pub root: Option<SelectedPresentGuiRootRegistration>,
    pub authored_node: Option<SelectedPresentGuiAuthoredNodeIdentity>,
    pub authored_child_path: Option<Vec<u32>>,
    pub graphical_child_path: Option<Vec<u32>>,
    pub static_d3d_captures: Vec<RawStaticD3dCapture>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPresentGuiAuthoredNodeIdentity {
    pub runtime_widget_vtable_rva_hex: String,
    pub authored_type_vtable_rva_hex: String,
    pub selected_gui_definition: UsedContentKey,
    pub authored_source_path: String,
    pub authored_symbol: String,
    pub authored_source_line: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticGuiNode {
    node_ordinal: u32,
    #[serde(skip)]
    graphical_node_ptr: usize,
    graphical_node_ptr_hex: String,
    graphical_node_vtable_ptr_hex: String,
    render_vslot_50_target_ptr_hex: String,
    render_vslot_50_target_rva_hex: Option<String>,
    exact_instant_text_dispatch: bool,
    exact_generic_sprite_dispatch: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticGuiList {
    list_invocation_ordinal: String,
    candidate_submit_index: String,
    candidate_frame_sequence: String,
    thread_id: u32,
    begin_qpc_ticks: String,
    end_qpc_ticks: String,
    graphics_ptr_hex: String,
    list_ptr_hex: String,
    declared_node_count: u32,
    captured_node_count: u32,
    nodes_truncated: bool,
    nodes: Vec<DiagnosticGuiNode>,
    owner_runs: Vec<DiagnosticOwnerRun>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticOwnerJoin {
    owner_ptr_hex: String,
    context_id: String,
    view_id: String,
    element_id: String,
    source_kind: String,
    source_value: String,
    selected_final_text_sha256: String,
    selected_final_text_hex: String,
    selected_final_text_digest_valid: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticCounterOwnerJoin {
    owner_ptr_hex: String,
    gui_root_ptr_hex: String,
    wrapper_ptr_hex: String,
    paint_root_ptr_hex: String,
    grid_container_ptr_hex: String,
    owner_kind: i32,
    graphical_ancestry_edges: u32,
    gui_owner_vtable_rva_hex: Option<String>,
    runtime_root_vtable_rva_hex: Option<String>,
    graphical_node_vtable_rva_hex: Option<String>,
    selected_gui_definition: Option<UsedContentKey>,
    authored_surface_identity: Option<&'static str>,
    runtime_root: &'static str,
    runtime_widget_ptr_hex: Option<String>,
    runtime_widget_vtable_rva_hex: Option<String>,
    authored_type_ptr_hex: Option<String>,
    authored_type_vtable_rva_hex: Option<String>,
    authored_source_path: Option<String>,
    authored_symbol: Option<String>,
    authored_source_line: Option<u32>,
    authored_child_path: Option<Vec<u32>>,
    graphical_child_path: Option<Vec<u32>>,
    path_validation: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticOwnerRun {
    list_invocation_ordinal: String,
    node_ordinal: u32,
    thread_id: u32,
    layout_object_ptr_hex: String,
    graphics_ptr_hex: String,
    render_context_ptr_hex: String,
    begin_qpc_ticks: String,
    end_qpc_ticks: String,
    engine_instance_token: Option<String>,
    paint_ordinal: Option<u32>,
    owner: Option<DiagnosticOwnerJoin>,
    counter_owner: Option<DiagnosticCounterOwnerJoin>,
    transform_f32_bits: Option<[u32; 16]>,
    object_rect_candidate_i32: Option<[i32; 4]>,
    object_clip_candidate_i32: Option<[i32; 4]>,
    engine_token_candidates: DiagnosticEngineTokenCandidates,
    draws: Vec<DiagnosticDrawFingerprint>,
    capture_failure_count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticEngineTokenCandidates {
    graphical_node_ptr_decimal: String,
    layout_object_ptr_decimal: String,
    owner_ptr_decimal: Option<String>,
    present_scoped_candidate_sha256: String,
    authority_status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticShaderBinding {
    stage: &'static str,
    shader_ptr_hex: String,
    bytecode_sha256: Option<String>,
    creation_sequence: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticPayloadBinding {
    class: &'static str,
    stage: Option<&'static str>,
    slot: u32,
    object_ptr_hex: String,
    view_ptr_hex: Option<String>,
    payload_sha256: Option<String>,
    descriptor_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticDrawFingerprint {
    diagnostic_sequence: String,
    capture_sequence: String,
    qpc_ticks: String,
    producer_rva_hex: String,
    draw_kind: &'static str,
    draw_arguments_i64: [i64; 5],
    render_context_ptr_hex: String,
    shader_program_ptr_hex: String,
    shaders: Vec<DiagnosticShaderBinding>,
    payload_bindings: Vec<DiagnosticPayloadBinding>,
    target: Option<RenderTargetDesc>,
    target_descriptor_sha256: Option<String>,
    geometry_state_sha256: String,
    pipeline_state_sha256: String,
    output_state_sha256: String,
    draw_payload_sha256: String,
    raw_d3d_fingerprint_sha256: String,
    capture_complete: bool,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticOwnerRunRegrade {
    list_invocation_ordinal: String,
    node_ordinal: u32,
    exact_vslot_dispatch: bool,
    list_and_owner_thread_match: bool,
    list_and_owner_graphics_match: bool,
    layout_equals_graphical_node: bool,
    registered_owner_resolved: bool,
    registered_owner_path_candidate_present: bool,
    diagnostic_token_candidate_present: bool,
    qpc_order_complete: bool,
    draw_count: u32,
    one_render_target_descriptor: bool,
    all_draw_fingerprints_complete: bool,
    candidate_join_sha256: String,
    owner_path_token_candidate_sha256: String,
    missing_production_authority: Vec<&'static str>,
    promotion_eligible: bool,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticPresentCapture {
    schema: &'static str,
    authority_executable_sha256: &'static str,
    gui_list_consumer_rva_hex: &'static str,
    gui_list_consumer_job_id: &'static str,
    gui_list_consumer_manifest_sha256: &'static str,
    gui_list_consumer_decompile_sha256: &'static str,
    gui_list_consumer_instructions_sha256: &'static str,
    instant_text_render_rva_hex: &'static str,
    instant_text_render_job_id: &'static str,
    instant_text_render_manifest_sha256: &'static str,
    instant_text_render_decompile_sha256: &'static str,
    units_stack_update_rva_hex: &'static str,
    generic_sprite_render_rva_hex: &'static str,
    updater_generic_hook_job_id: &'static str,
    updater_generic_hook_manifest_sha256: &'static str,
    hook_sentinel_job_id: &'static str,
    hook_sentinel_artifact_sha256: &'static str,
    production_promotion_enabled: bool,
    process_start_nonce: String,
    hook_epoch_nonce: String,
    candidate_submit_index: String,
    candidate_frame_sequence: String,
    game_hour_raw: i32,
    present_global_order: u32,
    present_qpc_ticks: String,
    qpc_frequency_hz: String,
    lists: Vec<DiagnosticGuiList>,
    owner_run_regrades: Vec<DiagnosticOwnerRunRegrade>,
    rejected_list_count: u64,
    rejected_owner_run_count: u64,
    rejected_draw_count: u64,
    queue_overflow_count: u64,
    receipt_body_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PointerFreeCap6Receipt {
    schema: &'static str,
    body: PointerFreeCap6ReceiptBody,
    body_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PointerFreeCap6ReceiptBody {
    authority_executable_sha256: &'static str,
    units_stack_update_rva_hex: &'static str,
    generic_sprite_render_rva_hex: &'static str,
    gui_list_consumer_rva_hex: &'static str,
    units_stack_update_job_id: &'static str,
    units_stack_update_manifest_sha256: &'static str,
    hook_sentinel_job_id: &'static str,
    hook_sentinel_artifact_sha256: &'static str,
    owner_path_live_receipt_job_id: &'static str,
    owner_path_live_receipt_sha256: &'static str,
    common_draw_live_receipt_job_id: &'static str,
    common_draw_live_receipt_sha256: &'static str,
    candidate_submit_index: String,
    candidate_frame_sequence: String,
    game_hour_raw: i32,
    present_global_order: u32,
    present_qpc_ticks: String,
    qpc_frequency_hz: String,
    counter_owner_count: u32,
    generic_owner_run_count: u32,
    final_draw_count: u32,
    selected_target_descriptor_sha256: Option<String>,
    selected_present_sealable: bool,
    generic_owner_runs: Vec<PointerFreeCap6OwnerRun>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PointerFreeCap6OwnerRun {
    list_invocation_ordinal: String,
    node_ordinal: u32,
    present_owner_ordinal: u32,
    owner_kind: i32,
    graphical_ancestry_edges: u32,
    exact_generic_sprite_dispatch: bool,
    begin_qpc_ticks: String,
    end_qpc_ticks: String,
    draws: Vec<PointerFreeCap6Draw>,
    capture_failure_count: u32,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PointerFreeCap6Draw {
    diagnostic_sequence: String,
    capture_sequence: String,
    qpc_ticks: String,
    producer_rva_hex: String,
    draw_kind: &'static str,
    draw_arguments_i64: [i64; 5],
    shader_bytecode_sha256: Vec<String>,
    payload_bindings: Vec<PointerFreeCap6PayloadBinding>,
    target: Option<RenderTargetDesc>,
    target_descriptor_sha256: Option<String>,
    capture_complete: bool,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct PointerFreeCap6PayloadBinding {
    class: &'static str,
    stage: Option<&'static str>,
    slot: u32,
    payload_sha256: Option<String>,
    descriptor_sha256: Option<String>,
}

pub(super) fn install(module_base: usize) -> Result<(), String> {
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let output_dir = std::env::var_os(EVIDENCE_OUT_DIR_ENV)
        .map(PathBuf::from)
        .map(|path| {
            if !path.is_absolute() || !path.is_dir() {
                Err(format!(
                    "{EVIDENCE_OUT_DIR_ENV} must name an existing absolute directory"
                ))
            } else {
                Ok(path)
            }
        })
        .transpose()?;
    let list_target = module_base
        .checked_add(GUI_LIST_CONSUMER_RVA)
        .ok_or("A25 GUI list consumer address overflowed")?;
    let text_target = module_base
        .checked_add(INSTANT_TEXT_RENDER_RVA)
        .ok_or("A25 instant-text render address overflowed")?;
    let stack_update_target = module_base
        .checked_add(UNITS_STACK_UPDATE_RVA)
        .ok_or("A25 units-stack update address overflowed")?;
    let generic_sprite_target = module_base
        .checked_add(GENERIC_SPRITE_RENDER_RVA)
        .ok_or("A25 generic-sprite render address overflowed")?;
    verify_sentinel(
        list_target,
        &GUI_LIST_CONSUMER_SENTINEL,
        "A25 GUI list consumer",
    )?;
    verify_sentinel(
        text_target,
        &INSTANT_TEXT_RENDER_SENTINEL,
        "A25 instant-text render",
    )?;
    verify_sentinel(
        stack_update_target,
        &UNITS_STACK_UPDATE_SENTINEL,
        "A25 units-stack update",
    )?;
    verify_sentinel(
        generic_sprite_target,
        &GENERIC_SPRITE_RENDER_SENTINEL,
        "A25 generic-sprite render",
    )?;
    let specs: [HookSpec; 4] = [
        (
            list_target,
            hook_gui_list_consumer as *mut c_void,
            &ORIGINAL_GUI_LIST_CONSUMER,
        ),
        (
            text_target,
            hook_instant_text_render as *mut c_void,
            &ORIGINAL_INSTANT_TEXT_RENDER,
        ),
        (
            stack_update_target,
            hook_units_stack_update as *mut c_void,
            &ORIGINAL_UNITS_STACK_UPDATE,
        ),
        (
            generic_sprite_target,
            hook_generic_sprite_render as *mut c_void,
            &ORIGINAL_GENERIC_SPRITE_RENDER,
        ),
    ];
    {
        let mut state = STATE
            .get_or_init(|| Mutex::new(BridgeState::default()))
            .lock()
            .map_err(|_| "A25 GUI bridge state poisoned")?;
        *state = BridgeState {
            module_base,
            output_dir,
            ..BridgeState::default()
        };
    }
    COUNTER_OWNER_PATHS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "A25 counter-owner path state poisoned")?
        .clear();
    let targets = create_and_enable_hooks(&specs, "A25 common-GUI owner bridge")?;
    *TARGETS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "A25 GUI bridge target state poisoned")? = targets;
    NEXT_LIST_INVOCATION.store(1, Ordering::Release);
    NEXT_DIAGNOSTIC_DRAW_SEQUENCE.store(1, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    CALLBACKS_OPEN.store(true, Ordering::Release);
    Ok(())
}

pub(super) fn uninstall() -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    CALLBACKS_OPEN.store(false, Ordering::Release);
    let base = STATE
        .get()
        .and_then(|state| state.lock().ok().map(|state| state.module_base))
        .unwrap_or(0);
    let specs: [HookSpec; 4] = [
        (
            base.saturating_add(GUI_LIST_CONSUMER_RVA),
            hook_gui_list_consumer as *mut c_void,
            &ORIGINAL_GUI_LIST_CONSUMER,
        ),
        (
            base.saturating_add(INSTANT_TEXT_RENDER_RVA),
            hook_instant_text_render as *mut c_void,
            &ORIGINAL_INSTANT_TEXT_RENDER,
        ),
        (
            base.saturating_add(UNITS_STACK_UPDATE_RVA),
            hook_units_stack_update as *mut c_void,
            &ORIGINAL_UNITS_STACK_UPDATE,
        ),
        (
            base.saturating_add(GENERIC_SPRITE_RENDER_RVA),
            hook_generic_sprite_render as *mut c_void,
            &ORIGINAL_GENERIC_SPRITE_RENDER,
        ),
    ];
    let created = TARGETS
        .get()
        .and_then(|targets| targets.lock().ok().map(|targets| targets.len()))
        .unwrap_or(0);
    let removal = if base != 0 && created != 0 {
        rollback_created_hooks(&specs, created)
    } else {
        Ok(())
    };
    removal?;
    INSTALLED.store(false, Ordering::Release);
    if let Some(targets) = TARGETS.get() {
        targets
            .lock()
            .map_err(|_| "A25 GUI bridge target state poisoned")?
            .clear();
    }
    if let Some(state) = STATE.get() {
        *state
            .lock()
            .map_err(|_| "A25 GUI bridge state poisoned")? = BridgeState::default();
    }
    if let Some(paths) = COUNTER_OWNER_PATHS.get() {
        paths
            .lock()
            .map_err(|_| "A25 counter-owner path state poisoned")?
            .clear();
    }
    LIST_STACK.with(|stack| stack.borrow_mut().clear());
    ACTIVE_OWNER_RUN.with(|run| *run.borrow_mut() = None);
    Ok(())
}

pub(super) fn owns_original_slot(original: &'static AtomicUsize) -> bool {
    std::ptr::eq(original, &ORIGINAL_GUI_LIST_CONSUMER)
        || std::ptr::eq(original, &ORIGINAL_INSTANT_TEXT_RENDER)
        || std::ptr::eq(original, &ORIGINAL_UNITS_STACK_UPDATE)
        || std::ptr::eq(original, &ORIGINAL_GENERIC_SPRITE_RENDER)
}

pub(super) fn begin_teardown() {
    CALLBACKS_OPEN.store(false, Ordering::Release);
}

pub(super) fn wait_for_quiescence() -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) != 0 {
        if std::time::Instant::now() >= deadline {
            return Err("A25 GUI bridge callbacks did not quiesce after disable".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

pub(super) fn owner_run_is_active() -> bool {
    INSTALLED.load(Ordering::Acquire)
        && STATE
            .get()
            .and_then(|state| state.lock().ok())
            .is_some_and(|state| {
                state.output_dir.is_some() || product_capture_is_active_locked(&state)
            })
        && ACTIVE_OWNER_RUN.with(|run| run.borrow().is_some())
}

fn product_capture_is_active_locked(state: &BridgeState) -> bool {
    PRODUCT_PROMOTION_ENABLED
        && INSTALLED.load(Ordering::Acquire)
        && CALLBACKS_OPEN.load(Ordering::Acquire)
        && state.module_base != 0
        && !state.gui_definitions.is_empty()
}

fn product_capture_is_active() -> bool {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .is_some_and(|state| product_capture_is_active_locked(&state))
}

pub(super) fn discard_candidate() {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.rejected_list_count = state.rejected_list_count.saturating_add(
        state.active_lists.len() as u64 + state.completed_lists.len() as u64,
    );
    state.active_lists.clear();
    state.completed_lists.clear();
    state.selected_present_gui_roots.clear();
    state.selected_present_gui_draws.clear();
    state.captured_node_count = 0;
    state.current_candidate = None;
    state.next_paint_ordinal = 0;
    LIST_STACK.with(|stack| stack.borrow_mut().clear());
    ACTIVE_OWNER_RUN.with(|run| *run.borrow_mut() = None);
}

#[cfg(test)]
pub(super) fn reset_test_state() {
    INSTALLED.store(false, Ordering::Release);
    CALLBACKS_OPEN.store(false, Ordering::Release);
    IN_FLIGHT_CALLBACKS.store(0, Ordering::Release);
    NEXT_LIST_INVOCATION.store(1, Ordering::Release);
    NEXT_DIAGNOSTIC_DRAW_SEQUENCE.store(1, Ordering::Release);
    *STATE
        .get_or_init(|| Mutex::new(BridgeState::default()))
        .lock()
        .expect("A25 GUI bridge test state poisoned") = BridgeState::default();
    if let Some(targets) = TARGETS.get() {
        targets
            .lock()
            .expect("A25 GUI bridge test target state poisoned")
            .clear();
    }
    if let Some(paths) = COUNTER_OWNER_PATHS.get() {
        paths
            .lock()
            .expect("A25 GUI bridge test counter-owner state poisoned")
            .clear();
    }
    LIST_STACK.with(|stack| stack.borrow_mut().clear());
    ACTIVE_OWNER_RUN.with(|run| *run.borrow_mut() = None);
}

fn current_pending_render_frame() -> Option<PendingRenderFrame> {
    super::STATE
        .get()?
        .lock()
        .ok()?
        .pending_render_frame
}

pub(super) fn bind_effective_vfs_winner_index(
    binding: &crate::RawEffectiveVfsWinnerIndexBinding,
) -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("A25 GUI bridge is not installed".into());
    }
    binding.validate()?;
    let mut gui_definitions = BTreeMap::new();
    for raw in binding.ordered_rows.iter().filter(|row| {
        row.content.logical_path.starts_with("interface/")
            && row.content.logical_path.ends_with(".gui")
    }) {
        binding.authorize_content(&raw.content)?;
        let definition = used_content_key(binding, &raw.content)?;
        if gui_definitions
            .insert(raw.content.logical_path.clone(), definition)
            .is_some()
        {
            return Err("effective VFS contains a duplicate GUI winner".into());
        }
    }
    let definition = gui_definitions
        .get("interface/mapicons.gui")
        .cloned()
        .ok_or("effective VFS omits interface/mapicons.gui")?;
    runtime_text_inputs_live_lane::a25_post_emit_hook::bind_authenticated_font_assets(
        authenticated_font_asset_receipts(binding)?,
    )?;
    runtime_text_inputs_live_lane::a25_post_emit_hook::bind_authenticated_inline_icon_content(
        authenticated_inline_icon_content(binding)?,
    )?;
    let mut state = STATE
        .get()
        .ok_or("A25 GUI bridge state is absent")?
        .lock()
        .map_err(|_| "A25 GUI bridge state poisoned")?;
    if state.mapicons_gui_definition.as_ref().is_some_and(|existing| {
        existing != &definition
    }) {
        return Err("A25 GUI bridge mapicons winner was rebound".into());
    }
    if !state.gui_definitions.is_empty() && state.gui_definitions != gui_definitions {
        return Err("A25 GUI bridge GUI winner index was rebound".into());
    }
    state.mapicons_gui_definition = Some(definition);
    state.gui_definitions = gui_definitions;
    Ok(())
}

fn authenticated_inline_icon_content(
    binding: &crate::RawEffectiveVfsWinnerIndexBinding,
) -> Result<Vec<(String, String)>, String> {
    let mut rows = Vec::new();
    for row in binding.ordered_rows.iter().filter(|row| {
        let path = row.content.logical_path.as_str();
        path.ends_with(".gfx")
            || path.ends_with(".gui")
            || path.ends_with(".dds")
            || path.ends_with(".tga")
            || path.ends_with(".png")
    }) {
        binding.authorize_content(&row.content)?;
        rows.push((
            row.content.logical_path.clone(),
            row.content.payload_sha256.clone(),
        ));
    }
    if rows.is_empty() {
        return Err("authenticated VFS contains no inline-icon definitions or textures".into());
    }
    Ok(rows)
}

fn authenticated_font_asset_receipts(
    binding: &crate::RawEffectiveVfsWinnerIndexBinding,
) -> Result<
    Vec<runtime_text_inputs_live_lane::owner_glyph_capture::FontAssetReceipt>,
    String,
> {
    let rows = binding
        .ordered_rows
        .iter()
        .map(|row| (row.content.logical_path.as_str(), &row.content))
        .collect::<BTreeMap<_, _>>();
    let mut receipts = Vec::new();
    for (definition_path, definition) in rows
        .iter()
        .filter(|(path, _)| path.ends_with(".fnt"))
    {
        let stem = definition_path
            .strip_suffix(".fnt")
            .ok_or("font definition suffix disappeared")?;
        let mut numbered = rows
            .iter()
            .filter(|(path, _)| {
                path.strip_prefix(stem).is_some_and(|suffix| {
                    suffix
                        .strip_prefix('_')
                        .and_then(|suffix| suffix.strip_suffix(".dds"))
                        .is_some_and(|ordinal| {
                            !ordinal.is_empty()
                                && ordinal.bytes().all(|byte| byte.is_ascii_digit())
                        })
                })
            })
            .map(|(path, content)| ((*path).to_string(), *content))
            .collect::<Vec<_>>();
        numbered.sort_by(|left, right| left.0.cmp(&right.0));
        let atlas = if !numbered.is_empty() {
            numbered
        } else {
            [format!("{stem}.dds"), format!("{stem}.tga")]
                .into_iter()
                .find_map(|path| rows.get(path.as_str()).map(|content| vec![(path, *content)]))
                .unwrap_or_default()
        };
        if atlas.is_empty() {
            continue;
        }
        receipts.push(runtime_text_inputs_live_lane::owner_glyph_capture::FontAssetReceipt {
            identity: (*definition_path).to_string(),
            descriptor_sha256: definition.payload_sha256.clone(),
            atlas_sha256: atlas
                .into_iter()
                .map(|(_, content)| content.payload_sha256.clone())
                .collect(),
        });
    }
    if receipts.is_empty() {
        return Err("authenticated VFS contains no bindable bitmap-font assets".into());
    }
    Ok(receipts)
}

fn used_content_key(
    binding: &crate::RawEffectiveVfsWinnerIndexBinding,
    raw: &crate::RawVfsContentIdentity,
) -> Result<UsedContentKey, String> {
    let layer_kind = match raw.layer_kind.as_str() {
        "base" => ContentLayerKind::Base,
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => return Err("GUI winner has an unsupported layer kind".into()),
    };
    let definition = UsedContentKey {
        effective_vfs_manifest_sha256: binding.authority_manifest_sha256.clone(),
        normalized_game_relative_path: raw.logical_path.clone(),
        winning_layer_ordinal: raw.winning_layer_ordinal,
        layer_kind,
        layer_id: raw.layer_id.clone(),
        source_sha256: raw.source_sha256.clone(),
        payload_sha256: raw.payload_sha256.clone(),
        payload_bytes: raw.payload_bytes,
    };
    definition.validate()?;
    Ok(definition)
}

pub(super) fn register_selected_present_gui_root(
    candidate_submit_index: u64,
    candidate_frame_sequence: u64,
    authored_surface_identity: &str,
    runtime_root_identity: &str,
    effective_interface_winner: &str,
    callback_qpc_ticks: u64,
    game_hour_raw: i32,
    controller_ptr: usize,
    runtime_root_ptr: usize,
    runtime_graphical_subobject_ptr: usize,
) -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("A25 GUI bridge is not installed".into());
    }
    if authored_surface_identity.trim().is_empty()
        || runtime_root_identity.trim().is_empty()
        || authored_surface_identity != authored_surface_identity.trim()
        || runtime_root_identity != runtime_root_identity.trim()
        || authored_surface_identity.contains('/')
        || authored_surface_identity.contains('\\')
        || runtime_root_identity.contains('/')
        || runtime_root_identity.contains('\\')
        || candidate_submit_index == 0
        || candidate_frame_sequence == 0
        || callback_qpc_ticks == 0
        || controller_ptr == 0
        || runtime_root_ptr == 0
        || runtime_graphical_subobject_ptr == 0
    {
        return Err("selected-Present GUI root registration is incomplete".into());
    }
    let normalized = normalize_game_relative_path(effective_interface_winner)?;
    if normalized != effective_interface_winner
        || !normalized.starts_with("interface/")
        || !normalized.ends_with(".gui")
    {
        return Err("selected-Present GUI root winner is not one canonical interface GUI path".into());
    }
    let registration = SelectedPresentGuiRootRegistration {
        candidate_submit_index,
        candidate_frame_sequence,
        authored_surface_identity: authored_surface_identity.to_string(),
        runtime_root_identity: runtime_root_identity.to_string(),
        effective_interface_winner: effective_interface_winner.to_string(),
        callback_qpc_ticks,
        game_hour_raw,
        controller_ptr,
        runtime_root_ptr,
        runtime_graphical_subobject_ptr,
    };
    let mut state = STATE
        .get()
        .ok_or("A25 GUI bridge state is absent")?
        .lock()
        .map_err(|_| "A25 GUI bridge state poisoned")?;
    if !state.gui_definitions.contains_key(effective_interface_winner) {
        return Err("selected-Present GUI root winner is absent from the authenticated VFS".into());
    }
    let key = (
        candidate_submit_index,
        candidate_frame_sequence,
        runtime_graphical_subobject_ptr,
    );
    if state
        .selected_present_gui_roots
        .get(&key)
        .is_some_and(|existing| existing != &registration)
    {
        return Err("selected-Present GUI root was rebound within one candidate".into());
    }
    if !state.selected_present_gui_roots.contains_key(&key)
        && state.selected_present_gui_roots.len() >= MAX_SELECTED_PRESENT_GUI_ROOTS
    {
        return Err("selected-Present GUI root registration limit exceeded".into());
    }
    let matching_draw_updates = state
        .selected_present_gui_draws
        .iter()
        .enumerate()
        .filter(|(_, capture)| {
            capture.candidate_submit_index == candidate_submit_index
                && capture.candidate_frame_sequence == candidate_frame_sequence
                && graphical_is_descendant_or_self(
                    runtime_graphical_subobject_ptr,
                    capture.graphical_node_ptr,
                )
        })
        .map(|(index, capture)| {
            let authored_child_path =
                runtime_authored_child_path(runtime_root_ptr, capture.runtime_widget_ptr)?;
            let graphical_child_path = graphical_child_path(
                runtime_graphical_subobject_ptr,
                capture.graphical_node_ptr,
            )?;
            capture.authored_node.as_ref()?;
            Some((index, authored_child_path, graphical_child_path))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("selected-Present GUI root cannot derive exact authored/graphical child paths")?;
    if matching_draw_updates.iter().any(|(index, _, _)| {
        state.selected_present_gui_draws[*index]
            .root
            .as_ref()
            .is_some_and(|existing| existing != &registration)
    }) {
        return Err("selected-Present GUI draw matched conflicting registered roots".into());
    }
    state
        .selected_present_gui_roots
        .insert(key, registration.clone());
    for (index, authored_child_path, graphical_child_path) in matching_draw_updates {
        state.selected_present_gui_draws[index].root = Some(registration.clone());
        state.selected_present_gui_draws[index].authored_child_path =
            Some(authored_child_path);
        state.selected_present_gui_draws[index].graphical_child_path =
            Some(graphical_child_path);
    }
    Ok(())
}

pub(super) fn take_selected_present_gui_draws(
    candidate_submit_index: u64,
    candidate_frame_sequence: u64,
) -> Result<Vec<SelectedPresentGuiDrawCapture>, String> {
    if candidate_submit_index == 0 || candidate_frame_sequence == 0 {
        return Err("selected-Present GUI draw candidate identity is incomplete".into());
    }
    let mut state = STATE
        .get()
        .ok_or("A25 GUI bridge state is absent")?
        .lock()
        .map_err(|_| "A25 GUI bridge state poisoned")?;
    if state.selected_present_gui_draws.iter().any(|capture| {
        capture.candidate_submit_index != candidate_submit_index
            || capture.candidate_frame_sequence != candidate_frame_sequence
    }) {
        return Err("selected-Present GUI draw queue spans multiple candidates".into());
    }
    let mut captures = mem::take(&mut state.selected_present_gui_draws);
    let module_base = state.module_base;
    state.selected_present_gui_roots.retain(
        |(submit_index, frame_sequence, _), _| {
            *submit_index != candidate_submit_index
                || *frame_sequence != candidate_frame_sequence
        },
    );
    drop(state);
    for capture in &mut captures {
        if capture.root.is_none() {
            derive_automatic_global_root(capture, module_base)?;
        }
        if capture.root.is_none()
            || capture.authored_node.is_none()
            || capture.authored_child_path.is_none()
            || capture.graphical_child_path.is_none()
        {
            return Err("selected-Present GUI draw has no complete mechanical root identity".into());
        }
    }
    Ok(captures)
}

pub(super) fn static_d3d_captures_for_active_candidate(
) -> Result<Vec<RawStaticD3dCapture>, String> {
    let state = STATE
        .get()
        .ok_or("A25 GUI bridge state is absent")?
        .lock()
        .map_err(|_| "A25 GUI bridge state poisoned")?;
    let Some(candidate) = state.current_candidate else {
        return state
            .selected_present_gui_draws
            .is_empty()
            .then_some(Vec::new())
            .ok_or_else(|| "A25 GUI static captures have no active Present candidate".into());
    };
    if state.selected_present_gui_draws.iter().any(|capture| {
        (capture.candidate_submit_index, capture.candidate_frame_sequence) != candidate
    }) {
        return Err("A25 GUI static capture queue spans multiple candidates".into());
    }
    Ok(state
        .selected_present_gui_draws
        .iter()
        .flat_map(|capture| capture.static_d3d_captures.iter().cloned())
        .collect())
}

fn allocate_common_gui_operation(
    pending: PendingRenderFrame,
    list_invocation_ordinal: u64,
    node_ordinal: u32,
) -> Result<crate::a25_runtime_text_present::CommonGuiOperationIdentity, String> {
    let readiness = super::early_capture_readiness()?;
    let paint_ordinal = {
        let mut state = STATE
            .get()
            .ok_or("A25 GUI bridge state is absent")?
            .lock()
            .map_err(|_| "A25 GUI bridge state poisoned")?;
        if state.current_candidate != Some((pending.submit_index, pending.frame_sequence)) {
            return Err("A25 GUI operation differs from the active Present candidate".into());
        }
        let paint_ordinal = state.next_paint_ordinal;
        state.next_paint_ordinal = paint_ordinal
            .checked_add(1)
            .ok_or("A25 GUI paint ordinal overflow")?;
        paint_ordinal
    };
    crate::a25_runtime_text_present::common_gui_operation_identity(
        readiness.process.process_start_nonce,
        readiness.hook_epoch_nonce,
        pending.submit_index,
        pending.frame_sequence,
        list_invocation_ordinal,
        node_ordinal,
        paint_ordinal,
    )
}

fn maybe_allocate_common_gui_operation(
    list_invocation_ordinal: u64,
    node_ordinal: u32,
) -> Result<Option<crate::a25_runtime_text_present::CommonGuiOperationIdentity>, String> {
    if !product_capture_is_active()
        && !runtime_text_inputs_live_lane::owner_glyph_probe::
            production_automation_is_active()
    {
        return Ok(None);
    }
    let pending = current_pending_render_frame()
        .ok_or("common-GUI owner has no pending Present candidate")?;
    allocate_common_gui_operation(pending, list_invocation_ordinal, node_ordinal).map(Some)
}

unsafe extern "system" fn hook_gui_list_consumer(graphics: usize, list: usize) {
    let _callback = CallbackFlight::enter();
    let original_address = ORIGINAL_GUI_LIST_CONSUMER.load(Ordering::Acquire);
    if original_address == 0 {
        return;
    }
    let original: GuiListConsumerFn = unsafe { mem::transmute(original_address) };
    if !CALLBACKS_OPEN.load(Ordering::Acquire) {
        unsafe { original(graphics, list) };
        return;
    }
    let Some(candidate) = current_pending_render_frame() else {
        unsafe { original(graphics, list) };
        return;
    };
    let begin_qpc_ticks = query_performance_counter().unwrap_or(0);
    let invocation_ordinal = NEXT_LIST_INVOCATION.fetch_add(1, Ordering::AcqRel);
    let node_capture = capture_nodes(list);
    let (declared_node_count, nodes, nodes_truncated) = match node_capture {
        Some(value) => value,
        None => {
            reject_list();
            unsafe { original(graphics, list) };
            return;
        }
    };
    let thread_id = unsafe {
        windows_sys::Win32::System::Threading::GetCurrentThreadId()
    };
    let diagnostic = DiagnosticGuiList {
        list_invocation_ordinal: invocation_ordinal.to_string(),
        candidate_submit_index: candidate.submit_index.to_string(),
        candidate_frame_sequence: candidate.frame_sequence.to_string(),
        thread_id,
        begin_qpc_ticks: begin_qpc_ticks.to_string(),
        end_qpc_ticks: "0".into(),
        graphics_ptr_hex: pointer_hex(graphics),
        list_ptr_hex: pointer_hex(list),
        declared_node_count,
        captured_node_count: nodes.len() as u32,
        nodes_truncated,
        nodes: nodes.clone(),
        owner_runs: Vec::new(),
    };
    let accepted = STATE
        .get()
        .and_then(|state| state.lock().ok())
        .is_some_and(|mut state| {
            let candidate_key = (candidate.submit_index, candidate.frame_sequence);
            if state.current_candidate != Some(candidate_key) {
                state.rejected_list_count = state.rejected_list_count.saturating_add(
                    state.active_lists.len() as u64 + state.completed_lists.len() as u64,
                );
                state.active_lists.clear();
                state.completed_lists.clear();
                state.selected_present_gui_roots.clear();
                state.selected_present_gui_draws.clear();
                state.captured_node_count = 0;
                state.next_paint_ordinal = 0;
                state.current_candidate = Some(candidate_key);
            }
            if state.active_lists.len() >= MAX_LISTS_PER_PRESENT
                || state.captured_node_count.saturating_add(nodes.len())
                    > MAX_CAPTURED_NODES_PER_PRESENT
            {
                state.queue_overflow_count = state.queue_overflow_count.saturating_add(1);
                return false;
            }
            state.captured_node_count = state.captured_node_count.saturating_add(nodes.len());
            state
                .active_lists
                .insert(invocation_ordinal, diagnostic)
                .is_none()
        });
    if !accepted {
        reject_list();
        unsafe { original(graphics, list) };
        return;
    }
    LIST_STACK.with(|stack| {
        stack.borrow_mut().push(ActiveListTls {
            invocation_ordinal,
            nodes,
        })
    });
    unsafe { original(graphics, list) };
    let stack_ok = LIST_STACK.with(|stack| {
        stack
            .borrow_mut()
            .pop()
            .is_some_and(|active| active.invocation_ordinal == invocation_ordinal)
    });
    if !stack_ok {
        reject_list();
    }
    finish_list(invocation_ordinal, query_performance_counter().unwrap_or(0));
}

unsafe extern "system" fn hook_instant_text_render(
    layout_object: usize,
    graphics: usize,
    render_context: usize,
    transform: usize,
) {
    let _callback = CallbackFlight::enter();
    let original_address = ORIGINAL_INSTANT_TEXT_RENDER.load(Ordering::Acquire);
    if original_address == 0 {
        return;
    }
    let original: InstantTextRenderFn = unsafe { mem::transmute(original_address) };
    if !CALLBACKS_OPEN.load(Ordering::Acquire) {
        unsafe { original(layout_object, graphics, render_context, transform) };
        return;
    }
    let matched = LIST_STACK.with(|stack| {
        stack.borrow().last().and_then(|active| {
            active
                .nodes
                .iter()
                .find(|node| {
                    node.graphical_node_ptr == layout_object
                        && node.exact_instant_text_dispatch
                })
                .cloned()
                .map(|node| (active.invocation_ordinal, node.node_ordinal))
        })
    });
    let Some((list_invocation_ordinal, node_ordinal)) = matched else {
        reject_owner_run();
        poison_active_owner_run();
        unsafe { original(layout_object, graphics, render_context, transform) };
        return;
    };
    let nested = ACTIVE_OWNER_RUN.with(|run| run.borrow().is_some());
    if nested {
        reject_owner_run();
        poison_active_owner_run();
        unsafe { original(layout_object, graphics, render_context, transform) };
        return;
    }
    let owner = runtime_text_inputs_live_lane::a729_hook::observed_owner_for_render_layout(
        layout_object,
    )
    .map(diagnostic_owner);
    let common_gui_operation = maybe_allocate_common_gui_operation(
        list_invocation_ordinal,
        node_ordinal,
    )
    .ok()
    .flatten();
    let production_scope_started = begin_production_owner_scope(
        layout_object,
        common_gui_operation,
    )
    .unwrap_or(false);
    let active = ActiveOwnerRun {
        list_invocation_ordinal,
        node_ordinal,
        thread_id: unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
        layout_object_ptr: layout_object,
        graphics_ptr: graphics,
        render_context_ptr: render_context,
        start_qpc_ticks: query_performance_counter().unwrap_or(0),
        game_hour_raw: super::capture_present_game_hour_raw().ok(),
        common_gui_operation,
        is_generic_sprite_owner: false,
        owner,
        counter_owner: None,
        transform_f32_bits: read_u32x16(transform),
        object_rect_candidate_i32: read_object_rect_candidate(layout_object),
        object_clip_candidate_i32: read_i32x4(layout_object.saturating_add(0x158)),
        draws: Vec::new(),
        static_d3d_captures: Vec::new(),
        capture_failure_count: 0,
    };
    ACTIVE_OWNER_RUN.with(|run| *run.borrow_mut() = Some(active));
    unsafe { original(layout_object, graphics, render_context, transform) };
    if production_scope_started
        && runtime_text_inputs_live_lane::owner_glyph_probe::
            finish_production_owner_render()
            .is_err()
    {
        reject_owner_run();
        poison_active_owner_run();
    }
    finish_owner_run(query_performance_counter().unwrap_or(0));
}

fn begin_production_owner_scope(
    layout_object: usize,
    operation: Option<crate::a25_runtime_text_present::CommonGuiOperationIdentity>,
) -> Result<bool, String> {
    if !runtime_text_inputs_live_lane::owner_glyph_probe::
        production_automation_is_active()
    {
        return Ok(false);
    }
    let pending = current_pending_render_frame()
        .ok_or("A25 common-GUI owner has no pending render candidate")?;
    let readiness = super::early_capture_readiness()?;
    let game_hour_raw = super::capture_present_game_hour_raw()?;
    let frame_hour = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
        .and_then(|hour| u64::try_from(hour).ok())
        .ok_or("A25 common-GUI owner has a noncanonical game hour")?;
    let destination_rect = read_object_rect_candidate(layout_object)
        .ok_or("A25 common-GUI owner rectangle is unreadable")?;
    if destination_rect[2] <= destination_rect[0]
        || destination_rect[3] <= destination_rect[1]
    {
        return Err("A25 common-GUI owner rectangle has non-positive extent".into());
    }
    let operation = operation
        .ok_or("A25 common-GUI text owner has no allocated operation identity")?;
    runtime_text_inputs_live_lane::owner_glyph_probe::begin_production_owner_render(
        layout_object,
        runtime_text_inputs_live_lane::owner_glyph_probe::OwnerGlyphPresentCandidate {
            process_start_nonce: readiness.process.process_start_nonce,
            hook_epoch_nonce: readiness.hook_epoch_nonce,
            submit_index: pending.submit_index,
            frame_sequence: pending.frame_sequence,
            frame_hour,
        },
        operation.engine_instance_token,
        operation.paint_ordinal,
        destination_rect,
        None,
        None,
    )
}

unsafe extern "system" fn hook_units_stack_update(
    stack: usize,
    selected_unit: usize,
    flag_3: u8,
    flag_4: u8,
) {
    let _callback = CallbackFlight::enter();
    let original_address = ORIGINAL_UNITS_STACK_UPDATE.load(Ordering::Acquire);
    if original_address == 0 {
        return;
    }
    let original: UnitsStackUpdateFn = unsafe { mem::transmute(original_address) };
    unsafe { original(stack, selected_unit, flag_3, flag_4) };
    if CALLBACKS_OPEN.load(Ordering::Acquire) {
        capture_counter_owner_paths(stack);
    }
}

unsafe extern "system" fn hook_generic_sprite_render(
    layout_object: usize,
    graphics: usize,
    render_context: usize,
    transform: usize,
    argument_5: u32,
    argument_6: u32,
    argument_7: usize,
) {
    let _callback = CallbackFlight::enter();
    let original_address = ORIGINAL_GENERIC_SPRITE_RENDER.load(Ordering::Acquire);
    if original_address == 0 {
        return;
    }
    let original: GenericSpriteRenderFn = unsafe { mem::transmute(original_address) };
    if !CALLBACKS_OPEN.load(Ordering::Acquire) {
        unsafe {
            original(
                layout_object,
                graphics,
                render_context,
                transform,
                argument_5,
                argument_6,
                argument_7,
            )
        };
        return;
    }
    let matched = LIST_STACK.with(|stack| {
        stack.borrow().last().and_then(|active| {
            active
                .nodes
                .iter()
                .find(|node| {
                    node.graphical_node_ptr == layout_object
                        && node.exact_generic_sprite_dispatch
                })
                .cloned()
                .map(|node| (active.invocation_ordinal, node.node_ordinal))
        })
    });
    let Some((list_invocation_ordinal, node_ordinal)) = matched else {
        unsafe {
            original(
                layout_object,
                graphics,
                render_context,
                transform,
                argument_5,
                argument_6,
                argument_7,
            )
        };
        return;
    };
    let counter_owner = resolve_counter_owner_path(layout_object)
        .map(|(owner_path, ancestry_edges)| {
            diagnostic_counter_owner(owner_path, layout_object, ancestry_edges)
        });
    if counter_owner.is_none() && !product_capture_is_active() {
        unsafe {
            original(
                layout_object,
                graphics,
                render_context,
                transform,
                argument_5,
                argument_6,
                argument_7,
            )
        };
        return;
    }
    if ACTIVE_OWNER_RUN.with(|run| run.borrow().is_some()) {
        reject_owner_run();
        poison_active_owner_run();
        unsafe {
            original(
                layout_object,
                graphics,
                render_context,
                transform,
                argument_5,
                argument_6,
                argument_7,
            )
        };
        return;
    }
    let active = ActiveOwnerRun {
        list_invocation_ordinal,
        node_ordinal,
        thread_id: unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
        layout_object_ptr: layout_object,
        graphics_ptr: graphics,
        render_context_ptr: render_context,
        start_qpc_ticks: query_performance_counter().unwrap_or(0),
        game_hour_raw: super::capture_present_game_hour_raw().ok(),
        common_gui_operation: maybe_allocate_common_gui_operation(
            list_invocation_ordinal,
            node_ordinal,
        )
        .ok()
        .flatten(),
        is_generic_sprite_owner: true,
        owner: None,
        counter_owner,
        transform_f32_bits: read_u32x16(transform),
        object_rect_candidate_i32: read_object_rect_candidate(layout_object),
        object_clip_candidate_i32: read_i32x4(layout_object.saturating_add(0x158)),
        draws: Vec::new(),
        static_d3d_captures: Vec::new(),
        capture_failure_count: 0,
    };
    ACTIVE_OWNER_RUN.with(|run| *run.borrow_mut() = Some(active));
    unsafe {
        original(
            layout_object,
            graphics,
            render_context,
            transform,
            argument_5,
            argument_6,
            argument_7,
        )
    };
    finish_owner_run(query_performance_counter().unwrap_or(0));
}

pub(super) fn record_final_draw(
    producer_rva: u32,
    draw_call: RawDrawCall,
    render_context: usize,
    shader_program_ptr: usize,
) {
    if !owner_run_is_active() {
        return;
    }
    if shader_program_ptr == 0 || render_context == 0 {
        reject_draw();
        ACTIVE_OWNER_RUN.with(|run| {
            if let Some(run) = run.borrow_mut().as_mut() {
                run.capture_failure_count = run.capture_failure_count.saturating_add(1);
            }
        });
        return;
    }
    let diagnostic_sequence =
        NEXT_DIAGNOSTIC_DRAW_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    let Some(capture_sequence) = next_catalog_observation_sequence() else {
        reject_draw();
        return;
    };
    let Some(captured) =
        capture_d3d_core(capture_sequence, render_context, shader_program_ptr, draw_call)
    else {
        reject_draw();
        ACTIVE_OWNER_RUN.with(|run| {
            if let Some(run) = run.borrow_mut().as_mut() {
                run.capture_failure_count = run.capture_failure_count.saturating_add(1);
            }
        });
        return;
    };
    let fingerprint = diagnostic_fingerprint(
        diagnostic_sequence,
        capture_sequence,
        producer_rva,
        draw_call,
        &captured,
    );
    let Some(static_d3d_capture) = static_d3d_capture_from_core(
        capture_sequence,
        raw_static_draw_call(draw_call),
        captured,
    ) else {
        reject_draw();
        ACTIVE_OWNER_RUN.with(|run| {
            if let Some(run) = run.borrow_mut().as_mut() {
                run.capture_failure_count = run.capture_failure_count.saturating_add(1);
            }
        });
        return;
    };
    let output_targets_admitted = super::STATE
        .get()
        .and_then(|state| state.lock().ok())
        .is_some_and(|mut state| {
            state.pending_render_frame.is_some()
                && record_registered_color_target_uses_locked(
                    &mut state,
                    capture_sequence,
                    &static_d3d_capture.state.ordered_color_target_view_ptrs,
                )
                .is_some()
        });
    if !output_targets_admitted {
        reject_draw();
        ACTIVE_OWNER_RUN.with(|run| {
            if let Some(run) = run.borrow_mut().as_mut() {
                run.capture_failure_count = run.capture_failure_count.saturating_add(1);
            }
        });
        return;
    }
    ACTIVE_OWNER_RUN.with(|run| {
        let mut run = run.borrow_mut();
        let Some(run) = run.as_mut() else {
            reject_draw();
            return;
        };
        if run.draws.len() >= MAX_DRAWS_PER_OWNER_RUN {
            run.capture_failure_count = run.capture_failure_count.saturating_add(1);
            reject_draw();
            return;
        }
        run.draws.push(fingerprint);
        run.static_d3d_captures.push(static_d3d_capture);
    });
}

pub(super) fn seal_present(
    present_global_order: u32,
    present_qpc_ticks: u64,
    qpc_frequency_hz: u64,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    game_hour_raw: i32,
) -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let (
        output_dir,
        candidate,
        lists,
        rejected_list_count,
        rejected_owner_run_count,
        rejected_draw_count,
        queue_overflow_count,
    ) = {
        let mut state = STATE
            .get()
            .ok_or("A25 GUI bridge state is absent")?
            .lock()
            .map_err(|_| "A25 GUI bridge state poisoned")?;
        if !state.active_lists.is_empty() {
            state.rejected_list_count = state
                .rejected_list_count
                .saturating_add(state.active_lists.len() as u64);
            state.active_lists.clear();
        }
        let lists = state.completed_lists.drain(..).collect::<Vec<_>>();
        let candidate = state.current_candidate.take();
        state.captured_node_count = 0;
        state.next_paint_ordinal = 0;
        (
            state.output_dir.clone(),
            candidate,
            lists,
            state.rejected_list_count,
            state.rejected_owner_run_count,
            state.rejected_draw_count,
            state.queue_overflow_count,
        )
    };
    if lists.is_empty() {
        return Ok(());
    }
    let (candidate_submit_index, candidate_frame_sequence) = candidate
        .ok_or("A25 GUI diagnostic lists have no pending render candidate")?;
    if lists.iter().any(|list| {
        list.candidate_submit_index != candidate_submit_index.to_string()
            || list.candidate_frame_sequence != candidate_frame_sequence.to_string()
    }) {
        return Err("A25 GUI diagnostic list candidate identity is inconsistent".into());
    }
    let regrades = lists
        .iter()
        .flat_map(regrade_owner_runs)
        .collect::<Vec<_>>();
    let mut capture = DiagnosticPresentCapture {
        schema: "hoi4.a25.gui-owner-run-diagnostic.v1",
        authority_executable_sha256: A729_EXECUTABLE_SHA256,
        gui_list_consumer_rva_hex: "0x02228c20",
        gui_list_consumer_job_id:
            "20260720T140158Z-w5-a25-gui-list-consumer-abi-50153dfb",
        gui_list_consumer_manifest_sha256:
            "9cef9790657a006993c84582017e0b6ad327072326491a120c0e0f151990af2a",
        gui_list_consumer_decompile_sha256:
            "6f7a3800721afc719f0f1577e7ea925f2db0260d7118532eb5b984957e1d7700",
        gui_list_consumer_instructions_sha256:
            "308c96d416c212481c26d541dbd41ce2482648ac4f63bb5959a108594df95d85",
        instant_text_render_rva_hex: "0x022d8350",
        instant_text_render_job_id:
            "20260720T124822Z-w5-a25-callsite-recovery-134af434",
        instant_text_render_manifest_sha256:
            "8c1ed3ff735f3ed4d33a9beaa7f4961054897ec7682adec64d1e4f9c75c7d2ff",
        instant_text_render_decompile_sha256:
            "52a3e1eac235b4961f6c9ae53b0911cdbef67c7be540861488f82c6bdffac4d3",
        units_stack_update_rva_hex: "0x018ffe60",
        generic_sprite_render_rva_hex: "0x022e95c0",
        updater_generic_hook_job_id:
            "20260721T095716Z-wave6-cap6-a25-updater-generic-hook-abi-890bb2f4",
        updater_generic_hook_manifest_sha256:
            "41e59a4ae0ed9531ae92c06ca08ab543b8040c015bb64b62ef8aabdb2c1a9f9f",
        hook_sentinel_job_id: "20260721T095228Z-wave6-cap6-a25-hook-sentinels-ae1a60f2",
        hook_sentinel_artifact_sha256:
            "de0e56dbfa8f1a17807ef86af58f828a525d85c0bb6d12856ff8729513f647fc",
        production_promotion_enabled: PRODUCT_PROMOTION_ENABLED,
        process_start_nonce: process_start_nonce.to_string(),
        hook_epoch_nonce: hook_epoch_nonce.to_string(),
        candidate_submit_index: candidate_submit_index.to_string(),
        candidate_frame_sequence: candidate_frame_sequence.to_string(),
        game_hour_raw,
        present_global_order,
        present_qpc_ticks: present_qpc_ticks.to_string(),
        qpc_frequency_hz: qpc_frequency_hz.to_string(),
        lists,
        owner_run_regrades: regrades,
        rejected_list_count,
        rejected_owner_run_count,
        rejected_draw_count,
        queue_overflow_count,
        receipt_body_sha256: String::new(),
    };
    capture.receipt_body_sha256 = sha256_json(&capture)?;
    let Some(output_dir) = output_dir else {
        return Ok(());
    };
    let path = output_dir.join(format!(
        "a25-gui-owner-run-p{present_global_order}-qpc{present_qpc_ticks}.json"
    ));
    persist_create_new_or_equal(&path, &capture)?;
    let cap6 = pointer_free_cap6_receipt(&capture)?;
    if cap6.body.generic_owner_runs.is_empty() {
        return Ok(());
    }
    let cap6_path = output_dir.join(format!(
        "cap6-counter-draw-p{present_global_order}-qpc{present_qpc_ticks}.json"
    ));
    persist_create_new_or_equal(&cap6_path, &cap6)
}

fn pointer_free_cap6_receipt(
    capture: &DiagnosticPresentCapture,
) -> Result<PointerFreeCap6Receipt, String> {
    let mut owner_ordinals = BTreeMap::<String, u32>::new();
    let mut generic_owner_runs = Vec::new();
    for list in &capture.lists {
        for run in &list.owner_runs {
            let Some(counter_owner) = &run.counter_owner else {
                continue;
            };
            let next_ordinal = owner_ordinals.len() as u32;
            let present_owner_ordinal = *owner_ordinals
                .entry(counter_owner.owner_ptr_hex.clone())
                .or_insert(next_ordinal);
            let exact_generic_sprite_dispatch = list.nodes.iter().any(|node| {
                node.node_ordinal == run.node_ordinal && node.exact_generic_sprite_dispatch
            });
            let draws = run
                .draws
                .iter()
                .map(|draw| PointerFreeCap6Draw {
                    diagnostic_sequence: draw.diagnostic_sequence.clone(),
                    capture_sequence: draw.capture_sequence.clone(),
                    qpc_ticks: draw.qpc_ticks.clone(),
                    producer_rva_hex: draw.producer_rva_hex.clone(),
                    draw_kind: draw.draw_kind,
                    draw_arguments_i64: draw.draw_arguments_i64,
                    shader_bytecode_sha256: draw
                        .shaders
                        .iter()
                        .filter_map(|shader| shader.bytecode_sha256.clone())
                        .collect(),
                    payload_bindings: draw
                        .payload_bindings
                        .iter()
                        .map(|binding| PointerFreeCap6PayloadBinding {
                            class: binding.class,
                            stage: binding.stage,
                            slot: binding.slot,
                            payload_sha256: binding.payload_sha256.clone(),
                            descriptor_sha256: binding.descriptor_sha256.clone(),
                        })
                        .collect(),
                    target: draw.target.clone(),
                    target_descriptor_sha256: draw.target_descriptor_sha256.clone(),
                    capture_complete: draw.capture_complete,
                })
                .collect();
            generic_owner_runs.push(PointerFreeCap6OwnerRun {
                list_invocation_ordinal: run.list_invocation_ordinal.clone(),
                node_ordinal: run.node_ordinal,
                present_owner_ordinal,
                owner_kind: counter_owner.owner_kind,
                graphical_ancestry_edges: counter_owner.graphical_ancestry_edges,
                exact_generic_sprite_dispatch,
                begin_qpc_ticks: run.begin_qpc_ticks.clone(),
                end_qpc_ticks: run.end_qpc_ticks.clone(),
                draws,
                capture_failure_count: run.capture_failure_count,
            });
        }
    }
    let target_descriptors = generic_owner_runs
        .iter()
        .flat_map(|run| run.draws.iter())
        .filter_map(|draw| draw.target_descriptor_sha256.clone())
        .collect::<BTreeSet<_>>();
    let selected_target_descriptor_sha256 = if target_descriptors.len() == 1 {
        target_descriptors.iter().next().cloned()
    } else {
        None
    };
    let final_draw_count = generic_owner_runs
        .iter()
        .map(|run| run.draws.len() as u32)
        .sum();
    let selected_present_sealable = !generic_owner_runs.is_empty()
        && selected_target_descriptor_sha256.is_some()
        && generic_owner_runs.iter().all(|run| {
            run.exact_generic_sprite_dispatch
                && run.capture_failure_count == 0
                && !run.draws.is_empty()
                && run
                    .draws
                    .iter()
                    .all(|draw| draw.capture_complete && draw.target_descriptor_sha256.is_some())
        });
    let body = PointerFreeCap6ReceiptBody {
        authority_executable_sha256: A729_EXECUTABLE_SHA256,
        units_stack_update_rva_hex: "0x018ffe60",
        generic_sprite_render_rva_hex: "0x022e95c0",
        gui_list_consumer_rva_hex: "0x02228c20",
        units_stack_update_job_id:
            "20260721T095716Z-wave6-cap6-a25-updater-generic-hook-abi-890bb2f4",
        units_stack_update_manifest_sha256:
            "41e59a4ae0ed9531ae92c06ca08ab543b8040c015bb64b62ef8aabdb2c1a9f9f",
        hook_sentinel_job_id: "20260721T095228Z-wave6-cap6-a25-hook-sentinels-ae1a60f2",
        hook_sentinel_artifact_sha256:
            "de0e56dbfa8f1a17807ef86af58f828a525d85c0bb6d12856ff8729513f647fc",
        owner_path_live_receipt_job_id: "20260721T093120Z-trace-calls-473d223a",
        owner_path_live_receipt_sha256:
            "724d1aabafc7b22a08dcce78c06a3e4eb98f4da9115717f829c3769d74340d89",
        common_draw_live_receipt_job_id: "20260721T094647Z-trace-calls-4b74a232",
        common_draw_live_receipt_sha256:
            "643fdf05c0424a80873d7e843a00b56811a7a9d597b8eb88209d19b6d7fa1bcf",
        candidate_submit_index: capture.candidate_submit_index.clone(),
        candidate_frame_sequence: capture.candidate_frame_sequence.clone(),
        game_hour_raw: capture.game_hour_raw,
        present_global_order: capture.present_global_order,
        present_qpc_ticks: capture.present_qpc_ticks.clone(),
        qpc_frequency_hz: capture.qpc_frequency_hz.clone(),
        counter_owner_count: owner_ordinals.len() as u32,
        generic_owner_run_count: generic_owner_runs.len() as u32,
        final_draw_count,
        selected_target_descriptor_sha256,
        selected_present_sealable,
        generic_owner_runs,
    };
    let body_sha256 = sha256_json(&body)?;
    Ok(PointerFreeCap6Receipt {
        schema: "hoi4.cap6.counter-final-d3d-pointer-free.v1",
        body,
        body_sha256,
    })
}

fn capture_nodes(list: usize) -> Option<(u32, Vec<DiagnosticGuiNode>, bool)> {
    let data = read_usize(list)?;
    let count = read_i32(list.checked_add(0x0c)?)?;
    let count = usize::try_from(count).ok()?;
    if count > MAX_LIST_NODES || (count != 0 && data == 0) {
        return None;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let text_target = base.checked_add(INSTANT_TEXT_RENDER_RVA)?;
    let generic_sprite_target = base.checked_add(GENERIC_SPRITE_RENDER_RVA)?;
    let retained = count.min(MAX_CAPTURED_NODES_PER_PRESENT);
    let mut nodes = Vec::with_capacity(retained);
    for ordinal in 0..retained {
        let node = read_usize(data.checked_add(ordinal.checked_mul(8)?)?)?;
        let vtable = read_usize(node)?;
        let render_target = read_usize(vtable.checked_add(0x50)?)?;
        let render_rva = render_target
            .checked_sub(base)
            .filter(|rva| *rva <= u32::MAX as usize)
            .map(|rva| format!("0x{rva:08x}"));
        nodes.push(DiagnosticGuiNode {
            node_ordinal: ordinal as u32,
            graphical_node_ptr: node,
            graphical_node_ptr_hex: pointer_hex(node),
            graphical_node_vtable_ptr_hex: pointer_hex(vtable),
            render_vslot_50_target_ptr_hex: pointer_hex(render_target),
            render_vslot_50_target_rva_hex: render_rva,
            exact_instant_text_dispatch: render_target == text_target,
            exact_generic_sprite_dispatch: render_target == generic_sprite_target,
        });
    }
    Some((count as u32, nodes, retained != count))
}

fn capture_counter_owner_paths(stack: usize) {
    const MAX_ACTIVE_COUNTERS_PER_GRID: usize = 4096;
    let Some(grid) = stack.checked_add(0x88).and_then(read_usize) else {
        return;
    };
    let Some(grid_container) = grid.checked_add(0x140).and_then(read_usize) else {
        return;
    };
    let Some(owner_data) = grid.checked_add(0x1A0).and_then(read_usize) else {
        return;
    };
    let Some(owner_count) = grid.checked_add(0x1AC).and_then(read_i32) else {
        return;
    };
    let Ok(owner_count) = usize::try_from(owner_count) else {
        return;
    };
    if owner_count > MAX_ACTIVE_COUNTERS_PER_GRID || (owner_count != 0 && owner_data == 0) {
        return;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let counter_vtable_a = base.saturating_add(0x029F_F890);
    let counter_vtable_b = base.saturating_add(0x029F_F930);
    let mut observed = Vec::with_capacity(owner_count);
    for index in 0..owner_count {
        let Some(owner) = index
            .checked_mul(8)
            .and_then(|offset| owner_data.checked_add(offset))
            .and_then(read_usize)
        else {
            return;
        };
        let Some(owner_vtable) = read_usize(owner) else {
            continue;
        };
        if owner_vtable != counter_vtable_a && owner_vtable != counter_vtable_b {
            continue;
        }
        let Some(gui_root) = owner.checked_add(0x10).and_then(read_usize) else {
            continue;
        };
        let Some(wrapper) = gui_root.checked_add(0x15F0).and_then(read_usize) else {
            continue;
        };
        let Some(paint_root) = gui_root.checked_add(0x15F8).and_then(read_usize) else {
            continue;
        };
        let wrapper_parent = wrapper.checked_add(0x118).and_then(read_usize);
        let paint_root_parent = paint_root.checked_add(0x118).and_then(read_usize);
        if wrapper_parent != Some(grid_container) || paint_root_parent != Some(wrapper) {
            continue;
        }
        let Some(owner_kind) = owner.checked_add(0x180).and_then(read_i32) else {
            continue;
        };
        observed.push(CounterOwnerPath {
            owner,
            gui_root,
            wrapper,
            paint_root,
            grid_container,
            owner_kind,
        });
    }
    let Some(mut paths) = COUNTER_OWNER_PATHS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .ok()
    else {
        return;
    };
    if paths.len().saturating_add(observed.len()) > MAX_COUNTER_OWNER_PATHS {
        paths.clear();
    }
    for path in observed {
        paths.insert(path.paint_root, path);
    }
}

fn resolve_counter_owner_path(layout_object: usize) -> Option<(CounterOwnerPath, u32)> {
    let paths = COUNTER_OWNER_PATHS.get()?.lock().ok()?;
    let mut graphical = layout_object;
    for edge_count in 0..=MAX_GRAPHICAL_ANCESTRY_DEPTH {
        if let Some(path) = paths.get(&graphical) {
            let path = path.clone();
            if validate_counter_owner_path(&path) {
                return Some((path, edge_count as u32));
            }
            return None;
        }
        let parent = graphical.checked_add(0x118).and_then(read_usize)?;
        if parent == 0 || parent == graphical {
            return None;
        }
        graphical = parent;
    }
    None
}

fn validate_counter_owner_path(path: &CounterOwnerPath) -> bool {
    path.owner.checked_add(0x10).and_then(read_usize) == Some(path.gui_root)
        && path.gui_root.checked_add(0x15F0).and_then(read_usize) == Some(path.wrapper)
        && path.gui_root.checked_add(0x15F8).and_then(read_usize) == Some(path.paint_root)
        && path.wrapper.checked_add(0x118).and_then(read_usize) == Some(path.grid_container)
        && path.paint_root.checked_add(0x118).and_then(read_usize) == Some(path.wrapper)
        && path.owner.checked_add(0x180).and_then(read_i32) == Some(path.owner_kind)
}

fn diagnostic_counter_owner(
    path: CounterOwnerPath,
    graphical_node: usize,
    graphical_ancestry_edges: u32,
) -> DiagnosticCounterOwnerJoin {
    let module_base = STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map_or(0, |state| state.module_base);
    let runtime_widget = graphical_node
        .checked_add(0x148)
        .and_then(read_usize)
        .filter(|value| *value != 0);
    let authored_identity = runtime_widget
        .and_then(|widget| runtime_authored_identity(widget, module_base));
    let selected_gui_definition = authored_identity
        .as_ref()
        .map(|identity| identity.selected_gui_definition.clone());
    let authored_child_path = runtime_widget
        .and_then(|widget| runtime_authored_child_path(path.gui_root, widget));
    let graphical_child_path = graphical_child_path(path.paint_root, graphical_node);
    let authored_surface_identity = match path.owner_kind {
        0 => Some("unit_counter"),
        1 => Some("unit_counter_navy"),
        2 => Some("unit_counter_railway_gun"),
        _ => None,
    };
    DiagnosticCounterOwnerJoin {
        owner_ptr_hex: pointer_hex(path.owner),
        gui_root_ptr_hex: pointer_hex(path.gui_root),
        wrapper_ptr_hex: pointer_hex(path.wrapper),
        paint_root_ptr_hex: pointer_hex(path.paint_root),
        grid_container_ptr_hex: pointer_hex(path.grid_container),
        owner_kind: path.owner_kind,
        graphical_ancestry_edges,
        gui_owner_vtable_rva_hex: object_vtable_rva(path.owner, module_base),
        runtime_root_vtable_rva_hex: object_vtable_rva(path.gui_root, module_base),
        graphical_node_vtable_rva_hex: object_vtable_rva(graphical_node, module_base),
        selected_gui_definition,
        authored_surface_identity,
        runtime_root: "mapicons_container",
        runtime_widget_ptr_hex: runtime_widget.map(pointer_hex),
        runtime_widget_vtable_rva_hex: runtime_widget
            .and_then(|widget| object_vtable_rva(widget, module_base)),
        authored_type_ptr_hex: authored_identity
            .as_ref()
            .map(|identity| pointer_hex(identity.authored_type_ptr)),
        authored_type_vtable_rva_hex: authored_identity
            .as_ref()
            .map(|identity| identity.authored_type_vtable_rva_hex.clone()),
        authored_source_path: authored_identity
            .as_ref()
            .map(|identity| identity.source_path.clone()),
        authored_symbol: authored_identity
            .as_ref()
            .map(|identity| identity.symbol.clone()),
        authored_source_line: authored_identity
            .as_ref()
            .map(|identity| identity.source_line),
        authored_child_path,
        graphical_child_path,
        path_validation: "owner+0x10->root;root+0x15f0->wrapper;root+0x15f8->paint_root;parent+0x118",
    }
}

fn diagnostic_owner(
    owner: runtime_text_inputs_live_lane::a729_hook::ObservedOwnerRenderJoin,
) -> DiagnosticOwnerJoin {
    let selected_final_text_digest_valid = owner.selected_final_text_sha256
        == format!("{:x}", Sha256::digest(&owner.selected_final_text_bytes));
    let (source_kind, source_value) = match owner.source {
        RuntimeTextSourceIdentity::AuthoredToken(token) => ("authored_token".into(), token),
        RuntimeTextSourceIdentity::SetterCallerRva(rva) => {
            ("setter_caller_rva".into(), format!("0x{rva:08x}"))
        }
    };
    DiagnosticOwnerJoin {
        owner_ptr_hex: pointer_hex(owner.owner),
        context_id: owner.context_id,
        view_id: owner.view_id,
        element_id: owner.element_id,
        source_kind,
        source_value,
        selected_final_text_sha256: owner.selected_final_text_sha256,
        selected_final_text_hex: hex_bytes(&owner.selected_final_text_bytes),
        selected_final_text_digest_valid,
    }
}

fn finish_list(invocation_ordinal: u64, end_qpc_ticks: u64) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    let Some(mut list) = state.active_lists.remove(&invocation_ordinal) else {
        state.rejected_list_count = state.rejected_list_count.saturating_add(1);
        return;
    };
    list.end_qpc_ticks = end_qpc_ticks.to_string();
    state.completed_lists.push_back(list);
}

fn finish_owner_run(end_qpc_ticks: u64) {
    let Some(mut active) = ACTIVE_OWNER_RUN.with(|run| run.borrow_mut().take()) else {
        reject_owner_run();
        return;
    };
    let owner_ptr = active
        .owner
        .as_ref()
        .and_then(|owner| parse_pointer_hex(&owner.owner_ptr_hex))
        .or_else(|| {
            active
                .counter_owner
                .as_ref()
                .and_then(|owner| parse_pointer_hex(&owner.owner_ptr_hex))
        });
    let candidates = DiagnosticEngineTokenCandidates {
        graphical_node_ptr_decimal: active.layout_object_ptr.to_string(),
        layout_object_ptr_decimal: active.layout_object_ptr.to_string(),
        owner_ptr_decimal: owner_ptr.map(|value| value.to_string()),
        present_scoped_candidate_sha256: scope_candidate_sha256(
            active.list_invocation_ordinal,
            active.node_ordinal,
            active.thread_id,
            active.start_qpc_ticks,
            active.layout_object_ptr,
            owner_ptr,
        ),
        authority_status: "diagnostic_candidate_not_engine_issued",
    };
    let production_operation = active.common_gui_operation;
    let production_static_d3d_captures = mem::take(&mut active.static_d3d_captures);
    let module_base = STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map_or(0, |state| state.module_base);
    let production_runtime_widget_ptr = active
        .layout_object_ptr
        .checked_add(0x148)
        .and_then(read_usize)
        .filter(|pointer| *pointer != 0)
        .unwrap_or(0);
    let production_authored_node = (production_runtime_widget_ptr != 0)
        .then(|| runtime_authored_identity(production_runtime_widget_ptr, module_base))
        .flatten()
        .map(selected_present_authored_node);
    let production_capture_ready = production_operation.is_some()
        && active.capture_failure_count == 0
        && if active.is_generic_sprite_owner {
            !production_static_d3d_captures.is_empty()
        } else {
            active.owner.is_some()
        };
    let run = DiagnosticOwnerRun {
        list_invocation_ordinal: active.list_invocation_ordinal.to_string(),
        node_ordinal: active.node_ordinal,
        thread_id: active.thread_id,
        layout_object_ptr_hex: pointer_hex(active.layout_object_ptr),
        graphics_ptr_hex: pointer_hex(active.graphics_ptr),
        render_context_ptr_hex: pointer_hex(active.render_context_ptr),
        begin_qpc_ticks: active.start_qpc_ticks.to_string(),
        end_qpc_ticks: end_qpc_ticks.to_string(),
        engine_instance_token: active
            .common_gui_operation
            .map(|operation| operation.engine_instance_token.to_string()),
        paint_ordinal: active
            .common_gui_operation
            .map(|operation| operation.paint_ordinal),
        owner: active.owner,
        counter_owner: active.counter_owner,
        transform_f32_bits: active.transform_f32_bits,
        object_rect_candidate_i32: active.object_rect_candidate_i32,
        object_clip_candidate_i32: active.object_clip_candidate_i32,
        engine_token_candidates: candidates,
        draws: active.draws,
        capture_failure_count: active.capture_failure_count,
    };
    let appended = STATE
        .get()
        .and_then(|state| state.lock().ok())
        .is_some_and(|mut state| {
            let appended = state
                .active_lists
                .get_mut(&active.list_invocation_ordinal)
                .is_some_and(|list| {
                    list.owner_runs.push(run);
                    true
                });
            if !appended {
                return false;
            }
            if !production_capture_ready {
                return true;
            }
            let Some((candidate_submit_index, candidate_frame_sequence)) =
                state.current_candidate
            else {
                return false;
            };
            let matching_roots = state
                .selected_present_gui_roots
                .values()
                .filter(|root| {
                    root.candidate_submit_index == candidate_submit_index
                        && root.candidate_frame_sequence == candidate_frame_sequence
                        && graphical_is_descendant_or_self(
                            root.runtime_graphical_subobject_ptr,
                            active.layout_object_ptr,
                        )
                })
                .cloned()
                .collect::<Vec<_>>();
            if matching_roots.len() > 1
                || state.selected_present_gui_draws.len() >= MAX_CAPTURED_NODES_PER_PRESENT
            {
                return false;
            }
            let root = matching_roots.into_iter().next();
            let authored_child_path = root.as_ref().and_then(|root| {
                runtime_authored_child_path(
                    root.runtime_root_ptr,
                    production_runtime_widget_ptr,
                )
            });
            let graphical_child_path = root.as_ref().and_then(|root| {
                graphical_child_path(
                    root.runtime_graphical_subobject_ptr,
                    active.layout_object_ptr,
                )
            });
            if root.is_some()
                && (production_authored_node.is_none()
                    || authored_child_path.is_none()
                    || graphical_child_path.is_none())
            {
                return false;
            }
            state
                .selected_present_gui_draws
                .push(SelectedPresentGuiDrawCapture {
                    candidate_submit_index,
                    candidate_frame_sequence,
                    list_invocation_ordinal: active.list_invocation_ordinal,
                    node_ordinal: active.node_ordinal,
                    owner_begin_qpc_ticks: active.start_qpc_ticks,
                    owner_game_hour_raw: active.game_hour_raw,
                    graphical_node_ptr: active.layout_object_ptr,
                    runtime_widget_ptr: production_runtime_widget_ptr,
                    operation: production_operation.expect("checked above"),
                    root,
                    authored_node: production_authored_node,
                    authored_child_path,
                    graphical_child_path,
                    static_d3d_captures: if active.is_generic_sprite_owner {
                        production_static_d3d_captures
                    } else {
                        Vec::new()
                    },
                });
            true
        });
    if !appended {
        reject_owner_run();
    }
}

fn diagnostic_fingerprint(
    diagnostic_sequence: u64,
    capture_sequence: u64,
    producer_rva: u32,
    draw_call: RawDrawCall,
    captured: &CapturedD3dCore,
) -> DiagnosticDrawFingerprint {
    let qpc_ticks = query_performance_counter().unwrap_or(0);
    let core = &captured.state;
    let shaders = diagnostic_shaders(core, capture_sequence);
    let payload_bindings = diagnostic_payload_bindings(
        core,
        &captured.buffer_snapshots,
        &captured.texture_resource_snapshots,
        &captured.shader_resource_view_snapshots,
    );
    let target = diagnostic_target(core, capture_sequence);
    let target_descriptor_sha256 = target.as_ref().and_then(|target| sha256_json(target).ok());
    let geometry_state_sha256 = hash_geometry_state(core);
    let pipeline_state_sha256 = hash_pipeline_state(core, &shaders);
    let output_state_sha256 = hash_output_state(core, target_descriptor_sha256.as_deref());
    let draw_payload_sha256 = hash_draw_payload(
        draw_call,
        &payload_bindings,
        &captured.buffer_snapshots,
        &captured.texture_resource_snapshots,
    );
    let raw_d3d_fingerprint_sha256 = hash_raw_d3d(
        capture_sequence,
        producer_rva,
        core,
        &geometry_state_sha256,
        &pipeline_state_sha256,
        &output_state_sha256,
        &draw_payload_sha256,
    );
    let required_shader_bytes_present = shaders.iter().any(|shader| {
        shader.stage == "vertex"
            && shader.bytecode_sha256.is_some()
            && shader.creation_sequence.is_some()
    }) && shaders.iter().any(|shader| {
        shader.stage == "pixel"
            && shader.bytecode_sha256.is_some()
            && shader.creation_sequence.is_some()
    });
    let all_payload_bytes_present = !payload_bindings.is_empty()
        && payload_bindings
            .iter()
            .any(|binding| binding.class == "vertex_buffer")
        && payload_bindings
            .iter()
            .any(|binding| binding.class == "shader_resource")
        && payload_bindings.iter().all(|binding| {
            binding.payload_sha256.is_some()
                && (binding.class != "shader_resource" || binding.descriptor_sha256.is_some())
        });
    let capture_complete = target_descriptor_sha256.is_some()
        && required_shader_bytes_present
        && all_payload_bytes_present
        && qpc_ticks != 0;
    DiagnosticDrawFingerprint {
        diagnostic_sequence: diagnostic_sequence.to_string(),
        capture_sequence: capture_sequence.to_string(),
        qpc_ticks: qpc_ticks.to_string(),
        producer_rva_hex: format!("0x{producer_rva:08x}"),
        draw_kind: draw_kind(draw_call),
        draw_arguments_i64: draw_arguments(draw_call),
        render_context_ptr_hex: pointer_hex(core.render_context_ptr),
        shader_program_ptr_hex: pointer_hex(core.shader_program_ptr),
        shaders,
        payload_bindings,
        target,
        target_descriptor_sha256,
        geometry_state_sha256,
        pipeline_state_sha256,
        output_state_sha256,
        draw_payload_sha256,
        raw_d3d_fingerprint_sha256,
        capture_complete,
    }
}

fn diagnostic_shaders(
    core: &NeutralD3dDrawState,
    capture_sequence: u64,
) -> Vec<DiagnosticShaderBinding> {
    let candidates = [
        (RawShaderStage::Vertex, Some(core.vertex_shader_ptr)),
        (RawShaderStage::Pixel, Some(core.pixel_shader_ptr)),
        (RawShaderStage::Geometry, core.geometry_shader_ptr),
        (RawShaderStage::Hull, core.hull_shader_ptr),
        (RawShaderStage::Domain, core.domain_shader_ptr),
        (RawShaderStage::Compute, core.compute_shader_ptr),
    ];
    let creations = super::STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map(|state| state.shader_creation_observations.clone())
        .unwrap_or_default();
    candidates
        .into_iter()
        .filter_map(|(stage, pointer)| {
            let pointer = pointer?;
            let creation = creations
                .iter()
                .filter(|creation| {
                    creation.shader_ptr == pointer
                        && creation.stage == stage
                        && creation.creation_sequence <= capture_sequence
                })
                .max_by_key(|creation| creation.creation_sequence);
            Some(DiagnosticShaderBinding {
                stage: stage_name(stage),
                shader_ptr_hex: pointer_hex(pointer),
                bytecode_sha256: creation
                    .map(|creation| format!("{:x}", Sha256::digest(&creation.bytecode))),
                creation_sequence: creation.map(|creation| creation.creation_sequence.to_string()),
            })
        })
        .collect()
}

fn diagnostic_payload_bindings(
    core: &NeutralD3dDrawState,
    buffers: &[RawBufferSnapshot],
    textures: &[RawTextureResourceSnapshot],
    views: &[RawShaderResourceViewSnapshot],
) -> Vec<DiagnosticPayloadBinding> {
    let mut output = Vec::new();
    for texture in &core.ordered_texture_objects {
        let view = views.iter().find(|view| {
            view.shader_resource_view_ptr == texture.shader_resource_view_ptr
                && view.resource_ptr == texture.resource_ptr
        });
        let resource = textures
            .iter()
            .find(|resource| resource.resource_ptr == texture.resource_ptr);
        output.push(DiagnosticPayloadBinding {
            class: "shader_resource",
            stage: Some(stage_name(texture.stage)),
            slot: u32::from(texture.slot),
            object_ptr_hex: pointer_hex(texture.resource_ptr),
            view_ptr_hex: Some(pointer_hex(texture.shader_resource_view_ptr)),
            payload_sha256: resource.map(hash_texture_payload),
            descriptor_sha256: view.map(|view| hash_view_desc(&view.desc)),
        });
    }
    for constant in &core.ordered_constant_buffer_objects {
        let buffer = buffers
            .iter()
            .find(|buffer| buffer.buffer_ptr == constant.object_ptr);
        output.push(DiagnosticPayloadBinding {
            class: "constant_buffer",
            stage: Some(stage_name(constant.stage)),
            slot: u32::from(constant.slot),
            object_ptr_hex: pointer_hex(constant.object_ptr),
            view_ptr_hex: None,
            payload_sha256: buffer.map(|buffer| buffer.payload.captured_sha256.clone()),
            descriptor_sha256: None,
        });
    }
    for vertex in &core.ordered_vertex_buffer_objects {
        let buffer = buffers
            .iter()
            .find(|buffer| buffer.buffer_ptr == vertex.buffer_ptr);
        output.push(DiagnosticPayloadBinding {
            class: "vertex_buffer",
            stage: None,
            slot: vertex.input_slot,
            object_ptr_hex: pointer_hex(vertex.buffer_ptr),
            view_ptr_hex: None,
            payload_sha256: buffer.map(|buffer| buffer.payload.captured_sha256.clone()),
            descriptor_sha256: None,
        });
    }
    if let Some(index) = &core.index_buffer {
        let buffer = buffers
            .iter()
            .find(|buffer| buffer.buffer_ptr == index.buffer_ptr);
        output.push(DiagnosticPayloadBinding {
            class: "index_buffer",
            stage: None,
            slot: 0,
            object_ptr_hex: pointer_hex(index.buffer_ptr),
            view_ptr_hex: None,
            payload_sha256: buffer.map(|buffer| buffer.payload.captured_sha256.clone()),
            descriptor_sha256: None,
        });
    }
    output
}

fn diagnostic_target(
    core: &NeutralD3dDrawState,
    capture_sequence: u64,
) -> Option<RenderTargetDesc> {
    if core.ordered_color_target_view_ptrs.len() != 1 {
        return None;
    }
    let view = core.ordered_color_target_view_ptrs[0];
    let mut registration = super::STATE
        .get()?
        .lock()
        .ok()?
        .registered_render_targets
        .get(&view)?
        .clone();
    registration.capture_sequence = capture_sequence;
    let receipt = registration.materialize().ok()?;
    let prepared_target = match receipt.kind {
        RawRegisteredRenderTargetKind::SwapchainBackBuffer { buffer_index: 0 } => {
            PreparedRenderTarget::SwapchainBackBuffer
        }
        RawRegisteredRenderTargetKind::OpaqueRenderTarget { .. } => {
            PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: receipt.attachment_ordinal,
            }
        }
        _ => return None,
    };
    let mip_level = match receipt.rtv_desc.view_dimension_u32 {
        4 | 6 => receipt.rtv_desc.union_words_u32[0],
        5 => 0,
        _ => return None,
    };
    Some(RenderTargetDesc {
        prepared_target,
        width: receipt.resource_desc.width.checked_shr(mip_level).unwrap_or(0).max(1),
        height: receipt
            .resource_desc
            .height
            .checked_shr(mip_level)
            .unwrap_or(0)
            .max(1),
        dxgi_format_u32: receipt.rtv_desc.dxgi_format_u32,
        mip_level,
        sample_count: receipt.resource_desc.sample_count,
        sample_quality: receipt.resource_desc.sample_quality,
        persistent_content: None,
        exact_resource: None,
    })
}

fn regrade_owner_runs(list: &DiagnosticGuiList) -> Vec<DiagnosticOwnerRunRegrade> {
    list.owner_runs
        .iter()
        .map(|run| {
            let node = list
                .nodes
                .iter()
                .find(|node| node.node_ordinal == run.node_ordinal);
            let target_hashes = run
                .draws
                .iter()
                .filter_map(|draw| draw.target_descriptor_sha256.as_deref())
                .collect::<BTreeSet<_>>();
            let exact_vslot_dispatch = node.is_some_and(|node| {
                node.exact_instant_text_dispatch || node.exact_generic_sprite_dispatch
            });
            let list_and_owner_thread_match = run.thread_id == list.thread_id;
            let list_and_owner_graphics_match = run.graphics_ptr_hex == list.graphics_ptr_hex;
            let layout_equals_graphical_node = node.is_some_and(|node| {
                node.graphical_node_ptr_hex == run.layout_object_ptr_hex
            });
            let registered_owner_resolved = run.owner.is_some() || run.counter_owner.is_some();
            let text_owner_path_present = run.owner.as_ref().is_some_and(|owner| {
                let source_valid = match owner.source_kind.as_str() {
                    "authored_token" => !owner.source_value.trim().is_empty(),
                    "setter_caller_rva" => u32::from_str_radix(
                        owner.source_value.strip_prefix("0x").unwrap_or(""),
                        16,
                    )
                    .is_ok_and(|rva| rva != 0),
                    _ => false,
                };
                !owner.context_id.trim().is_empty()
                    && !owner.view_id.trim().is_empty()
                    && !owner.element_id.trim().is_empty()
                    && source_valid
                    && owner.selected_final_text_sha256.len() == 64
                    && owner.selected_final_text_digest_valid
            });
            let counter_owner_path_present = run.counter_owner.as_ref().is_some_and(|owner| {
                owner.owner_kind >= 0
                    && owner.graphical_ancestry_edges <= MAX_GRAPHICAL_ANCESTRY_DEPTH as u32
                    && owner.path_validation
                        == "owner+0x10->root;root+0x15f0->wrapper;root+0x15f8->paint_root;parent+0x118"
            });
            let registered_owner_path_candidate_present =
                text_owner_path_present || counter_owner_path_present;
            let diagnostic_token_candidate_present =
                run.engine_token_candidates.present_scoped_candidate_sha256.len() == 64
                    && run.engine_token_candidates.authority_status
                        == "diagnostic_candidate_not_engine_issued";
            let list_begin = list.begin_qpc_ticks.parse::<u64>().ok();
            let list_end = list.end_qpc_ticks.parse::<u64>().ok();
            let run_begin = run.begin_qpc_ticks.parse::<u64>().ok();
            let run_end = run.end_qpc_ticks.parse::<u64>().ok();
            let draw_qpcs = run
                .draws
                .iter()
                .map(|draw| draw.qpc_ticks.parse::<u64>())
                .collect::<Result<Vec<_>, _>>()
                .ok();
            let qpc_order_complete = match (
                list_begin,
                list_end,
                run_begin,
                run_end,
                draw_qpcs.as_deref(),
            ) {
                (Some(list_begin), Some(list_end), Some(run_begin), Some(run_end), Some(draws)) => {
                    list_begin != 0
                        && list_begin <= run_begin
                        && run_begin <= run_end
                        && run_end <= list_end
                        && draws.iter().all(|qpc| *qpc >= run_begin && *qpc <= run_end)
                        && draws.windows(2).all(|pair| pair[0] <= pair[1])
                }
                _ => false,
            };
            let all_draw_fingerprints_complete = !run.draws.is_empty()
                && run.draws.iter().all(|draw| draw.capture_complete)
                && run.capture_failure_count == 0;
            let mut hasher = Sha256::new();
            hasher.update(b"hoi4-a25-gui-owner-run-candidate.v1\0");
            hasher.update(list.list_invocation_ordinal.as_bytes());
            hasher.update(run.node_ordinal.to_le_bytes());
            hasher.update(run.thread_id.to_le_bytes());
            hasher.update(run.layout_object_ptr_hex.as_bytes());
            hasher.update(run.graphics_ptr_hex.as_bytes());
            hasher.update(run.render_context_ptr_hex.as_bytes());
            hasher.update(
                run.engine_token_candidates
                    .present_scoped_candidate_sha256
                    .as_bytes(),
            );
            if let Some(owner) = &run.owner {
                hasher.update(owner.owner_ptr_hex.as_bytes());
                hasher.update(owner.context_id.as_bytes());
                hasher.update(owner.view_id.as_bytes());
                hasher.update(owner.element_id.as_bytes());
                hasher.update(owner.source_kind.as_bytes());
                hasher.update(owner.source_value.as_bytes());
                hasher.update(owner.selected_final_text_sha256.as_bytes());
                hasher.update(owner.selected_final_text_hex.as_bytes());
                hasher.update([u8::from(owner.selected_final_text_digest_valid)]);
            }
            if let Some(owner) = &run.counter_owner {
                hasher.update(owner.owner_ptr_hex.as_bytes());
                hasher.update(owner.gui_root_ptr_hex.as_bytes());
                hasher.update(owner.wrapper_ptr_hex.as_bytes());
                hasher.update(owner.paint_root_ptr_hex.as_bytes());
                hasher.update(owner.grid_container_ptr_hex.as_bytes());
                hasher.update(owner.owner_kind.to_le_bytes());
                hasher.update(owner.graphical_ancestry_edges.to_le_bytes());
            }
            for draw in &run.draws {
                hasher.update(draw.raw_d3d_fingerprint_sha256.as_bytes());
            }
            let candidate_join_sha256 = format!("{:x}", hasher.finalize());
            let mut owner_path_token = Sha256::new();
            owner_path_token.update(b"hoi4-a25-owner-path-token-candidate.v1\0");
            owner_path_token.update(list.list_invocation_ordinal.as_bytes());
            owner_path_token.update(run.node_ordinal.to_le_bytes());
            owner_path_token.update(run.thread_id.to_le_bytes());
            owner_path_token.update(
                run.engine_token_candidates
                    .present_scoped_candidate_sha256
                    .as_bytes(),
            );
            if let Some(owner) = &run.owner {
                owner_path_token.update(owner.context_id.as_bytes());
                owner_path_token.update(owner.view_id.as_bytes());
                owner_path_token.update(owner.element_id.as_bytes());
                owner_path_token.update(owner.source_kind.as_bytes());
                owner_path_token.update(owner.source_value.as_bytes());
            }
            if let Some(owner) = &run.counter_owner {
                owner_path_token.update(owner.owner_ptr_hex.as_bytes());
                owner_path_token.update(owner.owner_kind.to_le_bytes());
                owner_path_token.update(owner.graphical_ancestry_edges.to_le_bytes());
            }
            DiagnosticOwnerRunRegrade {
                list_invocation_ordinal: list.list_invocation_ordinal.clone(),
                node_ordinal: run.node_ordinal,
                exact_vslot_dispatch,
                list_and_owner_thread_match,
                list_and_owner_graphics_match,
                layout_equals_graphical_node,
                registered_owner_resolved,
                registered_owner_path_candidate_present,
                diagnostic_token_candidate_present,
                qpc_order_complete,
                draw_count: run.draws.len() as u32,
                one_render_target_descriptor: !run.draws.is_empty()
                    && target_hashes.len() == 1
                    && run
                        .draws
                        .iter()
                        .all(|draw| draw.target_descriptor_sha256.is_some()),
                all_draw_fingerprints_complete,
                candidate_join_sha256,
                owner_path_token_candidate_sha256: format!(
                    "{:x}",
                    owner_path_token.finalize()
                ),
                missing_production_authority: vec![
                    "engine_issued_instance_token",
                    "selected_effective_vfs_gui_definition",
                    "authored_surface_and_complete_child_path",
                    "reviewed_live_control_observation_receipt",
                ],
                promotion_eligible: false,
            }
        })
        .collect()
}

fn hash_geometry_state(core: &NeutralD3dDrawState) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a25-gui-geometry-diagnostic.v1\0");
    hasher.update(core.primitive_topology_u32.to_le_bytes());
    hasher.update(pointer_bytes(core.input_layout_ptr.unwrap_or(0)));
    for vertex in &core.ordered_vertex_buffer_objects {
        hasher.update(vertex.input_slot.to_le_bytes());
        hasher.update(pointer_bytes(vertex.buffer_ptr));
        hasher.update(vertex.stride_bytes.to_le_bytes());
        hasher.update(vertex.byte_offset.to_le_bytes());
    }
    if let Some(index) = &core.index_buffer {
        hasher.update(pointer_bytes(index.buffer_ptr));
        hasher.update(index.dxgi_format_u32.to_le_bytes());
        hasher.update(index.byte_offset.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn hash_pipeline_state(
    core: &NeutralD3dDrawState,
    shaders: &[DiagnosticShaderBinding],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a25-gui-pipeline-diagnostic.v1\0");
    hasher.update(pointer_bytes(core.shader_program_ptr));
    hasher.update(pointer_bytes(core.blend_state_ptr.unwrap_or(0)));
    for value in core.blend_factor_rgba_f32_bits {
        hasher.update(value.to_le_bytes());
    }
    hasher.update(core.sample_mask_u32.to_le_bytes());
    hasher.update(pointer_bytes(core.depth_stencil_state_ptr.unwrap_or(0)));
    hasher.update(core.stencil_reference_u32.to_le_bytes());
    hasher.update(pointer_bytes(core.rasterizer_state_ptr.unwrap_or(0)));
    for shader in shaders {
        hasher.update(shader.stage.as_bytes());
        hasher.update(shader.shader_ptr_hex.as_bytes());
        if let Some(digest) = &shader.bytecode_sha256 {
            hasher.update(digest.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

fn hash_output_state(
    core: &NeutralD3dDrawState,
    target_descriptor_sha256: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a25-gui-output-diagnostic.v1\0");
    for view in &core.ordered_color_target_view_ptrs {
        hasher.update(pointer_bytes(*view));
    }
    hasher.update(pointer_bytes(core.depth_stencil_view_ptr.unwrap_or(0)));
    for viewport in &core.ordered_viewports_f32_bits {
        for value in viewport {
            hasher.update(value.to_le_bytes());
        }
    }
    for scissor in &core.ordered_scissor_rects_i32 {
        for value in scissor {
            hasher.update(value.to_le_bytes());
        }
    }
    if let Some(digest) = target_descriptor_sha256 {
        hasher.update(digest.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn hash_draw_payload(
    draw_call: RawDrawCall,
    bindings: &[DiagnosticPayloadBinding],
    buffers: &[RawBufferSnapshot],
    textures: &[RawTextureResourceSnapshot],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a25-gui-draw-payload-diagnostic.v1\0");
    hasher.update(draw_kind(draw_call).as_bytes());
    for value in draw_arguments(draw_call) {
        hasher.update(value.to_le_bytes());
    }
    for binding in bindings {
        hasher.update(binding.class.as_bytes());
        if let Some(stage) = binding.stage {
            hasher.update(stage.as_bytes());
        }
        hasher.update(binding.slot.to_le_bytes());
        hasher.update(binding.object_ptr_hex.as_bytes());
        if let Some(digest) = &binding.payload_sha256 {
            hasher.update(digest.as_bytes());
        }
    }
    for buffer in buffers {
        hasher.update(pointer_bytes(buffer.buffer_ptr));
        hasher.update(buffer.payload.captured_sha256.as_bytes());
    }
    for texture in textures {
        hasher.update(pointer_bytes(texture.resource_ptr));
        hasher.update(hash_texture_payload(texture).as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn hash_raw_d3d(
    capture_sequence: u64,
    producer_rva: u32,
    core: &NeutralD3dDrawState,
    geometry: &str,
    pipeline: &str,
    output: &str,
    payload: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a25-gui-raw-d3d-fingerprint.v1\0");
    hasher.update(capture_sequence.to_le_bytes());
    hasher.update(producer_rva.to_le_bytes());
    hasher.update(pointer_bytes(core.render_context_ptr));
    hasher.update(pointer_bytes(core.device_context_ptr));
    hasher.update(geometry.as_bytes());
    hasher.update(pipeline.as_bytes());
    hasher.update(output.as_bytes());
    hasher.update(payload.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn hash_texture_payload(texture: &RawTextureResourceSnapshot) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a25-gui-texture-payload.v1\0");
    hasher.update(texture.width.to_le_bytes());
    hasher.update(texture.height.to_le_bytes());
    hasher.update(texture.depth.to_le_bytes());
    hasher.update(texture.mip_levels.to_le_bytes());
    hasher.update(texture.array_size.to_le_bytes());
    hasher.update(texture.dxgi_format_u32.to_le_bytes());
    for subresource in &texture.ordered_subresources {
        hasher.update(subresource.subresource_index.to_le_bytes());
        hasher.update(subresource.mip_level.to_le_bytes());
        hasher.update(subresource.array_slice.to_le_bytes());
        hasher.update(subresource.row_pitch.to_le_bytes());
        hasher.update(subresource.slice_pitch.to_le_bytes());
        hasher.update(subresource.payload.captured_sha256.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn hash_view_desc(desc: &RawResourceViewDesc) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a25-gui-view-desc.v1\0");
    hasher.update(desc.dxgi_format_u32.to_le_bytes());
    hasher.update(desc.view_dimension_u32.to_le_bytes());
    hasher.update(desc.flags_u32.to_le_bytes());
    for value in desc.union_words_u32 {
        hasher.update(value.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn scope_candidate_sha256(
    list_invocation_ordinal: u64,
    node_ordinal: u32,
    thread_id: u32,
    begin_qpc_ticks: u64,
    layout_object: usize,
    owner: Option<usize>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a25-gui-engine-token-candidate.v1\0");
    hasher.update(list_invocation_ordinal.to_le_bytes());
    hasher.update(node_ordinal.to_le_bytes());
    hasher.update(thread_id.to_le_bytes());
    hasher.update(begin_qpc_ticks.to_le_bytes());
    hasher.update(pointer_bytes(layout_object));
    hasher.update(pointer_bytes(owner.unwrap_or(0)));
    format!("{:x}", hasher.finalize())
}

fn draw_kind(draw: RawDrawCall) -> &'static str {
    match draw {
        RawDrawCall::Draw { .. } => "draw",
        RawDrawCall::DrawIndexed { .. } => "draw_indexed",
        RawDrawCall::DrawInstanced { .. } => "draw_instanced",
        RawDrawCall::DrawIndexedInstanced { .. } => "draw_indexed_instanced",
    }
}

fn raw_static_draw_call(draw: RawDrawCall) -> RawStaticDrawCall {
    match draw {
        RawDrawCall::Draw {
            vertex_count,
            start_vertex,
        } => RawStaticDrawCall::Draw {
            vertex_count,
            start_vertex,
        },
        RawDrawCall::DrawIndexed {
            index_count,
            start_index,
            base_vertex,
        } => RawStaticDrawCall::DrawIndexed {
            index_count,
            start_index,
            base_vertex,
        },
        RawDrawCall::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        } => RawStaticDrawCall::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        },
        RawDrawCall::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        } => RawStaticDrawCall::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        },
    }
}

fn draw_arguments(draw: RawDrawCall) -> [i64; 5] {
    match draw {
        RawDrawCall::Draw {
            vertex_count,
            start_vertex,
        } => [i64::from(vertex_count), i64::from(start_vertex), 0, 0, 0],
        RawDrawCall::DrawIndexed {
            index_count,
            start_index,
            base_vertex,
        } => [
            i64::from(index_count),
            i64::from(start_index),
            i64::from(base_vertex),
            0,
            0,
        ],
        RawDrawCall::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        } => [
            i64::from(vertex_count_per_instance),
            i64::from(instance_count),
            i64::from(start_vertex),
            i64::from(start_instance),
            0,
        ],
        RawDrawCall::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        } => [
            i64::from(index_count_per_instance),
            i64::from(instance_count),
            i64::from(start_index),
            i64::from(base_vertex),
            i64::from(start_instance),
        ],
    }
}

fn stage_name(stage: RawShaderStage) -> &'static str {
    match stage {
        RawShaderStage::Vertex => "vertex",
        RawShaderStage::Pixel => "pixel",
        RawShaderStage::Geometry => "geometry",
        RawShaderStage::Hull => "hull",
        RawShaderStage::Domain => "domain",
        RawShaderStage::Compute => "compute",
    }
}

fn object_vtable_rva(object: usize, module_base: usize) -> Option<String> {
    if object == 0 || module_base == 0 {
        return None;
    }
    let vtable = read_usize(object)?;
    let rva = u32::try_from(vtable.checked_sub(module_base)?).ok()?;
    (rva != 0).then(|| format!("0x{rva:08x}"))
}

fn runtime_authored_identity(
    runtime_widget: usize,
    module_base: usize,
) -> Option<RuntimeAuthoredIdentity> {
    let authored_type_ptr = runtime_widget.checked_add(0x60).and_then(read_usize)?;
    if authored_type_ptr == 0 {
        return None;
    }
    let authored_type_vtable = read_usize(authored_type_ptr)?;
    let getter_target = authored_type_vtable
        .checked_add(0x48)
        .and_then(read_usize)?;
    let image_size = loaded_pe_image_size(module_base)?;
    if !export_target_within_image(module_base, image_size, getter_target, 1) {
        return None;
    }
    let getter: unsafe extern "system" fn(usize) -> usize =
        unsafe { mem::transmute(getter_target) };
    let symbol_string = unsafe { getter(authored_type_ptr) };
    let symbol = read_msvc_string(symbol_string, 4_096)?;
    if symbol.trim() != symbol
        || symbol.is_empty()
        || symbol.contains('/')
        || symbol.contains('\\')
        || symbol.bytes().any(|byte| byte.is_ascii_control())
    {
        return None;
    }
    let raw_source_path = read_msvc_string(authored_type_ptr.checked_add(0x48)?, 4_096)?;
    let source_path = normalize_game_relative_path(&raw_source_path).ok()?;
    if !source_path.starts_with("interface/")
        || !source_path.ends_with(".gui")
    {
        return None;
    }
    let source_line = read_u32(authored_type_ptr.checked_add(0xe0)?)?;
    if source_line == 0 {
        return None;
    }
    let selected_gui_definition = STATE
        .get()
        .and_then(|state| state.lock().ok())
        .and_then(|state| state.gui_definitions.get(&source_path).cloned())?;
    Some(RuntimeAuthoredIdentity {
        authored_type_ptr,
        runtime_widget_vtable_rva_hex: object_vtable_rva(runtime_widget, module_base)?,
        authored_type_vtable_rva_hex: object_vtable_rva(authored_type_ptr, module_base)?,
        selected_gui_definition,
        source_path,
        symbol,
        source_line,
    })
}

fn selected_present_authored_node(
    identity: RuntimeAuthoredIdentity,
) -> SelectedPresentGuiAuthoredNodeIdentity {
    SelectedPresentGuiAuthoredNodeIdentity {
        runtime_widget_vtable_rva_hex: identity.runtime_widget_vtable_rva_hex,
        authored_type_vtable_rva_hex: identity.authored_type_vtable_rva_hex,
        selected_gui_definition: identity.selected_gui_definition,
        authored_source_path: identity.source_path,
        authored_symbol: identity.symbol,
        authored_source_line: identity.source_line,
    }
}

fn runtime_authored_child_path(runtime_root: usize, runtime_widget: usize) -> Option<Vec<u32>> {
    child_path_through_parent_vectors(
        runtime_root,
        runtime_widget,
        0x108,
        0x10d8,
        0x10e4,
    )
}

fn graphical_child_path(graphical_root: usize, graphical_node: usize) -> Option<Vec<u32>> {
    child_path_through_parent_vectors(
        graphical_root,
        graphical_node,
        0x118,
        0x0d0,
        0x0dc,
    )
}

fn graphical_is_descendant_or_self(graphical_root: usize, graphical_node: usize) -> bool {
    if graphical_root == 0 || graphical_node == 0 {
        return false;
    }
    let mut current = graphical_node;
    for _ in 0..=MAX_GRAPHICAL_ANCESTRY_DEPTH {
        if current == graphical_root {
            return true;
        }
        let Some(parent) = current.checked_add(0x118).and_then(read_usize) else {
            return false;
        };
        if parent == 0 || parent == current {
            return false;
        }
        current = parent;
    }
    false
}

fn derive_automatic_global_root(
    capture: &mut SelectedPresentGuiDrawCapture,
    module_base: usize,
) -> Result<(), String> {
    let runtime_root = ultimate_parent(capture.runtime_widget_ptr, 0x108)
        .ok_or("selected-Present GUI runtime widget has no finite root")?;
    let graphical_root = ultimate_parent(capture.graphical_node_ptr, 0x118)
        .ok_or("selected-Present GUI graphical node has no finite root")?;
    let root_identity = runtime_authored_identity(runtime_root, module_base)
        .ok_or("selected-Present GUI runtime root has no authenticated authored identity")?;
    let authored_child_path = runtime_authored_child_path(runtime_root, capture.runtime_widget_ptr)
        .ok_or("selected-Present GUI runtime root cannot derive its authored child path")?;
    let graphical_child_path = graphical_child_path(graphical_root, capture.graphical_node_ptr)
        .ok_or("selected-Present GUI graphical root cannot derive its child path")?;
    capture.root = Some(SelectedPresentGuiRootRegistration {
        candidate_submit_index: capture.candidate_submit_index,
        candidate_frame_sequence: capture.candidate_frame_sequence,
        authored_surface_identity: root_identity.symbol.clone(),
        runtime_root_identity: root_identity.symbol,
        effective_interface_winner: root_identity.source_path,
        callback_qpc_ticks: capture.owner_begin_qpc_ticks,
        game_hour_raw: capture
            .owner_game_hour_raw
            .ok_or("selected-Present GUI owner has no exact game hour")?,
        controller_ptr: runtime_root,
        runtime_root_ptr: runtime_root,
        runtime_graphical_subobject_ptr: graphical_root,
    });
    capture.authored_child_path = Some(authored_child_path);
    capture.graphical_child_path = Some(graphical_child_path);
    Ok(())
}

fn ultimate_parent(mut current: usize, parent_offset: usize) -> Option<usize> {
    if current == 0 {
        return None;
    }
    for _ in 0..MAX_GRAPHICAL_ANCESTRY_DEPTH {
        let parent = read_usize(current.checked_add(parent_offset)?)?;
        if parent == 0 {
            return Some(current);
        }
        if parent == current {
            return None;
        }
        current = parent;
    }
    None
}

fn child_path_through_parent_vectors(
    root: usize,
    leaf: usize,
    parent_offset: usize,
    children_offset: usize,
    child_count_offset: usize,
) -> Option<Vec<u32>> {
    if root == 0 || leaf == 0 {
        return None;
    }
    let mut reverse_path = Vec::new();
    let mut current = leaf;
    for _ in 0..=MAX_GRAPHICAL_ANCESTRY_DEPTH {
        if current == root {
            reverse_path.reverse();
            return Some(reverse_path);
        }
        let parent = read_usize(current.checked_add(parent_offset)?)?;
        if parent == 0 || parent == current {
            return None;
        }
        let children = read_usize(parent.checked_add(children_offset)?)?;
        let count = read_i32(parent.checked_add(child_count_offset)?)?;
        if children == 0 || count <= 0 || count as usize > MAX_LIST_NODES {
            return None;
        }
        let mut matched = None;
        for index in 0..count as usize {
            let slot = children.checked_add(index.checked_mul(mem::size_of::<usize>())?)?;
            if read_usize(slot)? == current {
                if matched.replace(index).is_some() {
                    return None;
                }
            }
        }
        reverse_path.push(u32::try_from(matched?).ok()?);
        current = parent;
    }
    None
}

fn read_u32x16(address: usize) -> Option<[u32; 16]> {
    let mut bytes = [0u8; 64];
    read_exact(address, &mut bytes)?;
    let mut output = [0u32; 16];
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        output[index] = u32::from_le_bytes(chunk.try_into().ok()?);
    }
    Some(output)
}

fn read_i32x4(address: usize) -> Option<[i32; 4]> {
    Some([
        read_i32(address)?,
        read_i32(address.checked_add(4)?)?,
        read_i32(address.checked_add(8)?)?,
        read_i32(address.checked_add(12)?)?,
    ])
}

fn read_object_rect_candidate(layout_object: usize) -> Option<[i32; 4]> {
    Some([
        read_i32(layout_object.checked_add(0x210)?)?,
        read_i32(layout_object.checked_add(0x214)?)?,
        read_i32(layout_object.checked_add(0x220)?)?,
        read_i32(layout_object.checked_add(0x224)?)?,
    ])
}

fn pointer_hex(pointer: usize) -> String {
    format!("0x{pointer:016x}")
}

fn pointer_bytes(pointer: usize) -> [u8; 8] {
    (pointer as u64).to_le_bytes()
}

fn parse_pointer_hex(value: &str) -> Option<usize> {
    usize::from_str_radix(value.strip_prefix("0x")?, 16).ok()
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing into String cannot fail");
    }
    output
}

fn sha256_json(value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn persist_create_new_or_equal(
    path: &Path,
    capture: &impl Serialize,
) -> Result<(), String> {
    use std::io::Write;
    let mut bytes = serde_json::to_vec_pretty(capture).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    if path.exists() {
        if std::fs::read(path).map_err(|error| error.to_string())? == bytes {
            return Ok(());
        }
        return Err(format!(
            "A25 GUI diagnostic path already exists with different bytes: {}",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("A25 GUI diagnostic path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("A25 GUI diagnostic path has no file name: {}", path.display()))?;
    let mut temporary = None;
    for attempt in 0..16u32 {
        let candidate = parent.join(format!(
            ".{file_name}.tmp-{}-{}-{attempt}",
            std::process::id(),
            query_performance_counter().unwrap_or(0)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                file.write_all(&bytes).map_err(|error| error.to_string())?;
                file.sync_all().map_err(|error| error.to_string())?;
                temporary = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    let temporary = temporary.ok_or("A25 GUI diagnostic temporary-name space exhausted")?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let removal = std::fs::remove_file(&temporary);
            if std::fs::read(path).map_err(|read_error| read_error.to_string())? == bytes {
                removal.map_err(|remove_error| remove_error.to_string())?;
                Ok(())
            } else {
                let _ = removal;
                Err(format!(
                    "A25 GUI diagnostic path raced with different bytes: {}",
                    path.display()
                ))
            }
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error.to_string())
        }
    }
}

fn reject_list() {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.rejected_list_count = state.rejected_list_count.saturating_add(1);
    }
}

fn reject_owner_run() {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.rejected_owner_run_count = state.rejected_owner_run_count.saturating_add(1);
    }
}

fn reject_draw() {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.rejected_draw_count = state.rejected_draw_count.saturating_add(1);
    }
}

fn poison_active_owner_run() {
    ACTIVE_OWNER_RUN.with(|run| {
        if let Some(run) = run.borrow_mut().as_mut() {
            run.capture_failure_count = run.capture_failure_count.saturating_add(1);
        }
    });
}
