//! Signature-gated a729 producer for exact resolved tooltip instances.
//!
//! A26 owns only the generic tooltip-controller detour. Final text and owner
//! destruction arrive through A25's sole shared setter/destructor detours.

use crate::{
    rows_from_final_resolved_text, ExactTooltipInstanceSource, RectI32, ResolvedTooltipInstance,
};
use core::ffi::c_void;
use minhook::MinHook;
use runtime_text_inputs_live_lane::a729_hook::{
    install_a729_runtime_text_hooks, register_post_setter_observer,
    register_pre_destructor_observer,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const TOOLTIP_CONTROLLER_RVA: u32 = 0x002A_4080;
pub const TOOLTIP_EXTENT_ACCESSOR_RVA: u32 = 0x0228_F100;
const TEXTBOX_VTABLE_RVA: usize = 0x02B2_B418;
const CONTROLLER_TOOLTIP_OWNER_OFFSET: usize = 0x4E8;
const CONTROLLER_POSITION_OFFSET: usize = 0x4E0;
const CONTROLLER_VIEWPORT_OFFSET: usize = 0x4F0;
const CONTROLLER_HOVER_CONTEXT_OFFSET: usize = 0x4F8;
const HOVER_CONTEXT_OBJECT_OFFSET: usize = 0x140;
const VIEWPORT_WIDTH_OFFSET: usize = 0x180;
const VIEWPORT_HEIGHT_OFFSET: usize = 0x184;
const TEXTBOX_AUTHORED_NAME_OFFSET: usize = 0x98;
const CGUI_TYPE_OFFSET: usize = 0x60;
const CGUI_TYPE_NAME_OFFSET: usize = 0x28;
const GUI_VISIBLE_FLAGS_OFFSET: usize = 0x75;
const TEXTBOX_CHILD_FINAL_STRING_OFFSET: usize = 0x110;

const CONTROLLER_SENTINEL: [u8; 16] = [
    0x48, 0x8B, 0xC4, 0x48, 0x89, 0x58, 0x08, 0x88, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];

/// Function-evidence manifest for the controller, position, and extent accessors.
pub const PRODUCER_MANIFEST_SHA256: [u8; 32] =
    hex32(b"2bc2bc389cfcadb3903ad6c79a03645a5fe4b7102feac1eb982f89e6aabac9c5");

type ControllerFn = unsafe extern "system" fn(usize, u8);
type ExtentFn = unsafe extern "system" fn(usize, *mut i32) -> *mut i32;

static ORIGINAL_CONTROLLER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

thread_local! {
    static ACTIVE_CONTROLLER: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone)]
struct PendingText {
    final_text: String,
    setter_caller_rva: u32,
    hovered_type_name: String,
}

#[derive(Clone)]
struct LiveSnapshot {
    generation: u32,
    position: [i32; 2],
    extent: [i32; 2],
    viewport: [i32; 2],
    text: PendingText,
}

#[derive(Default)]
struct HookState {
    pending: HashMap<usize, PendingText>,
    active: HashMap<usize, LiveSnapshot>,
    generations: HashMap<usize, u32>,
    next_generation: u32,
    controller_calls: u64,
    setter_events: u64,
    destructor_events: u64,
    current_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub controller_calls: u64,
    pub setter_events: u64,
    pub destructor_events: u64,
    pub pending_owner_count: u64,
    pub visible_instance_count: u64,
    pub current_error: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddress {
    pub tooltip_controller: usize,
}

pub struct A729ResolvedTooltipHookSource {
    pov_country_or_global: String,
}

impl A729ResolvedTooltipHookSource {
    pub fn new(pov_country_or_global: impl Into<String>) -> Result<Self, String> {
        let pov_country_or_global = pov_country_or_global.into();
        if pov_country_or_global.trim().is_empty() {
            return Err("A26 POV country/global identity is empty".to_string());
        }
        Ok(Self {
            pov_country_or_global,
        })
    }

    pub fn diagnostics(&self) -> HookDiagnostics {
        let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
            return HookDiagnostics::default();
        };
        HookDiagnostics {
            controller_calls: state.controller_calls,
            setter_events: state.setter_events,
            destructor_events: state.destructor_events,
            pending_owner_count: state.pending.len() as u64,
            visible_instance_count: state.active.len() as u64,
            current_error: state.current_error.is_some(),
        }
    }

    fn snapshots(&self, frame_hour: u64) -> Result<Vec<ResolvedTooltipInstance>, String> {
        let state = STATE
            .get()
            .ok_or_else(|| "A26 a729 hook state is not initialized".to_string())?
            .lock()
            .map_err(|_| "A26 a729 hook state mutex poisoned".to_string())?;
        if let Some(error) = &state.current_error {
            return Err(error.clone());
        }
        if state.active.len() > 1 {
            return Err(
                "a729 global tooltip controller exposed multiple visible ToolTip owners".into(),
            );
        }
        let mut rows = state.active.values().cloned().collect::<Vec<_>>();
        rows.sort_by_key(|row| row.generation);
        rows.into_iter()
            .enumerate()
            .map(|(repeat_index, snapshot)| {
                let source_identity = format!(
                    "hovered-cgui-type:{};setter-caller-rva:0x{:08X}",
                    snapshot.text.hovered_type_name, snapshot.text.setter_caller_rva
                );
                let evaluation_context = format!(
                    "controller-rva:0x{TOOLTIP_CONTROLLER_RVA:08X};tooltip-owner-generation:{}",
                    snapshot.generation
                );
                let tooltip_rows = rows_from_final_resolved_text(
                    &snapshot.text.final_text,
                    &source_identity,
                    &evaluation_context,
                )
                .map_err(|error| format!("A26 final text row split failed: {error:?}"))?;
                Ok(ResolvedTooltipInstance {
                    frame_hour,
                    pov_country_or_global: self.pov_country_or_global.clone(),
                    view_id: "core.gui".to_string(),
                    element_path: "guiTypes/ToolTip".to_string(),
                    repeat_index: u32::try_from(repeat_index)
                        .map_err(|_| "A26 repeat index overflow".to_string())?,
                    screen_rect: RectI32 {
                        x: snapshot.position[0],
                        y: snapshot.position[1],
                        width: snapshot.extent[0],
                        height: snapshot.extent[1],
                    },
                    viewport_width_px: snapshot.viewport[0],
                    viewport_height_px: snapshot.viewport[1],
                    producer_record_identity: format!(
                        "a26:a729:controller=0x{TOOLTIP_CONTROLLER_RVA:08X}:generation={}:setter=0x{:08X}",
                        snapshot.generation, snapshot.text.setter_caller_rva
                    ),
                    rows: tooltip_rows,
                })
            })
            .collect()
    }
}

impl ExactTooltipInstanceSource for A729ResolvedTooltipHookSource {
    fn is_exact_a729_tooltip_source(&self) -> bool {
        INSTALLED.load(Ordering::Acquire)
    }

    fn producer_manifest_sha256(&self) -> [u8; 32] {
        PRODUCER_MANIFEST_SHA256
    }

    fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String> {
        let diagnostics = self.diagnostics();
        if diagnostics.current_error {
            let state = STATE
                .get()
                .ok_or_else(|| "A26 a729 hook state is not initialized".to_string())?
                .lock()
                .map_err(|_| "A26 a729 hook state mutex poisoned".to_string())?;
            return Err(state
                .current_error
                .clone()
                .unwrap_or_else(|| "A26 current completeness error".to_string()));
        }
        Ok(format!(
            "a26:a729:{frame_hour}:controller_calls={}:setter_events={}:visible={}",
            diagnostics.controller_calls,
            diagnostics.setter_events,
            diagnostics.visible_instance_count
        ))
    }

    fn expected_instance_count(&self, _: u64) -> Result<u32, String> {
        let count = self.snapshots(0)?.len();
        u32::try_from(count).map_err(|_| "A26 instance count exceeds u32".to_string())
    }

    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<ResolvedTooltipInstance>, String> {
        self.snapshots(frame_hour)
    }
}

/// Install the exact controller hook after A25 has authenticated and installed
/// its sole setter/destructor hooks. No hook is created on a sentinel mismatch.
pub fn install_a729_resolved_tooltip_hook() -> Result<InstalledHookAddress, String> {
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(InstalledHookAddress {
            tooltip_controller: MODULE_BASE.load(Ordering::Acquire)
                + TOOLTIP_CONTROLLER_RVA as usize,
        });
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".to_string());
    }
    let controller = base + TOOLTIP_CONTROLLER_RVA as usize;
    verify_sentinel(controller, &CONTROLLER_SENTINEL, "tooltip controller")?;
    install_a729_runtime_text_hooks()?;
    STATE.get_or_init(|| Mutex::new(HookState::default()));

    let original = unsafe {
        MinHook::create_hook(
            controller as *mut c_void,
            hook_tooltip_controller as *mut c_void,
        )
    }
    .map_err(|status| format!("A26 tooltip controller hook creation failed: {status}"))?;
    ORIGINAL_CONTROLLER.store(original as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);

    if let Err(error) = register_post_setter_observer(post_setter_observer) {
        let _ = unsafe { MinHook::remove_hook(controller as *mut c_void) };
        return Err(error);
    }
    if let Err(error) = register_pre_destructor_observer(pre_destructor_observer) {
        let _ = unsafe { MinHook::remove_hook(controller as *mut c_void) };
        return Err(error);
    }
    if let Err(status) = unsafe { MinHook::enable_hook(controller as *mut c_void) } {
        let _ = unsafe { MinHook::remove_hook(controller as *mut c_void) };
        return Err(format!(
            "A26 tooltip controller hook enable failed: {status}"
        ));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(InstalledHookAddress {
        tooltip_controller: controller,
    })
}

unsafe extern "system" fn hook_tooltip_controller(controller: usize, update: u8) {
    let previous = ACTIVE_CONTROLLER.with(|active| active.replace(controller));
    let original = ORIGINAL_CONTROLLER.load(Ordering::Acquire);
    if original != 0 {
        let original: ControllerFn = unsafe { mem::transmute(original) };
        unsafe { original(controller, update) };
    }
    snapshot_after_controller(controller);
    ACTIVE_CONTROLLER.with(|active| active.set(previous));
}

unsafe extern "system" fn post_setter_observer(owner: usize, setter_child: usize, caller_rva: u32) {
    if !is_tooltip_owner(owner) {
        return;
    }
    let controller = ACTIVE_CONTROLLER.with(Cell::get);
    let result = (|| {
        if controller == 0 {
            return Err(
                "ToolTip setter executed outside authenticated controller scope".to_string(),
            );
        }
        let final_text = read_engine_string(
            setter_child
                .checked_add(TEXTBOX_CHILD_FINAL_STRING_OFFSET)
                .ok_or_else(|| "ToolTip child text address overflow".to_string())?,
        )
        .ok_or_else(|| "ToolTip final engine string unreadable".to_string())?;
        let final_text = String::from_utf8(final_text)
            .map_err(|_| "ToolTip final engine string is not UTF-8".to_string())?;
        let hovered_type_name = hovered_type_name(controller)
            .ok_or_else(|| "ToolTip hovered CGuiType name unreadable".to_string())?;
        Ok(PendingText {
            final_text,
            setter_caller_rva: caller_rva,
            hovered_type_name,
        })
    })();
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.setter_events = state.setter_events.saturating_add(1);
        match result {
            Ok(text) => {
                state.pending.insert(owner, text);
                state.current_error = None;
            }
            Err(error) => state.current_error = Some(error),
        }
    }
}

unsafe extern "system" fn pre_destructor_observer(owner: usize, _: u32) {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.pending.remove(&owner);
        state.active.remove(&owner);
        state.generations.remove(&owner);
        state.destructor_events = state.destructor_events.saturating_add(1);
        state.current_error = None;
    }
}

fn snapshot_after_controller(controller: usize) {
    let result = snapshot_values(controller);
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.controller_calls = state.controller_calls.saturating_add(1);
        match result {
            Ok(SnapshotResult::Hidden(owner)) => {
                state.active.remove(&owner);
                state.current_error = None;
            }
            Ok(SnapshotResult::Visible {
                owner,
                position,
                extent,
                viewport,
            }) => {
                let Some(text) = state.pending.get(&owner).cloned() else {
                    state.active.remove(&owner);
                    state.current_error = Some(
                        "visible ToolTip has no final-setter record in the authenticated controller scope"
                            .to_string(),
                    );
                    return;
                };
                let generation = if let Some(generation) = state.generations.get(&owner) {
                    *generation
                } else {
                    let generation = state.next_generation;
                    state.next_generation = state.next_generation.saturating_add(1);
                    state.generations.insert(owner, generation);
                    generation
                };
                state.active.clear();
                state.active.insert(
                    owner,
                    LiveSnapshot {
                        generation,
                        position,
                        extent,
                        viewport,
                        text,
                    },
                );
                state.current_error = None;
            }
            Err(error) => {
                state.active.clear();
                state.current_error = Some(error);
            }
        }
    }
}

enum SnapshotResult {
    Hidden(usize),
    Visible {
        owner: usize,
        position: [i32; 2],
        extent: [i32; 2],
        viewport: [i32; 2],
    },
}

fn snapshot_values(controller: usize) -> Result<SnapshotResult, String> {
    let owner = read_usize(
        controller
            .checked_add(CONTROLLER_TOOLTIP_OWNER_OFFSET)
            .ok_or_else(|| "tooltip owner address overflow".to_string())?,
    )
    .ok_or_else(|| "tooltip owner unreadable".to_string())?;
    if !is_tooltip_owner(owner) {
        return Err("controller ToolTip owner failed exact vtable/name validation".into());
    }
    let flags = read_u8(
        owner
            .checked_add(GUI_VISIBLE_FLAGS_OFFSET)
            .ok_or_else(|| "tooltip flags address overflow".to_string())?,
    )
    .ok_or_else(|| "tooltip visibility flags unreadable".to_string())?;
    if flags & 0x08 == 0 {
        return Ok(SnapshotResult::Hidden(owner));
    }
    let position = read_i32_pair(
        controller
            .checked_add(CONTROLLER_POSITION_OFFSET)
            .ok_or_else(|| "tooltip position address overflow".to_string())?,
    )
    .ok_or_else(|| "tooltip final packed position unreadable".to_string())?;
    let viewport_owner = read_usize(
        controller
            .checked_add(CONTROLLER_VIEWPORT_OFFSET)
            .ok_or_else(|| "tooltip viewport address overflow".to_string())?,
    )
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
    Ok(SnapshotResult::Visible {
        owner,
        position,
        extent,
        viewport,
    })
}

fn hovered_type_name(controller: usize) -> Option<String> {
    let context = read_usize(controller.checked_add(CONTROLLER_HOVER_CONTEXT_OFFSET)?)?;
    let object = read_usize(context.checked_add(HOVER_CONTEXT_OBJECT_OFFSET)?)?;
    let gui_type = read_usize(object.checked_add(CGUI_TYPE_OFFSET)?)?;
    String::from_utf8(read_engine_string(
        gui_type.checked_add(CGUI_TYPE_NAME_OFFSET)?,
    )?)
    .ok()
}

fn is_tooltip_owner(owner: usize) -> bool {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 || read_usize(owner) != base.checked_add(TEXTBOX_VTABLE_RVA) {
        return false;
    }
    let cached = read_engine_string(owner + TEXTBOX_AUTHORED_NAME_OFFSET)
        .and_then(|bytes| String::from_utf8(bytes).ok());
    if cached.as_deref() == Some("ToolTip") {
        return true;
    }
    let type_name = read_usize(owner + CGUI_TYPE_OFFSET)
        .and_then(|gui_type| read_engine_string(gui_type + CGUI_TYPE_NAME_OFFSET))
        .and_then(|bytes| String::from_utf8(bytes).ok());
    type_name.as_deref() == Some("ToolTip")
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
