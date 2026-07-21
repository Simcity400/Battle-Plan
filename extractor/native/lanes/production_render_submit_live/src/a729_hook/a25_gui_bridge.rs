//! Evidence-only A25 GUI list/owner/final-D3D bridge.
//!
//! This module deliberately does not call the GUI-paint production admission
//! APIs. It records pointer-bearing diagnostic observations behind an explicit
//! evidence-output environment variable so one live run can recover the
//! missing owner/path/token law without turning a candidate into authority.

use super::*;
use portable_render_stream::{PreparedRenderTarget, RenderTargetDesc};
use runtime_text_inputs_live_lane::RuntimeTextSourceIdentity;
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub const EVIDENCE_OUT_DIR_ENV: &str = "BATTLE_PLAN_A25_GUI_BRIDGE_EVIDENCE_OUT_DIR";
pub const GUI_LIST_CONSUMER_RVA: usize = 0x0222_8C20;
pub const INSTANT_TEXT_RENDER_RVA: usize = 0x022D_8350;
pub const PRODUCT_PROMOTION_ENABLED: bool = false;

const GUI_LIST_CONSUMER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xEC, 0x58,
    0x48,
];
const INSTANT_TEXT_RENDER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x6C, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20,
    0x57,
];
const MAX_LIST_NODES: usize = 1_048_576;
const MAX_CAPTURED_NODES_PER_PRESENT: usize = 262_144;
const MAX_LISTS_PER_PRESENT: usize = 16_384;
const MAX_DRAWS_PER_OWNER_RUN: usize = 65_536;

static ORIGINAL_GUI_LIST_CONSUMER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_INSTANT_TEXT_RENDER: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_LIST_INVOCATION: AtomicU64 = AtomicU64::new(1);
static NEXT_DIAGNOSTIC_DRAW_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static STATE: OnceLock<Mutex<BridgeState>> = OnceLock::new();
static TARGETS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();

thread_local! {
    static LIST_STACK: RefCell<Vec<ActiveListTls>> = const { RefCell::new(Vec::new()) };
    static ACTIVE_OWNER_RUN: RefCell<Option<ActiveOwnerRun>> = const { RefCell::new(None) };
}

type GuiListConsumerFn = unsafe extern "system" fn(usize, usize);
type InstantTextRenderFn = unsafe extern "system" fn(usize, usize, usize, usize);

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
    owner: Option<DiagnosticOwnerJoin>,
    transform_f32_bits: Option<[u32; 16]>,
    object_rect_candidate_i32: Option<[i32; 4]>,
    object_clip_candidate_i32: Option<[i32; 4]>,
    draws: Vec<DiagnosticDrawFingerprint>,
    capture_failure_count: u32,
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
struct DiagnosticOwnerRun {
    list_invocation_ordinal: String,
    node_ordinal: u32,
    thread_id: u32,
    layout_object_ptr_hex: String,
    graphics_ptr_hex: String,
    render_context_ptr_hex: String,
    begin_qpc_ticks: String,
    end_qpc_ticks: String,
    owner: Option<DiagnosticOwnerJoin>,
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

pub(super) fn install(module_base: usize) -> Result<(), String> {
    let Some(output_dir) = std::env::var_os(EVIDENCE_OUT_DIR_ENV) else {
        return Ok(());
    };
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let output_dir = PathBuf::from(output_dir);
    if !output_dir.is_absolute() || !output_dir.is_dir() {
        return Err(format!(
            "{EVIDENCE_OUT_DIR_ENV} must name an existing absolute directory"
        ));
    }
    let list_target = module_base
        .checked_add(GUI_LIST_CONSUMER_RVA)
        .ok_or("A25 GUI list consumer address overflowed")?;
    let text_target = module_base
        .checked_add(INSTANT_TEXT_RENDER_RVA)
        .ok_or("A25 instant-text render address overflowed")?;
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
    let specs: [HookSpec; 2] = [
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
    ];
    {
        let mut state = STATE
            .get_or_init(|| Mutex::new(BridgeState::default()))
            .lock()
            .map_err(|_| "A25 GUI bridge state poisoned")?;
        *state = BridgeState {
            module_base,
            output_dir: Some(output_dir),
            ..BridgeState::default()
        };
    }
    let targets = create_and_enable_hooks(&specs, "A25 evidence-only GUI bridge")?;
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
    let specs: [HookSpec; 2] = [
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
    LIST_STACK.with(|stack| stack.borrow_mut().clear());
    ACTIVE_OWNER_RUN.with(|run| *run.borrow_mut() = None);
    Ok(())
}

pub(super) fn owns_original_slot(original: &'static AtomicUsize) -> bool {
    std::ptr::eq(original, &ORIGINAL_GUI_LIST_CONSUMER)
        || std::ptr::eq(original, &ORIGINAL_INSTANT_TEXT_RENDER)
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
        && ACTIVE_OWNER_RUN.with(|run| run.borrow().is_some())
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
    state.captured_node_count = 0;
    state.current_candidate = None;
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
                state.captured_node_count = 0;
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
    let active = ActiveOwnerRun {
        list_invocation_ordinal,
        node_ordinal,
        thread_id: unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
        layout_object_ptr: layout_object,
        graphics_ptr: graphics,
        render_context_ptr: render_context,
        start_qpc_ticks: query_performance_counter().unwrap_or(0),
        owner,
        transform_f32_bits: read_u32x16(transform),
        object_rect_candidate_i32: read_object_rect_candidate(layout_object),
        object_clip_candidate_i32: read_i32x4(layout_object.saturating_add(0x158)),
        draws: Vec::new(),
        capture_failure_count: 0,
    };
    ACTIVE_OWNER_RUN.with(|run| *run.borrow_mut() = Some(active));
    unsafe { original(layout_object, graphics, render_context, transform) };
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
        captured,
    );
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
        (
            state
                .output_dir
                .clone()
                .ok_or("A25 GUI bridge output directory is absent")?,
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
    let path = output_dir.join(format!(
        "a25-gui-owner-run-p{present_global_order}-qpc{present_qpc_ticks}.json"
    ));
    persist_create_new_or_equal(&path, &capture)
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
        });
    }
    Some((count as u32, nodes, retained != count))
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
    let Some(active) = ACTIVE_OWNER_RUN.with(|run| run.borrow_mut().take()) else {
        reject_owner_run();
        return;
    };
    let owner_ptr = active
        .owner
        .as_ref()
        .and_then(|owner| parse_pointer_hex(&owner.owner_ptr_hex));
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
    let run = DiagnosticOwnerRun {
        list_invocation_ordinal: active.list_invocation_ordinal.to_string(),
        node_ordinal: active.node_ordinal,
        thread_id: active.thread_id,
        layout_object_ptr_hex: pointer_hex(active.layout_object_ptr),
        graphics_ptr_hex: pointer_hex(active.graphics_ptr),
        render_context_ptr_hex: pointer_hex(active.render_context_ptr),
        begin_qpc_ticks: active.start_qpc_ticks.to_string(),
        end_qpc_ticks: end_qpc_ticks.to_string(),
        owner: active.owner,
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
            state
                .active_lists
                .get_mut(&active.list_invocation_ordinal)
                .is_some_and(|list| {
                    list.owner_runs.push(run);
                    true
                })
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
    captured: CapturedD3dCore,
) -> DiagnosticDrawFingerprint {
    let qpc_ticks = query_performance_counter().unwrap_or(0);
    let core = captured.state;
    let shaders = diagnostic_shaders(&core, capture_sequence);
    let payload_bindings = diagnostic_payload_bindings(
        &core,
        &captured.buffer_snapshots,
        &captured.texture_resource_snapshots,
        &captured.shader_resource_view_snapshots,
    );
    let target = diagnostic_target(&core, capture_sequence);
    let target_descriptor_sha256 = target.as_ref().and_then(|target| sha256_json(target).ok());
    let geometry_state_sha256 = hash_geometry_state(&core);
    let pipeline_state_sha256 = hash_pipeline_state(&core, &shaders);
    let output_state_sha256 = hash_output_state(&core, target_descriptor_sha256.as_deref());
    let draw_payload_sha256 = hash_draw_payload(
        draw_call,
        &payload_bindings,
        &captured.buffer_snapshots,
        &captured.texture_resource_snapshots,
    );
    let raw_d3d_fingerprint_sha256 = hash_raw_d3d(
        capture_sequence,
        producer_rva,
        &core,
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
            let exact_vslot_dispatch =
                node.is_some_and(|node| node.exact_instant_text_dispatch);
            let list_and_owner_thread_match = run.thread_id == list.thread_id;
            let list_and_owner_graphics_match = run.graphics_ptr_hex == list.graphics_ptr_hex;
            let layout_equals_graphical_node = node.is_some_and(|node| {
                node.graphical_node_ptr_hex == run.layout_object_ptr_hex
            });
            let registered_owner_resolved = run.owner.is_some();
            let registered_owner_path_candidate_present = run.owner.as_ref().is_some_and(|owner| {
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
    capture: &DiagnosticPresentCapture,
) -> Result<(), String> {
    use std::io::Write;
    let mut bytes = serde_json::to_vec_pretty(capture).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(path).map_err(|error| error.to_string())? != bytes {
                return Err(format!(
                    "A25 GUI diagnostic path already exists with different bytes: {}",
                    path.display()
                ));
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
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
