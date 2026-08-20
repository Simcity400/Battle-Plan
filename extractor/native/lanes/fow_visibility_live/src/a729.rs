use core::ffi::c_void;
use inline_detour::InlineDetourEnableError;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released_with_retained_relay,
    ExpectedRvaHook, HookTeardownAttestation, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::fmt;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{PovVisibilitySnapshot, MAX_POV_COUNTRY_ID, MAX_PROVINCE_SLOTS};

pub const PRODUCER_RVA: u32 = 0x0160_6EA0;
pub const PRODUCER_LOGICAL_NAME: &str = "gameplay.d94.pov_province_visibility_producer";
pub const HOOK_RECEIPT_COUNT: usize = 1;
const PRODUCER_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x60, 0x83, 0xba, 0x20, 0x05,
];
const EXPECTED_HOOK: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: PRODUCER_LOGICAL_NAME,
    requested_rva: PRODUCER_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &PRODUCER_SENTINEL,
};
const CURRENT_BUFFER_OFFSET: usize = 0x00;
const PREVIOUS_BUFFER_OFFSET: usize = 0x08;
const PROVINCE_COUNT_OFFSET: usize = 0x10;
const POV_COUNTRY_ID_OFFSET: usize = 0x14;
const FLAGS_OFFSET: usize = 0x18;
const DIRTY_BYTE_OFFSET: usize = FLAGS_OFFSET;
const TOGGLE_EPOCH_BYTE_OFFSET: usize = 0x1B;
const GAMESTATE_POV_PRIMARY_OFFSET: usize = 0x520;
const GAMESTATE_POV_FALLBACK_OFFSET: usize = 0x524;
const GAMESTATE_CURRENT_DATE_OFFSET: usize = 0x468;

const _: () = assert!(FLAGS_OFFSET == POV_COUNTRY_ID_OFFSET + 4);
const _: () = assert!(DIRTY_BYTE_OFFSET == POV_COUNTRY_ID_OFFSET + 4);
const _: () = assert!(TOGGLE_EPOCH_BYTE_OFFSET == POV_COUNTRY_ID_OFFSET + 7);
const _: () = assert!(GAMESTATE_POV_FALLBACK_OFFSET == GAMESTATE_POV_PRIMARY_OFFSET + 4);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PovVisibilityHookError {
    LifecyclePoisoned,
    ModuleUnavailable,
    ProducerAddressOverflow,
    ProducerReadFailed(String),
    ProducerSentinelMismatch,
    HookCreationFailed(String),
    HookEnableFailed(String),
    LatestSnapshotPoisoned,
    ReceiptCustodyPoisoned,
    ReceiptRefused(String),
    ReceiptChangedDuringObservation,
    ReceiptAlreadyReleased,
    InstalledReceiptUnavailable,
    ReleasedReceiptUnavailable,
    ReleaseRefused(String),
    HookDisableFailed(String),
}

impl fmt::Display for PovVisibilityHookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LifecyclePoisoned => write!(f, "D94 visibility hook lifecycle poisoned"),
            Self::ModuleUnavailable => write!(f, "D94 visibility hook module unavailable"),
            Self::ProducerAddressOverflow => {
                write!(f, "D94 visibility producer address overflow")
            }
            Self::ProducerReadFailed(detail) => {
                write!(f, "D94 visibility producer read failed: {detail}")
            }
            Self::ProducerSentinelMismatch => {
                write!(f, "D94 visibility producer sentinel mismatch")
            }
            Self::HookCreationFailed(detail) => {
                write!(f, "D94 visibility producer hook creation failed: {detail}")
            }
            Self::HookEnableFailed(detail) => {
                write!(f, "D94 visibility producer hook enable failed: {detail}")
            }
            Self::LatestSnapshotPoisoned => {
                write!(f, "D94 visibility latest snapshot poisoned")
            }
            Self::ReceiptCustodyPoisoned => {
                write!(f, "D94 visibility hook receipt custody poisoned")
            }
            Self::ReceiptRefused(detail) => write!(
                f,
                "D94 visibility production hook receipt refused, installation rolled back: {detail}"
            ),
            Self::ReceiptChangedDuringObservation => {
                write!(f, "D94 visibility hook receipt changed during observation")
            }
            Self::ReceiptAlreadyReleased => {
                write!(f, "D94 visibility hook receipt was already released")
            }
            Self::InstalledReceiptUnavailable => {
                write!(f, "D94 installed production hook receipt is unavailable")
            }
            Self::ReleasedReceiptUnavailable => {
                write!(f, "D94 released production hook receipt is unavailable")
            }
            Self::ReleaseRefused(detail) => {
                write!(f, "D94 visibility hook release receipt refused: {detail}")
            }
            Self::HookDisableFailed(detail) => {
                write!(f, "D94 visibility hook disable failed: {detail}")
            }
        }
    }
}

impl std::error::Error for PovVisibilityHookError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PovVisibilityDrainError {
    NotInstalled,
    StateUnavailable,
    LatestSnapshotPoisoned,
}

impl fmt::Display for PovVisibilityDrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => write!(f, "visibility producer hook is not installed"),
            Self::StateUnavailable => write!(f, "visibility hook state unavailable"),
            Self::LatestSnapshotPoisoned => write!(f, "visibility latest snapshot poisoned"),
        }
    }
}

impl std::error::Error for PovVisibilityDrainError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PovVisibilityInvocationOutcome {
    Republished,
    UnchangedNotRecaptured,
    ObservationRefused,
    PublicationUnavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PovVisibilityInvocationCounts {
    pub republished: u64,
    pub unchanged_not_recaptured: u64,
    pub observation_refused: u64,
    pub publication_unavailable: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DirtyProbe {
    dirty: bool,
    pov_country_id: u32,
    toggle_epoch: u8,
}

type ProducerFn = unsafe extern "system" fn(usize, usize);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Header {
    current: usize,
    previous: usize,
    count: u32,
    pov_country_id: u32,
    flags: u32,
}

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static LATEST: OnceLock<Mutex<Option<std::sync::Arc<PovVisibilitySnapshot>>>> = OnceLock::new();
static SNAPSHOT_PRESENT: AtomicBool = AtomicBool::new(false);
static REPUBLISHED_COUNT: AtomicU64 = AtomicU64::new(0);
static UNCHANGED_SKIP_COUNT: AtomicU64 = AtomicU64::new(0);
static OBSERVATION_REFUSED_COUNT: AtomicU64 = AtomicU64::new(0);
static PUBLICATION_UNAVAILABLE_COUNT: AtomicU64 = AtomicU64::new(0);
static ENGINE_ORIGINAL_SKIPPED_COUNT: AtomicU64 = AtomicU64::new(0);
static LIFECYCLE: Mutex<()> = Mutex::new(());
static HOOK_RECEIPTS: Mutex<HookReceiptState> = Mutex::new(HookReceiptState::EMPTY);

/// once installation is observed enabled, and is exchanged for a release
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

pub(crate) struct ProducerAdmission {
    closed: AtomicBool,
    active: AtomicUsize,
    post_close_refused: AtomicU64,
    refused_reported_through: AtomicU64,
}

pub(crate) struct AdmissionGuard<'a>(&'a ProducerAdmission);

impl Drop for AdmissionGuard<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ProducerAdmission {
    pub(crate) const fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            post_close_refused: AtomicU64::new(0),
            refused_reported_through: AtomicU64::new(0),
        }
    }

    pub(crate) fn enter(&self) -> (AdmissionGuard<'_>, bool) {
        self.active.fetch_add(1, Ordering::AcqRel);
        let guard = AdmissionGuard(self);
        if self.closed.load(Ordering::Acquire) {
            self.post_close_refused.fetch_add(1, Ordering::AcqRel);
            return (guard, false);
        }
        (guard, true)
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub(crate) fn reopen(&self) {
        self.closed.store(false, Ordering::Release);
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) fn post_close_refused_total(&self) -> u64 {
        self.post_close_refused.load(Ordering::Acquire)
    }

    /// window that lost nothing reports a discard and `admit_hook_release`
    /// the second is a consequence of process-lifetime state living in an
    pub(crate) fn take_post_close_refused(&self) -> u64 {
        let total = self.post_close_refused.load(Ordering::Acquire);
        let reported = self.refused_reported_through.swap(total, Ordering::AcqRel);
        total.saturating_sub(reported)
    }

    pub(crate) fn quiesce(&self, stable: usize) {
        let mut consecutive = 0;
        while consecutive < stable {
            if self.active.load(Ordering::Acquire) == 0 {
                consecutive += 1;
            } else {
                consecutive = 0;
            }
            std::thread::yield_now();
        }
    }
}

#[cfg(test)]
mod producer_admission_tests {
    use super::{ProducerAdmission, PRODUCER_QUIESCE_STABLE_READS};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn a_callback_admitted_after_close_is_refused_and_counted() {
        let gate = Arc::new(ProducerAdmission::new());

        let (early, admitted) = gate.enter();
        assert!(
            admitted,
            "an entrant before the close must publish normally"
        );
        assert_eq!(gate.active(), 1);
        drop(early);

        let parked = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let worker = {
            let gate = Arc::clone(&gate);
            let parked = Arc::clone(&parked);
            let release = Arc::clone(&release);
            std::thread::spawn(move || {
                let (guard, admitted) = gate.enter();
                parked.store(true, Ordering::Release);
                while !release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                drop(guard);
                admitted
            })
        };
        while !parked.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        assert_eq!(gate.active(), 1, "the worker is demonstrably inside");

        gate.close();
        assert!(gate.is_closed());

        let (late, admitted) = gate.enter();
        assert!(!admitted, "an entrant after the close must not publish");
        drop(late);
        assert_eq!(
            gate.post_close_refused_total(),
            1,
            "a refused late observation must be counted, not silent"
        );

        release.store(true, Ordering::Release);
        assert!(
            worker.join().expect("worker joins"),
            "a callback admitted before the close keeps its admission"
        );
        assert_eq!(gate.post_close_refused_total(), 1);
    }

    #[test]
    fn quiesce_waits_for_an_occupant_and_needs_consecutive_zeroes() {
        let gate = Arc::new(ProducerAdmission::new());
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let left = Arc::new(AtomicBool::new(false));

        let worker = {
            let gate = Arc::clone(&gate);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            let left = Arc::clone(&left);
            std::thread::spawn(move || {
                let (guard, _) = gate.enter();
                entered.store(true, Ordering::Release);
                while !release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                drop(guard);
                left.store(true, Ordering::Release);
            })
        };
        while !entered.load(Ordering::Acquire) {
            std::thread::yield_now();
        }

        let releaser = {
            let release = Arc::clone(&release);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(60));
                release.store(true, Ordering::Release);
            })
        };

        gate.quiesce(PRODUCER_QUIESCE_STABLE_READS);
        assert!(
            left.load(Ordering::Acquire),
            "quiesce returned while an invocation was still inside"
        );
        assert_eq!(gate.active(), 0);

        worker.join().expect("worker joins");
        releaser.join().expect("releaser joins");

        assert!(PRODUCER_QUIESCE_STABLE_READS > 1);
        gate.quiesce(PRODUCER_QUIESCE_STABLE_READS);
    }

    #[test]
    fn a_refusal_that_lands_after_the_sample_survives_to_the_next_take() {
        let gate = Arc::new(ProducerAdmission::new());
        let at_threshold = Arc::new(AtomicBool::new(false));
        let proceed = Arc::new(AtomicBool::new(false));

        let worker = {
            let gate = Arc::clone(&gate);
            let at_threshold = Arc::clone(&at_threshold);
            let proceed = Arc::clone(&proceed);
            std::thread::spawn(move || {
                at_threshold.store(true, Ordering::Release);
                while !proceed.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                let (guard, admitted) = gate.enter();
                drop(guard);
                admitted
            })
        };
        while !at_threshold.load(Ordering::Acquire) {
            std::thread::yield_now();
        }

        gate.close();
        gate.quiesce(PRODUCER_QUIESCE_STABLE_READS);
        let sampled_by_this_capture = gate.take_post_close_refused();

        assert_eq!(
            sampled_by_this_capture, 0,
            "the sample still cannot see a callback that has not entered yet"
        );

        gate.reopen();

        proceed.store(true, Ordering::Release);
        let admitted = worker.join().expect("worker joins");
        assert!(
            admitted,
            "the gate is open again, so the late entrant is admitted -- which \
             is why the count, not the gate, is what has to preserve it"
        );

        gate.close();
        let (late, admitted) = gate.enter();
        assert!(!admitted);
        drop(late);
        assert_eq!(
            gate.post_close_refused_total(),
            1,
            "the cumulative counter never resets across a capture boundary"
        );
        assert_eq!(
            gate.take_post_close_refused(),
            1,
            "a refusal that survived a reopen is reported by the next take"
        );
        assert_eq!(
            gate.take_post_close_refused(),
            0,
            "and reported exactly once: the take is consuming"
        );
    }

    #[test]
    fn reopen_neither_recharges_nor_erases_the_previous_captures_refusals() {
        let gate = ProducerAdmission::new();
        gate.close();
        let (guard, admitted) = gate.enter();
        assert!(!admitted);
        drop(guard);
        assert_eq!(gate.post_close_refused_total(), 1);

        assert_eq!(gate.take_post_close_refused(), 1);

        gate.close();
        let (late, admitted) = gate.enter();
        assert!(!admitted);
        drop(late);

        gate.reopen();
        assert!(!gate.is_closed());
        assert_eq!(
            gate.post_close_refused_total(),
            2,
            "reopen must not erase a refusal it never reported"
        );
        assert_eq!(
            gate.take_post_close_refused(),
            1,
            "exactly the one refusal nobody has reported yet, not both"
        );

        let (guard, admitted) = gate.enter();
        assert!(admitted);
        drop(guard);
    }
}

pub(crate) const PRODUCER_QUIESCE_STABLE_READS: usize = 8;

static ADMISSION: ProducerAdmission = ProducerAdmission::new();

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn invocation_counts() -> PovVisibilityInvocationCounts {
    PovVisibilityInvocationCounts {
        republished: REPUBLISHED_COUNT.load(Ordering::Acquire),
        unchanged_not_recaptured: UNCHANGED_SKIP_COUNT.load(Ordering::Acquire),
        observation_refused: OBSERVATION_REFUSED_COUNT.load(Ordering::Acquire),
        publication_unavailable: PUBLICATION_UNAVAILABLE_COUNT.load(Ordering::Acquire),
    }
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    let receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| PovVisibilityHookError::ReceiptCustodyPoisoned.to_string())?;
    receipts
        .installed
        .map(|installed| [installed])
        .ok_or_else(|| PovVisibilityHookError::InstalledReceiptUnavailable.to_string())
}

/// Consume the release receipt. The take is one-shot: a second call after the
/// minting a second release.
pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String>
{
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| PovVisibilityHookError::ReceiptCustodyPoisoned.to_string())?;
    let released = receipts
        .released
        .ok_or_else(|| PovVisibilityHookError::ReleasedReceiptUnavailable.to_string())?;
    receipts.released = None;
    receipts.removed_by_custody = false;
    Ok([released])
}

pub(crate) trait HookOps {
    fn create(&self, target: usize) -> Result<usize, MH_STATUS>;
    fn enable(&self, target: usize) -> Result<(), InlineDetourEnableError>;
    fn disable(&self, target: usize) -> Result<(), InlineDetourReleaseError>;
    fn remove_and_free(&self, target: usize) -> Result<(), InlineDetourReleaseError>;
}

pub(crate) struct MinHookOps;

impl HookOps for MinHookOps {
    fn create(&self, target: usize) -> Result<usize, MH_STATUS> {
        unsafe { MinHook::create_hook(target as *mut c_void, hook_producer as *mut c_void) }
            .map(|original| original as usize)
    }

    fn enable(&self, target: usize) -> Result<(), InlineDetourEnableError> {
        unsafe { inline_detour::enable(target as *mut c_void) }
    }

    fn disable(&self, target: usize) -> Result<(), InlineDetourReleaseError> {
        unsafe { inline_detour::disable(target as *mut c_void) }
    }

    fn remove_and_free(&self, target: usize) -> Result<(), InlineDetourReleaseError> {
        // NEVER-CALLED: and deliberately so -- see the trait declaration above.
        // This lane RETAINS its trampoline for the life of the process;
        // `uninstall_with_ops` closes admission, `disable`s (which frees
        // nothing) and quiesces, and never reaches this. The operation is named
        // and implemented precisely so that "the trampoline is never freed"
        // becomes assertable: the teardown tests read back the performed
        // sequence and FAIL if this ever appears in it.
        //
        // So there is no caller to drain, and a `DRAINED-BY-CALLER:` marker
        // here would have named one that does not exist -- worse than no marker
        // at all, because it reads as audited.
        unsafe { inline_detour::remove(target as *mut c_void) }
    }
}

pub fn install() -> Result<(), PovVisibilityHookError> {
    let _guard = LIFECYCLE
        .lock()
        .map_err(|_| PovVisibilityHookError::LifecyclePoisoned)?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err(PovVisibilityHookError::ModuleUnavailable);
    }
    let target = base
        .checked_add(PRODUCER_RVA as usize)
        .ok_or(PovVisibilityHookError::ProducerAddressOverflow)?;
    let mut actual = [0u8; PRODUCER_SENTINEL.len()];
    read_exact(target, &mut actual).map_err(PovVisibilityHookError::ProducerReadFailed)?;
    if actual != PRODUCER_SENTINEL {
        return Err(PovVisibilityHookError::ProducerSentinelMismatch);
    }
    install_at_with_ops(base, target, &MinHookOps)
}

fn install_at_with_ops(
    base: usize,
    target: usize,
    ops: &dyn HookOps,
) -> Result<(), PovVisibilityHookError> {
    install_at_with_ops_and_slot(base, target, ops, clear_publication_slot)
}

fn clear_publication_slot() -> Result<(), PovVisibilityHookError> {
    let latest = LATEST.get_or_init(|| Mutex::new(None));
    match latest.lock() {
        Ok(mut latest) => {
            *latest = None;
            SNAPSHOT_PRESENT.store(false, Ordering::Release);
            Ok(())
        }
        Err(_) => Err(PovVisibilityHookError::LatestSnapshotPoisoned),
    }
}

fn install_at_with_ops_and_slot(
    base: usize,
    target: usize,
    ops: &dyn HookOps,
    clear_slot: impl FnOnce() -> Result<(), PovVisibilityHookError>,
) -> Result<(), PovVisibilityHookError> {
    clear_slot()?;
    let original = match ops.create(target) {
        Ok(original) => original,
        Err(MH_STATUS::MH_ERROR_ALREADY_CREATED) => {
            let retained = ORIGINAL.load(Ordering::Acquire);
            if retained == 0 {
                return Err(PovVisibilityHookError::HookCreationFailed(
                    "producer site is already created but no retained trampoline address is held"
                        .into(),
                ));
            }
            retained
        }
        Err(status) => {
            return Err(PovVisibilityHookError::HookCreationFailed(
                status.to_string(),
            ))
        }
    };
    ORIGINAL.store(original, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    if let Err(status) = ops.enable(target) {
        retain_and_disable(target, ops);
        return Err(PovVisibilityHookError::HookEnableFailed(status.to_string()));
    }
    if let Err(error) = observe_install_receipt(target) {
        retain_and_disable(target, ops);
        return Err(error);
    }
    ADMISSION.reopen();
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

/// result and then clear module ownership, so a removal that failed left
/// asking only `installed()` would then release the lane, see success, and
static FAILED_INSTALL_REMOVAL_UNPROVED: AtomicBool = AtomicBool::new(false);

pub fn take_post_close_refused_observations() -> u64 {
    ADMISSION.take_post_close_refused()
}

pub fn post_close_refused_observations_total() -> u64 {
    ADMISSION.post_close_refused_total()
}

pub fn engine_original_skipped_invocations() -> u64 {
    ENGINE_ORIGINAL_SKIPPED_COUNT.load(Ordering::Acquire)
}

/// is nothing to release, and its teardown consults it beside `installed()` --
pub fn failed_install_removal_unproved() -> bool {
    FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire)
}

/// step: it restores the original target bytes -- which is what the release
/// its own detour-body ranges as "retained for process lifetime" against the
fn retain_and_disable(target: usize, ops: &dyn HookOps) {
    if ops.disable(target).is_err() {
        FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
        return;
    }
    MODULE_BASE.store(0, Ordering::Release);
}

fn observe_install_receipt(
    target: usize,
) -> Result<InstalledRvaHookReceipt, PovVisibilityHookError> {
    {
        let receipts = HOOK_RECEIPTS
            .lock()
            .map_err(|_| PovVisibilityHookError::ReceiptCustodyPoisoned)?;
        if receipts.released.is_some() {
            return Err(PovVisibilityHookError::ReceiptAlreadyReleased);
        }
        if let Some(installed) = receipts.installed {
            return Ok(installed);
        }
    }
    let token = next_teardown_owner_token().map_err(PovVisibilityHookError::ReceiptRefused)?;
    let observed = observe_enabled_minhook_rel5(
        EXPECTED_HOOK,
        target,
        hook_producer as *const () as usize,
        token,
    )
    .map_err(PovVisibilityHookError::ReceiptRefused)?;
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| PovVisibilityHookError::ReceiptCustodyPoisoned)?;
    if receipts.released.is_some() {
        return Err(PovVisibilityHookError::ReceiptAlreadyReleased);
    }
    match receipts.installed {
        Some(existing) if existing == observed => Ok(existing),
        Some(_) => Err(PovVisibilityHookError::ReceiptChangedDuringObservation),
        None => {
            receipts.installed = Some(observed);
            Ok(observed)
        }
    }
}

fn observe_release_receipt() -> Result<ReleasedRvaHookReceipt, PovVisibilityHookError> {
    let installed = {
        let receipts = HOOK_RECEIPTS
            .lock()
            .map_err(|_| PovVisibilityHookError::ReceiptCustodyPoisoned)?;
        if let Some(released) = receipts.released {
            return Ok(released);
        }
        if !receipts.removed_by_custody {
            return Err(PovVisibilityHookError::ReleaseRefused(
                "release attestation lacks owner-performed hook removal".into(),
            ));
        }
        receipts
            .installed
            .ok_or(PovVisibilityHookError::InstalledReceiptUnavailable)?
    };
    // `observe_released_with_retained_relay` re-reads from process memory
    let attestation = HookTeardownAttestation {
        callbacks_quiesced: true,
        hook_removed: true,
        relay_freed: false,
    };
    let released = observe_released_with_retained_relay(EXPECTED_HOOK, installed, attestation)
        .map_err(PovVisibilityHookError::ReleaseRefused)?;
    let mut receipts = HOOK_RECEIPTS
        .lock()
        .map_err(|_| PovVisibilityHookError::ReceiptCustodyPoisoned)?;
    match receipts.released {
        Some(existing) if existing == released => Ok(existing),
        Some(_) => Err(PovVisibilityHookError::ReceiptChangedDuringObservation),
        None if receipts.installed == Some(installed) => {
            receipts.released = Some(released);
            receipts.installed = None;
            Ok(released)
        }
        None => Err(PovVisibilityHookError::ReceiptChangedDuringObservation),
    }
}

pub fn drain_latest() -> Result<Option<PovVisibilitySnapshot>, PovVisibilityDrainError> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err(PovVisibilityDrainError::NotInstalled);
    }
    let latest = LATEST
        .get()
        .ok_or(PovVisibilityDrainError::StateUnavailable)?;
    let shared = latest
        .lock()
        .map_err(|_| PovVisibilityDrainError::LatestSnapshotPoisoned)?
        .clone();
    // Materialised AFTER the lock is released: the producer never waits on
    Ok(shared.map(|snapshot| (*snapshot).clone()))
}

pub fn uninstall() -> Result<(), PovVisibilityHookError> {
    let _guard = LIFECYCLE
        .lock()
        .map_err(|_| PovVisibilityHookError::LifecyclePoisoned)?;
    uninstall_with_ops(&MinHookOps)
}

/// consecutively, and mint the release receipt from the restored target bytes.
fn uninstall_with_ops(ops: &dyn HookOps) -> Result<(), PovVisibilityHookError> {
    let was_installed = INSTALLED.swap(false, Ordering::AcqRel);
    if !was_installed && !FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire) {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
        return Err(PovVisibilityHookError::HookDisableFailed(
            "D94 teardown has no module base to remove its detour from".into(),
        ));
    }
    let target = base
        .checked_add(PRODUCER_RVA as usize)
        .ok_or(PovVisibilityHookError::ProducerAddressOverflow)?;
    let mut errors = Vec::new();
    ADMISSION.close();
    match ops.disable(target) {
        Ok(())
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {
            match HOOK_RECEIPTS.lock() {
                // MinHook's bookkeeping -- is what the release receipt verifies,
                Ok(mut receipts) => receipts.removed_by_custody = true,
                Err(_) => errors.push(PovVisibilityHookError::ReceiptCustodyPoisoned),
            }
            MODULE_BASE.store(0, Ordering::Release);
            FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);
        }
        Err(status) => {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            errors.push(PovVisibilityHookError::HookDisableFailed(
                status.to_string(),
            ))
        }
    }
    ADMISSION.quiesce(PRODUCER_QUIESCE_STABLE_READS);
    if errors.is_empty() {
        // The restored original prefix is what mints the release receipt, so
        observe_release_receipt()?;
        Ok(())
    } else {
        Err(errors.remove(0))
    }
}

static REBUILD_STATE: AtomicU64 = AtomicU64::new(0);

const REBUILD_ACTIVE_MASK: u64 = 0xFFFF_FFFF;
const REBUILD_GENERATION_STEP: u64 = 1 << 32;

struct RebuildWindow {
    closed: bool,
}

impl RebuildWindow {
    fn open() -> Self {
        REBUILD_STATE.fetch_add(REBUILD_GENERATION_STEP | 1, Ordering::AcqRel);
        Self { closed: false }
    }

    fn close(mut self) {
        self.closed = true;
        REBUILD_STATE.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for RebuildWindow {
    fn drop(&mut self) {
        if !self.closed {
            REBUILD_STATE.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

fn run_producer_invocation<RunOriginal>(
    intel: usize,
    gamestate: usize,
    run_original: RunOriginal,
) -> PovVisibilityInvocationOutcome
where
    RunOriginal: FnOnce(),
{
    run_producer_invocation_from(
        REBUILD_STATE.load(Ordering::Acquire),
        intel,
        gamestate,
        run_original,
    )
}

fn run_producer_invocation_from<RunOriginal>(
    rebuild_state_at_entry: u64,
    intel: usize,
    gamestate: usize,
    run_original: RunOriginal,
) -> PovVisibilityInvocationOutcome
where
    RunOriginal: FnOnce(),
{
    let before = read_dirty_probe(intel);
    let pov_input = read_pov_input(gamestate);
    let hour_raw = read_producer_hour(gamestate);
    let window = RebuildWindow::open();
    run_original();
    window.close();
    publish_capture_if_producer_rebuilt(
        intel,
        before,
        pov_input,
        ProducerHourWitness {
            hour_raw,
            rebuild_state_at_entry,
        },
    )
}

unsafe extern "system" fn hook_producer(intel: usize, gamestate: usize) {
    let (_lease, admitted) = ADMISSION.enter();
    let original_addr = ORIGINAL.load(Ordering::Acquire);
    if original_addr == 0 {
        ENGINE_ORIGINAL_SKIPPED_COUNT.fetch_add(1, Ordering::AcqRel);
        return;
    }
    if !admitted {
        let original: ProducerFn = mem::transmute(original_addr);
        original(intel, gamestate);
        return;
    }
    let original: ProducerFn = mem::transmute(original_addr);
    run_producer_invocation(intel, gamestate, || original(intel, gamestate));
}

fn read_producer_hour(gamestate: usize) -> Option<i32> {
    read_u32(gamestate.checked_add(GAMESTATE_CURRENT_DATE_OFFSET)?)
        .ok()
        .map(|word| word as i32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProducerHourWitness {
    hour_raw: Option<i32>,
    rebuild_state_at_entry: u64,
}

impl ProducerHourWitness {
    #[cfg(test)]
    const UNKNOWN: Self = Self {
        hour_raw: None,
        rebuild_state_at_entry: 0,
    };

    fn hour_for(self, rebuild_state_at_copy: u64) -> Option<i32> {
        if rebuild_state_at_copy
            == self
                .rebuild_state_at_entry
                .wrapping_add(REBUILD_GENERATION_STEP)
        {
            self.hour_raw
        } else {
            None
        }
    }
}

fn publish_capture_if_producer_rebuilt(
    intel: usize,
    before: Option<DirtyProbe>,
    pov_input: Option<u32>,
    witness: ProducerHourWitness,
) -> PovVisibilityInvocationOutcome {
    if !producer_rebuilt(intel, before, pov_input) {
        UNCHANGED_SKIP_COUNT.fetch_add(1, Ordering::AcqRel);
        return PovVisibilityInvocationOutcome::UnchangedNotRecaptured;
    }
    publish_capture(intel, witness)
}

fn producer_rebuilt(intel: usize, before: Option<DirtyProbe>, pov_input: Option<u32>) -> bool {
    if !SNAPSHOT_PRESENT.load(Ordering::Acquire) {
        return true;
    }
    let Some(before) = before else {
        return true;
    };
    if before.dirty {
        return true;
    }
    match pov_input {
        None => return true,
        Some(pov_input) if pov_input != before.pov_country_id => return true,
        Some(_) => {}
    }
    let Some(after) = read_dirty_probe(intel) else {
        return true;
    };
    before.pov_country_id != after.pov_country_id || before.toggle_epoch != after.toggle_epoch
}

fn read_pov_input(gamestate: usize) -> Option<u32> {
    let mut probe = [0u8; 8];
    read_exact(
        gamestate.checked_add(GAMESTATE_POV_PRIMARY_OFFSET)?,
        &mut probe,
    )
    .ok()?;
    let primary = u32::from_le_bytes([probe[0], probe[1], probe[2], probe[3]]);
    Some(if (primary as i32) > 0 {
        primary
    } else {
        u32::from_le_bytes([probe[4], probe[5], probe[6], probe[7]])
    })
}

fn read_dirty_probe(intel: usize) -> Option<DirtyProbe> {
    let mut probe = [0u8; 8];
    read_exact(intel.checked_add(POV_COUNTRY_ID_OFFSET)?, &mut probe).ok()?;
    Some(DirtyProbe {
        pov_country_id: u32::from_le_bytes([probe[0], probe[1], probe[2], probe[3]]),
        dirty: probe[DIRTY_BYTE_OFFSET - POV_COUNTRY_ID_OFFSET] != 0,
        toggle_epoch: probe[TOGGLE_EPOCH_BYTE_OFFSET - POV_COUNTRY_ID_OFFSET],
    })
}

fn publish_capture(intel: usize, witness: ProducerHourWitness) -> PovVisibilityInvocationOutcome {
    let snapshot = capture_atomic(intel, witness).map(std::sync::Arc::new);
    let refused = snapshot.is_none();
    let Some(latest) = LATEST.get() else {
        PUBLICATION_UNAVAILABLE_COUNT.fetch_add(1, Ordering::AcqRel);
        return PovVisibilityInvocationOutcome::PublicationUnavailable;
    };
    let Ok(mut slot) = latest.lock() else {
        PUBLICATION_UNAVAILABLE_COUNT.fetch_add(1, Ordering::AcqRel);
        return PovVisibilityInvocationOutcome::PublicationUnavailable;
    };
    let outgoing = std::mem::replace(&mut *slot, snapshot);
    SNAPSHOT_PRESENT.store(!refused, Ordering::Release);
    drop(slot);
    drop(outgoing);
    if refused {
        OBSERVATION_REFUSED_COUNT.fetch_add(1, Ordering::AcqRel);
        PovVisibilityInvocationOutcome::ObservationRefused
    } else {
        REPUBLISHED_COUNT.fetch_add(1, Ordering::AcqRel);
        PovVisibilityInvocationOutcome::Republished
    }
}

#[cfg(test)]
static BRACKETED_CAPTURES: AtomicU64 = AtomicU64::new(0);

fn capture_atomic(intel: usize, witness: ProducerHourWitness) -> Option<PovVisibilitySnapshot> {
    capture_atomic_with(intel, witness, read_exact)
}

/// `REBUILD_EPOCH` reads, every bound check, and the two comparisons that
fn capture_atomic_with<ReadBody>(
    intel: usize,
    witness: ProducerHourWitness,
    mut read_body: ReadBody,
) -> Option<PovVisibilitySnapshot>
where
    ReadBody: FnMut(usize, &mut [u8]) -> Result<(), String>,
{
    #[cfg(test)]
    BRACKETED_CAPTURES.fetch_add(1, Ordering::AcqRel);
    // calls, which the compiler cannot hoist or sink across, so `Acquire` is
    // the closing load: a trailing `Acquire` alone does not hold earlier plain
    let rebuild_before = REBUILD_STATE.load(Ordering::Acquire);
    if rebuild_before & REBUILD_ACTIVE_MASK != 0 {
        return None;
    }
    let before = read_header(intel)?;
    let count = usize::try_from(before.count).ok()?;
    if count > MAX_PROVINCE_SLOTS {
        compact_contract::bound_trips::record_bound_trip(
            compact_contract::bound_trips::BOUND_TRIP_MAP_PROVINCE_SLOTS,
            count as u64,
        );
        return None;
    }
    if before.current == 0
        || before.previous == 0
        || count == 0
        || before.pov_country_id == 0
        || before.pov_country_id > MAX_POV_COUNTRY_ID
    {
        return None;
    }
    let mut province_levels = vec![0u8; count];
    read_body(before.current, &mut province_levels).ok()?;
    let after = read_header(intel)?;
    if before != after {
        return None;
    }
    if REBUILD_STATE.load(Ordering::Acquire) != rebuild_before {
        return None;
    }
    let capture_sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if capture_sequence == 0 {
        return None;
    }
    Some(PovVisibilitySnapshot {
        capture_sequence,
        pov_country_id: before.pov_country_id,
        producer_hour_raw: witness.hour_for(rebuild_before),
        province_levels,
    })
}

fn read_header(base: usize) -> Option<Header> {
    Some(Header {
        current: read_usize(base.checked_add(CURRENT_BUFFER_OFFSET)?).ok()?,
        previous: read_usize(base.checked_add(PREVIOUS_BUFFER_OFFSET)?).ok()?,
        count: read_u32(base.checked_add(PROVINCE_COUNT_OFFSET)?).ok()?,
        pov_country_id: read_u32(base.checked_add(POV_COUNTRY_ID_OFFSET)?).ok()?,
        flags: read_u32(base.checked_add(FLAGS_OFFSET)?).ok()?,
    })
}

fn read_usize(address: usize) -> Result<usize, String> {
    let mut bytes = [0u8; mem::size_of::<usize>()];
    read_exact(address, &mut bytes)?;
    Ok(usize::from_le_bytes(bytes))
}
fn read_u32(address: usize) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_exact(address: usize, out: &mut [u8]) -> Result<(), String> {
    if address == 0 {
        return Err("visibility read from null".into());
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
    if ok == 0 || read != out.len() {
        Err(format!("visibility read failed at {address:#x}"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    const FLAGS_DIRTY_SET: u32 = 0x0000_0001;

    struct IntelFixture {
        block: Vec<u8>,
        current: Vec<u8>,
        previous: Vec<u8>,
    }

    impl IntelFixture {
        fn new(province_levels: Vec<u8>, pov_country_id: u32, flags: u32) -> Self {
            let count = province_levels.len() as u32;
            let previous = vec![0u8; province_levels.len().max(1)];
            let mut fixture = Self {
                block: vec![0u8; 0x20],
                current: province_levels,
                previous,
            };
            let current_ptr = fixture.current.as_mut_ptr() as usize as u64;
            let previous_ptr = fixture.previous.as_mut_ptr() as usize as u64;
            fixture.write_u64(CURRENT_BUFFER_OFFSET, current_ptr);
            fixture.write_u64(PREVIOUS_BUFFER_OFFSET, previous_ptr);
            fixture.write_u32(PROVINCE_COUNT_OFFSET, count);
            fixture.write_u32(POV_COUNTRY_ID_OFFSET, pov_country_id);
            fixture.write_u32(FLAGS_OFFSET, flags);
            fixture
        }

        fn write_u64(&mut self, offset: usize, value: u64) {
            self.block[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }

        fn write_u32(&mut self, offset: usize, value: u32) {
            self.block[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn set_current_pointer(&mut self, value: u64) {
            self.write_u64(CURRENT_BUFFER_OFFSET, value);
        }

        fn set_previous_pointer(&mut self, value: u64) {
            self.write_u64(PREVIOUS_BUFFER_OFFSET, value);
        }

        fn set_count(&mut self, value: u32) {
            self.write_u32(PROVINCE_COUNT_OFFSET, value);
        }

        fn set_pov_country_id(&mut self, value: u32) {
            self.write_u32(POV_COUNTRY_ID_OFFSET, value);
        }

        fn set_flags(&mut self, value: u32) {
            self.write_u32(FLAGS_OFFSET, value);
        }

        fn overwrite_current(&mut self, value: u8) {
            self.current.fill(value);
        }

        fn address(&self) -> usize {
            self.block.as_ptr() as usize
        }

        fn address_mut(&mut self) -> usize {
            self.block.as_mut_ptr() as usize
        }
    }

    const FIXTURE_GAME_HOUR_RAW: i32 = 60_832_312;

    struct GameStateFixture {
        block: Vec<u8>,
    }

    impl GameStateFixture {
        fn new(pov_country_id: u32) -> Self {
            let mut fixture = Self {
                block: vec![0u8; GAMESTATE_POV_FALLBACK_OFFSET + 8],
            };
            fixture.set_pov_country_id(pov_country_id);
            fixture.set_current_date_raw(FIXTURE_GAME_HOUR_RAW);
            fixture
        }

        fn set_pov_country_id(&mut self, value: u32) {
            let at = GAMESTATE_POV_PRIMARY_OFFSET;
            self.block[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn set_current_date_raw(&mut self, value: i32) {
            let at = GAMESTATE_CURRENT_DATE_OFFSET;
            self.block[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn address(&self) -> usize {
            self.block.as_ptr() as usize
        }
    }

    fn entry_witness(hour_raw: Option<i32>) -> ProducerHourWitness {
        ProducerHourWitness {
            hour_raw,
            rebuild_state_at_entry: REBUILD_STATE.load(Ordering::Acquire),
        }
    }

    fn uncontested_witness(hour_raw: Option<i32>) -> ProducerHourWitness {
        let witness = entry_witness(hour_raw);
        RebuildWindow::open().close();
        witness
    }

    fn run_detour_body(
        fixture: &mut IntelFixture,
        gamestate: &GameStateFixture,
        emulated_original: impl FnOnce(&mut IntelFixture),
    ) -> PovVisibilityInvocationOutcome {
        let intel = fixture.address();
        let gamestate = gamestate.address();
        run_producer_invocation(intel, gamestate, || emulated_original(fixture))
    }

    fn emulate_rebuild(fixture: &mut IntelFixture, province_value: u8) {
        fixture.overwrite_current(province_value);
        fixture.set_flags(0);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum RecordedHookOp {
        Create,
        Enable,
        Disable,
        RemoveAndFree,
    }

    struct RecordingHookOps {
        performed: Mutex<Vec<RecordedHookOp>>,
        create: Result<usize, MH_STATUS>,
        enable: Result<(), InlineDetourEnableError>,
        disable: Result<(), InlineDetourReleaseError>,
        remove: Result<(), InlineDetourReleaseError>,
    }

    impl RecordingHookOps {
        fn succeeding(trampoline: usize) -> Self {
            Self {
                performed: Mutex::new(Vec::new()),
                create: Ok(trampoline),
                enable: Ok(()),
                disable: Ok(()),
                remove: Ok(()),
            }
        }

        fn performed(&self) -> Vec<RecordedHookOp> {
            self.performed
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }

        fn record(&self, op: RecordedHookOp) {
            self.performed
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(op);
        }
    }

    impl HookOps for RecordingHookOps {
        fn create(&self, _target: usize) -> Result<usize, MH_STATUS> {
            self.record(RecordedHookOp::Create);
            self.create
        }

        fn enable(&self, _target: usize) -> Result<(), InlineDetourEnableError> {
            self.record(RecordedHookOp::Enable);
            self.enable
        }

        fn disable(&self, _target: usize) -> Result<(), InlineDetourReleaseError> {
            self.record(RecordedHookOp::Disable);
            self.disable
        }

        fn remove_and_free(&self, _target: usize) -> Result<(), InlineDetourReleaseError> {
            self.record(RecordedHookOp::RemoveAndFree);
            self.remove
        }
    }

    static ENGINE_ORIGINAL_CALLS: AtomicU64 = AtomicU64::new(0);

    unsafe extern "system" fn counting_original(_intel: usize, _gamestate: usize) {
        ENGINE_ORIGINAL_CALLS.fetch_add(1, Ordering::AcqRel);
    }

    fn reset_lane_state() {
        INSTALLED.store(false, Ordering::Release);
        ADMISSION.reopen();
        MODULE_BASE.store(0, Ordering::Release);
        if let Some(latest) = LATEST.get() {
            if let Ok(mut slot) = latest.lock() {
                *slot = None;
            }
        }
        SNAPSHOT_PRESENT.store(false, Ordering::Release);
        if let Ok(mut receipts) = HOOK_RECEIPTS.lock() {
            *receipts = HookReceiptState::EMPTY;
        }
    }

    fn arm_drain_path() {
        LATEST.get_or_init(|| Mutex::new(None));
        INSTALLED.store(true, Ordering::Release);
    }

    #[test]
    fn header_reader_binds_every_field_to_its_own_offset() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut block = vec![0u8; 0x20];
        block[0x00..0x08].copy_from_slice(&0x1111_1111_1111_1111u64.to_le_bytes());
        block[0x08..0x10].copy_from_slice(&0x2222_2222_2222_2222u64.to_le_bytes());
        block[0x10..0x14].copy_from_slice(&0x3333_3333u32.to_le_bytes());
        block[0x14..0x18].copy_from_slice(&0x4444_4444u32.to_le_bytes());
        block[0x18..0x1C].copy_from_slice(&0x5555_5555u32.to_le_bytes());
        block[0x1C..0x20].copy_from_slice(&0x6666_6666u32.to_le_bytes());

        let header = read_header(block.as_ptr() as usize).expect("header read");

        assert_eq!(header.current, 0x1111_1111_1111_1111u64 as usize);
        assert_eq!(header.previous, 0x2222_2222_2222_2222u64 as usize);
        assert_eq!(header.count, 0x3333_3333);
        assert_eq!(header.pov_country_id, 0x4444_4444);
        assert_eq!(header.flags, 0x5555_5555);
    }

    #[test]
    fn drain_refuses_with_the_not_installed_error_before_install() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();

        assert_eq!(drain_latest(), Err(PovVisibilityDrainError::NotInstalled));
        assert_eq!(
            PovVisibilityDrainError::NotInstalled.to_string(),
            "visibility producer hook is not installed"
        );
    }

    #[test]
    fn installed_publication_reaches_the_emitter_drain() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let province_levels = vec![0u8, 1, 2, 3, 250, 255, 7, 8];
        let fixture = IntelFixture::new(province_levels.clone(), 41, 0);
        arm_drain_path();

        publish_capture(fixture.address(), ProducerHourWitness::UNKNOWN);
        let drained = drain_latest().expect("drain after publication");
        let snapshot = drained.expect("a complete current-POV frame");

        assert_eq!(snapshot.pov_country_id, 41);
        assert_eq!(snapshot.province_levels, province_levels);
        assert!(snapshot.capture_sequence > 0);

        let payload = snapshot.encode().expect("emitter payload");
        assert_eq!(payload[..4], crate::CARRIER_MAGIC);
        let round_trip = PovVisibilitySnapshot::decode(&payload).expect("decode");
        assert_eq!(round_trip, snapshot);
        assert_eq!(round_trip.province_levels, province_levels);

        reset_lane_state();
    }

    #[test]
    fn a_refused_observation_clears_the_previous_publication() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let good = IntelFixture::new(vec![9u8; 16], 7, 0);
        arm_drain_path();
        publish_capture(good.address(), ProducerHourWitness::UNKNOWN);
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("published frame")
                .province_levels,
            vec![9u8; 16]
        );

        let mut torn = IntelFixture::new(vec![1u8; 16], 7, 0);
        torn.set_current_pointer(0);
        publish_capture(torn.address(), ProducerHourWitness::UNKNOWN);

        assert_eq!(drain_latest(), Ok(None));

        reset_lane_state();
    }

    #[test]
    fn each_accepted_observation_advances_the_capture_sequence() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let fixture = IntelFixture::new(vec![3u8; 4], 12, 0);
        arm_drain_path();

        publish_capture(fixture.address(), ProducerHourWitness::UNKNOWN);
        let first = drain_latest().expect("drain").expect("frame");
        publish_capture(fixture.address(), ProducerHourWitness::UNKNOWN);
        let second = drain_latest().expect("drain").expect("frame");

        assert_eq!(second.capture_sequence, first.capture_sequence + 1);
        assert_eq!(second.province_levels, first.province_levels);
        assert_eq!(second.pov_country_id, 12);

        reset_lane_state();
    }

    #[test]
    fn province_count_bound_admits_the_bound_and_refuses_one_past_it() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let at_bound = IntelFixture::new(vec![1u8; MAX_PROVINCE_SLOTS], 5, 0);
        let captured = capture_atomic(at_bound.address(), ProducerHourWitness::UNKNOWN)
            .expect("bound is admitted");
        assert_eq!(captured.province_levels.len(), MAX_PROVINCE_SLOTS);
        assert_eq!(captured.pov_country_id, 5);

        let mut past_bound = IntelFixture::new(vec![1u8; MAX_PROVINCE_SLOTS], 5, 0);
        past_bound.set_count(MAX_PROVINCE_SLOTS as u32 + 1);
        assert_eq!(
            capture_atomic(past_bound.address(), ProducerHourWitness::UNKNOWN),
            None
        );
    }

    #[test]
    fn over_budget_count_files_under_the_slots_unit_not_the_province_id_unit() {
        use compact_contract::bound_trips as bt;

        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _ = bt::drain_frame_trips();

        let mut past_bound = IntelFixture::new(vec![1u8; MAX_PROVINCE_SLOTS], 5, 0);
        past_bound.set_count(MAX_PROVINCE_SLOTS as u32 + 1);
        assert_eq!(
            capture_atomic(past_bound.address(), ProducerHourWitness::UNKNOWN),
            None
        );

        let drained = bt::drain_frame_trips();
        assert_eq!(
            drained[bt::BOUND_TRIP_MAP_PROVINCE_SLOTS].0,
            1,
            "the count budget refusal must tick the slots unit"
        );
        assert_eq!(
            drained[bt::BOUND_TRIP_MAP_PROVINCE_SLOTS].1,
            MAX_PROVINCE_SLOTS as u64 + 1,
            "the raw sample must carry the COUNT SEEN, which is what sizing the budget needs"
        );
        assert_eq!(
            drained[bt::BOUND_TRIP_MAP_PROVINCE_ID].0,
            0,
            "no province IDENTITY was out of range, so the identity unit must not tick"
        );
    }

    #[test]
    fn pov_country_bound_admits_the_bound_and_refuses_one_past_it() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let at_bound = IntelFixture::new(vec![2u8; 8], MAX_POV_COUNTRY_ID, 0);
        let captured = capture_atomic(at_bound.address(), ProducerHourWitness::UNKNOWN)
            .expect("bound is admitted");
        assert_eq!(captured.pov_country_id, MAX_POV_COUNTRY_ID);
        assert_eq!(captured.province_levels, vec![2u8; 8]);

        let mut past_bound = IntelFixture::new(vec![2u8; 8], MAX_POV_COUNTRY_ID, 0);
        past_bound.set_pov_country_id(MAX_POV_COUNTRY_ID + 1);
        assert_eq!(
            capture_atomic(past_bound.address(), ProducerHourWitness::UNKNOWN),
            None
        );
    }

    #[test]
    fn incomplete_engine_state_is_refused_rather_than_captured() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let complete = IntelFixture::new(vec![4u8; 8], 6, 0);
        assert_eq!(
            capture_atomic(complete.address(), ProducerHourWitness::UNKNOWN)
                .expect("complete state is captured")
                .province_levels,
            vec![4u8; 8]
        );

        let mut unset_pov = IntelFixture::new(vec![4u8; 8], 6, 0);
        unset_pov.set_pov_country_id(0);
        assert_eq!(
            capture_atomic(unset_pov.address(), ProducerHourWitness::UNKNOWN),
            None
        );

        let mut empty_vector = IntelFixture::new(vec![4u8; 8], 6, 0);
        empty_vector.set_count(0);
        assert_eq!(
            capture_atomic(empty_vector.address(), ProducerHourWitness::UNKNOWN),
            None
        );

        let mut null_current = IntelFixture::new(vec![4u8; 8], 6, 0);
        null_current.set_current_pointer(0);
        assert_eq!(
            capture_atomic(null_current.address(), ProducerHourWitness::UNKNOWN),
            None
        );

        let mut null_previous = IntelFixture::new(vec![4u8; 8], 6, 0);
        null_previous.set_previous_pointer(0);
        assert_eq!(
            capture_atomic(null_previous.address(), ProducerHourWitness::UNKNOWN),
            None
        );
    }

    #[test]
    fn production_hook_specification_is_admitted_and_identity_is_fail_closed() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let token = next_teardown_owner_token().expect("teardown owner token");
        let refusal = observe_enabled_minhook_rel5(EXPECTED_HOOK, 0x1000, 0x2000, token)
            .expect_err("no test process is exact-a729 hoi4.exe");
        assert!(
            refusal.starts_with("production hook main module is ")
                && refusal.ends_with(", not hoi4.exe"),
            "expected a module-identity refusal, got {refusal}"
        );

        let unadmitted_spec = ExpectedRvaHook {
            target_patch_len: 6,
            ..EXPECTED_HOOK
        };
        let token = next_teardown_owner_token().expect("teardown owner token");
        assert_eq!(
            observe_enabled_minhook_rel5(unadmitted_spec, 0x1000, 0x2000, token)
                .expect_err("a six-byte patch width is not an admitted specification"),
            "production hook expected specification is not admitted"
        );

        assert_eq!(EXPECTED_HOOK.primitive_id, MINHOOK_NORMAL_REL5_PRIMITIVE_ID);
        assert_eq!(EXPECTED_HOOK.logical_name, PRODUCER_LOGICAL_NAME);
        assert_eq!(EXPECTED_HOOK.original_prefix, &PRODUCER_SENTINEL);
    }

    #[test]
    fn receipt_logical_name_is_the_registered_d94_identifier() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(
            PRODUCER_LOGICAL_NAME,
            "gameplay.d94.pov_province_visibility_producer"
        );
        assert!(PRODUCER_LOGICAL_NAME.starts_with("gameplay.d94."));
    }

    #[test]
    fn the_pinned_passive_ready_roster_carries_this_exact_site() {
        use compact_contract::passive_ready::{
            PinnedHookTargetSpecV1, PINNED_PRODUCTION_HOOK_SPECS_V1,
        };
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let pinned = PINNED_PRODUCTION_HOOK_SPECS_V1
            .iter()
            .find(|spec| spec.logical_name == PRODUCER_LOGICAL_NAME)
            .expect("D94 has a pinned production hook row");
        assert!(!pinned.mandatory);
        match pinned.target {
            PinnedHookTargetSpecV1::Hoi4Rva {
                module_name,
                rva,
                patch_len,
                primitive_id,
            } => {
                assert_eq!(module_name, "hoi4.exe");
                assert_eq!(rva, EXPECTED_HOOK.requested_rva);
                assert_eq!(patch_len, EXPECTED_HOOK.target_patch_len);
                assert_eq!(primitive_id, EXPECTED_HOOK.primitive_id);
            }
            other => panic!("D94 is pinned as {other:?}, not an exact hoi4.exe RVA site"),
        }
    }

    #[test]
    fn carrier_bounds_are_the_published_admitted_values() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(MAX_PROVINCE_SLOTS, 65_535);
        assert_eq!(MAX_POV_COUNTRY_ID, 2_147_483_646);
    }

    #[test]
    fn the_detour_body_publishes_what_it_observed() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let province_levels = vec![11u8, 22, 33, 44];
        let fixture = IntelFixture::new(province_levels.clone(), 63, 0);
        let gamestate = GameStateFixture::new(63);
        arm_drain_path();

        unsafe extern "system" fn inert_original(_intel: usize, _gamestate: usize) {}
        ORIGINAL.store(inert_original as *const () as usize, Ordering::Release);
        unsafe { hook_producer(fixture.address(), gamestate.address()) };

        let snapshot = drain_latest()
            .expect("drain after the detour ran")
            .expect("the detour published a frame");
        assert_eq!(snapshot.province_levels, province_levels);
        assert_eq!(snapshot.pov_country_id, 63);
        assert_eq!(ADMISSION.active(), 0);

        ORIGINAL.store(0, Ordering::Release);
        reset_lane_state();
    }

    #[test]
    fn the_dirty_predicate_is_the_byte_the_producer_tests_at_0x18() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut fixture = IntelFixture::new(vec![1u8; 4], 77, 0);

        let clear = read_dirty_probe(fixture.address()).expect("probe");
        assert!(!clear.dirty);
        assert_eq!(clear.pov_country_id, 77);
        assert_eq!(clear.toggle_epoch, 0);

        fixture.set_flags(0x0000_0001);
        assert!(read_dirty_probe(fixture.address()).expect("probe").dirty);
        assert_eq!(FLAGS_DIRTY_SET, 0x0000_0001);

        for not_the_predicate in [0x0000_0100u32, 0x0001_0000u32] {
            fixture.set_flags(not_the_predicate);
            let probe = read_dirty_probe(fixture.address()).expect("probe");
            assert!(
                !probe.dirty,
                "flags {not_the_predicate:#010x} is not the producer's dirty byte"
            );
            assert_eq!(probe.toggle_epoch, 0);
        }

        fixture.set_flags(0xAB00_0000);
        let probe = read_dirty_probe(fixture.address()).expect("probe");
        assert!(!probe.dirty);
        assert_eq!(probe.toggle_epoch, 0xAB);
    }

    #[test]
    fn an_unchanged_invocation_copies_nothing_and_keeps_the_standing_frame() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        let gamestate = GameStateFixture::new(21);
        assert_eq!(
            run_detour_body(&mut fixture, &gamestate, |_| {}),
            PovVisibilityInvocationOutcome::Republished,
            "the first invocation primes the empty slot"
        );
        let standing = drain_latest().expect("drain").expect("primed frame");
        assert_eq!(standing.province_levels, vec![5u8; 32]);
        let before_counts = invocation_counts();

        let outcome = run_detour_body(&mut fixture, &gamestate, |fixture| {
            fixture.overwrite_current(9)
        });

        assert_eq!(
            outcome,
            PovVisibilityInvocationOutcome::UnchangedNotRecaptured
        );
        let after = drain_latest().expect("drain").expect("the standing frame");
        assert_eq!(after.province_levels, vec![5u8; 32], "no vector was copied");
        assert_eq!(after.capture_sequence, standing.capture_sequence);
        let after_counts = invocation_counts();
        assert_eq!(
            after_counts.unchanged_not_recaptured,
            before_counts.unchanged_not_recaptured + 1
        );
        assert_eq!(after_counts.republished, before_counts.republished);

        reset_lane_state();
    }

    #[test]
    fn a_dirty_byte_set_before_the_call_is_captured() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        let gamestate = GameStateFixture::new(21);
        run_detour_body(&mut fixture, &gamestate, |_| {});
        let standing = drain_latest().expect("drain").expect("primed frame");

        fixture.set_flags(FLAGS_DIRTY_SET);
        let outcome = run_detour_body(&mut fixture, &gamestate, |fixture| {
            emulate_rebuild(fixture, 9)
        });

        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);
        let after = drain_latest().expect("drain").expect("the rebuilt frame");
        assert_eq!(after.province_levels, vec![9u8; 32]);
        assert!(after.capture_sequence > standing.capture_sequence);

        reset_lane_state();
    }

    #[test]
    fn a_pov_change_the_producer_marks_itself_is_captured() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        let mut gamestate = GameStateFixture::new(21);
        run_detour_body(&mut fixture, &gamestate, |_| {});
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("primed")
                .pov_country_id,
            21
        );

        gamestate.set_pov_country_id(34);
        let outcome = run_detour_body(&mut fixture, &gamestate, |fixture| {
            fixture.set_pov_country_id(34);
            fixture.set_flags(FLAGS_DIRTY_SET);
            emulate_rebuild(fixture, 9);
        });

        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);
        let after = drain_latest().expect("drain").expect("the rebuilt frame");
        assert_eq!(after.pov_country_id, 34);
        assert_eq!(after.province_levels, vec![9u8; 32]);

        reset_lane_state();
    }

    #[test]
    fn a_pov_the_producer_re_reads_as_reverted_is_still_captured() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        let mut gamestate = GameStateFixture::new(21);
        run_detour_body(&mut fixture, &gamestate, |_| {});
        drain_latest().expect("drain").expect("primed frame");

        gamestate.set_pov_country_id(34);
        let outcome = run_detour_body(&mut fixture, &gamestate, |fixture| {
            fixture.set_pov_country_id(21);
            fixture.set_flags(FLAGS_DIRTY_SET);
            emulate_rebuild(fixture, 9);
        });

        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);
        let after = drain_latest().expect("drain").expect("the rebuilt frame");
        assert_eq!(after.province_levels, vec![9u8; 32]);
        assert_eq!(after.pov_country_id, 21);

        reset_lane_state();
    }

    #[test]
    fn a_pov_change_seen_only_in_the_producers_output_is_captured() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        let gamestate = GameStateFixture::new(21);
        run_detour_body(&mut fixture, &gamestate, |_| {});
        drain_latest().expect("drain").expect("primed frame");

        let outcome = run_detour_body(&mut fixture, &gamestate, |fixture| {
            fixture.set_pov_country_id(34);
            fixture.set_flags(FLAGS_DIRTY_SET);
            emulate_rebuild(fixture, 9);
        });

        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);
        let after = drain_latest().expect("drain").expect("the rebuilt frame");
        assert_eq!(after.pov_country_id, 34);
        assert_eq!(after.province_levels, vec![9u8; 32]);

        reset_lane_state();
    }

    #[test]
    fn a_toggle_epoch_change_the_producer_marks_itself_is_captured() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        let gamestate = GameStateFixture::new(21);
        run_detour_body(&mut fixture, &gamestate, |_| {});
        drain_latest().expect("drain").expect("primed frame");

        let outcome = run_detour_body(&mut fixture, &gamestate, |fixture| {
            fixture.set_pov_country_id(21);
            fixture.set_flags(0x7F00_0000 | FLAGS_DIRTY_SET);
            fixture.overwrite_current(9);
            fixture.set_flags(0x7F00_0000);
        });

        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);
        let after = drain_latest().expect("drain").expect("the rebuilt frame");
        assert_eq!(after.pov_country_id, 21);
        assert_eq!(after.province_levels, vec![9u8; 32]);

        reset_lane_state();
    }

    #[test]
    fn the_first_invocation_after_install_publishes_with_a_clear_dirty_byte() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut settled = IntelFixture::new(vec![6u8; 24], 44, 0);
        let gamestate = GameStateFixture::new(44);

        let outcome = run_detour_body(&mut settled, &gamestate, |_| {});

        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("a primed frame")
                .province_levels,
            vec![6u8; 24]
        );

        reset_lane_state();
    }

    #[test]
    fn an_unreadable_predicate_falls_through_to_a_full_capture() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        let gamestate = GameStateFixture::new(21);
        run_detour_body(&mut fixture, &gamestate, |_| {});
        drain_latest().expect("drain").expect("primed frame");
        fixture.overwrite_current(9);

        let outcome = publish_capture_if_producer_rebuilt(
            fixture.address(),
            None,
            Some(21),
            ProducerHourWitness::UNKNOWN,
        );

        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("frame")
                .province_levels,
            vec![9u8; 32]
        );

        fixture.overwrite_current(13);
        let readable = read_dirty_probe(fixture.address());
        assert!(readable.is_some());
        assert_eq!(
            publish_capture_if_producer_rebuilt(
                fixture.address(),
                readable,
                None,
                ProducerHourWitness::UNKNOWN
            ),
            PovVisibilityInvocationOutcome::Republished
        );
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("frame")
                .province_levels,
            vec![13u8; 32]
        );

        assert_eq!(read_dirty_probe(0), None);
        assert_eq!(read_pov_input(0), None);

        reset_lane_state();
    }

    #[test]
    fn a_skipped_invocation_is_not_an_empty_or_absent_measurement() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        let gamestate = GameStateFixture::new(21);
        run_detour_body(&mut fixture, &gamestate, |_| {});

        let skipped = run_detour_body(&mut fixture, &gamestate, |_| {});
        assert_eq!(
            skipped,
            PovVisibilityInvocationOutcome::UnchangedNotRecaptured
        );
        let standing = drain_latest()
            .expect("a skip is not a drain error")
            .expect("a skip is not an absent measurement");
        assert!(
            !standing.province_levels.is_empty(),
            "a skip is not an empty measurement"
        );
        assert_eq!(standing.province_levels, vec![5u8; 32]);

        assert_ne!(skipped, PovVisibilityInvocationOutcome::ObservationRefused);
        let mut torn = IntelFixture::new(vec![1u8; 32], 21, FLAGS_DIRTY_SET);
        torn.set_current_pointer(0);
        assert_eq!(
            run_detour_body(&mut torn, &gamestate, |_| {}),
            PovVisibilityInvocationOutcome::ObservationRefused
        );
        assert_eq!(drain_latest(), Ok(None));

        let mut recovered = IntelFixture::new(vec![8u8; 32], 21, 0);
        assert_eq!(
            run_detour_body(&mut recovered, &gamestate, |_| {}),
            PovVisibilityInvocationOutcome::Republished
        );

        reset_lane_state();
    }

    #[test]
    fn the_installed_detour_skips_the_copy_when_the_producer_rebuilt_nothing() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![0u8; 32], 21, 0);
        let gamestate = GameStateFixture::new(21);

        static REWRITE_VALUE: AtomicUsize = AtomicUsize::new(0x11);
        unsafe extern "system" fn rewriting_original(intel: usize, _gamestate: usize) {
            let current = *(intel as *const usize);
            let count = *((intel + PROVINCE_COUNT_OFFSET) as *const u32) as usize;
            let value = REWRITE_VALUE.fetch_add(1, Ordering::AcqRel) as u8;
            ptr::write_bytes(current as *mut u8, value, count);
        }
        ORIGINAL.store(rewriting_original as *const () as usize, Ordering::Release);

        unsafe { hook_producer(fixture.address_mut(), gamestate.address()) };
        let primed = drain_latest().expect("drain").expect("primed frame");
        assert_eq!(primed.province_levels, vec![0x11u8; 32]);

        unsafe { hook_producer(fixture.address_mut(), gamestate.address()) };
        let standing = drain_latest().expect("drain").expect("the standing frame");
        assert_eq!(
            standing.province_levels,
            vec![0x11u8; 32],
            "the second invocation rebuilt nothing and must copy nothing"
        );
        assert_eq!(standing.capture_sequence, primed.capture_sequence);
        assert_eq!(ADMISSION.active(), 0);

        ORIGINAL.store(0, Ordering::Release);
        reset_lane_state();
    }

    #[test]
    fn the_detour_samples_the_predicate_before_the_original_clears_it() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let mut fixture = IntelFixture::new(vec![0u8; 32], 21, 0);
        let gamestate = GameStateFixture::new(21);

        unsafe extern "system" fn inert_original(_intel: usize, _gamestate: usize) {}
        ORIGINAL.store(inert_original as *const () as usize, Ordering::Release);
        unsafe { hook_producer(fixture.address_mut(), gamestate.address()) };
        let primed = drain_latest().expect("drain").expect("primed frame");
        assert_eq!(primed.province_levels, vec![0u8; 32]);

        unsafe extern "system" fn rebuilding_original(intel: usize, _gamestate: usize) {
            let current = *(intel as *const usize);
            let count = *((intel + PROVINCE_COUNT_OFFSET) as *const u32) as usize;
            ptr::write_bytes(current as *mut u8, 0x5A, count);
            *((intel + FLAGS_OFFSET) as *mut u32) = 0;
        }
        fixture.set_flags(FLAGS_DIRTY_SET);
        ORIGINAL.store(rebuilding_original as *const () as usize, Ordering::Release);
        unsafe { hook_producer(fixture.address_mut(), gamestate.address()) };

        let rebuilt = drain_latest()
            .expect("drain")
            .expect("the rebuilt frame must still be published");
        assert_eq!(
            rebuilt.province_levels,
            vec![0x5Au8; 32],
            "a rebuild announced only by the pre-call dirty byte was dropped"
        );
        assert!(rebuilt.capture_sequence > primed.capture_sequence);

        ORIGINAL.store(0, Ordering::Release);
        reset_lane_state();
    }

    #[test]
    fn a_header_that_changes_under_the_copy_is_refused() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let pinned = IntelFixture::new(vec![3u8; 8], 4, 0);
        let entered = BRACKETED_CAPTURES.load(Ordering::Acquire);
        capture_atomic(pinned.address(), ProducerHourWitness::UNKNOWN)
            .expect("the pinning fixture is admissible");
        assert_eq!(
            BRACKETED_CAPTURES.load(Ordering::Acquire),
            entered + 1,
            "capture_atomic no longer routes through the bracketed helper"
        );

        let mut null_current = IntelFixture::new(vec![1u8; 4], 6, 0);
        null_current.set_current_pointer(0);
        let mut over_bound = IntelFixture::new(vec![1u8; 4], 6, 0);
        over_bound.set_count(MAX_PROVINCE_SLOTS as u32 + 1);
        let mut unset_pov = IntelFixture::new(vec![1u8; 4], 6, 0);
        unset_pov.set_pov_country_id(0);
        for (path, address) in [
            ("admitted", pinned.address()),
            ("null current buffer", null_current.address()),
            ("count past the bound", over_bound.address()),
            ("unset pov", unset_pov.address()),
        ] {
            let via_entry = capture_atomic(address, ProducerHourWitness::UNKNOWN)
                .map(|snapshot| (snapshot.pov_country_id, snapshot.province_levels));
            let via_helper = capture_atomic_with(address, ProducerHourWitness::UNKNOWN, read_exact)
                .map(|snapshot| (snapshot.pov_country_id, snapshot.province_levels));
            assert_eq!(
                via_entry, via_helper,
                "capture_atomic disagreed with the bracketed helper on: {path}"
            );
        }

        let disturbances: [(&str, usize, u64, usize); 5] = [
            ("current", CURRENT_BUFFER_OFFSET, 0xDEAD_BEEF, 8),
            ("previous", PREVIOUS_BUFFER_OFFSET, 0xFEED_FACE, 8),
            ("count", PROVINCE_COUNT_OFFSET, 16, 4),
            ("pov_country_id", POV_COUNTRY_ID_OFFSET, 2, 4),
            ("flags", FLAGS_OFFSET, u64::from(FLAGS_DIRTY_SET), 4),
        ];

        for (field, offset, value, width) in disturbances {
            let mut fixture = IntelFixture::new(vec![7u8; 32], 1, 0);
            let base = fixture.address_mut();

            let undisturbed = capture_atomic(base, ProducerHourWitness::UNKNOWN)
                .unwrap_or_else(|| panic!("undisturbed capture must be accepted ({field})"));
            assert_eq!(undisturbed.province_levels, vec![7u8; 32]);
            assert_eq!(undisturbed.pov_country_id, 1);

            let mut landed = false;
            let torn = capture_atomic_with(base, ProducerHourWitness::UNKNOWN, |address, out| {
                read_exact(address, out)?;
                let bytes = value.to_le_bytes();
                let slot = (base + offset) as *mut u8;
                for (index, byte) in bytes.iter().take(width).enumerate() {
                    unsafe { slot.add(index).write_volatile(*byte) };
                }
                landed = true;
                Ok(())
            });

            assert!(landed, "the injected body read never ran ({field})");
            assert_eq!(
                torn, None,
                "a header whose {field} changed under the copy was accepted"
            );
        }
    }

    #[test]
    fn a_rebuild_that_disturbs_no_header_field_is_refused() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        const SLOTS: usize = 64;
        const HALF: usize = SLOTS / 2;

        let rebuild_body = |body: *mut u8| {
            for index in 0..SLOTS {
                unsafe { body.add(index).write_volatile(0) };
            }
            for index in 0..SLOTS {
                unsafe { body.add(index).write_volatile(0xAB) };
            }
        };
        let copy_across_a_rebuild = |body: *mut u8| {
            move |address: usize, out: &mut [u8]| -> Result<(), String> {
                read_exact(address, &mut out[..HALF])?;
                rebuild_body(body);
                read_exact(address + HALF, &mut out[HALF..])?;
                Ok(())
            }
        };

        let mut blind = IntelFixture::new(vec![7u8; SLOTS], 1, 0);
        let blind_base = blind.address_mut();
        let blind_body = blind.current.as_mut_ptr();
        let accepted = capture_atomic_with(
            blind_base,
            ProducerHourWitness::UNKNOWN,
            copy_across_a_rebuild(blind_body),
        )
        .expect("the header bracket is blind to a rebuild that moves no header field");
        let mut expected_tear = vec![7u8; HALF];
        expected_tear.extend_from_slice(&[0xABu8; HALF]);
        assert_eq!(
            accepted.province_levels, expected_tear,
            "the injected rebuild did not straddle the copy, so nothing was torn"
        );

        let mut guarded = IntelFixture::new(vec![7u8; SLOTS], 1, 0);
        let guarded_base = guarded.address_mut();
        let guarded_body = guarded.current.as_mut_ptr();
        let mut guarded_landed = false;
        let torn = capture_atomic_with(
            guarded_base,
            ProducerHourWitness::UNKNOWN,
            |address, out| {
                read_exact(address, &mut out[..HALF])?;
                let window = RebuildWindow::open();
                rebuild_body(guarded_body);
                window.close();
                read_exact(address + HALF, &mut out[HALF..])?;
                guarded_landed = true;
                Ok(())
            },
        );
        assert!(guarded_landed, "the injected body read never ran (guarded)");
        assert_eq!(
            torn, None,
            "a body rebuilt under the copy was accepted because the header never moved"
        );
    }

    #[test]
    fn a_capture_taken_while_two_of_three_invocations_still_run_is_refused() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let fixture = IntelFixture::new(vec![7u8; 32], 1, 0);

        let first = RebuildWindow::open();
        let second = RebuildWindow::open();
        let third = RebuildWindow::open();
        first.close();

        let during = capture_atomic(fixture.address(), ProducerHourWitness::UNKNOWN);
        assert_eq!(
            during, None,
            "a capture was accepted while two producer invocations were still rebuilding"
        );

        second.close();
        let with_one_left = capture_atomic(fixture.address(), ProducerHourWitness::UNKNOWN);
        assert_eq!(
            with_one_left, None,
            "a capture was accepted while one producer invocation was still rebuilding"
        );

        third.close();
        assert!(
            capture_atomic(fixture.address(), ProducerHourWitness::UNKNOWN).is_some(),
            "captures must resume once every invocation has finished"
        );
    }

    #[test]
    fn the_shipped_hook_entry_opens_a_rebuild_window() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let fixture = IntelFixture::new(vec![5u8; 8], 63, 0);
        let gamestate = GameStateFixture::new(63);
        arm_drain_path();

        unsafe extern "system" fn inert_original(_intel: usize, _gamestate: usize) {}
        ORIGINAL.store(inert_original as *const () as usize, Ordering::Release);

        let before = REBUILD_STATE.load(Ordering::Acquire);
        unsafe { hook_producer(fixture.address(), gamestate.address()) };
        let after = REBUILD_STATE.load(Ordering::Acquire);

        assert_eq!(
            after,
            before + REBUILD_GENERATION_STEP,
            "the shipped hook entry did not open and close exactly one rebuild window"
        );
        assert_eq!(
            after & REBUILD_ACTIVE_MASK,
            0,
            "the shipped hook entry left an invocation in flight"
        );

        ORIGINAL.store(0, Ordering::Release);
        reset_lane_state();
    }

    #[test]
    fn a_capture_that_opens_inside_a_rebuild_window_is_refused() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let fixture = IntelFixture::new(vec![7u8; 32], 1, 0);

        let window = RebuildWindow::open();
        let during = capture_atomic(fixture.address(), ProducerHourWitness::UNKNOWN);
        window.close();
        let after = capture_atomic(fixture.address(), ProducerHourWitness::UNKNOWN);

        assert_eq!(
            during, None,
            "a capture that began while a producer invocation was in flight was accepted"
        );
        assert!(
            after.is_some(),
            "captures must resume once the rebuild window closes"
        );
    }

    #[test]
    fn the_shipped_detour_opens_and_closes_exactly_one_rebuild_window() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let gamestate = GameStateFixture::new(21);
        let mut fixture = IntelFixture::new(vec![1u8; 32], 21, FLAGS_DIRTY_SET);

        let opened = REBUILD_STATE.load(Ordering::Acquire);
        assert_eq!(
            opened & REBUILD_ACTIVE_MASK,
            0,
            "no invocation may be in flight between detour calls"
        );
        let outcome = run_detour_body(&mut fixture, &gamestate, |fixture| {
            emulate_rebuild(fixture, 0x5A)
        });
        let closed = REBUILD_STATE.load(Ordering::Acquire);

        assert_eq!(
            outcome,
            PovVisibilityInvocationOutcome::Republished,
            "the capture ran with the rebuild window still open"
        );
        assert_eq!(
            closed,
            opened + REBUILD_GENERATION_STEP,
            "the detour must open and close exactly one rebuild window per invocation"
        );

        reset_lane_state();
    }

    #[test]
    fn an_abandoned_rebuild_window_does_not_strand_the_epoch() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let fixture = IntelFixture::new(vec![7u8; 32], 1, 0);

        let before = REBUILD_STATE.load(Ordering::Acquire);
        drop(RebuildWindow::open());
        let after = REBUILD_STATE.load(Ordering::Acquire);

        assert_eq!(
            after,
            before + REBUILD_GENERATION_STEP,
            "an abandoned rebuild window never closed"
        );
        assert!(
            capture_atomic(fixture.address(), ProducerHourWitness::UNKNOWN).is_some(),
            "an abandoned rebuild window refused every later capture"
        );
    }

    #[test]
    fn a_concurrent_header_writer_yields_no_incoherent_accepted_frame() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut fixture = IntelFixture::new(vec![7u8; MAX_PROVINCE_SLOTS], 1, 0);
        let header = fixture.address_mut() + POV_COUNTRY_ID_OFFSET;
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let writer_stop = std::sync::Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            let slot = header as *mut u32;
            let mut next = 1u32;
            while !writer_stop.load(Ordering::Acquire) {
                next = if next == 1 { 2 } else { 1 };
                unsafe { slot.write_volatile(next) };
            }
        });

        let mut violation: Option<String> = None;
        for _ in 0..500 {
            let Some(accepted) = capture_atomic(fixture.address(), ProducerHourWitness::UNKNOWN)
            else {
                continue;
            };
            if accepted.pov_country_id != 1 && accepted.pov_country_id != 2 {
                violation = Some(format!(
                    "accepted a POV the writer never stored: {}",
                    accepted.pov_country_id
                ));
                break;
            }
            if accepted.province_levels != vec![7u8; MAX_PROVINCE_SLOTS] {
                violation = Some(format!(
                    "accepted a province vector the producer never held: {} bytes, first foreign byte at {:?}",
                    accepted.province_levels.len(),
                    accepted.province_levels.iter().position(|byte| *byte != 7)
                ));
                break;
            }
        }

        stop.store(true, Ordering::Release);
        let joined = writer.join();

        assert_eq!(violation, None);
        joined.expect("writer thread");
    }

    #[test]
    fn install_outside_the_admitted_module_publishes_no_receipt() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();

        let error = install().expect_err("the test process is not exact-a729 hoi4.exe");
        assert_ne!(error, PovVisibilityHookError::ModuleUnavailable);
        assert!(!installed());
        assert_eq!(
            installed_hook_receipts(),
            Err(PovVisibilityHookError::InstalledReceiptUnavailable.to_string())
        );
        assert_eq!(drain_latest(), Err(PovVisibilityDrainError::NotInstalled));

        reset_lane_state();
    }

    // Fails if the release receipt ever becomes available without this lane
    #[test]
    fn release_receipt_is_unavailable_until_this_custody_removes_the_hook() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();

        assert_eq!(
            take_released_hook_receipts(),
            Err(PovVisibilityHookError::ReleasedReceiptUnavailable.to_string())
        );
        assert_eq!(
            observe_release_receipt(),
            Err(PovVisibilityHookError::ReleaseRefused(
                "release attestation lacks owner-performed hook removal".into()
            ))
        );

        reset_lane_state();
    }

    // second call must not manufacture a second release.
    #[test]
    fn uninstall_without_an_installation_is_a_typed_no_op() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();

        assert_eq!(uninstall(), Ok(()));
        assert_eq!(uninstall(), Ok(()));
        assert_eq!(
            take_released_hook_receipts(),
            Err(PovVisibilityHookError::ReleasedReceiptUnavailable.to_string())
        );

        reset_lane_state();
    }

    #[test]
    fn producer_sentinel_matches_the_exact_a729_image_bytes() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(
            PRODUCER_SENTINEL,
            [
                0x40, 0x53, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x60, 0x83, 0xba,
                0x20, 0x05,
            ]
        );
        assert_eq!(
            &PRODUCER_SENTINEL[..EXPECTED_HOOK.target_patch_len as usize],
            &[0x40, 0x53, 0x56, 0x57, 0x41]
        );
    }

    // Process-lifetime trampoline retention

    fn arm_installed_capture(trampoline: usize) {
        arm_drain_path();
        ORIGINAL.store(trampoline, Ordering::Release);
        MODULE_BASE.store(0x1_0000, Ordering::Release);
        INSTALLED.store(true, Ordering::Release);
        ADMISSION.reopen();
    }

    #[test]
    fn teardown_disables_and_retains_and_never_frees_the_trampoline() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let trampoline = counting_original as *const () as usize;
        arm_installed_capture(trampoline);

        let ops = RecordingHookOps::succeeding(0);
        // `hoi4.exe`, so the release refusal here is expected and is not what
        let _ = uninstall_with_ops(&ops);

        assert_eq!(
            ops.performed(),
            vec![RecordedHookOp::Disable],
            "teardown may restore the target's bytes and nothing else: a \
             `RemoveAndFree` here is the trampoline being freed under a thread \
             that has already branched into the detour"
        );
        assert_eq!(
            ORIGINAL.load(Ordering::Acquire),
            trampoline,
            "the retained trampoline address must survive teardown"
        );
        assert!(!installed());

        reset_lane_state();
    }

    #[test]
    fn a_callback_that_branched_in_before_teardown_still_calls_the_engines_producer() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let trampoline = counting_original as *const () as usize;
        arm_installed_capture(trampoline);
        let fixture = IntelFixture::new(vec![5u8; 16], 21, 0);
        let gamestate = GameStateFixture::new(21);
        let intel_address = fixture.address();
        let gamestate_address = gamestate.address();

        let calls_before = ENGINE_ORIGINAL_CALLS.load(Ordering::Acquire);
        let skips_before = engine_original_skipped_invocations();
        let published_before = invocation_counts().republished;

        let at_threshold = Arc::new(AtomicBool::new(false));
        let proceed = Arc::new(AtomicBool::new(false));
        let worker = {
            let at_threshold = Arc::clone(&at_threshold);
            let proceed = Arc::clone(&proceed);
            let intel = intel_address;
            let gamestate = gamestate_address;
            std::thread::spawn(move || {
                at_threshold.store(true, Ordering::Release);
                while !proceed.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                unsafe { hook_producer(intel, gamestate) };
            })
        };
        while !at_threshold.load(Ordering::Acquire) {
            std::thread::yield_now();
        }

        let ops = RecordingHookOps::succeeding(0);
        let _ = uninstall_with_ops(&ops);
        let sampled_by_registry = ADMISSION.take_post_close_refused();
        assert_eq!(
            sampled_by_registry, 0,
            "unchanged and not hidden: the sample cannot see a callback that \
             has not entered yet"
        );

        proceed.store(true, Ordering::Release);
        worker.join().expect("the late callback returns");

        assert_eq!(
            ENGINE_ORIGINAL_CALLS.load(Ordering::Acquire),
            calls_before + 1,
            "the late callback must run the engine's own producer: skipping it \
             changes what the game computes for that frame"
        );
        assert_eq!(
            engine_original_skipped_invocations(),
            skips_before,
            "no invocation may take the no-trampoline return path"
        );
        assert_eq!(
            invocation_counts().republished,
            published_before,
            "and it must not publish into a slot nobody reads again"
        );
        assert_eq!(
            ADMISSION.take_post_close_refused(),
            1,
            "its refusal is counted, and survives to the next take"
        );

        reset_lane_state();
    }

    #[test]
    fn an_invocation_with_no_trampoline_is_counted_as_a_skipped_engine_original() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();
        let fixture = IntelFixture::new(vec![7u8; 8], 21, 0);
        let gamestate = GameStateFixture::new(21);
        ORIGINAL.store(0, Ordering::Release);

        let skips_before = engine_original_skipped_invocations();
        let published_before = invocation_counts().republished;
        unsafe { hook_producer(fixture.address(), gamestate.address()) };

        assert_eq!(
            engine_original_skipped_invocations(),
            skips_before + 1,
            "an invocation that did not run the engine's producer must be counted"
        );
        assert_eq!(
            invocation_counts().republished,
            published_before,
            "and it observed nothing to publish"
        );
        assert_eq!(ADMISSION.active(), 0, "the lease is still released");

        reset_lane_state();
    }

    #[test]
    fn a_refused_attestation_retains_the_enabled_sites_trampoline() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        ORIGINAL.store(0, Ordering::Release);
        let trampoline = counting_original as *const () as usize;
        let ops = RecordingHookOps::succeeding(trampoline);

        let error = install_at_with_ops(0x1_0000, 0x1_1000, &ops)
            .expect_err("the test process is not exact-a729 hoi4.exe");
        assert!(
            matches!(error, PovVisibilityHookError::ReceiptRefused(_)),
            "expected an attestation refusal, got {error:?}"
        );

        assert_eq!(
            ops.performed(),
            vec![
                RecordedHookOp::Create,
                RecordedHookOp::Enable,
                RecordedHookOp::Disable
            ],
            "a rollback over an ENABLED site may disable and must not free"
        );
        assert_eq!(
            ORIGINAL.load(Ordering::Acquire),
            trampoline,
            "the trampoline address is retained across a refused install"
        );
        assert!(!installed());

        ORIGINAL.store(0, Ordering::Release);
        reset_lane_state();
    }

    #[test]
    fn a_reinstall_adopts_the_retained_creation_rather_than_failing() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let trampoline = counting_original as *const () as usize;
        ORIGINAL.store(trampoline, Ordering::Release);

        let ops = RecordingHookOps {
            performed: Mutex::new(Vec::new()),
            create: Err(MH_STATUS::MH_ERROR_ALREADY_CREATED),
            enable: Ok(()),
            disable: Ok(()),
            remove: Ok(()),
        };
        let error = install_at_with_ops(0x1_0000, 0x1_1000, &ops)
            .expect_err("the test process is not exact-a729 hoi4.exe");
        assert!(
            matches!(error, PovVisibilityHookError::ReceiptRefused(_)),
            "the already-created site must proceed to attestation, got {error:?}"
        );
        assert_eq!(
            ops.performed(),
            vec![
                RecordedHookOp::Create,
                RecordedHookOp::Enable,
                RecordedHookOp::Disable
            ],
            "an adopted creation is re-enabled, not re-created"
        );
        assert_eq!(ORIGINAL.load(Ordering::Acquire), trampoline);

        ORIGINAL.store(0, Ordering::Release);
        let ops = RecordingHookOps {
            performed: Mutex::new(Vec::new()),
            create: Err(MH_STATUS::MH_ERROR_ALREADY_CREATED),
            enable: Ok(()),
            disable: Ok(()),
            remove: Ok(()),
        };
        let error = install_at_with_ops(0x1_0000, 0x1_1000, &ops)
            .expect_err("an already-created site with no retained address is not installable");
        match error {
            PovVisibilityHookError::HookCreationFailed(detail) => {
                assert!(detail.contains("retained trampoline"), "{detail}")
            }
            other => panic!("expected a typed creation failure, got {other:?}"),
        }
        assert_eq!(
            ops.performed(),
            vec![RecordedHookOp::Create],
            "nothing may be enabled without a trampoline address"
        );

        reset_lane_state();
    }

    #[test]
    fn no_install_path_reaches_the_trampoline_free() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let trampoline = counting_original as *const () as usize;

        reset_lane_state();
        ORIGINAL.store(0, Ordering::Release);
        let fresh = RecordingHookOps::succeeding(trampoline);
        let _ = install_at_with_ops(0x1_0000, 0x1_1000, &fresh);
        assert!(
            !fresh.performed().contains(&RecordedHookOp::RemoveAndFree),
            "a fresh install must not free its trampoline: {:?}",
            fresh.performed()
        );

        reset_lane_state();
        ORIGINAL.store(trampoline, Ordering::Release);
        let adopted = RecordingHookOps {
            performed: Mutex::new(Vec::new()),
            create: Err(MH_STATUS::MH_ERROR_ALREADY_CREATED),
            enable: Ok(()),
            disable: Ok(()),
            remove: Ok(()),
        };
        let _ = install_at_with_ops(0x1_0000, 0x1_1000, &adopted);
        assert!(
            !adopted.performed().contains(&RecordedHookOp::RemoveAndFree),
            "an adopted site was enabled by a previous capture and may still              have a poised entrant: {:?}",
            adopted.performed()
        );
        assert_eq!(ORIGINAL.load(Ordering::Acquire), trampoline);

        reset_lane_state();
        ORIGINAL.store(0, Ordering::Release);
        let enable_failed = RecordingHookOps {
            performed: Mutex::new(Vec::new()),
            create: Ok(trampoline),
            enable: Err(InlineDetourEnableError::MinHook(
                MH_STATUS::MH_ERROR_NOT_EXECUTABLE,
            )),
            disable: Ok(()),
            remove: Ok(()),
        };
        let error = install_at_with_ops(0x1_0000, 0x1_1000, &enable_failed)
            .expect_err("a failed enable is not an install");
        assert!(matches!(error, PovVisibilityHookError::HookEnableFailed(_)));
        assert_eq!(
            enable_failed.performed(),
            vec![
                RecordedHookOp::Create,
                RecordedHookOp::Enable,
                RecordedHookOp::Disable
            ],
            "an enable that may have patched the target retains"
        );
        assert_eq!(
            ORIGINAL.load(Ordering::Acquire),
            trampoline,
            "and keeps the trampoline address callable"
        );

        ORIGINAL.store(0, Ordering::Release);
        reset_lane_state();
    }

    #[test]
    fn a_refused_publication_reset_creates_nothing_at_all() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        let trampoline = counting_original as *const () as usize;
        ORIGINAL.store(trampoline, Ordering::Release);

        let ops = RecordingHookOps::succeeding(0xDEAD_BEEF);
        let error = install_at_with_ops_and_slot(0x1_0000, 0x1_1000, &ops, || {
            Err(PovVisibilityHookError::LatestSnapshotPoisoned)
        })
        .expect_err("a refused publication reset is not an install");

        assert_eq!(error, PovVisibilityHookError::LatestSnapshotPoisoned);
        assert!(
            ops.performed().is_empty(),
            "nothing may be created or adopted before the slot is cleared: {:?}",
            ops.performed()
        );
        assert_eq!(
            ORIGINAL.load(Ordering::Acquire),
            trampoline,
            "and the retained address of a previous capture is untouched"
        );
        assert!(!installed());

        arm_drain_path();
        let standing = IntelFixture::new(vec![3u8; 16], 21, 0);
        publish_capture(standing.address(), ProducerHourWitness::UNKNOWN);
        assert!(
            SNAPSHOT_PRESENT.load(Ordering::Acquire),
            "the fixture must actually be published for this to test anything"
        );
        assert!(drain_latest().expect("drain").is_some());
        INSTALLED.store(false, Ordering::Release);

        let ops = RecordingHookOps::succeeding(trampoline);
        let _ = install_at_with_ops(0x1_0000, 0x1_1000, &ops);

        assert!(
            !SNAPSHOT_PRESENT.load(Ordering::Acquire),
            "a new capture must not inherit the previous one's published frame"
        );
        arm_drain_path();
        assert_eq!(
            drain_latest().expect("drain"),
            None,
            "the shipped install path must clear the slot, not merely be able to"
        );

        ORIGINAL.store(0, Ordering::Release);
        reset_lane_state();
    }

    #[test]
    fn a_failed_disable_latches_an_unproved_removal() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);

        let ops = RecordingHookOps {
            performed: Mutex::new(Vec::new()),
            create: Ok(0),
            enable: Ok(()),
            disable: Err(InlineDetourReleaseError::MinHook(
                MH_STATUS::MH_ERROR_NOT_EXECUTABLE,
            )),
            remove: Ok(()),
        };
        retain_and_disable(0x1_1000, &ops);

        assert_eq!(ops.performed(), vec![RecordedHookOp::Disable]);
        assert!(
            failed_install_removal_unproved(),
            "a disable that failed cannot prove the detour left the target"
        );

        FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);
        reset_lane_state();
    }

    // which is what stops the registry accepting the release. The release
    #[test]
    fn a_failed_disable_refuses_the_teardown_before_any_release() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_installed_capture(counting_original as *const () as usize);

        let ops = RecordingHookOps {
            performed: Mutex::new(Vec::new()),
            create: Ok(0),
            enable: Ok(()),
            disable: Err(InlineDetourReleaseError::MinHook(
                MH_STATUS::MH_ERROR_NOT_EXECUTABLE,
            )),
            remove: Ok(()),
        };
        let error = uninstall_with_ops(&ops).expect_err("a failed disable is not a teardown");
        assert!(
            matches!(error, PovVisibilityHookError::HookDisableFailed(_)),
            "expected a typed disable failure, got {error:?}"
        );
        assert_eq!(
            take_released_hook_receipts(),
            Err(PovVisibilityHookError::ReleasedReceiptUnavailable.to_string()),
            "no release may be minted over a detour that is still in the target"
        );

        reset_lane_state();
    }

    #[test]
    fn a_refused_teardown_is_retried_and_the_retry_can_still_reach_the_target() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_installed_capture(counting_original as *const () as usize);
        let base = MODULE_BASE.load(Ordering::Acquire);
        assert!(base != 0, "the fixture must be installed at a real base");

        let refusing = RecordingHookOps {
            performed: Mutex::new(Vec::new()),
            create: Ok(0),
            enable: Ok(()),
            disable: Err(InlineDetourReleaseError::MinHook(
                MH_STATUS::MH_ERROR_NOT_EXECUTABLE,
            )),
            remove: Ok(()),
        };
        uninstall_with_ops(&refusing).expect_err("a refused disable is not a teardown");
        assert!(!installed(), "admission is closed either way");
        assert!(
            failed_install_removal_unproved(),
            "a teardown whose disable refused leaves the detour LIVE and must say so"
        );
        assert_eq!(
            MODULE_BASE.load(Ordering::Acquire),
            base,
            "the retry needs the address the detour is at"
        );

        // The retry runs, reaches the same target, and closes it. The release
        let succeeding = RecordingHookOps::succeeding(counting_original as *const () as usize);
        let _ = uninstall_with_ops(&succeeding);
        assert_eq!(
            succeeding.performed(),
            vec![RecordedHookOp::Disable],
            "the retry must actually attempt the disable, not return Ok on a clear INSTALLED flag"
        );
        assert!(
            !failed_install_removal_unproved(),
            "a proved removal clears the latch"
        );
        assert_eq!(MODULE_BASE.load(Ordering::Acquire), 0);

        let idle = RecordingHookOps::succeeding(counting_original as *const () as usize);
        uninstall_with_ops(&idle).expect("an already-torn-down lane is a no-op");
        assert!(
            idle.performed().is_empty(),
            "a proved removal must not be attempted again: {:?}",
            idle.performed()
        );

        // provability check and the standard-group release predicate), so a
        let source = include_str!("../../../collector/src/attach_log.rs");
        let anchor = source
            .find("\"D94 teardown: {error}\"")
            .expect("the production hook registry teardown reports a D94 failure");
        let guard = &source[anchor.saturating_sub(700)..anchor];
        assert!(
            guard.contains("fow_visibility_live_lane::failed_install_removal_unproved()"),
            "the production hook registry teardown must consult the unproved-removal latch \
             beside installed(), as it already does for D99, D95 and D97 -- asking installed() \
             alone skips the one lane whose live patch it cannot see; observed guard: {guard:?}"
        );

        reset_lane_state();
        FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);
    }

    #[test]
    fn a_failed_install_whose_rollback_could_not_disable_is_still_retryable() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);
        MODULE_BASE.store(0x1_0000, Ordering::Release);
        let target = 0x1_0000usize + PRODUCER_RVA as usize;

        let refusing = RecordingHookOps {
            performed: Mutex::new(Vec::new()),
            create: Ok(0),
            enable: Ok(()),
            disable: Err(InlineDetourReleaseError::MinHook(
                MH_STATUS::MH_ERROR_NOT_EXECUTABLE,
            )),
            remove: Ok(()),
        };
        retain_and_disable(target, &refusing);
        assert!(
            failed_install_removal_unproved(),
            "a rollback whose disable refused cannot prove the detour left the target"
        );
        assert_eq!(
            MODULE_BASE.load(Ordering::Acquire),
            0x1_0000,
            "the retry needs the base the failed install patched at"
        );

        let succeeding = RecordingHookOps::succeeding(counting_original as *const () as usize);
        let _ = uninstall_with_ops(&succeeding);
        assert_eq!(
            succeeding.performed(),
            vec![RecordedHookOp::Disable],
            "the retry must attempt the disable at the recorded base"
        );
        assert!(!failed_install_removal_unproved());
        assert_eq!(MODULE_BASE.load(Ordering::Acquire), 0);

        MODULE_BASE.store(0x1_0000, Ordering::Release);
        let clean = RecordingHookOps::succeeding(counting_original as *const () as usize);
        retain_and_disable(target, &clean);
        assert!(!failed_install_removal_unproved());
        assert_eq!(MODULE_BASE.load(Ordering::Acquire), 0);

        reset_lane_state();
        FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);
    }

    #[test]
    fn producer_rva_matches_the_published_fow1_authority() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(format!("0x{PRODUCER_RVA:08x}"), "0x01606ea0");
        assert_eq!(EXPECTED_HOOK.requested_rva, u64::from(PRODUCER_RVA));
    }

    #[test]
    fn a_publication_landing_after_a_drain_is_current_at_the_next_drain() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();

        let early_fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        publish_capture(early_fixture.address(), ProducerHourWitness::UNKNOWN);
        let early = drain_latest().expect("drain").expect("the published frame");
        assert_eq!(early.province_levels, vec![5u8; 32]);

        let later_fixture = IntelFixture::new(vec![9u8; 32], 21, 0);
        publish_capture(later_fixture.address(), ProducerHourWitness::UNKNOWN);

        let next = drain_latest()
            .expect("drain")
            .expect("a publication that lands after a drain must still be there");
        assert_eq!(
            next.province_levels,
            vec![9u8; 32],
            "a publication that lands after a drain must be current at the next \
             drain, not skipped"
        );
        assert!(
            next.capture_sequence > early.capture_sequence,
            "the producer minted a new capture, so the consumer must be able to \
             tell it from the frame it replaced"
        );

        let again = drain_latest().expect("drain").expect("still retained");
        assert_eq!(again.province_levels, vec![9u8; 32]);
        assert_eq!(again.capture_sequence, next.capture_sequence);

        reset_lane_state();
    }

    #[test]
    fn a_retained_frame_reports_its_producers_hour_not_the_drains() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();

        let observed_in = FIXTURE_GAME_HOUR_RAW;
        let fixture = IntelFixture::new(vec![5u8; 32], 21, 0);
        publish_capture(fixture.address(), uncontested_witness(Some(observed_in)));

        let first = drain_latest().expect("drain").expect("the published frame");
        assert_eq!(first.producer_hour_raw, Some(observed_in));

        for drain in 0..3 {
            let again = drain_latest().expect("drain").expect("still retained");
            assert_eq!(
                again.producer_hour_raw,
                Some(observed_in),
                "a re-served frame keeps its producer's hour (drain {drain})"
            );
            assert_eq!(again.capture_sequence, first.capture_sequence);
            let bytes = again.encode().expect("the retained frame encodes");
            assert_eq!(
                PovVisibilitySnapshot::decode(&bytes)
                    .expect("round trip")
                    .producer_hour_raw,
                Some(observed_in)
            );
        }

        reset_lane_state();
    }

    #[test]
    fn the_stamped_hour_is_the_one_the_producer_entered_on() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();

        let mut fixture = IntelFixture::new(vec![3u8; 16], 21, FLAGS_DIRTY_SET);
        let intel = fixture.address_mut();
        let mut gamestate = GameStateFixture::new(21);
        let gamestate_address = gamestate.address();
        let date_word = gamestate.block[GAMESTATE_CURRENT_DATE_OFFSET..].as_mut_ptr() as *mut i32;

        let outcome = run_producer_invocation(intel, gamestate_address, || {
            unsafe { date_word.write_unaligned(FIXTURE_GAME_HOUR_RAW + 1) };
            emulate_rebuild(&mut fixture, 3);
        });
        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);

        let published = drain_latest().expect("drain").expect("the published frame");
        assert_eq!(
            published.producer_hour_raw,
            Some(FIXTURE_GAME_HOUR_RAW),
            "the hour is sampled before the producer runs, not after"
        );

        reset_lane_state();
    }

    #[test]
    fn an_unchanged_invocation_does_not_restamp_the_standing_frames_hour() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();

        let mut fixture = IntelFixture::new(vec![4u8; 24], 21, FLAGS_DIRTY_SET);
        let intel = fixture.address_mut();
        let observed = GameStateFixture::new(21);
        assert_eq!(
            run_producer_invocation(intel, observed.address(), || emulate_rebuild(
                &mut fixture,
                4
            )),
            PovVisibilityInvocationOutcome::Republished
        );
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("the published frame")
                .producer_hour_raw,
            Some(FIXTURE_GAME_HOUR_RAW)
        );

        let mut later = GameStateFixture::new(21);
        later.set_current_date_raw(FIXTURE_GAME_HOUR_RAW + 1);
        assert_eq!(
            run_producer_invocation(intel, later.address(), || {}),
            PovVisibilityInvocationOutcome::UnchangedNotRecaptured
        );
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("the standing frame")
                .producer_hour_raw,
            Some(FIXTURE_GAME_HOUR_RAW),
            "an invocation that rebuilt nothing must not re-date the standing frame"
        );

        reset_lane_state();
    }

    #[test]
    fn a_vector_a_later_invocation_rebuilt_carries_no_hour() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();

        let fixture = IntelFixture::new(vec![6u8; 16], 21, 0);

        let witness = uncontested_witness(Some(FIXTURE_GAME_HOUR_RAW));
        publish_capture(fixture.address(), witness);
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("the published frame")
                .producer_hour_raw,
            Some(FIXTURE_GAME_HOUR_RAW),
            "an uncontested capture carries its own invocation's hour"
        );

        RebuildWindow::open().close();
        publish_capture(fixture.address(), witness);
        let published = drain_latest()
            .expect("drain")
            .expect("the vector is still published");
        assert_eq!(
            published.province_levels,
            vec![6u8; 16],
            "the observation itself is kept: only the attribution is unknown"
        );
        assert_eq!(
            published.producer_hour_raw, None,
            "a vector some later invocation rebuilt may not carry this \
             invocation's hour"
        );

        reset_lane_state();
    }

    #[test]
    fn a_nested_invocation_inside_this_one_also_costs_the_hour() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();

        let mut fixture = IntelFixture::new(vec![7u8; 16], 21, FLAGS_DIRTY_SET);
        let intel = fixture.address_mut();
        let gamestate = GameStateFixture::new(21);

        let outcome = run_producer_invocation(intel, gamestate.address(), || {
            RebuildWindow::open().close();
            emulate_rebuild(&mut fixture, 7);
        });
        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);

        let published = drain_latest()
            .expect("drain")
            .expect("the vector is still published");
        assert_eq!(published.province_levels, vec![7u8; 16]);
        assert_eq!(
            published.producer_hour_raw, None,
            "a nested invocation's rebuild may not inherit the outer \
             invocation's hour"
        );

        reset_lane_state();
    }

    #[test]
    fn an_invocation_between_this_ones_entry_and_its_window_also_costs_the_hour() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reset_lane_state();
        arm_drain_path();

        let mut fixture = IntelFixture::new(vec![8u8; 16], 21, FLAGS_DIRTY_SET);
        let intel = fixture.address_mut();
        let gamestate = GameStateFixture::new(21);

        let rebuild_state_at_entry = REBUILD_STATE.load(Ordering::Acquire);
        RebuildWindow::open().close();

        let outcome = run_producer_invocation_from(
            rebuild_state_at_entry,
            intel,
            gamestate.address(),
            || emulate_rebuild(&mut fixture, 8),
        );
        assert_eq!(outcome, PovVisibilityInvocationOutcome::Republished);

        let published = drain_latest()
            .expect("drain")
            .expect("the vector is still published");
        assert_eq!(published.province_levels, vec![8u8; 16]);
        assert_eq!(
            published.producer_hour_raw, None,
            "an invocation that ran before this one's window may not have its \
             vector dated by this one"
        );

        let mut clean = IntelFixture::new(vec![9u8; 16], 21, FLAGS_DIRTY_SET);
        let clean_intel = clean.address_mut();
        let entry = REBUILD_STATE.load(Ordering::Acquire);
        run_producer_invocation_from(entry, clean_intel, gamestate.address(), || {
            emulate_rebuild(&mut clean, 9)
        });
        assert_eq!(
            drain_latest()
                .expect("drain")
                .expect("the published frame")
                .producer_hour_raw,
            Some(FIXTURE_GAME_HOUR_RAW),
            "an uncontested invocation through this entry point keeps its hour"
        );

        reset_lane_state();
    }

    #[test]
    fn an_unreadable_game_state_carries_no_hour_rather_than_a_substitute() {
        let _serial = TEST_SERIAL
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for unreadable in [0usize, usize::MAX, 1] {
            assert_eq!(
                read_producer_hour(unreadable),
                None,
                "an unreadable game state carries no hour"
            );
        }
        let mut gamestate = GameStateFixture::new(21);
        gamestate.set_current_date_raw(-1);
        assert_eq!(read_producer_hour(gamestate.address()), Some(-1));
        gamestate.set_current_date_raw(FIXTURE_GAME_HOUR_RAW);
        assert_eq!(
            read_producer_hour(gamestate.address()),
            Some(FIXTURE_GAME_HOUR_RAW)
        );
    }
}
