#![cfg(windows)]

use compact_contract::division_combat_stats as contract;
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released, ExpectedRvaHook,
    HookTeardownAttestation, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::collections::VecDeque;
use std::fmt;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{
    CapturedFrame, ObservedDivision, ObservedRound, SideClass, MAX_COMMITTED_PER_ROUND,
    MAX_QUEUED_ROUNDS,
};

pub const ROUND_LOGICAL_NAME: &str = "gameplay.d99.land_combat_round_entry";
pub const HOOK_RECEIPT_COUNT: usize = 1;

pub const ROUND_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54,
];

const EXPECTED_HOOK: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: ROUND_LOGICAL_NAME,
    requested_rva: contract::COMBAT_ROUND_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &ROUND_SENTINEL,
};

type CombatRoundFn = unsafe extern "system" fn(usize, usize);

const WRAPPER_WINDOW_LEN: usize = contract::DIVISION_DEFENCE_BASE_OFFSET + mem::size_of::<i64>();
const STAT_WINDOW_LEN: usize =
    contract::STAT_HARD_ATTACK_OFFSET - contract::STAT_DEFENSE_OFFSET + mem::size_of::<i64>();

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
const LIFECYCLE_ABSENT: usize = 0;
const LIFECYCLE_INSTALLED: usize = 1;
const LIFECYCLE_TEARDOWN_PENDING: usize = 2;
static LIFECYCLE_STATE: AtomicUsize = AtomicUsize::new(LIFECYCLE_ABSENT);
static CAPTURE_EPOCH: AtomicU64 = AtomicU64::new(0);
static CAPTURE_ARMED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static CAPTURE: OnceLock<Mutex<CaptureState>> = OnceLock::new();
static LIFECYCLE: Mutex<()> = Mutex::new(());
static HOOK_RECEIPTS: Mutex<HookReceiptState> = Mutex::new(HookReceiptState::EMPTY);

#[derive(Debug, Default)]
struct CaptureState {
    rounds: VecDeque<ObservedRound>,
    drops: DropTally,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DropTally {
    queue_full: u64,
    unrecognised_side: u64,
    unlinked_combat: u64,
    unreadable_side_state: u64,
    oversized_roster: u64,
    unresolvable_division: u64,
}

impl DropTally {
    fn add(&mut self, other: DropTally) {
        self.queue_full = self.queue_full.saturating_add(other.queue_full);
        self.unrecognised_side = self
            .unrecognised_side
            .saturating_add(other.unrecognised_side);
        self.unlinked_combat = self.unlinked_combat.saturating_add(other.unlinked_combat);
        self.unreadable_side_state = self
            .unreadable_side_state
            .saturating_add(other.unreadable_side_state);
        self.oversized_roster = self.oversized_roster.saturating_add(other.oversized_roster);
        self.unresolvable_division = self
            .unresolvable_division
            .saturating_add(other.unresolvable_division);
    }

    fn counter_mut(&mut self, drop: SideDrop) -> &mut u64 {
        match drop {
            SideDrop::UnrecognisedSide => &mut self.unrecognised_side,
            SideDrop::UnlinkedCombat => &mut self.unlinked_combat,
            SideDrop::UnreadableSideState => &mut self.unreadable_side_state,
            SideDrop::OversizedRoster => &mut self.oversized_roster,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SideDrop {
    UnrecognisedSide,
    UnlinkedCombat,
    UnreadableSideState,
    OversizedRoster,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HookReceiptState {
    installed: Option<InstalledRvaHookReceipt>,
    released: Option<ReleasedRvaHookReceipt>,
    removed_by_custody: bool,
}

impl HookReceiptState {
    const EMPTY: Self = Self {
        installed: None,
        released: None,
        removed_by_custody: false,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DivisionCombatStatsHookError {
    LifecyclePoisoned,
    ModuleUnavailable,
    RoundAddressOverflow,
    RoundReadFailed(String),
    RoundSentinelMismatch,
    HookCreationFailed(String),
    HookEnableFailed(String),
    QueuePoisoned,
    ReceiptCustodyPoisoned,
    ReceiptRefused(String),
    ReceiptChangedDuringObservation,
    ReceiptAlreadyReleased,
    InstalledReceiptUnavailable,
    ReleasedReceiptUnavailable,
    ReleaseRefused(String),
    HookDisableFailed(String),
    HookRemovalFailed(String),
}

impl fmt::Display for DivisionCombatStatsHookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LifecyclePoisoned => write!(f, "D99 combat round hook lifecycle poisoned"),
            Self::ModuleUnavailable => write!(f, "D99 combat round hook module unavailable"),
            Self::RoundAddressOverflow => write!(f, "D99 combat round address overflow"),
            Self::RoundReadFailed(detail) => write!(f, "D99 combat round read failed: {detail}"),
            Self::RoundSentinelMismatch => write!(f, "D99 combat round sentinel mismatch"),
            Self::HookCreationFailed(detail) => {
                write!(f, "D99 combat round hook creation failed: {detail}")
            }
            Self::HookEnableFailed(detail) => {
                write!(f, "D99 combat round hook enable failed: {detail}")
            }
            Self::QueuePoisoned => write!(f, "D99 combat round capture state poisoned"),
            Self::ReceiptCustodyPoisoned => {
                write!(f, "D99 combat round hook receipt custody poisoned")
            }
            Self::ReceiptRefused(detail) => write!(
                f,
                "D99 production hook receipt refused, installation rolled back: {detail}"
            ),
            Self::ReceiptChangedDuringObservation => {
                write!(
                    f,
                    "D99 combat round hook receipt changed during observation"
                )
            }
            Self::ReceiptAlreadyReleased => {
                write!(f, "D99 combat round hook receipt was already released")
            }
            Self::InstalledReceiptUnavailable => {
                write!(f, "D99 installed production hook receipt is unavailable")
            }
            Self::ReleasedReceiptUnavailable => {
                write!(f, "D99 released production hook receipt is unavailable")
            }
            Self::ReleaseRefused(detail) => {
                write!(f, "D99 combat round hook release receipt refused: {detail}")
            }
            Self::HookDisableFailed(detail) => {
                write!(f, "D99 combat round hook disable failed: {detail}")
            }
            Self::HookRemovalFailed(detail) => {
                write!(f, "D99 combat round hook removal failed: {detail}")
            }
        }
    }
}

impl std::error::Error for DivisionCombatStatsHookError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivisionCombatStatsDrainError {
    NotInstalled,
    StateUnavailable,
    QueuePoisoned,
}

impl fmt::Display for DivisionCombatStatsDrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => write!(f, "combat round hook is not installed"),
            Self::StateUnavailable => write!(f, "combat round capture state unavailable"),
            Self::QueuePoisoned => write!(f, "combat round capture state poisoned"),
        }
    }
}

impl std::error::Error for DivisionCombatStatsDrainError {}

struct CallbackLease;
impl CallbackLease {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for CallbackLease {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn installed() -> bool {
    LIFECYCLE_STATE.load(Ordering::Acquire) == LIFECYCLE_INSTALLED
}

pub fn teardown_pending() -> bool {
    LIFECYCLE_STATE.load(Ordering::Acquire) == LIFECYCLE_TEARDOWN_PENDING
}

pub fn capture_armed() -> bool {
    CAPTURE_ARMED.load(Ordering::Acquire)
}

pub fn arm_capture() {
    if LIFECYCLE_STATE.load(Ordering::Acquire) != LIFECYCLE_INSTALLED {
        return;
    }
    CAPTURE_ARMED.store(true, Ordering::Release);
}

pub fn disarm_capture() {
    seal_capture();
    if let Some(state) = CAPTURE.get() {
        if let Ok(mut state) = state.lock() {
            state.rounds.clear();
            state.drops = DropTally::default();
        }
    }
    DROPPED_LOCK_BUSY.store(0, Ordering::Release);
}

fn seal_capture() {
    CAPTURE_ARMED.store(false, Ordering::Release);
    CAPTURE_EPOCH.fetch_add(1, Ordering::AcqRel);
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    let receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| DivisionCombatStatsHookError::ReceiptCustodyPoisoned.to_string())?;
    receipts
        .installed
        .map(|installed| [installed])
        .ok_or_else(|| DivisionCombatStatsHookError::InstalledReceiptUnavailable.to_string())
}

pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String>
{
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| DivisionCombatStatsHookError::ReceiptCustodyPoisoned.to_string())?;
    let released = receipts
        .released
        .ok_or_else(|| DivisionCombatStatsHookError::ReleasedReceiptUnavailable.to_string())?;
    receipts.released = None;
    receipts.removed_by_custody = false;
    Ok([released])
}

pub fn install() -> Result<(), DivisionCombatStatsHookError> {
    let _guard = LIFECYCLE
        .lock()
        .map_err(|_| DivisionCombatStatsHookError::LifecyclePoisoned)?;
    match LIFECYCLE_STATE.load(Ordering::Acquire) {
        LIFECYCLE_INSTALLED => return Ok(()),
        LIFECYCLE_TEARDOWN_PENDING => {
            return Err(DivisionCombatStatsHookError::ReceiptRefused(
                "a previous teardown could not prove the patch was removed; \
                 uninstall must succeed before this site can be installed again"
                    .into(),
            ))
        }
        _ => {}
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err(DivisionCombatStatsHookError::ModuleUnavailable);
    }
    let target = base
        .checked_add(contract::COMBAT_ROUND_RVA as usize)
        .ok_or(DivisionCombatStatsHookError::RoundAddressOverflow)?;
    let mut actual = [0u8; ROUND_SENTINEL.len()];
    read_exact(target, &mut actual).map_err(DivisionCombatStatsHookError::RoundReadFailed)?;
    if actual != ROUND_SENTINEL {
        return Err(DivisionCombatStatsHookError::RoundSentinelMismatch);
    }
    let original =
        unsafe { MinHook::create_hook(target as *mut c_void, hook_combat_round as *mut c_void) }
            .map_err(|status| {
                DivisionCombatStatsHookError::HookCreationFailed(status.to_string())
            })?;
    ORIGINAL.store(original as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    let state = CAPTURE.get_or_init(|| Mutex::new(CaptureState::default()));
    match state.lock() {
        Ok(mut state) => {
            state.rounds.clear();
            state.drops = DropTally::default();
        }
        Err(_) => {
            rollback_failed_install(target);
            return Err(DivisionCombatStatsHookError::QueuePoisoned);
        }
    }
    DROPPED_LOCK_BUSY.store(0, Ordering::Release);
    if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
        let _ = unsafe { inline_detour::remove(target as *mut c_void) };
        ORIGINAL.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(DivisionCombatStatsHookError::HookEnableFailed(
            status.to_string(),
        ));
    }
    if let Err(error) = observe_install_receipt(target) {
        rollback_failed_install(target);
        return Err(error);
    }
    LIFECYCLE_STATE.store(LIFECYCLE_INSTALLED, Ordering::Release);
    Ok(())
}

fn rollback_failed_install(target: usize) {
    let disabled = matches!(
        unsafe { inline_detour::disable(target as *mut c_void) },
        Ok(())
            | Err(InlineDetourReleaseError::MinHook(
                MH_STATUS::MH_ERROR_DISABLED
            ))
            | Err(InlineDetourReleaseError::MinHook(
                MH_STATUS::MH_ERROR_NOT_CREATED
            ))
    );
    let removed = if disabled {
        while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
        }
        matches!(
            unsafe { inline_detour::remove(target as *mut c_void) },
            Ok(())
                | Err(InlineDetourReleaseError::MinHook(
                    MH_STATUS::MH_ERROR_NOT_CREATED
                ))
        )
    } else {
        false
    };
    apply_teardown_outcome(TeardownOutcome {
        patch_gone: disabled && removed,
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TeardownOutcome {
    patch_gone: bool,
}

fn teardown_state_transition(outcome: TeardownOutcome) -> (bool, usize) {
    if outcome.patch_gone {
        (true, LIFECYCLE_ABSENT)
    } else {
        (false, LIFECYCLE_TEARDOWN_PENDING)
    }
}

fn apply_teardown_outcome(outcome: TeardownOutcome) {
    let (clear_addresses, state) = teardown_state_transition(outcome);
    if clear_addresses {
        ORIGINAL.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
    }
    LIFECYCLE_STATE.store(state, Ordering::Release);
}

fn observe_install_receipt(
    target: usize,
) -> Result<InstalledRvaHookReceipt, DivisionCombatStatsHookError> {
    {
        let receipts = HOOK_RECEIPTS
            .lock()
            .map_err(|_| DivisionCombatStatsHookError::ReceiptCustodyPoisoned)?;
        if receipts.released.is_some() {
            return Err(DivisionCombatStatsHookError::ReceiptAlreadyReleased);
        }
        if let Some(installed) = receipts.installed {
            return Ok(installed);
        }
    }
    let token =
        next_teardown_owner_token().map_err(DivisionCombatStatsHookError::ReceiptRefused)?;
    let observed = observe_enabled_minhook_rel5(
        EXPECTED_HOOK,
        target,
        hook_combat_round as *const () as usize,
        token,
    )
    .map_err(DivisionCombatStatsHookError::ReceiptRefused)?;
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| DivisionCombatStatsHookError::ReceiptCustodyPoisoned)?;
    if receipts.released.is_some() {
        return Err(DivisionCombatStatsHookError::ReceiptAlreadyReleased);
    }
    match receipts.installed {
        Some(existing) if existing == observed => Ok(existing),
        Some(_) => Err(DivisionCombatStatsHookError::ReceiptChangedDuringObservation),
        None => {
            receipts.installed = Some(observed);
            Ok(observed)
        }
    }
}

/// release.
fn installed_receipt_exists() -> Result<bool, DivisionCombatStatsHookError> {
    let receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| DivisionCombatStatsHookError::ReceiptCustodyPoisoned)?;
    Ok(receipts.installed.is_some() || receipts.released.is_some())
}

fn observe_release_receipt() -> Result<ReleasedRvaHookReceipt, DivisionCombatStatsHookError> {
    let installed = {
        let receipts = HOOK_RECEIPTS
            .lock()
            .map_err(|_| DivisionCombatStatsHookError::ReceiptCustodyPoisoned)?;
        if let Some(released) = receipts.released {
            return Ok(released);
        }
        if !receipts.removed_by_custody {
            return Err(DivisionCombatStatsHookError::ReleaseRefused(
                "release attestation lacks owner-performed hook removal".into(),
            ));
        }
        receipts
            .installed
            .ok_or(DivisionCombatStatsHookError::InstalledReceiptUnavailable)?
    };
    let attestation = HookTeardownAttestation {
        callbacks_quiesced: true,
        hook_removed: true,
        relay_freed: true,
    };
    let released = observe_released(EXPECTED_HOOK, installed, attestation)
        .map_err(DivisionCombatStatsHookError::ReleaseRefused)?;
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| DivisionCombatStatsHookError::ReceiptCustodyPoisoned)?;
    match receipts.released {
        Some(existing) if existing == released => Ok(existing),
        Some(_) => Err(DivisionCombatStatsHookError::ReceiptChangedDuringObservation),
        None if receipts.installed == Some(installed) => {
            receipts.released = Some(released);
            receipts.installed = None;
            Ok(released)
        }
        None => Err(DivisionCombatStatsHookError::ReceiptChangedDuringObservation),
    }
}

pub fn drain_latest() -> Result<CapturedFrame, DivisionCombatStatsDrainError> {
    if LIFECYCLE_STATE.load(Ordering::Acquire) != LIFECYCLE_INSTALLED {
        return Err(DivisionCombatStatsDrainError::NotInstalled);
    }
    let state = CAPTURE
        .get()
        .ok_or(DivisionCombatStatsDrainError::StateUnavailable)?;
    let (rounds, drops) = {
        let mut state = state
            .lock()
            .map_err(|_| DivisionCombatStatsDrainError::QueuePoisoned)?;
        let rounds: Vec<ObservedRound> = state.rounds.drain(..).collect();
        let drops = mem::take(&mut state.drops);
        (rounds, drops)
    };
    Ok(CapturedFrame {
        rounds,
        dropped_lock_busy: DROPPED_LOCK_BUSY.swap(0, Ordering::AcqRel),
        dropped_queue_full: drops.queue_full,
        dropped_unrecognised_side: drops.unrecognised_side,
        dropped_unlinked_combat: drops.unlinked_combat,
        dropped_unreadable_side_state: drops.unreadable_side_state,
        dropped_oversized_roster: drops.oversized_roster,
        dropped_unresolvable_division: drops.unresolvable_division,
    })
}

pub fn uninstall() -> Result<(), DivisionCombatStatsHookError> {
    let _guard = LIFECYCLE
        .lock()
        .map_err(|_| DivisionCombatStatsHookError::LifecyclePoisoned)?;
    if LIFECYCLE_STATE.load(Ordering::Acquire) == LIFECYCLE_ABSENT {
        return Ok(());
    }
    seal_capture();
    let base = MODULE_BASE.load(Ordering::Acquire);
    let target = base
        .checked_add(contract::COMBAT_ROUND_RVA as usize)
        .ok_or(DivisionCombatStatsHookError::RoundAddressOverflow)?;

    match unsafe { inline_detour::disable(target as *mut c_void) } {
        Ok(())
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
        Err(status) => {
            apply_teardown_outcome(TeardownOutcome { patch_gone: false });
            return Err(DivisionCombatStatsHookError::HookDisableFailed(
                status.to_string(),
            ));
        }
    }
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
    match unsafe { inline_detour::remove(target as *mut c_void) } {
        Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
        Err(status) => {
            apply_teardown_outcome(TeardownOutcome { patch_gone: false });
            return Err(DivisionCombatStatsHookError::HookRemovalFailed(
                status.to_string(),
            ));
        }
    }
    match HOOK_RECEIPTS.lock() {
        Ok(mut receipts) => receipts.removed_by_custody = true,
        Err(_) => {
            apply_teardown_outcome(TeardownOutcome { patch_gone: false });
            return Err(DivisionCombatStatsHookError::ReceiptCustodyPoisoned);
        }
    }
    // the release receipt, so this cannot run before removal succeeded — but it CAN
    // A site that was never attested has no installed receipt to release. That
    // not prove removal: the site is pending, and demanding a release receipt it
    let attested = match installed_receipt_exists() {
        Ok(exists) => exists,
        Err(error) => {
            apply_teardown_outcome(TeardownOutcome { patch_gone: false });
            return Err(error);
        }
    };
    if attested {
        if let Err(error) = observe_release_receipt() {
            apply_teardown_outcome(TeardownOutcome { patch_gone: false });
            return Err(error);
        }
    }
    apply_teardown_outcome(TeardownOutcome { patch_gone: true });
    Ok(())
}

/// to say what it dropped. That leaves a window: if removal or release attestation
static INSTALL_ABORT_DISCARD: Mutex<production_hook_receipt::InstallAbortDiscard> =
    Mutex::new(production_hook_receipt::InstallAbortDiscard::ZERO);

fn seal_and_take_producer_custody() {
    seal_capture();
    let mut observed = production_hook_receipt::InstallAbortDiscard::ZERO;
    match CAPTURE.get().map(|state| state.lock()) {
        Some(Ok(mut state)) => {
            observed.retained_rows = state
                .rounds
                .iter()
                .map(|round| round.divisions.len() as u64)
                .sum();
            let drops = mem::take(&mut state.drops);
            observed.discarded_diagnostics = drops.queue_full
                + drops.unrecognised_side
                + drops.unlinked_combat
                + drops.unreadable_side_state
                + drops.oversized_roster
                + drops.unresolvable_division;
            state.rounds.clear();
        }
        Some(Err(poisoned)) => {
            observed.poisoned = true;
            let mut state = poisoned.into_inner();
            observed.retained_rows = state
                .rounds
                .iter()
                .map(|round| round.divisions.len() as u64)
                .sum();
            state.rounds.clear();
            state.drops = DropTally::default();
        }
        None => {}
    }
    observed.discarded_diagnostics += DROPPED_LOCK_BUSY.swap(0, Ordering::AcqRel);
    INSTALL_ABORT_DISCARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .merge(observed);
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn force_lifecycle_installed_for_test() {
    LIFECYCLE_STATE.store(LIFECYCLE_INSTALLED, Ordering::Release);
}

/// Abandon a partially completed production install: seal, release, discard.
pub fn abort_partial_install_and_uninstall(
) -> Result<production_hook_receipt::InstallAbortDiscard, String> {
    seal_and_take_producer_custody();
    // RELEASE. This quiesces callbacks and removes the detour, and it is the step
    uninstall().map_err(|error| format!("D99 install abort: {error}"))?;
    Ok(mem::replace(
        &mut *INSTALL_ABORT_DISCARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        production_hook_receipt::InstallAbortDiscard::ZERO,
    ))
}

unsafe extern "system" fn hook_combat_round(side: usize, opponent: usize) {
    let _lease = CallbackLease::enter();
    let original_addr = ORIGINAL.load(Ordering::Acquire);
    let epoch = CAPTURE_EPOCH.load(Ordering::Acquire);
    if CAPTURE_ARMED.load(Ordering::Acquire)
        && LIFECYCLE_STATE.load(Ordering::Acquire) == LIFECYCLE_INSTALLED
    {
        capture_round(side, epoch);
    }
    if original_addr != 0 {
        let original: CombatRoundFn = mem::transmute(original_addr);
        original(side, opponent);
    }
}

fn capture_round(side: usize, epoch: u64) {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let mut drops = DropTally::default();
    let round = match observe_side(base, side, &mut drops) {
        Ok(round) => Some(round),
        Err(drop) => {
            *drops.counter_mut(drop) += 1;
            None
        }
    };
    publish_capture(epoch, round, drops);
}

fn publish_capture(epoch: u64, round: Option<ObservedRound>, mut drops: DropTally) -> bool {
    let Some(state) = CAPTURE.get() else {
        DROPPED_LOCK_BUSY.fetch_add(1, Ordering::AcqRel);
        return false;
    };
    match state.try_lock() {
        Ok(mut state) => {
            if epoch != CAPTURE_EPOCH.load(Ordering::Acquire) {
                return true;
            }
            if let Some(round) = round {
                if state.rounds.len() >= MAX_QUEUED_ROUNDS {
                    drops.queue_full = drops.queue_full.saturating_add(1);
                } else {
                    state.rounds.push_back(round);
                }
            }
            state.drops.add(drops);
            false
        }
        Err(_) => {
            DROPPED_LOCK_BUSY.fetch_add(1, Ordering::AcqRel);
            false
        }
    }
}

fn observe_side(
    base: usize,
    side: usize,
    drops: &mut DropTally,
) -> Result<ObservedRound, SideDrop> {
    use SideDrop::{OversizedRoster, UnlinkedCombat, UnreadableSideState, UnrecognisedSide};

    let side_vtable_rva = read_u64(side)
        .ok_or(UnrecognisedSide)?
        .checked_sub(base as u64)
        .ok_or(UnrecognisedSide)?;
    let side_class = if side_vtable_rva == contract::SIDE_VTABLE_RVA as u64 {
        SideClass::Primary
    } else if side_vtable_rva == contract::SIDE_SIBLING_VTABLE_RVA as u64 {
        SideClass::Sibling
    } else {
        return Err(UnrecognisedSide);
    };

    let at = |object: usize, offset: usize, reason: SideDrop| -> Result<usize, SideDrop> {
        object.checked_add(offset).ok_or(reason)
    };
    let combat = read_u64(at(
        side,
        contract::SIDE_PARENT_COMBAT_OFFSET,
        UnlinkedCombat,
    )?)
    .ok_or(UnlinkedCombat)? as usize;
    let attacker = read_u64(at(
        combat,
        contract::COMBAT_ATTACKER_SIDE_OFFSET,
        UnlinkedCombat,
    )?)
    .ok_or(UnlinkedCombat)? as usize;
    let defender = read_u64(at(
        combat,
        contract::COMBAT_DEFENDER_SIDE_OFFSET,
        UnlinkedCombat,
    )?)
    .ok_or(UnlinkedCombat)? as usize;
    let role = if attacker == side {
        contract::SIDE_ATTACKER
    } else if defender == side {
        contract::SIDE_DEFENDER
    } else {
        return Err(UnlinkedCombat);
    };
    let handle = read_u64(at(combat, contract::COMBAT_HANDLE_OFFSET, UnlinkedCombat)?)
        .ok_or(UnlinkedCombat)?;
    let combat_id = (handle >> 32) as u32;
    if combat_id == 0 {
        return Err(UnlinkedCombat);
    }

    let discriminator = read_u8(at(
        side,
        contract::SIDE_DISCRIMINATOR_OFFSET,
        UnreadableSideState,
    )?)
    .ok_or(UnreadableSideState)?;
    let bonus_offset = if discriminator != 0 {
        contract::COMBAT_SIDE_BONUS_DISCRIMINATOR_SET_OFFSET
    } else {
        contract::COMBAT_SIDE_BONUS_DISCRIMINATOR_CLEAR_OFFSET
    };
    let combat_side_bonus_raw =
        read_i64(at(combat, bonus_offset, UnreadableSideState)?).ok_or(UnreadableSideState)?;

    let data = read_u64(at(
        side,
        contract::SIDE_COMMITTED_DATA_OFFSET,
        UnreadableSideState,
    )?)
    .ok_or(UnreadableSideState)? as usize;
    let capacity = read_u32(at(
        side,
        contract::SIDE_COMMITTED_CAPACITY_OFFSET,
        UnreadableSideState,
    )?)
    .ok_or(UnreadableSideState)?;
    let committed_count = read_u32(at(
        side,
        contract::SIDE_COMMITTED_COUNT_OFFSET,
        UnreadableSideState,
    )?)
    .ok_or(UnreadableSideState)?;
    if committed_count > capacity || committed_count as usize > MAX_COMMITTED_PER_ROUND {
        return Err(OversizedRoster);
    }

    let divisions = walk_committed_vector(committed_count, drops, |index| {
        index
            .checked_mul(mem::size_of::<usize>())
            .and_then(|offset| data.checked_add(offset))
            .and_then(read_u64)
            .and_then(|wrapper| observe_division(base, wrapper as usize, combat_side_bonus_raw))
    });

    Ok(ObservedRound {
        combat_id,
        side: role,
        side_class,
        side_ptr: side,
        committed_count,
        divisions,
    })
}

fn walk_committed_vector(
    committed_count: u32,
    drops: &mut DropTally,
    mut read_element: impl FnMut(usize) -> Option<ObservedDivision>,
) -> Vec<ObservedDivision> {
    let mut divisions = Vec::with_capacity(committed_count as usize);
    for index in 0..committed_count as usize {
        match read_element(index) {
            Some(division) => divisions.push(division),
            None => {
                drops.unresolvable_division = drops.unresolvable_division.saturating_add(1);
            }
        }
    }
    divisions
}

fn observe_division(
    base: usize,
    wrapper: usize,
    combat_side_bonus_raw: i64,
) -> Option<ObservedDivision> {
    let division_id = resolve_division_id(base, wrapper)?;

    let count_before = read_u32(wrapper.checked_add(contract::DIVISION_MODIFIER_COUNT_OFFSET)?)?;
    let mut window = [0u8; WRAPPER_WINDOW_LEN];
    read_exact(wrapper, &mut window).ok()?;
    let count_after = read_u32(wrapper.checked_add(contract::DIVISION_MODIFIER_COUNT_OFFSET)?)?;

    if count_before != count_after {
        return Some(ObservedDivision {
            division_id,
            observation: contract::RowObservation::TornRead,
            stats: None,
            scale: None,
            combat_side_bonus_raw: None,
            modifier_count_before: count_before,
            modifier_count_after: count_after,
        });
    }

    let scale = contract::ScaleFields {
        attack_accumulator_raw: window_i64(&window, contract::DIVISION_ATTACK_ACCUMULATOR_OFFSET)?,
        defence_accumulator_raw: window_i64(
            &window,
            contract::DIVISION_DEFENCE_ACCUMULATOR_OFFSET,
        )?,
        attack_base_raw: window_i64(&window, contract::DIVISION_ATTACK_BASE_OFFSET)?,
        defence_base_raw: window_i64(&window, contract::DIVISION_DEFENCE_BASE_OFFSET)?,
    };
    let stat_block = window_i64(&window, contract::DIVISION_STAT_BLOCK_PTR_OFFSET)? as usize;
    let stats = (stat_block != 0)
        .then(|| read_stat_block(stat_block))
        .flatten();

    let observation = match (stat_block, stats) {
        (0, _) => contract::RowObservation::StatBlockNull,
        (_, Some(_)) => contract::RowObservation::RoundEntryFold,
        (_, None) => contract::RowObservation::Unread,
    };
    let carries_scale = observation.carries_scale();

    Some(ObservedDivision {
        division_id,
        observation,
        stats: observation.carries_stats().then_some(()).and(stats),
        scale: carries_scale.then_some(scale),
        combat_side_bonus_raw: carries_scale.then_some(combat_side_bonus_raw),
        modifier_count_before: count_before,
        modifier_count_after: count_after,
    })
}

fn resolve_division_id(base: usize, wrapper: usize) -> Option<u32> {
    if read_u64(wrapper)?.checked_sub(base as u64)? != contract::DIVISION_WRAPPER_VTABLE_RVA as u64
    {
        return None;
    }
    let body = read_u64(wrapper.checked_add(contract::DIVISION_WRAPPER_TO_BODY_OFFSET)?)? as usize;
    if read_u64(body)?.checked_sub(base as u64)? != contract::DIVISION_BODY_VTABLE_RVA as u64 {
        return None;
    }
    let division_id = read_u32(body.checked_add(contract::DIVISION_BODY_ID_OFFSET)?)?;
    (division_id != 0).then_some(division_id)
}

fn read_stat_block(stat_block: usize) -> Option<contract::CachedStatBlock> {
    let mut window = [0u8; STAT_WINDOW_LEN];
    read_exact(
        stat_block.checked_add(contract::STAT_DEFENSE_OFFSET)?,
        &mut window,
    )
    .ok()?;
    let member = |offset: usize| -> Option<i64> {
        window_i64(&window, offset.checked_sub(contract::STAT_DEFENSE_OFFSET)?)
    };
    Some(contract::CachedStatBlock {
        defense_raw: member(contract::STAT_DEFENSE_OFFSET)?,
        breakthrough_raw: member(contract::STAT_BREAKTHROUGH_OFFSET)?,
        hardness_raw: member(contract::STAT_HARDNESS_OFFSET)?,
        soft_attack_raw: member(contract::STAT_SOFT_ATTACK_OFFSET)?,
        hard_attack_raw: member(contract::STAT_HARD_ATTACK_OFFSET)?,
    })
}

fn window_i64(window: &[u8], at: usize) -> Option<i64> {
    let end = at.checked_add(mem::size_of::<i64>())?;
    let bytes: [u8; 8] = window.get(at..end)?.try_into().ok()?;
    Some(i64::from_le_bytes(bytes))
}

fn read_exact(address: usize, into: &mut [u8]) -> Result<(), String> {
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            into.as_mut_ptr().cast(),
            into.len(),
            &mut read,
        )
    };
    if ok == 0 || read != into.len() {
        return Err(format!("read {} bytes at {address:#x} failed", into.len()));
    }
    Ok(())
}

fn read_u8(address: usize) -> Option<u8> {
    let mut bytes = [0u8; 1];
    read_exact(address, &mut bytes).ok()?;
    Some(bytes[0])
}

fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_u64(address: usize) -> Option<u64> {
    let mut bytes = [0u8; 8];
    read_exact(address, &mut bytes).ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn read_i64(address: usize) -> Option<i64> {
    read_u64(address).map(|value| value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    static GLOBAL_STATE_SERIAL: Mutex<()> = Mutex::new(());

    fn global_state_lock() -> std::sync::MutexGuard<'static, ()> {
        GLOBAL_STATE_SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn division(index: usize) -> ObservedDivision {
        ObservedDivision {
            division_id: 4_000 + index as u32,
            observation: contract::RowObservation::Unread,
            stats: None,
            scale: None,
            combat_side_bonus_raw: None,
            modifier_count_before: 0,
            modifier_count_after: 0,
        }
    }

    #[test]
    fn the_hook_specification_is_the_admitted_primitive_at_the_proved_site() {
        assert_eq!(EXPECTED_HOOK.primitive_id, MINHOOK_NORMAL_REL5_PRIMITIVE_ID);
        assert_eq!(EXPECTED_HOOK.target_patch_len, 5);
        assert_eq!(
            EXPECTED_HOOK.requested_rva,
            contract::COMBAT_ROUND_RVA as u64
        );
        assert_eq!(EXPECTED_HOOK.requested_rva, 0x012A_3220);
        assert_eq!(EXPECTED_HOOK.logical_name, ROUND_LOGICAL_NAME);
        assert_eq!(EXPECTED_HOOK.original_prefix, &ROUND_SENTINEL);
        assert_eq!(
            &ROUND_SENTINEL[..5],
            &[0x48, 0x89, 0x54, 0x24, 0x10],
            "the patched prefix must be the complete first instruction"
        );
        assert!(ROUND_SENTINEL.len() >= usize::from(EXPECTED_HOOK.target_patch_len));
        assert_eq!(HOOK_RECEIPT_COUNT, 1);
    }

    #[test]
    fn the_bulk_read_windows_cover_exactly_the_carried_offsets() {
        assert_eq!(WRAPPER_WINDOW_LEN, 0x1A0 + 8);
        for offset in [
            contract::DIVISION_STAT_BLOCK_PTR_OFFSET,
            contract::DIVISION_ATTACK_ACCUMULATOR_OFFSET,
            contract::DIVISION_DEFENCE_ACCUMULATOR_OFFSET,
            contract::DIVISION_ATTACK_BASE_OFFSET,
            contract::DIVISION_DEFENCE_BASE_OFFSET,
        ] {
            assert!(
                offset + 8 <= WRAPPER_WINDOW_LEN,
                "wrapper window does not reach {offset:#x}"
            );
        }
        assert_eq!(STAT_WINDOW_LEN, 0xC8 - 0xA8 + 8);
        for offset in [
            contract::STAT_DEFENSE_OFFSET,
            contract::STAT_BREAKTHROUGH_OFFSET,
            contract::STAT_HARDNESS_OFFSET,
            contract::STAT_SOFT_ATTACK_OFFSET,
            contract::STAT_HARD_ATTACK_OFFSET,
        ] {
            assert!(
                offset - contract::STAT_DEFENSE_OFFSET + 8 <= STAT_WINDOW_LEN,
                "stat window does not reach {offset:#x}"
            );
        }
    }

    #[test]
    fn window_reads_are_little_endian_and_bounded() {
        let mut window = [0u8; 24];
        window[8..16].copy_from_slice(&(-2_i64).to_le_bytes());
        window[16..24].copy_from_slice(&123_456_789_i64.to_le_bytes());
        assert_eq!(window_i64(&window, 0), Some(0));
        assert_eq!(window_i64(&window, 8), Some(-2));
        assert_eq!(window_i64(&window, 16), Some(123_456_789));
        assert_eq!(
            window_i64(&window, 17),
            None,
            "a partial tail is not a value"
        );
        assert_eq!(window_i64(&window, 24), None);
        assert_eq!(window_i64(&window, usize::MAX), None);
    }

    #[test]
    fn draining_an_uninstalled_hook_is_typed_not_empty() {
        let _serial = global_state_lock();
        assert!(!installed());
        assert_eq!(
            drain_latest(),
            Err(DivisionCombatStatsDrainError::NotInstalled)
        );
    }

    #[test]
    fn every_side_authentication_failure_is_a_typed_tally() {
        let mut drops = DropTally::default();
        assert_eq!(
            observe_side(0x1000, 0x10, &mut drops),
            Err(SideDrop::UnrecognisedSide),
            "an unreadable side must be a typed drop, not a bare None"
        );

        let mut tally = DropTally::default();
        let reasons = [
            SideDrop::UnrecognisedSide,
            SideDrop::UnlinkedCombat,
            SideDrop::UnreadableSideState,
            SideDrop::OversizedRoster,
        ];
        for (index, reason) in reasons.iter().enumerate() {
            *tally.counter_mut(*reason) += index as u64 + 1;
        }
        assert_eq!(tally.unrecognised_side, 1);
        assert_eq!(tally.unlinked_combat, 2);
        assert_eq!(tally.unreadable_side_state, 3);
        assert_eq!(tally.oversized_roster, 4);
        assert_eq!(
            tally.unresolvable_division, 0,
            "a side reason must not land on the per-element counter"
        );
    }

    #[test]
    fn one_unreadable_element_costs_one_element_and_is_tallied() {
        let mut drops = DropTally::default();
        let divisions =
            walk_committed_vector(4, &mut drops, |index| (index != 1).then(|| division(index)));
        assert_eq!(drops.unresolvable_division, 1);
        assert_eq!(
            divisions.iter().map(|d| d.division_id).collect::<Vec<_>>(),
            vec![4_000, 4_002, 4_003],
            "an unreadable element must not take its neighbours with it"
        );

        let mut drops = DropTally::default();
        let divisions =
            walk_committed_vector(3, &mut drops, |index| (index != 0).then(|| division(index)));
        assert_eq!(drops.unresolvable_division, 1);
        assert_eq!(
            divisions.len(),
            2,
            "a bad first element must not abandon the walk"
        );

        let mut drops = DropTally::default();
        let divisions = walk_committed_vector(3, &mut drops, |_| None);
        assert_eq!(
            drops.unresolvable_division, 3,
            "every lost element must be tallied"
        );
        assert!(divisions.is_empty());

        let mut drops = DropTally::default();
        let divisions = walk_committed_vector(0, &mut drops, |index| Some(division(index)));
        assert_eq!(drops.unresolvable_division, 0);
        assert!(divisions.is_empty());

        let mut drops = DropTally::default();
        let divisions = walk_committed_vector(3, &mut drops, |index| Some(division(index)));
        assert_eq!(drops.unresolvable_division, 0);
        assert_eq!(divisions.len(), 3);
    }

    #[test]
    fn every_drop_reason_accumulates_into_its_own_field() {
        let setters: [(fn(&mut DropTally), fn(&DropTally) -> u64, &str); 6] = [
            (|d| d.queue_full = 1, |d| d.queue_full, "queue_full"),
            (
                |d| d.unrecognised_side = 1,
                |d| d.unrecognised_side,
                "unrecognised_side",
            ),
            (
                |d| d.unlinked_combat = 1,
                |d| d.unlinked_combat,
                "unlinked_combat",
            ),
            (
                |d| d.unreadable_side_state = 1,
                |d| d.unreadable_side_state,
                "unreadable_side_state",
            ),
            (
                |d| d.oversized_roster = 1,
                |d| d.oversized_roster,
                "oversized_roster",
            ),
            (
                |d| d.unresolvable_division = 1,
                |d| d.unresolvable_division,
                "unresolvable_division",
            ),
        ];
        for (set, get, name) in setters {
            let mut one = DropTally::default();
            set(&mut one);
            let mut total = DropTally::default();
            total.add(one);
            total.add(one);
            assert_eq!(get(&total), 2, "{name} does not accumulate");
            let others: u64 = [
                total.queue_full,
                total.unrecognised_side,
                total.unlinked_combat,
                total.unreadable_side_state,
                total.oversized_roster,
                total.unresolvable_division,
            ]
            .iter()
            .sum();
            assert_eq!(others, 2, "{name} leaked into another field");
        }
    }

    #[test]
    fn capture_does_nothing_until_it_is_armed() {
        let _serial = global_state_lock();
        let state = CAPTURE.get_or_init(|| Mutex::new(CaptureState::default()));
        let reset = || {
            let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
            state.rounds.clear();
            state.drops = DropTally::default();
        };
        let tallied = || {
            let state = state.lock().unwrap_or_else(|p| p.into_inner());
            state.drops
        };
        let previous_base = MODULE_BASE.swap(0x1000, Ordering::AcqRel);
        let previous_state = LIFECYCLE_STATE.swap(LIFECYCLE_INSTALLED, Ordering::AcqRel);

        assert_eq!(ORIGINAL.load(Ordering::Acquire), 0);
        let detour = |side: usize| unsafe { hook_combat_round(side, 0) };

        disarm_capture();
        reset();
        assert!(!capture_armed());
        detour(0x10);
        assert_eq!(
            tallied(),
            DropTally::default(),
            "a disarmed detour must tally nothing"
        );

        arm_capture();
        assert!(capture_armed());
        detour(0x10);
        assert_eq!(
            tallied().unrecognised_side,
            1,
            "an armed detour must reach the capture path and tally its drop"
        );

        disarm_capture();
        assert_eq!(
            tallied(),
            DropTally::default(),
            "disarming must discard the queue"
        );
        LIFECYCLE_STATE.store(previous_state, Ordering::Release);
        MODULE_BASE.store(previous_base, Ordering::Release);
    }

    #[test]
    fn a_failed_rollback_leaves_the_site_reachable_by_teardown() {
        let (clear_addresses, state) =
            teardown_state_transition(TeardownOutcome { patch_gone: false });
        assert!(
            !clear_addresses,
            "an unproved removal must keep the trampoline, or a detour that does              fire returns without calling the original and skips combat resolution"
        );
        assert_eq!(
            state, LIFECYCLE_TEARDOWN_PENDING,
            "an unproved removal must stay reachable by teardown"
        );
        assert_ne!(state, LIFECYCLE_ABSENT, "absent would strand a live patch");
        assert_ne!(
            state, LIFECYCLE_INSTALLED,
            "installed would advertise a hook whose receipt was never observed"
        );
    }

    #[test]
    fn the_failed_install_rollback_quiesces_before_removing() {
        let source = include_str!("a729_hook.rs").replace("\r\n", "\n");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("a729_hook.rs has a test module")
            .0;
        let start = production
            .find("fn rollback_failed_install(")
            .expect("rollback is declared in production");
        let body = &production[start..];
        let end = body
            .find(
                "
}
",
            )
            .expect("rollback's body is closed");
        let body = &body[..end];
        let wait = body
            .find("ACTIVE_CALLBACKS.load(")
            .expect("rollback must wait for callbacks to quiesce");
        let remove = body
            .find("inline_detour::remove(")
            .expect("rollback removes the hook");
        assert!(
            wait < remove,
            "rollback frees the trampoline before callbacks have quiesced"
        );
    }

    /// Law: the release attestation runs BEFORE the state a retry needs is dropped.
    /// `observe_release_receipt` is fallible. Clearing `INSTALLED` first made a
    #[test]
    fn a_failed_release_attestation_keeps_the_retry_route_open() {
        let source = include_str!("a729_hook.rs").replace("\r\n", "\n");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("a729_hook.rs has a test module")
            .0;
        let start = production
            .find("pub fn uninstall()")
            .expect("uninstall is declared in production");
        let body = &production[start..];
        let end = body
            .find(
                "
}
",
            )
            .expect("uninstall's body is closed");
        let body = &body[..end];

        let attest = body
            .find("observe_release_receipt()")
            .expect("uninstall must attest the release inside its own body");
        let release = body
            .find("apply_teardown_outcome(TeardownOutcome { patch_gone: true });")
            .expect("uninstall must release the site inside its own body");
        assert!(
            attest < release,
            "uninstall drops the retry state before attesting, so a failed              attestation could never be retried"
        );
    }

    #[test]
    fn an_unproved_removal_is_neither_absent_nor_installed() {
        assert_eq!(
            teardown_state_transition(TeardownOutcome { patch_gone: true }),
            (true, LIFECYCLE_ABSENT),
            "a proved removal clears the addresses and releases the site"
        );
        assert_eq!(
            teardown_state_transition(TeardownOutcome { patch_gone: false }),
            (false, LIFECYCLE_TEARDOWN_PENDING),
            "an unproved removal keeps the trampoline and enters teardown-pending"
        );
        let states = [
            LIFECYCLE_ABSENT,
            LIFECYCLE_INSTALLED,
            LIFECYCLE_TEARDOWN_PENDING,
        ];
        for (i, a) in states.iter().enumerate() {
            for b in states.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn a_teardown_pending_site_is_not_reported_as_installed_or_drainable() {
        let _serial = global_state_lock();
        let previous = LIFECYCLE_STATE.swap(LIFECYCLE_TEARDOWN_PENDING, Ordering::AcqRel);
        assert!(
            !installed(),
            "teardown-pending must not report as installed"
        );
        assert!(teardown_pending());
        assert_eq!(
            drain_latest(),
            Err(DivisionCombatStatsDrainError::NotInstalled),
            "a site whose receipt was never observed must not be drainable"
        );
        LIFECYCLE_STATE.store(previous, Ordering::Release);
    }

    #[test]
    fn the_capture_epoch_is_stamped_before_the_arming_flag_is_read() {
        let source = include_str!("a729_hook.rs").replace("\r\n", "\n");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("a729_hook.rs has a test module")
            .0;
        let start = production
            .find("unsafe extern \"system\" fn hook_combat_round(")
            .expect("the detour is declared in production");
        let body = &production[start..];
        let end = body.find("\n}\n").expect("the detour body is closed");
        let body = &body[..end];
        let stamp = body
            .find("CAPTURE_EPOCH.load(")
            .expect("the detour stamps the capture epoch");
        let armed = body
            .find("CAPTURE_ARMED.load(")
            .expect("the detour reads the arming flag");
        assert!(
            stamp < armed,
            "the detour reads the arming flag before stamping the epoch, so a \
             disarm in between is invisible to publication"
        );
    }

    #[test]
    fn a_failed_disable_neither_waits_nor_removes() {
        let source = include_str!("a729_hook.rs").replace("\r\n", "\n");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("a729_hook.rs has a test module")
            .0;
        let start = production
            .find("fn rollback_failed_install(")
            .expect("rollback is declared in production");
        let body = &production[start..];
        let end = body.find("\n}\n").expect("rollback's body is closed");
        let body = &body[..end];
        let guard = body
            .find("let removed = if disabled {")
            .expect("removal must be guarded by a proved disable");
        let wait = body
            .find("ACTIVE_CALLBACKS.load(")
            .expect("rollback waits for callbacks to quiesce");
        let remove = body
            .find("inline_detour::remove(")
            .expect("rollback removes the hook");
        assert!(
            guard < wait && wait < remove,
            "the quiesce wait must sit INSIDE the proved-disable branch, or a \
             failed disable spins forever"
        );
    }

    /// refused observation leaves a live hook with nothing to release. Demanding a
    /// release receipt it never had would strand the site teardown-pending forever
    #[test]
    fn a_never_attested_site_needs_no_release_receipt() {
        let _serial = global_state_lock();
        assert!(
            !installed_receipt_exists().expect("receipt custody is readable"),
            "no receipt should exist before any installation"
        );

        let source = include_str!("a729_hook.rs").replace("\r\n", "\n");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("a729_hook.rs has a test module")
            .0;
        let start = production
            .find("pub fn uninstall()")
            .expect("uninstall is declared in production");
        let body = &production[start..];
        let end = body.find("\n}\n").expect("uninstall body is closed");
        let body = &body[..end];
        assert!(
            body.contains("if attested {"),
            "the release attestation is not guarded by whether a receipt was ever              minted, so a site whose install was refused before attestation can              never be retired"
        );
        assert!(
            body.contains("match installed_receipt_exists()"),
            "uninstall demands a release receipt unconditionally, so a site whose \
             installation was refused before attestation can never be retired"
        );
        let removal = body
            .find("inline_detour::remove(")
            .expect("uninstall removes the hook");
        let tail: String = body[removal..]
            .lines()
            .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        assert!(
            !tail.contains('?'),
            "an exit after removal propagates with `?` and skips \
             apply_teardown_outcome, so a removed hook stays advertised as \
             installed:\n{tail}"
        );
    }

    /// Law: an install abort seals the producer, PHYSICALLY releases the site, and
    /// a release it never performed, which is the fail-open shape this project
    #[test]
    fn an_install_abort_seals_releases_and_reports_its_discard() {
        let _serial = global_state_lock();
        let state = CAPTURE.get_or_init(|| Mutex::new(CaptureState::default()));

        {
            let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
            state.rounds.clear();
            state.rounds.push_back(ObservedRound {
                combat_id: 7,
                side: contract::SIDE_ATTACKER,
                side_class: SideClass::Primary,
                side_ptr: 0x1000,
                committed_count: 2,
                divisions: vec![division(0), division(1)],
            });
            state.rounds.push_back(ObservedRound {
                combat_id: 7,
                side: contract::SIDE_DEFENDER,
                side_class: SideClass::Primary,
                side_ptr: 0x2000,
                committed_count: 1,
                divisions: vec![division(2)],
            });
            state.drops = DropTally {
                unrecognised_side: 4,
                ..DropTally::default()
            };
        }
        DROPPED_LOCK_BUSY.store(1, Ordering::Release);
        LIFECYCLE_STATE.store(LIFECYCLE_INSTALLED, Ordering::Release);
        arm_capture();
        assert!(installed() && capture_armed());

        let discard =
            abort_partial_install_and_uninstall().expect("the abort must complete cleanly");

        // RELEASED. This is the assertion the registry's own re-read duplicates.
        assert!(
            !installed(),
            "the abort reported success while the site was still installed"
        );
        assert!(
            !teardown_pending(),
            "a proved removal must not leave the site pending"
        );
        assert!(!capture_armed(), "the abort must stop the producer");
        {
            let source = include_str!("a729_hook.rs").replace("\r\n", "\n");
            let production = source
                .split_once("#[cfg(test)]")
                .expect("a729_hook.rs has a test module")
                .0;
            let custody = production
                .find("fn seal_and_take_producer_custody() {")
                .expect("the custody helper is declared in production");
            let body = &production[custody..];
            let body = &body[..body
                .find(
                    "
}
",
                )
                .expect("its body is closed")];
            let seal = body.find("seal_capture();").expect("it seals");
            let count = body
                .find("observed.retained_rows")
                .expect("it counts what the producer held");
            assert!(
                seal < count,
                "the producer is counted before it is sealed, so a live detour can                  add rows between the count and the removal"
            );

            let abort = production
                .find("pub fn abort_partial_install_and_uninstall(")
                .expect("the abort is declared in production");
            let body = &production[abort..];
            let body = &body[..body
                .find(
                    "
}
",
                )
                .expect("its body is closed")];
            let take = body
                .find("seal_and_take_producer_custody();")
                .expect("the abort takes custody");
            let release = body.find("uninstall()").expect("the abort releases");
            assert!(
                take < release,
                "the abort releases before taking custody, so a failed release                  loses the count it should have reported"
            );
        }
        assert_eq!(
            discard.retained_rows, 3,
            "the discard must count the division observations the wire never received"
        );
        assert_eq!(discard.discarded_diagnostics, 5);
        assert_eq!(
            discard.reserved_rows, 0,
            "this lane has no reservation stage"
        );
        assert!(!discard.poisoned);
        assert!(state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .rounds
            .is_empty());

        let again = abort_partial_install_and_uninstall().expect("idempotent");
        assert_eq!(again.retained_rows, 0);
        assert_eq!(again.discarded_diagnostics, 0);

        LIFECYCLE_STATE.store(LIFECYCLE_ABSENT, Ordering::Release);
    }

    #[test]
    fn a_round_in_flight_when_capture_stopped_is_not_published() {
        let _serial = global_state_lock();
        let state = CAPTURE.get_or_init(|| Mutex::new(CaptureState::default()));
        let previous_base = MODULE_BASE.swap(0x1000, Ordering::AcqRel);
        let previous_state = LIFECYCLE_STATE.swap(LIFECYCLE_INSTALLED, Ordering::AcqRel);
        let tallied = || {
            let state = state.lock().unwrap_or_else(|p| p.into_inner());
            state.drops
        };

        arm_capture();
        {
            let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
            state.rounds.clear();
            state.drops = DropTally::default();
        }
        capture_round(0x10, CAPTURE_EPOCH.load(Ordering::Acquire));
        assert_eq!(
            tallied().unrecognised_side,
            1,
            "a round captured while armed must publish"
        );

        {
            let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
            state.drops = DropTally::default();
        }
        let stamped = CAPTURE_EPOCH.load(Ordering::Acquire);
        CAPTURE_EPOCH.fetch_add(1, Ordering::AcqRel);
        assert_ne!(stamped, CAPTURE_EPOCH.load(Ordering::Acquire));
        assert!(
            publish_capture(
                stamped,
                None,
                DropTally {
                    unrecognised_side: 1,
                    ..DropTally::default()
                }
            ),
            "the publish helper reports refusal, not success"
        );
        assert_eq!(
            tallied(),
            DropTally::default(),
            "a round stamped before capture stopped must not be published"
        );

        disarm_capture();
        LIFECYCLE_STATE.store(previous_state, Ordering::Release);
        MODULE_BASE.store(previous_base, Ordering::Release);
    }

    #[test]
    fn capture_is_inert_without_a_module_base() {
        let _serial = global_state_lock();
        let state = CAPTURE.get_or_init(|| Mutex::new(CaptureState::default()));
        {
            let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
            state.rounds.clear();
            state.drops = DropTally::default();
        }
        let previous_base = MODULE_BASE.swap(0, Ordering::AcqRel);
        let previous_state = LIFECYCLE_STATE.swap(LIFECYCLE_INSTALLED, Ordering::AcqRel);
        arm_capture();
        capture_round(0xDEAD_BEEF, CAPTURE_EPOCH.load(Ordering::Acquire));
        let drops = {
            let state = state.lock().unwrap_or_else(|p| p.into_inner());
            state.drops
        };
        assert_eq!(drops, DropTally::default());
        disarm_capture();
        LIFECYCLE_STATE.store(previous_state, Ordering::Release);
        MODULE_BASE.store(previous_base, Ordering::Release);
    }

    #[test]
    fn arming_is_refused_unless_the_site_is_installed() {
        let _serial = global_state_lock();
        for state in [LIFECYCLE_ABSENT, LIFECYCLE_TEARDOWN_PENDING] {
            disarm_capture();
            let previous = LIFECYCLE_STATE.swap(state, Ordering::AcqRel);
            arm_capture();
            assert!(
                !capture_armed(),
                "arming must be refused in lifecycle state {state}"
            );
            LIFECYCLE_STATE.store(previous, Ordering::Release);
        }
        let previous = LIFECYCLE_STATE.swap(LIFECYCLE_INSTALLED, Ordering::AcqRel);
        arm_capture();
        assert!(capture_armed(), "an installed site must be armable");
        disarm_capture();
        LIFECYCLE_STATE.store(previous, Ordering::Release);
    }
}
