use crate::hook::Hook;
use engine_allocation as allocation;
use engine_land_inputs::clogistics_status as native;
use engine_native_memory::{Memory, memory::LiveMemory, read};
use engine_native::cgame_state;
use std::{
    io,
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering, fence},
    },
};

static BASE: AtomicUsize = AtomicUsize::new(0);
static WRITE: AtomicUsize = AtomicUsize::new(0);
static ADVANCE: AtomicUsize = AtomicUsize::new(0);
static CLEAR: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static FAILED_READS: AtomicU64 = AtomicU64::new(0);
static UNATTRIBUTED: AtomicU64 = AtomicU64::new(0);
static REFUSALS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
const REASONS: [&str; 12] = [
    "logistics balance write record unreadable",
    "logistics balance write values invalid",
    "logistics balance write index outside record",
    "logistics history pointer exceeds usize",
    "logistics balance write did not store its operand",
    "logistics available operand unreadable",
    "logistics native available operand is negative",
    "overlapping native balance writes",
    "native history changed during balance write",
    "unidentified native balance store during write",
    "foreign unwind or staging epoch changed during native write",
    "unclassified native operand refusal",
];
const UNWOUND: &str = "foreign unwind or staging epoch changed during native write";
const OVERLAP: &str = "overlapping native balance writes";
const HISTORY: &str = "native history changed during balance write";
const UNIDENTIFIED: &str = "unidentified native balance store during write";
fn refuse(reason: &'static str) {
    let index = REASONS
        .iter()
        .position(|candidate| *candidate == reason)
        .unwrap_or(REASONS.len() - 1);
    REFUSALS[index].fetch_add(1, Ordering::Relaxed);
    FAILED_READS.fetch_add(1, Ordering::Relaxed);
}

static CENSUS_REFUSALS: AtomicU64 = AtomicU64::new(0);
static CENSUS_REASON: Mutex<Option<&'static str>> = Mutex::new(None);
fn refuse_census(reason: &'static str) {
    CENSUS_REFUSALS.fetch_add(1, Ordering::Relaxed);
    *CENSUS_REASON
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(reason);
    LOG.store(ptr::null_mut(), Ordering::SeqCst);
}

static UNREGISTERED: AtomicU64 = AtomicU64::new(0);
static UNHOOKED: AtomicBool = AtomicBool::new(false);
static LOG: AtomicPtr<Storage> = AtomicPtr::new(ptr::null_mut());
static PENDING: AtomicPtr<Storage> = AtomicPtr::new(ptr::null_mut());
struct Retained {
    owners: Vec<Vec<Storage>>,
    scratch: Vec<native::BalanceOwner>,
    scratch_charge: Option<crate::raw_memory::Charge>,
}
impl Retained {
    fn clear(&mut self) {
        self.owners = Vec::new();
        self.scratch = Vec::new();
        self.scratch_charge = None;
    }
}
static STORAGE: Mutex<Retained> = Mutex::new(Retained {
    owners: Vec::new(),
    scratch: Vec::new(),
    scratch_charge: None,
});

struct Guard;
impl Guard {
    fn enter() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

const WRITERS: u64 = 0xffff_ffff;
const STAMP_EPOCH: u64 = 1 << 32;
fn settled(stamp: u64) -> bool {
    stamp & WRITERS == 0
}
#[derive(Default)]
struct Stamp(AtomicU64);
impl Stamp {
    fn load(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn enter(&self) -> StampWrite<'_> {
        let previous = self.0.fetch_add(1, Ordering::SeqCst);
        StampWrite {
            stamp: self,
            overlapped: !settled(previous),
            complete: false,
        }
    }
}
struct StampWrite<'a> {
    stamp: &'a Stamp,
    overlapped: bool,
    complete: bool,
}
impl Drop for StampWrite<'_> {
    fn drop(&mut self) {
        self.stamp.0.fetch_add(STAMP_EPOCH - 1, Ordering::SeqCst);
    }
}

const CONTENDED: u64 = 1 << 32;
const CERTIFIED: u64 = 1 << 33;
const CELL_EPOCH: u64 = 1 << 34;
#[derive(Default)]
struct Cell {
    state: AtomicU64,
    values: AtomicUsize,
    balance: AtomicI64,
    available: AtomicI64,
    generation: AtomicU64,
    unattributed: AtomicU64,
}
struct CellWrite<'a> {
    cell: &'a Cell,
    claimant: bool,
    published: bool,
    counted: bool,
    refusal: Option<&'static str>,
}
impl CellWrite<'_> {
    fn publish(&mut self, value: native::BalanceWrite, generation: u64, unattributed: u64) {
        let state = self.cell.state.load(Ordering::SeqCst);
        if !self.claimant || state & (WRITERS | CONTENDED) != 1 {
            self.refusal = Some(OVERLAP);
            return;
        }
        self.cell.values.store(value.values, Ordering::Relaxed);
        self.cell
            .balance
            .store(value.balance_raw, Ordering::Relaxed);
        self.cell
            .available
            .store(value.available_raw, Ordering::Relaxed);
        self.cell.generation.store(generation, Ordering::Relaxed);
        self.cell
            .unattributed
            .store(unattributed, Ordering::Relaxed);
        let certified = (state & !(WRITERS | CERTIFIED)).wrapping_add(CELL_EPOCH) | CERTIFIED;
        if self
            .cell
            .state
            .compare_exchange(state, certified, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.published = true;
        } else {
            self.refusal = Some(OVERLAP);
        }
    }
}
impl Drop for CellWrite<'_> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let _ = self
            .cell
            .state
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |state| {
                Some(if state & WRITERS == 1 {
                    (state & !(WRITERS | CONTENDED | CERTIFIED)).wrapping_add(CELL_EPOCH)
                } else {
                    (state - 1) & !CERTIFIED
                })
            });
        if self.counted {
            refuse(self.refusal.unwrap_or(UNWOUND));
        }
    }
}
impl Cell {
    fn enter(&self, counted: bool) -> CellWrite<'_> {
        let previous = self.state.fetch_add(1, Ordering::SeqCst);
        let claimant = previous & WRITERS == 0;
        if !claimant {
            self.state.fetch_or(CONTENDED, Ordering::SeqCst);
        }
        CellWrite {
            cell: self,
            claimant,
            published: false,
            counted,
            refusal: (!claimant).then_some(OVERLAP),
        }
    }
    fn read(&self, row: &native::Row, generation: u64, unattributed: u64) -> Option<i64> {
        if !settled(generation) || !settled(unattributed) {
            return None;
        }
        let state = self.state.load(Ordering::SeqCst);
        if state & (WRITERS | CONTENDED | CERTIFIED) != CERTIFIED {
            return None;
        }
        let values = self.values.load(Ordering::Relaxed);
        let balance = self.balance.load(Ordering::Relaxed);
        let available = self.available.load(Ordering::Relaxed);
        let stored_generation = self.generation.load(Ordering::Relaxed);
        let stored_unattributed = self.unattributed.load(Ordering::Relaxed);
        fence(Ordering::Acquire);
        (state == self.state.load(Ordering::Relaxed)
            && values == row.balance_values
            && balance == row.balance_raw
            && stored_generation == generation
            && stored_unattributed == unattributed)
            .then_some(available)
    }
}
struct Owner {
    identity: native::BalanceOwner,
    start: usize,
    generation: Stamp,
}
struct Storage {
    load_signal: i64,
    owners: Vec<Owner>,
    cells: Vec<Cell>,
    unattributed: Stamp,
    _charge: crate::raw_memory::Charge,
}
impl Storage {
    fn new(identities: &[native::BalanceOwner], load_signal: i64) -> io::Result<Self> {
        let mut owners = allocation::vec(identities.len())?;
        let mut count = 0;
        for identity in identities {
            owners.push(Owner {
                identity: *identity,
                start: count,
                generation: Stamp::default(),
            });
            count = allocation::add(count, identity.fields as usize)?;
        }
        let cells = allocation::default_vec(count)?;
        let mut charge = crate::raw_memory::Charge::default();
        charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&cells),
            auxiliary_vectors: crate::raw_memory::vector_bytes(&owners),
            ..Default::default()
        });
        Ok(Self {
            load_signal,
            owners,
            cells,
            unattributed: Stamp::default(),
            _charge: charge,
        })
    }
    fn owner(&self, history: usize) -> Option<&Owner> {
        self.owners
            .binary_search_by_key(&history, |owner| owner.identity.history)
            .ok()
            .map(|index| &self.owners[index])
    }
    fn cell(&self, history: usize, archetype: u32) -> Option<(&Owner, &Cell)> {
        let owner = self.owner(history)?;
        (archetype < owner.identity.fields)
            .then(|| (owner, &self.cells[owner.start + archetype as usize]))
    }
    fn fill(&self, rows: &mut [native::Row]) {
        let unattributed = self.unattributed.load();
        for row in rows.iter_mut() {
            if let Some((owner, cell)) = self.cell(row.balance_history, row.archetype)
                && owner.identity.country == row.country
            {
                let generation = owner.generation.load();
                row.available_raw = cell.read(row, generation, unattributed);
                if generation != owner.generation.load() {
                    row.available_raw = None;
                }
            }
        }
        if unattributed != self.unattributed.load() {
            for row in rows {
                row.available_raw = None;
            }
        }
    }
}

fn bound() -> [Option<&'static Storage>; 2] {
    let current = unsafe { LOG.load(Ordering::SeqCst).as_ref() };
    let pending = unsafe { PENDING.load(Ordering::SeqCst).as_ref() };
    let pending =
        pending.filter(|pending| !current.is_some_and(|current| ptr::eq(*pending, current)));
    [current, pending]
}

pub fn prepare(memory: &impl Memory, base: usize) -> io::Result<usize> {
    let Some(clock) = engine_allocation::advisory(|| cgame_state::read_clock(memory, base)) else {
        refuse_census("logistics clock preparation unavailable");
        LOG.store(ptr::null_mut(), Ordering::SeqCst);
        return Ok(0);
    };
    let state = clock.game_state as usize;
    let count = match native::owner_capacity(memory, base, state) {
        Ok(count) => count,
        Err(reason) => {
            refuse_census(reason);
            return Ok(0);
        }
    };
    let mut retained = STORAGE
        .lock()
        .map_err(|_| io::Error::other("logistics operand storage poisoned"))?;
    let Retained {
        owners: storage,
        scratch: identities,
        scratch_charge,
    } = &mut *retained;
    let current = LOG.load(Ordering::SeqCst);
    if ACTIVE.load(Ordering::SeqCst) == 0 {
        let pending = PENDING.swap(ptr::null_mut(), Ordering::SeqCst);
        storage.retain(|owner| owner.as_ptr() == current || owner.as_ptr() == pending);
    }
    allocation::resize_default(identities, count)?;
    scratch_charge
        .get_or_insert_with(crate::raw_memory::Charge::default)
        .set(crate::raw_memory::Capacity {
            auxiliary_vectors: crate::raw_memory::vector_bytes(identities),
            ..Default::default()
        });
    let len = match native::walk_balance_owners(memory, base, state, identities) {
        engine_native_memory::walk::Outcome::Rows { len } => len,
        engine_native_memory::walk::Outcome::Unavailable { reason } => {
            refuse_census(reason);
            return Ok(0);
        }
        engine_native_memory::walk::Outcome::Truncated { .. } => {
            refuse_census("logistics owner census truncated");
            return Ok(0);
        }
    };
    identities.truncate(len);
    identities.sort_unstable_by_key(|owner| owner.history);
    if identities
        .windows(2)
        .any(|pair| pair[0].history == pair[1].history)
    {
        refuse_census("logistics balance history shared by countries");
        return Ok(0);
    }
    let current = LOG.load(Ordering::SeqCst);
    if !current.is_null() {
        let current = unsafe { &*current };
        if current.load_signal == clock.play_session_started_at
            && current.owners.len() == identities.len()
            && current
                .owners
                .iter()
                .zip(identities.iter())
                .all(|(owner, identity)| owner.identity == *identity)
        {
            return Ok(current.cells.len());
        }
    }
    let next = Storage::new(identities, clock.play_session_started_at)?;
    let rows = next.cells.len();
    let mut owner = allocation::vec(1)?;
    owner.push(next);
    allocation::reserve(storage, 1)?;
    if allocation::capture_stopped() {
        return Err(allocation::oom());
    }
    storage.push(owner);
    let pointer = &raw mut storage.last_mut().unwrap()[0];
    PENDING.store(pointer, Ordering::SeqCst);
    if ACTIVE.load(Ordering::SeqCst) == 0 {
        LOG.store(pointer, Ordering::SeqCst);
    }
    Ok(rows)
}

/// Clears retained heap storage so detach/reattach begins without observations.
///
/// # Safety
/// Hooks must be removed and all callbacks/readers quiescent before release.
pub unsafe fn release() {
    debug_assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
    ENABLED.store(false, Ordering::SeqCst);
    LOG.store(ptr::null_mut(), Ordering::SeqCst);
    PENDING.store(ptr::null_mut(), Ordering::SeqCst);
    STORAGE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

pub fn fill(memory: &impl Memory, base: usize, rows: &mut [native::Row]) {
    let _guard = Guard::enter();
    for row in rows.iter_mut() {
        row.available_raw = None;
    }
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let pointer = LOG.load(Ordering::SeqCst);
    if pointer.is_null() {
        return;
    }
    let storage = unsafe { &*pointer };
    let signal = || {
        engine_sites::at(base, cgame_state::PLAY_SESSION_STARTED_AT_RVA)
            .and_then(|at| read::<i64>(memory, at))
    };
    if signal() != Some(storage.load_signal) {
        return;
    }
    storage.fill(rows);
    if LOG.load(Ordering::SeqCst) != pointer || signal() != Some(storage.load_signal) {
        for row in rows {
            row.available_raw = None;
        }
    }
}

struct HistoryChange<'a> {
    write: StampWrite<'a>,
    counted: bool,
}
impl Drop for HistoryChange<'_> {
    fn drop(&mut self) {
        if self.counted && !self.write.complete {
            refuse(UNWOUND);
        }
    }
}
/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn history_change(history: usize, target: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::LogisticsBalance);
    let _guard = Guard::enter();
    let mut changes = if ENABLED.load(Ordering::SeqCst) {
        let mut counted = true;
        bound().map(|storage| {
            let owner = storage?.owner(history)?;
            let write = owner.generation.enter();
            if counted && write.overlapped {
                refuse(OVERLAP);
            }
            let change = HistoryChange { write, counted };
            counted = false;
            Some(change)
        })
    } else {
        [None, None]
    };
    let original: unsafe extern "system-unwind" fn(usize) = unsafe { std::mem::transmute(target) };
    timing.original(|| unsafe { original(history) });
    for change in changes.iter_mut().flatten() {
        change.write.complete = true;
    }
}
unsafe extern "system-unwind" fn advance(history: usize) {
    unsafe { history_change(history, ADVANCE.load(Ordering::Acquire)) };
}
unsafe extern "system-unwind" fn clear(history: usize) {
    unsafe { history_change(history, CLEAR.load(Ordering::Acquire)) };
}

#[unsafe(naked)]
unsafe extern "system-unwind" fn daily_entry(_: usize, _: i64, _: u32) {
    core::arch::naked_asm!("mov r9, rsp", "jmp {body}", body = sym daily);
}
#[unsafe(naked)]
unsafe extern "system-unwind" fn refresh_entry(_: usize, _: i64, _: u32) {
    core::arch::naked_asm!("mov r9, rsp", "jmp {body}", body = sym refresh);
}

struct CellBinding<'a> {
    storage: &'a Storage,
    owner: &'a Owner,
    values: usize,
    generation: u64,
    unattributed: u64,
    write: CellWrite<'a>,
}
enum Binding<'a> {
    Cell(CellBinding<'a>),
    Unidentified(StampWrite<'a>),
}
fn bind<'a>(
    storages: [Option<&'a Storage>; 2],
    identity: Result<(usize, usize), &'static str>,
    archetype: u32,
    stores: impl FnOnce() -> bool,
) -> [Option<Binding<'a>>; 2] {
    match identity {
        Ok((history, values)) => {
            let mut counted = true;
            let bindings = storages.map(|storage| {
                let storage = storage?;
                let (owner, cell) = storage.cell(history, archetype)?;
                let unattributed = storage.unattributed.load();
                let generation = owner.generation.load();
                let mut write = cell.enter(counted);
                counted = false;
                if !settled(generation) {
                    write.refusal = Some(HISTORY);
                } else if !settled(unattributed) {
                    write.refusal = Some(UNIDENTIFIED);
                }
                Some(Binding::Cell(CellBinding {
                    storage,
                    owner,
                    values,
                    generation,
                    unattributed,
                    write,
                }))
            });
            if counted {
                UNREGISTERED.fetch_add(1, Ordering::Relaxed);
            }
            bindings
        }
        Err(reason) => {
            refuse(reason);
            if !stores() {
                return [None, None];
            }
            UNATTRIBUTED.fetch_add(1, Ordering::Relaxed);
            storages.map(|storage| Some(Binding::Unidentified(storage?.unattributed.enter())))
        }
    }
}
impl CellBinding<'_> {
    fn finish(
        &mut self,
        signal: Option<i64>,
        value: &mut dyn FnMut() -> Result<native::BalanceWrite, &'static str>,
    ) {
        if self.write.refusal.is_some() {
            return;
        }
        let refusal = if signal != Some(self.storage.load_signal) {
            UNWOUND
        } else if self.owner.generation.load() != self.generation {
            HISTORY
        } else if self.storage.unattributed.load() != self.unattributed {
            UNIDENTIFIED
        } else {
            match value() {
                Ok(value)
                    if value.history == self.owner.identity.history
                        && value.values == self.values =>
                {
                    self.write
                        .publish(value, self.generation, self.unattributed);
                    return;
                }
                Ok(_) => HISTORY,
                Err(reason) => reason,
            }
        };
        self.write.refusal = Some(refusal);
    }
}
/// # Safety
/// The arguments are ones the hooked balance write's original accepts, with `offset` that call
/// site's, and every object they reach stays valid for the reads that site makes.
unsafe fn write(record: usize, balance: i64, archetype: u32, stack: usize, offset: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::LogisticsBalance);
    let _guard = Guard::enter();
    let base = BASE.load(Ordering::Acquire);
    let memory = unsafe { LiveMemory::new() };
    let signal = || {
        engine_sites::at(base, cgame_state::PLAY_SESSION_STARTED_AT_RVA)
            .and_then(|at| read::<i64>(&memory, at))
    };
    let mut bindings = match ENABLED
        .load(Ordering::SeqCst)
        .then(bound)
        .filter(|storages| storages.iter().any(Option::is_some))
    {
        Some(storages) => bind(
            storages,
            native::balance_identity(&memory, record, archetype),
            archetype,
            || native::balance_write_stores(&memory, record, archetype).unwrap_or(true),
        ),
        None => [None, None],
    };
    let original: unsafe extern "system-unwind" fn(usize, i64, u32) =
        unsafe { std::mem::transmute(WRITE.load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(record, balance, archetype) });
    let signal = signal();
    let mut read = None;
    let mut value = || {
        *read.get_or_insert_with(|| {
            native::balance_write(&memory, record, balance, archetype, stack, offset)
        })
    };
    for binding in bindings.iter_mut().flatten() {
        match binding {
            Binding::Cell(binding) => binding.finish(signal, &mut value),
            Binding::Unidentified(write) => write.complete = true,
        }
    }
}
unsafe extern "system-unwind" fn daily(record: usize, balance: i64, archetype: u32, stack: usize) {
    unsafe {
        write(
            record,
            balance,
            archetype,
            stack,
            native::BALANCE_CALLS[0].1,
        )
    };
}
unsafe extern "system-unwind" fn refresh(
    record: usize,
    balance: i64,
    archetype: u32,
    stack: usize,
) {
    unsafe {
        write(
            record,
            balance,
            archetype,
            stack,
            native::BALANCE_CALLS[1].1,
        )
    };
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        BASE.store(base, Ordering::Release);
        let mut hooks = Vec::new();
        let located = || -> Result<_, &'static str> {
            let call = |key| {
                engine_sites::hook(base, key)
                    .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
            };
            let target = |key| engine_sites::hook(base, key).map(|code| code.address);
            let [daily, refresh] = native::BALANCE_CALLS.map(|(key, _)| key);
            let [first, second, third] = native::HISTORY_CALLS.map(|(key, _)| key);
            Ok((
                [call(daily)?, call(refresh)?],
                [call(first)?, call(second)?, call(third)?],
                [
                    target(native::BALANCE_WRITE_RVA)?,
                    target(native::HISTORY_CALLS[0].1)?,
                    target(native::HISTORY_CALLS[1].1)?,
                ],
            ))
        };
        let (balance, history, [write, advance_target, clear_target]) = match located() {
            Ok(located) => located,
            Err(cause) => {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Sentinel,
                    None,
                    cause,
                );
                UNHOOKED.store(true, Ordering::Relaxed);
                return Ok(Self { hooks });
            }
        };
        WRITE.store(write, Ordering::Release);
        ADVANCE.store(advance_target, Ordering::Release);
        CLEAR.store(clear_target, Ordering::Release);
        UNHOOKED.store(false, Ordering::Relaxed);
        for (index, (at, sentinel)) in balance.into_iter().enumerate() {
            let callback = [
                daily_entry as *const () as usize,
                refresh_entry as *const () as usize,
            ][index];
            hooks.push(
                unsafe { Hook::new_call(at, callback, sentinel) }
                    .map_err(|_| "logistics balance CALL construction failed")?,
            );
        }
        for ((at, sentinel), (_, target)) in history.into_iter().zip(native::HISTORY_CALLS) {
            let callback = if target == native::HISTORY_CALLS[0].1 {
                advance as *const () as usize
            } else {
                clear as *const () as usize
            };
            hooks.push(
                unsafe { Hook::new_call(at, callback, sentinel) }
                    .map_err(|_| "logistics history CALL construction failed")?,
            );
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        for step in 0..self.hooks.len() {
            let index = if enabled {
                step
            } else {
                self.hooks.len() - step - 1
            };
            if !self.hooks[index]
                .set_enabled(enabled)
                .map_err(|_| "logistics balance CALL state change failed")?
            {
                return Ok(false);
            }
        }
        ENABLED.store(
            enabled
                && self.hooks.len() == native::BALANCE_CALLS.len() + native::HISTORY_CALLS.len(),
            Ordering::SeqCst,
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn value(
        history: usize,
        values: usize,
        balance_raw: i64,
        available_raw: i64,
    ) -> native::BalanceWrite {
        native::BalanceWrite {
            history,
            values,
            archetype: 0,
            balance_raw,
            available_raw,
        }
    }
    fn certify(cell: &Cell, value: native::BalanceWrite, generation: u64) {
        let mut write = cell.enter(false);
        write.publish(value, generation, 0);
        assert!(write.published);
    }
    fn row(
        country: u32,
        history: usize,
        values: usize,
        archetype: u32,
        balance_raw: i64,
    ) -> native::Row {
        native::Row {
            country,
            archetype,
            balance_raw,
            balance_history: history,
            balance_values: values,
            ..Default::default()
        }
    }
    fn native_write(
        storages: [Option<&Storage>; 2],
        history: usize,
        values: usize,
        archetype: u32,
        written: native::BalanceWrite,
    ) {
        let mut bindings = bind(storages, Ok((history, values)), archetype, || true);
        finish(&mut bindings, written);
    }
    fn finish(bindings: &mut [Option<Binding<'_>>; 2], written: native::BalanceWrite) {
        for binding in bindings.iter_mut().flatten() {
            match binding {
                Binding::Cell(binding) => binding.finish(Some(SIGNAL), &mut || Ok(written)),
                Binding::Unidentified(write) => write.complete = true,
            }
        }
    }
    const SIGNAL: i64 = 123;
    const A: usize = 0x100;
    const B: usize = 0x200;
    fn two_countries() -> Storage {
        Storage::new(
            &[
                native::BalanceOwner {
                    country: 1,
                    history: A,
                    fields: 2,
                },
                native::BalanceOwner {
                    country: 2,
                    history: B,
                    fields: 1,
                },
            ],
            SIGNAL,
        )
        .unwrap()
    }
    fn certified_rows(storage: &Storage) -> [native::Row; 3] {
        native_write(
            [Some(storage), None],
            A,
            0xa000,
            0,
            value(A, 0xa000, -1, 10),
        );
        native_write(
            [Some(storage), None],
            A,
            0xa000,
            1,
            value(A, 0xa000, -2, 20),
        );
        native_write(
            [Some(storage), None],
            B,
            0xb000,
            0,
            value(B, 0xb000, -3, 30),
        );
        let rows = [
            row(1, A, 0xa000, 0, -1),
            row(1, A, 0xa000, 1, -2),
            row(2, B, 0xb000, 0, -3),
        ];
        assert_eq!(available(storage, rows), [Some(10), Some(20), Some(30)]);
        rows
    }
    fn available(storage: &Storage, mut rows: [native::Row; 3]) -> [Option<i64>; 3] {
        storage.fill(&mut rows);
        rows.map(|row| row.available_raw)
    }

    #[test]
    fn same_balance_in_another_record_cannot_reuse_available() {
        let cell = Cell::default();
        let mut row = row(0, 1, 100, 0, -10);
        assert_eq!(cell.read(&row, 0, 0), None);
        certify(&cell, value(1, 100, -10, 30), 0);
        assert_eq!(cell.read(&row, 0, 0), Some(30));
        row.balance_values = 200;
        assert_eq!(cell.read(&row, 0, 0), None);
        row.balance_values = 100;
        row.balance_raw = -11;
        assert_eq!(cell.read(&row, 0, 0), None);
        certify(&cell, value(1, 100, -11, 5), 0);
        assert_eq!(cell.read(&row, 0, 0), Some(5));
        let write = cell.enter(false);
        assert_eq!(cell.read(&row, 0, 0), None);
        drop(write);
        assert_eq!(cell.read(&row, 0, 0), None);
    }

    #[test]
    fn overlapping_writers_of_one_cell_certify_neither_and_history_epochs_do_not_revive_zero() {
        let cell = Cell::default();
        let row = row(0, 1, 100, 0, 0);
        let mut first = cell.enter(false);
        let second = cell.enter(false);
        assert_eq!(cell.read(&row, 0, 0), None);
        first.publish(value(1, 100, 0, 17), 0, 0);
        assert!(!first.published);
        drop(second);
        assert_eq!(cell.read(&row, 0, 0), None);
        drop(first);
        assert_eq!(cell.read(&row, 0, 0), None);
        certify(&cell, value(1, 100, 0, 17), 0);
        assert_eq!(cell.read(&row, 0, 0), Some(17));
        assert_eq!(cell.read(&row, STAMP_EPOCH, 0), None);
        assert_eq!(cell.read(&row, 30 * STAMP_EPOCH, 0), None);
        assert_eq!(cell.read(&row, 1, 0), None);
        certify(&cell, value(1, 100, 0, 0), 30 * STAMP_EPOCH);
        assert_eq!(cell.read(&row, 30 * STAMP_EPOCH, 0), Some(0));
        drop(cell.enter(false));
        assert_eq!(cell.read(&row, 30 * STAMP_EPOCH, 0), None);
    }

    #[test]
    fn one_refused_cell_leaves_every_other_country_and_archetype_certified() {
        let storage = two_countries();
        let rows = certified_rows(&storage);
        let mut first = bind([Some(&storage), None], Ok((A, 0xa000)), 0, || true);
        let mut second = bind([Some(&storage), None], Ok((A, 0xa000)), 0, || true);
        finish(&mut first, value(A, 0xa000, -1, 11));
        finish(&mut second, value(A, 0xa000, -1, 12));
        drop((first, second));
        assert_eq!(available(&storage, rows), [None, Some(20), Some(30)]);
        let mut unreadable = bind([Some(&storage), None], Ok((B, 0xb000)), 0, || true);
        for binding in unreadable.iter_mut().flatten() {
            if let Binding::Cell(binding) = binding {
                binding.finish(Some(SIGNAL), &mut || {
                    Err("logistics available operand unreadable")
                });
            }
        }
        drop(unreadable);
        assert_eq!(available(&storage, rows), [None, Some(20), None]);
        native_write(
            [Some(&storage), None],
            A,
            0xa000,
            0,
            value(A, 0xa000, -1, 13),
        );
        assert_eq!(available(&storage, rows), [Some(13), Some(20), None]);
    }

    #[test]
    fn history_change_withdraws_only_its_own_country_cells() {
        let storage = two_countries();
        let rows = certified_rows(&storage);
        let owner = storage.owner(A).unwrap();
        let change = owner.generation.enter();
        assert!(!change.overlapped);
        let mut changing = bind([Some(&storage), None], Ok((A, 0xa000)), 1, || true);
        native_write(
            [Some(&storage), None],
            B,
            0xb000,
            0,
            value(B, 0xb000, -3, 31),
        );
        assert_eq!(available(&storage, rows), [None, None, Some(31)]);
        drop(change);
        finish(&mut changing, value(A, 0xa000, -2, 21));
        drop(changing);
        assert_eq!(available(&storage, rows), [None, None, Some(31)]);
        native_write(
            [Some(&storage), None],
            A,
            0xa000,
            1,
            value(A, 0xa000, -2, 22),
        );
        assert_eq!(available(&storage, rows), [None, Some(22), Some(31)]);
    }

    #[test]
    fn unidentified_store_withdraws_every_cell_and_a_native_no_op_withdraws_none() {
        let storage = two_countries();
        let rows = certified_rows(&storage);
        let skipped = bind(
            [Some(&storage), None],
            Err("logistics balance write index outside record"),
            7,
            || false,
        );
        assert!(skipped.iter().all(Option::is_none));
        assert_eq!(available(&storage, rows), [Some(10), Some(20), Some(30)]);
        let mut unknown = bind(
            [Some(&storage), None],
            Err("logistics balance write record unreadable"),
            0,
            || true,
        );
        let mut overlapped = bind([Some(&storage), None], Ok((B, 0xb000)), 0, || true);
        finish(&mut unknown, value(0, 0, 0, 0));
        drop(unknown);
        finish(&mut overlapped, value(B, 0xb000, -3, 32));
        drop(overlapped);
        assert_eq!(available(&storage, rows), [None, None, None]);
        native_write(
            [Some(&storage), None],
            B,
            0xb000,
            0,
            value(B, 0xb000, -3, 33),
        );
        assert_eq!(available(&storage, rows), [None, None, Some(33)]);
    }

    #[test]
    fn native_store_predicate_follows_the_signed_record_count() {
        let mut bytes = [0_u8; 0x20];
        bytes[0x1c..0x20].copy_from_slice(&3_i32.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(native::balance_write_stores(&memory, 0x1010, 2), Some(true));
        assert_eq!(
            native::balance_write_stores(&memory, 0x1010, 3),
            Some(false)
        );
        assert_eq!(
            native::balance_write_stores(&memory, 0x1010, u32::MAX),
            Some(true)
        );
        assert_eq!(native::balance_write_stores(&memory, 0x2000, 0), None);
    }

    #[test]
    fn write_during_staging_publication_binds_both_storages_without_global_refusal() {
        let old = two_countries();
        let new = two_countries();
        let old_rows = certified_rows(&old);
        let mut early = bind([Some(&old), Some(&new)], Ok((A, 0xa000)), 0, || true);
        let mut late = bind([Some(&new), None], Ok((A, 0xa000)), 0, || true);
        finish(&mut late, value(A, 0xa000, -1, 40));
        drop(late);
        finish(&mut early, value(A, 0xa000, -1, 41));
        drop(early);
        let mut rows = [row(1, A, 0xa000, 0, -1), row(2, B, 0xb000, 0, -3)];
        new.fill(&mut rows);
        assert_eq!(rows.map(|row| row.available_raw), [None, None]);
        assert_eq!(available(&old, old_rows), [Some(41), Some(20), Some(30)]);
        native_write([Some(&new), None], B, 0xb000, 0, value(B, 0xb000, -3, 42));
        new.fill(&mut rows);
        assert_eq!(rows.map(|row| row.available_raw), [None, Some(42)]);
    }

    #[test]
    fn bound_storages_are_the_published_and_publishing_ones() {
        static EXCLUSIVE: Mutex<()> = Mutex::new(());
        let _guard = EXCLUSIVE.lock().unwrap();
        let mut old = two_countries();
        let mut new = two_countries();
        LOG.store(&raw mut old, Ordering::SeqCst);
        PENDING.store(&raw mut new, Ordering::SeqCst);
        let [current, pending] = bound();
        assert!(current.is_some_and(|storage| ptr::eq(storage, &raw const old)));
        assert!(pending.is_some_and(|storage| ptr::eq(storage, &raw const new)));
        PENDING.store(&raw mut old, Ordering::SeqCst);
        assert!(bound()[1].is_none());
        let mut across = bind([Some(&new), None], Ok((B, 0xb000)), 0, || true);
        for binding in across.iter_mut().flatten() {
            if let Binding::Cell(binding) = binding {
                binding.finish(Some(SIGNAL + 1), &mut || Ok(value(B, 0xb000, -3, 9)));
            }
        }
        drop(across);
        let mut rows = [row(2, B, 0xb000, 0, -3)];
        new.fill(&mut rows);
        assert_eq!(rows[0].available_raw, None);
        LOG.store(ptr::null_mut(), Ordering::SeqCst);
        PENDING.store(ptr::null_mut(), Ordering::SeqCst);
    }

    #[test]
    fn quiescent_detach_drops_storage_and_reattach_has_no_observation() {
        let identities = [native::BalanceOwner {
            country: 9,
            history: 100,
            fields: 1,
        }];
        let storage = Storage::new(&identities, 123).unwrap();
        certify(&storage.cells[0], value(100, 200, 0, 17), 0);
        let mut retained = Retained {
            owners: vec![vec![storage]],
            scratch: identities.to_vec(),
            scratch_charge: None,
        };
        retained.clear();
        assert_eq!(retained.owners.capacity(), 0);
        assert_eq!(retained.scratch.capacity(), 0);
        let fresh = Storage::new(&identities, 123).unwrap();
        assert_eq!(fresh.cells[0].read(&row(9, 100, 200, 0, 0), 0, 0), None);
    }

    #[test]
    fn balance_storage_finds_each_owner_field_cell() {
        let identities: Vec<_> = (0..5)
            .map(|slot| native::BalanceOwner {
                country: slot * 7 + 1,
                history: slot as usize * 0x100 + 1,
                fields: 37,
            })
            .collect();
        let storage = Storage::new(&identities, 123).unwrap();
        assert_eq!(storage.cells.len(), 5 * 37);
        let (owner, _) = storage.cell(4 * 0x100 + 1, 36).unwrap();
        assert_eq!(owner.identity.country, 4 * 7 + 1);
        assert!(storage.cell(owner.identity.history, 37).is_none());
        assert!(Storage::new(&[], 123).unwrap().cells.is_empty());
    }
}
