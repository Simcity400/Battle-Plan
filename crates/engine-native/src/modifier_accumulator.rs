//! Inline sorted modifier accumulator, including the country accumulator.

use crate::{Memory, ccountry, cpdx_array::Array, modifier_entry, read};

/// Modifier-accumulator data pointer.
pub const DATA: usize = 0x00;
/// Modifier-accumulator u32 capacity.
pub const CAPACITY: usize = 0x08;
/// Modifier-accumulator count word, interpreted with each reader's signedness.
pub const COUNT: usize = 0x0c;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Value {
    pub raw: i64,
    pub present: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveValue {
    /// Unmodified accumulator operand; remains present when its feature is disabled.
    pub raw: i64,
    pub present: bool,
    /// Effective value after feature eligibility and optional [0, 100000] clamp.
    pub effective: i64,
}

/// Requested native getter operand, retaining its lazy value-read boundary.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct RequestedOperand {
    pub raw: Option<i64>,
    pub present: bool,
    pub effective: i64,
}

/// Independent bounds for accumulator observations and complete native entries.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotCapacity {
    pub nodes: usize,
    pub entries: usize,
}

/// A raw lookup in the arena that produced it, valid until that arena is cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotObservation {
    node: usize,
    key: u32,
    generation: u64,
}

#[derive(Debug)]
struct SnapshotNode {
    address: usize,
    start: usize,
    len: usize,
    result: Result<(), &'static str>,
}

const SNAPSHOT_BATCH: usize = 64;
const SNAPSHOT_FIELDS: [crate::memory::RecordField; 1] = [crate::memory::RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: modifier_entry::STRIDE,
}];
const SNAPSHOT_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&SNAPSHOT_FIELDS, modifier_entry::STRIDE)
        .expect("modifier snapshot entry layout");

/// Owned raw accumulators for one acquisition pass, growing in the pass. Each
/// address, including a failed source, is acquired once; successful entry
/// prefixes and the first failure remain owned until cleared.
#[derive(Debug, Default)]
pub struct SnapshotArena {
    nodes: Vec<SnapshotNode>,
    entries: Vec<[u8; 16]>,
    index: Vec<usize>,
    capacity: SnapshotCapacity,
    required: SnapshotCapacity,
    generation: u64,
}

impl SnapshotArena {
    pub fn vacant() -> Self {
        Self::default()
    }

    pub fn try_new(capacity: SnapshotCapacity) -> std::io::Result<Self> {
        let slots = if capacity.nodes == 0 {
            0
        } else {
            capacity
                .nodes
                .checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or_else(crate::allocation::oom)?
        };
        Ok(Self {
            nodes: crate::allocation::vec(capacity.nodes)?,
            entries: crate::allocation::vec(capacity.entries)?,
            index: crate::allocation::filled(slots, 0)?,
            capacity,
            required: SnapshotCapacity::default(),
            generation: 0,
        })
    }

    #[cfg(test)]
    pub fn new(capacity: SnapshotCapacity) -> Self {
        Self::try_new(capacity).unwrap()
    }

    pub fn clear(&mut self) {
        if !self.nodes.is_empty() {
            self.index.fill(0);
        }
        self.nodes.clear();
        self.entries.clear();
        self.required = SnapshotCapacity::default();
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn capacity(&self) -> SnapshotCapacity {
        self.capacity
    }

    /// Distinct-address demand, and the entries actually read.
    pub fn required_capacity(&self) -> SnapshotCapacity {
        self.required
    }

    /// Heap payload; the containing packet accounts for this object's inline size.
    pub fn allocated_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<SnapshotNode>()
            + self.entries.capacity() * std::mem::size_of::<[u8; 16]>()
            + self.index.capacity() * std::mem::size_of::<usize>()
    }

    /// Resize only while the owning frame is off the acquisition threads.
    /// Existing observations are discarded, including when no growth is needed.
    pub fn try_grow(&mut self, capacity: SnapshotCapacity) -> std::io::Result<()> {
        let capacity = SnapshotCapacity {
            nodes: capacity.nodes.max(self.capacity.nodes),
            entries: capacity.entries.max(self.capacity.entries),
        };
        if capacity == self.capacity {
            self.clear();
        } else {
            let generation = self.generation.wrapping_add(1);
            *self = Self::try_new(capacity)?;
            self.generation = generation;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn grow(&mut self, capacity: SnapshotCapacity) {
        self.try_grow(capacity).unwrap();
    }

    fn index_slot(&self, address: usize) -> usize {
        let mask = self.index.len() - 1;
        let mut slot = (address >> 4).wrapping_mul(0x9e37_79b9) & mask;
        while self.index[slot] != 0 && self.nodes[self.index[slot] - 1].address != address {
            slot = (slot + 1) & mask;
        }
        slot
    }

    /// Acquire a complete raw array from an authenticated country pointer.
    /// All keys and the full unused suffix are validated before success.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        country: usize,
        key: u32,
    ) -> Result<SnapshotObservation, &'static str> {
        validate_keys(&[key], 1)?;
        let address = country
            .checked_add(ccountry::MODIFIER_ACCUMULATOR)
            .ok_or("modifier accumulator address overflow")?;
        self.capture_at(memory, address, key)
    }

    /// Acquire at the actual accumulator header address. The caller owns source
    /// identity and graph authentication, just as for `lookup_at`. Country and
    /// explicit-address calls share a node only when their addresses are equal.
    pub fn capture_at(
        &mut self,
        memory: &impl Memory,
        address: usize,
        key: u32,
    ) -> Result<SnapshotObservation, &'static str> {
        validate_keys(&[key], 1)?;
        let node = match self.existing(address) {
            Some(node) => node,
            None => {
                let node = self.insert(address)?;
                let result = self.capture_node(memory, node);
                self.nodes[node].result = result;
                node
            }
        };
        self.nodes[node].result?;
        Ok(SnapshotObservation {
            node,
            key,
            generation: self.generation,
        })
    }

    fn existing(&self, address: usize) -> Option<usize> {
        if self.index.is_empty() {
            return None;
        }
        self.index[self.index_slot(address)].checked_sub(1)
    }

    /// A new incomplete node for `address`, which has none.
    fn insert(&mut self, address: usize) -> Result<usize, &'static str> {
        let mut slot = (!self.index.is_empty()).then(|| self.index_slot(address));
        self.required.nodes = self.required.nodes.max(self.nodes.len().saturating_add(1));
        if self.nodes.len() == self.capacity.nodes {
            let count = crate::allocation::add(self.nodes.len(), 1)
                .map_err(|_| "modifier snapshot allocation failed")?;
            let capacity = self
                .capacity
                .nodes
                .checked_mul(2)
                .ok_or("modifier snapshot demand overflow")?
                .max(count);
            let slots = capacity
                .checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or("modifier snapshot demand overflow")?;
            let additional = capacity - self.nodes.len();
            crate::allocation::reserve_exact(&mut self.nodes, additional)
                .map_err(|_| "modifier snapshot allocation failed")?;
            self.index = crate::allocation::filled(slots, 0)
                .map_err(|_| "modifier snapshot allocation failed")?;
            self.capacity.nodes = capacity;
            for node in 0..self.nodes.len() {
                let at = self.index_slot(self.nodes[node].address);
                self.index[at] = node + 1;
            }
            slot = Some(self.index_slot(address));
        }
        let node = self.nodes.len();
        self.nodes.push(SnapshotNode {
            address,
            start: self.entries.len(),
            len: 0,
            result: Err("modifier snapshot incomplete"),
        });
        self.index[slot.expect("nonzero modifier snapshot node capacity")] = node + 1;
        Ok(node)
    }

    /// Serve `address` from `entries`, a complete observation of its array
    /// that an earlier capture made and a certificate keeps: later lookups
    /// read nothing for it. An address this capture already observed keeps
    /// its observation; a refused allocation leaves it to be read.
    pub fn hold(&mut self, address: usize, entries: &[[u8; 16]]) {
        if self.existing(address).is_some()
            || crate::allocation::reserve(&mut self.entries, entries.len()).is_err()
        {
            return;
        }
        let Ok(node) = self.insert(address) else {
            return;
        };
        self.nodes[node].start = self.entries.len();
        self.nodes[node].len = entries.len();
        self.nodes[node].result = Ok(());
        self.entries.extend_from_slice(entries);
        self.required.entries = self.required.entries.saturating_add(entries.len());
        self.capacity.entries = self.capacity.entries.max(self.entries.len());
    }

    /// Observe `address`'s complete array as a lookup there would (once per
    /// capture; a later lookup is served from it) and return its entries;
    /// `None` when the observation failed or its allocation was refused.
    pub fn observe(&mut self, memory: &impl Memory, address: usize) -> Option<&[[u8; 16]]> {
        if self.existing(address).is_none() {
            let node = self.insert(address).ok()?;
            let result = self.capture_node(memory, node);
            self.nodes[node].result = result;
        }
        self.observed(address)
    }

    /// The complete entries this capture observed at `address`, for a later
    /// capture to [`Self::hold`].
    pub fn observed(&self, address: usize) -> Option<&[[u8; 16]]> {
        let node = &self.nodes[self.existing(address)?];
        node.result
            .ok()
            .map(|()| &self.entries[node.start..node.start + node.len])
    }

    /// Read only an address already observed in this arena. No native access,
    /// allocation, demand update or generation change occurs. Some(Err) retains
    /// the stored failure; None means this address has no observation.
    pub fn existing_at(&self, address: usize, key: u32) -> Option<Result<Value, &'static str>> {
        if let Err(reason) = validate_keys(&[key], 1) {
            return Some(Err(reason));
        }
        if self.index.is_empty() {
            return None;
        }
        let node = self.index[self.index_slot(address)].checked_sub(1)?;
        Some(self.value(SnapshotObservation {
            node,
            key,
            generation: self.generation,
        }))
    }
    fn capture_node(&mut self, memory: &impl Memory, node: usize) -> Result<(), &'static str> {
        let address = self.nodes[node].address;
        let header = read::<[u8; 16]>(memory, address)
            .ok_or("modifier accumulator header unreadable or invalid")?;
        let array = Array::from_header(header, modifier_entry::STRIDE)
            .ok_or("modifier accumulator header unreadable or invalid")?;
        let mut previous_key = 0;
        let mut first = 0;
        while first < array.count as usize {
            let count = (array.count as usize - first).min(SNAPSHOT_BATCH);
            let base = array
                .element(first as u32, modifier_entry::STRIDE)
                .ok_or("modifier accumulator entry address overflow")?;
            let start = self.entries.len();
            let mut scratch = [[0; 16]; SNAPSHOT_BATCH];
            let completed = memory.copy_records(
                crate::memory::RecordSources::Strided {
                    base,
                    stride: modifier_entry::STRIDE,
                    count,
                },
                &SNAPSHOT_LAYOUT,
                scratch[..count].as_flattened_mut(),
            );
            crate::allocation::reserve(&mut self.entries, completed)
                .map_err(|_| "modifier snapshot allocation failed")?;
            self.entries.extend_from_slice(&scratch[..completed]);
            self.capacity.entries = self.capacity.entries.max(self.entries.len());
            self.required.entries = self.required.entries.saturating_add(completed);
            self.nodes[node].len += completed;
            // A batch can read ahead of a malformed key. Keep every successful
            // entry, but preserve the first source-order failure.
            for bytes in &self.entries[start..] {
                let key = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                if key <= previous_key {
                    return Err("modifier accumulator keys not positive and increasing");
                }
                previous_key = key;
            }
            if completed != count {
                return Err("modifier accumulator entry unreadable");
            }
            first += count;
        }
        Ok(())
    }

    /// Resolve the raw value using only the immutable captured entries.
    pub fn value(&self, observation: SnapshotObservation) -> Result<Value, &'static str> {
        if observation.generation != self.generation {
            return Err("modifier snapshot observation expired");
        }
        let node = self
            .nodes
            .get(observation.node)
            .ok_or("modifier snapshot observation invalid")?;
        node.result?;
        lookup_snapshot(
            self.entries[node.start..node.start + node.len].as_flattened(),
            observation.key,
        )
    }
}

/// Resolves several strictly increasing keys in one census of the native sorted
/// accumulator. Raw values are never clamped or rescaled; callers apply the
/// metadata branch only when they need the engine's derived effective value.
/// The country pointer must come from the authenticated country census.
pub fn lookup_many(
    memory: &impl Memory,
    country: usize,
    sorted_keys: &[u32],
    output: &mut [Value],
) -> Result<(), &'static str> {
    validate_keys(sorted_keys, output.len())?;
    let address = if sorted_keys.is_empty() {
        0
    } else {
        country
            .checked_add(ccountry::MODIFIER_ACCUMULATOR)
            .ok_or("modifier accumulator address overflow")?
    };
    lookup_many_at(memory, address, sorted_keys, output)
}

fn validate_keys(sorted_keys: &[u32], output_len: usize) -> Result<(), &'static str> {
    if output_len != sorted_keys.len()
        || sorted_keys.first() == Some(&0)
        || sorted_keys.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("modifier lookup keys or output invalid");
    }
    Ok(())
}

/// Every entry of the accumulator at `address`, as [`lookup_many_at`] reads the array:
/// `visit` sees each complete entry's key and raw value, keys ascending. It fails as
/// that lookup fails, after visiting the complete entries before the failure; the caller
/// drops them.
pub fn entries_at(
    memory: &impl Memory,
    address: usize,
    visit: &mut impl FnMut(u32, i64),
) -> Result<(), &'static str> {
    let header = read::<[u8; 16]>(memory, address)
        .ok_or("modifier accumulator header unreadable or invalid")?;
    let array = Array::from_header(header, modifier_entry::STRIDE)
        .ok_or("modifier accumulator header unreadable or invalid")?;
    let mut previous_key = 0;
    let mut bytes = [0; SNAPSHOT_BATCH * modifier_entry::STRIDE];
    let mut first = 0;
    while first < array.count as usize {
        let count = (array.count as usize - first).min(SNAPSHOT_BATCH);
        let base = array
            .element(first as u32, modifier_entry::STRIDE)
            .ok_or("modifier accumulator entry address overflow")?;
        let completed = memory.copy_records(
            crate::memory::RecordSources::Strided {
                base,
                stride: modifier_entry::STRIDE,
                count,
            },
            &SNAPSHOT_LAYOUT,
            &mut bytes[..count * modifier_entry::STRIDE],
        );
        for entry in
            bytes[..completed * modifier_entry::STRIDE].chunks_exact(modifier_entry::STRIDE)
        {
            let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
            if key <= previous_key {
                return Err("modifier accumulator keys not positive and increasing");
            }
            previous_key = key;
            visit(key, i64::from_le_bytes(entry[8..].try_into().unwrap()));
        }
        if completed != count {
            return Err("modifier accumulator entry unreadable");
        }
        first += count;
    }
    Ok(())
}

/// The same exact raw census for an arbitrary authenticated accumulator address,
/// not necessarily the accumulator embedded in a country. Read the header once
/// and validate all entries, including the unselected suffix.
pub fn lookup_many_at(
    memory: &impl Memory,
    address: usize,
    sorted_keys: &[u32],
    output: &mut [Value],
) -> Result<(), &'static str> {
    lookup_many_at_keeping(memory, address, sorted_keys, output, &mut |_| {})
}

/// [`lookup_many_at`], offering each batch of entries it read completely to
/// `keep`, in source order, before validating them: the same reads, results
/// and failures.
pub fn lookup_many_at_keeping(
    memory: &impl Memory,
    address: usize,
    sorted_keys: &[u32],
    output: &mut [Value],
    keep: &mut impl FnMut(&[u8]),
) -> Result<(), &'static str> {
    validate_keys(sorted_keys, output.len())?;
    output.fill(Value::default());
    if sorted_keys.is_empty() {
        return Ok(());
    }
    let header = read::<[u8; 16]>(memory, address)
        .ok_or("modifier accumulator header unreadable or invalid")?;
    let array = Array::from_header(header, modifier_entry::STRIDE)
        .ok_or("modifier accumulator header unreadable or invalid")?;
    let mut previous_key = 0;
    let mut wanted = 0;
    let mut bytes = [0; SNAPSHOT_BATCH * modifier_entry::STRIDE];
    let mut first = 0;
    while first < array.count as usize {
        let count = (array.count as usize - first).min(SNAPSHOT_BATCH);
        let base = array
            .element(first as u32, modifier_entry::STRIDE)
            .ok_or("modifier accumulator entry address overflow")?;
        let completed = memory.copy_records(
            crate::memory::RecordSources::Strided {
                base,
                stride: modifier_entry::STRIDE,
                count,
            },
            &SNAPSHOT_LAYOUT,
            &mut bytes[..count * modifier_entry::STRIDE],
        );
        keep(&bytes[..completed * modifier_entry::STRIDE]);
        // Only complete entries may affect keys or output. Validate their
        // ordered prefix before reporting a later batch fault, including when
        // all requested keys were already found in an earlier batch.
        for entry in
            bytes[..completed * modifier_entry::STRIDE].chunks_exact(modifier_entry::STRIDE)
        {
            let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
            if key <= previous_key {
                return Err("modifier accumulator keys not positive and increasing");
            }
            previous_key = key;
            while wanted < sorted_keys.len() && sorted_keys[wanted] < key {
                wanted += 1;
            }
            if wanted < sorted_keys.len() && sorted_keys[wanted] == key {
                output[wanted] = Value {
                    raw: i64::from_le_bytes(entry[8..].try_into().unwrap()),
                    present: true,
                };
                wanted += 1;
            }
        }
        if completed != count {
            return Err("modifier accumulator entry unreadable");
        }
        first += count;
    }
    Ok(())
}

/// One native sorted-key lookup. Unlike a complete snapshot, this reads only
/// search keys and the matching value; an unrelated entry's unreadable value
/// cannot refuse the requested operand.
pub fn lookup_requested_at(
    memory: &impl Memory,
    address: usize,
    key: u32,
) -> Result<Value, &'static str> {
    if let Some(result) = memory.modifier_accumulator_value(address, key) {
        return result;
    }
    // Native lookup consumes data and signed count, never allocation capacity.
    let data = read::<u64>(memory, address).ok_or("modifier accumulator data unreadable")? as usize;
    let count = address
        .checked_add(COUNT)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("modifier accumulator count unreadable")?;
    if count < 0 {
        return Err("modifier accumulator count negative");
    }
    let array = Array {
        data,
        count: count as u32,
    };
    let (mut first, mut end) = (0, array.count);
    while first < end {
        let mid = first + (end - first) / 2;
        let entry = array
            .element(mid, modifier_entry::STRIDE)
            .ok_or("modifier accumulator entry address overflow")?;
        let found = entry
            .checked_add(modifier_entry::KEY)
            .and_then(|at| read::<u32>(memory, at))
            .ok_or("modifier accumulator search key unreadable")?;
        match key.cmp(&found) {
            std::cmp::Ordering::Less => end = mid,
            std::cmp::Ordering::Greater => first = mid + 1,
            std::cmp::Ordering::Equal => {
                return entry
                    .checked_add(modifier_entry::VALUE)
                    .and_then(|at| read::<i64>(memory, at))
                    .map(|raw| Value { raw, present: true })
                    .ok_or("modifier accumulator selected value unreadable");
            }
        }
    }
    Ok(Value::default())
}

pub fn lookup(memory: &impl Memory, country: usize, key: u32) -> Result<Value, &'static str> {
    let mut values = [Value::default()];
    lookup_many(memory, country, &[key], &mut values)?;
    Ok(values[0])
}

pub fn lookup_at(memory: &impl Memory, address: usize, key: u32) -> Result<Value, &'static str> {
    validate_keys(&[key], 1)?;
    if let Some(result) = memory.modifier_accumulator_value(address, key) {
        return result;
    }
    let mut values = [Value::default()];
    lookup_many_at(memory, address, &[key], &mut values)?;
    Ok(values[0])
}

/// Validate every entry, including the unused suffix, of an owned full-array
/// snapshot. Header and extent authentication belong to the owner of the
/// pass-local storage.
pub fn validate_snapshot(entries: &[u8]) -> Result<(), &'static str> {
    if !entries.len().is_multiple_of(modifier_entry::STRIDE) {
        return Err("modifier snapshot byte length invalid");
    }
    let mut previous = 0;
    for entry in entries.chunks_exact(modifier_entry::STRIDE) {
        let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
        if key <= previous {
            return Err("modifier accumulator keys not positive and increasing");
        }
        previous = key;
    }
    Ok(())
}

/// Binary search an unchanged owned snapshot previously accepted by
/// validate_snapshot. Only raw presence/value are shared; effective metadata
/// still runs at each caller through lookup_effective_at.
pub fn lookup_snapshot(entries: &[u8], key: u32) -> Result<Value, &'static str> {
    validate_keys(&[key], 1)?;
    if !entries.len().is_multiple_of(modifier_entry::STRIDE) {
        return Err("modifier snapshot byte length invalid");
    }
    let mut lower = 0;
    let mut upper = entries.len() / modifier_entry::STRIDE;
    while lower < upper {
        let middle = lower + (upper - lower) / 2;
        let entry =
            &entries[middle * modifier_entry::STRIDE..(middle + 1) * modifier_entry::STRIDE];
        let candidate = u32::from_le_bytes(entry[..4].try_into().unwrap());
        match candidate.cmp(&key) {
            std::cmp::Ordering::Less => lower = middle + 1,
            std::cmp::Ordering::Greater => upper = middle,
            std::cmp::Ordering::Equal => {
                return Ok(Value {
                    raw: i64::from_le_bytes(entry[8..16].try_into().unwrap()),
                    present: true,
                });
            }
        }
    }
    Ok(Value::default())
}

/// Passive getter preserving raw presence separately from the effective
/// value. Absent keys do not consult modifier metadata.
pub fn lookup_effective_at(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
) -> Result<EffectiveValue, &'static str> {
    let value = lookup_at(memory, address, key)?;
    Ok(EffectiveValue {
        raw: value.raw,
        present: value.present,
        effective: if value.present {
            crate::modifier_definition::effective(memory, base, key, value.raw)?
        } else {
            0
        },
    })
}

/// Requested-key effective lookup. It follows only the reached search probes
/// and tests eligibility before reading the raw value.
pub fn requested_effective_at(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
) -> Result<i64, &'static str> {
    requested_operand_at(memory, base, address, key).map(|value| value.effective)
}

/// The same native probe/lazy eligibility order, retaining only a value the
/// getter actually reached. Disabled features need no raw value read.
pub fn requested_operand_at(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
) -> Result<RequestedOperand, &'static str> {
    requested_operand(memory, base, address, key, None)
}

/// Native requested-key probes with eligibility/clamp metadata shared across a tree walk.
pub fn requested_effective_memo(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
    memo: &crate::modifier_definition::DefinitionMemo,
) -> Result<i64, &'static str> {
    requested_operand(memory, base, address, key, Some(memo)).map(|operand| operand.effective)
}
fn requested_operand(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
    memo: Option<&crate::modifier_definition::DefinitionMemo>,
) -> Result<RequestedOperand, &'static str> {
    let data =
        read::<u64>(memory, address).ok_or("strategic action modifier data unreadable")? as usize;
    let count = address
        .checked_add(COUNT)
        .and_then(|p| read::<i32>(memory, p))
        .ok_or("strategic action modifier count unreadable")?;
    if count < 0 {
        // A negative extent skips probes but still reaches the final dereference.
        return Err("strategic action modifier count negative");
    }
    if count == 0 {
        return Ok(RequestedOperand::default());
    }
    let end = data
        .checked_add(count as usize * modifier_entry::STRIDE)
        .ok_or("strategic action modifier extent overflow")?;
    let mut first = data;
    let mut remaining = count as usize;
    while remaining != 0 {
        let old = remaining;
        remaining >>= 1;
        let candidate = first
            .checked_add(remaining * modifier_entry::STRIDE)
            .ok_or("strategic action modifier probe overflow")?;
        let candidate_key =
            read::<u32>(memory, candidate).ok_or("strategic action modifier key unreadable")?;
        if candidate_key < key {
            // Advance by half plus the old count's low bit, including even counts.
            first = candidate
                .checked_add((old & 1) * modifier_entry::STRIDE)
                .ok_or("strategic action modifier probe overflow")?;
        }
    }
    if first == end {
        return Ok(RequestedOperand::default());
    }
    let found = read::<u32>(memory, first).ok_or("strategic action modifier key unreadable")?;
    if key < found {
        return Ok(RequestedOperand::default());
    }
    let mut raw = None;
    let get = || {
        let value = first
            .checked_add(modifier_entry::VALUE)
            .and_then(|p| read::<i64>(memory, p))
            .ok_or("strategic action modifier value unreadable")?;
        raw = Some(value);
        Ok(value)
    };
    let effective = if let Some(memo) = memo {
        crate::modifier_definition::effective_lazy_memo(memory, base, key, get, memo)?
    } else {
        crate::modifier_definition::effective_lazy(memory, base, key, get)?
    };
    Ok(RequestedOperand {
        raw,
        present: true,
        effective,
    })
}

#[cfg(test)]
mod tests;

/// One raw native getter key, without copying or validating unrelated entries.
/// The owning writer certificate supplies the sorted-array lifetime.
pub fn lookup_key_at(
    memory: &impl Memory,
    address: usize,
    key: u32,
) -> Result<Value, &'static str> {
    let data = read::<u64>(memory, address)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or("modifier point data unavailable")?;
    let count = address
        .checked_add(COUNT)
        .and_then(|at| read::<i32>(memory, at))
        .filter(|v| *v >= 0)
        .ok_or("modifier point count unavailable")? as usize;
    let mut first = 0usize;
    let mut remaining = count;
    while remaining != 0 {
        let half = remaining / 2;
        let index = first + half;
        let at = index
            .checked_mul(modifier_entry::STRIDE)
            .and_then(|off| data.checked_add(off))
            .ok_or("modifier point address overflow")?;
        let found = read::<u32>(memory, at).ok_or("modifier point key unavailable")?;
        if found < key {
            first = index + 1;
            remaining -= half + 1;
        } else {
            remaining = half;
        }
    }
    if first == count {
        return Ok(Value::default());
    }
    let at = first
        .checked_mul(modifier_entry::STRIDE)
        .and_then(|off| data.checked_add(off))
        .ok_or("modifier point address overflow")?;
    if read::<u32>(memory, at).ok_or("modifier point key unavailable")? != key {
        return Ok(Value::default());
    }
    let raw = at
        .checked_add(modifier_entry::VALUE)
        .and_then(|at| read::<i64>(memory, at))
        .ok_or("modifier point value unavailable")?;
    Ok(Value { raw, present: true })
}
#[cfg(test)]
mod point_tests {
    use super::*;
    // regression: 25443bcda9 Fix faction hook spans, retain account marks and carry subject contribution
    #[test]
    fn one_of_many_keys_does_not_read_the_unselected_payloads() {
        struct Counted {
            bytes: Vec<u8>,
            reads: std::cell::Cell<usize>,
        }
        impl Memory for Counted {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                crate::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes,
                }
                .copy(at, out)
            }
        }
        let mut bytes = vec![0; 64 + 200 * modifier_entry::STRIDE];
        bytes[16..24].copy_from_slice(&64u64.to_le_bytes());
        bytes[16 + COUNT..16 + COUNT + 4].copy_from_slice(&200i32.to_le_bytes());
        for index in 0..200 {
            let at = 64 + index * modifier_entry::STRIDE;
            bytes[at..at + 4].copy_from_slice(&(index as u32 * 2 + 1).to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&(-17i64).to_le_bytes());
        }
        let memory = Counted {
            bytes,
            reads: std::cell::Cell::new(0),
        };
        assert_eq!(
            lookup_key_at(&memory, 16, 301).unwrap(),
            Value {
                raw: -17,
                present: true
            }
        );
        assert!(memory.reads.get() < 20);
        assert_eq!(lookup_key_at(&memory, 16, 300).unwrap(), Value::default());
    }
}
