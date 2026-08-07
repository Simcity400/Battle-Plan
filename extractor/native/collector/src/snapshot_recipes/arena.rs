//! Raw custody: the byte arena an observation copies into, and the ledger the
//! whole observation is charged against.
//!
//! The arena is one growable buffer. Copied intervals are OFFSETS into it, not
//! pointers, so growth never invalidates anything the executor is holding and
//! the frozen snapshot can be moved without fixing anything up.
//!
//! # Custody is a budget, and exhaustion is a refusal
//!
//! The re-pricing raised raw custody from ~256 MiB to 512 MiB and stated the
//! rule that goes with it: "allocation failure or a full second slot must fail
//! closed as backpressure". Both failure modes are represented:
//!
//!  * [`CustodyRefusal::BudgetExhausted`] — the observation asked for more
//!    than its accounted ceiling.
//!  * [`CustodyRefusal::AllocationFailed`] — the allocator itself said no.
//!
//! Neither is recoverable by copying less: the arena does not shrink a request
//! to fit, and there is no partial-allocation return.
//!
//! # The copied bytes are not the only bytes
//!
//! The tables that INDEX the copies are the same order of magnitude as the
//! copies. At the substrate's own bench shape — 100k intervals of 48 B — the
//! arena holds 4.6 MiB and [`super::dedup::OwnershipMap`]'s record table, its
//! three address-sorted index runs, its merge buffer and its two link arenas
//! hold about 8 MiB between them. An accounting that saw only the arena would
//! report a third of what the observation actually took from the machine, and
//! would let a recipe with a pathological fan-out — one interval, a million
//! dependency-chain nodes — sit at 0.0% of budget while it exhausted the box.
//!
//! So there is one [`CustodyLedger`] per observation and both sides charge it.
//! [`Arena::held`] is copied bytes PLUS charged table bytes, and every table
//! that grows with the work charges its capacity before it takes it. See
//! [`super::dedup::OwnershipMap`] for the growth points.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::{CustodyRefusal, DEFAULT_CUSTODY_BUDGET_BYTES};

/// One observation's custody accounting, shared by everything that holds
/// memory on its behalf.
///
/// # Why it is shared rather than owned
///
/// The two holders — the byte arena and the ownership map's tables — are
/// separate structures with separate lifetimes and no ownership relation, and
/// the budget is a property of the OBSERVATION, not of either of them. A
/// ledger each would be two ceilings of 512 MiB, which is a 1 GiB ceiling
/// wearing the wrong label.
///
/// The counters are atomics so the ledger can be shared without making
/// [`Arena`] `!Send`; they are never contended (one observation is one
/// thread's work, and there is no worker-thread scaling in this substrate),
/// so every access is `Relaxed`.
#[derive(Debug)]
pub struct CustodyLedger {
    budget: u64,
    /// Bytes of arena CAPACITY. Capacity rather than the length of the copies
    /// in it, for the same reason `tables` is: a `Vec` that has taken 512 MiB
    /// from the allocator is holding 512 MiB whether or not every byte of it
    /// has been copied into yet, and an arena charged by length alone can be
    /// admitted at the ceiling while holding twice it.
    arena: AtomicU64,
    /// Bytes of index and link table CAPACITY. Capacity, not length: custody
    /// is what was taken from the allocator, not what is currently in use.
    tables: AtomicU64,
}

impl CustodyLedger {
    /// A ledger on `budget`, which the caller has already checked against
    /// [`DEFAULT_CUSTODY_BUDGET_BYTES`].
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

    /// Everything the observation holds: arena capacity plus table capacity.
    pub fn held(&self) -> u64 {
        self.arena.load(Ordering::Relaxed) + self.tables.load(Ordering::Relaxed)
    }

    /// Room left for arena capacity after the tables have taken their share.
    fn arena_room(&self) -> u64 {
        self.budget
            .saturating_sub(self.tables.load(Ordering::Relaxed))
    }

    /// Charge `delta` bytes of ARENA capacity, refusing before the allocator
    /// is touched.
    pub fn charge_arena(&self, delta: u64) -> Result<(), CustodyRefusal> {
        self.admit(delta)?;
        self.arena.fetch_add(delta, Ordering::Relaxed);
        Ok(())
    }

    /// Give back arena capacity that was charged and then not taken.
    ///
    /// The charge has to happen before the allocator is called — that is what
    /// makes the ceiling a ceiling — so an allocator that then refuses leaves
    /// the ledger holding bytes nothing holds. Releasing keeps `held` equal to
    /// the footprint, which is what the refusal it is about to report names.
    pub fn release_arena(&self, delta: u64) {
        // Saturating. Every release in this module gives back an amount it
        // charged, so it cannot go under; the method is public, and a counter
        // that wrapped to `u64::MAX` would turn a bookkeeping slip into an
        // observation that believes it holds sixteen exabytes.
        let _ = self
            .arena
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                Some(held.saturating_sub(delta))
            });
    }

    /// Charge arena capacity the allocator handed over on top of what was
    /// asked for. See [`Self::charge_tables_overshoot`] — same contract.
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

    /// Admit `delta` more bytes against the budget, or refuse.
    ///
    /// The check is in `u128` so a request near `u64::MAX` is refused on the
    /// budget rather than wrapping into a small one.
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

    /// Charge `delta` bytes of TABLE capacity, refusing before the allocator
    /// is touched.
    ///
    /// Callers charge first and allocate second, so a refusal is a refusal to
    /// take the memory rather than a report that it was already taken.
    pub fn charge_tables(&self, delta: u64) -> Result<(), CustodyRefusal> {
        self.admit(delta)?;
        self.tables.fetch_add(delta, Ordering::Relaxed);
        Ok(())
    }

    /// Give back table capacity that has been dropped.
    ///
    /// Custody is what the observation HOLDS, so a table it has let go of must
    /// stop counting against the ceiling — otherwise a structure built after
    /// one is freed is admitted against bytes nothing holds, and an
    /// observation can fail at the ceiling with the room already available.
    pub fn release_tables(&self, delta: u64) {
        // Saturating, for the reason given on `release_arena`.
        let _ = self
            .tables
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                Some(held.saturating_sub(delta))
            });
    }

    /// Charge table capacity the allocator handed over ON TOP of what was
    /// asked for.
    ///
    /// `Vec::try_reserve_exact` is documented as free to round its request up,
    /// so the capacity a table ends up with can exceed the capacity it was
    /// charged for. Reconciling keeps the ledger equal to the real footprint;
    /// the memory is already held when this runs, so a refusal here aborts the
    /// observation rather than preventing the allocation — which is still
    /// fail-closed, just one step later.
    pub fn charge_tables_overshoot(&self, delta: u64) -> Result<(), CustodyRefusal> {
        if delta == 0 {
            return Ok(());
        }
        let result = self.admit(delta);
        self.tables.fetch_add(delta, Ordering::Relaxed);
        result
    }
}

/// Bytes a `Vec` holds: its CAPACITY, which is what it took from the
/// allocator, not its length.
pub fn vec_bytes<T>(table: &Vec<T>) -> u64 {
    table.capacity() as u64 * std::mem::size_of::<T>() as u64
}

/// Bytes a `hashbrown` table of `capacity` usable slots takes.
///
/// It keeps `buckets` slots plus one control byte each and never loads past
/// 7/8, so a table advertising `capacity` has
/// `next_power_of_two(ceil(capacity * 8 / 7))` of them. On top of that its
/// control array carries one trailing SIMD group, so the layout is
/// `buckets * size_of::<T>() + buckets + Group::WIDTH` control bytes, plus
/// whatever the element alignment rounds up.
///
/// This is a MODEL, not a measurement — the allocation is not observable from
/// outside — and it is deliberately the upper end of one, because a custody
/// figure that under-reports is the failure mode that matters. The slack term
/// covers both the trailing group and the alignment padding.
pub fn hash_table_bytes<T>(capacity: usize) -> u64 {
    if capacity == 0 {
        return 0;
    }
    /// One `hashbrown` SIMD control group on every 64-bit target it builds a
    /// vectorised backend for, and its scalar fallback's group is smaller.
    const GROUP_WIDTH: u64 = 16;
    let buckets = (capacity as u64 * 8).div_ceil(7).next_power_of_two();
    let element = std::mem::size_of::<T>() as u64;
    buckets * (element + 1) + GROUP_WIDTH + std::mem::align_of::<T>() as u64
}

/// A table's capacity when it next grows.
///
/// Doubling, with a floor that stops a table from paying a charge, a
/// reconciliation and an allocator round trip for each of its first few
/// elements.
fn next_capacity(current: usize) -> usize {
    const FLOOR: usize = 16;
    if current == 0 {
        FLOOR
    } else {
        current.saturating_mul(2).max(FLOOR)
    }
}

/// Push onto a table whose capacity is charged to `ledger`, refusing BEFORE
/// the allocator is touched.
///
/// # Why this and not `Vec::push`
///
/// `Vec::push` grows on its own amortised schedule and tells nobody, so a
/// ledger built on it would be a ledger of guesses. Here the growth step is
/// this module's decision, the ledger admits it before `try_reserve_exact`
/// runs, and whatever the allocator rounded the request up to is reconciled
/// afterwards — so `CustodyLedger::table_bytes` is the real footprint of every
/// accounted table at every moment, not an estimate of it.
///
/// `try_reserve_exact` rather than `reserve_exact`: an allocation the system
/// refuses is [`CustodyRefusal::AllocationFailed`], which is a typed refusal
/// that gaps the observation, not an abort of the process the game is
/// injected into.
pub fn charged_push<T>(
    table: &mut Vec<T>,
    ledger: &CustodyLedger,
    item: T,
) -> Result<(), CustodyRefusal> {
    charged_reserve(table, ledger, 1)?;
    table.push(item);
    Ok(())
}

/// Make room for `additional` more elements in `table`, charging any capacity
/// growth to `ledger` first. A no-op when the room is already there.
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
    table
        .try_reserve_exact(target - table.len())
        .map_err(|_| CustodyRefusal::AllocationFailed {
            held: ledger.held(),
            requested: asked,
        })?;
    // `try_reserve_exact` is allowed to hand back more than it was asked for.
    // Charge the surplus, so the ledger is the footprint rather than the
    // request.
    let overshoot = (table.capacity() - before) as u64 * element - asked;
    ledger.charge_tables_overshoot(overshoot)
}

/// How the arena obtains memory. Behind a trait so allocation FAILURE is a
/// reachable state in tests: `Vec::try_reserve` only fails at sizes no test
/// should try to allocate.
pub trait ArenaAllocator: core::fmt::Debug {
    /// Ensure `buf` can take `additional` more bytes without reallocating.
    ///
    /// EXACT, not amortised. [`Arena::alloc_zeroed`] chooses the growth step
    /// itself and charges it to custody before calling this; an implementation
    /// that grew by its own schedule on top would take capacity the ledger
    /// never admitted.
    fn reserve(&self, buf: &mut Vec<u8>, additional: usize) -> Result<(), ()>;
}

/// The production allocator: the global one, through the fallible API.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemArenaAllocator;

impl ArenaAllocator for SystemArenaAllocator {
    fn reserve(&self, buf: &mut Vec<u8>, additional: usize) -> Result<(), ()> {
        buf.try_reserve_exact(additional).map_err(|_| ())
    }
}

/// The byte arena. Its accounting lives in the shared [`CustodyLedger`].
#[derive(Debug)]
pub struct Arena {
    bytes: Vec<u8>,
    ledger: Arc<CustodyLedger>,
    allocator: Box<dyn ArenaAllocator + Send + Sync>,
}

impl Arena {
    /// An arena on the default 512 MiB ceiling.
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

    /// The ledger this arena charges. Hand it to every other structure the
    /// same observation allocates through, so they share one ceiling.
    pub fn ledger(&self) -> Arc<CustodyLedger> {
        Arc::clone(&self.ledger)
    }

    pub fn ledger_ref(&self) -> &CustodyLedger {
        &self.ledger
    }

    pub fn budget(&self) -> u64 {
        self.ledger.budget()
    }

    /// Bytes currently in custody: arena capacity plus charged table capacity.
    pub fn held(&self) -> u64 {
        self.ledger.held()
    }

    /// Copied bytes only — the arena's LENGTH, not the capacity custody is
    /// charged for.
    pub fn copied_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Reserve `len` zeroed bytes and return their offset.
    ///
    /// # The arena's growth is this method's decision, not the `Vec`'s
    ///
    /// Charging the arena by its LENGTH under-reports it: `Vec`'s amortised
    /// growth doubles, so a buffer 387 MiB long can have taken 768 MiB, and a
    /// ledger reading 387 would happily admit more. So the growth step is
    /// chosen here, admitted before it is taken, and CAPPED at the room the
    /// tables have left — an arena cannot double past the ceiling because it
    /// is never asked to. `ArenaAllocator::reserve` is an exact reservation
    /// for the same reason: the doubling is above it, not inside it.
    ///
    /// Every check is in `u128` or on `usize`-checked arithmetic, so an
    /// observation that asks for a span near `u64::MAX` is refused on the
    /// budget rather than wrapping into a small request.
    pub fn alloc_zeroed(&mut self, len: u64) -> Result<usize, CustodyRefusal> {
        // NOT admitted here. `held` already contains the whole capacity, so
        // admitting `len` on top of it charges bytes that are already paid
        // for, and a request that fits inside the capacity the arena is
        // holding would be refused for want of room it already has. Admission
        // belongs on the growth below, and only there.
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
            // Doubling, floored at the request and ceilinged at the room the
            // tables left. The `min` is what makes the capacity accounting
            // exact rather than optimistic.
            let target = before.saturating_mul(2).max(needed).min(room);
            // `room >= before` holds whenever `held <= budget` does, because
            // `held` already contains `before` — and every path that can push
            // `held` past the budget aborts the observation before reaching
            // here. Checked rather than assumed: a `target` below the current
            // capacity would underflow the growth, and this is the one place
            // the two counters are compared.
            if target < needed || target < before {
                return Err(CustodyRefusal::BudgetExhausted {
                    budget: self.ledger.budget(),
                    held: self.ledger.held(),
                    requested: len,
                });
            }
            let growth = (target - before) as u64;
            self.ledger.charge_arena(growth)?;
            if self.allocator.reserve(&mut self.bytes, target - filled).is_err() {
                // Charged but never taken. Give it back before reporting, so
                // the refusal's `held` is the footprint and not the attempt.
                self.ledger.release_arena(growth);
                return Err(CustodyRefusal::AllocationFailed {
                    held: self.ledger.held(),
                    requested: len,
                });
            }
            // An exact reservation may still round up. Charge whatever it
            // really took, so the ledger is the footprint.
            //
            // This is the one place the observation can hold more than the
            // ceiling, and the amount is exactly the allocator's rounding of a
            // single request — `try_reserve_exact` asks for a precise byte
            // count, so for the system allocator on a `Vec<u8>` it is zero.
            // Whatever it is, it is discovered on the same call that caused
            // it, the observation aborts on the refusal, and the arena is
            // dropped. There is no path that keeps running above the ceiling.
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

    /// Hand the bytes to the frozen snapshot. Consumes the arena: nothing can
    /// append to an image a reader is already holding.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}
