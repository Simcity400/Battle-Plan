#![cfg(windows)]

use core::{cell::Cell, ffi::c_void, mem, ptr};
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released,
    observe_released_set, ExpectedRvaHook, HookTeardownAttestation, InstallAbortDiscard,
    InstalledRvaHookReceipt, ReleasedRvaHookReceipt, MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicI64, AtomicU64, AtomicU8, AtomicUsize, Ordering,
};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const OUTER_CFEX_MEMBER_WRITER_RVA: usize = 0x1961_DD0;
pub const ATTRIBUTED_RECORD_WRITER_RVA: usize = 0x01D_2C90;
pub const COMBAT_SOURCE_RECORD_WRITER_RVA: usize = 0x01D_2D10;
pub const CAUSE_RECORD_WRITER_RVA: usize = 0x01D_2DA0;
pub const SUNK_SHIP_INFO_VTABLE_RVA: usize = 0x294_1608;
pub const MAX_PENDING_EVENTS: usize = 65_536;
pub const MAX_CAPTURE_TEXT: usize = 256;
pub const EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_A33_SUNK_SHIP_EVIDENCE";

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const SUNK_SHIP_HISTORY_VECTOR_OFFSET: usize = 0x590;
const CGAMESTATE_CURRENT_DATE_OFFSET: usize = 0x468;
const SUNK_SHIP_INFO_SINK_DATE_OFFSET: usize = 0x58;

const OUTER_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48,
];
const RECORD_WRITER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];

const PRODUCTION_HOOK_SPECS: [ExpectedRvaHook; 4] = [
    ExpectedRvaHook {
        logical_name: "gameplay.d86.attributed_record_writer",
        requested_rva: ATTRIBUTED_RECORD_WRITER_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &RECORD_WRITER_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d86.cause_record_writer",
        requested_rva: CAUSE_RECORD_WRITER_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &RECORD_WRITER_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d86.combat_source_record_writer",
        requested_rva: COMBAT_SOURCE_RECORD_WRITER_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &RECORD_WRITER_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d86.outer_cfex_member_writer",
        requested_rva: OUTER_CFEX_MEMBER_WRITER_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &OUTER_SENTINEL,
    },
];

const HOOK_NAMES: [&str; 4] = [
    "gameplay.d86.outer_cfex_member_writer",
    "gameplay.d86.attributed_record_writer",
    "gameplay.d86.combat_source_record_writer",
    "gameplay.d86.cause_record_writer",
];

fn hook_batch(targets: &[usize]) -> Vec<(&'static str, *mut c_void)> {
    targets
        .iter()
        .enumerate()
        .map(|(index, target)| (HOOK_NAMES[index], *target as *mut c_void))
        .collect()
}

const INSTALL_PHASE_NONE: u8 = 0;
const INSTALL_PHASE_ACTIVATING: u8 = 1;
const INSTALL_PHASE_ACTIVE: u8 = 2;
const INSTALL_PHASE_RESTORE_REQUIRED: u8 = 3;
const INSTALL_MODE_NONE: u8 = 0;
const INSTALL_MODE_PRODUCTION: u8 = 1;
const INSTALL_MODE_EVIDENCE: u8 = 2;
const HOOK_OUTER: u8 = 1 << 0;
const HOOK_ATTRIBUTED: u8 = 1 << 1;
const HOOK_COMBAT_SOURCE: u8 = 1 << 2;
const HOOK_CAUSE: u8 = 1 << 3;
const ALL_HOOKS: u8 = HOOK_OUTER | HOOK_ATTRIBUTED | HOOK_COMBAT_SOURCE | HOOK_CAUSE;

type OuterFn = unsafe extern "system" fn(usize, usize, usize, usize, u8);
type AttributedWriterFn = unsafe extern "system" fn(usize, usize, usize, usize);
type CombatSourceWriterFn = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize);
type CauseWriterFn = unsafe extern "system" fn(usize, usize, usize, usize, u32);

static ORIGINAL_OUTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ATTRIBUTED_WRITER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_COMBAT_SOURCE_WRITER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CAUSE_WRITER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(2);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static NEXT_DRAIN_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static UNBOUND_EPOCH_SNAPSHOT_ABSENT: AtomicU64 = AtomicU64::new(0);
static UNBOUND_MEMBER_UNRESOLVED: AtomicU64 = AtomicU64::new(0);
static UNBOUND_BRACKET_THREAD_MISMATCH: AtomicU64 = AtomicU64::new(0);
static UNBOUND_GAMESTATE_MISMATCH: AtomicU64 = AtomicU64::new(0);
// ordered by a release/acquire pair on the LAST store instead -- see
static UNBOUND_MEMBER_UNRESOLVED_CAPTURED: AtomicBool = AtomicBool::new(false);
static UNBOUND_MEMBER_UNRESOLVED_ADDRESS: AtomicU64 = AtomicU64::new(0);
static UNBOUND_MEMBER_UNRESOLVED_PUBLICATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_MEMBERS: AtomicU64 = AtomicU64::new(0);
static UNBOUND_MEMBER_UNRESOLVED_EVENT_HOUR: AtomicI64 = AtomicI64::new(i64::MIN);
static UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_HOUR: AtomicI64 = AtomicI64::new(i64::MIN);
static DROPPED_REENTRANT: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static LOST_UNBOUND_SINCE_INSTALL: AtomicU64 = AtomicU64::new(0);
static LOST_REENTRANT_SINCE_INSTALL: AtomicU64 = AtomicU64::new(0);
static LOST_UNREADABLE_SINCE_INSTALL: AtomicU64 = AtomicU64::new(0);
static LOST_LOCK_BUSY_SINCE_INSTALL: AtomicU64 = AtomicU64::new(0);
static LOST_OVERFLOW_SINCE_INSTALL: AtomicU64 = AtomicU64::new(0);
static SEQUENCE_REGRESSION_SINCE_INSTALL: AtomicBool = AtomicBool::new(false);
static PRODUCER_OBSERVED_DATE: AtomicI32 = AtomicI32::new(0);
static COMPLETENESS_POISONED: AtomicBool = AtomicBool::new(false);
static EVIDENCE_INSTALL: AtomicBool = AtomicBool::new(false);
static BASELINE_MEASURED_EMPTY: AtomicBool = AtomicBool::new(false);
static BASELINE_REVALIDATED_EMPTY: AtomicBool = AtomicBool::new(false);
static BASELINE_COMPLETE: AtomicBool = AtomicBool::new(false);
static BASELINE_REVALIDATED_COMPLETE: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<CaptureQueueState>> = OnceLock::new();
static HISTORY: OnceLock<Mutex<HistoryState>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static PRODUCTION_HOOK_RECEIPTS: Mutex<[Option<InstalledRvaHookReceipt>; 4]> =
    Mutex::new([None; 4]);
static RELEASED_HOOK_RECEIPTS: Mutex<[Option<ReleasedRvaHookReceipt>; 4]> = Mutex::new([None; 4]);
static INSTALL_PHASE: AtomicU8 = AtomicU8::new(INSTALL_PHASE_NONE);
static INSTALL_MODE: AtomicU8 = AtomicU8::new(INSTALL_MODE_NONE);
static CREATED_HOOKS: AtomicU8 = AtomicU8::new(0);
static ENABLED_HOOKS: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static OUTER_CONTEXT: Cell<Option<OuterContext>> = const { Cell::new(None) };
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OuterContext {
    combat_id: u32,
    combat_generation: u64,
    battle_slot_index_raw: u32,
    thread_id: u32,
    member_epoch_publication_sequence: u64,
    member_epoch_game_hour_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSunkShipCapture {
    pub event_sequence: u64,
    pub combat_join_present: bool,
    pub thread_id: u32,
    pub member_epoch_publication_sequence: u64,
    pub member_epoch_game_hour_raw: i32,
    pub combat_id: u32,
    pub combat_generation: u64,
    pub victim_member_battle_slot_index: u32,
    pub victim_country_id_raw: u32,
    pub attacker_country_id_raw: u32,
    pub ui_source_country_id_raw: u32,
    pub sink_date_raw: i32,
    pub assisted_kill: bool,
    pub victim_ship_reference_raw: [u32; 2],
    pub damage_source_reference_raw: [u32; 2],
    pub task_force_id_raw: u32,
    pub terminal_context_raw: [u32; 2],
    pub cause_record_flag: bool,
    pub ship_name: crate::ExactText,
    pub ship_class_name: crate::ExactText,
    pub attribution_source_name: crate::ExactText,
    pub attribution_source_class_name: crate::ExactText,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDrain {
    pub hook_epoch: u64,
    pub events: Vec<RawSunkShipCapture>,
    pub dropped_unbound: u64,
    pub dropped_reentrant: u64,
    pub dropped_unreadable: u64,
    pub dropped_lock_busy: u64,
    pub dropped_overflow: u64,
    pub completeness_poisoned: bool,
    pub baseline_measured_empty: bool,
    pub baseline_revalidated_empty: bool,
    pub baseline_complete: bool,
    pub baseline_revalidated_complete: bool,
    pub evidence_install: bool,
    pub installed: bool,
    pub install_phase: u8,
    pub install_mode: u8,
}

impl HookDrain {
    pub fn loss_free(&self) -> bool {
        self.dropped_unbound == 0
            && self.dropped_reentrant == 0
            && self.dropped_unreadable == 0
            && self.dropped_lock_busy == 0
            && self.dropped_overflow == 0
            && !self.completeness_poisoned
    }

    pub fn evidence_fixture_contract_satisfied(&self) -> bool {
        self.evidence_install
            && self.hook_epoch != 0
            && self.baseline_measured_empty
            && self.baseline_revalidated_empty
            && self.loss_free()
            && !self.events.is_empty()
            && self.events.iter().all(|event| {
                event.event_sequence > 1
                    && event.thread_id != 0
                    && if event.combat_join_present {
                        event.member_epoch_publication_sequence != 0
                            && event.member_epoch_game_hour_raw > 0
                            && event.combat_id != 0
                            && event.combat_generation != 0
                    } else {
                        event.member_epoch_publication_sequence == 0
                            && event.member_epoch_game_hour_raw == 0
                            && event.combat_id == 0
                            && event.combat_generation == 0
                            && event.victim_member_battle_slot_index == 0
                    }
                    && event.victim_country_id_raw != 0
                    && (event.attacker_country_id_raw != 0 || event.cause_record_flag)
                    && event.sink_date_raw > 0
                    && !(event.cause_record_flag && event.assisted_kill)
                    && event.victim_ship_reference_raw != [0, 0]
                    && event
                        .ship_name
                        .present()
                        .is_some_and(|name| !name.is_empty() && name.len() < MAX_CAPTURE_TEXT)
                    && event
                        .ship_class_name
                        .present()
                        .is_some_and(|name| !name.is_empty() && name.len() < MAX_CAPTURE_TEXT)
                    && event
                        .attribution_source_name
                        .present()
                        .is_some_and(|name| name.len() < MAX_CAPTURE_TEXT)
                    && event
                        .attribution_source_class_name
                        .present()
                        .is_some_and(|name| name.len() < MAX_CAPTURE_TEXT)
            })
            && self
                .events
                .windows(2)
                .all(|pair| pair[0].event_sequence < pair[1].event_sequence)
    }
}

#[derive(Default)]
struct HistoryState {
    baseline_complete: bool,
    events: Vec<crate::SunkShipEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DrainReservationState {
    reservation_id: u64,
    row_count: usize,
    poisoned: bool,
    dropped_unbound: u64,
    dropped_reentrant: u64,
    dropped_unreadable: u64,
    dropped_lock_busy: u64,
    dropped_overflow: u64,
    installed: bool,
    install_phase: u8,
    install_mode: u8,
}

impl DrainReservationState {
    fn same_reserved_prefix(self, other: Self) -> bool {
        self.row_count == other.row_count
            && self.dropped_unbound == other.dropped_unbound
            && self.dropped_reentrant == other.dropped_reentrant
            && self.dropped_unreadable == other.dropped_unreadable
            && self.dropped_lock_busy == other.dropped_lock_busy
            && self.dropped_overflow == other.dropped_overflow
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SealedCompleteSnapshotState {
    reserved_prefix: DrainReservationState,
    snapshot: crate::SunkShipSnapshot,
}

#[derive(Debug)]
struct CaptureQueueState {
    pending: VecDeque<RawSunkShipCapture>,
    reservation: Option<DrainReservationState>,
    sealed_complete_snapshot: Option<SealedCompleteSnapshotState>,
}

impl CaptureQueueState {
    fn new() -> Self {
        Self {
            pending: VecDeque::with_capacity(MAX_PENDING_EVENTS),
            reservation: None,
            sealed_complete_snapshot: None,
        }
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.reservation = None;
        self.sealed_complete_snapshot = None;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookDrainReservation {
    pub reservation_id: u64,
    pub drain: HookDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteSnapshotReservation {
    pub reservation_id: u64,
    pub snapshot: crate::SunkShipSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HistoryVectorBaseline {
    data: usize,
    capacity: u32,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    ProductionSchedulingDisabled,
    LiveFixtureNotAccepted,
    Runtime(String),
}

pub fn install() -> Result<(), InstallError> {
    if !crate::PRODUCTION_SCHEDULING_ENABLED {
        return Err(InstallError::ProductionSchedulingDisabled);
    }
    install_promoted(false).map_err(InstallError::Runtime)
}

pub fn install_for_evidence() -> Result<(), InstallError> {
    if !evidence_activation_enabled() {
        return Err(InstallError::LiveFixtureNotAccepted);
    }
    install_promoted(true).map_err(InstallError::Runtime)
}

pub fn evidence_activation_enabled() -> bool {
    std::env::var(EVIDENCE_ACTIVATION_ENV).ok().as_deref() == Some("1")
}

fn install_promoted(evidence_install: bool) -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A33 install lock poisoned")?;
    let requested_mode = if evidence_install {
        INSTALL_MODE_EVIDENCE
    } else {
        INSTALL_MODE_PRODUCTION
    };
    if INSTALLED.load(Ordering::Acquire) {
        if INSTALL_PHASE.load(Ordering::Acquire) != INSTALL_PHASE_ACTIVE
            || INSTALL_MODE.load(Ordering::Acquire) != requested_mode
        {
            return Err("A33 idempotent install requested a different or invalid hook mode".into());
        }
        if !evidence_install {
            installed_hook_receipts()?;
        }
        return Ok(());
    }
    if INSTALL_PHASE.load(Ordering::Acquire) != INSTALL_PHASE_NONE
        || INSTALL_MODE.load(Ordering::Acquire) != INSTALL_MODE_NONE
        || CREATED_HOOKS.load(Ordering::Acquire) != 0
        || ENABLED_HOOKS.load(Ordering::Acquire) != 0
    {
        return Err("A33 prior failed activation still requires teardown".into());
    }
    if PRODUCTION_HOOK_RECEIPTS
        .lock()
        .map_err(|_| "A33 production hook receipt lock poisoned before install")?
        .iter()
        .any(Option::is_some)
    {
        return Err("A33 prior installed hook receipts were not released".into());
    }
    if RELEASED_HOOK_RECEIPTS
        .lock()
        .map_err(|_| "A33 released hook receipt lock poisoned before install")?
        .iter()
        .any(Option::is_some)
    {
        return Err("A33 prior released hook receipts were not consumed".into());
    }
    if STATE.get().is_some_and(|state| {
        state
            .lock()
            .map(|queue| queue.sealed_complete_snapshot.is_some())
            .unwrap_or(true)
    }) {
        return Err("A33 sealed complete snapshot still requires terminal sink commit".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A33 GetModuleHandleW(NULL) returned null".into());
    }
    let outer = base + OUTER_CFEX_MEMBER_WRITER_RVA;
    let attributed_writer = base + ATTRIBUTED_RECORD_WRITER_RVA;
    let combat_source_writer = base + COMBAT_SOURCE_RECORD_WRITER_RVA;
    let cause_writer = base + CAUSE_RECORD_WRITER_RVA;
    let targets = [outer, attributed_writer, combat_source_writer, cause_writer];
    verify_sentinel(outer, &OUTER_SENTINEL)?;
    for writer in [attributed_writer, combat_source_writer, cause_writer] {
        verify_sentinel(writer, &RECORD_WRITER_SENTINEL)?;
    }
    let baseline_before = read_complete_history_baseline(base)?;
    let baseline_measured_empty = baseline_before.events.is_empty();
    let hook_epoch = NEXT_HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    if hook_epoch == 0 {
        return Err("A33 hook epoch sequence exhausted".into());
    }
    STATE
        .get_or_init(|| Mutex::new(CaptureQueueState::new()))
        .lock()
        .map_err(|_| "A33 queue lock poisoned before install")?
        .reset();
    let next_sequence = u64::try_from(baseline_before.events.len())
        .ok()
        .and_then(|count| count.checked_add(2))
        .ok_or("A33 baseline event sequence exhausted")?;
    MODULE_BASE.store(base, Ordering::Release);
    INSTALL_MODE.store(requested_mode, Ordering::Release);
    EVIDENCE_INSTALL.store(evidence_install, Ordering::Release);
    INSTALL_PHASE.store(INSTALL_PHASE_ACTIVATING, Ordering::Release);
    let original_outer =
        match unsafe { MinHook::create_hook(outer as *mut c_void, hook_outer as *mut c_void) } {
            Ok(original) => original,
            Err(status) => {
                clear_hook_custody();
                return Err(format!("A33 outer create failed: {status}"));
            }
        };
    CREATED_HOOKS.fetch_or(HOOK_OUTER, Ordering::AcqRel);
    let original_attributed_writer = match unsafe {
        MinHook::create_hook(
            attributed_writer as *mut c_void,
            hook_attributed_writer as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            return Err(finish_failed_activation(
                format!("A33 attributed writer create failed: {status}"),
                cleanup_created_hooks(base, &[outer]),
            ));
        }
    };
    CREATED_HOOKS.fetch_or(HOOK_ATTRIBUTED, Ordering::AcqRel);
    let original_combat_source_writer = match unsafe {
        MinHook::create_hook(
            combat_source_writer as *mut c_void,
            hook_combat_source_writer as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            return Err(finish_failed_activation(
                format!("A33 combat-source writer create failed: {status}"),
                cleanup_created_hooks(base, &[outer, attributed_writer]),
            ));
        }
    };
    CREATED_HOOKS.fetch_or(HOOK_COMBAT_SOURCE, Ordering::AcqRel);
    let original_cause_writer = match unsafe {
        MinHook::create_hook(
            cause_writer as *mut c_void,
            hook_cause_writer as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            return Err(finish_failed_activation(
                format!("A33 cause writer create failed: {status}"),
                cleanup_created_hooks(base, &[outer, attributed_writer, combat_source_writer]),
            ));
        }
    };
    CREATED_HOOKS.fetch_or(HOOK_CAUSE, Ordering::AcqRel);
    ORIGINAL_OUTER.store(original_outer as usize, Ordering::Release);
    ORIGINAL_ATTRIBUTED_WRITER.store(original_attributed_writer as usize, Ordering::Release);
    ORIGINAL_COMBAT_SOURCE_WRITER.store(original_combat_source_writer as usize, Ordering::Release);
    ORIGINAL_CAUSE_WRITER.store(original_cause_writer as usize, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(hook_epoch, Ordering::Release);
    NEXT_SEQUENCE.store(next_sequence, Ordering::Release);
    DROPPED_UNBOUND.store(0, Ordering::Release);
    UNBOUND_EPOCH_SNAPSHOT_ABSENT.store(0, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED.store(0, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_HOUR.store(i64::MIN, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_EVENT_HOUR.store(i64::MIN, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_ADDRESS.store(0, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_PUBLICATION_SEQUENCE.store(0, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_MEMBERS.store(0, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_CAPTURED.store(false, Ordering::Release);
    UNBOUND_BRACKET_THREAD_MISMATCH.store(0, Ordering::Release);
    UNBOUND_GAMESTATE_MISMATCH.store(0, Ordering::Release);
    DROPPED_REENTRANT.store(0, Ordering::Release);
    DROPPED_UNREADABLE.store(0, Ordering::Release);
    DROPPED_LOCK_BUSY.store(0, Ordering::Release);
    DROPPED_OVERFLOW.store(0, Ordering::Release);
    for cause in LOSS_CAUSES {
        cause.cumulative.store(0, Ordering::Release);
    }
    SEQUENCE_REGRESSION_SINCE_INSTALL.store(false, Ordering::Release);
    PRODUCER_OBSERVED_DATE.store(0, Ordering::Release);
    COMPLETENESS_POISONED.store(false, Ordering::Release);
    BASELINE_MEASURED_EMPTY.store(baseline_measured_empty, Ordering::Release);
    BASELINE_COMPLETE.store(!evidence_install, Ordering::Release);
    BASELINE_REVALIDATED_COMPLETE.store(false, Ordering::Release);
    EVIDENCE_INSTALL.store(evidence_install, Ordering::Release);
    let mut enable_batch = hook_batch(&targets);
    enable_batch.reverse();
    let enabled = unsafe { inline_detour::enable_batch(&enable_batch) };
    for (live, hook_bit) in enabled.still_live().iter().zip([
        HOOK_CAUSE,
        HOOK_COMBAT_SOURCE,
        HOOK_ATTRIBUTED,
        HOOK_OUTER,
    ]) {
        if *live {
            ENABLED_HOOKS.fetch_or(hook_bit, Ordering::AcqRel);
        }
    }
    if let Some((name, status)) = enabled.refusal() {
        return Err(finish_failed_activation(
            format!("A33 enable failed for {name}: {status}"),
            cleanup_after_failed_install(base, &targets),
        ));
    }
    if !evidence_install {
        if let Err(error) = observe_and_persist_new_production_hook_receipts(base) {
            return Err(finish_failed_activation(
                format!("A33 installed hook receipt observation failed: {error}"),
                cleanup_after_failed_install_and_release(base, &targets),
            ));
        }
    }
    let baseline_after = match read_complete_history_baseline(base) {
        Ok(baseline) => baseline,
        Err(error) => {
            return Err(finish_failed_activation(
                format!("A33 post-enable baseline revalidation failed: {error}"),
                cleanup_after_failed_install_and_release(base, &targets),
            ));
        }
    };
    let baseline_revalidated_empty = baseline_after.events.is_empty();
    BASELINE_REVALIDATED_EMPTY.store(baseline_revalidated_empty, Ordering::Release);
    let baseline_revalidated_complete = baseline_after == baseline_before;
    BASELINE_REVALIDATED_COMPLETE.store(
        !evidence_install && baseline_revalidated_complete,
        Ordering::Release,
    );
    if !evidence_install && !baseline_revalidated_complete {
        return Err(finish_failed_activation(
            "A33 history changed while the production hooks were activating".into(),
            cleanup_after_failed_install_and_release(base, &targets),
        ));
    }
    if !evidence_install {
        let mut history = match HISTORY
            .get_or_init(|| Mutex::new(HistoryState::default()))
            .lock()
        {
            Ok(history) => history,
            Err(_) => {
                return Err(finish_failed_activation(
                    "A33 history lock poisoned during production install".into(),
                    cleanup_after_failed_install_and_release(base, &targets),
                ));
            }
        };
        *history = HistoryState {
            baseline_complete: true,
            events: baseline_after.events,
        };
    }
    if CREATED_HOOKS.load(Ordering::Acquire) != ALL_HOOKS
        || ENABLED_HOOKS.load(Ordering::Acquire) != ALL_HOOKS
    {
        return Err(finish_failed_activation(
            "A33 activation lost per-site hook custody".into(),
            cleanup_after_failed_install_and_release(base, &targets),
        ));
    }
    INSTALL_PHASE.store(INSTALL_PHASE_ACTIVE, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn teardown_required() -> bool {
    if INSTALL_PHASE.load(Ordering::Acquire) != INSTALL_PHASE_NONE {
        return true;
    }
    if STATE.get().is_some_and(|state| {
        state
            .lock()
            .map(|queue| {
                !queue.pending.is_empty()
                    || queue.reservation.is_some()
                    || queue.sealed_complete_snapshot.is_some()
            })
            .unwrap_or(true)
    }) {
        return true;
    }
    if PRODUCTION_HOOK_RECEIPTS
        .lock()
        .map(|receipts| receipts.iter().any(Option::is_some))
        .unwrap_or(true)
    {
        return true;
    }
    RELEASED_HOOK_RECEIPTS
        .lock()
        .map(|receipts| receipts.iter().any(Option::is_some))
        .unwrap_or(true)
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; 4], String> {
    if !INSTALLED.load(Ordering::Acquire)
        || INSTALL_PHASE.load(Ordering::Acquire) != INSTALL_PHASE_ACTIVE
        || INSTALL_MODE.load(Ordering::Acquire) != INSTALL_MODE_PRODUCTION
        || EVIDENCE_INSTALL.load(Ordering::Acquire)
    {
        return Err("A33 production hook receipts require an active production install".into());
    }
    let receipts = complete_installed_hook_receipts()?;
    let observed =
        reobserve_production_hook_receipts(MODULE_BASE.load(Ordering::Acquire), receipts)?;
    if observed != receipts {
        return Err("A33 installed production hook receipts changed after activation".into());
    }
    Ok(receipts)
}

pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 4], String> {
    let _lifecycle = INSTALL_LOCK
        .lock()
        .map_err(|_| "A33 lifecycle lock poisoned while taking released receipts")?;
    if STATE.get().is_some_and(|state| {
        state
            .lock()
            .map(|queue| {
                !queue.pending.is_empty()
                    || queue.reservation.is_some()
                    || queue.sealed_complete_snapshot.is_some()
            })
            .unwrap_or(true)
    }) {
        return Err(
            "A33 cannot take releases while terminal drain custody remains unresolved".into(),
        );
    }
    if PRODUCTION_HOOK_RECEIPTS
        .lock()
        .map_err(|_| "A33 production hook receipt lock poisoned")?
        .iter()
        .any(Option::is_some)
    {
        return Err("A33 cannot take releases while installed receipt custody remains".into());
    }
    let mut released = RELEASED_HOOK_RECEIPTS
        .lock()
        .map_err(|_| "A33 released hook receipt lock poisoned")?;
    let complete = [
        released[0].ok_or("A33 released hook receipt 0 is absent")?,
        released[1].ok_or("A33 released hook receipt 1 is absent")?,
        released[2].ok_or("A33 released hook receipt 2 is absent")?,
        released[3].ok_or("A33 released hook receipt 3 is absent")?,
    ];
    validate_complete_released_hook_receipts(&complete)?;
    *released = [None; 4];
    Ok(complete)
}

fn validate_complete_released_hook_receipts(
    receipts: &[ReleasedRvaHookReceipt; 4],
) -> Result<(), String> {
    let expected_detours = [
        hook_attributed_writer as usize,
        hook_cause_writer as usize,
        hook_combat_source_writer as usize,
        hook_outer as usize,
    ];
    let mut observed_base = None;
    let mut owner_tokens = BTreeSet::new();
    for ((expected, receipt), expected_detour) in PRODUCTION_HOOK_SPECS
        .iter()
        .zip(receipts)
        .zip(expected_detours)
    {
        let requested_rva = usize::try_from(expected.requested_rva)
            .map_err(|_| format!("{} release RVA does not fit usize", expected.logical_name))?;
        let receipt_base = receipt
            .observed_runtime_address
            .checked_sub(requested_rva)
            .ok_or_else(|| {
                format!(
                    "{} release runtime address underflow",
                    expected.logical_name
                )
            })?;
        if receipt_base == 0
            || observed_base.is_some_and(|base| base != receipt_base)
            || !owner_tokens.insert(receipt.teardown_owner_token)
            || !receipt.released
            || receipt.logical_name != expected.logical_name
            || receipt.observed_module_name != "hoi4.exe"
            || receipt.observed_module_sha256 != production_hook_receipt::A729_EXE_SHA256
            || receipt.observed_pe_identity_sha256 == [0; 32]
            || receipt.observed_pe_timestamp == 0
            || receipt.observed_size_of_image == 0
            || receipt.observed_rva != expected.requested_rva
            || receipt.target_patch_len != expected.target_patch_len
            || receipt.primitive_id != expected.primitive_id
            || receipt.hook_spec_sha256 == [0; 32]
            || receipt.original_prefix_len != expected.original_prefix.len() as u16
            || receipt.original_prefix_sha256 == [0; 32]
            || receipt.restored_prefix_sha256 != receipt.original_prefix_sha256
            || receipt.observed_relay_address == 0
            || receipt.observed_detour_address != expected_detour
            || receipt.teardown_owner_token == 0
            || !receipt.callbacks_quiesced
            || !receipt.hook_removed
            || !receipt.relay_freed
        {
            return Err(format!(
                "{} terminal release receipt is incomplete",
                expected.logical_name
            ));
        }
        observed_base = Some(receipt_base);
    }
    Ok(())
}

pub fn publish_complete_baseline(events: Vec<crate::SunkShipEvent>) -> Result<(), String> {
    if !crate::PRODUCTION_SCHEDULING_ENABLED {
        return Err("A33 production scheduling is disabled".into());
    }
    if !INSTALLED.load(Ordering::Acquire)
        || INSTALL_PHASE.load(Ordering::Acquire) != INSTALL_PHASE_ACTIVE
        || INSTALL_MODE.load(Ordering::Acquire) != INSTALL_MODE_PRODUCTION
        || EVIDENCE_INSTALL.load(Ordering::Acquire)
        || !BASELINE_COMPLETE.load(Ordering::Acquire)
        || !BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire)
    {
        return Err(
            "A33 complete baseline requires an installed, coherently revalidated production hook"
                .into(),
        );
    }
    let producer_loss = producer_loss_since_install();
    let snapshot = crate::SunkShipSnapshot {
        source_sequence: events.last().map_or(1, |event| event.event_sequence),
        producer_complete: producer_loss.is_none(),
        records_with_unavailable_text: crate::records_with_unavailable_text(&events),
        producer_hour_raw: producer_hour_raw(),
        producer_loss,
        events,
    };
    crate::validate_snapshot(&snapshot)
        .map_err(|error| format!("A33 baseline invalid: {error:?}"))?;
    let history = HISTORY
        .get_or_init(|| Mutex::new(HistoryState::default()))
        .lock()
        .map_err(|_| "A33 history lock poisoned")?;
    if !history.baseline_complete || history.events != snapshot.events {
        return Err("A33 supplied baseline does not match the authenticated live vector".into());
    }
    Ok(())
}

pub fn drain_before_member_epoch_republish() -> Result<(), String> {
    if !crate::PRODUCTION_SCHEDULING_ENABLED || EVIDENCE_INSTALL.load(Ordering::Acquire) {
        return Ok(());
    }
    let reservation = peek_complete_snapshot().map_err(|refusal| refusal.message)?;
    commit_drain_to_history(reservation.reservation_id)
}

fn capture_to_event(capture: RawSunkShipCapture) -> crate::SunkShipEvent {
    crate::SunkShipEvent {
        event_sequence: capture.event_sequence,
        combat_join_present: capture.combat_join_present,
        combat_id: capture.combat_id,
        combat_generation: capture.combat_generation,
        victim_country_id_raw: capture.victim_country_id_raw,
        attacker_country_id_raw: capture.attacker_country_id_raw,
        ui_source_country_id_raw: capture.ui_source_country_id_raw,
        sink_date_raw: capture.sink_date_raw,
        victim_member_battle_slot_index: capture.victim_member_battle_slot_index,
        flags: if capture.assisted_kill {
            crate::EVENT_FLAG_ASSISTED_KILL
        } else {
            crate::EVENT_FLAG_KILLED
        },
        victim_ship_reference_raw: capture.victim_ship_reference_raw,
        damage_source_reference_raw: capture.damage_source_reference_raw,
        task_force_id_raw: capture.task_force_id_raw,
        terminal_context_raw: capture.terminal_context_raw,
        cause_record_flag: capture.cause_record_flag,
        ship_name: capture.ship_name,
        ship_class_name: capture.ship_class_name,
        attribution_source_name: capture.attribution_source_name,
        attribution_source_class_name: capture.attribution_source_class_name,
    }
}

fn rollback_drain_error(reservation_id: u64, primary: String) -> String {
    match rollback_drain(reservation_id) {
        Ok(()) => primary,
        Err(rollback) => format!("{primary}; drain rollback failed: {rollback}"),
    }
}

pub fn current_complete_snapshot() -> Result<crate::SunkShipSnapshot, String> {
    if !INSTALLED.load(Ordering::Acquire)
        || INSTALL_PHASE.load(Ordering::Acquire) != INSTALL_PHASE_ACTIVE
        || INSTALL_MODE.load(Ordering::Acquire) != INSTALL_MODE_PRODUCTION
        || EVIDENCE_INSTALL.load(Ordering::Acquire)
        || !BASELINE_COMPLETE.load(Ordering::Acquire)
        || !BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire)
    {
        return Err(
            "A33 production hook is not installed with a complete revalidated baseline".into(),
        );
    }
    let history = HISTORY
        .get_or_init(|| Mutex::new(HistoryState::default()))
        .lock()
        .map_err(|_| "A33 history lock poisoned")?;
    if !history.baseline_complete {
        return Err("A33 complete history is unavailable".into());
    }
    let producer_loss = producer_loss_since_install();
    Ok(crate::SunkShipSnapshot {
        source_sequence: history
            .events
            .last()
            .map_or(1, |event| event.event_sequence),
        producer_complete: producer_loss.is_none(),
        records_with_unavailable_text: crate::records_with_unavailable_text(&history.events),
        producer_hour_raw: producer_hour_raw(),
        producer_loss,
        events: history.events.clone(),
    })
}

pub fn peek_complete_snapshot() -> Result<CompleteSnapshotReservation, ReservationRefusal> {
    let reservation = peek_drain().map_err(ReservationRefusal::unclassified)?;
    let reservation_id = reservation.reservation_id;
    let result = (|| -> Result<CompleteSnapshotReservation, ReservationRefusal> {
        if let Some(snapshot) = sealed_complete_snapshot_for(reservation_id)
            .map_err(ReservationRefusal::unclassified)?
        {
            return Ok(CompleteSnapshotReservation {
                reservation_id,
                snapshot,
            });
        }
        admit_history_drain(&reservation.drain)?;
        let snapshot = complete_snapshot_with_raw_extension(&reservation.drain.events)
            .map_err(ReservationRefusal::unclassified)?;
        Ok(CompleteSnapshotReservation {
            reservation_id,
            snapshot,
        })
    })();
    match result {
        Ok(reservation) => Ok(reservation),
        Err(mut refusal) => {
            refusal.message = rollback_drain_error(reservation_id, refusal.message);
            Err(refusal)
        }
    }
}

fn complete_snapshot_with_raw_extension(
    raw_events: &[RawSunkShipCapture],
) -> Result<crate::SunkShipSnapshot, String> {
    let mut snapshot = current_complete_snapshot()?;
    extend_complete_snapshot(&mut snapshot, raw_events)?;
    Ok(snapshot)
}

fn extend_complete_snapshot(
    snapshot: &mut crate::SunkShipSnapshot,
    raw_events: &[RawSunkShipCapture],
) -> Result<(), String> {
    let pending = raw_events
        .iter()
        .cloned()
        .map(capture_to_event)
        .collect::<Vec<_>>();
    validate_history_extension(&snapshot.events, &pending)?;
    snapshot.events.extend(pending);
    snapshot.source_sequence = snapshot
        .events
        .last()
        .map_or(1, |event| event.event_sequence);
    snapshot.records_with_unavailable_text = crate::records_with_unavailable_text(&snapshot.events);
    crate::validate_snapshot(snapshot)
        .map_err(|error| format!("A33 reserved complete snapshot is invalid: {error:?}"))
}

fn sealed_complete_snapshot_for(
    reservation_id: u64,
) -> Result<Option<crate::SunkShipSnapshot>, String> {
    let queue = STATE
        .get_or_init(|| Mutex::new(CaptureQueueState::new()))
        .lock()
        .map_err(|_| "A33 queue lock poisoned")?;
    let reservation = matching_drain_reservation(&queue, reservation_id)?;
    let Some(sealed) = queue.sealed_complete_snapshot.as_ref() else {
        return Ok(None);
    };
    if !sealed.reserved_prefix.same_reserved_prefix(reservation) {
        return Err("A33 sealed complete snapshot no longer matches the pending raw prefix".into());
    }
    Ok(Some(sealed.snapshot.clone()))
}

pub fn drain() -> Result<HookDrain, String> {
    let reservation = peek_drain()?;
    if let Err(commit) = commit_drain(reservation.reservation_id) {
        return Err(rollback_drain_error(
            reservation.reservation_id,
            format!("A33 compatibility drain commit failed: {commit}"),
        ));
    }
    Ok(reservation.drain)
}

pub fn peek_drain() -> Result<HookDrainReservation, String> {
    let mut queue = STATE
        .get_or_init(|| Mutex::new(CaptureQueueState::new()))
        .lock()
        .map_err(|_| "A33 queue lock poisoned")?;
    if let Some(active) = queue.reservation {
        return Err(format!(
            "A33 drain reservation {} is already active for {} row(s)",
            active.reservation_id, active.row_count
        ));
    }
    let reservation_id = next_drain_reservation_id()?;
    let completeness = load_completeness_state();
    let state = DrainReservationState {
        reservation_id,
        row_count: queue.pending.len(),
        poisoned: completeness.poisoned,
        dropped_unbound: completeness.dropped_unbound,
        dropped_reentrant: completeness.dropped_reentrant,
        dropped_unreadable: completeness.dropped_unreadable,
        dropped_lock_busy: completeness.dropped_lock_busy,
        dropped_overflow: completeness.dropped_overflow,
        installed: INSTALLED.load(Ordering::Acquire),
        install_phase: INSTALL_PHASE.load(Ordering::Acquire),
        install_mode: INSTALL_MODE.load(Ordering::Acquire),
    };
    let drain = HookDrain {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        events: queue
            .pending
            .iter()
            .take(state.row_count)
            .cloned()
            .collect(),
        dropped_unbound: state.dropped_unbound,
        dropped_reentrant: state.dropped_reentrant,
        dropped_unreadable: state.dropped_unreadable,
        dropped_lock_busy: state.dropped_lock_busy,
        dropped_overflow: state.dropped_overflow,
        completeness_poisoned: state.poisoned,
        baseline_measured_empty: BASELINE_MEASURED_EMPTY.load(Ordering::Acquire),
        baseline_revalidated_empty: BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire),
        baseline_complete: BASELINE_COMPLETE.load(Ordering::Acquire),
        baseline_revalidated_complete: BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire),
        evidence_install: EVIDENCE_INSTALL.load(Ordering::Acquire),
        installed: state.installed,
        install_phase: state.install_phase,
        install_mode: state.install_mode,
    };
    queue.reservation = Some(state);
    Ok(HookDrainReservation {
        reservation_id,
        drain,
    })
}

fn next_drain_reservation_id() -> Result<u64, String> {
    loop {
        let reservation_id = NEXT_DRAIN_RESERVATION_ID.load(Ordering::Acquire);
        if reservation_id == 0 || reservation_id == u64::MAX {
            return Err("A33 drain reservation id exhausted".into());
        }
        if NEXT_DRAIN_RESERVATION_ID
            .compare_exchange_weak(
                reservation_id,
                reservation_id + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            return Ok(reservation_id);
        }
    }
}

pub fn commit_drain(reservation_id: u64) -> Result<(), String> {
    let mut queue = STATE
        .get_or_init(|| Mutex::new(CaptureQueueState::new()))
        .lock()
        .map_err(|_| "A33 queue lock poisoned")?;
    let reservation = matching_drain_reservation(&queue, reservation_id)?;
    if queue
        .sealed_complete_snapshot
        .as_ref()
        .is_some_and(|sealed| !sealed.reserved_prefix.same_reserved_prefix(reservation))
    {
        return Err("A33 sealed complete snapshot does not match the committed raw prefix".into());
    }
    if queue.pending.len() < reservation.row_count {
        return Err("A33 reserved drain prefix was truncated".into());
    }
    for (counter, amount) in [
        (&DROPPED_UNBOUND, reservation.dropped_unbound),
        (&DROPPED_REENTRANT, reservation.dropped_reentrant),
        (&DROPPED_UNREADABLE, reservation.dropped_unreadable),
        (&DROPPED_LOCK_BUSY, reservation.dropped_lock_busy),
        (&DROPPED_OVERFLOW, reservation.dropped_overflow),
    ] {
        if counter.load(Ordering::Acquire) < amount {
            return Err("A33 reserved diagnostic counter regressed".into());
        }
    }
    queue.pending.drain(..reservation.row_count);
    queue.reservation = None;
    queue.sealed_complete_snapshot = None;
    DROPPED_UNBOUND.fetch_sub(reservation.dropped_unbound, Ordering::AcqRel);
    DROPPED_REENTRANT.fetch_sub(reservation.dropped_reentrant, Ordering::AcqRel);
    DROPPED_UNREADABLE.fetch_sub(reservation.dropped_unreadable, Ordering::AcqRel);
    DROPPED_LOCK_BUSY.fetch_sub(reservation.dropped_lock_busy, Ordering::AcqRel);
    DROPPED_OVERFLOW.fetch_sub(reservation.dropped_overflow, Ordering::AcqRel);
    Ok(())
}

pub fn commit_drain_to_history(reservation_id: u64) -> Result<(), String> {
    let mut queue = STATE
        .get_or_init(|| Mutex::new(CaptureQueueState::new()))
        .lock()
        .map_err(|_| "A33 queue lock poisoned")?;
    let reservation = matching_drain_reservation(&queue, reservation_id)?;
    if queue.sealed_complete_snapshot.is_some() {
        return Err(
            "A33 sealed terminal snapshot must use terminal raw-prefix commit custody".into(),
        );
    }
    if queue.pending.len() < reservation.row_count {
        return Err("A33 reserved drain prefix was truncated".into());
    }
    let drain = HookDrain {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        events: queue
            .pending
            .iter()
            .take(reservation.row_count)
            .cloned()
            .collect(),
        dropped_unbound: reservation.dropped_unbound,
        dropped_reentrant: reservation.dropped_reentrant,
        dropped_unreadable: reservation.dropped_unreadable,
        dropped_lock_busy: reservation.dropped_lock_busy,
        dropped_overflow: reservation.dropped_overflow,
        completeness_poisoned: reservation.poisoned,
        baseline_measured_empty: BASELINE_MEASURED_EMPTY.load(Ordering::Acquire),
        baseline_revalidated_empty: BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire),
        baseline_complete: BASELINE_COMPLETE.load(Ordering::Acquire),
        baseline_revalidated_complete: BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire),
        evidence_install: EVIDENCE_INSTALL.load(Ordering::Acquire),
        installed: reservation.installed,
        install_phase: reservation.install_phase,
        install_mode: reservation.install_mode,
    };
    admit_history_drain(&drain).map_err(|refusal| refusal.message)?;
    let events = drain
        .events
        .into_iter()
        .map(capture_to_event)
        .collect::<Vec<_>>();
    let mut history = HISTORY
        .get_or_init(|| Mutex::new(HistoryState::default()))
        .lock()
        .map_err(|_| "A33 history lock poisoned")?;
    if !history.baseline_complete {
        return Err("A33 complete-history baseline is absent".into());
    }
    validate_history_extension(&history.events, &events)?;
    for (counter, amount) in [
        (&DROPPED_UNBOUND, reservation.dropped_unbound),
        (&DROPPED_REENTRANT, reservation.dropped_reentrant),
        (&DROPPED_UNREADABLE, reservation.dropped_unreadable),
        (&DROPPED_LOCK_BUSY, reservation.dropped_lock_busy),
        (&DROPPED_OVERFLOW, reservation.dropped_overflow),
    ] {
        if counter.load(Ordering::Acquire) < amount {
            return Err("A33 reserved diagnostic counter regressed".into());
        }
    }

    queue.pending.drain(..reservation.row_count);
    queue.reservation = None;
    DROPPED_UNBOUND.fetch_sub(reservation.dropped_unbound, Ordering::AcqRel);
    DROPPED_REENTRANT.fetch_sub(reservation.dropped_reentrant, Ordering::AcqRel);
    DROPPED_UNREADABLE.fetch_sub(reservation.dropped_unreadable, Ordering::AcqRel);
    DROPPED_LOCK_BUSY.fetch_sub(reservation.dropped_lock_busy, Ordering::AcqRel);
    DROPPED_OVERFLOW.fetch_sub(reservation.dropped_overflow, Ordering::AcqRel);
    history.events.extend(events);
    Ok(())
}

fn complete_history_drain_refusal(drain: &HookDrain) -> Option<CompleteHistoryRefusal> {
    if drain.evidence_install {
        return Some(CompleteHistoryRefusal::new(
            CompleteHistoryRefusalCode::EvidenceInstall,
            "A33 drain came from an evidence-mode install, which never carries production \
             complete-history authority"
                .into(),
        ));
    }
    if !drain.baseline_complete {
        return Some(CompleteHistoryRefusal::new(
            CompleteHistoryRefusalCode::BaselineIncomplete,
            format!(
                "A33 install did not establish a complete history baseline \
                 (at reservation: installed={}, phase={}, mode={})",
                drain.installed, drain.install_phase, drain.install_mode,
            ),
        ));
    }
    if !drain.baseline_revalidated_complete {
        return Some(CompleteHistoryRefusal::new(
            CompleteHistoryRefusalCode::BaselineNotRevalidated,
            "A33 history baseline was not revalidated identical across hook activation".into(),
        ));
    }
    let losses = [
        (
            CompleteHistoryRefusalCode::DroppedUnbound,
            "unbound",
            drain.dropped_unbound,
        ),
        (
            CompleteHistoryRefusalCode::DroppedReentrant,
            "reentrant",
            drain.dropped_reentrant,
        ),
        (
            CompleteHistoryRefusalCode::DroppedUnreadable,
            "unreadable",
            drain.dropped_unreadable,
        ),
        (
            CompleteHistoryRefusalCode::DroppedLockBusy,
            "lock_busy",
            drain.dropped_lock_busy,
        ),
        (
            CompleteHistoryRefusalCode::DroppedOverflow,
            "overflow",
            drain.dropped_overflow,
        ),
    ];
    if let Some((code, _, _)) = losses.iter().find(|(_, _, count)| *count != 0) {
        let detail = losses
            .iter()
            .map(|(_, name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut mask = 0u32;
        let mut total = 0u64;
        for (index, (_, _, count)) in losses.iter().enumerate() {
            if *count != 0 {
                mask |= 1 << index;
            }
            total = total.saturating_add(*count);
        }
        return Some(
            CompleteHistoryRefusal::new(
                *code,
                format!(
                    "A33 hook lost {total} callback(s) since install, so the history is no \
                     longer complete ({detail})"
                ),
            )
            .with_losses(mask, total),
        );
    }
    if drain.completeness_poisoned {
        return Some(CompleteHistoryRefusal::new(
            CompleteHistoryRefusalCode::CompletenessPoisoned,
            "A33 completeness was poisoned by a non-increasing event sequence".into(),
        ));
    }
    None
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompleteHistoryRefusalCode {
    EvidenceInstall = 1,
    BaselineIncomplete = 2,
    BaselineNotRevalidated = 3,
    DroppedUnbound = 4,
    DroppedReentrant = 5,
    DroppedUnreadable = 6,
    DroppedLockBusy = 7,
    DroppedOverflow = 8,
    CompletenessPoisoned = 9,
}

impl CompleteHistoryRefusalCode {
    fn denies_authority(self) -> bool {
        match self {
            Self::EvidenceInstall | Self::BaselineIncomplete | Self::BaselineNotRevalidated => true,
            Self::DroppedUnbound
            | Self::DroppedReentrant
            | Self::DroppedUnreadable
            | Self::DroppedLockBusy
            | Self::DroppedOverflow
            | Self::CompletenessPoisoned => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteHistoryRefusal {
    pub code: CompleteHistoryRefusalCode,
    pub message: String,
    pub drop_mask: u32,
    pub drop_total: u64,
}

impl CompleteHistoryRefusal {
    fn new(code: CompleteHistoryRefusalCode, message: String) -> Self {
        Self {
            code,
            message,
            drop_mask: 0,
            drop_total: 0,
        }
    }

    fn with_losses(mut self, drop_mask: u32, drop_total: u64) -> Self {
        self.drop_mask = drop_mask;
        self.drop_total = drop_total;
        self
    }
}

impl fmt::Display for CompleteHistoryRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservationRefusal {
    pub code: u32,
    pub drop_mask: u32,
    pub drop_total: u64,
    pub message: String,
}

impl ReservationRefusal {
    fn unclassified(message: String) -> Self {
        Self {
            code: 0,
            drop_mask: 0,
            drop_total: 0,
            message,
        }
    }
}

impl From<CompleteHistoryRefusal> for ReservationRefusal {
    fn from(refusal: CompleteHistoryRefusal) -> Self {
        Self {
            code: refusal.code as u32,
            drop_mask: refusal.drop_mask,
            drop_total: refusal.drop_total,
            message: refusal.message,
        }
    }
}

impl From<String> for ReservationRefusal {
    fn from(message: String) -> Self {
        Self::unclassified(message)
    }
}

impl fmt::Display for ReservationRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn admit_history_drain(drain: &HookDrain) -> Result<(), CompleteHistoryRefusal> {
    match complete_history_drain_refusal(drain) {
        Some(refusal) if refusal.code.denies_authority() => Err(refusal),
        _ => Ok(()),
    }
}

fn validate_history_extension(
    history: &[crate::SunkShipEvent],
    events: &[crate::SunkShipEvent],
) -> Result<(), String> {
    let prior_sequence = history.last().map(|event| event.event_sequence);
    if prior_sequence
        .zip(events.first())
        .is_some_and(|(prior, first)| first.event_sequence <= prior)
        || events
            .windows(2)
            .any(|pair| pair[1].event_sequence <= pair[0].event_sequence)
    {
        SEQUENCE_REGRESSION_SINCE_INSTALL.store(true, Ordering::Release);
        COMPLETENESS_POISONED.store(true, Ordering::Release);
        return Err("A33 event sequence is not strictly increasing".into());
    }
    Ok(())
}

pub fn rollback_drain(reservation_id: u64) -> Result<(), String> {
    let mut queue = STATE
        .get_or_init(|| Mutex::new(CaptureQueueState::new()))
        .lock()
        .map_err(|_| "A33 queue lock poisoned")?;
    let reservation = matching_drain_reservation(&queue, reservation_id)?;
    if queue
        .sealed_complete_snapshot
        .as_ref()
        .is_some_and(|sealed| !sealed.reserved_prefix.same_reserved_prefix(reservation))
    {
        return Err(
            "A33 sealed complete snapshot does not match the rolled-back raw prefix".into(),
        );
    }
    queue.reservation = None;
    Ok(())
}

fn matching_drain_reservation(
    queue: &CaptureQueueState,
    reservation_id: u64,
) -> Result<DrainReservationState, String> {
    match queue.reservation {
        Some(reservation) if reservation.reservation_id == reservation_id => Ok(reservation),
        Some(reservation) => Err(format!(
            "A33 drain reservation mismatch: active={}, supplied={reservation_id}",
            reservation.reservation_id
        )),
        None => Err(format!(
            "A33 drain reservation {reservation_id} is not active"
        )),
    }
}

fn clone_active_drain_reservation(reservation_id: u64) -> Result<HookDrainReservation, String> {
    let queue = STATE
        .get_or_init(|| Mutex::new(CaptureQueueState::new()))
        .lock()
        .map_err(|_| "A33 queue lock poisoned")?;
    let reservation = matching_drain_reservation(&queue, reservation_id)?;
    if queue.pending.len() < reservation.row_count {
        return Err("A33 reserved drain prefix was truncated".into());
    }
    Ok(HookDrainReservation {
        reservation_id,
        drain: HookDrain {
            hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
            events: queue
                .pending
                .iter()
                .take(reservation.row_count)
                .cloned()
                .collect(),
            dropped_unbound: reservation.dropped_unbound,
            dropped_reentrant: reservation.dropped_reentrant,
            dropped_unreadable: reservation.dropped_unreadable,
            dropped_lock_busy: reservation.dropped_lock_busy,
            dropped_overflow: reservation.dropped_overflow,
            completeness_poisoned: reservation.poisoned,
            baseline_measured_empty: BASELINE_MEASURED_EMPTY.load(Ordering::Acquire),
            baseline_revalidated_empty: BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire),
            baseline_complete: BASELINE_COMPLETE.load(Ordering::Acquire),
            baseline_revalidated_complete: BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire),
            evidence_install: EVIDENCE_INSTALL.load(Ordering::Acquire),
            installed: reservation.installed,
            install_phase: reservation.install_phase,
            install_mode: reservation.install_mode,
        },
    })
}

pub fn seal_drain_and_uninstall() -> Result<HookDrain, String> {
    let reservation = seal_reserve_and_uninstall()?;
    let reservation_id = reservation.reservation_id;
    if let Err(error) = commit_drain(reservation_id) {
        return Err(rollback_drain_error(
            reservation_id,
            format!("A33 compatibility sealed-drain commit failed: {error}"),
        ));
    }
    Ok(reservation.drain)
}

pub fn seal_reserve_and_uninstall() -> Result<HookDrainReservation, String> {
    seal_reserve_and_uninstall_inner(None, None)
}

/// forever. Physical restoration, quiescence and release-receipt observation
pub fn abort_partial_install_and_uninstall() -> Result<InstallAbortDiscard, String> {
    let reservation = seal_reserve_and_uninstall()?;
    let discard = InstallAbortDiscard {
        retained_rows: reservation.drain.events.len() as u64,
        reserved_rows: reservation.drain.events.len() as u64,
        discarded_diagnostics: reservation
            .drain
            .dropped_unbound
            .saturating_add(reservation.drain.dropped_reentrant)
            .saturating_add(reservation.drain.dropped_unreadable)
            .saturating_add(reservation.drain.dropped_lock_busy)
            .saturating_add(reservation.drain.dropped_overflow),
        poisoned: reservation.drain.completeness_poisoned,
    };
    if let Err(error) = commit_drain(reservation.reservation_id) {
        return Err(rollback_drain_error(
            reservation.reservation_id,
            format!("A33 install abort discard failed: {error}"),
        ));
    }
    Ok(discard)
}

pub fn seal_reserved_drain_and_uninstall(reservation_id: u64) -> Result<(), String> {
    let expected_snapshot = complete_snapshot_for_active_reservation(reservation_id)?;
    let reservation =
        seal_reserve_and_uninstall_inner(Some(reservation_id), Some(&expected_snapshot))?;
    if reservation.reservation_id != reservation_id {
        return Err(rollback_drain_error(
            reservation.reservation_id,
            "A33 sealed reservation identity changed".into(),
        ));
    }
    Ok(())
}

fn complete_snapshot_for_active_reservation(
    reservation_id: u64,
) -> Result<crate::SunkShipSnapshot, String> {
    if let Some(snapshot) = sealed_complete_snapshot_for(reservation_id)? {
        return Ok(snapshot);
    }
    let reservation = clone_active_drain_reservation(reservation_id)?;
    admit_history_drain(&reservation.drain).map_err(|refusal| refusal.message)?;
    complete_snapshot_with_raw_extension(&reservation.drain.events)
}

fn persist_sealed_complete_snapshot(
    reservation_id: u64,
    expected_snapshot: &crate::SunkShipSnapshot,
) -> Result<bool, String> {
    let (reserved_prefix, raw_events) = {
        let queue = STATE
            .get_or_init(|| Mutex::new(CaptureQueueState::new()))
            .lock()
            .map_err(|_| "A33 queue lock poisoned while sealing complete snapshot")?;
        let active = matching_drain_reservation(&queue, reservation_id)?;
        let reserved_prefix = DrainReservationState {
            reservation_id,
            row_count: queue.pending.len(),
            dropped_unbound: DROPPED_UNBOUND.load(Ordering::Acquire),
            dropped_reentrant: DROPPED_REENTRANT.load(Ordering::Acquire),
            dropped_unreadable: DROPPED_UNREADABLE.load(Ordering::Acquire),
            dropped_lock_busy: DROPPED_LOCK_BUSY.load(Ordering::Acquire),
            dropped_overflow: DROPPED_OVERFLOW.load(Ordering::Acquire),
            poisoned: active.poisoned,
            installed: active.installed,
            install_phase: active.install_phase,
            install_mode: active.install_mode,
        };
        if !active.same_reserved_prefix(reserved_prefix) && queue.pending.len() < active.row_count {
            return Err("A33 terminal raw prefix was truncated while hooks quiesced".into());
        }
        (
            reserved_prefix,
            queue.pending.iter().cloned().collect::<Vec<_>>(),
        )
    };
    let mut snapshot = {
        let history = HISTORY
            .get_or_init(|| Mutex::new(HistoryState::default()))
            .lock()
            .map_err(|_| "A33 history lock poisoned while sealing complete snapshot")?;
        if !history.baseline_complete {
            return Err("A33 complete-history baseline is absent while sealing".into());
        }
        let producer_loss = producer_loss_since_install();
        crate::SunkShipSnapshot {
            source_sequence: history
                .events
                .last()
                .map_or(1, |event| event.event_sequence),
            producer_complete: producer_loss.is_none(),
            records_with_unavailable_text: crate::records_with_unavailable_text(&history.events),
            producer_hour_raw: producer_hour_raw(),
            producer_loss,
            events: history.events.clone(),
        }
    };
    extend_complete_snapshot(&mut snapshot, &raw_events)?;
    let snapshot_changed = snapshot != *expected_snapshot;
    let mut queue = STATE
        .get_or_init(|| Mutex::new(CaptureQueueState::new()))
        .lock()
        .map_err(|_| "A33 queue lock poisoned while persisting sealed snapshot")?;
    let active = matching_drain_reservation(&queue, reservation_id)?;
    if queue.pending.len() != reserved_prefix.row_count
        || !active.same_reserved_prefix(DrainReservationState {
            row_count: active.row_count,
            ..reserved_prefix
        })
    {
        return Err("A33 terminal raw prefix changed while sealing snapshot custody".into());
    }
    queue.reservation = Some(reserved_prefix);
    queue.sealed_complete_snapshot = Some(SealedCompleteSnapshotState {
        reserved_prefix,
        snapshot,
    });
    Ok(snapshot_changed)
}

fn seal_reserve_and_uninstall_inner(
    existing_reservation_id: Option<u64>,
    expected_complete_snapshot: Option<&crate::SunkShipSnapshot>,
) -> Result<HookDrainReservation, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A33 teardown lock poisoned")?;
    let install_phase = INSTALL_PHASE.load(Ordering::Acquire);
    if install_phase == INSTALL_PHASE_NONE {
        let reservation = match existing_reservation_id {
            Some(reservation_id) => clone_active_drain_reservation(reservation_id),
            None => peek_drain(),
        }?;
        naval_combat_live_lane::clear_member_epoch_snapshot()?;
        return Ok(reservation);
    }
    if install_phase != INSTALL_PHASE_ACTIVATING
        && install_phase != INSTALL_PHASE_ACTIVE
        && install_phase != INSTALL_PHASE_RESTORE_REQUIRED
    {
        return Err("A33 teardown found an invalid install phase".into());
    }
    let install_mode = INSTALL_MODE.load(Ordering::Acquire);
    if install_mode != INSTALL_MODE_PRODUCTION && install_mode != INSTALL_MODE_EVIDENCE {
        return Err("A33 teardown found an invalid install mode".into());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("A33 teardown lost its module-base custody".into());
    }
    INSTALLED.store(false, Ordering::Release);
    let outer = base + OUTER_CFEX_MEMBER_WRITER_RVA;
    let attributed_writer = base + ATTRIBUTED_RECORD_WRITER_RVA;
    let combat_source_writer = base + COMBAT_SOURCE_RECORD_WRITER_RVA;
    let cause_writer = base + CAUSE_RECORD_WRITER_RVA;
    let enabled = ENABLED_HOOKS.load(Ordering::Acquire);
    if enabled & HOOK_OUTER != 0 {
        disable(outer)?;
    }
    wait_for_callbacks()?;
    let all_targets = [outer, attributed_writer, combat_source_writer, cause_writer];
    let all_batch = hook_batch(&all_targets);
    let writers: Vec<_> = all_batch[1..]
        .iter()
        .copied()
        .zip([HOOK_ATTRIBUTED, HOOK_COMBAT_SOURCE, HOOK_CAUSE])
        .filter(|(_, hook_bit)| enabled & hook_bit != 0)
        .map(|(target, _)| target)
        .collect();
    if let Some((name, status)) = unsafe { inline_detour::disable_batch(&writers) }.first_failure()
    {
        return Err(format!("A33 disable failed for {name}: {status}"));
    }
    wait_for_callbacks()?;
    let reservation = match existing_reservation_id {
        Some(reservation_id) => clone_active_drain_reservation(reservation_id)?,
        None => peek_drain()?,
    };
    if let Some(expected_snapshot) = expected_complete_snapshot {
        if persist_sealed_complete_snapshot(reservation.reservation_id, expected_snapshot)? {
            return Err(rollback_drain_error(
                reservation.reservation_id,
                "A33 terminal snapshot changed while callbacks were quiescing; retry the durable sealed snapshot"
                    .into(),
            ));
        }
    }
    let teardown = (|| {
        let created = CREATED_HOOKS.load(Ordering::Acquire);
        let removal: Vec<_> = all_batch
            .iter()
            .rev()
            .copied()
            .zip([HOOK_CAUSE, HOOK_COMBAT_SOURCE, HOOK_ATTRIBUTED, HOOK_OUTER])
            .filter(|(_, hook_bit)| created & hook_bit != 0)
            .map(|(target, _)| target)
            .collect();
        if let Some((name, status)) =
            unsafe { inline_detour::remove_batch(&removal) }.first_failure()
        {
            return Err(format!("A33 remove failed for {name}: {status}"));
        }
        verify_all_sentinels(base)?;
        if install_mode == INSTALL_MODE_PRODUCTION {
            release_and_publish_production_hook_receipts(install_phase == INSTALL_PHASE_ACTIVE)?;
        }
        INSTALLED.store(false, Ordering::Release);
        clear_hook_custody();
        naval_combat_live_lane::clear_member_epoch_snapshot()?;
        Ok(())
    })();
    if let Err(error) = teardown {
        return Err(rollback_drain_error(reservation.reservation_id, error));
    }
    Ok(reservation)
}

fn disable(target: usize) -> Result<(), String> {
    match unsafe { inline_detour::disable(target as *mut c_void) } {
        Ok(())
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => Ok(()),
        Err(status) => Err(format!("A33 disable failed: {status}")),
    }
}

fn wait_for_callbacks() -> Result<(), String> {
    let mut yields = 0u32;
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("A33 callbacks did not quiesce".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    Ok(())
}

fn cleanup_after_failed_install(base: usize, targets: &[usize]) -> Result<(), String> {
    if let Some((name, status)) =
        unsafe { inline_detour::disable_batch(&hook_batch(targets)) }.first_failure()
    {
        return Err(format!("A33 disable failed for {name}: {status}"));
    }
    wait_for_callbacks()?;
    cleanup_created_hooks(base, targets)
}

fn cleanup_after_failed_install_and_release(base: usize, targets: &[usize]) -> Result<(), String> {
    cleanup_after_failed_install(base, targets)?;
    release_persisted_hook_receipts(false, false)
}

fn finish_failed_activation(primary: String, cleanup: Result<(), String>) -> String {
    INSTALLED.store(false, Ordering::Release);
    match cleanup {
        Ok(()) => {
            clear_hook_custody();
            primary
        }
        Err(cleanup) => {
            INSTALL_PHASE.store(INSTALL_PHASE_RESTORE_REQUIRED, Ordering::Release);
            format!("{primary}; teardown required: {cleanup}")
        }
    }
}

fn clear_hook_custody() {
    reset_originals();
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    EVIDENCE_INSTALL.store(false, Ordering::Release);
    BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
    BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
    BASELINE_COMPLETE.store(false, Ordering::Release);
    BASELINE_REVALIDATED_COMPLETE.store(false, Ordering::Release);
    ENABLED_HOOKS.store(0, Ordering::Release);
    CREATED_HOOKS.store(0, Ordering::Release);
    INSTALL_MODE.store(INSTALL_MODE_NONE, Ordering::Release);
    INSTALL_PHASE.store(INSTALL_PHASE_NONE, Ordering::Release);
}

fn cleanup_created_hooks(base: usize, targets: &[usize]) -> Result<(), String> {
    let mut failures = Vec::new();
    let mut batch = hook_batch(targets);
    batch.reverse();
    failures.extend(
        unsafe { inline_detour::remove_batch(&batch) }
            .failures()
            .into_iter()
            .map(|(name, status)| format!("remove {name} failed: {status}")),
    );
    if let Err(error) = verify_all_sentinels(base) {
        failures.push(error);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "A33 rollback restoration failed: {}",
            failures.join("; ")
        ))
    }
}

fn observe_and_persist_new_production_hook_receipts(base: usize) -> Result<(), String> {
    for index in 0..PRODUCTION_HOOK_SPECS.len() {
        let existing = PRODUCTION_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 production hook receipt lock poisoned during observation")?[index];
        if let Some(existing) = existing {
            let observed =
                observe_production_hook_receipt(base, index, existing.teardown_owner_token)?;
            if observed != existing {
                return Err(format!(
                    "A33 installed hook receipt {index} changed during retry"
                ));
            }
            continue;
        }
        if RELEASED_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 released hook receipt lock poisoned during observation")?[index]
            .is_some()
        {
            return Err(format!(
                "A33 hook receipt {index} was already released during install"
            ));
        }
        let observed = observe_production_hook_receipt(base, index, next_teardown_owner_token()?)?;
        let mut installed = PRODUCTION_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 production hook receipt lock poisoned during observation")?;
        if installed[index].is_some() {
            return Err(format!(
                "A33 installed hook receipt {index} changed before publication"
            ));
        }
        installed[index] = Some(observed);
    }
    complete_installed_hook_receipts().map(|_| ())
}

fn observe_production_hook_receipt(
    base: usize,
    index: usize,
    owner_token: u64,
) -> Result<InstalledRvaHookReceipt, String> {
    let expected = *PRODUCTION_HOOK_SPECS
        .get(index)
        .ok_or("A33 production hook receipt index is invalid")?;
    let detour = match index {
        0 => hook_attributed_writer as usize,
        1 => hook_cause_writer as usize,
        2 => hook_combat_source_writer as usize,
        3 => hook_outer as usize,
        _ => return Err("A33 production hook detour index is invalid".into()),
    };
    observe_enabled_minhook_rel5(expected, hook_target(base, expected)?, detour, owner_token)
}

fn reobserve_production_hook_receipts(
    base: usize,
    installed: [InstalledRvaHookReceipt; 4],
) -> Result<[InstalledRvaHookReceipt; 4], String> {
    Ok([
        observe_production_hook_receipt(base, 0, installed[0].teardown_owner_token)?,
        observe_production_hook_receipt(base, 1, installed[1].teardown_owner_token)?,
        observe_production_hook_receipt(base, 2, installed[2].teardown_owner_token)?,
        observe_production_hook_receipt(base, 3, installed[3].teardown_owner_token)?,
    ])
}

fn complete_installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; 4], String> {
    let installed = PRODUCTION_HOOK_RECEIPTS
        .lock()
        .map_err(|_| "A33 production hook receipt lock poisoned")?;
    Ok([
        installed[0].ok_or("A33 installed hook receipt 0 is absent")?,
        installed[1].ok_or("A33 installed hook receipt 1 is absent")?,
        installed[2].ok_or("A33 installed hook receipt 2 is absent")?,
        installed[3].ok_or("A33 installed hook receipt 3 is absent")?,
    ])
}

fn release_persisted_hook_receipts(
    require_complete: bool,
    retain_complete_releases: bool,
) -> Result<(), String> {
    let teardown = HookTeardownAttestation {
        callbacks_quiesced: true,
        hook_removed: true,
        relay_freed: true,
    };
    if require_complete {
        let installed = complete_installed_hook_receipts()?;
        {
            let released = RELEASED_HOOK_RECEIPTS
                .lock()
                .map_err(|_| "A33 released hook receipt lock poisoned during teardown")?;
            if released.iter().any(Option::is_some) {
                return Err("A33 complete teardown found partial local release custody".into());
            }
        }
        let released = observe_released_set(PRODUCTION_HOOK_SPECS, installed, [teardown; 4])?;
        let mut installed_slot = PRODUCTION_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 production hook receipt lock poisoned during teardown")?;
        let mut released_slot = RELEASED_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 released hook receipt lock poisoned during teardown")?;
        if *installed_slot
            != [
                Some(installed[0]),
                Some(installed[1]),
                Some(installed[2]),
                Some(installed[3]),
            ]
            || released_slot.iter().any(Option::is_some)
        {
            return Err("A33 complete hook receipt custody changed during release".into());
        }
        *released_slot = [
            Some(released[0]),
            Some(released[1]),
            Some(released[2]),
            Some(released[3]),
        ];
        *installed_slot = [None; 4];
        return Ok(());
    }
    for index in 0..PRODUCTION_HOOK_SPECS.len() {
        let installed = PRODUCTION_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 production hook receipt lock poisoned during teardown")?[index];
        let released = RELEASED_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 released hook receipt lock poisoned during teardown")?[index];
        if released.is_some() {
            if installed.is_some() {
                return Err(format!(
                    "A33 hook receipt {index} is both installed and released"
                ));
            }
            continue;
        }
        let Some(installed) = installed else {
            if require_complete {
                return Err(format!("A33 teardown lost installed hook receipt {index}"));
            }
            continue;
        };
        let released = observe_released(PRODUCTION_HOOK_SPECS[index], installed, teardown)?;
        let mut installed_slot = PRODUCTION_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 production hook receipt lock poisoned during teardown")?;
        let mut released_slot = RELEASED_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 released hook receipt lock poisoned during teardown")?;
        if installed_slot[index] != Some(installed) || released_slot[index].is_some() {
            return Err(format!(
                "A33 hook receipt {index} custody changed during release"
            ));
        }
        released_slot[index] = Some(released);
        installed_slot[index] = None;
    }
    if !retain_complete_releases {
        if PRODUCTION_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 production hook receipt lock poisoned during rollback")?
            .iter()
            .any(Option::is_some)
        {
            return Err("A33 rollback retains installed hook receipt custody".into());
        }
        *RELEASED_HOOK_RECEIPTS
            .lock()
            .map_err(|_| "A33 released hook receipt lock poisoned during rollback")? = [None; 4];
    }
    Ok(())
}

fn release_and_publish_production_hook_receipts(required: bool) -> Result<(), String> {
    release_persisted_hook_receipts(required, required)
}

fn hook_target(base: usize, expected: ExpectedRvaHook) -> Result<usize, String> {
    let rva = usize::try_from(expected.requested_rva)
        .map_err(|_| format!("{} RVA does not fit usize", expected.logical_name))?;
    base.checked_add(rva)
        .ok_or_else(|| format!("{} runtime address overflow", expected.logical_name))
}

fn reset_originals() {
    ORIGINAL_OUTER.store(0, Ordering::Release);
    ORIGINAL_ATTRIBUTED_WRITER.store(0, Ordering::Release);
    ORIGINAL_COMBAT_SOURCE_WRITER.store(0, Ordering::Release);
    ORIGINAL_CAUSE_WRITER.store(0, Ordering::Release);
}

unsafe extern "system" fn hook_outer(a: usize, b: usize, c: usize, d: usize, e: u8) {
    let original: OuterFn = mem::transmute(ORIGINAL_OUTER.load(Ordering::Acquire));
    let _active = CallbackGuard::enter();
    let snapshot = naval_combat_live_lane::try_current_member_epoch_snapshot();
    let resolution = match snapshot {
        None => {
            UNBOUND_EPOCH_SNAPSHOT_ABSENT.fetch_add(1, Ordering::Relaxed);
            None
        }
        Some(snapshot) if !member_epoch_snapshot_is_published(snapshot.receipt()) => {
            UNBOUND_EPOCH_SNAPSHOT_ABSENT.fetch_add(1, Ordering::Relaxed);
            None
        }
        Some(snapshot) => {
            let receipt = snapshot.receipt();
            match snapshot.resolve_member(a) {
                Ok(resolved) => Some((resolved, receipt)),
                Err(_) => {
                    UNBOUND_MEMBER_UNRESOLVED.fetch_add(1, Ordering::Relaxed);
                    capture_first_member_unresolved(
                        a as u64,
                        receipt.publication_sequence,
                        u64::from(receipt.member_bindings),
                        i64::from(receipt.game_hour_raw),
                        live_game_hour_raw(),
                    );
                    None
                }
            }
        }
    };
    if resolution.is_none() {
        poison(LOSS_UNBOUND);
    }
    let context = resolution.map(|(resolved, receipt)| OuterContext {
        combat_id: resolved.epoch.combat_id,
        combat_generation: resolved.epoch.generation,
        battle_slot_index_raw: resolved.battle_slot_index_raw,
        thread_id: unsafe { GetCurrentThreadId() },
        member_epoch_publication_sequence: receipt.publication_sequence,
        member_epoch_game_hour_raw: receipt.game_hour_raw,
    });
    OUTER_CONTEXT.with(|slot| {
        let prior = slot.replace(context);
        original(a, b, c, d, e);
        slot.set(prior);
    });
}

unsafe extern "system" fn hook_attributed_writer(
    game_state: usize,
    victim: usize,
    attacker: usize,
    task_force: usize,
) {
    let _active = CallbackGuard::enter();
    let before_count = history_vector_count(game_state);
    let original: AttributedWriterFn =
        mem::transmute(ORIGINAL_ATTRIBUTED_WRITER.load(Ordering::Acquire));
    original(game_state, victim, attacker, task_force);
    observe_writer_append(game_state, before_count);
}

unsafe extern "system" fn hook_combat_source_writer(
    game_state: usize,
    victim: usize,
    attacker_country: usize,
    damage_source: usize,
    task_force: usize,
    attacker: usize,
) {
    let _active = CallbackGuard::enter();
    let before_count = history_vector_count(game_state);
    let original: CombatSourceWriterFn =
        mem::transmute(ORIGINAL_COMBAT_SOURCE_WRITER.load(Ordering::Acquire));
    original(
        game_state,
        victim,
        attacker_country,
        damage_source,
        task_force,
        attacker,
    );
    observe_writer_append(game_state, before_count);
}

unsafe extern "system" fn hook_cause_writer(
    game_state: usize,
    victim: usize,
    attacker_country: usize,
    task_force: usize,
    cause: u32,
) {
    let _active = CallbackGuard::enter();
    let before_count = history_vector_count(game_state);
    let original: CauseWriterFn = mem::transmute(ORIGINAL_CAUSE_WRITER.load(Ordering::Acquire));
    original(game_state, victim, attacker_country, task_force, cause);
    observe_writer_append(game_state, before_count);
}

fn resolve_append_bracket(
    context: Option<OuterContext>,
    current_thread_id: u32,
) -> Result<Option<OuterContext>, ()> {
    match context {
        Some(context) if context.thread_id == 0 || context.thread_id != current_thread_id => {
            Err(())
        }
        context => Ok(context),
    }
}

fn observe_writer_append(game_state: usize, before_count: Option<u32>) {
    let Ok(context) = resolve_append_bracket(OUTER_CONTEXT.with(Cell::get), unsafe {
        GetCurrentThreadId()
    }) else {
        UNBOUND_BRACKET_THREAD_MISMATCH.fetch_add(1, Ordering::Relaxed);
        poison(LOSS_UNBOUND);
        return;
    };
    let singleton = MODULE_BASE
        .load(Ordering::Acquire)
        .checked_add(CGAMESTATE_SINGLETON_RVA)
        .and_then(read_value::<usize>);
    if singleton != Some(game_state) {
        UNBOUND_GAMESTATE_MISMATCH.fetch_add(1, Ordering::Relaxed);
        poison(LOSS_UNBOUND);
        return;
    }
    let nested = IN_CALLBACK.with(|flag| flag.replace(true));
    if nested {
        poison(LOSS_REENTRANT);
        return;
    }
    let appended_record =
        before_count.and_then(|count| history_vector_appended_record(game_state, count));
    if let Some(capture) = appended_record.and_then(|record| capture_record(record, context)) {
        if let Some(state) = STATE.get() {
            match state.try_lock() {
                Ok(mut queue) if queue.pending.len() < MAX_PENDING_EVENTS => {
                    queue.pending.push_back(capture)
                }
                Ok(_) => poison(LOSS_OVERFLOW),
                Err(_) => poison(LOSS_LOCK_BUSY),
            }
        } else {
            poison(LOSS_UNREADABLE);
        }
    } else {
        poison(LOSS_UNREADABLE);
    }
    IN_CALLBACK.with(|flag| flag.set(false));
}

fn history_vector_count(game_state: usize) -> Option<u32> {
    if game_state == 0 {
        return None;
    }
    let vector = game_state.checked_add(SUNK_SHIP_HISTORY_VECTOR_OFFSET)?;
    let capacity: u32 = read_value(vector + 0x08)?;
    let count: u32 = read_value(vector + 0x0c)?;
    if count > capacity || capacity > crate::MAX_EVENTS as u32 {
        return None;
    }
    Some(count)
}

fn history_vector_appended_record(game_state: usize, before_count: u32) -> Option<usize> {
    let vector = game_state.checked_add(SUNK_SHIP_HISTORY_VECTOR_OFFSET)?;
    let data: usize = read_value(vector)?;
    let capacity: u32 = read_value(vector + 0x08)?;
    let count: u32 = read_value(vector + 0x0c)?;
    if data == 0
        || count != before_count.checked_add(1)?
        || count > capacity
        || capacity > crate::MAX_EVENTS as u32
    {
        return None;
    }
    let last_offset = usize::try_from(count - 1).ok()?.checked_mul(8)?;
    read_value(data.checked_add(last_offset)?)
}

fn capture_record(address: usize, context: Option<OuterContext>) -> Option<RawSunkShipCapture> {
    if address == 0 {
        return None;
    }
    let vtable: usize = read_value(address)?;
    if vtable != MODULE_BASE.load(Ordering::Acquire) + SUNK_SHIP_INFO_VTABLE_RVA {
        return None;
    }
    let class_ptr: usize = read_value(address + 0x68)?;
    let attribution_class_ptr: usize = read_value(address + 0x70)?;
    let RecordTexts {
        ship_name,
        ship_class_name,
        attribution_source_name,
        attribution_source_class_name,
    } = read_record_texts(address, class_ptr, attribution_class_ptr);
    let sink_date_raw: i32 = read_value(address + SUNK_SHIP_INFO_SINK_DATE_OFFSET)?;
    let game_state: usize = read_value(
        MODULE_BASE
            .load(Ordering::Acquire)
            .checked_add(CGAMESTATE_SINGLETON_RVA)?,
    )?;
    if game_state == 0
        || sink_date_raw
            != read_value::<i32>(game_state.checked_add(CGAMESTATE_CURRENT_DATE_OFFSET)?)?
    {
        return None;
    }
    observe_producer_hour(sink_date_raw);
    let assisted_kill_raw: u8 = read_value(address + 0xa1)?;
    let cause_record_flag_raw: u8 = read_value(address + 0xa0)?;
    if assisted_kill_raw > 1 || cause_record_flag_raw > 1 {
        return None;
    }
    let task_force: usize = read_value(address + 0x90)?;
    let task_force_id_raw = if task_force == 0 {
        0
    } else {
        read_value(task_force.checked_add(0xa4)?)?
    };
    let event_sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if event_sequence == 0 {
        return None;
    }
    Some(RawSunkShipCapture {
        event_sequence,
        combat_join_present: context.is_some(),
        thread_id: match context {
            Some(context) => context.thread_id,
            None => unsafe { GetCurrentThreadId() },
        },
        member_epoch_publication_sequence: context
            .map_or(0, |context| context.member_epoch_publication_sequence),
        member_epoch_game_hour_raw: context.map_or(0, |context| context.member_epoch_game_hour_raw),
        combat_id: context.map_or(0, |context| context.combat_id),
        combat_generation: context.map_or(0, |context| context.combat_generation),
        victim_member_battle_slot_index: context.map_or(0, |context| context.battle_slot_index_raw),
        victim_country_id_raw: read_value(address + 0x48)?,
        attacker_country_id_raw: read_value(address + 0x4c)?,
        ui_source_country_id_raw: read_value(address + 0x78)?,
        sink_date_raw,
        assisted_kill: assisted_kill_raw != 0,
        victim_ship_reference_raw: [read_value(address + 0x7c)?, read_value(address + 0x80)?],
        damage_source_reference_raw: [read_value(address + 0x84)?, read_value(address + 0x88)?],
        task_force_id_raw,
        terminal_context_raw: [read_value(address + 0x98)?, read_value(address + 0x9c)?],
        cause_record_flag: cause_record_flag_raw != 0,
        ship_name,
        ship_class_name,
        attribution_source_name,
        attribution_source_class_name,
    })
}

fn read_msvc_string(address: usize) -> Result<String, crate::TextUnavailable> {
    use crate::TextUnavailable as Why;
    let len: usize = read_value(address + 0x10).ok_or(Why::LengthWordUnreadable)?;
    let cap: usize = read_value(address + 0x18).ok_or(Why::CapacityWordUnreadable)?;
    if len == 0 {
        return Err(Why::EngineEmpty);
    }
    if len >= MAX_CAPTURE_TEXT {
        return Err(Why::LengthExceedsCaptureBound);
    }
    if cap < len {
        return Err(Why::CapacityBelowLength);
    }
    let data = if cap < 16 {
        address
    } else {
        read_value::<usize>(address).ok_or(Why::PayloadUnreadable)?
    };
    let mut bytes = [0u8; MAX_CAPTURE_TEXT];
    read_exact(data, &mut bytes[..len]).ok_or(Why::PayloadUnreadable)?;
    if bytes[..len].contains(&0) {
        return Err(Why::EmbeddedNul);
    }
    String::from_utf8(bytes[..len].to_vec()).map_err(|_| Why::NotUtf8)
}

fn exact_named_text(address: usize) -> crate::ExactText {
    match read_msvc_string(address) {
        Ok(text) => crate::ExactText::Present(text),
        Err(reason) => crate::ExactText::Unavailable(reason),
    }
}

struct RecordTexts {
    ship_name: crate::ExactText,
    ship_class_name: crate::ExactText,
    attribution_source_name: crate::ExactText,
    attribution_source_class_name: crate::ExactText,
}

fn read_record_texts(
    address: usize,
    class_ptr: usize,
    attribution_class_ptr: usize,
) -> RecordTexts {
    fn through(base: usize, decode: fn(usize) -> crate::ExactText) -> crate::ExactText {
        match base.checked_add(0x68) {
            Some(at) => decode(at),
            None => crate::ExactText::Unavailable(crate::TextUnavailable::PayloadUnreadable),
        }
    }
    RecordTexts {
        ship_name: exact_named_text(address + 0x08),
        ship_class_name: through(class_ptr, exact_named_text),
        attribution_source_name: exact_optional_text(address + 0x28),
        attribution_source_class_name: if attribution_class_ptr == 0 {
            crate::ExactText::Present(String::new())
        } else {
            through(attribution_class_ptr, exact_optional_text)
        },
    }
}

fn exact_optional_text(address: usize) -> crate::ExactText {
    match read_msvc_string(address) {
        Ok(text) => crate::ExactText::Present(text),
        Err(crate::TextUnavailable::EngineEmpty) => crate::ExactText::Present(String::new()),
        Err(reason) => crate::ExactText::Unavailable(reason),
    }
}

fn admit_baseline_records<F>(count: u32, mut decode: F) -> Result<Vec<crate::SunkShipEvent>, String>
where
    F: FnMut(u32) -> Result<Option<crate::SunkShipEvent>, String>,
{
    let mut events = Vec::with_capacity(count as usize);
    for index in 0..count {
        if let Some(event) = decode(index)? {
            events.push(event);
        }
    }
    if events.len() != count as usize {
        return Err(format!(
            "A33 baseline admitted {} of the {count} records the history vector holds; \
             a complete sinking history cannot pass over a record",
            events.len()
        ));
    }
    Ok(events)
}

fn read_complete_history_baseline(module_base: usize) -> Result<HistoryVectorBaseline, String> {
    let game_state: usize = read_value(
        module_base
            .checked_add(CGAMESTATE_SINGLETON_RVA)
            .ok_or("A33 game-state singleton address overflow")?,
    )
    .ok_or("A33 game-state singleton is unreadable")?;
    if game_state == 0 {
        return Err("A33 game-state singleton is null".into());
    }
    let vector = game_state
        .checked_add(SUNK_SHIP_HISTORY_VECTOR_OFFSET)
        .ok_or("A33 sunk-ship history vector address overflow")?;
    let data: usize = read_value(vector).ok_or("A33 history vector data is unreadable")?;
    let capacity: u32 =
        read_value(vector + 0x08).ok_or("A33 history vector capacity is unreadable")?;
    let count: u32 = read_value(vector + 0x0c).ok_or("A33 history vector count is unreadable")?;
    if count > capacity || usize::try_from(capacity).unwrap_or(usize::MAX) > crate::MAX_EVENTS {
        return Err("A33 history vector count/capacity is structurally invalid".into());
    }
    if capacity != 0 && data == 0 {
        return Err("A33 allocated history vector has a null data pointer".into());
    }
    let current_date: i32 = read_value(
        game_state
            .checked_add(CGAMESTATE_CURRENT_DATE_OFFSET)
            .ok_or("A33 current-date address overflow")?,
    )
    .ok_or("A33 current date is unreadable")?;
    if current_date <= 0 {
        return Err("A33 current date is invalid".into());
    }
    observe_producer_hour(current_date);

    let mut record_addresses = Vec::with_capacity(count as usize);
    let mut unique_addresses = BTreeSet::new();
    let events = admit_baseline_records(count, |index| {
        let slot = usize::try_from(index)
            .ok()
            .and_then(|value| value.checked_mul(mem::size_of::<usize>()))
            .and_then(|offset| data.checked_add(offset))
            .ok_or("A33 history vector slot address overflow")?;
        let record: usize =
            read_value(slot).ok_or("A33 history vector record pointer is unreadable")?;
        if record == 0 || !unique_addresses.insert(record) {
            return Err("A33 history vector contains a null or duplicate record".into());
        }
        let event_sequence = u64::from(index)
            .checked_add(2)
            .ok_or("A33 baseline sequence overflow")?;
        let event = read_baseline_record(module_base, record, event_sequence, current_date)
            .map_err(|why| {
                format!(
                    "A33 existing sunk-history record {index} of {count} at 0x{record:016x} is unusable: {why}"
                )
            })?;
        record_addresses.push(record);
        Ok(Some(event))
    })?;

    let data_after: usize =
        read_value(vector).ok_or("A33 history vector data revalidation failed")?;
    let capacity_after: u32 =
        read_value(vector + 0x08).ok_or("A33 history vector capacity revalidation failed")?;
    let count_after: u32 =
        read_value(vector + 0x0c).ok_or("A33 history vector count revalidation failed")?;
    if (data_after, capacity_after, count_after) != (data, capacity, count) {
        return Err("A33 history vector changed during baseline decode".into());
    }
    for (index, expected) in record_addresses.into_iter().enumerate() {
        let slot = index
            .checked_mul(mem::size_of::<usize>())
            .and_then(|offset| data.checked_add(offset))
            .ok_or("A33 history vector revalidation address overflow")?;
        if read_value::<usize>(slot) != Some(expected) {
            return Err("A33 history vector record order changed during baseline decode".into());
        }
    }
    let snapshot = crate::SunkShipSnapshot {
        source_sequence: events.last().map_or(1, |event| event.event_sequence),
        producer_complete: true,
        records_with_unavailable_text: crate::records_with_unavailable_text(&events),
        producer_hour_raw: Some(current_date),
        producer_loss: None,
        events: events.clone(),
    };
    crate::validate_snapshot(&snapshot)
        .map_err(|error| format!("A33 existing history baseline is invalid: {error:?}"))?;
    Ok(HistoryVectorBaseline {
        data,
        capacity,
        events,
    })
}

fn read_baseline_record(
    module_base: usize,
    address: usize,
    event_sequence: u64,
    current_date: i32,
) -> Result<crate::SunkShipEvent, String> {
    fn field<T>(value: Option<T>, what: &str) -> Result<T, String> {
        value.ok_or_else(|| format!("{what} is unreadable"))
    }
    if address == 0 {
        return Err("record pointer is null".into());
    }
    let vtable = field(read_value::<usize>(address), "vtable")?;
    let expected_vtable = module_base + SUNK_SHIP_INFO_VTABLE_RVA;
    if vtable != expected_vtable {
        return Err(format!(
            "vtable is 0x{vtable:016x}, expected 0x{expected_vtable:016x}"
        ));
    }
    let class_ptr: usize = field(read_value(address + 0x68), "ship class pointer (+0x68)")?;
    let attribution_class_ptr: usize = field(
        read_value(address + 0x70),
        "attribution class pointer (+0x70)",
    )?;
    let RecordTexts {
        ship_name,
        ship_class_name,
        attribution_source_name,
        attribution_source_class_name,
    } = read_record_texts(address, class_ptr, attribution_class_ptr);
    let sink_date_raw: i32 = field(
        read_value(address + SUNK_SHIP_INFO_SINK_DATE_OFFSET),
        "sink date",
    )?;
    if sink_date_raw <= 0 || sink_date_raw > current_date {
        return Err(format!(
            "sink date {sink_date_raw} is outside 1..={current_date}"
        ));
    }
    let assisted_kill_raw: u8 = field(read_value(address + 0xa1), "assisted-kill flag (+0xa1)")?;
    let cause_record_flag_raw: u8 = field(read_value(address + 0xa0), "cause-record flag (+0xa0)")?;
    if assisted_kill_raw > 1 || cause_record_flag_raw > 1 {
        return Err(format!(
            "flag bytes out of range: assisted_kill={assisted_kill_raw}, cause_record={cause_record_flag_raw}"
        ));
    }
    let task_force: usize = field(read_value(address + 0x90), "task force pointer (+0x90)")?;
    let task_force_id_raw = if task_force == 0 {
        0
    } else {
        field(
            task_force.checked_add(0xa4).and_then(read_value),
            "task force id (task force+0xa4)",
        )?
    };
    Ok(crate::SunkShipEvent {
        event_sequence,
        combat_join_present: false,
        combat_id: 0,
        combat_generation: 0,
        victim_country_id_raw: field(read_value(address + 0x48), "victim country id (+0x48)")?,
        attacker_country_id_raw: field(read_value(address + 0x4c), "attacker country id (+0x4c)")?,
        ui_source_country_id_raw: field(
            read_value(address + 0x78),
            "ui source country id (+0x78)",
        )?,
        sink_date_raw,
        victim_member_battle_slot_index: 0,
        flags: if assisted_kill_raw != 0 {
            crate::EVENT_FLAG_ASSISTED_KILL
        } else {
            crate::EVENT_FLAG_KILLED
        },
        victim_ship_reference_raw: [
            field(
                read_value(address + 0x7c),
                "victim ship reference lo (+0x7c)",
            )?,
            field(
                read_value(address + 0x80),
                "victim ship reference hi (+0x80)",
            )?,
        ],
        damage_source_reference_raw: [
            field(
                read_value(address + 0x84),
                "damage source reference lo (+0x84)",
            )?,
            field(
                read_value(address + 0x88),
                "damage source reference hi (+0x88)",
            )?,
        ],
        task_force_id_raw,
        terminal_context_raw: [
            field(read_value(address + 0x98), "terminal context lo (+0x98)")?,
            field(read_value(address + 0x9c), "terminal context hi (+0x9c)")?,
        ],
        cause_record_flag: cause_record_flag_raw != 0,
        ship_name,
        ship_class_name,
        attribution_source_name,
        attribution_source_class_name,
    })
}

fn live_game_hour_raw() -> i64 {
    let hour = MODULE_BASE
        .load(Ordering::Acquire)
        .checked_add(CGAMESTATE_SINGLETON_RVA)
        .and_then(read_value::<usize>)
        .filter(|singleton| *singleton != 0)
        .and_then(|singleton| singleton.checked_add(CGAMESTATE_CURRENT_DATE_OFFSET))
        .and_then(read_value::<i32>);
    match hour {
        Some(hour) => i64::from(hour),
        None => i64::MIN,
    }
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

fn verify_all_sentinels(base: usize) -> Result<(), String> {
    verify_sentinel(base + OUTER_CFEX_MEMBER_WRITER_RVA, &OUTER_SENTINEL)?;
    for rva in [
        ATTRIBUTED_RECORD_WRITER_RVA,
        COMBAT_SOURCE_RECORD_WRITER_RVA,
        CAUSE_RECORD_WRITER_RVA,
    ] {
        verify_sentinel(base + rva, &RECORD_WRITER_SENTINEL)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnboundCauseCounts {
    pub epoch_snapshot_absent: u64,
    pub member_unresolved: u64,
    pub bracket_thread_mismatch: u64,
    pub gamestate_mismatch: u64,
    pub first_member_unresolved: Option<MemberUnresolvedWitness>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberUnresolvedWitness {
    pub member_address: u64,
    pub publication_sequence: u64,
    pub snapshot_members: u64,
    pub snapshot_game_hour_raw: i64,
    pub event_game_hour_raw: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberUnresolvedShape {
    PublishedEmpty,
    PublishedStale,
    SameHourOrdering,
    SnapshotAheadOfEvent,
    EventHourUnavailable,
}

impl MemberUnresolvedWitness {
    pub fn shape(&self) -> MemberUnresolvedShape {
        if self.snapshot_members == 0 {
            return MemberUnresolvedShape::PublishedEmpty;
        }
        if self.event_game_hour_raw == i64::MIN {
            return MemberUnresolvedShape::EventHourUnavailable;
        }
        // is the atomic memory ordering, and two different `Ordering`s in one
        if self.snapshot_game_hour_raw < self.event_game_hour_raw {
            MemberUnresolvedShape::PublishedStale
        } else if self.snapshot_game_hour_raw == self.event_game_hour_raw {
            MemberUnresolvedShape::SameHourOrdering
        } else {
            MemberUnresolvedShape::SnapshotAheadOfEvent
        }
    }

    pub fn shape_code(&self) -> u64 {
        match self.shape() {
            MemberUnresolvedShape::PublishedEmpty => 0,
            MemberUnresolvedShape::PublishedStale => 1,
            MemberUnresolvedShape::SameHourOrdering => 2,
            MemberUnresolvedShape::SnapshotAheadOfEvent => 3,
            MemberUnresolvedShape::EventHourUnavailable => 4,
        }
    }
}

fn member_epoch_snapshot_is_published(
    receipt: naval_combat_live_lane::MemberEpochPublicationReceipt,
) -> bool {
    receipt.publication_sequence != 0
}

fn capture_first_member_unresolved(
    member_address: u64,
    publication_sequence: u64,
    snapshot_members: u64,
    snapshot_game_hour_raw: i64,
    event_game_hour_raw: i64,
) {
    if UNBOUND_MEMBER_UNRESOLVED_CAPTURED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    UNBOUND_MEMBER_UNRESOLVED_ADDRESS.store(member_address, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_PUBLICATION_SEQUENCE.store(publication_sequence, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_MEMBERS.store(snapshot_members, Ordering::Release);
    UNBOUND_MEMBER_UNRESOLVED_EVENT_HOUR.store(event_game_hour_raw, Ordering::Release);
    // is `Release` and sequenced before it, so a reader that acquires THIS one
    UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_HOUR.store(snapshot_game_hour_raw, Ordering::Release);
}

fn first_member_unresolved_witness() -> Option<MemberUnresolvedWitness> {
    // store in the group is `Release`. The sentinel cannot collide with a real
    // READ ORDER IS LOAD-BEARING, and this is what it fixes. An `Acquire` load
    // payload was loaded first and the hour second, the acquire ordered
    // is what makes the release/acquire pair actually cover the group.
    let snapshot_game_hour_raw = UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_HOUR.load(Ordering::Acquire);
    if snapshot_game_hour_raw == i64::MIN {
        return None;
    }
    let member_address = UNBOUND_MEMBER_UNRESOLVED_ADDRESS.load(Ordering::Acquire);
    let publication_sequence =
        UNBOUND_MEMBER_UNRESOLVED_PUBLICATION_SEQUENCE.load(Ordering::Acquire);
    let snapshot_members = UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_MEMBERS.load(Ordering::Acquire);
    let event_game_hour_raw = UNBOUND_MEMBER_UNRESOLVED_EVENT_HOUR.load(Ordering::Acquire);
    Some(MemberUnresolvedWitness {
        member_address,
        publication_sequence,
        snapshot_members,
        snapshot_game_hour_raw,
        event_game_hour_raw,
    })
}

pub fn unbound_cause_counts() -> UnboundCauseCounts {
    UnboundCauseCounts {
        epoch_snapshot_absent: UNBOUND_EPOCH_SNAPSHOT_ABSENT.load(Ordering::Acquire),
        member_unresolved: UNBOUND_MEMBER_UNRESOLVED.load(Ordering::Acquire),
        bracket_thread_mismatch: UNBOUND_BRACKET_THREAD_MISMATCH.load(Ordering::Acquire),
        gamestate_mismatch: UNBOUND_GAMESTATE_MISMATCH.load(Ordering::Acquire),
        first_member_unresolved: first_member_unresolved_witness(),
    }
}

#[derive(Clone, Copy)]
struct LossCause {
    reservable: &'static AtomicU64,
    cumulative: &'static AtomicU64,
}

const LOSS_UNBOUND: LossCause = LossCause {
    reservable: &DROPPED_UNBOUND,
    cumulative: &LOST_UNBOUND_SINCE_INSTALL,
};
const LOSS_REENTRANT: LossCause = LossCause {
    reservable: &DROPPED_REENTRANT,
    cumulative: &LOST_REENTRANT_SINCE_INSTALL,
};
const LOSS_UNREADABLE: LossCause = LossCause {
    reservable: &DROPPED_UNREADABLE,
    cumulative: &LOST_UNREADABLE_SINCE_INSTALL,
};
const LOSS_LOCK_BUSY: LossCause = LossCause {
    reservable: &DROPPED_LOCK_BUSY,
    cumulative: &LOST_LOCK_BUSY_SINCE_INSTALL,
};
const LOSS_OVERFLOW: LossCause = LossCause {
    reservable: &DROPPED_OVERFLOW,
    cumulative: &LOST_OVERFLOW_SINCE_INSTALL,
};
const LOSS_CAUSES: [LossCause; 5] = [
    LOSS_UNBOUND,
    LOSS_REENTRANT,
    LOSS_UNREADABLE,
    LOSS_LOCK_BUSY,
    LOSS_OVERFLOW,
];

/// RELEASED after it.
/// Writing the permanent record first closes it. The `Release` on the
/// in this thread's program order: every reader acquire-loads the reservable
fn poison(cause: LossCause) {
    saturating_increment(cause.cumulative);
    cause.reservable.fetch_add(1, Ordering::Release);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
}

fn saturating_increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        (value != u64::MAX).then(|| value + 1)
    });
}

/// written before the reservable counter is released, so a reader that can see
/// is the order the release/acquire pairing in `poison` requires.
pub fn producer_loss_since_install() -> Option<crate::SunkShipProducerLoss> {
    let sequence_regression = SEQUENCE_REGRESSION_SINCE_INSTALL.load(Ordering::Acquire);
    let reservable = load_completeness_state();
    let loss = crate::SunkShipProducerLoss {
        dropped_unbound: LOST_UNBOUND_SINCE_INSTALL
            .load(Ordering::Acquire)
            .max(reservable.dropped_unbound),
        dropped_reentrant: LOST_REENTRANT_SINCE_INSTALL
            .load(Ordering::Acquire)
            .max(reservable.dropped_reentrant),
        dropped_unreadable: LOST_UNREADABLE_SINCE_INSTALL
            .load(Ordering::Acquire)
            .max(reservable.dropped_unreadable),
        dropped_lock_busy: LOST_LOCK_BUSY_SINCE_INSTALL
            .load(Ordering::Acquire)
            .max(reservable.dropped_lock_busy),
        dropped_overflow: LOST_OVERFLOW_SINCE_INSTALL
            .load(Ordering::Acquire)
            .max(reservable.dropped_overflow),
        sequence_regression,
    };
    loss.names_a_cause().then_some(loss)
}

pub fn cumulative_drop_witness() -> Option<(u32, u64)> {
    let loss = producer_loss_since_install()?;
    let counts = [
        loss.dropped_unbound,
        loss.dropped_reentrant,
        loss.dropped_unreadable,
        loss.dropped_lock_busy,
        loss.dropped_overflow,
    ];
    let mut mask = 0u32;
    for (index, count) in counts.iter().enumerate() {
        if *count != 0 {
            mask |= 1 << index;
        }
    }
    (mask != 0).then_some((mask, loss.dropped_total()))
}

fn producer_hour_raw() -> Option<i32> {
    match PRODUCER_OBSERVED_DATE.load(Ordering::Acquire) {
        date if date > 0 => Some(date),
        _ => None,
    }
}

fn observe_producer_hour(date_raw: i32) {
    if date_raw > 0 {
        PRODUCER_OBSERVED_DATE.fetch_max(date_raw, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CompletenessState {
    poisoned: bool,
    dropped_unbound: u64,
    dropped_reentrant: u64,
    dropped_unreadable: u64,
    dropped_lock_busy: u64,
    dropped_overflow: u64,
}

/// `poison` increments a counter and only then release-stores the latch. Read
/// This order excludes it. An acquire-load that observes the latch true
/// synchronizes-with the release-store that set it, so every increment
fn load_completeness_state() -> CompletenessState {
    let poisoned = COMPLETENESS_POISONED.load(Ordering::Acquire);
    CompletenessState {
        poisoned,
        dropped_unbound: DROPPED_UNBOUND.load(Ordering::Acquire),
        dropped_reentrant: DROPPED_REENTRANT.load(Ordering::Acquire),
        dropped_unreadable: DROPPED_UNREADABLE.load(Ordering::Acquire),
        dropped_lock_busy: DROPPED_LOCK_BUSY.load(Ordering::Acquire),
        dropped_overflow: DROPPED_OVERFLOW.load(Ordering::Acquire),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static WITNESS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear_member_unresolved_witness() {
        UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_HOUR.store(i64::MIN, Ordering::Release);
        UNBOUND_MEMBER_UNRESOLVED_EVENT_HOUR.store(i64::MIN, Ordering::Release);
        UNBOUND_MEMBER_UNRESOLVED_ADDRESS.store(0, Ordering::Release);
        UNBOUND_MEMBER_UNRESOLVED_PUBLICATION_SEQUENCE.store(0, Ordering::Release);
        UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_MEMBERS.store(0, Ordering::Release);
        UNBOUND_MEMBER_UNRESOLVED_CAPTURED.store(false, Ordering::Release);
    }

    #[test]
    fn an_unpublished_member_epoch_snapshot_is_not_a_member_resolution_failure() {
        let unpublished = naval_combat_live_lane::MemberEpochPublicationReceipt::default();
        assert_eq!(
            unpublished.publication_sequence, 0,
            "sequence 0 is the reserved never-published value the split turns on"
        );
        assert!(
            !member_epoch_snapshot_is_published(unpublished),
            "the default snapshot is the shape t25 saw; counting it as a \
             member-resolution failure blames the wrong lane"
        );

        let published = naval_combat_live_lane::MemberEpochPublicationReceipt {
            publication_sequence: 1,
            ..naval_combat_live_lane::MemberEpochPublicationReceipt::default()
        };
        assert!(
            member_epoch_snapshot_is_published(published),
            "a real publication with no members is still a publication, and a \
             member missing from it IS a resolution failure"
        );
    }

    #[test]
    fn no_unresolved_member_publishes_no_witness() {
        let _guard = WITNESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_member_unresolved_witness();
        assert_eq!(first_member_unresolved_witness(), None);
    }

    #[test]
    fn the_first_unresolved_member_wins_and_is_described_in_full() {
        let _guard = WITNESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_member_unresolved_witness();

        capture_first_member_unresolved(0x0000_7FF6_1234_5678, 41, 92, 1_234_567, 1_234_568);
        capture_first_member_unresolved(0x0000_7FF6_9999_9999, 42, 7, 1_234_568, 1_234_569);

        assert_eq!(
            first_member_unresolved_witness(),
            Some(MemberUnresolvedWitness {
                member_address: 0x0000_7FF6_1234_5678,
                publication_sequence: 41,
                snapshot_members: 92,
                snapshot_game_hour_raw: 1_234_567,
                event_game_hour_raw: 1_234_568,
            }),
            "the second refusal must not replace the first"
        );
    }

    #[test]
    fn the_witness_distinguishes_an_empty_snapshot_from_a_populated_one() {
        let _guard = WITNESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        clear_member_unresolved_witness();
        capture_first_member_unresolved(0x1000, 5, 0, 900, 901);
        let empty = first_member_unresolved_witness().expect("witness");
        assert_eq!(empty.snapshot_members, 0);
        assert_eq!(empty.shape(), MemberUnresolvedShape::PublishedEmpty);
        assert_eq!(empty.shape_code(), 0);

        clear_member_unresolved_witness();
        capture_first_member_unresolved(0x2000, 6, 116, 900, 901);
        let stale = first_member_unresolved_witness().expect("witness");
        assert_eq!(stale.snapshot_members, 116);
        assert_eq!(stale.snapshot_game_hour_raw, 900);
        assert_eq!(stale.event_game_hour_raw, 901);
        assert_eq!(stale.shape(), MemberUnresolvedShape::PublishedStale);
        assert_eq!(stale.shape_code(), 1);

        clear_member_unresolved_witness();
        capture_first_member_unresolved(0x3000, 7, 116, 901, 901);
        let ordering = first_member_unresolved_witness().expect("witness");
        assert_eq!(ordering.member_address, 0x3000);
        assert_eq!(ordering.shape(), MemberUnresolvedShape::SameHourOrdering);
        assert_eq!(ordering.shape_code(), 2);

        clear_member_unresolved_witness();
        capture_first_member_unresolved(0x4000, 8, 116, 902, 901);
        let ahead = first_member_unresolved_witness().expect("witness");
        assert_eq!(ahead.shape(), MemberUnresolvedShape::SnapshotAheadOfEvent);
        assert_eq!(ahead.shape_code(), 3);

        clear_member_unresolved_witness();
        capture_first_member_unresolved(0x5000, 9, 116, 901, i64::MIN);
        let unknown = first_member_unresolved_witness().expect("witness");
        assert_eq!(unknown.shape(), MemberUnresolvedShape::EventHourUnavailable);
        assert_eq!(unknown.shape_code(), 4);
    }

    /// The hour is stored last and every store in the group is `Release`, so
    /// the reader's `Acquire` on it is what orders the group. But an acquire
    /// Deliberately NOT pinned: the memory ordering of the payload stores. A
    /// release store publishes everything sequenced before it on that thread, so
    #[test]
    fn the_witness_reader_gates_on_the_hour_before_it_loads_the_payload() {
        // comma, turning `Ordering::Acquire)` into `Ordering::Acquire,)`.
        fn squeeze(text: &str) -> String {
            let packed: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            packed.replace(",)", ")")
        }
        let source = include_str!("a729_hook.rs");
        let body = squeeze(
            source
                .split_once("fn first_member_unresolved_witness(")
                .expect("the witness reader")
                .1
                .split_once("\n}")
                .expect("its body")
                .0,
        );
        // The whole gate as one block: the ACQUIRE load, the sentinel test and
        let gate_block = squeeze(
            "let snapshot_game_hour_raw = \
             UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_HOUR.load(Ordering::Acquire); \
             if snapshot_game_hour_raw == i64::MIN { return None; }",
        );
        assert!(
            body.contains(&gate_block),
            "the readiness gate must be an Acquire load, a sentinel test, and a \
             return -- all three"
        );
        let bail = body.find(&gate_block).expect("the readiness gate") + gate_block.len();
        for payload in [
            "UNBOUND_MEMBER_UNRESOLVED_ADDRESS.load",
            "UNBOUND_MEMBER_UNRESOLVED_PUBLICATION_SEQUENCE.load",
            "UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_MEMBERS.load",
            "UNBOUND_MEMBER_UNRESOLVED_EVENT_HOUR.load",
        ] {
            let at = body
                .find(payload)
                .unwrap_or_else(|| panic!("the reader must load {payload}"));
            assert!(
                bail < at,
                "{payload} is loaded before the readiness gate returns, so an \
                 acquire on the hour orders nothing and a half-written group \
                 can be published as a witness"
            );
        }

        // LAST and with `Release`, or the gate certifies a group that is not yet
        let writer = squeeze(
            source
                .split_once("fn capture_first_member_unresolved(")
                .expect("the witness writer")
                .1
                .split_once("\n}")
                .expect("its body")
                .0,
        );
        let signal = writer
            .find(&squeeze(
                "UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_HOUR.store(snapshot_game_hour_raw, \
                 Ordering::Release);",
            ))
            .expect("the writer must store the readiness signal with Release");
        // Only the SIGNAL needs `Release`. A release store publishes every write
        // carried, so requiring `Release` on each payload store would pin an
        for payload in [
            "UNBOUND_MEMBER_UNRESOLVED_ADDRESS.store(member_address,",
            "UNBOUND_MEMBER_UNRESOLVED_PUBLICATION_SEQUENCE.store(publication_sequence,",
            "UNBOUND_MEMBER_UNRESOLVED_SNAPSHOT_MEMBERS.store(snapshot_members,",
            "UNBOUND_MEMBER_UNRESOLVED_EVENT_HOUR.store(event_game_hour_raw,",
        ] {
            let at = writer
                .find(&squeeze(payload))
                .unwrap_or_else(|| panic!("the writer must store {payload}"));
            assert!(
                at < signal,
                "{payload} is stored after the readiness signal, so a reader \
                 that has already passed the gate can miss it"
            );
        }
        // The CAS's memory ordering is deliberately NOT pinned. Its job is
        // separate release/acquire pair on the readiness store checked above.
        // Requiring `AcqRel`/`Acquire` here would pin a rule the model does not
        let cas_at = writer
            .find(&squeeze(
                "if UNBOUND_MEMBER_UNRESOLVED_CAPTURED.compare_exchange(false, true,",
            ))
            .expect("first-wins must compare-exchange the latch from false to true");
        let cas = &writer[cas_at..];
        let cas_end = cas.find('}').expect("the first-wins block must be closed") + 1;
        assert!(
            cas[..cas_end].ends_with(&squeeze(").is_err() { return; }")),
            "the CAS's own failure must RETURN, with nothing between, or the \
             loser overwrites the winner"
        );
    }

    #[test]
    fn a_negative_snapshot_hour_round_trips_through_the_unit_channel() {
        let _guard = WITNESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_member_unresolved_witness();
        capture_first_member_unresolved(0x3000, 7, 12, -4_242, -4_240);
        let witness = first_member_unresolved_witness().expect("witness");
        assert_eq!(witness.snapshot_game_hour_raw, -4_242);
        assert_eq!(witness.event_game_hour_raw, -4_240);
        for hour in [witness.snapshot_game_hour_raw, witness.event_game_hour_raw] {
            let on_the_wire = hour as u64;
            assert_eq!(
                i64::from_le_bytes(on_the_wire.to_le_bytes()),
                hour,
                "the unit carries the raw bits and reads back as the same signed hour"
            );
        }
        assert_eq!(witness.shape(), MemberUnresolvedShape::PublishedStale);
    }

    fn authoritative_drain() -> HookDrain {
        HookDrain {
            hook_epoch: 1,
            events: Vec::new(),
            dropped_unbound: 0,
            dropped_reentrant: 0,
            dropped_unreadable: 0,
            dropped_lock_busy: 0,
            dropped_overflow: 0,
            completeness_poisoned: false,
            baseline_measured_empty: true,
            baseline_revalidated_empty: true,
            baseline_complete: true,
            baseline_revalidated_complete: true,
            evidence_install: false,
            installed: true,
            install_phase: INSTALL_PHASE_ACTIVE,
            install_mode: INSTALL_MODE_PRODUCTION,
        }
    }

    #[test]
    fn complete_history_refusal_names_each_distinct_cause() {
        assert_eq!(complete_history_drain_refusal(&authoritative_drain()), None);

        let mut reasons: Vec<String> = Vec::new();
        let mut codes: Vec<CompleteHistoryRefusalCode> = Vec::new();
        let mutations: [(&str, fn(&mut HookDrain)); 9] = [
            ("evidence_install", |drain| drain.evidence_install = true),
            ("baseline_complete", |drain| drain.baseline_complete = false),
            ("baseline_revalidated", |drain| {
                drain.baseline_revalidated_complete = false
            }),
            ("poisoned", |drain| drain.completeness_poisoned = true),
            ("unbound", |drain| drain.dropped_unbound = 1),
            ("reentrant", |drain| drain.dropped_reentrant = 1),
            ("unreadable", |drain| drain.dropped_unreadable = 1),
            ("lock_busy", |drain| drain.dropped_lock_busy = 1),
            ("overflow", |drain| drain.dropped_overflow = 1),
        ];
        for (label, mutate) in mutations {
            let mut drain = authoritative_drain();
            mutate(&mut drain);
            let refusal = complete_history_drain_refusal(&drain)
                .unwrap_or_else(|| panic!("{label} must refuse the drain"));
            reasons.push(refusal.message);
            codes.push(refusal.code);
        }

        let mut distinct = reasons.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            reasons.len(),
            "each refusal cause must be distinguishable on the wire: {reasons:#?}"
        );

        let mut distinct_codes = codes.clone();
        distinct_codes.sort_by_key(|code| *code as u32);
        distinct_codes.dedup();
        assert_eq!(
            distinct_codes.len(),
            codes.len(),
            "each refusal cause must carry its own code: {codes:#?}"
        );
    }

    #[test]
    fn complete_history_refusal_reports_exact_loss_counts() {
        let mut drain = authoritative_drain();
        drain.dropped_unreadable = 3;
        drain.dropped_overflow = 11;
        let refusal = complete_history_drain_refusal(&drain).expect("losses must refuse");
        let reason = &refusal.message;
        assert!(reason.contains("unreadable=3"), "{reason}");
        assert!(reason.contains("overflow=11"), "{reason}");
        assert!(reason.contains("lost 14 callback(s)"), "{reason}");
        assert_eq!(refusal.code, CompleteHistoryRefusalCode::DroppedUnreadable);
        assert_eq!(refusal.drop_mask, 0b1_0100);
        assert_eq!(refusal.drop_total, 14);
    }

    #[test]
    fn a_poisoning_drop_is_named_as_the_drop_and_not_as_a_sequence_fault() {
        let cases: [(CompleteHistoryRefusalCode, fn(&mut HookDrain)); 5] = [
            (CompleteHistoryRefusalCode::DroppedUnbound, |drain| {
                drain.dropped_unbound = 1
            }),
            (CompleteHistoryRefusalCode::DroppedReentrant, |drain| {
                drain.dropped_reentrant = 1
            }),
            (CompleteHistoryRefusalCode::DroppedUnreadable, |drain| {
                drain.dropped_unreadable = 1
            }),
            (CompleteHistoryRefusalCode::DroppedLockBusy, |drain| {
                drain.dropped_lock_busy = 1
            }),
            (CompleteHistoryRefusalCode::DroppedOverflow, |drain| {
                drain.dropped_overflow = 1
            }),
        ];
        for (expected, mutate) in cases {
            let mut drain = authoritative_drain();
            mutate(&mut drain);
            drain.completeness_poisoned = true;
            let refusal =
                complete_history_drain_refusal(&drain).expect("a lost callback must refuse");
            assert_eq!(
                refusal.code, expected,
                "a drop that latched the poison flag must still be named as the \
                 drop it was, not as a non-increasing event sequence: {}",
                refusal.message
            );
            assert!(
                !refusal.message.contains("non-increasing"),
                "the message must not assert a cause that did not occur: {}",
                refusal.message
            );
        }

        let mut sequence_fault = authoritative_drain();
        sequence_fault.completeness_poisoned = true;
        assert_eq!(
            complete_history_drain_refusal(&sequence_fault)
                .expect("a poisoned latch with no loss must still refuse")
                .code,
            CompleteHistoryRefusalCode::CompletenessPoisoned,
        );
    }

    /// increments and only then release-stores the latch, so a reader that
    /// acquire-loads the latch first and the counters second cannot observe the
    /// The `Release` on the reservable increment is what carries the order
    /// across threads: readers acquire-load the reservable counter before the
    /// reservable increment must be a Release`.
    #[test]
    fn poison_sets_the_counter_before_the_completeness_latch() {
        const POISON_SOURCE: &str = include_str!("a729_hook.rs");
        let body = POISON_SOURCE
            .split_once("fn poison(cause: LossCause) {")
            .expect("poison")
            .1
            .split_once('}')
            .expect("poison body")
            .0;
        let increment = body
            .find("cause.reservable.fetch_add(1")
            .expect("poison must count the loss");
        let cumulative = body
            .find("saturating_increment(cause.cumulative)")
            .expect("poison must count the loss since install");
        let latch = body
            .find("COMPLETENESS_POISONED.store(true")
            .expect("poison must latch completeness");
        assert!(
            cumulative < increment,
            "the permanent counter must be raised BEFORE the consumable one, or a \
             commit can clear a loss that was never permanently recorded: {body}"
        );
        assert!(
            body[increment..].starts_with("cause.reservable.fetch_add(1, Ordering::Release)"),
            "the reservable increment must be a Release, or the order above holds \
             only in this thread's program order: {body}"
        );
        assert!(
            increment < latch && cumulative < latch,
            "both counters must be incremented BEFORE the latch is released, or a \
             reader can observe the latch without the count that caused it: {body}"
        );
    }

    #[test]
    fn each_loss_cause_owns_its_own_pair_of_counters() {
        for (index, cause) in LOSS_CAUSES.iter().enumerate() {
            for (other_index, other) in LOSS_CAUSES.iter().enumerate() {
                if index == other_index {
                    continue;
                }
                assert!(
                    !std::ptr::eq(cause.reservable, other.reservable),
                    "two loss causes share the reservable counter at index {index}"
                );
                assert!(
                    !std::ptr::eq(cause.cumulative, other.cumulative),
                    "two loss causes share the cumulative counter at index {index}"
                );
            }
            assert!(
                !std::ptr::eq(cause.reservable, cause.cumulative),
                "a cause's reservable and cumulative counters must be different \
                 statics, or a commit would clear the permanent record too"
            );
        }
    }

    #[test]
    fn a_loss_visible_only_to_the_reservable_counter_is_still_named() {
        let _guard = WITNESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let restore = DROPPED_LOCK_BUSY.load(Ordering::Acquire);
        DROPPED_LOCK_BUSY.store(1, Ordering::Release);
        let observed = producer_loss_since_install();
        DROPPED_LOCK_BUSY.store(restore, Ordering::Release);

        let loss = observed.unwrap_or_else(|| {
            panic!(
                "a loss the reservation can see must be named, not left to the \
                 cumulative counter: {observed:?}"
            )
        });
        assert_eq!(loss.dropped_lock_busy, 1);
        assert!(loss.names_a_cause());
    }

    #[test]
    fn a_saturated_cumulative_loss_counter_does_not_wrap() {
        let _guard = WITNESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let restore = LOST_OVERFLOW_SINCE_INSTALL.load(Ordering::Acquire);
        let restore_reservable = DROPPED_OVERFLOW.load(Ordering::Acquire);
        let restore_latch = COMPLETENESS_POISONED.load(Ordering::Acquire);
        LOST_OVERFLOW_SINCE_INSTALL.store(u64::MAX, Ordering::Release);
        poison(LOSS_OVERFLOW);
        let observed = LOST_OVERFLOW_SINCE_INSTALL.load(Ordering::Acquire);
        LOST_OVERFLOW_SINCE_INSTALL.store(restore, Ordering::Release);
        DROPPED_OVERFLOW.store(restore_reservable, Ordering::Release);
        COMPLETENESS_POISONED.store(restore_latch, Ordering::Release);

        assert_eq!(
            observed,
            u64::MAX,
            "a saturated cumulative counter must not wrap back to zero"
        );
    }

    #[test]
    fn the_completeness_snapshot_reads_the_latch_before_the_counters() {
        const SOURCE: &str = include_str!("a729_hook.rs");
        let body = SOURCE
            .split_once("fn load_completeness_state() -> CompletenessState {")
            .expect("the completeness snapshot")
            .1
            .split_once("\n}")
            .expect("its body")
            .0;
        let latch = body
            .find("COMPLETENESS_POISONED.load")
            .expect("the snapshot must read the latch");
        for counter in [
            "DROPPED_UNBOUND.load",
            "DROPPED_REENTRANT.load",
            "DROPPED_UNREADABLE.load",
            "DROPPED_LOCK_BUSY.load",
            "DROPPED_OVERFLOW.load",
        ] {
            let at = body
                .find(counter)
                .unwrap_or_else(|| panic!("the snapshot must read {counter}"));
            assert!(
                latch < at,
                "{counter} is read before the latch, so a drop landing between \
                 them yields a zero count with the latch set and is published as \
                 a sequence fault"
            );
        }

        let live_latch_field = format!("completeness_poisoned: {}_POISONED.load", "COMPLETENESS");
        assert!(
            !SOURCE.contains(&live_latch_field),
            "a HookDrain must take its latch from the same snapshot as its counters"
        );
    }

    #[test]
    fn production_hook_receipt_roster_is_exact_and_stable_name_ordered() {
        assert_eq!(
            PRODUCTION_HOOK_SPECS.map(|spec| (spec.logical_name, spec.requested_rva)),
            [
                (
                    "gameplay.d86.attributed_record_writer",
                    ATTRIBUTED_RECORD_WRITER_RVA as u64,
                ),
                (
                    "gameplay.d86.cause_record_writer",
                    CAUSE_RECORD_WRITER_RVA as u64,
                ),
                (
                    "gameplay.d86.combat_source_record_writer",
                    COMBAT_SOURCE_RECORD_WRITER_RVA as u64,
                ),
                (
                    "gameplay.d86.outer_cfex_member_writer",
                    OUTER_CFEX_MEMBER_WRITER_RVA as u64,
                ),
            ]
        );
        assert!(PRODUCTION_HOOK_SPECS.iter().all(|spec| {
            spec.target_patch_len == 5
                && spec.primitive_id == MINHOOK_NORMAL_REL5_PRIMITIVE_ID
                && spec.original_prefix.len() == 16
        }));
    }

    fn event(sequence: u64) -> RawSunkShipCapture {
        RawSunkShipCapture {
            event_sequence: sequence,
            combat_join_present: true,
            thread_id: 7,
            member_epoch_publication_sequence: 3,
            member_epoch_game_hour_raw: 123,
            combat_id: 4,
            combat_generation: 2,
            victim_member_battle_slot_index: 1,
            victim_country_id_raw: 5,
            attacker_country_id_raw: 6,
            ui_source_country_id_raw: 8,
            sink_date_raw: 123,
            assisted_kill: false,
            victim_ship_reference_raw: [9, 10],
            damage_source_reference_raw: [11, 12],
            task_force_id_raw: 13,
            terminal_context_raw: [14, 15],
            cause_record_flag: false,
            ship_name: "HMS Exact".into(),
            ship_class_name: "Destroyer".into(),
            attribution_source_name: "KMS Exact".into(),
            attribution_source_class_name: "Cruiser".into(),
        }
    }

    fn candidate() -> HookDrain {
        HookDrain {
            hook_epoch: 9,
            events: vec![event(2)],
            baseline_measured_empty: true,
            baseline_revalidated_empty: true,
            evidence_install: true,
            ..HookDrain::default()
        }
    }

    #[test]
    fn evidence_candidate_requires_empty_baseline_joined_event_and_loss_free_drain() {
        assert!(candidate().evidence_fixture_contract_satisfied());
        let mut missing_baseline = candidate();
        missing_baseline.baseline_measured_empty = false;
        assert!(!missing_baseline.evidence_fixture_contract_satisfied());
        let mut changed_during_activation = candidate();
        changed_during_activation.baseline_revalidated_empty = false;
        assert!(!changed_during_activation.evidence_fixture_contract_satisfied());
        let mut unbound = candidate();
        unbound.dropped_unbound = 1;
        assert!(!unbound.evidence_fixture_contract_satisfied());
        let mut missing_join = candidate();
        missing_join.events[0].member_epoch_publication_sequence = 0;
        assert!(!missing_join.evidence_fixture_contract_satisfied());
    }

    #[test]
    fn evidence_candidate_rejects_non_monotonic_event_sequence() {
        let mut drain = candidate();
        drain.events.push(event(2));
        assert!(!drain.evidence_fixture_contract_satisfied());
    }

    #[test]
    fn evidence_candidate_preserves_optional_attacker_absence() {
        let mut drain = candidate();
        drain.events[0].damage_source_reference_raw = [0, 0];
        drain.events[0].attribution_source_name = crate::ExactText::Present(String::new());
        drain.events[0].attribution_source_class_name = crate::ExactText::Present(String::new());
        assert!(drain.evidence_fixture_contract_satisfied());
    }

    #[test]
    fn evidence_fixture_still_demands_every_text_field_of_a_live_sink() {
        for unavailable in [
            crate::TextUnavailable::EngineEmpty,
            crate::TextUnavailable::PayloadUnreadable,
        ] {
            let mut drain = candidate();
            drain.events[0].ship_name = crate::ExactText::Unavailable(unavailable);
            assert!(
                !drain.evidence_fixture_contract_satisfied(),
                "the activation-evidence gate is stricter than production admission: \
                 a freshly observed live sink must decode whole ({unavailable:?})"
            );
        }
        let mut empty_present_name = candidate();
        empty_present_name.events[0].ship_name = crate::ExactText::Present(String::new());
        assert!(!empty_present_name.evidence_fixture_contract_satisfied());
    }

    struct FabricatedString {
        object: Box<[u8; 0x20]>,
        _heap: Option<Box<[u8]>>,
    }

    impl FabricatedString {
        fn inline(bytes: &[u8], len: usize, cap: usize) -> Self {
            let mut object = Box::new([0u8; 0x20]);
            object[..bytes.len()].copy_from_slice(bytes);
            object[0x10..0x18].copy_from_slice(&len.to_le_bytes());
            object[0x18..0x20].copy_from_slice(&cap.to_le_bytes());
            Self {
                object,
                _heap: None,
            }
        }

        fn heap(bytes: &[u8], len: usize, cap: usize) -> Self {
            let heap = bytes.to_vec().into_boxed_slice();
            let mut object = Box::new([0u8; 0x20]);
            object[..8].copy_from_slice(&(heap.as_ptr() as usize).to_le_bytes());
            object[0x10..0x18].copy_from_slice(&len.to_le_bytes());
            object[0x18..0x20].copy_from_slice(&cap.to_le_bytes());
            Self {
                object,
                _heap: Some(heap),
            }
        }

        fn heap_pointer_to(address: usize, len: usize, cap: usize) -> Self {
            let mut object = Box::new([0u8; 0x20]);
            object[..8].copy_from_slice(&address.to_le_bytes());
            object[0x10..0x18].copy_from_slice(&len.to_le_bytes());
            object[0x18..0x20].copy_from_slice(&cap.to_le_bytes());
            Self {
                object,
                _heap: None,
            }
        }

        fn address(&self) -> usize {
            self.object.as_ptr() as usize
        }
    }

    fn unreadable_address() -> usize {
        use windows_sys::Win32::System::Memory::{VirtualAlloc, MEM_RESERVE, PAGE_NOACCESS};
        let base =
            unsafe { VirtualAlloc(ptr::null(), 0x10_000, MEM_RESERVE, PAGE_NOACCESS) } as usize;
        assert_ne!(base, 0, "reserving a probe region must succeed");
        base + 0x1_000
    }

    #[test]
    fn every_msvc_string_refusal_names_its_own_exact_cause() {
        use crate::TextUnavailable as Why;

        let short = FabricatedString::inline(b"HMS Exact", 9, 15);
        assert_eq!(
            read_msvc_string(short.address()),
            Ok("HMS Exact".to_string()),
            "an inline small string decodes from the object's own buffer"
        );

        let long = FabricatedString::heap(b"HMS Considerably Longer", 23, 31);
        assert_eq!(
            read_msvc_string(long.address()),
            Ok("HMS Considerably Longer".to_string()),
            "a capacity of 16 or more means the characters live behind the +0x00 pointer"
        );

        let empty = FabricatedString::inline(b"", 0, 15);
        assert_eq!(
            read_msvc_string(empty.address()),
            Err(Why::EngineEmpty),
            "a well-formed empty string is its own outcome, never a read failure"
        );

        let overlong = FabricatedString::heap_pointer_to(0x1_000, MAX_CAPTURE_TEXT, usize::MAX);
        assert_eq!(
            read_msvc_string(overlong.address()),
            Err(Why::LengthExceedsCaptureBound),
            "the bound is ours, and it is reported as ours"
        );

        let incoherent = FabricatedString::inline(b"HMS Exact", 9, 8);
        assert_eq!(
            read_msvc_string(incoherent.address()),
            Err(Why::CapacityBelowLength),
            "capacity below length is not a std::string at all"
        );

        let dangling = FabricatedString::heap_pointer_to(unreadable_address(), 9, 31);
        assert_eq!(
            read_msvc_string(dangling.address()),
            Err(Why::PayloadUnreadable),
            "a readable header with unreadable characters is its own outcome"
        );

        let nul = FabricatedString::inline(b"HM\0S", 4, 15);
        assert_eq!(read_msvc_string(nul.address()), Err(Why::EmbeddedNul));

        let not_utf8 = FabricatedString::inline(&[0xff, 0xfe, 0xfd], 3, 15);
        assert_eq!(read_msvc_string(not_utf8.address()), Err(Why::NotUtf8));

        assert_eq!(
            read_msvc_string(unreadable_address()),
            Err(Why::LengthWordUnreadable),
            "the length word is read first, so an unmapped object refuses there"
        );

        use windows_sys::Win32::System::Memory::{
            VirtualAlloc, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE,
        };
        let region =
            unsafe { VirtualAlloc(ptr::null(), 0x2_000, MEM_RESERVE, PAGE_READWRITE) } as usize;
        assert_ne!(region, 0);
        let committed =
            unsafe { VirtualAlloc(region as *const _, 0x1_000, MEM_COMMIT, PAGE_READWRITE) }
                as usize;
        assert_eq!(committed, region, "only the first page is committed");
        let straddling = region + 0x1_000 - 0x18;
        unsafe {
            ptr::write_bytes(straddling as *mut u8, 0, 0x18);
            ptr::write((straddling + 0x10) as *mut usize, 9usize);
        }
        assert_eq!(
            read_msvc_string(straddling),
            Err(Why::CapacityWordUnreadable),
            "a readable length word and an unreadable capacity word is its own outcome"
        );
    }

    #[test]
    fn a_named_field_reports_emptiness_while_an_optional_field_carries_it() {
        let empty = FabricatedString::inline(b"", 0, 15);
        assert_eq!(
            exact_named_text(empty.address()),
            crate::ExactText::Unavailable(crate::TextUnavailable::EngineEmpty),
            "a ship name the engine left empty is an explicit absence of text"
        );
        assert_eq!(
            exact_optional_text(empty.address()),
            crate::ExactText::Present(String::new()),
            "an empty +0x28 is the engine's own 'no attacking ship' value, \
             which a729's own constructors write, so it is a measurement"
        );

        let dangling = FabricatedString::heap_pointer_to(unreadable_address(), 9, 31);
        for field in [
            exact_named_text(dangling.address()),
            exact_optional_text(dangling.address()),
        ] {
            assert_eq!(
                field,
                crate::ExactText::Unavailable(crate::TextUnavailable::PayloadUnreadable)
            );
        }
    }

    struct FabricatedRecord {
        record: Box<[u8; 0xa8]>,
        class_object: Box<[u8; 0x88]>,
        _ship_name: FabricatedString,
    }

    impl FabricatedRecord {
        fn new(ship_name: FabricatedString) -> Self {
            let mut class_object = Box::new([0u8; 0x88]);
            let class_name = FabricatedString::inline(b"Destroyer", 9, 15);
            class_object[0x68..0x88].copy_from_slice(class_name.object.as_slice());

            let mut record = Box::new([0u8; 0xa8]);
            record[0x08..0x28].copy_from_slice(ship_name.object.as_slice());
            record[0x38..0x40].copy_from_slice(&0usize.to_le_bytes());
            record[0x40..0x48].copy_from_slice(&15usize.to_le_bytes());
            record[0x48..0x4c].copy_from_slice(&3u32.to_le_bytes());
            record[0x4c..0x50].copy_from_slice(&9u32.to_le_bytes());
            record[0x58..0x5c].copy_from_slice(&1_000i32.to_le_bytes());
            record[0x68..0x70].copy_from_slice(&(class_object.as_ptr() as usize).to_le_bytes());
            record[0x78..0x7c].copy_from_slice(&5u32.to_le_bytes());
            record[0x7c..0x80].copy_from_slice(&11u32.to_le_bytes());
            record[0x80..0x84].copy_from_slice(&12u32.to_le_bytes());
            record[0x84..0x88].copy_from_slice(&13u32.to_le_bytes());
            record[0x88..0x8c].copy_from_slice(&14u32.to_le_bytes());
            record[0x98..0x9c].copy_from_slice(&16u32.to_le_bytes());
            record[0x9c..0xa0].copy_from_slice(&17u32.to_le_bytes());
            Self {
                record,
                class_object,
                _ship_name: ship_name,
            }
        }

        fn install_vtable(&mut self) -> usize {
            let vtable = self.class_object.as_ptr() as usize;
            self.record[..8].copy_from_slice(&vtable.to_le_bytes());
            vtable.wrapping_sub(SUNK_SHIP_INFO_VTABLE_RVA)
        }

        fn address(&self) -> usize {
            self.record.as_ptr() as usize
        }
    }

    #[test]
    fn read_baseline_record_admits_a_record_the_engine_left_unnamed() {
        let mut named = FabricatedRecord::new(FabricatedString::inline(b"HMS Exact", 9, 15));
        let module_base = named.install_vtable();
        let decoded = read_baseline_record(module_base, named.address(), 2, 2_000)
            .expect("a named record still decodes");
        assert_eq!(
            decoded.ship_name,
            crate::ExactText::Present("HMS Exact".into())
        );
        assert_eq!(
            decoded.ship_class_name,
            crate::ExactText::Present("Destroyer".into())
        );
        assert_eq!(decoded.victim_country_id_raw, 3);
        assert_eq!(decoded.sink_date_raw, 1_000);
        assert!(!decoded.has_unavailable_text());

        let mut unnamed = FabricatedRecord::new(FabricatedString::inline(b"", 0, 15));
        let module_base = unnamed.install_vtable();
        let decoded = read_baseline_record(module_base, unnamed.address(), 3, 2_000)
            .expect("an unnamed record is admitted, not refused");
        assert_eq!(
            decoded.ship_name,
            crate::ExactText::Unavailable(crate::TextUnavailable::EngineEmpty)
        );
        assert!(decoded.has_unavailable_text());
        assert_eq!(
            decoded.ship_class_name,
            crate::ExactText::Present("Destroyer".into())
        );
        assert_eq!(decoded.attacker_country_id_raw, 9);
        assert_eq!(decoded.victim_ship_reference_raw, [11, 12]);
        assert_eq!(decoded.terminal_context_raw, [16, 17]);

        let mut torn = FabricatedRecord::new(FabricatedString::heap_pointer_to(
            unreadable_address(),
            9,
            31,
        ));
        let module_base = torn.install_vtable();
        let decoded = read_baseline_record(module_base, torn.address(), 4, 2_000)
            .expect("a torn name is a field problem, not a record problem");
        assert_eq!(
            decoded.ship_name,
            crate::ExactText::Unavailable(crate::TextUnavailable::PayloadUnreadable)
        );
    }

    #[test]
    fn read_baseline_record_still_refuses_what_makes_a_record_unidentifiable() {
        let mut record = FabricatedRecord::new(FabricatedString::inline(b"HMS Exact", 9, 15));
        let module_base = record.install_vtable();

        let error = read_baseline_record(
            module_base.wrapping_add(0x1_000),
            record.address(),
            2,
            2_000,
        )
        .expect_err("a mismatched vtable is not a CSunkShipInfo");
        assert!(error.contains("vtable is 0x"), "{error}");

        let error = read_baseline_record(module_base, record.address(), 2, 500)
            .expect_err("a sinking cannot postdate the campaign");
        assert!(
            error.contains("sink date 1000 is outside 1..=500"),
            "{error}"
        );

        assert!(read_baseline_record(module_base, 0, 2, 2_000)
            .is_err_and(|error| error.contains("record pointer is null")));

        let mut bad_flag = FabricatedRecord::new(FabricatedString::inline(b"HMS Exact", 9, 15));
        let module_base = bad_flag.install_vtable();
        bad_flag.record[0xa1] = 7;
        let error = read_baseline_record(module_base, bad_flag.address(), 2, 2_000)
            .expect_err("a flag byte outside 0..=1 is not a flag");
        assert!(error.contains("flag bytes out of range"), "{error}");

        let mut unformable = FabricatedRecord::new(FabricatedString::inline(b"HMS Exact", 9, 15));
        let module_base = unformable.install_vtable();
        unformable.record[0x68..0x70].copy_from_slice(&usize::MAX.to_le_bytes());
        let decoded = read_baseline_record(module_base, unformable.address(), 2, 2_000)
            .expect("an unformable text address does not unidentify the record");
        assert_eq!(
            decoded.ship_class_name,
            crate::ExactText::Unavailable(crate::TextUnavailable::PayloadUnreadable)
        );

        let mut bad_class = FabricatedRecord::new(FabricatedString::inline(b"HMS Exact", 9, 15));
        let module_base = bad_class.install_vtable();
        bad_class.record[0x68..0x70].copy_from_slice(&unreadable_address().to_le_bytes());
        let decoded = read_baseline_record(module_base, bad_class.address(), 2, 2_000)
            .expect("an unreadable class name does not unidentify the record");
        assert_eq!(
            decoded.ship_class_name,
            crate::ExactText::Unavailable(crate::TextUnavailable::LengthWordUnreadable)
        );
    }

    #[test]
    fn an_unnamed_record_is_admitted_and_accounted_across_the_whole_baseline() {
        let mut events: Vec<crate::SunkShipEvent> = (0..3).map(baseline_event).collect();
        events[1].ship_name = crate::ExactText::Unavailable(crate::TextUnavailable::EngineEmpty);
        let admitted = admit_baseline_records(3, |index| Ok(Some(events[index as usize].clone())))
            .expect("an unnamed record is admitted, not refused");
        assert_eq!(admitted.len(), 3);

        let snapshot = crate::SunkShipSnapshot {
            source_sequence: admitted.last().unwrap().event_sequence,
            producer_complete: true,
            records_with_unavailable_text: crate::records_with_unavailable_text(&admitted),
            producer_hour_raw: None,
            producer_loss: None,
            events: admitted,
        };
        assert_eq!(snapshot.records_with_unavailable_text, 1);
        crate::validate_snapshot(&snapshot)
            .expect("a complete history with one unnamed record is valid");
    }

    #[test]
    fn a_passed_over_record_fails_the_baseline_instead_of_shortening_the_history() {
        let events: Vec<crate::SunkShipEvent> = (0..4).map(baseline_event).collect();
        assert_eq!(
            admit_baseline_records(4, |index| Ok(Some(events[index as usize].clone())))
                .map(|admitted| admitted.len()),
            Ok(4)
        );

        let error = admit_baseline_records(4, |index| {
            if index == 2 {
                return Ok(None);
            }
            Ok(Some(events[index as usize].clone()))
        })
        .expect_err("a history that passed over a record is not complete");
        assert!(
            error.contains("admitted 3 of the 4 records"),
            "the refusal must say how many records went missing: {error}"
        );

        let propagated = admit_baseline_records(4, |index| {
            if index == 2 {
                return Err("record 2 is unidentifiable".into());
            }
            Ok(Some(events[index as usize].clone()))
        })
        .expect_err("a genuinely unidentifiable record still fails closed");
        assert_eq!(propagated, "record 2 is unidentifiable");
    }

    #[test]
    fn a_live_rows_unavailable_field_is_accounted_when_it_extends_the_history() {
        let baseline: Vec<crate::SunkShipEvent> = (0..2).map(baseline_event).collect();
        let mut snapshot = crate::SunkShipSnapshot {
            source_sequence: baseline.last().unwrap().event_sequence,
            producer_complete: true,
            records_with_unavailable_text: crate::records_with_unavailable_text(&baseline),
            producer_hour_raw: None,
            producer_loss: None,
            events: baseline,
        };
        assert_eq!(snapshot.records_with_unavailable_text, 0);

        let mut raw = event(4);
        raw.ship_class_name =
            crate::ExactText::Unavailable(crate::TextUnavailable::CapacityBelowLength);
        extend_complete_snapshot(&mut snapshot, &[raw])
            .expect("a live row with an unavailable field extends the history");

        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(
            snapshot.records_with_unavailable_text, 1,
            "the extended history accounts for the live row's absence, not the baseline's"
        );
        crate::validate_snapshot(&snapshot).expect("the extended snapshot is self-consistent");
    }

    #[test]
    fn an_append_with_no_naval_combat_bracket_is_a_sinking_not_an_unbound_loss() {
        assert_eq!(
            resolve_append_bracket(None, 7),
            Ok(None),
            "a training-accident or mine sinking arrives with no outer bracket \
             at all; that is the engine's normal shape, not a lost record"
        );
    }

    #[test]
    fn a_bracket_owned_by_another_thread_is_still_an_unbound_loss() {
        let foreign = OuterContext {
            combat_id: 4,
            combat_generation: 2,
            battle_slot_index_raw: 1,
            thread_id: 8,
            member_epoch_publication_sequence: 3,
            member_epoch_game_hour_raw: 123,
        };
        assert_eq!(resolve_append_bracket(Some(foreign), 7), Err(()));
        assert_eq!(
            resolve_append_bracket(
                Some(OuterContext {
                    thread_id: 0,
                    ..foreign
                }),
                7
            ),
            Err(()),
            "a bracket with no thread identity cannot be bound to this append"
        );
        assert_eq!(
            resolve_append_bracket(
                Some(OuterContext {
                    thread_id: 7,
                    ..foreign
                }),
                7
            ),
            Ok(Some(OuterContext {
                thread_id: 7,
                ..foreign
            })),
            "the ordinary naval-combat bracket is unchanged"
        );
    }

    #[test]
    fn an_unbracketed_capture_becomes_an_event_with_no_live_combat_join() {
        let mut capture = event(2);
        capture.combat_join_present = false;
        capture.combat_id = 0;
        capture.combat_generation = 0;
        capture.victim_member_battle_slot_index = 0;
        capture.member_epoch_publication_sequence = 0;
        capture.member_epoch_game_hour_raw = 0;
        capture.attacker_country_id_raw = 0;
        capture.cause_record_flag = true;
        capture.attribution_source_name = crate::ExactText::Present("SUNK_BY_MINES".into());

        let decoded = capture_to_event(capture);
        assert!(!decoded.combat_join_present);
        assert_eq!(
            (
                decoded.combat_id,
                decoded.combat_generation,
                decoded.victim_member_battle_slot_index
            ),
            (0, 0, 0),
            "an unbracketed sinking must not carry a combat identity it never had"
        );
        crate::validate_snapshot(&crate::SunkShipSnapshot {
            source_sequence: decoded.event_sequence,
            producer_complete: true,
            records_with_unavailable_text: 0,
            producer_hour_raw: None,
            producer_loss: None,
            events: vec![decoded],
        })
        .expect("a mine sinking with no attacker and no combat join is a complete row");
    }

    #[test]
    fn a_bracketed_capture_still_carries_its_exact_combat_join() {
        let decoded = capture_to_event(event(2));
        assert!(decoded.combat_join_present);
        assert_eq!((decoded.combat_id, decoded.combat_generation), (4, 2));
        assert_eq!(decoded.victim_member_battle_slot_index, 1);
    }

    #[test]
    fn the_evidence_fixture_accepts_an_unbracketed_sinking_but_refuses_a_half_join() {
        let mut unbracketed = event(2);
        unbracketed.combat_join_present = false;
        unbracketed.combat_id = 0;
        unbracketed.combat_generation = 0;
        unbracketed.victim_member_battle_slot_index = 0;
        unbracketed.member_epoch_publication_sequence = 0;
        unbracketed.member_epoch_game_hour_raw = 0;
        unbracketed.attacker_country_id_raw = 0;
        unbracketed.cause_record_flag = true;

        let mut drain = candidate();
        drain.events = vec![unbracketed.clone()];
        assert!(
            drain.evidence_fixture_contract_satisfied(),
            "an accident or mine sinking is exactly what the fixture is for"
        );

        let mut stale = unbracketed.clone();
        stale.combat_id = 4;
        drain.events = vec![stale];
        assert!(
            !drain.evidence_fixture_contract_satisfied(),
            "a row that says it has no join may not carry one anyway"
        );

        let mut no_attacker_but_not_a_cause_record = unbracketed;
        no_attacker_but_not_a_cause_record.cause_record_flag = false;
        drain.events = vec![no_attacker_but_not_a_cause_record];
        assert!(
            !drain.evidence_fixture_contract_satisfied(),
            "only a cause record may omit its attacking country"
        );
    }

    fn baseline_event(index: u32) -> crate::SunkShipEvent {
        crate::SunkShipEvent {
            event_sequence: u64::from(index) + 2,
            combat_join_present: false,
            combat_id: 0,
            combat_generation: 0,
            victim_country_id_raw: 3,
            attacker_country_id_raw: 9,
            ui_source_country_id_raw: 5,
            sink_date_raw: 16_959_300,
            victim_member_battle_slot_index: 0,
            flags: crate::EVENT_FLAG_KILLED,
            victim_ship_reference_raw: [11, 12],
            damage_source_reference_raw: [13, 14],
            task_force_id_raw: 15,
            terminal_context_raw: [16, 17],
            cause_record_flag: false,
            ship_name: crate::ExactText::Present(format!("HMS Exact {index}")),
            ship_class_name: crate::ExactText::Present("Destroyer".into()),
            attribution_source_name: crate::ExactText::Present(String::new()),
            attribution_source_class_name: crate::ExactText::Present(String::new()),
        }
    }
}
