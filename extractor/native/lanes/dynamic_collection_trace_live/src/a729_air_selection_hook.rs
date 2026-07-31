//! Passive exact-a729 observer for the quick-wing air-selection collection.
//!
//! upstream owner candidate universe to the selected/sorted 32-byte bucket,
//! concrete row pool, standard list traversal, resolved geometry, and fixed
//! scrollbar state.  The retained record is pointer-free.  Any unreadable
//! field, unexpected vtable, predicate disagreement, resolver mismatch, or
//! incomplete traversal poisons the owner; no measured-empty substitute is
//! emitted.

use crate::a729_air_mio_controller::{
    compute_identity_record_sha256, A729AirMioControllerSource, AirMioAuthorityReceipt,
    AirMioControllerKind, AirMioControllerObservation, ControllerCandidate,
    ControllerCandidateKey, ControllerSlot, ControllerSourceLane, PortableItemIdentity,
    PortableObjectResolverRecord, RejectionEvidence, ResolvedListState, ResolverInputSource,
    RuntimeRowTemplate, AIR_BUCKET_1_BYTE_5B4_RVA, AIR_BUCKET_2_BIT_20_RVA,
    AIR_BUCKET_3_TYPE_3_VALUE_RVA, AIR_COMMON_ARCHETYPE_GATE_RVA, AIR_COMMON_BIT_33_RVA,
    AIR_COMMON_EXCLUDED_MASK_RVA, AIR_COMMON_MASK_ZERO_RVA, AIR_PASSIVE_HOOK_JOB_ID,
    AIR_PASSIVE_HOOK_MANIFEST_SHA256, A729_BINARY_ID, A729_EXECUTABLE_SHA256,
    PAIR_TO_OBJECT_RESOLVER_JOB_ID, PAIR_TO_OBJECT_RESOLVER_MANIFEST_SHA256,
    PAIR_TO_OBJECT_RESOLVER_RVA, REFERENCE_TABLE_LOOKUP_RVA,
};
use crate::RectI32;
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const AIR_MATERIALIZE_RVA: usize = 0x01F4_A980;
pub const AIR_MATERIALIZE_HOOK_JOB_ID: &str = AIR_PASSIVE_HOOK_JOB_ID;
pub const AIR_MATERIALIZE_HOOK_MANIFEST_SHA256: &str = AIR_PASSIVE_HOOK_MANIFEST_SHA256;

const CONTROLLER_WRAPPER_OFFSET: usize = 0x5A0;
const CONTROLLER_ROW_VECTOR_OFFSET: usize = 0x5B0;
const CONTROLLER_ROW_COUNT_OFFSET: usize = 0x5BC;
const CONTROLLER_LIST_OFFSET: usize = 0x5C8;
const WRAPPER_BUILDER_OFFSET: usize = 0x10;
const WRAPPER_BUCKET_OFFSET: usize = 0x38;
const BUILDER_OWNER_OFFSET: usize = 0x28;
const BUILDER_ARCHETYPE_CONTEXT_OFFSET: usize = 0x30;
const OWNER_CANDIDATES_OFFSET: usize = 0x20;
const OWNER_CANDIDATE_COUNT_OFFSET: usize = 0x2C;
const OPTION_STRIDE: usize = 0x20;
const OPTION_EQUIPMENT_OFFSET: usize = 0x10;
const OBJECT_SELF_REFERENCE_OFFSET: usize = 0x08;
const OBJECT_FLAGS_OFFSET: usize = 0x5A8;
const OBJECT_BUCKET_1_BYTE_OFFSET: usize = 0x5B4;
const OBJECT_ARCHETYPE_INPUT_OFFSET: usize = 0x310;

const LIST_HEAD_OFFSET: usize = 0x90;
const LIST_COUNT_OFFSET: usize = 0xA0;
const LIST_SCROLLBAR_OFFSET: usize = 0x190;
const NODE_ITEM_OFFSET: usize = 0;
const NODE_NEXT_OFFSET: usize = 0x10;
const ROW_GUI_OFFSET: usize = 0x38;
const ROW_OPTION_OFFSET: usize = 0x50;
const SCROLL_MIN_OFFSET: usize = 0x1558;
const SCROLL_MAX_OFFSET: usize = 0x1560;
const SCROLL_TARGET_OFFSET: usize = 0x15F8;
const SCROLL_CURRENT_OFFSET: usize = 0x1600;

const STANDARD_LIST_PRIMARY_VTABLE_RVA: usize = 0x02B2_CB48;
const STANDARD_LIST_SECONDARY_VTABLE_RVA: usize = 0x02B2_CC78;
const AIR_ROW_PRIMARY_VTABLE_RVA: usize = 0x02A9_1800;
const AIR_ROW_GUI_VTABLE_RVA: usize = 0x02A9_1870;
const EXTENDED_SCROLLBAR_PRIMARY_VTABLE_RVA: usize = 0x02B3_2FF0;
const EXTENDED_SCROLLBAR_PLUS_80_VTABLE_RVA: usize = 0x02B3_3270;
const EXTENDED_SCROLLBAR_PLUS_B0_VTABLE_RVA: usize = 0x02B3_3300;

const MAX_PENDING: usize = 4_096;
const MAX_ROWS: usize = crate::MAX_ITEMS_PER_COLLECTION;
const AIR_MATERIALIZE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x6C, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];

type AirMaterializeFn = unsafe extern "system" fn(usize);
type PairResolverFn = unsafe extern "system" fn(*const u32) -> usize;
type ArchetypeGateFn = unsafe extern "system" fn(usize, usize) -> u8;
type TypeThreeValueFn = unsafe extern "system" fn(usize, i32) -> i32;
type GeometryFn = unsafe extern "system" fn(usize, *mut i32) -> *mut i32;

static ORIGINAL_MATERIALIZE: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static INSTALL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static GAME_CLOCK_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

#[derive(Default)]
struct HookState {
    controller_calls: u64,
    transition_sequence: u64,
    observations: VecDeque<AirMioControllerObservation>,
    poison: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirSelectionHookDiagnostics {
    pub installed: bool,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub controller_calls: u64,
    pub transition_sequence: u64,
    pub pending_observations: usize,
    pub poisoned: bool,
}

pub struct A729AirSelectionHookOwner {
    target: usize,
    epoch: usize,
    active: bool,
}

impl A729AirSelectionHookOwner {
    pub fn install(game_clock_address: usize) -> Result<Self, String> {
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 air install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("A28 air hook already has a live owner".into());
        }
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        let base = module as usize;
        if base == 0 {
            return Err("GetModuleHandleW(NULL) returned null".into());
        }
        verify_executable_sha256(module)?;
        verify_sentinel(
            base + compact_contract::build_profile::C_GAME_STATE_DAILY_UPDATE_OFFSET,
            &compact_contract::build_profile::DAILY_UPDATE_SENTINEL,
            "A28 air daily-update anchor",
        )?;
        verify_sentinel(
            base + compact_contract::build_profile::GET_LONG_STRING_OFFSET,
            &compact_contract::build_profile::GET_LONG_STRING_SENTINEL,
            "A28 air long-string anchor",
        )?;
        let exact_clock = read_usize(base + compact_contract::build_profile::CG_STATE_DATE_OFFSET)
            .ok_or("A28 air exact game-clock root is unreadable")?;
        if game_clock_address != exact_clock {
            return Err("A28 air game-clock address differs from the exact a729 root".into());
        }
        if game_clock_address == 0 || read_i32(game_clock_address).is_none() {
            return Err("A28 air game-clock address is unreadable".into());
        }
        let target = base + AIR_MATERIALIZE_RVA;
        verify_sentinel(target, &AIR_MATERIALIZE_SENTINEL, "air materializer")?;
        reset_state()?;
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| value.checked_add(1))
            .map_err(|_| "A28 air install epoch exhausted".to_string())?
            + 1;
        let original = unsafe {
            MinHook::create_hook(target as *mut c_void, hook_air_materialize as *mut c_void)
        }
        .map_err(|status| format!("A28 air hook creation failed: {status}"))?;
        ORIGINAL_MATERIALIZE.store(original as usize, Ordering::Release);
        MODULE_BASE.store(base, Ordering::Release);
        GAME_CLOCK_ADDRESS.store(game_clock_address, Ordering::Release);
        CALLBACKS_OPEN.store(true, Ordering::Release);
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            CALLBACKS_OPEN.store(false, Ordering::Release);
            let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
            ORIGINAL_MATERIALIZE.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            GAME_CLOCK_ADDRESS.store(0, Ordering::Release);
            return Err(format!("A28 air hook enable failed: {status}"));
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(Self { target, epoch, active: true })
    }

    pub fn diagnostics(&self) -> Result<AirSelectionHookDiagnostics, String> {
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 air hook owner epoch is stale".into());
        }
        diagnostics()
    }

    pub fn drain_observations(&self) -> Result<Vec<AirMioControllerObservation>, String> {
        self.require_active()?;
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 air hook poisoned: {error}"));
        }
        Ok(state.observations.drain(..).collect())
    }

    /// The carrier admits the latest collection observed during this exact raw
    /// game hour. It never relabels an earlier-hour callback as current.
    pub fn drain_latest_source(
        &self,
        game_hour_raw: i32,
    ) -> Result<A729AirMioControllerSource, String> {
        self.require_active()?;
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 air hook poisoned: {error}"));
        }
        let latest = state
            .observations
            .pop_back()
            .ok_or("A28 air hook has no post-return observation")?;
        state.observations.clear();
        drop(state);
        if latest.authority.captured_game_hour_raw != game_hour_raw {
            return Err(format!(
                "A28 air observation raw hour {} differs from drain hour {game_hour_raw}",
                latest.authority.captured_game_hour_raw
            ));
        }
        let frame_hour = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
            .and_then(|hour| u64::try_from(hour).ok())
            .ok_or("A28 air captured game hour is not a compact calendar index")?;
        A729AirMioControllerSource::admit_hook_owned_air(frame_hour, vec![latest])
    }

    fn require_active(&self) -> Result<(), String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A28 air hook owner is not active".into());
        }
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 air hook owner epoch is stale".into());
        }
        Ok(())
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 air install mutex poisoned".to_string())?;
        CALLBACKS_OPEN.store(false, Ordering::Release);
        match unsafe { MinHook::disable_hook(self.target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A28 air hook disable failed: {status}")),
        }
        wait_for_quiescence(Duration::from_secs(30))?;
        match unsafe { MinHook::remove_hook(self.target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A28 air hook removal failed: {status}")),
        }
        ORIGINAL_MATERIALIZE.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        GAME_CLOCK_ADDRESS.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        self.active = false;
        Ok(())
    }
}

impl Drop for A729AirSelectionHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

unsafe extern "system" fn hook_air_materialize(controller: usize) {
    // Count every detour entry, including an invocation that observes
    // CALLBACKS_OPEN=false during uninstall. The original trampoline remains
    // in use until this guard drops, so removal can wait for exact quiescence.
    let _in_flight = InFlight::enter();
    call_original(controller);
    if !CALLBACKS_OPEN.load(Ordering::Acquire) {
        return;
    }
    if let Err(error) = capture_post_return(controller) {
        poison(error);
    }
}

unsafe fn call_original(controller: usize) {
    let address = ORIGINAL_MATERIALIZE.load(Ordering::Acquire);
    if address != 0 {
        let original: AirMaterializeFn = mem::transmute(address);
        original(controller);
    }
}

fn capture_post_return(controller: usize) -> Result<(), String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("module base absent during A28 air callback".into());
    }
    let captured_game_hour_raw = read_i32(GAME_CLOCK_ADDRESS.load(Ordering::Acquire))
        .ok_or("A28 air callback game clock unreadable")?;
    let wrapper = read_usize(controller + CONTROLLER_WRAPPER_OFFSET)
        .ok_or("air wrapper pointer unreadable")?;
    let builder = read_usize(wrapper + WRAPPER_BUILDER_OFFSET)
        .ok_or("air builder pointer unreadable")?;
    let bucket = read_i32(wrapper + WRAPPER_BUCKET_OFFSET).ok_or("air bucket unreadable")?;
    let (lane, bucket_vector_offset) = bucket_law(bucket)?;
    let owner = read_usize(builder + BUILDER_OWNER_OFFSET).ok_or("air owner pointer unreadable")?;
    let source = read_usize(owner + OWNER_CANDIDATES_OFFSET)
        .ok_or("air candidate vector unreadable")?;
    let source_count = bounded_count(
        read_i32(owner + OWNER_CANDIDATE_COUNT_OFFSET).ok_or("air candidate count unreadable")?,
        "air candidate",
    )?;
    let archetype_context = read_usize(builder + BUILDER_ARCHETYPE_CONTEXT_OFFSET)
        .ok_or("air archetype context unreadable")?;

    let mut candidates = Vec::with_capacity(source_count);
    let mut keys_by_pair = BTreeMap::new();
    for ordinal in 0..source_count {
        let object = read_usize(source + ordinal * mem::size_of::<usize>())
            .ok_or_else(|| format!("air candidate pointer {ordinal} unreadable"))?;
        let (pair, resolver) = resolve_object(base, object)?;
        let key = ControllerCandidateKey {
            source_lane: lane,
            source_instance_key: ((pair.0 as u64) << 32) | pair.1 as u64,
            identity: PortableItemIdentity::ReferencedObject {
                object_type: pair.0,
                stable_id: pair.1,
            },
        };
        if keys_by_pair.insert(pair, key).is_some() {
            return Err(format!("air candidate identity {:?} is duplicated", pair));
        }
        let rejection = classify_candidate(base, object, archetype_context, lane, pair)?;
        let identity_record_sha256 =
            compute_identity_record_sha256(AirMioControllerKind::AirSelection, &resolver);
        candidates.push(ControllerCandidate {
            key,
            identity_resolver: resolver,
            identity_record_sha256,
            source_ordinal: ordinal as u32,
            rejection,
        });
    }

    let selected_vector = read_usize(builder + bucket_vector_offset)
        .ok_or("air selected bucket vector unreadable")?;
    let selected_count = bounded_count(
        read_i32(builder + bucket_vector_offset + 0x0C)
            .ok_or("air selected bucket count unreadable")?,
        "air selected bucket",
    )?;
    let mut accepted_order = Vec::with_capacity(selected_count);
    let mut selected_options = Vec::with_capacity(selected_count);
    let mut selected_pairs = BTreeSet::new();
    for ordinal in 0..selected_count {
        let option = selected_vector + ordinal * OPTION_STRIDE;
        let object = read_usize(option + OPTION_EQUIPMENT_OFFSET)
            .ok_or_else(|| format!("air selected option {ordinal} object unreadable"))?;
        let pair = read_pair(object + OBJECT_SELF_REFERENCE_OFFSET)
            .ok_or_else(|| format!("air selected option {ordinal} identity unreadable"))?;
        let (resolved_pair, _) = resolve_object(base, object)?;
        if resolved_pair != pair {
            return Err(format!(
                "air selected option {ordinal} resolver identity differs"
            ));
        }
        if !selected_pairs.insert(pair) {
            return Err(format!("air selected identity {:?} is duplicated", pair));
        }
        let key = *keys_by_pair
            .get(&pair)
            .ok_or_else(|| format!("air selected identity {:?} is absent from source", pair))?;
        accepted_order.push(key);
        selected_options.push(option);
    }
    let expected_included = candidates
        .iter()
        .filter(|candidate| candidate.rejection.is_none())
        .map(|candidate| candidate.key)
        .collect::<BTreeSet<_>>();
    if accepted_order.iter().copied().collect::<BTreeSet<_>>() != expected_included {
        return Err("air selected bucket differs from authenticated source dispositions".into());
    }

    let row_vector = read_usize(controller + CONTROLLER_ROW_VECTOR_OFFSET)
        .ok_or("air row vector unreadable")?;
    let row_count = bounded_count(
        read_i32(controller + CONTROLLER_ROW_COUNT_OFFSET).ok_or("air row count unreadable")?,
        "air row",
    )?;
    if row_count != selected_count {
        return Err("air row count differs from selected bucket count".into());
    }
    let list = read_usize(controller + CONTROLLER_LIST_OFFSET).ok_or("air list unreadable")?;
    gate_vtable(list, base + STANDARD_LIST_PRIMARY_VTABLE_RVA, "air list primary")?;
    gate_vtable(list + 8, base + STANDARD_LIST_SECONDARY_VTABLE_RVA, "air list secondary")?;
    let list_count = bounded_count(
        read_i32(list + LIST_COUNT_OFFSET).ok_or("air list count unreadable")?,
        "air list",
    )?;
    if list_count != selected_count {
        return Err("air list traversal count differs from selected bucket count".into());
    }

    let mut slots = Vec::with_capacity(row_count);
    let mut slot_ids = BTreeSet::new();
    let mut node = read_usize(list + LIST_HEAD_OFFSET).ok_or("air list head unreadable")?;
    for ordinal in 0..row_count {
        if node == 0 {
            return Err(format!("air list chain ended at ordinal {ordinal}"));
        }
        let row = read_usize(row_vector + ordinal * mem::size_of::<usize>())
            .ok_or_else(|| format!("air row {ordinal} unreadable"))?;
        gate_vtable(row, base + AIR_ROW_PRIMARY_VTABLE_RVA, "air row primary")?;
        let row_gui = row + ROW_GUI_OFFSET;
        gate_vtable(row_gui, base + AIR_ROW_GUI_VTABLE_RVA, "air row GUI")?;
        let node_gui = read_usize(node + NODE_ITEM_OFFSET)
            .ok_or_else(|| format!("air list node {ordinal} item unreadable"))?;
        if node_gui != row_gui {
            return Err(format!("air row/list traversal mismatch at ordinal {ordinal}"));
        }
        let row_option = read_usize(row + ROW_OPTION_OFFSET)
            .ok_or_else(|| format!("air row {ordinal} source option unreadable"))?;
        if row_option != selected_options[ordinal] {
            return Err(format!("air row binding mismatch at ordinal {ordinal}"));
        }
        let key = accepted_order[ordinal];
        if !matches!(key.identity, PortableItemIdentity::ReferencedObject { .. }) {
            return Err("air row carried a non-reference identity".into());
        }
        // The controller destroys/rebuilds its row vector on every call. This
        // is a frame-local one-based materialization ordinal, not a claim of
        // cross-frame pool identity and never a transient process address.
        let stable_slot_id = u32::try_from(ordinal)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or("air stable slot ordinal overflow")?;
        if stable_slot_id == 0 || !slot_ids.insert(stable_slot_id) {
            return Err("air stable slot identity is zero or duplicated".into());
        }
        let position = invoke_geometry(row_gui, 0x78, "air row resolved position")?;
        let extent = invoke_geometry(row_gui, 0x28, "air row resolved extent")?;
        require_positive_extent(extent, "air row")?;
        slots.push(ControllerSlot {
            stable_slot_id,
            bound_candidate: key,
            template: RuntimeRowTemplate::AirPlaneTypeSelectionEntry,
            rect: RectI32 {
                x: position[0],
                y: position[1],
                width: extent[0],
                height: extent[1],
            },
        });
        node = read_usize(node + NODE_NEXT_OFFSET).ok_or("air next list link unreadable")?;
    }
    if node != 0 {
        return Err("air list chain contains more nodes than authenticated count".into());
    }

    let list_interface = list + 8;
    let list_position = invoke_geometry(list_interface, 0xA0, "air list position")?;
    let viewport = invoke_geometry(list_interface, 0x30, "air list viewport")?;
    let content = invoke_geometry(list_interface, 0xE0, "air list content")?;
    require_positive_extent(viewport, "air viewport")?;
    if content[0] < 0 || content[1] < 0 {
        return Err("air list content extent is negative".into());
    }
    let scrollbar = read_usize(list + LIST_SCROLLBAR_OFFSET).ok_or("air scrollbar unreadable")?;
    gate_vtable(scrollbar, base + EXTENDED_SCROLLBAR_PRIMARY_VTABLE_RVA, "air scrollbar")?;
    gate_vtable(
        scrollbar + 0x80,
        base + EXTENDED_SCROLLBAR_PLUS_80_VTABLE_RVA,
        "air scrollbar +0x80",
    )?;
    gate_vtable(
        scrollbar + 0xB0,
        base + EXTENDED_SCROLLBAR_PLUS_B0_VTABLE_RVA,
        "air scrollbar +0xB0",
    )?;
    let list_state = ResolvedListState {
        clip_rect: RectI32 {
            x: list_position[0],
            y: list_position[1],
            width: viewport[0],
            height: viewport[1],
        },
        content_extent_pixels: (content[0], content[1]),
        viewport_extent_pixels: (viewport[0], viewport[1]),
        scroll_min_fixed: read_i64(scrollbar + SCROLL_MIN_OFFSET)
            .ok_or("air scroll minimum unreadable")?,
        scroll_max_fixed: read_i64(scrollbar + SCROLL_MAX_OFFSET)
            .ok_or("air scroll maximum unreadable")?,
        scroll_current_fixed: read_i64(scrollbar + SCROLL_CURRENT_OFFSET)
            .ok_or("air scroll current unreadable")?,
        scroll_target_fixed: read_i64(scrollbar + SCROLL_TARGET_OFFSET)
            .ok_or("air scroll target unreadable")?,
    };
    validate_scroll(&list_state)?;

    let qpc_ticks = qpc()?;
    let producer_record_sha256 = producer_digest(bucket, &candidates, &accepted_order, &slots, &list_state);
    let mut state = state_lock()?;
    state.controller_calls = state.controller_calls.saturating_add(1);
    state.transition_sequence = state
        .transition_sequence
        .checked_add(1)
        .ok_or("A28 air transition sequence exhausted")?;
    let transition_sequence = state.transition_sequence;
    state.observations.push_back(AirMioControllerObservation {
        kind: AirMioControllerKind::AirSelection,
        authority: AirMioAuthorityReceipt {
            binary_id: A729_BINARY_ID.into(),
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            function_job_id: AIR_PASSIVE_HOOK_JOB_ID.into(),
            function_manifest_sha256: AIR_PASSIVE_HOOK_MANIFEST_SHA256.into(),
            producer_rvas: vec![
                0x01F4_A980, 0x01CD_B770, 0x0195_1BA0, 0x0195_1A10, 0x0195_2420,
                0x00E5_05E0, 0x0100_5DD0, 0x01FC_49D0,
            ],
            transition_sequence,
            observation_qpc_ticks: qpc_ticks,
            captured_game_hour_raw,
            owner_instance_key: 0,
            producer_record_sha256,
            post_return: true,
            candidate_universe_complete: true,
            rejection_predicates_complete: true,
            stable_identity_complete: true,
            list_state_complete: true,
        },
        candidates,
        accepted_order,
        slots,
        list_state,
    });
    if state.observations.len() > MAX_PENDING {
        state.observations.pop_front();
    }
    Ok(())
}

fn bucket_law(bucket: i32) -> Result<(ControllerSourceLane, usize), String> {
    match bucket {
        0 => Ok((ControllerSourceLane::AirBucket0, 0x38)),
        1 => Ok((ControllerSourceLane::AirBucket1, 0x50)),
        2 => Ok((ControllerSourceLane::AirBucket2, 0x68)),
        3 => Ok((ControllerSourceLane::AirBucket3, 0x80)),
        _ => Err(format!("air wrapper selected unsupported bucket {bucket}")),
    }
}

fn resolve_object(base: usize, object: usize) -> Result<((u32, u32), PortableObjectResolverRecord), String> {
    let pair = read_pair(object + OBJECT_SELF_REFERENCE_OFFSET)
        .ok_or("air object self-reference unreadable")?;
    if pair.0 == 0 || pair.1 == 0 {
        return Err("air object self-reference contains zero".into());
    }
    let raw = [pair.0, pair.1];
    let resolver: PairResolverFn = unsafe { mem::transmute(base + PAIR_TO_OBJECT_RESOLVER_RVA as usize) };
    let resolved = unsafe { resolver(raw.as_ptr()) };
    if resolved == 0 || resolved != object {
        return Err("air pair resolver did not return the source object".into());
    }
    let resolved_pair = read_pair(resolved + OBJECT_SELF_REFERENCE_OFFSET)
        .ok_or("air resolved object self-reference unreadable")?;
    if resolved_pair != pair {
        return Err("air pair resolver round trip differs".into());
    }
    Ok((pair, PortableObjectResolverRecord {
        source: ResolverInputSource::AirOptionEquipmentSelfReference,
        resolver_rva: PAIR_TO_OBJECT_RESOLVER_RVA,
        lookup_rva: REFERENCE_TABLE_LOOKUP_RVA,
        resolver_job_id: PAIR_TO_OBJECT_RESOLVER_JOB_ID.into(),
        resolver_manifest_sha256: PAIR_TO_OBJECT_RESOLVER_MANIFEST_SHA256.into(),
        raw_object_type: pair.0,
        raw_stable_id: pair.1,
        resolved_object_type: resolved_pair.0,
        resolved_stable_id: resolved_pair.1,
        resolved_non_null: true,
        round_trip_self_reference_matches: true,
    }))
}

fn classify_candidate(
    base: usize,
    object: usize,
    archetype_context: usize,
    lane: ControllerSourceLane,
    pair: (u32, u32),
) -> Result<Option<RejectionEvidence>, String> {
    let flags = read_u64(object + OBJECT_FLAGS_OFFSET).ok_or("air candidate flags unreadable")?;
    let byte_5b4 = read_u8(object + OBJECT_BUCKET_1_BYTE_OFFSET)
        .ok_or("air candidate bucket byte unreadable")?;
    let (predicate_rva, reason, helper_value) = if flags & 0x1F00_37FC00 == 0 {
        (AIR_COMMON_MASK_ZERO_RVA, "common_air_mask_zero", 0)
    } else if flags & 0x1C00_010000 != 0 {
        (AIR_COMMON_EXCLUDED_MASK_RVA, "common_excluded_mask_nonzero", 0)
    } else if flags & (1u64 << 33) != 0 {
        (AIR_COMMON_BIT_33_RVA, "common_bit_33_set", 0)
    } else {
        let gate: ArchetypeGateFn = unsafe { mem::transmute(base + AIR_COMMON_ARCHETYPE_GATE_RVA as usize) };
        let gate_value = unsafe { gate(archetype_context, object + OBJECT_ARCHETYPE_INPUT_OFFSET) };
        if gate_value == 0 {
            (AIR_COMMON_ARCHETYPE_GATE_RVA, "common_archetype_gate_false", gate_value as i32)
        } else {
            match lane {
                ControllerSourceLane::AirBucket1 if byte_5b4 == 0 => {
                    (AIR_BUCKET_1_BYTE_5B4_RVA, "bucket_1_byte_5b4_zero", gate_value as i32)
                }
                ControllerSourceLane::AirBucket2 if flags & (1u64 << 20) != 0 => {
                    (AIR_BUCKET_2_BIT_20_RVA, "bucket_2_bit_20_set", gate_value as i32)
                }
                ControllerSourceLane::AirBucket3 => {
                    let predicate: TypeThreeValueFn =
                        unsafe { mem::transmute(base + AIR_BUCKET_3_TYPE_3_VALUE_RVA as usize) };
                    let value = unsafe { predicate(object, 3) };
                    if value <= 0 {
                        (AIR_BUCKET_3_TYPE_3_VALUE_RVA, "bucket_3_type_3_value_nonpositive", value)
                    } else {
                        return Ok(None);
                    }
                }
                _ => return Ok(None),
            }
        }
    };
    let mut canonical = Vec::with_capacity(96);
    canonical.extend_from_slice(b"hoi4-a28-air-rejection-v1\0");
    canonical.extend_from_slice(&pair.0.to_le_bytes());
    canonical.extend_from_slice(&pair.1.to_le_bytes());
    canonical.extend_from_slice(&(lane as u32).to_le_bytes());
    canonical.extend_from_slice(&flags.to_le_bytes());
    canonical.push(byte_5b4);
    canonical.extend_from_slice(&predicate_rva.to_le_bytes());
    canonical.extend_from_slice(&helper_value.to_le_bytes());
    canonical.extend_from_slice(reason.as_bytes());
    canonical.push(0);
    Ok(Some(RejectionEvidence {
        reason_code: reason.into(),
        predicate_rva,
        predicate_record_sha256: format!("{:x}", Sha256::digest(canonical)),
    }))
}

fn producer_digest(
    bucket: i32,
    candidates: &[ControllerCandidate],
    accepted: &[ControllerCandidateKey],
    slots: &[ControllerSlot],
    list: &ResolvedListState,
) -> String {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"hoi4-a28-air-materialization-v1\0");
    canonical.extend_from_slice(&bucket.to_le_bytes());
    canonical.extend_from_slice(&(candidates.len() as u64).to_le_bytes());
    for candidate in candidates {
        canonical.extend_from_slice(&candidate.source_ordinal.to_le_bytes());
        canonical.extend_from_slice(&candidate.key.source_instance_key.to_le_bytes());
        canonical.extend_from_slice(candidate.identity_record_sha256.as_bytes());
        canonical.push(0);
        if let Some(rejection) = &candidate.rejection {
            canonical.extend_from_slice(&rejection.predicate_rva.to_le_bytes());
            canonical.extend_from_slice(rejection.predicate_record_sha256.as_bytes());
        }
        canonical.push(0);
    }
    canonical.extend_from_slice(&(accepted.len() as u64).to_le_bytes());
    for key in accepted {
        canonical.extend_from_slice(&key.source_instance_key.to_le_bytes());
    }
    for slot in slots {
        canonical.extend_from_slice(&slot.stable_slot_id.to_le_bytes());
        for value in [slot.rect.x, slot.rect.y, slot.rect.width, slot.rect.height] {
            canonical.extend_from_slice(&value.to_le_bytes());
        }
    }
    for value in [
        list.clip_rect.x,
        list.clip_rect.y,
        list.clip_rect.width,
        list.clip_rect.height,
        list.content_extent_pixels.0,
        list.content_extent_pixels.1,
        list.viewport_extent_pixels.0,
        list.viewport_extent_pixels.1,
    ] {
        canonical.extend_from_slice(&value.to_le_bytes());
    }
    for value in [
        list.scroll_min_fixed,
        list.scroll_max_fixed,
        list.scroll_current_fixed,
        list.scroll_target_fixed,
    ] {
        canonical.extend_from_slice(&value.to_le_bytes());
    }
    format!("{:x}", Sha256::digest(canonical))
}

struct InFlight;

impl InFlight {
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

fn diagnostics() -> Result<AirSelectionHookDiagnostics, String> {
    let state = state_lock()?;
    Ok(AirSelectionHookDiagnostics {
        installed: INSTALLED.load(Ordering::Acquire),
        callbacks_open: CALLBACKS_OPEN.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT.load(Ordering::Acquire),
        controller_calls: state.controller_calls,
        transition_sequence: state.transition_sequence,
        pending_observations: state.observations.len(),
        poisoned: state.poison.is_some(),
    })
}

fn reset_state() -> Result<(), String> {
    let mut state = state_lock()?;
    *state = HookState::default();
    Ok(())
}

fn state_lock() -> Result<std::sync::MutexGuard<'static, HookState>, String> {
    STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "A28 air hook state mutex poisoned".to_string())
}

fn poison(error: String) {
    if let Ok(mut state) = state_lock() {
        state.poison.get_or_insert(error);
    }
}

fn invoke_geometry(object: usize, slot: usize, label: &str) -> Result<[i32; 2], String> {
    let vtable = read_usize(object).ok_or_else(|| format!("{label} vtable unreadable"))?;
    let function = read_usize(vtable.checked_add(slot).ok_or_else(|| format!("{label} slot overflow"))?)
        .ok_or_else(|| format!("{label} function unreadable"))?;
    if function == 0 {
        return Err(format!("{label} function is null"));
    }
    let callback: GeometryFn = unsafe { mem::transmute(function) };
    let mut output = [0i32; 2];
    unsafe { callback(object, output.as_mut_ptr()); }
    Ok(output)
}

fn gate_vtable(object: usize, expected: usize, label: &str) -> Result<(), String> {
    let actual = read_usize(object).ok_or_else(|| format!("{label} vtable unreadable"))?;
    if actual != expected {
        return Err(format!("{label} vtable mismatch: expected 0x{expected:X}, observed 0x{actual:X}"));
    }
    Ok(())
}

fn validate_scroll(state: &ResolvedListState) -> Result<(), String> {
    if state.scroll_max_fixed < state.scroll_min_fixed
        || !(state.scroll_min_fixed..=state.scroll_max_fixed).contains(&state.scroll_current_fixed)
        || !(state.scroll_min_fixed..=state.scroll_max_fixed).contains(&state.scroll_target_fixed)
    {
        return Err("air fixed scrollbar state is outside its range".into());
    }
    Ok(())
}

fn require_positive_extent(extent: [i32; 2], label: &str) -> Result<(), String> {
    if extent[0] <= 0 || extent[1] <= 0 {
        return Err(format!("{label} extent is not positive"));
    }
    Ok(())
}

fn bounded_count(value: i32, label: &str) -> Result<usize, String> {
    if value < 0 || value as usize > MAX_ROWS {
        return Err(format!("{label} count {value} exceeds bound"));
    }
    Ok(value as usize)
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual = rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn verify_executable_sha256(module: *mut c_void) -> Result<(), String> {
    let mut path = vec![0u16; 32_768];
    let length = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) };
    if length == 0 || length as usize >= path.len() {
        return Err("A28 air executable path is unavailable or truncated".into());
    }
    path.truncate(length as usize);
    let path = std::path::PathBuf::from(String::from_utf16(&path).map_err(|_| {
        "A28 air executable path is not valid UTF-16".to_string()
    })?);
    let mut file = File::open(path).map_err(|error| format!("A28 air executable open failed: {error}"))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("A28 air executable hash read failed: {error}"))?;
        if read == 0 { break; }
        hasher.update(&chunk[..read]);
    }
    if format!("{:x}", hasher.finalize()) != A729_EXECUTABLE_SHA256 {
        return Err("A28 air running executable SHA-256 differs from exact a729".into());
    }
    Ok(())
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err("A28 air hook callbacks did not quiesce".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

fn qpc() -> Result<u64, String> {
    let mut value = 0i64;
    if unsafe { QueryPerformanceCounter(&mut value) } == 0 || value <= 0 {
        return Err("QueryPerformanceCounter failed".into());
    }
    Ok(value as u64)
}

fn read_pair(address: usize) -> Option<(u32, u32)> {
    Some((read_u32(address)?, read_u32(address + 4)?))
}

fn read_u8(address: usize) -> Option<u8> { Some(*rpm(address, 1)?.first()?) }
fn read_u32(address: usize) -> Option<u32> { Some(u32::from_le_bytes(rpm(address, 4)?.try_into().ok()?)) }
fn read_i32(address: usize) -> Option<i32> { Some(i32::from_le_bytes(rpm(address, 4)?.try_into().ok()?)) }
fn read_u64(address: usize) -> Option<u64> { Some(u64::from_le_bytes(rpm(address, 8)?.try_into().ok()?)) }
fn read_i64(address: usize) -> Option<i64> { Some(i64::from_le_bytes(rpm(address, 8)?.try_into().ok()?)) }
fn read_usize(address: usize) -> Option<usize> {
    Some(usize::from_le_bytes(rpm(address, mem::size_of::<usize>())?.try_into().ok()?))
}

fn rpm(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 { return None; }
    let mut output = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            output.as_mut_ptr().cast(),
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(output)
}
