#![cfg(windows)]

use crate::{
    DailyUpdateWitness, DynamicModifierWitnessRow, EraseBoundaryWitness,
    LifecycleWitnessDiagnostics, LifecycleWitnessProcessIdentity, LifecycleWitnessReceipt,
    ProductionZeroRemoval, EXECUTABLE_SHA256, ZERO_REMOVAL_JOB_ID, ZERO_REMOVAL_MANIFEST_SHA256,
};
use core::{cell::RefCell, ffi::c_void, mem, ptr};
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released, ExpectedRvaHook,
    HookTeardownAttestation, InstallAbortDiscard, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
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
const COUNTRY_VTABLE_RVA: usize = 0x027D_0E90;
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
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
const ENTRY_ERASE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x41, 0x54, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b,
];
const PRODUCTION_HOOK_SPECS: [ExpectedRvaHook; 2] = [
    ExpectedRvaHook {
        logical_name: "gameplay.d67.daily_modifier_update",
        requested_rva: DAILY_UPDATE_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &DAILY_UPDATE_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d67.modifier_entry_erase",
        requested_rva: ENTRY_ERASE_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &ENTRY_ERASE_SENTINEL,
    },
];

const HOOK_NAMES: [&str; 2] = [
    "gameplay.d67.daily_modifier_update",
    "gameplay.d67.modifier_entry_erase",
];

fn hook_batch(sites: InstalledHookAddresses) -> [(&'static str, *mut c_void); 2] {
    let targets = [sites.daily_update, sites.entry_erase];
    core::array::from_fn(|index| (HOOK_NAMES[index], targets[index] as *mut c_void))
}

type DailyUpdateFn = unsafe extern "system" fn(usize) -> i64;
type EntryEraseFn = unsafe extern "system" fn(usize, i32, i32) -> i32;

static ORIGINAL_DAILY_UPDATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENTRY_ERASE: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static RELEASE_PENDING: AtomicBool = AtomicBool::new(false);
static RELEASE_CALLBACKS_QUIESCED: AtomicBool = AtomicBool::new(false);
static RELEASE_REMOVED_MASK: AtomicUsize = AtomicUsize::new(0);
static EVIDENCE_MODE: AtomicBool = AtomicBool::new(false);
static PRODUCTION_MODE: AtomicBool = AtomicBool::new(false);
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
static WITNESS_MUTATION_GENERATION: AtomicU64 = AtomicU64::new(1);
static PRODUCTION_ZERO_MUTATION_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static QUEUE: OnceLock<Mutex<ReservableQueue<DailyUpdateWitness>>> = OnceLock::new();
static PRODUCTION_ZERO_QUEUE: OnceLock<Mutex<ReservableQueue<ProductionZeroRemoval>>> =
    OnceLock::new();
static HOOK_RECEIPTS: OnceLock<Mutex<HookReceiptState>> = OnceLock::new();
static SEALED_TERMINAL: OnceLock<Mutex<Option<SealedTerminalState>>> = OnceLock::new();
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QueueReservation {
    id: u64,
    row_count: usize,
    owner: ReservationOwner,
    mutation_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReservationOwner {
    Delivery,
    Teardown,
}

#[derive(Debug)]
struct ReservableQueue<T> {
    pending: VecDeque<T>,
    reservation: Option<QueueReservation>,
}

impl<T> ReservableQueue<T> {
    fn new() -> Self {
        Self {
            pending: VecDeque::with_capacity(MAX_PENDING_WITNESSES),
            reservation: None,
        }
    }

    fn pending_count(&self) -> usize {
        self.pending.len().saturating_sub(
            self.reservation
                .map_or(0, |reservation| reservation.row_count),
        )
    }

    fn reserved_count(&self) -> usize {
        self.reservation
            .map_or(0, |reservation| reservation.row_count)
    }
}

#[derive(Default)]
struct HookReceiptState {
    installing: [Option<InstalledRvaHookReceipt>; 2],
    installed: Option<[InstalledRvaHookReceipt; 2]>,
    releasing: [Option<ReleasedRvaHookReceipt>; 2],
    released: Option<[ReleasedRvaHookReceipt; 2]>,
}

#[derive(Clone, Debug)]
struct SealedTerminalState {
    reservation_id: u64,
    drain: HookDrain,
    released: Option<[ReleasedRvaHookReceipt; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CountryContainerClass {
    Exact { owner_tag_db_id: u32 },
    Unrelated,
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
    pub witness_mutation_generation: u64,
    pub production_zero_mutation_generation: u64,
    pub witnesses: Vec<DailyUpdateWitness>,
    pub diagnostics: LifecycleWitnessDiagnostics,
    pub production_zero_removals: Vec<ProductionZeroRemoval>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProductionZeroDrain {
    pub mutation_generation: u64,
    pub events: Vec<ProductionZeroRemoval>,
    pub completeness_poisoned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WitnessReservation {
    pub reservation_id: u64,
    pub mutation_generation: u64,
    pub hook_epoch: u64,
    pub witnesses: Vec<DailyUpdateWitness>,
    pub diagnostics: LifecycleWitnessDiagnostics,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionZeroReservation {
    pub reservation_id: u64,
    pub mutation_generation: u64,
    pub drain: ProductionZeroDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedTerminalReservation {
    pub reservation_id: u64,
    pub drain: HookDrain,
}

pub fn evidence_activation_enabled() -> bool {
    std::env::var(EVIDENCE_ACTIVATION_ENV).ok().as_deref() == Some("1")
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn release_pending() -> bool {
    RELEASE_PENDING.load(Ordering::Acquire)
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; 2], String> {
    if !INSTALLED.load(Ordering::Acquire) || RELEASE_PENDING.load(Ordering::Acquire) {
        return Err("D67 production hooks are not live for installed-receipt observation".into());
    }
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("D67 production hook receipts are not initialized")?;
    let receipts = receipts
        .lock()
        .map_err(|_| "D67 production hook receipt mutex poisoned")?;
    let installed = receipts
        .installed
        .ok_or_else(|| "D67 exact installed-hook receipts are unavailable".to_string())?;
    let targets = addresses(MODULE_BASE.load(Ordering::Acquire));
    let targets = [targets.daily_update, targets.entry_erase];
    let detours = [hook_daily_update as usize, hook_entry_erase as usize];
    let mut reobserved = Vec::with_capacity(PRODUCTION_HOOK_SPECS.len());
    for (((expected, installed), target), detour) in PRODUCTION_HOOK_SPECS
        .into_iter()
        .zip(installed)
        .zip(targets)
        .zip(detours)
    {
        let observed =
            observe_enabled_minhook_rel5(expected, target, detour, installed.teardown_owner_token)?;
        if observed != installed {
            return Err(format!(
                "{} live installed-hook receipt changed",
                expected.logical_name
            ));
        }
        reobserved.push(observed);
    }
    reobserved
        .try_into()
        .map_err(|_| "D67 installed-hook reobservation cardinality mismatch".into())
}

pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 2], String> {
    if RELEASE_PENDING.load(Ordering::Acquire) {
        return Err("D67 release receipts are unavailable while teardown is pending".into());
    }
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("D67 production hook receipts are not initialized")?;
    let mut receipts = receipts
        .lock()
        .map_err(|_| "D67 production hook receipt mutex poisoned")?;
    receipts
        .released
        .take()
        .ok_or_else(|| "D67 exact released-hook receipts are unavailable".into())
}

pub fn production_callbacks_active() -> bool {
    ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0
}

pub fn production_callback_epoch() -> u64 {
    NEXT_SEQUENCE.load(Ordering::Acquire)
}

pub fn production_zero_mutation_generation() -> Result<u64, String> {
    if !INSTALLED.load(Ordering::Acquire) || RELEASE_PENDING.load(Ordering::Acquire) {
        return Err("A18 production zero mutation generation is not live".into());
    }
    if production_callbacks_active() {
        return Err("A18 production callback is active at mutation-generation read".into());
    }
    if COMPLETENESS_POISONED.load(Ordering::Acquire) {
        return Err("A18 production zero mutation generation is completeness-poisoned".into());
    }
    let generation = PRODUCTION_ZERO_MUTATION_GENERATION.load(Ordering::Acquire);
    if generation == 0 || generation == u64::MAX {
        return Err("A18 production zero mutation generation is exhausted".into());
    }
    Ok(generation)
}

pub fn witness_mutation_generation() -> Result<u64, String> {
    if !INSTALLED.load(Ordering::Acquire) || RELEASE_PENDING.load(Ordering::Acquire) {
        return Err("A18 witness mutation generation is not live".into());
    }
    if production_callbacks_active() {
        return Err("A18 callback is active at witness mutation-generation read".into());
    }
    if COMPLETENESS_POISONED.load(Ordering::Acquire) {
        return Err("A18 witness mutation generation is completeness-poisoned".into());
    }
    let generation = WITNESS_MUTATION_GENERATION.load(Ordering::Acquire);
    if generation == 0 || generation == u64::MAX {
        return Err("A18 witness mutation generation is exhausted".into());
    }
    Ok(generation)
}

pub fn verify_production_zero_unchanged_since(expected_generation: u64) -> Result<(), String> {
    if production_callbacks_active() {
        return Err("A18 production callback is active at post-walk fence".into());
    }
    let observed = production_zero_mutation_generation()?;
    if observed != expected_generation {
        return Err(format!(
            "A18 production zero queue changed during current-instance walk: expected generation \
             {expected_generation}, observed {observed}"
        ));
    }
    if production_callbacks_active() {
        return Err("A18 production callback became active across post-walk fence".into());
    }
    Ok(())
}

pub fn verify_production_zero_reservation_unchanged(
    reservation: &ProductionZeroReservation,
) -> Result<(), String> {
    if production_callbacks_active() {
        return Err("A18 production callback is active at reserved post-walk fence".into());
    }
    let queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned at post-walk fence")?;
    match queue.reservation {
        Some(active)
            if active.owner == ReservationOwner::Delivery
                && active.id == reservation.reservation_id
                && active.mutation_generation == reservation.mutation_generation => {}
        Some(active) => {
            return Err(format!(
                "A18 production zero reservation changed before post-walk fence: active={}, \
                 supplied={}",
                active.id, reservation.reservation_id
            ))
        }
        None => return Err("A18 production zero reservation ended before post-walk fence".into()),
    }
    verify_production_zero_unchanged_since(reservation.mutation_generation)
}

pub fn verify_witness_reservation_unchanged(
    reservation: &WitnessReservation,
) -> Result<(), String> {
    if production_callbacks_active() {
        return Err("A18 callback is active at witness post-publication fence".into());
    }
    let queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned at post-publication fence")?;
    match queue.reservation {
        Some(active)
            if active.owner == ReservationOwner::Delivery
                && active.id == reservation.reservation_id
                && active.mutation_generation == reservation.mutation_generation => {}
        Some(active) => {
            return Err(format!(
                "A18 witness reservation changed before post-publication fence: active={}, \
                 supplied={}",
                active.id, reservation.reservation_id
            ))
        }
        None => return Err("A18 witness reservation ended before post-publication fence".into()),
    }
    let observed = witness_mutation_generation()?;
    if observed != reservation.mutation_generation {
        return Err(format!(
            "A18 witness queue changed during publication: expected generation {}, observed \
             {observed}",
            reservation.mutation_generation
        ));
    }
    if production_callbacks_active() {
        return Err("A18 callback became active across witness post-publication fence".into());
    }
    Ok(())
}

pub fn install_fixture_hooks() -> Result<InstalledHookAddresses, String> {
    if !evidence_activation_enabled() {
        return Err(format!(
            "A18 evidence hook requires {EVIDENCE_ACTIVATION_ENV}=1"
        ));
    }
    install_hooks(true, false)
}

pub fn install_production_hooks() -> Result<InstalledHookAddresses, String> {
    if !crate::production_dmi2_admission_ready() {
        return Err("A18 production hook requires a fully pinned DMI2 admission".into());
    }
    install_hooks(false, true)
}

fn install_hooks(
    evidence_mode: bool,
    production_mode: bool,
) -> Result<InstalledHookAddresses, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A18 install lock poisoned")?;
    if RELEASE_PENDING.load(Ordering::Acquire) {
        return Err("A18 prior hook release is still pending".into());
    }
    if INSTALLED.load(Ordering::Acquire) {
        let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
        if production_mode {
            if hook_receipts_have_installed()? {
                installed_hook_receipts()?;
            } else {
                observe_install_receipts(sites)?;
            }
        }
        if evidence_mode {
            EVIDENCE_MODE.store(true, Ordering::Release);
        }
        if production_mode {
            PRODUCTION_MODE.store(true, Ordering::Release);
        }
        return Ok(sites);
    }
    prepare_hook_receipts_for_install()?;
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
    EVIDENCE_MODE.store(evidence_mode, Ordering::Release);
    PRODUCTION_MODE.store(production_mode, Ordering::Release);
    let original_update = match unsafe {
        MinHook::create_hook(
            sites.daily_update as *mut c_void,
            hook_daily_update as *mut c_void,
        )
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
        MinHook::create_hook(
            sites.entry_erase as *mut c_void,
            hook_entry_erase as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            if let Err(cleanup) = cleanup_after_failed_install(sites) {
                let pending = begin_release_pending()
                    .err()
                    .map(|error| format!("; release-pending state failed: {error}"))
                    .unwrap_or_default();
                return Err(format!(
                    "A18 erase create failed: {status}; {cleanup}{pending}"
                ));
            }
            clear_hook_state();
            return Err(format!("A18 erase create failed: {status}"));
        }
    };
    ORIGINAL_ENTRY_ERASE.store(original_erase as usize, Ordering::Release);

    let mut batch = hook_batch(sites);
    batch.reverse();
    let enabled = unsafe { inline_detour::enable_batch(&batch) };
    if let Some((name, status)) = enabled.refusal() {
        if let Err(cleanup) = cleanup_after_failed_install(sites) {
            let pending = begin_release_pending()
                .err()
                .map(|error| format!("; release-pending state failed: {error}"))
                .unwrap_or_default();
            return Err(format!(
                "A18 hook enable failed for {name}: {status}; {cleanup}{pending}"
            ));
        }
        clear_hook_state();
        return Err(format!("A18 hook enable failed for {name}: {status}"));
    }
    if production_mode {
        if let Err(error) = observe_install_receipts(sites) {
            if let Err(cleanup) = cleanup_after_failed_install(sites) {
                let pending = begin_release_pending()
                    .err()
                    .map(|pending| format!("; release-pending state failed: {pending}"))
                    .unwrap_or_default();
                return Err(format!(
                    "A18 exact installed-hook receipt failed: {error}; {cleanup}{pending}"
                ));
            }
            clear_hook_state();
            return Err(format!("A18 exact installed-hook receipt failed: {error}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn drain() -> Result<HookDrain, String> {
    let mut witness_queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned")?;
    let mut zero_queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned")?;
    if witness_queue.reservation.is_some() || zero_queue.reservation.is_some() {
        return Err(format!(
            "A18 drain blocked by live reservation(s): witness reserved={}, pending={}; \
             production zero reserved={}, pending={}",
            witness_queue.reserved_count(),
            witness_queue.pending_count(),
            zero_queue.reserved_count(),
            zero_queue.pending_count()
        ));
    }
    if !witness_queue.pending.is_empty() {
        advance_mutation_generation(&WITNESS_MUTATION_GENERATION)
            .map_err(|_| "A18 witness mutation generation exhausted before drain")?;
    }
    if !zero_queue.pending.is_empty() {
        advance_mutation_generation(&PRODUCTION_ZERO_MUTATION_GENERATION)
            .map_err(|_| "A18 production zero mutation generation exhausted before drain")?;
    }
    let witnesses = witness_queue.pending.drain(..).collect::<Vec<_>>();
    let production_zero_removals = zero_queue.pending.drain(..).collect::<Vec<_>>();
    Ok(HookDrain {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        witness_mutation_generation: WITNESS_MUTATION_GENERATION.load(Ordering::Acquire),
        production_zero_mutation_generation: PRODUCTION_ZERO_MUTATION_GENERATION
            .load(Ordering::Acquire),
        diagnostics: diagnostics(witnesses.len() as u64),
        witnesses,
        production_zero_removals,
    })
}

pub fn drain_production_zero_removals() -> Result<ProductionZeroDrain, String> {
    let mut queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned")?;
    reject_live_reservation("production zero", &queue)?;
    if !queue.pending.is_empty() {
        advance_mutation_generation(&PRODUCTION_ZERO_MUTATION_GENERATION)
            .map_err(|_| "A18 production zero mutation generation exhausted before drain")?;
    }
    let events = queue.pending.drain(..).collect::<Vec<_>>();
    Ok(ProductionZeroDrain {
        mutation_generation: PRODUCTION_ZERO_MUTATION_GENERATION.load(Ordering::Acquire),
        events,
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
    })
}

pub fn peek_witnesses() -> Result<WitnessReservation, String> {
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned")?;
    let mutation_generation = witness_mutation_generation()?;
    let (reservation_id, witnesses) = reserve_pending(&mut queue, "witness", mutation_generation)?;
    Ok(WitnessReservation {
        reservation_id,
        mutation_generation,
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        diagnostics: diagnostics(witnesses.len() as u64),
        witnesses,
    })
}

pub fn commit_witnesses(reservation_id: u64) -> Result<(), String> {
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned")?;
    commit_reservation(
        &mut queue,
        reservation_id,
        "witness",
        &WITNESS_MUTATION_GENERATION,
    )
}

pub fn rollback_witnesses(reservation_id: u64) -> Result<(), String> {
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned")?;
    rollback_reservation(&mut queue, reservation_id, "witness")
}

pub fn peek_production_zero_removals() -> Result<ProductionZeroReservation, String> {
    let mut queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned")?;
    let mutation_generation = production_zero_mutation_generation()?;
    let (reservation_id, events) =
        reserve_pending(&mut queue, "production zero", mutation_generation)?;
    Ok(ProductionZeroReservation {
        reservation_id,
        mutation_generation,
        drain: ProductionZeroDrain {
            mutation_generation,
            events,
            completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
        },
    })
}

pub fn commit_production_zero_removals(reservation_id: u64) -> Result<(), String> {
    let mut queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned")?;
    commit_reservation(
        &mut queue,
        reservation_id,
        "production zero",
        &PRODUCTION_ZERO_MUTATION_GENERATION,
    )
}

pub fn rollback_production_zero_removals(reservation_id: u64) -> Result<(), String> {
    let mut queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned")?;
    rollback_reservation(&mut queue, reservation_id, "production zero")
}

fn reserve_pending<T: Clone>(
    queue: &mut ReservableQueue<T>,
    label: &str,
    mutation_generation: u64,
) -> Result<(u64, Vec<T>), String> {
    if let Some(reservation) = queue.reservation {
        return Err(format!(
            "A18 {label} queue already has reservation {} (reserved={}, pending={})",
            reservation.id,
            reservation.row_count,
            queue.pending_count()
        ));
    }
    let reservation_id = next_reservation_id(label)?;
    let row_count = queue.pending.len().min(MAX_PENDING_WITNESSES);
    queue.reservation = Some(QueueReservation {
        id: reservation_id,
        row_count,
        owner: ReservationOwner::Delivery,
        mutation_generation,
    });
    Ok((
        reservation_id,
        queue.pending.iter().take(row_count).cloned().collect(),
    ))
}

fn next_reservation_id(label: &str) -> Result<u64, String> {
    NEXT_RESERVATION_ID
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1).filter(|next| *next != 0)
        })
        .map_err(|_| format!("A18 {label} reservation id exhausted"))
}

fn commit_reservation<T>(
    queue: &mut ReservableQueue<T>,
    reservation_id: u64,
    label: &str,
    mutation_generation: &AtomicU64,
) -> Result<(), String> {
    let reservation = matching_reservation(queue, reservation_id, label)?;
    if queue.pending.len() < reservation.row_count {
        return Err(format!(
            "A18 {label} reservation {} lost its prefix (reserved={}, retained={})",
            reservation.id,
            reservation.row_count,
            queue.pending.len()
        ));
    }
    if reservation.row_count != 0 {
        advance_mutation_generation(mutation_generation)
            .map_err(|_| format!("A18 {label} mutation generation exhausted before commit"))?;
    }
    queue.pending.drain(..reservation.row_count);
    queue.reservation = None;
    Ok(())
}

fn rollback_reservation<T>(
    queue: &mut ReservableQueue<T>,
    reservation_id: u64,
    label: &str,
) -> Result<(), String> {
    matching_reservation(queue, reservation_id, label)?;
    queue.reservation = None;
    Ok(())
}

fn matching_reservation<T>(
    queue: &ReservableQueue<T>,
    reservation_id: u64,
    label: &str,
) -> Result<QueueReservation, String> {
    match queue.reservation {
        Some(reservation)
            if reservation.id == reservation_id
                && reservation.owner == ReservationOwner::Delivery =>
        {
            Ok(reservation)
        }
        Some(reservation) if reservation.owner == ReservationOwner::Teardown => Err(format!(
            "A18 {label} queue is owned by release-pending teardown"
        )),
        Some(reservation) => Err(format!(
            "A18 {label} reservation mismatch: active={}, requested={reservation_id}",
            reservation.id
        )),
        None => Err(format!(
            "A18 {label} queue has no active reservation {reservation_id}"
        )),
    }
}

fn reject_live_reservation<T>(label: &str, queue: &ReservableQueue<T>) -> Result<(), String> {
    if let Some(reservation) = queue.reservation {
        return Err(format!(
            "A18 {label} queue has live reservation {} (reserved={}, pending={})",
            reservation.id,
            reservation.row_count,
            queue.pending_count()
        ));
    }
    Ok(())
}

pub fn receipt_from_drain(drain: HookDrain) -> LifecycleWitnessReceipt {
    let HookDrain {
        hook_epoch,
        witnesses,
        diagnostics,
        ..
    } = drain;
    LifecycleWitnessReceipt {
        schema: "hoi4.a18.dynamic_modifier_lifecycle_witness.v1".into(),
        executable_sha256: EXECUTABLE_SHA256.into(),
        exact20_job_id: ZERO_REMOVAL_JOB_ID.into(),
        exact20_manifest_sha256: ZERO_REMOVAL_MANIFEST_SHA256.into(),
        update_function_rva: DAILY_UPDATE_RVA as u32,
        erase_function_rva: ENTRY_ERASE_RVA as u32,
        process_identity: current_process_identity(),
        evidence_install: true,
        hook_epoch,
        witnesses,
        diagnostics,
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
    if PRODUCTION_MODE.load(Ordering::Acquire) || RELEASE_PENDING.load(Ordering::Acquire) {
        return Err(
            "A18 production teardown requires seal_reserve_and_uninstall followed by \
             post-sink commit_sealed_terminal or rollback_sealed_terminal"
                .into(),
        );
    }
    if !INSTALLED.load(Ordering::Acquire) {
        return drain();
    }
    let reservation = seal_reserve_and_uninstall()?;
    let reservation_id = reservation.reservation_id;
    if let Err(error) = commit_sealed_terminal(reservation_id) {
        let rollback = rollback_sealed_terminal(reservation_id)
            .err()
            .map(|rollback| format!("; terminal rollback failed: {rollback}"))
            .unwrap_or_default();
        return Err(format!(
            "A18 fixture compatibility teardown commit failed: {error}{rollback}"
        ));
    }
    Ok(reservation.drain)
}

pub fn seal_reserve_and_uninstall() -> Result<SealedTerminalReservation, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A18 teardown lock poisoned")?;
    {
        let terminal = SEALED_TERMINAL
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "A18 sealed terminal mutex poisoned")?;
        if let Some(active) = terminal.as_ref() {
            return Err(format!(
                "A18 sealed terminal reservation {} is already active",
                active.reservation_id
            ));
        }
    }
    if !INSTALLED.load(Ordering::Acquire) && !RELEASE_PENDING.load(Ordering::Acquire) {
        return Err("A18 cannot seal terminal queues without live or release-pending hooks".into());
    }
    if !RELEASE_PENDING.load(Ordering::Acquire) {
        if hook_receipts_have_installed()? {
            installed_hook_receipts()?;
        }
        begin_release_pending()?;
    }
    let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
    disable(sites.daily_update)?;
    wait_for_callbacks()?;
    disable(sites.entry_erase)?;
    wait_for_callbacks()?;
    RELEASE_CALLBACKS_QUIESCED.store(true, Ordering::Release);
    remove_release_targets(sites)?;
    let released = observe_release_receipts_if_owned(sites)?;
    stage_sealed_terminal(released)
}

/// it. This is the sole transition that publishes released-hook receipts and
pub fn commit_sealed_terminal(reservation_id: u64) -> Result<(), String> {
    consume_sealed_terminal(reservation_id, "commit").map(|_| ())
}

/// finish, and consumes the release custody its removed hooks left behind.
pub fn abort_sealed_terminal(reservation_id: u64) -> Result<InstallAbortDiscard, String> {
    consume_sealed_terminal(reservation_id, "install abort")
}

/// Seals, releases and discards the whole D67 carrier for a production hook
/// carrier and its release receipts in teardown custody for a later retry.
pub fn abort_partial_install_and_uninstall() -> Result<InstallAbortDiscard, String> {
    let reservation = seal_reserve_and_uninstall()?;
    let reservation_id = reservation.reservation_id;
    match abort_sealed_terminal(reservation_id) {
        Ok(discard) => Ok(discard),
        Err(error) => Err(match rollback_sealed_terminal(reservation_id) {
            Ok(()) => error,
            Err(rollback) => format!("{error}; terminal rollback failed: {rollback}"),
        }),
    }
}

/// a relevant timed instance leaves it at zero), and `admit_hook_release`
/// zero-observation release whose discard is non-empty -- so a benign counter
fn install_abort_discard_for_terminal(
    drain: &HookDrain,
    witness_pending: usize,
    production_zero_pending: usize,
) -> InstallAbortDiscard {
    InstallAbortDiscard {
        retained_rows: (witness_pending as u64).saturating_add(production_zero_pending as u64),
        reserved_rows: (drain.witnesses.len() as u64)
            .saturating_add(drain.production_zero_removals.len() as u64),
        discarded_diagnostics: drain
            .diagnostics
            .dropped_reentrant
            .saturating_add(drain.diagnostics.dropped_unreadable)
            .saturating_add(drain.diagnostics.dropped_lock_busy)
            .saturating_add(drain.diagnostics.dropped_overflow),
        poisoned: drain.diagnostics.completeness_poisoned,
    }
}

pub fn inapplicable_daily_update_count() -> u64 {
    IGNORED_WITHOUT_TIMED.load(Ordering::Acquire)
}

fn consume_sealed_terminal(
    reservation_id: u64,
    label: &str,
) -> Result<InstallAbortDiscard, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A18 teardown lock poisoned")?;
    if !RELEASE_PENDING.load(Ordering::Acquire) {
        return Err(format!(
            "A18 sealed terminal {label} requires release-pending custody"
        ));
    }
    let terminal_mutex = SEALED_TERMINAL.get_or_init(|| Mutex::new(None));
    let mut terminal_slot = terminal_mutex
        .lock()
        .map_err(|_| "A18 sealed terminal mutex poisoned")?;
    let terminal = match terminal_slot.as_ref() {
        Some(terminal) if terminal.reservation_id == reservation_id => terminal.clone(),
        Some(terminal) => {
            return Err(format!(
                "A18 sealed terminal reservation mismatch: active={}, requested={reservation_id}",
                terminal.reservation_id
            ))
        }
        None => {
            return Err(format!(
                "A18 sealed terminal reservation {reservation_id} is not active"
            ))
        }
    };

    let receipts_mutex = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let mut receipts = receipts_mutex
        .lock()
        .map_err(|_| "D67 production hook receipt mutex poisoned at terminal commit")?;
    if terminal.released.is_some() != receipts.installed.is_some() {
        return Err("D67 released-hook custody does not match installed-hook custody".into());
    }
    if receipts.released.is_some() {
        return Err("D67 prior released-hook receipts were not consumed".into());
    }

    let mut witness_queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned at terminal commit")?;
    let mut zero_queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned at terminal commit")?;
    verify_terminal_queue(
        "witness",
        &witness_queue,
        &terminal.drain.witnesses,
        terminal.drain.witness_mutation_generation,
        &WITNESS_MUTATION_GENERATION,
    )?;
    verify_terminal_queue(
        "production zero",
        &zero_queue,
        &terminal.drain.production_zero_removals,
        terminal.drain.production_zero_mutation_generation,
        &PRODUCTION_ZERO_MUTATION_GENERATION,
    )?;

    if !witness_queue.pending.is_empty()
        && WITNESS_MUTATION_GENERATION.load(Ordering::Acquire) >= u64::MAX - 1
    {
        return Err("A18 witness mutation generation exhausted at terminal commit".into());
    }
    if !zero_queue.pending.is_empty()
        && PRODUCTION_ZERO_MUTATION_GENERATION.load(Ordering::Acquire) >= u64::MAX - 1
    {
        return Err("A18 production zero mutation generation exhausted at terminal commit".into());
    }
    if !witness_queue.pending.is_empty() {
        advance_mutation_generation(&WITNESS_MUTATION_GENERATION)
            .map_err(|_| "A18 witness mutation generation exhausted at terminal commit")?;
    }
    if !zero_queue.pending.is_empty() {
        advance_mutation_generation(&PRODUCTION_ZERO_MUTATION_GENERATION)
            .map_err(|_| "A18 production zero mutation generation exhausted at terminal commit")?;
    }

    let discard = install_abort_discard_for_terminal(
        &terminal.drain,
        witness_queue.pending.len(),
        zero_queue.pending.len(),
    );
    witness_queue.pending.clear();
    witness_queue.reservation = None;
    zero_queue.pending.clear();
    zero_queue.reservation = None;
    receipts.installing = [None; 2];
    receipts.installed = None;
    receipts.releasing = [None; 2];
    receipts.released = terminal.released;
    *terminal_slot = None;
    RELEASE_PENDING.store(false, Ordering::Release);
    drop(zero_queue);
    drop(witness_queue);
    drop(receipts);
    drop(terminal_slot);
    clear_hook_state();
    Ok(discard)
}

/// terminal queues and partial release receipts remain in teardown custody,
pub fn rollback_sealed_terminal(reservation_id: u64) -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A18 teardown lock poisoned")?;
    if !RELEASE_PENDING.load(Ordering::Acquire) {
        return Err("A18 sealed terminal rollback requires release-pending custody".into());
    }
    let mut terminal = SEALED_TERMINAL
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "A18 sealed terminal mutex poisoned")?;
    match terminal.as_ref() {
        Some(active) if active.reservation_id == reservation_id => {
            *terminal = None;
            Ok(())
        }
        Some(active) => Err(format!(
            "A18 sealed terminal reservation mismatch: active={}, requested={reservation_id}",
            active.reservation_id
        )),
        None => Err(format!(
            "A18 sealed terminal reservation {reservation_id} is not active"
        )),
    }
}

fn begin_release_pending() -> Result<(), String> {
    let witness_id = next_reservation_id("teardown witness")?;
    let zero_id = next_reservation_id("teardown production zero")?;
    let mut witness_queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned before teardown")?;
    let mut zero_queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned before teardown")?;
    if witness_queue.reservation.is_some() || zero_queue.reservation.is_some() {
        return Err(format!(
            "A18 teardown refused before disabling hooks: witness reserved={}, pending={}; \
             production zero reserved={}, pending={}",
            witness_queue.reserved_count(),
            witness_queue.pending_count(),
            zero_queue.reserved_count(),
            zero_queue.pending_count()
        ));
    }
    witness_queue.reservation = Some(QueueReservation {
        id: witness_id,
        row_count: witness_queue.pending.len(),
        owner: ReservationOwner::Teardown,
        mutation_generation: WITNESS_MUTATION_GENERATION.load(Ordering::Acquire),
    });
    zero_queue.reservation = Some(QueueReservation {
        id: zero_id,
        row_count: zero_queue.pending.len(),
        owner: ReservationOwner::Teardown,
        mutation_generation: PRODUCTION_ZERO_MUTATION_GENERATION.load(Ordering::Acquire),
    });
    let mut removed_mask = 0usize;
    let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
    if verify_sentinel(sites.daily_update, &DAILY_UPDATE_SENTINEL).is_ok() {
        removed_mask |= 1;
    }
    if verify_sentinel(sites.entry_erase, &ENTRY_ERASE_SENTINEL).is_ok() {
        removed_mask |= 2;
    }
    RELEASE_REMOVED_MASK.store(removed_mask, Ordering::Release);
    RELEASE_CALLBACKS_QUIESCED.store(false, Ordering::Release);
    RELEASE_PENDING.store(true, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    Ok(())
}

fn remove_release_targets(sites: InstalledHookAddresses) -> Result<(), String> {
    // latches `RELEASE_CALLBACKS_QUIESCED` and only then calls this. The drain
    // DRAINED-BY-CALLER: `seal_reserve_and_uninstall` disables the daily-update
    // site, waits for callbacks, disables the entry-erase site, waits again,
    // latches `RELEASE_CALLBACKS_QUIESCED` and only then calls this. The drain
    // is per site and interleaved with the disables on purpose -- each site's
    // bodies are drained while the other is still admitting -- so it cannot be
    // restated here as one wait over both.
    let mut failures = Vec::new();
    // Both release targets are removed under one freeze instead of two, in the
    let removed = unsafe { inline_detour::remove_batch(&hook_batch(sites)) };
    for (index, (name, outcome)) in removed.outcomes().iter().enumerate() {
        let bit = 1usize << index;
        match outcome {
            Ok(())
                if removed.was_already_released(name)
                    && RELEASE_REMOVED_MASK.load(Ordering::Acquire) & bit == 0 =>
            {
                failures.push(format!(
                    "{name}: hook was not created and this custody has no prior removal"
                ));
            }
            Ok(()) => {
                RELEASE_REMOVED_MASK.fetch_or(bit, Ordering::AcqRel);
            }
            Err(status) => failures.push(format!("{name}: {status}")),
        }
    }
    if failures.is_empty() && RELEASE_REMOVED_MASK.load(Ordering::Acquire) == 0b11 {
        Ok(())
    } else {
        Err(format!(
            "A18 hook release remains pending (removed mask {:02b}): {}",
            RELEASE_REMOVED_MASK.load(Ordering::Acquire),
            failures.join("; ")
        ))
    }
}

fn stage_sealed_terminal(
    released: Option<[ReleasedRvaHookReceipt; 2]>,
) -> Result<SealedTerminalReservation, String> {
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let receipts = receipts
        .lock()
        .map_err(|_| "D67 production hook receipt mutex poisoned at terminal seal")?;
    if released.is_some() != receipts.installed.is_some() {
        return Err("D67 released-hook custody does not match installed-hook custody".into());
    }
    if receipts.released.is_some() {
        return Err("D67 prior released-hook receipts were not consumed".into());
    }
    let mut witness_queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned at terminal seal")?;
    let mut zero_queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned at terminal seal")?;
    for (label, queue) in [
        ("witness", witness_queue.reservation),
        ("production zero", zero_queue.reservation),
    ] {
        match queue {
            Some(reservation) if reservation.owner == ReservationOwner::Teardown => {}
            _ => {
                return Err(format!(
                    "A18 {label} queue lost its release-pending teardown reservation"
                ))
            }
        }
    }

    let witness_count = witness_queue.pending.len();
    let zero_count = zero_queue.pending.len();
    if let Some(reservation) = witness_queue.reservation.as_mut() {
        reservation.row_count = witness_count;
        reservation.mutation_generation = WITNESS_MUTATION_GENERATION.load(Ordering::Acquire);
    }
    if let Some(reservation) = zero_queue.reservation.as_mut() {
        reservation.row_count = zero_count;
        reservation.mutation_generation =
            PRODUCTION_ZERO_MUTATION_GENERATION.load(Ordering::Acquire);
    }
    let witnesses = witness_queue.pending.iter().cloned().collect::<Vec<_>>();
    let production_zero_removals = zero_queue.pending.iter().cloned().collect::<Vec<_>>();
    let drain = HookDrain {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        witness_mutation_generation: WITNESS_MUTATION_GENERATION.load(Ordering::Acquire),
        production_zero_mutation_generation: PRODUCTION_ZERO_MUTATION_GENERATION
            .load(Ordering::Acquire),
        diagnostics: diagnostics(witnesses.len() as u64),
        witnesses,
        production_zero_removals,
    };

    let reservation_id = next_reservation_id("sealed terminal")?;
    let mut terminal = SEALED_TERMINAL
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "A18 sealed terminal mutex poisoned")?;
    if let Some(active) = terminal.as_ref() {
        return Err(format!(
            "A18 sealed terminal reservation {} is already active",
            active.reservation_id
        ));
    }
    *terminal = Some(SealedTerminalState {
        reservation_id,
        drain: drain.clone(),
        released,
    });
    Ok(SealedTerminalReservation {
        reservation_id,
        drain,
    })
}

fn verify_terminal_queue<T: PartialEq>(
    label: &str,
    queue: &ReservableQueue<T>,
    expected: &[T],
    expected_generation: u64,
    mutation_generation: &AtomicU64,
) -> Result<(), String> {
    let reservation = queue
        .reservation
        .ok_or_else(|| format!("A18 {label} queue lost terminal teardown custody"))?;
    if reservation.owner != ReservationOwner::Teardown {
        return Err(format!("A18 {label} queue terminal custody changed owner"));
    }
    if reservation.row_count != expected.len()
        || queue.pending.len() != expected.len()
        || !queue.pending.iter().eq(expected.iter())
    {
        return Err(format!(
            "A18 {label} queue changed under sealed terminal reservation"
        ));
    }
    let observed_generation = mutation_generation.load(Ordering::Acquire);
    if reservation.mutation_generation != expected_generation
        || observed_generation != expected_generation
    {
        return Err(format!(
            "A18 {label} queue generation changed under sealed terminal reservation: \
             expected={expected_generation}, reserved={}, observed={observed_generation}",
            reservation.mutation_generation
        ));
    }
    Ok(())
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        daily_update: base + DAILY_UPDATE_RVA,
        entry_erase: base + ENTRY_ERASE_RVA,
    }
}

fn reset_state(epoch: u64) -> Result<(), String> {
    let witness_queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 witness queue lock poisoned before install")?;
    let zero_queue = PRODUCTION_ZERO_QUEUE
        .get_or_init(|| Mutex::new(ReservableQueue::new()))
        .lock()
        .map_err(|_| "A18 production zero queue lock poisoned before install")?;
    if !witness_queue.pending.is_empty()
        || witness_queue.reservation.is_some()
        || !zero_queue.pending.is_empty()
        || zero_queue.reservation.is_some()
    {
        return Err(format!(
            "A18 reinstall refused with retained queue state: witness reserved={}, pending={}; \
             production zero reserved={}, pending={}",
            witness_queue.reserved_count(),
            witness_queue.pending_count(),
            zero_queue.reserved_count(),
            zero_queue.pending_count()
        ));
    }
    drop(zero_queue);
    drop(witness_queue);
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
    let owner_tag_db_id = match classify_country_container(container) {
        Ok(CountryContainerClass::Exact { owner_tag_db_id }) => owner_tag_db_id,
        Ok(CountryContainerClass::Unrelated) => return unsafe { original(container) },
        Err(()) => {
            poison(&DROPPED_UNREADABLE);
            return unsafe { original(container) };
        }
    };
    let nested = OPEN_UPDATE.with(|slot| slot.borrow().is_some());
    if nested {
        poison(&DROPPED_REENTRANT);
        return unsafe { original(container) };
    }
    let before_hour = current_game_hour_raw();
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

    let (
        Some(game_hour_raw_before),
        Some(game_hour_raw_after),
        Some((owner_before, before)),
        Some((owner_after, after)),
        Some(open),
    ) = (before_hour, after_hour, before, after, open)
    else {
        poison(&DROPPED_UNREADABLE);
        return result;
    };
    if owner_before != owner_tag_db_id || owner_before != owner_after || open.poisoned {
        poison(&DROPPED_UNREADABLE);
        return result;
    }
    let relevant = before.iter().any(|row| row.remaining_days.is_some())
        || after.iter().any(|row| row.remaining_days.is_some())
        || open.erase_boundaries.iter().any(|boundary| {
            boundary
                .rows_before_erase
                .iter()
                .any(|row| row.remaining_days == Some(0))
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
    let witness = DailyUpdateWitness {
        sequence,
        thread_id: unsafe { GetCurrentThreadId() },
        owner_tag_db_id: owner_before,
        game_hour_raw_before,
        game_hour_raw_after,
        before,
        erase_boundaries: open.erase_boundaries,
        after,
    };
    if PRODUCTION_MODE.load(Ordering::Acquire) {
        retain_production_zero_removals(&witness);
    }
    if EVIDENCE_MODE.load(Ordering::Acquire) {
        retain(witness);
    }
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
    let queue = QUEUE.get_or_init(|| Mutex::new(ReservableQueue::new()));
    match queue.try_lock() {
        Ok(mut queue) if queue.pending.len() < MAX_PENDING_WITNESSES => {
            if advance_mutation_generation(&WITNESS_MUTATION_GENERATION).is_ok() {
                queue.pending.push_back(witness);
            } else {
                poison(&DROPPED_OVERFLOW);
            }
        }
        Ok(_) => poison(&DROPPED_OVERFLOW),
        Err(_) => poison(&DROPPED_LOCK_BUSY),
    }
}

fn retain_production_zero_removals(witness: &DailyUpdateWitness) {
    if witness.game_hour_raw_before != witness.game_hour_raw_after
        || !witness.validates_exact_daily_transform()
    {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let events = witness
        .erase_boundaries
        .iter()
        .flat_map(|boundary| boundary.rows_before_erase.iter())
        .filter(|row| row.remaining_days == Some(0))
        .cloned()
        .map(|row| ProductionZeroRemoval {
            sequence: witness.sequence,
            owner_tag_db_id: witness.owner_tag_db_id,
            observed_game_hour_raw: witness.game_hour_raw_after,
            row,
        })
        .collect::<Vec<_>>();
    if events.is_empty() {
        return;
    }
    let queue = PRODUCTION_ZERO_QUEUE.get_or_init(|| Mutex::new(ReservableQueue::new()));
    match queue.try_lock() {
        Ok(mut queue)
            if queue.pending.len().saturating_add(events.len()) <= MAX_PENDING_WITNESSES =>
        {
            if advance_mutation_generation(&PRODUCTION_ZERO_MUTATION_GENERATION).is_ok() {
                queue.pending.extend(events);
            } else {
                poison(&DROPPED_OVERFLOW);
            }
        }
        Ok(_) => poison(&DROPPED_OVERFLOW),
        Err(_) => poison(&DROPPED_LOCK_BUSY),
    }
}

fn poison(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::AcqRel);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
    if EVIDENCE_MODE.load(Ordering::Acquire) {
        let _ = advance_mutation_generation(&WITNESS_MUTATION_GENERATION);
    }
    if PRODUCTION_MODE.load(Ordering::Acquire) {
        let _ = advance_mutation_generation(&PRODUCTION_ZERO_MUTATION_GENERATION);
    }
}

fn advance_mutation_generation(generation: &AtomicU64) -> Result<u64, ()> {
    generation
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(1)
                .filter(|next| *next != 0 && *next != u64::MAX)
        })
        .map(|previous| previous + 1)
        .map_err(|_| ())
}

fn snapshot_container(container: usize) -> Option<(u32, Vec<DynamicModifierWitnessRow>)> {
    let owner_tag_db_id = match classify_country_container(container).ok()? {
        CountryContainerClass::Exact { owner_tag_db_id } => owner_tag_db_id,
        CountryContainerClass::Unrelated => return None,
    };
    let vector = container.checked_add(CONTAINER_ENTRY_VECTOR_OFF)?;
    let count = read_stable::<u32>(container.checked_add(CONTAINER_COUNT_OFF)?)?;
    snapshot_vector_range(vector, 0, count).map(|rows| (owner_tag_db_id, rows))
}

fn classify_country_container(container: usize) -> Result<CountryContainerClass, ()> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let container_vtable = read_stable::<usize>(container).ok_or(())?;
    if Some(container_vtable) != base.checked_add(CONTAINER_VTABLE_RVA) {
        return Ok(CountryContainerClass::Unrelated);
    }
    let Some(owner) = container.checked_sub(COUNTRY_DYNAMIC_MODIFIER_CONTAINER_OFF) else {
        return Ok(CountryContainerClass::Unrelated);
    };
    let backref =
        read_stable::<usize>(container.checked_add(CONTAINER_BACKREF_OFF).ok_or(())?).ok_or(())?;
    if backref != owner {
        return Ok(CountryContainerClass::Unrelated);
    }
    let owner_vtable = read_stable::<usize>(owner).ok_or(())?;
    if Some(owner_vtable) != base.checked_add(COUNTRY_VTABLE_RVA) {
        return Ok(CountryContainerClass::Unrelated);
    }
    let owner_tag_db_id = read_stable::<u32>(owner.checked_add(COUNTRY_TAG_DB_ID_OFF).ok_or(())?)
        .filter(|tag_db_id| *tag_db_id != 0)
        .ok_or(())?;
    Ok(CountryContainerClass::Exact { owner_tag_db_id })
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
        return if start == 0 && end == 0 {
            Some(Vec::new())
        } else {
            None
        };
    }
    if !heap_pointer(data) {
        return None;
    }
    let mut rows = Vec::with_capacity((end - start) as usize);
    for index in start..end {
        let entry = data.checked_add(index as usize * ENTRY_STRIDE)?;
        rows.push(read_entry(entry)?);
    }
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
        raw[ENTRY_DEFINITION_OFF..ENTRY_DEFINITION_OFF + 8]
            .try_into()
            .ok()?,
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
        raw[ENTRY_VALUES_BEGIN_OFF..ENTRY_VALUES_BEGIN_OFF + 8]
            .try_into()
            .ok()?,
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
    for range in [
        0..8,
        ENTRY_DEFINITION_OFF..ENTRY_DEFINITION_OFF + 8,
        ENTRY_VALUES_BEGIN_OFF..ENTRY_VALUES_BEGIN_OFF + 8,
    ] {
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
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return None;
    }
    Some(text.to_owned())
}

fn current_game_hour_raw() -> Option<i32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let game_state = read_stable::<usize>(base.checked_add(CGAMESTATE_SINGLETON_RVA)?)?;
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

fn verify_all_sentinels(sites: InstalledHookAddresses) -> Result<(), String> {
    verify_sentinel(sites.daily_update, &DAILY_UPDATE_SENTINEL)?;
    verify_sentinel(sites.entry_erase, &ENTRY_ERASE_SENTINEL)
}

fn observe_install_receipts(sites: InstalledHookAddresses) -> Result<(), String> {
    let targets = [sites.daily_update, sites.entry_erase];
    let detours = [hook_daily_update as usize, hook_entry_erase as usize];
    let receipt_state = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    {
        let mut receipts = receipt_state
            .lock()
            .map_err(|_| "D67 production hook receipt mutex poisoned")?;
        receipts.installing = [None; 2];
        receipts.releasing = [None; 2];
    }
    for (index, ((expected, target), detour)) in PRODUCTION_HOOK_SPECS
        .into_iter()
        .zip(targets)
        .zip(detours)
        .enumerate()
    {
        let observed =
            observe_enabled_minhook_rel5(expected, target, detour, next_teardown_owner_token()?)?;
        receipt_state
            .lock()
            .map_err(|_| "D67 production hook receipt mutex poisoned")?
            .installing[index] = Some(observed);
    }
    let mut receipts = receipt_state
        .lock()
        .map_err(|_| "D67 production hook receipt mutex poisoned")?;
    let observed = [
        receipts.installing[0].ok_or("D67 daily-update install receipt is absent")?,
        receipts.installing[1].ok_or("D67 entry-erase install receipt is absent")?,
    ];
    receipts.installing = [None; 2];
    receipts.installed = Some(observed);
    receipts.releasing = [None; 2];
    receipts.released = None;
    Ok(())
}

fn hook_receipts_have_installed() -> Result<bool, String> {
    Ok(HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "D67 production hook receipt mutex poisoned")?
        .installed
        .is_some())
}

fn prepare_hook_receipts_for_install() -> Result<(), String> {
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let receipts = receipts
        .lock()
        .map_err(|_| "D67 production hook receipt mutex poisoned before install")?;
    if receipts.released.is_some() {
        return Err("D67 untaken released-hook receipts block reinstall".into());
    }
    if receipts.installing.iter().any(Option::is_some)
        || receipts.releasing.iter().any(Option::is_some)
    {
        return Err("D67 partial hook receipt custody blocks reinstall".into());
    }
    if receipts.installed.is_some() {
        return Err("D67 prior installed-hook custody blocks reinstall".into());
    }
    RELEASE_REMOVED_MASK.store(0, Ordering::Release);
    RELEASE_CALLBACKS_QUIESCED.store(false, Ordering::Release);
    Ok(())
}

fn observe_release_receipts_if_owned(
    sites: InstalledHookAddresses,
) -> Result<Option<[ReleasedRvaHookReceipt; 2]>, String> {
    if !RELEASE_CALLBACKS_QUIESCED.load(Ordering::Acquire)
        || RELEASE_REMOVED_MASK.load(Ordering::Acquire) != 0b11
    {
        return Err("D67 release attestation is incomplete".into());
    }
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let mut receipts = receipts
        .lock()
        .map_err(|_| "D67 production hook receipt mutex poisoned")?;
    let Some(installed) = receipts.installed else {
        for (index, expected) in PRODUCTION_HOOK_SPECS.into_iter().enumerate() {
            let Some(partial) = receipts.installing[index] else {
                continue;
            };
            observe_released(expected, partial, completed_teardown_attestation())?;
            receipts.installing[index] = None;
        }
        verify_all_sentinels(sites)?;
        return Ok(None);
    };
    for (index, (expected, installed)) in
        PRODUCTION_HOOK_SPECS.into_iter().zip(installed).enumerate()
    {
        if receipts.releasing[index].is_none() {
            receipts.releasing[index] = Some(observe_released(
                expected,
                installed,
                completed_teardown_attestation(),
            )?);
        }
    }
    let released = [
        receipts.releasing[0].ok_or("D67 daily-update release receipt is absent")?,
        receipts.releasing[1].ok_or("D67 entry-erase release receipt is absent")?,
    ];
    Ok(Some(released))
}

fn completed_teardown_attestation() -> HookTeardownAttestation {
    HookTeardownAttestation {
        callbacks_quiesced: RELEASE_CALLBACKS_QUIESCED.load(Ordering::Acquire),
        hook_removed: RELEASE_REMOVED_MASK.load(Ordering::Acquire) == 0b11,
        relay_freed: RELEASE_REMOVED_MASK.load(Ordering::Acquire) == 0b11,
    }
}

fn disable(target: usize) -> Result<(), String> {
    match unsafe { inline_detour::disable(target as *mut c_void) } {
        Ok(())
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => Ok(()),
        Err(status) => Err(format!("A18 hook disable failed: {status}")),
    }
}

fn cleanup_after_failed_install(sites: InstalledHookAddresses) -> Result<(), String> {
    let daily_disable = disable(sites.daily_update);
    let erase_disable = disable(sites.entry_erase);
    daily_disable?;
    erase_disable?;
    wait_for_callbacks()?;
    let mut batch = hook_batch(sites);
    batch.reverse();
    if let Some((name, status)) = unsafe { inline_detour::remove_batch(&batch) }.first_failure() {
        return Err(format!("A18 rollback remove failed for {name}: {status}"));
    }
    verify_all_sentinels(sites)?;
    RELEASE_CALLBACKS_QUIESCED.store(true, Ordering::Release);
    RELEASE_REMOVED_MASK.store(0b11, Ordering::Release);
    observe_release_receipts_if_owned(sites)?;
    RELEASE_CALLBACKS_QUIESCED.store(false, Ordering::Release);
    RELEASE_REMOVED_MASK.store(0, Ordering::Release);
    Ok(())
}

fn clear_hook_state() {
    ORIGINAL_DAILY_UPDATE.store(0, Ordering::Release);
    ORIGINAL_ENTRY_ERASE.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    EVIDENCE_MODE.store(false, Ordering::Release);
    PRODUCTION_MODE.store(false, Ordering::Release);
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
        assert_eq!(
            PRODUCTION_HOOK_SPECS.map(|spec| spec.logical_name),
            [
                "gameplay.d67.daily_modifier_update",
                "gameplay.d67.modifier_entry_erase",
            ]
        );
        assert!(PRODUCTION_HOOK_SPECS
            .iter()
            .all(|spec| spec.target_patch_len == 5
                && spec.primitive_id == MINHOOK_NORMAL_REL5_PRIMITIVE_ID));
    }

    #[test]
    fn production_latches_remain_closed() {
        assert!(crate::PRODUCTION_DMI2_SCHEDULING_ENABLED);
        assert!(!crate::LIVE_FIXTURE_ACCEPTED);
    }

    fn test_witness(sequence: u64) -> crate::DailyUpdateWitness {
        crate::DailyUpdateWitness {
            sequence,
            thread_id: 1,
            owner_tag_db_id: 2,
            game_hour_raw_before: 3,
            game_hour_raw_after: 4,
            before: Vec::new(),
            erase_boundaries: Vec::new(),
            after: Vec::new(),
        }
    }

    fn test_zero_removal(sequence: u64) -> crate::ProductionZeroRemoval {
        crate::ProductionZeroRemoval {
            sequence,
            owner_tag_db_id: 2,
            observed_game_hour_raw: 3,
            row: crate::DynamicModifierWitnessRow {
                modifier_id: "test".into(),
                enabled: true,
                values_raw: Vec::new(),
                remaining_days: None,
                pointer_masked_entry_bytes: Vec::new(),
            },
        }
    }

    #[test]
    fn install_abort_counts_zero_removals_as_discarded_rows_not_diagnostics() {
        let drain = HookDrain {
            witnesses: vec![test_witness(1), test_witness(2)],
            production_zero_removals: vec![
                test_zero_removal(3),
                test_zero_removal(4),
                test_zero_removal(5),
            ],
            diagnostics: crate::LifecycleWitnessDiagnostics {
                retained_updates: 9,
                ignored_without_timed_instance: 1,
                dropped_reentrant: 2,
                dropped_unreadable: 3,
                dropped_lock_busy: 4,
                dropped_overflow: 5,
                completeness_poisoned: true,
            },
            ..HookDrain::default()
        };

        let discard = install_abort_discard_for_terminal(&drain, 2, 3);

        assert_eq!(discard.retained_rows, 5);
        assert_eq!(discard.reserved_rows, 5);
        assert_eq!(discard.discarded_diagnostics, 14);
        assert!(discard.poisoned);
    }

    /// `admit_hook_release` refuses a
    /// zero-observation release whose discard is non-empty -- so a lane that
    #[test]
    fn an_inapplicable_daily_update_is_a_census_not_a_discarded_diagnostic() {
        let mut drain = HookDrain {
            diagnostics: crate::LifecycleWitnessDiagnostics {
                ignored_without_timed_instance: 250_000,
                ..crate::LifecycleWitnessDiagnostics::default()
            },
            ..HookDrain::default()
        };

        let discard = install_abort_discard_for_terminal(&drain, 0, 0);
        assert_eq!(discard.discarded_diagnostics, 0);
        assert!(
            discard.is_empty(),
            "a window that only skipped inapplicable updates discarded nothing, got {discard:?}"
        );

        drain.diagnostics.dropped_overflow = 1;
        assert_eq!(
            install_abort_discard_for_terminal(&drain, 0, 0).discarded_diagnostics,
            1
        );
    }

    #[test]
    fn reservation_commit_removes_only_the_reserved_prefix() {
        let mut queue = ReservableQueue::new();
        let generation = AtomicU64::new(1);
        queue.pending.extend([1u32, 2]);
        let (reservation_id, reserved) = reserve_pending(&mut queue, "test", 1).unwrap();
        assert_eq!(reserved, vec![1, 2]);

        queue.pending.push_back(3);
        assert!(reserve_pending(&mut queue, "test", 2).is_err());
        commit_reservation(&mut queue, reservation_id, "test", &generation).unwrap();
        assert_eq!(queue.pending.into_iter().collect::<Vec<_>>(), vec![3]);
        assert_eq!(generation.load(Ordering::Acquire), 2);
    }

    #[test]
    fn empty_queue_still_produces_a_fenceable_reservation() {
        let mut queue = ReservableQueue::<u32>::new();
        let (reservation_id, reserved) = reserve_pending(&mut queue, "test", 7).unwrap();
        assert!(reserved.is_empty());
        assert_eq!(queue.reservation.unwrap().mutation_generation, 7);
        rollback_reservation(&mut queue, reservation_id, "test").unwrap();
        assert!(queue.reservation.is_none());
    }

    #[test]
    fn reservation_rollback_preserves_order_and_concurrent_append() {
        let mut queue = ReservableQueue::new();
        let generation = AtomicU64::new(1);
        queue.pending.extend([4u32, 5]);
        let (reservation_id, reserved) = reserve_pending(&mut queue, "test", 1).unwrap();
        assert_eq!(reserved, vec![4, 5]);
        queue.pending.push_back(6);

        assert!(commit_reservation(
            &mut queue,
            reservation_id.wrapping_add(1),
            "test",
            &generation
        )
        .is_err());
        rollback_reservation(&mut queue, reservation_id, "test").unwrap();
        assert_eq!(queue.pending.into_iter().collect::<Vec<_>>(), vec![4, 5, 6]);
    }

    /// listed below: the install and release latches, the release attestation
    #[test]
    fn production_mode_refuses_the_fixture_teardown_shim_without_touching_state() {
        fn snapshot() -> Vec<(&'static str, u64)> {
            let (witness_pending, witness_reserved) = QUEUE
                .get_or_init(|| Mutex::new(ReservableQueue::new()))
                .lock()
                .map(|queue| (queue.pending_count() as u64, queue.reserved_count() as u64))
                .unwrap_or((u64::MAX, u64::MAX));
            let (zero_pending, zero_reserved) = PRODUCTION_ZERO_QUEUE
                .get_or_init(|| Mutex::new(ReservableQueue::new()))
                .lock()
                .map(|queue| (queue.pending_count() as u64, queue.reserved_count() as u64))
                .unwrap_or((u64::MAX, u64::MAX));
            let sealed = SEALED_TERMINAL
                .get_or_init(|| Mutex::new(None))
                .lock()
                .map(|slot| slot.as_ref().map_or(0, |state| state.reservation_id))
                .unwrap_or(u64::MAX);
            vec![
                ("installed", INSTALLED.load(Ordering::Acquire) as u64),
                (
                    "release_pending",
                    RELEASE_PENDING.load(Ordering::Acquire) as u64,
                ),
                (
                    "release_removed_mask",
                    RELEASE_REMOVED_MASK.load(Ordering::Acquire) as u64,
                ),
                (
                    "release_callbacks_quiesced",
                    RELEASE_CALLBACKS_QUIESCED.load(Ordering::Acquire) as u64,
                ),
                (
                    "active_hook_epoch",
                    ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
                ),
                ("module_base", MODULE_BASE.load(Ordering::Acquire) as u64),
                (
                    "witness_generation",
                    WITNESS_MUTATION_GENERATION.load(Ordering::Acquire),
                ),
                (
                    "zero_generation",
                    PRODUCTION_ZERO_MUTATION_GENERATION.load(Ordering::Acquire),
                ),
                ("witness_pending", witness_pending),
                ("witness_reserved", witness_reserved),
                ("zero_pending", zero_pending),
                ("zero_reserved", zero_reserved),
                ("sealed_terminal", sealed),
            ]
        }

        let production_before = PRODUCTION_MODE.load(Ordering::Acquire);
        let before = snapshot();
        PRODUCTION_MODE.store(true, Ordering::Release);

        let first = seal_drain_and_uninstall();
        let second = seal_drain_and_uninstall();
        let after = snapshot();
        PRODUCTION_MODE.store(production_before, Ordering::Release);

        let first = first.expect_err(
            "a production install owes the two-phase terminal transaction, not the shim",
        );
        let second = second.expect_err("the refusal is not one-shot");
        assert!(
            first.contains("seal_reserve_and_uninstall")
                && first.contains("commit_sealed_terminal")
                && first.contains("rollback_sealed_terminal"),
            "the refusal must name the path it is redirecting to, got: {first}"
        );
        assert_eq!(
            first, second,
            "the refusal returns before any state transition, so retrying it \
             can only ever reproduce it"
        );
        assert_eq!(
            before, after,
            "the shim must refuse before touching any teardown-owned state"
        );
    }

    #[test]
    fn mutation_generation_never_wraps_or_admits_exhaustion() {
        let generation = AtomicU64::new(u64::MAX - 1);
        assert!(advance_mutation_generation(&generation).is_err());
        assert_eq!(generation.load(Ordering::Acquire), u64::MAX - 1);
    }
}
