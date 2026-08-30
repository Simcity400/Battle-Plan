use crate::a729_air_mio_controller::{
    compute_identity_record_sha256, A729AirMioControllerSource, AirMioAuthorityReceipt,
    AirMioControllerKind, AirMioControllerObservation, ControllerCandidate, ControllerCandidateKey,
    ControllerSlot, ControllerSourceLane, PortableItemIdentity, PortableObjectResolverRecord,
    RejectionEvidence, ResolvedListState, ResolverInputSource, RuntimeRowTemplate, A729_BINARY_ID,
    A729_EXECUTABLE_SHA256, FINAL_FAMILY_JOB_ID, FINAL_FAMILY_MANIFEST_SHA256,
    PAIR_TO_OBJECT_RESOLVER_JOB_ID, PAIR_TO_OBJECT_RESOLVER_MANIFEST_SHA256,
    PAIR_TO_OBJECT_RESOLVER_RVA, REFERENCE_TABLE_LOOKUP_RVA,
};
use crate::RectI32;
use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, VecDeque};
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

pub const MIO_BONUS_REBUILD_RVA: usize = 0x01F0_5960;
pub const MIO_BONUS_HOOK_JOB_ID: &str =
    "20260722T071816Z-a28-mio-history-and-bonus-final-exhausti-08b8a7f4";
pub const MIO_BONUS_HOOK_MANIFEST_SHA256: &str =
    "a182725f84e611185fb3c4284130d1db27bae22763dd4b9f11470df8c6ef76ab";
/// Retained exact-a729 export that closes the side-effect-free scalar ABI used
pub const MIO_BONUS_AGGREGATE_JOB_ID: &str =
    "20260717T110616Z-x15-trade-modifier-container-lookups-f55331ec";
pub const MIO_BONUS_AGGREGATE_MANIFEST_SHA256: &str =
    "aede22bdb156b49c703d70ac6934ed6370bbe4c48b828bb938502b778c9c6471";
pub const MIO_BONUS_AGGREGATE_DECOMPILE_SHA256: &str =
    "3a2f3c18ac191c848eeaf433a90d490a4bfa6c3d271a1c3398ebb0990513c126";

const CONTROLLER_ORGANISATION_OFFSET: usize = 0x30;
const CONTROLLER_SEPARATOR_VECTOR_OFFSET: usize = 0x2078;
const CONTROLLER_FIXED_VECTOR_OFFSET: usize = 0x2090;
const CONTROLLER_EQUIPMENT_VECTOR_OFFSET: usize = 0x20A8;
const CONTROLLER_OPTIONAL_BONUS_OFFSET: usize = 0x20C0;
const CONTROLLER_LIST_OFFSET: usize = 0x3860;

const VECTOR_COUNT_OFFSET: usize = 0x0C;
const OBJECT_SELF_REFERENCE_OFFSET: usize = 0x08;
const CONTROLLER_ORGANISATION_REFERENCE_OFFSET: usize = 0x84;
const ORGANISATION_EQUIPMENT_DATABASE_OFFSET: usize = 0x110;
const EQUIPMENT_DATABASE_OPTIONAL_TRAIT_OFFSET: usize = 0x28;
const EQUIPMENT_DATABASE_GROUP_VECTOR_OFFSET: usize = 0x98;
const EQUIPMENT_DATABASE_GROUP_COUNT_OFFSET: usize = 0xA4;
const EQUIPMENT_GROUP_STRIDE: usize = 0x28;
const EQUIPMENT_GROUP_SEMANTIC_KEY_OFFSET: usize = 0x20;

use crate::standard_list::{
    COUNT_OFFSET as LIST_COUNT_OFFSET, HEAD_OFFSET as LIST_HEAD_OFFSET, NODE_ITEM_OFFSET,
    NODE_NEXT_OFFSET, PRIMARY_VTABLE_RVA as STANDARD_LIST_PRIMARY_VTABLE_RVA,
    SCROLLBAR_OFFSET as LIST_SCROLLBAR_OFFSET,
    SECONDARY_VTABLE_RVA as STANDARD_LIST_SECONDARY_VTABLE_RVA,
};

const ROW_GUI_OFFSET: usize = 0x38;
use crate::standard_list::scrollbar::{
    CURRENT_OFFSET as SCROLL_CURRENT_OFFSET, MAX_OFFSET as SCROLL_MAX_OFFSET,
    MIN_OFFSET as SCROLL_MIN_OFFSET, PLUS_80_VTABLE_RVA as EXTENDED_SCROLLBAR_PLUS_80_VTABLE_RVA,
    PLUS_B0_VTABLE_RVA as EXTENDED_SCROLLBAR_PLUS_B0_VTABLE_RVA,
    PRIMARY_VTABLE_RVA as EXTENDED_SCROLLBAR_PRIMARY_VTABLE_RVA,
    TARGET_OFFSET as SCROLL_TARGET_OFFSET,
};


const MODIFIER_ROW_PRIMARY_VTABLE_RVA: usize = 0x02A8_DD08;
const MODIFIER_ROW_GUI_VTABLE_RVA: usize = 0x02A8_DD78;
const BONUS_ROW_PRIMARY_VTABLE_RVA: usize = 0x02A8_DE58;
const BONUS_ROW_GUI_VTABLE_RVA: usize = 0x02A8_DEC8;
const EQUIPMENT_ROW_PRIMARY_VTABLE_RVA: usize = 0x02A8_E0E0;
const EQUIPMENT_ROW_GUI_VTABLE_RVA: usize = 0x02A8_E150;
const SEPARATOR_ROW_PRIMARY_VTABLE_RVA: usize = 0x02A8_DBD0;
const SEPARATOR_ROW_GUI_VTABLE_RVA: usize = 0x02A8_DC40;

const MIO_FIXED_PREDICATE_RVA: u32 = 0x01F0_6630;
const MIO_EQUIPMENT_PREDICATE_RVA: u32 = 0x01F0_5CD0;
const MIO_SECTION_PREDICATE_RVA: u32 = 0x01F0_5960;
const AGGREGATE_MODIFIER_RVA: usize = 0x00DA_5230;
const FIXED_BONUS_GLOBAL_RVA: usize = 0x0331_8B84;
const FIXED_FLAG_268_RVA: usize = 0x0331_B570;
const FIXED_FLAG_269_RVA: usize = 0x0331_B9B8;
const FIXED_FLAG_26A_RVA: usize = 0x0331_B600;

const FIXED_SECTION_KEY: u32 = 0x01F0_5A3C;
const EQUIPMENT_SECTION_KEY: u32 = 0x01F0_5BAA;
const OPTIONAL_BONUS_KEY: u32 = 0x01F0_6032;
const FIXED_KEYS: [u32; 8] = [0x26B, 0x267, 0x268, 0x269, 0x26A, 0x26D, 0x26E, 0x26F];

const MAX_PENDING: usize = 4_096;
const MAX_ROWS: usize = crate::MAX_ITEMS_PER_COLLECTION;
const MIO_BONUS_REBUILD_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x54, 0x41, 0x56,
];

type MioBonusRebuildFn = unsafe extern "system" fn(usize);
type PairResolverFn = unsafe extern "system" fn(*const u32) -> usize;
type GeometryFn = unsafe extern "system" fn(usize, *mut i32) -> *mut i32;
type AggregateModifierFn =
    unsafe extern "system" fn(usize, *mut i64, i64, i32, i64, u8, usize) -> *mut i64;

static ORIGINAL_REBUILD: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static CALLBACKS_OPEN: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static INSTALL_EPOCH: AtomicUsize = AtomicUsize::new(0);
static GAME_CLOCK_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();

#[derive(Default)]
struct HookState {
    controller_calls: u64,
    transition_sequence: u64,
    observations: VecDeque<AirMioControllerObservation>,
    poison: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MioBonusHookDiagnostics {
    pub installed: bool,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub controller_calls: u64,
    pub transition_sequence: u64,
    pub pending_observations: usize,
    pub poisoned: bool,
}

pub struct A729MioBonusHookOwner {
    target: usize,
    epoch: usize,
    active: bool,
}

impl A729MioBonusHookOwner {
    pub fn install(game_clock_address: usize) -> Result<Self, String> {
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 MIO bonus install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("A28 MIO bonus hook already has a live owner".into());
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
            "A28 MIO bonus daily-update anchor",
        )?;
        verify_sentinel(
            base + compact_contract::build_profile::GET_LONG_STRING_OFFSET,
            &compact_contract::build_profile::GET_LONG_STRING_SENTINEL,
            "A28 MIO bonus long-string anchor",
        )?;
        let exact_clock = read_usize(base + compact_contract::build_profile::CG_STATE_DATE_OFFSET)
            .ok_or("A28 MIO bonus exact game-clock root is unreadable")?;
        if game_clock_address != exact_clock {
            return Err("A28 MIO bonus game-clock address differs from the exact a729 root".into());
        }
        if game_clock_address == 0 || read_i32(game_clock_address).is_none() {
            return Err("A28 MIO bonus game-clock address is unreadable".into());
        }
        let target = base + MIO_BONUS_REBUILD_RVA;
        verify_sentinel(target, &MIO_BONUS_REBUILD_SENTINEL, "MIO bonus rebuild")?;
        reset_state()?;
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "A28 MIO bonus install epoch exhausted".to_string())?
            + 1;
        let original = unsafe {
            MinHook::create_hook(target as *mut c_void, hook_mio_bonus_rebuild as *mut c_void)
        }
        .map_err(|status| format!("A28 MIO bonus hook creation failed: {status}"))?;
        ORIGINAL_REBUILD.store(original as usize, Ordering::Release);
        MODULE_BASE.store(base, Ordering::Release);
        GAME_CLOCK_ADDRESS.store(game_clock_address, Ordering::Release);
        CALLBACKS_OPEN.store(true, Ordering::Release);
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            CALLBACKS_OPEN.store(false, Ordering::Release);
            let _ = unsafe { inline_detour::disable(target as *mut c_void) };
            if let Err(quiescence) = wait_for_quiescence(Duration::from_secs(30)) {
                return Err(format!(
                    "A28 MIO bonus hook enable failed: {status}; detour retained: {quiescence}"
                ));
            }
            if let Err(removal) = unsafe { inline_detour::remove(target as *mut c_void) } {
                return Err(format!(
                    "A28 MIO bonus hook enable failed: {status}; detour retained, removal failed: {removal}"
                ));
            }
            ORIGINAL_REBUILD.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            GAME_CLOCK_ADDRESS.store(0, Ordering::Release);
            return Err(format!("A28 MIO bonus hook enable failed: {status}"));
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(Self {
            target,
            epoch,
            active: true,
        })
    }

    pub fn diagnostics(&self) -> Result<MioBonusHookDiagnostics, String> {
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 MIO bonus hook owner epoch is stale".into());
        }
        diagnostics()
    }

    pub fn drain_observations(&self) -> Result<Vec<AirMioControllerObservation>, String> {
        self.require_active()?;
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 MIO bonus hook poisoned: {error}"));
        }
        Ok(state.observations.drain(..).collect())
    }

    pub fn drain_latest_source(
        &self,
        game_hour_raw: i32,
    ) -> Result<A729AirMioControllerSource, String> {
        self.require_active()?;
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 MIO bonus hook poisoned: {error}"));
        }
        let observation = state
            .observations
            .pop_back()
            .ok_or("A28 MIO bonus hook has no complete post-return observation")?;
        state.observations.clear();
        drop(state);
        if observation.authority.captured_game_hour_raw != game_hour_raw {
            return Err(format!(
                "A28 MIO bonus observation raw hour differs from drain hour {game_hour_raw}"
            ));
        }
        let frame_hour = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
            .and_then(|hour| u64::try_from(hour).ok())
            .ok_or("A28 MIO bonus captured game hour is not a compact calendar index")?;
        A729AirMioControllerSource::admit_hook_owned_mio_bonus(frame_hour, vec![observation])
    }

    fn require_active(&self) -> Result<(), String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A28 MIO bonus hook owner is not active".into());
        }
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 MIO bonus hook owner epoch is stale".into());
        }
        Ok(())
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 MIO bonus install mutex poisoned".to_string())?;
        CALLBACKS_OPEN.store(false, Ordering::Release);
        match unsafe { inline_detour::disable(self.target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!("A28 MIO bonus hook disable failed: {status}"));
            }
        }
        wait_for_quiescence(Duration::from_secs(30))?;
        match unsafe { inline_detour::remove(self.target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!("A28 MIO bonus hook removal failed: {status}"));
            }
        }
        ORIGINAL_REBUILD.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        GAME_CLOCK_ADDRESS.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        self.active = false;
        Ok(())
    }
}

impl Drop for A729MioBonusHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

unsafe extern "system" fn hook_mio_bonus_rebuild(controller: usize) {
    let _in_flight = InFlight::enter();
    call_original(controller);
    if !CALLBACKS_OPEN.load(Ordering::Acquire) {
        return;
    }
    if let Err(error) = capture_post_return(controller) {
        poison(error);
    }
}

unsafe fn call_original(controller: usize) {
    let address = ORIGINAL_REBUILD.load(Ordering::Acquire);
    if address != 0 {
        let original: MioBonusRebuildFn = mem::transmute(address);
        original(controller);
    }
}

#[derive(Clone, Copy)]
struct FixedDisposition {
    semantic_key: u32,
    included: bool,
    deciding_value: i64,
}

#[derive(Clone, Copy)]
struct ExpectedRow {
    key: ControllerCandidateKey,
    row: usize,
    template: RuntimeRowTemplate,
}

fn capture_post_return(controller: usize) -> Result<(), String> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("module base absent during A28 MIO bonus callback".into());
    }
    let captured_game_hour_raw = read_i32(GAME_CLOCK_ADDRESS.load(Ordering::Acquire))
        .ok_or("A28 MIO bonus callback game clock unreadable")?;
    let organisation_handle = read_usize(controller + CONTROLLER_ORGANISATION_OFFSET)
        .ok_or("MIO bonus organisation handle unreadable")?;
    if organisation_handle == 0 {
        return Err("MIO bonus rebuild has a null organisation handle".into());
    }
    let organisation_pair = read_valid_pair(
        organisation_handle + CONTROLLER_ORGANISATION_REFERENCE_OFFSET,
        "MIO bonus controller organisation reference",
    )?;
    let organisation = resolve_pair_pointer(base, organisation_pair)?;
    let resolver = resolve_identity(base, organisation_pair)?;
    let resolver_digest =
        compute_identity_record_sha256(AirMioControllerKind::MioAggregatedBonus, &resolver);

    let fixed = fixed_dispositions(base, organisation)?;
    let fixed_included = fixed.iter().filter(|item| item.included).count();
    let fixed_rows = pointer_vector(
        controller + CONTROLLER_FIXED_VECTOR_OFFSET,
        "MIO fixed bonus vector",
    )?;
    if fixed_rows.len() != fixed_included {
        return Err(format!(
            "MIO fixed bonus predicate/vector counts differ: {fixed_included}/{}",
            fixed_rows.len()
        ));
    }

    let equipment_database = read_usize(organisation + ORGANISATION_EQUIPMENT_DATABASE_OFFSET)
        .ok_or("MIO equipment database pointer unreadable")?;
    if equipment_database == 0 {
        return Err("MIO equipment database pointer is null".into());
    }
    let optional_source = read_usize(equipment_database + EQUIPMENT_DATABASE_OPTIONAL_TRAIT_OFFSET)
        .ok_or("MIO optional initial-trait source unreadable")?;
    let optional_row = read_usize(controller + CONTROLLER_OPTIONAL_BONUS_OFFSET)
        .ok_or("MIO optional bonus row pointer unreadable")?;
    if (optional_source != 0) != (optional_row != 0) {
        return Err("MIO optional bonus source/row presence differs".into());
    }

    let group_count = bounded_count(
        read_i32(equipment_database + EQUIPMENT_DATABASE_GROUP_COUNT_OFFSET)
            .ok_or("MIO equipment-group count unreadable")?,
        "MIO equipment-group",
    )?;
    let group_source = read_usize(equipment_database + EQUIPMENT_DATABASE_GROUP_VECTOR_OFFSET)
        .ok_or("MIO equipment-group source vector unreadable")?;
    if group_count != 0 && group_source == 0 {
        return Err("MIO equipment-group source vector is null".into());
    }
    let equipment_rows = pointer_vector(
        controller + CONTROLLER_EQUIPMENT_VECTOR_OFFSET,
        "MIO equipment-group row vector",
    )?;
    if equipment_rows.len() != group_count {
        return Err(format!(
            "MIO equipment source/row counts differ: {group_count}/{}",
            equipment_rows.len()
        ));
    }

    let fixed_section_present = fixed_included != 0;
    let equipment_section_present = optional_source != 0 || group_count != 0;
    let separators = pointer_vector(
        controller + CONTROLLER_SEPARATOR_VECTOR_OFFSET,
        "MIO bonus separator vector",
    )?;
    let expected_separator_count = (if fixed_section_present { 1 } else { 0 })
        + (if equipment_section_present { 1 } else { 0 });
    if separators.len() != expected_separator_count {
        return Err(format!(
            "MIO bonus separator count differs: {expected_separator_count}/{}",
            separators.len()
        ));
    }

    let mut candidates = Vec::new();
    let mut expected_rows = Vec::new();
    let mut separator_cursor = 0usize;
    let mut fixed_cursor = 0usize;
    let mut source_ordinal = 0u32;

    let fixed_section_key = atom_key(
        ControllerSourceLane::MioFixedBonus,
        FIXED_SECTION_KEY,
        MIO_SECTION_PREDICATE_RVA,
        organisation_pair,
    );
    let fixed_section_rejection = (!fixed_section_present).then(|| {
        rejection_evidence(
            ControllerSourceLane::MioFixedBonus,
            FIXED_SECTION_KEY as u64,
            FIXED_SECTION_KEY,
            "mio_fixed_section_empty",
            MIO_SECTION_PREDICATE_RVA,
            fixed_included as i64,
        )
    });
    candidates.push(candidate(
        fixed_section_key,
        resolver.clone(),
        &resolver_digest,
        source_ordinal,
        fixed_section_rejection,
    ));
    source_ordinal += 1;
    if fixed_section_present {
        expected_rows.push(ExpectedRow {
            key: fixed_section_key,
            row: separators[separator_cursor],
            template: RuntimeRowTemplate::MioDetailSeparator,
        });
        separator_cursor += 1;
    }

    for disposition in fixed {
        let key = atom_key(
            ControllerSourceLane::MioFixedBonus,
            disposition.semantic_key,
            MIO_FIXED_PREDICATE_RVA,
            organisation_pair,
        );
        let rejection = (!disposition.included).then(|| {
            rejection_evidence(
                ControllerSourceLane::MioFixedBonus,
                disposition.semantic_key as u64,
                disposition.semantic_key,
                fixed_rejection_reason(disposition.semantic_key),
                MIO_FIXED_PREDICATE_RVA,
                disposition.deciding_value,
            )
        });
        candidates.push(candidate(
            key,
            resolver.clone(),
            &resolver_digest,
            source_ordinal,
            rejection,
        ));
        source_ordinal = source_ordinal
            .checked_add(1)
            .ok_or("MIO bonus source ordinal overflow")?;
        if disposition.included {
            expected_rows.push(ExpectedRow {
                key,
                row: fixed_rows[fixed_cursor],
                template: RuntimeRowTemplate::MioDetailModifierItem,
            });
            fixed_cursor += 1;
        }
    }

    let equipment_section_key = atom_key(
        ControllerSourceLane::MioEquipmentGroup,
        EQUIPMENT_SECTION_KEY,
        MIO_SECTION_PREDICATE_RVA,
        organisation_pair,
    );
    let equipment_section_rejection = (!equipment_section_present).then(|| {
        rejection_evidence(
            ControllerSourceLane::MioEquipmentGroup,
            EQUIPMENT_SECTION_KEY as u64,
            EQUIPMENT_SECTION_KEY,
            "mio_equipment_section_empty",
            MIO_SECTION_PREDICATE_RVA,
            (group_count + if optional_source != 0 { 1 } else { 0 }) as i64,
        )
    });
    candidates.push(candidate(
        equipment_section_key,
        resolver.clone(),
        &resolver_digest,
        source_ordinal,
        equipment_section_rejection,
    ));
    source_ordinal = source_ordinal
        .checked_add(1)
        .ok_or("MIO bonus source ordinal overflow")?;
    if equipment_section_present {
        expected_rows.push(ExpectedRow {
            key: equipment_section_key,
            row: separators[separator_cursor],
            template: RuntimeRowTemplate::MioDetailSeparator,
        });
        separator_cursor += 1;
    }

    let optional_key = atom_key(
        ControllerSourceLane::MioEquipmentGroup,
        OPTIONAL_BONUS_KEY,
        MIO_EQUIPMENT_PREDICATE_RVA,
        organisation_pair,
    );
    let optional_rejection = (optional_source == 0).then(|| {
        rejection_evidence(
            ControllerSourceLane::MioEquipmentGroup,
            OPTIONAL_BONUS_KEY as u64,
            OPTIONAL_BONUS_KEY,
            "mio_initial_trait_absent",
            MIO_EQUIPMENT_PREDICATE_RVA,
            0,
        )
    });
    candidates.push(candidate(
        optional_key,
        resolver.clone(),
        &resolver_digest,
        source_ordinal,
        optional_rejection,
    ));
    source_ordinal = source_ordinal
        .checked_add(1)
        .ok_or("MIO bonus source ordinal overflow")?;
    if optional_source != 0 {
        expected_rows.push(ExpectedRow {
            key: optional_key,
            row: optional_row,
            template: RuntimeRowTemplate::MioDetailBonusItem,
        });
    }

    let mut group_keys = BTreeSet::new();
    for ordinal in 0..group_count {
        let record = checked_stride(
            group_source,
            ordinal,
            EQUIPMENT_GROUP_STRIDE,
            "MIO equipment-group",
        )?;
        let semantic_key = read_u32(record + EQUIPMENT_GROUP_SEMANTIC_KEY_OFFSET)
            .ok_or("MIO equipment-group semantic key unreadable")?;
        if semantic_key == 0 || !group_keys.insert(semantic_key) {
            return Err("MIO equipment-group semantic key is zero or duplicated".into());
        }
        let key = atom_key(
            ControllerSourceLane::MioEquipmentGroup,
            semantic_key,
            MIO_EQUIPMENT_PREDICATE_RVA,
            organisation_pair,
        );
        candidates.push(candidate(
            key,
            resolver.clone(),
            &resolver_digest,
            source_ordinal,
            None,
        ));
        source_ordinal = source_ordinal
            .checked_add(1)
            .ok_or("MIO bonus source ordinal overflow")?;
        expected_rows.push(ExpectedRow {
            key,
            row: equipment_rows[ordinal],
            template: RuntimeRowTemplate::MioEquipmentTypeWindow,
        });
    }
    if separator_cursor != separators.len() || fixed_cursor != fixed_rows.len() {
        return Err("MIO bonus row-vector accounting did not close".into());
    }

    let list = read_usize(controller + CONTROLLER_LIST_OFFSET)
        .ok_or("MIO bonus list pointer unreadable")?;
    gate_vtable(
        list,
        base + STANDARD_LIST_PRIMARY_VTABLE_RVA,
        "MIO bonus list primary",
    )?;
    gate_vtable(
        list + 8,
        base + STANDARD_LIST_SECONDARY_VTABLE_RVA,
        "MIO bonus list secondary",
    )?;
    let list_count = bounded_count(
        read_i32(list + LIST_COUNT_OFFSET).ok_or("MIO bonus list count unreadable")?,
        "MIO bonus list",
    )?;
    if list_count != expected_rows.len() {
        return Err(format!(
            "MIO bonus materialized/list counts differ: {}/{}",
            expected_rows.len(),
            list_count
        ));
    }

    let mut accepted_order = Vec::with_capacity(expected_rows.len());
    let mut slots = Vec::with_capacity(expected_rows.len());
    let mut node = read_usize(list + LIST_HEAD_OFFSET).ok_or("MIO bonus list head unreadable")?;
    for (ordinal, expected) in expected_rows.into_iter().enumerate() {
        if expected.row == 0 {
            return Err(format!("MIO bonus row {ordinal} is null"));
        }
        gate_row_template(base, expected.row, expected.template)?;
        if node == 0 {
            return Err(format!("MIO bonus list ended at row {ordinal}"));
        }
        let row_gui = expected.row + ROW_GUI_OFFSET;
        let node_gui =
            read_usize(node + NODE_ITEM_OFFSET).ok_or("MIO bonus list-node item unreadable")?;
        if node_gui != row_gui {
            return Err(format!(
                "MIO bonus source/list traversal differs at row {ordinal}"
            ));
        }
        let position = invoke_geometry(row_gui, 0x78, "MIO bonus row resolved position")?;
        let extent = invoke_geometry(row_gui, 0x28, "MIO bonus row resolved extent")?;
        require_positive_extent(extent, "MIO bonus row")?;
        let stable_slot_id = u32::try_from(ordinal)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or("MIO bonus stable slot overflow")?;
        accepted_order.push(expected.key);
        slots.push(ControllerSlot {
            stable_slot_id,
            bound_candidate: expected.key,
            template: expected.template,
            rect: RectI32 {
                x: position[0],
                y: position[1],
                width: extent[0],
                height: extent[1],
            },
        });
        node = read_usize(node + NODE_NEXT_OFFSET).ok_or("MIO bonus next list link unreadable")?;
    }
    if node != 0 {
        return Err("MIO bonus list contains more nodes than authenticated count".into());
    }
    let included_keys = candidates
        .iter()
        .filter(|candidate| candidate.rejection.is_none())
        .map(|candidate| candidate.key)
        .collect::<BTreeSet<_>>();
    if accepted_order.iter().copied().collect::<BTreeSet<_>>() != included_keys
        || accepted_order.len() != included_keys.len()
    {
        return Err("MIO bonus accepted order differs from complete source dispositions".into());
    }

    let list_state = capture_list_state(base, list)?;
    let producer_record_sha256 = producer_digest(
        organisation_pair,
        &candidates,
        &accepted_order,
        &slots,
        &list_state,
    );
    let qpc_ticks = qpc()?;
    let mut state = state_lock()?;
    state.controller_calls = state.controller_calls.saturating_add(1);
    state.transition_sequence = state
        .transition_sequence
        .checked_add(1)
        .ok_or("A28 MIO bonus transition sequence exhausted")?;
    let observation = AirMioControllerObservation {
        kind: AirMioControllerKind::MioAggregatedBonus,
        authority: AirMioAuthorityReceipt {
            binary_id: A729_BINARY_ID.into(),
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            function_job_id: FINAL_FAMILY_JOB_ID.into(),
            function_manifest_sha256: FINAL_FAMILY_MANIFEST_SHA256.into(),
            producer_rvas: vec![0x01F0_5960, 0x01F0_6630, 0x01F0_5CD0, 0x01FA_4B90],
            transition_sequence: state.transition_sequence,
            observation_qpc_ticks: qpc_ticks,
            captured_game_hour_raw,
            owner_instance_key: pair_key(organisation_pair),
            producer_record_sha256,
            post_return: true,
            candidate_universe_complete: true,
            rejection_predicates_complete: true,
            stable_identity_complete: true,
            list_state_complete: true,
        },
        candidates,
        accepted_order,
        slots,
        list_state,
    };
    state.observations.push_back(observation);
    if state.observations.len() > MAX_PENDING {
        state.observations.pop_front();
    }
    Ok(())
}

fn fixed_dispositions(base: usize, organisation: usize) -> Result<[FixedDisposition; 8], String> {
    let global_26b = read_u8(base + FIXED_BONUS_GLOBAL_RVA)
        .ok_or("MIO fixed bonus global 0x26b unreadable")? as i64;
    let flag_268 = if read_usize(base + FIXED_FLAG_268_RVA)
        .ok_or("MIO fixed bonus flag 0x268 unreadable")?
        != 0
    {
        1
    } else {
        0
    };
    let flag_269 =
        if read_u32(base + FIXED_FLAG_269_RVA).ok_or("MIO fixed bonus flag 0x269 unreadable")? != 0
        {
            1
        } else {
            0
        };
    let flag_26a = if read_usize(base + FIXED_FLAG_26A_RVA)
        .ok_or("MIO fixed bonus flag 0x26a unreadable")?
        != 0
    {
        1
    } else {
        0
    };
    let value_26d = aggregate_modifier(
        base,
        organisation,
        read_i64(organisation + 0xC8).ok_or("MIO fixed base 0x26d unreadable")?,
        0x26D,
    )?;
    let value_26e = aggregate_modifier(base, organisation, 100_000, 0x26E)?;
    let value_26f = aggregate_modifier(base, organisation, 100_000, 0x26F)?;
    Ok([
        FixedDisposition {
            semantic_key: FIXED_KEYS[0],
            included: global_26b != 0,
            deciding_value: global_26b,
        },
        FixedDisposition {
            semantic_key: FIXED_KEYS[1],
            included: true,
            deciding_value: 1,
        },
        FixedDisposition {
            semantic_key: FIXED_KEYS[2],
            included: flag_268 != 0,
            deciding_value: flag_268,
        },
        FixedDisposition {
            semantic_key: FIXED_KEYS[3],
            included: flag_269 != 0,
            deciding_value: flag_269,
        },
        FixedDisposition {
            semantic_key: FIXED_KEYS[4],
            included: flag_26a != 0,
            deciding_value: flag_26a,
        },
        FixedDisposition {
            semantic_key: FIXED_KEYS[5],
            included: value_26d != 100_000,
            deciding_value: value_26d,
        },
        FixedDisposition {
            semantic_key: FIXED_KEYS[6],
            included: value_26e != 100_000,
            deciding_value: value_26e,
        },
        FixedDisposition {
            semantic_key: FIXED_KEYS[7],
            included: value_26f != 100_000,
            deciding_value: value_26f,
        },
    ])
}

fn aggregate_modifier(
    base: usize,
    organisation: usize,
    baseline: i64,
    modifier_id: i32,
) -> Result<i64, String> {
    let callback: AggregateModifierFn = unsafe { mem::transmute(base + AGGREGATE_MODIFIER_RVA) };
    let mut output = 0i64;
    let output_pointer: *mut i64 = &mut output;
    let returned =
        unsafe { callback(organisation, output_pointer, baseline, modifier_id, 0, 0, 0) };
    if returned != output_pointer {
        return Err(format!(
            "MIO aggregate modifier 0x{modifier_id:X} returned a foreign output"
        ));
    }
    Ok(output)
}

fn fixed_rejection_reason(semantic_key: u32) -> &'static str {
    match semantic_key {
        0x26B => "mio_fixed_26b_global_disabled",
        0x268 => "mio_fixed_268_feature_disabled",
        0x269 => "mio_fixed_269_feature_disabled",
        0x26A => "mio_fixed_26a_feature_disabled",
        0x26D => "mio_fixed_26d_default_value",
        0x26E => "mio_fixed_26e_default_value",
        0x26F => "mio_fixed_26f_default_value",
        _ => "mio_fixed_unavailable",
    }
}

fn atom_key(
    lane: ControllerSourceLane,
    semantic_key: u32,
    producer_rva: u32,
    subject: (u32, u32),
) -> ControllerCandidateKey {
    ControllerCandidateKey {
        source_lane: lane,
        source_instance_key: semantic_key as u64,
        identity: PortableItemIdentity::ControllerAtom {
            producer_rva,
            semantic_key,
            subject_type: subject.0,
            subject_id: subject.1,
        },
    }
}

fn candidate(
    key: ControllerCandidateKey,
    identity_resolver: PortableObjectResolverRecord,
    identity_record_sha256: &str,
    source_ordinal: u32,
    rejection: Option<RejectionEvidence>,
) -> ControllerCandidate {
    ControllerCandidate {
        key,
        identity_resolver,
        identity_record_sha256: identity_record_sha256.into(),
        source_ordinal,
        rejection,
    }
}

fn pointer_vector(vector: usize, label: &str) -> Result<Vec<usize>, String> {
    let count = bounded_count(
        read_i32(vector + VECTOR_COUNT_OFFSET)
            .ok_or_else(|| format!("{label} count unreadable"))?,
        label,
    )?;
    let base = read_usize(vector).ok_or_else(|| format!("{label} pointer unreadable"))?;
    if count != 0 && base == 0 {
        return Err(format!("{label} pointer is null"));
    }
    let mut output = Vec::with_capacity(count);
    for ordinal in 0..count {
        let address = checked_stride(base, ordinal, mem::size_of::<usize>(), label)?;
        let item =
            read_usize(address).ok_or_else(|| format!("{label} item {ordinal} unreadable"))?;
        if item == 0 {
            return Err(format!("{label} item {ordinal} is null"));
        }
        output.push(item);
    }
    Ok(output)
}

fn gate_row_template(base: usize, row: usize, template: RuntimeRowTemplate) -> Result<(), String> {
    let (primary, gui) = match template {
        RuntimeRowTemplate::MioDetailModifierItem => {
            (MODIFIER_ROW_PRIMARY_VTABLE_RVA, MODIFIER_ROW_GUI_VTABLE_RVA)
        }
        RuntimeRowTemplate::MioDetailBonusItem => {
            (BONUS_ROW_PRIMARY_VTABLE_RVA, BONUS_ROW_GUI_VTABLE_RVA)
        }
        RuntimeRowTemplate::MioEquipmentTypeWindow => (
            EQUIPMENT_ROW_PRIMARY_VTABLE_RVA,
            EQUIPMENT_ROW_GUI_VTABLE_RVA,
        ),
        RuntimeRowTemplate::MioDetailSeparator => (
            SEPARATOR_ROW_PRIMARY_VTABLE_RVA,
            SEPARATOR_ROW_GUI_VTABLE_RVA,
        ),
        _ => return Err("MIO bonus row has a foreign runtime template".into()),
    };
    gate_vtable(row, base + primary, "MIO bonus row primary")?;
    gate_vtable(row + ROW_GUI_OFFSET, base + gui, "MIO bonus row GUI")
}

fn capture_list_state(base: usize, list: usize) -> Result<ResolvedListState, String> {
    let interface = list + 8;
    let position = invoke_geometry(interface, 0xA0, "MIO bonus list position")?;
    let viewport = invoke_geometry(interface, 0x30, "MIO bonus list viewport")?;
    let content = invoke_geometry(interface, 0xE0, "MIO bonus list content")?;
    require_positive_extent(viewport, "MIO bonus viewport")?;
    if content[0] < 0 || content[1] < 0 {
        return Err("MIO bonus content extent is negative".into());
    }
    let scrollbar =
        read_usize(list + LIST_SCROLLBAR_OFFSET).ok_or("MIO bonus scrollbar pointer unreadable")?;
    gate_vtable(
        scrollbar,
        base + EXTENDED_SCROLLBAR_PRIMARY_VTABLE_RVA,
        "MIO bonus scrollbar",
    )?;
    gate_vtable(
        scrollbar + 0x80,
        base + EXTENDED_SCROLLBAR_PLUS_80_VTABLE_RVA,
        "MIO bonus scrollbar +0x80",
    )?;
    gate_vtable(
        scrollbar + 0xB0,
        base + EXTENDED_SCROLLBAR_PLUS_B0_VTABLE_RVA,
        "MIO bonus scrollbar +0xB0",
    )?;
    let state = ResolvedListState {
        clip_rect: RectI32 {
            x: position[0],
            y: position[1],
            width: viewport[0],
            height: viewport[1],
        },
        content_extent_pixels: (content[0], content[1]),
        viewport_extent_pixels: (viewport[0], viewport[1]),
        scroll_min_fixed: read_i64(scrollbar + SCROLL_MIN_OFFSET)
            .ok_or("MIO bonus scroll minimum unreadable")?,
        scroll_max_fixed: read_i64(scrollbar + SCROLL_MAX_OFFSET)
            .ok_or("MIO bonus scroll maximum unreadable")?,
        scroll_current_fixed: read_i64(scrollbar + SCROLL_CURRENT_OFFSET)
            .ok_or("MIO bonus scroll current unreadable")?,
        scroll_target_fixed: read_i64(scrollbar + SCROLL_TARGET_OFFSET)
            .ok_or("MIO bonus scroll target unreadable")?,
    };
    validate_scroll(&state)?;
    Ok(state)
}

fn resolve_identity(base: usize, pair: (u32, u32)) -> Result<PortableObjectResolverRecord, String> {
    let resolved = resolve_pair_pointer(base, pair)?;
    let resolved_pair = read_valid_pair(
        resolved + OBJECT_SELF_REFERENCE_OFFSET,
        "MIO bonus resolved organisation self-reference",
    )?;
    if resolved_pair != pair {
        return Err("MIO bonus pair resolver round trip differs".into());
    }
    Ok(PortableObjectResolverRecord {
        source: ResolverInputSource::MioBonusOrganisationSubject,
        resolver_rva: PAIR_TO_OBJECT_RESOLVER_RVA,
        lookup_rva: REFERENCE_TABLE_LOOKUP_RVA,
        resolver_job_id: PAIR_TO_OBJECT_RESOLVER_JOB_ID.into(),
        resolver_manifest_sha256: PAIR_TO_OBJECT_RESOLVER_MANIFEST_SHA256.into(),
        raw_object_type: pair.0,
        raw_stable_id: pair.1,
        resolved_object_type: resolved_pair.0,
        resolved_stable_id: resolved_pair.1,
        resolved_non_null: true,
        round_trip_self_reference_matches: true,
    })
}

fn resolve_pair_pointer(base: usize, pair: (u32, u32)) -> Result<usize, String> {
    require_valid_pair(pair, "MIO bonus resolver input")?;
    let raw = [pair.0, pair.1];
    let resolver: PairResolverFn =
        unsafe { mem::transmute(base + PAIR_TO_OBJECT_RESOLVER_RVA as usize) };
    let resolved = unsafe { resolver(raw.as_ptr()) };
    if resolved == 0 {
        return Err(format!("MIO bonus resolver returned null for {pair:?}"));
    }
    Ok(resolved)
}

fn rejection_evidence(
    lane: ControllerSourceLane,
    source_instance_key: u64,
    semantic_key: u32,
    reason: &str,
    predicate_rva: u32,
    deciding_value: i64,
) -> RejectionEvidence {
    let mut canonical = Vec::with_capacity(128);
    canonical.extend_from_slice(b"hoi4-a28-mio-bonus-rejection-v1\0");
    canonical.extend_from_slice(&(lane as u32).to_le_bytes());
    canonical.extend_from_slice(&source_instance_key.to_le_bytes());
    canonical.extend_from_slice(&semantic_key.to_le_bytes());
    canonical.extend_from_slice(&predicate_rva.to_le_bytes());
    canonical.extend_from_slice(&deciding_value.to_le_bytes());
    canonical.extend_from_slice(reason.as_bytes());
    canonical.push(0);
    RejectionEvidence {
        reason_code: reason.into(),
        predicate_rva,
        predicate_record_sha256: format!("{:x}", Sha256::digest(canonical)),
    }
}

fn producer_digest(
    organisation_pair: (u32, u32),
    candidates: &[ControllerCandidate],
    accepted: &[ControllerCandidateKey],
    slots: &[ControllerSlot],
    list: &ResolvedListState,
) -> String {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"hoi4-a28-mio-bonus-materialization-v1\0");
    canonical.extend_from_slice(MIO_BONUS_AGGREGATE_JOB_ID.as_bytes());
    canonical.push(0);
    canonical.extend_from_slice(MIO_BONUS_AGGREGATE_MANIFEST_SHA256.as_bytes());
    canonical.push(0);
    canonical.extend_from_slice(MIO_BONUS_AGGREGATE_DECOMPILE_SHA256.as_bytes());
    canonical.push(0);
    canonical.extend_from_slice(&organisation_pair.0.to_le_bytes());
    canonical.extend_from_slice(&organisation_pair.1.to_le_bytes());
    canonical.extend_from_slice(&(candidates.len() as u64).to_le_bytes());
    for candidate in candidates {
        canonical.extend_from_slice(&candidate.source_ordinal.to_le_bytes());
        canonical.extend_from_slice(&(candidate.key.source_lane as u32).to_le_bytes());
        canonical.extend_from_slice(&candidate.key.source_instance_key.to_le_bytes());
        if let PortableItemIdentity::ControllerAtom {
            producer_rva,
            semantic_key,
            ..
        } = candidate.key.identity
        {
            canonical.extend_from_slice(&producer_rva.to_le_bytes());
            canonical.extend_from_slice(&semantic_key.to_le_bytes());
        }
        canonical.extend_from_slice(candidate.identity_record_sha256.as_bytes());
        canonical.push(0);
        if let Some(rejection) = &candidate.rejection {
            canonical.extend_from_slice(&rejection.predicate_rva.to_le_bytes());
            canonical.extend_from_slice(rejection.predicate_record_sha256.as_bytes());
        }
        canonical.push(0);
    }
    canonical.extend_from_slice(&(accepted.len() as u64).to_le_bytes());
    for key in accepted {
        canonical.extend_from_slice(&(key.source_lane as u32).to_le_bytes());
        canonical.extend_from_slice(&key.source_instance_key.to_le_bytes());
    }
    for slot in slots {
        canonical.extend_from_slice(&slot.stable_slot_id.to_le_bytes());
        canonical.extend_from_slice(&(slot.template as u32).to_le_bytes());
        for value in [slot.rect.x, slot.rect.y, slot.rect.width, slot.rect.height] {
            canonical.extend_from_slice(&value.to_le_bytes());
        }
    }
    for value in [
        list.clip_rect.x,
        list.clip_rect.y,
        list.clip_rect.width,
        list.clip_rect.height,
        list.content_extent_pixels.0,
        list.content_extent_pixels.1,
        list.viewport_extent_pixels.0,
        list.viewport_extent_pixels.1,
    ] {
        canonical.extend_from_slice(&value.to_le_bytes());
    }
    for value in [
        list.scroll_min_fixed,
        list.scroll_max_fixed,
        list.scroll_current_fixed,
        list.scroll_target_fixed,
    ] {
        canonical.extend_from_slice(&value.to_le_bytes());
    }
    format!("{:x}", Sha256::digest(canonical))
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

fn diagnostics() -> Result<MioBonusHookDiagnostics, String> {
    let state = state_lock()?;
    Ok(MioBonusHookDiagnostics {
        installed: INSTALLED.load(Ordering::Acquire),
        callbacks_open: CALLBACKS_OPEN.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT.load(Ordering::Acquire),
        controller_calls: state.controller_calls,
        transition_sequence: state.transition_sequence,
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
        .map_err(|_| "A28 MIO bonus hook state mutex poisoned".to_string())
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
            .ok_or_else(|| format!("{label} slot overflow"))?,
    )
    .ok_or_else(|| format!("{label} function unreadable"))?;
    if function == 0 {
        return Err(format!("{label} function is null"));
    }
    let callback: GeometryFn = unsafe { mem::transmute(function) };
    let mut output = [0i32; 2];
    unsafe {
        callback(object, output.as_mut_ptr());
    }
    Ok(output)
}

fn gate_vtable(object: usize, expected: usize, label: &str) -> Result<(), String> {
    let actual = read_usize(object).ok_or_else(|| format!("{label} vtable unreadable"))?;
    if actual != expected {
        return Err(format!(
            "{label} vtable mismatch: expected 0x{expected:X}, observed 0x{actual:X}"
        ));
    }
    Ok(())
}

fn validate_scroll(state: &ResolvedListState) -> Result<(), String> {
    if state.scroll_max_fixed < state.scroll_min_fixed
        || !(state.scroll_min_fixed..=state.scroll_max_fixed).contains(&state.scroll_current_fixed)
        || !(state.scroll_min_fixed..=state.scroll_max_fixed).contains(&state.scroll_target_fixed)
    {
        return Err("MIO bonus fixed scrollbar state is outside its range".into());
    }
    Ok(())
}

fn require_positive_extent(extent: [i32; 2], label: &str) -> Result<(), String> {
    if extent[0] <= 0 || extent[1] <= 0 {
        return Err(format!("{label} extent is not positive"));
    }
    Ok(())
}

fn bounded_count(value: i32, label: &str) -> Result<usize, String> {
    if value < 0 || value as usize > MAX_ROWS {
        return Err(format!("{label} count {value} exceeds bound"));
    }
    Ok(value as usize)
}

fn checked_stride(
    base: usize,
    ordinal: usize,
    stride: usize,
    label: &str,
) -> Result<usize, String> {
    base.checked_add(
        ordinal
            .checked_mul(stride)
            .ok_or_else(|| format!("{label} stride overflow"))?,
    )
    .ok_or_else(|| format!("{label} address overflow"))
}

fn require_valid_pair(pair: (u32, u32), label: &str) -> Result<(u32, u32), String> {
    if pair.0 == 0 || pair.1 == 0 {
        return Err(format!("{label} contains zero"));
    }
    Ok(pair)
}

fn pair_key(pair: (u32, u32)) -> u64 {
    (u64::from(pair.0) << 32) | u64::from(pair.1)
}

fn read_valid_pair(address: usize, label: &str) -> Result<(u32, u32), String> {
    require_valid_pair(
        read_pair(address).ok_or_else(|| format!("{label} unreadable"))?,
        label,
    )
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual =
        rpm(address, expected.len()).ok_or_else(|| format!("{label} sentinel unreadable"))?;
    if actual != expected {
        return Err(format!("{label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn verify_executable_sha256(module: *mut c_void) -> Result<(), String> {
    let mut path = vec![0u16; 32_768];
    let length = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) };
    if length == 0 || length as usize >= path.len() {
        return Err("A28 MIO bonus executable path is unavailable or truncated".into());
    }
    path.truncate(length as usize);
    let path = std::path::PathBuf::from(
        String::from_utf16(&path)
            .map_err(|_| "A28 MIO bonus executable path is not valid UTF-16".to_string())?,
    );
    let mut file = File::open(path)
        .map_err(|error| format!("A28 MIO bonus executable open failed: {error}"))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("A28 MIO bonus executable hash read failed: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    if format!("{:x}", hasher.finalize()) != A729_EXECUTABLE_SHA256 {
        return Err("A28 MIO bonus running executable SHA-256 differs from exact a729".into());
    }
    Ok(())
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err("A28 MIO bonus hook callbacks did not quiesce".into());
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

fn read_pair(address: usize) -> Option<(u32, u32)> {
    Some((read_u32(address)?, read_u32(address + 4)?))
}

fn read_u8(address: usize) -> Option<u8> {
    Some(*rpm(address, 1)?.first()?)
}

fn read_u32(address: usize) -> Option<u32> {
    Some(u32::from_le_bytes(rpm(address, 4)?.try_into().ok()?))
}

fn read_i32(address: usize) -> Option<i32> {
    Some(i32::from_le_bytes(rpm(address, 4)?.try_into().ok()?))
}

fn read_i64(address: usize) -> Option<i64> {
    Some(i64::from_le_bytes(rpm(address, 8)?.try_into().ok()?))
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
