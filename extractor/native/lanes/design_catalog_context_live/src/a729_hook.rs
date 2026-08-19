use crate::{
    encode_payload, CarrierError, DesignCatalogContextFrame, DesignerFamily, DesignerViewContext,
};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
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
pub const HOOK_RECEIPT_COUNT: usize = 1;

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
static FAILED_INSTALL_REMOVAL_UNPROVED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static ADMISSION_OPEN: AtomicBool = AtomicBool::new(false);
const QUIESCE_YIELDS: usize = 4096;

struct CallbackLease;

impl CallbackLease {
    fn enter() -> Self {
        IN_FLIGHT_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for CallbackLease {
    fn drop(&mut self) {
        IN_FLIGHT_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn drained_after_disable() -> bool {
    ADMISSION_OPEN.store(false, Ordering::Release);
    for _ in 0..QUIESCE_YIELDS {
        if IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0 {
            return true;
        }
        std::thread::yield_now();
    }
    IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0
}

fn retained_for_in_flight_bodies() -> String {
    format!(
        "A27 designer refresh detour retained: {} callback body/bodies still in flight after disable",
        IN_FLIGHT_CALLBACKS.load(Ordering::Acquire)
    )
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
    ADMISSION_OPEN.store(true, Ordering::Release);
    if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
        match unsafe { inline_detour::disable(target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(refusal) => {
                FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
                return Err(format!(
                    "A27 designer refresh hook enable failed: {status}; disable failed: {refusal}"
                ));
            }
        }
        if !drained_after_disable() {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A27 designer refresh hook enable failed: {status}; {}",
                retained_for_in_flight_bodies()
            ));
        }
        if let Err(removal) = unsafe { inline_detour::remove(target as *mut c_void) } {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A27 designer refresh hook enable failed: {status}; removal failed: {removal}"
            ));
        }
        ORIGINAL_REFRESH.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("A27 designer refresh hook enable failed: {status}"));
    }
    if let Err(error) = HOOK_RECEIPTS.observe_installed(
        base,
        [SiteDetour::new(
            "gameplay.d78.designer_refresh",
            hook_refresh as *const () as usize,
        )],
    ) {
        ADMISSION_OPEN.store(false, Ordering::Release);
        let disabled = unsafe { inline_detour::disable(target as *mut c_void) };
        if disabled.is_err() {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A27 designer refresh hook receipt refused: {error}; rollback failed: {}",
                disabled.unwrap_err()
            ));
        }
        // `release_pending() == false`, so the registry recorded a clean
        // release over it and a later install could refuse with
        if !drained_after_disable() {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A27 designer refresh hook receipt refused: {error}; {}",
                retained_for_in_flight_bodies()
            ));
        }
        if let Err(status) = unsafe { inline_detour::remove(target as *mut c_void) } {
            FAILED_INSTALL_REMOVAL_UNPROVED.store(true, Ordering::Release);
            return Err(format!(
                "A27 designer refresh hook receipt refused: {error}; removal failed: {status}"
            ));
        }
        HOOK_RECEIPTS.discard_all_custody();
        ORIGINAL_REFRESH.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!(
            "A27 designer refresh hook receipt refused: {error}"
        ));
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(InstalledHookAddress {
        designer_refresh: target,
    })
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; HOOK_RECEIPT_COUNT], String> {
    HOOK_RECEIPTS.installed_receipts()
}

/// Consume the release receipt. One-shot, like every other lane's take.
pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; HOOK_RECEIPT_COUNT], String>
{
    HOOK_RECEIPTS.take_released_receipts()
}

/// True while this module holds a release it could not PROVE: a failed install
pub fn release_pending() -> bool {
    FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire)
        || teardown_pending()
        || (!installed() && HOOK_RECEIPTS.may_still_hold_installed_custody())
}

/// The narrower half of [`release_pending`], and the one an abort can actually
pub fn failed_install_cleanup_required() -> bool {
    FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire)
}

/// post-abort proof kept seeing `release_pending()` and refusing, the orphan
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
    match unsafe { inline_detour::disable(target as *mut c_void) } {
        Ok(())
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
        | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
        Err(status) => {
            return Err(format!(
                "A27 designer refresh hook disable retry failed: {status}"
            ))
        }
    }
    if !drained_after_disable() {
        return Err(retained_for_in_flight_bodies());
    }
    unsafe { inline_detour::remove(target as *mut c_void) }
        .map_err(|status| format!("A27 designer refresh hook removal retry failed: {status}"))?;
    // `release_pending` true over a site that is now provably out.
    // module still reporting a pending release rather than a clean one -- and
    HOOK_RECEIPTS.discard_all_custody();
    FAILED_INSTALL_REMOVAL_UNPROVED.store(false, Ordering::Release);
    ORIGINAL_REFRESH.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    Ok(())
}

/// would publish a release over a detour still in the executable.
const TEARDOWN_OWES_NOTHING: u8 = 0;
const TEARDOWN_OWES_DISABLE: u8 = 1;
const TEARDOWN_OWES_REMOVE: u8 = 2;
const TEARDOWN_OWES_RECEIPT: u8 = 3;

static TEARDOWN_PHASE: AtomicU8 = AtomicU8::new(TEARDOWN_OWES_NOTHING);

const fn teardown_owes(phase: u8) -> (bool, bool, bool) {
    match phase {
        TEARDOWN_OWES_DISABLE => (true, true, true),
        TEARDOWN_OWES_REMOVE => (false, true, true),
        TEARDOWN_OWES_RECEIPT => (false, false, true),
        _ => (false, false, false),
    }
}

pub fn teardown_pending() -> bool {
    TEARDOWN_PHASE.load(Ordering::Acquire) != TEARDOWN_OWES_NOTHING
}

/// Releases the site, RESUMABLY.
/// nothing, while `release_pending()` stayed true through the surviving receipt
pub fn uninstall() -> Result<(), String> {
    if FAILED_INSTALL_REMOVAL_UNPROVED.load(Ordering::Acquire) {
        retry_failed_install_cleanup()?;
    }
    if INSTALLED.swap(false, Ordering::AcqRel) {
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
        // exactly this step. Freeing here instead is the use-after-free
        if !drained_after_disable() {
            return Err(retained_for_in_flight_bodies());
        }
        unsafe { inline_detour::remove(target as *mut c_void) }
            .map_err(|status| format!("A27 designer refresh hook removal failed: {status}"))?;
        TEARDOWN_PHASE.store(TEARDOWN_OWES_RECEIPT, Ordering::Release);
    }
    // Mint the release receipt only for a teardown that provably restored the
    // attempt or an earlier one -- and `observe_released_set` then
    if owes_receipt && HOOK_RECEIPTS.has_complete_installed_custody() {
        HOOK_RECEIPTS
            .publish_release(true)
            .map_err(|error| format!("A27 designer refresh release receipt refused: {error}"))?;
    }
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
    let _lease = CallbackLease::enter();
    let nested = IN_CALLBACK.with(|guard| guard.replace(true));
    let original = ORIGINAL_REFRESH.load(Ordering::Acquire);
    if original != 0 {
        let original: RefreshFn = unsafe { mem::transmute(original) };
        unsafe { original(view) };
    }
    if !nested && ADMISSION_OPEN.load(Ordering::Acquire) {
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

    /// `hook_refresh` is the use-after-free `inline_detour::remove`'s contract
    /// releases -- and one that never returned is a hung game at teardown.
    #[test]
    fn the_drain_refuses_while_a_detour_body_is_in_flight_and_closes_admission() {
        assert_eq!(
            IN_FLIGHT_CALLBACKS.load(Ordering::Acquire),
            0,
            "another test left a lease behind"
        );
        ADMISSION_OPEN.store(true, Ordering::Release);
        let lease = CallbackLease::enter();
        assert_eq!(IN_FLIGHT_CALLBACKS.load(Ordering::Acquire), 1);
        assert!(
            !drained_after_disable(),
            "the drain reported quiescence with a callback body in flight"
        );
        assert!(
            !ADMISSION_OPEN.load(Ordering::Acquire),
            "the drain left admission open"
        );
        assert!(retained_for_in_flight_bodies().contains("retained"));
        drop(lease);
        assert!(
            drained_after_disable(),
            "the drain refused after every body returned"
        );
    }

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

    /// so rather than reporting a release it did not perform.
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

    /// release over a detour still in the executable.
    #[test]
    fn an_interrupted_teardown_resumes_at_the_step_that_failed() {
        assert_eq!(teardown_owes(TEARDOWN_OWES_DISABLE), (true, true, true));
        assert_eq!(teardown_owes(TEARDOWN_OWES_REMOVE), (false, true, true));
        assert_eq!(teardown_owes(TEARDOWN_OWES_RECEIPT), (false, false, true));
        assert_eq!(teardown_owes(TEARDOWN_OWES_NOTHING), (false, false, false));

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
