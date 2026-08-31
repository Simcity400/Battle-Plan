use crate::a729_inline_hook::{self, CallbackBinding, InstalledHookSet, SiteKind};
use crate::{
    encode_payload, A729EventLedger, CaptureStartFiredBaseline, ExactFiredEventSource, FireInput,
    FiredEventFrame, FiredEventRow, FiredEventSnapshot, HookTelemetry, MaterialisationInput,
    ResolutionInput, ResolutionReason,
    CERTIFIED_ROUTE_EXECUTABLE_SHA256, CERTIFIED_ROUTE_MANIFEST_SHA256, CERTIFIED_ROUTE_RECEIPT,
    PRODUCER_MANIFEST_SHA256,
};
use core::cell::RefCell;
use core::ffi::c_void;
use core::mem;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

type FireFn = unsafe extern "system" fn(usize, usize, *const u32, *const u32, usize, u8);
type MaterialiseFn = unsafe extern "system" fn(usize);
type SelectedDescriptionFn = unsafe extern "system" fn(usize, usize, usize, u8) -> usize;
type PictureSelectorFn = unsafe extern "system" fn(usize, usize, usize);
type SpriteLookupFn = unsafe extern "system" fn(usize, usize) -> usize;
type PlayerChoiceFn = unsafe extern "system" fn(usize, usize);
type TimeoutFn = unsafe extern "system" fn(usize) -> u8;
type OptionCommandFn =
    unsafe extern "system" fn(usize, u32, usize, *const u32, usize, u32) -> usize;
type AdmissionFn = unsafe extern "system" fn(usize, usize, u8) -> u8;

static ORIGINAL_FIRE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MATERIALISE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SELECTED_DESCRIPTION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PICTURE_SELECTOR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SPRITE_LOOKUP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PLAYER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TIMEOUT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_COMMAND: AtomicUsize = AtomicUsize::new(0);
static A19_DELAYED_EVENT_DISPATCH_OBSERVER: AtomicUsize = AtomicUsize::new(0);
static NEXT_TERMINAL_CUSTODY_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static RUNTIME: OnceLock<Mutex<Option<Arc<Runtime>>>> = OnceLock::new();
static SOURCE: A729RuntimeSource = A729RuntimeSource;

#[derive(Clone, Copy, Debug)]
pub struct RawFire {
    pub dispatcher: usize,
    pub event: usize,
    pub actor_tag: *const u32,
    pub recipient_tag: *const u32,
    pub event_scope: usize,
    pub immediate: u8,
}

unsafe impl Send for RawFire {}
unsafe impl Sync for RawFire {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct A19DelayedEventDispatchObservation {
    pub authenticated: u32,
    pub event: usize,
    pub actor_tag_id: u32,
    pub recipient_tag_id: u32,
    pub event_scope: usize,
}

pub type A19DelayedEventDispatchObserver =
    unsafe extern "system" fn(A19DelayedEventDispatchObservation);

pub fn register_a19_delayed_event_dispatch_observer(
    observer: A19DelayedEventDispatchObserver,
) -> Result<(), String> {
    let address = observer as usize;
    if address == 0 {
        return Err("A19 delayed-event dispatch observer is null".into());
    }
    let slot = runtime_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.as_ref().is_some_and(|runtime| {
        runtime.terminal_custody_state.load(Ordering::Acquire) != TERMINAL_CUSTODY_ACTIVE
            || !runtime.accepting.load(Ordering::Acquire)
    }) {
        return Err("A19 delayed-event dispatch observer cannot register after D75 sealing".into());
    }
    let result = A19_DELAYED_EVENT_DISPATCH_OBSERVER
        .compare_exchange(0, address, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| "A19 delayed-event dispatch observer is already registered".into());
    drop(slot);
    result
}

pub fn unregister_a19_delayed_event_dispatch_observer() -> Result<(), String> {
    let previous = A19_DELAYED_EVENT_DISPATCH_OBSERVER.swap(0, Ordering::AcqRel);
    if previous == 0 {
        return Err("A19 delayed-event dispatch observer is not registered".into());
    }
    Ok(())
}

fn notify_a19_delayed_event_dispatch(raw: RawFire) {
    let address = A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire);
    if address == 0 || raw.event == 0 || raw.event_scope == 0 {
        return;
    }
    if raw.actor_tag.is_null() || raw.recipient_tag.is_null() {
        return;
    }
    let observation = A19DelayedEventDispatchObservation {
        authenticated: 1,
        event: raw.event,
        actor_tag_id: unsafe { ptr::read(raw.actor_tag) },
        recipient_tag_id: unsafe { ptr::read(raw.recipient_tag) },
        event_scope: raw.event_scope,
    };
    let observer: A19DelayedEventDispatchObserver = unsafe { mem::transmute(address) };
    unsafe { observer(observation) };
}

fn notify_a19_delayed_event_dispatch_unavailable() {
    let address = A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire);
    if address == 0 {
        return;
    }
    let observer: A19DelayedEventDispatchObserver = unsafe { mem::transmute(address) };
    unsafe {
        observer(A19DelayedEventDispatchObservation {
            authenticated: 0,
            event: 0,
            actor_tag_id: 0,
            recipient_tag_id: 0,
            event_scope: 0,
        })
    };
}

#[derive(Clone, Debug)]
pub struct RawMaterialisation {
    pub event_window: usize,
    pub event: usize,
    pub admitted_option_pointers: Vec<usize>,
    pub selected_desc_key: Option<String>,
    pub shown_picture_gfx: Option<String>,
    pub runtime_text_owners: Vec<ObservedRuntimeTextOwner>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedRuntimeTextOwner {
    pub textbox_owner: usize,
    pub setter_caller_rva: u32,
    pub context_id: String,
    pub view_id: String,
    pub element_id: String,
}

#[derive(Clone, Copy, Debug)]
pub struct RawResolution {
    pub path: ResolutionPath,
    pub owner: usize,
    pub event_sequence_hint: Option<u64>,
    pub instance_id: u32,
    pub event: usize,
    pub recipient_tag_id: u32,
    pub event_scope: usize,
    pub option_ordinal: u32,
}

unsafe impl Send for RawResolution {}
unsafe impl Sync for RawResolution {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionPath {
    Ai,
    Player,
    Timeout,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoverageAttestation {
    pub all_event_class_routes_certified: bool,
    pub player_ai_timeout_no_choice_routes_certified: bool,
}

pub(crate) fn certified_route_attestation() -> Result<CoverageAttestation, String> {
    if PRODUCER_MANIFEST_SHA256 != CERTIFIED_ROUTE_MANIFEST_SHA256 {
        return Err(format!(
            "XUI21 static route authority mismatch: producer manifest is not the exact-a729 certified manifest from {CERTIFIED_ROUTE_RECEIPT} (executable sha256 {CERTIFIED_ROUTE_EXECUTABLE_SHA256})"
        ));
    }
    Ok(CoverageAttestation {
        all_event_class_routes_certified: true,
        player_ai_timeout_no_choice_routes_certified: true,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageMatrixObservation {
    pub cell_index: u8,
    pub popup_class: String,
    pub resolution_path: String,
    pub event_id: String,
    pub recipient_tag: String,
    pub fire_date: String,
    pub fire_hour: u64,
    pub fire_tick: u64,
    pub event_sequence: u64,
    pub instance_id: u32,
    pub chosen_option_ordinal: Option<u16>,
    pub resolved_tick: u64,
    pub materialised_before_resolution: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageLifecycleWitnesses {
    pub repeated_event_id: Option<RepeatedEventIdWitness>,
    pub same_hour_multiple_firings: Option<SameHourFiringsWitness>,
    pub capture_start_pending: Option<CaptureStartPendingWitness>,
    pub non_materialised_resolution: Option<NonMaterialisedResolutionWitness>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepeatedEventIdWitness {
    pub event_id: String,
    pub first_sequence: u64,
    pub second_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SameHourFiringsWitness {
    pub fire_hour: u64,
    pub first_sequence: u64,
    pub second_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureStartPendingWitness {
    pub event_id: String,
    pub event_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NonMaterialisedResolutionWitness {
    pub event_id: String,
    pub event_sequence: u64,
    pub resolution_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageReceipt {
    pub schema: String,
    pub session_id: String,
    pub program: String,
    pub image_base: String,
    pub executable_sha256: String,
    pub producer_manifest_sha256: String,
    pub matrix_bits: u32,
    pub observations: Vec<CoverageMatrixObservation>,
    pub lifecycle: CoverageLifecycleWitnesses,
    pub hook_telemetry: Option<CoverageTelemetryWitness>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageTelemetryWitness {
    pub fire_hook_validated: bool,
    pub option_hook_validated: bool,
    pub materialisation_hook_validated: bool,
    pub timeout_hook_validated: bool,
    pub all_event_classes_covered: bool,
    pub player_ai_timeout_no_choice_covered: bool,
    pub capture_start_baseline_complete: bool,
    pub queue_capacity: u32,
    pub queue_high_water_mark: u32,
    pub dropped_fire_rows: u64,
    pub dropped_resolution_rows: u64,
    pub correlation_failures: u64,
    pub invalid_definition_joins: u64,
    pub fired_ledger_shadow_mismatches: u64,
    pub capture_start_fired_ids_unresolved: u64,
}

impl CoverageTelemetryWitness {
    fn from_hook_telemetry(value: &HookTelemetry) -> Self {
        Self {
            fire_hook_validated: value.fire_hook_validated,
            option_hook_validated: value.option_hook_validated,
            materialisation_hook_validated: value.materialisation_hook_validated,
            timeout_hook_validated: value.timeout_hook_validated,
            all_event_classes_covered: value.all_event_classes_covered,
            player_ai_timeout_no_choice_covered: value.player_ai_timeout_no_choice_covered,
            capture_start_baseline_complete: value.capture_start_baseline_complete,
            queue_capacity: value.queue_capacity,
            queue_high_water_mark: value.queue_high_water_mark,
            dropped_fire_rows: value.dropped_fire_rows,
            dropped_resolution_rows: value.dropped_resolution_rows,
            correlation_failures: value.correlation_failures,
            invalid_definition_joins: value.invalid_definition_joins,
            fired_ledger_shadow_mismatches: value.fired_ledger_shadow_mismatches,
            capture_start_fired_ids_unresolved: value.capture_start_fired_ids_unresolved,
        }
    }
}

impl CoverageReceipt {
    pub fn complete(&self) -> bool {
        self.matrix_bits == (1 << 20) - 1
            && self.observations.len() == 20
            && self.lifecycle.repeated_event_id.is_some()
            && self.lifecycle.same_hour_multiple_firings.is_some()
            && self.lifecycle.capture_start_pending.is_some()
            && self.lifecycle.non_materialised_resolution.is_some()
            && self.hook_telemetry.as_ref().is_some_and(|telemetry| {
                telemetry.fire_hook_validated
                    && telemetry.option_hook_validated
                    && telemetry.materialisation_hook_validated
                    && telemetry.timeout_hook_validated
                    && telemetry.all_event_classes_covered
                    && telemetry.player_ai_timeout_no_choice_covered
                    && telemetry.capture_start_baseline_complete
                    && telemetry.queue_capacity > 0
                    && telemetry.queue_high_water_mark <= telemetry.queue_capacity
                    && telemetry.dropped_fire_rows == 0
                    && telemetry.dropped_resolution_rows == 0
                    && telemetry.correlation_failures == 0
                    && telemetry.invalid_definition_joins == 0
                    && telemetry.fired_ledger_shadow_mismatches == 0
                    && telemetry.capture_start_fired_ids_unresolved == 0
            })
    }
}

pub struct PendingBaseline {
    pub rows: Vec<FireInput>,
    pub complete: bool,
}

pub trait A729EventBridge: Send + Sync + 'static {
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn pending_baseline(&self) -> Result<PendingBaseline, String>;
    fn fire_input(&self, raw: RawFire) -> Result<Option<FireInput>, String>;
    fn fire_committed(
        &self,
        raw: RawFire,
        event_sequence: u64,
        command_instance_id: Option<u32>,
    ) -> Result<(), String>;
    fn materialisation_input(
        &self,
        raw: RawMaterialisation,
    ) -> Result<MaterialisationInput, String>;
    fn resolution_input(&self, raw: RawResolution) -> Result<Option<ResolutionInput>, String>;
    fn coverage_receipt(&self) -> Result<Option<CoverageReceipt>, String> {
        Ok(None)
    }
    fn capture_start_fired_baseline(&self) -> Result<CaptureStartFiredBaseline, String>;
    fn fired_ledger_shadow_matches(&self) -> Result<bool, String>;
}

struct Runtime {
    bridge: Arc<dyn A729EventBridge>,
    ledger: A729EventLedger,
    hooks: Mutex<Option<InstalledHookSet>>,
    hook_profile: RuntimeHookProfile,
    accepting: AtomicBool,
    in_flight: AtomicUsize,
    epoch: AtomicU64,
    terminal_custody_state: AtomicU8,
    authoritative_sink_identity_sha256: [u8; 32],
    authoritative_sink_authority: Mutex<Option<A729AuthoritativeSinkAuthority>>,
    d64_detach_authority: Mutex<
        Option<
            scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
                D64DetachAuthority,
        >,
    >,
}

const TERMINAL_CUSTODY_ACTIVE: u8 = 0;
const TERMINAL_CUSTODY_RESERVED: u8 = 1;
const TERMINAL_CUSTODY_RELEASED: u8 = 2;
const MAX_TERMINAL_CUSTODY_WAIT: Duration = Duration::from_secs(30);

fn terminal_custody_nonce() -> Result<[u8; 32], String> {
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8, len: u32, flags: u32) -> i32;
    }
    let mut nonce = [0u8; 32];
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            nonce.as_mut_ptr(),
            nonce.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 || nonce == [0; 32] {
        return Err(format!(
            "XUI21 terminal custody CSPRNG failed with NTSTATUS {status:#010x}"
        ));
    }
    Ok(nonce)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729TerminalLedgerSnapshot {
    capture_session_sha256: [u8; 32],
    custody_identity_sha256: [u8; 32],
    pub terminal_epoch: u64,
    pub frame: FiredEventFrame,
    pub payload_sha256: [u8; 32],
}

impl A729TerminalLedgerSnapshot {
    pub fn capture_session_sha256(&self) -> [u8; 32] {
        self.capture_session_sha256
    }

    pub fn encode_payload(&self) -> Result<Vec<u8>, String> {
        let payload = encode_payload(&self.frame)
            .map_err(|error| format!("XUI21 terminal payload encoding failed: {error:?}"))?;
        let digest: [u8; 32] = Sha256::digest(&payload).into();
        if digest != self.payload_sha256 {
            return Err("XUI21 terminal payload digest changed after reservation".into());
        }
        Ok(payload)
    }
}

/// release hooks. The caller must explicitly commit after sink acceptance, or
#[must_use = "terminal D75 custody must be committed or explicitly aborted"]
pub struct A729TerminalCustodyReservation {
    runtime: Option<Arc<Runtime>>,
    snapshot: A729TerminalLedgerSnapshot,
    semantic_hook_outcomes: Vec<a729_inline_hook::InstalledExactRvaOutcome>,
    observer_detached_before_final_d64: bool,
    d64_permit_issued: bool,
    d64_terminal_lifecycle_envelope_sha256: Option<[u8; 32]>,
    d64_terminal_del2_sha256: Option<[u8; 32]>,
    d64_poisoned_abort_envelope_sha256: Option<[u8; 32]>,
    publication_permit_issued: bool,
    publication_permit_nonce: Option<[u8; 32]>,
}

#[must_use = "the permit must be consumed by the final D64 drain binding"]
pub struct A729D64FinalDrainPermit {
    authority: Option<
        scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::D64DetachAuthority,
    >,
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
}

#[must_use = "D64 failure retains authority for retry or authenticated abort"]
pub struct A729D64FinalDrainAttemptFailure {
    message: String,
    permit: A729D64FinalDrainPermit,
}

impl A729D64FinalDrainAttemptFailure {
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn retry_available(&self) -> bool {
        self.permit
            .authority
            .as_ref()
            .is_some_and(|authority| authority.terminal_attempts_remaining() != 0)
    }

    pub fn into_recovery_permit(self) -> A729D64FinalDrainPermit {
        self.permit
    }
}

impl core::fmt::Debug for A729D64FinalDrainAttemptFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("A729D64FinalDrainAttemptFailure")
            .field("message", &self.message)
            .field(
                "retry_available",
                &self
                    .permit
                    .authority
                    .as_ref()
                    .is_some_and(|authority| authority.terminal_attempts_remaining() != 0),
            )
            .finish()
    }
}

impl core::fmt::Display for A729D64FinalDrainAttemptFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for A729D64FinalDrainAttemptFailure {}

impl A729D64FinalDrainPermit {
    pub fn run_final_d64_drain(
        mut self,
    ) -> Result<
        scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
            AuthenticatedD64TerminalDrainReceipt,
        A729D64FinalDrainAttemptFailure,
    >{
        let Some(authority) = self.authority.take() else {
            return Err(A729D64FinalDrainAttemptFailure {
                message: "XUI21 final D64 authority was already consumed".into(),
                permit: self,
            });
        };
        let result = scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
            seal_drain_and_uninstall_for_d75_terminal_custody(
                authority,
                self.custody_identity_sha256,
                self.frame_hour,
                self.elapsed_wall_ms,
            );
        match result {
            Ok(receipt) => Ok(receipt),
            Err(failure) => {
                let message = failure.message().to_string();
                self.authority = Some(failure.into_authority());
                Err(A729D64FinalDrainAttemptFailure {
                    message,
                    permit: self,
                })
            }
        }
    }

    pub fn abort_final_d64_custody(
        mut self,
        reason_sha256: [u8; 32],
    ) -> Result<
        scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
            AuthenticatedPoisonedD64AbortReceipt,
        A729D64AbortAttemptFailure,
    >{
        let Some(authority) = self.authority.take() else {
            return Err(A729D64AbortAttemptFailure {
                message: "XUI21 final D64 abort authority was already consumed".into(),
                permit: self,
            });
        };
        match scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
            abort_d75_terminal_custody(
                authority,
                self.custody_identity_sha256,
                self.frame_hour,
                self.elapsed_wall_ms,
                reason_sha256,
            ) {
            Ok(receipt) => Ok(receipt),
            Err(failure) => {
                let message = failure.message().to_string();
                self.authority = Some(failure.into_authority());
                Err(A729D64AbortAttemptFailure {
                    message,
                    permit: self,
                })
            }
        }
    }
}

#[must_use = "D64 abort failure retains authority for another bounded cleanup attempt"]
pub struct A729D64AbortAttemptFailure {
    message: String,
    permit: A729D64FinalDrainPermit,
}

impl A729D64AbortAttemptFailure {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn into_retry_permit(self) -> A729D64FinalDrainPermit {
        self.permit
    }
}

impl core::fmt::Debug for A729D64AbortAttemptFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("A729D64AbortAttemptFailure")
            .field("message", &self.message)
            .field(
                "retry_available",
                &self
                    .permit
                    .authority
                    .as_ref()
                    .is_some_and(|authority| authority.terminal_attempts_remaining() != 0),
            )
            .finish()
    }
}

impl core::fmt::Display for A729D64AbortAttemptFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for A729D64AbortAttemptFailure {}

impl A729TerminalCustodyReservation {
    pub fn snapshot(&self) -> &A729TerminalLedgerSnapshot {
        &self.snapshot
    }

    /// zero check is retained in the release receipt and checked again at
    pub fn confirm_dispatch_observer_detached_before_final_d64(
        &mut self,
    ) -> Result<A729D64FinalDrainPermit, String> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| "XUI21 terminal custody was already released".to_string())?;
        if runtime.in_flight.load(Ordering::Acquire) != 0 {
            return Err(
                "XUI21 D75 ledger transactions are still in flight at observer detachment".into(),
            );
        }
        if A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0 {
            return Err(
                "XUI21 D75->D64 observer is still registered at the terminal boundary".into(),
            );
        }
        if self.d64_permit_issued {
            return Err("XUI21 final D64 drain permit was already issued".into());
        }
        let authority = runtime
            .d64_detach_authority
            .lock()
            .map_err(|_| "XUI21 D64 detach-authority mutex poisoned".to_string())?
            .take()
            .ok_or_else(|| "XUI21 D64 detach authority was not bound before READY".to_string())?;
        self.observer_detached_before_final_d64 = true;
        self.d64_permit_issued = true;
        Ok(A729D64FinalDrainPermit {
            authority: Some(authority),
            custody_identity_sha256: self.snapshot.custody_identity_sha256,
            frame_hour: self.snapshot.frame.frame_hour,
            elapsed_wall_ms: self.snapshot.frame.elapsed_wall_ms,
        })
    }

    pub fn bind_final_d64_drain_receipt(
        &mut self,
        receipt: &scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
            AuthenticatedD64TerminalDrainReceipt,
    ) -> Result<(), String> {
        if !self.observer_detached_before_final_d64
            || A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0
        {
            return Err("XUI21 final D64 receipt cannot bind before observer detachment".into());
        }
        if self.d64_terminal_lifecycle_envelope_sha256.is_some() {
            return Err("XUI21 final D64 receipt is already bound".into());
        }
        if receipt.custody_identity_sha256() != self.snapshot.custody_identity_sha256
            || receipt.frame_hour() != self.snapshot.frame.frame_hour
            || receipt.elapsed_wall_ms() != self.snapshot.frame.elapsed_wall_ms
        {
            return Err("XUI21 final D64 drain permit does not belong to this custody".into());
        }
        let observed_d64_digest: [u8; 32] = Sha256::digest(receipt.lifecycle_envelope()).into();
        if !receipt.diagnostics_clean()
            || receipt.lifecycle_envelope().is_empty()
            || receipt.lifecycle_envelope_sha256() == [0; 32]
            || observed_d64_digest != receipt.lifecycle_envelope_sha256()
        {
            return Err(
                "XUI21 final D64 drain receipt is incomplete, poisoned, or digest-invalid".into(),
            );
        }
        self.d64_terminal_lifecycle_envelope_sha256 = Some(receipt.lifecycle_envelope_sha256());
        let observed_del2_digest: [u8; 32] = Sha256::digest(receipt.terminal_del2()).into();
        if receipt.terminal_del2().is_empty()
            || receipt.terminal_del2_sha256() == [0; 32]
            || observed_del2_digest != receipt.terminal_del2_sha256()
        {
            self.d64_terminal_lifecycle_envelope_sha256 = None;
            return Err("XUI21 final D64 DEL2 custody is empty or digest-invalid".into());
        }
        self.d64_terminal_del2_sha256 = Some(receipt.terminal_del2_sha256());
        Ok(())
    }

    pub fn bind_poisoned_d64_abort_receipt(
        &mut self,
        receipt: &scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
            AuthenticatedPoisonedD64AbortReceipt,
    ) -> Result<(), String> {
        if !self.observer_detached_before_final_d64
            || A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0
        {
            return Err("XUI21 poisoned D64 abort cannot bind before observer detachment".into());
        }
        if self.d64_terminal_lifecycle_envelope_sha256.is_some()
            || self.d64_terminal_del2_sha256.is_some()
            || self.d64_poisoned_abort_envelope_sha256.is_some()
        {
            return Err("XUI21 terminal D64 custody is already bound".into());
        }
        if receipt.custody_identity_sha256() != self.snapshot.custody_identity_sha256
            || receipt.frame_hour() != self.snapshot.frame.frame_hour
            || receipt.elapsed_wall_ms() != self.snapshot.frame.elapsed_wall_ms
            || !receipt.capture_poisoned()
        {
            return Err("XUI21 poisoned D64 abort receipt does not belong to this custody".into());
        }
        let observed: [u8; 32] = Sha256::digest(receipt.abort_envelope()).into();
        if receipt.abort_envelope().is_empty()
            || receipt.abort_envelope_sha256() == [0; 32]
            || observed != receipt.abort_envelope_sha256()
        {
            return Err("XUI21 poisoned D64 abort receipt is digest-invalid".into());
        }
        self.d64_poisoned_abort_envelope_sha256 = Some(receipt.abort_envelope_sha256());
        Ok(())
    }

    pub fn take_terminal_publication_permit(
        &mut self,
        terminal_h4cr_frame: &[u8],
    ) -> Result<A729TerminalPublicationPermit, String> {
        if self.runtime.is_none() {
            return Err("XUI21 terminal custody was already released".into());
        }
        if self.d64_terminal_lifecycle_envelope_sha256.is_none()
            || self.d64_terminal_del2_sha256.is_none()
            || self.d64_poisoned_abort_envelope_sha256.is_some()
        {
            return Err(
                "XUI21 terminal publication requires a clean authenticated final D64 drain".into(),
            );
        }
        if self.publication_permit_issued {
            return Err("XUI21 terminal publication permit was already issued".into());
        }
        let nonce = terminal_custody_nonce()?;
        let d64_terminal_lifecycle_envelope_sha256 = self
            .d64_terminal_lifecycle_envelope_sha256
            .ok_or("XUI21 final D64 digest disappeared before publication binding")?;
        let d64_terminal_del2_sha256 = self
            .d64_terminal_del2_sha256
            .ok_or("XUI21 final D64 DEL2 digest disappeared before publication binding")?;
        let terminal_frame_hour_raw = i32::try_from(self.snapshot.frame.frame_hour)
            .map_err(|_| "XUI21 terminal frame hour exceeds the H4CR i32 range")?;
        let mut permit = A729TerminalPublicationPermit {
            capture_session_sha256: self.snapshot.capture_session_sha256,
            custody_identity_sha256: self.snapshot.custody_identity_sha256,
            authoritative_sink_identity_sha256: self
                .runtime
                .as_ref()
                .ok_or("XUI21 runtime disappeared before publication binding")?
                .authoritative_sink_identity_sha256,
            terminal_payload_sha256: self.snapshot.payload_sha256,
            d64_terminal_lifecycle_envelope_sha256,
            d64_terminal_del2_sha256,
            terminal_frame_hour_raw,
            authenticated_terminal_h4cr_frame_sha256: Sha256::digest(terminal_h4cr_frame).into(),
            raw_d75_record_sequence: 0,
            raw_d64_record_sequence: 0,
            terminal_frame_end_sequence: 0,
            permit_nonce: nonce,
        };
        let binding = authenticate_terminal_h4cr_frame(&permit, terminal_h4cr_frame)?;
        permit.raw_d75_record_sequence = binding.raw_d75_record_sequence;
        permit.raw_d64_record_sequence = binding.raw_d64_record_sequence;
        permit.terminal_frame_end_sequence = binding.terminal_frame_end_sequence;
        self.publication_permit_issued = true;
        self.publication_permit_nonce = Some(nonce);
        Ok(permit)
    }
}

#[must_use = "the one-shot permit must be consumed only after complete authoritative sink acceptance"]
pub struct A729TerminalPublicationPermit {
    capture_session_sha256: [u8; 32],
    custody_identity_sha256: [u8; 32],
    authoritative_sink_identity_sha256: [u8; 32],
    terminal_payload_sha256: [u8; 32],
    d64_terminal_lifecycle_envelope_sha256: [u8; 32],
    d64_terminal_del2_sha256: [u8; 32],
    terminal_frame_hour_raw: i32,
    authenticated_terminal_h4cr_frame_sha256: [u8; 32],
    raw_d75_record_sequence: u64,
    raw_d64_record_sequence: u64,
    terminal_frame_end_sequence: u64,
    permit_nonce: [u8; 32],
}

impl A729TerminalPublicationPermit {
    pub fn bind_authoritative_sink<'a, W: Write>(
        &self,
        authority: A729AuthoritativeSinkAuthority,
        writer: &'a mut W,
    ) -> Result<A729AuthoritativeTerminalSink<'a>, String> {
        if authority.sink_identity_sha256 != self.authoritative_sink_identity_sha256 {
            return Err(
                "XUI21 authoritative sink capability belongs to another runtime session".into(),
            );
        }
        Ok(A729AuthoritativeTerminalSink {
            writer,
            permit_nonce: self.permit_nonce,
            sink_identity_sha256: authority.sink_identity_sha256,
        })
    }
}

#[must_use = "publication failure must be classified as pre-write retryable or sink-indeterminate"]
pub struct A729TerminalPublicationFailure {
    message: String,
    retry_permit: Option<A729TerminalPublicationPermit>,
    sink_state_indeterminate: bool,
}

impl A729TerminalPublicationFailure {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn retry_permit_available(&self) -> bool {
        self.retry_permit.is_some()
    }

    pub fn sink_state_indeterminate(&self) -> bool {
        self.sink_state_indeterminate
    }

    pub fn into_retry_permit(mut self) -> Result<A729TerminalPublicationPermit, Self> {
        if let Some(permit) = self.retry_permit.take() {
            Ok(permit)
        } else {
            Err(self)
        }
    }
}

impl core::fmt::Debug for A729TerminalPublicationFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("A729TerminalPublicationFailure")
            .field("message", &self.message)
            .field("permit_retained", &self.retry_permit.is_some())
            .field("sink_state_indeterminate", &self.sink_state_indeterminate)
            .finish()
    }
}

#[must_use = "authoritative sink authority must be consumed by terminal publication"]
pub struct A729AuthoritativeSinkAuthority {
    sink_identity_sha256: [u8; 32],
}

#[must_use = "failed sink-authority restoration retains the authority for recovery"]
pub struct A729AuthoritativeSinkAuthorityRestoreFailure {
    message: String,
    authority: A729AuthoritativeSinkAuthority,
}

impl A729AuthoritativeSinkAuthorityRestoreFailure {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn into_authority(self) -> A729AuthoritativeSinkAuthority {
        self.authority
    }
}

impl core::fmt::Debug for A729AuthoritativeSinkAuthorityRestoreFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("A729AuthoritativeSinkAuthorityRestoreFailure")
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl core::fmt::Display for A729AuthoritativeSinkAuthorityRestoreFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for A729AuthoritativeSinkAuthorityRestoreFailure {}

pub fn take_authoritative_terminal_sink_authority() -> Result<A729AuthoritativeSinkAuthority, String>
{
    let runtime = runtime_slot()
        .lock()
        .map_err(|_| "XUI21 runtime slot poisoned".to_string())?
        .as_ref()
        .cloned()
        .ok_or_else(|| "XUI21 runtime is not initialized".to_string())?;
    if runtime.hook_profile != RuntimeHookProfile::ProductionSemantic
        || !runtime.accepting.load(Ordering::Acquire)
        || runtime.terminal_custody_state.load(Ordering::Acquire) != TERMINAL_CUSTODY_ACTIVE
    {
        return Err(
            "XUI21 authoritative sink capability requires active production custody".into(),
        );
    }
    let taken = runtime
        .authoritative_sink_authority
        .lock()
        .map_err(|_| "XUI21 authoritative sink authority mutex poisoned".to_string())?
        .take()
        .ok_or_else(|| "XUI21 authoritative sink capability was already taken".to_string());
    taken
}

pub fn restore_authoritative_terminal_sink_authority(
    authority: A729AuthoritativeSinkAuthority,
) -> Result<(), A729AuthoritativeSinkAuthorityRestoreFailure> {
    let runtime_slot_guard = match runtime_slot().lock() {
        Ok(slot) => slot,
        Err(_) => {
            return Err(A729AuthoritativeSinkAuthorityRestoreFailure {
                message: "XUI21 runtime slot is poisoned".into(),
                authority,
            })
        }
    };
    let Some(runtime) = runtime_slot_guard.as_ref() else {
        return Err(A729AuthoritativeSinkAuthorityRestoreFailure {
            message: "XUI21 runtime is not initialized".into(),
            authority,
        });
    };
    if runtime.authoritative_sink_identity_sha256 != authority.sink_identity_sha256 {
        return Err(A729AuthoritativeSinkAuthorityRestoreFailure {
            message: "XUI21 authoritative sink capability belongs to another runtime session"
                .into(),
            authority,
        });
    }
    if runtime.hook_profile != RuntimeHookProfile::ProductionSemantic
        || !runtime.accepting.load(Ordering::Acquire)
        || runtime.terminal_custody_state.load(Ordering::Acquire) != TERMINAL_CUSTODY_ACTIVE
    {
        return Err(A729AuthoritativeSinkAuthorityRestoreFailure {
            message:
                "XUI21 authoritative sink capability can restore only before terminal reservation"
                    .into(),
            authority,
        });
    }
    let mut slot = match runtime.authoritative_sink_authority.lock() {
        Ok(slot) => slot,
        Err(_) => {
            return Err(A729AuthoritativeSinkAuthorityRestoreFailure {
                message: "XUI21 authoritative sink authority mutex is poisoned".into(),
                authority,
            })
        }
    };
    if slot.is_some() {
        return Err(A729AuthoritativeSinkAuthorityRestoreFailure {
            message: "XUI21 authoritative sink capability slot is already occupied".into(),
            authority,
        });
    }
    *slot = Some(authority);
    Ok(())
}

pub struct A729AuthoritativeTerminalSink<'a> {
    writer: &'a mut dyn Write,
    permit_nonce: [u8; 32],
    sink_identity_sha256: [u8; 32],
}

pub fn publish_terminal_h4cr_to_authoritative_sink(
    permit: A729TerminalPublicationPermit,
    terminal_h4cr_frame: &[u8],
    sink: &mut A729AuthoritativeTerminalSink<'_>,
) -> Result<A729DurableTerminalFrameAcceptanceReceipt, A729TerminalPublicationFailure> {
    if sink.permit_nonce != permit.permit_nonce
        || sink.sink_identity_sha256 != permit.authoritative_sink_identity_sha256
    {
        return Err(A729TerminalPublicationFailure {
            message: "XUI21 authoritative terminal sink belongs to another custody permit".into(),
            retry_permit: Some(permit),
            sink_state_indeterminate: false,
        });
    }
    let binding = match authenticate_terminal_h4cr_frame(&permit, terminal_h4cr_frame) {
        Ok(binding) => binding,
        Err(message) => {
            return Err(A729TerminalPublicationFailure {
                message,
                retry_permit: Some(permit),
                sink_state_indeterminate: false,
            });
        }
    };
    if let Err(refusal) = write_terminal_frame_to_sink(sink.writer, terminal_h4cr_frame) {
        let message = format!(
            "XUI21 authoritative terminal H4CR sink rejected publication: {}",
            refusal.error
        );
        return Err(if refusal.reached_the_sink {
            A729TerminalPublicationFailure {
                message,
                retry_permit: None,
                sink_state_indeterminate: true,
            }
        } else {
            A729TerminalPublicationFailure {
                message,
                retry_permit: Some(permit),
                sink_state_indeterminate: false,
            }
        });
    }
    let acceptance_authenticator_sha256 = terminal_acceptance_authenticator(
        permit.permit_nonce,
        permit.capture_session_sha256,
        permit.custody_identity_sha256,
        permit.authoritative_sink_identity_sha256,
        permit.terminal_payload_sha256,
        permit.d64_terminal_lifecycle_envelope_sha256,
        permit.d64_terminal_del2_sha256,
        permit.terminal_frame_hour_raw,
        binding.raw_d75_record_sequence,
        binding.raw_d64_record_sequence,
        binding.terminal_frame_end_sequence,
        permit.authenticated_terminal_h4cr_frame_sha256,
    );
    Ok(A729DurableTerminalFrameAcceptanceReceipt {
        capture_session_sha256: permit.capture_session_sha256,
        custody_identity_sha256: permit.custody_identity_sha256,
        authoritative_sink_identity_sha256: permit.authoritative_sink_identity_sha256,
        terminal_payload_sha256: permit.terminal_payload_sha256,
        d64_terminal_lifecycle_envelope_sha256: permit.d64_terminal_lifecycle_envelope_sha256,
        d64_terminal_del2_sha256: permit.d64_terminal_del2_sha256,
        terminal_frame_hour_raw: permit.terminal_frame_hour_raw,
        raw_d75_record_sequence: binding.raw_d75_record_sequence,
        raw_d64_record_sequence: binding.raw_d64_record_sequence,
        terminal_frame_end_sequence: binding.terminal_frame_end_sequence,
        authenticated_terminal_h4cr_frame_sha256: permit.authenticated_terminal_h4cr_frame_sha256,
        acceptance_authenticator_sha256,
    })
}

struct TerminalSinkRefusal {
    error: std::io::Error,
    reached_the_sink: bool,
}

const TERMINAL_SINK_INTERRUPTED_RETRIES: u32 = 64;

fn refusal_is_at_the_front_door(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::PermissionDenied
            | ErrorKind::InvalidInput
            | ErrorKind::Unsupported
            | ErrorKind::NotFound
    )
}

fn write_terminal_frame_to_sink(
    writer: &mut dyn Write,
    frame: &[u8],
) -> Result<(), TerminalSinkRefusal> {
    let mut written = 0usize;
    let mut interrupted = 0u32;
    while written < frame.len() {
        match writer.write(&frame[written..]) {
            Ok(0) => {
                return Err(TerminalSinkRefusal {
                    error: std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "authoritative terminal sink accepted no further bytes",
                    ),
                    reached_the_sink: true,
                });
            }
            Ok(accepted) => written += accepted,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                interrupted += 1;
                if interrupted > TERMINAL_SINK_INTERRUPTED_RETRIES {
                    return Err(TerminalSinkRefusal {
                        error,
                        reached_the_sink: true,
                    });
                }
            }
            Err(error) => {
                let reached_the_sink = written > 0 || !refusal_is_at_the_front_door(&error);
                return Err(TerminalSinkRefusal {
                    error,
                    reached_the_sink,
                });
            }
        }
    }
    writer.flush().map_err(|error| TerminalSinkRefusal {
        error,
        reached_the_sink: true,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AuthenticatedTerminalFrameBinding {
    raw_d75_record_sequence: u64,
    raw_d64_record_sequence: u64,
    terminal_frame_end_sequence: u64,
}

fn authenticate_terminal_h4cr_frame(
    permit: &A729TerminalPublicationPermit,
    bytes: &[u8],
) -> Result<AuthenticatedTerminalFrameBinding, String> {
    let observed_frame_sha256: [u8; 32] = Sha256::digest(bytes).into();
    if observed_frame_sha256 != permit.authenticated_terminal_h4cr_frame_sha256 {
        return Err("XUI21 terminal H4CR bytes differ from the pre-authorized frame".into());
    }
    let mut offset = 0usize;
    let mut prior_sequence = None;
    let mut d75_sequence = None;
    let mut d64_sequence = None;
    let mut d75_perf_bound = false;
    let mut frame_end_sequence = None;
    while offset < bytes.len() {
        let header = compact_contract::parse_compact_record_header(&bytes[offset..])
            .map_err(|error| format!("XUI21 terminal H4CR framing is invalid: {error}"))?;
        if header.game_hour_raw != permit.terminal_frame_hour_raw {
            return Err(
                "XUI21 terminal H4CR contains a mixed or non-reserved game-hour label".into(),
            );
        }
        let sequence = header.sequence;
        if sequence == 0 || prior_sequence.is_some_and(|prior| sequence != prior + 1) {
            return Err("XUI21 terminal H4CR sequence range is invalid".into());
        }
        let end = offset
            .checked_add(header.record_len())
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "XUI21 terminal H4CR record is truncated".to_string())?;
        let payload = &bytes[offset + compact_contract::COMPACT_RECORD_HEADER_LEN..end];
        compact_contract::validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        )
        .map_err(|error| format!("XUI21 terminal H4CR semantics are invalid: {error}"))?;
        if header.domain_id == crate::DOMAIN_ID
            && header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA
        {
            let payload_sha256: [u8; 32] = Sha256::digest(payload).into();
            if d75_sequence.is_some() || payload_sha256 != permit.terminal_payload_sha256 {
                return Err(
                    "XUI21 terminal H4CR does not contain exactly the reserved D75 payload".into(),
                );
            }
            d75_sequence = Some(sequence);
        }
        if header.domain_id == crate::DOMAIN_ID
            && header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF
        {
            if d75_perf_bound
                || !perf_diagnostics_bind_d64_terminal_envelope(
                    payload,
                    permit.d64_terminal_lifecycle_envelope_sha256,
                    permit.d64_terminal_del2_sha256,
                )?
            {
                return Err(
                    "XUI21 terminal D75 PERF does not uniquely bind the authenticated D64 envelope"
                        .into(),
                );
            }
            d75_perf_bound = true;
        }
        if header.domain_id == 64
            && header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA
        {
            if d64_sequence.replace(sequence).is_some() {
                return Err("XUI21 terminal H4CR repeats D64 DATA".into());
            }
            if !d64_data_binds_terminal_del2(payload, permit.d64_terminal_del2_sha256)? {
                return Err(
                    "XUI21 terminal D64 DATA does not contain the authenticated DEL2 suffix".into(),
                );
            }
        }
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_END {
            if end != bytes.len() || frame_end_sequence.is_some() {
                return Err("XUI21 terminal H4CR FRAME_END is not uniquely final".into());
            }
            frame_end_sequence = Some(sequence);
        }
        prior_sequence = Some(sequence);
        offset = end;
    }
    let d75_sequence =
        d75_sequence.ok_or_else(|| "XUI21 terminal H4CR lacks reserved D75 DATA".to_string())?;
    let d64_sequence = d64_sequence
        .ok_or_else(|| "XUI21 terminal H4CR lacks authenticated D64 DATA".to_string())?;
    if !d75_perf_bound {
        return Err("XUI21 terminal H4CR lacks D75 PERF D64-envelope binding".into());
    }
    let frame_end_sequence = frame_end_sequence
        .ok_or_else(|| "XUI21 terminal H4CR lacks final FRAME_END".to_string())?;
    if frame_end_sequence <= d75_sequence {
        return Err("XUI21 terminal H4CR closes before its reserved D75 DATA".into());
    }
    if permit.raw_d75_record_sequence != 0
        && (permit.raw_d75_record_sequence != d75_sequence
            || permit.raw_d64_record_sequence != d64_sequence
            || permit.terminal_frame_end_sequence != frame_end_sequence)
    {
        return Err("XUI21 terminal H4CR sequence binding changed".into());
    }
    Ok(AuthenticatedTerminalFrameBinding {
        raw_d75_record_sequence: d75_sequence,
        raw_d64_record_sequence: d64_sequence,
        terminal_frame_end_sequence: frame_end_sequence,
    })
}

fn perf_diagnostics_bind_d64_terminal_envelope(
    payload: &[u8],
    expected_envelope_digest: [u8; 32],
    expected_del2_digest: [u8; 32],
) -> Result<bool, String> {
    const PERF_BASE_LEN: usize = 40;
    const TRAILER_HEADER_LEN: usize = 8;
    const ENTRY_HEADER_LEN: usize = 12;
    const EXPECTED_NAMES: [&[u8]; 8] = [
        b"d64_terminal_envelope_sha256_0",
        b"d64_terminal_envelope_sha256_1",
        b"d64_terminal_envelope_sha256_2",
        b"d64_terminal_envelope_sha256_3",
        b"d64_terminal_del2_sha256_0",
        b"d64_terminal_del2_sha256_1",
        b"d64_terminal_del2_sha256_2",
        b"d64_terminal_del2_sha256_3",
    ];
    if payload.len() < PERF_BASE_LEN + TRAILER_HEADER_LEN
        || &payload[PERF_BASE_LEN..PERF_BASE_LEN + 4] != b"PDU1"
    {
        return Ok(false);
    }
    let trailer_version =
        u16::from_le_bytes([payload[PERF_BASE_LEN + 4], payload[PERF_BASE_LEN + 5]]);
    if trailer_version
        != compact_contract::record_semantics::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION
        && trailer_version
            != compact_contract::record_semantics::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2
    {
        return Ok(false);
    }
    let count =
        u16::from_le_bytes([payload[PERF_BASE_LEN + 6], payload[PERF_BASE_LEN + 7]]) as usize;
    let mut cursor = PERF_BASE_LEN + TRAILER_HEADER_LEN;
    let mut observed = [None; EXPECTED_NAMES.len()];
    for _ in 0..count {
        let header_end = cursor
            .checked_add(ENTRY_HEADER_LEN)
            .filter(|end| *end <= payload.len())
            .ok_or("XUI21 terminal PERF diagnostic entry is truncated")?;
        let name_len = u16::from_le_bytes([payload[cursor], payload[cursor + 1]]) as usize;
        if trailer_version
            == compact_contract::record_semantics::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION
        {
            if payload[cursor + 2..cursor + 4] != [0, 0] {
                return Err("XUI21 terminal PERF diagnostic reserved field is nonzero".into());
            }
        } else {
            let declaration = u16::from_le_bytes([payload[cursor + 2], payload[cursor + 3]]);
            if compact_contract::diagnostic_channel::UnitRole::from_code(
                (declaration & 0x00FF) as u8,
            )
            .is_none()
                || compact_contract::diagnostic_channel::UnitPresence::from_code(
                    (declaration >> 8) as u8,
                )
                .is_none()
            {
                return Err("XUI21 terminal PERF diagnostic declaration is undefined".into());
            }
        }
        let value = u64::from_le_bytes(
            payload[cursor + 4..cursor + 12]
                .try_into()
                .map_err(|_| "XUI21 terminal PERF diagnostic value is truncated")?,
        );
        let name_end = header_end
            .checked_add(name_len)
            .filter(|end| *end <= payload.len())
            .ok_or("XUI21 terminal PERF diagnostic name is truncated")?;
        let name = &payload[header_end..name_end];
        if let Some(index) = EXPECTED_NAMES.iter().position(|expected| *expected == name) {
            if observed[index].replace(value).is_some() {
                return Err("XUI21 terminal PERF repeats a D64 envelope digest unit".into());
            }
        }
        cursor = name_end;
    }
    if cursor != payload.len() {
        return Err("XUI21 terminal PERF diagnostic trailer has trailing bytes".into());
    }
    let mut expected = [0u8; 64];
    expected[..32].copy_from_slice(&expected_envelope_digest);
    expected[32..].copy_from_slice(&expected_del2_digest);
    Ok(observed.into_iter().enumerate().all(|(index, value)| {
        value
            == Some(u64::from_le_bytes(
                expected[index * 8..index * 8 + 8]
                    .try_into()
                    .expect("fixed SHA-256 unit"),
            ))
    }))
}

fn d64_data_binds_terminal_del2(
    payload: &[u8],
    expected_del2_sha256: [u8; 32],
) -> Result<bool, String> {
    const D64_DATA_BASE_HEADER_LEN: usize = 20;
    // The d64 base row is 24 bytes: owner, event id, flags, originator, due,
    // scope country (`attach_log.rs::COMPACT_DELAYED_EVENT_ROW_LEN`). At 20 this
    // began hashing 4*n bytes before DEL2 and every populated frame failed
    // custody.
    const D64_DATA_ROW_LEN: usize = 24;
    if payload.len() < D64_DATA_BASE_HEADER_LEN {
        return Err("XUI21 terminal D64 DATA payload is truncated".into());
    }
    let row_count = u32::from_le_bytes(
        payload[8..12]
            .try_into()
            .expect("checked terminal D64 row count"),
    );
    let repeated = u32::from_le_bytes(
        payload[12..16]
            .try_into()
            .expect("checked terminal D64 row count"),
    );
    if row_count != repeated {
        return Err("XUI21 terminal D64 DATA row counts disagree".into());
    }
    let base_len = D64_DATA_BASE_HEADER_LEN
        .checked_add(
            usize::try_from(row_count)
                .map_err(|_| "XUI21 terminal D64 DATA row count overflow")?
                .checked_mul(D64_DATA_ROW_LEN)
                .ok_or("XUI21 terminal D64 DATA row byte length overflow")?,
        )
        .filter(|base_len| *base_len < payload.len())
        .ok_or("XUI21 terminal D64 DATA lacks its DEL2 suffix")?;
    let del2 = &payload[base_len..];
    Ok(del2.starts_with(b"DEL2") && <[u8; 32]>::from(Sha256::digest(del2)) == expected_del2_sha256)
}

pub struct A729DurableTerminalFrameAcceptanceReceipt {
    capture_session_sha256: [u8; 32],
    custody_identity_sha256: [u8; 32],
    authoritative_sink_identity_sha256: [u8; 32],
    terminal_payload_sha256: [u8; 32],
    d64_terminal_lifecycle_envelope_sha256: [u8; 32],
    d64_terminal_del2_sha256: [u8; 32],
    terminal_frame_hour_raw: i32,
    raw_d75_record_sequence: u64,
    raw_d64_record_sequence: u64,
    terminal_frame_end_sequence: u64,
    authenticated_terminal_h4cr_frame_sha256: [u8; 32],
    acceptance_authenticator_sha256: [u8; 32],
}

impl A729DurableTerminalFrameAcceptanceReceipt {
    pub fn capture_session_sha256(&self) -> [u8; 32] {
        self.capture_session_sha256
    }

    pub fn custody_identity_sha256(&self) -> [u8; 32] {
        self.custody_identity_sha256
    }

    pub fn terminal_payload_sha256(&self) -> [u8; 32] {
        self.terminal_payload_sha256
    }

    pub fn raw_d75_record_sequence(&self) -> u64 {
        self.raw_d75_record_sequence
    }

    pub fn d64_terminal_lifecycle_envelope_sha256(&self) -> [u8; 32] {
        self.d64_terminal_lifecycle_envelope_sha256
    }

    pub fn d64_terminal_del2_sha256(&self) -> [u8; 32] {
        self.d64_terminal_del2_sha256
    }

    pub fn terminal_frame_hour_raw(&self) -> i32 {
        self.terminal_frame_hour_raw
    }

    pub fn raw_d64_record_sequence(&self) -> u64 {
        self.raw_d64_record_sequence
    }

    pub fn terminal_frame_end_sequence(&self) -> u64 {
        self.terminal_frame_end_sequence
    }

    pub fn authenticated_terminal_h4cr_frame_sha256(&self) -> [u8; 32] {
        self.authenticated_terminal_h4cr_frame_sha256
    }
}

fn terminal_acceptance_authenticator(
    permit_nonce: [u8; 32],
    capture_session_sha256: [u8; 32],
    custody_identity_sha256: [u8; 32],
    authoritative_sink_identity_sha256: [u8; 32],
    terminal_payload_sha256: [u8; 32],
    d64_terminal_lifecycle_envelope_sha256: [u8; 32],
    d64_terminal_del2_sha256: [u8; 32],
    terminal_frame_hour_raw: i32,
    raw_d75_record_sequence: u64,
    raw_d64_record_sequence: u64,
    terminal_frame_end_sequence: u64,
    authenticated_terminal_h4cr_frame_sha256: [u8; 32],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"battle-plan.d75.durable_terminal_h4cr_acceptance.v1\0");
    digest.update(permit_nonce);
    digest.update(capture_session_sha256);
    digest.update(custody_identity_sha256);
    digest.update(authoritative_sink_identity_sha256);
    digest.update(terminal_payload_sha256);
    digest.update(d64_terminal_lifecycle_envelope_sha256);
    digest.update(d64_terminal_del2_sha256);
    digest.update(terminal_frame_hour_raw.to_le_bytes());
    digest.update(raw_d75_record_sequence.to_le_bytes());
    digest.update(raw_d64_record_sequence.to_le_bytes());
    digest.update(terminal_frame_end_sequence.to_le_bytes());
    digest.update(authenticated_terminal_h4cr_frame_sha256);
    digest.finalize().into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729TerminalCustodyReceipt {
    pub capture_session_sha256: [u8; 32],
    pub custody_identity_sha256: [u8; 32],
    pub terminal_epoch: u64,
    pub frame_hour: u64,
    pub row_count: u64,
    pub terminal_payload_sha256: [u8; 32],
    pub telemetry: HookTelemetry,
    pub fired_events_complete: bool,
    pub completeness_poisoned: bool,
    pub observer_detached_before_final_d64: bool,
    pub d64_final_drain_bound: bool,
    pub d64_terminal_lifecycle_envelope_sha256: Option<[u8; 32]>,
    pub d64_terminal_del2_sha256: Option<[u8; 32]>,
    pub d64_poisoned_abort_envelope_sha256: Option<[u8; 32]>,
    pub terminal_h4cr_d75_sequence: Option<u64>,
    pub terminal_h4cr_d64_sequence: Option<u64>,
    pub terminal_h4cr_frame_end_sequence: Option<u64>,
    pub authenticated_terminal_h4cr_frame_sha256: Option<[u8; 32]>,
    pub semantic_hook_outcomes: Vec<a729_inline_hook::InstalledExactRvaOutcome>,
    pub semantic_hook_release_receipts:
        Vec<a729_inline_hook::ReleasedProductionSemanticHookReceipt>,
    pub semantic_hooks_uninstalled: bool,
    pub publication_committed: bool,
    pub custody_successful: bool,
}

#[must_use = "failed terminal custody must be explicitly aborted"]
pub struct A729FailedTerminalCustodyReservation {
    runtime: Option<Arc<Runtime>>,
    capture_session_sha256: [u8; 32],
    custody_identity_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    failure: String,
    observer_detached: bool,
    d64_poisoned_abort_envelope_sha256: Option<[u8; 32]>,
}

impl A729FailedTerminalCustodyReservation {
    pub fn failure(&self) -> &str {
        &self.failure
    }

    pub fn confirm_dispatch_observer_detached(&mut self) -> Result<(), String> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| "XUI21 failed terminal custody was already released".to_string())?;
        if runtime.in_flight.load(Ordering::Acquire) != 0 {
            return Err("XUI21 failed custody still has D75 ledger transactions in flight".into());
        }
        if A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0 {
            return Err("XUI21 failed custody still has a registered D75->D64 observer".into());
        }
        self.observer_detached = true;
        Ok(())
    }

    pub fn take_d64_abort_permit(&mut self) -> Result<A729D64FinalDrainPermit, String> {
        if !self.observer_detached
            || A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0
        {
            return Err(
                "XUI21 failed custody requires observer detachment before D64 abort".into(),
            );
        }
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| "XUI21 failed terminal custody was already released".to_string())?;
        let authority = runtime
            .d64_detach_authority
            .lock()
            .map_err(|_| "XUI21 D64 detach-authority mutex poisoned".to_string())?
            .take()
            .ok_or_else(|| "XUI21 failed custody D64 authority was already issued".to_string())?;
        Ok(A729D64FinalDrainPermit {
            authority: Some(authority),
            custody_identity_sha256: self.custody_identity_sha256,
            frame_hour: self.frame_hour,
            elapsed_wall_ms: self.elapsed_wall_ms,
        })
    }

    pub fn bind_poisoned_d64_abort_receipt(
        &mut self,
        receipt: &scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
            AuthenticatedPoisonedD64AbortReceipt,
    ) -> Result<(), String> {
        if receipt.custody_identity_sha256() != self.custody_identity_sha256
            || receipt.frame_hour() != self.frame_hour
            || receipt.elapsed_wall_ms() != self.elapsed_wall_ms
            || !receipt.capture_poisoned()
        {
            return Err("XUI21 failed-custody D64 abort receipt identity mismatch".into());
        }
        let observed: [u8; 32] = Sha256::digest(receipt.abort_envelope()).into();
        if receipt.abort_envelope().is_empty()
            || receipt.abort_envelope_sha256() == [0; 32]
            || observed != receipt.abort_envelope_sha256()
        {
            return Err("XUI21 failed-custody D64 abort receipt is digest-invalid".into());
        }
        self.d64_poisoned_abort_envelope_sha256 = Some(receipt.abort_envelope_sha256());
        Ok(())
    }
}

pub struct A729TerminalCustodySealFailure {
    message: String,
    recovery: Option<A729FailedTerminalCustodyReservation>,
}

impl A729TerminalCustodySealFailure {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn into_recovery(self) -> Option<A729FailedTerminalCustodyReservation> {
        self.recovery
    }
}

impl core::fmt::Debug for A729TerminalCustodySealFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("A729TerminalCustodySealFailure")
            .field("message", &self.message)
            .field("recovery_available", &self.recovery.is_some())
            .finish()
    }
}

impl core::fmt::Display for A729TerminalCustodySealFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for A729TerminalCustodySealFailure {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729FailedTerminalCustodyReceipt {
    pub capture_session_sha256: [u8; 32],
    pub custody_identity_sha256: [u8; 32],
    pub frame_hour: u64,
    pub d64_poisoned_abort_envelope_sha256: [u8; 32],
    pub failure: String,
    pub observer_detached: bool,
    pub semantic_hooks_uninstalled: bool,
    pub semantic_hook_release_receipts:
        Vec<a729_inline_hook::ReleasedProductionSemanticHookReceipt>,
    pub completeness_poisoned: bool,
    pub custody_successful: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeHookProfile {
    ProductionSemantic,
    #[cfg(test)]
    FullEvidenceUnshipped,
}

impl RuntimeHookProfile {
    const fn materialisation_hook_installed(self) -> bool {
        match self {
            Self::ProductionSemantic => false,
            #[cfg(test)]
            Self::FullEvidenceUnshipped => true,
        }
    }
}

fn runtime_slot() -> &'static Mutex<Option<Arc<Runtime>>> {
    RUNTIME.get_or_init(|| Mutex::new(None))
}

struct RuntimeTransaction {
    runtime: Arc<Runtime>,
}

impl Drop for RuntimeTransaction {
    fn drop(&mut self) {
        self.runtime.epoch.fetch_add(1, Ordering::AcqRel);
        self.runtime.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

fn begin_runtime_transaction(
    callback: &a729_inline_hook::CallbackGuard,
) -> Option<RuntimeTransaction> {
    if !callback.semantic_admission_open() {
        return None;
    }
    let runtime = runtime_slot().lock().ok()?.as_ref()?.clone();
    if !runtime.accepting.load(Ordering::Acquire) {
        return None;
    }
    runtime.in_flight.fetch_add(1, Ordering::AcqRel);
    if !runtime.accepting.load(Ordering::Acquire) {
        runtime.in_flight.fetch_sub(1, Ordering::AcqRel);
        return None;
    }
    runtime.epoch.fetch_add(1, Ordering::AcqRel);
    Some(RuntimeTransaction { runtime })
}

#[derive(Clone, Copy)]
struct CommandObservation {
    instance_id: u32,
    event: usize,
    recipient_tag_id: u32,
    event_scope: usize,
    option_ordinal: u32,
}

#[derive(Default)]
struct MaterialisationCapture {
    event_window: usize,
    event: usize,
    admitted_option_pointers: Vec<usize>,
    selected_desc_key: Option<String>,
    shown_picture_gfx: Option<String>,
    runtime_text_owners: Vec<ObservedRuntimeTextOwner>,
    runtime_text_pin_failed: bool,
}

pub fn observe_materialising_runtime_text_owner(observation: ObservedRuntimeTextOwner) -> bool {
    if observation.textbox_owner == 0
        || observation.context_id.trim().is_empty()
        || observation.view_id.trim().is_empty()
        || observation.element_id.trim().is_empty()
    {
        return false;
    }
    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        let Some(capture) = context.materialisation.as_mut() else {
            return false;
        };
        if let Some(existing) = capture
            .runtime_text_owners
            .iter_mut()
            .find(|row| row.textbox_owner == observation.textbox_owner)
        {
            *existing = observation;
        } else {
            capture.runtime_text_owners.push(observation);
        }
        true
    })
}

pub fn materialisation_capture_active() -> bool {
    CONTEXT.with(|context| context.borrow().materialisation.is_some())
}

pub fn note_materialising_runtime_text_pin_failure(textbox_owner: usize) {
    CONTEXT.with(|context| {
        if let Some(capture) = context.borrow_mut().materialisation.as_mut() {
            capture.runtime_text_pin_failed = true;
            capture
                .runtime_text_owners
                .retain(|row| row.textbox_owner != textbox_owner);
        }
    });
}

struct PictureProbe {
    selected: Option<String>,
}

fn record_successful_picture_candidate(probe: &mut PictureProbe, candidate: String) {
    if probe.selected.is_none() {
        probe.selected = Some(candidate);
    }
}

#[derive(Default)]
struct CallbackContext {
    materialisation: Option<MaterialisationCapture>,
    picture_probe: Option<PictureProbe>,
    resolution_stack: Vec<ResolutionFrame>,
}

struct ResolutionFrame {
    path: ResolutionPath,
    owner: usize,
    event_sequence_hint: Option<u64>,
    instance_id: u32,
    event: usize,
    recipient_tag_id: u32,
    event_scope: usize,
    observation: Option<CommandObservation>,
}

thread_local! {
    static CONTEXT: RefCell<CallbackContext> = RefCell::new(CallbackContext::default());
}

pub fn install_a729_event_runtime(
    image_base: usize,
    bridge: Arc<dyn A729EventBridge>,
    capacity: usize,
) -> Result<(), String> {
    install_a729_event_runtime_inner(
        image_base,
        bridge,
        capacity,
        RuntimeHookProfile::ProductionSemantic,
    )
}

#[cfg(test)]
pub fn install_a729_full_evidence_runtime_unshipped(
    image_base: usize,
    bridge: Arc<dyn A729EventBridge>,
    capacity: usize,
) -> Result<(), String> {
    install_a729_event_runtime_inner(
        image_base,
        bridge,
        capacity,
        RuntimeHookProfile::FullEvidenceUnshipped,
    )
}

fn install_a729_event_runtime_inner(
    image_base: usize,
    bridge: Arc<dyn A729EventBridge>,
    capacity: usize,
    hook_profile: RuntimeHookProfile,
) -> Result<(), String> {
    if bridge.producer_manifest_sha256() != PRODUCER_MANIFEST_SHA256 {
        return Err("XUI21 bridge manifest does not match the pinned producer graph".into());
    }
    let authoritative_sink_identity_sha256 = terminal_custody_nonce()?;
    let runtime = Arc::new(Runtime {
        bridge,
        ledger: A729EventLedger::new(capacity),
        hooks: Mutex::new(None),
        hook_profile,
        accepting: AtomicBool::new(false),
        in_flight: AtomicUsize::new(0),
        epoch: AtomicU64::new(0),
        terminal_custody_state: AtomicU8::new(TERMINAL_CUSTODY_ACTIVE),
        authoritative_sink_identity_sha256,
        authoritative_sink_authority: Mutex::new(Some(A729AuthoritativeSinkAuthority {
            sink_identity_sha256: authoritative_sink_identity_sha256,
        })),
        d64_detach_authority: Mutex::new(None),
    });
    let mut slot = runtime_slot()
        .lock()
        .map_err(|_| "XUI21 runtime slot poisoned".to_string())?;
    if slot.is_some() {
        return Err("XUI21 runtime already has an active session".into());
    }
    let mut installed = match hook_profile {
        RuntimeHookProfile::ProductionSemantic => {
            let bindings = production_semantic_callback_bindings();
            unsafe { a729_inline_hook::install_production_semantic(image_base, &bindings) }
                .map_err(|error| {
                    format!("XUI21 semantic-only hook installation failed: {error:?}")
                })?
        }
        #[cfg(test)]
        RuntimeHookProfile::FullEvidenceUnshipped => {
            let bindings = full_evidence_callback_bindings_unshipped();
            unsafe { a729_inline_hook::install_full_evidence_unshipped(image_base, &bindings) }
                .map_err(|error| {
                    format!("XUI21 unshipped full-evidence hook installation failed: {error:?}")
                })?
        }
    };
    runtime
        .ledger
        .record_installation_validation(hook_profile.materialisation_hook_installed());

    let finish = (|| {
        let baseline = runtime.bridge.pending_baseline()?;
        for row in baseline.rows {
            runtime.ledger.record_fire(row);
        }
        runtime.ledger.record_baseline_complete(baseline.complete);
        runtime
            .ledger
            .record_capture_start_fired_baseline(runtime.bridge.capture_start_fired_baseline()?);
        let coverage = certified_route_attestation()?;
        runtime.ledger.record_route_certification(
            coverage.all_event_class_routes_certified,
            coverage.player_ai_timeout_no_choice_routes_certified,
        );
        if !runtime.bridge.fired_ledger_shadow_matches()? {
            runtime.ledger.note_shadow_mismatch();
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = finish {
        installed
            .try_uninstall()
            .map_err(|uninstall| format!("{error}; hook rollback failed: {uninstall:?}"))?;
        return Err(error);
    }
    {
        let mut hooks = runtime
            .hooks
            .lock()
            .map_err(|_| "XUI21 hook-set mutex poisoned".to_string())?;
        *hooks = Some(installed);
    }
    runtime.accepting.store(true, Ordering::Release);
    *slot = Some(runtime.clone());
    runtime
        .hooks
        .lock()
        .map_err(|_| "XUI21 hook-set mutex poisoned".to_string())?
        .as_mut()
        .ok_or_else(|| "XUI21 hook set disappeared before publication".to_string())?
        .publish();
    Ok(())
}

#[must_use = "bind failure retains D64 authority for authenticated cancellation"]
pub struct A729D64AuthorityBindFailure {
    message: String,
    authority:
        scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::D64DetachAuthority,
}

impl A729D64AuthorityBindFailure {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn into_authority(
        self,
    ) -> scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::D64DetachAuthority
    {
        self.authority
    }
}

impl core::fmt::Debug for A729D64AuthorityBindFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("A729D64AuthorityBindFailure")
            .field("message", &self.message)
            .finish()
    }
}

impl core::fmt::Display for A729D64AuthorityBindFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for A729D64AuthorityBindFailure {}

pub fn bind_d64_terminal_detach_authority(
    authority: scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
        D64DetachAuthority,
) -> Result<(), A729D64AuthorityBindFailure> {
    let slot = runtime_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(runtime) = slot.as_ref() else {
        return Err(A729D64AuthorityBindFailure {
            message: "XUI21 runtime is not initialized".into(),
            authority,
        });
    };
    if runtime.hook_profile != RuntimeHookProfile::ProductionSemantic
        || !runtime.accepting.load(Ordering::Acquire)
        || runtime.terminal_custody_state.load(Ordering::Acquire) != TERMINAL_CUSTODY_ACTIVE
        || A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) == 0
    {
        return Err(A729D64AuthorityBindFailure {
            message: "XUI21 D64 detach authority cannot bind outside active production custody"
                .into(),
            authority,
        });
    }
    let mut bound = match runtime.d64_detach_authority.lock() {
        Ok(bound) => bound,
        Err(_) => {
            return Err(A729D64AuthorityBindFailure {
                message: "XUI21 D64 detach-authority mutex poisoned".into(),
                authority,
            })
        }
    };
    if bound.is_some() {
        return Err(A729D64AuthorityBindFailure {
            message: "XUI21 D64 detach authority is already bound".into(),
            authority,
        });
    }
    *bound = Some(authority);
    Ok(())
}

/// two-phase terminal-custody API so hook release can never precede terminal
pub fn uninstall_a729_event_runtime() -> Result<(), String> {
    let slot = runtime_slot()
        .lock()
        .map_err(|_| "XUI21 runtime slot poisoned".to_string())?;
    if a729_inline_hook::failed_install_cleanup_pending() {
        a729_inline_hook::retry_failed_install_cleanup().map_err(|error| {
            format!(
                "XUI21 failed-install hook custody is still live; DLL unload is forbidden: {error:?}"
            )
        })?;
        if a729_inline_hook::failed_install_cleanup_pending() {
            return Err(
                "XUI21 failed-install hook custody remains pending after cleanup retry; DLL unload is forbidden"
                    .into(),
            );
        }
    }
    let runtime = slot.as_ref().cloned();
    drop(slot);
    let Some(runtime) = runtime else {
        return Ok(());
    };
    if runtime.hook_profile == RuntimeHookProfile::ProductionSemantic {
        return Err("XUI21 production semantic hooks require two-phase terminal custody".into());
    }
    if runtime.terminal_custody_state.load(Ordering::Acquire) == TERMINAL_CUSTODY_RESERVED {
        return Err(
            "XUI21 terminal custody is reserved; commit or abort it before uninstall".into(),
        );
    }
    runtime.accepting.store(false, Ordering::Release);
    let mut hooks = runtime
        .hooks
        .lock()
        .map_err(|_| "XUI21 hook-set mutex poisoned".to_string())?;
    if let Some(installed) = hooks.as_mut() {
        installed
            .try_uninstall()
            .map_err(|error| format!("XUI21 exact hook uninstall failed: {error:?}"))?;
    }
    *hooks = None;
    drop(hooks);
    let mut slot = runtime_slot()
        .lock()
        .map_err(|_| "XUI21 runtime slot poisoned".to_string())?;
    if slot
        .as_ref()
        .is_some_and(|active| Arc::ptr_eq(active, &runtime))
    {
        *slot = None;
    }
    Ok(())
}

fn wait_for_runtime_transactions(runtime: &Runtime, timeout: Duration) -> Result<(), String> {
    if timeout > MAX_TERMINAL_CUSTODY_WAIT {
        return Err("XUI21 terminal custody wait exceeds the 30-second hard bound".into());
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| "XUI21 terminal custody wait overflowed".to_string())?;
    loop {
        let in_flight = runtime.in_flight.load(Ordering::Acquire);
        if in_flight == 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "XUI21 terminal custody timed out with {in_flight} ledger transactions in flight"
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn exact_three_semantic_outcomes(
    runtime: &Runtime,
) -> Result<Vec<a729_inline_hook::InstalledExactRvaOutcome>, String> {
    if runtime.hook_profile != RuntimeHookProfile::ProductionSemantic {
        return Err("XUI21 terminal custody requires the production semantic profile".into());
    }
    let outcomes = runtime
        .hooks
        .lock()
        .map_err(|_| "XUI21 hook-set mutex poisoned".to_string())?
        .as_ref()
        .map(InstalledHookSet::installed_exact_rva_outcomes)
        .ok_or_else(|| "XUI21 terminal custody has no installed hook set".to_string())?;
    let expected = [
        a729_inline_hook::InstalledExactRvaOutcome {
            rva: 0x0A028A0,
            patch_len: 15,
        },
        a729_inline_hook::InstalledExactRvaOutcome {
            rva: 0x1227360,
            patch_len: 16,
        },
        a729_inline_hook::InstalledExactRvaOutcome {
            rva: 0x1527B30,
            patch_len: 17,
        },
    ];
    if outcomes.as_slice() != expected.as_slice() {
        return Err(
            "XUI21 terminal custody hook set is not the exact three-site semantic roster".into(),
        );
    }
    Ok(outcomes)
}

fn terminal_snapshot(
    runtime: &Runtime,
    capture_session_sha256: [u8; 32],
    custody_sequence: u64,
    custody_nonce: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<A729TerminalLedgerSnapshot, String> {
    if runtime.accepting.load(Ordering::Acquire)
        || runtime.in_flight.load(Ordering::Acquire) != 0
        || runtime.terminal_custody_state.load(Ordering::Acquire) != TERMINAL_CUSTODY_RESERVED
    {
        return Err("XUI21 terminal snapshot requires sealed exclusive custody".into());
    }
    let terminal_epoch = runtime.epoch.load(Ordering::Acquire);
    let coverage = certified_route_attestation()?;
    runtime.ledger.record_route_certification(
        coverage.all_event_class_routes_certified,
        coverage.player_ai_timeout_no_choice_routes_certified,
    );
    match runtime.bridge.fired_ledger_shadow_matches() {
        Ok(true) => {}
        Ok(false) => runtime.ledger.note_shadow_mismatch(),
        Err(error) => return Err(error),
    }
    let snapshot = runtime.ledger.snapshot_rows(frame_hour, elapsed_wall_ms)?;
    if runtime.in_flight.load(Ordering::Acquire) != 0
        || runtime.epoch.load(Ordering::Acquire) != terminal_epoch
    {
        return Err("XUI21 terminal ledger changed after quiescence".into());
    }
    let frame = FiredEventFrame {
        elapsed_wall_ms,
        frame_hour,
        producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
        fired_events_complete: snapshot.telemetry.zero_gap(),
        telemetry: snapshot.telemetry,
        capture_start_fired_event_ids: snapshot.capture_start_fired_event_ids,
        rows: snapshot.rows,
    };
    let payload = encode_payload(&frame)
        .map_err(|error| format!("XUI21 terminal payload encoding failed: {error:?}"))?;
    let payload_sha256: [u8; 32] = Sha256::digest(payload).into();
    let mut identity = Sha256::new();
    identity.update(b"battle-plan.d75_terminal_custody.v1\0");
    identity.update(capture_session_sha256);
    identity.update(custody_sequence.to_le_bytes());
    identity.update(custody_nonce);
    identity.update(terminal_epoch.to_le_bytes());
    identity.update(frame_hour.to_le_bytes());
    identity.update(elapsed_wall_ms.to_le_bytes());
    identity.update(payload_sha256);
    Ok(A729TerminalLedgerSnapshot {
        capture_session_sha256,
        custody_identity_sha256: identity.finalize().into(),
        terminal_epoch,
        frame,
        payload_sha256,
    })
}

/// could ever release D75's three semantic detours or D64's two: live run t73
/// work today while one is outstanding (`take_released_hook_receipts`,
/// transition). Every partial-install abort and zero-observation release would
/// Lock order is `runtime_slot` (released) then this runtime's authority slot
fn ensure_d64_terminal_detach_authority_bound() -> Result<(), String> {
    ensure_d64_terminal_detach_authority_bound_with(
        scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
            register_d75_terminal_detach_authority,
    )
}

fn ensure_d64_terminal_detach_authority_bound_with(
    mint: impl FnOnce() -> Result<
        scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::D64DetachAuthority,
        String,
    >,
) -> Result<(), String> {
    let runtime = {
        let slot = runtime_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slot.as_ref()
            .cloned()
            .ok_or_else(|| "XUI21 runtime is not initialized".to_string())?
    };
    if runtime
        .d64_detach_authority
        .lock()
        .map_err(|_| "XUI21 D64 detach-authority mutex poisoned".to_string())?
        .is_some()
    {
        return Ok(());
    }
    if runtime.hook_profile != RuntimeHookProfile::ProductionSemantic
        || !runtime.accepting.load(Ordering::Acquire)
        || runtime.terminal_custody_state.load(Ordering::Acquire) != TERMINAL_CUSTODY_ACTIVE
        || A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) == 0
    {
        return Err(
            "XUI21 D64 detach authority cannot bind outside active production custody".into(),
        );
    }
    let authority = mint().map_err(|error| {
        format!("XUI21 D64 detach authority was not issued for terminal sealing: {error}")
    })?;
    let mut bound = runtime
        .d64_detach_authority
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if bound.is_some() {
        drop(bound);
        let mut reason = Sha256::new();
        reason.update(b"battle-plan.d75.seal_mint_lost_race.v1\0");
        let reason_sha256: [u8; 32] = reason.finalize().into();
        let mut surplus = authority;
        let mut refusals = Vec::new();
        for _ in 0..2 {
            match scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
                cancel_unbound_d75_terminal_detach_authority(surplus, reason_sha256)
            {
                Ok(_) => {
                    return Err(
                        "XUI21 D64 detach authority was already bound when the seal minted one; \
                         the surplus authority was cancelled"
                            .to_string(),
                    )
                }
                Err(failure) => {
                    refusals.push(failure.message().to_string());
                    surplus = failure.into_authority();
                }
            }
        }
        return Err(format!(
            "XUI21 D64 detach authority was already bound when the seal minted one, and the \
             surplus authority could not be cancelled twice, so D64 custody is stranded: {}",
            refusals.join("; ")
        ));
    }
    *bound = Some(authority);
    Ok(())
}

fn preseal_failure(message: impl Into<String>) -> A729TerminalCustodySealFailure {
    A729TerminalCustodySealFailure {
        message: message.into(),
        recovery: None,
    }
}

fn reserved_seal_failure(
    runtime: Arc<Runtime>,
    capture_session_sha256: [u8; 32],
    custody_sequence: u64,
    custody_nonce: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    message: String,
) -> A729TerminalCustodySealFailure {
    let mut identity = Sha256::new();
    identity.update(b"battle-plan.d75_failed_terminal_custody.v1\0");
    identity.update(capture_session_sha256);
    identity.update(custody_sequence.to_le_bytes());
    identity.update(custody_nonce);
    identity.update(frame_hour.to_le_bytes());
    identity.update(elapsed_wall_ms.to_le_bytes());
    identity.update(Sha256::digest(message.as_bytes()));
    let custody_identity_sha256 = identity.finalize().into();
    A729TerminalCustodySealFailure {
        message: message.clone(),
        recovery: Some(A729FailedTerminalCustodyReservation {
            runtime: Some(runtime),
            capture_session_sha256,
            custody_identity_sha256,
            frame_hour,
            elapsed_wall_ms,
            failure: message,
            observer_detached: false,
            d64_poisoned_abort_envelope_sha256: None,
        }),
    }
}

pub fn seal_and_reserve_a729_terminal_custody(
    capture_session_sha256: [u8; 32],
    frame_hour: u64,
    elapsed_wall_ms: u64,
    wait_timeout: Duration,
) -> Result<A729TerminalCustodyReservation, A729TerminalCustodySealFailure> {
    if capture_session_sha256 == [0; 32] {
        return Err(preseal_failure(
            "XUI21 terminal custody capture-session digest is zero",
        ));
    }
    if frame_hour == 0 || i32::try_from(frame_hour).is_err() {
        return Err(preseal_failure(
            "XUI21 terminal custody frame hour is outside the shared D64/H4CR production range",
        ));
    }
    let custody_sequence = NEXT_TERMINAL_CUSTODY_SEQUENCE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .map_err(|_| preseal_failure("XUI21 terminal custody sequence exhausted"))?;
    let custody_nonce = terminal_custody_nonce().map_err(preseal_failure)?;
    // acquire the runtime slot on every D75 callback.
    if let Err(error) = ensure_d64_terminal_detach_authority_bound() {
        return Err(preseal_failure(error));
    }
    let slot = runtime_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime = slot
        .as_ref()
        .cloned()
        .ok_or_else(|| preseal_failure("XUI21 runtime is not initialized"))?;
    runtime
        .terminal_custody_state
        .compare_exchange(
            TERMINAL_CUSTODY_ACTIVE,
            TERMINAL_CUSTODY_RESERVED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| preseal_failure("XUI21 terminal custody is already reserved or released"))?;
    runtime.accepting.store(false, Ordering::Release);
    drop(slot);
    if let Err(error) = wait_for_runtime_transactions(&runtime, wait_timeout) {
        return Err(reserved_seal_failure(
            runtime,
            capture_session_sha256,
            custody_sequence,
            custody_nonce,
            frame_hour,
            elapsed_wall_ms,
            error,
        ));
    }
    let semantic_hook_outcomes = match exact_three_semantic_outcomes(&runtime) {
        Ok(outcomes) => outcomes,
        Err(error) => {
            return Err(reserved_seal_failure(
                runtime,
                capture_session_sha256,
                custody_sequence,
                custody_nonce,
                frame_hour,
                elapsed_wall_ms,
                error,
            ));
        }
    };
    let snapshot = match terminal_snapshot(
        &runtime,
        capture_session_sha256,
        custody_sequence,
        custody_nonce,
        frame_hour,
        elapsed_wall_ms,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(reserved_seal_failure(
                runtime,
                capture_session_sha256,
                custody_sequence,
                custody_nonce,
                frame_hour,
                elapsed_wall_ms,
                error,
            ));
        }
    };
    Ok(A729TerminalCustodyReservation {
        runtime: Some(runtime),
        snapshot,
        semantic_hook_outcomes,
        observer_detached_before_final_d64: false,
        d64_permit_issued: false,
        d64_terminal_lifecycle_envelope_sha256: None,
        d64_terminal_del2_sha256: None,
        d64_poisoned_abort_envelope_sha256: None,
        publication_permit_issued: false,
        publication_permit_nonce: None,
    })
}

fn uninstall_reserved_semantic_hooks(
    runtime: &Arc<Runtime>,
    wait_timeout: Duration,
) -> Result<Vec<a729_inline_hook::ReleasedProductionSemanticHookReceipt>, String> {
    if A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0 {
        return Err("XUI21 cannot release D75 hooks while the D64 observer is registered".into());
    }
    if wait_timeout > MAX_TERMINAL_CUSTODY_WAIT {
        return Err("XUI21 hook-release wait exceeds the 30-second hard bound".into());
    }
    {
        let slot = runtime_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !slot
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, runtime))
        {
            return Err("XUI21 active runtime changed during terminal custody".into());
        }
    }
    let deadline = Instant::now()
        .checked_add(wait_timeout)
        .ok_or_else(|| "XUI21 hook-release wait overflowed".to_string())?;
    let mut hooks = runtime
        .hooks
        .lock()
        .map_err(|_| "XUI21 hook-set mutex poisoned".to_string())?;
    let installed = hooks
        .as_mut()
        .ok_or_else(|| "XUI21 terminal custody lost its installed hook set".to_string())?;
    loop {
        match installed.try_uninstall() {
            Ok(()) => break,
            Err(
                a729_inline_hook::InstallError::CallbacksInFlight(_)
                | a729_inline_hook::InstallError::InstructionPointerInHook,
            ) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            Err(error) => {
                return Err(format!(
                    "XUI21 exact semantic hook release failed: {error:?}"
                ))
            }
        }
    }
    let released_receipts = installed.released_receipts().to_vec();
    let observed_release_roster = released_receipts
        .iter()
        .map(|receipt| (receipt.logical_name(), receipt.rva(), receipt.patch_len()))
        .collect::<Vec<_>>();
    let expected_release_roster = vec![
        ("gameplay.d75.common_fire", 0x0A028A0, 15),
        ("gameplay.d75.timeout_resolution", 0x1227360, 16),
        ("gameplay.d75.option_command", 0x1527B30, 17),
    ];
    let owner_token = released_receipts
        .first()
        .map(|receipt| receipt.teardown_owner_token())
        .unwrap_or(0);
    if released_receipts.len() != 3
        || observed_release_roster != expected_release_roster
        || owner_token == 0
        || released_receipts.iter().any(|receipt| {
            !receipt.custody_complete()
                || receipt.teardown_owner_token() != owner_token
                || receipt.original_prefix_sha256() == [0; 32]
                || receipt.destination_binding_sha256() == [0; 32]
        })
    {
        return Err("XUI21 exact custom release receipts are incomplete".into());
    }
    *hooks = None;
    drop(hooks);
    runtime
        .terminal_custody_state
        .store(TERMINAL_CUSTODY_RELEASED, Ordering::Release);
    let mut slot = runtime_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    debug_assert!(slot
        .as_ref()
        .is_some_and(|active| Arc::ptr_eq(active, &runtime)));
    *slot = None;
    Ok(released_receipts)
}

fn terminal_custody_receipt(
    reservation: &A729TerminalCustodyReservation,
    durable_acceptance: Option<&A729DurableTerminalFrameAcceptanceReceipt>,
    semantic_hook_release_receipts: Vec<a729_inline_hook::ReleasedProductionSemanticHookReceipt>,
) -> A729TerminalCustodyReceipt {
    let frame = &reservation.snapshot.frame;
    let complete = frame.fired_events_complete && frame.telemetry.zero_gap();
    let publication_committed = durable_acceptance.is_some();
    A729TerminalCustodyReceipt {
        capture_session_sha256: reservation.snapshot.capture_session_sha256,
        custody_identity_sha256: reservation.snapshot.custody_identity_sha256,
        terminal_epoch: reservation.snapshot.terminal_epoch,
        frame_hour: frame.frame_hour,
        row_count: frame.rows.len() as u64,
        terminal_payload_sha256: reservation.snapshot.payload_sha256,
        telemetry: frame.telemetry.clone(),
        fired_events_complete: complete,
        completeness_poisoned: !complete || !publication_committed,
        observer_detached_before_final_d64: reservation.observer_detached_before_final_d64,
        d64_final_drain_bound: reservation.d64_terminal_lifecycle_envelope_sha256.is_some(),
        d64_terminal_lifecycle_envelope_sha256: reservation.d64_terminal_lifecycle_envelope_sha256,
        d64_terminal_del2_sha256: reservation.d64_terminal_del2_sha256,
        d64_poisoned_abort_envelope_sha256: reservation.d64_poisoned_abort_envelope_sha256,
        terminal_h4cr_d75_sequence: durable_acceptance
            .map(A729DurableTerminalFrameAcceptanceReceipt::raw_d75_record_sequence),
        terminal_h4cr_d64_sequence: durable_acceptance
            .map(A729DurableTerminalFrameAcceptanceReceipt::raw_d64_record_sequence),
        terminal_h4cr_frame_end_sequence: durable_acceptance
            .map(A729DurableTerminalFrameAcceptanceReceipt::terminal_frame_end_sequence),
        authenticated_terminal_h4cr_frame_sha256: durable_acceptance.map(
            A729DurableTerminalFrameAcceptanceReceipt::authenticated_terminal_h4cr_frame_sha256,
        ),
        semantic_hook_outcomes: reservation.semantic_hook_outcomes.clone(),
        semantic_hook_release_receipts,
        semantic_hooks_uninstalled: true,
        publication_committed,
        custody_successful: complete
            && publication_committed
            && reservation.observer_detached_before_final_d64
            && reservation.d64_terminal_lifecycle_envelope_sha256.is_some()
            && reservation.d64_terminal_del2_sha256.is_some(),
    }
}

pub fn commit_a729_terminal_custody_and_uninstall(
    reservation: &mut A729TerminalCustodyReservation,
    durable_acceptance: &A729DurableTerminalFrameAcceptanceReceipt,
    wait_timeout: Duration,
) -> Result<A729TerminalCustodyReceipt, String> {
    if !reservation.publication_permit_issued {
        return Err("XUI21 terminal publication permit was not issued".into());
    }
    let permit_nonce = reservation
        .publication_permit_nonce
        .ok_or_else(|| "XUI21 terminal publication permit was not issued".to_string())?;
    let expected_authenticator = terminal_acceptance_authenticator(
        permit_nonce,
        reservation.snapshot.capture_session_sha256,
        reservation.snapshot.custody_identity_sha256,
        reservation
            .runtime
            .as_ref()
            .ok_or("XUI21 runtime disappeared before acceptance verification")?
            .authoritative_sink_identity_sha256,
        reservation.snapshot.payload_sha256,
        durable_acceptance.d64_terminal_lifecycle_envelope_sha256,
        durable_acceptance.d64_terminal_del2_sha256,
        durable_acceptance.terminal_frame_hour_raw,
        durable_acceptance.raw_d75_record_sequence,
        durable_acceptance.raw_d64_record_sequence,
        durable_acceptance.terminal_frame_end_sequence,
        durable_acceptance.authenticated_terminal_h4cr_frame_sha256,
    );
    if durable_acceptance.capture_session_sha256 != reservation.snapshot.capture_session_sha256
        || durable_acceptance.custody_identity_sha256
            != reservation.snapshot.custody_identity_sha256
        || reservation
            .runtime
            .as_ref()
            .map(|runtime| runtime.authoritative_sink_identity_sha256)
            != Some(durable_acceptance.authoritative_sink_identity_sha256)
        || durable_acceptance.terminal_payload_sha256 != reservation.snapshot.payload_sha256
        || Some(durable_acceptance.d64_terminal_lifecycle_envelope_sha256)
            != reservation.d64_terminal_lifecycle_envelope_sha256
        || Some(durable_acceptance.d64_terminal_del2_sha256) != reservation.d64_terminal_del2_sha256
        || i32::try_from(reservation.snapshot.frame.frame_hour).ok()
            != Some(durable_acceptance.terminal_frame_hour_raw)
        || durable_acceptance.acceptance_authenticator_sha256 != expected_authenticator
    {
        return Err(
            "XUI21 durable terminal H4CR acceptance does not match reserved custody".into(),
        );
    }
    if !reservation.snapshot.frame.fired_events_complete
        || !reservation.snapshot.frame.telemetry.zero_gap()
    {
        return Err(
            "XUI21 terminal D75 telemetry is completeness-poisoned; abort is required".into(),
        );
    }
    if !reservation.observer_detached_before_final_d64
        || A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0
    {
        return Err("XUI21 D75->D64 observer detachment was not confirmed before final D64".into());
    }
    if reservation.d64_terminal_lifecycle_envelope_sha256.is_none()
        || reservation.d64_terminal_del2_sha256.is_none()
    {
        return Err("XUI21 final D64 drain receipt was not bound after observer detachment".into());
    }
    let runtime = reservation
        .runtime
        .as_ref()
        .cloned()
        .ok_or_else(|| "XUI21 terminal custody was already released".to_string())?;
    let release_receipts = uninstall_reserved_semantic_hooks(&runtime, wait_timeout)?;
    reservation.runtime = None;
    reservation.publication_permit_nonce = None;
    Ok(terminal_custody_receipt(
        reservation,
        Some(durable_acceptance),
        release_receipts,
    ))
}

pub fn abort_a729_terminal_custody_and_uninstall(
    reservation: &mut A729TerminalCustodyReservation,
    wait_timeout: Duration,
) -> Result<A729TerminalCustodyReceipt, String> {
    if !reservation.observer_detached_before_final_d64
        || A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0
    {
        return Err("XUI21 abort requires caller-coordinated D75->D64 observer detachment".into());
    }
    if (reservation.d64_terminal_lifecycle_envelope_sha256.is_none()
        && reservation.d64_poisoned_abort_envelope_sha256.is_none())
        || (reservation.d64_terminal_lifecycle_envelope_sha256.is_some()
            && reservation.d64_terminal_del2_sha256.is_none())
    {
        return Err(
            "XUI21 abort requires authenticated D64 terminal drain or poisoned cleanup".into(),
        );
    }
    let runtime = reservation
        .runtime
        .as_ref()
        .cloned()
        .ok_or_else(|| "XUI21 terminal custody was already released".to_string())?;
    let release_receipts = uninstall_reserved_semantic_hooks(&runtime, wait_timeout)?;
    reservation.runtime = None;
    Ok(terminal_custody_receipt(
        reservation,
        None,
        release_receipts,
    ))
}

/// Proof that a partially installed production semantic roster released all
#[derive(Clone, Debug)]
pub struct A729PartialInstallAbortReceipt {
    pub discard: production_hook_receipt::InstallAbortDiscard,
    pub semantic_hooks_uninstalled: bool,
    pub semantic_hook_release_receipts:
        Vec<a729_inline_hook::ReleasedProductionSemanticHookReceipt>,
}

/// Releases the production semantic runtime of an install that never became a
/// ledger transactions, and releases through the same
/// release-receipt roster check. The only thing relaxed is the destination of
pub fn abort_partial_install_and_uninstall(
    wait_timeout: Duration,
) -> Result<A729PartialInstallAbortReceipt, String> {
    if A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0 {
        return Err(
            "XUI21 install abort requires caller-coordinated D75->D64 observer detachment".into(),
        );
    }
    let runtime = runtime_slot()
        .lock()
        .map_err(|_| "XUI21 runtime slot poisoned".to_string())?
        .as_ref()
        .cloned();
    let Some(runtime) = runtime else {
        return Err("XUI21 install abort has no installed runtime to release".into());
    };
    if runtime.hook_profile != RuntimeHookProfile::ProductionSemantic {
        return Err(
            "XUI21 install abort is only defined for the production semantic hook profile".into(),
        );
    }
    if runtime.terminal_custody_state.load(Ordering::Acquire) != TERMINAL_CUSTODY_ACTIVE {
        return Err(
            "XUI21 install abort refuses a reserved or released terminal custody; commit or abort \
             that reservation instead"
                .into(),
        );
    }
    runtime.accepting.store(false, Ordering::Release);
    wait_for_runtime_transactions(&runtime, wait_timeout)?;
    // The ledger is emptied before the fallible hook release below. If that
    // release fails, the rows are already gone while this returns `Err` and the
    // the three semantic hooks have actually been released.
    accumulate_install_abort_discard(runtime.ledger.discard_retained_rows());
    let semantic_hook_release_receipts = uninstall_reserved_semantic_hooks(&runtime, wait_timeout)?;
    Ok(A729PartialInstallAbortReceipt {
        discard: take_install_abort_discard(),
        semantic_hooks_uninstalled: true,
        semantic_hook_release_receipts,
    })
}

static INSTALL_ABORT_DISCARD: Mutex<production_hook_receipt::InstallAbortDiscard> =
    Mutex::new(production_hook_receipt::InstallAbortDiscard::ZERO);

fn accumulate_install_abort_discard(observed: production_hook_receipt::InstallAbortDiscard) {
    INSTALL_ABORT_DISCARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .merge(observed);
}

/// release completed, so a fresh runtime cannot inherit an earlier count.
fn take_install_abort_discard() -> production_hook_receipt::InstallAbortDiscard {
    std::mem::replace(
        &mut *INSTALL_ABORT_DISCARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        production_hook_receipt::InstallAbortDiscard::ZERO,
    )
}

/// Releases a runtime that failed after semantic admission had already
pub fn abort_failed_a729_terminal_custody_and_uninstall(
    recovery: &mut A729FailedTerminalCustodyReservation,
    wait_timeout: Duration,
) -> Result<A729FailedTerminalCustodyReceipt, String> {
    if !recovery.observer_detached
        || A19_DELAYED_EVENT_DISPATCH_OBSERVER.load(Ordering::Acquire) != 0
    {
        return Err(
            "XUI21 failed-custody abort requires caller-coordinated observer detachment".into(),
        );
    }
    let d64_poisoned_abort_envelope_sha256 =
        recovery.d64_poisoned_abort_envelope_sha256.ok_or_else(|| {
            "XUI21 failed-custody abort requires authenticated poisoned D64 cleanup".to_string()
        })?;
    let runtime = recovery
        .runtime
        .as_ref()
        .cloned()
        .ok_or_else(|| "XUI21 failed terminal custody was already released".to_string())?;
    let release_receipts = uninstall_reserved_semantic_hooks(&runtime, wait_timeout)?;
    recovery.runtime = None;
    Ok(A729FailedTerminalCustodyReceipt {
        capture_session_sha256: recovery.capture_session_sha256,
        custody_identity_sha256: recovery.custody_identity_sha256,
        frame_hour: recovery.frame_hour,
        d64_poisoned_abort_envelope_sha256,
        failure: recovery.failure.clone(),
        observer_detached: recovery.observer_detached,
        semantic_hooks_uninstalled: true,
        semantic_hook_release_receipts: release_receipts,
        completeness_poisoned: true,
        custody_successful: false,
    })
}

pub struct A729RuntimeSource;

impl crate::exact_source_seal::Sealed for A729RuntimeSource {}

impl ExactFiredEventSource for A729RuntimeSource {
    fn is_authenticated_a729_hook_source(&self) -> bool {
        runtime_installed()
    }

    fn producer_manifest_sha256(&self) -> [u8; 32] {
        PRODUCER_MANIFEST_SHA256
    }

    fn snapshot(
        &self,
        frame_hour: u64,
        elapsed_wall_ms: u64,
    ) -> Result<FiredEventSnapshot, String> {
        let runtime = runtime_slot()
            .lock()
            .map_err(|_| "XUI21 runtime slot poisoned".to_string())?
            .as_ref()
            .cloned()
            .ok_or_else(|| "XUI21 runtime is not initialized".to_string())?;
        if !runtime_installed() {
            return Err("XUI21 exact hook set is not installed".into());
        }
        if runtime.in_flight.load(Ordering::Acquire) != 0 {
            return Err("XUI21 snapshot overlaps an in-flight callback transaction".into());
        }
        let epoch = runtime.epoch.load(Ordering::Acquire);
        let coverage = certified_route_attestation()?;
        runtime.ledger.record_route_certification(
            coverage.all_event_class_routes_certified,
            coverage.player_ai_timeout_no_choice_routes_certified,
        );
        match runtime.bridge.fired_ledger_shadow_matches() {
            Ok(true) => {}
            Ok(false) => runtime.ledger.note_shadow_mismatch(),
            Err(error) => return Err(error),
        }
        let snapshot = runtime.ledger.snapshot_rows(frame_hour, elapsed_wall_ms)?;
        let mut coverage_receipt = runtime.bridge.coverage_receipt()?;
        if let Some(receipt) = coverage_receipt.as_mut() {
            receipt.hook_telemetry =
                Some(CoverageTelemetryWitness::from_hook_telemetry(&snapshot.telemetry));
        }
        if runtime.in_flight.load(Ordering::Acquire) != 0
            || runtime.epoch.load(Ordering::Acquire) != epoch
        {
            return Err("XUI21 callback transaction changed during snapshot".into());
        }
        if let Some(receipt) = coverage_receipt {
            persist_coverage_receipt(&receipt)?;
        }
        Ok(snapshot)
    }
}

fn persist_coverage_receipt(receipt: &CoverageReceipt) -> Result<Option<PathBuf>, String> {
    let Some(directory) = std::env::var_os("BATTLE_PLAN_XUI21_COVERAGE_RECEIPT_DIR").or_else(|| std::env::var_os("HOI4_XUI21_COVERAGE_RECEIPT_DIR")) else {
        return Ok(None);
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("XUI21 coverage receipt directory failed: {error}"))?;
    let mut bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("XUI21 coverage receipt serialization failed: {error}"))?;
    bytes.push(b'\n');
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let safe_session = receipt
        .session_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let path = directory.join(format!("xui21-{safe_session}-{digest}.json"));
    persist_create_new_or_equal(&path, &bytes)?;
    Ok(Some(path))
}

fn persist_create_new_or_equal(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| format!("XUI21 coverage receipt write failed: {error}"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read(path)
                .map_err(|read| format!("XUI21 coverage receipt reread failed: {read}"))?;
            if existing != bytes {
                return Err(format!(
                    "XUI21 coverage receipt path already contains different bytes: {}",
                    path.display()
                ));
            }
        }
        Err(error) => return Err(format!("XUI21 coverage receipt create failed: {error}")),
    }
    Ok(())
}

pub fn installed_source() -> Option<&'static A729RuntimeSource> {
    runtime_installed().then_some(&SOURCE)
}

pub fn installed_production_hook_outcomes(
) -> Result<Vec<a729_inline_hook::InstalledExactRvaOutcome>, String> {
    let runtime = runtime_slot()
        .lock()
        .map_err(|_| "XUI21 runtime slot poisoned".to_string())?
        .as_ref()
        .cloned()
        .ok_or_else(|| "XUI21 runtime is not initialized".to_string())?;
    if !runtime.accepting.load(Ordering::Acquire) {
        return Err("XUI21 runtime is not accepting callbacks".into());
    }
    if runtime.hook_profile != RuntimeHookProfile::ProductionSemantic {
        return Err("XUI21 active runtime is not the production semantic hook profile".into());
    }
    let outcomes = runtime
        .hooks
        .lock()
        .map_err(|_| "XUI21 hook-set mutex poisoned".to_string())?
        .as_ref()
        .map(InstalledHookSet::installed_exact_rva_outcomes)
        .ok_or_else(|| "XUI21 semantic hook set is not installed".to_string())?;
    Ok(outcomes)
}

/// teardown owner that must later mint the matching release receipts.
pub fn installed_production_hook_custody_receipts(
) -> Result<Vec<a729_inline_hook::InstalledProductionSemanticHookReceipt>, String> {
    let runtime = runtime_slot()
        .lock()
        .map_err(|_| "XUI21 runtime slot poisoned".to_string())?
        .as_ref()
        .cloned()
        .ok_or_else(|| "XUI21 runtime is not initialized".to_string())?;
    if !runtime.accepting.load(Ordering::Acquire)
        || runtime.hook_profile != RuntimeHookProfile::ProductionSemantic
    {
        return Err("XUI21 production hook custody is not active".into());
    }
    let receipts = runtime
        .hooks
        .lock()
        .map_err(|_| "XUI21 hook-set mutex poisoned".to_string())?
        .as_ref()
        .map(InstalledHookSet::installed_production_receipts)
        .ok_or_else(|| "XUI21 semantic hook set is not installed".to_string())?;
    if receipts.len() != 3 || receipts.iter().any(|receipt| !receipt.custody_active()) {
        return Err("XUI21 exact three-site install custody is incomplete".into());
    }
    Ok(receipts)
}

fn runtime_installed() -> bool {
    let runtime = runtime_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    let Some(runtime) = runtime else { return false };
    runtime.accepting.load(Ordering::Acquire)
        && runtime
            .hooks
            .lock()
            .ok()
            .is_some_and(|hooks| hooks.is_some())
}

fn production_semantic_callback_bindings() -> Vec<CallbackBinding> {
    vec![
        binding(
            SiteKind::Fire,
            hook_fire as *const c_void,
            Some(&ORIGINAL_FIRE),
        ),
        binding(
            SiteKind::OptionCommand,
            hook_option_command as *const c_void,
            Some(&ORIGINAL_COMMAND),
        ),
        binding(
            SiteKind::Timeout,
            hook_timeout as *const c_void,
            Some(&ORIGINAL_TIMEOUT),
        ),
    ]
}

#[cfg(test)]
fn full_evidence_callback_bindings_unshipped() -> Vec<CallbackBinding> {
    vec![
        binding(
            SiteKind::Fire,
            hook_fire as *const c_void,
            Some(&ORIGINAL_FIRE),
        ),
        binding(
            SiteKind::Materialise,
            hook_materialise as *const c_void,
            Some(&ORIGINAL_MATERIALISE),
        ),
        binding(
            SiteKind::SelectedDescription,
            hook_selected_description as *const c_void,
            Some(&ORIGINAL_SELECTED_DESCRIPTION),
        ),
        binding(
            SiteKind::PictureSelector,
            hook_picture_selector as *const c_void,
            Some(&ORIGINAL_PICTURE_SELECTOR),
        ),
        binding(
            SiteKind::SpriteLookup,
            hook_sprite_lookup as *const c_void,
            Some(&ORIGINAL_SPRITE_LOOKUP),
        ),
        binding(
            SiteKind::PlayerChoice,
            hook_player_choice as *const c_void,
            Some(&ORIGINAL_PLAYER),
        ),
        binding(
            SiteKind::Timeout,
            hook_timeout as *const c_void,
            Some(&ORIGINAL_TIMEOUT),
        ),
        binding(
            SiteKind::OptionCommand,
            hook_option_command as *const c_void,
            Some(&ORIGINAL_COMMAND),
        ),
        binding(
            SiteKind::OptionAdmission,
            hook_option_admission as *const c_void,
            None,
        ),
    ]
}

fn binding(
    kind: SiteKind,
    callback: *const c_void,
    original_slot: Option<&'static AtomicUsize>,
) -> CallbackBinding {
    CallbackBinding {
        kind,
        callback,
        original_slot,
    }
}

unsafe extern "system" fn hook_fire(
    dispatcher: usize,
    event: usize,
    actor_tag: *const u32,
    recipient_tag: *const u32,
    event_scope: usize,
    immediate: u8,
) {
    let callback = a729_inline_hook::CallbackGuard::enter();
    let transaction = begin_runtime_transaction(&callback);
    let runtime = transaction.as_ref().map(|txn| txn.runtime.as_ref());
    let raw = RawFire {
        dispatcher,
        event,
        actor_tag,
        recipient_tag,
        event_scope,
        immediate,
    };
    let mut event_sequence = None;
    if let Some(runtime) = runtime {
        match runtime.bridge.fire_input(raw) {
            Ok(Some(row)) => {
                notify_a19_delayed_event_dispatch(raw);
                event_sequence = Some(row.event_sequence);
                runtime.ledger.record_fire(row);
            }
            Ok(None) => notify_a19_delayed_event_dispatch(raw),
            Err(_) => {
                notify_a19_delayed_event_dispatch_unavailable();
                runtime.ledger.note_invalid_definition_join();
            }
        }
    }
    if runtime.is_some() {
        push_ai_resolution(dispatcher, raw, event_sequence);
    }
    if let Some(original) = original::<FireFn>(&ORIGINAL_FIRE) {
        unsafe {
            original(
                dispatcher,
                event,
                actor_tag,
                recipient_tag,
                event_scope,
                immediate,
            )
        };
    }
    if let (Some(runtime), Some(event_sequence)) = (runtime, event_sequence) {
        let command_instance_id = CONTEXT.with(|context| {
            context
                .borrow()
                .resolution_stack
                .last()
                .and_then(|frame| frame.observation.map(|value| value.instance_id))
        });
        if runtime
            .bridge
            .fire_committed(raw, event_sequence, command_instance_id)
            .is_err()
        {
            runtime.ledger.note_correlation_failure();
        }
    }
    if runtime.is_some() {
        commit_resolution(runtime);
    }
}

unsafe extern "system" fn hook_materialise(event_window: usize) {
    let callback = a729_inline_hook::CallbackGuard::enter();
    let transaction = begin_runtime_transaction(&callback);
    let runtime = transaction.as_ref().map(|txn| txn.runtime.as_ref());
    let event = unsafe { read_usize(event_window.wrapping_add(0x68)) }.unwrap_or(0);
    if runtime.is_some() {
        CONTEXT.with(|context| {
            context.borrow_mut().materialisation = Some(MaterialisationCapture {
                event_window,
                event,
                ..MaterialisationCapture::default()
            })
        });
    }
    if let Some(original) = original::<MaterialiseFn>(&ORIGINAL_MATERIALISE) {
        unsafe { original(event_window) };
    }
    let capture = runtime
        .is_some()
        .then(|| CONTEXT.with(|context| context.borrow_mut().materialisation.take()))
        .flatten();
    if let Some(runtime) = runtime {
        let Some(capture) = capture else {
            runtime.ledger.note_correlation_failure();
            return;
        };
        if capture.runtime_text_pin_failed {
            runtime.ledger.note_invalid_definition_join();
            return;
        }
        match runtime.bridge.materialisation_input(RawMaterialisation {
            event_window: capture.event_window,
            event: capture.event,
            admitted_option_pointers: capture.admitted_option_pointers,
            selected_desc_key: capture.selected_desc_key,
            shown_picture_gfx: capture.shown_picture_gfx,
            runtime_text_owners: capture.runtime_text_owners,
        }) {
            Ok(row) => runtime.ledger.record_materialisation(row),
            Err(_) => runtime.ledger.note_invalid_definition_join(),
        }
    }
}

unsafe extern "system" fn hook_selected_description(
    out: usize,
    candidates: usize,
    scope: usize,
    randomize: u8,
) -> usize {
    let callback_guard = a729_inline_hook::CallbackGuard::enter();
    let result = original::<SelectedDescriptionFn>(&ORIGINAL_SELECTED_DESCRIPTION)
        .map(|original| unsafe { original(out, candidates, scope, randomize) })
        .unwrap_or(0);
    if callback_guard.semantic_admission_open() {
        CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let Some(capture) = context.materialisation.as_mut() else {
                return;
            };
            if capture.event != 0 && candidates == capture.event.wrapping_add(0x3d0) {
                if let Ok(value) = unsafe { read_engine_string(out) } {
                    if !value.is_empty() {
                        capture.selected_desc_key = Some(value);
                    }
                }
            }
        });
    }
    result
}

unsafe extern "system" fn hook_picture_selector(
    event_window: usize,
    widget_owner: usize,
    scope: usize,
) {
    let callback_guard = a729_inline_hook::CallbackGuard::enter();
    let event = unsafe { read_usize(event_window.wrapping_add(0x68)) }.unwrap_or(0);
    let raw_picture = if event == 0 {
        None
    } else {
        unsafe { read_engine_string(event.wrapping_add(0x3e8)) }
            .ok()
            .filter(|value| !value.is_empty())
    };
    let armed = callback_guard.semantic_admission_open()
        && CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let active = context
                .materialisation
                .as_ref()
                .is_some_and(|capture| capture.event == event && event != 0);
            if active {
                context.picture_probe = raw_picture.map(|_| PictureProbe { selected: None });
            }
            active
        });
    if let Some(original) = original::<PictureSelectorFn>(&ORIGINAL_PICTURE_SELECTOR) {
        unsafe { original(event_window, widget_owner, scope) };
    }
    if armed {
        CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let selected = context
                .picture_probe
                .take()
                .and_then(|probe| probe.selected);
            if let (Some(capture), Some(selected)) = (context.materialisation.as_mut(), selected) {
                capture.shown_picture_gfx = Some(selected);
            }
        });
    }
}

unsafe extern "system" fn hook_sprite_lookup(table: usize, key: usize) -> usize {
    let callback_guard = a729_inline_hook::CallbackGuard::enter();
    let result = original::<SpriteLookupFn>(&ORIGINAL_SPRITE_LOOKUP)
        .map(|original| unsafe { original(table, key) })
        .unwrap_or(0);
    if result != 0 && callback_guard.semantic_admission_open() {
        if let Ok(candidate) = unsafe { read_engine_string(key) } {
            CONTEXT.with(|context| {
                let mut context = context.borrow_mut();
                let Some(probe) = context.picture_probe.as_mut() else {
                    return;
                };
                record_successful_picture_candidate(probe, candidate);
            });
        }
    }
    result
}

unsafe extern "system" fn hook_player_choice(event_window: usize, button_event: usize) {
    let callback = a729_inline_hook::CallbackGuard::enter();
    let transaction = begin_runtime_transaction(&callback);
    let runtime = transaction.as_ref().map(|txn| txn.runtime.as_ref());
    if runtime.is_some() {
        push_window_resolution(ResolutionPath::Player, event_window);
    }
    if let Some(original) = original::<PlayerChoiceFn>(&ORIGINAL_PLAYER) {
        unsafe { original(event_window, button_event) };
    }
    if runtime.is_some() {
        commit_resolution(runtime);
    }
}

unsafe extern "system" fn hook_timeout(owner: usize) -> u8 {
    let callback = a729_inline_hook::CallbackGuard::enter();
    let transaction = begin_runtime_transaction(&callback);
    let runtime = transaction.as_ref().map(|txn| txn.runtime.as_ref());
    if runtime.is_some() {
        push_window_resolution(ResolutionPath::Timeout, owner);
    }
    let result = original::<TimeoutFn>(&ORIGINAL_TIMEOUT)
        .map(|original| unsafe { original(owner) })
        .unwrap_or(0);
    if runtime.is_some() {
        commit_resolution(runtime);
    }
    result
}

unsafe extern "system" fn hook_option_command(
    out: usize,
    instance_id: u32,
    event: usize,
    recipient_tag: *const u32,
    event_scope: usize,
    option_ordinal: u32,
) -> usize {
    let callback_guard = a729_inline_hook::CallbackGuard::enter();
    let transaction = begin_runtime_transaction(&callback_guard);
    let runtime = transaction.as_ref().map(|txn| txn.runtime.as_ref());
    let result = original::<OptionCommandFn>(&ORIGINAL_COMMAND)
        .map(|original| unsafe {
            original(
                out,
                instance_id,
                event,
                recipient_tag,
                event_scope,
                option_ordinal,
            )
        })
        .unwrap_or(0);
    if result != 0 && runtime.is_some() {
        let recipient_tag_id = if recipient_tag.is_null() {
            0
        } else {
            unsafe { ptr::read(recipient_tag) }
        };
        let observation = CommandObservation {
            instance_id,
            event,
            recipient_tag_id,
            event_scope,
            option_ordinal,
        };
        let bracketed = CONTEXT.with(|context| {
            if let Some(frame) = context.borrow_mut().resolution_stack.last_mut() {
                frame.observation = Some(observation);
                true
            } else {
                false
            }
        });
        if !bracketed {
            if let Some(runtime) = runtime {
                match runtime.bridge.resolution_input(RawResolution {
                    path: ResolutionPath::Player,
                    owner: 0,
                    event_sequence_hint: None,
                    instance_id,
                    event,
                    recipient_tag_id,
                    event_scope,
                    option_ordinal,
                }) {
                    Ok(Some(row)) => runtime.ledger.record_resolution(row),
                    Ok(None) => {}
                    Err(_) => runtime.ledger.note_correlation_failure(),
                }
            }
        }
    }
    result
}

unsafe extern "system" fn hook_option_admission(option: usize, scope: usize, mode: u8) -> u8 {
    let callback_guard = a729_inline_hook::CallbackGuard::enter();
    if option == 0 {
        return 0;
    }
    let gated = unsafe { ptr::read((option + 0x1f5) as *const u8) } != 0;
    if gated && unsafe { ptr::read((option + 0x1f4) as *const u8) } != mode {
        return 0;
    }
    let predicate = option + 0x198;
    let vtable = unsafe { ptr::read(predicate as *const usize) };
    let callback = unsafe { ptr::read((vtable + 0x18) as *const usize) };
    if callback == 0 {
        return 0;
    }
    let callback: AdmissionFn = unsafe { mem::transmute(callback) };
    let admitted = unsafe { callback(predicate, scope, mode) };
    if admitted != 0 && callback_guard.semantic_admission_open() {
        CONTEXT.with(|context| {
            if let Some(capture) = context.borrow_mut().materialisation.as_mut() {
                capture.admitted_option_pointers.push(option);
            }
        });
    }
    admitted
}

fn push_ai_resolution(owner: usize, raw: RawFire, event_sequence_hint: Option<u64>) {
    let recipient_tag_id = if raw.recipient_tag.is_null() {
        0
    } else {
        unsafe { ptr::read(raw.recipient_tag) }
    };
    CONTEXT.with(|context| {
        context.borrow_mut().resolution_stack.push(ResolutionFrame {
            path: ResolutionPath::Ai,
            owner,
            event_sequence_hint,
            instance_id: 0,
            event: raw.event,
            recipient_tag_id,
            event_scope: raw.event_scope,
            observation: None,
        });
    });
}

fn push_window_resolution(path: ResolutionPath, owner: usize) {
    let instance_id = unsafe { read_u32(owner.wrapping_add(0x60)) }.unwrap_or(0);
    let event = unsafe { read_usize(owner.wrapping_add(0x68)) }.unwrap_or(0);
    let event_scope = owner.wrapping_add(0x78);
    let recipient_tag_id = unsafe { read_u32(event_scope.wrapping_add(0x08)) }.unwrap_or(0);
    CONTEXT.with(|context| {
        context.borrow_mut().resolution_stack.push(ResolutionFrame {
            path,
            owner,
            event_sequence_hint: None,
            instance_id,
            event,
            recipient_tag_id,
            event_scope,
            observation: None,
        });
    });
}

fn commit_resolution(runtime: Option<&Runtime>) {
    let frame = CONTEXT.with(|context| context.borrow_mut().resolution_stack.pop());
    let Some(frame) = frame else {
        if let Some(runtime) = runtime {
            runtime.ledger.note_correlation_failure();
        }
        return;
    };
    let ResolutionFrame {
        path,
        owner,
        event_sequence_hint,
        instance_id,
        event,
        recipient_tag_id,
        event_scope,
        observation,
    } = frame;
    let Some(observation) = observation else {
        if path == ResolutionPath::Timeout {
            if let Some(runtime) = runtime {
                match runtime.bridge.resolution_input(RawResolution {
                    path,
                    owner,
                    event_sequence_hint,
                    instance_id,
                    event,
                    recipient_tag_id,
                    event_scope,
                    option_ordinal: u32::MAX,
                }) {
                    Ok(Some(mut row)) => {
                        row.reason = ResolutionReason::Timeout;
                        row.chosen_option_ordinal = None;
                        runtime.ledger.record_resolution(row);
                    }
                    Ok(None) => {}
                    Err(_) => runtime.ledger.note_correlation_failure(),
                }
            }
        }
        return;
    };
    if let Some(runtime) = runtime {
        match runtime.bridge.resolution_input(RawResolution {
            path,
            owner,
            event_sequence_hint,
            instance_id: observation.instance_id,
            event: observation.event,
            recipient_tag_id: observation.recipient_tag_id,
            event_scope: observation.event_scope,
            option_ordinal: observation.option_ordinal,
        }) {
            Ok(Some(row)) => runtime.ledger.record_resolution(row),
            Ok(None) => {}
            Err(_) => runtime.ledger.note_correlation_failure(),
        }
    }
}

fn original<T: Copy>(slot: &AtomicUsize) -> Option<T> {
    let address = slot.load(Ordering::Acquire);
    (address != 0).then(|| unsafe { mem::transmute_copy(&address) })
}

unsafe fn read_usize(address: usize) -> Option<usize> {
    (address != 0).then(|| unsafe { ptr::read(address as *const usize) })
}

unsafe fn read_u32(address: usize) -> Option<u32> {
    (address != 0).then(|| unsafe { ptr::read(address as *const u32) })
}

unsafe fn read_engine_string(address: usize) -> Result<String, ()> {
    if address == 0 {
        return Err(());
    }
    let length = unsafe { ptr::read(address.wrapping_add(0x10) as *const usize) };
    let capacity = unsafe { ptr::read(address.wrapping_add(0x18) as *const usize) };
    if length > 4096 || capacity < length {
        return Err(());
    }
    let data = if capacity > 0x0f {
        unsafe { ptr::read(address as *const usize) }
    } else {
        address
    };
    if data == 0 {
        return Err(());
    }
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, length) };
    String::from_utf8(bytes.to_vec()).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PopupSubtype, ResolutionReason};

    type Dword = u32;
    type Bool = i32;
    const MEM_COMMIT: Dword = 0x1000;
    const MEM_RESERVE: Dword = 0x2000;
    const MEM_RELEASE: Dword = 0x8000;
    const PAGE_EXECUTE_READWRITE: Dword = 0x40;
    static TEST_RUNTIME_LOCK: Mutex<()> = Mutex::new(());

    #[link(name = "kernel32")]
    extern "system" {
        fn VirtualAlloc(
            address: *mut c_void,
            size: usize,
            allocation_type: Dword,
            protect: Dword,
        ) -> *mut c_void;
        fn VirtualFree(address: *mut c_void, size: usize, free_type: Dword) -> Bool;
    }

    unsafe extern "system" fn admit(predicate: usize, scope: usize, mode: u8) -> u8 {
        u8::from(predicate != 0 && scope == 0x1234 && mode == 7)
    }

    unsafe extern "system" fn delayed_dispatch_observer(_: A19DelayedEventDispatchObservation) {}

    fn bind_test_d64_authority() {
        bind_d64_terminal_detach_authority(
            scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
                mint_test_detach_authority(),
        )
        .unwrap();
    }

    fn append_test_h4cr_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&record_flags.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn test_terminal_perf_payload(envelope_sha256: [u8; 32], del2_sha256: [u8; 32]) -> Vec<u8> {
        let names = [
            "d64_terminal_envelope_sha256_0",
            "d64_terminal_envelope_sha256_1",
            "d64_terminal_envelope_sha256_2",
            "d64_terminal_envelope_sha256_3",
            "d64_terminal_del2_sha256_0",
            "d64_terminal_del2_sha256_1",
            "d64_terminal_del2_sha256_2",
            "d64_terminal_del2_sha256_3",
        ];
        let mut digest_bytes = [0u8; 64];
        digest_bytes[..32].copy_from_slice(&envelope_sha256);
        digest_bytes[32..].copy_from_slice(&del2_sha256);
        let mut payload = vec![0u8; 40];
        payload.extend_from_slice(b"PDU1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&(names.len() as u16).to_le_bytes());
        for (index, name) in names.into_iter().enumerate() {
            payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
            payload.extend_from_slice(&0u16.to_le_bytes());
            payload.extend_from_slice(
                &u64::from_le_bytes(
                    digest_bytes[index * 8..index * 8 + 8]
                        .try_into()
                        .expect("fixed digest unit"),
                )
                .to_le_bytes(),
            );
            payload.extend_from_slice(name.as_bytes());
        }
        payload
    }

    fn test_terminal_frame(
        game_hour_raw: i32,
        d75_payload: &[u8],
        envelope_sha256: [u8; 32],
        del2: &[u8],
    ) -> Vec<u8> {
        let del2_sha256: [u8; 32] = Sha256::digest(del2).into();
        let perf = test_terminal_perf_payload(envelope_sha256, del2_sha256);
        // ONE base row, so the DEL2 offset actually depends on the stride: a
        // zero-row header cannot tell 20 from 24.
        let mut d64 = vec![0u8; 20];
        d64[8..12].copy_from_slice(&1u32.to_le_bytes());
        d64[12..16].copy_from_slice(&1u32.to_le_bytes());
        d64.extend_from_slice(&[0u8; 24]);
        d64.extend_from_slice(del2);
        let mut frame = Vec::new();
        append_test_h4cr_record(
            &mut frame,
            10,
            game_hour_raw,
            crate::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf,
        );
        append_test_h4cr_record(
            &mut frame,
            11,
            game_hour_raw,
            crate::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            d75_payload,
        );
        append_test_h4cr_record(
            &mut frame,
            12,
            game_hour_raw,
            64,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d64,
        );
        append_test_h4cr_record(
            &mut frame,
            13,
            game_hour_raw,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        frame
    }

    struct ScriptedSink {
        accept_before_failing: usize,
        failure: std::io::ErrorKind,
        accepted: Vec<u8>,
        flush_failure: Option<std::io::ErrorKind>,
    }

    impl ScriptedSink {
        fn declining(failure: std::io::ErrorKind) -> Self {
            Self {
                accept_before_failing: 0,
                failure,
                accepted: Vec::new(),
                flush_failure: None,
            }
        }

        fn accepting_then_failing(prefix: usize, failure: std::io::ErrorKind) -> Self {
            Self {
                accept_before_failing: prefix,
                failure,
                accepted: Vec::new(),
                flush_failure: None,
            }
        }

        fn failing_only_on_flush(failure: std::io::ErrorKind) -> Self {
            Self {
                accept_before_failing: usize::MAX,
                failure: std::io::ErrorKind::Other,
                accepted: Vec::new(),
                flush_failure: Some(failure),
            }
        }
    }

    impl Write for ScriptedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let remaining = self
                .accept_before_failing
                .saturating_sub(self.accepted.len());
            if remaining == 0 {
                return Err(std::io::Error::new(self.failure, "scripted sink refusal"));
            }
            let take = remaining.min(buf.len());
            self.accepted.extend_from_slice(&buf[..take]);
            Ok(take)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            match self.flush_failure {
                Some(kind) => Err(std::io::Error::new(kind, "scripted flush refusal")),
                None => Ok(()),
            }
        }
    }

    fn terminal_publication_fixture() -> (A729TerminalPublicationPermit, Vec<u8>) {
        let d75_payload = b"reserved-d75-payload";
        let envelope_sha256 = [0x64; 32];
        let del2 = b"DEL2-authenticated-terminal-suffix";
        let del2_sha256: [u8; 32] = Sha256::digest(del2).into();
        let frame = test_terminal_frame(144, d75_payload, envelope_sha256, del2);
        let permit = A729TerminalPublicationPermit {
            capture_session_sha256: [0x51; 32],
            custody_identity_sha256: [0x52; 32],
            authoritative_sink_identity_sha256: [0x50; 32],
            terminal_payload_sha256: Sha256::digest(d75_payload).into(),
            d64_terminal_lifecycle_envelope_sha256: envelope_sha256,
            d64_terminal_del2_sha256: del2_sha256,
            terminal_frame_hour_raw: 144,
            authenticated_terminal_h4cr_frame_sha256: Sha256::digest(&frame).into(),
            raw_d75_record_sequence: 11,
            raw_d64_record_sequence: 12,
            terminal_frame_end_sequence: 13,
            permit_nonce: [0x53; 32],
        };
        (permit, frame)
    }

    fn publish_into(
        sink_writer: &mut ScriptedSink,
    ) -> Result<A729DurableTerminalFrameAcceptanceReceipt, A729TerminalPublicationFailure> {
        let (permit, frame) = terminal_publication_fixture();
        let mut sink = permit
            .bind_authoritative_sink(
                A729AuthoritativeSinkAuthority {
                    sink_identity_sha256: [0x50; 32],
                },
                sink_writer,
            )
            .unwrap();
        publish_terminal_h4cr_to_authoritative_sink(permit, &frame, &mut sink)
    }

    fn expect_refusal(
        outcome: Result<A729DurableTerminalFrameAcceptanceReceipt, A729TerminalPublicationFailure>,
        context: &str,
    ) -> A729TerminalPublicationFailure {
        match outcome {
            Ok(_) => panic!("{context}: the publication must have been refused"),
            Err(failure) => failure,
        }
    }

    #[test]
    fn a_sink_that_declined_the_write_returns_a_spendable_retry_permit() {
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidInput,
            std::io::ErrorKind::Unsupported,
            std::io::ErrorKind::NotFound,
        ] {
            let mut writer = ScriptedSink::declining(kind);
            let failure = expect_refusal(publish_into(&mut writer), "the sink declined the write");
            assert!(
                writer.accepted.is_empty(),
                "{kind:?}: the fixture must not have accepted a byte"
            );
            assert!(
                !failure.sink_state_indeterminate(),
                "{kind:?}: a declined request left the sink exactly as it was found"
            );
            assert!(
                failure.retry_permit_available(),
                "{kind:?}: the permit must be spendable, or the bounded retry budget \
                 has no attempt it may use"
            );
            failure
                .into_retry_permit()
                .map_err(|_| ())
                .expect("the permit must come back out");
        }
    }

    #[test]
    fn a_failure_after_the_first_byte_is_never_reported_as_determinate() {
        let mut writer =
            ScriptedSink::accepting_then_failing(4, std::io::ErrorKind::PermissionDenied);
        let failure = expect_refusal(publish_into(&mut writer), "the sink failed mid-frame");
        assert_eq!(writer.accepted.len(), 4, "the fixture must accept a prefix");
        assert!(
            failure.sink_state_indeterminate(),
            "a sink holding part of the frame is in an unknown state"
        );
        assert!(
            !failure.retry_permit_available(),
            "a permit spent against a sink holding a partial frame would publish twice"
        );

        let mut writer = ScriptedSink::failing_only_on_flush(std::io::ErrorKind::PermissionDenied);
        let failure = expect_refusal(publish_into(&mut writer), "the flush failed");
        assert!(
            failure.sink_state_indeterminate(),
            "an accepted-but-unflushed frame is not proof of anything"
        );
        assert!(!failure.retry_permit_available());
    }

    #[test]
    fn only_a_declined_request_is_a_front_door_refusal() {
        use std::io::ErrorKind;
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::InvalidInput,
            ErrorKind::Unsupported,
            ErrorKind::NotFound,
        ] {
            assert!(
                refusal_is_at_the_front_door(&std::io::Error::new(kind, "declined")),
                "{kind:?} declines the request without taking any of it"
            );
        }
        for kind in [
            ErrorKind::TimedOut,
            ErrorKind::BrokenPipe,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::NotConnected,
            ErrorKind::WriteZero,
            ErrorKind::Interrupted,
            ErrorKind::WouldBlock,
            ErrorKind::OutOfMemory,
            ErrorKind::Other,
        ] {
            assert!(
                !refusal_is_at_the_front_door(&std::io::Error::new(kind, "uncertain")),
                "{kind:?} does not prove the sink took none of the request"
            );
        }
    }

    #[test]
    fn an_endlessly_interrupted_write_is_abandoned_as_indeterminate() {
        let mut writer = ScriptedSink::declining(std::io::ErrorKind::Interrupted);
        let failure = expect_refusal(
            publish_into(&mut writer),
            "the sink never accepted the frame",
        );
        assert!(failure.sink_state_indeterminate());
        assert!(!failure.retry_permit_available());
    }

    #[test]
    fn a_sink_that_accepts_the_whole_frame_still_mints_the_receipt() {
        let (_, frame) = terminal_publication_fixture();
        let mut writer =
            ScriptedSink::accepting_then_failing(frame.len(), std::io::ErrorKind::Other);
        let acceptance = match publish_into(&mut writer) {
            Ok(acceptance) => acceptance,
            Err(failure) => panic!("a complete write is an acceptance: {failure:?}"),
        };
        assert_eq!(writer.accepted, frame);
        assert_eq!(acceptance.terminal_frame_hour_raw(), 144);
    }

    #[test]
    fn terminal_publication_binds_d64_del2_hour_and_opaque_sink_permit() {
        let d75_payload = b"reserved-d75-payload";
        let envelope_sha256 = [0x64; 32];
        let del2 = b"DEL2-authenticated-terminal-suffix";
        let del2_sha256: [u8; 32] = Sha256::digest(del2).into();
        let frame = test_terminal_frame(144, d75_payload, envelope_sha256, del2);
        let permit = A729TerminalPublicationPermit {
            capture_session_sha256: [0x51; 32],
            custody_identity_sha256: [0x52; 32],
            authoritative_sink_identity_sha256: [0x50; 32],
            terminal_payload_sha256: Sha256::digest(d75_payload).into(),
            d64_terminal_lifecycle_envelope_sha256: envelope_sha256,
            d64_terminal_del2_sha256: del2_sha256,
            terminal_frame_hour_raw: 144,
            authenticated_terminal_h4cr_frame_sha256: Sha256::digest(&frame).into(),
            raw_d75_record_sequence: 11,
            raw_d64_record_sequence: 12,
            terminal_frame_end_sequence: 13,
            permit_nonce: [0x53; 32],
        };
        let mut sink_bytes = Vec::new();
        let mut sink = permit
            .bind_authoritative_sink(
                A729AuthoritativeSinkAuthority {
                    sink_identity_sha256: [0x50; 32],
                },
                &mut sink_bytes,
            )
            .unwrap();
        let acceptance =
            publish_terminal_h4cr_to_authoritative_sink(permit, &frame, &mut sink).unwrap();
        drop(sink);
        assert_eq!(sink_bytes, frame);
        assert_eq!(acceptance.d64_terminal_del2_sha256(), del2_sha256);
        assert_eq!(acceptance.terminal_frame_hour_raw(), 144);

        let substituted_del2 = b"DEL2-substituted-terminal-suffix";
        let substituted = test_terminal_frame(144, d75_payload, envelope_sha256, substituted_del2);
        let substituted_permit = A729TerminalPublicationPermit {
            capture_session_sha256: [0x51; 32],
            custody_identity_sha256: [0x52; 32],
            authoritative_sink_identity_sha256: [0x50; 32],
            terminal_payload_sha256: Sha256::digest(d75_payload).into(),
            d64_terminal_lifecycle_envelope_sha256: envelope_sha256,
            d64_terminal_del2_sha256: del2_sha256,
            terminal_frame_hour_raw: 144,
            authenticated_terminal_h4cr_frame_sha256: Sha256::digest(&substituted).into(),
            raw_d75_record_sequence: 0,
            raw_d64_record_sequence: 0,
            terminal_frame_end_sequence: 0,
            permit_nonce: [0x54; 32],
        };
        assert!(authenticate_terminal_h4cr_frame(&substituted_permit, &substituted).is_err());

        let wrong_hour = test_terminal_frame(145, d75_payload, envelope_sha256, del2);
        let wrong_hour_permit = A729TerminalPublicationPermit {
            authenticated_terminal_h4cr_frame_sha256: Sha256::digest(&wrong_hour).into(),
            ..substituted_permit
        };
        assert!(authenticate_terminal_h4cr_frame(&wrong_hour_permit, &wrong_hour).is_err());
    }

    #[test]
    fn picture_join_keeps_first_successful_selector_key_without_name_inference() {
        let mut probe = PictureProbe { selected: None };
        let first = "GFX_report_event_GER_runtime_variant".to_string();
        let later = "authored_picture_token".to_string();
        record_successful_picture_candidate(&mut probe, first.clone());
        record_successful_picture_candidate(&mut probe, later);
        assert_eq!(probe.selected, Some(first));
    }

    #[test]
    fn branch_bearing_admission_leaf_is_reproduced_exactly() {
        let _admission = a729_inline_hook::open_test_semantic_admission();
        let mut option = Box::new([0usize; 80]);
        let mut vtable = Box::new([0usize; 4]);
        vtable[3] = admit as *const () as usize;
        let base = option.as_mut_ptr() as usize;
        unsafe {
            ptr::write((base + 0x198) as *mut usize, vtable.as_ptr() as usize);
            ptr::write((base + 0x1f5) as *mut u8, 1);
            ptr::write((base + 0x1f4) as *mut u8, 7);
        }
        CONTEXT.with(|context| {
            context.borrow_mut().materialisation = Some(MaterialisationCapture::default())
        });
        assert_eq!(unsafe { hook_option_admission(base, 0x1234, 6) }, 0);
        assert_eq!(unsafe { hook_option_admission(base, 0x1234, 7) }, 1);
        let capture = CONTEXT.with(|context| context.borrow_mut().materialisation.take().unwrap());
        assert_eq!(capture.admitted_option_pointers, vec![base]);
    }

    struct CompleteBridge;

    impl A729EventBridge for CompleteBridge {
        fn producer_manifest_sha256(&self) -> [u8; 32] {
            PRODUCER_MANIFEST_SHA256
        }
        fn capture_start_fired_baseline(&self) -> Result<CaptureStartFiredBaseline, String> {
            Ok(CaptureStartFiredBaseline::default())
        }
        fn pending_baseline(&self) -> Result<PendingBaseline, String> {
            Ok(PendingBaseline {
                complete: true,
                rows: vec![FireInput {
                    event_id: "germany.1".into(),
                    recipient_tag: "GER".into(),
                    fire_date: "1936.1.1.12".into(),
                    fire_hour: 0,
                    fire_tick: 12,
                    event_sequence: 1,
                    popup_subtype: PopupSubtype::Country,
                    effective_timeout_hours: 312,
                    timeout_tick: 324,
                }],
            })
        }
        fn fire_input(&self, _: RawFire) -> Result<Option<FireInput>, String> {
            Err("unused".into())
        }
        fn fire_committed(&self, _: RawFire, _: u64, _: Option<u32>) -> Result<(), String> {
            Ok(())
        }
        fn materialisation_input(
            &self,
            _: RawMaterialisation,
        ) -> Result<MaterialisationInput, String> {
            Ok(MaterialisationInput {
                event_sequence: 1,
                visible_option_ordinals: vec![0],
                selected_desc_key: Some("germany.1.d".into()),
                shown_picture_gfx: Some("GFX_report_event_german_reich".into()),
                popup_subtype: PopupSubtype::Country,
                timeout_tick: 324,
                runtime_title_text_context_id: None,
                runtime_description_text_context_id: None,
                runtime_option_text_contexts: Vec::new(),
            })
        }
        fn resolution_input(&self, _: RawResolution) -> Result<Option<ResolutionInput>, String> {
            Ok(Some(ResolutionInput {
                event_sequence: 1,
                chosen_option_ordinal: Some(0),
                resolved_tick: 13,
                reason: ResolutionReason::Selected,
            }))
        }
        fn fired_ledger_shadow_matches(&self) -> Result<bool, String> {
            Ok(true)
        }
    }

    fn exercise_terminal_h4fd_custody(poison_d75: bool) {
        use compact_contract::frame_degraded::{
            FrameDegradedDiagnosis, FrameDegradedDomain, FrameDegradedRecordV2,
        };
        let _test_runtime = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                0x0160_0000,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { a729_inline_hook::seed_exact_test_image(base) };
        install_a729_event_runtime(base as usize, Arc::new(CompleteBridge), 8).unwrap();
        let sink_authority = take_authoritative_terminal_sink_authority().unwrap();
        if poison_d75 {
            let runtime = runtime_slot().lock().unwrap().as_ref().unwrap().clone();
            runtime.ledger.note_shadow_mismatch();
        }
        register_a19_delayed_event_dispatch_observer(delayed_dispatch_observer).unwrap();
        bind_test_d64_authority();
        unregister_a19_delayed_event_dispatch_observer().unwrap();
        let mut custody =
            seal_and_reserve_a729_terminal_custody([0x61; 32], 1, 2, Duration::from_millis(250))
                .unwrap();
        assert_eq!(custody.snapshot().frame.fired_events_complete, !poison_d75);
        assert_eq!(custody.snapshot().frame.telemetry.zero_gap(), !poison_d75);
        assert_eq!(
            custody.snapshot().frame.telemetry.fired_ledger_shadow_mismatches,
            u64::from(poison_d75),
        );
        assert_eq!(custody.snapshot().frame.rows.len(), 1);
        let d64_receipt = custody
            .confirm_dispatch_observer_detached_before_final_d64()
            .unwrap()
            .run_final_d64_drain()
            .unwrap();
        custody.bind_final_d64_drain_receipt(&d64_receipt).unwrap();
        let terminal_payload = custody.snapshot().encode_payload().unwrap();
        let mut frame = test_terminal_frame(
            1,
            &terminal_payload,
            custody.d64_terminal_lifecycle_envelope_sha256.unwrap(),
            d64_receipt.terminal_del2(),
        );
        if poison_d75 {
            let status_offset = compact_contract::COMPACT_RECORD_HEADER_LEN + 32;
            frame[status_offset..status_offset + 4]
                .copy_from_slice(&compact_contract::COMPACT_STATUS_PARTIAL_DATA.to_le_bytes());
        }
        let end_len = compact_contract::COMPACT_RECORD_HEADER_LEN
            + compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN;
        let mut frame_end = frame.split_off(frame.len() - end_len);
        let mut next_sequence = 13u64;
        let mut d64_perf = vec![0u8; 40];
        for offset in [16usize, 20, 24, 28] {
            d64_perf[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
        }
        append_test_h4cr_record(
            &mut frame,
            next_sequence,
            1,
            64,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &d64_perf,
        );
        next_sequence += 1;
        if !poison_d75 {
            let mut degraded_perf = vec![0u8; 40];
            degraded_perf[32..36]
                .copy_from_slice(&compact_contract::COMPACT_STATUS_PARTIAL_DATA.to_le_bytes());
            append_test_h4cr_record(
                &mut frame,
                next_sequence,
                1,
                29,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                0,
                &degraded_perf,
            );
            next_sequence += 1;
        }
        let emitted_domains = if poison_d75 { 2u32 } else { 3u32 };
        frame_end[8..16].copy_from_slice(&(next_sequence + 1).to_le_bytes());
        let frame_end_payload = compact_contract::COMPACT_RECORD_HEADER_LEN;
        frame_end[frame_end_payload + 16..frame_end_payload + 20]
            .copy_from_slice(&emitted_domains.to_le_bytes());
        frame_end[frame_end_payload + 20..frame_end_payload + 24]
            .copy_from_slice(&emitted_domains.to_le_bytes());
        let reason = "terminal_fixture_loss=1";
        let notice = FrameDegradedRecordV2 {
            degraded_hour: 1,
            domains: vec![FrameDegradedDomain::from_status(
                if poison_d75 { crate::DOMAIN_ID } else { 29 },
                compact_contract::COMPACT_STATUS_PARTIAL_DATA,
                FrameDegradedDiagnosis::undeclared(
                    compact_contract::diagnostic_channel::Authority::SelfDomain,
                    reason,
                ),
            )
            .unwrap()],
        }
        .encode()
        .unwrap();
        append_test_h4cr_record(
            &mut frame,
            next_sequence,
            1,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_DEGRADED,
            0,
            &notice,
        );
        frame.extend_from_slice(&frame_end);
        let permit = custody.take_terminal_publication_permit(&frame).unwrap();
        let mut changed = frame.clone();
        let reason_start = changed
            .windows(reason.len())
            .position(|bytes| bytes == reason.as_bytes())
            .unwrap();
        changed[reason_start + reason.len() - 1] = b'2';
        let mut sink_bytes = Vec::new();
        let mut sink = permit
            .bind_authoritative_sink(sink_authority, &mut sink_bytes)
            .unwrap();
        let failure = expect_refusal(
            publish_terminal_h4cr_to_authoritative_sink(permit, &changed, &mut sink),
            "post-permit H4FD mutation",
        );
        assert!(failure.message().contains("pre-authorized frame"));
        assert!(!failure.sink_state_indeterminate());
        let permit = failure.into_retry_permit().unwrap();
        let acceptance =
            publish_terminal_h4cr_to_authoritative_sink(permit, &frame, &mut sink).unwrap();
        drop(sink);
        assert_eq!(sink_bytes, frame);
        assert_eq!(acceptance.terminal_frame_end_sequence(), next_sequence + 1);
        if poison_d75 {
            let error = commit_a729_terminal_custody_and_uninstall(
                &mut custody,
                &acceptance,
                Duration::from_millis(250),
            )
            .unwrap_err();
            assert_eq!(
                error,
                "XUI21 terminal D75 telemetry is completeness-poisoned; abort is required",
            );
            assert!(custody.runtime.is_some());
            assert!(custody.runtime.as_ref().unwrap().hooks.lock().unwrap().is_some());
            let receipt = abort_a729_terminal_custody_and_uninstall(
                &mut custody,
                Duration::from_millis(250),
            )
            .unwrap();
            assert!(!receipt.custody_successful);
            assert!(!receipt.publication_committed);
            assert!(receipt.completeness_poisoned);
            assert_eq!(receipt.row_count, 1);
            assert_eq!(receipt.semantic_hook_release_receipts.len(), 3);
        } else {
            let receipt = commit_a729_terminal_custody_and_uninstall(
                &mut custody,
                &acceptance,
                Duration::from_millis(250),
            )
            .unwrap();
            assert!(receipt.custody_successful);
            assert!(receipt.publication_committed);
            assert!(!receipt.completeness_poisoned);
            assert_eq!(receipt.row_count, 1);
            assert_eq!(receipt.semantic_hook_release_receipts.len(), 3);
        }
        assert!(installed_source().is_none());
        for site in a729_inline_hook::FULL_EVIDENCE_SITES {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected,
            );
        }
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    #[test]
    fn terminal_h4fd_preserves_complete_d75_custody_and_rejects_mutation() {
        exercise_terminal_h4fd_custody(false);
    }

    #[test]
    fn terminal_h4fd_does_not_authorize_poisoned_d75_custody_commit() {
        exercise_terminal_h4fd_custody(true);
    }

    #[test]
    fn complete_install_terminal_publication_and_three_site_release_are_atomic() {
        let _test_runtime = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { a729_inline_hook::seed_exact_test_image(base) };
        install_a729_event_runtime(base as usize, Arc::new(CompleteBridge), 8).unwrap();
        let sink_authority = take_authoritative_terminal_sink_authority().unwrap();
        let source = installed_source().expect("installed exact source");
        let payload = crate::collect_and_encode(source, 0, 1).unwrap();
        assert!(
            crate::decode_payload(&payload)
                .unwrap()
                .fired_events_complete
        );
        let outcomes = installed_production_hook_outcomes().unwrap();
        let installed_custody = installed_production_hook_custody_receipts().unwrap();
        assert_eq!(installed_custody.len(), 3);
        let install_owner = installed_custody[0].teardown_owner_token();
        assert!(install_owner != 0);
        assert!(installed_custody
            .iter()
            .all(|receipt| receipt.custody_active()
                && receipt.teardown_owner_token() == install_owner));
        assert_eq!(
            outcomes,
            vec![
                a729_inline_hook::InstalledExactRvaOutcome {
                    rva: 0x0A028A0,
                    patch_len: 15,
                },
                a729_inline_hook::InstalledExactRvaOutcome {
                    rva: 0x1227360,
                    patch_len: 16,
                },
                a729_inline_hook::InstalledExactRvaOutcome {
                    rva: 0x1527B30,
                    patch_len: 17,
                },
            ]
        );
        register_a19_delayed_event_dispatch_observer(delayed_dispatch_observer).unwrap();
        bind_test_d64_authority();
        let mut custody =
            seal_and_reserve_a729_terminal_custody([0x51; 32], 1, 2, Duration::from_millis(250))
                .unwrap();
        assert!(custody
            .confirm_dispatch_observer_detached_before_final_d64()
            .is_err());
        unregister_a19_delayed_event_dispatch_observer().unwrap();
        let d64_permit = custody
            .confirm_dispatch_observer_detached_before_final_d64()
            .unwrap();
        let d64_receipt = d64_permit.run_final_d64_drain().unwrap();
        custody.bind_final_d64_drain_receipt(&d64_receipt).unwrap();
        let terminal_payload = custody.snapshot().encode_payload().unwrap();
        let terminal_digest: [u8; 32] = Sha256::digest(&terminal_payload).into();
        assert_eq!(terminal_digest, custody.snapshot().payload_sha256);
        assert_eq!(custody.snapshot().frame.rows.len(), 1);
        let d64_digest = custody
            .d64_terminal_lifecycle_envelope_sha256
            .expect("test D64 digest bound");
        let d64_del2_digest = custody
            .d64_terminal_del2_sha256
            .expect("test D64 DEL2 digest bound");
        let terminal_frame = test_terminal_frame(
            1,
            &terminal_payload,
            d64_digest,
            d64_receipt.terminal_del2(),
        );
        let permit = custody
            .take_terminal_publication_permit(&terminal_frame)
            .unwrap();
        let mismatched_acceptance = A729DurableTerminalFrameAcceptanceReceipt {
            capture_session_sha256: custody.snapshot.capture_session_sha256,
            custody_identity_sha256: custody.snapshot.custody_identity_sha256,
            authoritative_sink_identity_sha256: permit.authoritative_sink_identity_sha256,
            terminal_payload_sha256: [0xA5; 32],
            d64_terminal_lifecycle_envelope_sha256: d64_digest,
            d64_terminal_del2_sha256: d64_del2_digest,
            terminal_frame_hour_raw: 1,
            raw_d75_record_sequence: 11,
            raw_d64_record_sequence: 12,
            terminal_frame_end_sequence: 13,
            authenticated_terminal_h4cr_frame_sha256: Sha256::digest(&terminal_frame).into(),
            acceptance_authenticator_sha256: terminal_acceptance_authenticator(
                permit.permit_nonce,
                custody.snapshot.capture_session_sha256,
                custody.snapshot.custody_identity_sha256,
                permit.authoritative_sink_identity_sha256,
                [0xA5; 32],
                d64_digest,
                d64_del2_digest,
                1,
                11,
                12,
                13,
                Sha256::digest(&terminal_frame).into(),
            ),
        };
        assert!(commit_a729_terminal_custody_and_uninstall(
            &mut custody,
            &mismatched_acceptance,
            Duration::from_millis(250),
        )
        .is_err());
        let mut sink_bytes = Vec::new();
        let mut sink = permit
            .bind_authoritative_sink(sink_authority, &mut sink_bytes)
            .unwrap();
        let matching_acceptance =
            publish_terminal_h4cr_to_authoritative_sink(permit, &terminal_frame, &mut sink)
                .unwrap();
        drop(sink);
        assert_eq!(sink_bytes, terminal_frame);
        let receipt = commit_a729_terminal_custody_and_uninstall(
            &mut custody,
            &matching_acceptance,
            Duration::from_millis(250),
        )
        .unwrap();
        assert!(receipt.custody_successful);
        assert!(receipt.publication_committed);
        assert!(receipt.semantic_hooks_uninstalled);
        assert_eq!(receipt.semantic_hook_release_receipts.len(), 3);
        let owner_token = receipt.semantic_hook_release_receipts[0].teardown_owner_token();
        assert!(owner_token != 0);
        assert_eq!(owner_token, install_owner);
        assert!(receipt
            .semantic_hook_release_receipts
            .iter()
            .all(|released| released.custody_complete()
                && released.teardown_owner_token() == owner_token));
        assert!(!receipt.completeness_poisoned);
        assert_eq!(receipt.row_count, 1);
        assert_eq!(receipt.semantic_hook_outcomes, outcomes);
        assert!(installed_source().is_none());
        for site in a729_inline_hook::FULL_EVIDENCE_SITES {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected
            );
        }
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    #[test]
    fn terminal_abort_releases_hooks_but_can_never_claim_success() {
        let _test_runtime = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { a729_inline_hook::seed_exact_test_image(base) };
        install_a729_event_runtime(base as usize, Arc::new(CompleteBridge), 8).unwrap();
        register_a19_delayed_event_dispatch_observer(delayed_dispatch_observer).unwrap();
        bind_test_d64_authority();
        unregister_a19_delayed_event_dispatch_observer().unwrap();
        let mut custody =
            seal_and_reserve_a729_terminal_custody([0x52; 32], 1, 2, Duration::from_millis(250))
                .unwrap();
        let d64_permit = custody
            .confirm_dispatch_observer_detached_before_final_d64()
            .unwrap();
        let d64_abort = d64_permit.abort_final_d64_custody([0xAB; 32]).unwrap();
        custody.bind_poisoned_d64_abort_receipt(&d64_abort).unwrap();
        let receipt =
            abort_a729_terminal_custody_and_uninstall(&mut custody, Duration::from_millis(250))
                .unwrap();
        assert!(!receipt.publication_committed);
        assert!(!receipt.custody_successful);
        assert!(receipt.completeness_poisoned);
        assert!(receipt.semantic_hooks_uninstalled);
        assert_eq!(receipt.semantic_hook_release_receipts.len(), 3);
        assert!(installed_source().is_none());
        for site in a729_inline_hook::FULL_EVIDENCE_SITES {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected
            );
        }
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    #[test]
    fn failed_seal_returns_poisoned_recovery_that_can_release_hooks() {
        let _test_runtime = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { a729_inline_hook::seed_exact_test_image(base) };
        install_a729_event_runtime(base as usize, Arc::new(CompleteBridge), 8).unwrap();
        register_a19_delayed_event_dispatch_observer(delayed_dispatch_observer).unwrap();
        bind_test_d64_authority();
        unregister_a19_delayed_event_dispatch_observer().unwrap();
        let runtime = runtime_slot().lock().unwrap().as_ref().unwrap().clone();
        runtime.in_flight.store(1, Ordering::Release);
        let failure = match seal_and_reserve_a729_terminal_custody([0x53; 32], 1, 2, Duration::ZERO)
        {
            Ok(_) => panic!("in-flight timeout must return failed custody"),
            Err(failure) => failure,
        };
        assert!(failure.message().contains("timed out"));
        assert!(!runtime.accepting.load(Ordering::Acquire));
        assert_eq!(
            runtime.terminal_custody_state.load(Ordering::Acquire),
            TERMINAL_CUSTODY_RESERVED
        );
        runtime.in_flight.store(0, Ordering::Release);
        let mut recovery = failure
            .into_recovery()
            .expect("post-seal failure carries recovery custody");
        recovery.confirm_dispatch_observer_detached().unwrap();
        let d64_abort_permit = recovery.take_d64_abort_permit().unwrap();
        let d64_abort = d64_abort_permit
            .abort_final_d64_custody([0xAC; 32])
            .unwrap();
        recovery
            .bind_poisoned_d64_abort_receipt(&d64_abort)
            .unwrap();
        let receipt = abort_failed_a729_terminal_custody_and_uninstall(
            &mut recovery,
            Duration::from_millis(250),
        )
        .unwrap();
        assert!(receipt.completeness_poisoned);
        assert!(!receipt.custody_successful);
        assert!(receipt.semantic_hooks_uninstalled);
        assert_eq!(receipt.semantic_hook_release_receipts.len(), 3);
        assert!(installed_source().is_none());
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    /// either the terminal path or the bounded recovery driver could release
    #[test]
    fn terminal_sealing_mints_the_d64_detach_authority_when_none_is_bound() {
        let _test_runtime = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { a729_inline_hook::seed_exact_test_image(base) };
        install_a729_event_runtime(base as usize, Arc::new(CompleteBridge), 8).unwrap();
        let _sink_authority = take_authoritative_terminal_sink_authority().unwrap();
        register_a19_delayed_event_dispatch_observer(delayed_dispatch_observer).unwrap();

        let failure = match seal_and_reserve_a729_terminal_custody([0x54; 32], 1, 2, Duration::ZERO)
        {
            Ok(_) => panic!("the seal cannot reserve custody D64 never authorised"),
            Err(failure) => failure,
        };
        assert!(
            failure
                .message()
                .contains("XUI21 D64 detach authority was not issued for terminal sealing"),
            "the seal must ASK D64 for an authority, not refuse for want of one: {}",
            failure.message()
        );
        assert!(
            failure
                .message()
                .contains("A19 D64 detach authority requires installed hooks"),
            "D64's own refusal must reach the caller verbatim: {}",
            failure.message()
        );
        assert!(
            !failure
                .message()
                .contains("was not bound before terminal sealing"),
            "the pre-mint refusal is exactly the defect this closes: {}",
            failure.message()
        );

        assert_eq!(
            runtime_slot()
                .lock()
                .unwrap()
                .as_ref()
                .expect("runtime survives a pre-seal refusal")
                .terminal_custody_state
                .load(Ordering::Acquire),
            TERMINAL_CUSTODY_ACTIVE
        );
        unregister_a19_delayed_event_dispatch_observer().unwrap();
        let abort = abort_partial_install_and_uninstall(Duration::from_millis(250)).unwrap();
        assert!(abort.semantic_hooks_uninstalled);
        assert!(installed_source().is_none());
        for site in a729_inline_hook::FULL_EVIDENCE_SITES {
            assert_eq!(
                unsafe { std::slice::from_raw_parts(base.add(site.rva), site.expected.len()) },
                site.expected,
                "{} must be byte-identical again",
                site.rva
            );
        }
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    #[test]
    fn a_minted_d64_authority_is_stored_and_the_terminal_confirmation_consumes_it() {
        let _test_runtime = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let size = 0x0160_0000usize;
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        assert!(!base.is_null());
        unsafe { a729_inline_hook::seed_exact_test_image(base) };
        install_a729_event_runtime(base as usize, Arc::new(CompleteBridge), 8).unwrap();
        let _sink_authority = take_authoritative_terminal_sink_authority().unwrap();
        register_a19_delayed_event_dispatch_observer(delayed_dispatch_observer).unwrap();

        ensure_d64_terminal_detach_authority_bound_with(|| {
            Ok(scripted_state_flags_variables_live_lane::a729_delayed_event_lifecycle::
                mint_test_detach_authority())
        })
        .expect("a minted authority binds to the active production runtime");

        ensure_d64_terminal_detach_authority_bound_with(|| {
            panic!("an already-bound session must not mint a second authority")
        })
        .expect("the bound slot short-circuits");

        let mut reservation =
            seal_and_reserve_a729_terminal_custody([0x55; 32], 1, 2, Duration::from_millis(250))
                .expect("the seal reserves once D64 has authorised the detach");
        unregister_a19_delayed_event_dispatch_observer().unwrap();
        let permit = reservation
            .confirm_dispatch_observer_detached_before_final_d64()
            .expect("the terminal confirmation consumes the stored authority");
        let d64_abort = permit.abort_final_d64_custody([0xAD; 32]).unwrap();
        reservation
            .bind_poisoned_d64_abort_receipt(&d64_abort)
            .unwrap();
        let receipt =
            abort_a729_terminal_custody_and_uninstall(&mut reservation, Duration::from_millis(250))
                .unwrap();
        assert!(receipt.semantic_hooks_uninstalled);
        assert!(installed_source().is_none());
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }
}
