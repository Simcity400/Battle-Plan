//! relay; the relay performs a pointer-free absolute jump to the Rust ABI

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
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::convoy_route_source::{
    read_exact_a729_client_route, ExactA729RouteRead, RouteSourceError,
};
use crate::convoy_terminal_attribution::{
    a729_call_window_is_exact, activation_blockers, capture_candidate, prepare_a729_hook,
    A729PreCollapseRegisters, ActivationBlocker, ConvoyTerminalAttributionCarrier, ExactMemory,
    A729_CALL_BYTES, A729_EXE_SHA256, A729_HOOK_RVA, MAX_COUNTRY_TAG_DB_ID,
};
use crate::convoy_terminal_runtime::{
    clear_convoy_terminal_resolver_snapshot, convoy_terminal_resolver_ever_partial,
    current_convoy_terminal_publication_receipt, try_current_convoy_terminal_resolver_snapshot,
    ResolverPublicationReceipt,
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
    fn GetModuleHandleW(module_name: *const u16) -> Handle;
    fn GetModuleFileNameW(module: Handle, filename: *mut u16, size: Dword) -> Dword;
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
    pub dropped_pre_hour_unreadable: u64,
    pub dropped_receipt_hour_mismatch: u64,
    pub dropped_post_hour_unreadable: u64,
    pub dropped_post_hour_changed: u64,
    pub dropped_resolver_snapshot_absent: u64,
    pub dropped_resolver_unpublished: u64,
    pub receipt_hour_lag_max: u64,
    pub receipt_hour_lead_max: u64,
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
    pub resolver_ever_partial: bool,
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
    FreezeRefused(inline_detour::FreezeRefusal),
    InstructionPointerInPatchSite,
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

pub fn publish_h_convoy_evidence_game_state(game_state: usize, game_hour_raw: i32) {
    if convoy_terminal_evidence_activation_enabled() && game_state != 0 {
        EVIDENCE_GAME_STATE.store(game_state, Ordering::Release);
        EVIDENCE_GAME_HOUR.store(game_hour_raw as u32 as u64, Ordering::Release);
    }
}

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
    DROP_PRE_HOUR_UNREADABLE,
    DROP_RECEIPT_HOUR_MISMATCH,
    DROP_POST_HOUR_UNREADABLE,
    DROP_POST_HOUR_CHANGED,
    DROP_RESOLVER_SNAPSHOT_ABSENT,
    DROP_RESOLVER_UNPUBLISHED,
    RECEIPT_HOUR_LAG_MAX,
    RECEIPT_HOUR_LEAD_MAX,
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
        resolver_ever_partial: convoy_terminal_resolver_ever_partial(),
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

/// restoration, quiescence and release-receipt observation — is the same seal
pub fn abort_partial_install_and_uninstall(
    max_events: usize,
) -> Result<InstallAbortDiscard, String> {
    let reservation = seal_and_drain_convoy_transport_attack_payload(max_events)
        .map_err(|error| format!("D90 install abort seal failed: {error:?}"))?;
    let Some(reservation) = reservation else {
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
        resolver_unbound_convoy_clients: drain.resolver.unbound_convoy_clients,
        resolver_ever_partial: drain.resolver_ever_partial,
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
            dropped_pre_hour_unreadable: telemetry.dropped_pre_hour_unreadable,
            dropped_receipt_hour_mismatch: telemetry.dropped_receipt_hour_mismatch,
            dropped_post_hour_unreadable: telemetry.dropped_post_hour_unreadable,
            dropped_post_hour_changed: telemetry.dropped_post_hour_changed,
            dropped_resolver_snapshot_absent: telemetry.dropped_resolver_snapshot_absent,
            dropped_resolver_unpublished: telemetry.dropped_resolver_unpublished,
            receipt_hour_lag_max: telemetry.receipt_hour_lag_max,
            receipt_hour_lead_max: telemetry.receipt_hour_lead_max,
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
        dropped_pre_hour_unreadable: load(&DROP_PRE_HOUR_UNREADABLE),
        dropped_receipt_hour_mismatch: load(&DROP_RECEIPT_HOUR_MISMATCH),
        dropped_post_hour_unreadable: load(&DROP_POST_HOUR_UNREADABLE),
        dropped_post_hour_changed: load(&DROP_POST_HOUR_CHANGED),
        dropped_resolver_snapshot_absent: load(&DROP_RESOLVER_SNAPSHOT_ABSENT),
        dropped_resolver_unpublished: load(&DROP_RESOLVER_UNPUBLISHED),
        receipt_hour_lag_max: load(&RECEIPT_HOUR_LAG_MAX),
        receipt_hour_lead_max: load(&RECEIPT_HOUR_LEAD_MAX),
        dropped_d70_pre: load(&DROP_D70_PRE),
        dropped_d70_post: load(&DROP_D70_POST),
        dropped_d70_mutation: load(&DROP_D70_MUTATION),
        dropped_queue_busy: load(&DROP_QUEUE_BUSY),
        dropped_queue_full: load(&DROP_QUEUE_FULL),
    }
}

struct HeldObserverState<'a> {
    reservation: Option<MutexGuard<'a, DrainReservationState>>,
    queue: Option<MutexGuard<'a, VecDeque<ConvoyTerminalValidatedEvent>>>,
    positive: Option<MutexGuard<'a, Option<HConvoyPositiveRawObservation>>>,
}

fn lock_observer_state() -> HeldObserverState<'static> {
    HeldObserverState {
        reservation: DRAIN_RESERVATION
            .get()
            .map(|slot| slot.lock().unwrap_or_else(|poison| poison.into_inner())),
        queue: EVENT_QUEUE
            .get()
            .map(|slot| slot.lock().unwrap_or_else(|poison| poison.into_inner())),
        positive: POSITIVE_RAW_OBSERVATION
            .get()
            .map(|slot| slot.lock().unwrap_or_else(|poison| poison.into_inner())),
    }
}

fn reset_observer_state_with(held: &mut HeldObserverState<'_>) {
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
        &DROP_PRE_HOUR_UNREADABLE,
        &DROP_RECEIPT_HOUR_MISMATCH,
        &DROP_POST_HOUR_UNREADABLE,
        &DROP_POST_HOUR_CHANGED,
        &DROP_RESOLVER_SNAPSHOT_ABSENT,
        &DROP_RESOLVER_UNPUBLISHED,
        &RECEIPT_HOUR_LAG_MAX,
        &RECEIPT_HOUR_LEAD_MAX,
    ] {
        counter.store(0, Ordering::Release);
    }
    PRODUCER_SEQUENCE.store(1, Ordering::Release);
    QUEUE_MUTATION_GENERATION.store(1, Ordering::Release);
    SEALED_PUBLICATION_PENDING.store(false, Ordering::Release);
    if let Some(reservation) = held.reservation.as_mut() {
        reservation.sealed_snapshot = None;
    }
    if let Some(queue) = held.queue.as_mut() {
        queue.clear();
    }
    if let Some(positive) = held.positive.as_mut() {
        **positive = None;
    }
    EVIDENCE_HEARTBEAT_SEQUENCE.store(1, Ordering::Release);
}

fn reset_observer_state_for_install() {
    let mut held = lock_observer_state();
    reset_observer_state_with(&mut held);
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

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolverRefusal {
    Unpublished,
    HourMismatch,
}

#[cfg(windows)]
fn classify_resolver_receipt(
    publication_sequence: u64,
    receipt_game_hour_raw: i32,
    engine_hour: i32,
) -> Option<ResolverRefusal> {
    if publication_sequence == 0 {
        return Some(ResolverRefusal::Unpublished);
    }
    if receipt_game_hour_raw != engine_hour {
        return Some(ResolverRefusal::HourMismatch);
    }
    None
}

fn record_receipt_hour_distance(receipt_game_hour_raw: i32, engine_hour: i32) {
    let (direction, magnitude) = receipt_hour_distance(receipt_game_hour_raw, engine_hour);
    let counter = match direction {
        ReceiptHourDirection::Lag => &RECEIPT_HOUR_LAG_MAX,
        ReceiptHourDirection::Lead => &RECEIPT_HOUR_LEAD_MAX,
    };
    counter.fetch_max(magnitude, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReceiptHourDirection {
    Lag,
    Lead,
}

fn receipt_hour_distance(
    receipt_game_hour_raw: i32,
    engine_hour: i32,
) -> (ReceiptHourDirection, u64) {
    let distance = i64::from(engine_hour) - i64::from(receipt_game_hour_raw);
    if distance >= 0 {
        (ReceiptHourDirection::Lag, distance as u64)
    } else {
        (ReceiptHourDirection::Lead, distance.unsigned_abs())
    }
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
        DROP_PRE_HOUR_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let Some(resolver) = try_current_convoy_terminal_resolver_snapshot() else {
        DROP_RESOLVER.fetch_add(1, Ordering::Relaxed);
        DROP_RESOLVER_SNAPSHOT_ABSENT.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let receipt = resolver.receipt();
    match classify_resolver_receipt(receipt.publication_sequence, receipt.game_hour_raw, hour) {
        None => {}
        Some(ResolverRefusal::Unpublished) => {
            DROP_RESOLVER.fetch_add(1, Ordering::Relaxed);
            DROP_RESOLVER_UNPUBLISHED.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        Some(ResolverRefusal::HourMismatch) => {
            DROP_HOUR_OR_EPOCH.fetch_add(1, Ordering::Relaxed);
            DROP_RECEIPT_HOUR_MISMATCH.fetch_add(1, Ordering::Relaxed);
            record_receipt_hour_distance(receipt.game_hour_raw, hour);
            return None;
        }
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
        DROP_POST_HOUR_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if hour != pre.carrier.game_hour_raw {
        DROP_HOUR_OR_EPOCH.fetch_add(1, Ordering::Relaxed);
        DROP_POST_HOUR_CHANGED.fetch_add(1, Ordering::Relaxed);
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

pub fn install_convoy_terminal_hook(
    executable_sha256: &str,
) -> Result<(), ConvoyTerminalInstallError> {
    let blockers = activation_blockers();
    if !blockers.is_empty() {
        return Err(ConvoyTerminalInstallError::PromotionBlocked(blockers));
    }
    install_convoy_terminal_hook_inner(executable_sha256, HookInstallMode::Production)
}

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
    if mode.is_production() {
        production_hook_receipt::prime_main_module_authority()
            .map_err(ConvoyTerminalInstallError::HookReceipt)?;
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
    let mut held_observer_state = lock_observer_state();
    let suspended = suspend_all_other_threads()?;
    let patch_ranges = [
        (
            prepared.hook_address,
            prepared.hook_address + A729_CALL_BYTES.len(),
        ),
        (relay as usize, relay as usize + RELAY_LEN),
    ];
    if let Err(error) = patch_sites_are_clear(&suspended, &patch_ranges) {
        drop(suspended);
        unsafe { VirtualFree(relay.cast(), 0, MEM_RELEASE) };
        return Err(error);
    }
    if a729_call_window_is_exact(&memory, module_base, executable_sha256).is_err() {
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
    reset_observer_state_with(&mut held_observer_state);
    // AND RELEASED IMMEDIATELY, still inside the window, which is the whole
    // ACQUIRING one is: it hands ownership on and waits for nothing. Nothing
    drop(held_observer_state);
    MODULE_BASE.store(module_base, Ordering::Release);
    ORIGINAL_TARGET.store(prepared.original_target_address, Ordering::Release);
    TEARDOWN.store(false, Ordering::Release);
    if let Err(error) = unsafe { write_code(prepared.hook_address, &patched_call) } {
        let outcome = rollback_failed_install(
            &mut state,
            &memory,
            mode,
            teardown_owner_token,
            prepared.hook_address,
            relay as usize,
            original_call,
            patched_call,
        );
        drop(suspended);
        if outcome == RollbackOutcome::Restored {
            reset_observer_state_for_install();
        }
        return Err(outcome.into_error(error.into()));
    }
    let mut exact_patch = [0u8; A729_CALL_BYTES.len()];
    if !memory.read_exact(prepared.hook_address, &mut exact_patch) || exact_patch != patched_call {
        let outcome = rollback_failed_install(
            &mut state,
            &memory,
            mode,
            teardown_owner_token,
            prepared.hook_address,
            relay as usize,
            original_call,
            patched_call,
        );
        drop(suspended);
        if outcome == RollbackOutcome::Restored {
            reset_observer_state_for_install();
        }
        return Err(outcome.into_error(ConvoyTerminalInstallError::PatchedBytesDrift));
    }
    if !relay_targets_convoy_terminal_hook(&memory, relay as usize) {
        let outcome = rollback_failed_install(
            &mut state,
            &memory,
            mode,
            teardown_owner_token,
            prepared.hook_address,
            relay as usize,
            original_call,
            patched_call,
        );
        drop(suspended);
        if outcome == RollbackOutcome::Restored {
            reset_observer_state_for_install();
        }
        return Err(outcome.into_error(ConvoyTerminalInstallError::RelayDrift));
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
                let outcome = rollback_failed_install(
                    &mut state,
                    &memory,
                    mode,
                    teardown_owner_token,
                    prepared.hook_address,
                    relay as usize,
                    original_call,
                    patched_call,
                );
                drop(suspended);
                if outcome == RollbackOutcome::Restored {
                    reset_observer_state_for_install();
                }
                return Err(outcome.into_error(ConvoyTerminalInstallError::HookReceipt(error)));
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
) -> RollbackOutcome {
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
                        Err(error) => RollbackRefusal::WriteFailed(error),
                        Ok(()) => RollbackRefusal::BytesNotRestored,
                    };
                    (false, observed_after, Some(error))
                }
            }
        }
        None => (false, None, Some(RollbackRefusal::BytesUnreadable)),
    };

    if patch_restored && rollback_error.is_none() {
        if unsafe { VirtualFree(relay as *mut c_void, 0, MEM_RELEASE) } != 0 {
            ORIGINAL_TARGET.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            EVIDENCE_GAME_STATE.store(0, Ordering::Release);
            EVIDENCE_GAME_HOUR.store(0, Ordering::Release);
            TEARDOWN.store(false, Ordering::Release);
            INSTALLED.store(false, Ordering::Release);
            *state = None;
            return RollbackOutcome::Restored;
        }
    }

    let rollback_error = rollback_error.unwrap_or(RollbackRefusal::RelayReleaseFailed);
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
    RollbackOutcome::Retained(rollback_error)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RollbackOutcome {
    /// The original bytes are back and the relay is released.
    Restored,
    Retained(RollbackRefusal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RollbackRefusal {
    WriteFailed(WriteRefusal),
    BytesNotRestored,
    BytesUnreadable,
    RelayReleaseFailed,
}

impl RollbackOutcome {
    fn into_error(self, install_error: ConvoyTerminalInstallError) -> ConvoyTerminalInstallError {
        let refusal = match self {
            Self::Restored => return install_error,
            Self::Retained(refusal) => refusal,
        };
        ConvoyTerminalInstallError::InstallRollbackFailed {
            install_error: format!("{install_error:?}"),
            rollback_error: match refusal {
                RollbackRefusal::WriteFailed(error) => format!("{error:?}"),
                RollbackRefusal::BytesNotRestored => {
                    "exact original bytes were not observed after rollback".to_string()
                }
                RollbackRefusal::BytesUnreadable => {
                    "failed to observe D90 bytes before guarded rollback".to_string()
                }
                RollbackRefusal::RelayReleaseFailed => "D90 relay release failed".to_string(),
            },
        }
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
        let suspended = suspend_all_other_threads()?;
        let patch_ranges = [
            (installed.callsite, installed.callsite + 5),
            (installed.relay, installed.relay + RELAY_LEN),
        ];
        if let Err(error) = patch_sites_are_clear(&suspended, &patch_ranges) {
            drop(suspended);
            return Err(error);
        }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteRefusal {
    Protect,
    Flush,
}

impl From<WriteRefusal> for ConvoyTerminalInstallError {
    fn from(refusal: WriteRefusal) -> Self {
        match refusal {
            WriteRefusal::Protect => Self::PatchProtect,
            WriteRefusal::Flush => Self::PatchFlush,
        }
    }
}

unsafe fn write_code(address: usize, bytes: &[u8]) -> Result<(), WriteRefusal> {
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
        return Err(WriteRefusal::Protect);
    }
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len()) };
    let mut ignored = 0;
    if unsafe { VirtualProtect(address as *mut c_void, bytes.len(), old, &mut ignored) } == 0 {
        return Err(WriteRefusal::Protect);
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), address as *const c_void, bytes.len()) }
        == 0
    {
        return Err(WriteRefusal::Flush);
    }
    Ok(())
}

fn suspend_all_other_threads() -> Result<inline_detour::ProcessFreeze, ConvoyTerminalInstallError> {
    inline_detour::freeze_process().map_err(ConvoyTerminalInstallError::FreezeRefused)
}

fn patch_sites_are_clear(
    suspended: &inline_detour::ProcessFreeze,
    ranges: &[(usize, usize)],
) -> Result<(), ConvoyTerminalInstallError> {
    if suspended
        .any_instruction_pointer_in(ranges)
        .map_err(ConvoyTerminalInstallError::FreezeRefused)?
    {
        return Err(ConvoyTerminalInstallError::InstructionPointerInPatchSite);
    }
    Ok(())
}

#[cfg(test)]
mod freeze_call_site_tests {
    pub(super) fn production_code() -> String {
        let source = include_str!("convoy_terminal_hook_windows.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod ")
            .expect("this module has a test module")
            .0
            .to_string()
    }

    #[test]
    fn every_d90_code_mutation_happens_under_the_shared_freeze() {
        let production = production_code();
        for (signature, write) in [
            (
                "fn install_convoy_terminal_hook_inner(",
                "write_code(prepared.hook_address",
            ),
            (
                "fn restore_and_quiesce_convoy_terminal_hook(",
                "write_code(installed.callsite",
            ),
        ] {
            let body = production
                .split_once(signature)
                .unwrap_or_else(|| panic!("{signature} is declared in production"))
                .1;
            let freeze = body
                .find("suspend_all_other_threads()")
                .unwrap_or_else(|| panic!("{signature} must freeze the process"));
            let clear = body
                .find("patch_sites_are_clear(&suspended,")
                .unwrap_or_else(|| {
                    panic!("{signature} must prove the patch sites are clear of frozen threads")
                });
            let mutation = body
                .find(write)
                .unwrap_or_else(|| panic!("{signature} must still contain {write}"));
            assert!(
                freeze < clear && clear < mutation,
                "{signature}: the order must be freeze, then check, then write \
                 (freeze at {freeze}, check at {clear}, write at {mutation})"
            );
        }
    }

    #[test]
    fn the_receipt_authority_is_bought_before_the_freeze_not_inside_it() {
        let production = production_code();
        let body = production
            .split_once("fn install_convoy_terminal_hook_inner(")
            .expect("the installer is declared in production")
            .1;
        let prime = body
            .find("prime_main_module_authority()")
            .expect("the installer must prime the receipt authority");
        let freeze = body
            .find("suspend_all_other_threads()")
            .expect("the installer must freeze the process");
        let observe = body
            .find("observe_enabled_rel32_callsite(")
            .expect("the installer must still mint its installed receipt");
        assert!(
            prime < freeze,
            "the receipt authority must be built with the game RUNNING \
             (prime at {prime}, freeze at {freeze})"
        );
        assert!(
            freeze < observe,
            "this pin assumes the receipt observation is still inside the \
             window; if it has been hoisted out, the priming is no longer the \
             thing standing between a player's game and a whole-file hash \
             taken with every thread suspended -- re-derive this test"
        );
    }

    #[test]
    fn d90_declares_no_thread_suspender_of_its_own() {
        let production = production_code();
        for (prefix, call) in [
            ("Suspend", "Thread("),
            ("Open", "Thread("),
            ("Thread32", "First("),
            ("Thread32", "Next("),
        ] {
            let banned = format!("{prefix}{call}");
            assert!(
                !production.contains(&banned),
                "D90 must not call {banned} itself; its private suspender read any \
                 stop of the thread walk as the end of the list, and enumerated once"
            );
        }
        assert!(
            production.contains("inline_detour::freeze_process()"),
            "D90 rewrites live bytes and must take the shared freeze"
        );
    }
}

#[cfg(all(test, windows))]
mod resolver_refusal_tests {
    use super::{classify_resolver_receipt, ResolverRefusal};

    #[test]
    fn an_unpublished_receipt_is_not_an_hour_disagreement() {
        assert_eq!(
            classify_resolver_receipt(0, 0, 4_211),
            Some(ResolverRefusal::Unpublished)
        );
        assert_eq!(
            classify_resolver_receipt(0, 4_211, 4_211),
            Some(ResolverRefusal::Unpublished)
        );
    }

    #[test]
    fn a_published_receipt_from_another_hour_is_an_hour_disagreement() {
        assert_eq!(
            classify_resolver_receipt(7, 4_210, 4_211),
            Some(ResolverRefusal::HourMismatch)
        );
    }

    #[test]
    fn a_published_receipt_for_this_hour_admits_the_candidate() {
        assert_eq!(classify_resolver_receipt(7, 4_211, 4_211), None);
    }

    #[test]
    fn each_refusal_reaches_its_own_counter() {
        let production = super::freeze_call_site_tests::production_code();
        let routing = production
            .split_once("match classify_resolver_receipt(")
            .expect("prepare_observation classifies the receipt")
            .1
            .split_once("let sequence = PRODUCER_SEQUENCE")
            .expect("the routing precedes the sequence mint")
            .0;
        for (arm, counter) in [
            ("Some(ResolverRefusal::Unpublished) => {", "DROP_RESOLVER"),
            (
                "Some(ResolverRefusal::HourMismatch) => {",
                "DROP_HOUR_OR_EPOCH",
            ),
        ] {
            let body = routing
                .split_once(arm)
                .unwrap_or_else(|| panic!("{arm} must be routed explicitly"))
                .1
                .split_once("}")
                .expect("arm body")
                .0;
            assert!(
                body.contains(&format!("{counter}.fetch_add(1, Ordering::Relaxed);")),
                "{arm} must increment {counter}; collapsing the two arms onto one \
                 counter is the defect this split exists to remove"
            );
            assert!(
                body.contains("return None;"),
                "{arm} must REFUSE, not merely count: without the return the \
                 candidate reaches the sequence mint and is admitted"
            );
        }
        assert!(
            !routing.contains("publication_sequence == 0 ||"),
            "the collapsed guard must not come back alongside the classifier"
        );
    }

    #[test]
    fn every_conflated_drop_site_also_names_itself() {
        let production = super::freeze_call_site_tests::production_code();
        for (aggregate, sites) in [
            (
                "DROP_HOUR_OR_EPOCH",
                &[
                    "DROP_PRE_HOUR_UNREADABLE",
                    "DROP_RECEIPT_HOUR_MISMATCH",
                    "DROP_POST_HOUR_UNREADABLE",
                    "DROP_POST_HOUR_CHANGED",
                ][..],
            ),
            (
                "DROP_RESOLVER",
                &["DROP_RESOLVER_SNAPSHOT_ABSENT", "DROP_RESOLVER_UNPUBLISHED"][..],
            ),
        ] {
            let increment = format!("{aggregate}.fetch_add(1, Ordering::Relaxed);");
            let aggregate_increments = production.matches(&increment).count();
            let site_increments: usize = sites
                .iter()
                .map(|site| {
                    let site_increment = format!("{site}.fetch_add(1, Ordering::Relaxed);");
                    let count = production.matches(&site_increment).count();
                    assert_eq!(
                        count, 1,
                        "{site} must be incremented at exactly one site, or it \
                         names a place rather than a cause"
                    );
                    count
                })
                .sum();
            assert_eq!(
                aggregate_increments,
                sites.len(),
                "{aggregate} is incremented at {aggregate_increments} sites but \
                 only {} are enumerated here; an unenumerated site is a cause \
                 the wire cannot name",
                sites.len()
            );
            assert_eq!(
                aggregate_increments, site_increments,
                "every site that increments {aggregate} must also increment its \
                 own counter; {aggregate_increments} aggregate increments but \
                 only {site_increments} site increments means one site is still \
                 anonymous"
            );
        }
    }
}

#[cfg(all(test, windows))]
mod receipt_hour_distance_tests {
    use super::{receipt_hour_distance, ReceiptHourDirection};

    #[test]
    fn a_refused_receipt_records_which_side_of_the_clock_it_sat_on() {
        assert_eq!(
            receipt_hour_distance(4_210, 4_211),
            (ReceiptHourDirection::Lag, 1),
            "the receipt is one hour BEHIND the engine: the publisher lagged"
        );
        assert_eq!(
            receipt_hour_distance(4_211, 4_210),
            (ReceiptHourDirection::Lead, 1),
            "the receipt is one hour AHEAD of the engine: the clock inverted"
        );
        assert_eq!(
            receipt_hour_distance(4_211, 4_211),
            (ReceiptHourDirection::Lag, 0),
            "an equal receipt is zero LAG, not lead: a receipt that agrees has \
             not inverted the clock"
        );
        // wrap to 1 in release, reporting a ten-billion-hour disagreement as a
        assert_eq!(
            receipt_hour_distance(i32::MAX, i32::MIN),
            (ReceiptHourDirection::Lead, 4_294_967_295),
            "the widest possible disagreement is reported at its true size"
        );
        assert_eq!(
            receipt_hour_distance(i32::MIN, i32::MAX),
            (ReceiptHourDirection::Lag, 4_294_967_295)
        );
    }
}

#[cfg(test)]
mod frozen_window_allocation_tests {
    use super::*;
    use crate::convoy_terminal_attribution::PrepareError;

    #[test]
    fn the_allocation_probe_is_this_test_binarys_allocator() {
        assert!(
            freeze_window_audit::the_probe_is_installed(),
            "the #[global_allocator] in lib.rs is not in force, so every \
             allocation count in this module is vacuously zero"
        );
    }

    #[test]
    fn the_frozen_byte_gate_allocates_nothing_when_it_refuses() {
        let memory = ProcessMemory;
        let mut refusals = [PrepareError::NullImageBase; 3];
        let ((), allocations) = freeze_window_audit::observed(|| {
            refusals[0] = a729_call_window_is_exact(&memory, 0, A729_EXE_SHA256).unwrap_err();
            refusals[1] =
                a729_call_window_is_exact(&memory, 0x1000, "not the a729 hash").unwrap_err();
            refusals[2] =
                a729_call_window_is_exact(&memory, usize::MAX, A729_EXE_SHA256).unwrap_err();
        });
        assert_eq!(refusals[0], PrepareError::NullImageBase);
        assert_eq!(refusals[1], PrepareError::BinaryHashMismatch);
        assert_eq!(refusals[2], PrepareError::AddressOverflow);
        assert_eq!(allocations, 0, "the frozen byte gate reached the allocator");
    }

    #[test]
    fn the_frozen_rollback_allocates_nothing() {
        let original_call = A729_CALL_BYTES;
        let mut site = original_call;
        let mut state: Option<InstalledState> = None;
        let memory = ProcessMemory;
        let (outcome, allocations) = freeze_window_audit::observed(|| {
            rollback_failed_install(
                &mut state,
                &memory,
                HookInstallMode::Production,
                None,
                site.as_mut_ptr() as usize,
                0,
                original_call,
                [0x90; 5],
            )
        });
        assert_eq!(
            outcome,
            RollbackOutcome::Retained(RollbackRefusal::RelayReleaseFailed),
            "a relay that cannot be released is retained custody"
        );
        assert!(state.is_some(), "retained custody must be published");
        assert_eq!(allocations, 0, "the frozen rollback reached the allocator");

        let (rendered, allocations) = freeze_window_audit::observed(|| {
            outcome.into_error(ConvoyTerminalInstallError::PatchedBytesDrift)
        });
        assert!(
            matches!(
                rendered,
                ConvoyTerminalInstallError::InstallRollbackFailed { .. }
            ),
            "{rendered:?}"
        );
        assert!(
            allocations > 0,
            "rendering the sentence must be what allocates -- if this is zero \
             the test is no longer proving that the rendering was moved out of \
             the stopped window"
        );
    }
}
