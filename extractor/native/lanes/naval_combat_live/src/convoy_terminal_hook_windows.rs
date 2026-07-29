//! Fail-closed a729 CTA1 pre-collapse call-site observer.
//!
//! The installer is gated by the exact-a729 static authority and full byte
//! window. It
//! replaces only the authenticated five-byte CALL with a rel32 call to a near
//! relay; the relay performs a pointer-free absolute jump to the Rust ABI
//! wrapper. The wrapper invokes the original aggregator exactly once and emits
//! only after a same-hour, exact D70 before/after mutation has been proved.

#![cfg(all(windows, target_arch = "x86_64"))]

use core::cell::Cell;
use core::ffi::c_void;
use core::ptr;
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_rel32_callsite, observe_released, ExpectedRvaHook,
    HookTeardownAttestation, InstallAbortDiscard, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    D90_REL32_CALLSITE_PRIMITIVE_ID,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsString;
use std::io::Read;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::convoy_route_source::{
    read_exact_a729_client_route, ExactA729RouteRead, RouteSourceError,
};
use crate::convoy_terminal_attribution::{
    activation_blockers, capture_candidate, prepare_a729_hook, A729PreCollapseRegisters,
    ActivationBlocker, ConvoyTerminalAttributionCarrier, ExactMemory, A729_CALL_BYTES,
    A729_EXE_SHA256, A729_HOOK_RVA, MAX_COUNTRY_TAG_DB_ID,
};
use crate::convoy_terminal_runtime::{
    clear_convoy_terminal_resolver_snapshot, current_convoy_terminal_publication_receipt,
    try_current_convoy_terminal_resolver_snapshot, ResolverPublicationReceipt,
};
use crate::convoy_transport_attack::{
    encode_convoy_transport_attack_batch, BatchDecodeError, ConvoyD70MutationKind,
    ConvoyD70MutationProof, ConvoyTransportAttackRow, ConvoyTransportAttackTelemetry,
    ConvoyTransportAttackWitness,
};

type Bool = i32;
type Dword = u32;
type Handle = *mut c_void;
type TerminalAggregator = unsafe extern "system" fn(usize, u32, usize, usize) -> usize;

const MEM_COMMIT: Dword = 0x1000;
const MEM_RESERVE: Dword = 0x2000;
const MEM_RELEASE: Dword = 0x8000;
const PAGE_READWRITE: Dword = 0x04;
const PAGE_EXECUTE_READ: Dword = 0x20;
const PAGE_EXECUTE_READWRITE: Dword = 0x40;
const TH32CS_SNAPTHREAD: Dword = 0x0000_0004;
const THREAD_SUSPEND_RESUME: Dword = 0x0002;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const ERROR_INVALID_HANDLE: Dword = 6;
const ERROR_INVALID_PARAMETER: Dword = 87;
const RELAY_LEN: usize = 14;
const ALLOCATION_GRANULARITY: usize = 0x1_0000;
const MAX_REL32_DISTANCE: usize = 0x7FFF_0000;
const MAX_D70_ROWS: usize = 65_536;
const MAX_CONVOYS_PER_D70_ROW: u32 = 1_000_000;
const EVENT_QUEUE_CAPACITY: usize = 4_096;
const QUIESCE_TIMEOUT: Duration = Duration::from_secs(5);

const GS_CURRENT_HOUR_OFF: usize = 0x468;
const GS_D70_BEGIN_OFF: usize = 0x5A8;
const GS_D70_CAPACITY_OFF: usize = 0x5B0;
const GS_D70_COUNT_OFF: usize = 0x5B4;
const SUNK_CONVOY_INFO_VTABLE_RVA: usize = 0x295_D008;
const D90_HOOK_EXPECTATION: ExpectedRvaHook = ExpectedRvaHook {
    logical_name: "gameplay.d90.convoy_terminal_route_callsite",
    requested_rva: A729_HOOK_RVA as u64,
    target_patch_len: A729_CALL_BYTES.len() as u16,
    primitive_id: D90_REL32_CALLSITE_PRIMITIVE_ID,
    original_prefix: &A729_CALL_BYTES,
};

#[repr(C)]
struct ThreadEntry32 {
    size: Dword,
    usage: Dword,
    thread_id: Dword,
    owner_process_id: Dword,
    base_priority: i32,
    delta_priority: i32,
    flags: Dword,
}

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        allocation_type: Dword,
        protect: Dword,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: Dword) -> Bool;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: Dword,
        old_protect: *mut Dword,
    ) -> Bool;
    fn FlushInstructionCache(process: Handle, address: *const c_void, size: usize) -> Bool;
    fn ReadProcessMemory(
        process: Handle,
        base_address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        bytes_read: *mut usize,
    ) -> Bool;
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentProcessId() -> Dword;
    fn GetCurrentThreadId() -> Dword;
    fn GetModuleHandleW(module_name: *const u16) -> Handle;
    fn GetModuleFileNameW(module: Handle, filename: *mut u16, size: Dword) -> Dword;
    fn CreateToolhelp32Snapshot(flags: Dword, process_id: Dword) -> Handle;
    fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry32) -> Bool;
    fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry32) -> Bool;
    fn OpenThread(access: Dword, inherit: Bool, thread_id: Dword) -> Handle;
    fn SuspendThread(thread: Handle) -> Dword;
    fn ResumeThread(thread: Handle) -> Dword;
    fn CloseHandle(handle: Handle) -> Bool;
    fn GetLastError() -> Dword;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum D70MutationKind {
    IncrementExisting,
    AppendFirstSeen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct D70MutationProof {
    pub kind: D70MutationKind,
    pub ordinal: u32,
    pub before_rows: u32,
    pub after_rows: u32,
    pub before_count: u32,
    pub after_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConvoyTerminalValidatedEvent {
    pub carrier: ConvoyTerminalAttributionCarrier,
    pub d70: D70MutationProof,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ConvoyTerminalHookTelemetry {
    pub callbacks_seen: u64,
    pub original_calls: u64,
    pub events_emitted: u64,
    pub dropped_no_original: u64,
    pub dropped_teardown: u64,
    pub dropped_nested: u64,
    pub dropped_capture_busy: u64,
    pub dropped_hour_or_epoch: u64,
    pub dropped_resolver: u64,
    pub dropped_candidate: u64,
    pub dropped_d70_pre: u64,
    pub dropped_d70_post: u64,
    pub dropped_d70_mutation: u64,
    pub dropped_queue_busy: u64,
    pub dropped_queue_full: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConvoyTerminalDrain {
    pub events: Vec<ConvoyTerminalValidatedEvent>,
    pub cumulative_telemetry: ConvoyTerminalHookTelemetry,
    pub queue_drain_complete: bool,
    pub correlation_poisoned: bool,
    pub remaining_events: u32,
    pub active_callbacks: u32,
    pub producer_sequence_next: u64,
    pub queue_mutation_generation: u64,
    pub resolver: ResolverPublicationReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvoyTerminalDrainReservation {
    pub reservation_id: u64,
    pub drain: ConvoyTerminalDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvoyTerminalPayloadReservation {
    pub reservation_id: u64,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ConvoyTerminalHookHeartbeat {
    pub installed: bool,
    pub teardown_requested: bool,
    pub active_callbacks: usize,
    pub resolver: ResolverPublicationReceipt,
    pub telemetry: ConvoyTerminalHookTelemetry,
    pub producer_sequence_next: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvoyTerminalInstallError {
    PromotionBlocked(Vec<ActivationBlocker>),
    EvidenceActivationDisabled,
    BinaryGate,
    AlreadyInstalled,
    TeardownPending,
    ModuleBase,
    AddressOverflow,
    RelayAllocation,
    RelayProtect,
    RelayFlush,
    RelayFree,
    RelayDrift,
    Rel32OutOfRange,
    ThreadSnapshot,
    ThreadEnumeration,
    ThreadOpen(Dword),
    ThreadSuspend(Dword),
    ByteGateDrift,
    PatchProtect,
    PatchFlush,
    PatchedBytesDrift,
    HookReceipt(String),
    UntakenReleaseReceipt,
    RecoveryPending,
    ReservationPending,
    SealedTeardownRequired,
    InstallRollbackFailed {
        install_error: String,
        rollback_error: String,
    },
    QuiesceTimeout,
    ResolverClear(String),
}

pub const H_CONVOY_EVIDENCE_ACTIVATION_ENV: &str = "BATTLE_PLAN_ENABLE_H_CONVOY_EVIDENCE_HOOK";

/// Explicit process-launch authorization for the isolated acquisition hook.
/// It neither changes the domain registry nor satisfies production blockers.
pub fn convoy_terminal_evidence_activation_enabled() -> bool {
    matches!(
        std::env::var(H_CONVOY_EVIDENCE_ACTIVATION_ENV).as_deref(),
        Ok("1")
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvoyTerminalSealError {
    Hook(ConvoyTerminalInstallError),
    Batch(BatchDecodeError),
    Reservation(String),
}

impl From<ConvoyTerminalInstallError> for ConvoyTerminalSealError {
    fn from(value: ConvoyTerminalInstallError) -> Self {
        Self::Hook(value)
    }
}

impl From<BatchDecodeError> for ConvoyTerminalSealError {
    fn from(value: BatchDecodeError) -> Self {
        Self::Batch(value)
    }
}

impl From<String> for ConvoyTerminalSealError {
    fn from(value: String) -> Self {
        Self::Reservation(value)
    }
}

#[derive(Clone, Copy)]
enum HookInstallMode {
    Production,
    Evidence,
}

impl HookInstallMode {
    fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

#[derive(Clone, Copy)]
struct InstalledState {
    mode: HookInstallMode,
    callsite: usize,
    relay: usize,
    original_call: [u8; 5],
    patched_call: [u8; 5],
    owned_mutation: Option<[u8; 5]>,
    install_admitted: bool,
    patch_enabled: bool,
    patch_restored: bool,
    relay_freed: bool,
    relay_verified_before_removal: bool,
    recovery_pending: bool,
    teardown_owner_token: Option<u64>,
    installed_receipt: Option<InstalledRvaHookReceipt>,
    released_receipt: Option<ReleasedRvaHookReceipt>,
}

#[derive(Clone, Debug)]
struct ActiveDrainReservation {
    reservation_id: u64,
    drain: ConvoyTerminalDrain,
}

struct DrainReservationState {
    next_reservation_id: u64,
    active: Option<ActiveDrainReservation>,
    sealed_snapshot: Option<ConvoyTerminalDrain>,
}

impl Default for DrainReservationState {
    fn default() -> Self {
        Self {
            next_reservation_id: 1,
            active: None,
            sealed_snapshot: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct HConvoyD70Identity {
    pub month_raw: i32,
    pub killer_tag_db_id: u32,
    pub owner_tag_db_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HConvoyD70Row {
    #[serde(flatten)]
    pub identity: HConvoyD70Identity,
    pub count: u32,
}

struct PreObservation {
    carrier: ConvoyTerminalAttributionCarrier,
    d70: Vec<HConvoyD70Row>,
    raw: HConvoyPositiveRawObservation,
}

/// Pointer-bearing, process-local custody retained only for the isolated
/// H-CONVOY fixture.  The acquisition owner serializes these windows into
/// hashed raw-role files; CTA1/CTA2 remain pointer-free.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HConvoyRouteRawObservation {
    pub client_id: u32,
    pub client_type_tag: u32,
    pub producer_kind: u8,
    pub source_kind: u8,
    pub destination_kind: u8,
    pub source_id: u32,
    pub destination_id: u32,
    pub reference_registry: usize,
    pub reference_slot: usize,
    pub route_object: usize,
    pub route_slot_bytes: Vec<u8>,
    pub route_object_bytes: Vec<u8>,
    pub source_endpoint_object: usize,
    pub destination_endpoint_object: usize,
    pub source_endpoint_bytes: Vec<u8>,
    pub destination_endpoint_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HConvoyPositiveRawObservation {
    pub module_base: usize,
    pub context_bytes: Vec<u8>,
    pub game_hour_raw: i32,
    pub rcx: usize,
    pub edx: u32,
    pub r8: usize,
    pub r9: usize,
    pub r12: usize,
    pub r15: usize,
    pub killer_tag_db_id: u32,
    pub member_bytes: Vec<u8>,
    pub combat_id: u32,
    pub combat_generation: u64,
    pub route: HConvoyRouteRawObservation,
    pub d70_before: Vec<HConvoyD70Row>,
    pub d70_after: Vec<HConvoyD70Row>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HConvoyControlRawObservation {
    pub game_hour_raw: i32,
    pub member_address: usize,
    pub member_bytes: Vec<u8>,
    pub combat_id: u32,
    pub combat_generation: u64,
    pub route: HConvoyRouteRawObservation,
    pub d70: Vec<HConvoyD70Row>,
    pub heartbeat: ConvoyTerminalHookHeartbeat,
    pub heartbeat_sequence: u64,
}

struct ProcessMemory;

impl ExactMemory for ProcessMemory {
    fn read_exact(&self, address: usize, out: &mut [u8]) -> bool {
        if out.is_empty() {
            return true;
        }
        if address == 0 {
            return false;
        }
        let mut read = 0usize;
        unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                address as *const c_void,
                out.as_mut_ptr().cast(),
                out.len(),
                &mut read,
            ) != 0
                && read == out.len()
        }
    }
}

/// Resolve one complete client handle through the exact engine reference
/// registry law and concrete producer serializer layout. This uses only
/// repeated process-memory reads: it performs no engine call, save correlation,
/// or endpoint inference.
pub fn read_live_a729_client_route(
    client: crate::convoy_terminal_attribution::EntityHandle,
) -> Result<ExactA729RouteRead, RouteSourceError> {
    let module_base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    read_exact_a729_client_route(&ProcessMemory, module_base, client)
}

fn read_exact_bytes(memory: &ProcessMemory, address: usize, len: usize) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    memory.read_exact(address, &mut bytes).then_some(bytes)
}

fn capture_route_raw(
    memory: &ProcessMemory,
    module_base: usize,
    client: crate::convoy_terminal_attribution::EntityHandle,
) -> Option<HConvoyRouteRawObservation> {
    let read = read_exact_a729_client_route(memory, module_base, client).ok()?;
    let route_object_len = match read.resolution.producer_kind {
        crate::convoy_terminal_attribution::RouteProducerKind::NavalTransport => 0x58,
        crate::convoy_terminal_attribution::RouteProducerKind::LendLease => 0xD8,
        crate::convoy_terminal_attribution::RouteProducerKind::Export => return None,
    };
    let route_slot_bytes = read_exact_bytes(memory, read.reference_slot, 24)?;
    let route_object_bytes = read_exact_bytes(memory, read.route_object, route_object_len)?;
    let (source_endpoint_bytes, destination_endpoint_bytes) = if read.resolution.producer_kind
        == crate::convoy_terminal_attribution::RouteProducerKind::LendLease
    {
        (
            read_exact_bytes(memory, read.source_endpoint_object, 0x5C)?,
            read_exact_bytes(memory, read.destination_endpoint_object, 0x5C)?,
        )
    } else {
        (Vec::new(), Vec::new())
    };
    Some(HConvoyRouteRawObservation {
        client_id: client.id,
        client_type_tag: client.type_tag,
        producer_kind: read.resolution.producer_kind as u8,
        source_kind: read.resolution.source.kind as u8,
        destination_kind: read.resolution.destination.kind as u8,
        source_id: read.resolution.source.id,
        destination_id: read.resolution.destination.id,
        reference_registry: read.reference_registry,
        reference_slot: read.reference_slot,
        route_object: read.route_object,
        route_slot_bytes,
        route_object_bytes,
        source_endpoint_object: read.source_endpoint_object,
        destination_endpoint_object: read.destination_endpoint_object,
        source_endpoint_bytes,
        destination_endpoint_bytes,
    })
}

/// Publish the validated game-state root used by the collector's clean
/// d49/d50 walk.  It is usable only while the evidence environment gate is
/// armed and its hour agrees with the immutable resolver receipt.
pub fn publish_h_convoy_evidence_game_state(game_state: usize, game_hour_raw: i32) {
    if convoy_terminal_evidence_activation_enabled() && game_state != 0 {
        EVIDENCE_GAME_STATE.store(game_state, Ordering::Release);
        EVIDENCE_GAME_HOUR.store(game_hour_raw as u32 as u64, Ordering::Release);
    }
}

/// Capture one no-loss boundary without invoking engine code.  The requested
/// client must have exactly one active d49/d50 member in the current immutable
/// resolver epoch.
pub fn capture_h_convoy_control_raw(
    client: crate::convoy_terminal_attribution::EntityHandle,
) -> Result<HConvoyControlRawObservation, String> {
    if !convoy_terminal_evidence_activation_enabled() || !convoy_terminal_hook_installed() {
        return Err("H-CONVOY control snapshot is not armed".into());
    }
    let game_state = EVIDENCE_GAME_STATE.load(Ordering::Acquire);
    let hour = EVIDENCE_GAME_HOUR.load(Ordering::Acquire) as u32 as i32;
    let resolver = current_convoy_terminal_publication_receipt()
        .map_err(|error| format!("H-CONVOY resolver receipt unavailable: {error}"))?;
    if game_state == 0 || resolver.game_hour_raw != hour || resolver.publication_sequence == 0 {
        return Err("H-CONVOY game-state/resolver boundary is stale".into());
    }
    let memory = ProcessMemory;
    let module_base = MODULE_BASE.load(Ordering::Acquire);
    let snapshot = try_current_convoy_terminal_resolver_snapshot()
        .ok_or("H-CONVOY resolver snapshot is unavailable")?;
    let route = capture_route_raw(&memory, module_base, client)
        .ok_or("H-CONVOY route raw window is unavailable")?;
    let mut selected = None;
    for (member, epoch) in snapshot.evidence_member_epochs() {
        let Some(observed) = memory.handle(
            member
                .checked_add(crate::convoy_terminal_attribution::CFEX_MEMBER_CLIENT_HANDLE_OFF)
                .ok_or("H-CONVOY member address overflow")?,
        ) else {
            continue;
        };
        if observed == client {
            if selected.is_some() {
                return Err("H-CONVOY client maps to multiple active combat members".into());
            }
            selected = Some((member, epoch));
        }
    }
    let (member_address, epoch) =
        selected.ok_or("H-CONVOY client lacks an active combat member")?;
    let member_bytes = read_exact_bytes(&memory, member_address, 0x124)
        .ok_or("H-CONVOY member raw window is unreadable")?;
    let d70 =
        read_d70(&memory, module_base, game_state).ok_or("H-CONVOY D70 vector is unreadable")?;
    Ok(HConvoyControlRawObservation {
        game_hour_raw: hour,
        member_address,
        member_bytes,
        combat_id: epoch.combat_id,
        combat_generation: epoch.generation,
        route,
        d70,
        heartbeat: convoy_terminal_hook_heartbeat(),
        heartbeat_sequence: EVIDENCE_HEARTBEAT_SEQUENCE.fetch_add(1, Ordering::AcqRel),
    })
}

pub fn take_h_convoy_positive_raw() -> Result<HConvoyPositiveRawObservation, String> {
    POSITIVE_RAW_OBSERVATION
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "H-CONVOY positive raw observation slot poisoned".to_string())?
        .take()
        .ok_or_else(|| "H-CONVOY positive raw observation is absent".to_string())
}

macro_rules! counters {
    ($($name:ident),+ $(,)?) => { $(static $name: AtomicU64 = AtomicU64::new(0);)+ };
}

counters!(
    CALLBACKS_SEEN,
    ORIGINAL_CALLS,
    EVENTS_EMITTED,
    DROP_NO_ORIGINAL,
    DROP_TEARDOWN,
    DROP_NESTED,
    DROP_CAPTURE_BUSY,
    DROP_HOUR_OR_EPOCH,
    DROP_RESOLVER,
    DROP_CANDIDATE,
    DROP_D70_PRE,
    DROP_D70_POST,
    DROP_D70_MUTATION,
    DROP_QUEUE_BUSY,
    DROP_QUEUE_FULL,
);

static INSTALLED: AtomicBool = AtomicBool::new(false);
static TEARDOWN: AtomicBool = AtomicBool::new(false);
static SEALED_PUBLICATION_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static CAPTURE_BUSY: AtomicBool = AtomicBool::new(false);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TARGET: AtomicUsize = AtomicUsize::new(0);
static PRODUCER_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static QUEUE_MUTATION_GENERATION: AtomicU64 = AtomicU64::new(1);
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());
static INSTALL_STATE: Mutex<Option<InstalledState>> = Mutex::new(None);
static RELEASED_HOOK_RECEIPT: Mutex<Option<ReleasedRvaHookReceipt>> = Mutex::new(None);
static EVENT_QUEUE: OnceLock<Mutex<VecDeque<ConvoyTerminalValidatedEvent>>> = OnceLock::new();
static DRAIN_RESERVATION: OnceLock<Mutex<DrainReservationState>> = OnceLock::new();
static POSITIVE_RAW_OBSERVATION: OnceLock<Mutex<Option<HConvoyPositiveRawObservation>>> =
    OnceLock::new();
static EVIDENCE_GAME_STATE: AtomicUsize = AtomicUsize::new(0);
static EVIDENCE_GAME_HOUR: AtomicU64 = AtomicU64::new(0);
static EVIDENCE_HEARTBEAT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

struct ActiveCallback;
impl ActiveCallback {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for ActiveCallback {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct CallbackNesting;
impl CallbackNesting {
    fn try_enter() -> Option<Self> {
        IN_CALLBACK.with(|flag| if flag.replace(true) { None } else { Some(Self) })
    }
}
impl Drop for CallbackNesting {
    fn drop(&mut self) {
        IN_CALLBACK.with(|flag| flag.set(false));
    }
}

struct CaptureLease;
impl CaptureLease {
    fn try_acquire() -> Option<Self> {
        CAPTURE_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}
impl Drop for CaptureLease {
    fn drop(&mut self) {
        CAPTURE_BUSY.store(false, Ordering::Release);
    }
}

pub fn convoy_terminal_hook_installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Report lifecycle custody that still requires an explicit retry, commit,
/// rollback, or receipt handoff even when the live callsite is no longer
/// installed.
pub fn convoy_terminal_hook_lifecycle_pending() -> bool {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let install_state_pending = INSTALL_STATE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .as_ref()
        .is_some_and(|installed| {
            installed.recovery_pending
                || installed.patch_restored
                || !installed.patch_enabled
                || TEARDOWN.load(Ordering::Acquire)
        });
    if install_state_pending || SEALED_PUBLICATION_PENDING.load(Ordering::Acquire) {
        return true;
    }
    if DRAIN_RESERVATION.get().is_some_and(|reservation| {
        reservation
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .active
            .is_some()
    }) {
        return true;
    }
    RELEASED_HOOK_RECEIPT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .is_some()
}

/// Return the exact production receipt only while the authenticated D90
/// callsite patch remains installed and enabled.
pub fn installed_hook_receipt() -> Option<InstalledRvaHookReceipt> {
    if !convoy_terminal_hook_installed() {
        return None;
    }
    let state = INSTALL_STATE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let installed = state.as_ref().filter(|installed| {
        installed.mode.is_production()
            && installed.patch_enabled
            && !installed.patch_restored
            && !installed.recovery_pending
    })?;
    let stored = installed.installed_receipt?;
    if installed.teardown_owner_token != Some(stored.teardown_owner_token) {
        return None;
    }
    let memory = ProcessMemory;
    let mut exact_patch = [0u8; A729_CALL_BYTES.len()];
    if !memory.read_exact(installed.callsite, &mut exact_patch)
        || exact_patch != installed.patched_call
        || !relay_targets_convoy_terminal_hook(&memory, installed.relay)
    {
        return None;
    }
    let observed = observe_enabled_rel32_callsite(
        D90_HOOK_EXPECTATION,
        installed.callsite,
        installed.relay,
        stored.teardown_owner_token,
    )
    .ok()?;
    (observed == stored).then_some(stored)
}

/// Consume the exact release receipt minted only after D90's original CALL
/// bytes, callback quiescence, callsite removal, and relay release are all
/// attested.
pub fn take_released_hook_receipt() -> Option<ReleasedRvaHookReceipt> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    RELEASED_HOOK_RECEIPT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
}

pub fn convoy_terminal_hook_heartbeat() -> ConvoyTerminalHookHeartbeat {
    let resolver = try_current_convoy_terminal_resolver_snapshot()
        .map(|snapshot| snapshot.receipt())
        .unwrap_or_default();
    ConvoyTerminalHookHeartbeat {
        installed: convoy_terminal_hook_installed(),
        teardown_requested: TEARDOWN.load(Ordering::Acquire),
        active_callbacks: ACTIVE_CALLBACKS.load(Ordering::Acquire),
        resolver,
        telemetry: telemetry(),
        producer_sequence_next: PRODUCER_SEQUENCE.load(Ordering::Acquire),
    }
}

/// Reserve one immutable bounded prefix without consuming it. Only one
/// reservation may exist; callbacks can continue appending behind the prefix.
pub fn peek_convoy_terminal_events(
    max_events: usize,
) -> Result<ConvoyTerminalDrainReservation, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "D90 hook lifecycle state poisoned during reservation".to_string())?;
    let state = INSTALL_STATE
        .lock()
        .map_err(|_| "D90 install state poisoned during reservation".to_string())?;
    if !INSTALLED.load(Ordering::Acquire) && !SEALED_PUBLICATION_PENDING.load(Ordering::Acquire) {
        return Err("D90 event reservation requires an installed or sealed hook epoch".into());
    }
    if INSTALLED.load(Ordering::Acquire)
        && !state
            .as_ref()
            .is_some_and(|installed| installed.patch_enabled && !installed.patch_restored)
    {
        return Err("D90 event reservation requires an installed hook".into());
    }
    peek_convoy_terminal_events_inner(max_events, false)
}

fn peek_convoy_terminal_events_inner(
    max_events: usize,
    allow_quiesced_teardown: bool,
) -> Result<ConvoyTerminalDrainReservation, String> {
    if allow_quiesced_teardown
        && (ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 || !TEARDOWN.load(Ordering::Acquire))
    {
        return Err("D90 sealed reservation boundary is not quiescent".into());
    }
    let mut reservation = DRAIN_RESERVATION
        .get_or_init(|| Mutex::new(DrainReservationState::default()))
        .lock()
        .map_err(|_| "D90 drain reservation state poisoned".to_string())?;
    if let Some(active) = &reservation.active {
        return Err(format!(
            "D90 drain reservation {} is already active",
            active.reservation_id
        ));
    }
    if SEALED_PUBLICATION_PENDING.load(Ordering::Acquire) {
        let drain = reservation
            .sealed_snapshot
            .clone()
            .ok_or("D90 sealed publication snapshot is unavailable")?;
        if max_events < drain.events.len() {
            return Err(format!(
                "D90 sealed publication requires capacity for {} events",
                drain.events.len()
            ));
        }
        let reservation_id = reservation.next_reservation_id;
        reservation.next_reservation_id = reservation
            .next_reservation_id
            .checked_add(1)
            .ok_or("D90 drain reservation ID exhausted")?;
        reservation.active = Some(ActiveDrainReservation {
            reservation_id,
            drain: drain.clone(),
        });
        return Ok(ConvoyTerminalDrainReservation {
            reservation_id,
            drain,
        });
    }

    let telemetry_before = telemetry();
    let active_before = ACTIVE_CALLBACKS.load(Ordering::Acquire);
    let producer_before = PRODUCER_SEQUENCE.load(Ordering::Acquire);
    let resolver_before = current_convoy_terminal_publication_receipt();
    let queue = EVENT_QUEUE.get_or_init(|| Mutex::new(VecDeque::new()));
    let queue = queue
        .lock()
        .map_err(|_| "D90 event queue poisoned during reservation".to_string())?;
    let take = max_events.min(EVENT_QUEUE_CAPACITY).min(queue.len());
    let events = queue.iter().take(take).copied().collect::<Vec<_>>();
    let remaining_events = u32::try_from(queue.len().saturating_sub(take)).unwrap_or(u32::MAX);
    let queue_mutation_generation = QUEUE_MUTATION_GENERATION.load(Ordering::Acquire);
    let telemetry_after = telemetry();
    let active_after = ACTIVE_CALLBACKS.load(Ordering::Acquire);
    let producer_after = PRODUCER_SEQUENCE.load(Ordering::Acquire);
    let resolver_after = current_convoy_terminal_publication_receipt();
    drop(queue);

    let correlation_poisoned = telemetry_before != telemetry_after
        || active_before != active_after
        || producer_before != producer_after
        || resolver_before.is_err()
        || resolver_after.is_err()
        || resolver_before != resolver_after;
    let drain = ConvoyTerminalDrain {
        events,
        cumulative_telemetry: telemetry_after,
        queue_drain_complete: remaining_events == 0 && !correlation_poisoned,
        correlation_poisoned,
        remaining_events,
        active_callbacks: u32::try_from(active_after).unwrap_or(u32::MAX),
        producer_sequence_next: producer_after,
        queue_mutation_generation,
        resolver: resolver_after.unwrap_or_default(),
    };
    let reservation_id = reservation.next_reservation_id;
    reservation.next_reservation_id = reservation
        .next_reservation_id
        .checked_add(1)
        .ok_or("D90 drain reservation ID exhausted")?;
    reservation.active = Some(ActiveDrainReservation {
        reservation_id,
        drain: drain.clone(),
    });
    Ok(ConvoyTerminalDrainReservation {
        reservation_id,
        drain,
    })
}

pub fn commit_convoy_terminal_events(reservation_id: u64) -> Result<(), String> {
    let mut reservation = DRAIN_RESERVATION
        .get_or_init(|| Mutex::new(DrainReservationState::default()))
        .lock()
        .map_err(|_| "D90 drain reservation state poisoned".to_string())?;
    let active = reservation
        .active
        .as_ref()
        .ok_or("D90 has no active drain reservation")?;
    if active.reservation_id != reservation_id {
        return Err(format!(
            "D90 drain reservation mismatch: active {}, supplied {reservation_id}",
            active.reservation_id
        ));
    }
    let queue = EVENT_QUEUE.get_or_init(|| Mutex::new(VecDeque::new()));
    let mut queue = queue
        .lock()
        .map_err(|_| "D90 event queue poisoned during reservation commit".to_string())?;
    if queue.len() < active.drain.events.len()
        || !queue
            .iter()
            .take(active.drain.events.len())
            .copied()
            .eq(active.drain.events.iter().copied())
    {
        return Err("D90 reserved event prefix changed before commit".into());
    }
    if !active.drain.events.is_empty() {
        advance_queue_mutation_generation()?;
    }
    queue.drain(..active.drain.events.len());
    let sealed_complete =
        active.drain.remaining_events == 0 && SEALED_PUBLICATION_PENDING.load(Ordering::Acquire);
    drop(queue);
    reservation.active = None;
    if sealed_complete {
        SEALED_PUBLICATION_PENDING.store(false, Ordering::Release);
        reservation.sealed_snapshot = None;
    }
    Ok(())
}

pub fn rollback_convoy_terminal_events(reservation_id: u64) -> Result<(), String> {
    let mut reservation = DRAIN_RESERVATION
        .get_or_init(|| Mutex::new(DrainReservationState::default()))
        .lock()
        .map_err(|_| "D90 drain reservation state poisoned".to_string())?;
    let active = reservation
        .active
        .as_ref()
        .ok_or("D90 has no active drain reservation")?;
    if active.reservation_id != reservation_id {
        return Err(format!(
            "D90 drain reservation mismatch: active {}, supplied {reservation_id}",
            active.reservation_id
        ));
    }
    reservation.active = None;
    Ok(())
}

/// Encode one reserved prefix. Encoding failure releases the reservation;
/// successful encoding leaves it owned by the caller until sink publication
/// is explicitly committed or rolled back.
pub fn peek_convoy_transport_attack_payload(
    max_events: usize,
) -> Result<Option<ConvoyTerminalPayloadReservation>, ConvoyTerminalSealError> {
    if !INSTALLED.load(Ordering::Acquire) && !SEALED_PUBLICATION_PENDING.load(Ordering::Acquire) {
        return Ok(None);
    }
    let reservation = peek_convoy_terminal_events(max_events)?;
    match encode_convoy_transport_attack_drain(reservation.drain, true) {
        Ok(payload) => Ok(Some(ConvoyTerminalPayloadReservation {
            reservation_id: reservation.reservation_id,
            payload,
        })),
        Err(error) => {
            rollback_convoy_terminal_events(reservation.reservation_id)?;
            Err(error.into())
        }
    }
}

pub fn commit_convoy_transport_attack_payload(reservation_id: u64) -> Result<(), String> {
    commit_convoy_terminal_events(reservation_id)
}

pub fn rollback_convoy_transport_attack_payload(reservation_id: u64) -> Result<(), String> {
    rollback_convoy_terminal_events(reservation_id)
}

/// Seals, restores and discards the whole D90 carrier for a production hook
/// registry that could not complete its install.
///
/// The normal teardown only consumes an empty sealed payload, because a
/// non-empty one is owed to the terminal DATA frame. A partial install has no
/// terminal frame and never will, so a non-empty payload is discarded here with
/// an exact row count rather than refused forever. The physical half — callsite
/// restoration, quiescence and release-receipt observation — is the same seal
/// the normal path uses, and a failed discard rolls the reservation back so the
/// exact carrier stays retryable.
pub fn abort_partial_install_and_uninstall(
    max_events: usize,
) -> Result<InstallAbortDiscard, String> {
    let reservation = seal_and_drain_convoy_transport_attack_payload(max_events)
        .map_err(|error| format!("D90 install abort seal failed: {error:?}"))?;
    let Some(reservation) = reservation else {
        // The lane held no installed lease; nothing to discard, and the seal
        // has already finalized any release-only custody.
        return Ok(InstallAbortDiscard::default());
    };
    let decoded = match crate::decode_convoy_transport_attack_batch(&reservation.payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            let _ = rollback_convoy_transport_attack_payload(reservation.reservation_id);
            return Err(format!("D90 install abort decode failed: {error:?}"));
        }
    };
    let discard = InstallAbortDiscard {
        retained_rows: decoded.rows.len() as u64,
        reserved_rows: decoded.rows.len() as u64,
        discarded_diagnostics: 0,
        poisoned: !decoded.exact_events_complete,
    };
    if let Err(error) = commit_convoy_transport_attack_payload(reservation.reservation_id) {
        let _ = rollback_convoy_transport_attack_payload(reservation.reservation_id);
        return Err(format!("D90 install abort discard failed: {error}"));
    }
    Ok(discard)
}

fn encode_convoy_transport_attack_drain(
    drain: ConvoyTerminalDrain,
    observed_hook_installed: bool,
) -> Result<Vec<u8>, BatchDecodeError> {
    let rows = drain
        .events
        .iter()
        .map(|event| ConvoyTransportAttackRow {
            carrier: event.carrier,
            d70: ConvoyD70MutationProof {
                kind: match event.d70.kind {
                    D70MutationKind::IncrementExisting => ConvoyD70MutationKind::IncrementExisting,
                    D70MutationKind::AppendFirstSeen => ConvoyD70MutationKind::AppendFirstSeen,
                },
                ordinal: event.d70.ordinal,
                before_rows: event.d70.before_rows,
                after_rows: event.d70.after_rows,
                before_count: event.d70.before_count,
                after_count: event.d70.after_count,
            },
        })
        .collect::<Vec<_>>();
    let telemetry = drain.cumulative_telemetry;
    let witness = ConvoyTransportAttackWitness {
        hook_installed: observed_hook_installed,
        queue_drain_complete: drain.queue_drain_complete && !drain.correlation_poisoned,
        producer_sequence_next: drain.producer_sequence_next,
        drained_rows: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        remaining_events: drain.remaining_events,
        active_callbacks: drain.active_callbacks,
        resolver_publication_sequence: drain.resolver.publication_sequence,
        resolver_game_hour_raw: drain.resolver.game_hour_raw,
        resolver_member_bindings: drain.resolver.member_bindings,
        resolver_route_bindings: drain.resolver.route_bindings,
        telemetry: ConvoyTransportAttackTelemetry {
            callbacks_seen: telemetry.callbacks_seen,
            original_calls: telemetry.original_calls,
            events_emitted: telemetry.events_emitted,
            dropped_no_original: telemetry.dropped_no_original,
            dropped_teardown: telemetry.dropped_teardown,
            dropped_nested: telemetry.dropped_nested,
            dropped_capture_busy: telemetry.dropped_capture_busy,
            dropped_hour_or_epoch: telemetry.dropped_hour_or_epoch,
            dropped_resolver: telemetry.dropped_resolver,
            dropped_candidate: telemetry.dropped_candidate,
            dropped_d70_pre: telemetry.dropped_d70_pre,
            dropped_d70_post: telemetry.dropped_d70_post,
            dropped_d70_mutation: telemetry.dropped_d70_mutation,
            dropped_queue_busy: telemetry.dropped_queue_busy,
            dropped_queue_full: telemetry.dropped_queue_full,
        },
    };
    encode_convoy_transport_attack_batch(&rows, witness)
}

fn telemetry() -> ConvoyTerminalHookTelemetry {
    let load = |counter: &AtomicU64| counter.load(Ordering::Acquire);
    ConvoyTerminalHookTelemetry {
        callbacks_seen: load(&CALLBACKS_SEEN),
        original_calls: load(&ORIGINAL_CALLS),
        events_emitted: load(&EVENTS_EMITTED),
        dropped_no_original: load(&DROP_NO_ORIGINAL),
        dropped_teardown: load(&DROP_TEARDOWN),
        dropped_nested: load(&DROP_NESTED),
        dropped_capture_busy: load(&DROP_CAPTURE_BUSY),
        dropped_hour_or_epoch: load(&DROP_HOUR_OR_EPOCH),
        dropped_resolver: load(&DROP_RESOLVER),
        dropped_candidate: load(&DROP_CANDIDATE),
        dropped_d70_pre: load(&DROP_D70_PRE),
        dropped_d70_post: load(&DROP_D70_POST),
        dropped_d70_mutation: load(&DROP_D70_MUTATION),
        dropped_queue_busy: load(&DROP_QUEUE_BUSY),
        dropped_queue_full: load(&DROP_QUEUE_FULL),
    }
}

fn reset_observer_state_for_install() {
    for counter in [
        &CALLBACKS_SEEN,
        &ORIGINAL_CALLS,
        &EVENTS_EMITTED,
        &DROP_NO_ORIGINAL,
        &DROP_TEARDOWN,
        &DROP_NESTED,
        &DROP_CAPTURE_BUSY,
        &DROP_HOUR_OR_EPOCH,
        &DROP_RESOLVER,
        &DROP_CANDIDATE,
        &DROP_D70_PRE,
        &DROP_D70_POST,
        &DROP_D70_MUTATION,
        &DROP_QUEUE_BUSY,
        &DROP_QUEUE_FULL,
    ] {
        counter.store(0, Ordering::Release);
    }
    PRODUCER_SEQUENCE.store(1, Ordering::Release);
    QUEUE_MUTATION_GENERATION.store(1, Ordering::Release);
    SEALED_PUBLICATION_PENDING.store(false, Ordering::Release);
    if let Some(reservation) = DRAIN_RESERVATION.get() {
        reservation
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .sealed_snapshot = None;
    }
    if let Some(queue) = EVENT_QUEUE.get() {
        queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
    }
    if let Some(slot) = POSITIVE_RAW_OBSERVATION.get() {
        *slot.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    }
    EVIDENCE_HEARTBEAT_SEQUENCE.store(1, Ordering::Release);
}

unsafe extern "system" fn convoy_terminal_hook(
    game_state: usize,
    count: u32,
    killer_ref: usize,
    owner_ref: usize,
) -> usize {
    let _active = ActiveCallback::enter();
    CALLBACKS_SEEN.fetch_add(1, Ordering::Relaxed);
    let original_address = ORIGINAL_TARGET.load(Ordering::Acquire);
    if original_address == 0 {
        DROP_NO_ORIGINAL.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    let original: TerminalAggregator = unsafe { core::mem::transmute(original_address) };
    if TEARDOWN.load(Ordering::Acquire) {
        DROP_TEARDOWN.fetch_add(1, Ordering::Relaxed);
        let result = unsafe { original(game_state, count, killer_ref, owner_ref) };
        ORIGINAL_CALLS.fetch_add(1, Ordering::Relaxed);
        return result;
    }
    let Some(_nesting) = CallbackNesting::try_enter() else {
        DROP_NESTED.fetch_add(1, Ordering::Relaxed);
        let result = unsafe { original(game_state, count, killer_ref, owner_ref) };
        ORIGINAL_CALLS.fetch_add(1, Ordering::Relaxed);
        return result;
    };
    let Some(_capture) = CaptureLease::try_acquire() else {
        DROP_CAPTURE_BUSY.fetch_add(1, Ordering::Relaxed);
        let result = unsafe { original(game_state, count, killer_ref, owner_ref) };
        ORIGINAL_CALLS.fetch_add(1, Ordering::Relaxed);
        return result;
    };

    let pre = prepare_observation(game_state, count, killer_ref, owner_ref);
    let result = unsafe { original(game_state, count, killer_ref, owner_ref) };
    ORIGINAL_CALLS.fetch_add(1, Ordering::Relaxed);
    if let Some(pre) = pre {
        complete_observation(game_state, pre);
    }
    result
}

fn prepare_observation(
    game_state: usize,
    count: u32,
    killer_ref: usize,
    owner_ref: usize,
) -> Option<PreObservation> {
    if count != 1 || game_state == 0 || killer_ref == 0 || owner_ref < 0x20 {
        DROP_CANDIDATE.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let memory = ProcessMemory;
    let Some(hour) = read_stable_i32(&memory, game_state.checked_add(GS_CURRENT_HOUR_OFF)?) else {
        DROP_HOUR_OR_EPOCH.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let Some(resolver) = try_current_convoy_terminal_resolver_snapshot() else {
        DROP_RESOLVER.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let receipt = resolver.receipt();
    if receipt.publication_sequence == 0 || receipt.game_hour_raw != hour {
        DROP_HOUR_OR_EPOCH.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let sequence = PRODUCER_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if sequence == 0 {
        DROP_CANDIDATE.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let member = owner_ref - 0x20;
    let registers = A729PreCollapseRegisters {
        rcx_game_state: game_state,
        edx_count: count,
        r8_killer_country_ref: killer_ref,
        r9_owner_country_ref: owner_ref,
        r12_member: member,
        r15_killer_country_ref: killer_ref,
    };
    let module_base = MODULE_BASE.load(Ordering::Acquire);
    let carrier = match capture_candidate(
        &memory,
        module_base,
        registers,
        sequence,
        hour,
        resolver.as_ref(),
        resolver.as_ref(),
    ) {
        Ok(carrier) => carrier,
        Err(_) => {
            DROP_CANDIDATE.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    let mut context_bytes = vec![0u8; crate::convoy_terminal_attribution::A729_CONTEXT_BYTES.len()];
    if !memory.read_exact(
        module_base.checked_add(crate::convoy_terminal_attribution::A729_CONTEXT_RVA)?,
        &mut context_bytes,
    ) || context_bytes != crate::convoy_terminal_attribution::A729_CONTEXT_BYTES
    {
        DROP_CANDIDATE.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let member_bytes = match read_exact_bytes(&memory, member, 0x124) {
        Some(bytes) => bytes,
        None => {
            DROP_CANDIDATE.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    let route = match capture_route_raw(&memory, module_base, carrier.client) {
        Some(route)
            if route.producer_kind == carrier.route.producer_kind as u8
                && route.source_kind == carrier.route.source.kind as u8
                && route.destination_kind == carrier.route.destination.kind as u8
                && route.source_id == carrier.route.source.id
                && route.destination_id == carrier.route.destination.id =>
        {
            route
        }
        _ => {
            DROP_CANDIDATE.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    let d70 = match read_d70(&memory, module_base, game_state) {
        Some(rows) => rows,
        None => {
            DROP_D70_PRE.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    let raw = HConvoyPositiveRawObservation {
        module_base,
        context_bytes,
        game_hour_raw: hour,
        rcx: registers.rcx_game_state,
        edx: registers.edx_count,
        r8: registers.r8_killer_country_ref,
        r9: registers.r9_owner_country_ref,
        r12: registers.r12_member,
        r15: registers.r15_killer_country_ref,
        killer_tag_db_id: carrier.killer_tag_db_id,
        member_bytes,
        combat_id: carrier.combat.combat_id,
        combat_generation: carrier.combat.generation,
        route,
        d70_before: d70.clone(),
        d70_after: Vec::new(),
    };
    Some(PreObservation { carrier, d70, raw })
}

fn complete_observation(game_state: usize, pre: PreObservation) {
    let memory = ProcessMemory;
    let Some(hour) = read_stable_i32(
        &memory,
        game_state.checked_add(GS_CURRENT_HOUR_OFF).unwrap_or(0),
    ) else {
        DROP_HOUR_OR_EPOCH.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if hour != pre.carrier.game_hour_raw {
        DROP_HOUR_OR_EPOCH.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let module_base = MODULE_BASE.load(Ordering::Acquire);
    let Some(after) = read_d70(&memory, module_base, game_state) else {
        DROP_D70_POST.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(d70) = prove_d70_mutation(
        &pre.d70,
        &after,
        pre.carrier.owner_tag_db_id,
        pre.carrier.killer_tag_db_id,
    ) else {
        DROP_D70_MUTATION.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut raw = pre.raw;
    raw.d70_after = after;
    let Ok(mut slot) = POSITIVE_RAW_OBSERVATION
        .get_or_init(|| Mutex::new(None))
        .try_lock()
    else {
        DROP_QUEUE_BUSY.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if slot.is_some() {
        DROP_CANDIDATE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    *slot = Some(raw);
    drop(slot);
    enqueue(ConvoyTerminalValidatedEvent {
        carrier: pre.carrier,
        d70,
    });
}

fn enqueue(event: ConvoyTerminalValidatedEvent) {
    let queue = EVENT_QUEUE.get_or_init(|| Mutex::new(VecDeque::new()));
    let Ok(mut queue) = queue.try_lock() else {
        DROP_QUEUE_BUSY.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if queue.len() >= EVENT_QUEUE_CAPACITY {
        DROP_QUEUE_FULL.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if advance_queue_mutation_generation().is_err() {
        DROP_QUEUE_FULL.fetch_add(1, Ordering::Relaxed);
        return;
    }
    queue.push_back(event);
    EVENTS_EMITTED.fetch_add(1, Ordering::Relaxed);
}

fn advance_queue_mutation_generation() -> Result<u64, String> {
    QUEUE_MUTATION_GENERATION
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
            generation.checked_add(1)
        })
        .map(|generation| generation + 1)
        .map_err(|_| "D90 queue mutation generation exhausted".into())
}

fn read_d70(
    memory: &ProcessMemory,
    module_base: usize,
    game_state: usize,
) -> Option<Vec<HConvoyD70Row>> {
    if module_base == 0 || game_state == 0 {
        return None;
    }
    let read_header = || {
        Some((
            memory.u64(game_state.checked_add(GS_D70_BEGIN_OFF)?)? as usize,
            memory.u32(game_state.checked_add(GS_D70_CAPACITY_OFF)?)?,
            memory.u32(game_state.checked_add(GS_D70_COUNT_OFF)?)?,
        ))
    };
    let first = read_header()?;
    let (begin, capacity, count) = first;
    if count > capacity || count as usize > MAX_D70_ROWS || (count != 0 && begin == 0) {
        return None;
    }
    let expected_vtable = module_base.checked_add(SUNK_CONVOY_INFO_VTABLE_RVA)? as u64;
    let mut identities = BTreeSet::new();
    let mut rows = Vec::with_capacity(count as usize);
    for ordinal in 0..count as usize {
        let slot = begin.checked_add(ordinal.checked_mul(8)?)?;
        let object = memory.u64(slot)? as usize;
        if object == 0 || memory.u64(object)? != expected_vtable {
            return None;
        }
        let fields = || {
            Some(HConvoyD70Row {
                identity: HConvoyD70Identity {
                    month_raw: read_i32(memory, object.checked_add(0x08)?)?,
                    killer_tag_db_id: memory.u32(object.checked_add(0x10)?)?,
                    owner_tag_db_id: memory.u32(object.checked_add(0x14)?)?,
                },
                count: memory.u32(object.checked_add(0x0C)?)?,
            })
        };
        let row = fields()?;
        if fields()? != row
            || row.count == 0
            || row.count > MAX_CONVOYS_PER_D70_ROW
            || row.identity.killer_tag_db_id == 0
            || row.identity.killer_tag_db_id > MAX_COUNTRY_TAG_DB_ID
            || row.identity.owner_tag_db_id == 0
            || row.identity.owner_tag_db_id > MAX_COUNTRY_TAG_DB_ID
            || !identities.insert(row.identity)
        {
            return None;
        }
        rows.push(row);
    }
    (read_header()? == first).then_some(rows)
}

fn prove_d70_mutation(
    before: &[HConvoyD70Row],
    after: &[HConvoyD70Row],
    owner: u32,
    killer: u32,
) -> Option<D70MutationProof> {
    if after.len() == before.len() {
        let mut changed = None;
        for (ordinal, (old, new)) in before.iter().zip(after).enumerate() {
            if old == new {
                continue;
            }
            if changed.is_some()
                || old.identity != new.identity
                || old.identity.owner_tag_db_id != owner
                || old.identity.killer_tag_db_id != killer
                || new.count != old.count.checked_add(1)?
            {
                return None;
            }
            changed = Some((ordinal, old.count, new.count));
        }
        let (ordinal, before_count, after_count) = changed?;
        return Some(D70MutationProof {
            kind: D70MutationKind::IncrementExisting,
            ordinal: u32::try_from(ordinal).ok()?,
            before_rows: u32::try_from(before.len()).ok()?,
            after_rows: u32::try_from(after.len()).ok()?,
            before_count,
            after_count,
        });
    }
    if after.len() != before.len().checked_add(1)? || after.get(..before.len())? != before {
        return None;
    }
    let appended = after.last()?;
    if appended.identity.owner_tag_db_id != owner
        || appended.identity.killer_tag_db_id != killer
        || appended.count != 1
    {
        return None;
    }
    Some(D70MutationProof {
        kind: D70MutationKind::AppendFirstSeen,
        ordinal: u32::try_from(before.len()).ok()?,
        before_rows: u32::try_from(before.len()).ok()?,
        after_rows: u32::try_from(after.len()).ok()?,
        before_count: 0,
        after_count: 1,
    })
}

fn read_i32(memory: &ProcessMemory, address: usize) -> Option<i32> {
    memory
        .u32(address)
        .map(|value| i32::from_le_bytes(value.to_le_bytes()))
}

fn read_stable_i32(memory: &ProcessMemory, address: usize) -> Option<i32> {
    let first = read_i32(memory, address)?;
    (read_i32(memory, address)? == first).then_some(first)
}

/// Install only after the fresh exact-a729 static authority and compile-time
/// activation gate are present. Any missing gate returns `PromotionBlocked`
/// before process mutation.
pub fn install_convoy_terminal_hook(
    executable_sha256: &str,
) -> Result<(), ConvoyTerminalInstallError> {
    let blockers = activation_blockers();
    if !blockers.is_empty() {
        return Err(ConvoyTerminalInstallError::PromotionBlocked(blockers));
    }
    install_convoy_terminal_hook_inner(executable_sha256, HookInstallMode::Production)
}

/// Install the exact hook only for the isolated H-CONVOY loss/no-loss capture.
/// This regression-only acquisition path does not mint production authority.
pub fn install_convoy_terminal_evidence_hook(
    executable_sha256: &str,
) -> Result<(), ConvoyTerminalInstallError> {
    if !convoy_terminal_evidence_activation_enabled() {
        return Err(ConvoyTerminalInstallError::EvidenceActivationDisabled);
    }
    install_convoy_terminal_hook_inner(executable_sha256, HookInstallMode::Evidence)
}

fn install_convoy_terminal_hook_inner(
    executable_sha256: &str,
    mode: HookInstallMode,
) -> Result<(), ConvoyTerminalInstallError> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if !executable_sha256.eq_ignore_ascii_case(A729_EXE_SHA256) {
        return Err(ConvoyTerminalInstallError::BinaryGate);
    }
    // The caller's expected hash is a policy input, not an observation. Hash
    // the executable backing the current process before any allocation or
    // code mutation so passing the a729 constant cannot make the binary gate
    // tautological. The exact in-memory call window is revalidated below.
    if current_process_executable_sha256()?.as_str() != A729_EXE_SHA256 {
        return Err(ConvoyTerminalInstallError::BinaryGate);
    }
    let mut state = INSTALL_STATE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(existing) = *state {
        return Err(if existing.recovery_pending {
            ConvoyTerminalInstallError::RecoveryPending
        } else if existing.patch_restored {
            ConvoyTerminalInstallError::TeardownPending
        } else {
            ConvoyTerminalInstallError::AlreadyInstalled
        });
    }
    if RELEASED_HOOK_RECEIPT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .is_some()
    {
        return Err(ConvoyTerminalInstallError::UntakenReleaseReceipt);
    }
    if DRAIN_RESERVATION
        .get_or_init(|| Mutex::new(DrainReservationState::default()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .active
        .is_some()
    {
        return Err(ConvoyTerminalInstallError::ReservationPending);
    }
    if SEALED_PUBLICATION_PENDING.load(Ordering::Acquire) {
        return Err(ConvoyTerminalInstallError::ReservationPending);
    }
    let module_base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if module_base == 0 {
        return Err(ConvoyTerminalInstallError::ModuleBase);
    }
    let memory = ProcessMemory;
    let prepared = prepare_a729_hook(&memory, module_base, executable_sha256)
        .map_err(|_| ConvoyTerminalInstallError::BinaryGate)?;
    if mode.is_production() && !prepared.activation_blockers.is_empty() {
        return Err(ConvoyTerminalInstallError::PromotionBlocked(
            prepared.activation_blockers,
        ));
    }
    let teardown_owner_token = mode
        .is_production()
        .then(next_teardown_owner_token)
        .transpose()
        .map_err(ConvoyTerminalInstallError::HookReceipt)?;
    let relay = unsafe { allocate_near_relay(prepared.hook_address)? };
    let patched_call = match make_rel32_call(prepared.hook_address, relay as usize) {
        Some(bytes) => bytes,
        None => {
            unsafe { VirtualFree(relay.cast(), 0, MEM_RELEASE) };
            return Err(ConvoyTerminalInstallError::Rel32OutOfRange);
        }
    };
    let suspended = unsafe { SuspendedProcessThreads::suspend_all_other_threads()? };
    if prepare_a729_hook(&memory, module_base, executable_sha256).is_err() {
        drop(suspended);
        unsafe { VirtualFree(relay.cast(), 0, MEM_RELEASE) };
        return Err(ConvoyTerminalInstallError::ByteGateDrift);
    }
    let mut original_call = [0u8; A729_CALL_BYTES.len()];
    if !memory.read_exact(prepared.hook_address, &mut original_call)
        || original_call != A729_CALL_BYTES
    {
        drop(suspended);
        unsafe { VirtualFree(relay.cast(), 0, MEM_RELEASE) };
        return Err(ConvoyTerminalInstallError::ByteGateDrift);
    }
    reset_observer_state_for_install();
    MODULE_BASE.store(module_base, Ordering::Release);
    ORIGINAL_TARGET.store(prepared.original_target_address, Ordering::Release);
    TEARDOWN.store(false, Ordering::Release);
    if let Err(error) = unsafe { write_code(prepared.hook_address, &patched_call) } {
        let error = rollback_failed_install(
            &mut state,
            &memory,
            mode,
            teardown_owner_token,
            prepared.hook_address,
            relay as usize,
            original_call,
            patched_call,
            error,
        );
        drop(suspended);
        return Err(error);
    }
    let mut exact_patch = [0u8; A729_CALL_BYTES.len()];
    if !memory.read_exact(prepared.hook_address, &mut exact_patch) || exact_patch != patched_call {
        let error = rollback_failed_install(
            &mut state,
            &memory,
            mode,
            teardown_owner_token,
            prepared.hook_address,
            relay as usize,
            original_call,
            patched_call,
            ConvoyTerminalInstallError::PatchedBytesDrift,
        );
        drop(suspended);
        return Err(error);
    }
    if !relay_targets_convoy_terminal_hook(&memory, relay as usize) {
        let error = rollback_failed_install(
            &mut state,
            &memory,
            mode,
            teardown_owner_token,
            prepared.hook_address,
            relay as usize,
            original_call,
            patched_call,
            ConvoyTerminalInstallError::RelayDrift,
        );
        drop(suspended);
        return Err(error);
    }
    let installed_receipt = match teardown_owner_token {
        Some(token) => match observe_enabled_rel32_callsite(
            D90_HOOK_EXPECTATION,
            prepared.hook_address,
            relay as usize,
            token,
        ) {
            Ok(receipt) => Some(receipt),
            Err(error) => {
                let error = rollback_failed_install(
                    &mut state,
                    &memory,
                    mode,
                    teardown_owner_token,
                    prepared.hook_address,
                    relay as usize,
                    original_call,
                    patched_call,
                    ConvoyTerminalInstallError::HookReceipt(error),
                );
                drop(suspended);
                return Err(error);
            }
        },
        None => None,
    };
    *state = Some(InstalledState {
        mode,
        callsite: prepared.hook_address,
        relay: relay as usize,
        original_call,
        patched_call,
        owned_mutation: Some(patched_call),
        install_admitted: true,
        patch_enabled: true,
        patch_restored: false,
        relay_freed: false,
        relay_verified_before_removal: false,
        recovery_pending: false,
        teardown_owner_token,
        installed_receipt,
        released_receipt: None,
    });
    INSTALLED.store(true, Ordering::Release);
    drop(suspended);
    Ok(())
}

fn current_process_executable_sha256() -> Result<String, ConvoyTerminalInstallError> {
    let mut path = [0u16; 32_768];
    let len = unsafe { GetModuleFileNameW(ptr::null_mut(), path.as_mut_ptr(), path.len() as Dword) }
        as usize;
    if len == 0 || len >= path.len() {
        return Err(ConvoyTerminalInstallError::BinaryGate);
    }
    let path = PathBuf::from(OsString::from_wide(&path[..len]));
    let mut file = std::fs::File::open(path).map_err(|_| ConvoyTerminalInstallError::BinaryGate)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| ConvoyTerminalInstallError::BinaryGate)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[allow(clippy::too_many_arguments)]
fn rollback_failed_install(
    state: &mut Option<InstalledState>,
    memory: &ProcessMemory,
    mode: HookInstallMode,
    teardown_owner_token: Option<u64>,
    callsite: usize,
    relay: usize,
    original_call: [u8; 5],
    patched_call: [u8; 5],
    install_error: ConvoyTerminalInstallError,
) -> ConvoyTerminalInstallError {
    let install_error_text = format!("{install_error:?}");
    let mut observed = [0u8; A729_CALL_BYTES.len()];
    let observed_before = memory
        .read_exact(callsite, &mut observed)
        .then_some(observed);

    let (patch_restored, owned_mutation, rollback_error) = match observed_before {
        Some(bytes) if bytes == original_call => (true, Some(bytes), None),
        Some(_bytes) => {
            let write_result = unsafe { write_code(callsite, &original_call) };
            let mut after = [0u8; A729_CALL_BYTES.len()];
            let observed_after = memory.read_exact(callsite, &mut after).then_some(after);
            match (write_result, observed_after) {
                (Ok(()), Some(restored)) if restored == original_call => {
                    (true, Some(restored), None)
                }
                (write_result, observed_after) => {
                    let error = match write_result {
                        Err(error) => format!("{error:?}"),
                        Ok(()) => "exact original bytes were not observed after rollback".into(),
                    };
                    (false, observed_after, Some(error))
                }
            }
        }
        None => (
            false,
            None,
            Some("failed to observe D90 bytes before guarded rollback".into()),
        ),
    };

    if patch_restored && rollback_error.is_none() {
        if unsafe { VirtualFree(relay as *mut c_void, 0, MEM_RELEASE) } != 0 {
            ORIGINAL_TARGET.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            EVIDENCE_GAME_STATE.store(0, Ordering::Release);
            EVIDENCE_GAME_HOUR.store(0, Ordering::Release);
            TEARDOWN.store(false, Ordering::Release);
            INSTALLED.store(false, Ordering::Release);
            reset_observer_state_for_install();
            *state = None;
            return install_error;
        }
    }

    let rollback_error = rollback_error.unwrap_or_else(|| "D90 relay release failed".into());
    *state = Some(InstalledState {
        mode,
        callsite,
        relay,
        original_call,
        patched_call,
        owned_mutation,
        install_admitted: false,
        patch_enabled: false,
        patch_restored,
        relay_freed: false,
        relay_verified_before_removal: false,
        recovery_pending: true,
        teardown_owner_token,
        installed_receipt: None,
        released_receipt: None,
    });
    INSTALLED.store(false, Ordering::Release);
    TEARDOWN.store(true, Ordering::Release);
    ConvoyTerminalInstallError::InstallRollbackFailed {
        install_error: install_error_text,
        rollback_error,
    }
}

fn restore_and_quiesce_convoy_terminal_hook(
) -> Result<Option<InstalledState>, ConvoyTerminalInstallError> {
    let mut state = INSTALL_STATE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(mut installed) = *state else {
        return Ok(None);
    };
    TEARDOWN.store(true, Ordering::Release);
    if !installed.patch_restored {
        let suspended = unsafe { SuspendedProcessThreads::suspend_all_other_threads()? };
        let memory = ProcessMemory;
        let mut current = [0u8; 5];
        if !memory.read_exact(installed.callsite, &mut current) {
            drop(suspended);
            return Err(ConvoyTerminalInstallError::PatchedBytesDrift);
        }
        let guard_matches = match installed.owned_mutation {
            Some(owned) => current == owned,
            None if installed.recovery_pending => {
                current == installed.original_call || current == installed.patched_call
            }
            None => false,
        };
        if !guard_matches
            || (!installed.recovery_pending
                && (!installed.patch_enabled || current != installed.patched_call))
        {
            drop(suspended);
            return Err(ConvoyTerminalInstallError::PatchedBytesDrift);
        }
        if installed.install_admitted {
            installed.relay_verified_before_removal =
                relay_targets_convoy_terminal_hook(&memory, installed.relay);
        }
        if let Err(error) = unsafe { write_code(installed.callsite, &installed.original_call) } {
            let mut observed = [0u8; 5];
            installed.owned_mutation = memory
                .read_exact(installed.callsite, &mut observed)
                .then_some(observed);
            installed.patch_enabled = false;
            installed.recovery_pending = true;
            *state = Some(installed);
            INSTALLED.store(false, Ordering::Release);
            drop(suspended);
            return Err(ConvoyTerminalInstallError::InstallRollbackFailed {
                install_error: "D90 teardown restoration failed".into(),
                rollback_error: format!("{error:?}"),
            });
        }
        let mut restored = [0u8; A729_CALL_BYTES.len()];
        let restored_observed = memory.read_exact(installed.callsite, &mut restored);
        if !restored_observed || restored != installed.original_call {
            installed.owned_mutation = restored_observed.then_some(restored);
            installed.patch_enabled = false;
            installed.recovery_pending = true;
            *state = Some(installed);
            INSTALLED.store(false, Ordering::Release);
            drop(suspended);
            return Err(ConvoyTerminalInstallError::InstallRollbackFailed {
                install_error: "D90 teardown restoration was not exact".into(),
                rollback_error: "exact original bytes were not observed after write".into(),
            });
        }
        installed.owned_mutation = Some(restored);
        installed.patch_enabled = false;
        installed.patch_restored = true;
        installed.recovery_pending = false;
        *state = Some(installed);
        INSTALLED.store(false, Ordering::Release);
        drop(suspended);
    }
    if unsafe {
        FlushInstructionCache(
            GetCurrentProcess(),
            installed.callsite as *const c_void,
            A729_CALL_BYTES.len(),
        )
    } == 0
    {
        return Err(ConvoyTerminalInstallError::PatchFlush);
    }
    let deadline = Instant::now() + QUIESCE_TIMEOUT;
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            // The callsite is restored. Retain the relay and original pointer
            // so an in-flight callback can finish safely; retrying teardown can
            // reclaim them after quiescence.
            return Err(ConvoyTerminalInstallError::QuiesceTimeout);
        }
        std::thread::yield_now();
    }
    if installed.install_admitted && !installed.relay_verified_before_removal {
        return Err(ConvoyTerminalInstallError::RelayDrift);
    }
    Ok(Some(installed))
}

fn finalize_convoy_terminal_uninstall(
    restored: Option<InstalledState>,
) -> Result<(), ConvoyTerminalInstallError> {
    let mut state = INSTALL_STATE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(mut installed) = *state else {
        if restored.is_some() {
            return Err(ConvoyTerminalInstallError::RecoveryPending);
        }
        clear_convoy_terminal_resolver_snapshot()
            .map_err(ConvoyTerminalInstallError::ResolverClear)?;
        ORIGINAL_TARGET.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        EVIDENCE_GAME_STATE.store(0, Ordering::Release);
        EVIDENCE_GAME_HOUR.store(0, Ordering::Release);
        TEARDOWN.store(false, Ordering::Release);
        return Ok(());
    };
    if !installed.patch_restored || ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        return Err(ConvoyTerminalInstallError::RecoveryPending);
    }
    if !installed.relay_freed {
        if unsafe { VirtualFree(installed.relay as *mut c_void, 0, MEM_RELEASE) } == 0 {
            return Err(ConvoyTerminalInstallError::RelayFree);
        }
        installed.relay_freed = true;
        *state = Some(installed);
    }
    if installed.mode.is_production() && installed.install_admitted {
        let receipt = installed.installed_receipt.ok_or_else(|| {
            ConvoyTerminalInstallError::HookReceipt(
                "D90 admitted production install lost its receipt".into(),
            )
        })?;
        if installed.teardown_owner_token != Some(receipt.teardown_owner_token) {
            return Err(ConvoyTerminalInstallError::HookReceipt(
                "D90 teardown owner token custody changed".into(),
            ));
        }
        if installed.released_receipt.is_none() {
            installed.released_receipt = Some(
                observe_released(
                    D90_HOOK_EXPECTATION,
                    receipt,
                    HookTeardownAttestation {
                        callbacks_quiesced: ACTIVE_CALLBACKS.load(Ordering::Acquire) == 0,
                        hook_removed: installed.patch_restored && !installed.patch_enabled,
                        relay_freed: installed.relay_freed,
                    },
                )
                .map_err(ConvoyTerminalInstallError::HookReceipt)?,
            );
            *state = Some(installed);
        }
    } else if installed.mode.is_production() {
        if installed.installed_receipt.is_some() || installed.released_receipt.is_some() {
            return Err(ConvoyTerminalInstallError::HookReceipt(
                "D90 failed production install retained receipt authority".into(),
            ));
        }
    } else if installed.teardown_owner_token.is_some()
        || installed.installed_receipt.is_some()
        || installed.released_receipt.is_some()
    {
        return Err(ConvoyTerminalInstallError::HookReceipt(
            "D90 evidence lifecycle acquired production receipt custody".into(),
        ));
    }
    clear_convoy_terminal_resolver_snapshot().map_err(ConvoyTerminalInstallError::ResolverClear)?;
    let released_receipt = installed.released_receipt;
    ORIGINAL_TARGET.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    EVIDENCE_GAME_STATE.store(0, Ordering::Release);
    EVIDENCE_GAME_HOUR.store(0, Ordering::Release);
    let mut released_slot = RELEASED_HOOK_RECEIPT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if released_receipt.is_some() && released_slot.is_some() {
        return Err(ConvoyTerminalInstallError::UntakenReleaseReceipt);
    }
    if let Some(receipt) = released_receipt {
        *released_slot = Some(receipt);
    }
    *state = None;
    TEARDOWN.store(false, Ordering::Release);
    Ok(())
}

/// Restore the callsite, wait for every in-flight callback, and reserve the
/// final queue prefix while the resolver receipt is still resident.
/// The returned token remains rollbackable until the caller confirms sink
/// publication; teardown never commits the queue on the caller's behalf.
pub fn seal_and_drain_convoy_transport_attack_payload(
    max_events: usize,
) -> Result<Option<ConvoyTerminalPayloadReservation>, ConvoyTerminalSealError> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "D90 hook lifecycle state poisoned during sealed teardown".to_string())?;
    if DRAIN_RESERVATION
        .get_or_init(|| Mutex::new(DrainReservationState::default()))
        .lock()
        .map_err(|_| "D90 drain reservation state poisoned".to_string())?
        .active
        .is_some()
    {
        return Err(ConvoyTerminalSealError::Reservation(
            "D90 sealed teardown refused an active reservation".into(),
        ));
    }
    let installed = restore_and_quiesce_convoy_terminal_hook()?;
    let Some(installed) = installed else {
        finalize_convoy_terminal_uninstall(None)?;
        return Ok(None);
    };
    let reservation = peek_convoy_terminal_events_inner(max_events, true)?;
    if reservation.drain.remaining_events != 0 {
        rollback_convoy_terminal_events(reservation.reservation_id)?;
        return Err(ConvoyTerminalSealError::Reservation(format!(
            "D90 sealed reservation capacity left {} event(s) behind",
            reservation.drain.remaining_events
        )));
    }
    let payload = match encode_convoy_transport_attack_drain(reservation.drain.clone(), true) {
        Ok(payload) => payload,
        Err(error) => {
            rollback_convoy_terminal_events(reservation.reservation_id)?;
            return Err(error.into());
        }
    };
    if let Err(error) = finalize_convoy_terminal_uninstall(Some(installed)) {
        rollback_convoy_terminal_events(reservation.reservation_id)?;
        return Err(error.into());
    }
    DRAIN_RESERVATION
        .get_or_init(|| Mutex::new(DrainReservationState::default()))
        .lock()
        .map_err(|_| "D90 drain reservation state poisoned".to_string())?
        .sealed_snapshot = Some(reservation.drain.clone());
    SEALED_PUBLICATION_PENDING.store(true, Ordering::Release);
    Ok(Some(ConvoyTerminalPayloadReservation {
        reservation_id: reservation.reservation_id,
        payload,
    }))
}

pub fn uninstall_convoy_terminal_hook() -> Result<(), ConvoyTerminalInstallError> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if DRAIN_RESERVATION
        .get_or_init(|| Mutex::new(DrainReservationState::default()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .active
        .is_some()
    {
        return Err(ConvoyTerminalInstallError::ReservationPending);
    }
    if INSTALL_STATE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .as_ref()
        .is_some_and(|installed| installed.mode.is_production() && installed.install_admitted)
    {
        return Err(ConvoyTerminalInstallError::SealedTeardownRequired);
    }
    let installed = restore_and_quiesce_convoy_terminal_hook()?;
    finalize_convoy_terminal_uninstall(installed)
}

unsafe fn allocate_near_relay(callsite: usize) -> Result<*mut u8, ConvoyTerminalInstallError> {
    let hook = convoy_terminal_hook as *const c_void;
    let aligned = callsite & !(ALLOCATION_GRANULARITY - 1);
    let mut distance = 0usize;
    while distance <= MAX_REL32_DISTANCE {
        let candidates = [aligned.checked_add(distance), aligned.checked_sub(distance)];
        for candidate in candidates.into_iter().flatten() {
            if make_rel32_call(callsite, candidate).is_none() {
                continue;
            }
            let relay = unsafe {
                VirtualAlloc(
                    candidate as *mut c_void,
                    RELAY_LEN,
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_READWRITE,
                ) as *mut u8
            };
            if relay.is_null() {
                continue;
            }
            let mut bytes = [0u8; RELAY_LEN];
            bytes[..6].copy_from_slice(&[0xFF, 0x25, 0, 0, 0, 0]);
            bytes[6..].copy_from_slice(&(hook as usize as u64).to_le_bytes());
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), relay, bytes.len()) };
            let mut old = 0;
            if unsafe { VirtualProtect(relay.cast(), RELAY_LEN, PAGE_EXECUTE_READ, &mut old) } == 0
            {
                unsafe { VirtualFree(relay.cast(), 0, MEM_RELEASE) };
                return Err(ConvoyTerminalInstallError::RelayProtect);
            }
            if unsafe { FlushInstructionCache(GetCurrentProcess(), relay.cast(), RELAY_LEN) } == 0 {
                unsafe { VirtualFree(relay.cast(), 0, MEM_RELEASE) };
                return Err(ConvoyTerminalInstallError::RelayFlush);
            }
            return Ok(relay);
        }
        distance = match distance.checked_add(ALLOCATION_GRANULARITY) {
            Some(next) => next,
            None => break,
        };
    }
    Err(ConvoyTerminalInstallError::RelayAllocation)
}

fn make_rel32_call(callsite: usize, destination: usize) -> Option<[u8; 5]> {
    let resume = callsite.checked_add(5)?;
    let displacement = (destination as i128) - (resume as i128);
    let displacement = i32::try_from(displacement).ok()?;
    let mut bytes = [0u8; 5];
    bytes[0] = 0xE8;
    bytes[1..].copy_from_slice(&displacement.to_le_bytes());
    Some(bytes)
}

fn relay_targets_convoy_terminal_hook(memory: &ProcessMemory, relay: usize) -> bool {
    let mut bytes = [0u8; RELAY_LEN];
    memory.read_exact(relay, &mut bytes)
        && bytes[..6] == [0xFF, 0x25, 0, 0, 0, 0]
        && u64::from_le_bytes(bytes[6..14].try_into().unwrap())
            == convoy_terminal_hook as usize as u64
}

unsafe fn write_code(address: usize, bytes: &[u8]) -> Result<(), ConvoyTerminalInstallError> {
    let mut old = 0;
    if unsafe {
        VirtualProtect(
            address as *mut c_void,
            bytes.len(),
            PAGE_EXECUTE_READWRITE,
            &mut old,
        )
    } == 0
    {
        return Err(ConvoyTerminalInstallError::PatchProtect);
    }
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len()) };
    let mut ignored = 0;
    if unsafe { VirtualProtect(address as *mut c_void, bytes.len(), old, &mut ignored) } == 0 {
        return Err(ConvoyTerminalInstallError::PatchProtect);
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), address as *const c_void, bytes.len()) }
        == 0
    {
        return Err(ConvoyTerminalInstallError::PatchFlush);
    }
    Ok(())
}

struct SuspendedProcessThreads {
    handles: Vec<Handle>,
}

impl SuspendedProcessThreads {
    unsafe fn suspend_all_other_threads() -> Result<Self, ConvoyTerminalInstallError> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(ConvoyTerminalInstallError::ThreadSnapshot);
        }
        let process_id = unsafe { GetCurrentProcessId() };
        let current_thread_id = unsafe { GetCurrentThreadId() };
        let mut entry = ThreadEntry32 {
            size: core::mem::size_of::<ThreadEntry32>() as Dword,
            usage: 0,
            thread_id: 0,
            owner_process_id: 0,
            base_priority: 0,
            delta_priority: 0,
            flags: 0,
        };
        let mut handles = Vec::new();
        let mut present = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while present {
            if entry.owner_process_id == process_id && entry.thread_id != current_thread_id {
                let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread_id) };
                if handle.is_null() {
                    let error = unsafe { GetLastError() };
                    if error == ERROR_INVALID_PARAMETER {
                        present = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
                        continue;
                    }
                    unsafe { CloseHandle(snapshot) };
                    resume_and_close(&mut handles);
                    return Err(ConvoyTerminalInstallError::ThreadOpen(entry.thread_id));
                }
                if unsafe { SuspendThread(handle) } == Dword::MAX {
                    let error = unsafe { GetLastError() };
                    unsafe { CloseHandle(handle) };
                    if error == ERROR_INVALID_HANDLE || error == ERROR_INVALID_PARAMETER {
                        present = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
                        continue;
                    }
                    unsafe { CloseHandle(snapshot) };
                    resume_and_close(&mut handles);
                    return Err(ConvoyTerminalInstallError::ThreadSuspend(entry.thread_id));
                }
                handles.push(handle);
            }
            present = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        let enumeration_error = unsafe { GetLastError() };
        unsafe { CloseHandle(snapshot) };
        if enumeration_error != 0 && enumeration_error != 18 {
            resume_and_close(&mut handles);
            return Err(ConvoyTerminalInstallError::ThreadEnumeration);
        }
        Ok(Self { handles })
    }
}

impl Drop for SuspendedProcessThreads {
    fn drop(&mut self) {
        resume_and_close(&mut self.handles);
    }
}

fn resume_and_close(handles: &mut Vec<Handle>) {
    for handle in handles.drain(..).rev() {
        unsafe {
            ResumeThread(handle);
            CloseHandle(handle);
        }
    }
}
