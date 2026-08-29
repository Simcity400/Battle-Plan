use crate::{
    alert_icon_frame, ActiveAlertRow, AlertDedupIdentity, HudSurface, RawHudControllerRoot,
    TopbarAlertCounts, ACTIVE_ALERT_TYPE_COUNT, MAPMODE_SETUP_RVA, MAX_ACTIVE_ALERT_ROWS,
    MINIMAP_SETUP_RVA, THEATRE_SETUP_RVA, TOPBAR_SETUP_RVA,
};
use core::cell::Cell;
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use std::collections::BTreeMap;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const TOPBAR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
];
const MINIMAP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
const MAPMODE_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0x6c,
];
const THEATRE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56, 0x48, 0x8b,
];

const TOPBAR_RUNTIME_ROOT: usize = 0x78;
const MINIMAP_RUNTIME_ROOT: usize = 0x1490;
const MAPMODE_RUNTIME_ROOT: usize = 0x2ed8;
const THEATRE_RUNTIME_ROOT: usize = 0x58;
const RUNTIME_DEFINITION: usize = 0x15d0;
const DEFINITION_ROOT_NAME_VSLOT: usize = 0x48;
const MAX_MODULE_IMAGE_SPAN: usize = 0x0400_0000;
const MAX_MSVC_STRING_CAPACITY: usize = 1 << 20;
const TOPBAR_COUNTRY_TAG_DB_ID: usize = 0x9830;
const TOPBAR_MARKET_STATE: usize = 0x98d8;
const MARKET_ONGOING_COUNT: usize = 0x30;
const MARKET_NON_DELIVERING_COUNT: usize = 0x48;
const INTEL_STATE_RVA: usize = 0x0332_4490;
const INTEL_COMPLETED_COUNT: usize = 0x0c;
const INTEL_PREPARED_COUNT: usize = 0x24;
const INTEL_AVAILABLE_COUNT: usize = 0x6c;
const ALERT_UI_ROOT_RVA: usize = 0x0331_72e0;
const ALERT_MANAGER_OFFSET: usize = 0x798;
const ALERT_VECTOR_BASE: usize = 0x16950;
const ALERT_VECTOR_STRIDE: usize = 0x18;
const ALERT_SLOT_STRIDE: usize = 0x468;
const ALERT_SLOT_PRIORITY: usize = 0x430;
const ALERT_SLOT_IDENTITY_0: usize = 0x438;
const ALERT_SLOT_IDENTITY_1: usize = 0x440;
const ALERT_SLOT_TEMPLATE_CLASS: usize = 0x448;
const ALERT_SLOT_IDENTITY_2: usize = 0x450;
const ALERT_SLOT_ICON: usize = 0x458;
const ALERT_SLOT_ORDER: usize = 0x460;
const GAME_STATE_RVA: usize = 0x0331_6e90;
const COUNTRY_TABLE: usize = 0x2e0;
const COUNTRY_COUNT: usize = 0x2ec;
const COUNTRY_DB_ID: usize = 0x60;

type SetupFn = unsafe extern "system" fn(usize);
type DefinitionRootNameFn = unsafe extern "system" fn(usize) -> usize;

static ORIGINAL_TOPBAR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MINIMAP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MAPMODE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_THEATRE: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

thread_local! {
    static ACTIVE_CALLBACKS: Cell<u8> = const { Cell::new(0) };
}

static IN_FLIGHT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static ADMISSION_OPEN: AtomicBool = AtomicBool::new(false);
const QUIESCE_YIELDS: usize = 4096;

static RETAINED_AFTER_FAILED_INSTALL: AtomicBool = AtomicBool::new(false);

pub fn retained_after_failed_install() -> bool {
    RETAINED_AFTER_FAILED_INSTALL.load(Ordering::Acquire)
}

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
    for _ in 0..QUIESCE_YIELDS {
        if IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0 {
            return true;
        }
        std::thread::yield_now();
    }
    IN_FLIGHT_CALLBACKS.load(Ordering::Acquire) == 0
}

fn disable_drain_and_remove(resolved: InstalledAddresses) -> Result<(), String> {
    ADMISSION_OPEN.store(false, Ordering::Release);
    for target in targets(resolved) {
        match unsafe { inline_detour::disable(target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => return Err(format!("HUD {target:#x} hook disable failed: {status}")),
        }
    }
    if !drained_after_disable() {
        return Err(format!(
            "HUD teardown retained {} disabled detour(s): {} callback body/bodies still in flight",
            targets(resolved).len(),
            IN_FLIGHT_CALLBACKS.load(Ordering::Acquire)
        ));
    }
    for target in targets(resolved) {
        match unsafe { inline_detour::remove(target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => return Err(format!("HUD {target:#x} hook removal failed: {status}")),
        }
    }
    Ok(())
}

compact_contract::wire_counters! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct D79StateCounters, prefix = "d79_hook_";

    callback_count,
    rejected_observation_count,
}

#[derive(Default)]
struct HookState {
    latest: BTreeMap<HudSurface, RawHudControllerRoot>,
    counters: D79StateCounters,
    current_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub topbar_setup: usize,
    pub minimap_setup: usize,
    pub mapmode_setup: usize,
    pub theatre_setup: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub callback_count: u64,
    pub rejected_observation_count: u64,
    pub authenticated_surface_count: usize,
    pub in_flight_callbacks: usize,
    pub next_sequence: u64,
    pub current_error: Option<String>,
}

pub fn install() -> Result<InstalledAddresses, String> {
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    if RETAINED_AFTER_FAILED_INSTALL.load(Ordering::Acquire) {
        return Err(
            "HUD install refused: a previous install retained live detours; \
             run uninstall to finish the teardown first"
                .into(),
        );
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let resolved = addresses(base);
    for (address, sentinel, label) in [
        (resolved.topbar_setup, &TOPBAR_SENTINEL, "topbar setup"),
        (resolved.minimap_setup, &MINIMAP_SENTINEL, "minimap setup"),
        (resolved.mapmode_setup, &MAPMODE_SENTINEL, "mapmode setup"),
        (resolved.theatre_setup, &THEATRE_SENTINEL, "theatre setup"),
    ] {
        verify_sentinel(address, sentinel, label)?;
    }
    STATE.get_or_init(|| Mutex::new(HookState::default()));
    let specs = [
        (
            resolved.topbar_setup,
            hook_topbar as *mut c_void,
            &ORIGINAL_TOPBAR,
        ),
        (
            resolved.minimap_setup,
            hook_minimap as *mut c_void,
            &ORIGINAL_MINIMAP,
        ),
        (
            resolved.mapmode_setup,
            hook_mapmode as *mut c_void,
            &ORIGINAL_MAPMODE,
        ),
        (
            resolved.theatre_setup,
            hook_theatre as *mut c_void,
            &ORIGINAL_THEATRE,
        ),
    ];
    let mut created = Vec::new();
    for (target, detour, original) in specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(value) => {
                original.store(value as usize, Ordering::Release);
                created.push(target);
            }
            Err(status) => {
                for target in created {
                    let _ = unsafe { inline_detour::remove(target as *mut c_void) };
                }
                clear_originals();
                return Err(format!("HUD {target:#x} hook creation failed: {status}"));
            }
        }
    }
    ADMISSION_OPEN.store(true, Ordering::Release);
    for target in targets(resolved) {
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            match disable_drain_and_remove(resolved) {
                Ok(()) => {
                    clear_originals();
                    return Err(format!("HUD {target:#x} hook enable failed: {status}"));
                }
                Err(retained) => {
                    MODULE_BASE.store(base, Ordering::Release);
                    RETAINED_AFTER_FAILED_INSTALL.store(true, Ordering::Release);
                    return Err(format!(
                        "HUD {target:#x} hook enable failed: {status}; rollback retained: {retained}"
                    ));
                }
            }
        }
    }
    MODULE_BASE.store(base, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(resolved)
}

pub fn uninstall() -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) && !RETAINED_AFTER_FAILED_INSTALL.load(Ordering::Acquire)
    {
        return Ok(());
    }
    let resolved = addresses(MODULE_BASE.load(Ordering::Acquire));
    disable_drain_and_remove(resolved)?;
    RETAINED_AFTER_FAILED_INSTALL.store(false, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    clear_originals();
    if let Some(state) = STATE.get() {
        *state.lock().map_err(|_| "HUD hook state poisoned")? = HookState::default();
    }
    Ok(())
}

pub fn drain_complete_roots() -> Result<Vec<RawHudControllerRoot>, String> {
    let roots = latest_complete_roots()?;
    let mut state = STATE
        .get()
        .ok_or("HUD hook state is unavailable")?
        .lock()
        .map_err(|_| "HUD hook state poisoned")?;
    state.latest.clear();
    Ok(roots)
}

pub fn latest_complete_roots() -> Result<Vec<RawHudControllerRoot>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 HUD hooks are not installed".into());
    }
    let state = STATE
        .get()
        .ok_or("HUD hook state is unavailable")?
        .lock()
        .map_err(|_| "HUD hook state poisoned")?;
    if let Some(error) = &state.current_error {
        return Err(error.clone());
    }
    if state.latest.len() != 4 {
        return Err(format!(
            "HUD root set is incomplete: authenticated {}/4 surfaces",
            state.latest.len()
        ));
    }
    let mut roots = state.latest.values().cloned().collect::<Vec<_>>();
    roots.sort_by_key(|root| root.sequence);
    Ok(roots)
}

pub fn topbar_alert_counts(roots: &[RawHudControllerRoot]) -> Result<TopbarAlertCounts, String> {
    let topbar = roots
        .iter()
        .find(|root| root.surface == HudSurface::Topbar)
        .ok_or("HUD root set has no topbar controller")?;
    if topbar.setup_rva != TOPBAR_SETUP_RVA || topbar.controller_ptr == 0 {
        return Err("HUD topbar controller identity is invalid".into());
    }
    let country_tag_db_id = read_u32(topbar.controller_ptr + TOPBAR_COUNTRY_TAG_DB_ID)
        .filter(|value| *value != 0)
        .ok_or("HUD topbar country tag DB id is unavailable")?;
    let market = read_usize(topbar.controller_ptr + TOPBAR_MARKET_STATE)
        .filter(|pointer| *pointer != 0)
        .ok_or("HUD topbar market-state pointer is unavailable")?;
    let module_base = MODULE_BASE.load(Ordering::Acquire);
    if module_base == 0 {
        return Err("HUD module base is unavailable".into());
    }
    let intel = module_base + INTEL_STATE_RVA;
    let intel_available =
        read_i32(intel + INTEL_AVAILABLE_COUNT).ok_or("HUD intel-available count is unreadable")?;
    let intel_prepared =
        read_i32(intel + INTEL_PREPARED_COUNT).ok_or("HUD intel-prepared count is unreadable")?;
    let intel_completed =
        read_i32(intel + INTEL_COMPLETED_COUNT).ok_or("HUD intel-completed count is unreadable")?;
    let intel_prepared_or_completed = intel_prepared
        .checked_add(intel_completed)
        .ok_or("HUD intel prepared/completed count overflow")?;
    let counts = TopbarAlertCounts {
        country_tag_db_id,
        intel_available,
        intel_prepared_or_completed,
        market_non_delivering: read_i32(market + MARKET_NON_DELIVERING_COUNT)
            .ok_or("HUD non-delivering market count is unreadable")?,
        market_ongoing: read_i32(market + MARKET_ONGOING_COUNT)
            .ok_or("HUD ongoing market count is unreadable")?,
        active_alerts: active_alert_snapshot(module_base)?,
    };
    if [
        counts.intel_available,
        counts.intel_prepared_or_completed,
        counts.market_non_delivering,
        counts.market_ongoing,
    ]
    .into_iter()
    .any(|count| !(0..=1_000_000).contains(&count))
    {
        return Err("HUD topbar alert count is out of bounds".into());
    }
    Ok(counts)
}

fn active_alert_snapshot(module_base: usize) -> Result<Vec<ActiveAlertRow>, String> {
    let ui_root = read_usize(module_base + ALERT_UI_ROOT_RVA)
        .filter(|pointer| *pointer != 0)
        .ok_or("HUD alert UI root is unavailable")?;
    let manager = read_usize(ui_root + ALERT_MANAGER_OFFSET)
        .filter(|pointer| *pointer != 0)
        .ok_or("HUD alert manager is unavailable")?;
    let game_state = read_usize(module_base + GAME_STATE_RVA)
        .filter(|pointer| *pointer != 0)
        .ok_or("HUD alert country registry root is unavailable")?;
    let country_table = read_usize(game_state + COUNTRY_TABLE)
        .filter(|pointer| *pointer != 0)
        .ok_or("HUD alert country table is unavailable")?;
    let country_count = read_i32(game_state + COUNTRY_COUNT)
        .filter(|count| *count > 0)
        .ok_or("HUD alert country count is invalid")? as u32;

    let mut rows = Vec::new();
    for alert_type in 0..ACTIVE_ALERT_TYPE_COUNT {
        let vector = manager + ALERT_VECTOR_BASE + usize::from(alert_type) * ALERT_VECTOR_STRIDE;
        let count = read_i32(vector + 0x0c)
            .filter(|count| *count >= 0 && *count as usize <= MAX_ACTIVE_ALERT_ROWS)
            .ok_or("HUD active-alert vector count is invalid")? as usize;
        if rows.len().saturating_add(count) > MAX_ACTIVE_ALERT_ROWS {
            return Err("HUD active-alert snapshot exceeds its row bound".into());
        }
        let data = if count == 0 {
            0
        } else {
            read_usize(vector)
                .filter(|pointer| *pointer != 0)
                .ok_or("HUD active-alert vector data is unavailable")?
        };
        for source_ordinal in 0..count {
            let slot = data + source_ordinal * ALERT_SLOT_STRIDE;
            let identity_0 = read_usize(slot + ALERT_SLOT_IDENTITY_0)
                .ok_or("HUD active-alert identity[0] is unreadable")?;
            let identity_1 = read_usize(slot + ALERT_SLOT_IDENTITY_1)
                .ok_or("HUD active-alert identity[1] is unreadable")?;
            let identity_2 = read_usize(slot + ALERT_SLOT_IDENTITY_2)
                .ok_or("HUD active-alert identity[2] is unreadable")?;
            let (dedup_identity, country_target_db_id) =
                if identity_0 == 0 && identity_1 == 0 && identity_2 == 0 {
                    (AlertDedupIdentity::ZeroTuple, None)
                } else if matches!(alert_type, 1 | 2 | 5 | 6 | 7 | 0x2d | 0x2e)
                    && identity_0 != 0
                    && identity_1 == 0
                    && identity_2 == 0
                {
                    let country_id = read_u32(identity_0 + COUNTRY_DB_ID)
                        .filter(|id| *id != 0 && *id < country_count)
                        .ok_or("HUD active-alert country target id is invalid")?;
                    if read_usize(country_table + country_id as usize * mem::size_of::<usize>())
                        != Some(identity_0)
                    {
                        return Err("HUD active-alert country target failed registry join".into());
                    }
                    (AlertDedupIdentity::CountryDbId, Some(country_id))
                } else if matches!(alert_type, 3 | 4 | 0x2b)
                    && identity_0 == 0
                    && identity_1 != 0
                    && identity_2 == 0
                {
                    (AlertDedupIdentity::CommandGroupOmitted, None)
                } else if matches!(alert_type, 0x16 | 0x3c)
                    && identity_0 != 0
                    && identity_1 == 0
                    && identity_2 == 0
                {
                    (AlertDedupIdentity::OpaqueEngineObjectOmitted, None)
                } else {
                    return Err(format!(
                        "HUD active-alert type {alert_type} has an unauthenticated dedup tuple"
                    ));
                };
            let icon = read_usize(slot + ALERT_SLOT_ICON)
                .ok_or("HUD active-alert icon field is unreadable")?;
            let icon_present = icon != 0;
            let icon_frame = if icon_present {
                let widget_alert_type =
                    read_u32(icon + 0x48).ok_or("HUD active-alert icon type is unreadable")?;
                if widget_alert_type != u32::from(alert_type) {
                    return Err("HUD active-alert slot/widget type join is invalid".into());
                }
                Some(
                    alert_icon_frame(alert_type)
                        .ok_or("HUD active-alert icon type is out of range")?,
                )
            } else {
                None
            };
            let display_order = read_i32(slot + ALERT_SLOT_ORDER)
                .ok_or("HUD active-alert display order is unreadable")?;
            if (icon_present && display_order < 0) || (!icon_present && display_order != -1) {
                return Err("HUD active-alert icon/order join is invalid".into());
            }
            rows.push(ActiveAlertRow {
                alert_type,
                source_ordinal: u16::try_from(source_ordinal)
                    .map_err(|_| "HUD active-alert source ordinal overflow")?,
                priority: read_i32(slot + ALERT_SLOT_PRIORITY)
                    .ok_or("HUD active-alert priority is unreadable")?,
                template_class: read_u32(slot + ALERT_SLOT_TEMPLATE_CLASS)
                    .ok_or("HUD active-alert template class is unreadable")?,
                dedup_identity,
                country_target_db_id,
                icon_present,
                icon_frame,
                display_order,
            });
        }
    }
    Ok(rows)
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|state| state.lock().ok()) else {
        return HookDiagnostics::default();
    };
    HookDiagnostics {
        callback_count: state.counters.callback_count,
        rejected_observation_count: state.counters.rejected_observation_count,
        authenticated_surface_count: state.latest.len(),
        in_flight_callbacks: IN_FLIGHT_CALLBACKS.load(Ordering::Acquire),
        next_sequence: NEXT_SEQUENCE.load(Ordering::Acquire),
        current_error: state.current_error.clone(),
    }
}

unsafe extern "system" fn hook_topbar(controller: usize) {
    let _lease = CallbackLease::enter();
    call_original_and_capture(
        controller,
        &ORIGINAL_TOPBAR,
        HudSurface::Topbar,
        TOPBAR_SETUP_RVA,
        TOPBAR_RUNTIME_ROOT,
    );
}

unsafe extern "system" fn hook_minimap(controller: usize) {
    let _lease = CallbackLease::enter();
    call_original_and_capture(
        controller,
        &ORIGINAL_MINIMAP,
        HudSurface::Minimap,
        MINIMAP_SETUP_RVA,
        MINIMAP_RUNTIME_ROOT,
    );
}

unsafe extern "system" fn hook_mapmode(controller: usize) {
    let _lease = CallbackLease::enter();
    call_original_and_capture(
        controller,
        &ORIGINAL_MAPMODE,
        HudSurface::Mapmodes,
        MAPMODE_SETUP_RVA,
        MAPMODE_RUNTIME_ROOT,
    );
}

unsafe extern "system" fn hook_theatre(controller: usize) {
    let _lease = CallbackLease::enter();
    call_original_and_capture(
        controller,
        &ORIGINAL_THEATRE,
        HudSurface::TheatreSelector,
        THEATRE_SETUP_RVA,
        THEATRE_RUNTIME_ROOT,
    );
}

fn call_original_and_capture(
    controller: usize,
    original_slot: &AtomicUsize,
    surface: HudSurface,
    setup_rva: u32,
    root_offset: usize,
) {
    let callback_bit = match surface {
        HudSurface::Topbar => 1,
        HudSurface::Minimap => 1 << 1,
        HudSurface::Mapmodes => 1 << 2,
        HudSurface::TheatreSelector => 1 << 3,
        HudSurface::DesignerCountryEquipment
        | HudSurface::DesignerTank
        | HudSurface::DesignerPlane => 1 << 7,
    };
    let prior_callbacks = ACTIVE_CALLBACKS.with(|guard| {
        let prior = guard.get();
        guard.set(prior | callback_bit);
        prior
    });
    let nested = prior_callbacks & callback_bit != 0;
    let original = original_slot.load(Ordering::Acquire);
    if original != 0 {
        let original: SetupFn = unsafe { mem::transmute(original) };
        unsafe { original(controller) };
    }
    if !nested && ADMISSION_OPEN.load(Ordering::Acquire) {
        let result = capture_root(controller, surface, setup_rva, root_offset);
        if let Some(mut state) = STATE.get().and_then(|state| state.lock().ok()) {
            state.counters.bump(|counters| &mut counters.callback_count);
            match result {
                Ok(root) => {
                    state.latest.insert(surface, root);
                }
                Err(error) => {
                    state
                        .counters
                        .bump(|counters| &mut counters.rejected_observation_count);
                    state.current_error = Some(error);
                }
            }
        }
    }
    ACTIVE_CALLBACKS.with(|guard| guard.set(prior_callbacks));
}

fn capture_root(
    controller: usize,
    surface: HudSurface,
    setup_rva: u32,
    root_offset: usize,
) -> Result<RawHudControllerRoot, String> {
    if controller == 0 {
        return Err(format!(
            "{} setup supplied null controller",
            surface.token()
        ));
    }
    let runtime_root_ptr = read_usize(controller.saturating_add(root_offset))
        .filter(|pointer| *pointer != 0)
        .ok_or_else(|| format!("{} runtime root is null or unreadable", surface.token()))?;
    let runtime_root_name = runtime_root_name(runtime_root_ptr)?;
    Ok(RawHudControllerRoot {
        sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        setup_rva,
        surface,
        controller_ptr: controller,
        runtime_root_ptr,
        runtime_root_name,
    })
}

pub fn runtime_root_name(runtime_root_ptr: usize) -> Result<String, String> {
    if runtime_root_ptr == 0 {
        return Err("HUD runtime root is null".into());
    }
    let definition = read_usize(runtime_root_ptr.saturating_add(RUNTIME_DEFINITION))
        .filter(|pointer| *pointer != 0)
        .ok_or("HUD runtime GUI definition is null or unreadable")?;
    let vtable = read_usize(definition).ok_or("HUD runtime GUI definition vtable unreadable")?;
    let getter = read_usize(vtable.saturating_add(DEFINITION_ROOT_NAME_VSLOT))
        .ok_or("HUD runtime GUI root-name virtual unreadable")?;
    let mut base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    }
    if base == 0 || getter < base || getter >= base.saturating_add(MAX_MODULE_IMAGE_SPAN) {
        return Err("HUD runtime GUI root-name virtual is outside pinned module".into());
    }
    let getter: DefinitionRootNameFn = unsafe { mem::transmute(getter) };
    let string_object = unsafe { getter(definition) };
    if string_object == 0 {
        return Err("HUD runtime GUI root-name virtual returned null".into());
    }
    let value = read_msvc_string(string_object)?;
    crate::validate_runtime_root_name(&value)?;
    Ok(value)
}

fn read_msvc_string(address: usize) -> Result<String, String> {
    let mut object = [0u8; 0x20];
    if !read_exact(address, &mut object) {
        return Err("HUD runtime GUI root-name string object unreadable".into());
    }
    let len = usize::from_le_bytes(object[0x10..0x18].try_into().unwrap());
    let capacity = usize::from_le_bytes(object[0x18..0x20].try_into().unwrap());
    if len == 0
        || len > crate::MAX_RUNTIME_ROOT_NAME_BYTES
        || capacity < len
        || capacity > MAX_MSVC_STRING_CAPACITY
    {
        return Err("HUD runtime GUI root-name string bounds are invalid".into());
    }
    let mut bytes = vec![0u8; len.saturating_add(1)];
    if capacity <= 0x0f {
        if len >= 16 || object[len] != 0 {
            return Err("HUD inline runtime GUI root name is not terminated".into());
        }
        bytes[..len].copy_from_slice(&object[..len]);
    } else {
        let data = usize::from_le_bytes(object[..mem::size_of::<usize>()].try_into().unwrap());
        if data == 0 || !read_exact(data, &mut bytes) || bytes[len] != 0 {
            return Err("HUD heap runtime GUI root name is unreadable or unterminated".into());
        }
    }
    bytes.truncate(len);
    String::from_utf8(bytes).map_err(|_| "HUD runtime GUI root name is not UTF-8".into())
}

fn addresses(base: usize) -> InstalledAddresses {
    InstalledAddresses {
        topbar_setup: base + TOPBAR_SETUP_RVA as usize,
        minimap_setup: base + MINIMAP_SETUP_RVA as usize,
        mapmode_setup: base + MAPMODE_SETUP_RVA as usize,
        theatre_setup: base + THEATRE_SETUP_RVA as usize,
    }
}

fn targets(addresses: InstalledAddresses) -> [usize; 4] {
    [
        addresses.topbar_setup,
        addresses.minimap_setup,
        addresses.mapmode_setup,
        addresses.theatre_setup,
    ]
}

fn clear_originals() {
    for original in [
        &ORIGINAL_TOPBAR,
        &ORIGINAL_MINIMAP,
        &ORIGINAL_MAPMODE,
        &ORIGINAL_THEATRE,
    ] {
        original.store(0, Ordering::Release);
    }
}

fn verify_sentinel(address: usize, expected: &[u8; 16], label: &str) -> Result<(), String> {
    let mut actual = [0u8; 16];
    if !read_exact(address, &mut actual) {
        return Err(format!("HUD {label} sentinel unreadable"));
    }
    if &actual != expected {
        return Err(format!(
            "HUD {label} sentinel mismatch: expected {expected:02X?}, got {actual:02X?}"
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

fn read_usize(address: usize) -> Option<usize> {
    let mut bytes = [0u8; mem::size_of::<usize>()];
    read_exact(address, &mut bytes).then(|| usize::from_le_bytes(bytes))
}

fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes).then(|| u32::from_le_bytes(bytes))
}

fn read_i32(address: usize) -> Option<i32> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes).then(|| i32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// is the use-after-free `inline_detour::remove`'s contract exists to stop;
    #[test]
    fn the_drain_refuses_while_a_detour_body_is_in_flight_and_passes_once_it_returns() {
        let outside = IN_FLIGHT_CALLBACKS.load(Ordering::Acquire);
        assert_eq!(outside, 0, "another test left a lease behind");
        let lease = CallbackLease::enter();
        assert_eq!(IN_FLIGHT_CALLBACKS.load(Ordering::Acquire), 1);
        assert!(
            !drained_after_disable(),
            "the drain reported quiescence with a callback body in flight"
        );
        drop(lease);
        assert!(
            drained_after_disable(),
            "the drain refused after every body returned"
        );
    }

    #[test]
    fn exact_a729_addresses_offsets_and_sentinels_are_pinned() {
        let base = 0x1400_0000_0usize;
        let resolved = addresses(base);
        assert_eq!(resolved.topbar_setup, base + 0x0188_BA30);
        assert_eq!(resolved.minimap_setup, base + 0x017B_9A80);
        assert_eq!(resolved.mapmode_setup, base + 0x0156_3C80);
        assert_eq!(resolved.theatre_setup, base + 0x0187_8E40);
        assert_eq!(TOPBAR_RUNTIME_ROOT, 0x78);
        assert_eq!(MINIMAP_RUNTIME_ROOT, 0x1490);
        assert_eq!(MAPMODE_RUNTIME_ROOT, 0x2ed8);
        assert_eq!(THEATRE_RUNTIME_ROOT, 0x58);
        assert_eq!(RUNTIME_DEFINITION, 0x15d0);
        assert_eq!(DEFINITION_ROOT_NAME_VSLOT, 0x48);
        assert_eq!(TOPBAR_SENTINEL.len(), 16);
        assert_eq!(MINIMAP_SENTINEL.len(), 16);
        assert_eq!(MAPMODE_SENTINEL.len(), 16);
        assert_eq!(THEATRE_SENTINEL.len(), 16);
    }
}
