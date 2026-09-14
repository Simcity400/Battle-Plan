//! Hourly native ETA operands from the shared division census. Route duration
//! arithmetic and row conversion consume owned scalars on the spool thread.
use super::production::Sources;
use crate::{
    sampling::qpc,
    spooling::{table_iter, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, cequipment_variant_pool::DivisionSource, cunit, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{DivisionEtaRow, TableId, tables},
};
use std::{cell::RefCell, mem::size_of};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionEta];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    legs: usize,
    cache_entries: usize,
    transport_entries: usize,
    modifier_accumulators: usize,
    modifier_bytes: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 8192,
            legs: 32 * 1024,
            cache_entries: 32 * 1024,
            transport_entries: 1024,
            modifier_accumulators: 1024,
            modifier_bytes: 32 * 1024 * 16,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<cunit::EtaInputs>,
    legs: Arena<cunit::EtaLeg>,
    cache: RefCell<SmallReadCache>,
    transport: TransportMemo,
    modifiers: RefCell<ModifierSnapshots>,
}
impl Captured {
    pub(crate) fn empty() -> Self {
        Self::with_capacities(
            &Capacities {
                rows: 0,
                legs: 0,
                cache_entries: 0,
                transport_entries: 0,
                modifier_accumulators: 0,
                modifier_bytes: 0,
            },
            &[0],
        )
    }
    pub(crate) fn fail(&mut self, reason: &'static str, checked: bool) {
        self.reset();
        self.rows.outcome = Outcome::Unavailable { reason };
        self.rows.checked = checked;
    }
    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(c.rows, successful[0]),
            legs: Arena::new(c.legs, successful[0]),
            cache: RefCell::new(SmallReadCache::new(c.cache_entries)),
            transport: TransportMemo::new(c.transport_entries),
            modifiers: RefCell::new(ModifierSnapshots::new(
                c.modifier_accumulators,
                c.modifier_bytes,
            )),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.legs.reset();
        self.cache.get_mut().clear();
        self.transport.clear();
        self.modifiers.get_mut().clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.legs.allocated_bytes()
            + self.cache.borrow().allocated_bytes()
            + self.transport.allocated_bytes()
            + self.modifiers.borrow().allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.rows.grow(&mut c.rows, successful[0]);
        self.legs.grow(&mut c.legs, successful[0]);
        self.cache.get_mut().grow(c.cache_entries);
        self.transport.grow(c.transport_entries);
        self.modifiers
            .get_mut()
            .grow(c.modifier_accumulators, c.modifier_bytes);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
        sources: Sources<'_, DivisionSource>,
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        self.reset();
        let started = qpc();
        let cached = CacheRef {
            source: memory,
            cache: &self.cache,
            modifiers: Some(&self.modifiers),
        };
        let mut first = cunit::walk_eta_with_transport(
            &cached,
            base,
            state,
            sources.first,
            sources.first_outcome,
            &mut self.rows.rows,
            &mut self.legs.first,
            |owner, region| {
                self.transport.resolve(owner, region, || {
                    engine::cnaval_unit_transfer::movement_speed(
                        &cached, base, state, owner, region,
                    )
                })
            },
        );
        if !self.transport.verify_and_clear(|owner, region| {
            engine::cnaval_unit_transfer::movement_speed(memory, base, state, owner, region)
        }) {
            first.outcome = Outcome::Unavailable {
                reason: "division ETA transport speed changed or unreadable",
            };
        }
        if !self.modifiers.borrow_mut().verify_and_clear(memory) {
            first.outcome = Outcome::Unavailable {
                reason: "division ETA modifier snapshot changed or unreadable",
            };
        }
        if !self.cache.borrow_mut().verify_and_clear(memory) {
            first.outcome = Outcome::Unavailable {
                reason: "division ETA cached operands changed or unreadable",
            };
        }
        self.rows.outcome = first.outcome;
        self.legs.len = first.legs_len;
        self.legs.required = first.required_legs;
        let mut pass = false;
        if needed[0] {
            self.rows.checked = true;
            let mut second = cunit::walk_eta_with_transport(
                &cached,
                base,
                state,
                sources.second,
                sources.second_outcome,
                &mut self.rows.second,
                &mut self.legs.second,
                |owner, region| {
                    self.transport.resolve(owner, region, || {
                        engine::cnaval_unit_transfer::movement_speed(
                            &cached, base, state, owner, region,
                        )
                    })
                },
            );
            if !self.transport.verify_and_clear(|owner, region| {
                engine::cnaval_unit_transfer::movement_speed(memory, base, state, owner, region)
            }) {
                second.outcome = Outcome::Unavailable {
                    reason: "division ETA transport speed changed or unreadable",
                };
            }
            if !self.modifiers.borrow_mut().verify_and_clear(memory) {
                second.outcome = Outcome::Unavailable {
                    reason: "division ETA modifier snapshot changed or unreadable",
                };
            }
            if !self.cache.borrow_mut().verify_and_clear(memory) {
                second.outcome = Outcome::Unavailable {
                    reason: "division ETA cached operands changed or unreadable",
                };
            }
            let rows = self.rows.compare(second.outcome);
            let legs = self.legs.compare(second.legs_len, second.required_legs);
            self.rows.mismatches += usize::from(!legs);
            pass = rows && legs;
        }
        ([pass], [qpc().saturating_sub(started).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "division ETA operands changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcome = self.outcomes()[0];
        if let Outcome::Unavailable { reason } = outcome {
            return [unavailable(reason)];
        }
        let Some(inputs) = self.rows.rows.get(..outcome.len()) else {
            return [unavailable("division ETA row span invalid")];
        };
        let Some(legs) = self.legs.first.get(..self.legs.len) else {
            return [unavailable("division ETA leg arena invalid")];
        };
        let rows: Result<Vec<_>, _> = inputs
            .iter()
            .map(|input| {
                let (next_remaining, destination_remaining) = cunit::eta_remaining(input, legs)?;
                Ok(DivisionEtaRow {
                    division: input.division,
                    next: input.next,
                    destination: input.destination,
                    path_total: input.path_total,
                    next_remaining,
                    destination_remaining,
                })
            })
            .collect();
        let rows = match rows {
            Ok(rows) => rows,
            Err(reason) => return [unavailable(reason)],
        };
        [table_iter::<tables::DivisionEta, _>(
            outcome,
            rows.into_iter(),
            std::convert::identity,
        )]
    }
}

/// Owned copies of the existing authenticated division census. Allocation and
/// growth are off-thread; omitted source rows remain explicit truncation.
pub(crate) struct OwnedSources {
    first: Vec<DivisionSource>,
    second: Vec<DivisionSource>,
    outcomes: [Outcome; 2],
}
impl OwnedSources {
    pub(crate) fn new(capacity: usize, checked: bool) -> Self {
        Self {
            first: vec![DivisionSource::default(); capacity],
            second: vec![DivisionSource::default(); if checked { capacity } else { 0 }],
            outcomes: [super::MISSING; 2],
        }
    }
    pub(crate) fn copy_from(&mut self, source: Sources<'_, DivisionSource>, checked: bool) {
        self.clear();
        for (pass, (input, outcome)) in [
            (source.first, source.first_outcome),
            (source.second, source.second_outcome),
        ]
        .into_iter()
        .enumerate()
        {
            if pass == 1 && !checked {
                break;
            }
            let output = if pass == 0 {
                &mut self.first
            } else {
                &mut self.second
            };
            self.outcomes[pass] = match outcome {
                Outcome::Unavailable { .. } => outcome,
                Outcome::Truncated { unread_rows: 0, .. } => Outcome::Unavailable {
                    reason: "ETA parent truncation has no omitted rows",
                },
                _ if outcome.len() > input.len() => Outcome::Unavailable {
                    reason: "ETA source count exceeds parent storage",
                },
                _ => {
                    let len = outcome.len().min(output.len());
                    output[..len].copy_from_slice(&input[..len]);
                    let missing = match outcome {
                        Outcome::Truncated { unread_rows, .. } => unread_rows as usize,
                        _ => 0,
                    };
                    let omitted = outcome.len().saturating_sub(len).saturating_add(missing);
                    if omitted == 0 {
                        Outcome::Rows { len }
                    } else {
                        super::truncated_source(len, omitted)
                    }
                }
            };
        }
    }
    pub(crate) fn view(&self) -> Sources<'_, DivisionSource> {
        Sources {
            first: &self.first,
            first_outcome: self.outcomes[0],
            second: &self.second,
            second_outcome: self.outcomes[1],
        }
    }
    pub(crate) fn clear(&mut self) {
        for (rows, outcome) in [
            (&mut self.first, self.outcomes[0]),
            (&mut self.second, self.outcomes[1]),
        ] {
            let len = rows.len().min(outcome.len());
            rows[..len].fill(DivisionSource::default());
        }
        self.outcomes = [super::MISSING; 2];
    }
    pub(crate) fn grow(&mut self, capacity: usize, checked: bool) {
        self.clear();
        crate::table_capture::grow_vec(&mut self.first, capacity);
        crate::table_capture::comparison(
            &mut self.second,
            capacity,
            if checked {
                0
            } else {
                crate::sampling::CONSISTENCY_TARGET
            },
        );
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        (self.first.capacity() + self.second.capacity()) * size_of::<DivisionSource>()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AcquisitionReport {
    pub matched: bool,
    pub ticks: u64,
}

const SMALL_READ_BYTES: usize = 32;

#[derive(Clone, Copy, Default)]
struct ModifierSnapshot {
    address: usize,
    header: [u8; 16],
    start: usize,
    len: usize,
    occupied: bool,
}

/// Full raw sorted accumulators, owned only until this pass's direct final
/// verification. A collision or exhausted arena uses the original live reader;
/// no eviction, allocation, partial snapshot, or cross-pass reuse is allowed.
struct ModifierSnapshots {
    entries: Vec<ModifierSnapshot>,
    used: Vec<usize>,
    used_len: usize,
    bytes: Vec<u8>,
    bytes_len: usize,
}

impl ModifierSnapshots {
    fn new(capacity: usize, bytes: usize) -> Self {
        Self {
            entries: vec![ModifierSnapshot::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
            bytes: vec![0; bytes],
            bytes_len: 0,
        }
    }

    fn lookup(
        &mut self,
        source: &impl Memory,
        address: usize,
        key: u32,
    ) -> Option<Result<engine::modifier_accumulator::Value, &'static str>> {
        if self.entries.is_empty() {
            return None;
        }
        let slot = ((address >> 4) ^ (address >> 17)) % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if entry.address == address {
                Some(engine::modifier_accumulator::lookup_snapshot(
                    &self.bytes[entry.start..entry.start + entry.len],
                    key,
                ))
            } else {
                None
            };
        }
        let Some(header) = engine::read::<[u8; 16]>(source, address) else {
            return Some(Err("modifier accumulator header unreadable or invalid"));
        };
        let Some(array) =
            engine::cpdx_array::Array::from_header(header, engine::modifier_entry::STRIDE)
        else {
            return Some(Err("modifier accumulator header unreadable or invalid"));
        };
        let Some(len) = (array.count as usize).checked_mul(engine::modifier_entry::STRIDE) else {
            return Some(Err("modifier snapshot byte length overflow"));
        };
        let end = self
            .bytes_len
            .checked_add(len)
            .filter(|&end| end <= self.bytes.len())?;
        let bytes = &mut self.bytes[self.bytes_len..end];
        if len != 0 && !engine::read_slice(source, array.data, bytes) {
            return Some(Err("modifier snapshot entries unreadable"));
        }
        if let Err(reason) = engine::modifier_accumulator::validate_snapshot(bytes) {
            return Some(Err(reason));
        }
        if engine::read::<[u8; 16]>(source, address) != Some(header) {
            return Some(Err("modifier accumulator header changed"));
        }
        let result = engine::modifier_accumulator::lookup_snapshot(bytes, key);
        self.entries[slot] = ModifierSnapshot {
            address,
            header,
            start: self.bytes_len,
            len,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        self.bytes_len = end;
        Some(result)
    }

    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = ModifierSnapshot::default();
        }
        self.used_len = 0;
        self.bytes_len = 0;
    }

    fn verify_and_clear(&mut self, source: &impl Memory) -> bool {
        let mut matched = true;
        let mut fresh = [0_u8; 64 * 16];
        for &slot in &self.used[..self.used_len] {
            let entry = self.entries[slot];
            matched &= engine::read::<[u8; 16]>(source, entry.address) == Some(entry.header);
            // The stored header was fully bounded before insertion. Continue
            // checking all retained bytes even if a prior header/read failed.
            let data = u64::from_le_bytes(entry.header[..8].try_into().unwrap()) as usize;
            let mut offset = 0;
            while offset < entry.len {
                let count = (entry.len - offset).min(fresh.len());
                let copied = engine::read_slice(source, data + offset, &mut fresh[..count]);
                matched &= copied
                    && fresh[..count]
                        == self.bytes[entry.start + offset..entry.start + offset + count];
                offset += count;
            }
            matched &= engine::read::<[u8; 16]>(source, entry.address) == Some(entry.header);
            self.entries[slot] = ModifierSnapshot::default();
        }
        self.used_len = 0;
        self.bytes_len = 0;
        matched
    }

    fn grow(&mut self, capacity: usize, bytes: usize) {
        self.clear();
        if capacity > self.entries.len() {
            self.entries.resize(capacity, ModifierSnapshot::default());
            self.used.resize(capacity, 0);
        }
        if bytes > self.bytes.len() {
            self.bytes.resize(bytes, 0);
        }
    }

    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<ModifierSnapshot>()
            + self.used.capacity() * size_of::<usize>()
            + self.bytes.capacity()
    }
}

#[derive(Clone, Copy, Default)]
struct TransportEntry {
    owner: u32,
    region: usize,
    speed: i64,
    occupied: bool,
}

/// Derived results are local to one base/game-state/pass. Even reads which
/// collided in SmallReadCache participate in the direct final recomputation of
/// every retained result. No eviction or inferred success on failed misses.
struct TransportMemo {
    entries: Vec<TransportEntry>,
    used: Vec<usize>,
    used_len: usize,
}
impl TransportMemo {
    fn new(capacity: usize) -> Self {
        Self {
            entries: vec![TransportEntry::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
        }
    }
    fn resolve(
        &mut self,
        owner: u32,
        region: usize,
        calculate: impl FnOnce() -> Result<i64, &'static str>,
    ) -> Result<i64, &'static str> {
        if self.entries.is_empty() {
            return calculate();
        }
        let hash = (owner as usize).wrapping_mul(0x9e37_79b9) ^ (region >> 3) ^ (region >> 19);
        let slot = hash % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if entry.owner == owner && entry.region == region {
                Ok(entry.speed)
            } else {
                calculate()
            };
        }
        let speed = calculate()?;
        self.entries[slot] = TransportEntry {
            owner,
            region,
            speed,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        Ok(speed)
    }
    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = TransportEntry::default();
        }
        self.used_len = 0;
    }
    fn verify_and_clear(
        &mut self,
        mut calculate: impl FnMut(u32, usize) -> Result<i64, &'static str>,
    ) -> bool {
        let mut matched = true;
        for &slot in &self.used[..self.used_len] {
            let entry = &mut self.entries[slot];
            let result = calculate(entry.owner, entry.region);
            matched &= result == Ok(entry.speed);
            *entry = TransportEntry::default();
        }
        self.used_len = 0;
        matched
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if self.entries.len() < capacity {
            self.entries.resize(capacity, TransportEntry::default());
            self.used.resize(capacity, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<TransportEntry>()
            + self.used.capacity() * size_of::<usize>()
    }
}

#[derive(Clone, Copy, Default)]
struct CacheEntry {
    address: usize,
    len: usize,
    bytes: [u8; SMALL_READ_BYTES],
}

/// Callback-local exact reads only. Occupied slots are never evicted: a
/// collision is acquired normally, leaving every retained observation available
/// for the mandatory direct verification at the end of this independent pass.
struct SmallReadCache {
    entries: Vec<CacheEntry>,
    used: Vec<usize>,
    used_len: usize,
}
impl SmallReadCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: vec![CacheEntry::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
        }
    }
    fn slot(&self, address: usize, len: usize) -> Option<usize> {
        if self.entries.is_empty() || len == 0 || len > SMALL_READ_BYTES {
            None
        } else {
            let hash = (address >> 3) ^ (address >> 19) ^ len.wrapping_mul(0x9e37_79b9);
            Some(hash % self.entries.len())
        }
    }
    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = CacheEntry::default();
        }
        self.used_len = 0;
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if self.entries.len() < capacity {
            self.entries.resize(capacity, CacheEntry::default());
            self.used.resize(capacity, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<CacheEntry>()
            + self.used.capacity() * size_of::<usize>()
    }
    fn verify_and_clear(&mut self, memory: &impl Memory) -> bool {
        let mut matched = true;
        let mut fresh = [0; SMALL_READ_BYTES];
        for &slot in &self.used[..self.used_len] {
            let entry = &mut self.entries[slot];
            // Do not short-circuit after a mismatch: every retained read must
            // be directly checked, even when the enclosing walk already failed.
            let copied = memory.copy(entry.address, &mut fresh[..entry.len]);
            matched &= copied && fresh[..entry.len] == entry.bytes[..entry.len];
            *entry = CacheEntry::default();
        }
        self.used_len = 0;
        matched
    }
}

struct CacheRef<'a, M> {
    source: &'a M,
    cache: &'a RefCell<SmallReadCache>,
    modifiers: Option<&'a RefCell<ModifierSnapshots>>,
}
impl<M: Memory> Memory for CacheRef<'_, M> {
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<engine::modifier_accumulator::Value, &'static str>> {
        self.modifiers?
            .borrow_mut()
            .lookup(self.source, address, key)
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let slot = {
            let cache = self.cache.borrow();
            let Some(slot) = cache.slot(address, destination.len()) else {
                return self.source.copy(address, destination);
            };
            let entry = &cache.entries[slot];
            if entry.len == destination.len() && entry.address == address {
                destination.copy_from_slice(&entry.bytes[..entry.len]);
                return true;
            }
            (slot, entry.len == 0)
        };
        if !self.source.copy(address, destination) {
            return false;
        }
        if slot.1 {
            let mut cache = self.cache.borrow_mut();
            let mut entry = CacheEntry {
                address,
                len: destination.len(),
                ..CacheEntry::default()
            };
            entry.bytes[..destination.len()].copy_from_slice(destination);
            cache.entries[slot.0] = entry;
            let next = cache.used_len;
            cache.used[next] = slot.0;
            cache.used_len += 1;
        }
        true
    }
    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }
    // No immutable_view: callers must not bypass acquisition or final checks.
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn conversion_sorts_census_order_and_rejects_duplicate_handles() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 3,
                legs: 0,
                cache_entries: 0,
                transport_entries: 0,
                modifier_accumulators: 0,
                modifier_bytes: 0,
            },
            &[0],
        );
        for (row, division) in capture.rows.rows.iter_mut().zip([9, 2, 7]) {
            row.division = division;
        }
        for outcome in [
            Outcome::Rows { len: 3 },
            Outcome::Truncated {
                len: 3,
                unread_rows: 4,
            },
        ] {
            capture.rows.outcome = outcome;
            let [view] = capture.tables();
            let rows = match view {
                TableView::Complete(TableRows::DivisionEta(rows)) => rows,
                TableView::Truncated {
                    rows: TableRows::DivisionEta(rows),
                    missing: 4,
                } => rows,
                other => panic!("unexpected ETA view {other:?}"),
            };
            assert_eq!(
                rows.iter().map(|r| r.division).collect::<Vec<_>>(),
                [2, 7, 9]
            );
        }
        capture.rows.rows[2].division = 2;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
        capture.rows.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 0,
        };
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }
    use crate::sampling::CONSISTENCY_TARGET;
    use std::cell::Cell;

    #[test]
    fn owned_eta_sources_preserve_both_passes_capacity_faults_and_release_pointers() {
        let first = [
            DivisionSource {
                pointer: 0x1000,
                division: 1,
                ..Default::default()
            },
            DivisionSource {
                pointer: 0x2000,
                division: 2,
                ..Default::default()
            },
        ];
        let second = [DivisionSource {
            pointer: 0x3000,
            division: 3,
            ..Default::default()
        }];
        let mut owned = OwnedSources::new(1, true);
        owned.copy_from(
            Sources {
                first: &first,
                first_outcome: Outcome::Truncated {
                    len: 2,
                    unread_rows: 3,
                },
                second: &second,
                second_outcome: Outcome::Rows { len: 1 },
            },
            true,
        );
        assert_eq!(
            owned.outcomes,
            [
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 4
                },
                Outcome::Rows { len: 1 }
            ]
        );
        assert_eq!(owned.first[0], first[0]);
        assert_eq!(owned.second[0], second[0]);
        owned.clear();
        assert_eq!(owned.first[0].pointer, 0);
        assert_eq!(owned.second[0].pointer, 0);
        owned.copy_from(
            Sources {
                first: &first,
                first_outcome: Outcome::Unavailable {
                    reason: "parent fault",
                },
                second: &second,
                second_outcome: Outcome::Rows { len: 2 },
            },
            true,
        );
        assert_eq!(
            owned.outcomes[0],
            Outcome::Unavailable {
                reason: "parent fault"
            }
        );
        assert!(matches!(owned.outcomes[1], Outcome::Unavailable { .. }));
        let bytes = owned.allocated_bytes();
        owned.grow(2, false);
        assert_eq!(owned.first.len(), 2);
        assert!(owned.second.is_empty());
        assert!(owned.allocated_bytes() >= bytes);
        owned.copy_from(
            Sources {
                first: &first,
                first_outcome: Outcome::Rows { len: 2 },
                second: &second,
                second_outcome: Outcome::Rows { len: 1 },
            },
            false,
        );
        assert_eq!(owned.outcomes[0], Outcome::Rows { len: 2 });
        assert_eq!(owned.outcomes[1], super::super::MISSING);
    }

    struct Observed {
        bytes: RefCell<[u8; 256]>,
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Cell<Option<usize>>,
    }
    impl Observed {
        fn new() -> Self {
            Self {
                bytes: RefCell::new(std::array::from_fn(|i| i as u8)),
                calls: RefCell::new(Vec::new()),
                fault: Cell::new(None),
            }
        }
        fn accumulator(&self, address: usize, data: usize, values: &[(u32, i64)]) {
            let mut bytes = self.bytes.borrow_mut();
            bytes[address..address + 16].fill(0);
            bytes[address..address + 8].copy_from_slice(&(data as u64).to_le_bytes());
            bytes[address + 8..address + 12].copy_from_slice(&(values.len() as u32).to_le_bytes());
            bytes[address + 12..address + 16].copy_from_slice(&(values.len() as u32).to_le_bytes());
            for (index, (key, raw)) in values.iter().enumerate() {
                let at = data + index * 16;
                bytes[at..at + 16].fill(0);
                bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
                bytes[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
            }
        }
    }
    impl Memory for Observed {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            if self.fault.get() == Some(address) {
                destination.fill(255);
                return false;
            }
            let Some(end) = address.checked_add(destination.len()) else {
                return false;
            };
            let bytes = self.bytes.borrow();
            let Some(source) = bytes.get(address..end) else {
                return false;
            };
            destination.copy_from_slice(source);
            true
        }
    }

    #[test]
    fn modifier_snapshot_batches_all_entries_and_reuses_raw_presence_only() {
        use engine::modifier_accumulator::{Value, lookup_at};
        let source = Observed::new();
        source.accumulator(16, 64, &[(2, i64::MIN), (5, 0), (9, i64::MAX)]);
        let cache = RefCell::new(SmallReadCache::new(1));
        let modifiers = RefCell::new(ModifierSnapshots::new(8, 128));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: Some(&modifiers),
        };
        for _ in 0..3 {
            assert_eq!(
                lookup_at(&memory, 16, 2),
                Ok(Value {
                    raw: i64::MIN,
                    present: true
                })
            );
            assert_eq!(
                lookup_at(&memory, 16, 5),
                Ok(Value {
                    raw: 0,
                    present: true
                })
            );
            assert_eq!(
                lookup_at(&memory, 16, 9),
                Ok(Value {
                    raw: i64::MAX,
                    present: true
                })
            );
            assert_eq!(lookup_at(&memory, 16, 6), Ok(Value::default()));
        }
        assert_eq!(&*source.calls.borrow(), &[(16, 16), (64, 48), (16, 16)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(modifiers.borrow_mut().verify_and_clear(&source));
        assert_eq!(&source.calls.borrow()[3..], &[(16, 16), (64, 48), (16, 16)]);
        assert_eq!(modifiers.borrow().used_len, 0);
        assert_eq!(modifiers.borrow().bytes_len, 0);
        assert!(
            modifiers
                .borrow()
                .entries
                .iter()
                .all(|entry| !entry.occupied && entry.address == 0 && entry.header == [0; 16])
        );
    }

    #[test]
    fn modifier_snapshot_verifies_unused_keys_values_padding_and_all_header_fields() {
        for mutation in [16, 24, 28, 96, 100, 104] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 0), (9, 13)]);
            let mut snapshots = ModifierSnapshots::new(8, 128);
            let first = snapshots.lookup(&source, 16, 2).unwrap().unwrap();
            source.bytes.borrow_mut()[mutation] ^= 1;
            assert_eq!(snapshots.lookup(&source, 16, 2), Some(Ok(first)));
            assert!(
                !snapshots.verify_and_clear(&source),
                "missed mutation at {mutation}"
            );
            assert_eq!(snapshots.used_len, 0);
            assert_eq!(snapshots.bytes_len, 0);
        }
        let source = Observed::new();
        source.accumulator(16, 64, &[(2, 7)]);
        let mut snapshots = ModifierSnapshots::new(8, 128);
        assert_eq!(snapshots.lookup(&source, 16, 2).unwrap().unwrap().raw, 7);
        source.bytes.borrow_mut()[72..80].copy_from_slice(&11_i64.to_le_bytes());
        assert!(!snapshots.verify_and_clear(&source));
        assert_eq!(snapshots.lookup(&source, 16, 2).unwrap().unwrap().raw, 11);
        assert!(snapshots.verify_and_clear(&source));
    }

    #[test]
    fn modifier_snapshot_faults_check_every_retained_array_and_clear_failed_passes() {
        for fault in [16, 64, 32, 128] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 11)]);
            source.accumulator(32, 128, &[(3, 17)]);
            let mut snapshots = ModifierSnapshots::new(8, 128);
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
            assert!(snapshots.lookup(&source, 32, 3).unwrap().is_ok());
            source.calls.borrow_mut().clear();
            source.fault.set(Some(fault));
            assert!(!snapshots.verify_and_clear(&source));
            assert_eq!(
                &*source.calls.borrow(),
                &[(16, 16), (64, 32), (16, 16), (32, 16), (128, 16), (32, 16)]
            );
            assert_eq!(snapshots.used_len, 0);
            source.fault.set(None);
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
            // A later failed lookup must not discard prior retained inputs:
            // capture finalizes them even when the walk is already unavailable.
            source.fault.set(Some(32));
            assert!(snapshots.lookup(&source, 32, 3).unwrap().is_err());
            source.bytes.borrow_mut()[72] ^= 1;
            assert!(!snapshots.verify_and_clear(&source));
            assert_eq!(snapshots.used_len, 0);
        }
    }

    #[test]
    fn modifier_snapshot_rejects_unsorted_suffix_and_failed_initial_reads() {
        let source = Observed::new();
        let mut snapshots = ModifierSnapshots::new(8, 128);
        for suffix in [0, 1, 5] {
            source.accumulator(16, 64, &[(2, 7), (5, 11), (suffix, 13)]);
            assert_eq!(
                snapshots.lookup(&source, 16, 2),
                Some(Err("modifier accumulator keys not positive and increasing"))
            );
            assert_eq!(snapshots.used_len, 0);
            assert_eq!(snapshots.bytes_len, 0);
        }
        source.accumulator(16, 64, &[(2, 7)]);
        for fault in [16, 64] {
            source.fault.set(Some(fault));
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_err());
            assert_eq!(snapshots.used_len, 0);
        }
        source.fault.set(None);
        assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
        assert!(snapshots.verify_and_clear(&source));
    }

    #[test]
    fn modifier_snapshot_collisions_and_capacity_fallback_keep_full_original_validation() {
        use engine::modifier_accumulator::lookup_at;
        for (slots, arena) in [(0, 0), (1, 128), (8, 32)] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 11)]);
            source.accumulator(32, 128, &[(3, 17), (7, 19)]);
            let cache = RefCell::new(SmallReadCache::new(0));
            let modifiers = RefCell::new(ModifierSnapshots::new(slots, arena));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: Some(&modifiers),
            };
            assert_eq!(lookup_at(&memory, 16, 2).unwrap().raw, 7);
            assert_eq!(lookup_at(&memory, 32, 3).unwrap().raw, 17);
            source.bytes.borrow_mut()[136..144].copy_from_slice(&23_i64.to_le_bytes());
            assert_eq!(lookup_at(&memory, 32, 3).unwrap().raw, 23);
            source.bytes.borrow_mut()[144..148].copy_from_slice(&3_u32.to_le_bytes());
            assert!(
                lookup_at(&memory, 32, 3).is_err(),
                "fallback skipped malformed suffix"
            );
            assert_eq!(modifiers.borrow().used_len, usize::from(slots != 0));
            assert!(modifiers.borrow_mut().verify_and_clear(&source));
        }
    }

    #[test]
    fn modifier_snapshot_empty_arrays_reset_growth_and_accounting_are_bounded() {
        let source = Observed::new();
        source.accumulator(16, 64, &[]);
        let mut snapshots = ModifierSnapshots::new(0, 0);
        assert!(snapshots.lookup(&source, 16, 1).is_none());
        assert_eq!(snapshots.allocated_bytes(), 0);
        snapshots.grow(8, 128);
        let bytes = snapshots.allocated_bytes();
        let pointers = (
            snapshots.entries.as_ptr(),
            snapshots.used.as_ptr(),
            snapshots.bytes.as_ptr(),
        );
        assert!(bytes >= 8 * (size_of::<ModifierSnapshot>() + size_of::<usize>()) + 128);
        for _ in 0..3 {
            assert_eq!(
                snapshots.lookup(&source, 16, 1),
                Some(Ok(engine::modifier_accumulator::Value::default()))
            );
            assert!(snapshots.verify_and_clear(&source));
            assert_eq!(
                (
                    snapshots.entries.as_ptr(),
                    snapshots.used.as_ptr(),
                    snapshots.bytes.as_ptr()
                ),
                pointers
            );
            assert_eq!(snapshots.allocated_bytes(), bytes);
        }
        assert!(
            source
                .calls
                .borrow()
                .iter()
                .all(|&(address, len)| (address, len) == (16, 16))
        );
        assert!(snapshots.lookup(&source, 16, 1).unwrap().is_ok());
        snapshots.clear();
        assert_eq!(snapshots.used_len, 0);
        assert_eq!(snapshots.bytes_len, 0);
        assert!(
            snapshots
                .entries
                .iter()
                .all(|entry| !entry.occupied && entry.address == 0)
        );
    }

    #[test]
    fn modifier_snapshot_initial_header_reread_rejects_changes_including_empty_arrays() {
        struct Changed {
            source: Observed,
            reads: Cell<usize>,
            changed_byte: usize,
        }
        impl Memory for Changed {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if !self.source.copy(address, destination) {
                    return false;
                }
                if address == 16 {
                    self.reads.set(self.reads.get() + 1);
                    if self.reads.get() > 1 {
                        destination[self.changed_byte] ^= 1;
                    }
                }
                true
            }
        }
        for count in [0, 1] {
            for changed_byte in [0, 8, 12] {
                let source = Changed {
                    source: Observed::new(),
                    reads: Cell::new(0),
                    changed_byte,
                };
                source.source.accumulator(16, 64, &[(2, 7)][..count]);
                let mut snapshots = ModifierSnapshots::new(8, 128);
                assert_eq!(
                    snapshots.lookup(&source, 16, 2),
                    Some(Err("modifier accumulator header changed"))
                );
                assert_eq!(snapshots.used_len, 0);
                assert_eq!(snapshots.bytes_len, 0);
            }
        }
    }

    #[test]
    fn modifier_snapshot_direct_verification_reaches_the_final_partial_batch() {
        struct Wide {
            bytes: RefCell<Vec<u8>>,
            calls: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Wide {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, destination.len()));
                engine::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes.borrow(),
                }
                .copy(address, destination)
            }
        }
        let source = Wide {
            bytes: RefCell::new(vec![0; 64 + 65 * 16]),
            calls: RefCell::new(Vec::new()),
        };
        {
            let mut bytes = source.bytes.borrow_mut();
            bytes[16..24].copy_from_slice(&64_u64.to_le_bytes());
            bytes[24..28].copy_from_slice(&65_u32.to_le_bytes());
            bytes[28..32].copy_from_slice(&65_u32.to_le_bytes());
            for index in 0..65 {
                let at = 64 + index * 16;
                bytes[at..at + 4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            }
        }
        let mut snapshots = ModifierSnapshots::new(8, 65 * 16);
        assert!(snapshots.lookup(&source, 16, 1).unwrap().is_ok());
        source.bytes.borrow_mut()[64 + 64 * 16 + 8] = 1;
        source.calls.borrow_mut().clear();
        assert!(!snapshots.verify_and_clear(&source));
        assert_eq!(
            &*source.calls.borrow(),
            &[(16, 16), (64, 1024), (1088, 16), (16, 16)]
        );
        assert_eq!(snapshots.used_len, 0);
    }

    #[test]
    fn transport_memo_hits_are_exact_and_end_at_each_independent_pass() {
        let mut memo = TransportMemo::new(8);
        let calls = Cell::new(0);
        for _ in 0..4 {
            assert_eq!(
                memo.resolve(u32::MAX, 0x1000, || {
                    calls.set(calls.get() + 1);
                    Ok(i64::MIN)
                }),
                Ok(i64::MIN)
            );
        }
        assert_eq!(calls.get(), 1);
        assert!(memo.verify_and_clear(|owner, region| {
            assert_eq!((owner, region), (u32::MAX, 0x1000));
            calls.set(calls.get() + 1);
            Ok(i64::MIN)
        }));
        assert_eq!(calls.get(), 2);
        assert_eq!(memo.used_len, 0);
        assert!(
            memo.entries
                .iter()
                .all(|e| !e.occupied && e.region == 0 && e.owner == 0 && e.speed == 0)
        );
        assert_eq!(memo.resolve(u32::MAX, 0x1000, || Ok(17)), Ok(17));
        assert!(memo.verify_and_clear(|_, _| Ok(17)));
    }

    #[test]
    fn transport_collisions_and_failed_misses_never_replace_a_retained_key() {
        let mut memo = TransportMemo::new(1);
        assert_eq!(
            memo.resolve(1, 8, || Err("initial fault")),
            Err("initial fault")
        );
        assert_eq!(memo.used_len, 0);
        assert_eq!(memo.resolve(1, 8, || Ok(11)), Ok(11));
        assert_eq!(memo.resolve(2, 8, || Ok(22)), Ok(22));
        assert_eq!(
            memo.resolve(1, 16, || Err("collision fault")),
            Err("collision fault")
        );
        assert_eq!(memo.resolve(2, 8, || Ok(23)), Ok(23));
        assert_eq!(
            memo.resolve(1, 8, || panic!("retained key recalculated early")),
            Ok(11)
        );
        assert_eq!(memo.used_len, 1);
        assert!(memo.verify_and_clear(|owner, region| {
            assert_eq!((owner, region), (1, 8));
            Ok(11)
        }));
    }

    #[test]
    fn transport_verification_checks_all_results_after_change_or_fault_and_clears_keys() {
        for fault in [false, true] {
            let mut memo = TransportMemo::new(8);
            assert_eq!(memo.resolve(1, 8, || Ok(11)), Ok(11));
            assert_eq!(memo.resolve(1, 16, || Ok(22)), Ok(22));
            let mut calls = 0;
            assert!(!memo.verify_and_clear(|_, region| {
                calls += 1;
                if region == 8 {
                    if fault { Err("verify fault") } else { Ok(12) }
                } else {
                    Ok(22)
                }
            }));
            assert_eq!(calls, 2);
            assert_eq!(memo.used_len, 0);
            assert!(memo.entries.iter().all(|e| !e.occupied && e.region == 0));
        }
    }

    #[test]
    fn transport_direct_verification_covers_operands_not_retained_by_small_read_cache() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(1));
        let cached = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
        };
        // The first operand occupies the sole read-cache slot. The derived
        // speed's second operand collides, so small-read verification alone
        // cannot discover its later change.
        let mut memo = TransportMemo::new(1);
        assert_eq!(
            memo.resolve(7, 8, || {
                let a = engine::read::<u8>(&cached, 32).ok_or("a")?;
                let b = engine::read::<u8>(&cached, 64).ok_or("b")?;
                Ok(i64::from(a) + i64::from(b))
            }),
            Ok(96)
        );
        source.bytes.borrow_mut()[64] = 65;
        assert_eq!(memo.resolve(7, 8, || panic!("expected memo hit")), Ok(96));
        assert!(!memo.verify_and_clear(|_, _| {
            let a = engine::read::<u8>(&source, 32).ok_or("a")?;
            let b = engine::read::<u8>(&source, 64).ok_or("b")?;
            Ok(i64::from(a) + i64::from(b))
        }));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn transport_capacity_reset_and_accounting_do_not_allocate_during_use() {
        let mut memo = TransportMemo::new(0);
        assert_eq!(memo.resolve(0, 0, || Ok(9)), Ok(9));
        assert_eq!(memo.allocated_bytes(), 0);
        memo.grow(2);
        let bytes = memo.allocated_bytes();
        let entry_pointer = memo.entries.as_ptr();
        let used_pointer = memo.used.as_ptr();
        assert!(bytes >= 2 * (size_of::<TransportEntry>() + size_of::<usize>()));
        for _ in 0..3 {
            assert_eq!(memo.resolve(0, 0, || Ok(9)), Ok(9));
            assert!(memo.verify_and_clear(|_, _| Ok(9)));
            assert_eq!(memo.entries.as_ptr(), entry_pointer);
            assert_eq!(memo.used.as_ptr(), used_pointer);
        }
        assert_eq!(memo.resolve(1, 8, || Ok(10)), Ok(10));
        memo.clear();
        assert_eq!(memo.used_len, 0);
        assert!(memo.entries.iter().all(|e| !e.occupied && e.region == 0));
        assert_eq!(memo.allocated_bytes(), bytes);
    }

    #[test]
    fn exact_small_reads_share_only_within_one_verified_pass() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
        };
        let mut bytes = [0; 8];
        for _ in 0..4 {
            assert!(memory.copy(64, &mut bytes));
        }
        assert_eq!(bytes, [64, 65, 66, 67, 68, 69, 70, 71]);
        assert_eq!(source.calls.borrow().as_slice(), &[(64, 8)]);
        assert!(memory.immutable_view(64, 8).is_none());
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(source.calls.borrow().as_slice(), &[(64, 8), (64, 8)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(
            cache
                .borrow()
                .entries
                .iter()
                .all(|e| e.address == 0 && e.len == 0 && e.bytes == [0; 32])
        );
        source.bytes.borrow_mut()[64] = 17;
        assert!(memory.copy(64, &mut bytes));
        assert_eq!(bytes[0], 17);
        assert_eq!(source.calls.borrow().len(), 3);
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn collisions_different_lengths_large_and_empty_reads_never_evict_or_alias() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(1));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
        };
        assert!(memory.copy(64, &mut [0; 8]));
        assert!(memory.copy(72, &mut [0; 8]));
        assert!(memory.copy(72, &mut [0; 8]));
        assert!(memory.copy(64, &mut [0; 4]));
        assert!(memory.copy(64, &mut [0; 4]));
        assert!(memory.copy(64, &mut [0; 8]));
        assert!(memory.copy(96, &mut [0; 33]));
        assert!(memory.copy(96, &mut [0; 33]));
        assert!(memory.copy(96, &mut []));
        assert_eq!(source.calls.borrow().len(), 8);
        assert_eq!(cache.borrow().used_len, 1);
        assert_eq!(cache.borrow().entries[0].address, 64);
        assert_eq!(cache.borrow().entries[0].len, 8);
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(source.calls.borrow().last(), Some(&(64, 8)));
    }

    #[test]
    fn failed_initial_reads_are_not_cached_and_verification_faults_fail_the_pass() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
        };
        source.fault.set(Some(32));
        assert!(!memory.copy(32, &mut [0; 8]));
        assert!(!memory.copy(32, &mut [0; 8]));
        assert_eq!(cache.borrow().used_len, 0);
        source.fault.set(None);
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(memory.copy(40, &mut [0; 8]));
        source.fault.set(Some(32));
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        // Verification must still inspect the second retained span after the
        // first faults, and must clear both before another independent pass.
        assert_eq!(&source.calls.borrow()[4..], &[(32, 8), (40, 8)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(cache.borrow().entries.iter().all(|e| e.len == 0));
    }

    #[test]
    fn final_direct_reread_exposes_a_mutation_hidden_by_an_intermediate_cache_hit() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
        };
        let mut bytes = [0; 8];
        assert!(memory.copy(32, &mut bytes));
        source.bytes.borrow_mut()[32] ^= 1;
        assert!(memory.copy(32, &mut bytes));
        assert_eq!(bytes[0], 32);
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert!(memory.copy(32, &mut bytes));
        assert_eq!(bytes[0], 33);
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn cache_reset_growth_and_accounting_are_off_callback_and_bounded() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(0));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
        };
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(memory.copy(32, &mut [0; 8]));
        assert_eq!(cache.borrow().allocated_bytes(), 0);
        cache.borrow_mut().grow(2);
        let allocated = cache.borrow().allocated_bytes();
        assert!(allocated >= 2 * (size_of::<CacheEntry>() + size_of::<usize>()));
        assert!(memory.copy(32, &mut [0; 8]));
        cache.borrow_mut().clear();
        assert_eq!(cache.borrow().used_len, 0);
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(cache.borrow().allocated_bytes(), allocated);
    }

    #[test]
    fn conversion_uses_owned_edges_and_keeps_separate_leg_rounding() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 1,
                legs: 2,
                cache_entries: 2,
                transport_entries: 2,
                modifier_accumulators: 0,
                modifier_bytes: 0,
            },
            &[0],
        );
        capture.rows.rows[0] = cunit::EtaInputs {
            division: 7,
            next: 10,
            destination: 11,
            path_total: 2,
            leg_start: 0,
            leg_len: 2,
            next_leg_count: 1,
            numerator: 1,
            current_speed: 3,
        };
        capture.legs.first[..2].copy_from_slice(&[
            cunit::EtaLeg {
                distance: 2,
                speed: 3,
            },
            cunit::EtaLeg {
                distance: 2,
                speed: 3,
            },
        ]);
        capture.legs.len = 2;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::DivisionEta(rows))] = capture.tables() else {
            panic!("expected exact ETA rows");
        };
        assert_eq!(rows[0].next_remaining, 33_333);
        assert_eq!(rows[0].destination_remaining, 99_999);
        assert_eq!(rows[0].path_total, 2);
        capture.legs.len = 1;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }

    #[test]
    fn failures_and_truncated_empty_prefix_are_not_standstill() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 1,
                legs: 1,
                cache_entries: 2,
                transport_entries: 2,
                modifier_accumulators: 0,
                modifier_bytes: 0,
            },
            &[0],
        );
        capture.rows.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 8,
        };
        assert!(matches!(
            capture.tables(),
            [TableView::Truncated { missing: 8, .. }]
        ));
        capture.rows.checked = true;
        capture.rows.mismatches = 1;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }

    #[test]
    fn independent_eta_window_retires_only_comparison_storage() {
        let mut caps = Capacities {
            rows: 1,
            legs: 1,
            cache_entries: 2,
            transport_entries: 2,
            modifier_accumulators: 0,
            modifier_bytes: 0,
        };
        let mut capture = Captured::with_capacities(&caps, &[0]);
        capture.legs.required = 3;
        capture.grow(&mut caps, &[CONSISTENCY_TARGET]);
        assert!(capture.legs.first.len() >= 3);
        assert!(capture.rows.second.is_empty());
        assert!(capture.legs.second.is_empty());
        assert!(!capture.rows.rows.is_empty());
    }
}
