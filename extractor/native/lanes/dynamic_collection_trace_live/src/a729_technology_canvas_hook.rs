use crate::a729_technology_canvas::{
    expected_owner_construction_record_sha256, ObserverBoundary, OwnerRegistryWitness,
    A729_EXECUTABLE_SHA256, AUTHENTICATED_EXACT20_HOOK_CONTRACT, CONTROLLER_MODE_OFFSET,
    MODE6_OWNER_SLOT_OFFSET, OWNER_CONSTRUCTOR_RVA, POPULATION_COMMIT_RVA, REFRESH_COMMIT_RVA,
    TECHNOLOGY_MODE,
};
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::Read;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const CLOSE_BUTTON_CORRECTION_JOB_ID: &str =
    "20260723T043254Z-technology-canvas-static-names-ab754d4d";
pub const CLOSE_BUTTON_CORRECTION_ARTIFACT_SHA256: &str =
    "a8bd8fe0fa4e08a18323d521a223a77315ad4595497bb9f9e7d1f122f4df7564";
pub const ACTIVE_ROOT_GRAPHICAL_JOIN_JOB_ID: &str =
    "20260723T043033Z-ui09-production-runtime-canvas-final-8b2d5ee3";
pub const ACTIVE_ROOT_GRAPHICAL_JOIN_ARTIFACT_SHA256: &str =
    "5fbd2a63f1ab3ca036bb596b1218ae36daefc2719b408dc03603e4bd85b37d8d";
pub const TECHNOLOGY_CANVAS_AUTHORED_SURFACE_IDENTITY: &str = "technology_canvas";
pub const TECHNOLOGY_CANVAS_RUNTIME_ROOT_IDENTITY: &str = "countrytechtreeview";
pub const TECHNOLOGY_CANVAS_EFFECTIVE_INTERFACE_WINNER: &str = "interface/countrytechtreeview.gui";

const ACTIVE_FOLDER_VECTOR_OFFSET: usize = 0x19F8;
const ACTIVE_FOLDER_COUNT_OFFSET: usize = 0x1A04;
const ACTIVE_FOLDER_ORDINAL_OFFSET: usize = 0x1A90;
const GUI_SUBOBJECT_OFFSET: usize = 0x30;
pub const FOLDER_GRID_MATERIALIZED_VECTOR_OFFSET: usize = 0x1A0;
pub const FOLDER_GRID_MATERIALIZED_COUNT_OFFSET: usize = 0x1AC;
const MAX_FOLDER_ROOTS: usize = 4_096;
const MAX_PENDING: usize = 4_096;

type OwnerConstructorFn = unsafe extern "system" fn(usize, usize) -> usize;
type CommitFn = unsafe extern "system" fn(usize);

static ORIGINAL_OWNER_CONSTRUCTOR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_POPULATION_COMMIT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_REFRESH_COMMIT: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static GAME_STATE_ROOT_SLOT: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static OWNER_CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static COMMIT_CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static INSTALL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyCanvasHookBoundary {
    pub commit_sequence: u64,
    pub callback_rva: usize,
    pub boundary: ObserverBoundary,
    pub callback_qpc_ticks: u64,
    pub game_hour_raw: i32,
    pub owner_root_ptr: usize,
    pub controller_ptr: usize,
    pub owner_mode6_slot_value: usize,
    pub owner_registry_witness: OwnerRegistryWitness,
    pub active_folder_ordinal: u32,
    pub active_folder_count: u32,
    pub active_folder_root_ptr: usize,
    pub runtime_graphical_subobject_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyCanvasPresentRegistration {
    pub authored_surface_identity: &'static str,
    pub runtime_root_identity: &'static str,
    pub effective_interface_winner: &'static str,
    pub callback_qpc_ticks: u64,
    pub game_hour_raw: i32,
    pub controller_ptr: usize,
    pub active_folder_ordinal: u32,
    pub active_folder_root_ptr: usize,
    pub runtime_graphical_subobject_ptr: usize,
}

impl TechnologyCanvasHookBoundary {
    pub fn present_registration(&self) -> TechnologyCanvasPresentRegistration {
        TechnologyCanvasPresentRegistration {
            authored_surface_identity: TECHNOLOGY_CANVAS_AUTHORED_SURFACE_IDENTITY,
            runtime_root_identity: TECHNOLOGY_CANVAS_RUNTIME_ROOT_IDENTITY,
            effective_interface_winner: TECHNOLOGY_CANVAS_EFFECTIVE_INTERFACE_WINNER,
            callback_qpc_ticks: self.callback_qpc_ticks,
            game_hour_raw: self.game_hour_raw,
            controller_ptr: self.controller_ptr,
            active_folder_ordinal: self.active_folder_ordinal,
            active_folder_root_ptr: self.active_folder_root_ptr,
            runtime_graphical_subobject_ptr: self.runtime_graphical_subobject_ptr,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyCanvasHookDiagnostics {
    pub installed: bool,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub registered_owners: usize,
    pub owner_sequence: u64,
    pub commit_sequence: u64,
    pub pending_observations: usize,
    pub poisoned: bool,
}

#[derive(Clone, Debug)]
struct OwnerRecord {
    owner_root_ptr: usize,
    witness: OwnerRegistryWitness,
}

#[derive(Default)]
struct HookState {
    owner_sequence: u64,
    commit_sequence: u64,
    owners_by_controller: BTreeMap<usize, OwnerRecord>,
    observations: VecDeque<TechnologyCanvasHookBoundary>,
    poison: Option<String>,
}

pub struct A729TechnologyCanvasHookOwner {
    owner_target: usize,
    population_target: usize,
    refresh_target: usize,
    epoch: usize,
    active: bool,
}

impl A729TechnologyCanvasHookOwner {
    pub fn install(game_state_root_slot: usize) -> Result<Self, String> {
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "technology canvas install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("technology canvas hooks already have a live owner".into());
        }
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        let base = module as usize;
        if base == 0 {
            return Err("GetModuleHandleW(NULL) returned null".into());
        }
        verify_executable_sha256(module)?;
        verify_sentinel(
            base + compact_contract::build_profile::C_GAME_STATE_DAILY_UPDATE_OFFSET,
            &compact_contract::build_profile::DAILY_UPDATE_SENTINEL,
            "technology canvas daily-update anchor",
        )?;
        let expected_root_slot = base
            .checked_add(compact_contract::build_profile::CG_STATE_DATE_OFFSET)
            .ok_or("technology canvas game-state root-slot address overflow")?;
        if game_state_root_slot != expected_root_slot || read_usize(game_state_root_slot).is_none()
        {
            return Err(
                "technology canvas game-state root slot differs from exact a729 authority".into(),
            );
        }

        let owner_target = base + OWNER_CONSTRUCTOR_RVA;
        let population_target = base + POPULATION_COMMIT_RVA;
        let refresh_target = base + REFRESH_COMMIT_RVA;
        for (rva, target, label) in [
            (OWNER_CONSTRUCTOR_RVA, owner_target, "owner constructor"),
            (
                POPULATION_COMMIT_RVA,
                population_target,
                "population commit",
            ),
            (REFRESH_COMMIT_RVA, refresh_target, "refresh commit"),
        ] {
            let sentinel = exact_sentinel(rva)?;
            verify_sentinel(target, &sentinel, label)?;
        }

        reset_state()?;
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "technology canvas install epoch exhausted".to_string())?
            + 1;

        let original_owner = unsafe {
            MinHook::create_hook(
                owner_target as *mut c_void,
                hook_owner_constructor as *mut c_void,
            )
        }
        .map_err(|status| format!("technology canvas owner hook creation failed: {status}"))?;
        ORIGINAL_OWNER_CONSTRUCTOR.store(original_owner as usize, Ordering::Release);

        let original_population = match unsafe {
            MinHook::create_hook(
                population_target as *mut c_void,
                hook_population_commit as *mut c_void,
            )
        } {
            Ok(original) => original,
            Err(status) => {
                let cleaned = cleanup_created_hooks(&[owner_target]);
                if !cleaned {
                    return Err(format!(
                        "technology canvas rollback RETAINED created hooks; teardown still owed"
                    ));
                }
                return Err(format!(
                    "technology canvas population hook creation failed: {status}"
                ));
            }
        };
        ORIGINAL_POPULATION_COMMIT.store(original_population as usize, Ordering::Release);

        let original_refresh = match unsafe {
            MinHook::create_hook(
                refresh_target as *mut c_void,
                hook_refresh_commit as *mut c_void,
            )
        } {
            Ok(original) => original,
            Err(status) => {
                let cleaned = cleanup_created_hooks(&[population_target, owner_target]);
                if !cleaned {
                    return Err(format!(
                        "technology canvas rollback RETAINED created hooks; teardown still owed"
                    ));
                }
                return Err(format!(
                    "technology canvas refresh hook creation failed: {status}"
                ));
            }
        };
        ORIGINAL_REFRESH_COMMIT.store(original_refresh as usize, Ordering::Release);

        MODULE_BASE.store(base, Ordering::Release);
        GAME_STATE_ROOT_SLOT.store(game_state_root_slot, Ordering::Release);
        for (target, label) in [
            (refresh_target, "refresh"),
            (population_target, "population"),
        ] {
            if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
                OWNER_CALLBACKS_OPEN.store(false, Ordering::Release);
                COMMIT_CALLBACKS_OPEN.store(false, Ordering::Release);
                for rollback in [refresh_target, population_target, owner_target] {
                    let _ = unsafe { inline_detour::disable(rollback as *mut c_void) };
                }
                if let Err(quiescence) = wait_for_quiescence(Duration::from_secs(30)) {
                    return Err(format!(
                        "technology canvas {label} hook enable failed: {status}; disabled hooks could not quiesce: {quiescence}"
                    ));
                }
                let cleaned =
                    cleanup_created_hooks(&[refresh_target, population_target, owner_target]);
                if !cleaned {
                    return Err(format!(
                        "technology canvas rollback RETAINED created hooks; teardown still owed"
                    ));
                }
                return Err(format!(
                    "technology canvas {label} hook enable failed: {status}"
                ));
            }
        }
        OWNER_CALLBACKS_OPEN.store(true, Ordering::Release);
        if let Err(status) = unsafe { inline_detour::enable(owner_target as *mut c_void) } {
            OWNER_CALLBACKS_OPEN.store(false, Ordering::Release);
            COMMIT_CALLBACKS_OPEN.store(false, Ordering::Release);
            for rollback in [refresh_target, population_target, owner_target] {
                let _ = unsafe { inline_detour::disable(rollback as *mut c_void) };
            }
            if let Err(quiescence) = wait_for_quiescence(Duration::from_secs(30)) {
                return Err(format!(
                    "technology canvas owner hook enable failed: {status}; disabled hooks could not quiesce: {quiescence}"
                ));
            }
            let cleaned = cleanup_created_hooks(&[refresh_target, population_target, owner_target]);
            if !cleaned {
                return Err(format!(
                    "technology canvas rollback RETAINED created hooks; teardown still owed"
                ));
            }
            return Err(format!(
                "technology canvas owner hook enable failed: {status}"
            ));
        }
        COMMIT_CALLBACKS_OPEN.store(true, Ordering::Release);
        INSTALLED.store(true, Ordering::Release);
        Ok(Self {
            owner_target,
            population_target,
            refresh_target,
            epoch,
            active: true,
        })
    }

    pub fn diagnostics(&self) -> Result<TechnologyCanvasHookDiagnostics, String> {
        self.require_active()?;
        diagnostics()
    }

    pub fn drain_process_local_boundaries(
        &self,
    ) -> Result<Vec<TechnologyCanvasHookBoundary>, String> {
        self.require_active()?;
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("technology canvas hook poisoned: {error}"));
        }
        Ok(state.observations.drain(..).collect())
    }

    fn require_active(&self) -> Result<(), String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("technology canvas hook owner is not active".into());
        }
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("technology canvas hook owner epoch is stale".into());
        }
        Ok(())
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "technology canvas install mutex poisoned".to_string())?;
        OWNER_CALLBACKS_OPEN.store(false, Ordering::Release);
        COMMIT_CALLBACKS_OPEN.store(false, Ordering::Release);
        for (target, label) in [
            (self.refresh_target, "refresh"),
            (self.population_target, "population"),
            (self.owner_target, "owner"),
        ] {
            match unsafe { inline_detour::disable(target as *mut c_void) } {
                Ok(())
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                Err(status) => {
                    return Err(format!(
                        "technology canvas {label} hook disable failed: {status}"
                    ))
                }
            }
        }
        wait_for_quiescence(Duration::from_secs(30))?;
        let mut failures = Vec::new();
        for (target, label) in [
            (self.refresh_target, "refresh"),
            (self.population_target, "population"),
            (self.owner_target, "owner"),
        ] {
            match unsafe { inline_detour::remove(target as *mut c_void) } {
                Ok(())
                | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
                Err(status) => failures.push(format!("{label} removal failed: {status}")),
            }
        }
        if !failures.is_empty() {
            return Err(format!("technology canvas {}", failures.join(", ")));
        }
        ORIGINAL_REFRESH_COMMIT.store(0, Ordering::Release);
        ORIGINAL_POPULATION_COMMIT.store(0, Ordering::Release);
        ORIGINAL_OWNER_CONSTRUCTOR.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        GAME_STATE_ROOT_SLOT.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        self.active = false;
        Ok(())
    }
}

impl Drop for A729TechnologyCanvasHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

unsafe extern "system" fn hook_owner_constructor(owner: usize, context: usize) -> usize {
    let _in_flight = InFlight::enter();
    let result = call_original_owner(owner, context);
    if OWNER_CALLBACKS_OPEN.load(Ordering::Acquire) {
        if let Err(error) = register_owner(owner, result) {
            poison(error);
        }
    }
    result
}

unsafe extern "system" fn hook_population_commit(controller: usize) {
    let _in_flight = InFlight::enter();
    call_original_commit(
        ORIGINAL_POPULATION_COMMIT.load(Ordering::Acquire),
        controller,
    );
    if COMMIT_CALLBACKS_OPEN.load(Ordering::Acquire) {
        if let Err(error) = capture_commit(
            controller,
            POPULATION_COMMIT_RVA,
            ObserverBoundary::PopulationCommitted,
        ) {
            poison(error);
        }
    }
}

unsafe extern "system" fn hook_refresh_commit(controller: usize) {
    let _in_flight = InFlight::enter();
    call_original_commit(ORIGINAL_REFRESH_COMMIT.load(Ordering::Acquire), controller);
    if COMMIT_CALLBACKS_OPEN.load(Ordering::Acquire) {
        if let Err(error) = capture_commit(
            controller,
            REFRESH_COMMIT_RVA,
            ObserverBoundary::RefreshCommitted,
        ) {
            poison(error);
        }
    }
}

unsafe fn call_original_owner(owner: usize, context: usize) -> usize {
    let address = ORIGINAL_OWNER_CONSTRUCTOR.load(Ordering::Acquire);
    if address == 0 {
        return owner;
    }
    let original: OwnerConstructorFn = mem::transmute(address);
    original(owner, context)
}

unsafe fn call_original_commit(address: usize, controller: usize) {
    if address != 0 {
        let original: CommitFn = mem::transmute(address);
        original(controller);
    }
}

fn register_owner(owner: usize, result: usize) -> Result<(), String> {
    if owner == 0 || result != owner {
        return Err("technology canvas owner constructor returned foreign identity".into());
    }
    let controller = read_usize(owner + MODE6_OWNER_SLOT_OFFSET)
        .ok_or("technology canvas mode-6 owner slot unreadable")?;
    if controller == 0 {
        return Err("technology canvas mode-6 owner slot is null".into());
    }
    let mode = read_u32(controller + CONTROLLER_MODE_OFFSET)
        .ok_or("technology canvas controller mode unreadable")?;
    if mode != TECHNOLOGY_MODE {
        return Err(format!(
            "technology canvas owner registered controller mode {mode}, expected {TECHNOLOGY_MODE}"
        ));
    }
    let qpc = qpc()?;
    let mut state = state_lock()?;
    state.owner_sequence = state
        .owner_sequence
        .checked_add(1)
        .ok_or("technology canvas owner sequence exhausted")?;
    let registry_manifest_sha256 = registry_manifest_sha256();
    let mut witness = OwnerRegistryWitness {
        registry_epoch: INSTALL_EPOCH.load(Ordering::Acquire) as u64,
        owner_sequence: state.owner_sequence,
        construction_qpc_ticks: qpc,
        owner_construction_record_sha256: String::new(),
        registry_manifest_sha256,
    };
    witness.owner_construction_record_sha256 = expected_owner_construction_record_sha256(&witness);
    state.owners_by_controller.insert(
        controller,
        OwnerRecord {
            owner_root_ptr: owner,
            witness,
        },
    );
    Ok(())
}

fn capture_commit(
    controller: usize,
    callback_rva: usize,
    boundary: ObserverBoundary,
) -> Result<(), String> {
    if controller == 0 {
        return Err("technology canvas commit received null controller".into());
    }
    let mode = read_u32(controller + CONTROLLER_MODE_OFFSET)
        .ok_or("technology canvas commit controller mode unreadable")?;
    if mode != TECHNOLOGY_MODE {
        return Err(format!(
            "technology canvas commit observed controller mode {mode}, expected {TECHNOLOGY_MODE}"
        ));
    }
    let game_hour_raw = read_stable_game_hour_raw()?;
    let qpc = qpc()?;
    let folder_count = bounded_nonnegative(
        read_i32(controller + ACTIVE_FOLDER_COUNT_OFFSET)
            .ok_or("technology canvas active-folder count unreadable")?,
        "active-folder count",
    )?;
    if folder_count == 0 {
        return Err("technology canvas committed with no active folder roots".into());
    }
    let folder_ordinal_raw = read_i32(controller + ACTIVE_FOLDER_ORDINAL_OFFSET)
        .ok_or("technology canvas active-folder ordinal unreadable")?;
    let folder_ordinal = usize::try_from(folder_ordinal_raw)
        .map_err(|_| "technology canvas active-folder ordinal is negative".to_string())?;
    if folder_ordinal >= folder_count {
        return Err(format!(
            "technology canvas active-folder ordinal {folder_ordinal} is outside count {folder_count}"
        ));
    }
    let folder_vector = read_usize(controller + ACTIVE_FOLDER_VECTOR_OFFSET)
        .ok_or("technology canvas active-folder vector unreadable")?;
    if folder_vector == 0 {
        return Err("technology canvas active-folder vector is null".into());
    }
    let active_root = read_usize(
        folder_vector
            .checked_add(folder_ordinal * mem::size_of::<usize>())
            .ok_or("technology canvas active-folder address overflow")?,
    )
    .ok_or("technology canvas active-folder root unreadable")?;
    let graphical_subobject = active_root
        .checked_add(GUI_SUBOBJECT_OFFSET)
        .ok_or("technology canvas graphical-subobject address overflow")?;
    if active_root == 0
        || read_usize(graphical_subobject)
            .filter(|vtable| *vtable != 0)
            .is_none()
    {
        return Err("technology canvas active-folder GUI root is invalid".into());
    }

    let mut state = state_lock()?;
    let owner = state
        .owners_by_controller
        .get(&controller)
        .cloned()
        .ok_or("technology canvas commit has no constructor-owned registry entry")?;
    let owner_slot = read_usize(owner.owner_root_ptr + MODE6_OWNER_SLOT_OFFSET)
        .ok_or("technology canvas owner mode-6 slot unreadable at commit")?;
    if owner_slot != controller {
        return Err("technology canvas owner/controller relationship changed before commit".into());
    }
    state.commit_sequence = state
        .commit_sequence
        .checked_add(1)
        .ok_or("technology canvas commit sequence exhausted")?;
    let commit_sequence = state.commit_sequence;
    if state.observations.len() >= MAX_PENDING {
        return Err("technology canvas pending observation bound exceeded".into());
    }
    state.observations.push_back(TechnologyCanvasHookBoundary {
        commit_sequence,
        callback_rva,
        boundary,
        callback_qpc_ticks: qpc,
        game_hour_raw,
        owner_root_ptr: owner.owner_root_ptr,
        controller_ptr: controller,
        owner_mode6_slot_value: owner_slot,
        owner_registry_witness: owner.witness,
        active_folder_ordinal: folder_ordinal as u32,
        active_folder_count: folder_count as u32,
        active_folder_root_ptr: active_root,
        runtime_graphical_subobject_ptr: graphical_subobject,
    });
    Ok(())
}

fn exact_sentinel(rva: usize) -> Result<[u8; 16], String> {
    let contract = AUTHENTICATED_EXACT20_HOOK_CONTRACT
        .ok_or("technology canvas exact-a729 hook contract is absent")?;
    contract
        .function_sites
        .iter()
        .find(|site| site.rva == rva)
        .map(|site| site.entry_sentinel)
        .ok_or_else(|| format!("technology canvas contract has no sentinel for RVA 0x{rva:X}"))
}

fn registry_manifest_sha256() -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"the game:a729:technology-canvas-owner-registry-topology:v1\0");
    bytes.extend_from_slice(A729_EXECUTABLE_SHA256.as_bytes());
    bytes.extend_from_slice(&(OWNER_CONSTRUCTOR_RVA as u64).to_le_bytes());
    bytes.extend_from_slice(&(MODE6_OWNER_SLOT_OFFSET as u64).to_le_bytes());
    bytes.extend_from_slice(&(CONTROLLER_MODE_OFFSET as u64).to_le_bytes());
    bytes.extend_from_slice(&(POPULATION_COMMIT_RVA as u64).to_le_bytes());
    bytes.extend_from_slice(&(REFRESH_COMMIT_RVA as u64).to_le_bytes());
    hex_digest(Sha256::digest(bytes))
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = digest.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn diagnostics() -> Result<TechnologyCanvasHookDiagnostics, String> {
    let state = state_lock()?;
    Ok(TechnologyCanvasHookDiagnostics {
        installed: INSTALLED.load(Ordering::Acquire),
        callbacks_open: OWNER_CALLBACKS_OPEN.load(Ordering::Acquire)
            && COMMIT_CALLBACKS_OPEN.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT.load(Ordering::Acquire),
        registered_owners: state.owners_by_controller.len(),
        owner_sequence: state.owner_sequence,
        commit_sequence: state.commit_sequence,
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
        .map_err(|_| "technology canvas hook state mutex poisoned".to_string())
}

fn poison(error: String) {
    if let Ok(mut state) = state_lock() {
        state.poison.get_or_insert(error);
    }
}

#[must_use = "a retained target is still owed a teardown and must reach the caller's error"]
fn cleanup_created_hooks(targets: &[usize]) -> bool {
    OWNER_CALLBACKS_OPEN.store(false, Ordering::Release);
    COMMIT_CALLBACKS_OPEN.store(false, Ordering::Release);
    let mut removed = true;
    for target in targets {
        if unsafe { inline_detour::remove(*target as *mut c_void) }.is_err() {
            removed = false;
        }
    }
    if !removed {
        return false;
    }
    ORIGINAL_REFRESH_COMMIT.store(0, Ordering::Release);
    ORIGINAL_POPULATION_COMMIT.store(0, Ordering::Release);
    ORIGINAL_OWNER_CONSTRUCTOR.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    GAME_STATE_ROOT_SLOT.store(0, Ordering::Release);
    true
}

fn verify_executable_sha256(module: *mut c_void) -> Result<(), String> {
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err("technology canvas executable path is unavailable or truncated".into());
    }
    let path = String::from_utf16(&buffer[..length as usize])
        .map_err(|_| "technology canvas executable path is not valid UTF-16".to_string())?;
    let mut file = File::open(path)
        .map_err(|error| format!("technology canvas executable open failed: {error}"))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("technology canvas executable hash read failed: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    if hex_digest(hasher.finalize()) != A729_EXECUTABLE_SHA256 {
        return Err("technology canvas running executable SHA-256 differs from exact a729".into());
    }
    Ok(())
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual =
        rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn bounded_nonnegative(value: i32, label: &str) -> Result<usize, String> {
    let value = usize::try_from(value).map_err(|_| format!("{label} is negative"))?;
    if value > MAX_FOLDER_ROOTS {
        return Err(format!("{label} {value} exceeds bound {MAX_FOLDER_ROOTS}"));
    }
    Ok(value)
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err("technology canvas hook callbacks did not quiesce".into());
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

fn read_stable_game_hour_raw() -> Result<i32, String> {
    let root_slot = GAME_STATE_ROOT_SLOT.load(Ordering::Acquire);
    if root_slot == 0 {
        return Err("technology canvas game-state root slot is unbound".into());
    }
    let game_state_before = read_usize(root_slot)
        .filter(|pointer| *pointer != 0)
        .ok_or("technology canvas game-state root is null or unreadable")?;
    let hour_address = game_state_before
        .checked_add(compact_contract::build_profile::CGAMESTATE_CURRENT_HOUR)
        .ok_or("technology canvas current-hour address overflow")?;
    let hour_before =
        read_i32(hour_address).ok_or("technology canvas current hour is unreadable")?;
    let hour_after =
        read_i32(hour_address).ok_or("technology canvas current hour is unreadable")?;
    let game_state_after = read_usize(root_slot)
        .filter(|pointer| *pointer != 0)
        .ok_or("technology canvas game-state root is null or unreadable")?;
    if game_state_before != game_state_after || hour_before != hour_after {
        return Err("technology canvas game-state root/hour changed during stable read".into());
    }
    compact_contract::calendar::hour_index_from_raw(hour_before)
        .ok_or("technology canvas current hour is outside the canonical calendar range")?;
    Ok(hour_before)
}

struct InFlight;

impl InFlight {
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
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
