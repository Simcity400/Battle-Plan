//! Exact-a729 delayed-event lifecycle observer for A19.
//!
//! Two lane-owned detours bracket the country hourly update and definition
//! cancellation paths. Dispatch is observed through d75's already-installed
//! common fire detour; this module never installs a competing hook at

#![cfg(windows)]

use core::{cell::RefCell, ffi::c_void, mem, ptr};
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released, ExpectedRvaHook,
    HookTeardownAttestation, InstallAbortDiscard, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const HOURLY_UPDATE_RVA: usize = 0x006E_FE10;
pub const DEFINITION_CANCEL_RVA: usize = 0x006D_D7F0;
pub const EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_A19_DELAYED_EVENT_EVIDENCE";
/// The exact hourly/cancel detours are installed passively whenever the
/// existing delayed-events domain is scheduled. Missing roots, callback loss,
/// or incomplete lifecycle custody fail the observation closed.
pub const PRODUCTION_SCHEDULING_ENABLED: bool = true;
/// Capacity of the reservable lifecycle queue. An enqueue that would exceed it
/// is refused and charged to `dropped_overflow`, which clears the ledger's
/// `complete` flag -- so one drain never carries more rows than this, and the
/// replay consumer applies the same ceiling to the ledger it reads back.
pub const MAX_PENDING_EVENTS: usize = portable_admission::MAX_DELAYED_EVENT_LIFECYCLE_EVENTS;

const CGAMESTATE_SINGLETON_RVA: usize = 0x0331_6E90;
const CGAMESTATE_CURRENT_DATE_OFF: usize = 0x468;
const CCOUNTRY_VTABLE_RVA: usize = 0x027D_0E90;
const CCOUNTRY_TAG_OFF: usize = 0x08;
const CCOUNTRY_DELAYED_BEGIN_OFF: usize = 0x1290;
const CCOUNTRY_DELAYED_CAPACITY_OFF: usize = 0x1298;
const CCOUNTRY_DELAYED_COUNT_OFF: usize = 0x129C;
const CDELAYED_EVENT_VTABLE_RVA: usize = 0x027D_0FB8;
const CDELAYED_EVENT_EVENT_OFF: usize = 0x08;
const CDELAYED_EVENT_SCOPE_OFF: usize = 0x10;
const CDELAYED_EVENT_SCOPE_TAG_OFF: usize = 0x18;
const CDELAYED_EVENT_SCOPE_RANDOM_A_OFF: usize = 0x1C;
const CDELAYED_EVENT_SCOPE_RANDOM_B_OFF: usize = 0x20;
const CDELAYED_EVENT_ORIGINATOR_OFF: usize = 0xC0;
const CDELAYED_EVENT_DUE_OFF: usize = 0xC4;
const CEVENT_COMBINED_ID_OFF: usize = 0x0C;
/// Owner/originator ceiling for a lifecycle row. Shared with the replay
/// consumer, which used to apply no upper bound at all.
const MAX_COUNTRY_TAG_DB_ID: u32 = portable_admission::MAX_DELAYED_EVENT_LIFECYCLE_TAG_DB_ID;
const MIN_COMBINED_EVENT_ID: u32 = 100_000;
const MAX_COMBINED_EVENT_ID: u32 = 100_099_999;
const MAX_DUE_HOURS: u32 = 500_000;
/// Plausibility ceiling on ONE country's `CCountry+0x129C` delayed-event vector
/// count. Numerically equal to [`MAX_PENDING_EVENTS`] and deliberately NOT
/// shared with it: that one is the lifecycle hook queue's capacity, which the
/// replay consumer re-applies to the ledger it reads back, while this bounds an
/// engine vector this scan walks. Coupling two different quantities because
/// they happen to spell the same number is how a bound acquires a meaning
/// nobody chose.
const MAX_EVENTS_PER_COUNTRY: u32 = 65_536;
const HEAP_LO: usize = 0x100_0000_0000;
const HEAP_HI: usize = 0x7ff0_0000_0000;

const HOURLY_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
const DEFINITION_CANCEL_SENTINEL: [u8; 16] = [
    0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x89, 0x5c, 0x24, 0x40, 0x48, 0x8b, 0xf9, 0x33, 0xdb,
];

const EXPECTED_HOOKS: [ExpectedRvaHook; 2] = [
    ExpectedRvaHook {
        logical_name: "gameplay.d64.definition_cancel",
        requested_rva: DEFINITION_CANCEL_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &DEFINITION_CANCEL_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d64.hourly_delayed_event_update",
        requested_rva: HOURLY_UPDATE_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &HOURLY_UPDATE_SENTINEL,
    },
];

type HourlyUpdateFn = unsafe extern "system" fn(usize);
type DefinitionCancelFn = unsafe extern "system" fn(usize, usize);

static ORIGINAL_HOURLY_UPDATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DEFINITION_CANCEL: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static DISPATCH_OBSERVER_REGISTERED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static COMPLETENESS_POISONED: AtomicBool = AtomicBool::new(false);
static DROPPED_REENTRANT: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_CORRELATION: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static NEXT_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_DETACH_AUTHORITY_ID: AtomicU64 = AtomicU64::new(1);
static LIFECYCLE_MUTATION_GENERATION: AtomicU64 = AtomicU64::new(2);
static ACTIVE_LIFECYCLE_MUTATIONS: AtomicUsize = AtomicUsize::new(0);
static QUEUE: OnceLock<Mutex<ReservableLifecycleQueue>> = OnceLock::new();
static HOOK_RECEIPTS: OnceLock<Mutex<HookReceiptState>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static OPEN_CALL: RefCell<Option<OpenCall>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum LifecycleReason {
    RetainedDecrement = 1,
    Executed = 2,
    FailedTriggerConsumed = 3,
    DefinitionCancelled = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub thread_id: u32,
    pub owner_tag_db_id: u32,
    pub event_id: u32,
    pub originator_tag_db_id: u32,
    pub scope_random: u64,
    pub due_before: u32,
    pub due_after: Option<u32>,
    pub reason: LifecycleReason,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleDiagnostics {
    pub hook_epoch: u64,
    pub dispatch_observer_registered: bool,
    pub completeness_poisoned: bool,
    pub dropped_reentrant: u64,
    pub dropped_unreadable: u64,
    pub dropped_correlation: u64,
    pub dropped_overflow: u64,
    pub dropped_lock_busy: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleDrain {
    pub events: Vec<LifecycleEvent>,
    pub diagnostics: LifecycleDiagnostics,
    pub mutation_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub hourly_update: usize,
    pub definition_cancel: usize,
}

#[derive(Default)]
struct HookReceiptState {
    installing: Vec<InstalledRvaHookReceipt>,
    installed: Option<[InstalledRvaHookReceipt; 2]>,
    release_pending: bool,
    release_full_roster: bool,
    releasing: Vec<ReleasedRvaHookReceipt>,
    released: Option<[ReleasedRvaHookReceipt; 2]>,
    active_detach_authority_id: Option<u64>,
    terminal_custody_identity_sha256: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReservationOwner {
    Delivery,
    Teardown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QueueReservation {
    id: u64,
    row_count: usize,
    owner: ReservationOwner,
}

#[derive(Debug)]
struct ReservableLifecycleQueue {
    pending: VecDeque<LifecycleEvent>,
    reservation: Option<QueueReservation>,
}

impl ReservableLifecycleQueue {
    fn new() -> Self {
        Self {
            pending: VecDeque::with_capacity(MAX_PENDING_EVENTS),
            reservation: None,
        }
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.reservation = None;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleDrainReservation {
    pub reservation_id: u64,
    pub drain: LifecycleDrain,
}

pub const D64_TERMINAL_LIFECYCLE_SCHEMA: &str = "battle-plan.d64.lifecycle_terminal_envelope.v1";
pub const D64_POISONED_ABORT_SCHEMA: &str = "battle-plan.d64.poisoned_abort_envelope.v1";
pub const D64_TERMINAL_ATTEMPT_BUDGET: u8 = 3;

/// Opaque, one-session authority to detach the D75 bridge and terminate D64.
/// It is minted only after live re-observation of both exact installed D64
/// hooks while the D75 dispatch observer is registered.
#[must_use = "D64 detach authority must terminate or abort D64 custody"]
pub struct D64DetachAuthority {
    authority_id: u64,
    terminal_attempts_remaining: u8,
}

/// Test-only opaque authority used by dependent lane tests. This feature is
/// enabled only as a dev-dependency and is absent from shipped artifacts.
#[cfg(feature = "test-support")]
pub fn mint_test_detach_authority() -> D64DetachAuthority {
    D64DetachAuthority {
        authority_id: u64::MAX,
        terminal_attempts_remaining: D64_TERMINAL_ATTEMPT_BUDGET,
    }
}

impl D64DetachAuthority {
    pub fn terminal_attempts_remaining(&self) -> u8 {
        self.terminal_attempts_remaining
    }
}

#[must_use = "terminal seal failure retains D64 authority for retry or abort"]
pub struct D64TerminalSealFailure {
    message: String,
    authority: D64DetachAuthority,
}

impl D64TerminalSealFailure {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn retry_available(&self) -> bool {
        self.authority.terminal_attempts_remaining != 0
    }

    pub fn into_authority(self) -> D64DetachAuthority {
        self.authority
    }
}

impl fmt::Display for D64TerminalSealFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl fmt::Debug for D64TerminalSealFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("D64TerminalSealFailure")
            .field("message", &self.message)
            .field(
                "retry_available",
                &self.authority.terminal_attempts_remaining,
            )
            .finish()
    }
}

impl std::error::Error for D64TerminalSealFailure {}

#[must_use = "terminal abort failure retains D64 authority for cleanup retry"]
pub struct D64TerminalAbortFailure {
    message: String,
    authority: D64DetachAuthority,
}

impl D64TerminalAbortFailure {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn into_authority(self) -> D64DetachAuthority {
        self.authority
    }
}

impl fmt::Display for D64TerminalAbortFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl fmt::Debug for D64TerminalAbortFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("D64TerminalAbortFailure")
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl std::error::Error for D64TerminalAbortFailure {}

/// Non-replayable terminal D64 custody result. Fields are private so only the
/// exact sealed producer below can construct one.
#[must_use = "authenticated D64 terminal custody must be bound to D75 publication"]
pub struct AuthenticatedD64TerminalDrainReceipt {
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    payload: Vec<u8>,
    payload_sha256: [u8; 32],
    terminal_del2: Vec<u8>,
    terminal_del2_sha256: [u8; 32],
    drain: LifecycleDrain,
    released_hooks: [ReleasedRvaHookReceipt; 2],
}

impl AuthenticatedD64TerminalDrainReceipt {
    pub fn custody_identity_sha256(&self) -> [u8; 32] {
        self.custody_identity_sha256
    }

    pub fn frame_hour(&self) -> u64 {
        self.frame_hour
    }

    pub fn elapsed_wall_ms(&self) -> u64 {
        self.elapsed_wall_ms
    }

    pub fn lifecycle_envelope(&self) -> &[u8] {
        &self.payload
    }

    pub fn lifecycle_envelope_sha256(&self) -> [u8; 32] {
        self.payload_sha256
    }

    /// Canonical terminal DEL2 bytes which must be the exact suffix of the
    /// terminal D64 DATA payload. Keeping this serialization in the sealed D64
    /// producer prevents a caller from pairing the authenticated lifecycle
    /// envelope with a substituted D64 carrier.
    pub fn terminal_del2(&self) -> &[u8] {
        &self.terminal_del2
    }

    pub fn terminal_del2_sha256(&self) -> [u8; 32] {
        self.terminal_del2_sha256
    }

    pub fn drain(&self) -> &LifecycleDrain {
        &self.drain
    }

    pub fn released_hooks(&self) -> &[ReleasedRvaHookReceipt; 2] {
        &self.released_hooks
    }

    pub fn diagnostics_clean(&self) -> bool {
        terminal_diagnostics_clean(&self.drain.diagnostics)
    }
}

#[must_use = "authenticated poisoned D64 abort must be bound to D75 failure custody"]
pub struct AuthenticatedPoisonedD64AbortReceipt {
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    reason_sha256: [u8; 32],
    payload: Vec<u8>,
    payload_sha256: [u8; 32],
    drain: LifecycleDrain,
    released_hooks: [ReleasedRvaHookReceipt; 2],
}

impl AuthenticatedPoisonedD64AbortReceipt {
    pub fn custody_identity_sha256(&self) -> [u8; 32] {
        self.custody_identity_sha256
    }

    pub fn frame_hour(&self) -> u64 {
        self.frame_hour
    }

    pub fn elapsed_wall_ms(&self) -> u64 {
        self.elapsed_wall_ms
    }

    pub fn reason_sha256(&self) -> [u8; 32] {
        self.reason_sha256
    }

    pub fn abort_envelope(&self) -> &[u8] {
        &self.payload
    }

    pub fn abort_envelope_sha256(&self) -> [u8; 32] {
        self.payload_sha256
    }

    pub fn drain(&self) -> &LifecycleDrain {
        &self.drain
    }

    pub fn released_hooks(&self) -> &[ReleasedRvaHookReceipt; 2] {
        &self.released_hooks
    }

    pub fn capture_poisoned(&self) -> bool {
        true
    }

    pub fn preterminal_cancellation(&self) -> bool {
        self.frame_hour == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallKind {
    Hourly,
    DefinitionCancel { event: usize },
}

#[derive(Clone, Debug)]
struct SnapshotRow {
    object: usize,
    event: usize,
    event_scope: usize,
    owner_tag_db_id: u32,
    event_id: u32,
    originator_tag_db_id: u32,
    scope_random: u64,
    due: u32,
}

#[derive(Debug)]
struct OpenCall {
    kind: CallKind,
    owner_tag_db_id: u32,
    before: Vec<SnapshotRow>,
    dispatched_objects: BTreeSet<usize>,
    poisoned: bool,
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

struct LifecycleMutationGuard;
impl LifecycleMutationGuard {
    fn enter() -> Self {
        if ACTIVE_LIFECYCLE_MUTATIONS.fetch_add(1, Ordering::AcqRel) == 0 {
            advance_lifecycle_mutation_generation();
        }
        Self
    }
}
impl Drop for LifecycleMutationGuard {
    fn drop(&mut self) {
        if ACTIVE_LIFECYCLE_MUTATIONS.fetch_sub(1, Ordering::AcqRel) == 1 {
            advance_lifecycle_mutation_generation();
        }
    }
}

pub fn evidence_activation_enabled() -> bool {
    std::env::var(EVIDENCE_ACTIVATION_ENV).ok().as_deref() == Some("1")
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Whether the A19 dispatch observer is currently registered.
///
/// The collector's production hook registry needs a LANE-owned answer to
/// "is the D75 producer edge still open". Its own install bookkeeping is not
/// one: the flag is set at install and cleared only by the abort path, so a
/// capture that released through the typed terminal transaction leaves it
/// permanently true.
pub fn dispatch_observer_registered() -> bool {
    DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire)
}

pub fn set_dispatch_observer_registered(registered: bool) {
    let _mutation = LifecycleMutationGuard::enter();
    DISPATCH_OBSERVER_REGISTERED.store(registered, Ordering::Release);
    if !registered && INSTALLED.load(Ordering::Acquire) {
        poison(&DROPPED_CORRELATION);
    }
}

/// Called synchronously from d75's authenticated common-fire detour. No raw
/// address is retained beyond this function.
pub fn observe_dispatch(
    event: usize,
    actor_tag_id: u32,
    recipient_tag_id: u32,
    event_scope: usize,
) {
    OPEN_CALL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(open) = slot.as_mut() else { return };
        if !matches!(open.kind, CallKind::Hourly) || recipient_tag_id != open.owner_tag_db_id {
            return;
        }
        let matches = open
            .before
            .iter()
            .filter(|row| {
                row.event == event
                    && row.event_scope == event_scope
                    && row.originator_tag_db_id == actor_tag_id
                    && row.owner_tag_db_id == recipient_tag_id
            })
            .map(|row| row.object)
            .collect::<Vec<_>>();
        if let [object] = matches.as_slice() {
            if !open.dispatched_objects.insert(*object) {
                open.poisoned = true;
            }
        } else {
            open.poisoned = true;
        }
    });
}

/// Fail closed when the shared d75 detour observed a dispatch but its bridge
/// could not authenticate the raw arguments. Without this notification, an
/// executed delayed event could be mislabeled as a false-trigger consumption.
pub fn observe_dispatch_unavailable() {
    OPEN_CALL.with(|slot| {
        if let Some(open) = slot.borrow_mut().as_mut() {
            if matches!(open.kind, CallKind::Hourly) {
                open.poisoned = true;
            }
        }
    });
}

pub fn install_fixture_hooks() -> Result<InstalledHookAddresses, String> {
    if !evidence_activation_enabled() {
        return Err(format!(
            "A19 evidence hooks require {EVIDENCE_ACTIVATION_ENV}=1"
        ));
    }
    install_hooks()
}

pub fn install_production_hooks() -> Result<InstalledHookAddresses, String> {
    if !PRODUCTION_SCHEDULING_ENABLED {
        return Err("A19 lifecycle production scheduling is not promoted".into());
    }
    install_hooks()
}

fn install_hooks() -> Result<InstalledHookAddresses, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A19 install lock poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        reobserve_installed_hook_receipts()?;
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    require_receipt_slot_available_for_install()?;
    if !DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire) {
        return Err("A19 shared d75 dispatch observer is not registered".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A19 GetModuleHandleW(NULL) returned null".into());
    }
    let sites = addresses(base);
    verify_sentinel(sites.hourly_update, &HOURLY_UPDATE_SENTINEL)?;
    verify_sentinel(sites.definition_cancel, &DEFINITION_CANCEL_SENTINEL)?;
    let epoch = NEXT_HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    if epoch == 0 {
        return Err("A19 hook epoch exhausted".into());
    }
    reset_state(epoch)?;

    let original_hourly = unsafe {
        MinHook::create_hook(
            sites.hourly_update as *mut c_void,
            hook_hourly_update as *mut c_void,
        )
    }
    .map_err(|status| {
        clear_hook_state();
        format!("A19 hourly hook create failed: {status}")
    })?;
    ORIGINAL_HOURLY_UPDATE.store(original_hourly as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);

    let original_cancel = match unsafe {
        MinHook::create_hook(
            sites.definition_cancel as *mut c_void,
            hook_definition_cancel as *mut c_void,
        )
    } {
        Ok(original) => original,
        Err(status) => {
            if let Err(cleanup) = cleanup_hooks(sites) {
                INSTALLED.store(true, Ordering::Release);
                return Err(format!(
                    "A19 cancellation hook create failed: {status}; rollback failed: {cleanup}"
                ));
            }
            clear_hook_state();
            return Err(format!("A19 cancellation hook create failed: {status}"));
        }
    };
    ORIGINAL_DEFINITION_CANCEL.store(original_cancel as usize, Ordering::Release);
    for target in [sites.definition_cancel, sites.hourly_update] {
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            if let Err(cleanup) = cleanup_hooks(sites) {
                INSTALLED.store(true, Ordering::Release);
                return Err(format!(
                    "A19 hook enable failed at 0x{target:X}: {status}; rollback failed: {cleanup}"
                ));
            }
            clear_hook_state();
            return Err(format!("A19 hook enable failed at 0x{target:X}: {status}"));
        }
    }
    if let Err(error) = observe_install_receipts(sites) {
        if let Err(cleanup) = cleanup_hooks(sites) {
            INSTALLED.store(true, Ordering::Release);
            return Err(format!(
                "A19 exact installed-hook receipt failed: {error}; rollback failed: {cleanup}"
            ));
        }
        if let Err(release) = settle_failed_install_receipts() {
            return Err(format!(
                "A19 exact installed-hook receipt failed: {error}; rollback release pending: {release}"
            ));
        }
        clear_hook_state();
        return Err(format!("A19 exact installed-hook receipt failed: {error}"));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; 2], String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A19 installed-hook receipt lifecycle lock poisoned")?;
    reobserve_installed_hook_receipts()
}

pub fn register_d75_terminal_detach_authority() -> Result<D64DetachAuthority, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A19 D64 detach-authority lifecycle lock poisoned")?;
    if !INSTALLED.load(Ordering::Acquire) || !DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire) {
        return Err(
            "A19 D64 detach authority requires installed hooks and a registered D75 observer"
                .into(),
        );
    }
    reobserve_installed_hook_receipts()?;
    let authority_id = next_detach_authority_id()?;
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("A19 production hook receipts are not initialized")?;
    let mut receipts = receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    if receipts.active_detach_authority_id.is_some()
        || receipts.terminal_custody_identity_sha256.is_some()
        || receipts.release_pending
        || receipts.released.is_some()
    {
        return Err("A19 D64 detach authority was already issued or teardown has begun".into());
    }
    receipts.active_detach_authority_id = Some(authority_id);
    Ok(D64DetachAuthority {
        authority_id,
        terminal_attempts_remaining: D64_TERMINAL_ATTEMPT_BUDGET,
    })
}

pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 2], String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A19 released-hook receipt lifecycle lock poisoned")?;
    if detach_authority_active()? {
        return Err("A19 released-hook receipts are reserved by D75 terminal authority".into());
    }
    take_released_hook_receipts_locked()
}

fn take_released_hook_receipts_locked() -> Result<[ReleasedRvaHookReceipt; 2], String> {
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("A19 production hook receipts are not initialized")?;
    let mut receipts = receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    receipts
        .released
        .take()
        .ok_or_else(|| "A19 exact released-hook receipts are unavailable".into())
}

fn released_hook_receipts_locked() -> Result<[ReleasedRvaHookReceipt; 2], String> {
    HOOK_RECEIPTS
        .get()
        .ok_or("A19 production hook receipts are not initialized")?
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?
        .released
        .ok_or_else(|| "A19 exact released-hook receipts are unavailable".into())
}

/// Compatibility-only destructive drain. Production publication must use
/// [`peek_lifecycle_drain`], then commit only after its sink transaction
/// succeeds.
pub fn drain() -> Result<LifecycleDrain, String> {
    if detach_authority_active()? {
        return Err(
            "A19 compatibility drain is disabled while D75 detach authority is active".into(),
        );
    }
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()))
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    if let Some(reservation) = queue.reservation {
        return Err(format!(
            "A19 compatibility drain blocked by {:?} reservation {}",
            reservation.owner, reservation.id
        ));
    }
    let (diagnostics, mutation_generation) = snapshot_diagnostics_and_generation()?;
    let events = if queue.pending.is_empty() {
        Vec::new()
    } else {
        let _mutation = LifecycleMutationGuard::enter();
        queue.pending.drain(..).collect::<Vec<_>>()
    };
    Ok(LifecycleDrain {
        events,
        diagnostics,
        mutation_generation,
    })
}

/// Peeks the complete current lifecycle prefix without removing it. Hook
/// callbacks may continue appending behind this immutable reservation.
pub fn peek_lifecycle_drain() -> Result<LifecycleDrainReservation, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("A19 lifecycle hooks are not installed for delivery reservation".into());
    }
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()))
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    if let Some(reservation) = queue.reservation {
        return Err(format!(
            "A19 lifecycle queue already has {:?} reservation {}",
            reservation.owner, reservation.id
        ));
    }
    let reservation_id = next_reservation_id()?;
    let row_count = queue.pending.len();
    let (diagnostics, mutation_generation) = snapshot_diagnostics_and_generation()?;
    queue.reservation = Some(QueueReservation {
        id: reservation_id,
        row_count,
        owner: ReservationOwner::Delivery,
    });
    Ok(LifecycleDrainReservation {
        reservation_id,
        drain: LifecycleDrain {
            events: queue.pending.iter().take(row_count).cloned().collect(),
            diagnostics,
            mutation_generation,
        },
    })
}

/// Commits exactly the prefix returned by [`peek_lifecycle_drain`]. Rows
/// appended concurrently after the peek remain queued in original order.
pub fn commit_lifecycle_drain(reservation_id: u64) -> Result<(), String> {
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()))
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    let reservation = matching_delivery_reservation(&queue, reservation_id)?;
    if queue.pending.len() < reservation.row_count {
        return Err(format!(
            "A19 lifecycle reservation {} lost its prefix (reserved={}, retained={})",
            reservation.id,
            reservation.row_count,
            queue.pending.len()
        ));
    }
    let _mutation = (reservation.row_count != 0).then(LifecycleMutationGuard::enter);
    queue.pending.drain(..reservation.row_count);
    queue.reservation = None;
    Ok(())
}

/// Releases one delivery reservation without removing or reordering events.
pub fn rollback_lifecycle_drain(reservation_id: u64) -> Result<(), String> {
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()))
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    matching_delivery_reservation(&queue, reservation_id)?;
    queue.reservation = None;
    Ok(())
}

pub fn lifecycle_mutation_generation() -> u64 {
    LIFECYCLE_MUTATION_GENERATION.load(Ordering::Acquire)
}

pub fn verify_lifecycle_mutation_generation_unchanged(expected: u64) -> Result<(), String> {
    let observed = lifecycle_mutation_generation();
    if expected == 0
        || expected == u64::MAX
        || expected & 1 != 0
        || observed != expected
        || ACTIVE_LIFECYCLE_MUTATIONS.load(Ordering::Acquire) != 0
    {
        return Err(format!(
            "A19 lifecycle mutation generation changed: expected {expected}, observed {observed}"
        ));
    }
    Ok(())
}

fn snapshot_diagnostics_and_generation() -> Result<(LifecycleDiagnostics, u64), String> {
    for _ in 0..10_000 {
        if ACTIVE_LIFECYCLE_MUTATIONS.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
            continue;
        }
        let before = lifecycle_mutation_generation();
        if before == 0 || before == u64::MAX || before & 1 != 0 {
            return Err("A19 lifecycle mutation generation is invalid".into());
        }
        let snapshot = diagnostics();
        let after = lifecycle_mutation_generation();
        if before == after
            && after & 1 == 0
            && ACTIVE_LIFECYCLE_MUTATIONS.load(Ordering::Acquire) == 0
        {
            return Ok((snapshot, after));
        }
        std::thread::yield_now();
    }
    Err("A19 lifecycle diagnostics did not reach a stable mutation generation".into())
}

fn next_reservation_id() -> Result<u64, String> {
    loop {
        let id = NEXT_RESERVATION_ID.load(Ordering::Acquire);
        if id == 0 || id == u64::MAX {
            return Err("A19 lifecycle reservation ID space exhausted".into());
        }
        if NEXT_RESERVATION_ID
            .compare_exchange_weak(id, id + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(id);
        }
    }
}

fn next_detach_authority_id() -> Result<u64, String> {
    loop {
        let id = NEXT_DETACH_AUTHORITY_ID.load(Ordering::Acquire);
        if id == 0 || id == u64::MAX {
            return Err("A19 D64 detach-authority ID space exhausted".into());
        }
        if NEXT_DETACH_AUTHORITY_ID
            .compare_exchange_weak(id, id + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(id);
        }
    }
}

fn detach_authority_active() -> Result<bool, String> {
    let Some(receipts) = HOOK_RECEIPTS.get() else {
        return Ok(false);
    };
    Ok(receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?
        .active_detach_authority_id
        .is_some())
}

fn matching_delivery_reservation(
    queue: &ReservableLifecycleQueue,
    reservation_id: u64,
) -> Result<QueueReservation, String> {
    match queue.reservation {
        Some(reservation)
            if reservation.id == reservation_id
                && reservation.owner == ReservationOwner::Delivery =>
        {
            Ok(reservation)
        }
        Some(reservation) if reservation.owner == ReservationOwner::Teardown => {
            Err("A19 lifecycle queue is sealed by teardown custody".into())
        }
        Some(reservation) => Err(format!(
            "A19 lifecycle reservation mismatch: active={}, requested={reservation_id}",
            reservation.id
        )),
        None => Err(format!(
            "A19 lifecycle queue has no active reservation {reservation_id}"
        )),
    }
}

fn begin_teardown_queue_custody() -> Result<(), String> {
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()))
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    match queue.reservation {
        Some(reservation) if reservation.owner == ReservationOwner::Teardown => Ok(()),
        Some(reservation) => Err(format!(
            "A19 teardown blocked by live lifecycle delivery reservation {}",
            reservation.id
        )),
        None => {
            queue.reservation = Some(QueueReservation {
                id: next_reservation_id()?,
                row_count: 0,
                owner: ReservationOwner::Teardown,
            });
            Ok(())
        }
    }
}

fn teardown_queue_owned() -> Result<bool, String> {
    let Some(queue) = QUEUE.get() else {
        return Ok(false);
    };
    let queue = queue
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    Ok(queue
        .reservation
        .is_some_and(|reservation| reservation.owner == ReservationOwner::Teardown))
}

fn peek_teardown_queue() -> Result<LifecycleDrain, String> {
    let queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()))
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    queue
        .reservation
        .filter(|reservation| reservation.owner == ReservationOwner::Teardown)
        .ok_or("A19 lifecycle queue lacks teardown custody")?;
    let (diagnostics, mutation_generation) = snapshot_diagnostics_and_generation()?;
    Ok(LifecycleDrain {
        events: queue.pending.iter().cloned().collect(),
        diagnostics,
        mutation_generation,
    })
}

fn drain_teardown_queue() -> Result<LifecycleDrain, String> {
    let mut queue = QUEUE
        .get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()))
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    let reservation = queue
        .reservation
        .filter(|reservation| reservation.owner == ReservationOwner::Teardown)
        .ok_or("A19 lifecycle queue lacks teardown custody")?;
    let (diagnostics, mutation_generation) = snapshot_diagnostics_and_generation()?;
    let events = if queue.pending.is_empty() {
        Vec::new()
    } else {
        let _mutation = LifecycleMutationGuard::enter();
        queue.pending.drain(..).collect::<Vec<_>>()
    };
    queue.reservation = None;
    let _ = reservation;
    Ok(LifecycleDrain {
        events,
        diagnostics,
        mutation_generation,
    })
}

pub fn seal_drain_and_uninstall() -> Result<LifecycleDrain, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A19 teardown lock poisoned")?;
    if detach_authority_active()? {
        return Err("A19 D64 teardown requires the issued D75 detach authority".into());
    }
    seal_drain_and_uninstall_locked()
}

/// Seals, drains and removes the D64 hook pair for a production hook registry
/// that could not complete its install, discarding the drained lifecycle
/// events instead of publishing them.
///
/// The D75 producer edge must already be closed: the caller unregisters the
/// A19 dispatch observer and releases the XUI21 semantic hooks first, so this
/// drain cannot race a live synchronous dispatch. If a D75 terminal detach
/// authority was ever issued, this refuses — that capture owns the transition
/// and must finish it through the authenticated terminal or abort path.
pub fn abort_partial_install_and_uninstall() -> Result<InstallAbortDiscard, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A19 teardown lock poisoned")?;
    if detach_authority_active()? {
        return Err(
            "A19 D64 install abort refuses an issued D75 detach authority; the authenticated \
             terminal custody owns this transition"
                .into(),
        );
    }
    if DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire) {
        return Err(
            "A19 D64 install abort requires the D75 dispatch observer to be unregistered first"
                .into(),
        );
    }
    let drain = seal_drain_and_uninstall_locked()?;
    Ok(InstallAbortDiscard {
        retained_rows: drain.events.len() as u64,
        reserved_rows: 0,
        discarded_diagnostics: drain
            .diagnostics
            .dropped_reentrant
            .saturating_add(drain.diagnostics.dropped_unreadable)
            .saturating_add(drain.diagnostics.dropped_correlation)
            .saturating_add(drain.diagnostics.dropped_overflow)
            .saturating_add(drain.diagnostics.dropped_lock_busy),
        poisoned: drain.diagnostics.completeness_poisoned,
    })
}

fn seal_drain_and_uninstall_locked() -> Result<LifecycleDrain, String> {
    let Some(teardown_owned) = prepare_teardown_locked()? else {
        return drain();
    };
    let drained = if teardown_owned {
        drain_teardown_queue()
    } else {
        drain()
    };
    clear_hook_state();
    drained
}

fn prepare_teardown_locked() -> Result<Option<bool>, String> {
    let was_installed = INSTALLED.load(Ordering::Acquire);
    let mut release_pending = release_pending()?;
    let mut teardown_owned = teardown_queue_owned()?;
    if !was_installed && !release_pending && !teardown_owned {
        return Ok(None);
    }
    if release_pending && !teardown_owned {
        begin_teardown_queue_custody()?;
        teardown_owned = true;
    }
    if was_installed {
        begin_teardown_queue_custody()?;
        teardown_owned = true;
        let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
        cleanup_hooks(sites)?;
        // From this point the exact original prefixes are live again. Never
        // claim that the detours remain installed, even if receipt
        // materialization encounters a transient module-read failure.
        INSTALLED.store(false, Ordering::Release);
        release_pending = begin_release_pending_if_owned()?;
    }
    if release_pending {
        observe_pending_release_receipts()?;
    }
    Ok(Some(teardown_owned))
}

pub fn seal_drain_and_uninstall_for_d75_terminal_custody(
    mut authority: D64DetachAuthority,
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<AuthenticatedD64TerminalDrainReceipt, D64TerminalSealFailure> {
    if authority.terminal_attempts_remaining == 0 {
        return Err(D64TerminalSealFailure {
            message: "A19 D64 terminal retry budget is exhausted; authenticated abort is required"
                .into(),
            authority,
        });
    }
    authority.terminal_attempts_remaining -= 1;
    match seal_drain_and_uninstall_for_d75_terminal_custody_inner(
        authority.authority_id,
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
    ) {
        Ok(receipt) => Ok(receipt),
        Err(message) => Err(D64TerminalSealFailure { message, authority }),
    }
}

fn seal_drain_and_uninstall_for_d75_terminal_custody_inner(
    authority_id: u64,
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<AuthenticatedD64TerminalDrainReceipt, String> {
    if custody_identity_sha256 == [0; 32] || frame_hour == 0 {
        return Err("A19 terminal D64 custody identity or frame hour is invalid".into());
    }
    #[cfg(feature = "test-support")]
    if authority_id == u64::MAX {
        return mint_test_terminal_drain_receipt(
            custody_identity_sha256,
            frame_hour,
            elapsed_wall_ms,
        );
    }
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A19 terminal D64 custody lifecycle lock poisoned")?;
    validate_detach_authority_id(authority_id)?;
    // Seal delivery before detaching the D75 bridge. A live delivery
    // reservation therefore cannot strand enabled D64 hooks without their
    // synchronous dispatch observer.
    begin_teardown_queue_custody()?;
    authorize_terminal_dispatch_detach(authority_id, custody_identity_sha256)?;
    let teardown_owned =
        prepare_teardown_locked()?.ok_or("A19 terminal D64 custody has no active hook teardown")?;
    if !teardown_owned {
        return Err("A19 terminal D64 custody does not own the sealed lifecycle queue".into());
    }

    let drain = peek_teardown_queue()?;
    validate_terminal_drain(&drain)?;
    let released_hooks = released_hook_receipts_locked()?;
    validate_terminal_release_receipts(&released_hooks)?;
    let payload = encode_terminal_lifecycle_envelope(
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        &drain,
        &released_hooks,
    )?;
    let payload_sha256 = Sha256::digest(&payload).into();
    let terminal_del2 = encode_terminal_del2(&drain);
    let terminal_del2_sha256 = Sha256::digest(&terminal_del2).into();

    let (committed, taken_releases) = commit_terminal_custody(
        authority_id,
        custody_identity_sha256,
        &drain,
        &released_hooks,
    )?;
    clear_hook_state();
    Ok(AuthenticatedD64TerminalDrainReceipt {
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        payload,
        payload_sha256,
        terminal_del2,
        terminal_del2_sha256,
        drain: committed,
        released_hooks: taken_releases,
    })
}

pub fn abort_d75_terminal_custody(
    authority: D64DetachAuthority,
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    reason_sha256: [u8; 32],
) -> Result<AuthenticatedPoisonedD64AbortReceipt, D64TerminalAbortFailure> {
    match abort_d75_terminal_custody_inner(
        authority.authority_id,
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        reason_sha256,
        false,
    ) {
        Ok(receipt) => Ok(receipt),
        Err(message) => Err(D64TerminalAbortFailure { message, authority }),
    }
}

pub fn cancel_unbound_d75_terminal_detach_authority(
    authority: D64DetachAuthority,
    reason_sha256: [u8; 32],
) -> Result<AuthenticatedPoisonedD64AbortReceipt, D64TerminalAbortFailure> {
    let mut identity = Sha256::new();
    identity.update(b"battle-plan.d64.unbound_detach_cancellation.v1\0");
    identity.update(authority.authority_id.to_le_bytes());
    identity.update(ACTIVE_HOOK_EPOCH.load(Ordering::Acquire).to_le_bytes());
    identity.update(reason_sha256);
    let custody_identity_sha256 = identity.finalize().into();
    match abort_d75_terminal_custody_inner(
        authority.authority_id,
        custody_identity_sha256,
        0,
        0,
        reason_sha256,
        true,
    ) {
        Ok(receipt) => Ok(receipt),
        Err(message) => Err(D64TerminalAbortFailure { message, authority }),
    }
}

fn abort_d75_terminal_custody_inner(
    authority_id: u64,
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    reason_sha256: [u8; 32],
    preterminal_cancellation: bool,
) -> Result<AuthenticatedPoisonedD64AbortReceipt, String> {
    if custody_identity_sha256 == [0; 32]
        || (!preterminal_cancellation && frame_hour == 0)
        || reason_sha256 == [0; 32]
    {
        return Err("A19 poisoned D64 abort identity, frame hour, or reason is invalid".into());
    }
    #[cfg(feature = "test-support")]
    if authority_id == u64::MAX {
        return mint_test_poisoned_abort_receipt(
            custody_identity_sha256,
            frame_hour,
            elapsed_wall_ms,
            reason_sha256,
        );
    }
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A19 poisoned D64 abort lifecycle lock poisoned")?;
    validate_detach_authority_id(authority_id)?;
    begin_teardown_queue_custody()?;
    authorize_terminal_dispatch_detach(authority_id, custody_identity_sha256)?;
    let teardown_owned =
        prepare_teardown_locked()?.ok_or("A19 poisoned D64 abort has no active hook teardown")?;
    if !teardown_owned {
        return Err("A19 poisoned D64 abort does not own the sealed lifecycle queue".into());
    }

    let drain = peek_teardown_queue()?;
    let released_hooks = released_hook_receipts_locked()?;
    validate_terminal_release_receipts(&released_hooks)?;
    let payload = encode_poisoned_abort_envelope(
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        reason_sha256,
        &drain,
        &released_hooks,
    )?;
    let payload_sha256 = Sha256::digest(&payload).into();
    let (committed, taken_releases) = commit_terminal_custody(
        authority_id,
        custody_identity_sha256,
        &drain,
        &released_hooks,
    )?;
    clear_hook_state();
    Ok(AuthenticatedPoisonedD64AbortReceipt {
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        reason_sha256,
        payload,
        payload_sha256,
        drain: committed,
        released_hooks: taken_releases,
    })
}

fn validate_detach_authority_id(authority_id: u64) -> Result<(), String> {
    if authority_id == 0 {
        return Err("A19 D64 detach authority is invalid".into());
    }
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("A19 production hook receipts are not initialized")?
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    if receipts.active_detach_authority_id != Some(authority_id) {
        return Err("A19 D64 detach authority is stale, foreign, or already consumed".into());
    }
    Ok(())
}

fn authorize_terminal_dispatch_detach(
    authority_id: u64,
    custody_identity_sha256: [u8; 32],
) -> Result<(), String> {
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let mut receipts = receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    if receipts.active_detach_authority_id != Some(authority_id) {
        return Err("A19 terminal D64 detach authority changed".into());
    }
    match receipts.terminal_custody_identity_sha256 {
        Some(existing) if existing != custody_identity_sha256 => {
            return Err("A19 terminal D64 custody identity changed".into())
        }
        Some(_) => {
            if DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire) || !teardown_queue_owned()? {
                return Err(
                    "A19 terminal D64 custody replay is not in a sealed retry state".into(),
                );
            }
            return Ok(());
        }
        None => {}
    }
    if DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire) {
        let _mutation = LifecycleMutationGuard::enter();
        DISPATCH_OBSERVER_REGISTERED
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "A19 terminal D64 dispatch observer detach raced")?;
    } else if !teardown_queue_owned()? {
        return Err("A19 terminal D64 dispatch observer detached without sealed custody".into());
    }
    receipts.terminal_custody_identity_sha256 = Some(custody_identity_sha256);
    Ok(())
}

fn commit_terminal_custody(
    authority_id: u64,
    custody_identity_sha256: [u8; 32],
    expected_drain: &LifecycleDrain,
    expected_releases: &[ReleasedRvaHookReceipt; 2],
) -> Result<(LifecycleDrain, [ReleasedRvaHookReceipt; 2]), String> {
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("A19 production hook receipts are not initialized")?;
    let mut receipts = receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    if receipts.active_detach_authority_id != Some(authority_id) {
        return Err("A19 terminal D64 detach authority was lost before commit".into());
    }
    if receipts.terminal_custody_identity_sha256 != Some(custody_identity_sha256) {
        return Err("A19 terminal D64 custody identity was lost before commit".into());
    }
    if receipts.release_pending
        || receipts.release_full_roster
        || receipts.installed.is_some()
        || !receipts.installing.is_empty()
        || !receipts.releasing.is_empty()
    {
        return Err("A19 terminal D64 hook receipt release is incomplete".into());
    }
    let released = receipts
        .released
        .ok_or("A19 exact released-hook receipts are unavailable")?;
    if &released != expected_releases {
        return Err("A19 terminal D64 released-hook custody changed before commit".into());
    }

    let mut queue = QUEUE
        .get()
        .ok_or("A19 lifecycle queue is not initialized")?
        .lock()
        .map_err(|_| "A19 lifecycle queue lock poisoned")?;
    queue
        .reservation
        .filter(|reservation| reservation.owner == ReservationOwner::Teardown)
        .ok_or("A19 lifecycle queue lacks teardown custody")?;
    let (diagnostics, mutation_generation) = snapshot_diagnostics_and_generation()?;
    let observed_drain = LifecycleDrain {
        events: queue.pending.iter().cloned().collect(),
        diagnostics,
        mutation_generation,
    };
    if &observed_drain != expected_drain {
        return Err("A19 terminal D64 queue changed under sealed custody".into());
    }

    let _mutation = (!queue.pending.is_empty()).then(LifecycleMutationGuard::enter);
    queue.pending.clear();
    queue.reservation = None;
    receipts.released = None;
    receipts.active_detach_authority_id = None;
    receipts.terminal_custody_identity_sha256 = None;
    Ok((observed_drain, released))
}

fn terminal_diagnostics_clean(diagnostics: &LifecycleDiagnostics) -> bool {
    diagnostics.hook_epoch != 0
        && !diagnostics.dispatch_observer_registered
        && !diagnostics.completeness_poisoned
        && diagnostics.dropped_reentrant == 0
        && diagnostics.dropped_unreadable == 0
        && diagnostics.dropped_correlation == 0
        && diagnostics.dropped_overflow == 0
        && diagnostics.dropped_lock_busy == 0
}

fn validate_terminal_drain(drain: &LifecycleDrain) -> Result<(), String> {
    if !terminal_diagnostics_clean(&drain.diagnostics) {
        return Err(
            "A19 terminal D64 diagnostics are incomplete, poisoned, dropped, or not detached"
                .into(),
        );
    }
    if drain.mutation_generation == 0 || drain.mutation_generation == u64::MAX {
        return Err("A19 terminal D64 mutation generation is invalid".into());
    }
    if drain.events.windows(2).any(|rows| {
        rows[0]
            .sequence
            .checked_add(1)
            .map_or(true, |expected| rows[1].sequence != expected)
    }) || drain.events.iter().any(|row| {
        row.sequence == 0
            || row.game_hour_raw == i32::MIN
            || row.thread_id == 0
            || !valid_tag(row.owner_tag_db_id)
            || !valid_tag(row.originator_tag_db_id)
            || !(MIN_COMBINED_EVENT_ID..=MAX_COMBINED_EVENT_ID).contains(&row.event_id)
            || row.due_before > MAX_DUE_HOURS
            || row.due_after.is_some_and(|due| due > MAX_DUE_HOURS)
    }) {
        return Err("A19 terminal D64 lifecycle event sequence or fields are invalid".into());
    }
    Ok(())
}

fn validate_terminal_release_receipts(
    released: &[ReleasedRvaHookReceipt; 2],
) -> Result<(), String> {
    for (expected, released) in EXPECTED_HOOKS.into_iter().zip(released) {
        let expected_prefix_sha256: [u8; 32] = Sha256::digest(expected.original_prefix).into();
        if !released.released
            || released.logical_name != expected.logical_name
            || released.observed_module_name != "hoi4.exe"
            || released.observed_module_sha256 != production_hook_receipt::A729_EXE_SHA256
            || released.observed_pe_identity_sha256 == [0; 32]
            || released.observed_pe_timestamp == 0
            || released.observed_size_of_image == 0
            || released.observed_rva != expected.requested_rva
            || released.target_patch_len != expected.target_patch_len
            || released.primitive_id != expected.primitive_id
            || released.hook_spec_sha256 == [0; 32]
            || released.original_prefix_len != expected.original_prefix.len() as u16
            || released.original_prefix_sha256 != expected_prefix_sha256
            || released.teardown_owner_token == 0
            || released.restored_prefix_sha256 != expected_prefix_sha256
            || !released.callbacks_quiesced
            || !released.hook_removed
            || !released.relay_freed
        {
            return Err(format!(
                "{} terminal release receipt is incomplete",
                expected.logical_name
            ));
        }
    }
    Ok(())
}

fn encode_terminal_lifecycle_envelope(
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    drain: &LifecycleDrain,
    released_hooks: &[ReleasedRvaHookReceipt; 2],
) -> Result<Vec<u8>, String> {
    let event_count = u32::try_from(drain.events.len())
        .map_err(|_| "A19 terminal D64 lifecycle event count exceeds u32")?;
    let mut payload = Vec::new();
    payload.extend_from_slice(D64_TERMINAL_LIFECYCLE_SCHEMA.as_bytes());
    payload.push(0);
    payload.extend_from_slice(&custody_identity_sha256);
    payload.extend_from_slice(&frame_hour.to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&drain.diagnostics.hook_epoch.to_le_bytes());
    payload.extend_from_slice(&drain.mutation_generation.to_le_bytes());
    payload.push(u8::from(drain.diagnostics.dispatch_observer_registered));
    payload.push(u8::from(!drain.diagnostics.dispatch_observer_registered));
    payload.push(u8::from(drain.diagnostics.completeness_poisoned));
    payload.extend_from_slice(&[0; 5]);
    payload.extend_from_slice(&drain.diagnostics.dropped_reentrant.to_le_bytes());
    payload.extend_from_slice(&drain.diagnostics.dropped_unreadable.to_le_bytes());
    payload.extend_from_slice(&drain.diagnostics.dropped_correlation.to_le_bytes());
    payload.extend_from_slice(&drain.diagnostics.dropped_overflow.to_le_bytes());
    payload.extend_from_slice(&drain.diagnostics.dropped_lock_busy.to_le_bytes());
    payload.extend_from_slice(&event_count.to_le_bytes());
    payload.extend_from_slice(&(released_hooks.len() as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    for row in &drain.events {
        payload.extend_from_slice(&row.sequence.to_le_bytes());
        payload.extend_from_slice(&row.game_hour_raw.to_le_bytes());
        payload.extend_from_slice(&row.thread_id.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.event_id.to_le_bytes());
        payload.extend_from_slice(&row.originator_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&(row.reason as u32).to_le_bytes());
        payload.extend_from_slice(&row.scope_random.to_le_bytes());
        payload.extend_from_slice(&row.due_before.to_le_bytes());
        payload.push(u8::from(row.due_after.is_some()));
        payload.extend_from_slice(&[0; 3]);
        payload.extend_from_slice(&row.due_after.unwrap_or(0).to_le_bytes());
    }
    for released in released_hooks {
        push_terminal_string(&mut payload, released.logical_name)?;
        push_terminal_string(&mut payload, released.observed_module_name)?;
        payload.extend_from_slice(&released.observed_module_sha256);
        payload.extend_from_slice(&released.observed_pe_identity_sha256);
        payload.extend_from_slice(&released.observed_pe_timestamp.to_le_bytes());
        payload.extend_from_slice(&released.observed_size_of_image.to_le_bytes());
        payload.extend_from_slice(&released.observed_rva.to_le_bytes());
        payload.extend_from_slice(&released.target_patch_len.to_le_bytes());
        push_terminal_string(&mut payload, released.primitive_id)?;
        payload.extend_from_slice(&released.hook_spec_sha256);
        payload.extend_from_slice(&released.original_prefix_len.to_le_bytes());
        payload.extend_from_slice(&released.original_prefix_sha256);
        payload.extend_from_slice(&released.teardown_owner_token.to_le_bytes());
        payload.extend_from_slice(&released.restored_prefix_sha256);
        payload.push(u8::from(released.callbacks_quiesced));
        payload.push(u8::from(released.hook_removed));
        payload.push(u8::from(released.relay_freed));
        payload.push(u8::from(released.released));
    }
    payload.push(1);
    Ok(payload)
}

#[cfg(feature = "test-support")]
fn test_terminal_drain_and_releases() -> (LifecycleDrain, [ReleasedRvaHookReceipt; 2]) {
    let drain = LifecycleDrain {
        events: Vec::new(),
        diagnostics: LifecycleDiagnostics {
            hook_epoch: 1,
            dispatch_observer_registered: false,
            completeness_poisoned: false,
            dropped_reentrant: 0,
            dropped_unreadable: 0,
            dropped_correlation: 0,
            dropped_overflow: 0,
            dropped_lock_busy: 0,
        },
        mutation_generation: 2,
    };
    let owner = u64::MAX - 1;
    let releases = EXPECTED_HOOKS
        .map(|expected| production_hook_receipt::mint_test_released_receipt(expected, owner));
    (drain, releases)
}

#[cfg(feature = "test-support")]
fn mint_test_terminal_drain_receipt(
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<AuthenticatedD64TerminalDrainReceipt, String> {
    let (drain, released_hooks) = test_terminal_drain_and_releases();
    validate_terminal_drain(&drain)?;
    validate_terminal_release_receipts(&released_hooks)?;
    let payload = encode_terminal_lifecycle_envelope(
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        &drain,
        &released_hooks,
    )?;
    let payload_sha256 = Sha256::digest(&payload).into();
    let terminal_del2 = encode_terminal_del2(&drain);
    let terminal_del2_sha256 = Sha256::digest(&terminal_del2).into();
    Ok(AuthenticatedD64TerminalDrainReceipt {
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        payload,
        payload_sha256,
        terminal_del2,
        terminal_del2_sha256,
        drain,
        released_hooks,
    })
}

#[cfg(feature = "test-support")]
fn mint_test_poisoned_abort_receipt(
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    reason_sha256: [u8; 32],
) -> Result<AuthenticatedPoisonedD64AbortReceipt, String> {
    let (drain, released_hooks) = test_terminal_drain_and_releases();
    validate_terminal_release_receipts(&released_hooks)?;
    let payload = encode_poisoned_abort_envelope(
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        reason_sha256,
        &drain,
        &released_hooks,
    )?;
    let payload_sha256 = Sha256::digest(&payload).into();
    Ok(AuthenticatedPoisonedD64AbortReceipt {
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        reason_sha256,
        payload,
        payload_sha256,
        drain,
        released_hooks,
    })
}

/// Encode the canonical DEL2 suffix used by the terminal D64 DATA record.
/// This intentionally mirrors the stable wire contract consumed by
/// `wire_format::d64`; terminal publication authenticates these exact bytes.
fn encode_terminal_del2(drain: &LifecycleDrain) -> Vec<u8> {
    const MAGIC: &[u8; 4] = b"DEL2";
    const VERSION: u16 = 1;
    const HEADER_LEN: u16 = 48;
    const ROW_LEN: u16 = 48;
    const FLAG_COMPLETE: u32 = 1 << 0;
    const ROW_FLAG_DUE_AFTER: u32 = 1 << 0;

    let complete = drain.diagnostics.hook_epoch != 0
        && !drain.diagnostics.dispatch_observer_registered
        && !drain.diagnostics.completeness_poisoned
        && drain.diagnostics.dropped_reentrant == 0
        && drain.diagnostics.dropped_unreadable == 0
        && drain.diagnostics.dropped_correlation == 0
        && drain.diagnostics.dropped_overflow == 0
        && drain.diagnostics.dropped_lock_busy == 0;
    let first_sequence = drain.events.first().map(|row| row.sequence).unwrap_or(0);
    let last_sequence = drain.events.last().map(|row| row.sequence).unwrap_or(0);
    let mut payload = Vec::with_capacity(
        usize::from(HEADER_LEN)
            .saturating_add(drain.events.len().saturating_mul(usize::from(ROW_LEN))),
    );
    payload.extend_from_slice(MAGIC);
    payload.extend_from_slice(&VERSION.to_le_bytes());
    payload.extend_from_slice(&HEADER_LEN.to_le_bytes());
    payload.extend_from_slice(&ROW_LEN.to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&(if complete { FLAG_COMPLETE } else { 0 }).to_le_bytes());
    payload.extend_from_slice(&(drain.events.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&drain.diagnostics.hook_epoch.to_le_bytes());
    payload.extend_from_slice(&first_sequence.to_le_bytes());
    payload.extend_from_slice(&last_sequence.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in &drain.events {
        payload.extend_from_slice(&row.sequence.to_le_bytes());
        payload.extend_from_slice(&row.game_hour_raw.to_le_bytes());
        payload.extend_from_slice(&(row.reason as u32).to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.event_id.to_le_bytes());
        payload.extend_from_slice(&row.originator_tag_db_id.to_le_bytes());
        payload.extend_from_slice(
            &(if row.due_after.is_some() {
                ROW_FLAG_DUE_AFTER
            } else {
                0
            })
            .to_le_bytes(),
        );
        payload.extend_from_slice(&row.scope_random.to_le_bytes());
        payload.extend_from_slice(&row.due_before.to_le_bytes());
        payload.extend_from_slice(&row.due_after.unwrap_or(0).to_le_bytes());
    }
    payload
}

fn encode_poisoned_abort_envelope(
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    reason_sha256: [u8; 32],
    drain: &LifecycleDrain,
    released_hooks: &[ReleasedRvaHookReceipt; 2],
) -> Result<Vec<u8>, String> {
    let lifecycle = encode_terminal_lifecycle_envelope(
        custody_identity_sha256,
        frame_hour,
        elapsed_wall_ms,
        drain,
        released_hooks,
    )?;
    let lifecycle_len = u32::try_from(lifecycle.len())
        .map_err(|_| "A19 poisoned D64 abort lifecycle envelope exceeds u32")?;
    let mut payload = Vec::new();
    payload.extend_from_slice(D64_POISONED_ABORT_SCHEMA.as_bytes());
    payload.push(0);
    payload.extend_from_slice(&custody_identity_sha256);
    payload.extend_from_slice(&frame_hour.to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&reason_sha256);
    payload.extend_from_slice(&lifecycle_len.to_le_bytes());
    payload.extend_from_slice(&lifecycle);
    payload.push(1); // Capture completeness is irrevocably poisoned.
    payload.push(1); // Queue and exact hook teardown custody are complete.
    Ok(payload)
}

fn push_terminal_string(payload: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let length =
        u16::try_from(value.len()).map_err(|_| "A19 terminal D64 envelope string exceeds u16")?;
    payload.extend_from_slice(&length.to_le_bytes());
    payload.extend_from_slice(value.as_bytes());
    Ok(())
}

unsafe extern "system" fn hook_hourly_update(country: usize) {
    let _active = CallbackGuard::enter();
    let original_address = ORIGINAL_HOURLY_UPDATE.load(Ordering::Acquire);
    if original_address == 0 {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let original: HourlyUpdateFn = unsafe { mem::transmute(original_address) };
    if OPEN_CALL.with(|slot| slot.borrow().is_some()) {
        poison(&DROPPED_REENTRANT);
        unsafe { original(country) };
        return;
    }
    let before = snapshot_country(country);
    match before {
        Some((owner_tag_db_id, rows)) => OPEN_CALL.with(|slot| {
            *slot.borrow_mut() = Some(OpenCall {
                kind: CallKind::Hourly,
                owner_tag_db_id,
                before: rows,
                dispatched_objects: BTreeSet::new(),
                poisoned: !DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire),
            })
        }),
        None => poison(&DROPPED_UNREADABLE),
    }
    unsafe { original(country) };
    let open = OPEN_CALL.with(|slot| slot.borrow_mut().take());
    let after = snapshot_country(country);
    let Some(open) = open else { return };
    let Some((after_owner, after_rows)) = after else {
        poison(&DROPPED_UNREADABLE);
        return;
    };
    if open.poisoned || after_owner != open.owner_tag_db_id {
        poison(&DROPPED_CORRELATION);
        return;
    }
    retain_hourly(open, after_rows);
}

unsafe extern "system" fn hook_definition_cancel(country: usize, event: usize) {
    let _active = CallbackGuard::enter();
    let original_address = ORIGINAL_DEFINITION_CANCEL.load(Ordering::Acquire);
    if original_address == 0 {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let original: DefinitionCancelFn = unsafe { mem::transmute(original_address) };
    if OPEN_CALL.with(|slot| slot.borrow().is_some()) {
        poison(&DROPPED_REENTRANT);
        unsafe { original(country, event) };
        return;
    }
    let before = snapshot_country(country);
    match before {
        Some((owner_tag_db_id, rows)) => OPEN_CALL.with(|slot| {
            *slot.borrow_mut() = Some(OpenCall {
                kind: CallKind::DefinitionCancel { event },
                owner_tag_db_id,
                before: rows,
                dispatched_objects: BTreeSet::new(),
                poisoned: false,
            })
        }),
        None => poison(&DROPPED_UNREADABLE),
    }
    unsafe { original(country, event) };
    let open = OPEN_CALL.with(|slot| slot.borrow_mut().take());
    let after = snapshot_country(country);
    let (Some(open), Some((after_owner, after_rows))) = (open, after) else {
        poison(&DROPPED_UNREADABLE);
        return;
    };
    if open.poisoned || after_owner != open.owner_tag_db_id {
        poison(&DROPPED_CORRELATION);
        return;
    }
    retain_cancellations(open, after_rows);
}

fn retain_hourly(open: OpenCall, after: Vec<SnapshotRow>) {
    let after_by_object = after
        .iter()
        .map(|row| (row.object, row))
        .collect::<BTreeMap<_, _>>();
    let mut emitted = Vec::with_capacity(open.before.len());
    for before in &open.before {
        if let Some(after) = after_by_object.get(&before.object) {
            if !same_identity(before, after) || before.due.checked_sub(1) != Some(after.due) {
                poison(&DROPPED_CORRELATION);
                return;
            }
            emitted.push(event_from(
                before,
                Some(after.due),
                LifecycleReason::RetainedDecrement,
            ));
        } else {
            let reason = if open.dispatched_objects.contains(&before.object) {
                LifecycleReason::Executed
            } else {
                LifecycleReason::FailedTriggerConsumed
            };
            emitted.push(event_from(before, None, reason));
        }
    }
    if open
        .dispatched_objects
        .iter()
        .any(|object| after_by_object.contains_key(object))
    {
        poison(&DROPPED_CORRELATION);
        return;
    }
    retain_all(emitted);
}

fn retain_cancellations(open: OpenCall, after: Vec<SnapshotRow>) {
    let CallKind::DefinitionCancel { event } = open.kind else {
        poison(&DROPPED_CORRELATION);
        return;
    };
    let after_objects = after.iter().map(|row| row.object).collect::<BTreeSet<_>>();
    let mut emitted = Vec::new();
    for row in &open.before {
        if !after_objects.contains(&row.object) {
            if row.event != event {
                poison(&DROPPED_CORRELATION);
                return;
            }
            emitted.push(event_from(row, None, LifecycleReason::DefinitionCancelled));
        }
    }
    retain_all(emitted);
}

fn event_from(
    row: &SnapshotRow,
    due_after: Option<u32>,
    reason: LifecycleReason,
) -> LifecycleEvent {
    LifecycleEvent {
        sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel),
        game_hour_raw: current_game_hour_raw().unwrap_or(i32::MIN),
        thread_id: unsafe { GetCurrentThreadId() },
        owner_tag_db_id: row.owner_tag_db_id,
        event_id: row.event_id,
        originator_tag_db_id: row.originator_tag_db_id,
        scope_random: row.scope_random,
        due_before: row.due,
        due_after,
        reason,
    }
}

fn retain_all(events: Vec<LifecycleEvent>) {
    if events
        .iter()
        .any(|row| row.sequence == 0 || row.game_hour_raw == i32::MIN)
    {
        poison(&DROPPED_UNREADABLE);
        return;
    }
    let queue = QUEUE.get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()));
    match queue.try_lock() {
        Ok(mut queue) if queue.pending.len().saturating_add(events.len()) <= MAX_PENDING_EVENTS => {
            if !events.is_empty() {
                let _mutation = LifecycleMutationGuard::enter();
                queue.pending.extend(events);
            }
        }
        Ok(_) => poison(&DROPPED_OVERFLOW),
        Err(_) => poison(&DROPPED_LOCK_BUSY),
    }
}

fn same_identity(a: &SnapshotRow, b: &SnapshotRow) -> bool {
    a.event == b.event
        && a.event_scope == b.event_scope
        && a.owner_tag_db_id == b.owner_tag_db_id
        && a.event_id == b.event_id
        && a.originator_tag_db_id == b.originator_tag_db_id
        && a.scope_random == b.scope_random
}

fn snapshot_country(country: usize) -> Option<(u32, Vec<SnapshotRow>)> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if !heap_pointer(country)
        || read_stable::<usize>(country)? != base.checked_add(CCOUNTRY_VTABLE_RVA)?
    {
        return None;
    }
    let owner_tag_db_id = read_stable::<u32>(country.checked_add(CCOUNTRY_TAG_OFF)?)?;
    if !valid_tag(owner_tag_db_id) {
        return None;
    }
    let begin_addr = country.checked_add(CCOUNTRY_DELAYED_BEGIN_OFF)?;
    let capacity_addr = country.checked_add(CCOUNTRY_DELAYED_CAPACITY_OFF)?;
    let count_addr = country.checked_add(CCOUNTRY_DELAYED_COUNT_OFF)?;
    let header = (
        read_value::<usize>(begin_addr)?,
        read_value::<u32>(capacity_addr)?,
        read_value::<u32>(count_addr)?,
    );
    if header
        != (
            read_value::<usize>(begin_addr)?,
            read_value::<u32>(capacity_addr)?,
            read_value::<u32>(count_addr)?,
        )
        || header.2 > header.1
        || header.2 > MAX_EVENTS_PER_COUNTRY
        || (header.2 != 0 && !heap_pointer(header.0))
    {
        return None;
    }
    let mut rows = Vec::with_capacity(header.2 as usize);
    for index in 0..header.2 as usize {
        let object = read_stable::<usize>(header.0.checked_add(index.checked_mul(8)?)?)?;
        rows.push(snapshot_row(base, owner_tag_db_id, object)?);
    }
    if header
        != (
            read_value::<usize>(begin_addr)?,
            read_value::<u32>(capacity_addr)?,
            read_value::<u32>(count_addr)?,
        )
    {
        return None;
    }
    Some((owner_tag_db_id, rows))
}

fn snapshot_row(base: usize, owner_tag_db_id: u32, object: usize) -> Option<SnapshotRow> {
    if !heap_pointer(object)
        || read_stable::<usize>(object)? != base.checked_add(CDELAYED_EVENT_VTABLE_RVA)?
    {
        return None;
    }
    let event = read_stable::<usize>(object.checked_add(CDELAYED_EVENT_EVENT_OFF)?)?;
    if !heap_pointer(event) {
        return None;
    }
    let event_id = read_stable::<u32>(event.checked_add(CEVENT_COMBINED_ID_OFF)?)?;
    let scope_tag_db_id = read_stable::<u32>(object.checked_add(CDELAYED_EVENT_SCOPE_TAG_OFF)?)?;
    let originator_tag_db_id =
        read_stable::<u32>(object.checked_add(CDELAYED_EVENT_ORIGINATOR_OFF)?)?;
    let random_a = read_stable::<u32>(object.checked_add(CDELAYED_EVENT_SCOPE_RANDOM_A_OFF)?)?;
    let random_b = read_stable::<u32>(object.checked_add(CDELAYED_EVENT_SCOPE_RANDOM_B_OFF)?)?;
    let due = read_stable::<u32>(object.checked_add(CDELAYED_EVENT_DUE_OFF)?)?;
    if scope_tag_db_id != owner_tag_db_id
        || !valid_tag(originator_tag_db_id)
        || !(MIN_COMBINED_EVENT_ID..=MAX_COMBINED_EVENT_ID).contains(&event_id)
        || due > MAX_DUE_HOURS
    {
        return None;
    }
    Some(SnapshotRow {
        object,
        event,
        event_scope: object.checked_add(CDELAYED_EVENT_SCOPE_OFF)?,
        owner_tag_db_id,
        event_id,
        originator_tag_db_id,
        scope_random: u64::from(random_a) | (u64::from(random_b) << 32),
        due,
    })
}

fn current_game_hour_raw() -> Option<i32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let singleton = read_stable::<usize>(base.checked_add(CGAMESTATE_SINGLETON_RVA)?)?;
    read_stable::<i32>(singleton.checked_add(CGAMESTATE_CURRENT_DATE_OFF)?)
}

fn valid_tag(value: u32) -> bool {
    value != 0 && value <= MAX_COUNTRY_TAG_DB_ID
}

fn heap_pointer(value: usize) -> bool {
    (HEAP_LO..HEAP_HI).contains(&value)
}

fn read_stable<T: Copy + PartialEq>(address: usize) -> Option<T> {
    let first = read_value::<T>(address)?;
    (read_value::<T>(address)? == first).then_some(first)
}

fn read_value<T: Copy>(address: usize) -> Option<T> {
    let mut value = mem::MaybeUninit::<T>::uninit();
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            value.as_mut_ptr() as *mut c_void,
            mem::size_of::<T>(),
            &mut read,
        )
    };
    (ok != 0 && read == mem::size_of::<T>()).then(|| unsafe { value.assume_init() })
}

fn verify_sentinel(address: usize, expected: &[u8]) -> Result<(), String> {
    let mut observed = vec![0u8; expected.len()];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            observed.as_mut_ptr() as *mut c_void,
            observed.len(),
            &mut read,
        )
    };
    if ok == 0 || read != expected.len() || observed != expected {
        return Err(format!("A19 exact-byte sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        hourly_update: base.saturating_add(HOURLY_UPDATE_RVA),
        definition_cancel: base.saturating_add(DEFINITION_CANCEL_RVA),
    }
}

fn receipt_detours() -> [usize; 2] {
    [hook_definition_cancel as usize, hook_hourly_update as usize]
}

fn require_receipt_slot_available_for_install() -> Result<(), String> {
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let receipts = receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    if receipts.release_pending {
        return Err("A19 prior hook release receipt is still pending".into());
    }
    if receipts.released.is_some() {
        return Err("A19 prior released-hook receipt must be taken before reinstall".into());
    }
    if receipts.installed.is_some() {
        return Err("A19 prior installed-hook custody was not released".into());
    }
    if !receipts.installing.is_empty() {
        return Err("A19 partial installed-hook custody was not released".into());
    }
    if !receipts.releasing.is_empty() || receipts.release_full_roster {
        return Err("A19 prior hook release transaction was not completed".into());
    }
    if receipts.active_detach_authority_id.is_some() {
        return Err("A19 prior D64 detach authority was not consumed".into());
    }
    if receipts.terminal_custody_identity_sha256.is_some() {
        return Err("A19 prior terminal D64 custody was not consumed".into());
    }
    Ok(())
}

fn reobserve_installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; 2], String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("A19 hooks are not currently installed".into());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("A19 installed hook module base is unavailable".into());
    }
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("A19 production hook receipts are not initialized")?
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    if receipts.release_pending {
        return Err("A19 installed-hook receipt is already release-pending".into());
    }
    let installed = receipts
        .installed
        .ok_or("A19 exact installed-hook receipts are unavailable")?;
    drop(receipts);

    let sites = addresses(base);
    let targets = [sites.definition_cancel, sites.hourly_update];
    let detours = receipt_detours();
    let mut reobserved = Vec::with_capacity(EXPECTED_HOOKS.len());
    for (((expected, target), detour), installed) in EXPECTED_HOOKS
        .into_iter()
        .zip(targets)
        .zip(detours)
        .zip(installed)
    {
        let live =
            observe_enabled_minhook_rel5(expected, target, detour, installed.teardown_owner_token)?;
        if live != installed {
            return Err(format!(
                "{} live installed-hook receipt changed",
                expected.logical_name
            ));
        }
        reobserved.push(live);
    }
    reobserved
        .try_into()
        .map_err(|_| "A19 installed-hook reobservation cardinality mismatch".into())
}

fn observe_install_receipts(sites: InstalledHookAddresses) -> Result<(), String> {
    let targets = [sites.definition_cancel, sites.hourly_update];
    let detours = receipt_detours();
    let receipt_state = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    {
        let receipts = receipt_state
            .lock()
            .map_err(|_| "A19 production hook receipt mutex poisoned")?;
        if !receipts.installing.is_empty()
            || receipts.installed.is_some()
            || receipts.release_pending
            || receipts.release_full_roster
            || !receipts.releasing.is_empty()
            || receipts.released.is_some()
        {
            return Err("A19 production hook receipt slot is not empty at install".into());
        }
    }
    for ((expected, target), detour) in EXPECTED_HOOKS.into_iter().zip(targets).zip(detours) {
        let observed =
            observe_enabled_minhook_rel5(expected, target, detour, next_teardown_owner_token()?)?;
        receipt_state
            .lock()
            .map_err(|_| "A19 production hook receipt mutex poisoned")?
            .installing
            .push(observed);
    }
    let mut receipts = receipt_state
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    let observed: [InstalledRvaHookReceipt; 2] = mem::take(&mut receipts.installing)
        .try_into()
        .map_err(|_| "A19 installed-hook receipt cardinality mismatch")?;
    receipts.installed = Some(observed);
    Ok(())
}

pub fn release_pending() -> Result<bool, String> {
    let Some(receipts) = HOOK_RECEIPTS.get() else {
        return Ok(false);
    };
    let receipts = receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    Ok(receipts.release_pending)
}

fn begin_release_pending_if_owned() -> Result<bool, String> {
    let receipts = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let mut receipts = receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    if receipts.released.is_some() {
        return Err("A19 cannot begin release while an older release receipt is untaken".into());
    }
    if receipts.release_pending {
        return Ok(true);
    }
    if receipts.installed.is_none() && receipts.installing.is_empty() {
        return Ok(false);
    }
    if !receipts.releasing.is_empty() || receipts.release_full_roster {
        return Err("A19 hook release transaction state is inconsistent".into());
    }
    receipts.release_full_roster = receipts.installed.is_some();
    receipts.release_pending = true;
    Ok(true)
}

fn settle_failed_install_receipts() -> Result<(), String> {
    if begin_release_pending_if_owned()? {
        observe_pending_release_receipts()?;
    }
    Ok(())
}

fn observe_pending_release_receipts() -> Result<(), String> {
    if INSTALLED.load(Ordering::Acquire) {
        return Err("A19 cannot release receipts while hooks are marked installed".into());
    }
    let receipts = HOOK_RECEIPTS
        .get()
        .ok_or("A19 production hook receipts are not initialized")?;
    let mut receipts = receipts
        .lock()
        .map_err(|_| "A19 production hook receipt mutex poisoned")?;
    if !receipts.release_pending {
        return Err("A19 hook release is not pending".into());
    }
    if receipts.released.is_some() {
        return Err("A19 released-hook receipt slot is already occupied".into());
    }
    let attestation = HookTeardownAttestation {
        callbacks_quiesced: true,
        hook_removed: true,
        relay_freed: true,
    };
    if receipts.installed.is_some()
        && (!receipts.installing.is_empty() || !receipts.releasing.is_empty())
    {
        return Err("A19 full-roster release collided with partial receipt custody".into());
    }
    if let Some(installed) = receipts.installed.take() {
        receipts.installing.extend(installed);
    }
    if receipts.installing.is_empty() && receipts.releasing.is_empty() {
        return Err("A19 release-pending custody has no hook receipts".into());
    }
    while let Some(installed) = receipts.installing.first().copied() {
        let expected = EXPECTED_HOOKS
            .into_iter()
            .find(|expected| {
                expected.logical_name == installed.logical_name
                    && expected.requested_rva == installed.requested_rva
            })
            .ok_or("A19 installed-hook receipt is outside the D64 roster")?;
        let released = observe_released(expected, installed, attestation)?;
        receipts.installing.remove(0);
        receipts.releasing.push(released);
    }
    if receipts.release_full_roster {
        let released: [ReleasedRvaHookReceipt; 2] =
            mem::take(&mut receipts.releasing)
                .try_into()
                .map_err(|_| "A19 released-hook receipt cardinality mismatch")?;
        receipts.released = Some(released);
    } else {
        receipts.releasing.clear();
    }
    receipts.release_pending = false;
    receipts.release_full_roster = false;
    Ok(())
}

fn cleanup_hooks(sites: InstalledHookAddresses) -> Result<(), String> {
    for target in [sites.hourly_update, sites.definition_cancel] {
        match unsafe { inline_detour::disable(target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => return Err(format!("A19 hook disable failed: {status}")),
        }
    }
    wait_for_callbacks()?;
    for target in [sites.hourly_update, sites.definition_cancel] {
        match unsafe { inline_detour::remove(target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => return Err(format!("A19 hook remove failed: {status}")),
        }
    }
    verify_sentinel(sites.definition_cancel, &DEFINITION_CANCEL_SENTINEL)?;
    verify_sentinel(sites.hourly_update, &HOURLY_UPDATE_SENTINEL)?;
    Ok(())
}

fn wait_for_callbacks() -> Result<(), String> {
    for _ in 0..10_000 {
        if ACTIVE_CALLBACKS.load(Ordering::Acquire) == 0 {
            return Ok(());
        }
        std::thread::yield_now();
    }
    Err("A19 callbacks did not quiesce".into())
}

fn reset_state(epoch: u64) -> Result<(), String> {
    QUEUE
        .get_or_init(|| Mutex::new(ReservableLifecycleQueue::new()))
        .lock()
        .map_err(|_| "A19 queue lock poisoned")?
        .reset();
    ACTIVE_HOOK_EPOCH.store(epoch, Ordering::Release);
    NEXT_SEQUENCE.store(1, Ordering::Release);
    ACTIVE_LIFECYCLE_MUTATIONS.store(0, Ordering::Release);
    LIFECYCLE_MUTATION_GENERATION.store(2, Ordering::Release);
    COMPLETENESS_POISONED.store(false, Ordering::Release);
    for counter in [
        &DROPPED_REENTRANT,
        &DROPPED_UNREADABLE,
        &DROPPED_CORRELATION,
        &DROPPED_OVERFLOW,
        &DROPPED_LOCK_BUSY,
    ] {
        counter.store(0, Ordering::Release);
    }
    Ok(())
}

fn clear_hook_state() {
    ORIGINAL_HOURLY_UPDATE.store(0, Ordering::Release);
    ORIGINAL_DEFINITION_CANCEL.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    OPEN_CALL.with(|slot| *slot.borrow_mut() = None);
}

fn advance_lifecycle_mutation_generation() {
    if LIFECYCLE_MUTATION_GENERATION
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
            generation.checked_add(1)
        })
        .is_err()
    {
        COMPLETENESS_POISONED.store(true, Ordering::Release);
    }
}

fn poison(counter: &AtomicU64) {
    let _mutation = LifecycleMutationGuard::enter();
    counter.fetch_add(1, Ordering::AcqRel);
    COMPLETENESS_POISONED.store(true, Ordering::Release);
}

fn diagnostics() -> LifecycleDiagnostics {
    LifecycleDiagnostics {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        dispatch_observer_registered: DISPATCH_OBSERVER_REGISTERED.load(Ordering::Acquire),
        completeness_poisoned: COMPLETENESS_POISONED.load(Ordering::Acquire),
        dropped_reentrant: DROPPED_REENTRANT.load(Ordering::Acquire),
        dropped_unreadable: DROPPED_UNREADABLE.load(Ordering::Acquire),
        dropped_correlation: DROPPED_CORRELATION.load(Ordering::Acquire),
        dropped_overflow: DROPPED_OVERFLOW.load(Ordering::Acquire),
        dropped_lock_busy: DROPPED_LOCK_BUSY.load(Ordering::Acquire),
    }
}
