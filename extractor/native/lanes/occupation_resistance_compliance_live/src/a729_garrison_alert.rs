//! Exact-a729 observation of the `alert_not_enough_garrison` producer.
//!
//! The hook does not reproduce the engine's stockpile or manpower laws. It
//! brackets the actual predicate/detail producer and observes the final
//! archetype-keyed shortage accumulator. Complete current-hour detail calls
//! publish active shortages; an exact predicate-only success publishes a
//! measured inactive result. Predicate failure and partial callbacks never
//! become fabricated empty alerts.

use minhook::{MinHook, MH_STATUS};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const ALERT_PRODUCER_RVA: usize = 0x00af_e8a0;
pub const ARCHETYPE_POOL_CTOR_RVA: usize = 0x00ff_7380;
pub const ARCHETYPE_POOL_ADD_RVA: usize = 0x00ff_7710;
pub const LOCALIZATION_BUILDER_RVA: usize = 0x0223_0f20;
pub const ARCHETYPE_STABLE_ID_OFF: usize = 0x418;
pub const FIXED_POINT_DIVISOR: i64 = 100_000;
const GAME_STATE_ROOT_RVA: usize = 0x0331_6e90;
const MAX_EQUIPMENT_ROWS: usize = 4_096;

const ALERT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
    0x57,
];
const POOL_CTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
    0x48,
];
const POOL_ADD_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x49, 0x8b, 0xd8, 0x41, 0xb0, 0x01, 0xe8, 0x8f, 0x39,
    0x00,
];
const LOCALIZATION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x41, 0x56, 0x41, 0x57,
    0x48,
];
const MANPOWER_KEY: &[u8] = b"alert_not_enough_garrison_delayed\0";

type AlertProducerFn = unsafe extern "system" fn(usize, usize) -> u8;
type PoolCtorFn = unsafe extern "system" fn(usize) -> usize;
type PoolAddFn = unsafe extern "system" fn(usize, usize, i64);
type LocalizationFn = unsafe extern "system" fn(usize, usize, usize, u32) -> usize;

static ORIGINAL_ALERT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_POOL_CTOR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_POOL_ADD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_LOCALIZATION: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static DROPPED_PUBLISH: AtomicBool = AtomicBool::new(false);
static PUBLISHED: OnceLock<Mutex<BTreeMap<i32, AlertObservation>>> = OnceLock::new();
static POISONED_HOURS: OnceLock<Mutex<BTreeSet<i32>>> = OnceLock::new();
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static ACTIVE: RefCell<Option<WorkingObservation>> = const { RefCell::new(None) };
    static NESTED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertEquipmentShortage {
    pub archetype_id: u32,
    pub whole_units: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertObservation {
    pub game_hour_raw: i32,
    pub country_ptr: u64,
    pub predicate_returned: bool,
    pub missing_manpower: i64,
    pub missing_equipment: Vec<AlertEquipmentShortage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertDrain {
    pub observations: Vec<AlertObservation>,
    pub poisoned_hours: Vec<i32>,
    pub publish_lock_dropped: bool,
}

struct WorkingObservation {
    game_hour_raw: Option<i32>,
    country_ptr: usize,
    detail_output: usize,
    constructor_count: u8,
    final_accumulator: usize,
    missing_manpower: Option<i64>,
    equipment_raw: BTreeMap<u32, (usize, i64)>,
    poisoned: bool,
}

struct CallbackGuard;
impl CallbackGuard {
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for CallbackGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn install() -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "a729 garrison-alert hook lifecycle lock poisoned".to_string())?;
    if installed() {
        return Ok(());
    }
    if MODULE_BASE.load(Ordering::Acquire) != 0 {
        return Err("a729 garrison-alert prior hook set has not completed removal".into());
    }
    PUBLISHED
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "a729 garrison-alert publication lock poisoned")?
        .clear();
    POISONED_HOURS
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .lock()
        .map_err(|_| "a729 garrison-alert poison lock poisoned")?
        .clear();
    DROPPED_PUBLISH.store(false, Ordering::Release);
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("a729 garrison-alert hook has no main-module base".into());
    }
    let targets = [
        (base + ALERT_PRODUCER_RVA, &ALERT_SENTINEL[..]),
        (base + ARCHETYPE_POOL_CTOR_RVA, &POOL_CTOR_SENTINEL[..]),
        (base + ARCHETYPE_POOL_ADD_RVA, &POOL_ADD_SENTINEL[..]),
        (base + LOCALIZATION_BUILDER_RVA, &LOCALIZATION_SENTINEL[..]),
    ];
    for (address, sentinel) in targets {
        let observed = read_exact(address, sentinel.len())
            .ok_or("a729 garrison-alert hook sentinel is unreadable")?;
        if observed != sentinel {
            return Err(format!(
                "a729 garrison-alert hook sentinel mismatch at {address:#x}"
            ));
        }
    }
    let specs = [
        (
            base + ALERT_PRODUCER_RVA,
            alert_detour as *mut c_void,
            &ORIGINAL_ALERT,
        ),
        (
            base + ARCHETYPE_POOL_CTOR_RVA,
            pool_ctor_detour as *mut c_void,
            &ORIGINAL_POOL_CTOR,
        ),
        (
            base + ARCHETYPE_POOL_ADD_RVA,
            pool_add_detour as *mut c_void,
            &ORIGINAL_POOL_ADD,
        ),
        (
            base + LOCALIZATION_BUILDER_RVA,
            localization_detour as *mut c_void,
            &ORIGINAL_LOCALIZATION,
        ),
    ];
    MODULE_BASE.store(base, Ordering::Release);
    let mut created = 0usize;
    for (target, detour, original) in specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(trampoline) => {
                original.store(trampoline as usize, Ordering::Release);
                created += 1;
            }
            Err(error) => {
                let mut cleanup_failures = Vec::new();
                for (rollback_target, _, _) in specs[..created].iter().rev() {
                    match unsafe { MinHook::remove_hook(*rollback_target as *mut c_void) } {
                        Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                        Err(status) => cleanup_failures.push(format!(
                            "MinHook remove {rollback_target:#x}: {status:?}"
                        )),
                    }
                }
                if cleanup_failures.is_empty() {
                    clear_hook_state();
                }
                let cleanup = (!cleanup_failures.is_empty())
                    .then(|| format!("; rollback failed: {}", cleanup_failures.join(", ")))
                    .unwrap_or_default();
                return Err(format!("MinHook create {target:#x}: {error:?}{cleanup}"));
            }
        }
    }
    let mut enabled = 0usize;
    for (target, _, _) in specs {
        if let Err(error) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            let mut cleanup_failures = Vec::new();
            for (rollback_target, _, _) in specs[..enabled].iter().rev() {
                match unsafe { MinHook::disable_hook(*rollback_target as *mut c_void) } {
                    Ok(())
                    | Err(MH_STATUS::MH_ERROR_DISABLED)
                    | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                    Err(status) => cleanup_failures.push(format!(
                        "MinHook disable {rollback_target:#x}: {status:?}"
                    )),
                }
            }
            if cleanup_failures.is_empty() {
                if let Err(status) = wait_for_quiescence() {
                    cleanup_failures.push(status);
                }
            }
            if cleanup_failures.is_empty() {
                for (rollback_target, _, _) in specs.iter().rev() {
                    match unsafe { MinHook::remove_hook(*rollback_target as *mut c_void) } {
                        Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                        Err(status) => cleanup_failures.push(format!(
                            "MinHook remove {rollback_target:#x}: {status:?}"
                        )),
                    }
                }
            }
            if cleanup_failures.is_empty() {
                clear_hook_state();
            }
            let cleanup = (!cleanup_failures.is_empty())
                .then(|| format!("; rollback failed: {}", cleanup_failures.join(", ")))
                .unwrap_or_default();
            return Err(format!("MinHook enable {target:#x}: {error:?}{cleanup}"));
        }
        enabled += 1;
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn take_for_hour(game_hour_raw: i32) -> Result<Option<AlertObservation>, String> {
    let mut poisoned = POISONED_HOURS
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .lock()
        .map_err(|_| "a729 garrison-alert poison lock poisoned")?;
    let was_poisoned = poisoned.remove(&game_hour_raw);
    poisoned.retain(|hour| *hour >= game_hour_raw.saturating_sub(1));
    let mut published = PUBLISHED
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "a729 garrison-alert publication lock poisoned")?;
    let result = published.remove(&game_hour_raw);
    published.retain(|hour, _| *hour >= game_hour_raw.saturating_sub(1));
    let dropped = DROPPED_PUBLISH.swap(false, Ordering::AcqRel);
    Ok((!was_poisoned && !dropped).then_some(result).flatten())
}

pub fn seal_drain_and_uninstall() -> Result<AlertDrain, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "a729 garrison-alert hook lifecycle lock poisoned".to_string())?;
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base != 0 {
        INSTALLED.store(false, Ordering::Release);
        let targets = [
            base + LOCALIZATION_BUILDER_RVA,
            base + ARCHETYPE_POOL_ADD_RVA,
            base + ARCHETYPE_POOL_CTOR_RVA,
            base + ALERT_PRODUCER_RVA,
        ];
        let mut failures = Vec::new();
        for target in targets {
            match unsafe { MinHook::disable_hook(target as *mut c_void) } {
                Ok(())
                | Err(MH_STATUS::MH_ERROR_DISABLED)
                | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                Err(error) => failures.push(format!("MinHook disable {target:#x}: {error:?}")),
            }
        }
        if !failures.is_empty() {
            return Err(failures.join(", "));
        }
        wait_for_quiescence()?;
        for target in targets {
            match unsafe { MinHook::remove_hook(target as *mut c_void) } {
                Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                Err(error) => failures.push(format!("MinHook remove {target:#x}: {error:?}")),
            }
        }
        if !failures.is_empty() {
            return Err(failures.join(", "));
        }
        clear_hook_state();
    }
    let mut published = PUBLISHED
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "a729 garrison-alert publication lock poisoned")?;
    let mut poisoned = POISONED_HOURS
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .lock()
        .map_err(|_| "a729 garrison-alert poison lock poisoned")?;
    Ok(AlertDrain {
        observations: mem::take(&mut *published).into_values().collect(),
        poisoned_hours: mem::take(&mut *poisoned).into_iter().collect(),
        publish_lock_dropped: DROPPED_PUBLISH.swap(false, Ordering::AcqRel),
    })
}

fn wait_for_quiescence() -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if std::time::Instant::now() >= deadline {
            return Err("a729 garrison-alert callbacks did not quiesce after disable".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

fn clear_hook_state() {
    ORIGINAL_ALERT.store(0, Ordering::Release);
    ORIGINAL_POOL_CTOR.store(0, Ordering::Release);
    ORIGINAL_POOL_ADD.store(0, Ordering::Release);
    ORIGINAL_LOCALIZATION.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
}

unsafe extern "system" fn alert_detour(country: usize, detail: usize) -> u8 {
    let _guard = CallbackGuard::enter();
    let original: AlertProducerFn = mem::transmute(ORIGINAL_ALERT.load(Ordering::Acquire));
    let nested = ACTIVE.with(|slot| slot.borrow().is_some());
    if nested || !installed() {
        if nested {
            NESTED.with(|flag| flag.set(true));
        }
        return original(country, detail);
    }
    // Alert-manager membership calls the same function with a null detail
    // sink. A true predicate result is itself a complete measured-empty
    // observation: the exact producer has proved there is no positive
    // manpower or whole-equipment shortage. Retaining this route is required
    // to distinguish an inactive alert from an unavailable producer. A false
    // predicate result is deliberately not published here because its
    // shortage children exist only on the subsequent detail call.
    if detail == 0 {
        let game_hour_raw = capture_game_hour_raw();
        let predicate_returned = original(country, detail) != 0;
        if predicate_returned {
            if let Some(game_hour_raw) = game_hour_raw {
                publish_observation(AlertObservation {
                    game_hour_raw,
                    country_ptr: country as u64,
                    predicate_returned,
                    missing_manpower: 0,
                    missing_equipment: Vec::new(),
                });
            }
        }
        return predicate_returned as u8;
    }
    ACTIVE.with(|slot| {
        *slot.borrow_mut() = Some(WorkingObservation {
            game_hour_raw: capture_game_hour_raw(),
            country_ptr: country,
            detail_output: detail,
            constructor_count: 0,
            final_accumulator: 0,
            missing_manpower: None,
            equipment_raw: BTreeMap::new(),
            poisoned: false,
        });
    });
    NESTED.with(|flag| flag.set(false));
    let predicate_returned = original(country, detail) != 0;
    let nested = NESTED.with(|flag| flag.replace(false));
    let working = ACTIVE.with(|slot| slot.borrow_mut().take());
    let Some(working) = working else {
        return predicate_returned as u8;
    };
    if nested
        || working.poisoned
        || working.constructor_count != 2
        || working.final_accumulator == 0
        || working.detail_output != detail
    {
        return predicate_returned as u8;
    }
    let Some(game_hour_raw) = working.game_hour_raw else {
        return predicate_returned as u8;
    };
    let mut missing_equipment = Vec::with_capacity(working.equipment_raw.len());
    for (archetype_id, (_, raw)) in working.equipment_raw {
        let whole_units = raw / FIXED_POINT_DIVISOR;
        if whole_units <= 0 {
            return predicate_returned as u8;
        }
        missing_equipment.push(AlertEquipmentShortage {
            archetype_id,
            whole_units,
        });
    }
    let missing_manpower = working.missing_manpower.unwrap_or(0);
    if (!predicate_returned) != (missing_manpower > 0 || !missing_equipment.is_empty()) {
        return predicate_returned as u8;
    }
    let observation = AlertObservation {
        game_hour_raw,
        country_ptr: country as u64,
        predicate_returned,
        missing_manpower,
        missing_equipment,
    };
    publish_observation(observation);
    predicate_returned as u8
}

fn publish_observation(observation: AlertObservation) {
    let Ok(mut poisoned) = POISONED_HOURS
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .try_lock()
    else {
        DROPPED_PUBLISH.store(true, Ordering::Release);
        return;
    };
    if poisoned.contains(&observation.game_hour_raw) {
        return;
    }
    if let Ok(mut published) = PUBLISHED
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .try_lock()
    {
        match published.entry(observation.game_hour_raw) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(observation);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get() != &observation {
                    // Two different observations in one game hour are not an
                    // atomic alert state. Delete the publication so d52 emits
                    // explicit source-unavailable rather than selecting one.
                    entry.remove();
                    poisoned.insert(observation.game_hour_raw);
                }
            }
        }
    } else {
        DROPPED_PUBLISH.store(true, Ordering::Release);
    }
}

unsafe extern "system" fn pool_ctor_detour(this: usize) -> usize {
    let _guard = CallbackGuard::enter();
    ACTIVE.with(|slot| {
        if let Some(active) = slot.borrow_mut().as_mut() {
            active.constructor_count = active.constructor_count.saturating_add(1);
            if active.constructor_count == 2 {
                active.final_accumulator = this;
            } else if active.constructor_count > 2 {
                active.poisoned = true;
            }
        }
    });
    let original: PoolCtorFn = mem::transmute(ORIGINAL_POOL_CTOR.load(Ordering::Acquire));
    original(this)
}

unsafe extern "system" fn pool_add_detour(pool: usize, archetype: usize, amount_raw: i64) {
    let _guard = CallbackGuard::enter();
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(active) = slot.as_mut() else {
            return;
        };
        if pool != active.final_accumulator || pool == 0 {
            return;
        }
        let id1 = read_u32(archetype.saturating_add(ARCHETYPE_STABLE_ID_OFF));
        let id2 = read_u32(archetype.saturating_add(ARCHETYPE_STABLE_ID_OFF));
        let Some(archetype_id) = id1.filter(|id| Some(*id) == id2 && *id != 0) else {
            active.poisoned = true;
            return;
        };
        if amount_raw <= 0 || active.equipment_raw.len() >= MAX_EQUIPMENT_ROWS {
            active.poisoned = true;
            return;
        }
        match active.equipment_raw.entry(archetype_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((archetype, amount_raw));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if entry.get().0 != archetype {
                    active.poisoned = true;
                } else if let Some(sum) = entry.get().1.checked_add(amount_raw) {
                    entry.get_mut().1 = sum;
                } else {
                    active.poisoned = true;
                }
            }
        }
    });
    let original: PoolAddFn = mem::transmute(ORIGINAL_POOL_ADD.load(Ordering::Acquire));
    original(pool, archetype, amount_raw);
}

unsafe extern "system" fn localization_detour(
    out: usize,
    key: usize,
    parameters: usize,
    parameter_count: u32,
) -> usize {
    let _guard = CallbackGuard::enter();
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(active) = slot.as_mut() else {
            return;
        };
        if parameter_count == 1
            && read_exact(key, MANPOWER_KEY.len()).as_deref() == Some(MANPOWER_KEY)
        {
            let value = read_i32(parameters.saturating_add(0x60));
            match value {
                Some(value) if value > 0 => active.missing_manpower = Some(i64::from(value)),
                _ => active.poisoned = true,
            }
        }
    });
    let original: LocalizationFn =
        mem::transmute(ORIGINAL_LOCALIZATION.load(Ordering::Acquire));
    original(out, key, parameters, parameter_count)
}

fn capture_game_hour_raw() -> Option<i32> {
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    let root1 = read_usize(base.checked_add(GAME_STATE_ROOT_RVA)?)?;
    let root2 = read_usize(base.checked_add(GAME_STATE_ROOT_RVA)?)?;
    if root1 == 0 || root1 != root2 {
        return None;
    }
    let address = root1.checked_add(compact_contract::build_profile::CGAMESTATE_CURRENT_HOUR)?;
    let hour1 = read_i32(address)?;
    let hour2 = read_i32(address)?;
    (hour1 == hour2 && compact_contract::calendar::hour_index_from_raw(hour1).is_some())
        .then_some(hour1)
}

fn read_exact(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 || len == 0 {
        return None;
    }
    let mut bytes = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            bytes.as_mut_ptr() as *mut c_void,
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(bytes)
}

fn read_u32(address: usize) -> Option<u32> {
    let bytes = read_exact(address, 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_i32(address: usize) -> Option<i32> {
    let bytes = read_exact(address, 4)?;
    Some(i32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_usize(address: usize) -> Option<usize> {
    let bytes = read_exact(address, mem::size_of::<usize>())?;
    Some(usize::from_le_bytes(bytes.try_into().ok()?))
}
