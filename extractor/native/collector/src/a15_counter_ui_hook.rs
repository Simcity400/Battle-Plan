//! Exact-a729 land-counter training/dig-in presentation observer.
//!
//! The observer brackets `CUnitCounterItem`'s land badge update, records every
//! division accepted by the update's exact eligibility producer, and publishes
//! only the post-update GUI visibility/frame result for the same game hour.
//! D20 readers admit a row only when its division id and current raw hour match;
//! missing, stale, overflowing, or unreadable observations remain absent.

use core::ffi::c_void;
use minhook::MinHook;
use std::cell::RefCell;
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const COUNTER_LAND_UPDATE_RVA: usize = 0x018c_5ce0;
pub const MEMBER_ELIGIBILITY_RVA: usize = 0x0160_68c0;
pub const TRAINING_PREDICATE_RVA: usize = 0x00c7_9cc0;

const GAME_STATE_CURRENT_DATE_OFFSET: usize = 0x468;
const COUNTER_KIND_OFFSET: usize = 0x180;
const COUNTER_DIG_CONTROL_OFFSET: usize = 0xd8;
const COUNTER_TRAINING_CONTROL_OFFSET: usize = 0xf8;
/// Exact-a729 `CUnitCounterItem` member bound from the literal
const COUNTER_INVALID_ORDERS_CONTROL_OFFSET: usize = 0x88;
const CONTROL_STATE_OFFSET: usize = 0x75;
const CONTROL_VISIBLE_MASK: u8 = 8;
const WRAPPER_TO_BODY: usize = 0x10;
const BODY_TYPE_OFFSET: usize = 0x8;
const BODY_ID_OFFSET: usize = 0xc;
const BODY_DIG_IN_OFFSET: usize = 0x448;
const BODY_DIG_IN_CAP_OFFSET: usize = 0x450;
const DIVISION_TYPE: u32 = 0x1d;
const DIVISION_ALT_TYPE: u32 = 0x26;
const MAX_MEMBERS: usize = 1024;
const OBSERVATION_SLOTS: usize = 65_536;
const OBSERVATION_PROBES: usize = 16;

const COUNTER_UPDATE_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x88, 0x54, 0x24, 0x10, 0x55, 0x53,
];
const MEMBER_ELIGIBILITY_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x9b, 0x09, 0xd1, 0x01, 0x00, 0x0f,
];
const TRAINING_PREDICATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xb9, 0xc0, 0x00, 0x00,
];

type CounterUpdateFn = unsafe extern "system" fn(usize, u8, *const i32, *const i32, i32) -> u64;
type MemberEligibilityFn = unsafe extern "system" fn(usize) -> u8;
type TrainingPredicateFn = unsafe extern "system" fn(usize) -> u8;

static ORIGINAL_COUNTER_UPDATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MEMBER_ELIGIBILITY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TRAINING_PREDICATE: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static GAME_STATE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static BROKEN: AtomicBool = AtomicBool::new(false);
static DRAINING: AtomicBool = AtomicBool::new(false);
static INVALID_ORDERS_ACTIVATED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CounterObservation {
    pub training_visible: bool,
    /// `0` hidden, `1` visible/not-all-capped, `2` visible/all-capped.
    pub dig_in_frame: u8,
    /// Present only when the evidence-only producer was explicitly activated.
    pub invalid_orders_visible: Option<bool>,
}

struct ObservationSlot {
    /// `(raw_hour as u32) << 32 | division_id`; zero is never published.
    key: AtomicU64,
    bits: AtomicU64,
}

impl ObservationSlot {
    const fn new() -> Self {
        Self {
            key: AtomicU64::new(0),
            bits: AtomicU64::new(0),
        }
    }
}

static OBSERVATIONS: [ObservationSlot; OBSERVATION_SLOTS] =
    [const { ObservationSlot::new() }; OBSERVATION_SLOTS];

#[derive(Clone, Copy)]
struct Member {
    body: usize,
    eligibility: u8,
}

impl Member {
    const EMPTY: Self = Self {
        body: 0,
        eligibility: 0,
    };
}

struct OpenCounterUpdate {
    counter: usize,
    count: usize,
    overflowed: bool,
    training_evaluated: bool,
    training_predicate_true: bool,
    members: [Member; MAX_MEMBERS],
}

impl OpenCounterUpdate {
    fn new(counter: usize) -> Self {
        Self {
            counter,
            count: 0,
            overflowed: false,
            training_evaluated: false,
            training_predicate_true: false,
            members: [Member::EMPTY; MAX_MEMBERS],
        }
    }

    fn push_member(&mut self, body: usize, eligibility: u8) {
        if body == 0 || self.members[..self.count].iter().any(|row| row.body == body) {
            return;
        }
        if self.count == MAX_MEMBERS {
            self.overflowed = true;
            return;
        }
        self.members[self.count] = Member { body, eligibility };
        self.count += 1;
    }
}

thread_local! {
    static OPEN_UPDATE: RefCell<Option<OpenCounterUpdate>> = const { RefCell::new(None) };
}

struct CallbackGuard;

impl CallbackGuard {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }

    fn captures(&self) -> bool {
        !DRAINING.load(Ordering::Acquire)
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

unsafe extern "system" fn hook_counter_update(
    counter: usize,
    arg2: u8,
    arg3: *const i32,
    arg4: *const i32,
    arg5: i32,
) -> u64 {
    let guard = CallbackGuard::enter();
    let original: CounterUpdateFn = unsafe {
        mem::transmute(ORIGINAL_COUNTER_UPDATE.load(Ordering::Acquire))
    };
    let land_counter = guard.captures() && read_value::<u32>(counter + COUNTER_KIND_OFFSET) == Some(0);
    let opened = land_counter && OPEN_UPDATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            false
        } else {
            *slot = Some(OpenCounterUpdate::new(counter));
            true
        }
    });
    let result = unsafe { original(counter, arg2, arg3, arg4, arg5) };
    if opened {
        let update = OPEN_UPDATE.with(|slot| slot.borrow_mut().take());
        if let Some(update) = update {
            publish_update(update);
        }
    }
    result
}

unsafe extern "system" fn hook_member_eligibility(wrapper: usize) -> u8 {
    let guard = CallbackGuard::enter();
    let original: MemberEligibilityFn = unsafe {
        mem::transmute(ORIGINAL_MEMBER_ELIGIBILITY.load(Ordering::Acquire))
    };
    let result = unsafe { original(wrapper) };
    if guard.captures() {
        OPEN_UPDATE.with(|slot| {
            if let Some(update) = slot.borrow_mut().as_mut() {
                update.push_member(wrapper.saturating_add(WRAPPER_TO_BODY), result);
            }
        });
    }
    result
}

unsafe extern "system" fn hook_training_predicate(body: usize) -> u8 {
    let guard = CallbackGuard::enter();
    let original: TrainingPredicateFn = unsafe {
        mem::transmute(ORIGINAL_TRAINING_PREDICATE.load(Ordering::Acquire))
    };
    let result = unsafe { original(body) };
    if guard.captures() {
        OPEN_UPDATE.with(|slot| {
            if let Some(update) = slot.borrow_mut().as_mut() {
                update.training_evaluated = true;
                update.training_predicate_true |= result != 0;
            }
        });
    }
    result
}

fn publish_update(update: OpenCounterUpdate) {
    if update.overflowed || update.count == 0 || !update.training_evaluated {
        return;
    }
    let Some(raw_hour) = current_raw_hour() else {
        return;
    };
    let Some(training_visible) = control_visible(update.counter, COUNTER_TRAINING_CONTROL_OFFSET) else {
        return;
    };
    // This is a mechanical postcondition on the exact predicate call and the
    // post-update control state. A mismatch is safer as absence than as a
    // guessed replay badge.
    if training_visible != update.training_predicate_true {
        return;
    }
    let dig_control_visible = control_visible(update.counter, COUNTER_DIG_CONTROL_OFFSET);
    let mut any_positive_dig = false;
    let mut all_positive_capped = true;
    for member in &update.members[..update.count] {
        if member.eligibility < 250 {
            continue;
        }
        let Some(current) = read_value::<i64>(member.body + BODY_DIG_IN_OFFSET) else {
            return;
        };
        let Some(cap) = read_value::<i64>(member.body + BODY_DIG_IN_CAP_OFFSET) else {
            return;
        };
        if current > 0 {
            any_positive_dig = true;
            all_positive_capped &= current >= cap;
        }
    }
    let dig_in_frame = match dig_control_visible {
        Some(false) => 0,
        Some(true) if any_positive_dig => 1 + u8::from(all_positive_capped),
        _ => return,
    };
    let invalid_orders_visible = if INVALID_ORDERS_ACTIVATED.load(Ordering::Acquire) {
        // This is the postcondition of the engine's own land-counter update,
        // not a replay-side inference from orders or movement.
        match control_visible(update.counter, COUNTER_INVALID_ORDERS_CONTROL_OFFSET) {
            Some(visible) => Some(visible),
            None => return,
        }
    } else {
        None
    };
    for member in &update.members[..update.count] {
        if member.eligibility < 250 {
            continue;
        }
        let Some(unit_type) = read_value::<u32>(member.body + BODY_TYPE_OFFSET) else {
            continue;
        };
        if unit_type != DIVISION_TYPE && unit_type != DIVISION_ALT_TYPE {
            continue;
        }
        let Some(division_id) = read_value::<u32>(member.body + BODY_ID_OFFSET) else {
            continue;
        };
        publish_observation(
            division_id,
            raw_hour,
            CounterObservation {
                training_visible,
                dig_in_frame,
                invalid_orders_visible,
            },
        );
    }
}

fn current_raw_hour() -> Option<i32> {
    let game_state = GAME_STATE.load(Ordering::Acquire);
    read_value::<i32>(game_state.checked_add(GAME_STATE_CURRENT_DATE_OFFSET)?)
}

fn control_visible(counter: usize, control_offset: usize) -> Option<bool> {
    let control = read_value::<usize>(counter.checked_add(control_offset)?)?;
    if control == 0 {
        return None;
    }
    let state = read_value::<u8>(control.checked_add(CONTROL_STATE_OFFSET)?)?;
    Some(state & CONTROL_VISIBLE_MASK != 0)
}

fn observation_key(division_id: u32, raw_hour: i32) -> u64 {
    ((raw_hour as u32 as u64) << 32) | u64::from(division_id)
}

fn slot_start(division_id: u32) -> usize {
    (division_id.wrapping_mul(0x9e37_79b1) as usize) & (OBSERVATION_SLOTS - 1)
}

fn publish_observation(division_id: u32, raw_hour: i32, observation: CounterObservation) {
    let key = observation_key(division_id, raw_hour);
    if key == 0 {
        return;
    }
    let bits = u64::from(observation.training_visible)
        | (u64::from(observation.dig_in_frame) << 1)
        | observation
            .invalid_orders_visible
            .map_or(0, |visible| (1u64 << 3) | (u64::from(visible) << 4));
    let start = slot_start(division_id);
    for probe in 0..OBSERVATION_PROBES {
        let slot = &OBSERVATIONS[(start + probe) & (OBSERVATION_SLOTS - 1)];
        let old = slot.key.load(Ordering::Acquire);
        if old == 0 || old as u32 == division_id {
            slot.key.store(0, Ordering::Release);
            slot.bits.store(bits, Ordering::Relaxed);
            slot.key.store(key, Ordering::Release);
            return;
        }
    }
}

pub(crate) fn observation_for_current_hour(division_id: u32) -> Option<CounterObservation> {
    if !INSTALLED.load(Ordering::Acquire) || DRAINING.load(Ordering::Acquire) {
        return None;
    }
    let raw_hour = current_raw_hour()?;
    let expected = observation_key(division_id, raw_hour);
    let start = slot_start(division_id);
    for probe in 0..OBSERVATION_PROBES {
        let slot = &OBSERVATIONS[(start + probe) & (OBSERVATION_SLOTS - 1)];
        let key = slot.key.load(Ordering::Acquire);
        if key == expected {
            let bits = slot.bits.load(Ordering::Relaxed);
            let dig_in_frame = ((bits >> 1) & 0x3) as u8;
            if dig_in_frame <= 2 {
                return Some(CounterObservation {
                    training_visible: bits & 1 != 0,
                    dig_in_frame,
                    invalid_orders_visible: if bits & (1 << 3) != 0 {
                        Some(bits & (1 << 4) != 0)
                    } else {
                        None
                    },
                });
            }
            return None;
        }
    }
    None
}

pub(crate) fn install(module_base: usize, game_state: usize) -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "A15 counter hook install lock poisoned".to_string())?;
    if BROKEN.load(Ordering::Acquire) {
        return Err("A15 counter hook lifecycle is broken; process restart required".into());
    }
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    if module_base == 0 || game_state == 0 || read_value::<i32>(game_state + GAME_STATE_CURRENT_DATE_OFFSET).is_none() {
        return Err("A15 counter hook received an unreadable module/game-state root".into());
    }
    let specs = hook_specs(module_base);
    for (label, target, sentinel) in [
        ("land counter update", specs[0].0, COUNTER_UPDATE_SENTINEL.as_slice()),
        ("member eligibility", specs[1].0, MEMBER_ELIGIBILITY_SENTINEL.as_slice()),
        ("training predicate", specs[2].0, TRAINING_PREDICATE_SENTINEL.as_slice()),
    ] {
        verify_sentinel(label, target, sentinel)?;
    }
    for slot in &OBSERVATIONS {
        slot.key.store(0, Ordering::Relaxed);
        slot.bits.store(0, Ordering::Relaxed);
    }
    MODULE_BASE.store(module_base, Ordering::Release);
    GAME_STATE.store(game_state, Ordering::Release);
    INVALID_ORDERS_ACTIVATED.store(
        std::env::var_os("BATTLE_PLAN_CAP6_INVALID_ORDERS_UI").or_else(|| std::env::var_os("HOI4_CAP6_INVALID_ORDERS_UI")).as_deref()
            == Some(std::ffi::OsStr::new("1")),
        Ordering::Release,
    );
    DRAINING.store(false, Ordering::Release);
    let mut created = 0usize;
    for &(target, detour, original) in &specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(trampoline) => {
                original.store(trampoline as usize, Ordering::Release);
                created += 1;
            }
            Err(status) => {
                let rollback = rollback(&specs, created, 0);
                return Err(format!("A15 hook creation failed at 0x{target:X}: {status}{rollback}"));
            }
        }
    }
    for (enabled, (target, _, _)) in specs.iter().enumerate() {
        if let Err(status) = unsafe { MinHook::enable_hook(*target as *mut c_void) } {
            let rollback = rollback(&specs, created, enabled);
            return Err(format!("A15 hook enable failed at 0x{target:X}: {status}{rollback}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub(crate) fn uninstall() -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "A15 counter hook install lock poisoned".to_string())?;
    if BROKEN.load(Ordering::Acquire) {
        return Err("A15 counter hook lifecycle is broken; process restart required".into());
    }
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    DRAINING.store(true, Ordering::Release);
    let base = MODULE_BASE.load(Ordering::Acquire);
    let specs = hook_specs(base);
    let mut failures = Vec::new();
    for (target, _, _) in specs.iter().rev() {
        if let Err(status) = unsafe { MinHook::disable_hook(*target as *mut c_void) } {
            failures.push(format!("0x{target:X}: {status}"));
        }
    }
    if !failures.is_empty() {
        BROKEN.store(true, Ordering::Release);
        return Err(format!("A15 hook disable incomplete at {}; process restart required", failures.join(", ")));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            BROKEN.store(true, Ordering::Release);
            return Err("A15 hook callbacks did not quiesce; process restart required".into());
        }
        thread::yield_now();
    }
    for (target, _, original) in specs.iter().rev() {
        match unsafe { MinHook::remove_hook(*target as *mut c_void) } {
            Ok(()) => original.store(0, Ordering::Release),
            Err(status) => failures.push(format!("0x{target:X}: {status}")),
        }
    }
    if !failures.is_empty() {
        BROKEN.store(true, Ordering::Release);
        return Err(format!("A15 hook removal incomplete at {}; process restart required", failures.join(", ")));
    }
    OPEN_UPDATE.with(|slot| *slot.borrow_mut() = None);
    MODULE_BASE.store(0, Ordering::Release);
    GAME_STATE.store(0, Ordering::Release);
    INVALID_ORDERS_ACTIVATED.store(false, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    DRAINING.store(false, Ordering::Release);
    Ok(())
}

type HookSpec = (usize, *mut c_void, &'static AtomicUsize);

fn hook_specs(base: usize) -> [HookSpec; 3] {
    [
        (base + COUNTER_LAND_UPDATE_RVA, hook_counter_update as *mut c_void, &ORIGINAL_COUNTER_UPDATE),
        (base + MEMBER_ELIGIBILITY_RVA, hook_member_eligibility as *mut c_void, &ORIGINAL_MEMBER_ELIGIBILITY),
        (base + TRAINING_PREDICATE_RVA, hook_training_predicate as *mut c_void, &ORIGINAL_TRAINING_PREDICATE),
    ]
}

fn rollback(specs: &[HookSpec], created: usize, enabled: usize) -> String {
    DRAINING.store(true, Ordering::Release);
    let mut failures = Vec::new();
    for (target, _, _) in specs.iter().take(enabled).rev() {
        if let Err(status) = unsafe { MinHook::disable_hook(*target as *mut c_void) } {
            failures.push(format!("0x{target:X} disable: {status}"));
        }
    }
    for (target, _, original) in specs.iter().take(created).rev() {
        match unsafe { MinHook::remove_hook(*target as *mut c_void) } {
            Ok(()) => original.store(0, Ordering::Release),
            Err(status) => failures.push(format!("0x{target:X} remove: {status}")),
        }
    }
    MODULE_BASE.store(0, Ordering::Release);
    GAME_STATE.store(0, Ordering::Release);
    if failures.is_empty() {
        DRAINING.store(false, Ordering::Release);
        String::new()
    } else {
        BROKEN.store(true, Ordering::Release);
        format!("; rollback incomplete at {}", failures.join(", "))
    }
}

fn verify_sentinel(label: &str, address: usize, expected: &[u8]) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    if !read_bytes(address, &mut actual) || actual != expected {
        return Err(format!("A15 {label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn read_value<T: Copy>(address: usize) -> Option<T> {
    if address == 0 {
        return None;
    }
    let mut value = mem::MaybeUninit::<T>::uninit();
    let output = unsafe {
        std::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), mem::size_of::<T>())
    };
    if !read_bytes(address, output) {
        return None;
    }
    Some(unsafe { value.assume_init() })
}

fn read_bytes(address: usize, output: &mut [u8]) -> bool {
    if address == 0 || output.is_empty() {
        return false;
    }
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            output.as_mut_ptr().cast::<c_void>(),
            output.len(),
            &mut read,
        )
    };
    ok != 0 && read == output.len()
}
