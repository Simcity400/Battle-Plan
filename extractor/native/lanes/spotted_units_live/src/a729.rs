use core::ffi::c_void;
use minhook::MinHook;
use production_hook_receipt::rel5_roster::Rel5HookRoster;
use production_hook_receipt::{
    ExpectedRvaHook, InstallAbortDiscard, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    SiteDetour, MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{SpottedUnitHandle, SpottedUnitsSnapshot, MAX_POV_COUNTRY_ID};

pub const INTEL_GETTER_RVA: u32 = 0x0160_68C0;
pub const LAND_FRAME_RVA: u32 = 0x0165_85E0;
pub const LAND_ELIGIBILITY_RVA: u32 = 0x00BE_2AB0;
pub const TASKFORCE_FRAME_RVA: u32 = 0x018D_DF50;
const GAMESTATE_PTR_RVA: usize = 0x0331_6E90;
const NAVY_DEBUG_OVERRIDE_RVA: usize = 0x0331_7252;
const LAND_ICON_DISPLAY_FLAG_RVA: usize = 0x0331_8B3C;
const TASKFORCE_ICON_GATE_OFFSET: usize = 0x5B8;
const TASKFORCE_SOURCE_COUNT_OFFSET: usize = 0x104;
const GAMESTATE_CURRENT_DATE: usize = 0x468;

const GETTER_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x9b, 0x09, 0xd1, 0x01, 0x00, 0x48,
];
const LAND_FRAME_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x48, 0x08, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
];
const LAND_ELIGIBILITY_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x83, 0xb9, 0x88, 0x00, 0x00, 0x00, 0x00, 0x74, 0x3e,
];
const TASKFORCE_FRAME_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];

pub const HOOK_RECEIPT_COUNT: usize = 4;

const SITE_INTEL_GETTER: usize = 0;
const SITE_LAND_FRAME: usize = 1;
const SITE_LAND_ELIGIBILITY: usize = 2;
const SITE_TASKFORCE_FRAME: usize = 3;

pub const PRODUCTION_HOOK_SPECS: [ExpectedRvaHook; HOOK_RECEIPT_COUNT] = [
    ExpectedRvaHook {
        logical_name: "gameplay.d95.intel_getter",
        requested_rva: INTEL_GETTER_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &GETTER_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d95.land_frame",
        requested_rva: LAND_FRAME_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &LAND_FRAME_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d95.land_eligibility",
        requested_rva: LAND_ELIGIBILITY_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &LAND_ELIGIBILITY_SENTINEL,
    },
    ExpectedRvaHook {
        logical_name: "gameplay.d95.taskforce_frame",
        requested_rva: TASKFORCE_FRAME_RVA as u64,
        target_patch_len: 5,
        primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
        original_prefix: &TASKFORCE_FRAME_SENTINEL,
    },
];

fn hook_batch(targets: &[usize]) -> Vec<(&'static str, *mut c_void)> {
    targets
        .iter()
        .enumerate()
        .map(|(index, target)| {
            (
                PRODUCTION_HOOK_SPECS[index].logical_name,
                *target as *mut c_void,
            )
        })
        .collect()
}

static HOOK_RECEIPTS: Rel5HookRoster<HOOK_RECEIPT_COUNT> =
    Rel5HookRoster::new("D95", PRODUCTION_HOOK_SPECS);

type IntelGetterFn = unsafe extern "system" fn(usize) -> u32;
type LandFrameFn = unsafe extern "system" fn(usize);
type LandEligibilityFn = unsafe extern "system" fn(usize) -> u8;
type TaskforceFrameFn = unsafe extern "system" fn(usize, usize, usize);

static ORIGINAL_GETTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_LAND_FRAME: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_LAND_ELIGIBILITY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TASKFORCE_FRAME: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
/// A release left a detour PATCHED INTO THE GAME, so this unit's sites can
static RETAINED_LIVE: AtomicBool = AtomicBool::new(false);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static SITE_ENTRIES: [AtomicU64; HOOK_RECEIPT_COUNT] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static LATEST: OnceLock<Mutex<LatestFrames>> = OnceLock::new();
static LIFECYCLE: Mutex<()> = Mutex::new(());

/// Lifetime per-arm census, in `CENSUS_*` index order. Published on the wire so
const CENSUS_LAND: usize = 0;
const CENSUS_TASKFORCE: usize = 1;
const CENSUS_SURFACE_COUNT: usize = 2;

/// process lifetime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FrameCensus {
    candidates_seen: u64,
    rejected_unreadable_handle: u64,
    rejected_below_intel_threshold: u64,
    rejected_by_eligibility: u64,
    rejected_taskforce_state: u64,
    abandoned_pending: u64,
    admitted: u64,
    frames_without_enumeration: u64,
    frames_producer_gate_closed: u64,
    admitted_rows_lost_to_replacement: u64,
    frames_source_collection_empty: u64,
}

impl FrameCensus {
    fn accounted(&self) -> u64 {
        self.rejected_unreadable_handle
            .saturating_add(self.rejected_below_intel_threshold)
            .saturating_add(self.rejected_by_eligibility)
            .saturating_add(self.rejected_taskforce_state)
            .saturating_add(self.abandoned_pending)
            .saturating_add(self.admitted)
    }

    fn accumulate(&mut self, other: &FrameCensus) {
        self.candidates_seen = self.candidates_seen.saturating_add(other.candidates_seen);
        self.rejected_unreadable_handle = self
            .rejected_unreadable_handle
            .saturating_add(other.rejected_unreadable_handle);
        self.rejected_below_intel_threshold = self
            .rejected_below_intel_threshold
            .saturating_add(other.rejected_below_intel_threshold);
        self.rejected_by_eligibility = self
            .rejected_by_eligibility
            .saturating_add(other.rejected_by_eligibility);
        self.rejected_taskforce_state = self
            .rejected_taskforce_state
            .saturating_add(other.rejected_taskforce_state);
        self.abandoned_pending = self
            .abandoned_pending
            .saturating_add(other.abandoned_pending);
        self.admitted = self.admitted.saturating_add(other.admitted);
        self.frames_without_enumeration = self
            .frames_without_enumeration
            .saturating_add(other.frames_without_enumeration);
        self.frames_producer_gate_closed = self
            .frames_producer_gate_closed
            .saturating_add(other.frames_producer_gate_closed);
        self.frames_source_collection_empty = self
            .frames_source_collection_empty
            .saturating_add(other.frames_source_collection_empty);
        self.admitted_rows_lost_to_replacement = self
            .admitted_rows_lost_to_replacement
            .saturating_add(other.admitted_rows_lost_to_replacement);
    }
}

#[derive(Clone)]
struct CompleteFrame {
    sequence: u64,
    pov_country_id: u32,
    producer_hour_raw: Option<i32>,
    rows: std::sync::Arc<[SpottedUnitHandle]>,
    rows_certified: bool,
    served: bool,
}

#[derive(Default)]
struct LatestFrames {
    land: Option<CompleteFrame>,
    taskforces: Option<CompleteFrame>,
    /// Lifetime census per surface, indexed by `CENSUS_*`.
    census: [FrameCensus; CENSUS_SURFACE_COUNT],
}

struct BuildingFrame {
    pov_country_id: u32,
    producer_hour_raw: Option<i32>,
    rows: BTreeMap<(u32, u32), u8>,
    pending_land: Option<SpottedUnitHandle>,
    enumeration_gate_open: Option<bool>,
    source_collection_len: Option<i32>,
    census: FrameCensus,
}

thread_local! {
    static LAND_BUILD: RefCell<Option<BuildingFrame>> = const { RefCell::new(None) };
    static TASKFORCE_BUILD: RefCell<Option<BuildingFrame>> = const { RefCell::new(None) };
    static CURRENT_SURFACE: Cell<Option<usize>> = const { Cell::new(None) };
}

struct CallbackLease;
impl CallbackLease {
    fn enter(site: usize) -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        if let Some(entries) = SITE_ENTRIES.get(site) {
            entries.fetch_add(1, Ordering::Relaxed);
        }
        Self
    }
}
impl Drop for CallbackLease {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn install() -> Result<(), String> {
    let _guard = LIFECYCLE
        .lock()
        .map_err(|_| "spotted-unit hook lifecycle poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    // A previous release left a detour live. `INSTALLED` is false, so without
    if RETAINED_LIVE.load(Ordering::Acquire) {
        return Err(
            "spotted-unit hooks cannot be reinstalled: a previous release left a live detour"
                .into(),
        );
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("spotted-unit hook module unavailable".into());
    }
    let specs: [(u32, &[u8], *mut c_void, &AtomicUsize); 4] = [
        (
            INTEL_GETTER_RVA,
            &GETTER_SENTINEL,
            hook_intel_getter as *mut c_void,
            &ORIGINAL_GETTER,
        ),
        (
            LAND_FRAME_RVA,
            &LAND_FRAME_SENTINEL,
            hook_land_frame as *mut c_void,
            &ORIGINAL_LAND_FRAME,
        ),
        (
            LAND_ELIGIBILITY_RVA,
            &LAND_ELIGIBILITY_SENTINEL,
            hook_land_eligibility as *mut c_void,
            &ORIGINAL_LAND_ELIGIBILITY,
        ),
        (
            TASKFORCE_FRAME_RVA,
            &TASKFORCE_FRAME_SENTINEL,
            hook_taskforce_frame as *mut c_void,
            &ORIGINAL_TASKFORCE_FRAME,
        ),
    ];
    let mut created = Vec::new();
    for (rva, sentinel, detour, original_slot) in specs {
        let target = match base.checked_add(rva as usize) {
            Some(target) => target,
            None => {
                if !rollback(&created) {
                    clear_originals();
                }
                return Err("spotted-unit hook address overflow".into());
            }
        };
        let mut actual = vec![0u8; sentinel.len()];
        if let Err(error) = read_exact(target, &mut actual) {
            if !rollback(&created) {
                clear_originals();
            }
            return Err(error);
        }
        if actual != sentinel {
            if !rollback(&created) {
                clear_originals();
            }
            return Err(format!(
                "spotted-unit hook sentinel mismatch at RVA {rva:#x}"
            ));
        }
        let original = match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(value) => value,
            Err(status) => {
                if !rollback(&created) {
                    clear_originals();
                }
                return Err(format!(
                    "spotted-unit hook creation failed at RVA {rva:#x}: {status}"
                ));
            }
        };
        original_slot.store(original as usize, Ordering::Release);
        created.push(target);
    }
    MODULE_BASE.store(base, Ordering::Release);
    let mut latest = match LATEST
        .get_or_init(|| Mutex::new(LatestFrames::default()))
        .lock()
    {
        Ok(latest) => latest,
        Err(_) => {
            if !rollback(&created) {
                clear_originals();
            }
            MODULE_BASE.store(0, Ordering::Release);
            return Err("spotted-unit latest frames poisoned".into());
        }
    };
    *latest = LatestFrames::default();
    drop(latest);
    let enabled = unsafe { inline_detour::enable_batch(&hook_batch(&created)) };
    if let Some((name, status)) = enabled.refusal() {
        if !rollback(&created) {
            clear_originals();
        }
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("spotted-unit hook {name} enable failed: {status}"));
    }
    if let Err(error) = HOOK_RECEIPTS.observe_installed(base, detours()) {
        let retained = rollback(&created);
        if !retained {
            clear_originals();
            HOOK_RECEIPTS.discard_all_custody();
        }
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("spotted-unit hook receipt refused: {error}"));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

fn detours() -> [SiteDetour; HOOK_RECEIPT_COUNT] {
    [
        SiteDetour::new(
            "gameplay.d95.intel_getter",
            hook_intel_getter as *const () as usize,
        ),
        SiteDetour::new(
            "gameplay.d95.land_frame",
            hook_land_frame as *const () as usize,
        ),
        SiteDetour::new(
            "gameplay.d95.land_eligibility",
            hook_land_eligibility as *const () as usize,
        ),
        SiteDetour::new(
            "gameplay.d95.taskforce_frame",
            hook_taskforce_frame as *const () as usize,
        ),
    ]
}

pub fn site_entry_counts() -> [u64; HOOK_RECEIPT_COUNT] {
    let mut counts = [0u64; HOOK_RECEIPT_COUNT];
    for (slot, entries) in counts.iter_mut().zip(SITE_ENTRIES.iter()) {
        *slot = entries.load(Ordering::Relaxed);
    }
    counts
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// True while this lane holds a release it could not PROVE.
///     release only when every disable AND every removal came back clean, so an
pub fn release_pending() -> bool {
    RETAINED_LIVE.load(Ordering::Acquire)
        || (!installed() && HOOK_RECEIPTS.may_still_hold_installed_custody())
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.installed_receipts()
}

/// Consume the release receipts. One-shot: a second call after the registry has
/// taken custody reports them unavailable rather than minting a second release.
pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String>
{
    HOOK_RECEIPTS.take_released_receipts()
}

/// Release this lane for a production install that is being given up, and
pub fn abort_partial_install_and_uninstall() -> Result<InstallAbortDiscard, String> {
    let retained = match drain_latest() {
        Ok(Some(_)) => 1,
        Ok(None) => 0,
        Err(error) if error.contains("are not installed") => 0,
        Err(_) => 1,
    };
    uninstall().map_err(|error| format!("D95 install abort: {error}"))?;
    // exactly the state a retained release leaves behind. A claimed release over
    // a live detour is not a release.
    if RETAINED_LIVE.load(Ordering::Acquire) {
        return Err(
            "D95 install abort could not prove its detours were removed; a previous release \
             retained a live patch"
                .into(),
        );
    }
    Ok(InstallAbortDiscard {
        retained_rows: retained,
        ..InstallAbortDiscard::ZERO
    })
}

pub fn drain_latest() -> Result<Option<SpottedUnitsSnapshot>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("spotted-unit hooks are not installed".into());
    }
    let mut latest = LATEST
        .get()
        .ok_or("spotted-unit hook state unavailable")?
        .lock()
        .map_err(|_| "spotted-unit latest frames poisoned")?;
    let (land, taskforces) = pick_frames(&latest);
    let decided: Result<Option<SnapshotHeader>, &'static str> =
        match snapshot_header(land.as_ref(), taskforces.as_ref()) {
            Ok(Some(header)) => match header.validate(land.as_ref(), taskforces.as_ref()) {
                Ok(()) => {
                    mark_served(latest.land.as_mut(), header.land_frame_sequence);
                    mark_served(latest.taskforces.as_mut(), header.taskforce_frame_sequence);
                    Ok(Some(header))
                }
                Err(reason) => Err(reason),
            },
            Ok(None) => Ok(None),
            Err(reason) => Err(reason),
        };
    drop(latest);
    match decided {
        Ok(Some(header)) => Ok(Some(header.materialise(land, taskforces))),
        Ok(None) => Ok(None),
        Err(reason) => Err(reason.to_string()),
    }
}

struct SnapshotHeader {
    capture_sequence: u64,
    pov_country_id: u32,
    land_frame_sequence: Option<u64>,
    taskforce_frame_sequence: Option<u64>,
    land_frame_hour_raw: Option<i32>,
    taskforce_frame_hour_raw: Option<i32>,
}

impl SnapshotHeader {
    fn validate(
        &self,
        land: Option<&CompleteFrame>,
        taskforces: Option<&CompleteFrame>,
    ) -> Result<(), &'static str> {
        compact_contract::spotted_units::validate_snapshot_header(
            self.capture_sequence,
            self.pov_country_id,
            self.land_frame_sequence,
            self.taskforce_frame_sequence,
            self.land_frame_hour_raw,
            self.taskforce_frame_hour_raw,
            land.map(|frame| frame.rows.len()).unwrap_or(0),
            taskforces.map(|frame| frame.rows.len()).unwrap_or(0),
        )?;
        if land.is_some_and(|frame| !frame.rows_certified) {
            return Err("spotted-units land row is outside the admitted range");
        }
        if taskforces.is_some_and(|frame| !frame.rows_certified) {
            return Err("spotted-units taskforce row is outside the admitted range");
        }
        Ok(())
    }

    fn materialise(
        self,
        land: Option<CompleteFrame>,
        taskforces: Option<CompleteFrame>,
    ) -> SpottedUnitsSnapshot {
        SpottedUnitsSnapshot {
            capture_sequence: self.capture_sequence,
            pov_country_id: self.pov_country_id,
            land_frame_sequence: self.land_frame_sequence,
            taskforce_frame_sequence: self.taskforce_frame_sequence,
            land_frame_hour_raw: self.land_frame_hour_raw,
            taskforce_frame_hour_raw: self.taskforce_frame_hour_raw,
            land: land.map(|frame| frame.rows.to_vec()).unwrap_or_default(),
            taskforces: taskforces
                .map(|frame| frame.rows.to_vec())
                .unwrap_or_default(),
        }
    }
}

fn snapshot_header(
    land: Option<&CompleteFrame>,
    taskforces: Option<&CompleteFrame>,
) -> Result<Option<SnapshotHeader>, &'static str> {
    let pov_country_id = land
        .map(|frame| frame.pov_country_id)
        .or_else(|| taskforces.map(|frame| frame.pov_country_id));
    let Some(pov_country_id) = pov_country_id else {
        return Ok(None);
    };
    let capture_sequence = next_sequence().ok_or("spotted-unit capture sequence exhausted")?;
    Ok(Some(SnapshotHeader {
        capture_sequence,
        pov_country_id,
        land_frame_sequence: land.map(|frame| frame.sequence),
        taskforce_frame_sequence: taskforces.map(|frame| frame.sequence),
        land_frame_hour_raw: land.and_then(|frame| frame.producer_hour_raw),
        taskforce_frame_hour_raw: taskforces.and_then(|frame| frame.producer_hour_raw),
    }))
}

fn pick_frames(latest: &LatestFrames) -> (Option<CompleteFrame>, Option<CompleteFrame>) {
    let mut land = latest.land.clone();
    let mut taskforces = latest.taskforces.clone();
    if let (Some(a), Some(b)) = (&land, &taskforces) {
        if a.pov_country_id != b.pov_country_id {
            if a.sequence > b.sequence {
                taskforces = None;
            } else {
                land = None;
            }
        }
    }
    (land, taskforces)
}

#[cfg(test)]
fn record_served(latest: &mut LatestFrames, snapshot: Option<&SpottedUnitsSnapshot>) {
    let Some(snapshot) = snapshot else {
        return;
    };
    mark_served(latest.land.as_mut(), snapshot.land_frame_sequence);
    mark_served(
        latest.taskforces.as_mut(),
        snapshot.taskforce_frame_sequence,
    );
}

fn mark_served(slot: Option<&mut CompleteFrame>, served_sequence: Option<u64>) {
    let (Some(frame), Some(served_sequence)) = (slot, served_sequence) else {
        return;
    };
    if frame.sequence == served_sequence {
        frame.served = true;
    }
}

/// The lifetime per-arm census, indexed by `CENSUS_*`. Empty before the first
pub fn census_units() -> [[u64; CENSUS_UNIT_COUNT]; CENSUS_SURFACE_COUNT] {
    let Some(latest) = LATEST.get().and_then(|latest| latest.lock().ok()) else {
        return [[0; CENSUS_UNIT_COUNT]; CENSUS_SURFACE_COUNT];
    };
    let mut out = [[0; CENSUS_UNIT_COUNT]; CENSUS_SURFACE_COUNT];
    for (index, census) in latest.census.iter().enumerate() {
        out[index] = [
            census.candidates_seen,
            census.rejected_unreadable_handle,
            census.rejected_below_intel_threshold,
            census.rejected_by_eligibility,
            census.rejected_taskforce_state,
            census.abandoned_pending,
            census.admitted,
            census.frames_without_enumeration,
            census.frames_producer_gate_closed,
            census.frames_source_collection_empty,
            census.admitted_rows_lost_to_replacement,
        ];
    }
    out
}

pub const CENSUS_CANDIDATES_SEEN: usize = 0;
pub const CENSUS_REJECTED_UNREADABLE_HANDLE: usize = 1;
pub const CENSUS_REJECTED_BELOW_INTEL_THRESHOLD: usize = 2;
pub const CENSUS_REJECTED_BY_ELIGIBILITY: usize = 3;
pub const CENSUS_REJECTED_TASKFORCE_STATE: usize = 4;
pub const CENSUS_ABANDONED_PENDING: usize = 5;
pub const CENSUS_ADMITTED: usize = 6;
pub const CENSUS_FRAMES_WITHOUT_ENUMERATION: usize = 7;
pub const CENSUS_FRAMES_PRODUCER_GATE_CLOSED: usize = 8;
pub const CENSUS_FRAMES_SOURCE_COLLECTION_EMPTY: usize = 9;
pub const CENSUS_ADMITTED_ROWS_LOST_TO_REPLACEMENT: usize = 10;
pub const CENSUS_SURFACE_LAND: usize = CENSUS_LAND;
pub const CENSUS_SURFACE_TASKFORCE: usize = CENSUS_TASKFORCE;
pub const CENSUS_UNIT_COUNT: usize = 11;
pub const CENSUS_SURFACES: usize = CENSUS_SURFACE_COUNT;

#[cfg(test)]
fn snapshot_from_latest(latest: &LatestFrames) -> Result<Option<SpottedUnitsSnapshot>, String> {
    let (land, taskforces) = pick_frames(latest);
    snapshot_from_frames(land.as_ref(), taskforces.as_ref())
}

#[cfg(test)]
fn snapshot_from_frames(
    land: Option<&CompleteFrame>,
    taskforces: Option<&CompleteFrame>,
) -> Result<Option<SpottedUnitsSnapshot>, String> {
    let Some(header) = snapshot_header(land, taskforces).map_err(str::to_string)? else {
        return Ok(None);
    };
    header.validate(land, taskforces).map_err(str::to_string)?;
    let snapshot = header.materialise(land.cloned(), taskforces.cloned());
    snapshot.validate()?;
    Ok(Some(snapshot))
}

pub fn uninstall() -> Result<(), String> {
    let _guard = LIFECYCLE
        .lock()
        .map_err(|_| "spotted-unit hook lifecycle poisoned")?;
    if !INSTALLED.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let base = MODULE_BASE.swap(0, Ordering::AcqRel);
    let mut errors = Vec::new();
    let canonical_targets = [
        base.saturating_add(INTEL_GETTER_RVA as usize),
        base.saturating_add(LAND_FRAME_RVA as usize),
        base.saturating_add(LAND_ELIGIBILITY_RVA as usize),
        base.saturating_add(TASKFORCE_FRAME_RVA as usize),
    ];
    let mut batch = hook_batch(&canonical_targets);
    batch.reverse();
    let disabled = unsafe { inline_detour::disable_batch(&batch) };
    let retained = !disabled.all_succeeded();
    errors.extend(
        disabled
            .failures()
            .into_iter()
            .map(|(name, status)| format!("spotted-unit hook {name} disable failed: {status}")),
    );
    if !retained {
        while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
        }
        errors.extend(
            unsafe { inline_detour::remove_batch(&batch) }
                .failures()
                .into_iter()
                .map(|(name, status)| format!("spotted-unit hook {name} removal failed: {status}")),
        );
    }
    if retained {
        RETAINED_LIVE.store(true, Ordering::Release);
    } else {
        clear_originals();
    }
    LAND_BUILD.with(|slot| *slot.borrow_mut() = None);
    TASKFORCE_BUILD.with(|slot| *slot.borrow_mut() = None);
    // Mint the release receipts only for a teardown that provably restored
    // every target. `observe_released_set` re-reads each site's bytes and
    // Receipt custody is published only when the physical release ALSO reported
    // describe, and a release receipt is the registry's evidence that the site
    if !retained && errors.is_empty() {
        if let Err(error) = HOOK_RECEIPTS.publish_release(true) {
            errors.push(format!(
                "spotted-unit hook release receipt refused: {error}"
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

unsafe extern "system" fn hook_land_frame(owner: usize) {
    let _lease = CallbackLease::enter(SITE_LAND_FRAME);
    let original: LandFrameFn = match original_fn(ORIGINAL_LAND_FRAME.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return,
    };
    let prior = LAND_BUILD.with(|slot| slot.borrow_mut().take());
    let gate_open = land_icon_display_flag_open();
    let frame = current_pov_country_id().map(|pov| {
        let mut frame = new_building_frame(pov);
        frame.enumeration_gate_open = gate_open;
        frame
    });
    LAND_BUILD.with(|slot| *slot.borrow_mut() = frame);
    let prior_surface = CURRENT_SURFACE.with(|slot| slot.replace(Some(CENSUS_LAND)));
    original(owner);
    CURRENT_SURFACE.with(|slot| slot.set(prior_surface));
    let complete = LAND_BUILD.with(|slot| slot.borrow_mut().take());
    LAND_BUILD.with(|slot| *slot.borrow_mut() = prior);
    if let Some(frame) = complete {
        publish(frame, true);
    }
}

unsafe extern "system" fn hook_taskforce_frame(owner: usize, source: usize, context: usize) {
    let _lease = CallbackLease::enter(SITE_TASKFORCE_FRAME);
    let original: TaskforceFrameFn =
        match original_fn(ORIGINAL_TASKFORCE_FRAME.load(Ordering::Acquire)) {
            Some(value) => value,
            None => return,
        };
    let prior = TASKFORCE_BUILD.with(|slot| slot.borrow_mut().take());
    let debug_override = MODULE_BASE
        .load(Ordering::Acquire)
        .checked_add(NAVY_DEBUG_OVERRIDE_RVA)
        .and_then(|address| read_u8(address).ok())
        .unwrap_or(1);
    let gate_open = taskforce_icon_gate_open(owner);
    let source_collection_len = taskforce_source_collection_len(source);
    let frame = if debug_override == 0 {
        current_pov_country_id().map(|pov| {
            let mut frame = new_building_frame(pov);
            frame.enumeration_gate_open = gate_open;
            frame.source_collection_len = source_collection_len;
            frame
        })
    } else {
        None
    };
    TASKFORCE_BUILD.with(|slot| *slot.borrow_mut() = frame);
    let prior_surface = CURRENT_SURFACE.with(|slot| slot.replace(Some(CENSUS_TASKFORCE)));
    original(owner, source, context);
    CURRENT_SURFACE.with(|slot| slot.set(prior_surface));
    let complete = TASKFORCE_BUILD.with(|slot| slot.borrow_mut().take());
    TASKFORCE_BUILD.with(|slot| *slot.borrow_mut() = prior);
    if let Some(frame) = complete {
        publish(frame, false);
    }
}

unsafe extern "system" fn hook_intel_getter(unit: usize) -> u32 {
    let _lease = CallbackLease::enter(SITE_INTEL_GETTER);
    let original: IntelGetterFn = match original_fn(ORIGINAL_GETTER.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return 0,
    };
    let raw = original(unit) & 0xff;
    let handle = read_handle(unit, raw as u8);
    match CURRENT_SURFACE.with(Cell::get) {
        Some(CENSUS_LAND) => LAND_BUILD.with(|slot| {
            if let Some(frame) = slot.borrow_mut().as_mut() {
                offer_land_candidate(frame, handle);
            }
        }),
        Some(CENSUS_TASKFORCE) => {
            let taskforce_state = handle.filter(|row| row.raw_intel >= 150).map(|row| {
                (
                    row,
                    read_i32(unit.saturating_add(0x20c))
                        .ok()
                        .is_some_and(|value| value < 1),
                )
            });
            TASKFORCE_BUILD.with(|slot| {
                if let Some(frame) = slot.borrow_mut().as_mut() {
                    offer_taskforce_candidate(frame, handle.is_some(), taskforce_state);
                }
            });
        }
        _ => {}
    }
    raw
}

fn offer_land_candidate(frame: &mut BuildingFrame, handle: Option<SpottedUnitHandle>) {
    frame.census.candidates_seen = frame.census.candidates_seen.saturating_add(1);
    if frame.pending_land.is_some() {
        frame.census.abandoned_pending = frame.census.abandoned_pending.saturating_add(1);
    }
    match handle {
        None => {
            frame.census.rejected_unreadable_handle =
                frame.census.rejected_unreadable_handle.saturating_add(1);
        }
        Some(row) if row.raw_intel < 250 => {
            frame.census.rejected_below_intel_threshold = frame
                .census
                .rejected_below_intel_threshold
                .saturating_add(1);
        }
        Some(_) => {}
    }
    frame.pending_land = handle.filter(|row| row.raw_intel >= 250);
}

fn offer_land_eligibility(frame: &mut BuildingFrame, admitted: bool) {
    let Some(row) = frame.pending_land.take() else {
        return;
    };
    if admitted {
        frame.census.admitted = frame.census.admitted.saturating_add(1);
        insert_row(frame, row);
    } else {
        frame.census.rejected_by_eligibility =
            frame.census.rejected_by_eligibility.saturating_add(1);
    }
}

fn offer_taskforce_candidate(
    frame: &mut BuildingFrame,
    readable: bool,
    state: Option<(SpottedUnitHandle, bool)>,
) {
    frame.census.candidates_seen = frame.census.candidates_seen.saturating_add(1);
    match state {
        None if !readable => {
            frame.census.rejected_unreadable_handle =
                frame.census.rejected_unreadable_handle.saturating_add(1);
        }
        None => {
            frame.census.rejected_below_intel_threshold = frame
                .census
                .rejected_below_intel_threshold
                .saturating_add(1);
        }
        Some((_, false)) => {
            frame.census.rejected_taskforce_state =
                frame.census.rejected_taskforce_state.saturating_add(1);
        }
        Some((row, true)) => {
            frame.census.admitted = frame.census.admitted.saturating_add(1);
            insert_row(frame, row);
        }
    }
}

unsafe extern "system" fn hook_land_eligibility(wrapper: usize) -> u8 {
    let _lease = CallbackLease::enter(SITE_LAND_ELIGIBILITY);
    let original: LandEligibilityFn =
        match original_fn(ORIGINAL_LAND_ELIGIBILITY.load(Ordering::Acquire)) {
            Some(value) => value,
            None => return 0,
        };
    let admitted = original(wrapper);
    LAND_BUILD.with(|slot| {
        if let Some(frame) = slot.borrow_mut().as_mut() {
            offer_land_eligibility(frame, admitted != 0);
        }
    });
    admitted
}

fn new_building_frame(pov_country_id: u32) -> BuildingFrame {
    BuildingFrame {
        pov_country_id,
        producer_hour_raw: current_game_hour_raw(),
        rows: BTreeMap::new(),
        pending_land: None,
        enumeration_gate_open: None,
        source_collection_len: None,
        census: FrameCensus::default(),
    }
}

fn land_icon_display_flag_open() -> Option<bool> {
    MODULE_BASE
        .load(Ordering::Acquire)
        .checked_add(LAND_ICON_DISPLAY_FLAG_RVA)
        .and_then(|address| read_u8(address).ok())
        .map(|flag| flag != 0)
}

fn taskforce_icon_gate_open(owner: usize) -> Option<bool> {
    owner
        .checked_add(TASKFORCE_ICON_GATE_OFFSET)
        .and_then(|address| read_u8(address).ok())
        .map(|gate| gate != 0)
}

fn taskforce_source_collection_len(source: usize) -> Option<i32> {
    source
        .checked_add(TASKFORCE_SOURCE_COUNT_OFFSET)
        .and_then(|address| read_i32(address).ok())
}

fn insert_row(frame: &mut BuildingFrame, row: SpottedUnitHandle) {
    frame
        .rows
        .entry((row.type_id, row.stable_id))
        .and_modify(|value| *value = (*value).max(row.raw_intel))
        .or_insert(row.raw_intel);
}

fn publish(frame: BuildingFrame, land: bool) {
    let Some(sequence) = next_sequence() else {
        return;
    };
    // mutex is taken, and the outgoing frame is dropped AFTER it is released,
    let (census, complete) = materialise_frame(frame, sequence, land);
    let outgoing = LATEST.get().and_then(|latest| {
        latest
            .lock()
            .ok()
            .map(|mut slot| commit_complete(&mut slot, census, complete, land))
    });
    drop(outgoing);
}

#[cfg(test)]
fn commit_frame(latest: &mut LatestFrames, frame: BuildingFrame, land: bool, sequence: u64) {
    let (census, complete) = materialise_frame(frame, sequence, land);
    drop(commit_complete(latest, census, complete, land));
}

fn materialise_frame(
    frame: BuildingFrame,
    sequence: u64,
    land: bool,
) -> (FrameCensus, CompleteFrame) {
    let mut census = frame.census;
    if census.candidates_seen == 0 {
        census.frames_without_enumeration = 1;
        match (frame.enumeration_gate_open, frame.source_collection_len) {
            (Some(false), _) => census.frames_producer_gate_closed = 1,
            (Some(true), Some(0)) => census.frames_source_collection_empty = 1,
            _ => {}
        }
    }
    let minimum_intel = if land {
        compact_contract::spotted_units::LAND_MINIMUM_INTEL
    } else {
        compact_contract::spotted_units::TASKFORCE_MINIMUM_INTEL
    };
    let mut rows_certified = true;
    let rows: std::sync::Arc<[SpottedUnitHandle]> = frame
        .rows
        .into_iter()
        .map(|((type_id, stable_id), raw_intel)| {
            let row = SpottedUnitHandle {
                type_id,
                stable_id,
                raw_intel,
            };
            rows_certified &=
                compact_contract::spotted_units::row_in_admitted_range(&row, minimum_intel);
            row
        })
        .collect();
    let complete = CompleteFrame {
        sequence,
        pov_country_id: frame.pov_country_id,
        producer_hour_raw: frame.producer_hour_raw,
        rows,
        rows_certified,
        served: false,
    };
    (census, complete)
}

/// caller drops it after the lock is released — its row buffer is the one
fn commit_complete(
    latest: &mut LatestFrames,
    mut census: FrameCensus,
    complete: CompleteFrame,
    land: bool,
) -> Option<CompleteFrame> {
    let index = if land { CENSUS_LAND } else { CENSUS_TASKFORCE };
    let slot = if land {
        &mut latest.land
    } else {
        &mut latest.taskforces
    };
    if let Some(outgoing) = slot.as_ref() {
        if !outgoing.served && !outgoing.rows.is_empty() {
            census.admitted_rows_lost_to_replacement = outgoing.rows.len() as u64;
        }
    }
    latest.census[index].accumulate(&census);
    slot.replace(complete)
}

fn read_handle(unit: usize, raw_intel: u8) -> Option<SpottedUnitHandle> {
    let packed = read_u64(unit.checked_add(0x18)?).ok()?;
    let type_id = packed as u32;
    let stable_id = (packed >> 32) as u32;
    (type_id != 0 && stable_id != 0).then_some(SpottedUnitHandle {
        type_id,
        stable_id,
        raw_intel,
    })
}

fn current_game_hour_raw() -> Option<i32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let gamestate = read_usize(base.checked_add(GAMESTATE_PTR_RVA)?).ok()?;
    read_i32(gamestate.checked_add(GAMESTATE_CURRENT_DATE)?).ok()
}

fn current_pov_country_id() -> Option<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let gamestate = read_usize(base.checked_add(GAMESTATE_PTR_RVA)?).ok()?;
    let primary = read_u32(gamestate.checked_add(0x520)?).ok()?;
    let id = if primary > 0 {
        primary
    } else {
        read_u32(gamestate.checked_add(0x524)?).ok()?
    };
    (id > 0 && id <= MAX_POV_COUNTRY_ID).then_some(id)
}

fn next_sequence() -> Option<u64> {
    let value = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    (value != 0).then_some(value)
}

#[must_use]
fn rollback(created: &[usize]) -> bool {
    let mut batch = hook_batch(created);
    batch.reverse();
    let retained = !unsafe { inline_detour::disable_batch(&batch) }.all_succeeded();
    if retained {
        RETAINED_LIVE.store(true, Ordering::Release);
        return true;
    }
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
    let _ = unsafe { inline_detour::remove_batch(&batch) };
    false
}
fn clear_originals() {
    ORIGINAL_GETTER.store(0, Ordering::Release);
    ORIGINAL_LAND_FRAME.store(0, Ordering::Release);
    ORIGINAL_LAND_ELIGIBILITY.store(0, Ordering::Release);
    ORIGINAL_TASKFORCE_FRAME.store(0, Ordering::Release);
}
fn original_fn<T>(address: usize) -> Option<T>
where
    T: Copy,
{
    (address != 0).then(|| unsafe { mem::transmute_copy(&address) })
}
fn read_u8(address: usize) -> Result<u8, String> {
    let mut b = [0; 1];
    read_exact(address, &mut b)?;
    Ok(b[0])
}
fn read_u32(address: usize) -> Result<u32, String> {
    let mut b = [0; 4];
    read_exact(address, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_i32(address: usize) -> Result<i32, String> {
    let mut b = [0; 4];
    read_exact(address, &mut b)?;
    Ok(i32::from_le_bytes(b))
}
fn read_u64(address: usize) -> Result<u64, String> {
    let mut b = [0; 8];
    read_exact(address, &mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn read_usize(address: usize) -> Result<usize, String> {
    let mut b = [0; mem::size_of::<usize>()];
    read_exact(address, &mut b)?;
    Ok(usize::from_le_bytes(b))
}
fn read_exact(address: usize, out: &mut [u8]) -> Result<(), String> {
    if address == 0 {
        return Err("spotted-unit read from null".into());
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
        Err(format!("spotted-unit read failed at {address:#x}"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_sequence_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn squeeze(text: &str) -> String {
        let packed: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        packed.replace(",)", ")")
    }

    fn production_code() -> String {
        let source = include_str!("a729.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("a729.rs has a test module")
            .0
            .to_string()
    }

    /// cannot restore safely, so "the release failed" and "the detour is gone"
    #[test]
    fn no_path_clears_the_originals_while_a_detour_may_still_be_live() {
        let production = production_code();
        let clears = production.matches("clear_originals()").count();
        assert!(
            clears >= 2,
            "expected the teardown and rollback clears to still exist, found {clears}"
        );
        assert_eq!(
            production
                .matches("let retained = rollback(&created);")
                .count(),
            1,
            "the receipt-failure rollback must keep the retention answer"
        );
        assert!(
            production.contains(
                "let retained = rollback(&created);\n        if !retained {\n            \
                 clear_originals();"
            ),
            "the receipt-failure path's `clear_originals()` must sit behind its \
             retention answer, like every other clear in this module"
        );
        assert_eq!(
            production
                .matches("HOOK_RECEIPTS.discard_all_custody();")
                .count(),
            1,
            "there is exactly one receipt discard, on the failed-install path"
        );
        let discard_block = production
            .split_once("let retained = rollback(&created);")
            .expect("the receipt-failure rollback is declared in production")
            .1
            .split_once("\n        }")
            .expect("the retention branch is closed")
            .0;
        assert!(
            discard_block.contains("HOOK_RECEIPTS.discard_all_custody();"),
            "the receipt custody may only be discarded once the rollback has \
             PROVED the detours are gone; a retained detour keeps both records"
        );
        for (guarded, count) in [
            ("if !rollback(&created) {", 6usize),
            ("if !retained {", 2usize),
            ("if retained {\n        RETAINED_LIVE.store(true, Ordering::Release);\n    } else {\n        clear_originals();\n    }", 1usize),
        ] {
            assert_eq!(
                production.matches(guarded).count(),
                count,
                "every `clear_originals()` must sit behind a retention check;                  `{guarded}` no longer appears {count} times"
            );
        }
        let before_rollback = production
            .split_once("fn rollback(")
            .expect("rollback is declared in production")
            .0;
        assert!(
            before_rollback.trim_end().ends_with("#[must_use]"),
            "rollback's retention answer must be impossible to drop on the floor"
        );
    }

    /// Law: "the release failed" is decided by what is still PATCHED, not by
    /// the old `already_released()` test and remains broader than asking only
    #[test]
    fn retention_is_decided_by_what_is_still_patched_not_by_who_refused() {
        let production = production_code();
        assert_eq!(
            production.matches(".all_succeeded();").count(),
            2,
            "both the rollback and the teardown release loops must decide \
             retention from the complete batch outcome"
        );
        assert!(
            !production.contains("status.detour_retained()"),
            "`detour_retained()` is narrower than the question being asked: it \
             says the FREEZE refused, not that the target is still patched"
        );
    }

    #[test]
    fn a_retained_release_permanently_refuses_the_next_install() {
        let production = production_code();
        assert_eq!(
            production
                .matches("RETAINED_LIVE.store(true, Ordering::Release)")
                .count(),
            2,
            "both retention sites -- rollback and teardown -- must set the latch"
        );
        assert!(
            !production.contains("RETAINED_LIVE.store(false"),
            "the latch is one-way; there is no state in which reinstalling is \
             safe again"
        );
        let install = production
            .split_once("pub fn install()")
            .expect("install is declared in production")
            .1
            .split_once("let base = unsafe")
            .expect("install resolves the module base")
            .0;
        assert!(
            install.contains("RETAINED_LIVE.load(Ordering::Acquire)"),
            "install must refuse BEFORE it touches a target that may still be \
             hooked"
        );
    }

    #[test]
    fn the_receipt_specs_match_the_installers_own_site_table() {
        let expected: [(u32, &[u8]); HOOK_RECEIPT_COUNT] = [
            (INTEL_GETTER_RVA, &GETTER_SENTINEL),
            (LAND_FRAME_RVA, &LAND_FRAME_SENTINEL),
            (LAND_ELIGIBILITY_RVA, &LAND_ELIGIBILITY_SENTINEL),
            (TASKFORCE_FRAME_RVA, &TASKFORCE_FRAME_SENTINEL),
        ];
        for (index, (rva, sentinel)) in expected.iter().enumerate() {
            let spec = PRODUCTION_HOOK_SPECS[index];
            assert_eq!(
                spec.requested_rva,
                u64::from(*rva),
                "spec {index} ({}) must pin the RVA the installer patches",
                spec.logical_name
            );
            assert_eq!(
                spec.original_prefix, *sentinel,
                "spec {index} ({}) must carry the same sixteen bytes the \
                 installer's own sentinel check reads",
                spec.logical_name
            );
            assert_eq!(
                spec.target_patch_len, 5,
                "MinHook writes a five-byte REL5 jump"
            );
            assert_eq!(spec.primitive_id, MINHOOK_NORMAL_REL5_PRIMITIVE_ID);
            assert!(spec.logical_name.starts_with("gameplay.d95."));
        }
        let mut names = PRODUCTION_HOOK_SPECS
            .iter()
            .map(|spec| spec.logical_name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            HOOK_RECEIPT_COUNT,
            "every D95 receipt name must be distinct"
        );
        let mut rvas = PRODUCTION_HOOK_SPECS
            .iter()
            .map(|spec| spec.requested_rva)
            .collect::<Vec<_>>();
        rvas.sort_unstable();
        rvas.dedup();
        assert_eq!(
            rvas.len(),
            HOOK_RECEIPT_COUNT,
            "every D95 site must be distinct"
        );
    }

    /// THIS TEST FAILED IN `--release` AND PASSED IN DEBUG until 2026-08-05.
    /// site's index and this fails in `--release` (two bodies become identical
    #[test]
    fn every_site_has_its_own_detour() {
        let detours = detours();
        assert!(detours.iter().all(|detour| detour.address != 0));
        let mut distinct = detours.iter().map(|d| d.address).collect::<Vec<_>>();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            HOOK_RECEIPT_COUNT,
            "each D95 site needs its own detour"
        );
        for (index, spec) in PRODUCTION_HOOK_SPECS.iter().enumerate() {
            assert_eq!(
                detours[index].logical_name, spec.logical_name,
                "the detour table must name the same sites as the spec table"
            );
        }
        production_hook_receipt::bind_detours_by_identity("D95", &PRODUCTION_HOOK_SPECS, detours)
            .expect("the D95 detour roster must bind to its own specs by name");
    }

    #[test]
    fn every_detour_body_counts_its_own_entries() {
        let production = production_code();
        for site in [
            "SITE_INTEL_GETTER",
            "SITE_LAND_FRAME",
            "SITE_LAND_ELIGIBILITY",
            "SITE_TASKFORCE_FRAME",
        ] {
            assert_eq!(
                production
                    .matches(&format!("CallbackLease::enter({site})"))
                    .count(),
                1,
                "exactly one detour body may take the lease for {site}"
            );
        }
        assert_eq!(
            production.matches("CallbackLease::enter(").count(),
            HOOK_RECEIPT_COUNT,
            "every callback lease must name the site taking it"
        );
        assert!(
            production.contains("entries.fetch_add(1, Ordering::Relaxed);"),
            "the lease must record the entry; a lease that only counted the \
             global active-callback total would leave the four bodies byte \
             identical and foldable again"
        );
    }

    #[test]
    fn current_spotted_frames_are_reusable_across_speed_five_hourly_drains() {
        let _sequence_guard = capture_sequence_test_lock();
        let latest = LatestFrames {
            land: Some(CompleteFrame {
                sequence: 11,
                pov_country_id: 3,
                producer_hour_raw: Some(4_100),
                rows: std::sync::Arc::from(vec![SpottedUnitHandle {
                    type_id: 7,
                    stable_id: 9,
                    raw_intel: 250,
                }]),
                rows_certified: true,
                served: false,
            }),
            taskforces: Some(CompleteFrame {
                sequence: 12,
                pov_country_id: 3,
                producer_hour_raw: Some(4_099),
                rows: std::sync::Arc::from(vec![SpottedUnitHandle {
                    type_id: 8,
                    stable_id: 10,
                    raw_intel: 150,
                }]),
                rows_certified: true,
                served: false,
            }),
            census: Default::default(),
        };

        let first = snapshot_from_latest(&latest).unwrap().unwrap();
        let second = snapshot_from_latest(&latest).unwrap().unwrap();
        assert_eq!(first.land_frame_sequence, Some(11));
        assert_eq!(first.taskforce_frame_sequence, Some(12));
        assert_eq!(second.land_frame_sequence, Some(11));
        assert_eq!(second.taskforce_frame_sequence, Some(12));
        assert_eq!(first.land, second.land);
        assert_eq!(first.taskforces, second.taskforces);
        assert_ne!(first.capture_sequence, second.capture_sequence);
    }

    #[test]
    fn a_publication_landing_after_a_drain_is_current_at_the_next_drain() {
        let _sequence_guard = capture_sequence_test_lock();
        let land = |sequence: u64| CompleteFrame {
            sequence,
            pov_country_id: 7,
            producer_hour_raw: Some(4_242),
            rows: std::sync::Arc::from(vec![SpottedUnitHandle {
                type_id: 1,
                stable_id: sequence as u32,
                raw_intel: 250,
            }]),
            rows_certified: true,
            served: false,
        };
        let mut latest = LatestFrames {
            land: Some(land(11)),
            taskforces: None,
            census: Default::default(),
        };

        let early = snapshot_from_latest(&latest)
            .expect("the retained land frame drains")
            .expect("a complete current-POV frame");
        assert_eq!(early.land_frame_sequence, Some(11));
        assert!(
            latest.land.is_some(),
            "a drain must not consume the frame it served"
        );

        latest.land = Some(land(12));

        let next = snapshot_from_latest(&latest)
            .expect("the replaced slot drains")
            .expect("the new producer frame");
        assert_eq!(
            next.land_frame_sequence,
            Some(12),
            "a publication that lands after a drain must be current at the next \
             drain, not skipped"
        );
        assert_eq!(
            next.capture_sequence,
            early.capture_sequence + 1,
            "one drain call mints exactly one capture sequence, so the collector \
             reading once per frame keeps the sequence continuous across the \
             reorder"
        );
    }

    fn handle(stable_id: u32, raw_intel: u8) -> SpottedUnitHandle {
        SpottedUnitHandle {
            type_id: 4,
            stable_id,
            raw_intel,
        }
    }

    #[test]
    fn materialisation_certifies_exactly_what_the_row_validator_does() {
        let mut seed = 0x1234_5678_9ABC_DEF0u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        for case in 0..400 {
            let land = case % 2 == 0;
            let mut frame = new_building_frame(7);
            for _ in 0..(next() % 12) {
                frame
                    .rows
                    .insert((next() % 3, next() % 5), (next() % 256) as u8);
            }
            let (_census, complete) = materialise_frame(frame, 9, land);
            let minimum = if land {
                compact_contract::spotted_units::LAND_MINIMUM_INTEL
            } else {
                compact_contract::spotted_units::TASKFORCE_MINIMUM_INTEL
            };
            let validator = compact_contract::spotted_units::validate_surface_rows(
                if land { "land" } else { "taskforce" },
                &complete.rows,
                minimum,
            )
            .is_ok();
            assert_eq!(complete.rows_certified, validator, "case {case}");
        }
    }

    #[test]
    fn publish_time_certification_matches_the_full_validator() {
        let mut low = new_building_frame(7);
        low.rows.insert((4, 9), 249);
        let (_census, frame) = materialise_frame(low, 5, true);
        assert!(!frame.rows_certified);
        let full = SpottedUnitsSnapshot {
            capture_sequence: 1,
            pov_country_id: 7,
            land_frame_sequence: Some(5),
            taskforce_frame_sequence: None,
            land_frame_hour_raw: None,
            taskforce_frame_hour_raw: None,
            land: frame.rows.to_vec(),
            taskforces: Vec::new(),
        };
        assert!(full
            .validate()
            .unwrap_err()
            .contains("outside the admitted range"));
        let mut latest = LatestFrames::default();
        latest.land = Some(frame);
        assert!(snapshot_from_latest(&latest)
            .unwrap_err()
            .contains("outside the admitted range"));
        let mut ok = new_building_frame(7);
        ok.rows.insert((4, 9), 249);
        let (_census, frame) = materialise_frame(ok, 6, false);
        assert!(frame.rows_certified);
        let mut latest = LatestFrames::default();
        latest.taskforces = Some(frame);
        let snapshot = snapshot_from_latest(&latest).unwrap().unwrap();
        assert!(snapshot.validate().is_ok());
        assert_eq!(snapshot.taskforces.len(), 1);
    }

    #[test]
    fn every_land_candidate_is_censused_under_exactly_one_arm() {
        let cases: [(Option<SpottedUnitHandle>, FrameCensus, bool); 4] = [
            (
                None,
                FrameCensus {
                    candidates_seen: 1,
                    rejected_unreadable_handle: 1,
                    ..FrameCensus::default()
                },
                false,
            ),
            (
                Some(handle(1, 0)),
                FrameCensus {
                    candidates_seen: 1,
                    rejected_below_intel_threshold: 1,
                    ..FrameCensus::default()
                },
                false,
            ),
            (
                Some(handle(2, 249)),
                FrameCensus {
                    candidates_seen: 1,
                    rejected_below_intel_threshold: 1,
                    ..FrameCensus::default()
                },
                false,
            ),
            (
                Some(handle(3, 250)),
                FrameCensus {
                    candidates_seen: 1,
                    ..FrameCensus::default()
                },
                true,
            ),
        ];
        for (offered, expected, pending) in cases {
            let mut frame = new_building_frame(7);
            offer_land_candidate(&mut frame, offered);
            assert_eq!(frame.census, expected, "offered {offered:?}");
            assert_eq!(
                frame.pending_land.is_some(),
                pending,
                "only a candidate that cleared the engine's own threshold may \
                 reach the eligibility arm: {offered:?}"
            );
            assert!(frame.rows.is_empty(), "the getter arm never commits a row");
        }
    }

    #[test]
    fn the_eligibility_arm_decides_only_a_pending_candidate() {
        let mut admitted = new_building_frame(7);
        offer_land_candidate(&mut admitted, Some(handle(3, 255)));
        offer_land_eligibility(&mut admitted, true);
        assert_eq!(admitted.census.admitted, 1);
        assert_eq!(admitted.census.rejected_by_eligibility, 0);
        assert_eq!(admitted.rows.len(), 1);

        let mut refused = new_building_frame(7);
        offer_land_candidate(&mut refused, Some(handle(3, 255)));
        offer_land_eligibility(&mut refused, false);
        assert_eq!(refused.census.admitted, 0);
        assert_eq!(refused.census.rejected_by_eligibility, 1);
        assert!(refused.rows.is_empty());

        let mut unpaired = new_building_frame(7);
        offer_land_eligibility(&mut unpaired, false);
        assert_eq!(
            unpaired.census,
            FrameCensus::default(),
            "an eligibility call with nothing pending is not this frame's \
             candidate being rejected"
        );
    }

    #[test]
    fn every_taskforce_candidate_is_censused_under_exactly_one_arm() {
        let cases: [(bool, Option<(SpottedUnitHandle, bool)>, FrameCensus, usize); 4] = [
            (
                false,
                None,
                FrameCensus {
                    candidates_seen: 1,
                    rejected_unreadable_handle: 1,
                    ..FrameCensus::default()
                },
                0,
            ),
            (
                true,
                None,
                FrameCensus {
                    candidates_seen: 1,
                    rejected_below_intel_threshold: 1,
                    ..FrameCensus::default()
                },
                0,
            ),
            (
                true,
                Some((handle(5, 150), false)),
                FrameCensus {
                    candidates_seen: 1,
                    rejected_taskforce_state: 1,
                    ..FrameCensus::default()
                },
                0,
            ),
            (
                true,
                Some((handle(5, 150), true)),
                FrameCensus {
                    candidates_seen: 1,
                    admitted: 1,
                    ..FrameCensus::default()
                },
                1,
            ),
        ];
        for (readable, state, expected, rows) in cases {
            let mut frame = new_building_frame(7);
            offer_taskforce_candidate(&mut frame, readable, state);
            assert_eq!(
                frame.census, expected,
                "readable={readable} state={state:?}"
            );
            assert_eq!(frame.rows.len(), rows);
        }
    }

    #[test]
    fn a_producer_call_that_enumerated_nothing_is_counted_and_still_published() {
        let mut latest = LatestFrames::default();

        let mut real = new_building_frame(11);
        offer_land_candidate(&mut real, Some(handle(1, 255)));
        offer_land_eligibility(&mut real, true);
        offer_land_candidate(&mut real, Some(handle(2, 250)));
        offer_land_eligibility(&mut real, true);
        commit_frame(&mut latest, real, true, 41);
        assert_eq!(
            latest.land.as_ref().map(|frame| frame.rows.len()),
            Some(2),
            "a producer call that enumerated is stored"
        );

        let empty = new_building_frame(11);
        commit_frame(&mut latest, empty, true, 42);
        assert_eq!(
            latest
                .land
                .as_ref()
                .map(|frame| (frame.sequence, frame.rows.len())),
            Some((42, 0)),
            "an empty producer call is a publication, not a refusal"
        );
        assert_eq!(
            latest.census[CENSUS_LAND].frames_without_enumeration, 1,
            "and the wire is told the frame had nothing to enumerate"
        );
        assert_eq!(latest.census[CENSUS_LAND].candidates_seen, 2);
        assert_eq!(latest.census[CENSUS_LAND].admitted, 2);
    }

    #[test]
    fn an_empty_frame_is_charged_to_suppression_or_measured_empty_but_never_both() {
        const CASES: [(Option<bool>, Option<i32>, u64, u64); 9] = [
            (Some(false), Some(0), 1, 0),
            (Some(false), Some(7), 1, 0),
            (Some(false), None, 1, 0),
            (Some(true), Some(0), 0, 1),
            (Some(true), Some(7), 0, 0),
            (Some(true), None, 0, 0),
            (None, Some(0), 0, 0),
            (None, Some(7), 0, 0),
            (None, None, 0, 0),
        ];
        let fresh = new_building_frame(11);
        assert_eq!(
            fresh.enumeration_gate_open, None,
            "a frame starts with NO gate observation; only a hook that read the \
             gate may supply one"
        );
        assert_eq!(
            fresh.source_collection_len, None,
            "and with no source count, which is not the same as a count of zero"
        );

        for (index, (gate, source, want_closed, want_empty)) in CASES.into_iter().enumerate() {
            let mut latest = LatestFrames::default();
            let mut frame = new_building_frame(11);
            frame.enumeration_gate_open = gate;
            frame.source_collection_len = source;
            commit_frame(&mut latest, frame, false, 41 + index as u64);
            let census = latest.census[CENSUS_TASKFORCE];
            assert_eq!(
                census.frames_without_enumeration, 1,
                "gate={gate:?} source={source:?}: an empty frame is always counted as empty"
            );
            assert_eq!(
                census.frames_producer_gate_closed, want_closed,
                "gate={gate:?} source={source:?}: suppression charge"
            );
            assert_eq!(
                census.frames_source_collection_empty, want_empty,
                "gate={gate:?} source={source:?}: measured-empty charge"
            );
            assert!(
                census
                    .frames_producer_gate_closed
                    .saturating_add(census.frames_source_collection_empty)
                    <= census.frames_without_enumeration,
                "gate={gate:?} source={source:?}: the two charges must not \
                 double-count one frame"
            );
        }

        for gate in [Some(false), Some(true)] {
            let mut latest = LatestFrames::default();
            let mut enumerated = new_building_frame(11);
            enumerated.enumeration_gate_open = gate;
            enumerated.source_collection_len = Some(0);
            offer_taskforce_candidate(&mut enumerated, true, Some((handle(5, 150), true)));
            commit_frame(&mut latest, enumerated, false, 60);
            let census = latest.census[CENSUS_TASKFORCE];
            assert_eq!(
                census.frames_without_enumeration, 0,
                "gate={gate:?}: a frame that enumerated is not an empty frame"
            );
            assert_eq!(census.frames_producer_gate_closed, 0, "gate={gate:?}");
            assert_eq!(census.frames_source_collection_empty, 0, "gate={gate:?}");
            assert_eq!(census.admitted, 1, "gate={gate:?}");
        }
    }

    #[test]
    fn admitted_rows_discarded_before_any_drain_are_counted_and_served_rows_are_not() {
        let taskforce = |sequence: u64, pov: u32| CompleteFrame {
            sequence,
            pov_country_id: pov,
            producer_hour_raw: Some(4_242),
            rows: std::sync::Arc::from(vec![
                SpottedUnitHandle {
                    type_id: 1,
                    stable_id: 70,
                    raw_intel: 200,
                },
                SpottedUnitHandle {
                    type_id: 1,
                    stable_id: 71,
                    raw_intel: 200,
                },
            ]),
            rows_certified: true,
            served: false,
        };

        let mut latest = LatestFrames {
            taskforces: Some(taskforce(11, 7)),
            ..LatestFrames::default()
        };
        commit_frame(&mut latest, new_building_frame(7), false, 12);
        assert_eq!(
            latest.census[CENSUS_TASKFORCE].admitted_rows_lost_to_replacement, 2,
            "rows admitted into a frame no drain reached are lost, and the wire \
             must say so"
        );

        let mut latest = LatestFrames {
            taskforces: Some(taskforce(11, 7)),
            ..LatestFrames::default()
        };
        let snapshot = snapshot_from_latest(&latest)
            .expect("the retained task-force frame drains")
            .expect("a complete current-POV frame");
        assert_eq!(snapshot.taskforce_frame_sequence, Some(11));
        record_served(&mut latest, Some(&snapshot));
        commit_frame(&mut latest, new_building_frame(7), false, 12);
        assert_eq!(
            latest.census[CENSUS_TASKFORCE].admitted_rows_lost_to_replacement, 0,
            "a frame a drain already carried loses nothing when it is replaced"
        );

        let mut latest = LatestFrames {
            land: Some(CompleteFrame {
                sequence: 20,
                pov_country_id: 9,
                producer_hour_raw: Some(4_242),
                rows: std::sync::Arc::from(Vec::new()),
                rows_certified: true,
                served: false,
            }),
            taskforces: Some(taskforce(11, 7)),
            ..LatestFrames::default()
        };
        let snapshot = snapshot_from_latest(&latest)
            .expect("the mismatched pair drains")
            .expect("a complete current-POV frame");
        assert_eq!(
            snapshot.taskforce_frame_sequence, None,
            "the older, POV-disagreeing task-force frame is not part of the answer"
        );
        record_served(&mut latest, Some(&snapshot));
        commit_frame(&mut latest, new_building_frame(9), false, 21);
        assert_eq!(
            latest.census[CENSUS_TASKFORCE].admitted_rows_lost_to_replacement, 2,
            "a surface the drain dropped was not served, so its admitted rows \
             are still a loss when the producer replaces them"
        );

        let mut latest = LatestFrames {
            taskforces: Some(taskforce(12, 7)),
            ..LatestFrames::default()
        };
        let snapshot = snapshot_from_latest(&latest)
            .expect("the higher-numbered frame drains")
            .expect("a complete current-POV frame");
        assert_eq!(snapshot.taskforce_frame_sequence, Some(12));
        record_served(&mut latest, Some(&snapshot));
        latest.taskforces = Some(taskforce(11, 7));
        commit_frame(&mut latest, new_building_frame(7), false, 13);
        assert_eq!(
            latest.census[CENSUS_TASKFORCE].admitted_rows_lost_to_replacement, 2,
            "a frame that committed after a higher-numbered one was served is \
             still unserved, and its rows are still a loss"
        );
    }

    #[test]
    fn each_hook_reads_its_own_gate_off_the_parameter_the_engine_tests() {
        let production = squeeze(&production_code());
        for (pattern, why) in [
            (
                "let gate_open = land_icon_display_flag_open();",
                "the land hook must read the land display flag",
            ),
            (
                "let gate_open = taskforce_icon_gate_open(owner);",
                "the task-force gate is `param_1 + 0x5b8`, so it is read off `owner` unchanged",
            ),
            (
                "let source_collection_len = taskforce_source_collection_len(source);",
                "the task-force source count is `param_2 + 0x104`, so it is read off \
                 `source` unchanged",
            ),
            (
                "frame.enumeration_gate_open = gate_open;",
                "each hook must stamp the frame with the gate it read",
            ),
        ] {
            assert!(production.contains(&squeeze(pattern)), "{why}");
        }
    }

    #[test]
    fn the_taskforce_readers_read_the_engines_own_offsets() {
        const ENGINE_GATE_OFFSET: usize = 0x5b8;
        const ENGINE_COUNT_OFFSET: usize = 0x104;
        assert_eq!(
            TASKFORCE_ICON_GATE_OFFSET, ENGINE_GATE_OFFSET,
            "`0x1418ddf50` tests `*(char *)(param_1 + 0x5b8)`"
        );
        assert_eq!(
            TASKFORCE_SOURCE_COUNT_OFFSET, ENGINE_COUNT_OFFSET,
            "`0x1418ddf50` bounds its loop by `*(int *)(param_2 + 0x104)`"
        );

        let mut object = vec![0u8; 0x600];
        object[ENGINE_GATE_OFFSET] = 1;
        object[ENGINE_COUNT_OFFSET..ENGINE_COUNT_OFFSET + 4].copy_from_slice(&37i32.to_le_bytes());
        let base = object.as_ptr() as usize;

        assert_eq!(
            taskforce_icon_gate_open(base),
            Some(true),
            "a nonzero byte at owner+0x5b8 is an OPEN gate"
        );
        assert_eq!(
            taskforce_source_collection_len(base),
            Some(37),
            "the source count is the i32 at source+0x104"
        );

        object[ENGINE_GATE_OFFSET] = 0;
        object[ENGINE_COUNT_OFFSET..ENGINE_COUNT_OFFSET + 4].copy_from_slice(&0i32.to_le_bytes());
        assert_eq!(
            taskforce_icon_gate_open(base),
            Some(false),
            "a zero byte at owner+0x5b8 is a CLOSED gate"
        );
        assert_eq!(
            taskforce_source_collection_len(base),
            Some(0),
            "a zero count is a real observation, not an absence"
        );

        assert_eq!(
            taskforce_icon_gate_open(0),
            None,
            "an unreadable gate must not be reported as open or closed"
        );
        assert_eq!(
            taskforce_source_collection_len(0),
            None,
            "an unreadable count must not be reported as zero"
        );
        assert_eq!(taskforce_icon_gate_open(usize::MAX), None);
        assert_eq!(taskforce_source_collection_len(usize::MAX), None);
    }

    #[test]
    fn every_offered_candidate_leaves_through_exactly_one_arm() {
        let _sequence_guard = capture_sequence_test_lock();
        let mut frame = new_building_frame(11);
        offer_land_candidate(&mut frame, None);
        offer_land_candidate(&mut frame, Some(handle(1, 249)));
        offer_land_candidate(&mut frame, Some(handle(2, 250)));
        offer_land_eligibility(&mut frame, false);
        offer_land_candidate(&mut frame, Some(handle(3, 255)));
        offer_land_eligibility(&mut frame, true);
        assert_eq!(frame.census.candidates_seen, 4);
        assert_eq!(frame.census.accounted(), 4);
        assert!(frame.pending_land.is_none());

        let mut displaced = new_building_frame(11);
        offer_land_candidate(&mut displaced, Some(handle(4, 250)));
        offer_land_candidate(&mut displaced, Some(handle(5, 250)));
        offer_land_eligibility(&mut displaced, true);
        offer_land_candidate(&mut displaced, Some(handle(6, 100)));
        assert_eq!(displaced.census.candidates_seen, 3);
        assert_eq!(displaced.census.abandoned_pending, 1);
        assert_eq!(
            displaced.census.accounted(),
            3,
            "a displaced candidate must still be accounted for"
        );

        let mut taskforce = new_building_frame(11);
        offer_taskforce_candidate(&mut taskforce, false, None);
        offer_taskforce_candidate(&mut taskforce, true, None);
        offer_taskforce_candidate(&mut taskforce, true, Some((handle(7, 150), false)));
        offer_taskforce_candidate(&mut taskforce, true, Some((handle(8, 200), true)));
        assert_eq!(taskforce.census.candidates_seen, 4);
        assert_eq!(taskforce.census.accounted(), 4);
    }

    #[test]
    fn each_surface_carries_its_own_producers_hour_across_a_reuse() {
        let _sequence_guard = capture_sequence_test_lock();
        let mut latest = LatestFrames::default();

        let mut land = new_building_frame(3);
        land.producer_hour_raw = Some(4_100);
        offer_land_candidate(&mut land, Some(handle(1, 255)));
        offer_land_eligibility(&mut land, true);
        commit_frame(&mut latest, land, true, 11);

        let mut taskforce = new_building_frame(3);
        taskforce.producer_hour_raw = Some(4_099);
        offer_taskforce_candidate(&mut taskforce, true, Some((handle(2, 200), true)));
        commit_frame(&mut latest, taskforce, false, 12);

        let first = snapshot_from_latest(&latest).unwrap().unwrap();
        assert_eq!(first.land_frame_hour_raw, Some(4_100));
        assert_eq!(first.taskforce_frame_hour_raw, Some(4_099));

        // The next drain re-serves both retained frames. Neither may acquire
        let second = snapshot_from_latest(&latest).unwrap().unwrap();
        assert_eq!(second.land_frame_hour_raw, Some(4_100));
        assert_eq!(second.taskforce_frame_hour_raw, Some(4_099));
        assert_ne!(second.capture_sequence, first.capture_sequence);

        let mut unclocked = new_building_frame(3);
        unclocked.producer_hour_raw = None;
        offer_land_candidate(&mut unclocked, Some(handle(4, 251)));
        offer_land_eligibility(&mut unclocked, true);
        commit_frame(&mut latest, unclocked, true, 13);
        let third = snapshot_from_latest(&latest).unwrap().unwrap();
        assert_eq!(third.land_frame_hour_raw, None);
        assert_eq!(third.taskforce_frame_hour_raw, Some(4_099));
        third
            .encode()
            .expect("an unclocked producer frame still encodes");
    }

    #[test]
    fn a_candidates_surface_is_the_producer_it_was_called_from() {
        let production = production_code();
        assert!(
            production.contains("CURRENT_SURFACE.with(Cell::get)"),
            "the getter must attribute a candidate by the producer it is inside"
        );
        for setter in [
            "CURRENT_SURFACE.with(|slot| slot.replace(Some(CENSUS_LAND)))",
            "CURRENT_SURFACE.with(|slot| slot.replace(Some(CENSUS_TASKFORCE)))",
        ] {
            assert_eq!(
                production.matches(setter).count(),
                1,
                "exactly one producer body may claim a surface: {setter}"
            );
        }
        assert_eq!(
            production
                .matches("CURRENT_SURFACE.with(|slot| slot.set(prior_surface));")
                .count(),
            2,
            "each producer must restore the surface its caller owned, so a \
             nested producer's candidates are the INNER producer's"
        );
    }
}
