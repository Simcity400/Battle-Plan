use crate::{
    CollectionTrace, ExactControllerTraceSource, MaterializedSlotTrace, RectI32, SourceItemTrace,
    TraceFrame,
};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use std::cell::RefCell;
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

pub const TECHNOLOGY_BADGE_CONTROLLER_RVA: usize = 0x01BC_1A00;
pub const TECHNOLOGY_BADGE_POSITION_RVA: usize = 0x022C_A2E0;
pub const STANDARD_GRID_PRIMARY_VTABLE_RVA: usize = 0x02B2_F458;
pub const STANDARD_GRID_SECONDARY_VTABLE_RVA: usize = 0x02B2_F6E0;
pub const SPECIALIZATION_BADGE_ROW_VTABLE_RVA: usize = 0x02A3_6B60;

pub const TECHNOLOGY_BADGE_HOOK_JOB_ID: &str =
    "20260720T141305Z-w5-a28-tech-badge-owner-callgraph-comple-b890bceb";
pub const TECHNOLOGY_BADGE_HOOK_MANIFEST_SHA256: &str =
    "f4644574640b69d5ae650ed6a68564cf7efb3b09f5a05b3888b65338f9e77d5e";
pub const TECHNOLOGY_BADGE_SENTINEL_JOB_ID: &str =
    "20260720T215038Z-w5final-a28-tech-overlap-hook-sentinels-ef563094";
pub const TECHNOLOGY_BADGE_SENTINEL_ARTIFACT_SHA256: &str =
    "3eba11027150a60121e8d45e92dbb6e21e4dc85d744c27baaa531a732460e94d";

const SOURCE_VECTOR_OFFSET: usize = 0x140;
const SOURCE_COUNT_OFFSET: usize = 0x14C;
const GRID_CHILDREN_OFFSET: usize = 0x1A0;
const GRID_CHILD_COUNT_OFFSET: usize = 0x1AC;
const GRID_CONTENT_EXTENT_OFFSET: usize = 0x168;
const GRID_GUI_OFFSET: usize = 0x140;
const GRID_SECONDARY_OFFSET: usize = 0x1F8;
const ROW_GUI_POINTER_OFFSET: usize = 0x10;
const SENTINEL_SPECIALIZATION: u32 = 0x4C17;
const MAX_PENDING: usize = 4_096;

const CONTROLLER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
const POSITION_SENTINEL: [u8; 16] = [
    0x48, 0x83, 0xEC, 0x28, 0x4C, 0x8B, 0x99, 0x40, 0x01, 0x00, 0x00, 0x44, 0x8B, 0xC2, 0x48, 0xC1,
];

type ControllerFn = unsafe extern "system" fn(usize, usize, u8, u32, usize);
type PositionFn = unsafe extern "system" fn(usize, u64);
type GeometryFn = unsafe extern "system" fn(usize, *mut i32) -> *mut i32;

static ORIGINAL_CONTROLLER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_POSITION: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static INSTALL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

thread_local! {
    static PENDING_CONTROLLER: RefCell<Vec<PendingController>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyBadgeGridObservation {
    pub transition_sequence: u64,
    pub observation_qpc_ticks: u64,
    pub source_tokens: Vec<u32>,
    pub row_rect: RectI32,
    pub clip_rect: RectI32,
    pub content_extent_pixels: (i32, i32),
    pub viewport_extent_pixels: (i32, i32),
}

#[derive(Default)]
struct PendingController {
    source_tokens: Vec<u32>,
    captured_grid: Option<CapturedGrid>,
}

#[derive(Clone, Debug)]
struct CapturedGrid {
    row_rect: RectI32,
    clip_rect: RectI32,
    content_extent_pixels: (i32, i32),
    viewport_extent_pixels: (i32, i32),
}

#[derive(Default)]
struct HookState {
    controller_calls: u64,
    transition_sequence: u64,
    observations: VecDeque<TechnologyBadgeGridObservation>,
    poison: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyBadgeHookDiagnostics {
    pub installed: bool,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub controller_calls: u64,
    pub transition_sequence: u64,
    pub pending_observations: usize,
    pub poisoned: bool,
}

pub struct A729TechnologyBadgeGridHookOwner {
    controller_target: usize,
    position_target: usize,
    epoch: usize,
    active: bool,
}

impl A729TechnologyBadgeGridHookOwner {
    pub fn install() -> Result<Self, String> {
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 technology badge install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("A28 technology badge hooks already have a live owner".into());
        }
        let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
        if base == 0 {
            return Err("GetModuleHandleW(NULL) returned null".into());
        }
        let controller_target = base + TECHNOLOGY_BADGE_CONTROLLER_RVA;
        let position_target = base + TECHNOLOGY_BADGE_POSITION_RVA;
        verify_sentinel(
            controller_target,
            &CONTROLLER_SENTINEL,
            "technology badge controller",
        )?;
        verify_sentinel(
            position_target,
            &POSITION_SENTINEL,
            "technology badge position",
        )?;
        reset_state()?;
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "A28 technology badge install epoch exhausted".to_string())?
            + 1;
        let original_controller = unsafe {
            MinHook::create_hook(
                controller_target as *mut c_void,
                hook_controller as *mut c_void,
            )
        }
        .map_err(|status| format!("technology badge controller hook creation failed: {status}"))?;
        ORIGINAL_CONTROLLER.store(original_controller as usize, Ordering::Release);
        let original_position = match unsafe {
            MinHook::create_hook(position_target as *mut c_void, hook_position as *mut c_void)
        } {
            Ok(original) => original,
            Err(status) => {
                let _ = unsafe { inline_detour::remove(controller_target as *mut c_void) };
                ORIGINAL_CONTROLLER.store(0, Ordering::Release);
                return Err(format!(
                    "technology badge position hook creation failed: {status}"
                ));
            }
        };
        ORIGINAL_POSITION.store(original_position as usize, Ordering::Release);
        MODULE_BASE.store(base, Ordering::Release);
        CALLBACKS_OPEN.store(true, Ordering::Release);
        if let Err(status) = unsafe { inline_detour::enable(position_target as *mut c_void) } {
            cleanup_created_hooks(controller_target, position_target)?;
            return Err(format!(
                "technology badge position hook enable failed: {status}"
            ));
        }
        if let Err(status) = unsafe { inline_detour::enable(controller_target as *mut c_void) } {
            let _ = unsafe { inline_detour::disable(position_target as *mut c_void) };
            cleanup_created_hooks(controller_target, position_target)?;
            return Err(format!(
                "technology badge controller hook enable failed: {status}"
            ));
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(Self {
            controller_target,
            position_target,
            epoch,
            active: true,
        })
    }

    pub fn diagnostics(&self) -> Result<TechnologyBadgeHookDiagnostics, String> {
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 technology badge hook owner epoch is stale".into());
        }
        diagnostics()
    }

    pub fn drain_observations(&self) -> Result<Vec<TechnologyBadgeGridObservation>, String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A28 technology badge hook owner is not active".into());
        }
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 technology badge hook poisoned: {error}"));
        }
        Ok(state.observations.drain(..).collect())
    }

    pub fn drain_source(
        &self,
        frame_hour: u64,
        owner_identities: Vec<TechnologyBadgeOwnerIdentity>,
    ) -> Result<A729TechnologyBadgeGridSource, String> {
        let observations = self.drain_observations()?;
        if observations.len() != owner_identities.len() {
            return Err("technology badge observation/owner-identity counts differ".into());
        }
        A729TechnologyBadgeGridSource::try_join(
            frame_hour,
            observations.into_iter().zip(owner_identities).collect(),
        )
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 technology badge install mutex poisoned".to_string())?;
        CALLBACKS_OPEN.store(false, Ordering::Release);
        unsafe { inline_detour::disable(self.controller_target as *mut c_void) }
            .or_else(|status| {
                if status.already_released() {
                    Ok(())
                } else {
                    Err(status)
                }
            })
            .map_err(|status| format!("technology badge controller disable failed: {status}"))?;
        unsafe { inline_detour::disable(self.position_target as *mut c_void) }
            .or_else(|status| {
                if status.already_released() {
                    Ok(())
                } else {
                    Err(status)
                }
            })
            .map_err(|status| format!("technology badge position disable failed: {status}"))?;
        wait_for_quiescence(Duration::from_secs(30))?;
        let mut removal_failures = Vec::new();
        for (target, label) in [
            (self.position_target, "position"),
            (self.controller_target, "controller"),
        ] {
            match unsafe { inline_detour::remove(target as *mut c_void) } {
                Ok(())
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                Err(status) => removal_failures
                    .push(format!("technology badge {label} removal failed: {status}")),
            }
        }
        if !removal_failures.is_empty() {
            return Err(removal_failures.join(", "));
        }
        ORIGINAL_POSITION.store(0, Ordering::Release);
        ORIGINAL_CONTROLLER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        self.active = false;
        Ok(())
    }
}

impl Drop for A729TechnologyBadgeGridHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Debug)]
pub struct A729TechnologyBadgeGridSource {
    frame_hour: u64,
    collections: Vec<CollectionTrace>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyBadgeOwnerIdentity {
    pub collection_instance_id: String,
}

impl A729TechnologyBadgeGridSource {
    pub fn try_join(
        frame_hour: u64,
        joined: Vec<(TechnologyBadgeGridObservation, TechnologyBadgeOwnerIdentity)>,
    ) -> Result<Self, String> {
        if joined.is_empty() {
            return Err("A28 technology badge source has no complete owner join".into());
        }
        let mut ids = BTreeSet::new();
        let mut collections = Vec::with_capacity(joined.len());
        for (observation, identity) in joined {
            let collection = observation.into_collection(frame_hour, identity)?;
            if !ids.insert(collection.collection_instance_id.clone()) {
                return Err("duplicate technology badge collection identity in frame".into());
            }
            collections.push(collection);
        }
        crate::validate_frame(&TraceFrame {
            elapsed_wall_ms: 0,
            status_flags: 0,
            frame_hour,
            collections: collections.clone(),
        })
        .map_err(|error| format!("technology badge DCT1 validation failed: {error:?}"))?;
        Ok(Self {
            frame_hour,
            collections,
        })
    }
}

impl crate::source_seal::Sealed for A729TechnologyBadgeGridSource {}

impl ExactControllerTraceSource for A729TechnologyBadgeGridSource {
    fn is_exact_a729_controller_source(&self) -> bool {
        true
    }

    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<CollectionTrace>, String> {
        if frame_hour != self.frame_hour {
            return Err(format!(
                "A28 technology badge observation hour {} differs from requested hour {frame_hour}",
                self.frame_hour
            ));
        }
        Ok(self.collections.clone())
    }
}

impl TechnologyBadgeGridObservation {
    fn into_collection(
        self,
        frame_hour: u64,
        identity: TechnologyBadgeOwnerIdentity,
    ) -> Result<CollectionTrace, String> {
        if self.source_tokens.is_empty() {
            return Err("technology badge observation has an empty source vector".into());
        }
        let first = self.source_tokens[0];
        let second = self
            .source_tokens
            .get(1)
            .copied()
            .unwrap_or(SENTINEL_SPECIALIZATION);
        let pair_id = format!("technology-specialization-pair:{first:08x}:{second:08x}");
        let mut source_items = Vec::with_capacity(self.source_tokens.len().saturating_sub(1));
        source_items.push(SourceItemTrace {
            stable_item_id: pair_id.clone(),
            source_ordinal: 0,
            included: true,
            filter_reason: None,
        });
        for (wire_ordinal, (source_ordinal, token)) in self
            .source_tokens
            .iter()
            .copied()
            .enumerate()
            .skip(2)
            .enumerate()
        {
            source_items.push(SourceItemTrace {
                stable_item_id: format!(
                    "technology-specialization-unrendered:{source_ordinal:04}:{token:08x}"
                ),
                source_ordinal: (wire_ordinal + 1) as u32,
                included: false,
                filter_reason: Some("controller_two_badge_limit".into()),
            });
        }
        let collection_instance_id = identity.collection_instance_id;
        require_authored_technology_badge_collection(&collection_instance_id)?;
        Ok(CollectionTrace {
            collection_instance_id: collection_instance_id.clone(),
            frame_hour,
            source_items,
            ordered_item_ids: vec![pair_id.clone()],
            group_boundaries: Vec::new(),
            row_template_id: "special_project_specialization".into(),
            materialized_slots: vec![MaterializedSlotTrace {
                slot_id: format!("{collection_instance_id}/composite-badge-row"),
                bound_item_id: pair_id,
                row_template_id: "special_project_specialization".into(),
                rect: self.row_rect,
            }],
            clip_rect: self.clip_rect,
            content_extent_pixels: self.content_extent_pixels,
            viewport_extent_pixels: self.viewport_extent_pixels,
            scroll_min_fixed: 0,
            scroll_max_fixed: 0,
            scroll_current_fixed: 0,
            scroll_target_fixed: 0,
        })
    }
}

fn require_authored_technology_badge_collection(collection_id: &str) -> Result<(), String> {
    const PREFIX: &str = "interface/countrytechtreeview.gui#collection-";
    let Some(suffix) = collection_id.strip_prefix(PREFIX) else {
        return Err(
            "technology badge owner identity is not a countrytechtreeview collection".into(),
        );
    };
    if suffix.len() != 4 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("technology badge owner identity has a malformed collection ordinal".into());
    }
    let ordinal = suffix
        .parse::<u16>()
        .map_err(|_| "technology badge owner identity ordinal is unreadable".to_string())?;
    if !(64..=86).contains(&ordinal) {
        return Err(
            "technology badge owner identity is outside the 23 authored badge grids".into(),
        );
    }
    Ok(())
}

unsafe extern "system" fn hook_controller(
    owner: usize,
    source_owner: usize,
    selected: u8,
    mode: u32,
    optional_owner: usize,
) {
    let in_flight = InFlight::enter();
    if !in_flight.admitted() {
        call_original_controller(owner, source_owner, selected, mode, optional_owner);
        return;
    }
    let tokens = match read_source_tokens(source_owner) {
        Ok(tokens) => tokens,
        Err(error) => {
            poison(error);
            call_original_controller(owner, source_owner, selected, mode, optional_owner);
            return;
        }
    };
    let push_result = PENDING_CONTROLLER
        .try_with(|pending| {
            pending
                .try_borrow_mut()
                .map_err(|_| "technology badge TLS controller stack already borrowed".to_string())?
                .push(PendingController {
                    source_tokens: tokens,
                    captured_grid: None,
                });
            Ok::<(), String>(())
        })
        .map_err(|_| "technology badge TLS controller stack unavailable".to_string())
        .and_then(|result| result);
    if let Err(error) = push_result {
        poison(error);
        call_original_controller(owner, source_owner, selected, mode, optional_owner);
        return;
    }
    call_original_controller(owner, source_owner, selected, mode, optional_owner);
    let pending = PENDING_CONTROLLER
        .try_with(|pending| {
            pending
                .try_borrow_mut()
                .map_err(|_| "technology badge TLS controller stack already borrowed".to_string())?
                .pop()
                .ok_or_else(|| "technology badge TLS controller stack underflow".to_string())
        })
        .unwrap_or_else(|_| Err("technology badge TLS controller stack unavailable".into()));
    match pending {
        Ok(pending) => {
            if pending.source_tokens.is_empty() {
                return;
            }
            let Some(grid) = pending.captured_grid else {
                poison("technology badge controller returned without nested grid position".into());
                return;
            };
            if let Err(error) = publish_observation(pending.source_tokens, grid) {
                poison(error.to_string());
            }
        }
        Err(error) => poison(error),
    }
}

unsafe extern "system" fn hook_position(grid: usize, packed_position: u64) {
    let in_flight = InFlight::enter();
    if !in_flight.admitted() {
        call_original_position(grid, packed_position);
        return;
    }
    call_original_position(grid, packed_position);
    let has_pending = PENDING_CONTROLLER
        .try_with(|pending| {
            pending
                .try_borrow()
                .map(|pending| !pending.is_empty())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if !has_pending {
        return;
    }
    match capture_grid(grid) {
        Ok(capture) => {
            let capture_result = PENDING_CONTROLLER
                .try_with(|pending| {
                    let mut pending = pending.try_borrow_mut().map_err(|_| {
                        "technology badge TLS controller stack already borrowed".to_string()
                    })?;
                    let current = pending
                        .last_mut()
                        .ok_or_else(|| "technology badge TLS controller stack empty".to_string())?;
                    if current.captured_grid.replace(capture).is_some() {
                        return Err(
                            "technology badge controller positioned more than one grid".into()
                        );
                    }
                    Ok::<(), String>(())
                })
                .map_err(|_| "technology badge TLS controller stack unavailable".to_string())
                .and_then(|result| result);
            if let Err(error) = capture_result {
                poison(error);
            }
        }
        Err(error) => poison(error),
    }
}

fn read_source_tokens(source_owner: usize) -> Result<Vec<u32>, String> {
    if source_owner == 0 {
        return Err("technology badge source owner is null".into());
    }
    let vector = source_owner
        .checked_add(SOURCE_VECTOR_OFFSET)
        .ok_or("technology badge source vector offset overflow")?;
    let count = read_u32(source_owner + SOURCE_COUNT_OFFSET)
        .ok_or("technology badge source count unreadable")? as usize;
    if count > crate::MAX_ITEMS_PER_COLLECTION {
        return Err(format!(
            "technology badge source count {count} exceeds bound"
        ));
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let data = read_usize(vector).ok_or("technology badge source data unreadable")?;
    let bytes = rpm(
        data,
        count
            .checked_mul(4)
            .ok_or("technology badge source size overflow")?,
    )
    .ok_or("technology badge source token bytes unreadable")?;
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("exact chunk")))
        .collect())
}

fn capture_grid(grid: usize) -> Result<CapturedGrid, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("module base absent during technology badge grid capture".into());
    }
    gate_vtable(
        grid,
        base + STANDARD_GRID_PRIMARY_VTABLE_RVA,
        "technology badge standard grid primary",
    )?;
    gate_vtable(
        grid + GRID_SECONDARY_OFFSET,
        base + STANDARD_GRID_SECONDARY_VTABLE_RVA,
        "technology badge standard grid secondary",
    )?;
    let child_count = read_u32(grid + GRID_CHILD_COUNT_OFFSET)
        .ok_or("technology badge grid child count unreadable")? as usize;
    if child_count != 1 {
        return Err(format!(
            "technology badge grid must contain one composite row, observed {child_count}"
        ));
    }
    let children = read_usize(grid + GRID_CHILDREN_OFFSET)
        .ok_or("technology badge grid child vector unreadable")?;
    let row = read_usize(children).ok_or("technology badge row pointer unreadable")?;
    gate_vtable(
        row,
        base + SPECIALIZATION_BADGE_ROW_VTABLE_RVA,
        "technology badge composite row",
    )?;
    let row_gui = read_usize(row + ROW_GUI_POINTER_OFFSET)
        .ok_or("technology badge row GUI pointer unreadable")?;
    let row_position = invoke_geometry(row_gui, 0x78, "technology badge row position")?;
    let row_extent = invoke_geometry(row_gui, 0x28, "technology badge row extent")?;
    require_positive(row_extent, "technology badge row")?;

    let grid_gui =
        read_usize(grid + GRID_GUI_OFFSET).ok_or("technology badge grid GUI pointer unreadable")?;
    let grid_position = invoke_geometry(grid_gui, 0x78, "technology badge grid position")?;
    let viewport = invoke_geometry(grid_gui, 0x28, "technology badge grid viewport")?;
    require_positive(viewport, "technology badge viewport")?;
    let content_width = read_i32(grid + GRID_CONTENT_EXTENT_OFFSET)
        .ok_or("technology badge content width unreadable")?;
    let content_height = read_i32(grid + GRID_CONTENT_EXTENT_OFFSET + 4)
        .ok_or("technology badge content height unreadable")?;
    if content_width < 0 || content_height < 0 {
        return Err("technology badge content extent is negative".into());
    }
    Ok(CapturedGrid {
        row_rect: RectI32 {
            x: row_position[0],
            y: row_position[1],
            width: row_extent[0],
            height: row_extent[1],
        },
        clip_rect: RectI32 {
            x: grid_position[0],
            y: grid_position[1],
            width: viewport[0],
            height: viewport[1],
        },
        content_extent_pixels: (content_width, content_height),
        viewport_extent_pixels: (viewport[0], viewport[1]),
    })
}

fn publish_observation(source_tokens: Vec<u32>, grid: CapturedGrid) -> Result<(), String> {
    let qpc_ticks = qpc()?;
    let mut state = state_lock()?;
    state.controller_calls = state.controller_calls.saturating_add(1);
    state.transition_sequence = state
        .transition_sequence
        .checked_add(1)
        .ok_or("technology badge transition sequence exhausted")?;
    let sequence = state.transition_sequence;
    state
        .observations
        .push_back(TechnologyBadgeGridObservation {
            transition_sequence: sequence,
            observation_qpc_ticks: qpc_ticks,
            source_tokens,
            row_rect: grid.row_rect,
            clip_rect: grid.clip_rect,
            content_extent_pixels: grid.content_extent_pixels,
            viewport_extent_pixels: grid.viewport_extent_pixels,
        });
    if state.observations.len() > MAX_PENDING {
        state.observations.pop_front();
    }
    Ok(())
}

unsafe fn call_original_controller(
    owner: usize,
    source_owner: usize,
    selected: u8,
    mode: u32,
    optional_owner: usize,
) {
    let address = ORIGINAL_CONTROLLER.load(Ordering::Acquire);
    if address != 0 {
        let original: ControllerFn = mem::transmute(address);
        original(owner, source_owner, selected, mode, optional_owner);
    }
}

unsafe fn call_original_position(grid: usize, packed_position: u64) {
    let address = ORIGINAL_POSITION.load(Ordering::Acquire);
    if address != 0 {
        let original: PositionFn = mem::transmute(address);
        original(grid, packed_position);
    }
}

fn cleanup_created_hooks(controller_target: usize, position_target: usize) -> Result<(), String> {
    CALLBACKS_OPEN.store(false, Ordering::Release);
    let _ = unsafe { inline_detour::disable(controller_target as *mut c_void) };
    let _ = unsafe { inline_detour::disable(position_target as *mut c_void) };
    wait_for_quiescence(Duration::from_secs(30)).map_err(|error| {
        format!("technology badge rollback left disabled hooks installed: {error}")
    })?;
    let _ = unsafe { inline_detour::remove(position_target as *mut c_void) };
    let _ = unsafe { inline_detour::remove(controller_target as *mut c_void) };
    ORIGINAL_POSITION.store(0, Ordering::Release);
    ORIGINAL_CONTROLLER.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    Ok(())
}

struct InFlight;

impl InFlight {
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }

    fn admitted(&self) -> bool {
        CALLBACKS_OPEN.load(Ordering::Acquire)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

fn diagnostics() -> Result<TechnologyBadgeHookDiagnostics, String> {
    let state = state_lock()?;
    Ok(TechnologyBadgeHookDiagnostics {
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
        .map_err(|_| "A28 technology badge hook state mutex poisoned".to_string())
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

fn require_positive(extent: [i32; 2], label: &str) -> Result<(), String> {
    if extent[0] <= 0 || extent[1] <= 0 {
        return Err(format!("{label} extent is not positive"));
    }
    Ok(())
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual =
        rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err("A28 technology badge callbacks did not quiesce".into());
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

fn read_i32(address: usize) -> Option<i32> {
    Some(i32::from_le_bytes(rpm(address, 4)?.try_into().ok()?))
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
