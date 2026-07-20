//! Dormant exact-a729 A33 same-thread writer hook.
//!
//! The implementation is complete but cannot install while either promotion
//! latch in the parent module is false. The outer CFEXMember writer establishes
//! the exact d49/d50 epoch/slot in TLS; the nested CSunkShipInfo constructor is
//! observed only after its original returns. Callback capture is bounded,
//! capacity-bounded, and any loss poisons completeness.

#![cfg(windows)]

use core::{cell::Cell, ffi::c_void, mem, ptr};
use minhook::{MinHook, MH_STATUS};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const OUTER_CFEX_MEMBER_WRITER_RVA: usize = 0x1961_DD0;
pub const RECORD_CONSTRUCTOR_RVA: usize = 0x0C1_DF70;
pub const SUNK_SHIP_INFO_VTABLE_RVA: usize = 0x294_1608;
pub const MAX_PENDING_EVENTS: usize = 65_536;
pub const MAX_CAPTURE_TEXT: usize = 256;

const OUTER_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24,
    0x10, 0x48,
];
const CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
    0x20, 0x57,
];

type OuterFn = unsafe extern "system" fn(usize, usize, usize, usize, u8);
type ConstructorFn = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize) -> usize;

static ORIGINAL_OUTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CONSTRUCTOR: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static DROPPED_REENTRANT: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static COMPLETENESS_POISONED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<VecDeque<RawSunkShipCapture>>> = OnceLock::new();
static HISTORY: OnceLock<Mutex<HistoryState>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static OUTER_CONTEXT: Cell<Option<OuterContext>> = const { Cell::new(None) };
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy)]
struct OuterContext {
    combat_id: u32,
    combat_generation: u64,
    battle_slot_index_raw: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSunkShipCapture {
    pub event_sequence: u64,
    pub combat_id: u32,
    pub combat_generation: u64,
    pub victim_member_battle_slot_index: u32,
    pub victim_country_identity_raw: [u32; 2],
    pub killer_country_id_raw: u32,
    pub sink_date_raw: i32,
    pub assisted_kill: bool,
    pub ship_name: String,
    pub ship_class_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDrain {
    pub events: Vec<RawSunkShipCapture>,
    pub dropped_unbound: u64,
    pub dropped_reentrant: u64,
    pub dropped_unreadable: u64,
    pub dropped_lock_busy: u64,
    pub dropped_overflow: u64,
    pub completeness_poisoned: bool,
}

#[derive(Default)]
struct HistoryState {
    baseline_complete: bool,
    events: Vec<crate::SunkShipEvent>,
}

struct CallbackGuard;
impl CallbackGuard {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for CallbackGuard {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    ProductionSchedulingDisabled,
    LiveFixtureNotAccepted,
    Runtime,
}

pub fn install() -> Result<(), InstallError> {
    if !crate::PRODUCTION_SCHEDULING_ENABLED {
        return Err(InstallError::ProductionSchedulingDisabled);
    }
    if !crate::LIVE_FIXTURE_ACCEPTED {
        return Err(InstallError::LiveFixtureNotAccepted);
    }
    install_promoted().map_err(|_| InstallError::Runtime)
}

fn install_promoted() -> Result<(), String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A33 install lock poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A33 GetModuleHandleW(NULL) returned null".into());
    }
    let outer = base + OUTER_CFEX_MEMBER_WRITER_RVA;
    let constructor = base + RECORD_CONSTRUCTOR_RVA;
    verify_sentinel(outer, &OUTER_SENTINEL)?;
    verify_sentinel(constructor, &CONSTRUCTOR_SENTINEL)?;
    STATE.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)));
    let original_outer = unsafe {
        MinHook::create_hook(outer as *mut c_void, hook_outer as *mut c_void)
    }
    .map_err(|status| format!("A33 outer create failed: {status}"))?;
    let original_constructor = match unsafe {
        MinHook::create_hook(constructor as *mut c_void, hook_constructor as *mut c_void)
    } {
        Ok(original) => original,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(outer as *mut c_void) };
            return Err(format!("A33 constructor create failed: {status}"));
        }
    };
    ORIGINAL_OUTER.store(original_outer as usize, Ordering::Release);
    ORIGINAL_CONSTRUCTOR.store(original_constructor as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    for target in [constructor, outer] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            let _ = unsafe { MinHook::disable_hook(constructor as *mut c_void) };
            let _ = unsafe { MinHook::disable_hook(outer as *mut c_void) };
            let _ = unsafe { MinHook::remove_hook(constructor as *mut c_void) };
            let _ = unsafe { MinHook::remove_hook(outer as *mut c_void) };
            return Err(format!("A33 enable failed: {status}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Publish the complete pre-hook history before any live event is admitted.
/// The live fixture must authenticate this baseline; an empty vector is valid
/// only when the fixture proves measured-empty history.
pub fn publish_complete_baseline(events: Vec<crate::SunkShipEvent>) -> Result<(), String> {
    if !crate::LIVE_FIXTURE_ACCEPTED {
        return Err("A33 live fixture has not authenticated a complete baseline".into());
    }
    let snapshot = crate::SunkShipSnapshot {
        source_sequence: events.last().map_or(1, |event| event.event_sequence),
        producer_complete: true,
        events,
    };
    crate::validate_snapshot(&snapshot).map_err(|error| format!("A33 baseline invalid: {error:?}"))?;
    *HISTORY
        .get_or_init(|| Mutex::new(HistoryState::default()))
        .lock()
        .map_err(|_| "A33 history lock poisoned")? = HistoryState {
        baseline_complete: true,
        events: snapshot.events,
    };
    COMPLETENESS_POISONED.store(false, Ordering::Release);
    Ok(())
}

/// Must run immediately before the collector replaces the d49/d50 member map.
/// Thus callbacks produced under the old map are drained before publication of
/// the next one. Any loss or missing baseline destroys the complete history.
pub fn drain_before_member_epoch_republish() -> Result<(), String> {
    if !crate::PRODUCTION_SCHEDULING_ENABLED {
        return Ok(());
    }
    let drained = drain()?;
    if drained.completeness_poisoned
        || drained.dropped_unbound != 0
        || drained.dropped_reentrant != 0
        || drained.dropped_unreadable != 0
        || drained.dropped_lock_busy != 0
        || drained.dropped_overflow != 0
    {
        return Err("A33 hook telemetry lost complete-history authority".into());
    }
    let mut history = HISTORY
        .get_or_init(|| Mutex::new(HistoryState::default()))
        .lock()
        .map_err(|_| "A33 history lock poisoned")?;
    if !history.baseline_complete {
        return Err("A33 complete-history baseline is absent".into());
    }
    for capture in drained.events {
        let event = crate::SunkShipEvent {
            event_sequence: capture.event_sequence,
            combat_id: capture.combat_id,
            combat_generation: capture.combat_generation,
            victim_country_identity_raw: capture.victim_country_identity_raw,
            killer_country_id_raw: capture.killer_country_id_raw,
            sink_date_raw: capture.sink_date_raw,
            victim_member_battle_slot_index: capture.victim_member_battle_slot_index,
            flags: if capture.assisted_kill {
                crate::EVENT_FLAG_ASSISTED_KILL
            } else {
                crate::EVENT_FLAG_KILLED
            },
            ship_name: capture.ship_name,
            ship_class_name: capture.ship_class_name,
        };
        if history
            .events
            .last()
            .is_some_and(|previous| event.event_sequence <= previous.event_sequence)
        {
            COMPLETENESS_POISONED.store(true, Ordering::Release);
            return Err("A33 event sequence is not strictly increasing".into());
        }
        history.events.push(event);
    }
    Ok(())
}

pub fn current_complete_snapshot() -> Result<crate::SunkShipSnapshot, String> {
    let history = HISTORY
        .get_or_init(|| Mutex::new(HistoryState::default()))
        .lock()
        .map_err(|_| "A33 history lock poisoned")?;
    if !history.baseline_complete || COMPLETENESS_POISONED.load(Ordering::Acquire) {
        return Err("A33 complete history is unavailable".into());
    }
    Ok(crate::SunkShipSnapshot {
        source_sequence: history.events.last().map_or(1, |event| event.event_sequence),
        producer_complete: true,
        events: history.events.clone(),
    })
}

pub fn drain() -> Result<HookDrain, String> {
    let mut queue = STATE
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)))
        .lock()
        .map_err(|_| "A33 queue lock poisoned")?;
    Ok(HookDrain {
        events: queue.drain(..).collect(),
        dropped_unbound: DROPPED_UNBOUND.swap(0, Ordering::AcqRel),
        dropped_reentrant: DROPPED_REENTRANT.swap(0, Ordering::AcqRel),
        dropped_unreadable: DROPPED_UNREADABLE.swap(0, Ordering::AcqRel),
        dropped_lock_busy: DROPPED_LOCK_BUSY.swap(0, Ordering::AcqRel),
        dropped_overflow: DROPPED_OVERFLOW.swap(0, Ordering::AcqRel),
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
    })
}

pub fn seal_drain_and_uninstall() -> Result<HookDrain, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A33 teardown lock poisoned")?;
    if !INSTALLED.load(Ordering::Acquire) {
        return drain();
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let outer = base + OUTER_CFEX_MEMBER_WRITER_RVA;
    let constructor = base + RECORD_CONSTRUCTOR_RVA;
    disable(outer)?;
    let mut yields = 0u32;
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("A33 callbacks did not quiesce".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    disable(constructor)?;
    let drained = drain()?;
    for target in [constructor, outer] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A33 remove failed: {status}")),
        }
    }
    INSTALLED.store(false, Ordering::Release);
    ORIGINAL_OUTER.store(0, Ordering::Release);
    ORIGINAL_CONSTRUCTOR.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    naval_combat_live_lane::clear_member_epoch_snapshot()?;
    Ok(drained)
}

fn disable(target: usize) -> Result<(), String> {
    match unsafe { MinHook::disable_hook(target as *mut c_void) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => Ok(()),
        Err(status) => Err(format!("A33 disable failed: {status}")),
    }
}

unsafe extern "system" fn hook_outer(a: usize, b: usize, c: usize, d: usize, e: u8) {
    let original: OuterFn = mem::transmute(ORIGINAL_OUTER.load(Ordering::Acquire));
    let _active = CallbackGuard::enter();
    let resolution = naval_combat_live_lane::try_current_member_epoch_snapshot()
        .and_then(|snapshot| snapshot.resolve_member(a).ok());
    if resolution.is_none() {
        poison(&DROPPED_UNBOUND);
    }
    let context = resolution.map(|resolved| OuterContext {
        combat_id: resolved.epoch.combat_id,
        combat_generation: resolved.epoch.generation,
        battle_slot_index_raw: resolved.battle_slot_index_raw,
    });
    OUTER_CONTEXT.with(|slot| {
        let prior = slot.replace(context);
        original(a, b, c, d, e);
        slot.set(prior);
    });
}

unsafe extern "system" fn hook_constructor(
    out: usize,
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
) -> usize {
    let original: ConstructorFn = mem::transmute(ORIGINAL_CONSTRUCTOR.load(Ordering::Acquire));
    let result = original(out, a, b, c, d, e);
    let Some(context) = OUTER_CONTEXT.with(Cell::get) else {
        return result;
    };
    let nested = IN_CALLBACK.with(|flag| flag.replace(true));
    if nested {
        poison(&DROPPED_REENTRANT);
        return result;
    }
    if let Some(capture) = capture_record(result, context) {
        if let Some(state) = STATE.get() {
            match state.try_lock() {
                Ok(mut queue) if queue.len() < MAX_PENDING_EVENTS => queue.push_back(capture),
                Ok(_) => poison(&DROPPED_OVERFLOW),
                Err(_) => poison(&DROPPED_LOCK_BUSY),
            }
        } else {
            poison(&DROPPED_UNREADABLE);
        }
    } else {
        poison(&DROPPED_UNREADABLE);
    }
    IN_CALLBACK.with(|flag| flag.set(false));
    result
}

fn capture_record(address: usize, context: OuterContext) -> Option<RawSunkShipCapture> {
    if address == 0 {
        return None;
    }
    let vtable: usize = read_value(address)?;
    if vtable != MODULE_BASE.load(Ordering::Acquire) + SUNK_SHIP_INFO_VTABLE_RVA {
        return None;
    }
    let ship_name = read_msvc_string(address + 0x08)?;
    let class_ptr: usize = read_value(address + 0x68)?;
    let ship_class_name = read_msvc_string(class_ptr.checked_add(0x68)?)?;
    let event_sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if event_sequence == 0 {
        return None;
    }
    Some(RawSunkShipCapture {
        event_sequence,
        combat_id: context.combat_id,
        combat_generation: context.combat_generation,
        victim_member_battle_slot_index: context.battle_slot_index_raw,
        victim_country_identity_raw: [read_value(address + 0x48)?, read_value(address + 0x4c)?],
        killer_country_id_raw: read_value(address + 0x78)?,
        sink_date_raw: read_value(address + 0x60)?,
        assisted_kill: read_value::<u8>(address + 0xa1)? != 0,
        ship_name,
        ship_class_name,
    })
}

fn read_msvc_string(address: usize) -> Option<String> {
    let len: usize = read_value(address + 0x10)?;
    let cap: usize = read_value(address + 0x18)?;
    if len == 0 || len >= MAX_CAPTURE_TEXT || cap < len {
        return None;
    }
    let data = if cap < 16 { address } else { read_value::<usize>(address)? };
    let mut bytes = [0u8; MAX_CAPTURE_TEXT];
    read_exact(data, &mut bytes[..len])?;
    if bytes[..len].contains(&0) {
        return None;
    }
    String::from_utf8(bytes[..len].to_vec()).ok()
}

fn read_value<T: Copy>(address: usize) -> Option<T> {
    let mut value = mem::MaybeUninit::<T>::uninit();
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), mem::size_of::<T>())
    };
    read_exact(address, bytes)?;
    Some(unsafe { value.assume_init() })
}

fn read_exact(address: usize, out: &mut [u8]) -> Option<()> {
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
    (ok != 0 && read == out.len()).then_some(())
}

fn verify_sentinel(address: usize, expected: &[u8]) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    read_exact(address, &mut actual).ok_or_else(|| "A33 sentinel unreadable".to_string())?;
    if actual != expected {
        return Err(format!("A33 sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn poison(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
}
