//! Exact-build terminal land-combat writer capture for hoi4.exe 1.19.2.0.a729.
//!
//! The two writer detours establish the owning active-combat pointer. Their
//! only shared record allocator is detoured as a nested observer, after the
//! engine has copied the complete 0x448-byte record into its global table.
//! Raw bytes never cross the replay wire: the collector projects only proven
//! pointer-free scalar fields from this process-local carrier.

#![cfg(windows)]

use core::cell::Cell;
use core::ffi::c_void;
use core::mem;
use core::ptr;
use minhook::{MinHook, MH_STATUS};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const NORMAL_WRITER_RVA: usize = 0x0CCC_7F0;
pub const OVERRUN_WRITER_RVA: usize = 0x0CCC_100;
pub const RECORD_ALLOCATOR_RVA: usize = 0x0CC9_F10;
pub const TERMINAL_RECORD_VTABLE_RVA: usize = 0x0294_6FB8;
pub const GAME_DATE_VTABLE_RVA: usize = 0x0270_2388;
pub const TERMINAL_RECORD_LEN: usize = 0x448;
pub const WRITER_KIND_OVERRUN: u32 = 1;
pub const WRITER_KIND_NORMAL: u32 = 2;

/// Production activation is admitted by the completed exact-10 replacement
/// receipt. Runtime installation still fails closed on the three entry-byte
/// sentinels below and on every queue/projection gate.
pub const TERMINAL_WRITER_ACTIVATION: bool = true;
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const EXACT10_POOL_STATUS_RECEIPT_SHA256: &str =
    "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395";
pub const TERMINAL_FUNCTION_MANIFEST_SHA256: &str =
    "3e5c698b7e151edc704b32f90f663b29de85e52d06f0563229885f8849637726";
pub const TERMINAL_SENTINEL_DUMP_SHA256: &str =
    "176a2731bdf88194b92328f417e51b37a33180e93ac339df4aa3236b6f78217c";
pub const TERMINAL_LAYOUT_MANIFEST_SHA256: &str =
    "6a25f5c1ac32637271d58bbb6de6dcf7bfbcf43896de605426a41a78153df2c3";
pub const TERMINAL_LOG_ENTRY_MANIFEST_SHA256: &str =
    "dd19cb97391dcaa77b8eb4c2f9980d4b86fa1e703342e5e9843b76995a9edb31";
pub const TERMINAL_WRITER_CALLER_MANIFEST_SHA256: &str =
    "727ad4ca363610b06d31afcc0a4082c359147a4de92423508eb09c49e85be3cd";
pub const TERMINAL_ALTERNATE_WRITER_MANIFEST_SHA256: &str =
    "492e712cd283a75b9b6357eb42f4bc98137d1ab138cfa857164f8d353f675c28";

const MAX_PENDING_RECORDS: usize = 8_192;
const LAND_COMBAT_TYPE: u32 = 62;
const NORMAL_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55,
];
const OVERRUN_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];
const ALLOCATOR_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x57, 0x41, 0x55, 0x48, 0x83, 0xec, 0x40, 0x4c, 0x63, 0x05, 0xec, 0x9c, 0x65, 0x02,
];

type NormalWriterFn = unsafe extern "system" fn(usize, usize, usize, usize, usize);
type OverrunWriterFn = unsafe extern "system" fn(usize, usize, usize);
type RecordAllocatorFn = unsafe extern "system" fn(usize) -> u64;

static ORIGINAL_NORMAL: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_OVERRUN: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ALLOCATOR: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static ACTIVE_WRITER_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static WRITER_CONTEXT: Cell<Option<WriterContext>> = const { Cell::new(None) };
}

#[derive(Clone, Copy)]
struct WriterContext {
    combat_ptr: usize,
    writer_kind: u32,
}

struct ActiveWriterCallback;

impl ActiveWriterCallback {
    fn enter() -> Self {
        ACTIVE_WRITER_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ActiveWriterCallback {
    fn drop(&mut self) {
        ACTIVE_WRITER_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatBinding {
    pub combat_ptr: usize,
    pub combat_id: u32,
    pub generation: u64,
}

/// Process-local exact record carrier. `record_bytes` may contain engine
/// pointers and therefore MUST be projected before serialization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTerminalRecordCapture {
    pub combat_id: u32,
    pub generation: u64,
    pub record_index: u32,
    pub writer_kind: u32,
    pub record_bytes: [u8; TERMINAL_RECORD_LEN],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalDrain {
    pub records: Vec<RawTerminalRecordCapture>,
    pub dropped_lock_busy: u64,
    pub dropped_unbound: u64,
    pub dropped_unreadable: u64,
    pub dropped_overflow: u64,
}

struct HookState {
    bindings: BTreeMap<usize, (u32, u64)>,
    records: VecDeque<RawTerminalRecordCapture>,
}

impl Default for HookState {
    fn default() -> Self {
        Self {
            bindings: BTreeMap::new(),
            // Allocate off the game thread during hook installation so the
            // terminal callback never grows the carrier heap.
            records: VecDeque::with_capacity(MAX_PENDING_RECORDS),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub normal_writer: usize,
    pub overrun_writer: usize,
    pub record_allocator: usize,
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        normal_writer: base.saturating_add(NORMAL_WRITER_RVA),
        overrun_writer: base.saturating_add(OVERRUN_WRITER_RVA),
        record_allocator: base.saturating_add(RECORD_ALLOCATOR_RVA),
    }
}

pub fn install() -> Result<InstalledHookAddresses, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal writer install mutex poisoned".to_string())?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let sites = addresses(base);
    verify_sentinel(sites.normal_writer, &NORMAL_SENTINEL, "normal terminal writer")?;
    verify_sentinel(sites.overrun_writer, &OVERRUN_SENTINEL, "overrun terminal writer")?;
    verify_sentinel(sites.record_allocator, &ALLOCATOR_SENTINEL, "terminal record allocator")?;
    STATE.get_or_init(|| Mutex::new(HookState::default()));

    let normal = unsafe {
        MinHook::create_hook(sites.normal_writer as *mut c_void, hook_normal_writer as *mut c_void)
    }
    .map_err(|status| format!("normal terminal writer hook creation failed: {status}"))?;
    let overrun = match unsafe {
        MinHook::create_hook(sites.overrun_writer as *mut c_void, hook_overrun_writer as *mut c_void)
    } {
        Ok(value) => value,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(sites.normal_writer as *mut c_void) };
            return Err(format!("overrun terminal writer hook creation failed: {status}"));
        }
    };
    let allocator = match unsafe {
        MinHook::create_hook(sites.record_allocator as *mut c_void, hook_record_allocator as *mut c_void)
    } {
        Ok(value) => value,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(sites.overrun_writer as *mut c_void) };
            let _ = unsafe { MinHook::remove_hook(sites.normal_writer as *mut c_void) };
            return Err(format!("terminal record allocator hook creation failed: {status}"));
        }
    };
    ORIGINAL_NORMAL.store(normal as usize, Ordering::Release);
    ORIGINAL_OVERRUN.store(overrun as usize, Ordering::Release);
    ORIGINAL_ALLOCATOR.store(allocator as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    // Arm the passive nested observer before either writer can publish a
    // context. This prevents a writer from reaching an unhooked allocator in
    // the installation window.
    for target in [sites.record_allocator, sites.overrun_writer, sites.normal_writer] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            for rollback in [sites.record_allocator, sites.overrun_writer, sites.normal_writer] {
                let _ = unsafe { MinHook::disable_hook(rollback as *mut c_void) };
                let _ = unsafe { MinHook::remove_hook(rollback as *mut c_void) };
            }
            ORIGINAL_NORMAL.store(0, Ordering::Release);
            ORIGINAL_OVERRUN.store(0, Ordering::Release);
            ORIGINAL_ALLOCATOR.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("terminal hook enable failed at 0x{target:X}: {status}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Disable writers first, then the nested allocator observer. The collector
/// must drain the final queue before calling this function.
pub fn uninstall() -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal writer uninstall mutex poisoned".to_string())?;
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
    for target in [sites.normal_writer, sites.overrun_writer, sites.record_allocator] {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("terminal hook disable failed at 0x{target:X}: {status}")),
        }
    }
    for target in [sites.record_allocator, sites.overrun_writer, sites.normal_writer] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("terminal hook removal failed at 0x{target:X}: {status}")),
        }
    }
    INSTALLED.store(false, Ordering::Release);
    ORIGINAL_NORMAL.store(0, Ordering::Release);
    ORIGINAL_OVERRUN.store(0, Ordering::Release);
    ORIGINAL_ALLOCATOR.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    WRITER_CONTEXT.with(|slot| slot.set(None));
    if let Some(state) = STATE.get() {
        *state
            .lock()
            .map_err(|_| "terminal hook state mutex poisoned during uninstall".to_string())? =
            HookState::default();
    }
    Ok(())
}

/// Stop writer admission, wait for every admitted writer callback to leave its
/// nested allocator, drain the final carrier, then remove all detours. This is
/// the production teardown path; it prevents a terminal record from racing the
/// final drain or being erased by hook-state cleanup.
pub fn seal_drain_and_uninstall() -> Result<TerminalDrain, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal writer seal mutex poisoned".to_string())?;
    if !INSTALLED.load(Ordering::Acquire) {
        return drain();
    }
    let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
    for target in [sites.normal_writer, sites.overrun_writer] {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("terminal writer seal failed at 0x{target:X}: {status}")),
        }
    }
    let mut yields = 0u32;
    while ACTIVE_WRITER_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("terminal writer callbacks did not quiesce during teardown".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    match unsafe { MinHook::disable_hook(sites.record_allocator as *mut c_void) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
        Err(status) => {
            return Err(format!(
                "terminal allocator seal failed at 0x{:X}: {status}",
                sites.record_allocator
            ));
        }
    }
    let drained = drain()?;
    for target in [sites.record_allocator, sites.overrun_writer, sites.normal_writer] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("terminal hook removal failed at 0x{target:X}: {status}")),
        }
    }
    INSTALLED.store(false, Ordering::Release);
    ORIGINAL_NORMAL.store(0, Ordering::Release);
    ORIGINAL_OVERRUN.store(0, Ordering::Release);
    ORIGINAL_ALLOCATOR.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    WRITER_CONTEXT.with(|slot| slot.set(None));
    if let Some(state) = STATE.get() {
        let mut state = state
            .lock()
            .map_err(|_| "terminal hook state mutex poisoned during seal".to_string())?;
        state.bindings.clear();
        debug_assert!(state.records.is_empty());
    }
    Ok(drained)
}

/// Replace the process-pointer join only after the preceding queue has been
/// drained. This preserves bindings for combats that ended between frames.
pub fn publish_active_combat_bindings(bindings: &[CombatBinding]) -> Result<(), String> {
    let mut next = BTreeMap::new();
    for binding in bindings {
        if binding.combat_ptr == 0 || binding.combat_id == 0 || binding.generation == 0 {
            return Err("invalid terminal combat binding".into());
        }
        if next
            .insert(binding.combat_ptr, (binding.combat_id, binding.generation))
            .is_some()
        {
            return Err("duplicate terminal combat pointer binding".into());
        }
    }
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook binding mutex poisoned".to_string())?;
    state.bindings = next;
    Ok(())
}

pub fn drain() -> Result<TerminalDrain, String> {
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook drain mutex poisoned".to_string())?;
    Ok(TerminalDrain {
        records: state.records.drain(..).collect(),
        dropped_lock_busy: DROPPED_LOCK_BUSY.swap(0, Ordering::AcqRel),
        dropped_unbound: DROPPED_UNBOUND.swap(0, Ordering::AcqRel),
        dropped_unreadable: DROPPED_UNREADABLE.swap(0, Ordering::AcqRel),
        dropped_overflow: DROPPED_OVERFLOW.swap(0, Ordering::AcqRel),
    })
}

pub fn requeue(records: Vec<RawTerminalRecordCapture>) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook requeue mutex poisoned".to_string())?;
    for record in records {
        if state.records.len() == MAX_PENDING_RECORDS {
            state.records.pop_front();
            DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        }
        state.records.push_back(record);
    }
    Ok(())
}

pub fn project_pointer_free_record(
    capture: &RawTerminalRecordCapture,
) -> Result<crate::TerminalCombatRecordRow, String> {
    let bytes = &capture.record_bytes;
    let base = MODULE_BASE.load(Ordering::Acquire);
    let record_vtable = u64::from_le_bytes(
        bytes[0..8].try_into().expect("fixed terminal vtable slice"),
    );
    let end_date_vtable = u64::from_le_bytes(
        bytes[0x428..0x430]
            .try_into()
            .expect("fixed terminal end-date slice"),
    );
    let location_date_vtable = u64::from_le_bytes(
        bytes[0x438..0x440]
            .try_into()
            .expect("fixed terminal location-date slice"),
    );
    if base == 0
        || record_vtable != base.saturating_add(TERMINAL_RECORD_VTABLE_RVA) as u64
        || end_date_vtable != base.saturating_add(GAME_DATE_VTABLE_RVA) as u64
        || location_date_vtable != base.saturating_add(GAME_DATE_VTABLE_RVA) as u64
    {
        return Err(format!(
            "terminal record {} class/date vtable mismatch",
            capture.record_index
        ));
    }
    let row = crate::TerminalCombatRecordRow {
        combat_id: capture.combat_id,
        generation: capture.generation,
        record_index: capture.record_index,
        writer_kind: capture.writer_kind,
        attacker_manpower_loss_raw: i64::from_le_bytes(
            bytes[0x10..0x18].try_into().expect("fixed terminal slice"),
        ),
        defender_manpower_loss_raw: i64::from_le_bytes(
            bytes[0x220..0x228].try_into().expect("fixed terminal slice"),
        ),
        end_date_raw: u32::from_le_bytes(
            bytes[0x430..0x434].try_into().expect("fixed terminal slice"),
        ),
        location: u32::from_le_bytes(
            bytes[0x440..0x444].try_into().expect("fixed terminal slice"),
        ),
        winner_side_raw: bytes[0x444],
        winter_raw: bytes[0x445],
        result_selector_raw: bytes[0x446],
        layout_side_raw: bytes[0x447],
    };
    match (row.writer_kind, row.result_selector_raw) {
        (WRITER_KIND_OVERRUN, 0) | (WRITER_KIND_NORMAL, 1) => Ok(row),
        _ => Err(format!(
            "terminal record {} writer/result byte mismatch ({}/{})",
            row.record_index, row.writer_kind, row.result_selector_raw
        )),
    }
}

unsafe extern "system" fn hook_normal_writer(
    param_1: usize,
    combat_ptr: usize,
    param_3: usize,
    param_4: usize,
    param_5: usize,
) {
    let _active = ActiveWriterCallback::enter();
    let original = ORIGINAL_NORMAL.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    WRITER_CONTEXT.with(|slot| {
        let previous = slot.replace(Some(WriterContext {
            combat_ptr,
            writer_kind: WRITER_KIND_NORMAL,
        }));
        let function: NormalWriterFn = unsafe { mem::transmute(original) };
        unsafe { function(param_1, combat_ptr, param_3, param_4, param_5) };
        slot.set(previous);
    });
}

unsafe extern "system" fn hook_overrun_writer(param_1: usize, param_2: usize, combat_ptr: usize) {
    let _active = ActiveWriterCallback::enter();
    let original = ORIGINAL_OVERRUN.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    WRITER_CONTEXT.with(|slot| {
        let previous = slot.replace(Some(WriterContext {
            combat_ptr,
            writer_kind: WRITER_KIND_OVERRUN,
        }));
        let function: OverrunWriterFn = unsafe { mem::transmute(original) };
        unsafe { function(param_1, param_2, combat_ptr) };
        slot.set(previous);
    });
}

unsafe extern "system" fn hook_record_allocator(record_ptr: usize) -> u64 {
    let original = ORIGINAL_ALLOCATOR.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    let function: RecordAllocatorFn = unsafe { mem::transmute(original) };
    let record_index = unsafe { function(record_ptr) };
    WRITER_CONTEXT.with(|slot| {
        if let Some(context) = slot.get() {
            capture_record(context, record_index, record_ptr);
        }
    });
    record_index
}

fn capture_record(context: WriterContext, record_index: u64, record_ptr: usize) {
    let Some(identity_addr) = context.combat_ptr.checked_add(0x08) else {
        DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut identity = [0u8; 8];
    let mut identity_read = 0usize;
    let identity_ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            identity_addr as *const c_void,
            identity.as_mut_ptr().cast(),
            identity.len(),
            &mut identity_read,
        )
    } != 0;
    if !identity_ok
        || identity_read != identity.len()
        || u32::from_le_bytes(identity[0..4].try_into().expect("fixed combat type slice"))
            != LAND_COMBAT_TYPE
    {
        DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let live_combat_id =
        u32::from_le_bytes(identity[4..8].try_into().expect("fixed combat id slice"));
    let Ok(mut state) = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .try_lock()
    else {
        DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(&(combat_id, generation)) = state.bindings.get(&context.combat_ptr) else {
        DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if combat_id != live_combat_id {
        DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let Ok(record_index) = u32::try_from(record_index) else {
        DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut record_bytes = [0u8; TERMINAL_RECORD_LEN];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            record_ptr as *const c_void,
            record_bytes.as_mut_ptr().cast(),
            record_bytes.len(),
            &mut read,
        )
    } != 0;
    if !ok || read != record_bytes.len() {
        DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if state.records.len() == MAX_PENDING_RECORDS {
        state.records.pop_front();
        DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
    }
    state.records.push_back(RawTerminalRecordCapture {
        combat_id,
        generation,
        record_index,
        writer_kind: context.writer_kind,
        record_bytes,
    });
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
    } != 0;
    if !ok || read != expected.len() || actual != expected {
        return Err(format!("a729 {label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}
