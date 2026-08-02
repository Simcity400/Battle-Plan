//! Exact-a729 passive acquisition owner for A31 national-focus presentation.
//!
//! The same hook set serves the ordinary collector and optional fixture owner.
//! It makes `ExactA729FocusObservationBoundary` reachable in-process. A
//! selected-Present transaction admits
//! only a completed update that happened before the selected Present's QPC
//! frontier. Process pointers are never included in an admitted snapshot or
//! evidence record.

use crate::a729_focus_item_producer::{
    materialize_pointer_free_snapshot, ExactA729FocusObservationBoundary,
    ProcessLocalFrameObservation, ProcessLocalItemObservation, VectorHeader,
    DETAIL_REFRESH_RVA, DYNAMIC_REFRESH_RVA, HIGHLIGHT_PRODUCER_RVA, ITEM_STATE_PRODUCER_RVA,
    ITEM_STATE_WRITER_RVA,
    VECTOR_PUBLISHER_RVA, VIEW_VTABLE,
};
use crate::a729_focus_item_inline;
use compact_contract::national_focus_presentation::{
    encode_payload, validate_snapshot, AuthorityState, Frame, RectI32, SelectedPresentBinding,
    Snapshot, AUTHORED_SURFACE_IDENTITY, EFFECTIVE_INTERFACE_WINNER,
    EXACT20_FUNCTION_MANIFEST_SHA256, EXACT20_JOB_RECEIPT_SHA256, EXECUTABLE_SHA256,
    MAX_UI_PASS_OPERATIONS, RUNTIME_ROOT_IDENTITY,
};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use std::collections::{BTreeMap, BTreeSet};
use std::cell::Cell;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const EXACT_A729_JOB_ID: &str =
    "20260720T104531Z-w5-a31-consolidated-owner-frame-order-pr-79b1ba63";
pub const FRESH_ACQUISITION_JOB_ID: &str =
    "20260720T062545Z-w4-a31-owner-runnable-producer-e5090f36";
pub const FRESH_ACQUISITION_MANIFEST_SHA256: &str =
    "510353dae716bccaaf37f988b0f7101e31552ecf687ecbd613bce1dd2efb784a";
pub const SENTINEL_DUMP_JOB_ID: &str =
    "20260720T062745Z-w4-a31-hook-entry-sentinels-284611de";
pub const SENTINEL_DUMP_SHA256: &str =
    "498d09aa4161bedf4087983b47ee97626c4d78df9a4537026aff2ea1b4671429";
pub const COUNTRY_CONTEXT_SENTINEL_JOB_ID: &str =
    "20260720T065515Z-w4-a31-country-context-hook-sentinel-44d091b1";
pub const COUNTRY_CONTEXT_SENTINEL_DUMP_SHA256: &str =
    "4c85cfe84aee03808798f821e90f8ab5a7e019e990be67ee48b47d34c09d83b4";
pub const FRAME_ORDER_JOB_ID: &str =
    "20260720T103726Z-w5-a31-exact-frame-order-boundary-a49db01b";
pub const FRAME_ORDER_MANIFEST_SHA256: &str =
    "c3c2a38805153497ad3298893a1b101582aa0651604a742e78214108b7820745";
pub const FRAME_ORDER_JOB_RECEIPT_SHA256: &str =
    "58ec9118271f47d517ed1b29cb3c900dccc4e0f60e553bbb9bbd47e3de4d082b";
pub const DETAIL_REFRESH_SENTINEL_JOB_ID: &str =
    "20260720T102614Z-w5-a31-detail-refresh-entry-sentinel-7f8e06ea";
pub const DETAIL_REFRESH_SENTINEL_DUMP_SHA256: &str =
    "31966be8db16e8aaf55465e901b6d0e8b704f8ecf0af0b83371fc540bc69723b";
pub const INLINE_INSTRUMENTATION_JOB_ID: &str =
    "20260720T213713Z-w5final-a31-exact-display-joint-instrume-56d3fc12";
pub const INLINE_INSTRUMENTATION_MANIFEST_SHA256: &str =
    "76cb5caf7dc418b5fbe79dac037440ccb3cecd34140ee9019ad33c96950e0273";
pub const INLINE_INSTRUMENTATION_JOB_RECEIPT_SHA256: &str =
    "5c0dcd6032b2aa747ed02bec309c9c41d0697e445b46b86b91f9fb4132f5059a";
pub const FOCUS_GUI_ROOT_JOB_ID: &str =
    "20260723T045841Z-a31-focus-canvas-runtime-graphical-root-d0639223";
pub const FOCUS_GUI_ROOT_MANIFEST_SHA256: &str =
    "95e8555d6c6444f54acb03d9fa93f259c3cd75c9e5220ec155a79b9adb0d76b4";

/// The exact passive observer is admitted by executable identity, sentinels,
/// complete callback witnesses and a selected-Present frontier. A missing
/// refresh remains typed unavailable; fixture transitions are regression-only.
pub const PRODUCTION_ACTIVATION_AUTHORIZED: bool = true;
pub const COMPACT_DOMAIN_89_SCHEDULED: bool = true;

const MAX_ITEMS: usize = 131_072;
/// `CNationalFocusView` installs its secondary graphical vtable at +0x30 in
/// the exact-a729 constructor. This is the common GUI traversal owner; the
/// enclosing focus controller at +0 is retained separately and is not
/// registered as the drawable root.
const FOCUS_GUI_SUBOBJECT_OFFSET: usize = 0x30;
/// Runtime `CContainerWindow*` used by `0x141377E00`.
const FOCUS_CANVAS_RUNTIME_ROOT_OFFSET: usize = 0x588;
/// `CNationalFocusTreeItem` constructor `0x14136A5C0` stores the runtime GUI
/// object used for every named child lookup in `param_1[2]`.
const FOCUS_ITEM_RUNTIME_GUI_ROOT_OFFSET: usize = 0x10;
/// Uniform focus-canvas scale consumed and clamped by `0x141377E00`.
const FOCUS_CANVAS_SCALE_OFFSET: usize = 0x2118;
const FOCUS_CANVAS_POSITION_WIDGET_OFFSET: usize = 0x590;
const GRAPHICAL_POSITION_SUBOBJECT_OFFSET: usize = 0x30;
const GRAPHICAL_POSITION_VSLOT: usize = 0x98;
const CONTAINER_CONTENT_EXTENT_VSLOT: usize = 0x0e8;
const CONTAINER_VERTICAL_SCROLLBAR_OFFSET: usize = 0x15e0;
const CONTAINER_HORIZONTAL_SCROLLBAR_OFFSET: usize = 0x15e8;
const SCROLLBAR_VALUE_SUBOBJECT_OFFSET: usize = 0x80;
const SCROLLBAR_VALUE_VSLOT: usize = 0x20;
const QUIESCE_TIMEOUT: Duration = Duration::from_secs(30);
const VECTOR_PUBLISHER_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10,
    0x48,
];
const DYNAMIC_REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x68, 0x20, 0x56, 0x57, 0x41, 0x54,
    0x41,
];
const DETAIL_REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
    0x48,
];
const ITEM_STATE_WRITER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56, 0x48,
    0x8d,
];
const ITEM_STATE_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10, 0x44, 0x88, 0x40, 0x18, 0x55, 0x56, 0x57, 0x41,
    0x54,
];
const HIGHLIGHT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10, 0x48, 0x89, 0x78, 0x20,
    0x55,
];

type VectorPublisherFn = unsafe extern "system" fn(usize, usize, usize, usize);
type DynamicRefreshFn = unsafe extern "system" fn(usize, f32, usize);
type DetailRefreshFn = unsafe extern "system" fn(usize);
type ItemStateWriterFn = unsafe extern "system" fn(usize, u32);
type ItemStateProducerFn = unsafe extern "system" fn(usize, usize, u8, f32, usize);
type HighlightFn = unsafe extern "system" fn(usize, u32);
type JointCountryRendererFn =
    unsafe extern "system" fn(usize, usize, usize, usize, usize, usize);

static ORIGINAL_VECTOR_PUBLISHER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DYNAMIC_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DETAIL_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ITEM_STATE_WRITER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ITEM_STATE_PRODUCER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_HIGHLIGHT: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static HOOK_SET_PRESENT: AtomicBool = AtomicBool::new(false);
static LIFECYCLE_HEALTHY: AtomicBool = AtomicBool::new(false);
static TEARDOWN_ACTIVE: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static HEALTH_EPOCH: AtomicU64 = AtomicU64::new(1);
static NEXT_CALLBACK_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static DECODER: OnceLock<Mutex<Option<Arc<dyn ExactA729FocusLiveDecoder>>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub vector_publisher: usize,
    pub dynamic_refresh: usize,
    pub detail_refresh: usize,
    pub item_state_writer: usize,
    pub item_state_producer: usize,
    pub highlight_producer: usize,
    pub display_percent_callsite: usize,
    pub joint_country_callsite: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPresentStamp {
    pub frame_hour: u64,
    pub selected_present_global_order: u32,
    pub selected_present_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    /// Raw injected-render-hook selection receipt digest.
    pub selected_present_receipt_sha256: String,
    /// Canonical portable-render-stream `SelectedPresentReceipt` digest.
    pub portable_selected_present_receipt_sha256: String,
}

/// Process-local owner registration consumed by the selected-Present GUI
/// bridge. `focus_view_ptr` is the paired-refresh controller and
/// `runtime_graphical_root_ptr` is its exact +0x30 graphical subobject; neither
/// pointer can enter NFP1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NationalFocusCanvasPresentRegistration {
    pub authored_surface_identity: &'static str,
    pub runtime_root_identity: &'static str,
    pub effective_interface_winner: &'static str,
    pub callback_qpc_ticks: u64,
    pub frame_hour: u64,
    pub focus_view_ptr: usize,
    pub runtime_root_ptr: usize,
    pub runtime_graphical_root_ptr: usize,
}

/// Process-local identity for one visible focus and the runtime GUI subtree
/// constructed for it. These pointers are join keys only and must be erased
/// before NFP1 or the selected-Present UI pass is serialized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NationalFocusGuiItemRegistration {
    pub focus_id: String,
    pub item_ptr: usize,
    pub runtime_gui_root_ptr: usize,
}

/// Complete process-local A31 side of the common-GUI join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingNationalFocusGuiJoin {
    pub root: NationalFocusCanvasPresentRegistration,
    pub items: Vec<NationalFocusGuiItemRegistration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NationalFocusCanvasRuntimeGeometry {
    pub sample_started_qpc_ticks: u64,
    pub sample_completed_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub content_extent_pixels: [i32; 2],
    pub canvas_origin_pixels_fixed5: [i64; 2],
    pub pan_pixels_fixed5: [i64; 2],
    pub canvas_scale_bits: [u32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusSelectedPresentOperations {
    pub focus_id: String,
    pub operation_ordinals: Vec<u32>,
    pub selected_present_screen_rect: RectI32,
}

/// Exact values returned by the global selected-Present owner/GUI bridge.
/// There are deliberately no defaults. The focus lane remains unavailable
/// until the bridge supplies this complete record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPresentCanvasBinding {
    pub selected_present_candidate_id: u64,
    pub selected_present_ordinal: u32,
    pub selected_present_cutoff_qpc_ticks: u64,
    pub selected_present_pass_identity_sha256: String,
    pub selected_present_engine_instance_token: u64,
    pub authored_surface_identity: String,
    pub runtime_root_identity: String,
    pub effective_interface_winner: String,
    pub render_target_extent_pixels: [i32; 2],
    pub client_extent_pixels: [i32; 2],
    pub clip_rect: RectI32,
    pub content_extent_pixels: [i32; 2],
    pub viewport_extent_pixels: [i32; 2],
    pub canvas_origin_pixels_fixed5: [i64; 2],
    pub pan_pixels_fixed5: [i64; 2],
    pub canvas_scale_bits: [u32; 2],
    pub focus_operations: Vec<FocusSelectedPresentOperations>,
    pub canvas_operation_ordinals: Vec<u32>,
    pub selected_present_ui_operation_order: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRefreshEpoch {
    pub generation: u64,
    pub callback_sequence: u64,
    pub completion_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub frame_hour: u64,
    pub country_tag: String,
    pub view_address: usize,
    pub vector_publish_seen: bool,
    pub published_view_address: Option<usize>,
    pub country_context_address: Option<usize>,
    pub item_state_by_address: BTreeMap<usize, u8>,
    pub highlight_by_address: BTreeMap<usize, bool>,
    pub display_percent_by_definition: BTreeMap<usize, u8>,
    pub joint_country_by_item: BTreeMap<usize, Option<String>>,
}

/// The collector supplies the exact process reader. This indirection is
/// intentional: the lane owns hook lifecycle/order, while the collector owns
/// its SEH-guarded reader and already-authenticated focus-token decoder.
pub trait ExactA729FocusLiveDecoder: Send + Sync {
    fn decode_completed_refresh(
        &self,
        epoch: &RawRefreshEpoch,
    ) -> Result<ProcessLocalFrameObservation, String>;
    fn stable_focus_id(&self, definition_address: usize) -> Result<String, String>;
    fn stable_country_tag(&self, tag_db_id: u32) -> Result<String, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    snapshot: Snapshot,
    focus_view_ptr: usize,
    runtime_root_ptr: usize,
    runtime_graphical_root_ptr: usize,
    runtime_geometry: NationalFocusCanvasRuntimeGeometry,
    focus_gui_items: Vec<NationalFocusGuiItemRegistration>,
    same_epoch_vector_publish_seen: bool,
    generation: u64,
    callback_sequence: u64,
    completion_qpc_ticks: u64,
    qpc_frequency_hz: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcquisitionReceipt {
    pub schema: &'static str,
    pub binary_sha256: [u8; 32],
    pub function_manifest_sha256: [u8; 32],
    pub job_receipt_sha256: [u8; 32],
    pub generation: u64,
    pub arm_qpc_ticks: u64,
    pub frame_hour: u64,
    pub country_tag: String,
    pub callback_sequence: u64,
    pub completion_qpc_ticks: u64,
    pub selected_present_global_order: u32,
    pub selected_present_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub selected_present_receipt_sha256: String,
    pub portable_selected_present_receipt_sha256: String,
    pub selected_present_binding: SelectedPresentBinding,
    pub source_sequence: u64,
    pub visible_item_count: u32,
    pub same_epoch_vector_publish_seen: bool,
    pub vector_publish_callbacks: u64,
    pub dynamic_refresh_callbacks: u64,
    pub detail_refresh_callbacks: u64,
    pub state_writer_callbacks: u64,
    pub state_producer_callbacks: u64,
    pub highlight_callbacks: u64,
    pub display_percent_callbacks: u64,
    pub joint_country_callbacks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub receipt: AcquisitionReceipt,
    pub frame: Frame,
    pub encoded_nfp1: Vec<u8>,
}

#[derive(Default)]
struct HookState {
    active: Option<ActiveEpoch>,
    admitted: Option<(Snapshot, AcquisitionReceipt)>,
    fatal_error: Option<String>,
    vector_publish_callbacks: u64,
    dynamic_refresh_callbacks: u64,
    detail_refresh_callbacks: u64,
    state_writer_callbacks: u64,
    state_producer_callbacks: u64,
    highlight_callbacks: u64,
    display_percent_callbacks: u64,
    joint_country_callbacks: u64,
}

struct ActiveEpoch {
    generation: u64,
    arm_qpc_ticks: u64,
    arm_qpc_frequency_hz: u64,
    frame_hour: u64,
    country_tag: String,
    expected_country_object_address: usize,
    vector_publish_seen: bool,
    published_view_address: Option<usize>,
    country_context_address: Option<usize>,
    item_state_by_address: BTreeMap<usize, u8>,
    highlight_by_address: BTreeMap<usize, bool>,
    display_percent_by_definition: BTreeMap<usize, u8>,
    joint_country_by_item: BTreeMap<usize, Option<String>>,
    pending_dynamic_view_address: Option<usize>,
    transient_cycle_active: bool,
    candidate: Option<Candidate>,
}

struct CallbackGuard;
impl CallbackGuard {
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for CallbackGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Boundary<'a>(&'a dyn ExactA729FocusLiveDecoder);
impl ExactA729FocusObservationBoundary for Boundary<'_> {
    fn read_complete_observation(
        &self,
        _frame_hour: u64,
        _country_tag: &str,
    ) -> Result<ProcessLocalFrameObservation, String> {
        Err("A31 live boundary materializes synchronously at refresh completion".into())
    }
    fn stable_focus_id(&self, definition_address: usize) -> Result<String, String> {
        self.0.stable_focus_id(definition_address)
    }
}

thread_local! {
    /// The exact `0x141379a50` invocation currently executing on this game
    /// thread.  Its positive joint-country callsite has no item argument, so
    /// the callsite detour joins through this bounded, nest-restoring slot.
    static CURRENT_JOINT_ITEM: Cell<usize> = const { Cell::new(0) };
}

struct CurrentJointItemGuard(usize);
impl CurrentJointItemGuard {
    fn enter(item: usize) -> Self {
        let previous = CURRENT_JOINT_ITEM.with(|slot| slot.replace(item));
        Self(previous)
    }
}
impl Drop for CurrentJointItemGuard {
    fn drop(&mut self) {
        CURRENT_JOINT_ITEM.with(|slot| slot.set(self.0));
    }
}

pub struct A729HookedFocusSource;
impl compact_contract::national_focus_presentation::ExactNationalFocusPresentationSource
    for A729HookedFocusSource
{
    fn is_exact_a729_focus_view_source(&self) -> bool {
        INSTALLED.load(Ordering::Acquire) && LIFECYCLE_HEALTHY.load(Ordering::Acquire)
    }
    fn producer_manifest_sha256(&self) -> [u8; 32] {
        EXACT20_FUNCTION_MANIFEST_SHA256
    }
    fn collect_snapshot(&self, frame_hour: u64, country_tag: &str) -> Result<Snapshot, String> {
        let state = state().lock().map_err(|_| "A31 hook state poisoned")?;
        if let Some(error) = &state.fatal_error {
            return Err(error.clone());
        }
        let (snapshot, _) = state
            .admitted
            .as_ref()
            .ok_or("A31 has no selected-Present-admitted snapshot")?;
        if snapshot.country_tag != country_tag
            || !snapshot.producer_record_identity.contains(&format!("a31:{frame_hour}:"))
        {
            return Err("A31 admitted snapshot does not match requested frame identity".into());
        }
        Ok(snapshot.clone())
    }
}

fn state() -> &'static Mutex<HookState> {
    STATE.get_or_init(|| Mutex::new(HookState::default()))
}

fn decoder() -> Result<Arc<dyn ExactA729FocusLiveDecoder>, String> {
    DECODER
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|slot| slot.as_ref().cloned())
        .ok_or_else(|| "A31 exact process decoder is not registered".into())
}

fn capture_gate_open() -> bool {
    INSTALLED.load(Ordering::Acquire)
        && LIFECYCLE_HEALTHY.load(Ordering::Acquire)
        && !TEARDOWN_ACTIVE.load(Ordering::Acquire)
}

pub fn arm_selected_present_epoch(
    frame_hour: u64,
    country_tag: &str,
    expected_country_object_address: usize,
) -> Result<u64, String> {
    if !capture_gate_open() {
        return Err("A31 fixture hooks are not lifecycle-healthy".into());
    }
    if expected_country_object_address == 0
        || country_tag.len() != 3
        || !country_tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err("A31 fixture country tag is not canonical".into());
    }
    let (arm_qpc_ticks, arm_qpc_frequency_hz) =
        qpc_now().ok_or("A31 arm QPC read failed")?;
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::AcqRel);
    if generation == 0 {
        return Err("A31 fixture generation overflowed".into());
    }
    let mut state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    if state.active.is_some() {
        return Err("A31 fixture epoch already active".into());
    }
    state.admitted = None;
    state.fatal_error = None;
    state.vector_publish_callbacks = 0;
    state.dynamic_refresh_callbacks = 0;
    state.detail_refresh_callbacks = 0;
    state.state_writer_callbacks = 0;
    state.state_producer_callbacks = 0;
    state.highlight_callbacks = 0;
    state.display_percent_callbacks = 0;
    state.joint_country_callbacks = 0;
    state.active = Some(ActiveEpoch {
        generation,
        arm_qpc_ticks,
        arm_qpc_frequency_hz,
        frame_hour,
        country_tag: country_tag.into(),
        expected_country_object_address,
        vector_publish_seen: false,
        published_view_address: None,
        country_context_address: None,
        item_state_by_address: BTreeMap::new(),
        highlight_by_address: BTreeMap::new(),
        display_percent_by_definition: BTreeMap::new(),
        joint_country_by_item: BTreeMap::new(),
        pending_dynamic_view_address: None,
        transient_cycle_active: false,
        candidate: None,
    });
    Ok(generation)
}

/// Drop an unsealed epoch before re-arming for a new hour/country. No partial
/// candidate is admitted, and no process-local identity escapes this reset.
pub fn cancel_active_epoch() -> Result<(), String> {
    let mut state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    state.active = None;
    state.admitted = None;
    state.fatal_error = None;
    Ok(())
}

/// Return the exact process-local focus root that the global GUI owner registry
/// must register before it can seal the selected Present. Absence of a complete
/// refresh is a hard unavailable result; callers must not substitute the
/// country tag, focus-tree id, or authored GUI file for this runtime owner.
pub fn pending_canvas_present_registration(
    generation: u64,
) -> Result<NationalFocusCanvasPresentRegistration, String> {
    Ok(pending_focus_gui_join(generation)?.root)
}

/// Return the exact focus canvas root and every visible focus's runtime GUI
/// subtree. The selected-Present owner bridge joins draws through the runtime
/// parent link at +0x108; authored names and screen rectangles are never used
/// as ownership keys.
pub fn pending_focus_gui_join(
    generation: u64,
) -> Result<PendingNationalFocusGuiJoin, String> {
    let state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    if let Some(error) = &state.fatal_error {
        return Err(error.clone());
    }
    let active = state
        .active
        .as_ref()
        .ok_or("A31 no focus epoch is active")?;
    if active.generation != generation {
        return Err("A31 focus registration generation differs".into());
    }
    let candidate = active
        .candidate
        .as_ref()
        .ok_or("A31 focus canvas has no complete refresh to register")?;
    if candidate.runtime_graphical_root_ptr == 0 {
        return Err("A31 focus canvas runtime graphical root is null".into());
    }
    let graphical_vtable = read_process_usize(candidate.runtime_graphical_root_ptr)
        .ok_or("A31 focus canvas graphical subobject vtable is unreadable")?;
    authenticate_and_normalize_image_address(graphical_vtable, VIEW_VTABLE)
        .map_err(|_| "A31 focus canvas graphical subobject vtable is foreign".to_string())?;
    if candidate.focus_gui_items.len() != candidate.snapshot.visible_items.len() {
        return Err("A31 focus GUI item-root frontier is incomplete".into());
    }
    let root = NationalFocusCanvasPresentRegistration {
        authored_surface_identity: AUTHORED_SURFACE_IDENTITY,
        runtime_root_identity: RUNTIME_ROOT_IDENTITY,
        effective_interface_winner: EFFECTIVE_INTERFACE_WINNER,
        callback_qpc_ticks: candidate.completion_qpc_ticks,
        frame_hour: active.frame_hour,
        focus_view_ptr: candidate.focus_view_ptr,
        runtime_root_ptr: candidate.runtime_root_ptr,
        runtime_graphical_root_ptr: candidate.runtime_graphical_root_ptr,
    };
    Ok(PendingNationalFocusGuiJoin {
        root,
        items: candidate.focus_gui_items.clone(),
    })
}

/// Sample the exact focus ContainerWindow geometry through the same pure
/// getters consumed by `0x141377E00`. The returned QPC bounds the observation;
/// callers must reject it when it cannot be joined to their selected Present.
pub fn sample_pending_focus_gui_geometry(
    generation: u64,
) -> Result<NationalFocusCanvasRuntimeGeometry, String> {
    let state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    if let Some(error) = &state.fatal_error {
        return Err(error.clone());
    }
    let active = state
        .active
        .as_ref()
        .ok_or("A31 no focus epoch is active")?;
    if active.generation != generation {
        return Err("A31 focus geometry generation differs".into());
    }
    let candidate = active
        .candidate
        .as_ref()
        .ok_or("A31 focus canvas has no completed geometry observation")?;
    Ok(candidate.runtime_geometry.clone())
}

fn sample_focus_gui_geometry(
    focus_view_ptr: usize,
    runtime_root_ptr: usize,
) -> Result<NationalFocusCanvasRuntimeGeometry, String> {
    let (sample_started_qpc_ticks, qpc_frequency_hz) =
        qpc_now().ok_or("A31 focus geometry QPC read failed")?;
    let content_extent_pixels = runtime_content_extent(runtime_root_ptr)?;
    let horizontal_pan =
        runtime_scrollbar_value(runtime_root_ptr, CONTAINER_HORIZONTAL_SCROLLBAR_OFFSET)?;
    let vertical_pan =
        runtime_scrollbar_value(runtime_root_ptr, CONTAINER_VERTICAL_SCROLLBAR_OFFSET)?;
    let canvas_origin_pixels = runtime_canvas_origin_pixels(focus_view_ptr)?;
    let scale_address = focus_view_ptr
        .checked_add(FOCUS_CANVAS_SCALE_OFFSET)
        .ok_or("A31 focus canvas scale address overflowed")?;
    let scale_bits =
        read_process_u32(scale_address).ok_or("A31 focus canvas scale was unreadable")?;
    let scale = f32::from_bits(scale_bits);
    if content_extent_pixels.iter().any(|value| *value <= 0)
        || !scale.is_finite()
        || scale <= 0.0
    {
        return Err("A31 focus canvas runtime geometry is invalid".into());
    }
    let (sample_completed_qpc_ticks, completed_frequency_hz) =
        qpc_now().ok_or("A31 focus geometry closing QPC read failed")?;
    if completed_frequency_hz != qpc_frequency_hz
        || sample_completed_qpc_ticks < sample_started_qpc_ticks
    {
        return Err("A31 focus geometry QPC interval is invalid".into());
    }
    Ok(NationalFocusCanvasRuntimeGeometry {
        sample_started_qpc_ticks,
        sample_completed_qpc_ticks,
        qpc_frequency_hz,
        content_extent_pixels,
        canvas_origin_pixels_fixed5: [
            i64::from(canvas_origin_pixels[0])
                .checked_mul(32)
                .ok_or("A31 focus canvas origin X fixed5 overflowed")?,
            i64::from(canvas_origin_pixels[1])
                .checked_mul(32)
                .ok_or("A31 focus canvas origin Y fixed5 overflowed")?,
        ],
        pan_pixels_fixed5: [
            exact_fixed5(horizontal_pan)?,
            exact_fixed5(vertical_pan)?,
        ],
        canvas_scale_bits: [scale_bits, scale_bits],
    })
}

/// Exact internal-callsite instrumentation owned by the custom harness feeds
/// the final `ESI -> EDX` value from `0x141bb636e` through this bounded join.
/// A function-entry observation alone is deliberately insufficient.
pub fn record_display_percent_callsite(
    generation: u64,
    definition_address: usize,
    percent: u8,
) -> Result<(), String> {
    if percent > 100 || definition_address == 0 {
        return Err("A31 display-percent observation is outside the exact domain".into());
    }
    let mut state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    state.display_percent_callbacks = state.display_percent_callbacks.saturating_add(1);
    let active = active_generation(&mut state, generation)?;
    if !active.transient_cycle_active {
        return Err("A31 display-percent callsite occurred outside the paired refresh cycle".into());
    }
    if let Some(previous) = active
        .display_percent_by_definition
        .insert(definition_address, percent)
    {
        if previous != percent {
            state.fatal_error = Some("A31 conflicting same-epoch display percentages".into());
            return Err(state.fatal_error.clone().unwrap());
        }
    }
    Ok(())
}

/// Exact joint-view helper instrumentation records measured hidden as `None`;
/// absence from the map remains an unobserved leaf and rejects materialization.
pub fn record_joint_country_observation(
    generation: u64,
    item_address: usize,
    country_tag: Option<&str>,
) -> Result<(), String> {
    if item_address == 0
        || country_tag.is_some_and(|tag| {
            tag.len() != 3
                || !tag
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
    {
        return Err("A31 joint-country observation identity is invalid".into());
    }
    let value = country_tag.map(str::to_owned);
    let mut state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    state.joint_country_callbacks = state.joint_country_callbacks.saturating_add(1);
    let active = active_generation(&mut state, generation)?;
    if !active.transient_cycle_active {
        return Err("A31 joint-country observation occurred outside the paired refresh cycle".into());
    }
    if let Some(previous) = active
        .joint_country_by_item
        .insert(item_address, value.clone())
    {
        // The item-producer entry writes measured hidden first.  The exact
        // positive callsite is allowed to refine that one provisional None to
        // its authenticated tag; every other disagreement is fatal.
        if previous != value && !(previous.is_none() && value.is_some()) {
            state.fatal_error = Some("A31 conflicting same-epoch joint-country leaves".into());
            return Err(state.fatal_error.clone().unwrap());
        }
    }
    Ok(())
}

fn active_generation(
    state: &mut HookState,
    generation: u64,
) -> Result<&mut ActiveEpoch, String> {
    let active = state.active.as_mut().ok_or("A31 no fixture epoch is active")?;
    if active.generation != generation {
        return Err("A31 fixture generation does not match".into());
    }
    Ok(active)
}

pub fn seal_selected_present_epoch(
    _generation: u64,
    _selected: SelectedPresentStamp,
) -> Result<AcquisitionReceipt, String> {
    Err(
        "A31 selected Present omitted exact focus-canvas owner/geometry/operation binding"
            .into(),
    )
}

pub fn seal_selected_present_epoch_with_canvas(
    generation: u64,
    selected: SelectedPresentStamp,
    canvas: SelectedPresentCanvasBinding,
) -> Result<AcquisitionReceipt, String> {
    if !capture_gate_open() {
        return Err("A31 fixture hooks are not lifecycle-healthy".into());
    }
    if selected.selected_present_global_order == 0
        || selected.selected_present_qpc_ticks == 0
        || selected.qpc_frequency_hz == 0
        || !is_sha256(&selected.selected_present_receipt_sha256)
        || !is_sha256(&selected.portable_selected_present_receipt_sha256)
    {
        return Err("A31 selected-Present stamp is incomplete".into());
    }
    if canvas.selected_present_candidate_id == 0
        || canvas.selected_present_cutoff_qpc_ticks < selected.selected_present_qpc_ticks
        || !is_sha256(&canvas.selected_present_pass_identity_sha256)
        || canvas.selected_present_engine_instance_token == 0
        || canvas.authored_surface_identity != AUTHORED_SURFACE_IDENTITY
        || canvas.runtime_root_identity != RUNTIME_ROOT_IDENTITY
        || canvas.effective_interface_winner != EFFECTIVE_INTERFACE_WINNER
    {
        return Err("A31 focus-canvas selected-Present owner binding is incomplete or foreign".into());
    }
    let mut state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    if let Some(error) = &state.fatal_error {
        return Err(error.clone());
    }
    let active = state.active.take().ok_or("A31 no fixture epoch is active")?;
    if active.generation != generation || active.frame_hour != selected.frame_hour {
        state.fatal_error = Some("A31 fixture and selected-Present identities differ".into());
        return Err(state.fatal_error.clone().unwrap());
    }
    let mut candidate = active
        .candidate
        .ok_or("A31 epoch contained no complete focus refresh")?;
    if candidate.generation != generation
        || active.arm_qpc_frequency_hz != selected.qpc_frequency_hz
        || candidate.qpc_frequency_hz != selected.qpc_frequency_hz
        || candidate.completion_qpc_ticks < active.arm_qpc_ticks
        || candidate.completion_qpc_ticks > selected.selected_present_qpc_ticks
    {
        state.fatal_error = Some("A31 refresh did not precede the selected Present".into());
        return Err(state.fatal_error.clone().unwrap());
    }
    if (state.vector_publish_callbacks > 0) != candidate.same_epoch_vector_publish_seen
        || state.dynamic_refresh_callbacks == 0
        || state.detail_refresh_callbacks == 0
        || state.state_writer_callbacks == 0
        || state.state_producer_callbacks == 0
        || state.highlight_callbacks == 0
        || state.display_percent_callbacks == 0
        || state.joint_country_callbacks == 0
    {
        state.fatal_error = Some("A31 transaction-local hook witnesses are incomplete".into());
        return Err(state.fatal_error.clone().unwrap());
    }
    bind_canvas_to_snapshot(&mut candidate.snapshot, &canvas)?;
    let selected_present_binding = SelectedPresentBinding {
        materialization_qpc_ticks: candidate.completion_qpc_ticks,
        selected_present_global_order: selected.selected_present_global_order,
        selected_present_candidate_id: canvas.selected_present_candidate_id,
        selected_present_ordinal: canvas.selected_present_ordinal,
        selected_present_qpc_ticks: selected.selected_present_qpc_ticks,
        selected_present_cutoff_qpc_ticks: canvas.selected_present_cutoff_qpc_ticks,
        qpc_frequency_hz: selected.qpc_frequency_hz,
        selected_present_receipt_sha256: selected
            .portable_selected_present_receipt_sha256
            .clone(),
        selected_present_pass_identity_sha256: canvas
            .selected_present_pass_identity_sha256
            .clone(),
        selected_present_engine_instance_token: canvas.selected_present_engine_instance_token,
        authored_surface_identity: canvas.authored_surface_identity.clone(),
        runtime_root_identity: canvas.runtime_root_identity.clone(),
        effective_interface_winner: canvas.effective_interface_winner.clone(),
        render_target_extent_pixels: canvas.render_target_extent_pixels,
        client_extent_pixels: canvas.client_extent_pixels,
        clip_rect: canvas.clip_rect,
        content_extent_pixels: canvas.content_extent_pixels,
        viewport_extent_pixels: canvas.viewport_extent_pixels,
        canvas_origin_pixels_fixed5: canvas.canvas_origin_pixels_fixed5,
        pan_pixels_fixed5: canvas.pan_pixels_fixed5,
        canvas_scale_bits: canvas.canvas_scale_bits,
    };
    let receipt = AcquisitionReceipt {
        schema: "hoi4.a31.selected_present_acquisition_receipt.v3",
        binary_sha256: EXECUTABLE_SHA256,
        function_manifest_sha256: EXACT20_FUNCTION_MANIFEST_SHA256,
        job_receipt_sha256: EXACT20_JOB_RECEIPT_SHA256,
        generation,
        arm_qpc_ticks: active.arm_qpc_ticks,
        frame_hour: selected.frame_hour,
        country_tag: candidate.snapshot.country_tag.clone(),
        callback_sequence: candidate.callback_sequence,
        completion_qpc_ticks: candidate.completion_qpc_ticks,
        selected_present_global_order: selected.selected_present_global_order,
        selected_present_qpc_ticks: selected.selected_present_qpc_ticks,
        qpc_frequency_hz: selected.qpc_frequency_hz,
        selected_present_receipt_sha256: selected.selected_present_receipt_sha256,
        portable_selected_present_receipt_sha256:
            selected.portable_selected_present_receipt_sha256,
        selected_present_binding,
        source_sequence: candidate.snapshot.source_sequence,
        visible_item_count: candidate.snapshot.visible_items.len() as u32,
        same_epoch_vector_publish_seen: candidate.same_epoch_vector_publish_seen,
        vector_publish_callbacks: state.vector_publish_callbacks,
        dynamic_refresh_callbacks: state.dynamic_refresh_callbacks,
        detail_refresh_callbacks: state.detail_refresh_callbacks,
        state_writer_callbacks: state.state_writer_callbacks,
        state_producer_callbacks: state.state_producer_callbacks,
        highlight_callbacks: state.highlight_callbacks,
        display_percent_callbacks: state.display_percent_callbacks,
        joint_country_callbacks: state.joint_country_callbacks,
    };
    state.admitted = Some((candidate.snapshot, receipt.clone()));
    Ok(receipt)
}

fn bind_canvas_to_snapshot(
    snapshot: &mut Snapshot,
    canvas: &SelectedPresentCanvasBinding,
) -> Result<(), String> {
    let positive_extent = |extent: [i32; 2]| extent.iter().all(|value| *value > 0);
    let scale_valid = canvas.canvas_scale_bits.iter().all(|bits| {
        let scale = f32::from_bits(*bits);
        scale.is_finite() && scale > 0.0
    });
    let clip_right = canvas.clip_rect.x.checked_add(canvas.clip_rect.width);
    let clip_bottom = canvas.clip_rect.y.checked_add(canvas.clip_rect.height);
    if !positive_extent(canvas.render_target_extent_pixels)
        || !positive_extent(canvas.client_extent_pixels)
        || !positive_extent(canvas.content_extent_pixels)
        || !positive_extent(canvas.viewport_extent_pixels)
        || canvas.clip_rect.x < 0
        || canvas.clip_rect.y < 0
        || canvas.clip_rect.width <= 0
        || canvas.clip_rect.height <= 0
        || clip_right.is_none_or(|right| {
            right > canvas.render_target_extent_pixels[0]
                || right > canvas.client_extent_pixels[0]
        })
        || clip_bottom.is_none_or(|bottom| {
            bottom > canvas.render_target_extent_pixels[1]
                || bottom > canvas.client_extent_pixels[1]
        })
        || canvas.viewport_extent_pixels[0] > canvas.content_extent_pixels[0]
        || canvas.viewport_extent_pixels[1] > canvas.content_extent_pixels[1]
        || canvas.viewport_extent_pixels[0] > canvas.client_extent_pixels[0]
        || canvas.viewport_extent_pixels[1] > canvas.client_extent_pixels[1]
        || !scale_valid
    {
        return Err("A31 focus-canvas geometry/transform is incomplete or out of bounds".into());
    }
    if canvas.focus_operations.len() != snapshot.visible_items.len()
        || canvas.selected_present_ui_operation_order.is_empty()
        || canvas.selected_present_ui_operation_order.len() > MAX_UI_PASS_OPERATIONS
        || canvas.canvas_operation_ordinals.len() > MAX_UI_PASS_OPERATIONS
    {
        return Err("A31 focus-canvas operation ownership does not cover every visible focus".into());
    }
    let mut by_focus = BTreeMap::new();
    let mut mapped = BTreeSet::new();
    for operations in &canvas.focus_operations {
        if operations.operation_ordinals.is_empty()
            || operations.operation_ordinals.len() > MAX_UI_PASS_OPERATIONS
            || operations
                .operation_ordinals
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || by_focus
                .insert(
                    operations.focus_id.as_str(),
                    operations.operation_ordinals.as_slice(),
                )
                .is_some()
        {
            return Err("A31 focus-canvas per-focus operation set is empty, duplicate, or unordered".into());
        }
        for ordinal in &operations.operation_ordinals {
            if !mapped.insert(*ordinal) {
                return Err("A31 selected-Present operation belongs to multiple focuses".into());
            }
            if mapped.len() > MAX_UI_PASS_OPERATIONS {
                return Err("A31 selected-Present operation union exceeds carrier bound".into());
            }
        }
        if operations.selected_present_screen_rect.width <= 0
            || operations.selected_present_screen_rect.height <= 0
        {
            return Err("A31 focus selected-Present screen rectangle is empty".into());
        }
    }
    for item in &mut snapshot.visible_items {
        let operations = by_focus
            .remove(item.focus_id.as_str())
            .ok_or("A31 selected-Present omitted one focus identity")?;
        let screen_rect = canvas
            .focus_operations
            .iter()
            .find(|candidate| candidate.focus_id == item.focus_id)
            .ok_or("A31 selected-Present focus rectangle join was lost")?
            .selected_present_screen_rect;
        item.selected_present_ui_operation_ordinals = operations.to_vec();
        item.selected_present_screen_rect = screen_rect;
    }
    if canvas
        .canvas_operation_ordinals
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err("A31 canvas-level selected-Present operations are unordered".into());
    }
    for ordinal in &canvas.canvas_operation_ordinals {
        if !mapped.insert(*ordinal) {
            return Err("A31 selected-Present operation has multiple focus-canvas owners".into());
        }
        if mapped.len() > MAX_UI_PASS_OPERATIONS {
            return Err("A31 selected-Present operation union exceeds carrier bound".into());
        }
    }
    if !by_focus.is_empty()
        || canvas
            .selected_present_ui_operation_order
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || canvas.selected_present_ui_operation_order
            != mapped.iter().copied().collect::<Vec<_>>()
    {
        return Err("A31 selected-Present operation union is not complete exact order".into());
    }
    snapshot.selected_present_ui_operation_order =
        canvas.selected_present_ui_operation_order.clone();
    snapshot.selected_present_canvas_operation_ordinals =
        canvas.canvas_operation_ordinals.clone();
    validate_snapshot(snapshot).map_err(|error| format!("{error:?}"))
}

/// Evidence-mode drain. It uses the same NFP1 validator and encoder but does
/// not alter either production activation latch. The resulting record is for
/// the root-owned fixture verifier, never the scheduled compact stream.
pub fn drain_evidence_record(elapsed_wall_ms: u64) -> Result<EvidenceRecord, String> {
    let mut state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    let (snapshot, receipt) = state
        .admitted
        .take()
        .ok_or("A31 has no admitted selected-Present record")?;
    validate_snapshot(&snapshot).map_err(|error| format!("{error:?}"))?;
    let frame = Frame {
        elapsed_wall_ms,
        frame_hour: receipt.frame_hour,
        status_flags: 0,
        authority: AuthorityState::Exact20Receipt,
        selected_present: Some(receipt.selected_present_binding.clone()),
        snapshot: Some(snapshot),
    };
    let encoded_nfp1 = encode_payload(&frame).map_err(|error| format!("{error:?}"))?;
    Ok(EvidenceRecord {
        receipt,
        frame,
        encoded_nfp1,
    })
}

#[cfg(feature = "a729-focus-fixture-hook")]
pub fn install_for_fixture_capture(
    decoder: Arc<dyn ExactA729FocusLiveDecoder>,
) -> Result<InstalledAddresses, String> {
    register_decoder(decoder)?;
    install_hook_set()
}

pub fn install() -> Result<InstalledAddresses, String> {
    if !PRODUCTION_ACTIVATION_AUTHORIZED || !COMPACT_DOMAIN_89_SCHEDULED {
        return Err("A31 production hook activation is not authorized".into());
    }
    install_hook_set()
}

pub fn install_production(
    decoder: Arc<dyn ExactA729FocusLiveDecoder>,
) -> Result<InstalledAddresses, String> {
    if !PRODUCTION_ACTIVATION_AUTHORIZED || !COMPACT_DOMAIN_89_SCHEDULED {
        return Err("A31 production hook activation is not authorized".into());
    }
    register_decoder(decoder)?;
    install_hook_set()
}

fn register_decoder(candidate: Arc<dyn ExactA729FocusLiveDecoder>) -> Result<(), String> {
    let slot = DECODER.get_or_init(|| Mutex::new(None));
    let mut slot = slot.lock().map_err(|_| "A31 decoder mutex poisoned")?;
    if HOOK_SET_PRESENT.load(Ordering::Acquire)
        && slot
            .as_ref()
            .is_some_and(|current| !Arc::ptr_eq(current, &candidate))
    {
        return Err("A31 a different exact process decoder is registered".into());
    }
    *slot = Some(candidate);
    Ok(())
}

fn install_hook_set() -> Result<InstalledAddresses, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A31 install mutex poisoned")?;
    if INSTALLED.load(Ordering::Acquire) && LIFECYCLE_HEALTHY.load(Ordering::Acquire) {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    if HOOK_SET_PRESENT.load(Ordering::Acquire) {
        return Err("A31 hook set is lifecycle-poisoned; uninstall before retry".into());
    }
    let _ = decoder()?;
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A31 GetModuleHandleW(NULL) returned null".into());
    }
    let resolved = addresses(base);
    for (target, sentinel, label) in [
        (resolved.vector_publisher, &VECTOR_PUBLISHER_SENTINEL[..], "vector publisher"),
        (resolved.dynamic_refresh, &DYNAMIC_REFRESH_SENTINEL[..], "dynamic refresh"),
        (resolved.detail_refresh, &DETAIL_REFRESH_SENTINEL[..], "detail refresh"),
        (resolved.item_state_writer, &ITEM_STATE_WRITER_SENTINEL[..], "item state writer"),
        (resolved.item_state_producer, &ITEM_STATE_PRODUCER_SENTINEL[..], "item state producer"),
        (resolved.highlight_producer, &HIGHLIGHT_SENTINEL[..], "highlight producer"),
    ] {
        verify_sentinel(target, sentinel, label)?;
    }
    MODULE_BASE.store(base, Ordering::Release);
    TEARDOWN_ACTIVE.store(false, Ordering::Release);
    LIFECYCLE_HEALTHY.store(false, Ordering::Release);
    HOOK_SET_PRESENT.store(true, Ordering::Release);
    let specs = [
        (resolved.vector_publisher, hook_vector_publisher as *mut c_void, &ORIGINAL_VECTOR_PUBLISHER),
        (resolved.dynamic_refresh, hook_dynamic_refresh as *mut c_void, &ORIGINAL_DYNAMIC_REFRESH),
        (resolved.detail_refresh, hook_detail_refresh as *mut c_void, &ORIGINAL_DETAIL_REFRESH),
        (resolved.item_state_writer, hook_item_state_writer as *mut c_void, &ORIGINAL_ITEM_STATE_WRITER),
        (resolved.item_state_producer, hook_item_state_producer as *mut c_void, &ORIGINAL_ITEM_STATE_PRODUCER),
        (resolved.highlight_producer, hook_highlight as *mut c_void, &ORIGINAL_HIGHLIGHT),
    ];
    for (target, detour, original) in specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(trampoline) => original.store(trampoline as usize, Ordering::Release),
            Err(status) => return rollback_install(resolved, format!("A31 hook create failed at {target:#x}: {status}")),
        }
    }
    for target in targets(resolved) {
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            return rollback_install(resolved, format!("A31 hook enable failed at {target:#x}: {status}"));
        }
    }
    if let Err(error) = a729_focus_item_inline::install(
        base,
        inline_record_display_percent as *const c_void,
        hook_joint_country_renderer as *const c_void,
    ) {
        return rollback_install(
            resolved,
            format!("A31 inline instrumentation install failed: {error}"),
        );
    }
    *state().lock().map_err(|_| "A31 hook state poisoned")? = HookState::default();
    HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
    INSTALLED.store(true, Ordering::Release);
    LIFECYCLE_HEALTHY.store(true, Ordering::Release);
    Ok(resolved)
}

fn rollback_install(resolved: InstalledAddresses, reason: String) -> Result<InstalledAddresses, String> {
    TEARDOWN_ACTIVE.store(true, Ordering::Release);
    HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
    let removal = disable_and_remove(resolved);
    if removal.is_ok() && IN_FLIGHT.load(Ordering::Acquire) == 0 {
        clear_hook_state();
    }
    Err(match removal {
        Ok(()) => reason,
        Err(error) => format!("{reason}; rollback anomaly: {error}"),
    })
}

pub fn uninstall() -> Result<(), String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A31 install mutex poisoned")?;
    if !HOOK_SET_PRESENT.load(Ordering::Acquire) {
        return Ok(());
    }
    TEARDOWN_ACTIVE.store(true, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    LIFECYCLE_HEALTHY.store(false, Ordering::Release);
    HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
    let resolved = addresses(MODULE_BASE.load(Ordering::Acquire));
    disable_and_remove(resolved)?;
    let deadline = Instant::now() + QUIESCE_TIMEOUT;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let remaining = IN_FLIGHT.load(Ordering::Acquire);
    if remaining != 0 {
        return Err(format!("A31 teardown has {remaining} in-flight callbacks"));
    }
    a729_focus_item_inline::release_relays()?;
    clear_hook_state();
    Ok(())
}

fn clear_hook_state() {
    ORIGINAL_VECTOR_PUBLISHER.store(0, Ordering::Release);
    ORIGINAL_DYNAMIC_REFRESH.store(0, Ordering::Release);
    ORIGINAL_DETAIL_REFRESH.store(0, Ordering::Release);
    ORIGINAL_ITEM_STATE_WRITER.store(0, Ordering::Release);
    ORIGINAL_ITEM_STATE_PRODUCER.store(0, Ordering::Release);
    ORIGINAL_HIGHLIGHT.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    HOOK_SET_PRESENT.store(false, Ordering::Release);
    TEARDOWN_ACTIVE.store(false, Ordering::Release);
    if let Ok(mut state) = state().lock() {
        *state = HookState::default();
    }
    if let Some(slot) = DECODER.get() {
        if let Ok(mut slot) = slot.lock() {
            *slot = None;
        }
    }
}

fn disable_and_remove(resolved: InstalledAddresses) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = a729_focus_item_inline::restore_sites() {
        errors.push(format!("restore inline instrumentation: {error}"));
    }
    for target in targets(resolved) {
        match unsafe { inline_detour::disable(target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => errors.push(format!("disable {target:#x}: {status}")),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let deadline = Instant::now() + QUIESCE_TIMEOUT;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let remaining = IN_FLIGHT.load(Ordering::Acquire);
    if remaining != 0 {
        return Err(format!(
            "A31 teardown has {remaining} in-flight callbacks after disabling every entry boundary"
        ));
    }
    if let Err(error) = a729_focus_item_inline::release_relays() {
        return Err(format!("release inline relays: {error}"));
    }
    for target in targets(resolved) {
        match unsafe { inline_detour::remove(target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => errors.push(format!("remove {target:#x}: {status}")),
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

fn addresses(base: usize) -> InstalledAddresses {
    InstalledAddresses {
        vector_publisher: base + VECTOR_PUBLISHER_RVA,
        dynamic_refresh: base + DYNAMIC_REFRESH_RVA,
        detail_refresh: base + DETAIL_REFRESH_RVA,
        item_state_writer: base + ITEM_STATE_WRITER_RVA,
        item_state_producer: base + ITEM_STATE_PRODUCER_RVA,
        highlight_producer: base + HIGHLIGHT_PRODUCER_RVA,
        display_percent_callsite: base + a729_focus_item_inline::DISPLAY_PERCENT_CALLSITE_RVA,
        joint_country_callsite: base + a729_focus_item_inline::JOINT_COUNTRY_CALLSITE_RVA,
    }
}

/// Authenticate one relocated process address and return its canonical
/// image-base-0x140000000 evidence VA for the pointer-erasing validator.
pub fn authenticate_and_normalize_image_address(
    runtime_address: usize,
    evidence_address: usize,
) -> Result<usize, String> {
    const EVIDENCE_IMAGE_BASE: usize = 0x1400_0000_0;
    let base = MODULE_BASE.load(Ordering::Acquire);
    let rva = evidence_address
        .checked_sub(EVIDENCE_IMAGE_BASE)
        .ok_or("A31 evidence address is below the image base")?;
    let expected = base
        .checked_add(rva)
        .ok_or("A31 relocated image address overflowed")?;
    if base == 0 || runtime_address != expected {
        return Err("A31 runtime vtable does not match the relocated exact-a729 image".into());
    }
    Ok(evidence_address)
}
fn targets(value: InstalledAddresses) -> [usize; 6] {
    [value.vector_publisher, value.dynamic_refresh, value.detail_refresh, value.item_state_writer, value.item_state_producer, value.highlight_producer]
}

unsafe extern "system" fn hook_vector_publisher(a: usize, b: usize, c: usize, d: usize) {
    let _guard = CallbackGuard::enter();
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    let original = ORIGINAL_VECTOR_PUBLISHER.load(Ordering::Acquire);
    if original == 0 { return; }
    let original: VectorPublisherFn = unsafe { mem::transmute(original) };
    unsafe { original(a, b, c, d) };
    if !capture_gate_open() || HEALTH_EPOCH.load(Ordering::Acquire) != epoch { return; }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut state) = state().lock() {
            state.vector_publish_callbacks = state.vector_publish_callbacks.saturating_add(1);
            let conflicting_owner = state
                .active
                .as_ref()
                .and_then(|active| active.published_view_address)
                .is_some_and(|published| published != a);
            if conflicting_owner {
                state.fatal_error = Some(
                    "A31 armed epoch observed more than one focus-view owner".into(),
                );
            } else if let Some(active) = state.active.as_mut() {
                    active.vector_publish_seen = true;
                    active.published_view_address = Some(a);
            }
        }
    }));
}

unsafe extern "system" fn hook_item_state_writer(item: usize, code: u32) {
    let _guard = CallbackGuard::enter();
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    let original = ORIGINAL_ITEM_STATE_WRITER.load(Ordering::Acquire);
    if original == 0 { return; }
    let original: ItemStateWriterFn = unsafe { mem::transmute(original) };
    unsafe { original(item, code) };
    if !capture_gate_open() || HEALTH_EPOCH.load(Ordering::Acquire) != epoch { return; }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut state) = state().lock() {
            state.state_writer_callbacks = state.state_writer_callbacks.saturating_add(1);
            if code > 3 || item == 0 {
                state.fatal_error = Some("A31 state writer emitted invalid identity/domain".into());
            } else if let Some(active) = state.active.as_mut() {
                active.item_state_by_address.insert(item, code as u8);
            }
        }
    }));
}

unsafe extern "system" fn hook_item_state_producer(
    item: usize,
    country_context: usize,
    changed: u8,
    delta: f32,
    context: usize,
) {
    let _guard = CallbackGuard::enter();
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    let original = ORIGINAL_ITEM_STATE_PRODUCER.load(Ordering::Acquire);
    if original == 0 { return; }
    if capture_gate_open() {
        if let Ok(mut state) = state().lock() {
            let cycle_active = state
                .active
                .as_ref()
                .is_some_and(|active| active.transient_cycle_active);
            if cycle_active && item != 0 {
                if let Some(active) = state.active.as_mut() {
                    // This is the exact measured-hidden default.  The positive
                    // renderer callsite may refine it to Some(tag) before the
                    // producer returns.
                    active.joint_country_by_item.insert(item, None);
                }
            }
        }
    }
    let original: ItemStateProducerFn = unsafe { mem::transmute(original) };
    let _current_item = CurrentJointItemGuard::enter(item);
    unsafe { original(item, country_context, changed, delta, context) };
    if !capture_gate_open() || HEALTH_EPOCH.load(Ordering::Acquire) != epoch { return; }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut state) = state().lock() {
            state.state_producer_callbacks = state.state_producer_callbacks.saturating_add(1);
            let conflicting = state
                .active
                .as_ref()
                .is_some_and(|active| {
                    active.expected_country_object_address != country_context
                        || active
                            .country_context_address
                            .is_some_and(|prior| prior != country_context)
                });
            if item == 0 || country_context == 0 || conflicting {
                state.fatal_error = Some(
                    "A31 item producer country context differs from the authenticated owner"
                        .into(),
                );
            } else if let Some(active) = state.active.as_mut() {
                active.country_context_address = Some(country_context);
            }
        }
    }));
}

unsafe extern "system" fn hook_highlight(item: usize, state_code: u32) {
    let _guard = CallbackGuard::enter();
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    let original = ORIGINAL_HIGHLIGHT.load(Ordering::Acquire);
    if original == 0 { return; }
    let original: HighlightFn = unsafe { mem::transmute(original) };
    unsafe { original(item, state_code) };
    if !capture_gate_open() || HEALTH_EPOCH.load(Ordering::Acquire) != epoch { return; }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut state) = state().lock() {
            state.highlight_callbacks = state.highlight_callbacks.saturating_add(1);
            if state_code > 3 || item == 0 {
                state.fatal_error = Some("A31 highlight writer emitted invalid identity/domain".into());
            } else if let Some(active) = state.active.as_mut() {
                active.highlight_by_address.insert(item, state_code == 3);
            }
        }
    }));
}

unsafe extern "system" fn hook_dynamic_refresh(view: usize, delta: f32, context: usize) {
    let _guard = CallbackGuard::enter();
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    let original = ORIGINAL_DYNAMIC_REFRESH.load(Ordering::Acquire);
    if original == 0 { return; }
    if capture_gate_open() {
        if let Ok(mut state) = state().lock() {
            if let Some(active) = state.active.as_mut() {
                active.item_state_by_address.clear();
                active.highlight_by_address.clear();
                active.display_percent_by_definition.clear();
                active.joint_country_by_item.clear();
                active.pending_dynamic_view_address = Some(view);
                active.transient_cycle_active = true;
                active.candidate = None;
            }
        }
    }
    let original: DynamicRefreshFn = unsafe { mem::transmute(original) };
    unsafe { original(view, delta, context) };
    if !capture_gate_open() || HEALTH_EPOCH.load(Ordering::Acquire) != epoch { return; }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut state) = state().lock() {
            state.dynamic_refresh_callbacks = state.dynamic_refresh_callbacks.saturating_add(1);
        }
    }));
}

/// Exact frame callers update item state through `0x14137a780` and only then
/// update national/joint detail nodes through `0x14137a600`. Materialization
/// at the earlier return loses the display-percent and joint-country leaves.
/// Admit only the paired dynamic -> detail sequence for the same view.
unsafe extern "system" fn hook_detail_refresh(view: usize) {
    let _guard = CallbackGuard::enter();
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    let original = ORIGINAL_DETAIL_REFRESH.load(Ordering::Acquire);
    if original == 0 { return; }
    let original: DetailRefreshFn = unsafe { mem::transmute(original) };
    unsafe { original(view) };
    if !capture_gate_open() || HEALTH_EPOCH.load(Ordering::Acquire) != epoch { return; }
    let _ = catch_unwind(AssertUnwindSafe(|| capture_completed_refresh(view)));
}

fn capture_completed_refresh(view: usize) {
    let sequence = NEXT_CALLBACK_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    let Some((ticks, frequency)) = qpc_now() else { poison("A31 QPC read failed"); return; };
    let raw = {
        let mut state = match state().lock() { Ok(value) => value, Err(_) => return };
        state.detail_refresh_callbacks = state.detail_refresh_callbacks.saturating_add(1);
        let Some(active) = state.active.as_mut() else { return };
        if !consume_pending_dynamic_view(&mut active.pending_dynamic_view_address, view) {
            return;
        }
        active.transient_cycle_active = false;
        RawRefreshEpoch {
            generation: active.generation,
            callback_sequence: sequence,
            completion_qpc_ticks: ticks,
            qpc_frequency_hz: frequency,
            frame_hour: active.frame_hour,
            country_tag: active.country_tag.clone(),
            view_address: view,
            vector_publish_seen: active.vector_publish_seen,
            published_view_address: active.published_view_address,
            country_context_address: active.country_context_address,
            item_state_by_address: active.item_state_by_address.clone(),
            highlight_by_address: active.highlight_by_address.clone(),
            display_percent_by_definition: active.display_percent_by_definition.clone(),
            joint_country_by_item: active.joint_country_by_item.clone(),
        }
    };
    let decoder = match decoder() { Ok(value) => value, Err(error) => { poison(&error); return; } };
    let observation = match decoder.decode_completed_refresh(&raw) {
        Ok(value) => value,
        Err(error) => { poison(&format!("A31 refresh decode failed: {error}")); return; }
    };
    if observation.items.len() > MAX_ITEMS {
        poison("A31 refresh exceeded maximum item count");
        return;
    }
    let snapshot = match materialize_pointer_free_snapshot(&observation, &Boundary(decoder.as_ref())) {
        Ok(value) => value,
        Err(error) => { poison(&format!("A31 pointer-free materialization failed: {error}")); return; }
    };
    let mut focus_gui_items = Vec::with_capacity(observation.items.len());
    for (item, visible) in observation.items.iter().zip(&snapshot.visible_items) {
        let Some(root_address) = item
            .item_address
            .checked_add(FOCUS_ITEM_RUNTIME_GUI_ROOT_OFFSET)
        else {
            poison("A31 focus item GUI-root address overflowed");
            return;
        };
        let Some(runtime_gui_root_ptr) = read_process_usize(root_address) else {
            poison("A31 focus item GUI root was unreadable");
            return;
        };
        if runtime_gui_root_ptr == 0 || read_process_usize(runtime_gui_root_ptr).is_none() {
            poison("A31 focus item GUI root was null or had no readable vtable");
            return;
        }
        focus_gui_items.push(NationalFocusGuiItemRegistration {
            focus_id: visible.focus_id.clone(),
            item_ptr: item.item_address,
            runtime_gui_root_ptr,
        });
    }
    let Some(runtime_graphical_root_ptr) = observation
        .view_address
        .checked_add(FOCUS_GUI_SUBOBJECT_OFFSET)
    else {
        poison("A31 focus graphical subobject address overflowed");
        return;
    };
    let Some(runtime_root_address) = observation
        .view_address
        .checked_add(FOCUS_CANVAS_RUNTIME_ROOT_OFFSET)
    else {
        poison("A31 focus canvas runtime-root address overflowed");
        return;
    };
    let Some(runtime_root_ptr) = read_process_usize(runtime_root_address) else {
        poison("A31 focus canvas runtime root was unreadable");
        return;
    };
    if runtime_root_ptr == 0 || read_process_usize(runtime_root_ptr).is_none() {
        poison("A31 focus canvas runtime root was null or had no readable vtable");
        return;
    }
    let runtime_geometry =
        match sample_focus_gui_geometry(observation.view_address, runtime_root_ptr) {
            Ok(value) => value,
            Err(error) => {
                poison(&format!("A31 focus canvas geometry sampling failed: {error}"));
                return;
            }
        };
    let mut state = match state().lock() { Ok(value) => value, Err(_) => return };
    if state
        .active
        .as_ref()
        .map_or(true, |active| active.generation != raw.generation)
    {
        state.fatal_error = Some("A31 epoch changed during refresh decode".into());
        return;
    }
    let active = state.active.as_mut().expect("checked above");
    active.candidate = Some(Candidate {
        snapshot,
        focus_view_ptr: observation.view_address,
        runtime_root_ptr,
        runtime_graphical_root_ptr,
        runtime_geometry: runtime_geometry.clone(),
        focus_gui_items,
        same_epoch_vector_publish_seen: raw.vector_publish_seen,
        generation: raw.generation,
        callback_sequence: sequence,
        completion_qpc_ticks: runtime_geometry.sample_completed_qpc_ticks,
        qpc_frequency_hz: runtime_geometry.qpc_frequency_hz,
    });
}

/// Called by the exact `0x141bb636e` relay before the displaced
/// `mov edx,esi; mov rcx,rax` pair is replayed.  The relay supplies the
/// authenticated detail definition pointer from `R14+0xf50` and the final
/// percentage from ESI.
unsafe extern "system" fn inline_record_display_percent(
    definition_address: usize,
    percent: u32,
) {
    let _guard = CallbackGuard::enter();
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    if !capture_gate_open() || percent > 100 {
        return;
    }
    let generation = state()
        .lock()
        .ok()
        .and_then(|state| state.active.as_ref().map(|active| active.generation));
    let Some(generation) = generation else { return };
    let result = catch_unwind(AssertUnwindSafe(|| {
        record_display_percent_callsite(generation, definition_address, percent as u8)
    }));
    if HEALTH_EPOCH.load(Ordering::Acquire) != epoch {
        return;
    }
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => poison(&error),
        Err(_) => poison("A31 display-percent inline callback panicked"),
    }
}

/// ABI-compatible replacement for the one joint-country renderer call inside
/// `0x141379a50`.  R8 is a pointer to the exact tag DB id selected by the
/// producer; the current item is joined through the nest-restoring TLS slot.
unsafe extern "system" fn hook_joint_country_renderer(
    a: usize,
    b: usize,
    tag_db_id_address: usize,
    d: usize,
    e: usize,
    f: usize,
) {
    let _guard = CallbackGuard::enter();
    if capture_gate_open() {
        let item = CURRENT_JOINT_ITEM.with(Cell::get);
        let generation = state()
            .lock()
            .ok()
            .and_then(|state| state.active.as_ref().map(|active| active.generation));
        if item == 0 || tag_db_id_address == 0 || generation.is_none() {
            poison("A31 joint-country callsite lost its item/generation join");
        } else if let Some(tag_db_id) = read_process_u32(tag_db_id_address) {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let tag = decoder()?.stable_country_tag(tag_db_id)?;
                record_joint_country_observation(generation.unwrap(), item, Some(&tag))
            }));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => poison(&error),
                Err(_) => poison("A31 joint-country inline callback panicked"),
            }
        } else {
            poison("A31 joint-country DB id was unreadable");
        }
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let original: JointCountryRendererFn = unsafe {
        mem::transmute(base + a729_focus_item_inline::JOINT_COUNTRY_RENDERER_RVA)
    };
    unsafe { original(a, b, tag_db_id_address, d, e, f) };
}

fn read_process_u32(address: usize) -> Option<u32> {
    let mut value = 0u32;
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            (&mut value as *mut u32).cast(),
            core::mem::size_of::<u32>(),
            &mut read,
        )
    };
    (ok != 0 && read == core::mem::size_of::<u32>()).then_some(value)
}

fn read_process_usize(address: usize) -> Option<usize> {
    let mut value = 0usize;
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            (&mut value as *mut usize).cast(),
            core::mem::size_of::<usize>(),
            &mut read,
        )
    };
    (ok != 0 && read == core::mem::size_of::<usize>()).then_some(value)
}

fn runtime_content_extent(runtime_root: usize) -> Result<[i32; 2], String> {
    type ContentExtentFn = unsafe extern "system" fn(usize, *mut [i32; 2]) -> *const i32;
    let vtable = read_process_usize(runtime_root)
        .ok_or("A31 focus ContainerWindow vtable was unreadable")?;
    let target = vtable
        .checked_add(CONTAINER_CONTENT_EXTENT_VSLOT)
        .and_then(read_process_usize)
        .ok_or("A31 focus ContainerWindow content-extent getter was unreadable")?;
    if target == 0 {
        return Err("A31 focus ContainerWindow content-extent getter is null".into());
    }
    let getter: ContentExtentFn = unsafe { mem::transmute(target) };
    let mut extent = [0i32; 2];
    let returned = unsafe { getter(runtime_root, &mut extent) };
    if returned.is_null() || extent.iter().any(|value| *value <= 0) {
        return Err("A31 focus ContainerWindow returned an invalid content extent".into());
    }
    Ok(extent)
}

fn runtime_canvas_origin_pixels(focus_view: usize) -> Result<[i32; 2], String> {
    type GraphicalPositionFn = unsafe extern "system" fn(usize, *mut [i32; 2]) -> *const i32;
    let widget = focus_view
        .checked_add(FOCUS_CANVAS_POSITION_WIDGET_OFFSET)
        .and_then(read_process_usize)
        .ok_or("A31 focus canvas position widget was unreadable")?;
    let graphical = widget
        .checked_add(GRAPHICAL_POSITION_SUBOBJECT_OFFSET)
        .ok_or("A31 focus canvas graphical-position address overflowed")?;
    let vtable = read_process_usize(graphical)
        .ok_or("A31 focus canvas graphical-position vtable was unreadable")?;
    let target = vtable
        .checked_add(GRAPHICAL_POSITION_VSLOT)
        .and_then(read_process_usize)
        .ok_or("A31 focus canvas graphical-position getter was unreadable")?;
    if target == 0 {
        return Err("A31 focus canvas graphical-position getter is null".into());
    }
    let getter: GraphicalPositionFn = unsafe { mem::transmute(target) };
    let mut position = [0i32; 2];
    let returned = unsafe { getter(graphical, &mut position) };
    if returned.is_null() {
        return Err("A31 focus canvas graphical-position getter returned null".into());
    }
    Ok(position)
}

fn runtime_scrollbar_value(
    runtime_root: usize,
    scrollbar_offset: usize,
) -> Result<f32, String> {
    type ScrollbarValueFn = unsafe extern "system" fn(usize, u8) -> f32;
    let scrollbar = runtime_root
        .checked_add(scrollbar_offset)
        .and_then(read_process_usize)
        .ok_or("A31 focus ContainerWindow scrollbar was unreadable")?;
    let value_subobject = scrollbar
        .checked_add(SCROLLBAR_VALUE_SUBOBJECT_OFFSET)
        .ok_or("A31 focus scrollbar value-subobject address overflowed")?;
    let vtable = read_process_usize(value_subobject)
        .ok_or("A31 focus scrollbar value vtable was unreadable")?;
    let target = vtable
        .checked_add(SCROLLBAR_VALUE_VSLOT)
        .and_then(read_process_usize)
        .ok_or("A31 focus scrollbar value getter was unreadable")?;
    if target == 0 {
        return Err("A31 focus scrollbar value getter is null".into());
    }
    let getter: ScrollbarValueFn = unsafe { mem::transmute(target) };
    let value = unsafe { getter(value_subobject, 1) };
    if !value.is_finite() {
        return Err("A31 focus scrollbar returned a non-finite value".into());
    }
    Ok(value)
}

fn exact_fixed5(value: f32) -> Result<i64, String> {
    let scaled = f64::from(value) * 32.0;
    if !scaled.is_finite() || scaled.fract() != 0.0 {
        return Err("A31 focus runtime value is not exactly representable in fixed5".into());
    }
    if scaled < i64::MIN as f64 || scaled > i64::MAX as f64 {
        return Err("A31 focus runtime fixed5 value exceeds i64".into());
    }
    Ok(scaled as i64)
}

fn consume_pending_dynamic_view(pending: &mut Option<usize>, detail_view: usize) -> bool {
    if *pending != Some(detail_view) {
        return false;
    }
    *pending = None;
    true
}

fn poison(message: &str) {
    if let Ok(mut state) = state().lock() {
        state.fatal_error = Some(message.into());
    }
}

fn qpc_now() -> Option<(u64, u64)> {
    let mut ticks = 0i64;
    let mut frequency = 0i64;
    if unsafe { QueryPerformanceCounter(&mut ticks) } == 0
        || unsafe { QueryPerformanceFrequency(&mut frequency) } == 0
        || ticks <= 0
        || frequency <= 0
    {
        None
    } else {
        Some((ticks as u64, frequency as u64))
    }
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            actual.as_mut_ptr().cast(),
            actual.len(),
            &mut read,
        )
    };
    if ok == 0 || read != actual.len() || actual != expected {
        return Err(format!("A31 exact-a729 {label} sentinel mismatch at {address:#x}"));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Helper for the collector's SEH reader: constructs the exact observation
/// after it has independently double-read and authenticated every pointer.
pub fn assemble_process_observation(
    raw: &RawRefreshEpoch,
    source_sequence: u64,
    view_vtable: usize,
    opening_header: VectorHeader,
    opening_item_addresses: Vec<usize>,
    mut items: Vec<ProcessLocalItemObservation>,
    closing_item_addresses: Vec<usize>,
    closing_header: VectorHeader,
) -> Result<ProcessLocalFrameObservation, String> {
    for item in &mut items {
        item.presentation_state_code = *raw
            .item_state_by_address
            .get(&item.item_address)
            .ok_or("A31 state-writer leaf was missed")?;
        item.highlight_glow_visible = raw.highlight_by_address.get(&item.item_address).copied();
        item.display_progress_percent = raw
            .display_percent_by_definition
            .get(&item.definition_address)
            .copied();
        item.joint_viewing_country_tag = raw
            .joint_country_by_item
            .get(&item.item_address)
            .cloned();
    }
    Ok(ProcessLocalFrameObservation {
        source_sequence,
        frame_hour: raw.frame_hour,
        country_tag: raw.country_tag.clone(),
        view_address: raw.view_address,
        view_vtable,
        opening_header,
        opening_item_addresses,
        items,
        closing_item_addresses,
        closing_header,
        rebuild_completed: raw.vector_publish_seen,
        dynamic_refresh_completed: true,
        transient_epoch_completed: true,
    })
}

#[cfg(test)]
mod frame_order_tests {
    use super::{cancel_active_epoch, consume_pending_dynamic_view, state};

    #[test]
    fn detail_refresh_requires_and_consumes_the_same_pending_view() {
        let mut pending = None;
        assert!(!consume_pending_dynamic_view(&mut pending, 0x1000));

        pending = Some(0x2000);
        assert!(!consume_pending_dynamic_view(&mut pending, 0x3000));
        assert_eq!(pending, Some(0x2000));

        assert!(consume_pending_dynamic_view(&mut pending, 0x2000));
        assert_eq!(pending, None);
        assert!(!consume_pending_dynamic_view(&mut pending, 0x2000));
    }

    #[test]
    fn campaign_cancel_clears_unsealed_epoch_error_state() {
        {
            let mut state = state().lock().expect("A31 test state");
            state.fatal_error = Some("old campaign".into());
        }
        cancel_active_epoch().expect("cancel old campaign epoch");
        let state = state().lock().expect("A31 test state");
        assert!(state.active.is_none());
        assert!(state.admitted.is_none());
        assert!(state.fatal_error.is_none());
    }
}
