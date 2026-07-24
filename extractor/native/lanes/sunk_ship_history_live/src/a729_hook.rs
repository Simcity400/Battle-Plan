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
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24,
    0x10, 0x48,
];
const RECORD_WRITER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
    0x20, 0x57,
];

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
static STATE: OnceLock<Mutex<VecDeque<RawSunkShipCapture>>> = OnceLock::new();
static HISTORY: OnceLock<Mutex<HistoryState>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

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
            && self.events.windows(2).all(|pair| {
                pair[0].event_sequence < pair[1].event_sequence
            })
    }
}

#[derive(Default)]
struct HistoryState {
    baseline_complete: bool,
    events: Vec<crate::SunkShipEvent>,
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
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A33 install lock poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
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
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)))
        .lock()
        .map_err(|_| "A33 queue lock poisoned before install")?
        .clear();
    let original_outer = unsafe {
        MinHook::create_hook(outer as *mut c_void, hook_outer as *mut c_void)
    }
    .map_err(|status| format!("A33 outer create failed: {status}"))?;
    let original_attributed_writer = match unsafe {
        MinHook::create_hook(
            attributed_writer as *mut c_void,
            hook_attributed_writer as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(outer as *mut c_void) };
            return Err(format!("A33 attributed writer create failed: {status}"));
        }
    };
    let original_combat_source_writer = match unsafe {
        MinHook::create_hook(
            combat_source_writer as *mut c_void,
            hook_combat_source_writer as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            remove_hooks_best_effort(&[attributed_writer, outer]);
            return Err(format!("A33 combat-source writer create failed: {status}"));
        }
    };
    let original_cause_writer = match unsafe {
        MinHook::create_hook(cause_writer as *mut c_void, hook_cause_writer as *mut c_void)
    } {
        Ok(original) => original,
        Err(status) => {
            remove_hooks_best_effort(&[combat_source_writer, attributed_writer, outer]);
            return Err(format!("A33 cause writer create failed: {status}"));
        }
    };
    ORIGINAL_OUTER.store(original_outer as usize, Ordering::Release);
    ORIGINAL_ATTRIBUTED_WRITER.store(original_attributed_writer as usize, Ordering::Release);
    ORIGINAL_COMBAT_SOURCE_WRITER.store(
        original_combat_source_writer as usize,
        Ordering::Release,
    );
    ORIGINAL_CAUSE_WRITER.store(original_cause_writer as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(hook_epoch, Ordering::Release);
    let next_sequence = u64::try_from(baseline_before.events.len())
        .ok()
        .and_then(|count| count.checked_add(2))
        .ok_or("A33 baseline event sequence exhausted")?;
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
    for target in [cause_writer, combat_source_writer, attributed_writer, outer] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            if let Err(cleanup) = cleanup_after_failed_install(&targets) {
                // Keep the original pointers and installation marker alive so
                // collector teardown can retry without freeing an in-flight
                // trampoline.
                INSTALLED.store(true, Ordering::Release);
                return Err(format!("A33 enable failed: {status}; {cleanup}"));
            }
            reset_originals();
            MODULE_BASE.store(0, Ordering::Release);
            ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
            EVIDENCE_INSTALL.store(false, Ordering::Release);
            BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
            BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
            BASELINE_COMPLETE.store(false, Ordering::Release);
            BASELINE_REVALIDATED_COMPLETE.store(false, Ordering::Release);
            return Err(format!("A33 enable failed: {status}"));
        }
    }
    let baseline_after = match read_complete_history_baseline(base) {
        Ok(baseline) => baseline,
        Err(error) => {
            if let Err(cleanup) = cleanup_after_failed_install(&targets) {
                INSTALLED.store(true, Ordering::Release);
                return Err(format!(
                    "A33 post-enable baseline revalidation failed: {error}; {cleanup}"
                ));
            }
            reset_originals();
            MODULE_BASE.store(0, Ordering::Release);
            ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
            EVIDENCE_INSTALL.store(false, Ordering::Release);
            BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
            BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
            BASELINE_COMPLETE.store(false, Ordering::Release);
            BASELINE_REVALIDATED_COMPLETE.store(false, Ordering::Release);
            return Err(format!("A33 post-enable baseline revalidation failed: {error}"));
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
        if let Err(cleanup) = cleanup_after_failed_install(&targets) {
            INSTALLED.store(true, Ordering::Release);
            return Err(format!(
                "A33 history changed while the production hooks were activating; {cleanup}"
            ));
        }
        reset_originals();
        MODULE_BASE.store(0, Ordering::Release);
        ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
        EVIDENCE_INSTALL.store(false, Ordering::Release);
        BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
        BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
        BASELINE_COMPLETE.store(false, Ordering::Release);
        BASELINE_REVALIDATED_COMPLETE.store(false, Ordering::Release);
        return Err("A33 history changed while the production hooks were activating".into());
    }
    if !evidence_install {
        let mut history = match HISTORY
            .get_or_init(|| Mutex::new(HistoryState::default()))
            .lock()
        {
            Ok(history) => history,
            Err(_) => {
                if let Err(cleanup) = cleanup_after_failed_install(&targets) {
                    INSTALLED.store(true, Ordering::Release);
                    return Err(format!(
                        "A33 history lock poisoned during production install; {cleanup}"
                    ));
                }
                reset_originals();
                MODULE_BASE.store(0, Ordering::Release);
                ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
                BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
                BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
                BASELINE_COMPLETE.store(false, Ordering::Release);
                BASELINE_REVALIDATED_COMPLETE.store(false, Ordering::Release);
                return Err("A33 history lock poisoned during production install".into());
            }
        };
        *history = HistoryState {
            baseline_complete: true,
            events: baseline_after.events,
        };
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Revalidate an externally retained copy of the complete pre-hook history.
/// Production installation already decodes and publishes the exact vector; a
/// caller may only confirm that same pointer-free baseline, never replace it.
pub fn publish_complete_baseline(events: Vec<crate::SunkShipEvent>) -> Result<(), String> {
    if !crate::PRODUCTION_SCHEDULING_ENABLED {
        return Err("A33 production scheduling is disabled".into());
    }
    if !INSTALLED.load(Ordering::Acquire)
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
    crate::validate_snapshot(&snapshot).map_err(|error| format!("A33 baseline invalid: {error:?}"))?;
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
    let drained = drain()?;
    if drained.completeness_poisoned
        || drained.dropped_unbound != 0
        || drained.dropped_reentrant != 0
        || drained.dropped_unreadable != 0
        || drained.dropped_lock_busy != 0
        || drained.dropped_overflow != 0
    {
        return Err("A33 hook telemetry lost complete-history authority".into());
    }
    let mut history = HISTORY
        .get_or_init(|| Mutex::new(HistoryState::default()))
        .lock()
        .map_err(|_| "A33 history lock poisoned")?;
    if !history.baseline_complete {
        return Err("A33 complete-history baseline is absent".into());
    }
    for capture in drained.events {
        let event = crate::SunkShipEvent {
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
        };
        if history
            .events
            .last()
            .is_some_and(|previous| event.event_sequence <= previous.event_sequence)
        {
            COMPLETENESS_POISONED.store(true, Ordering::Release);
            return Err("A33 event sequence is not strictly increasing".into());
        }
        history.events.push(event);
    }
    Ok(())
}

pub fn current_complete_snapshot() -> Result<crate::SunkShipSnapshot, String> {
    if !INSTALLED.load(Ordering::Acquire)
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
        source_sequence: history.events.last().map_or(1, |event| event.event_sequence),
        producer_complete: true,
        events: history.events.clone(),
    })
}

pub fn drain() -> Result<HookDrain, String> {
    let mut queue = STATE
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)))
        .lock()
        .map_err(|_| "A33 queue lock poisoned")?;
    Ok(HookDrain {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        events: queue.drain(..).collect(),
        dropped_unbound: DROPPED_UNBOUND.swap(0, Ordering::AcqRel),
        dropped_reentrant: DROPPED_REENTRANT.swap(0, Ordering::AcqRel),
        dropped_unreadable: DROPPED_UNREADABLE.swap(0, Ordering::AcqRel),
        dropped_lock_busy: DROPPED_LOCK_BUSY.swap(0, Ordering::AcqRel),
        dropped_overflow: DROPPED_OVERFLOW.swap(0, Ordering::AcqRel),
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
        baseline_measured_empty: BASELINE_MEASURED_EMPTY.load(Ordering::Acquire),
        baseline_revalidated_empty: BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire),
        baseline_complete: BASELINE_COMPLETE.load(Ordering::Acquire),
        baseline_revalidated_complete: BASELINE_REVALIDATED_COMPLETE.load(Ordering::Acquire),
        evidence_install: EVIDENCE_INSTALL.load(Ordering::Acquire),
    })
}

pub fn seal_drain_and_uninstall() -> Result<HookDrain, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A33 teardown lock poisoned")?;
    if !INSTALLED.load(Ordering::Acquire) {
        return drain();
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let outer = base + OUTER_CFEX_MEMBER_WRITER_RVA;
    let attributed_writer = base + ATTRIBUTED_RECORD_WRITER_RVA;
    let combat_source_writer = base + COMBAT_SOURCE_RECORD_WRITER_RVA;
    let cause_writer = base + CAUSE_RECORD_WRITER_RVA;
    disable(outer)?;
    wait_for_callbacks()?;
    for writer in [attributed_writer, combat_source_writer, cause_writer] {
        disable(writer)?;
    }
    wait_for_callbacks()?;
    let drained = drain()?;
    for target in [cause_writer, combat_source_writer, attributed_writer, outer] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A33 remove failed: {status}")),
        }
    }
    INSTALLED.store(false, Ordering::Release);
    reset_originals();
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    EVIDENCE_INSTALL.store(false, Ordering::Release);
    BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
    BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
    BASELINE_COMPLETE.store(false, Ordering::Release);
    BASELINE_REVALIDATED_COMPLETE.store(false, Ordering::Release);
    naval_combat_live_lane::clear_member_epoch_snapshot()?;
    Ok(drained)
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

fn cleanup_after_failed_install(targets: &[usize]) -> Result<(), String> {
    for &target in targets {
        disable(target)?;
    }
    wait_for_callbacks()?;
    for &target in targets.iter().rev() {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A33 rollback remove failed: {status}")),
        }
    }
    Ok(())
}

fn remove_hooks_best_effort(targets: &[usize]) {
    for &target in targets {
        let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
    }
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
    let resolution = naval_combat_live_lane::try_current_member_epoch_snapshot().and_then(|snapshot| {
        let receipt = snapshot.receipt();
        snapshot.resolve_member(a).ok().map(|resolved| (resolved, receipt))
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
    let original: CauseWriterFn =
        mem::transmute(ORIGINAL_CAUSE_WRITER.load(Ordering::Acquire));
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
    let appended_record = before_count.and_then(|count| history_vector_appended_record(game_state, count));
    if let Some(capture) = appended_record.and_then(|record| capture_record(record, context)) {
        if let Some(state) = STATE.get() {
            match state.try_lock() {
                Ok(mut queue) if queue.len() < MAX_PENDING_EVENTS => queue.push_back(capture),
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
    let data = if cap < 16 { address } else { read_value::<usize>(address)? };
    let mut bytes = [0u8; MAX_CAPTURE_TEXT];
    read_exact(data, &mut bytes[..len])?;
    if bytes[..len].contains(&0) {
        return None;
    }
    String::from_utf8(bytes[..len].to_vec()).ok()
}

fn read_complete_history_baseline(
    module_base: usize,
) -> Result<HistoryVectorBaseline, String> {
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
    let capacity: u32 = read_value(vector + 0x08)
        .ok_or("A33 history vector capacity is unreadable")?;
    let count: u32 = read_value(vector + 0x0c)
        .ok_or("A33 history vector count is unreadable")?;
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
        let event = read_baseline_record(module_base, record, event_sequence, current_date)
            .ok_or("A33 existing sunk-history record is incomplete or unreadable")?;
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

fn read_baseline_record(
    module_base: usize,
    address: usize,
    event_sequence: u64,
    current_date: i32,
) -> Option<crate::SunkShipEvent> {
    if address == 0
        || read_value::<usize>(address)? != module_base + SUNK_SHIP_INFO_VTABLE_RVA
    {
        return None;
    }
    let ship_name = read_msvc_string(address + 0x08, true)?;
    let class_ptr: usize = read_value(address + 0x68)?;
    let ship_class_name = read_msvc_string(class_ptr.checked_add(0x68)?, true)?;
    let attribution_source_name = read_msvc_string(address + 0x28, false)?;
    let attribution_class_ptr: usize = read_value(address + 0x70)?;
    let attribution_source_class_name = if attribution_class_ptr == 0 {
        String::new()
    } else {
        read_msvc_string(attribution_class_ptr.checked_add(0x68)?, false)?
    };
    let sink_date_raw: i32 = read_value(address + SUNK_SHIP_INFO_SINK_DATE_OFFSET)?;
    if sink_date_raw <= 0 || sink_date_raw > current_date {
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
    Some(crate::SunkShipEvent {
        event_sequence,
        combat_join_present: false,
        combat_id: 0,
        combat_generation: 0,
        victim_country_id_raw: read_value(address + 0x48)?,
        attacker_country_id_raw: read_value(address + 0x4c)?,
        ui_source_country_id_raw: read_value(address + 0x78)?,
        sink_date_raw,
        victim_member_battle_slot_index: 0,
        flags: if assisted_kill_raw != 0 {
            crate::EVENT_FLAG_ASSISTED_KILL
        } else {
            crate::EVENT_FLAG_KILLED
        },
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

fn poison(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

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
