//! Exact-a729 current-state proof for an observation-free D71 frame.
//!
//! The passive Air/MIO hooks intentionally do not drive the client. When
//! neither controller materializer runs during an hour, this probe follows the
//! authenticated `CInGameIdler` singleton to its live GUI registry and asks
//! for the same two authored windows used by the engine setup routines. A
//! clean empty D71 frame is admitted only when both runtime windows exist and
//! both carry the engine's inactive bit. An active or unreadable window stays
//! unavailable; no foreground input, coordinates, panel activation, or GUI
//! construction is performed.

use core::ffi::c_void;
use std::mem;
use std::ptr;
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const HEADLESS_OWNER_ROOT_JOB_ID: &str =
    "20260724T143723Z-wave3-d71-game-gui-owner-root-13eb1c4d";
pub const HEADLESS_OWNER_ROOT_MANIFEST_SHA256: &str =
    "7e140b1c441a0b68326141dd66cb0169bd44c2ca4d4df927b245cda32a58e07b";
pub const HEADLESS_ACTIVE_STATE_JOB_ID: &str =
    "20260724T142836Z-d71-headless-controller-activation-audit-dd7e493a";
pub const HEADLESS_ACTIVE_STATE_MANIFEST_SHA256: &str =
    "cf969af82304a7ce1529bc6ea79d389c01b258b3690baeb5a4cdd15f6da867a2";
pub const GUI_REGISTRY_READ_JOB_ID: &str =
    "20260724T144125Z-wave3-d71-gui-registry-read-root-2884fe21";
pub const GUI_REGISTRY_READ_MANIFEST_SHA256: &str =
    "a44af54c79b851490800240e5d99eb3c97ed80b7d79b77ccd0c886f51614a359";
pub const MIO_WINDOW_NAME_JOB_ID: &str =
    "20260724T144051Z-wave3-d71-authored-mio-window-name-f37f7020";
pub const MIO_WINDOW_NAME_ARTIFACT_SHA256: &str =
    "0f0ad316fcd98fd5c338d4253f770b217bff0c3ee7971b845549f31e4319328d";

const CLOCK_SINGLETON_A_RVA: usize = 0x0331_72E0;
const CLOCK_SINGLETON_B_RVA: usize = 0x0331_72E8;
const CLOCK_VTABLE_RVA: usize = 0x0295_2510;
const CLOCK_GUI_REGISTRY_OFFSET: usize = 0x04F8;
const GUI_REGISTRY_WINDOW_LOOKUP_SLOT: usize = 0x60;
const WINDOW_ACTIVE_FLAGS_OFFSET: usize = 0xA5;
const WINDOW_ACTIVE_BIT: u8 = 0x08;

const AIR_SELECTION_WINDOW_NAME: &[u8] = b"air_selection_view\0";
const MIO_LIST_WINDOW_NAME: &[u8] = b"industrial_organisation_list_window\0";

#[repr(C)]
struct EngineStringView {
    data: usize,
    inline_tail: usize,
    len: usize,
    capacity: usize,
}

type WindowLookupFn =
    unsafe extern "system" fn(usize, *const EngineStringView, u8) -> usize;

/// Opaque same-hour proof. Only this module can construct it, so the sealed
/// Air/MIO source cannot be opened by a caller-supplied boolean.
#[derive(Clone, Copy, Debug)]
pub struct A729InactiveControllerProof {
    frame_hour: u64,
}

impl A729InactiveControllerProof {
    pub(crate) fn matches_hour(&self, frame_hour: u64) -> bool {
        self.frame_hour == frame_hour
    }
}

/// Returns `Some` only when both tracked controller windows are present and
/// inactive in the engine-owned registry. `None` means at least one controller
/// is active, so absence of a materializer observation remains fail-closed.
pub fn prove_all_tracked_controllers_inactive(
    frame_hour: u64,
) -> Result<Option<A729InactiveControllerProof>, String> {
    let module = unsafe { GetModuleHandleW(ptr::null()) };
    let base = module as usize;
    if base == 0 {
        return Err("A28 headless proof has no main-module base".into());
    }

    let clock_a = read_usize(base + CLOCK_SINGLETON_A_RVA)
        .ok_or("A28 headless proof clock singleton A is unreadable")?;
    let clock_b = read_usize(base + CLOCK_SINGLETON_B_RVA)
        .ok_or("A28 headless proof clock singleton B is unreadable")?;
    if clock_a == 0 || clock_a != clock_b {
        return Err("A28 headless proof clock singletons are absent or disagree".into());
    }
    if read_usize(clock_a) != Some(base + CLOCK_VTABLE_RVA) {
        return Err("A28 headless proof clock vtable differs from exact a729".into());
    }

    let registry = read_usize(clock_a + CLOCK_GUI_REGISTRY_OFFSET)
        .ok_or("A28 headless proof GUI registry root is unreadable")?;
    if registry == 0 {
        return Err("A28 headless proof GUI registry root is null".into());
    }
    let registry_vtable =
        read_usize(registry).ok_or("A28 headless proof GUI registry vtable is unreadable")?;
    let lookup_address = read_usize(registry_vtable + GUI_REGISTRY_WINDOW_LOOKUP_SLOT)
        .ok_or("A28 headless proof GUI lookup slot is unreadable")?;
    if lookup_address < base || lookup_address >= base.saturating_add(0x0400_0000) {
        return Err("A28 headless proof GUI lookup target is outside the exact image".into());
    }
    let lookup: WindowLookupFn = unsafe { mem::transmute(lookup_address) };

    let air = lookup_window(
        lookup,
        registry,
        "air_selection_view",
        AIR_SELECTION_WINDOW_NAME,
    )?;
    let mio = lookup_window(
        lookup,
        registry,
        "industrial_organisation_list_window",
        MIO_LIST_WINDOW_NAME,
    )?;
    let air_flags = read_u8(air + WINDOW_ACTIVE_FLAGS_OFFSET)
        .ok_or("A28 headless proof air window flags are unreadable")?;
    let mio_flags = read_u8(mio + WINDOW_ACTIVE_FLAGS_OFFSET)
        .ok_or("A28 headless proof MIO window flags are unreadable")?;
    if (air_flags | mio_flags) & WINDOW_ACTIVE_BIT != 0 {
        return Ok(None);
    }
    Ok(Some(A729InactiveControllerProof { frame_hour }))
}

fn lookup_window(
    lookup: WindowLookupFn,
    registry: usize,
    label: &str,
    name: &'static [u8],
) -> Result<usize, String> {
    let len = name
        .len()
        .checked_sub(1)
        .ok_or_else(|| "A28 headless proof has an empty authored window name".to_string())?;
    let view = EngineStringView {
        data: name.as_ptr() as usize,
        inline_tail: 0,
        len,
        capacity: len,
    };
    let window = unsafe { lookup(registry, &view, 0) };
    if window == 0 {
        return Err(format!(
            "A28 headless proof authored window lookup returned null for {label}"
        ));
    }
    Ok(window)
}

fn read_usize(address: usize) -> Option<usize> {
    let mut value = 0usize;
    read_exact(address, &mut value).then_some(value)
}

fn read_u8(address: usize) -> Option<u8> {
    let mut value = 0u8;
    read_exact(address, &mut value).then_some(value)
}

fn read_exact<T>(address: usize, out: &mut T) -> bool {
    if address < 0x1_0000 {
        return false;
    }
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out as *mut T as *mut c_void,
            mem::size_of::<T>(),
            &mut read,
        )
    };
    ok != 0 && read == mem::size_of::<T>()
}
