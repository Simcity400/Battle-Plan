//! Exact-a729 arbitrary-PC checkpoints for transient division attrition components.
//!
//! Each authenticated site is replaced by a five-byte relative jump to an
//! allocation within +/-2 GiB.  The generated stub executes the displaced
//! relocation-free instructions on the authenticated side of the checkpoint,
//! preserves RFLAGS, every GPR, and the complete FXSAVE64 state (including
//! XMM0..15 and MXCSR), calls a non-unwinding Rust reducer with ABI shadow space
//! and a 16-byte-aligned stack, then resumes without a scratch-register leak.

use core::ffi::c_void;
use core::ptr;
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::{
    ExactDivisionAttritionComponentBatch, ExactDivisionAttritionComponentRow,
    ATTRITION_COMPONENT_LIVE_FLAGS, ATTRITION_COMPONENT_MAX_ROWS,
    ATTRITION_COMPONENT_OBSERVATION_MASK, ATTRITION_REFRESH_CALLER_RVA, MAX_OBSERVER_TAG_DB_ID,
    MAX_PROVINCE_ID,
};

type Bool = i32;
type Dword = u32;
type Handle = *mut c_void;

const MEM_COMMIT: Dword = 0x1000;
const MEM_RESERVE: Dword = 0x2000;
const MEM_RELEASE: Dword = 0x8000;
const PAGE_EXECUTE_READ: Dword = 0x20;
const PAGE_EXECUTE_READWRITE: Dword = 0x40;
const TH32CS_SNAPTHREAD: Dword = 0x0000_0004;
const THREAD_SUSPEND_RESUME: Dword = 0x0002;
const THREAD_GET_CONTEXT: Dword = 0x0008;
const CONTEXT_CONTROL_AMD64: Dword = 0x0010_0001;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const ERROR_INVALID_HANDLE: Dword = 6;
const ERROR_INVALID_PARAMETER: Dword = 87;
const ALLOCATION_GRANULARITY: usize = 0x1_0000;
const STUB_ALLOCATION_LEN: usize = 0x1000;
const MAX_REL32_DISTANCE: usize = 0x7000_0000;
const CDIVISION_VTABLE_RVA: usize = 0x0294_3CB0;
const CURRENT_TICK_RVA: usize = 0x0331_BFE0;
const REFRESH_RETURN_RVA: usize = 0x00C7_959D;
const FULL_COMPONENT_MASK: u64 = (1 << 10) - 1;

pub const CHECKPOINT_SENTINEL_JOB_ID: &str =
    "20260721T085011Z-wave6-arc1-inline-sites-root-confirm-f06bc8e0";
pub const CHECKPOINT_SENTINEL_ARTIFACT_SHA256: &str =
    "739d4221cd6de08e7909d31cfac16a052977a30b8d46cca71a33356da55bafd7";

#[repr(C)]
struct ThreadEntry32 {
    size: Dword,
    usage: Dword,
    thread_id: Dword,
    owner_process_id: Dword,
    base_priority: i32,
    delta_priority: i32,
    flags: Dword,
}

#[repr(C, align(16))]
struct ThreadContext64 {
    homes: [u64; 6],
    context_flags: Dword,
    mx_csr: Dword,
    segments: [u16; 6],
    eflags: Dword,
    debug_registers: [u64; 6],
    registers_through_r15: [u64; 16],
    rip: u64,
    extended: [u8; 976],
}

impl ThreadContext64 {
    fn control() -> Self {
        let mut value: Self = unsafe { core::mem::zeroed() };
        value.context_flags = CONTEXT_CONTROL_AMD64;
        value
    }
}
const _: [(); 1232] = [(); core::mem::size_of::<ThreadContext64>()];

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        allocation_type: Dword,
        protect: Dword,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: Dword) -> Bool;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: Dword,
        old_protect: *mut Dword,
    ) -> Bool;
    fn FlushInstructionCache(process: Handle, address: *const c_void, size: usize) -> Bool;
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentProcessId() -> Dword;
    fn GetCurrentThreadId() -> Dword;
    fn CreateToolhelp32Snapshot(flags: Dword, process_id: Dword) -> Handle;
    fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry32) -> Bool;
    fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry32) -> Bool;
    fn OpenThread(access: Dword, inherit: Bool, thread_id: Dword) -> Handle;
    fn SuspendThread(thread: Handle) -> Dword;
    fn ResumeThread(thread: Handle) -> Dword;
    fn CloseHandle(handle: Handle) -> Bool;
    fn GetLastError() -> Dword;
    fn GetThreadContext(thread: Handle, context: *mut ThreadContext64) -> Bool;
    fn GetModuleHandleW(module_name: *const u16) -> Handle;
    fn ReadProcessMemory(
        process: Handle,
        base_address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        bytes_read: *mut usize,
    ) -> Bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
enum Checkpoint {
    Entry = 0,
    Upkeep = 1,
    Terrain = 2,
    Equipment = 3,
    State = 4,
    SupplyA = 5,
    SupplyB = 6,
    StateModifiers = 7,
    WeatherModifiers = 8,
    DerivedTerms = 9,
    Training = 10,
    BorderWar = 11,
    Final = 12,
    CachedJoin = 13,
}

#[derive(Clone, Copy)]
enum CallbackSide {
    BeforeDisplaced,
    AfterDisplaced,
}

#[derive(Clone, Copy)]
struct HookSite {
    checkpoint: Checkpoint,
    rva: usize,
    expected: &'static [u8],
    displaced: &'static [u8],
    callback_side: CallbackSide,
    continuation_rva: usize,
}

const SITES: &[HookSite] = &[
    HookSite { checkpoint: Checkpoint::Entry, rva: 0x00C6_5DC0, expected: &[0x48,0x8b,0xc4,0x48,0x89,0x58,0x08], displaced: &[0x48,0x8b,0xc4,0x48,0x89,0x58,0x08], callback_side: CallbackSide::BeforeDisplaced, continuation_rva: 0x00C6_5DC7 },
    HookSite { checkpoint: Checkpoint::Upkeep, rva: 0x00C6_5E3C, expected: &[0x48,0x8b,0x83,0xb8,0x00,0x00,0x00], displaced: &[0x48,0x8b,0x83,0xb8,0x00,0x00,0x00], callback_side: CallbackSide::BeforeDisplaced, continuation_rva: 0x00C6_5E43 },
    HookSite { checkpoint: Checkpoint::Terrain, rva: 0x00C6_5ED7, expected: &[0x48,0x8b,0xde,0x48,0x89,0x5c,0x24,0x58], displaced: &[0x48,0x8b,0xde,0x48,0x89,0x5c,0x24,0x58], callback_side: CallbackSide::BeforeDisplaced, continuation_rva: 0x00C6_5EDF },
    HookSite { checkpoint: Checkpoint::Equipment, rva: 0x00C6_608D, expected: &[0x48,0x8b,0x7c,0x24,0x60], displaced: &[0x48,0x8b,0x7c,0x24,0x60], callback_side: CallbackSide::BeforeDisplaced, continuation_rva: 0x00C6_6092 },
    HookSite { checkpoint: Checkpoint::State, rva: 0x00C6_61D4, expected: &[0x48,0x03,0xd8,0x4d,0x85,0xe4], displaced: &[0x48,0x03,0xd8,0x4d,0x85,0xe4], callback_side: CallbackSide::AfterDisplaced, continuation_rva: 0x00C6_61DA },
    // The short branch is authenticated but deliberately not relocated.  The
    // stub executes only ADD and resumes at its exact branch target/join.
    HookSite { checkpoint: Checkpoint::SupplyA, rva: 0x00C6_625F, expected: &[0x48,0x03,0xda,0xeb,0x4f], displaced: &[0x48,0x03,0xda], callback_side: CallbackSide::AfterDisplaced, continuation_rva: 0x00C6_62B3 },
    HookSite { checkpoint: Checkpoint::SupplyB, rva: 0x00C6_62A0, expected: &[0x48,0x03,0xda,0x4d,0x85,0xe4], displaced: &[0x48,0x03,0xda,0x4d,0x85,0xe4], callback_side: CallbackSide::AfterDisplaced, continuation_rva: 0x00C6_62A6 },
    HookSite { checkpoint: Checkpoint::StateModifiers, rva: 0x00C6_67FE, expected: &[0x48,0x03,0x5d,0xd8,0x48,0x8b,0x55,0x08], displaced: &[0x48,0x03,0x5d,0xd8,0x48,0x8b,0x55,0x08], callback_side: CallbackSide::AfterDisplaced, continuation_rva: 0x00C6_6806 },
    HookSite { checkpoint: Checkpoint::WeatherModifiers, rva: 0x00C6_6AA7, expected: &[0x48,0x03,0x5d,0xe0,0x49,0x8b,0x8d,0xc0,0x00,0x00,0x00], displaced: &[0x48,0x03,0x5d,0xe0,0x49,0x8b,0x8d,0xc0,0x00,0x00,0x00], callback_side: CallbackSide::AfterDisplaced, continuation_rva: 0x00C6_6AB2 },
    HookSite { checkpoint: Checkpoint::DerivedTerms, rva: 0x00C6_6F43, expected: &[0x48,0x03,0xd8,0x48,0x8b,0x74,0x24,0x60], displaced: &[0x48,0x03,0xd8,0x48,0x8b,0x74,0x24,0x60], callback_side: CallbackSide::AfterDisplaced, continuation_rva: 0x00C6_6F4B },
    HookSite { checkpoint: Checkpoint::Training, rva: 0x00C6_6F70, expected: &[0x48,0x89,0x5c,0x24,0x58], displaced: &[0x48,0x89,0x5c,0x24,0x58], callback_side: CallbackSide::BeforeDisplaced, continuation_rva: 0x00C6_6F75 },
    HookSite { checkpoint: Checkpoint::BorderWar, rva: 0x00C6_7106, expected: &[0x48,0x89,0x5c,0x24,0x58], displaced: &[0x48,0x89,0x5c,0x24,0x58], callback_side: CallbackSide::BeforeDisplaced, continuation_rva: 0x00C6_710B },
    HookSite { checkpoint: Checkpoint::Final, rva: 0x00C6_7307, expected: &[0x48,0x89,0x54,0x24,0x58], displaced: &[0x48,0x89,0x54,0x24,0x58], callback_side: CallbackSide::AfterDisplaced, continuation_rva: 0x00C6_730C },
    HookSite { checkpoint: Checkpoint::CachedJoin, rva: 0x00C7_95A0, expected: &[0x48,0x89,0x8b,0x38,0x04,0x00,0x00], displaced: &[0x48,0x89,0x8b,0x38,0x04,0x00,0x00], callback_side: CallbackSide::AfterDisplaced, continuation_rva: 0x00C7_95A7 },
];

#[derive(Clone, Copy)]
struct SavedRegisters {
    base: *const u64,
    original_rsp: usize,
}

impl SavedRegisters {
    unsafe fn word(&self, index: usize) -> u64 { unsafe { self.base.add(index).read() } }
    unsafe fn r15(&self) -> u64 { unsafe { self.word(0) } }
    unsafe fn r14(&self) -> u64 { unsafe { self.word(1) } }
    unsafe fn r13(&self) -> u64 { unsafe { self.word(2) } }
    unsafe fn r12(&self) -> u64 { unsafe { self.word(3) } }
    unsafe fn rbx(&self) -> u64 { unsafe { self.word(11) } }
    unsafe fn rdx(&self) -> u64 { unsafe { self.word(12) } }
    unsafe fn rcx(&self) -> u64 { unsafe { self.word(13) } }
    unsafe fn rax(&self) -> u64 { unsafe { self.word(14) } }
    unsafe fn rbp(&self) -> u64 { unsafe { self.word(10) } }
    unsafe fn rsi(&self) -> u64 { unsafe { self.word(9) } }
    unsafe fn rdi(&self) -> u64 { unsafe { self.word(8) } }
    unsafe fn r8(&self) -> u64 { unsafe { self.word(7) } }
}

#[derive(Clone)]
struct Transaction {
    division: usize,
    raw_division_id: u32,
    province_id: u32,
    owner_tag_db_id: u32,
    tick: i64,
    sequence: u64,
    components: [i64; 10],
    mask: u64,
    next_component: usize,
    last_accumulator: i64,
    pre_multiplier_total: Option<i64>,
    final_multiplier: Option<i64>,
    final_total: Option<i64>,
}

thread_local! {
    static TRANSACTION: RefCell<Option<Transaction>> = const { RefCell::new(None) };
}

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static ADMISSION_OPEN: AtomicBool = AtomicBool::new(false);
static CALLBACKS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static NEXT_DRAIN_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static DROPPED_ROWS: AtomicU32 = AtomicU32::new(0);
static ROWS: OnceLock<Mutex<VecDeque<ExactDivisionAttritionComponentRow>>> = OnceLock::new();
static LIFECYCLE: Mutex<Option<InstalledSet>> = Mutex::new(None);

unsafe extern "system" fn checkpoint_callback(
    checkpoint_raw: u32,
    saved_base: *const u64,
    _fxsave_state: *const u8,
    original_rsp: usize,
) {
    if !ADMISSION_OPEN.load(Ordering::Acquire) || saved_base.is_null() || original_rsp == 0 {
        return;
    }
    let checkpoint = match checkpoint_raw {
        0 => Checkpoint::Entry,
        1 => Checkpoint::Upkeep,
        2 => Checkpoint::Terrain,
        3 => Checkpoint::Equipment,
        4 => Checkpoint::State,
        5 => Checkpoint::SupplyA,
        6 => Checkpoint::SupplyB,
        7 => Checkpoint::StateModifiers,
        8 => Checkpoint::WeatherModifiers,
        9 => Checkpoint::DerivedTerms,
        10 => Checkpoint::Training,
        11 => Checkpoint::BorderWar,
        12 => Checkpoint::Final,
        13 => Checkpoint::CachedJoin,
        _ => return,
    };
    let registers = SavedRegisters { base: saved_base, original_rsp };
    if catch_unwind(AssertUnwindSafe(|| unsafe { reduce_checkpoint(checkpoint, registers) })).is_err() {
        TRANSACTION.with(|slot| {
            if slot.borrow_mut().take().is_some() { drop_row(); }
        });
    }
}

unsafe fn reduce_checkpoint(checkpoint: Checkpoint, registers: SavedRegisters) {
    match checkpoint {
        Checkpoint::Entry => unsafe { begin_transaction(registers) },
        Checkpoint::Upkeep => {
            let snapshot = unsafe { read_i64(registers.original_rsp + 0x58) };
            if let Some(snapshot) = snapshot { observe_accumulator(0, snapshot); } else { lose_transaction(); }
        }
        Checkpoint::Terrain => observe_accumulator(1, unsafe { registers.rsi() as i64 }),
        Checkpoint::Equipment => observe_accumulator(2, unsafe { registers.rbx() as i64 }),
        Checkpoint::State => observe_accumulator(3, unsafe { registers.rbx() as i64 }),
        Checkpoint::SupplyA | Checkpoint::SupplyB => {
            observe_accumulator(4, unsafe { registers.rbx() as i64 });
        }
        Checkpoint::StateModifiers => observe_accumulator(5, unsafe { registers.rbx() as i64 }),
        Checkpoint::WeatherModifiers => observe_accumulator(6, unsafe { registers.rbx() as i64 }),
        Checkpoint::DerivedTerms => observe_accumulator(7, unsafe { registers.rbx() as i64 }),
        Checkpoint::Training => observe_accumulator(8, unsafe { registers.rbx() as i64 }),
        Checkpoint::BorderWar => observe_accumulator(9, unsafe { registers.rbx() as i64 }),
        Checkpoint::Final => finish_calculator(
            unsafe { registers.rdi() as i64 },
            unsafe { registers.rdx() as i64 },
        ),
        Checkpoint::CachedJoin => unsafe { finish_cached_join(registers) },
    }
}

unsafe fn begin_transaction(registers: SavedRegisters) {
    // An entry while a targeted transaction remains open is reentrancy/loss,
    // never an authenticated zero branch.
    TRANSACTION.with(|slot| {
        if slot.borrow_mut().take().is_some() { drop_row(); }
    });
    let base = MODULE_BASE.load(Ordering::Acquire);
    let return_address = unsafe { read_usize(registers.original_rsp) };
    if return_address != Some(base + REFRESH_RETURN_RVA) || unsafe { registers.r8() } != 0 {
        return;
    }
    let division = unsafe { registers.rcx() as usize };
    if division == 0
        || unsafe { read_usize(division) } != Some(base + CDIVISION_VTABLE_RVA)
    {
        drop_row();
        return;
    }
    let type_id = unsafe { read_u32(division + 0x08) };
    let raw_division_id = unsafe { read_u32(division + 0x0c) };
    let province_id = unsafe { read_u32(division + 0x18) };
    let owner_tag_db_id = unsafe { read_u32(division + 0x1c8) };
    let tick = unsafe { read_i32(base + CURRENT_TICK_RVA) };
    if !matches!(type_id, Some(4713 | 51))
        || !matches!(raw_division_id, Some(value) if value > 0 && value <= i32::MAX as u32)
        || !matches!(province_id, Some(value) if value > 0 && value <= MAX_PROVINCE_ID)
        || !matches!(owner_tag_db_id, Some(value) if value > 0 && value <= MAX_OBSERVER_TAG_DB_ID)
        || !matches!(tick, Some(value) if value >= 0)
    {
        drop_row();
        return;
    }
    let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if sequence == 0 {
        drop_row();
        return;
    }
    TRANSACTION.with(|slot| {
        *slot.borrow_mut() = Some(Transaction {
            division,
            raw_division_id: raw_division_id.unwrap(),
            province_id: province_id.unwrap(),
            owner_tag_db_id: owner_tag_db_id.unwrap(),
            tick: i64::from(tick.unwrap()),
            sequence,
            components: [0; 10],
            mask: 0,
            next_component: 0,
            last_accumulator: 0,
            pre_multiplier_total: None,
            final_multiplier: None,
            final_total: None,
        });
    });
}

fn observe_accumulator(component: usize, snapshot: i64) {
    TRANSACTION.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(tx) = borrowed.as_mut() else { return; };
        if component < tx.next_component || component >= tx.components.len() {
            borrowed.take();
            drop_row();
            return;
        }
        // Only three exact control-flow joins authenticate a zero mutation:
        // equipment joins the optional terrain block, supply joins the
        // optional state add, and border joins the optional training add.
        // No arbitrary later checkpoint is allowed to turn a missed callback
        // into zero.
        if tx.next_component < component
            && !seal_authenticated_zeros(tx, component)
        {
            borrowed.take();
            drop_row();
            return;
        }
        let Some(delta) = snapshot.checked_sub(tx.last_accumulator) else {
            borrowed.take();
            drop_row();
            return;
        };
        tx.components[component] = delta;
        tx.mask |= 1 << component;
        tx.next_component = component + 1;
        tx.last_accumulator = snapshot;
    });
}

fn seal_authenticated_zeros(tx: &mut Transaction, target: usize) -> bool {
    // Upkeep is mandatory. Terrain may be sealed only by its authenticated
    // equipment join. After the first three phases, every component is a
    // conditional accumulator mutation: because the entire site set is
    // installed atomically and callback failure clears TLS, observing a later
    // callback in the same intact transaction mechanically proves every
    // intervening mutation PC was bypassed.
    let allowed = (tx.next_component == 1 && target == 2)
        || (tx.next_component >= 3 && target > tx.next_component);
    if !allowed { return false; }
    while tx.next_component < target {
        tx.components[tx.next_component] = 0;
        tx.mask |= 1 << tx.next_component;
        tx.next_component += 1;
    }
    true
}

fn finish_calculator(final_multiplier: i64, final_total: i64) {
    TRANSACTION.with(|slot| {
        let mut borrowed = slot.borrow_mut();
        let Some(tx) = borrowed.as_mut() else { return; };
        // The final multiplier is the authenticated join after all conditional
        // mutations. With mandatory upkeep/terrain/equipment already proven,
        // it may seal any still-bypassed conditional suffix 3..9.
        if tx.next_component < 3 || tx.next_component > tx.components.len() {
            borrowed.take();
            drop_row();
            return;
        }
        while tx.next_component < 10 {
            tx.components[tx.next_component] = 0;
            tx.mask |= 1 << tx.next_component;
            tx.next_component += 1;
        }
        let recomposed = tx.components.iter().try_fold(0i64, |sum, value| sum.checked_add(*value));
        let expected_final = i128::from(tx.last_accumulator)
            .checked_mul(i128::from(final_multiplier))
            .and_then(|value| value.checked_div(100_000))
            .and_then(|value| i64::try_from(value).ok());
        if tx.mask != FULL_COMPONENT_MASK
            || recomposed != Some(tx.last_accumulator)
            || expected_final != Some(final_total)
        {
            borrowed.take();
            drop_row();
            return;
        }
        tx.pre_multiplier_total = Some(tx.last_accumulator);
        tx.final_multiplier = Some(final_multiplier);
        tx.final_total = Some(final_total);
    });
}

unsafe fn finish_cached_join(registers: SavedRegisters) {
    let division = unsafe { registers.rbx() as usize };
    let cached_total = unsafe { registers.rcx() as i64 };
    TRANSACTION.with(|slot| {
        let Some(tx) = slot.borrow_mut().take() else { return; };
        let identity_still_exact = division == tx.division
            && unsafe { read_usize(division) } == Some(MODULE_BASE.load(Ordering::Acquire) + CDIVISION_VTABLE_RVA)
            && unsafe { read_u32(division + 0x0c) } == Some(tx.raw_division_id)
            && unsafe { read_u32(division + 0x18) } == Some(tx.province_id)
            && unsafe { read_u32(division + 0x1c8) } == Some(tx.owner_tag_db_id)
            && unsafe { read_i64(division + 0x438) } == Some(cached_total);
        if !identity_still_exact
            || tx.mask != ATTRITION_COMPONENT_OBSERVATION_MASK
            || tx.final_total != Some(cached_total)
            || tx.pre_multiplier_total.is_none()
            || tx.final_multiplier.is_none()
        {
            drop_row();
            return;
        }
        publish(ExactDivisionAttritionComponentRow {
            raw_division_id: tx.raw_division_id,
            province_id: tx.province_id,
            owner_tag_db_id: tx.owner_tag_db_id,
            flags: ATTRITION_COMPONENT_LIVE_FLAGS,
            observation_mask: tx.mask,
            upkeep_total_raw: tx.components[0],
            terrain_raw: tx.components[1],
            equipment_raw: tx.components[2],
            state_raw: tx.components[3],
            supply_raw: tx.components[4],
            state_modifiers_raw: tx.components[5],
            weather_modifiers_raw: tx.components[6],
            derived_terms_raw: tx.components[7],
            training_raw: tx.components[8],
            border_war_raw: tx.components[9],
            pre_multiplier_total_raw: tx.pre_multiplier_total.unwrap(),
            final_multiplier_raw: tx.final_multiplier.unwrap(),
            final_total_raw: cached_total,
            cached_total_raw: cached_total,
            tick: tx.tick,
            sequence: tx.sequence,
            refresh_caller_rva: ATTRITION_REFRESH_CALLER_RVA,
        });
    });
}

fn lose_transaction() {
    TRANSACTION.with(|slot| {
        if slot.borrow_mut().take().is_some() { drop_row(); }
    });
}

fn drop_row() {
    let _ = DROPPED_ROWS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        Some(value.saturating_add(1))
    });
}

fn publish(row: ExactDivisionAttritionComponentRow) {
    let Some(rows) = ROWS.get() else { drop_row(); return; };
    let Ok(mut rows) = rows.lock() else { drop_row(); return; };
    if rows.len() == ATTRITION_COMPONENT_MAX_ROWS {
        rows.pop_front();
        drop_row();
    }
    rows.push_back(row);
}

unsafe fn read_u32(address: usize) -> Option<u32> {
    let mut value = [0u8; 4];
    unsafe { read_exact(address, &mut value) }.then(|| u32::from_le_bytes(value))
}
unsafe fn read_i32(address: usize) -> Option<i32> {
    let mut value = [0u8; 4];
    unsafe { read_exact(address, &mut value) }.then(|| i32::from_le_bytes(value))
}
unsafe fn read_i64(address: usize) -> Option<i64> {
    let mut value = [0u8; 8];
    unsafe { read_exact(address, &mut value) }.then(|| i64::from_le_bytes(value))
}
unsafe fn read_usize(address: usize) -> Option<usize> {
    let mut value = [0u8; core::mem::size_of::<usize>()];
    unsafe { read_exact(address, &mut value) }.then(|| usize::from_le_bytes(value))
}

unsafe fn read_exact(address: usize, out: &mut [u8]) -> bool {
    if address == 0 { return false; }
    let mut read = 0usize;
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr().cast(),
            out.len(),
            &mut read,
        ) != 0 && read == out.len()
    }
}

struct InstalledHook {
    site: HookSite,
    target: *mut u8,
    stub: *mut u8,
    stub_len: usize,
    original: Vec<u8>,
}
unsafe impl Send for InstalledHook {}

struct InstalledSet {
    hooks: Vec<InstalledHook>,
}
unsafe impl Send for InstalledSet {}

impl InstalledSet {
    fn execution_ranges(&self) -> Vec<(usize, usize)> {
        let mut ranges = Vec::with_capacity(self.hooks.len() * 2);
        for hook in &self.hooks {
            ranges.push((hook.target as usize, hook.target as usize + hook.original.len()));
            if !hook.stub.is_null() {
                ranges.push((hook.stub as usize, hook.stub as usize + hook.stub_len));
            }
        }
        ranges
    }
}

pub fn install() -> Result<(), String> {
    let mut lifecycle = LIFECYCLE.lock().map_err(|_| "ARC1 lifecycle poisoned")?;
    if lifecycle.is_some() {
        return if ADMISSION_OPEN.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err("ARC1 retained a failed patch transaction; uninstall must restore it first".into())
        };
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 { return Err("ARC1 module base unavailable".into()); }

    // All sentinels are checked before any executable byte is changed.
    for site in SITES {
        let target = base.checked_add(site.rva).ok_or("ARC1 site overflow")? as *const u8;
        let actual = unsafe { std::slice::from_raw_parts(target, site.expected.len()) };
        if actual != site.expected {
            return Err(format!("ARC1 exact-a729 sentinel mismatch at RVA {:#x}", site.rva));
        }
    }

    let mut hooks = Vec::with_capacity(SITES.len());
    for site in SITES {
        let target = base + site.rva;
        let continuation = base + site.continuation_rva;
        let stub = match unsafe { allocate_near(target) } {
            Ok(value) => value,
            Err(error) => { free_stubs(&mut hooks); return Err(error); }
        };
        let code = match emit_stub(*site, continuation, checkpoint_callback as *const c_void) {
            Ok(value) => value,
            Err(error) => {
                unsafe { VirtualFree(stub.cast(), 0, MEM_RELEASE); }
                free_stubs(&mut hooks);
                return Err(error);
            }
        };
        if code.len() > STUB_ALLOCATION_LEN {
            unsafe { VirtualFree(stub.cast(), 0, MEM_RELEASE); }
            free_stubs(&mut hooks);
            return Err("ARC1 generated stub exceeds allocation".into());
        }
        unsafe { ptr::copy_nonoverlapping(code.as_ptr(), stub, code.len()); }
        if unsafe { FlushInstructionCache(GetCurrentProcess(), stub.cast(), code.len()) } == 0 {
            unsafe { VirtualFree(stub.cast(), 0, MEM_RELEASE); }
            free_stubs(&mut hooks);
            return Err(format!("ARC1 stub flush failed at RVA {:#x}", site.rva));
        }
        let mut old = 0;
        if unsafe { VirtualProtect(stub.cast(), STUB_ALLOCATION_LEN, PAGE_EXECUTE_READ, &mut old) } == 0 {
            unsafe { VirtualFree(stub.cast(), 0, MEM_RELEASE); }
            free_stubs(&mut hooks);
            return Err(format!("ARC1 stub protect failed at RVA {:#x}", site.rva));
        }
        hooks.push(InstalledHook {
            site: *site,
            target: target as *mut u8,
            stub,
            stub_len: code.len(),
            original: site.expected.to_vec(),
        });
    }

    let suspended = match unsafe { SuspendedProcessThreads::suspend_all_other_threads() } {
        Ok(value) => value,
        Err(error) => { free_stubs(&mut hooks); return Err(error); }
    };
    let ranges: Vec<_> = hooks
        .iter()
        .map(|hook| (hook.target as usize, hook.target as usize + hook.original.len()))
        .collect();
    let inside = match suspended.any_instruction_pointer_in(&ranges) {
        Ok(value) => value,
        Err(error) => { drop(suspended); free_stubs(&mut hooks); return Err(error); }
    };
    if inside {
        drop(suspended);
        free_stubs(&mut hooks);
        return Err("ARC1 thread instruction pointer is inside a patch site".into());
    }
    // Allocation can take time. Recheck every exact sentinel with all other
    // threads stopped immediately before the first non-atomic five-byte write.
    let mut drift_rva = None;
    for hook in &hooks {
        let mut actual = vec![0u8; hook.original.len()];
        if !unsafe { read_exact(hook.target as usize, &mut actual) }
            || actual.as_slice() != hook.original.as_slice()
        {
            drift_rva = Some(hook.site.rva);
            break;
        }
    }
    if let Some(rva) = drift_rva {
        drop(suspended);
        free_stubs(&mut hooks);
        return Err(format!("ARC1 prepatch sentinel drift at RVA {rva:#x}"));
    }
    let rows = ROWS.get_or_init(|| Mutex::new(VecDeque::new()));
    let Ok(mut queue) = rows.lock() else {
        drop(suspended);
        free_stubs(&mut hooks);
        return Err("ARC1 queue poisoned".into());
    };
    queue.clear();
    drop(queue);
    DROPPED_ROWS.store(0, Ordering::Release);
    MODULE_BASE.store(base, Ordering::Release);
    let hook_epoch = NEXT_HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    if hook_epoch == 0 {
        MODULE_BASE.store(0, Ordering::Release);
        drop(suspended);
        free_stubs(&mut hooks);
        return Err("ARC1 hook epoch exhausted".into());
    }
    ACTIVE_HOOK_EPOCH.store(hook_epoch, Ordering::Release);

    let mut patched = 0usize;
    for hook in &hooks {
        if let Err(error) = unsafe { patch_rel32(hook.target, hook.original.len(), hook.stub) } {
            let mut rollback_clean = true;
            for attempted in hooks[..=patched].iter().rev() {
                if unsafe { restore_hook(attempted) }.is_err() { rollback_clean = false; }
            }
            drop(suspended);
            if rollback_clean {
                ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
                MODULE_BASE.store(0, Ordering::Release);
                free_stubs(&mut hooks);
            } else {
                // One or more sites may still target these pages.  Retain the
                // complete set under the lifecycle lock with admission closed;
                // explicit uninstall may safely retry restoration.
                ADMISSION_OPEN.store(false, Ordering::Release);
                *lifecycle = Some(InstalledSet { hooks });
            }
            return Err(error);
        }
        patched += 1;
    }
    ADMISSION_OPEN.store(true, Ordering::Release);
    drop(suspended);
    *lifecycle = Some(InstalledSet { hooks });
    Ok(())
}

pub fn drain() -> Result<ExactDivisionAttritionComponentBatch, String> {
    if !ADMISSION_OPEN.load(Ordering::Acquire) {
        return Err("ARC1 hooks are not installed".into());
    }
    let rows = ROWS.get().ok_or("ARC1 queue unavailable")?;
    let mut queue = rows.lock().map_err(|_| "ARC1 queue poisoned")?;
    let mut latest = BTreeMap::<u32, ExactDivisionAttritionComponentRow>::new();
    for row in queue.drain(..) {
        match latest.get(&row.raw_division_id) {
            Some(current) if current.sequence >= row.sequence => {}
            Some(_) => {
                latest.insert(row.raw_division_id, row);
            }
            None => { latest.insert(row.raw_division_id, row); }
        }
    }
    let drain_sequence = NEXT_DRAIN_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if drain_sequence == 0 { return Err("ARC1 drain sequence exhausted".into()); }
    Ok(ExactDivisionAttritionComponentBatch {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        drain_sequence,
        dropped_rows: DROPPED_ROWS.swap(0, Ordering::AcqRel),
        rows: latest.into_values().collect(),
    })
}

pub fn uninstall() -> Result<(), String> {
    let mut lifecycle = LIFECYCLE.lock().map_err(|_| "ARC1 lifecycle poisoned")?;
    let Some(set) = lifecycle.as_mut() else { return Ok(()); };
    let reopen_admission = ADMISSION_OPEN.swap(false, Ordering::AcqRel);
    let suspended = match unsafe { SuspendedProcessThreads::suspend_all_other_threads() } {
        Ok(value) => value,
        Err(error) => {
            if reopen_admission { ADMISSION_OPEN.store(true, Ordering::Release); }
            return Err(error);
        }
    };
    let in_flight = CALLBACKS_IN_FLIGHT.load(Ordering::Acquire);
    let inside = match suspended.any_instruction_pointer_in(&set.execution_ranges()) {
        Ok(value) => value,
        Err(error) => {
            drop(suspended);
            if reopen_admission { ADMISSION_OPEN.store(true, Ordering::Release); }
            return Err(error);
        }
    };
    if in_flight != 0 || inside {
        drop(suspended);
        if reopen_admission { ADMISSION_OPEN.store(true, Ordering::Release); }
        return Err(format!("ARC1 callbacks/execution still active ({in_flight})"));
    }
    for hook in set.hooks.iter().rev() {
        if let Err(error) = unsafe { restore_hook(hook) } {
            drop(suspended);
            return Err(error);
        }
    }
    drop(suspended);
    for hook in &mut set.hooks {
        if hook.stub.is_null() { continue; }
        if unsafe { VirtualFree(hook.stub.cast(), 0, MEM_RELEASE) } == 0 {
            return Err(format!("ARC1 stub free failed at RVA {:#x}", hook.site.rva));
        }
        hook.stub = ptr::null_mut();
    }
    lifecycle.take();
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    TRANSACTION.with(|slot| { slot.borrow_mut().take(); });
    Ok(())
}

fn emit_stub(site: HookSite, continuation: usize, callback: *const c_void) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(256);
    if matches!(site.callback_side, CallbackSide::AfterDisplaced) {
        out.extend_from_slice(site.displaced);
    }
    // pushfq + RAX,RCX,RDX,RBX,RBP,RSI,RDI,R8..R15.  The reducer's offsets
    // are pinned to this order.  Original RSP is saved_base+0x80.
    out.push(0x9c);
    out.extend_from_slice(&[0x50,0x51,0x52,0x53,0x55,0x56,0x57]);
    for opcode in 0x50u8..=0x57 { out.extend_from_slice(&[0x41, opcode]); }
    out.extend_from_slice(&[0x49,0x89,0xe4]); // mov r12,rsp (original r12 is saved)
    out.extend_from_slice(&[0x48,0x83,0xe4,0xf0]); // and rsp,-16
    out.extend_from_slice(&[0x48,0x81,0xec,0x20,0x02,0x00,0x00]); // shadow + FXSAVE64
    out.extend_from_slice(&[0x48,0x0f,0xae,0x84,0x24,0x20,0x00,0x00,0x00]); // fxsave64 [rsp+20]
    // The count is incremented in the stub before CALL and decremented only
    // after RET, closing both callback-region teardown races.
    out.extend_from_slice(&[0x48,0xb8]);
    out.extend_from_slice(&(core::ptr::addr_of!(CALLBACKS_IN_FLIGHT) as u64).to_le_bytes());
    out.extend_from_slice(&[0xf0,0x48,0xff,0x00]); // lock inc qword ptr [rax]
    out.push(0xb9); out.extend_from_slice(&(site.checkpoint as u32).to_le_bytes());
    out.extend_from_slice(&[0x4c,0x89,0xe2]); // mov rdx,r12
    out.extend_from_slice(&[0x4c,0x8d,0x44,0x24,0x20]); // lea r8,[rsp+20]
    out.extend_from_slice(&[0x4d,0x8d,0x8c,0x24,0x80,0x00,0x00,0x00]); // lea r9,[r12+80]
    out.extend_from_slice(&[0x48,0xb8]); out.extend_from_slice(&(callback as u64).to_le_bytes());
    out.extend_from_slice(&[0xff,0xd0]);
    out.extend_from_slice(&[0x48,0xb8]);
    out.extend_from_slice(&(core::ptr::addr_of!(CALLBACKS_IN_FLIGHT) as u64).to_le_bytes());
    out.extend_from_slice(&[0xf0,0x48,0xff,0x08]); // lock dec qword ptr [rax]
    out.extend_from_slice(&[0x48,0x0f,0xae,0x8c,0x24,0x20,0x00,0x00,0x00]); // fxrstor64
    out.extend_from_slice(&[0x4c,0x89,0xe4]); // mov rsp,r12
    for opcode in (0x58u8..=0x5f).rev() { out.extend_from_slice(&[0x41, opcode]); }
    out.extend_from_slice(&[0x5f,0x5e,0x5d,0x5b,0x5a,0x59,0x58]);
    out.push(0x9d);
    if matches!(site.callback_side, CallbackSide::BeforeDisplaced) {
        out.extend_from_slice(site.displaced);
    }
    // RIP-indirect absolute continuation; no live GPR is consumed.
    out.extend_from_slice(&[0xff,0x25,0x00,0x00,0x00,0x00]);
    out.extend_from_slice(&(continuation as u64).to_le_bytes());
    Ok(out)
}

unsafe fn allocate_near(target: usize) -> Result<*mut u8, String> {
    let center = target & !(ALLOCATION_GRANULARITY - 1);
    for distance in (0..=MAX_REL32_DISTANCE).step_by(ALLOCATION_GRANULARITY) {
        for candidate in [center.checked_sub(distance), center.checked_add(distance)] {
            let Some(candidate) = candidate else { continue; };
            let allocation = unsafe {
                VirtualAlloc(candidate as *mut c_void, STUB_ALLOCATION_LEN, MEM_RESERVE | MEM_COMMIT, PAGE_EXECUTE_READWRITE)
            } as *mut u8;
            if allocation.is_null() { continue; }
            let delta = (allocation as isize as i128) - ((target + 5) as i128);
            if delta >= i32::MIN as i128 && delta <= i32::MAX as i128 {
                return Ok(allocation);
            }
            unsafe { VirtualFree(allocation.cast(), 0, MEM_RELEASE); }
        }
    }
    Err(format!("ARC1 near allocation failed for {target:#x}"))
}

unsafe fn patch_rel32(target: *mut u8, span: usize, destination: *mut u8) -> Result<(), String> {
    let delta = destination as isize as i128 - (target as usize + 5) as i128;
    let delta = i32::try_from(delta).map_err(|_| "ARC1 rel32 destination out of range")?;
    let mut old = 0;
    if unsafe { VirtualProtect(target.cast(), span, PAGE_EXECUTE_READWRITE, &mut old) } == 0 {
        return Err("ARC1 target protect failed".into());
    }
    unsafe {
        target.write(0xe9);
        ptr::copy_nonoverlapping(delta.to_le_bytes().as_ptr(), target.add(1), 4);
        for offset in 5..span { target.add(offset).write(0x90); }
    }
    let mut ignored = 0;
    if unsafe { VirtualProtect(target.cast(), span, old, &mut ignored) } == 0 {
        return Err("ARC1 target protection restore failed".into());
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), target.cast(), span) } == 0 {
        return Err("ARC1 target instruction-cache flush failed".into());
    }
    Ok(())
}

unsafe fn restore_hook(hook: &InstalledHook) -> Result<(), String> {
    let target = hook.target;
    let original = &hook.original;
    let mut current = vec![0u8; original.len()];
    if !unsafe { read_exact(target as usize, &mut current) } {
        return Err("ARC1 restore current-byte read failed".into());
    }
    if current.as_slice() == original.as_slice() { return Ok(()); }
    let delta = hook.stub as isize as i128 - (target as usize + 5) as i128;
    let delta = i32::try_from(delta).map_err(|_| "ARC1 restore rel32 out of range")?;
    let mut expected_patch = vec![0x90; original.len()];
    expected_patch[0] = 0xe9;
    expected_patch[1..5].copy_from_slice(&delta.to_le_bytes());
    if current != expected_patch {
        return Err(format!("ARC1 restore refused foreign bytes at RVA {:#x}", hook.site.rva));
    }
    let mut old = 0;
    if unsafe { VirtualProtect(target.cast(), original.len(), PAGE_EXECUTE_READWRITE, &mut old) } == 0 {
        return Err("ARC1 restore protect failed".into());
    }
    unsafe { ptr::copy_nonoverlapping(original.as_ptr(), target, original.len()); }
    let mut ignored = 0;
    if unsafe { VirtualProtect(target.cast(), original.len(), old, &mut ignored) } == 0 {
        return Err("ARC1 restore protection restore failed".into());
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), target.cast(), original.len()) } == 0 {
        return Err("ARC1 restore instruction-cache flush failed".into());
    }
    let mut restored = vec![0u8; original.len()];
    if !unsafe { read_exact(target as usize, &mut restored) }
        || restored.as_slice() != original.as_slice()
    {
        return Err("ARC1 restore byte verification failed".into());
    }
    Ok(())
}

fn free_stubs(hooks: &mut Vec<InstalledHook>) {
    for hook in hooks.drain(..).rev() {
        unsafe { VirtualFree(hook.stub.cast(), 0, MEM_RELEASE); }
    }
}

struct SuspendedProcessThreads { handles: Vec<Handle> }

impl SuspendedProcessThreads {
    unsafe fn suspend_all_other_threads() -> Result<Self, String> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE { return Err("ARC1 thread snapshot failed".into()); }
        let process_id = unsafe { GetCurrentProcessId() };
        let current_thread_id = unsafe { GetCurrentThreadId() };
        let mut entry = ThreadEntry32 {
            size: core::mem::size_of::<ThreadEntry32>() as Dword,
            usage: 0, thread_id: 0, owner_process_id: 0,
            base_priority: 0, delta_priority: 0, flags: 0,
        };
        let mut handles = Vec::new();
        let mut present = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while present {
            if entry.owner_process_id == process_id && entry.thread_id != current_thread_id {
                let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT, 0, entry.thread_id) };
                if handle.is_null() {
                    let error = unsafe { GetLastError() };
                    if error != ERROR_INVALID_PARAMETER {
                        unsafe { CloseHandle(snapshot); }
                        resume_and_close(&mut handles);
                        return Err(format!("ARC1 thread open failed: {}", entry.thread_id));
                    }
                } else if unsafe { SuspendThread(handle) } == Dword::MAX {
                    let error = unsafe { GetLastError() };
                    unsafe { CloseHandle(handle); }
                    if error != ERROR_INVALID_HANDLE && error != ERROR_INVALID_PARAMETER {
                        unsafe { CloseHandle(snapshot); }
                        resume_and_close(&mut handles);
                        return Err(format!("ARC1 thread suspend failed: {}", entry.thread_id));
                    }
                } else {
                    handles.push(handle);
                }
            }
            present = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot); }
        Ok(Self { handles })
    }

    fn any_instruction_pointer_in(&self, ranges: &[(usize, usize)]) -> Result<bool, String> {
        for handle in &self.handles {
            let mut context = ThreadContext64::control();
            if unsafe { GetThreadContext(*handle, &mut context) } == 0 {
                return Err(format!("ARC1 GetThreadContext failed: {}", unsafe { GetLastError() }));
            }
            let rip = context.rip as usize;
            if ranges.iter().any(|(start, end)| rip >= *start && rip < *end) { return Ok(true); }
        }
        Ok(false)
    }
}

impl Drop for SuspendedProcessThreads {
    fn drop(&mut self) { resume_and_close(&mut self.handles); }
}

fn resume_and_close(handles: &mut Vec<Handle>) {
    for handle in handles.drain(..).rev() {
        unsafe { ResumeThread(handle); CloseHandle(handle); }
    }
}
