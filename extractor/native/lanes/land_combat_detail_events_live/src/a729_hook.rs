#![cfg(windows)]

use crate::terminal_coordinator::{
    individual_terminal_api_blocked, release_admitted_callback, try_admit_callback,
    try_admit_individual_api,
};
use core::cell::Cell;
use core::ffi::c_void;
use core::mem;
use core::ptr;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released,
    observe_released_set, ExpectedRvaHook, HookTeardownAttestation, InstallAbortDiscard,
    InstalledRvaHookReceipt, ReleasedRvaHookReceipt, MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const NORMAL_WRITER_RVA: usize = 0x0CCC_7F0;
pub const OVERRUN_WRITER_RVA: usize = 0x0CCC_100;
pub const RECORD_ALLOCATOR_RVA: usize = 0x0CC9_F10;
pub const TERMINAL_RECORD_VTABLE_RVA: usize = 0x0294_6FB8;
pub const GAME_DATE_VTABLE_RVA: usize = 0x0270_2388;
pub const TERMINAL_RECORD_LEN: usize = 0x448;
pub(crate) const MINHOOK_RELAY_LEN: usize = 14;
pub const WRITER_KIND_OVERRUN: u32 = 1;
pub const WRITER_KIND_NORMAL: u32 = 2;

/// release receipts.
pub const TERMINAL_WRITER_ACTIVATION: bool = true;
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const EXACT10_POOL_STATUS_RECEIPT_SHA256: &str =
    "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395";
pub const TERMINAL_FUNCTION_MANIFEST_SHA256: &str =
    "3e5c698b7e151edc704b32f90f663b29de85e52d06f0563229885f8849637726";
pub const TERMINAL_SENTINEL_DUMP_SHA256: &str =
    "176a2731bdf88194b92328f417e51b37a33180e93ac339df4aa3236b6f78217c";
pub const TERMINAL_LAYOUT_MANIFEST_SHA256: &str =
    "6a25f5c1ac32637271d58bbb6de6dcf7bfbcf43896de605426a41a78153df2c3";
pub const TERMINAL_LOG_ENTRY_MANIFEST_SHA256: &str =
    "dd19cb97391dcaa77b8eb4c2f9980d4b86fa1e703342e5e9843b76995a9edb31";
pub const TERMINAL_WRITER_CALLER_MANIFEST_SHA256: &str =
    "727ad4ca363610b06d31afcc0a4082c359147a4de92423508eb09c49e85be3cd";
pub const TERMINAL_ALTERNATE_WRITER_MANIFEST_SHA256: &str =
    "492e712cd283a75b9b6357eb42f4bc98137d1ab138cfa857164f8d353f675c28";

const MAX_PENDING_RECORDS: usize = 8_192;
const LAND_COMBAT_TYPE: u32 = 62;
const NORMAL_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55,
];
const OVERRUN_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55,
];
const ALLOCATOR_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x57, 0x41, 0x55, 0x48, 0x83, 0xec, 0x40, 0x4c, 0x63, 0x05, 0xec, 0x9c, 0x65, 0x02,
];
type NormalWriterFn = unsafe extern "system" fn(usize, usize, usize, usize, usize);
type OverrunWriterFn = unsafe extern "system" fn(usize, usize, usize);
type RecordAllocatorFn = unsafe extern "system" fn(usize) -> u64;

static ORIGINAL_NORMAL: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_OVERRUN: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ALLOCATOR: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static DROPPED_LOCK_BUSY: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNBOUND: AtomicU64 = AtomicU64::new(0);
static DROPPED_UNREADABLE: AtomicU64 = AtomicU64::new(0);
static DROPPED_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static NEXT_RECORD_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_WRITER_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_ALLOCATOR_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static ADMITTED_WRITER_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static ADMITTED_ALLOCATOR_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static CREATED_HOOK_MASK: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static HOOK_RECEIPTS: OnceLock<Mutex<HookReceiptState>> = OnceLock::new();
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static WRITER_CONTEXT: Cell<Option<WriterContext>> = const { Cell::new(None) };
}

#[derive(Clone, Copy)]
struct WriterContext {
    combat_ptr: usize,
    writer_kind: u32,
}

struct ActiveWriterCallback;

impl ActiveWriterCallback {
    #[inline(never)]
    fn enter() -> Self {
        ACTIVE_WRITER_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ActiveWriterCallback {
    fn drop(&mut self) {
        ACTIVE_WRITER_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ActiveAllocatorCallback;

impl ActiveAllocatorCallback {
    #[inline(never)]
    fn enter() -> Self {
        ACTIVE_ALLOCATOR_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ActiveAllocatorCallback {
    fn drop(&mut self) {
        ACTIVE_ALLOCATOR_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct AdmittedWriterCallback;

impl AdmittedWriterCallback {
    fn try_enter() -> Option<Self> {
        try_admit_callback(&ADMITTED_WRITER_CALLBACKS).then_some(Self)
    }
}

impl Drop for AdmittedWriterCallback {
    fn drop(&mut self) {
        release_admitted_callback(&ADMITTED_WRITER_CALLBACKS);
    }
}

struct AdmittedAllocatorCallback;

impl AdmittedAllocatorCallback {
    fn enter_nested() -> Self {
        ADMITTED_ALLOCATOR_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for AdmittedAllocatorCallback {
    fn drop(&mut self) {
        release_admitted_callback(&ADMITTED_ALLOCATOR_CALLBACKS);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatBinding {
    pub combat_ptr: usize,
    pub combat_id: u32,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTerminalRecordCapture {
    pub combat_id: u32,
    pub generation: u64,
    pub record_index: u32,
    pub writer_kind: u32,
    pub record_bytes: [u8; TERMINAL_RECORD_LEN],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalDrain {
    pub records: Vec<RawTerminalRecordCapture>,
    pub dropped_lock_busy: u64,
    pub dropped_unbound: u64,
    pub dropped_unreadable: u64,
    pub dropped_overflow: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRecordReservation {
    pub reservation_id: u64,
    pub drain: TerminalDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedTerminalReservation {
    pub reservation_id: u64,
    pub drain: TerminalDrain,
}

type ReservedRecordIdentity = (u32, u64, u32, u32);

fn reserved_record_identity(record: &RawTerminalRecordCapture) -> ReservedRecordIdentity {
    (
        record.combat_id,
        record.generation,
        record.record_index,
        record.writer_kind,
    )
}

struct ReservedRecordCustody {
    reservation_id: u64,
    identities: Vec<ReservedRecordIdentity>,
    consumed: Option<Vec<usize>>,
}

#[derive(Clone, Copy)]
struct ActiveTerminalReservation {
    reservation_id: u64,
    /// `record_count` entries are pinned for the reservation's lifetime.
    record_count: usize,
    diagnostics: [u64; 4],
    sealed_terminal: bool,
}

struct HookState {
    bindings: BTreeMap<usize, (u32, u64)>,
    records: VecDeque<RawTerminalRecordCapture>,
    reservation: Option<ActiveTerminalReservation>,
    reserved_custody: Option<ReservedRecordCustody>,
    reservation_admission_sealed: bool,
}

struct HookReceiptState {
    installed: [Option<InstalledRvaHookReceipt>; 3],
    released: [Option<ReleasedRvaHookReceipt>; 3],
    release_delivery_committed: bool,
    aborted_install_removed: bool,
    aborted_release_history: Vec<[Option<ReleasedRvaHookReceipt>; 3]>,
    rollback_relays: Vec<usize>,
}

impl Default for HookReceiptState {
    fn default() -> Self {
        Self {
            installed: [None; 3],
            released: [None; 3],
            release_delivery_committed: false,
            aborted_install_removed: false,
            aborted_release_history: Vec::new(),
            rollback_relays: Vec::new(),
        }
    }
}

impl Default for HookState {
    fn default() -> Self {
        Self {
            bindings: BTreeMap::new(),
            records: VecDeque::with_capacity(MAX_PENDING_RECORDS),
            reservation: None,
            reserved_custody: None,
            reservation_admission_sealed: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHookAddresses {
    pub normal_writer: usize,
    pub overrun_writer: usize,
    pub record_allocator: usize,
}

fn addresses(base: usize) -> InstalledHookAddresses {
    InstalledHookAddresses {
        normal_writer: base.saturating_add(NORMAL_WRITER_RVA),
        overrun_writer: base.saturating_add(OVERRUN_WRITER_RVA),
        record_allocator: base.saturating_add(RECORD_ALLOCATOR_RVA),
    }
}

fn expected_hooks() -> [ExpectedRvaHook; 3] {
    [
        ExpectedRvaHook {
            logical_name: "gameplay.d55.terminal_normal_writer",
            requested_rva: NORMAL_WRITER_RVA as u64,
            target_patch_len: 5,
            primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
            original_prefix: &NORMAL_SENTINEL,
        },
        ExpectedRvaHook {
            logical_name: "gameplay.d55.terminal_overrun_writer",
            requested_rva: OVERRUN_WRITER_RVA as u64,
            target_patch_len: 5,
            primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
            original_prefix: &OVERRUN_SENTINEL,
        },
        ExpectedRvaHook {
            logical_name: "gameplay.d55.terminal_record_allocator",
            requested_rva: RECORD_ALLOCATOR_RVA as u64,
            target_patch_len: 5,
            primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
            original_prefix: &ALLOCATOR_SENTINEL,
        },
    ]
}

const HOOK_NAMES: [&str; 3] = [
    "gameplay.d55.terminal_normal_writer",
    "gameplay.d55.terminal_overrun_writer",
    "gameplay.d55.terminal_record_allocator",
];

fn hook_batch(targets: &[usize; 3]) -> [(&'static str, *mut c_void); 3] {
    core::array::from_fn(|index| (HOOK_NAMES[index], targets[index] as *mut c_void))
}

fn writer_hook_batch(sites: InstalledHookAddresses) -> [(&'static str, *mut c_void); 2] {
    [
        (HOOK_NAMES[0], sites.normal_writer as *mut c_void),
        (HOOK_NAMES[1], sites.overrun_writer as *mut c_void),
    ]
}

fn complete_installed_receipts(
    state: &HookReceiptState,
) -> Result<[InstalledRvaHookReceipt; 3], String> {
    match state.installed {
        [Some(normal), Some(overrun), Some(allocator)] => Ok([normal, overrun, allocator]),
        _ => Err(format!(
            "terminal installed hook receipts are incomplete ({:?})",
            state.installed.map(|receipt| receipt.is_some())
        )),
    }
}

fn complete_released_receipts(
    state: &HookReceiptState,
) -> Result<[ReleasedRvaHookReceipt; 3], String> {
    match state.released {
        [Some(normal), Some(overrun), Some(allocator)] => Ok([normal, overrun, allocator]),
        _ => Err(format!(
            "terminal released hook receipts are incomplete ({:?})",
            state.released.map(|receipt| receipt.is_some())
        )),
    }
}

/// nothing to release.
pub fn terminal_release_pending() -> bool {
    terminal_release_pending_from(
        INSTALLED.load(Ordering::Acquire),
        CREATED_HOOK_MASK.load(Ordering::Acquire),
    )
}

/// The state logic of [`terminal_release_pending`], separated from the process
pub fn terminal_release_pending_from(installed: bool, created_mask: usize) -> bool {
    installed || created_mask != 0
}

/// Whether this lane still owns RELEASE CUSTODY that no transaction has
/// resolved: it published its three released-hook receipts during a seal, and
/// This is a DIFFERENT question from [`terminal_release_pending`], which asks
/// leaves -- both of those answer false while the released receipts sit here
/// with `release_delivery_committed == false` and the drained records still in
/// `take_released_hook_receipts` refuses it with "terminal released hook
pub fn terminal_release_custody_unresolved() -> bool {
    let receipts = match HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
    {
        Ok(receipts) => receipts,
        Err(_) => return true,
    };
    terminal_release_custody_unresolved_from(
        receipts.released.iter().any(Option::is_some),
        receipts.release_delivery_committed,
    )
}

/// The state logic of [`terminal_release_custody_unresolved`], separated from
pub const fn terminal_release_custody_unresolved_from(
    any_released_receipt_held: bool,
    release_delivery_committed: bool,
) -> bool {
    any_released_receipt_held && !release_delivery_committed
}

pub fn installed_hook_receipts() -> Result<[InstalledRvaHookReceipt; 3], String> {
    let state = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "terminal hook receipt mutex poisoned".to_string())?;
    complete_installed_receipts(&state)
}

pub fn take_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 3], String> {
    let _individual = try_admit_individual_api("terminal release receipt handoff")?;
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal hook release handoff mutex poisoned".to_string())?;
    if INSTALLED.load(Ordering::Acquire) {
        return Err(
            "terminal released hook receipts requested while hooks remain installed".into(),
        );
    }
    let state_mutex = STATE
        .get()
        .ok_or_else(|| "terminal hook state is unavailable during release handoff".to_string())?;
    let state = state_mutex
        .lock()
        .map_err(|_| "terminal hook state mutex poisoned during release handoff".to_string())?;
    if !state.reservation_admission_sealed {
        return Err("terminal released hook receipts requested before admission was sealed".into());
    }
    if state.reservation.is_some() {
        return Err(
            "terminal released hook receipts await sealed terminal reservation commit".into(),
        );
    }
    let diagnostics = diagnostic_snapshot();
    if !state.records.is_empty() || diagnostics.iter().any(|count| *count != 0) {
        return Err(format!(
            "terminal released hook receipt handoff retains terminal custody (records={}, diagnostics={diagnostics:?})",
            state.records.len()
        ));
    }
    let mut state = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "terminal hook receipt mutex poisoned".to_string())?;
    if !state.release_delivery_committed {
        return Err("terminal released hook receipts await sink commit".into());
    }
    let released = complete_released_receipts(&state)?;
    state.released = [None; 3];
    state.release_delivery_committed = false;
    Ok(released)
}

/// Borrow the exact released receipt set for the combined D55 terminal
pub(crate) fn combined_released_hook_receipts() -> Result<[ReleasedRvaHookReceipt; 3], String> {
    if !individual_terminal_api_blocked() {
        return Err("terminal combined release receipt snapshot requires combined custody".into());
    }
    if INSTALLED.load(Ordering::Acquire) {
        return Err("terminal combined release receipt snapshot requires released hooks".into());
    }
    let carrier = STATE
        .get()
        .ok_or_else(|| {
            "terminal hook state is unavailable during combined receipt snapshot".to_string()
        })?
        .lock()
        .map_err(|_| {
            "terminal hook state mutex poisoned during combined receipt snapshot".to_string()
        })?;
    match carrier.reservation {
        Some(active) if active.sealed_terminal && carrier.reservation_admission_sealed => {}
        _ => {
            return Err(
                "terminal combined release receipt snapshot requires sealed carrier custody".into(),
            )
        }
    }
    let state = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "terminal hook receipt mutex poisoned during combined snapshot".to_string())?;
    if state.release_delivery_committed {
        return Err("terminal combined release receipt delivery was already committed".into());
    }
    complete_released_receipts(&state)
}

pub fn install() -> Result<InstalledHookAddresses, String> {
    let _individual = try_admit_individual_api("terminal install")?;
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal writer install mutex poisoned".to_string())?;
    if INSTALLED.load(Ordering::Acquire) {
        let sites = addresses(MODULE_BASE.load(Ordering::Acquire));
        live_reobserve_installed_hook_receipts(sites)?;
        return Ok(sites);
    }
    resolve_aborted_install_receipts()?;
    {
        let receipt_state = HOOK_RECEIPTS
            .get_or_init(|| Mutex::new(HookReceiptState::default()))
            .lock()
            .map_err(|_| "terminal hook receipt mutex poisoned during install".to_string())?;
        if receipt_state.installed.iter().any(Option::is_some) {
            return Err("terminal hook has unresolved installed receipts while uninstalled".into());
        }
        if receipt_state.released.iter().any(Option::is_some) {
            return Err("terminal hook release receipts must be taken before reinstall".into());
        }
        if receipt_state.release_delivery_committed {
            return Err("terminal hook has committed delivery without released receipts".into());
        }
        if !receipt_state.rollback_relays.is_empty() {
            return Err("terminal hook retains failed-install relay custody".into());
        }
        if CREATED_HOOK_MASK.load(Ordering::Acquire) != 0 {
            return Err("terminal hook retains failed-install created-site custody".into());
        }
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let sites = addresses(base);
    verify_sentinel(
        sites.normal_writer,
        &NORMAL_SENTINEL,
        "normal terminal writer",
    )?;
    verify_sentinel(
        sites.overrun_writer,
        &OVERRUN_SENTINEL,
        "overrun terminal writer",
    )?;
    verify_sentinel(
        sites.record_allocator,
        &ALLOCATOR_SENTINEL,
        "terminal record allocator",
    )?;
    MODULE_BASE.store(base, Ordering::Release);
    STATE.get_or_init(|| Mutex::new(HookState::default()));

    let normal = match unsafe {
        MinHook::create_hook(
            sites.normal_writer as *mut c_void,
            hook_normal_writer as *mut c_void,
        )
    } {
        Ok(value) => value,
        Err(status) => {
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!(
                "normal terminal writer hook creation failed: {status}"
            ));
        }
    };
    CREATED_HOOK_MASK.fetch_or(1 << 0, Ordering::AcqRel);
    let overrun = match unsafe {
        MinHook::create_hook(
            sites.overrun_writer as *mut c_void,
            hook_overrun_writer as *mut c_void,
        )
    } {
        Ok(value) => {
            CREATED_HOOK_MASK.fetch_or(1 << 1, Ordering::AcqRel);
            value
        }
        Err(status) => {
            let cleanup = rollback_failed_install(sites);
            return Err(match cleanup {
                Ok(()) => format!("overrun terminal writer hook creation failed: {status}"),
                Err(cleanup) => format!(
                    "overrun terminal writer hook creation failed: {status}; rollback failed: {cleanup}"
                ),
            });
        }
    };
    let allocator = match unsafe {
        MinHook::create_hook(
            sites.record_allocator as *mut c_void,
            hook_record_allocator as *mut c_void,
        )
    } {
        Ok(value) => {
            CREATED_HOOK_MASK.fetch_or(1 << 2, Ordering::AcqRel);
            value
        }
        Err(status) => {
            let cleanup = rollback_failed_install(sites);
            return Err(match cleanup {
                Ok(()) => format!("terminal record allocator hook creation failed: {status}"),
                Err(cleanup) => format!(
                    "terminal record allocator hook creation failed: {status}; rollback failed: {cleanup}"
                ),
            });
        }
    };
    if let Err(error) = open_reservation_admission_for_install() {
        let cleanup = rollback_failed_install(sites);
        return Err(match cleanup {
            Ok(()) => error,
            Err(cleanup) => format!("{error}; rollback failed: {cleanup}"),
        });
    }
    ORIGINAL_NORMAL.store(normal as usize, Ordering::Release);
    ORIGINAL_OVERRUN.store(overrun as usize, Ordering::Release);
    ORIGINAL_ALLOCATOR.store(allocator as usize, Ordering::Release);
    let mut batch = hook_batch(&[
        sites.normal_writer,
        sites.overrun_writer,
        sites.record_allocator,
    ]);
    batch.reverse();
    let enabled = unsafe { inline_detour::enable_batch(&batch) };
    if let Some((name, status)) = enabled.refusal() {
        return Err(fail_open_install(
            sites,
            format!("terminal hook enable failed for {name}: {status}"),
        ));
    }
    match observe_installed_hook_receipts(sites) {
        Ok(_) => {}
        Err(error) => {
            return Err(fail_open_install(
                sites,
                format!("terminal hook installed receipt failed: {error}"),
            ));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(sites)
}

fn observe_installed_hook_receipts(
    sites: InstalledHookAddresses,
) -> Result<[InstalledRvaHookReceipt; 3], String> {
    let expected = expected_hooks();
    let targets = [
        sites.normal_writer,
        sites.overrun_writer,
        sites.record_allocator,
    ];
    let detours = [
        hook_normal_writer as usize,
        hook_overrun_writer as usize,
        hook_record_allocator as usize,
    ];
    for index in 0..3 {
        let mut state = HOOK_RECEIPTS
            .get_or_init(|| Mutex::new(HookReceiptState::default()))
            .lock()
            .map_err(|_| "terminal hook receipt mutex poisoned during install".to_string())?;
        if state.released[index].is_some() {
            return Err(format!(
                "{} already has a released receipt",
                expected[index].logical_name
            ));
        }
        if state.installed[index].is_some() {
            continue;
        }
        let owner_token = next_teardown_owner_token()?;
        let observed = observe_enabled_minhook_rel5(
            expected[index],
            targets[index],
            detours[index],
            owner_token,
        )?;
        state.installed[index] = Some(observed);
    }
    installed_hook_receipts()
}

fn live_reobserve_installed_hook_receipts(
    sites: InstalledHookAddresses,
) -> Result<[InstalledRvaHookReceipt; 3], String> {
    let stored = installed_hook_receipts()?;
    let expected = expected_hooks();
    let targets = [
        sites.normal_writer,
        sites.overrun_writer,
        sites.record_allocator,
    ];
    let detours = [
        hook_normal_writer as usize,
        hook_overrun_writer as usize,
        hook_record_allocator as usize,
    ];
    for index in 0..3 {
        let receipt = stored[index];
        if receipt.logical_name != expected[index].logical_name
            || receipt.requested_rva != expected[index].requested_rva
            || receipt.observed_rva != expected[index].requested_rva
            || receipt.observed_runtime_address != targets[index]
            || receipt.observed_detour_address != detours[index]
            || !receipt.enabled
            || receipt.patch_above
        {
            return Err(format!(
                "{} stored receipt changed on install retry",
                expected[index].logical_name
            ));
        }
        verify_live_minhook_rel5(
            targets[index],
            receipt.observed_relay_address,
            detours[index],
        )?;
    }
    Ok(stored)
}

fn verify_live_minhook_rel5(
    target: usize,
    expected_relay: usize,
    expected_detour: usize,
) -> Result<(), String> {
    let patched = read_exact_bytes(target, 5)?;
    if patched.first().copied() != Some(0xe9) {
        return Err(format!(
            "terminal live MinHook target 0x{target:X} is not JMP rel32"
        ));
    }
    let displacement = i32::from_le_bytes(
        patched[1..5]
            .try_into()
            .map_err(|_| "terminal live MinHook displacement is truncated")?,
    );
    let relay = target
        .checked_add(5)
        .and_then(|next| next.checked_add_signed(displacement as isize))
        .ok_or("terminal live MinHook relay address overflow")?;
    if relay != expected_relay {
        return Err(format!(
            "terminal live MinHook relay mismatch (0x{relay:X} != 0x{expected_relay:X})"
        ));
    }
    let relay_stub = read_exact_bytes(relay, MINHOOK_RELAY_LEN)?;
    if relay_stub[..6] != [0xff, 0x25, 0x00, 0x00, 0x00, 0x00] {
        return Err("terminal live MinHook relay stub mismatch".into());
    }
    let detour_u64 = u64::from_le_bytes(
        relay_stub[6..14]
            .try_into()
            .map_err(|_| "terminal live MinHook detour is truncated")?,
    );
    let detour = usize::try_from(detour_u64)
        .map_err(|_| "terminal live MinHook detour does not fit usize")?;
    if detour != expected_detour {
        return Err(format!(
            "terminal live MinHook detour mismatch (0x{detour:X} != 0x{expected_detour:X})"
        ));
    }
    Ok(())
}

fn open_reservation_admission_for_install() -> Result<(), String> {
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook state mutex poisoned during install admission".to_string())?;
    let diagnostics = diagnostic_snapshot();
    if !state.records.is_empty()
        || state.reservation.is_some()
        || diagnostics.iter().any(|count| *count != 0)
    {
        return Err(format!(
            "terminal hook install refused residual state (records={}, reserved={}, diagnostics={diagnostics:?})",
            state.records.len(),
            state.reservation.is_some()
        ));
    }
    state.bindings.clear();
    state.reservation_admission_sealed = false;
    Ok(())
}

fn fail_open_install(sites: InstalledHookAddresses, primary: String) -> String {
    let seal = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook state mutex poisoned while resealing install".to_string())
        .map(|mut state| state.reservation_admission_sealed = true);
    let cleanup = rollback_failed_install(sites);
    let release = if cleanup.is_ok() {
        mark_aborted_install_removed()
            .and_then(|()| observe_release_receipt_slots())
            .and_then(|()| archive_resolved_aborted_install())
    } else {
        Ok(())
    };
    if cleanup.is_ok() && release.is_ok() {
        clear_hook_runtime_addresses();
    }
    let mut errors = vec![primary];
    if let Err(error) = seal {
        errors.push(error);
    }
    if let Err(error) = cleanup {
        errors.push(format!("rollback failed: {error}"));
    }
    if let Err(error) = release {
        errors.push(format!("partial receipt release failed: {error}"));
    }
    errors.join("; ")
}

fn mark_aborted_install_removed() -> Result<(), String> {
    let mut state = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "terminal hook receipt mutex poisoned during abort".to_string())?;
    state.aborted_install_removed = true;
    Ok(())
}

fn archive_resolved_aborted_install() -> Result<(), String> {
    let mut state = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "terminal hook receipt mutex poisoned during abort archive".to_string())?;
    if state.installed.iter().any(Option::is_some) {
        return Err("terminal aborted install still owns installed receipts".into());
    }
    if state.released.iter().any(Option::is_some) {
        let released = state.released;
        state.aborted_release_history.push(released);
        state.released = [None; 3];
    }
    state.release_delivery_committed = false;
    state.aborted_install_removed = false;
    Ok(())
}

fn resolve_aborted_install_receipts() -> Result<(), String> {
    let (mut removed, installed_pending, rollback_pending) = {
        let state = HOOK_RECEIPTS
            .get_or_init(|| Mutex::new(HookReceiptState::default()))
            .lock()
            .map_err(|_| "terminal hook receipt mutex poisoned during abort retry".to_string())?;
        (
            state.aborted_install_removed,
            state.installed.iter().any(Option::is_some),
            !state.rollback_relays.is_empty(),
        )
    };
    let created_pending = CREATED_HOOK_MASK.load(Ordering::Acquire) != 0;
    if !removed && !installed_pending && !rollback_pending && !created_pending {
        return Ok(());
    }
    if !removed {
        let base = MODULE_BASE.load(Ordering::Acquire);
        if base == 0 {
            return Err("terminal aborted install has no module base for rollback retry".into());
        }
        rollback_failed_install(addresses(base))?;
        mark_aborted_install_removed()?;
        removed = true;
    }
    debug_assert!(removed);
    observe_release_receipt_slots()?;
    archive_resolved_aborted_install()?;
    clear_hook_runtime_addresses();
    Ok(())
}

fn rollback_failed_install(sites: InstalledHookAddresses) -> Result<(), String> {
    let live_relays = observe_live_terminal_relays(sites)?;
    let retained_relays = {
        let mut receipts = HOOK_RECEIPTS
            .get_or_init(|| Mutex::new(HookReceiptState::default()))
            .lock()
            .map_err(|_| {
                "terminal hook receipt mutex poisoned while retaining rollback relays".to_string()
            })?;
        for relay in live_relays {
            if !receipts.rollback_relays.contains(&relay) {
                receipts.rollback_relays.push(relay);
            }
        }
        receipts.rollback_relays.clone()
    };
    let mut errors = Vec::new();
    errors.extend(
        unsafe { inline_detour::disable_batch(&writer_hook_batch(sites)) }
            .failures()
            .into_iter()
            .map(|(name, status)| format!("disable {name}: {status}")),
    );
    if errors.is_empty() {
        if let Err(error) = wait_for_writer_quiescence() {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        match unsafe { inline_detour::disable(sites.record_allocator as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => errors.push(format!("disable 0x{:X}: {status}", sites.record_allocator)),
        }
    }
    if errors.is_empty() {
        if let Err(error) = wait_for_allocator_quiescence() {
            errors.push(error);
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let _retained_relays = retained_relays;
    let mut removal = hook_batch(&[
        sites.normal_writer,
        sites.overrun_writer,
        sites.record_allocator,
    ]);
    removal.reverse();
    for ((name, outcome), index) in unsafe { inline_detour::remove_batch(&removal) }
        .outcomes()
        .iter()
        .zip([2usize, 1, 0])
    {
        match outcome {
            Ok(()) => {
                CREATED_HOOK_MASK.fetch_and(!(1usize << index), Ordering::AcqRel);
            }
            Err(status) => errors.push(format!("remove {name}: {status}")),
        }
    }
    for (address, expected, label) in [
        (
            sites.normal_writer,
            NORMAL_SENTINEL.as_slice(),
            "normal terminal writer",
        ),
        (
            sites.overrun_writer,
            OVERRUN_SENTINEL.as_slice(),
            "overrun terminal writer",
        ),
        (
            sites.record_allocator,
            ALLOCATOR_SENTINEL.as_slice(),
            "terminal record allocator",
        ),
    ] {
        if let Err(error) = verify_sentinel(address, expected, label) {
            errors.push(error);
        }
    }
    if errors.is_empty() && CREATED_HOOK_MASK.load(Ordering::Acquire) == 0 {
        let mut receipts = HOOK_RECEIPTS
            .get_or_init(|| Mutex::new(HookReceiptState::default()))
            .lock()
            .map_err(|_| {
                "terminal hook receipt mutex poisoned while clearing rollback relays".to_string()
            })?;
        receipts.rollback_relays.clear();
        Ok(())
    } else {
        if CREATED_HOOK_MASK.load(Ordering::Acquire) != 0 {
            errors.push(format!(
                "terminal failed-install created-site custody remains at mask 0x{:X}",
                CREATED_HOOK_MASK.load(Ordering::Acquire)
            ));
        }
        Err(errors.join("; "))
    }
}

fn observe_live_terminal_relays(sites: InstalledHookAddresses) -> Result<Vec<usize>, String> {
    let mut relays = Vec::with_capacity(3);
    for (target, original, label) in [
        (
            sites.normal_writer,
            NORMAL_SENTINEL.as_slice(),
            "normal terminal writer",
        ),
        (
            sites.overrun_writer,
            OVERRUN_SENTINEL.as_slice(),
            "overrun terminal writer",
        ),
        (
            sites.record_allocator,
            ALLOCATOR_SENTINEL.as_slice(),
            "terminal allocator",
        ),
    ] {
        let bytes = read_exact_bytes(target, 5)?;
        if bytes == original[..5] {
            continue;
        }
        if bytes[0] != 0xE9 {
            return Err(format!(
                "{label} has neither its original prefix nor an E9 MinHook relay"
            ));
        }
        let displacement = i32::from_le_bytes(
            bytes[1..5]
                .try_into()
                .expect("fixed terminal MinHook rel32 slice"),
        ) as isize;
        let relay = target
            .checked_add(5)
            .and_then(|next| next.checked_add_signed(displacement))
            .ok_or_else(|| format!("{label} MinHook relay destination overflowed"))?;
        relays.push(relay);
    }
    Ok(relays)
}

fn clear_hook_runtime_addresses() {
    clear_hook_trampoline_addresses();
    MODULE_BASE.store(0, Ordering::Release);
}

fn clear_hook_trampoline_addresses() {
    ORIGINAL_NORMAL.store(0, Ordering::Release);
    ORIGINAL_OVERRUN.store(0, Ordering::Release);
    ORIGINAL_ALLOCATOR.store(0, Ordering::Release);
}

fn release_hook_receipts(
    sites: InstalledHookAddresses,
) -> Result<[ReleasedRvaHookReceipt; 3], String> {
    // DRAINED-BY-CALLER: the seal above this call disables both writers,
    // `wait_for_writer_quiescence()?`, disables the allocator, and
    // `wait_for_allocator_quiescence()?` -- each with `?`, so a refused disable
    // or a drain that did not reach zero returns BEFORE reaching here rather
    // than being recorded and freed over. The two groups are drained
    // separately because the allocator admits on its own schedule, which is
    // why this cannot be restated as one wait in here.
    let installed = {
        let state = HOOK_RECEIPTS
            .get_or_init(|| Mutex::new(HookReceiptState::default()))
            .lock()
            .map_err(|_| "terminal hook receipt mutex poisoned before release".to_string())?;
        if state.aborted_install_removed {
            return Err("terminal production release overlaps an aborted install".into());
        }
        if state.released.iter().any(Option::is_some) {
            return Err("terminal production release has pre-existing released slots".into());
        }
        complete_installed_receipts(&state)?
    };
    if CREATED_HOOK_MASK.load(Ordering::Acquire) != 0b111 {
        return Err(format!(
            "terminal production release created-site mask is 0x{:X}, expected 0x7",
            CREATED_HOOK_MASK.load(Ordering::Acquire)
        ));
    }
    let mut removal = hook_batch(&[
        sites.normal_writer,
        sites.overrun_writer,
        sites.record_allocator,
    ]);
    removal.reverse();
    // Production release frees all three relays with one freeze instead of
    let removed = unsafe { inline_detour::remove_batch(&removal) };
    let mut removal_error = None;
    for ((name, outcome), index) in removed.outcomes().iter().zip([2usize, 1, 0]) {
        match outcome {
            Ok(()) => {
                CREATED_HOOK_MASK.fetch_and(!(1usize << index), Ordering::AcqRel);
            }
            Err(status) => {
                removal_error.get_or_insert_with(|| {
                    format!("terminal hook removal failed for {name}: {status}")
                });
            }
        }
    }
    if let Some(error) = removal_error {
        return Err(error);
    }
    if CREATED_HOOK_MASK.load(Ordering::Acquire) != 0 {
        return Err(format!(
            "terminal production release retains created-site mask 0x{:X}",
            CREATED_HOOK_MASK.load(Ordering::Acquire)
        ));
    }
    let released = observe_released_set(
        expected_hooks(),
        installed,
        [completed_teardown_attestation(); 3],
    )?;
    let mut state = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "terminal hook receipt mutex poisoned during release".to_string())?;
    if state.installed != installed.map(Some) || state.released.iter().any(Option::is_some) {
        return Err("terminal production release custody changed before publication".into());
    }
    state.installed = [None; 3];
    state.released = released.map(Some);
    state.release_delivery_committed = false;
    Ok(released)
}

fn observe_release_receipt_slots() -> Result<(), String> {
    let expected = expected_hooks();
    for index in 0..3 {
        let mut state = HOOK_RECEIPTS
            .get_or_init(|| Mutex::new(HookReceiptState::default()))
            .lock()
            .map_err(|_| "terminal hook receipt mutex poisoned during release".to_string())?;
        if state.released[index].is_some() {
            continue;
        }
        let Some(installed) = state.installed[index] else {
            continue;
        };
        let released =
            observe_released(expected[index], installed, completed_teardown_attestation())?;
        state.released[index] = Some(released);
        state.installed[index] = None;
    }
    Ok(())
}

fn completed_teardown_attestation() -> HookTeardownAttestation {
    HookTeardownAttestation {
        callbacks_quiesced: true,
        hook_removed: true,
        relay_freed: true,
    }
}

pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn uninstall() -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal uninstall")?;
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal writer uninstall mutex poisoned".to_string())?;
    if INSTALLED.load(Ordering::Acquire) {
        return Err(
            "terminal hook uninstall requires explicit sealed reserve/commit/rollback custody"
                .into(),
        );
    }
    if let Some(state_mutex) = STATE.get() {
        let state = state_mutex
            .lock()
            .map_err(|_| "terminal hook state mutex poisoned during uninstall guard".to_string())?;
        if state.reservation.is_some() {
            return Err("terminal hook uninstall observed uncommitted sealed custody".into());
        }
        let diagnostics = diagnostic_snapshot();
        if !state.records.is_empty() || diagnostics.iter().any(|count| *count != 0) {
            return Err(format!(
                "terminal hook uninstall observed residual carrier custody (records={}, diagnostics={diagnostics:?})",
                state.records.len()
            ));
        }
    }
    let receipts = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "terminal hook receipt mutex poisoned during uninstall guard".to_string())?;
    if receipts.installed.iter().any(Option::is_some) {
        return Err("terminal hook uninstall observed unresolved installed receipt custody".into());
    }
    if receipts.released.iter().any(Option::is_some) && !receipts.release_delivery_committed {
        return Err("terminal hook uninstall observed uncommitted released receipt custody".into());
    }
    Ok(())
}

/// Stop hook admission, release all three exact hook custodies, and reserve the
pub fn seal_and_reserve_all_for_uninstall() -> Result<SealedTerminalReservation, String> {
    let _individual = try_admit_individual_api("terminal seal")?;
    seal_and_reserve_all_impl()
}

pub(crate) fn seal_and_reserve_all_for_combined() -> Result<SealedTerminalReservation, String> {
    if !individual_terminal_api_blocked() {
        return Err("terminal combined seal requires combined D55 custody".into());
    }
    seal_and_reserve_all_impl()
}

fn seal_and_reserve_all_impl() -> Result<SealedTerminalReservation, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal writer seal mutex poisoned".to_string())?;
    let was_installed = INSTALLED.load(Ordering::Acquire);
    let sites = was_installed.then(|| addresses(MODULE_BASE.load(Ordering::Acquire)));
    let state_mutex = STATE
        .get()
        .ok_or_else(|| "terminal hook state is unavailable during seal".to_string())?;
    {
        let mut state = state_mutex
            .lock()
            .map_err(|_| "terminal hook state mutex poisoned during seal".to_string())?;
        if state.reservation.is_some() {
            return Err("terminal sealed carrier already has an active reservation".into());
        }
        state.reservation_admission_sealed = true;
    }
    if let Some(sites) = sites {
        if let Some((name, status)) =
            unsafe { inline_detour::disable_batch(&writer_hook_batch(sites)) }.first_failure()
        {
            return Err(format!("terminal writer seal failed for {name}: {status}"));
        }
        wait_for_writer_quiescence()?;
        match unsafe { inline_detour::disable(sites.record_allocator as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => {
                return Err(format!(
                    "terminal allocator seal failed at 0x{:X}: {status}",
                    sites.record_allocator
                ));
            }
        }
        wait_for_allocator_quiescence()?;
    }
    let mut state = state_mutex
        .lock()
        .map_err(|_| "terminal hook state mutex poisoned during seal".to_string())?;
    // This lock is held through release-custody publication and full carrier
    if let Some(sites) = sites {
        release_hook_receipts(sites)?;
        INSTALLED.store(false, Ordering::Release);
        // Trampolines cease to be callable after exact release, but the
        clear_hook_trampoline_addresses();
        WRITER_CONTEXT.with(|slot| slot.set(None));
    } else {
        let receipts = HOOK_RECEIPTS
            .get_or_init(|| Mutex::new(HookReceiptState::default()))
            .lock()
            .map_err(|_| "terminal hook receipt mutex poisoned during seal retry".to_string())?;
        complete_released_receipts(&receipts)?;
        if receipts.release_delivery_committed {
            return Err("terminal sealed carrier was already sink-committed".into());
        }
    }
    let reservation = reserve_full_terminal_drain_locked(&mut state)?;
    state.bindings.clear();
    debug_assert!(state.reservation_admission_sealed);
    drop(state);
    Ok(reservation)
}

fn reserve_full_terminal_drain_locked(
    state: &mut HookState,
) -> Result<SealedTerminalReservation, String> {
    require_no_retained_custody(state)?;
    let reservation_id = next_terminal_reservation_id()?;
    let diagnostics = diagnostic_snapshot();
    let record_count = state.records.len();
    let drain = TerminalDrain {
        records: state.records.iter().cloned().collect(),
        dropped_lock_busy: diagnostics[0],
        dropped_unbound: diagnostics[1],
        dropped_unreadable: diagnostics[2],
        dropped_overflow: diagnostics[3],
    };
    state.reservation = Some(ActiveTerminalReservation {
        reservation_id,
        record_count,
        diagnostics,
        sealed_terminal: true,
    });
    Ok(SealedTerminalReservation {
        reservation_id,
        drain,
    })
}

pub fn commit_sealed_terminal_reservation(reservation_id: u64) -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal sealed commit")?;
    consume_sealed_terminal_reservation(reservation_id, "commit").map(|_| ())
}

/// finish, and resolves the release custody the hooks already left behind.
/// detours and published their release receipts by the time this runs, so this
pub fn abort_sealed_terminal_reservation(
    reservation_id: u64,
) -> Result<InstallAbortDiscard, String> {
    let _individual = try_admit_individual_api("terminal sealed install abort")?;
    consume_sealed_terminal_reservation(reservation_id, "install abort")
}

/// Seals, physically releases and then discards the whole D55 terminal carrier
pub fn abort_partial_install_and_uninstall() -> Result<InstallAbortDiscard, String> {
    let reservation = seal_and_reserve_all_for_uninstall()?;
    abort_sealed_terminal_reservation(reservation.reservation_id)
}

fn consume_sealed_terminal_reservation(
    reservation_id: u64,
    label: &str,
) -> Result<InstallAbortDiscard, String> {
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| format!("terminal sealed {label} mutex poisoned"))?;
    let state_mutex = STATE
        .get()
        .ok_or_else(|| format!("terminal hook state is unavailable during sealed {label}"))?;
    let mut state = state_mutex
        .lock()
        .map_err(|_| format!("terminal hook state mutex poisoned during sealed {label}"))?;
    let active = matching_sealed_reservation(&state, reservation_id)?;
    validate_reserved_diagnostics(active, reservation_id)?;
    if state.records.len() < active.record_count {
        return Err("terminal sealed reservation lost its record prefix".into());
    }
    let mut receipts = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| format!("terminal hook receipt mutex poisoned during sealed {label}"))?;
    complete_released_receipts(&receipts)?;
    if receipts.release_delivery_committed {
        return Err("terminal sealed release delivery was already committed".into());
    }
    let discard = InstallAbortDiscard {
        retained_rows: active.record_count as u64,
        reserved_rows: active.record_count as u64,
        discarded_diagnostics: active.diagnostics.iter().copied().sum(),
        poisoned: active.diagnostics.iter().any(|count| *count != 0),
    };
    state.records.drain(..active.record_count);
    subtract_reserved_diagnostics(active);
    state.reservation = None;
    state.reserved_custody = None;
    receipts.release_delivery_committed = true;
    MODULE_BASE.store(0, Ordering::Release);
    Ok(discard)
}

pub fn rollback_sealed_terminal_reservation(reservation_id: u64) -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal sealed rollback")?;
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal sealed rollback mutex poisoned".to_string())?;
    let state_mutex = STATE
        .get()
        .ok_or_else(|| "terminal hook state is unavailable during sealed rollback".to_string())?;
    let mut state = state_mutex
        .lock()
        .map_err(|_| "terminal hook state mutex poisoned during sealed rollback".to_string())?;
    matching_sealed_reservation(&state, reservation_id)?;
    state.reservation = None;
    state.reserved_custody = None;
    Ok(())
}

pub(crate) fn preflight_combined_commit(reservation_id: u64) -> Result<(), String> {
    if !individual_terminal_api_blocked() {
        return Err("terminal combined commit preflight requires combined D55 custody".into());
    }
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal combined commit preflight mutex poisoned".to_string())?;
    let state_mutex = STATE.get().ok_or_else(|| {
        "terminal hook state is unavailable during combined commit preflight".to_string()
    })?;
    let state = state_mutex.lock().map_err(|_| {
        "terminal hook state mutex poisoned during combined commit preflight".to_string()
    })?;
    let active = matching_sealed_reservation(&state, reservation_id)?;
    validate_reserved_diagnostics(active, reservation_id)?;
    if state.records.len() < active.record_count {
        return Err("terminal combined reservation lost its record prefix".into());
    }
    let receipts = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| {
            "terminal hook receipt mutex poisoned during combined commit preflight".to_string()
        })?;
    complete_released_receipts(&receipts)?;
    if receipts.release_delivery_committed {
        return Err("terminal combined release delivery was already committed".into());
    }
    Ok(())
}

pub(crate) fn commit_combined_preflighted(reservation_id: u64) {
    let _guard = match INSTALL_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let state_mutex = STATE
        .get()
        .expect("combined terminal commit was preflighted with state");
    let mut state = match state_mutex.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    let active = state
        .reservation
        .expect("combined terminal commit was preflighted with a reservation");
    debug_assert_eq!(active.reservation_id, reservation_id);
    debug_assert!(active.sealed_terminal);
    debug_assert!(state.reservation_admission_sealed);
    debug_assert!(state.records.len() >= active.record_count);
    state.records.drain(..active.record_count);
    subtract_reserved_diagnostics(active);
    state.reservation = None;
    state.reserved_custody = None;
    let receipt_mutex = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let mut receipts = match receipt_mutex.lock() {
        Ok(receipts) => receipts,
        Err(poisoned) => poisoned.into_inner(),
    };
    debug_assert!(complete_released_receipts(&receipts).is_ok());
    debug_assert!(!receipts.release_delivery_committed);
    receipts.released = [None; 3];
    receipts.release_delivery_committed = false;
    MODULE_BASE.store(0, Ordering::Release);
}

pub(crate) fn preflight_combined_rollback(reservation_id: u64) -> Result<(), String> {
    if !individual_terminal_api_blocked() {
        return Err("terminal combined rollback preflight requires combined D55 custody".into());
    }
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal combined rollback preflight mutex poisoned".to_string())?;
    let state_mutex = STATE.get().ok_or_else(|| {
        "terminal hook state is unavailable during combined rollback preflight".to_string()
    })?;
    let state = state_mutex.lock().map_err(|_| {
        "terminal hook state mutex poisoned during combined rollback preflight".to_string()
    })?;
    matching_sealed_reservation(&state, reservation_id)?;
    Ok(())
}

/// Release only the active reservation token after a successful combined
/// rollback preflight. The raw records, diagnostics, and release receipts stay
pub(crate) fn rollback_combined_preflighted(reservation_id: u64) {
    let _guard = match INSTALL_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let state_mutex = STATE
        .get()
        .expect("combined terminal rollback was preflighted with state");
    let mut state = match state_mutex.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    let active = state
        .reservation
        .expect("combined terminal rollback was preflighted with a reservation");
    debug_assert_eq!(active.reservation_id, reservation_id);
    debug_assert!(active.sealed_terminal);
    debug_assert!(state.reservation_admission_sealed);
    state.reservation = None;
    state.reserved_custody = None;
}

fn matching_sealed_reservation(
    state: &HookState,
    reservation_id: u64,
) -> Result<ActiveTerminalReservation, String> {
    if !state.reservation_admission_sealed {
        return Err("terminal sealed reservation admission unexpectedly reopened".into());
    }
    match state.reservation {
        Some(active) if active.reservation_id == reservation_id && active.sealed_terminal => {
            Ok(active)
        }
        Some(active) if active.reservation_id != reservation_id => Err(format!(
            "terminal sealed reservation id mismatch ({reservation_id} != {})",
            active.reservation_id
        )),
        Some(_) => Err("terminal reservation is not sealed terminal custody".into()),
        None => Err("terminal sealed reservation is unavailable".into()),
    }
}

pub fn seal_drain_and_uninstall() -> Result<TerminalDrain, String> {
    Err(
        "terminal seal_drain_and_uninstall is disabled; use explicit sealed reserve/commit/rollback custody"
            .into(),
    )
}

fn wait_for_writer_quiescence() -> Result<(), String> {
    let mut yields = 0u32;
    while ACTIVE_WRITER_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("terminal writer callbacks did not quiesce during teardown".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    Ok(())
}

fn wait_for_allocator_quiescence() -> Result<(), String> {
    let mut yields = 0u32;
    while ACTIVE_ALLOCATOR_CALLBACKS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("terminal allocator callbacks did not quiesce during teardown".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    Ok(())
}

pub(crate) fn wait_for_combined_capture_quiescence() -> Result<(), String> {
    if !individual_terminal_api_blocked() {
        return Err("terminal combined quiescence requires combined D55 custody".into());
    }
    let mut yields = 0u32;
    while ADMITTED_WRITER_CALLBACKS.load(Ordering::Acquire) != 0
        || ADMITTED_ALLOCATOR_CALLBACKS.load(Ordering::Acquire) != 0
    {
        if yields == 1_000_000 {
            return Err(
                "terminal admitted capture callbacks did not quiesce during combined teardown"
                    .into(),
            );
        }
        yields += 1;
        std::thread::yield_now();
    }
    if let Some(state_mutex) = STATE.get() {
        let state = state_mutex.lock().map_err(|_| {
            "terminal hook state mutex poisoned after combined quiescence".to_string()
        })?;
        if state.reservation.is_some() {
            return Err(
                "terminal combined cutoff found unresolved individual reservation custody".into(),
            );
        }
    }
    let receipts = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| {
            "terminal hook receipt mutex poisoned after combined quiescence".to_string()
        })?;
    if receipts.released.iter().any(Option::is_some) || receipts.release_delivery_committed {
        return Err(
            "terminal combined cutoff found unresolved individual release receipt custody".into(),
        );
    }
    if !INSTALLED.load(Ordering::Acquire) && receipts.installed.iter().any(Option::is_some) {
        return Err(
            "terminal combined cutoff found unresolved partial install receipt custody".into(),
        );
    }
    if !receipts.rollback_relays.is_empty() {
        return Err(
            "terminal combined cutoff found unresolved failed-install relay custody".into(),
        );
    }
    if !INSTALLED.load(Ordering::Acquire) && CREATED_HOOK_MASK.load(Ordering::Acquire) != 0 {
        return Err(format!(
            "terminal combined cutoff found unresolved created-site mask 0x{:X}",
            CREATED_HOOK_MASK.load(Ordering::Acquire)
        ));
    }
    Ok(())
}

pub fn publish_active_combat_bindings(bindings: &[CombatBinding]) -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal binding publication")?;
    publish_active_combat_bindings_impl(bindings)
}

fn publish_active_combat_bindings_impl(bindings: &[CombatBinding]) -> Result<(), String> {
    let mut next = BTreeMap::new();
    for binding in bindings {
        if binding.combat_ptr == 0 || binding.combat_id == 0 || binding.generation == 0 {
            return Err("invalid terminal combat binding".into());
        }
        if next
            .insert(binding.combat_ptr, (binding.combat_id, binding.generation))
            .is_some()
        {
            return Err("duplicate terminal combat pointer binding".into());
        }
    }
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook binding mutex poisoned".to_string())?;
    if state.reservation_admission_sealed {
        return Err("terminal combat binding publication is sealed".into());
    }
    state.bindings = next;
    Ok(())
}

pub(crate) fn publish_combined_final_bindings(bindings: &[CombatBinding]) -> Result<(), String> {
    if !individual_terminal_api_blocked() {
        return Err("terminal combined binding publication requires combined D55 custody".into());
    }
    publish_active_combat_bindings_impl(bindings)
}

pub fn drain() -> Result<TerminalDrain, String> {
    Err(
        "terminal drain is disabled; use reserve_pending_records with explicit commit/rollback custody"
            .into(),
    )
}

fn diagnostic_counters() -> [&'static AtomicU64; 4] {
    [
        &DROPPED_LOCK_BUSY,
        &DROPPED_UNBOUND,
        &DROPPED_UNREADABLE,
        &DROPPED_OVERFLOW,
    ]
}

fn diagnostic_snapshot() -> [u64; 4] {
    diagnostic_counters().map(|counter| counter.load(Ordering::Acquire))
}

/// hide a disposition that failed to release its own state, and a second clear
/// here would make the release-side clears untestable.
fn require_no_retained_custody(state: &HookState) -> Result<(), String> {
    match &state.reserved_custody {
        Some(custody) => Err(format!(
            "terminal hook retains record custody from reservation {} with no reservation to match",
            custody.reservation_id
        )),
        None => Ok(()),
    }
}

fn next_terminal_reservation_id() -> Result<u64, String> {
    let reservation_id = NEXT_RECORD_RESERVATION_ID
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .map_err(|_| "terminal hook reservation id exhausted".to_string())?;
    if reservation_id == 0 {
        return Err("terminal hook reservation id is zero".into());
    }
    Ok(reservation_id)
}

fn validate_reserved_diagnostics(
    active: ActiveTerminalReservation,
    reservation_id: u64,
) -> Result<(), String> {
    for (counter, snapshot) in diagnostic_counters().into_iter().zip(active.diagnostics) {
        if counter.load(Ordering::Acquire) < snapshot {
            return Err(format!(
                "terminal hook diagnostic counter fell below reservation {reservation_id}"
            ));
        }
    }
    Ok(())
}

fn subtract_reserved_diagnostics(active: ActiveTerminalReservation) {
    for (counter, snapshot) in diagnostic_counters().into_iter().zip(active.diagnostics) {
        counter.fetch_sub(snapshot, Ordering::AcqRel);
    }
}

pub fn reserve_pending_records() -> Result<Option<TerminalRecordReservation>, String> {
    let _individual = try_admit_individual_api("terminal record reservation")?;
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("terminal hook record reservation requires installed hooks".into());
    }
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook reservation mutex poisoned".to_string())?;
    if state.reservation_admission_sealed {
        return Err("terminal hook record reservation admission is sealed".into());
    }
    if state.reservation.is_some() {
        return Err("terminal hook already has an active record reservation".into());
    }
    require_no_retained_custody(&state)?;
    let records = state.records.iter().cloned().collect::<Vec<_>>();
    let diagnostics = diagnostic_snapshot();
    if records.is_empty() && diagnostics.iter().all(|count| *count == 0) {
        return Ok(None);
    }
    let reservation_id = next_terminal_reservation_id()?;
    let record_count = records.len();
    state.reservation = Some(ActiveTerminalReservation {
        reservation_id,
        record_count,
        diagnostics,
        sealed_terminal: false,
    });
    state.reserved_custody = Some(ReservedRecordCustody {
        reservation_id,
        identities: records.iter().map(reserved_record_identity).collect(),
        consumed: None,
    });
    Ok(Some(TerminalRecordReservation {
        reservation_id,
        drain: TerminalDrain {
            records,
            dropped_lock_busy: diagnostics[0],
            dropped_unbound: diagnostics[1],
            dropped_unreadable: diagnostics[2],
            dropped_overflow: diagnostics[3],
        },
    }))
}

pub fn declare_record_reservation_consumption(
    reservation_id: u64,
    consumed: &[usize],
) -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal record consumption declaration")?;
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook reservation consumption mutex poisoned".to_string())?;
    let active = match state.reservation {
        Some(active) if active.reservation_id == reservation_id => active,
        Some(active) => {
            return Err(format!(
                "terminal hook reservation consumption id mismatch ({reservation_id} != {})",
                active.reservation_id
            ));
        }
        None => {
            return Err(
                "terminal hook reservation consumption without an active reservation".into(),
            );
        }
    };
    if active.sealed_terminal {
        return Err("terminal hook sealed reservation has no publisher declaration".into());
    }
    let custody = match state.reserved_custody.as_mut() {
        Some(custody) if custody.reservation_id == reservation_id => custody,
        _ => return Err("terminal hook reservation consumption has no matching custody".into()),
    };
    if custody.consumed.is_some() {
        return Err("terminal hook reservation consumption is already declared".into());
    }
    if consumed.windows(2).any(|pair| pair[0] >= pair[1])
        || consumed
            .last()
            .is_some_and(|index| *index >= active.record_count)
    {
        return Err(format!(
            "terminal hook reservation consumption is not strictly increasing inside 0..{}",
            active.record_count
        ));
    }
    custody.consumed = Some(consumed.to_vec());
    Ok(())
}

pub fn commit_record_reservation(reservation_id: u64) -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal record commit")?;
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook reservation commit mutex poisoned".to_string())?;
    let active = match state.reservation {
        Some(active) if active.reservation_id == reservation_id => active,
        Some(active) => {
            return Err(format!(
                "terminal hook reservation commit id mismatch ({reservation_id} != {})",
                active.reservation_id
            ));
        }
        None => {
            return Err("terminal hook reservation commit without an active reservation".into())
        }
    };
    if active.sealed_terminal {
        return Err(
            "terminal hook sealed reservation requires commit_sealed_terminal_reservation".into(),
        );
    }
    validate_reserved_diagnostics(active, reservation_id)?;
    match state.reserved_custody.as_ref() {
        Some(custody) if custody.reservation_id != reservation_id => {
            return Err("terminal hook reservation commit custody id mismatch".into())
        }
        Some(custody) if custody.consumed.is_none() => {
            return Err("terminal hook reservation commit without a declared consumption".into())
        }
        Some(_) => {}
        None => return Err("terminal hook reservation commit has no record custody".into()),
    }
    let custody = state
        .reserved_custody
        .as_ref()
        .expect("record custody was just matched");
    if state.records.len() < custody.identities.len()
        || custody
            .identities
            .iter()
            .enumerate()
            .any(|(index, identity)| {
                state.records.get(index).map(reserved_record_identity) != Some(*identity)
            })
    {
        return Err("terminal hook reserved records moved before commit".into());
    }
    let consumed = state
        .reserved_custody
        .take()
        .and_then(|custody| custody.consumed)
        .expect("declared consumption was just matched");
    for index in consumed.iter().rev() {
        state.records.remove(*index);
    }
    subtract_reserved_diagnostics(active);
    state.reservation = None;
    Ok(())
}

pub fn rollback_record_reservation(reservation_id: u64) -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal record rollback")?;
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook reservation rollback mutex poisoned".to_string())?;
    match state.reservation {
        Some(active) if active.reservation_id == reservation_id => {
            if active.sealed_terminal {
                return Err(
                    "terminal hook sealed reservation requires rollback_sealed_terminal_reservation"
                        .into(),
                );
            }
            state.reservation = None;
            state.reserved_custody = None;
            Ok(())
        }
        Some(active) => Err(format!(
            "terminal hook reservation rollback id mismatch ({reservation_id} != {})",
            active.reservation_id
        )),
        None => Err("terminal hook reservation rollback without an active reservation".into()),
    }
}

/// Release a reservation whose frame was rejected by its own drop diagnostics,
/// is released, an append into a full queue may still evict the front record,
pub fn release_record_reservation_diagnostics(reservation_id: u64) -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal record diagnostic release")?;
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook diagnostic release mutex poisoned".to_string())?;
    let active = match state.reservation {
        Some(active) if active.reservation_id == reservation_id => active,
        Some(active) => {
            return Err(format!(
                "terminal hook diagnostic release id mismatch ({reservation_id} != {})",
                active.reservation_id
            ));
        }
        None => {
            return Err("terminal hook diagnostic release without an active reservation".into());
        }
    };
    if active.sealed_terminal {
        return Err(
            "terminal hook sealed reservation requires commit_sealed_terminal_reservation".into(),
        );
    }
    validate_reserved_diagnostics(active, reservation_id)?;
    subtract_reserved_diagnostics(active);
    state.reservation = None;
    state.reserved_custody = None;
    Ok(())
}

pub fn requeue(records: Vec<RawTerminalRecordCapture>) -> Result<(), String> {
    let _individual = try_admit_individual_api("terminal record requeue")?;
    if records.is_empty() {
        return Ok(());
    }
    let mut state = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .lock()
        .map_err(|_| "terminal hook requeue mutex poisoned".to_string())?;
    if state.reservation_admission_sealed {
        return Err("terminal hook requeue admission is sealed".into());
    }
    if state.reservation.is_some() {
        return Err("terminal hook requeue refused an active record reservation".into());
    }
    for record in records.into_iter().rev() {
        if state.records.len() == MAX_PENDING_RECORDS {
            state.records.pop_back();
            DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        }
        state.records.push_front(record);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalRecordUnpublishable {
    EndedPhaseUnreachable,
    AuthenticationFailed,
    LocationDisagreesWithEndedPhase,
}

impl TerminalRecordUnpublishable {
    pub const fn name(self) -> &'static str {
        match self {
            Self::EndedPhaseUnreachable => "ended_phase_unreachable",
            Self::AuthenticationFailed => "authentication_failed",
            Self::LocationDisagreesWithEndedPhase => "location_disagrees_with_ended_phase",
        }
    }

    pub const fn code(self) -> u64 {
        match self {
            Self::EndedPhaseUnreachable => 1,
            Self::AuthenticationFailed => 2,
            Self::LocationDisagreesWithEndedPhase => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalRecordVerdict {
    Publishable(crate::TerminalCombatRecordRow),
    EndedPhasePending,
    Unpublishable(TerminalRecordUnpublishable),
}

pub fn judge_terminal_record(
    capture: &RawTerminalRecordCapture,
    ended_location: Option<Option<u32>>,
    epoch_is_live: bool,
) -> TerminalRecordVerdict {
    let Some(expected_location) = ended_location else {
        return if epoch_is_live {
            TerminalRecordVerdict::EndedPhasePending
        } else {
            TerminalRecordVerdict::Unpublishable(TerminalRecordUnpublishable::EndedPhaseUnreachable)
        };
    };
    let Ok(row) = project_pointer_free_record(capture) else {
        return TerminalRecordVerdict::Unpublishable(
            TerminalRecordUnpublishable::AuthenticationFailed,
        );
    };
    if expected_location
        .map(|location| location == row.location)
        .unwrap_or(true)
    {
        TerminalRecordVerdict::Publishable(row)
    } else {
        TerminalRecordVerdict::Unpublishable(
            TerminalRecordUnpublishable::LocationDisagreesWithEndedPhase,
        )
    }
}

pub fn project_pointer_free_record(
    capture: &RawTerminalRecordCapture,
) -> Result<crate::TerminalCombatRecordRow, String> {
    let bytes = &capture.record_bytes;
    let base = MODULE_BASE.load(Ordering::Acquire);
    let record_vtable =
        u64::from_le_bytes(bytes[0..8].try_into().expect("fixed terminal vtable slice"));
    let end_date_vtable = u64::from_le_bytes(
        bytes[0x428..0x430]
            .try_into()
            .expect("fixed terminal end-date slice"),
    );
    let location_date_vtable = u64::from_le_bytes(
        bytes[0x438..0x440]
            .try_into()
            .expect("fixed terminal location-date slice"),
    );
    if base == 0
        || record_vtable != base.saturating_add(TERMINAL_RECORD_VTABLE_RVA) as u64
        || end_date_vtable != base.saturating_add(GAME_DATE_VTABLE_RVA) as u64
        || location_date_vtable != base.saturating_add(GAME_DATE_VTABLE_RVA) as u64
    {
        return Err(format!(
            "terminal record {} class/date vtable mismatch",
            capture.record_index
        ));
    }
    let row = crate::TerminalCombatRecordRow {
        combat_id: capture.combat_id,
        generation: capture.generation,
        record_index: capture.record_index,
        writer_kind: capture.writer_kind,
        attacker_manpower_loss_raw: i64::from_le_bytes(
            bytes[0x10..0x18].try_into().expect("fixed terminal slice"),
        ),
        defender_manpower_loss_raw: i64::from_le_bytes(
            bytes[0x220..0x228]
                .try_into()
                .expect("fixed terminal slice"),
        ),
        end_date_raw: u32::from_le_bytes(
            bytes[0x430..0x434]
                .try_into()
                .expect("fixed terminal slice"),
        ),
        location: u32::from_le_bytes(
            bytes[0x440..0x444]
                .try_into()
                .expect("fixed terminal slice"),
        ),
        winner_side_raw: bytes[0x444],
        winter_raw: bytes[0x445],
        result_selector_raw: bytes[0x446],
        layout_side_raw: bytes[0x447],
    };
    match (row.writer_kind, row.result_selector_raw) {
        (WRITER_KIND_OVERRUN, 0) | (WRITER_KIND_NORMAL, 1) => Ok(row),
        _ => Err(format!(
            "terminal record {} writer/result byte mismatch ({}/{})",
            row.record_index, row.writer_kind, row.result_selector_raw
        )),
    }
}

unsafe extern "system" fn hook_normal_writer(
    param_1: usize,
    combat_ptr: usize,
    param_3: usize,
    param_4: usize,
    param_5: usize,
) {
    let _active = ActiveWriterCallback::enter();
    let original = ORIGINAL_NORMAL.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    let function: NormalWriterFn = unsafe { mem::transmute(original) };
    let Some(_admitted) = AdmittedWriterCallback::try_enter() else {
        WRITER_CONTEXT.with(|slot| {
            let previous = slot.replace(None);
            unsafe { function(param_1, combat_ptr, param_3, param_4, param_5) };
            slot.set(previous);
        });
        return;
    };
    WRITER_CONTEXT.with(|slot| {
        let previous = slot.replace(Some(WriterContext {
            combat_ptr,
            writer_kind: WRITER_KIND_NORMAL,
        }));
        unsafe { function(param_1, combat_ptr, param_3, param_4, param_5) };
        slot.set(previous);
    });
}

unsafe extern "system" fn hook_overrun_writer(param_1: usize, param_2: usize, combat_ptr: usize) {
    let _active = ActiveWriterCallback::enter();
    let original = ORIGINAL_OVERRUN.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    let function: OverrunWriterFn = unsafe { mem::transmute(original) };
    let Some(_admitted) = AdmittedWriterCallback::try_enter() else {
        WRITER_CONTEXT.with(|slot| {
            let previous = slot.replace(None);
            unsafe { function(param_1, param_2, combat_ptr) };
            slot.set(previous);
        });
        return;
    };
    WRITER_CONTEXT.with(|slot| {
        let previous = slot.replace(Some(WriterContext {
            combat_ptr,
            writer_kind: WRITER_KIND_OVERRUN,
        }));
        unsafe { function(param_1, param_2, combat_ptr) };
        slot.set(previous);
    });
}

unsafe extern "system" fn hook_record_allocator(record_ptr: usize) -> u64 {
    let _active = ActiveAllocatorCallback::enter();
    let original = ORIGINAL_ALLOCATOR.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    let context = WRITER_CONTEXT.with(|slot| slot.get());
    let _admitted = context.map(|_| AdmittedAllocatorCallback::enter_nested());
    let function: RecordAllocatorFn = unsafe { mem::transmute(original) };
    let record_index = unsafe { function(record_ptr) };
    if let Some(context) = context {
        capture_record(context, record_index, record_ptr);
    }
    record_index
}

fn capture_record(context: WriterContext, record_index: u64, record_ptr: usize) {
    let Some(identity_addr) = context.combat_ptr.checked_add(0x08) else {
        DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut identity = [0u8; 8];
    let mut identity_read = 0usize;
    let identity_ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            identity_addr as *const c_void,
            identity.as_mut_ptr().cast(),
            identity.len(),
            &mut identity_read,
        )
    } != 0;
    if !identity_ok
        || identity_read != identity.len()
        || u32::from_le_bytes(identity[0..4].try_into().expect("fixed combat type slice"))
            != LAND_COMBAT_TYPE
    {
        DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let live_combat_id =
        u32::from_le_bytes(identity[4..8].try_into().expect("fixed combat id slice"));
    let Ok(mut state) = STATE
        .get_or_init(|| Mutex::new(HookState::default()))
        .try_lock()
    else {
        DROPPED_LOCK_BUSY.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if state.reservation_admission_sealed {
        return;
    }
    let Some(&(combat_id, generation)) = state.bindings.get(&context.combat_ptr) else {
        DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if combat_id != live_combat_id {
        DROPPED_UNBOUND.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let Ok(record_index) = u32::try_from(record_index) else {
        DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut record_bytes = [0u8; TERMINAL_RECORD_LEN];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            record_ptr as *const c_void,
            record_bytes.as_mut_ptr().cast(),
            record_bytes.len(),
            &mut read,
        )
    } != 0;
    if !ok || read != record_bytes.len() {
        DROPPED_UNREADABLE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if state.records.len() == MAX_PENDING_RECORDS {
        if state.reservation.is_some() {
            DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
            return;
        }
        state.records.pop_front();
        DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
    }
    state.records.push_back(RawTerminalRecordCapture {
        combat_id,
        generation,
        record_index,
        writer_kind: context.writer_kind,
        record_bytes,
    });
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let actual = read_exact_bytes(address, expected.len())?;
    if actual != expected {
        return Err(format!("a729 {label} sentinel mismatch at 0x{address:X}"));
    }
    Ok(())
}

fn read_exact_bytes(address: usize, len: usize) -> Result<Vec<u8>, String> {
    if address == 0 || len == 0 {
        return Err("terminal hook memory observation range is empty".into());
    }
    let mut actual = vec![0u8; len];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            actual.as_mut_ptr().cast(),
            actual.len(),
            &mut read,
        )
    } != 0;
    if !ok || read != len {
        return Err(format!(
            "terminal hook memory observation failed at 0x{address:X}"
        ));
    }
    Ok(actual)
}

// Process-lifetime detour region retention
//   retained. They record their relay and detour ranges here, they release
//   and the terminal release both call `MinHook::remove_hook`, and
// quiescence path and a `CREATED_HOOK_MASK` that the release gate requires to

#[repr(C)]
struct RuntimeFunctionEntry {
    begin_address: u32,
    end_address: u32,
    unwind_data: u32,
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlLookupFunctionEntry(
        control_pc: u64,
        image_base: *mut u64,
        history_table: *mut c_void,
    ) -> *const RuntimeFunctionEntry;
}

/// One retained code region, recorded at teardown and never released.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RetainedDetourRegion {
    pub(crate) label: String,
    /// Half-open `[start, end)` runtime addresses retained for process lifetime.
    pub(crate) regions: Vec<(usize, usize)>,
    pub(crate) callbacks_closed_to_new_entries: bool,
    pub(crate) retained_for_process_lifetime: bool,
    pub(crate) teardown_owner_token: Option<u64>,
}

static RETAINED_DETOUR_REGIONS: OnceLock<Mutex<Vec<RetainedDetourRegion>>> = OnceLock::new();

fn retained_detour_regions_slot() -> &'static Mutex<Vec<RetainedDetourRegion>> {
    RETAINED_DETOUR_REGIONS.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn retained_detour_regions() -> Result<Vec<RetainedDetourRegion>, String> {
    Ok(retained_detour_regions_slot()
        .lock()
        .map_err(|_| "retained detour region registry mutex poisoned".to_string())?
        .clone())
}

/// Refuse to attest process-lifetime retention over a region the census does
pub(crate) fn require_regions_recorded_retained(
    label: &str,
    teardown_owner_token: u64,
    regions: &[(usize, usize)],
) -> Result<(), String> {
    if regions.is_empty() {
        return Err(format!(
            "{label} attested process-lifetime retention over no region at all"
        ));
    }
    let census = retained_detour_regions()?;
    for (start, end) in regions {
        if *start == 0 || start >= end {
            return Err(format!(
                "{label} attested retention over 0x{start:X}..0x{end:X}, which is not a real range"
            ));
        }
        let recorded = census.iter().any(|entry| {
            entry.retained_for_process_lifetime
                && entry.teardown_owner_token == Some(teardown_owner_token)
                && entry.regions.iter().any(|(recorded_start, recorded_end)| {
                    recorded_start == start && recorded_end == end
                })
        });
        if !recorded {
            return Err(format!(
                "{label} would attest process-lifetime retention for 0x{start:X}..0x{end:X} \
                 under teardown owner {teardown_owner_token}, which the retention census does \
                 not record"
            ));
        }
    }
    Ok(())
}

/// could be executing our detour code". Under process-lifetime retention that
pub(crate) fn exact_runtime_function_range(
    function: usize,
    label: &str,
) -> Result<(usize, usize), String> {
    if function == 0 {
        return Err(format!("{label} runtime function address is null"));
    }
    let mut image_base = 0u64;
    let entry =
        unsafe { RtlLookupFunctionEntry(function as u64, &mut image_base, ptr::null_mut()) };
    if entry.is_null() || image_base == 0 {
        return Err(format!(
            "{label} has no PE64 runtime function entry at 0x{function:X}"
        ));
    }
    let entry = unsafe { &*entry };
    let base = usize::try_from(image_base)
        .map_err(|_| format!("{label} runtime function image base does not fit usize"))?;
    let start = base
        .checked_add(entry.begin_address as usize)
        .ok_or_else(|| format!("{label} runtime function start overflowed"))?;
    let end = base
        .checked_add(entry.end_address as usize)
        .ok_or_else(|| format!("{label} runtime function end overflowed"))?;
    if start >= end || function < start || function >= end {
        return Err(format!(
            "{label} runtime function range 0x{start:X}..0x{end:X} does not contain 0x{function:X}"
        ));
    }
    Ok((start, end))
}

pub(crate) fn close_callbacks_and_retain_regions(
    label: &str,
    teardown_owner_token: Option<u64>,
    counters: &[&AtomicUsize],
    regions: &[(usize, usize)],
) -> Result<RetainedDetourRegion, String> {
    if counters.is_empty() {
        return Err(format!("{label} supplied no callback counter to close"));
    }
    if regions.is_empty() {
        return Err(format!("{label} supplied no region to retain"));
    }
    for (start, end) in regions {
        if *start == 0 || start >= end {
            return Err(format!(
                "{label} retained region 0x{start:X}..0x{end:X} is not a real range"
            ));
        }
    }
    for counter in counters {
        let observed = counter.load(Ordering::Acquire);
        if observed != 0 {
            return Err(format!(
                "{label} still admits callbacks: {observed} active at retention"
            ));
        }
    }
    let retained = RetainedDetourRegion {
        label: label.to_string(),
        regions: regions.to_vec(),
        callbacks_closed_to_new_entries: true,
        retained_for_process_lifetime: true,
        teardown_owner_token,
    };
    retained_detour_regions_slot()
        .lock()
        .map_err(|_| format!("{label} retained detour region registry mutex poisoned"))?
        .push(retained.clone());
    Ok(retained)
}

#[cfg(test)]
static TERMINAL_HOOK_STATIC_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod terminal_diagnostic_release_tests {
    use super::*;

    /// FALSIFIABILITY: make `release_record_reservation_diagnostics` pop the
    #[test]
    fn releasing_poisoned_diagnostics_keeps_the_unprojected_record() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut state = STATE
            .get_or_init(|| Mutex::new(HookState::default()))
            .lock()
            .expect("terminal state");
        state.records.clear();
        state.reservation = None;
        state.reservation_admission_sealed = false;
        state.records.push_back(RawTerminalRecordCapture {
            combat_id: 7,
            generation: 3,
            record_index: 0,
            writer_kind: 0,
            record_bytes: [0u8; TERMINAL_RECORD_LEN],
        });
        DROPPED_UNREADABLE.store(1, Ordering::Release);
        state.reservation = Some(ActiveTerminalReservation {
            reservation_id: 4_242,
            record_count: 1,
            diagnostics: [0, 0, 1, 0],
            sealed_terminal: false,
        });
        drop(state);

        release_record_reservation_diagnostics(4_242).expect("diagnostic release");

        assert_eq!(
            DROPPED_UNREADABLE.load(Ordering::Acquire),
            0,
            "the drop snapshot must be consumed or every later frame re-reports it"
        );
        let mut state = STATE.get().expect("terminal state").lock().expect("lock");
        assert_eq!(
            state.records.len(),
            1,
            "the record was never projected, so it must still be offered next frame"
        );
        assert_eq!(state.records[0].combat_id, 7);
        assert!(state.reservation.is_none(), "custody must be released");
        state.records.clear();
        DROPPED_UNREADABLE.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod terminal_record_verdict_tests {
    use super::*;

    const TEST_BASE: usize = 0x1_0000_0000;

    fn authentic_capture(
        combat_id: u32,
        generation: u64,
        record_index: u32,
        location: u32,
    ) -> RawTerminalRecordCapture {
        let mut record_bytes = [0u8; TERMINAL_RECORD_LEN];
        record_bytes[0..8].copy_from_slice(
            &(TEST_BASE.saturating_add(TERMINAL_RECORD_VTABLE_RVA) as u64).to_le_bytes(),
        );
        let date_vtable = (TEST_BASE.saturating_add(GAME_DATE_VTABLE_RVA) as u64).to_le_bytes();
        record_bytes[0x428..0x430].copy_from_slice(&date_vtable);
        record_bytes[0x438..0x440].copy_from_slice(&date_vtable);
        record_bytes[0x440..0x444].copy_from_slice(&location.to_le_bytes());
        record_bytes[0x446] = 1;
        RawTerminalRecordCapture {
            combat_id,
            generation,
            record_index,
            writer_kind: WRITER_KIND_NORMAL,
            record_bytes,
        }
    }

    struct BaseGuard;

    impl Drop for BaseGuard {
        fn drop(&mut self) {
            MODULE_BASE.store(0, Ordering::Release);
        }
    }

    fn publish_test_module_base() -> BaseGuard {
        MODULE_BASE.store(TEST_BASE, Ordering::Release);
        BaseGuard
    }

    #[test]
    fn a_live_epoch_is_pending_and_an_absent_one_is_proven_unreachable() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _base = publish_test_module_base();
        let capture = authentic_capture(11, 4, 0, 77);

        assert_eq!(
            judge_terminal_record(&capture, None, true),
            TerminalRecordVerdict::EndedPhasePending,
            "an epoch the roster still lists has its one publishable frame ahead of it"
        );
        assert_eq!(
            judge_terminal_record(&capture, None, false),
            TerminalRecordVerdict::Unpublishable(
                TerminalRecordUnpublishable::EndedPhaseUnreachable
            ),
            "an epoch in neither set has already spent its one publishable frame"
        );
    }

    #[test]
    fn a_capture_that_fails_authentication_is_decided_not_deferred() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _base = publish_test_module_base();
        let mut capture = authentic_capture(11, 4, 0, 77);
        capture.record_bytes[0] ^= 0xFF;

        assert_eq!(
            judge_terminal_record(&capture, Some(None), false),
            TerminalRecordVerdict::Unpublishable(TerminalRecordUnpublishable::AuthenticationFailed),
        );

        let mut capture = authentic_capture(11, 4, 0, 77);
        capture.record_bytes[0x446] = 0;
        assert_eq!(
            judge_terminal_record(&capture, Some(None), false),
            TerminalRecordVerdict::Unpublishable(TerminalRecordUnpublishable::AuthenticationFailed),
        );
    }

    #[test]
    fn a_location_disagreement_with_the_only_ended_row_is_final() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _base = publish_test_module_base();
        let capture = authentic_capture(11, 4, 0, 77);

        assert_eq!(
            judge_terminal_record(&capture, Some(Some(77)), false),
            TerminalRecordVerdict::Publishable(
                project_pointer_free_record(&capture).expect("authentic capture projects")
            ),
        );
        assert_eq!(
            judge_terminal_record(&capture, Some(Some(78)), false),
            TerminalRecordVerdict::Unpublishable(
                TerminalRecordUnpublishable::LocationDisagreesWithEndedPhase
            ),
        );
        assert!(matches!(
            judge_terminal_record(&capture, Some(None), false),
            TerminalRecordVerdict::Publishable(_)
        ));
    }
}

#[cfg(test)]
mod terminal_reservation_consumption_tests {
    use super::*;

    struct InstalledGuard;

    impl Drop for InstalledGuard {
        fn drop(&mut self) {
            INSTALLED.store(false, Ordering::Release);
            if let Some(state) = STATE.get() {
                let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
                state.records.clear();
                state.reservation = None;
                state.reservation_admission_sealed = true;
            }
            for counter in diagnostic_counters() {
                counter.store(0, Ordering::Release);
            }
        }
    }

    fn queue(captures: &[(u32, u64)]) -> InstalledGuard {
        let guard = InstalledGuard;
        {
            let mut state = STATE
                .get_or_init(|| Mutex::new(HookState::default()))
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            state.records.clear();
            state.reservation = None;
            state.reservation_admission_sealed = false;
            for (combat_id, generation) in captures {
                state.records.push_back(RawTerminalRecordCapture {
                    combat_id: *combat_id,
                    generation: *generation,
                    record_index: 0,
                    writer_kind: WRITER_KIND_NORMAL,
                    record_bytes: [0u8; TERMINAL_RECORD_LEN],
                });
            }
        }
        for counter in diagnostic_counters() {
            counter.store(0, Ordering::Release);
        }
        INSTALLED.store(true, Ordering::Release);
        guard
    }

    fn queued_epochs() -> Vec<(u32, u64)> {
        STATE
            .get()
            .expect("terminal state")
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .records
            .iter()
            .map(|record| (record.combat_id, record.generation))
            .collect()
    }

    #[test]
    fn the_whole_queue_is_offered_so_a_decided_front_cannot_strand_the_rest() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _installed = queue(&[(1, 1), (2, 1), (3, 1)]);

        let reservation = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        assert_eq!(
            reservation
                .drain
                .records
                .iter()
                .map(|record| (record.combat_id, record.generation))
                .collect::<Vec<_>>(),
            vec![(1, 1), (2, 1), (3, 1)],
            "every queued record must be offered, in order"
        );
        assert_eq!(
            queued_epochs(),
            vec![(1, 1), (2, 1), (3, 1)],
            "reserving must not remove or reorder anything"
        );

        declare_record_reservation_consumption(reservation.reservation_id, &[0, 1])
            .expect("declared consumption");
        commit_record_reservation(reservation.reservation_id).expect("commit");
        assert_eq!(
            queued_epochs(),
            vec![(3, 1)],
            "exactly the decided records are removed and the rest keeps its order"
        );
    }

    #[test]
    fn a_record_that_cannot_be_decided_does_not_block_the_one_behind_it() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _installed = queue(&[(20, 1), (21, 1), (22, 1)]);

        let reservation = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        declare_record_reservation_consumption(reservation.reservation_id, &[1])
            .expect("declared consumption");
        commit_record_reservation(reservation.reservation_id).expect("commit");
        assert_eq!(
            queued_epochs(),
            vec![(20, 1), (22, 1)],
            "only the decided record leaves, and the undecided ones keep their order"
        );
    }

    #[test]
    fn an_undeclared_commit_is_refused_and_changes_nothing() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _installed = queue(&[(30, 1), (31, 1)]);

        let reservation = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        assert!(
            commit_record_reservation(reservation.reservation_id).is_err(),
            "an undeclared commit must fail closed"
        );
        assert_eq!(
            queued_epochs(),
            vec![(30, 1), (31, 1)],
            "a refused commit must not remove anything"
        );
        rollback_record_reservation(reservation.reservation_id).expect("rollback");
    }

    /// `release_record_reservation_diagnostics` and the retry's declaration is
    #[test]
    fn a_declaration_does_not_survive_its_reservation() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _installed = queue(&[(40, 1), (41, 1)]);

        let first = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        declare_record_reservation_consumption(first.reservation_id, &[0]).expect("declare");
        rollback_record_reservation(first.reservation_id).expect("rollback");

        let second = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        declare_record_reservation_consumption(second.reservation_id, &[1])
            .expect("a rolled-back declaration must not block the retry");
        commit_record_reservation(second.reservation_id).expect("commit");
        assert_eq!(queued_epochs(), vec![(40, 1)]);

        // The diagnostic release is the other non-committing disposition and
        DROPPED_UNREADABLE.store(1, Ordering::Release);
        let third = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        declare_record_reservation_consumption(third.reservation_id, &[0]).expect("declare");
        release_record_reservation_diagnostics(third.reservation_id).expect("release");
        let fourth = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        declare_record_reservation_consumption(fourth.reservation_id, &[0])
            .expect("a released declaration must not block the retry");
        rollback_record_reservation(fourth.reservation_id).expect("rollback");
    }

    /// The dispositions own the declaration's lifetime. One that outlived its
    /// reservation means a disposition failed to release its own state, and
    #[test]
    fn reserving_refuses_a_declaration_that_outlived_its_reservation() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _installed = queue(&[(50, 1)]);
        {
            let mut state = STATE
                .get()
                .expect("terminal state")
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            state.reserved_custody = Some(ReservedRecordCustody {
                reservation_id: 9_999,
                identities: vec![(50, 1, 0, WRITER_KIND_NORMAL)],
                consumed: Some(vec![0]),
            });
        }

        assert!(
            reserve_pending_records().is_err(),
            "a declaration with no reservation is a released-state bug and must fail closed"
        );

        let mut state = STATE
            .get()
            .expect("terminal state")
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        assert!(
            state.reservation.is_none(),
            "the refused reservation must not have taken custody"
        );

        state.reserved_custody = Some(ReservedRecordCustody {
            reservation_id: 9_999,
            identities: Vec::new(),
            consumed: None,
        });
        assert!(
            reserve_full_terminal_drain_locked(&mut state).is_err(),
            "the sealed constructor must refuse retained custody too"
        );
        assert!(
            state.reservation.is_none(),
            "the refused sealed reservation must not have taken custody"
        );
        state.reserved_custody = None;
    }

    /// check has to be an identity check rather than a bounds check -- a bounds
    #[test]
    fn a_commit_refuses_when_the_reserved_records_moved_underneath_it() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _installed = queue(&[(60, 1), (61, 1), (62, 1)]);

        let reservation = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        declare_record_reservation_consumption(reservation.reservation_id, &[1])
            .expect("declare the middle record");
        {
            let mut state = STATE
                .get()
                .expect("terminal state")
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            state.records.pop_front();
            state.records.push_back(RawTerminalRecordCapture {
                combat_id: 63,
                generation: 1,
                record_index: 0,
                writer_kind: WRITER_KIND_NORMAL,
                record_bytes: [0u8; TERMINAL_RECORD_LEN],
            });
        }

        assert!(
            commit_record_reservation(reservation.reservation_id).is_err(),
            "a commit whose reserved records moved must refuse rather than remove by index"
        );
        assert_eq!(
            queued_epochs(),
            vec![(61, 1), (62, 1), (63, 1)],
            "a refused commit must remove nothing"
        );
        rollback_record_reservation(reservation.reservation_id).expect("rollback");
    }

    #[test]
    fn the_declaration_is_bounded_single_shot_and_exactly_what_the_commit_removes() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _installed = queue(&[(4, 2), (5, 2), (6, 2)]);

        let reservation = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        assert!(
            declare_record_reservation_consumption(reservation.reservation_id, &[0, 3]).is_err(),
            "a publisher must not name a record outside its own reservation"
        );
        assert!(
            declare_record_reservation_consumption(reservation.reservation_id, &[1, 0]).is_err(),
            "the declaration must be canonical so the commit's removal order is defined"
        );
        declare_record_reservation_consumption(reservation.reservation_id, &[0]).expect("declare");
        assert!(
            declare_record_reservation_consumption(reservation.reservation_id, &[0, 1]).is_err(),
            "a publisher must not be able to widen its claim by redeclaring"
        );
        commit_record_reservation(reservation.reservation_id).expect("commit");
        assert_eq!(
            queued_epochs(),
            vec![(5, 2), (6, 2)],
            "the commit must honour the declaration, not the reserved count"
        );
    }

    #[test]
    fn a_rollback_returns_every_reserved_record_in_order() {
        let _serial = TERMINAL_HOOK_STATIC_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _installed = queue(&[(7, 3), (8, 3)]);

        let reservation = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        rollback_record_reservation(reservation.reservation_id).expect("rollback");
        assert_eq!(queued_epochs(), vec![(7, 3), (8, 3)]);

        let again = reserve_pending_records()
            .expect("reservation")
            .expect("records are pending");
        assert_eq!(
            again
                .drain
                .records
                .iter()
                .map(|record| (record.combat_id, record.generation))
                .collect::<Vec<_>>(),
            vec![(7, 3), (8, 3)],
        );
        rollback_record_reservation(again.reservation_id).expect("rollback");
    }
}

#[cfg(test)]
mod minhook_relay_layout_tests {
    use super::MINHOOK_RELAY_LEN;

    #[test]
    fn relay_len_covers_exactly_the_ff25_prefix_and_its_absolute_destination() {
        const FF25_RIP_INDIRECT_PREFIX_LEN: usize = 6;
        assert_eq!(
            MINHOOK_RELAY_LEN,
            FF25_RIP_INDIRECT_PREFIX_LEN + core::mem::size_of::<u64>()
        );
    }
}

#[cfg(test)]
mod retained_detour_region_tests {
    use super::{
        close_callbacks_and_retain_regions, exact_runtime_function_range,
        require_regions_recorded_retained, retained_detour_regions, retained_detour_regions_slot,
        RetainedDetourRegion,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[inline(never)]
    fn sample_non_leaf_function_in_this_image(seed: usize) -> usize {
        let mut buffer = [0usize; 8];
        for (index, slot) in buffer.iter_mut().enumerate() {
            *slot = seed.wrapping_add(index);
        }
        buffer.iter().copied().fold(0usize, usize::wrapping_add)
            + std::hint::black_box(buffer).len()
    }

    #[test]
    fn runtime_function_range_is_absolute_and_contains_the_queried_address() {
        let function = sample_non_leaf_function_in_this_image as usize;
        let (start, end) = exact_runtime_function_range(function, "sample")
            .expect("a non-leaf function in this DLL has a PE64 unwind entry");
        assert!(start < end, "range must be non-empty: {start:X}..{end:X}");
        assert!(
            (start..end).contains(&function),
            "0x{function:X} must lie inside 0x{start:X}..0x{end:X}"
        );
        assert!(start > 0x1_0000, "range must be absolute, got 0x{start:X}");
    }

    #[test]
    fn retention_is_refused_while_callbacks_are_still_admitted() {
        static BUSY: AtomicUsize = AtomicUsize::new(0);
        BUSY.store(3, Ordering::Release);
        let error = close_callbacks_and_retain_regions(
            "test busy roster",
            Some(1),
            &[&BUSY],
            &[(0x1000, 0x1010)],
        )
        .expect_err("a roster with active callbacks must not be recorded as retained");
        assert!(error.contains("still admits callbacks"), "{error}");
        assert!(error.contains('3'), "{error}");
    }

    #[test]
    fn closed_roster_is_recorded_as_retained_for_process_lifetime() {
        static IDLE: AtomicUsize = AtomicUsize::new(0);
        let regions = [(0x2000usize, 0x2040usize)];
        let receipt =
            close_callbacks_and_retain_regions("test idle roster", Some(7), &[&IDLE], &regions)
                .unwrap();
        assert_eq!(receipt.teardown_owner_token, Some(7));
        assert!(receipt.callbacks_closed_to_new_entries);
        assert!(receipt.retained_for_process_lifetime);
        assert_eq!(receipt.regions, regions.to_vec());
        assert!(
            retained_detour_regions()
                .unwrap()
                .iter()
                .any(|entry| entry.label == "test idle roster"
                    && entry.retained_for_process_lifetime),
            "retention must be recorded in the append-only registry"
        );
    }

    /// `retained_for_process_lifetime`) instead of on the exact recorded range.
    #[test]
    fn attesting_retention_requires_the_census_to_record_that_exact_region() {
        static IDLE: AtomicUsize = AtomicUsize::new(0);
        let recorded = (0x5100_0000usize, 0x5100_0040usize);
        let never_recorded = (0x5200_0000usize, 0x5200_0040usize);
        const OWNER: u64 = 0x5100;

        require_regions_recorded_retained("census probe", OWNER, &[recorded])
            .expect_err("an unrecorded region must not be attestable as retained");

        close_callbacks_and_retain_regions(
            "census probe roster",
            Some(OWNER),
            &[&IDLE],
            &[recorded],
        )
        .unwrap();

        require_regions_recorded_retained("census probe", OWNER, &[recorded])
            .expect("the census now records this exact region");

        let error = require_regions_recorded_retained("census probe", OWNER + 1, &[recorded])
            .expect_err("another lifecycle's retention must not answer for this one");
        assert!(
            error.contains("the retention census does not record"),
            "{error}"
        );
        assert!(error.contains(&(OWNER + 1).to_string()), "{error}");

        let error = require_regions_recorded_retained("census probe", OWNER, &[never_recorded])
            .expect_err("a region the census never saw must still be refused");
        assert!(
            error.contains("the retention census does not record"),
            "{error}"
        );

        let overlapping = (recorded.0, recorded.1 + 0x10);
        require_regions_recorded_retained("census probe", OWNER, &[overlapping])
            .expect_err("only the exact recorded range may be attested");

        require_regions_recorded_retained("census probe", OWNER, &[])
            .expect_err("attesting retention over nothing must be refused");

        let recorded_but_not_retained = (0x5400_0000usize, 0x5400_0040usize);
        retained_detour_regions_slot()
            .lock()
            .expect("retention census is not poisoned")
            .push(RetainedDetourRegion {
                label: "census probe without retention".to_string(),
                regions: vec![recorded_but_not_retained],
                callbacks_closed_to_new_entries: true,
                retained_for_process_lifetime: false,
                teardown_owner_token: Some(OWNER),
            });
        let error =
            require_regions_recorded_retained("census probe", OWNER, &[recorded_but_not_retained])
                .expect_err(
                    "a census entry that claims no retention cannot back a retained receipt",
                );
        assert!(
            error.contains("the retention census does not record"),
            "{error}"
        );
    }
}
