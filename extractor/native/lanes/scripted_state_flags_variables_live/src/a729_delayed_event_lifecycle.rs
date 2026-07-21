//! Exact-a729 delayed-event lifecycle observer for A19.
//!
//! Two lane-owned detours bracket the country hourly update and definition
//! cancellation paths. Dispatch is observed through d75's already-installed
//! common fire detour; this module never installs a competing hook at

#![cfg(windows)]

use core::{cell::RefCell, ffi::c_void, mem, ptr};
use minhook::{MinHook, MH_STATUS};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const HOURLY_UPDATE_RVA: usize = 0x006E_FE10;
pub const DEFINITION_CANCEL_RVA: usize = 0x006D_D7F0;
pub const EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_A19_DELAYED_EVENT_EVIDENCE";
pub const PRODUCTION_SCHEDULING_ENABLED: bool = false;
pub const MAX_PENDING_EVENTS: usize = 65_536;

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const CGAMESTATE_CURRENT_DATE_OFF: usize = 0x468;
const CCOUNTRY_VTABLE_RVA: usize = 0x027D_0E90;
const CCOUNTRY_TAG_OFF: usize = 0x08;
const CCOUNTRY_DELAYED_BEGIN_OFF: usize = 0x1290;
const CCOUNTRY_DELAYED_CAPACITY_OFF: usize = 0x1298;
const CCOUNTRY_DELAYED_COUNT_OFF: usize = 0x129C;
const CDELAYED_EVENT_VTABLE_RVA: usize = 0x027D_0FB8;
const CDELAYED_EVENT_EVENT_OFF: usize = 0x08;
const CDELAYED_EVENT_SCOPE_OFF: usize = 0x10;
const CDELAYED_EVENT_SCOPE_TAG_OFF: usize = 0x18;
const CDELAYED_EVENT_SCOPE_RANDOM_A_OFF: usize = 0x1C;
const CDELAYED_EVENT_SCOPE_RANDOM_B_OFF: usize = 0x20;
const CDELAYED_EVENT_ORIGINATOR_OFF: usize = 0xC0;
const CDELAYED_EVENT_DUE_OFF: usize = 0xC4;
const CEVENT_COMBINED_ID_OFF: usize = 0x0C;
const MAX_COUNTRY_TAG_DB_ID: u32 = 100_000;
const MIN_COMBINED_EVENT_ID: u32 = 100_000;
const MAX_COMBINED_EVENT_ID: u32 = 100_099_999;
const MAX_DUE_HOURS: u32 = 500_000;
const MAX_EVENTS_PER_COUNTRY: u32 = 65_536;
const HEAP_LO: usize = 0x100_0000_0000;
const HEAP_HI: usize = 0x7ff0_0000_0000;

const HOURLY_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
    0x20, 0x57,
];
const DEFINITION_CANCEL_SENTINEL: [u8; 16] = [
    0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x89, 0x5c, 0x24, 0x40, 0x48, 0x8b, 0xf9,
    0x33, 0xdb,
];

type HourlyUpdateFn = unsafe extern "system" fn(usize);
type DefinitionCancelFn = unsafe extern "system" fn(usize, usize);

static ORIGINAL_HOURLY_UPDATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DEFINITION_CANCEL: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static DISPATCH_OBSERVER_REGISTERED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static COMPLETENESS_POISONED: AtomicBool = AtomicBool::new(false);
static DROPPED_REENTRANT: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_CORRELATION: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static QUEUE: OnceLock<Mutex<VecDeque<LifecycleEvent>>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static OPEN_CALL: RefCell<Option<OpenCall>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum LifecycleReason {
    RetainedDecrement = 1,
    Executed = 2,
    FailedTriggerConsumed = 3,
    DefinitionCancelled = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub thread_id: u32,
    pub owner_tag_db_id: u32,
    pub event_id: u32,
    pub originator_tag_db_id: u32,
    pub scope_random: u64,
    pub due_before: u32,
    pub due_after: Option<u32>,
    pub reason: LifecycleReason,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleDiagnostics {
    pub hook_epoch: u64,
    pub dispatch_observer_registered: bool,
    pub completeness_poisoned: bool,
    pub dropped_reentrant: u64,
    pub dropped_unreadable: u64,
    pub dropped_correlation: u64,
    pub dropped_overflow: u64,
    pub dropped_lock_busy: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleDrain {
    pub events: Vec<LifecycleEvent>,
    pub diagnostics: LifecycleDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub hourly_update: usize,
    pub definition_cancel: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallKind {
    Hourly,
    DefinitionCancel { event: usize },
}

#[derive(Clone, Debug)]
struct SnapshotRow {
    object: usize,
    event: usize,
    event_scope: usize,
    owner_tag_db_id: u32,
    event_id: u32,
    originator_tag_db_id: u32,
    scope_random: u64,
    due: u32,
}

#[derive(Debug)]
struct OpenCall {
    kind: CallKind,
    owner_tag_db_id: u32,
    before: Vec<SnapshotRow>,
    dispatched_objects: BTreeSet<usize>,
    poisoned: bool,
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

pub fn evidence_activation_enabled() -> bool {
    std::env::var(EVIDENCE_ACTIVATION_ENV).ok().as_deref() == Some("1")
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn set_dispatch_observer_registered(registered: bool) {
    DISPATCH_OBSERVER_REGISTERED.store(registered, Ordering::Release);
    if !registered && INSTALLED.load(Ordering::Acquire) {
        poison(&DROPPED_CORRELATION);
    }
}

/// Called synchronously from d75's authenticated common-fire detour. No raw
/// address is retained beyond this function.
pub fn observe_dispatch(
    event: usize,
    actor_tag_id: u32,
    recipient_tag_id: u32,
    event_scope: usize,
) {
    OPEN_CALL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(open) = slot.as_mut() else { return };
        if !matches!(open.kind, CallKind::Hourly) || recipient_tag_id != open.owner_tag_db_id {
            return;
        }
        let matches = open
            .before
            .iter()
            .filter(|row| {
                row.event == event
                    && row.event_scope == event_scope
                    && row.originator_tag_db_id == actor_tag_id
                    && row.owner_tag_db_id == recipient_tag_id
            })
            .map(|row| row.object)
            .collect::<Vec<_>>();
        if let [object] = matches.as_slice() {
            if !open.dispatched_objects.insert(*object) {
                open.poisoned = true;
            }
        } else {
            open.poisoned = true;
        }
    });
}

/// Fail closed when the shared d75 detour observed a dispatch but its bridge
/// could not authenticate the raw arguments. Without this notification, an
/// executed delayed event could be mislabeled as a false-trigger consumption.
pub fn observe_dispatch_unavailable() {
    OPEN_CALL.with(|slot| {
        if let Some(open) = slot.borrow_mut().as_mut() {
            if matches!(open.kind, CallKind::Hourly) {
                open.poisoned = true;
            }
        }
    });
}

pub fn install_fixture_hooks() -> Result<InstalledHookAddresses, String> {
    if !evidence_activation_enabled() {
        return Err(format!("A19 evidence hooks require {EVIDENCE_ACTIVATION_ENV}=1"));
    }
    install_hooks()
}

pub fn install_production_hooks() -> Result<InstalledHookAddresses, String> {
    if !PRODUCTION_SCHEDULING_ENABLED {
        return Err("A19 lifecycle production scheduling is not promoted".into());
    }
    install_hooks()
}

fn install_hooks() -> Result<InstalledHookAddresses, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A19 install lock poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    if !DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire) {
        return Err("A19 shared d75 dispatch observer is not registered".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A19 GetModuleHandleW(NULL) returned null".into());
    }
    let sites = addresses(base);
    verify_sentinel(sites.hourly_update, &HOURLY_UPDATE_SENTINEL)?;
    verify_sentinel(sites.definition_cancel, &DEFINITION_CANCEL_SENTINEL)?;
    let epoch = NEXT_HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    if epoch == 0 {
        return Err("A19 hook epoch exhausted".into());
    }
    reset_state(epoch)?;

    let original_hourly = unsafe {
        MinHook::create_hook(
            sites.hourly_update as *mut c_void,
            hook_hourly_update as *mut c_void,
        )
    }
    .map_err(|status| format!("A19 hourly hook create failed: {status}"))?;
    ORIGINAL_HOURLY_UPDATE.store(original_hourly as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);

    let original_cancel = match unsafe {
        MinHook::create_hook(
            sites.definition_cancel as *mut c_void,
            hook_definition_cancel as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(sites.hourly_update as *mut c_void) };
            clear_hook_state();
            return Err(format!("A19 cancellation hook create failed: {status}"));
        }
    };
    ORIGINAL_DEFINITION_CANCEL.store(original_cancel as usize, Ordering::Release);
    for target in [sites.definition_cancel, sites.hourly_update] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            let _ = cleanup_hooks(sites);
            clear_hook_state();
            return Err(format!("A19 hook enable failed at 0x{target:X}: {status}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn drain() -> Result<LifecycleDrain, String> {
    let events = QUEUE
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)))
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?
        .drain(..)
        .collect();
    Ok(LifecycleDrain {
        events,
        diagnostics: diagnostics(),
    })
}

pub fn seal_drain_and_uninstall() -> Result<LifecycleDrain, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A19 teardown lock poisoned")?;
    if !INSTALLED.load(Ordering::Acquire) {
        return drain();
    }
    let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
    cleanup_hooks(sites)?;
    wait_for_callbacks()?;
    let drained = drain()?;
    INSTALLED.store(false, Ordering::Release);
    clear_hook_state();
    Ok(drained)
}

unsafe extern "system" fn hook_hourly_update(country: usize) {
    let _active = CallbackGuard::enter();
    let original_address = ORIGINAL_HOURLY_UPDATE.load(Ordering::Acquire);
    if original_address == 0 {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let original: HourlyUpdateFn = unsafe { mem::transmute(original_address) };
    if OPEN_CALL.with(|slot| slot.borrow().is_some()) {
        poison(&DROPPED_REENTRANT);
        unsafe { original(country) };
        return;
    }
    let before = snapshot_country(country);
    match before {
        Some((owner_tag_db_id, rows)) => OPEN_CALL.with(|slot| {
            *slot.borrow_mut() = Some(OpenCall {
                kind: CallKind::Hourly,
                owner_tag_db_id,
                before: rows,
                dispatched_objects: BTreeSet::new(),
                poisoned: !DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire),
            })
        }),
        None => poison(&DROPPED_UNREADABLE),
    }
    unsafe { original(country) };
    let open = OPEN_CALL.with(|slot| slot.borrow_mut().take());
    let after = snapshot_country(country);
    let Some(open) = open else { return };
    let Some((after_owner, after_rows)) = after else {
        poison(&DROPPED_UNREADABLE);
        return;
    };
    if open.poisoned || after_owner != open.owner_tag_db_id {
        poison(&DROPPED_CORRELATION);
        return;
    }
    retain_hourly(open, after_rows);
}

unsafe extern "system" fn hook_definition_cancel(country: usize, event: usize) {
    let _active = CallbackGuard::enter();
    let original_address = ORIGINAL_DEFINITION_CANCEL.load(Ordering::Acquire);
    if original_address == 0 {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let original: DefinitionCancelFn = unsafe { mem::transmute(original_address) };
    if OPEN_CALL.with(|slot| slot.borrow().is_some()) {
        poison(&DROPPED_REENTRANT);
        unsafe { original(country, event) };
        return;
    }
    let before = snapshot_country(country);
    match before {
        Some((owner_tag_db_id, rows)) => OPEN_CALL.with(|slot| {
            *slot.borrow_mut() = Some(OpenCall {
                kind: CallKind::DefinitionCancel { event },
                owner_tag_db_id,
                before: rows,
                dispatched_objects: BTreeSet::new(),
                poisoned: false,
            })
        }),
        None => poison(&DROPPED_UNREADABLE),
    }
    unsafe { original(country, event) };
    let open = OPEN_CALL.with(|slot| slot.borrow_mut().take());
    let after = snapshot_country(country);
    let (Some(open), Some((after_owner, after_rows))) = (open, after) else {
        poison(&DROPPED_UNREADABLE);
        return;
    };
    if open.poisoned || after_owner != open.owner_tag_db_id {
        poison(&DROPPED_CORRELATION);
        return;
    }
    retain_cancellations(open, after_rows);
}

fn retain_hourly(open: OpenCall, after: Vec<SnapshotRow>) {
    let after_by_object = after
        .iter()
        .map(|row| (row.object, row))
        .collect::<BTreeMap<_, _>>();
    let mut emitted = Vec::with_capacity(open.before.len());
    for before in &open.before {
        if let Some(after) = after_by_object.get(&before.object) {
            if !same_identity(before, after) || before.due.checked_sub(1) != Some(after.due) {
                poison(&DROPPED_CORRELATION);
                return;
            }
            emitted.push(event_from(
                before,
                Some(after.due),
                LifecycleReason::RetainedDecrement,
            ));
        } else {
            let reason = if open.dispatched_objects.contains(&before.object) {
                LifecycleReason::Executed
            } else {
                LifecycleReason::FailedTriggerConsumed
            };
            emitted.push(event_from(before, None, reason));
        }
    }
    if open
        .dispatched_objects
        .iter()
        .any(|object| after_by_object.contains_key(object))
    {
        poison(&DROPPED_CORRELATION);
        return;
    }
    retain_all(emitted);
}

fn retain_cancellations(open: OpenCall, after: Vec<SnapshotRow>) {
    let CallKind::DefinitionCancel { event } = open.kind else {
        poison(&DROPPED_CORRELATION);
        return;
    };
    let after_objects = after.iter().map(|row| row.object).collect::<BTreeSet<_>>();
    let mut emitted = Vec::new();
    for row in &open.before {
        if !after_objects.contains(&row.object) {
            if row.event != event {
                poison(&DROPPED_CORRELATION);
                return;
            }
            emitted.push(event_from(
                row,
                None,
                LifecycleReason::DefinitionCancelled,
            ));
        }
    }
    retain_all(emitted);
}

fn event_from(
    row: &SnapshotRow,
    due_after: Option<u32>,
    reason: LifecycleReason,
) -> LifecycleEvent {
    LifecycleEvent {
        sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel),
        game_hour_raw: current_game_hour_raw().unwrap_or(i32::MIN),
        thread_id: unsafe { GetCurrentThreadId() },
        owner_tag_db_id: row.owner_tag_db_id,
        event_id: row.event_id,
        originator_tag_db_id: row.originator_tag_db_id,
        scope_random: row.scope_random,
        due_before: row.due,
        due_after,
        reason,
    }
}

fn retain_all(events: Vec<LifecycleEvent>) {
    if events.iter().any(|row| row.sequence == 0 || row.game_hour_raw == i32::MIN) {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let queue = QUEUE.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)));
    match queue.try_lock() {
        Ok(mut queue) if queue.len().saturating_add(events.len()) <= MAX_PENDING_EVENTS => {
            queue.extend(events)
        }
        Ok(_) => poison(&DROPPED_OVERFLOW),
        Err(_) => poison(&DROPPED_LOCK_BUSY),
    }
}

fn same_identity(a: &SnapshotRow, b: &SnapshotRow) -> bool {
    a.event == b.event
        && a.event_scope == b.event_scope
        && a.owner_tag_db_id == b.owner_tag_db_id
        && a.event_id == b.event_id
        && a.originator_tag_db_id == b.originator_tag_db_id
        && a.scope_random == b.scope_random
}

fn snapshot_country(country: usize) -> Option<(u32, Vec<SnapshotRow>)> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if !heap_pointer(country)
        || read_stable::<usize>(country)? != base.checked_add(CCOUNTRY_VTABLE_RVA)?
    {
        return None;
    }
    let owner_tag_db_id = read_stable::<u32>(country.checked_add(CCOUNTRY_TAG_OFF)?)?;
    if !valid_tag(owner_tag_db_id) {
        return None;
    }
    let begin_addr = country.checked_add(CCOUNTRY_DELAYED_BEGIN_OFF)?;
    let capacity_addr = country.checked_add(CCOUNTRY_DELAYED_CAPACITY_OFF)?;
    let count_addr = country.checked_add(CCOUNTRY_DELAYED_COUNT_OFF)?;
    let header = (
        read_value::<usize>(begin_addr)?,
        read_value::<u32>(capacity_addr)?,
        read_value::<u32>(count_addr)?,
    );
    if header != (
        read_value::<usize>(begin_addr)?,
        read_value::<u32>(capacity_addr)?,
        read_value::<u32>(count_addr)?,
    ) || header.2 > header.1
        || header.2 > MAX_EVENTS_PER_COUNTRY
        || (header.2 != 0 && !heap_pointer(header.0))
    {
        return None;
    }
    let mut rows = Vec::with_capacity(header.2 as usize);
    for index in 0..header.2 as usize {
        let object = read_stable::<usize>(header.0.checked_add(index.checked_mul(8)?)?)?;
        rows.push(snapshot_row(base, owner_tag_db_id, object)?);
    }
    if header
        != (
            read_value::<usize>(begin_addr)?,
            read_value::<u32>(capacity_addr)?,
            read_value::<u32>(count_addr)?,
        )
    {
        return None;
    }
    Some((owner_tag_db_id, rows))
}

fn snapshot_row(base: usize, owner_tag_db_id: u32, object: usize) -> Option<SnapshotRow> {
    if !heap_pointer(object)
        || read_stable::<usize>(object)? != base.checked_add(CDELAYED_EVENT_VTABLE_RVA)?
    {
        return None;
    }
    let event = read_stable::<usize>(object.checked_add(CDELAYED_EVENT_EVENT_OFF)?)?;
    if !heap_pointer(event) {
        return None;
    }
    let event_id = read_stable::<u32>(event.checked_add(CEVENT_COMBINED_ID_OFF)?)?;
    let scope_tag_db_id =
        read_stable::<u32>(object.checked_add(CDELAYED_EVENT_SCOPE_TAG_OFF)?)?;
    let originator_tag_db_id =
        read_stable::<u32>(object.checked_add(CDELAYED_EVENT_ORIGINATOR_OFF)?)?;
    let random_a =
        read_stable::<u32>(object.checked_add(CDELAYED_EVENT_SCOPE_RANDOM_A_OFF)?)?;
    let random_b =
        read_stable::<u32>(object.checked_add(CDELAYED_EVENT_SCOPE_RANDOM_B_OFF)?)?;
    let due = read_stable::<u32>(object.checked_add(CDELAYED_EVENT_DUE_OFF)?)?;
    if scope_tag_db_id != owner_tag_db_id
        || !valid_tag(originator_tag_db_id)
        || !(MIN_COMBINED_EVENT_ID..=MAX_COMBINED_EVENT_ID).contains(&event_id)
        || due > MAX_DUE_HOURS
    {
        return None;
    }
    Some(SnapshotRow {
        object,
        event,
        event_scope: object.checked_add(CDELAYED_EVENT_SCOPE_OFF)?,
        owner_tag_db_id,
        event_id,
        originator_tag_db_id,
        scope_random: u64::from(random_a) | (u64::from(random_b) << 32),
        due,
    })
}

fn current_game_hour_raw() -> Option<i32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let singleton = read_stable::<usize>(base.checked_add(CGAMESTATE_SINGLETON_RVA)?)?;
    read_stable::<i32>(singleton.checked_add(CGAMESTATE_CURRENT_DATE_OFF)?)
}

fn valid_tag(value: u32) -> bool {
    value != 0 && value <= MAX_COUNTRY_TAG_DB_ID
}

fn heap_pointer(value: usize) -> bool {
    (HEAP_LO..HEAP_HI).contains(&value)
}

fn read_stable<T: Copy + PartialEq>(address: usize) -> Option<T> {
    let first = read_value::<T>(address)?;
    (read_value::<T>(address)? == first).then_some(first)
}

fn read_value<T: Copy>(address: usize) -> Option<T> {
    let mut value = mem::MaybeUninit::<T>::uninit();
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            value.as_mut_ptr() as *mut c_void,
            mem::size_of::<T>(),
            &mut read,
        )
    };
    (ok != 0 && read == mem::size_of::<T>()).then(|| unsafe { value.assume_init() })
}

fn verify_sentinel(address: usize, expected: &[u8]) -> Result<(), String> {
    let mut observed = vec![0u8; expected.len()];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            observed.as_mut_ptr() as *mut c_void,
            observed.len(),
            &mut read,
        )
    };
    if ok == 0 || read != expected.len() || observed != expected {
        return Err(format!("A19 exact-byte sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        hourly_update: base.saturating_add(HOURLY_UPDATE_RVA),
        definition_cancel: base.saturating_add(DEFINITION_CANCEL_RVA),
    }
}

fn cleanup_hooks(sites: InstalledHookAddresses) -> Result<(), String> {
    for target in [sites.hourly_update, sites.definition_cancel] {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A19 hook disable failed: {status}")),
        }
    }
    wait_for_callbacks()?;
    for target in [sites.hourly_update, sites.definition_cancel] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A19 hook remove failed: {status}")),
        }
    }
    Ok(())
}

fn wait_for_callbacks() -> Result<(), String> {
    for _ in 0..10_000 {
        if ACTIVE_CALLBACKS.load(Ordering::Acquire) == 0 {
            return Ok(());
        }
        std::thread::yield_now();
    }
    Err("A19 callbacks did not quiesce".into())
}

fn reset_state(epoch: u64) -> Result<(), String> {
    QUEUE
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)))
        .lock()
        .map_err(|_| "A19 queue lock poisoned")?
        .clear();
    ACTIVE_HOOK_EPOCH.store(epoch, Ordering::Release);
    NEXT_SEQUENCE.store(1, Ordering::Release);
    COMPLETENESS_POISONED.store(false, Ordering::Release);
    for counter in [
        &DROPPED_REENTRANT,
        &DROPPED_UNREADABLE,
        &DROPPED_CORRELATION,
        &DROPPED_OVERFLOW,
        &DROPPED_LOCK_BUSY,
    ] {
        counter.store(0, Ordering::Release);
    }
    Ok(())
}

fn clear_hook_state() {
    ORIGINAL_HOURLY_UPDATE.store(0, Ordering::Release);
    ORIGINAL_DEFINITION_CANCEL.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    OPEN_CALL.with(|slot| *slot.borrow_mut() = None);
}

fn poison(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::AcqRel);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
}

fn diagnostics() -> LifecycleDiagnostics {
    LifecycleDiagnostics {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        dispatch_observer_registered: DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire),
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
        dropped_reentrant: DROPPED_REENTRANT.load(Ordering::Acquire),
        dropped_unreadable: DROPPED_UNREADABLE.load(Ordering::Acquire),
        dropped_correlation: DROPPED_CORRELATION.load(Ordering::Acquire),
        dropped_overflow: DROPPED_OVERFLOW.load(Ordering::Acquire),
        dropped_lock_busy: DROPPED_LOCK_BUSY.load(Ordering::Acquire),
    }
}
