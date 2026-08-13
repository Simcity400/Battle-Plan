//! Signature-gated post-refresh observer for `0x1417797A0`.

use crate::{
    encode_payload, CarrierError, DesignCatalogContextFrame, DesignerFamily, DesignerViewContext,
};
use core::ffi::c_void;
use minhook::MinHook;
use production_hook_receipt::rel5_roster::Rel5HookRoster;
use production_hook_receipt::{
    ExpectedRvaHook, InstalledRvaHookReceipt, ReleasedRvaHookReceipt, SiteDetour,
    MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::cell::Cell;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const DESIGNER_REFRESH_RVA: u32 = 0x0177_97A0;
pub const COUNTRY_FROM_TAG_RVA: u32 = 0x00BA_56A0;
pub const DESIGNER_GRID_PRODUCER_RVA: u32 = 0x0177_D780;
pub const DESIGNER_ROW_BIND_RVA: u32 = 0x0149_2B70;

const REFRESH_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xD9, 0x48, 0x8B, 0x89, 0xA0, 0xF3, 0x00, 0x00,
];
/// This module owns exactly one production site.
pub const HOOK_RECEIPT_COUNT: usize = 1;

/// The post-refresh observer at `0x1417797A0`.
///
/// The logical name says `d78` because that is whose producer this is: the
/// designer view context. It travelled inside `StandardHookReceiptGroup::D85`
/// until 2026-08-13, because d85 could not produce a row without it --
/// `install_a729_selected_designer_hooks` installs this first and calls it the
/// "d78 prerequisite", since equipment family ordering is joined to this
/// refresh -- and this comment said the name would move when a D78 group
/// existed. It has: the owner descheduled domain 85
/// (`documents/DECISIONS.md`), the seven selected-designer detours left the
/// production hook registry with it, and this site stayed as
/// `StandardHookReceiptGroup::D78`'s only member because domain 78 was never
/// descheduled. The registry now calls
/// `crate::install_a729_design_catalog_context_hook` directly rather than the
/// wrapper that would drag the seven back in.
pub const PRODUCTION_HOOK_SPECS: [ExpectedRvaHook; HOOK_RECEIPT_COUNT] = [ExpectedRvaHook {
    logical_name: "gameplay.d78.designer_refresh",
    requested_rva: DESIGNER_REFRESH_RVA as u64,
    target_patch_len: 5,
    primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
    original_prefix: &REFRESH_SENTINEL,
}];

static HOOK_RECEIPTS: Rel5HookRoster<HOOK_RECEIPT_COUNT> =
    Rel5HookRoster::new("D78", PRODUCTION_HOOK_SPECS);

const VIEW_HANDLER: usize = 0x3788;
const VIEW_MODE: usize = 0x3790;
const VIEW_INTEL_CONTAINER: usize = 0xF3A0;
const VIEW_ROW_POOL_DATA: usize = 0xF3C0;
const VIEW_ROW_POOL_COUNT: usize = 0xF3CC;
const VIEW_ROW_POOL_USED: usize = 0xF3D8;
const VIEW_SELECTED_TAGS_DATA: usize = 0xF418;
const VIEW_SELECTED_TAGS_COUNT: usize = 0xF424;
const VIEW_OUR_FILTER: usize = 0x10D98;
const VIEW_FOREIGN_FILTER: usize = 0x10D99;
const VIEW_OBSOLETE_FILTER: usize = 0x10D9A;
const GUI_VISIBLE_FLAGS: usize = 0xA5;
const ROW_BOUND_VARIANT: usize = 0x560;
const ROW_VTABLE: usize = 0x029B_2268;
const VARIANT_VTABLE: usize = 0x0293_AFB8;
const VARIANT_TYPE: usize = 0x08;
const VARIANT_ID: usize = 0x0C;
const TYPE_VARIANT: u32 = 70;
const HANDLER_CURRENT_TAG_VSLOT: usize = 0xA0;
const MAX_MODULE_IMAGE_SPAN: usize = 0x0400_0000;

type RefreshFn = unsafe extern "system" fn(usize);
type CurrentTagFn = unsafe extern "system" fn(usize) -> usize;
type CountryFromTagFn = unsafe extern "system" fn(usize) -> usize;

static ORIGINAL_REFRESH: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
/// A failed installation could not PROVE it removed its detour.
///
/// `INSTALLED` is set only after the enable succeeds, so an enable failure --
/// which may still have patched the target -- leaves every other predicate
/// answering "nothing here". One-way, like D94's twin.
static FAILED_INSTALL_REMOVAL_UNPROVED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Debug, Default)]
struct HookState {
    next_transition_sequence: u64,
    latest: Option<DesignerViewContext>,
    latest_view_ptr: usize,
    latest_runtime_root_ptr: usize,
    current_error: Option<String>,
    callback_count: u64,
}

/// Process-local bridge for the independently serialized HUD-role observer.
/// The pointers never enter DCX1 and are consumed only while the collector is
/// resident. `context` remains the exact d78 family/visibility/order authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalDesignerRoot {
    pub context: DesignerViewContext,
    pub view_ptr: usize,
    pub runtime_root_ptr: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddress {
    pub designer_refresh: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub callback_count: u64,
    pub latest_transition_sequence: u64,
    pub has_complete_snapshot: bool,
    pub current_error: Option<String>,
}

pub fn install() -> Result<InstalledHookAddress, String> {
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(InstalledHookAddress {
            designer_refresh: MODULE_BASE.load(Ordering::Acquire) + DESIGNER_REFRESH_RVA as usize,
        });
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let target = base + DESIGNER_REFRESH_RVA as usize;
    verify_sentinel(target)?;
    STATE.get_or_init(|| Mutex::new(HookState::default()));
    let original =
        unsafe { MinHook::create_hook(target as *mut c_void, hook_refresh as *mut c_void) }
            .map_err(|status| format!("A27 designer refresh hook creation failed: {status}"))?;
    ORIGINAL_REFRESH.store(original as usize, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
        // An enable that reported failure may still have patched the target,
        // and `remove` here is the only attempt to take it back out. Its result
        // was discarded outright before 2026-08-03, which left a possibly-live
        // detour with `INSTALLED` false and nothing recording it.
        if let Err(removal) = unsafe { inline_detour::remove(target as *mut c_void) } {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            // BOTH statics survive an unproved removal, and neither is
            // bookkeeping. `MODULE_BASE` is how the cleanup retry finds the
            // target at all. `ORIGINAL_REFRESH` is worse than bookkeeping: the
            // detour may be live, and `hook_refresh` calls the game's original
            // function THROUGH this pointer -- zeroing it while a patched site
            // can still dispatch into the callback made the collector skip the
            // engine's own work, which is a mutated simulation rather than a
            // lost row.
            return Err(format!(
                "A27 designer refresh hook enable failed: {status}; removal failed: {removal}"
            ));
        }
        ORIGINAL_REFRESH.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("A27 designer refresh hook enable failed: {status}"));
    }
    // Fail closed on identity. The observation re-derives the module from the
    // running process, refuses anything that is not an exact-a729 `hoi4.exe`,
    // re-checks the admitted original prefix against that file's own bytes at
    // this RVA, and binds the installed relay to this detour. An installation
    // that cannot be attested is rolled back rather than shipped.
    if let Err(error) = HOOK_RECEIPTS.observe_installed(
        base,
        [SiteDetour::new(
            "gameplay.d78.designer_refresh",
            hook_refresh as *const () as usize,
        )],
    )
    {
        let disabled = unsafe { inline_detour::disable(target as *mut c_void) };
        if disabled.is_err() {
            // The target may still be patched, so the custody record is the
            // only remaining evidence that it is. Keep it, and latch.
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A27 designer refresh hook receipt refused: {error}; rollback failed: {}",
                disabled.unwrap_err()
            ));
        }
        // The removal result is NOT discarded. It was, and that lost the only
        // record of a live detour on this branch: `disable` succeeding says the
        // site is inert, not that the patch is gone, and a refused `remove`
        // leaves a created MinHook target behind. With the result thrown away
        // the module then answered `installed() == false` and
        // `release_pending() == false`, so the registry recorded a clean
        // release over it and a later install could refuse with
        // MH_ERROR_ALREADY_CREATED.
        if let Err(status) = unsafe { inline_detour::remove(target as *mut c_void) } {
            // Keep the base and the custody: `retry_failed_install_cleanup`
            // needs the first to find the target and the second is the
            // remaining evidence of what may still be patched.
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A27 designer refresh hook receipt refused: {error}; removal failed: {status}"
            ));
        }
        HOOK_RECEIPTS.discard_all_custody();
        ORIGINAL_REFRESH.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("A27 designer refresh hook receipt refused: {error}"));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(InstalledHookAddress {
        designer_refresh: target,
    })
}

/// True once this module owns an enabled, receipt-observed production hook.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// The exact enabled-byte receipt for the single d78 refresh site.
pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.installed_receipts()
}

/// Consume the release receipt. One-shot, like every other lane's take.
pub fn take_released_hook_receipts(
) -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.take_released_receipts()
}

/// True while this module holds a release it could not PROVE: a failed install
/// whose removal did not take, an ORDINARY teardown that stopped part-way, or
/// receipt custody still on the installed side.
///
/// The middle clause is not redundant with the third. A teardown that disabled
/// the site and then failed to remove it holds no receipt custody worth the
/// name, and its `INSTALLED` flag is already false, so without the phase this
/// predicate answered "nothing outstanding" over a created MinHook target.
pub fn release_pending() -> bool {
    FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire)
        || teardown_pending()
        || (!installed() && HOOK_RECEIPTS.may_still_hold_installed_custody())
}

/// True while a FAILED install left a removal this module could not prove.
///
/// The narrower half of [`release_pending`], and the one an abort can actually
/// act on: it names the state [`retry_failed_install_cleanup`] exists to clear.
pub fn failed_install_cleanup_required() -> bool {
    FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire)
}

/// Retries the removal a failed install could not prove.
///
/// # The dead end this exists to open
///
/// `install` latches `FAILED_INSTALL_REMOVAL_UNPROVED` when its own rollback
/// could not take the detour back out, and it clears `INSTALLED` and
/// `MODULE_BASE` on the way past. `uninstall` then returns `Ok` immediately --
/// `INSTALLED` is already false -- WITHOUT retrying anything, so the registry's
/// post-abort proof kept seeing `release_pending()` and refusing, the orphan
/// driver kept calling the same no-op teardown, and the loop only ended when
/// orphan recovery gave up with the detour possibly still live. D45 has had the
/// primitive that breaks this since its own rollback grew one; this is the
/// same shape for the d78 site, and `clear_failed_install_cleanup_before_abort`
/// in the collector is what calls it.
///
/// The overwhelmingly likely cause of the original failure is a refused thread
/// freeze, which is transient -- so a retry is a real second chance rather
/// than a ritual.
///
/// `MODULE_BASE` is re-derived rather than read, because the failing path
/// zeroed it before latching. `MH_RemoveHook` disables an enabled hook on the
/// way out, so removal alone covers both shapes of failure: an `enable` that
/// reported failure and could not be removed, and a receipt refusal whose
/// `disable` did not take.
pub fn retry_failed_install_cleanup() -> Result<(), String> {
    if !FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire) {
        return Err("A27 designer refresh failed-install cleanup has no retryable custody".into());
    }
    let mut base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    }
    if base == 0 {
        return Err("A27 designer refresh cleanup could not resolve the module base".into());
    }
    let target = base + DESIGNER_REFRESH_RVA as usize;
    unsafe { inline_detour::remove(target as *mut c_void) }
        .map_err(|status| format!("A27 designer refresh hook removal retry failed: {status}"))?;
    // The install never completed, so any custody on the installed side is a
    // partial roster from the attempt that failed. Discarding it is what
    // `install`'s own rollback does with it; leaving it would keep
    // `release_pending` true over a site that is now provably out.
    //
    // BEFORE the latch is cleared, deliberately. `discard_all_custody` is
    // silent on a poisoned lock, so a discard that did not take must leave this
    // module still reporting a pending release rather than a clean one -- and
    // the latch is the last thing standing between the two.
    HOOK_RECEIPTS.discard_all_custody();
    // Only now. The latch is the sole record that a detour may be live, so it
    // is cleared exactly when a removal has come back clean -- never on the
    // way in, and never because the retry was merely attempted.
    FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);
    ORIGINAL_REFRESH.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    Ok(())
}

/// What an interrupted teardown still owes, so a RETRY RESUMES instead of
/// starting over or, worse, doing nothing.
///
/// The phases exist because each of the three steps can fail independently and
/// none of them is safely repeatable from the top: `MH_DisableHook` on an
/// already-disabled hook is an error, so a retry that blindly re-disabled would
/// never converge, and a retry that blindly skipped straight to the receipt
/// would publish a release over a detour still in the executable.
///
/// See [`teardown_owes`] for what each value means and
/// [`uninstall`] for the transitions.
const TEARDOWN_OWES_NOTHING: u8 = 0;
const TEARDOWN_OWES_DISABLE: u8 = 1;
const TEARDOWN_OWES_REMOVE: u8 = 2;
const TEARDOWN_OWES_RECEIPT: u8 = 3;

/// The teardown phase this module is parked in. Set the moment teardown becomes
/// owed and advanced only by a step that came back clean.
static TEARDOWN_PHASE: AtomicU8 = AtomicU8::new(TEARDOWN_OWES_NOTHING);

/// The three questions a teardown attempt asks itself, from the phase it is
/// resuming at: `(disable, remove, publish receipt)`.
///
/// Free-standing and total so the resumption rule is testable without a patched
/// `hoi4.exe` — the physical half needs a live target, this half is where a
/// wedge or a premature receipt actually comes from.
const fn teardown_owes(phase: u8) -> (bool, bool, bool) {
    match phase {
        TEARDOWN_OWES_DISABLE => (true, true, true),
        TEARDOWN_OWES_REMOVE => (false, true, true),
        TEARDOWN_OWES_RECEIPT => (false, false, true),
        // TEARDOWN_OWES_NOTHING, and any value that could only come from a
        // corrupted store: owing nothing is the only safe reading, because the
        // caller has already established there is work to do before asking.
        _ => (false, false, false),
    }
}

/// True while an interrupted teardown still owes physical work or a receipt.
pub fn teardown_pending() -> bool {
    TEARDOWN_PHASE.load(Ordering::Acquire) != TEARDOWN_OWES_NOTHING
}

/// Releases the site, RESUMABLY.
///
/// # The wedge this shape exists to prevent
///
/// This used to clear `INSTALLED` first and then run disable, remove and
/// receipt-publication, any of which can fail — a refused process freeze is the
/// ordinary cause and is transient. A failure left `INSTALLED` false with the
/// work half done, so every later call returned `Ok` at the top having done
/// nothing, while `release_pending()` stayed true through the surviving receipt
/// custody. The registry's post-abort proof therefore refused forever, the
/// orphan driver re-ran the same no-op teardown, and one transient freeze
/// refusal became a permanently live detour.
///
/// Now the owed work is recorded in [`TEARDOWN_PHASE`] before any of it is
/// attempted, each step advances the phase only when it comes back clean, and a
/// retry picks up exactly where the last attempt stopped.
pub fn uninstall() -> Result<(), String> {
    // A FAILED INSTALL is serviced first, and by every caller rather than only
    // by the production registry's abort. The legacy evidence-mode install site
    // discards its error and the matching teardown calls this function, so a
    // failed install whose removal could not be proved reached the
    // owes-nothing return below and cleaned up nothing at all -- there is no
    // registry in that mode to call `retry_failed_install_cleanup` for it.
    if FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire) {
        retry_failed_install_cleanup()?;
    }
    if INSTALLED.swap(false, Ordering::AcqRel) {
        // Newly owed. Never downgrade a phase an earlier interrupted attempt
        // already advanced past.
        let _ = TEARDOWN_PHASE.compare_exchange(
            TEARDOWN_OWES_NOTHING,
            TEARDOWN_OWES_DISABLE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    let phase = TEARDOWN_PHASE.load(Ordering::Acquire);
    if phase == TEARDOWN_OWES_NOTHING {
        return Ok(());
    }
    let (owes_disable, owes_remove, owes_receipt) = teardown_owes(phase);
    // Re-derived when a previous attempt already zeroed it, for the same reason
    // `retry_failed_install_cleanup` re-derives.
    let mut base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    }
    if base == 0 {
        return Err("A27 designer refresh teardown could not resolve the module base".into());
    }
    let target = base + DESIGNER_REFRESH_RVA as usize;
    if owes_disable {
        unsafe { inline_detour::disable(target as *mut c_void) }
            .map_err(|status| format!("A27 designer refresh hook disable failed: {status}"))?;
        TEARDOWN_PHASE.store(TEARDOWN_OWES_REMOVE, Ordering::Release);
    }
    if owes_remove {
        unsafe { inline_detour::remove(target as *mut c_void) }
            .map_err(|status| format!("A27 designer refresh hook removal failed: {status}"))?;
        TEARDOWN_PHASE.store(TEARDOWN_OWES_RECEIPT, Ordering::Release);
    }
    // Mint the release receipt only for a teardown that provably restored the
    // target. Every physical step above returned clean to reach here -- on this
    // attempt or an earlier one -- and `observe_released_set` then
    // independently re-reads the site's bytes and refuses unless the admitted
    // original prefix is back.
    //
    // `relay_freed: true` is the truth of this path: `inline_detour::remove` is
    // MinHook's `MH_RemoveHook` and frees the trampoline.
    if owes_receipt && HOOK_RECEIPTS.has_complete_installed_custody() {
        HOOK_RECEIPTS
            .publish_release(true)
            .map_err(|error| format!("A27 designer refresh release receipt refused: {error}"))?;
    }
    // The detour is out and the receipt is settled. Clearing the failed-install
    // latch here too is correct and not a shortcut: reaching this line means a
    // removal came back clean for this exact site, which is the whole of what
    // that latch records.
    TEARDOWN_PHASE.store(TEARDOWN_OWES_NOTHING, Ordering::Release);
    FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);
    ORIGINAL_REFRESH.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    if let Some(state) = STATE.get().and_then(|state| state.lock().ok()) {
        drop(state);
    }
    Ok(())
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics::default();
    };
    HookDiagnostics {
        callback_count: state.callback_count,
        latest_transition_sequence: state
            .latest
            .as_ref()
            .map(|row| row.transition_sequence)
            .unwrap_or(0),
        has_complete_snapshot: state.latest.is_some(),
        current_error: state.current_error.clone(),
    }
}

pub fn latest_active_role_root() -> Result<Option<ProcessLocalDesignerRoot>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("A27 exact a729 designer hook is not installed".into());
    }
    let state = STATE
        .get()
        .ok_or("A27 hook state is not initialized")?
        .lock()
        .map_err(|_| "A27 hook state mutex poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    let Some(context) = state.latest.clone() else {
        return Ok(None);
    };
    if !context.active {
        return Ok(None);
    }
    if state.latest_view_ptr == 0 || state.latest_runtime_root_ptr == 0 {
        return Err("A27 active designer transition omitted its process-local root".into());
    }
    Ok(Some(ProcessLocalDesignerRoot {
        context,
        view_ptr: state.latest_view_ptr,
        runtime_root_ptr: state.latest_runtime_root_ptr,
    }))
}

pub fn encode_latest(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err(CarrierError::Source(
            "A27 exact a729 designer hook is not installed".into(),
        ));
    }
    let state = STATE
        .get()
        .ok_or_else(|| CarrierError::Source("A27 hook state is not initialized".into()))?
        .lock()
        .map_err(|_| CarrierError::Source("A27 hook state mutex poisoned".into()))?;
    if let Some(error) = &state.current_error {
        return Err(CarrierError::Source(error.clone()));
    }
    let context = state.latest.clone().ok_or_else(|| {
        CarrierError::Source("A27 has not observed a complete designer transition".into())
    })?;
    encode_payload(&DesignCatalogContextFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        completeness_record_identity: format!(
            "a27:a729:refresh_rva_{DESIGNER_REFRESH_RVA:08X}:transition={}",
            context.transition_sequence
        ),
        context: Some(context),
    })
}

unsafe extern "system" fn hook_refresh(view: usize) {
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let original = ORIGINAL_REFRESH.load(Ordering::Acquire);
    if original != 0 {
        let original: RefreshFn = unsafe { mem::transmute(original) };
        unsafe { original(view) };
    }
    if !nested {
        let result = capture_after_refresh(view);
        if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
            state.callback_count = state.callback_count.saturating_add(1);
            match result {
                Ok((mut context, runtime_root_ptr)) => {
                    state.next_transition_sequence =
                        state.next_transition_sequence.saturating_add(1).max(1);
                    context.transition_sequence = state.next_transition_sequence;
                    state.latest = Some(context);
                    state.latest_view_ptr = view;
                    state.latest_runtime_root_ptr = runtime_root_ptr;
                    state.current_error = None;
                }
                Err(error) => {
                    state.latest_view_ptr = 0;
                    state.latest_runtime_root_ptr = 0;
                    state.current_error = Some(error);
                }
            }
        }
    }
    IN_CALLBACK.with(|guard| guard.set(nested));
}

fn capture_after_refresh(view: usize) -> Result<(DesignerViewContext, usize), String> {
    if view == 0 {
        return Err("A27 refresh supplied null view".into());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("A27 module base is unavailable".into());
    }
    let family = DesignerFamily::from_mode(read_i32(view + VIEW_MODE).ok_or("mode unreadable")?)
        .map_err(|error| format!("{error:?}"))?;
    let container = read_usize(view + VIEW_INTEL_CONTAINER).ok_or("intel container unreadable")?;
    if container == 0 {
        return Err("intel container is null".into());
    }
    let active = read_u8(container + GUI_VISIBLE_FLAGS)
        .ok_or("intel container visibility unreadable")?
        & 0x08
        != 0;
    let current_country_tag_id = current_country_tag_id(view, base)?;
    let selected_count = read_u32(view + VIEW_SELECTED_TAGS_COUNT)
        .ok_or("selected-country count unreadable")? as usize;
    if selected_count > crate::MAX_SELECTED_COUNTRIES {
        return Err("selected-country count exceeds bound".into());
    }
    let selected_data = read_usize(view + VIEW_SELECTED_TAGS_DATA)
        .ok_or("selected-country data pointer unreadable")?;
    if selected_count != 0 && selected_data == 0 {
        return Err("selected-country data pointer is null".into());
    }
    let mut selected_country_tag_ids = Vec::with_capacity(selected_count);
    for index in 0..selected_count {
        selected_country_tag_ids.push(
            read_u32(selected_data + index * 4)
                .ok_or_else(|| format!("selected-country row {index} unreadable"))?,
        );
    }
    let ordered_design_ids = if active {
        capture_ordered_design_ids(view, base)?
    } else {
        Vec::new()
    };
    let context = DesignerViewContext {
        transition_sequence: 1,
        active,
        family,
        our_filter_enabled: read_bool_byte(view + VIEW_OUR_FILTER, "our filter")?,
        foreign_filter_enabled: read_bool_byte(view + VIEW_FOREIGN_FILTER, "foreign filter")?,
        obsolete_filter_enabled: read_bool_byte(view + VIEW_OBSOLETE_FILTER, "obsolete filter")?,
        current_country_tag_id,
        selected_country_tag_ids,
        ordered_design_ids,
    };
    crate::validate_context(&context).map_err(|error| format!("{error:?}"))?;
    Ok((context, container))
}

fn capture_ordered_design_ids(view: usize, base: usize) -> Result<Vec<u32>, String> {
    let count = read_u32(view + VIEW_ROW_POOL_COUNT).ok_or("row-pool count unreadable")? as usize;
    let used = read_u32(view + VIEW_ROW_POOL_USED).ok_or("row-pool used unreadable")? as usize;
    if used > count || count > crate::MAX_ORDERED_DESIGNS {
        return Err("row-pool used/count invariant failed".into());
    }
    let data = read_usize(view + VIEW_ROW_POOL_DATA).ok_or("row-pool data unreadable")?;
    if used != 0 && data == 0 {
        return Err("row-pool data is null for nonzero used count".into());
    }
    let mut out = Vec::with_capacity(used);
    for index in 0..used {
        let row = read_usize(data + index * mem::size_of::<usize>())
            .ok_or_else(|| format!("row-pool pointer {index} unreadable"))?;
        if row == 0 || read_usize(row) != Some(base + ROW_VTABLE) {
            return Err(format!("row-pool row {index} failed exact vtable gate"));
        }
        let variant = read_usize(row + ROW_BOUND_VARIANT)
            .ok_or_else(|| format!("row-pool row {index} bound variant unreadable"))?;
        if variant == 0
            || read_usize(variant) != Some(base + VARIANT_VTABLE)
            || read_u32(variant + VARIANT_TYPE) != Some(TYPE_VARIANT)
        {
            return Err(format!("row-pool row {index} failed variant identity gate"));
        }
        out.push(
            read_u32(variant + VARIANT_ID)
                .filter(|id| *id != 0)
                .ok_or_else(|| format!("row-pool row {index} has invalid stable id"))?,
        );
    }
    Ok(out)
}

fn current_country_tag_id(view: usize, base: usize) -> Result<u32, String> {
    let handler = read_usize(view + VIEW_HANDLER).ok_or("designer handler unreadable")?;
    if handler == 0 {
        return Err("designer handler is null".into());
    }
    let vtable = read_usize(handler).ok_or("designer handler vtable unreadable")?;
    let getter = read_usize(vtable + HANDLER_CURRENT_TAG_VSLOT)
        .ok_or("designer current-tag virtual unreadable")?;
    if getter < base || getter >= base.saturating_add(MAX_MODULE_IMAGE_SPAN) {
        return Err("designer current-tag virtual is outside pinned module".into());
    }
    let getter: CurrentTagFn = unsafe { mem::transmute(getter) };
    let tag_identity = unsafe { getter(handler) };
    if tag_identity == 0 {
        return Err("designer current-tag virtual returned null".into());
    }
    let resolver: CountryFromTagFn =
        unsafe { mem::transmute(base + COUNTRY_FROM_TAG_RVA as usize) };
    let country = unsafe { resolver(tag_identity) };
    if country == 0 {
        return Err("country resolver returned null".into());
    }
    read_u32(country + 0x08)
        .filter(|tag| *tag != 0)
        .ok_or_else(|| "current country stable tag id unreadable/zero".into())
}

fn read_bool_byte(address: usize, label: &str) -> Result<bool, String> {
    match read_u8(address) {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        Some(value) => Err(format!("{label} byte is non-boolean ({value})")),
        None => Err(format!("{label} byte unreadable")),
    }
}

fn verify_sentinel(address: usize) -> Result<(), String> {
    let mut actual = [0u8; REFRESH_SENTINEL.len()];
    if !read_exact(address, &mut actual) {
        return Err("A27 designer refresh sentinel unreadable".into());
    }
    if actual != REFRESH_SENTINEL {
        return Err(format!(
            "A27 designer refresh sentinel mismatch: expected {:02X?}, got {:02X?}",
            REFRESH_SENTINEL, actual
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
fn read_u8(address: usize) -> Option<u8> {
    let mut b = [0; 1];
    read_exact(address, &mut b).then_some(b[0])
}
fn read_u32(address: usize) -> Option<u32> {
    let mut b = [0; 4];
    read_exact(address, &mut b).then(|| u32::from_le_bytes(b))
}
fn read_i32(address: usize) -> Option<i32> {
    read_u32(address).map(|v| v as i32)
}
fn read_usize(address: usize) -> Option<usize> {
    let mut b = [0; mem::size_of::<usize>()];
    read_exact(address, &mut b).then(|| usize::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Law: the single pinned spec describes the site this module patches.
    ///
    /// The name says `d78` because that is whose producer it is, and since
    /// 2026-08-13 the install unit that patches it is named for d78 too.
    ///
    /// Mutation proof: change `DESIGNER_REFRESH_RVA` or `REFRESH_SENTINEL`
    /// without changing the spec, and this fails.
    #[test]
    fn the_receipt_spec_matches_the_site_this_module_patches() {
        let spec = PRODUCTION_HOOK_SPECS[0];
        assert_eq!(spec.logical_name, "gameplay.d78.designer_refresh");
        assert_eq!(spec.requested_rva, u64::from(DESIGNER_REFRESH_RVA));
        assert_eq!(spec.original_prefix, &REFRESH_SENTINEL[..]);
        assert_eq!(spec.target_patch_len, 5);
        assert_eq!(spec.primitive_id, MINHOOK_NORMAL_REL5_PRIMITIVE_ID);
    }

    #[test]
    fn sentinel_is_exact_a729_prologue() {
        assert_eq!(
            REFRESH_SENTINEL,
            [
                0x40, 0x53, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xD9, 0x48, 0x8B, 0x89, 0xA0, 0xF3,
                0, 0
            ]
        );
    }

    /// Law: the cleanup retry refuses when there is nothing to retry, and says
    /// so rather than reporting a release it did not perform.
    ///
    /// This is the half of the primitive that can be exercised without a
    /// patched `hoi4.exe`, and it is the half that decides whether the abort
    /// path can be lied to. `clear_failed_install_cleanup_before_abort` only
    /// calls the retry when `failed_install_cleanup_required()` is true, so a
    /// retry that returned `Ok` over an unlatched module would let a caller
    /// that skipped the predicate believe a removal happened.
    ///
    /// The success half needs a live target: it calls `MH_RemoveHook` on a real
    /// detour. What is checked here is the refusal and the predicate's resting
    /// state.
    ///
    /// kills: making `retry_failed_install_cleanup` return `Ok(())` when the
    /// unproved-removal latch is clear.
    #[test]
    fn the_failed_install_cleanup_retry_refuses_when_nothing_is_latched() {
        assert!(
            !failed_install_cleanup_required(),
            "a fresh test process has installed nothing and latched nothing"
        );
        let refusal = retry_failed_install_cleanup()
            .expect_err("a retry with no retryable custody must refuse");
        assert!(
            refusal.contains("no retryable custody"),
            "the refusal must say WHY it declined, not merely fail: {refusal}"
        );
        assert!(
            !failed_install_cleanup_required(),
            "a refused retry must not latch anything either"
        );
    }

    /// Law: an interrupted teardown RESUMES at the step that failed, and a
    /// teardown that owes nothing does nothing.
    ///
    /// This is the offline half of the wedge fix. The physical half needs a
    /// patched `hoi4.exe`; what can be checked here is the rule that decides
    /// which steps a retry runs, and that rule is where both failure modes
    /// live: repeating `disable` on an already-disabled hook never converges
    /// (MinHook refuses it), and skipping straight to the receipt publishes a
    /// release over a detour still in the executable.
    ///
    /// kills: making `teardown_owes` return `(true, true, true)` for every
    /// phase (the restart-from-the-top bug), or `(false, false, true)` for
    /// `TEARDOWN_OWES_DISABLE` (the premature-receipt bug).
    #[test]
    fn an_interrupted_teardown_resumes_at_the_step_that_failed() {
        assert_eq!(teardown_owes(TEARDOWN_OWES_DISABLE), (true, true, true));
        assert_eq!(teardown_owes(TEARDOWN_OWES_REMOVE), (false, true, true));
        assert_eq!(teardown_owes(TEARDOWN_OWES_RECEIPT), (false, false, true));
        assert_eq!(teardown_owes(TEARDOWN_OWES_NOTHING), (false, false, false));

        // The phases are ordered, and the order is what makes "resume" mean
        // anything: a later phase owes a strict subset of an earlier one's work.
        for (earlier, later) in [
            (TEARDOWN_OWES_DISABLE, TEARDOWN_OWES_REMOVE),
            (TEARDOWN_OWES_REMOVE, TEARDOWN_OWES_RECEIPT),
            (TEARDOWN_OWES_RECEIPT, TEARDOWN_OWES_NOTHING),
        ] {
            assert!(earlier < later || later == TEARDOWN_OWES_NOTHING);
            let (ed, er, ec) = teardown_owes(earlier);
            let (ld, lr, lc) = teardown_owes(later);
            assert!(ed >= ld && er >= lr && ec >= lc, "{earlier} -> {later}");
        }
    }

    /// Law: a module that never installed owes no teardown, and says so without
    /// touching the executable.
    ///
    /// `uninstall` is called unconditionally by several collector paths, and
    /// the production hook registry's D78 arm is one of them. In a test process
    /// nothing is patched, so the only correct behaviour is an immediate clean
    /// return — an attempt to disable an unpatched site would be a MinHook
    /// error on every capture that ended without installing.
    ///
    /// kills: dropping the `TEARDOWN_OWES_NOTHING` early return.
    #[test]
    fn a_module_that_never_installed_owes_no_teardown() {
        assert!(!installed());
        assert!(!teardown_pending());
        assert_eq!(uninstall(), Ok(()));
        assert!(!teardown_pending());
        assert!(!release_pending());
    }

    #[test]
    fn exact_rvas_and_pointer_free_context_are_stable() {
        assert_eq!(DESIGNER_REFRESH_RVA, 0x0177_97A0);
        assert_eq!(DESIGNER_GRID_PRODUCER_RVA, 0x0177_D780);
        assert_eq!(DESIGNER_ROW_BIND_RVA, 0x0149_2B70);
        assert_eq!(mem::size_of::<usize>(), 8);
        assert_eq!(ROW_BOUND_VARIANT, 0x560);
    }
}
