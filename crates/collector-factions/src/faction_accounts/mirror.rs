use super::Account;
use engine_allocation as allocation;
use std::{
    ptr,
    sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, Ordering},
};
#[repr(align(64))]
struct Hot(AtomicU64);
const ACTIVE_BITS: u32 = 20;
const ACTIVE_MASK: u64 = (1 << ACTIVE_BITS) - 1;
const VERSION_STEP: u64 = 1 << ACTIVE_BITS;
static POINTER: AtomicPtr<Storage> = AtomicPtr::new(ptr::null_mut());
static CLOSED: AtomicBool = AtomicBool::new(false);
static STATE: Hot = Hot(AtomicU64::new(0));
static MISSED: AtomicBool = AtomicBool::new(false);
pub const INITIATIVE: usize = 0;
pub const CONTRIBUTION: usize = 1;
pub const GAIN_A: usize = 2;
pub const GAIN_B: usize = 3;
pub const PREVIOUS_SCORE: usize = 4;
pub const INFLUENCE_SHARE: usize = 5;
pub const PROJECTION_SHARE: usize = 6;
pub const PROJECTION_RANK: usize = 7;
pub const PROJECTION_START: usize = 8;
#[derive(Clone, Copy)]
pub enum Operation {
    Set,
    Add,
    Subtract,
}
pub struct Flight;
impl Flight {
    #[inline]
    pub fn new() -> Self {
        STATE.0.fetch_add(VERSION_STEP + 1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Flight {
    #[inline]
    fn drop(&mut self) {
        STATE
            .0
            .fetch_add(VERSION_STEP.wrapping_sub(1), Ordering::SeqCst);
    }
}
pub fn active() -> usize {
    (STATE.0.load(Ordering::SeqCst) & ACTIVE_MASK) as usize
}
pub fn baseline_stamp() -> Option<u64> {
    let state = STATE.0.load(Ordering::SeqCst);
    (state & ACTIVE_MASK == 0).then_some(state >> ACTIVE_BITS)
}
pub fn quiet() -> Option<u64> {
    let state = STATE.0.load(Ordering::SeqCst);
    (state & ACTIVE_MASK == 0 && !MISSED.load(Ordering::SeqCst)).then_some(state >> ACTIVE_BITS)
}
struct Slot {
    address: usize,
    influence: Vec<AtomicI64>,
    projection: Vec<AtomicI64>,
    values: [AtomicI64; 8],
    initiative_anchor: AtomicI64,
    initiative_product: AtomicI64,
    initiative_steps: AtomicU64,
    shares: [AtomicI64; 6],
    projection_sources: [AtomicI64; 7],
    projection_derived: AtomicU64,
    influence_faction: AtomicU64,
    total_inputs: [AtomicI64; 2],
    rank_pending: [AtomicI64; 5],
    rank: [AtomicI64; 5],
}
pub struct Storage {
    data: usize,
    stride: usize,
    slots: Vec<Slot>,
    dirty: Vec<AtomicU64>,
    invalid: AtomicBool,
    missing_projection_sources: AtomicBool,
    missing_share_inputs: AtomicBool,
}
impl Storage {
    pub fn new(data: usize, stride: usize, rows: &[Account]) -> Result<Box<Self>, &'static str> {
        if stride == 0 {
            return Err("faction operand slot stride is zero");
        }
        let mut slots = Vec::new();
        let mut dirty = Vec::new();
        allocation::reserve(&mut slots, rows.len())
            .map_err(|_| "faction operand slot allocation failed")?;
        allocation::reserve(&mut dirty, rows.len().div_ceil(64))
            .map_err(|_| "faction operand marks allocation failed")?;
        for (index, row) in rows.iter().enumerate() {
            let mut influence = Vec::new();
            allocation::reserve(&mut influence, row.influence.len())
                .map_err(|_| "faction influence operand allocation failed")?;
            influence.extend(row.influence.iter().map(|value| AtomicI64::new(*value)));
            let mut projection = Vec::new();
            allocation::reserve(&mut projection, row.projection.len())
                .map_err(|_| "faction projection operand allocation failed")?;
            projection.extend(row.projection.iter().map(|value| AtomicI64::new(*value)));
            let values = [
                row.initiative_raw,
                row.contribution_raw,
                row.contribution_gain_a_raw,
                row.contribution_gain_b_raw,
                row.previous_war_score_raw,
                row.influence_share_raw,
                row.projection_share_raw,
                i64::from(row.projection_rank),
            ]
            .map(AtomicI64::new);
            slots.push(Slot {
                address: data
                    .checked_add(
                        index
                            .checked_mul(stride)
                            .ok_or("faction operand slot address overflow")?,
                    )
                    .ok_or("faction operand slot address overflow")?,
                influence,
                projection,
                values,
                initiative_anchor: AtomicI64::new(row.initiative_raw),
                initiative_product: AtomicI64::new(0),
                initiative_steps: AtomicU64::new(0),
                shares: std::array::from_fn(|_| AtomicI64::new(0)),
                projection_sources: std::array::from_fn(|_| AtomicI64::new(0)),
                projection_derived: AtomicU64::new(0),
                influence_faction: AtomicU64::new(0),
                total_inputs: std::array::from_fn(|_| AtomicI64::new(0)),
                rank_pending: std::array::from_fn(|_| AtomicI64::new(0)),
                rank: std::array::from_fn(|_| AtomicI64::new(0)),
            });
        }
        dirty.extend((0..rows.len().div_ceil(64)).map(|_| AtomicU64::new(0)));
        allocation::boxed(Self {
            data,
            stride,
            slots,
            dirty,
            invalid: AtomicBool::new(false),
            missing_projection_sources: AtomicBool::new(false),
            missing_share_inputs: AtomicBool::new(false),
        })
        .map_err(|_| "faction operand storage allocation failed")
    }
    #[inline]
    fn mark_dirty(&self, index: usize) {
        let word = &self.dirty[index / 64];
        let bit = 1u64 << (index % 64);
        if word.load(Ordering::SeqCst) & bit == 0 {
            word.fetch_or(bit, Ordering::SeqCst);
        }
    }
    fn index(&self, address: usize) -> Option<usize> {
        let offset = address.checked_sub(self.data)?;
        if offset % self.stride != 0 {
            return None;
        }
        let index = offset / self.stride;
        (self.slots.get(index)?.address == address).then_some(index)
    }
    fn write(
        &self,
        address: usize,
        influence: bool,
        field: usize,
        value: i64,
        operation: Operation,
    ) {
        let Some(index) = self.index(address) else {
            return;
        };
        self.write_at(index, influence, field, value, operation);
    }
    fn write_at(
        &self,
        index: usize,
        influence: bool,
        field: usize,
        value: i64,
        operation: Operation,
    ) {
        let slot = &self.slots[index];
        let target = if influence {
            slot.influence.get(field)
        } else if field >= PROJECTION_START {
            slot.projection.get(field - PROJECTION_START)
        } else {
            slot.values.get(field)
        };
        let Some(target) = target else {
            self.invalid.store(true, Ordering::SeqCst);
            return;
        };
        if !influence && field == INITIATIVE {
            slot.initiative_anchor.store(value, Ordering::Relaxed);
            slot.initiative_product.store(0, Ordering::Relaxed);
            slot.initiative_steps.store(0, Ordering::Relaxed);
        }
        match operation {
            Operation::Set => target.store(value, Ordering::Relaxed),
            Operation::Add => {
                target.fetch_add(value, Ordering::Relaxed);
            }
            Operation::Subtract => {
                target.fetch_sub(value, Ordering::Relaxed);
            }
        }
        self.mark_dirty(index);
    }
    fn projection_sources(&self, index: usize, term: usize, source: &[i64; 7]) {
        let indices: &[usize] = match term {
            0 => &[0, 1],
            1 => &[2],
            2 => &[3, 4],
            3 => &[5, 6],
            _ => {
                self.invalid.store(true, Ordering::SeqCst);
                return;
            }
        };
        for &at in indices {
            self.slots[index].projection_sources[at].store(source[at], Ordering::Relaxed);
        }
        self.slots[index]
            .projection_derived
            .fetch_or(1 << term, Ordering::Relaxed);
        self.mark_dirty(index);
    }
    fn refresh(&self, address: usize, projection: bool, numerator: i64, denominator: i64) {
        let Some(index) = self.index(address) else {
            return;
        };
        let slot = &self.slots[index];
        let offset = if projection { 3 } else { 0 };
        slot.shares[offset].store(numerator, Ordering::Relaxed);
        slot.shares[offset + 1].store(denominator, Ordering::Relaxed);
        let boundary = slot.shares[offset + 2].load(Ordering::Relaxed) as u32;
        let boundary = boundary.wrapping_add(1).max(1);
        if !projection {
            let (faction, phase, _) = crate::faction_pools::phase(0).unwrap_or_default();
            slot.influence_faction.store(faction, Ordering::Relaxed);
            slot.total_inputs[0].store(phase as i64, Ordering::Relaxed);
            slot.total_inputs[1].store(numerator, Ordering::Relaxed);
        }
        slot.shares[offset + 2].store(i64::from(boundary), Ordering::Relaxed);
        slot.values[if projection {
            PROJECTION_SHARE
        } else {
            INFLUENCE_SHARE
        }]
        .store(
            replay_data::rows::FactionAccountRow::normalized(numerator, denominator),
            Ordering::Relaxed,
        );
        self.mark_dirty(index);
    }
    fn rank_input(&self, address: usize, numerator: i64, order: u32) {
        let Some(index) = self.index(address) else {
            return;
        };
        let (faction, phase, count) = crate::faction_pools::phase(1).unwrap_or_default();
        for (target, value) in self.slots[index].rank_pending.iter().zip([
            numerator.max(0),
            order as i64,
            faction as i64,
            phase as i64,
            count as i64,
        ]) {
            target.store(value, Ordering::Relaxed);
        }
    }
    fn rank_commit(&self, address: usize, native: i32) {
        let Some(index) = self.index(address) else {
            return;
        };
        let slot = &self.slots[index];
        for (target, source) in slot.rank.iter().zip(&slot.rank_pending) {
            target.store(source.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        slot.values[PROJECTION_RANK].store(native as i64, Ordering::Relaxed);
        self.mark_dirty(index);
    }
    fn passive(&self, address: usize, product: i64) {
        let Some(index) = self.index(address) else {
            return;
        };
        let slot = &self.slots[index];
        let current = slot.values[INITIATIVE].load(Ordering::Relaxed);
        let steps = slot.initiative_steps.load(Ordering::Relaxed);
        if slot.initiative_product.load(Ordering::Relaxed) != product || steps == u32::MAX as u64 {
            slot.initiative_anchor.store(current, Ordering::Relaxed);
            slot.initiative_product.store(product, Ordering::Relaxed);
            slot.initiative_steps.store(0, Ordering::Relaxed);
        }
        slot.initiative_steps.fetch_add(1, Ordering::Relaxed);
        slot.values[INITIATIVE].store(
            current.wrapping_add(product / 100_000).max(0),
            Ordering::Relaxed,
        );
        self.mark_dirty(index);
    }
    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.slots.capacity() * size_of::<Slot>()
            + self.dirty.capacity() * size_of::<AtomicU64>()
            + self
                .slots
                .iter()
                .map(|slot| {
                    (slot.influence.capacity() + slot.projection.capacity())
                        * size_of::<AtomicI64>()
                })
                .sum::<usize>()
    }
    pub fn reset(&self, index: usize, row: &Account) -> Result<(), &'static str> {
        let slot = self
            .slots
            .get(index)
            .ok_or("faction operand reset index invalid")?;
        if slot.influence.len() != row.influence.len() {
            return Err(
                "faction influence operand geometry changed without array lifetime notification",
            );
        }
        for (target, value) in slot.influence.iter().zip(&row.influence) {
            target.store(*value, Ordering::Relaxed);
        }
        if slot.projection.len() != row.projection.len() {
            return Err(
                "faction projection operand geometry changed without lifetime notification",
            );
        }
        for (target, value) in slot.projection.iter().zip(&row.projection) {
            target.store(*value, Ordering::Relaxed);
        }
        for (target, value) in slot.values.iter().zip([
            row.initiative_raw,
            row.contribution_raw,
            row.contribution_gain_a_raw,
            row.contribution_gain_b_raw,
            row.previous_war_score_raw,
            row.influence_share_raw,
            row.projection_share_raw,
            i64::from(row.projection_rank),
        ]) {
            target.store(value, Ordering::Relaxed);
        }
        slot.influence_faction.store(0, Ordering::Relaxed);
        for value in &slot.total_inputs {
            value.store(0, Ordering::Relaxed);
        }
        for value in slot.rank.iter().chain(&slot.rank_pending) {
            value.store(0, Ordering::Relaxed);
        }
        slot.projection_derived.store(0, Ordering::Relaxed);
        for value in &slot.projection_sources {
            value.store(0, Ordering::Relaxed);
        }
        for value in &slot.shares {
            value.store(0, Ordering::Relaxed);
        }
        slot.initiative_anchor
            .store(row.initiative_raw, Ordering::Relaxed);
        slot.initiative_product.store(0, Ordering::Relaxed);
        slot.initiative_steps.store(0, Ordering::Relaxed);
        Ok(())
    }
    pub fn drain(&self, rows: &mut [Account]) -> Result<bool, &'static str> {
        if self.missing_projection_sources.load(Ordering::SeqCst) {
            return Err("faction projection setter lacks its supplying getter operands");
        }
        if self.missing_share_inputs.load(Ordering::SeqCst) {
            return Err("faction share refresh supplying denominator unreadable");
        }
        if self.invalid.load(Ordering::SeqCst) {
            return Err("faction operand writer selected an invalid statistic");
        }
        let mut changed = false;
        for (word, dirty) in self.dirty.iter().enumerate() {
            let mut bits = dirty.swap(0, Ordering::SeqCst);
            while bits != 0 {
                let index = word * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let slot = &self.slots[index];
                let row = &mut rows[index];
                for (value, source) in row.influence.iter_mut().zip(&slot.influence) {
                    *value = source.load(Ordering::Relaxed);
                }
                for (value, source) in row.projection.iter_mut().zip(&slot.projection) {
                    *value = source.load(Ordering::Relaxed);
                }
                let values = slot
                    .values
                    .each_ref()
                    .map(|value| value.load(Ordering::Relaxed));
                row.projection_derived = slot.projection_derived.load(Ordering::Relaxed) as u8;
                row.projection_sources = slot
                    .projection_sources
                    .each_ref()
                    .map(|value| value.load(Ordering::Relaxed));
                row.initiative_raw = values[0];
                row.initiative_anchor_raw = slot.initiative_anchor.load(Ordering::Relaxed);
                row.initiative_product_raw = slot.initiative_product.load(Ordering::Relaxed);
                row.initiative_steps = slot.initiative_steps.load(Ordering::Relaxed) as u32;
                row.contribution_raw = values[1];
                row.contribution_gain_a_raw = values[2];
                row.contribution_gain_b_raw = values[3];
                row.previous_war_score_raw = values[4];
                let shares = slot
                    .shares
                    .each_ref()
                    .map(|value| value.load(Ordering::Relaxed));
                row.influence_numerator_raw = shares[0];
                row.influence_denominator_raw = shares[1];
                row.influence_refresh = shares[2] as u32;
                row.projection_numerator_raw = shares[3];
                row.projection_denominator_raw = shares[4];
                row.projection_refresh = shares[5] as u32;
                row.influence_faction = slot.influence_faction.load(Ordering::Relaxed);
                row.influence_total_phase = slot.total_inputs[0].load(Ordering::Relaxed) as u64;
                row.influence_total_raw = slot.total_inputs[1].load(Ordering::Relaxed);
                let rank = slot
                    .rank
                    .each_ref()
                    .map(|value| value.load(Ordering::Relaxed));
                row.rank_input_raw = rank[0];
                row.rank_input_order = rank[1] as u32;
                row.rank_faction = rank[2] as u64;
                row.rank_refresh = rank[3] as u64;
                row.rank_members = rank[4] as u32;
                row.influence_share_raw = values[5];
                row.projection_share_raw = values[6];
                row.projection_rank = values[7] as i32;
                changed = true;
            }
        }
        Ok(changed)
    }
}
#[derive(Default)]
pub struct Holder {
    storage: Option<Box<Storage>>,
}
impl Holder {
    pub fn install(&mut self, mut storage: Box<Storage>) -> Result<(), &'static str> {
        CLOSED.store(true, Ordering::SeqCst);
        if active() != 0 {
            CLOSED.store(false, Ordering::SeqCst);
            return Err("faction operand publication overlaps a writer");
        }
        MISSED.store(false, Ordering::SeqCst);
        POINTER.store(&raw mut *storage, Ordering::SeqCst);
        let old = self.storage.replace(storage);
        CLOSED.store(false, Ordering::SeqCst);
        drop(old);
        Ok(())
    }
    pub fn needs_baseline(&self) -> bool {
        MISSED.load(Ordering::SeqCst)
            || self.storage.as_ref().is_none_or(|storage| {
                storage.invalid.load(Ordering::SeqCst)
                    || storage.missing_projection_sources.load(Ordering::SeqCst)
                    || storage.missing_share_inputs.load(Ordering::SeqCst)
            })
    }
    pub fn allocated_bytes(&self) -> usize {
        self.storage
            .as_ref()
            .map_or(0, |storage| storage.allocated_bytes())
    }
    pub fn storage(&self) -> Result<&Storage, &'static str> {
        self.storage
            .as_deref()
            .ok_or("faction operand baseline unavailable")
    }
}
impl Drop for Holder {
    fn drop(&mut self) {
        let Some(storage) = self.storage.take() else {
            return;
        };
        if ptr::eq(POINTER.load(Ordering::SeqCst), &raw const *storage) {
            CLOSED.store(true, Ordering::SeqCst);
            POINTER.store(ptr::null_mut(), Ordering::SeqCst);
            if active() != 0 {
                std::mem::forget(storage);
            }
        }
    }
}
pub fn observe(address: usize, influence: bool, field: usize, value: i64, operation: Operation) {
    if CLOSED.load(Ordering::SeqCst) {
        MISSED.store(true, Ordering::SeqCst);
        return;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if pointer.is_null() {
        return;
    }
    // SAFETY: Flight admission precedes the pointer load; publication and Drop
    // close admission and retain old storage when a flight is active.
    unsafe { &*pointer }.write(address, influence, field, value, operation);
}
thread_local! {static RANK_RECEIVER:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
pub fn rank_receiver(receiver: usize) {
    RANK_RECEIVER.with(|cell| cell.set(receiver));
}
pub fn rank_input(numerator: i64, order: u32) {
    if CLOSED.load(Ordering::SeqCst) {
        MISSED.store(true, Ordering::SeqCst);
        return;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if !pointer.is_null() {
        unsafe { &*pointer }.rank_input(RANK_RECEIVER.with(|cell| cell.get()), numerator, order);
    }
}
pub fn rank_commit(receiver: usize, native: i32) {
    if CLOSED.load(Ordering::SeqCst) {
        MISSED.store(true, Ordering::SeqCst);
        return;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if !pointer.is_null() {
        unsafe { &*pointer }.rank_commit(receiver, native);
    }
}
thread_local! { static PROJECTION_SOURCE: std::cell::Cell<([i64;7],u8)> = const {std::cell::Cell::new(([0;7],0))}; }
pub fn projection_operand(index: usize, value: i64) {
    PROJECTION_SOURCE.with(|cell| {
        let (mut values, mut mask) = cell.get();
        values[index] = value;
        if matches!(index, 0 | 2 | 3 | 5) {
            mask &= !(match index {
                0 => 3,
                2 => 4,
                3 => 24,
                _ => 96,
            });
        }
        mask |= 1 << index;
        cell.set((values, mask));
    });
}
pub fn projection_sources(address: usize, term: usize, value: i64) {
    let required = match term {
        0 => 3,
        1 => 4,
        2 => 24,
        3 => 96,
        _ => 0,
    };
    let source = PROJECTION_SOURCE.with(|cell| {
        let (values, mask) = cell.get();
        cell.set((values, mask & !required));
        (values, mask)
    });
    if CLOSED.load(Ordering::SeqCst) {
        MISSED.store(true, Ordering::SeqCst);
        return;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if !pointer.is_null() {
        let storage = unsafe { &*pointer };
        let Some(index) = storage.index(address) else {
            return;
        };
        if required == 0 || source.1 & required != required {
            storage
                .missing_projection_sources
                .store(true, Ordering::SeqCst);
        } else {
            storage.projection_sources(index, term, &source.0);
        }
        storage.write_at(index, false, PROJECTION_START + term, value, Operation::Set);
    }
}
pub fn refresh(address: usize, projection: bool, numerator: i64, denominator: i64) {
    if CLOSED.load(Ordering::SeqCst) {
        MISSED.store(true, Ordering::SeqCst);
        return;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if !pointer.is_null() {
        unsafe { &*pointer }.refresh(address, projection, numerator, denominator);
    }
}
pub fn passive(address: usize, product: i64) {
    if CLOSED.load(Ordering::SeqCst) {
        MISSED.store(true, Ordering::SeqCst);
        return;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if !pointer.is_null() {
        unsafe { &*pointer }.passive(address, product);
    }
}

fn rank_address(storage: &Storage, canonical_slot: Option<u32>) -> Option<usize> {
    canonical_slot
        .and_then(|slot| storage.slots.get(slot as usize))
        .map(|slot| slot.address)
}
pub fn rank_country(canonical_slot: Option<u32>) {
    let pointer = POINTER.load(Ordering::SeqCst);
    let receiver = if CLOSED.load(Ordering::SeqCst) || pointer.is_null() {
        None
    } else {
        rank_address(unsafe { &*pointer }, canonical_slot)
    };
    rank_receiver(receiver.unwrap_or(0));
}
pub fn refuse(receiver: usize) {
    let pointer = POINTER.load(Ordering::SeqCst);
    if !pointer.is_null() && !CLOSED.load(Ordering::SeqCst) {
        let storage = unsafe { &*pointer };
        if storage.index(receiver).is_some() {
            storage.missing_share_inputs.store(true, Ordering::SeqCst);
        }
    }
}
pub fn projection_factory_total(total: Option<i64>) {
    PROJECTION_SOURCE.with(|cell| {
        let (mut values, mut mask) = cell.get();
        if let Some(total) = total.filter(|_| mask & 1 != 0) {
            values[1] = total.wrapping_sub((values[0] / 100_000).max(0));
            mask |= 2;
        } else {
            mask &= !2;
        }
        cell.set((values, mask));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_account_slots_reject_invalid_addresses_and_replaced_geometry() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let rows = vec![Account::default(); 3];
        let mut storage = Storage::new(0x1000, 0xd0, &rows).unwrap();
        for address in [0, 0xfff, 0x1000, 0x1001, 0x10d0, 0x11a0, 0x1270, usize::MAX] {
            assert_eq!(
                storage.index(address),
                storage
                    .slots
                    .binary_search_by_key(&address, |slot| slot.address)
                    .ok()
            );
        }
        storage.slots[1].address += 8;
        assert_eq!(storage.index(0x10d0), None);
        assert_eq!(Storage::new(0x2000, 0xd0, &[]).unwrap().index(0x2000), None);
        assert!(Storage::new(0x1000, 0, &rows).is_err());
        assert!(Storage::new(usize::MAX - 1, 0xd0, &rows).is_err());

        let mut holder = Holder::default();
        holder.install(storage).unwrap();
        let mut fresh = vec![Account::default(); 3];
        holder
            .install(Storage::new(0x2000, 0xd0, &fresh).unwrap())
            .unwrap();
        collector_test_support::with_allocations_denied(|| {
            let _flight = Flight::new();
            observe(0x1000, false, CONTRIBUTION, 99, Operation::Set);
        });
        assert!(!holder.storage().unwrap().drain(&mut fresh).unwrap());
    }

    #[test]
    fn projection_reuses_one_slot_and_keeps_source_consumption_and_literal_writes() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut rows = vec![Account {
            projection: vec![0; 4],
            ..Default::default()
        }];
        let mut holder = Holder::default();
        holder
            .install(Storage::new(0x1000, 0xd0, &rows).unwrap())
            .unwrap();
        let sources = [101, 202, 303, 404, 505, 606, 707];
        collector_test_support::with_allocations_denied(|| {
            let _flight = Flight::new();
            for (index, value) in sources.into_iter().enumerate() {
                projection_operand(index, value);
            }
            for term in 0..4 {
                projection_sources(0x1000, term, 10 + term as i64);
            }
        });
        let storage = holder.storage().unwrap();
        assert!(storage.drain(&mut rows).unwrap());
        assert_eq!(rows[0].projection, [10, 11, 12, 13]);
        assert_eq!(rows[0].projection_sources, sources);
        assert_eq!(rows[0].projection_derived, 15);
        assert!(!storage.drain(&mut rows).unwrap());
        collector_test_support::with_allocations_denied(|| {
            let _flight = Flight::new();
            projection_operand(2, 999);
            projection_sources(0x1001, 1, 80);
            assert!(!storage.missing_projection_sources.load(Ordering::SeqCst));
            assert!(!storage.drain(&mut rows).unwrap());
            projection_sources(0x1000, 1, 81);
        });
        assert_eq!(storage.slots[0].projection[1].load(Ordering::Relaxed), 81);
        assert_eq!(
            storage.drain(&mut rows),
            Err("faction projection setter lacks its supplying getter operands")
        );
    }

    #[test]
    fn ranks_without_a_registered_faction_keep_native_results_and_revoke_old_derivations() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut rows = vec![Account::default()];
        let storage = Storage::new(0x1000, 0xd0, &rows).unwrap();
        for (target, value) in storage.slots[0].rank_pending.iter().zip([300, 0, 77, 8, 1]) {
            target.store(value, Ordering::Relaxed);
        }
        storage.rank_commit(0x1000, 1);
        storage.drain(&mut rows).unwrap();
        assert_eq!(rows[0].rank_refresh, 8);
        for rank in [4, 2, 1] {
            collector_test_support::with_allocations_denied(|| {
                crate::faction_pools::phase_call(0xdead0000, 1, || {
                    assert_eq!(crate::faction_pools::phase(1), None);
                    storage.rank_input(0x1000, 900, 0);
                    storage.rank_commit(0x1000, rank);
                    0
                });
                storage.drain(&mut rows).unwrap();
            });
            assert_eq!(rows[0].projection_rank, rank);
            assert_eq!(rows[0].rank_input_raw, 900);
            assert_eq!(rows[0].rank_input_order, 0);
            assert_eq!(rows[0].rank_refresh, 0);
            assert_eq!(rows[0].rank_faction, 0);
            assert!(!storage.invalid.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn uncertified_shares_revoke_old_total_derivations_but_keep_supplied_operands() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut rows = vec![Account::default()];
        let storage = Storage::new(0x1000, 0xd0, &rows).unwrap();
        storage.slots[0]
            .influence_faction
            .store(77, Ordering::Relaxed);
        storage.slots[0].total_inputs[0].store(8, Ordering::Relaxed);
        storage.slots[0].total_inputs[1].store(300, Ordering::Relaxed);
        collector_test_support::with_allocations_denied(|| {
            crate::faction_pools::phase_call(0xdead0000, 0, || {
                assert_eq!(crate::faction_pools::phase(0), None);
                storage.refresh(0x1000, false, 200, 1000);
                0
            });
            storage.drain(&mut rows).unwrap();
        });
        assert_eq!(rows[0].influence_share_raw, 20_000);
        assert_eq!(rows[0].influence_numerator_raw, 200);
        assert_eq!(rows[0].influence_denominator_raw, 1000);
        assert_eq!(rows[0].influence_refresh, 1);
        assert_eq!(rows[0].influence_faction, 0);
        assert_eq!(rows[0].influence_total_phase, 0);
        assert_eq!(rows[0].influence_total_raw, 200);
    }

    #[test]
    fn all_native_statistic_slots_follow_their_arrays_without_enum_caps() {
        let mut rows = vec![
            Account {
                influence: vec![11; 7],
                projection: vec![17; 6],
                ..Default::default()
            };
            1500
        ];
        let storage = Storage::new(0x1000, 0xd0, &rows).unwrap();
        collector_test_support::with_allocations_denied(|| {
            for account in 0..rows.len() {
                let address = 0x1000 + account * 0xd0;
                for statistic in 0..7 {
                    storage.write(address, true, statistic, i64::MAX, Operation::Set);
                    storage.write(address, true, statistic, 1, Operation::Add);
                }
                for statistic in 0..6 {
                    storage.write(
                        address,
                        false,
                        PROJECTION_START + statistic,
                        -29,
                        Operation::Set,
                    );
                }
            }
            assert!(storage.drain(&mut rows).unwrap());
            assert!(!storage.drain(&mut rows).unwrap());
        });
        for row in &rows {
            assert_eq!(row.influence, [i64::MIN; 7]);
            assert_eq!(row.projection, [-29; 6]);
        }
        storage.write(0x1000, true, 7, 99, Operation::Set);
        assert_eq!(
            storage.drain(&mut rows),
            Err("faction operand writer selected an invalid statistic")
        );
    }

    #[test]
    fn missed_callbacks_require_a_fresh_baseline_instead_of_clearing_the_refusal() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut holder = Holder::default();
        let rows = vec![Account::default()];
        holder
            .install(Storage::new(0x1000, 0xd0, &rows).unwrap())
            .unwrap();
        assert!(!holder.needs_baseline());
        CLOSED.store(true, Ordering::SeqCst);
        {
            let _flight = Flight::new();
            observe(0x1000, false, CONTRIBUTION, 17, Operation::Add);
        }
        CLOSED.store(false, Ordering::SeqCst);
        assert_eq!(quiet(), None);
        assert!(holder.needs_baseline());
        let mut fresh = vec![Account {
            contribution_raw: 29,
            ..Default::default()
        }];
        holder
            .install(Storage::new(0x1000, 0xd0, &fresh).unwrap())
            .unwrap();
        assert!(!holder.needs_baseline());
        assert!(!holder.storage().unwrap().drain(&mut fresh).unwrap());
        assert_eq!(fresh[0].contribution_raw, 29);
    }

    #[test]
    fn rank_alias_selects_canonical_account_and_keeps_its_supplying_certificate() {
        let bytes = super::super::tests::alias_fixture();
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let reader = engine_native::ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
        let mut rows = (0..4)
            .map(|account| Account {
                account,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        rows[3].country = 5;
        let storage = Storage::new(0x8000, 0xd0, &rows).unwrap();
        let canonical = reader.resolve(9).unwrap().canonical_slot;
        assert_eq!(
            reader.resolve(rows[3].country).unwrap().canonical_slot,
            canonical
        );
        let address = rank_address(&storage, Some(canonical)).unwrap();
        assert_eq!(address, 0x8000 + 3 * 0xd0);
        for (target, value) in storage.slots[3].rank_pending.iter().zip([123, 0, 77, 8, 1]) {
            target.store(value, Ordering::Relaxed);
        }
        collector_test_support::with_allocations_denied(|| {
            storage.rank_commit(address, 1);
            storage.drain(&mut rows).unwrap();
        });
        assert_eq!(
            (
                rows[3].rank_input_raw,
                rows[3].rank_faction,
                rows[3].rank_refresh,
                rows[3].rank_members
            ),
            (123, 77, 8, 1)
        );
        assert_eq!(rows[0].rank_refresh, 0);
    }
    #[test]
    fn final_operands_select_one_account_without_native_reads_or_allocations() {
        let mut rows = (0..40)
            .map(|account| Account {
                account,
                influence: vec![0; 5],
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let storage = Storage::new(0x1000, 0xd0, &rows).unwrap();
        collector_test_support::with_allocations_denied(|| {
            storage.write(0x1000 + 39 * 0xd0, true, 2, -7, Operation::Set);
            storage.write(
                0x1000 + 39 * 0xd0,
                false,
                CONTRIBUTION,
                i64::MAX,
                Operation::Add,
            );
            storage.write(0x1000 + 39 * 0xd0, false, CONTRIBUTION, 1, Operation::Add);
            assert!(storage.drain(&mut rows).unwrap());
            assert!(!storage.drain(&mut rows).unwrap());
        });
        assert_eq!(rows[39].influence, [0, 0, -7, 0, 0]);
        assert_eq!(rows[39].contribution_raw, i64::MIN);
        assert_eq!(rows[0].contribution_raw, 0);
    }
    #[test]
    fn passive_boundaries_keep_the_anchor_sparse_and_capture_only_changed_products() {
        let mut rows = vec![Account {
            initiative_raw: 100,
            ..Default::default()
        }];
        let storage = Storage::new(0x1000, 0xd0, &rows).unwrap();
        collector_test_support::with_allocations_denied(|| {
            storage.passive(0x1000, 250_001);
            storage.passive(0x1000, 250_001);
            storage.drain(&mut rows).unwrap();
            assert_eq!(
                (
                    rows[0].initiative_anchor_raw,
                    rows[0].initiative_product_raw,
                    rows[0].initiative_steps
                ),
                (100, 250_001, 2)
            );
            storage.passive(0x1000, -150_001);
            storage.drain(&mut rows).unwrap();
            assert_eq!(
                (
                    rows[0].initiative_anchor_raw,
                    rows[0].initiative_steps,
                    rows[0].initiative_raw
                ),
                (104, 1, 103)
            );
            storage.write(0x1000, false, INITIATIVE, 7, Operation::Set);
            storage.drain(&mut rows).unwrap();
            assert_eq!(
                (rows[0].initiative_anchor_raw, rows[0].initiative_steps),
                (7, 0)
            );
        });
    }
}
