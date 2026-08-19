//! arena holds 4.6 MiB and [`super::dedup::OwnershipMap`]'s record table, its
//! [`super::dedup::OwnershipMap`] for the growth points.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::{CustodyRefusal, DEFAULT_CUSTODY_BUDGET_BYTES};

/// The two holders — the byte arena and the ownership map's tables — are
/// separate structures with separate lifetimes and no ownership relation, and
#[derive(Debug)]
pub struct CustodyLedger {
    budget: u64,
    arena: AtomicU64,
    tables: AtomicU64,
}

impl CustodyLedger {
    pub fn new(budget: u64) -> Arc<Self> {
        Arc::new(Self {
            budget,
            arena: AtomicU64::new(0),
            tables: AtomicU64::new(0),
        })
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn held(&self) -> u64 {
        self.arena.load(Ordering::Relaxed) + self.tables.load(Ordering::Relaxed)
    }

    fn arena_room(&self) -> u64 {
        self.budget
            .saturating_sub(self.tables.load(Ordering::Relaxed))
    }

    pub fn charge_arena(&self, delta: u64) -> Result<(), CustodyRefusal> {
        self.admit(delta)?;
        self.arena.fetch_add(delta, Ordering::Relaxed);
        Ok(())
    }

    pub fn release_arena(&self, delta: u64) {
        // Saturating. Every release in this module gives back an amount it
        let _ = self
            .arena
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                Some(held.saturating_sub(delta))
            });
    }

    pub fn charge_arena_overshoot(&self, delta: u64) -> Result<(), CustodyRefusal> {
        if delta == 0 {
            return Ok(());
        }
        let result = self.admit(delta);
        self.arena.fetch_add(delta, Ordering::Relaxed);
        result
    }

    pub fn arena_bytes(&self) -> u64 {
        self.arena.load(Ordering::Relaxed)
    }

    pub fn table_bytes(&self) -> u64 {
        self.tables.load(Ordering::Relaxed)
    }

    pub fn admit(&self, delta: u64) -> Result<(), CustodyRefusal> {
        let held = self.held();
        if u128::from(held) + u128::from(delta) > u128::from(self.budget) {
            return Err(CustodyRefusal::BudgetExhausted {
                budget: self.budget,
                held,
                requested: delta,
            });
        }
        Ok(())
    }

    pub fn charge_tables(&self, delta: u64) -> Result<(), CustodyRefusal> {
        self.admit(delta)?;
        self.tables.fetch_add(delta, Ordering::Relaxed);
        Ok(())
    }

    pub fn release_tables(&self, delta: u64) {
        // Saturating, for the reason given on `release_arena`.
        let _ = self
            .tables
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                Some(held.saturating_sub(delta))
            });
    }

    pub fn charge_tables_overshoot(&self, delta: u64) -> Result<(), CustodyRefusal> {
        if delta == 0 {
            return Ok(());
        }
        let result = self.admit(delta);
        self.tables.fetch_add(delta, Ordering::Relaxed);
        result
    }
}

pub fn vec_bytes<T>(table: &Vec<T>) -> u64 {
    table.capacity() as u64 * std::mem::size_of::<T>() as u64
}

pub fn hash_table_bytes<T>(capacity: usize) -> u64 {
    if capacity == 0 {
        return 0;
    }
    const GROUP_WIDTH: u64 = 16;
    let buckets = (capacity as u64 * 8).div_ceil(7).next_power_of_two();
    let element = std::mem::size_of::<T>() as u64;
    buckets * (element + 1) + GROUP_WIDTH + std::mem::align_of::<T>() as u64
}

fn next_capacity(current: usize) -> usize {
    const FLOOR: usize = 16;
    if current == 0 {
        FLOOR
    } else {
        current.saturating_mul(2).max(FLOOR)
    }
}

pub fn charged_push<T>(
    table: &mut Vec<T>,
    ledger: &CustodyLedger,
    item: T,
) -> Result<(), CustodyRefusal> {
    charged_reserve(table, ledger, 1)?;
    table.push(item);
    Ok(())
}

pub fn charged_reserve<T>(
    table: &mut Vec<T>,
    ledger: &CustodyLedger,
    additional: usize,
) -> Result<(), CustodyRefusal> {
    let wanted = table.len().saturating_add(additional);
    if wanted <= table.capacity() {
        return Ok(());
    }
    let element = std::mem::size_of::<T>() as u64;
    let before = table.capacity();
    let target = next_capacity(before).max(wanted);
    let asked = (target - before) as u64 * element;
    ledger.charge_tables(asked)?;
    table.try_reserve_exact(target - table.len()).map_err(|_| {
        CustodyRefusal::AllocationFailed {
            held: ledger.held(),
            requested: asked,
        }
    })?;
    let overshoot = (table.capacity() - before) as u64 * element - asked;
    ledger.charge_tables_overshoot(overshoot)
}

pub trait ArenaAllocator: core::fmt::Debug {
    fn reserve(&self, buf: &mut Vec<u8>, additional: usize) -> Result<(), ()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemArenaAllocator;

impl ArenaAllocator for SystemArenaAllocator {
    fn reserve(&self, buf: &mut Vec<u8>, additional: usize) -> Result<(), ()> {
        buf.try_reserve_exact(additional).map_err(|_| ())
    }
}

#[derive(Debug)]
pub struct Arena {
    bytes: Vec<u8>,
    ledger: Arc<CustodyLedger>,
    allocator: Box<dyn ArenaAllocator + Send + Sync>,
}

impl Arena {
    pub fn new() -> Self {
        Self::with_budget(DEFAULT_CUSTODY_BUDGET_BYTES)
    }

    pub fn with_budget(budget: u64) -> Self {
        Self::on_ledger(CustodyLedger::new(budget), Box::new(SystemArenaAllocator))
    }

    pub fn with_allocator(budget: u64, allocator: Box<dyn ArenaAllocator + Send + Sync>) -> Self {
        Self::on_ledger(CustodyLedger::new(budget), allocator)
    }

    pub fn on_ledger(
        ledger: Arc<CustodyLedger>,
        allocator: Box<dyn ArenaAllocator + Send + Sync>,
    ) -> Self {
        Self {
            bytes: Vec::new(),
            ledger,
            allocator,
        }
    }

    pub fn ledger(&self) -> Arc<CustodyLedger> {
        Arc::clone(&self.ledger)
    }

    pub fn ledger_ref(&self) -> &CustodyLedger {
        &self.ledger
    }

    pub fn budget(&self) -> u64 {
        self.ledger.budget()
    }

    pub fn held(&self) -> u64 {
        self.ledger.held()
    }

    pub fn copied_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }

    #[inline]
    pub fn alloc_zeroed(&mut self, len: u64) -> Result<usize, CustodyRefusal> {
        let filled = self.bytes.len();
        if let Ok(additional) = usize::try_from(len) {
            if let Some(needed) = filled.checked_add(additional) {
                if needed <= self.bytes.capacity() {
                    self.bytes.resize(needed, 0);
                    return Ok(filled);
                }
            }
        }
        self.alloc_zeroed_growing(len)
    }

    #[cold]
    fn alloc_zeroed_growing(&mut self, len: u64) -> Result<usize, CustodyRefusal> {
        let additional = usize::try_from(len).map_err(|_| CustodyRefusal::AllocationFailed {
            held: self.ledger.held(),
            requested: len,
        })?;
        let filled = self.bytes.len();
        let needed = filled
            .checked_add(additional)
            .ok_or(CustodyRefusal::AllocationFailed {
                held: self.ledger.held(),
                requested: len,
            })?;
        if needed > self.bytes.capacity() {
            let room = usize::try_from(self.ledger.arena_room()).unwrap_or(usize::MAX);
            if needed > room {
                return Err(CustodyRefusal::BudgetExhausted {
                    budget: self.ledger.budget(),
                    held: self.ledger.held(),
                    requested: len,
                });
            }
            let before = self.bytes.capacity();
            let target = before.saturating_mul(2).max(needed).min(room);
            if target < needed || target < before {
                return Err(CustodyRefusal::BudgetExhausted {
                    budget: self.ledger.budget(),
                    held: self.ledger.held(),
                    requested: len,
                });
            }
            let growth = (target - before) as u64;
            self.ledger.charge_arena(growth)?;
            if self
                .allocator
                .reserve(&mut self.bytes, target - filled)
                .is_err()
            {
                self.ledger.release_arena(growth);
                return Err(CustodyRefusal::AllocationFailed {
                    held: self.ledger.held(),
                    requested: len,
                });
            }
            let overshoot = (self.bytes.capacity() - before) as u64 - growth;
            self.ledger.charge_arena_overshoot(overshoot)?;
        }
        self.bytes.resize(needed, 0);
        Ok(filled)
    }

    pub fn slice(&self, offset: usize, len: usize) -> &[u8] {
        &self.bytes[offset..offset + len]
    }

    pub fn slice_mut(&mut self, offset: usize, len: usize) -> &mut [u8] {
        &mut self.bytes[offset..offset + len]
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}
