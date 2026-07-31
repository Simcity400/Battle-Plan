//! Exact-a729 A40 peace-conference transition observer.
//!
//! Exact-a729 closes the identity question negatively: peace actions are serialized value
//! objects without intrinsic allocation ids, and the terminal summary is an
//! aggregate formatter without result objects or an allocation-to-result join.
//! The observer retains bounded raw process-local witnesses for optional
//! validation and publishes only the proven portable refresh identity/scalars
//! to the production carrier. Production reduction must use complete
//! snapshots, snapshot-scoped row ids, and explicitly unjoined terminal rows;
//! it may never promote pointers, inferred ordinals, or payload hashes into
//! engine lifecycle identity.

use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::cell::Cell;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::peace_conference_live::{
    ActivePeaceConferenceObservation, PeaceConferenceObservation, PeaceConferenceReference,
    PeaceConferenceTransportFrame,
};

pub const CONFERENCE_REFRESH_RVA: u32 = 0x0184_EB30;
pub const CLAIM_BREAKDOWN_RVA: u32 = 0x019C_E290;
pub const CONTEST_BREAKDOWN_RVA: u32 = 0x019C_D540;
pub const TERMINAL_SUMMARY_RVA: u32 = 0x00B6_FF40;
pub const REFERENCE_RESOLVER_RVA: u32 = 0x0220_A3D0;

pub const EXACT_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const STATIC_PRODUCER_CONSUMER_MANIFEST_SHA256: &str =
    "da15e19ccb176ca17b7df0d54b1572ad792be671b0c004e834318ca2c25ba845";
pub const HOOK_SENTINEL_ARTIFACT_SHA256: &str =
    "1fcb96ea47b374d47bfd0287043004de62b20f6e14a53e015c1ef8c32776b441";
pub const NEGATIVE_IDENTITY_LAYOUT_MANIFEST_SHA256: &str =
    "e7aedb65fae4955b819ec73231307d37da6a287380217af8bf9d4c5d70483f60";
pub const NEGATIVE_IDENTITY_VTABLE_A_MANIFEST_SHA256: &str =
    "184998042c415c11229cc5b101e5d0758990c323f807d0639d129e8def9b915c";
pub const NEGATIVE_IDENTITY_VTABLE_B_MANIFEST_SHA256: &str =
    "1fb2e8c659d9454cd8f2b1fc37a60b5bfa0baa566d915977c06d96e62a79720b";

const CONFERENCE_REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x18, 0x55, 0x56, 0x57, 0x48, 0x83, 0xEC, 0x30, 0x48, 0x8B,
    0xF9, 0x48,
];
const CLAIM_BREAKDOWN_SENTINEL: [u8; 16] = [
    0x48, 0x8B, 0xC4, 0x4C, 0x89, 0x40, 0x18, 0x48, 0x89, 0x50, 0x10, 0x48, 0x89, 0x48,
    0x08, 0x55,
];
const CONTEST_BREAKDOWN_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4C, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57,
    0x41, 0x54,
];
const TERMINAL_SUMMARY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x4C, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
    0x56, 0x41,
];

const WINDOW_CONFERENCE_REFERENCE: usize = 0x5108;
const WINDOW_CAPTURE_START: usize = 0x50F8;
const WINDOW_CAPTURE_LEN: usize = 0xE8;
const CONFERENCE_REFERENCE: usize = 0x08;
const CONFERENCE_CURRENT_TURN: usize = 0xD8;
const CONFERENCE_COMPLETION: usize = 0x170;
const CONFERENCE_CAPTURE_LEN: usize = 0x300;
const CLAIM_ACTION: usize = 0x08;
const CLAIM_CAPTURE_LEN: usize = 0x40;
const ACTION_TOKEN: usize = 0x08;
const ACTION_CAPTURE_LEN: usize = 0x200;
const TERMINAL_VIEW_SUMMARY: usize = 0xAB8;
const TERMINAL_SUMMARY_CAPTURE_LEN: usize = 0x200;
const TERMINAL_CONTAINER_OFFSETS: [usize; 5] = [0xF8, 0x110, 0x128, 0x140, 0x158];
const TERMINAL_CHILD_CAPTURE_LEN: usize = 0x200;
const MAX_EVENTS: usize = 16_384;
const MAX_CONTEST_CLAIMS: usize = 4_096;
const MAX_TERMINAL_CHILDREN_PER_CONTAINER: usize = 4_096;

type OneArgReturnFn = unsafe extern "system" fn(usize) -> u64;
type OneArgVoidFn = unsafe extern "system" fn(usize);
type FourArgReturnFn = unsafe extern "system" fn(usize, usize, usize, usize) -> u64;
type ReferenceResolverFn = unsafe extern "system" fn(*const u32) -> usize;

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CONFERENCE_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CLAIM_BREAKDOWN: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CONTEST_BREAKDOWN: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TERMINAL_SUMMARY: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static DROPPED_REENTRANT: AtomicUsize = AtomicUsize::new(0);
static DROPPED_LOCK_BUSY: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawMemoryBlock {
    /// Process-local diagnostic address. It is never a portable identity.
    pub process_local_address: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawClaimObservation {
    pub process_local_claim_pointer: u64,
    pub process_local_action_pointer: u64,
    pub action_token_raw: u32,
    pub claim: RawMemoryBlock,
    pub action: RawMemoryBlock,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTerminalContainer {
    pub summary_offset: u32,
    pub declared_count: u32,
    pub children: Vec<RawMemoryBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeaceEvidenceEventKind {
    ConferenceRefresh {
        conference_reference_raw: [u32; 2],
        process_local_conference_pointer: u64,
        current_turn_raw: i32,
        completed_raw: bool,
        window: RawMemoryBlock,
        conference: RawMemoryBlock,
    },
    ClaimBreakdown {
        claim: RawClaimObservation,
    },
    ContestBreakdown {
        claims: Vec<RawClaimObservation>,
    },
    TerminalSummary {
        process_local_summary_pointer: u64,
        summary: RawMemoryBlock,
        containers: Vec<RawTerminalContainer>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceEvidenceEvent {
    pub event_sequence: u64,
    pub refresh_epoch: u64,
    pub kind: PeaceEvidenceEventKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub callbacks_observed: u64,
    pub events_retained: u64,
    pub refresh_epochs: u64,
    pub dropped_reentrant: u64,
    pub dropped_lock_busy: u64,
    pub dropped_unreadable: u64,
    pub dropped_overflow: u64,
    pub completeness_poisoned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookDrain {
    pub events: Vec<PeaceEvidenceEvent>,
    pub diagnostics: HookDiagnostics,
}

impl HookDrain {
    pub fn loss_free(&self) -> bool {
        !self.diagnostics.completeness_poisoned
            && self.diagnostics.dropped_reentrant == 0
            && self.diagnostics.dropped_lock_busy == 0
            && self.diagnostics.dropped_unreadable == 0
            && self.diagnostics.dropped_overflow == 0
            && self.diagnostics.events_retained == self.events.len() as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub conference_refresh: usize,
    pub claim_breakdown: usize,
    pub contest_breakdown: usize,
    pub terminal_summary: usize,
}

#[derive(Debug, Default)]
struct HookState {
    next_event_sequence: u64,
    next_refresh_sequence: u64,
    refresh_epoch: u64,
    events: Vec<PeaceEvidenceEvent>,
    diagnostics: HookDiagnostics,
    latest_refresh: Option<LatestRefresh>,
}

#[derive(Clone, Copy, Debug)]
struct LatestRefresh {
    refresh_sequence: u64,
    refresh_epoch: u64,
    observation: Option<(PeaceConferenceReference, i32, bool)>,
}

pub fn install_for_evidence() -> Result<InstalledHookAddresses, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "A40 peace hook lifecycle mutex poisoned".to_string())?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(installed_addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    if MODULE_BASE.load(Ordering::Acquire) != 0 {
        return Err("A40 peace prior hook set has not completed quiescent removal".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A40 peace GetModuleHandleW(NULL) returned null".into());
    }
    for (rva, sentinel) in [
        (CONFERENCE_REFRESH_RVA, &CONFERENCE_REFRESH_SENTINEL[..]),
        (CLAIM_BREAKDOWN_RVA, &CLAIM_BREAKDOWN_SENTINEL[..]),
        (CONTEST_BREAKDOWN_RVA, &CONTEST_BREAKDOWN_SENTINEL[..]),
        (TERMINAL_SUMMARY_RVA, &TERMINAL_SUMMARY_SENTINEL[..]),
    ] {
        verify_sentinel(base + rva as usize, sentinel)?;
    }
    *STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "A40 peace hook state mutex poisoned")? = HookState::default();
    DROPPED_REENTRANT.store(0, Ordering::Release);
    DROPPED_LOCK_BUSY.store(0, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    let addresses = installed_addresses(base);
    let hooks = [
        (
            addresses.conference_refresh,
            hook_conference_refresh as *mut c_void,
            &ORIGINAL_CONFERENCE_REFRESH,
        ),
        (
            addresses.claim_breakdown,
            hook_claim_breakdown as *mut c_void,
            &ORIGINAL_CLAIM_BREAKDOWN,
        ),
        (
            addresses.contest_breakdown,
            hook_contest_breakdown as *mut c_void,
            &ORIGINAL_CONTEST_BREAKDOWN,
        ),
        (
            addresses.terminal_summary,
            hook_terminal_summary as *mut c_void,
            &ORIGINAL_TERMINAL_SUMMARY,
        ),
    ];
    let mut created = Vec::new();
    for (target, detour, original_slot) in hooks {
        let original = match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(original) => original,
            Err(status) => {
                for rollback in created.iter().rev() {
                    let _ = unsafe { MinHook::remove_hook(*rollback as *mut c_void) };
                }
                clear_originals();
                MODULE_BASE.store(0, Ordering::Release);
                return Err(format!(
                    "A40 peace hook creation at {target:#x} failed: {status}"
                ));
            }
        };
        original_slot.store(original as usize, Ordering::Release);
        created.push(target);
    }
    for target in &created {
        if let Err(status) = unsafe { inline_detour::enable(*target as *mut c_void) } {
            let mut cleanup_failures = Vec::new();
            for rollback in created.iter().rev() {
                match unsafe { MinHook::disable_hook(*rollback as *mut c_void) } {
                    Ok(())
                    | Err(MH_STATUS::MH_ERROR_DISABLED)
                    | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                    Err(error) => cleanup_failures.push(format!(
                        "disable {rollback:#x}: {error}"
                    )),
                }
            }
            if cleanup_failures.is_empty() {
                if let Err(error) = wait_for_quiescence() {
                    cleanup_failures.push(error);
                }
            }
            if cleanup_failures.is_empty() {
                for rollback in created.iter().rev() {
                    match unsafe { MinHook::remove_hook(*rollback as *mut c_void) } {
                        Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                        Err(error) => cleanup_failures.push(format!(
                            "remove {rollback:#x}: {error}"
                        )),
                    }
                }
            }
            if !cleanup_failures.is_empty() {
                return Err(format!(
                    "A40 peace hook enable at {target:#x} failed: {status}; rollback failed: {}",
                    cleanup_failures.join(", ")
                ));
            }
            clear_originals();
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!(
                "A40 peace hook enable at {target:#x} failed: {status}"
            ));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(addresses)
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Production uses the same exact sentinels and passive detours. Keeping a
/// separately named entry point prevents evidence receipts from being treated
/// as the source of product authority.
pub fn install() -> Result<InstalledHookAddresses, String> {
    install_for_evidence()
}

/// Materialize the latest exact refresh as a pointer-free PCS1 frame. An
/// all-zero reference observed at the refresh boundary is exact inactive.
pub fn production_payload(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, String> {
    let mut state = STATE
        .get()
        .ok_or("A40 peace production hook state is not initialized")?
        .lock()
        .map_err(|_| "A40 peace production hook state mutex poisoned")?;
    let latest = state
        .latest_refresh
        .take()
        .ok_or("A40 peace refresh was not observed")?;
    let observation = match latest.observation {
        None => PeaceConferenceObservation::Inactive {
            refresh_sequence: latest.refresh_sequence,
        },
        Some((conference_reference, current_turn_raw, completed)) => {
            PeaceConferenceObservation::Active(ActivePeaceConferenceObservation {
                refresh_sequence: latest.refresh_sequence,
                refresh_epoch: latest.refresh_epoch,
                conference_reference,
                current_turn_raw,
                completed,
                semantic_snapshot_complete: false,
            })
        }
    };
    crate::peace_conference_live::encode_payload(&PeaceConferenceTransportFrame {
        frame_hour,
        elapsed_wall_ms,
        status_flags,
        observation: Some(observation),
    })
    .map_err(|error| format!("A40 peace PCS1 encode failed: {error:?}"))
}

/// Drop any process-local conference refresh identity at a campaign boundary.
/// Optional evidence events remain append-only; production refresh identity
/// restarts because registered references are scoped to the new campaign.
pub fn reset_campaign_observations() -> Result<(), String> {
    let mut state = STATE
        .get()
        .ok_or("A40 peace hook state is not initialized")?
        .lock()
        .map_err(|_| "A40 peace hook state mutex poisoned")?;
    state.next_refresh_sequence = 0;
    state.refresh_epoch = 0;
    state.latest_refresh = None;
    Ok(())
}

pub fn diagnostics() -> HookDiagnostics {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .map(|state| {
            let mut diagnostics = state.diagnostics;
            diagnostics.events_retained = state.events.len() as u64;
            diagnostics.refresh_epochs = state.refresh_epoch;
            diagnostics.dropped_reentrant =
                DROPPED_REENTRANT.load(Ordering::Acquire) as u64;
            diagnostics.dropped_lock_busy =
                DROPPED_LOCK_BUSY.load(Ordering::Acquire) as u64;
            diagnostics.completeness_poisoned |= diagnostics.dropped_reentrant != 0
                || diagnostics.dropped_lock_busy != 0;
            diagnostics
        })
        .unwrap_or_default()
}

pub fn seal_drain_and_uninstall() -> Result<HookDrain, String> {
    uninstall_hooks()?;
    let mut state = STATE
        .get()
        .ok_or("A40 peace hook state is not initialized")?
        .lock()
        .map_err(|_| "A40 peace hook state mutex poisoned")?;
    state.diagnostics.events_retained = state.events.len() as u64;
    state.diagnostics.refresh_epochs = state.refresh_epoch;
    state.diagnostics.dropped_reentrant =
        DROPPED_REENTRANT.load(Ordering::Acquire) as u64;
    state.diagnostics.dropped_lock_busy =
        DROPPED_LOCK_BUSY.load(Ordering::Acquire) as u64;
    state.diagnostics.completeness_poisoned |= state.diagnostics.dropped_reentrant != 0
        || state.diagnostics.dropped_lock_busy != 0;
    Ok(HookDrain {
        events: mem::take(&mut state.events),
        diagnostics: state.diagnostics,
    })
}

pub fn uninstall_hooks() -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "A40 peace hook lifecycle mutex poisoned".to_string())?;
    let needs_disable = INSTALLED.load(Ordering::Acquire);
    if !needs_disable && MODULE_BASE.load(Ordering::Acquire) == 0 {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let addresses = installed_addresses(base);
    let targets = [
        addresses.terminal_summary,
        addresses.contest_breakdown,
        addresses.claim_breakdown,
        addresses.conference_refresh,
    ];
    let mut first_error = None;
    if needs_disable {
        for target in &targets {
            if let Err(status) = unsafe { MinHook::disable_hook(*target as *mut c_void) } {
                first_error.get_or_insert_with(|| {
                    format!("A40 peace hook disable at {target:#x} failed: {status}")
                });
            }
        }
        if let Some(error) = first_error.as_ref() {
            return Err(error.clone());
        }
        INSTALLED.store(false, Ordering::Release);
    }
    wait_for_quiescence()?;
    for target in targets {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                first_error.get_or_insert_with(|| {
                    format!("A40 peace hook removal at {target:#x} failed: {status}")
                });
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    clear_originals();
    MODULE_BASE.store(0, Ordering::Release);
    Ok(())
}

fn installed_addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        conference_refresh: base + CONFERENCE_REFRESH_RVA as usize,
        claim_breakdown: base + CLAIM_BREAKDOWN_RVA as usize,
        contest_breakdown: base + CONTEST_BREAKDOWN_RVA as usize,
        terminal_summary: base + TERMINAL_SUMMARY_RVA as usize,
    }
}

fn clear_originals() {
    ORIGINAL_CONFERENCE_REFRESH.store(0, Ordering::Release);
    ORIGINAL_CLAIM_BREAKDOWN.store(0, Ordering::Release);
    ORIGINAL_CONTEST_BREAKDOWN.store(0, Ordering::Release);
    ORIGINAL_TERMINAL_SUMMARY.store(0, Ordering::Release);
}

struct CallbackGuard;

impl CallbackGuard {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn wait_for_quiescence() -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) != 0 {
        if std::time::Instant::now() >= deadline {
            return Err("A40 peace hook callbacks did not quiesce after disable".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

unsafe extern "system" fn hook_conference_refresh(window: usize) -> u64 {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let original = ORIGINAL_CONFERENCE_REFRESH.load(Ordering::Acquire);
    let result = if original == 0 {
        0
    } else {
        let original: OneArgReturnFn = unsafe { mem::transmute(original) };
        unsafe { original(window) }
    };
    if nested {
        DROPPED_REENTRANT.fetch_add(1, Ordering::AcqRel);
    } else {
        publish_capture(capture_conference_refresh(window), true);
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
    result
}

unsafe extern "system" fn hook_claim_breakdown(
    output: usize,
    context: usize,
    claim: usize,
    label: usize,
) -> u64 {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    if nested {
        DROPPED_REENTRANT.fetch_add(1, Ordering::AcqRel);
    } else {
        publish_capture(capture_claim_breakdown(claim).map(Some), false);
    }
    let original = ORIGINAL_CLAIM_BREAKDOWN.load(Ordering::Acquire);
    let result = if original == 0 {
        0
    } else {
        let original: FourArgReturnFn = unsafe { mem::transmute(original) };
        unsafe { original(output, context, claim, label) }
    };
    IN_CALLBACK.with(|guard| guard.set(nested));
    result
}

unsafe extern "system" fn hook_contest_breakdown(
    output: usize,
    context: usize,
    claims: usize,
    label: usize,
) -> u64 {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    if nested {
        DROPPED_REENTRANT.fetch_add(1, Ordering::AcqRel);
    } else {
        publish_capture(capture_contest_breakdown(claims).map(Some), false);
    }
    let original = ORIGINAL_CONTEST_BREAKDOWN.load(Ordering::Acquire);
    let result = if original == 0 {
        0
    } else {
        let original: FourArgReturnFn = unsafe { mem::transmute(original) };
        unsafe { original(output, context, claims, label) }
    };
    IN_CALLBACK.with(|guard| guard.set(nested));
    result
}

unsafe extern "system" fn hook_terminal_summary(view: usize) {
    let _callback = CallbackGuard::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    if nested {
        DROPPED_REENTRANT.fetch_add(1, Ordering::AcqRel);
    } else {
        publish_capture(capture_terminal_summary(view).map(Some), false);
    }
    let original = ORIGINAL_TERMINAL_SUMMARY.load(Ordering::Acquire);
    if original != 0 {
        let original: OneArgVoidFn = unsafe { mem::transmute(original) };
        unsafe { original(view) };
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

fn capture_conference_refresh(window: usize) -> Result<Option<PeaceEvidenceEventKind>, String> {
    if window == 0 {
        return Err("A40 peace refresh supplied null window".into());
    }
    let first = [
        read_u32(window + WINDOW_CONFERENCE_REFERENCE)
            .ok_or("A40 peace conference reference word 0 unreadable")?,
        read_u32(window + WINDOW_CONFERENCE_REFERENCE + 4)
            .ok_or("A40 peace conference reference word 1 unreadable")?,
    ];
    if first == [0, 0] {
        // This refresh boundary also runs while the peace window has no active
        // conference. That is a measured inactive observation, not evidence
        // loss and not an active lifecycle event.
        return Ok(None);
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let resolver: ReferenceResolverFn =
        unsafe { mem::transmute(base + REFERENCE_RESOLVER_RVA as usize) };
    let conference = unsafe { resolver(first.as_ptr()) };
    if conference == 0 {
        return Err("A40 peace conference reference did not resolve".into());
    }
    let object_reference = [
        read_u32(conference + CONFERENCE_REFERENCE)
            .ok_or("A40 peace resolved conference reference word 0 unreadable")?,
        read_u32(conference + CONFERENCE_REFERENCE + 4)
            .ok_or("A40 peace resolved conference reference word 1 unreadable")?,
    ];
    let second = [
        read_u32(window + WINDOW_CONFERENCE_REFERENCE)
            .ok_or("A40 peace conference reference word 0 reread failed")?,
        read_u32(window + WINDOW_CONFERENCE_REFERENCE + 4)
            .ok_or("A40 peace conference reference word 1 reread failed")?,
    ];
    if first != second || first != object_reference {
        return Err("A40 peace conference reference changed or failed object back-join".into());
    }
    Ok(Some(PeaceEvidenceEventKind::ConferenceRefresh {
        conference_reference_raw: first,
        process_local_conference_pointer: conference as u64,
        current_turn_raw: read_i32(conference + CONFERENCE_CURRENT_TURN)
            .ok_or("A40 peace current turn unreadable")?,
        completed_raw: read_usize(conference + CONFERENCE_COMPLETION)
            .ok_or("A40 peace completion field unreadable")?
            != 0,
        window: read_block(window + WINDOW_CAPTURE_START, WINDOW_CAPTURE_LEN)?,
        conference: read_block(conference, CONFERENCE_CAPTURE_LEN)?,
    }))
}

fn capture_claim_breakdown(claim: usize) -> Result<PeaceEvidenceEventKind, String> {
    Ok(PeaceEvidenceEventKind::ClaimBreakdown {
        claim: capture_claim(claim)?,
    })
}

fn capture_contest_breakdown(claims: usize) -> Result<PeaceEvidenceEventKind, String> {
    let data = read_usize(claims).ok_or("A40 peace contest vector data unreadable")?;
    let count = read_i32(claims + 0x0C).ok_or("A40 peace contest vector count unreadable")?;
    if count < 0 || count as usize > MAX_CONTEST_CLAIMS || (count != 0 && data == 0) {
        return Err("A40 peace contest vector shape is invalid/out of bounds".into());
    }
    let mut observations = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let claim = read_usize(data + index * mem::size_of::<usize>())
            .filter(|pointer| *pointer != 0)
            .ok_or_else(|| format!("A40 peace contest claim {index} is null/unreadable"))?;
        observations.push(capture_claim(claim)?);
    }
    Ok(PeaceEvidenceEventKind::ContestBreakdown {
        claims: observations,
    })
}

fn capture_terminal_summary(view: usize) -> Result<PeaceEvidenceEventKind, String> {
    let summary = read_usize(view + TERMINAL_VIEW_SUMMARY)
        .filter(|pointer| *pointer != 0)
        .ok_or("A40 peace terminal summary pointer is null/unreadable")?;
    let mut containers = Vec::with_capacity(TERMINAL_CONTAINER_OFFSETS.len());
    for offset in TERMINAL_CONTAINER_OFFSETS {
        let data = read_usize(summary + offset)
            .ok_or_else(|| format!("A40 peace terminal container {offset:#x} data unreadable"))?;
        let count = read_i32(summary + offset + 0x0C)
            .ok_or_else(|| format!("A40 peace terminal container {offset:#x} count unreadable"))?;
        if count < 0
            || count as usize > MAX_TERMINAL_CHILDREN_PER_CONTAINER
            || (count != 0 && data == 0)
        {
            return Err(format!(
                "A40 peace terminal container {offset:#x} shape is invalid/out of bounds"
            ));
        }
        let mut children = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            let child = read_usize(data + index * mem::size_of::<usize>())
                .filter(|pointer| *pointer != 0)
                .ok_or_else(|| {
                    format!("A40 peace terminal container {offset:#x} child {index} unreadable")
                })?;
            children.push(read_block(child, TERMINAL_CHILD_CAPTURE_LEN)?);
        }
        containers.push(RawTerminalContainer {
            summary_offset: offset as u32,
            declared_count: count as u32,
            children,
        });
    }
    Ok(PeaceEvidenceEventKind::TerminalSummary {
        process_local_summary_pointer: summary as u64,
        summary: read_block(summary, TERMINAL_SUMMARY_CAPTURE_LEN)?,
        containers,
    })
}

fn capture_claim(claim: usize) -> Result<RawClaimObservation, String> {
    if claim == 0 {
        return Err("A40 peace claim pointer is null".into());
    }
    let action = read_usize(claim + CLAIM_ACTION)
        .filter(|pointer| *pointer != 0)
        .ok_or("A40 peace claim action pointer is null/unreadable")?;
    Ok(RawClaimObservation {
        process_local_claim_pointer: claim as u64,
        process_local_action_pointer: action as u64,
        action_token_raw: read_u32(action + ACTION_TOKEN)
            .ok_or("A40 peace action token unreadable")?,
        claim: read_block(claim, CLAIM_CAPTURE_LEN)?,
        action: read_block(action, ACTION_CAPTURE_LEN)?,
    })
}

fn publish_capture(
    result: Result<Option<PeaceEvidenceEventKind>, String>,
    advances_epoch: bool,
) {
    let Some(state) = STATE.get() else {
        return;
    };
    let Ok(mut state) = state.try_lock() else {
        DROPPED_LOCK_BUSY.fetch_add(1, Ordering::AcqRel);
        return;
    };
    state.diagnostics.callbacks_observed =
        state.diagnostics.callbacks_observed.saturating_add(1);
    let kind = match result {
        Ok(Some(kind)) => Some(kind),
        Ok(None) if advances_epoch => None,
        Ok(None) => return,
        Err(_) => {
            state.diagnostics.dropped_unreadable =
                state.diagnostics.dropped_unreadable.saturating_add(1);
            state.diagnostics.completeness_poisoned = true;
            return;
        }
    };
    if advances_epoch {
        state.refresh_epoch = state.refresh_epoch.saturating_add(1).max(1);
    }
    if advances_epoch {
        state.next_refresh_sequence = state.next_refresh_sequence.saturating_add(1).max(1);
        let refresh_sequence = state.next_refresh_sequence;
        let refresh_epoch = state.refresh_epoch;
        let observation = match kind.as_ref() {
            Some(PeaceEvidenceEventKind::ConferenceRefresh {
                conference_reference_raw,
                current_turn_raw,
                completed_raw,
                ..
            }) => Some((
                PeaceConferenceReference {
                    object_type: conference_reference_raw[0],
                    object_id: conference_reference_raw[1],
                },
                *current_turn_raw,
                *completed_raw,
            )),
            None => None,
            Some(_) => {
                state.diagnostics.completeness_poisoned = true;
                return;
            }
        };
        state.latest_refresh = Some(LatestRefresh {
            refresh_sequence,
            refresh_epoch,
            observation,
        });
    }
    let Some(kind) = kind else {
        return;
    };
    // Evidence retention is bounded independently of the production refresh
    // latch. Even after the optional raw-event buffer fills, the newest exact
    // refresh above remains publishable to PCS1.
    if state.events.len() >= MAX_EVENTS {
        state.diagnostics.dropped_overflow =
            state.diagnostics.dropped_overflow.saturating_add(1);
        state.diagnostics.completeness_poisoned = true;
        return;
    }
    state.next_event_sequence = state.next_event_sequence.saturating_add(1).max(1);
    let event_sequence = state.next_event_sequence;
    let refresh_epoch = state.refresh_epoch;
    state.events.push(PeaceEvidenceEvent {
        event_sequence,
        refresh_epoch,
        kind,
    });
}

fn verify_sentinel(address: usize, expected: &[u8]) -> Result<(), String> {
    let mut actual = vec![0; expected.len()];
    if !read_exact(address, &mut actual) {
        return Err(format!(
            "A40 peace hook sentinel at {address:#x} unreadable"
        ));
    }
    if actual != expected {
        return Err(format!(
            "A40 peace hook sentinel mismatch at {address:#x}"
        ));
    }
    Ok(())
}

fn read_block(address: usize, len: usize) -> Result<RawMemoryBlock, String> {
    let mut bytes = vec![0; len];
    if !read_exact(address, &mut bytes) {
        return Err(format!(
            "A40 peace raw block {address:#x}+{len:#x} unreadable"
        ));
    }
    Ok(RawMemoryBlock {
        process_local_address: address as u64,
        bytes,
    })
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

fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0; 4];
    read_exact(address, &mut bytes).then(|| u32::from_le_bytes(bytes))
}

fn read_i32(address: usize) -> Option<i32> {
    read_u32(address).map(|value| value as i32)
}

fn read_usize(address: usize) -> Option<usize> {
    let mut bytes = [0; mem::size_of::<usize>()];
    read_exact(address, &mut bytes).then(|| usize::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hook_contract_is_frozen() {
        assert_eq!(CONFERENCE_REFRESH_RVA, 0x0184_EB30);
        assert_eq!(CLAIM_BREAKDOWN_RVA, 0x019C_E290);
        assert_eq!(CONTEST_BREAKDOWN_RVA, 0x019C_D540);
        assert_eq!(TERMINAL_SUMMARY_RVA, 0x00B6_FF40);
        assert_eq!(REFERENCE_RESOLVER_RVA, 0x0220_A3D0);
        assert_eq!(CONFERENCE_REFRESH_SENTINEL.len(), 16);
        assert_eq!(CLAIM_BREAKDOWN_SENTINEL.len(), 16);
        assert_eq!(CONTEST_BREAKDOWN_SENTINEL.len(), 16);
        assert_eq!(TERMINAL_SUMMARY_SENTINEL.len(), 16);
    }

    #[test]
    fn loss_free_is_strict() {
        let mut drain = HookDrain {
            events: Vec::new(),
            diagnostics: HookDiagnostics::default(),
        };
        assert!(drain.loss_free());
        drain.diagnostics.dropped_unreadable = 1;
        assert!(!drain.loss_free());
    }
}
