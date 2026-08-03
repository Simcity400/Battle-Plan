use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::rel5_roster::Rel5HookRoster;
use production_hook_receipt::{
    ExpectedRvaHook, InstallAbortDiscard, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
    MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::cell::RefCell;
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

const GETTER_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x30,
    0x80, 0x3d, 0x9b, 0x09, 0xd1, 0x01, 0x00, 0x48,
];
const LAND_FRAME_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x48, 0x08, 0x53,
    0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
];
const LAND_ELIGIBILITY_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x83,
    0xb9, 0x88, 0x00, 0x00, 0x00, 0x00, 0x74, 0x3e,
];
const TASKFORCE_FRAME_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x44,
    0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];

/// The canonical roster admits exactly these four D95 sites.
pub const HOOK_RECEIPT_COUNT: usize = 4;

/// The exact production-hook specifications this lane may install, in the same
/// order as the `specs` table `install` walks and as the `detours` array
/// `observe_installed` is handed. Every field is checked by
/// `production_hook_receipt` against the running image before a receipt is
/// minted: an unadmitted primitive, a patch width other than five, or an
/// original prefix that is not this exact a729 function's first sixteen bytes
/// is refused outright.
///
/// The `original_prefix` of each entry is the SAME constant the installer's own
/// sentinel check reads, so the byte gate and the receipt cannot describe
/// different functions.
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
/// never be reasoned about as clean again.
///
/// Deliberately one-way, like the module pin: nothing clears it. `INSTALLED`
/// goes false on a retained teardown, which on its own would let the next
/// `install` walk into `create_hook` on a target that is still hooked, take
/// the `MH_ERROR_ALREADY_CREATED` arm, and clear the originals underneath the
/// live detour -- or spin forever draining callbacks that detour is still
/// producing. Refusing to install again is the only outcome that stays true.
static RETAINED_LIVE: AtomicBool = AtomicBool::new(false);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static LATEST: OnceLock<Mutex<LatestFrames>> = OnceLock::new();
static LIFECYCLE: Mutex<()> = Mutex::new(());

#[derive(Clone)]
struct CompleteFrame {
    sequence: u64,
    pov_country_id: u32,
    rows: Vec<SpottedUnitHandle>,
}

#[derive(Default)]
struct LatestFrames {
    land: Option<CompleteFrame>,
    taskforces: Option<CompleteFrame>,
}

struct BuildingFrame {
    pov_country_id: u32,
    rows: BTreeMap<(u32, u32), u8>,
    pending_land: Option<SpottedUnitHandle>,
}

thread_local! {
    static LAND_BUILD: RefCell<Option<BuildingFrame>> = const { RefCell::new(None) };
    static TASKFORCE_BUILD: RefCell<Option<BuildingFrame>> = const { RefCell::new(None) };
}

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

pub fn install() -> Result<(), String> {
    let _guard = LIFECYCLE.lock().map_err(|_| "spotted-unit hook lifecycle poisoned")?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    // A previous release left a detour live. `INSTALLED` is false, so without
    // this the installer below would run against sites that are still hooked:
    // `create_hook` answers MH_ERROR_ALREADY_CREATED, the failure arm calls
    // `rollback(&[])` -- which has nothing to disable, so it reports no
    // retention -- and the originals are cleared under a live detour, or the
    // callback drain never finishes because that detour is still producing
    // callbacks.
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
        (INTEL_GETTER_RVA, &GETTER_SENTINEL, hook_intel_getter as *mut c_void, &ORIGINAL_GETTER),
        (LAND_FRAME_RVA, &LAND_FRAME_SENTINEL, hook_land_frame as *mut c_void, &ORIGINAL_LAND_FRAME),
        (LAND_ELIGIBILITY_RVA, &LAND_ELIGIBILITY_SENTINEL, hook_land_eligibility as *mut c_void, &ORIGINAL_LAND_ELIGIBILITY),
        (TASKFORCE_FRAME_RVA, &TASKFORCE_FRAME_SENTINEL, hook_taskforce_frame as *mut c_void, &ORIGINAL_TASKFORCE_FRAME),
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
            return Err(format!("spotted-unit hook sentinel mismatch at RVA {rva:#x}"));
        }
        let original = match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(value) => value,
            Err(status) => {
                if !rollback(&created) {
                    clear_originals();
                }
                return Err(format!("spotted-unit hook creation failed at RVA {rva:#x}: {status}"));
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
    for target in &created {
        if let Err(status) = unsafe { inline_detour::enable(*target as *mut c_void) } {
            if !rollback(&created) {
                clear_originals();
            }
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("spotted-unit hook enable failed: {status}"));
        }
    }
    // Fail closed on identity, exactly as D94 does. The observation re-derives
    // the module from the running process, refuses anything that is not an
    // exact-a729 `hoi4.exe`, re-checks each admitted original prefix against
    // that file's own bytes at the site's RVA, and binds each installed relay to
    // the detour this lane actually passed to MinHook. An installation that
    // cannot be attested is rolled back rather than shipped: without a receipt
    // the production hook registry has nothing to put on the wire, and a
    // silently receipt-less install is how a domain reports itself installed
    // while the READY evidence describes no patch at all.
    if let Err(error) = HOOK_RECEIPTS.observe_installed(base, detours()) {
        let retained = rollback(&created);
        if !retained {
            clear_originals();
            // Only once the detours are provably gone. A retained detour is
            // still in the executable, and the partial custody `observe_installed`
            // persisted for the sites it DID attest is evidence that a patch
            // exists -- discarding it would delete that. `RETAINED_LIVE` is the
            // primary record and is one-way, but the two are independent
            // answers on purpose and neither is thrown away while the other
            // could be wrong.
            HOOK_RECEIPTS.discard_all_custody();
        }
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("spotted-unit hook receipt refused: {error}"));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

/// The detour addresses, in `PRODUCTION_HOOK_SPECS` order.
///
/// A function rather than a `const`: a function item's address is not an
/// integer at compile time, so this cannot be a constant. `observe_installed`
/// refuses any site whose relay does not resolve to the address in this list,
/// so a mis-ordered entry is a hard install failure rather than a mislabelled
/// receipt.
fn detours() -> [usize; HOOK_RECEIPT_COUNT] {
    [
        hook_intel_getter as *const () as usize,
        hook_land_frame as *const () as usize,
        hook_land_eligibility as *const () as usize,
        hook_taskforce_frame as *const () as usize,
    ]
}

/// True once this lane owns enabled, receipt-observed production hooks.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// True while this lane holds a release it could not PROVE.
///
/// Two independent ways that happens, and `installed()` reports false for both:
///
///   * `RETAINED_LIVE` -- a disable that did not take. The detour is still in
///     the executable and the latch is one-way, so this stays true forever.
///   * receipt custody still on the installed side. `uninstall` publishes the
///     release only when every disable AND every removal came back clean, so an
///     uninstall that errored leaves the installed receipts in place with
///     `INSTALLED` already false.
pub fn release_pending() -> bool {
    RETAINED_LIVE.load(Ordering::Acquire)
        || (!installed() && HOOK_RECEIPTS.may_still_hold_installed_custody())
}

/// The exact enabled-byte receipts for the four D95 production hook sites.
pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.installed_receipts()
}

/// Consume the release receipts. One-shot: a second call after the registry has
/// taken custody reports them unavailable rather than minting a second release.
pub fn take_released_hook_receipts(
) -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.take_released_receipts()
}

/// Release this lane for a production install that is being given up, and
/// report what the abort threw away.
///
/// D95 keeps ONE current land frame and ONE current taskforce frame rather than
/// a queue, and they are complete map-visibility frames rather than events, so
/// the discard total is at most two retained observations. They are drained
/// BEFORE the uninstall: after the detours are gone `drain_latest` refuses and
/// the evidence is unrecoverable.
pub fn abort_partial_install_and_uninstall() -> Result<InstallAbortDiscard, String> {
    let retained = match drain_latest() {
        Ok(Some(_)) => 1,
        Ok(None) => 0,
        // "Not installed" means there was nothing to retain. Any other refusal
        // is an answer we did not get, so it counts as a retained observation
        // rather than as silence.
        Err(error) if error.contains("are not installed") => 0,
        Err(_) => 1,
    };
    uninstall().map_err(|error| format!("D95 install abort: {error}"))?;
    // `uninstall` returns success whenever INSTALLED was already false, which is
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
    let latest = LATEST.get().ok_or("spotted-unit hook state unavailable")?
        .lock().map_err(|_| "spotted-unit latest frames poisoned")?;
    snapshot_from_latest(&latest)
}

fn snapshot_from_latest(latest: &LatestFrames) -> Result<Option<SpottedUnitsSnapshot>, String> {
    // These are complete current map-visibility frames, not one-shot events.
    // A speed-5 game can advance several hourly collector boundaries between
    // renderer producer calls, so retain and reuse the last authenticated
    // frame until its producer replaces it. Consuming the slots here created
    // false source-unavailable/coverage holes on every intervening hour.
    let mut land = latest.land.clone();
    let mut taskforces = latest.taskforces.clone();
    if let (Some(a), Some(b)) = (&land, &taskforces) {
        if a.pov_country_id != b.pov_country_id {
            if a.sequence > b.sequence { taskforces = None; } else { land = None; }
        }
    }
    let pov_country_id = land.as_ref().map(|frame| frame.pov_country_id)
        .or_else(|| taskforces.as_ref().map(|frame| frame.pov_country_id));
    let Some(pov_country_id) = pov_country_id else { return Ok(None); };
    let capture_sequence = next_sequence().ok_or("spotted-unit capture sequence exhausted")?;
    let snapshot = SpottedUnitsSnapshot {
        capture_sequence,
        pov_country_id,
        land_frame_sequence: land.as_ref().map(|frame| frame.sequence),
        taskforce_frame_sequence: taskforces.as_ref().map(|frame| frame.sequence),
        land: land.map(|frame| frame.rows).unwrap_or_default(),
        taskforces: taskforces.map(|frame| frame.rows).unwrap_or_default(),
    };
    snapshot.validate()?;
    Ok(Some(snapshot))
}

pub fn uninstall() -> Result<(), String> {
    let _guard = LIFECYCLE.lock().map_err(|_| "spotted-unit hook lifecycle poisoned")?;
    if !INSTALLED.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let base = MODULE_BASE.swap(0, Ordering::AcqRel);
    let mut errors = Vec::new();
    // A detour the process could not be frozen for is still LIVE. The wait
    // below is for callbacks to drain, and a live detour keeps admitting them,
    // so entering it after a retained disable spins for the life of the
    // process. Track retention and skip both the wait and the removal.
    let mut retained = false;
    for rva in [TASKFORCE_FRAME_RVA, LAND_ELIGIBILITY_RVA, LAND_FRAME_RVA, INTEL_GETTER_RVA] {
        let target = base.saturating_add(rva as usize) as *mut c_void;
        match unsafe { inline_detour::disable(target) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                // Same rule as `rollback`: every status that is not one of the
                // two "nothing was patched" answers leaves the target patched.
                retained |= !status.already_released();
                errors.push(format!("spotted-unit hook disable failed: {status}"));
            }
        }
    }
    if !retained {
        while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
        }
        for rva in [TASKFORCE_FRAME_RVA, LAND_ELIGIBILITY_RVA, LAND_FRAME_RVA, INTEL_GETTER_RVA] {
            let target = base.saturating_add(rva as usize) as *mut c_void;
            match unsafe { inline_detour::remove(target) } {
                Ok(())
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                Err(status) => errors.push(format!("spotted-unit hook removal failed: {status}")),
            }
        }
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
    // refuses unless the admitted original prefix is back, so this cannot
    // attest a restoration that did not happen -- but a retained detour must not
    // even be offered to it, because the attestation would then be about the one
    // site that came back while three others stayed patched.
    //
    // `relay_freed: true` is the truth of the branch above: this path calls
    // `inline_detour::remove`, which is MinHook's `MH_RemoveHook` and frees the
    // trampoline. The retained branch calls only `disable` and mints nothing.
    //
    // Receipt custody is published only when the physical release ALSO reported
    // no errors. A removal failure leaves MinHook state this lane cannot
    // describe, and a release receipt is the registry's evidence that the site
    // is closed.
    if !retained && errors.is_empty() {
        if let Err(error) = HOOK_RECEIPTS.publish_release(true) {
            errors.push(format!("spotted-unit hook release receipt refused: {error}"));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

unsafe extern "system" fn hook_land_frame(owner: usize) {
    let _lease = CallbackLease::enter();
    let original: LandFrameFn = match original_fn(ORIGINAL_LAND_FRAME.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return,
    };
    let prior = LAND_BUILD.with(|slot| slot.borrow_mut().take());
    let frame = current_pov_country_id().map(new_building_frame);
    LAND_BUILD.with(|slot| *slot.borrow_mut() = frame);
    original(owner);
    let complete = LAND_BUILD.with(|slot| slot.borrow_mut().take());
    LAND_BUILD.with(|slot| *slot.borrow_mut() = prior);
    if let Some(frame) = complete { publish(frame, true); }
}

unsafe extern "system" fn hook_taskforce_frame(owner: usize, source: usize, context: usize) {
    let _lease = CallbackLease::enter();
    let original: TaskforceFrameFn = match original_fn(ORIGINAL_TASKFORCE_FRAME.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return,
    };
    let prior = TASKFORCE_BUILD.with(|slot| slot.borrow_mut().take());
    let debug_override = MODULE_BASE.load(Ordering::Acquire)
        .checked_add(NAVY_DEBUG_OVERRIDE_RVA)
        .and_then(|address| read_u8(address).ok())
        .unwrap_or(1);
    let frame = if debug_override == 0 { current_pov_country_id().map(new_building_frame) } else { None };
    TASKFORCE_BUILD.with(|slot| *slot.borrow_mut() = frame);
    original(owner, source, context);
    let complete = TASKFORCE_BUILD.with(|slot| slot.borrow_mut().take());
    TASKFORCE_BUILD.with(|slot| *slot.borrow_mut() = prior);
    if let Some(frame) = complete { publish(frame, false); }
}

unsafe extern "system" fn hook_intel_getter(unit: usize) -> u32 {
    let _lease = CallbackLease::enter();
    let original: IntelGetterFn = match original_fn(ORIGINAL_GETTER.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return 0,
    };
    let raw = original(unit) & 0xff;
    let handle = read_handle(unit, raw as u8);
    LAND_BUILD.with(|slot| {
        if let Some(frame) = slot.borrow_mut().as_mut() {
            frame.pending_land = handle.filter(|row| row.raw_intel >= 250);
        }
    });
    if let Some(row) = handle.filter(|row| row.raw_intel >= 150) {
        let taskforce_admitted = read_i32(unit.saturating_add(0x20c)).ok().is_some_and(|value| value < 1);
        if taskforce_admitted {
            TASKFORCE_BUILD.with(|slot| {
                if let Some(frame) = slot.borrow_mut().as_mut() { insert_row(frame, row); }
            });
        }
    }
    raw
}

unsafe extern "system" fn hook_land_eligibility(wrapper: usize) -> u8 {
    let _lease = CallbackLease::enter();
    let original: LandEligibilityFn = match original_fn(ORIGINAL_LAND_ELIGIBILITY.load(Ordering::Acquire)) {
        Some(value) => value,
        None => return 0,
    };
    let admitted = original(wrapper);
    LAND_BUILD.with(|slot| {
        if let Some(frame) = slot.borrow_mut().as_mut() {
            let pending = frame.pending_land.take();
            if admitted != 0 {
                if let Some(row) = pending { insert_row(frame, row); }
            }
        }
    });
    admitted
}

fn new_building_frame(pov_country_id: u32) -> BuildingFrame {
    BuildingFrame { pov_country_id, rows: BTreeMap::new(), pending_land: None }
}

fn insert_row(frame: &mut BuildingFrame, row: SpottedUnitHandle) {
    frame.rows.entry((row.type_id, row.stable_id))
        .and_modify(|value| *value = (*value).max(row.raw_intel))
        .or_insert(row.raw_intel);
}

fn publish(frame: BuildingFrame, land: bool) {
    let Some(sequence) = next_sequence() else { return; };
    let rows = frame.rows.into_iter().map(|((type_id, stable_id), raw_intel)| {
        SpottedUnitHandle { type_id, stable_id, raw_intel }
    }).collect();
    let complete = CompleteFrame { sequence, pov_country_id: frame.pov_country_id, rows };
    if let Some(latest) = LATEST.get() {
        if let Ok(mut slot) = latest.lock() {
            if land { slot.land = Some(complete); } else { slot.taskforces = Some(complete); }
        }
    }
}

fn read_handle(unit: usize, raw_intel: u8) -> Option<SpottedUnitHandle> {
    let packed = read_u64(unit.checked_add(0x18)?).ok()?;
    let type_id = packed as u32;
    let stable_id = (packed >> 32) as u32;
    (type_id != 0 && stable_id != 0).then_some(SpottedUnitHandle { type_id, stable_id, raw_intel })
}

fn current_pov_country_id() -> Option<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let gamestate = read_usize(base.checked_add(GAMESTATE_PTR_RVA)?).ok()?;
    let primary = read_u32(gamestate.checked_add(0x520)?).ok()?;
    let id = if primary > 0 { primary } else { read_u32(gamestate.checked_add(0x524)?).ok()? };
    (id > 0 && id <= MAX_POV_COUNTRY_ID).then_some(id)
}

fn next_sequence() -> Option<u64> {
    let value = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    (value != 0).then_some(value)
}

/// Undo a partial installation. Returns TRUE when a detour is still live
/// because the process could not be frozen to restore it.
///
/// A `true` here forbids the caller from clearing the originals. A live detour
/// whose `ORIGINAL_*` slot reads zero returns WITHOUT calling the engine's own
/// function, which silently changes what the game computes -- a worse outcome
/// than the leaked trampoline this path already accepts.
#[must_use]
fn rollback(created: &[usize]) -> bool {
    // Same rule as `uninstall`: a retained detour is still admitting
    // callbacks, so the drain below would never finish. Leaving the
    // trampolines allocated is the strictly better failure -- the module is
    // pinned against unload, so a poised entrant keeps running real code.
    let mut retained = false;
    for target in created.iter().rev() {
        if let Err(status) = unsafe { inline_detour::disable(*target as *mut c_void) } {
            // NOT `detour_retained()`, which answers only "the freeze was
            // refused". `MH_DisableHook` also returns MH_ERROR_MEMORY_PROTECT
            // and MH_ERROR_MEMORY_ALLOC with the target STILL PATCHED, and
            // reading those as a release is how the drain below starts
            // spinning against a detour that is still admitting callbacks.
            // Only the two statuses that mean there was nothing patched to
            // begin with are a release.
            retained |= !status.already_released();
        }
    }
    if retained {
        RETAINED_LIVE.store(true, Ordering::Release);
        return true;
    }
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        std::thread::yield_now();
    }
    for target in created.iter().rev() {
        let _ = unsafe { inline_detour::remove(*target as *mut c_void) };
    }
    false
}
fn clear_originals() {
    ORIGINAL_GETTER.store(0, Ordering::Release);
    ORIGINAL_LAND_FRAME.store(0, Ordering::Release);
    ORIGINAL_LAND_ELIGIBILITY.store(0, Ordering::Release);
    ORIGINAL_TASKFORCE_FRAME.store(0, Ordering::Release);
}
fn original_fn<T>(address: usize) -> Option<T> where T: Copy {
    (address != 0).then(|| unsafe { mem::transmute_copy(&address) })
}
fn read_u8(address: usize) -> Result<u8, String> { let mut b = [0; 1]; read_exact(address, &mut b)?; Ok(b[0]) }
fn read_u32(address: usize) -> Result<u32, String> { let mut b = [0; 4]; read_exact(address, &mut b)?; Ok(u32::from_le_bytes(b)) }
fn read_i32(address: usize) -> Result<i32, String> { let mut b = [0; 4]; read_exact(address, &mut b)?; Ok(i32::from_le_bytes(b)) }
fn read_u64(address: usize) -> Result<u64, String> { let mut b = [0; 8]; read_exact(address, &mut b)?; Ok(u64::from_le_bytes(b)) }
fn read_usize(address: usize) -> Result<usize, String> { let mut b = [0; mem::size_of::<usize>()]; read_exact(address, &mut b)?; Ok(usize::from_le_bytes(b)) }
fn read_exact(address: usize, out: &mut [u8]) -> Result<(), String> {
    if address == 0 { return Err("spotted-unit read from null".into()); }
    let mut read = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), address as *const c_void, out.as_mut_ptr().cast(), out.len(), &mut read) };
    if ok == 0 || read != out.len() { Err(format!("spotted-unit read failed at {address:#x}")) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The production text of this module, up to its test module.
    ///
    /// Line endings are normalised because the scans below match
    /// multi-line shapes, and git hands this file back with CRLF on a
    /// fresh checkout. Without this the guards go red on a clean tree
    /// and green on the tree that wrote them -- the worst direction.
    fn production_code() -> String {
        let source = include_str!("a729.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("a729.rs has a test module")
            .0
            .to_string()
    }

    /// Law: the `ORIGINAL_*` slots are never cleared while a detour is still
    /// patched into the game.
    ///
    /// Each detour reads its slot and RETURNS WITHOUT CALLING the engine's own
    /// function when the slot is zero. So clearing the slots under a live
    /// detour does not merely leak a hook, it deletes engine behaviour from a
    /// running game and the capture keeps going as though nothing happened --
    /// strictly worse than the leaked trampoline this teardown already
    /// accepts. `inline_detour::disable` refuses rather than restore bytes it
    /// cannot restore safely, so "the release failed" and "the detour is gone"
    /// stopped being the same thing, and every clear has to be guarded.
    ///
    /// A source scan because the guarded paths are the failure arms of an
    /// installer that only runs against a real image: there is no way to drive
    /// them offline, and asserting nothing at all would be worse. Delete any
    /// guard below and this names the file.
    #[test]
    fn no_path_clears_the_originals_while_a_detour_may_still_be_live() {
        let production = production_code();
        let clears = production.matches("clear_originals()").count();
        assert!(
            clears >= 2,
            "expected the teardown and rollback clears to still exist, found {clears}"
        );
        // The receipt observation added on 2026-08-03 is a seventh way
        // `install` can fail after sites have been created, and it rolls back
        // through the same retention check. It binds the answer first --
        // `let retained = rollback(&created);` -- because it needs it twice: to
        // guard the `clear_originals()` and to decide whether the receipt
        // custody may be discarded. So the count below stays at the original
        // six and the seventh site is asserted in its own bound form.
        assert_eq!(
            production.matches("let retained = rollback(&created);").count(),
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
        for (guarded, count) in [
            ("if !rollback(&created) {", 6usize),
            // Two: the teardown's own retention branch, and the receipt-failure
            // rollback added on 2026-08-03.
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
    /// which component refused.
    ///
    /// `detour_retained()` answers only "the freeze was refused".
    /// `MH_DisableHook` also returns MH_ERROR_MEMORY_PROTECT and
    /// MH_ERROR_MEMORY_ALLOC with the target still patched, so keying
    /// retention on `detour_retained()` reports a live detour as released --
    /// and then the callback drain spins against a detour that is still
    /// producing callbacks, and the originals are cleared under it. Only
    /// `already_released()` -- MH_ERROR_DISABLED and MH_ERROR_NOT_CREATED,
    /// the two answers that mean nothing was patched -- is a release.
    ///
    /// A source scan for the same reason as the law above: these are the
    /// failure arms of a teardown that only runs against a real image.
    #[test]
    fn retention_is_decided_by_what_is_still_patched_not_by_who_refused() {
        let production = production_code();
        assert_eq!(
            production.matches("retained |= !status.already_released();").count(),
            2,
            "both the rollback and the teardown release loops must decide \
             retention from `already_released()`"
        );
        assert!(
            !production.contains("status.detour_retained()"),
            "`detour_retained()` is narrower than the question being asked: it \
             says the FREEZE refused, not that the target is still patched"
        );
    }

    /// Law: a unit that retained a live detour never installs again.
    ///
    /// A retained teardown clears `INSTALLED` but leaves the sites hooked. The
    /// next `install` would then reach `create_hook` on an already-hooked
    /// target, take the MH_ERROR_ALREADY_CREATED arm, call `rollback(&[])` --
    /// which has nothing to disable and therefore reports no retention -- and
    /// clear the originals under the live detour, or spin forever draining
    /// callbacks that detour is still producing. The latch is one-way for the
    /// same reason the module pin is: there is no state in which it becomes
    /// safe again.
    #[test]
    fn a_retained_release_permanently_refuses_the_next_install() {
        let production = production_code();
        assert_eq!(
            production.matches("RETAINED_LIVE.store(true, Ordering::Release)").count(),
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

    /// Law: the receipt roster describes the sites the installer actually
    /// patches, in the order the installer patches them.
    ///
    /// The two tables are independent literals -- `install` walks its own
    /// `specs` array and `PRODUCTION_HOOK_SPECS` is what the receipts and the
    /// pinned allowlist are built from -- so nothing but this holds them
    /// together. A drift here is not a cosmetic mismatch: `observe_installed`
    /// is handed `detours()` positionally, so a reordered spec table would bind
    /// each receipt to the wrong detour and the whole install would be refused
    /// live, on a real image, with no offline signal.
    ///
    /// Mutation proof: swap two entries of `PRODUCTION_HOOK_SPECS`, or change
    /// one `original_prefix` to a different sentinel, and this fails.
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
            assert_eq!(spec.target_patch_len, 5, "MinHook writes a five-byte REL5 jump");
            assert_eq!(spec.primitive_id, MINHOOK_NORMAL_REL5_PRIMITIVE_ID);
            assert!(spec.logical_name.starts_with("gameplay.d95."));
        }
        // Distinct names and distinct sites: the registry sorts and
        // de-duplicates receipts by logical name, so a duplicated name would
        // silently shrink the roster it thinks it installed.
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

    /// The detour table is the same width as the spec table and holds four
    /// distinct, non-null addresses.
    ///
    /// `observe_installed` binds `specs[i]` to `detours[i]`, so a repeated
    /// address here would mean two sites claiming the same detour -- which a
    /// live install would catch and an offline build would not.
    #[test]
    fn every_site_has_its_own_detour() {
        let detours = detours();
        assert!(detours.iter().all(|address| *address != 0));
        let mut distinct = detours.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            HOOK_RECEIPT_COUNT,
            "each D95 site needs its own detour"
        );
    }

    #[test]
    fn current_spotted_frames_are_reusable_across_speed_five_hourly_drains() {
        let latest = LatestFrames {
            land: Some(CompleteFrame {
                sequence: 11,
                pov_country_id: 3,
                rows: vec![SpottedUnitHandle {
                    type_id: 7,
                    stable_id: 9,
                    raw_intel: 250,
                }],
            }),
            taskforces: Some(CompleteFrame {
                sequence: 12,
                pov_country_id: 3,
                rows: vec![SpottedUnitHandle {
                    type_id: 8,
                    stable_id: 10,
                    raw_intel: 150,
                }],
            }),
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
}
