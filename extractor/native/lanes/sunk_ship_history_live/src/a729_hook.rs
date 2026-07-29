//! Exact-a729 A33 same-thread writer hook with a raw evidence-only gate.
//!
//! Production install first decodes the complete existing `CGameState+0x590`
//! vector into pointer-free rows, then requires the exact vector header, order,
//! and decoded rows to remain unchanged across detour activation. An explicit
//! owner-run environment gate may install the same byte-checked detours solely
//! to write raw activation evidence; it cannot emit D86. The outer CFEXMember
//! writer establishes the exact d49/d50 epoch/slot in TLS. All three global
//! history writer wrappers are observed after their originals return; each
//! callback requires exactly one new vector entry and then copies that completed
//! CSunkShipInfo. Callback capture is bounded, capacity-bounded, and any loss
//! poisons completeness.

#![cfg(windows)]

use core::{cell::Cell, ffi::c_void, mem, ptr};
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released,
    observe_released_set, ExpectedRvaHook, HookTeardownAttestation, InstallAbortDiscard,
    InstalledRvaHookReceipt, ReleasedRvaHookReceipt, MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
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
// Sequence 1 is the authenticated empty baseline. The first terminal event is
// 2 so a source-sequence consumer cannot confuse empty history with one row.
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(2);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static NEXT_DRAIN_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static DROPPED_REENTRANT: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
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

#[derive(Clone, Copy)]
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
    pub ship_name: String,
    pub ship_class_name: String,
    pub attribution_source_name: String,
    pub attribution_source_class_name: String,
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
    /// Exact `CGameState+0x590` pointer-vector was structurally valid and empty before
    /// any detour was enabled. This is the only currently admitted complete
    /// pre-hook baseline for an activation fixture.
    pub baseline_measured_empty: bool,
    /// The same vector was measured empty again after the outer detour and all
    /// three post-append writer detours were active, closing the pre-hook
    /// observation race.
    pub baseline_revalidated_empty: bool,
    /// A complete pointer-free decode of the existing history vector was
    /// accepted before the production detours were enabled.
    pub baseline_complete: bool,
    /// The exact vector header and pointer-free rows were unchanged after all
    /// production detours were enabled.
    pub baseline_revalidated_complete: bool,
    /// True only for the explicit owner-run evidence gate. Such a drain is raw
    /// activation evidence and can never be encoded as an authoritative D86.
    pub evidence_install: bool,
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
                    && event.member_epoch_publication_sequence != 0
                    && event.member_epoch_game_hour_raw > 0
                    && event.combat_id != 0
                    && event.combat_generation != 0
                    && event.victim_country_id_raw != 0
                    && event.attacker_country_id_raw != 0
                    && event.sink_date_raw > 0
                    && !(event.cause_record_flag && event.assisted_kill)
                    && event.victim_ship_reference_raw != [0, 0]
                    && !event.ship_name.is_empty()
                    && !event.ship_class_name.is_empty()
                    && event.ship_name.len() < MAX_CAPTURE_TEXT
                    && event.ship_class_name.len() < MAX_CAPTURE_TEXT
                    && event.attribution_source_name.len() < MAX_CAPTURE_TEXT
                    && event.attribution_source_class_name.len() < MAX_CAPTURE_TEXT
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
    dropped_unbound: u64,
    dropped_reentrant: u64,
    dropped_unreadable: u64,
    dropped_lock_busy: u64,
    dropped_overflow: u64,
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

/// Explicit nonproduction receipt mode. It never changes ordinary product
/// scheduling and is reachable only through the isolated evidence opt-in.
pub fn install_for_evidence() -> Result<(), InstallError> {
    if !evidence_activation_enabled() {
        return Err(InstallError::LiveFixtureNotAccepted);
    }
    install_promoted(true).map_err(InstallError::Runtime)
}

/// The evidence gate is deliberately distinct from both production latches.
/// It only enables raw hook telemetry and never changes D86 scheduling,
/// completeness, authority, or [`crate::collect_and_encode`] admission.
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
    DROPPED_REENTRANT.store(0, Ordering::Release);
    DROPPED_UNREADABLE.store(0, Ordering::Release);
    DROPPED_LOCK_BUSY.store(0, Ordering::Release);
    DROPPED_OVERFLOW.store(0, Ordering::Release);
    COMPLETENESS_POISONED.store(false, Ordering::Release);
    BASELINE_MEASURED_EMPTY.store(baseline_measured_empty, Ordering::Release);
    BASELINE_COMPLETE.store(!evidence_install, Ordering::Release);
    BASELINE_REVALIDATED_COMPLETE.store(false, Ordering::Release);
    EVIDENCE_INSTALL.store(evidence_install, Ordering::Release);
    for (target, hook_bit) in [
        (cause_writer, HOOK_CAUSE),
        (combat_source_writer, HOOK_COMBAT_SOURCE),
        (attributed_writer, HOOK_ATTRIBUTED),
        (outer, HOOK_OUTER),
    ] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            return Err(finish_failed_activation(
                format!("A33 enable failed at 0x{target:X}: {status}"),
                cleanup_after_failed_install(base, &targets),
            ));
        }
        ENABLED_HOOKS.fetch_or(hook_bit, Ordering::AcqRel);
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

/// Revalidate an externally retained copy of the complete pre-hook history.
/// Production installation already decodes and publishes the exact vector; a
/// caller may only confirm that same pointer-free baseline, never replace it.
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
    let snapshot = crate::SunkShipSnapshot {
        source_sequence: events.last().map_or(1, |event| event.event_sequence),
        producer_complete: true,
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

/// Must run immediately before the collector replaces the d49/d50 member map.
/// Thus callbacks produced under the old map are drained before publication of
/// the next one. Any loss or missing baseline destroys the complete history.
pub fn drain_before_member_epoch_republish() -> Result<(), String> {
    if !crate::PRODUCTION_SCHEDULING_ENABLED || EVIDENCE_INSTALL.load(Ordering::Acquire) {
        return Ok(());
    }
    let reservation = peek_complete_snapshot()?;
    commit_drain_to_history(reservation.reservation_id)
}

fn capture_to_event(capture: RawSunkShipCapture) -> crate::SunkShipEvent {
    crate::SunkShipEvent {
        event_sequence: capture.event_sequence,
        combat_join_present: true,
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
    if !history.baseline_complete || COMPLETENESS_POISONED.load(Ordering::Acquire) {
        return Err("A33 complete history is unavailable".into());
    }
    Ok(crate::SunkShipSnapshot {
        source_sequence: history
            .events
            .last()
            .map_or(1, |event| event.event_sequence),
        producer_complete: true,
        events: history.events.clone(),
    })
}

/// Reserve the exact pending writer prefix and return the complete D86
/// snapshot that would result from committing that prefix. Neither the raw
/// queue nor the complete-history baseline is mutated until
/// [`commit_drain_to_history`] succeeds after authoritative sink publication.
pub fn peek_complete_snapshot() -> Result<CompleteSnapshotReservation, String> {
    let reservation = peek_drain()?;
    let reservation_id = reservation.reservation_id;
    let result = (|| {
        if let Some(snapshot) = sealed_complete_snapshot_for(reservation_id)? {
            return Ok(CompleteSnapshotReservation {
                reservation_id,
                snapshot,
            });
        }
        validate_complete_history_drain(&reservation.drain)?;
        let snapshot = complete_snapshot_with_raw_extension(&reservation.drain.events)?;
        Ok(CompleteSnapshotReservation {
            reservation_id,
            snapshot,
        })
    })();
    match result {
        Ok(reservation) => Ok(reservation),
        Err(error) => Err(rollback_drain_error(reservation_id, error)),
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
    let state = DrainReservationState {
        reservation_id,
        row_count: queue.pending.len(),
        dropped_unbound: DROPPED_UNBOUND.load(Ordering::Acquire),
        dropped_reentrant: DROPPED_REENTRANT.load(Ordering::Acquire),
        dropped_unreadable: DROPPED_UNREADABLE.load(Ordering::Acquire),
        dropped_lock_busy: DROPPED_LOCK_BUSY.load(Ordering::Acquire),
        dropped_overflow: DROPPED_OVERFLOW.load(Ordering::Acquire),
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
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
        baseline_measured_empty: BASELINE_MEASURED_EMPTY.load(Ordering::Acquire),
        baseline_revalidated_empty: BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire),
        baseline_complete: BASELINE_COMPLETE.load(Ordering::Acquire),
        baseline_revalidated_complete: BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire),
        evidence_install: EVIDENCE_INSTALL.load(Ordering::Acquire),
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

/// Commit a reserved raw prefix and its corresponding complete-history
/// extension as one lane-owned transaction. All fallible validation happens
/// before either store is mutated, so a caller can safely retry after
/// [`rollback_drain`] when publication fails.
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
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
        baseline_measured_empty: BASELINE_MEASURED_EMPTY.load(Ordering::Acquire),
        baseline_revalidated_empty: BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire),
        baseline_complete: BASELINE_COMPLETE.load(Ordering::Acquire),
        baseline_revalidated_complete: BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire),
        evidence_install: EVIDENCE_INSTALL.load(Ordering::Acquire),
    };
    validate_complete_history_drain(&drain)?;
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

fn validate_complete_history_drain(drain: &HookDrain) -> Result<(), String> {
    if drain.evidence_install
        || !drain.baseline_complete
        || !drain.baseline_revalidated_complete
        || drain.completeness_poisoned
        || drain.dropped_unbound != 0
        || drain.dropped_reentrant != 0
        || drain.dropped_unreadable != 0
        || drain.dropped_lock_busy != 0
        || drain.dropped_overflow != 0
    {
        return Err("A33 hook telemetry lost complete-history authority".into());
    }
    Ok(())
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
            completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
            baseline_measured_empty: BASELINE_MEASURED_EMPTY.load(Ordering::Acquire),
            baseline_revalidated_empty: BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire),
            baseline_complete: BASELINE_COMPLETE.load(Ordering::Acquire),
            baseline_revalidated_complete: BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire),
            evidence_install: EVIDENCE_INSTALL.load(Ordering::Acquire),
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

/// Restore every owned hook site, quiesce callbacks, and reserve the terminal
/// raw prefix without consuming it. The caller must commit only after the
/// terminal sink accepts the payload, or roll back on every failure path.
pub fn seal_reserve_and_uninstall() -> Result<HookDrainReservation, String> {
    seal_reserve_and_uninstall_inner(None, None)
}

/// Seals, restores and discards the whole D86 carrier for a production hook
/// registry that could not complete its install.
///
/// The normal teardown only consumes an empty reservation, because a non-empty
/// one means observations that the terminal DATA frame owes to the sink. A
/// partial install has no terminal frame and never will, so here a non-empty
/// reservation is discarded with an exact count instead of being refused
/// forever. Physical restoration, quiescence and release-receipt observation
/// are unchanged, and a failed discard still rolls the reservation back so the
/// exact carrier stays retryable.
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

/// Complete teardown around an already active reservation. This lets the
/// collector build the terminal complete snapshot first, then stop callbacks
/// without releasing or replacing that exact queue prefix.
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
    validate_complete_history_drain(&reservation.drain)?;
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
        };
        if !active.same_reserved_prefix(reserved_prefix) && queue.pending.len() < active.row_count {
            return Err("A33 terminal raw prefix was truncated while hooks quiesced".into());
        }
        (
            reserved_prefix,
            queue.pending.iter().cloned().collect::<Vec<_>>(),
        )
    };
    if COMPLETENESS_POISONED.load(Ordering::Acquire)
        || reserved_prefix.dropped_unbound != 0
        || reserved_prefix.dropped_reentrant != 0
        || reserved_prefix.dropped_unreadable != 0
        || reserved_prefix.dropped_lock_busy != 0
        || reserved_prefix.dropped_overflow != 0
    {
        return Err("A33 terminal hook telemetry lost complete-history authority".into());
    }
    let mut snapshot = {
        let history = HISTORY
            .get_or_init(|| Mutex::new(HistoryState::default()))
            .lock()
            .map_err(|_| "A33 history lock poisoned while sealing complete snapshot")?;
        if !history.baseline_complete {
            return Err("A33 complete-history baseline is absent while sealing".into());
        }
        crate::SunkShipSnapshot {
            source_sequence: history
                .events
                .last()
                .map_or(1, |event| event.event_sequence),
            producer_complete: true,
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
    // From this point onward the full roster is no longer live. Durable phase,
    // mode, per-site masks, and indexed receipts retain teardown custody.
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
    for (writer, hook_bit) in [
        (attributed_writer, HOOK_ATTRIBUTED),
        (combat_source_writer, HOOK_COMBAT_SOURCE),
        (cause_writer, HOOK_CAUSE),
    ] {
        if enabled & hook_bit != 0 {
            disable(writer)?;
        }
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
        for (target, hook_bit) in [
            (cause_writer, HOOK_CAUSE),
            (combat_source_writer, HOOK_COMBAT_SOURCE),
            (attributed_writer, HOOK_ATTRIBUTED),
            (outer, HOOK_OUTER),
        ] {
            if created & hook_bit == 0 {
                continue;
            }
            match unsafe { MinHook::remove_hook(target as *mut c_void) } {
                Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                Err(status) => return Err(format!("A33 remove failed: {status}")),
            }
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
    match unsafe { MinHook::disable_hook(target as *mut c_void) } {
        Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => Ok(()),
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
    for &target in targets {
        disable(target)?;
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
    for &target in targets.iter().rev() {
        if let Err(status) = unsafe { MinHook::remove_hook(target as *mut c_void) } {
            if status != MH_STATUS::MH_ERROR_NOT_CREATED {
                failures.push(format!("remove 0x{target:X} failed: {status}"));
            }
        }
    }
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
    let resolution =
        naval_combat_live_lane::try_current_member_epoch_snapshot().and_then(|snapshot| {
            let receipt = snapshot.receipt();
            snapshot
                .resolve_member(a)
                .ok()
                .map(|resolved| (resolved, receipt))
        });
    if resolution.is_none() {
        poison(&DROPPED_UNBOUND);
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

fn observe_writer_append(game_state: usize, before_count: Option<u32>) {
    let Some(context) = OUTER_CONTEXT.with(Cell::get) else {
        poison(&DROPPED_UNBOUND);
        return;
    };
    if context.thread_id == 0 || context.thread_id != unsafe { GetCurrentThreadId() } {
        poison(&DROPPED_UNBOUND);
        return;
    }
    let singleton = MODULE_BASE
        .load(Ordering::Acquire)
        .checked_add(CGAMESTATE_SINGLETON_RVA)
        .and_then(read_value::<usize>);
    if singleton != Some(game_state) {
        poison(&DROPPED_UNBOUND);
        return;
    }
    let nested = IN_CALLBACK.with(|flag| flag.replace(true));
    if nested {
        poison(&DROPPED_REENTRANT);
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

fn capture_record(address: usize, context: OuterContext) -> Option<RawSunkShipCapture> {
    if address == 0 {
        return None;
    }
    let vtable: usize = read_value(address)?;
    if vtable != MODULE_BASE.load(Ordering::Acquire) + SUNK_SHIP_INFO_VTABLE_RVA {
        return None;
    }
    let ship_name = read_msvc_string(address + 0x08, true)?;
    let class_ptr: usize = read_value(address + 0x68)?;
    let ship_class_name = read_msvc_string(class_ptr.checked_add(0x68)?, true)?;
    // The serializer admits an empty attacker name for sources without a
    // concrete attacking ship. Preserve that exact absence rather than
    // poisoning an otherwise readable sunk-history event.
    let attribution_source_name = read_msvc_string(address + 0x28, false)?;
    let attribution_class_ptr: usize = read_value(address + 0x70)?;
    let attribution_source_class_name = if attribution_class_ptr == 0 {
        String::new()
    } else {
        read_msvc_string(attribution_class_ptr.checked_add(0x68)?, false)?
    };
    // +0x60 is the following embedded CGameDate vtable, not the date scalar.
    // Re-read the singleton date on the same callback thread so a future
    // layout drift cannot silently turn that vtable into an apparently valid
    // positive sink timestamp.
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
        thread_id: context.thread_id,
        member_epoch_publication_sequence: context.member_epoch_publication_sequence,
        member_epoch_game_hour_raw: context.member_epoch_game_hour_raw,
        combat_id: context.combat_id,
        combat_generation: context.combat_generation,
        victim_member_battle_slot_index: context.battle_slot_index_raw,
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

fn read_msvc_string(address: usize, required: bool) -> Option<String> {
    let len: usize = read_value(address + 0x10)?;
    let cap: usize = read_value(address + 0x18)?;
    if len == 0 {
        return if required { None } else { Some(String::new()) };
    }
    if len >= MAX_CAPTURE_TEXT || cap < len {
        return None;
    }
    let data = if cap < 16 {
        address
    } else {
        read_value::<usize>(address)?
    };
    let mut bytes = [0u8; MAX_CAPTURE_TEXT];
    read_exact(data, &mut bytes[..len])?;
    if bytes[..len].contains(&0) {
        return None;
    }
    String::from_utf8(bytes[..len].to_vec()).ok()
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

    let mut record_addresses = Vec::with_capacity(count as usize);
    let mut unique_addresses = BTreeSet::new();
    let mut events = Vec::with_capacity(count as usize);
    for index in 0..count {
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
        // Name the record and the field. The bare message this replaced could not
        // distinguish 2,690 good records from one bad one.
        let event = read_baseline_record(module_base, record, event_sequence, current_date)
            .map_err(|why| {
                format!(
                    "A33 existing sunk-history record {index} of {count} at 0x{record:016x} is unusable: {why}"
                )
            })?;
        record_addresses.push(record);
        events.push(event);
    }

    // Re-read both the vector header and every pointer slot. A writer racing
    // attach must not turn a prefix read into a claimed complete baseline.
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

/// Decodes one existing `CSunkShipInfo`.
///
/// Every refusal names the field that refused. This previously returned a bare
/// `Option`, so roughly ten distinct causes -- vtable mismatch, an over-long or
/// non-UTF-8 string, a sink date outside the campaign, an out-of-range flag byte,
/// any unreadable scalar -- collapsed into one `None` and the caller could only
/// say "incomplete or unreadable". A live capture then failed on a campaign whose
/// save holds 2,690 well-formed sunk-ship records, with no way to tell which
/// record or which field had refused.
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
    let ship_name = field(read_msvc_string(address + 0x08, true), "ship name (+0x08)")?;
    let class_ptr: usize = field(read_value(address + 0x68), "ship class pointer (+0x68)")?;
    let ship_class_name = field(
        class_ptr
            .checked_add(0x68)
            .and_then(|at| read_msvc_string(at, true)),
        "ship class name (class+0x68)",
    )?;
    let attribution_source_name = field(
        read_msvc_string(address + 0x28, false),
        "attribution source name (+0x28)",
    )?;
    let attribution_class_ptr: usize =
        field(read_value(address + 0x70), "attribution class pointer (+0x70)")?;
    let attribution_source_class_name = if attribution_class_ptr == 0 {
        String::new()
    } else {
        field(
            attribution_class_ptr
                .checked_add(0x68)
                .and_then(|at| read_msvc_string(at, false)),
            "attribution source class name (attribution class+0x68)",
        )?
    };
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
    let cause_record_flag_raw: u8 =
        field(read_value(address + 0xa0), "cause-record flag (+0xa0)")?;
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
        attacker_country_id_raw: field(
            read_value(address + 0x4c),
            "attacker country id (+0x4c)",
        )?,
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
            field(read_value(address + 0x7c), "victim ship reference lo (+0x7c)")?,
            field(read_value(address + 0x80), "victim ship reference hi (+0x80)")?,
        ],
        damage_source_reference_raw: [
            field(read_value(address + 0x84), "damage source reference lo (+0x84)")?,
            field(read_value(address + 0x88), "damage source reference hi (+0x88)")?,
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

fn poison(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        drain.events[0].attribution_source_name.clear();
        drain.events[0].attribution_source_class_name.clear();
        assert!(drain.evidence_fixture_contract_satisfied());
    }
}
