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
    VECTOR_PUBLISHER_RVA,
};
use crate::a729_focus_item_inline;
use compact_contract::national_focus_presentation::{
    encode_payload, validate_snapshot, AuthorityState, Frame, SelectedPresentBinding, Snapshot,
    EXACT20_FUNCTION_MANIFEST_SHA256, EXACT20_JOB_RECEIPT_SHA256, EXECUTABLE_SHA256,
};
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::collections::BTreeMap;
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

/// The exact passive observer is admitted by executable identity, sentinels,
/// complete callback witnesses and a selected-Present frontier. A missing
/// refresh remains typed unavailable; fixture transitions are regression-only.
pub const PRODUCTION_ACTIVATION_AUTHORIZED: bool = true;
pub const COMPACT_DOMAIN_89_SCHEDULED: bool = true;

const MAX_ITEMS: usize = 131_072;
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
    generation: u64,
    selected: SelectedPresentStamp,
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
    let mut state = state().lock().map_err(|_| "A31 hook state poisoned")?;
    if let Some(error) = &state.fatal_error {
        return Err(error.clone());
    }
    let active = state.active.take().ok_or("A31 no fixture epoch is active")?;
    if active.generation != generation || active.frame_hour != selected.frame_hour {
        state.fatal_error = Some("A31 fixture and selected-Present identities differ".into());
        return Err(state.fatal_error.clone().unwrap());
    }
    let candidate = active
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
    let receipt = AcquisitionReceipt {
        schema: "hoi4.a31.selected_present_acquisition_receipt.v2",
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
        selected_present: Some(SelectedPresentBinding {
            materialization_qpc_ticks: receipt.completion_qpc_ticks,
            selected_present_global_order: receipt.selected_present_global_order,
            selected_present_qpc_ticks: receipt.selected_present_qpc_ticks,
            qpc_frequency_hz: receipt.qpc_frequency_hz,
            selected_present_receipt_sha256: receipt
                .portable_selected_present_receipt_sha256
                .clone(),
        }),
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
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
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
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
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
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
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
        same_epoch_vector_publish_seen: raw.vector_publish_seen,
        generation: raw.generation,
        callback_sequence: sequence,
        completion_qpc_ticks: ticks,
        qpc_frequency_hz: frequency,
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
