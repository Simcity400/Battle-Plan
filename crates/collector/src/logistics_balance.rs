//! Retains the actual available-stock operand of each native balance write.
//! Five CALL replacements preserve the caller's frame and native continuation.
//! No allocation, container walk, game getter, or cache refresh runs in a hook.
use crate::hook::Hook;
use engine::{
    Memory, allocation, cgame_state, clogistics_status as native, memory::LiveMemory, read,
};
use std::{
    io,
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

static BASE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static ADMISSION: AtomicBool = AtomicBool::new(true);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static FAILED_READS: AtomicU64 = AtomicU64::new(0);
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
    "native mutation during staging publication",
    "foreign unwind or staging epoch changed during native write",
    "unclassified native operand refusal",
];
fn refuse(reason: &'static str) {
    let index = REASONS
        .iter()
        .position(|candidate| *candidate == reason)
        .unwrap_or(REASONS.len() - 1);
    REFUSALS[index].fetch_add(1, Ordering::Relaxed);
    FAILED_READS.fetch_add(1, Ordering::SeqCst);
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
// Published storage never moves or dies before callback quiescence. Retention
// growth is checked before publication, including a failed replacement.
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

// The test resumes publication between these observations. Production uses
// an empty inlined step; admission must precede the storage pointer.
#[inline]
fn staging_snapshot(
    admission: &AtomicBool,
    log: &AtomicPtr<Storage>,
    between_reads: impl FnOnce(),
) -> (bool, *mut Storage) {
    let admitted = admission.load(Ordering::SeqCst);
    between_reads();
    (admitted, log.load(Ordering::SeqCst))
}

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

#[derive(Default)]
struct Cell {
    sequence: AtomicU64,
    values: AtomicUsize,
    balance: AtomicI64,
    available: AtomicI64,
    generation: AtomicU64,
}
struct CellWrite<'a> {
    cell: &'a Cell,
    sequence: u64,
    complete: bool,
    refusal: &'static str,
}
impl CellWrite<'_> {
    fn publish(mut self, value: native::BalanceWrite, generation: u64) {
        self.cell.values.store(value.values, Ordering::Relaxed);
        self.cell
            .balance
            .store(value.balance_raw, Ordering::Relaxed);
        self.cell
            .available
            .store(value.available_raw, Ordering::Relaxed);
        self.cell.generation.store(generation, Ordering::Relaxed);
        self.cell
            .sequence
            .store(self.sequence.wrapping_add(2), Ordering::SeqCst);
        self.complete = true;
    }
}
impl Drop for CellWrite<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.cell.sequence.store(0, Ordering::SeqCst);
            refuse(self.refusal);
        }
    }
}
impl Cell {
    // Claimed before the native store, never after it: even equal balances
    // cannot let an overlapping older writer replace the newer availability.
    fn begin(&self) -> Option<CellWrite<'_>> {
        let sequence = self.sequence.load(Ordering::SeqCst);
        if sequence & 1 != 0
            || self
                .sequence
                .compare_exchange(
                    sequence,
                    sequence.wrapping_add(1),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_err()
        {
            return None;
        }
        Some(CellWrite {
            cell: self,
            sequence,
            complete: false,
            refusal: "foreign unwind or staging epoch changed during native write",
        })
    }
    fn read(&self, row: &native::Row, generation: u64) -> Option<i64> {
        let sequence = self.sequence.load(Ordering::SeqCst);
        if sequence == 0 || sequence & 1 != 0 || generation & 1 != 0 {
            return None;
        }
        let values = self.values.load(Ordering::Relaxed);
        let balance = self.balance.load(Ordering::Relaxed);
        let available = self.available.load(Ordering::Relaxed);
        let stored_generation = self.generation.load(Ordering::Relaxed);
        (sequence == self.sequence.load(Ordering::SeqCst)
            && values == row.balance_values
            && balance == row.balance_raw
            && stored_generation == generation)
            .then_some(available)
    }
}
struct Owner {
    identity: native::BalanceOwner,
    start: usize,
    generation: AtomicU64,
}
struct Storage {
    load_signal: i64,
    failures: u64,
    owners: Vec<Owner>,
    cells: Vec<Cell>,
    _charge: crate::raw_memory::Charge,
}
impl Storage {
    fn new(
        identities: &[native::BalanceOwner],
        load_signal: i64,
        failures: u64,
    ) -> io::Result<Self> {
        let mut owners = allocation::vec(identities.len())?;
        let mut count = 0;
        for identity in identities {
            owners.push(Owner {
                identity: *identity,
                start: count,
                generation: AtomicU64::new(0),
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
            failures,
            owners,
            cells,
            _charge: charge,
        })
    }
    fn cell(&self, history: usize, archetype: u32) -> Option<(&Owner, &Cell)> {
        let index = self
            .owners
            .binary_search_by_key(&history, |owner| owner.identity.history)
            .ok()?;
        let owner = &self.owners[index];
        (archetype < owner.identity.fields)
            .then(|| (owner, &self.cells[owner.start + archetype as usize]))
    }
}

/// Off-thread checked acquisition; returns exact row capacity, including zero.
/// A load or failed operand starts fresh storage. Current supply/convoy values
/// are never used to backfill cells which no native balance write has reached.
pub fn prepare(memory: &impl Memory, base: usize) -> io::Result<usize> {
    let Some(clock) = engine::allocation::advisory(|| cgame_state::read_clock(memory, base)) else {
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
        storage.retain(|owner| owner.as_ptr() == current);
    }
    allocation::resize_default(identities, count)?;
    scratch_charge
        .get_or_insert_with(crate::raw_memory::Charge::default)
        .set(crate::raw_memory::Capacity {
            auxiliary_vectors: crate::raw_memory::vector_bytes(identities),
            ..Default::default()
        });
    let len = match native::walk_balance_owners(memory, base, state, identities) {
        engine::walk::Outcome::Rows { len } => len,
        engine::walk::Outcome::Unavailable { reason } => {
            refuse_census(reason);
            return Ok(0);
        }
        engine::walk::Outcome::Truncated { .. } => {
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
    let failures = FAILED_READS.load(Ordering::SeqCst);
    let current = LOG.load(Ordering::SeqCst);
    if !current.is_null() {
        let current = unsafe { &*current };
        if current.load_signal == clock.play_session_started_at
            && current.failures == failures
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
    let next = Storage::new(identities, clock.play_session_started_at, failures)?;
    let rows = next.cells.len();
    let mut owner = allocation::vec(1)?;
    owner.push(next);
    allocation::reserve(storage, 1)?;
    if allocation::capture_stopped() {
        return Err(allocation::oom());
    }
    storage.push(owner);
    let pointer = &mut storage.last_mut().unwrap()[0] as *mut Storage;
    // A closed-gate native mutation latches a refusal before calling the
    // game. Its failure epoch predates this offer, so even a late entrant
    // cannot make this fresh staging certify overlapping old native work.
    ADMISSION.store(false, Ordering::SeqCst);
    if ACTIVE.load(Ordering::SeqCst) == 0 {
        LOG.store(pointer, Ordering::SeqCst);
    }
    ADMISSION.store(true, Ordering::SeqCst);
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
    let (admitted, pointer) = staging_snapshot(&ADMISSION, &LOG, || {});
    if !admitted {
        return;
    }
    if pointer.is_null() {
        return;
    }
    let storage = unsafe { &*pointer };
    let signal = || {
        base.checked_add(cgame_state::PLAY_SESSION_STARTED_AT_RVA)
            .and_then(|at| read::<i64>(memory, at))
    };
    if signal() != Some(storage.load_signal)
        || storage.failures != FAILED_READS.load(Ordering::SeqCst)
    {
        return;
    }
    for row in rows.iter_mut() {
        if let Some((owner, cell)) = storage.cell(row.balance_history, row.archetype) {
            let generation = owner.generation.load(Ordering::SeqCst);
            if owner.identity.country == row.country {
                row.available_raw = cell.read(row, generation);
                if generation != owner.generation.load(Ordering::SeqCst) {
                    row.available_raw = None;
                }
            }
        }
    }
    if LOG.load(Ordering::SeqCst) != pointer
        || signal() != Some(storage.load_signal)
        || storage.failures != FAILED_READS.load(Ordering::SeqCst)
    {
        for row in rows {
            row.available_raw = None;
        }
    }
}

struct HistoryChange<'a> {
    owner: &'a Owner,
    complete: bool,
}
impl Drop for HistoryChange<'_> {
    fn drop(&mut self) {
        self.owner.generation.fetch_add(1, Ordering::SeqCst);
        if !self.complete {
            refuse("foreign unwind or staging epoch changed during native write");
        }
    }
}
unsafe fn history_change(history: usize, target: usize) {
    let _guard = Guard::enter();
    let (admitted, pointer) = staging_snapshot(&ADMISSION, &LOG, || {});
    if ENABLED.load(Ordering::SeqCst) && !admitted {
        refuse("native mutation during staging publication");
    }
    let mut change = if ENABLED.load(Ordering::SeqCst) && admitted && !pointer.is_null() {
        let storage = unsafe { &*pointer };
        storage
            .owners
            .binary_search_by_key(&history, |owner| owner.identity.history)
            .ok()
            .map(|index| {
                let owner = &storage.owners[index];
                if owner.generation.fetch_add(1, Ordering::SeqCst) & 1 != 0 {
                    refuse("overlapping native balance writes");
                }
                HistoryChange {
                    owner,
                    complete: false,
                }
            })
    } else {
        None
    };
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(BASE.load(Ordering::Acquire) + target) };
    unsafe { original(history) };
    if let Some(change) = &mut change {
        change.complete = true;
    }
    drop(change);
}
unsafe extern "system-unwind" fn advance(history: usize) {
    unsafe { history_change(history, native::HISTORY_CALLS[0].2) };
}
unsafe extern "system-unwind" fn clear(history: usize) {
    unsafe { history_change(history, native::HISTORY_CALLS[1].2) };
}

// No stack or nonvolatile register changes: Rust bodies have normal unwind
// metadata and return directly to the verified native CALL continuation.
#[unsafe(naked)]
unsafe extern "system-unwind" fn daily_entry(_: usize, _: i64, _: u32) {
    core::arch::naked_asm!("mov r9, rsp", "jmp {body}", body = sym daily);
}
#[unsafe(naked)]
unsafe extern "system-unwind" fn refresh_entry(_: usize, _: i64, _: u32) {
    core::arch::naked_asm!("mov r9, rsp", "jmp {body}", body = sym refresh);
}
unsafe fn write(record: usize, balance: i64, archetype: u32, stack: usize, offset: usize) {
    let _guard = Guard::enter();
    let base = BASE.load(Ordering::Acquire);
    let memory = unsafe { LiveMemory::new() };
    // Acquire both storage epoch and per-cell exclusion before native code.
    // Old callbacks may not bind their operands to a newly published staging.
    let (admitted, pointer) = staging_snapshot(&ADMISSION, &LOG, || {});
    let signal = || {
        base.checked_add(cgame_state::PLAY_SESSION_STARTED_AT_RVA)
            .and_then(|at| read::<i64>(&memory, at))
    };
    if ENABLED.load(Ordering::SeqCst) && !admitted {
        refuse("native mutation during staging publication");
    }
    let pending = if ENABLED.load(Ordering::SeqCst) && admitted && !pointer.is_null() {
        let storage = unsafe { &*pointer };
        if signal() != Some(storage.load_signal) {
            None
        } else {
            match native::balance_identity(&memory, record, archetype) {
                Ok((history, values)) => match storage.cell(history, archetype) {
                    Some((owner, cell)) => {
                        let generation = owner.generation.load(Ordering::SeqCst);
                        if generation & 1 != 0 {
                            refuse("native history changed during balance write");
                            None
                        } else {
                            match cell.begin() {
                                Some(guard) => Some((storage, owner, generation, values, guard)),
                                None => {
                                    refuse("overlapping native balance writes");
                                    None
                                }
                            }
                        }
                    }
                    None => {
                        UNREGISTERED.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                },
                Err(reason) => {
                    refuse(reason);
                    None
                }
            }
        }
    } else {
        None
    };
    let original: unsafe extern "system-unwind" fn(usize, i64, u32) =
        unsafe { std::mem::transmute(base + native::BALANCE_WRITE_RVA) };
    unsafe { original(record, balance, archetype) };
    if let Some((storage, owner, generation, values, mut guard)) = pending {
        if LOG.load(Ordering::SeqCst) == pointer
            && signal() == Some(storage.load_signal)
            && generation == owner.generation.load(Ordering::SeqCst)
        {
            match native::balance_write(&memory, record, balance, archetype, stack, offset) {
                Ok(value) if value.history == owner.identity.history && value.values == values => {
                    guard.publish(value, generation)
                }
                Ok(_) => guard.refusal = "native history changed during balance write",
                Err(reason) => guard.refusal = reason,
            }
        } else if generation != owner.generation.load(Ordering::SeqCst) {
            guard.refusal = "native history changed during balance write";
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
            native::BALANCE_CALLS[0].2,
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
            native::BALANCE_CALLS[1].2,
        )
    };
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        BASE.store(base, Ordering::Release);
        let mut hooks = Vec::new();
        if native::BALANCE_CALLS
            .iter()
            .chain(native::HISTORY_CALLS.iter())
            .any(|(rva, sentinel, _)| {
                base.checked_add(*rva)
                    .and_then(|site| read::<[u8; 16]>(memory, site))
                    != Some(*sentinel)
            })
        {
            UNHOOKED.store(true, Ordering::Relaxed);
            return Ok(Self { hooks });
        }
        UNHOOKED.store(false, Ordering::Relaxed);
        for (index, (rva, sentinel, _)) in native::BALANCE_CALLS.iter().enumerate() {
            let callback = [
                daily_entry as *const () as usize,
                refresh_entry as *const () as usize,
            ][index];
            hooks.push(
                unsafe { Hook::new_call(base + rva, callback, *sentinel) }
                    .map_err(|_| "logistics balance CALL construction failed")?,
            );
        }
        for (rva, sentinel, target) in native::HISTORY_CALLS {
            let callback = if target == native::HISTORY_CALLS[0].2 {
                advance as *const () as usize
            } else {
                clear as *const () as usize
            };
            hooks.push(
                unsafe { Hook::new_call(base + rva, callback, sentinel) }
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

pub fn status() -> impl serde::Serialize {
    let census_reason = *CENSUS_REASON
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    crate::status::status!({"enabled":ENABLED.load(Ordering::SeqCst), "unhooked":UNHOOKED.load(Ordering::Relaxed),
        "failed_operands":FAILED_READS.load(Ordering::Relaxed), "unregistered_writes":UNREGISTERED.load(Ordering::Relaxed),
        "census_refusals":CENSUS_REFUSALS.load(Ordering::Relaxed), "last_census_refusal":census_reason,
        "refusals":crate::status::sequence(REFUSALS.iter().zip(REASONS).filter_map(|(count, reason)| {
            let count = count.load(Ordering::Relaxed);
            (count != 0).then_some(crate::status::status!({"reason":reason,"count":count}))
        })),
        "scope":"actual availability at the last matching native balance write; absent before observation or after refusal"})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_entry_during_closed_publication_cannot_admit_an_old_storage_pointer() {
        let mut old = std::mem::MaybeUninit::<Storage>::uninit();
        let mut new = std::mem::MaybeUninit::<Storage>::uninit();
        let old = old.as_mut_ptr();
        let new = new.as_mut_ptr();
        let log = AtomicPtr::new(old);
        // Publisher has closed admission and observed zero active callbacks.
        let admission = AtomicBool::new(false);
        let (admitted, pointer) = staging_snapshot(&admission, &log, || {
            // Resume the publisher while the late entrant is between reads.
            log.store(new, Ordering::SeqCst);
            admission.store(true, Ordering::SeqCst);
        });
        assert!(!admitted);
        assert_eq!(pointer, new);
        assert_eq!(staging_snapshot(&admission, &log, || {}), (true, new));
    }
    #[test]
    fn same_balance_in_another_record_cannot_reuse_available() {
        let cell = Cell::default();
        let mut row = native::Row {
            balance_values: 100,
            balance_raw: -10,
            ..Default::default()
        };
        assert_eq!(cell.read(&row, 0), None);
        cell.begin().unwrap().publish(
            native::BalanceWrite {
                history: 1,
                values: 100,
                archetype: 0,
                balance_raw: -10,
                available_raw: 30,
            },
            0,
        );
        assert_eq!(cell.read(&row, 0), Some(30));
        row.balance_values = 200;
        assert_eq!(cell.read(&row, 0), None);
        row.balance_values = 100;
        row.balance_raw = -11;
        assert_eq!(cell.read(&row, 0), None);
        cell.begin().unwrap().publish(
            native::BalanceWrite {
                history: 1,
                values: 100,
                archetype: 0,
                balance_raw: -11,
                available_raw: 5,
            },
            0,
        );
        assert_eq!(cell.read(&row, 0), Some(5));
        cell.sequence.fetch_add(1, Ordering::SeqCst);
        assert_eq!(cell.read(&row, 0), None);
    }
    #[test]
    fn equal_overlapping_store_cannot_replace_an_older_claim_and_resets_do_not_revive_zero() {
        let cell = Cell::default();
        let row = native::Row {
            balance_values: 100,
            balance_raw: 0,
            ..Default::default()
        };
        let first = cell.begin().unwrap();
        assert!(cell.begin().is_none());
        assert_eq!(cell.read(&row, 0), None);
        first.publish(
            native::BalanceWrite {
                history: 1,
                values: 100,
                archetype: 0,
                balance_raw: 0,
                available_raw: 17,
            },
            0,
        );
        assert_eq!(cell.read(&row, 0), Some(17));
        // One reset, or thirty advances returning to the identical pointers.
        assert_eq!(cell.read(&row, 2), None);
        assert_eq!(cell.read(&row, 60), None);
        assert_eq!(cell.read(&row, 1), None);
        cell.begin().unwrap().publish(
            native::BalanceWrite {
                history: 1,
                values: 100,
                archetype: 0,
                balance_raw: 0,
                available_raw: 0,
            },
            60,
        );
        assert_eq!(cell.read(&row, 60), Some(0));
        // An unwound/refused native write destroys its old certification.
        drop(cell.begin().unwrap());
        assert_eq!(cell.read(&row, 60), None);
    }
    #[test]
    fn quiescent_detach_drops_storage_and_reattach_has_no_observation() {
        let identities = [native::BalanceOwner {
            country: 9,
            history: 100,
            fields: 1,
        }];
        let storage = Storage::new(&identities, 123, 0).unwrap();
        storage.cells[0].begin().unwrap().publish(
            native::BalanceWrite {
                history: 100,
                values: 200,
                archetype: 0,
                balance_raw: 0,
                available_raw: 17,
            },
            0,
        );
        let mut retained = Retained {
            owners: vec![vec![storage]],
            scratch: identities.to_vec(),
            scratch_charge: None,
        };
        retained.clear();
        assert_eq!(retained.owners.capacity(), 0);
        assert_eq!(retained.scratch.capacity(), 0);
        let fresh = Storage::new(&identities, 123, 0).unwrap();
        assert_eq!(
            fresh.cells[0].read(
                &native::Row {
                    balance_values: 200,
                    ..Default::default()
                },
                0
            ),
            None
        );
    }
    #[test]
    fn live_five_thousand_country_and_mod_archetype_capacity_has_no_chosen_cap() {
        let identities: Vec<_> = (0..5000)
            .map(|slot| native::BalanceOwner {
                country: slot * 7 + 1,
                history: slot as usize * 0x100 + 1,
                fields: 37,
            })
            .collect();
        let storage = Storage::new(&identities, 123, 0).unwrap();
        assert_eq!(storage.cells.len(), 5000 * 37);
        let (owner, _) = storage.cell(4999 * 0x100 + 1, 36).unwrap();
        assert_eq!(owner.identity.country, 4999 * 7 + 1);
        assert!(storage.cell(owner.identity.history, 37).is_none());
        assert!(Storage::new(&[], 123, 0).unwrap().cells.is_empty());
    }
}
