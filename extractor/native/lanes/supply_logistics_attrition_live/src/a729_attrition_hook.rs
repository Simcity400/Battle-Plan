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
    // The PROVINCE refusal is split out and ATTRIBUTED. It used to share this
    // arm with the type, division-id, owner, tick and read failures, so the
    // only output was the aggregate `dropped_rows` counter and neither the
    // reason nor the offending identity survived. The row is still dropped --
    // acceptance is unchanged -- but a capture can now name what caused it.
    if let Some(value) = province_id {
        if value != 0
            && compact_contract::bound_trips::bound_exceeded_u32(
                value,
                MAX_PROVINCE_ID,
                compact_contract::bound_trips::BOUND_TRIP_MAP_PROVINCE_ID,
            )
        {
            drop_row();
            return;
        }
    }
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

/// EVERY BUFFER THE STOPPED WINDOW NEEDS, RESERVED BEFORE THE FIRST SUSPENSION.
///
/// The window compares each site's live bytes against what it expects three
/// times -- once before the first write, and twice more per site during a
/// restore -- and each comparison used to allocate its own `vec![0u8; ..]` with
/// the whole game stopped. That is the hang this module's audit exists to
/// prevent: the allocator's lock can be held by a thread that can no longer
/// run.
///
/// So the buffers are reserved out here, where the game is still moving, sized
/// to the WIDEST site rather than to each one. Inside the window the code takes
/// `&mut scratch.observed[..len]` -- a subslice of memory that already exists.
/// Nothing in it grows, and `reserved_for` is the only place it is sized.
struct FrozenScratch {
    /// The live bytes read back from a site.
    observed: Vec<u8>,
    /// The patch a restore expects to find there.
    expected: Vec<u8>,
    /// The bytes read back after a restore, to verify it.
    restored: Vec<u8>,
}

impl FrozenScratch {
    fn reserved_for(hooks: &[InstalledHook]) -> Self {
        let widest = hooks
            .iter()
            .map(|hook| hook.original.len())
            .max()
            .unwrap_or_default();
        Self {
            observed: vec![0u8; widest],
            expected: vec![0u8; widest],
            restored: vec![0u8; widest],
        }
    }
}

/// Why a restore refused, WITHOUT building the sentence that says so.
///
/// `restore_hook` runs inside the stopped window on both the install rollback
/// path and the whole of `uninstall`, and it used to build its messages there --
/// on the failure paths, which is the worst place for it, because the hang
/// would then happen exactly when something had already gone wrong. This is
/// `Copy` and carries no owned memory; [`RestoreRefusal::message`] turns it into
/// ARC1's own error vocabulary AFTER the game is running again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestoreRefusal {
    CurrentBytesUnreadable,
    Rel32OutOfRange,
    /// Someone else's bytes are at the site; ARC1 will not overwrite them.
    ForeignBytes,
    ProtectFailed,
    ProtectionRestoreFailed,
    FlushFailed,
    VerificationFailed,
}

impl RestoreRefusal {
    fn message(self, rva: usize) -> String {
        match self {
            Self::CurrentBytesUnreadable => "ARC1 restore current-byte read failed".to_string(),
            Self::Rel32OutOfRange => "ARC1 restore rel32 out of range".to_string(),
            Self::ForeignBytes => {
                format!("ARC1 restore refused foreign bytes at RVA {rva:#x}")
            }
            Self::ProtectFailed => "ARC1 restore protect failed".to_string(),
            Self::ProtectionRestoreFailed => "ARC1 restore protection restore failed".to_string(),
            Self::FlushFailed => "ARC1 restore instruction-cache flush failed".to_string(),
            Self::VerificationFailed => "ARC1 restore byte verification failed".to_string(),
        }
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

    // EVERYTHING THE STOPPED WINDOW WILL NEED, BUILT WHILE THE GAME IS STILL
    // RUNNING. The ranges, the scratch buffers and the row queue were all
    // produced between the freeze and its drop, where an allocation is a hang
    // rather than a slow path.
    let ranges: Vec<_> = hooks
        .iter()
        .map(|hook| (hook.target as usize, hook.target as usize + hook.original.len()))
        .collect();
    let mut scratch = FrozenScratch::reserved_for(&hooks);
    // THE LOCK IS TAKEN HERE; THE QUEUE IS STILL EMPTIED DOWN THERE.
    //
    // Acquiring a mutex with every other thread suspended is the hang -- the
    // holder may be a thread that can no longer run. HOLDING one across the
    // freeze is not: nothing is waited for, and any game thread that wants it
    // is suspended rather than blocking us.
    //
    // So the acquisition moves out and the `clear` stays exactly where it was,
    // after every refusal gate. Emptying it out here instead would destroy a
    // queue that a freeze refusal, an instruction-pointer refusal or a sentinel
    // drift then declines to replace -- deleting observations because an
    // install that never happened was attempted.
    //
    // The `clear` allocates and frees nothing: `ExactDivisionAttritionComponentRow`
    // is `Copy`, so no element owns memory, and clearing keeps the buffer.
    let rows = ROWS.get_or_init(|| Mutex::new(VecDeque::new()));
    let Ok(mut queue) = rows.lock() else {
        free_stubs(&mut hooks);
        return Err("ARC1 queue poisoned".into());
    };

    let suspended = match suspend_all_other_threads() {
        Ok(value) => value,
        Err(error) => { free_stubs(&mut hooks); return Err(error); }
    };
    let inside = match any_instruction_pointer_in(&suspended, &ranges) {
        Ok(value) => value,
        Err(refusal) => {
            drop(suspended);
            free_stubs(&mut hooks);
            return Err(freeze_refusal_message(refusal));
        }
    };
    if inside {
        drop(suspended);
        free_stubs(&mut hooks);
        return Err("ARC1 thread instruction pointer is inside a patch site".into());
    }
    // Allocation can take time. Recheck every exact sentinel with all other
    // threads stopped immediately before the first non-atomic five-byte write.
    let drift_rva = unsafe { first_drifted_site(&hooks, &mut scratch) };
    if let Some(rva) = drift_rva {
        drop(suspended);
        free_stubs(&mut hooks);
        return Err(format!("ARC1 prepatch sentinel drift at RVA {rva:#x}"));
    }
    // PAST EVERY REFUSAL GATE, so this is the first point at which discarding
    // the previous session's rows is not discarding them for nothing. The lock
    // was taken before the freeze; only the emptying happens here.
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
                if unsafe { restore_hook(attempted, &mut scratch) }.is_err() {
                    rollback_clean = false;
                }
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
            // Rendered after the resume: `error` is a `&'static str` for
            // exactly as long as the process is stopped.
            return Err(error.to_string());
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
    // Both of these were built inside the stopped window: `execution_ranges`
    // reserves a vector, and every `restore_hook` used to allocate three more.
    let ranges = set.execution_ranges();
    let mut scratch = FrozenScratch::reserved_for(&set.hooks);
    let suspended = match suspend_all_other_threads() {
        Ok(value) => value,
        Err(error) => {
            if reopen_admission { ADMISSION_OPEN.store(true, Ordering::Release); }
            return Err(error);
        }
    };
    let in_flight = CALLBACKS_IN_FLIGHT.load(Ordering::Acquire);
    let inside = match any_instruction_pointer_in(&suspended, &ranges) {
        Ok(value) => value,
        Err(refusal) => {
            drop(suspended);
            if reopen_admission { ADMISSION_OPEN.store(true, Ordering::Release); }
            return Err(freeze_refusal_message(refusal));
        }
    };
    if in_flight != 0 || inside {
        drop(suspended);
        if reopen_admission { ADMISSION_OPEN.store(true, Ordering::Release); }
        return Err(format!("ARC1 callbacks/execution still active ({in_flight})"));
    }
    let mut refused = None;
    for hook in set.hooks.iter().rev() {
        if let Err(refusal) = unsafe { restore_hook(hook, &mut scratch) } {
            refused = Some((refusal, hook.site.rva));
            break;
        }
    }
    drop(suspended);
    if let Some((refusal, rva)) = refused {
        // Rendered after the resume; inside the window it is two `Copy` words.
        return Err(refusal.message(rva));
    }
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

/// Write one site's five-byte branch, ALLOCATING NOTHING.
///
/// It runs with the process stopped, so its refusals are `&'static str` and the
/// caller turns one into ARC1's `String` error after the resume. `.into()` here
/// was a `String` built inside the stopped window.
unsafe fn patch_rel32(
    target: *mut u8,
    span: usize,
    destination: *mut u8,
) -> Result<(), &'static str> {
    let delta = destination as isize as i128 - (target as usize + 5) as i128;
    let delta = i32::try_from(delta).map_err(|_| "ARC1 rel32 destination out of range")?;
    let mut old = 0;
    if unsafe { VirtualProtect(target.cast(), span, PAGE_EXECUTE_READWRITE, &mut old) } == 0 {
        return Err("ARC1 target protect failed");
    }
    unsafe {
        target.write(0xe9);
        ptr::copy_nonoverlapping(delta.to_le_bytes().as_ptr(), target.add(1), 4);
        for offset in 5..span { target.add(offset).write(0x90); }
    }
    let mut ignored = 0;
    if unsafe { VirtualProtect(target.cast(), span, old, &mut ignored) } == 0 {
        return Err("ARC1 target protection restore failed");
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), target.cast(), span) } == 0 {
        return Err("ARC1 target instruction-cache flush failed");
    }
    Ok(())
}

/// The first site whose live bytes are no longer its recorded original, or
/// `None` when every one still matches. ALLOCATES NOTHING.
///
/// A named function rather than a loop inside `install` so that the allocation
/// proof can drive THE PRODUCTION CODE. A test that re-typed this loop would
/// prove that the copy in the test allocates nothing, which is not the claim.
unsafe fn first_drifted_site(hooks: &[InstalledHook], scratch: &mut FrozenScratch) -> Option<usize> {
    for hook in hooks {
        let span = hook.original.len();
        let actual = &mut scratch.observed[..span];
        if !unsafe { read_exact(hook.target as usize, actual) } || actual != hook.original.as_slice()
        {
            return Some(hook.site.rva);
        }
    }
    None
}

/// Restore one site's original bytes, ALLOCATING NOTHING.
///
/// Every buffer it uses is a subslice of `scratch`, which was sized before the
/// process was stopped, and every refusal is a `Copy` reason rendered by the
/// caller once the game is running again.
unsafe fn restore_hook(
    hook: &InstalledHook,
    scratch: &mut FrozenScratch,
) -> Result<(), RestoreRefusal> {
    let target = hook.target;
    let original = &hook.original;
    let span = original.len();
    let current = &mut scratch.observed[..span];
    if !unsafe { read_exact(target as usize, current) } {
        return Err(RestoreRefusal::CurrentBytesUnreadable);
    }
    if current == original.as_slice() {
        return Ok(());
    }
    let delta = hook.stub as isize as i128 - (target as usize + 5) as i128;
    let delta = i32::try_from(delta).map_err(|_| RestoreRefusal::Rel32OutOfRange)?;
    let expected_patch = &mut scratch.expected[..span];
    expected_patch.fill(0x90);
    expected_patch[0] = 0xe9;
    expected_patch[1..5].copy_from_slice(&delta.to_le_bytes());
    if scratch.observed[..span] != scratch.expected[..span] {
        return Err(RestoreRefusal::ForeignBytes);
    }
    let mut old = 0;
    if unsafe { VirtualProtect(target.cast(), span, PAGE_EXECUTE_READWRITE, &mut old) } == 0 {
        return Err(RestoreRefusal::ProtectFailed);
    }
    unsafe { ptr::copy_nonoverlapping(original.as_ptr(), target, span); }
    let mut ignored = 0;
    if unsafe { VirtualProtect(target.cast(), span, old, &mut ignored) } == 0 {
        return Err(RestoreRefusal::ProtectionRestoreFailed);
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), target.cast(), span) } == 0 {
        return Err(RestoreRefusal::FlushFailed);
    }
    let restored = &mut scratch.restored[..span];
    if !unsafe { read_exact(target as usize, restored) } || restored != original.as_slice() {
        return Err(RestoreRefusal::VerificationFailed);
    }
    Ok(())
}

fn free_stubs(hooks: &mut Vec<InstalledHook>) {
    for hook in hooks.drain(..).rev() {
        unsafe { VirtualFree(hook.stub.cast(), 0, MEM_RELEASE); }
    }
}

/// The process freeze this module patches under.
///
/// It used to be hand-rolled here: a single toolhelp pass that read any
/// `Thread32Next` stop as the end of the list, so it could certify a truncated
/// enumeration and could not see a thread created after its snapshot. Both
/// holes are closed by `inline_detour`'s freeze, which refuses a broken
/// enumeration and iterates to a fixed point; ARC1 keeps its own error
/// vocabulary by rendering the typed refusal into an ARC1 message.
fn suspend_all_other_threads() -> Result<inline_detour::ProcessFreeze, String> {
    inline_detour::freeze_process().map_err(|refusal| format!("ARC1 patch refused: {refusal}"))
}

/// Is any frozen thread stopped inside a range about to be rewritten?
///
/// Fail-closed exactly as before: a context that cannot be read is a refusal,
/// never "not inside".
///
/// IT IS CALLED WITH THE PROCESS ALREADY STOPPED, so it hands the typed refusal
/// back instead of rendering it. `format!` here built a `String` while every
/// other thread was suspended -- on the refusal path, so the hang would have
/// happened exactly when the freeze had already found something wrong.
/// [`freeze_refusal_message`] is what says it, after the resume.
fn any_instruction_pointer_in(
    suspended: &inline_detour::ProcessFreeze,
    ranges: &[(usize, usize)],
) -> Result<bool, inline_detour::FreezeRefusal> {
    suspended.any_instruction_pointer_in(ranges)
}

fn freeze_refusal_message(refusal: inline_detour::FreezeRefusal) -> String {
    format!("ARC1 patch refused: {refusal}")
}

/// WHAT THE STOPPED WINDOW ACTUALLY REACHED, MEASURED.
///
/// The source scan in `lib.rs` proves nothing here is SPELLED like an
/// allocation, on every path, whether or not a test drives it. It cannot prove
/// the other half: that a subslice of a reserved buffer really does not grow,
/// or that a comparison of two `&[u8]` really does not copy. That is what this
/// module does -- it runs the window's own helpers under a counting allocator
/// and reads the number.
///
/// Neither proof subsumes the other. The scan covers paths no test reaches; the
/// count covers spellings no roster contains.
#[cfg(test)]
mod frozen_window_allocation_tests {
    use super::*;

    /// A hook whose "live bytes" are an ordinary buffer in this process, so
    /// `read_exact` -- which is `ReadProcessMemory` against our OWN process --
    /// reads it exactly as it reads the game's code.
    fn hook_over(bytes: &mut [u8], original: Vec<u8>) -> InstalledHook {
        InstalledHook {
            site: SITES[0],
            target: bytes.as_mut_ptr(),
            stub: ptr::null_mut(),
            stub_len: 0,
            original,
        }
    }

    /// Without this every count below is zero for the wrong reason.
    #[test]
    fn the_allocation_probe_is_this_test_binarys_allocator() {
        assert!(
            freeze_window_audit::the_probe_is_installed(),
            "the #[global_allocator] in lib.rs is not in force, so every \
             allocation count in this module is vacuously zero"
        );
    }

    /// THE SENTINEL RECHECK ALLOCATES NOTHING -- the real one, not a copy.
    ///
    /// What production change would make this fail? Restoring the
    /// `vec![0u8; hook.original.len()]` this loop used to build per site, or
    /// any `to_vec`/`format!` added to it later.
    #[test]
    fn rechecking_every_sentinel_allocates_nothing() {
        let mut first = [0x11u8; 8];
        let mut second = [0x22u8; 8];
        let mut hooks = vec![
            hook_over(&mut first, vec![0x11; 8]),
            hook_over(&mut second, vec![0x22; 8]),
        ];
        let mut scratch = FrozenScratch::reserved_for(&hooks);

        let (clean, allocations) =
            freeze_window_audit::observed(|| unsafe { first_drifted_site(&hooks, &mut scratch) });
        assert_eq!(clean, None, "matching bytes are not drift");
        assert_eq!(allocations, 0, "the sentinel recheck reached the allocator");

        // And the drifted answer -- the path that used to build a message.
        hooks[1].original[0] = 0x99;
        let (drifted, allocations) =
            freeze_window_audit::observed(|| unsafe { first_drifted_site(&hooks, &mut scratch) });
        assert_eq!(drifted, Some(SITES[0].rva), "drift must be reported");
        assert_eq!(allocations, 0, "reporting drift reached the allocator");
    }

    /// A RESTORE ALLOCATES NOTHING, on the path that succeeds without writing
    /// and on the path that refuses.
    ///
    /// Both are the frozen-window paths that used to build three `Vec`s and a
    /// `format!`. The paths that rewrite bytes need a real patched site and are
    /// covered by the source scan instead.
    #[test]
    fn a_restore_allocates_nothing() {
        let mut untouched = [0x33u8; 8];
        let already = hook_over(&mut untouched, vec![0x33; 8]);
        let mut scratch = FrozenScratch::reserved_for(std::slice::from_ref(&already));
        let (outcome, allocations) =
            freeze_window_audit::observed(|| unsafe { restore_hook(&already, &mut scratch) });
        assert_eq!(outcome, Ok(()), "bytes that already match need no restore");
        assert_eq!(allocations, 0, "an already-restored site allocated");

        // Bytes that are neither the original nor ARC1's own patch: refused,
        // and the RVA-bearing sentence is built by the caller after the resume.
        // The stub is the site itself, purely so the rel32 to it is in range
        // and the refusal reached is the FOREIGN-BYTES one under test rather
        // than the out-of-range one.
        let mut foreign = [0x44u8; 8];
        let mut stranger = hook_over(&mut foreign, vec![0x33; 8]);
        stranger.stub = stranger.target;
        let mut scratch = FrozenScratch::reserved_for(std::slice::from_ref(&stranger));
        let (outcome, allocations) =
            freeze_window_audit::observed(|| unsafe { restore_hook(&stranger, &mut scratch) });
        assert_eq!(outcome, Err(RestoreRefusal::ForeignBytes));
        assert_eq!(allocations, 0, "refusing foreign bytes allocated");
    }

    /// The scratch is sized to the WIDEST site, so no site's subslice grows it.
    #[test]
    fn the_scratch_covers_every_site_it_was_reserved_for() {
        let mut narrow = [0u8; 5];
        let mut wide = [0u8; 17];
        let hooks = vec![
            hook_over(&mut narrow, vec![0; 5]),
            hook_over(&mut wide, vec![0; 17]),
        ];
        let scratch = FrozenScratch::reserved_for(&hooks);
        for hook in &hooks {
            assert!(scratch.observed.len() >= hook.original.len());
            assert!(scratch.expected.len() >= hook.original.len());
            assert!(scratch.restored.len() >= hook.original.len());
        }
    }
}

#[cfg(test)]
mod freeze_call_site_tests {
    /// This module's production text, up to its test modules.
    ///
    /// A text pin rather than a behavioural one, and the reason is the same one
    /// the shared freeze's own call-site pin gives: `install` and `uninstall`
    /// here refuse before they reach the freeze unless the running process is
    /// the exact a729 `hoi4.exe`, so no offline test can drive either far
    /// enough to observe a freeze counter move. What CAN be stated is that the
    /// freeze is taken, that it is asked about the patch sites, and that both
    /// happen before the first byte is rewritten.
    fn production_code() -> String {
        let source = include_str!("a729_attrition_hook.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod ")
            .expect("this module has a test module")
            .0
            .to_string()
    }

    fn body_of(production: &str, signature: &str) -> String {
        production
            .split_once(signature)
            .unwrap_or_else(|| panic!("{signature} is declared in production"))
            .1
            .split_once("\npub fn ")
            .map(|(body, _)| body.to_string())
            .unwrap_or_else(|| {
                production
                    .split_once(signature)
                    .expect("checked above")
                    .1
                    .to_string()
            })
    }

    /// Falsifiable: fails if ARC1 ever rewrites a live callsite without the
    /// shared process freeze, or without asking where the frozen threads are.
    ///
    /// The mutations this catches, spelled as a future edit would write them:
    /// deleting the `suspend_all_other_threads()` call from `install` or
    /// `uninstall`; deleting the `any_instruction_pointer_in` check so a thread
    /// parked in a five-byte site is patched around; or reordering either after
    /// `patch_rel32`/`restore_hook`, which would rewrite the bytes first and
    /// stop the world afterwards.
    #[test]
    fn every_arc1_code_mutation_happens_under_the_shared_freeze() {
        let production = production_code();
        for (signature, write) in [
            ("pub fn install() -> Result<(), String> {", "patch_rel32("),
            ("pub fn uninstall() -> Result<(), String> {", "restore_hook("),
        ] {
            let body = body_of(&production, signature);
            let freeze = body
                .find("suspend_all_other_threads()")
                .unwrap_or_else(|| panic!("{signature} must freeze the process"));
            let clear = body
                .find("any_instruction_pointer_in(&suspended,")
                .unwrap_or_else(|| {
                    panic!("{signature} must ask where the frozen threads are stopped")
                });
            let mutation = body
                .find(write)
                .unwrap_or_else(|| panic!("{signature} must still contain {write}"));
            assert!(
                freeze < clear && clear < mutation,
                "{signature}: the order must be freeze, then check, then write \
                 (freeze at {freeze}, check at {clear}, write at {mutation})"
            );
        }
    }

    /// Falsifiable: fails if this module reaches for a private suspender again.
    ///
    /// `inline_detour`'s workspace scan is the enforcement; this is the local
    /// copy, so a lane editing THIS file sees the failure in its own crate
    /// rather than in a crate it may not be building.
    #[test]
    fn arc1_declares_no_thread_suspender_of_its_own() {
        let production = production_code();
        for fragment in ["Suspend", "Open", "Thread32", "GetThread"] {
            let banned = format!("{fragment}{}", match fragment {
                "Suspend" | "Open" => "Thread(",
                "Thread32" => "First(",
                _ => "Context(",
            });
            assert!(
                !production.contains(&banned),
                "ARC1 must not call {banned} itself; the shared freeze owns thread \
                 suspension, and every private copy of it certified a truncated \
                 thread list"
            );
        }
        assert!(
            production.contains("inline_detour::freeze_process()"),
            "ARC1 rewrites live bytes and must take the shared freeze"
        );
    }
}
