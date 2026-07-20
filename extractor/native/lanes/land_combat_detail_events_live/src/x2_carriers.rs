//! Dormant exact-build carriers for the two remaining X2 live observations.
//!
//! Neither carrier is production-active. `WIDTH_CARRIER_ACTIVATION` stays
//! false until the two bracketing control calls in X2-WIDTH-RUNTIME have been
//! obtained; `CURRENT_LOSS_CARRIER_ACTIVATION` stays false until the exact
//! current-epoch loss writer and its two authoritative-hour control have been
//! obtained. The implemented width detours are therefore a sealed engineering
//! seam, not an inference-based reader. The loss seam retains future raw writer
//! bytes/key pointers only process-locally and requires pointer erasure before
//! any D55 row can be constructed.

#![cfg(windows)]

use core::cell::RefCell;
use core::ffi::c_void;
use core::{mem, ptr};
use minhook::{MinHook, MH_STATUS};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{
    CombatSide, CurrentLossObservationRow, WidthObservationRow,
    CURRENT_LOSS_REQUIRED_FLAG_MASK, MAX_DIVISION_ID, MAX_SIDE_UNITS,
    WIDTH_REQUIRED_FLAG_MASK,
};

pub const WIDTH_CARRIER_ACTIVATION: bool = false;
pub const CURRENT_LOSS_CARRIER_ACTIVATION: bool = false;
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const WIDTH_PRODUCER_INSTRUCTION_EXPORT_SHA256: &str =
    "753c4a74263fb9d791307442420fd3f2b4729a5f2e973ae26378b39e00fe2ff1";
pub const EXACT10_POOL_STATUS_RECEIPT_SHA256: &str =
    "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395";
pub const WIDTH_PRODUCER_MANIFEST_SHA256: &str =
    "a89ca3fa33225c8059a8f50a370aaba7329ac671b3f6c7ac1733822bfb9899ab";
pub const WIDTH_PRODUCER_PCODE_SHA256: &str =
    "89c077608595ffbff0de24382d70209672e790869c12b36f19cf1479c62dcd55";
pub const WIDTH_LEAF_MANIFEST_SHA256: &str =
    "875367e4aa124667515c1cdde14daed988d0a78e6d914972beed3476bca2dfb0";
pub const WIDTH_LEAF_INSTRUCTION_EXPORT_SHA256: &str =
    "8fd7bf09afeebdba78a45d0e4bfc326ea3f2612a64cfa9706cdc7172dd43e794";
pub const WIDTH_LEAF_PCODE_SHA256: &str =
    "59278dd430871b91cda3f5777a04750e1694ff96f571308e3e25ca079e40462b";

/// No production writer specification exists yet. Leaving the RVA and
/// sentinel absent is part of the fail-closed contract, not a TODO guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthenticatedLossWriterContract {
    pub writer_rva: usize,
    pub sentinel: [u8; 16],
}

pub const AUTHENTICATED_LOSS_WRITER: Option<AuthenticatedLossWriterContract> = None;

pub const WIDTH_PRODUCER_RVA: usize = 0x1299_610;
pub const DIVISION_WIDTH_LEAF_RVA: usize = 0x0C6_E270;
pub const DIVISION_WRAPPER_VTABLE_RVA: usize = 0x2943_AD0;
pub const DIVISION_VTABLE_RVA: usize = 0x2943_CB0;

const DIVISION_BODY_DELTA: usize = 0x10;
const DIVISION_ID_OFFSET: usize = 0x0c;
const MAX_WRAPPERS_PER_CALL: usize = 128;
const MAX_PENDING_WIDTH_CALLS: usize = 8_192;
const MAX_PENDING_LOSS_OBSERVATIONS: usize = 8_192;

// hoi4.exe 1.19.2.0.a729, SHA-256
// 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad.
// 753c4a74263fb9d791307442420fd3f2b4729a5f2e973ae26378b39e00fe2ff1.
const WIDTH_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
const DIVISION_WIDTH_LEAF_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];

type WidthProducerFn = unsafe extern "system" fn(usize, *mut i64) -> *mut i64;
type DivisionWidthLeafFn = unsafe extern "system" fn(usize, *mut i64, usize) -> *mut i64;

static ORIGINAL_WIDTH_PRODUCER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DIVISION_WIDTH_LEAF: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static WIDTH_INSTALLED: AtomicBool = AtomicBool::new(false);
static NEXT_CAPTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static ACTIVE_WIDTH_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static WIDTH_DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static WIDTH_DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static WIDTH_DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static WIDTH_DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static WIDTH_DROPPED_NESTED: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_INVALID: AtomicU64 = AtomicU64::new(0);
static LOSS_DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static WIDTH_STATE: OnceLock<Mutex<WidthState>> = OnceLock::new();
static LOSS_STATE: OnceLock<Mutex<LossState>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static WIDTH_CONTEXT: RefCell<Option<WidthContext>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RawWrapperResult {
    wrapper_ptr: usize,
    width_raw: i64,
    identity_validated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawWidthCallCapture {
    pub side_ptr: usize,
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub capture_sequence: u64,
    pub final_width_raw: i64,
    wrappers: [RawWrapperResult; MAX_WRAPPERS_PER_CALL],
    wrapper_count: usize,
    pub byte_gates_validated: bool,
}

impl RawWidthCallCapture {
    pub fn wrapper_count(&self) -> usize {
        self.wrapper_count
    }
}

#[derive(Clone)]
struct WidthContext {
    side_ptr: usize,
    capture_sequence: u64,
    wrappers: [RawWrapperResult; MAX_WRAPPERS_PER_CALL],
    wrapper_count: usize,
    overflowed: bool,
}

impl WidthContext {
    fn new(side_ptr: usize, capture_sequence: u64) -> Self {
        Self {
            side_ptr,
            capture_sequence,
            wrappers: [RawWrapperResult::default(); MAX_WRAPPERS_PER_CALL],
            wrapper_count: 0,
            overflowed: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideBinding {
    pub side_ptr: usize,
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
}

struct WidthState {
    bindings: BTreeMap<usize, SideBinding>,
    calls: VecDeque<RawWidthCallCapture>,
}

impl Default for WidthState {
    fn default() -> Self {
        Self {
            bindings: BTreeMap::new(),
            calls: VecDeque::with_capacity(MAX_PENDING_WIDTH_CALLS),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WidthDrain {
    pub calls: Vec<RawWidthCallCapture>,
    pub dropped_lock_busy: u64,
    pub dropped_unbound: u64,
    pub dropped_unreadable: u64,
    pub dropped_overflow: u64,
    pub dropped_nested: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLossWriterObservation {
    /// Process-local identity only; this type must never be serialized.
    pub combat_ptr: usize,
    pub side_ptr: usize,
    pub capture_sequence: u64,
    pub writer_rva: usize,
    pub observed_entry_bytes: [u8; 16],
    pub manpower_before_bytes: [u8; 8],
    pub manpower_after_bytes: [u8; 8],
    pub equipment: Vec<RawEquipmentLossDelta>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEquipmentLossDelta {
    /// Process-local map key pointer; erased by `project_loss_observation`.
    pub equipment_key_ptr: usize,
    pub before_bytes: [u8; 8],
    pub after_bytes: [u8; 8],
}

/// Exact current-epoch and equipment identity bridge supplied by the already
/// validated D21/D55 capture at drain time. It is intentionally independent of
/// any unproven loss-map layout.
pub trait LossIdentityResolver {
    fn active_epoch(
        &self,
        combat_ptr: usize,
        side_ptr: usize,
    ) -> Option<(u32, u64, CombatSide)>;
    fn equipment_id(&self, equipment_key_ptr: usize) -> Option<u32>;
}

struct LossState {
    observations: VecDeque<RawLossWriterObservation>,
}

impl Default for LossState {
    fn default() -> Self {
        Self {
            observations: VecDeque::with_capacity(MAX_PENDING_LOSS_OBSERVATIONS),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LossDrain {
    pub observations: Vec<RawLossWriterObservation>,
    pub dropped_lock_busy: u64,
    pub dropped_invalid: u64,
    pub dropped_overflow: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledWidthHookAddresses {
    pub producer: usize,
    pub division_leaf: usize,
}

struct ActiveWidthCallback;

impl ActiveWidthCallback {
    fn enter() -> Self {
        ACTIVE_WIDTH_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ActiveWidthCallback {
    fn drop(&mut self) {
        ACTIVE_WIDTH_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn width_addresses(base: usize) -> InstalledWidthHookAddresses {
    InstalledWidthHookAddresses {
        producer: base.saturating_add(WIDTH_PRODUCER_RVA),
        division_leaf: base.saturating_add(DIVISION_WIDTH_LEAF_RVA),
    }
}

/// The install path is intentionally fail-closed until the X2 control card is
/// satisfied and the compile-time activation is deliberately promoted.
pub fn install_width_observer() -> Result<InstalledWidthHookAddresses, String> {
    if !WIDTH_CARRIER_ACTIVATION {
        return Err("X2 width carrier inactive: exact bracketing control observations not yet authenticated".into());
    }
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 width install mutex poisoned".to_string())?;
    if WIDTH_INSTALLED.load(Ordering::Acquire) {
        return Ok(width_addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let sites = width_addresses(base);
    verify_sentinel(sites.producer, &WIDTH_PRODUCER_SENTINEL, "width producer")?;
    verify_sentinel(
        sites.division_leaf,
        &DIVISION_WIDTH_LEAF_SENTINEL,
        "division width leaf",
    )?;
    WIDTH_STATE.get_or_init(|| Mutex::new(WidthState::default()));

    let leaf = unsafe {
        MinHook::create_hook(
            sites.division_leaf as *mut c_void,
            hook_division_width_leaf as *mut c_void,
        )
    }
    .map_err(|status| format!("division width leaf hook creation failed: {status}"))?;
    let producer = match unsafe {
        MinHook::create_hook(
            sites.producer as *mut c_void,
            hook_width_producer as *mut c_void,
        )
    } {
        Ok(value) => value,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(sites.division_leaf as *mut c_void) };
            return Err(format!("width producer hook creation failed: {status}"));
        }
    };
    ORIGINAL_DIVISION_WIDTH_LEAF.store(leaf as usize, Ordering::Release);
    ORIGINAL_WIDTH_PRODUCER.store(producer as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    for target in [sites.division_leaf, sites.producer] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            for rollback in [sites.producer, sites.division_leaf] {
                let _ = unsafe { MinHook::disable_hook(rollback as *mut c_void) };
                let _ = unsafe { MinHook::remove_hook(rollback as *mut c_void) };
            }
            ORIGINAL_DIVISION_WIDTH_LEAF.store(0, Ordering::Release);
            ORIGINAL_WIDTH_PRODUCER.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("width hook enable failed at 0x{target:X}: {status}"));
        }
    }
    WIDTH_INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn width_observer_installed() -> bool {
    WIDTH_INSTALLED.load(Ordering::Acquire)
}

pub fn install_current_loss_observer() -> Result<AuthenticatedLossWriterContract, String> {
    if !CURRENT_LOSS_CARRIER_ACTIVATION {
        return Err(
            "X2 loss carrier inactive: exact writer and two-hour control not yet authenticated"
                .into(),
        );
    }
    let contract = AUTHENTICATED_LOSS_WRITER.ok_or_else(|| {
        "X2 loss carrier has no authenticated writer RVA/sentinel contract".to_string()
    })?;
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let address = base
        .checked_add(contract.writer_rva)
        .ok_or_else(|| "X2 loss writer address overflow".to_string())?;
    verify_sentinel(address, &contract.sentinel, "current loss writer")?;
    Ok(contract)
}

pub fn uninstall_width_observer() -> Result<(), String> {
    seal_drain_and_uninstall_width_observer().map(|_| ())
}

/// Disable producer admission first, wait for all admitted producer/leaf
/// callbacks, drain exactly once, then remove detours and clear pointer state.
pub fn seal_drain_and_uninstall_width_observer() -> Result<WidthDrain, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "X2 width uninstall mutex poisoned".to_string())?;
    if !WIDTH_INSTALLED.load(Ordering::Acquire) {
        return drain_width_calls();
    }
    let sites = width_addresses(MODULE_BASE.load(Ordering::Acquire));
    for target in [sites.producer, sites.division_leaf] {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("width hook disable failed at 0x{target:X}: {status}")),
        }
    }
    while ACTIVE_WIDTH_CALLBACKS.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
    let drained = drain_width_calls()?;
    for target in [sites.producer, sites.division_leaf] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("width hook removal failed at 0x{target:X}: {status}")),
        }
    }
    WIDTH_INSTALLED.store(false, Ordering::Release);
    ORIGINAL_WIDTH_PRODUCER.store(0, Ordering::Release);
    ORIGINAL_DIVISION_WIDTH_LEAF.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    WIDTH_CONTEXT.with(|slot| *slot.borrow_mut() = None);
    if let Some(state) = WIDTH_STATE.get() {
        *state
            .lock()
            .map_err(|_| "X2 width state mutex poisoned".to_string())? = WidthState::default();
    }
    Ok(drained)
}

pub fn publish_side_bindings(bindings: &[SideBinding]) -> Result<(), String> {
    let mut next = BTreeMap::new();
    for binding in bindings {
        if binding.side_ptr == 0
            || binding.combat_id == 0
            || binding.generation == 0
            || next.insert(binding.side_ptr, binding.clone()).is_some()
        {
            return Err("invalid or duplicate X2 width side binding".into());
        }
    }
    let state = WIDTH_STATE.get_or_init(|| Mutex::new(WidthState::default()));
    state
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?
        .bindings = next;
    Ok(())
}

pub fn drain_width_calls() -> Result<WidthDrain, String> {
    let mut state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    Ok(WidthDrain {
        calls: state.calls.drain(..).collect(),
        dropped_lock_busy: WIDTH_DROPPED_LOCK_BUSY.swap(0, Ordering::AcqRel),
        dropped_unbound: WIDTH_DROPPED_UNBOUND.swap(0, Ordering::AcqRel),
        dropped_unreadable: WIDTH_DROPPED_UNREADABLE.swap(0, Ordering::AcqRel),
        dropped_overflow: WIDTH_DROPPED_OVERFLOW.swap(0, Ordering::AcqRel),
        dropped_nested: WIDTH_DROPPED_NESTED.swap(0, Ordering::AcqRel),
    })
}

/// Erase wrapper and side pointers only after the same-epoch binding, unique
/// wrapper/division identity, and exact final-sum gates all close. D55 decode
/// separately requires the grouped wrapper count to equal the same-frame
/// authenticated side-detail committed count.
pub fn project_width_call(capture: &RawWidthCallCapture) -> Result<Vec<WidthObservationRow>, String> {
    if !capture.byte_gates_validated || capture.wrapper_count == 0 {
        return Err("width capture lacks byte gates or wrapper rows".into());
    }
    let state = WIDTH_STATE
        .get_or_init(|| Mutex::new(WidthState::default()))
        .lock()
        .map_err(|_| "X2 width state mutex poisoned".to_string())?;
    let binding = state
        .bindings
        .get(&capture.side_ptr)
        .ok_or_else(|| "width capture side is not bound to an active epoch".to_string())?;
    if (binding.combat_id, binding.generation, binding.side)
        != (capture.combat_id, capture.generation, capture.side)
    {
        return Err("width capture active epoch changed before pointer erasure".into());
    }
    let mut sum = 0i64;
    let mut rows = Vec::with_capacity(capture.wrapper_count);
    let mut division_ids = BTreeSet::new();
    for (ordinal, raw) in capture.wrappers[..capture.wrapper_count].iter().enumerate() {
        if !raw.identity_validated {
            return Err("width wrapper identity gate failed".into());
        }
        let division_id = resolve_wrapper_division_id(raw.wrapper_ptr)
            .ok_or_else(|| "width wrapper division identity became unreadable".to_string())?;
        if !division_ids.insert(division_id) {
            return Err("width wrapper division identity repeats within one call".into());
        }
        sum = sum
            .checked_add(raw.width_raw)
            .ok_or_else(|| "width exact sum overflow".to_string())?;
        rows.push(WidthObservationRow {
            combat_id: binding.combat_id,
            generation: binding.generation,
            side: binding.side,
            capture_sequence: capture.capture_sequence,
            ordinal: ordinal as u32,
            wrapper_count: capture.wrapper_count as u32,
            division_id,
            control_flags: WIDTH_REQUIRED_FLAG_MASK,
            wrapper_width_raw: raw.width_raw,
            final_width_raw: capture.final_width_raw,
        });
    }
    if sum != capture.final_width_raw {
        return Err("width wrapper sum does not equal producer final result".into());
    }
    Ok(rows)
}

/// Queue raw writer operands only after a future exact detour matches the
/// promoted RVA and entry sentinel. Pointer-bearing bytes remain process-local.
pub fn submit_raw_loss_observation(observation: RawLossWriterObservation) -> Result<(), String> {
    if !CURRENT_LOSS_CARRIER_ACTIVATION {
        return Err("X2 loss carrier inactive: exact writer and two-hour control not yet authenticated".into());
    }
    validate_raw_loss_observation(&observation).map_err(|error| {
        LOSS_DROPPED_INVALID.fetch_add(1, Ordering::Relaxed);
        error
    })?;
    let state = LOSS_STATE.get_or_init(|| Mutex::new(LossState::default()));
    let Ok(mut state) = state.try_lock() else {
        LOSS_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
        return Err("X2 loss queue busy".into());
    };
    if state.observations.len() >= MAX_PENDING_LOSS_OBSERVATIONS {
        LOSS_DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return Err("X2 loss queue full".into());
    }
    state.observations.push_back(observation);
    Ok(())
}

pub fn drain_loss_observations() -> Result<LossDrain, String> {
    let mut state = LOSS_STATE
        .get_or_init(|| Mutex::new(LossState::default()))
        .lock()
        .map_err(|_| "X2 loss state mutex poisoned".to_string())?;
    Ok(LossDrain {
        observations: state.observations.drain(..).collect(),
        dropped_lock_busy: LOSS_DROPPED_LOCK_BUSY.swap(0, Ordering::AcqRel),
        dropped_invalid: LOSS_DROPPED_INVALID.swap(0, Ordering::AcqRel),
        dropped_overflow: LOSS_DROPPED_OVERFLOW.swap(0, Ordering::AcqRel),
    })
}

pub fn project_loss_observation(
    observation: &RawLossWriterObservation,
    resolver: &impl LossIdentityResolver,
) -> Result<Vec<CurrentLossObservationRow>, String> {
    validate_raw_loss_observation(observation)?;
    let (combat_id, generation, side) = resolver
        .active_epoch(observation.combat_ptr, observation.side_ptr)
        .ok_or_else(|| "X2 loss pointers do not resolve to one current active epoch".to_string())?;
    if combat_id == 0 || generation == 0 {
        return Err("X2 loss active-epoch identity is null".into());
    }
    let manpower_before_raw = i64::from_le_bytes(observation.manpower_before_bytes);
    let manpower_after_raw = i64::from_le_bytes(observation.manpower_after_bytes);
    if manpower_before_raw == manpower_after_raw {
        return Err("X2 loss manpower control has no delta".into());
    }
    let mut equipment_ids = BTreeSet::new();
    let mut rows = Vec::with_capacity(observation.equipment.len());
    for (ordinal, equipment) in observation.equipment.iter().enumerate() {
        let equipment_id = resolver
            .equipment_id(equipment.equipment_key_ptr)
            .filter(|id| *id != 0)
            .ok_or_else(|| "X2 loss equipment key has no stable pointer-free identity".to_string())?;
        let equipment_before_raw = i64::from_le_bytes(equipment.before_bytes);
        let equipment_after_raw = i64::from_le_bytes(equipment.after_bytes);
        if !equipment_ids.insert(equipment_id) || equipment_before_raw == equipment_after_raw {
            return Err("X2 loss equipment identity repeats or has no delta".into());
        }
        rows.push(CurrentLossObservationRow {
            combat_id,
            generation,
            side,
            capture_sequence: observation.capture_sequence,
            ordinal: ordinal as u32,
            equipment_count: observation.equipment.len() as u32,
            equipment_id,
            control_flags: CURRENT_LOSS_REQUIRED_FLAG_MASK,
            manpower_before_raw,
            manpower_after_raw,
            equipment_before_raw,
            equipment_after_raw,
        });
    }
    Ok(rows)
}

fn validate_raw_loss_observation(observation: &RawLossWriterObservation) -> Result<(), String> {
    let contract = AUTHENTICATED_LOSS_WRITER
        .ok_or_else(|| "X2 loss writer contract is not authenticated".to_string())?;
    if observation.combat_ptr == 0
        || observation.side_ptr == 0
        || observation.capture_sequence == 0
        || observation.equipment.is_empty()
        || observation.equipment.len() > MAX_SIDE_UNITS
        || observation.writer_rva != contract.writer_rva
        || observation.observed_entry_bytes != contract.sentinel
        || observation.manpower_before_bytes == observation.manpower_after_bytes
    {
        return Err("X2 loss observation fails pointer, byte, or manpower-delta gate".into());
    }
    if observation.equipment.iter().any(|entry| {
        entry.equipment_key_ptr == 0 || entry.before_bytes == entry.after_bytes
    }) {
        return Err("X2 loss observation fails raw equipment key/delta gate".into());
    }
    Ok(())
}

unsafe extern "system" fn hook_width_producer(side_ptr: usize, out: *mut i64) -> *mut i64 {
    let _active = ActiveWidthCallback::enter();
    let original_addr = ORIGINAL_WIDTH_PRODUCER.load(Ordering::Acquire);
    if original_addr == 0 {
        return ptr::null_mut();
    }
    let original: WidthProducerFn = unsafe { mem::transmute(original_addr) };
    let nested = WIDTH_CONTEXT.with(|slot| slot.borrow().is_some());
    if nested {
        WIDTH_DROPPED_NESTED.fetch_add(1, Ordering::Relaxed);
        // Suppress leaf admission for the nested producer so its wrappers do
        // not contaminate the outer synchronous call's exact sum/order.
        let outer = WIDTH_CONTEXT.with(|slot| slot.borrow_mut().take());
        let returned = unsafe { original(side_ptr, out) };
        WIDTH_CONTEXT.with(|slot| *slot.borrow_mut() = outer);
        return returned;
    }
    let sequence = NEXT_CAPTURE_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    WIDTH_CONTEXT.with(|slot| *slot.borrow_mut() = Some(WidthContext::new(side_ptr, sequence)));
    let returned = unsafe { original(side_ptr, out) };
    let context = WIDTH_CONTEXT.with(|slot| slot.borrow_mut().take());
    let Some(context) = context else {
        WIDTH_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    if context.overflowed {
        WIDTH_DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return returned;
    }
    let final_ptr = if returned.is_null() { out } else { returned };
    let Some(final_width_raw) = read_i64(final_ptr as usize) else {
        WIDTH_DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    let state = WIDTH_STATE.get_or_init(|| Mutex::new(WidthState::default()));
    let Ok(mut state) = state.try_lock() else {
        WIDTH_DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    let Some(binding) = state.bindings.get(&side_ptr).cloned() else {
        WIDTH_DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return returned;
    };
    if state.calls.len() >= MAX_PENDING_WIDTH_CALLS {
        WIDTH_DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return returned;
    }
    state.calls.push_back(RawWidthCallCapture {
        side_ptr: context.side_ptr,
        combat_id: binding.combat_id,
        generation: binding.generation,
        side: binding.side,
        capture_sequence: context.capture_sequence,
        final_width_raw,
        wrappers: context.wrappers,
        wrapper_count: context.wrapper_count,
        byte_gates_validated: true,
    });
    returned
}

unsafe extern "system" fn hook_division_width_leaf(
    wrapper_ptr: usize,
    out: *mut i64,
    debug_sink: usize,
) -> *mut i64 {
    let _active = ActiveWidthCallback::enter();
    let original_addr = ORIGINAL_DIVISION_WIDTH_LEAF.load(Ordering::Acquire);
    if original_addr == 0 {
        return ptr::null_mut();
    }
    let original: DivisionWidthLeafFn = unsafe { mem::transmute(original_addr) };
    let returned = unsafe { original(wrapper_ptr, out, debug_sink) };
    let result_ptr = if returned.is_null() { out } else { returned };
    let width_raw = read_i64(result_ptr as usize);
    WIDTH_CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(context) = slot.as_mut() else {
            return;
        };
        if context.wrapper_count >= MAX_WRAPPERS_PER_CALL {
            context.overflowed = true;
            return;
        }
        let Some(width_raw) = width_raw else {
            context.overflowed = true;
            return;
        };
        let identity_validated = resolve_wrapper_division_id(wrapper_ptr).is_some();
        context.wrappers[context.wrapper_count] = RawWrapperResult {
            wrapper_ptr,
            width_raw,
            identity_validated,
        };
        context.wrapper_count += 1;
    });
    returned
}

fn resolve_wrapper_division_id(wrapper_ptr: usize) -> Option<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0
        || read_usize(wrapper_ptr)? != base.checked_add(DIVISION_WRAPPER_VTABLE_RVA)?
    {
        return None;
    }
    let division = wrapper_ptr.checked_add(DIVISION_BODY_DELTA)?;
    if read_usize(division)? != base.checked_add(DIVISION_VTABLE_RVA)? {
        return None;
    }
    let id = read_u32(division.checked_add(DIVISION_ID_OFFSET)?)?;
    (id > 0 && id <= MAX_DIVISION_ID).then_some(id)
}

fn verify_sentinel(address: usize, expected: &[u8], name: &str) -> Result<(), String> {
    let actual = read_process_bytes(address, expected.len())
        .ok_or_else(|| format!("{name} sentinel unreadable at 0x{address:X}"))?;
    if actual != expected {
        return Err(format!("{name} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn read_process_bytes(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 || len == 0 {
        return None;
    }
    let mut out = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr().cast::<c_void>(),
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(out)
}

fn read_i64(address: usize) -> Option<i64> {
    let bytes = read_process_bytes(address, 8)?;
    Some(i64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u32(address: usize) -> Option<u32> {
    let bytes = read_process_bytes(address, 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_usize(address: usize) -> Option<usize> {
    let bytes = read_process_bytes(address, mem::size_of::<usize>())?;
    Some(usize::from_le_bytes(bytes.try_into().ok()?))
}
