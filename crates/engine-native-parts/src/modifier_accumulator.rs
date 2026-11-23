
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::modifier_entry;

pub const DATA: usize = 0x00;
pub const CAPACITY: usize = 0x08;
pub const COUNT: usize = 0x0c;

use engine_native_memory::modifier_cache::Value;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveValue {
    pub raw: i64,
    pub present: bool,
    pub effective: i64,
}

pub use game_model::modifiers::RequestedOperand;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotCapacity {
    pub nodes: usize,
    pub entries: usize,
}

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
const REQUESTED_SLICE_MAX: usize = 256;
const SINGLE_SLICE_MAX: usize = 32;
const SNAPSHOT_FIELDS: [engine_native_memory::memory::RecordField; 1] = [engine_native_memory::memory::RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: modifier_entry::STRIDE,
}];
const SNAPSHOT_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&SNAPSHOT_FIELDS, modifier_entry::STRIDE)
        .expect("modifier snapshot entry layout");

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
                .ok_or_else(engine_allocation::oom)?
        };
        Ok(Self {
            nodes: engine_allocation::vec(capacity.nodes)?,
            entries: engine_allocation::vec(capacity.entries)?,
            index: engine_allocation::filled(slots, 0)?,
            capacity,
            required: SnapshotCapacity::default(),
            generation: 0,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
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

    pub fn required_capacity(&self) -> SnapshotCapacity {
        self.required
    }

    pub fn allocated_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<SnapshotNode>()
            + self.entries.capacity() * std::mem::size_of::<[u8; 16]>()
            + self.index.capacity() * std::mem::size_of::<usize>()
    }

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

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        country: usize,
        key: u32,
    ) -> Result<SnapshotObservation, &'static str> {
        validate_keys(&[key], 1)?;
        let address = country
            .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
            .ok_or("modifier accumulator address overflow")?;
        self.capture_at(memory, address, key)
    }

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

    fn insert(&mut self, address: usize) -> Result<usize, &'static str> {
        let mut slot = (!self.index.is_empty()).then(|| self.index_slot(address));
        self.required.nodes = self.required.nodes.max(self.nodes.len().saturating_add(1));
        if self.nodes.len() == self.capacity.nodes {
            let count = engine_allocation::add(self.nodes.len(), 1)
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
            engine_allocation::reserve_exact(&mut self.nodes, additional)
                .map_err(|_| "modifier snapshot allocation failed")?;
            self.index = engine_allocation::filled(slots, 0)
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

    pub fn hold(&mut self, address: usize, entries: &[[u8; 16]]) {
        if self.existing(address).is_some()
            || engine_allocation::reserve(&mut self.entries, entries.len()).is_err()
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

    pub fn observe(&mut self, memory: &impl Memory, address: usize) -> Option<&[[u8; 16]]> {
        if self.existing(address).is_none() {
            let node = self.insert(address).ok()?;
            let result = self.capture_node(memory, node);
            self.nodes[node].result = result;
        }
        self.observed(address)
    }

    pub fn observed(&self, address: usize) -> Option<&[[u8; 16]]> {
        let node = &self.nodes[self.existing(address)?];
        node.result
            .ok()
            .map(|()| &self.entries[node.start..node.start + node.len])
    }

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
                engine_native_memory::memory::RecordSources::Strided {
                    base,
                    stride: modifier_entry::STRIDE,
                    count,
                },
                &SNAPSHOT_LAYOUT,
                scratch[..count].as_flattened_mut(),
            );
            engine_allocation::reserve(&mut self.entries, completed)
                .map_err(|_| "modifier snapshot allocation failed")?;
            self.entries.extend_from_slice(&scratch[..completed]);
            self.capacity.entries = self.capacity.entries.max(self.entries.len());
            self.required.entries = self.required.entries.saturating_add(completed);
            self.nodes[node].len += completed;
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
            .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
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
            engine_native_memory::memory::RecordSources::Strided {
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

pub fn lookup_many_at(
    memory: &impl Memory,
    address: usize,
    sorted_keys: &[u32],
    output: &mut [Value],
) -> Result<(), &'static str> {
    lookup_many_at_keeping(memory, address, sorted_keys, output, &mut |_| {})
}

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
            engine_native_memory::memory::RecordSources::Strided {
                base,
                stride: modifier_entry::STRIDE,
                count,
            },
            &SNAPSHOT_LAYOUT,
            &mut bytes[..count * modifier_entry::STRIDE],
        );
        keep(&bytes[..completed * modifier_entry::STRIDE]);
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

fn read_requested_array(memory: &impl Memory, address: usize) -> Result<Array, &'static str> {
    if let Some(header) = read::<[u8; 16]>(memory, address) {
        let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
        let count = i32::from_le_bytes(header[COUNT..COUNT + 4].try_into().unwrap());
        if count < 0 {
            return Err("modifier accumulator count negative");
        }
        return Ok(Array {
            data,
            count: count as u32,
        });
    }
    let data = read::<u64>(memory, address).ok_or("modifier accumulator data unreadable")? as usize;
    let count = address
        .checked_add(COUNT)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("modifier accumulator count unreadable")?;
    if count < 0 {
        return Err("modifier accumulator count negative");
    }
    Ok(Array {
        data,
        count: count as u32,
    })
}

fn search_entries_local(entries: &[u8], key: u32) -> Value {
    let mut first = 0;
    let mut end = entries.len() / modifier_entry::STRIDE;
    while first < end {
        let mid = first + (end - first) / 2;
        let entry = &entries[mid * modifier_entry::STRIDE..(mid + 1) * modifier_entry::STRIDE];
        let found = u32::from_le_bytes(entry[..4].try_into().unwrap());
        match key.cmp(&found) {
            std::cmp::Ordering::Less => end = mid,
            std::cmp::Ordering::Greater => first = mid + 1,
            std::cmp::Ordering::Equal => {
                return Value {
                    raw: i64::from_le_bytes(entry[8..16].try_into().unwrap()),
                    present: true,
                };
            }
        }
    }
    Value::default()
}

fn search_array_remote(
    memory: &impl Memory,
    array: Array,
    key: u32,
) -> Result<Value, &'static str> {
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

pub fn lookup_requested_at(
    memory: &impl Memory,
    address: usize,
    key: u32,
) -> Result<Value, &'static str> {
    if let Some(result) = memory.modifier_accumulator_value(address, key) {
        return result;
    }
    let data = read::<u64>(memory, address).ok_or("modifier accumulator data unreadable")? as usize;
    let count = address
        .checked_add(COUNT)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("modifier accumulator count unreadable")?;
    if count < 0 {
        return Err("modifier accumulator count negative");
    }
    search_array_remote(
        memory,
        Array {
            data,
            count: count as u32,
        },
        key,
    )
}

pub fn lookup_requested_many_at(
    memory: &impl Memory,
    address: usize,
    keys: &[u32],
    output: &mut [Value],
) -> Result<(), &'static str> {
    if keys.len() != output.len() {
        return Err("modifier lookup keys or output invalid");
    }
    if keys.is_empty() {
        return Ok(());
    }
    let mut unresolved = 0usize;
    let mut cached = [false; 64];
    for (index, (&key, slot)) in keys.iter().zip(output.iter_mut()).enumerate() {
        if let Some(result) = memory.modifier_accumulator_value(address, key) {
            *slot = result?;
            if index < cached.len() {
                cached[index] = true;
            }
        } else {
            *slot = Value::default();
            unresolved += 1;
        }
    }
    if unresolved == 0 {
        return Ok(());
    }
    let array = read_requested_array(memory, address)?;
    let count = array.count as usize;
    if count == 0 {
        return Ok(());
    }
    if count <= REQUESTED_SLICE_MAX && (unresolved >= 2 || count <= SINGLE_SLICE_MAX) {
        let mut bytes = [0_u8; REQUESTED_SLICE_MAX * modifier_entry::STRIDE];
        let span = &mut bytes[..count * modifier_entry::STRIDE];
        if array.data != 0 && crate::read_slice(memory, array.data, span) {
            for (index, (&key, slot)) in keys.iter().zip(output.iter_mut()).enumerate() {
                if index < cached.len() && cached[index] {
                    continue;
                }
                if index >= cached.len() && memory.modifier_accumulator_value(address, key).is_some()
                {
                    continue;
                }
                *slot = search_entries_local(span, key);
            }
            return Ok(());
        }
    }
    for (index, (&key, slot)) in keys.iter().zip(output.iter_mut()).enumerate() {
        if index < cached.len() && cached[index] {
            continue;
        }
        if index >= cached.len() && memory.modifier_accumulator_value(address, key).is_some() {
            continue;
        }
        *slot = search_array_remote(memory, array, key)?;
    }
    Ok(())
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

pub fn requested_effective_at(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
) -> Result<i64, &'static str> {
    requested_operand_at(memory, base, address, key).map(|value| value.effective)
}

pub fn requested_operand_at(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
) -> Result<RequestedOperand, &'static str> {
    requested_operand(memory, base, address, key, None)
}

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
    #[test]
    fn one_of_many_keys_does_not_read_the_unselected_payloads() {
        struct Counted {
            bytes: Vec<u8>,
            reads: std::cell::Cell<usize>,
        }
        impl Memory for Counted {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                engine_native_memory::memory::SliceMemory {
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
