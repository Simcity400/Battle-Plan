//! Evidence-only paired detours for the exact a729 daily modifier lifecycle.
//!
//! The outer detour observes a coherent container before and after the daily
//! update. The nested erase detour copies the erased rows before the engine
//! moves/destroys them, which is the only reliable place to retain the exact
//! zero state. Installation is byte-sentinel gated and requires an explicit
//! owner-run environment switch. It does not enable production DMI2 output.

#![cfg(windows)]

use crate::{
    DailyUpdateWitness, DynamicModifierWitnessRow, EraseBoundaryWitness,
    LifecycleWitnessDiagnostics, LifecycleWitnessProcessIdentity, LifecycleWitnessReceipt, EXECUTABLE_SHA256,
    ZERO_REMOVAL_JOB_ID, ZERO_REMOVAL_MANIFEST_SHA256,
};
use core::{cell::RefCell, ffi::c_void, mem, ptr};
use minhook::{MinHook, MH_STATUS};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetProcessTimes,
};

pub const DAILY_UPDATE_RVA: usize = 0x0054_BEB0;
pub const ENTRY_ERASE_RVA: usize = 0x0060_4D30;
pub const EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_A18_DYNMOD_EVIDENCE";
pub const MAX_PENDING_WITNESSES: usize = 4_096;

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const CGAMESTATE_CURRENT_DATE_OFF: usize = 0x468;
const COUNTRY_DYNAMIC_MODIFIER_CONTAINER_OFF: usize = 0xE58;
const COUNTRY_TAG_DB_ID_OFF: usize = 0x08;
const CONTAINER_VTABLE_RVA: usize = 0x027B_FDE8;
const DEFINITION_VTABLE_RVA: usize = 0x027B_FD48;
const CONTAINER_BACKREF_OFF: usize = 0x08;
const CONTAINER_ENTRY_VECTOR_OFF: usize = 0x28;
const CONTAINER_COUNT_OFF: usize = 0x34;
const ENTRY_STRIDE: usize = 0x40;
const ENTRY_DAYS_OFF: usize = 0x10;
const ENTRY_DEFINITION_OFF: usize = 0x18;
const ENTRY_ENABLED_OFF: usize = 0x20;
const ENTRY_VALUES_BEGIN_OFF: usize = 0x28;
const ENTRY_VALUES_CAPACITY_OFF: usize = 0x30;
const ENTRY_VALUES_COUNT_OFF: usize = 0x34;
const DEFINITION_ID_OFF: usize = 0x28;
const MAX_ENTRIES_PER_COUNTRY: u32 = 512;
const MAX_VALUES_PER_ENTRY: u32 = 256;
const MAX_MODIFIER_ID_LEN: usize = 128;
const HEAP_LO: usize = 0x100_0000_0000;
const HEAP_HI: usize = 0x7ff0_0000_0000;

const DAILY_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
    0x18, 0x57,
];
const ENTRY_ERASE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x41, 0x54, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20,
    0x48, 0x8b,
];

type DailyUpdateFn = unsafe extern "system" fn(usize) -> i64;
type EntryEraseFn = unsafe extern "system" fn(usize, i32, i32) -> i32;

static ORIGINAL_DAILY_UPDATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENTRY_ERASE: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static IGNORED_WITHOUT_TIMED: AtomicU64 = AtomicU64::new(0);
static DROPPED_REENTRANT: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static COMPLETENESS_POISONED: AtomicBool = AtomicBool::new(false);
static QUEUE: OnceLock<Mutex<VecDeque<DailyUpdateWitness>>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static OPEN_UPDATE: RefCell<Option<OpenUpdate>> = const { RefCell::new(None) };
}

#[derive(Debug)]
struct OpenUpdate {
    vector_header: usize,
    erase_boundaries: Vec<EraseBoundaryWitness>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub daily_update: usize,
    pub entry_erase: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDrain {
    pub hook_epoch: u64,
    pub witnesses: Vec<DailyUpdateWitness>,
    pub diagnostics: LifecycleWitnessDiagnostics,
}

pub fn evidence_activation_enabled() -> bool {
    std::env::var(EVIDENCE_ACTIVATION_ENV).ok().as_deref() == Some("1")
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn install_fixture_hooks() -> Result<InstalledHookAddresses, String> {
    if !evidence_activation_enabled() {
        return Err(format!("A18 evidence hook requires {EVIDENCE_ACTIVATION_ENV}=1"));
    }
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A18 install lock poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A18 GetModuleHandleW(NULL) returned null".into());
    }
    let sites = addresses(base);
    verify_sentinel(sites.daily_update, &DAILY_UPDATE_SENTINEL)?;
    verify_sentinel(sites.entry_erase, &ENTRY_ERASE_SENTINEL)?;

    let epoch = NEXT_HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    if epoch == 0 {
        return Err("A18 hook epoch exhausted".into());
    }
    reset_state(epoch)?;
    let original_update = match unsafe {
        MinHook::create_hook(sites.daily_update as *mut c_void, hook_daily_update as *mut c_void)
    } {
        Ok(original) => original,
        Err(status) => {
            clear_hook_state();
            return Err(format!("A18 daily-update create failed: {status}"));
        }
    };
    ORIGINAL_DAILY_UPDATE.store(original_update as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    let original_erase = match unsafe {
        MinHook::create_hook(sites.entry_erase as *mut c_void, hook_entry_erase as *mut c_void)
    } {
        Ok(original) => original,
        Err(status) => {
            if let Err(cleanup) = cleanup_after_failed_install(sites) {
                INSTALLED.store(true, Ordering::Release);
                return Err(format!("A18 erase create failed: {status}; {cleanup}"));
            }
            clear_hook_state();
            return Err(format!("A18 erase create failed: {status}"));
        }
    };
    ORIGINAL_ENTRY_ERASE.store(original_erase as usize, Ordering::Release);

    // Enable the nested observer first. It is inert without OPEN_UPDATE.
    for target in [sites.entry_erase, sites.daily_update] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            if let Err(cleanup) = cleanup_after_failed_install(sites) {
                // Preserve originals/module identity and make collector
                // teardown retry. Never hide a detour that may still target
                // this DLL before FreeLibrary.
                INSTALLED.store(true, Ordering::Release);
                return Err(format!(
                    "A18 hook enable failed at 0x{target:X}: {status}; {cleanup}"
                ));
            }
            clear_hook_state();
            return Err(format!("A18 hook enable failed at 0x{target:X}: {status}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn drain() -> Result<HookDrain, String> {
    let witnesses = QUEUE
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_WITNESSES)))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned")?
        .drain(..)
        .collect::<Vec<_>>();
    Ok(HookDrain {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        diagnostics: diagnostics(witnesses.len() as u64),
        witnesses,
    })
}

pub fn receipt_from_drain(drain: HookDrain) -> LifecycleWitnessReceipt {
    LifecycleWitnessReceipt {
        schema: "hoi4.a18.dynamic_modifier_lifecycle_witness.v1".into(),
        executable_sha256: EXECUTABLE_SHA256.into(),
        exact20_job_id: ZERO_REMOVAL_JOB_ID.into(),
        exact20_manifest_sha256: ZERO_REMOVAL_MANIFEST_SHA256.into(),
        update_function_rva: DAILY_UPDATE_RVA as u32,
        erase_function_rva: ENTRY_ERASE_RVA as u32,
        process_identity: current_process_identity(),
        evidence_install: true,
        hook_epoch: drain.hook_epoch,
        witnesses: drain.witnesses,
        diagnostics: drain.diagnostics,
        production_admission_ready: false,
    }
}

fn current_process_identity() -> LifecycleWitnessProcessIdentity {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let process = unsafe { GetCurrentProcess() };
    let has_times =
        unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } != 0;
    let create_time_100ns = if has_times {
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime)
    } else {
        0
    };
    LifecycleWitnessProcessIdentity {
        pid: unsafe { GetCurrentProcessId() },
        create_time_100ns,
        executable_sha256: EXECUTABLE_SHA256.into(),
    }
}

pub fn seal_drain_and_uninstall() -> Result<HookDrain, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A18 teardown lock poisoned")?;
    if !INSTALLED.load(Ordering::Acquire) {
        return drain();
    }
    let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
    disable(sites.daily_update)?;
    wait_for_callbacks()?;
    disable(sites.entry_erase)?;
    wait_for_callbacks()?;
    let drained = drain()?;
    for target in [sites.daily_update, sites.entry_erase] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A18 hook remove failed: {status}")),
        }
    }
    INSTALLED.store(false, Ordering::Release);
    clear_hook_state();
    Ok(drained)
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        daily_update: base + DAILY_UPDATE_RVA,
        entry_erase: base + ENTRY_ERASE_RVA,
    }
}

fn reset_state(epoch: u64) -> Result<(), String> {
    QUEUE
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_WITNESSES)))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned before install")?
        .clear();
    ACTIVE_HOOK_EPOCH.store(epoch, Ordering::Release);
    NEXT_SEQUENCE.store(1, Ordering::Release);
    IGNORED_WITHOUT_TIMED.store(0, Ordering::Release);
    DROPPED_REENTRANT.store(0, Ordering::Release);
    DROPPED_UNREADABLE.store(0, Ordering::Release);
    DROPPED_LOCK_BUSY.store(0, Ordering::Release);
    DROPPED_OVERFLOW.store(0, Ordering::Release);
    COMPLETENESS_POISONED.store(false, Ordering::Release);
    Ok(())
}

fn diagnostics(retained_updates: u64) -> LifecycleWitnessDiagnostics {
    LifecycleWitnessDiagnostics {
        retained_updates,
        ignored_without_timed_instance: IGNORED_WITHOUT_TIMED.load(Ordering::Acquire),
        dropped_reentrant: DROPPED_REENTRANT.load(Ordering::Acquire),
        dropped_unreadable: DROPPED_UNREADABLE.load(Ordering::Acquire),
        dropped_lock_busy: DROPPED_LOCK_BUSY.load(Ordering::Acquire),
        dropped_overflow: DROPPED_OVERFLOW.load(Ordering::Acquire),
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
    }
}

unsafe extern "system" fn hook_daily_update(container: usize) -> i64 {
    let _active = CallbackGuard::enter();
    let original_address = ORIGINAL_DAILY_UPDATE.load(Ordering::Acquire);
    if original_address == 0 {
        poison(&DROPPED_UNREADABLE);
        return 0;
    }
    let original: DailyUpdateFn = unsafe { mem::transmute(original_address) };
    let nested = OPEN_UPDATE.with(|slot| slot.borrow().is_some());
    if nested {
        poison(&DROPPED_REENTRANT);
        return unsafe { original(container) };
    }
    let before_hour = current_game_hour_raw();
    let owner = container
        .checked_sub(COUNTRY_DYNAMIC_MODIFIER_CONTAINER_OFF)
        .and_then(|country| country.checked_add(COUNTRY_TAG_DB_ID_OFF))
        .and_then(read_stable::<u32>);
    let before = snapshot_container(container);
    let vector_header = container.checked_add(CONTAINER_ENTRY_VECTOR_OFF);
    OPEN_UPDATE.with(|slot| {
        *slot.borrow_mut() = Some(OpenUpdate {
            vector_header: vector_header.unwrap_or(0),
            erase_boundaries: Vec::new(),
            poisoned: vector_header.is_none(),
        });
    });
    let result = unsafe { original(container) };
    let open = OPEN_UPDATE.with(|slot| slot.borrow_mut().take());
    let after = snapshot_container(container);
    let after_hour = current_game_hour_raw();

    let (Some(owner_tag_db_id), Some(game_hour_raw_before), Some(game_hour_raw_after), Some(before), Some(after), Some(open)) =
        (owner, before_hour, after_hour, before, after, open)
    else {
        poison(&DROPPED_UNREADABLE);
        return result;
    };
    if owner_tag_db_id == 0 || open.poisoned {
        poison(&DROPPED_UNREADABLE);
        return result;
    }
    let relevant = before.iter().any(|row| row.remaining_days.is_some())
        || after.iter().any(|row| row.remaining_days.is_some())
        || open.erase_boundaries.iter().any(|boundary| {
            boundary.rows_before_erase.iter().any(|row| row.remaining_days == Some(0))
        });
    if !relevant {
        IGNORED_WITHOUT_TIMED.fetch_add(1, Ordering::AcqRel);
        return result;
    }
    let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if sequence == 0 {
        poison(&DROPPED_OVERFLOW);
        return result;
    }
    retain(DailyUpdateWitness {
        sequence,
        thread_id: unsafe { GetCurrentThreadId() },
        owner_tag_db_id,
        game_hour_raw_before,
        game_hour_raw_after,
        before,
        erase_boundaries: open.erase_boundaries,
        after,
    });
    result
}

unsafe extern "system" fn hook_entry_erase(vector_header: usize, start: i32, end: i32) -> i32 {
    let _active = CallbackGuard::enter();
    let original_address = ORIGINAL_ENTRY_ERASE.load(Ordering::Acquire);
    if original_address == 0 {
        poison(&DROPPED_UNREADABLE);
        return 0;
    }
    let original: EntryEraseFn = unsafe { mem::transmute(original_address) };
    OPEN_UPDATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(open) = slot.as_mut() else { return };
        if open.vector_header != vector_header || start < 0 || end <= start {
            open.poisoned = true;
            return;
        }
        match snapshot_vector_range(vector_header, start as u32, end as u32) {
            Some(rows) => open.erase_boundaries.push(EraseBoundaryWitness {
                start_index: start as u32,
                end_index: end as u32,
                rows_before_erase: rows,
            }),
            None => open.poisoned = true,
        }
    });
    unsafe { original(vector_header, start, end) }
}

fn retain(witness: DailyUpdateWitness) {
    let queue = QUEUE.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_WITNESSES)));
    match queue.try_lock() {
        Ok(mut queue) if queue.len() < MAX_PENDING_WITNESSES => queue.push_back(witness),
        Ok(_) => poison(&DROPPED_OVERFLOW),
        Err(_) => poison(&DROPPED_LOCK_BUSY),
    }
}

fn poison(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::AcqRel);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
}

fn snapshot_container(container: usize) -> Option<Vec<DynamicModifierWitnessRow>> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let owner = container.checked_sub(COUNTRY_DYNAMIC_MODIFIER_CONTAINER_OFF)?;
    if read_stable::<usize>(container)? != base.checked_add(CONTAINER_VTABLE_RVA)?
        || read_stable::<usize>(container.checked_add(CONTAINER_BACKREF_OFF)?)? != owner
    {
        return None;
    }
    let vector = container.checked_add(CONTAINER_ENTRY_VECTOR_OFF)?;
    let count = read_stable::<u32>(container.checked_add(CONTAINER_COUNT_OFF)?)?;
    snapshot_vector_range(vector, 0, count)
}

fn snapshot_vector_range(
    vector_header: usize,
    start: u32,
    end: u32,
) -> Option<Vec<DynamicModifierWitnessRow>> {
    let data = read_stable::<usize>(vector_header)?;
    let capacity = read_stable::<u32>(vector_header.checked_add(8)?)?;
    let count = read_stable::<u32>(vector_header.checked_add(12)?)?;
    if count > capacity || count > MAX_ENTRIES_PER_COUNTRY || start > end || end > count {
        return None;
    }
    if count == 0 {
        return if start == 0 && end == 0 { Some(Vec::new()) } else { None };
    }
    if !heap_pointer(data) {
        return None;
    }
    let mut rows = Vec::with_capacity((end - start) as usize);
    for index in start..end {
        let entry = data.checked_add(index as usize * ENTRY_STRIDE)?;
        rows.push(read_entry(entry)?);
    }
    // Re-read the header so an update outside the paired detour cannot be
    // accepted as a coherent snapshot.
    if read_value::<usize>(vector_header) != Some(data)
        || read_value::<u32>(vector_header + 8) != Some(capacity)
        || read_value::<u32>(vector_header + 12) != Some(count)
    {
        return None;
    }
    Some(rows)
}

fn read_entry(entry: usize) -> Option<DynamicModifierWitnessRow> {
    let raw = read_bytes(entry, ENTRY_STRIDE)?;
    if read_bytes(entry, ENTRY_STRIDE)? != raw {
        return None;
    }
    let days_raw = i32::from_le_bytes(raw[ENTRY_DAYS_OFF..ENTRY_DAYS_OFF + 4].try_into().ok()?);
    let remaining_days = match days_raw {
        -1 => None,
        days if days >= 0 => Some(days as u32),
        _ => return None,
    };
    let definition = usize::from_le_bytes(
        raw[ENTRY_DEFINITION_OFF..ENTRY_DEFINITION_OFF + 8].try_into().ok()?,
    );
    if !heap_pointer(definition) {
        return None;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if read_stable::<usize>(definition)? != base.checked_add(DEFINITION_VTABLE_RVA)? {
        return None;
    }
    let modifier_id = read_msvc_string_stable(definition.checked_add(DEFINITION_ID_OFF)?)?;
    let enabled = match raw[ENTRY_ENABLED_OFF] {
        0 => false,
        1 => true,
        _ => return None,
    };
    let values_begin = usize::from_le_bytes(
        raw[ENTRY_VALUES_BEGIN_OFF..ENTRY_VALUES_BEGIN_OFF + 8].try_into().ok()?,
    );
    let values_capacity = u32::from_le_bytes(
        raw[ENTRY_VALUES_CAPACITY_OFF..ENTRY_VALUES_CAPACITY_OFF + 4]
            .try_into()
            .ok()?,
    );
    let values_count = u32::from_le_bytes(
        raw[ENTRY_VALUES_COUNT_OFF..ENTRY_VALUES_COUNT_OFF + 4]
            .try_into()
            .ok()?,
    );
    if values_count > values_capacity || values_count > MAX_VALUES_PER_ENTRY {
        return None;
    }
    let values_bytes = if values_count == 0 {
        Vec::new()
    } else {
        if !heap_pointer(values_begin) {
            return None;
        }
        let values = read_bytes(values_begin, values_count as usize * 8)?;
        if read_bytes(values_begin, values_count as usize * 8)? != values {
            return None;
        }
        values
    };
    let values_raw = values_bytes
        .chunks_exact(8)
        .map(|bytes| i64::from_le_bytes(bytes.try_into().expect("eight-byte chunk")))
        .collect::<Vec<_>>();
    let mut pointer_masked_entry_bytes = raw;
    for range in [0..8, ENTRY_DEFINITION_OFF..ENTRY_DEFINITION_OFF + 8, ENTRY_VALUES_BEGIN_OFF..ENTRY_VALUES_BEGIN_OFF + 8] {
        pointer_masked_entry_bytes[range].fill(0);
    }
    Some(DynamicModifierWitnessRow {
        modifier_id,
        enabled,
        values_raw,
        remaining_days,
        pointer_masked_entry_bytes,
    })
}

fn read_msvc_string_stable(address: usize) -> Option<String> {
    let first = read_msvc_string(address)?;
    let second = read_msvc_string(address)?;
    (first == second).then_some(first)
}

fn read_msvc_string(address: usize) -> Option<String> {
    let header = read_bytes(address, 0x20)?;
    let length = usize::from_le_bytes(header[0x10..0x18].try_into().ok()?);
    let capacity = usize::from_le_bytes(header[0x18..0x20].try_into().ok()?);
    if length == 0 || length > MAX_MODIFIER_ID_LEN || length > capacity || capacity >= (1 << 20) {
        return None;
    }
    let chars = if capacity == 15 {
        address
    } else {
        if capacity < 16 {
            return None;
        }
        usize::from_le_bytes(header[0..8].try_into().ok()?)
    };
    let bytes = read_bytes(chars, length.checked_add(1)?)?;
    if bytes.last().copied() != Some(0) {
        return None;
    }
    let text = std::str::from_utf8(&bytes[..length]).ok()?;
    if !text.bytes().all(|byte| byte.is_ascii_graphic() || byte == b' ') {
        return None;
    }
    Some(text.to_owned())
}

fn current_game_hour_raw() -> Option<i32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let game_state = read_value::<usize>(base.checked_add(CGAMESTATE_SINGLETON_RVA)?)?;
    if !heap_pointer(game_state) {
        return None;
    }
    read_stable(game_state.checked_add(CGAMESTATE_CURRENT_DATE_OFF)?)
}

fn heap_pointer(value: usize) -> bool {
    (HEAP_LO..HEAP_HI).contains(&value) && value % 8 == 0
}

fn read_stable<T: Copy + PartialEq>(address: usize) -> Option<T> {
    let first = read_value(address)?;
    let second = read_value(address)?;
    (first == second).then_some(first)
}

fn read_value<T: Copy>(address: usize) -> Option<T> {
    let bytes = read_bytes(address, mem::size_of::<T>())?;
    Some(unsafe { ptr::read_unaligned(bytes.as_ptr().cast::<T>()) })
}

fn read_bytes(address: usize, length: usize) -> Option<Vec<u8>> {
    if address == 0 || length == 0 {
        return None;
    }
    let mut bytes = vec![0u8; length];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            length,
            &mut read,
        )
    };
    (ok != 0 && read == length).then_some(bytes)
}

fn verify_sentinel(address: usize, expected: &[u8]) -> Result<(), String> {
    let actual = read_bytes(address, expected.len())
        .ok_or_else(|| format!("A18 sentinel unreadable at 0x{address:X}"))?;
    if actual != expected {
        return Err(format!("A18 exact-a729 sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn disable(target: usize) -> Result<(), String> {
    match unsafe { MinHook::disable_hook(target as *mut c_void) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => Ok(()),
        Err(status) => Err(format!("A18 hook disable failed: {status}")),
    }
}

fn cleanup_after_failed_install(sites: InstalledHookAddresses) -> Result<(), String> {
    let daily_disable = disable(sites.daily_update);
    let erase_disable = disable(sites.entry_erase);
    daily_disable?;
    erase_disable?;
    wait_for_callbacks()?;
    for target in [sites.entry_erase, sites.daily_update] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A18 rollback remove failed: {status}")),
        }
    }
    Ok(())
}

fn clear_hook_state() {
    ORIGINAL_DAILY_UPDATE.store(0, Ordering::Release);
    ORIGINAL_ENTRY_ERASE.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
}

fn wait_for_callbacks() -> Result<(), String> {
    let mut yields = 0u32;
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("A18 callbacks did not quiesce".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_sentinels_are_not_ambiguous_short_patterns() {
        assert_eq!(DAILY_UPDATE_SENTINEL.len(), 16);
        assert_eq!(ENTRY_ERASE_SENTINEL.len(), 16);
        assert_ne!(DAILY_UPDATE_SENTINEL, ENTRY_ERASE_SENTINEL);
    }

    #[test]
    fn production_latches_remain_closed() {
        assert!(!crate::PRODUCTION_DMI2_SCHEDULING_ENABLED);
        assert!(!crate::LIVE_FIXTURE_ACCEPTED);
    }
}
