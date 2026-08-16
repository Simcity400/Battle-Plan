//! Exact-a729 owner registry and materialized-order hook for the five overlap
//! instances reached by the retained A28 controller roots.
//!
//! Four controller constructors/setup functions and the divisions-summary
//! pool constructor register the concrete overlap pointer with its authored,
//! pointer-free collection ID.  `COverlappingElementsBox::Layout` is then
//! observed post-return only for a registered pointer.  This closes stored
//! forward/reverse order and resolved child rectangles without pretending the
//! generic overlap object owns pre-filter source accounting.  A complete DCT1
//! source can be constructed only by joining an owner-captured controller
//! ledger through [`A729NamedOverlapCompleteSource::try_join`].

use crate::{
    CollectionTrace, ExactControllerTraceSource, MaterializedSlotTrace, RectI32, SourceItemTrace,
    TraceFrame,
};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const PEACE_BENEFICIARY_CTOR_RVA: usize = 0x01E3_4B30;
pub const AGENCY_UPGRADE_ENTRY_CTOR_RVA: usize = 0x01A0_0A50;
pub const DIVISIONS_SUMMARY_ITEM_CTOR_RVA: usize = 0x01CE_5430;
pub const DIVISIONS_SUMMARY_POOL_RVA: usize = 0x01CE_6B40;
pub const MARKET_PURCHASE_SETUP_RVA: usize = 0x0203_4060;
pub const OVERLAP_LAYOUT_RVA: usize = 0x0237_2190;

pub const NAMED_OVERLAP_HOOK_JOB_ID: &str =
    "20260720T141317Z-w5-a28-overlap-owner-callgraph-complete-b8a10eba";
pub const NAMED_OVERLAP_HOOK_MANIFEST_SHA256: &str =
    "74d14dced950d49ce7d47e4575bec7b04c84d5739cbc17c09681928b2d72eee4";
pub const NAMED_OVERLAP_SENTINEL_JOB_ID: &str =
    "20260720T215038Z-w5final-a28-tech-overlap-hook-sentinels-ef563094";
pub const NAMED_OVERLAP_SENTINEL_ARTIFACT_SHA256: &str =
    "3eba11027150a60121e8d45e92dbb6e21e4dc85d744c27baaa531a732460e94d";
pub const NAMED_OVERLAP_NAME_XREF_JOB_ID: &str =
    "20260720T214805Z-w5final-a28-five-overlap-controller-name-f2ef9841";
pub const NAMED_OVERLAP_NAME_XREF_ARTIFACT_SHA256: &str =
    "11c73a7d35c701023fb9bfd83a8dc58ba7cd294cc409ed98d659b08bc6c4ed80";

pub const PEACE_COLLECTION_ID: &str =
    "interface/peaceconferencewindow.gui#collection-0003";
pub const UPGRADE_COLLECTION_ID: &str =
    "interface/countryintelligenceagencyview.gui#collection-0008";
pub const DIPLOMACY_DIVISIONS_COLLECTION_ID: &str =
    "interface/countrydiplomacyview.gui#collection-0034";
pub const UNIT_DIVISIONS_COLLECTION_ID: &str = "interface/unitview.gui#collection-0003";
pub const MARKET_COLLECTION_ID: &str =
    "interface/international_market/marketpurchasedraftwindow.gui#collection-0002";

const PEACE_OVERLAP_OFFSET: usize = 0x5D0;
const UPGRADE_OVERLAP_OFFSET: usize = 0x1068;
const DIVISIONS_OVERLAP_OFFSET: usize = 0x58;
const DIVISIONS_POOL_DATA_OFFSET: usize = 0x18;
const DIVISIONS_POOL_COUNT_OFFSET: usize = 0x24;
const MARKET_OVERLAP_OFFSET: usize = 0x1340;
const OWNER_WIDTH_OFFSET: usize = 0x88;
const OWNER_HEIGHT_OFFSET: usize = 0x8C;
const LIST_INTERFACE_OFFSET: usize = 0xA8;
const REVERSE_FLAG_OFFSET: usize = 0x13C;
const CHILD_GUI_OFFSET: usize = 0x38;
const MAX_PENDING: usize = 4_096;

const PEACE_SENTINEL: [u8; 16] = [
    0x48, 0x8B, 0xC4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x48, 0x08, 0x55, 0x56, 0x57,
    0x41, 0x54,
];
const UPGRADE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x18, 0x48, 0x89, 0x4C, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41,
    0x54, 0x41,
];
const DIVISIONS_ITEM_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x4C, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41,
    0x54, 0x41,
];
const DIVISIONS_POOL_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
    0x20, 0x57,
];
const MARKET_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
    0x41, 0x57,
];
const LAYOUT_SENTINEL: [u8; 16] = [
    0x41, 0x56, 0x48, 0x81, 0xEC, 0x80, 0x00, 0x00, 0x00, 0x48, 0x8B, 0x01, 0x4C, 0x8B,
    0xF1, 0xFF,
];

type PeaceCtorFn = unsafe extern "system" fn(usize, usize, usize, u32) -> usize;
type UpgradeCtorFn = unsafe extern "system" fn(
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
) -> usize;
type DivisionsItemCtorFn = unsafe extern "system" fn(usize, usize, u8) -> usize;
type DivisionsPoolFn = unsafe extern "system" fn(usize, usize);
type MarketSetupFn = unsafe extern "system" fn(usize);
type LayoutFn = unsafe extern "system" fn(usize);
type ListAccessorFn = unsafe extern "system" fn(usize) -> usize;
type GeometryFn = unsafe extern "system" fn(usize, *mut i32) -> *mut i32;

static ORIGINAL_PEACE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_UPGRADE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DIVISIONS_ITEM: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DIVISIONS_POOL: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MARKET: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_LAYOUT: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static INSTALL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlapMaterializedChild {
    pub stored_ordinal: u32,
    pub rect: RectI32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedOverlapObservation {
    pub transition_sequence: u64,
    pub observation_qpc_ticks: u64,
    pub controller_registration_sequence: u64,
    pub collection_instance_id: String,
    pub materialized_children: Vec<OverlapMaterializedChild>,
    pub clip_rect: RectI32,
    pub content_extent_pixels: (i32, i32),
    pub viewport_extent_pixels: (i32, i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedOverlapControllerLedger {
    pub source_items: Vec<SourceItemTrace>,
    pub ordered_item_ids: Vec<String>,
    pub row_template_id: String,
    pub slot_ids_in_materialized_order: Vec<String>,
    pub slot_template_ids_in_materialized_order: Vec<String>,
}

#[derive(Clone, Debug)]
struct RegisteredOwner {
    collection_instance_id: &'static str,
    registration_sequence: u64,
}

#[derive(Default)]
struct HookState {
    controller_registrations: u64,
    layout_calls: u64,
    transition_sequence: u64,
    registered: BTreeMap<usize, RegisteredOwner>,
    observations: VecDeque<NamedOverlapObservation>,
    poison: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedOverlapHookDiagnostics {
    pub installed: bool,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub controller_registrations: u64,
    pub registered_owner_count: usize,
    pub layout_calls: u64,
    pub pending_observations: usize,
    pub poisoned: bool,
}

pub struct A729NamedOverlapHookOwner {
    targets: [usize; 6],
    epoch: usize,
    active: bool,
}

impl A729NamedOverlapHookOwner {
    pub fn install() -> Result<Self, String> {
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 named-overlap install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("A28 named-overlap hooks already have a live owner".into());
        }
        let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
        if base == 0 {
            return Err("GetModuleHandleW(NULL) returned null".into());
        }
        let targets = [
            base + PEACE_BENEFICIARY_CTOR_RVA,
            base + AGENCY_UPGRADE_ENTRY_CTOR_RVA,
            base + DIVISIONS_SUMMARY_ITEM_CTOR_RVA,
            base + DIVISIONS_SUMMARY_POOL_RVA,
            base + MARKET_PURCHASE_SETUP_RVA,
            base + OVERLAP_LAYOUT_RVA,
        ];
        for (address, sentinel, label) in [
            (targets[0], &PEACE_SENTINEL[..], "peace beneficiary overlap owner"),
            (targets[1], &UPGRADE_SENTINEL[..], "agency upgrade overlap owner"),
            (targets[2], &DIVISIONS_ITEM_SENTINEL[..], "divisions overlap item owner"),
            (targets[3], &DIVISIONS_POOL_SENTINEL[..], "divisions overlap pool owner"),
            (targets[4], &MARKET_SENTINEL[..], "market overlap owner"),
            (targets[5], &LAYOUT_SENTINEL[..], "overlap layout"),
        ] {
            verify_sentinel(address, sentinel, label)?;
        }
        reset_state()?;
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| value.checked_add(1))
            .map_err(|_| "A28 named-overlap install epoch exhausted".to_string())?
            + 1;
        let hooks = [
            hook_peace as *mut c_void,
            hook_upgrade as *mut c_void,
            hook_divisions_item as *mut c_void,
            hook_divisions_pool as *mut c_void,
            hook_market as *mut c_void,
            hook_layout as *mut c_void,
        ];
        let originals = [
            &ORIGINAL_PEACE,
            &ORIGINAL_UPGRADE,
            &ORIGINAL_DIVISIONS_ITEM,
            &ORIGINAL_DIVISIONS_POOL,
            &ORIGINAL_MARKET,
            &ORIGINAL_LAYOUT,
        ];
        let mut created = 0usize;
        for index in 0..targets.len() {
            match unsafe { MinHook::create_hook(targets[index] as *mut c_void, hooks[index]) } {
                Ok(original) => {
                    originals[index].store(original as usize, Ordering::Release);
                    created += 1;
                }
                Err(status) => {
                    let removed = remove_created(&targets, &originals, created);
                    return Err(if removed {
                        format!("named-overlap hook {index} creation failed: {status}")
                    } else {
                        format!(
                            "named-overlap hook {index} creation failed: {status};                              created prefix RETAINED, teardown still owed"
                        )
                    });
                }
            }
        }
        CALLBACKS_OPEN.store(true, Ordering::Release);
        // Enable the shared consumer first. Foreign layout calls are ignored,
        // while every subsequently enabled owner registration is observable.
        if let Err(status) = unsafe { inline_detour::enable(targets[5] as *mut c_void) } {
            // A FAILED ENABLE MAY STILL HAVE PATCHED (see `remove_created`), so
            // this arm goes through the same close/disable/drain/free it does
            // rather than freeing straight out.
            let removed = remove_created(&targets, &originals, targets.len());
            return Err(if removed {
                format!("named-overlap layout hook enable failed: {status}")
            } else {
                format!(
                    "named-overlap layout hook enable failed: {status};                      created prefix RETAINED, teardown still owed"
                )
            });
        }
        for index in 0..5 {
            if let Err(status) = unsafe { inline_detour::enable(targets[index] as *mut c_void) } {
                // THE FAILING TARGET IS DISABLED TOO, and that is the fix here.
                // This arm used to disable `take(index)` and the shared
                // consumer and then drain -- leaving `targets[index]` itself,
                // whose enable had just reported failure, still patched and
                // still admitting, so the count it waited on was over a set
                // that could refill. `remove_created` now closes admission and
                // disables the WHOLE prefix before it measures anything.
                let removed = remove_created(&targets, &originals, targets.len());
                return Err(if removed {
                    format!("named-overlap hook {index} enable failed: {status}")
                } else {
                    format!(
                        "named-overlap hook {index} enable failed: {status};                          created prefix RETAINED, teardown still owed"
                    )
                });
            }
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(Self {
            targets,
            epoch,
            active: true,
        })
    }

    pub fn diagnostics(&self) -> Result<NamedOverlapHookDiagnostics, String> {
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 named-overlap hook owner epoch is stale".into());
        }
        diagnostics()
    }

    pub fn drain_observations(&self) -> Result<Vec<NamedOverlapObservation>, String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A28 named-overlap hook owner is not active".into());
        }
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 named-overlap hook poisoned: {error}"));
        }
        Ok(state.observations.drain(..).collect())
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 named-overlap install mutex poisoned".to_string())?;
        CALLBACKS_OPEN.store(false, Ordering::Release);
        // Stop all producers before removing the shared layout consumer.
        for target in self.targets.iter().take(5) {
            match unsafe { inline_detour::disable(*target as *mut c_void) } {
                Ok(())
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                Err(status) => {
                    return Err(format!(
                        "named-overlap hook {target:#x} disable failed: {status}"
                    ));
                }
            }
        }
        match unsafe { inline_detour::disable(self.targets[5] as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!(
                    "named-overlap layout hook disable failed: {status}"
                ));
            }
        }
        wait_for_quiescence(Duration::from_secs(30))?;
        let mut removal_failures = Vec::new();
        for target in self.targets.iter().rev() {
            match unsafe { inline_detour::remove(*target as *mut c_void) } {
                Ok(())
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                Err(status) => removal_failures.push(format!(
                    "named-overlap hook {target:#x} removal failed: {status}"
                )),
            }
        }
        if !removal_failures.is_empty() {
            return Err(removal_failures.join(", "));
        }
        for original in original_slots() {
            original.store(0, Ordering::Release);
        }
        INSTALLED.store(false, Ordering::Release);
        self.active = false;
        Ok(())
    }
}

impl Drop for A729NamedOverlapHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Debug)]
pub struct A729NamedOverlapCompleteSource {
    frame_hour: u64,
    collections: Vec<CollectionTrace>,
}

impl A729NamedOverlapCompleteSource {
    pub fn try_join(
        frame_hour: u64,
        joined: Vec<(NamedOverlapObservation, NamedOverlapControllerLedger)>,
    ) -> Result<Self, String> {
        if joined.is_empty() {
            return Err("A28 named-overlap join is empty".into());
        }
        let mut collections = Vec::with_capacity(joined.len());
        for (observation, ledger) in joined {
            collections.push(join_complete_collection(frame_hour, observation, ledger)?);
        }
        crate::validate_frame(&TraceFrame {
            elapsed_wall_ms: 0,
            status_flags: 0,
            frame_hour,
            collections: collections.clone(),
        })
        .map_err(|error| format!("named-overlap DCT1 validation failed: {error:?}"))?;
        Ok(Self {
            frame_hour,
            collections,
        })
    }
}

impl crate::source_seal::Sealed for A729NamedOverlapCompleteSource {}

impl ExactControllerTraceSource for A729NamedOverlapCompleteSource {
    fn is_exact_a729_controller_source(&self) -> bool {
        true
    }

    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<CollectionTrace>, String> {
        if frame_hour != self.frame_hour {
            return Err(format!(
                "A28 named-overlap observation hour {} differs from requested hour {frame_hour}",
                self.frame_hour
            ));
        }
        Ok(self.collections.clone())
    }
}

fn join_complete_collection(
    frame_hour: u64,
    observation: NamedOverlapObservation,
    ledger: NamedOverlapControllerLedger,
) -> Result<CollectionTrace, String> {
    if ledger.row_template_id.trim().is_empty() {
        return Err("named-overlap controller row template is empty".into());
    }
    let mut source_ids = BTreeSet::new();
    let mut included = Vec::new();
    for (ordinal, item) in ledger.source_items.iter().enumerate() {
        if item.source_ordinal as usize != ordinal || !source_ids.insert(item.stable_item_id.as_str()) {
            return Err("named-overlap source identity/ordinal is invalid".into());
        }
        if item.included {
            if item.filter_reason.is_some() {
                return Err("named-overlap included source has a filter reason".into());
            }
            included.push(item.stable_item_id.clone());
        } else {
            match item.filter_reason.as_deref() {
                Some(reason) if !reason.trim().is_empty() => {}
                _ => {
                    return Err("named-overlap excluded source lacks a filter reason".into());
                }
            }
        }
    }
    if ledger.ordered_item_ids != included {
        return Err("named-overlap final order differs from included controller source".into());
    }
    let count = observation.materialized_children.len();
    if ledger.ordered_item_ids.len() != count
        || ledger.slot_ids_in_materialized_order.len() != count
        || ledger.slot_template_ids_in_materialized_order.len() != count
    {
        return Err("named-overlap controller/materialized counts differ".into());
    }
    let mut slot_ids = BTreeSet::new();
    let materialized_slots = observation
        .materialized_children
        .into_iter()
        .zip(ledger.ordered_item_ids.iter())
        .zip(ledger.slot_ids_in_materialized_order.iter())
        .zip(ledger.slot_template_ids_in_materialized_order.iter())
        .enumerate()
        .map(|(ordinal, (((child, bound_item_id), slot_id), template_id))| {
            if child.stored_ordinal as usize != ordinal
                || slot_id.trim().is_empty()
                || template_id.trim().is_empty()
                || !slot_ids.insert(slot_id.as_str())
            {
                return Err("named-overlap materialized slot ledger is invalid".to_string());
            }
            Ok(MaterializedSlotTrace {
                slot_id: slot_id.clone(),
                bound_item_id: bound_item_id.clone(),
                row_template_id: template_id.clone(),
                rect: child.rect,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CollectionTrace {
        collection_instance_id: observation.collection_instance_id,
        frame_hour,
        source_items: ledger.source_items,
        ordered_item_ids: ledger.ordered_item_ids,
        group_boundaries: Vec::new(),
        row_template_id: ledger.row_template_id,
        materialized_slots,
        clip_rect: observation.clip_rect,
        content_extent_pixels: observation.content_extent_pixels,
        viewport_extent_pixels: observation.viewport_extent_pixels,
        scroll_min_fixed: 0,
        scroll_max_fixed: 0,
        scroll_current_fixed: 0,
        scroll_target_fixed: 0,
    })
}

unsafe extern "system" fn hook_peace(
    owner: usize,
    gui: usize,
    beneficiary: usize,
    index: u32,
) -> usize {
    // THE LEASE IS TAKEN BEFORE THE ADMISSION TEST, so the unadmitted
    // arm below -- which still calls the game's original, through the
    // trampoline -- is counted too.
    let in_flight = InFlight::enter();
    if !in_flight.admitted() {
        return call_original_peace(owner, gui, beneficiary, index);
    }
    let result = call_original_peace(owner, gui, beneficiary, index);
    // The constructor's authenticated owner boundary is its returned item,
    // even though exact-a729 currently returns `param_1`.
    register_owner_field(result, PEACE_OVERLAP_OFFSET, PEACE_COLLECTION_ID);
    result
}

unsafe extern "system" fn hook_upgrade(
    owner: usize,
    idler: usize,
    upgrade: usize,
    branch: usize,
    window: usize,
    agency: usize,
    name: usize,
) -> usize {
    // THE LEASE IS TAKEN BEFORE THE ADMISSION TEST, so the unadmitted
    // arm below -- which still calls the game's original, through the
    // trampoline -- is counted too.
    let in_flight = InFlight::enter();
    if !in_flight.admitted() {
        return call_original_upgrade(owner, idler, upgrade, branch, window, agency, name);
    }
    let result = call_original_upgrade(owner, idler, upgrade, branch, window, agency, name);
    // Bind the returned item, matching the exact constructor contract.
    register_owner_field(result, UPGRADE_OVERLAP_OFFSET, UPGRADE_COLLECTION_ID);
    result
}

unsafe extern "system" fn hook_divisions_item(owner: usize, parent: usize, selectable: u8) -> usize {
    // THE LEASE IS TAKEN BEFORE THE ADMISSION TEST, so the unadmitted
    // arm below -- which still calls the game's original, through the
    // trampoline -- is counted too.
    let in_flight = InFlight::enter();
    if !in_flight.admitted() {
        return call_original_divisions_item(owner, parent, selectable);
    }
    let result = call_original_divisions_item(owner, parent, selectable);
    match read_usize(result + DIVISIONS_OVERLAP_OFFSET) {
        Some(overlap) if overlap != 0 => register_divisions_owner(overlap),
        _ => poison("divisions-summary returned item overlap is unreadable/null".into()),
    }
    result
}

unsafe extern "system" fn hook_divisions_pool(pool: usize, gui: usize) {
    // THE LEASE IS TAKEN BEFORE THE ADMISSION TEST, so the unadmitted
    // arm below -- which still calls the game's original, through the
    // trampoline -- is counted too.
    let in_flight = InFlight::enter();
    if !in_flight.admitted() {
        call_original_divisions_pool(pool, gui);
        return;
    }
    call_original_divisions_pool(pool, gui);
    if let Err(error) = register_divisions_pool(pool) {
        poison(error);
    }
}

unsafe extern "system" fn hook_market(owner: usize) {
    // THE LEASE IS TAKEN BEFORE THE ADMISSION TEST, so the unadmitted
    // arm below -- which still calls the game's original, through the
    // trampoline -- is counted too.
    let in_flight = InFlight::enter();
    if !in_flight.admitted() {
        call_original_market(owner);
        return;
    }
    call_original_market(owner);
    register_owner_field(owner, MARKET_OVERLAP_OFFSET, MARKET_COLLECTION_ID);
}

unsafe extern "system" fn hook_layout(owner: usize) {
    // THE LEASE IS TAKEN BEFORE THE ADMISSION TEST, so the unadmitted
    // arm below -- which still calls the game's original, through the
    // trampoline -- is counted too.
    let in_flight = InFlight::enter();
    if !in_flight.admitted() {
        call_original_layout(owner);
        return;
    }
    call_original_layout(owner);
    let registration = match state_lock() {
        Ok(mut state) => {
            state.layout_calls = state.layout_calls.saturating_add(1);
            state.registered.get(&owner).cloned()
        }
        Err(error) => {
            poison(error);
            return;
        }
    };
    let Some(registration) = registration else {
        return;
    };
    if let Err(error) = capture_layout(owner, registration) {
        poison(error);
    }
}

fn register_owner_field(owner: usize, offset: usize, collection_id: &'static str) {
    match read_usize(owner + offset) {
        Some(overlap) if overlap != 0 => {
            if let Err(error) = register_owner(overlap, collection_id) {
                poison(error);
            }
        }
        _ => poison(format!("named-overlap owner field +0x{offset:X} is unreadable/null")),
    }
}

fn register_divisions_pool(pool: usize) -> Result<(), String> {
    let count = read_u32(pool + DIVISIONS_POOL_COUNT_OFFSET)
        .ok_or("divisions-summary pool count unreadable")? as usize;
    if count > crate::MAX_ITEMS_PER_COLLECTION {
        return Err("divisions-summary pool count exceeds bound".into());
    }
    let data = read_usize(pool + DIVISIONS_POOL_DATA_OFFSET)
        .ok_or("divisions-summary pool data unreadable")?;
    for index in 0..count {
        let item = read_usize(data + index * mem::size_of::<usize>())
            .ok_or("divisions-summary item pointer unreadable")?;
        let overlap = read_usize(item + DIVISIONS_OVERLAP_OFFSET)
            .ok_or("divisions-summary overlap pointer unreadable")?;
        register_divisions_owner(overlap);
    }
    Ok(())
}

fn register_divisions_owner(overlap: usize) {
    let width = read_i32(overlap + OWNER_WIDTH_OFFSET);
    let height = read_i32(overlap + OWNER_HEIGHT_OFFSET);
    let collection_id = match (width, height) {
        (Some(202), Some(27)) => Some(DIPLOMACY_DIVISIONS_COLLECTION_ID),
        (Some(192), Some(27)) => Some(UNIT_DIVISIONS_COLLECTION_ID),
        _ => None,
    };
    match collection_id {
        Some(collection_id) => {
            if let Err(error) = register_owner(overlap, collection_id) {
                poison(error);
            }
        }
        None => poison(format!(
            "divisions-summary overlap has foreign authored extent {:?}x{:?}",
            width, height
        )),
    }
}

fn register_owner(overlap: usize, collection_id: &'static str) -> Result<(), String> {
    if overlap == 0 {
        return Err("named-overlap registration has a null runtime owner".into());
    }
    let mut state = state_lock()?;
    state.controller_registrations = state
        .controller_registrations
        .checked_add(1)
        .ok_or("named-overlap registration sequence exhausted")?;
    let sequence = state.controller_registrations;
    if let Some(existing) = state.registered.get(&overlap) {
        if existing.collection_instance_id != collection_id {
            return Err("named-overlap pointer was registered to two authored collections".into());
        }
    }
    state.registered.insert(
        overlap,
        RegisteredOwner {
            collection_instance_id: collection_id,
            registration_sequence: sequence,
        },
    );
    Ok(())
}

fn capture_layout(owner: usize, registration: RegisteredOwner) -> Result<(), String> {
    let list_interface = owner + LIST_INTERFACE_OFFSET;
    let vtable = read_usize(list_interface).ok_or("overlap list-interface vtable unreadable")?;
    let accessor_address = read_usize(vtable + 0x58)
        .ok_or("overlap list accessor function unreadable")?;
    if accessor_address == 0 {
        return Err("overlap list accessor function is null".into());
    }
    let accessor: ListAccessorFn = unsafe { mem::transmute(accessor_address) };
    let list = unsafe { accessor(list_interface) };
    if list == 0 {
        return Err("overlap list accessor returned null".into());
    }
    let count = read_u32(list + 0x10).ok_or("overlap child count unreadable")? as usize;
    if count > crate::MAX_ITEMS_PER_COLLECTION {
        return Err("overlap child count exceeds bound".into());
    }
    let reverse = read_u8(owner + REVERSE_FLAG_OFFSET).ok_or("overlap reverse flag unreadable")? != 0;
    let mut node = read_usize(list + if reverse { 8 } else { 0 })
        .ok_or("overlap list endpoint unreadable")?;
    let mut children = Vec::with_capacity(count);
    for ordinal in 0..count {
        if node == 0 {
            return Err(format!("overlap child list ended at ordinal {ordinal}"));
        }
        let child = read_usize(node).ok_or("overlap child pointer unreadable")?;
        let child_gui = child
            .checked_add(CHILD_GUI_OFFSET)
            .ok_or("overlap child GUI offset overflow")?;
        let position = invoke_geometry(child_gui, 0x78, "overlap child position")?;
        let extent = invoke_geometry(child_gui, 0x28, "overlap child extent")?;
        if extent[0] <= 0 || extent[1] <= 0 {
            return Err("overlap child extent is not positive".into());
        }
        children.push(OverlapMaterializedChild {
            stored_ordinal: ordinal as u32,
            rect: RectI32 {
                x: position[0],
                y: position[1],
                width: extent[0],
                height: extent[1],
            },
        });
        node = read_usize(node + if reverse { 8 } else { 0x10 })
            .ok_or("overlap child link unreadable")?;
    }
    if node != 0 {
        return Err("overlap list contains more nodes than authenticated count".into());
    }
    let position = invoke_geometry(owner, 0x78, "overlap owner position")?;
    let width = read_i32(owner + OWNER_WIDTH_OFFSET).ok_or("overlap owner width unreadable")?;
    let height = read_i32(owner + OWNER_HEIGHT_OFFSET).ok_or("overlap owner height unreadable")?;
    if width <= 0 || height <= 0 {
        return Err("overlap owner extent is not positive".into());
    }
    let qpc_ticks = qpc()?;
    let mut state = state_lock()?;
    state.transition_sequence = state
        .transition_sequence
        .checked_add(1)
        .ok_or("named-overlap transition sequence exhausted")?;
    let sequence = state.transition_sequence;
    state.observations.push_back(NamedOverlapObservation {
        transition_sequence: sequence,
        observation_qpc_ticks: qpc_ticks,
        controller_registration_sequence: registration.registration_sequence,
        collection_instance_id: registration.collection_instance_id.into(),
        materialized_children: children,
        clip_rect: RectI32 {
            x: position[0],
            y: position[1],
            width,
            height,
        },
        content_extent_pixels: (width, height),
        viewport_extent_pixels: (width, height),
    });
    if state.observations.len() > MAX_PENDING {
        state.observations.pop_front();
    }
    Ok(())
}

unsafe fn call_original_peace(owner: usize, gui: usize, beneficiary: usize, index: u32) -> usize {
    let address = ORIGINAL_PEACE.load(Ordering::Acquire);
    if address == 0 {
        return 0;
    }
    let original: PeaceCtorFn = mem::transmute(address);
    original(owner, gui, beneficiary, index)
}

unsafe fn call_original_upgrade(
    owner: usize,
    idler: usize,
    upgrade: usize,
    branch: usize,
    window: usize,
    agency: usize,
    name: usize,
) -> usize {
    let address = ORIGINAL_UPGRADE.load(Ordering::Acquire);
    if address == 0 {
        return 0;
    }
    let original: UpgradeCtorFn = mem::transmute(address);
    original(owner, idler, upgrade, branch, window, agency, name)
}

unsafe fn call_original_divisions_item(owner: usize, parent: usize, selectable: u8) -> usize {
    let address = ORIGINAL_DIVISIONS_ITEM.load(Ordering::Acquire);
    if address == 0 {
        return 0;
    }
    let original: DivisionsItemCtorFn = mem::transmute(address);
    original(owner, parent, selectable)
}

unsafe fn call_original_divisions_pool(pool: usize, gui: usize) {
    let address = ORIGINAL_DIVISIONS_POOL.load(Ordering::Acquire);
    if address != 0 {
        let original: DivisionsPoolFn = mem::transmute(address);
        original(pool, gui);
    }
}

unsafe fn call_original_market(owner: usize) {
    let address = ORIGINAL_MARKET.load(Ordering::Acquire);
    if address != 0 {
        let original: MarketSetupFn = mem::transmute(address);
        original(owner);
    }
}

unsafe fn call_original_layout(owner: usize) {
    let address = ORIGINAL_LAYOUT.load(Ordering::Acquire);
    if address != 0 {
        let original: LayoutFn = mem::transmute(address);
        original(owner);
    }
}

fn original_slots() -> [&'static AtomicUsize; 6] {
    [
        &ORIGINAL_PEACE,
        &ORIGINAL_UPGRADE,
        &ORIGINAL_DIVISIONS_ITEM,
        &ORIGINAL_DIVISIONS_POOL,
        &ORIGINAL_MARKET,
        &ORIGINAL_LAYOUT,
    ]
}

/// Take the created prefix back out: close admission, disable the WHOLE
/// prefix, drain ONCE, then free.
///
/// # Why the order changed
///
/// This used to disable and free one target at a time, with nothing between
/// the two. Both halves of that were wrong:
///
/// * A per-target drain is not a drain at all while the SIBLINGS are still
///   patched in -- they keep admitting, so the count never settles for the
///   reason the caller wants it to. Admission closes and the whole prefix is
///   disabled before anything is measured.
/// * "It only ever runs after a failed `enable`, so nothing was patched" is
///   FALSE IN THIS PRODUCT. `minhook_h4/vendor/src/hook.c`'s `EnableHookLL`
///   writes the patch, then sets `isEnabled` and reports
///   `MH_ERROR_MEMORY_PROTECT` if the protection restore or the
///   instruction-cache flush did not complete -- "the bytes ARE written by the
///   time these run". So a game thread can be inside a detour body here, and it
///   returns through the trampoline the free releases.
///
/// A prefix that cannot be proved quiet is RETAINED: disabled, so nothing new
/// enters it, with trampolines and originals kept for whoever is inside. The
/// caller reports that rather than treating cleanup as done.
#[must_use = "a retained prefix is still owed a teardown and must reach the caller's error"]
fn remove_created(targets: &[usize; 6], originals: &[&AtomicUsize; 6], created: usize) -> bool {
    CALLBACKS_OPEN.store(false, Ordering::Release);
    for index in (0..created).rev() {
        let _ = unsafe { inline_detour::disable(targets[index] as *mut c_void) };
    }
    if wait_for_quiescence(Duration::from_secs(30)).is_err() {
        return false;
    }
    // QUIESCED: admission closed, the whole created prefix disabled, and the
    // in-flight count observed at zero -- all three in this function, above
    // this line, with nothing between them that could reopen any of them.
    let mut removed = true;
    for index in (0..created).rev() {
        if unsafe { inline_detour::remove(targets[index] as *mut c_void) }.is_err() {
            // RETAINED. The original stays published, because a target that is
            // still created may still be reachable and a zeroed original makes
            // its detour skip the game's own function.
            removed = false;
            continue;
        }
        originals[index].store(0, Ordering::Release);
    }
    removed
}

struct InFlight;

impl InFlight {
    /// ALWAYS COUNTS, admitted or not.
    ///
    /// This used to return `None` when `CALLBACKS_OPEN` was already closed --
    /// and the detour then went on to CALL THE ORIGINAL anyway, uncounted. That
    /// is the exact thread teardown must wait for: it is inside this crate's
    /// own detour body, which the freeze cannot see, and it RETURNS THROUGH THE
    /// TRAMPOLINE that `inline_detour::remove` frees. A drain taken after
    /// closing admission read zero straight past it.
    ///
    /// So entry and ADMISSION are now two different questions. The count says
    /// "a thread is in the body"; [`InFlight::admitted`] says "and it may
    /// capture". Closing the gate stops the capture and never stops the count.
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }

    /// Whether this entrant may do the observation half. The call through the
    /// original happens either way -- it is the game's own function.
    fn admitted(&self) -> bool {
        CALLBACKS_OPEN.load(Ordering::Acquire)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

fn diagnostics() -> Result<NamedOverlapHookDiagnostics, String> {
    let state = state_lock()?;
    Ok(NamedOverlapHookDiagnostics {
        installed: INSTALLED.load(Ordering::Acquire),
        callbacks_open: CALLBACKS_OPEN.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT.load(Ordering::Acquire),
        controller_registrations: state.controller_registrations,
        registered_owner_count: state.registered.len(),
        layout_calls: state.layout_calls,
        pending_observations: state.observations.len(),
        poisoned: state.poison.is_some(),
    })
}

fn reset_state() -> Result<(), String> {
    let mut state = state_lock()?;
    *state = HookState::default();
    Ok(())
}

fn state_lock() -> Result<std::sync::MutexGuard<'static, HookState>, String> {
    STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "A28 named-overlap hook state mutex poisoned".to_string())
}

fn poison(error: String) {
    if let Ok(mut state) = state_lock() {
        state.poison.get_or_insert(error);
    }
}

fn invoke_geometry(object: usize, slot: usize, label: &str) -> Result<[i32; 2], String> {
    let vtable = read_usize(object).ok_or_else(|| format!("{label} vtable unreadable"))?;
    let function = read_usize(
        vtable
            .checked_add(slot)
            .ok_or_else(|| format!("{label} vtable slot overflow"))?,
    )
    .ok_or_else(|| format!("{label} function pointer unreadable"))?;
    if function == 0 {
        return Err(format!("{label} function pointer is null"));
    }
    let callback: GeometryFn = unsafe { mem::transmute(function) };
    let mut out = [0i32; 2];
    unsafe {
        callback(object, out.as_mut_ptr());
    }
    Ok(out)
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual = rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err("A28 named-overlap callbacks did not quiesce".into());
        }
        std::thread::yield_now();
    }
    Ok(())
}

fn qpc() -> Result<u64, String> {
    let mut value = 0i64;
    if unsafe { QueryPerformanceCounter(&mut value) } == 0 || value <= 0 {
        return Err("QueryPerformanceCounter failed".into());
    }
    Ok(value as u64)
}

fn read_u8(address: usize) -> Option<u8> {
    rpm(address, 1)?.first().copied()
}

fn read_u32(address: usize) -> Option<u32> {
    Some(u32::from_le_bytes(rpm(address, 4)?.try_into().ok()?))
}

fn read_i32(address: usize) -> Option<i32> {
    Some(i32::from_le_bytes(rpm(address, 4)?.try_into().ok()?))
}

fn read_usize(address: usize) -> Option<usize> {
    Some(usize::from_le_bytes(
        rpm(address, mem::size_of::<usize>())?.try_into().ok()?,
    ))
}

fn rpm(address: usize, len: usize) -> Option<Vec<u8>> {
    if address == 0 {
        return None;
    }
    let mut output = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            output.as_mut_ptr().cast(),
            len,
            &mut read,
        )
    };
    (ok != 0 && read == len).then_some(output)
}
