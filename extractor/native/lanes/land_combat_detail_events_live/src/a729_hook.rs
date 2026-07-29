//! Exact-build terminal land-combat writer capture for hoi4.exe 1.19.2.0.a729.
//!
//! The two writer detours establish the owning active-combat pointer. Their
//! only shared record allocator is detoured as a nested observer, after the
//! engine has copied the complete 0x448-byte record into its global table.
//! Raw bytes never cross the replay wire: the collector projects only proven
//! pointer-free scalar fields from this process-local carrier.

#![cfg(windows)]

use crate::terminal_coordinator::{
    individual_terminal_api_blocked, release_admitted_callback, try_admit_callback,
    try_admit_individual_api,
};
use core::cell::Cell;
use core::ffi::c_void;
use core::mem;
use core::ptr;
use minhook::{MinHook, MH_STATUS};
use production_hook_receipt::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released,
    observe_released_set, ExpectedRvaHook, HookTeardownAttestation, InstalledRvaHookReceipt,
    ReleasedRvaHookReceipt, MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
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
pub const WRITER_KIND_OVERRUN: u32 = 1;
pub const WRITER_KIND_NORMAL: u32 = 2;

/// Runtime production admission requires exact installed receipts for all
/// three MinHook sites in addition to their entry-byte sentinels. Teardown
/// likewise remains incomplete until all three original prefixes have exact
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
const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_EXECUTE_READ: u32 = 0x20;
const SHIM_ALLOCATION_LEN: usize = 0x2000;
const SHIM_DATA_OFFSET: usize = 0x1000;
const SHIM_GATEWAY_OFFSET: usize = 0x80;
const SHIM_ACTIVE_OFFSET: usize = SHIM_DATA_OFFSET;
const SHIM_DISPATCH_OFFSET: usize = SHIM_DATA_OFFSET + 8;
const SHIM_TRAMPOLINE_OFFSET: usize = SHIM_DATA_OFFSET + 16;

#[link(name = "kernel32")]
extern "system" {
    fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        allocation_type: u32,
        protect: u32,
    ) -> *mut c_void;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: u32,
        old_protect: *mut u32,
    ) -> i32;
}

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

/// Permanently allocated entry-safe detour owner. The executable shim and
/// MinHook trampoline are intentionally retained until process exit. Teardown
/// atomically switches `dispatch` to the standalone gateway before target
/// restoration; a callback parked before the first shim instruction can then
/// only take the external trampoline path and can never re-enter this DLL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProcessLifetimeDispatchShim {
    allocation: usize,
    entry: usize,
    gateway: usize,
    active: usize,
    dispatch: usize,
    trampoline: usize,
    code_len: usize,
}

unsafe impl Send for ProcessLifetimeDetourShim {}
unsafe impl Sync for ProcessLifetimeDetourShim {}

impl ProcessLifetimeDispatchShim {
    pub(crate) fn allocate(
        callback: usize,
        integer_arg_count: u8,
        label: &str,
    ) -> Result<Self, String> {
        if callback == 0 || !(1..=8).contains(&integer_arg_count) {
            return Err(format!(
                "{label} process-lifetime callback/argument count is invalid"
            ));
        }
        let allocation = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                SHIM_ALLOCATION_LEN,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
        } as usize;
        if allocation == 0 {
            return Err(format!("{label} process-lifetime shim allocation failed"));
        }
        let entry = allocation;
        let gateway = allocation
            .checked_add(SHIM_GATEWAY_OFFSET)
            .ok_or_else(|| format!("{label} gateway address overflowed"))?;
        let active = allocation
            .checked_add(SHIM_ACTIVE_OFFSET)
            .ok_or_else(|| format!("{label} active-counter address overflowed"))?;
        let dispatch = allocation
            .checked_add(SHIM_DISPATCH_OFFSET)
            .ok_or_else(|| format!("{label} dispatch address overflowed"))?;
        let trampoline = allocation
            .checked_add(SHIM_TRAMPOLINE_OFFSET)
            .ok_or_else(|| format!("{label} trampoline-slot address overflowed"))?;
        let code = build_process_lifetime_shim(entry, active, dispatch, integer_arg_count)?;
        let gateway_code = build_absolute_slot_jump(gateway, trampoline)?;
        unsafe {
            ptr::copy_nonoverlapping(code.as_ptr(), entry as *mut u8, code.len());
            ptr::copy_nonoverlapping(
                gateway_code.as_ptr(),
                gateway as *mut u8,
                gateway_code.len(),
            );
            (active as *mut AtomicU64).write(AtomicU64::new(0));
            (dispatch as *mut AtomicUsize).write(AtomicUsize::new(callback));
            (trampoline as *mut AtomicUsize).write(AtomicUsize::new(0));
        }
        let mut old_protect = 0u32;
        if unsafe {
            VirtualProtect(
                allocation as *mut c_void,
                SHIM_DATA_OFFSET,
                PAGE_EXECUTE_READ,
                &mut old_protect,
            )
        } == 0
        {
            return Err(format!("{label} process-lifetime code protection failed"));
        }
        if unsafe {
            FlushInstructionCache(
                GetCurrentProcess(),
                allocation as *const c_void,
                SHIM_DATA_OFFSET,
            )
        } == 0
        {
            return Err(format!(
                "{label} process-lifetime instruction-cache flush failed"
            ));
        }
        Ok(Self {
            allocation,
            entry,
            gateway,
            active,
            dispatch,
            trampoline,
            code_len: code.len().max(SHIM_GATEWAY_OFFSET + gateway_code.len()),
        })
    }

    pub(crate) fn entry(self) -> usize {
        self.entry
    }

    pub(crate) fn gateway(self) -> usize {
        self.gateway
    }

    pub(crate) fn allocation(self) -> usize {
        self.allocation
    }

    pub(crate) fn code_len(self) -> usize {
        self.code_len
    }

    pub(crate) fn active_counter_address(self) -> usize {
        self.active
    }

    pub(crate) fn dispatch_slot_address(self) -> usize {
        self.dispatch
    }

    pub(crate) fn trampoline_slot_address(self) -> usize {
        self.trampoline
    }

    pub(crate) fn bind_trampoline(self, trampoline: usize, label: &str) -> Result<(), String> {
        if trampoline == 0 {
            return Err(format!("{label} process-lifetime trampoline is null"));
        }
        let slot = unsafe { &*(self.trampoline as *const AtomicUsize) };
        match slot.compare_exchange(0, trampoline, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => Ok(()),
            Err(existing) if existing == trampoline => Ok(()),
            Err(_) => Err(format!(
                "{label} process-lifetime trampoline changed after binding"
            )),
        }
    }

    pub(crate) fn activate(self, callback: usize, label: &str) -> Result<(), String> {
        if callback == 0
            || unsafe { &*(self.trampoline as *const AtomicUsize) }.load(Ordering::Acquire) == 0
        {
            return Err(format!("{label} process-lifetime shim is not fully bound"));
        }
        if unsafe { &*(self.active as *const AtomicU64) }.load(Ordering::Acquire) != 0 {
            return Err(format!(
                "{label} process-lifetime shim is active during activation"
            ));
        }
        unsafe { &*(self.dispatch as *const AtomicUsize) }.store(callback, Ordering::Release);
        Ok(())
    }

    pub(crate) fn switch_to_passthrough(self) {
        unsafe { &*(self.dispatch as *const AtomicUsize) }.store(self.gateway, Ordering::Release);
    }

    pub(crate) fn wait_for_dll_dispatch_quiescence(self, label: &str) -> Result<(), String> {
        let mut yields = 0u32;
        while unsafe { &*(self.active as *const AtomicU64) }.load(Ordering::Acquire) != 0 {
            if yields == 1_000_000 {
                return Err(format!(
                    "{label} process-lifetime DLL dispatches did not quiesce"
                ));
            }
            yields += 1;
            std::thread::yield_now();
        }
        Ok(())
    }
}

fn rel32(from_after: usize, to: usize, label: &str) -> Result<[u8; 4], String> {
    let displacement = (to as i128) - (from_after as i128);
    let displacement = i32::try_from(displacement)
        .map_err(|_| format!("{label} RIP-relative displacement is out of range"))?;
    Ok(displacement.to_le_bytes())
}

fn emit_rip_slot_instruction(
    code: &mut Vec<u8>,
    entry: usize,
    prefix: &[u8],
    slot: usize,
    label: &str,
) -> Result<(), String> {
    code.extend_from_slice(prefix);
    let after = entry
        .checked_add(code.len())
        .and_then(|value| value.checked_add(4))
        .ok_or_else(|| format!("{label} instruction address overflowed"))?;
    code.extend_from_slice(&rel32(after, slot, label)?);
    Ok(())
}

fn build_process_lifetime_shim(
    entry: usize,
    active: usize,
    dispatch: usize,
    integer_arg_count: u8,
) -> Result<Vec<u8>, String> {
    let mut code = Vec::with_capacity(96);
    // lock inc qword ptr [rip+active]
    emit_rip_slot_instruction(
        &mut code,
        entry,
        &[0xF0, 0x48, 0xFF, 0x05],
        active,
        "process-lifetime active increment",
    )?;
    // mov rax,qword ptr [rip+dispatch]
    emit_rip_slot_instruction(
        &mut code,
        entry,
        &[0x48, 0x8B, 0x05],
        dispatch,
        "process-lifetime dispatch load",
    )?;
    let stack_arg_count = usize::from(integer_arg_count.saturating_sub(4));
    let stack_copy_len = stack_arg_count
        .checked_mul(8)
        .ok_or_else(|| "process-lifetime stack-copy length overflowed".to_string())?;
    let aligned_stack_copy = stack_copy_len
        .checked_add(15)
        .map(|value| value & !15)
        .ok_or_else(|| "process-lifetime stack-copy alignment overflowed".to_string())?;
    let frame_len = 0x28usize
        .checked_add(aligned_stack_copy)
        .ok_or_else(|| "process-lifetime stack frame overflowed".to_string())?;
    let frame_len_u8 = u8::try_from(frame_len)
        .map_err(|_| "process-lifetime stack frame exceeds imm8".to_string())?;
    code.extend_from_slice(&[0x48, 0x83, 0xEC, frame_len_u8]); // sub rsp,frame
    for stack_index in 0..stack_arg_count {
        let source = frame_len
            .checked_add(0x28 + stack_index * 8)
            .ok_or_else(|| "process-lifetime stack source overflowed".to_string())?;
        let destination = 0x20 + stack_index * 8;
        // mov r10,qword ptr [rsp+disp32]
        code.extend_from_slice(&[0x4C, 0x8B, 0x94, 0x24]);
        code.extend_from_slice(
            &u32::try_from(source)
                .map_err(|_| "process-lifetime stack source exceeds disp32".to_string())?
                .to_le_bytes(),
        );
        // mov qword ptr [rsp+disp8],r10
        code.extend_from_slice(&[
            0x4C,
            0x89,
            0x54,
            0x24,
            u8::try_from(destination)
                .map_err(|_| "process-lifetime stack destination exceeds disp8".to_string())?,
        ]);
    }
    code.extend_from_slice(&[0xFF, 0xD0]); // call rax
    code.extend_from_slice(&[0x48, 0x83, 0xC4, frame_len_u8]); // add rsp,frame
                                                               // lock dec qword ptr [rip+active]
    emit_rip_slot_instruction(
        &mut code,
        entry,
        &[0xF0, 0x48, 0xFF, 0x0D],
        active,
        "process-lifetime active decrement",
    )?;
    code.push(0xC3); // ret
    Ok(code)
}

fn build_absolute_slot_jump(gateway: usize, trampoline_slot: usize) -> Result<[u8; 6], String> {
    let mut code = [0xFF, 0x25, 0, 0, 0, 0];
    let displacement = rel32(
        gateway
            .checked_add(code.len())
            .ok_or_else(|| "process-lifetime gateway address overflowed".to_string())?,
        trampoline_slot,
        "process-lifetime trampoline gateway",
    )?;
    code[2..6].copy_from_slice(&displacement);
    Ok(code)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatBinding {
    pub combat_ptr: usize,
    pub combat_id: u32,
    pub generation: u64,
}

/// Process-local exact record carrier. `record_bytes` may contain engine
/// pointers and therefore MUST be projected before serialization.
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

/// A bounded, non-destructive publication view of at most the oldest terminal
/// record plus all current diagnostics. Any record remains at the queue front
/// until the matching reservation is committed. A failed publication must roll
/// the reservation back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRecordReservation {
    pub reservation_id: u64,
    /// Publishable reservation payload. `records` contains at most one row,
    /// while the diagnostic fields are exact non-destructive counter
    /// snapshots taken at the same reservation boundary.
    pub drain: TerminalDrain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedTerminalReservation {
    pub reservation_id: u64,
    pub drain: TerminalDrain,
}

#[derive(Clone, Copy)]
struct ActiveTerminalReservation {
    reservation_id: u64,
    record_count: usize,
    diagnostics: [u64; 4],
    sealed_terminal: bool,
}

struct HookState {
    bindings: BTreeMap<usize, (u32, u64)>,
    records: VecDeque<RawTerminalRecordCapture>,
    reservation: Option<ActiveTerminalReservation>,
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
            // Allocate off the game thread during hook installation so the
            // terminal callback never grows the carrier heap.
            records: VecDeque::with_capacity(MAX_PENDING_RECORDS),
            reservation: None,
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
/// transaction without consuming producer custody. The receipts remain owned
/// here until the combined sink transaction has been preflighted and committed.
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
    // Publish the exact module owner immediately before the first fallible
    // MinHook create. Per-site bits retain even disabled creations.
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
    // Arm the passive nested observer before either writer can publish a
    // context. This prevents a writer from reaching an unhooked allocator in
    // the installation window.
    for target in [
        sites.record_allocator,
        sites.overrun_writer,
        sites.normal_writer,
    ] {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            return Err(fail_open_install(
                sites,
                format!("terminal hook enable failed at 0x{target:X}: {status}"),
            ));
        }
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
    let relay_stub = read_exact_bytes(relay, 14)?;
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
    for target in [sites.normal_writer, sites.overrun_writer] {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("disable 0x{target:X}: {status}")),
        }
    }
    if errors.is_empty() {
        if let Err(error) = wait_for_writer_quiescence() {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        match unsafe { MinHook::disable_hook(sites.record_allocator as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("disable 0x{:X}: {status}", sites.record_allocator)),
        }
    }
    if errors.is_empty() {
        if let Err(error) = wait_for_allocator_quiescence() {
            errors.push(error);
        }
    }
    // Never remove a target while a writer could still be entering its nested
    // allocator. Retaining disabled hooks and live trampolines is the safe
    // fail-closed state when quiescence or disablement cannot be proven.
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let _retained_relays = retained_relays;
    for (index, target) in [
        sites.normal_writer,
        sites.overrun_writer,
        sites.record_allocator,
    ]
    .into_iter()
    .enumerate()
    .rev()
    {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {
                CREATED_HOOK_MASK.fetch_and(!(1usize << index), Ordering::AcqRel);
            }
            Err(status) => errors.push(format!("remove 0x{target:X}: {status}")),
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
    for (index, target) in [
        sites.normal_writer,
        sites.overrun_writer,
        sites.record_allocator,
    ]
    .into_iter()
    .enumerate()
    .rev()
    {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {
                CREATED_HOOK_MASK.fetch_and(!(1usize << index), Ordering::AcqRel);
            }
            Err(status) => {
                return Err(format!(
                    "terminal hook removal failed at 0x{target:X}: {status}"
                ));
            }
        }
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

/// Legacy non-transactional teardown entry point.
///
/// Production callers must use `seal_and_reserve_all_for_uninstall`, publish
/// the returned drain, and then explicitly commit or roll back its reservation.
/// Refusing here prevents a caller from releasing hook custody and
/// self-attesting sink delivery without that transaction.
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
/// complete sealed terminal carrier without removing records or diagnostics.
/// Commit only after the final H4CR reaches its sink; roll back on sink failure.
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
        // Close admission before disabling any target. This prevents a live
        // reservation or callback append from entering between the quiescence
        // boundary and the complete terminal snapshot below.
        state.reservation_admission_sealed = true;
    }
    if let Some(sites) = sites {
        for target in [sites.normal_writer, sites.overrun_writer] {
            match unsafe { MinHook::disable_hook(target as *mut c_void) } {
                Ok(())
                | Err(MH_STATUS::MH_ERROR_DISABLED)
                | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
                Err(status) => {
                    return Err(format!(
                        "terminal writer seal failed at 0x{target:X}: {status}"
                    ))
                }
            }
        }
        wait_for_writer_quiescence()?;
        match unsafe { MinHook::disable_hook(sites.record_allocator as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
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
    // reservation after the earlier admission seal and callback quiescence.
    if let Some(sites) = sites {
        release_hook_receipts(sites)?;
        INSTALLED.store(false, Ordering::Release);
        // Trampolines cease to be callable after exact release, but the
        // executable base remains immutable projection custody until every raw
        // terminal record has been authenticated and sink-committed.
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
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| "terminal sealed commit mutex poisoned".to_string())?;
    let state_mutex = STATE
        .get()
        .ok_or_else(|| "terminal hook state is unavailable during sealed commit".to_string())?;
    let mut state = state_mutex
        .lock()
        .map_err(|_| "terminal hook state mutex poisoned during sealed commit".to_string())?;
    let active = matching_sealed_reservation(&state, reservation_id)?;
    validate_reserved_diagnostics(active, reservation_id)?;
    if state.records.len() < active.record_count {
        return Err("terminal sealed reservation lost its record prefix".into());
    }
    let mut receipts = HOOK_RECEIPTS
        .get_or_init(|| Mutex::new(HookReceiptState::default()))
        .lock()
        .map_err(|_| "terminal hook receipt mutex poisoned during sealed commit".to_string())?;
    complete_released_receipts(&receipts)?;
    if receipts.release_delivery_committed {
        return Err("terminal sealed release delivery was already committed".into());
    }
    state.records.drain(..active.record_count);
    subtract_reserved_diagnostics(active);
    state.reservation = None;
    receipts.release_delivery_committed = true;
    MODULE_BASE.store(0, Ordering::Release);
    Ok(())
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

/// Apply a previously successful combined-commit preflight. The coordinator
/// serializes this with every individual terminal API and calls it only after
/// every D55 lane has preflighted, so no fallible work remains. Poisoned locks
/// are recovered because their invariants were independently checked by the
/// preflight immediately preceding the atomic multi-lane mutation.
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
    let receipt_mutex = HOOK_RECEIPTS.get_or_init(|| Mutex::new(HookReceiptState::default()));
    let mut receipts = match receipt_mutex.lock() {
        Ok(receipts) => receipts,
        Err(poisoned) => poisoned.into_inner(),
    };
    debug_assert!(complete_released_receipts(&receipts).is_ok());
    debug_assert!(!receipts.release_delivery_committed);
    // The combined reservation itself carried this exact immutable receipt
    // array through the same durable sink transaction as the terminal rows.
    // Clear producer custody here so combined commit has no residual
    // individual handoff and reinstall is immediately eligible.
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
/// in producer custody and can be reserved identically on retry.
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

/// Removed compatibility path retained only as a fail-closed source-API guard.
///
/// A terminal drain cannot be consumed safely without an external durable sink
/// boundary. Callers must migrate to the explicit sealed reservation API.
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

/// Replace the process-pointer join only after the preceding queue has been
/// drained. This preserves bindings for combats that ended between frames.
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
        "terminal drain is disabled; use reserve_next_record with explicit commit/rollback custody"
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

/// Reserve a clone of the oldest terminal record without removing or moving
/// it, together with exact snapshots of every diagnostic counter. A
/// diagnostic-only reservation is returned when no record is pending. Only one
/// reservation may exist so a publisher cannot overtake an older record while
/// hook callbacks append newer records at the back.
pub fn reserve_next_record() -> Result<Option<TerminalRecordReservation>, String> {
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
    let record = state.records.front().cloned();
    let diagnostics = diagnostic_snapshot();
    if record.is_none() && diagnostics.iter().all(|count| *count == 0) {
        return Ok(None);
    }
    let reservation_id = next_terminal_reservation_id()?;
    let record_count = usize::from(record.is_some());
    state.reservation = Some(ActiveTerminalReservation {
        reservation_id,
        record_count,
        diagnostics,
        sealed_terminal: false,
    });
    Ok(Some(TerminalRecordReservation {
        reservation_id,
        drain: TerminalDrain {
            records: record.into_iter().collect(),
            dropped_lock_busy: diagnostics[0],
            dropped_unbound: diagnostics[1],
            dropped_unreadable: diagnostics[2],
            dropped_overflow: diagnostics[3],
        },
    }))
}

/// Commit the exact active reservation after its projected row has been
/// durably published. The reserved front record is the only record removed.
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
    match active.record_count {
        0 => {}
        1 if state.records.pop_front().is_some() => {}
        1 => return Err("terminal hook reserved record disappeared before commit".into()),
        _ => return Err("terminal hook reservation record count exceeds one".into()),
    }
    subtract_reserved_diagnostics(active);
    state.reservation = None;
    Ok(())
}

/// Roll back a failed publication. The front record remains in place and will
/// be offered again before every record appended while it was reserved.
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
            Ok(())
        }
        Some(active) => Err(format!(
            "terminal hook reservation rollback id mismatch ({reservation_id} != {})",
            active.reservation_id
        )),
        None => Err("terminal hook reservation rollback without an active reservation".into()),
    }
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
    // These records predate anything appended after the legacy drain. Restore
    // them at the front, in their original order, rather than allowing newer
    // callback records to overtake them.
    for record in records.into_iter().rev() {
        if state.records.len() == MAX_PENDING_RECORDS {
            state.records.pop_back();
            DROPPED_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        }
        state.records.push_front(record);
    }
    Ok(())
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
        // Never evict the front while a publisher holds its reservation. Drop
        // the incoming newest record instead, preserving the exact order and
        // identity promised by the active reservation.
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
