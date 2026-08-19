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
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

pub const MIO_HISTORY_REFRESH_RVA: usize = 0x01F0_AC90;
pub const MIO_HISTORY_HOOK_JOB_ID: &str =
    "20260722T071816Z-a28-mio-history-and-bonus-final-exhausti-08b8a7f4";
pub const MIO_HISTORY_HOOK_MANIFEST_SHA256: &str =
    "a182725f84e611185fb3c4284130d1db27bae22763dd4b9f11470df8c6ef76ab";

const CONTROLLER_ORGANISATION_OFFSET: usize = 0x30;
const CONTROLLER_TAB_OFFSET: usize = 0x40;
const CONTROLLER_VARIANT_POOL_OFFSET: usize = 0x20C8;
const CONTROLLER_PRODUCTION_POOL_OFFSET: usize = 0x20E0;
const CONTROLLER_VARIANT_LIST_OFFSET: usize = 0x3850;
const CONTROLLER_PRODUCTION_LIST_OFFSET: usize = 0x3858;

const OBJECT_SELF_REFERENCE_OFFSET: usize = 0x08;
const ORGANISATION_LINES_OFFSET: usize = 0xD8;
const ORGANISATION_LINE_COUNT_OFFSET: usize = 0xE4;
const ORGANISATION_TABLE_OFFSET: usize = 0xF8;
const ORGANISATION_TABLE_LAST_OFFSET: usize = 0x104;
const ORGANISATION_TABLE_OVERFLOW_OFFSET: usize = 0x108;
const PRODUCTION_LINE_STRIDE: usize = 0x18;
const PRODUCTION_LINE_ITEM_OFFSET: usize = 0x10;
const PRODUCTION_LINE_ITEM_PAIR_OFFSET: usize = 0x88;
const TABLE_ENTRY_STRIDE: usize = 0x40;
const TABLE_ENTRY_DISTANCE_OFFSET: usize = 0x04;
const TABLE_ENTRY_PAIR_OFFSET: usize = 0x08;
const TABLE_ENTRY_ACTIVE_OFFSET: usize = 0x30;
const TABLE_ENTRY_PRODUCTION_COUNT_OFFSET: usize = 0x38;

const POOL_COUNT_OFFSET: usize = 0x0C;
const LIST_HEAD_OFFSET: usize = 0x90;
const LIST_COUNT_OFFSET: usize = 0xA0;
const LIST_SCROLLBAR_OFFSET: usize = 0x190;
const NODE_ITEM_OFFSET: usize = 0;
const NODE_NEXT_OFFSET: usize = 0x10;
const ROW_GUI_OFFSET: usize = 0x38;
const ROW_ORGANISATION_PAIR_OFFSET: usize = 0x70;
const ROW_HISTORY_TYPE_OFFSET: usize = 0x78;
const ROW_ITEM_PAIR_OFFSET: usize = 0x7C;
const SCROLL_MIN_OFFSET: usize = 0x1558;
const SCROLL_MAX_OFFSET: usize = 0x1560;
const SCROLL_TARGET_OFFSET: usize = 0x15F8;
const SCROLL_CURRENT_OFFSET: usize = 0x1600;

const STANDARD_LIST_PRIMARY_VTABLE_RVA: usize = 0x02B2_CB48;
const STANDARD_LIST_SECONDARY_VTABLE_RVA: usize = 0x02B2_CC78;
const HISTORY_ROW_PRIMARY_VTABLE_RVA: usize = 0x02A8_E230;
const HISTORY_ROW_GUI_VTABLE_RVA: usize = 0x02A8_E2A0;
const HISTORY_ROW_THIRD_VTABLE_RVA: usize = 0x02A8_E368;
const EXTENDED_SCROLLBAR_PRIMARY_VTABLE_RVA: usize = 0x02B3_2FF0;
const EXTENDED_SCROLLBAR_PLUS_80_VTABLE_RVA: usize = 0x02B3_3270;
const EXTENDED_SCROLLBAR_PLUS_B0_VTABLE_RVA: usize = 0x02B3_3300;

const VARIANT_PREDICATE_RVA: u32 = 0x00DA_1420;
const PRODUCTION_PREDICATE_RVA: u32 = 0x00DA_1680;
const MAX_PENDING: usize = 4_096;
const MAX_ROWS: usize = crate::MAX_ITEMS_PER_COLLECTION;
const MIO_HISTORY_REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];

type MioHistoryRefreshFn = unsafe extern "system" fn(usize);
type PairResolverFn = unsafe extern "system" fn(*const u32) -> usize;
type GeometryFn = unsafe extern "system" fn(usize, *mut i32) -> *mut i32;

static ORIGINAL_REFRESH: AtomicUsize = AtomicUsize::new(0);
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
    observation_pairs: VecDeque<(AirMioControllerObservation, AirMioControllerObservation)>,
    poison: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MioHistoryHookDiagnostics {
    pub installed: bool,
    pub callbacks_open: bool,
    pub in_flight_callbacks: usize,
    pub controller_calls: u64,
    pub transition_sequence: u64,
    pub pending_observation_pairs: usize,
    pub poisoned: bool,
}

pub struct A729MioHistoryHookOwner {
    target: usize,
    epoch: usize,
    active: bool,
}

impl A729MioHistoryHookOwner {
    pub fn install(game_clock_address: usize) -> Result<Self, String> {
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 MIO history install mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Err("A28 MIO history hook already has a live owner".into());
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
            "A28 MIO daily-update anchor",
        )?;
        verify_sentinel(
            base + compact_contract::build_profile::GET_LONG_STRING_OFFSET,
            &compact_contract::build_profile::GET_LONG_STRING_SENTINEL,
            "A28 MIO long-string anchor",
        )?;
        let exact_clock = read_usize(base + compact_contract::build_profile::CG_STATE_DATE_OFFSET)
            .ok_or("A28 MIO exact game-clock root is unreadable")?;
        if game_clock_address != exact_clock {
            return Err("A28 MIO game-clock address differs from the exact a729 root".into());
        }
        if game_clock_address == 0 || read_i32(game_clock_address).is_none() {
            return Err("A28 MIO history game-clock address is unreadable".into());
        }
        let target = base + MIO_HISTORY_REFRESH_RVA;
        verify_sentinel(target, &MIO_HISTORY_REFRESH_SENTINEL, "MIO history refresh")?;
        reset_state()?;
        let epoch = INSTALL_EPOCH
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "A28 MIO history install epoch exhausted".to_string())?
            + 1;
        let original = unsafe {
            MinHook::create_hook(
                target as *mut c_void,
                hook_mio_history_refresh as *mut c_void,
            )
        }
        .map_err(|status| format!("A28 MIO history hook creation failed: {status}"))?;
        ORIGINAL_REFRESH.store(original as usize, Ordering::Release);
        MODULE_BASE.store(base, Ordering::Release);
        GAME_CLOCK_ADDRESS.store(game_clock_address, Ordering::Release);
        CALLBACKS_OPEN.store(true, Ordering::Release);
        if let Err(status) = unsafe { inline_detour::enable(target as *mut c_void) } {
            CALLBACKS_OPEN.store(false, Ordering::Release);
            let _ = unsafe { inline_detour::disable(target as *mut c_void) };
            if let Err(quiescence) = wait_for_quiescence(Duration::from_secs(30)) {
                return Err(format!(
                    "A28 MIO history hook enable failed: {status}; detour retained: {quiescence}"
                ));
            }
            if let Err(removal) = unsafe { inline_detour::remove(target as *mut c_void) } {
                return Err(format!(
                    "A28 MIO history hook enable failed: {status}; detour retained, removal failed: {removal}"
                ));
            }
            ORIGINAL_REFRESH.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            GAME_CLOCK_ADDRESS.store(0, Ordering::Release);
            return Err(format!("A28 MIO history hook enable failed: {status}"));
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(Self {
            target,
            epoch,
            active: true,
        })
    }

    pub fn diagnostics(&self) -> Result<MioHistoryHookDiagnostics, String> {
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 MIO history hook owner epoch is stale".into());
        }
        diagnostics()
    }

    pub fn drain_observation_pairs(
        &self,
    ) -> Result<Vec<(AirMioControllerObservation, AirMioControllerObservation)>, String> {
        self.require_active()?;
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 MIO history hook poisoned: {error}"));
        }
        Ok(state.observation_pairs.drain(..).collect())
    }

    pub fn drain_latest_source(
        &self,
        game_hour_raw: i32,
    ) -> Result<A729AirMioControllerSource, String> {
        self.require_active()?;
        let mut state = state_lock()?;
        if let Some(error) = &state.poison {
            return Err(format!("A28 MIO history hook poisoned: {error}"));
        }
        let (variant, production) = state
            .observation_pairs
            .pop_back()
            .ok_or("A28 MIO history hook has no complete post-return pair")?;
        state.observation_pairs.clear();
        drop(state);
        if variant.authority.captured_game_hour_raw != game_hour_raw
            || production.authority.captured_game_hour_raw != game_hour_raw
        {
            return Err(format!(
                "A28 MIO observation raw hour differs from drain hour {game_hour_raw}"
            ));
        }
        let frame_hour = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
            .and_then(|hour| u64::try_from(hour).ok())
            .ok_or("A28 MIO captured game hour is not a compact calendar index")?;
        A729AirMioControllerSource::admit_hook_owned_mio_history(
            frame_hour,
            vec![variant, production],
        )
    }

    fn require_active(&self) -> Result<(), String> {
        if !self.active || !INSTALLED.load(Ordering::Acquire) {
            return Err("A28 MIO history hook owner is not active".into());
        }
        if self.epoch != INSTALL_EPOCH.load(Ordering::Acquire) {
            return Err("A28 MIO history hook owner epoch is stale".into());
        }
        Ok(())
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let _guard = INSTALL_LOCK
            .lock()
            .map_err(|_| "A28 MIO history install mutex poisoned".to_string())?;
        CALLBACKS_OPEN.store(false, Ordering::Release);
        match unsafe { inline_detour::disable(self.target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!("A28 MIO history hook disable failed: {status}"));
            }
        }
        wait_for_quiescence(Duration::from_secs(30))?;
        match unsafe { inline_detour::remove(self.target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!("A28 MIO history hook removal failed: {status}"));
            }
        }
        ORIGINAL_REFRESH.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        GAME_CLOCK_ADDRESS.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        self.active = false;
        Ok(())
    }
}

impl Drop for A729MioHistoryHookOwner {
    fn drop(&mut self) {
        if self.active && self.uninstall().is_err() {
            INSTALLED.store(true, Ordering::Release);
        }
    }
}

unsafe extern "system" fn hook_mio_history_refresh(controller: usize) {
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
    let address = ORIGINAL_REFRESH.load(Ordering::Acquire);
    if address != 0 {
        let original: MioHistoryRefreshFn = mem::transmute(address);
        original(controller);
    }
}

fn capture_post_return(controller: usize) -> Result<(), String> {
    if read_i32(controller + CONTROLLER_TAB_OFFSET).ok_or("MIO tab unreadable")? != 1 {
        return Ok(());
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("module base absent during A28 MIO history callback".into());
    }
    let captured_game_hour_raw = read_i32(GAME_CLOCK_ADDRESS.load(Ordering::Acquire))
        .ok_or("A28 MIO history callback game clock unreadable")?;
    let detail_window =
        read_usize(controller + 0x38).ok_or("MIO detail-window pointer unreadable")?;
    if detail_window == 0 {
        return Ok(());
    }
    let detail_flags =
        read_u8(detail_window + 0xA5).ok_or("MIO detail-window active flags unreadable")?;
    if detail_flags & 0x08 == 0 {
        return Ok(());
    }
    let organisation = read_usize(controller + CONTROLLER_ORGANISATION_OFFSET)
        .ok_or("MIO organisation pointer unreadable")?;
    if organisation == 0 {
        return Err("MIO history tab has a null organisation".into());
    }
    let organisation_pair = read_valid_pair(
        organisation + OBJECT_SELF_REFERENCE_OFFSET,
        "MIO organisation self-reference",
    )?;
    let resolved_organisation = resolve_pair_pointer(base, organisation_pair)?;
    if resolved_organisation != organisation {
        return Err("MIO organisation resolver round trip differs".into());
    }

    let variant_raw = variant_candidates(organisation)?;
    let production_raw = production_candidates(base, organisation)?;
    let variant = materialize_history(
        base,
        controller,
        organisation_pair,
        HistoryFamily::Variant,
        variant_raw,
    )?;
    let production = materialize_history(
        base,
        controller,
        organisation_pair,
        HistoryFamily::Production,
        production_raw,
    )?;

    let qpc_ticks = qpc()?;
    let mut state = state_lock()?;
    state.controller_calls = state.controller_calls.saturating_add(1);
    state.transition_sequence = state
        .transition_sequence
        .checked_add(1)
        .ok_or("A28 MIO history transition sequence exhausted")?;
    let sequence = state.transition_sequence;
    let owner_instance_key = pair_key(organisation_pair);
    let variant = into_observation(
        variant,
        sequence,
        qpc_ticks,
        captured_game_hour_raw,
        owner_instance_key,
    );
    let production = into_observation(
        production,
        sequence,
        qpc_ticks,
        captured_game_hour_raw,
        owner_instance_key,
    );
    state.observation_pairs.push_back((variant, production));
    if state.observation_pairs.len() > MAX_PENDING {
        state.observation_pairs.pop_front();
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryFamily {
    Variant,
    Production,
}

impl HistoryFamily {
    const fn kind(self) -> AirMioControllerKind {
        match self {
            Self::Variant => AirMioControllerKind::MioVariantHistory,
            Self::Production => AirMioControllerKind::MioProductionHistory,
        }
    }

    const fn history_type(self) -> i32 {
        match self {
            Self::Variant => 1,
            Self::Production => 2,
        }
    }

    const fn pool_offset(self) -> usize {
        match self {
            Self::Variant => CONTROLLER_VARIANT_POOL_OFFSET,
            Self::Production => CONTROLLER_PRODUCTION_POOL_OFFSET,
        }
    }

    const fn list_offset(self) -> usize {
        match self {
            Self::Variant => CONTROLLER_VARIANT_LIST_OFFSET,
            Self::Production => CONTROLLER_PRODUCTION_LIST_OFFSET,
        }
    }

    const fn resolver_source(self) -> ResolverInputSource {
        match self {
            Self::Variant => ResolverInputSource::MioVariantHistoryEntryReference,
            Self::Production => ResolverInputSource::MioProductionHistoryEntryReference,
        }
    }

    fn producer_rvas(self) -> Vec<u32> {
        match self {
            Self::Variant => vec![0x00DA_1420, 0x01F0_6360, 0x01FA_5390, 0x01FA_6630],
            Self::Production => {
                vec![0x00DA_1680, 0x01F0_6360, 0x01FA_5390, 0x01FA_6630]
            }
        }
    }
}

#[derive(Clone, Debug)]
struct RawCandidate {
    lane: ControllerSourceLane,
    source_instance_key: u64,
    pair: (u32, u32),
    rejection: Option<RejectionEvidence>,
}

#[derive(Clone, Debug)]
struct CapturedFamily {
    family: HistoryFamily,
    candidates: Vec<ControllerCandidate>,
    accepted_order: Vec<ControllerCandidateKey>,
    slots: Vec<ControllerSlot>,
    list_state: ResolvedListState,
    producer_record_sha256: String,
}

fn variant_candidates(organisation: usize) -> Result<Vec<RawCandidate>, String> {
    let entries = occupied_table_entries(organisation)?;
    let mut candidates = Vec::with_capacity(entries.len());
    let mut pairs = BTreeSet::new();
    for (_slot, entry) in entries {
        let pair = read_valid_pair(entry + TABLE_ENTRY_PAIR_OFFSET, "MIO variant table pair")?;
        if !pairs.insert(pair) {
            return Err(format!("MIO variant table repeats identity {pair:?}"));
        }
        let active = read_u8(entry + TABLE_ENTRY_ACTIVE_OFFSET)
            .ok_or("MIO variant active flag unreadable")?;
        let rejection = (active == 0).then(|| {
            rejection_evidence(
                HistoryFamily::Variant,
                ControllerSourceLane::MioVariantTable,
                pair_key(pair),
                pair,
                "mio_variant_entry_inactive",
                VARIANT_PREDICATE_RVA,
                active as i64,
            )
        });
        candidates.push(RawCandidate {
            lane: ControllerSourceLane::MioVariantTable,
            source_instance_key: pair_key(pair),
            pair,
            rejection,
        });
    }
    Ok(candidates)
}

fn production_candidates(base: usize, organisation: usize) -> Result<Vec<RawCandidate>, String> {
    let line_vector = read_usize(organisation + ORGANISATION_LINES_OFFSET)
        .ok_or("MIO production-line vector unreadable")?;
    let line_count = bounded_count(
        read_i32(organisation + ORGANISATION_LINE_COUNT_OFFSET)
            .ok_or("MIO production-line count unreadable")?,
        "MIO production-line",
    )?;
    let mut candidates = Vec::new();
    let mut accepted_pairs = BTreeSet::new();
    let mut line_keys = BTreeSet::new();
    for ordinal in 0..line_count {
        let line = checked_stride(line_vector, ordinal, PRODUCTION_LINE_STRIDE, "MIO line")?;
        let item = read_usize(line + PRODUCTION_LINE_ITEM_OFFSET)
            .ok_or_else(|| format!("MIO production-line {ordinal} item unreadable"))?;
        if item == 0 {
            return Err(format!(
                "MIO production-line {ordinal} has a null identity-bearing item"
            ));
        }
        let pair = read_valid_pair(
            item + PRODUCTION_LINE_ITEM_PAIR_OFFSET,
            "MIO production-line item pair",
        )?;
        let line_pair = read_valid_pair(
            item + OBJECT_SELF_REFERENCE_OFFSET,
            "MIO production-line object self-reference",
        )?;
        if resolve_pair_pointer(base, line_pair)? != item {
            return Err(format!(
                "MIO production-line {ordinal} object resolver round trip differs"
            ));
        }
        let line_key = pair_key(line_pair);
        if !line_keys.insert(line_key) {
            return Err(format!(
                "MIO production-line source repeats stable object {line_pair:?}"
            ));
        }
        accepted_pairs.insert(pair);
        candidates.push(RawCandidate {
            lane: ControllerSourceLane::MioProductionLine,
            source_instance_key: line_key,
            pair,
            rejection: None,
        });
    }

    for (_slot, entry) in occupied_table_entries(organisation)? {
        let pair = read_valid_pair(entry + TABLE_ENTRY_PAIR_OFFSET, "MIO production table pair")?;
        let source_instance_key = pair_key(pair);
        let production_count = read_i32(entry + TABLE_ENTRY_PRODUCTION_COUNT_OFFSET)
            .ok_or("MIO production table count unreadable")?;
        let (rejection, accepted) = if production_count <= 0 {
            (
                Some(rejection_evidence(
                    HistoryFamily::Production,
                    ControllerSourceLane::MioProductionTable,
                    source_instance_key,
                    pair,
                    "mio_production_table_count_nonpositive",
                    PRODUCTION_PREDICATE_RVA,
                    production_count as i64,
                )),
                false,
            )
        } else if accepted_pairs.contains(&pair) {
            (
                Some(rejection_evidence(
                    HistoryFamily::Production,
                    ControllerSourceLane::MioProductionTable,
                    source_instance_key,
                    pair,
                    "mio_production_table_duplicate",
                    PRODUCTION_PREDICATE_RVA,
                    production_count as i64,
                )),
                false,
            )
        } else {
            (None, true)
        };
        if accepted {
            accepted_pairs.insert(pair);
        }
        candidates.push(RawCandidate {
            lane: ControllerSourceLane::MioProductionTable,
            source_instance_key,
            pair,
            rejection,
        });
    }
    Ok(candidates)
}

fn occupied_table_entries(organisation: usize) -> Result<Vec<(u64, usize)>, String> {
    let base = read_usize(organisation + ORGANISATION_TABLE_OFFSET)
        .ok_or("MIO organisation table pointer unreadable")?;
    if base == 0 {
        return Err("MIO organisation table pointer is null".into());
    }
    let last = read_i32(organisation + ORGANISATION_TABLE_LAST_OFFSET)
        .ok_or("MIO organisation table last index unreadable")?;
    let overflow = read_u8(organisation + ORGANISATION_TABLE_OVERFLOW_OFFSET)
        .ok_or("MIO organisation table overflow index unreadable")? as i64;
    let slot_count = i64::from(last)
        .checked_add(overflow)
        .and_then(|value| value.checked_add(1))
        .ok_or("MIO organisation table bound overflow")?;
    if slot_count <= 0 || slot_count as usize > MAX_ROWS {
        return Err(format!(
            "MIO organisation table bound {slot_count} is invalid"
        ));
    }
    let end = base
        .checked_add(
            (slot_count as usize)
                .checked_mul(TABLE_ENTRY_STRIDE)
                .ok_or("MIO organisation table byte bound overflow")?,
        )
        .ok_or("MIO organisation table end overflow")?;
    let mut entry = base;
    let mut entries = Vec::new();
    while entry < end {
        let distance = read_i8(entry + TABLE_ENTRY_DISTANCE_OFFSET)
            .ok_or("MIO organisation table distance unreadable")?;
        if distance != 0 {
            let slot = (entry - base) / TABLE_ENTRY_STRIDE;
            entries.push((slot as u64 + 1, entry));
        }
        entry = entry
            .checked_add(TABLE_ENTRY_STRIDE)
            .ok_or("MIO organisation table walk overflow")?;
    }
    if entry != end {
        return Err("MIO organisation table stride does not meet bound".into());
    }
    Ok(entries)
}

fn materialize_history(
    base: usize,
    controller: usize,
    organisation_pair: (u32, u32),
    family: HistoryFamily,
    raw: Vec<RawCandidate>,
) -> Result<CapturedFamily, String> {
    let mut candidates = Vec::with_capacity(raw.len());
    let mut included_by_pair: BTreeMap<(u32, u32), VecDeque<ControllerCandidateKey>> =
        BTreeMap::new();
    let mut expected_production_order = Vec::new();
    for (ordinal, candidate) in raw.into_iter().enumerate() {
        let key = ControllerCandidateKey {
            source_lane: candidate.lane,
            source_instance_key: candidate.source_instance_key,
            identity: PortableItemIdentity::ReferencedObject {
                object_type: candidate.pair.0,
                stable_id: candidate.pair.1,
            },
        };
        let resolver = resolve_identity(base, candidate.pair, family.resolver_source())?;
        if candidate.rejection.is_none() {
            included_by_pair
                .entry(candidate.pair)
                .or_default()
                .push_back(key);
            if family == HistoryFamily::Production {
                expected_production_order.push((candidate.pair, key));
            }
        }
        let identity_record_sha256 = compute_identity_record_sha256(family.kind(), &resolver);
        candidates.push(ControllerCandidate {
            key,
            identity_resolver: resolver,
            identity_record_sha256,
            source_ordinal: ordinal as u32,
            rejection: candidate.rejection,
        });
    }

    let pool = controller + family.pool_offset();
    let pool_vector = read_usize(pool).ok_or("MIO history row-pool vector unreadable")?;
    let pool_count = bounded_count(
        read_i32(pool + POOL_COUNT_OFFSET).ok_or("MIO history row-pool count unreadable")?,
        "MIO history row-pool",
    )?;
    let list = read_usize(controller + family.list_offset())
        .ok_or("MIO history list pointer unreadable")?;
    gate_vtable(
        list,
        base + STANDARD_LIST_PRIMARY_VTABLE_RVA,
        "MIO history list primary",
    )?;
    gate_vtable(
        list + 8,
        base + STANDARD_LIST_SECONDARY_VTABLE_RVA,
        "MIO history list secondary",
    )?;
    let list_count = bounded_count(
        read_i32(list + LIST_COUNT_OFFSET).ok_or("MIO history list count unreadable")?,
        "MIO history list",
    )?;
    let included_count = candidates
        .iter()
        .filter(|candidate| candidate.rejection.is_none())
        .count();
    if list_count != included_count || pool_count < list_count {
        return Err(format!(
            "MIO {:?} included/list/pool counts differ: {included_count}/{list_count}/{pool_count}",
            family
        ));
    }

    let mut accepted_order = Vec::with_capacity(list_count);
    let mut slots = Vec::with_capacity(list_count);
    let mut node = read_usize(list + LIST_HEAD_OFFSET).ok_or("MIO history list head unreadable")?;
    for pool_ordinal in 0..pool_count {
        let row = read_usize(
            pool_vector
                .checked_add(
                    pool_ordinal
                        .checked_mul(mem::size_of::<usize>())
                        .ok_or("MIO history row-pool stride overflow")?,
                )
                .ok_or("MIO history row-pool address overflow")?,
        )
        .ok_or_else(|| format!("MIO history row {pool_ordinal} unreadable"))?;
        gate_vtable(
            row,
            base + HISTORY_ROW_PRIMARY_VTABLE_RVA,
            "MIO history row",
        )?;
        gate_vtable(
            row + ROW_GUI_OFFSET,
            base + HISTORY_ROW_GUI_VTABLE_RVA,
            "MIO history row GUI",
        )?;
        gate_vtable(
            row + 0x68,
            base + HISTORY_ROW_THIRD_VTABLE_RVA,
            "MIO history row third subobject",
        )?;
        let history_type =
            read_i32(row + ROW_HISTORY_TYPE_OFFSET).ok_or("MIO history row type unreadable")?;
        if history_type != family.history_type() {
            return Err(format!(
                "MIO {:?} row {pool_ordinal} has type {history_type}",
                family
            ));
        }
        let row_organisation = read_pair(row + ROW_ORGANISATION_PAIR_OFFSET)
            .ok_or("MIO history row organisation pair unreadable")?;
        if row_organisation != organisation_pair {
            return Err(format!(
                "MIO {:?} row {pool_ordinal} organisation differs",
                family
            ));
        }
        let pair =
            read_pair(row + ROW_ITEM_PAIR_OFFSET).ok_or("MIO history row item pair unreadable")?;
        if pool_ordinal >= list_count {
            if pair != (0, 0) {
                return Err(format!(
                    "MIO {:?} inactive reusable row {pool_ordinal} retains a binding",
                    family
                ));
            }
            continue;
        }
        let pair = require_valid_pair(pair, "MIO history materialized row pair")?;
        if family == HistoryFamily::Production
            && expected_production_order
                .get(pool_ordinal)
                .map(|entry| entry.0)
                != Some(pair)
        {
            return Err(format!(
                "MIO production row {pool_ordinal} differs from line/table producer order"
            ));
        }
        let key = included_by_pair
            .get_mut(&pair)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| {
                format!(
                    "MIO {:?} materialized pair {pair:?} is absent from included source",
                    family
                )
            })?;
        if node == 0 {
            return Err(format!("MIO {:?} list ended at row {pool_ordinal}", family));
        }
        let row_gui = row + ROW_GUI_OFFSET;
        let node_gui =
            read_usize(node + NODE_ITEM_OFFSET).ok_or("MIO history list-node item unreadable")?;
        if node_gui != row_gui {
            return Err(format!(
                "MIO {:?} pool/list traversal differs at row {pool_ordinal}",
                family
            ));
        }
        let position = invoke_geometry(row_gui, 0x78, "MIO history row resolved position")?;
        let extent = invoke_geometry(row_gui, 0x28, "MIO history row resolved extent")?;
        require_positive_extent(extent, "MIO history row")?;
        accepted_order.push(key);
        slots.push(ControllerSlot {
            stable_slot_id: u32::try_from(pool_ordinal)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or("MIO history stable slot overflow")?,
            bound_candidate: key,
            template: RuntimeRowTemplate::MioHistoryItem,
            rect: RectI32 {
                x: position[0],
                y: position[1],
                width: extent[0],
                height: extent[1],
            },
        });
        node =
            read_usize(node + NODE_NEXT_OFFSET).ok_or("MIO history next list link unreadable")?;
    }
    if node != 0 {
        return Err(format!(
            "MIO {:?} list contains more nodes than authenticated count",
            family
        ));
    }
    if included_by_pair.values().any(|keys| !keys.is_empty()) {
        return Err(format!(
            "MIO {:?} included source contains unmaterialized candidates",
            family
        ));
    }

    let list_state = capture_list_state(base, list, family)?;
    let producer_record_sha256 = producer_digest(
        family,
        organisation_pair,
        &candidates,
        &accepted_order,
        &slots,
        &list_state,
    );
    Ok(CapturedFamily {
        family,
        candidates,
        accepted_order,
        slots,
        list_state,
        producer_record_sha256,
    })
}

fn capture_list_state(
    base: usize,
    list: usize,
    family: HistoryFamily,
) -> Result<ResolvedListState, String> {
    let interface = list + 8;
    let position = invoke_geometry(interface, 0xA0, "MIO history list position")?;
    let viewport = invoke_geometry(interface, 0x30, "MIO history list viewport")?;
    let content = invoke_geometry(interface, 0xE0, "MIO history list content")?;
    require_positive_extent(viewport, "MIO history viewport")?;
    if content[0] < 0 || content[1] < 0 {
        return Err(format!("MIO {:?} content extent is negative", family));
    }
    let scrollbar = read_usize(list + LIST_SCROLLBAR_OFFSET)
        .ok_or("MIO history scrollbar pointer unreadable")?;
    gate_vtable(
        scrollbar,
        base + EXTENDED_SCROLLBAR_PRIMARY_VTABLE_RVA,
        "MIO history scrollbar",
    )?;
    gate_vtable(
        scrollbar + 0x80,
        base + EXTENDED_SCROLLBAR_PLUS_80_VTABLE_RVA,
        "MIO history scrollbar +0x80",
    )?;
    gate_vtable(
        scrollbar + 0xB0,
        base + EXTENDED_SCROLLBAR_PLUS_B0_VTABLE_RVA,
        "MIO history scrollbar +0xB0",
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
            .ok_or("MIO history scroll minimum unreadable")?,
        scroll_max_fixed: read_i64(scrollbar + SCROLL_MAX_OFFSET)
            .ok_or("MIO history scroll maximum unreadable")?,
        scroll_current_fixed: read_i64(scrollbar + SCROLL_CURRENT_OFFSET)
            .ok_or("MIO history scroll current unreadable")?,
        scroll_target_fixed: read_i64(scrollbar + SCROLL_TARGET_OFFSET)
            .ok_or("MIO history scroll target unreadable")?,
    };
    validate_scroll(&state)?;
    Ok(state)
}

fn into_observation(
    captured: CapturedFamily,
    transition_sequence: u64,
    observation_qpc_ticks: u64,
    captured_game_hour_raw: i32,
    owner_instance_key: u64,
) -> AirMioControllerObservation {
    AirMioControllerObservation {
        kind: captured.family.kind(),
        authority: AirMioAuthorityReceipt {
            binary_id: A729_BINARY_ID.into(),
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            function_job_id: FINAL_FAMILY_JOB_ID.into(),
            function_manifest_sha256: FINAL_FAMILY_MANIFEST_SHA256.into(),
            producer_rvas: captured.family.producer_rvas(),
            transition_sequence,
            observation_qpc_ticks,
            captured_game_hour_raw,
            owner_instance_key,
            producer_record_sha256: captured.producer_record_sha256,
            post_return: true,
            candidate_universe_complete: true,
            rejection_predicates_complete: true,
            stable_identity_complete: true,
            list_state_complete: true,
        },
        candidates: captured.candidates,
        accepted_order: captured.accepted_order,
        slots: captured.slots,
        list_state: captured.list_state,
    }
}

fn resolve_identity(
    base: usize,
    pair: (u32, u32),
    source: ResolverInputSource,
) -> Result<PortableObjectResolverRecord, String> {
    let resolved = resolve_pair_pointer(base, pair)?;
    let resolved_pair = read_valid_pair(
        resolved + OBJECT_SELF_REFERENCE_OFFSET,
        "MIO history resolved object self-reference",
    )?;
    if resolved_pair != pair {
        return Err("MIO history pair resolver round trip differs".into());
    }
    Ok(PortableObjectResolverRecord {
        source,
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
    require_valid_pair(pair, "MIO history resolver input")?;
    let raw = [pair.0, pair.1];
    let resolver: PairResolverFn =
        unsafe { mem::transmute(base + PAIR_TO_OBJECT_RESOLVER_RVA as usize) };
    let resolved = unsafe { resolver(raw.as_ptr()) };
    if resolved == 0 {
        return Err(format!("MIO history resolver returned null for {pair:?}"));
    }
    Ok(resolved)
}

fn rejection_evidence(
    family: HistoryFamily,
    lane: ControllerSourceLane,
    source_instance_key: u64,
    pair: (u32, u32),
    reason: &str,
    predicate_rva: u32,
    deciding_value: i64,
) -> RejectionEvidence {
    let mut canonical = Vec::with_capacity(128);
    canonical.extend_from_slice(b"hoi4-a28-mio-history-rejection-v1\0");
    canonical.extend_from_slice(&(family as u32).to_le_bytes());
    canonical.extend_from_slice(&(lane as u32).to_le_bytes());
    canonical.extend_from_slice(&source_instance_key.to_le_bytes());
    canonical.extend_from_slice(&pair.0.to_le_bytes());
    canonical.extend_from_slice(&pair.1.to_le_bytes());
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
    family: HistoryFamily,
    organisation_pair: (u32, u32),
    candidates: &[ControllerCandidate],
    accepted: &[ControllerCandidateKey],
    slots: &[ControllerSlot],
    list: &ResolvedListState,
) -> String {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"hoi4-a28-mio-history-materialization-v1\0");
    canonical.extend_from_slice(&(family as u32).to_le_bytes());
    canonical.extend_from_slice(&organisation_pair.0.to_le_bytes());
    canonical.extend_from_slice(&organisation_pair.1.to_le_bytes());
    canonical.extend_from_slice(&(candidates.len() as u64).to_le_bytes());
    for candidate in candidates {
        canonical.extend_from_slice(&candidate.source_ordinal.to_le_bytes());
        canonical.extend_from_slice(&(candidate.key.source_lane as u32).to_le_bytes());
        canonical.extend_from_slice(&candidate.key.source_instance_key.to_le_bytes());
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

fn diagnostics() -> Result<MioHistoryHookDiagnostics, String> {
    let state = state_lock()?;
    Ok(MioHistoryHookDiagnostics {
        installed: INSTALLED.load(Ordering::Acquire),
        callbacks_open: CALLBACKS_OPEN.load(Ordering::Acquire),
        in_flight_callbacks: IN_FLIGHT.load(Ordering::Acquire),
        controller_calls: state.controller_calls,
        transition_sequence: state.transition_sequence,
        pending_observation_pairs: state.observation_pairs.len(),
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
        .map_err(|_| "A28 MIO history hook state mutex poisoned".to_string())
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
        return Err("MIO history fixed scrollbar state is outside its range".into());
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
        return Err("A28 MIO executable path is unavailable or truncated".into());
    }
    path.truncate(length as usize);
    let path = std::path::PathBuf::from(
        String::from_utf16(&path)
            .map_err(|_| "A28 MIO executable path is not valid UTF-16".to_string())?,
    );
    let mut file =
        File::open(path).map_err(|error| format!("A28 MIO executable open failed: {error}"))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("A28 MIO executable hash read failed: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    if format!("{:x}", hasher.finalize()) != A729_EXECUTABLE_SHA256 {
        return Err("A28 MIO running executable SHA-256 differs from exact a729".into());
    }
    Ok(())
}

fn wait_for_quiescence(timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            return Err("A28 MIO history hook callbacks did not quiesce".into());
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

fn read_i8(address: usize) -> Option<i8> {
    Some(*rpm(address, 1)?.first()? as i8)
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
