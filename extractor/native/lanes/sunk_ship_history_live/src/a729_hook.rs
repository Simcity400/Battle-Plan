//! Exact-a729 A33 same-thread writer hook with a raw evidence-only gate.
//!
//! Production installation remains blocked while either promotion latch in the
//! parent module is false. An explicit owner-run environment gate may install
//! the same byte-checked detours solely to write raw activation evidence; it
//! cannot emit D86 or satisfy either latch. The outer CFEXMember writer
//! establishes the exact d49/d50 epoch/slot in TLS; the nested CSunkShipInfo
//! constructor is observed only after its original returns. Callback capture
//! is bounded, capacity-bounded, and any loss poisons completeness.

#![cfg(windows)]

use core::{cell::Cell, ffi::c_void, mem, ptr};
use minhook::{MinHook, MH_STATUS};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const OUTER_CFEX_MEMBER_WRITER_RVA: usize = 0x1961_DD0;
pub const RECORD_CONSTRUCTOR_RVA: usize = 0x0C1_DF70;
pub const SUNK_SHIP_INFO_VTABLE_RVA: usize = 0x294_1608;
pub const MAX_PENDING_EVENTS: usize = 65_536;
pub const MAX_CAPTURE_TEXT: usize = 256;
pub const EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_A33_SUNK_SHIP_EVIDENCE";

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const SUNK_SHIP_HISTORY_VECTOR_OFFSET: usize = 0x590;

const OUTER_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24,
    0x10, 0x48,
];
const CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
    0x20, 0x57,
];

type OuterFn = unsafe extern "system" fn(usize, usize, usize, usize, u8);
type ConstructorFn = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize) -> usize;

static ORIGINAL_OUTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CONSTRUCTOR: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
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
    pub victim_country_identity_raw: [u32; 2],
    pub killer_country_id_raw: u32,
    pub sink_date_raw: i32,
    pub assisted_kill: bool,
    pub ship_name: String,
    pub ship_class_name: String,
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
    /// either detour was enabled. This is the only currently admitted complete
    /// pre-hook baseline for an activation fixture.
    pub baseline_measured_empty: bool,
    /// The same vector was measured empty again after both detours were active,
    /// closing the pre-hook observation race.
    pub baseline_revalidated_empty: bool,
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
                event.event_sequence != 0
                    && event.thread_id != 0
                    && event.member_epoch_publication_sequence != 0
                    && event.member_epoch_game_hour_raw > 0
                    && event.combat_id != 0
                    && event.combat_generation != 0
                    && event.victim_country_identity_raw != [0, 0]
                    && event.killer_country_id_raw != 0
                    && event.sink_date_raw > 0
                    && !event.ship_name.is_empty()
                    && !event.ship_class_name.is_empty()
                    && event.ship_name.len() < MAX_CAPTURE_TEXT
                    && event.ship_class_name.len() < MAX_CAPTURE_TEXT
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
    let production = crate::PRODUCTION_SCHEDULING_ENABLED && crate::LIVE_FIXTURE_ACCEPTED;
    let evidence = evidence_activation_enabled();
    if !production && !evidence && !crate::PRODUCTION_SCHEDULING_ENABLED {
        return Err(InstallError::ProductionSchedulingDisabled);
    }
    if !production && !evidence {
        return Err(InstallError::LiveFixtureNotAccepted);
    }
    install_promoted(evidence && !production).map_err(InstallError::Runtime)
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
    let constructor = base + RECORD_CONSTRUCTOR_RVA;
    verify_sentinel(outer, &OUTER_SENTINEL)?;
    verify_sentinel(constructor, &CONSTRUCTOR_SENTINEL)?;
    let baseline_measured_empty = measure_empty_history_baseline(base)?;
    if !evidence_install && !baseline_measured_empty {
        return Err("A33 production install requires a measured-empty complete-history baseline".into());
    }
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
    let original_constructor = match unsafe {
        MinHook::create_hook(constructor as *mut c_void, hook_constructor as *mut c_void)
    } {
        Ok(original) => original,
        Err(status) => {
            let _ = unsafe { MinHook::remove_hook(outer as *mut c_void) };
            return Err(format!("A33 constructor create failed: {status}"));
        }
    };
    ORIGINAL_OUTER.store(original_outer as usize, Ordering::Release);
    ORIGINAL_CONSTRUCTOR.store(original_constructor as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(hook_epoch, Ordering::Release);
    NEXT_SEQUENCE.store(1, Ordering::Release);
    DROPPED_UNBOUND.store(0, Ordering::Release);
    DROPPED_REENTRANT.store(0, Ordering::Release);
    DROPPED_UNREADABLE.store(0, Ordering::Release);
    DROPPED_LOCK_BUSY.store(0, Ordering::Release);
    DROPPED_OVERFLOW.store(0, Ordering::Release);
    COMPLETENESS_POISONED.store(false, Ordering::Release);
    BASELINE_MEASURED_EMPTY.store(baseline_measured_empty, Ordering::Release);
    EVIDENCE_INSTALL.store(evidence_install, Ordering::Release);
    for target in [constructor, outer] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            if let Err(cleanup) = cleanup_after_failed_install(outer, constructor) {
                // Keep the original pointers and installation marker alive so
                // collector teardown can retry without freeing an in-flight
                // trampoline.
                INSTALLED.store(true, Ordering::Release);
                return Err(format!("A33 enable failed: {status}; {cleanup}"));
            }
            ORIGINAL_OUTER.store(0, Ordering::Release);
            ORIGINAL_CONSTRUCTOR.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
            EVIDENCE_INSTALL.store(false, Ordering::Release);
            BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
            BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
            return Err(format!("A33 enable failed: {status}"));
        }
    }
    let baseline_revalidated_empty = match measure_empty_history_baseline(base) {
        Ok(empty) => empty,
        Err(error) => {
            if let Err(cleanup) = cleanup_after_failed_install(outer, constructor) {
                INSTALLED.store(true, Ordering::Release);
                return Err(format!(
                    "A33 post-enable baseline revalidation failed: {error}; {cleanup}"
                ));
            }
            ORIGINAL_OUTER.store(0, Ordering::Release);
            ORIGINAL_CONSTRUCTOR.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
            EVIDENCE_INSTALL.store(false, Ordering::Release);
            BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
            BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
            return Err(format!("A33 post-enable baseline revalidation failed: {error}"));
        }
    };
    BASELINE_REVALIDATED_EMPTY.store(baseline_revalidated_empty, Ordering::Release);
    if !evidence_install && !baseline_revalidated_empty {
        if let Err(cleanup) = cleanup_after_failed_install(outer, constructor) {
            INSTALLED.store(true, Ordering::Release);
            return Err(format!(
                "A33 history changed while the production hooks were activating; {cleanup}"
            ));
        }
        ORIGINAL_OUTER.store(0, Ordering::Release);
        ORIGINAL_CONSTRUCTOR.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
        EVIDENCE_INSTALL.store(false, Ordering::Release);
        BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
        BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
        return Err("A33 history changed while the production hooks were activating".into());
    }
    if !evidence_install {
        let mut history = match HISTORY
            .get_or_init(|| Mutex::new(HistoryState::default()))
            .lock()
        {
            Ok(history) => history,
            Err(_) => {
                if let Err(cleanup) = cleanup_after_failed_install(outer, constructor) {
                    INSTALLED.store(true, Ordering::Release);
                    return Err(format!(
                        "A33 history lock poisoned during production install; {cleanup}"
                    ));
                }
                ORIGINAL_OUTER.store(0, Ordering::Release);
                ORIGINAL_CONSTRUCTOR.store(0, Ordering::Release);
                MODULE_BASE.store(0, Ordering::Release);
                ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
                BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
                BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
                return Err("A33 history lock poisoned during production install".into());
            }
        };
        *history = HistoryState {
            baseline_complete: true,
            events: Vec::new(),
        };
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Publish the complete pre-hook history before any live event is admitted.
/// The live fixture must authenticate this baseline; an empty vector is valid
/// only when the fixture proves measured-empty history.
pub fn publish_complete_baseline(events: Vec<crate::SunkShipEvent>) -> Result<(), String> {
    if !crate::PRODUCTION_SCHEDULING_ENABLED || !crate::LIVE_FIXTURE_ACCEPTED {
        return Err("A33 live fixture has not authenticated a complete baseline".into());
    }
    if !INSTALLED.load(Ordering::Acquire)
        || EVIDENCE_INSTALL.load(Ordering::Acquire)
        || !BASELINE_MEASURED_EMPTY.load(Ordering::Acquire)
        || !BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire)
    {
        return Err(
            "A33 complete baseline requires an installed production hook and measured-empty history"
                .into(),
        );
    }
    if !events.is_empty() {
        return Err(
            "A33 production baseline publication currently admits only measured-empty history"
                .into(),
        );
    }
    let snapshot = crate::SunkShipSnapshot {
        source_sequence: events.last().map_or(1, |event| event.event_sequence),
        producer_complete: true,
        events,
    };
    crate::validate_snapshot(&snapshot).map_err(|error| format!("A33 baseline invalid: {error:?}"))?;
    *HISTORY
        .get_or_init(|| Mutex::new(HistoryState::default()))
        .lock()
        .map_err(|_| "A33 history lock poisoned")? = HistoryState {
        baseline_complete: true,
        events: snapshot.events,
    };
    COMPLETENESS_POISONED.store(false, Ordering::Release);
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
            combat_id: capture.combat_id,
            combat_generation: capture.combat_generation,
            victim_country_identity_raw: capture.victim_country_identity_raw,
            killer_country_id_raw: capture.killer_country_id_raw,
            sink_date_raw: capture.sink_date_raw,
            victim_member_battle_slot_index: capture.victim_member_battle_slot_index,
            flags: if capture.assisted_kill {
                crate::EVENT_FLAG_ASSISTED_KILL
            } else {
                crate::EVENT_FLAG_KILLED
            },
            ship_name: capture.ship_name,
            ship_class_name: capture.ship_class_name,
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
        || !BASELINE_MEASURED_EMPTY.load(Ordering::Acquire)
        || !BASELINE_REVALIDATED_EMPTY.load(Ordering::Acquire)
    {
        return Err("A33 production hook is not installed with a measured-empty baseline".into());
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
    let constructor = base + RECORD_CONSTRUCTOR_RVA;
    disable(outer)?;
    wait_for_callbacks()?;
    disable(constructor)?;
    wait_for_callbacks()?;
    let drained = drain()?;
    for target in [constructor, outer] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A33 remove failed: {status}")),
        }
    }
    INSTALLED.store(false, Ordering::Release);
    ORIGINAL_OUTER.store(0, Ordering::Release);
    ORIGINAL_CONSTRUCTOR.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    EVIDENCE_INSTALL.store(false, Ordering::Release);
    BASELINE_MEASURED_EMPTY.store(false, Ordering::Release);
    BASELINE_REVALIDATED_EMPTY.store(false, Ordering::Release);
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

fn cleanup_after_failed_install(outer: usize, constructor: usize) -> Result<(), String> {
    let outer_disable = disable(outer);
    let constructor_disable = disable(constructor);
    outer_disable?;
    constructor_disable?;
    wait_for_callbacks()?;
    for target in [constructor, outer] {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A33 rollback remove failed: {status}")),
        }
    }
    Ok(())
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

unsafe extern "system" fn hook_constructor(
    out: usize,
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
) -> usize {
    let _active = CallbackGuard::enter();
    let original: ConstructorFn = mem::transmute(ORIGINAL_CONSTRUCTOR.load(Ordering::Acquire));
    let result = original(out, a, b, c, d, e);
    let Some(context) = OUTER_CONTEXT.with(Cell::get) else {
        return result;
    };
    if context.thread_id == 0 || context.thread_id != unsafe { GetCurrentThreadId() } {
        poison(&DROPPED_UNBOUND);
        return result;
    }
    let nested = IN_CALLBACK.with(|flag| flag.replace(true));
    if nested {
        poison(&DROPPED_REENTRANT);
        return result;
    }
    if let Some(capture) = capture_record(result, context) {
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
    result
}

fn capture_record(address: usize, context: OuterContext) -> Option<RawSunkShipCapture> {
    if address == 0 {
        return None;
    }
    let vtable: usize = read_value(address)?;
    if vtable != MODULE_BASE.load(Ordering::Acquire) + SUNK_SHIP_INFO_VTABLE_RVA {
        return None;
    }
    let ship_name = read_msvc_string(address + 0x08)?;
    let class_ptr: usize = read_value(address + 0x68)?;
    let ship_class_name = read_msvc_string(class_ptr.checked_add(0x68)?)?;
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
        victim_country_identity_raw: [read_value(address + 0x48)?, read_value(address + 0x4c)?],
        killer_country_id_raw: read_value(address + 0x78)?,
        sink_date_raw: read_value(address + 0x60)?,
        assisted_kill: read_value::<u8>(address + 0xa1)? != 0,
        ship_name,
        ship_class_name,
    })
}

fn read_msvc_string(address: usize) -> Option<String> {
    let len: usize = read_value(address + 0x10)?;
    let cap: usize = read_value(address + 0x18)?;
    if len == 0 || len >= MAX_CAPTURE_TEXT || cap < len {
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

fn measure_empty_history_baseline(module_base: usize) -> Result<bool, String> {
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
    if count != 0 && data == 0 {
        return Err("A33 nonempty history vector has a null data pointer".into());
    }
    Ok(count == 0)
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
            victim_country_identity_raw: [5, 6],
            killer_country_id_raw: 8,
            sink_date_raw: 123,
            assisted_kill: false,
            ship_name: "HMS Exact".into(),
            ship_class_name: "Destroyer".into(),
        }
    }

    fn candidate() -> HookDrain {
        HookDrain {
            hook_epoch: 9,
            events: vec![event(1)],
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
        drain.events.push(event(1));
        assert!(!drain.evidence_fixture_contract_satisfied());
    }
}
