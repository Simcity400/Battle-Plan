//! Exact-a729 process-local observers for the proven A39 refresh/bind roots.
//!
//! This module intentionally stops before production activation. The hooks
//! retain only short-lived process-local pointers for an authenticated decoder;
//! no pointer is representable in CPR1. The source remains unauthenticated
//! until the post-hook receipt and required mixed fixtures are admitted.

use crate::{
    validate_snapshot, AcePortraitObservation, CharacterPresentationSnapshot,
    ExactCharacterPresentationSource, TraitTreeObservation,
};
use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const TRAIT_REFRESH_RVA: u32 = 0x016B_DC20;
pub const ACE_REFRESH_RVA: u32 = 0x0201_E420;
pub const ACE_ROW_BIND_RVA: u32 = 0x0201_EDC0;

// Retained exact-a729 ABI/sentinel authorities:
// - trait refresh decompile d44c1228..., instructions ea9ad44f...;
// - ace refresh decompile cfc1c6ea..., instructions 6059b45a...;
// - ace bind decompile abb2ce24..., instructions ae58cf9c....
// The complete hashes and closed historical job identities are recorded in
// H_A39_PREREQUISITE_ENGINEERING_20260720.md.

/// Deliberately false until a hook receipt plus mixed land/navy and literal /
/// fallback ace fixtures are admitted. `install()` and the carrier source fail
/// closed while this is false; the harness-only install is explicit.
pub const PRODUCTION_ACTIVATION_AUTHORIZED: bool = false;

const TRAIT_REFRESH_SENTINEL: [u8; 18] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x30, 0x49,
    0x63, 0xf0,
];
const ACE_REFRESH_SENTINEL: [u8; 21] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
    0x24, 0x28, 0xf0, 0xff, 0xff,
];
const ACE_ROW_BIND_SENTINEL: [u8; 17] = [
    0x48, 0x8b, 0xc4, 0x4c, 0x89, 0x48, 0x20, 0x4c, 0x89, 0x40, 0x18, 0x55, 0x53, 0x56, 0x57, 0x41,
    0x54,
];
const MAX_ACE_BINDS_PER_REFRESH: usize = 4_096;
const TRAIT_SELECTED_LEADER_REFERENCE: usize = 0x78;
const ACE_OBJECT_ID: usize = 0x0c;

type TraitRefreshFn = unsafe extern "system" fn(usize, usize, i32);
type AceRefreshFn = unsafe extern "system" fn(usize) -> u64;
type AceRowBindFn = unsafe extern "system" fn(usize, usize, usize, usize, u32);

static ORIGINAL_TRAIT_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ACE_REFRESH: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ACE_ROW_BIND: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static HOOK_SET_PRESENT: AtomicBool = AtomicBool::new(false);
static LIFECYCLE_HEALTHY: AtomicBool = AtomicBool::new(false);
static TEARDOWN_ACTIVE: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static HEALTH_EPOCH: AtomicU64 = AtomicU64::new(1);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static FIXTURE_DECODER: OnceLock<Mutex<Option<Arc<dyn ExactA729BoundaryDecoder>>>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static ACTIVE_CALLBACKS: Cell<u8> = const { Cell::new(0) };
    static ACTIVE_ACE_BINDS: RefCell<Option<Vec<ProcessLocalAceBind>>> = const { RefCell::new(None) };
    static DECODER_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

const CALLBACK_TRAIT_REFRESH: u8 = 1;
const CALLBACK_ACE_REFRESH: u8 = 1 << 1;
const CALLBACK_ACE_BIND: u8 = 1 << 2;
const CALLBACK_QUIESCE_TIMEOUT: Duration = Duration::from_secs(30);

struct CallbackEpoch;

impl CallbackEpoch {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackEpoch {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn capture_gate_open() -> bool {
    INSTALLED.load(Ordering::Acquire)
        && LIFECYCLE_HEALTHY.load(Ordering::Acquire)
        && !TEARDOWN_ACTIVE.load(Ordering::Acquire)
}

fn capture_epoch_witness() -> Option<u64> {
    let epoch = HEALTH_EPOCH.load(Ordering::Acquire);
    (capture_gate_open() && HEALTH_EPOCH.load(Ordering::Acquire) == epoch).then_some(epoch)
}

fn capture_epoch_is_current(epoch: u64) -> bool {
    epoch_witness_matches(
        epoch,
        HEALTH_EPOCH.load(Ordering::Acquire),
        capture_gate_open(),
    )
}

fn epoch_witness_matches(entry_epoch: u64, current_epoch: u64, gate_open: bool) -> bool {
    gate_open && entry_epoch == current_epoch
}

fn same_decoder(
    existing: &Arc<dyn ExactA729BoundaryDecoder>,
    candidate: &Arc<dyn ExactA729BoundaryDecoder>,
) -> bool {
    Arc::ptr_eq(existing, candidate)
}

struct DecoderCallGuard;

impl DecoderCallGuard {
    fn enter() -> Result<Self, String> {
        let nested = DECODER_ACTIVE.with(|active| active.replace(true));
        if nested {
            return Err("A39 fixture decoder re-entered the adapter".into());
        }
        Ok(Self)
    }
}

impl Drop for DecoderCallGuard {
    fn drop(&mut self) {
        DECODER_ACTIVE.with(|active| active.set(false));
    }
}

#[derive(Default)]
struct HookState {
    latest_trait_component: Option<DecodedTraitComponent>,
    latest_ace_component: Option<DecodedAceComponent>,
    trait_refresh_count: u64,
    ace_refresh_count: u64,
    ace_bind_count: u64,
    rejected_observation_count: u64,
    /// Intentional fatal fixture-epoch poison. Any ambiguous callback requires
    /// uninstall/reinstall; a success from the other component cannot revive
    /// stale evidence.
    current_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodedTraitComponent {
    sequence: u64,
    rows: Vec<TraitTreeObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodedAceComponent {
    sequence: u64,
    rows: Vec<AcePortraitObservation>,
}

/// Short-lived input to the not-yet-promoted trait decoder. These addresses
/// are never serialized and must be consumed in the hooked process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalTraitRefresh {
    pub sequence: u64,
    pub controller_ptr: usize,
    /// Exact CReferenceObject pair copied from controller+0x78 after refresh.
    /// It is a process-local join key, not a portable unit-leader identity.
    pub selected_leader_reference_raw: u64,
    pub refresh_argument_raw: i32,
}

/// One exact row-bind call in post-sort callback order. The row and ace
/// pointers are process-local correlation evidence, not replay identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalAceBind {
    pub visible_ordinal: u32,
    /// Stable CAceEntry+0x0C identity copied while `ace_ptr` is live.
    pub ace_id: u32,
    pub row_ptr: usize,
    pub ace_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalAceRefresh {
    pub sequence: u64,
    pub controller_ptr: usize,
    pub binds: Vec<ProcessLocalAceBind>,
}

/// Future receipt-bound decoder seam. Implementations must immediately copy
/// stable owner/leader/trait/ace/portrait values and must never persist an
/// address from either process-local refresh record. A decoder may perform only
/// crash-safe read-only memory/registry joins; it must not call engine/UI code,
/// install hooks, or otherwise re-enter the observed callback graph.
pub trait ExactA729BoundaryDecoder: Send + Sync {
    fn decode_trait_refresh(
        &self,
        boundary: &ProcessLocalTraitRefresh,
    ) -> Result<Vec<TraitTreeObservation>, String>;

    fn decode_ace_refresh(
        &self,
        boundary: &ProcessLocalAceRefresh,
    ) -> Result<Vec<AcePortraitObservation>, String>;
}

/// Reserved d82 source. It is useful to the capture harness now, but cannot
/// authenticate as a production source while the promotion constant is false.
#[derive(Clone, Copy, Debug, Default)]
pub struct A729ObservedCharacterPresentationSource;

impl A729ObservedCharacterPresentationSource {
    pub const fn new() -> Self {
        Self
    }

    /// Harness-only candidate decode used to generate the missing mixed
    /// fixtures. This never changes the source authentication result.
    pub fn decode_fixture_candidate(&self) -> Result<CharacterPresentationSnapshot, String> {
        let (trait_component, ace_component) = drain_decoded_component_pair()?;
        let snapshot = CharacterPresentationSnapshot {
            source_sequence: trait_component.sequence.max(ace_component.sequence),
            trait_trees: trait_component.rows,
            ace_portraits: ace_component.rows,
        };
        validate_decoded_snapshot(&snapshot)?;
        Ok(snapshot)
    }
}

impl ExactCharacterPresentationSource for A729ObservedCharacterPresentationSource {
    fn is_exact_a729_character_presentation_source(&self) -> bool {
        // Promotion deliberately requires a code change that installs a
        // sealed receipt-bound decoder. Flipping one public boolean cannot
        // authenticate an arbitrary caller-supplied decoder.
        false
    }

    fn read_character_presentation(&self) -> Result<CharacterPresentationSnapshot, String> {
        if !PRODUCTION_ACTIVATION_AUTHORIZED {
            return Err(
                "A39 exact-a729 live source is reserved pending hook receipt and mixed fixtures"
                    .into(),
            );
        }
        Err("A39 production source requires a sealed admitted identity decoder".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub trait_refresh: usize,
    pub ace_refresh: usize,
    pub ace_row_bind: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub installed_for_fixture_capture: bool,
    pub lifecycle_healthy: bool,
    pub teardown_active: bool,
    pub in_flight_callbacks: usize,
    pub production_activation_authorized: bool,
    pub trait_refresh_count: u64,
    pub ace_refresh_count: u64,
    pub ace_bind_count: u64,
    pub rejected_observation_count: u64,
    pub has_complete_boundary_batch: bool,
    pub current_error: Option<String>,
}

/// Production entry point. It cannot install until the evidence gate changes.
pub fn install() -> Result<InstalledAddresses, String> {
    if !PRODUCTION_ACTIVATION_AUTHORIZED {
        return Err(
            "A39 production hook activation is not authorized; use the capture harness entry point"
                .into(),
        );
    }
    install_hook_set()
}

/// Explicit harness-only entry point for collecting the receipts and mixed
/// fixtures required to promote d82. Every root is sentinel-gated before any
/// detour is created, and partial installation is rolled back.
#[cfg(feature = "a729-fixture-hook")]
pub fn install_for_fixture_capture(
    decoder: Arc<dyn ExactA729BoundaryDecoder>,
) -> Result<InstalledAddresses, String> {
    let slot = FIXTURE_DECODER.get_or_init(|| Mutex::new(None));
    {
        let mut slot = slot
            .lock()
            .map_err(|_| "A39 fixture decoder mutex poisoned")?;
        if let Some(existing) = slot.as_ref() {
            if !same_decoder(existing, &decoder) {
                return Err("A39 a different fixture decoder is already registered".into());
            }
        } else {
            *slot = Some(decoder);
        }
    }
    let result = install_hook_set();
    if result.is_err() && !HOOK_SET_PRESENT.load(Ordering::Acquire) {
        if let Ok(mut slot) = slot.lock() {
            *slot = None;
        }
    }
    result
}

fn install_hook_set() -> Result<InstalledAddresses, String> {
    let _install_guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A39 install mutex poisoned")?;
    if INSTALLED.load(Ordering::Acquire)
        && LIFECYCLE_HEALTHY.load(Ordering::Acquire)
        && MODULE_BASE.load(Ordering::Acquire) != 0
        && originals_ready()
    {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    if HOOK_SET_PRESENT.load(Ordering::Acquire) {
        return Err(
            "A39 hook set is lifecycle-poisoned; complete uninstall before reinstalling".into(),
        );
    }
    if FIXTURE_DECODER
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|slot| slot.as_ref().cloned())
        .is_none()
    {
        return Err("A39 fixture decoder is not registered".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A39 GetModuleHandleW(NULL) returned null".into());
    }
    let resolved = addresses(base);
    for (address, sentinel, label) in [
        (
            resolved.trait_refresh,
            &TRAIT_REFRESH_SENTINEL[..],
            "trait refresh",
        ),
        (
            resolved.ace_refresh,
            &ACE_REFRESH_SENTINEL[..],
            "ace refresh",
        ),
        (
            resolved.ace_row_bind,
            &ACE_ROW_BIND_SENTINEL[..],
            "ace row bind",
        ),
    ] {
        verify_sentinel(address, sentinel, label)?;
    }
    let state = STATE.get_or_init(|| Mutex::new(HookState::default()));
    *state.lock().map_err(|_| "A39 hook state poisoned")? = HookState::default();
    MODULE_BASE.store(base, Ordering::Release);
    LIFECYCLE_HEALTHY.store(false, Ordering::Release);
    let specs = [
        (
            resolved.trait_refresh,
            hook_trait_refresh as *mut c_void,
            &ORIGINAL_TRAIT_REFRESH,
        ),
        (
            resolved.ace_refresh,
            hook_ace_refresh as *mut c_void,
            &ORIGINAL_ACE_REFRESH,
        ),
        (
            resolved.ace_row_bind,
            hook_ace_row_bind as *mut c_void,
            &ORIGINAL_ACE_ROW_BIND,
        ),
    ];
    HOOK_SET_PRESENT.store(true, Ordering::Release);
    for (target, detour, original) in specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(value) => {
                original.store(value as usize, Ordering::Release);
            }
            Err(status) => {
                TEARDOWN_ACTIVE.store(true, Ordering::Release);
                HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
                let rollback = disable_and_remove(resolved);
                if rollback.all_removed && IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0 {
                    clear_removed_hook_set();
                }
                return Err(match rollback.error {
                    Some(rollback_error) => format!(
                        "A39 {target:#x} hook creation failed: {status}; rollback anomaly: {rollback_error}"
                    ),
                    None => format!("A39 {target:#x} hook creation failed: {status}"),
                });
            }
        }
    }
    for target in targets(resolved) {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            TEARDOWN_ACTIVE.store(true, Ordering::Release);
            HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
            let rollback = disable_and_remove(resolved);
            if rollback.all_removed && IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0 {
                clear_removed_hook_set();
            }
            return Err(match rollback.error {
                Some(rollback_error) => format!(
                    "A39 {target:#x} hook enable failed: {status}; rollback anomaly: {rollback_error}"
                ),
                None => format!("A39 {target:#x} hook enable failed: {status}"),
            });
        }
    }
    *state.lock().map_err(|_| "A39 hook state poisoned")? = HookState::default();
    HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
    INSTALLED.store(true, Ordering::Release);
    LIFECYCLE_HEALTHY.store(true, Ordering::Release);
    Ok(resolved)
}

pub fn uninstall() -> Result<(), String> {
    let _install_guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "A39 install mutex poisoned")?;
    if !HOOK_SET_PRESENT.load(Ordering::Acquire) {
        return Ok(());
    }
    let resolved = addresses(MODULE_BASE.load(Ordering::Acquire));
    TEARDOWN_ACTIVE.store(true, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    LIFECYCLE_HEALTHY.store(false, Ordering::Release);
    HEALTH_EPOCH.fetch_add(1, Ordering::AcqRel);
    let removal = disable_and_remove(resolved);
    let in_flight = IN_FLIGHT_CALLBACKS.load(Ordering::Acquire);
    if removal.all_removed && in_flight == 0 {
        clear_removed_hook_set();
        if let Some(state) = STATE.get() {
            *state.lock().map_err(|_| "A39 hook state poisoned")? = HookState::default();
        }
    }
    if let Some(error) = removal.error {
        // Keep the trampolines and module base intact: a failed disable/remove
        // may have left a detour callable unless every remove succeeded.
        return Err(error);
    }
    if in_flight != 0 {
        return Err(format!(
            "A39 teardown has {in_flight} in-flight callback(s); retry uninstall after quiescence"
        ));
    }
    if !removal.all_removed {
        return Err("A39 hook removal was incomplete without a MinHook status".into());
    }
    Ok(())
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics {
            production_activation_authorized: PRODUCTION_ACTIVATION_AUTHORIZED,
            lifecycle_healthy: LIFECYCLE_HEALTHY.load(Ordering::Acquire),
            teardown_active: TEARDOWN_ACTIVE.load(Ordering::Acquire),
            in_flight_callbacks: IN_FLIGHT_CALLBACKS.load(Ordering::Acquire),
            ..HookDiagnostics::default()
        };
    };
    HookDiagnostics {
        installed_for_fixture_capture: INSTALLED.load(Ordering::Acquire)
            && LIFECYCLE_HEALTHY.load(Ordering::Acquire),
        lifecycle_healthy: LIFECYCLE_HEALTHY.load(Ordering::Acquire),
        teardown_active: TEARDOWN_ACTIVE.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT_CALLBACKS.load(Ordering::Acquire),
        production_activation_authorized: PRODUCTION_ACTIVATION_AUTHORIZED,
        trait_refresh_count: state.trait_refresh_count,
        ace_refresh_count: state.ace_refresh_count,
        ace_bind_count: state.ace_bind_count,
        rejected_observation_count: state.rejected_observation_count,
        has_complete_boundary_batch: INSTALLED.load(Ordering::Acquire)
            && LIFECYCLE_HEALTHY.load(Ordering::Acquire)
            && state.latest_trait_component.is_some()
            && state.latest_ace_component.is_some()
            && state.current_error.is_none(),
        current_error: state.current_error.clone(),
    }
}

/// Consumes one already pointer-free trait fixture component.
pub fn drain_decoded_trait_candidate() -> Result<(u64, Vec<TraitTreeObservation>), String> {
    if !INSTALLED.load(Ordering::Acquire) || !LIFECYCLE_HEALTHY.load(Ordering::Acquire) {
        return Err("A39 exact-a729 fixture hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("A39 hook state is unavailable")?
        .lock()
        .map_err(|_| "A39 hook state poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    state
        .latest_trait_component
        .take()
        .map(|component| (component.sequence, component.rows))
        .ok_or_else(|| "A39 has not decoded a complete trait refresh".into())
}

/// Consumes one already pointer-free ace fixture component.
pub fn drain_decoded_ace_candidate() -> Result<(u64, Vec<AcePortraitObservation>), String> {
    if !INSTALLED.load(Ordering::Acquire) || !LIFECYCLE_HEALTHY.load(Ordering::Acquire) {
        return Err("A39 exact-a729 fixture hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("A39 hook state is unavailable")?
        .lock()
        .map_err(|_| "A39 hook state poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    state
        .latest_ace_component
        .take()
        .map(|component| (component.sequence, component.rows))
        .ok_or_else(|| "A39 has not decoded a complete ace refresh".into())
}

fn drain_decoded_component_pair() -> Result<(DecodedTraitComponent, DecodedAceComponent), String> {
    if !INSTALLED.load(Ordering::Acquire) || !LIFECYCLE_HEALTHY.load(Ordering::Acquire) {
        return Err("A39 exact-a729 fixture hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("A39 hook state is unavailable")?
        .lock()
        .map_err(|_| "A39 hook state poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    if state.latest_trait_component.is_none() || state.latest_ace_component.is_none() {
        return Err("A39 decoded trait/ace component pair is incomplete".into());
    }
    Ok((
        state.latest_trait_component.take().unwrap(),
        state.latest_ace_component.take().unwrap(),
    ))
}

unsafe extern "system" fn hook_trait_refresh(
    controller: usize,
    refresh_argument_ptr: usize,
    refresh_argument_raw: i32,
) {
    let _epoch = CallbackEpoch::enter();
    let admitted_epoch = capture_epoch_witness();
    let prior_callbacks = enter_callback(CALLBACK_TRAIT_REFRESH);
    let nested = prior_callbacks & CALLBACK_TRAIT_REFRESH != 0;
    if nested {
        reject_callback("A39 recursive trait refresh rejected");
    }
    let original = ORIGINAL_TRAIT_REFRESH.load(Ordering::Acquire);
    if original != 0 {
        let original: TraitRefreshFn = unsafe { mem::transmute(original) };
        unsafe { original(controller, refresh_argument_ptr, refresh_argument_raw) };
    } else {
        reject_callback("A39 trait refresh trampoline unavailable");
    }
    if admitted_epoch.is_some_and(capture_epoch_is_current) && !nested && original != 0 {
        record_trait_refresh(controller, refresh_argument_raw);
    }
    leave_callback(prior_callbacks);
}

unsafe extern "system" fn hook_ace_refresh(controller: usize) -> u64 {
    let _epoch = CallbackEpoch::enter();
    let admitted_epoch = capture_epoch_witness();
    let prior_callbacks = enter_callback(CALLBACK_ACE_REFRESH);
    let nested = prior_callbacks & CALLBACK_ACE_REFRESH != 0;
    if nested {
        reject_callback("A39 recursive ace refresh rejected");
    }
    let capture = admitted_epoch.is_some() && !nested && begin_ace_refresh();
    let original = ORIGINAL_ACE_REFRESH.load(Ordering::Acquire);
    let result = if original == 0 {
        reject_callback("A39 ace refresh trampoline unavailable");
        0
    } else {
        let original: AceRefreshFn = unsafe { mem::transmute(original) };
        unsafe { original(controller) }
    };
    if capture && admitted_epoch.is_some_and(capture_epoch_is_current) {
        finish_ace_refresh(controller);
    } else if capture {
        cancel_ace_refresh();
    }
    leave_callback(prior_callbacks);
    result
}

unsafe extern "system" fn hook_ace_row_bind(
    row_ptr: usize,
    ace_ptr: usize,
    argument_3: usize,
    argument_4: usize,
    argument_5_raw: u32,
) {
    let _epoch = CallbackEpoch::enter();
    let admitted_epoch = capture_epoch_witness();
    let prior_callbacks = enter_callback(CALLBACK_ACE_BIND);
    let nested = prior_callbacks & CALLBACK_ACE_BIND != 0;
    if nested {
        reject_callback("A39 recursive ace row bind rejected");
    }
    let original = ORIGINAL_ACE_ROW_BIND.load(Ordering::Acquire);
    if original != 0 {
        let original: AceRowBindFn = unsafe { mem::transmute(original) };
        unsafe { original(row_ptr, ace_ptr, argument_3, argument_4, argument_5_raw) };
    } else {
        reject_callback("A39 ace row-bind trampoline unavailable");
    }
    if admitted_epoch.is_some_and(capture_epoch_is_current) && !nested && original != 0 {
        record_ace_bind(row_ptr, ace_ptr);
    }
    leave_callback(prior_callbacks);
}

fn record_trait_refresh(controller_ptr: usize, refresh_argument_raw: i32) {
    if controller_ptr == 0 {
        reject_callback("A39 trait refresh supplied null controller");
        return;
    }
    let Some(selected_leader_reference_raw) = controller_ptr
        .checked_add(TRAIT_SELECTED_LEADER_REFERENCE)
        .and_then(read_u64)
    else {
        reject_callback("A39 selected leader reference was unreadable after trait refresh");
        return;
    };
    let boundary = ProcessLocalTraitRefresh {
        sequence: next_sequence(),
        controller_ptr,
        selected_leader_reference_raw,
        refresh_argument_raw,
    };
    let decoded = decode_trait_synchronously(&boundary);
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.trait_refresh_count = state.trait_refresh_count.saturating_add(1);
    match decoded {
        Ok(component) if state.current_error.is_none() => {
            state.latest_trait_component = Some(component);
        }
        Ok(_) => {}
        Err(error) => reject(&mut state, &error),
    }
}

fn begin_ace_refresh() -> bool {
    let started = ACTIVE_ACE_BINDS.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        if scratch.is_some() {
            false
        } else {
            *scratch = Some(Vec::new());
            true
        }
    });
    if !started {
        reject_callback("A39 overlapping ace refresh rejected");
    }
    started
}

fn cancel_ace_refresh() {
    ACTIVE_ACE_BINDS.with(|scratch| *scratch.borrow_mut() = None);
}

fn record_ace_bind(row_ptr: usize, ace_ptr: usize) {
    if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
        state.ace_bind_count = state.ace_bind_count.saturating_add(1);
    }
    if !ACTIVE_ACE_BINDS.with(|scratch| scratch.borrow().is_some()) {
        // A foreign-thread or out-of-refresh bind cannot enter another
        // thread's post-sort ordinal set and cannot poison its batch.
        return;
    }
    if row_ptr == 0 || ace_ptr == 0 {
        reject_callback("A39 ace bind supplied null row or ace");
        return;
    }
    let Some(ace_id) = ace_ptr.checked_add(ACE_OBJECT_ID).and_then(read_u32) else {
        reject_callback("A39 ace id was unreadable after row bind");
        return;
    };
    if ace_id == 0 || ace_id > i32::MAX as u32 {
        reject_callback("A39 ace row bind carried invalid ace id");
        return;
    }
    let result = ACTIVE_ACE_BINDS.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let Some(binds) = scratch.as_mut() else {
            return Ok(());
        };
        if binds.len() >= MAX_ACE_BINDS_PER_REFRESH {
            return Err("A39 ace bind count exceeded fixture bound");
        }
        let visible_ordinal = binds.len() as u32;
        binds.push(ProcessLocalAceBind {
            visible_ordinal,
            ace_id,
            row_ptr,
            ace_ptr,
        });
        Ok(())
    });
    if let Err(error) = result {
        reject_callback(error);
    }
}

fn finish_ace_refresh(controller_ptr: usize) {
    let Some(binds) = ACTIVE_ACE_BINDS.with(|scratch| scratch.borrow_mut().take()) else {
        return;
    };
    if controller_ptr == 0 {
        reject_callback("A39 ace refresh supplied null controller");
        return;
    }
    let boundary = ProcessLocalAceRefresh {
        sequence: next_sequence(),
        controller_ptr,
        binds,
    };
    let decoded = decode_ace_synchronously(&boundary);
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    state.ace_refresh_count = state.ace_refresh_count.saturating_add(1);
    match decoded {
        Ok(component) if state.current_error.is_none() => {
            state.latest_ace_component = Some(component);
        }
        Ok(_) => {}
        Err(error) => reject(&mut state, &error),
    }
}

fn reject(state: &mut HookState, message: &str) {
    state.rejected_observation_count = state.rejected_observation_count.saturating_add(1);
    state.current_error = Some(message.into());
    state.latest_trait_component = None;
    state.latest_ace_component = None;
    ACTIVE_ACE_BINDS.with(|scratch| *scratch.borrow_mut() = None);
}

fn reject_callback(message: &str) {
    let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return;
    };
    reject(&mut state, message);
}

fn next_sequence() -> u64 {
    NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed).max(1)
}

fn enter_callback(bit: u8) -> u8 {
    ACTIVE_CALLBACKS.with(|guard| {
        let prior = guard.get();
        guard.set(prior | bit);
        prior
    })
}

fn leave_callback(prior: u8) {
    ACTIVE_CALLBACKS.with(|guard| guard.set(prior));
}

fn decoder() -> Result<Arc<dyn ExactA729BoundaryDecoder>, String> {
    FIXTURE_DECODER
        .get()
        .ok_or("A39 fixture decoder is unavailable")?
        .lock()
        .map_err(|_| "A39 fixture decoder mutex poisoned")?
        .as_ref()
        .cloned()
        .ok_or_else(|| "A39 fixture decoder is unavailable".into())
}

fn decode_trait_synchronously(
    boundary: &ProcessLocalTraitRefresh,
) -> Result<DecodedTraitComponent, String> {
    let _decoder_guard = DecoderCallGuard::enter()?;
    let decoder = decoder()?;
    let rows = catch_unwind(AssertUnwindSafe(|| decoder.decode_trait_refresh(boundary)))
        .map_err(|_| "A39 trait fixture decoder panicked")??;
    let candidate = CharacterPresentationSnapshot {
        source_sequence: boundary.sequence,
        trait_trees: rows.clone(),
        ace_portraits: Vec::new(),
    };
    validate_decoded_snapshot(&candidate)?;
    Ok(DecodedTraitComponent {
        sequence: boundary.sequence,
        rows,
    })
}

fn decode_ace_synchronously(
    boundary: &ProcessLocalAceRefresh,
) -> Result<DecodedAceComponent, String> {
    let _decoder_guard = DecoderCallGuard::enter()?;
    let decoder = decoder()?;
    let rows = catch_unwind(AssertUnwindSafe(|| decoder.decode_ace_refresh(boundary)))
        .map_err(|_| "A39 ace fixture decoder panicked")??;
    validate_snapshot(&CharacterPresentationSnapshot {
        source_sequence: boundary.sequence,
        trait_trees: Vec::new(),
        ace_portraits: rows.clone(),
    })
    .map_err(|error| format!("{error:?}"))?;
    Ok(DecodedAceComponent {
        sequence: boundary.sequence,
        rows,
    })
}

fn validate_decoded_snapshot(snapshot: &CharacterPresentationSnapshot) -> Result<(), String> {
    if snapshot
        .trait_trees
        .iter()
        .flat_map(|tree| &tree.nodes)
        .any(|node| node.progress_raw.is_some())
    {
        return Err("A39 live decoder populated unauthenticated trait progress".into());
    }
    validate_snapshot(snapshot).map_err(|error| format!("{error:?}"))
}

fn addresses(base: usize) -> InstalledAddresses {
    InstalledAddresses {
        trait_refresh: base + TRAIT_REFRESH_RVA as usize,
        ace_refresh: base + ACE_REFRESH_RVA as usize,
        ace_row_bind: base + ACE_ROW_BIND_RVA as usize,
    }
}

fn targets(addresses: InstalledAddresses) -> [usize; 3] {
    [
        addresses.trait_refresh,
        addresses.ace_refresh,
        addresses.ace_row_bind,
    ]
}

fn originals_ready() -> bool {
    [
        &ORIGINAL_TRAIT_REFRESH,
        &ORIGINAL_ACE_REFRESH,
        &ORIGINAL_ACE_ROW_BIND,
    ]
    .into_iter()
    .all(|original| original.load(Ordering::Acquire) != 0)
}

struct HookRemovalOutcome {
    error: Option<String>,
    all_removed: bool,
}

fn disable_and_remove(addresses: InstalledAddresses) -> HookRemovalOutcome {
    let mut first_error = None;
    let mut all_removed = true;
    let mut disable_failed = false;
    for target in targets(addresses) {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                disable_failed = true;
                first_error.get_or_insert_with(|| {
                    format!("A39 {target:#x} hook disable failed: {status}")
                });
            }
        }
    }
    if disable_failed {
        return HookRemovalOutcome {
            error: first_error,
            all_removed: false,
        };
    }
    if let Err(error) = wait_for_callback_quiescence(CALLBACK_QUIESCE_TIMEOUT) {
        first_error.get_or_insert(error);
        return HookRemovalOutcome {
            error: first_error,
            all_removed: false,
        };
    }
    for target in targets(addresses) {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                all_removed = false;
                first_error.get_or_insert_with(|| {
                    format!("A39 {target:#x} hook removal failed: {status}")
                });
            }
        }
    }
    HookRemovalOutcome {
        error: first_error,
        all_removed,
    }
}

fn wait_for_callback_quiescence(timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let in_flight = IN_FLIGHT_CALLBACKS.load(Ordering::Acquire);
        if in_flight == 0 {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "A39 teardown timed out with {in_flight} in-flight callback(s); removal deferred"
            ));
        }
        std::thread::yield_now();
    }
}

fn clear_removed_hook_set() {
    INSTALLED.store(false, Ordering::Release);
    HOOK_SET_PRESENT.store(false, Ordering::Release);
    LIFECYCLE_HEALTHY.store(false, Ordering::Release);
    TEARDOWN_ACTIVE.store(false, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    clear_originals();
    if let Some(slot) = FIXTURE_DECODER.get() {
        if let Ok(mut slot) = slot.lock() {
            *slot = None;
        }
    }
}

fn clear_originals() {
    for original in [
        &ORIGINAL_TRAIT_REFRESH,
        &ORIGINAL_ACE_REFRESH,
        &ORIGINAL_ACE_ROW_BIND,
    ] {
        original.store(0, Ordering::Release);
    }
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    if !read_exact(address, &mut actual) {
        return Err(format!("A39 {label} sentinel unreadable"));
    }
    if actual.as_slice() != expected {
        return Err(format!(
            "A39 {label} sentinel mismatch: expected {expected:02X?}, got {actual:02X?}"
        ));
    }
    Ok(())
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
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes).then(|| u32::from_le_bytes(bytes))
}

fn read_u64(address: usize) -> Option<u64> {
    let mut bytes = [0u8; 8];
    read_exact(address, &mut bytes).then(|| u64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        TraitNodeObservation, TraitNodeState, TraitTreeRoot,
    };

    fn candidate(progress_raw: Option<i64>) -> CharacterPresentationSnapshot {
        CharacterPresentationSnapshot {
            source_sequence: 1,
            trait_trees: vec![TraitTreeObservation {
                owner_tag_db_id: 1,
                unit_leader_id: 7,
                root: TraitTreeRoot::ControllerOffset120,
                nodes: vec![TraitNodeObservation {
                    trait_token: "panzer_leader".into(),
                    state: TraitNodeState::Picked,
                    progress_raw,
                }],
            }],
            ace_portraits: vec![AcePortraitObservation {
                owner_tag_db_id: 1,
                ace_id: 11,
                visible_ordinal: 0,
                portrait_gfx: "GFX_GER_ace_m_7".into(),
            }],
        }
    }

    struct Decoder;

    impl ExactA729BoundaryDecoder for Decoder {
        fn decode_trait_refresh(
            &self,
            _boundary: &ProcessLocalTraitRefresh,
        ) -> Result<Vec<TraitTreeObservation>, String> {
            Ok(candidate(None).trait_trees)
        }

        fn decode_ace_refresh(
            &self,
            _boundary: &ProcessLocalAceRefresh,
        ) -> Result<Vec<AcePortraitObservation>, String> {
            Ok(candidate(None).ace_portraits)
        }
    }

    #[test]
    fn production_activation_is_fail_closed() {
        let _decoder = Decoder;
        let source = A729ObservedCharacterPresentationSource::new();
        assert!(!source.is_exact_a729_character_presentation_source());
        assert!(source.read_character_presentation().is_err());
    }

    #[test]
    fn live_candidate_forbids_unproven_progress() {
        assert!(validate_decoded_snapshot(&candidate(None)).is_ok());
        assert!(validate_decoded_snapshot(&candidate(Some(100_000))).is_err());
    }

    #[test]
    fn pinned_rvas_and_sentinels_match_retained_exports() {
        let base = 0x1400_0000_0usize;
        let resolved = addresses(base);
        assert_eq!(resolved.trait_refresh, base + 0x016B_DC20);
        assert_eq!(resolved.ace_refresh, base + 0x0201_E420);
        assert_eq!(resolved.ace_row_bind, base + 0x0201_EDC0);
        assert_eq!(TRAIT_REFRESH_SENTINEL.len(), 18);
        assert_eq!(ACE_REFRESH_SENTINEL.len(), 21);
        assert_eq!(ACE_ROW_BIND_SENTINEL.len(), 17);
        assert_eq!(TRAIT_SELECTED_LEADER_REFERENCE, 0x78);
        assert_eq!(ACE_OBJECT_ID, 0x0c);
    }

    #[test]
    fn stale_or_prehealthy_epoch_witness_never_admits() {
        assert!(!epoch_witness_matches(7, 7, false));
        assert!(!epoch_witness_matches(7, 8, true));
        assert!(epoch_witness_matches(8, 8, true));
    }

    #[test]
    fn decoder_registration_accepts_only_the_same_arc() {
        let first: Arc<dyn ExactA729BoundaryDecoder> = Arc::new(Decoder);
        let same = Arc::clone(&first);
        let different: Arc<dyn ExactA729BoundaryDecoder> = Arc::new(Decoder);
        assert!(same_decoder(&first, &same));
        assert!(!same_decoder(&first, &different));
    }

    #[test]
    fn decoder_reentry_guard_is_fail_closed_and_releases() {
        let outer = DecoderCallGuard::enter().unwrap();
        assert!(DecoderCallGuard::enter().is_err());
        drop(outer);
        assert!(DecoderCallGuard::enter().is_ok());
    }
}
