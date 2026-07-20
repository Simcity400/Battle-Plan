//! Exact-a729 ordered speed/pause transition capture for X12.
//!
//! All three post-mutation callbacks publish through one mutex-owned sequence
//! and queue. Queue loss and callback read failures are sticky terminal errors;
//! callers must fail closed rather than falling back to hourly anchors while
//! claiming sub-hour history.

use core::ffi::c_void;
use minhook::MinHook;
use std::cell::Cell;
use std::collections::VecDeque;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const SPEED_SETTER_RVA: usize = 0x001E_DAD0;
pub const PAUSE_TOGGLE_RVA: usize = 0x00DC_6520;
pub const PAUSE_FORCED_RVA: usize = 0x00DB_E260;
/// Remains false until the retained X12 lifecycle fixture closes the live gate.
pub const PRODUCTION_ACTIVATION: bool = false;
pub const EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_X12_SPEED_PAUSE_HOOK";

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const CLOCK_SINGLETON_A_RVA: usize = 0x0331_72E0;
const CLOCK_SINGLETON_B_RVA: usize = 0x0331_72E8;
const CLOCK_VTABLE_RVA: usize = 0x0295_2510;
const CURRENT_HOUR: usize = 0x468;
const TICK_PROGRESS: usize = 0x4B8;
const SPEED_INDEX: usize = 0x4BC;
const PAUSED: usize = 0x6C1;
const MAX_PENDING: usize = 4096;

const SPEED_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x48, 0x83, 0xEC, 0x40, 0x4C, 0x89, 0x74, 0x24, 0x30, 0x85, 0xD2, 0x79, 0x04, 0x33,
];
const PAUSE_TOGGLE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7C, 0x24, 0x20, 0x55,
];
const PAUSE_FORCED_SENTINEL: [u8; 16] = [
    0x83, 0xEA, 0x09, 0x74, 0x15, 0x83, 0xEA, 0x01, 0x74, 0x09, 0x83, 0xFA, 0x03, 0x75, 0x2A, 0x33,
];

type SpeedSetterFn = unsafe extern "system" fn(usize, i32);
type PauseToggleFn = unsafe extern "system" fn(usize, u8, usize);
type PauseForcedFn = unsafe extern "system" fn(usize, i32);

static ORIGINAL_SPEED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PAUSE_TOGGLE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PAUSE_FORCED: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    Speed,
    PauseToggle,
    PauseForced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeedPauseTransition {
    pub sequence: u64,
    pub kind: TransitionKind,
    pub game_hour_raw: i32,
    pub speed_raw: i32,
    pub paused: bool,
    pub tick_progress_f32_bits: u32,
    pub thread_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObservedState {
    game_hour_raw: i32,
    speed_raw: i32,
    paused: bool,
    tick_progress_f32_bits: u32,
}

#[derive(Debug, Default)]
struct HookState {
    next_sequence: u64,
    pending: VecDeque<SpeedPauseTransition>,
    callback_count: u64,
    owner_thread_id: Option<u32>,
    current_error: Option<String>,
}

impl HookState {
    fn fresh() -> Self {
        Self {
            pending: VecDeque::with_capacity(MAX_PENDING),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub speed_setter: usize,
    pub pause_toggle: usize,
    pub pause_forced: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub callback_count: u64,
    pub last_sequence: u64,
    pub pending: usize,
    pub current_error: Option<String>,
}

pub fn install() -> Result<InstalledAddresses, String> {
    let base = (unsafe { GetModuleHandleW(ptr::null()) }) as usize;
    if base == 0 {
        return Err("X12 GetModuleHandleW(NULL) returned null".into());
    }
    let addresses = InstalledAddresses {
        speed_setter: base + SPEED_SETTER_RVA,
        pause_toggle: base + PAUSE_TOGGLE_RVA,
        pause_forced: base + PAUSE_FORCED_RVA,
    };
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(addresses);
    }
    verify_sentinel(addresses.speed_setter, &SPEED_SENTINEL, "speed setter")?;
    verify_sentinel(addresses.pause_toggle, &PAUSE_TOGGLE_SENTINEL, "pause toggle")?;
    verify_sentinel(addresses.pause_forced, &PAUSE_FORCED_SENTINEL, "forced pause")?;
    let state = STATE.get_or_init(|| Mutex::new(HookState::fresh()));
    *state
        .lock()
        .map_err(|_| "X12 hook state mutex poisoned before install")? = HookState::fresh();

    let speed = unsafe {
        MinHook::create_hook(addresses.speed_setter as *mut c_void, hook_speed as *mut c_void)
    }
    .map_err(|status| format!("X12 speed hook creation failed: {status}"))?;
    ORIGINAL_SPEED.store(speed as usize, Ordering::Release);
    let pause_toggle = match unsafe {
        MinHook::create_hook(addresses.pause_toggle as *mut c_void, hook_pause_toggle as *mut c_void)
    } {
        Ok(original) => original,
        Err(status) => {
            rollback_created(&addresses);
            return Err(format!("X12 pause-toggle hook creation failed: {status}"));
        }
    };
    ORIGINAL_PAUSE_TOGGLE.store(pause_toggle as usize, Ordering::Release);
    let pause_forced = match unsafe {
        MinHook::create_hook(addresses.pause_forced as *mut c_void, hook_pause_forced as *mut c_void)
    } {
        Ok(original) => original,
        Err(status) => {
            rollback_created(&addresses);
            return Err(format!("X12 forced-pause hook creation failed: {status}"));
        }
    };
    ORIGINAL_PAUSE_FORCED.store(pause_forced as usize, Ordering::Release);

    for (target, name) in [
        (addresses.speed_setter, "speed"),
        (addresses.pause_toggle, "pause-toggle"),
        (addresses.pause_forced, "forced-pause"),
    ] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            rollback_enabled(&addresses);
            return Err(format!("X12 {name} hook enable failed: {status}"));
        }
    }
    MODULE_BASE.store(base, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(addresses)
}

pub fn uninstall() -> Result<(), String> {
    if !INSTALLED.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let base = MODULE_BASE.swap(0, Ordering::AcqRel);
    let addresses = InstalledAddresses {
        speed_setter: base + SPEED_SETTER_RVA,
        pause_toggle: base + PAUSE_TOGGLE_RVA,
        pause_forced: base + PAUSE_FORCED_RVA,
    };
    let mut errors = Vec::new();
    for (target, name) in [
        (addresses.speed_setter, "speed"),
        (addresses.pause_toggle, "pause-toggle"),
        (addresses.pause_forced, "forced-pause"),
    ] {
        if let Err(status) = unsafe { MinHook::disable_hook(target as *mut c_void) } {
            errors.push(format!("disable {name}: {status}"));
        }
        if let Err(status) = unsafe { MinHook::remove_hook(target as *mut c_void) } {
            errors.push(format!("remove {name}: {status}"));
        }
    }
    ORIGINAL_SPEED.store(0, Ordering::Release);
    ORIGINAL_PAUSE_TOGGLE.store(0, Ordering::Release);
    ORIGINAL_PAUSE_FORCED.store(0, Ordering::Release);
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn evidence_activation_enabled() -> bool {
    PRODUCTION_ACTIVATION
        || std::env::var(EVIDENCE_ACTIVATION_ENV)
            .ok()
            .as_deref()
            == Some("1")
}

pub fn drain() -> Result<Vec<SpeedPauseTransition>, String> {
    if !installed() {
        return Err("X12 speed/pause hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("X12 hook state is not initialized")?
        .lock()
        .map_err(|_| "X12 hook state mutex poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    Ok(state.pending.drain(..).collect())
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics::default();
    };
    HookDiagnostics {
        callback_count: state.callback_count,
        last_sequence: state.next_sequence,
        pending: state.pending.len(),
        current_error: state.current_error.clone(),
    }
}

unsafe extern "system" fn hook_speed(cgame_state: usize, requested: i32) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let before = (!nested).then(|| observe(cgame_state)).transpose();
    let original = ORIGINAL_SPEED.load(Ordering::Acquire);
    if original != 0 {
        let original: SpeedSetterFn = unsafe { mem::transmute(original) };
        unsafe { original(cgame_state, requested) };
    }
    if !nested {
        publish_change(TransitionKind::Speed, before, observe(cgame_state));
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_pause_toggle(idler: usize, mode: u8, text: usize) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let before = (!nested).then(|| observe_from_idler(idler)).transpose();
    let original = ORIGINAL_PAUSE_TOGGLE.load(Ordering::Acquire);
    if original != 0 {
        let original: PauseToggleFn = unsafe { mem::transmute(original) };
        unsafe { original(idler, mode, text) };
    }
    if !nested {
        publish_change(TransitionKind::PauseToggle, before, observe_from_idler(idler));
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_pause_forced(idler: usize, reason: i32) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let before = (!nested).then(|| observe_from_idler(idler)).transpose();
    let original = ORIGINAL_PAUSE_FORCED.load(Ordering::Acquire);
    if original != 0 {
        let original: PauseForcedFn = unsafe { mem::transmute(original) };
        unsafe { original(idler, reason) };
    }
    if !nested {
        publish_change(TransitionKind::PauseForced, before, observe_from_idler(idler));
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

fn publish_change(
    kind: TransitionKind,
    before: Result<Option<ObservedState>, String>,
    after: Result<ObservedState, String>,
) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else { return; };
    state.callback_count = state.callback_count.saturating_add(1);
    let before = match before {
        Ok(before) => before,
        Err(error) => { state.current_error = Some(error); return; }
    };
    let after = match after {
        Ok(after) => after,
        Err(error) => { state.current_error = Some(error); return; }
    };
    let Some(before) = before else {
        state.current_error = Some("X12 outer callback omitted its pre-mutation observation".into());
        return;
    };
    let thread_id = unsafe { GetCurrentThreadId() };
    match state.owner_thread_id {
        Some(owner) if owner != thread_id => {
            state.current_error = Some(format!(
                "X12 transition callbacks crossed threads ({owner} then {thread_id}); total ordering is unproved"
            ));
            return;
        }
        None => state.owner_thread_id = Some(thread_id),
        Some(_) => {}
    }
    if (kind == TransitionKind::Speed && before.paused != after.paused)
        || (kind != TransitionKind::Speed && before.speed_raw != after.speed_raw)
    {
        state.current_error = Some(
            "X12 callback changed both speed and pause; one-row attribution would lose ordering"
                .into(),
        );
        return;
    }
    let changed = match kind {
        TransitionKind::Speed => before.speed_raw != after.speed_raw,
        TransitionKind::PauseToggle | TransitionKind::PauseForced => before.paused != after.paused,
    };
    if !changed { return; }
    if state.pending.len() >= MAX_PENDING {
        state.current_error = Some("X12 transition queue overflow; ordering is incomplete".into());
        return;
    }
    let Some(sequence) = state.next_sequence.checked_add(1) else {
        state.current_error = Some("X12 transition sequence overflow".into());
        return;
    };
    state.next_sequence = sequence;
    state.pending.push_back(SpeedPauseTransition {
        sequence,
        kind,
        game_hour_raw: after.game_hour_raw,
        speed_raw: after.speed_raw,
        paused: after.paused,
        tick_progress_f32_bits: after.tick_progress_f32_bits,
        thread_id,
    });
}

fn observe_from_idler(idler: usize) -> Result<ObservedState, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let base = if base == 0 {
        (unsafe { GetModuleHandleW(ptr::null()) }) as usize
    } else {
        base
    };
    let clock_a = read_usize(base + CLOCK_SINGLETON_A_RVA).ok_or("X12 clock A unreadable")?;
    let clock_b = read_usize(base + CLOCK_SINGLETON_B_RVA).ok_or("X12 clock B unreadable")?;
    if idler == 0 || clock_a != idler || clock_b != idler {
        return Err("X12 pause callback owner differs from both clock singleton slots".into());
    }
    observe(read_usize(base + CGAMESTATE_SINGLETON_RVA).ok_or("X12 CGameState singleton unreadable")?)
}

fn observe(cgame_state: usize) -> Result<ObservedState, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let base = if base == 0 {
        (unsafe { GetModuleHandleW(ptr::null()) }) as usize
    } else {
        base
    };
    if cgame_state == 0 || read_usize(base + CGAMESTATE_SINGLETON_RVA) != Some(cgame_state) {
        return Err("X12 speed callback owner differs from CGameState singleton".into());
    }
    let clock_a = read_usize(base + CLOCK_SINGLETON_A_RVA).ok_or("X12 clock A unreadable")?;
    let clock_b = read_usize(base + CLOCK_SINGLETON_B_RVA).ok_or("X12 clock B unreadable")?;
    if clock_a == 0 || clock_a != clock_b || read_usize(clock_a) != Some(base + CLOCK_VTABLE_RVA) {
        return Err("X12 clock singleton/vtable identity mismatch".into());
    }
    let speed_raw = read_i32(cgame_state + SPEED_INDEX).ok_or("X12 speed unreadable")?;
    let paused_raw = read_u8(clock_a + PAUSED).ok_or("X12 pause unreadable")?;
    let tick_progress_f32_bits = read_u32(cgame_state + TICK_PROGRESS).ok_or("X12 tick progress unreadable")?;
    let tick = f32::from_bits(tick_progress_f32_bits);
    if !(0..=4).contains(&speed_raw) || paused_raw > 1 || !tick.is_finite() || !(0.0..=1.0).contains(&tick) || tick_progress_f32_bits == 0x8000_0000 {
        return Err("X12 post-mutation state failed range/finite validation".into());
    }
    Ok(ObservedState {
        game_hour_raw: read_i32(cgame_state + CURRENT_HOUR).ok_or("X12 game hour unreadable")?,
        speed_raw,
        paused: paused_raw != 0,
        tick_progress_f32_bits,
    })
}

fn verify_sentinel(address: usize, expected: &[u8; 16], name: &str) -> Result<(), String> {
    let mut actual = [0u8; 16];
    if !read_process(address, &mut actual) || &actual != expected {
        return Err(format!("X12 {name} sentinel mismatch"));
    }
    Ok(())
}

fn rollback_created(addresses: &InstalledAddresses) {
    for target in [addresses.speed_setter, addresses.pause_toggle, addresses.pause_forced] {
        let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
    }
    ORIGINAL_SPEED.store(0, Ordering::Release);
    ORIGINAL_PAUSE_TOGGLE.store(0, Ordering::Release);
    ORIGINAL_PAUSE_FORCED.store(0, Ordering::Release);
}

fn rollback_enabled(addresses: &InstalledAddresses) {
    for target in [addresses.speed_setter, addresses.pause_toggle, addresses.pause_forced] {
        let _ = unsafe { MinHook::disable_hook(target as *mut c_void) };
        let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
    }
    ORIGINAL_SPEED.store(0, Ordering::Release);
    ORIGINAL_PAUSE_TOGGLE.store(0, Ordering::Release);
    ORIGINAL_PAUSE_FORCED.store(0, Ordering::Release);
}

fn read_process(address: usize, out: &mut [u8]) -> bool {
    if address == 0 { return false; }
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
    ok != 0 && read == out.len()
}

fn read_u8(address: usize) -> Option<u8> { let mut b=[0;1]; read_process(address,&mut b).then_some(b[0]) }
fn read_u32(address: usize) -> Option<u32> { let mut b=[0;4]; read_process(address,&mut b).then_some(u32::from_le_bytes(b)) }
fn read_i32(address: usize) -> Option<i32> { read_u32(address).map(|v| v as i32) }
fn read_usize(address: usize) -> Option<usize> { let mut b=[0;mem::size_of::<usize>()]; read_process(address,&mut b).then_some(usize::from_le_bytes(b)) }
