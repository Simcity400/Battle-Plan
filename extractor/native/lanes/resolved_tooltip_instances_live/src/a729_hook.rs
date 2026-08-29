use crate::{
    ExactTooltipInstanceSource, RectI32, ResolvedTooltipInstance, SelectedPresentStamp,
    TooltipTraceFrame,
};
use core::ffi::c_void;
use minhook::MinHook;
use std::collections::VecDeque;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const TOOLTIP_CONTROLLER_RVA: u32 = 0x002A_4080;
pub const TOOLTIP_EXTENT_ACCESSOR_RVA: u32 = 0x0228_F100;
pub const TOOLTIP_FINAL_TEXT_SETTER_RVA: u32 = 0x0228_EE80;
pub const IN_GAME_IDLER_VTABLE_RVA: u32 = 0x0295_2510;
pub const IN_GAME_IDLER_UPDATE_SLOT_RVA: u32 = 0x0295_2530;
pub const IN_GAME_IDLER_UPDATE_RVA: u32 = 0x00DB_F240;
const TEXTBOX_VTABLE_RVA: usize = 0x02B2_B418;
const CONTROLLER_TOOLTIP_OWNER_OFFSET: usize = 0x4E8;
const CONTROLLER_POSITION_OFFSET: usize = 0x4E0;
const CONTROLLER_VIEWPORT_OFFSET: usize = 0x4F0;
const CONTROLLER_HOVER_CONTEXT_OFFSET: usize = 0x4F8;
const HOVER_CONTEXT_OBJECT_OFFSET: usize = 0x140;
const VIEWPORT_WIDTH_OFFSET: usize = 0x180;
const VIEWPORT_HEIGHT_OFFSET: usize = 0x184;
const TEXTBOX_AUTHORED_NAME_OFFSET: usize = 0x98;
const TEXTBOX_FINAL_TEXT_OFFSET: usize = 0x130;
const CGUI_TYPE_OFFSET: usize = 0x60;
const CGUI_TYPE_NAME_OFFSET: usize = 0x28;
const GUI_VISIBLE_FLAGS_OFFSET: usize = 0x75;
const MAX_PENDING_OBSERVATIONS: usize = 4_096;

const CONTROLLER_SENTINEL: [u8; 16] = [
    0x48, 0x8B, 0xC4, 0x48, 0x89, 0x58, 0x08, 0x88, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
const EXTENT_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0x89, 0x88, 0x00, 0x00, 0x00, 0x48, 0x8B, 0xDA,
];
const FINAL_TEXT_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7C, 0x24, 0x18, 0x4C,
];

pub const PRODUCER_MANIFEST_SHA256: [u8; 32] =
    hex32(b"a23b63ccefc021eeced9a1a918588d3e2fbba07e5d1afb4d53f83cc7c1640177");
pub const NONSUSPENDING_SOURCE_MANIFEST_SHA256: [u8; 32] =
    hex32(b"d1cd35e661d9184f99e7efa3c8a7a92c5cf5976fd15df10bc449804c5f1bdb44");

type ControllerFn = unsafe extern "system" fn(usize, u8);
type ExtentFn = unsafe extern "system" fn(usize, *mut i32) -> *mut i32;

static ORIGINAL_CONTROLLER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static INSTALL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

#[derive(Clone)]
struct VisibleObservation {
    sequence: u64,
    qpc_ticks: u64,
    owner_generation: u64,
    position: [i32; 2],
    extent: [i32; 2],
    viewport: [i32; 2],
    hovered_element_authored_name: String,
    tooltip_owner_authored_name: String,
    final_resolved_text: String,
}

#[derive(Clone)]
enum ControllerObservation {
    Hidden { sequence: u64, qpc_ticks: u64 },
    Visible(VisibleObservation),
}

compact_contract::wire_counters! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct D72StateCounters, prefix = "d72_hook_";

    controller_calls,
    observation_sequence,
    owner_generation,
}

#[derive(Default)]
struct HookState {
    counters: D72StateCounters,
    observations: VecDeque<ControllerObservation>,
    last_sealed_present_qpc_ticks: u64,
    last_owner_address: usize,
    poison: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub installed: bool,
    pub install_epoch: u64,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub controller_calls: u64,
    pub observation_sequence: u64,
    pub owner_generation: u64,
    pub has_observed_baseline: bool,
    pub poisoned: bool,
}

pub struct A729ResolvedTooltipHookOwner {
    target: usize,
    epoch: usize,
    active: bool,
}

impl A729ResolvedTooltipHookOwner {
    pub fn install() -> Result<Self, String> {
        let _install = INSTALL_LOCK
            .lock()
            .map_err(|_| "A26 install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("A26 tooltip hook already has a live owner".into());
        }
        let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
        if base == 0 {
            return Err("GetModuleHandleW(NULL) returned null".to_string());
        }
        let target = base + TOOLTIP_CONTROLLER_RVA as usize;
        let extent = base + TOOLTIP_EXTENT_ACCESSOR_RVA as usize;
        let final_text_setter = base + TOOLTIP_FINAL_TEXT_SETTER_RVA as usize;
        verify_sentinel(target, &CONTROLLER_SENTINEL, "tooltip controller")?;
        verify_sentinel(extent, &EXTENT_SENTINEL, "tooltip extent accessor")?;
        verify_sentinel(
            final_text_setter,
            &FINAL_TEXT_SETTER_SENTINEL,
            "tooltip final-text setter",
        )?;
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "A26 hook install epoch exhausted".to_string())?
            + 1;
        {
            let mut state = STATE
                .get_or_init(|| Mutex::new(HookState::default()))
                .lock()
                .map_err(|_| "A26 hook state mutex poisoned".to_string())?;
            *state = HookState::default();
        }
        let original = unsafe {
            MinHook::create_hook(
                target as *mut c_void,
                hook_tooltip_controller as *mut c_void,
            )
        }
        .map_err(|status| format!("A26 tooltip controller hook creation failed: {status}"))?;
        ORIGINAL_CONTROLLER.store(original as usize, Ordering::Release);
        MODULE_BASE.store(base, Ordering::Release);
        CALLBACKS_OPEN.store(true, Ordering::Release);
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            CALLBACKS_OPEN.store(false, Ordering::Release);
            let _ = unsafe { inline_detour::disable(target as *mut c_void) };
            if let Err(quiescence) = wait_for_quiescence(Duration::from_secs(30)) {
                return Err(format!(
                    "A26 tooltip controller hook enable failed: {status}; detour retained: {quiescence}"
                ));
            }
            if let Err(removal) = unsafe { inline_detour::remove(target as *mut c_void) } {
                return Err(format!(
                    "A26 tooltip controller hook enable failed: {status}; detour retained, removal failed: {removal}"
                ));
            }
            ORIGINAL_CONTROLLER.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!(
                "A26 tooltip controller hook enable failed: {status}"
            ));
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(Self {
            target,
            epoch,
            active: true,
        })
    }

    pub fn source(
        &self,
        pov_country_or_global: impl Into<String>,
    ) -> Result<A729ResolvedTooltipHookSource, String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A26 hook owner is not active".into());
        }
        A729ResolvedTooltipHookSource::new(self.epoch, pov_country_or_global)
    }

    pub fn reset_campaign_observations(&self) -> Result<(), String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A26 hook owner is not active".into());
        }
        let mut state = STATE
            .get_or_init(|| Mutex::new(HookState::default()))
            .lock()
            .map_err(|_| "A26 hook state mutex poisoned".to_string())?;
        clear_campaign_observations(&mut state);
        Ok(())
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _install = INSTALL_LOCK
            .lock()
            .map_err(|_| "A26 install mutex poisoned".to_string())?;
        CALLBACKS_OPEN.store(false, Ordering::Release);
        unsafe { inline_detour::disable(self.target as *mut c_void) }
            .map_err(|status| format!("A26 controller disable failed: {status}"))?;
        wait_for_quiescence(Duration::from_secs(30))?;
        unsafe { inline_detour::remove(self.target as *mut c_void) }
            .map_err(|status| format!("A26 controller removal failed: {status}"))?;
        ORIGINAL_CONTROLLER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        self.active = false;
        Ok(())
    }
}

fn clear_campaign_observations(state: &mut HookState) {
    *state = HookState::default();
}

impl Drop for A729ResolvedTooltipHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

pub struct A729ResolvedTooltipHookSource {
    install_epoch: usize,
    pov_country_or_global: String,
}

impl A729ResolvedTooltipHookSource {
    fn new(install_epoch: usize, pov_country_or_global: impl Into<String>) -> Result<Self, String> {
        let pov_country_or_global = pov_country_or_global.into();
        if !crate::valid_pov_country_or_global(&pov_country_or_global) {
            return Err("A26 POV country/global identity is not portable".to_string());
        }
        Ok(Self {
            install_epoch,
            pov_country_or_global,
        })
    }
}

impl ExactTooltipInstanceSource for A729ResolvedTooltipHookSource {
    fn is_exact_a729_tooltip_source(&self) -> bool {
        INSTALLED.load(Ordering::Acquire)
            && CALLBACKS_OPEN.load(Ordering::Acquire)
            && MODULE_BASE.load(Ordering::Acquire) != 0
            && INSTALL_EPOCH.load(Ordering::Acquire) == self.install_epoch
    }

    fn producer_manifest_sha256(&self) -> [u8; 32] {
        PRODUCER_MANIFEST_SHA256
    }

    fn capture_selected_present(
        &self,
        selected_present: &SelectedPresentStamp,
        elapsed_wall_ms: u64,
        status_flags: u32,
    ) -> Result<TooltipTraceFrame, String> {
        let _install = INSTALL_LOCK
            .lock()
            .map_err(|_| "A26 install mutex poisoned".to_string())?;
        if !self.is_exact_a729_tooltip_source() {
            return Err("A26 hook source is not owned by an active installation".into());
        }
        if status_flags != 0 {
            return Err("A26 successful source cannot emit compact failure flags".into());
        }
        if selected_present.selected_present_candidate_id == 0
            || selected_present.selected_present_qpc_ticks == 0
        {
            return Err("A26 selected Present identity is incomplete".into());
        }
        crate::validate_sha256_hex(
            &selected_present.selected_present_receipt_sha256,
            "selected Present receipt",
        )
        .map_err(|error| format!("{error:?}"))?;
        let mut state = STATE
            .get()
            .ok_or_else(|| "A26 hook state is not initialized".to_string())?
            .lock()
            .map_err(|_| "A26 hook state mutex poisoned".to_string())?;
        if let Some(error) = &state.poison {
            return Err(error.clone());
        }
        let observation = state
            .observations
            .iter()
            .filter(|observation| {
                let qpc = observation_qpc(observation);
                qpc > state.last_sealed_present_qpc_ticks
                    && qpc <= selected_present.selected_present_qpc_ticks
            })
            .max_by_key(|observation| {
                (
                    observation_qpc(observation),
                    observation_sequence(observation),
                )
            })
            .cloned();
        let Some(observation) = observation else {
            return Err(if state.counters.controller_calls == 0 {
                "A26 has no controller observation; startup silence is unobtained".into()
            } else {
                "A26 has no controller observation inside the selected-Present interval".into()
            });
        };
        let (sequence, observation_qpc_ticks, instances) = match observation {
            ControllerObservation::Hidden {
                sequence,
                qpc_ticks,
            } => (sequence, qpc_ticks, Vec::new()),
            ControllerObservation::Visible(value) => {
                let instance = ResolvedTooltipInstance {
                    frame_hour: selected_present.frame_hour,
                    pov_country_or_global: self.pov_country_or_global.clone(),
                    hovered_element_authored_name: value.hovered_element_authored_name,
                    tooltip_owner_authored_name: value.tooltip_owner_authored_name,
                    repeat_index: 0,
                    screen_rect: RectI32 {
                        x: value.position[0],
                        y: value.position[1],
                        width: value.extent[0],
                        height: value.extent[1],
                    },
                    viewport_width_px: value.viewport[0],
                    viewport_height_px: value.viewport[1],
                    producer_record_identity: format!(
                        "a26:a729:controller=0x{TOOLTIP_CONTROLLER_RVA:08X}:owner-generation={}:observation={}:qpc={}",
                        value.owner_generation, value.sequence, value.qpc_ticks
                    ),
                    final_resolved_text: value.final_resolved_text,
                };
                (value.sequence, value.qpc_ticks, vec![instance])
            }
        };
        state.last_sealed_present_qpc_ticks = selected_present.selected_present_qpc_ticks;
        state.observations.retain(|observation| {
            observation_qpc(observation) > selected_present.selected_present_qpc_ticks
        });
        Ok(TooltipTraceFrame {
            elapsed_wall_ms,
            status_flags,
            frame_hour: selected_present.frame_hour,
            selected_present_candidate_id: selected_present.selected_present_candidate_id,
            selected_present_ordinal: selected_present.selected_present_ordinal,
            selected_present_qpc_ticks: selected_present.selected_present_qpc_ticks,
            selected_present_receipt_sha256: selected_present
                .selected_present_receipt_sha256
                .clone(),
            producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
            completeness_record_identity: format!(
                "a26:a729:selected-present={}:observation={}:observation-qpc={}",
                selected_present.selected_present_candidate_id, sequence, observation_qpc_ticks
            ),
            expected_instance_count: instances.len() as u32,
            instances,
        })
    }
}

pub fn diagnostics() -> HookDiagnostics {
    let state = STATE.get().and_then(|state| state.lock().ok());
    HookDiagnostics {
        installed: INSTALLED.load(Ordering::Acquire),
        install_epoch: INSTALL_EPOCH.load(Ordering::Acquire) as u64,
        callbacks_open: CALLBACKS_OPEN.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT_CALLBACKS.load(Ordering::Acquire),
        controller_calls: state
            .as_ref()
            .map_or(0, |state| state.counters.controller_calls),
        observation_sequence: state
            .as_ref()
            .map_or(0, |state| state.counters.observation_sequence),
        owner_generation: state
            .as_ref()
            .map_or(0, |state| state.counters.owner_generation),
        has_observed_baseline: state
            .as_ref()
            .is_some_and(|state| state.counters.controller_calls > 0),
        poisoned: state.as_ref().is_some_and(|state| state.poison.is_some()),
    }
}

struct InFlightGuard {
    admitted: bool,
}

impl InFlightGuard {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self {
            admitted: CALLBACKS_OPEN.load(Ordering::Acquire),
        }
    }

    fn may_publish(&self) -> bool {
        self.admitted && CALLBACKS_OPEN.load(Ordering::Acquire)
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

unsafe extern "system" fn hook_tooltip_controller(controller: usize, update: u8) {
    let guard = InFlightGuard::enter();
    let original = ORIGINAL_CONTROLLER.load(Ordering::Acquire);
    if original != 0 {
        let original: ControllerFn = unsafe { mem::transmute(original) };
        unsafe { original(controller, update) };
    }
    if guard.may_publish() {
        publish_controller_observation(controller);
    }
}

fn publish_controller_observation(controller: usize) {
    let result = snapshot_values(controller);
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.counters.bump(|counters| &mut counters.controller_calls);
    let sequence = state
        .counters
        .bump(|counters| &mut counters.observation_sequence);
    match result {
        Ok(SnapshotResult::Hidden {
            qpc_ticks,
            owner_address,
        }) => {
            update_owner_generation(&mut state, owner_address);
            push_observation(
                &mut state,
                ControllerObservation::Hidden {
                    sequence,
                    qpc_ticks,
                },
            );
        }
        Ok(SnapshotResult::Visible {
            qpc_ticks,
            owner_address,
            position,
            extent,
            viewport,
            hovered_element_authored_name,
            tooltip_owner_authored_name,
            final_resolved_text,
        }) => {
            let owner_generation = update_owner_generation(&mut state, owner_address);
            push_observation(
                &mut state,
                ControllerObservation::Visible(VisibleObservation {
                    sequence,
                    qpc_ticks,
                    owner_generation,
                    position,
                    extent,
                    viewport,
                    hovered_element_authored_name,
                    tooltip_owner_authored_name,
                    final_resolved_text,
                }),
            );
        }
        Err(error) => {
            state.poison.get_or_insert(error);
            state.observations.clear();
        }
    }
}

fn observation_qpc(observation: &ControllerObservation) -> u64 {
    match observation {
        ControllerObservation::Hidden { qpc_ticks, .. } => *qpc_ticks,
        ControllerObservation::Visible(observation) => observation.qpc_ticks,
    }
}

fn observation_sequence(observation: &ControllerObservation) -> u64 {
    match observation {
        ControllerObservation::Hidden { sequence, .. } => *sequence,
        ControllerObservation::Visible(observation) => observation.sequence,
    }
}

fn push_observation(state: &mut HookState, observation: ControllerObservation) {
    if observation_qpc(&observation) <= state.last_sealed_present_qpc_ticks {
        return;
    }
    if state.observations.len() == MAX_PENDING_OBSERVATIONS {
        state
            .poison
            .get_or_insert_with(|| "A26 controller observation queue overflowed".into());
        state.observations.clear();
        return;
    }
    state.observations.push_back(observation);
}

fn update_owner_generation(state: &mut HookState, owner_address: usize) -> u64 {
    if state.last_owner_address != owner_address {
        state.last_owner_address = owner_address;
        state.counters.bump(|counters| &mut counters.owner_generation);
    }
    state.counters.owner_generation
}

enum SnapshotResult {
    Hidden {
        qpc_ticks: u64,
        owner_address: usize,
    },
    Visible {
        qpc_ticks: u64,
        owner_address: usize,
        position: [i32; 2],
        extent: [i32; 2],
        viewport: [i32; 2],
        hovered_element_authored_name: String,
        tooltip_owner_authored_name: String,
        final_resolved_text: String,
    },
}

fn snapshot_values(controller: usize) -> Result<SnapshotResult, String> {
    let qpc_ticks = qpc_ticks()?;
    let owner = read_usize(controller + CONTROLLER_TOOLTIP_OWNER_OFFSET)
        .ok_or_else(|| "tooltip owner unreadable".to_string())?;
    let tooltip_owner_authored_name = validate_tooltip_owner(owner)?;
    let flags = read_u8(owner + GUI_VISIBLE_FLAGS_OFFSET)
        .ok_or_else(|| "tooltip visibility flags unreadable".to_string())?;
    if flags & 0x08 == 0 {
        return Ok(SnapshotResult::Hidden {
            qpc_ticks,
            owner_address: owner,
        });
    }
    let position = read_i32_pair(controller + CONTROLLER_POSITION_OFFSET)
        .ok_or_else(|| "tooltip final packed position unreadable".to_string())?;
    let viewport_owner = read_usize(controller + CONTROLLER_VIEWPORT_OFFSET)
        .ok_or_else(|| "tooltip viewport owner unreadable".to_string())?;
    let viewport = [
        read_i32(viewport_owner + VIEWPORT_WIDTH_OFFSET)
            .ok_or_else(|| "tooltip viewport width unreadable".to_string())?,
        read_i32(viewport_owner + VIEWPORT_HEIGHT_OFFSET)
            .ok_or_else(|| "tooltip viewport height unreadable".to_string())?,
    ];
    let mut extent = [0i32; 2];
    let base = MODULE_BASE.load(Ordering::Acquire);
    let accessor: ExtentFn = unsafe { mem::transmute(base + TOOLTIP_EXTENT_ACCESSOR_RVA as usize) };
    unsafe { accessor(owner, extent.as_mut_ptr()) };
    if extent[0] <= 0 || extent[1] <= 0 || viewport[0] <= 0 || viewport[1] <= 0 {
        return Err("tooltip extent or viewport is nonpositive".into());
    }
    let final_resolved_text = read_engine_string(owner + TEXTBOX_FINAL_TEXT_OFFSET)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            "visible tooltip CTextBox final post-layout text unreadable or empty".to_string()
        })?;
    let hovered_element_authored_name = hovered_type_name(controller)
        .ok_or_else(|| "tooltip hovered authored CGuiType name unreadable".to_string())?;
    Ok(SnapshotResult::Visible {
        qpc_ticks,
        owner_address: owner,
        position,
        extent,
        viewport,
        hovered_element_authored_name,
        tooltip_owner_authored_name,
        final_resolved_text,
    })
}

fn validate_tooltip_owner(owner: usize) -> Result<String, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 || read_usize(owner) != base.checked_add(TEXTBOX_VTABLE_RVA) {
        return Err("controller ToolTip owner failed exact CTextBox vtable validation".into());
    }
    let name = read_engine_string(owner + TEXTBOX_AUTHORED_NAME_OFFSET)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| "tooltip owner authored name unreadable".to_string())?;
    if name != "ToolTip" {
        return Err("controller tooltip owner authored name is not ToolTip".into());
    }
    Ok(name)
}

fn hovered_type_name(controller: usize) -> Option<String> {
    let context = read_usize(controller.checked_add(CONTROLLER_HOVER_CONTEXT_OFFSET)?)?;
    let object = read_usize(context.checked_add(HOVER_CONTEXT_OBJECT_OFFSET)?)?;
    let gui_type = read_usize(object.checked_add(CGUI_TYPE_OFFSET)?)?;
    String::from_utf8(read_engine_string(
        gui_type.checked_add(CGUI_TYPE_NAME_OFFSET)?,
    )?)
    .ok()
    .filter(|name| !name.is_empty())
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    while IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) != 0 {
        if started.elapsed() >= timeout {
            return Err("A26 controller callbacks did not quiesce".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

fn qpc_ticks() -> Result<u64, String> {
    let mut value = 0i64;
    if unsafe { QueryPerformanceCounter(&mut value) } == 0 || value <= 0 {
        return Err("QueryPerformanceCounter failed".into());
    }
    Ok(value as u64)
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual =
        rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn read_engine_string(address: usize) -> Option<Vec<u8>> {
    let len = read_usize(address.checked_add(0x10)?)?;
    let capacity = read_usize(address.checked_add(0x18)?)?;
    if len > capacity || len > crate::MAX_STRING_BYTES {
        return None;
    }
    if len == 0 {
        return Some(Vec::new());
    }
    let data = if capacity <= 15 {
        address
    } else {
        read_usize(address)?
    };
    rpm(data, len)
}

fn read_u8(address: usize) -> Option<u8> {
    Some(*rpm(address, 1)?.first()?)
}

fn read_i32(address: usize) -> Option<i32> {
    Some(i32::from_le_bytes(rpm(address, 4)?.try_into().ok()?))
}

fn read_i32_pair(address: usize) -> Option<[i32; 2]> {
    let bytes = rpm(address, 8)?;
    Some([
        i32::from_le_bytes(bytes[0..4].try_into().ok()?),
        i32::from_le_bytes(bytes[4..8].try_into().ok()?),
    ])
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

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0u8; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    out
}

#[cfg(test)]
mod campaign_reset_tests {
    use super::*;

    #[test]
    fn campaign_reset_drops_queue_frontier_and_poison() {
        let mut state = HookState::default();
        state.last_sealed_present_qpc_ticks = 99;
        state.counters.controller_calls = 7;
        state.poison = Some("old campaign".into());
        clear_campaign_observations(&mut state);
        assert_eq!(state.last_sealed_present_qpc_ticks, 0);
        assert_eq!(state.counters.controller_calls, 0);
        assert!(state.observations.is_empty());
        assert!(state.poison.is_none());
    }
}
