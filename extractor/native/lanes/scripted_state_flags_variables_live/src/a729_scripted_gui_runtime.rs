use crate::ScriptedGuiRuntimeFrame;
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use runtime_text_inputs_live_lane::a729_hook::{
    install_a729_runtime_text_hooks, register_pre_setter_observer, register_sprite_setter_observer,
    unregister_pre_setter_observer, unregister_sprite_setter_observer,
};
use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const OWNER_RVA: u32 = 0x00B7_9C90;
pub const UPDATER_RVA: u32 = 0x00B7_9820;
pub const PRIMARY_TEXT_RETURN_RVA: u32 = 0x00B7_C3CA;
pub const SPRITE_RETURN_RVA: u32 = 0x00B7_CAC1;
pub const INVALID_REBUILD_GENERATION: u32 = u32::MAX;
pub const PRODUCTION_ACTIVATION_AUTHORIZED: bool = true;

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const CGAMESTATE_CURRENT_DATE_OFF: usize = 0x468;
const MANAGER_VECTOR: usize = 0x20;
const MANAGER_COUNT: usize = 0x2C;
const MANAGER_GENERATION: usize = 0x68;
const UPDATER_LIVE_WINDOW: usize = 0xF8;
const UPDATER_DEFINITION: usize = 0xE8;
const UPDATER_CONTEXT: usize = 0x3C0;
const CONTEXT_COUNTRY_TAG_DATABASE_ID: usize = 0xA8;
const UPDATER_ROOT_VISIBLE: usize = 0xB79;
const COUNTRY_DATABASE_RVA: usize = 0x0331_89C8;
const MAX_COUNTRY_TAG_TABLE_CAPACITY: usize = i32::MAX as usize;
const MAX_COUNTRY_TAG_DB_ID: usize = (i32::MAX as usize) - 1;
const COUNTRY_TAG_TABLE_AUTHENTICATION_ROWS: usize = 4;
const EFFECT_WIDGET_VECTOR: usize = 0x990;
const EFFECT_WIDGET_COUNT: usize = 0x99C;
const VISIBLE_WIDGET_PRIMARY_VECTOR: usize = 0x9A8;
const VISIBLE_WIDGET_PRIMARY_COUNT: usize = 0x9B4;
const VISIBLE_WIDGET_SECONDARY_VECTOR: usize = 0x9C0;
const VISIBLE_WIDGET_SECONDARY_COUNT: usize = 0x9CC;
const RESOLVED_PROPERTY_VECTOR: usize = 0xA50;
const RESOLVED_PROPERTY_COUNT: usize = 0xA5C;
const RESOLVED_PROPERTY_STRIDE: usize = 0x88;
const WIDGET_HIDDEN_STATE: usize = 0x75;
const WIDGET_NAME_VTABLE_SLOT: usize = 0xA8;
const WIDGET_ENABLED_VTABLE_SLOT: usize = 0x278;
const MAX_UPDATERS: usize = 65_536;
const MAX_VALUES_PER_UPDATER: usize = 131_072;
const MAX_RECEIPTS: usize = 64;
const CALLBACK_QUIESCE_TIMEOUT: Duration = Duration::from_secs(30);

const OWNER_SENTINEL: [u8; 20] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
    0x48, 0x83, 0xec, 0x30,
];
const UPDATER_SENTINEL: [u8; 20] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x81, 0xec, 0x20,
    0x01, 0x00, 0x00, 0x44,
];

type OwnerFn = unsafe extern "system" fn(usize);
type UpdaterFn = unsafe extern "system" fn(usize, u8);

static ORIGINAL_OWNER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_UPDATER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static EXPECTED_GAME_HOUR_RAW: AtomicI32 = AtomicI32::new(i32::MIN);
static CLAIMED_GAME_HOUR_RAW: AtomicI32 = AtomicI32::new(i32::MIN);
static NEXT_PRODUCER_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static REJECTED_TRANSACTIONS: AtomicU64 = AtomicU64::new(0);
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());
static COMPILER: OnceLock<Mutex<Option<Arc<dyn ExactA729A38TransactionCompiler>>>> =
    OnceLock::new();
static RECEIPTS: OnceLock<Mutex<ReceiptQueueState>> = OnceLock::new();

thread_local! {
    static ACTIVE_OWNER: RefCell<Option<ActiveOwner>> = const { RefCell::new(None) };
    static ACTIVE_UPDATER: RefCell<Option<ProcessLocalA38UpdaterObservation>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum A38MaterializedValueKind {
    PrimaryText,
    Sprite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalA38MaterializedValue {
    pub kind: A38MaterializedValueKind,
    pub widget_ptr: usize,
    pub caller_rva: u32,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalA38WidgetObservation {
    pub widget_ptr: usize,
    pub name: String,
    pub visible: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalA38PropertyBinding {
    pub target_ptr: usize,
    pub authored_element: String,
    pub authored_property: String,
    pub resolved_element: String,
    pub resolved_property: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalA38UpdaterObservation {
    pub updater_ptr: usize,
    pub definition_id: Option<String>,
    pub country_tag: Option<String>,
    pub live_window_before: usize,
    pub live_window_after: usize,
    pub root_visible: Option<bool>,
    pub effect_widgets: Vec<ProcessLocalA38WidgetObservation>,
    pub visible_widgets: Vec<ProcessLocalA38WidgetObservation>,
    pub property_bindings: Vec<ProcessLocalA38PropertyBinding>,
    pub values: Vec<ProcessLocalA38MaterializedValue>,
    pub rejected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalA38OwnerTransaction {
    pub owner_rva: u32,
    pub manager_ptr: usize,
    pub owner_thread_id: u32,
    pub game_hour_raw: i32,
    pub rebuild_generation: u32,
    pub producer_sequence: u64,
    pub updater_order: Vec<ProcessLocalA38UpdaterObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedA38OwnerReceipt {
    pub process_start_nonce: u64,
    pub game_hour_raw: i32,
    pub hour_index: u64,
    pub producer_sequence: u64,
    pub producer_record_identity: String,
    pub completed_owner_rva: u32,
    pub rebuild_generation: u32,
    pub manager_order: Vec<(String, String)>,
    pub frame: ScriptedGuiRuntimeFrame,
}

#[derive(Debug, PartialEq, Eq)]
pub struct A38OwnerReceiptReservation {
    pub reservation_id: u64,
    pub receipt: AuthenticatedA38OwnerReceipt,
}

pub trait ExactA729A38TransactionCompiler: Send + Sync {
    fn identity_catalog_epoch(&self) -> u64;
    fn compile_transaction(
        &self,
        transaction: &ProcessLocalA38OwnerTransaction,
    ) -> Result<AuthenticatedA38OwnerReceipt, String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub owner: usize,
    pub updater: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub installed: bool,
    pub queued_receipts: u64,
    pub active_reservation_id: Option<u64>,
    pub rejected_transactions: u64,
    pub next_producer_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReceiptReservationState {
    reservation_id: u64,
    hour_index: u64,
    producer_sequence: u64,
    producer_record_identity: String,
}

struct ReceiptQueueState {
    receipts: VecDeque<AuthenticatedA38OwnerReceipt>,
    reservation: Option<ReceiptReservationState>,
    next_reservation_id: u64,
}

impl Default for ReceiptQueueState {
    fn default() -> Self {
        Self {
            receipts: VecDeque::new(),
            reservation: None,
            next_reservation_id: 1,
        }
    }
}

struct ActiveOwner {
    manager_ptr: usize,
    owner_thread_id: u32,
    game_hour_raw: i32,
    producer_sequence: u64,
    updaters: Vec<ProcessLocalA38UpdaterObservation>,
    rejected: bool,
}

struct InFlightGuard;
impl InFlightGuard {
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn register_transaction_compiler(
    compiler: Arc<dyn ExactA729A38TransactionCompiler>,
) -> Result<(), String> {
    if compiler.identity_catalog_epoch() == 0 {
        return Err("A38 compiler identity catalog epoch is zero".into());
    }
    let slot = COMPILER.get_or_init(|| Mutex::new(None));
    let mut slot = slot.lock().map_err(|_| "A38 compiler mutex poisoned")?;
    if slot.is_some() {
        return Err("A38 transaction compiler is already registered".into());
    }
    *slot = Some(compiler);
    Ok(())
}

pub fn publish_expected_game_hour_raw(game_hour_raw: i32) {
    EXPECTED_GAME_HOUR_RAW.store(game_hour_raw, Ordering::Release);
}

pub fn arm_capture(game_hour_raw: i32) -> Result<(), String> {
    if game_hour_raw < 0 || !INSTALLED.load(Ordering::Acquire) {
        return Err("A38 capture arm has no installed exact owner or valid hour".into());
    }
    let queue = RECEIPTS.get_or_init(|| Mutex::new(ReceiptQueueState::default()));
    if !queue
        .lock()
        .map_err(|_| "A38 receipt queue poisoned")?
        .receipts
        .is_empty()
    {
        return Err("A38 capture arm found an undrained prior receipt".into());
    }
    if CLAIMED_GAME_HOUR_RAW.load(Ordering::Acquire) != i32::MIN {
        return Err("A38 prior capture owner is still active".into());
    }
    EXPECTED_GAME_HOUR_RAW
        .compare_exchange(i32::MIN, game_hour_raw, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "A38 capture is already armed")?;
    Ok(())
}

pub fn cancel_capture(game_hour_raw: i32) {
    if EXPECTED_GAME_HOUR_RAW
        .compare_exchange(game_hour_raw, i32::MIN, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let _ = CLAIMED_GAME_HOUR_RAW.compare_exchange(
            game_hour_raw,
            i32::MIN,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

pub fn install() -> Result<InstalledAddresses, String> {
    if !PRODUCTION_ACTIVATION_AUTHORIZED {
        return Err("A38 production activation is compile-time disabled".into());
    }
    install_impl()
}

pub fn install_for_fixture() -> Result<InstalledAddresses, String> {
    install_impl()
}

fn install_impl() -> Result<InstalledAddresses, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "A38 hook lifecycle mutex poisoned".to_string())?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    if MODULE_BASE.load(Ordering::Acquire) != 0 {
        return Err("A38 prior hook set has not completed quiescent removal".into());
    }
    if compiler().is_err() {
        return Err("A38 collector-owned transaction compiler is unavailable".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A38 GetModuleHandleW(NULL) returned null".into());
    }
    let resolved = addresses(base);
    verify_sentinel(resolved.owner, &OWNER_SENTINEL, "owner")?;
    verify_sentinel(resolved.updater, &UPDATER_SENTINEL, "updater")?;
    MODULE_BASE.store(base, Ordering::Release);
    install_a729_runtime_text_hooks().map_err(|error| {
        MODULE_BASE.store(0, Ordering::Release);
        format!("A38 shared A25 hook install failed: {error}")
    })?;
    register_pre_setter_observer(a38_text_observer).map_err(|error| {
        MODULE_BASE.store(0, Ordering::Release);
        format!("A38 shared text observer registration failed: {error}")
    })?;
    if let Err(error) = register_sprite_setter_observer(a38_sprite_observer) {
        unregister_pre_setter_observer(a38_text_observer);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!(
            "A38 shared sprite observer registration failed: {error}"
        ));
    }
    RECEIPTS.get_or_init(|| Mutex::new(ReceiptQueueState::default()));
    NEXT_PRODUCER_SEQUENCE.store(1, Ordering::Release);
    REJECTED_TRANSACTIONS.store(0, Ordering::Release);
    let owner_original =
        unsafe { MinHook::create_hook(resolved.owner as *mut c_void, hook_owner as *mut c_void) }
            .map_err(|status| {
            unregister_a38_observers();
            MODULE_BASE.store(0, Ordering::Release);
            format!("A38 owner hook creation failed: {status}")
        })?;
    let updater_original = match unsafe {
        MinHook::create_hook(resolved.updater as *mut c_void, hook_updater as *mut c_void)
    } {
        Ok(value) => value,
        Err(status) => {
            let removed = unsafe { inline_detour::remove(resolved.owner as *mut c_void) };
            unregister_a38_observers();
            if removed.is_ok()
                || matches!(
                    removed,
                    Err(InlineDetourReleaseError::MinHook(
                        MH_STATUS::MH_ERROR_NOT_CREATED
                    ))
                )
            {
                MODULE_BASE.store(0, Ordering::Release);
                ORIGINAL_OWNER.store(0, Ordering::Release);
            }
            return Err(format!("A38 updater hook creation failed: {status}"));
        }
    };
    ORIGINAL_OWNER.store(owner_original as usize, Ordering::Release);
    ORIGINAL_UPDATER.store(updater_original as usize, Ordering::Release);
    for target in [resolved.updater, resolved.owner] {
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            for cleanup in [resolved.owner, resolved.updater] {
                let _ = unsafe { inline_detour::disable(cleanup as *mut c_void) };
            }
            wait_for_in_flight_callbacks()?;
            for cleanup in [resolved.updater, resolved.owner] {
                let _ = unsafe { inline_detour::remove(cleanup as *mut c_void) };
            }
            unregister_a38_observers();
            clear_hook_state();
            return Err(format!("A38 hook enable failed at 0x{target:X}: {status}"));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(resolved)
}

pub fn reserve_receipt(hour_index: u64) -> Result<Option<A38OwnerReceiptReservation>, String> {
    // callback has published and released its claim. This closes the small
    if EXPECTED_GAME_HOUR_RAW.load(Ordering::Acquire) != i32::MIN
        || CLAIMED_GAME_HOUR_RAW.load(Ordering::Acquire) != i32::MIN
    {
        return Ok(None);
    }
    let queue = RECEIPTS.get_or_init(|| Mutex::new(ReceiptQueueState::default()));
    let mut queue = queue.lock().map_err(|_| "A38 receipt queue poisoned")?;
    if let Some(active) = &queue.reservation {
        return Err(format!(
            "A38 receipt reservation {} is already active",
            active.reservation_id
        ));
    }
    let matches = queue
        .receipts
        .iter()
        .enumerate()
        .filter_map(|(index, receipt)| (receipt.hour_index == hour_index).then_some(index))
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err("A38 receipt queue has multiple transactions for the requested hour".into());
    }
    let Some(index) = matches.first().copied() else {
        return Ok(None);
    };
    let receipt = queue
        .receipts
        .get(index)
        .cloned()
        .ok_or("A38 receipt reservation index disappeared")?;
    let reservation_id = queue.next_reservation_id.max(1);
    queue.next_reservation_id = reservation_id.checked_add(1).unwrap_or(1);
    queue.reservation = Some(ReceiptReservationState {
        reservation_id,
        hour_index: receipt.hour_index,
        producer_sequence: receipt.producer_sequence,
        producer_record_identity: receipt.producer_record_identity.clone(),
    });
    Ok(Some(A38OwnerReceiptReservation {
        reservation_id,
        receipt,
    }))
}

pub fn commit_receipt(reservation_id: u64) -> Result<(), String> {
    let queue = RECEIPTS.get_or_init(|| Mutex::new(ReceiptQueueState::default()));
    let mut queue = queue.lock().map_err(|_| "A38 receipt queue poisoned")?;
    let reservation = queue
        .reservation
        .clone()
        .ok_or("A38 receipt commit has no active reservation")?;
    if reservation.reservation_id != reservation_id {
        return Err(format!(
            "A38 receipt reservation mismatch: active {}, supplied {reservation_id}",
            reservation.reservation_id
        ));
    }
    let same_hour_count = queue
        .receipts
        .iter()
        .filter(|receipt| receipt.hour_index == reservation.hour_index)
        .count();
    if same_hour_count != 1 {
        return Err(format!(
            "A38 reserved hour gained {same_hour_count} queued transactions before commit"
        ));
    }
    let index = queue
        .receipts
        .iter()
        .position(|receipt| {
            receipt.hour_index == reservation.hour_index
                && receipt.producer_sequence == reservation.producer_sequence
                && receipt.producer_record_identity == reservation.producer_record_identity
        })
        .ok_or("A38 reserved receipt disappeared before commit")?;
    queue.receipts.remove(index);
    queue.reservation = None;
    Ok(())
}

/// Release custody without consuming the queued receipt. A subsequent frame
pub fn rollback_receipt(reservation_id: u64) -> Result<(), String> {
    let queue = RECEIPTS.get_or_init(|| Mutex::new(ReceiptQueueState::default()));
    let mut queue = queue.lock().map_err(|_| "A38 receipt queue poisoned")?;
    let reservation = queue
        .reservation
        .as_ref()
        .ok_or("A38 receipt rollback has no active reservation")?;
    if reservation.reservation_id != reservation_id {
        return Err(format!(
            "A38 receipt reservation mismatch: active {}, supplied {reservation_id}",
            reservation.reservation_id
        ));
    }
    queue.reservation = None;
    Ok(())
}

pub fn diagnostics() -> Diagnostics {
    let queue = RECEIPTS.get().and_then(|queue| queue.lock().ok());
    Diagnostics {
        installed: INSTALLED.load(Ordering::Acquire),
        queued_receipts: queue
            .as_ref()
            .map_or(0, |queue| queue.receipts.len() as u64),
        active_reservation_id: queue
            .as_ref()
            .and_then(|queue| queue.reservation.as_ref())
            .map(|reservation| reservation.reservation_id),
        rejected_transactions: REJECTED_TRANSACTIONS.load(Ordering::Acquire),
        next_producer_sequence: NEXT_PRODUCER_SEQUENCE.load(Ordering::Acquire),
    }
}

pub fn uninstall() -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "A38 hook lifecycle mutex poisoned".to_string())?;
    if !INSTALLED.load(Ordering::Acquire) && MODULE_BASE.load(Ordering::Acquire) == 0 {
        EXPECTED_GAME_HOUR_RAW.store(i32::MIN, Ordering::Release);
        CLAIMED_GAME_HOUR_RAW.store(i32::MIN, Ordering::Release);
        unregister_a38_observers();
        return Ok(());
    }
    let resolved = addresses(MODULE_BASE.load(Ordering::Acquire));
    for target in [resolved.owner, resolved.updater] {
        match unsafe { inline_detour::disable(target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => return Err(format!("A38 hook disable failed: {status}")),
        }
    }
    unregister_a38_observers();
    wait_for_in_flight_callbacks()?;
    for target in [resolved.owner, resolved.updater] {
        match unsafe { inline_detour::remove(target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => return Err(format!("A38 hook removal failed: {status}")),
        }
    }
    clear_hook_state();
    Ok(())
}

fn unregister_a38_observers() {
    unregister_pre_setter_observer(a38_text_observer);
    unregister_sprite_setter_observer(a38_sprite_observer);
}

fn wait_for_in_flight_callbacks() -> Result<(), String> {
    let started = Instant::now();
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if started.elapsed() >= CALLBACK_QUIESCE_TIMEOUT {
            return Err("A38 hook teardown timed out waiting for callbacks".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

unsafe extern "system" fn hook_owner(manager_ptr: usize) {
    let _in_flight = InFlightGuard::enter();
    let nested = ACTIVE_OWNER.with(|slot| slot.borrow().is_some());
    let before_hour = observed_game_hour_raw();
    let claimed_hour = (!nested && manager_ptr != 0)
        .then(|| before_hour.ok())
        .flatten()
        .filter(|game_hour_raw| claim_capture_owner(*game_hour_raw));
    if claimed_hour.is_none() {
        reject();
    } else {
        let sequence = NEXT_PRODUCER_SEQUENCE.fetch_add(1, Ordering::AcqRel).max(1);
        ACTIVE_OWNER.with(|slot| {
            *slot.borrow_mut() = Some(ActiveOwner {
                manager_ptr,
                owner_thread_id: unsafe { GetCurrentThreadId() },
                game_hour_raw: claimed_hour.unwrap_or(i32::MIN),
                producer_sequence: sequence,
                updaters: Vec::new(),
                rejected: false,
            });
        });
    }
    let original = ORIGINAL_OWNER.load(Ordering::Acquire);
    if original != 0 {
        let original: OwnerFn = unsafe { mem::transmute(original) };
        unsafe { original(manager_ptr) };
    }
    if !nested {
        let active = ACTIVE_OWNER.with(|slot| slot.borrow_mut().take());
        if let Some(active) = active {
            let game_hour_raw = active.game_hour_raw;
            if original == 0 || seal_owner_transaction(active).is_err() {
                release_capture_owner_claim(game_hour_raw);
                reject();
            }
        }
    }
}

unsafe extern "system" fn hook_updater(updater_ptr: usize, force: u8) {
    let _in_flight = InFlightGuard::enter();
    let owner_active = ACTIVE_OWNER.with(|slot| slot.borrow().is_some());
    let nested = ACTIVE_UPDATER.with(|slot| slot.borrow().is_some());
    if owner_active && !nested && updater_ptr != 0 {
        let live_window_before =
            read_usize(updater_ptr.saturating_add(UPDATER_LIVE_WINDOW)).unwrap_or(0);
        ACTIVE_UPDATER.with(|slot| {
            *slot.borrow_mut() = Some(ProcessLocalA38UpdaterObservation {
                updater_ptr,
                definition_id: None,
                country_tag: None,
                live_window_before,
                live_window_after: 0,
                root_visible: None,
                effect_widgets: Vec::new(),
                visible_widgets: Vec::new(),
                property_bindings: Vec::new(),
                values: Vec::new(),
                rejected: false,
            });
        });
    } else if owner_active {
        poison_active_owner();
    }
    let original = ORIGINAL_UPDATER.load(Ordering::Acquire);
    if original != 0 {
        let original: UpdaterFn = unsafe { mem::transmute(original) };
        unsafe { original(updater_ptr, force) };
    }
    if owner_active && !nested && updater_ptr != 0 {
        let observation = ACTIVE_UPDATER.with(|slot| slot.borrow_mut().take());
        if let Some(mut observation) = observation {
            observation.live_window_after =
                read_usize(updater_ptr.saturating_add(UPDATER_LIVE_WINDOW)).unwrap_or(0);
            observation.root_visible = read_u8(updater_ptr.saturating_add(UPDATER_ROOT_VISIBLE))
                .and_then(|value| (value <= 1).then_some(value != 0));
            if populate_completed_updater_observation(&mut observation).is_err()
                || original == 0
                || observation.values.len() > MAX_VALUES_PER_UPDATER
            {
                observation.rejected = true;
            }
            ACTIVE_OWNER.with(|slot| {
                if let Some(owner) = slot.borrow_mut().as_mut() {
                    if owner.updaters.len() >= MAX_UPDATERS {
                        owner.rejected = true;
                    } else {
                        owner.updaters.push(observation);
                    }
                }
            });
        } else {
            poison_active_owner();
        }
    }
}

unsafe extern "system" fn a38_text_observer(
    _owner: usize,
    widget: usize,
    text_object: usize,
    caller_rva: u32,
) {
    if caller_rva != PRIMARY_TEXT_RETURN_RVA {
        return;
    }
    record_value(
        A38MaterializedValueKind::PrimaryText,
        widget,
        text_object,
        caller_rva,
    );
}

unsafe extern "system" fn a38_sprite_observer(
    widget: usize,
    string_object: usize,
    _mode: u8,
    caller_rva: u32,
) {
    if caller_rva != SPRITE_RETURN_RVA {
        return;
    }
    record_value(
        A38MaterializedValueKind::Sprite,
        widget,
        string_object,
        caller_rva,
    );
}

fn record_value(
    kind: A38MaterializedValueKind,
    widget: usize,
    string_object: usize,
    caller_rva: u32,
) {
    let value = read_msvc_string(string_object);
    ACTIVE_UPDATER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(updater) = slot.as_mut() else {
            return;
        };
        match value {
            Ok(value) if updater.values.len() < MAX_VALUES_PER_UPDATER => {
                updater.values.push(ProcessLocalA38MaterializedValue {
                    kind,
                    widget_ptr: widget,
                    caller_rva,
                    value,
                });
            }
            _ => updater.rejected = true,
        }
    });
}

fn populate_completed_updater_observation(
    observation: &mut ProcessLocalA38UpdaterObservation,
) -> Result<(), String> {
    let updater = observation.updater_ptr;
    let definition = read_usize(
        updater
            .checked_add(UPDATER_DEFINITION)
            .ok_or("A38 definition field overflow")?,
    )
    .filter(|value| *value != 0)
    .ok_or("A38 updater definition is unavailable")?;
    observation.definition_id = Some(read_stable_msvc_string(
        definition
            .checked_add(0x08)
            .ok_or("A38 definition id overflow")?,
    )?);
    observation.country_tag = Some(read_country_tag(updater)?);
    observation.effect_widgets = read_widget_vector(
        updater,
        EFFECT_WIDGET_VECTOR,
        EFFECT_WIDGET_COUNT,
        "effect-enabled",
    )?;
    let mut visible_widgets = read_widget_vector(
        updater,
        VISIBLE_WIDGET_PRIMARY_VECTOR,
        VISIBLE_WIDGET_PRIMARY_COUNT,
        "primary-visible",
    )?;
    visible_widgets.extend(read_widget_vector(
        updater,
        VISIBLE_WIDGET_SECONDARY_VECTOR,
        VISIBLE_WIDGET_SECONDARY_COUNT,
        "secondary-visible",
    )?);
    if visible_widgets.len() > MAX_VALUES_PER_UPDATER {
        return Err("A38 visible widget inventory exceeds bound".into());
    }
    observation.visible_widgets = visible_widgets;
    observation.property_bindings = read_property_bindings(updater)?;
    if observation.live_window_after == 0 || observation.root_visible.is_none() {
        return Err("A38 updater returned without a live window/result".into());
    }
    Ok(())
}

fn read_country_tag(updater: usize) -> Result<String, String> {
    let country_id = read_u32(
        updater
            .checked_add(UPDATER_CONTEXT + CONTEXT_COUNTRY_TAG_DATABASE_ID)
            .ok_or("A38 country context overflow")?,
    )
    .and_then(|value| usize::try_from(value).ok())
    .filter(|value| *value <= MAX_COUNTRY_TAG_DB_ID)
    .ok_or("A38 updater country database id is invalid")?;
    let base = MODULE_BASE.load(Ordering::Acquire);
    let database = read_usize(
        base.checked_add(COUNTRY_DATABASE_RVA)
            .ok_or("A38 country database slot overflow")?,
    )
    .filter(|value| *value != 0)
    .ok_or("A38 country database is unavailable")?;
    let table = read_usize(database)
        .filter(|value| *value != 0)
        .ok_or("A38 country tag table is unavailable")?;
    let capacity = read_u32(database.saturating_add(8))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value <= MAX_COUNTRY_TAG_TABLE_CAPACITY)
        .ok_or("A38 country tag table capacity is invalid")?;
    let active_count = read_u32(database.saturating_add(12))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value != 0 && *value <= capacity)
        .ok_or("A38 active country tag count is invalid")?;
    for index in 0..COUNTRY_TAG_TABLE_AUTHENTICATION_ROWS.min(active_count) {
        let row = read_stable_msvc_string(table.saturating_add(index * 0x20))?;
        if row.len() != 3 || (index > 0 && !country_tag_grammar_ok(&row)) {
            return Err("A38 country tag table is not a tag vector".into());
        }
    }
    if country_id == 0 || country_id >= active_count {
        return Err("A38 updater country id is outside the tag table".into());
    }
    if read_usize(database).filter(|value| *value != 0) != Some(table)
        || read_u32(database.saturating_add(12)).and_then(|value| usize::try_from(value).ok())
            != Some(active_count)
    {
        return Err("A38 country tag table changed during the read".into());
    }
    let tag = read_stable_msvc_string(
        table
            .checked_add(country_id.saturating_mul(0x20))
            .ok_or("A38 country tag address overflow")?,
    )?;
    if tag.len() != 3 || !country_tag_grammar_ok(&tag) {
        return Err("A38 country tag is not canonical".into());
    }
    Ok(tag)
}

fn country_tag_grammar_ok(tag: &str) -> bool {
    tag.len() == 3
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn read_widget_vector(
    updater: usize,
    pointer_offset: usize,
    count_offset: usize,
    label: &str,
) -> Result<Vec<ProcessLocalA38WidgetObservation>, String> {
    let (pointer, count) = stable_vector_header(updater, pointer_offset, count_offset)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if pointer == 0 {
        return Err(format!("A38 {label} vector has null storage"));
    }
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let widget = read_usize(
            pointer
                .checked_add(index.saturating_mul(8))
                .ok_or_else(|| format!("A38 {label} widget address overflow"))?,
        )
        .filter(|value| *value != 0)
        .ok_or_else(|| format!("A38 {label} widget is unavailable"))?;
        rows.push(read_widget_observation(widget)?);
    }
    if stable_vector_header(updater, pointer_offset, count_offset)? != (pointer, count) {
        return Err(format!(
            "A38 {label} vector changed during owner observation"
        ));
    }
    Ok(rows)
}

fn stable_vector_header(
    owner: usize,
    pointer_offset: usize,
    count_offset: usize,
) -> Result<(usize, usize), String> {
    let read = || {
        let pointer = read_usize(owner.checked_add(pointer_offset)?)?;
        let count = read_u32(owner.checked_add(count_offset)?)? as usize;
        (count <= MAX_VALUES_PER_UPDATER).then_some((pointer, count))
    };
    let before = read().ok_or("A38 materialization vector header is invalid")?;
    let after = read().ok_or("A38 materialization vector header is invalid")?;
    if before != after {
        return Err("A38 materialization vector header is unstable".into());
    }
    Ok(before)
}

fn read_widget_observation(widget: usize) -> Result<ProcessLocalA38WidgetObservation, String> {
    let vtable = read_usize(widget)
        .filter(|value| module_address(*value))
        .ok_or("A38 widget vtable is unavailable")?;
    let name_getter = read_usize(vtable.saturating_add(WIDGET_NAME_VTABLE_SLOT))
        .filter(|value| module_address(*value))
        .ok_or("A38 widget name getter is unavailable")?;
    let enabled_getter = read_usize(vtable.saturating_add(WIDGET_ENABLED_VTABLE_SLOT))
        .filter(|value| module_address(*value))
        .ok_or("A38 widget enabled getter is unavailable")?;
    type NameGetter = unsafe extern "system" fn(usize) -> usize;
    type EnabledGetter = unsafe extern "system" fn(usize) -> i32;
    let name_source = unsafe {
        let name_getter: NameGetter = mem::transmute(name_getter);
        name_getter(widget)
    };
    let name = read_stable_msvc_string(name_source)?;
    if name.is_empty() {
        return Err("A38 widget name is empty".into());
    }
    let enabled = unsafe {
        let enabled_getter: EnabledGetter = mem::transmute(enabled_getter);
        enabled_getter(widget) != 3
    };
    let state = read_u8(widget.saturating_add(WIDGET_HIDDEN_STATE))
        .ok_or("A38 widget visibility state is unavailable")?;
    Ok(ProcessLocalA38WidgetObservation {
        widget_ptr: widget,
        name,
        visible: state & 0x10 == 0,
        enabled,
    })
}

fn read_property_bindings(updater: usize) -> Result<Vec<ProcessLocalA38PropertyBinding>, String> {
    let (pointer, count) =
        stable_vector_header(updater, RESOLVED_PROPERTY_VECTOR, RESOLVED_PROPERTY_COUNT)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if pointer == 0 {
        return Err("A38 property-binding vector has null storage".into());
    }
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let row = pointer
            .checked_add(index.saturating_mul(RESOLVED_PROPERTY_STRIDE))
            .ok_or("A38 property-binding row overflow")?;
        rows.push(ProcessLocalA38PropertyBinding {
            target_ptr: read_usize(row).unwrap_or(0),
            authored_element: read_stable_msvc_string(row.saturating_add(0x08))?,
            authored_property: read_stable_msvc_string(row.saturating_add(0x28))?,
            resolved_element: read_stable_msvc_string(row.saturating_add(0x48))?,
            resolved_property: read_stable_msvc_string(row.saturating_add(0x68))?,
        });
    }
    if stable_vector_header(updater, RESOLVED_PROPERTY_VECTOR, RESOLVED_PROPERTY_COUNT)?
        != (pointer, count)
    {
        return Err("A38 property-binding vector changed during owner observation".into());
    }
    Ok(rows)
}

fn read_stable_msvc_string(address: usize) -> Result<String, String> {
    let before = read_msvc_string(address)?;
    let after = read_msvc_string(address)?;
    if before != after {
        return Err("A38 MSVC string changed during owner observation".into());
    }
    Ok(before)
}

fn module_address(address: usize) -> bool {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 || address < base {
        return false;
    }
    let Some(pe_offset) = read_u32(base.saturating_add(0x3c)).map(|value| value as usize) else {
        return false;
    };
    let Some(image_size) =
        read_u32(base.saturating_add(pe_offset).saturating_add(0x50)).map(|value| value as usize)
    else {
        return false;
    };
    address < base.saturating_add(image_size)
}

fn seal_owner_transaction(active: ActiveOwner) -> Result<(), String> {
    if active.rejected || active.updaters.iter().any(|row| row.rejected) {
        return Err("A38 owner transaction was poisoned".into());
    }
    let after_hour = observed_game_hour_raw()?;
    if after_hour != active.game_hour_raw || after_hour < 0 {
        return Err("A38 owner transaction crossed engine hour".into());
    }
    let generation = read_u32(active.manager_ptr.saturating_add(MANAGER_GENERATION))
        .ok_or("A38 manager generation is unreadable")?;
    if generation == INVALID_REBUILD_GENERATION {
        return Err("A38 owner returned with rebuild sentinel generation".into());
    }
    let count = read_u32(active.manager_ptr.saturating_add(MANAGER_COUNT))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|count| *count <= MAX_UPDATERS)
        .ok_or("A38 manager count is invalid")?;
    let vector = read_usize(active.manager_ptr.saturating_add(MANAGER_VECTOR)).unwrap_or(0);
    if count != active.updaters.len() || (count != 0 && vector == 0) {
        return Err("A38 owner/updater coverage differs from post-owner manager order".into());
    }
    for (index, observed) in active.updaters.iter().enumerate() {
        let pointer = read_usize(vector.saturating_add(index.saturating_mul(8)))
            .ok_or("A38 post-owner updater pointer is unreadable")?;
        if pointer == 0 || pointer != observed.updater_ptr {
            return Err("A38 post-owner manager order differs from nested updater order".into());
        }
    }
    let transaction = ProcessLocalA38OwnerTransaction {
        owner_rva: OWNER_RVA,
        manager_ptr: active.manager_ptr,
        owner_thread_id: active.owner_thread_id,
        game_hour_raw: active.game_hour_raw,
        rebuild_generation: generation,
        producer_sequence: active.producer_sequence,
        updater_order: active.updaters,
    };
    let compiler = compiler()?;
    let epoch = compiler.identity_catalog_epoch();
    if epoch == 0 {
        return Err("A38 compiler identity epoch is zero".into());
    }
    let receipt = catch_unwind(AssertUnwindSafe(|| {
        compiler.compile_transaction(&transaction)
    }))
    .map_err(|_| "A38 transaction compiler panicked")??;
    if compiler.identity_catalog_epoch() != epoch {
        return Err("A38 identity catalog changed during transaction compilation".into());
    }
    regrade_receipt(&transaction, &receipt)?;
    EXPECTED_GAME_HOUR_RAW
        .compare_exchange(
            active.game_hour_raw,
            i32::MIN,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| "A38 capture arm changed before receipt publication")?;
    let queue = RECEIPTS.get_or_init(|| Mutex::new(ReceiptQueueState::default()));
    let mut queue = queue.lock().map_err(|_| "A38 receipt queue poisoned")?;
    if queue.receipts.len() >= MAX_RECEIPTS {
        return Err("A38 receipt queue capacity exhausted".into());
    }
    if queue
        .receipts
        .back()
        .is_some_and(|prior| prior.producer_sequence >= receipt.producer_sequence)
    {
        return Err("A38 receipt sequence is not strictly monotonic".into());
    }
    queue.receipts.push_back(receipt);
    CLAIMED_GAME_HOUR_RAW
        .compare_exchange(
            active.game_hour_raw,
            i32::MIN,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| "A38 capture owner claim changed before receipt publication")?;
    Ok(())
}

fn regrade_receipt(
    transaction: &ProcessLocalA38OwnerTransaction,
    receipt: &AuthenticatedA38OwnerReceipt,
) -> Result<(), String> {
    if receipt.process_start_nonce == 0
        || receipt.completed_owner_rva != OWNER_RVA
        || receipt.rebuild_generation != transaction.rebuild_generation
        || receipt.producer_sequence != transaction.producer_sequence
        || receipt.game_hour_raw != transaction.game_hour_raw
        || receipt.frame.hour_index != receipt.hour_index
        || receipt.frame.producer_record_identity != receipt.producer_record_identity
        || receipt.manager_order.len() != transaction.updater_order.len()
        || receipt.frame.rows.len() != receipt.manager_order.len()
    {
        return Err("A38 pointer-free receipt disagrees with its owner transaction".into());
    }
    let unique = receipt
        .manager_order
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if unique.len() != receipt.manager_order.len() {
        return Err("A38 receipt repeats a manager identity".into());
    }
    for (index, (row, identity)) in receipt
        .frame
        .rows
        .iter()
        .zip(receipt.manager_order.iter())
        .enumerate()
    {
        if row.manager_order_index as usize != index
            || (&row.country_tag, &row.gui_id) != (&identity.0, &identity.1)
        {
            return Err("A38 receipt row/order identity mismatch".into());
        }
    }
    if receipt.frame.expected_gui_keys != unique {
        return Err("A38 receipt frame coverage differs from manager order".into());
    }
    crate::validate_scripted_gui_runtime_frame(&receipt.frame)
        .map_err(|error| format!("A38 compiled frame failed carrier validation: {error:?}"))
}

fn compiler() -> Result<Arc<dyn ExactA729A38TransactionCompiler>, String> {
    COMPILER
        .get()
        .ok_or("A38 transaction compiler is unavailable")?
        .lock()
        .map_err(|_| "A38 compiler mutex poisoned")?
        .as_ref()
        .cloned()
        .ok_or_else(|| "A38 transaction compiler is unavailable".into())
}

fn observed_game_hour_raw() -> Result<i32, String> {
    let expected = EXPECTED_GAME_HOUR_RAW.load(Ordering::Acquire);
    if expected == i32::MIN {
        return Err("A38 collector game hour is unavailable".into());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let slot = base
        .checked_add(CGAMESTATE_SINGLETON_RVA)
        .ok_or("A38 game-state slot overflow")?;
    let state = read_usize(slot)
        .filter(|value| *value != 0)
        .ok_or("A38 game state unreadable")?;
    let observed = read_u32(
        state
            .checked_add(CGAMESTATE_CURRENT_DATE_OFF)
            .ok_or("A38 game-hour overflow")?,
    )
    .map(|value| value as i32)
    .ok_or("A38 game hour unreadable")?;
    if observed != expected
        || read_usize(slot) != Some(state)
        || read_u32(state.saturating_add(CGAMESTATE_CURRENT_DATE_OFF)).map(|value| value as i32)
            != Some(observed)
    {
        return Err("A38 game hour/state changed during observation".into());
    }
    Ok(observed)
}

fn claim_capture_owner(game_hour_raw: i32) -> bool {
    if CLAIMED_GAME_HOUR_RAW
        .compare_exchange(i32::MIN, game_hour_raw, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    if EXPECTED_GAME_HOUR_RAW.load(Ordering::Acquire) == game_hour_raw {
        true
    } else {
        let _ = CLAIMED_GAME_HOUR_RAW.compare_exchange(
            game_hour_raw,
            i32::MIN,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        false
    }
}

fn release_capture_owner_claim(game_hour_raw: i32) {
    let _ = EXPECTED_GAME_HOUR_RAW.compare_exchange(
        game_hour_raw,
        i32::MIN,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    let _ = CLAIMED_GAME_HOUR_RAW.compare_exchange(
        game_hour_raw,
        i32::MIN,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn poison_active_owner() {
    ACTIVE_OWNER.with(|slot| {
        if let Some(owner) = slot.borrow_mut().as_mut() {
            owner.rejected = true;
        }
    });
}

fn reject() {
    REJECTED_TRANSACTIONS.fetch_add(1, Ordering::Relaxed);
}

fn addresses(base: usize) -> InstalledAddresses {
    InstalledAddresses {
        owner: base.saturating_add(OWNER_RVA as usize),
        updater: base.saturating_add(UPDATER_RVA as usize),
    }
}

fn clear_hook_state() {
    INSTALLED.store(false, Ordering::Release);
    ORIGINAL_OWNER.store(0, Ordering::Release);
    ORIGINAL_UPDATER.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    EXPECTED_GAME_HOUR_RAW.store(i32::MIN, Ordering::Release);
    CLAIMED_GAME_HOUR_RAW.store(i32::MIN, Ordering::Release);
    ACTIVE_OWNER.with(|slot| *slot.borrow_mut() = None);
    ACTIVE_UPDATER.with(|slot| *slot.borrow_mut() = None);
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    if !read_exact(address, &mut actual) || actual != expected {
        return Err(format!("A38 {label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn read_exact(address: usize, out: &mut [u8]) -> bool {
    if address == 0 || out.is_empty() {
        return false;
    }
    let mut read = 0usize;
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            out.len(),
            &mut read,
        ) != 0
            && read == out.len()
    }
}

fn read_u8(address: usize) -> Option<u8> {
    let mut bytes = [0u8; 1];
    read_exact(address, &mut bytes).then_some(bytes[0])
}
fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes).then(|| u32::from_le_bytes(bytes))
}
fn read_u64(address: usize) -> Option<u64> {
    let mut bytes = [0u8; 8];
    read_exact(address, &mut bytes).then(|| u64::from_le_bytes(bytes))
}
fn read_usize(address: usize) -> Option<usize> {
    read_u64(address).and_then(|value| usize::try_from(value).ok())
}

fn read_msvc_string(address: usize) -> Result<String, String> {
    if address == 0 {
        return Err("A38 consumer supplied null string".into());
    }
    let len = read_u64(address.saturating_add(0x10))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("A38 consumer string length unreadable")?;
    let capacity = read_u64(address.saturating_add(0x18))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("A38 consumer string capacity unreadable")?;
    if len > 65_536 || capacity < len {
        return Err("A38 consumer string bounds invalid".into());
    }
    if len == 0 {
        return Ok(String::new());
    }
    let data = if capacity <= 15 {
        address
    } else {
        read_usize(address)
            .filter(|value| *value != 0)
            .ok_or("A38 consumer string pointer unreadable")?
    };
    let mut bytes = vec![0u8; len];
    if !read_exact(data, &mut bytes) {
        return Err("A38 consumer string bytes unreadable".into());
    }
    String::from_utf8(bytes).map_err(|_| "A38 consumer string is not UTF-8".into())
}
