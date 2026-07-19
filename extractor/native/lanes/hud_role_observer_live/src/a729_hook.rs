//! Signature-gated post-setup observers for the four proven a729 HUD roots.

use crate::{
    HudSurface, RawHudControllerRoot, MAPMODE_SETUP_RVA, MINIMAP_SETUP_RVA, THEATRE_SETUP_RVA,
    TOPBAR_SETUP_RVA,
};
use core::cell::Cell;
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::collections::BTreeMap;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const TOPBAR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
];
const MINIMAP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
const MAPMODE_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0x6c,
];
const THEATRE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56, 0x48, 0x8b,
];

const TOPBAR_RUNTIME_ROOT: usize = 0x78;
const MINIMAP_RUNTIME_ROOT: usize = 0x1490;
const MAPMODE_RUNTIME_ROOT: usize = 0x2ed8;
const THEATRE_RUNTIME_ROOT: usize = 0x58;
const RUNTIME_DEFINITION: usize = 0x15d0;
const DEFINITION_ROOT_NAME_VSLOT: usize = 0x48;
const MAX_MODULE_IMAGE_SPAN: usize = 0x0400_0000;
const MAX_MSVC_STRING_CAPACITY: usize = 1 << 20;

type SetupFn = unsafe extern "system" fn(usize);
type DefinitionRootNameFn = unsafe extern "system" fn(usize) -> usize;

static ORIGINAL_TOPBAR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MINIMAP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MAPMODE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_THEATRE: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

thread_local! {
    static ACTIVE_CALLBACKS: Cell<u8> = const { Cell::new(0) };
}

#[derive(Default)]
struct HookState {
    latest: BTreeMap<HudSurface, RawHudControllerRoot>,
    callback_count: u64,
    rejected_observation_count: u64,
    current_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub topbar_setup: usize,
    pub minimap_setup: usize,
    pub mapmode_setup: usize,
    pub theatre_setup: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub callback_count: u64,
    pub rejected_observation_count: u64,
    pub authenticated_surface_count: usize,
    pub current_error: Option<String>,
}

pub fn install() -> Result<InstalledAddresses, String> {
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let resolved = addresses(base);
    for (address, sentinel, label) in [
        (resolved.topbar_setup, &TOPBAR_SENTINEL, "topbar setup"),
        (resolved.minimap_setup, &MINIMAP_SENTINEL, "minimap setup"),
        (resolved.mapmode_setup, &MAPMODE_SENTINEL, "mapmode setup"),
        (resolved.theatre_setup, &THEATRE_SENTINEL, "theatre setup"),
    ] {
        verify_sentinel(address, sentinel, label)?;
    }
    STATE.get_or_init(|| Mutex::new(HookState::default()));
    let specs = [
        (
            resolved.topbar_setup,
            hook_topbar as *mut c_void,
            &ORIGINAL_TOPBAR,
        ),
        (
            resolved.minimap_setup,
            hook_minimap as *mut c_void,
            &ORIGINAL_MINIMAP,
        ),
        (
            resolved.mapmode_setup,
            hook_mapmode as *mut c_void,
            &ORIGINAL_MAPMODE,
        ),
        (
            resolved.theatre_setup,
            hook_theatre as *mut c_void,
            &ORIGINAL_THEATRE,
        ),
    ];
    let mut created = Vec::new();
    for (target, detour, original) in specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(value) => {
                original.store(value as usize, Ordering::Release);
                created.push(target);
            }
            Err(status) => {
                for target in created {
                    let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
                }
                clear_originals();
                return Err(format!("HUD {target:#x} hook creation failed: {status}"));
            }
        }
    }
    for target in targets(resolved) {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            for target in targets(resolved) {
                let _ = unsafe { MinHook::disable_hook(target as *mut c_void) };
                let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
            }
            clear_originals();
            return Err(format!("HUD {target:#x} hook enable failed: {status}"));
        }
    }
    MODULE_BASE.store(base, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(resolved)
}

pub fn uninstall() -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let resolved = addresses(MODULE_BASE.load(Ordering::Acquire));
    for target in targets(resolved) {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("HUD {target:#x} hook disable failed: {status}")),
        }
    }
    for target in targets(resolved) {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("HUD {target:#x} hook removal failed: {status}")),
        }
    }
    INSTALLED.store(false, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    clear_originals();
    if let Some(state) = STATE.get() {
        *state.lock().map_err(|_| "HUD hook state poisoned")? = HookState::default();
    }
    Ok(())
}

/// Returns one post-setup root for every proven HUD surface. Any callback/read
/// failure rejects the drain; partial root sets remain queued until complete.
pub fn drain_complete_roots() -> Result<Vec<RawHudControllerRoot>, String> {
    let roots = latest_complete_roots()?;
    let mut state = STATE
        .get()
        .ok_or("HUD hook state is unavailable")?
        .lock()
        .map_err(|_| "HUD hook state poisoned")?;
    state.latest.clear();
    Ok(roots)
}

/// Non-consuming snapshot used by the compact activation carrier. Repeated
/// hourly frames retain the last authenticated setup roots without leaking any
/// pointer into the wire payload.
pub fn latest_complete_roots() -> Result<Vec<RawHudControllerRoot>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 HUD hooks are not installed".into());
    }
    let state = STATE
        .get()
        .ok_or("HUD hook state is unavailable")?
        .lock()
        .map_err(|_| "HUD hook state poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    if state.latest.len() != 4 {
        return Err(format!(
            "HUD root set is incomplete: authenticated {}/4 surfaces",
            state.latest.len()
        ));
    }
    let mut roots = state.latest.values().cloned().collect::<Vec<_>>();
    roots.sort_by_key(|root| root.sequence);
    Ok(roots)
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics::default();
    };
    HookDiagnostics {
        callback_count: state.callback_count,
        rejected_observation_count: state.rejected_observation_count,
        authenticated_surface_count: state.latest.len(),
        current_error: state.current_error.clone(),
    }
}

unsafe extern "system" fn hook_topbar(controller: usize) {
    call_original_and_capture(
        controller,
        &ORIGINAL_TOPBAR,
        HudSurface::Topbar,
        TOPBAR_SETUP_RVA,
        TOPBAR_RUNTIME_ROOT,
    );
}

unsafe extern "system" fn hook_minimap(controller: usize) {
    call_original_and_capture(
        controller,
        &ORIGINAL_MINIMAP,
        HudSurface::Minimap,
        MINIMAP_SETUP_RVA,
        MINIMAP_RUNTIME_ROOT,
    );
}

unsafe extern "system" fn hook_mapmode(controller: usize) {
    call_original_and_capture(
        controller,
        &ORIGINAL_MAPMODE,
        HudSurface::Mapmodes,
        MAPMODE_SETUP_RVA,
        MAPMODE_RUNTIME_ROOT,
    );
}

unsafe extern "system" fn hook_theatre(controller: usize) {
    call_original_and_capture(
        controller,
        &ORIGINAL_THEATRE,
        HudSurface::TheatreSelector,
        THEATRE_SETUP_RVA,
        THEATRE_RUNTIME_ROOT,
    );
}

fn call_original_and_capture(
    controller: usize,
    original_slot: &AtomicUsize,
    surface: HudSurface,
    setup_rva: u32,
    root_offset: usize,
) {
    let callback_bit = match surface {
        HudSurface::Topbar => 1,
        HudSurface::Minimap => 1 << 1,
        HudSurface::Mapmodes => 1 << 2,
        HudSurface::TheatreSelector => 1 << 3,
        // Designer roots are observed by A27's existing post-refresh hook and
        // never enter this four-detour helper.
        HudSurface::DesignerCountryEquipment
        | HudSurface::DesignerTank
        | HudSurface::DesignerPlane => 1 << 7,
    };
    let prior_callbacks = ACTIVE_CALLBACKS.with(|guard| {
        let prior = guard.get();
        guard.set(prior | callback_bit);
        prior
    });
    let nested = prior_callbacks & callback_bit != 0;
    let original = original_slot.load(Ordering::Acquire);
    if original != 0 {
        let original: SetupFn = unsafe { mem::transmute(original) };
        unsafe { original(controller) };
    }
    if !nested {
        let result = capture_root(controller, surface, setup_rva, root_offset);
        if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
            state.callback_count = state.callback_count.saturating_add(1);
            match result {
                Ok(root) => {
                    state.latest.insert(surface, root);
                }
                Err(error) => {
                    state.rejected_observation_count =
                        state.rejected_observation_count.saturating_add(1);
                    state.current_error = Some(error);
                }
            }
        }
    }
    ACTIVE_CALLBACKS.with(|guard| guard.set(prior_callbacks));
}

fn capture_root(
    controller: usize,
    surface: HudSurface,
    setup_rva: u32,
    root_offset: usize,
) -> Result<RawHudControllerRoot, String> {
    if controller == 0 {
        return Err(format!(
            "{} setup supplied null controller",
            surface.token()
        ));
    }
    let runtime_root_ptr = read_usize(controller.saturating_add(root_offset))
        .filter(|pointer| *pointer != 0)
        .ok_or_else(|| format!("{} runtime root is null or unreadable", surface.token()))?;
    let runtime_root_name = runtime_root_name(runtime_root_ptr)?;
    Ok(RawHudControllerRoot {
        sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        setup_rva,
        surface,
        controller_ptr: controller,
        runtime_root_ptr,
        runtime_root_name,
    })
}

/// Calls the a729 GUI-definition root-name virtual proven at `0x1422A54E0`.
/// The returned MSVC string is copied immediately; no pointer leaves the
/// collector process.
pub fn runtime_root_name(runtime_root_ptr: usize) -> Result<String, String> {
    if runtime_root_ptr == 0 {
        return Err("HUD runtime root is null".into());
    }
    let definition = read_usize(runtime_root_ptr.saturating_add(RUNTIME_DEFINITION))
        .filter(|pointer| *pointer != 0)
        .ok_or("HUD runtime GUI definition is null or unreadable")?;
    let vtable = read_usize(definition).ok_or("HUD runtime GUI definition vtable unreadable")?;
    let getter = read_usize(vtable.saturating_add(DEFINITION_ROOT_NAME_VSLOT))
        .ok_or("HUD runtime GUI root-name virtual unreadable")?;
    let mut base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    }
    if base == 0 || getter < base || getter >= base.saturating_add(MAX_MODULE_IMAGE_SPAN) {
        return Err("HUD runtime GUI root-name virtual is outside pinned module".into());
    }
    let getter: DefinitionRootNameFn = unsafe { mem::transmute(getter) };
    let string_object = unsafe { getter(definition) };
    if string_object == 0 {
        return Err("HUD runtime GUI root-name virtual returned null".into());
    }
    let value = read_msvc_string(string_object)?;
    crate::validate_runtime_root_name(&value)?;
    Ok(value)
}

fn read_msvc_string(address: usize) -> Result<String, String> {
    let mut object = [0u8; 0x20];
    if !read_exact(address, &mut object) {
        return Err("HUD runtime GUI root-name string object unreadable".into());
    }
    let len = usize::from_le_bytes(object[0x10..0x18].try_into().unwrap());
    let capacity = usize::from_le_bytes(object[0x18..0x20].try_into().unwrap());
    if len == 0
        || len > crate::MAX_RUNTIME_ROOT_NAME_BYTES
        || capacity < len
        || capacity > MAX_MSVC_STRING_CAPACITY
    {
        return Err("HUD runtime GUI root-name string bounds are invalid".into());
    }
    let mut bytes = vec![0u8; len.saturating_add(1)];
    if capacity <= 0x0f {
        if len >= 16 || object[len] != 0 {
            return Err("HUD inline runtime GUI root name is not terminated".into());
        }
        bytes[..len].copy_from_slice(&object[..len]);
    } else {
        let data = usize::from_le_bytes(object[..mem::size_of::<usize>()].try_into().unwrap());
        if data == 0 || !read_exact(data, &mut bytes) || bytes[len] != 0 {
            return Err("HUD heap runtime GUI root name is unreadable or unterminated".into());
        }
    }
    bytes.truncate(len);
    String::from_utf8(bytes).map_err(|_| "HUD runtime GUI root name is not UTF-8".into())
}

fn addresses(base: usize) -> InstalledAddresses {
    InstalledAddresses {
        topbar_setup: base + TOPBAR_SETUP_RVA as usize,
        minimap_setup: base + MINIMAP_SETUP_RVA as usize,
        mapmode_setup: base + MAPMODE_SETUP_RVA as usize,
        theatre_setup: base + THEATRE_SETUP_RVA as usize,
    }
}

fn targets(addresses: InstalledAddresses) -> [usize; 4] {
    [
        addresses.topbar_setup,
        addresses.minimap_setup,
        addresses.mapmode_setup,
        addresses.theatre_setup,
    ]
}

fn clear_originals() {
    for original in [
        &ORIGINAL_TOPBAR,
        &ORIGINAL_MINIMAP,
        &ORIGINAL_MAPMODE,
        &ORIGINAL_THEATRE,
    ] {
        original.store(0, Ordering::Release);
    }
}

fn verify_sentinel(address: usize, expected: &[u8; 16], label: &str) -> Result<(), String> {
    let mut actual = [0u8; 16];
    if !read_exact(address, &mut actual) {
        return Err(format!("HUD {label} sentinel unreadable"));
    }
    if &actual != expected {
        return Err(format!(
            "HUD {label} sentinel mismatch: expected {expected:02X?}, got {actual:02X?}"
        ));
    }
    Ok(())
}

fn read_exact(address: usize, out: &mut [u8]) -> bool {
    if address == 0 || out.is_empty() {
        return false;
    }
    let mut read = 0usize;
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            out.len(),
            &mut read,
        ) != 0
            && read == out.len()
    }
}

fn read_usize(address: usize) -> Option<usize> {
    let mut bytes = [0u8; mem::size_of::<usize>()];
    read_exact(address, &mut bytes).then(|| usize::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_a729_addresses_offsets_and_sentinels_are_pinned() {
        let base = 0x1400_0000_0usize;
        let resolved = addresses(base);
        assert_eq!(resolved.topbar_setup, base + 0x0188_BA30);
        assert_eq!(resolved.minimap_setup, base + 0x017B_9A80);
        assert_eq!(resolved.mapmode_setup, base + 0x0156_3C80);
        assert_eq!(resolved.theatre_setup, base + 0x0187_8E40);
        assert_eq!(TOPBAR_RUNTIME_ROOT, 0x78);
        assert_eq!(MINIMAP_RUNTIME_ROOT, 0x1490);
        assert_eq!(MAPMODE_RUNTIME_ROOT, 0x2ed8);
        assert_eq!(THEATRE_RUNTIME_ROOT, 0x58);
        assert_eq!(RUNTIME_DEFINITION, 0x15d0);
        assert_eq!(DEFINITION_ROOT_NAME_VSLOT, 0x48);
        assert_eq!(TOPBAR_SENTINEL.len(), 16);
        assert_eq!(MINIMAP_SENTINEL.len(), 16);
        assert_eq!(MAPMODE_SENTINEL.len(), 16);
        assert_eq!(THEATRE_SENTINEL.len(), 16);
    }
}
