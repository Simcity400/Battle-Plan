//! Optional battle-plan transport inputs from one quiescent wing census.
//! Source failures stay local to the field; no result survives this pass.

use crate::{Memory, allocation, cair_wing_pool, csub_unit_definition as definition, read};
use std::{io, mem::size_of};

/// Complete raid reference stored on the primary wing.
pub const RAID_REFERENCE: usize = 0xa2c;
const TRANSPORT: u32 = 1 << 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaidReference {
    pub raw: u64,
    pub resolves: bool,
}

#[derive(Clone, Copy, Debug)]
struct Pool {
    index: Option<u32>,
    transport: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
struct Slot<T> {
    key: u64,
    value: Option<T>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            key: 0,
            value: None,
        }
    }
}

/// The caller creates/grows this storage off-thread and charges its actual
/// vector capacities. At most one distinct optional key per admitted map row
/// belongs to each arena; half-full tables retain failed observations too.
#[derive(Debug, Default)]
pub struct Scratch {
    pools: Vec<Slot<Pool>>,
    wings: Vec<Slot<Option<u64>>>,
    handles: Vec<Slot<Option<bool>>>,
    pub required_map_capacity: usize,
    pub failures: usize,
}

impl Scratch {
    /// Constructor fallback follows `allocation::owned_vec`: allocation
    /// failure stops capture unless the caller is preparing an advisory hint.
    pub fn owned(map_capacity: usize) -> Self {
        match Self::with_capacity(map_capacity) {
            Ok(scratch) => scratch,
            Err(error) => {
                let _ = allocation::owned_vec::<u8>(Err(error));
                Self::default()
            }
        }
    }

    pub fn with_capacity(map_capacity: usize) -> io::Result<Self> {
        let mut scratch = Self::default();
        scratch.grow(map_capacity)?;
        Ok(scratch)
    }

    /// Checked off-thread growth; never called by a native reader.
    pub fn grow(&mut self, map_capacity: usize) -> io::Result<()> {
        let slots = slot_count(map_capacity).ok_or_else(allocation::oom)?;
        fn grow<T>(values: &mut Vec<Slot<T>>, slots: usize) -> io::Result<()> {
            allocation::reserve_exact(values, slots.saturating_sub(values.len()))?;
            values.resize_with(slots.max(values.len()), Slot::default);
            Ok(())
        }
        grow(&mut self.pools, slots)?;
        grow(&mut self.wings, slots)?;
        grow(&mut self.handles, slots)?;
        self.clear();
        Ok(())
    }

    pub fn fits(&self, map_capacity: usize) -> bool {
        slot_count(map_capacity).is_some_and(|needed| {
            self.pools.len() >= needed && self.wings.len() >= needed && self.handles.len() >= needed
        })
    }

    pub fn clear(&mut self) {
        for value in &mut self.pools {
            *value = Slot::default();
        }
        for value in &mut self.wings {
            *value = Slot::default();
        }
        for value in &mut self.handles {
            *value = Slot::default();
        }
        self.reset_metadata();
    }

    /// Idle owners reset diagnostics only. `Pass::new` always clears the
    /// private observations before the next independent acquisition.
    pub fn reset_metadata(&mut self) {
        self.required_map_capacity = 0;
        self.failures = 0;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.pools.capacity() * size_of::<Slot<Pool>>()
            + self.wings.capacity() * size_of::<Slot<Option<u64>>>()
            + self.handles.capacity() * size_of::<Slot<Option<bool>>>()
    }
}

fn slot_count(map_capacity: usize) -> Option<usize> {
    if map_capacity == 0 {
        return Some(0);
    }
    map_capacity.checked_mul(2)?.checked_next_power_of_two()
}

/// An empty slot is reserved before the reader runs. Occupancy and probes
/// are bounded by allocated slots; no observed failure can be evicted.
fn memo<T: Copy>(
    values: &mut [Slot<T>],
    count: &mut usize,
    key: u64,
    read: impl FnOnce() -> T,
) -> Option<T> {
    let length = values.len();
    let mask = length.checked_sub(1)?;
    let first = key.wrapping_mul(0x9e37_79b9_7f4a_7c15) as usize & mask;
    for offset in 0..length {
        let slot = &mut values[first.wrapping_add(offset) & mask];
        if slot.value.is_some() && slot.key == key {
            return slot.value;
        }
        if slot.value.is_none() {
            if *count >= length / 2 {
                return None;
            }
            slot.key = key;
            let value = read();
            slot.value = Some(value);
            *count += 1;
            return Some(value);
        }
    }
    None
}

/// Fixed base and database/header observations belong to this invocation.
pub struct Pass<'a> {
    base: usize,
    capacity: usize,
    scratch: &'a mut Scratch,
    database: Option<Result<definition::Database, &'static str>>,
    resolver: crate::creference_registry::ScopedResolver,
    counts: [usize; 3],
    exhausted: bool,
}

impl<'a> Pass<'a> {
    pub fn new(base: usize, capacity: usize, scratch: &'a mut Scratch) -> Self {
        scratch.clear();
        if !scratch.fits(capacity) {
            scratch.required_map_capacity = capacity;
        }
        Self {
            base,
            capacity,
            scratch,
            database: None,
            resolver: crate::creference_registry::ScopedResolver::new(base),
            counts: [0; 3],
            exhausted: false,
        }
    }

    /// Worker-only growth preserves every already-observed optional key,
    /// including failed observations, rather than rereading earlier wings.
    pub fn ensure_capacity(&mut self, required: usize) -> io::Result<()> {
        if required <= self.capacity && self.scratch.fits(required) {
            return Ok(());
        }
        let capacity = required.max(self.capacity.saturating_mul(2));
        let slots = slot_count(capacity).ok_or_else(allocation::oom)?;
        fn grow<T: Copy>(values: &mut Vec<Slot<T>>, slots: usize) -> io::Result<()> {
            if values.len() >= slots {
                return Ok(());
            }
            let mut replacement: Vec<Slot<T>> = Vec::new();
            allocation::resize_default(&mut replacement, slots)?;
            let mask = slots - 1;
            for value in values.iter().filter(|value| value.value.is_some()) {
                let mut index = value.key.wrapping_mul(0x9e37_79b9_7f4a_7c15) as usize & mask;
                while replacement[index].value.is_some() {
                    index = (index + 1) & mask;
                }
                replacement[index] = *value;
            }
            *values = replacement;
            Ok(())
        }
        grow(&mut self.scratch.pools, slots)?;
        grow(&mut self.scratch.wings, slots)?;
        grow(&mut self.scratch.handles, slots)?;
        self.capacity = capacity;
        self.scratch.required_map_capacity = 0;
        Ok(())
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        wing: usize,
        pool: usize,
        row: &mut super::Row,
    ) {
        row.pool_subunit = None;
        row.raid_reference = None;
        if self.exhausted {
            self.scratch.failures += 1;
            return;
        }
        let pool = memo(
            &mut self.scratch.pools,
            &mut self.counts[0],
            pool as u64,
            || pool_identity(memory, self.base, pool, &mut self.database),
        );
        let Some(pool) = pool else {
            self.capacity_failure();
            return;
        };
        row.pool_subunit = pool.index;
        if pool.index.is_none() {
            self.scratch.failures += 1;
        }
        if row.mission_mask != 0
            || row.eligibility_veto != 0
            || row.eligibility_counter > row.eligibility_limit
        {
            return;
        }
        match pool.transport {
            Some(false) => return,
            None => {
                self.scratch.failures += 1;
                return;
            }
            Some(true) => (),
        }
        let raw = memo(
            &mut self.scratch.wings,
            &mut self.counts[1],
            wing as u64,
            || field::<u64>(memory, wing, RAID_REFERENCE).ok(),
        );
        let raw = match raw {
            None => {
                self.capacity_failure();
                return;
            }
            Some(None) => {
                self.scratch.failures += 1;
                return;
            }
            Some(Some(raw)) => raw,
        };
        let resolves = if raw == 0 {
            Some(Some(false))
        } else {
            memo(&mut self.scratch.handles, &mut self.counts[2], raw, || {
                (|| {
                    let Some(pointer) = self.resolver.resolve(memory, raw)? else {
                        return Ok(false);
                    };
                    let identity = field::<[u8; 16]>(memory, pointer, 0)?;
                    if u64::from_le_bytes(identity[..8].try_into().unwrap())
                        != self
                            .base
                            .checked_add(crate::craid_instance::VTABLE_RVA)
                            .ok_or("raid module overflow")? as u64
                        || u64::from_le_bytes(identity[8..].try_into().unwrap()) != raw
                        || raw as u32 != crate::craid_instance::REFERENCE_TYPE_VALUE
                    {
                        return Err("air wing raid identity mismatch");
                    }
                    Ok(true)
                })()
                .ok()
            })
        };
        match resolves {
            Some(Some(resolves)) => row.raid_reference = Some(RaidReference { raw, resolves }),
            Some(None) => self.scratch.failures += 1,
            None => self.capacity_failure(),
        }
    }

    fn capacity_failure(&mut self) {
        self.exhausted = true;
        self.scratch.required_map_capacity = self.capacity;
        self.scratch.failures += 1;
    }
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|address| read(memory, address))
        .ok_or("air transport operand unreadable or address overflow")
}

fn header(memory: &impl Memory, base: usize, pointer: usize) -> Result<(u32, bool), &'static str> {
    let bytes = field::<[u8; 17]>(memory, pointer, 0)?;
    let vtable = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let token = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    match bytes[16] {
        1 if vtable
            == base
                .checked_add(definition::VTABLE_RVA)
                .ok_or("subunit module overflow")? as u64 =>
        {
            Ok((token, true))
        }
        0 if vtable
            == base
                .checked_add(definition::NULL_VTABLE_RVA)
                .ok_or("subunit module overflow")? as u64
            && token == definition::NULL_TOKEN =>
        {
            Ok((token, false))
        }
        _ => Err("air pool definition identity mismatch"),
    }
}

fn pool_identity(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    database: &mut Option<Result<definition::Database, &'static str>>,
) -> Pool {
    let identity = (|| {
        let pointer = usize::try_from(field::<u64>(memory, pool, cair_wing_pool::DEFINITION)?)
            .map_err(|_| "air definition pointer overflow")?;
        let (token, real) = header(memory, base, pointer)?;
        if !real {
            return Ok((0, pointer));
        }
        let index = field::<i32>(memory, pointer, definition::DATABASE_INDEX)?;
        let data = *database.get_or_insert_with(|| definition::Database::read(memory, base));
        let global = data?.entry(memory, index)?;
        let (global_token, global_real) = header(memory, base, global)?;
        if !global_real || global_token != token {
            return Err("air pool definition differs from its content database entry");
        }
        Ok((index as u32, pointer))
    })();
    match identity {
        Ok((index, pointer)) => Pool {
            index: Some(index),
            transport: if index == 0 {
                Some(false)
            } else {
                field::<u32>(memory, pointer, definition::AIR_FLAGS)
                    .ok()
                    .map(|flags| flags & TRANSPORT != 0)
            },
        },
        Err(_) => Pool {
            index: None,
            transport: None,
        },
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "alloc-fault-injection"))]
mod allocation_tests;
