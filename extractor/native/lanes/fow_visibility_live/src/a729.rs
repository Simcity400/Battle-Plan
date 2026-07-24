use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{PovVisibilitySnapshot, MAX_POV_COUNTRY_ID, MAX_PROVINCE_SLOTS};

pub const PRODUCER_RVA: u32 = 0x0160_6EA0;
const PRODUCER_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57,
    0x48, 0x83, 0xec, 0x60, 0x83, 0xba, 0x20, 0x05,
];
const CURRENT_BUFFER_OFFSET: usize = 0x00;
const PREVIOUS_BUFFER_OFFSET: usize = 0x08;
const PROVINCE_COUNT_OFFSET: usize = 0x10;
const POV_COUNTRY_ID_OFFSET: usize = 0x14;
const FLAGS_OFFSET: usize = 0x18;

type ProducerFn = unsafe extern "system" fn(usize, usize);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Header {
    current: usize,
    previous: usize,
    count: u32,
    pov_country_id: u32,
    flags: u32,
}

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static LATEST: OnceLock<Mutex<Option<PovVisibilitySnapshot>>> = OnceLock::new();
static LIFECYCLE: Mutex<()> = Mutex::new(());

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
    let _guard = LIFECYCLE.lock().map_err(|_| "visibility hook lifecycle poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("visibility hook module unavailable".into());
    }
    let target = base.checked_add(PRODUCER_RVA as usize)
        .ok_or("visibility producer address overflow")?;
    let mut actual = [0u8; PRODUCER_SENTINEL.len()];
    read_exact(target, &mut actual)?;
    if actual != PRODUCER_SENTINEL {
        return Err("visibility producer sentinel mismatch".into());
    }
    let original = unsafe {
        MinHook::create_hook(target as *mut c_void, hook_producer as *mut c_void)
    }.map_err(|status| format!("visibility producer hook creation failed: {status}"))?;
    ORIGINAL.store(original as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    let latest = LATEST.get_or_init(|| Mutex::new(None));
    let mut latest = match latest.lock() {
        Ok(latest) => latest,
        Err(_) => {
            let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
            ORIGINAL.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            return Err("visibility latest snapshot poisoned".into());
        }
    };
    *latest = None;
    drop(latest);
    if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
        let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
        ORIGINAL.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("visibility producer hook enable failed: {status}"));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn drain_latest() -> Result<Option<PovVisibilitySnapshot>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("visibility producer hook is not installed".into());
    }
    let latest = LATEST.get().ok_or("visibility hook state unavailable")?;
    // This hook captures the engine's current complete visibility buffer, not
    // a one-shot event. Keep the last authenticated current buffer available
    // across multiple hourly collector frames until the producer replaces it.
    // A torn producer observation still clears the slot in `hook_producer`.
    Ok(latest
        .lock()
        .map_err(|_| "visibility latest snapshot poisoned")?
        .clone())
}

pub fn uninstall() -> Result<(), String> {
    let _guard = LIFECYCLE.lock().map_err(|_| "visibility hook lifecycle poisoned")?;
    if !INSTALLED.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let base = MODULE_BASE.swap(0, Ordering::AcqRel);
    let target = base.checked_add(PRODUCER_RVA as usize)
        .ok_or("visibility producer address overflow")?;
    let mut errors = Vec::new();
    match unsafe { MinHook::disable_hook(target as *mut c_void) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
        Err(status) => errors.push(format!("visibility hook disable failed: {status}")),
    }
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
    match unsafe { MinHook::remove_hook(target as *mut c_void) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
        Err(status) => errors.push(format!("visibility hook removal failed: {status}")),
    }
    ORIGINAL.store(0, Ordering::Release);
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

unsafe extern "system" fn hook_producer(intel: usize, gamestate: usize) {
    let _lease = CallbackLease::enter();
    let original_addr = ORIGINAL.load(Ordering::Acquire);
    if original_addr == 0 {
        return;
    }
    let original: ProducerFn = mem::transmute(original_addr);
    original(intel, gamestate);
    let snapshot = capture_atomic(intel);
    if let Some(latest) = LATEST.get() {
        if let Ok(mut slot) = latest.lock() {
            // A torn or invalid current observation clears the slot. Never
            // let a previous render-frame vector masquerade as current.
            *slot = snapshot;
        }
    }
}

fn capture_atomic(intel: usize) -> Option<PovVisibilitySnapshot> {
    let before = read_header(intel)?;
    let count = usize::try_from(before.count).ok()?;
    if before.current == 0
        || before.previous == 0
        || count == 0
        || count > MAX_PROVINCE_SLOTS
        || before.pov_country_id == 0
        || before.pov_country_id > MAX_POV_COUNTRY_ID
    {
        return None;
    }
    let mut province_levels = vec![0u8; count];
    read_exact(before.current, &mut province_levels).ok()?;
    let after = read_header(intel)?;
    if before != after {
        return None;
    }
    let capture_sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if capture_sequence == 0 {
        return None;
    }
    Some(PovVisibilitySnapshot {
        capture_sequence,
        pov_country_id: before.pov_country_id,
        province_levels,
    })
}

fn read_header(base: usize) -> Option<Header> {
    Some(Header {
        current: read_usize(base.checked_add(CURRENT_BUFFER_OFFSET)?).ok()?,
        previous: read_usize(base.checked_add(PREVIOUS_BUFFER_OFFSET)?).ok()?,
        count: read_u32(base.checked_add(PROVINCE_COUNT_OFFSET)?).ok()?,
        pov_country_id: read_u32(base.checked_add(POV_COUNTRY_ID_OFFSET)?).ok()?,
        flags: read_u32(base.checked_add(FLAGS_OFFSET)?).ok()?,
    })
}

fn read_usize(address: usize) -> Result<usize, String> {
    let mut bytes = [0u8; mem::size_of::<usize>()];
    read_exact(address, &mut bytes)?;
    Ok(usize::from_le_bytes(bytes))
}
fn read_u32(address: usize) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_exact(address: usize, out: &mut [u8]) -> Result<(), String> {
    if address == 0 {
        return Err("visibility read from null".into());
    }
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr().cast(),
            out.len(),
            &mut read,
        )
    };
    if ok == 0 || read != out.len() {
        Err(format!("visibility read failed at {address:#x}"))
    } else {
        Ok(())
    }
}
