//! Two consumers of the original strategic-navy +0x68 region ledger.
//! Mission display requires nonnull registrations; transport skips null
//! missions and +8 backlinks. Preserve independent results, occurrence
//! order and the first attempted failure. No navy walk, inferred regional
//! membership or pointer publication occurs. Controls live only within
//! the admitted callback.

use crate::{
    Memory, cnaval_mission,
    cpdx_array::{self, Array},
    ctask_force,
};

pub type Fact<T> = Result<T, &'static str>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerObservation {
    pub required: usize,
    /// Complete nullable pointer list; zero is a native null occurrence.
    pub nullable: Fact<usize>,
    /// The strict ledger's capacity request or first null/read failure.
    pub strict: Fact<usize>,
}

/// Replaces the original pointer walk, rather than repeating it. An exhausted
/// arena keeps the strict reader's no-read capacity request; an incomplete raw
/// prefix may be inspected but cannot be published as a complete transport list.
pub fn observe_ledger(
    memory: &impl Memory,
    missions: Array,
    output: &mut [usize],
) -> LedgerObservation {
    let required = missions.count as usize;
    if required > output.len() {
        return LedgerObservation {
            required,
            nullable: Err("transport nullable ledger arena exhausted"),
            strict: Ok(required),
        };
    }
    let mut strict = Ok(required);
    let copied = cpdx_array::pointers(
        memory,
        missions,
        "navy ledger mission unreadable or null",
        |index, mission| {
            if mission == 0 && strict.is_ok() {
                strict = Err("navy ledger mission unreadable or null");
            }
            output[index as usize] = mission;
            Ok(())
        },
    );
    if let Err(reason) = copied
        && strict.is_ok()
    {
        strict = Err(reason);
    }
    LedgerObservation {
        required,
        nullable: copied.map(|()| required),
        strict,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Receiver {
    pub pointer: usize,
    pub full_handle: u64,
}

/// Consume the original mission header and full unit certificate. Null
/// mission/backlink skips precede receiver authentication. The certificate
/// is the counter-identity class/vtable/handle/registry join, separate
/// from fleet-roster identity. The callback owns world, hour and quiescent
/// boundary admission; there is no generation word.
pub fn receiver(
    base: usize,
    mission: usize,
    original_header: Fact<&[u8]>,
    original_unit_identity: impl FnOnce(usize) -> Fact<(u64, u32)>,
) -> Fact<Option<Receiver>> {
    if mission == 0 {
        return Ok(None);
    }
    let header: &[u8; 16] = original_header?
        .try_into()
        .map_err(|_| "transport original mission header incomplete")?;
    let pointer = usize::try_from(u64::from_le_bytes(header[8..].try_into().unwrap()))
        .map_err(|_| "transport mission receiver pointer overflow")?;
    if pointer == 0 {
        return Ok(None);
    }
    let expected = base
        .checked_add(cnaval_mission::VTABLE_RVA)
        .ok_or("transport mission module overflow")? as u64;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected
        || pointer.checked_add(ctask_force::NAVAL_MISSION) != Some(mission)
    {
        return Err("transport original mission identity refused");
    }
    let (full_handle, class) = original_unit_identity(pointer)?;
    if class != 1 {
        return Err("transport mission receiver is not original class1");
    }
    Ok(Some(Receiver {
        pointer,
        full_handle,
    }))
}

/// Off-thread sizing of the same nullable mission receiver from its
/// original ship array; no model or runtime value is recorded.
pub fn ship_demand(memory: &impl Memory, base: usize, mission: usize) -> Fact<usize> {
    if mission == 0 {
        return Ok(0);
    }
    let header = crate::read::<[u8; 16]>(memory, mission)
        .ok_or("transport mission sizing header unreadable")?;
    let mut registry = crate::creference_registry::HeaderResolver::default();
    let receiver = receiver(base, mission, Ok(&header), |pointer| {
        crate::cunit::counter_identity(memory, base, pointer, &mut registry)
    })?;
    let Some(receiver) = receiver else {
        return Ok(0);
    };
    let address = receiver
        .pointer
        .checked_add(ctask_force::SHIPS)
        .ok_or("transport ship sizing address overflow")?;
    let ships = Array::read(memory, address, 8).ok_or("transport ship sizing header invalid")?;
    Ok(ships.count as usize)
}

/// One attempted identity, successful or refused, retained by primary pointer.
/// The arena is transient and must be cleared before callback return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverIdentity {
    pub pointer: usize,
    pub identity: Fact<(u64, u32)>,
}

impl Default for ReceiverIdentity {
    fn default() -> Self {
        Self {
            pointer: 0,
            identity: Err("transport receiver identity not observed"),
        }
    }
}

/// Checked off-thread storage demand for a half-full same-pass pointer index.
pub fn index_capacity(entries: usize) -> Option<usize> {
    if entries == 0 {
        return Some(0);
    }
    entries.checked_mul(2)?.checked_next_power_of_two()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerSlot {
    pointer: usize,
    index: usize,
}

impl Default for PointerSlot {
    fn default() -> Self {
        Self {
            pointer: 0,
            index: usize::MAX,
        }
    }
}

/// Caller-owned hash index for original pointer observations, including failed
/// headers or identities. Values index the caller's admitted arena. Clearing
/// and lookup allocate nothing; the index owns no native observation or lease.
pub struct PointerIndex<'a> {
    slots: crate::walk::Buffer<'a, PointerSlot>,
    entry_capacity: usize,
    len: usize,
}

impl<'a> PointerIndex<'a> {
    pub fn new(slots: &'a mut [PointerSlot], entry_capacity: usize) -> Fact<Self> {
        Self::from_buffer(crate::walk::Buffer::Fixed(slots), entry_capacity)
    }

    /// Worker-only index growth preserves the original observations' indices.
    pub fn new_growing(slots: &'a mut Vec<PointerSlot>, entry_capacity: usize) -> Fact<Self> {
        Self::from_buffer(crate::walk::Buffer::Growing(slots), entry_capacity)
    }

    pub(crate) fn from_buffer(
        mut slots: crate::walk::Buffer<'a, PointerSlot>,
        entry_capacity: usize,
    ) -> Fact<Self> {
        let required =
            index_capacity(entry_capacity).ok_or("transport pointer index capacity overflow")?;
        if !slots.ensure(required)? || (!slots.is_empty() && !slots.len().is_power_of_two()) {
            return Err("transport pointer index arena exhausted");
        }
        slots.fill(PointerSlot::default());
        Ok(Self {
            slots,
            entry_capacity,
            len: 0,
        })
    }

    fn slot(&self, pointer: usize) -> usize {
        Self::slot_in(pointer, self.slots.len())
    }

    fn slot_in(pointer: usize, count: usize) -> usize {
        // Internal hash only; no game ordering, identity or hash rule is inferred.
        let mut hash = pointer as u64;
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((hash ^ (hash >> 31)) as usize) & (count - 1)
    }

    fn grow_entries(&mut self, required: usize) -> Fact<()> {
        if required <= self.entry_capacity {
            return Ok(());
        }
        let crate::walk::Buffer::Growing(slots) = &mut self.slots else {
            return Err("transport pointer index entries exhausted");
        };
        let capacity =
            index_capacity(required).ok_or("transport pointer index capacity overflow")?;
        let mut grown = crate::allocation::default_vec::<PointerSlot>(capacity)
            .map_err(|_| "transport pointer index allocation failed")?;
        for entry in slots.iter().filter(|entry| entry.index != usize::MAX) {
            let mut slot = Self::slot_in(entry.pointer, grown.len());
            while grown[slot].index != usize::MAX {
                slot = (slot + 1) & (grown.len() - 1);
            }
            grown[slot] = *entry;
        }
        **slots = grown;
        self.entry_capacity = capacity / 2;
        Ok(())
    }

    pub fn find(&self, pointer: usize) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        let mut slot = self.slot(pointer);
        loop {
            let entry = self.slots[slot];
            if entry.index == usize::MAX {
                return None;
            }
            if entry.pointer == pointer {
                return Some(entry.index);
            }
            slot = (slot + 1) & (self.slots.len() - 1);
        }
    }

    pub fn insert(&mut self, pointer: usize, index: usize) -> Fact<()> {
        if let Some(previous) = self.find(pointer) {
            return (previous == index)
                .then_some(())
                .ok_or("transport original pointer index differs");
        }
        self.grow_entries(
            index
                .checked_add(1)
                .ok_or("transport pointer index capacity overflow")?
                .max(self.len + 1),
        )?;
        if self.slots.is_empty() {
            return Err("transport pointer index entries exhausted");
        }
        let mut slot = self.slot(pointer);
        loop {
            let entry = self.slots[slot];
            if entry.index == usize::MAX {
                if self.len == self.entry_capacity || index >= self.entry_capacity {
                    return Err("transport pointer index entries exhausted");
                }
                self.slots[slot] = PointerSlot { pointer, index };
                self.len += 1;
                return Ok(());
            }
            if entry.pointer == pointer {
                return (entry.index == index)
                    .then_some(())
                    .ok_or("transport original pointer index differs");
            }
            slot = (slot + 1) & (self.slots.len() - 1);
        }
    }
}

impl Drop for PointerIndex<'_> {
    fn drop(&mut self) {
        self.slots.fill(PointerSlot::default());
    }
}

/// Same-pass receiver authentication with fixed callback or growing worker
/// storage. Both the unit facts and
/// the registry's null/failed roots are retained; a repeat cannot heal failure.
pub struct IdentityReader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    registry: crate::creference_registry::ScopedResolver,
    entries: crate::walk::Buffer<'a, ReceiverIdentity>,
    index: PointerIndex<'a>,
    len: usize,
    required: usize,
}

impl<'a, M: Memory> IdentityReader<'a, M> {
    pub fn new(
        memory: &'a M,
        base: usize,
        entries: &'a mut [ReceiverIdentity],
        slots: &'a mut [PointerSlot],
    ) -> Fact<Self> {
        let index = PointerIndex::new(slots, entries.len())?;
        Self::from_buffers(memory, base, crate::walk::Buffer::Fixed(entries), index)
    }

    pub fn new_growing(
        memory: &'a M,
        base: usize,
        entries: &'a mut Vec<ReceiverIdentity>,
        slots: &'a mut Vec<PointerSlot>,
    ) -> Fact<Self> {
        let index = PointerIndex::new_growing(slots, entries.len())?;
        Self::from_buffers(memory, base, crate::walk::Buffer::Growing(entries), index)
    }

    fn from_buffers(
        memory: &'a M,
        base: usize,
        entries: crate::walk::Buffer<'a, ReceiverIdentity>,
        index: PointerIndex<'a>,
    ) -> Fact<Self> {
        Ok(Self {
            memory,
            base,
            registry: crate::creference_registry::ScopedResolver::new(base),
            entries,
            index,
            len: 0,
            required: 0,
        })
    }

    pub fn observe(&mut self, pointer: usize) -> Fact<(u64, u32)> {
        if let Some(index) = self.index.find(pointer) {
            return self.entries[index].identity;
        }
        // Once exhausted, unknown distinct receivers cannot be counted exactly.
        // Keep an explicit floor, without reading or retrying any native input.
        self.required = self.required.max(self.len.saturating_add(1));
        // Transport queries reach the original receiver certificate once.
        self.entries.ensure(self.required)?;
        let Some(entry) = self.entries.get_mut(self.len) else {
            return Err("transport receiver identity arena exhausted");
        };
        self.index.insert(pointer, self.len)?;
        let identity =
            crate::cunit::counter_identity_with(self.memory, self.base, pointer, |handle| {
                self.registry.resolve(self.memory, handle)
            });
        *entry = ReceiverIdentity { pointer, identity };
        self.len += 1;
        identity
    }

    pub fn find(&self, pointer: usize) -> Option<usize> {
        self.index.find(pointer)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Minimum distinct receiver capacity witnessed before an early refusal.
    pub fn required(&self) -> usize {
        self.required
    }
}

/// Ship queries are reached only for nonzero mission +0x30 and signed
/// unit +0x1B4 <= 0. Borrow original receiver observations. Earlier skips
/// do not demand later operands, and failed demand stays failed.
pub fn risk_demand(
    ledger_occurrence: Fact<bool>,
    authenticated_backlink: Fact<bool>,
    in_regions: Fact<u8>,
    combat_references: Fact<i32>,
) -> Fact<bool> {
    if !ledger_occurrence? || !authenticated_backlink? || in_regions? == 0 {
        return Ok(false);
    }
    Ok(combat_references? <= 0)
}

#[cfg(test)]
mod tests;
