use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{SpottedUnitHandle, SpottedUnitsSnapshot, MAX_POV_COUNTRY_ID};

pub const INTEL_GETTER_RVA: u32 = 0x0160_68C0;
pub const LAND_FRAME_RVA: u32 = 0x0165_85E0;
pub const LAND_ELIGIBILITY_RVA: u32 = 0x00BE_2AB0;
pub const TASKFORCE_FRAME_RVA: u32 = 0x018D_DF50;
const GAMESTATE_PTR_RVA: usize = 0x0331_6E90;
const NAVY_DEBUG_OVERRIDE_RVA: usize = 0x0331_7252;

const GETTER_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x30,
    0x80, 0x3d, 0x9b, 0x09, 0xd1, 0x01, 0x00, 0x48,
];
const LAND_FRAME_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x48, 0x08, 0x53,
    0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
];
const LAND_ELIGIBILITY_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x83,
    0xb9, 0x88, 0x00, 0x00, 0x00, 0x00, 0x74, 0x3e,
];
const TASKFORCE_FRAME_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x44,
    0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];

type IntelGetterFn = unsafe extern "system" fn(usize) -> u32;
type LandFrameFn = unsafe extern "system" fn(usize);
type LandEligibilityFn = unsafe extern "system" fn(usize) -> u8;
type TaskforceFrameFn = unsafe extern "system" fn(usize, usize, usize);

static ORIGINAL_GETTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_LAND_FRAME: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_LAND_ELIGIBILITY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TASKFORCE_FRAME: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static LATEST: OnceLock<Mutex<LatestFrames>> = OnceLock::new();
static LIFECYCLE: Mutex<()> = Mutex::new(());

#[derive(Clone)]
struct CompleteFrame {
    sequence: u64,
    pov_country_id: u32,
    rows: Vec<SpottedUnitHandle>,
}

#[derive(Default)]
struct LatestFrames {
    land: Option<CompleteFrame>,
    taskforces: Option<CompleteFrame>,
}

struct BuildingFrame {
    pov_country_id: u32,
    rows: BTreeMap<(u32, u32), u8>,
    pending_land: Option<SpottedUnitHandle>,
}

thread_local! {
    static LAND_BUILD: RefCell<Option<BuildingFrame>> = const { RefCell::new(None) };
    static TASKFORCE_BUILD: RefCell<Option<BuildingFrame>> = const { RefCell::new(None) };
}

struct CallbackLease;
impl CallbackLease {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for CallbackLease {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn install() -> Result<(), String> {
    let _guard = LIFECYCLE.lock().map_err(|_| "spotted-unit hook lifecycle poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("spotted-unit hook module unavailable".into());
    }
    let specs: [(u32, &[u8], *mut c_void, &AtomicUsize); 4] = [
        (INTEL_GETTER_RVA, &GETTER_SENTINEL, hook_intel_getter as *mut c_void, &ORIGINAL_GETTER),
        (LAND_FRAME_RVA, &LAND_FRAME_SENTINEL, hook_land_frame as *mut c_void, &ORIGINAL_LAND_FRAME),
        (LAND_ELIGIBILITY_RVA, &LAND_ELIGIBILITY_SENTINEL, hook_land_eligibility as *mut c_void, &ORIGINAL_LAND_ELIGIBILITY),
        (TASKFORCE_FRAME_RVA, &TASKFORCE_FRAME_SENTINEL, hook_taskforce_frame as *mut c_void, &ORIGINAL_TASKFORCE_FRAME),
    ];
    let mut created = Vec::new();
    for (rva, sentinel, detour, original_slot) in specs {
        let target = match base.checked_add(rva as usize) {
            Some(target) => target,
            None => {
                rollback(&created);
                clear_originals();
                return Err("spotted-unit hook address overflow".into());
            }
        };
        let mut actual = vec![0u8; sentinel.len()];
        if let Err(error) = read_exact(target, &mut actual) {
            rollback(&created);
            clear_originals();
            return Err(error);
        }
        if actual != sentinel {
            rollback(&created);
            clear_originals();
            return Err(format!("spotted-unit hook sentinel mismatch at RVA {rva:#x}"));
        }
        let original = match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(value) => value,
            Err(status) => {
                rollback(&created);
                clear_originals();
                return Err(format!("spotted-unit hook creation failed at RVA {rva:#x}: {status}"));
            }
        };
        original_slot.store(original as usize, Ordering::Release);
        created.push(target);
    }
    MODULE_BASE.store(base, Ordering::Release);
    let mut latest = match LATEST
        .get_or_init(|| Mutex::new(LatestFrames::default()))
        .lock()
    {
        Ok(latest) => latest,
        Err(_) => {
            rollback(&created);
            clear_originals();
            MODULE_BASE.store(0, Ordering::Release);
            return Err("spotted-unit latest frames poisoned".into());
        }
    };
    *latest = LatestFrames::default();
    drop(latest);
    for target in &created {
        if let Err(status) = unsafe { MinHook::enable_hook(*target as *mut c_void) } {
            rollback(&created);
            clear_originals();
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("spotted-unit hook enable failed: {status}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn drain_latest() -> Result<Option<SpottedUnitsSnapshot>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("spotted-unit hooks are not installed".into());
    }
    let latest = LATEST.get().ok_or("spotted-unit hook state unavailable")?
        .lock().map_err(|_| "spotted-unit latest frames poisoned")?;
    snapshot_from_latest(&latest)
}

fn snapshot_from_latest(latest: &LatestFrames) -> Result<Option<SpottedUnitsSnapshot>, String> {
    // These are complete current map-visibility frames, not one-shot events.
    // A speed-5 game can advance several hourly collector boundaries between
    // renderer producer calls, so retain and reuse the last authenticated
    // frame until its producer replaces it. Consuming the slots here created
    // false source-unavailable/coverage holes on every intervening hour.
    let mut land = latest.land.clone();
    let mut taskforces = latest.taskforces.clone();
    if let (Some(a), Some(b)) = (&land, &taskforces) {
        if a.pov_country_id != b.pov_country_id {
            if a.sequence > b.sequence { taskforces = None; } else { land = None; }
        }
    }
    let pov_country_id = land.as_ref().map(|frame| frame.pov_country_id)
        .or_else(|| taskforces.as_ref().map(|frame| frame.pov_country_id));
    let Some(pov_country_id) = pov_country_id else { return Ok(None); };
    let capture_sequence = next_sequence().ok_or("spotted-unit capture sequence exhausted")?;
    let snapshot = SpottedUnitsSnapshot {
        capture_sequence,
        pov_country_id,
        land_frame_sequence: land.as_ref().map(|frame| frame.sequence),
        taskforce_frame_sequence: taskforces.as_ref().map(|frame| frame.sequence),
        land: land.map(|frame| frame.rows).unwrap_or_default(),
        taskforces: taskforces.map(|frame| frame.rows).unwrap_or_default(),
    };
    snapshot.validate()?;
    Ok(Some(snapshot))
}

pub fn uninstall() -> Result<(), String> {
    let _guard = LIFECYCLE.lock().map_err(|_| "spotted-unit hook lifecycle poisoned")?;
    if !INSTALLED.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let base = MODULE_BASE.swap(0, Ordering::AcqRel);
    let mut errors = Vec::new();
    for rva in [TASKFORCE_FRAME_RVA, LAND_ELIGIBILITY_RVA, LAND_FRAME_RVA, INTEL_GETTER_RVA] {
        let target = base.saturating_add(rva as usize) as *mut c_void;
        match unsafe { MinHook::disable_hook(target) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("spotted-unit hook disable failed: {status}")),
        }
    }
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
    for rva in [TASKFORCE_FRAME_RVA, LAND_ELIGIBILITY_RVA, LAND_FRAME_RVA, INTEL_GETTER_RVA] {
        let target = base.saturating_add(rva as usize) as *mut c_void;
        match unsafe { MinHook::remove_hook(target) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("spotted-unit hook removal failed: {status}")),
        }
    }
    clear_originals();
    LAND_BUILD.with(|slot| *slot.borrow_mut() = None);
    TASKFORCE_BUILD.with(|slot| *slot.borrow_mut() = None);
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

unsafe extern "system" fn hook_land_frame(owner: usize) {
    let _lease = CallbackLease::enter();
    let original: LandFrameFn = match original_fn(ORIGINAL_LAND_FRAME.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return,
    };
    let prior = LAND_BUILD.with(|slot| slot.borrow_mut().take());
    let frame = current_pov_country_id().map(new_building_frame);
    LAND_BUILD.with(|slot| *slot.borrow_mut() = frame);
    original(owner);
    let complete = LAND_BUILD.with(|slot| slot.borrow_mut().take());
    LAND_BUILD.with(|slot| *slot.borrow_mut() = prior);
    if let Some(frame) = complete { publish(frame, true); }
}

unsafe extern "system" fn hook_taskforce_frame(owner: usize, source: usize, context: usize) {
    let _lease = CallbackLease::enter();
    let original: TaskforceFrameFn = match original_fn(ORIGINAL_TASKFORCE_FRAME.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return,
    };
    let prior = TASKFORCE_BUILD.with(|slot| slot.borrow_mut().take());
    let debug_override = MODULE_BASE.load(Ordering::Acquire)
        .checked_add(NAVY_DEBUG_OVERRIDE_RVA)
        .and_then(|address| read_u8(address).ok())
        .unwrap_or(1);
    let frame = if debug_override == 0 { current_pov_country_id().map(new_building_frame) } else { None };
    TASKFORCE_BUILD.with(|slot| *slot.borrow_mut() = frame);
    original(owner, source, context);
    let complete = TASKFORCE_BUILD.with(|slot| slot.borrow_mut().take());
    TASKFORCE_BUILD.with(|slot| *slot.borrow_mut() = prior);
    if let Some(frame) = complete { publish(frame, false); }
}

unsafe extern "system" fn hook_intel_getter(unit: usize) -> u32 {
    let _lease = CallbackLease::enter();
    let original: IntelGetterFn = match original_fn(ORIGINAL_GETTER.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return 0,
    };
    let raw = original(unit) & 0xff;
    let handle = read_handle(unit, raw as u8);
    LAND_BUILD.with(|slot| {
        if let Some(frame) = slot.borrow_mut().as_mut() {
            frame.pending_land = handle.filter(|row| row.raw_intel >= 250);
        }
    });
    if let Some(row) = handle.filter(|row| row.raw_intel >= 150) {
        let taskforce_admitted = read_i32(unit.saturating_add(0x20c)).ok().is_some_and(|value| value < 1);
        if taskforce_admitted {
            TASKFORCE_BUILD.with(|slot| {
                if let Some(frame) = slot.borrow_mut().as_mut() { insert_row(frame, row); }
            });
        }
    }
    raw
}

unsafe extern "system" fn hook_land_eligibility(wrapper: usize) -> u8 {
    let _lease = CallbackLease::enter();
    let original: LandEligibilityFn = match original_fn(ORIGINAL_LAND_ELIGIBILITY.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return 0,
    };
    let admitted = original(wrapper);
    LAND_BUILD.with(|slot| {
        if let Some(frame) = slot.borrow_mut().as_mut() {
            let pending = frame.pending_land.take();
            if admitted != 0 {
                if let Some(row) = pending { insert_row(frame, row); }
            }
        }
    });
    admitted
}

fn new_building_frame(pov_country_id: u32) -> BuildingFrame {
    BuildingFrame { pov_country_id, rows: BTreeMap::new(), pending_land: None }
}

fn insert_row(frame: &mut BuildingFrame, row: SpottedUnitHandle) {
    frame.rows.entry((row.type_id, row.stable_id))
        .and_modify(|value| *value = (*value).max(row.raw_intel))
        .or_insert(row.raw_intel);
}

fn publish(frame: BuildingFrame, land: bool) {
    let Some(sequence) = next_sequence() else { return; };
    let rows = frame.rows.into_iter().map(|((type_id, stable_id), raw_intel)| {
        SpottedUnitHandle { type_id, stable_id, raw_intel }
    }).collect();
    let complete = CompleteFrame { sequence, pov_country_id: frame.pov_country_id, rows };
    if let Some(latest) = LATEST.get() {
        if let Ok(mut slot) = latest.lock() {
            if land { slot.land = Some(complete); } else { slot.taskforces = Some(complete); }
        }
    }
}

fn read_handle(unit: usize, raw_intel: u8) -> Option<SpottedUnitHandle> {
    let packed = read_u64(unit.checked_add(0x18)?).ok()?;
    let type_id = packed as u32;
    let stable_id = (packed >> 32) as u32;
    (type_id != 0 && stable_id != 0).then_some(SpottedUnitHandle { type_id, stable_id, raw_intel })
}

fn current_pov_country_id() -> Option<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let gamestate = read_usize(base.checked_add(GAMESTATE_PTR_RVA)?).ok()?;
    let primary = read_u32(gamestate.checked_add(0x520)?).ok()?;
    let id = if primary > 0 { primary } else { read_u32(gamestate.checked_add(0x524)?).ok()? };
    (id > 0 && id <= MAX_POV_COUNTRY_ID).then_some(id)
}

fn next_sequence() -> Option<u64> {
    let value = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    (value != 0).then_some(value)
}

fn rollback(created: &[usize]) {
    for target in created.iter().rev() {
        let _ = unsafe { MinHook::disable_hook(*target as *mut c_void) };
    }
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
    for target in created.iter().rev() {
        let _ = unsafe { MinHook::remove_hook(*target as *mut c_void) };
    }
}
fn clear_originals() {
    ORIGINAL_GETTER.store(0, Ordering::Release);
    ORIGINAL_LAND_FRAME.store(0, Ordering::Release);
    ORIGINAL_LAND_ELIGIBILITY.store(0, Ordering::Release);
    ORIGINAL_TASKFORCE_FRAME.store(0, Ordering::Release);
}
fn original_fn<T>(address: usize) -> Option<T> where T: Copy {
    (address != 0).then(|| unsafe { mem::transmute_copy(&address) })
}
fn read_u8(address: usize) -> Result<u8, String> { let mut b = [0; 1]; read_exact(address, &mut b)?; Ok(b[0]) }
fn read_u32(address: usize) -> Result<u32, String> { let mut b = [0; 4]; read_exact(address, &mut b)?; Ok(u32::from_le_bytes(b)) }
fn read_i32(address: usize) -> Result<i32, String> { let mut b = [0; 4]; read_exact(address, &mut b)?; Ok(i32::from_le_bytes(b)) }
fn read_u64(address: usize) -> Result<u64, String> { let mut b = [0; 8]; read_exact(address, &mut b)?; Ok(u64::from_le_bytes(b)) }
fn read_usize(address: usize) -> Result<usize, String> { let mut b = [0; mem::size_of::<usize>()]; read_exact(address, &mut b)?; Ok(usize::from_le_bytes(b)) }
fn read_exact(address: usize, out: &mut [u8]) -> Result<(), String> {
    if address == 0 { return Err("spotted-unit read from null".into()); }
    let mut read = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), address as *const c_void, out.as_mut_ptr().cast(), out.len(), &mut read) };
    if ok == 0 || read != out.len() { Err(format!("spotted-unit read failed at {address:#x}")) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_spotted_frames_are_reusable_across_speed_five_hourly_drains() {
        let latest = LatestFrames {
            land: Some(CompleteFrame {
                sequence: 11,
                pov_country_id: 3,
                rows: vec![SpottedUnitHandle {
                    type_id: 7,
                    stable_id: 9,
                    raw_intel: 250,
                }],
            }),
            taskforces: Some(CompleteFrame {
                sequence: 12,
                pov_country_id: 3,
                rows: vec![SpottedUnitHandle {
                    type_id: 8,
                    stable_id: 10,
                    raw_intel: 150,
                }],
            }),
        };

        let first = snapshot_from_latest(&latest).unwrap().unwrap();
        let second = snapshot_from_latest(&latest).unwrap().unwrap();
        assert_eq!(first.land_frame_sequence, Some(11));
        assert_eq!(first.taskforce_frame_sequence, Some(12));
        assert_eq!(second.land_frame_sequence, Some(11));
        assert_eq!(second.taskforce_frame_sequence, Some(12));
        assert_eq!(first.land, second.land);
        assert_eq!(first.taskforces, second.taskforces);
        assert_ne!(first.capture_sequence, second.capture_sequence);
    }
}
