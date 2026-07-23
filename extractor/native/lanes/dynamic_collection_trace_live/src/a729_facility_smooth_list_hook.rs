//! Exact-a729 facility-specialization smooth-list owner hook.
//!
//! The sole detour wraps `CFacilitiesTabView::SetupDerived`.  The original
//! function has already enumerated every specialization token, constructed one
//! `specialization_item` per token, appended the rows in source order, and
//! completed list layout when this observer runs.  The observer then gates the
//! concrete list, row, and scrollbar vtables and copies a pointer-free
//! collection observation.  No generic list/layout function is detoured.

use crate::{
    CollectionTrace, ExactControllerTraceSource, MaterializedSlotTrace, RectI32, SourceItemTrace,
};
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::collections::{BTreeSet, VecDeque};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const FACILITY_SETUP_RVA: usize = 0x01C9_D840;
pub const FACILITY_LIST_OFFSET: usize = 0xB58;
pub const STANDARD_LIST_PRIMARY_VTABLE_RVA: usize = 0x02B2_CB48;
pub const STANDARD_LIST_SECONDARY_VTABLE_RVA: usize = 0x02B2_CC78;
pub const SPECIALIZATION_ROW_PRIMARY_VTABLE_RVA: usize = 0x02A4_9938;
pub const SPECIALIZATION_ROW_GUI_VTABLE_RVA: usize = 0x02A4_99A8;
pub const EXTENDED_SCROLLBAR_PRIMARY_VTABLE_RVA: usize = 0x02B3_2FF0;
pub const EXTENDED_SCROLLBAR_PLUS_80_VTABLE_RVA: usize = 0x02B3_3270;
pub const EXTENDED_SCROLLBAR_PLUS_B0_VTABLE_RVA: usize = 0x02B3_3300;

pub const FACILITY_HOOK_JOB_ID: &str =
    "20260720T141140Z-w5-a28-facility-complete-hook-authority-d14d7d1d";
pub const FACILITY_HOOK_MANIFEST_SHA256: &str =
    "fc5640f163fb89f9c52ae8db73344498f8f004adff5c048d913023807ea5c392";

const LIST_HEAD_OFFSET: usize = 0x90;
const LIST_COUNT_OFFSET: usize = 0xA0;
const LIST_SCROLLBAR_OFFSET: usize = 0x190;
const NODE_ITEM_OFFSET: usize = 0;
const NODE_NEXT_OFFSET: usize = 0x10;
const ROW_GUI_OFFSET: usize = 0x38;
const ROW_TOKEN_OFFSET: usize = 0x70;
const SCROLL_MIN_OFFSET: usize = 0x1558;
const SCROLL_MAX_OFFSET: usize = 0x1560;
const SCROLL_TARGET_OFFSET: usize = 0x15F8;
const SCROLL_CURRENT_OFFSET: usize = 0x1600;
const MAX_PENDING: usize = 4_096;
const MAX_ROWS: usize = crate::MAX_ITEMS_PER_COLLECTION;

const FACILITY_SETUP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D, 0xAC, 0x24,
];

type FacilitySetupFn = unsafe extern "system" fn(usize);
type GeometryFn = unsafe extern "system" fn(usize, *mut i32) -> *mut i32;

static ORIGINAL_SETUP: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static INSTALL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacilitySmoothListObservation {
    pub transition_sequence: u64,
    pub observation_qpc_ticks: u64,
    pub tokens: Vec<u32>,
    pub row_rects: Vec<RectI32>,
    pub clip_rect: RectI32,
    pub content_extent_pixels: (i32, i32),
    pub viewport_extent_pixels: (i32, i32),
    pub scroll_min_fixed: i64,
    pub scroll_max_fixed: i64,
    pub scroll_current_fixed: i64,
    pub scroll_target_fixed: i64,
}

#[derive(Default)]
struct HookState {
    controller_calls: u64,
    transition_sequence: u64,
    observations: VecDeque<FacilitySmoothListObservation>,
    poison: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacilityHookDiagnostics {
    pub installed: bool,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub controller_calls: u64,
    pub transition_sequence: u64,
    pub pending_observations: usize,
    pub poisoned: bool,
}

pub struct A729FacilitySmoothListHookOwner {
    target: usize,
    epoch: usize,
    active: bool,
}

impl A729FacilitySmoothListHookOwner {
    pub fn install() -> Result<Self, String> {
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 facility install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("A28 facility hook already has a live owner".into());
        }
        let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
        if base == 0 {
            return Err("GetModuleHandleW(NULL) returned null".into());
        }
        let target = base + FACILITY_SETUP_RVA;
        verify_sentinel(target, &FACILITY_SETUP_SENTINEL, "facility SetupDerived")?;
        reset_state()?;
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| value.checked_add(1))
            .map_err(|_| "A28 facility install epoch exhausted".to_string())?
            + 1;
        let original = unsafe {
            MinHook::create_hook(
                target as *mut c_void,
                hook_facility_setup as *mut c_void,
            )
        }
        .map_err(|status| format!("A28 facility hook creation failed: {status}"))?;
        ORIGINAL_SETUP.store(original as usize, Ordering::Release);
        MODULE_BASE.store(base, Ordering::Release);
        CALLBACKS_OPEN.store(true, Ordering::Release);
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            CALLBACKS_OPEN.store(false, Ordering::Release);
            let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
            ORIGINAL_SETUP.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("A28 facility hook enable failed: {status}"));
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(Self {
            target,
            epoch,
            active: true,
        })
    }

    pub fn diagnostics(&self) -> Result<FacilityHookDiagnostics, String> {
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 facility hook owner epoch is stale".into());
        }
        diagnostics()
    }

    pub fn drain_observations(
        &self,
    ) -> Result<Vec<FacilitySmoothListObservation>, String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A28 facility hook owner is not active".into());
        }
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 facility hook owner epoch is stale".into());
        }
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 facility hook poisoned: {error}"));
        }
        Ok(state.observations.drain(..).collect())
    }

    pub fn drain_source(
        &self,
        frame_hour: u64,
    ) -> Result<A729FacilitySmoothListSource, String> {
        let observations = self.drain_observations()?;
        A729FacilitySmoothListSource::try_from_observations(frame_hour, observations)
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 facility install mutex poisoned".to_string())?;
        CALLBACKS_OPEN.store(false, Ordering::Release);
        match unsafe { MinHook::disable_hook(self.target as *mut c_void) } {
            Ok(())
            | Err(MH_STATUS::MH_ERROR_DISABLED)
            | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                return Err(format!("A28 facility hook disable failed: {status}"));
            }
        }
        wait_for_quiescence(Duration::from_secs(30))?;
        match unsafe { MinHook::remove_hook(self.target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                return Err(format!("A28 facility hook removal failed: {status}"));
            }
        }
        ORIGINAL_SETUP.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        self.active = false;
        Ok(())
    }
}

impl Drop for A729FacilitySmoothListHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Debug)]
pub struct A729FacilitySmoothListSource {
    frame_hour: u64,
    collections: Vec<CollectionTrace>,
}

impl A729FacilitySmoothListSource {
    pub fn try_from_observations(
        frame_hour: u64,
        observations: Vec<FacilitySmoothListObservation>,
    ) -> Result<Self, String> {
        if observations.is_empty() {
            return Err("A28 facility source has no post-return observation".into());
        }
        let mut collections = Vec::with_capacity(observations.len());
        for observation in observations {
            collections.push(observation.into_collection(frame_hour)?);
        }
        Ok(Self {
            frame_hour,
            collections,
        })
    }
}

impl crate::source_seal::Sealed for A729FacilitySmoothListSource {}

impl ExactControllerTraceSource for A729FacilitySmoothListSource {
    fn is_exact_a729_controller_source(&self) -> bool {
        true
    }

    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<CollectionTrace>, String> {
        if frame_hour != self.frame_hour {
            return Err(format!(
                "A28 facility observation hour {} differs from requested hour {frame_hour}",
                self.frame_hour
            ));
        }
        Ok(self.collections.clone())
    }
}

impl FacilitySmoothListObservation {
    fn into_collection(self, frame_hour: u64) -> Result<CollectionTrace, String> {
        if self.tokens.len() != self.row_rects.len() {
            return Err("facility source/row count mismatch".into());
        }
        let mut unique = BTreeSet::new();
        for token in &self.tokens {
            if !unique.insert(*token) {
                return Err(format!(
                    "facility source contains duplicate LexerToken {token:08x}"
                ));
            }
        }
        let ids: Vec<String> = self
            .tokens
            .iter()
            .map(|token| format!("facility-specialization-lexer-token:{token:08x}"))
            .collect();
        let source_items = ids
            .iter()
            .enumerate()
            .map(|(ordinal, stable_item_id)| SourceItemTrace {
                stable_item_id: stable_item_id.clone(),
                source_ordinal: ordinal as u32,
                included: true,
                filter_reason: None,
            })
            .collect();
        let materialized_slots = ids
            .iter()
            .zip(self.row_rects)
            .map(|(bound_item_id, rect)| MaterializedSlotTrace {
                slot_id: format!(
                    "interface/special_projects/facility_roster.gui#collection-0001/{bound_item_id}"
                ),
                bound_item_id: bound_item_id.clone(),
                row_template_id: "specialization_item".into(),
                rect,
            })
            .collect();
        Ok(CollectionTrace {
            collection_instance_id:
                "interface/special_projects/facility_roster.gui#collection-0001".into(),
            frame_hour,
            source_items,
            ordered_item_ids: ids,
            group_boundaries: Vec::new(),
            row_template_id: "specialization_item".into(),
            materialized_slots,
            clip_rect: self.clip_rect,
            content_extent_pixels: self.content_extent_pixels,
            viewport_extent_pixels: self.viewport_extent_pixels,
            scroll_min_fixed: self.scroll_min_fixed,
            scroll_max_fixed: self.scroll_max_fixed,
            scroll_current_fixed: self.scroll_current_fixed,
            scroll_target_fixed: self.scroll_target_fixed,
        })
    }
}

unsafe extern "system" fn hook_facility_setup(owner: usize) {
    let Some(_in_flight) = InFlight::enter() else {
        call_original(owner);
        return;
    };
    call_original(owner);
    if let Err(error) = capture_post_return(owner) {
        poison(error);
    }
}

unsafe fn call_original(owner: usize) {
    let address = ORIGINAL_SETUP.load(Ordering::Acquire);
    if address != 0 {
        let original: FacilitySetupFn = mem::transmute(address);
        original(owner);
    }
}

fn capture_post_return(owner: usize) -> Result<(), String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("module base absent during facility callback".into());
    }
    let list = read_usize(
        owner
            .checked_add(FACILITY_LIST_OFFSET)
            .ok_or("facility owner offset overflow")?,
    )
    .ok_or("facility list pointer unreadable")?;
    gate_vtable(
        list,
        base + STANDARD_LIST_PRIMARY_VTABLE_RVA,
        "facility standard-list primary",
    )?;
    gate_vtable(
        list + 8,
        base + STANDARD_LIST_SECONDARY_VTABLE_RVA,
        "facility standard-list secondary",
    )?;
    let count = read_u32(list + LIST_COUNT_OFFSET).ok_or("facility row count unreadable")? as usize;
    if count > MAX_ROWS {
        return Err(format!("facility row count {count} exceeds bound"));
    }
    let mut tokens = Vec::with_capacity(count);
    let mut row_rects = Vec::with_capacity(count);
    let mut node = read_usize(list + LIST_HEAD_OFFSET).ok_or("facility list head unreadable")?;
    for ordinal in 0..count {
        if node == 0 {
            return Err(format!("facility row chain ended at ordinal {ordinal}"));
        }
        let row_gui =
            read_usize(node + NODE_ITEM_OFFSET).ok_or("facility row GUI pointer unreadable")?;
        if row_gui < ROW_GUI_OFFSET {
            return Err("facility row GUI pointer underflows row base".into());
        }
        let row = row_gui - ROW_GUI_OFFSET;
        gate_vtable(
            row,
            base + SPECIALIZATION_ROW_PRIMARY_VTABLE_RVA,
            "facility specialization row primary",
        )?;
        gate_vtable(
            row_gui,
            base + SPECIALIZATION_ROW_GUI_VTABLE_RVA,
            "facility specialization row GUI",
        )?;
        let token = read_u32(row + ROW_TOKEN_OFFSET).ok_or("facility row token unreadable")?;
        let position = invoke_geometry(row_gui, 0x78, "facility row resolved position")?;
        let extent = invoke_geometry(row_gui, 0x28, "facility row resolved extent")?;
        require_positive_extent(extent, "facility row")?;
        tokens.push(token);
        row_rects.push(RectI32 {
            x: position[0],
            y: position[1],
            width: extent[0],
            height: extent[1],
        });
        node = read_usize(node + NODE_NEXT_OFFSET).ok_or("facility next row link unreadable")?;
    }
    if node != 0 {
        return Err("facility row chain contains more nodes than authenticated count".into());
    }

    let list_interface = list + 8;
    let position = invoke_geometry(list_interface, 0xA0, "facility list resolved position")?;
    let viewport = invoke_geometry(list_interface, 0x30, "facility list viewport extent")?;
    let content = invoke_geometry(list_interface, 0xE0, "facility list content extent")?;
    require_positive_extent(viewport, "facility viewport")?;
    if content[0] < 0 || content[1] < 0 {
        return Err("facility content extent is negative".into());
    }

    let scrollbar =
        read_usize(list + LIST_SCROLLBAR_OFFSET).ok_or("facility scrollbar pointer unreadable")?;
    gate_vtable(
        scrollbar,
        base + EXTENDED_SCROLLBAR_PRIMARY_VTABLE_RVA,
        "facility extended scrollbar primary",
    )?;
    gate_vtable(
        scrollbar + 0x80,
        base + EXTENDED_SCROLLBAR_PLUS_80_VTABLE_RVA,
        "facility extended scrollbar +0x80",
    )?;
    gate_vtable(
        scrollbar + 0xB0,
        base + EXTENDED_SCROLLBAR_PLUS_B0_VTABLE_RVA,
        "facility extended scrollbar +0xB0",
    )?;
    let scroll_min_fixed =
        read_i64(scrollbar + SCROLL_MIN_OFFSET).ok_or("facility scroll minimum unreadable")?;
    let scroll_max_fixed =
        read_i64(scrollbar + SCROLL_MAX_OFFSET).ok_or("facility scroll maximum unreadable")?;
    let scroll_current_fixed =
        read_i64(scrollbar + SCROLL_CURRENT_OFFSET).ok_or("facility scroll current unreadable")?;
    let scroll_target_fixed =
        read_i64(scrollbar + SCROLL_TARGET_OFFSET).ok_or("facility scroll target unreadable")?;
    if scroll_max_fixed < scroll_min_fixed
        || !(scroll_min_fixed..=scroll_max_fixed).contains(&scroll_current_fixed)
        || !(scroll_min_fixed..=scroll_max_fixed).contains(&scroll_target_fixed)
    {
        return Err("facility fixed scroll state is outside its range".into());
    }

    let qpc_ticks = qpc()?;
    let mut state = state_lock()?;
    state.controller_calls = state.controller_calls.saturating_add(1);
    state.transition_sequence = state
        .transition_sequence
        .checked_add(1)
        .ok_or("facility transition sequence exhausted")?;
    let sequence = state.transition_sequence;
    state.observations.push_back(FacilitySmoothListObservation {
        transition_sequence: sequence,
        observation_qpc_ticks: qpc_ticks,
        tokens,
        row_rects,
        clip_rect: RectI32 {
            x: position[0],
            y: position[1],
            width: viewport[0],
            height: viewport[1],
        },
        content_extent_pixels: (content[0], content[1]),
        viewport_extent_pixels: (viewport[0], viewport[1]),
        scroll_min_fixed,
        scroll_max_fixed,
        scroll_current_fixed,
        scroll_target_fixed,
    });
    if state.observations.len() > MAX_PENDING {
        state.observations.pop_front();
    }
    Ok(())
}

struct InFlight;

impl InFlight {
    fn enter() -> Option<Self> {
        if !CALLBACKS_OPEN.load(Ordering::Acquire) {
            return None;
        }
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        if !CALLBACKS_OPEN.load(Ordering::Acquire) {
            IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Self)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

fn diagnostics() -> Result<FacilityHookDiagnostics, String> {
    let state = state_lock()?;
    Ok(FacilityHookDiagnostics {
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
        .map_err(|_| "A28 facility hook state mutex poisoned".to_string())
}

fn poison(error: String) {
    if let Ok(mut state) = state_lock() {
        state.poison.get_or_insert(error);
    }
}

fn invoke_geometry(object: usize, slot: usize, label: &str) -> Result<[i32; 2], String> {
    let vtable = read_usize(object).ok_or_else(|| format!("{label} vtable unreadable"))?;
    let function = read_usize(
        vtable
            .checked_add(slot)
            .ok_or_else(|| format!("{label} vtable slot overflow"))?,
    )
    .ok_or_else(|| format!("{label} function pointer unreadable"))?;
    if function == 0 {
        return Err(format!("{label} function pointer is null"));
    }
    let callback: GeometryFn = unsafe { mem::transmute(function) };
    let mut out = [0i32; 2];
    unsafe {
        callback(object, out.as_mut_ptr());
    }
    Ok(out)
}

fn gate_vtable(object: usize, expected: usize, label: &str) -> Result<(), String> {
    let actual = read_usize(object).ok_or_else(|| format!("{label} vtable unreadable"))?;
    if actual != expected {
        return Err(format!(
            "{label} vtable mismatch: expected 0x{expected:X}, observed 0x{actual:X}"
        ));
    }
    Ok(())
}

fn require_positive_extent(extent: [i32; 2], label: &str) -> Result<(), String> {
    if extent[0] <= 0 || extent[1] <= 0 {
        return Err(format!("{label} extent is not positive"));
    }
    Ok(())
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual = rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err("A28 facility hook callbacks did not quiesce".into());
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

fn read_u32(address: usize) -> Option<u32> {
    Some(u32::from_le_bytes(rpm(address, 4)?.try_into().ok()?))
}

fn read_i64(address: usize) -> Option<i64> {
    Some(i64::from_le_bytes(rpm(address, 8)?.try_into().ok()?))
}

fn read_usize(address: usize) -> Option<usize> {
    Some(usize::from_le_bytes(
        rpm(address, mem::size_of::<usize>())?.try_into().ok()?,
    ))
}

fn rpm(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 {
        return None;
    }
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
