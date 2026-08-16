//! Exact-a729 ordered speed/pause transition capture for X12.
//!
//! All three post-mutation callbacks publish through one mutex-owned sequence
//! and queue. Queue loss and callback read failures are sticky terminal errors;
//! callers must fail closed rather than falling back to hourly anchors while
//! claiming sub-hour history.

use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released, ExpectedRvaHook,
    HookTeardownAttestation, InstallAbortDiscard, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::cell::Cell;
use std::collections::VecDeque;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const SPEED_SETTER_RVA: usize = 0x001E_DAD0;
pub const PAUSE_TOGGLE_RVA: usize = 0x00DC_6520;
pub const PAUSE_FORCED_RVA: usize = 0x00DB_E260;
/// Production-active for the pinned a729 executable. Each installation and
/// callback remains sentinel-, identity-, sequence-, and queue-integrity-gated;
/// same-hour lifecycle captures are optional regression evidence.
pub const PRODUCTION_ACTIVATION: bool = true;
pub const EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_X12_SPEED_PAUSE_HOOK";

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const CLOCK_SINGLETON_A_RVA: usize = 0x0331_72E0;
const CLOCK_SINGLETON_B_RVA: usize = 0x0331_72E8;
const CLOCK_VTABLE_RVA: usize = 0x0295_2510;
const CURRENT_HOUR: usize = 0x468;
const TICK_PROGRESS: usize = 0x4B8;
const SPEED_INDEX: usize = 0x4BC;
const PAUSED: usize = 0x6C1;
const MAX_PENDING: usize = 4096;

const SPEED_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x48, 0x83, 0xEC, 0x40, 0x4C, 0x89, 0x74, 0x24, 0x30, 0x85, 0xD2, 0x79, 0x04, 0x33,
];
const PAUSE_TOGGLE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7C, 0x24, 0x20, 0x55,
];
const PAUSE_FORCED_SENTINEL: [u8; 16] = [
    0x83, 0xEA, 0x09, 0x74, 0x15, 0x83, 0xEA, 0x01, 0x74, 0x09, 0x83, 0xFA, 0x03, 0x75, 0x2A, 0x33,
];
const HOOK_RECEIPT_COUNT: usize = 3;
const EXPECTED_HOOKS: [ExpectedRvaHook; HOOK_RECEIPT_COUNT] = [
    ExpectedRvaHook {
        logical_name: "gameplay.d57.pause_forced",
        requested_rva: PAUSE_FORCED_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &PAUSE_FORCED_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d57.pause_toggle",
        requested_rva: PAUSE_TOGGLE_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &PAUSE_TOGGLE_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d57.speed_setter",
        requested_rva: SPEED_SETTER_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &SPEED_SENTINEL,
    },
];

/// The three D57 targets in the install/teardown order used by this lane.
const HOOK_NAMES: [&str; HOOK_RECEIPT_COUNT] = [
    "gameplay.d57.speed_setter",
    "gameplay.d57.pause_toggle",
    "gameplay.d57.pause_forced",
];

fn hook_batch(addresses: &InstalledAddresses) -> [(&'static str, *mut c_void); HOOK_RECEIPT_COUNT] {
    let targets = [
        addresses.speed_setter,
        addresses.pause_toggle,
        addresses.pause_forced,
    ];
    core::array::from_fn(|index| (HOOK_NAMES[index], targets[index] as *mut c_void))
}

type SpeedSetterFn = unsafe extern "system" fn(usize, i32);
type PauseToggleFn = unsafe extern "system" fn(usize, u8, usize);
type PauseForcedFn = unsafe extern "system" fn(usize, i32);

static ORIGINAL_SPEED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PAUSE_TOGGLE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PAUSE_FORCED: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// One-way latch: a removal or rollback ran and could NOT be proved to have
/// finished.
///
/// `INSTALLED` alone is a one-sided answer and always was. FOUR paths store or
/// leave `false` in it while sites may still be created, enabled or
/// unwitnessed, and every one of them sets this latch:
///
///  1. `seal_after_final_drain_and_uninstall_inner`, receipt-owned removal
///     failing partway -- some of the three sites may still be patched;
///  2. the same function, `observe_release_receipts` failing after every target
///     came out -- the bytes are gone but nothing witnessed it;
///  3. the same function's PARTIAL-DISABLE restore path, when the restore or
///     its receipt re-observation fails -- the set is neither cleanly installed
///     nor cleanly removed;
///  4. `with_rollback_result` receiving `Err` -- the rollback failed to
///     establish a COMPLETE, WITNESSED undo. That covers two different states
///     and the latch is deliberately set for both: sites may still be created
///     or enabled (removal did not happen), or removal succeeded and the
///     EVIDENCE for it is missing -- sentinel verification or release-receipt
///     observation can fail after every site is out. `INSTALLED` is no guide to
///     which: most of these paths leave it false, while `rollback_enabled`'s
///     retryable-receipt branch deliberately leaves it TRUE because every
///     detour is still enabled and owned and no site may be removed until the
///     exact installed roster exists. This latch does not track `INSTALLED`, it
///     tracks what is still OWED -- cleanup, proof, or both.
///
/// Anything asking "may a D57 detour still be executing" has to read this as
/// well as `installed()`, which is what `release_pending` is for.
///
/// Cleared only by a removal that reaches the end with no errors, or by a fresh
/// successful install -- which is safe because unresolved module or receipt
/// custody stops that success from being reached. Never cleared by an observer.
static REMOVAL_UNPROVED: AtomicBool = AtomicBool::new(false);
static ACCEPTING_CALLBACKS: AtomicBool = AtomicBool::new(false);
static CALLBACKS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static TERMINAL_SEALED: AtomicBool = AtomicBool::new(false);
static CALLBACK_FATAL: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());
static HOOK_RECEIPTS: Mutex<HookReceiptState> = Mutex::new(HookReceiptState::EMPTY);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HookReceiptState {
    installed: [Option<InstalledRvaHookReceipt>; HOOK_RECEIPT_COUNT],
    released: [Option<ReleasedRvaHookReceipt>; HOOK_RECEIPT_COUNT],
    removed_by_custody: [bool; HOOK_RECEIPT_COUNT],
}

impl HookReceiptState {
    const EMPTY: Self = Self {
        installed: [None; HOOK_RECEIPT_COUNT],
        released: [None; HOOK_RECEIPT_COUNT],
        removed_by_custody: [false; HOOK_RECEIPT_COUNT],
    };

    fn has_installed(&self) -> bool {
        self.installed.iter().any(Option::is_some)
    }

    fn has_released(&self) -> bool {
        self.released.iter().any(Option::is_some)
    }
}

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    Speed,
    PauseToggle,
    PauseForced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeedPauseTransition {
    pub sequence: u64,
    pub kind: TransitionKind,
    pub game_hour_raw: i32,
    pub speed_raw: i32,
    pub paused: bool,
    pub tick_progress_f32_bits: u32,
    pub thread_id: u32,
}

/// Exact state immediately before the first observed mutation.  Early-launch
/// capture can legitimately see engine initialization mutate speed or pause
/// before D57 emits its first hourly frame, so an empty first frame is not a
/// valid baseline requirement.  This hook-owned observation lets the fold
/// replay sequence one from a real pre-mutation state instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeedPauseBaseline {
    pub game_hour_raw: i32,
    pub speed_raw: i32,
    pub paused: bool,
    pub tick_progress_f32_bits: u32,
    pub thread_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeedPauseObservedState {
    pub game_hour_raw: i32,
    pub speed_raw: i32,
    pub paused: bool,
    pub tick_progress_f32_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeedPauseDrain {
    pub current: SpeedPauseObservedState,
    pub baseline: Option<SpeedPauseBaseline>,
    pub transitions: Vec<SpeedPauseTransition>,
}

/// One immutable prefix of the hook-owned evidence queue. The prefix remains
/// owned by the hook until the caller confirms successful D57 publication with
/// [`commit_batch`]. A failed publication must call [`rollback_batch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeedPauseDrainReservation {
    pub reservation_id: u64,
    pub batch: SpeedPauseDrain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObservedState {
    game_hour_raw: i32,
    speed_raw: i32,
    paused: bool,
    tick_progress_f32_bits: u32,
}

#[derive(Debug, Default)]
struct HookState {
    next_sequence: u64,
    /// Set when a committed publication has exposed the current hook-owned
    /// state as the recorder's sequence-zero hourly baseline before the first
    /// mutation.  Without this latch the first later callback would emit a
    /// second sequence-zero pre-mutation baseline, which the cumulative fold
    /// must (correctly) reject as an ambiguous history restart.
    clean_baseline_drained: bool,
    pending_baseline: Option<SpeedPauseBaseline>,
    pending: VecDeque<SpeedPauseTransition>,
    next_reservation_id: u64,
    reservation: Option<EvidenceReservation>,
    callback_count: u64,
    owner_thread_id: Option<u32>,
    current_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EvidenceReservation {
    id: u64,
    kind: EvidenceReservationKind,
    baseline_reserved: bool,
    transition_count: usize,
    first_sequence: Option<u64>,
    last_sequence: Option<u64>,
    commits_clean_baseline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvidenceReservationKind {
    Delivery,
    Terminal,
}

impl HookState {
    fn fresh() -> Self {
        Self {
            pending: VecDeque::with_capacity(MAX_PENDING),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub speed_setter: usize,
    pub pause_toggle: usize,
    pub pause_forced: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub callback_count: u64,
    pub last_sequence: u64,
    pub pending_baseline: bool,
    pub pending: usize,
    pub reservation_id: Option<u64>,
    pub reserved_transitions: usize,
    pub current_error: Option<String>,
}

/// Exact enabled-byte receipts for the canonical stable-name-ordered D57
/// roster: forced pause, pause toggle, then speed setter.
pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    let receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned")?;
    match receipts.installed {
        [Some(forced), Some(toggle), Some(speed)] => Ok([forced, toggle, speed]),
        _ => Err("X12 complete installed-hook receipts are unavailable".into()),
    }
}

/// Consume the complete stable-name-ordered release receipt set. No partial
/// release array is ever exposed.
pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String>
{
    if let Some(state) = STATE.get() {
        let state = state
            .lock()
            .map_err(|_| "X12 hook state mutex poisoned before receipt handoff")?;
        if state.pending_baseline.is_some()
            || !state.pending.is_empty()
            || state.reservation.is_some()
        {
            return Err(
                "X12 released-hook receipts remain bound to uncommitted terminal evidence".into(),
            );
        }
    }
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned")?;
    let released = match receipts.released {
        [Some(forced), Some(toggle), Some(speed)] => [forced, toggle, speed],
        _ => return Err("X12 complete released-hook receipts are unavailable".into()),
    };
    receipts.released = [None; HOOK_RECEIPT_COUNT];
    receipts.removed_by_custody = [false; HOOK_RECEIPT_COUNT];
    Ok(released)
}

pub fn install() -> Result<InstalledAddresses, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "X12 hook lifecycle mutex poisoned".to_string())?;
    let base = (unsafe { GetModuleHandleW(ptr::null()) }) as usize;
    if base == 0 {
        return Err("X12 GetModuleHandleW(NULL) returned null".into());
    }
    let addresses = InstalledAddresses {
        speed_setter: base + SPEED_SETTER_RVA,
        pause_toggle: base + PAUSE_TOGGLE_RVA,
        pause_forced: base + PAUSE_FORCED_RVA,
    };
    if HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned before install")?
        .has_released()
    {
        return Err("X12 released-hook receipts must be taken before a new installation".into());
    }
    if INSTALLED.load(Ordering::Acquire) {
        let stored = observe_install_receipts(&addresses)?;
        reobserve_install_receipts(&addresses, stored)?;
        return Ok(addresses);
    }
    if MODULE_BASE.load(Ordering::Acquire) != 0 {
        return Err(
            "X12 prior hook runtime custody remains unresolved; reinstall is forbidden".into(),
        );
    }
    if HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned before install")?
        .has_installed()
    {
        return Err("X12 prior hook custody remains unresolved; reinstall is forbidden".into());
    }
    if let Some(state) = STATE.get() {
        let state = state
            .lock()
            .map_err(|_| "X12 hook state mutex poisoned before install")?;
        if state.pending_baseline.is_some()
            || !state.pending.is_empty()
            || state.reservation.is_some()
        {
            return Err(
                "X12 prior terminal evidence remains unresolved; reinstall is forbidden".into(),
            );
        }
    }
    verify_sentinel(addresses.speed_setter, &SPEED_SENTINEL, "speed setter")?;
    verify_sentinel(
        addresses.pause_toggle,
        &PAUSE_TOGGLE_SENTINEL,
        "pause toggle",
    )?;
    verify_sentinel(
        addresses.pause_forced,
        &PAUSE_FORCED_SENTINEL,
        "forced pause",
    )?;
    let state = STATE.get_or_init(|| Mutex::new(HookState::fresh()));
    *state
        .lock()
        .map_err(|_| "X12 hook state mutex poisoned before install")? = HookState::fresh();
    CALLBACK_FATAL.store(false, Ordering::Release);
    CALLBACKS_IN_FLIGHT.store(0, Ordering::Release);
    TERMINAL_SEALED.store(false, Ordering::Release);
    ACCEPTING_CALLBACKS.store(true, Ordering::Release);

    let speed = match unsafe {
        MinHook::create_hook(
            addresses.speed_setter as *mut c_void,
            hook_speed as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            ACCEPTING_CALLBACKS.store(false, Ordering::Release);
            return Err(format!("X12 speed hook creation failed: {status}"));
        }
    };
    ORIGINAL_SPEED.store(speed as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    let pause_toggle = match unsafe {
        MinHook::create_hook(
            addresses.pause_toggle as *mut c_void,
            hook_pause_toggle as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            let reason = format!("X12 pause-toggle hook creation failed: {status}");
            return Err(with_rollback_result(reason, rollback_created(&addresses)));
        }
    };
    ORIGINAL_PAUSE_TOGGLE.store(pause_toggle as usize, Ordering::Release);
    let pause_forced = match unsafe {
        MinHook::create_hook(
            addresses.pause_forced as *mut c_void,
            hook_pause_forced as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            let reason = format!("X12 forced-pause hook creation failed: {status}");
            return Err(with_rollback_result(reason, rollback_created(&addresses)));
        }
    };
    ORIGINAL_PAUSE_FORCED.store(pause_forced as usize, Ordering::Release);

    // One batch freezes the process once instead of once per target. The input
    // order is unchanged, and no game thread can run between the three patches.
    let enabled = unsafe { inline_detour::enable_batch(&hook_batch(&addresses)) };
    if let Some((name, status)) = enabled.refusal() {
        let reason = format!("X12 {name} hook enable failed: {status}");
        return Err(with_rollback_result(reason, rollback_enabled(&addresses)));
    }
    if let Err(error) = observe_install_receipts(&addresses) {
        let reason = format!("X12 enabled-hook receipt failed: {error}");
        return Err(with_rollback_result(reason, rollback_enabled(&addresses)));
    }
    INSTALLED.store(true, Ordering::Release);
    // A fresh install supersedes any earlier unproved removal: these are new
    // hooks with new receipts, and the debt was about the old ones.
    REMOVAL_UNPROVED.store(false, Ordering::Release);
    Ok(addresses)
}

pub fn uninstall() -> Result<(), String> {
    seal_after_final_drain_and_uninstall_inner(None, false)
}

/// Discards every retained D57 mutation and removes the three detours, for the
/// single case where the production hook registry could not finish installing
/// its canonical roster.
///
/// This is the abort half of the transaction whose commit half is
/// [`commit_terminal_batch_and_uninstall`]. A partially installed roster can
/// never assemble the terminal DATA frame that commit publishes into, so the
/// sealed prefix has nowhere to go and is discarded with an exact count. The
/// physical half is unchanged: admission close, disable, quiescence wait,
/// receipt-owned removal and release-receipt observation all still run, and any
/// of them failing still fails this abort.
pub fn abort_partial_install_and_uninstall() -> Result<InstallAbortDiscard, String> {
    seal_after_final_drain_and_uninstall_inner(None, true)?;
    Ok(take_install_abort_discard())
}

/// Running total of what install aborts have discarded from this lane.
///
/// Producer custody has to be cleared before the receipt-owned hook removal
/// below, or that removal's own custody guard refuses. That leaves a window: if
/// removal or release-receipt observation then fails, the evidence is already
/// gone while the abort returns `Err` and the caller records nothing. The count
/// is therefore accumulated here rather than returned straight out, so the
/// retry that finds an already-empty lane still reports the exact number of
/// observations the first attempt dropped.
static INSTALL_ABORT_DISCARD: Mutex<InstallAbortDiscard> = Mutex::new(InstallAbortDiscard::ZERO);

fn accumulate_install_abort_discard(observed: InstallAbortDiscard) {
    INSTALL_ABORT_DISCARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .merge(observed);
}

/// Takes the running total. Called only after a complete physical release, so
/// a lane that comes back fresh does not inherit an earlier abort's count.
fn take_install_abort_discard() -> InstallAbortDiscard {
    mem::replace(
        &mut *INSTALL_ABORT_DISCARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        InstallAbortDiscard::ZERO,
    )
}

/// Close the callback admission gate, remove all three detours, and prove that
/// no mutation escaped the collector's final committed D57 publication.
///
/// The old drain-then-uninstall sequence had a real race: a callback could
/// publish after the final commit but before MinHook disabled the target.  This
/// operation closes admission first, disables the targets, waits for callbacks
/// that entered before the close to finish, and only then examines the queue
/// and sticky error state.  Any such late mutation is a terminal capture error.
pub fn seal_after_final_drain_and_uninstall() -> Result<(), String> {
    seal_after_final_drain_and_uninstall_inner(None, false)
}

fn seal_after_final_drain_and_uninstall_inner(
    retained_terminal_reservation_id: Option<u64>,
    discard_custody: bool,
) -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "X12 hook lifecycle mutex poisoned".to_string())?;
    seal_after_final_drain_and_uninstall_locked(retained_terminal_reservation_id, discard_custody)
}

/// Records and clears every retained D57 evidence custody so the teardown
/// guard below does not refuse an aborted partial install forever.
///
/// Callers must already have closed admission and waited for callback
/// quiescence; otherwise a live callback can append behind the clear.
fn discard_install_abort_custody_locked(state: &mut HookState, discard: &mut InstallAbortDiscard) {
    discard.retained_rows = state.pending.len() as u64;
    discard.reserved_rows = state
        .reservation
        .map_or(0, |reservation| reservation.transition_count as u64);
    discard.discarded_diagnostics = u64::from(state.pending_baseline.is_some());
    discard.poisoned |= state.current_error.is_some();
    state.pending.clear();
    state.pending_baseline = None;
    state.reservation = None;
    state.current_error = None;
}

fn seal_after_final_drain_and_uninstall_locked(
    retained_terminal_reservation_id: Option<u64>,
    discard_custody: bool,
) -> Result<(), String> {
    let was_installed = INSTALLED.swap(false, Ordering::AcqRel);
    let base = MODULE_BASE.load(Ordering::Acquire);
    if !was_installed && base == 0 {
        return Ok(());
    }
    ACCEPTING_CALLBACKS.store(false, Ordering::Release);
    let addresses = InstalledAddresses {
        speed_setter: base + SPEED_SETTER_RVA,
        pause_toggle: base + PAUSE_TOGGLE_RVA,
        pause_forced: base + PAUSE_FORCED_RVA,
    };
    let mut errors = Vec::new();
    let installed_receipts =
        match installed_hook_receipts().or_else(|_| observe_install_receipts(&addresses)) {
            Ok(receipts) => Some(receipts),
            Err(error) => {
                let release_retry_in_progress = HOOK_RECEIPTS
                    .lock()
                    .map(|receipts| receipts.has_released())
                    .unwrap_or(false);
                if !release_retry_in_progress {
                    errors.push(error);
                    // The enabled set is still physically intact. Retain it and
                    // its partial opaque receipt custody until the exact missing
                    // observations can be completed on a later teardown retry.
                    INSTALLED.store(true, Ordering::Release);
                    ACCEPTING_CALLBACKS.store(true, Ordering::Release);
                    return Err(errors.join("; "));
                }
                None
            }
        };
    let targets = [
        (addresses.speed_setter, "speed"),
        (addresses.pause_toggle, "pause-toggle"),
        (addresses.pause_forced, "forced-pause"),
    ];
    let mut disabled = [false; 3];
    for (index, ((target, name), receipt_index)) in
        targets.iter().copied().zip([2usize, 1, 0]).enumerate()
    {
        match disable_hook_owned_by_receipt_custody(receipt_index, target, name) {
            Ok(()) => disabled[index] = true,
            Err(error) => errors.push(error),
        }
    }

    if !disabled.iter().all(|value| *value) {
        let mut fully_reenabled = true;
        let restore: Vec<_> = hook_batch(&addresses)
            .into_iter()
            .enumerate()
            .filter(|(index, _)| disabled[*index])
            .map(|(_, target)| target)
            .collect();
        // Restore every successfully disabled site with one freeze rather than
        // one per site, preserving the original speed/toggle/forced ordering.
        // GATED ON THE WHOLE-BATCH VERDICT, not on whether any string came
        // back. A restore batch that refuses rolls itself back, so no site is
        // re-enabled and every one of them is named.
        let restored = unsafe { inline_detour::enable_batch(&restore) };
        if !restored.all_succeeded() {
            fully_reenabled = false;
            for (name, status) in restored.failures() {
                errors.push(format!("restore {name} after partial disable: {status}"));
            }
        }
        if fully_reenabled {
            match installed_receipts {
                Some(receipts) => {
                    if let Err(error) = reobserve_install_receipts(&addresses, receipts) {
                        fully_reenabled = false;
                        errors.push(format!(
                            "X12 restored hook set failed live receipt observation: {error}"
                        ));
                    }
                }
                None => fully_reenabled = false,
            }
        }
        MODULE_BASE.store(base, Ordering::Release);
        if fully_reenabled {
            INSTALLED.store(true, Ordering::Release);
            ACCEPTING_CALLBACKS.store(true, Ordering::Release);
        } else {
            // NOT fully restored, and `INSTALLED` is being left FALSE over a set
            // that is neither cleanly installed nor cleanly removed: some sites
            // are disabled, some are patched, and if the receipt re-observation
            // is what failed then nothing has witnessed which is which. An
            // observer reading `installed()` alone would call that absent.
            REMOVAL_UNPROVED.store(true, Ordering::Release);
        }
        return Err(errors.join("; "));
    }

    if let Err(error) = wait_for_callback_quiescence("teardown") {
        errors.push(error);
        // All hooks remain created (but disabled), all trampolines, receipts,
        // pending evidence, and module custody remain intact for a safe retry.
        INSTALLED.store(true, Ordering::Release);
        return Err(errors.join("; "));
    }
    if CALLBACK_FATAL.load(Ordering::Acquire) {
        if discard_custody {
            // An install abort has no capture to poison: bank the latch and
            // clear it so the aborted roster leaves no sticky state behind.
            accumulate_install_abort_discard(InstallAbortDiscard {
                poisoned: true,
                ..InstallAbortDiscard::ZERO
            });
            CALLBACK_FATAL.store(false, Ordering::Release);
        } else {
            errors.push("X12 callback admission/state failed during final teardown".into());
        }
    }
    let mut pending_custody = false;
    match STATE.get() {
        Some(state) => match state.lock() {
            Ok(mut state) => {
                if discard_custody {
                    let mut observed = InstallAbortDiscard::ZERO;
                    discard_install_abort_custody_locked(&mut state, &mut observed);
                    accumulate_install_abort_discard(observed);
                }
                if let Some(error) = &state.current_error {
                    errors.push(format!("X12 final hook state failed: {error}"));
                }
                let retained_terminal_reservation =
                    retained_terminal_reservation_id.and_then(|reservation_id| {
                        state.reservation.filter(|reservation| {
                            reservation.id == reservation_id
                                && reservation.kind == EvidenceReservationKind::Terminal
                        })
                    });
                if let Some(reservation) = retained_terminal_reservation {
                    if let Err(error) = validate_reserved_evidence(&state, reservation) {
                        errors.push(format!(
                            "X12 retained terminal evidence changed before hook release: {error}"
                        ));
                        pending_custody = true;
                    }
                    if reservation.transition_count != state.pending.len()
                        || reservation.baseline_reserved != state.pending_baseline.is_some()
                    {
                        errors.push(
                            "X12 retained terminal reservation does not cover the complete sealed evidence"
                                .into(),
                        );
                        pending_custody = true;
                    }
                } else if state.pending_baseline.is_some()
                    || !state.pending.is_empty()
                    || state.reservation.is_some()
                {
                    errors.push(format!(
                    "X12 baseline/transition evidence remained after the final D57 publication (baseline={}, transitions={}, reservation={:?})",
                    state.pending_baseline.is_some(),
                    state.pending.len(),
                    state.reservation.map(|reservation| reservation.id),
                ));
                    pending_custody = true;
                }
            }
            Err(_) => {
                errors.push("X12 final hook state mutex unavailable".into());
                pending_custody = true;
            }
        },
        None => {
            errors.push("X12 final hook state is unavailable".into());
            pending_custody = true;
        }
    }
    if pending_custody {
        // Evidence remains available to the explicit reservation protocol; a
        // retry may publish/commit or roll it back before teardown continues.
        INSTALLED.store(true, Ordering::Release);
        return Err(errors.join("; "));
    }

    let mut removal_failed = false;
    for ((target, name), receipt_index) in targets.iter().copied().zip([2usize, 1, 0]) {
        if let Err(error) = remove_hook_owned_by_receipt_custody(receipt_index, target, name) {
            removal_failed = true;
            errors.push(error);
        }
    }
    if removal_failed {
        // The complete receipt/trampoline/module custody remains available for
        // another teardown attempt. Never advertise a partially removed set as
        // a live installation.
        INSTALLED.store(false, Ordering::Release);
        // ... but do not let that read as "provably absent" either. Some of the
        // three sites may still be patched, and `installed()` is about to say
        // false over them.
        REMOVAL_UNPROVED.store(true, Ordering::Release);
        return Err(errors.join("; "));
    }

    match observe_release_receipts() {
        Ok(_) => {}
        Err(error) => {
            errors.push(format!("X12 hook release receipt failed: {error}"));
            INSTALLED.store(false, Ordering::Release);
            // The bytes are out -- every target above removed cleanly -- but
            // nothing WITNESSED it. An unwitnessed removal is exactly the state
            // this latch exists to keep visible.
            REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(errors.join("; "));
        }
    }

    ORIGINAL_SPEED.store(0, Ordering::Release);
    ORIGINAL_PAUSE_TOGGLE.store(0, Ordering::Release);
    ORIGINAL_PAUSE_FORCED.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    TERMINAL_SEALED.store(false, Ordering::Release);
    if errors.is_empty() {
        // Removed, and witnessed. This is the ONLY place the debt is cleared by
        // a removal, and it is reached only when every target came out and
        // `observe_release_receipts` succeeded.
        REMOVAL_UNPROVED.store(false, Ordering::Release);
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Whether a removal is still owed a proof.
///
/// The companion to [`installed`], and the reason a caller must read BOTH. This
/// lane clears `INSTALLED` on the way out of a removal that failed partway and
/// on the way out of one whose release receipts could not be observed, so
/// `installed()` answers false in two states where a detour may still be
/// executing or where nothing witnessed it leaving. Same primitive D45 and D78
/// carry; this lane did without one until 2026-08-13, which is why it was the
/// one receipt group that could not answer
/// `GROUPS_THAT_ANSWER_UNPROVED_CLEANUP` truthfully.
pub fn release_pending() -> bool {
    REMOVAL_UNPROVED.load(Ordering::Acquire)
}

pub fn activation_enabled() -> bool {
    PRODUCTION_ACTIVATION || std::env::var(EVIDENCE_ACTIVATION_ENV).ok().as_deref() == Some("1")
}

/// Backward-compatible name used by diagnostic harnesses. This now reports the
/// unified production-or-diagnostics activation state.
pub fn evidence_activation_enabled() -> bool {
    activation_enabled()
}

/// Atomically reserve the first pre-mutation baseline and the current
/// transition prefix without removing either from hook-owned storage.
///
/// Only one reservation may be live. Callbacks may continue appending after
/// the reserved prefix while the caller builds and publishes its D57 frame.
pub fn peek_batch() -> Result<SpeedPauseDrainReservation, String> {
    peek_batch_with_kind(EvidenceReservationKind::Delivery)
}

fn peek_batch_with_kind(
    reservation_kind: EvidenceReservationKind,
) -> Result<SpeedPauseDrainReservation, String> {
    if !installed() {
        return Err("X12 speed/pause hooks are not installed".into());
    }
    if reservation_kind == EvidenceReservationKind::Delivery
        && TERMINAL_SEALED.load(Ordering::Acquire)
    {
        return Err("X12 ordinary delivery reservation refused terminal sealed custody".into());
    }
    if CALLBACK_FATAL.load(Ordering::Acquire) {
        return Err("X12 callback admission/state failed".into());
    }
    let state_mutex = STATE.get().ok_or("X12 hook state is not initialized")?;
    for _ in 0..1024 {
        if CALLBACKS_IN_FLIGHT.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
            continue;
        }
        let mut state = state_mutex
            .lock()
            .map_err(|_| "X12 hook state mutex poisoned")?;
        if CALLBACKS_IN_FLIGHT.load(Ordering::Acquire) != 0 {
            drop(state);
            std::thread::yield_now();
            continue;
        }
        if let Some(error) = &state.current_error {
            return Err(error.clone());
        }
        if let Some(reservation) = state.reservation {
            return Err(format!(
                "X12 evidence reservation {} is still pending",
                reservation.id
            ));
        }
        let base = MODULE_BASE.load(Ordering::Acquire);
        let cgame_state = read_usize(base + CGAMESTATE_SINGLETON_RVA)
            .ok_or("X12 CGameState singleton unreadable during atomic reservation")?;
        let current = observe(cgame_state)?;
        if CALLBACKS_IN_FLIGHT.load(Ordering::Acquire) != 0 {
            drop(state);
            std::thread::yield_now();
            continue;
        }
        let (reservation_id, baseline, transitions) =
            reserve_pending_evidence(&mut state, reservation_kind)?;
        return Ok(SpeedPauseDrainReservation {
            reservation_id,
            batch: SpeedPauseDrain {
                current: SpeedPauseObservedState {
                    game_hour_raw: current.game_hour_raw,
                    speed_raw: current.speed_raw,
                    paused: current.paused,
                    tick_progress_f32_bits: current.tick_progress_f32_bits,
                },
                baseline,
                transitions,
            },
        });
    }
    Err("X12 callbacks did not quiesce for an atomic D57 reservation".into())
}

fn reserve_pending_evidence(
    state: &mut HookState,
    kind: EvidenceReservationKind,
) -> Result<(u64, Option<SpeedPauseBaseline>, Vec<SpeedPauseTransition>), String> {
    if let Some(reservation) = state.reservation {
        return Err(format!(
            "X12 evidence reservation {} is still pending",
            reservation.id
        ));
    }
    let baseline = state.pending_baseline;
    let transitions: Vec<_> = state.pending.iter().copied().collect();
    let reservation_id = state
        .next_reservation_id
        .checked_add(1)
        .ok_or("X12 evidence reservation sequence overflow")?;
    state.next_reservation_id = reservation_id;
    state.reservation = Some(EvidenceReservation {
        id: reservation_id,
        kind,
        baseline_reserved: baseline.is_some(),
        transition_count: transitions.len(),
        first_sequence: transitions.first().map(|row| row.sequence),
        last_sequence: transitions.last().map(|row| row.sequence),
        commits_clean_baseline: state.next_sequence == 0
            && baseline.is_none()
            && transitions.is_empty(),
    });
    Ok((reservation_id, baseline, transitions))
}

/// Commit one successfully published reservation. Only the exact reserved
/// prefix is removed; observations appended after the peek remain queued.
pub fn commit_batch(reservation_id: u64) -> Result<(), String> {
    update_reservation(reservation_id, EvidenceReservationKind::Delivery, true)
}

/// Release one failed-publication reservation without consuming evidence.
pub fn rollback_batch(reservation_id: u64) -> Result<(), String> {
    update_reservation(reservation_id, EvidenceReservationKind::Delivery, false)
}

/// Close callback admission, disable all three D57 mutation producers, wait
/// for in-flight callbacks, and reserve the complete terminal evidence batch.
/// The hook set remains disabled but installed until sink acknowledgement.
pub fn seal_and_reserve_terminal_batch() -> Result<SpeedPauseDrainReservation, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "X12 hook lifecycle mutex poisoned".to_string())?;
    if !installed() {
        return Err("X12 speed/pause hooks are not installed".into());
    }
    if !TERMINAL_SEALED.load(Ordering::Acquire) {
        let base = MODULE_BASE.load(Ordering::Acquire);
        let addresses = InstalledAddresses {
            speed_setter: base + SPEED_SETTER_RVA,
            pause_toggle: base + PAUSE_TOGGLE_RVA,
            pause_forced: base + PAUSE_FORCED_RVA,
        };
        let complete_receipts = observe_install_receipts(&addresses)?;
        reobserve_install_receipts(&addresses, complete_receipts)?;
        ACCEPTING_CALLBACKS.store(false, Ordering::Release);
        let targets = hook_batch(&addresses);
        let mut disabled = [false; 3];
        let mut failures = Vec::new();
        // The seal now costs one freeze for all three targets. Per-target
        // outcomes retain the old order, while the freeze prevents interleaving.
        let sealed = unsafe { inline_detour::disable_batch(&targets) };
        for (index, (name, outcome)) in sealed.outcomes().iter().enumerate() {
            match outcome {
                Ok(()) => disabled[index] = true,
                Err(status) => failures.push(format!("disable {name}: {status}")),
            }
        }
        // AND THE BATCH'S OWN REFUSAL, which no per-target outcome carries.
        //
        // `disable_batch` reports MinHook's already-disabled as SUCCESS --
        // nothing is patched, which is the postcondition a release asks for --
        // and that is right everywhere except here. When the batch's own
        // `MH_ApplyQueued` refuses, one target's bytes may have been restored
        // with the protection restore or the instruction-cache flush left
        // unpublished -- MinHook names neither the target it stopped at nor
        // whether it got that far -- and the completion loop then sees
        // `MH_ERROR_DISABLED` for that target and folds it into `Ok`. THIS
        // path goes on to wait for callback quiescence and seal, which is the
        // claim that no callback can enter a detour any more; an unpublished
        // restore is precisely the state in which a thread still executing the
        // stale instruction stream can.
        //
        // Fail closed and name the cause. The restore below then re-enables
        // every target this attempt took down, and for an unpublished one that
        // is a completed `EnableHookLL` -- which clears the latch, so the
        // recovery is real and not just a report.
        if let Some(refusal) = sealed.apply_refusal() {
            failures.push(format!("X12 seal batch apply refused: {refusal}"));
        }
        if !failures.is_empty() {
            let restore: Vec<_> = targets
                .into_iter()
                .enumerate()
                .filter(|(index, _)| disabled[*index])
                .map(|(_, target)| target)
                .collect();
            // ADMISSION REOPENS ONLY OVER A ROSTER THAT IS WHOLLY BACK, and
            // the coverage half is load-bearing on its own. The restore set is
            // the sites this attempt took DOWN; a seal that refused before it
            // took any of them down leaves that set EMPTY, and an empty batch
            // trivially all-succeeds. Reading only the verdict therefore
            // reopened callbacks over a roster with a site still disabled --
            // which is the recording this lane exists to refuse: producers
            // admitted while one of them is dark writes an incomplete hour as
            // a complete one.
            let restore_report = unsafe { inline_detour::enable_batch(&restore) };
            let mut restored = restore.len() == targets.len() && restore_report.all_succeeded();
            if !restore_report.all_succeeded() {
                restored = false;
                for (name, status) in restore_report.failures() {
                    failures.push(format!("restore {name}: {status}"));
                }
            }
            if restore.len() != targets.len() {
                failures.push(format!(
                    "X12 restore covered {} of {} sites, so callbacks stay closed",
                    restore.len(),
                    targets.len()
                ));
            }
            if restored {
                ACCEPTING_CALLBACKS.store(true, Ordering::Release);
            }
            return Err(failures.join("; "));
        }
        wait_for_callback_quiescence("terminal reservation")?;
        TERMINAL_SEALED.store(true, Ordering::Release);
    }
    peek_batch_with_kind(EvidenceReservationKind::Terminal)
}

/// Release the disabled hook set and publish exact release receipts while the
/// complete terminal prefix remains reserved, then consume that prefix. If
/// physical teardown or receipt materialization fails, the exact evidence
/// remains retryable and cannot be lost before the sink-accepted commit.
pub fn commit_terminal_batch_and_uninstall(reservation_id: u64) -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "X12 hook lifecycle mutex poisoned".to_string())?;
    seal_after_final_drain_and_uninstall_locked(Some(reservation_id), false)?;
    update_reservation(reservation_id, EvidenceReservationKind::Terminal, true)
}

/// Release a failed terminal publication while keeping callbacks sealed. The
/// same complete batch remains available for a later terminal reservation.
pub fn rollback_terminal_batch(reservation_id: u64) -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "X12 hook lifecycle mutex poisoned".to_string())?;
    let receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned during terminal rollback")?;
    let physical_teardown_started = !INSTALLED.load(Ordering::Acquire)
        || MODULE_BASE.load(Ordering::Acquire) == 0
        || receipts.removed_by_custody.iter().any(|removed| *removed)
        || receipts.released.iter().any(Option::is_some);
    drop(receipts);
    if physical_teardown_started {
        return Err(
            "X12 terminal rollback is forbidden after physical teardown starts; retry the exact reservation"
                .into(),
        );
    }
    update_reservation(reservation_id, EvidenceReservationKind::Terminal, false)
}

fn update_reservation(
    reservation_id: u64,
    expected_kind: EvidenceReservationKind,
    commit: bool,
) -> Result<(), String> {
    let state_mutex = STATE.get().ok_or("X12 hook state is not initialized")?;
    let mut state = state_mutex
        .lock()
        .map_err(|_| "X12 hook state mutex poisoned")?;
    let reservation = state
        .reservation
        .ok_or("X12 has no pending evidence reservation")?;
    if reservation.id != reservation_id {
        return Err(format!(
            "X12 evidence reservation mismatch (expected {}, received {reservation_id})",
            reservation.id
        ));
    }
    if reservation.kind != expected_kind {
        return Err(format!(
            "X12 {:?} reservation cannot be consumed through the {:?} path",
            reservation.kind, expected_kind
        ));
    }
    if !commit {
        state.reservation = None;
        return Ok(());
    }
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    commit_pending_evidence(&mut state, reservation)
}

fn commit_pending_evidence(
    state: &mut HookState,
    reservation: EvidenceReservation,
) -> Result<(), String> {
    validate_reserved_evidence(state, reservation)?;
    if reservation.baseline_reserved {
        state.pending_baseline = None;
    }
    state.pending.drain(..reservation.transition_count);
    if reservation.commits_clean_baseline {
        // A callback may have appended sequence one after this empty prefix was
        // reserved. The published current-state anchor is then the authoritative
        // sequence-zero baseline, so discard only that concurrently synthesized
        // pre-mutation baseline while retaining its transition.
        state.clean_baseline_drained = true;
        state.pending_baseline = None;
    }
    state.reservation = None;
    Ok(())
}

fn validate_reserved_evidence(
    state: &HookState,
    reservation: EvidenceReservation,
) -> Result<(), String> {
    if (reservation.baseline_reserved && state.pending_baseline.is_none())
        || (!reservation.baseline_reserved
            && state.pending_baseline.is_some()
            && !reservation.commits_clean_baseline)
    {
        return Err("X12 reserved baseline prefix changed before commit".into());
    }
    if state.pending.len() < reservation.transition_count {
        return Err("X12 reserved transition prefix shrank before commit".into());
    }
    let observed_first = (reservation.transition_count != 0)
        .then(|| state.pending.front().map(|row| row.sequence))
        .flatten();
    let observed_last = reservation
        .transition_count
        .checked_sub(1)
        .and_then(|index| state.pending.get(index))
        .map(|row| row.sequence);
    if observed_first != reservation.first_sequence || observed_last != reservation.last_sequence {
        return Err("X12 reserved transition prefix changed before commit".into());
    }
    Ok(())
}

/// Compatibility helper for diagnostic callers that have no downstream
/// publication boundary. Production D57 publication must use the explicit
/// peek/commit/rollback protocol.
pub fn drain_with_baseline() -> Result<SpeedPauseDrain, String> {
    let reservation = peek_batch()?;
    let reservation_id = reservation.reservation_id;
    let batch = reservation.batch;
    commit_batch(reservation_id)?;
    Ok(batch)
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics::default();
    };
    HookDiagnostics {
        callback_count: state.callback_count,
        last_sequence: state.next_sequence,
        pending_baseline: state.pending_baseline.is_some(),
        pending: state.pending.len(),
        reservation_id: state.reservation.map(|reservation| reservation.id),
        reserved_transitions: state
            .reservation
            .map(|reservation| reservation.transition_count)
            .unwrap_or(0),
        current_error: state.current_error.clone(),
    }
}

unsafe extern "system" fn hook_speed(cgame_state: usize, requested: i32) {
    let _callback = CallbackFlight::enter();
    let accepting = ACCEPTING_CALLBACKS.load(Ordering::Acquire);
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    if nested {
        // A nested hooked mutation could otherwise be collapsed into the
        // outer callback's before/after pair. Reject the capture instead of
        // claiming an exact transition order that was not retained.
        CALLBACK_FATAL.store(true, Ordering::Release);
    }
    let before = (!nested).then(|| observe(cgame_state)).transpose();
    let original = ORIGINAL_SPEED.load(Ordering::Acquire);
    if original != 0 {
        let original: SpeedSetterFn = unsafe { mem::transmute(original) };
        unsafe { original(cgame_state, requested) };
    }
    if !accepting {
        CALLBACK_FATAL.store(true, Ordering::Release);
    } else if !nested {
        publish_change(TransitionKind::Speed, before, observe(cgame_state));
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_pause_toggle(idler: usize, mode: u8, text: usize) {
    let _callback = CallbackFlight::enter();
    let accepting = ACCEPTING_CALLBACKS.load(Ordering::Acquire);
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    if nested {
        CALLBACK_FATAL.store(true, Ordering::Release);
    }
    let before = (!nested).then(|| observe_from_idler(idler)).transpose();
    let original = ORIGINAL_PAUSE_TOGGLE.load(Ordering::Acquire);
    if original != 0 {
        let original: PauseToggleFn = unsafe { mem::transmute(original) };
        unsafe { original(idler, mode, text) };
    }
    if !accepting {
        CALLBACK_FATAL.store(true, Ordering::Release);
    } else if !nested {
        publish_change(
            TransitionKind::PauseToggle,
            before,
            observe_from_idler(idler),
        );
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

unsafe extern "system" fn hook_pause_forced(idler: usize, reason: i32) {
    let _callback = CallbackFlight::enter();
    let accepting = ACCEPTING_CALLBACKS.load(Ordering::Acquire);
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    if nested {
        CALLBACK_FATAL.store(true, Ordering::Release);
    }
    let before = (!nested).then(|| observe_from_idler(idler)).transpose();
    let original = ORIGINAL_PAUSE_FORCED.load(Ordering::Acquire);
    if original != 0 {
        let original: PauseForcedFn = unsafe { mem::transmute(original) };
        unsafe { original(idler, reason) };
    }
    if !accepting {
        CALLBACK_FATAL.store(true, Ordering::Release);
    } else if !nested {
        publish_change(
            TransitionKind::PauseForced,
            before,
            observe_from_idler(idler),
        );
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

fn publish_change(
    kind: TransitionKind,
    before: Result<Option<ObservedState>, String>,
    after: Result<ObservedState, String>,
) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        CALLBACK_FATAL.store(true, Ordering::Release);
        return;
    };
    state.callback_count = state.callback_count.saturating_add(1);
    let before = match before {
        Ok(before) => before,
        Err(error) => {
            state.current_error = Some(error);
            return;
        }
    };
    let after = match after {
        Ok(after) => after,
        Err(error) => {
            state.current_error = Some(error);
            return;
        }
    };
    let Some(before) = before else {
        state.current_error =
            Some("X12 outer callback omitted its pre-mutation observation".into());
        return;
    };
    let thread_id = unsafe { GetCurrentThreadId() };
    match state.owner_thread_id {
        Some(owner) if owner != thread_id => {
            state.current_error = Some(format!(
                "X12 transition callbacks crossed threads ({owner} then {thread_id}); total ordering is unproved"
            ));
            return;
        }
        None => state.owner_thread_id = Some(thread_id),
        Some(_) => {}
    }
    if (kind == TransitionKind::Speed && before.paused != after.paused)
        || (kind != TransitionKind::Speed && before.speed_raw != after.speed_raw)
    {
        state.current_error = Some(
            "X12 callback changed both speed and pause; one-row attribution would lose ordering"
                .into(),
        );
        return;
    }
    let changed = match kind {
        TransitionKind::Speed => before.speed_raw != after.speed_raw,
        TransitionKind::PauseToggle | TransitionKind::PauseForced => before.paused != after.paused,
    };
    if !changed {
        return;
    }
    if state.pending.len() >= MAX_PENDING {
        state.current_error = Some("X12 transition queue overflow; ordering is incomplete".into());
        return;
    }
    let Some(sequence) = state.next_sequence.checked_add(1) else {
        state.current_error = Some("X12 transition sequence overflow".into());
        return;
    };
    if state.next_sequence == 0 && !state.clean_baseline_drained {
        state.pending_baseline = Some(SpeedPauseBaseline {
            game_hour_raw: before.game_hour_raw,
            speed_raw: before.speed_raw,
            paused: before.paused,
            tick_progress_f32_bits: before.tick_progress_f32_bits,
            thread_id,
        });
    }
    state.next_sequence = sequence;
    state.pending.push_back(SpeedPauseTransition {
        sequence,
        kind,
        game_hour_raw: after.game_hour_raw,
        speed_raw: after.speed_raw,
        paused: after.paused,
        tick_progress_f32_bits: after.tick_progress_f32_bits,
        thread_id,
    });
}

fn observe_from_idler(idler: usize) -> Result<ObservedState, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let base = if base == 0 {
        (unsafe { GetModuleHandleW(ptr::null()) }) as usize
    } else {
        base
    };
    let clock_a = read_usize(base + CLOCK_SINGLETON_A_RVA).ok_or("X12 clock A unreadable")?;
    let clock_b = read_usize(base + CLOCK_SINGLETON_B_RVA).ok_or("X12 clock B unreadable")?;
    if idler == 0 || clock_a != idler || clock_b != idler {
        return Err("X12 pause callback owner differs from both clock singleton slots".into());
    }
    observe(
        read_usize(base + CGAMESTATE_SINGLETON_RVA).ok_or("X12 CGameState singleton unreadable")?,
    )
}

fn observe(cgame_state: usize) -> Result<ObservedState, String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let base = if base == 0 {
        (unsafe { GetModuleHandleW(ptr::null()) }) as usize
    } else {
        base
    };
    if cgame_state == 0 || read_usize(base + CGAMESTATE_SINGLETON_RVA) != Some(cgame_state) {
        return Err("X12 speed callback owner differs from CGameState singleton".into());
    }
    let clock_a = read_usize(base + CLOCK_SINGLETON_A_RVA).ok_or("X12 clock A unreadable")?;
    let clock_b = read_usize(base + CLOCK_SINGLETON_B_RVA).ok_or("X12 clock B unreadable")?;
    if clock_a == 0 || clock_a != clock_b || read_usize(clock_a) != Some(base + CLOCK_VTABLE_RVA) {
        return Err("X12 clock singleton/vtable identity mismatch".into());
    }
    let speed_raw = read_i32(cgame_state + SPEED_INDEX).ok_or("X12 speed unreadable")?;
    let paused_raw = read_u8(clock_a + PAUSED).ok_or("X12 pause unreadable")?;
    let tick_progress_f32_bits =
        read_u32(cgame_state + TICK_PROGRESS).ok_or("X12 tick progress unreadable")?;
    let tick = f32::from_bits(tick_progress_f32_bits);
    if !(0..=4).contains(&speed_raw)
        || paused_raw > 1
        || !tick.is_finite()
        || !(0.0..=1.0).contains(&tick)
        || tick_progress_f32_bits == 0x8000_0000
    {
        return Err("X12 post-mutation state failed range/finite validation".into());
    }
    Ok(ObservedState {
        game_hour_raw: read_i32(cgame_state + CURRENT_HOUR).ok_or("X12 game hour unreadable")?,
        speed_raw,
        paused: paused_raw != 0,
        tick_progress_f32_bits,
    })
}

fn verify_sentinel(address: usize, expected: &[u8; 16], name: &str) -> Result<(), String> {
    let mut actual = [0u8; 16];
    if !read_process(address, &mut actual) || &actual != expected {
        return Err(format!("X12 {name} sentinel mismatch"));
    }
    Ok(())
}

fn observe_install_receipts(
    addresses: &InstalledAddresses,
) -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    observe_install_receipt_site(
        0,
        EXPECTED_HOOKS[0],
        addresses.pause_forced,
        hook_pause_forced as usize,
    )?;
    observe_install_receipt_site(
        1,
        EXPECTED_HOOKS[1],
        addresses.pause_toggle,
        hook_pause_toggle as usize,
    )?;
    observe_install_receipt_site(
        2,
        EXPECTED_HOOKS[2],
        addresses.speed_setter,
        hook_speed as usize,
    )?;
    installed_hook_receipts()
}

fn observe_install_receipt_site(
    index: usize,
    expected: ExpectedRvaHook,
    target: usize,
    detour: usize,
) -> Result<InstalledRvaHookReceipt, String> {
    {
        let receipts = HOOK_RECEIPTS
            .lock()
            .map_err(|_| "X12 hook receipt mutex poisoned during install observation")?;
        if receipts.released[index].is_some() {
            return Err(format!(
                "X12 hook receipt site {index} was already released"
            ));
        }
        if let Some(installed) = receipts.installed[index] {
            return Ok(installed);
        }
    }
    let observed =
        observe_enabled_minhook_rel5(expected, target, detour, next_teardown_owner_token()?)?;
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned after install observation")?;
    if receipts.released[index].is_some() {
        return Err(format!(
            "X12 hook receipt site {index} released during install observation"
        ));
    }
    match receipts.installed[index] {
        Some(existing) if existing == observed => Ok(existing),
        Some(_) => Err(format!(
            "X12 hook receipt site {index} changed during install observation"
        )),
        None => {
            receipts.installed[index] = Some(observed);
            Ok(observed)
        }
    }
}

fn reobserve_install_receipts(
    addresses: &InstalledAddresses,
    installed: [InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT],
) -> Result<(), String> {
    let forced = observe_enabled_minhook_rel5(
        EXPECTED_HOOKS[0],
        addresses.pause_forced,
        hook_pause_forced as usize,
        installed[0].teardown_owner_token,
    )?;
    let toggle = observe_enabled_minhook_rel5(
        EXPECTED_HOOKS[1],
        addresses.pause_toggle,
        hook_pause_toggle as usize,
        installed[1].teardown_owner_token,
    )?;
    let speed = observe_enabled_minhook_rel5(
        EXPECTED_HOOKS[2],
        addresses.speed_setter,
        hook_speed as usize,
        installed[2].teardown_owner_token,
    )?;
    if [forced, toggle, speed] != installed {
        return Err("X12 live installed-hook receipts changed".into());
    }
    Ok(())
}

fn observe_release_receipts() -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    if !HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned before release observation")?
        .removed_by_custody
        .iter()
        .all(|removed| *removed)
    {
        return Err("X12 release attestation lacks complete owner-performed hook removal".into());
    }
    let attestation = HookTeardownAttestation {
        callbacks_quiesced: true,
        hook_removed: true,
        relay_freed: true,
    };
    observe_release_receipt_site(0, EXPECTED_HOOKS[0], attestation)?;
    observe_release_receipt_site(1, EXPECTED_HOOKS[1], attestation)?;
    observe_release_receipt_site(2, EXPECTED_HOOKS[2], attestation)?;
    let receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned after release observation")?;
    match receipts.released {
        [Some(forced), Some(toggle), Some(speed)] => Ok([forced, toggle, speed]),
        _ => Err("X12 complete released-hook receipt set is unavailable".into()),
    }
}

fn observe_release_receipt_site(
    index: usize,
    expected: ExpectedRvaHook,
    attestation: HookTeardownAttestation,
) -> Result<ReleasedRvaHookReceipt, String> {
    let installed = {
        let receipts = HOOK_RECEIPTS
            .lock()
            .map_err(|_| "X12 hook receipt mutex poisoned during release observation")?;
        if !receipts.removed_by_custody[index] {
            return Err(format!(
                "X12 hook receipt site {index} was not removed by this custody"
            ));
        }
        if let Some(released) = receipts.released[index] {
            return Ok(released);
        }
        receipts.installed[index].ok_or_else(|| {
            format!("X12 hook receipt site {index} has no installed custody to release")
        })?
    };
    let released = observe_released(expected, installed, attestation)?;
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned after release observation")?;
    match receipts.released[index] {
        Some(existing) if existing == released => Ok(existing),
        Some(_) => Err(format!(
            "X12 hook receipt site {index} changed during release observation"
        )),
        None if receipts.installed[index] == Some(installed) => {
            receipts.released[index] = Some(released);
            receipts.installed[index] = None;
            Ok(released)
        }
        None => Err(format!(
            "X12 installed hook receipt site {index} changed before release publication"
        )),
    }
}

fn remove_hook_owned_by_receipt_custody(
    receipt_index: usize,
    target: usize,
    name: &str,
) -> Result<(), String> {
    // DRAINED-BY-CALLER: both callers reach here only past
    // `wait_for_callback_quiescence(..)` -- the teardown at the "teardown"
    // stage, the failed-install path at the "install rollback" stage -- and
    // that helper closes `ACCEPTING_CALLBACKS` and refuses unless the in-flight
    // count reads zero. This function is the free plus its custody bookkeeping;
    // the three targets are disabled and drained as one set, so a per-target
    // drain here would measure a count the siblings keep refilling.
    let already_removed = HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned before removal")?
        .removed_by_custody[receipt_index];
    match unsafe { inline_detour::remove(target as *mut c_void) } {
        Ok(()) => {
            HOOK_RECEIPTS
                .lock()
                .map_err(|_| "X12 hook receipt mutex poisoned after removal")?
                .removed_by_custody[receipt_index] = true;
            Ok(())
        }
        Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) if already_removed => Ok(()),
        Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => Err(format!(
            "remove {name}: hook was not created and this custody has no prior removal"
        )),
        Err(status) => Err(format!("remove {name}: {status}")),
    }
}

fn disable_hook_owned_by_receipt_custody(
    receipt_index: usize,
    target: usize,
    name: &str,
) -> Result<(), String> {
    let already_removed = HOOK_RECEIPTS
        .lock()
        .map_err(|_| "X12 hook receipt mutex poisoned before disable")?
        .removed_by_custody[receipt_index];
    match unsafe { inline_detour::disable(target as *mut c_void) } {
        Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED)) => Ok(()),
        Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) if already_removed => Ok(()),
        Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => Err(format!(
            "disable {name}: hook was not created and this custody has no prior removal"
        )),
        Err(status) => Err(format!("disable {name}: {status}")),
    }
}

fn wait_for_callback_quiescence(stage: &str) -> Result<(), String> {
    let wait_started = Instant::now();
    while CALLBACKS_IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if wait_started.elapsed() >= Duration::from_secs(5) {
            return Err(format!(
                "{} X12 callback(s) remained in flight during {stage}",
                CALLBACKS_IN_FLIGHT.load(Ordering::Acquire)
            ));
        }
        std::thread::yield_now();
    }
    Ok(())
}

/// Formats an install failure together with the outcome of its safe rollback --
/// and records the DEBT when that rollback failed.
///
/// Every install-time rollback in this module funnels through here, and a
/// rollback that returned `Err` failed to establish a complete, witnessed undo:
/// either detours remain, or they are out and the evidence of their removal is
/// missing (sentinel verification and release-receipt observation can both fail
/// after every site has come out). Either way something is OWED, which is what
/// `REMOVAL_UNPROVED` records, and `INSTALLED` is no guide to which -- most of
/// these paths leave it false, while `rollback_enabled`'s retryable-receipt
/// branch deliberately leaves it TRUE so no site is removed before the exact
/// installed roster exists. Setting the latch here rather than at each caller
/// is deliberate: this function is the one thing every rollback path already
/// has in common, so a new install-failure exit cannot forget it.
fn with_rollback_result(reason: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => reason,
        Err(error) => {
            REMOVAL_UNPROVED.store(true, Ordering::Release);
            format!("{reason}; safe rollback failed: {error}")
        }
    }
}

fn rollback_created(addresses: &InstalledAddresses) -> Result<(), String> {
    // NEVER-ENABLED: every caller is a `MinHook::create_hook` failure arm, and
    // `inline_detour::enable_batch` does not run until all three creates have
    // succeeded. Nothing was patched into the game on any path that reaches
    // here, so no thread can have entered a detour body and there is nothing to
    // drain. The enable-failure route has its own function, `rollback_enabled`,
    // and that one is not this one.
    ACCEPTING_CALLBACKS.store(false, Ordering::Release);
    // Removing the three created sites now holds one freeze instead of three;
    // the batch preserves the original speed/toggle/forced ordering.
    let mut errors: Vec<_> = unsafe { inline_detour::remove_batch(&hook_batch(addresses)) }
        .failures()
        .into_iter()
        .map(|(name, status)| format!("remove created hook {name}: {status}"))
        .collect();
    if errors.is_empty() {
        for (target, sentinel, name) in [
            (addresses.speed_setter, &SPEED_SENTINEL, "speed setter"),
            (
                addresses.pause_toggle,
                &PAUSE_TOGGLE_SENTINEL,
                "pause toggle",
            ),
            (
                addresses.pause_forced,
                &PAUSE_FORCED_SENTINEL,
                "forced pause",
            ),
        ] {
            if let Err(error) = verify_sentinel(target, sentinel, name) {
                errors.push(format!("rollback restoration: {error}"));
            }
        }
    }
    if errors.is_empty() {
        ORIGINAL_SPEED.store(0, Ordering::Release);
        ORIGINAL_PAUSE_TOGGLE.store(0, Ordering::Release);
        ORIGINAL_PAUSE_FORCED.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        Ok(())
    } else {
        INSTALLED.store(false, Ordering::Release);
        Err(errors.join("; "))
    }
}

fn rollback_enabled(addresses: &InstalledAddresses) -> Result<(), String> {
    let retry_partial_install = HOOK_RECEIPTS
        .lock()
        .map(|receipts| receipts.has_installed() && !receipts.installed.iter().all(Option::is_some))
        .map_err(|_| "X12 hook receipt mutex poisoned before install rollback")?;
    if retry_partial_install {
        if let Err(error) = observe_install_receipts(addresses) {
            // Every detour remains enabled and globally owned. Do not remove
            // any site until the complete exact installed roster exists,
            // otherwise no authentic release set can ever be materialized.
            INSTALLED.store(true, Ordering::Release);
            ACCEPTING_CALLBACKS.store(true, Ordering::Release);
            return Err(format!(
                "X12 partial installed-hook receipt completion remains retryable: {error}"
            ));
        }
    }
    ACCEPTING_CALLBACKS.store(false, Ordering::Release);
    let mut errors = Vec::new();
    for ((target, name), receipt_index) in [
        (addresses.speed_setter, "speed"),
        (addresses.pause_toggle, "pause-toggle"),
        (addresses.pause_forced, "forced-pause"),
    ]
    .into_iter()
    .zip([2usize, 1, 0])
    {
        if let Err(error) = disable_hook_owned_by_receipt_custody(receipt_index, target, name) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        if let Err(error) = wait_for_callback_quiescence("install rollback") {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        if CALLBACK_FATAL.load(Ordering::Acquire) {
            errors.push("X12 callback admission/state failed during install rollback".into());
        }
        match STATE.get() {
            Some(state) => match state.lock() {
                Ok(state) => {
                    if let Some(error) = &state.current_error {
                        errors.push(format!("X12 install rollback hook state failed: {error}"));
                    }
                    if state.pending_baseline.is_some()
                        || !state.pending.is_empty()
                        || state.reservation.is_some()
                    {
                        errors.push(format!(
                            "X12 install rollback retained evidence (baseline={}, transitions={}, reservation={:?})",
                            state.pending_baseline.is_some(),
                            state.pending.len(),
                            state.reservation.map(|reservation| reservation.id),
                        ));
                    }
                }
                Err(_) => {
                    errors.push("X12 hook state mutex unavailable during install rollback".into())
                }
            },
            None => errors.push("X12 hook state unavailable during install rollback".into()),
        }
    }
    if errors.is_empty() {
        for ((target, name), receipt_index) in [
            (addresses.speed_setter, "speed"),
            (addresses.pause_toggle, "pause-toggle"),
            (addresses.pause_forced, "forced-pause"),
        ]
        .into_iter()
        .zip([2usize, 1, 0])
        {
            if let Err(error) = remove_hook_owned_by_receipt_custody(receipt_index, target, name) {
                errors.push(error);
            }
        }
    }
    if errors.is_empty() {
        for (target, sentinel, name) in [
            (addresses.speed_setter, &SPEED_SENTINEL, "speed setter"),
            (
                addresses.pause_toggle,
                &PAUSE_TOGGLE_SENTINEL,
                "pause toggle",
            ),
            (
                addresses.pause_forced,
                &PAUSE_FORCED_SENTINEL,
                "forced pause",
            ),
        ] {
            if let Err(error) = verify_sentinel(target, sentinel, name) {
                errors.push(format!("rollback restoration: {error}"));
            }
        }
    }
    if errors.is_empty() {
        let has_installed_receipts = match HOOK_RECEIPTS.lock() {
            Ok(receipts) => receipts.has_installed(),
            Err(_) => {
                errors.push("X12 hook receipt mutex poisoned during install rollback".into());
                false
            }
        };
        if has_installed_receipts {
            if let Err(error) = observe_release_receipts() {
                errors.push(format!(
                    "X12 partial install receipt release failed during rollback: {error}"
                ));
            }
        }
    }
    if errors.is_empty() {
        ORIGINAL_SPEED.store(0, Ordering::Release);
        ORIGINAL_PAUSE_TOGGLE.store(0, Ordering::Release);
        ORIGINAL_PAUSE_FORCED.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        Ok(())
    } else {
        INSTALLED.store(false, Ordering::Release);
        Err(errors.join("; "))
    }
}

fn read_process(address: usize, out: &mut [u8]) -> bool {
    if address == 0 {
        return false;
    }
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
    ok != 0 && read == out.len()
}

fn read_u8(address: usize) -> Option<u8> {
    let mut b = [0; 1];
    read_process(address, &mut b).then_some(b[0])
}
fn read_u32(address: usize) -> Option<u32> {
    let mut b = [0; 4];
    read_process(address, &mut b).then_some(u32::from_le_bytes(b))
}
fn read_i32(address: usize) -> Option<i32> {
    read_u32(address).map(|v| v as i32)
}
fn read_usize(address: usize) -> Option<usize> {
    let mut b = [0; mem::size_of::<usize>()];
    read_process(address, &mut b).then_some(usize::from_le_bytes(b))
}

struct CallbackFlight;

impl CallbackFlight {
    fn enter() -> Self {
        CALLBACKS_IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackFlight {
    fn drop(&mut self) {
        CALLBACKS_IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(speed_raw: i32, paused: bool) -> ObservedState {
        ObservedState {
            game_hour_raw: 10,
            speed_raw,
            paused,
            tick_progress_f32_bits: 0x3e80_0000,
        }
    }

    fn transition(sequence: u64, speed_raw: i32) -> SpeedPauseTransition {
        SpeedPauseTransition {
            sequence,
            kind: TransitionKind::Speed,
            game_hour_raw: 10,
            speed_raw,
            paused: false,
            tick_progress_f32_bits: 0x3e80_0000,
            thread_id: 7,
        }
    }

    #[test]
    fn exact_build_passive_hook_is_production_authorized() {
        assert!(PRODUCTION_ACTIVATION);
    }

    #[test]
    fn production_receipt_roster_is_exact_and_stable_name_ordered() {
        assert_eq!(
            EXPECTED_HOOKS.map(|expected| expected.logical_name),
            [
                "gameplay.d57.pause_forced",
                "gameplay.d57.pause_toggle",
                "gameplay.d57.speed_setter",
            ]
        );
        assert_eq!(
            EXPECTED_HOOKS.map(|expected| expected.requested_rva),
            [
                PAUSE_FORCED_RVA as u64,
                PAUSE_TOGGLE_RVA as u64,
                SPEED_SETTER_RVA as u64,
            ]
        );
        assert!(EXPECTED_HOOKS.iter().all(|expected| {
            expected.target_patch_len == 5
                && expected.primitive_id == MINHOOK_NORMAL_REL5_PRIMITIVE_ID
                && expected.original_prefix.len() == 16
        }));
    }

    #[test]
    fn a_failed_physical_release_never_loses_an_earlier_install_abort_discard() {
        // Custody is cleared before the receipt-owned hook removal, so an abort
        // that clears evidence and then fails during removal must not let the
        // retry -- which finds an already-empty lane -- report zero.
        accumulate_install_abort_discard(InstallAbortDiscard {
            retained_rows: 6,
            reserved_rows: 6,
            discarded_diagnostics: 1,
            poisoned: true,
        });
        let mut retry_state = HookState::fresh();
        let mut observed = InstallAbortDiscard::ZERO;
        discard_install_abort_custody_locked(&mut retry_state, &mut observed);
        accumulate_install_abort_discard(observed);

        let total = take_install_abort_discard();

        assert_eq!(total.retained_rows, 6);
        assert_eq!(total.reserved_rows, 6);
        assert_eq!(total.discarded_diagnostics, 1);
        assert!(total.poisoned);
        assert!(take_install_abort_discard().is_empty());
    }

    #[test]
    fn install_abort_discard_clears_every_custody_the_teardown_guard_refuses() {
        let mut state = HookState::fresh();
        state.pending_baseline = Some(SpeedPauseBaseline {
            game_hour_raw: 10,
            speed_raw: 2,
            paused: false,
            tick_progress_f32_bits: 0x3e80_0000,
            thread_id: 7,
        });
        state.pending.push_back(transition(1, 3));
        state.pending.push_back(transition(2, 4));
        reserve_pending_evidence(&mut state, EvidenceReservationKind::Delivery).unwrap();
        state.current_error = Some("X12 producer poisoned".into());
        let mut discard = InstallAbortDiscard::default();

        discard_install_abort_custody_locked(&mut state, &mut discard);

        assert_eq!(discard.retained_rows, 2);
        assert_eq!(discard.reserved_rows, 2);
        assert_eq!(discard.discarded_diagnostics, 1);
        assert!(discard.poisoned);
        // Every clause of the teardown custody guard must now be false, or the
        // aborted partial install can never physically release its hooks.
        assert!(state.pending.is_empty());
        assert!(state.pending_baseline.is_none());
        assert!(state.reservation.is_none());
        assert!(state.current_error.is_none());
    }

    #[test]
    fn clean_reservation_commits_the_recorder_owned_sequence_zero_baseline() {
        let mut state = HookState::fresh();
        let (reservation_id, baseline, transitions) =
            reserve_pending_evidence(&mut state, EvidenceReservationKind::Delivery).unwrap();

        assert!(baseline.is_none());
        assert!(transitions.is_empty());
        assert!(!state.clean_baseline_drained);
        let reservation = state.reservation.unwrap();
        assert_eq!(reservation.id, reservation_id);
        commit_pending_evidence(&mut state, reservation).unwrap();
        assert!(state.clean_baseline_drained);
        assert_eq!(state.next_sequence, 0);
        assert!(state.reservation.is_none());
    }

    #[test]
    fn mutation_before_first_reservation_retains_the_hook_owned_baseline() {
        let mut state = HookState::fresh();
        state.next_sequence = 1;
        state.pending_baseline = Some(SpeedPauseBaseline {
            game_hour_raw: 10,
            speed_raw: 2,
            paused: false,
            tick_progress_f32_bits: 0x3e80_0000,
            thread_id: 7,
        });
        state.pending.push_back(transition(1, 3));

        let (_, baseline, transitions) =
            reserve_pending_evidence(&mut state, EvidenceReservationKind::Delivery).unwrap();
        assert!(baseline.is_some());
        assert_eq!(transitions.len(), 1);
        assert!(!state.clean_baseline_drained);
        assert!(state.pending_baseline.is_some());
        assert_eq!(state.pending.len(), 1);
    }

    #[test]
    fn commit_consumes_only_reserved_prefix_and_preserves_concurrent_append() {
        let mut state = HookState::fresh();
        state.next_sequence = 1;
        state.pending.push_back(transition(1, 3));
        let (reservation_id, _, _) =
            reserve_pending_evidence(&mut state, EvidenceReservationKind::Delivery).unwrap();

        state.next_sequence = 2;
        state.pending.push_back(transition(2, 4));
        let reservation = state.reservation.unwrap();
        assert_eq!(reservation.id, reservation_id);
        commit_pending_evidence(&mut state, reservation).unwrap();

        assert_eq!(
            state
                .pending
                .iter()
                .map(|row| row.sequence)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert!(state.reservation.is_none());
    }

    #[test]
    fn rollback_releases_reservation_without_consuming_evidence() {
        let mut state = HookState::fresh();
        state.next_sequence = 1;
        state.pending.push_back(transition(1, 3));
        let (reservation_id, _, _) =
            reserve_pending_evidence(&mut state, EvidenceReservationKind::Delivery).unwrap();

        assert_eq!(state.reservation.unwrap().id, reservation_id);
        state.reservation = None;
        assert_eq!(
            state
                .pending
                .iter()
                .map(|row| row.sequence)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn clean_reservation_commit_discards_only_late_synthesized_baseline() {
        let mut state = HookState::fresh();
        let (_, _, _) =
            reserve_pending_evidence(&mut state, EvidenceReservationKind::Delivery).unwrap();
        state.next_sequence = 1;
        state.pending_baseline = Some(SpeedPauseBaseline {
            game_hour_raw: 10,
            speed_raw: 2,
            paused: false,
            tick_progress_f32_bits: 0x3e80_0000,
            thread_id: 7,
        });
        state.pending.push_back(transition(1, 3));

        let reservation = state.reservation.unwrap();
        commit_pending_evidence(&mut state, reservation).unwrap();

        assert!(state.clean_baseline_drained);
        assert!(state.pending_baseline.is_none());
        assert_eq!(
            state
                .pending
                .iter()
                .map(|row| row.sequence)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn production_transition_queue_overflow_is_sticky_and_emits_no_partial_row() {
        let state = STATE.get_or_init(|| Mutex::new(HookState::fresh()));
        let mut guard = state.lock().expect("test hook state lock");
        *guard = HookState::fresh();
        let mut retained = transition(1, 2);
        retained.thread_id = unsafe { GetCurrentThreadId() };
        guard.pending = std::iter::repeat(retained).take(MAX_PENDING).collect();
        drop(guard);

        publish_change(
            TransitionKind::Speed,
            Ok(Some(observed(2, false))),
            Ok(observed(3, false)),
        );

        let guard = state.lock().expect("test hook state lock after publish");
        assert_eq!(guard.pending.len(), MAX_PENDING);
        assert_eq!(guard.next_sequence, 0);
        assert_eq!(
            guard.current_error.as_deref(),
            Some("X12 transition queue overflow; ordering is incomplete")
        );
        drop(guard);
        *state.lock().expect("reset test hook state") = HookState::fresh();
    }
}
