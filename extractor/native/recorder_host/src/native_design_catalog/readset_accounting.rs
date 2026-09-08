//! Shared A27 native read/retention accounting; no worker and no wire.
//! Reservations are monotone and fund closing calls before any opening.
use super::{failure, CatalogFailure, FailureKind, Memory, Result};
use super::slot_identity::Budget;
use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;

pub(super) const VERIFICATION_LIMIT: usize = 128 * 1024 * 1024;
pub(super) type SharedBudget = Rc<RefCell<Budget>>;
pub(super) fn budget() -> SharedBudget {
    // Operational work limits, not 16ms qualification. Includes repeated and
    // unsuccessful reads and both phases. JSON/fold storage has its own owner.
    Rc::new(RefCell::new(Budget::new(VERIFICATION_LIMIT, 2_000_000, 256 * 1024 * 1024)))
}
fn budget_error() -> CatalogFailure { failure(FailureKind::BoundExceeded, "A27 shared work/storage budget") }
struct Record { address: u64, width: usize, expected: Option<Vec<u8>> }
#[derive(Clone, Copy)]
struct Index { address: u64, width: usize, record: usize }
const EMPTY: Index = Index { address: 0, width: 0, record: usize::MAX };
const MAX_RECORDS: usize = 1_000_000;

pub(super) struct Accounting {
    pub budget: SharedBudget,
    records: Vec<Record>,
    index: Vec<Index>,
    reserved_records: usize,
    fatal: Option<CatalogFailure>,
    closed: bool,
}
pub(super) struct Recipe {
    bytes: usize,
    calls: usize,
}
impl Accounting {
    pub fn new(budget: SharedBudget) -> Self {
        Self { budget, records: Vec::new(), index: Vec::new(),
            reserved_records: 0, fatal: None, closed: false }
    }
    fn retain(&mut self, bytes: usize) -> Result<()> {
        self.budget.borrow_mut().allocation(bytes).map_err(|_| budget_error())
    }
    fn hash(address: u64, width: usize) -> usize {
        let mut h = address ^ (width as u64).wrapping_mul(0x9e3779b97f4a7c15);
        h ^= h >> 30; h = h.wrapping_mul(0xbf58476d1ce4e5b9);
        h ^= h >> 27; h = h.wrapping_mul(0x94d049bb133111eb);
        (h ^ (h >> 31)) as usize
    }
    fn bucket(index: &[Index], address: u64, width: usize) -> usize {
        let mut slot = Self::hash(address, width) & (index.len() - 1);
        while index[slot].record != usize::MAX &&
            (index[slot].address != address || index[slot].width != width) {
            slot = (slot + 1) & (index.len() - 1);
        }
        slot
    }
    fn room(&mut self, extra: usize) -> Result<()> {
        let needed = self.records.len().checked_add(self.reserved_records)
            .and_then(|n| n.checked_add(extra)).filter(|n| *n <= MAX_RECORDS)
            .ok_or_else(budget_error)?;
        if needed > self.records.capacity() {
            let capacity = needed.max(16).checked_next_power_of_two().ok_or_else(budget_error)?;
            // Charge each allocation request's entire payload, not merely the
            // net growth. Old buffers never refund the cumulative work meter.
            self.retain(capacity.checked_mul(size_of::<Record>()).ok_or_else(budget_error)?)?;
            self.records.try_reserve_exact(capacity - self.records.len())
                .map_err(|_| failure(FailureKind::BoundExceeded, "A27 trace allocation"))?;
        }
        let capacity = needed.max(8).checked_mul(2)
            .and_then(usize::checked_next_power_of_two).ok_or_else(budget_error)?;
        if capacity > self.index.len() {
            self.retain(capacity.checked_mul(size_of::<Index>()).ok_or_else(budget_error)?)?;
            let mut replacement = Vec::new();
            replacement.try_reserve_exact(capacity)
                .map_err(|_| failure(FailureKind::BoundExceeded, "A27 address index allocation"))?;
            replacement.resize(capacity, EMPTY);
            for entry in &self.index {
                if entry.record != usize::MAX {
                    let at = Self::bucket(&replacement, entry.address, entry.width);
                    replacement[at] = *entry;
                }
            }
            self.index = replacement;
        }
        Ok(())
    }
    fn charge(&mut self, bytes: usize, calls: usize, output: usize) -> Result<()> {
        let allocation = bytes.checked_mul(3).and_then(|n| n.checked_add(output))
            .ok_or_else(budget_error)?;
        // memory-return buffer, retained/returned copy, closing memory buffer.
        self.budget.borrow_mut().reserve_work(bytes.checked_mul(2).ok_or_else(budget_error)?,
            calls.checked_mul(2).ok_or_else(budget_error)?).map_err(|_| budget_error())?;
        self.retain(allocation)
    }
    fn latch<T>(&mut self, result: Result<T>) -> Result<T> {
        if let Err(error) = &result {
            if error.kind != FailureKind::Unreadable && self.fatal.is_none() {
                self.fatal = Some(error.clone());
            }
        }
        result
    }
    pub fn reserve_recipe(&mut self, bytes: usize, calls: usize, output: usize) -> Result<Recipe> {
        let result = (|| {
            if self.closed { return Err(budget_error()); }
            if let Some(error) = &self.fatal { return Err(error.clone()); }
            self.room(calls)?;
            self.charge(bytes, calls, output)?;
            self.reserved_records += calls;
            Ok(Recipe { bytes, calls })
        })();
        self.latch(result)
    }
    pub fn release_recipe(&mut self, recipe: Recipe) {
        // Unused capacity may serve another owner, but no counter is refunded.
        self.reserved_records -= recipe.calls;
    }
    pub fn qualify(&mut self, bytes: usize, calls: usize, allocation: usize, spans: usize)
        -> std::result::Result<(), super::slot_identity::Refusal> {
        use super::slot_identity::RefusalKind as K;
        let result = self.qualify_inner(bytes, calls, allocation, spans);
        if let Err(error) = &result {
            if matches!(error.kind, K::ReadBudget | K::AllocationBudget | K::TraceBound | K::AllocationFailed)
                && self.fatal.is_none() {
                // A local slot refusal cannot authorize publishing an acquisition
                // whose complete work/storage reservation failed. Keep its first
                // fatal cause; seal still visits every funded attempted span.
                self.fatal = Some(failure(FailureKind::BoundExceeded, error.field));
            }
        }
        result
    }
    fn qualify_inner(&mut self, bytes: usize, calls: usize, allocation: usize, spans: usize)
        -> std::result::Result<(), super::slot_identity::Refusal> {
        use super::slot_identity::{Refusal, RefusalKind as K};
        let refusal = |kind, field| Refusal { kind, field };
        if self.closed { return Err(refusal(K::Closed, "A27 readset")); }
        if self.fatal.is_some() { return Err(refusal(K::ReadBudget, "failed A27 acquisition")); }
        if self.records.len().checked_add(self.reserved_records)
            .and_then(|n| n.checked_add(spans)).is_none_or(|n| n > MAX_RECORDS) {
            return Err(refusal(K::TraceBound, "complete census spans"));
        }
        self.room(spans).map_err(|error| {
            let kind = if matches!(error.field.as_str(),
                "A27 trace allocation" | "A27 address index allocation") {
                K::AllocationFailed
            } else { K::AllocationBudget };
            refusal(kind, "A27 census trace capacity")
        })?;
        // Slot observer specifies opening+closing work; its Vec-returning host
        // adapter additionally needs 3 bytes of read payload per opening byte.
        let payload = (bytes / 2).checked_mul(3).and_then(|n| n.checked_add(allocation))
            .ok_or_else(|| refusal(K::AllocationBudget, "complete census capacity"))?;
        self.budget.borrow().qualify(bytes, calls, payload)
    }
    pub fn output(&mut self, bytes: usize) -> Result<()> {
        if let Some(error) = &self.fatal { return Err(error.clone()); }
        let result = if self.closed { Err(budget_error()) } else { self.retain(bytes) };
        self.latch(result)
    }
    fn open_inner<M: Memory>(&mut self, memory: &M, address: u64, width: usize,
        recipe: Option<&mut Recipe>) -> Result<Vec<u8>> {
        if self.closed { return Err(failure(FailureKind::InvalidRoot, "closed A27 readset")); }
        if width == 0 { return Ok(Vec::new()); }
        if address == 0 || address.checked_add(width as u64).is_none() {
            return Err(failure(FailureKind::InvalidRoot, "read address"));
        }
        if let Some(error) = &self.fatal { return Err(error.clone()); }
        if let Some(recipe) = recipe {
            // This fixed opening recipe was funded before acquisition; all of
            // its attempted reads also retain their prepaid outer closing call.
            if recipe.calls == 0 || width > recipe.bytes { return Err(budget_error()); }
            recipe.calls -= 1; recipe.bytes -= width; self.reserved_records -= 1;
        } else {
            self.room(1)?;
            self.charge(width, 1, 0)?;
        }
        let prior_slot = Self::bucket(&self.index, address, width);
        let previous = self.index[prior_slot].record;
        let record = self.records.len();
        // Address and storage are retained BEFORE opening; failure has no bytes
        // to compare and cannot later become measured through recovery.
        self.records.push(Record { address, width, expected: None });
        let bytes = memory.read(address, width)
            .map_err(|_| failure(FailureKind::Unreadable, "opening read"))?;
        if bytes.len() != width {
            return Err(failure(FailureKind::Unreadable, "short read"));
        }
        let changed = previous != usize::MAX &&
            self.records[previous].expected.as_ref().is_some_and(|old| old != &bytes);
        self.records[record].expected = Some(bytes);
        self.index[prior_slot] = Index { address, width, record };
        if changed { return Err(failure(FailureKind::Torn, "repeated field")); }
        let mut result = Vec::new();
        result.try_reserve_exact(width)
            .map_err(|_| failure(FailureKind::BoundExceeded, "A27 returned read allocation"))?;
        result.extend_from_slice(self.records[record].expected.as_ref().unwrap());
        Ok(result)
    }
    pub fn read<M: Memory>(&mut self, memory: &M, address: u64, width: usize,
        recipe: Option<&mut Recipe>) -> Result<Vec<u8>> {
        let result = self.open_inner(memory, address, width, recipe);
        self.latch(result)
    }
    pub fn seal<M: Memory>(&mut self, memory: &M) -> Result<()> {
        if self.closed { return Err(failure(FailureKind::InvalidRoot, "second A27 seal")); }
        self.closed = true;
        let mut failure_seen = self.fatal.clone();
        for record in &self.records {
            let closing = memory.read(record.address, record.width);
            // A failed opening has already returned Unreadable to its caller;
            // an optional field retains that refusal even when it recovers.
            if let Some(expected) = &record.expected {
                let error = match closing {
                    Ok(actual) if actual == *expected => None,
                    Ok(actual) if actual.len() == record.width =>
                        Some(failure(FailureKind::Torn, "roots/fields/membership seal")),
                    _ => Some(failure(FailureKind::Unreadable, "seal read")),
                };
                if failure_seen.is_none() { failure_seen = error; }
            }
        }
        failure_seen.map_or(Ok(()), Err)
    }
}
