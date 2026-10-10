//! Passive transport cache, pool and reservation inputs borrowed during the
//! original authenticated country-row observation. No game calls, cache writes
//! or retries occur. Variant selection uses the first nonzero quantity, including
//! negative values, without flags; the complete pool reduction checks every flag
//! even for zero and negative quantities. Cache, first-variant, pool and
//! reservation failures remain independent. Selectors and memos retire before
//! the country loan returns.
//!
//! Admission requires the recorded main OS thread, authenticated matching idler
//! owners and an absent or completed original task. This excludes independent
//! UI cache writers before borrowing the graph. Pointer identity or worker
//! completion alone cannot establish admission. Runtime acceptance and measured
//! cost remain separate checks.
//!
//! Construction initializes the pool, empty reservations and dirty signed cache.
//! Loading restores the pool and invalidates the cache; serialization omits cache
//! and reservation buckets. Pool additions, transfers, removals and diversions
//! to raid stock invalidate it; getters can fill it lazily. Reservation writers
//! grow the pointer vector, update allocated amounts, or swap-erase and destroy
//! the final subscriber's bucket. Destruction releases buckets and pool arrays.
//!
//! Variant stats can change without a new handle: assignment replaces stat blocks,
//! copying retains them, post-load work rebuilds them, and category changes alter
//! the classifier. Destruction retires registration. Memos therefore bind only
//! the original loan, even when pointers or full handles remain equal. Registry
//! authentication and worker completion do not establish foreign-writer exclusion,
//! load exclusion or that every game task has joined.
use crate::{
    Memory, ccountry, cequipment_variant as variant, cequipment_variant_pool as pool,
    cpdx_array::{self, Array},
    creference_registry,
    memory::read_slice,
    read,
};
type Fact<T> = Result<T, &'static str>;

// Transport detection reads detection slots and speed; interception reads
// visibility alone. The stat cache stores signed i64 values at +0x100
// with eight-byte entries.
const SURFACE_DETECTION: usize = 0x198;
const SUBMARINE_DETECTION: usize = 0x1a0;
const VISIBILITY: usize = 0x1a8;
const SPEED: usize = 0x208;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cache {
    pub valid: u8,
    pub raw: Option<i64>,
}
/// None means the old row never attempted the operand; Err is never retried.
pub type OriginalCache = Option<Fact<Cache>>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PoolEntry {
    pub variant: Option<u64>,
    pub flags: Option<u64>,
    pub quantity: i64,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Reservation {
    pub priority: i32,
    pub quantity: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub variant: u64,
    pub surface_detection: Fact<i64>,
    pub submarine_detection: Fact<i64>,
    pub visibility: Fact<i64>,
    pub speed: Fact<i64>,
}
#[derive(Debug, Clone, Copy)]
pub struct Observation {
    pub cache: Fact<Cache>,
    pub unreserved: Fact<i32>,
    pub first: Fact<Option<Variant>>,
    pub priority: Fact<i32>,
    pub pool: Fact<usize>,
    pub reservations: Fact<usize>,
    pub pool_required: usize,
    pub reservations_required: usize,
    pub classifier_failure: Option<&'static str>,
    pub variant_identity_failure: Option<&'static str>,
}
impl Default for Observation {
    fn default() -> Self {
        Self {
            cache: Err("transport original convoy cache unavailable"),
            priority: Err("transport native transfer priority unavailable"),
            unreserved: Err("transport unreserved convoys unavailable"),
            first: Err("transport first convoy variant unavailable"),
            pool: Err("transport original convoy pool unavailable"),
            reservations: Err("transport original convoy reservations unavailable"),
            pool_required: 0,
            reservations_required: 0,
            classifier_failure: None,
            variant_identity_failure: None,
        }
    }
}

struct Memo {
    pointer: usize,
    identity: Fact<u64>,
    flags: Option<Fact<u64>>,
    stats: Option<Fact<Variant>>,
}
pub struct Scratch {
    entries: Vec<Memo>,
    slots: Vec<usize>,
    capacity: usize,
    required: usize,
    registry: Option<creference_registry::ScopedResolver>,
    priority: Option<Fact<i32>>,
}
impl Scratch {
    pub fn try_new(capacity: usize) -> std::io::Result<Self> {
        let count = crate::transport_ledger_observation::index_capacity(capacity)
            .ok_or_else(crate::allocation::oom)?;
        let mut slots = crate::allocation::vec(count)?;
        slots.resize(count, usize::MAX);
        Ok(Self {
            entries: crate::allocation::vec(capacity)?,
            slots,
            capacity,
            required: 0,
            registry: None,
            priority: None,
        })
    }
    pub fn vacant() -> Self {
        Self {
            entries: Vec::new(),
            slots: Vec::new(),
            capacity: 0,
            required: 0,
            registry: None,
            priority: None,
        }
    }
    pub fn begin(&mut self) {
        self.retire();
        self.required = 0;
    }
    pub fn retire(&mut self) {
        for entry in &mut self.entries {
            entry.pointer = 0;
        }
        self.entries.clear();
        self.slots.fill(usize::MAX);
        self.registry = None;
        self.priority = None;
    }
    pub fn required(&self) -> usize {
        self.required
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Memo>()
            + self.slots.capacity() * std::mem::size_of::<usize>()
    }
    fn slot(pointer: usize, count: usize) -> usize {
        let mut hash = pointer as u64;
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((hash ^ (hash >> 31)) as usize) & (count - 1)
    }
    fn grow(&mut self, required: usize) -> Fact<()> {
        if required <= self.capacity {
            return Ok(());
        }
        let count = crate::transport_ledger_observation::index_capacity(required)
            .ok_or("transport convoy variant index extent overflow")?;
        let capacity = count / 2;
        let mut slots = crate::allocation::filled(count, usize::MAX)
            .map_err(|_| "transport convoy variant index allocation failed")?;
        let additional = capacity - self.entries.len();
        crate::allocation::reserve(&mut self.entries, additional)
            .map_err(|_| "transport convoy variant memo allocation failed")?;
        for (index, entry) in self.entries.iter().enumerate() {
            let mut slot = Self::slot(entry.pointer, count);
            while slots[slot] != usize::MAX {
                slot = (slot + 1) & (count - 1);
            }
            slots[slot] = index;
        }
        self.slots = slots;
        self.capacity = capacity;
        Ok(())
    }
    fn entry(&mut self, memory: &impl Memory, base: usize, pointer: usize) -> Fact<&mut Memo> {
        if self.slots.is_empty() {
            return Err("transport convoy variant memo exhausted");
        }
        // A half-full checked index avoids a quadratic variant census. Its
        // hash has no bearing on native occurrence order or identity.
        let mut slot = Self::slot(pointer, self.slots.len());
        while self.slots[slot] != usize::MAX {
            let index = self.slots[slot];
            if self.entries[index].pointer == pointer {
                return Ok(&mut self.entries[index]);
            }
            slot = (slot + 1) & (self.slots.len() - 1);
        }
        self.required = self.required.saturating_add(1);
        if self.entries.len() == self.capacity {
            return Err("transport convoy variant memo exhausted");
        }
        let identity = (|| {
            if pointer == 0 {
                return Err("transport convoy variant is null");
            }
            let handle = variant::identity(memory, base, pointer)?;
            if handle >> 32 == 0 {
                return Err("transport convoy variant id is zero");
            }
            let registry = self
                .registry
                .get_or_insert_with(|| creference_registry::ScopedResolver::new(base));
            if registry.resolve(memory, handle)? != Some(pointer) {
                return Err("transport convoy variant registry identity mismatch");
            }
            Ok(handle)
        })();
        self.slots[slot] = self.entries.len();
        self.entries.push(Memo {
            pointer,
            identity,
            flags: None,
            stats: None,
        });
        Ok(self.entries.last_mut().unwrap())
    }
    fn flags(&mut self, memory: &impl Memory, base: usize, pointer: usize) -> Fact<(u64, u64)> {
        let entry = self.entry(memory, base, pointer)?;
        let handle = entry.identity?;
        let flags = *entry.flags.get_or_insert_with(|| {
            at(pointer, variant::CLASSIFIER_FLAGS).and_then(|address| {
                read(memory, address).ok_or("transport convoy variant flags unreadable")
            })
        });
        Ok((handle, flags?))
    }
    fn stats(&mut self, memory: &impl Memory, base: usize, pointer: usize) -> Fact<Variant> {
        let entry = self.entry(memory, base, pointer)?;
        let handle = entry.identity?;
        *entry.stats.get_or_insert_with(|| {
            // Read these signed cached stats without reconstructing them from authored
            // equipment. Interception reaches visibility alone. Detection reaches
            // the detection caches and speed: a fault in any of those leaves
            // cannot erase the authenticated first variant or its visibility.
            let stat = |offset, reason| {
                at(pointer, offset).and_then(|address| read(memory, address).ok_or(reason))
            };
            let visibility = stat(VISIBILITY, "transport convoy visibility cache unreadable");
            let surface_detection = stat(
                SURFACE_DETECTION,
                "transport convoy surface detection cache unreadable",
            );
            let submarine_detection = stat(
                SUBMARINE_DETECTION,
                "transport convoy submarine detection cache unreadable",
            );
            let speed = stat(SPEED, "transport convoy speed cache unreadable");
            Ok(Variant {
                variant: handle,
                surface_detection,
                submarine_detection,
                visibility,
                speed,
            })
        })
    }
}
fn at(pointer: usize, offset: usize) -> Fact<usize> {
    pointer
        .checked_add(offset)
        .ok_or("transport convoy address overflow")
}

/// Off-thread complete header demand from the authenticated country.
pub fn demand(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<(usize, usize), &'static str> {
    let root = at(country.pointer, ccountry::CONVOY_STATE)?;
    if read::<u64>(memory, root) != Some(at(base, super::VTABLE_RVA)? as u64) {
        return Err("transport embedded CConvoys identity mismatch");
    }
    // Consume the complete pool and signed reservation range.
    let pool = pool::entries(memory, base, at(root, super::EQUIPMENT_POOL)?)?;
    let reservations = Array::read(memory, at(root, super::RESERVATIONS)?, 8)
        .ok_or("transport convoy reservation sizing header unreadable or invalid")?;
    Ok((pool.count as usize, reservations.count as usize))
}

/// Original country authentication owns this receiver; controls are local and
/// cannot escape. Country acquisition retires `scratch` before joining its loan.
pub fn capture(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    original: OriginalCache,
    pool_output: &mut [PoolEntry],
    reservation_output: &mut [Reservation],
    scratch: &mut Scratch,
) -> Observation {
    capture_buffer(
        memory,
        base,
        country,
        original,
        (crate::walk::Buffer::Fixed(pool_output), 0),
        (crate::walk::Buffer::Fixed(reservation_output), 0),
        scratch,
    )
}

/// Countries' worker continues from the original headers and variant attempts.
pub fn capture_growing(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    original: OriginalCache,
    pool: (&mut Vec<PoolEntry>, usize),
    reservations: (&mut Vec<Reservation>, usize),
    scratch: &mut Scratch,
) -> Observation {
    capture_buffer(
        memory,
        base,
        country,
        original,
        (crate::walk::Buffer::Growing(pool.0), pool.1),
        (crate::walk::Buffer::Growing(reservations.0), reservations.1),
        scratch,
    )
}

fn capture_buffer(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    original: OriginalCache,
    (mut pool_output, pool_start): (crate::walk::Buffer<'_, PoolEntry>, usize),
    (mut reservation_output, reservation_start): (crate::walk::Buffer<'_, Reservation>, usize),
    scratch: &mut Scratch,
) -> Observation {
    // The transport query passes this runtime signed i32 value to its calculation. Read it once for
    // the original country pass, including a failed observation; no default.
    let mut result = Observation {
        priority: *scratch.priority.get_or_insert_with(|| {
            at(base, 0x0331_b7dc).and_then(|address| {
                read(memory, address).ok_or("transport native transfer priority unreadable")
            })
        }),
        ..Default::default()
    };
    let root = match at(country.pointer, ccountry::CONVOY_STATE) {
        Ok(root) => root,
        Err(reason) => {
            result.cache = Err(reason);
            return result;
        }
    };
    // The alternate getter is independent of CConvoys authentication/cache.
    result.unreserved = at(country.pointer, 0x1288).and_then(|address| {
        read(memory, address).ok_or("transport unreserved convoy count unreadable")
    });
    let identity = (|| {
        let expected = base
            .checked_add(super::VTABLE_RVA)
            .ok_or("transport embedded CConvoys module overflow")? as u64;
        if read::<u64>(memory, root) != Some(expected) {
            return Err("transport embedded CConvoys identity mismatch");
        }
        // Dynamic country allocation can restamp the country without changing its
        // embedded convoy owner tag. Getters consume the pool without requiring
        // that equality; the original country loan owns the current identity.
        Ok(())
    })();
    if let Err(reason) = identity {
        result.cache = Err(reason);
        result.pool = Err(reason);
        result.first = Err(reason);
        result.reservations = Err(reason);
        return result;
    }
    result.cache = original.unwrap_or_else(|| {
        let valid = read::<u8>(memory, at(root, super::CACHE_VALID)?)
            .ok_or("transport original convoy cache validity unreadable")?;
        let raw = if valid == 0 {
            None
        } else {
            Some(
                read(memory, at(root, super::CACHE_RAW)?)
                    .ok_or("transport original convoy held cache unreadable")?,
            )
        };
        Ok(Cache { valid, raw })
    });
    let source =
        at(root, super::EQUIPMENT_POOL).and_then(|pointer| pool::entries(memory, base, pointer));
    result.pool = source.and_then(|source| {
        result.pool_required = source.count as usize;
        let end = pool_start
            .checked_add(result.pool_required)
            .ok_or("transport convoy pool arena extent overflow")?;
        if !pool_output.ensure(end)? {
            return Err("transport convoy pool arena exhausted");
        }
        if matches!(pool_output, crate::walk::Buffer::Growing(_)) {
            // The pool walk visits all original variant entries, including zero
            // quantities. Retain the same-pass failed memo while growing it.
            scratch.grow(
                scratch
                    .entries
                    .len()
                    .checked_add(result.pool_required)
                    .ok_or("transport convoy variant memo extent overflow")?,
            )?;
        }
        let dirty = result.cache.is_ok_and(|cache| cache.valid == 0);
        let mut first = None;
        let mut bytes = [0_u8; 64 * 16];
        for start in (0..source.count).step_by(64) {
            let count = ((source.count - start) as usize).min(64);
            let address = source
                .element(start, 16)
                .ok_or("transport convoy pool address overflow")?;
            if !read_slice(memory, address, &mut bytes[..count * 16]) {
                return Err("transport convoy pool payload unreadable");
            }
            for (index, bytes) in bytes[..count * 16].chunks_exact(16).enumerate() {
                let pointer = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
                    .map_err(|_| "transport convoy variant pointer overflow")?;
                let quantity = i64::from_le_bytes(bytes[8..].try_into().unwrap());
                let mut entry = PoolEntry {
                    quantity,
                    ..Default::default()
                };
                if dirty {
                    match scratch.flags(memory, base, pointer) {
                        Ok((handle, flags)) => {
                            entry.variant = Some(handle);
                            entry.flags = Some(flags);
                        }
                        Err(reason) => {
                            result.classifier_failure.get_or_insert(reason);
                        }
                    }
                }
                if quantity != 0 {
                    // Keep the unfiltered pool census for saved identities.
                    match scratch
                        .entry(memory, base, pointer)
                        .and_then(|memo| memo.identity)
                    {
                        Ok(handle) => entry.variant = Some(handle),
                        Err(reason) => {
                            result.variant_identity_failure.get_or_insert(reason);
                        }
                    }
                    if first.is_none() {
                        first = Some(scratch.stats(memory, base, pointer));
                    }
                }
                pool_output[pool_start + start as usize + index] = entry;
            }
        }
        result.first = first.transpose();
        Ok(result.pool_required)
    });
    if let Err(reason) = result.pool {
        result.first = Err(reason);
    }
    result.reservations = (|| {
        let source = Array::read(memory, at(root, super::RESERVATIONS)?, 8)
            .ok_or("transport convoy reservation header unreadable or invalid")?;
        result.reservations_required = source.count as usize;
        // Consume every original signed reservation occurrence.
        let end = reservation_start
            .checked_add(result.reservations_required)
            .ok_or("transport convoy reservation arena extent overflow")?;
        if !reservation_output.ensure(end)? {
            return Err("transport convoy reservation arena exhausted");
        }
        cpdx_array::pointers(
            memory,
            source,
            "transport convoy reservation pointer unreadable",
            |index, pointer| {
                if pointer == 0 {
                    return Err("transport convoy reservation pointer is null");
                }
                let bytes = read::<[u8; 8]>(memory, pointer)
                    .ok_or("transport convoy reservation fields unreadable")?;
                reservation_output[reservation_start + index as usize] = Reservation {
                    priority: i32::from_le_bytes(bytes[..4].try_into().unwrap()),
                    quantity: i32::from_le_bytes(bytes[4..].try_into().unwrap()),
                };
                Ok(())
            },
        )?;
        Ok(result.reservations_required)
    })();
    result
}

#[cfg(test)]
mod tests;
